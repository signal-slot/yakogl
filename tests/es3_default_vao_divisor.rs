// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The divisor state of the default VAO and of a named VAO, read back with
//! glGetVertexAttribiv(GL_VERTEX_ATTRIB_ARRAY_DIVISOR): the call sequence of
//! dEQP-GLES3.functional.default_vertex_array_object.vertex_attrib_divisor
//! (tools/cts/gles3-mustpass.txt:1591, es3fDefaultVertexArrayObjectTests.cpp
//! at VK-GL-CTS 93bca01). The case itself is glBindVertexArray(0),
//! glVertexAttribDivisor(0, 3), glGetError() == GL_NO_ERROR ("not an error on
//! the default VAO in ES 3.0"); it queries nothing. The tests here add the
//! read-back with the same entry points: the initial value of every attribute
//! is 0, and ES 3.0 6.2 (table 6.2): the divisor
//! is vertex array object state, so each VAO owns its own and the default VAO
//! keeps its values while a named VAO is bound.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_default_vao_divisor
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,pvr-instance-divisor --test es3_default_vao_divisor
//! ```
//!
//! The table is a hook-less one with the PowerVR's `max_instance_divisor` (1,
//! or 0xffff under the feature; the real table links the C generators). The
//! divisor query is the GL core's and does not depend on it; the last test
//! reads values back across the limit.

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::{GLint, GLuint};
use yakogl::gl::vao::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL that makes one ----------------- */

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

/// A hook-less table with the PowerVR's `max_instance_divisor` (1, or 0xffff
/// under `pvr-instance-divisor`). The real table is not named: it links the C
/// command generators, which a state test needs no more than the GPU.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-default-vao-divisor".as_ptr() as *const c_char,
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
    max_instance_divisor: if cfg!(feature = "pvr-instance-divisor") { 0xffff } else { 1 },
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindVertexArray(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn gen() -> GLuint {
    let mut v = 0;
    // SAFETY: room for one name.
    unsafe { glGenVertexArrays(1, &mut v) };
    assert_ne!(v, 0);
    v
}

fn max_attribs() -> u32 {
    let mut n = 0;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut n) };
    assert!(n >= 16, "ES 3.0 minimum");
    n as u32
}

fn div(i: u32) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribiv(i, GL_VERTEX_ATTRIB_ARRAY_DIVISOR, &mut v) };
    v
}

fn divf(i: u32) -> f32 {
    let mut v = -1.0;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribfv(i, GL_VERTEX_ATTRIB_ARRAY_DIVISOR, &mut v) };
    v
}

/// A divisor for attribute `i` of VAO number `salt`: 1..=5, different per
/// attribute so that a swapped or shared slot shows.
fn want(i: u32, salt: u32) -> u32 {
    1 + (i * 7 + salt * 3) % 5
}

/// The dEQP case verbatim: a divisor on the default VAO raises no error.
#[test]
fn deqp_vertex_attrib_divisor_default_vao_no_error() {
    let _g = context();
    unsafe {
        glBindVertexArray(0);
        glVertexAttribDivisor(0, 3);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    assert_eq!(div(0), 3);
    unsafe { glVertexAttribDivisor(0, 0) };
}

/// The initial value of every attribute of the default VAO is 0, as is that of
/// a new named VAO.
#[test]
fn initial_divisor_is_zero() {
    let _g = context();
    let n = max_attribs();
    for i in 0..n {
        assert_eq!(div(i), 0, "default VAO, attribute {i}");
        assert_eq!(divf(i), 0.0);
    }
    let v = gen();
    unsafe { glBindVertexArray(v) };
    for i in 0..n {
        assert_eq!(div(i), 0, "new VAO, attribute {i}");
    }
    unsafe {
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// Set a divisor on every attribute of the default VAO and read each back, as an int and as a float; then reset to 0 and read again.
#[test]
fn default_vao_set_and_read_back() {
    let _g = context();
    let n = max_attribs();
    for i in 0..n {
        unsafe { glVertexAttribDivisor(i, want(i, 0)) };
        assert_eq!(div(i), want(i, 0) as GLint, "attribute {i}");
        assert_eq!(divf(i), want(i, 0) as f32);
    }
    for i in 0..n {
        assert_eq!(div(i), want(i, 0) as GLint, "no attribute disturbs another ({i})");
    }
    for i in 0..n {
        unsafe { glVertexAttribDivisor(i, 0) };
        assert_eq!(div(i), 0, "back to 0, attribute {i}");
    }
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
}

/// The default VAO keeps its divisors across a bind and unbind of a named VAO;
/// the named VAO has its own, which it keeps across the same trip the other
/// way, and a change on it stays off the default.
#[test]
fn default_and_named_vao_keep_their_own_divisors() {
    let _g = context();
    let n = max_attribs();
    let v = gen();
    for i in 0..n {
        unsafe { glVertexAttribDivisor(i, want(i, 0)) };
    }
    unsafe { glBindVertexArray(v) };
    for i in 0..n {
        assert_eq!(div(i), 0, "the named VAO starts at 0, not at the default's ({i})");
        unsafe { glVertexAttribDivisor(i, want(i, 1) + 10) };
        assert_eq!(div(i), (want(i, 1) + 10) as GLint, "named VAO, attribute {i}");
    }
    unsafe { glBindVertexArray(0) };
    for i in 0..n {
        assert_eq!(div(i), want(i, 0) as GLint, "default VAO after the trip, attribute {i}");
    }
    unsafe { glBindVertexArray(v) };
    for i in 0..n {
        assert_eq!(div(i), (want(i, 1) + 10) as GLint, "named VAO after the trip, attribute {i}");
    }
    unsafe {
        glVertexAttribDivisor(3, 0);
        glBindVertexArray(0);
    }
    assert_eq!(div(3), want(3, 0) as GLint, "a change on the named VAO stays off the default");
    unsafe {
        for i in 0..n {
            glVertexAttribDivisor(i, 0);
        }
        glDeleteVertexArrays(1, &v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// A divisor above the PowerVR's hardware limit reads back unchanged: the
/// state is the GL core's, the back end only decides how a draw takes it.
#[test]
fn divisor_reads_back_across_the_back_end_limit() {
    let _g = context();
    unsafe {
        for d in [1u32, 0xffff, 0x10000, 0x7fff_ffff] {
            glVertexAttribDivisor(5, d);
            assert_eq!(div(5), d as GLint, "divisor {d:#x}");
        }
        glVertexAttribDivisor(5, 0);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}
