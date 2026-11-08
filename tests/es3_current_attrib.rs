// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The typed current vertex value, through the public GL API on the host
//! (docs/es3-cts-failures.md Appendix A, task T1).
//!
//! ES 3.0 2.7 and 6.1.14: a current attribute value is a float, an int or a
//! uint vector, as the command that set it; glGetVertexAttribfv/iv/Iiv/Iuiv
//! with GL_CURRENT_VERTEX_ATTRIB convert it to the type they return. Every
//! (setter, getter) pair, the initial (0, 0, 0, 1) and a context reset.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_current_attrib
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
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
    name: c"es3-current-attrib".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
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
    // SAFETY: plain GL calls on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

const BIG: u32 = 0xFFFF_FFF0;

fn getf(i: u32) -> [f32; 4] {
    let mut v = [0f32; 4];
    unsafe { glGetVertexAttribfv(i, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}
fn geti(i: u32) -> [i32; 4] {
    let mut v = [0i32; 4];
    unsafe { glGetVertexAttribiv(i, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}
fn getii(i: u32) -> [i32; 4] {
    let mut v = [0i32; 4];
    unsafe { glGetVertexAttribIiv(i, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}
fn getui(i: u32) -> [u32; 4] {
    let mut v = [0u32; 4];
    unsafe { glGetVertexAttribIuiv(i, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}

#[test]
fn initial_value_is_a_float_0_0_0_1() {
    let _g = context();
    assert_eq!(getf(5), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(geti(5), [0, 0, 0, 1]);
    assert_eq!(getii(5), [0, 0, 0, 1]);
    assert_eq!(getui(5), [0, 0, 0, 1]);
}

#[test]
fn float_value_reads_as_float_and_converts_to_integers() {
    let _g = context();
    unsafe { glVertexAttrib4f(1, 1.5, -2.5, 3.4, -0.4) };
    assert_eq!(getf(1), [1.5, -2.5, 3.4, -0.4]);
    assert_eq!(geti(1), [2, -3, 3, 0]);
    assert_eq!(getii(1), [2, -3, 3, 0]);
    assert_eq!(getui(1), [2, 0, 3, 0]);
    unsafe { glVertexAttrib2f(1, 7.0, 8.0) };
    assert_eq!(getf(1), [7.0, 8.0, 0.0, 1.0]);
    assert_eq!(getii(1), [7, 8, 0, 1]);
    unsafe { assert_eq!(glGetError(), GL_NO_ERROR) };
}

#[test]
fn int_value_reads_exactly_as_int_and_converts_to_float_and_uint() {
    let _g = context();
    unsafe { glVertexAttribI4i(2, -1, i32::MIN, i32::MAX, 5) };
    assert_eq!(getii(2), [-1, i32::MIN, i32::MAX, 5]);
    assert_eq!(geti(2), [-1, i32::MIN, i32::MAX, 5]);
    assert_eq!(getf(2), [-1.0, i32::MIN as f32, i32::MAX as f32, 5.0]);
    assert_eq!(getui(2), [0, 0, i32::MAX as u32, 5]);
    let v = [9i32, -9, 0, 1];
    unsafe { glVertexAttribI4iv(2, v.as_ptr()) };
    assert_eq!(getii(2), v);
    assert_eq!(getf(2), [9.0, -9.0, 0.0, 1.0]);
}

#[test]
fn uint_value_reads_exactly_as_uint_and_converts_to_float_and_int() {
    let _g = context();
    unsafe { glVertexAttribI4ui(3, 0, 1, BIG, u32::MAX) };
    assert_eq!(getui(3), [0, 1, BIG, u32::MAX]);
    assert_eq!(getf(3), [0.0, 1.0, BIG as f32, u32::MAX as f32]);
    assert_eq!(getii(3), [0, 1, i32::MAX, i32::MAX]);
    assert_eq!(geti(3), [0, 1, i32::MAX, i32::MAX]);
    let v = [4u32, 3, 2, 1];
    unsafe { glVertexAttribI4uiv(3, v.as_ptr()) };
    assert_eq!(getui(3), v);
}

#[test]
fn the_last_setter_decides_the_type() {
    let _g = context();
    unsafe { glVertexAttribI4i(4, -7, 0, 0, 0) };
    unsafe { glVertexAttrib4f(4, 0.5, 0.0, 0.0, 1.0) };
    assert_eq!(getf(4), [0.5, 0.0, 0.0, 1.0]);
    unsafe { glVertexAttribI4ui(4, 7, 0, 0, 0) };
    assert_eq!(getf(4), [7.0, 0.0, 0.0, 0.0]);
    // another attribute is not touched
    assert_eq!(getf(6), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn a_bad_index_is_an_error() {
    let _g = context();
    let mut v = [0f32; 4];
    unsafe {
        glGetVertexAttribfv(99, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr());
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glVertexAttribI4i(99, 0, 0, 0, 0);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
    }
}
