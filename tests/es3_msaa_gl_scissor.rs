// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The scissor test during a 4x to 1x resolve blit, into a renderbuffer and
//! into a texture, on the host (docs/notes/es3-msaa-renderbuffers.md (a), (b)).
//!
//! ES 3.0.6 4.3.3 (BlitFramebuffer): "When values are written to the draw
//! buffers, blit operations bypass the fragment pipeline. The only fragment
//! operations which affect a blit are the pixel ownership test, the scissor
//! test and sRGB conversion (see section 4.1.8). Color, depth, and stencil
//! masks (see section 4.2.2) are ignored." Dithering is not in that list, so
//! it does not apply either. The scissor test limits which destination pixels
//! are written; a box that reaches past the destination is clipped to it, and
//! with GL_SCISSOR_TEST off the box is not looked at.
//!
//! Each case resolves 4x RGBA8 renderbuffer sources into a 1x RGBA8
//! destination that already holds an earlier colour, once with the box inside
//! the 8x8 destination, once with it partly outside (past the top/right edge
//! and, in another case, below/left of the origin), once with the test off.
//! The destination bytes are checked inside and outside the box. The colour
//! mask and dither are set to their non-default values during the scissored
//! blit as well, to show that they change nothing.
//!
//! The stand-in back end is that of tests/es3_msaa_gl_state.rs: its job-end
//! resolve fills each 4x target a clear touched with that clear's colour. The
//! sources are cleared with every state at its default; the state is set only
//! around the blit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_scissor
//! ```

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
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
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
    name: c"es3-msaa-gl-scissor".as_ptr() as *const c_char,
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

/* ---- helpers for the cases ---------------------------------------------- */

/// What a resolve is into.
#[derive(Clone, Copy, Debug)]
enum Dst {
    Renderbuffer,
    Texture,
}

/// A 1x RGBA8 destination bound as the draw framebuffer; the texture and
/// framebuffer names are deleted on drop.
struct Target {
    fbo: Option<Fbo>,
    fb: GLuint,
    tex: GLuint,
}

impl Drop for Target {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            if self.fbo.is_none() {
                glDeleteFramebuffers(1, &self.fb);
                glDeleteTextures(1, &self.tex);
            }
        }
    }
}

fn target(kind: Dst) -> Target {
    match kind {
        Dst::Renderbuffer => {
            let f = dest();
            let fb = f.fb;
            Target { fbo: Some(f), fb, tex: 0 }
        }
        Dst::Texture => {
            let (mut fb, mut tex) = (0, 0);
            // SAFETY: plain GL calls; one name each out.
            unsafe {
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8 as GLint,
                    8,
                    8,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            }
            assert_eq!(err(), GL_NO_ERROR);
            Target { fbo: None, fb, tex }
        }
    }
}

/// A destination of `kind` holding `first` everywhere, and a 4x source of
/// `second` ready to resolve into it.
fn set_up(kind: Dst, first: [f32; 4], second: [f32; 4]) -> (Fbo, Target, Fbo) {
    let s1 = source(first);
    let dst = target(kind);
    assert_eq!(resolve(s1.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(first)), "{kind:?}: the priming resolve");
    let s2 = source(second);
    (s1, dst, s2)
}

/// Scissor on with `bx` = (x, y, w, h), plus the colour mask and dither at
/// their non-default values (ignored by a blit).
fn scissor_on(bx: [GLint; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(bx[0], bx[1], bx[2], bx[3]);
        glColorMask(T, F, T, F);
        glDisable(GL_DITHER);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The destination pixels in the 8x8 grid that `bx` (clipped) covers take
/// `inside`, the rest `outside`.
fn check(kind: Dst, dst: GLuint, bx: [GLint; 4], inside: [u8; 4], outside: [u8; 4]) {
    let px = pixels_of(dst);
    for y in 0..8i32 {
        for x in 0..8i32 {
            let in_box = x >= bx[0] && x < bx[0] + bx[2] && y >= bx[1] && y < bx[1] + bx[3];
            let want = if in_box { inside } else { outside };
            assert_eq!(px[(y * 8 + x) as usize], want, "{kind:?}: pixel ({x}, {y}), in the box: {in_box}");
        }
    }
}

fn scissored(kind: Dst, bx: [GLint; 4]) {
    let _c = context();
    let (_s1, dst, s2) = set_up(kind, GREEN, RED);
    scissor_on(bx);
    assert_eq!(resolve(s2.fb, dst.fb), GL_NO_ERROR, "a scissor is no reason to refuse a resolve");
    check(kind, dst.fb, bx, bytes(RED), bytes(GREEN));
}

/* ---- box inside the destination ------------------------------------------ */

/// (1) The box (2, 3, 4, 2) inside the 8x8 destination: only its pixels take
/// the new colour.
#[test]
fn box_inside_renderbuffer() {
    scissored(Dst::Renderbuffer, [2, 3, 4, 2]);
}

#[test]
fn box_inside_texture() {
    scissored(Dst::Texture, [2, 3, 4, 2]);
}

/// A one-pixel box at a corner: exactly one pixel is written.
#[test]
fn one_pixel_box_renderbuffer() {
    scissored(Dst::Renderbuffer, [7, 7, 1, 1]);
}

#[test]
fn one_pixel_box_texture() {
    scissored(Dst::Texture, [0, 0, 1, 1]);
}

/* ---- box partly outside the destination ---------------------------------- */

/// (2) The box (5, 5, 6, 6) reaches past the top and right edges: pixels
/// 5..8 in both axes are written, nothing else, and there is no error.
#[test]
fn box_past_far_edge_renderbuffer() {
    scissored(Dst::Renderbuffer, [5, 5, 6, 6]);
}

#[test]
fn box_past_far_edge_texture() {
    scissored(Dst::Texture, [5, 5, 6, 6]);
}

/// The box (-3, -2, 6, 5) starts below and left of the origin: pixels 0..3 in
/// x and 0..3 in y are written.
#[test]
fn box_before_origin_renderbuffer() {
    scissored(Dst::Renderbuffer, [-3, -2, 6, 5]);
}

#[test]
fn box_before_origin_texture() {
    scissored(Dst::Texture, [-3, -2, 6, 5]);
}

/// A box that covers the destination and more writes all of it.
#[test]
fn box_covering_everything() {
    let kind = Dst::Texture;
    let _c = context();
    let (_s1, dst, s2) = set_up(kind, GREEN, RED);
    scissor_on([-4, -4, 32, 32]);
    assert_eq!(resolve(s2.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(RED)));
}

/// A box wholly outside writes nothing, into a texture too.
#[test]
fn box_wholly_outside_texture() {
    let _c = context();
    let (_s1, dst, s2) = set_up(Dst::Texture, GREEN, RED);
    scissor_on([8, 0, 4, 4]);
    assert_eq!(resolve(s2.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(GREEN)));
}

/* ---- scissor off --------------------------------------------------------- */

/// (3) With the test off the box (2, 3, 4, 2) is not used: the whole region is
/// written, whatever the colour mask and dither say.
fn unscissored(kind: Dst) {
    let _c = context();
    let (_s1, dst, s2) = set_up(kind, GREEN, RED);
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_SCISSOR_TEST);
        glScissor(2, 3, 4, 2);
        glColorMask(T, F, T, F);
        glDisable(GL_DITHER);
    }
    assert_eq!(resolve(s2.fb, dst.fb), GL_NO_ERROR);
    assert!(all(dst.fb, bytes(RED)), "{kind:?}: every pixel, green channel included");
}

#[test]
fn scissor_off_renderbuffer() {
    unscissored(Dst::Renderbuffer);
}

#[test]
fn scissor_off_texture() {
    unscissored(Dst::Texture);
}

/* ---- the box and a sub-rectangle ----------------------------------------- */

/// (4) The scissor box and the blit rectangle both limit the write: a blit of
/// (0, 0, 6, 6) with the box (3, 3, 4, 4) writes the overlap, (3..6, 3..6).
#[test]
fn box_and_blit_rectangle_intersect() {
    let _c = context();
    let (_s1, dst, s2) = set_up(Dst::Texture, GREEN, RED);
    scissor_on([3, 3, 4, 4]);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, s2.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glBlitFramebuffer(0, 0, 6, 6, 0, 0, 6, 6, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR);
    check(Dst::Texture, dst.fb, [3, 3, 3, 3], bytes(RED), bytes(GREEN));
}
