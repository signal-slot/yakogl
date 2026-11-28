// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_mixprec4_*` reference
//! (Mesa 26.2.1 drm-shim, plan #402 item `ref-mixprec4`,
//! `docs/notes/mali-deqp-mixprec4.md`) against the Mali online compiler.
//! The references are comparison shapes of the mixed-precision refusal at
//! 8ff01055 (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`)
//! that `deqp_mixprec3_*`, `deqp_mixcol_*` and the other mixed-precision
//! references do not draw: a different compare operator or width than the one
//! each of those draws.
//!
//! ```text
//! shape         board cases it stands for (dEQP-GLES2.functional.shaders.operator.*)   the shape
//! gt_vec2       float_compare.greaterThan.highp_vec2_fragment                         greaterThan(highp vec2, highp vec2),
//!                                                                                     bvec2 into color.yz
//! ge_vec4       float_compare.greaterThanEqual.highp_vec4_fragment                    greaterThanEqual, bvec4 into color.xyzw
//! eq_vec4       float_compare.equal.highp_vec4_fragment                               equal on vec4, bvec4 into color.xyzw
//! le_vec2       float_compare.lessThanEqual.highp_vec2_fragment                       lessThanEqual, bvec2 into color.yz
//! bin_ge_float  binary_operator.{greater_or_equal,greater,less,less_or_equal}.         `in0 >= in1` of two highp floats,
//!               highp_float_fragment                                                  bool into color.yz
//! bin_eq_vec4   binary_operator.equal.highp_vec4_fragment                             `in0 == in1` of two highp vec4, one
//!                                                                                     bool into color.yz
//! ```
//!
//! Every reference is byte-equal since plan #402's `lower-mixprec3`
//! ([`MATCHED`]); before it `gt_vec2`, `eq_vec4`, `le_vec2`, `bin_ge_float`
//! and `bin_eq_vec4` met the MIX refusal and `ge_vec4` the IrIndex back-end
//! defect (of the fp16 compare path, which the fp32 compare no longer
//! takes). [`refusals_pinned`] checks that every `deqp_mixprec4_*`
//! reference on disk is listed; a reference that is refused again goes
//! back to [`REFUSED`] with its text and an `#[ignore]`. The fragment shaders are
//! written from the pattern of the `deqp_mixcol_hp_bool` and
//! `deqp_mixprec3_hp_bvec4` shaders (es2fShaderOperatorTests.cpp
//! ShaderOperatorCase, VK-GL-CTS 93bca01) with the operator and width
//! changed; no dEQP build was at hand to take them from a `.qpa`. Drawn with
//! `tools/mali/deqp_ref.sh --instrument shim`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references byte-equal now (plan #402's lower-mixprec3): a compare
/// of the highp varyings' island kept fp32 (`FCMP_OR.f32`, nir_fs.rs
/// `walk_nodes`), its two loads in the order of NIR's sources before
/// `nir_opt_move` (nir_fs.rs `swapped_compares`; `gt_vec2`, `le_vec2`).
const MATCHED: &[&str] = &["gt_vec2", "ge_vec4", "eq_vec4", "le_vec2", "bin_ge_float", "bin_eq_vec4"];

/// Every refused reference with its whole refusal text: none.
const REFUSED: &[(&str, &str)] = &[];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_mixprec4_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_mixprec4_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_mixprec4_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
fn gt_vec2() {
    mali_ref::assert_matches("deqp_mixprec4_gt_vec2");
}

#[test]
fn ge_vec4() {
    mali_ref::assert_matches("deqp_mixprec4_ge_vec4");
}

#[test]
fn eq_vec4() {
    mali_ref::assert_matches("deqp_mixprec4_eq_vec4");
}

#[test]
fn le_vec2() {
    mali_ref::assert_matches("deqp_mixprec4_le_vec2");
}

#[test]
fn bin_ge_float() {
    mali_ref::assert_matches("deqp_mixprec4_bin_ge_float");
}

#[test]
fn bin_eq_vec4() {
    mali_ref::assert_matches("deqp_mixprec4_bin_eq_vec4");
}
