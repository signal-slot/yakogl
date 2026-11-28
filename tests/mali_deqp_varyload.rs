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
//! eight shapes above are the cap. Since plan #402's `lower-mixprec3`,
//! `rnd57`, `rnd31`, `basic33`, `cond19` and `tex110` are byte-equal
//! ([`MATCHED`]); `cond36` and `basic8` compile with other words than
//! Mesa's ([`DIFFERS`], held by [`differences_pinned`]), `swz41` is refused.
//! A refused or differing test is `#[ignore]`d and [`refusals_pinned`]
//! (running) holds its refusal and checks that every `deqp_varyload_*`
//! reference on disk is listed. The lowering that lifts one removes its
//! `ignore` and moves it to [`MATCHED`] in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varyload
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";

/// The references byte-equal now, their tests running (plan #402's
/// lower-mixprec3): `rnd57`, `-0.0 * b` folded to `-0.0` before the link
/// (nir_fs.rs `walk_nodes`); `rnd31`, the pair (b, a) converted with its
/// odd lane low (`Conversions::swap` in nir_fs.rs); `basic33`, a slot of
/// two variables converted at their first reader and the dead colour
/// stores dropped (mod.rs `overwritten_stores`); `cond19` and `tex110`,
/// overwritten `gl_FragColor` stores dropped, so their varyings are none.
const MATCHED: &[&str] = &["rnd57", "rnd31", "basic33", "cond19", "tex110"];

/// The references that compile with other words than Mesa's: `cond36`
/// (Mesa's product at fp32 of lanes the front end folded to constants,
/// FAU words `ba00ba00, 00008000` against none) and `basic8` (Mesa splits
/// the z, w pair and fuses each lane alone: 18 fragment words, 19 Mesa's).
const DIFFERS: &[&str] = &["cond36", "basic8"];

/// Every refused reference with its whole refusal text.
const REFUSED: &[(&str, &str)] = &[
    /* the vertex shader's store of `b`, which only an unselected lane of
     * `vec3(c, b, a).b` reads (proto-varyload) */
    ("swz41", LOAD),
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
    let mut listed: Vec<String> = REFUSED
        .iter()
        .map(|(n, _)| *n)
        .chain(MATCHED.iter().copied())
        .chain(DIFFERS.iter().copied())
        .map(str::to_string)
        .collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_varyload_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_varyload_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// Each [`DIFFERS`] reference compiles, with other words than Mesa's.
#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_varyload_{name}"));
        let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}

#[test]
fn rnd57() {
    mali_ref::assert_matches("deqp_varyload_rnd57");
}

#[test]
fn rnd31() {
    mali_ref::assert_matches("deqp_varyload_rnd31");
}

#[test]
#[ignore = "differs from its reference; pinned by differences_pinned"]
fn cond36() {
    mali_ref::assert_matches("deqp_varyload_cond36");
}

#[test]
fn basic33() {
    mali_ref::assert_matches("deqp_varyload_basic33");
}

#[test]
fn cond19() {
    mali_ref::assert_matches("deqp_varyload_cond19");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn swz41() {
    mali_ref::assert_matches("deqp_varyload_swz41");
}

#[test]
#[ignore = "differs from its reference; pinned by differences_pinned"]
fn basic8() {
    mali_ref::assert_matches("deqp_varyload_basic8");
}

#[test]
fn tex110() {
    mali_ref::assert_matches("deqp_varyload_tex110");
}
