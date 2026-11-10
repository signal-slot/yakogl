// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3.functional.negative_api.buffer.read_pixels_fbo_format_mismatch
//! on the host (docs/notes/es3-negative-api.md section 3; es3fNegativeBufferApiTests.cpp).
//!
//! glReadPixels on a framebuffer whose colour format is incompatible with
//! the (format, type) pair is GL_INVALID_OPERATION: GL_FLOAT on an RGBA8,
//! RGBA32I or RGBA32UI texture. A 4-sample RGBA8 renderbuffer on the
//! framebuffer makes `GL_SAMPLE_BUFFERS` 1 and glReadPixels
//! GL_INVALID_OPERATION. The case failed only at the storage call
//! (`glRenderbufferStorageMultisample(4)` answered GL_INVALID_OPERATION), not
//! in glReadPixels; the multisample part waits for the multisample
//! renderbuffers of plan #339: while `GL_MAX_SAMPLES` is 1 (src/gl/framebuffer.rs:3484
//! refuses `samples > 1`) it skips itself and prints why; it runs once a back end offers 4.
//!
//! The multisample test skips itself, printing why, where the back end's
//! `GL_MAX_SAMPLES` is below 4. No GPU runs: the stand-in draws nothing.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glReadPixels, glRenderbufferStorageMultisample,
};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
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
    name: c"es3-negapi-readpixels".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(64, 64).is_null());
        });
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A framebuffer (bound) with a 32x32 texture of (internal format, format,
/// type) as its colour attachment. `None`, with the reason printed, where the
/// back end offers no such texture or colour target (the GC7000UL takes no
/// 32-bit integer texture: GL_INVALID_ENUM at glTexImage2D).
fn texture_fbo(internal: GLenum, format: GLenum, ty: GLenum) -> Option<GLuint> {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; no pixels.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 32, 32, 0, format, ty, core::ptr::null());
        let e = glGetError();
        if e != GL_NO_ERROR {
            eprintln!("skipped: a {internal:#x} texture is error {e:#x} on this back end");
            return None;
        }
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if st != GL_FRAMEBUFFER_COMPLETE {
            eprintln!("skipped: a {internal:#x} colour target is status {st:#x} on this back end");
            return None;
        }
    }
    Some(fbo)
}

fn read(format: GLenum, ty: GLenum) -> GLenum {
    let mut buf = [0u8; 4 * 4 * 16];
    // SAFETY: 4x4 pixels of up to 16 bytes fit `buf`; an error reads nothing.
    unsafe { glReadPixels(0, 0, 4, 4, format, ty, buf.as_mut_ptr() as *mut c_void) };
    err()
}

#[test]
fn rgba8_texture_read_as_float_is_invalid_operation() {
    let _g = context();
    assert!(texture_fbo(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE).is_some(), "every back end renders RGBA8");
    assert_eq!(read(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION);
}

#[test]
fn rgba32i_texture_read_as_float_is_invalid_operation() {
    let _g = context();
    if texture_fbo(GL_RGBA32I, GL_RGBA_INTEGER, GL_INT).is_none() {
        return;
    }
    assert_eq!(read(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION);
}

#[test]
fn rgba32ui_texture_read_as_float_is_invalid_operation() {
    let _g = context();
    if texture_fbo(GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT).is_none() {
        return;
    }
    assert_eq!(read(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION);
}

/// The dEQP sequence after the three texture reads: a 4-sample RGBA8
/// renderbuffer, a complete FBO, SAMPLE_BUFFERS 1, glReadPixels
/// GL_INVALID_OPERATION.
#[test]
fn multisample_renderbuffer_read_is_invalid_operation() {
    let _g = context();
    let mut max = 0;
    // SAFETY: one GLint.
    unsafe { glGetIntegerv(GL_MAX_SAMPLES, &mut max) };
    if max < 4 {
        eprintln!("skipped: GL_MAX_SAMPLES is {max} (< 4) on this back end");
        return;
    }
    let (mut fbo, mut rb) = (0, 0);
    // SAFETY: room for one name each; one GLint.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 32, 32);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let mut sb = 0;
        glGetIntegerv(GL_SAMPLE_BUFFERS, &mut sb);
        assert_eq!(err(), GL_NO_ERROR, "storage, attach, status and the query raise no error");
        assert_eq!(sb, 1, "GL_SAMPLE_BUFFERS");
    }
    assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE), GL_INVALID_OPERATION);
}
