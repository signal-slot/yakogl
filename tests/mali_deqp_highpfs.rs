// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_highpfs_*` (plan #387, item
//! highpfs-refs, `docs/notes/mali-highpfs-lowering.md`): the four groups of
//! train 1/2 steps that only the notes describe, the highp fragment shapes
//! (`hp_*`, arith steps 1a/1b/3 at highp), step 4a's pair trees (`pt_*`,
//! mediump `+ - *`, their assignments and `min max clamp`), the
//! `mat4_pair_unread` siblings (`m4_*`) and the `load_mat3` siblings (`l3_*`)
//! of `docs/notes/mali-deqp-irindex.md` 5.2 and 5.3. Each is the dEQP-GLES2
//! case its `.license` names, its sources generated through the dEQP
//! template (VK-GL-CTS 93bca01) as the note says, drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610 drm-shim,
//! `--uniform-values`). `tests/mali_ref` does the compare.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). One that compiles with other
//! words than Mesa's is in [`DIFFERS`], its byte-compare `#[ignore]`d too. One
//! that compiles byte-equal is in [`LOWERED`] and its test runs. The lowering
//! step that lifts a reference moves it in the same commit, drops its
//! `#[ignore]` and updates the note. `references_parse` checks every file and
//! the fragment word count the note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_highpfs
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal at the tip, without the
/// `deqp_highpfs_` prefix (the pair-tree forms step 4a already covers).
const LOWERED: &[&str] =
    &["pt_add_assign_vec3", "pt_add_vec3", "pt_mul_float_vec4", "pt_sub_assign_vec3", "pt_sub_vec3"];

/// The references that compile with other words than Mesa's (hazards): the
/// mediump `vec3 * float` product, whose halves are paired and placed in
/// another order (the note, step PT3).
const DIFFERS: &[&str] = &["pt_mul_assign_vec3_float", "pt_mul_vec3_float"];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("hp_cos_vec3", ARITH),
    ("hp_exp2_float", FORM),
    ("hp_exp2_vec2", FORM),
    ("hp_exp_float", ARITH),
    ("hp_exp_vec3", ARITH),
    ("hp_log2_vec3", ARITH),
    ("hp_log_vec3", ARITH),
    ("hp_mod_vec3", FORM),
    ("hp_mod_vec3_float", LOAD),
    ("hp_pow_vec3", ARITH),
    ("hp_sign_vec3", ARITH),
    ("hp_sin_vec2", ARITH),
    ("hp_sin_vec3", ARITH),
    ("hp_sin_vec4", ARITH),
    ("hp_smoothstep_float_float_vec3", LOAD),
    ("hp_smoothstep_vec3", FORM),
    ("hp_tan_vec3", ARITH),
    ("l3_float_mat3_fs", IR_INDEX),
    ("l3_float_to_mat3_vs", IR_INDEX),
    ("l3_mat3_to_mat2_fs", IR_INDEX),
    ("l3_mat3_to_mat3_vs", IR_INDEX),
    ("l3_mat3_to_mat4_fs", IR_INDEX),
    ("l3_vec3x3_to_mat3_fs", IR_INDEX),
    ("l3_vec3x3_to_mat3_vs", IR_INDEX),
    ("m4_add_lowp_float", IR_INDEX),
    ("m4_mul_float", ALGEBRAIC),
    ("m4_postdec", IR_INDEX),
    ("m4_postinc", IR_INDEX),
    ("m4_sub_float", IR_INDEX),
    ("pt_add_float_vec2", FORM),
    ("pt_add_vec2", FORM),
    ("pt_add_vec3_float", OPERAND),
    ("pt_add_vec4", FORM),
    ("pt_add_vec4_float", FORM),
    ("pt_clamp_vec2_float_float", FORM),
    ("pt_clamp_vec3", OPERAND),
    ("pt_max_vec4", FORM),
    ("pt_min_vec3", OPERAND),
    ("pt_min_vec3_float", OPERAND),
    ("pt_mul_assign_vec3", OPERAND),
    ("pt_mul_vec3", OPERAND),
    ("pt_mul_vec4", FORM),
    ("pt_sub_float_vec4", OPERAND),
];

/// `MaliError::Arithmetic`.
const ARITH: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";
/// `MaliError::ColourForm`.
const FORM: &str = "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
/// `MaliError::FsOperandForm`.
const OPERAND: &str = "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet";
/// `MaliError::FsVaryingLoad`.
const LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";
/// `MaliError::AlgebraicForm`.
const ALGEBRAIC: &str = "Valhall: a shape Mesa's algebraic rules rewrite without a reference (max(x, 0.0), clamp(x, -1.0, 1.0), a negated float(bool) in a product) is not lowered yet";
/// `MaliError::Defect(IrIndex)`.
const IR_INDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("hp_cos_vec3", 39),
    ("hp_exp2_float", 12),
    ("hp_exp2_vec2", 18),
    ("hp_exp_float", 13),
    ("hp_exp_vec3", 25),
    ("hp_log2_vec3", 25),
    ("hp_log_vec3", 30),
    ("hp_mod_vec3", 28),
    ("hp_mod_vec3_float", 23),
    ("hp_pow_vec3", 35),
    ("hp_sign_vec3", 21),
    ("hp_sin_vec2", 29),
    ("hp_sin_vec3", 39),
    ("hp_sin_vec4", 52),
    ("hp_smoothstep_float_float_vec3", 32),
    ("hp_smoothstep_vec3", 35),
    ("hp_tan_vec3", 54),
    ("l3_float_mat3_fs", 29),
    ("l3_float_to_mat3_vs", 30),
    ("l3_mat3_to_mat2_fs", 20),
    ("l3_mat3_to_mat3_vs", 30),
    ("l3_mat3_to_mat4_fs", 41),
    ("l3_vec3x3_to_mat3_fs", 30),
    ("l3_vec3x3_to_mat3_vs", 30),
    ("m4_add_lowp_float", 37),
    ("m4_mul_float", 31),
    ("m4_postdec", 48),
    ("m4_postinc", 48),
    ("m4_sub_float", 37),
    ("pt_add_assign_vec3", 20),
    ("pt_add_float_vec2", 13),
    ("pt_add_vec2", 15),
    ("pt_add_vec3", 17),
    ("pt_add_vec3_float", 13),
    ("pt_add_vec4", 17),
    ("pt_add_vec4_float", 14),
    ("pt_clamp_vec2_float_float", 19),
    ("pt_clamp_vec3", 26),
    ("pt_max_vec4", 22),
    ("pt_min_vec3", 20),
    ("pt_min_vec3_float", 15),
    ("pt_mul_assign_vec3", 20),
    ("pt_mul_assign_vec3_float", 15),
    ("pt_mul_float_vec4", 15),
    ("pt_mul_vec3", 17),
    ("pt_mul_vec3_float", 13),
    ("pt_mul_vec4", 17),
    ("pt_sub_assign_vec3", 20),
    ("pt_sub_float_vec4", 15),
    ("pt_sub_vec3", 17),
];

/// `dEQP-GLES2.functional.shaders.operator.angle_and_trigonometry.cos.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_cos_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_cos_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.exp2.highp_float_fragment` (step HF2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_exp2_float() {
    mali_ref::assert_matches("deqp_highpfs_hp_exp2_float");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.exp2.highp_vec2_fragment` (step HF2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_exp2_vec2() {
    mali_ref::assert_matches("deqp_highpfs_hp_exp2_vec2");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.exp.highp_float_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_exp_float() {
    mali_ref::assert_matches("deqp_highpfs_hp_exp_float");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.exp.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_exp_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_exp_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.log2.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_log2_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_log2_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.log.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_log_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_log_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.mod.highp_vec3_fragment` (step HF2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_mod_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_mod_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.mod.highp_vec3_float_fragment` (step HF2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_mod_vec3_float() {
    mali_ref::assert_matches("deqp_highpfs_hp_mod_vec3_float");
}

/// `dEQP-GLES2.functional.shaders.operator.exponential.pow.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_pow_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_pow_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.sign.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sign_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_sign_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.angle_and_trigonometry.sin.highp_vec2_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sin_vec2() {
    mali_ref::assert_matches("deqp_highpfs_hp_sin_vec2");
}

/// `dEQP-GLES2.functional.shaders.operator.angle_and_trigonometry.sin.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sin_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_sin_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.angle_and_trigonometry.sin.highp_vec4_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sin_vec4() {
    mali_ref::assert_matches("deqp_highpfs_hp_sin_vec4");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.smoothstep.highp_float_vec3_fragment` (step HF2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_smoothstep_float_float_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_smoothstep_float_float_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.smoothstep.highp_vec3_fragment` (step HF2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_smoothstep_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_smoothstep_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.angle_and_trigonometry.tan.highp_vec3_fragment` (step HF1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_tan_vec3() {
    mali_ref::assert_matches("deqp_highpfs_hp_tan_vec3");
}

/// `dEQP-GLES2.functional.shaders.functions.datatypes.float_mat3_fragment` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_float_mat3_fs() {
    mali_ref::assert_matches("deqp_highpfs_l3_float_mat3_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.float_to_mat3_vertex` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_float_to_mat3_vs() {
    mali_ref::assert_matches("deqp_highpfs_l3_float_to_mat3_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.matrix_to_matrix.mat3_to_mat2_fragment` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_mat3_to_mat2_fs() {
    mali_ref::assert_matches("deqp_highpfs_l3_mat3_to_mat2_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.matrix_to_matrix.mat3_to_mat3_vertex` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_mat3_to_mat3_vs() {
    mali_ref::assert_matches("deqp_highpfs_l3_mat3_to_mat3_vs");
}

/// `dEQP-GLES2.functional.shaders.conversions.matrix_to_matrix.mat3_to_mat4_fragment` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_mat3_to_mat4_fs() {
    mali_ref::assert_matches("deqp_highpfs_l3_mat3_to_mat4_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.matrix_combine.vec3_vec3_vec3_to_mat3_fragment` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_vec3x3_to_mat3_fs() {
    mali_ref::assert_matches("deqp_highpfs_l3_vec3x3_to_mat3_fs");
}

/// `dEQP-GLES2.functional.shaders.conversions.matrix_combine.vec3_vec3_vec3_to_mat3_vertex` (step L3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn l3_vec3x3_to_mat3_vs() {
    mali_ref::assert_matches("deqp_highpfs_l3_vec3x3_to_mat3_vs");
}

/// `dEQP-GLES2.functional.shaders.matrix.add.dynamic_lowp_mat4_float_fragment` (step M4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn m4_add_lowp_float() {
    mali_ref::assert_matches("deqp_highpfs_m4_add_lowp_float");
}

/// `dEQP-GLES2.functional.shaders.matrix.mul.dynamic_mediump_mat4_float_fragment` (step M4 (next: constprod K8)).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn m4_mul_float() {
    mali_ref::assert_matches("deqp_highpfs_m4_mul_float");
}

/// `dEQP-GLES2.functional.shaders.matrix.post_decrement.dynamic_mediump_mat4_fragment` (step M4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn m4_postdec() {
    mali_ref::assert_matches("deqp_highpfs_m4_postdec");
}

/// `dEQP-GLES2.functional.shaders.matrix.post_increment.dynamic_mediump_mat4_fragment` (step M4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn m4_postinc() {
    mali_ref::assert_matches("deqp_highpfs_m4_postinc");
}

/// `dEQP-GLES2.functional.shaders.matrix.sub.dynamic_mediump_mat4_float_fragment` (step M4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn m4_sub_float() {
    mali_ref::assert_matches("deqp_highpfs_m4_sub_float");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add_assign_effect.mediump_vec3_fragment` (step PT0).
#[test]
fn pt_add_assign_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_assign_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add.mediump_float_vec2_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_add_float_vec2() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_float_vec2");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add.mediump_vec2_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_add_vec2() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_vec2");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add.mediump_vec3_fragment` (step PT0).
#[test]
fn pt_add_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add.mediump_vec3_float_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_add_vec3_float() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_vec3_float");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add.mediump_vec4_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_add_vec4() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_vec4");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.add.mediump_vec4_float_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_add_vec4_float() {
    mali_ref::assert_matches("deqp_highpfs_pt_add_vec4_float");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.clamp.mediump_vec2_float_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_clamp_vec2_float_float() {
    mali_ref::assert_matches("deqp_highpfs_pt_clamp_vec2_float_float");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.clamp.mediump_vec3_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_clamp_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_clamp_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.max.mediump_vec4_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_max_vec4() {
    mali_ref::assert_matches("deqp_highpfs_pt_max_vec4");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.min.mediump_vec3_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_min_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_min_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.common_functions.min.mediump_vec3_float_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_min_vec3_float() {
    mali_ref::assert_matches("deqp_highpfs_pt_min_vec3_float");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul_assign_effect.mediump_vec3_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_mul_assign_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_mul_assign_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul_assign_effect.mediump_vec3_float_fragment` (step PT3).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by refusals_pinned"]
fn pt_mul_assign_vec3_float() {
    mali_ref::assert_matches("deqp_highpfs_pt_mul_assign_vec3_float");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul.mediump_float_vec4_fragment` (step PT0).
#[test]
fn pt_mul_float_vec4() {
    mali_ref::assert_matches("deqp_highpfs_pt_mul_float_vec4");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul.mediump_vec3_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_mul_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_mul_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul.mediump_vec3_float_fragment` (step PT3).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by refusals_pinned"]
fn pt_mul_vec3_float() {
    mali_ref::assert_matches("deqp_highpfs_pt_mul_vec3_float");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.mul.mediump_vec4_fragment` (step PT2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_mul_vec4() {
    mali_ref::assert_matches("deqp_highpfs_pt_mul_vec4");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.sub_assign_effect.mediump_vec3_fragment` (step PT0).
#[test]
fn pt_sub_assign_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_sub_assign_vec3");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.sub.mediump_float_vec4_fragment` (step PT1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn pt_sub_float_vec4() {
    mali_ref::assert_matches("deqp_highpfs_pt_sub_float_vec4");
}

/// `dEQP-GLES2.functional.shaders.operator.binary_operator.sub.mediump_vec3_fragment` (step PT0).
#[test]
fn pt_sub_vec3() {
    mali_ref::assert_matches("deqp_highpfs_pt_sub_vec3");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_highpfs_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// The words of `region` in an expect.json, or None when the region is
/// absent; a word that is not 16 hex digits is an error.
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

/// Every reference on disk parses: both shaders are there and not empty,
/// the trace is the reference's own (`deqp_ref.sh` names it after the
/// prefix), `vs.position` and `fs.fragment` have 64-bit hex words, and
/// `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_highpfs_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_highpfs_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_highpfs_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_highpfs_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                None if region == "vs.varying" => {}
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(n)) => {
                    if region == "fs.fragment" && n != *fs_words {
                        bad.push(format!("{name}: fs.fragment has {n} words, FS_WORDS says {fs_words}"));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] is byte-equal, and the three
/// lists cover every `deqp_highpfs_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_highpfs_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_highpfs_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let to = if mali_ref::differences(&r, &pair).is_empty() { "LOWERED" } else { "DIFFERS" };
                changed.push(format!("{name}: now compiles; move it to {to}"));
            }
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_highpfs_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in LOWERED {
        let r = mali_ref::Reference::load(&format!("deqp_highpfs_{name}"));
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
