// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glTexSubImage2D / glTexSubImage3D (format, type) against the texture's
//! internal format (ES 3.0.6 3.8.5, table 3.2; dEQP-GLES3
//! negative_api.texture.texsubimage2d and texsubimage3d): `GL_RGB` with
//! `GL_FLOAT` into an unsized `GL_RGBA` texture is GL_INVALID_OPERATION
//! (GL_OES_texture_float is not claimed), while `GL_FLOAT` into the sized
//! float formats and into depth textures still works.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Not in a `report-es2` build: the ES 2.0 surface unpacks `GL_FLOAT`.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D, glTexImage3D, glTexSubImage2D, glTexSubImage3D};
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
    name: c"es3-negapi-texsub".as_ptr() as *const c_char,
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

const NULL: *const c_void = core::ptr::null();

/// A new bound 32 x 32 2D texture of `internalformat`, level 0.
fn tex2d(internalformat: GLenum, format: GLenum, type_: GLenum) {
    let mut t = 0;
    // SAFETY: room for one name; no pixels; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 32, 32, 0, format, type_, NULL);
    }
    assert_eq!(err(), GL_NO_ERROR, "texture {internalformat:#x}");
}

/// A new bound 4 x 4 x 4 3D texture.
fn tex3d(internalformat: GLenum, format: GLenum, type_: GLenum) {
    let mut t = 0;
    // SAFETY: room for one name; no pixels; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_3D, t);
        glTexImage3D(GL_TEXTURE_3D, 0, internalformat as GLint, 4, 4, 4, 0, format, type_, NULL);
    }
    assert_eq!(err(), GL_NO_ERROR, "texture {internalformat:#x}");
}

/// The error of a 4 x 4 sub-image at the origin read from `data`.
fn sub2d(format: GLenum, type_: GLenum, data: *const c_void) -> GLenum {
    // SAFETY: `data` is null or holds 4 x 4 texels of at most 16 bytes.
    unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, format, type_, data) };
    err()
}

/// The error of a 4 x 4 x 4 sub-image at the origin read from `data`.
fn sub3d(format: GLenum, type_: GLenum, data: *const c_void) -> GLenum {
    // SAFETY: `data` is null or holds 4 x 4 x 4 texels of at most 16 bytes.
    unsafe { glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, format, type_, data) };
    err()
}

/* ---- negative_api.texture.texsubimage2d ----------------------------------- */

#[test]
fn texsubimage2d_deqp_sequence() {
    let _g = context();
    tex2d(GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE);
    // SAFETY: plain GL calls, null pixels.
    unsafe {
        glTexSubImage2D(0, 0, 0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, 4, GL_UNSIGNED_BYTE, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGB, 0, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
    }
    assert_eq!(sub2d(GL_RGBA, GL_UNSIGNED_SHORT_5_6_5, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub2d(GL_RGB, GL_UNSIGNED_SHORT_4_4_4_4, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub2d(GL_RGB, GL_UNSIGNED_SHORT_5_5_5_1, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub2d(GL_RGBA_INTEGER, GL_UNSIGNED_INT, NULL), GL_INVALID_OPERATION);
    /* the row that was accepted */
    assert_eq!(sub2d(GL_RGB, GL_FLOAT, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub2d(GL_RGBA, GL_FLOAT, NULL), GL_INVALID_OPERATION);
}

#[test]
fn texsubimage2d_float_with_data_is_refused_too() {
    let _g = context();
    tex2d(GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE);
    let px = vec![0u8; 4 * 4 * 16];
    assert_eq!(sub2d(GL_RGB, GL_FLOAT, px.as_ptr() as *const c_void), GL_INVALID_OPERATION);
}

#[test]
fn texsubimage2d_controls_still_pass() {
    let _g = context();
    let px = vec![0u8; 4 * 4 * 16];
    let p = px.as_ptr() as *const c_void;
    tex2d(GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE);
    assert_eq!(sub2d(GL_RGBA, GL_UNSIGNED_BYTE, p), GL_NO_ERROR);
    assert_eq!(sub2d(GL_RGB, GL_UNSIGNED_BYTE, p), GL_NO_ERROR);
    tex2d(GL_RGB32F, GL_RGB, GL_FLOAT);
    assert_eq!(sub2d(GL_RGB, GL_FLOAT, p), GL_NO_ERROR);
    tex2d(GL_R32F, GL_RED, GL_FLOAT);
    assert_eq!(sub2d(GL_RED, GL_FLOAT, p), GL_NO_ERROR);
    tex2d(GL_RGBA32F, GL_RGBA, GL_FLOAT);
    assert_eq!(sub2d(GL_RGBA, GL_FLOAT, p), GL_NO_ERROR);
    tex2d(GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);
    assert_eq!(sub2d(GL_DEPTH_COMPONENT, GL_FLOAT, p), GL_NO_ERROR);
}

/* ---- negative_api.texture.texsubimage3d ----------------------------------- */

#[test]
fn texsubimage3d_deqp_sequence() {
    let _g = context();
    tex3d(GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE);
    // SAFETY: plain GL calls, null pixels.
    unsafe {
        glTexSubImage3D(0, 0, 0, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
        glTexSubImage3D(GL_TEXTURE_2D, 0, 0, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
        glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, 0, GL_UNSIGNED_BYTE, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
        glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGB, 0, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
    }
    assert_eq!(sub3d(GL_RGB, GL_UNSIGNED_SHORT_4_4_4_4, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub3d(GL_RGB, GL_UNSIGNED_SHORT_5_5_5_1, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub3d(GL_RGBA_INTEGER, GL_UNSIGNED_INT, NULL), GL_INVALID_OPERATION);
    assert_eq!(sub3d(GL_RGB, GL_FLOAT, NULL), GL_INVALID_OPERATION);
}

#[test]
fn texsubimage3d_controls_still_pass() {
    let _g = context();
    let px = vec![0u8; 4 * 4 * 4 * 16];
    let p = px.as_ptr() as *const c_void;
    tex3d(GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE);
    assert_eq!(sub3d(GL_RGBA, GL_UNSIGNED_BYTE, p), GL_NO_ERROR);
    tex3d(GL_RGB32F, GL_RGB, GL_FLOAT);
    assert_eq!(sub3d(GL_RGB, GL_FLOAT, p), GL_NO_ERROR);
    tex3d(GL_R32F, GL_RED, GL_FLOAT);
    assert_eq!(sub3d(GL_RED, GL_FLOAT, p), GL_NO_ERROR);
}
