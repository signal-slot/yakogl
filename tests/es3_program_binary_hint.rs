// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_PROGRAM_BINARY_RETRIEVABLE_HINT (ES 3.0.6 2.12.3 and 6.1.12; dEQP-GLES3
//! state_query.shader.program_binary): glProgramParameteri stores the hint
//! on the program object and glGetProgramiv returns it; the default is
//! GL_FALSE. A value other than GL_TRUE/GL_FALSE is GL_INVALID_VALUE and an
//! unknown pname GL_INVALID_ENUM.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Not in a `report-es2` build: ES 2.0 has neither command.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3::glProgramParameteri;
use yakogl::gl::program::{glCreateProgram, glDeleteProgram, glGetProgramiv};
use yakogl::gl::state::glGetError;
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
    name: c"es3-program-binary-hint".as_ptr() as *const c_char,
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

fn hint(p: GLuint) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one GLint of room.
    unsafe { glGetProgramiv(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, &mut v) };
    v
}

#[test]
fn the_hint_is_stored_and_returned() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        assert_eq!(hint(p), GL_FALSE as GLint, "the default");
        assert_eq!(glGetError(), GL_NO_ERROR);
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(hint(p), GL_TRUE as GLint);
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_FALSE as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(hint(p), GL_FALSE as GLint);
        glDeleteProgram(p);
    }
}

#[test]
fn a_bad_value_or_pname_is_an_error_and_keeps_the_hint() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, 2);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        assert_eq!(hint(p), GL_TRUE as GLint, "a refused value changes nothing");
        glProgramParameteri(p, GL_TEXTURE_WRAP_S, GL_TRUE as GLint);
        assert_eq!(glGetError(), GL_INVALID_ENUM);
        glDeleteProgram(p);
    }
}

#[test]
fn a_new_program_starts_at_false_again() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        glDeleteProgram(p);
        let q = glCreateProgram();
        assert_eq!(hint(q), GL_FALSE as GLint);
        glDeleteProgram(q);
    }
}
