// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the four 32-bit-channel integer colour targets, R32I,
//! R32UI, RG32I and RG32UI, on the vivante host back end (item
//! gc-int32-rt-test of plan #335; docs/es3/int-rt-gc.md, the table and "The
//! 32-bit-channel formats").
//!
//! The PE has no 32-bit integer format: R32I/UI are drawn as
//! `PE_FORMAT_R32F` and RG32I/UI as `PE_FORMAT_G32R32F`, both under the
//! output mode `UIF32`, which stores the register's 32 bits as they are.
//! RG32I/UI have 8-byte pixels and so render into the 4x4-tiled shadow of
//! `src/vivante/wide_target.rs` (`0087345f`); R32I/UI are LINEAR targets at
//! their own pitch. What this file pins, for each of the four, today:
//!
//! - the framebuffer is complete with the format as a renderbuffer colour
//!   attachment. A **texture** of the format is stored (the GC
//!   mask holds the four classes, gc-r32-flip) and, since the GL core's
//!   `integer_texture_surface` has rows for them (gl-r32-surface, `6c5b004f`),
//!   it is a GL_FRAMEBUFFER_COMPLETE colour attachment too, and a
//!   glClearBuffer{i,ui}v into it reads back bit-exact (RC17,
//!   fbo.completeness `renderable.texture.*32*`);
//! - a draw and a glClearBuffer{i,ui}v emit the whole PE_COLOR_FORMAT word
//!   (FORMAT_EXT, FORMAT_MASK, FORMAT_EXT's own write mask clear; a draw
//!   0x14000710 / 0x15000710, COMPONENTS without A and so no OVERWRITE; a
//!   clear 0x14010f10 / 0x15010f10, as the back end writes an alpha-less
//!   target's A on a clear), OUTPUT_MODE0 `UIF32`, no unorm saturate, no
//!   blend with GL_BLEND on, and the surface words (linear at the even-padded
//!   pitch, or the shadow's);
//! - the read-back of a constant integer colour is bit-exact through
//!   glReadPixels(GL_RGBA_INTEGER), with values above 2^24 (which a float
//!   path would round) and negative ones.
//!
//! The GPU is modelled as in tests/es3_int_rt_mrt_gc.rs: the stand-in OSAL's
//! submit stores the colour into the colour surface the stream names
//! (`PE_PIPE_COLOR_ADDR`, `PE_COLOR_STRIDE`), linear for a 4-byte target and
//! in 4x4 tiles for an 8-byte one (the shadow). That checks the storage
//! layout, the copy-back and the read path, not that the PE stores the
//! integers as `UIF32` says: that is the board's (docs/es3/int-rt-gc.md,
//! "for the board to confirm").
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int32_rt_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{pe_color_format, GC_INTEGER_RT_FORMATS, GC_INT_RT_64BPP};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;
use yakogl::vivante::shader::ps_color_output_mode;

/// The online compiler's store is one static for the process: every test
/// takes this one lock (as tests/es3_int_rt_gc.rs).
static STORE: Mutex<()> = Mutex::new(());

/// The value of `#define <name>` in driver/src/vivante/hw/state_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/state_3d.xml.h");
    let text = std::fs::read_to_string(path).expect("the vendored rnndb header");
    let line = text
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(name) && l.starts_with("#define"))
        .unwrap_or_else(|| panic!("{name} is in state_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

/* ---- the four formats ------------------------------------------------------ */

/// A 32-bit-channel integer target: the GL format, its surface, the
/// fragment output type, the PE format (rnndb name), the channel count and
/// whether signed.
struct Int32 {
    internal: GLenum,
    surf: u8,
    ty: &'static str,
    pe: &'static str,
    channels: usize,
    signed: bool,
}

const INT32: [Int32; 4] = [
    Int32 { internal: GL_R32I, surf: GLES_SURF_R32I, ty: "ivec4", pe: "PE_FORMAT_R32F", channels: 1, signed: true },
    Int32 { internal: GL_R32UI, surf: GLES_SURF_R32UI, ty: "uvec4", pe: "PE_FORMAT_R32F", channels: 1, signed: false },
    Int32 {
        internal: GL_RG32I,
        surf: GLES_SURF_RG32I,
        ty: "ivec4",
        pe: "PE_FORMAT_G32R32F",
        channels: 2,
        signed: true,
    },
    Int32 {
        internal: GL_RG32UI,
        surf: GLES_SURF_RG32UI,
        ty: "uvec4",
        pe: "PE_FORMAT_G32R32F",
        channels: 2,
        signed: false,
    },
];

/// The constant colour of the draws, as the shader writes it and as its
/// bits: above 2^24 (a float path rounds 16777217 to 16777216) and, for the
/// signed formats, negative; past 2^31 for the unsigned ones.
const SIGNED: [i32; 4] = [-16_777_217, 2_000_000_001, -5, 9];
const UNSIGNED: [u32; 4] = [4_000_000_001, 16_777_217, 5, 9];

impl Int32 {
    fn name(&self) -> &'static str {
        gles_surf_name(self.surf)
    }

    fn bytes(&self) -> u32 {
        4 * self.channels as u32
    }

    /// The colour's four words.
    fn colour(&self) -> [u32; 4] {
        if self.signed {
            SIGNED.map(|v| v as u32)
        } else {
            UNSIGNED
        }
    }

    /// A fragment shader writing the constant colour.
    fn fs(&self) -> String {
        let one = if self.signed {
            format!("ivec4({}, {}, {}, {})", SIGNED[0], SIGNED[1], SIGNED[2], SIGNED[3])
        } else {
            format!("uvec4({}u, {}u, {}u, {}u)", UNSIGNED[0], UNSIGNED[1], UNSIGNED[2], UNSIGNED[3])
        };
        format!("#version 300 es\nlayout(location = 0) out highp {} o;\nvoid main() {{ o = {one}; }}\n", self.ty)
    }

    /// The pixel the modelled PE stores for `c`: its first `channels` words.
    fn pixel(&self, c: [u32; 4]) -> Vec<u8> {
        c[..self.channels].iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// What glReadPixels(GL_RGBA_INTEGER) returns for a pixel stored from `c`:
    /// the channels the format has, then 0 and alpha 1 (ES 3.0 4.3.1).
    fn read_as(&self, c: [u32; 4]) -> [u32; 4] {
        let mut v = [0, 0, 0, 1];
        v[..self.channels].copy_from_slice(&c[..self.channels]);
        v
    }

    /// The whole PE_COLOR_FORMAT word of a one-target draw or clear. A draw
    /// into a target without alpha keeps A out of COMPONENTS (0x7) and so
    /// loses OVERWRITE (as tests/es3_int_rt_gc.rs); a clear writes the
    /// alpha-less target's A too (`clear_alpha`, src/vivante/backend.rs), so
    /// COMPONENTS 0xf and OVERWRITE.
    fn color_format_word(&self, is_clear: bool) -> u32 {
        let pe = if self.channels == 1 { 0x1400_0000 } else { 0x1500_0000 };
        pe | if is_clear { 0x0001_0f10 } else { 0x0000_0710 }
    }
}

/* ---- the stand-in OSAL: host memory, every stream, a model of the PE ------ */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// While set, every kick that names a colour surface gets `(w, h, bytes
    /// a pixel, pixel)` stored into it: linear at the stream's stride for 4
    /// bytes a pixel, 4x4-tiled for 8 (the shadow).
    model: Option<(u32, u32, u32, Vec<u8>)>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

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
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| p.allocs.remove(&m.gpu));
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// Byte offset of pixel (x, y) in a 4x4-tiled surface `tw` pixels wide at 8
/// bytes a pixel: written here from the hardware's layout, not taken from
/// `wide_target.rs`, so that a wrong copy there shows.
fn tiled_byte(x: u32, y: u32, tw: u32) -> usize {
    (((y / 4) * (tw / 4) + x / 4) * 16 + (y % 4) * 4 + x % 4) as usize * 8
}

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let stream = words.to_vec();
        if let Some((w, h, bpp, pixel)) = p.model.clone() {
            let ws = writes(&stream);
            let addr = ws.iter().rev().find(|(r, _)| *r == pe_pipe_color_addr(0)).map(|(_, v)| *v as u64);
            let stride = ws.iter().rev().find(|(r, _)| *r == rnndb("VIVS_PE_COLOR_STRIDE")).map(|(_, v)| *v);
            if let (Some(addr), Some(stride)) = (addr, stride) {
                if let Some((base, mem)) = p.allocs.range_mut(..=addr).next_back().map(|(g, m)| (*g, m)) {
                    let off = (addr - base) as usize;
                    // SAFETY: the surface holds the target's pixels from `off` on.
                    let bytes = unsafe { core::slice::from_raw_parts_mut(mem.as_mut_ptr() as *mut u8, mem.len() * 8) };
                    for y in 0..h {
                        for x in 0..w {
                            let at = off
                                + if bpp == 8 { tiled_byte(x, y, stride / 8) } else { (y * stride + x * bpp) as usize };
                            bytes[at..at + bpp as usize].copy_from_slice(&pixel);
                        }
                    }
                }
            }
        }
        p.kicks.push(stream);
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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| p.model = None);
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    with_platform(|p| {
        /* leaked, not freed, as tests/gc_npot_repeat.rs: an address a static
         * still names must stay memory */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL helpers ------------------------------------------------------------ */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
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

fn program(fs: &str) -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

fn renderbuffer(internal: GLenum, w: u32, h: u32) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, internal, w as GLsizei, h as GLsizei);
    }
    assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(0x{internal:x}, {w}x{h})");
    rb
}

/// A framebuffer with `rb` at colour attachment 0, its one draw buffer; the
/// status it reports.
fn framebuffer(rb: GLuint) -> GLenum {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    let status = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        let buf = GL_COLOR_ATTACHMENT0;
        glDrawBuffers(1, &buf);
        glCheckFramebufferStatus(GL_FRAMEBUFFER)
    };
    assert_eq!(err(), GL_NO_ERROR, "framebuffer");
    status
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// Draw the quad over the `w` x `h` viewport with `fs` and GL_BLEND on,
/// finish, and return the error and the writes of the streams submitted since.
fn draw(fs: &str, w: u32, h: u32) -> (GLenum, Vec<(u32, u32)>) {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    let e = unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glUseProgram(program(fs));
        /* ignored for an integer buffer (ES 3.0 4.1.8) */
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        let e = glGetError();
        glFinish();
        e
    };
    (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
}

/// glClearBufferiv / glClearBufferuiv of draw buffer 0 with `c`, finished;
/// the error and the writes of the streams submitted since.
fn clear(t: &Int32, c: [u32; 4]) -> (GLenum, Vec<(u32, u32)>) {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: four values; plain GL calls.
    let e = unsafe {
        if t.signed {
            glClearBufferiv(GL_COLOR, 0, c.as_ptr() as *const GLint);
        } else {
            glClearBufferuiv(GL_COLOR, 0, c.as_ptr());
        }
        let e = glGetError();
        glFinish();
        e
    };
    (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
}

fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
    w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
}

/// Bytes untiled out of shadows since init.
fn untiled() -> u64 {
    yakogl::vivante::wide_target::counts().1
}

/// The whole `w` x `h` target read back as GL_RGBA_INTEGER, the one integer
/// read format ES 3.0 always takes, as INT or UNSIGNED_INT.
fn read(t: &Int32, w: u32, h: u32) -> Vec<u32> {
    let mut buf = vec![0xdead_beefu32; (w * h) as usize * 4];
    let ty = if t.signed { GL_INT } else { GL_UNSIGNED_INT };
    // SAFETY: the buffer holds w * h pixels of four 32-bit words.
    unsafe { glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA_INTEGER, ty, buf.as_mut_ptr() as *mut _) };
    assert_eq!(err(), GL_NO_ERROR, "{}: glReadPixels", t.name());
    buf
}

fn assert_read_back(t: &Int32, w: u32, h: u32, c: [u32; 4]) {
    let got = read(t, w, h);
    let want = t.read_as(c);
    for (i, px) in got.chunks(4).enumerate() {
        assert_eq!(px, want, "{} {w}x{h}: pixel ({}, {})", t.name(), i as u32 % w, i as u32 / w);
    }
}

/// The words of a one-target draw or clear into `t` at `w` x `h`: the whole
/// PE_COLOR_FORMAT word, OUTPUT_MODE0 UIF32, no saturate, no blend, and the
/// surface: linear at the storage pitch (`draw_pitch`: the width rounded up
/// to even, 4 bytes a pixel) for R32I/UI, the tiled shadow
/// (page-aligned, padded stride, SINGLE_BUFFER(2), the target's bytes copied
/// back) for RG32I/UI.
fn assert_words(t: &Int32, is_clear: bool, ws: &[(u32, u32)], w: u32, h: u32, copied_back: u64) {
    let name = t.name();
    let color = last(ws, GC_PE_COLOR_FORMAT).unwrap_or_else(|| panic!("{name}: nothing reached the PE"));
    let field = |mask: &str, v: u32| (v << rnndb(mask).trailing_zeros()) & rnndb(mask);
    let word = field("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK", rnndb(t.pe))
        | rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_MASK")
        | field("VIVS_PE_COLOR_FORMAT_COMPONENTS__MASK", if is_clear { 0xf } else { 0x7 })
        | if is_clear { rnndb("VIVS_PE_COLOR_FORMAT_OVERWRITE") } else { 0 };
    assert_eq!(color, word, "{name}: PE_COLOR_FORMAT from the rnndb fields");
    assert_eq!(color, t.color_format_word(is_clear), "{name}: PE_COLOR_FORMAT {color:#010x}");
    assert_eq!(color & rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_EXT_MASK"), 0, "{name}: FORMAT_EXT written");
    let tiled = rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED") | rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED_NEW");
    assert_eq!(color & tiled, 0, "{name}: no SUPER_TILED bit");

    let ext = last(ws, GC_PS_CONTROL_EXT).expect("PS_CONTROL_EXT");
    assert_eq!(ext & 0x7, rnndb("COLOR_OUTPUT_MODE_UIF32"), "{name}: OUTPUT_MODE0 UIF32");
    let ctl = last(ws, GC_PS_CONTROL).expect("PS_CONTROL");
    assert_eq!(ctl & GC_PS_CONTROL_SATURATE_RT0, 0, "{name}: an integer is not saturated");
    assert_eq!(last(ws, GC_PE_ALPHA_CONFIG), Some(0), "{name}: never blended");

    let logic = last(ws, GC_PE_LOGIC_OP).expect("PE_LOGIC_OP");
    assert_eq!(logic & rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER_MASK"), 0, "{name}: SINGLE_BUFFER written");
    let single_mask = rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER__MASK");
    let single = (logic & single_mask) >> single_mask.trailing_zeros();
    let stride = last(ws, rnndb("VIVS_PE_COLOR_STRIDE")).expect("PE_COLOR_STRIDE");
    let addr = last(ws, pe_pipe_color_addr(0)).expect("PE_PIPE_COLOR_ADDR(0)");
    assert_eq!(last(ws, pe_pipe_color_addr(1)), Some(addr), "{name}: both pipes, one surface");
    if t.bytes() == 8 {
        let (pw, _) = yakogl::vivante::wide_target::padded(w, h);
        assert_eq!(single, 2, "{name}: SINGLE_BUFFER(2), the tiled 64-bit value");
        assert_eq!(stride, pw * 8, "{name} {w}x{h}: the shadow's padded stride");
        assert_eq!(stride, yakogl::vivante::wide_target::stride(w), "{name}");
        assert_eq!(addr & 0xfff, 0, "{name}: the shadow starts on a page");
        assert_eq!(copied_back, u64::from(w * h * 8), "{name} {w}x{h}: the shadow went back into the storage");
    } else {
        assert_eq!(single, 1, "{name}: SINGLE_BUFFER(1), linear");
        assert_eq!(stride, ((w + 1) & !1) * 4, "{name} {w}x{h}: linear pitch, the width rounded up to even");
        assert_eq!(copied_back, 0, "{name}: no shadow");
    }
}

/// Tile-aligned, and a size that pads both the 4x4 tile and the shadow's
/// 16-pixel row.
const SIZES: [(u32, u32); 2] = [(16, 16), (17, 13)];

/* ---- the tables ------------------------------------------------------------ */

/// The four are drawn (in `GC_INTEGER_RT_FORMATS`, `rt_format_drawable`),
/// as R32F / G32R32F under UIF32, and only RG32I/UI take the 64-bit shadow.
#[test]
fn the_four_are_drawable_as_the_float_pe_formats_under_uif32() {
    for t in &INT32 {
        let name = t.name();
        let bit = 1u32 << (t.surf - GLES_SURF_INT_FIRST);
        assert_ne!(GC_INTEGER_RT_FORMATS & bit, 0, "{name}: drawn");
        assert!(rt_format_drawable(t.surf), "{name}");
        assert_eq!(pe_color_format(t.surf), (rnndb(t.pe), true), "{name}: {} in FORMAT_EXT", t.pe);
        assert_eq!(ps_color_output_mode(t.surf), rnndb("COLOR_OUTPUT_MODE_UIF32"), "{name}");
        assert_eq!(gles_surf_bytes_per_pixel(t.surf), t.bytes(), "{name}: the pixel size of {}", t.pe);
        assert_eq!(GC_INT_RT_64BPP & bit != 0, t.bytes() == 8, "{name}: the 64-bit shadow set");
        let layout = gles_surf_int_layout(t.surf).expect("an integer layout");
        assert_eq!(usize::from(layout.channels), t.channels, "{name}");
    }
}

/* ---- completeness ---------------------------------------------------------- */

/// Each of the four as a renderbuffer colour attachment: complete.
#[test]
fn a_renderbuffer_of_each_is_a_complete_colour_attachment() {
    for t in &INT32 {
        on_gc7000(|| {
            let rb = renderbuffer(t.internal, 16, 16);
            assert_eq!(framebuffer(rb), GL_FRAMEBUFFER_COMPLETE, "{}", t.name());
        });
    }
}

/// A texture of the four is stored (GC_SAMPLED_TEXFMTS holds their classes
/// since gc-r32-flip; `es3_storage_or_refuse`, src/gl/texture.rs), so
/// glTexImage2D and glTexStorage2D raise nothing and the attachment is made.
/// The framebuffer is complete: `integer_texture_surface`
/// (src/gl/framebuffer.rs) has a surface for each of the four (RC17, dEQP
/// fbo.completeness `renderable.texture.*32*`). A glClearBuffer{i,ui}v into the
/// texture-backed target (values above 2^24, negative for the signed formats)
/// reads back bit for bit through glReadPixels(GL_RGBA_INTEGER).
#[test]
fn a_texture_of_each_is_stored_and_a_complete_attachment() {
    let (w, h) = (16, 16);
    for t in &INT32 {
        on_gc7000(|| {
            let name = t.name();
            let (format, ty) = (
                if t.channels == 1 { GL_RED_INTEGER } else { GL_RG_INTEGER },
                if t.signed { GL_INT } else { GL_UNSIGNED_INT },
            );
            let mut tex = [0; 2];
            // SAFETY: plain GL calls; no pixels.
            unsafe {
                glGenTextures(2, tex.as_mut_ptr());
                glBindTexture(GL_TEXTURE_2D, tex[0]);
                glTexImage2D(GL_TEXTURE_2D, 0, t.internal as GLint, 16, 16, 0, format, ty, core::ptr::null());
                assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glTexImage2D");
                glBindTexture(GL_TEXTURE_2D, tex[1]);
                glTexStorage2D(GL_TEXTURE_2D, 1, t.internal, 16, 16);
                assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glTexStorage2D");
                let mut fb = 0;
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex[1], 0);
                assert_eq!(glGetError(), GL_NO_ERROR, "{name}: the attachment itself");
                let buf = GL_COLOR_ATTACHMENT0;
                glDrawBuffers(1, &buf);
                assert_eq!(
                    glCheckFramebufferStatus(GL_FRAMEBUFFER),
                    GL_FRAMEBUFFER_COMPLETE,
                    "{name}: a texture has its own surface class (RC17)"
                );
            }
            let c = t.colour();
            with_platform(|p| p.model = Some((w, h, t.bytes(), t.pixel(c))));
            let (e, _) = clear(t, c);
            assert_eq!(e, GL_NO_ERROR, "{name}: glClearBuffer into the texture");
            assert_read_back(t, w, h, c);
        });
    }
}

/* ---- draws and clears ------------------------------------------------------ */

/// A draw of a constant colour into each: the words, and the colour read
/// back bit for bit from where the (modelled) PE stored it.
#[test]
fn a_draw_into_each_emits_its_words_and_reads_back_bit_exact() {
    for t in &INT32 {
        for (w, h) in SIZES {
            on_gc7000(|| {
                let rb = renderbuffer(t.internal, w, h);
                assert_eq!(framebuffer(rb), GL_FRAMEBUFFER_COMPLETE);
                let c = t.colour();
                with_platform(|p| p.model = Some((w, h, t.bytes(), t.pixel(c))));
                let untiled0 = untiled();
                let (e, ws) = draw(&t.fs(), w, h);
                assert_eq!(e, GL_NO_ERROR, "{} {w}x{h}: drawn", t.name());
                assert_words(t, false, &ws, w, h, untiled() - untiled0);
                assert_read_back(t, w, h, c);
            });
        }
    }
}

/// glClearBuffer{i,ui}v of each with values above 2^24 and negative ones:
/// the clear quad takes the same words as a draw, its constant is the four
/// raw values, and the colour reads back bit for bit.
#[test]
fn a_clear_of_each_emits_its_words_and_reads_back_bit_exact() {
    let colours: [[u32; 4]; 3] = [
        [(-16_777_217i32) as u32, 16_777_217, 0x8000_0001, 0x1234_5678],
        [0xffff_fff0, (-2i32) as u32, 7, 0],
        [0x7fff_ffff, 0x0100_0003, 0xdead_beef, 1],
    ];
    for t in &INT32 {
        for (w, h) in SIZES {
            for c in colours {
                on_gc7000(|| {
                    let name = t.name();
                    let rb = renderbuffer(t.internal, w, h);
                    assert_eq!(framebuffer(rb), GL_FRAMEBUFFER_COMPLETE);
                    with_platform(|p| p.model = Some((w, h, t.bytes(), t.pixel(c))));
                    let untiled0 = untiled();
                    let (e, ws) = clear(t, c);
                    assert_eq!(e, GL_NO_ERROR, "{name} {w}x{h}: glClearBuffer");
                    assert_words(t, true, &ws, w, h, untiled() - untiled0);
                    let ps: Vec<u32> = ws
                        .iter()
                        .filter(|(r, _)| (GC_SH_HALTI5_UNIFORMS_PS..GC_SH_HALTI5_UNIFORMS_PS + 0x2000).contains(r))
                        .map(|(_, v)| *v)
                        .collect();
                    assert!(ps.windows(4).any(|q| q == c), "{name}: the clear constant is the raw values: {ps:x?}");
                    assert_read_back(t, w, h, c);
                });
            }
        }
    }
}

/// A draw after a clear with the model off: the target keeps the cleared
/// colour (a 64-bit target's storage is tiled back into the shadow first, a
/// 32-bit one is drawn in place), read back bit for bit.
#[test]
fn a_draw_that_stores_nothing_keeps_the_cleared_colour() {
    for t in &INT32 {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let rb = renderbuffer(t.internal, w, h);
            assert_eq!(framebuffer(rb), GL_FRAMEBUFFER_COMPLETE);
            let c = [0x8000_0000, 0x0100_0001, 3, 4];
            with_platform(|p| p.model = Some((w, h, t.bytes(), t.pixel(c))));
            assert_eq!(clear(t, c).0, GL_NO_ERROR);
            with_platform(|p| p.model = None);
            let tiled0 = yakogl::vivante::wide_target::counts().0;
            assert_eq!(draw(&t.fs(), w, h).0, GL_NO_ERROR);
            let tiled_in = yakogl::vivante::wide_target::counts().0 - tiled0;
            let want = if t.bytes() == 8 { u64::from(w * h * 8) } else { 0 };
            assert_eq!(tiled_in, want, "{}: the storage tiled into a shadow", t.name());
            assert_read_back(t, w, h, c);
        });
    }
}
