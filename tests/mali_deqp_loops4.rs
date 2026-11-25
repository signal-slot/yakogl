// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mesa 26.2.1 references for the sibling dEQP-GLES2 shapes of step S8
//! of `docs/notes/mali-loops-lowering.md` (vertex kept loops with float
//! counters, `LOOP_CAP`) that had none (plan #387 item arith-lower-1). The
//! sources are the vertex shaders of `functional.shaders.loops.*_vertex` as
//! dEQP writes them (`docs/notes/mali-deqp-loops4.md`), drawn on Mesa's
//! drm-shim with `tools/mali/deqp_ref.sh --instrument shim`.
//!
//! Every pair is refused at the tip and [`refusals_pinned`] asserts the exact
//! `MaliError` text. The lowering that lifts one moves its entry from
//! [`REFUSALS`] to [`MATCHED`] in the same commit. A kept loop carries the pass
//! counter the walk adds, which the shared compare does not edit: compare it
//! with `CapEdit` as `tests/mali_deqp_loops.rs` says. [`compares`] holds the
//! `#[ignore]`d byte compares that become the gate then.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::LoopVertex`.
const LOOP_VERTEX: &str = "a loop in a vertex shader";

/// Every reference (without the `deqp_loops4_` prefix) still refused, with the
/// first refusal of its pair at the tip, without the "Valhall: " prefix and
/// the " is not lowered yet" suffix.
const REFUSALS: &[(&str, &str)] = &[
    ("vs_do_uni_basic_lowp_float", LOOP_VERTEX),
    ("vs_for_dyn_basic_highp_float", LOOP_VERTEX),
    ("vs_for_dyn_basic_lowp_float", LOOP_VERTEX),
    ("vs_for_uni_basic_mediump_float", LOOP_VERTEX),
    ("vs_wh_dyn_basic_lowp_float", LOOP_VERTEX),
    ("vs_wh_dyn_basic_mediump_float", LOOP_VERTEX),
    ("vs_wh_uni_basic_highp_float", LOOP_VERTEX),
    ("vs_wh_uni_basic_lowp_float", LOOP_VERTEX),
    ("vs_wh_uni_basic_mediump_float", LOOP_VERTEX),
];

/// References whose pair compiles and matches Mesa's words byte for byte
/// (`mali_ref::assert_matches`). Empty until a lowering lifts a refusal.
const MATCHED: &[&str] = &[];

fn refusal(name: &str) -> Result<(), String> {
    let r = mali_ref::Reference::load(&format!("deqp_loops4_{name}"));
    mali_ref::compile_pair(&r.vs, &r.fs).map(|_| ())
}

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        match refusal(name) {
            Ok(()) => panic!("{name} now compiles: add its byte compare and move it to MATCHED"),
            Err(e) => assert_eq!(
                e.strip_prefix("Valhall: ").and_then(|e| e.strip_suffix(" is not lowered yet")),
                Some(*text),
                "{name}: the refusal moved: {e}"
            ),
        }
    }
}

/// The compares that become the gate once a refusal is lifted (`--ignored`).
/// Each fails at the tip with its refusal.
#[test]
#[ignore = "refused at the tip: S8 of mali-loops-lowering.md"]
fn compares() {
    for (name, _) in REFUSALS {
        mali_ref::assert_matches(&format!("deqp_loops4_{name}"));
    }
}

#[test]
fn matched_compile() {
    for name in MATCHED {
        refusal(name).unwrap_or_else(|e| panic!("{name} is MATCHED but is refused: {e}"));
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
            n.strip_prefix("deqp_loops4_").and_then(|n| n.strip_suffix(".expect.json")).map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSALS.iter().map(|(n, _)| n.to_string()).chain(MATCHED.iter().map(|n| n.to_string())).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_loops4_* reference is not listed, or the other way round");
}
