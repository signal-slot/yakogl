// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! TEX_MULTI of plan #367 (lower-fma) on the Mali: fragment look-ups beside
//! another at a mediump computed coordinate (glmark2's desktop blur passes),
//! against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_texmulti_*` and `gm2_constfold_1..3`, byte for
//! byte (docs/notes/mali-tex-multi.md).
//!
//! Such a shader takes the highp route (`nir_fs::multi16`): the varying's
//! conversion at its first fp16 reader, each computed coordinate the
//! look-up's own `FADD_IMM.v2f16` of the converted pair and a constant pair,
//! `F16_TO_F32` of both halves into the staging words, the look-ups among
//! the nodes in program order, the colour's lanes paired over the look-ups'
//! words. The coordinate forms with no byte-equal lowering yet (a uniform
//! step, terrain's tilt-shift step computed from the varying) and every
//! other operation stay refused as before (TEX_MULTI).
//!
//! Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region. The
//! gm2_texmulti references were drawn with `--uniform-values`, the
//! gm2_constfold ones without.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_tex_multi
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const MULTI: &str = "a fragment shader texture look-up beside another";

/// `texture2D(v_uv) + texture2D(v_uv + vec2(0.0, 0.25))`: the first look-up
/// reads the load as it is (`TEX_SINGLE ... @r0:r1`), then the conversion
/// (`V2F32_TO_V2F16 r0, r0, r1`), the sum with `x + 0.0` kept
/// (`FADD_IMM.v2f16 r0, r0, #0x34000000`), both halves widened and the
/// second look-up; the texels summed word by word.
#[test]
fn gm2_texmulti_1_varying_beside_offset_matches_its_reference() {
    mali_ref::assert_matches("gm2_texmulti_1");
}

/// Two taps at `v_uv -+ vec2(0.25, 0.0)`, weighted: each tap its own
/// `FADD_IMM.v2f16` of the converted pair (the walk's shared `y + 0.0` is
/// not shared in NIR), the products `FMA.v2f16` of the look-ups' words.
#[test]
fn gm2_texmulti_2_two_offsets_match_their_reference() {
    mali_ref::assert_matches("gm2_texmulti_2");
}

/// A 5-tap vertical kernel at constant steps and weights, `+ 0.0` at the
/// end: the centre tap at the varying itself (`vec2(0.0, 0.0 * StepY)`
/// folds), the others each an `FADD_IMM.v2f16`, in program order.
#[test]
fn gm2_texmulti_3_five_taps_match_their_reference() {
    mali_ref::assert_matches("gm2_texmulti_3");
}

/// The separable blur's horizontal pass of glmark2's desktop scene
/// (5 taps at `vec2(k * TextureStepX, 0.0)`, desktop blur fs 68af's shape).
#[test]
fn gm2_constfold_1_blur_pass_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_constfold_1");
}

/// effect2d's Laplace kernel (9 taps, the weights 0.0 folded away, the
/// `-4.0` one a product).
#[test]
fn gm2_constfold_2_laplace_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_constfold_2");
}

/// The Laplace kernel with a `-0.0` weight: the centre tap's product
/// folds to 0.0 as Mesa's.
#[test]
fn gm2_constfold_3_negzero_weight_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_constfold_3");
}

/// The shapes with no byte-equal lowering yet stay refused: a step that is
/// a mediump uniform (gm2_texmulti_4, `FMA.v2f16` of `vec2(3u, 0)` by
/// `(-1, 1)` in Mesa) and terrain's tilt-shift, its step computed from the
/// varying (gm2_texmulti_5, _6, whose centre tap Mesa also moves); the
/// non-separable blur's `t * K1 * K1` (gm2_constfold_4) is the
/// constant-product gap, Mesa folding the two inexact constants.
#[test]
fn the_unmeasured_forms_stay_refused() {
    let pins = [
        ("gm2_texmulti_4", MULTI),
        ("gm2_texmulti_5", MULTI),
        ("gm2_texmulti_6", MULTI),
        ("gm2_constfold_4", "a constant product in a shape Mesa's algebraic rules move or fold"),
    ];
    let moved: Vec<String> = pins.iter().filter_map(|(n, t)| mali_ref::refusal_change(n, t)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

const VS: &str = "attribute vec4 a_pos; attribute vec2 a_uv; varying vec2 v_uv; \
                  void main() { v_uv = a_uv; gl_Position = a_pos; }";

/// Forms of look-ups beside another that no reference draws stay refused:
/// a constant first in the coordinate's sum, a product in it, a coordinate
/// from two varyings, the colour reading a varying or a uniform.
#[test]
fn forms_without_a_reference_stay_refused() {
    let head = "precision mediump float; varying vec2 v_uv; uniform sampler2D s; uniform vec4 u; void main() {";
    let bodies = [
        "gl_FragColor = texture2D(s, v_uv) + texture2D(s, vec2(0.25, 0.5) + v_uv);",
        "gl_FragColor = texture2D(s, v_uv) + texture2D(s, v_uv * 0.5);",
        "gl_FragColor = texture2D(s, v_uv) + texture2D(s, v_uv + u.xy);",
        "gl_FragColor = texture2D(s, v_uv) * v_uv.x + texture2D(s, v_uv + vec2(0.25, 0.5));",
        "gl_FragColor = texture2D(s, v_uv) * u + texture2D(s, v_uv + vec2(0.25, 0.5));",
    ];
    for b in bodies {
        let fs = format!("{head} {b} }}");
        match mali_ref::compile_pair(VS, &fs) {
            Ok(_) => panic!("`{b}` compiles, but no reference draws it"),
            Err(e) => assert!(e.contains("Valhall:"), "`{b}`: {e}"),
        }
    }
}
