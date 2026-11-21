// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The typed current vertex value is context state, not vertex array object
//! state: it survives VAO binds and deletes (docs/es3-cts-failures.md
//! Appendix A, task T1).
//!
//! ES 3.0 6.2 (state tables): a vertex array object captures the attribute
//! *arrays* (table 6.2), while the current value of each attribute, with its
//! float/int/uint type as set by glVertexAttrib* / glVertexAttribI4*, is
//! vertex array data *not in vertex array objects* (table 6.3). The VAO swap
//! must therefore never touch it. tests/es3_current_attrib.rs pins the
//! setters and the getters on one context; here the same value and its tag
//! are read back across glBindVertexArray, across a second setter of another
//! type while another VAO is bound, across glDeleteVertexArrays of the bound
//! VAO, and across a setter of the other family.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_current_attrib_vao
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::{GLint, GLuint, GL_FALSE_B};
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
    name: c"es3-current-attrib-vao".as_ptr() as *const c_char,
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

fn gen() -> GLuint {
    let mut v = 0;
    // SAFETY: room for one name.
    unsafe { glGenVertexArrays(1, &mut v) };
    assert_ne!(v, 0, "the VAO pool (15 names) is never exhausted here: every name is deleted");
    v
}

fn binding() -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(GL_VERTEX_ARRAY_BINDING, &mut v) };
    v
}

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
fn enabled(i: u32) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribiv(i, GL_VERTEX_ATTRIB_ARRAY_ENABLED, &mut v) };
    v
}

/// (a) An int value set with the default VAO bound keeps its value and its int
/// tag when another VAO is bound: the bind swaps the attribute arrays, not the
/// current values (ES 3.0 6.2, table 6.3).
#[test]
fn an_int_value_survives_binding_a_new_vao() {
    let _g = context();
    let v = gen();
    unsafe {
        glBindVertexArray(0);
        glVertexAttribI4i(2, -1, i32::MIN, i32::MAX, 7);
    }
    let before = (getii(2), geti(2), getf(2), getui(2));
    assert_eq!(before.0, [-1, i32::MIN, i32::MAX, 7], "exact as an int before the bind");
    unsafe { glBindVertexArray(v) };
    assert_eq!(binding(), v as GLint, "the new VAO really is bound");
    assert_eq!(getii(2), before.0, "the int value and its type follow the context, not the VAO");
    assert_eq!(geti(2), before.1);
    assert_eq!(getf(2), before.2, "an int value read as a float: converted, not the bit pattern");
    assert_eq!(getui(2), before.3, "negatives read as 0");
    unsafe {
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// (b) A uint value set while another VAO is bound is what a rebind of VAO 0
/// reads: VAO state does not save or restore the current values, so VAO 0
/// takes back its *arrays* only and the value set under the other VAO stands.
#[test]
fn binding_vao_0_again_reads_the_value_set_under_another_vao() {
    let _g = context();
    let v = gen();
    unsafe {
        glBindVertexArray(0);
        glVertexAttribI4i(2, 1, 2, 3, -4);
        glBindVertexArray(v);
        glVertexAttribI4ui(2, 0, 1, BIG, u32::MAX);
        glBindVertexArray(0);
        assert_eq!(binding(), 0);
    }
    assert_eq!(getui(2), [0, 1, BIG, u32::MAX], "exact as a uint: the old int value is gone");
    assert_eq!(getii(2), [0, 1, i32::MAX, i32::MAX], "the uint tag followed the value");
    assert_eq!(getf(2), [0.0, 1.0, BIG as f32, u32::MAX as f32]);
    unsafe {
        glDeleteVertexArrays(1, &v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// (c) Deleting the bound VAO keeps the value: the delete swaps the default
/// VAO's arrays back in and reuses the slot, and touches no current value.
#[test]
fn deleting_the_bound_vao_keeps_the_value_and_its_type() {
    let _g = context();
    let v = gen();
    unsafe {
        glBindVertexArray(0);
        glVertexAttribI4ui(2, BIG, 0, 1, 2);
        glBindVertexArray(v);
        glDeleteVertexArrays(1, &v);
        assert_eq!(binding(), 0, "the binding reverts to the default VAO");
        assert_eq!(glIsVertexArray(v), GL_FALSE_B, "the name is gone");
    }
    assert_eq!(getui(2), [BIG, 0, 1, 2]);
    assert_eq!(getii(2), [i32::MAX, 0, 1, 2], "still typed uint: the int read saturates");
    assert_eq!(getf(2), [BIG as f32, 0.0, 1.0, 2.0]);
    unsafe { assert_eq!(glGetError(), GL_NO_ERROR) };
}

/// (d) glVertexAttrib4f after an int setter retags the value as a float (the
/// last setter decides, tests/es3_current_attrib.rs): the float getter is
/// exact and the integer getters convert as there -- rounded to nearest,
/// halves away from zero, negatives to 0 -- and the new tag survives a bind.
#[test]
fn a_float_setter_after_an_int_one_retags_the_value_across_a_bind() {
    let _g = context();
    let v = gen();
    unsafe {
        glBindVertexArray(0);
        glVertexAttribI4i(2, -1, i32::MAX, -2, 3);
    }
    assert_eq!(getii(2), [-1, i32::MAX, -2, 3]);
    unsafe { glVertexAttrib4f(2, 1.5, -2.5, 3.5, -0.5) };
    assert_eq!(getf(2), [1.5, -2.5, 3.5, -0.5], "exact as a float");
    assert_eq!(geti(2), [2, -3, 4, -1], "rounded to nearest, halves away from zero");
    assert_eq!(getii(2), [2, -3, 4, -1]);
    assert_eq!(getui(2), [2, 0, 4, 0], "negatives read as 0");
    unsafe { glBindVertexArray(v) };
    assert_eq!(getf(2), [1.5, -2.5, 3.5, -0.5], "the float tag follows the context");
    assert_eq!(getii(2), [2, -3, 4, -1]);
    unsafe {
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// A new VAO starts with default arrays while the current values set under
/// the default VAO stand: the bind swaps table 6.2 state, not table 6.3 state.
#[test]
fn a_new_vao_swaps_the_arrays_and_not_the_current_values() {
    let _g = context();
    let v = gen();
    unsafe {
        glBindVertexArray(0);
        glVertexAttribI4i(2, 11, 22, 33, 44);
        glEnableVertexAttribArray(2);
        glBindVertexArray(v);
    }
    assert_eq!(binding(), v as GLint);
    assert_eq!(enabled(2), 0, "a fresh VAO: attribute 2's array is disabled again");
    assert_eq!(getii(2), [11, 22, 33, 44], "and its current value is where it was");
    unsafe {
        glBindVertexArray(0);
        glDisableVertexAttribArray(2);
        glDeleteVertexArrays(1, &v);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}
