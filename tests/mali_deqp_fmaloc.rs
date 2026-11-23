// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_fmaloc_*` (plan #387 item
//! `fmaloc-ref`): the dEQP-GLES2 shapes of the refusal texts plan #367 held
//! through glmark2 shapes only. FMA ("a fused multiply-add other than a
//! matrix product's", `fma_*`), TEX_COORD ("a texture coordinate other than
//! a vec2 varying read whole or two constants", `tc_*`) and TEX_VERTEX ("a
//! texture look-up other than texture2D(sampler2D, coordinate)", `tk_*`).
//! LOCALS ("more than 32 local variables") has no shape: no case has it as
//! its first refusal at the tip (`docs/notes/mali-deqp-fmaloc.md`).
//!
//! One test per reference, each a pair verbatim from dEQP's `.qpa` (the
//! `.license` sidecar names the case) drawn on Mesa's drm-shim with
//! `tools/mali/deqp_ref.sh --instrument shim`. `fma_constmat_mat`,
//! `fma_const_reflect` and `fma_const_matmat` compile byte-equal since the
//! vertex walk folds an `ffma` of three constants (the constrnd lowering).
//! Every other one is refused, its test ignored with its refusal quoted; the
//! lowering removes the `ignore` and the reference's entry in `REFUSALS`
//! together.
//! `refusals_pinned` runs now: it asserts each reference's current refusal
//! and names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fmaloc
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const FMA: &str = "a fused multiply-add other than a matrix product's (a built-in function's)";
const TEX_COORD: &str = "a texture coordinate other than a vec2 varying read whole or two constants";
const TEX_KIND: &str = "a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)";

/// `const mat3 * const mat3` in the vertex shader: Mesa folds the product
/// and the whole colour, so the link moves it into the fragment shader
/// (no varying program).
#[test]
fn fmaloc_fma_constmat_mat() {
    mali_ref::assert_matches("deqp_fmaloc_fma_constmat_mat");
}

/// `const mat3 * attribute vec3`.
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_constmat_vec() {
    mali_ref::assert_matches("deqp_fmaloc_fma_constmat_vec");
}

/// `attribute mat3 * attribute vec3`.
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_attrmat_vec() {
    mali_ref::assert_matches("deqp_fmaloc_fma_attrmat_vec");
}

/// `attribute mat4 * uniform mat4` (also `*=`, `matrix.mul_assign`).
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_attrmat_unimat() {
    mali_ref::assert_matches("deqp_fmaloc_fma_attrmat_unimat");
}

/// `asin(x)` of an attribute lane (`acos` is the same polynomial).
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_asin() {
    mali_ref::assert_matches("deqp_fmaloc_fma_asin");
}

/// `atan(x)` of an attribute lane.
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_atan() {
    mali_ref::assert_matches("deqp_fmaloc_fma_atan");
}

/// `atan(y, x)` of two attribute lanes.
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_atan2() {
    mali_ref::assert_matches("deqp_fmaloc_fma_atan2");
}

/// `reflect(float, float)`: the scalar form, which `is_reflect` does not
/// take (its dot product is a plain product).
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn fmaloc_fma_reflect_float() {
    mali_ref::assert_matches("deqp_fmaloc_fma_reflect_float");
}

/// `const float cval = asin(vec2(0.0)).y`: a constant expression; Mesa's
/// GLSL front end folds it and the link moves `out0` into the fragment
/// shader.
/// Its constant `ffma`s and `sqrt` fold now; the constant varying of 0.0
/// meets the fragment shader's defect, which a vertex shader of `out0 =
/// 0.0;` meets alone too (it predates the fold).
#[test]
#[ignore = "refused: Valhall back end defect (IrIndex) of the fragment shader reading a constant varying"]
fn fmaloc_fma_const_asin() {
    mali_ref::assert_matches("deqp_fmaloc_fma_const_asin");
}

/// `const float cval = reflect(1.0, 1.0)`.
#[test]
fn fmaloc_fma_const_reflect() {
    mali_ref::assert_matches("deqp_fmaloc_fma_const_reflect");
}

/// `const mat3 a = mat3(3.0) * mat3(4.0)` and the constant matrix chain of
/// `constant_expressions.operators.math_mat`.
#[test]
fn fmaloc_fma_const_matmat() {
    mali_ref::assert_matches("deqp_fmaloc_fma_const_matmat");
}

/// `textureCube(s, vec3(-1, -y, +x))`, `x`, `y` computed from a vec2
/// varying (`texture.specification.*_cube`): `CUBEFACE1`, `CUBEFACE2_V9`,
/// `CUBE_SSEL`/`CUBE_TSEL`, `TEX_SINGLE...cube.computed`.
#[test]
#[ignore = "refused: a texture coordinate other than a vec2 varying read whole or two constants"]
fn fmaloc_tc_cube_face() {
    mali_ref::assert_matches("deqp_fmaloc_tc_cube_face");
}

/// `textureCube(u, vec3(0.0))` in the `uniform_api` compare harness.
#[test]
#[ignore = "refused: a texture coordinate other than a vec2 varying read whole or two constants"]
fn fmaloc_tc_cube_const() {
    mali_ref::assert_matches("deqp_fmaloc_tc_cube_const");
}

/// `textureCube(a, c - b)` of two vec3 varyings (`shaders.random.texture`).
#[test]
#[ignore = "refused: a texture coordinate other than a vec2 varying read whole or two constants"]
fn fmaloc_tc_cube_rnd_sub() {
    mali_ref::assert_matches("deqp_fmaloc_tc_cube_rnd_sub");
}

/// `texture2D(a, vec2(c, b))`, `c` a constant `true` and `b` a float
/// varying: the coordinate `(1.0, b)` in two `MOV`s.
#[test]
#[ignore = "refused: a texture coordinate other than a vec2 varying read whole or two constants"]
fn fmaloc_tc_2d_scalar() {
    mali_ref::assert_matches("deqp_fmaloc_tc_2d_scalar");
}

/// `texture2D(s, v, u_bias)`, a uniform bias (`texture.mipmap.2d.bias.*`).
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_bias_uniform() {
    mali_ref::assert_matches("deqp_fmaloc_tk_bias_uniform");
}

/// `texture2D(s, v, v_lodBias)`, a varying bias: `computed_bias` with the
/// bias clamped, scaled to 8.8 fixed point and packed (`F32_TO_S32.rtz`,
/// `U16_TO_U32`).
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_bias_varying() {
    mali_ref::assert_matches("deqp_fmaloc_tk_bias_varying");
}

/// `texture2DProj(s, vec3 varying)`: `FRCP.f32` of `.z` and two products.
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_proj_vec3() {
    mali_ref::assert_matches("deqp_fmaloc_tk_proj_vec3");
}

/// `texture2DProj(s, vec4 varying)`: the same divide by `.w`.
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_proj_vec4() {
    mali_ref::assert_matches("deqp_fmaloc_tk_proj_vec4");
}

/// `textureCube(s, v, v_lodBias)`.
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_cube_bias() {
    mali_ref::assert_matches("deqp_fmaloc_tk_cube_bias");
}

/// `texture2DProj(a, b)` in a `shaders.random.texture` shader.
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_rnd_proj() {
    mali_ref::assert_matches("deqp_fmaloc_tk_rnd_proj");
}

/// `texture2D(a, c, float(b))`, `b` a constant int 4: a constant bias,
/// a FAU word (`0x00000400`, 4.0 in 8.8) as the `computed_bias` operand.
#[test]
#[ignore = "refused: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query)"]
fn fmaloc_tk_rnd_bias() {
    mali_ref::assert_matches("deqp_fmaloc_tk_rnd_bias");
}

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_fmaloc_fma_constmat_vec", FMA),
    ("deqp_fmaloc_fma_attrmat_vec", FMA),
    ("deqp_fmaloc_fma_attrmat_unimat", FMA),
    ("deqp_fmaloc_fma_asin", FMA),
    ("deqp_fmaloc_fma_atan", FMA),
    ("deqp_fmaloc_fma_atan2", FMA),
    ("deqp_fmaloc_fma_reflect_float", FMA),
    ("deqp_fmaloc_fma_const_asin", "an invariant of the pipeline did not hold (IrIndex)"),
    ("deqp_fmaloc_tc_cube_face", TEX_COORD),
    ("deqp_fmaloc_tc_cube_const", TEX_COORD),
    ("deqp_fmaloc_tc_cube_rnd_sub", TEX_COORD),
    ("deqp_fmaloc_tc_2d_scalar", TEX_COORD),
    ("deqp_fmaloc_tk_bias_uniform", TEX_KIND),
    ("deqp_fmaloc_tk_bias_varying", TEX_KIND),
    ("deqp_fmaloc_tk_proj_vec3", TEX_KIND),
    ("deqp_fmaloc_tk_proj_vec4", TEX_KIND),
    ("deqp_fmaloc_tk_cube_bias", TEX_KIND),
    ("deqp_fmaloc_tk_rnd_proj", TEX_KIND),
    ("deqp_fmaloc_tk_rnd_bias", TEX_KIND),
];

#[test]
fn refusals_pinned() {
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "references whose refusal moved:\n{}", moved.join("\n"));
}
