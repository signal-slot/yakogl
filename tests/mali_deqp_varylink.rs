// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! `deqp_varylink_*` references (`tools/mali/testdata/`, purpose in
//! `docs/notes/mali-deqp-varylink.md`). A reference the compiler still
//! refuses is `#[ignore]`d with its refusal; `refusals_pinned` runs now and
//! asserts each one's exact refusal, so lower-varylink moves the pins and
//! removes the ignores.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varylink
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "differs: Mesa narrows the merged lane once (FADD.f32 into h0, read .h00), ours narrows it twice (V2F32_TO_V2F16 r0, r0), lowered by lower-varylink"]
fn swz_xx() {
    mali_ref::assert_matches("deqp_varylink_swz_xx");
}

#[test]
#[ignore = "refused: a discard other than a highp fragment shader's `if (a < b) discard;` with no else, lowered by lower-varylink"]
fn discard() {
    mali_ref::assert_matches("deqp_varylink_discard");
}

#[test]
#[ignore = "refused: two varying lanes of equal value (Mesa's link merges them), lowered by lower-varylink"]
fn draw_single() {
    mali_ref::assert_matches("deqp_varylink_draw_single");
}

#[test]
#[ignore = "refused: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites), lowered by lower-varylink"]
fn va_multi3() {
    mali_ref::assert_matches("deqp_varylink_va_multi3");
}

#[test]
#[ignore = "refused: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites), lowered by lower-varylink"]
fn draw_multi() {
    mali_ref::assert_matches("deqp_varylink_draw_multi");
}

#[test]
#[ignore = "refused: a gl_Position lane left unwritten, lowered by lower-varylink"]
fn op_abs_vec2() {
    mali_ref::assert_matches("deqp_varylink_op_abs_vec2");
}

#[test]
#[ignore = "refused: a varying lane left unwritten, lowered by lower-varylink"]
fn unwritten() {
    mali_ref::assert_matches("deqp_varylink_unwritten");
}

/// The refusal each ignored reference meets at the tip.
#[test]
fn refusals_pinned() {
    const EQUAL: &str = "Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet";
    const NO_ATTRIBUTE: &str = "Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet";
    const DISCARD: &str = "Valhall: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) is not lowered yet";
    const POSITION: &str = "Valhall: a gl_Position lane left unwritten is not lowered yet";
    const VARYING: &str = "Valhall: a varying lane left unwritten is not lowered yet";
    let pins = [
        ("discard", DISCARD),
        ("draw_single", EQUAL),
        ("va_multi3", NO_ATTRIBUTE),
        ("draw_multi", NO_ATTRIBUTE),
        ("op_abs_vec2", POSITION),
        ("unwritten", VARYING),
    ];
    for (name, text) in pins {
        mali_ref::assert_refused_ref(&format!("deqp_varylink_{name}"), text);
    }
}

/// `swz_xx` compiles since the integer and boolean step of plan #377
/// (ab9cde96) takes its `vec4(RES, RES, RES, 1.0)` colour, with other words
/// than Mesa's: the `xx` varying's lane, which Mesa's link merges into one,
/// is narrowed to fp16 twice instead of once.
#[test]
fn differences_pinned() {
    let r = mali_ref::Reference::load("deqp_varylink_swz_xx");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("swz_xx: refused: {e}"));
    assert!(!mali_ref::differences(&r, &pair).is_empty(), "swz_xx: now byte-equal; remove its ignore and this pin");
}
