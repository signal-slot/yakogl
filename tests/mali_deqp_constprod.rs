// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_constprod_*` (plan #387, item
//! constprod-ref, `docs/notes/mali-deqp-constprod.md`), one test per
//! reference. `tests/mali_ref` does the compare; the references were drawn
//! with `tools/mali/deqp_ref.sh --instrument shim` (`--uniform-values`: each
//! uniform word of the FAU is the value gl_ref.py set).
//!
//! Each reference is the shader pair of one dEQP-GLES2 case, verbatim, the
//! smallest of the shape's cases. The refused ones stay `#[ignore]`d with
//! their refusal ([`REFUSED`], asserted exactly by `refusals_pinned`): the
//! lowering that lifts one must move it, in the same commit, to [`LOWERED`]
//! and drop the `#[ignore]`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_constprod
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal, without the `deqp_constprod_`
/// prefix: each has a byte-compare test below, not ignored.
const LOWERED: &[&str] = &["fma_matmul_mat_vs"];

/// The references that compile with other words than Mesa's: each has an
/// ignored byte-compare test.
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    (
        "alg_constmat_fs",
        "Valhall: a shape Mesa's algebraic rules rewrite without a reference (max(x, 0.0), clamp(x, -1.0, 1.0), a negated float(bool) in a product) is not lowered yet",
    ),
    (
        "chainlit_fs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "chainlit_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "constchain_attrib_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "constchain_degrees_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "constchain_fs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "constchain_loop_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "fma_foldcall_acos_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_foldcall_atan_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_foldcall_mod_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_foldcall_reflect_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_foldcall_refract_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_geom_reflect_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_geom_refract_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_invariance_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_matmul_const_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_matmul_dynamic_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_mulassign_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_random_trig_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_trig_acos_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "fma_trig_atan2_vs",
        "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet",
    ),
    (
        "foldcall_acos_fs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "foldcall_reflect_fs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "litfirst_array_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "litfirst_mat_fs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "litfirst_mat_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "litfirst_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "neglit_pair_fs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "neglit_pair_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
    (
        "reassoc_mixed_vs",
        "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet",
    ),
];

#[test]
#[ignore = "refused: matrix.mul.const_highp_mat2_float_fragment (pinned by refusals_pinned)"]
fn alg_constmat_fs() {
    mali_ref::assert_matches("deqp_constprod_alg_constmat_fs");
}

#[test]
#[ignore = "refused: random.conditionals.fragment.34 (pinned by refusals_pinned)"]
fn chainlit_fs() {
    mali_ref::assert_matches("deqp_constprod_chainlit_fs");
}

#[test]
#[ignore = "refused: random.conditionals.vertex.34 (pinned by refusals_pinned)"]
fn chainlit_vs() {
    mali_ref::assert_matches("deqp_constprod_chainlit_vs");
}

#[test]
#[ignore = "refused: default_vertex_attrib.vec4.vertex_attrib_1f (pinned by refusals_pinned)"]
fn constchain_attrib_vs() {
    mali_ref::assert_matches("deqp_constprod_constchain_attrib_vs");
}

#[test]
#[ignore = "refused: operator.angle_and_trigonometry.degrees.highp_vec2_vertex (pinned by refusals_pinned)"]
fn constchain_degrees_vs() {
    mali_ref::assert_matches("deqp_constprod_constchain_degrees_vs");
}

#[test]
#[ignore = "refused: random.comparison_ops.fragment.38 (pinned by refusals_pinned)"]
fn constchain_fs() {
    mali_ref::assert_matches("deqp_constprod_constchain_fs");
}

#[test]
#[ignore = "refused: invariance.highp.loop_0 (pinned by refusals_pinned)"]
fn constchain_loop_vs() {
    mali_ref::assert_matches("deqp_constprod_constchain_loop_vs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.trigonometry.acos_float_vertex (pinned by refusals_pinned)"]
fn fma_foldcall_acos_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_foldcall_acos_vs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.trigonometry.atan_separate_float_vertex (pinned by refusals_pinned)"]
fn fma_foldcall_atan_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_foldcall_atan_vs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.common.mod_float_vertex (pinned by refusals_pinned)"]
fn fma_foldcall_mod_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_foldcall_mod_vs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.geometric.reflect_float_vertex (pinned by refusals_pinned)"]
fn fma_foldcall_reflect_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_foldcall_reflect_vs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.geometric.refract_float_vertex (pinned by refusals_pinned)"]
fn fma_foldcall_refract_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_foldcall_refract_vs");
}

#[test]
#[ignore = "refused: operator.geometric.reflect.highp_float_vertex (pinned by refusals_pinned)"]
fn fma_geom_reflect_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_geom_reflect_vs");
}

#[test]
#[ignore = "refused: operator.geometric.refract.highp_float_float_vertex (pinned by refusals_pinned)"]
fn fma_geom_refract_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_geom_refract_vs");
}

#[test]
#[ignore = "refused: invariance.highp.subexpression_precision_highp (pinned by refusals_pinned)"]
fn fma_invariance_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_invariance_vs");
}

#[test]
#[ignore = "refused: matrix.mul.const_highp_mat2_mat2_vertex (pinned by refusals_pinned)"]
fn fma_matmul_const_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_matmul_const_vs");
}

#[test]
#[ignore = "refused: matrix.mul.dynamic_highp_mat3_vec3_vertex (pinned by refusals_pinned)"]
fn fma_matmul_dynamic_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_matmul_dynamic_vs");
}

#[test]
fn fma_matmul_mat_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_matmul_mat_vs");
}

#[test]
#[ignore = "refused: matrix.mul_assign.dynamic_highp_mat2_vertex (pinned by refusals_pinned)"]
fn fma_mulassign_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_mulassign_vs");
}

#[test]
#[ignore = "refused: random.trigonometric.vertex.22 (pinned by refusals_pinned)"]
fn fma_random_trig_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_random_trig_vs");
}

#[test]
#[ignore = "refused: operator.angle_and_trigonometry.acos.highp_float_vertex (pinned by refusals_pinned)"]
fn fma_trig_acos_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_trig_acos_vs");
}

#[test]
#[ignore = "refused: operator.angle_and_trigonometry.atan2.highp_float_vertex (pinned by refusals_pinned)"]
fn fma_trig_atan2_vs() {
    mali_ref::assert_matches("deqp_constprod_fma_trig_atan2_vs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.trigonometry.acos_float_fragment (pinned by refusals_pinned)"]
fn foldcall_acos_fs() {
    mali_ref::assert_matches("deqp_constprod_foldcall_acos_fs");
}

#[test]
#[ignore = "refused: constant_expressions.builtin_functions.geometric.reflect_float_fragment (pinned by refusals_pinned)"]
fn foldcall_reflect_fs() {
    mali_ref::assert_matches("deqp_constprod_foldcall_reflect_fs");
}

#[test]
#[ignore = "refused: functions.array_arguments.global_in_float_vertex (pinned by refusals_pinned)"]
fn litfirst_array_vs() {
    mali_ref::assert_matches("deqp_constprod_litfirst_array_vs");
}

#[test]
#[ignore = "refused: functions.datatypes.mat2_fragment (pinned by refusals_pinned)"]
fn litfirst_mat_fs() {
    mali_ref::assert_matches("deqp_constprod_litfirst_mat_fs");
}

#[test]
#[ignore = "refused: functions.datatypes.mat2_vertex (pinned by refusals_pinned)"]
fn litfirst_mat_vs() {
    mali_ref::assert_matches("deqp_constprod_litfirst_mat_vs");
}

#[test]
#[ignore = "refused: random.basic_expression.vertex.57 (pinned by refusals_pinned)"]
fn litfirst_vs() {
    mali_ref::assert_matches("deqp_constprod_litfirst_vs");
}

#[test]
#[ignore = "refused: functions.overloading.user_func_arguments_fragment (pinned by refusals_pinned)"]
fn neglit_pair_fs() {
    mali_ref::assert_matches("deqp_constprod_neglit_pair_fs");
}

#[test]
#[ignore = "refused: functions.overloading.user_func_arguments_vertex (pinned by refusals_pinned)"]
fn neglit_pair_vs() {
    mali_ref::assert_matches("deqp_constprod_neglit_pair_vs");
}

#[test]
#[ignore = "refused: random.all_features.vertex.94 (pinned by refusals_pinned)"]
fn reassoc_mixed_vs() {
    mali_ref::assert_matches("deqp_constprod_reassoc_mixed_vs");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] compiles byte-equal, and the
/// three lists cover every `deqp_constprod_*` reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_constprod_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_constprod_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_constprod_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to LOWERED or DIFFERS")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_constprod_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
