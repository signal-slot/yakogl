// SPDX-License-Identifier: Apache-2.0
// Host tests for the ES 2.0 texture enum predicates (Table A rows 24-27).
// Literal hex values on purpose: they check the constants independently.
#![allow(dead_code)]

#[path = "../src/gl/es2_texture_rules.rs"]
mod rules;
use rules::*;

#[test]
fn target_refused() {
    assert!(es3_only_tex_target(0x806f)); // TEXTURE_3D
    assert!(es3_only_tex_target(0x8c1a)); // TEXTURE_2D_ARRAY
}

#[test]
fn target_kept() {
    assert!(!es3_only_tex_target(0x0de1)); // TEXTURE_2D
    assert!(!es3_only_tex_target(0x8513)); // TEXTURE_CUBE_MAP
    assert!(!es3_only_tex_target(0x8d65)); // TEXTURE_EXTERNAL_OES
}

#[test]
fn param_set_refused() {
    // WRAP_R, BASE_LEVEL, MAX_LEVEL, MIN_LOD, MAX_LOD, COMPARE_MODE,
    // COMPARE_FUNC, SWIZZLE_R/G/B/A
    for p in [
        0x8072u32, 0x813c, 0x813d, 0x813a, 0x813b, 0x884c, 0x884d, 0x8e42, 0x8e43, 0x8e44, 0x8e45,
    ] {
        assert!(es3_only_tex_param_set(p), "set {}", p);
        assert!(es3_only_tex_param_get(p), "get {}", p);
    }
}

#[test]
fn param_kept() {
    // MIN_FILTER, MAG_FILTER, WRAP_S, WRAP_T, MIRRORED_REPEAT (a value), max anisotropy
    for p in [0x2801u32, 0x2800, 0x2802, 0x2803, 0x8370, 0x84fe] {
        assert!(!es3_only_tex_param_set(p), "set {}", p);
        assert!(!es3_only_tex_param_get(p), "get {}", p);
    }
}

#[test]
fn param_get_only_refused() {
    // TEXTURE_IMMUTABLE_FORMAT, TEXTURE_IMMUTABLE_LEVELS
    for p in [0x912fu32, 0x82df] {
        assert!(es3_only_tex_param_get(p), "get {}", p);
        assert!(!es3_only_tex_param_set(p), "set {}", p);
    }
}

#[test]
fn pixel_type_refused() {
    let rgba = 0x1908;
    // BYTE, SHORT, INT, HALF_FLOAT, 2_10_10_10_REV, 10F_11F_11F_REV,
    // 5_9_9_9_REV, FLOAT_32_UNSIGNED_INT_24_8_REV
    for t in [0x1400u32, 0x1402, 0x1404, 0x140b, 0x8368, 0x8c3b, 0x8c3e, 0x8dad] {
        assert!(es3_only_pixel_type(t, rgba), "type {}", t);
    }
    // UNSIGNED_INT with a non-depth format
    assert!(es3_only_pixel_type(0x1405, rgba));
    assert!(es3_only_pixel_type(0x1405, 0x84f9)); // DEPTH_STENCIL
}

#[test]
fn pixel_type_kept() {
    let rgba = 0x1908;
    assert!(!es3_only_pixel_type(0x8d61, rgba)); // HALF_FLOAT_OES
    assert!(!es3_only_pixel_type(0x1405, 0x1902)); // UNSIGNED_INT, DEPTH_COMPONENT
    assert!(!es3_only_pixel_type(0x84fa, 0x84f9)); // UNSIGNED_INT_24_8_OES, DEPTH_STENCIL_OES
    assert!(!es3_only_pixel_type(0x1403, 0x1902)); // UNSIGNED_SHORT, DEPTH_COMPONENT
    assert!(!es3_only_pixel_type(0x1401, rgba)); // UNSIGNED_BYTE
    assert!(!es3_only_pixel_type(0x8363, 0x1907)); // UNSIGNED_SHORT_5_6_5
    assert!(!es3_only_pixel_type(0x8033, rgba)); // UNSIGNED_SHORT_4_4_4_4
    assert!(!es3_only_pixel_type(0x8034, rgba)); // UNSIGNED_SHORT_5_5_5_1
    assert!(!es3_only_pixel_type(0x1406, rgba)); // FLOAT (OES_texture_float)
}
