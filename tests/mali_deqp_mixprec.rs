// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_mixprec_*` (plan #377, family
//! mixprec, `docs/notes/mali-deqp-mixprec.md`), one test per reference.
//! `tests/mali_ref` does the compare.
//!
//! The references in [`MATCHED`] compile byte-equal. Every reference
//! compiles now; the ones in [`DIFFERS`] compile with other words than
//! Mesa's, so their tests are `#[ignore]`d with the difference named.
//! `differences_pinned` runs now and asserts that each of them still
//! differs: a change that makes one byte-equal moves it to [`MATCHED`] and
//! removes its `#[ignore]` in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal (their tests run).
const MATCHED: &[&str] = &["cmpf", "cmpv2", "rot1", "rot3"];

/// The references that compile with other words than Mesa's: the `FMAX`
/// tree of the compare chain in another operand order (Mesa's `iand` order
/// follows which `b2f` nir_opt_algebraic rewrote first) and, for `cmpv4`
/// and `cmptex`, not rebalanced (GLSL rebalances the `return` expression of
/// `compare_vec4` before inlining).
const DIFFERS: &[&str] = &["cmpmat2", "cmptex", "cmpv3", "cmpv4"];

#[test]
fn cmpf() {
    mali_ref::assert_matches("deqp_mixprec_cmpf");
}

#[test]
#[ignore = "differs: the column maxima's order (Mesa computes column 0's FADD_IMM and FMAX before column 1's)"]
fn cmpmat2() {
    mali_ref::assert_matches("deqp_mixprec_cmpmat2");
}

#[test]
#[ignore = "differs: the texel lanes' differences are not paired, and the product chain is not rebalanced"]
fn cmptex() {
    mali_ref::assert_matches("deqp_mixprec_cmptex");
}

#[test]
fn cmpv2() {
    mali_ref::assert_matches("deqp_mixprec_cmpv2");
}

#[test]
#[ignore = "differs: the last FMAX's operand order (Mesa's `FMAX r0, r1.abs.h00, r0.h00`, z first)"]
fn cmpv3() {
    mali_ref::assert_matches("deqp_mixprec_cmpv3");
}

#[test]
#[ignore = "differs: the product chain is not rebalanced, and the FMAX operand order (Mesa's (w, z), then (zw, xy))"]
fn cmpv4() {
    mali_ref::assert_matches("deqp_mixprec_cmpv4");
}

#[test]
fn rot1() {
    mali_ref::assert_matches("deqp_mixprec_rot1");
}

#[test]
fn rot3() {
    mali_ref::assert_matches("deqp_mixprec_rot3");
}

#[test]
fn differences_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_mixprec_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = MATCHED.iter().chain(DIFFERS).copied().collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_mixprec_* reference has no test here");
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_mixprec_{name}"));
        let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}
