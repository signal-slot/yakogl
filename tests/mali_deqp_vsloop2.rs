// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsloop2_*` reference (Mesa
//! 26.2.1 drm-shim, plan #402 item `ref-vsloop2`,
//! `docs/notes/mali-deqp-vsloop2.md`) against the Mali online compiler: the
//! vertex-shader loops of the refusal "Valhall: a loop in a vertex shader is
//! not lowered yet" that Mesa KEEPS as loops (step 2 of the VS loop lowering;
//! step 1, unrolling, is `tests/mali_deqp_vsloop.rs`). The ones a lowering
//! takes run ([`MATCHED`]); the rest are `#[ignore]`d with their refusal.
//! `refusals_pinned` runs now and holds each refusal's exact text (the VS
//! one: it is compiled first), so a lowering must move its pin.
//!
//! HOST-ONLY measure at `c70d4813` (`tools/mali/deqp-host-measure.sh
//! --cases`, the 196 cases of the four slices
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv` whose
//! refusal is this one): 187 still stop at the loop refusal, 1 at a
//! non-finite constant op, 8 compile. Each of the 187 cases' programs was
//! drawn on Mesa's drm-shim: in 110 cases (90 Fail + 20 NotSupported on the
//! board) Mesa keeps at least one loop (shaderdb "N loops" > 0 in the
//! varying program), in 77 it keeps none (step 1). Of the 110, 88 are shapes
//! of earlier references (`deqp_loops_vs_*`, `deqp_loops2_vs_*`,
//! `deqp_loops3_vs_*`); the 22 below are not. The note has the full table.
//!
//! Every kept loop here has a bound Mesa cannot know: `ui_N*one`, where
//! `one = int(a_one + 0.5)` is read from an attribute, so the loop is
//! divergent. The earlier references hold the uniform-bound twins of the
//! first seven shapes. Mesa's varying program keeps the loop with one exit
//! test `BRANCHZ.reconverge <cond>` forward out of the loop (at the top of
//! a `for`/`while`, after the body of a `do_while`) and an unconditional
//! back edge `BRANCHZ.eq.reconverge 0x0` (a negative offset); a nested
//! loop has one pair per loop. `continue` and `break` guards become
//! selects and ORed exit conditions, not branches. The mediump `res` is
//! carried at fp16.
//!
//! | shape | the dEQP case drawn | cases (8ff01055) | Mesa |
//! |---|---|---|---|
//! | `for_dyn_conditional_continue` | `loops.for_dynamic_iterations.conditional_continue_vertex` | 3 Fail (`for`, `while`, `do_while`) | 1 loop, varying 34 instrs |
//! | `do_dyn_double_continue` | `loops.do_while_dynamic_iterations.double_continue_vertex` | 3 Fail | 1 loop, 34 instrs |
//! | `wh_dyn_infinite_with_conditional_break` | `loops.while_dynamic_iterations.infinite_with_conditional_break_vertex` | 3 Fail | 1 loop, 29 instrs |
//! | `for_dyn_mixed_break_continue` | `loops.for_dynamic_iterations.mixed_break_continue_vertex` | 3 Fail | 1 loop, 35 instrs |
//! | `do_dyn_nested_tricky_dataflow_1` | `loops.do_while_dynamic_iterations.nested_tricky_dataflow_1_vertex` | 3 Fail | 2 loops, 37 instrs |
//! | `wh_dyn_nested_tricky_dataflow_2` | `loops.while_dynamic_iterations.nested_tricky_dataflow_2_vertex` | 3 Fail | 2 loops, 37 instrs |
//! | `for_dyn_unconditional_continue` | `loops.for_dynamic_iterations.unconditional_continue_vertex` | 3 Fail | 1 loop, 29 instrs |
//! | `struct_dyn_nested_array` | `struct.uniform.dynamic_loop_nested_struct_array_vertex` | 1 NotSupported | the outer `ui_two` loop kept; the inner `ui_three` loop unrolled to 3 guarded passes (the `T b[3]` bound), 110 instrs |
//!
//! Every source is verbatim from the `.qpa` of the measure (each `.license`
//! sibling names its program).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsloop2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::LoopVertex`.
const LOOP_VERTEX: &str = "a loop in a vertex shader";

/// The references the vertex walk makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal, without the `Valhall: `
/// prefix and the ` is not lowered yet` suffix.
const REFUSED: &[(&str, &str)] = &[
    ("do_dyn_double_continue", LOOP_VERTEX),
    ("do_dyn_nested_tricky_dataflow_1", LOOP_VERTEX),
    ("for_dyn_conditional_continue", LOOP_VERTEX),
    ("for_dyn_mixed_break_continue", LOOP_VERTEX),
    ("for_dyn_unconditional_continue", LOOP_VERTEX),
    ("struct_dyn_nested_array", LOOP_VERTEX),
    ("wh_dyn_infinite_with_conditional_break", LOOP_VERTEX),
    ("wh_dyn_nested_tricky_dataflow_2", LOOP_VERTEX),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsloop2_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsloop2_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsloop2_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `for` with `if (i == one*ui_two) continue;`), lowered by lower-vsloop2"]
fn for_dyn_conditional_continue() {
    mali_ref::assert_matches("deqp_vsloop2_for_dyn_conditional_continue");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `do_while` with two `continue`s), lowered by lower-vsloop2"]
fn do_dyn_double_continue() {
    mali_ref::assert_matches("deqp_vsloop2_do_dyn_double_continue");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `while (true)` left by a `break`), lowered by lower-vsloop2"]
fn wh_dyn_infinite_with_conditional_break() {
    mali_ref::assert_matches("deqp_vsloop2_wh_dyn_infinite_with_conditional_break");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `for` with a `continue` and a `break` of one if/else if), lowered by lower-vsloop2"]
fn for_dyn_mixed_break_continue() {
    mali_ref::assert_matches("deqp_vsloop2_for_dyn_mixed_break_continue");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (nested divergent `do_while` loops carrying a value), lowered by lower-vsloop2"]
fn do_dyn_nested_tricky_dataflow_1() {
    mali_ref::assert_matches("deqp_vsloop2_do_dyn_nested_tricky_dataflow_1");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (nested divergent `while` loops carrying a value), lowered by lower-vsloop2"]
fn wh_dyn_nested_tricky_dataflow_2() {
    mali_ref::assert_matches("deqp_vsloop2_wh_dyn_nested_tricky_dataflow_2");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `for` ending in `continue`), lowered by lower-vsloop2"]
fn for_dyn_unconditional_continue() {
    mali_ref::assert_matches("deqp_vsloop2_for_dyn_unconditional_continue");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a uniform-bound loop over a struct array uniform, its inner loop unrolled by the array bound), lowered by lower-vsloop2"]
fn struct_dyn_nested_array() {
    mali_ref::assert_matches("deqp_vsloop2_struct_dyn_nested_array");
}
