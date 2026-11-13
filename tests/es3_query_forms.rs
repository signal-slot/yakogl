// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The float and 64-bit forms of the implementation limits answer what the
//! integer form does (ES 3.0.6 section 6.1.1: glGetFloatv and glGetInteger64v
//! convert the same state). dEQP's `implementation_limits` and
//! `state_query.integers*` cases query each limit as `_getinteger`,
//! `_getfloat` and `_getinteger64`; on the boards the `_getfloat` and
//! `_getinteger64` rows of a limit fail together with its `_getinteger` row
//! because the VALUE is below the ES 3.0 minimum (tests/es3_caps.rs
//! `known_gaps`), not because the float or 64-bit path differs: glGetFloatv and
//! glGetInteger64v widen `get_integerv`'s answer (gl/state.rs). This test pins
//! that, so a limit that moves moves in all three forms and the `_getfloat` /
//! `_getinteger64` rows are not a second defect to chase.
//!
//! Not run under `report-es2` (the ES 3.0-only queries are INVALID_ENUM).
//!
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_query_forms
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_query_forms
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_query_forms

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> std::sync::MutexGuard<'static, ()> {
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

/// The single-valued implementation-dependent limits of ES 3.0.6 Tables
/// 6.32-6.34 that the library answers.
const LIMITS: &[(&str, GLenum)] = &[
    ("MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS),
    ("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS),
    ("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS),
    ("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS),
    ("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS),
    ("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS),
    ("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS),
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
    ("MAX_SAMPLES", GL_MAX_SAMPLES),
    ("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS),
    ("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS),
    ("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE),
    ("MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE),
    ("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS),
    ("MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE),
    ("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE),
    ("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS),
    ("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS),
    ("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS),
    ("MAX_PROGRAM_TEXEL_OFFSET", GL_MAX_PROGRAM_TEXEL_OFFSET),
    ("MAX_ELEMENTS_VERTICES", GL_MAX_ELEMENTS_VERTICES),
    ("MAX_ELEMENTS_INDICES", GL_MAX_ELEMENTS_INDICES),
    ("MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX),
    ("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS),
    ("MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS),
    ("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS),
    ("NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS),
];

#[test]
fn float_and_int64_forms_answer_what_the_integer_form_does() {
    let _g = context();
    for &(name, pname) in LIMITS {
        let mut i = [i32::MIN; 8];
        let mut l = [i64::MIN; 8];
        let mut f = [f32::NAN; 8];
        // SAFETY: room for the eight values the widest pname returns.
        unsafe {
            glGetIntegerv(pname, i.as_mut_ptr());
            glGetInteger64v(pname, l.as_mut_ptr());
            glGetFloatv(pname, f.as_mut_ptr());
        }
        assert_ne!(i[0], i32::MIN, "{name}: glGetIntegerv wrote nothing");
        assert_eq!(l[0], i[0] as i64, "{name}: glGetInteger64v differs from glGetIntegerv");
        assert_eq!(f[0], i[0] as f32, "{name}: glGetFloatv differs from glGetIntegerv");
        // one value each: nothing past it is touched by any form
        assert_eq!(i[1], i32::MIN, "{name}: glGetIntegerv wrote more than one value");
        assert_eq!(l[1], i64::MIN, "{name}: glGetInteger64v wrote more than one value");
        assert!(f[1].is_nan(), "{name}: glGetFloatv wrote more than one value");
    }
}

#[test]
fn uniform_vectors_fit_in_components_and_combined_covers_the_default_block() {
    let _g = context();
    let geti = |p: GLenum| {
        let mut v = [i32::MIN; 4];
        // SAFETY: room for four values.
        unsafe { glGetIntegerv(p, v.as_mut_ptr()) };
        v[0] as i64
    };
    for (vecs, comps, combined) in [
        (GL_MAX_VERTEX_UNIFORM_VECTORS, GL_MAX_VERTEX_UNIFORM_COMPONENTS, GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS),
        (
            GL_MAX_FRAGMENT_UNIFORM_VECTORS,
            GL_MAX_FRAGMENT_UNIFORM_COMPONENTS,
            GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
        ),
    ] {
        assert!(geti(vecs) * 4 <= geti(comps), "vectors * 4 > components");
        assert!(geti(combined) >= geti(comps), "combined < default block");
    }
}
