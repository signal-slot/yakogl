// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The loop shapes of the dEQP-GLES2 tree that the Mali online compiler still
//! refuses at 9ac31f30, with their Mesa 26.2.1 references
//! `tools/mali/testdata/deqp_loops2_*` (plan #387 item loops-ref,
//! `docs/notes/mali-deqp-loops.md` section 9). Each reference is the vertex
//! and fragment shader of one dEQP case as the host measure
//! (`tools/mali/deqp-host-measure.sh`) logged them, drawn once on Mesa's
//! drm-shim with `--uniform-values`.
//!
//! No reference is byte-compared yet: every one is refused, and
//! `refusals_pinned` asserts the exact `MaliError` text of each (the vertex
//! shader's when it refuses, else the fragment shader's). The lowering that
//! lifts one moves its entry in [`REFUSALS`] in the same commit and, when the
//! pair compiles, adds `mali_ref::assert_matches` for it (a kept loop carries
//! the pass counter the walk adds, which the shared compare does not edit:
//! compare it with `CapEdit` in `src/shaderc/online/mali/tests.rs`, as
//! `tests/mali_deqp_loops.rs` says). `references_listed` keeps the table and
//! the directory in step.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::LoopVertex`.
const LOOP_VERTEX: &str = "a loop in a vertex shader";
/// `MaliError::LoopMediump`.
const LOOP_MEDIUMP: &str =
    "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)";
/// `MaliError::GuardMediump`.
const GUARD_MEDIUMP: &str = "a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls (Mesa's ladder of fp16 selects)";
/// `MaliError::LoopForm`.
const LOOP_FORM: &str = "a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside,";

/// Every reference (without the `deqp_loops2_` prefix) with the first
/// refusal of its pair at the tip, without the "Valhall: " prefix and the
/// " is not lowered yet" suffix. The prefix of a name is the stage of the
/// shader with the loop (`fs`, `vs`), the loop type (`for`, `wh`, `do`), the
/// bound (`uni` an int uniform, `con` a literal, `dyn` `one * ui_n`) and the
/// dEQP body; the `idx`, `ret`, `fn`, `inv` and `disc` names are the other
/// groups' own.
const REFUSALS: &[(&str, &str)] = &[
    ("fs_disc_always", LOOP_MEDIUMP),
    ("fs_disc_dyn", LOOP_MEDIUMP),
    ("fs_disc_dynamic", LOOP_MEDIUMP),
    ("fs_disc_texture", LOOP_MEDIUMP),
    ("fs_do_con_select_iteration_count", LOOP_MEDIUMP),
    ("fs_do_dyn_101_iterations", LOOP_MEDIUMP),
    ("fs_do_dyn_basic_highp_float", LOOP_MEDIUMP),
    ("fs_do_dyn_conditional_break", GUARD_MEDIUMP),
    ("fs_do_dyn_nested", LOOP_MEDIUMP),
    ("fs_do_dyn_select_iteration_count", LOOP_MEDIUMP),
    ("fs_do_uni_101_iterations", LOOP_MEDIUMP),
    ("fs_do_uni_basic_highp_float", LOOP_MEDIUMP),
    ("fs_do_uni_basic_mediump_float", LOOP_MEDIUMP),
    ("fs_do_uni_conditional_break", GUARD_MEDIUMP),
    ("fs_do_uni_conditional_continue", GUARD_MEDIUMP),
    ("fs_do_uni_nested", LOOP_MEDIUMP),
    ("fs_do_uni_select_iteration_count", LOOP_MEDIUMP),
    ("fs_for_con_select_iteration_count", LOOP_MEDIUMP),
    ("fs_for_dyn_101_iterations", LOOP_MEDIUMP),
    ("fs_for_dyn_basic_mediump_float", LOOP_MEDIUMP),
    ("fs_for_dyn_basic_mediump_int", LOOP_MEDIUMP),
    ("fs_for_dyn_conditional_body", LOOP_MEDIUMP),
    ("fs_for_dyn_conditional_break", LOOP_MEDIUMP),
    ("fs_for_dyn_nested", LOOP_MEDIUMP),
    ("fs_for_dyn_select_iteration_count", LOOP_MEDIUMP),
    ("fs_for_uni_101_iterations", LOOP_MEDIUMP),
    ("fs_for_uni_basic_highp_float", LOOP_MEDIUMP),
    ("fs_for_uni_compound_statement", LOOP_MEDIUMP),
    ("fs_for_uni_conditional_body", LOOP_MEDIUMP),
    ("fs_for_uni_empty_body", LOOP_MEDIUMP),
    ("fs_for_uni_function_call_inout", LOOP_MEDIUMP),
    ("fs_for_uni_nested", LOOP_MEDIUMP),
    ("fs_for_uni_nested_sequence", LOOP_MEDIUMP),
    ("fs_for_uni_nested_tricky_dataflow_1", LOOP_MEDIUMP),
    ("fs_for_uni_nested_tricky_dataflow_2", LOOP_MEDIUMP),
    ("fs_for_uni_sequence", LOOP_MEDIUMP),
    ("fs_for_uni_single_iteration", LOOP_MEDIUMP),
    ("fs_idx_mat4_dd", LOOP_MEDIUMP),
    ("fs_idx_mat4_sd", LOOP_MEDIUMP),
    ("fs_idx_tmp_float_dd", LOOP_MEDIUMP),
    ("fs_idx_tmp_float_dldl", LOOP_MEDIUMP),
    ("fs_idx_tmp_vec4_cd", LOOP_MEDIUMP),
    ("fs_idx_tmp_vec4_sd", LOOP_MEDIUMP),
    ("fs_idx_uni_float", LOOP_MEDIUMP),
    ("fs_idx_uni_vec4", LOOP_MEDIUMP),
    ("fs_idx_var_vec4_sd", LOOP_MEDIUMP),
    ("fs_idx_vec_direct_w", LOOP_MEDIUMP),
    ("fs_idx_vec_static_w", LOOP_MEDIUMP),
    ("fs_idx_vec_w_direct", LOOP_MEDIUMP),
    ("fs_idx_vec_w_dynsub", LOOP_MEDIUMP),
    ("fs_ret_dyn", LOOP_MEDIUMP),
    ("fs_ret_dynamic_loop_never", LOOP_MEDIUMP),
    ("fs_scope_while", LOOP_MEDIUMP),
    ("fs_struct_local", LOOP_MEDIUMP),
    ("fs_wh_con_101_iterations", LOOP_FORM),
    ("fs_wh_dyn_101_iterations", LOOP_MEDIUMP),
    ("fs_wh_dyn_basic_highp_float", LOOP_MEDIUMP),
    ("fs_wh_dyn_conditional_body", LOOP_MEDIUMP),
    ("fs_wh_dyn_conditional_break", LOOP_MEDIUMP),
    ("fs_wh_dyn_nested", LOOP_MEDIUMP),
    ("fs_wh_uni_101_iterations", LOOP_MEDIUMP),
    ("fs_wh_uni_compound_statement", LOOP_MEDIUMP),
    ("fs_wh_uni_conditional_body", LOOP_MEDIUMP),
    ("fs_wh_uni_conditional_break", LOOP_MEDIUMP),
    ("fs_wh_uni_conditional_continue", LOOP_MEDIUMP),
    ("fs_wh_uni_double_continue", LOOP_MEDIUMP),
    ("fs_wh_uni_nested", LOOP_MEDIUMP),
    ("vs_do_con_101_iterations", LOOP_VERTEX),
    ("vs_do_con_nested", LOOP_VERTEX),
    ("vs_do_dyn_basic_highp_float", LOOP_VERTEX),
    ("vs_do_dyn_conditional_body", LOOP_VERTEX),
    ("vs_do_dyn_conditional_break", LOOP_VERTEX),
    ("vs_do_dyn_nested_sequence", LOOP_VERTEX),
    ("vs_do_uni_basic_highp_float", LOOP_VERTEX),
    ("vs_do_uni_basic_mediump_float", LOOP_VERTEX),
    ("vs_do_uni_conditional_body", LOOP_VERTEX),
    ("vs_do_uni_conditional_break", LOOP_VERTEX),
    ("vs_do_uni_conditional_continue", LOOP_VERTEX),
    ("vs_do_uni_infinite_with_conditional_break", LOOP_VERTEX),
    ("vs_do_uni_nested", LOOP_VERTEX),
    ("vs_fn_after_seq", LOOP_VERTEX),
    ("vs_fn_mixed", LOOP_VERTEX),
    ("vs_fn_return_in_loop", LOOP_VERTEX),
    ("vs_for_dyn_basic_mediump_float", LOOP_VERTEX),
    ("vs_for_dyn_conditional_body", LOOP_VERTEX),
    ("vs_for_dyn_conditional_break", LOOP_VERTEX),
    ("vs_for_dyn_nested", LOOP_VERTEX),
    ("vs_for_uni_basic_lowp_float", LOOP_VERTEX),
    ("vs_for_uni_conditional_body", LOOP_VERTEX),
    ("vs_for_uni_conditional_break", LOOP_VERTEX),
    ("vs_for_uni_conditional_continue", LOOP_VERTEX),
    ("vs_for_uni_mixed_break_continue", LOOP_VERTEX),
    ("vs_for_uni_nested", LOOP_VERTEX),
    ("vs_for_uni_nested_sequence", LOOP_VERTEX),
    ("vs_for_uni_nested_tricky_dataflow_1", LOOP_VERTEX),
    ("vs_for_uni_nested_tricky_dataflow_2", LOOP_VERTEX),
    ("vs_for_uni_unconditional_break", LOOP_VERTEX),
    ("vs_idx_mat4_dldl", LOOP_VERTEX),
    ("vs_idx_tmp_vec4_dd", LOOP_VERTEX),
    ("vs_idx_tmp_vec4_dldl", LOOP_VERTEX),
    ("vs_idx_var_vec4_dd", LOOP_VERTEX),
    ("vs_idx_var_vec4_dldl", LOOP_VERTEX),
    ("vs_idx_vec_dldl", LOOP_VERTEX),
    ("vs_idx_vec_w_direct", LOOP_VERTEX),
    ("vs_idx_vec_w_dynsub", LOOP_VERTEX),
    ("vs_inv_highp_1", LOOP_VERTEX),
    ("vs_inv_lowp_loop_1", LOOP_VERTEX),
    ("vs_inv_mediump_1", LOOP_VERTEX),
    ("vs_inv_mediump_2", LOOP_VERTEX),
    ("vs_ret_dyn", LOOP_VERTEX),
    ("vs_ret_dynamic_loop_never", LOOP_VERTEX),
    ("vs_ret_infinite", LOOP_VERTEX),
    ("vs_ret_static", LOOP_VERTEX),
    ("vs_ret_static_dyn", LOOP_VERTEX),
    ("vs_ret_static_loop_always", LOOP_VERTEX),
    ("vs_ret_static_loop_never", LOOP_VERTEX),
    ("vs_wh_con_101_iterations", LOOP_VERTEX),
    ("vs_wh_dyn_basic_highp_float", LOOP_VERTEX),
    ("vs_wh_dyn_conditional_body", LOOP_VERTEX),
    ("vs_wh_dyn_conditional_break", LOOP_VERTEX),
    ("vs_wh_dyn_nested_sequence", LOOP_VERTEX),
    ("vs_wh_uni_conditional_body", LOOP_VERTEX),
    ("vs_wh_uni_conditional_break", LOOP_VERTEX),
    ("vs_wh_uni_conditional_continue", LOOP_VERTEX),
    ("vs_wh_uni_double_continue", LOOP_VERTEX),
    ("vs_wh_uni_nested", LOOP_VERTEX),
    ("vs_wh_uni_unconditional_continue", LOOP_VERTEX),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        let r = mali_ref::Reference::load(&format!("deqp_loops2_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!("{name} now compiles: add its byte compare and drop its entry in REFUSALS"),
            Err(e) => assert_eq!(
                e.strip_prefix("Valhall: ").and_then(|e| e.strip_suffix(" is not lowered yet")),
                Some(*text),
                "{name}: the refusal moved: {e}"
            ),
        }
    }
}

#[test]
fn references_listed() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("deqp_loops2_").and_then(|n| n.strip_suffix(".expect.json")).map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = REFUSALS.iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_loops2_* reference is not in REFUSALS, or the other way round");
}
