// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, vertex part (item es2-error-rules; the audit is
//! tools/cts/log/investigations/es2-error-rules.md, Table A "Vertex arrays").
//!
//! A `report-es2` build refuses what ES 2.0 does not define; an ES 3.0 build
//! behaves as before. Decision per row:
//!
//! - Row 6, `glVertexAttribPointer` type: `report-es2` refuses GL_INT,
//!   GL_UNSIGNED_INT, GL_HALF_FLOAT (0x140B) and the two 2_10_10_10_REV types
//!   with GL_INVALID_ENUM (ES 2.0 2.8 lists BYTE, UNSIGNED_BYTE, SHORT,
//!   UNSIGNED_SHORT, FIXED, FLOAT). Code change: GL_INT and GL_UNSIGNED_INT
//!   were still in the unconditional arm (src/gl/vertex.rs); the other three
//!   were already refused. GL_HALF_FLOAT_OES (0x8D61) is refused too:
//!   GL_OES_vertex_half_float is not advertised and the draw path has no
//!   conversion for it, so accepting it would be wrong; if the extension is
//!   ever listed, the arm and the draw conversion go in together.
//! - Row 7, `glVertexAttribIPointer`: no change. An entry point, no enum
//!   rule; an ES 2.0 client never resolves it (audit "Open questions").
//!   Pinned below: still works in both builds.
//! - Row 8, `glVertexAttribDivisor` and the instanced draws: no change, as
//!   row 7. (The draw calls are in draw.rs, not in this item.)
//! - Row 9, `glGetVertexAttribfv/iv`: GL_VERTEX_ATTRIB_ARRAY_INTEGER and
//!   _DIVISOR give GL_INVALID_ENUM in `report-es2`. Already so at the base
//!   commit (`get_attrib_int` in vertex.rs); this file now pins it.
//! - Row 10, `glGetVertexAttribIiv/Iuiv`, `glVertexAttribI4*`: no change, as
//!   row 7.
//! - Row 13, `glGenVertexArrays` and the other unsuffixed VAO names: no
//!   change. GL_OES_vertex_array_object is advertised and the unsuffixed
//!   names are the same calls (vao.rs); they take no enum.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_error_rules_vertex
//! cargo test --no-default-features --features host-test,gpu-powervr --test es2_error_rules_vertex
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
    name: c"es2-error-rules-vertex".as_ptr() as *const c_char,
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

/// Back to a known vertex state after the previous test: the default vertex
/// array object, no array buffer, every attribute disabled with the default
/// layout, divisor 0 and current value (0, 0, 0, 1), the error queue empty.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindVertexArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        for i in 0..max_attribs() {
            glDisableVertexAttribArray(i);
            glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, NULL);
            glVertexAttribDivisor(i, 0);
            glVertexAttrib4f(i, 0.0, 0.0, 0.0, 1.0);
        }
    }
    drain();
}

const NULL: *const c_void = core::ptr::null();
const GL_HALF_FLOAT_OES: GLenum = 0x8D61;
const ES2: bool = cfg!(feature = "report-es2");

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
    let mut v = 0;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut v) };
    v as GLuint
}

/// The answer, or -99 if the query raised an error (left in the queue).
fn geti(index: GLuint, pname: GLenum) -> GLint {
    let mut v = -99;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribiv(index, pname, &mut v) };
    v
}

fn getf(index: GLuint, pname: GLenum) -> GLfloat {
    let mut v = -99.0;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribfv(index, pname, &mut v) };
    v
}

/// Row 6: the six ES 2.0 types are accepted and recorded.
#[test]
fn es2_array_types_are_accepted() {
    let _g = context();
    for t in [GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_FIXED, GL_FLOAT] {
        // SAFETY: plain GL call.
        unsafe { glVertexAttribPointer(1, 3, t, 1, 12, NULL) };
        assert_eq!(err(), GL_NO_ERROR, "type 0x{t:x}");
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_TYPE), t as GLint);
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_SIZE), 3);
    }
}

/// Row 6: the ES 3.0-only types by build, and the OES half float neither
/// build advertises.
#[test]
fn es3_array_types_by_build() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttribPointer(1, 2, GL_SHORT, 0, 0, NULL);
        for t in [GL_INT, GL_UNSIGNED_INT, GL_HALF_FLOAT, GL_INT_2_10_10_10_REV, GL_UNSIGNED_INT_2_10_10_10_REV] {
            glVertexAttribPointer(1, 4, t, 0, 0, NULL);
            if ES2 {
                assert_eq!(err(), GL_INVALID_ENUM, "type 0x{t:x}");
                assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_SHORT as GLint, "not recorded");
                assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_SIZE), 2);
            } else {
                assert_eq!(err(), GL_NO_ERROR, "type 0x{t:x}");
                assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_TYPE), t as GLint);
            }
        }
        glVertexAttribPointer(1, 4, GL_HALF_FLOAT_OES, 0, 0, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
        /* a desktop type: in neither version */
        glVertexAttribPointer(1, 4, 0x140a, 0, 0, NULL);
        assert_eq!(err(), GL_INVALID_ENUM);
    }
}

/// Row 6: the value checks that come before the type check are unchanged.
#[test]
fn value_errors_are_unchanged() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttribPointer(1, 5, GL_FLOAT, 0, 0, NULL);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, -1, NULL);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribPointer(max_attribs(), 4, GL_FLOAT, 0, 0, NULL);
        assert_eq!(err(), GL_INVALID_VALUE);
    }
}

/// Row 9: the ES 2.0 pnames answer; the two ES 3.0 ones by build.
#[test]
fn query_pnames_by_build() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttribPointer(2, 3, GL_UNSIGNED_SHORT, 1, 6, NULL);
        glEnableVertexAttribArray(2);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 1);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_SIZE), 3);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 6);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_UNSIGNED_SHORT as GLint);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 1);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), 0);
        assert_eq!(getf(2, GL_VERTEX_ATTRIB_ARRAY_SIZE), 3.0);
        glVertexAttrib4f(2, 1.0, 2.0, 3.0, 4.0);
        let mut cur = [0.0f32; 4];
        glGetVertexAttribfv(2, GL_CURRENT_VERTEX_ATTRIB, cur.as_mut_ptr());
        assert_eq!(cur, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(err(), GL_NO_ERROR);

        for pname in [GL_VERTEX_ATTRIB_ARRAY_INTEGER, GL_VERTEX_ATTRIB_ARRAY_DIVISOR] {
            if ES2 {
                assert_eq!(geti(2, pname), -99, "pname 0x{pname:x}");
                assert_eq!(err(), GL_INVALID_ENUM);
                assert_eq!(getf(2, pname), -99.0);
                assert_eq!(err(), GL_INVALID_ENUM);
            } else {
                assert_eq!(geti(2, pname), 0, "pname 0x{pname:x}");
                assert_eq!(getf(2, pname), 0.0);
                assert_eq!(err(), GL_NO_ERROR);
            }
        }
        /* an unknown pname is an error in both */
        assert_eq!(geti(2, 0), -99);
        assert_eq!(err(), GL_INVALID_ENUM);
    }
}

/// Rows 7, 8, 10: the ES 3.0 entry points are left alone in both builds.
#[test]
fn es3_entry_points_are_unchanged() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttribIPointer(3, 2, GL_INT, 0, NULL);
        assert_eq!(err(), GL_NO_ERROR);
        glVertexAttribDivisor(3, 1);
        assert_eq!(err(), GL_NO_ERROR);
        glVertexAttribI4i(3, 1, -2, 3, -4);
        assert_eq!(err(), GL_NO_ERROR);
        let mut v = [0i32; 4];
        glGetVertexAttribIiv(3, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr());
        assert_eq!(v, [1, -2, 3, -4]);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/// Row 13: the unsuffixed names are the OES calls in both builds.
#[test]
fn unsuffixed_vao_names_work() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        let mut v = 0;
        glGenVertexArrays(1, &mut v);
        assert_ne!(v, 0);
        glBindVertexArray(v);
        assert_eq!(glIsVertexArray(v), GL_TRUE_B);
        assert_eq!(glIsVertexArrayOES(v), GL_TRUE_B);
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &v);
        assert_eq!(glIsVertexArray(v), GL_FALSE_B);
        assert_eq!(err(), GL_NO_ERROR);
    }
}
