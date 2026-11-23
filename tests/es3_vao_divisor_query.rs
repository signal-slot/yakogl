// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_VERTEX_ATTRIB_ARRAY_DIVISOR and GL_VERTEX_ATTRIB_ARRAY_INTEGER queries,
//! per vertex array object, through the public GL API on the host.
//!
//! ES 3.0 6.1.14: glGetVertexAttrib{f,i,Ii,Iui}v return both values of the
//! bound VAO's attribute array (Mesa: varray.c get_vertex_array_attrib). The
//! divisor is stored by glVertexAttribDivisor, the integer flag by
//! glVertexAttribIPointer (glVertexAttribPointer clears it).
//! An ES 2.0-reporting build (report-es2) does not know these pnames, so the
//! file is not built there.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_vao_divisor_query
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
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

/// All four getters; each must agree.
fn query(i: u32, pname: u32) -> i64 {
    let (mut f, mut iv, mut ii, mut iu) = (-1f32, -1i32, -1i32, u32::MAX);
    unsafe {
        glGetVertexAttribfv(i, pname, &mut f);
        glGetVertexAttribiv(i, pname, &mut iv);
        glGetVertexAttribIiv(i, pname, &mut ii);
        glGetVertexAttribIuiv(i, pname, &mut iu);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    assert_eq!(iv, ii);
    assert_eq!(iv as u32, iu);
    assert_eq!(f, iv as f32);
    iv as i64
}
fn divisor(i: u32) -> i64 {
    query(i, GL_VERTEX_ATTRIB_ARRAY_DIVISOR)
}
fn integer(i: u32) -> i64 {
    query(i, GL_VERTEX_ATTRIB_ARRAY_INTEGER)
}

fn gen_bind() -> u32 {
    let mut v = 0u32;
    unsafe {
        glGenVertexArrays(1, &mut v);
        glBindVertexArray(v);
    }
    v
}

#[test]
fn default_divisor_is_0_and_integer_is_false() {
    let _g = context();
    let _v = gen_bind();
    for i in [0, 1, 7] {
        assert_eq!(divisor(i), 0);
        assert_eq!(integer(i), GL_FALSE as i64);
    }
    unsafe { glBindVertexArray(0) };
}

#[test]
fn divisor_reads_back_exactly() {
    let _g = context();
    let _v = gen_bind();
    for d in [3u32, 0xffff, 0x10000, 1, 0x10000] {
        unsafe { glVertexAttribDivisor(2, d) };
        assert_eq!(divisor(2), d as i64);
    }
    assert_eq!(divisor(1), 0, "another index is untouched");
    unsafe { glBindVertexArray(0) };
}

#[test]
fn divisor_is_per_vao() {
    let _g = context();
    let a = gen_bind();
    unsafe { glVertexAttribDivisor(4, 3) };
    let b = gen_bind();
    assert_eq!(divisor(4), 0, "a new VAO starts at 0");
    unsafe { glVertexAttribDivisor(4, 0xffff) };
    unsafe { glBindVertexArray(a) };
    assert_eq!(divisor(4), 3);
    unsafe { glBindVertexArray(b) };
    assert_eq!(divisor(4), 0xffff);
    unsafe { glBindVertexArray(a) };
    assert_eq!(divisor(4), 3);
    unsafe { glBindVertexArray(0) };
}

#[test]
fn integer_follows_the_last_pointer_command() {
    let _g = context();
    let _v = gen_bind();
    let mut buf = 0u32;
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_ARRAY_BUFFER, buf);
        glVertexAttribIPointer(3, 2, GL_INT, 0, core::ptr::null());
    }
    assert_eq!(integer(3), GL_TRUE as i64);
    assert_eq!(integer(2), GL_FALSE as i64);
    unsafe { glVertexAttribPointer(3, 2, GL_FLOAT, 0, 0, core::ptr::null()) };
    assert_eq!(integer(3), GL_FALSE as i64);
    unsafe { glVertexAttribIPointer(3, 4, GL_UNSIGNED_BYTE, 0, core::ptr::null()) };
    assert_eq!(integer(3), GL_TRUE as i64);
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &buf);
        glBindVertexArray(0);
    }
}

#[test]
fn integer_is_per_vao() {
    let _g = context();
    let mut buf = 0u32;
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_ARRAY_BUFFER, buf);
    }
    let a = gen_bind();
    unsafe { glVertexAttribIPointer(1, 1, GL_INT, 0, core::ptr::null()) };
    let b = gen_bind();
    assert_eq!(integer(1), GL_FALSE as i64);
    unsafe { glBindVertexArray(a) };
    assert_eq!(integer(1), GL_TRUE as i64);
    unsafe { glBindVertexArray(b) };
    assert_eq!(integer(1), GL_FALSE as i64);
    unsafe {
        glBindVertexArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &buf);
    }
}

#[test]
fn divisor_index_out_of_range_is_invalid_value() {
    let _g = context();
    let mut max = 0i32;
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut max) };
    assert!(max > 0);
    unsafe {
        glVertexAttribDivisor(max as u32, 2);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glVertexAttribDivisor(u32::MAX, 2);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glVertexAttribDivisor(max as u32 - 1, 2);
        assert_eq!(glGetError(), GL_NO_ERROR);
        glVertexAttribDivisor(max as u32 - 1, 0);
    }
}
