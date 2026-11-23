// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The short forms of the current vertex value setters, through the public
//! GL API on the host (docs/es3-cts-failures.md Appendix A, task T1).
//!
//! ES 3.0 2.7: glVertexAttrib{1,2,3}f and their fv forms fill the components
//! they leave out with (y, z, w) = (0, 0, 1) and make the value a float;
//! glVertexAttribI4iv/I4uiv make it an int/uint; a later float call retags
//! it; an index >= GL_MAX_VERTEX_ATTRIBS is GL_INVALID_VALUE for every form.
//! tests/es3_current_attrib.rs covers the typed getters, this file the forms.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_current_attrib_forms
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
    name: c"es3-current-attrib-forms".as_ptr() as *const c_char,
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

fn max_attribs() -> u32 {
    let mut n = 0i32;
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut n) };
    assert!(n >= 16);
    n as u32
}

#[test]
fn one_two_three_float_forms_fill_y_z_w_with_0_0_1() {
    let _g = context();
    unsafe {
        glVertexAttrib4f(1, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib1f(1, 2.5);
        assert_eq!(getf(1), [2.5, 0.0, 0.0, 1.0]);
        glVertexAttrib4f(1, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib2f(1, 2.5, -3.5);
        assert_eq!(getf(1), [2.5, -3.5, 0.0, 1.0]);
        glVertexAttrib4f(1, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib3f(1, 2.5, -3.5, 4.5);
        assert_eq!(getf(1), [2.5, -3.5, 4.5, 1.0]);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn one_two_three_four_fv_forms_fill_y_z_w_with_0_0_1() {
    let _g = context();
    let v = [1.5f32, 2.5, 3.5, 4.5];
    unsafe {
        glVertexAttrib4f(2, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib1fv(2, v.as_ptr());
        assert_eq!(getf(2), [1.5, 0.0, 0.0, 1.0]);
        glVertexAttrib4f(2, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib2fv(2, v.as_ptr());
        assert_eq!(getf(2), [1.5, 2.5, 0.0, 1.0]);
        glVertexAttrib4f(2, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib3fv(2, v.as_ptr());
        assert_eq!(getf(2), [1.5, 2.5, 3.5, 1.0]);
        glVertexAttrib4f(2, 9.0, 9.0, 9.0, 9.0);
        glVertexAttrib4fv(2, v.as_ptr());
        assert_eq!(getf(2), v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn short_float_forms_read_back_through_the_integer_getters() {
    let _g = context();
    unsafe { glVertexAttrib2f(3, 6.0, 7.0) };
    assert_eq!(geti(3), [6, 7, 0, 1]);
    assert_eq!(getii(3), [6, 7, 0, 1]);
    assert_eq!(getui(3), [6, 7, 0, 1]);
}

#[test]
fn short_float_form_after_an_int_call_retags_to_float() {
    let _g = context();
    unsafe {
        glVertexAttribI4i(4, -5, -6, -7, -8);
        glVertexAttrib1f(4, 0.5);
        // float tag: the float getter returns the float value, not raw dwords
        assert_eq!(getf(4), [0.5, 0.0, 0.0, 1.0]);
        glVertexAttribI4ui(4, 1, 2, 3, 4);
        glVertexAttrib3fv(4, [0.25f32, 0.5, 0.75].as_ptr());
        assert_eq!(getf(4), [0.25, 0.5, 0.75, 1.0]);
    }
}

#[test]
fn int_vector_form_sets_the_int_tag() {
    let _g = context();
    let v = [-1i32, i32::MIN, 3, i32::MAX];
    unsafe {
        glVertexAttrib4f(5, 1.5, 1.5, 1.5, 1.5);
        glVertexAttribI4iv(5, v.as_ptr());
    }
    assert_eq!(getii(5), v);
    assert_eq!(geti(5), v);
    assert_eq!(getf(5), [-1.0, i32::MIN as f32, 3.0, i32::MAX as f32]);
}

#[test]
fn uint_vector_form_sets_the_uint_tag() {
    let _g = context();
    let v = [0u32, 1, BIG, u32::MAX];
    unsafe {
        glVertexAttrib4f(6, 1.5, 1.5, 1.5, 1.5);
        glVertexAttribI4uiv(6, v.as_ptr());
    }
    assert_eq!(getui(6), v);
    assert_eq!(getf(6), [0.0, 1.0, BIG as f32, u32::MAX as f32]);
}

#[test]
fn int_then_uint_then_float_vector_forms_each_retag() {
    let _g = context();
    unsafe {
        glVertexAttribI4iv(7, [-1i32, -2, -3, -4].as_ptr());
        assert_eq!(getii(7), [-1, -2, -3, -4]);
        glVertexAttribI4uiv(7, [u32::MAX, 2, 3, 4].as_ptr());
        assert_eq!(getui(7), [u32::MAX, 2, 3, 4]);
        glVertexAttrib4fv(7, [-1.5f32, 2.0, 3.0, 4.0].as_ptr());
        assert_eq!(getf(7), [-1.5, 2.0, 3.0, 4.0]);
        assert_eq!(geti(7), [-2, 2, 3, 4]);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

#[test]
fn every_form_rejects_an_index_at_max_vertex_attribs() {
    let _g = context();
    let n = max_attribs();
    let f = [1.0f32; 4];
    let i = [1i32; 4];
    let u = [1u32; 4];
    for idx in [n, n + 1, u32::MAX] {
        unsafe {
            glVertexAttrib1f(idx, 1.0);
            assert_eq!(glGetError(), GL_INVALID_VALUE, "1f {idx}");
            glVertexAttrib2f(idx, 1.0, 1.0);
            assert_eq!(glGetError(), GL_INVALID_VALUE, "2f {idx}");
            glVertexAttrib3f(idx, 1.0, 1.0, 1.0);
            assert_eq!(glGetError(), GL_INVALID_VALUE, "3f {idx}");
            glVertexAttrib4f(idx, 1.0, 1.0, 1.0, 1.0);
            assert_eq!(glGetError(), GL_INVALID_VALUE, "4f {idx}");
            glVertexAttrib1fv(idx, f.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "1fv {idx}");
            glVertexAttrib2fv(idx, f.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "2fv {idx}");
            glVertexAttrib3fv(idx, f.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "3fv {idx}");
            glVertexAttrib4fv(idx, f.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "4fv {idx}");
            glVertexAttribI4i(idx, 1, 1, 1, 1);
            assert_eq!(glGetError(), GL_INVALID_VALUE, "I4i {idx}");
            glVertexAttribI4ui(idx, 1, 1, 1, 1);
            assert_eq!(glGetError(), GL_INVALID_VALUE, "I4ui {idx}");
            glVertexAttribI4iv(idx, i.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "I4iv {idx}");
            glVertexAttribI4uiv(idx, u.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "I4uiv {idx}");
        }
    }
}

#[test]
fn the_last_valid_index_is_accepted_by_every_form() {
    let _g = context();
    let last = max_attribs() - 1;
    unsafe {
        glVertexAttrib1f(last, 3.0);
        assert_eq!(getf(last), [3.0, 0.0, 0.0, 1.0]);
        glVertexAttribI4iv(last, [1i32, 2, 3, 4].as_ptr());
        assert_eq!(getii(last), [1, 2, 3, 4]);
        glVertexAttribI4uiv(last, [5u32, 6, 7, 8].as_ptr());
        assert_eq!(getui(last), [5, 6, 7, 8]);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}
