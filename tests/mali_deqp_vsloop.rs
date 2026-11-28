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
    let mut listed: Vec<String> = REFUSED
        .iter()
        .map(|(n, _)| *n)
        .chain(MATCHED.iter().copied())
        .chain(G310_SHAPES.iter().map(|s| s.0))
        .map(str::to_string)
        .collect();
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

// Plan #387 item `vsloop-refs` (docs/notes/mali-vsloop-lowering.md): the
// shapes of the G310 vertex-loop family (179 lines of "a loop in a vertex
// shader" in the Verdin iMX95 run at e63ab28e, and 8 dynamic nested loops
// behind an int-uniform refusal there) that no committed reference held
// exactly. Their sources are dEQP's generators rebuilt (each `.license`
// says how); Mesa 26.2.1 drm-shim drew them, as every reference here.

/// `MaliError::LoopVertex`, for the tables below. (Kept apart from
/// `LOOP_VERTEX` above, which plan #402 `lower-vsloop` removes with that
/// table's last pin.)
const VS_LOOP: &str = "a loop in a vertex shader";

/// A reference's outcome at the tip.
#[derive(Clone, Copy, Debug)]
enum Tip {
    /// Refused; the refusal holds this text.
    Refused(&'static str),
    /// Refused with this text at the tip, and byte-equal on plan #402
    /// `lower-vsloop`'s result (`cbb6b45b`, step V1, not landed when these
    /// were drawn). The pin takes either until that lands; then the entry
    /// becomes `Matched`.
    RefusedOrMatched(&'static str),
    /// Compiles, and `vs.varying` is the one region that differs: `ours`
    /// words against Mesa's `mesa` (the kept loop's pass cap, a deliberate
    /// deviation compared by `CapEdit` in src/shaderc/online/mali/tests.rs,
    /// and what the note says besides).
    Compiled { ours: usize, mesa: usize },
}

/// Every new reference (without `deqp_vsloop_`), its step in the note's
/// section 4 and its outcome at the tip.
const G310_SHAPES: &[(&str, &str, Tip)] = &[
    ("do_con_conditional_body", "V1", Tip::RefusedOrMatched(VS_LOOP)),
    ("fn_return_in_loop_if", "V1", Tip::RefusedOrMatched(VS_LOOP)),
    ("for_dyn_infinite_with_unconditional_break_last", "V1", Tip::RefusedOrMatched(VS_LOOP)),
    ("ret_dynamic_loop_always", "V1", Tip::Refused(VS_LOOP)),
    ("for_con_101_iterations", "V2", Tip::Compiled { ours: 27, mesa: 24 }),
    ("do_dyn_basic_mediump_float", "V2", Tip::Compiled { ours: 31, mesa: 27 }),
    ("for_dyn_double_continue", "V3", Tip::Refused(VS_LOOP)),
    ("do_uni_double_continue", "V3", Tip::Refused(VS_LOOP)),
    ("for_uni_infinite_with_conditional_break", "V3", Tip::Refused(VS_LOOP)),
    ("do_dyn_infinite_with_conditional_break", "V3", Tip::Refused(VS_LOOP)),
    ("wh_dyn_mixed_break_continue", "V4", Tip::Refused(VS_LOOP)),
    ("do_uni_mixed_break_continue", "V4", Tip::Refused(VS_LOOP)),
    ("do_uni_nested_sequence", "V5", Tip::Refused(VS_LOOP)),
    ("wh_uni_nested_tricky_dataflow_1", "V5", Tip::Refused(VS_LOOP)),
    ("for_dyn_nested_tricky_dataflow_1", "V5", Tip::Refused(VS_LOOP)),
    ("do_dyn_nested_tricky_dataflow_2", "V5", Tip::Refused(VS_LOOP)),
];

/// New references whose Mesa words (all three programs) are an earlier
/// reference's: the loop type or the bound kind does not change Mesa's
/// program, so the lowering that takes the earlier one takes these.
const SAME_WORDS: &[(&str, &str)] = &[
    ("deqp_vsloop_do_con_conditional_body", "deqp_vsloop_con_cond_break"),
    ("deqp_vsloop_fn_return_in_loop_if", "deqp_vsloop_fn_after_break"),
    ("deqp_vsloop_for_dyn_infinite_with_unconditional_break_last", "deqp_vsloop_uni_inf_break_last"),
    ("deqp_vsloop_ret_dynamic_loop_always", "deqp_loops2_vs_ret_static_loop_always"),
    ("deqp_vsloop_for_uni_infinite_with_conditional_break", "deqp_loops2_vs_do_uni_infinite_with_conditional_break"),
    ("deqp_vsloop_do_dyn_infinite_with_conditional_break", "deqp_vsloop2_wh_dyn_infinite_with_conditional_break"),
];

/// Mesa's words of a reference: its expect.json after the `trace` line.
fn mesa_words(name: &str) -> String {
    let r = mali_ref::Reference::load(name);
    r.json.split("\"trace\"").nth(1).unwrap().split_once('\n').unwrap().1.to_string()
}

#[test]
fn g310_shapes_pinned() {
    let mut moved = Vec::new();
    for &(name, _, tip) in G310_SHAPES {
        let full = format!("deqp_vsloop_{name}");
        let r = mali_ref::Reference::load(&full);
        let got = mali_ref::compile_pair(&r.vs, &r.fs);
        match (tip, got) {
            (Tip::Refused(t), Err(e)) | (Tip::RefusedOrMatched(t), Err(e)) if e.contains(t) => {}
            (Tip::RefusedOrMatched(_), Ok(p)) if mali_ref::differences(&r, &p).is_empty() => {}
            (Tip::Compiled { ours, mesa }, Ok(p)) => {
                let d = mali_ref::differences(&r, &p);
                let want = format!("vs.varying: {ours} words ours, {mesa} reference");
                if d.len() != 1 || !d[0].starts_with(&want) {
                    moved.push(format!("{full}: expected only `{want}`, got:\n{}", d.join("\n")));
                }
            }
            (tip, Ok(_)) => moved.push(format!("{full}: now compiles (pinned {tip:?}); move its entry")),
            (tip, Err(e)) => moved.push(format!("{full}: pinned {tip:?}, refused with: {e}")),
        }
    }
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

#[test]
fn g310_same_words() {
    for (new, old) in SAME_WORDS {
        assert_eq!(mesa_words(new), mesa_words(old), "{new}: Mesa's program is {old}'s");
    }
}

/// The byte compare of every new reference: the step that takes one turns
/// its entry into a match (`Tip::Compiled` ones through `CapEdit`).
#[test]
#[ignore = "the vertex loop steps V1-V5 of docs/notes/mali-vsloop-lowering.md are not all lowered"]
fn g310_shapes_compare() {
    let failed: Vec<&str> = G310_SHAPES
        .iter()
        .filter(|(name, _, _)| {
            std::panic::catch_unwind(|| mali_ref::assert_matches(&format!("deqp_vsloop_{name}"))).is_err()
        })
        .map(|s| s.0)
        .collect();
    assert!(failed.is_empty(), "differ from Mesa: {failed:?}");
}
