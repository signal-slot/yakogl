// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_mixprec_*` (plan #377, family
//! mixprec, `docs/notes/mali-deqp-mixprec.md`), one test per reference.
//! `tests/mali_ref` does the compare.
//!
//! Every reference compiles byte-equal ([`MATCHED`]). A reference that
//! compiles with other words than Mesa's goes in [`DIFFERS`] with its test
//! `#[ignore]`d and the difference named; `differences_pinned` asserts that
//! each of them still differs and that every reference on disk has a test,
//! so a change that makes one byte-equal moves it to [`MATCHED`] and
//! removes its `#[ignore]` in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal (their tests run).
const MATCHED: &[&str] = &["cmpf", "cmpmat2", "cmptex", "cmpv2", "cmpv3", "cmpv4", "rot1", "rot3"];

/// The references that compile with other words than Mesa's (none now).
const DIFFERS: &[&str] = &[];

#[test]
fn cmpf() {
    mali_ref::assert_matches("deqp_mixprec_cmpf");
}

#[test]
fn cmpmat2() {
    mali_ref::assert_matches("deqp_mixprec_cmpmat2");
}

#[test]
fn cmptex() {
    mali_ref::assert_matches("deqp_mixprec_cmptex");
}

#[test]
fn cmpv2() {
    mali_ref::assert_matches("deqp_mixprec_cmpv2");
}

#[test]
fn cmpv3() {
    mali_ref::assert_matches("deqp_mixprec_cmpv3");
}

#[test]
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
