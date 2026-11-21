// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the per-target blend key list of c2d132ad
//! (gl::target_keys::draw_target_keys / draw_mixed_key) and cff449d2
//! (current_blend_key tries the mixed per-target key first for several
//! targets). Every ES 2.0 blend draw passes through that code, and the i.MX8MP
//! whole dEQP-GLES2 tree (0fbe61a3) predates both; its stage-1 promise is 0
//! Fail. ES 2.0 has one colour target, so the list is never mixed; this file
//! drives blend on and off through the real GL core and the real Vivante back
//! end over the stand-in OSAL and asserts for each draw: GL_NO_ERROR, no
//! refused draw (`gc_refused_draws()` unchanged), the single-target fragment
//! shader variant (the GC7000 blends in its pixel engine and its table has no
//! blend variants, so one program keeps one PS_INST_ADDR whatever the blend
//! state, the target or the mask), and the PE words equal to what the rule of
//! the parent commit (cff449d2~1, src/vivante/backend.rs emit_pe_state with
//! pe_overwrite.rs) gives, written out below from that source:
//!
//! - blend is on only when GL_BLEND is enabled and the state is not
//!   ONE/ZERO/ADD for both colour and alpha;
//! - PE_ALPHA_CONFIG: 0 without blend; else ENABLE_COLOR, SEPARATE_ALPHA when
//!   the RGB and alpha func or equation differ, the factors and equations in
//!   their fields; a target with no alpha bits reads destination alpha as 1:
//!   DST_ALPHA -> ONE, ONE_MINUS_DST_ALPHA and SRC_ALPHA_SATURATE -> ZERO;
//! - PE_ALPHA_BLEND_COLOR: the colour as four unorm8 bytes (R at bit 16);
//!   PE_ALPHA_COLOR_EXT0/1 as half floats;
//! - PE_COLOR_FORMAT COMPONENTS is glColorMask (RGB only for a target without
//!   alpha), OVERWRITE for a full mask, no blend, one sample (a target with
//!   no alpha never overwrites).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_blend_key
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glRenderbufferStorage,
};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{
    glBlendColor, glBlendEquationSeparate, glBlendFuncSeparate, glColorMask, glDisable, glEnable, glFinish, glGetError,
    glViewport,
};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are
/// process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs) */
        match p.allocs.remove(&m.gpu) {
            Some(a) => core::mem::forget(a.mem),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept and its fence raised; every kick completes at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let words = words.to_vec();
        p.kicks.push(words);
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- GL plumbing ------------------------------------------------------------------- */

/// Run `case` on the real Vivante back end over the stand-in OSAL and a
/// fresh 64x64 context.
fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: SERIAL is held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| core::mem::take(&mut p.faults));
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
/// A translucent flat colour, so that every blend factor reads something.
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.8, 0.6, 0.4, 0.5); }\n";

/// The program every draw uses, attribute `p` at 0.
fn program() -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The ES 2.0 colour targets.
#[derive(Clone, Copy, Debug)]
enum Target {
    /// The default framebuffer (the context's 64x64 surface).
    Window,
    /// A GL_RGBA / GL_UNSIGNED_BYTE texture.
    TexRgba8,
    /// A GL_RGBA8_OES renderbuffer.
    RbRgba8,
    Rb565,
    Rb4444,
    Rb5551,
}

const TARGETS: [Target; 6] =
    [Target::Window, Target::TexRgba8, Target::RbRgba8, Target::Rb565, Target::Rb4444, Target::Rb5551];

fn renderbuffer(format: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut r: GLuint = 0;
        glGenRenderbuffers(1, &mut r);
        glBindRenderbuffer(GL_RENDERBUFFER, r);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 64, 64);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, r);
    }
}

impl Target {
    /// The alpha bits GL reports for the target (the parent rule keys on them).
    fn alpha_bits(self) -> u32 {
        match self {
            Target::Window | Target::TexRgba8 | Target::RbRgba8 => 8,
            Target::Rb565 => 0,
            Target::Rb4444 => 4,
            Target::Rb5551 => 1,
        }
    }

    fn bind(self) {
        if !matches!(self, Target::Window) {
            // SAFETY: plain GL calls.
            unsafe {
                let mut fb: GLuint = 0;
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
            }
        }
        match self {
            Target::Window => {}
            Target::TexRgba8 => {
                // SAFETY: plain GL calls.
                unsafe {
                    let mut t: GLuint = 0;
                    glGenTextures(1, &mut t);
                    glBindTexture(GL_TEXTURE_2D, t);
                    let null = core::ptr::null();
                    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
                    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
                }
            }
            Target::RbRgba8 => renderbuffer(GL_RGBA8_OES),
            Target::Rb565 => renderbuffer(GL_RGB565),
            Target::Rb4444 => renderbuffer(GL_RGBA4),
            Target::Rb5551 => renderbuffer(GL_RGB5_A1),
        }
        // SAFETY: plain GL calls.
        unsafe {
            if !matches!(self, Target::Window) {
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{self:?}");
            }
            glViewport(0, 0, 64, 64);
        }
        no_error("the target");
    }
}

/* ---- the stream: the register file at each draw ------------------------------------ */

/// Every register the kicks write, replayed in order into `file` (a skipped
/// write is an unchanged register, which `file` keeps); the file as it stood
/// at the last draw, and the number of draws.
fn replay(kicks: &[Vec<u32>], file: &mut BTreeMap<u32, u32>) -> (Option<BTreeMap<u32, u32>>, usize) {
    let mut at_draw = None;
    let mut n = 0;
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        file.insert(base + 4 * k as u32, words[i + 1 + k]);
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    n += 1;
                    at_draw = Some(file.clone());
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    (at_draw, n)
}

/* ---- the parent commit's rule ------------------------------------------------------ */

/// The GL state of one draw.
#[derive(Clone, Copy, Debug)]
struct Blend {
    enabled: bool,
    src_rgb: GLenum,
    dst_rgb: GLenum,
    src_a: GLenum,
    dst_a: GLenum,
    eq_rgb: GLenum,
    eq_a: GLenum,
    colour: [f32; 4],
    /// glColorMask bits R=1 G=2 B=4 A=8.
    mask: u8,
}

impl Blend {
    const OFF: Blend = Blend {
        enabled: false,
        src_rgb: GL_ONE,
        dst_rgb: GL_ZERO,
        src_a: GL_ONE,
        dst_a: GL_ZERO,
        eq_rgb: GL_FUNC_ADD,
        eq_a: GL_FUNC_ADD,
        colour: [0.0; 4],
        mask: 0xf,
    };

    fn apply(self) {
        let m = |bit: u8| (self.mask & bit != 0) as GLboolean;
        // SAFETY: plain GL calls.
        unsafe {
            if self.enabled {
                glEnable(GL_BLEND);
            } else {
                glDisable(GL_BLEND);
            }
            glBlendFuncSeparate(self.src_rgb, self.dst_rgb, self.src_a, self.dst_a);
            glBlendEquationSeparate(self.eq_rgb, self.eq_a);
            glBlendColor(self.colour[0], self.colour[1], self.colour[2], self.colour[3]);
            glColorMask(m(1), m(2), m(4), m(8));
        }
    }
}

/// The PE_ALPHA_CONFIG factor code of a GL blend factor (state_3d.xml.h).
fn factor_code(f: GLenum) -> u32 {
    match f {
        GL_ZERO => 0x0,
        GL_ONE => 0x1,
        GL_SRC_COLOR => 0x2,
        GL_ONE_MINUS_SRC_COLOR => 0x3,
        GL_SRC_ALPHA => 0x4,
        GL_ONE_MINUS_SRC_ALPHA => 0x5,
        GL_DST_ALPHA => 0x6,
        GL_ONE_MINUS_DST_ALPHA => 0x7,
        GL_DST_COLOR => 0x8,
        GL_ONE_MINUS_DST_COLOR => 0x9,
        GL_SRC_ALPHA_SATURATE => 0xa,
        GL_CONSTANT_ALPHA => 0xb,
        GL_ONE_MINUS_CONSTANT_ALPHA => 0xc,
        GL_CONSTANT_COLOR => 0xd,
        GL_ONE_MINUS_CONSTANT_COLOR => 0xe,
        other => panic!("factor 0x{other:x}"),
    }
}

fn eq_code(e: GLenum) -> u32 {
    match e {
        GL_FUNC_ADD => 0,
        GL_FUNC_SUBTRACT => 1,
        GL_FUNC_REVERSE_SUBTRACT => 2,
        other => panic!("equation 0x{other:x}"),
    }
}

/// `unorm8` of the parent's backend.rs.
fn unorm8(f: f32) -> u32 {
    if f <= 0.0 {
        0
    } else if f >= 1.0 {
        255
    } else {
        (f * 255.0 + 0.5) as u32
    }
}

/// The IEEE half of a colour that is 0 or an exact binary fraction in (0, 1].
fn half(f: f32) -> u32 {
    if f == 0.0 {
        return 0;
    }
    let u = f.to_bits();
    assert_eq!(u & 0x1fff, 0, "{f} is not exact in a half");
    (((u >> 23) + 15 - 127) << 10) | ((u & 0x7f_ffff) >> 13)
}

/// The words the parent's emit_pe_state writes for a draw.
#[derive(Debug, PartialEq)]
struct PeWords {
    alpha_config: u32,
    blend_color: u32,
    ext0: u32,
    ext1: u32,
    components: u32,
    overwrite: bool,
}

/// The parent's rule for a draw of `b` on a target of `alpha_bits`.
fn parent_rule(b: Blend, alpha_bits: u32) -> PeWords {
    let blend = b.enabled
        && !(b.src_rgb == GL_ONE
            && b.dst_rgb == GL_ZERO
            && b.eq_rgb == GL_FUNC_ADD
            && b.src_a == GL_ONE
            && b.dst_a == GL_ZERO
            && b.eq_a == GL_FUNC_ADD);
    let factor = |f: GLenum| {
        let x = factor_code(f);
        if alpha_bits != 0 {
            x
        } else if x == 0x6 {
            0x1
        } else if x == 0x7 || x == 0xa {
            0x0
        } else {
            x
        }
    };
    let mut alpha_config = 0;
    if blend {
        let separate = b.src_rgb != b.src_a || b.dst_rgb != b.dst_a || b.eq_rgb != b.eq_a;
        alpha_config = 0x1
            | if separate { 0x1_0000 } else { 0 }
            | (factor(b.src_rgb) << 4)
            | (factor(b.dst_rgb) << 8)
            | (eq_code(b.eq_rgb) << 12)
            | (factor(b.src_a) << 20)
            | (factor(b.dst_a) << 24)
            | (eq_code(b.eq_a) << 28);
    }
    let c = b.colour;
    let blend_color = (unorm8(c[0]) << 16) | (unorm8(c[1]) << 8) | unorm8(c[2]) | (unorm8(c[3]) << 24);
    let mask = u32::from(b.mask);
    let (components, overwrite) = if alpha_bits != 0 { (mask, !blend && mask == 0xf) } else { (mask & 7, false) };
    PeWords {
        alpha_config,
        blend_color,
        ext0: half(c[0]) | (half(c[1]) << 16),
        ext1: half(c[2]) | (half(c[3]) << 16),
        components,
        overwrite,
    }
}

/* ---- the driver -------------------------------------------------------------------- */

/// One target, one program, one quad: `draw(b)` makes a draw under `b` and
/// asserts everything the file promises.
struct Rig {
    target: Target,
    file: BTreeMap<u32, u32>,
    ps_inst_addr: Option<u32>,
    draws: usize,
}

impl Rig {
    fn new(target: Target) -> Rig {
        target.bind();
        let p = program();
        let mut vbo: GLuint = 0;
        // SAFETY: plain GL calls; QUAD is a static.
        unsafe {
            glUseProgram(p);
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(
                GL_ARRAY_BUFFER,
                core::mem::size_of_val(&QUAD) as GLsizeiptr,
                QUAD.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        }
        no_error("the quad buffer");
        Rig { target, file: BTreeMap::new(), ps_inst_addr: None, draws: 0 }
    }

    fn draw(&mut self, b: Blend) {
        self.draw_as(b, b);
    }

    /// Draw under `b`; the words are the rule's for `stored`, what GL keeps
    /// of `b` (glBlendColor clamps).
    fn draw_as(&mut self, b: Blend, stored: Blend) {
        let what = format!("{:?} {b:?}", self.target);
        let refused = gc_refused_draws();
        with_platform(|p| p.kicks.clear());
        b.apply();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            glFinish();
        }
        no_error(&what);
        assert_eq!(gc_refused_draws(), refused, "{what}: a refused draw");
        let kicks = with_platform(|p| core::mem::take(&mut p.kicks));
        let (regs, n) = replay(&kicks, &mut self.file);
        assert_eq!(n, 1, "{what}: one draw");
        let regs = regs.unwrap();
        self.draws += 1;

        /* the single-target variant: one fragment program for every state */
        let ps = regs[&GC_PS_INST_ADDR];
        assert_eq!(*self.ps_inst_addr.get_or_insert(ps), ps, "{what}: another fragment shader variant");

        let cf = regs[&GC_PE_COLOR_FORMAT];
        let got = PeWords {
            alpha_config: regs[&GC_PE_ALPHA_CONFIG],
            blend_color: regs[&GC_PE_ALPHA_BLEND_COLOR],
            ext0: regs[&GC_PE_ALPHA_COLOR_EXT0],
            ext1: regs[&GC_PE_ALPHA_COLOR_EXT1],
            components: (cf >> 8) & 0xf,
            overwrite: cf & GC_PE_COLOR_FORMAT_OVERWRITE != 0,
        };
        let want = parent_rule(stored, self.target.alpha_bits());
        assert_eq!(got, want, "{what}: PE words (the parent's rule on the right)");
    }
}

const SRC: [GLenum; 15] = [
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
    GL_SRC_ALPHA_SATURATE,
];
/// The destination factors: every one but SRC_ALPHA_SATURATE.
const DST: &[GLenum] = &[
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
];
const EQS: [GLenum; 3] = [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT];
/// Colours exact in a half float, the dEQP blend cases' constants among them.
const COLOURS: [[f32; 4]; 4] = [[0.0, 0.0, 0.0, 0.0], [0.25, 0.5, 0.75, 1.0], [1.0, 0.125, 0.5, 0.25], [0.5; 4]];

fn each_target(f: impl Fn(&mut Rig)) {
    for t in TARGETS {
        with_backend(|| {
            let mut rig = Rig::new(t);
            rig.draw(Blend::OFF); /* the first draw of a process writes more state */
            f(&mut rig);
            assert!(rig.draws > 1);
        });
    }
}

/// Every func pair of dEQP-GLES2 fragment_ops.blend (15 source x 14
/// destination factors, SRC_ALPHA_SATURATE for the source only) under each
/// ES 2.0 equation, on every target; a draw with blend off between them.
#[test]
fn every_func_pair_and_equation_is_the_parent_rule() {
    each_target(|rig| {
        let mut k = 0;
        for eq in EQS {
            for &src in &SRC {
                for &dst in DST {
                    let colour = COLOURS[k % COLOURS.len()];
                    k += 1;
                    let b = Blend {
                        enabled: true,
                        src_rgb: src,
                        dst_rgb: dst,
                        src_a: src,
                        dst_a: dst,
                        eq_rgb: eq,
                        eq_a: eq,
                        colour,
                        mask: 0xf,
                    };
                    rig.draw(b);
                }
                rig.draw(Blend { enabled: false, src_rgb: src, eq_rgb: eq, eq_a: eq, ..Blend::OFF });
            }
        }
    });
}

/// Separate RGB and alpha funcs and equations (every pairing of the three
/// equations, a rotation of the factors) with glBlendColor values.
#[test]
fn separate_rgb_and_alpha_are_the_parent_rule() {
    each_target(|rig| {
        let mut k = 0;
        for eq_rgb in EQS {
            for eq_a in EQS {
                for i in 0..SRC.len() {
                    let b = Blend {
                        enabled: true,
                        src_rgb: SRC[i],
                        dst_rgb: DST[(i * 3 + 1) % DST.len()],
                        src_a: SRC[(i + 5) % SRC.len()],
                        dst_a: DST[(i * 7 + 2) % DST.len()],
                        eq_rgb,
                        eq_a,
                        colour: COLOURS[k % COLOURS.len()],
                        mask: 0xf,
                    };
                    k += 1;
                    rig.draw(b);
                }
            }
        }
        /* only the equations apart; only a func apart */
        rig.draw(Blend { enabled: true, eq_rgb: GL_FUNC_SUBTRACT, ..Blend::OFF });
        rig.draw(Blend { enabled: true, src_a: GL_SRC_ALPHA, ..Blend::OFF });
        rig.draw(Blend { enabled: true, dst_a: GL_ONE, ..Blend::OFF });
    });
}

/// The default funcs with GL_BLEND on are no blend (ONE/ZERO/ADD); one factor
/// off the default is blend.
#[test]
fn the_default_funcs_with_blend_on_are_no_blend() {
    each_target(|rig| {
        rig.draw(Blend { enabled: true, ..Blend::OFF });
        rig.draw(Blend { enabled: true, dst_rgb: GL_ONE, ..Blend::OFF });
        rig.draw(Blend { enabled: true, ..Blend::OFF });
        rig.draw(Blend { enabled: true, dst_a: GL_ONE, ..Blend::OFF });
        rig.draw(Blend { enabled: true, eq_a: GL_FUNC_REVERSE_SUBTRACT, ..Blend::OFF });
        rig.draw(Blend { enabled: true, ..Blend::OFF });
        rig.draw(Blend::OFF);
    });
}

/// Every glColorMask subset with blend off, with the classic
/// SRC_ALPHA / ONE_MINUS_SRC_ALPHA blend and with a constant-colour one.
#[test]
fn every_colour_mask_subset_is_the_parent_rule() {
    each_target(|rig| {
        for mask in 0..16u8 {
            rig.draw(Blend { mask, ..Blend::OFF });
            rig.draw(Blend {
                enabled: true,
                src_rgb: GL_SRC_ALPHA,
                dst_rgb: GL_ONE_MINUS_SRC_ALPHA,
                src_a: GL_SRC_ALPHA,
                dst_a: GL_ONE_MINUS_SRC_ALPHA,
                mask,
                ..Blend::OFF
            });
            rig.draw(Blend {
                enabled: true,
                src_rgb: GL_CONSTANT_COLOR,
                dst_rgb: GL_ONE_MINUS_CONSTANT_ALPHA,
                src_a: GL_ONE,
                dst_a: GL_ONE,
                eq_rgb: GL_FUNC_REVERSE_SUBTRACT,
                eq_a: GL_FUNC_SUBTRACT,
                colour: COLOURS[1],
                mask,
            });
        }
    });
}

/// glBlendColor's values are clamped to [0, 1] by the GL core before the PE.
#[test]
fn the_blend_colour_is_clamped() {
    each_target(|rig| {
        let c = [-1.0, 2.0, 0.5, 1.5];
        let b =
            Blend { enabled: true, src_rgb: GL_CONSTANT_COLOR, dst_rgb: GL_CONSTANT_ALPHA, colour: c, ..Blend::OFF };
        rig.draw_as(b, Blend { colour: c.map(|x| x.clamp(0.0, 1.0)), ..b });
    });
}
