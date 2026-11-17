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

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws (past its constant operation since plan #367's lower-fs-arith), lowered by lower-constop"]
fn constop_loops_for_basic_mediump_float_fs() {
    mali_ref::assert_matches("deqp_constop_loops_for_basic_mediump_float_fs");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws (past its constant operation since plan #367's lower-fs-arith), lowered by lower-constop"]
fn constop_loops_while_basic_highp_float_fs() {
    mali_ref::assert_matches("deqp_constop_loops_while_basic_highp_float_fs");
}

#[test]
#[ignore = "refused: a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly), lowered by lower-constop"]
fn constop_mat3sub_vs() {
    mali_ref::assert_matches("deqp_constop_mat3sub_vs");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (past its constant operation), lowered by lower-constop"]
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

#[test]
#[ignore = "compiles but differs: vs.position word 2 is CSEL.u32.eq r1, r0, 0x0, u3.w0, 0x0, the reference has MOV.i32 r1, u3.w1 (Mesa's `a_position * z` of z = -0.0 is -0.0, ours 0.0); lowered by lower-constop"]
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

#[test]
#[ignore = "refused: a fragment shader operand of a form or in a place no reference draws (past its constant operation: the vec3 sum's lanes alone), lowered by lower-constop"]
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

/// Where the two loops references are refused since plan #367's
/// lower-fs-arith lowered the fragment arithmetic in front of it: their
/// constant operation is no longer the first refusal.
const VARYING_LOAD: &str = "a fragment varying load or conversion of a form no reference draws";

/// Every ignored reference above that is refused, and the refusal the compiler
/// gives it now (`constop_nz_shared_vs` compiles but differs).
const REFUSED: &[(&str, &str)] = &[
    ("deqp_constop_ce_atan_separate_fs", "a constant product in a shape Mesa's algebraic rules move or fold"),
    ("deqp_constop_loops_for_basic_mediump_float_fs", VARYING_LOAD),
    ("deqp_constop_loops_while_basic_highp_float_fs", VARYING_LOAD),
    (
        "deqp_constop_mat3sub_vs",
        "a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly)",
    ),
    (
        "deqp_constop_matrix_preinc_mediump_mat3_fs",
        "a gl_FragColor lane other than the varying's own lane or a constant",
    ),
    ("deqp_constop_op_abs_highp_float_fs", "a fragment shader that mixes highp and mediump or lowp float values"),
    (
        "deqp_constop_op_add_assign_effect_mediump_vec3_fs",
        "a fragment shader operand of a form or in a place no reference draws",
    ),
];

#[test]
fn refusals_pinned() {
    let changed: Vec<String> = REFUSED.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
