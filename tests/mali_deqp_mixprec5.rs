// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_mixprec5_*` reference
//! (Mesa 26.2.1 drm-shim, plan #402 item `ref-mixprec5`,
//! `docs/notes/mali-deqp-mixprec5.md`) against the Mali online compiler.
//! The references are shapes of the two refusals "a fragment shader that
//! mixes highp and mediump or lowp float values" (MIX, 168 Fail on the board)
//! and "a fragment varying load or conversion of a form no reference draws"
//! (LOAD, 50 Fail) at 8ff01055 (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-
//! 8ff01055/refusals.tsv`) that no existing `deqp_*` fragment shader draws:
//! each source is the verbatim shader of a HOST-ONLY run of
//! `tools/mali/deqp-host-measure.sh` at the tip (VK-GL-CTS 93bca01), checked
//! byte for byte against every `tools/mali/testdata/*.frag`. Drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (GPU g610, `--uniform-values`).
//!
//! ```text
//! reference         dEQP-GLES2.functional.shaders.operator.*       text  the shape
//! mulasg_vec3_float binary_operator.*_assign_*.highp_vec3_float   LOAD  `vec3 *= float` (mul_assign_result), highp varyings, into color.xyz
//! div_vec2_float    binary_operator.div.highp_vec2_float          LOAD  `vec2 / float`, highp varyings, into color.yz
//! refract_vec2      geometric.refract.highp_vec2_float            LOAD  refract(vec2, vec2, float), three highp varyings, into color.yz
//! length_vec3       geometric.length.highp_vec3                   LOAD  length of a highp vec3, into color.x
//! mod_vec2_float    common_functions.mod.highp_vec2_float         LOAD  mod(vec2, float), into color.yz
//! lt_vec3           float_compare.lessThan.highp_vec3             MIX   lessThan on a highp vec3 pair, bvec3 into color.xyz
//! ```
//!
//! Every reference is refused at the tip, so its test is `#[ignore]`d and
//! [`refusals_pinned`] (running) holds its refusal and checks that every
//! `deqp_mixprec5_*` reference on disk is listed. The lowering that lifts one
//! (plan #402 item `lower-mixprec3` or a later one) removes its `ignore` and
//! moves it from [`REFUSED`] to [`MATCHED`] in the same commit. On the branch
//! of `lower-mixprec3` (81b3ce19, not yet merged) `lt_vec3` is byte-equal,
//! `refract_vec2` meets "arithmetic other than +, -, * ..." and `length_vec3`
//! still the MIX text; the other three are as here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec5
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const MIX: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";
const LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";

/// The references byte-equal now: none.
const MATCHED: &[&str] = &[];

/// Every refused reference with its whole refusal text.
const REFUSED: &[(&str, &str)] = &[
    ("mulasg_vec3_float", LOAD),
    ("div_vec2_float", LOAD),
    ("refract_vec2", LOAD),
    ("length_vec3", LOAD),
    ("mod_vec2_float", LOAD),
    ("lt_vec3", MIX),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_mixprec5_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_mixprec5_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_mixprec5_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn mulasg_vec3_float() {
    mali_ref::assert_matches("deqp_mixprec5_mulasg_vec3_float");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn div_vec2_float() {
    mali_ref::assert_matches("deqp_mixprec5_div_vec2_float");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn refract_vec2() {
    mali_ref::assert_matches("deqp_mixprec5_refract_vec2");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn length_vec3() {
    mali_ref::assert_matches("deqp_mixprec5_length_vec3");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn mod_vec2_float() {
    mali_ref::assert_matches("deqp_mixprec5_mod_vec2_float");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16); pinned by refusals_pinned"]
fn lt_vec3() {
    mali_ref::assert_matches("deqp_mixprec5_lt_vec3");
}
