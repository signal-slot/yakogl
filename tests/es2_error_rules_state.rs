// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, state and program part (item changelog-es2-final; the
//! audit is docs/investigations/es2-error-rules.md, Table A "State and
//! program", rows 14-17 and 19-21).
//!
//! A `report-es2` build refuses what ES 2.0 does not define with
//! GL_INVALID_ENUM; an ES 3.0 build behaves as before, so this file exists in
//! `report-es2` builds only. One test per row: the error, the state left
//! unchanged, and the ES 2.0 values still working.
//!
//! - Row 14, glEnable/glDisable: GL_RASTERIZER_DISCARD and
//!   GL_PRIMITIVE_RESTART_FIXED_INDEX.
//! - Row 15, glIsEnabled: the same two (returns GL_FALSE with the error).
//! - Row 16, glGetBooleanv/Integerv/Floatv/Integer64v: the same two as pnames.
//! - Row 17, glBlendEquation(Separate): GL_MIN and GL_MAX. GL_EXT_blend_minmax
//!   is not in ALL_EXTENSION_NAMES (src/gl/state.rs), so the row applies; the
//!   extension string is checked below.
//! - Row 19, glPixelStorei: the eight ES 3.0 names.
//! - Row 20, glGet*: the ES 3.0 pnames of the row.
//! - Row 21, glGetProgramiv: GL_PROGRAM_BINARY_LENGTH,
//!   GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_ACTIVE_UNIFORM_BLOCKS,
//!   GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_error_rules_state
//! ```

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
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
    name: c"es2-error-rules-state".as_ptr() as *const c_char,
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

/// Back to the defaults the tests rely on after the previous test.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBlendEquation(GL_FUNC_ADD);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glPixelStorei(GL_PACK_ALIGNMENT, 4);
        for cap in ES2_CAPS {
            glDisable(cap);
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

fn geti(pname: GLenum) -> GLint {
    let mut v = -99;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// The four getters on `pname`: each must raise GL_INVALID_ENUM and write nothing.
fn assert_all_getters_refuse(pname: GLenum) {
    let mut i = 0x5a5a;
    let mut b = 0x5au8;
    let mut f = 1234.5f32;
    let mut l = 0x5a5a as GLint64;
    // SAFETY: room for one value each.
    unsafe {
        glGetIntegerv(pname, &mut i);
        assert_eq!(err(), GL_INVALID_ENUM, "glGetIntegerv {pname:#x}");
        glGetBooleanv(pname, &mut b);
        assert_eq!(err(), GL_INVALID_ENUM, "glGetBooleanv {pname:#x}");
        glGetFloatv(pname, &mut f);
        assert_eq!(err(), GL_INVALID_ENUM, "glGetFloatv {pname:#x}");
        glGetInteger64v(pname, &mut l);
        assert_eq!(err(), GL_INVALID_ENUM, "glGetInteger64v {pname:#x}");
    }
    assert_eq!((i, b, f, l), (0x5a5a, 0x5a, 1234.5, 0x5a5a), "{pname:#x}: the output was written");
}

const ES3_CAPS: [GLenum; 2] = [GL_RASTERIZER_DISCARD, GL_PRIMITIVE_RESTART_FIXED_INDEX];
const ES2_CAPS: [GLenum; 9] = [
    GL_BLEND,
    GL_CULL_FACE,
    GL_DEPTH_TEST,
    GL_DITHER,
    GL_POLYGON_OFFSET_FILL,
    GL_SAMPLE_ALPHA_TO_COVERAGE,
    GL_SAMPLE_COVERAGE,
    GL_SCISSOR_TEST,
    GL_STENCIL_TEST,
];

#[test]
fn row14_enable_disable_refuse_es3_caps() {
    let _g = context();
    for cap in ES3_CAPS {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(cap);
            assert_eq!(err(), GL_INVALID_ENUM, "glEnable {cap:#x}");
            glDisable(cap);
            assert_eq!(err(), GL_INVALID_ENUM, "glDisable {cap:#x}");
        }
    }
    // the state did not change: the ES 3.0 flags the draw path reads are still off
    let c = yakogl::gl::context::current().expect("context");
    assert!(!c.primitive_restart && !c.rasterizer_discard);
    // the nine ES 2.0 caps still toggle
    for cap in ES2_CAPS {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(cap);
            assert_eq!(glIsEnabled(cap), GL_TRUE as GLboolean, "{cap:#x} on");
            glDisable(cap);
            assert_eq!(glIsEnabled(cap), GL_FALSE as GLboolean, "{cap:#x} off");
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn row15_is_enabled_refuses_es3_caps() {
    let _g = context();
    for cap in ES3_CAPS {
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glIsEnabled(cap) }, GL_FALSE as GLboolean, "{cap:#x}");
        assert_eq!(err(), GL_INVALID_ENUM, "glIsEnabled {cap:#x}");
    }
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        assert_eq!(glIsEnabled(GL_BLEND), GL_TRUE as GLboolean);
        glDisable(GL_BLEND);
        assert_eq!(glIsEnabled(GL_BLEND), GL_FALSE as GLboolean);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn row16_get_refuses_es3_caps() {
    let _g = context();
    for cap in ES3_CAPS {
        assert_all_getters_refuse(cap);
    }
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_SCISSOR_TEST) };
    assert_eq!(geti(GL_SCISSOR_TEST), 1);
    let mut b = 0u8;
    // SAFETY: room for one value.
    unsafe { glGetBooleanv(GL_SCISSOR_TEST, &mut b) };
    assert_eq!(b, 1);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
    assert_eq!(geti(GL_SCISSOR_TEST), 0);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn row17_blend_equation_refuses_min_max() {
    let _g = context();
    // GL_EXT_blend_minmax is not advertised, so ES 2.0's three are all there is
    // SAFETY: the extension string is NUL-terminated and static.
    let ext = unsafe { core::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    assert!(!ext.to_str().unwrap().contains("blend_minmax"));
    // SAFETY: plain GL calls.
    unsafe {
        glBlendEquationSeparate(GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT);
        for (rgb, a) in [(GL_MIN, GL_FUNC_ADD), (GL_FUNC_ADD, GL_MAX), (GL_MAX, GL_MIN)] {
            glBlendEquationSeparate(rgb, a);
            assert_eq!(err(), GL_INVALID_ENUM, "separate {rgb:#x} {a:#x}");
        }
        for m in [GL_MIN, GL_MAX] {
            glBlendEquation(m);
            assert_eq!(err(), GL_INVALID_ENUM, "glBlendEquation {m:#x}");
        }
    }
    // the state did not change
    assert_eq!(geti(GL_BLEND_EQUATION_RGB), GL_FUNC_SUBTRACT as GLint);
    assert_eq!(geti(GL_BLEND_EQUATION_ALPHA), GL_FUNC_REVERSE_SUBTRACT as GLint);
    // the three ES 2.0 equations work
    for m in [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT] {
        // SAFETY: plain GL call.
        unsafe { glBlendEquation(m) };
        assert_eq!(geti(GL_BLEND_EQUATION_RGB), m as GLint);
        assert_eq!(geti(GL_BLEND_EQUATION_ALPHA), m as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn row19_pixel_storei_refuses_es3_names() {
    let _g = context();
    let names = [
        GL_UNPACK_ROW_LENGTH,
        GL_UNPACK_SKIP_ROWS,
        GL_UNPACK_SKIP_PIXELS,
        GL_UNPACK_SKIP_IMAGES,
        GL_UNPACK_IMAGE_HEIGHT,
        GL_PACK_ROW_LENGTH,
        GL_PACK_SKIP_ROWS,
        GL_PACK_SKIP_PIXELS,
    ];
    for n in names {
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(n, 7) };
        assert_eq!(err(), GL_INVALID_ENUM, "{n:#x}");
    }
    // nothing was stored (the ES 3.0 build's getters are refused here, so read the context)
    let c = yakogl::gl::context::current().expect("context");
    assert_eq!(
        [c.unpack_row_length, c.unpack_skip_rows, c.unpack_skip_pixels, c.unpack_skip_images, c.unpack_image_height],
        [0; 5]
    );
    assert_eq!([c.pack_row_length, c.pack_skip_rows, c.pack_skip_pixels], [0; 3]);
    // the two ES 2.0 names work, and keep their value check
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 2);
    }
    assert_eq!((geti(GL_UNPACK_ALIGNMENT), geti(GL_PACK_ALIGNMENT)), (1, 2));
    // SAFETY: plain GL call.
    unsafe { glPixelStorei(GL_PACK_ALIGNMENT, 3) };
    assert_eq!(err(), GL_INVALID_VALUE);
    assert_eq!(geti(GL_PACK_ALIGNMENT), 2);
}

/// Every pname of audit row 20, the pixel-store names of row 19 included.
const ROW20: &[GLenum] = &[
    GL_COPY_READ_BUFFER_BINDING,
    GL_COPY_WRITE_BUFFER_BINDING,
    GL_PIXEL_PACK_BUFFER_BINDING,
    GL_PIXEL_UNPACK_BUFFER_BINDING,
    GL_UNIFORM_BUFFER_BINDING,
    GL_SAMPLER_BINDING,
    GL_TRANSFORM_FEEDBACK_BINDING,
    GL_UNPACK_ROW_LENGTH,
    GL_UNPACK_SKIP_ROWS,
    GL_UNPACK_SKIP_PIXELS,
    GL_UNPACK_SKIP_IMAGES,
    GL_UNPACK_IMAGE_HEIGHT,
    GL_PACK_ROW_LENGTH,
    GL_PACK_SKIP_ROWS,
    GL_PACK_SKIP_PIXELS,
    GL_MAX_VARYING_COMPONENTS,
    GL_MAX_VERTEX_OUTPUT_COMPONENTS,
    GL_MAX_FRAGMENT_INPUT_COMPONENTS,
    GL_MAX_VERTEX_UNIFORM_COMPONENTS,
    GL_MAX_FRAGMENT_UNIFORM_COMPONENTS,
    GL_MAX_VERTEX_UNIFORM_BLOCKS,
    GL_MAX_FRAGMENT_UNIFORM_BLOCKS,
    GL_MAX_COMBINED_UNIFORM_BLOCKS,
    GL_MAX_UNIFORM_BUFFER_BINDINGS,
    GL_MAX_UNIFORM_BLOCK_SIZE,
    GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT,
    GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
    GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
    GL_MAX_SAMPLES,
    GL_NUM_PROGRAM_BINARY_FORMATS,
    GL_PROGRAM_BINARY_FORMATS,
    GL_MAX_ARRAY_TEXTURE_LAYERS,
    GL_MAX_3D_TEXTURE_SIZE,
    GL_MAX_ELEMENT_INDEX,
    GL_MAX_ELEMENTS_VERTICES,
    GL_MAX_ELEMENTS_INDICES,
    GL_MAX_TEXTURE_LOD_BIAS,
    GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS,
    GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS,
    GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS,
    GL_TRANSFORM_FEEDBACK_ACTIVE,
    GL_TRANSFORM_FEEDBACK_PAUSED,
    GL_MIN_PROGRAM_TEXEL_OFFSET,
    GL_MAX_PROGRAM_TEXEL_OFFSET,
    GL_MAX_SERVER_WAIT_TIMEOUT,
    GL_READ_FRAMEBUFFER_BINDING,
    GL_READ_BUFFER,
    GL_TEXTURE_BINDING_2D_ARRAY,
    GL_TEXTURE_BINDING_3D,
    0x90EB, /* GL_MAX_COMPUTE_WORK_GROUP_INVOCATIONS (ES 3.1) */
];

#[test]
fn row20_get_refuses_es3_pnames() {
    let _g = context();
    for &p in ROW20 {
        assert_all_getters_refuse(p);
    }
    // the ES 2.0 pnames next to them still answer
    assert_eq!(geti(GL_MAX_TEXTURE_SIZE), 4096);
    assert!(geti(GL_MAX_VERTEX_ATTRIBS) >= 8);
    assert!(geti(GL_MAX_VARYING_VECTORS) >= 8);
    assert!(geti(GL_MAX_VERTEX_UNIFORM_VECTORS) >= 128);
    assert_eq!(geti(GL_TEXTURE_BINDING_2D), 0);
    assert_eq!(geti(GL_FRAMEBUFFER_BINDING), 0);
    assert_eq!(geti(GL_UNPACK_ALIGNMENT), 4);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn row21_get_programiv_refuses_es3_pnames() {
    let _g = context();
    // SAFETY: plain GL call.
    let p = unsafe { glCreateProgram() };
    assert_ne!(p, 0);
    for pname in [
        GL_PROGRAM_BINARY_LENGTH,
        GL_PROGRAM_BINARY_RETRIEVABLE_HINT,
        GL_ACTIVE_UNIFORM_BLOCKS,
        GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH,
    ] {
        let mut v = 0x5a5a;
        // SAFETY: room for one value.
        unsafe { glGetProgramiv(p, pname, &mut v) };
        assert_eq!(err(), GL_INVALID_ENUM, "{pname:#x}");
        assert_eq!(v, 0x5a5a, "{pname:#x}: the output was written");
    }
    // the nine ES 2.0 pnames answer without an error
    for pname in [
        GL_DELETE_STATUS,
        GL_LINK_STATUS,
        GL_VALIDATE_STATUS,
        GL_INFO_LOG_LENGTH,
        GL_ATTACHED_SHADERS,
        GL_ACTIVE_ATTRIBUTES,
        GL_ACTIVE_ATTRIBUTE_MAX_LENGTH,
        GL_ACTIVE_UNIFORMS,
        GL_ACTIVE_UNIFORM_MAX_LENGTH,
    ] {
        let mut v = 0x5a5a;
        // SAFETY: room for one value.
        unsafe { glGetProgramiv(p, pname, &mut v) };
        assert_eq!((err(), v), (GL_NO_ERROR, 0), "{pname:#x}");
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}
