// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The TMU texture type of a depth texture sampled with a compare on the
//! V3D 4.2 (item pin-shadow-type; the fix is item fix-shadow-type).
//!
//! The GL core keeps every depth texture (DEPTH_COMPONENT16/24/32F) as
//! GLES_TEXFMT_R32F, and `v3d::texture::tmu_format` (src/v3d/texture.rs, the
//! `GLES_TEXFMT_R32F` arms) gives it the colour type
//! V3D_TEXTURE_DATA_FORMAT_R32F (29) whatever the sampler does, also under
//! GL_TEXTURE_COMPARE_MODE = GL_COMPARE_REF_TO_TEXTURE. Mesa's V3D 4.2 format
//! table compares only on its depth types DEPTH_COMP16, DEPTH24_X8 and
//! DEPTH_COMP32F (gallium/drivers/v3d/v3dx_format_table.c:149-156); a compare
//! on the colour type R32F does not compare, so the shadow look-ups of
//! dEQP-GLES3 images T08, T09 and 21 cases of S32a render wrong images on
//! the Pi 4.
//!
//! Here: the type `tmu_format` gives a depth binding (`compare_op` is the
//! GL core's compare word: 16 = no compare, 0..7 a compare function, bit 3 a
//! fixed-point depth). With the compare off the type is R32F and stays so
//! after the fix (the control). With a compare it is R32F today; the tests
//! named `..._flip_when_shadow_type_is_depth_comp32f` flip with the fix.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_type_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use yakogl::backend::*;
use yakogl::v3d::ffi::*;
use yakogl::v3d::texture;

/// GL_LEQUAL under GL_COMPARE_REF_TO_TEXTURE: less (bit 0) | equal (bit 1).
const LEQUAL: u8 = 3;
/// Bit 3: a fixed-point depth (DEPTH_COMPONENT16/24).
const FIXED: u8 = 8;
/// Bit 4: no compare (GL_NONE).
const NO_COMPARE: u8 = 16;

/// A depth texture as the GL core binds it: R32F storage, `compare_op`.
fn depth(compare_op: u8) -> GlesTextureBind {
    GlesTextureBind {
        type_: GLES_TEXTYPE_2D,
        format: GLES_TEXFMT_R32F,
        layers: 1,
        width: 4,
        height: 4,
        stride_texels: 4,
        mip_levels: 1,
        compare_op,
        ..GlesTextureBind::ZERO
    }
}

fn type_of(b: &GlesTextureBind) -> u32 {
    texture::tmu_format(b).0
}

/// Control: with the compare off both depth classes are the colour type
/// R32F (stays true after the fix).
#[test]
fn a_depth_texture_without_compare_is_r32f() {
    // DEPTH_COMPONENT32F: not fixed point
    assert_eq!(type_of(&depth(NO_COMPARE)), V3D_TEXTURE_DATA_FORMAT_R32F);
    // DEPTH_COMPONENT24: fixed point
    assert_eq!(type_of(&depth(NO_COMPARE | FIXED)), V3D_TEXTURE_DATA_FORMAT_R32F);
}

/// DEPTH_COMPONENT32F with GL_COMPARE_REF_TO_TEXTURE is R32F today.
#[test]
fn a_compared_depth32f_texture_is_r32f_flip_when_shadow_type_is_depth_comp32f() {
    // fix-shadow-type changes this to V3D_TEXTURE_DATA_FORMAT_DEPTH_COMP32F
    // (Mesa's v3dx_format_table.c depth rows, 149-156), the type the TMU
    // compares on; it fixes the shadow Fails of images T08, T09 and S32a.
    assert_eq!(type_of(&depth(LEQUAL)), V3D_TEXTURE_DATA_FORMAT_R32F);
}

/// DEPTH_COMPONENT24 with GL_COMPARE_REF_TO_TEXTURE is R32F today.
#[test]
fn a_compared_depth24_texture_is_r32f_flip_when_shadow_type_is_depth_comp32f() {
    // fix-shadow-type changes this to V3D_TEXTURE_DATA_FORMAT_DEPTH_COMP32F
    // too (the core stores D24 as 32-bit floats), for images T08, T09, S32a.
    assert_eq!(type_of(&depth(LEQUAL | FIXED)), V3D_TEXTURE_DATA_FORMAT_R32F);
}
