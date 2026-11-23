// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_intval3_*` (plan #387, item
//! intval3-ref, `docs/notes/mali-deqp-intval3.md`): one or two dEQP-GLES2
//! pairs per shape of the must-pass cases whose first refusal is
//! `an integer or boolean value` at `3497799a` (206 cases, all fragment
//! shaders, HOST-ONLY count) that no `deqp_intval_*` reference covers.
//! `tests/mali_ref` does the compare. Every reference was drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (`drm_shim_ref.sh run ...
//! --uniform-values` on Mesa 26.2.1's G610 drm-shim; the command is in the
//! note), the pair verbatim from the case's `.qpa`.
//!
//! `rsg_sconv55_fs` is byte-equal since its dead `bvec4` local is left out
//! of the walk (`nir_fs.rs` `ir_live`). Every other reference is refused, so
//! its byte-compare is `#[ignore]`d and `refusals_pinned` holds the refusal
//! ([`REFUSED`]): the integer/boolean text, or for two the next one past
//! their dead int and bool values. The lowering step that lifts one moves it to
//! [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other words than
//! Mesa's) in the same commit, drops its `#[ignore]` and updates the note.
//! `references_parse` checks every file: the pair is there, each region's
//! words are 64-bit hex, the trace is the reference's own, and the fragment
//! program has the word count the note quotes ([`FS_WORDS`]).
//!
//! `reslib_const_int_vs`, `rsg_condcomb80_fs` and `rsg_sconv55_fs` get part
//! of their result from Mesa's link (a constant varying moved into the
//! fragment shader's FAU, or every varying dead): `mali_ref` runs the GL
//! core's link, which may not reach the same program (`tests/mali_ref`'s
//! module comment). They are pinned like the rest until a lowering says.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intval3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_intval3_` prefix: each has a byte-compare test below.
const LOWERED: &[&str] = &["rsg_sconv55_fs"];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const INT_VALUE: &str = "Valhall: an integer or boolean value is not lowered yet";
/// The later refusals of the references whose int and bool values are all
/// dead (`nir_fs.rs` `ir_live`).
const FRAGCOLOR: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const FS_UNIFORM: &str = "Valhall: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, is not lowered yet";

/// Every ignored reference with its whole `MaliError` message (all 22 were
/// the integer/boolean text at `3497799a`).
const REFUSED: &[(&str, &str)] = &[
    ("cond_complex_fs", INT_VALUE),
    ("cond_compound_fs", INT_VALUE),
    ("idx_tmp_vec3_dynwrite_fs", INT_VALUE),
    ("opfs_logical_and_bool", INT_VALUE),
    ("opfs_not_equal_int_highp", INT_VALUE),
    ("opfs_notequal_ivec3_mediump", INT_VALUE),
    ("opfs_post_decrement_effect_ivec2_highp", INT_VALUE),
    ("opfs_pre_decrement_result_int_mediump", INT_VALUE),
    /* the sequence's first operand is dead: the colour's lanes next */
    ("opfs_sequence_bool_vec2_mediump", FRAGCOLOR),
    ("opfs_step_float_vec2_mediump", INT_VALUE),
    ("opfs_step_vec3_highp", INT_VALUE),
    ("reslib_const_int_vs", INT_VALUE),
    ("reslib_func_bool_fs", INT_VALUE),
    ("reslib_inout_int_fs", INT_VALUE),
    ("reslib_int_to_bool_fs", INT_VALUE),
    ("reslib_ivec4_to_bvec4_fs", INT_VALUE),
    ("rsg_cond12_fs", INT_VALUE),
    ("rsg_condcomb80_fs", INT_VALUE),
    ("rsg_exp76_fs", INT_VALUE),
    ("rsg_sconv72_fs", INT_VALUE),
    /* every int and bool value dead: the uniform's form next */
    ("rsg_swizzle12_fs", FS_UNIFORM),
];

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included), as the note's
/// section 3 quotes them.
const FS_WORDS: &[(&str, usize)] = &[
    ("cond_complex_fs", 25),
    ("cond_compound_fs", 22),
    ("idx_tmp_vec3_dynwrite_fs", 77),
    ("opfs_logical_and_bool", 12),
    ("opfs_not_equal_int_highp", 11),
    ("opfs_notequal_ivec3_mediump", 20),
    ("opfs_post_decrement_effect_ivec2_highp", 17),
    ("opfs_pre_decrement_result_int_mediump", 13),
    ("opfs_sequence_bool_vec2_mediump", 25),
    ("opfs_step_float_vec2_mediump", 16),
    ("opfs_step_vec3_highp", 16),
    ("reslib_const_int_vs", 14),
    ("reslib_func_bool_fs", 13),
    ("reslib_inout_int_fs", 15),
    ("reslib_int_to_bool_fs", 15),
    ("reslib_ivec4_to_bvec4_fs", 28),
    ("rsg_cond12_fs", 31),
    ("rsg_condcomb80_fs", 7),
    ("rsg_exp76_fs", 27),
    ("rsg_sconv55_fs", 9),
    ("rsg_sconv72_fs", 7),
    ("rsg_swizzle12_fs", 11),
];

/// `shaders.conditionals.if.complex_condition_fragment`: `false || a && b`
/// of two fp16 compares, `LSHIFT_AND.v2i16`, the `if` as `MUX.v2i16`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn cond_complex_fs() {
    mali_ref::assert_matches("deqp_intval3_cond_complex_fs");
}

/// `shaders.conditionals.if.compound_statement_fragment`: a one-armed `if`
/// on an fp16 compare writing two outputs, `RES && isOk(...)`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn cond_compound_fs() {
    mali_ref::assert_matches("deqp_intval3_cond_compound_fs");
}

/// `shaders.indexing.tmp_array.vec3_dynamic_write_static_read_fragment`: a
/// local array written at `uniform int` indices, `ICMP_OR.s32.lt` against
/// the lane constants and a `MUX`/`CSEL` select per element.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn idx_tmp_vec3_dynwrite_fs() {
    mali_ref::assert_matches("deqp_intval3_idx_tmp_vec3_dynwrite_fs");
}

/// `shaders.operator.binary_operator.logical_and.bool_fragment`: `&&` of two
/// `> 0.0` compares is `FMIN.v2f16` and one `FCMP_OR.v2f16.lt.f1`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_logical_and_bool() {
    mali_ref::assert_matches("deqp_intval3_opfs_logical_and_bool");
}

/// `shaders.operator.binary_operator.not_equal.highp_int_fragment`:
/// `ICMP_OR.u32.ne.m1` of two run-time ints.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_not_equal_int_highp() {
    mali_ref::assert_matches("deqp_intval3_opfs_not_equal_int_highp");
}

/// `shaders.operator.int_compare.notEqual.mediump_ivec3_fragment`:
/// `notEqual` per lane, `vec3(bvec3)` as `MUX.v2i16`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_notequal_ivec3_mediump() {
    mali_ref::assert_matches("deqp_intval3_opfs_notequal_ivec3_mediump");
}

/// `shaders.operator.unary_operator.post_decrement_effect.highp_ivec2_fragment`:
/// `res--` is `IADD_IMM.i32 #0xFFFFFFFF` per lane.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_post_decrement_effect_ivec2_highp() {
    mali_ref::assert_matches("deqp_intval3_opfs_post_decrement_effect_ivec2_highp");
}

/// `shaders.operator.unary_operator.pre_decrement_result.mediump_int_fragment`:
/// `--in0` of a mediump int.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_pre_decrement_result_int_mediump() {
    mali_ref::assert_matches("deqp_intval3_opfs_pre_decrement_result_int_mediump");
}

/// `shaders.operator.sequence.no_side_effects.mediump_bool_vec2_fragment`:
/// `vec2(bool)` as an fp16 `.f1` compare, `F16_TO_S32` and `S32_TO_F32`.
#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn opfs_sequence_bool_vec2_mediump() {
    mali_ref::assert_matches("deqp_intval3_opfs_sequence_bool_vec2_mediump");
}

/// `shaders.operator.common_functions.step.mediump_float_vec2_fragment`:
/// `step` is `FCMP_OR.v2f16.ge.f1` (a 1.0/0.0 result).
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_step_float_vec2_mediump() {
    mali_ref::assert_matches("deqp_intval3_opfs_step_float_vec2_mediump");
}

/// `shaders.operator.common_functions.step.highp_vec3_fragment`: `step` at
/// highp is `FCMP_OR.f32.ge.m1` and a `MUX.v2i16` to 1.0/0.0.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn opfs_step_vec3_highp() {
    mali_ref::assert_matches("deqp_intval3_opfs_step_vec3_highp");
}

/// `shaders.constants.int_0_vertex`: the int `isOk` of a constant varying,
/// which Mesa's link moves into the FAU (no varying program).
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn reslib_const_int_vs() {
    mali_ref::assert_matches("deqp_intval3_reslib_const_int_vs");
}

/// `shaders.functions.datatypes.bool_bool_fragment`: `!bool(v)` is
/// `FCMP_OR.v2f16.eq`, compared with a `uniform bool` by `ICMP_OR.v2u16.eq`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn reslib_func_bool_fs() {
    mali_ref::assert_matches("deqp_intval3_reslib_func_bool_fs");
}

/// `shaders.functions.qualifiers.inout_int_fragment`: an `inout int`
/// negated, `ISUB.s32 0, x`, compared with a `uniform int`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn reslib_inout_int_fs() {
    mali_ref::assert_matches("deqp_intval3_reslib_inout_int_fs");
}

/// `shaders.conversions.scalar_to_scalar.int_to_bool_fragment`: `bool(int)`
/// is `ICMP_OR.u32.ne 0`, compared with a `uniform bool` word.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn reslib_int_to_bool_fs() {
    mali_ref::assert_matches("deqp_intval3_reslib_int_to_bool_fs");
}

/// `shaders.conversions.vector_to_vector.ivec4_to_bvec4_fragment`: four
/// `bool(int)` compared with a `uniform bvec4`, `ICMP_AND` and `LSHIFT_AND`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn reslib_ivec4_to_bvec4_fs() {
    mali_ref::assert_matches("deqp_intval3_reslib_ivec4_to_bvec4_fs");
}

/// `shaders.random.conditionals.fragment.12`: an `if` on `ivec2` uniform
/// arithmetic `!=`, `ICMP_OR.u32.ne` chained and `CSEL.u32.eq` per lane.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn rsg_cond12_fs() {
    mali_ref::assert_matches("deqp_intval3_rsg_cond12_fs");
}

/// `shaders.random.conditionals.combined.80`: `d != d` of a `uniform ivec4`
/// folds to false; the colour is the FAU's constants.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn rsg_condcomb80_fs() {
    mali_ref::assert_matches("deqp_intval3_rsg_condcomb80_fs");
}

/// `shaders.random.exponential.fragment.76`: a dead `int(varying)` and
/// `exp2(float(uniform int))` as `LDEXP.f32 1.0, u`.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn rsg_exp76_fs() {
    mali_ref::assert_matches("deqp_intval3_rsg_exp76_fs");
}

/// `shaders.random.scalar_conversion.fragment.55`: `vec4(..., uniform int)`
/// is `S32_TO_F32` of the FAU word; the `bvec4` local is dead.
#[test]
fn rsg_sconv55_fs() {
    mali_ref::assert_matches("deqp_intval3_rsg_sconv55_fs");
}

/// `shaders.random.scalar_conversion.fragment.72`: `vec4(bool(uniform
/// float), ...)` is `FCMP_OR.v2f16.ne.f1` of the FAU word.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn rsg_sconv72_fs() {
    mali_ref::assert_matches("deqp_intval3_rsg_sconv72_fs");
}

/// `shaders.random.swizzle.fragment.12`: every int and bool value is a dead
/// global or folds; no integer op is left.
#[test]
#[ignore = "refused: a mediump fragment shader uniform of another form; pinned by refusals_pinned"]
fn rsg_swizzle12_fs() {
    mali_ref::assert_matches("deqp_intval3_rsg_swizzle12_fs");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_intval3_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_intval3_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_intval3_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_intval3_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_intval3_{name}.trace"));
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
/// lists cover every `deqp_intval3_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_intval3_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_intval3_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intval3_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intval3_{name}"));
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
