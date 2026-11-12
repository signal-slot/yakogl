// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context: the render-target switches of
//! dEQP-GLES2, one stream (e299852c, docs/notes/gc-kick-continue.md C1).
//!
//! Since e299852c a draw into another target no longer ends the GC kick:
//! the stream goes on behind `emit_target_boundary` (colour, depth and
//! shader-L1 flush, TS flush, FE-PE stall, descriptor flush), and the next
//! draw's `emit_flushes` writes the colour/depth/texture flush and the
//! RA-PE stall before the new target's address. The i.MX8MP's whole
//! dEQP-GLES2 tree (0fbe61a3) predates that. These cases replay the
//! dEQP-GLES2 shapes that switch targets and read the submitted streams:
//!
//! - render into an FBO texture, then the window samples it, then
//!   glReadPixels (functional.fbo.render.*, functional.texture.* that
//!   render to a texture and sample it);
//! - the colour attachment re-specified between two draws, texture and
//!   renderbuffer (functional.fbo.render.recreate_colorbuffer.*);
//! - glClear on one target, then draws on another and back
//!   (functional.fbo.render.shared_colorbuffer_clear,
//!   functional.color_clear.*);
//! - glReadPixels of an FBO after draws into another target
//!   (functional.read_pixels.*);
//! - a CPU write (glTexSubImage2D) of a texture rendered and sampled in
//!   the open kick (functional.texture.*, functional.fbo.render.*).
//!
//! What is asserted is what e299852c must keep: the cache flush and the
//! stall between targets, each draw's target address, the sampling draw's
//! texture address, the kick count and order, and the pixels where the host
//! path can compute them (the stand-in OSAL executes no stream, so only
//! bytes the CPU wrote are known).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_rt_switch
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

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

/// functional.fbo.render.* and the functional.texture.* cases that render
/// into a texture and then sample it: glClear and a draw into an FBO's
/// texture, the default framebuffer bound, a quad sampling that texture in
/// the fragment stage, glReadPixels.
///
/// Asserted: no kick until glReadPixels, which submits one kick (with one
/// finish) holding both draws; the first draw (the clear fused into it)
/// into the texture's storage, the second into the window; the switch's
/// flushes and stalls between them; the sampling draw reads the storage
/// the first draw wrote.
#[test]
fn render_to_texture_then_sample_it_in_the_window_then_read_pixels() {
    on_gc7000(|| {
        let t = texture(None);
        fbo(t);
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let tex = sampling();
        let (k0, f0) = (kicks(), finishes());
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        quad(red, SIZE, SIZE);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        quad(tex, WIN, WIN);
        assert_eq!(kicks(), k0, "a target switch submitted a kick (e299852c keeps one stream)");
        read(WIN, WIN);
        assert_eq!(kicks() - k0, 1, "glReadPixels submits the open kick, once");
        assert_eq!(finishes() - f0, 1, "and waits for it before reading");

        let s = &streams_since(k0)[0];
        assert_eq!(s.draws.len(), 2, "the FBO's draw and the window's");
        let [storage, window] = s.targets()[..] else { unreachable!() };
        assert_ne!(storage, window, "the FBO's draw and the window's go to two targets");
        assert_eq!(s.sampled_by(1), storage, "the window's quad samples what the FBO's draw wrote");
        s.assert_switch(1, "FBO texture -> window");
    });
}

/// functional.fbo.render.recreate_colorbuffer.* (texture flavour): a draw
/// into the FBO, its texture's level 0 re-specified with glTexImage2D, a
/// second draw into the FBO, then the window samples the texture.
///
/// Asserted: the four draws are one kick (the re-specification gives new
/// storage and submits nothing); the second FBO draw goes to the new
/// storage, not the old one; each of the three switches has the flushes and
/// stalls; the window's quad samples the new storage; glReadPixels of the
/// FBO waits, and leaves no kick open.
#[test]
fn recreate_the_colour_texture_between_two_draws() {
    on_gc7000(|| {
        let t = texture(None);
        let fb = fbo(t);
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let green = flat([0.0, 1.0, 0.0, 1.0]);
        let tex = sampling();
        let k0 = kicks();
        quad(red, SIZE, SIZE);
        /* the window between them, as dEQP's fbo cases draw their
         * reference in the window */
        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        quad(red, WIN, WIN);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_image(None);
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
        quad(green, SIZE, SIZE);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        quad(tex, WIN, WIN);
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        let f0 = finishes();
        read(SIZE, SIZE);
        assert!(finishes() > f0, "glReadPixels of the FBO waited for nothing");

        let streams = streams_since(k0);
        /* glTexImage2D gives the texture new storage and leaves the old one
         * to the open kick: no submit */
        assert_eq!(streams.len(), 1, "the four draws and the re-specification are one kick");
        let s = &streams[0];
        assert_eq!(s.draws.len(), 4, "FBO, window, FBO again, window sampling");
        let [old, window, new, last] = s.targets()[..] else { unreachable!() };
        assert_ne!(old, window);
        assert_ne!(new, old, "the draw after glTexImage2D went to the texture's old storage");
        assert_ne!(new, window);
        assert_eq!(last, window, "the sampling quad goes to the window");
        assert_eq!(s.sampled_by(3), new, "the window samples the re-specified storage");
        for n in 1..4 {
            s.assert_switch(n, &format!("draw {} -> draw {n}", n - 1));
        }
        /* nothing left open: a glFinish after the read submits nothing */
        let k1 = kicks();
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        assert_eq!(kicks(), k1, "glReadPixels left a kick open");
    });
}

/// functional.fbo.render.recreate_colorbuffer.* (renderbuffer flavour): the
/// same with an RGB565 renderbuffer re-specified by glRenderbufferStorage.
///
/// Asserted: the three draws are one kick; the second FBO draw goes to the
/// new storage; both switches are fenced.
#[test]
fn recreate_the_colour_renderbuffer_between_two_draws() {
    on_gc7000(|| {
        let mut rb: GLuint = 0;
        let mut fb: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB565, SIZE, SIZE);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        no_error("renderbuffer FBO");
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let k0 = kicks();
        quad(red, SIZE, SIZE);
        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        quad(red, WIN, WIN);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB565, SIZE, SIZE);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        no_error("glRenderbufferStorage again");
        quad(red, SIZE, SIZE);
        read(SIZE, SIZE);

        let streams = streams_since(k0);
        assert_eq!(streams.len(), 1, "the three draws and the re-specification are one kick");
        let s = &streams[0];
        assert_eq!(s.draws.len(), 3, "FBO, window, FBO again");
        let [old, window, new] = s.targets()[..] else { unreachable!() };
        assert_ne!(old, window);
        assert_ne!(new, old, "the draw after glRenderbufferStorage went to the old storage");
        assert_ne!(new, window);
        for n in 1..3 {
            s.assert_switch(n, &format!("draw {} -> draw {n}", n - 1));
        }
    });
}

/// functional.fbo.render.shared_colorbuffer_clear and functional.color_clear.*:
/// glClear on FBO A with no draw after it, a draw into FBO B, then a draw
/// into A again (A, B, A). A's clear has no draw to fuse into, so it is
/// recorded on its own: a resolve-engine fill of A's storage, before B's
/// draw.
///
/// Asserted: no kick until glReadPixels, then one kick and one finish; in
/// it one RS fill and two draws, in the order fill, B, A; the fill's
/// destination is the storage A's draw names, not B's; before the fill the
/// colour and depth caches written back and an RA-PE stall (`emit_rs`);
/// after it, before B's colour address, an RA-PE stall and the switch's
/// fences; the B -> A switch fenced; a later read of B submits nothing.
#[test]
fn clear_one_target_then_draw_into_another() {
    on_gc7000(|| {
        let ta = texture(None);
        let fa = fbo(ta);
        let tb = texture(None);
        let fb_b = fbo(tb);
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let (k0, f0) = (kicks(), finishes());
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fa);
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glBindFramebuffer(GL_FRAMEBUFFER, fb_b);
        }
        no_error("glClear A");
        quad(red, SIZE, SIZE);
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fa) };
        quad(red, SIZE, SIZE);
        assert_eq!(kicks(), k0, "a target switch submitted a kick (e299852c keeps one stream)");
        read(SIZE, SIZE);
        assert_eq!(finishes() - f0, 1, "glReadPixels of A waits for the kick");
        let streams = streams_since(k0);
        assert_eq!(streams.len(), 1, "the clear of A and the draws into B and A are one kick");
        let s = &streams[0];
        assert_eq!(s.draws.len(), 2, "B's draw and A's");
        let [b, a] = s.targets()[..] else { unreachable!() };
        assert_ne!(a, b, "the two draws name one target");

        let fills: Vec<usize> =
            s.w.iter().enumerate().filter(|(_, &(r, _, _))| r == GC_RS_KICKER).map(|(n, _)| n).collect();
        assert_eq!(fills.len(), 1, "one RS operation: A's clear");
        let kicker = s.w[fills[0]].2;
        assert!(kicker < s.draws[0], "A's clear comes before B's draw");
        let r = &s.w[..fills[0]];
        let dest = r.iter().rev().find(|&&(reg, _, _)| reg == rs_pipe_dest_addr(0)).map(|&(_, v, _)| v);
        assert_eq!(dest, Some(a), "the clear fills A's storage, the one A's draw names");
        let cfg = r.iter().rposition(|&(reg, _, _)| reg == GC_RS_CONFIG).expect("the fill has no RS_CONFIG");
        assert_eq!(
            (r[cfg - 3].0, r[cfg - 3].1),
            (GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH),
            "the PE's caches written back before the fill"
        );
        assert_eq!(r[cfg - 2].0, GC_GL_SEMAPHORE_TOKEN, "then the PE drained");
        assert_eq!(
            (r[cfg - 1].0, r[cfg - 1].1),
            (GC_GL_STALL_TOKEN, stall_token(GC_SYNC_RA, GC_SYNC_PE)),
            "an RA-PE stall before the fill"
        );
        let to_b = s.target_word_of(0);
        let after = |reg: u32, v: u32| s.between(kicker, to_b).find(|w| (w.0, w.1) == (reg, v)).map(|w| w.2);
        let stall =
            after(GC_GL_STALL_TOKEN, stall_token(GC_SYNC_RA, GC_SYNC_PE)).expect("no RA-PE stall after the fill");
        let flush = s.between(kicker, to_b).find(|w| w.0 == GC_GL_FLUSH_CACHE).map(|w| w.2).unwrap();
        assert!(stall < flush, "the fill's RA-PE stall comes before the switch's flushes");
        s.assert_fenced(kicker, to_b, "fill A -> draw B");
        s.assert_switch(1, "draw B -> draw A");
        /* B read after the kick completed: nothing to submit */
        let k1 = kicks();
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb_b) };
        read(SIZE, SIZE);
        assert_eq!(kicks(), k1, "a read of B after the finish submitted another kick");
    });
}

/// functional.read_pixels.* after a target switch: an FBO whose texture the
/// CPU filled (glTexImage2D), draws into the window, then glReadPixels of
/// the FBO. The stand-in OSAL runs no stream, so the FBO's bytes are the
/// upload's, and the read must come back with them: it reads the FBO's
/// storage, not the window the last draw named.
///
/// Asserted: the pixels equal the upload, texel by texel; the read waited
/// for the window's kick.
#[test]
fn read_pixels_of_an_fbo_after_draws_into_the_window() {
    on_gc7000(|| {
        let t = texture(Some(&image(0x5a)));
        let fb = fbo(t);
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        let (k0, f0) = (kicks(), finishes());
        quad(red, WIN, WIN);
        quad(red, WIN, WIN);
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        let px = read(SIZE, SIZE);
        assert_eq!(kicks() - k0, 1, "glReadPixels submits the window's kick");
        assert!(finishes() > f0, "and waits for it");
        for y in 0..SIZE {
            for x in 0..SIZE {
                let at = ((y * SIZE + x) * 4) as usize;
                assert_eq!(px[at..at + 4], texel(x, y, 0x5a), "pixel ({x}, {y}) of the FBO");
            }
        }
    });
}

/// functional.texture.* and functional.fbo.render.* that render into a
/// texture, sample it, and then write it from the CPU: a draw into the FBO's
/// texture, a window draw sampling it, then glTexSubImage2D of the whole
/// level and glReadPixels of the FBO.
///
/// Asserted: the render and the sample stay one open kick until
/// glTexSubImage2D, which submits it and waits for it before the CPU writes
/// (the write cannot land under a render still queued); the switch in it
/// is fenced; the FBO's read-back is the CPU's data.
#[test]
fn cpu_write_after_a_render_and_a_sample_in_one_stream() {
    on_gc7000(|| {
        let t = texture(None);
        let fb = fbo(t);
        let red = flat([1.0, 0.0, 0.0, 1.0]);
        let tex = sampling();
        let k0 = kicks();
        quad(red, SIZE, SIZE);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        quad(tex, WIN, WIN);
        assert_eq!(kicks(), k0, "the render and the sample are still one open kick");
        let f0 = finishes();
        let data = image(0xa5);
        // SAFETY: a 16x16 RGBA image into the bound texture.
        unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, SIZE, SIZE, GL_RGBA, GL_UNSIGNED_BYTE, data.as_ptr().cast()) };
        no_error("glTexSubImage2D");
        assert_eq!(kicks() - k0, 1, "glTexSubImage2D submits the kick that renders into the texture");
        assert_eq!(finishes() - f0, 1, "and waits for it before the CPU writes");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        let px = read(SIZE, SIZE);
        let s = &streams_since(k0)[0];
        assert_eq!(s.draws.len(), 2, "the render and the sample in that kick");
        assert_eq!(s.sampled_by(1), s.target_of(0), "the window samples the texture the render wrote");
        s.assert_switch(1, "FBO texture -> window");
        for y in 0..SIZE {
            for x in 0..SIZE {
                let at = ((y * SIZE + x) * 4) as usize;
                assert_eq!(px[at..at + 4], texel(x, y, 0xa5), "pixel ({x}, {y}) after the CPU write");
            }
        }
    });
}
