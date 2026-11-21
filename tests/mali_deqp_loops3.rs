// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mesa 26.2.1 references that `docs/notes/mali-loops-lowering.md` asks
//! for before steps A0 and S8 (plan #387 item loops3-ref), drawn on Mesa's
//! drm-shim with `tools/mali/deqp_ref.sh --instrument shim`:
//!
//! * `fs_for_highp_40`: the pin at `src/shaderc/online/mali/tests.rs`
//!   (`for (int i = 0; i < 40; i++) c = c * v;`, highp) that A0 turns from a
//!   refusal into a compile. Mesa keeps the loop (it does not unroll 40
//!   iterations of a vec4 body).
//! * `vs_fn_after_seq`, `vs_fn_mixed`: the S8 shapes (a function whose
//!   loop(s) Mesa keeps in a vertex shader, one with a `return` in the loop).
//!   Their loop form is in `docs/notes/mali-deqp-loops3.md`. The sources are
//!   those of `deqp_loops2_vs_fn_*`, drawn again here.
//!
//! No reference is byte-compared yet: each is refused at the tip, and
//! [`refusals_pinned`] asserts the exact `MaliError` text. The lowering that
//! lifts one moves its entry from [`REFUSALS`] to [`MATCHED`] in the same
//! commit. A kept loop carries the pass counter the walk adds, which the
//! shared compare does not edit: compare it with `CapEdit` as
//! `tests/mali_deqp_loops.rs` says. [`compares`] holds the `#[ignore]`d
//! compares that become the gate then.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::LoopVertex`.
const LOOP_VERTEX: &str = "a loop in a vertex shader";
/// `MaliError::LoopForm`.
const LOOP_FORM: &str = "a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside,";

/// Every reference (without the `deqp_loops3_` prefix) still refused, with the
/// first refusal of its pair at the tip, without the "Valhall: " prefix and
/// the " is not lowered yet" suffix.
const REFUSALS: &[(&str, &str)] =
    &[("fs_for_highp_40", LOOP_FORM), ("vs_fn_after_seq", LOOP_VERTEX), ("vs_fn_mixed", LOOP_VERTEX)];

/// References whose pair compiles and matches Mesa's words byte for byte
/// (`mali_ref::assert_matches`). Empty until a lowering lifts a refusal.
const MATCHED: &[&str] = &[];

fn refusal(name: &str) -> Result<(), String> {
    let r = mali_ref::Reference::load(&format!("deqp_loops3_{name}"));
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
#[ignore = "refused at the tip: A0 (fs_for_highp_40) and S8 (vs_fn_*) of mali-loops-lowering.md"]
fn compares() {
    for (name, _) in REFUSALS {
        mali_ref::assert_matches(&format!("deqp_loops3_{name}"));
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
            n.strip_prefix("deqp_loops3_").and_then(|n| n.strip_suffix(".expect.json")).map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSALS.iter().map(|(n, _)| n.to_string()).chain(MATCHED.iter().map(|n| n.to_string())).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_loops3_* reference is not listed, or the other way round");
}
