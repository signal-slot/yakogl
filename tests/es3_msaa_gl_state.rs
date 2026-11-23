// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Per-fragment and buffer state during a 4x to 1x resolve blit, on the host
//! (docs/notes/es3-msaa-renderbuffers.md (a), (b)).
//!
//! ES 3.0.6 4.3.3: "When values are written to the draw buffers, blit
//! operations bypass the fragment pipeline. The only fragment operations
//! which affect a blit are the pixel ownership test, the scissor test and
//! sRGB conversion (see section 4.1.8). Color, depth, and stencil masks (see
//! section 4.2.2) are ignored." Dithering is not in that list, so it does not
//! apply. ES 3.0.6 3.1: GL_RASTERIZER_DISCARD discards primitives and makes
//! "the Clear and ClearBuffer* commands" be ignored; it names no other
//! command, so a blit is carried out with it enabled. Mesa 26.2.1
//! (src/mesa/main/blit.c) agrees: `_mesa_clip_blit` cuts the destination to
//! the scissor box, and nothing in the blit path looks at the colour mask,
//! dither or `RasterDiscard` (gallium's u_blitter draws with its own
//! rasterizer state). A GL_NONE draw or read buffer only drops the colour
//! copy ("If a buffer is specified in mask and does not exist in both the
//! read and draw framebuffers, the corresponding bit is silently ignored").
//!
//! The stand-in back end is that of tests/es3_msaa_gl.rs (`context_ms4`,
//! helpers copied, in the multi-target form of tests/es3_msaa_gl_mrt.rs): its
//! job-end resolve fills each 4x target a clear touched with that clear's
//! colour, and not before the job is finished. The sources are cleared with
//! every state below at its default (a clear is scissored, masked and
//! discarded); the state is set only around the blit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_state
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// A 4x target a clear touched: (`gpu`, w, h, colour).
type Cleared = (u64, u32, u32, [u8; 4]);

/// The 4x targets a clear touched since the last `finish`.
static PENDING: Mutex<Vec<Cleared>> = Mutex::new(Vec::new());

fn bytes(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v * 255.0 + 0.5) as u8)
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.samples > 1 && d.index_count == 0 {
        let mask = if d.clear_target_mask != 0 { d.clear_target_mask } else { 1 };
        let mut p = PENDING.lock().unwrap();
        for k in 0..(d.num_targets.max(1) as usize).min(8) {
            if mask & (1 << k) == 0 {
                continue;
            }
            let t = if k == 0 && d.targets[0].gpu == 0 {
                (d.target_gpu, d.target_w, d.target_h)
            } else {
                (d.targets[k].gpu, d.targets[k].w, d.targets[k].h)
            };
            p.push((t.0, t.1, t.2, bytes(d.clear_rgba)));
        }
    }
    0
}

/// Note (b)7 in the stand-in: when the jobs are complete, the memory of every
/// 4x target cleared holds the colour of its last clear, and not before (a
/// copy that did not wait reads the zeros the storage was made with).
unsafe extern "C" fn b_finish_resolve() -> i32 {
    for (gpu, w, h, c) in core::mem::take(&mut *PENDING.lock().unwrap()) {
        /* the stand-in OSAL's gpu address is the CPU pointer; RGBA8 rows of w*4 */
        let px = core::slice::from_raw_parts_mut(gpu as *mut [u8; 4], (w * h) as usize);
        px.fill(c);
    }
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-msaa-gl-state".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, finish: Some(b_finish_resolve), ..BACKEND };

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with `table` as the back end (a test that swapped in the 4x
/// one does not leak it into the next).
fn context_on(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a static table; the lock above serialises the tests. Plain GL calls.
    unsafe {
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    PENDING.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

/// The context with a stand-in back end that says `max_samples` 4.
fn context_ms4() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_MS4)
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments. A storage error comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, formats: &[GLenum]) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for (i, f) in formats.iter().enumerate() {
        let (e, rb) = storage(samples, *f);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb) };
    }
    assert_eq!(err(), GL_NO_ERROR);
    Ok(fbo)
}

/* ---- helpers of this file ---------------------------------------------- */

/// GL_TRUE and GL_FALSE as GLboolean.
const T: GLboolean = GL_TRUE as GLboolean;
const F: GLboolean = GL_FALSE as GLboolean;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// A colour dithering could move (not a multiple of 1/255 apart from 0 or 1).
const MID: [f32; 4] = [0.3, 0.6, 0.7, 0.45];

/// The per-fragment state this file changes, at its ES 3.0 defaults.
fn default_state() {
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_SCISSOR_TEST);
        glScissor(0, 0, 8, 8);
        glColorMask(T, T, T, T);
        glDisable(GL_RASTERIZER_DISCARD);
        glEnable(GL_DITHER);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The 4x context with [`default_state`]; the state goes back to the defaults
/// when the guard is dropped, so a failing test does not leak it.
struct Ctx {
    _g: MutexGuard<'static, ()>,
}

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: plain GL calls; the lock is still held.
        unsafe {
            glDisable(GL_SCISSOR_TEST);
            glColorMask(T, T, T, T);
            glDisable(GL_RASTERIZER_DISCARD);
            glEnable(GL_DITHER);
            while glGetError() != GL_NO_ERROR {}
        }
    }
}

fn context() -> Ctx {
    let g = context_ms4();
    default_state();
    Ctx { _g: g }
}

/// A 4x single-attachment RGBA8 framebuffer cleared to `c` (default state).
fn source(c: [f32; 4]) -> Fbo {
    let src = framebuffer(4, &[GL_RGBA8]).unwrap();
    // SAFETY: four GLfloats in.
    unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    src
}

/// A 1-sample single-attachment RGBA8 framebuffer.
fn dest() -> Fbo {
    framebuffer(0, &[GL_RGBA8]).unwrap()
}

/// The 8x8 pixels of colour attachment 0 of single-sample `fb`, row-major
/// from (0, 0).
fn pixels_of(fb: GLuint) -> Vec<[u8; 4]> {
    let mut px = vec![[0x11u8; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

fn all(fb: GLuint, c: [u8; 4]) -> bool {
    pixels_of(fb).iter().all(|p| *p == c)
}

/// The whole-surface 4x to 1x resolve of `read`'s COLOR_ATTACHMENT0 into
/// `dst`: the GL error.
fn resolve(read: GLuint, dst: GLuint) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    err()
}

/// `dst` holding `first` everywhere (a resolve with the default state).
fn primed(first: [f32; 4]) -> (Fbo, Fbo) {
    let src = source(first);
    let dst = dest();
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(first)), "the priming resolve");
    (src, dst)
}

/* ---- scissor: applies (ES 3.0.6 4.3.3) ---------------------------------- */

/// (1) With GL_SCISSOR_TEST on and the box (2, 3, 4, 2) inside the 8x8
/// region, only the pixels in the box are written: they take the second
/// resolve's colour, the rest keep the first's. The GL core cuts the
/// destination to the box (src/gl/blit.rs `blit_colour`), as Mesa's
/// `_mesa_clip_blit` does.
#[test]
fn scissor_cuts_the_resolve() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(2, 3, 4, 2);
    }
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR, "a scissor is no reason to refuse a resolve");
    let px = pixels_of(dst.fb);
    for y in 0..8 {
        for x in 0..8 {
            let inside = (2..6).contains(&x) && (3..5).contains(&y);
            let want = if inside { bytes(RED) } else { bytes(GREEN) };
            assert_eq!(px[y * 8 + x], want, "pixel ({x}, {y}), inside the box: {inside}");
        }
    }
}

/// (2) The box is only a test: GL_SCISSOR_TEST off with the same small box
/// resolves the whole region.
#[test]
fn scissor_box_without_the_test_does_not_cut() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL call.
    unsafe { glScissor(2, 3, 4, 2) };
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(RED)));
}

/// (3) A scissor box outside the region writes nothing, with no error.
#[test]
fn scissor_outside_the_region_writes_nothing() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(20, 20, 4, 4);
    }
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(GREEN)));
}

/* ---- colour mask: ignored (ES 3.0.6 4.3.3) ------------------------------ */

/// (4) glColorMask(TRUE, FALSE, TRUE, TRUE) does not mask a resolve: a
/// destination that held blue takes white in every channel, green included.
#[test]
fn colour_mask_is_ignored_by_the_resolve() {
    let _c = context();
    let (_first, dst) = primed(BLUE);
    let src = source(WHITE);
    // SAFETY: plain GL call.
    unsafe { glColorMask(T, F, T, T) };
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(WHITE)), "the green channel is written: {:?}", pixels_of(dst.fb)[0]);
}

/// (5) Every channel masked off still writes every channel.
#[test]
fn colour_mask_all_off_is_ignored_by_the_resolve() {
    let _c = context();
    let (_first, dst) = primed(BLUE);
    let src = source(RED);
    // SAFETY: plain GL call.
    unsafe { glColorMask(F, F, F, F) };
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(RED)));
}

/* ---- rasterizer discard: not named for blits (ES 3.0.6 3.1) ------------- */

/// (6) GL_RASTERIZER_DISCARD on: ES 3.0.6 3.1 makes it ignore primitives and
/// Clear/ClearBuffer* only, and Mesa's blit path does not look at it, so the
/// resolve is carried out. The GL core does that (src/gl/blit.rs never reads
/// `rasterizer_discard`).
#[test]
fn rasterizer_discard_does_not_stop_the_resolve() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_RASTERIZER_DISCARD) };
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(RED)), "the resolve is carried out under rasterizer discard");
}

/// (7) The contrast that keeps (6) honest: under GL_RASTERIZER_DISCARD a
/// clear of the 4x source is ignored, so a resolve after it copies the
/// earlier colour.
#[test]
fn rasterizer_discard_drops_the_source_clear_not_the_resolve() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(BLUE);
    // SAFETY: plain GL calls; four GLfloats in.
    unsafe {
        glEnable(GL_RASTERIZER_DISCARD);
        glBindFramebuffer(GL_FRAMEBUFFER, src.fb);
        glClearBufferfv(GL_COLOR, 0, RED.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(BLUE)), "the discarded clear left the source blue");
}

/* ---- dither: not applied (ES 3.0.6 4.3.3) ------------------------------- */

/// (8) GL_DITHER off and on give the same bytes, those of the source's
/// colour (the clear was made with dither at its default, on; the GL core
/// does not dither at all, src/gl/state.rs `GL_DITHER`).
#[test]
fn dither_off_and_on_resolve_the_same_bytes() {
    let _c = context();
    let src = source(MID);
    let off = dest();
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_DITHER) };
    assert_eq!(resolve(src.fb, off.fb), GL_NO_ERROR);
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_DITHER) };
    let on = dest();
    assert_eq!(resolve(src.fb, on.fb), GL_NO_ERROR);
    let (a, b) = (pixels_of(off.fb), pixels_of(on.fb));
    assert_eq!(a, b, "dither changes nothing in a resolve");
    assert!(a.iter().all(|p| *p == bytes(MID)), "the source's bytes: {:?}", a[0]);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glIsEnabled(GL_DITHER) }, T, "and the state stays as set");
}

/* ---- GL_NONE draw and read buffers: no copy, no error (ES 3.0.6 4.3.3) -- */

/// (9) The draw framebuffer's only draw buffer GL_NONE: the colour bit is
/// silently ignored, no error, nothing written.
#[test]
fn draw_buffer_none_is_a_no_op() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL calls; one enum in.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, dst.fb);
        glDrawBuffers(1, &GL_NONE);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(GREEN)), "nothing written");
}

/// (10) The 4x read framebuffer's read buffer GL_NONE: no error, nothing
/// copied.
#[test]
fn read_buffer_none_is_a_no_op() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_NONE);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(GREEN)), "nothing copied");
}

/// (11) Both GL_NONE, and a region the ES 3.0 multisample rule would refuse
/// were the colour copied (a scaled one): ES 3.0.6 4.3.3's multisample
/// INVALID_OPERATION for differing rectangles does not depend on the buffers,
/// so it is still raised (Mesa checks the sample counts and rectangles
/// before the buffers, blit.c `_mesa_blit_framebuffer`).
#[test]
fn none_buffers_keep_the_region_error() {
    let _c = context();
    let (_first, dst) = primed(GREEN);
    let src = source(RED);
    // SAFETY: plain GL calls on names this test made; one enum in.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, dst.fb);
        glDrawBuffers(1, &GL_NONE);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_NONE);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert!(all(dst.fb, bytes(GREEN)));
}

/* ---- all of it at once ------------------------------------------------- */

/// (12) Scissor on, a channel masked, discard on and dither off together:
/// only the scissor shows.
#[test]
fn every_state_at_once_only_the_scissor_shows() {
    let _c = context();
    let (_first, dst) = primed(BLUE);
    let src = source(WHITE);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 4, 8);
        glColorMask(F, T, T, T);
        glEnable(GL_RASTERIZER_DISCARD);
        glDisable(GL_DITHER);
    }
    assert_eq!(resolve(src.fb, dst.fb), GL_NO_ERROR);
    let px = pixels_of(dst.fb);
    for y in 0..8 {
        for x in 0..8 {
            let want = if x < 4 { bytes(WHITE) } else { bytes(BLUE) };
            assert_eq!(px[y * 8 + x], want, "pixel ({x}, {y})");
        }
    }
}
