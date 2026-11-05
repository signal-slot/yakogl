// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The integer-colour errors of glBlitFramebuffer (ES 3.0.6 4.3.3; dEQP-GLES3
//! negative_api.buffer.blit_framebuffer): GL_INVALID_OPERATION when the mask
//! has COLOR and the read buffer is integer while a draw buffer is not (or
//! the reverse), when signed and unsigned integer meet, and when the filter
//! is GL_LINEAR with an integer read buffer. An integer texture has no copy
//! path in the GL core (gl/blit.rs `attachment_image` gives it no image, and
//! the colour bit was dropped before the kinds were compared); the errors
//! must come before the colour is dropped for that.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glBlitFramebuffer, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glRenderbufferStorage,
};
use yakogl::gl::state::glGetError;
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
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
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/* ---- the tests ------------------------------------------------------------ */

/// A framebuffer with one 8x8 colour renderbuffer of `fmt`.
fn fbo(fmt: GLenum) -> GLuint {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls, room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, fmt, 8, 8);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    fb
}

/// A framebuffer with one 8x8 colour texture of the sized format `fmt`; None
/// when the build cannot make that texture.
fn fbo_tex(fmt: GLenum) -> Option<GLuint> {
    let (mut fb, mut tex) = (0, 0);
    let (format, ty) = match fmt {
        GL_RGBA32I => (GL_RGBA_INTEGER, GL_INT),
        GL_RGBA32UI => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        _ => (GL_RGBA, GL_UNSIGNED_BYTE),
    };
    // SAFETY: plain GL calls, room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt as GLint, 8, 8, 0, format, ty, core::ptr::null());
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        /* a build without that texture format (the Vivante one) refuses it here */
        if glGetError() != GL_NO_ERROR {
            return None;
        }
    }
    Some(fb)
}

/// Blit the whole 8x8 colour from a framebuffer of `src` to one of `dst`;
/// the error it leaves.
fn blit(src: GLenum, dst: GLenum, filter: GLenum) -> GLenum {
    blit_with(fbo(src), fbo(dst), filter)
}

/// The same with texture attachments (what dEQP's negative_api case uses).
fn blit_tex(src: GLenum, dst: GLenum, filter: GLenum) -> GLenum {
    blit_with(fbo_tex(src).expect("a texture"), fbo_tex(dst).expect("a texture"), filter)
}

fn blit_with(s: GLuint, d: GLuint, filter: GLenum) -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, s);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, d);
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, filter);
        let e = glGetError();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        e
    }
}

#[test]
fn integer_and_float_buffers_do_not_blit() {
    let _g = context();
    assert_eq!(blit(GL_RGBA32I, GL_RGBA8, GL_NEAREST), GL_INVALID_OPERATION, "int to float");
    assert_eq!(blit(GL_RGBA32UI, GL_RGBA8, GL_NEAREST), GL_INVALID_OPERATION, "uint to float");
    assert_eq!(blit(GL_RGBA8, GL_RGBA32I, GL_NEAREST), GL_INVALID_OPERATION, "float to int");
    assert_eq!(blit(GL_RGBA8, GL_RGBA32UI, GL_NEAREST), GL_INVALID_OPERATION, "float to uint");
}

#[test]
fn signed_and_unsigned_do_not_blit() {
    let _g = context();
    assert_eq!(blit(GL_RGBA32UI, GL_RGBA32I, GL_NEAREST), GL_INVALID_OPERATION, "uint to int");
    assert_eq!(blit(GL_RGBA32I, GL_RGBA32UI, GL_NEAREST), GL_INVALID_OPERATION, "int to uint");
}

#[test]
fn linear_with_an_integer_read_buffer_is_refused() {
    let _g = context();
    assert_eq!(blit(GL_RGBA32I, GL_RGBA32I, GL_LINEAR), GL_INVALID_OPERATION);
    assert_eq!(blit(GL_RGBA32UI, GL_RGBA32UI, GL_LINEAR), GL_INVALID_OPERATION);
}

#[test]
fn the_same_errors_for_texture_attachments() {
    let _g = context();
    if fbo_tex(GL_RGBA32I).is_none() || fbo_tex(GL_RGBA32UI).is_none() {
        return;
    }
    assert_eq!(blit_tex(GL_RGBA32I, GL_RGBA8, GL_NEAREST), GL_INVALID_OPERATION, "int to float");
    assert_eq!(blit_tex(GL_RGBA32UI, GL_RGBA8, GL_NEAREST), GL_INVALID_OPERATION, "uint to float");
    assert_eq!(blit_tex(GL_RGBA8, GL_RGBA32I, GL_NEAREST), GL_INVALID_OPERATION, "float to int");
    assert_eq!(blit_tex(GL_RGBA32UI, GL_RGBA32I, GL_NEAREST), GL_INVALID_OPERATION, "uint to int");
    assert_eq!(blit_tex(GL_RGBA32I, GL_RGBA32I, GL_LINEAR), GL_INVALID_OPERATION, "LINEAR");
    assert_eq!(blit_tex(GL_RGBA32I, GL_RGBA32I, GL_NEAREST), GL_NO_ERROR);
}

#[test]
fn integer_to_integer_nearest_is_no_error() {
    let _g = context();
    assert_eq!(blit(GL_RGBA32I, GL_RGBA32I, GL_NEAREST), GL_NO_ERROR);
    assert_eq!(blit(GL_RGBA32UI, GL_RGBA32UI, GL_NEAREST), GL_NO_ERROR);
}
