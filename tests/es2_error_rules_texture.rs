// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, texture part (issue #128; the audit is
//! tools/cts/log/investigations/es2-error-rules.md, Table A "Textures").
//!
//! A `report-es2` build refuses what ES 2.0 does not define; an ES 3.0 build
//! behaves as before. The guards are in src/gl/texture.rs, one predicate per
//! rule. Per row:
//!
//! - Row 24, targets `GL_TEXTURE_3D` and `GL_TEXTURE_2D_ARRAY`
//!   (`es3_only_texture_target`): `report-es2` gives GL_INVALID_ENUM from
//!   `glBindTexture`, `glGenerateMipmap`, `glTexParameteri/f/iv/fv` and
//!   `glGetTexParameteriv/fv` (all through `binding_for`). The ES 3.0 build
//!   binds them.
//! - Rows 25 and 26, texture parameter names outside ES 2.0 Table 3.10
//!   (`es3_only_tex_param`): setting `GL_TEXTURE_WRAP_R`, `_BASE_LEVEL`,
//!   `_MAX_LEVEL`, `_MIN_LOD`, `_MAX_LOD`, `_COMPARE_MODE`, `_COMPARE_FUNC`,
//!   `_SWIZZLE_R/G/B/A`, and getting those or `GL_TEXTURE_IMMUTABLE_FORMAT` /
//!   `_LEVELS`, is GL_INVALID_ENUM in `report-es2`, and a refused getter
//!   writes nothing. `GL_MIRRORED_REPEAT` as WRAP_S/T is ES 2.0 core and stays
//!   valid in both builds.
//! - Row 27, pixel `type` of `glTexImage2D` and `glTexSubImage2D`
//!   (`es3_only_pixel_type`, in the format/type check both calls share):
//!   GL_BYTE, GL_SHORT, GL_INT, GL_HALF_FLOAT (0x140B), the four packed
//!   ES 3.0 types, and GL_FLOAT (GL_OES_texture_float is not claimed) give
//!   GL_INVALID_ENUM in `report-es2`. GL_HALF_FLOAT_OES (0x8D61) and the
//!   GL_OES_depth_texture pairs (DEPTH_COMPONENT with UNSIGNED_SHORT /
//!   UNSIGNED_INT) stay accepted. GL_UNSIGNED_SHORT and GL_UNSIGNED_INT are
//!   types GL_OES_depth_texture adds, so with a colour format they are a
//!   known type in a pair the GL does not take: GL_INVALID_OPERATION, not
//!   GL_INVALID_ENUM (the audit row listed GL_UNSIGNED_INT as ES 3.0-only;
//!   the claimed extension makes it allowable). `glCopyTexImage2D` and
//!   `glCopyTexSubImage2D` take no pixel type.
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

/// Asserts that one call of row `row` gave GL_INVALID_ENUM, as ES 2.0
/// (`spec`) wants.
#[cfg(feature = "report-es2")]
fn invalid_enum(row: u32, call: &str, got: GLenum, spec: &str) {
    assert_eq!(got, GL_INVALID_ENUM, "row {row}: {call}: got 0x{got:x}, ES 2.0 wants GL_INVALID_ENUM ({spec})");
}

/// Row 24's spec reference.
#[cfg(feature = "report-es2")]
const ROW24_SPEC: &str = "ES 2.0 3.7.4, 3.7.11, 3.7.13, 6.1.3: TEXTURE_2D and TEXTURE_CUBE_MAP only";

/// Row 24: everything that goes through `binding_for`, and glBindTexture,
/// refuses the two ES 3.0 targets; the getters write nothing.
#[cfg(feature = "report-es2")]
#[test]
fn es2_3d_targets_are_invalid_enum() {
    let _g = context();
    /* a name never bound: the refusal comes before any type clash */
    let mut t = 0;
    // SAFETY: room for one name.
    unsafe { glGenTextures(1, &mut t) };
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for target in TARGETS_3D {
            let call = |name: &str| format!("{name}(0x{target:x})");
            glBindTexture(target, t);
            invalid_enum(24, &call("glBindTexture"), err(), ROW24_SPEC);
            glGenerateMipmap(target);
            invalid_enum(24, &call("glGenerateMipmap"), err(), ROW24_SPEC);
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            invalid_enum(24, &call("glTexParameteri"), err(), ROW24_SPEC);
            glTexParameterf(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLfloat);
            invalid_enum(24, &call("glTexParameterf"), err(), ROW24_SPEC);
            let iv = GL_NEAREST as GLint;
            glTexParameteriv(target, GL_TEXTURE_MIN_FILTER, &iv);
            invalid_enum(24, &call("glTexParameteriv"), err(), ROW24_SPEC);
            let fv = GL_NEAREST as GLfloat;
            glTexParameterfv(target, GL_TEXTURE_MIN_FILTER, &fv);
            invalid_enum(24, &call("glTexParameterfv"), err(), ROW24_SPEC);
            let mut i = -99;
            glGetTexParameteriv(target, GL_TEXTURE_MIN_FILTER, &mut i);
            invalid_enum(24, &call("glGetTexParameteriv"), err(), ROW24_SPEC);
            assert_eq!(i, -99, "row 24: a refused glGetTexParameteriv(0x{target:x}) writes nothing (ES 2.0 2.5)");
            let mut f = -99.0;
            glGetTexParameterfv(target, GL_TEXTURE_MIN_FILTER, &mut f);
            invalid_enum(24, &call("glGetTexParameterfv"), err(), ROW24_SPEC);
            assert_eq!(f, -99.0, "row 24: a refused glGetTexParameterfv(0x{target:x}) writes nothing (ES 2.0 2.5)");
        }
        /* the name stayed unbound: GL_TEXTURE_2D still takes it */
        glBindTexture(GL_TEXTURE_2D, t);
        assert_eq!(err(), GL_NO_ERROR, "row 24: the refused binds left the name without a target");
        glBindTexture(GL_TEXTURE_2D, 0);
    }
    drain();
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

/// Row 25: all four setters refuse every one of the eleven names.
#[cfg(feature = "report-es2")]
#[test]
fn es2_setting_es3_parameters_is_invalid_enum() {
    let _g = context();
    bind_new_2d();
    const SPEC: &str = "ES 2.0 3.7.4 Table 3.10: WRAP_S, WRAP_T, MIN_FILTER, MAG_FILTER only";
    // SAFETY: plain GL calls.
    unsafe {
        for (pname, value) in SET_PNAMES {
            glTexParameteri(GL_TEXTURE_2D, pname, value);
            invalid_enum(25, &format!("glTexParameteri(0x{pname:x})"), err(), SPEC);
            glTexParameterf(GL_TEXTURE_2D, pname, value as GLfloat);
            invalid_enum(25, &format!("glTexParameterf(0x{pname:x})"), err(), SPEC);
            glTexParameteriv(GL_TEXTURE_2D, pname, &value);
            invalid_enum(25, &format!("glTexParameteriv(0x{pname:x})"), err(), SPEC);
            let fv = value as GLfloat;
            glTexParameterfv(GL_TEXTURE_2D, pname, &fv);
            invalid_enum(25, &format!("glTexParameterfv(0x{pname:x})"), err(), SPEC);
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

/// Row 26: both getters refuse the two immutable-state names (and the
/// eleven of row 25) and write nothing.
#[cfg(feature = "report-es2")]
#[test]
fn es2_getting_es3_parameters_is_invalid_enum() {
    let _g = context();
    bind_new_2d();
    const SPEC: &str = "ES 2.0 6.1.3: value must be one of table 3.10";
    let names =
        [GL_TEXTURE_IMMUTABLE_FORMAT, GL_TEXTURE_IMMUTABLE_LEVELS].into_iter().chain(SET_PNAMES.map(|(p, _)| p));
    // SAFETY: plain GL calls, room for one value.
    unsafe {
        for pname in names {
            let mut i = -99;
            glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut i);
            invalid_enum(26, &format!("glGetTexParameteriv(0x{pname:x})"), err(), SPEC);
            assert_eq!(i, -99, "row 26: a refused glGetTexParameteriv(0x{pname:x}) writes nothing (ES 2.0 2.5)");
            let mut f = -99.0;
            glGetTexParameterfv(GL_TEXTURE_2D, pname, &mut f);
            invalid_enum(26, &format!("glGetTexParameterfv(0x{pname:x})"), err(), SPEC);
            assert_eq!(f, -99.0, "row 26: a refused glGetTexParameterfv(0x{pname:x}) writes nothing (ES 2.0 2.5)");
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

/// The pixel types of row 27 that ES 2.0 and the claimed extensions do not
/// have, each with a format that goes with it in ES 3.0 (the error must not
/// depend on the format being wrong). GL_FLOAT is GL_OES_texture_float's,
/// which is not claimed.
#[cfg(feature = "report-es2")]
const ES3_TYPES: [(GLenum, GLenum); 9] = [
    (GL_BYTE, GL_RGBA),
    (GL_SHORT, GL_RGBA),
    (GL_INT, GL_RGBA),
    (GL_HALF_FLOAT, GL_RGBA),
    (GL_FLOAT, GL_RGBA),
    (GL_UNSIGNED_INT_2_10_10_10_REV, GL_RGBA),
    (GL_UNSIGNED_INT_10F_11F_11F_REV, GL_RGB),
    (GL_UNSIGNED_INT_5_9_9_9_REV, GL_RGB),
    (GL_FLOAT_32_UNSIGNED_INT_24_8_REV, GL_DEPTH_STENCIL),
];

/// Row 27's spec reference.
#[cfg(feature = "report-es2")]
const ROW27_SPEC: &str =
    "ES 2.0 3.6.2 Table 3.2 and the claimed extensions' types; another type is GL_INVALID_ENUM (2.5)";

/// Row 27: glTexImage2D refuses every one of the nine types. Every type is
/// checked, not only the first.
#[cfg(feature = "report-es2")]
#[test]
fn es2_teximage2d_es3_types_are_invalid_enum() {
    let _g = context();
    bind_new_2d();
    for (type_, format) in ES3_TYPES {
        let call = format!("glTexImage2D(type 0x{type_:x}, format 0x{format:x})");
        invalid_enum(27, &call, tex_image(format, type_), ROW27_SPEC);
    }
}

/// Row 27: glTexSubImage2D refuses the same nine types into an RGBA /
/// UNSIGNED_BYTE texel, GL_HALF_FLOAT (0x140B) included (the rule is in the
/// format/type check the two calls share).
#[cfg(feature = "report-es2")]
#[test]
fn es2_texsubimage2d_es3_types_are_invalid_enum() {
    let _g = context();
    bind_new_2d();
    assert_eq!(tex_image(GL_RGBA, GL_UNSIGNED_BYTE), GL_NO_ERROR);
    for (type_, format) in ES3_TYPES {
        let call = format!("glTexSubImage2D(type 0x{type_:x}, format 0x{format:x})");
        invalid_enum(27, &call, tex_sub_image(format, type_), ROW27_SPEC);
    }
}

/// Row 27: GL_UNSIGNED_SHORT and GL_UNSIGNED_INT are GL_OES_depth_texture's
/// types, allowable on this surface; with a colour format the pair is not
/// one the GL takes, GL_INVALID_OPERATION, in both calls.
#[cfg(feature = "report-es2")]
#[test]
fn es2_depth_extension_types_with_a_colour_format_are_invalid_operation() {
    let _g = context();
    bind_new_2d();
    for type_ in [GL_UNSIGNED_SHORT, GL_UNSIGNED_INT] {
        assert_eq!(tex_image(GL_RGBA, type_), GL_INVALID_OPERATION, "glTexImage2D type 0x{type_:x}");
    }
    assert_eq!(tex_image(GL_RGBA, GL_UNSIGNED_BYTE), GL_NO_ERROR);
    for type_ in [GL_UNSIGNED_SHORT, GL_UNSIGNED_INT] {
        assert_eq!(tex_sub_image(GL_RGBA, type_), GL_INVALID_OPERATION, "glTexSubImage2D type 0x{type_:x}");
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
        /* The ES 3.0 build must not call the type unknown. A back end without
        a storage format for the pair (V3D has no SNORM, R11F or RGB9_E5
        texels) answers GL_INVALID_OPERATION, which is not a refusal of the
        type, so only GL_INVALID_ENUM is excluded. */
        assert_ne!(err(), GL_INVALID_ENUM, "internal 0x{internal:x} type 0x{type_:x}");
    }
}
