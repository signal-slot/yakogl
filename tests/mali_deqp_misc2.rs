// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_misc2_*` (plan #387, item
//! misc2-ref: the small refusal texts no other item owns; see
//! `docs/notes/mali-deqp-misc2.md`). One test per reference:
//!
//! - shape `invariant_arith` (`MaliError::Invariant`, an invariant output
//!   computed with arithmetic): `invariant_cse`, both shaders of
//!   `shaders.invariance.highp.common_subexpression_0` verbatim (its first
//!   program); `invariant_posonly`, that case's invariant `gl_Position`
//!   alone, with a constant varying and a constant colour (our text).
//!
//! The other texts of the item already have references of other items'
//! tests, or no dEQP source could be drawn here: the note says which.
//!
//! None compiles at the tip: each test is ignored with its refusal quoted;
//! the lowering removes the `ignore` and the reference's entry in
//! `REFUSALS` together. `refusals_pinned` runs now: it asserts each ignored
//! reference's exact current refusal and names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_misc2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: an invariant output (Mesa keeps its arithmetic exact, unfused)"]
fn misc2_invariant_cse() {
    mali_ref::assert_matches("deqp_misc2_invariant_cse");
}

#[test]
#[ignore = "refused: an invariant output (Mesa keeps its arithmetic exact, unfused)"]
fn misc2_invariant_posonly() {
    mali_ref::assert_matches("deqp_misc2_invariant_posonly");
}

const INVARIANT: &str = "an invariant output (Mesa keeps its arithmetic exact, unfused)";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] =
    &[("deqp_misc2_invariant_cse", INVARIANT), ("deqp_misc2_invariant_posonly", INVARIANT)];

/// The references that compile byte-equal (their tests run): none yet.
const MATCHED: &[&str] = &[];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_misc2_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_misc2_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
