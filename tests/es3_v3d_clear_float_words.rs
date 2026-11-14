// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The clear-colour words of float render targets (`clear_mrt::clear_color_words`,
//! plan #360 v3d-clear-words), against values worked out by hand from Mesa
//! 26.2.1's packing: `v3d_clear` copies `4 << internal_bpp` bytes of the
//! R16G16B16A16_FLOAT-packed colour (16F) or of the f32 bits (32F);
//! internal types from v3dx_format_table.c:220-331.
//!
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_clear_float_words
#![cfg(all(feature = "host-test", feature = "gpu-v3d"))]

use yakogl::v3d::clear_mrt::clear_color_words as w;
use yakogl::v3d::ffi::*;

const F16: u32 = V3D_INTERNAL_TYPE_16F;
const F32: u32 = V3D_INTERNAL_TYPE_32F;
const B32: u32 = V3D_INTERNAL_BPP_32;
const B64: u32 = V3D_INTERNAL_BPP_64;
const B128: u32 = V3D_INTERNAL_BPP_128;

/// Halves: 0.25 = 0x3400, 0.5 = 0x3800, 0.75 = 0x3a00, 1.0 = 0x3c00,
/// -1.0 = 0xbc00, -0.5 = 0xb800, -2.0 = 0xc000, 65504 = 0x7bff, inf = 0x7c00.
#[test]
fn half_targets_pack_two_halves_a_word() {
    /* RGBA16F, RG16F, R16F and R11F_G11F_B10F are all 16F at 64 bpp */
    assert_eq!(w(F16, B64, [0.25, 0.5, 0.75, 1.0]), [0x3800_3400, 0x3c00_3a00, 0, 0]);
    assert_eq!(w(F16, B64, [1.0, 0.0, 0.0, 1.0]), [0x0000_3c00, 0x3c00_0000, 0, 0]);
    assert_eq!(w(F16, B64, [-1.0, -0.5, -0.0, -2.0]), [0xb800_bc00, 0xc000_8000, 0, 0]);
    assert_eq!(w(F16, B64, [65504.0, -65504.0, 65504.0, 65504.0]), [0xfbff_7bff, 0x7bff_7bff, 0, 0]);
    /* beyond the largest half: infinity */
    assert_eq!(w(F16, B64, [65520.0, -1.0e9, 0.0, 0.0]), [0xfc00_7c00, 0, 0, 0]);
    /* nearest even: 1 + 2^-11 is a tie, to 1.0; 1 + 3 * 2^-11 a tie, to 1 + 2^-9 */
    let (t1, t3) = (1.0 + 2f32.powi(-11), 1.0 + 3.0 * 2f32.powi(-11));
    assert_eq!(w(F16, B64, [t1, t3, 0.0, 0.0]), [0x3c02_3c00, 0, 0, 0]);
}

#[test]
fn r32f_is_word_zero() {
    assert_eq!(w(F32, B32, [0.25, 0.5, 0.75, 1.0]), [0x3e80_0000, 0, 0, 0]);
    assert_eq!(w(F32, B32, [1.0, 0.0, 0.0, 1.0]), [0x3f80_0000, 0, 0, 0]);
    assert_eq!(w(F32, B32, [-1.5, 1.0, 1.0, 1.0]), [0xbfc0_0000, 0, 0, 0]);
    assert_eq!(w(F32, B32, [65504.0, 0.0, 0.0, 0.0]), [0x477f_e000, 0, 0, 0]);
}

#[test]
fn rg32f_is_words_zero_and_one() {
    assert_eq!(w(F32, B64, [0.25, 0.5, 0.75, 1.0]), [0x3e80_0000, 0x3f00_0000, 0, 0]);
    assert_eq!(w(F32, B64, [1.0, 0.0, 0.0, 1.0]), [0x3f80_0000, 0, 0, 0]);
    assert_eq!(w(F32, B64, [-1.5, -0.25, 1.0, 1.0]), [0xbfc0_0000, 0xbe80_0000, 0, 0]);
    assert_eq!(w(F32, B64, [65504.0, -65504.0, 0.0, 0.0]), [0x477f_e000, 0xc77f_e000, 0, 0]);
}

#[test]
fn rgba32f_is_all_four_words() {
    assert_eq!(w(F32, B128, [0.25, 0.5, 0.75, 1.0]), [0x3e80_0000, 0x3f00_0000, 0x3f40_0000, 0x3f80_0000]);
    assert_eq!(w(F32, B128, [1.0, 0.0, 0.0, 1.0]), [0x3f80_0000, 0, 0, 0x3f80_0000]);
    assert_eq!(w(F32, B128, [-1.0, -0.5, -0.25, -65504.0]), [0xbf80_0000, 0xbf00_0000, 0xbe80_0000, 0xc77f_e000]);
    /* raw bits, no conversion: an infinity and a value a half cannot hold are kept */
    assert_eq!(
        w(F32, B128, [f32::INFINITY, 65504.0, -0.0, 1.0e9]),
        [0x7f80_0000, 0x477f_e000, 0x8000_0000, 1.0e9f32.to_bits()]
    );
}

#[test]
fn other_types_are_not_float_targets() {
    assert_eq!(w(V3D_INTERNAL_TYPE_8, B32, [1.0; 4]), [0; 4]);
    assert_eq!(w(V3D_INTERNAL_TYPE_32UI, B128, [1.0; 4]), [0; 4]);
}
