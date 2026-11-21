// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_constop_*` (plan #377, family
//! constop: constant operations on non-finite or non-normal floats, -0.0
//! through a variable, fp16 constants that do not hold; see
//! `docs/notes/mali-deqp-constop.md`). One test per reference. A reference
//! the compiler still refuses is `#[ignore]`d with its refusal quoted, and
//! `refusals_pinned` pins that refusal; lower-constop removes the ignore and
//! the pin together.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_constop
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
fn constop_ce_asin_fs() {
    mali_ref::assert_matches("deqp_constop_ce_asin_fs");
}

#[test]
#[ignore = "refused: a constant product in a shape Mesa's algebraic rules move or fold (atan's expansion of constants; folded, it gives 0xc016cbd6 where Mesa's GLSL body gives 0xc016cbe4), lowered by lower-constop"]
fn constop_ce_atan_separate_fs() {
    mali_ref::assert_matches("deqp_constop_ce_atan_separate_fs");
}

#[test]
fn constop_ce_ceil_fs() {
    mali_ref::assert_matches("deqp_constop_ce_ceil_fs");
}

#[test]
fn constop_ce_dot_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_ce_dot_vec3_fs");
}

#[test]
fn constop_ce_floor_fs() {
    mali_ref::assert_matches("deqp_constop_ce_floor_fs");
}

#[test]
fn constop_ce_floor_vs() {
    mali_ref::assert_matches("deqp_constop_ce_floor_vs");
}

#[test]
fn constop_ce_fract_vec2_fs() {
    mali_ref::assert_matches("deqp_constop_ce_fract_vec2_fs");
}

#[test]
fn constop_ce_inversesqrt_vec4_fs() {
    mali_ref::assert_matches("deqp_constop_ce_inversesqrt_vec4_fs");
}

#[test]
fn constop_ce_mod_vec2_float_fs() {
    mali_ref::assert_matches("deqp_constop_ce_mod_vec2_float_fs");
}

#[test]
fn constop_ce_normalize_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_ce_normalize_vec3_fs");
}

#[test]
fn constop_ce_sign_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_ce_sign_vec3_fs");
}

#[test]
fn constop_ce_smoothstep_fs() {
    mali_ref::assert_matches("deqp_constop_ce_smoothstep_fs");
}

#[test]
fn constop_cf_denorm_fs() {
    mali_ref::assert_matches("deqp_constop_cf_denorm_fs");
}

#[test]
fn constop_cf_inf_fs() {
    mali_ref::assert_matches("deqp_constop_cf_inf_fs");
}

/// Byte-equal since lower-mixprec's rotated colour (`(w, x, y, 1.0)`: w
/// alone, then the aligned pair (x, y) converted once).
#[test]
fn constop_loops_for_basic_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_loops_for_basic_mediump_float_fs");
}

/// The highp counter folded away with the unrolled loop (`nir_fs.rs`
/// `folded_counter`): byte-equal to the mediump counter's reference.
#[test]
fn constop_loops_while_basic_highp_float_fs() {
    mali_ref::assert_matches("deqp_constop_loops_while_basic_highp_float_fs");
}

/// H1: the mediump `in0 - in1` and the sum of its columns folded at fp16,
/// each constant and each result rounded to half (`0xb8009000`,
/// `0x3c003265`, moved into the fragment shader by the link).
#[test]
fn constop_mat3sub_vs() {
    mali_ref::assert_matches("deqp_constop_mat3sub_vs");
}

/// The packed mat3's column 1 pair across two slots one `vec2`
/// (`MKVEC.v2i16 r1, r1^.h1, r2.h0`), the matrix's columns one variable
/// of one conversion per slot, and `vec4(s, 1.0) + vec4(s, 0.0)` kept as
/// Mesa's `fadd` of two `vec2` (`nir_fs.rs`).
#[test]
fn constop_matrix_preinc_mediump_mat3_fs() {
    mali_ref::assert_matches("deqp_constop_matrix_preinc_mediump_mat3_fs");
}

#[test]
fn constop_nz_local_cmp_fs() {
    mali_ref::assert_matches("deqp_constop_nz_local_cmp_fs");
}

#[test]
fn constop_nz_local_cmp_vs() {
    mali_ref::assert_matches("deqp_constop_nz_local_cmp_vs");
}

#[test]
fn constop_nz_prod_vs() {
    mali_ref::assert_matches("deqp_constop_nz_prod_vs");
}

/// `a_position * -0.0` is -0.0 in every lane (the zero's own sign), and
/// the w that selects -0.0 or 1.0 is read by the reciprocal as +0.0 with
/// no `FMAX` clamp (`nir.rs`, the viewport transform).
#[test]
fn constop_nz_shared_vs() {
    mali_ref::assert_matches("deqp_constop_nz_shared_vs");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (past its constant operation), lowered by lower-mixprec"]
fn constop_op_abs_highp_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_abs_highp_float_fs");
}

#[test]
fn constop_op_abs_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_abs_mediump_float_fs");
}

/// Each sum of `in0.zxy + in1.yzx` one lane (no two pair under
/// `alu_srcs_equal`), their `vec2` read by the `FMA.v2f16` (`nir_fs.rs`
/// `split_sum`).
#[test]
fn constop_op_add_assign_effect_mediump_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_op_add_assign_effect_mediump_vec3_fs");
}

#[test]
fn constop_op_degrees_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_degrees_mediump_float_fs");
}

#[test]
fn constop_op_minus_lowp_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_minus_lowp_float_fs");
}

#[test]
fn constop_struct_param_out_vs() {
    mali_ref::assert_matches("deqp_constop_struct_param_out_vs");
}

/// Every ignored reference above, and the refusal the compiler gives it now.
const REFUSED: &[(&str, &str)] = &[
    ("deqp_constop_ce_atan_separate_fs", "a constant product in a shape Mesa's algebraic rules move or fold"),
    ("deqp_constop_op_abs_highp_float_fs", "a fragment shader that mixes highp and mediump or lowp float values"),
];

#[test]
fn refusals_pinned() {
    let changed: Vec<String> = REFUSED.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
