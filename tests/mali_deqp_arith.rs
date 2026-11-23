// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_arith_*` (plan #387, item
//! `arith-ref`, `docs/notes/mali-deqp-arith.md`), one test per reference.
//! `tests/mali_ref` does the compare.
//!
//! The 144 references are the dEQP-GLES2 shapes whose first compile refusal
//! at the tip (`9ac31f30`) is one of the three texts of the ARITH family
//! ([`ARITH`], [`OPERAND`], [`CHAIN`]): each is one case's shader pair as dEQP
//! writes it (the first case of its shape, stage and precision, the scalar
//! `float` one where the shape has one), drawn with
//! `tools/mali/drm_shim_ref.sh` through `tools/mali/deqp_ref.sh --instrument
//! shim` (Mesa 26.2.1, `--uniform-values`). None compiles yet: every test is
//! `#[ignore]`d with its refusal pinned by [`refusals_pinned`], which asserts
//! the whole `MaliError` message. The lowering that lifts a reference moves
//! it to [`LOWERED`] (or [`DIFFERS`]) and un-ignores its test in the same
//! commit; the `deqp_arith_<shape>_<stage>_<precision>` names, the case counts
//! of each shape (HOST-ONLY) and the order of the lowering steps are in the
//! note.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_arith
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the lowering made byte-equal, without the `deqp_arith_`
/// prefix: each has a byte-compare test below.
const LOWERED: &[&str] = &[
    "matrix_div_fs_highp",
    "constexpr_fceil_vs_highp",
    "constexpr_fsign_vs_highp",
    /* arith step 3: floor, ceil, sign of a mediump varying lane, float `!=`
     * of varying lanes (fp32 at either precision) */
    "op_binary_not_equal_fs_highp",
    "op_binary_not_equal_fs_mediump",
    "op_common_ceil_fs_mediump",
    "op_common_floor_fs_mediump",
    "op_common_sign_fs_mediump",
    "op_float_notequal_fs_highp",
    "op_float_notequal_fs_mediump",
    /* arith step 4a: min, max, clamp of mediump varying lanes */
    "op_common_clamp_fs_mediump",
    "op_common_max_fs_mediump",
    "op_common_min_fs_mediump",
];

/// The references that compile with other words than Mesa's. None yet.
const DIFFERS: &[&str] = &[];

/// `MaliError::Arithmetic`.
const ARITH: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";
/// `MaliError::FsOperandForm`.
const OPERAND: &str = "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet";
/// `MaliError::Rebalance`.
const CHAIN: &str = "Valhall: three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered yet";

/// Every ignored reference with its whole `MaliError` message, in the note's
/// shape order: the shape, the stage and precision of the reference, how many
/// of the 1,627 measured cases its bucket holds (HOST-ONLY) and the dEQP case
/// the pair is.
const REFUSED: &[(&str, &str)] = &[
    /* constexpr.ball fs highp: 3 cases of shaders.constant_expressions.builtin_functions.vector_relational.all_bvec2_fragment */
    ("constexpr_ball_fs_highp", ARITH),
    /* constexpr.ball vs highp: 3 cases of shaders.constant_expressions.builtin_functions.vector_relational.all_bvec2_vertex */
    ("constexpr_ball_vs_highp", ARITH),
    /* constexpr.bany fs highp: 3 cases of shaders.constant_expressions.builtin_functions.vector_relational.any_bvec2_fragment */
    ("constexpr_bany_fs_highp", ARITH),
    /* constexpr.bany vs highp: 3 cases of shaders.constant_expressions.builtin_functions.vector_relational.any_bvec2_vertex */
    ("constexpr_bany_vs_highp", ARITH),
    /* constexpr.fceil vs highp: 4 cases of shaders.constant_expressions.builtin_functions.common.ceil_float_vertex */
    /* constexpr.fcos fs highp: 4 cases of shaders.constant_expressions.builtin_functions.trigonometry.cos_float_fragment */
    ("constexpr_fcos_fs_highp", ARITH),
    /* constexpr.fcos vs highp: 4 cases of shaders.constant_expressions.builtin_functions.trigonometry.cos_float_vertex */
    ("constexpr_fcos_vs_highp", ARITH),
    /* constexpr.fexp2 fs highp: 8 cases of shaders.constant_expressions.builtin_functions.exponential.exp2_float_fragment */
    ("constexpr_fexp2_fs_highp", ARITH),
    /* constexpr.fexp2 vs highp: 8 cases of shaders.constant_expressions.builtin_functions.exponential.exp2_float_vertex */
    ("constexpr_fexp2_vs_highp", ARITH),
    /* constexpr.flog2 vs highp: 12 cases of shaders.constant_expressions.builtin_functions.exponential.log2_float_vertex */
    ("constexpr_flog2_vs_highp", ARITH),
    /* constexpr.fsign vs highp: 4 cases of shaders.constant_expressions.builtin_functions.common.sign_float_vertex */
    /* constexpr.fsin fs highp: 9 cases of shaders.constant_expressions.builtin_functions.trigonometry.sin_float_fragment */
    ("constexpr_fsin_fs_highp", ARITH),
    /* constexpr.fsin vs highp: 9 cases of shaders.constant_expressions.builtin_functions.trigonometry.sin_float_vertex */
    ("constexpr_fsin_vs_highp", ARITH),
    /* indexing.dynamic_select fs none: 60 cases of shaders.indexing.tmp_array.float_const_write_dynamic_read_fragment */
    ("indexing_dynamic_select_fs_none", ARITH),
    /* isok_int_round fs mediump: 146 cases of shaders.swizzles.vector_swizzles.mediump_ivec2_gggg_vertex */
    ("isok_int_round_fs_mediump", ARITH),
    /* matrix.div fs highp: 18 cases of shaders.matrix.div.const_highp_mat2_float_fragment */
    /* matrix.div fs mediump: 36 cases of shaders.matrix.div.const_mediump_mat2_float_fragment */
    ("matrix_div_fs_mediump", ARITH),
    /* matrix.storeout fs highp: 11 cases of shaders.matrix.add.const_highp_mat3_float_fragment */
    ("matrix_storeout_fs_highp", OPERAND),
    /* matrix.storeout fs mediump: 18 cases of shaders.matrix.add.const_mediump_mat3_float_fragment */
    ("matrix_storeout_fs_mediump", OPERAND),
    /* op.angle.acos fs highp: 4 cases of shaders.operator.angle_and_trigonometry.acos.highp_float_fragment */
    ("op_angle_acos_fs_highp", ARITH),
    /* op.angle.acos fs mediump: 4 cases of shaders.operator.angle_and_trigonometry.acos.mediump_float_fragment */
    ("op_angle_acos_fs_mediump", ARITH),
    /* op.angle.asin fs highp: 4 cases of shaders.operator.angle_and_trigonometry.asin.highp_float_fragment */
    ("op_angle_asin_fs_highp", ARITH),
    /* op.angle.asin fs mediump: 4 cases of shaders.operator.angle_and_trigonometry.asin.mediump_float_fragment */
    ("op_angle_asin_fs_mediump", ARITH),
    /* op.angle.atan2 fs highp: 4 cases of shaders.operator.angle_and_trigonometry.atan2.highp_float_fragment */
    ("op_angle_atan2_fs_highp", ARITH),
    /* op.angle.atan2 fs mediump: 4 cases of shaders.operator.angle_and_trigonometry.atan2.mediump_float_fragment */
    ("op_angle_atan2_fs_mediump", ARITH),
    /* op.angle.atan fs highp: 4 cases of shaders.operator.angle_and_trigonometry.atan.highp_float_fragment */
    ("op_angle_atan_fs_highp", ARITH),
    /* op.angle.atan fs mediump: 4 cases of shaders.operator.angle_and_trigonometry.atan.mediump_float_fragment */
    ("op_angle_atan_fs_mediump", ARITH),
    /* op.angle.cos fs highp: 4 cases of shaders.operator.angle_and_trigonometry.cos.highp_float_fragment */
    ("op_angle_cos_fs_highp", ARITH),
    /* op.angle.cos fs mediump: 8 cases of shaders.operator.angle_and_trigonometry.cos.mediump_float_fragment */
    ("op_angle_cos_fs_mediump", ARITH),
    /* op.angle.cos vs highp: 4 cases of shaders.operator.angle_and_trigonometry.cos.highp_float_vertex */
    ("op_angle_cos_vs_highp", ARITH),
    /* op.angle.cos vs mediump: 8 cases of shaders.operator.angle_and_trigonometry.cos.mediump_float_vertex */
    ("op_angle_cos_vs_mediump", ARITH),
    /* op.angle.sin fs highp: 4 cases of shaders.operator.angle_and_trigonometry.sin.highp_float_fragment */
    ("op_angle_sin_fs_highp", ARITH),
    /* op.angle.sin fs mediump: 8 cases of shaders.operator.angle_and_trigonometry.sin.mediump_float_fragment */
    ("op_angle_sin_fs_mediump", ARITH),
    /* op.angle.sin vs highp: 4 cases of shaders.operator.angle_and_trigonometry.sin.highp_float_vertex */
    ("op_angle_sin_vs_highp", ARITH),
    /* op.angle.sin vs mediump: 8 cases of shaders.operator.angle_and_trigonometry.sin.mediump_float_vertex */
    ("op_angle_sin_vs_mediump", ARITH),
    /* op.angle.tan fs highp: 4 cases of shaders.operator.angle_and_trigonometry.tan.highp_float_fragment */
    ("op_angle_tan_fs_highp", ARITH),
    /* op.angle.tan fs mediump: 8 cases of shaders.operator.angle_and_trigonometry.tan.mediump_float_fragment */
    ("op_angle_tan_fs_mediump", ARITH),
    /* op.angle.tan vs highp: 4 cases of shaders.operator.angle_and_trigonometry.tan.highp_float_vertex */
    ("op_angle_tan_vs_highp", ARITH),
    /* op.angle.tan vs mediump: 8 cases of shaders.operator.angle_and_trigonometry.tan.mediump_float_vertex */
    ("op_angle_tan_vs_mediump", ARITH),
    /* op.binary.add_assign fs mediump: 8 cases of shaders.operator.binary_operator.add_assign_effect.mediump_vec3_float_fragment */
    ("op_binary_add_assign_fs_mediump", OPERAND),
    /* op.binary.add fs mediump: 8 cases of shaders.operator.binary_operator.add.mediump_float_vec3_fragment */
    ("op_binary_add_fs_mediump", OPERAND),
    /* op.binary.div_assign fs highp: 14 cases of shaders.operator.binary_operator.div_assign_effect.highp_float_fragment */
    ("op_binary_div_assign_fs_highp", ARITH),
    /* op.binary.div_assign fs mediump: 28 cases of shaders.operator.binary_operator.div_assign_effect.mediump_float_fragment */
    ("op_binary_div_assign_fs_mediump", ARITH),
    /* op.binary.div fs highp: 10 cases of shaders.operator.binary_operator.div.highp_float_fragment */
    ("op_binary_div_fs_highp", ARITH),
    /* op.binary.div fs mediump: 20 cases of shaders.operator.binary_operator.div.mediump_float_fragment */
    ("op_binary_div_fs_mediump", ARITH),
    /* op.binary.equal vs highp: 6 cases of shaders.operator.binary_operator.equal.highp_ivec2_vertex */
    ("op_binary_equal_vs_highp", ARITH),
    /* op.binary.equal vs mediump: 12 cases of shaders.operator.binary_operator.equal.mediump_ivec2_vertex */
    ("op_binary_equal_vs_mediump", ARITH),
    /* op.binary.equal vs none: 3 cases of shaders.operator.binary_operator.equal.bvec2_vertex */
    ("op_binary_equal_vs_none", ARITH),
    /* op.binary.mul_assign fs mediump: 4 cases of shaders.operator.binary_operator.mul_assign_effect.mediump_vec3_fragment */
    ("op_binary_mul_assign_fs_mediump", OPERAND),
    /* op.binary.mul fs mediump: 6 cases of shaders.operator.binary_operator.mul.mediump_float_vec3_fragment */
    ("op_binary_mul_fs_mediump", OPERAND),
    /* op.binary.not_equal vs highp: 6 cases of shaders.operator.binary_operator.not_equal.highp_ivec2_vertex */
    ("op_binary_not_equal_vs_highp", ARITH),
    /* op.binary.not_equal vs mediump: 12 cases of shaders.operator.binary_operator.not_equal.mediump_ivec2_vertex */
    ("op_binary_not_equal_vs_mediump", ARITH),
    /* op.binary.not_equal vs none: 3 cases of shaders.operator.binary_operator.not_equal.bvec2_vertex */
    ("op_binary_not_equal_vs_none", ARITH),
    /* op.binary.sub_assign fs mediump: 8 cases of shaders.operator.binary_operator.sub_assign_effect.mediump_vec3_float_fragment */
    ("op_binary_sub_assign_fs_mediump", OPERAND),
    /* op.binary.sub fs mediump: 8 cases of shaders.operator.binary_operator.sub.mediump_float_vec3_fragment */
    ("op_binary_sub_fs_mediump", OPERAND),
    /* op.bool.all vs none: 3 cases of shaders.operator.bool_compare.all.bvec2_vertex */
    ("op_bool_all_vs_none", ARITH),
    /* op.bool.any vs none: 3 cases of shaders.operator.bool_compare.any.bvec2_vertex */
    ("op_bool_any_vs_none", ARITH),
    /* op.common.abs fs mediump: 2 cases of shaders.operator.common_functions.abs.mediump_vec3_fragment */
    ("op_common_abs_fs_mediump", OPERAND),
    /* op.common.ceil fs highp: 4 cases of shaders.operator.common_functions.ceil.highp_float_fragment */
    ("op_common_ceil_fs_highp", ARITH),
    /* op.common.ceil vs highp: 4 cases of shaders.operator.common_functions.ceil.highp_float_vertex */
    ("op_common_ceil_vs_highp", ARITH),
    /* op.common.ceil vs mediump: 8 cases of shaders.operator.common_functions.ceil.mediump_float_vertex */
    ("op_common_ceil_vs_mediump", ARITH),
    /* op.common.clamp fs highp: 7 cases of shaders.operator.common_functions.clamp.highp_float_fragment */
    ("op_common_clamp_fs_highp", ARITH),
    /* op.common.clamp vs highp: 7 cases of shaders.operator.common_functions.clamp.highp_float_vertex */
    ("op_common_clamp_vs_highp", ARITH),
    /* op.common.clamp vs mediump: 14 cases of shaders.operator.common_functions.clamp.mediump_float_vertex */
    ("op_common_clamp_vs_mediump", ARITH),
    /* op.common.floor fs highp: 4 cases of shaders.operator.common_functions.floor.highp_float_fragment */
    ("op_common_floor_fs_highp", ARITH),
    /* op.common.max fs highp: 7 cases of shaders.operator.common_functions.max.highp_float_fragment */
    ("op_common_max_fs_highp", ARITH),
    /* op.common.max vs highp: 7 cases of shaders.operator.common_functions.max.highp_float_vertex */
    ("op_common_max_vs_highp", ARITH),
    /* op.common.max vs mediump: 14 cases of shaders.operator.common_functions.max.mediump_float_vertex */
    ("op_common_max_vs_mediump", ARITH),
    /* op.common.min fs highp: 7 cases of shaders.operator.common_functions.min.highp_float_fragment */
    ("op_common_min_fs_highp", ARITH),
    /* op.common.min vs highp: 7 cases of shaders.operator.common_functions.min.highp_float_vertex */
    ("op_common_min_vs_highp", ARITH),
    /* op.common.min vs mediump: 14 cases of shaders.operator.common_functions.min.mediump_float_vertex */
    ("op_common_min_vs_mediump", ARITH),
    /* op.common.mix fs mediump: 4 cases of shaders.operator.common_functions.mix.mediump_vec3_float_fragment */
    ("op_common_mix_fs_mediump", OPERAND),
    /* op.common.mod fs highp: 7 cases of shaders.operator.common_functions.mod.highp_float_fragment */
    ("op_common_mod_fs_highp", ARITH),
    /* op.common.mod fs mediump: 7 cases of shaders.operator.common_functions.mod.mediump_float_fragment */
    ("op_common_mod_fs_mediump", ARITH),
    /* op.common.sign fs highp: 4 cases of shaders.operator.common_functions.sign.highp_float_fragment */
    ("op_common_sign_fs_highp", ARITH),
    /* op.common.sign vs highp: 4 cases of shaders.operator.common_functions.sign.highp_float_vertex */
    ("op_common_sign_vs_highp", ARITH),
    /* op.common.sign vs mediump: 8 cases of shaders.operator.common_functions.sign.mediump_float_vertex */
    ("op_common_sign_vs_mediump", ARITH),
    /* op.common.smoothstep fs highp: 7 cases of shaders.operator.common_functions.smoothstep.highp_float_fragment */
    ("op_common_smoothstep_fs_highp", ARITH),
    /* op.common.smoothstep fs mediump: 14 cases of shaders.operator.common_functions.smoothstep.mediump_float_fragment */
    ("op_common_smoothstep_fs_mediump", ARITH),
    /* op.exponential.exp2 fs highp: 2 cases of shaders.operator.exponential.exp2.highp_vec3_fragment */
    ("op_exponential_exp2_fs_highp", OPERAND),
    /* op.exponential.exp2 fs mediump: 2 cases of shaders.operator.exponential.exp2.mediump_vec3_fragment */
    ("op_exponential_exp2_fs_mediump", OPERAND),
    /* op.exponential.exp2 vs highp: 4 cases of shaders.operator.exponential.exp2.highp_float_vertex */
    ("op_exponential_exp2_vs_highp", ARITH),
    /* op.exponential.exp2 vs mediump: 4 cases of shaders.operator.exponential.exp2.mediump_float_vertex */
    ("op_exponential_exp2_vs_mediump", ARITH),
    /* op.exponential.exp vs highp: 4 cases of shaders.operator.exponential.exp.highp_float_vertex */
    ("op_exponential_exp_vs_highp", ARITH),
    /* op.exponential.exp vs mediump: 4 cases of shaders.operator.exponential.exp.mediump_float_vertex */
    ("op_exponential_exp_vs_mediump", ARITH),
    /* op.exponential.log2 fs highp: 4 cases of shaders.operator.exponential.log2.highp_float_fragment */
    ("op_exponential_log2_fs_highp", ARITH),
    /* op.exponential.log2 fs mediump: 4 cases of shaders.operator.exponential.log2.mediump_float_fragment */
    ("op_exponential_log2_fs_mediump", ARITH),
    /* op.exponential.log2 vs highp: 4 cases of shaders.operator.exponential.log2.highp_float_vertex */
    ("op_exponential_log2_vs_highp", ARITH),
    /* op.exponential.log2 vs mediump: 4 cases of shaders.operator.exponential.log2.mediump_float_vertex */
    ("op_exponential_log2_vs_mediump", ARITH),
    /* op.exponential.log fs highp: 4 cases of shaders.operator.exponential.log.highp_float_fragment */
    ("op_exponential_log_fs_highp", ARITH),
    /* op.exponential.log fs mediump: 4 cases of shaders.operator.exponential.log.mediump_float_fragment */
    ("op_exponential_log_fs_mediump", ARITH),
    /* op.exponential.log vs highp: 4 cases of shaders.operator.exponential.log.highp_float_vertex */
    ("op_exponential_log_vs_highp", ARITH),
    /* op.exponential.log vs mediump: 4 cases of shaders.operator.exponential.log.mediump_float_vertex */
    ("op_exponential_log_vs_mediump", ARITH),
    /* op.exponential.pow fs highp: 4 cases of shaders.operator.exponential.pow.highp_float_fragment */
    ("op_exponential_pow_fs_highp", ARITH),
    /* op.exponential.pow fs mediump: 4 cases of shaders.operator.exponential.pow.mediump_float_fragment */
    ("op_exponential_pow_fs_mediump", ARITH),
    /* op.exponential.pow vs highp: 4 cases of shaders.operator.exponential.pow.highp_float_vertex */
    ("op_exponential_pow_vs_highp", ARITH),
    /* op.exponential.pow vs mediump: 4 cases of shaders.operator.exponential.pow.mediump_float_vertex */
    ("op_exponential_pow_vs_mediump", ARITH),
    /* op.geometric.cross fs mediump: 1 cases of shaders.operator.geometric.cross.mediump_vec3_fragment */
    ("op_geometric_cross_fs_mediump", OPERAND),
    /* op.geometric.faceforward fs highp: 4 cases of shaders.operator.geometric.faceforward.highp_float_fragment */
    ("op_geometric_faceforward_fs_highp", ARITH),
    /* op.geometric.faceforward fs mediump: 4 cases of shaders.operator.geometric.faceforward.mediump_float_fragment */
    ("op_geometric_faceforward_fs_mediump", ARITH),
    /* op.geometric.reflect fs highp: 1 cases of shaders.operator.geometric.reflect.highp_vec3_fragment */
    ("op_geometric_reflect_fs_highp", OPERAND),
    /* op.geometric.reflect fs mediump: 1 cases of shaders.operator.geometric.reflect.mediump_vec3_fragment */
    ("op_geometric_reflect_fs_mediump", OPERAND),
    /* op.geometric.refract fs highp: 4 cases of shaders.operator.geometric.refract.highp_float_float_fragment */
    ("op_geometric_refract_fs_highp", ARITH),
    /* op.geometric.refract fs mediump: 4 cases of shaders.operator.geometric.refract.mediump_float_float_fragment */
    ("op_geometric_refract_fs_mediump", ARITH),
    /* op.selection.select fs highp: 8 cases of shaders.operator.selection.highp_float_fragment */
    ("op_selection_select_fs_highp", ARITH),
    /* op.selection.select fs mediump: 16 cases of shaders.operator.selection.mediump_float_fragment */
    ("op_selection_select_fs_mediump", ARITH),
    /* op.selection.select fs none: 4 cases of shaders.operator.selection.bool_fragment */
    ("op_selection_select_fs_none", ARITH),
    /* op.unary.incdec fs mediump: 12 cases of shaders.operator.unary_operator.post_decrement_effect.mediump_vec3_fragment */
    ("op_unary_incdec_fs_mediump", OPERAND),
    /* op.unary.unary_minus fs mediump: 2 cases of shaders.operator.unary_operator.minus.mediump_vec3_fragment */
    ("op_unary_unary_minus_fs_mediump", OPERAND),
    /* other.ball vs none: 4 cases of fragment_ops.interaction.basic_shader.2 */
    ("other_ball_vs_none", ARITH),
    /* other.bany vs none: 4 cases of fragment_ops.interaction.basic_shader.10 */
    ("other_bany_vs_none", ARITH),
    /* other.bcsel fs mediump: 4 cases of shaders.functions.array_arguments.copy_global_in_on_call_fragment */
    ("other_bcsel_fs_mediump", ARITH),
    /* other.bcsel fs none: 4 cases of shaders.algorithm.hsl_to_rgb_fragment */
    ("other_bcsel_fs_none", ARITH),
    /* other.ffloor fs none: 14 cases of shaders.builtin_variable.max_combined_texture_image_units_fragment */
    ("other_ffloor_fs_none", ARITH),
    /* other.fmax vs mediump: 5 cases of shaders.functions.array_arguments.copy_global_in_on_call_vertex */
    ("other_fmax_vs_mediump", ARITH),
    /* other.frcp fs none: 3 cases of shaders.builtin_variable.fragcoord_w */
    ("other_frcp_fs_none", ARITH),
    /* other.fsin fs none: 5 cases of flush_finish.finish */
    ("other_fsin_fs_none", ARITH),
    /* other.storeoutput fs mediump: 10 cases of shaders.conversions.matrix_combine.bool_float_int_vec2_bool_bvec2_float_to_mat3_fragment */
    ("other_storeoutput_fs_mediump", OPERAND),
    /* other.storeoutput fs none: 9 cases of attribute_location.bind.vec2 */
    ("other_storeoutput_fs_none", OPERAND),
    /* other.x vs mediump: 3 cases of shaders.functions.misc.multi_arg_float_vertex */
    ("other_x_vs_mediump", CHAIN),
    /* random.bany vs none: 22 cases of shaders.random.all_features.vertex.30 */
    ("random_bany_vs_none", ARITH),
    /* random.fcos fs mediump: 13 cases of shaders.random.all_features.fragment.41 */
    ("random_fcos_fs_mediump", ARITH),
    /* random.fexp2 fs mediump: 33 cases of shaders.random.all_features.fragment.17 */
    ("random_fexp2_fs_mediump", ARITH),
    /* random.fexp2 vs none: 46 cases of shaders.random.all_features.vertex.11 */
    ("random_fexp2_vs_none", ARITH),
    /* random.flog2 fs mediump: 41 cases of shaders.random.all_features.fragment.1 */
    ("random_flog2_fs_mediump", ARITH),
    /* random.flog2 vs none: 24 cases of shaders.random.all_features.vertex.20 */
    ("random_flog2_vs_none", ARITH),
    /* random.fne fs mediump: 19 cases of shaders.random.all_features.fragment.29 */
    ("random_fne_fs_mediump", ARITH),
    /* random.frcp fs mediump: 15 cases of shaders.random.texture.fragment.1 */
    ("random_frcp_fs_mediump", ARITH),
    /* random.frsq fs mediump: 10 cases of shaders.random.all_features.fragment.35 */
    ("random_frsq_fs_mediump", ARITH),
    /* random.fsin fs mediump: 35 cases of shaders.random.all_features.fragment.18 */
    ("random_fsin_fs_mediump", ARITH),
    /* random.fsin vs none: 22 cases of shaders.random.all_features.vertex.72 */
    ("random_fsin_vs_none", ARITH),
    /* random.fsqrt fs mediump: 34 cases of shaders.random.all_features.fragment.0 */
    ("random_fsqrt_fs_mediump", ARITH),
    /* random.storeoutput fs mediump: 31 cases of shaders.random.basic_expression.combined.11 */
    ("random_storeoutput_fs_mediump", OPERAND),
    /* uniform_api.bool_compare vs none: 179 cases of uniform_api.value.assigned.by_pointer.get_uniform.basic.bvec2_api_float_both */
    ("uniform_api_bool_compare_vs_none", ARITH),
];

#[test]
#[ignore = "refused: arith (constexpr.ball, fs, highp; 3 cases)"]
fn constexpr_ball_fs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_ball_fs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.ball, vs, highp; 3 cases)"]
fn constexpr_ball_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_ball_vs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.bany, fs, highp; 3 cases)"]
fn constexpr_bany_fs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_bany_fs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.bany, vs, highp; 3 cases)"]
fn constexpr_bany_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_bany_vs_highp");
}

#[test]
fn constexpr_fceil_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fceil_vs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.fcos, fs, highp; 4 cases)"]
fn constexpr_fcos_fs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fcos_fs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.fcos, vs, highp; 4 cases)"]
fn constexpr_fcos_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fcos_vs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.fexp2, fs, highp; 8 cases)"]
fn constexpr_fexp2_fs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fexp2_fs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.fexp2, vs, highp; 8 cases)"]
fn constexpr_fexp2_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fexp2_vs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.flog2, vs, highp; 12 cases)"]
fn constexpr_flog2_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_flog2_vs_highp");
}

#[test]
fn constexpr_fsign_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fsign_vs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.fsin, fs, highp; 9 cases)"]
fn constexpr_fsin_fs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fsin_fs_highp");
}

#[test]
#[ignore = "refused: arith (constexpr.fsin, vs, highp; 9 cases)"]
fn constexpr_fsin_vs_highp() {
    mali_ref::assert_matches("deqp_arith_constexpr_fsin_vs_highp");
}

#[test]
#[ignore = "refused: arith (indexing.dynamic_select, fs, none; 60 cases)"]
fn indexing_dynamic_select_fs_none() {
    mali_ref::assert_matches("deqp_arith_indexing_dynamic_select_fs_none");
}

#[test]
#[ignore = "refused: arith (isok_int_round, fs, mediump; 146 cases)"]
fn isok_int_round_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_isok_int_round_fs_mediump");
}

#[test]
fn matrix_div_fs_highp() {
    mali_ref::assert_matches("deqp_arith_matrix_div_fs_highp");
}

#[test]
#[ignore = "refused: arith (matrix.div, fs, mediump; 36 cases)"]
fn matrix_div_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_matrix_div_fs_mediump");
}

#[test]
#[ignore = "refused: operand (matrix.storeout, fs, highp; 11 cases)"]
fn matrix_storeout_fs_highp() {
    mali_ref::assert_matches("deqp_arith_matrix_storeout_fs_highp");
}

#[test]
#[ignore = "refused: operand (matrix.storeout, fs, mediump; 18 cases)"]
fn matrix_storeout_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_matrix_storeout_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.acos, fs, highp; 4 cases)"]
fn op_angle_acos_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_acos_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.acos, fs, mediump; 4 cases)"]
fn op_angle_acos_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_acos_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.asin, fs, highp; 4 cases)"]
fn op_angle_asin_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_asin_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.asin, fs, mediump; 4 cases)"]
fn op_angle_asin_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_asin_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.atan2, fs, highp; 4 cases)"]
fn op_angle_atan2_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_atan2_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.atan2, fs, mediump; 4 cases)"]
fn op_angle_atan2_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_atan2_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.atan, fs, highp; 4 cases)"]
fn op_angle_atan_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_atan_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.atan, fs, mediump; 4 cases)"]
fn op_angle_atan_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_atan_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.cos, fs, highp; 4 cases)"]
fn op_angle_cos_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_cos_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.cos, fs, mediump; 8 cases)"]
fn op_angle_cos_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_cos_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.cos, vs, highp; 4 cases)"]
fn op_angle_cos_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_cos_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.cos, vs, mediump; 8 cases)"]
fn op_angle_cos_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_cos_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.sin, fs, highp; 4 cases)"]
fn op_angle_sin_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_sin_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.sin, fs, mediump; 8 cases)"]
fn op_angle_sin_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_sin_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.sin, vs, highp; 4 cases)"]
fn op_angle_sin_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_sin_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.sin, vs, mediump; 8 cases)"]
fn op_angle_sin_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_sin_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.tan, fs, highp; 4 cases)"]
fn op_angle_tan_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_tan_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.tan, fs, mediump; 8 cases)"]
fn op_angle_tan_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_tan_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.angle.tan, vs, highp; 4 cases)"]
fn op_angle_tan_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_angle_tan_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.angle.tan, vs, mediump; 8 cases)"]
fn op_angle_tan_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_angle_tan_vs_mediump");
}

#[test]
#[ignore = "refused: operand (op.binary.add_assign, fs, mediump; 8 cases)"]
fn op_binary_add_assign_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_add_assign_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.binary.add, fs, mediump; 8 cases)"]
fn op_binary_add_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_add_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.binary.div_assign, fs, highp; 14 cases)"]
fn op_binary_div_assign_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_binary_div_assign_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.binary.div_assign, fs, mediump; 28 cases)"]
fn op_binary_div_assign_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_div_assign_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.binary.div, fs, highp; 10 cases)"]
fn op_binary_div_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_binary_div_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.binary.div, fs, mediump; 20 cases)"]
fn op_binary_div_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_div_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.binary.equal, vs, highp; 6 cases)"]
fn op_binary_equal_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_binary_equal_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.binary.equal, vs, mediump; 12 cases)"]
fn op_binary_equal_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_equal_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.binary.equal, vs, none; 3 cases)"]
fn op_binary_equal_vs_none() {
    mali_ref::assert_matches("deqp_arith_op_binary_equal_vs_none");
}

#[test]
#[ignore = "refused: operand (op.binary.mul_assign, fs, mediump; 4 cases)"]
fn op_binary_mul_assign_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_mul_assign_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.binary.mul, fs, mediump; 6 cases)"]
fn op_binary_mul_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_mul_fs_mediump");
}

#[test]
fn op_binary_not_equal_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_binary_not_equal_fs_highp");
}

#[test]
fn op_binary_not_equal_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_not_equal_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.binary.not_equal, vs, highp; 6 cases)"]
fn op_binary_not_equal_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_binary_not_equal_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.binary.not_equal, vs, mediump; 12 cases)"]
fn op_binary_not_equal_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_not_equal_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.binary.not_equal, vs, none; 3 cases)"]
fn op_binary_not_equal_vs_none() {
    mali_ref::assert_matches("deqp_arith_op_binary_not_equal_vs_none");
}

#[test]
#[ignore = "refused: operand (op.binary.sub_assign, fs, mediump; 8 cases)"]
fn op_binary_sub_assign_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_sub_assign_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.binary.sub, fs, mediump; 8 cases)"]
fn op_binary_sub_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_binary_sub_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.bool.all, vs, none; 3 cases)"]
fn op_bool_all_vs_none() {
    mali_ref::assert_matches("deqp_arith_op_bool_all_vs_none");
}

#[test]
#[ignore = "refused: arith (op.bool.any, vs, none; 3 cases)"]
fn op_bool_any_vs_none() {
    mali_ref::assert_matches("deqp_arith_op_bool_any_vs_none");
}

#[test]
#[ignore = "refused: operand (op.common.abs, fs, mediump; 2 cases)"]
fn op_common_abs_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_abs_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.ceil, fs, highp; 4 cases)"]
fn op_common_ceil_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_ceil_fs_highp");
}

#[test]
fn op_common_ceil_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_ceil_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.ceil, vs, highp; 4 cases)"]
fn op_common_ceil_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_ceil_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.ceil, vs, mediump; 8 cases)"]
fn op_common_ceil_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_ceil_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.clamp, fs, highp; 7 cases)"]
fn op_common_clamp_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_clamp_fs_highp");
}

#[test]
fn op_common_clamp_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_clamp_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.clamp, vs, highp; 7 cases)"]
fn op_common_clamp_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_clamp_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.clamp, vs, mediump; 14 cases)"]
fn op_common_clamp_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_clamp_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.floor, fs, highp; 4 cases)"]
fn op_common_floor_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_floor_fs_highp");
}

#[test]
fn op_common_floor_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_floor_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.max, fs, highp; 7 cases)"]
fn op_common_max_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_max_fs_highp");
}

#[test]
fn op_common_max_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_max_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.max, vs, highp; 7 cases)"]
fn op_common_max_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_max_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.max, vs, mediump; 14 cases)"]
fn op_common_max_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_max_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.min, fs, highp; 7 cases)"]
fn op_common_min_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_min_fs_highp");
}

#[test]
fn op_common_min_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_min_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.min, vs, highp; 7 cases)"]
fn op_common_min_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_min_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.min, vs, mediump; 14 cases)"]
fn op_common_min_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_min_vs_mediump");
}

#[test]
#[ignore = "refused: operand (op.common.mix, fs, mediump; 4 cases)"]
fn op_common_mix_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_mix_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.mod, fs, highp; 7 cases)"]
fn op_common_mod_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_mod_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.mod, fs, mediump; 7 cases)"]
fn op_common_mod_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_mod_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.sign, fs, highp; 4 cases)"]
fn op_common_sign_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_sign_fs_highp");
}

#[test]
fn op_common_sign_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_sign_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.sign, vs, highp; 4 cases)"]
fn op_common_sign_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_sign_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.sign, vs, mediump; 8 cases)"]
fn op_common_sign_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_sign_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.common.smoothstep, fs, highp; 7 cases)"]
fn op_common_smoothstep_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_common_smoothstep_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.common.smoothstep, fs, mediump; 14 cases)"]
fn op_common_smoothstep_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_common_smoothstep_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.exponential.exp2, fs, highp; 2 cases)"]
fn op_exponential_exp2_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_exp2_fs_highp");
}

#[test]
#[ignore = "refused: operand (op.exponential.exp2, fs, mediump; 2 cases)"]
fn op_exponential_exp2_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_exp2_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.exp2, vs, highp; 4 cases)"]
fn op_exponential_exp2_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_exp2_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.exp2, vs, mediump; 4 cases)"]
fn op_exponential_exp2_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_exp2_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.exp, vs, highp; 4 cases)"]
fn op_exponential_exp_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_exp_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.exp, vs, mediump; 4 cases)"]
fn op_exponential_exp_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_exp_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.log2, fs, highp; 4 cases)"]
fn op_exponential_log2_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log2_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.log2, fs, mediump; 4 cases)"]
fn op_exponential_log2_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log2_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.log2, vs, highp; 4 cases)"]
fn op_exponential_log2_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log2_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.log2, vs, mediump; 4 cases)"]
fn op_exponential_log2_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log2_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.log, fs, highp; 4 cases)"]
fn op_exponential_log_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.log, fs, mediump; 4 cases)"]
fn op_exponential_log_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.log, vs, highp; 4 cases)"]
fn op_exponential_log_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.log, vs, mediump; 4 cases)"]
fn op_exponential_log_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_log_vs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.pow, fs, highp; 4 cases)"]
fn op_exponential_pow_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_pow_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.pow, fs, mediump; 4 cases)"]
fn op_exponential_pow_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_pow_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.exponential.pow, vs, highp; 4 cases)"]
fn op_exponential_pow_vs_highp() {
    mali_ref::assert_matches("deqp_arith_op_exponential_pow_vs_highp");
}

#[test]
#[ignore = "refused: arith (op.exponential.pow, vs, mediump; 4 cases)"]
fn op_exponential_pow_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_exponential_pow_vs_mediump");
}

#[test]
fn op_float_notequal_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_float_notequal_fs_highp");
}

#[test]
fn op_float_notequal_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_float_notequal_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.geometric.cross, fs, mediump; 1 cases)"]
fn op_geometric_cross_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_geometric_cross_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.geometric.faceforward, fs, highp; 4 cases)"]
fn op_geometric_faceforward_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_geometric_faceforward_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.geometric.faceforward, fs, mediump; 4 cases)"]
fn op_geometric_faceforward_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_geometric_faceforward_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.geometric.reflect, fs, highp; 1 cases)"]
fn op_geometric_reflect_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_geometric_reflect_fs_highp");
}

#[test]
#[ignore = "refused: operand (op.geometric.reflect, fs, mediump; 1 cases)"]
fn op_geometric_reflect_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_geometric_reflect_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.geometric.refract, fs, highp; 4 cases)"]
fn op_geometric_refract_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_geometric_refract_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.geometric.refract, fs, mediump; 4 cases)"]
fn op_geometric_refract_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_geometric_refract_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.selection.select, fs, highp; 8 cases)"]
fn op_selection_select_fs_highp() {
    mali_ref::assert_matches("deqp_arith_op_selection_select_fs_highp");
}

#[test]
#[ignore = "refused: arith (op.selection.select, fs, mediump; 16 cases)"]
fn op_selection_select_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_selection_select_fs_mediump");
}

#[test]
#[ignore = "refused: arith (op.selection.select, fs, none; 4 cases)"]
fn op_selection_select_fs_none() {
    mali_ref::assert_matches("deqp_arith_op_selection_select_fs_none");
}

#[test]
#[ignore = "refused: operand (op.unary.incdec, fs, mediump; 12 cases)"]
fn op_unary_incdec_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_unary_incdec_fs_mediump");
}

#[test]
#[ignore = "refused: operand (op.unary.unary_minus, fs, mediump; 2 cases)"]
fn op_unary_unary_minus_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_op_unary_unary_minus_fs_mediump");
}

#[test]
#[ignore = "refused: arith (other.ball, vs, none; 4 cases)"]
fn other_ball_vs_none() {
    mali_ref::assert_matches("deqp_arith_other_ball_vs_none");
}

#[test]
#[ignore = "refused: arith (other.bany, vs, none; 4 cases)"]
fn other_bany_vs_none() {
    mali_ref::assert_matches("deqp_arith_other_bany_vs_none");
}

#[test]
#[ignore = "refused: arith (other.bcsel, fs, mediump; 4 cases)"]
fn other_bcsel_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_other_bcsel_fs_mediump");
}

#[test]
#[ignore = "refused: arith (other.bcsel, fs, none; 4 cases)"]
fn other_bcsel_fs_none() {
    mali_ref::assert_matches("deqp_arith_other_bcsel_fs_none");
}

#[test]
#[ignore = "refused: arith (other.ffloor, fs, none; 14 cases)"]
fn other_ffloor_fs_none() {
    mali_ref::assert_matches("deqp_arith_other_ffloor_fs_none");
}

#[test]
#[ignore = "refused: arith (other.fmax, vs, mediump; 5 cases)"]
fn other_fmax_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_other_fmax_vs_mediump");
}

#[test]
#[ignore = "refused: arith (other.frcp, fs, none; 3 cases)"]
fn other_frcp_fs_none() {
    mali_ref::assert_matches("deqp_arith_other_frcp_fs_none");
}

#[test]
#[ignore = "refused: arith (other.fsin, fs, none; 5 cases)"]
fn other_fsin_fs_none() {
    mali_ref::assert_matches("deqp_arith_other_fsin_fs_none");
}

#[test]
#[ignore = "refused: operand (other.storeoutput, fs, mediump; 10 cases)"]
fn other_storeoutput_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_other_storeoutput_fs_mediump");
}

#[test]
#[ignore = "refused: operand (other.storeoutput, fs, none; 9 cases)"]
fn other_storeoutput_fs_none() {
    mali_ref::assert_matches("deqp_arith_other_storeoutput_fs_none");
}

#[test]
#[ignore = "refused: chain (other.x, vs, mediump; 3 cases)"]
fn other_x_vs_mediump() {
    mali_ref::assert_matches("deqp_arith_other_x_vs_mediump");
}

#[test]
#[ignore = "refused: arith (random.bany, vs, none; 22 cases)"]
fn random_bany_vs_none() {
    mali_ref::assert_matches("deqp_arith_random_bany_vs_none");
}

#[test]
#[ignore = "refused: arith (random.fcos, fs, mediump; 13 cases)"]
fn random_fcos_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_fcos_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.fexp2, fs, mediump; 33 cases)"]
fn random_fexp2_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_fexp2_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.fexp2, vs, none; 46 cases)"]
fn random_fexp2_vs_none() {
    mali_ref::assert_matches("deqp_arith_random_fexp2_vs_none");
}

#[test]
#[ignore = "refused: arith (random.flog2, fs, mediump; 41 cases)"]
fn random_flog2_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_flog2_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.flog2, vs, none; 24 cases)"]
fn random_flog2_vs_none() {
    mali_ref::assert_matches("deqp_arith_random_flog2_vs_none");
}

#[test]
#[ignore = "refused: arith (random.fne, fs, mediump; 19 cases)"]
fn random_fne_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_fne_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.frcp, fs, mediump; 15 cases)"]
fn random_frcp_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_frcp_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.frsq, fs, mediump; 10 cases)"]
fn random_frsq_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_frsq_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.fsin, fs, mediump; 35 cases)"]
fn random_fsin_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_fsin_fs_mediump");
}

#[test]
#[ignore = "refused: arith (random.fsin, vs, none; 22 cases)"]
fn random_fsin_vs_none() {
    mali_ref::assert_matches("deqp_arith_random_fsin_vs_none");
}

#[test]
#[ignore = "refused: arith (random.fsqrt, fs, mediump; 34 cases)"]
fn random_fsqrt_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_fsqrt_fs_mediump");
}

#[test]
#[ignore = "refused: operand (random.storeoutput, fs, mediump; 31 cases)"]
fn random_storeoutput_fs_mediump() {
    mali_ref::assert_matches("deqp_arith_random_storeoutput_fs_mediump");
}

#[test]
#[ignore = "refused: arith (uniform_api.bool_compare, vs, none; 179 cases)"]
fn uniform_api_bool_compare_vs_none() {
    mali_ref::assert_matches("deqp_arith_uniform_api_bool_compare_vs_none");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, and the three lists cover every `deqp_arith_*`
/// reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_arith_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_arith_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_arith_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to LOWERED or DIFFERS")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_arith_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
