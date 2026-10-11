// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, texture part (item es2-tex-tests; the audit is
//! tools/cts/log/investigations/es2-error-rules.md, Table A "Textures"). These tests
//! come before the guards: the guards in src/gl/texture.rs are not written
//! yet, and each `#[ignore = "BUG: ..."]` below names what main answers
//! today; the guard removes it.
//!
//! A `report-es2` build refuses what ES 2.0 does not define; an ES 3.0 build
//! behaves as before. Decision per row:
//!
//! - Row 24, targets `GL_TEXTURE_3D` and `GL_TEXTURE_2D_ARRAY`: `report-es2`
//!   gives GL_INVALID_ENUM from `glBindTexture`, `glGenerateMipmap`,
//!   `glTexParameteri/f/iv/fv` and `glGetTexParameteriv/fv` (all through
//!   `binding_for`). The ES 3.0 build binds them (pinned).
//! - Row 25, setting `GL_TEXTURE_WRAP_R`, `_BASE_LEVEL`, `_MAX_LEVEL`,
//!   `_MIN_LOD`, `_MAX_LOD`, `_COMPARE_MODE`, `_COMPARE_FUNC`,
//!   `_SWIZZLE_R/G/B/A`: GL_INVALID_ENUM in `report-es2`. `GL_MIRRORED_REPEAT`
//!   as WRAP_S/T is ES 2.0 core and stays valid in both builds.
//! - Row 26, getting `GL_TEXTURE_IMMUTABLE_FORMAT` and `_LEVELS`:
//!   GL_INVALID_ENUM in `report-es2`.
//! - Row 27, pixel `type` of `glTexImage2D` and `glTexSubImage2D`: GL_BYTE,
//!   GL_SHORT, GL_INT, GL_UNSIGNED_INT with a non-depth format, GL_HALF_FLOAT
//!   (0x140B) and the four packed ES 3.0 types give GL_INVALID_ENUM in
//!   `report-es2`. GL_HALF_FLOAT_OES (0x8D61) and the GL_OES_depth_texture
//!   pairs (DEPTH_COMPONENT with UNSIGNED_SHORT / UNSIGNED_INT) stay accepted.
//!   `glCopyTexImage2D` and `glCopyTexSubImage2D` take no pixel type (the
//!   type comes from the framebuffer), so there is no type refusal to test
//!   there; their internal-format checks (row 32) are not in this item.
//!
//! Ignored today (the guard is missing; each gives the error shown):
//!
//! - `es2_3d_targets_are_invalid_enum` (row 24): today GL_NO_ERROR from
//!   glGenerateMipmap, glTexParameter* and glGetTexParameter* on both
//!   targets; glBindTexture of a name bound to no target gives GL_NO_ERROR
//!   too (it binds), and of a name bound to 2D gives GL_INVALID_OPERATION.
//! - `es2_setting_es3_parameters_is_invalid_enum` (row 25): today
//!   GL_NO_ERROR for all eleven names (the audit: setting is accepted,
//!   reading is gated).
//! - `es2_getting_immutable_state_is_invalid_enum` (row 26): today
//!   GL_NO_ERROR, the value is 0.
//! - `es2_teximage2d_es3_types_are_invalid_enum` and
//!   `es2_texsubimage2d_es3_types_are_invalid_enum` (row 27): today
//!   GL_INVALID_OPERATION for GL_BYTE with RGBA (the first type tried; the
//!   other types are not reached before the first assertion fails, so the
//!   guard item must check each).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_error_rules_texture
//! cargo test --no-default-features --features host-test,gpu-powervr --test es2_error_rules_texture
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
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
    name: c"es2-error-rules-texture".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    reset();
    g
}

/// Back to a known texture state after the previous test: nothing bound on
/// the 2D target, the error queue empty.
fn reset() {
    // SAFETY: plain GL call.
    unsafe { glBindTexture(GL_TEXTURE_2D, 0) };
    drain();
}

const GL_HALF_FLOAT_OES: GLenum = 0x8D61;
/// Zeroed pixels, big enough for one RGBA texel of 8 bytes or less.
static PIXELS: [u8; 16] = [0; 16];

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

/// A fresh 2D texture, bound.
fn bind_new_2d() -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls, room for one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    assert_eq!(err(), GL_NO_ERROR);
    t
}

/// `glTexImage2D` of one texel with `format` and `type_`; the error it gave.
fn tex_image(format: GLenum, type_: GLenum) -> GLenum {
    // SAFETY: `PIXELS` holds at least one texel of any type used here.
    unsafe {
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, 1, 1, 0, format, type_, PIXELS.as_ptr() as *const c_void)
    };
    err()
}

/// `glTexSubImage2D` into the texel made by `tex_image(RGBA, UNSIGNED_BYTE)`.
fn tex_sub_image(format: GLenum, type_: GLenum) -> GLenum {
    // SAFETY: as `tex_image`.
    unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, format, type_, PIXELS.as_ptr() as *const c_void) };
    err()
}

/// The ES 3.0-only targets of row 24.
const TARGETS_3D: [GLenum; 2] = [GL_TEXTURE_3D, GL_TEXTURE_2D_ARRAY];

/// Row 24: refused in every entry that goes through `binding_for`.
#[cfg(feature = "report-es2")]
#[test]
#[ignore = "BUG: report-es2 binds GL_TEXTURE_3D / GL_TEXTURE_2D_ARRAY and answers GL_NO_ERROR for them (not ES 2.0: GL_INVALID_ENUM); waits for the texture.rs guard (row 24)"]
fn es2_3d_targets_are_invalid_enum() {
    let _g = context();
    /* a name never bound: the refusal must come before any type clash */
    let mut t = 0;
    // SAFETY: room for one name.
    unsafe { glGenTextures(1, &mut t) };
    let mut i = 0;
    let mut f = 0.0;
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for target in TARGETS_3D {
            glBindTexture(target, t);
            assert_eq!(err(), GL_INVALID_ENUM, "glBindTexture 0x{target:x}");
            glGenerateMipmap(target);
            assert_eq!(err(), GL_INVALID_ENUM, "glGenerateMipmap 0x{target:x}");
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameteri 0x{target:x}");
            glTexParameterf(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLfloat);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameterf 0x{target:x}");
            let iv = GL_NEAREST as GLint;
            glTexParameteriv(target, GL_TEXTURE_MIN_FILTER, &iv);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameteriv 0x{target:x}");
            let fv = GL_NEAREST as GLfloat;
            glTexParameterfv(target, GL_TEXTURE_MIN_FILTER, &fv);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameterfv 0x{target:x}");
            glGetTexParameteriv(target, GL_TEXTURE_MIN_FILTER, &mut i);
            assert_eq!(err(), GL_INVALID_ENUM, "glGetTexParameteriv 0x{target:x}");
            glGetTexParameterfv(target, GL_TEXTURE_MIN_FILTER, &mut f);
            assert_eq!(err(), GL_INVALID_ENUM, "glGetTexParameterfv 0x{target:x}");
        }
    }
}

/// Row 24: the ES 3.0 build binds both targets, sets WRAP_R and reads it back.
#[cfg(not(feature = "report-es2"))]
#[test]
fn es3_3d_targets_bind() {
    let _g = context();
    let mut t = 0;
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for target in TARGETS_3D {
            /* a texture takes the type of its first target */
            glGenTextures(1, &mut t);
            glBindTexture(target, t);
            assert_eq!(err(), GL_NO_ERROR, "glBindTexture 0x{target:x}");
            glBindTexture(target, 0);
            assert_eq!(err(), GL_NO_ERROR);
        }
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_3D, t);
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE as GLint);
        assert_eq!(err(), GL_NO_ERROR);
        let mut v = 0;
        glGetTexParameteriv(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, &mut v);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(v, GL_CLAMP_TO_EDGE as GLint);
        glBindTexture(GL_TEXTURE_3D, 0);
    }
}

/// Row 24, kept: the two ES 2.0 targets bind and take parameters.
#[test]
fn es2_targets_are_accepted() {
    let _g = context();
    let t = bind_new_2d();
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        let mut v = 0;
        glGetTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, &mut v);
        assert_eq!(v, GL_NEAREST as GLint);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        /* a 2D texture on the cube map target is a type clash, not an enum error */
        assert_ne!(err(), GL_INVALID_ENUM);
        glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
        assert_eq!(err(), GL_NO_ERROR);
        glBindTexture(GL_TEXTURE_2D, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The ES 3.0-only parameters of row 25.
const SET_PNAMES: [(GLenum, GLint); 11] = [
    (GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE as GLint),
    (GL_TEXTURE_BASE_LEVEL, 0),
    (GL_TEXTURE_MAX_LEVEL, 1000),
    (GL_TEXTURE_MIN_LOD, -1000),
    (GL_TEXTURE_MAX_LOD, 1000),
    (GL_TEXTURE_COMPARE_MODE, GL_NONE as GLint),
    (GL_TEXTURE_COMPARE_FUNC, GL_LEQUAL as GLint),
    (GL_TEXTURE_SWIZZLE_R, GL_RED as GLint),
    (GL_TEXTURE_SWIZZLE_G, GL_GREEN as GLint),
    (GL_TEXTURE_SWIZZLE_B, GL_BLUE as GLint),
    (GL_TEXTURE_SWIZZLE_A, GL_ALPHA as GLint),
];

/// Row 25: setting any of them is GL_INVALID_ENUM, by all four setters.
#[cfg(feature = "report-es2")]
#[test]
#[ignore = "BUG: report-es2 sets the ES 3.0 texture parameters (WRAP_R, BASE/MAX_LEVEL, LODs, COMPARE_*, SWIZZLE_*) with GL_NO_ERROR (not ES 2.0: GL_INVALID_ENUM); waits for the texture.rs guard (row 25)"]
fn es2_setting_es3_parameters_is_invalid_enum() {
    let _g = context();
    bind_new_2d();
    // SAFETY: plain GL calls.
    unsafe {
        for (pname, value) in SET_PNAMES {
            glTexParameteri(GL_TEXTURE_2D, pname, value);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameteri 0x{pname:x}");
            glTexParameterf(GL_TEXTURE_2D, pname, value as GLfloat);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameterf 0x{pname:x}");
            glTexParameteriv(GL_TEXTURE_2D, pname, &value);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameteriv 0x{pname:x}");
            let fv = value as GLfloat;
            glTexParameterfv(GL_TEXTURE_2D, pname, &fv);
            assert_eq!(err(), GL_INVALID_ENUM, "glTexParameterfv 0x{pname:x}");
        }
    }
}

/// Row 25: the ES 3.0 build takes them all with no error.
#[cfg(not(feature = "report-es2"))]
#[test]
fn es3_setting_es3_parameters_works() {
    let _g = context();
    bind_new_2d();
    // SAFETY: plain GL calls.
    unsafe {
        for (pname, value) in SET_PNAMES {
            glTexParameteri(GL_TEXTURE_2D, pname, value);
            assert_eq!(err(), GL_NO_ERROR, "glTexParameteri 0x{pname:x}");
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, GL_REPEAT as GLint);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/// Row 25, kept: GL_MIRRORED_REPEAT is ES 2.0 core, and so are the four
/// ES 2.0 parameters.
#[test]
fn mirrored_repeat_and_es2_parameters_are_accepted() {
    let _g = context();
    bind_new_2d();
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for pname in [GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T] {
            for mode in [GL_MIRRORED_REPEAT, GL_REPEAT, GL_CLAMP_TO_EDGE] {
                glTexParameteri(GL_TEXTURE_2D, pname, mode as GLint);
                assert_eq!(err(), GL_NO_ERROR, "pname 0x{pname:x} mode 0x{mode:x}");
                let mut v = 0;
                glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut v);
                assert_eq!(v, mode as GLint);
            }
            glTexParameterf(GL_TEXTURE_2D, pname, GL_MIRRORED_REPEAT as GLfloat);
            assert_eq!(err(), GL_NO_ERROR);
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/// Row 26: reading the immutability state is GL_INVALID_ENUM.
#[cfg(feature = "report-es2")]
#[test]
#[ignore = "BUG: report-es2 answers GL_TEXTURE_IMMUTABLE_FORMAT / _LEVELS with GL_NO_ERROR (not ES 2.0: GL_INVALID_ENUM); waits for the texture.rs guard (row 26)"]
fn es2_getting_immutable_state_is_invalid_enum() {
    let _g = context();
    bind_new_2d();
    let mut i = -99;
    let mut f = -99.0;
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for pname in [GL_TEXTURE_IMMUTABLE_FORMAT, GL_TEXTURE_IMMUTABLE_LEVELS] {
            glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut i);
            assert_eq!(err(), GL_INVALID_ENUM, "iv 0x{pname:x}");
            glGetTexParameterfv(GL_TEXTURE_2D, pname, &mut f);
            assert_eq!(err(), GL_INVALID_ENUM, "fv 0x{pname:x}");
        }
    }
}

/// Row 26: the ES 3.0 build answers (0 for a texture that is not immutable).
#[cfg(not(feature = "report-es2"))]
#[test]
fn es3_getting_immutable_state_works() {
    let _g = context();
    bind_new_2d();
    let mut i = -99;
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for pname in [GL_TEXTURE_IMMUTABLE_FORMAT, GL_TEXTURE_IMMUTABLE_LEVELS] {
            glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut i);
            assert_eq!(err(), GL_NO_ERROR, "pname 0x{pname:x}");
            assert_eq!(i, 0);
        }
    }
}

/// The ES 3.0-only pixel types of row 27, each with a format that goes with it
/// in ES 3.0 (the error must not depend on the format being wrong).
#[cfg(feature = "report-es2")]
const ES3_TYPES: [(GLenum, GLenum); 9] = [
    (GL_BYTE, GL_RGBA),
    (GL_SHORT, GL_RGBA),
    (GL_INT, GL_RGBA),
    (GL_UNSIGNED_INT, GL_RGBA),
    (GL_HALF_FLOAT, GL_RGBA),
    (GL_UNSIGNED_INT_2_10_10_10_REV, GL_RGBA),
    (GL_UNSIGNED_INT_10F_11F_11F_REV, GL_RGB),
    (GL_UNSIGNED_INT_5_9_9_9_REV, GL_RGB),
    (GL_FLOAT_32_UNSIGNED_INT_24_8_REV, GL_DEPTH_STENCIL),
];

/// Row 27: glTexImage2D refuses them with GL_INVALID_ENUM.
#[cfg(feature = "report-es2")]
#[test]
#[ignore = "BUG: report-es2 glTexImage2D answers GL_INVALID_OPERATION to the ES 3.0-only pixel types (ES 2.0 3.7.1: GL_INVALID_ENUM); waits for the texture.rs guard (row 27)"]
fn es2_teximage2d_es3_types_are_invalid_enum() {
    let _g = context();
    bind_new_2d();
    for (type_, format) in ES3_TYPES {
        assert_eq!(tex_image(format, type_), GL_INVALID_ENUM, "type 0x{type_:x} format 0x{format:x}");
    }
}

/// Row 27: glTexSubImage2D refuses them with GL_INVALID_ENUM.
#[cfg(feature = "report-es2")]
#[test]
#[ignore = "BUG: report-es2 glTexSubImage2D answers GL_INVALID_OPERATION to the ES 3.0-only pixel types (not ES 2.0: GL_INVALID_ENUM); waits for the texture.rs guard (row 27)"]
fn es2_texsubimage2d_es3_types_are_invalid_enum() {
    let _g = context();
    bind_new_2d();
    assert_eq!(tex_image(GL_RGBA, GL_UNSIGNED_BYTE), GL_NO_ERROR);
    for (type_, format) in ES3_TYPES {
        assert_eq!(tex_sub_image(format, type_), GL_INVALID_ENUM, "type 0x{type_:x} format 0x{format:x}");
    }
}

/// Row 27, kept: the ES 2.0 types and the OES ones are not refused.
#[cfg(feature = "report-es2")]
#[test]
fn es2_pixel_types_are_accepted() {
    let _g = context();
    bind_new_2d();
    for (format, type_) in [
        (GL_RGBA, GL_UNSIGNED_BYTE),
        (GL_RGB, GL_UNSIGNED_BYTE),
        (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
        (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1),
        (GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
        (GL_LUMINANCE, GL_UNSIGNED_BYTE),
        (GL_ALPHA, GL_UNSIGNED_BYTE),
        (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE),
    ] {
        assert_eq!(tex_image(format, type_), GL_NO_ERROR, "format 0x{format:x} type 0x{type_:x}");
    }
}

/// Row 27, kept: GL_HALF_FLOAT_OES (0x8D61) is not the ES 3.0 enum. It is
/// taken with the format the driver accepts today (RGBA, as in
/// GL_OES_texture_half_float).
#[test]
fn half_float_oes_is_accepted() {
    let _g = context();
    bind_new_2d();
    assert_eq!(tex_image(GL_RGBA, GL_HALF_FLOAT_OES), GL_NO_ERROR);
    assert_eq!(tex_sub_image(GL_RGBA, GL_HALF_FLOAT_OES), GL_NO_ERROR);
}

/// Row 27, kept: the GL_OES_depth_texture pairs, listed in `report-es2`.
#[test]
fn oes_depth_texture_types_are_accepted() {
    let _g = context();
    bind_new_2d();
    for type_ in [GL_UNSIGNED_SHORT, GL_UNSIGNED_INT] {
        assert_eq!(tex_image(GL_DEPTH_COMPONENT, type_), GL_NO_ERROR, "type 0x{type_:x}");
    }
}

/// Row 27: the ES 3.0 build keeps the types it has (pins today's behaviour).
#[cfg(not(feature = "report-es2"))]
#[test]
fn es3_pixel_types_are_unchanged() {
    let _g = context();
    bind_new_2d();
    for (format, internal, type_) in [
        (GL_RGBA, GL_RGBA8, GL_UNSIGNED_BYTE),
        (GL_RGBA, GL_RGBA8_SNORM, GL_BYTE),
        (GL_RGBA, GL_RGB10_A2, GL_UNSIGNED_INT_2_10_10_10_REV),
        (GL_RGB, GL_R11F_G11F_B10F, GL_UNSIGNED_INT_10F_11F_11F_REV),
        (GL_RGB, GL_RGB9_E5, GL_UNSIGNED_INT_5_9_9_9_REV),
        (GL_RGBA, GL_RGBA16F, GL_HALF_FLOAT),
    ] {
        // SAFETY: `PIXELS` holds one texel.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 1, 1, 0, format, type_, PIXELS.as_ptr() as *const c_void)
        };
        assert_eq!(err(), GL_NO_ERROR, "internal 0x{internal:x} type 0x{type_:x}");
    }
}
