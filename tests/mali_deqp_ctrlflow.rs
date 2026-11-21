// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_ctrlflow_*` (plan #387, item
//! ctrlflow-ref): the dEQP-GLES2 cases whose first refusal at the
//! integration tip `5cb80624` is `MaliError::ControlFlow` ("control flow
//! (if, a loop, a return before the end)", 73 cases) or
//! `MaliError::DiscardForm` ("a discard other than a highp fragment
//! shader's `if (a < b) discard;`", 16 cases); see
//! `docs/notes/mali-deqp-ctrlflow.md`. One test per reference, the name
//! without `deqp_ctrlflow_`, then the dEQP case (under
//! `dEQP-GLES2.functional.`) it is verbatim from:
//!
//! - `disc_basic_always`: `shaders.discard.basic_always`
//! - `disc_basic_dynamic`: `shaders.discard.basic_dynamic`
//! - `disc_basic_never`: `shaders.discard.basic_never`
//! - `disc_basic_uniform`: `shaders.discard.basic_uniform`
//! - `disc_function_dynamic`: `shaders.discard.function_dynamic`
//! - `disc_function_static_loop_always`: `shaders.discard.function_static_loop_always`
//! - `disc_static_loop_uniform`: `shaders.discard.static_loop_uniform`
//! - `fs_float_cond_colour_read`: `fragment_ops.interaction.basic_shader.24`
//! - `fs_float_cond_varying`: `shaders.random.conditionals.fragment.51`
//! - `fs_frontfacing`: `shaders.builtin_variable.frontfacing`
//! - `fs_int_cond`: `fragment_ops.interaction.basic_shader.69`
//! - `fs_int_cond_colour`: `shaders.random.conditionals.fragment.79`
//! - `fs_loop_break_continue`: `shaders.loops.do_while_uniform_iterations.mixed_break_continue_fragment`
//! - `fs_return_static_loop`: `shaders.return.return_in_static_loop_dynamic_fragment`
//! - `fs_shaderlib_and`: `shaders.conditionals.if.compound_statement_vertex`
//! - `fs_side_effect_or`: `shaders.operator.sequence.side_effects.affect_or`
//! - `fs_side_effect_ternary`: `shaders.operator.sequence.side_effects.affect_ternary`
//! - `fs_struct_select`: `shaders.struct.local.conditional_assignment_fragment`
//! - `fs_vec_cond`: `shaders.random.conditionals.fragment.28`
//! - `vs_elseif`: `shaders.conditionals.if.if_elseif_vertex`
//! - `vs_nested`: `shaders.random.conditionals.vertex.10`
//! - `vs_nested_combined`: `shaders.random.conditionals.combined.41`
//! - `vs_nested_hsl`: `shaders.algorithm.hsl_to_rgb_vertex`
//! - `vs_statement_side`: `shaders.random.conditionals.vertex.90`
//! - `vs_varying_else_side`: `shaders.random.conditionals.vertex.59`
//! - `vs_varying_one_side`: `shaders.linkage.varying_5`
//! - `vs_varying_select`: `shaders.random.conditionals.vertex.1`
//!
//! Two shapes already have a reference elsewhere and are not drawn again:
//! `loops.do_while_uniform_iterations.conditional_body_fragment` is
//! `deqp_loops_fs_dowhile_if` (`tests/mali_deqp_loops.rs`, the same pair and
//! the same words), and `deqp_varylink_discard` (`tests/mali_deqp_varylink.rs`)
//! has the words of `disc_basic_dynamic`.
//!
//! The pairs are verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` (VK-GL-CTS 93bca018), drawn once on
//! Mesa's drm-shim with `--uniform-values` (`tools/mali/deqp_ref.sh
//! --instrument shim`).
//!
//! None compiles at the tip: each test is ignored with its refusal quoted;
//! the lowering removes the `ignore` and the reference's entry in
//! [`REFUSALS`] together, and lists it in [`MATCHED`]. `refusals_pinned`
//! runs now: it asserts each ignored reference's exact current refusal and
//! names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_ctrlflow
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_basic_always() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_basic_always");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_basic_dynamic() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_basic_dynamic");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_basic_never() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_basic_never");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_basic_uniform() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_basic_uniform");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_function_dynamic() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_function_dynamic");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_function_static_loop_always() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_function_static_loop_always");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"]
fn ctrlflow_disc_static_loop_uniform() {
    mali_ref::assert_matches("deqp_ctrlflow_disc_static_loop_uniform");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_float_cond_colour_read() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_float_cond_colour_read");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_float_cond_varying() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_float_cond_varying");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_frontfacing() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_frontfacing");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_int_cond() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_int_cond");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_int_cond_colour() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_int_cond_colour");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_loop_break_continue() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_loop_break_continue");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_return_static_loop() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_return_static_loop");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_shaderlib_and() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_shaderlib_and");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_side_effect_or() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_side_effect_or");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_side_effect_ternary() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_side_effect_ternary");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_struct_select() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_struct_select");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_fs_vec_cond() {
    mali_ref::assert_matches("deqp_ctrlflow_fs_vec_cond");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_elseif() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_elseif");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_nested() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_nested");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_nested_combined() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_nested_combined");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_nested_hsl() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_nested_hsl");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_statement_side() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_statement_side");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_varying_else_side() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_varying_else_side");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_varying_one_side() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_varying_one_side");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end)"]
fn ctrlflow_vs_varying_select() {
    mali_ref::assert_matches("deqp_ctrlflow_vs_varying_select");
}

/// `MaliError::ControlFlow`.
const CONTROL_FLOW: &str = "control flow (if, a loop, a return before the end) is not lowered yet";
/// `MaliError::DiscardForm`.
const DISCARD_FORM: &str = "a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) is not lowered yet";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_ctrlflow_disc_basic_always", DISCARD_FORM),
    ("deqp_ctrlflow_disc_basic_dynamic", DISCARD_FORM),
    ("deqp_ctrlflow_disc_basic_never", DISCARD_FORM),
    ("deqp_ctrlflow_disc_basic_uniform", DISCARD_FORM),
    ("deqp_ctrlflow_disc_function_dynamic", DISCARD_FORM),
    ("deqp_ctrlflow_disc_function_static_loop_always", DISCARD_FORM),
    ("deqp_ctrlflow_disc_static_loop_uniform", DISCARD_FORM),
    ("deqp_ctrlflow_fs_float_cond_colour_read", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_float_cond_varying", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_frontfacing", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_int_cond", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_int_cond_colour", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_loop_break_continue", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_return_static_loop", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_shaderlib_and", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_side_effect_or", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_side_effect_ternary", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_struct_select", CONTROL_FLOW),
    ("deqp_ctrlflow_fs_vec_cond", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_elseif", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_nested", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_nested_combined", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_nested_hsl", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_statement_side", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_varying_else_side", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_varying_one_side", CONTROL_FLOW),
    ("deqp_ctrlflow_vs_varying_select", CONTROL_FLOW),
];

/// The references that compile byte-equal (their tests run): none yet.
const MATCHED: &[&str] = &[];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_ctrlflow_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_ctrlflow_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
