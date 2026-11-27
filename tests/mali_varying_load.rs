// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! VARYING_LOAD (#32, plan #367 lower-train-2, from proto-varying-load): a
//! mediump fragment shader whose fp16 trees read two or more vec3 varyings,
//! which the link packs into one 16-byte slot (the second one's x in the
//! first one's w). glmark2's bump fs 77c5 (`NormalEye - height_factor * dh.x
//! * TangentEye - ..`, line 46), terrain fs 4d26 (`mat3(vTangent, vBinormal,
//! vNormal)`, line 74) and ideas fs c565/1aca (`normalize(vertex_normal)`
//! beside `dot(reflection, eye_direction)`) have this form
//! (docs/notes/mali-varying-load.md).
//!
//! Mesa 26.2.1 on the drm-shim (tools/mali/testdata/gm2_vload_1..6, original
//! reductions; _2 stops at a later family) loads the slot `.v4` and converts
//! it in aligned lane pairs whatever variable each lane is: `V2F32_TO_V2F16
//! r1, r2^, r3^` is (v_n.z, v_e.x), and `MKVEC.v2i16 r3, r1.h1, r2.h0`
//! rebuilds v_e.xy for the fp16 sum.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_varying_load
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// Two vec3s summed: slot 0 `.v4` (v_n.xyz, v_e.x) converted as (x, y) and
/// (z, w), slot 1 `.v2` (v_e.yz) as one pair, `MKVEC.v2i16 r3, r1.h1,
/// r2.h0`, then `FADD.v2f16 r0, r0^, r3^` and `FADD.v2f16 r1, r1^.h00,
/// r2^.h11`.
#[test]
fn gm2_vload_1_two_vec3s_summed_match_their_reference() {
    mali_ref::assert_matches("gm2_vload_1");
}

/// Three vec3s across three slots, summed (77c5's NormalEye, TangentEye
/// and BitangentEye; 4d26's mat3 columns): slot 1 `.v4` (v_t.yz, v_b.xy)
/// in two pairs, slot 2's one lane (v_b.z) converted alone, `FADD.f32
/// r3.h0, r3^, 0x0.neg`.
#[test]
fn gm2_vload_5_three_vec3s_summed_match_their_reference() {
    mali_ref::assert_matches("gm2_vload_5");
}

/// 77c5's line 46 itself, a product of a vec3 whose lanes straddle two
/// slots (`v_n - 2.0 * v_t - 3.0 * v_b`): past VARYING_LOAD, the operand
/// of the rebuilt pair stops at FS_OPERAND, a family of its own (Mesa:
/// `FMA.v2f16 r0, 0x40000000.neg.h11, r3^, r0^` of the `MKVEC`'s pair).
/// When it moves, make this [`mali_ref::assert_matches`].
#[test]
fn gm2_vload_2_a_product_of_a_straddling_vec3_stays_refused() {
    mali_ref::assert_refused_ref("gm2_vload_2", "a fragment shader operand of a form or in a place");
}

/// ideas vs 51b2's layout, a vec4 between two vec3s, all read in one fp16
/// tree: Mesa lays the whole vec4 out first (slot 0) and the vec3s after it.
#[test]
fn gm2_vload_3_vec3_vec4_vec3_matches_its_reference() {
    mali_ref::assert_matches("gm2_vload_3");
}

/// The dot product of the two vec3s (c565's `dot(reflection,
/// eye_direction)`): the same conversions, then the `FMA.v2f16` chain.
#[test]
fn gm2_vload_4_dot_of_two_vec3s_matches_its_reference() {
    mali_ref::assert_matches("gm2_vload_4");
}

/// Four vec3s (4d26's vTangent, vBinormal, vNormal, vViewPosition): the
/// fourth starts at slot 2's odd lane 1, so its (x, y) is the high half of
/// the pair (v_n.z, v_v.x) and the low half of (v_v.y, v_v.z):
/// `MKVEC.v2i16 r3, r3^.h1, r2.h0`.
#[test]
fn gm2_vload_6_four_vec3s_match_their_reference() {
    mali_ref::assert_matches("gm2_vload_6");
}

/// On main the narrowed guard also reaches two older references, both now
/// byte-equal: gm2_varylink_pack_v3v4v3 (ideas' three varyings with
/// `vertex_position.w` as the alpha: ATEST reads the high half of the
/// (z, w) pair conversion, `ATEST .. r3.h1`) and sel_v_duplicate (two
/// varyings of equal value Mesa's link merges into one slot).
#[test]
fn older_packed_references_match_theirs() {
    mali_ref::assert_matches("gm2_varylink_pack_v3v4v3");
    mali_ref::assert_matches("sel_v_duplicate");
}

/// A shader that mixes precisions (gm2_cf_loop1_if_fs's `highp vec3 lv`
/// beside two packed mediump vec3s) keeps the varying-load refusal: past
/// it the back end has no measured form.
#[test]
fn a_mixed_precision_shader_keeps_the_varying_load_refusal() {
    mali_ref::assert_refused_ref("gm2_cf_loop1_if_fs", "a fragment varying load or conversion of a form");
}
