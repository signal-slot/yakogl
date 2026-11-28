// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the 4x depth/stencil resolve at the dEQP sizes
//! 119x131 and 40x40 (fbo.msaa, renderbuffer_resize; neither is a multiple
//! of 16x4), through the GL API into the Vivante back end on the host (item
//! gc-ds-plane-tests; docs/notes/es3-msaa-renderbuffers.md (g), (h)).
//!
//! A 4x DEPTH24_STENCIL8 renderbuffer FBO draws into a scale-2 depth surface
//! (`ds_cache`, twice the size on each axis, each pixel's four samples a 2x2
//! block of D24S8 words); the back end's depth and stencil resolve
//! (`depth_resolve`, `stencil_resolve`, `ds_cache::resolve_plane`) read
//! sample 0 of each pixel. The GPU draws nothing here, so a draw is recorded
//! (the back end's recorder keeps the `GlesDraw`'s depth and stencil
//! identity) and the test seeds the surface's words itself, a different word
//! in every sample, as tests/es3_msaa_ds_gc.rs; a clear is the cache's own
//! (the plane holds the clear value, no word is read).
//!
//! Per size: the plane read of the 4x renderbuffer after a draw (every pixel,
//! the last row and column included, and the pitch's padding untouched), the
//! plane read after a clear, and `glBlitFramebuffer` of depth and stencil
//! into a 1x D24S8 FBO. That blit is refused today (the Vivante table has no
//! `ds_write`, src/gl/blit.rs): the test pins GL_INVALID_OPERATION, nothing
//! copied; the `#[ignore]`d twin checks the expected result.
//!
//! HOST-ONLY: API verdicts on the host back end, not a board result. The 4x
//! renderbuffers need `msaa-fbo`:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,msaa-fbo --test es3_msaa_ds_gc_sizes
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-vivante",
    feature = "shaderc-online",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_draw, yakogl_backend_vivante};
use yakogl::vivante::ds_cache::{self, DsKey};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/* ---- the stand-in OSAL: host memory, every kick accepted ------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
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
        with_platform(|p| {
            /* leaked, not dropped: the back end may still hold a CPU view */
            if let Some(a) = p.allocs.remove(&m.gpu) {
                core::mem::forget(a);
            }
        });
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC || !p.allocs.contains_key(&k.stream_gpu) {
            return -22;
        }
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
/// 64x64 context; the lock serialises the tests.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = table();
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
    DRAWS.lock().unwrap().clear();
    with_platform(|p| {
        /* leaked, not freed, as tests/es3_mrt4_gc.rs */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- the recorder: the depth/stencil identity of each draw ------------------ */

/// `GlesDraw`'s depth and stencil identity and sample count of each draw.
static DRAWS: Mutex<Vec<(DsKey, u8)>> = Mutex::new(Vec::new());

unsafe extern "C" fn rec_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core passes a live draw.
    let g = unsafe { &*d };
    let key = ds_cache::key_of(g, 2);
    DRAWS.lock().unwrap().push((key, g.samples));
    // SAFETY: as above.
    unsafe { gc_draw(d) }
}

/// The Vivante table with the recorder in, built once from the real table.
fn table() -> &'static YakoglBackend {
    static T: OnceLock<usize> = OnceLock::new();
    let p = *T.get_or_init(|| {
        // SAFETY: a bitwise copy of a static table of plain data; leaked and
        // never mutated again.
        let mut t = unsafe { core::ptr::read(&yakogl_backend_vivante) };
        t.draw = Some(rec_draw);
        Box::leak(Box::new(t)) as *const YakoglBackend as usize
    });
    // SAFETY: the leaked table above.
    unsafe { &*(p as *const YakoglBackend) }
}

/* ---- GL helpers ------------------------------------------------------------- */

const SIZES: [(u32, u32); 2] = [(119, 131), (40, 40)];
const CLEAR_D: f32 = 0.375;
const CLEAR_S: u8 = 0x2a;
const SENTINEL_F: f32 = -7.5;
const SENTINEL_S: u8 = 0xa5;

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn clear_errors() {
    while err() != GL_NO_ERROR {}
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

fn colour_program() {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0, 0.0, 0.0, 1.0); }\n";
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
        glUseProgram(p);
    }
}

struct Fbo {
    fb: GLuint,
    ds: GLuint,
}

/// A bound `w` x `h` framebuffer with an RGBA8 and a DEPTH24_STENCIL8
/// renderbuffer of `samples` each, checked complete.
fn framebuffer(samples: GLsizei, w: u32, h: u32) -> Fbo {
    let (mut fb, mut rbs) = (0, [0; 2]);
    // SAFETY: plain GL calls on names this test makes.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(2, rbs.as_mut_ptr());
        for (rb, format, point) in
            [(rbs[0], GL_RGBA8, GL_COLOR_ATTACHMENT0), (rbs[1], GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT)]
        {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w as GLsizei, h as GLsizei);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{w}x{h} at {samples}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{w}x{h} at {samples}");
    }
    Fbo { fb, ds: rbs[1] }
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// Draw a whole-target quad with the depth and stencil tests on (both
/// written) and return the draw's depth/stencil key and sample count.
fn draw_with_ds(w: u32, h: u32) -> (DsKey, u8) {
    let quad = [-1.0f32, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    colour_program();
    DRAWS.lock().unwrap().clear();
    // SAFETY: plain GL calls; `quad` outlives the finish.
    unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_ALWAYS);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_ALWAYS, 0x33, 0xff);
        glStencilOp(GL_REPLACE, GL_REPLACE, GL_REPLACE);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, quad.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_STENCIL_TEST);
    }
    assert_eq!(err(), GL_NO_ERROR);
    DRAWS.lock().unwrap().last().copied().expect("the draw reached the back end")
}

fn clear_ds() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(CLEAR_D);
        glClearStencil(CLEAR_S as GLint);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The depth plane of `rb` through the table, `w` x `h` at pitch `pitch`.
fn read_depth(rb: GLuint, w: u32, h: u32, pitch: u32) -> (i32, Vec<f32>) {
    let mut v = vec![SENTINEL_F; (h * pitch) as usize];
    let f = yakogl_backend_vivante.depth_resolve.expect("depth_resolve");
    // SAFETY: `h` rows of `pitch` floats.
    let r = unsafe { f(ds_id(rb), v.as_mut_ptr(), w, h, pitch) };
    (r, v)
}

fn read_stencil(rb: GLuint, w: u32, h: u32, pitch: u32) -> (i32, Vec<u8>) {
    let mut v = vec![SENTINEL_S; (h * pitch) as usize];
    let f = yakogl_backend_vivante.stencil_resolve.expect("stencil_resolve");
    // SAFETY: `h` rows of `pitch` bytes.
    let r = unsafe { f(ds_id(rb), v.as_mut_ptr(), w, h, pitch) };
    (r, v)
}

fn depth_of(word: u32) -> f32 {
    ((word >> 8) as f64 / 16_777_215.0) as f32
}

/// A word for (row, column) of the scale-2 surface, different in every sample
/// of a pixel; the low bits of each plane name the sample.
fn word_at(r: usize, c: usize) -> u32 {
    let mut x = ((r * 4099 + c) as u32).wrapping_mul(0x9e37_79b9) | 1;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    let s = ((r & 1) << 1 | (c & 1)) as u32;
    (x & !0x0300 & !0x03) | s << 8 | s
}

/// Seed the surface of `key` with [`word_at`]: its row stride in words.
fn seed(key: DsKey) -> (&'static [u32], usize) {
    let (mem, bytes) = ds_cache::memory_of(key).expect("the 4x depth surface");
    let stride = ds_cache::stride_of(key).expect("a surface") as usize / 4;
    /* the allocation is guard + pixels + guard, the guard 0 or 256 KiB by the
     * `vivante-depth-guard` feature */
    let guard = (mem.size - bytes) / 2;
    // SAFETY: the allocation holds the guard, the pixels and a guard, word
    // aligned past the guard; it lives until the case ends (leaked after).
    let words = unsafe { core::slice::from_raw_parts_mut((mem.cpu as *mut u8).add(guard) as *mut u32, bytes / 4) };
    for (i, w) in words.iter_mut().enumerate() {
        *w = word_at(i / stride, i % stride);
    }
    (words, stride)
}

/* ---- the plane read of a 4x FBO after a draw, and after a clear --------------- */

/// A draw into a 4x D24S8 FBO of `w` x `h` keeps 4 samples; the surface's
/// words are sample-0 read for every pixel, the last row and column too.
fn check_draw_then_read(w: u32, h: u32) {
    on_gc7000(|| {
        let fbo = framebuffer(4, w, h);
        let (key, samples) = draw_with_ds(w, h);
        assert_eq!(samples, 4, "{w}x{h}: the draw keeps its 4 samples");
        assert_eq!(key.depth_id, ds_id(fbo.ds), "the draw names the renderbuffer");
        assert!(ds_cache::planes_of(key).is_some(), "{w}x{h}: the scale-2 surface holds the planes");
        let (words, stride) = seed(key);
        assert!(stride >= 2 * w as usize, "{w}x{h}: the surface's rows hold 2w words");
        let pitch = w + 3;
        let (r, d) = read_depth(fbo.ds, w, h, pitch);
        assert_eq!(r, 0, "{w}x{h}: depth read");
        let (r, s) = read_stencil(fbo.ds, w, h, pitch);
        assert_eq!(r, 0, "{w}x{h}: stencil read");
        for y in 0..h as usize {
            for x in 0..pitch as usize {
                let (gd, gs) = (d[y * pitch as usize + x], s[y * pitch as usize + x]);
                if x < w as usize {
                    let word = words[2 * y * stride + 2 * x];
                    assert_eq!(gd.to_bits(), depth_of(word).to_bits(), "{w}x{h}: depth ({x}, {y})");
                    assert_eq!(gs, (word & 0xff) as u8, "{w}x{h}: stencil ({x}, {y})");
                    assert_eq!(gs & 3, 0, "{w}x{h}: sample 0's marker at ({x}, {y})");
                } else {
                    assert_eq!(gd.to_bits(), SENTINEL_F.to_bits(), "{w}x{h}: depth pitch padding ({x}, {y})");
                    assert_eq!(gs, SENTINEL_S, "{w}x{h}: stencil pitch padding ({x}, {y})");
                }
            }
        }
    });
}

#[test]
fn a_119x131_4x_ds_draw_reads_sample_0_of_every_pixel() {
    check_draw_then_read(119, 131);
}

#[test]
fn a_40x40_4x_ds_draw_reads_sample_0_of_every_pixel() {
    check_draw_then_read(40, 40);
}

/// A clear of a 4x D24S8 FBO is a whole-plane fill: every pixel reads the
/// clear value, with no surface word read.
fn check_clear_then_read(w: u32, h: u32) {
    on_gc7000(|| {
        let fbo = framebuffer(4, w, h);
        clear_ds();
        let (r, d) = read_depth(fbo.ds, w, h, w);
        assert_eq!(r, 0, "{w}x{h}: depth read after a clear");
        assert!(d.iter().all(|v| v.to_bits() == CLEAR_D.to_bits()), "{w}x{h}: depth is the clear everywhere");
        let (r, s) = read_stencil(fbo.ds, w, h, w);
        assert_eq!(r, 0, "{w}x{h}: stencil read after a clear");
        assert!(s.iter().all(|&v| v == CLEAR_S), "{w}x{h}: stencil is the clear everywhere");
    });
}

#[test]
fn a_119x131_4x_ds_clear_reads_the_clear_everywhere() {
    check_clear_then_read(119, 131);
}

#[test]
fn a_40x40_4x_ds_clear_reads_the_clear_everywhere() {
    check_clear_then_read(40, 40);
}

/* ---- the resolve blit into a 1x FBO -------------------------------------------- */

/// A 4x FBO drawn and seeded, a cleared 1x FBO of the same size beside it,
/// bound as the blit's read and draw framebuffers.
fn blit_setup(w: u32, h: u32) -> (Fbo, Fbo, &'static [u32], usize) {
    let src = framebuffer(4, w, h);
    let (key, _) = draw_with_ds(w, h);
    let (words, stride) = seed(key);
    let dst = framebuffer(0, w, h);
    clear_ds();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
    }
    (src, dst, words, stride)
}

fn blit_ds(w: u32, h: u32) {
    let (wi, hi) = (w as GLint, h as GLint);
    clear_errors();
    // SAFETY: a plain GL call.
    unsafe { glBlitFramebuffer(0, 0, wi, hi, 0, 0, wi, hi, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, GL_NEAREST) };
}

/// Today: the depth and stencil resolve blit is refused (no `ds_write`), one
/// GL_INVALID_OPERATION, the destination still its clear, and the context
/// usable.
fn check_blit_refused(w: u32, h: u32) {
    on_gc7000(|| {
        let (_src, dst, _, _) = blit_setup(w, h);
        blit_ds(w, h);
        assert_eq!(err(), GL_INVALID_OPERATION, "{w}x{h}: the blit is refused");
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h}: exactly one error");
        let (r, d) = read_depth(dst.ds, w, h, w);
        assert_eq!(r, 0);
        assert!(d.iter().all(|v| v.to_bits() == CLEAR_D.to_bits()), "{w}x{h}: destination depth untouched");
        let (r, s) = read_stencil(dst.ds, w, h, w);
        assert_eq!(r, 0);
        assert!(s.iter().all(|&v| v == CLEAR_S), "{w}x{h}: destination stencil untouched");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, dst.fb);
            glClearDepthf(0.5);
            glClear(GL_DEPTH_BUFFER_BIT);
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h}: the context is usable after the refusal");
    });
}

/// What a working blit leaves: no error, the destination holds sample 0 of
/// each source pixel in both planes.
fn check_blit_resolves(w: u32, h: u32) {
    on_gc7000(|| {
        let (_src, dst, words, stride) = blit_setup(w, h);
        blit_ds(w, h);
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h}: the blit works");
        let (r, d) = read_depth(dst.ds, w, h, w);
        assert_eq!(r, 0);
        let (r, s) = read_stencil(dst.ds, w, h, w);
        assert_eq!(r, 0);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let word = words[2 * y * stride + 2 * x];
                let i = y * w as usize + x;
                assert!((d[i] - depth_of(word)).abs() <= 0.5 / 16_777_215.0, "{w}x{h}: depth ({x}, {y})");
                assert_eq!(s[i], (word & 0xff) as u8, "{w}x{h}: stencil ({x}, {y})");
            }
        }
    });
}

#[test]
fn a_119x131_4x_ds_resolve_blit_is_refused_today() {
    check_blit_refused(119, 131);
}

#[test]
fn a_40x40_4x_ds_resolve_blit_is_refused_today() {
    check_blit_refused(40, 40);
}

#[test]
#[ignore = "owner: gc-ds-write"]
fn a_119x131_4x_ds_resolve_blit_keeps_sample_0() {
    check_blit_resolves(119, 131);
}

#[test]
#[ignore = "owner: gc-ds-write"]
fn a_40x40_4x_ds_resolve_blit_keeps_sample_0() {
    check_blit_resolves(40, 40);
}

/// The sizes the file covers are the dEQP ones, not multiples of 16x4.
#[test]
fn the_sizes_are_not_aligned() {
    for (w, h) in SIZES {
        assert!(w % 16 != 0 || h % 4 != 0, "{w}x{h}");
    }
}
