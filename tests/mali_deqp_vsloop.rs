// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsloop_*` reference (Mesa
//! 26.2.1 drm-shim, plan #402 item `ref-vsloop`,
//! `docs/notes/mali-deqp-vsloop.md`) against the Mali online compiler: the
//! vertex-shader loops of the refusal "Valhall: a loop in a vertex shader is
//! not lowered yet" (176 Fail + 20 NotSupported in the NanoPC-T6 slices
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/`) whose shapes no
//! earlier reference holds. The ones a lowering takes run ([`MATCHED`]);
//! the rest are `#[ignore]`d with their refusal. `refusals_pinned` runs now
//! and holds each refusal's exact text (the VS one: it is compiled first),
//! so a lowering must move its pin.
//!
//! Every reference is drawn once on Mesa's drm-shim (`tools/mali/deqp_ref.sh
//! --instrument shim`, `--uniform-values`). Mesa leaves no loop in any of
//! them (shaderdb "0 loops" in all three programs): the constant-bound ones
//! are unrolled and their guards fold, the `break`-last loop runs once,
//! the `continue`-only loop is deleted, the functions are inlined and fold
//! to `-in0`. The four `con_*` programs are the same words (two `yzwx`
//! rotations folded into the store order), as are the two `fn_*`.
//!
//! | shape | the dEQP case drawn | cases (8ff01055) | covers |
//! |---|---|---|---|
//! | `con_cond_break` | `loops.for_constant_iterations.conditional_break_vertex` | 24 Fail | constant bound, a guard that folds: `conditional_{body,break,continue}`, `double_continue`, `mixed_break_continue`, `unconditional_{break,continue}`, `infinite_with_conditional_break`, each `for`/`while`/`do_while` |
//! | `con_mixed` | `loops.for_constant_iterations.mixed_break_continue_vertex` | (in the 24) | the `continue` and `break` of one `if`/`else if` |
//! | `con_double_continue` | `loops.while_constant_iterations.double_continue_vertex` | (in the 24) | two `continue`s, a `while` |
//! | `con_inf_cond_break` | `loops.do_while_constant_iterations.infinite_with_conditional_break_vertex` | (in the 24) | `do {..} while (true)` left by a constant-decided `break` |
//! | `uni_inf_break_last` | `loops.do_while_uniform_iterations.infinite_with_unconditional_break_last_vertex` | 18 Fail | `infinite_with_unconditional_break_{first,last}`, every loop type and count |
//! | `uni_only_continue` | `loops.do_while_uniform_iterations.only_continue_vertex` | 9 Fail | `only_continue`, every loop type and count |
//! | `fn_after_break` | `functions.control_flow.return_after_break_vertex` | 2 Fail | a constant loop with a `break` or `continue` in a function (`return_after_{break,continue}`) |
//! | `fn_nested` | `functions.control_flow.return_in_nested_loop_vertex` | 1 Fail | `return` inside nested constant loops in a function |
//!
//! Of the other 142 cases, 139 are shapes of earlier references (`deqp_loops_vs_*`,
//! `deqp_loops2_vs_*`, `deqp_loops3_vs_*`: the kept uniform and dynamic
//! loops, the 101-pass and nested constant ones, `return.*`, `invariance.*`,
//! `vector_subscript`'s dynamic loops, three `functions.control_flow`
//! cases); `functions.control_flow.return_in_loop_if_vertex` (1 Fail) and
//! `struct.uniform.{dynamic_,}loop_nested_struct_array_vertex` (1 Fail,
//! 1 NotSupported) have no reference (the eight slots are used).
//!
//! Every source is verbatim from the `.qpa` of the HOST-ONLY measure at
//! `63c6418d` (`tools/mali/deqp-host-measure.sh --cases`, the 196 cases).
//! There, 179 cases still stop at LoopVertex, 8 dynamic nested loops at
//! UniformType, one struct case at a non-finite constant op, and 8 constant
//! nested `for`/`while` loops compile. The note has the full table.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsloop
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
    ("con_cond_break", LOOP_VERTEX),
    ("con_double_continue", LOOP_VERTEX),
    ("con_inf_cond_break", LOOP_VERTEX),
    ("con_mixed", LOOP_VERTEX),
    ("fn_after_break", LOOP_VERTEX),
    ("fn_nested", LOOP_VERTEX),
    ("uni_inf_break_last", LOOP_VERTEX),
    ("uni_only_continue", LOOP_VERTEX),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsloop_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsloop_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsloop_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// The four constant-bound shapes are one program in Mesa: whatever the
/// guard, two passes rotate `res`, folded into the store order.
#[test]
fn constant_shapes_are_one_program() {
    let words = |n: &str| {
        let r = mali_ref::Reference::load(&format!("deqp_vsloop_{n}"));
        r.json.split("\"trace\"").nth(1).unwrap().split_once('\n').unwrap().1.to_string()
    };
    let first = words("con_cond_break");
    for n in ["con_mixed", "con_double_continue", "con_inf_cond_break"] {
        assert_eq!(words(n), first, "{n}: Mesa's program is con_cond_break's");
    }
    assert_eq!(words("fn_nested"), words("fn_after_break"), "fn_nested: Mesa's program is fn_after_break's");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a constant bound with a guard that folds once unrolled), lowered by lower-vsloop"]
fn con_cond_break() {
    mali_ref::assert_matches("deqp_vsloop_con_cond_break");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a constant bound, `continue` and `break` of one if/else if), lowered by lower-vsloop"]
fn con_mixed() {
    mali_ref::assert_matches("deqp_vsloop_con_mixed");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a constant-bound `while` with two `continue`s), lowered by lower-vsloop"]
fn con_double_continue() {
    mali_ref::assert_matches("deqp_vsloop_con_double_continue");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (`do {..} while (true)` left by a constant-decided `break`), lowered by lower-vsloop"]
fn con_inf_cond_break() {
    mali_ref::assert_matches("deqp_vsloop_con_inf_cond_break");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (an infinite loop whose last statement is `break`: one pass), lowered by lower-vsloop"]
fn uni_inf_break_last() {
    mali_ref::assert_matches("deqp_vsloop_uni_inf_break_last");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a uniform-bound loop of a lone `continue`: deleted), lowered by lower-vsloop"]
fn uni_only_continue() {
    mali_ref::assert_matches("deqp_vsloop_uni_only_continue");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a constant loop with a `break` in a function), lowered by lower-vsloop"]
fn fn_after_break() {
    mali_ref::assert_matches("deqp_vsloop_fn_after_break");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (`return` in nested constant loops in a function), lowered by lower-vsloop"]
fn fn_nested() {
    mali_ref::assert_matches("deqp_vsloop_fn_nested");
}
