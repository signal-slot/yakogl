// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_fsint_*` reference (Mesa
//! 26.2.1, plan #402 item `ref-fsint`, `docs/notes/mali-deqp-fsint.md`)
//! against the Mali online compiler. The refusal is `Valhall: an integer or
//! boolean value is not lowered yet` in a fragment shader (188 Fail and 22
//! NotSupported in the board slices `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`).
//! 206 of those 210 cases fall in forms the `deqp_intval*`, `deqp_intrnd`
//! and `deqp_gaps` references already draw; these eight are the lane counts,
//! precisions and operand mixes of the same case groups that none of them
//! draws. Each pair is written after the dEQP-GLES2 generator template of
//! the case it names (VK-GL-CTS 93bca01), drawn with `tools/mali/deqp_ref.sh
//! --instrument shim` (Mesa 26.2.1's G610 drm-shim, `--uniform-values`)
//! twice with the same bytes.
//!
//! | Ref | Case (`dEQP-GLES2.functional.shaders.`) | Cases of the form | Nearest reference | Mesa's fragment program | fs words |
//! |---|---|--:|---|---|--:|
//! | `ne_ivec4_highp` | `operator.binary_operator.not_equal.highp_ivec4_fragment` | 6 (`_ivec3`, `_ivec4`) | `intval4_ne_ivec2_fs` | `any(ine)` of four lanes: four `ICMP_OR.u32.ne.m1` chained through the third source | 21 |
//! | `notequal_ivec4_highp` | `operator.int_compare.notEqual.highp_ivec4_fragment` | 3 | `intval3_opfs_notequal_ivec3_mediump` | four `ICMP_OR.u32.ne.m1` + `MUX.v2i16`; the alpha lane computed (`ATEST` of `r0.h0`) | 26 |
//! | `predec_effect_ivec4_lowp` | `operator.unary_operator.pre_decrement_effect.lowp_ivec4_fragment` | 12 (`_ivec4`) | `intval3_opfs_post_decrement_effect_ivec2_highp` | four `IADD_IMM.i32 #0xFFFFFFFF` at 32 bits (lowp too), two `MKVEC` pairs, a computed alpha | 28 |
//! | `postdec_result_ivec3_mediump` | `operator.unary_operator.post_decrement_result.mediump_ivec3_fragment` | 12 (`post_decrement_result`) | none (`pre_decrement_result` has the `IADD_IMM`) | `F32_TO_S32.rtz` then `S32_TO_F32` per lane, no integer op, not folded to `ftrunc` | 18 |
//! | `sequence_bool_vec2_highp` | `operator.sequence.no_side_effects.highp_bool_vec2_fragment` | 1 | `intval3_opfs_sequence_bool_vec2_mediump` | fp32: `FCMP_OR.f32.lt.f1` (`b2f`), `FADD.f32`, `F32_TO_S32.rtz` | 21 |
//! | `step_float_lowp` | `operator.common_functions.step.lowp_float_fragment` | 3 (`step.*_float`) | `intval3_opfs_step_vec3_highp` | a lowp scalar `step` at fp32 (`FCMP_OR.f32.ge.m1`, `MUX`), the three constant lanes FAU words | 12 |
//! | `conv_vec2_ivec2_to_bvec4` | `conversions.vector_combine.vec2_ivec2_to_bvec4_fragment` | 5 (a float operand) | `intval3_reslib_ivec4_to_bvec4_fs` | `bool(vec2)` as `FCMP_OR.v2f16.ne.m1` of an fp16 pair, its lanes against the `uniform bvec4` at 16 bits (`ICMP_OR.v2u16.eq`, `S16_TO_S32`) | 26 |
//! | `conv_bool_float_int_to_bvec3` | `conversions.vector_combine.bool_float_int_to_bvec3_fragment` | (same 5) | `intval_reslib_mixed_fs` | the bool and the float lane in one `FCMP_OR.v2f16.ne.m1` (`.h10`), the int lane `F16_TO_S32` + `ICMP_OR.u32.ne` | 23 |
//!
//! At the tip every reference is refused, so each byte-compare is
//! `#[ignore]`d and `refusals_pinned` holds the refusal ([`REFUSED`]): the
//! integer/boolean text, or for `sequence_bool_vec2_highp` the next one (its
//! `in0 && in1` is dead, as in `intval3_opfs_sequence_bool_vec2_mediump`).
//! The lowering that lifts one moves it to [`MATCHED`] (byte-equal) or
//! [`DIFFERS`] (compiles with other words than Mesa's) in the same commit and
//! drops its `#[ignore]`. `fs_words_pinned` checks each reference's
//! `fs.fragment` word count ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsint
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const INT_VALUE: &str = "Valhall: an integer or boolean value is not lowered yet";
/// The refusal after the integer one once the dead `in0 && in1` is left out.
const FRAGCOLOR_LANE: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";

/// The references the lowering makes byte-equal, without the `deqp_fsint_`
/// prefix: each has a running byte-compare test below.
const MATCHED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

/// Every refused reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("conv_bool_float_int_to_bvec3", INT_VALUE),
    ("conv_vec2_ivec2_to_bvec4", INT_VALUE),
    ("ne_ivec4_highp", INT_VALUE),
    ("notequal_ivec4_highp", INT_VALUE),
    ("postdec_result_ivec3_mediump", INT_VALUE),
    ("predec_effect_ivec4_lowp", INT_VALUE),
    ("sequence_bool_vec2_highp", FRAGCOLOR_LANE),
    ("step_float_lowp", INT_VALUE),
];

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("conv_bool_float_int_to_bvec3", 23),
    ("conv_vec2_ivec2_to_bvec4", 26),
    ("ne_ivec4_highp", 21),
    ("notequal_ivec4_highp", 26),
    ("postdec_result_ivec3_mediump", 18),
    ("predec_effect_ivec4_lowp", 28),
    ("sequence_bool_vec2_highp", 21),
    ("step_float_lowp", 12),
];

fn on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_fsint_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    v.sort();
    v
}

#[test]
fn refusals_pinned() {
    let mut listed: Vec<String> = REFUSED
        .iter()
        .map(|(n, _)| *n)
        .chain(MATCHED.iter().copied())
        .chain(DIFFERS.iter().copied())
        .map(str::to_string)
        .collect();
    listed.sort();
    assert_eq!(on_disk(), listed, "every deqp_fsint_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_fsint_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_fsint_{name}"));
        let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}

/// The `fs.fragment` words of each reference number as [`FS_WORDS`] says,
/// and each word is 16 hex digits.
#[test]
fn fs_words_pinned() {
    let mut listed: Vec<String> = FS_WORDS.iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk(), listed, "FS_WORDS lists every deqp_fsint_* reference");
    for (name, want) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_fsint_{name}"));
        let at = r.json.find("\"fs.fragment\": {").unwrap_or_else(|| panic!("{name}: no fs.fragment"));
        let rest = &r.json[at..];
        let w = rest.find("\"words\": [").unwrap_or_else(|| panic!("{name}: no words")) + "\"words\": [".len();
        let list = &rest[w..w + rest[w..].find(']').unwrap()];
        let words: Vec<&str> = list.split(',').map(|s| s.trim().trim_matches('"')).filter(|s| !s.is_empty()).collect();
        for word in &words {
            assert!(word.len() == 16 && word.chars().all(|c| c.is_ascii_hexdigit()), "{name}: {word:?}");
        }
        assert_eq!(words.len(), *want, "{name}: fs.fragment words");
    }
}

#[test]
#[ignore = "refused: an integer or boolean value (any(ine) of four lanes); pinned by refusals_pinned"]
fn ne_ivec4_highp() {
    mali_ref::assert_matches("deqp_fsint_ne_ivec4_highp");
}

#[test]
#[ignore = "refused: an integer or boolean value (notEqual of ivec4, a computed alpha); pinned by refusals_pinned"]
fn notequal_ivec4_highp() {
    mali_ref::assert_matches("deqp_fsint_notequal_ivec4_highp");
}

#[test]
#[ignore = "refused: an integer or boolean value (four lowp int decrements); pinned by refusals_pinned"]
fn predec_effect_ivec4_lowp() {
    mali_ref::assert_matches("deqp_fsint_predec_effect_ivec4_lowp");
}

#[test]
#[ignore = "refused: an integer or boolean value (f2i then i2f, no integer op); pinned by refusals_pinned"]
fn postdec_result_ivec3_mediump() {
    mali_ref::assert_matches("deqp_fsint_postdec_result_ivec3_mediump");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (the fp32 b2f, sum and f2i); pinned by refusals_pinned"]
fn sequence_bool_vec2_highp() {
    mali_ref::assert_matches("deqp_fsint_sequence_bool_vec2_highp");
}

#[test]
#[ignore = "refused: an integer or boolean value (a lowp scalar step at fp32); pinned by refusals_pinned"]
fn step_float_lowp() {
    mali_ref::assert_matches("deqp_fsint_step_float_lowp");
}

/// Drawn with `--uniform-values`: the `uniform bvec4 ref_out0` is true,
/// false, true, false.
#[test]
#[ignore = "refused: an integer or boolean value (bool of an fp16 pair beside int lanes); pinned by refusals_pinned"]
fn conv_vec2_ivec2_to_bvec4() {
    mali_ref::assert_matches("deqp_fsint_conv_vec2_ivec2_to_bvec4");
}

/// Drawn with `--uniform-values`: the `uniform bvec3 ref_out0` is true,
/// false, true.
#[test]
#[ignore = "refused: an integer or boolean value (a bool and a float lane in one fp16 compare); pinned by refusals_pinned"]
fn conv_bool_float_int_to_bvec3() {
    mali_ref::assert_matches("deqp_fsint_conv_bool_float_int_to_bvec3");
}
