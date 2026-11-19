// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Texture coordinates (the TEX_COORD family of plan #367: ideas df64,
//! jellyfish 22fc's caustics look-up) and gl_FragColor lanes (FRAGCOLOR_LANE:
//! jellyfish 22fc's tail) on the Mali, against the Mesa 26.2.1 drm-shim
//! references `tools/mali/testdata/gm2_texcoord_1..4` and `gm2_fclane_1..4`,
//! byte for byte (docs/notes/mali-tex-coords.md "glmark2's coordinates",
//! docs/notes/mali-varying-constants.md "glmark2's colour lanes"):
//!
//! - the first two lanes of a wider varying as a highp coordinate, and the
//!   fp16 sum of two look-ups' texels (`FADD.v2f16` of their words);
//! - a vec2 varying the link packed into lanes 2 and 3 of a slot as a
//!   coordinate (`TEX_SINGLE ... @r6:r7, @r6:r7`);
//! - `pow(x, 2.0)` as `x * x` (nir_opt_algebraic.py:1883);
//! - a 2D look-up read once grafted where it is read (GLSL's
//!   `opt_tree_grafting`), which orders the handles' FAU words;
//! - an aligned pair of a varying's load the other way round in the colour,
//!   one `V2F32_TO_V2F16` with the sources swapped.
//!
//! Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region. The references
//! were drawn with gl_ref.py's `--uniform-values`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_texcoord_lane
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `texture2D(tex, v.zw)` alone: the vec4 varying's read pair is the one
/// the vertex shader stores, loaded `.v2` (equal before this lowering; it
/// pins the computed form).
#[test]
fn gm2_texcoord_3_the_zw_lanes_alone_match_their_reference() {
    mali_ref::assert_matches("gm2_texcoord_3");
}

/// `texture2D(s0, v.xy) + texture2D(s1, v.zw)`: one `.v4` load, each look-up
/// `TEX_SINGLE.16` in place on its pair, then `FADD.v2f16` of the two
/// results' words to the mediump colour.
#[test]
fn gm2_texcoord_4_two_halves_of_one_varying_match_their_reference() {
    mali_ref::assert_matches("gm2_texcoord_4");
}

/// `vec4(vd.g, vd.r, vd.b, va.r)`: the link packs the four lanes in one slot;
/// `V2F32_TO_V2F16 r0, r1^, r0^` converts the swapped pair, `r1, r2^, r3^`
/// the other.
#[test]
fn gm2_fclane_1_lanes_across_varyings_match_their_reference() {
    mali_ref::assert_matches("gm2_fclane_1");
}

/// jellyfish 22fc's tail: `t0` and `t1` packed in one slot (the second
/// look-up's coordinate lanes 2 and 3), `pow(vd.r, 2.0)` a product fused
/// into `FMA.f32 r0, r0^, r0^, r4^`, and `m`'s handle pushed before `c`'s.
#[test]
fn gm2_fclane_2_jellyfish_s_tail_matches_its_reference() {
    mali_ref::assert_matches("gm2_fclane_2");
}

/// The same tail with `vd.r * vd.r`: the words of gm2_fclane_2.
#[test]
fn gm2_fclane_3_the_tail_with_a_product_matches_its_reference() {
    mali_ref::assert_matches("gm2_fclane_3");
}

/// `vec4(v.rgb, v.a - 0.3)`: the control, equal before this lowering.
#[test]
fn gm2_fclane_4_the_control_matches_its_reference() {
    mali_ref::assert_matches("gm2_fclane_4");
}

/// ideas df64's coordinate `gl_FragCoord.xy / 32.0` is fp16 arithmetic on
/// the mediump built-in: FRAGCOORD_FP16, plan #367's lower-fragcoord16,
/// not this family. Pinned so that the item that lowers it updates this.
#[test]
fn gm2_texcoord_1_stays_refused_at_its_fp16_frag_coord() {
    mali_ref::assert_refused_ref("gm2_texcoord_1", "fp16 arithmetic on gl_FragCoord");
}

/// jellyfish 22fc's caustics coordinate compiles, but Mesa's link moves the
/// whole `v.x / 24.0 + t / 20.0` into the vertex shader (the uniform `t`
/// with it: `can_move_uniforms`, nir_opt_varyings.c:2006), where ours moves
/// `v.x / 24.0` and leaves `t / 20.0` in the fragment shader: the
/// vertex shader would need a uniform it does not declare (no item yet).
/// When that lands this test fails: make it [`mali_ref::assert_matches`].
#[test]
fn gm2_texcoord_2_compiles_and_still_differs_at_the_uniform_motion() {
    let r = mali_ref::Reference::load("gm2_texcoord_2");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("refused: {e}"));
    let d = mali_ref::differences(&r, &pair);
    assert!(!d.is_empty(), "gm2_texcoord_2 is byte-equal now: pin it with assert_matches");
    assert!(d.iter().any(|x| x.starts_with("vs.varying")), "the difference moved:\n{}", d.join("\n"));
}

const FS_HEAD: &str = "precision highp float; uniform sampler2D s0; uniform sampler2D s1; varying vec4 v; ";
const VS_V4: &str = "attribute vec4 p; attribute vec4 a; varying vec4 v; void main() { gl_Position = p; v = a; }";

/// What has no reference stays refused: a texel plus a constant, of a
/// highp shader's lowp sampler, to the mediump colour.
#[test]
fn a_texel_plus_a_constant_stays_refused() {
    let fs = format!("{FS_HEAD}void main() {{ gl_FragColor = texture2D(s0, v.xy) + 0.5; }}");
    mali_ref::assert_refused(VS_V4, &fs, "mixes highp and mediump");
}

/// `pow(x, 3.0)` (Mesa's `a * a * a`) has no reference.
#[test]
fn pow_by_three_stays_refused() {
    let fs = format!("{FS_HEAD}void main() {{ gl_FragColor = vec4(pow(v.x, 3.0)); }}");
    mali_ref::assert_refused(VS_V4, &fs, "arithmetic other than");
}

/// A uniform's aligned pair the other way round in the colour has no
/// reference (a varying's has, gm2_fclane_1).
#[test]
fn a_uniform_s_swapped_pair_stays_refused() {
    let fs = "precision highp float; uniform vec4 u; void main() { gl_FragColor = vec4(u.y, u.x, 0.0, 1.0); }";
    let vs = "attribute vec4 p; void main() { gl_Position = p; }";
    mali_ref::assert_refused(vs, fs, "gl_FragColor lane");
}

/// The coordinate of a mediump shader's look-up that is half of a wider
/// varying has no reference here (Mesa's words are inferred equal to the
/// highp ones, mali-tex-coords.md step 4, not measured on our side).
#[test]
fn a_mediump_shader_s_half_varying_coordinate_stays_refused() {
    let fs = "precision mediump float; uniform sampler2D s; varying vec4 v; \
              void main() { gl_FragColor = texture2D(s, v.xy); }";
    mali_ref::assert_refused(VS_V4, fs, "texture coordinate");
}
