// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The query forms of the ES 3.0 implementation limits: for every limit
//! (spec tables 6.29-6.33) `glGetFloatv` and `glGetInteger64v` answer what
//! `glGetIntegerv` answers, with the same value count and no error, and
//! `glGetBooleanv` is that value's non-zero test. These are the
//! `*_getfloat` / `*_getinteger64` / `*_getboolean` forms of dEQP-GLES3
//! `state_query.integers` / `.integers64`; the values themselves (including
//! those below the ES 3.0 minimum) are not asserted here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_query_forms
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,no-shader-table --test es3_query_forms
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,no-shader-table --test es3_query_forms
//! ```

#![cfg(all(
    feature = "host-test",
    any(feature = "gpu-v3d", feature = "gpu-powervr", feature = "gpu-vivante"),
    not(feature = "report-es2")
))]

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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
    name: c"es3-query-forms".as_ptr() as *const c_char,
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
    flush: None,
    finish: None,
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

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

fn error_clear() {
    // SAFETY: plain GL call.
    unsafe { while glGetError() != GL_NO_ERROR {} }
}

const LIMITS: &[(&str, GLenum)] = &[
    ("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE),
    ("MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE),
    ("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE),
    ("MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE),
    ("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS),
    ("MAX_TEXTURE_LOD_BIAS", GL_MAX_TEXTURE_LOD_BIAS),
    ("MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX),
    ("MAX_ELEMENTS_VERTICES", GL_MAX_ELEMENTS_VERTICES),
    ("MAX_ELEMENTS_INDICES", GL_MAX_ELEMENTS_INDICES),
    ("MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS),
    ("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS),
    ("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS),
    ("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS),
    ("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS),
    ("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS),
    ("MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS),
    ("MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS),
    ("MAX_VERTEX_UNIFORM_BLOCKS", GL_MAX_VERTEX_UNIFORM_BLOCKS),
    ("MAX_FRAGMENT_UNIFORM_BLOCKS", GL_MAX_FRAGMENT_UNIFORM_BLOCKS),
    ("MAX_COMBINED_UNIFORM_BLOCKS", GL_MAX_COMBINED_UNIFORM_BLOCKS),
    ("MAX_UNIFORM_BUFFER_BINDINGS", GL_MAX_UNIFORM_BUFFER_BINDINGS),
    ("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE),
    ("UNIFORM_BUFFER_OFFSET_ALIGNMENT", GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT),
    ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS),
    ("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS),
    ("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS),
    ("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS),
    ("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS),
    ("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS),
    ("MAX_PROGRAM_TEXEL_OFFSET", GL_MAX_PROGRAM_TEXEL_OFFSET),
    ("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS),
    ("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS),
    ("MAX_SAMPLES", GL_MAX_SAMPLES),
    ("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS),
    ("MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS),
    ("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS),
    ("MAX_SERVER_WAIT_TIMEOUT", GL_MAX_SERVER_WAIT_TIMEOUT),
    ("NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS),
    ("NUM_PROGRAM_BINARY_FORMATS", GL_NUM_PROGRAM_BINARY_FORMATS),
    ("NUM_EXTENSIONS", GL_NUM_EXTENSIONS),
    ("SUBPIXEL_BITS", GL_SUBPIXEL_BITS),
    ("DEPTH_BITS", GL_DEPTH_BITS),
    ("STENCIL_BITS", GL_STENCIL_BITS),
    ("MAX_VIEWPORT_DIMS", GL_MAX_VIEWPORT_DIMS),
    ("ALIASED_LINE_WIDTH_RANGE", GL_ALIASED_LINE_WIDTH_RANGE),
    ("ALIASED_POINT_SIZE_RANGE", GL_ALIASED_POINT_SIZE_RANGE),
];

/// Values a pname wrote, per query form (a sentinel marks the unwritten tail).
struct Forms {
    int: Vec<i32>,
    float: Vec<f32>,
    int64: Vec<i64>,
    boolean: Vec<u8>,
}

fn query(name: &str, pname: GLenum) -> Forms {
    let mut iv = [i32::MIN; 8];
    let mut fv = [f32::NAN; 8];
    let mut lv = [i64::MIN; 8];
    let mut bv = [0xEEu8; 8];
    // SAFETY: room for eight values, more than any pname here writes.
    unsafe {
        error_clear();
        glGetIntegerv(pname, iv.as_mut_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glGetIntegerv error");
        glGetFloatv(pname, fv.as_mut_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glGetFloatv error");
        glGetInteger64v(pname, lv.as_mut_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glGetInteger64v error");
        glGetBooleanv(pname, bv.as_mut_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glGetBooleanv error");
    }
    Forms {
        int: iv.iter().copied().take_while(|x| *x != i32::MIN).collect(),
        float: fv.iter().copied().take_while(|x| !x.is_nan()).collect(),
        int64: lv.iter().copied().take_while(|x| *x != i64::MIN).collect(),
        boolean: bv.iter().copied().take_while(|x| *x != 0xEE).collect(),
    }
}

#[test]
fn float_int64_and_boolean_forms_match_the_integer_form() {
    let _g = context();
    let mut bad = Vec::new();
    for &(name, pname) in LIMITS {
        let q = query(name, pname);
        let n = q.int.len();
        assert!(n >= 1, "{name}: glGetIntegerv wrote nothing");
        if (q.float.len(), q.int64.len(), q.boolean.len()) != (n, n, n) {
            bad.push(format!(
                "{name}: value counts int {n} float {} int64 {} bool {}",
                q.float.len(),
                q.int64.len(),
                q.boolean.len()
            ));
            continue;
        }
        for i in 0..n {
            let want = i64::from(q.int[i]);
            if q.int64[i] != want {
                bad.push(format!("{name}[{i}]: int64 {} != int {want}", q.int64[i]));
            }
            // the float is the integer, rounded to f32 as the conversion does
            if q.float[i] != want as f32 {
                bad.push(format!("{name}[{i}]: float {} != int {want}", q.float[i]));
            }
            if q.boolean[i] != (want != 0) as u8 {
                bad.push(format!("{name}[{i}]: boolean {} for int {want}", q.boolean[i]));
            }
        }
    }
    assert!(bad.is_empty(), "query forms disagree:\n{}", bad.join("\n"));
}

/// No limit reads negative through the wide forms (a stencil-mask-style
/// unsigned widening or a sign slip would); MIN_PROGRAM_TEXEL_OFFSET, the one
/// negative limit, is left out of the list.
#[test]
fn no_limit_reads_negative_in_the_wide_forms() {
    let _g = context();
    for &(name, pname) in LIMITS {
        let q = query(name, pname);
        assert!(q.int64.iter().all(|v| *v >= 0), "{name}: int64 {:?}", q.int64);
        assert!(q.float.iter().all(|v| *v >= 0.0), "{name}: float {:?}", q.float);
    }
}

/// MIN_PROGRAM_TEXEL_OFFSET is negative in every form, not widened as
/// unsigned.
#[test]
fn min_program_texel_offset_keeps_its_sign() {
    let _g = context();
    let q = query("MIN_PROGRAM_TEXEL_OFFSET", GL_MIN_PROGRAM_TEXEL_OFFSET);
    assert_eq!(q.int, [-8]);
    assert_eq!(q.int64, [-8]);
    assert_eq!(q.float, [-8.0]);
    assert_eq!(q.boolean, [1]);
}

/// An unknown pname is GL_INVALID_ENUM through all four forms and writes
/// nothing.
#[test]
fn unknown_pname_is_invalid_enum_in_every_form() {
    let _g = context();
    let mut iv = [7i32; 2];
    let mut fv = [7f32; 2];
    let mut lv = [7i64; 2];
    let mut bv = [7u8; 2];
    for form in 0..4 {
        // SAFETY: room for two values.
        unsafe {
            error_clear();
            match form {
                0 => glGetIntegerv(0x7FFF, iv.as_mut_ptr()),
                1 => glGetFloatv(0x7FFF, fv.as_mut_ptr()),
                2 => glGetInteger64v(0x7FFF, lv.as_mut_ptr()),
                _ => glGetBooleanv(0x7FFF, bv.as_mut_ptr()),
            }
            assert_eq!(glGetError(), GL_INVALID_ENUM, "form {form}");
        }
    }
    assert_eq!((iv[0], fv[0], lv[0], bv[0]), (7, 7.0, 7, 7));
}
