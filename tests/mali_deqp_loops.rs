// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_loops_*` reference (Mesa
//! 26.2.1 words of the dEQP-GLES2 loop shapes, `docs/notes/mali-deqp-loops.md`
//! section 5) against the Mali online compiler, through `tests/mali_ref`.
//!
//! Every reference is refused at the tip, so each test is ignored with its
//! refusal and the step (lower-loops-1 or lower-loops-2) that lowers it; the
//! lowering removes the `ignore` and the shape's entry in `REFUSALS`.
//! `refusals_pinned` runs now: it asserts each reference's exact current
//! refusal (the vertex shader's when it refuses, else the fragment shader's).
//! A kept-loop reference carries the pass counter in its words, which the
//! shared compare does not edit: its lowering compares it with `CapEdit` in
//! `src/shaderc/online/mali/tests.rs` and turns its test here into a compile
//! check (note section 5).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside, lowered by lower-loops-1"]
fn fs_101() {
    mali_ref::assert_matches("deqp_loops_fs_101");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-2"]
fn fs_cond_break() {
    mali_ref::assert_matches("deqp_loops_fs_cond_break");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-2"]
fn fs_cond_continue() {
    mali_ref::assert_matches("deqp_loops_fs_cond_continue");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-2"]
fn fs_double_continue() {
    mali_ref::assert_matches("deqp_loops_fs_double_continue");
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end), lowered by lower-loops-1"]
fn fs_dowhile_if() {
    mali_ref::assert_matches("deqp_loops_fs_dowhile_if");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-1"]
fn fs_dowhile_uni() {
    mali_ref::assert_matches("deqp_loops_fs_dowhile_uni");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-1"]
fn fs_float_uni() {
    mali_ref::assert_matches("deqp_loops_fs_float_uni");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-1"]
fn fs_for_uni() {
    mali_ref::assert_matches("deqp_loops_fs_for_uni");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-2"]
fn fs_idx_dyn() {
    mali_ref::assert_matches("deqp_loops_fs_idx_dyn");
}

#[test]
#[ignore = "refused: a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls (Mesa's ladder of fp16 selects), lowered by lower-loops-2"]
fn fs_inf_break() {
    mali_ref::assert_matches("deqp_loops_fs_inf_break");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-2"]
fn fs_mixed() {
    mali_ref::assert_matches("deqp_loops_fs_mixed");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-1"]
fn fs_select() {
    mali_ref::assert_matches("deqp_loops_fs_select");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-1"]
fn fs_uncond_break() {
    mali_ref::assert_matches("deqp_loops_fs_uncond_break");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-2"]
fn fs_vec_dyn() {
    mali_ref::assert_matches("deqp_loops_fs_vec_dyn");
}

#[test]
#[ignore = "refused: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge), lowered by lower-loops-1"]
fn fs_while_uni() {
    mali_ref::assert_matches("deqp_loops_fs_while_uni");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-1"]
fn vs_dowhile_const() {
    mali_ref::assert_matches("deqp_loops_vs_dowhile_const");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-2"]
fn vs_dowhile_uni() {
    mali_ref::assert_matches("deqp_loops_vs_dowhile_uni");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-2"]
fn vs_float_uni() {
    mali_ref::assert_matches("deqp_loops_vs_float_uni");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-1"]
fn vs_for_const() {
    mali_ref::assert_matches("deqp_loops_vs_for_const");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-2"]
fn vs_for_uni() {
    mali_ref::assert_matches("deqp_loops_vs_for_uni");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-2"]
fn vs_idx_dyn() {
    mali_ref::assert_matches("deqp_loops_vs_idx_dyn");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-1"]
fn vs_idx_static() {
    mali_ref::assert_matches("deqp_loops_vs_idx_static");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-1"]
fn vs_nested_const() {
    mali_ref::assert_matches("deqp_loops_vs_nested_const");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-2"]
fn vs_select() {
    mali_ref::assert_matches("deqp_loops_vs_select");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, the unitype family's first, then lower-loops-2"]
fn vs_vector() {
    mali_ref::assert_matches("deqp_loops_vs_vector");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, lowered by lower-loops-1"]
fn vs_while_const() {
    mali_ref::assert_matches("deqp_loops_vs_while_const");
}

/// The first refusal of each reference at the tip, without the "Valhall: "
/// prefix and the " is not lowered yet" suffix.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_loops_fs_101", "a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside,"),
    ("deqp_loops_fs_cond_break", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_cond_continue", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_double_continue", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_dowhile_if", "control flow (if, a loop, a return before the end)"),
    ("deqp_loops_fs_dowhile_uni", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_float_uni", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_for_uni", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_idx_dyn", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_inf_break", "a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls (Mesa's ladder of fp16 selects)"),
    ("deqp_loops_fs_mixed", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_select", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_uncond_break", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_vec_dyn", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_fs_while_uni", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)"),
    ("deqp_loops_vs_dowhile_const", "a loop in a vertex shader"),
    ("deqp_loops_vs_dowhile_uni", "a loop in a vertex shader"),
    ("deqp_loops_vs_float_uni", "a loop in a vertex shader"),
    ("deqp_loops_vs_for_const", "a loop in a vertex shader"),
    ("deqp_loops_vs_for_uni", "a loop in a vertex shader"),
    ("deqp_loops_vs_idx_dyn", "a loop in a vertex shader"),
    ("deqp_loops_vs_idx_static", "a loop in a vertex shader"),
    ("deqp_loops_vs_nested_const", "a loop in a vertex shader"),
    ("deqp_loops_vs_select", "a loop in a vertex shader"),
    ("deqp_loops_vs_vector", "a uniform other than a float, vec2, vec3, vec4 or mat4"),
    ("deqp_loops_vs_while_const", "a loop in a vertex shader"),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!("{name} now compiles: drop its ignore and its entry in REFUSALS"),
            Err(e) => assert_eq!(
                e.strip_prefix("Valhall: ").and_then(|e| e.strip_suffix(" is not lowered yet")),
                Some(*text),
                "{name}: the refusal moved: {e}"
            ),
        }
    }
}
