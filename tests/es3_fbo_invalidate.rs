// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glInvalidateFramebuffer and glInvalidateSubFramebuffer in the GL core
//! (ES 3.0 4.5.1; dEQP-GLES3 negative_api.buffer.invalidate_*, and the API
//! side of functional.fbo.invalidate.*), on a stand-in back end that draws
//! nothing: what the calls answer and what they leave behind.
//!
//! What is pinned:
//! * the error order (Mesa main/fbobject.c): a target that is not
//!   GL_FRAMEBUFFER, GL_READ_FRAMEBUFFER or GL_DRAW_FRAMEBUFFER is
//!   GL_INVALID_ENUM before anything else, a negative count, width or height
//!   GL_INVALID_VALUE, then the attachments. Until 2026-11-26 the
//!   sub-rectangle form checked the size first and answered GL_INVALID_VALUE
//!   to a bad target with a negative width;
//! * the attachments: the default framebuffer takes GL_COLOR, GL_DEPTH and
//!   GL_STENCIL and nothing else (GL_INVALID_ENUM, GL_COLOR_ATTACHMENT0 and
//!   GL_DEPTH_STENCIL_ATTACHMENT included); a framebuffer object takes
//!   GL_COLOR_ATTACHMENT0 .. GL_MAX_COLOR_ATTACHMENTS - 1, GL_DEPTH_ATTACHMENT,
//!   GL_STENCIL_ATTACHMENT and GL_DEPTH_STENCIL_ATTACHMENT, a colour point at
//!   or past the limit is GL_INVALID_OPERATION, GL_COLOR / GL_DEPTH /
//!   GL_STENCIL / GL_NONE / GL_BACK GL_INVALID_ENUM; one bad entry in a list
//!   fails the call;
//! * the framebuffer the target names: GL_READ_FRAMEBUFFER checks against the
//!   read binding, GL_DRAW_FRAMEBUFFER and GL_FRAMEBUFFER against the draw
//!   one, and the call leaves both bindings as they were;
//! * what is kept: "the contents of the named attachments become undefined"
//!   (4.5.1), so keeping them is conformant and this GL keeps them (the core
//!   validates and returns, the back end never hears of it). The pixels
//!   outside the sub-rectangle, which the spec does keep, and inside it, the
//!   attachments, their names and the completeness are pinned unchanged.
//!
//! The dEQP fbo.invalidate render cases check only the kept buffers and redraw
//! the invalidated ones, so a kept invalidation passes them; their AM62P
//! failures are back-end causes (the commit message maps them).
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-powervr,shaderc-online --test es3_fbo_invalidate` (the AM62P
//! configuration; `host-test,gpu-v3d,shaderc-online,no-shader-table` and
//! `host-test,gpu-vivante,shaderc-online` build it too). Not in a
//! `report-es2` build: the entry points are ES 3.0.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glDeleteRenderbuffers,
    glFramebufferRenderbuffer, glFramebufferTexture2D, glGenFramebuffers, glGenRenderbuffers,
    glGetFramebufferAttachmentParameteriv, glInvalidateFramebuffer, glInvalidateSubFramebuffer, glReadPixels,
    glRenderbufferStorage,
};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host (tests/es3_blit_int_err.rs's rig) ------------ */

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
    name: c"es3-fbo-invalidate".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn get(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// The error of glInvalidateFramebuffer(target, atts).
fn inv(target: GLenum, atts: &[GLenum]) -> GLenum {
    // SAFETY: `atts` holds the count it is given.
    unsafe { glInvalidateFramebuffer(target, atts.len() as GLsizei, atts.as_ptr()) };
    err()
}

/// The error of glInvalidateSubFramebuffer(target, atts, x, y, w, h).
fn inv_sub(target: GLenum, atts: &[GLenum], x: GLint, y: GLint, w: GLsizei, h: GLsizei) -> GLenum {
    // SAFETY: `atts` holds the count it is given.
    unsafe { glInvalidateSubFramebuffer(target, atts.len() as GLsizei, atts.as_ptr(), x, y, w, h) };
    err()
}

const W: usize = 8;
const H: usize = 8;

/// The RGBA8 pattern the colour texture is filled with: every byte differs.
fn pattern() -> Vec<u8> {
    (0..W * H * 4).map(|i| (i * 7 + 3) as u8).collect()
}

/// A framebuffer object with an 8 x 8 RGBA8 texture holding [`pattern`] at
/// GL_COLOR_ATTACHMENT0 and a DEPTH24_STENCIL8 renderbuffer at
/// GL_DEPTH_STENCIL_ATTACHMENT, checked complete and left bound to
/// GL_FRAMEBUFFER: (framebuffer, texture, renderbuffer).
fn fbo() -> (GLuint, GLuint, GLuint) {
    let (mut fb, mut tex, mut rb) = (0, 0, 0);
    let px = pattern();
    // SAFETY: room for one name each; `px` holds W x H RGBA8 texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            W as GLsizei,
            H as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        );
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, W as GLsizei, H as GLsizei);
        assert_eq!(err(), GL_NO_ERROR, "the attachments' storage");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "the attachments");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (fb, tex, rb)
}

fn drop_fbo((fb, tex, rb): (GLuint, GLuint, GLuint)) {
    // SAFETY: the names were made by [`fbo`].
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
        glDeleteRenderbuffers(1, &rb);
    }
}

/// The colour attachment's pixels through glReadPixels of the bound read
/// framebuffer.
fn read_back() -> Vec<u8> {
    let mut px = vec![0u8; W * H * 4];
    // SAFETY: `px` holds W x H RGBA8 pixels.
    unsafe {
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void)
    };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

/// The name of what is attached at `attachment` of the bound framebuffer.
fn attached(attachment: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe {
        glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, attachment, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME, &mut v)
    };
    assert_eq!(err(), GL_NO_ERROR, "the attachment query of {attachment:#x}");
    v
}

/* ---- the errors ------------------------------------------------------------ */

#[test]
fn a_target_that_is_no_framebuffer_target_is_invalid_enum() {
    let _g = context();
    let f = fbo();
    for t in [u32::MAX, GL_BACK, GL_TEXTURE_2D, GL_RENDERBUFFER] {
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0]), GL_INVALID_ENUM, "target {t:#x}");
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT0], 0, 0, 4, 4), GL_INVALID_ENUM, "target {t:#x}");
    }
    drop_fbo(f);
}

#[test]
fn the_sub_rectangle_checks_the_target_before_the_size() {
    let _g = context();
    /* a bad target and a negative width or height: the target first */
    assert_eq!(inv_sub(GL_BACK, &[GL_COLOR], 0, 0, -1, 4), GL_INVALID_ENUM);
    assert_eq!(inv_sub(u32::MAX, &[GL_COLOR], 0, 0, 4, -1), GL_INVALID_ENUM);
    /* a negative size before the attachments: GL_COLOR_ATTACHMENT0 is no
     * default framebuffer attachment, but the size is refused first */
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0], 0, 0, -1, 4), GL_INVALID_VALUE);
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &[GL_COLOR], 0, 0, 4, -1), GL_INVALID_VALUE);
}

#[test]
fn a_negative_count_or_size_is_invalid_value() {
    let _g = context();
    let a = [GL_COLOR_ATTACHMENT0];
    // SAFETY: `a` holds one enum; a negative count reads none.
    unsafe { glInvalidateFramebuffer(GL_FRAMEBUFFER, -1, a.as_ptr()) };
    assert_eq!(err(), GL_INVALID_VALUE);
    // SAFETY: as above.
    unsafe { glInvalidateSubFramebuffer(GL_FRAMEBUFFER, -1, a.as_ptr(), 0, 0, 4, 4) };
    assert_eq!(err(), GL_INVALID_VALUE);
    let f = fbo();
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &a, 0, 0, -1, 4), GL_INVALID_VALUE);
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &a, 0, 0, 4, -1), GL_INVALID_VALUE);
    /* an empty rectangle and one past the edges are legal */
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &a, 0, 0, 0, 0), GL_NO_ERROR);
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &a, -4, -4, 64, 64), GL_NO_ERROR);
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &a, 100, 100, 4, 4), GL_NO_ERROR);
    drop_fbo(f);
}

#[test]
fn no_attachments_is_no_error() {
    let _g = context();
    // SAFETY: a zero count reads nothing.
    unsafe { glInvalidateFramebuffer(GL_FRAMEBUFFER, 0, core::ptr::null()) };
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: as above.
    unsafe { glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 0, core::ptr::null(), 0, 0, 4, 4) };
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn the_default_framebuffer_takes_color_depth_and_stencil_only() {
    let _g = context();
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        assert_eq!(inv(t, &[GL_COLOR, GL_DEPTH, GL_STENCIL]), GL_NO_ERROR, "target {t:#x}");
        assert_eq!(inv_sub(t, &[GL_COLOR, GL_DEPTH, GL_STENCIL], 0, 0, 8, 16), GL_NO_ERROR, "target {t:#x}");
        for a in [
            GL_COLOR_ATTACHMENT0,
            GL_DEPTH_ATTACHMENT,
            GL_STENCIL_ATTACHMENT,
            GL_DEPTH_STENCIL_ATTACHMENT,
            GL_BACK,
            GL_FRONT,
            GL_NONE,
        ] {
            assert_eq!(inv(t, &[a]), GL_INVALID_ENUM, "target {t:#x}, {a:#x}");
            assert_eq!(inv_sub(t, &[a], 0, 0, 4, 4), GL_INVALID_ENUM, "target {t:#x}, {a:#x}");
        }
        /* one bad entry fails the list */
        assert_eq!(inv(t, &[GL_COLOR, GL_DEPTH_ATTACHMENT]), GL_INVALID_ENUM, "target {t:#x}");
    }
}

#[test]
fn a_framebuffer_object_takes_its_attachment_points() {
    let _g = context();
    let f = fbo();
    let max = get(GL_MAX_COLOR_ATTACHMENTS);
    assert!(max >= 1, "GL_MAX_COLOR_ATTACHMENTS {max}");
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER] {
        for m in 0..max as GLenum {
            assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0 + m]), GL_NO_ERROR, "COLOR_ATTACHMENT{m}");
            assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT0 + m], 0, 0, 4, 8), GL_NO_ERROR, "COLOR_ATTACHMENT{m}");
        }
        for a in [GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT] {
            assert_eq!(inv(t, &[a]), GL_NO_ERROR, "{a:#x}");
            assert_eq!(inv_sub(t, &[a], 0, 0, 4, 8), GL_NO_ERROR, "{a:#x}");
        }
        assert_eq!(
            inv(t, &[GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT]),
            GL_NO_ERROR
        );
        /* a colour point at or past the limit (up to COLOR_ATTACHMENT31) */
        for m in max as GLenum..32 {
            assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0 + m]), GL_INVALID_OPERATION, "COLOR_ATTACHMENT{m}");
            assert_eq!(
                inv_sub(t, &[GL_COLOR_ATTACHMENT0 + m], 0, 0, 4, 4),
                GL_INVALID_OPERATION,
                "COLOR_ATTACHMENT{m}"
            );
        }
        /* the default framebuffer's names and other enums */
        for a in [GL_COLOR, GL_DEPTH, GL_STENCIL, GL_NONE, GL_BACK, GL_TEXTURE_2D] {
            assert_eq!(inv(t, &[a]), GL_INVALID_ENUM, "{a:#x}");
            assert_eq!(inv_sub(t, &[a], 0, 0, 4, 4), GL_INVALID_ENUM, "{a:#x}");
        }
        /* one bad entry fails the list */
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0, GL_COLOR]), GL_INVALID_ENUM);
    }
    drop_fbo(f);
}

#[test]
fn the_target_names_the_read_or_the_draw_binding_and_both_stay() {
    let _g = context();
    let f = fbo();
    let fb = f.0;
    // SAFETY: plain GL calls; `fb` was made above.
    unsafe {
        /* read: the object; draw: the window */
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
    }
    assert_eq!(inv(GL_READ_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0, GL_DEPTH_STENCIL_ATTACHMENT]), GL_NO_ERROR);
    assert_eq!(inv(GL_READ_FRAMEBUFFER, &[GL_COLOR]), GL_INVALID_ENUM);
    assert_eq!(inv_sub(GL_READ_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0], 0, 0, 4, 4), GL_NO_ERROR);
    assert_eq!(inv_sub(GL_READ_FRAMEBUFFER, &[GL_DEPTH], 0, 0, 4, 4), GL_INVALID_ENUM);
    assert_eq!(inv(GL_DRAW_FRAMEBUFFER, &[GL_COLOR, GL_DEPTH]), GL_NO_ERROR);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0]), GL_INVALID_ENUM);
    assert_eq!(get(GL_READ_FRAMEBUFFER_BINDING), fb as GLint);
    assert_eq!(get(GL_DRAW_FRAMEBUFFER_BINDING), 0);

    // SAFETY: as above.
    unsafe {
        /* read: the window; draw: the object */
        glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb);
    }
    assert_eq!(inv(GL_READ_FRAMEBUFFER, &[GL_COLOR, GL_STENCIL]), GL_NO_ERROR);
    assert_eq!(inv(GL_READ_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0]), GL_INVALID_ENUM);
    assert_eq!(inv_sub(GL_READ_FRAMEBUFFER, &[GL_COLOR], 0, 0, 4, 4), GL_NO_ERROR);
    assert_eq!(inv(GL_DRAW_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0]), GL_NO_ERROR);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_STENCIL_ATTACHMENT]), GL_NO_ERROR);
    assert_eq!(inv(GL_DRAW_FRAMEBUFFER, &[GL_COLOR]), GL_INVALID_ENUM);
    assert_eq!(get(GL_READ_FRAMEBUFFER_BINDING), 0);
    assert_eq!(get(GL_DRAW_FRAMEBUFFER_BINDING), fb as GLint);
    drop_fbo(f);
}

/* ---- what the calls leave behind -------------------------------------------- */

#[test]
fn an_invalidation_keeps_the_attachments_and_the_completeness() {
    let _g = context();
    let f = fbo();
    let all = [GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT];
    assert_eq!(inv(GL_FRAMEBUFFER, &all), GL_NO_ERROR);
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &all, 0, 0, 4, 8), GL_NO_ERROR);
    assert_eq!(get(GL_FRAMEBUFFER_BINDING), f.0 as GLint);
    assert_eq!(attached(GL_COLOR_ATTACHMENT0), f.1 as GLint);
    assert_eq!(attached(GL_DEPTH_ATTACHMENT), f.2 as GLint);
    assert_eq!(attached(GL_STENCIL_ATTACHMENT), f.2 as GLint);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
    drop_fbo(f);
}

#[test]
fn the_pixels_outside_the_sub_rectangle_are_kept() {
    let _g = context();
    let f = fbo();
    assert_eq!(read_back(), pattern(), "before");
    /* the left half, as fbo.invalidate.sub.* and .format.* invalidate it */
    assert_eq!(inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0], 0, 0, W as GLsizei / 2, H as GLsizei), GL_NO_ERROR);
    let px = read_back();
    let want = pattern();
    for y in 0..H {
        let right = (y * W + W / 2) * 4..(y * W + W) * 4;
        assert_eq!(px[right.clone()], want[right], "row {y}, the half not invalidated");
    }
    /* and this GL keeps the invalidated half too (undefined by the spec) */
    assert_eq!(px, want, "the invalidated half");
    drop_fbo(f);
}

#[test]
fn a_whole_invalidation_keeps_the_pixels() {
    let _g = context();
    let f = fbo();
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0, GL_DEPTH_STENCIL_ATTACHMENT]), GL_NO_ERROR);
    /* undefined by the spec; kept here, so a read after it is the image */
    assert_eq!(read_back(), pattern());
    /* an invalidation that fails changes nothing either */
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT0, GL_COLOR]), GL_INVALID_ENUM);
    assert_eq!(read_back(), pattern());
    drop_fbo(f);
}
