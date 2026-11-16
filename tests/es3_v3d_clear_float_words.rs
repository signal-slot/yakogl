// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The render list's clear-colour words of the V3D's float render targets
//! (`clear_mrt::clear_color_words`, plan #360 v3d-clear-words), pinned
//! against values worked out by hand from Mesa 26.2.1's packing, not taken
//! from the function or from `state::clear_words`:
//!
//! * internal type and bpp per format, v3dx_format_table.c
//!   get_internal_type_bpp_for_output_format: RGBA16F and R11F_G11F_B10F
//!   16F at 64 bpp (:253-260), R16F and RG16F 16F at 64 bpp (:263-269),
//!   RGBA32F 32F at 128 bpp (:319-321), RG32F 32F at 64 bpp (:323-325),
//!   R32F 32F at 32 bpp (:327-329);
//! * `v3d_tlb_clear` (v3dx_draw.c:1706-1769) copies `4 << internal_bpp`
//!   bytes (:1704) of the colour packed as R16G16B16A16_FLOAT for 16F
//!   (:1739-1743, `_mesa_float_to_half`, nearest even) or of its f32 bits
//!   for 32F; the words after them stay 0.
//!
//! Halves used below: 0.25 = 0x3400, 0.5 = 0x3800, 0.75 = 0x3a00,
//! 1.0 = 0x3c00, -0.0 = 0x8000, -0.5 = 0xb800, -1.0 = 0xbc00, -2.0 = 0xc000,
//! 65504 = 0x7bff (the largest half), -65504 = 0xfbff, infinity = 0x7c00.
//! Floats: 0.25 = 0x3e80_0000, 0.5 = 0x3f00_0000, 0.75 = 0x3f40_0000,
//! 1.0 = 0x3f80_0000, -1.5 = 0xbfc0_0000, 65504 = 0x477f_e000.
//!
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_clear_float_words
#![cfg(all(feature = "host-test", feature = "gpu-v3d"))]

use yakogl::v3d::clear_mrt::clear_color_words as words;
use yakogl::v3d::ffi::{
    V3D_INTERNAL_BPP_128, V3D_INTERNAL_BPP_32, V3D_INTERNAL_BPP_64, V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_TYPE_32F,
    V3D_INTERNAL_TYPE_32UI, V3D_INTERNAL_TYPE_8,
};

/// RGBA16F, RG16F, R16F and R11F_G11F_B10F: all 16F at 64 bpp, so one
/// packing, four halves in words 0 and 1.
fn half(rgba: [f32; 4]) -> [u32; 4] {
    words(V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, rgba)
}

#[test]
fn rgba16f_rg16f_r16f_r11f_g11f_b10f_pack_two_halves_a_word() {
    assert_eq!(half([0.25, 0.5, 0.75, 1.0]), [0x3800_3400, 0x3c00_3a00, 0, 0]);
    assert_eq!(half([1.0, 0.0, 0.0, 1.0]), [0x0000_3c00, 0x3c00_0000, 0, 0]);
    assert_eq!(half([-1.0, -0.5, -0.0, -2.0]), [0xb800_bc00, 0xc000_8000, 0, 0]);
    assert_eq!(half([65504.0, -65504.0, 65504.0, 65504.0]), [0xfbff_7bff, 0x7bff_7bff, 0, 0]);
}

#[test]
fn half_rounding_is_nearest_even_and_overflows_to_infinity() {
    /* 65520 is the tie between 65504 (odd mantissa) and 65536: up, to infinity */
    assert_eq!(half([65520.0, -1.0e9, 0.0, 0.0]), [0xfc00_7c00, 0, 0, 0]);
    /* 1 + 2^-11 is a tie, to 1.0; 1 + 3 * 2^-11 is a tie, to 1 + 2^-9 */
    let (t1, t3) = (1.0 + 2f32.powi(-11), 1.0 + 3.0 * 2f32.powi(-11));
    assert_eq!(half([t1, t3, 0.0, 0.0]), [0x3c02_3c00, 0, 0, 0]);
}

#[test]
fn r32f_is_word_zero_alone() {
    let r = |c| words(V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_32, c);
    assert_eq!(r([0.25, 0.5, 0.75, 1.0]), [0x3e80_0000, 0, 0, 0]);
    assert_eq!(r([1.0, 0.0, 0.0, 1.0]), [0x3f80_0000, 0, 0, 0]);
    assert_eq!(r([-1.5, -1.0, -1.0, -1.0]), [0xbfc0_0000, 0, 0, 0]);
    assert_eq!(r([65504.0, 65504.0, 0.0, 0.0]), [0x477f_e000, 0, 0, 0]);
}

#[test]
fn rg32f_is_words_zero_and_one() {
    let rg = |c| words(V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_64, c);
    assert_eq!(rg([0.25, 0.5, 0.75, 1.0]), [0x3e80_0000, 0x3f00_0000, 0, 0]);
    assert_eq!(rg([1.0, 0.0, 0.0, 1.0]), [0x3f80_0000, 0, 0, 0]);
    assert_eq!(rg([-1.5, -0.25, 1.0, 1.0]), [0xbfc0_0000, 0xbe80_0000, 0, 0]);
    assert_eq!(rg([65504.0, -65504.0, 0.0, 0.0]), [0x477f_e000, 0xc77f_e000, 0, 0]);
}

#[test]
fn rgba32f_is_all_four_words_as_they_are() {
    let rgba = |c| words(V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_128, c);
    assert_eq!(rgba([0.25, 0.5, 0.75, 1.0]), [0x3e80_0000, 0x3f00_0000, 0x3f40_0000, 0x3f80_0000]);
    assert_eq!(rgba([1.0, 0.0, 0.0, 1.0]), [0x3f80_0000, 0, 0, 0x3f80_0000]);
    assert_eq!(rgba([-1.0, -0.5, -0.25, -1.5]), [0xbf80_0000, 0xbf00_0000, 0xbe80_0000, 0xbfc0_0000]);
    assert_eq!(rgba([65504.0, -65504.0, 65504.0, 1.0]), [0x477f_e000, 0xc77f_e000, 0x477f_e000, 0x3f80_0000]);
    /* no conversion: -0.0, an infinity and a value no half holds keep their bits */
    assert_eq!(rgba([-0.0, f32::INFINITY, 1.0e9, 65520.0]), [0x8000_0000, 0x7f80_0000, 0x4e6e_6b28, 0x477f_f000]);
}

#[test]
fn other_internal_types_are_not_float_targets() {
    assert_eq!(words(V3D_INTERNAL_TYPE_8, V3D_INTERNAL_BPP_32, [1.0; 4]), [0; 4]);
    assert_eq!(words(V3D_INTERNAL_TYPE_32UI, V3D_INTERNAL_BPP_128, [1.0; 4]), [0; 4]);
}
