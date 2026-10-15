// SPDX-License-Identifier: Apache-2.0
// ES 2.0 texture error rules: enums that ES 2.0 refuses but ES 3.0 accepts.
// Self-contained (no crate:: or super:: imports) because it is also included
// by tests/es2_texture_rules_pred.rs. Values are copied from consts.rs.
// See docs/notes/es2-error-rules.md, Table A rows 24-27.
#![allow(dead_code)]

const TEXTURE_3D: u32 = 0x806f;
const TEXTURE_2D_ARRAY: u32 = 0x8c1a;

const TEXTURE_WRAP_R: u32 = 0x8072;
const TEXTURE_MIN_LOD: u32 = 0x813a;
const TEXTURE_MAX_LOD: u32 = 0x813b;
const TEXTURE_BASE_LEVEL: u32 = 0x813c;
const TEXTURE_MAX_LEVEL: u32 = 0x813d;
const TEXTURE_COMPARE_MODE: u32 = 0x884c;
const TEXTURE_COMPARE_FUNC: u32 = 0x884d;
const TEXTURE_SWIZZLE_R: u32 = 0x8e42;
const TEXTURE_SWIZZLE_G: u32 = 0x8e43;
const TEXTURE_SWIZZLE_B: u32 = 0x8e44;
const TEXTURE_SWIZZLE_A: u32 = 0x8e45;
const TEXTURE_IMMUTABLE_FORMAT: u32 = 0x912f;
const TEXTURE_IMMUTABLE_LEVELS: u32 = 0x82df;

const BYTE: u32 = 0x1400;
const SHORT: u32 = 0x1402;
const INT: u32 = 0x1404;
const UNSIGNED_INT: u32 = 0x1405;
const HALF_FLOAT: u32 = 0x140b;
const UNSIGNED_INT_2_10_10_10_REV: u32 = 0x8368;
const UNSIGNED_INT_10F_11F_11F_REV: u32 = 0x8c3b;
const UNSIGNED_INT_5_9_9_9_REV: u32 = 0x8c3e;
const FLOAT_32_UNSIGNED_INT_24_8_REV: u32 = 0x8dad;
const DEPTH_COMPONENT: u32 = 0x1902;

/// Table A row 24: texture targets that ES 2.0 does not have.
pub(crate) const fn es3_only_tex_target(target: u32) -> bool {
    matches!(target, TEXTURE_3D | TEXTURE_2D_ARRAY)
}

/// Table A row 25: parameter names ES 2.0 refuses in glTexParameter*.
/// GL_MIRRORED_REPEAT (0x8370) is a value, not a name, and stays valid in ES 2.0.
pub(crate) const fn es3_only_tex_param_set(pname: u32) -> bool {
    matches!(
        pname,
        TEXTURE_WRAP_R
            | TEXTURE_BASE_LEVEL
            | TEXTURE_MAX_LEVEL
            | TEXTURE_MIN_LOD
            | TEXTURE_MAX_LOD
            | TEXTURE_COMPARE_MODE
            | TEXTURE_COMPARE_FUNC
            | TEXTURE_SWIZZLE_R
            | TEXTURE_SWIZZLE_G
            | TEXTURE_SWIZZLE_B
            | TEXTURE_SWIZZLE_A
    )
}

/// Table A rows 25 and 26: parameter names ES 2.0 refuses in glGetTexParameter*.
/// This is the row 25 set plus TEXTURE_IMMUTABLE_FORMAT and TEXTURE_IMMUTABLE_LEVELS.
pub(crate) const fn es3_only_tex_param_get(pname: u32) -> bool {
    es3_only_tex_param_set(pname) || matches!(pname, TEXTURE_IMMUTABLE_FORMAT | TEXTURE_IMMUTABLE_LEVELS)
}

/// Table A row 27: pixel types ES 2.0 refuses with GL_INVALID_ENUM.
/// GL_HALF_FLOAT_OES (0x8D61), the OES_depth_texture types and the ES 2.0
/// core types are accepted. UNSIGNED_INT is accepted only for DEPTH_COMPONENT.
pub(crate) const fn es3_only_pixel_type(ty: u32, format: u32) -> bool {
    match ty {
        BYTE
        | SHORT
        | INT
        | HALF_FLOAT
        | UNSIGNED_INT_2_10_10_10_REV
        | UNSIGNED_INT_10F_11F_11F_REV
        | UNSIGNED_INT_5_9_9_9_REV
        | FLOAT_32_UNSIGNED_INT_24_8_REV => true,
        UNSIGNED_INT => format != DEPTH_COMPONENT,
        _ => false,
    }
}
