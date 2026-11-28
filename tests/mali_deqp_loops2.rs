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
//! [`MATCHED`] is byte-compared (plan #402 `lower-vsloop`), [`DIFFERS`]
//! pinned as compiling with other words. For the rest `refusals_pinned`
//! asserts the exact `MaliError` text of each refused one (the vertex shader's when it
//! refuses, else the fragment shader's), and `compiled_pinned` that each of
//! [`COMPILED`] (the mediump kept loops of plan #377, docs/notes/
//! mali-deqp-loops.md section 10) compiles. The lowering that
//! lifts one moves its entry in [`REFUSALS`] in the same commit and, when the
//! pair compiles, adds `mali_ref::assert_matches` for it (a kept loop carries
//! the pass counter the walk adds, which the shared compare does not edit:
//! compare it with `CapEdit` in `src/shaderc/online/mali/tests.rs`, as
//! `tests/mali_deqp_loops.rs` says). `references_listed` keeps the table and
//! the directory in step.
//!
//! [`OPEN_STEPS`] (plan #387 item loops2-refs,
//! `docs/notes/mali-loops2-lowering.md`) holds the open ones of 80 more
//! references of this prefix: one per dEQP shape of each open loop step
//! (LP1-LP8 and IDX of that note) that no reference pinned before. The 20 of
//! LP4 and LP5 left it with plan #402 `lower-vsloop` (19 in [`MATCHED`], one
//! in [`DIFFERS`]). Their sources are the dEQP
//! generators' output rebuilt line for line (each `.license` says which).
//! `open_steps_refused` asserts each one's exact first refusal at the tip;
//! `open_steps_compares` is the byte compare, `#[ignore]`d while they are
//! refused. The step that lifts one moves it out of [`OPEN_STEPS`] (into
//! [`COMPILED`], or a `mali_ref::assert_matches` / `CapEdit` compare for a
//! loop Mesa keeps) in the same commit.
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
/// `MaliError::MixedPrecision`.
const MIXED: &str = "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)";
/// `MaliError::GuardMediump`.
const GUARD_MEDIUMP: &str = "a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls (Mesa's ladder of fp16 selects)";
/// `MaliError::ConstantProduct`: the `invariance.*.loop_*` pairs once their
/// constant loop is unrolled (plan #402 `lower-vsloop`).
const CONST_PRODUCT: &str = "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)";
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
    ("fs_disc_always", "a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"),
    ("fs_disc_dyn", "a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"),
    ("fs_disc_dynamic", "a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's)"),
    ("fs_do_dyn_basic_highp_float", MIXED),
    ("fs_do_dyn_conditional_break", GUARD_MEDIUMP),
    ("fs_do_dyn_nested", "control flow (if, a loop, a return before the end)"),
    ("fs_do_dyn_select_iteration_count", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_do_uni_basic_highp_float", MIXED),
    ("fs_do_uni_conditional_break", GUARD_MEDIUMP),
    ("fs_do_uni_conditional_continue", GUARD_MEDIUMP),
    ("fs_do_uni_nested", "control flow (if, a loop, a return before the end)"),
    ("fs_for_dyn_nested", LOOP_MEDIUMP),
    ("fs_for_dyn_select_iteration_count", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_for_uni_basic_highp_float", MIXED),
    ("fs_for_uni_nested", LOOP_MEDIUMP),
    ("fs_for_uni_nested_sequence", LOOP_MEDIUMP),
    ("fs_for_uni_nested_tricky_dataflow_1", LOOP_MEDIUMP),
    ("fs_for_uni_nested_tricky_dataflow_2", LOOP_MEDIUMP),
    ("fs_for_uni_sequence", LOOP_MEDIUMP),
    ("fs_idx_mat4_dd", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_idx_tmp_float_dd", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_idx_tmp_vec4_cd", LOOP_MEDIUMP),
    ("fs_idx_uni_float", "a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform,"),
    ("fs_idx_uni_vec4", "a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform,"),
    ("fs_idx_var_vec4_sd", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_idx_vec_direct_w", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_idx_vec_static_w", "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"),
    ("fs_idx_vec_w_direct", LOOP_FORM),
    ("fs_idx_vec_w_dynsub", LOOP_FORM),
    ("fs_ret_dyn", LOOP_FORM),
    ("fs_ret_dynamic_loop_never", LOOP_FORM),
    ("fs_struct_local", LOOP_MEDIUMP),
    ("fs_wh_dyn_basic_highp_float", MIXED),
    ("fs_wh_dyn_nested", LOOP_MEDIUMP),
    ("fs_wh_uni_nested", LOOP_MEDIUMP),
    ("vs_do_dyn_conditional_body", LOOP_VERTEX),
    ("vs_do_dyn_conditional_break", LOOP_VERTEX),
    ("vs_do_dyn_nested_sequence", LOOP_VERTEX),
    ("vs_do_uni_conditional_body", LOOP_VERTEX),
    ("vs_do_uni_conditional_break", LOOP_VERTEX),
    ("vs_do_uni_conditional_continue", LOOP_VERTEX),
    ("vs_do_uni_infinite_with_conditional_break", LOOP_VERTEX),
    ("vs_do_uni_nested", LOOP_VERTEX),
    ("vs_fn_after_seq", LOOP_VERTEX),
    ("vs_fn_mixed", LOOP_VERTEX),
    ("vs_for_dyn_conditional_body", LOOP_VERTEX),
    ("vs_for_dyn_conditional_break", LOOP_VERTEX),
    ("vs_for_dyn_nested", LOOP_VERTEX),
    ("vs_for_uni_conditional_body", LOOP_VERTEX),
    ("vs_for_uni_conditional_break", LOOP_VERTEX),
    ("vs_for_uni_conditional_continue", LOOP_VERTEX),
    ("vs_for_uni_mixed_break_continue", LOOP_VERTEX),
    ("vs_for_uni_nested", LOOP_VERTEX),
    ("vs_for_uni_nested_sequence", LOOP_VERTEX),
    ("vs_for_uni_nested_tricky_dataflow_1", LOOP_VERTEX),
    ("vs_for_uni_nested_tricky_dataflow_2", LOOP_VERTEX),
    ("vs_idx_mat4_dldl", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
    ("vs_idx_tmp_vec4_dd", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
    ("vs_idx_tmp_vec4_dldl", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
    ("vs_idx_var_vec4_dd", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
    ("vs_idx_var_vec4_dldl", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
    ("vs_idx_vec_dldl", LOOP_VERTEX),
    ("vs_idx_vec_w_direct", LOOP_VERTEX),
    ("vs_idx_vec_w_dynsub", LOOP_VERTEX),
    ("vs_inv_highp_1", CONST_PRODUCT),
    ("vs_inv_lowp_loop_1", CONST_PRODUCT),
    ("vs_inv_mediump_1", CONST_PRODUCT),
    ("vs_inv_mediump_2", CONST_PRODUCT),
    ("vs_ret_dyn", LOOP_VERTEX),
    ("vs_ret_dynamic_loop_never", LOOP_VERTEX),
    ("vs_ret_static_dyn", LOOP_VERTEX),
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

/// The references whose pair compiles since plan #377's mediump kept loop
/// (docs/notes/mali-deqp-loops.md section 10), each with whether its
/// fragment words are Mesa's with the loop cap's added and its registers
/// moved (checked by the spike's word alignment; no `CapEdit` list pins them
/// yet) or differ in form (a `while (i++ < n)` counter, a loop Mesa deletes,
/// a look-up's kept branch). The `while (i++ < n)` forms are `false` since
/// the pre-RA scheduler schedules a capped block as Mesa's without the cap
/// (sched.rs `schedule_capped`, spike-loops-rest3): the counter Mesa carries
/// as `i + 1` leaves our step before the test and a phi copy, which the cap's
/// pressure used to hide. `fs_wh_con_101_iterations` compiles since the
/// counted loop past 32 passes is kept (nir_fs.rs `rotated_loop`), in the
/// same `while (i++ < 101)` form. `fs_do_uni_select_iteration_count`
/// compiles since the `ub_true ? ui_three : 0` bound is a `CSEL.u32` on the
/// bool's word (as loops' `fs_select`); it is `false`: our counter's step
/// stands before the bound's `MOV`/`CSEL`, Mesa's after.
/// `fs_do_uni_basic_mediump_float` and `fs_for_dyn_basic_mediump_float`
/// compile since the fp16 float counter is one 16-bit phi of one lane (as
/// loops' `fs_float_uni`); the second's bound reads a varying, converted
/// since a kept loop's tests are roots of the liveness `convert_inputs16`
/// reads (nir_fs.rs `mark_live`).
/// The six vertex kept loops of a float counter compile since the carried
/// mediump `res` is two 16-bit pairs in the varying program (nir.rs
/// `vs_kept_begin`), an fp16 counter one 16-bit phi; the `true` ones are
/// Mesa's words with the cap's (`CapEdit` compares in tests.rs).
/// `vs_do_dyn_basic_highp_float` is `false`: its two fp32 products in the
/// header (the step's `uf_third * one` and the bound's `uf_one * one`)
/// stand in the source's order, Mesa's the other way round. The two
/// 101-pass constant loops compile since a counted vertex loop past the
/// unroll limit is kept: `vs_do_con_101_iterations` is Mesa's words with the
/// cap's; `vs_wh_con_101_iterations` (`while (i++ < 101)`) is `false`, its
/// `i + 1` before the compare of the old `i` (Mesa's after), as the fragment
/// twin `fs_wh_con_101_iterations`.
const COMPILED: &[(&str, bool)] = &[
    ("fs_disc_texture", false),
    ("fs_do_con_select_iteration_count", true),
    ("fs_do_dyn_101_iterations", true),
    ("fs_do_uni_101_iterations", true),
    ("fs_do_uni_basic_mediump_float", true),
    ("fs_do_uni_select_iteration_count", false),
    ("fs_for_con_select_iteration_count", true),
    ("fs_for_dyn_101_iterations", true),
    ("fs_for_dyn_basic_mediump_float", true),
    ("fs_for_dyn_basic_mediump_int", true),
    ("fs_for_dyn_conditional_body", true),
    ("fs_for_dyn_conditional_break", true),
    ("fs_for_uni_101_iterations", true),
    ("fs_for_uni_compound_statement", true),
    ("fs_for_uni_conditional_body", true),
    ("fs_for_uni_empty_body", false),
    ("fs_for_uni_function_call_inout", true),
    ("fs_for_uni_single_iteration", true),
    ("fs_idx_mat4_sd", false),
    ("fs_scope_while", false),
    ("fs_wh_con_101_iterations", false),
    ("fs_wh_dyn_101_iterations", false),
    ("fs_wh_dyn_conditional_body", false),
    ("fs_wh_dyn_conditional_break", false),
    ("fs_wh_uni_101_iterations", false),
    ("fs_wh_uni_compound_statement", false),
    ("fs_wh_uni_conditional_body", false),
    ("fs_wh_uni_conditional_break", true),
    ("fs_wh_uni_conditional_continue", false),
    ("fs_wh_uni_double_continue", false),
    ("vs_do_con_101_iterations", true),
    ("vs_do_dyn_basic_highp_float", false),
    ("vs_do_uni_basic_highp_float", true),
    ("vs_do_uni_basic_mediump_float", true),
    ("vs_for_dyn_basic_mediump_float", true),
    ("vs_for_uni_basic_lowp_float", true),
    ("vs_wh_con_101_iterations", false),
    ("vs_wh_dyn_basic_highp_float", true),
];

/// The vertex loops plan #402's `lower-vsloop` (step 1, `nir.rs`
/// `vs_put_jloop`) unrolls with their jumps, byte-equal to Mesa's words:
/// the constant `do_while` nest, the inlined `return`s in a constant or
/// infinite loop, each folded once its pass is known, and the 19 vertex
/// references of steps LP4 (constant bound) and LP5 (an unconditional break
/// or only a continue) of `docs/notes/mali-loops2-lowering.md`.
///
/// With them, the mediump fragment loops that read a local array by their
/// counter under a run-time bound (`fs_idx_*`), which Mesa unrolls by the
/// array's length with a guard per pass and one pass past the end
/// (nir_fs.rs `read_trips`; docs/notes/mali-deqp-loops.md section 15). No
/// loop is left, so no cap. `fs_idx_mat4_sd` of that shape is in
/// [`COMPILED`] (`false`): its swizzled vector products (`coords.yzwx *
/// 0.5`) pair other lanes than Mesa's.
const MATCHED: &[&str] = &[
    "fs_idx_tmp_float_dldl",
    "fs_idx_tmp_vec4_sd",
    "vs_do_con_nested",
    "vs_do_con_nested_sequence",
    "vs_do_con_nested_tricky_dataflow_2",
    "vs_do_con_only_continue",
    "vs_do_con_unconditional_continue",
    "vs_do_dyn_infinite_with_unconditional_break_last",
    "vs_do_dyn_unconditional_break",
    "vs_do_uni_infinite_with_unconditional_break_first",
    "vs_fn_return_in_loop",
    "vs_for_con_conditional_body",
    "vs_for_con_conditional_continue",
    "vs_for_con_double_continue",
    "vs_for_con_infinite_with_unconditional_break_first",
    "vs_for_con_unconditional_break",
    "vs_for_dyn_only_continue",
    "vs_for_uni_infinite_with_unconditional_break_last",
    "vs_ret_infinite",
    "vs_ret_static",
    "vs_ret_static_loop_always",
    "vs_ret_static_loop_never",
    "vs_wh_con_conditional_break",
    "vs_wh_con_infinite_with_conditional_break",
    "vs_wh_con_mixed_break_continue",
    "vs_wh_dyn_infinite_with_unconditional_break_first",
    "vs_wh_uni_only_continue",
];

/// The vertex loops `lower-vsloop` takes whose words differ from Mesa's
/// only in the fp16 narrowing: `for (i = 0; i < ui_three; i++) { res =
/// res.yzwx; break; }` is one guarded pass, ours fp32 `CSEL.u32` selects,
/// Mesa's `V2F32_TO_V2F16` and `MUX.v2i16` (deqp_vsmix_dyn_unconditional_break's
/// form), and its `while (i++ < ui_three)` twin (step LP5).
const DIFFERS: &[&str] = &["vs_for_uni_unconditional_break", "vs_wh_uni_unconditional_break"];

#[test]
fn matched() {
    for name in MATCHED {
        mali_ref::assert_matches(&format!("deqp_loops2_{name}"));
    }
}

#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_loops2_{name}"));
        let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}

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
fn compiled_pinned() {
    for (name, _) in COMPILED {
        let r = mali_ref::Reference::load(&format!("deqp_loops2_{name}"));
        if let Err(e) = mali_ref::compile_pair(&r.vs, &r.fs) {
            panic!("{name}: refused again: {e}");
        }
    }
}

/// The fragment walk's control-flow refusal (`MaliError::ControlFlow`).
const CONTROL_FLOW: &str = "control flow (if, a loop, a return before the end)";

/// The references of `docs/notes/mali-loops2-lowering.md` (without the
/// `deqp_loops2_` prefix), each with its step there and the first refusal of
/// its pair at the tip, without the "Valhall: " prefix and the " is not
/// lowered yet" suffix. Mesa keeps a loop in the `LP1`
/// `*_unconditional_continue`, `LP2`, `LP3`, `LP6`, `LP7`, `LP8` and
/// `vs_idx_vec3_dynsubw_dlr` references (compare those with `CapEdit`); the
/// `LP1` `*_only_continue` and other `IDX` ones have no loop in Mesa's words
/// (plain `mali_ref::assert_matches`). Steps LP4 and LP5 (vertex loops with
/// no loop in Mesa's words) left this table for [`MATCHED`] and
/// [`DIFFERS`] with plan #402's `lower-vsloop`.
const OPEN_STEPS: &[(&str, &str, &str)] = &[
    ("fs_for_uni_only_continue", "LP1", CONTROL_FLOW),
    ("fs_for_dyn_only_continue", "LP1", CONTROL_FLOW),
    ("fs_wh_uni_only_continue", "LP1", CONTROL_FLOW),
    ("fs_wh_dyn_only_continue", "LP1", CONTROL_FLOW),
    ("fs_do_uni_only_continue", "LP1", CONTROL_FLOW),
    ("fs_do_dyn_only_continue", "LP1", CONTROL_FLOW),
    ("fs_for_uni_unconditional_continue", "LP1", CONTROL_FLOW),
    ("fs_for_dyn_unconditional_continue", "LP1", CONTROL_FLOW),
    ("fs_wh_uni_unconditional_continue", "LP1", CONTROL_FLOW),
    ("fs_wh_dyn_unconditional_continue", "LP1", CONTROL_FLOW),
    ("fs_do_uni_unconditional_continue", "LP1", CONTROL_FLOW),
    ("fs_do_dyn_unconditional_continue", "LP1", CONTROL_FLOW),
    ("fs_do_dyn_conditional_continue", "LP2", GUARD_MEDIUMP),
    ("fs_do_dyn_double_continue", "LP2", GUARD_MEDIUMP),
    ("fs_do_uni_double_continue", "LP2", GUARD_MEDIUMP),
    ("fs_do_dyn_infinite_with_conditional_break", "LP2", GUARD_MEDIUMP),
    ("fs_do_uni_infinite_with_conditional_break", "LP2", GUARD_MEDIUMP),
    ("fs_wh_dyn_infinite_with_conditional_break", "LP2", GUARD_MEDIUMP),
    ("fs_wh_uni_infinite_with_conditional_break", "LP2", GUARD_MEDIUMP),
    ("fs_do_dyn_mixed_break_continue", "LP2", CONTROL_FLOW),
    ("fs_for_dyn_sequence", "LP3", LOOP_MEDIUMP),
    ("fs_wh_uni_sequence", "LP3", LOOP_MEDIUMP),
    ("fs_do_uni_sequence", "LP3", LOOP_MEDIUMP),
    ("fs_for_dyn_nested_sequence", "LP3", LOOP_MEDIUMP),
    ("fs_wh_uni_nested_sequence", "LP3", LOOP_MEDIUMP),
    ("fs_do_uni_nested_sequence", "LP3", LOOP_MEDIUMP),
    ("fs_for_dyn_nested_tricky_dataflow_1", "LP3", LOOP_MEDIUMP),
    ("fs_wh_uni_nested_tricky_dataflow_1", "LP3", LOOP_MEDIUMP),
    ("fs_wh_uni_nested_tricky_dataflow_2", "LP3", LOOP_MEDIUMP),
    ("fs_do_uni_nested_tricky_dataflow_1", "LP3", CONTROL_FLOW),
    ("fs_do_uni_nested_tricky_dataflow_2", "LP3", CONTROL_FLOW),
    ("fs_do_dyn_nested_tricky_dataflow_2", "LP3", CONTROL_FLOW),
    ("vs_for_uni_function_call_inout", "LP6", LOOP_VERTEX),
    ("vs_wh_dyn_function_call_inout", "LP6", LOOP_VERTEX),
    ("vs_do_uni_function_call_inout", "LP6", LOOP_VERTEX),
    ("vs_for_uni_unconditional_continue", "LP6", LOOP_VERTEX),
    ("vs_do_uni_unconditional_continue", "LP6", LOOP_VERTEX),
    ("vs_wh_dyn_unconditional_continue", "LP6", LOOP_VERTEX),
    ("vs_for_dyn_sequence", "LP7", LOOP_VERTEX),
    ("vs_wh_uni_sequence", "LP7", LOOP_VERTEX),
    ("vs_do_dyn_sequence", "LP7", LOOP_VERTEX),
    ("vs_for_dyn_nested_sequence", "LP7", LOOP_VERTEX),
    ("vs_wh_uni_nested_sequence", "LP7", LOOP_VERTEX),
    ("vs_wh_dyn_nested", "LP7", LOOP_VERTEX),
    ("vs_do_dyn_nested", "LP7", LOOP_VERTEX),
    ("vs_do_uni_nested_tricky_dataflow_1", "LP7", LOOP_VERTEX),
    ("vs_wh_dyn_nested_tricky_dataflow_1", "LP7", LOOP_VERTEX),
    ("vs_for_dyn_nested_tricky_dataflow_2", "LP7", LOOP_VERTEX),
    ("vs_for_uni_double_continue", "LP8", LOOP_VERTEX),
    ("vs_wh_dyn_conditional_continue", "LP8", LOOP_VERTEX),
    ("vs_do_dyn_conditional_continue", "LP8", LOOP_VERTEX),
    ("vs_wh_uni_mixed_break_continue", "LP8", LOOP_VERTEX),
    ("vs_do_dyn_mixed_break_continue", "LP8", LOOP_VERTEX),
    ("vs_for_dyn_infinite_with_conditional_break", "LP8", LOOP_VERTEX),
    ("vs_wh_uni_infinite_with_conditional_break", "LP8", LOOP_VERTEX),
    ("vs_wh_dyn_double_continue", "LP8", LOOP_VERTEX),
    ("vs_idx_tmp_float_dynw_dlr", "IDX", LOOP_VERTEX),
    ("vs_idx_tmp_vec4_dynw_dlr", "IDX", LOOP_VERTEX),
    ("vs_idx_mat3_dynw_dlr", "IDX", LOOP_VERTEX),
    ("vs_idx_vec3_dynsubw_dlr", "IDX", LOOP_VERTEX),
];

#[test]
fn open_steps_refused() {
    for (name, step, text) in OPEN_STEPS {
        let r = mali_ref::Reference::load(&format!("deqp_loops2_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!(
                "{name} now compiles: step {step} lifted it; add its byte compare and drop its entry in OPEN_STEPS"
            ),
            Err(e) => assert_eq!(
                e.strip_prefix("Valhall: ").and_then(|e| e.strip_suffix(" is not lowered yet")),
                Some(*text),
                "{name} ({step}): the refusal moved: {e}"
            ),
        }
    }
}

/// The byte compares that become the gate once a step lifts its refusals
/// (`--ignored`). Each fails at the tip with its refusal; a loop Mesa keeps
/// also needs `CapEdit` (see [`OPEN_STEPS`]).
#[test]
#[ignore = "refused at the tip: steps LP1-LP3, LP6-LP8 and IDX of docs/notes/mali-loops2-lowering.md"]
fn open_steps_compares() {
    for (name, _, _) in OPEN_STEPS {
        mali_ref::assert_matches(&format!("deqp_loops2_{name}"));
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
    let mut listed: Vec<String> = REFUSALS
        .iter()
        .map(|(n, _)| n.to_string())
        .chain(COMPILED.iter().map(|(n, _)| n.to_string()))
        .chain(MATCHED.iter().chain(DIFFERS).map(|n| n.to_string()))
        .collect();
    listed.extend(OPEN_STEPS.iter().map(|(n, _, _)| n.to_string()));
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_loops2_* reference is not in REFUSALS, or the other way round");
}
