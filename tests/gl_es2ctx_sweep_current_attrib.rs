// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0-context guard for bd114dcc ("gl: the current vertex value remembers
//! its type, the getters convert as Mesa does"), item es2ctx-guards-more.
//!
//! bd114dcc adds `GlContext::current_attrib_type` and makes
//! `glGetVertexAttribfv/iv` convert a `GL_CURRENT_VERTEX_ATTRIB` value by it.
//! tests/es3_current_attrib.rs covers the ES 3.0 pairs. An ES 2.0 context only
//! has `glVertexAttrib{1,2,3,4}f[v]` and the two getters (ES 2.0 2.7, 6.1.8),
//! so every value it can ever store is a float, and what it reads back must
//! be what ES 2.0 says:
//!
//! - the initial value is (0, 0, 0, 1) on every attribute;
//! - `1f`, `2f`, `3f`, `4f` and the `fv` forms fill the missing components
//!   with 0, 0, 1 (y, z, w) and touch no other attribute;
//! - `glGetVertexAttribfv` returns the stored floats bit for bit (a negative
//!   zero, a huge value and an infinity survive);
//! - `glGetVertexAttribiv` rounds to nearest, halves away from zero
//!   (ES 2.0 6.1.8: floats are converted by rounding), saturating;
//! - an index past `GL_MAX_VERTEX_ATTRIBS` is GL_INVALID_VALUE and stores
//!   and writes nothing; an unknown pname is GL_INVALID_ENUM.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test gl_es2ctx_sweep_current_attrib
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test gl_es2ctx_sweep_current_attrib
//! ```
//!
//! Compiled only with `report-es2`: an ES 3.0 build is not the context this
//! guards.

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
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
    name: c"es2ctx-sweep-current-attrib".as_ptr() as *const c_char,
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

fn max_attribs() -> GLuint {
    let mut n = 0;
    // SAFETY: plain GL call, room for one value.
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut n) };
    assert_eq!(err(), GL_NO_ERROR);
    n as GLuint
}

fn getf(index: GLuint) -> [f32; 4] {
    let mut v = [f32::NAN; 4];
    // SAFETY: plain GL call, room for four values.
    unsafe { glGetVertexAttribfv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn bits(v: [f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}

fn geti(index: GLuint) -> [GLint; 4] {
    let mut v = [i32::MIN; 4];
    // SAFETY: plain GL call, room for four values.
    unsafe { glGetVertexAttribiv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

#[test]
fn the_initial_current_value_is_0_0_0_1_on_every_attribute() {
    let _g = context();
    // attributes 0..n not yet written by any test of this file read the reset
    // value: ask the highest, which no test below sets
    let last = max_attribs() - 1;
    assert_eq!(bits(getf(last)), bits([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(geti(last), [0, 0, 0, 1]);
}

#[test]
fn the_f_forms_fill_the_missing_components_and_touch_no_other_attribute() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttrib4f(1, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib4f(3, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib1f(2, 5.0);
    }
    assert_eq!(getf(2), [5.0, 0.0, 0.0, 1.0]);
    // SAFETY: plain GL call.
    unsafe { glVertexAttrib2f(2, 5.0, 6.0) };
    assert_eq!(getf(2), [5.0, 6.0, 0.0, 1.0]);
    // SAFETY: plain GL call.
    unsafe { glVertexAttrib3f(2, 5.0, 6.0, 7.0) };
    assert_eq!(getf(2), [5.0, 6.0, 7.0, 1.0]);
    // SAFETY: plain GL call.
    unsafe { glVertexAttrib4f(2, 5.0, 6.0, 7.0, 8.0) };
    assert_eq!(getf(2), [5.0, 6.0, 7.0, 8.0]);
    assert_eq!(getf(1), [9.0; 4], "the neighbours are not touched");
    assert_eq!(getf(3), [9.0; 4]);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn the_fv_forms_fill_the_missing_components() {
    let _g = context();
    let v = [1.5f32, -2.5, 3.5, -4.5];
    // SAFETY: each call reads as many floats as its name says, v has four.
    unsafe {
        glVertexAttrib1fv(4, v.as_ptr());
        assert_eq!(getf(4), [1.5, 0.0, 0.0, 1.0]);
        glVertexAttrib2fv(4, v.as_ptr());
        assert_eq!(getf(4), [1.5, -2.5, 0.0, 1.0]);
        glVertexAttrib3fv(4, v.as_ptr());
        assert_eq!(getf(4), [1.5, -2.5, 3.5, 1.0]);
        glVertexAttrib4fv(4, v.as_ptr());
        assert_eq!(getf(4), v);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn the_float_getter_returns_the_stored_bits() {
    let _g = context();
    let v = [-0.0f32, 3.0e38, f32::INFINITY, f32::MIN_POSITIVE / 4.0];
    // SAFETY: plain GL call.
    unsafe { glVertexAttrib4f(5, v[0], v[1], v[2], v[3]) };
    assert_eq!(bits(getf(5)), bits(v));
}

#[test]
fn the_integer_getter_rounds_to_nearest_and_saturates() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe { glVertexAttrib4f(6, 0.5, -0.5, 2.4, -2.6) };
    assert_eq!(geti(6), [1, -1, 2, -3]);
    // SAFETY: plain GL calls.
    unsafe { glVertexAttrib4f(6, 7.0, -7.0, 1.0e20, -1.0e20) };
    assert_eq!(geti(6), [7, -7, i32::MAX, i32::MIN]);
}

#[test]
fn a_bad_index_or_pname_is_an_error_that_stores_and_writes_nothing() {
    let _g = context();
    let n = max_attribs();
    // SAFETY: plain GL calls.
    unsafe { glVertexAttrib4f(7, 1.0, 2.0, 3.0, 4.0) };
    // SAFETY: plain GL call.
    unsafe { glVertexAttrib4f(n, 8.0, 8.0, 8.0, 8.0) };
    assert_eq!(err(), GL_INVALID_VALUE);
    assert_eq!(getf(7), [1.0, 2.0, 3.0, 4.0]);
    let mut f = [f32::NAN; 4];
    let mut i = [i32::MIN; 4];
    // SAFETY: plain GL calls, room for four values.
    unsafe {
        glGetVertexAttribfv(n, GL_CURRENT_VERTEX_ATTRIB, f.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glGetVertexAttribiv(n, GL_CURRENT_VERTEX_ATTRIB, i.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glGetVertexAttribfv(7, 0x0001, f.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_ENUM);
        glGetVertexAttribiv(7, 0x0001, i.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_ENUM);
    }
    assert!(f.iter().all(|x| x.is_nan()) && i == [i32::MIN; 4], "a refused query writes nothing");
}
