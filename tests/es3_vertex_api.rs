// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 vertex attribute state through the public GL API on the host (item
//! vertex-int-attribs; docs/es3-vertex-gap.md names the dEQP buckets): the
//! argument checks of glVertexAttribPointer / glVertexAttribIPointer /
//! glVertexAttribDivisor, the ES 3.0 array types (half float, the packed
//! 2_10_10_10 types with size 4 only), the pure-integer current values of
//! glVertexAttribI4{i,ui}{,v} read back exactly by glGetVertexAttribI{i,ui}v,
//! the rounding of glGetVertexAttribiv, the GL_VERTEX_ATTRIB_ARRAY_INTEGER /
//! _DIVISOR queries, and (with the in-process compiler) the dwords an ivec4 /
//! uvec4 input is handed for a disabled array: the integers themselves.
//!
//! An ES 2.0-reporting build (`report-es2`) must answer GL_INVALID_ENUM for
//! the ES 3.0 types and pnames: `es2_build_refuses_the_es3_types_and_pnames`
//! runs only there. The entry points themselves exist in both builds, so the
//! argument and current-value tests run in both. The back end is a stand-in
//! that records the per-draw constants it is handed.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_vertex_api
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es3_vertex_api
//! ```

#![cfg(feature = "host-test")]
// the draw test needs the in-process compiler and the ES 3.0 report; its helpers are idle elsewhere
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
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

/// The per-draw constants the stand-in back end was handed: a disabled
/// array's current value reaches the back end as a zero-stride fetch of 16
/// bytes staged by the GL core (`GlesAttrib::cpu`, `cpu_bytes` 16). Each is
/// kept as its four dwords.
static CONSTANTS: Mutex<Vec<[u32; 4]>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for a in &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())] {
        if a.stride == 0 && a.cpu_bytes == 16 && !a.cpu.is_null() {
            let mut w = [0u32; 4];
            // SAFETY: the contract says `cpu` holds `cpu_bytes` bytes for the
            // duration of the call.
            core::ptr::copy_nonoverlapping(a.cpu as *const u8, w.as_mut_ptr() as *mut u8, 16);
            CONSTANTS.lock().unwrap().push(w);
        }
    }
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
    name: c"es3-vertex-api".as_ptr() as *const c_char,
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
    reset();
    g
}

/// Back to a known vertex state after the previous test: the default vertex
/// array object, no array buffer, every attribute disabled with the default
/// layout, divisor 0 and current value (0, 0, 0, 1), no program, the error
/// queue empty.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindVertexArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glUseProgram(0);
        for i in 0..max_attribs() {
            glDisableVertexAttribArray(i);
            glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, NULL);
            glVertexAttribDivisor(i, 0);
            glVertexAttrib4f(i, 0.0, 0.0, 0.0, 1.0);
        }
    }
    drain();
    CONSTANTS.lock().unwrap().clear();
}

const NULL: *const c_void = core::ptr::null();
/// GL_DOUBLE: a desktop type no ES accepts.
const GL_DOUBLE_ENUM: GLenum = 0x140a;

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

fn geti(index: GLuint, pname: GLenum) -> GLint {
    let mut v = [-99; 4];
    // SAFETY: room for the four values CURRENT_VERTEX_ATTRIB returns.
    unsafe { glGetVertexAttribiv(index, pname, v.as_mut_ptr()) };
    v[0]
}

fn getf(index: GLuint, pname: GLenum) -> GLfloat {
    let mut v = [-99.0; 4];
    // SAFETY: room for the four values CURRENT_VERTEX_ATTRIB returns.
    unsafe { glGetVertexAttribfv(index, pname, v.as_mut_ptr()) };
    v[0]
}

fn current_f(index: GLuint) -> [GLfloat; 4] {
    let mut v = [-99.0; 4];
    // SAFETY: room for four values.
    unsafe { glGetVertexAttribfv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}

fn current_iv(index: GLuint) -> [GLint; 4] {
    let mut v = [-99; 4];
    // SAFETY: room for four values.
    unsafe { glGetVertexAttribiv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}

fn current_i(index: GLuint) -> [GLint; 4] {
    let mut v = [-99; 4];
    // SAFETY: room for four values.
    unsafe { glGetVertexAttribIiv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}

fn current_ui(index: GLuint) -> [GLuint; 4] {
    let mut v = [0xdead_beef; 4];
    // SAFETY: room for four values.
    unsafe { glGetVertexAttribIuiv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
    v
}

fn pointer(index: GLuint) -> usize {
    let mut p = 0xDEAD as *mut c_void;
    // SAFETY: room for one pointer.
    unsafe { glGetVertexAttribPointerv(index, GL_VERTEX_ATTRIB_ARRAY_POINTER, &mut p) };
    p as usize
}

/// A new buffer, bound as the ARRAY_BUFFER (no storage).
fn buffer() -> GLuint {
    let mut b = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
    }
    b
}

/// A new vertex array object, bound.
fn vao() -> GLuint {
    let mut a = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenVertexArrays(1, &mut a);
        glBindVertexArray(a);
    }
    a
}

/* ---- glVertexAttribPointer / glVertexAttribIPointer arguments ------------- */

#[test]
fn pointer_bad_index_size_and_stride_are_invalid_value() {
    let _g = context();
    let max = max_attribs();
    // SAFETY: plain GL calls with null offsets.
    unsafe {
        glVertexAttribPointer(0, 2, GL_SHORT, 0, 0, NULL);
        glVertexAttribPointer(max, 4, GL_FLOAT, 0, 0, NULL);
        assert_eq!(err(), GL_INVALID_VALUE, "index = MAX_VERTEX_ATTRIBS");
        glVertexAttribIPointer(max, 4, GL_INT, 0, NULL);
        assert_eq!(err(), GL_INVALID_VALUE, "index (I)");
        for size in [0, 5, -1] {
            glVertexAttribPointer(0, size, GL_FLOAT, 0, 0, NULL);
            assert_eq!(err(), GL_INVALID_VALUE, "size {size}");
            glVertexAttribIPointer(0, size, GL_INT, 0, NULL);
            assert_eq!(err(), GL_INVALID_VALUE, "size {size} (I)");
        }
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, -1, NULL);
        assert_eq!(err(), GL_INVALID_VALUE, "negative stride");
        glVertexAttribIPointer(0, 4, GL_INT, -1, NULL);
        assert_eq!(err(), GL_INVALID_VALUE, "negative stride (I)");
    }
    assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_SHORT as GLint, "a refused call leaves the array alone");
    assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_SIZE), 2);
}

#[test]
fn pointer_bad_type_is_invalid_enum() {
    let _g = context();
    /* GL_HALF_FLOAT_OES: GL_OES_vertex_half_float is not claimed */
    let bad = [GL_NONE, GL_BOOL, GL_DOUBLE_ENUM, GL_FLOAT_VEC4, GL_UNSIGNED_SHORT_5_6_5, GL_RGBA, GL_HALF_FLOAT_OES];
    // SAFETY: plain GL calls.
    unsafe {
        for t in bad {
            glVertexAttribPointer(0, 4, t, 0, 0, NULL);
            assert_eq!(err(), GL_INVALID_ENUM, "type 0x{t:x}");
        }
        /* the integer entry point takes no float, fixed or half float, and no packed type */
        for t in [GL_FLOAT, GL_FIXED, GL_HALF_FLOAT, GL_INT_2_10_10_10_REV, GL_UNSIGNED_INT_2_10_10_10_REV, GL_NONE] {
            glVertexAttribIPointer(0, 4, t, 0, NULL);
            assert_eq!(err(), GL_INVALID_ENUM, "I type 0x{t:x}");
        }
    }
    assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_FLOAT as GLint);
}

#[test]
fn ipointer_records_an_integer_array_of_the_six_integer_types() {
    let _g = context();
    let b = buffer();
    // SAFETY: plain GL calls; the offset is a byte offset into the bound buffer.
    unsafe {
        for t in [GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_INT, GL_UNSIGNED_INT] {
            for size in 1..=4 {
                glVertexAttribIPointer(2, size, t, 12, 8usize as *const c_void);
                assert_eq!(err(), GL_NO_ERROR, "I type 0x{t:x} size {size}");
                assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_TYPE), t as GLint);
                assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_SIZE), size);
                assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 12);
                assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 0, "an integer array is never normalized");
                assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), b as GLint);
                assert_eq!(pointer(2), 8);
            }
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &b);
    }
}

#[test]
fn client_array_with_a_vao_bound_is_invalid_operation() {
    let _g = context();
    let data = [0u8; 16];
    // SAFETY: plain GL calls; the pointers are only recorded.
    unsafe {
        /* the default vertex array object takes client arrays */
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, data.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(pointer(0), data.as_ptr() as usize);
        let v = vao();
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, data.as_ptr() as *const c_void);
        assert_eq!(err(), GL_INVALID_OPERATION, "client pointer, VAO bound, no buffer");
        glVertexAttribIPointer(0, 4, GL_INT, 0, data.as_ptr() as *const c_void);
        assert_eq!(err(), GL_INVALID_OPERATION, "the same for the integer entry point");
        /* a null offset with no buffer is allowed, and so is any offset into a buffer */
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, NULL);
        assert_eq!(err(), GL_NO_ERROR);
        let b = buffer();
        glVertexAttribIPointer(0, 4, GL_INT, 0, 12usize as *const c_void);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(pointer(0), 12);
        assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), b as GLint);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &v);
        glDeleteBuffers(1, &b);
    }
}

/* ---- enable / disable / divisor ------------------------------------------ */

#[test]
fn enable_disable_and_divisor_take_only_a_valid_index() {
    let _g = context();
    let max = max_attribs();
    // SAFETY: plain GL calls.
    unsafe {
        glEnableVertexAttribArray(max);
        assert_eq!(err(), GL_INVALID_VALUE);
        glDisableVertexAttribArray(max);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribDivisor(max, 1);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribDivisor(GLuint::MAX, 0);
        assert_eq!(err(), GL_INVALID_VALUE);
        glEnableVertexAttribArray(max - 1);
        glVertexAttribDivisor(max - 1, 3);
        glVertexAttribDivisor(0, GLuint::MAX);
        assert_eq!(err(), GL_NO_ERROR, "any divisor value is legal");
        assert_eq!(geti(max - 1, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 1);
        glDisableVertexAttribArray(max - 1);
        assert_eq!(geti(max - 1, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 0);
    }
}

/* ---- current values ------------------------------------------------------- */

#[test]
fn float_current_values_default_and_fill_the_missing_components() {
    let _g = context();
    let max = max_attribs();
    for i in [0, max - 1] {
        assert_eq!(current_f(i), [0.0, 0.0, 0.0, 1.0]);
    }
    // SAFETY: plain GL calls; the arrays hold what each setter reads.
    unsafe {
        glVertexAttrib1f(3, 5.0);
        assert_eq!(current_f(3), [5.0, 0.0, 0.0, 1.0]);
        glVertexAttrib2f(3, 5.0, 6.0);
        assert_eq!(current_f(3), [5.0, 6.0, 0.0, 1.0]);
        glVertexAttrib3f(3, 5.0, 6.0, 7.0);
        assert_eq!(current_f(3), [5.0, 6.0, 7.0, 1.0]);
        glVertexAttrib4fv(3, [1.0, 2.0, 3.0, 4.0].as_ptr());
        assert_eq!(current_f(3), [1.0, 2.0, 3.0, 4.0]);
        glVertexAttrib2fv(3, [9.0, 8.0].as_ptr());
        assert_eq!(current_f(3), [9.0, 8.0, 0.0, 1.0]);
        glVertexAttrib4f(max, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttrib1fv(max, [0.0].as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        /* array state leaves the current value alone */
        glVertexAttribPointer(3, 2, GL_FLOAT, 0, 0, NULL);
        glEnableVertexAttribArray(3);
        glDisableVertexAttribArray(3);
    }
    assert_eq!(current_f(3), [9.0, 8.0, 0.0, 1.0]);
    assert_eq!(current_f(4), [0.0, 0.0, 0.0, 1.0], "other indices are untouched");
}

/// ES 3.0 6.1.2: a float state value read by an integer query is rounded to
/// the nearest integer (dEQP state_query.shader.current_vertex_attrib_float_to_int
/// accepts floor to ceil; nearest is inside that).
#[test]
fn get_vertex_attribiv_rounds_the_float_value_to_nearest() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttrib4f(0, 1.0, -2.0, 300.0, 0.0);
        assert_eq!(current_iv(0), [1, -2, 300, 0]);
        glVertexAttrib4f(0, 1.4, 1.6, -1.4, -1.6);
        assert_eq!(current_iv(0), [1, 2, -1, -2]);
        glVertexAttrib4f(0, 2.5, -2.5, 1.0e10, -1.0e10);
        assert_eq!(current_iv(0), [3, -3, GLint::MAX, GLint::MIN], "halves away from zero; saturating");
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// ES 3.0 2.7: a pure-integer current value is kept exactly, up to the full
/// 32-bit range, and comes back from the I-getters as it went in.
#[test]
fn integer_current_values_round_trip_exactly() {
    let _g = context();
    // SAFETY: plain GL calls; the vectors hold four values.
    unsafe {
        glVertexAttribI4i(6, -1, 0, 7, 1 << 20);
        assert_eq!(current_i(6), [-1, 0, 7, 1 << 20]);
        glVertexAttribI4i(6, (1 << 24) + 1, GLint::MIN, GLint::MAX, -(1 << 24) - 1);
        assert_eq!(current_i(6), [(1 << 24) + 1, GLint::MIN, GLint::MAX, -(1 << 24) - 1], "beyond a float's 24 bits");
        glVertexAttribI4ui(6, (1 << 24) + 1, u32::MAX, 0x8000_0001, 0xffff_fffe);
        assert_eq!(current_ui(6), [(1 << 24) + 1, u32::MAX, 0x8000_0001, 0xffff_fffe], "above i32::MAX");
        glVertexAttribI4iv(6, [-5, -6, -7, -8].as_ptr());
        assert_eq!(current_i(6), [-5, -6, -7, -8]);
        glVertexAttribI4uiv(6, [9, 10, 11, 12].as_ptr());
        assert_eq!(current_ui(6), [9, 10, 11, 12]);
        glVertexAttribI4iv(6, core::ptr::null());
        glVertexAttribI4uiv(6, core::ptr::null());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(current_ui(6), [9, 10, 11, 12], "a null vector is ignored");
        /* a float set afterwards is a float value again */
        glVertexAttrib4f(6, 1.0, 2.0, 3.0, 4.0);
        assert_eq!(current_f(6), [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(current_iv(6), [1, 2, 3, 4]);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn integer_current_value_setters_check_the_index() {
    let _g = context();
    let max = max_attribs();
    // SAFETY: plain GL calls; the vectors hold four values.
    unsafe {
        glVertexAttribI4i(max, 1, 1, 1, 1);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribI4ui(max, 1, 1, 1, 1);
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribI4iv(max, [1; 4].as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glVertexAttribI4uiv(max, [1u32; 4].as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
    }
    assert_eq!(current_f(max - 1), [0.0, 0.0, 0.0, 1.0], "nothing was stored anywhere");
}

/* ---- getters -------------------------------------------------------------- */

#[test]
fn getter_errors_write_nothing() {
    let _g = context();
    let max = max_attribs();
    let mut i = [-99; 4];
    let mut u = [77u32; 4];
    let mut f = [-99.0f32; 4];
    let mut p = 0xDEAD as *mut c_void;
    // SAFETY: room for four values each, one pointer.
    unsafe {
        glGetVertexAttribiv(max, GL_VERTEX_ATTRIB_ARRAY_SIZE, i.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glGetVertexAttribfv(max, GL_CURRENT_VERTEX_ATTRIB, f.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glGetVertexAttribIiv(max, GL_CURRENT_VERTEX_ATTRIB, i.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glGetVertexAttribIuiv(max, GL_CURRENT_VERTEX_ATTRIB, u.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glGetVertexAttribPointerv(max, GL_VERTEX_ATTRIB_ARRAY_POINTER, &mut p);
        assert_eq!(err(), GL_INVALID_VALUE);
        /* an unknown pname, and the pointer's own pname, are GL_INVALID_ENUM */
        for pname in [GL_VIEWPORT, GL_VERTEX_ATTRIB_ARRAY_POINTER, GL_NONE] {
            glGetVertexAttribiv(0, pname, i.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_ENUM, "iv 0x{pname:x}");
            glGetVertexAttribfv(0, pname, f.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_ENUM, "fv 0x{pname:x}");
            glGetVertexAttribIiv(0, pname, i.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_ENUM, "Iiv 0x{pname:x}");
            glGetVertexAttribIuiv(0, pname, u.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_ENUM, "Iuiv 0x{pname:x}");
        }
        glGetVertexAttribPointerv(0, GL_VERTEX_ATTRIB_ARRAY_SIZE, &mut p);
        assert_eq!(err(), GL_INVALID_ENUM);
        /* null destinations are tolerated */
        glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_SIZE, core::ptr::null_mut());
        glGetVertexAttribIiv(0, GL_CURRENT_VERTEX_ATTRIB, core::ptr::null_mut());
        glGetVertexAttribIuiv(0, GL_CURRENT_VERTEX_ATTRIB, core::ptr::null_mut());
        glGetVertexAttribPointerv(0, GL_VERTEX_ATTRIB_ARRAY_POINTER, core::ptr::null_mut());
        assert_eq!(err(), GL_NO_ERROR);
    }
    assert_eq!((i, u, f, p as usize), ([-99; 4], [77; 4], [-99.0; 4], 0xDEAD));
}

/* ------------------------------------------------------------- ES 2.0 -- */

/// The ES 2.0 report knows neither the ES 3.0 array types nor the ES 3.0
/// query pnames: GL_INVALID_ENUM, and the array is left as it was.
#[cfg(feature = "report-es2")]
#[test]
fn es2_build_refuses_the_es3_types_and_pnames() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttribPointer(1, 2, GL_SHORT, 0, 0, NULL);
        for t in [GL_HALF_FLOAT, GL_INT_2_10_10_10_REV, GL_UNSIGNED_INT_2_10_10_10_REV] {
            glVertexAttribPointer(1, 4, t, 0, 0, NULL);
            assert_eq!(err(), GL_INVALID_ENUM, "type 0x{t:x}");
            assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_SHORT as GLint);
            assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_SIZE), 2);
        }
        for pname in [GL_VERTEX_ATTRIB_ARRAY_INTEGER, GL_VERTEX_ATTRIB_ARRAY_DIVISOR] {
            assert_eq!(geti(1, pname), -99, "pname 0x{pname:x}");
            assert_eq!(err(), GL_INVALID_ENUM);
            assert_eq!(getf(1, pname), -99.0);
            assert_eq!(err(), GL_INVALID_ENUM);
        }
        /* the ES 2.0 pnames answer as before */
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 0);
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 0);
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), 0);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    #[test]
    fn packed_types_take_size_4_only() {
        let _g = context();
        // SAFETY: plain GL calls.
        unsafe {
            for t in [GL_INT_2_10_10_10_REV, GL_UNSIGNED_INT_2_10_10_10_REV] {
                glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, NULL);
                for size in 1..=3 {
                    glVertexAttribPointer(1, size, t, 0, 0, NULL);
                    assert_eq!(err(), GL_INVALID_OPERATION, "type 0x{t:x} size {size}");
                    assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_FLOAT as GLint, "not recorded");
                }
                glVertexAttribPointer(1, 4, t, 1, 0, NULL);
                assert_eq!(err(), GL_NO_ERROR);
                assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_TYPE), t as GLint);
                assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_SIZE), 4);
                assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 1);
            }
            /* a size outside 1..4 is GL_INVALID_VALUE before the packed rule */
            glVertexAttribPointer(1, 5, GL_INT_2_10_10_10_REV, 0, 0, NULL);
            assert_eq!(err(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn every_float_array_type_takes_sizes_1_to_4() {
        let _g = context();
        let types = [
            GL_BYTE,
            GL_UNSIGNED_BYTE,
            GL_SHORT,
            GL_UNSIGNED_SHORT,
            GL_INT,
            GL_UNSIGNED_INT,
            GL_FLOAT,
            GL_FIXED,
            GL_HALF_FLOAT,
        ];
        // SAFETY: plain GL calls.
        unsafe {
            for t in types {
                for size in 1..=4 {
                    glVertexAttribPointer(2, size, t, 1, 7, NULL);
                    assert_eq!(err(), GL_NO_ERROR, "type 0x{t:x} size {size}");
                    assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_TYPE), t as GLint);
                    assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_SIZE), size);
                    assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 7);
                    assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 1);
                    assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 0);
                }
            }
        }
    }

    #[test]
    fn the_integer_flag_follows_the_last_pointer_call() {
        let _g = context();
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribIPointer(2, 4, GL_INT, 0, NULL);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1);
            assert_eq!(getf(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1.0);
            glVertexAttribPointer(2, 4, GL_INT, 1, 0, NULL);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 0);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 1);
            glVertexAttribIPointer(2, 4, GL_UNSIGNED_BYTE, 0, NULL);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 0);
            /* a refused call changes nothing */
            glVertexAttribPointer(2, 3, GL_INT_2_10_10_10_REV, 0, 0, NULL);
            assert_eq!(err(), GL_INVALID_OPERATION);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1);
            glEnableVertexAttribArray(2);
            glVertexAttribDivisor(2, 2);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1, "enable and divisor are separate state");
        }
        assert_eq!(err(), GL_NO_ERROR);
    }

    #[test]
    fn the_divisor_is_queried_and_survives_a_new_pointer() {
        let _g = context();
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribDivisor(4, 2);
            assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 2);
            assert_eq!(getf(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 2.0);
            glVertexAttribPointer(4, 2, GL_SHORT, 0, 0, NULL);
            glVertexAttribIPointer(4, 2, GL_SHORT, 0, NULL);
            assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 2);
            glVertexAttribDivisor(4, 0);
            assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 0);
            assert_eq!(geti(5, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 0, "the neighbour is untouched");
        }
        assert_eq!(err(), GL_NO_ERROR);
    }

    #[test]
    fn initial_array_state() {
        let _g = context();
        for pname in [
            GL_VERTEX_ATTRIB_ARRAY_ENABLED,
            GL_VERTEX_ATTRIB_ARRAY_STRIDE,
            GL_VERTEX_ATTRIB_ARRAY_NORMALIZED,
            GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING,
            GL_VERTEX_ATTRIB_ARRAY_INTEGER,
            GL_VERTEX_ATTRIB_ARRAY_DIVISOR,
        ] {
            assert_eq!(geti(9, pname), 0, "pname 0x{pname:x}");
        }
        assert_eq!(geti(9, GL_VERTEX_ATTRIB_ARRAY_SIZE), 4);
        assert_eq!(geti(9, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_FLOAT as GLint);
        assert_eq!(pointer(9), 0);
        assert_eq!(err(), GL_NO_ERROR);
    }

    #[test]
    fn every_array_pname_agrees_across_the_four_getters() {
        let _g = context();
        let b = buffer();
        // SAFETY: plain GL calls; room for one value each.
        unsafe {
            glVertexAttribIPointer(10, 3, GL_UNSIGNED_SHORT, 12, 4usize as *const c_void);
            glEnableVertexAttribArray(10);
            glVertexAttribDivisor(10, 5);
            for pname in [
                GL_VERTEX_ATTRIB_ARRAY_ENABLED,
                GL_VERTEX_ATTRIB_ARRAY_SIZE,
                GL_VERTEX_ATTRIB_ARRAY_STRIDE,
                GL_VERTEX_ATTRIB_ARRAY_TYPE,
                GL_VERTEX_ATTRIB_ARRAY_NORMALIZED,
                GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING,
                GL_VERTEX_ATTRIB_ARRAY_INTEGER,
                GL_VERTEX_ATTRIB_ARRAY_DIVISOR,
            ] {
                let (mut i, mut ii, mut u, mut f) = (0, 0, 0, 0.0);
                glGetVertexAttribiv(10, pname, &mut i);
                glGetVertexAttribIiv(10, pname, &mut ii);
                glGetVertexAttribIuiv(10, pname, &mut u);
                glGetVertexAttribfv(10, pname, &mut f);
                assert_eq!(i, ii, "pname 0x{pname:x}");
                assert_eq!(i as u32, u, "pname 0x{pname:x}");
                assert_eq!(i as f32, f, "pname 0x{pname:x}");
            }
            assert_eq!(geti(10, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), b as GLint, "the buffer's name");
            assert_eq!(geti(10, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1);
            assert_eq!(geti(10, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 5);
            assert_eq!(pointer(10), 4);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDeleteBuffers(1, &b);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }

    /// Integer inputs read a disabled array's current value as the integer
    /// bits (ES 3.0 2.7), not as a float: the GL core stages the current
    /// value's dwords as they are, so what glVertexAttribI4i / I4ui stored
    /// must already be the integers. The stand-in back end records the
    /// constants it is handed.
    #[cfg(feature = "shaderc-online")]
    #[test]
    fn integer_current_values_reach_the_back_end_as_integers() {
        let _g = context();
        const VS: &str = "#version 300 es\nin vec4 a_pos;\nin ivec4 a_int;\nin uvec4 a_uint;\n\
            void main() { gl_Position = a_pos + vec4(a_int) + vec4(a_uint); }\n";
        const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
        let shader = |ty: GLenum, text: &str| -> GLuint {
            let text = CString::new(text).unwrap();
            // SAFETY: plain GL calls; one NUL-terminated source.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "{text:?} did not compile");
                s
            }
        };
        let (a_int, a_uint, a_pos) = (c"a_int", c"a_uint", c"a_pos");
        let ints: [GLint; 4] = [(1 << 24) + 1, GLint::MIN, -1, 7];
        let uints: [GLuint; 4] = [u32::MAX, 0x8000_0001, (1 << 24) + 1, 0];
        // SAFETY: plain GL calls; every pointer outlives its call.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "the program did not link");
            let (li, lu, lp) = (
                glGetAttribLocation(p, a_int.as_ptr()),
                glGetAttribLocation(p, a_uint.as_ptr()),
                glGetAttribLocation(p, a_pos.as_ptr()),
            );
            assert!(li >= 0 && lu >= 0 && lp >= 0, "every input is active: {li} {lu} {lp}");
            /* the position: a float32 array in a buffer, fetched in place */
            let quad = [0.0f32; 12];
            let mut b = 0;
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_ARRAY_BUFFER, b);
            glBufferData(GL_ARRAY_BUFFER, 48, quad.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glVertexAttribPointer(lp as GLuint, 4, GL_FLOAT, 0, 0, NULL);
            glEnableVertexAttribArray(lp as GLuint);
            glVertexAttribI4i(li as GLuint, ints[0], ints[1], ints[2], ints[3]);
            glVertexAttribI4uiv(lu as GLuint, uints.as_ptr());
            glUseProgram(p);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
            assert_eq!(err(), GL_NO_ERROR);
            glDisableVertexAttribArray(lp as GLuint);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDeleteBuffers(1, &b);
            glUseProgram(0);
            glDeleteProgram(p);
        }
        let got = CONSTANTS.lock().unwrap().clone();
        assert_eq!(got.len(), 2, "two disabled arrays were staged as constants: {got:x?}");
        assert!(got.contains(&ints.map(|v| v as u32)), "the ivec4 input's dwords: {got:x?}");
        assert!(got.contains(&uints), "the uvec4 input's dwords: {got:x?}");
    }
}
