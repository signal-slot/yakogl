// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-VAO vertex array state GL_VERTEX_ATTRIB_ARRAY_DIVISOR and
//! GL_VERTEX_ATTRIB_ARRAY_INTEGER, through glGetVertexAttribiv/fv/Iiv/Iuiv
//! (ES 3.0 6.1.14; Mesa's get_vertex_array_attrib). The divisor is the one the
//! GC back end sends to the hardware stream state.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_vao_divisor_query
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::vao::*;
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
    name: c"es3-vao-divisor-query".as_ptr() as *const c_char,
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
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

const DIV: u32 = GL_VERTEX_ATTRIB_ARRAY_DIVISOR;
const INT: u32 = GL_VERTEX_ATTRIB_ARRAY_INTEGER;

/// All four getters for one pname: they must agree.
fn get(i: u32, pname: u32) -> u32 {
    let (mut a, mut b, mut c, mut d) = (-7i32, -7.0f32, -7i32, 7u32);
    unsafe {
        glGetVertexAttribiv(i, pname, &mut a);
        glGetVertexAttribfv(i, pname, &mut b);
        glGetVertexAttribIiv(i, pname, &mut c);
        glGetVertexAttribIuiv(i, pname, &mut d);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    assert_eq!(a as u32, d, "iv vs Iuiv, pname {pname:#x}");
    assert_eq!(c as u32, d, "Iiv vs Iuiv, pname {pname:#x}");
    // a float holds the divisors used here exactly
    assert_eq!(b, d as f32, "fv vs Iuiv, pname {pname:#x}");
    d
}

fn gen_vao() -> u32 {
    let mut v = 0u32;
    unsafe { glGenVertexArrays(1, &mut v) };
    v
}

#[test]
fn defaults_are_zero_and_false() {
    let _g = context();
    for i in [0, 3, 15] {
        assert_eq!(get(i, DIV), 0);
        assert_eq!(get(i, INT), 0);
    }
}

#[test]
fn divisor_round_trips_exactly() {
    let _g = context();
    unsafe {
        for d in [3u32, 0xffff, 0x10000, 1, 0] {
            glVertexAttribDivisor(2, d);
            assert_eq!(get(2, DIV), d);
            assert_eq!(get(1, DIV), 0, "other index untouched");
        }
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn divisor_is_per_vao() {
    let _g = context();
    unsafe {
        let (a, b) = (gen_vao(), gen_vao());
        glBindVertexArray(a);
        glVertexAttribDivisor(1, 3);
        glBindVertexArray(b);
        assert_eq!(get(1, DIV), 0, "a new VAO starts at 0");
        glVertexAttribDivisor(1, 0x10000);
        glBindVertexArray(a);
        assert_eq!(get(1, DIV), 3);
        glBindVertexArray(b);
        assert_eq!(get(1, DIV), 0x10000);
        glBindVertexArray(0);
        assert_eq!(get(1, DIV), 0, "VAO 0 has its own");
        glBindVertexArray(a);
        glVertexAttribDivisor(1, 0xffff);
        glBindVertexArray(b);
        assert_eq!(get(1, DIV), 0x10000);
        glBindVertexArray(a);
        assert_eq!(get(1, DIV), 0xffff);
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &a);
        glDeleteVertexArrays(1, &b);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn integer_flag_follows_the_pointer_command_and_is_per_vao() {
    let _g = context();
    unsafe {
        let (a, b) = (gen_vao(), gen_vao());
        glBindVertexArray(a);
        assert_eq!(get(4, INT), 0);
        glVertexAttribIPointer(4, 2, GL_INT, 0, core::ptr::null());
        assert_eq!(get(4, INT), 1);
        glBindVertexArray(b);
        assert_eq!(get(4, INT), 0);
        glVertexAttribPointer(4, 2, GL_FLOAT, 0, 0, core::ptr::null());
        assert_eq!(get(4, INT), 0);
        glBindVertexArray(a);
        assert_eq!(get(4, INT), 1, "b's float pointer left a alone");
        glVertexAttribPointer(4, 4, GL_FLOAT, 0, 0, core::ptr::null());
        assert_eq!(get(4, INT), 0, "a float pointer clears it");
        glVertexAttribIPointer(4, 1, GL_UNSIGNED_INT, 0, core::ptr::null());
        assert_eq!(get(4, INT), 1);
        // the divisor is independent of the integer flag
        glVertexAttribDivisor(4, 5);
        assert_eq!(get(4, INT), 1);
        assert_eq!(get(4, DIV), 5);
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &a);
        glDeleteVertexArrays(1, &b);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn divisor_index_out_of_range_is_invalid_value() {
    let _g = context();
    unsafe {
        let mut max = 0i32;
        glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut max);
        assert!(max >= 16);
        let max = max as u32;
        glVertexAttribDivisor(max, 2);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glVertexAttribDivisor(u32::MAX, 2);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        // the last valid index works
        glVertexAttribDivisor(max - 1, 2);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(get(max - 1, DIV), 2);
        glVertexAttribDivisor(max - 1, 0);
        // the getters reject the index too
        let mut v = 0i32;
        glGetVertexAttribiv(max, DIV, &mut v);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
    }
}
