// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): an RGBA32F, RGBA32UI or RGBA32I texture as a colour
//! attachment is refused cleanly (item gc-w128-fbo of plan #366).
//!
//! The flip (gc-w128-flip) stores the three classes as two G32R32 planes
//! (docs/notes/gc-128bit-texture-sampling.md), but the GC has no 128-bit
//! render target yet: Mesa draws one as two G32R32F hardware targets, and
//! yakogl's pieces of that are inert behind the refusal
//! (docs/notes/gc-128bit-render-targets.md, `GC_INT_RT_128BPP`). RGBA32I and
//! RGBA32UI are colour-renderable in ES 3.0 (table 3.13); RGBA32F is not
//! without EXT_color_buffer_float. A framebuffer with such a texture attached
//! must therefore never be drawn into plane 0 as if it were one G32R32
//! surface. What this file pins, for each class and for three attachments
//! (glFramebufferTexture2D at attachment 0, glFramebufferTextureLayer of a 2D
//! array's layer 1 at attachment 0, and glFramebufferTexture2D at attachment
//! 1 beside an RGBA8 attachment 0):
//!
//! - the attachment call raises nothing, and glCheckFramebufferStatus
//!   answers GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT: `attachment_storage`
//!   (src/gl/framebuffer.rs) finds no colour-renderable storage, since
//!   `rt_format_drawable` refuses `GLES_SURF_RGBA32I/UI` on this back end and
//!   no 16-byte class is in its list of float render classes;
//! - glDrawArrays, glClear and glClearBuffer{f,i,ui}v raise
//!   GL_INVALID_FRAMEBUFFER_OPERATION at the call (ES 3.0 4.4.4.4) and the
//!   glFinish after them raises nothing (until 2026-11-16 glClear and
//!   glClearBufferfv raised it only at that glFinish, and the integer forms
//!   not at all). glReadPixels raises
//!   GL_INVALID_FRAMEBUFFER_OPERATION for RGBA32F, and GL_INVALID_OPERATION
//!   for GL_RGBA_INTEGER from the integer classes (the format check comes
//!   before the status in `read_pixels`). Not one kick is submitted for any
//!   of them, the texture's storage (both planes once the class is stored)
//!   is byte for byte what was uploaded, and the read-back buffer is not
//!   written;
//! - a later draw into the window that samples the texture reads what was
//!   uploaded: the descriptor's level 0 holds the model's texels (both
//!   planes, R,G in plane 0 and B,A in plane 1, once stored as planes).
//!
//! The classes must be in the GC sampled mask for the core to store them.
//! `backend::TEST_EXTRA_SAMPLED` is `#[cfg(test)]`, so an integration test
//! cannot add them (as tests/es3_wide128_planes_gc.rs): until
//! GC_SAMPLED_TEXFMTS holds them (gc-w128-flip) a closed integer class has no
//! storage (glTexImage* is GL_INVALID_OPERATION) and RGBA32F is held as
//! RGBA16F halves; stderr says which state a run checked. The refusals above
//! are the same in both states. Run locally once with the three classes added
//! to GC_SAMPLED_TEXFMTS (not committed): every test passed, with both
//! integer classes stored as planes and sampled back from both, RGBA32F still
//! as halves (`sized_format`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_fbo_gc
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
use yakogl::gl::names::{self, Kind};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

const COMMON_3D: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/common_3d.xml.h"));

/// The value of `#define <name>` in the vendored common_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let line = COMMON_3D
        .lines()
        .find(|l| l.starts_with("#define") && l.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is in common_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

/* ---- the stand-in OSAL (as tests/es3_wide128_planes_gc.rs) -------------- */

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

fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    let b = read_gpu(gpu, n * 4);
    let (words, _) = b.as_chunks::<4>();
    words.iter().map(|c| u32::from_le_bytes(*c)).collect()
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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- GL plumbing -------------------------------------------------------- */

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

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

fn kicks() -> usize {
    with_platform(|p| p.kicks.len())
}

/* ---- the classes, the attachments and the texels ------------------------ */

struct Wide {
    name: &'static str,
    class: u8,
    internal: GLenum,
    format: GLenum,
    ty: GLenum,
    /// The GLSL sampler / output prefix: "", "u" or "i".
    prefix: &'static str,
    /// Mesa's plane word (etnaviv_format.c:229-233).
    plane_ext: &'static str,
}

const WIDE: [Wide; 3] = [
    Wide {
        name: "RGBA32F",
        class: GLES_TEXFMT_RGBA32F,
        internal: GL_RGBA32F,
        format: GL_RGBA,
        ty: GL_FLOAT,
        prefix: "",
        plane_ext: "TEXTURE_FORMAT_EXT_G32R32F",
    },
    Wide {
        name: "RGBA32UI",
        class: GLES_TEXFMT_RGBA32UI,
        internal: GL_RGBA32UI,
        format: GL_RGBA_INTEGER,
        ty: GL_UNSIGNED_INT,
        prefix: "u",
        plane_ext: "TEXTURE_FORMAT_EXT_G32R32I",
    },
    Wide {
        name: "RGBA32I",
        class: GLES_TEXFMT_RGBA32I,
        internal: GL_RGBA32I,
        format: GL_RGBA_INTEGER,
        ty: GL_INT,
        prefix: "i",
        plane_ext: "TEXTURE_FORMAT_EXT_G32R32I",
    },
];

impl Wide {
    fn float(&self) -> bool {
        self.ty == GL_FLOAT
    }

    /// Is the class in the back end's sampled mask (so the GL core stores it)?
    fn open(&self) -> bool {
        (sampled_texfmts() & gles_texfmt_bit(self.class) as u64) != 0
    }
}

/// How the texture is attached.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Attach {
    /// glFramebufferTexture2D of a 2D texture at attachment 0.
    Tex2D,
    /// glFramebufferTextureLayer of layer 1 of a two-layer 2D array at
    /// attachment 0.
    Layer,
    /// glFramebufferTexture2D of a 2D texture at attachment 1, an RGBA8
    /// texture at attachment 0, both draw buffers on.
    Beside,
}

const ATTACHES: [Attach; 3] = [Attach::Tex2D, Attach::Layer, Attach::Beside];

const W: u32 = 13;
const H: u32 = 6;

impl Attach {
    fn layers(self) -> u32 {
        if self == Attach::Layer {
            2
        } else {
            1
        }
    }

    /// The draw buffer the texture is.
    fn buffer(self) -> u32 {
        if self == Attach::Beside {
            1
        } else {
            0
        }
    }
}

/// The four words of texel (x, y) of `layer`: a hash; for RGBA32F a quarter
/// below 256, exact in a half too, so the RGBA16F fallback of a closed class
/// holds the same values.
fn texel(float: bool, layer: u32, x: u32, y: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let mut h = layer.wrapping_mul(0x9e37_79b1)
            ^ x.wrapping_mul(0x85eb_ca6b)
            ^ y.wrapping_mul(0xc2b2_ae35)
            ^ (c as u32).wrapping_mul(0x27d4_eb2f);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *o = if float { ((h & 0x3ff) as f32 * 0.25).to_bits() } else { h };
    }
    out
}

/// The half of an f32 that one holds exactly (a quarter below 256).
fn half(bits: u32) -> u16 {
    let f = f32::from_bits(bits);
    if f == 0.0 {
        return 0;
    }
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    assert!((1..31).contains(&exp) && bits & 0x1fff == 0, "{f} is not exact in a half");
    (((bits >> 16) & 0x8000) | ((exp as u32) << 10) | ((bits >> 13) & 0x3ff)) as u16
}

fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

/// Byte offset of texel (x, y) in a 4x4-tiled image `w` texels wide at `bpp`
/// bytes a texel (docs/notes/gc-128bit-texture-sampling.md).
fn tiled(x: u32, y: u32, w: u32, bpp: u32) -> usize {
    (((y / 4) * (align4(w) / 4) + x / 4) * 16 + (y % 4) * 4 + x % 4) as usize * bpp as usize
}

/// A `W` x `H` texture of the class (a two-layer 2D array for
/// [`Attach::Layer`]) with every texel from [`texel`], nearest-filtered with
/// one level; its name and the error the upload raised.
fn make_texture(c: &Wide, a: Attach) -> (GLuint, GLenum) {
    let layers = a.layers();
    let img: Vec<u32> = (0..layers)
        .flat_map(|l| (0..H).flat_map(move |y| (0..W).flat_map(move |x| texel(c.float(), l, x, y))))
        .collect();
    let target = if a == Attach::Layer { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_2D };
    let mut t = 0;
    // SAFETY: plain GL calls; `img` holds W * H * layers texels of four words.
    let e = unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if a == Attach::Layer {
            glTexImage3D(
                target,
                0,
                c.internal as GLint,
                W as GLsizei,
                H as GLsizei,
                layers as GLsizei,
                0,
                c.format,
                c.ty,
                img.as_ptr().cast(),
            );
        } else {
            glTexImage2D(
                target,
                0,
                c.internal as GLint,
                W as GLsizei,
                H as GLsizei,
                0,
                c.format,
                c.ty,
                img.as_ptr().cast(),
            );
        }
        glGetError()
    };
    (t, e)
}

/// The whole storage of texture `name` and its stored class; None while it
/// has no CPU view (no image).
fn storage(name: GLuint) -> Option<(u8, Vec<u8>)> {
    let ctx = yakogl::gl::context::current().expect("a current context");
    let slot = names::slot_of(ctx, Kind::Texture, name)?;
    let t = ctx.texture_object(slot)?;
    if t.mem.cpu.is_null() {
        return None;
    }
    // SAFETY: the texture's own CPU mapping, `size` bytes.
    Some((t.texfmt, unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) }.to_vec()))
}

/// A framebuffer with the texture attached as `a` says; the attachment's
/// error and the status.
fn attach(tex: GLuint, a: Attach) -> (GLenum, GLenum) {
    // SAFETY: plain GL calls; no pixels for the RGBA8 texture.
    unsafe {
        let mut fb = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match a {
            Attach::Tex2D => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0),
            Attach::Layer => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1),
            Attach::Beside => {
                let mut rgba8 = 0;
                glGenTextures(1, &mut rgba8);
                glBindTexture(GL_TEXTURE_2D, rgba8);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8 as GLint,
                    W as GLsizei,
                    H as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                assert_eq!(glGetError(), GL_NO_ERROR, "the RGBA8 texture");
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rgba8, 0);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex, 0);
            }
        }
        let e = glGetError();
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
        glDrawBuffers(if a == Attach::Beside { 2 } else { 1 }, bufs.as_ptr());
        glReadBuffer(bufs[a.buffer() as usize]);
        assert_eq!(glGetError(), GL_NO_ERROR, "glDrawBuffers / glReadBuffer");
        (e, glCheckFramebufferStatus(GL_FRAMEBUFFER))
    }
}

/// The fragment shader of a draw into the framebuffer: the class's own
/// output type at the texture's draw buffer, a vec4 at attachment 0 beside it.
fn fs_into(c: &Wide, a: Attach) -> String {
    let v = if c.float() { "vec4(1.5, 2.5, 3.5, 4.5)".into() } else { format!("{}vec4(5, 6, 7, 8)", c.prefix) };
    if a == Attach::Beside {
        format!(
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o0;\nlayout(location = 1) out highp {}vec4 o1;\nvoid main() {{ o0 = vec4(0.5); o1 = {v}; }}\n",
            c.prefix
        )
    } else {
        format!(
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out highp {}vec4 o;\nvoid main() {{ o = {v}; }}\n",
            c.prefix
        )
    }
}

/* ---- the pins ------------------------------------------------------------ */

/// What the refusals did: for glDrawArrays, glClear, glClearBuffer* and
/// glReadPixels, the error after the call and the one a glFinish after it
/// raised.
type Errors = [(GLenum, GLenum); 4];

/// The error of `call`, then the error of a glFinish after it.
fn finished(call: impl FnOnce()) -> (GLenum, GLenum) {
    call();
    let e = err();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    (e, err())
}

/// glDrawArrays, glClear, glClearBuffer{f,i,ui}v and glReadPixels into the
/// bound framebuffer, each finished: their errors. Asserts that not one kick
/// was submitted and that the read-back buffer was not written.
fn refused_calls(c: &Wide, a: Attach, what: &str) -> Errors {
    let k0 = kicks();
    let p = program(&fs_into(c, a));
    let buf = a.buffer() as GLint;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, W as GLsizei, H as GLsizei);
        glUseProgram(p);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
    }
    // SAFETY: plain GL call.
    let draw = finished(|| unsafe { glDrawArrays(GL_TRIANGLE_STRIP, 0, 4) });
    // SAFETY: plain GL call.
    let clear = finished(|| unsafe { glClear(GL_COLOR_BUFFER_BIT) });
    let words = [0x8000_0001u32, 0x0100_0001, 0xdead_beef, 7];
    let floats = [1.5f32, -2.0, 3.25, 4.0];
    // SAFETY: four values each.
    let clear_buffer = finished(|| unsafe {
        match c.ty {
            GL_FLOAT => glClearBufferfv(GL_COLOR, buf, floats.as_ptr()),
            GL_INT => glClearBufferiv(GL_COLOR, buf, words.as_ptr() as *const GLint),
            _ => glClearBufferuiv(GL_COLOR, buf, words.as_ptr()),
        }
    });
    let mut px = vec![0x5a5a_5a5au32; (W * H * 4) as usize];
    let (format, ty) = if c.float() { (GL_RGBA, GL_FLOAT) } else { (GL_RGBA_INTEGER, c.ty) };
    // SAFETY: the buffer holds W * H texels of four words.
    let read =
        finished(|| unsafe { glReadPixels(0, 0, W as GLsizei, H as GLsizei, format, ty, px.as_mut_ptr().cast()) });
    assert_eq!(kicks(), k0, "{what}: a refused call submitted a kick");
    assert!(px.iter().all(|&v| v == 0x5a5a_5a5a), "{what}: glReadPixels wrote the buffer");
    [draw, clear, clear_buffer, read]
}

/// A draw into the window that samples the texture (layer 1 of the array)
/// on unit 0: the level 0 words of the descriptors of slots 0 and 1 and
/// what they address, checked against the model. Planes when the class is
/// stored as itself (R,G in plane 0, B,A in plane 1, both 4x4-tiled at 8
/// bytes a texel and EXT_G32R32*); RGBA16F halves when RGBA32F is held as
/// halves (4x4-tiled, 8 bytes a texel).
fn samples_the_upload(c: &Wide, a: Attach, tex: GLuint, stored: u8, what: &str) {
    let (sampler, target, at) = if a == Attach::Layer {
        ("sampler2DArray", GL_TEXTURE_2D_ARRAY, "ivec3(0, 0, 1)")
    } else {
        ("sampler2D", GL_TEXTURE_2D, "ivec2(0)")
    };
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {p}{sampler};\nuniform {p}{sampler} s;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, {at}, 0)); }}\n",
        p = c.prefix
    );
    let p = program(&fs);
    let k0 = kicks();
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, tex);
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        assert_eq!(glGetError(), GL_NO_ERROR, "{what}: the sampling draw");
        glFinish();
    }
    let ws = with_platform(|p| {
        assert!(p.kicks.len() > k0, "{what}: the sampling draw produced no kick");
        writes(p.kicks.last().unwrap())
    });
    let desc = |slot: u32| -> Option<Vec<u32>> {
        let a = ws.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
        (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
    };
    let d0 = desc(0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let lod0 = |d: &[u32]| u64::from(d[texdesc_lod_addr(0) as usize / 4]);
    let plane = (align4(W) * align4(H) * 8) as u64;
    let layers = a.layers();
    if stored == c.class {
        let d1 = desc(1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
        for (i, d) in [&d0, &d1].into_iter().enumerate() {
            assert_eq!(
                d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f,
                rnndb(c.plane_ext),
                "{what}: plane {i}: {}",
                c.plane_ext
            );
        }
        assert_eq!(lod0(&d1), lod0(&d0) + plane * u64::from(layers), "{what}: plane 1 half-way into the level");
        for layer in 0..layers {
            let off = plane * u64::from(layer);
            let p0 = words_at(lod0(&d0) + off, plane as usize / 4);
            let p1 = words_at(lod0(&d1) + off, plane as usize / 4);
            for y in 0..H {
                for x in 0..W {
                    let at = tiled(x, y, W, 8) / 4;
                    let want = texel(c.float(), layer, x, y);
                    assert_eq!([p0[at], p0[at + 1]], want[..2], "{what}: layer {layer}: R,G of ({x}, {y}) in plane 0");
                    assert_eq!([p1[at], p1[at + 1]], want[2..], "{what}: layer {layer}: B,A of ({x}, {y}) in plane 1");
                }
            }
        }
    } else {
        assert_eq!((c.class, stored), (GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA16F), "{what}: the one fallback");
        for layer in 0..layers {
            let img = read_gpu(lod0(&d0) + plane * u64::from(layer), plane as usize);
            for y in 0..H {
                for x in 0..W {
                    let at = tiled(x, y, W, 8);
                    let got: Vec<u16> = img[at..at + 8].chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
                    let want: Vec<u16> = texel(true, layer, x, y).iter().map(|&v| half(v)).collect();
                    assert_eq!(got, want, "{what}: layer {layer}: the halves of ({x}, {y})");
                }
            }
        }
    }
}

/// One class at one attachment: the upload, the status, the refusals, the
/// storage left alone and the texture sampled as uploaded.
fn check(c: &Wide, a: Attach) {
    let what = format!("{} {a:?}", c.name);
    let (tex, up) = make_texture(c, a);
    let before = storage(tex);
    let stored = before.as_ref().map(|s| s.0);
    if c.open() {
        assert_eq!(up, GL_NO_ERROR, "{what}: the upload");
        let s = stored.unwrap_or_else(|| panic!("{what}: an open class has storage"));
        if s != c.class {
            eprintln!(
                "{what}: in the GC sampled mask but stored as class {s} (sized_format holds GL_RGBA32F as halves)"
            );
        }
    } else if c.float() {
        eprintln!("{what}: not in GC_SAMPLED_TEXFMTS (gc-w128-flip): checking the RGBA16F halves");
        assert_eq!(up, GL_NO_ERROR, "{what}: GL_RGBA32F is held as halves");
        assert_eq!(stored, Some(GLES_TEXFMT_RGBA16F), "{what}: stored as RGBA16F");
    } else {
        eprintln!("{what}: not in GC_SAMPLED_TEXFMTS (gc-w128-flip): checking the closed state");
        assert_eq!(up, GL_INVALID_OPERATION, "{what}: es3_storage_or_refuse");
        assert_eq!(stored, None, "{what}: no storage");
    }

    let (attach_err, status) = attach(tex, a);
    assert_eq!(attach_err, GL_NO_ERROR, "{what}: the attachment call itself");
    assert_eq!(status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{what}: glCheckFramebufferStatus 0x{status:x}");

    let [draw, clear, clear_buffer, read] = refused_calls(c, a, &what);
    const IFO: GLenum = GL_INVALID_FRAMEBUFFER_OPERATION;
    assert_eq!(draw, (IFO, GL_NO_ERROR), "{what}: glDrawArrays, then glFinish");
    /* glClear and every glClearBuffer* check completeness at the call (ES 3.0
     * 4.4.4.4) and record nothing for the glFinish to emit */
    assert_eq!(clear, (IFO, GL_NO_ERROR), "{what}: glClear, then glFinish");
    assert_eq!(clear_buffer, (IFO, GL_NO_ERROR), "{what}: glClearBuffer, then glFinish");
    /* GL_RGBA_INTEGER is refused (GL_INVALID_OPERATION) before the status
     * is looked at: the read buffer has no integer image */
    let want = if c.float() { (IFO, GL_NO_ERROR) } else { (GL_INVALID_OPERATION, GL_NO_ERROR) };
    assert_eq!(read, want, "{what}: glReadPixels, then glFinish");
    assert!(before == storage(tex), "{what}: the texture's storage (both planes) changed");

    if let Some(s) = stored {
        samples_the_upload(c, a, tex, s, &what);
        assert!(before == storage(tex), "{what}: the sampling draw changed the storage");
    }
}

#[test]
fn an_rgba32f_attachment_is_incomplete_and_nothing_renders_into_it() {
    for a in ATTACHES {
        on_gc7000(|| check(&WIDE[0], a));
    }
}

#[test]
fn an_rgba32ui_attachment_is_incomplete_and_nothing_renders_into_it() {
    for a in ATTACHES {
        on_gc7000(|| check(&WIDE[1], a));
    }
}

#[test]
fn an_rgba32i_attachment_is_incomplete_and_nothing_renders_into_it() {
    for a in ATTACHES {
        on_gc7000(|| check(&WIDE[2], a));
    }
}

/// The surfaces stay out of the GC's render classes: what makes the
/// framebuffer incomplete rather than drawn into plane 0.
#[test]
fn the_128_bit_surfaces_are_not_drawable_on_the_gc() {
    for surf in [GLES_SURF_RGBA32I, GLES_SURF_RGBA32UI] {
        assert!(!rt_format_drawable(surf), "{}: a drawable GC render class", gles_surf_name(surf));
    }
}
