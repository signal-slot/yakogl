// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0-context guard for 6f7d7772 ("gl: program-binary retrievable hint
//! stored and returned"), item es2ctx-guards-more.
//!
//! 6f7d7772 keeps `GL_PROGRAM_BINARY_RETRIEVABLE_HINT` per program (set by
//! `glProgramParameteri`) and answers it from `glGetProgramiv`. ES 2.0 6.1.8
//! has nine `glGetProgramiv` pnames and no program binary: the query for the
//! hint or `GL_PROGRAM_BINARY_LENGTH` is GL_INVALID_ENUM in an ES 2.0
//! context, whatever `glProgramParameteri` stored (an ES 3.0 entry point an
//! ES 2.0 client never calls, kept answering as before). Pinned:
//!
//! - after `glProgramParameteri(p, HINT, GL_TRUE)` the ES 2.0 context still
//!   refuses `glGetProgramiv(p, HINT)` and `(p, BINARY_LENGTH)` with
//!   GL_INVALID_ENUM and writes nothing;
//! - the nine ES 2.0 pnames of a fresh program answer as before (all 0, the
//!   info log empty) with no error;
//! - `glProgramParameteri` keeps its checks: another pname is GL_INVALID_ENUM,
//!   a value other than GL_TRUE/GL_FALSE is GL_INVALID_VALUE, a name that is
//!   no program is GL_INVALID_OPERATION.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test gl_es2ctx_sweep_program_hint
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test gl_es2ctx_sweep_program_hint
//! ```
//!
//! Compiled only with `report-es2`: an ES 3.0 build answers the hint query
//! on purpose (tests/es3_program_binary_hint.rs).

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::glProgramParameteri;
use yakogl::gl::program::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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
    name: c"es2ctx-sweep-program-hint".as_ptr() as *const c_char,
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    drain();
    g
}

fn drain() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

/// The first error; the queue is empty again afterwards.
fn err() -> GLenum {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    drain();
    e
}

const SENTINEL: GLint = 0x5EED;

/// `glGetProgramiv`: the error and the value (`SENTINEL` if nothing was written).
fn get(p: GLuint, pname: GLenum) -> (GLenum, GLint) {
    let mut v = SENTINEL;
    // SAFETY: plain GL call, room for one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    (err(), v)
}

#[test]
fn the_hint_and_binary_length_stay_invalid_enum_after_the_hint_is_set() {
    let _g = context();
    // SAFETY: plain GL call.
    let p = unsafe { glCreateProgram() };
    assert_ne!(p, 0);
    for pname in [GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_PROGRAM_BINARY_LENGTH] {
        assert_eq!(get(p, pname), (GL_INVALID_ENUM, SENTINEL), "before, pname {pname:#x}");
    }
    // SAFETY: plain GL call.
    unsafe { glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint) };
    assert_eq!(err(), GL_NO_ERROR);
    for pname in [GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_PROGRAM_BINARY_LENGTH] {
        assert_eq!(get(p, pname), (GL_INVALID_ENUM, SENTINEL), "after, pname {pname:#x}");
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn the_nine_es2_pnames_of_a_fresh_program_are_unmoved() {
    let _g = context();
    // SAFETY: plain GL call.
    let p = unsafe { glCreateProgram() };
    for (name, pname) in [
        ("DELETE_STATUS", GL_DELETE_STATUS),
        ("LINK_STATUS", GL_LINK_STATUS),
        ("VALIDATE_STATUS", GL_VALIDATE_STATUS),
        ("ATTACHED_SHADERS", GL_ATTACHED_SHADERS),
        ("ACTIVE_ATTRIBUTES", GL_ACTIVE_ATTRIBUTES),
        ("ACTIVE_ATTRIBUTE_MAX_LENGTH", GL_ACTIVE_ATTRIBUTE_MAX_LENGTH),
        ("ACTIVE_UNIFORMS", GL_ACTIVE_UNIFORMS),
        ("ACTIVE_UNIFORM_MAX_LENGTH", GL_ACTIVE_UNIFORM_MAX_LENGTH),
        ("INFO_LOG_LENGTH", GL_INFO_LOG_LENGTH),
    ] {
        assert_eq!(get(p, pname), (GL_NO_ERROR, 0), "{name}");
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn glprogramparameteri_keeps_its_checks() {
    let _g = context();
    // SAFETY: plain GL call.
    let p = unsafe { glCreateProgram() };
    // SAFETY: plain GL calls.
    unsafe {
        glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, 2);
        assert_eq!(err(), GL_INVALID_VALUE);
        glProgramParameteri(p, GL_LINK_STATUS, GL_TRUE as GLint);
        assert_eq!(err(), GL_INVALID_ENUM);
        glProgramParameteri(p + 1000, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glDeleteProgram(p);
    }
    assert_eq!(err(), GL_NO_ERROR);
}
