// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer's colour-format errors (ES 3.0.6 4.3.3; dEQP-GLES3
//! negative_api.buffer.blit_framebuffer): with GL_COLOR_BUFFER_BIT in the
//! mask, GL_INVALID_OPERATION when the read buffer holds integers and a
//! draw buffer does not (or the reverse), when signed and unsigned integers
//! meet, and when the filter is GL_LINEAR with an integer read buffer.
//!
//! The images are RGBA32I, RGBA32UI, R16F and RGBA8 textures, the integer
//! ones as in the dEQP case: the GL core copies no integer texture
//! (gl/blit.rs `attachment_image`), which until 2026-11-07 dropped the
//! colour bit before these checks, so each such blit answered GL_NO_ERROR. A valid integer blit still answers
//! GL_NO_ERROR.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there (gl/framebuffer.rs `blit_framebuffer_is_es3_only`).

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBlitFramebuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glFramebufferTexture2D,
    glGenFramebuffers,
};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host ---------------------------------------------- */

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
    name: c"es3-blit-int-err".as_ptr() as *const c_char,
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
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn blit(filter: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, filter) };
}

/* ---- objects ----------------------------------------------------------- */

const RGBA32I: (GLenum, GLenum, GLenum) = (GL_RGBA32I, GL_RGBA_INTEGER, GL_INT);
const RGBA32UI: (GLenum, GLenum, GLenum) = (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT);
const R16F: (GLenum, GLenum, GLenum) = (GL_R16F, GL_RED, GL_HALF_FLOAT);
const RGBA8: (GLenum, GLenum, GLenum) = (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE);

/// The eight-bit integer format of the same signedness, for a back end that
/// samples no 32-bit integer texels (the GC7000's `GC_SAMPLED_TEXFMTS`): the
/// GL core copies neither (`attachment_image`), so the blit's checks see
/// the same case.
fn narrow(fmt: (GLenum, GLenum, GLenum)) -> (GLenum, GLenum, GLenum) {
    match fmt.0 {
        GL_RGBA32I => (GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE),
        GL_RGBA32UI => (GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE),
        _ => fmt,
    }
}

/// An 8 x 8 texture of `fmt` (internal format, format, type; an integer
/// one the build does not sample [`narrow`]ed) at GL_COLOR_ATTACHMENT0 of a
/// new framebuffer, checked complete: (framebuffer, texture).
fn colour_fbo(fmt: (GLenum, GLenum, GLenum)) -> (GLuint, GLuint) {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; no pixels; plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt.0 as GLint, 8, 8, 0, fmt.1, fmt.2, core::ptr::null());
        if err() != GL_NO_ERROR && narrow(fmt) != fmt {
            let n = narrow(fmt);
            glTexImage2D(GL_TEXTURE_2D, 0, n.0 as GLint, 8, 8, 0, n.1, n.2, core::ptr::null());
        }
        assert_eq!(err(), GL_NO_ERROR, "a {:#x} texture", fmt.0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "a {:#x} framebuffer", fmt.0);
    }
    (fb, tex)
}

/// The error of a colour blit from a `read` image into a `draw` one with
/// `filter`.
fn blit_error(read: (GLenum, GLenum, GLenum), draw: (GLenum, GLenum, GLenum), filter: GLenum) -> GLenum {
    let (rfb, rtex) = colour_fbo(read);
    let (dfb, dtex) = colour_fbo(draw);
    // SAFETY: plain GL calls; the names were made above.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, rfb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dfb);
        blit(filter);
        let e = err();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &rfb);
        glDeleteFramebuffers(1, &dfb);
        glDeleteTextures(1, &rtex);
        glDeleteTextures(1, &dtex);
        e
    }
}

/* ---- the errors ---------------------------------------------------------- */

#[test]
fn an_integer_read_buffer_into_a_float_draw_buffer_is_invalid_operation() {
    let _g = context();
    assert_eq!(blit_error(RGBA32I, R16F, GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit_error(RGBA32I, RGBA8, GL_NEAREST), GL_INVALID_OPERATION);
}

#[test]
fn unsigned_into_signed_integers_is_invalid_operation() {
    let _g = context();
    assert_eq!(blit_error(RGBA32UI, RGBA32I, GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit_error(RGBA32I, RGBA32UI, GL_NEAREST), GL_INVALID_OPERATION);
}

#[test]
fn a_float_read_buffer_into_an_integer_draw_buffer_is_invalid_operation() {
    let _g = context();
    assert_eq!(blit_error(R16F, RGBA32I, GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit_error(RGBA8, RGBA32UI, GL_NEAREST), GL_INVALID_OPERATION);
}

#[test]
fn linear_with_an_integer_read_buffer_is_invalid_operation() {
    let _g = context();
    assert_eq!(blit_error(RGBA32I, RGBA32I, GL_LINEAR), GL_INVALID_OPERATION);
    assert_eq!(blit_error(RGBA32UI, RGBA32UI, GL_LINEAR), GL_INVALID_OPERATION);
}

#[test]
fn a_nearest_blit_between_integer_images_of_one_kind_is_no_error() {
    let _g = context();
    assert_eq!(blit_error(RGBA32I, RGBA32I, GL_NEAREST), GL_NO_ERROR);
    assert_eq!(blit_error(RGBA32UI, RGBA32UI, GL_NEAREST), GL_NO_ERROR);
    /* and float into float, LINEAR included */
    assert_eq!(blit_error(RGBA8, RGBA8, GL_LINEAR), GL_NO_ERROR);
}
