// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the texture unit asked to sample the D24S8 depth
//! surface in place (src/vivante/depth_sample.rs), gl_test 222's
//! experiment: the answer the GL core gets and the descriptor the draw
//! emits (docs/notes/gc-depth-sample.md, section (d)).
//!
//! The real back end (`gc_init`, `yakogl_backend_vivante`) runs over a
//! stand-in OSAL that keeps every allocation in host memory and records
//! every stream it is handed, as tests/gc_mipmap_gpu.rs does. Two tests:
//!
//! 1. **The answer.** A depth plane made Live in the depth cache through
//!    the real `select` and `drawn` (as tests/gc_depth_resolve.rs does):
//!    `gc_depth_sampled` and the shared `backend::samples_rendered_depth`
//!    say no with the switch off (every GL use), and under it yes for a
//!    texture the surface covers and no for another object, a larger
//!    texture, a plane that holds a whole clear only and a plane nothing
//!    holds. No OSAL wait is made for any answer. The descriptor of a
//!    binding whose surface is gone is a refusal, -ENOTSUP, and a binding
//!    with no `depth_id` is not a depth view at all.
//! 2. **The descriptor.** Through the GL core: a 64x64 GL_DEPTH_COMPONENT24
//!    texture (GL_DEPTH_COMPONENT on the ES 2.0 report) alone in a
//!    framebuffer, case 177's depth ramp drawn into it, glFinish; then a
//!    draw that samples it over the window under each view. The stream's
//!    texture descriptor is read back from the stand-in's memory and checked
//!    against the note's table: D24X8 (0x11) with FORMAT_EXT 0, LINEAR
//!    addressing, the (d, 0, 0, 1) swizzle of ES 3.0 ((d, d, d, 1) under
//!    GL_OES_depth_texture),
//!    LINEAR_STRIDE the surface's (256 bytes for 64 pixels), SIZE 64x64, one
//!    level, SLICE stride x height, LOD 0 the surface's first pixel word
//!    (`guard` bytes into the surface's allocation), NEAREST and
//!    CLAMP_TO_EDGE with INT_FILTER off; the raw view A8R8G8B8 with the
//!    identity swizzle and INT_FILTER on; and with the switch off the same
//!    draw points at the texture's own R32F texels, resolved by the CPU (a
//!    read of the surface), where the views neither waited nor read it.
//!
//! What it cannot show: what the texture unit returns for a D24X8 view of
//! a linear surface. That is the board case (gl_test 222, `dtpu`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_sample
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F};
use yakogl::gl::consts::*;
use yakogl::gl::context::{ds_object_id, GlTexture};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glGetUniformLocation, glLinkProgram,
    glUniform1i, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{
    glClear, glClearDepthf, glDepthFunc, glDisable, glEnable, glFinish, glFlush, glGetError, glViewport,
};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::depth_sample::{self, View};
use yakogl::vivante::ds_cache::{self, DsKey};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// Every `wait`, the fence asked for.
    waits: Vec<i64>,
    /// Every `cache_invd`, the bytes invalidated: the CPU is about to read
    /// what the GPU wrote (the resolve reads the whole surface mapping).
    invalidates: Vec<usize>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
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
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_cache_invd(_cpu: *mut c_void, n: usize) {
    with_platform(|p| p.invalidates.push(n));
}

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
        p.kicks.push(words.to_vec());
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| p.waits.push(fence));
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
    cache_invd: Some(p_cache_invd),
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

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(gpu: u64, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size);
        // SAFETY: inside the allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
}

/// The allocation GPU address `gpu` lies in: its base and size.
fn allocation_of(gpu: u64) -> (u64, usize) {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        assert!(gpu < base + a.size as u64, "0x{gpu:x} is past the allocation at 0x{base:x}");
        (*base, a.size)
    })
}

fn waits() -> usize {
    with_platform(|p| p.waits.len())
}

/// The cache invalidations of `bytes` so far: CPU reads of an allocation of that size.
fn invalidates_of(bytes: usize) -> usize {
    with_platform(|p| p.invalidates.iter().filter(|&&n| n == bytes).count())
}

fn kicks() -> usize {
    with_platform(|p| p.kicks.len())
}

/// Puts the switch back to off when a test ends, however it ends.
struct ViewOff;
impl Drop for ViewOff {
    fn drop(&mut self) {
        depth_sample::set_view(0);
    }
}

/* ---- the stream decoder (tests/gc_mipmap_gpu.rs) ------------------------------------ */

/// One DRAW_INSTANCED of a stream, with the register file as the stream
/// had loaded it when the draw ran.
struct Draw {
    regs: BTreeMap<u32, u32>,
}

/// The draws of a stream, in order.
fn draws(words: &[u32]) -> Vec<Draw> {
    let mut regs = BTreeMap::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    regs.insert(base + 4 * k as u32, words[i + 1 + k]);
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(Draw { regs: regs.clone() });
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

/// Slot 0's descriptor and SAMP_CTRL0 of the last draw of the last kick.
struct Slot0 {
    desc: Vec<u8>,
    samp_ctrl0: u32,
}

impl Slot0 {
    fn last() -> Slot0 {
        let words = with_platform(|p| p.kicks.last().cloned()).expect("a kick");
        let d = draws(&words).pop().expect("a draw in the last kick");
        let r = |reg: u32| d.regs.get(&reg).copied().unwrap_or_else(|| panic!("register {reg:#x} never written"));
        Slot0 {
            desc: read_gpu(u64::from(r(nte_descriptor_addr(0))), GC_TEXDESC_BYTES),
            samp_ctrl0: r(nte_descriptor_samp_ctrl0(0)),
        }
    }

    fn dw(&self, off: u32) -> u32 {
        u32::from_le_bytes(self.desc[off as usize..off as usize + 4].try_into().unwrap())
    }
}

/* ---- test 1: the answer ----------------------------------------------------------- */

const ID: u32 = 0x4000_0064;
const OTHER: u32 = 0x4000_0065;
const KEY: DsKey = DsKey { depth_id: ID, depth_gen: 1, stencil_id: 0, stencil_gen: 0, scale: 1 };
const OTHER_KEY: DsKey = DsKey { depth_id: OTHER, depth_gen: 1, stencil_id: 0, stencil_gen: 0, scale: 1 };

/// The back end up over the stand-in OSAL, `case` run, the back end down.
fn with_init<R>(case: impl FnOnce() -> R) -> R {
    // SAFETY: the caller holds SERIAL; the tables are statics.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        assert_eq!(gc_init(), 0, "gc_init");
    }
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    gc_shutdown();
    let faults = with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

#[test]
fn the_answer_is_yes_only_under_the_switch_for_a_live_plane_the_surface_covers() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _off = ViewOff;
    with_init(|| {
        let ask = |id, w, h| {
            let a = depth_sample::gc_depth_sampled(id, w, h);
            assert_eq!(yakogl::backend::samples_rendered_depth(id, w, h), a, "the shared answer is the GC7000's");
            a
        };
        assert_eq!(depth_sample::view(), View::Off, "off by default");
        /* a 64x64 surface whose depth plane a draw wrote */
        ds_cache::select(KEY, 64, 64, 0, 1, None, |_| false, || 0).expect("a surface");
        assert!(!ask(ID, 64, 64), "a plane nothing wrote yet");
        ds_cache::drawn(KEY, [true, false], [None, None]);
        assert!(!ask(ID, 64, 64), "the switch is off: every GL use");
        let waits0 = waits();

        assert_eq!(depth_sample::set_view(1), 0);
        assert_eq!(depth_sample::view(), View::D24X8);
        assert!(ask(ID, 64, 64), "the texture the surface was made for");
        assert!(ask(ID, 60, 50) && ask(ID, 1, 1), "a smaller texture of the same object");
        assert!(!ask(ID, 65, 64) && !ask(ID, 64, 65), "a texture the surface does not cover");
        assert!(!ask(OTHER, 64, 64) && !ask(0, 64, 64), "another object, and no object");
        assert_eq!(
            depth_sample::surface(ID, 64, 64).map(|(_, stride)| stride),
            Some(256),
            "the surface's row stride: 64 pixels aligned to a supertile, 4 bytes each"
        );

        /* a plane that holds a whole clear only has no words behind it */
        ds_cache::select(OTHER_KEY, 64, 64, 0, 2, None, |_| false, || 0).expect("a second surface");
        ds_cache::drawn(OTHER_KEY, [false, false], [Some(1.0f32.to_bits()), None]);
        assert!(!ask(OTHER, 64, 64), "a whole clear's value only");
        ds_cache::drawn(OTHER_KEY, [true, false], [Some(1.0f32.to_bits()), None]);
        assert!(ask(OTHER, 64, 64), "written over the clear: Live");

        assert_eq!(depth_sample::set_view(2), 1);
        assert!(ask(ID, 64, 64), "the raw view answers the same");
        assert_eq!(waits(), waits0, "no answer waited");

        /* the descriptor's refusals */
        let bound =
            GlesTextureBind { depth_id: ID, width: 64, height: 64, format: GLES_TEXFMT_R32F, ..GlesTextureBind::ZERO };
        assert!(depth_sample::descriptor(&bound).is_ok_and(|v| v.is_some()), "a view for the live plane");
        let gone = GlesTextureBind { depth_id: 0x4000_0070, ..bound };
        assert_eq!(depth_sample::descriptor(&gone), Err(-yakogl::util::ENOTSUP), "no surface: the draw is refused");
        let texels = GlesTextureBind { depth_id: 0, ..bound };
        assert_eq!(depth_sample::descriptor(&texels), Ok(None), "no depth_id: the texels are the texture");
        assert_eq!(depth_sample::set_view(0), 2);
        assert_eq!(depth_sample::descriptor(&bound), Err(-yakogl::util::ENOTSUP), "the switch turned off in between");
        assert!(!ask(ID, 64, 64), "off again");
    });
}

/* ---- test 2: the descriptor, through the GL core --------------------------------- */

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

/// A program of the two sources with `attribs` bound in order, linked.
fn program(vs: &str, fs: &str, attribs: &[&core::ffi::CStr]) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        for (i, a) in attribs.iter().enumerate() {
            glBindAttribLocation(p, i as GLuint, a.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

const RAMP_VS: &str = "attribute vec3 a_pos;\nvoid main() { gl_Position = vec4(a_pos, 1.0); }\n";
const RAMP_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.0, 0.0, 1.0, 1.0); }\n";
/// Case 177's ramp (`zonly_ramp`): depth (x + y + 1) / 128 at GL pixel (x, y) of a 64x64 target.
static RAMP: [f32; 18] =
    [-1.0, -1.0, -1.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0];

const SAMPLE_VS: &str =
    "attribute vec4 a_pos;\nattribute vec2 a_uv;\nvarying vec2 v_uv;\nvoid main() { gl_Position = a_pos; v_uv = a_uv; }\n";
const SAMPLE_FS: &str = "precision highp float;\nuniform sampler2D u_tex;\nvarying vec2 v_uv;\nvoid main() { gl_FragColor = texture2D(u_tex, v_uv); }\n";
/// A full-window quad: x, y, z, w, u, v.
static QUAD: [f32; 36] = [
    -1.0, -1.0, 0.0, 1.0, 0.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, -1.0, -1.0, 0.0, 1.0,
    0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, -1.0, 1.0, 0.0, 1.0, 0.0, 1.0,
];

/// The 64x64 2D texture of the current context.
fn texture() -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref()
        .textures
        .iter()
        .find(|t| t.used && t.width == 64 && t.height == 64 && t.target == GL_TEXTURE_2D)
        .expect("the texture")
}

#[test]
fn the_descriptor_points_the_texture_unit_at_the_surface_under_the_switch() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _off = ViewOff;
    with_init(|| {
        // SAFETY: the back end is up; a fresh 64x64 context.
        let ctx = unsafe {
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null(), "context pool exhausted");
            c
        };
        let r = std::panic::catch_unwind(descriptor_case);
        // SAFETY: the context made above.
        unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
        r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    });
}

fn descriptor_case() {
    const FBO_W: u32 = 64;
    /* the depth texture alone in a framebuffer, as glmark2's shadow pass has it */
    let mut tex: GLuint = 0;
    let mut fbo: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            /* the sized format ES 3.0 makes depth-renderable; the unsized one
             * under GL_OES_depth_texture on the ES 2.0 report (gl_test.c
             * depth_target_format) */
            if yakogl::REPORT_ES2 { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 } as GLint,
            FBO_W as GLsizei,
            FBO_W as GLsizei,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "a depth-only framebuffer");
    }
    no_error("the depth texture and its framebuffer");
    let t = texture();
    assert_eq!(t.texfmt, GLES_TEXFMT_R32F, "the GL core keeps a depth texture's texels as R32F");
    let (texels_gpu, serial) = (t.mem.gpu, t.serial);
    let id = ds_object_id(serial, true);

    /* the ramp, drawn and finished: the kick that rendered the depth */
    let ramp = program(RAMP_VS, RAMP_FS, &[c"a_pos"]);
    // SAFETY: plain GL calls; RAMP is a static.
    unsafe {
        glUseProgram(ramp);
        glViewport(0, 0, FBO_W as GLsizei, FBO_W as GLsizei);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, RAMP.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    no_error("the ramp");
    assert!(depth_sample::surface(id, FBO_W, FBO_W).is_none(), "the switch is off: no surface is offered");
    depth_sample::set_view(1);
    let (lod0, stride) = depth_sample::surface(id, FBO_W, FBO_W).expect("the ramp's surface, Live");
    assert_eq!(stride, FBO_W * 4, "64 pixels: one supertile row");
    let (base, size) = allocation_of(lod0);
    let guard = (lod0 - base) as usize;
    assert_eq!(size, (stride * FBO_W) as usize + 2 * guard, "LOD 0 is `guard` bytes into the surface's own allocation");
    assert_ne!(lod0, texels_gpu, "the surface is not the texels");

    /* the sampling draw over the window */
    let sample = program(SAMPLE_VS, SAMPLE_FS, &[c"a_pos", c"a_uv"]);
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 64, 64);
        glUseProgram(sample);
        glUniform1i(glGetUniformLocation(sample, c"u_tex".as_ptr()), 0);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glEnableVertexAttribArray(0);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 24, QUAD.as_ptr() as *const c_void);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 24, QUAD.as_ptr().add(4) as *const c_void);
    }
    no_error("the sampling program");

    /* one sampling draw under `mode`, submitted: the waits and the CPU
     * reads of the surface it made, and slot 0's state */
    let sample_under = |mode: i32| -> (usize, usize, Slot0) {
        depth_sample::set_view(mode);
        let (w0, r0, k0) = (waits(), invalidates_of(size), kicks());
        // SAFETY: plain GL calls.
        unsafe {
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFlush();
        }
        no_error("the sampling draw");
        assert!(kicks() > k0, "the draw was submitted");
        let (waited, read) = (waits() - w0, invalidates_of(size) - r0);
        let s = Slot0::last();
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        (waited, read, s)
    };

    let linear_2d = |format: u32| {
        texdesc_type(GC_TEX_TYPE_2D) | texdesc_format(format) | texdesc_addressing_mode(GC_TEX_ADDRESSING_LINEAR)
    };
    let swizzle = |r: u32, g: u32, b: u32, a: u32| {
        texdesc_swizzle_r(r) | texdesc_swizzle_g(g) | texdesc_swizzle_b(b) | texdesc_swizzle_a(a)
    };
    let sampler = samp_uwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
        | samp_vwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
        | samp_wwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
        | samp_min(GC_TEX_FILTER_NEAREST)
        | samp_mip(GC_TEX_FILTER_NONE)
        | samp_mag(GC_TEX_FILTER_NEAREST);

    /* the common words of both views: the surface, linear, one level */
    let surface_words = |s: &Slot0, what: &str| {
        assert_eq!(s.dw(GC_TEXDESC_LINEAR_STRIDE), stride, "{what}: LINEAR_STRIDE is the surface's row stride");
        assert_eq!(s.dw(GC_TEXDESC_SIZE), texdesc_size_width(FBO_W) | texdesc_size_height(FBO_W), "{what}: SIZE");
        assert_eq!(s.dw(GC_TEXDESC_BASELOD), texdesc_baselod_base(0) | texdesc_baselod_max(0), "{what}: one level");
        assert_eq!(s.dw(GC_TEXDESC_SLICE), stride * FBO_W, "{what}: SLICE is stride x height");
        assert_eq!(s.dw(GC_TEXDESC_3D_CONFIG), texdesc_3d_depth(1), "{what}: depth 1");
        assert_eq!(s.dw(GC_TEXDESC_CONFIG2), GC_TEXDESC_CONFIG2_BASE, "{what}: CONFIG2");
        assert_eq!(u64::from(s.dw(texdesc_lod_addr(0))), lod0, "{what}: LOD 0 is the surface's first pixel word");
        assert_eq!(s.dw(texdesc_lod_addr(1)), 0, "{what}: no level 1");
        assert_eq!(s.samp_ctrl0 & 0x7fff, sampler, "{what}: NEAREST, CLAMP_TO_EDGE, no mip");
    };

    /* knob 1: Mesa's D24X8, depth in X, (d, 0, 0, 1) on ES 3.0 and (d, d, d, 1)
     * under GL_OES_depth_texture on the ES 2.0 report, INT_FILTER off */
    let (waited, read, s) = sample_under(1);
    assert_eq!((waited, read), (0, 0), "D24X8: the sampling draw neither waited nor read the surface on the CPU");
    assert_eq!(s.dw(GC_TEXDESC_CONFIG0), linear_2d(GC_TEX_FORMAT_D24X8), "D24X8: CONFIG0 (type, format 0x11, LINEAR)");
    let dd = if yakogl::REPORT_ES2 { GC_SWIZZLE_R } else { GC_SWIZZLE_0 };
    assert_eq!(
        s.dw(GC_TEXDESC_CONFIG1),
        texdesc_format_ext(0) | swizzle(GC_SWIZZLE_R, dd, dd, GC_SWIZZLE_1) | texdesc_halign(0),
        "D24X8: CONFIG1 (FORMAT_EXT 0, the depth swizzle of this report, HALIGN 0)"
    );
    assert_eq!(
        s.samp_ctrl0 & GC_SAMP_INT_FILTER,
        0,
        "D24X8: INT_FILTER off (texture_use_int_filter is false for depth)"
    );
    surface_words(&s, "D24X8");

    /* knob 2: the words as A8R8G8B8 texels, identity swizzle, INT_FILTER on */
    let (waited, read, s) = sample_under(2);
    assert_eq!((waited, read), (0, 0), "raw: no wait and no CPU read either");
    assert_eq!(s.dw(GC_TEXDESC_CONFIG0), linear_2d(GC_TEX_FORMAT_A8R8G8B8), "raw: CONFIG0 (A8R8G8B8, LINEAR)");
    assert_eq!(
        s.dw(GC_TEXDESC_CONFIG1),
        texdesc_format_ext(0) | swizzle(GC_SWIZZLE_R, GC_SWIZZLE_G, GC_SWIZZLE_B, GC_SWIZZLE_A) | texdesc_halign(0),
        "raw: CONFIG1 (identity swizzle: the word's bytes as they are)"
    );
    assert_eq!(s.samp_ctrl0 & GC_SAMP_INT_FILTER, GC_SAMP_INT_FILTER, "raw: INT_FILTER on, a unorm8 view");
    surface_words(&s, "raw");

    /* knob 0: today's path, the texels, resolved by the CPU (a read of the
     * whole surface mapping; the stand-in completes every kick at once, so
     * there is no kick left to wait for) */
    let (_, read, s) = sample_under(0);
    assert_eq!(read, 1, "off: the CPU resolve read the surface");
    assert_eq!(u64::from(s.dw(texdesc_lod_addr(0))), texels_gpu, "off: LOD 0 is the texture's own R32F texels");
    assert_ne!(s.dw(GC_TEXDESC_CONFIG0), linear_2d(GC_TEX_FORMAT_D24X8), "off: not the depth format");
    assert_ne!(s.dw(GC_TEXDESC_LINEAR_STRIDE), 0, "off: the texels' pitch");
}
