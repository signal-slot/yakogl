// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_mixprec_*` (plan #377, family
//! mixprec, `docs/notes/mali-deqp-mixprec.md`), one test per reference.
//! `tests/mali_ref` does the compare.
//!
//! All eight references are refused at the tip, so each test is `#[ignore]`d
//! until the lowering named in its reason lifts it. `refusals_pinned` runs
//! now and asserts the exact refusal of each ignored reference (the VS's when
//! the VS refuses, otherwise the FS's, without the `Valhall: ` prefix and the
//! ` is not lowered yet` suffix): a lowering that lifts one must remove its
//! `#[ignore]` and its pin in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// Every ignored reference below, without the `deqp_mixprec_` prefix, with
/// the refusal it gets at the tip.
const PINNED: &[(&str, &str)] = &[
    ("cmpf", "a gl_FragColor lane other than the varying's own lane or a constant"),
    ("cmpmat2", "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)"),
    ("cmptex", "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)"),
    ("cmpv2", "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)"),
    ("cmpv3", "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)"),
    ("cmpv4", "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)"),
    ("rot1", "a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order)"),
    ("rot3", "a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order)"),
];

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant, lowered by plan #367's FRAGCOLOR_LANE lowering, then lower-mixprec"]
fn cmpf() {
    mali_ref::assert_matches("deqp_mixprec_cmpf");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16), lowered by lower-mixprec"]
fn cmpmat2() {
    mali_ref::assert_matches("deqp_mixprec_cmpmat2");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16), lowered by lower-mixprec"]
fn cmptex() {
    mali_ref::assert_matches("deqp_mixprec_cmptex");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16), lowered by lower-mixprec"]
fn cmpv2() {
    mali_ref::assert_matches("deqp_mixprec_cmpv2");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16), lowered by lower-mixprec"]
fn cmpv3() {
    mali_ref::assert_matches("deqp_mixprec_cmpv3");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16), lowered by lower-mixprec"]
fn cmpv4() {
    mali_ref::assert_matches("deqp_mixprec_cmpv4");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order), lowered by lower-mixprec"]
fn rot1() {
    mali_ref::assert_matches("deqp_mixprec_rot1");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order), lowered by lower-mixprec"]
fn rot3() {
    mali_ref::assert_matches("deqp_mixprec_rot3");
}

#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_mixprec_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let named: Vec<&str> = PINNED.iter().map(|p| p.0).collect();
    assert_eq!(on_disk, named, "a deqp_mixprec_* reference has no test here");
    for (name, want) in PINNED {
        let r = mali_ref::Reference::load(&format!("deqp_mixprec_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!("{name}: now compiles; remove its #[ignore] and its pin"),
            Err(e) => {
                let e = e.strip_prefix("Valhall: ").unwrap_or(&e);
                let e = e.strip_suffix(" is not lowered yet").unwrap_or(e);
                assert_eq!(e, *want, "{name}: the refusal changed");
            }
        }
    }
}
