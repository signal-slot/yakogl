// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa 26.2.1 references `tools/mali/testdata/deqp_audit_*` (plan #387,
//! item audit-ref-2, `docs/notes/mali-deqp-compiled-audit.md`): the dEQP-GLES2
//! shaders that the Mali online compiler compiles (they were refused at
//! `37305608`) but not to Mesa's words. Each shape the audit's section 4
//! names has a reference here, drawn with
//! `tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_audit_<name> --instrument shim`
//! (`drm_shim_ref.sh run ... --uniform-values` on Mesa 26.2.1's G610
//! drm-shim, then `pan_ref_diff.py extract --draw`). `tests/mali_ref` does the
//! compare. A reference that compiles to other words has its compare test
//! `#[ignore]`d, and `differs_pinned` holds the shapes in [`DIFFERS`]. The lowering step that makes one byte-equal moves it to
//! [`LOWERED`] in the same commit and drops its `#[ignore]`; the cause tags
//! (D1, V1, F1, S1, A1, M1, L1, R1, K1) are the audit note's section 4.
//!
//! Where the shader text comes from. The dEQP sources were fetched
//! sparse (`VK-GL-CTS` `93bca01861b0e3ef3c387027a9791e6d065f900c`, only
//! `modules/gles2` and `data/gles2` at `$CONNECTED_DEPS_DIR/VK-GL-CTS-gles3`;
//! no `.qpa` log of a dEQP run was available, so no pair is the log's text
//! verbatim). Every pair is **rebuilt** from the generator code that makes it:
//! the operator, matrix, indexing, fragdata and uniform-api cases from the C++
//! `ShaderOperatorCase`, `ShaderMatrixCase::init`, `createVectorSubscriptCase`,
//! `FragDataIndexingCase::genSources` and `UniformCase`; the conversion,
//! swizzle, constant-expression and linkage cases from their `.test` rows or
//! `TestParams`, wrapped in the ShaderLibrary / ShaderConstExpr template that
//! the existing `deqp_intval*`, `deqp_constop_*` pairs show (that template's
//! source, `modules/glshared/glsShader{Library,ConstExpr}Case.cpp`, was not
//! fetched). So the text is dEQP's up to white space and the template's
//! declaration order. Two things are not dEQP's: a `uniform_api` case compares
//! its uniform against a random value of the case's seed, and here the
//! literal is fixed (0.5, or a fixed mat4 for the `info_query` case; the
//! `initial` cases use 0.5 and zeros), which changes a FAU constant and no
//! instruction; the `linkage` case's output check is written like the other
//! ShaderLibrary pairs.
//!
//! Not drawn, with the reason: the `shaders.random.*` pairs the audit names
//! (`conditionals.vertex.42`, `conditionals.fragment.10`,
//! `comparison_ops.fragment.21`, `conditionals.fragment.59` and the two L1
//! `random` pairs) come from dEQP's random shader generator (`rsg`) from a
//! seed, which needs a built dEQP, not the sparse sources; and
//! `attribute_location.bind_aliasing`, `bind_max_attributes.vec3`, which need
//! `glBindAttribLocation` (item board-baseline-rows adds it to `gl_ref.py`).
//! The audit's D1 shape has 13 cases, five are drawn.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_audit
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_audit_` prefix: each has a byte-compare test below.
const LOWERED: &[&str] = &[
    "d1_conv_bvec2_int_to_ivec3_fs",
    "d1_conv_bvec3_to_ivec3_fs",
    "d1_conv_bvec4_to_ivec2_fs",
    "d1_conv_vec2_bvec2_to_ivec4_fs",
    "l1_matrix_sub_uniform_mediump_mat3_vs",
];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[
    "a1_op_greater_or_equal_highp_float_vs",
    "a1_op_greaterthan_highp_ivec2_vs",
    "d1_conv_bool_bool_to_ivec2_fs",
    "f1_conv_float_to_mat3_fs",
    "f1_conv_vec3_float_to_mat2_fs",
    "f1_op_distance_mediump_float_fs",
    "f1_op_equal_mediump_vec4_fs",
    "f1_op_greaterthan_lowp_vec4_fs",
    "f1_swz_mediump_vec3_rb_fs",
    "f1_swz_mediump_vec3_sp_fs",
    "f1_swz_mediump_vec3_x_fs",
    "f1_swz_mediump_vec4_wwwy_fs",
    "f1_uniform_info_mat4_fs",
    "k1_cexpr_length_vec3_fs",
    "l1_op_greaterthan_mediump_ivec2_fs",
    "l1_op_mul_highp_int_ivec2_fs",
    "m1_cexpr_greaterthan_ivec2_vs",
    "m1_matrix_mul_uniform_highp_mat2_vs",
    "m1_uniform_initial_mat4_vs",
    "o_fragdata_valid_dynamic_index",
    "o_linkage_uniform_struct_vec4",
    "r1_idx_vec2_static_loop_write_component_read_fs",
    "r1_swz_mediump_vec4_qpts_vs",
    "r1_swz_mediump_vec4_xyzw_vs",
    "s1_matrix_add_dynamic_lowp_mat2_vs",
    "s1_swz_mediump_vec4_yyyy_fs",
    "v1_conv_bool_to_bool_vs",
    "v1_matrix_div_dynamic_mediump_mat3_vs",
    "v1_op_postinc_lowp_int_vs",
    "v1_op_sqrt_mediump_vec4_vs",
    "v1_swz_mediump_bvec2_grg_vs",
    "v1_uniform_info_float_both",
    "v1_uniform_initial_float_vs",
];

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.bvec2_int_to_ivec3_fragment` (cause D1): the odd-lane miscompile: `ivec3(in0, in1)` read lane x of the bvec2 for lane y; `b2i32` of a 16-bit mask now widens the lane's own half (`IADD.v2u16 r2, r0.h11, 0x0`).
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
fn d1_conv_bvec2_int_to_ivec3_fs() {
    mali_ref::assert_matches("deqp_audit_d1_conv_bvec2_int_to_ivec3_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.bool_bool_to_ivec2_fragment` (cause D1): two bool inputs widened to an ivec2. Each lane now widens its own mask; the words still differ: Mesa narrows and compares the two varyings as one pair (`V2F32_TO_V2F16`, `FCMP_OR.v2f16`), ours one at a time.
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn d1_conv_bool_bool_to_ivec2_fs() {
    mali_ref::assert_matches("deqp_audit_d1_conv_bool_bool_to_ivec2_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_vector.bvec3_to_ivec3_fragment` (cause D1): D1 again: a whole bvec3 widened.
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
fn d1_conv_bvec3_to_ivec3_fs() {
    mali_ref::assert_matches("deqp_audit_d1_conv_bvec3_to_ivec3_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_to_vector.bvec4_to_ivec2_fragment` (cause D1): D1 again: a bvec4 truncated to an ivec2.
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
fn d1_conv_bvec4_to_ivec2_fs() {
    mali_ref::assert_matches("deqp_audit_d1_conv_bvec4_to_ivec2_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.vector_combine.vec2_bvec2_to_ivec4_fragment` (cause D1): D1 again: a vec2 and a bvec2 into an ivec4.
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
fn d1_conv_vec2_bvec2_to_ivec4_fs() {
    mali_ref::assert_matches("deqp_audit_d1_conv_vec2_bvec2_to_ivec4_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_scalar.bool_to_bool_vertex` (cause V1): mediump `bool()` of a vertex attribute at fp32 (`F2b`, `nir.rs:4909`).
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_conv_bool_to_bool_vs() {
    mali_ref::assert_matches("deqp_audit_v1_conv_bool_to_bool_vs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_bvec2_grg_vertex` (cause V1): mediump bvec compare at fp32 in the vertex walk.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_swz_mediump_bvec2_grg_vs() {
    mali_ref::assert_matches("deqp_audit_v1_swz_mediump_bvec2_grg_vs");
}

/// `dEQP-GLES2.functional.shaders.operator.unary_operator.post_increment_result.lowp_int_vertex` (cause V1): lowp int arithmetic kept at fp32 where Mesa narrows.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_op_postinc_lowp_int_vs() {
    mali_ref::assert_matches("deqp_audit_v1_op_postinc_lowp_int_vs");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.sqrt.mediump_vec4_vertex` (cause V1): `FRSQ.f32` where Mesa has `FRSQ.f16`.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_op_sqrt_mediump_vec4_vs() {
    mali_ref::assert_matches("deqp_audit_v1_op_sqrt_mediump_vec4_vs");
}

/// `dEQP-GLES2.functional.shaders.matrix.div.dynamic_mediump_mat3_mat3_vertex` (cause V1): mediump matrix arithmetic at fp32.
/// Rebuilt from es2fShaderMatrixTests.cpp `ShaderMatrixCase::init`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_matrix_div_dynamic_mediump_mat3_vs() {
    mali_ref::assert_matches("deqp_audit_v1_matrix_div_dynamic_mediump_mat3_vs");
}

/// `dEQP-GLES2.functional.uniform_api.value.initial.get_uniform.basic.float_vertex` (cause V1): an fp32 compare of a widened fp16 uniform; Mesa `FCMP_OR.v2f16` against `2a66`.
/// Rebuilt from es2fUniformApiTests.cpp `generateVertexSource`/`generateFragmentSource` (the uniform's value is a fixed literal, see the header).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_uniform_initial_float_vs() {
    mali_ref::assert_matches("deqp_audit_v1_uniform_initial_float_vs");
}

/// `dEQP-GLES2.functional.uniform_api.info_query.basic.float_both` (cause V1): the same fp16 uniform compare, in both stages.
/// Rebuilt from es2fUniformApiTests.cpp `generateVertexSource`/`generateFragmentSource` (the uniform's value is a fixed literal, see the header).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn v1_uniform_info_float_both() {
    mali_ref::assert_matches("deqp_audit_v1_uniform_info_float_both");
}

/// `dEQP-GLES2.functional.uniform_api.value.initial.get_uniform.basic.mat4_vertex` (cause M1/V1): the mat4 form of the fp16 uniform compare.
/// Rebuilt from es2fUniformApiTests.cpp `generateVertexSource`/`generateFragmentSource` (the uniform's value is a fixed literal, see the header).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn m1_uniform_initial_mat4_vs() {
    mali_ref::assert_matches("deqp_audit_m1_uniform_initial_mat4_vs");
}

/// `dEQP-GLES2.functional.shaders.operator.float_compare.greaterThan.lowp_vec4_fragment` (cause F1): two fp32 varyings compared at fp16 (`nir_fs.rs:5950-6005`); Mesa `FCMP_OR.f32` and `MUX.v2i16`.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_op_greaterthan_lowp_vec4_fs() {
    mali_ref::assert_matches("deqp_audit_f1_op_greaterthan_lowp_vec4_fs");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.equal.mediump_vec4_fragment` (cause F1): the same fp32 compare of two varyings, `==`.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_op_equal_mediump_vec4_fs() {
    mali_ref::assert_matches("deqp_audit_f1_op_equal_mediump_vec4_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec3_rb_fragment` (cause F1): two one-lane `FCMP_OR.v2f16` where Mesa builds one vector compare.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_swz_mediump_vec3_rb_fs() {
    mali_ref::assert_matches("deqp_audit_f1_swz_mediump_vec3_rb_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec3_sp_fragment` (cause F1): as `rb`, with the `sp` swizzle.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_swz_mediump_vec3_sp_fs() {
    mali_ref::assert_matches("deqp_audit_f1_swz_mediump_vec3_sp_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec3_x_fragment` (cause F1): a scalar `isOk` (`abs(a-b) <= eps*abs(b)+eps`).
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_swz_mediump_vec3_x_fs() {
    mali_ref::assert_matches("deqp_audit_f1_swz_mediump_vec3_x_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec4_wwwy_fragment` (cause F1): as `rb`, for a vec4.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_swz_mediump_vec4_wwwy_fs() {
    mali_ref::assert_matches("deqp_audit_f1_swz_mediump_vec4_wwwy_fs");
}

/// `dEQP-GLES2.functional.shaders.operator.geometric.distance.mediump_float_fragment` (cause F1): `distance(float, float)` lowered to `sqrt(x*x)` (`builtin_call.rs:882-892`); Mesa `abs(a-b)`.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_op_distance_mediump_float_fs() {
    mali_ref::assert_matches("deqp_audit_f1_op_distance_mediump_float_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.matrix_combine.vec3_float_to_mat2_fragment` (cause F1): the `iand(fge, fge)` to `fge(fmax)` rule is missing in the `Ball` arm.
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_conv_vec3_float_to_mat2_fs() {
    mali_ref::assert_matches("deqp_audit_f1_conv_vec3_float_to_mat2_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.float_to_mat3_fragment` (cause F1): the same missing rule, a mat3.
/// Rebuilt from conversions.test through the ShaderLibrary template of the existing `deqp_intval3_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_conv_float_to_mat3_fs() {
    mali_ref::assert_matches("deqp_audit_f1_conv_float_to_mat3_fs");
}

/// `dEQP-GLES2.functional.uniform_api.info_query.basic.mat4_fragment` (cause F1): the same missing rule, a mat4 uniform compare.
/// Rebuilt from es2fUniformApiTests.cpp `generateVertexSource`/`generateFragmentSource` (the uniform's value is a fixed literal, see the header).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn f1_uniform_info_mat4_fs() {
    mali_ref::assert_matches("deqp_audit_f1_uniform_info_mat4_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec4_yyyy_fragment` (cause S1): the same opcodes in another order: Mesa does `abs(b)` and the `FMA` first.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn s1_swz_mediump_vec4_yyyy_fs() {
    mali_ref::assert_matches("deqp_audit_s1_swz_mediump_vec4_yyyy_fs");
}

/// `dEQP-GLES2.functional.shaders.matrix.add.dynamic_lowp_mat2_mat2_vertex` (cause S1): the same opcodes in another order, a vertex shader.
/// Rebuilt from es2fShaderMatrixTests.cpp `ShaderMatrixCase::init`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn s1_matrix_add_dynamic_lowp_mat2_vs() {
    mali_ref::assert_matches("deqp_audit_s1_matrix_add_dynamic_lowp_mat2_vs");
}

/// `dEQP-GLES2.functional.shaders.operator.int_compare.greaterThan.highp_ivec2_vertex` (cause A1): `LD_ATTR_IMM` order: ours loads index 1 first, Mesa index 2.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn a1_op_greaterthan_highp_ivec2_vs() {
    mali_ref::assert_matches("deqp_audit_a1_op_greaterthan_highp_ivec2_vs");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.greater_or_equal.highp_float_vertex` (cause A1): the same attribute-load order.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn a1_op_greater_or_equal_highp_float_vs() {
    mali_ref::assert_matches("deqp_audit_a1_op_greater_or_equal_highp_float_vs");
}

/// `dEQP-GLES2.functional.shaders.constant_expressions.builtin_functions.vector_relational.greaterThan_ivec2_vertex` (cause M1): `LSHIFT_OR .not` and `MUX(0, 1.0, ~b)` where Mesa has `MOV` and `MUX(1.0, 0, b)`; ours has a varying program, Mesa none.
/// Rebuilt from es2fShaderConstExprTests.cpp row through the template of the existing `deqp_constop_ce_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn m1_cexpr_greaterthan_ivec2_vs() {
    mali_ref::assert_matches("deqp_audit_m1_cexpr_greaterthan_ivec2_vs");
}

/// `dEQP-GLES2.functional.shaders.matrix.mul.uniform_highp_mat2_mat2_vertex` (cause M1): two FMA chains and an `FADD` where Mesa has one chain of four FMAs.
/// Rebuilt from es2fShaderMatrixTests.cpp `ShaderMatrixCase::init`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn m1_matrix_mul_uniform_highp_mat2_vs() {
    mali_ref::assert_matches("deqp_audit_m1_matrix_mul_uniform_highp_mat2_vs");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul.highp_int_ivec2_fragment` (cause L1): varying load `.v3` where Mesa keeps `.v4`.
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn l1_op_mul_highp_int_ivec2_fs() {
    mali_ref::assert_matches("deqp_audit_l1_op_mul_highp_int_ivec2_fs");
}

/// `dEQP-GLES2.functional.shaders.operator.int_compare.greaterThan.mediump_ivec2_fragment` (cause L1): the varying load order (Mesa loads `0x10` first).
/// Rebuilt from es2fShaderOperatorTests.cpp `ShaderOperatorCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn l1_op_greaterthan_mediump_ivec2_fs() {
    mali_ref::assert_matches("deqp_audit_l1_op_greaterthan_mediump_ivec2_fs");
}

/// `dEQP-GLES2.functional.shaders.matrix.sub.uniform_mediump_mat3_mat3_vertex` (cause L1): the constant `1.0` lane folded into the fragment shader where Mesa keeps it in the flat `.v4`.
/// Rebuilt from es2fShaderMatrixTests.cpp `ShaderMatrixCase::init`.
#[test]
fn l1_matrix_sub_uniform_mediump_mat3_vs() {
    mali_ref::assert_matches("deqp_audit_l1_matrix_sub_uniform_mediump_mat3_vs");
}

/// `dEQP-GLES2.functional.shaders.indexing.vector_subscript.vec2_static_loop_subscript_write_component_read_fragment` (cause R1): ours fuses `tmp.y + tmp.x` into one `FMA.v2f16`; Mesa keeps the product and adds.
/// Rebuilt from es2fShaderIndexingTests.cpp `createVectorSubscriptCase`.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn r1_idx_vec2_static_loop_write_component_read_fs() {
    mali_ref::assert_matches("deqp_audit_r1_idx_vec2_static_loop_write_component_read_fs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec4_xyzw_vertex` (cause R1): the fragment `isOk`: Mesa does the `.zw` pair first and puts `ATEST` after the colour `MKVEC`.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn r1_swz_mediump_vec4_xyzw_vs() {
    mali_ref::assert_matches("deqp_audit_r1_swz_mediump_vec4_xyzw_vs");
}

/// `dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec4_qpts_vertex` (cause R1): as `xyzw`, the `qpts` swizzle.
/// Rebuilt from swizzles.test through the ShaderLibrary template of the existing `deqp_intval_reslib_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn r1_swz_mediump_vec4_qpts_vs() {
    mali_ref::assert_matches("deqp_audit_r1_swz_mediump_vec4_qpts_vs");
}

/// `dEQP-GLES2.functional.shaders.constant_expressions.builtin_functions.geometric.length_vec3_fragment` (cause K1): `length(vec3(1.0))` folds to FAU `3fddb3d8`; Mesa `3fddb3d7` (1 ulp).
/// Rebuilt from es2fShaderConstExprTests.cpp row through the template of the existing `deqp_constop_ce_*` pairs.
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn k1_cexpr_length_vec3_fs() {
    mali_ref::assert_matches("deqp_audit_k1_cexpr_length_vec3_fs");
}

/// `dEQP-GLES2.functional.shaders.fragdata.valid_dynamic_index` (cause other): Mesa blends `gl_FragData[i]` into all 8 targets, ours into one.
/// Rebuilt from es2fShaderFragDataTests.cpp `genSources` (verbatim).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn o_fragdata_valid_dynamic_index() {
    mali_ref::assert_matches("deqp_audit_o_fragdata_valid_dynamic_index");
}

/// `dEQP-GLES2.functional.shaders.linkage.uniform_struct_vec4` (cause other): a varying computed from uniforms only, which Mesa's link moves into the fragment shader; Mesa cancels `(s+d)-d`.
/// Rebuilt from linkage.test `uniform_struct_vec4` (the vertex and fragment bodies verbatim, the output check as the ShaderLibrary writes it).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by differs_pinned"]
fn o_linkage_uniform_struct_vec4() {
    mali_ref::assert_matches("deqp_audit_o_linkage_uniform_struct_vec4");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_audit_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

fn region_words(json: &str, region: &str) -> Option<Result<usize, String>> {
    let at = json.find(&format!("\"{region}\": {{"))?;
    let rest = &json[at..];
    let words = match rest.find("\"words\": [") {
        Some(w) => &rest[w + "\"words\": [".len()..],
        None => return Some(Err(format!("{region}: no words"))),
    };
    let list = &words[..words.find(']').unwrap_or(0)];
    let mut n = 0;
    for w in list.split(',').map(|w| w.trim().trim_matches('"')).filter(|w| !w.is_empty()) {
        if w.len() != 16 || !w.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(Err(format!("{region}: {w:?} is not a 64-bit hex word")));
        }
        n += 1;
    }
    Some(Ok(n))
}

/// Every reference on disk is listed, its pair and trace are its own, and
/// each region has 64-bit hex words.
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "a deqp_audit_* reference is in neither LOWERED nor DIFFERS");
    let mut bad = Vec::new();
    for name in &names {
        let r = mali_ref::Reference::load(&format!("deqp_audit_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_audit_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_audit_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                None if region == "vs.varying" => {}
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(_)) => {}
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each DIFFERS shape still compiles and still is not Mesa's words; each
/// LOWERED one is byte-equal. A lowering step that moves one changes the
/// lists in the same commit.
#[test]
fn differs_pinned() {
    let mut changed = Vec::new();
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_audit_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED and drop its #[ignore]"));
                }
            }
            Err(e) => changed.push(format!("{name}: no longer compiles: {e}")),
        }
    }
    for name in LOWERED {
        let r = mali_ref::Reference::load(&format!("deqp_audit_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = mali_ref::differences(&r, &pair);
                if !d.is_empty() {
                    changed.push(format!("{name}: no longer byte-equal: {}", d[0]));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
