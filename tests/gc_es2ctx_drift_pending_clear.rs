// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context: a full pending clear kept across a
//! framebuffer bind (4255adf1) and an emptied framebuffer's clear not parked
//! (0db66553).
//!
//! 4255adf1 parks a whole-target clear of the window, or of an FBO of private
//! renderbuffers, on its framebuffer when another one is bound
//! (`draw::park_pending_clear`); it goes out at the next draw of that
//! framebuffer, a read, a flush, or a change of the framebuffer or its images.
//! 0db66553 parks only a complete framebuffer. Both move *when* a clear
//! reaches the GPU, and dEQP-GLES2 clears everywhere: functional.fbo.render.*
//! (shared_colorbuffer, recreate_*, resize_*), functional.fbo.api.*,
//! functional.read_pixels.*, the lifetime.attach cases and every
//! texture-from-FBO case. The i.MX8MP whole tree (0fbe61a3) predates both.
//!
//! The expected order is GL's, not the current output's: a clear of A is done
//! before anything that reads A (a glReadPixels, a draw that samples it), lands
//! on the images A had when it was cleared (not on storage specified later, not
//! on a deleted texture), and the clear of a framebuffer emptied since is no
//! error and no stale write. The stand-in OSAL keeps every submitted stream and
//! executes none, so a clear is found in the stream as an RS fill (colour words
//! and destination address), the target of a draw is its PE colour address, and
//! a sampled texture is the address in sampler slot 0's descriptor.
//!
//! Not reachable on this rig: eglSwapBuffers (no EGL surface). The window's
//! clear is driven out with glFinish, which 4255adf1 names as the same exit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_pending_clear
//! cargo clippy --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_pending_clear -- -D warnings
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]
// the plumbing is shared with gc_es2ctx_rt_switch.rs; not every helper is used here
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits ---------
 *
 * The same stand-in as tests/gc_tiled_copy_reuse.rs.
 */

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

/// Every `(register, value, word)` a LOAD_STATE of the stream writes, in
/// order; `word` is the index of the LOAD_STATE command that holds it.
fn writes(words: &[u32]) -> Vec<(u32, u32, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v, i));
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

/// Index in `words` of each DRAW_INSTANCED, in order.
fn draws_at(words: &[u32]) -> Vec<usize> {
    let (mut out, mut i) = (Vec::new(), 0usize);
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
            out.push(i);
        }
        i += match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => 1 + ((w >> 16) & 0x3ff) as usize,
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

/// Index in `words` of each front-end STALL command, with its token.
fn fe_stalls_at(words: &[u32]) -> Vec<(usize, u32)> {
    let (mut out, mut i) = (Vec::new(), 0usize);
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_STALL {
            out.push((i, words.get(i + 1).copied().unwrap_or(0)));
        }
        i += match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => 1 + ((w >> 16) & 0x3ff) as usize,
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
        let c = yakogl::gl::context::gl_context_create(WIN as u32, WIN as u32);
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

/* ---- GL plumbing -------------------------------------------------------------------- */

const WIN: GLsizei = 64;
const SIZE: GLsizei = 16;

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

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// dEQP-GLES2's fbo.render shaders, cut down: one flat colour into the FBO,
/// and the window's quad sampling the FBO's texture in the fragment stage.
const VS_FLAT: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS_FLAT: &str = "precision mediump float;\nuniform vec4 u;\nvoid main() { gl_FragColor = u; }\n";
const VS_TEX: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { t = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS_TEX: &str =
    "precision mediump float;\nuniform sampler2D s;\nvarying vec2 t;\nvoid main() { gl_FragColor = texture2D(s, t); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad with `prog` into whatever framebuffer is bound, `w` x `h`.
fn quad(prog: GLuint, w: GLsizei, h: GLsizei) {
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, w, h);
        glUseProgram(prog);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
}

fn flat(rgba: [f32; 4]) -> GLuint {
    let p = program(VS_FLAT, FS_FLAT);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u".as_ptr()), rgba[0], rgba[1], rgba[2], rgba[3]);
    }
    p
}

fn sampling() -> GLuint {
    let p = program(VS_TEX, FS_TEX);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
    }
    p
}

/// Texel (x, y) of an uploaded image, `salt` telling uploads apart.
fn texel(x: GLsizei, y: GLsizei, salt: u8) -> [u8; 4] {
    [x as u8 * 8, y as u8 * 8, salt, 0xff]
}

fn image(salt: u8) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            v.extend_from_slice(&texel(x, y, salt));
        }
    }
    v
}

/// (Re)specify level 0 of the bound 2D texture: 16x16 RGBA8, `data` or none.
fn tex_image(data: Option<&[u8]>) {
    // SAFETY: plain GL calls; `data` holds 16x16 RGBA texels when given.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            SIZE,
            SIZE,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            data.map_or(core::ptr::null(), |d| d.as_ptr().cast()),
        );
    }
    no_error("glTexImage2D");
}

/// A texture bound to unit 0, NEAREST, CLAMP_TO_EDGE (complete at one level).
fn texture(data: Option<&[u8]>) -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    tex_image(data);
    t
}

/// A framebuffer with `t` as colour attachment 0, bound.
fn fbo(t: GLuint) -> GLuint {
    let mut fb: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    no_error("framebuffer");
    fb
}

fn kicks() -> usize {
    with_platform(|p| p.kicks.len())
}

fn kicks_since(n: usize) -> Vec<Vec<u32>> {
    with_platform(|p| p.kicks[n..].to_vec())
}

fn finishes() -> u32 {
    yakogl::vivante::backend::gc_finishes()
}

/// glReadPixels of `w` x `h` RGBA8 at the origin of the bound framebuffer.
fn read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: `px` holds w x h RGBA texels.
    unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    no_error("glReadPixels");
    px
}

/* ---- reading a stream --------------------------------------------------------------- */

/// One submitted stream, with its writes and its draws.
struct Stream {
    words: Vec<u32>,
    w: Vec<(u32, u32, usize)>,
    draws: Vec<usize>,
}

impl Stream {
    fn new(words: Vec<u32>) -> Self {
        Self { w: writes(&words), draws: draws_at(&words), words }
    }

    /// The colour target's address of draw `n`: the last PE colour address
    /// written before it.
    fn target_of(&self, n: usize) -> u32 {
        let at = self.draws[n];
        self.w
            .iter()
            .rev()
            .find(|&&(r, _, i)| r == pe_pipe_color_addr(0) && i < at)
            .map(|&(_, v, _)| v)
            .unwrap_or_else(|| panic!("draw {n} has no colour address before it"))
    }

    /// The colour target of every draw, in order.
    fn targets(&self) -> Vec<u32> {
        (0..self.draws.len()).map(|n| self.target_of(n)).collect()
    }

    /// The word index of the PE colour address write that draw `n` uses.
    fn target_word_of(&self, n: usize) -> usize {
        let at = self.draws[n];
        self.w.iter().rev().find(|&&(r, _, i)| r == pe_pipe_color_addr(0) && i < at).map(|&(_, _, i)| i).unwrap()
    }

    /// Level 0's address in the descriptor of sampler slot 0 that draw `n`
    /// reads (the stand-in's arena holds the descriptor).
    fn sampled_by(&self, n: usize) -> u32 {
        let at = self.draws[n];
        let desc = self
            .w
            .iter()
            .rev()
            .find(|&&(r, _, i)| r == nte_descriptor_addr(0) && i < at)
            .map(|&(_, v, _)| u64::from(v))
            .unwrap_or_else(|| panic!("draw {n} names no descriptor for slot 0"));
        u32::from_le_bytes(read_gpu(desc, 4).try_into().unwrap())
    }

    /// What lies between words `from` (exclusive) and `to` (exclusive).
    fn between(&self, from: usize, to: usize) -> impl Iterator<Item = &(u32, u32, usize)> {
        self.w.iter().filter(move |&&(_, _, i)| i > from && i < to)
    }

    /// The target switch before draw `n` (n > 0, into another target than
    /// draw n - 1): after draw n - 1 and before draw n's colour address, in
    /// this order, the colour and depth caches written back (with the shader
    /// L1), the front end held until the pixel engine has drained, then the
    /// texture caches invalidated and the rasteriser held until the pixel
    /// engine has drained (`emit_target_boundary`, then `emit_flushes`).
    fn assert_switch(&self, n: usize, what: &str) {
        self.assert_fenced(self.draws[n - 1], self.target_word_of(n), what);
    }

    /// [`Self::assert_switch`] between any two words: what came before
    /// `from` (a draw, an RS operation) and the colour address at `to`.
    fn assert_fenced(&self, from: usize, to: usize, what: &str) {
        assert!(from < to, "{what}: the new target's colour address comes after word {from}");
        let colour = self
            .between(from, to)
            .find(|&&(r, v, _)| {
                r == GC_GL_FLUSH_CACHE && v & (GC_FLUSH_COLOR | GC_FLUSH_DEPTH) == GC_FLUSH_COLOR | GC_FLUSH_DEPTH
            })
            .map(|&(_, _, i)| i)
            .unwrap_or_else(|| panic!("{what}: no colour and depth flush before the new target"));
        let fe_pe = fe_stalls_at(&self.words)
            .into_iter()
            .find(|&(i, tok)| i > colour && i < to && tok == stall_token(GC_SYNC_FE, GC_SYNC_PE))
            .map(|(i, _)| i)
            .unwrap_or_else(|| panic!("{what}: no FE-PE stall after the flush, before the new target"));
        assert!(
            self.between(colour, fe_pe).any(|&(r, v, _)| r == GC_TS_FLUSH_CACHE && v == GC_TS_FLUSH),
            "{what}: no tile-status flush between the colour flush and the FE-PE stall"
        );
        let texture = self
            .between(fe_pe, to)
            .find(|&&(r, v, _)| r == GC_GL_FLUSH_CACHE && v & GC_FLUSH_TEXTURE != 0)
            .map(|&(_, _, i)| i)
            .unwrap_or_else(|| panic!("{what}: no texture cache flush after the FE-PE stall, before the new target"));
        assert!(
            self.between(fe_pe, to).any(|&(r, v, _)| r == GC_GL_FLUSH_CACHE && v & GC_FLUSH_TEXTUREVS != 0),
            "{what}: no vertex-stage texture cache flush before the new target"
        );
        assert!(
            self.between(texture, to)
                .any(|&(r, v, _)| r == GC_GL_STALL_TOKEN && v == stall_token(GC_SYNC_RA, GC_SYNC_PE)),
            "{what}: no RA-PE stall after the texture flush, before the new target"
        );
    }
}

/// The kicks submitted since `n`, one stream each.
fn streams_since(n: usize) -> Vec<Stream> {
    kicks_since(n).into_iter().map(Stream::new).collect()
}

/* ---- the cases ---------------------------------------------------------------------- */

/// The clear colour of every case: blue, as the A8R8G8B8 word of the RS fill.
const BLUE: u32 = 0xff00_00ff;

fn clear_blue() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    no_error("glClear");
}

fn bind(fb: GLuint) {
    // SAFETY: a plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    no_error("glBindFramebuffer");
}

/// A 16x16 RGBA8 renderbuffer, attached to colour 0 of the bound framebuffer.
fn attach_renderbuffer() -> GLuint {
    let mut rb: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8_OES, SIZE, SIZE);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    no_error("renderbuffer");
    rb
}

/// A framebuffer of one private renderbuffer (the kind 4255adf1 parks), bound.
fn rb_fbo() -> (GLuint, GLuint) {
    let mut fb: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let rb = attach_renderbuffer();
    // SAFETY: a plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
    (fb, rb)
}

/// What a case submitted: the streams since its start, with GL_NO_ERROR and no
/// draw refused.
struct Run {
    k0: usize,
    refused: u64,
}

impl Run {
    fn start() -> Self {
        Run { k0: kicks(), refused: yakogl::vivante::backend::gc_refused_draws() as u64 }
    }

    fn streams(&self) -> Vec<Stream> {
        no_error("the case");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws() as u64, self.refused, "a refused draw");
        streams_since(self.k0)
    }

    fn finish(&self) -> Vec<Stream> {
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        self.streams()
    }
}

impl Stream {
    /// Each RS fill of the stream: (word of its kicker, destination, first fill word).
    fn fills(&self) -> Vec<(usize, u32, u32)> {
        let (mut ctrl, mut dest, mut val) = (0, 0, 0);
        let mut out = Vec::new();
        for &(r, v, word) in &self.w {
            match r {
                GC_RS_CLEAR_CONTROL => ctrl = v & GC_RS_CLEAR_CONTROL_BITS_ALL,
                r if r == rs_pipe_dest_addr(0) => dest = v,
                r if r == rs_fill_value(0) => val = v,
                GC_RS_KICKER if ctrl != 0 => out.push((word, dest, val)),
                _ => {}
            }
        }
        out
    }
}

/// Every fill of `colour` in the streams, as (stream, word, destination).
fn fills_of(streams: &[Stream], colour: u32) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    for (n, s) in streams.iter().enumerate() {
        for (word, dest, val) in s.fills() {
            if val == colour {
                out.push((n, word, dest));
            }
        }
    }
    out
}

/// Every draw of the streams, as (stream, word, target).
fn draws_of(streams: &[Stream]) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    for (n, s) in streams.iter().enumerate() {
        for (d, t) in s.targets().into_iter().enumerate() {
            out.push((n, s.draws[d], t));
        }
    }
    out
}

/// The target of the first draw of `streams`.
fn first_target(streams: &[Stream]) -> u32 {
    draws_of(streams).first().expect("a draw").2
}

/// glClear A, bind B, draw into B, bind A, glReadPixels of A (the shape of
/// functional.fbo.render.shared_colorbuffer_clear and read_pixels.*): the
/// clear is in the submitted streams when the read returns, as a fill of A's
/// storage, not of B's; the read of A waited for it. A's storage is the one
/// A's draw names afterwards.
#[test]
fn a_clear_kept_across_a_bind_reaches_a_before_its_read() {
    on_gc7000(|| {
        let (a, _) = rb_fbo();
        let (b, _) = rb_fbo();
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let run = Run::start();
        let f0 = finishes();
        bind(a);
        clear_blue();
        bind(b);
        quad(red, SIZE, SIZE);
        bind(a);
        read(SIZE, SIZE);
        assert!(finishes() > f0, "glReadPixels of A waits for the GPU");
        let streams = run.streams();
        let fills = fills_of(&streams, BLUE);
        let draws = draws_of(&streams);
        assert_eq!(fills.len(), 1, "A's clear is in the streams when its read returns: {fills:?}");
        assert_eq!(draws.len(), 1, "only B's draw");
        assert_ne!(fills[0].2, draws[0].2, "the clear of A went to B's storage");
        /* A's own storage: the target of a draw into A afterwards */
        let after = Run::start();
        quad(red, SIZE, SIZE);
        let s2 = after.finish();
        assert_eq!(first_target(&s2), fills[0].2, "the clear filled A's storage");
        assert!(fills_of(&s2, BLUE).is_empty(), "A's clear was made twice");
    });
}

/// The clear of A kept over two rounds of binds with B drawn twice: made once.
#[test]
fn a_clear_kept_across_two_binds_is_made_once() {
    on_gc7000(|| {
        let (a, _) = rb_fbo();
        let (b, _) = rb_fbo();
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let run = Run::start();
        bind(a);
        clear_blue();
        bind(b);
        quad(red, SIZE, SIZE);
        bind(a);
        bind(b);
        quad(red, SIZE, SIZE);
        bind(a);
        read(SIZE, SIZE);
        let streams = run.finish();
        let fills = fills_of(&streams, BLUE);
        let draws = draws_of(&streams);
        assert_eq!(fills.len(), 1, "one clear of A, made once: {fills:?}");
        assert_eq!(draws.len(), 2);
        assert!(draws.iter().all(|d| d.2 != fills[0].2), "B's draws and A's clear share a storage");
    });
}

/// functional.fbo.render.recreate_colorbuffer.* (renderbuffer flavour):
/// glClear A, bind B, glRenderbufferStorage of A's renderbuffer (new
/// storage), bind A, draw. The clear belongs to the storage A had: it must
/// not land on the new one.
#[test]
fn a_kept_clear_does_not_land_on_respecified_renderbuffer_storage() {
    on_gc7000(|| {
        let (a, rb) = rb_fbo();
        let (b, _) = rb_fbo();
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        bind(a);
        let first = Run::start();
        quad(red, SIZE, SIZE);
        let old = first.finish().iter().flat_map(|s| s.targets()).next().expect("A's first draw");
        let run = Run::start();
        clear_blue();
        bind(b);
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8_OES, SIZE, SIZE);
        }
        no_error("glRenderbufferStorage");
        bind(a);
        quad(red, SIZE, SIZE);
        let streams = run.finish();
        assert_eq!(draws_of(&streams).len(), 1, "A's second draw");
        let new = first_target(&streams);
        assert_ne!(new, old, "glRenderbufferStorage kept A's storage");
        let fills = fills_of(&streams, BLUE);
        assert!(fills.iter().all(|f| f.2 != new), "the clear of A landed on the storage made after it: {fills:?}");
        for f in &fills {
            assert_eq!(f.2, old, "a clear of A went neither to its old storage nor its new");
        }
    });
}

/// functional.fbo.render.recreate_colorbuffer.* (texture flavour): glClear
/// A, bind B, glTexImage2D of A's colour texture (new storage), bind A, draw.
/// A texture attachment is not parked (a draw in between could sample it), so
/// the clear goes out at the bind to B, to the old storage; nothing is made to
/// the new one.
// DEFECT (issue text in the item report): glClear of texture FBO A stays pending
// after the bind to B (park_pending_clear parks renderbuffer FBOs only, and
// settle_pending_clear runs at a draw or flush, not at the bind), and
// glTexImage2D of A's texture (A not bound) does not flush it: the next draw into
// A fuses the clear into the re-specified storage.
#[test]
#[ignore = "known defect: a pending clear is applied to storage made after it"]
fn a_clear_before_a_texture_respecification_does_not_land_on_the_new_storage() {
    on_gc7000(|| {
        let ta = texture(None);
        let a = fbo(ta);
        let (b, _) = rb_fbo();
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        bind(a);
        let first = Run::start();
        quad(red, SIZE, SIZE);
        let old = first.finish().iter().flat_map(|s| s.targets()).next().expect("A's first draw");
        let run = Run::start();
        clear_blue();
        bind(b);
        // SAFETY: a plain GL call.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, ta);
            /* twice the size: new storage whatever the allocator reuses; its texels are the upload's */
            let data = vec![0x5au8; (4 * SIZE * SIZE * 4) as usize];
            let n = 2 * SIZE;
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, n, n, 0, GL_RGBA, GL_UNSIGNED_BYTE, data.as_ptr().cast());
        }
        no_error("glTexImage2D");
        bind(a);
        quad(red, 2 * SIZE, 2 * SIZE);
        let streams = run.finish();
        assert_eq!(draws_of(&streams).len(), 1);
        let new = first_target(&streams);
        assert_ne!(new, old, "glTexImage2D kept the storage");
        let fills = fills_of(&streams, BLUE);
        assert!(fills.iter().all(|f| f.2 != new), "the clear of A landed on the new storage: {fills:?}");
        for f in &fills {
            assert_eq!(f.2, old, "a clear of A went neither to its old storage nor its new");
        }
    });
}

/// glClear A (a texture FBO), bind B, glDeleteTextures of A's colour texture,
/// bind A again, glFinish: no error, no refused draw; the clear, if it is made
/// at all, fills the storage A had (the deleted texture's, which A, not bound
/// when the texture went, keeps: the orphan rule of ES 2.0 4.4.3).
#[test]
fn a_clear_before_a_texture_delete_is_no_error() {
    on_gc7000(|| {
        let ta = texture(None);
        let a = fbo(ta);
        let (b, _) = rb_fbo();
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        bind(a);
        let first = Run::start();
        quad(red, SIZE, SIZE);
        let old = first.finish().iter().flat_map(|s| s.targets()).next().expect("A's first draw");
        let run = Run::start();
        clear_blue();
        bind(b);
        // SAFETY: one live name.
        unsafe { glDeleteTextures(1, &ta) };
        no_error("glDeleteTextures");
        bind(a);
        let streams = run.finish();
        for f in fills_of(&streams, BLUE) {
            assert_eq!(f.2, old, "the clear of A went to another storage than A's");
        }
        assert!(streams.iter().all(|s| s.draws.is_empty()), "a draw came from nowhere");
    });
}

/// functional.fbo.api / lifetime.attach: glClear A (a texture FBO), detach its
/// texture while it is bound (A is emptied), attach a second texture that
/// holds data, draw into A. The clear belongs to the first texture: none lands
/// on the second (renderbuffer contents are undefined, a texture's are the
/// upload's). `through_bind` also binds B and A again between the detach and
/// the attach (an emptied framebuffer is not parked, 0db66553).
fn emptied_then_reattached(through_bind: bool) {
    on_gc7000(|| {
        let t1 = texture(None);
        let a = fbo(t1);
        let (b, _) = rb_fbo();
        let t2 = texture(Some(&image(7)));
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        bind(a);
        let first = Run::start();
        quad(red, SIZE, SIZE);
        let old = first.finish().iter().flat_map(|s| s.targets()).next().expect("A's first draw");
        let run = Run::start();
        clear_blue();
        // SAFETY: a plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0) };
        no_error("detach");
        if through_bind {
            bind(b);
            bind(a);
        }
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t2, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        quad(red, SIZE, SIZE);
        let streams = run.finish();
        assert_eq!(draws_of(&streams).len(), 1);
        let new = first_target(&streams);
        assert_ne!(new, old);
        let fills = fills_of(&streams, BLUE);
        assert!(
            fills.iter().all(|f| f.2 != new),
            "the stale clear of the emptied A landed on the new storage: {fills:?}"
        );
    });
}

// DEFECT (issue text in the item report): replace_attachment flushes a parked
// clear but not the bound framebuffer's pending one, so the clear of the old
// texture is fused into the next draw, which lands on the texture attached since.
#[test]
#[ignore = "known defect: a pending clear is applied to an attachment made after it"]
fn a_clear_of_a_framebuffer_emptied_since_is_not_stale_on_a_new_attachment() {
    emptied_then_reattached(false);
}

// DEFECT (the same cause; 0db66553 declines to park the emptied A, so its clear
// stays pending on A and reaches the texture attached later).
#[test]
#[ignore = "known defect: a pending clear is applied to an attachment made after it"]
fn a_clear_of_a_framebuffer_emptied_and_left_is_not_stale_on_a_new_attachment() {
    emptied_then_reattached(true);
}

/// lifetime.attach.deleted_*: glClear A (a texture FBO), empty A (detach the
/// texture), bind B, delete A, glFinish: no error (0db66553: the clear of an
/// emptied framebuffer is no GL_INVALID_FRAMEBUFFER_OPERATION at the bind or
/// the delete), no refused draw.
#[test]
fn a_clear_of_an_emptied_framebuffer_is_no_error_at_bind_or_delete() {
    on_gc7000(|| {
        let ta = texture(None);
        let a = fbo(ta);
        let (b, _) = rb_fbo();
        let run = Run::start();
        bind(a);
        clear_blue();
        // SAFETY: a plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0) };
        no_error("detach");
        bind(b);
        // SAFETY: one live name.
        unsafe { glDeleteFramebuffers(1, &a) };
        no_error("glDeleteFramebuffers");
        run.finish();
    });
}

/// The same for a renderbuffer FBO: empty it, bind another, delete the
/// framebuffer, delete the renderbuffer, glFinish.
#[test]
fn a_clear_of_an_emptied_renderbuffer_framebuffer_is_no_error_at_bind_or_delete() {
    on_gc7000(|| {
        let (a, rb) = rb_fbo();
        let (b, _) = rb_fbo();
        let run = Run::start();
        bind(a);
        clear_blue();
        // SAFETY: a plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0) };
        no_error("detach");
        bind(b);
        // SAFETY: live names.
        unsafe { glDeleteFramebuffers(1, &a) };
        no_error("glDeleteFramebuffers");
        // SAFETY: a live name.
        unsafe { glDeleteRenderbuffers(1, &rb) };
        no_error("glDeleteRenderbuffers");
        run.finish();
    });
}

/// glClear A (a texture FBO), then a draw into B that samples A's texture (the
/// texture-from-FBO cases of functional.fbo.render.* and functional.texture.*):
/// the clear is in the streams before the draw that samples it, and it fills the
/// storage the draw's sampler names.
#[test]
fn a_clear_of_a_texture_framebuffer_is_in_the_stream_before_the_draw_that_samples_it() {
    on_gc7000(|| {
        let ta = texture(None);
        let a = fbo(ta);
        let (b, _) = rb_fbo();
        let tex = sampling();
        let run = Run::start();
        bind(a);
        clear_blue();
        bind(b);
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, ta);
        }
        quad(tex, SIZE, SIZE);
        let streams = run.finish();
        let draws = draws_of(&streams);
        assert_eq!(draws.len(), 1, "the sampling draw");
        let (ds, dw, _) = draws[0];
        let sampled = streams[ds].sampled_by(0);
        let fills = fills_of(&streams, BLUE);
        assert!(
            fills.iter().any(|&(s, w, dest)| dest == sampled && (s, w) < (ds, dw)),
            "no clear of the sampled storage before the draw that samples it: fills {fills:?}, draw at {:?}, samples 0x{sampled:x}",
            (ds, dw)
        );
    });
}

/// The window's clear kept across a bind to an FBO, then glFinish (the exit
/// 4255adf1 names beside eglSwapBuffers, which this rig has no surface for): a
/// fill of the window is in the streams, no draw refused.
#[test]
fn a_window_clear_kept_across_an_fbo_bind_goes_out_at_glfinish() {
    on_gc7000(|| {
        let (a, _) = rb_fbo();
        let run = Run::start();
        bind(0);
        clear_blue();
        bind(a);
        let fills = fills_of(&run.finish(), BLUE);
        assert_eq!(fills.len(), 1, "the window's clear did not reach the GPU at glFinish: {fills:?}");
    });
}

/// The window cleared, an FBO bound and drawn, the window drawn (glmark2's and
/// dEQP's frame shape): the clear reaches the window's storage before the
/// window's draw, once, and not the FBO's storage.
#[test]
fn a_window_clear_kept_across_an_fbo_draw_precedes_the_window_draw() {
    on_gc7000(|| {
        let (a, _) = rb_fbo();
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let run = Run::start();
        bind(0);
        clear_blue();
        bind(a);
        quad(red, SIZE, SIZE);
        bind(0);
        quad(red, WIN, WIN);
        let streams = run.finish();
        let draws = draws_of(&streams);
        assert_eq!(draws.len(), 2, "the FBO's draw and the window's");
        let (fbo_draw, win_draw) = (draws[0], draws[1]);
        assert_ne!(fbo_draw.2, win_draw.2);
        let fills = fills_of(&streams, BLUE);
        assert_eq!(fills.len(), 1, "the window's clear is made once: {fills:?}");
        assert_eq!(fills[0].2, win_draw.2, "the clear of the window went to another storage");
        assert!(
            (fills[0].0, fills[0].1) < (win_draw.0, win_draw.1),
            "the window's clear comes after the window's draw"
        );
    });
}

/// The window cleared, then an FBO bound and glReadPixels of it: the read goes
/// to the FBO; the window's clear is still made (at the read or the finish), once.
#[test]
fn a_window_clear_kept_across_a_read_of_an_fbo_is_made_once() {
    on_gc7000(|| {
        let (a, _) = rb_fbo();
        let run = Run::start();
        bind(0);
        clear_blue();
        bind(a);
        read(SIZE, SIZE);
        let fills = fills_of(&run.finish(), BLUE);
        assert_eq!(fills.len(), 1, "the window's clear: {fills:?}");
    });
}
