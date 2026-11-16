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
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_asin_fs() {
    mali_ref::assert_matches("deqp_constop_ce_asin_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_atan_separate_fs() {
    mali_ref::assert_matches("deqp_constop_ce_atan_separate_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_ceil_fs() {
    mali_ref::assert_matches("deqp_constop_ce_ceil_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_dot_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_ce_dot_vec3_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_floor_fs() {
    mali_ref::assert_matches("deqp_constop_ce_floor_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_floor_vs() {
    mali_ref::assert_matches("deqp_constop_ce_floor_vs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_fract_vec2_fs() {
    mali_ref::assert_matches("deqp_constop_ce_fract_vec2_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_inversesqrt_vec4_fs() {
    mali_ref::assert_matches("deqp_constop_ce_inversesqrt_vec4_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_mod_vec2_float_fs() {
    mali_ref::assert_matches("deqp_constop_ce_mod_vec2_float_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_normalize_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_ce_normalize_vec3_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_sign_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_ce_sign_vec3_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_ce_smoothstep_fs() {
    mali_ref::assert_matches("deqp_constop_ce_smoothstep_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_cf_denorm_fs() {
    mali_ref::assert_matches("deqp_constop_cf_denorm_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_cf_inf_fs() {
    mali_ref::assert_matches("deqp_constop_cf_inf_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_loops_for_basic_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_loops_for_basic_mediump_float_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_loops_while_basic_highp_float_fs() {
    mali_ref::assert_matches("deqp_constop_loops_while_basic_highp_float_fs");
}

#[test]
#[ignore = "refused: a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly), lowered by lower-constop"]
fn constop_mat3sub_vs() {
    mali_ref::assert_matches("deqp_constop_mat3sub_vs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_matrix_preinc_mediump_mat3_fs() {
    mali_ref::assert_matches("deqp_constop_matrix_preinc_mediump_mat3_fs");
}

#[test]
#[ignore = "refused: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), lowered by lower-constop"]
fn constop_nz_local_cmp_fs() {
    mali_ref::assert_matches("deqp_constop_nz_local_cmp_fs");
}

#[test]
#[ignore = "refused: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), lowered by lower-constop"]
fn constop_nz_local_cmp_vs() {
    mali_ref::assert_matches("deqp_constop_nz_local_cmp_vs");
}

#[test]
#[ignore = "refused: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), lowered by lower-constop"]
fn constop_nz_prod_vs() {
    mali_ref::assert_matches("deqp_constop_nz_prod_vs");
}

#[test]
#[ignore = "refused: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), lowered by lower-constop"]
fn constop_nz_shared_vs() {
    mali_ref::assert_matches("deqp_constop_nz_shared_vs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_op_abs_highp_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_abs_highp_float_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_op_abs_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_abs_mediump_float_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_op_add_assign_effect_mediump_vec3_fs() {
    mali_ref::assert_matches("deqp_constop_op_add_assign_effect_mediump_vec3_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_op_degrees_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_degrees_mediump_float_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_op_minus_lowp_float_fs() {
    mali_ref::assert_matches("deqp_constop_op_minus_lowp_float_fs");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float, lowered by lower-constop"]
fn constop_struct_param_out_vs() {
    mali_ref::assert_matches("deqp_constop_struct_param_out_vs");
}

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSED: &[(&str, &str)] = &[
    ("deqp_constop_ce_asin_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_atan_separate_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_ceil_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_dot_vec3_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_floor_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_floor_vs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_fract_vec2_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_inversesqrt_vec4_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_mod_vec2_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_normalize_vec3_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_sign_vec3_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_ce_smoothstep_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_cf_denorm_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_cf_inf_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_loops_for_basic_mediump_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_loops_while_basic_highp_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_mat3sub_vs", "a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly)"),
    ("deqp_constop_matrix_preinc_mediump_mat3_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_nz_local_cmp_fs", "a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there)"),
    ("deqp_constop_nz_local_cmp_vs", "a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there)"),
    ("deqp_constop_nz_prod_vs", "a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there)"),
    ("deqp_constop_nz_shared_vs", "a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there)"),
    ("deqp_constop_op_abs_highp_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_op_abs_mediump_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_op_add_assign_effect_mediump_vec3_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_op_degrees_mediump_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_op_minus_lowp_float_fs", "a constant operation on or to a value that is not a finite normal float"),
    ("deqp_constop_struct_param_out_vs", "a constant operation on or to a value that is not a finite normal float"),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSED {
        mali_ref::assert_refused_ref(name, text);
    }
}
