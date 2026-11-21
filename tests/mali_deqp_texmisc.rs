// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_texmisc_*` (plan #387, item
//! texmisc-ref: the dEQP-GLES2 cases whose first refusal at the tip
//! `ca7cdc52` is `MaliError::TexVertex`, `MaliError::TexKind` or
//! `MaliError::FragCoordMediump`; see `docs/notes/mali-deqp-texmisc.md`).
//! One test per reference:
//!
//! - `TexVertex` (vertex shaders, the `compare_vec4` harness of
//!   `uniform_api` on a mediump sampler's texels): `tv_2d_compare`,
//!   `tv_cube_compare`, `tv_2d_array_compare`
//!   (`uniform_api.value.initial.render.{basic,basic_array}.sampler*_vertex`),
//!   `tv_rnd_2d` (`uniform_api.random.30`), `tv_rnd_cube_array` (`.33`);
//! - `TexKind` (fragment shaders, the cases new at the tip; the 27 of
//!   `37305608` are `deqp_fmaloc_tk_*`, pinned in `tests/mali_deqp_fmaloc.rs`):
//!   `tk_rnd_proj_swizzle` (`shaders.random.texture.fragment.131`),
//!   `tk_rnd_proj_sum` (`.142`), `tk_rnd_bias_fragcolor` (`.124`),
//!   `tk_rnd_bias_two` (`.41`), `tk_rnd_bias_proj` (`.108`),
//!   `tk_rnd_cube_bias` (`.121`);
//! - `FragCoordMediump` (fragment shaders that never read `gl_FragCoord`:
//!   the fp16 vector of int-converted lanes): `fcm_int_to_float`,
//!   `fcm_ivec2_to_vec2`, `fcm_int_bool_to_vec2`, `fcm_int_to_mat2`
//!   (`shaders.conversions.*`), `fcm_rnd_int_lanes`
//!   (`shaders.random.texture.fragment.88`), `fcm_eval_order`
//!   (`shaders.functions.misc.argument_eval_order_1_fragment`).
//!
//! The dEQP pairs are verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` (VK-GL-CTS 93bca01); each `.license`
//! sidecar names the case.
//!
//! None compiles at the tip: each test is ignored with its refusal quoted;
//! the lowering removes the `ignore` and the reference's entry in
//! `REFUSALS` together, and adds it to `MATCHED`. `refusals_pinned` runs
//! now: it asserts each ignored reference's exact current refusal, names
//! every one that moved, and fails for a `deqp_texmisc_*` reference in
//! neither list.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_texmisc
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum"]
fn texmisc_tv_2d_compare() {
    mali_ref::assert_matches("deqp_texmisc_tv_2d_compare");
}

#[test]
#[ignore = "refused: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum"]
fn texmisc_tv_cube_compare() {
    mali_ref::assert_matches("deqp_texmisc_tv_cube_compare");
}

#[test]
#[ignore = "refused: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum"]
fn texmisc_tv_2d_array_compare() {
    mali_ref::assert_matches("deqp_texmisc_tv_2d_array_compare");
}

#[test]
#[ignore = "refused: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum"]
fn texmisc_tv_rnd_2d() {
    mali_ref::assert_matches("deqp_texmisc_tv_rnd_2d");
}

#[test]
#[ignore = "refused: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum"]
fn texmisc_tv_rnd_cube_array() {
    mali_ref::assert_matches("deqp_texmisc_tv_rnd_cube_array");
}

#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn texmisc_tk_rnd_proj_swizzle() {
    mali_ref::assert_matches("deqp_texmisc_tk_rnd_proj_swizzle");
}

#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn texmisc_tk_rnd_proj_sum() {
    mali_ref::assert_matches("deqp_texmisc_tk_rnd_proj_sum");
}

#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn texmisc_tk_rnd_bias_fragcolor() {
    mali_ref::assert_matches("deqp_texmisc_tk_rnd_bias_fragcolor");
}

#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn texmisc_tk_rnd_bias_two() {
    mali_ref::assert_matches("deqp_texmisc_tk_rnd_bias_two");
}

#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn texmisc_tk_rnd_bias_proj() {
    mali_ref::assert_matches("deqp_texmisc_tk_rnd_bias_proj");
}

#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn texmisc_tk_rnd_cube_bias() {
    mali_ref::assert_matches("deqp_texmisc_tk_rnd_cube_bias");
}

#[test]
#[ignore = "refused: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y"]
fn texmisc_fcm_int_to_float() {
    mali_ref::assert_matches("deqp_texmisc_fcm_int_to_float");
}

#[test]
#[ignore = "refused: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y"]
fn texmisc_fcm_ivec2_to_vec2() {
    mali_ref::assert_matches("deqp_texmisc_fcm_ivec2_to_vec2");
}

#[test]
#[ignore = "refused: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y"]
fn texmisc_fcm_int_bool_to_vec2() {
    mali_ref::assert_matches("deqp_texmisc_fcm_int_bool_to_vec2");
}

#[test]
#[ignore = "refused: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y"]
fn texmisc_fcm_int_to_mat2() {
    mali_ref::assert_matches("deqp_texmisc_fcm_int_to_mat2");
}

#[test]
#[ignore = "refused: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y"]
fn texmisc_fcm_rnd_int_lanes() {
    mali_ref::assert_matches("deqp_texmisc_fcm_rnd_int_lanes");
}

#[test]
#[ignore = "refused: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y"]
fn texmisc_fcm_eval_order() {
    mali_ref::assert_matches("deqp_texmisc_fcm_eval_order");
}

const TEX_VERTEX: &str = "a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum";
const TEX_KIND: &str = "a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)";
const FRAG_COORD: &str = "fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_texmisc_tv_2d_compare", TEX_VERTEX),
    ("deqp_texmisc_tv_cube_compare", TEX_VERTEX),
    ("deqp_texmisc_tv_2d_array_compare", TEX_VERTEX),
    ("deqp_texmisc_tv_rnd_2d", TEX_VERTEX),
    ("deqp_texmisc_tv_rnd_cube_array", TEX_VERTEX),
    ("deqp_texmisc_tk_rnd_proj_swizzle", TEX_KIND),
    ("deqp_texmisc_tk_rnd_proj_sum", TEX_KIND),
    ("deqp_texmisc_tk_rnd_bias_fragcolor", TEX_KIND),
    ("deqp_texmisc_tk_rnd_bias_two", TEX_KIND),
    ("deqp_texmisc_tk_rnd_bias_proj", TEX_KIND),
    ("deqp_texmisc_tk_rnd_cube_bias", TEX_KIND),
    ("deqp_texmisc_fcm_int_to_float", FRAG_COORD),
    ("deqp_texmisc_fcm_ivec2_to_vec2", FRAG_COORD),
    ("deqp_texmisc_fcm_int_bool_to_vec2", FRAG_COORD),
    ("deqp_texmisc_fcm_int_to_mat2", FRAG_COORD),
    ("deqp_texmisc_fcm_rnd_int_lanes", FRAG_COORD),
    ("deqp_texmisc_fcm_eval_order", FRAG_COORD),
];

/// The references that compile byte-equal (their tests run): none yet.
const MATCHED: &[&str] = &[];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_texmisc_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_texmisc_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
