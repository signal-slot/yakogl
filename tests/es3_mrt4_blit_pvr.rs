// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer and glInvalidateFramebuffer on `GL_COLOR_ATTACHMENT3`
//! with the PowerVR back end (BXS-4-64, AM62P), through the GL API, on the
//! host (plan #368, pvr-blit4-test; docs/notes/mrt4-v3d-pvr.md).
//!
//! The PowerVR answers 3 draw targets today (`backend::draw_targets()`); the
//! host stand-in `backend::HOST_DRAW_TARGETS` = 4 lets the GL core accept a
//! fourth attachment, as `es3_msaa_pvr` does. A blit is carried out by the GL
//! core on the CPU (`gl::blit`), so these tests fill four RGBA8 textures
//! with known pixels and read the result back through `glReadPixels`; no draw
//! reaches the back end and the stand-in back end fills no pixels.
//!
//! Pinned: a read buffer of attachment 3 is the blit source; a draw buffer
//! list that reaches attachment 3 gets the blit (ES 3.0 4.3.3: every draw
//! buffer is written); `glInvalidateFramebuffer` and
//! `glInvalidateSubFramebuffer` accept `GL_COLOR_ATTACHMENT3`, leave its
//! pixels alone and tell the back end nothing (`attachment_gone` is not
//! called: the GL core validates and records nothing, the colour is always
//! kept), while attachment 4 is `GL_INVALID_OPERATION`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_blit_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

/// The ids `attachment_gone` was told, in order.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_gone(id: u32) {
    GONE.lock().unwrap().push(id);
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
    name: c"es3-mrt4-blit-pvr".as_ptr() as *const c_char,
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
    attachment_gone: Some(b_gone),
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
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
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_SCISSOR_TEST);
        glFinish();
    }
    GONE.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers of RGBA8 textures ------------------------------------- */

const W: usize = 8;
const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
/// The four attachments' fill colours (RGBA8), all different.
const FILL: [[u8; 4]; 4] = [[10, 20, 30, 255], [40, 50, 60, 255], [70, 80, 90, 255], [200, 100, 50, 255]];
const ZERO: [u8; 4] = [0, 0, 0, 0];
const NINE: [u8; 4] = [9, 8, 7, 255];

struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
        }
    }
}

/// A new `W` x `W` RGBA8 texture filled with `c`, left bound.
fn texture(c: [u8; 4], internal: GLenum, format: GLenum) -> GLuint {
    let px: Vec<u8> = (0..W * W).flat_map(|_| c).collect();
    let mut t = 0;
    // SAFETY: room for one name; `px` holds W*W four-byte pixels.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            internal as GLint,
            W as GLsizei,
            W as GLsizei,
            0,
            format,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    t
}

/// A bound framebuffer with one RGBA8 texture per entry as attachments
/// 0.., each filled with its colour. The framebuffer is complete.
fn fbo(fill: &[[u8; 4]]) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut f = Fbo { fb, tex: Vec::new() };
    for (i, c) in fill.iter().enumerate() {
        let t = texture(*c, GL_RGBA8, GL_RGBA);
        // SAFETY: a texture this test made.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, ATT[i], GL_TEXTURE_2D, t, 0) };
        f.tex.push(t);
    }
    // SAFETY: plain GL call.
    unsafe {
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    f
}

/// A framebuffer with attachments 0..=3 filled with `FILL`.
fn four() -> Fbo {
    fbo(&FILL)
}

fn draw_buffers(list: &[GLenum]) {
    // SAFETY: plain GL call with `list.len()` enums.
    unsafe { glDrawBuffers(list.len() as GLsizei, list.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers {list:x?}");
}

/// Blit the whole `W` x `W` area, nearest, colour only, `read` to `draw`;
/// `read_buffer` is the read framebuffer's read buffer. The GL error.
fn blit(read: &Fbo, read_buffer: GLenum, draw: &Fbo) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw.fb);
        glReadBuffer(read_buffer);
        assert_eq!(glGetError(), GL_NO_ERROR, "glReadBuffer {read_buffer:x}");
        let w = W as GLint;
        glBlitFramebuffer(0, 0, w, w, 0, 0, w, w, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        glGetError()
    }
}

/// The `W` x `W` pixels of attachment `n` of `fbo`, bottom row first.
fn pixels(fbo: &Fbo, n: usize) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; W * W];
    // SAFETY: room for W*W RGBA8 pixels; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glReadBuffer(ATT[n]);
        assert_eq!(glGetError(), GL_NO_ERROR);
        glReadPixels(0, 0, W as GLsizei, W as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels of attachment {n}");
    v
}

/// Every pixel of attachment `n` is `want`.
fn assert_all(fbo: &Fbo, n: usize, want: [u8; 4]) {
    let p = pixels(fbo, n);
    if let Some((i, bad)) = p.iter().enumerate().find(|(_, q)| **q != want) {
        panic!("attachment {n}: pixel {i} is {bad:?}, want {want:?}");
    }
}

/* ---- glBlitFramebuffer -------------------------------------------------- */

/// The fixtures read back as filled (a baseline for the rest).
#[test]
fn fixture_reads_back_per_attachment() {
    let _g = context();
    let f = four();
    for n in 0..4 {
        assert_all(&f, n, FILL[n]);
    }
}

/// `glReadBuffer(GL_COLOR_ATTACHMENT3)` is accepted on a four-attachment
/// framebuffer and `GL_READ_BUFFER` answers it.
#[test]
fn read_buffer_of_attachment3_is_accepted() {
    let _g = context();
    let f = four();
    // SAFETY: plain GL calls; one value.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        assert_eq!(glGetError(), GL_NO_ERROR);
        let mut v = 0;
        glGetIntegerv(GL_READ_BUFFER, &mut v);
        assert_eq!(v as GLenum, GL_COLOR_ATTACHMENT3);
    }
}

/// Attachment 3 as the source into a single-target framebuffer: its colour,
/// not attachment 0's.
#[test]
fn blit_from_attachment3_into_a_single_target_framebuffer() {
    let _g = context();
    let src = four();
    let dst = fbo(&[ZERO]);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT3, &dst), GL_NO_ERROR);
    assert_all(&dst, 0, FILL[3]);
    /* the source is left as it was */
    for n in 0..4 {
        assert_all(&src, n, FILL[n]);
    }
}

/// Each of the four read buffers picks its own attachment.
#[test]
fn blit_source_follows_the_read_buffer() {
    let _g = context();
    let src = four();
    let dst = fbo(&[ZERO]);
    for n in [3, 0, 2, 1, 3] {
        assert_eq!(blit(&src, ATT[n], &dst), GL_NO_ERROR);
        assert_all(&dst, 0, FILL[n]);
    }
}

/// Only a rectangle of attachment 3, scaled down 2:1 (nearest), lands in the
/// destination's corner; the rest of the destination is untouched.
#[test]
fn scaled_rectangle_from_attachment3() {
    let _g = context();
    let src = four();
    let dst = fbo(&[[1, 2, 3, 4]]);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    let p = pixels(&dst, 0);
    for y in 0..W {
        for x in 0..W {
            let want = if x < 4 && y < 4 { FILL[3] } else { [1, 2, 3, 4] };
            assert_eq!(p[y * W + x], want, "pixel ({x}, {y})");
        }
    }
}

/// ES 3.0 4.3.3: every draw buffer gets the blit. Draw buffers 0..=3 on the
/// destination all receive the colour of the source's attachment 0.
#[test]
fn blit_into_all_four_draw_buffers() {
    let _g = context();
    let src = fbo(&[NINE]);
    let dst = four();
    draw_buffers(&ATT);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT0, &dst), GL_NO_ERROR);
    for n in 0..4 {
        assert_all(&dst, n, NINE);
    }
}

/// Attachment 3 of one framebuffer into attachments 0..=3 of another: the
/// fourth read buffer and the fourth draw buffer together.
#[test]
fn blit_from_attachment3_into_all_four_draw_buffers() {
    let _g = context();
    let src = four();
    let dst = fbo(&[ZERO; 4]);
    draw_buffers(&ATT);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT3, &dst), GL_NO_ERROR);
    for n in 0..4 {
        assert_all(&dst, n, FILL[3]);
    }
}

/// A draw buffer list of `{NONE, NONE, NONE, ATTACHMENT3}` writes attachment
/// 3 alone.
#[test]
fn blit_into_attachment3_only() {
    let _g = context();
    let src = fbo(&[NINE]);
    let dst = four();
    draw_buffers(&[GL_NONE, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT0, &dst), GL_NO_ERROR);
    for n in 0..3 {
        assert_all(&dst, n, FILL[n]);
    }
    assert_all(&dst, 3, NINE);
}

/// Draw buffers 0 and 2 only: attachments 1 and 3 keep their pixels.
#[test]
fn blit_skips_attachments_that_are_not_draw_buffers() {
    let _g = context();
    let src = fbo(&[NINE]);
    let dst = four();
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT2, GL_NONE]);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT0, &dst), GL_NO_ERROR);
    assert_all(&dst, 0, NINE);
    assert_all(&dst, 1, FILL[1]);
    assert_all(&dst, 2, NINE);
    assert_all(&dst, 3, FILL[3]);
}

/// Reading attachment 3 of the framebuffer whose draw buffers include
/// attachment 0..=3 (the same framebuffer is read and drawn): attachment 3 is
/// its own destination, so `GL_INVALID_OPERATION` and nothing is written.
#[test]
fn blit_within_one_framebuffer_hits_the_shared_image() {
    let _g = context();
    let f = four();
    draw_buffers(&ATT);
    assert_eq!(blit(&f, GL_COLOR_ATTACHMENT3, &f), GL_INVALID_OPERATION);
    for n in 0..4 {
        assert_all(&f, n, FILL[n]);
    }
}

/// Same image read and drawn on attachment 3 alone.
#[test]
fn blit_from_attachment3_onto_itself_is_invalid_operation() {
    let _g = context();
    let f = four();
    draw_buffers(&[GL_NONE, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    assert_eq!(blit(&f, GL_COLOR_ATTACHMENT3, &f), GL_INVALID_OPERATION);
    assert_all(&f, 3, FILL[3]);
}

/// The scissor cuts the destination's attachment 3 as it does attachment 0.
#[test]
fn scissor_cuts_every_draw_buffer_including_attachment3() {
    let _g = context();
    let src = fbo(&[NINE]);
    let dst = four();
    draw_buffers(&ATT);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 2, 2);
    }
    let e = blit(&src, GL_COLOR_ATTACHMENT0, &dst);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
    assert_eq!(e, GL_NO_ERROR);
    for n in 0..4 {
        let p = pixels(&dst, n);
        for y in 0..W {
            for x in 0..W {
                let want = if x < 2 && y < 2 { NINE } else { FILL[n] };
                assert_eq!(p[y * W + x], want, "attachment {n} pixel ({x}, {y})");
            }
        }
    }
}

/// An integer attachment 3 as the source into a float draw buffer is
/// `GL_INVALID_OPERATION` and writes nothing.
#[test]
fn integer_attachment3_into_float_draw_buffer_is_invalid_operation() {
    let _g = context();
    let src = four();
    let t = texture(ZERO, GL_RGBA8UI, GL_RGBA_INTEGER);
    // SAFETY: a texture this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, src.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_TEXTURE_2D, t, 0);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    let dst = fbo(&[ZERO]);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT3, &dst), GL_INVALID_OPERATION);
    assert_all(&dst, 0, ZERO);
    // SAFETY: a name this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteTextures(1, &t);
    }
}

/// A read buffer of attachment 3 on a framebuffer with three attachments is
/// accepted by `glReadBuffer` (the point exists) but the blit has nothing to
/// read: the colour bit is ignored, no error, the destination is untouched.
#[test]
fn blit_from_an_unattached_attachment3_does_nothing() {
    let _g = context();
    let src = fbo(&FILL[..3]);
    let dst = fbo(&[[1, 2, 3, 4]]);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT3, &dst), GL_NO_ERROR);
    assert_all(&dst, 0, [1, 2, 3, 4]);
}

/* ---- glInvalidateFramebuffer, glInvalidateSubFramebuffer ----------------- */

fn invalidate(target: GLenum, list: &[GLenum]) -> GLenum {
    // SAFETY: plain GL call with `list.len()` enums.
    unsafe {
        glInvalidateFramebuffer(target, list.len() as GLsizei, list.as_ptr());
        glGetError()
    }
}

fn invalidate_sub(target: GLenum, list: &[GLenum], r: [GLint; 4]) -> GLenum {
    // SAFETY: plain GL call with `list.len()` enums.
    unsafe {
        glInvalidateSubFramebuffer(target, list.len() as GLsizei, list.as_ptr(), r[0], r[1], r[2], r[3]);
        glGetError()
    }
}

/// Attachment 3 alone, then 0..=3 together, on the draw framebuffer: no
/// error, the pixels stay (the colour is always kept) and the back end is
/// not told (`attachment_gone` is for released objects, not for discards).
#[test]
fn invalidate_attachment3_is_accepted_and_keeps_the_pixels() {
    let _g = context();
    let f = four();
    /* (a texture an earlier test deleted can be released late) */
    GONE.lock().unwrap().clear();
    assert_eq!(invalidate(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    assert_eq!(invalidate(GL_DRAW_FRAMEBUFFER, &ATT), GL_NO_ERROR);
    for n in 0..4 {
        assert_all(&f, n, FILL[n]);
    }
    assert!(GONE.lock().unwrap().is_empty(), "back end told: {:?}", GONE.lock().unwrap());
}

/// The same on the read framebuffer when it is not the draw one.
#[test]
fn invalidate_attachment3_of_the_read_framebuffer() {
    let _g = context();
    let r = four();
    let d = fbo(&[ZERO]);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, r.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, d.fb);
    }
    GONE.lock().unwrap().clear();
    assert_eq!(invalidate(GL_READ_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    /* the draw framebuffer has no attachment 3 but the point exists */
    assert_eq!(invalidate(GL_DRAW_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    assert_all(&r, 3, FILL[3]);
    assert!(GONE.lock().unwrap().is_empty());
}

/// A rectangle of attachment 3, also beside depth/stencil points.
#[test]
fn invalidate_sub_framebuffer_names_attachment3() {
    let _g = context();
    let f = four();
    GONE.lock().unwrap().clear();
    assert_eq!(invalidate_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], [1, 1, 4, 4]), GL_NO_ERROR);
    let all = [GL_COLOR_ATTACHMENT3, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT];
    assert_eq!(invalidate_sub(GL_FRAMEBUFFER, &all, [0, 0, 8, 8]), GL_NO_ERROR);
    /* an empty list and a zero-size rectangle are fine too */
    assert_eq!(invalidate_sub(GL_FRAMEBUFFER, &[], [0, 0, 1, 1]), GL_NO_ERROR);
    assert_eq!(invalidate_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], [0, 0, 0, 0]), GL_NO_ERROR);
    assert_eq!(invalidate_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], [0, 0, -1, 1]), GL_INVALID_VALUE);
    assert_all(&f, 3, FILL[3]);
    assert!(GONE.lock().unwrap().is_empty());
}

/// Attachment 4 is past `GL_MAX_COLOR_ATTACHMENTS` (4): `GL_INVALID_OPERATION`
/// (negative_api.buffer.invalidate_framebuffer), also beside a good name.
/// `GL_COLOR` is the window's name, not a framebuffer object's.
#[test]
fn invalidate_past_the_limit_is_an_error() {
    let _g = context();
    let _f = four();
    let mut v = 0;
    // SAFETY: one value.
    unsafe { glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &mut v) };
    assert_eq!(v, 4);
    assert_eq!(invalidate(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION);
    assert_eq!(invalidate(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION);
    assert_eq!(invalidate_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT4], [0, 0, 1, 1]), GL_INVALID_OPERATION);
    assert_eq!(invalidate(GL_FRAMEBUFFER, &[GL_COLOR]), GL_INVALID_ENUM);
}

/// An invalidate after a blit loses nothing: attachment 3 of the blit's
/// destination still reads what the blit wrote.
#[test]
fn invalidate_after_a_blit_keeps_attachment3() {
    let _g = context();
    let src = fbo(&[NINE]);
    let dst = four();
    draw_buffers(&ATT);
    assert_eq!(blit(&src, GL_COLOR_ATTACHMENT0, &dst), GL_NO_ERROR);
    // SAFETY: a name this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst.fb) };
    assert_eq!(invalidate(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR);
    assert_all(&dst, 3, NINE);
}
