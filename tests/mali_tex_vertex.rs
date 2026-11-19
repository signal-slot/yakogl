// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Two vertex texture look-ups (the TEX_VERTEX family of plan #367,
//! glmark2 terrain vs 18bc) on the Mali, against the Mesa 26.2.1 drm-shim
//! references `tools/mali/testdata/gm2_texvert_1..4`, byte for byte
//! (docs/notes/mali-vs-lookups.md section 7): two look-ups of one sampler,
//! of two samplers, terrain's displacement look-up beside its normal-map
//! look-up whose `* 2.0 - 1.0` is fp16 (`TEX_SINGLE.16`), and one look-up
//! twice (`nir_opt_cse`), whose copies then both read the fp16 texels.
//! Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region. The references
//! were drawn without gl_ref.py's `--uniform-values`: every uniform word of
//! their FAU is 0 ([`mali_ref::assert_matches_unset`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_tex_vertex
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `texture2D(s, uv)` and `texture2D(s, uv * 0.5 + 0.25)`: two
/// `TEX_SINGLE.32` of one handle, each straight to its varying.
#[test]
fn gm2_texvert_1_one_sampler_twice_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_texvert_1");
}

/// Two samplers at one coordinate: the handles `0x20001800` and
/// `0x20011801` in first-use order.
#[test]
fn gm2_texvert_2_two_samplers_match_their_reference() {
    mali_ref::assert_matches_unset("gm2_texvert_2");
}

/// terrain's shape: the displacement texel read by highp arithmetic stays
/// fp32 (every reader converts it up, `pan_nir_fuse_io_cvt`), the normal
/// map's `.xyz * 2.0 - 1.0` is mediump: `TEX_SINGLE.16.rgb @r0:r1`,
/// `FMA.v2f16 r0, r0^, ..` of x and y, `FMA.v2f16 r1, r1^.h00, ..` of z,
/// then `F16_TO_F32` per lane for the highp varying.
#[test]
fn gm2_texvert_3_terrain_displacement_and_normal_match_their_reference() {
    mali_ref::assert_matches_unset("gm2_texvert_3");
}

/// One look-up twice: one `TEX_SINGLE.16`, the first varying its four
/// lanes converted up, the second `t * vec4(0.5, 1.0, 1.0, 1.0) + a_pos`
/// with the constant pair one FAU word (`0x3C003800`) and the `* 1.0`
/// lanes folded away.
#[test]
fn gm2_texvert_4_one_look_up_twice_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_texvert_4");
}

/// What has no reference stays refused: a lowp sampler's texels read by a
/// mediump operation other than a product or a sum (fp16 in Mesa, not
/// built here).
#[test]
fn a_mediump_operation_other_than_a_product_or_sum_of_texels_stays_refused() {
    let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
    let vs = "uniform sampler2D s; attribute vec4 p; varying vec4 v; \
              void main() { gl_Position = p; v = vec4(max(texture2D(s, p.xy).x, 0.5)); }";
    mali_ref::assert_refused(vs, fs, "fp16 texels read by a mediump operation");
}

/// A look-up of a highp sampler is fp32 whatever reads it: a product of
/// its texels and a constant compiles at fp32 (no fp16 reader).
#[test]
fn a_highp_sampler_s_texels_stay_fp32() {
    let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
    let vs = "uniform highp sampler2D s; attribute vec4 p; varying vec4 v; \
              void main() { gl_Position = p; v = texture2D(s, p.xy) * 2.0; }";
    let pair = mali_ref::compile_pair(vs, fs).unwrap_or_else(|e| panic!("refused: {e}"));
    assert!(!pair.vs.code.is_empty());
}

/// The same look-up twice under highp weights (mali-vs-lookups.md §3.1's
/// shape, refused before CSE): one fp32 look-up, the pair compiles.
#[test]
fn the_same_look_up_twice_under_highp_weights_compiles() {
    let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
    let vs = "uniform sampler2D s; uniform vec4 w; attribute vec4 p; varying vec4 v; \
              void main() { gl_Position = p; v = w.x * texture2DLod(s, p.xy, 0.0) + w.y * texture2DLod(s, p.xy, 0.0); }";
    mali_ref::compile_pair(vs, fs).unwrap_or_else(|e| panic!("refused: {e}"));
}
