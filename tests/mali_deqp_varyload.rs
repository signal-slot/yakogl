// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_varyload_*` reference
//! (Mesa 26.2.1 drm-shim, plan #402 item `ref-varyload`,
//! `docs/notes/mali-deqp-varyload.md`) against the Mali online compiler.
//! The references are the shapes of the 16 `shaders.random.*` cases of the
//! 50 `a fragment varying load or conversion of a form no reference draws`
//! refusals at 8ff01055 (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/
//! refusals.tsv`) that `deqp_mixprec3`, `mixcol` and `gaps` do not draw.
//! Every source is the verbatim shader of a HOST-ONLY run of
//! `tools/mali/deqp-host-measure.sh` (VK-GL-CTS 93bca01), drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (GPU g610, `--uniform-values`).
//!
//! ```text
//! reference  dEQP-GLES2.functional.shaders.*                  the shape
//! rnd57      random.basic_expression.{fragment,combined}.57   mediump varyings a, b: vec4(a, a, -0.0 * b, a)
//! rnd31      random.basic_expression.{fragment,combined}.31   vec4(b, 1.0, a, a), an unused mediump int uniform
//! cond36     random.conditionals.fragment.36                  folded int constants and two varyings, a vec4 product
//! basic33    random.basic_expression.fragment.33              six mediump float varyings in sums and products
//! cond19     random.conditionals.fragment.19                  a mediump vec3 varying with scalar ones, swizzled vec4s
//! swz41      random.swizzle.fragment.41                       two mediump vec4 varyings, one swizzled, and a float one
//! basic8     random.basic_expression.fragment.8               four varyings and mediump vec4 and float uniforms
//! tex110     random.texture.fragment.110                      varyings, int uniforms and a samplerCube read
//! ```
//!
//! Not drawn (no reference): comparison_ops.fragment.15, swizzle.fragment.{15,36},
//! conditionals.fragment.75, scalar_conversion.{fragment,combined}.65: the
//! eight shapes above are the cap. Every reference is refused, so its test is
//! `#[ignore]`d and [`refusals_pinned`] (running) holds its refusal and
//! checks that every `deqp_varyload_*` reference on disk is listed. The
//! lowering that lifts one (plan #402 item `lower-mixprec3`) removes its
//! `ignore` and moves it from [`REFUSED`] to [`MATCHED`] in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varyload
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";

/// The references byte-equal now, their tests running.
const MATCHED: &[&str] = &[];

/// Every refused reference with its whole refusal text.
const REFUSED: &[(&str, &str)] = &[
    ("rnd57", LOAD),
    ("rnd31", LOAD),
    ("cond36", LOAD),
    ("basic33", LOAD),
    ("cond19", LOAD),
    ("swz41", LOAD),
    ("basic8", LOAD),
    ("tex110", LOAD),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_varyload_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_varyload_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_varyload_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn rnd57() {
    mali_ref::assert_matches("deqp_varyload_rnd57");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn rnd31() {
    mali_ref::assert_matches("deqp_varyload_rnd31");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn cond36() {
    mali_ref::assert_matches("deqp_varyload_cond36");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn basic33() {
    mali_ref::assert_matches("deqp_varyload_basic33");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn cond19() {
    mali_ref::assert_matches("deqp_varyload_cond19");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn swz41() {
    mali_ref::assert_matches("deqp_varyload_swz41");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn basic8() {
    mali_ref::assert_matches("deqp_varyload_basic8");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn tex110() {
    mali_ref::assert_matches("deqp_varyload_tex110");
}
