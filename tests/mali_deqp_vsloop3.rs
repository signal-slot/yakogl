// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsloop3_*` reference (Mesa
//! 26.2.1 drm-shim, plan #402 item `ref-vsloop3`,
//! `docs/notes/mali-deqp-vsloop3.md`) against the Mali online compiler: the
//! two vertex-shader loop shapes of the refusal "Valhall: a loop in a vertex
//! shader is not lowered yet" that neither the unrolling design
//! (`docs/notes/mali-deqp-vsloop.md`) nor the kept-loop design
//! (`docs/notes/mali-deqp-vsloop2.md`) holds: `sequence` (two kept loops one
//! after the other on one counter) and `function_call_inout` (one kept loop
//! whose body is an inlined `inout` call). The ones a lowering takes run
//! ([`MATCHED`]); the rest are `#[ignore]`d with their refusal.
//! `refusals_pinned` runs now and holds each refusal's exact text, so a
//! lowering must move its pin.
//!
//! Neither shape is in the 196 VS loop cases of
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`: at
//! `8ff01055` all 12 cases (`loops.{for,while,do_while}_{uniform,dynamic}_iterations.{sequence,function_call_inout}_vertex`,
//! 12 Fail on the board; the `constant` twins pass) stopped earlier at a
//! vertex uniform refusal ("a uniform other than a float, ..." for the
//! dynamic ones, "a vertex shader uniform that is not highp ..." for the
//! uniform ones). Since `lower-vsunif` they reach the loop walk: HOST-ONLY
//! at `c28470db`, each of the 12 programs gives the VS loop refusal (the
//! six `function_call_return` twins compile). Six of the 12 already have
//! references in `tests/mali_deqp_loops2.rs` (LP6 and LP7 of
//! `docs/notes/mali-loops2-lowering.md`); the six here are the other loop
//! type and bound combinations, so the 12 cases are all drawn.
//!
//! | shape | references here | earlier references | cases | Mesa (varying program) |
//! |---|---|---|---:|---|
//! | `sequence` | `for_uni_sequence`, `wh_dyn_sequence`, `do_uni_sequence` | `deqp_loops2_vs_{for_dyn,wh_uni,do_dyn}_sequence` | 6 Fail | **kept**, 2 loops in sequence, 32-38 instrs |
//! | `function_call_inout` | `for_dyn_function_call_inout`, `wh_uni_function_call_inout`, `do_dyn_function_call_inout` | `deqp_loops2_vs_{for_uni,wh_dyn,do_uni}_function_call_inout` | 6 Fail | `func` **inlined**, 1 loop **kept**, 24-32 instrs |
//!
//! Each source is rebuilt from `es2fShaderLoopTests.cpp`'s
//! `createSpecialLoopCase` (VK-GL-CTS 05ca62d); the same rebuild gives the
//! six `deqp_loops2_vs_*` sources above byte for byte (each `.license`
//! sibling names its case).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsloop3
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
    ("do_dyn_function_call_inout", LOOP_VERTEX),
    ("do_uni_sequence", LOOP_VERTEX),
    ("for_dyn_function_call_inout", LOOP_VERTEX),
    ("for_uni_sequence", LOOP_VERTEX),
    ("wh_dyn_sequence", LOOP_VERTEX),
    ("wh_uni_function_call_inout", LOOP_VERTEX),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsloop3_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsloop3_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsloop3_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a loop in a vertex shader (two uniform-bound `for` loops in sequence on one counter), the sequence lowering of mali-deqp-vsloop3.md"]
fn for_uni_sequence() {
    mali_ref::assert_matches("deqp_vsloop3_for_uni_sequence");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (two divergent `while (i++ < ...)` loops in sequence on one counter), the sequence lowering of mali-deqp-vsloop3.md"]
fn wh_dyn_sequence() {
    mali_ref::assert_matches("deqp_vsloop3_wh_dyn_sequence");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (two uniform-bound `do_while` loops in sequence on one counter), the sequence lowering of mali-deqp-vsloop3.md"]
fn do_uni_sequence() {
    mali_ref::assert_matches("deqp_vsloop3_do_uni_sequence");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `for` whose body is an inlined `inout` call), the inout lowering of mali-deqp-vsloop3.md"]
fn for_dyn_function_call_inout() {
    mali_ref::assert_matches("deqp_vsloop3_for_dyn_function_call_inout");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a uniform-bound `while` whose body is an inlined `inout` call), the inout lowering of mali-deqp-vsloop3.md"]
fn wh_uni_function_call_inout() {
    mali_ref::assert_matches("deqp_vsloop3_wh_uni_function_call_inout");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (a divergent `do_while` whose body is an inlined `inout` call), the inout lowering of mali-deqp-vsloop3.md"]
fn do_dyn_function_call_inout() {
    mali_ref::assert_matches("deqp_vsloop3_do_dyn_function_call_inout");
}
