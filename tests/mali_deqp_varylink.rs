// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! `deqp_varylink_*` references (`tools/mali/testdata/`, purpose in
//! `docs/notes/mali-deqp-varylink.md`). A reference the compiler still
//! refuses is `#[ignore]`d with its refusal, and one that compiles to other
//! words than Mesa's with what differs; `refusals_pinned` runs now and
//! asserts each one's exact refusal or difference, so the lowering that
//! reaches it moves the pin and removes the ignore.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varylink
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
fn swz_xx() {
    mali_ref::assert_matches("deqp_varylink_swz_xx");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (the fragment shader's mediump discard, another family); pinned by refusals_pinned"]
fn discard() {
    mali_ref::assert_matches("deqp_varylink_discard");
}

#[test]
fn draw_single() {
    mali_ref::assert_matches("deqp_varylink_draw_single");
}

#[test]
fn va_multi3() {
    mali_ref::assert_matches("deqp_varylink_va_multi3");
}

#[test]
fn draw_multi() {
    mali_ref::assert_matches("deqp_varylink_draw_multi");
}

#[test]
#[ignore = "differs: Mesa computes the mediump `abs` and `* 0.5 + 0.5` at fp16, the vertex walk at fp32 (mixprec's vertex side); pinned by refusals_pinned"]
fn op_abs_vec2() {
    mali_ref::assert_matches("deqp_varylink_op_abs_vec2");
}

#[test]
#[ignore = "refused: a varying lane left unwritten (shape U, left to lower-misc); pinned by refusals_pinned"]
fn unwritten() {
    mali_ref::assert_matches("deqp_varylink_unwritten");
}

/// Shape E2's first reference (`vertex_arrays.single_attribute`'s shader,
/// docs/notes/mali-varying-constants.md): three lanes of one uniform value
/// merge into one flat lane. Drawn without `--uniform-values`.
#[test]
fn varymerge_equal() {
    mali_ref::assert_matches_unset("varymerge_equal");
}

/// glmark2 ideas' vertex shader a09e (`color = vec4(c, c, c, 1.0)`, `c`
/// written under an `if` of a uniform): the three lanes are one value of
/// uniforms, merged into one flat lane, as the E2 rule gives them. The text
/// itself (`gm2_cf_ifuni_vs`) and #367's reduced shape
/// (`gm2_varylink_dup_uniform`).
#[test]
fn gm2_ideas_equal_uniform_lanes() {
    mali_ref::assert_matches("gm2_cf_ifuni_vs");
    mali_ref::assert_matches("gm2_varylink_dup_uniform");
}

/// The refusal each ignored reference meets at the tip, and for one that
/// compiles to other words, that only its vertex program differs.
#[test]
fn refusals_pinned() {
    const DISCARD: &str = "Valhall: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) is not lowered yet";
    const VARYING: &str = "Valhall: a varying lane left unwritten is not lowered yet";
    let pins = [("discard", DISCARD), ("unwritten", VARYING)];
    for (name, text) in pins {
        mali_ref::assert_refused_ref(&format!("deqp_varylink_{name}"), text);
    }
    /* compiles since the walk computes no lane past an operand's (shape P);
     * the fragment program is Mesa's, the vertex one is fp32 where Mesa's
     * is fp16 */
    let r = mali_ref::Reference::load("deqp_varylink_op_abs_vec2");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("op_abs_vec2 compiles");
    let d = mali_ref::differences(&r, &pair);
    assert!(!d.is_empty(), "op_abs_vec2 is byte-equal now: remove its #[ignore] and this pin");
    let fs: Vec<_> = d.iter().filter(|m| m.starts_with("fs")).collect();
    assert!(fs.is_empty(), "op_abs_vec2: its fragment program differs now:\n{fs:#?}");
}
