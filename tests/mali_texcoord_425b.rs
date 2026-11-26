// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's texture scenes' fragment shader 425b on the Mali (plan #367
//! lower-nonfinite-const, step texcoord-425b), against the Mesa 26.2.1
//! drm-shim references `tools/mali/testdata/gm2_tc425_*`, byte for byte
//! (docs/notes/mali-texcoord-varyings.md):
//!
//! - vs ace2's `Color` is `vec4(diffuse * vec3(1.0), 1.0)`. The link
//!   (`nir_opt_varyings`) keeps its three equal lanes as one component and
//!   drops the constant alpha, so `TextureCoord` takes lanes 1 and 2 of that
//!   one load: `LD_VAR_BUF_IMM ... .v3 @r0:r1:r2`, then `MOV.i32 r4, r1^`,
//!   `MOV.i32 r5, r2^` into the look-up's staging pair;
//! - Color's lane 0 is converted where the colour reads it, after the
//!   look-up (`FADD.f32.wait1 r0.h0`);
//! - `texel * Color` is `FMA.v2f16 r1, r2^, r0.h00` and `FMA.v2f16 r0, r3^,
//!   r0^` of `MKVEC.v2i16 r0, r0^.h0, u2.w1.h0` (the alpha's 1.0).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_texcoord_425b
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// 425b with vs ace2, as glmark2 builds the program.
#[test]
fn gm2_tc425_1_the_texture_scene_matches_its_reference() {
    mali_ref::assert_matches("gm2_tc425_1");
}

/// The same pair under the texture scene's own source text.
#[test]
fn gm2_tc425_4_matches_its_reference() {
    mali_ref::assert_matches("gm2_tc425_4");
}

/// _2, 425b with a highp default: Mesa keeps the lowp sampler's texel at
/// fp16 (`TEX_SINGLE ... .16`, then `F16_TO_F32` of each half) and the walk
/// does not. _3, the varyings declared in the other order: the link packs
/// the coordinate across two slots (`MOV.i32 r6, r5^`, `MOV.i32 r7, r0^` of
/// two loads). Both are pinned, so the item that lowers them updates this
/// file.
#[test]
fn gm2_tc425_2_and_3_stay_refused() {
    let tc = "a texture coordinate other than a vec2 varying read whole or two constants";
    let moved: Vec<String> =
        ["gm2_tc425_2", "gm2_tc425_3"].iter().filter_map(|n| mali_ref::refusal_change(n, tc)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// A coordinate in lanes 1 and 2 of a vec4 varying (`v.yz`) is no link
/// packing: its own load's lanes out of pair order, refused.
#[test]
fn a_swizzled_coordinate_of_one_varying_stays_refused() {
    let vs = "attribute vec4 p; varying vec4 v; void main() { gl_Position = p; v = p; }";
    let fs = "precision mediump float; uniform sampler2D s; varying vec4 v; \
              void main() { gl_FragColor = texture2D(s, v.yz); }";
    mali_ref::assert_refused(vs, fs, "a texture coordinate other than");
}
