// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The lit fragment shaders' built-ins on constants (plan #367
//! lower-nonfinite-const: shading blinn-phong-inf fs 994f and cel fs fea0,
//! bump fs d442, 487d and 77c5), against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_nonfin_*`, byte for byte
//! (docs/notes/mali-nonfinite-const.md):
//!
//! - a mediump `normalize`, `length`, `dot` or `distance` of constants (a
//!   literal, a global `const`) is Mesa's GLSL front end's fold at fp32,
//!   rounded once to fp16 by lower_precision: gm2_nonfin_2's
//!   `normalize(vec3(0.5, 0.5, 1.0)) * 0.5` is the FAU words 0x32883288 and
//!   0x36883688, gm2_nonfin_4's `length(LightPos)` 0x45d5;
//! - a lone product or sum of two constants outside a lane pair is folded the
//!   same way;
//! - `x / length(v)` is `x * inversesqrt(dot(v, v))` (`frcp(frcp(a))` of
//!   `lower_fsqrt`'s `frcp(frsq(a))`): gm2_nonfin_4's `FRSQ.f16` then one
//!   `FMA.v2f16`.
//!
//! gm2_nonfin_1 compiles one instruction longer than Mesa's (20 words
//! against 19: the reordering of `(v * r) * L`, nir_opt_algebraic.py:2394,
//! is lower-constprod-rest's); gm2_nonfin_3 and 5 stop at later families.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_nonfinite_const
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `normalize(vec3(0.5, 0.5, 1.0)) * 0.5 + Normal * 0.25`: the constant is
/// two FAU words of the fp32 fold rounded once, `FMA.v2f16 r0, r0^,
/// 0x34000000.h11, u2.w0` and the z lane's on `u2.w1`.
#[test]
fn gm2_nonfin_2_a_constant_normalize_matches_its_reference() {
    mali_ref::assert_matches("gm2_nonfin_2");
}

/// `length(LightPos) / length(v)` of a global `const vec4`: 0x45d5 (sqrt(34)
/// at fp32, rounded once) times `FRSQ.f16` of the run-time dot product.
#[test]
fn gm2_nonfin_4_a_constant_length_over_a_length_matches_its_reference() {
    mali_ref::assert_matches("gm2_nonfin_4");
}

/// cel fea0's shape: compiles, one instruction more than Mesa's (the
/// constant product's reordering, lower-constprod-rest).
#[test]
#[ignore = "20 words against Mesa's 19: (v * r) * L is reordered by nir_opt_algebraic.py:2394 (lower-constprod-rest)"]
fn gm2_nonfin_1_matches_its_reference() {
    mali_ref::assert_matches("gm2_nonfin_1");
}

/// gm2_nonfin_3 (blinn-phong-inf 994f's whole shape) and 5 (bump 77c5's
/// `if`-selected colour lane) are past the constants now, at the families
/// behind them: pinned, so the item that lowers them updates this file.
#[test]
fn gm2_nonfin_3_and_5_stop_at_their_next_families() {
    let pins = [
        ("gm2_nonfin_3", "a fragment shader operand of a form or in a place no reference draws"),
        ("gm2_nonfin_5", "a gl_FragColor lane other than the varying's own lane or a constant"),
    ];
    let moved: Vec<String> = pins.iter().filter_map(|(n, t)| mali_ref::refusal_change(n, t)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

const VS: &str = "attribute vec4 p; varying vec3 Normal; void main() { gl_Position = p; Normal = p.xyz; }";

/// A constant whose fold is past fp16's range (`length` of 60000.0 lanes,
/// 84852.8) has no reference: refused as a constant that is not a finite
/// normal float.
#[test]
fn a_constant_out_of_fp16_range_stays_refused() {
    let fs = "precision mediump float; varying vec3 Normal; \
              void main() { gl_FragColor = vec4(Normal * length(vec3(60000.0, 60000.0, 0.0)), 1.0); }";
    mali_ref::assert_refused(VS, fs, "not a finite normal float");
}

/// Two lanes of one half that are not a pair's (`normalize(vec3(1.0, 2.0,
/// 2.0))`, y and z both 0x3955): the pair graph would read z's product from
/// the x lane, so the shape stays refused (fsvec_nconst_mediump).
#[test]
fn a_folded_constant_with_two_equal_lanes_apart_stays_refused() {
    mali_ref::assert_refused_ref("fsvec_nconst_mediump", "arithmetic other than");
}
