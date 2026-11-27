// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! TEX_COORD2 on the Mali (plan #367 lower-train-2, from proto-texcoord2): a
//! lone look-up at two computed lanes of a wider varying, against the Mesa
//! 26.2.1 drm-shim references `tools/mali/testdata/gm2_nbshrf_3..5`, byte
//! for byte (docs/notes/mali-tex-coords.md, section "TEX_COORD2"):
//!
//! - gm2_nbshrf_4, `texture2D(m, sc.st * 0.5 + 0.5)` of a `vec4 sc`: the
//!   load `.v4`, `V2F32_TO_V2F16 r0, r0^, r1^` of lanes 0 and 1, one
//!   `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x38000000.h11`, then
//!   `F16_TO_F32` of each half and the two moves into the staging pair;
//! - gm2_nbshrf_3 (`sc / sc.w`) and _5 (shadow's whole test) are past the
//!   fp16 divide (dbdfb25b) and stop at the colour's lanes (FRAGCOLOR_LANE);
//!   they are pinned so the item that lowers it updates this file and admits
//!   their `FMA.v2f16 r0, r0^, r1^.h00, 0x0.neg` coordinate.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_texcoord2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// A vec4 varying's first two lanes in one multiply-add of splat constants.
/// Drawn with `--uniform-values` (931d5325); its fragment program reads no
/// uniform, so the draw without the option gives the same words.
#[test]
fn gm2_nbshrf_4_matches_its_reference() {
    mali_ref::assert_matches("gm2_nbshrf_4");
}

/// _3 and _5 divide the varying by its w first: past the divide, refused at
/// the colour's lanes before their coordinate is looked at.
#[test]
fn gm2_nbshrf_3_and_5_stop_at_the_colour_lanes() {
    let lane = "a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
    let moved: Vec<String> =
        ["gm2_nbshrf_3", "gm2_nbshrf_5"].iter().filter_map(|n| mali_ref::refusal_change(n, lane)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// The neighbours of nbshrf_4 that no reference draws keep the `TexCoord`
/// refusal: a sum alone (Mesa's `FADD_IMM.v2f16`, gm2_constfold_5's form),
/// a product alone, a constant pair of two values, lanes 2 and 3, a
/// uniform, a negation, and the colour reading another lane of the load.
#[test]
fn unmeasured_varying_coordinates_stay_refused() {
    const VS: &str = "attribute vec4 p; varying vec4 v; void main() { gl_Position = p; v = p; }";
    let tc = "a texture coordinate other than a vec2 varying read whole or two constants";
    for body in [
        "texture2D(s, v.st + 0.25)",
        "texture2D(s, v.st * 0.5)",
        "texture2D(s, v.st * vec2(0.5, 0.25) + 0.5)",
        "texture2D(s, v.st * 0.5 + vec2(0.5, 0.25))",
        "texture2D(s, v.zw * 0.5 + 0.5)",
        "texture2D(s, v.st * u)",
        "texture2D(s, -v.st)",
        "texture2D(s, v.st * 0.5 + 0.5) * v.z",
    ] {
        let fs = format!(
            "precision mediump float; uniform sampler2D s; uniform vec2 u; varying vec4 v; \
             void main() {{ gl_FragColor = {body}; }}"
        );
        mali_ref::assert_refused(VS, &fs, tc);
    }
}
