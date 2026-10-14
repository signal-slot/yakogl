// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, buffer part (item cts-tree-diff-readme; the audit is
//! docs/investigations/es2-error-rules.md, Table A "Buffers" rows 1-3).
//!
//! Only a `report-es2` build refuses; the ES 3.0 build keeps taking these
//! values (its own tests cover that). Run with
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_error_rules_buffer
//! ```

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es2-error-rules-buffer".as_ptr() as *const c_char,
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
    reset();
    g
}

/// Back to no buffer bound and an empty error queue.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        for t in [GL_ARRAY_BUFFER, GL_ELEMENT_ARRAY_BUFFER] {
            glBindBuffer(t, 0);
        }
    }
    drain();
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

const ES3_TARGETS: [GLenum; 6] = [
    GL_PIXEL_PACK_BUFFER,
    GL_PIXEL_UNPACK_BUFFER,
    GL_COPY_READ_BUFFER,
    GL_COPY_WRITE_BUFFER,
    GL_UNIFORM_BUFFER,
    GL_TRANSFORM_FEEDBACK_BUFFER,
];

/// A buffer of 16 bytes bound to GL_ARRAY_BUFFER.
fn array_buffer() -> GLuint {
    let mut b = 0;
    // SAFETY: plain GL calls; one name written.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 16, NULL, GL_STATIC_DRAW);
    }
    assert_eq!(err(), GL_NO_ERROR);
    b
}

const NULL: *const c_void = core::ptr::null();

/* ---- row 1: targets ------------------------------------------------------- */

#[test]
fn row1_es3_targets_are_invalid_enum_on_every_buffer_entry_point() {
    let _g = context();
    let _b = array_buffer();
    for t in ES3_TARGETS {
        // SAFETY: plain GL calls; the pointers are valid or null where the
        // call reads none.
        unsafe {
            glBindBuffer(t, 0);
            assert_eq!(err(), GL_INVALID_ENUM, "glBindBuffer 0x{t:x}");
            glBufferData(t, 16, NULL, GL_STATIC_DRAW);
            assert_eq!(err(), GL_INVALID_ENUM, "glBufferData 0x{t:x}");
            let d = [0u8; 4];
            glBufferSubData(t, 0, 4, d.as_ptr() as *const c_void);
            assert_eq!(err(), GL_INVALID_ENUM, "glBufferSubData 0x{t:x}");
            let mut v = -1;
            glGetBufferParameteriv(t, GL_BUFFER_SIZE, &mut v);
            assert_eq!(err(), GL_INVALID_ENUM, "glGetBufferParameteriv 0x{t:x}");
            assert_eq!(v, -1, "nothing written");
        }
    }
}

#[test]
fn row1_the_two_es2_targets_still_work() {
    let _g = context();
    let _b = array_buffer();
    let mut e = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenBuffers(1, &mut e);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, e);
        glBufferData(GL_ELEMENT_ARRAY_BUFFER, 8, NULL, GL_DYNAMIC_DRAW);
        let d = [1u8, 2, 3, 4];
        glBufferSubData(GL_ELEMENT_ARRAY_BUFFER, 0, 4, d.as_ptr() as *const c_void);
        let mut v = 0;
        glGetBufferParameteriv(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut v);
        assert_eq!(v, 8);
        glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut v);
        assert_eq!(v, 16);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/* ---- row 2: usage --------------------------------------------------------- */

#[test]
fn row2_usage_takes_only_the_three_draw_hints() {
    let _g = context();
    let _b = array_buffer();
    for u in [GL_STREAM_DRAW, GL_STATIC_DRAW, GL_DYNAMIC_DRAW] {
        // SAFETY: plain GL calls.
        unsafe {
            glBufferData(GL_ARRAY_BUFFER, 16, NULL, u);
            assert_eq!(err(), GL_NO_ERROR, "usage 0x{u:x}");
            let mut v = 0;
            glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_USAGE, &mut v);
            assert_eq!(v as GLenum, u);
        }
    }
    for u in
        [GL_STREAM_READ, GL_STREAM_COPY, GL_STATIC_READ, GL_STATIC_COPY, GL_DYNAMIC_READ, GL_DYNAMIC_COPY, 0, 0xFFFF]
    {
        // SAFETY: plain GL calls.
        unsafe {
            glBufferData(GL_ARRAY_BUFFER, 32, NULL, u);
            assert_eq!(err(), GL_INVALID_ENUM, "usage 0x{u:x}");
            let mut v = 0;
            glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut v);
            assert_eq!(v, 16, "the store is kept");
        }
    }
}

/* ---- row 3: parameters ---------------------------------------------------- */

#[test]
fn row3_es3_mapping_parameters_are_invalid_enum_the_oes_ones_stay() {
    let _g = context();
    let _b = array_buffer();
    for p in [GL_BUFFER_ACCESS_FLAGS, GL_BUFFER_MAP_OFFSET, GL_BUFFER_MAP_LENGTH] {
        let mut v = -1;
        // SAFETY: room for one value.
        unsafe { glGetBufferParameteriv(GL_ARRAY_BUFFER, p, &mut v) };
        assert_eq!(err(), GL_INVALID_ENUM, "pname 0x{p:x}");
        assert_eq!(v, -1);
    }
    for (p, want) in [
        (GL_BUFFER_MAPPED, 0),
        (GL_BUFFER_ACCESS_OES, GL_WRITE_ONLY_OES as GLint),
        (GL_BUFFER_USAGE, GL_STATIC_DRAW as GLint),
    ] {
        let mut v = -1;
        // SAFETY: room for one value.
        unsafe { glGetBufferParameteriv(GL_ARRAY_BUFFER, p, &mut v) };
        assert_eq!(err(), GL_NO_ERROR, "pname 0x{p:x}");
        assert_eq!(v, want, "pname 0x{p:x}");
    }
}
