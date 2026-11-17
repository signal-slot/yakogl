// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A fragment texture result read in a swizzle or a lane alone (the
//! TEX_SWIZZLE family of plan #367, terrain fs d697) on the Mali, against
//! the Mesa 26.2.1 drm-shim references `tools/mali/testdata/gm2_texswz_1..6`,
//! byte for byte (docs/notes/mali-tex-swizzle.md): `.r` alone in every lane,
//! `.a * 0.5` of a highp shader, `.yx` beside constants, `.wzyx`, terrain's
//! `mix(a, b, 1.0 - c)` of three look-ups and d697's grey `vec4(g, g, g,
//! c.a)`. Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_tex_swizzle
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `vec4(t.r)`: the look-up writes `.r` alone (`@r0`), ATEST reads its
/// half, the word `IADD.v2u16 r0, r0^.h00, 0x0` and its copy `MOV.i32 r1,
/// r0`.
#[test]
fn gm2_texswz_1_one_lane_in_every_lane_matches_its_reference() {
    mali_ref::assert_matches("gm2_texswz_1");
}

/// `vec4(t.a * 0.5)` of a highp shader: the lowp texel's product is fp16
/// (lower_precision), one lane, `FMA.v2f16 r0, r0^.h00, 0x38000000.h11,
/// 0x0.neg` of the `.a` the write mask packs alone into `r0.h0`.
#[test]
fn gm2_texswz_2_highp_lane_alone_matches_its_reference() {
    mali_ref::assert_matches("gm2_texswz_2");
}

/// `vec4(t.yx, 0.0, 1.0)`: the word swapped, `IADD.v2u16 r0, r0^.h10,
/// 0x0`, beside the constant word.
#[test]
fn gm2_texswz_3_swapped_pair_matches_its_reference() {
    mali_ref::assert_matches("gm2_texswz_3");
}

/// `t.wzyx` of a highp shader: both words swapped (`IADD.v2u16 r1,
/// r1^.h10` then `r2, r0^.h10`), ATEST on `.x`, the look-up word's low
/// half.
#[test]
fn gm2_texswz_4_reversed_matches_its_reference() {
    mali_ref::assert_matches("gm2_texswz_4");
}

/// `mix(a, b, 1.0 - c)` of three look-ups (terrain's blend): the third's
/// words negated in `FADD.v2f16`, the first's the `ffma`'s addend.
#[test]
fn gm2_texswz_5_mix_of_three_look_ups_matches_its_reference() {
    mali_ref::assert_matches("gm2_texswz_5");
}

/// terrain d697's grey: `g = dot(c.rgb, k)` of one lane in both halves of
/// the first word, the second `MKVEC.v2i16` of `g` and `c.a`'s half of the
/// look-up's second word, ATEST on that half.
#[test]
fn gm2_texswz_6_grey_beside_alpha_matches_its_reference() {
    mali_ref::assert_matches("gm2_texswz_6");
}

const VS: &str = "attribute vec4 a_pos; attribute vec2 a_uv; varying vec2 v_uv; \
                  void main() { v_uv = a_uv; gl_Position = a_pos; }";

/// The shapes past what the references draw stay refused: a swapped word
/// read by an operation (a swizzle on an operation's source), a pair of
/// two texels that are neither in order nor swapped, a texel before a
/// computed lane, and fp16 arithmetic on a highp shader's texels other
/// than one lane by a constant.
#[test]
fn the_shapes_no_reference_draws_stay_refused() {
    let fs = |body: &str| {
        format!(
            "precision mediump float; varying vec2 v_uv; uniform sampler2D u_s0; \
             void main() {{ vec4 t = texture2D(u_s0, v_uv); {body} }}"
        )
    };
    let lanes = "a texture result's lanes other than its own pairs in order";
    mali_ref::assert_refused(VS, &fs("gl_FragColor = vec4(t.yx * 2.0, 0.0, 1.0);"), lanes);
    mali_ref::assert_refused(VS, &fs("gl_FragColor = vec4(t.xz, t.yw);"), lanes);
    mali_ref::assert_refused(VS, &fs("gl_FragColor = vec4(t.r, t.r * 0.5, 0.0, 1.0);"), lanes);
    mali_ref::assert_refused(
        VS,
        "precision highp float; varying vec2 v_uv; uniform sampler2D u_s0; \
         void main() { gl_FragColor = vec4(texture2D(u_s0, v_uv).a + 0.5); }",
        "mixes highp and mediump",
    );
}
