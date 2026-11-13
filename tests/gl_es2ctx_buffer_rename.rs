// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Buffer delete and rename in an ES 2.0 context: da0eede3 (a deleted
//! buffer's name bound again is a new buffer, the objects holding the old
//! one keep its storage) changed src/gl/buffer.rs, tf.rs and vao.rs and was
//! tested only for ES 3.0 (tests/es3_tf_lifetime.rs). This replays the ES 2.0
//! sequences of dEQP-GLES2 functional.buffer.*, functional.vertex_arrays.*
//! and functional.negative_api.buffer.*: glGenBuffers, glBindBuffer,
//! glBufferData, glVertexAttribPointer, glDeleteBuffers while bound (ES 2.0
//! 2.9: every binding to it in the current context reverts to 0), glIsBuffer,
//! then the name used again by glBindBuffer and glBufferData.
//!
//! Asserted: glGetError after each call, GL_ARRAY_BUFFER_BINDING,
//! GL_ELEMENT_ARRAY_BUFFER_BINDING, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING and
//! GL_BUFFER_SIZE / GL_BUFFER_USAGE.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test gl_es2ctx_buffer_rename
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_es2ctx_buffer_rename
//! ```
//!
//! The context is the smallest OSAL and back end that make one, as in
//! tests/es3_tf_lifetime.rs. Nothing is drawn.

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
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
    name: c"es2ctx-buffer-rename".as_ptr() as *const c_char,
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
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLuint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v as GLuint
}

fn attrib_buffer(index: GLuint) -> GLuint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribiv(index, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut v) };
    v as GLuint
}

fn gen_buffer() -> GLuint {
    let mut b = 0;
    // SAFETY: one name.
    unsafe { glGenBuffers(1, &mut b) };
    assert_ne!(b, 0);
    b
}

fn buffer_param(target: GLenum, pname: GLenum) -> i32 {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetBufferParameteriv(target, pname, &mut v) };
    v
}

fn data(target: GLenum, n: usize, byte: u8, usage: GLenum) {
    let d = vec![byte; n];
    // SAFETY: `d` holds `n` bytes.
    unsafe { glBufferData(target, n as GLsizeiptr, d.as_ptr().cast(), usage) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn bind(target: GLenum, name: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glBindBuffer(target, name) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn is_buffer(name: GLuint) -> bool {
    // SAFETY: plain GL call.
    unsafe { glIsBuffer(name) == GL_TRUE_B }
}

fn delete(name: GLuint) {
    // SAFETY: one name.
    unsafe { glDeleteBuffers(1, &name) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn pointer(index: GLuint) {
    // SAFETY: offset 0 into the bound buffer.
    unsafe { glVertexAttribPointer(index, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null()) };
    assert_eq!(err(), GL_NO_ERROR);
}

/// buffer.* delete while bound, then glIsBuffer: the name is gone, the
/// binding is 0, and glBindBuffer on the stale name makes a new, empty buffer.
#[test]
fn delete_while_bound_reverts_the_binding_and_the_name_is_reusable() {
    let _g = context();
    let b = gen_buffer();
    assert!(!is_buffer(b), "a generated name is not a buffer until bound");
    bind(GL_ARRAY_BUFFER, b);
    assert!(is_buffer(b));
    data(GL_ARRAY_BUFFER, 64, 0x11, GL_STATIC_DRAW);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 64);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), GL_STATIC_DRAW as i32);
    assert_eq!(geti(GL_ARRAY_BUFFER_BINDING), b);

    delete(b);
    assert_eq!(geti(GL_ARRAY_BUFFER_BINDING), 0, "ES 2.0 2.9: the binding reverts to 0");
    assert!(!is_buffer(b));
    assert_eq!(err(), GL_NO_ERROR);

    // glBindBuffer on a deleted name: a new buffer with no data and defaults.
    bind(GL_ARRAY_BUFFER, b);
    assert!(is_buffer(b));
    assert_eq!(geti(GL_ARRAY_BUFFER_BINDING), b);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 0, "the new buffer is empty");
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), GL_STATIC_DRAW as i32);
    data(GL_ARRAY_BUFFER, 24, 0x22, GL_DYNAMIC_DRAW);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 24);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), GL_DYNAMIC_DRAW as i32);
    bind(GL_ARRAY_BUFFER, 0);
    delete(b);
}

/// vertex_arrays.*: an attribute pointer into a buffer that is then deleted
/// while bound; the attribute binding is 0 afterwards and the name, bound
/// again and filled, is not what the attribute points at.
#[test]
fn delete_with_an_attribute_attached_then_bind_the_name_again() {
    let _g = context();
    let b = gen_buffer();
    bind(GL_ARRAY_BUFFER, b);
    data(GL_ARRAY_BUFFER, 64, 0x33, GL_STATIC_DRAW);
    pointer(0);
    assert_eq!(attrib_buffer(0), b);

    delete(b);
    assert_eq!(geti(GL_ARRAY_BUFFER_BINDING), 0);
    assert_eq!(attrib_buffer(0), 0, "ES 2.0 2.9: the attribute's binding reverts to 0 too");
    assert!(!is_buffer(b));

    bind(GL_ARRAY_BUFFER, b);
    data(GL_ARRAY_BUFFER, 32, 0x44, GL_STATIC_DRAW);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 32);
    assert_eq!(attrib_buffer(0), 0, "a new buffer under the old name is not the attribute's");
    assert!(is_buffer(b));

    // Pointing the attribute at the new buffer works.
    pointer(0);
    assert_eq!(attrib_buffer(0), b);
    bind(GL_ARRAY_BUFFER, 0);
    delete(b);
    assert_eq!(attrib_buffer(0), 0);
}

/// Deleted while bound at ELEMENT_ARRAY: that binding reverts, the ARRAY
/// binding of another buffer is untouched, and the name rebinds.
#[test]
fn delete_while_bound_at_element_array_leaves_the_array_binding() {
    let _g = context();
    let (a, e) = (gen_buffer(), gen_buffer());
    bind(GL_ARRAY_BUFFER, a);
    data(GL_ARRAY_BUFFER, 16, 1, GL_STATIC_DRAW);
    bind(GL_ELEMENT_ARRAY_BUFFER, e);
    data(GL_ELEMENT_ARRAY_BUFFER, 12, 2, GL_STATIC_DRAW);
    delete(e);
    assert_eq!(geti(GL_ELEMENT_ARRAY_BUFFER_BINDING), 0);
    assert_eq!(geti(GL_ARRAY_BUFFER_BINDING), a);
    assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 16);
    bind(GL_ELEMENT_ARRAY_BUFFER, e);
    assert_eq!(buffer_param(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE), 0);
    data(GL_ELEMENT_ARRAY_BUFFER, 6, 3, GL_STREAM_DRAW);
    assert_eq!(buffer_param(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE), 6);
    bind(GL_ARRAY_BUFFER, 0);
    bind(GL_ELEMENT_ARRAY_BUFFER, 0);
    delete(a);
    delete(e);
}

/// A buffer bound at both targets is deleted: both bindings revert.
#[test]
fn delete_while_bound_at_both_targets_reverts_both() {
    let _g = context();
    let b = gen_buffer();
    bind(GL_ARRAY_BUFFER, b);
    data(GL_ARRAY_BUFFER, 8, 5, GL_STATIC_DRAW);
    bind(GL_ELEMENT_ARRAY_BUFFER, b);
    delete(b);
    assert_eq!((geti(GL_ARRAY_BUFFER_BINDING), geti(GL_ELEMENT_ARRAY_BUFFER_BINDING)), (0, 0));
    assert!(!is_buffer(b));
}

/// negative_api.buffer: with 0 bound, glBufferData / glBufferSubData /
/// glGetBufferParameteriv are GL_INVALID_OPERATION; after a delete-while-bound
/// the binding is 0 and the same calls give the same error, not a stale buffer.
#[test]
fn use_after_delete_while_bound_is_invalid_operation() {
    let _g = context();
    let b = gen_buffer();
    bind(GL_ARRAY_BUFFER, b);
    data(GL_ARRAY_BUFFER, 16, 7, GL_STATIC_DRAW);
    delete(b);
    let d = [0u8; 16];
    // SAFETY: `d` holds 16 bytes.
    unsafe { glBufferData(GL_ARRAY_BUFFER, 16, d.as_ptr().cast(), GL_STATIC_DRAW) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    // SAFETY: `d` holds 16 bytes.
    unsafe { glBufferSubData(GL_ARRAY_BUFFER, 0, 16, d.as_ptr().cast()) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    let mut v = 0;
    // SAFETY: room for one value.
    unsafe { glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut v) };
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/// Two deleted names rebound at the other target, and many cycles: every
/// round starts empty, nothing leaks into the next.
#[test]
fn rename_cycles_start_empty_each_time() {
    let _g = context();
    let (x, y) = (gen_buffer(), gen_buffer());
    for round in 0..40u32 {
        let n = 16 + (round as usize % 5) * 8;
        bind(GL_ARRAY_BUFFER, x);
        data(GL_ARRAY_BUFFER, n, round as u8, GL_STATIC_DRAW);
        bind(GL_ELEMENT_ARRAY_BUFFER, y);
        data(GL_ELEMENT_ARRAY_BUFFER, n + 4, round as u8, GL_STATIC_DRAW);
        pointer(1);
        delete(x);
        delete(y);
        assert_eq!(attrib_buffer(1), 0);
        bind(GL_ELEMENT_ARRAY_BUFFER, x);
        bind(GL_ARRAY_BUFFER, y);
        assert_eq!(buffer_param(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE), 0);
        assert_eq!(buffer_param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 0);
        bind(GL_ARRAY_BUFFER, 0);
        bind(GL_ELEMENT_ARRAY_BUFFER, 0);
        delete(x);
        delete(y);
        assert!(!is_buffer(x) && !is_buffer(y));
    }
}
