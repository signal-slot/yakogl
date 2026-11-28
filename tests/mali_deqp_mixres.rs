// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_mixres_*` (plan #387, item
//! mixres-refs, `docs/notes/mali-mixres-lowering.md`): the shapes of the 89
//! dEQP-GLES2 cases whose first refusal is `MixedPrecision` ("a fragment
//! shader that mixes highp and mediump or lowp float values") on the host at
//! 349f989f that no `deqp_mixprec*_*`, `deqp_mixcol_*` or `deqp_t3gap_*`
//! reference draws. Every pair is verbatim from the `.qpa` of a host
//! deqp-gles2 run (`tools/mali/deqp-host-measure.sh`), and every reference
//! was drawn with `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's
//! G610 drm-shim, `--uniform-values`); `tools/mali/g310_check.sh` gives the
//! same words on the G310v2 for all of them. `tests/mali_ref` does the
//! compare.
//!
//! A reference the compiler refuses has its test `#[ignore]`d, and
//! [`refusals_pinned`] holds its outcome ([`PINS`]). Two finished results
//! of plan #402 that have not landed change five of them, and either may
//! land before or after this test: `lower-mixprec3` (81b3ce19, the
//! highp-island compares and the overwritten output stores that train 3c's
//! M2-4c and M2-1 also do) makes `hp_lt_vec3` and `hp_bin_eq_vec3`
//! byte-equal and refuses `rnd_block_store` with [`FORM`]; `lower-vsconst`
//! (2c0c6596, a mediump shader's highp constant trees folded at fp32) makes
//! `hp_mat3_add` and `hp_mat4_sub` byte-equal. So a pin is the outcome at
//! 349f989f plus the outcomes measured at those commits, and any of them
//! passes. When a lowering step moves a reference to another outcome, the
//! same commit updates its pin, drops its `#[ignore]` if it is byte-equal
//! and updates the note. [`references_parse`] checks that every file loads
//! and that the fragment word count matches the one the note quotes
//! ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// What compiling a reference's pair gives.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    /// Refused with exactly this `MaliError` message.
    Refused(&'static str),
    /// Compiles with other words than Mesa's.
    Differs,
    /// Compiles byte-equal to Mesa's.
    Lowered,
}
use Outcome::{Differs, Lowered, Refused};

/// Every reference, without the `deqp_mixres_` prefix: its outcome at
/// 349f989f, then the other outcomes it has at 81b3ce19 or 2c0c6596 (see the
/// header); any of them passes.
const PINS: &[(&str, Outcome, &[Outcome])] = &[
    ("hp_bin_eq_vec3", Refused(MIX), &[Lowered]),
    ("hp_cos", Refused(MIX), &[]),
    ("hp_distance", Refused(MIX), &[]),
    ("hp_fract_vec3", Refused(MIX), &[]),
    ("hp_inout_param", Refused(MIX), &[]),
    ("hp_log", Refused(MIX), &[]),
    ("hp_log2", Refused(MIX), &[]),
    ("hp_loop_do_while_uniform", Refused(MIX), &[]),
    ("hp_loop_for_uniform", Refused(MIX), &[]),
    ("hp_loop_while_dynamic", Refused(MIX), &[]),
    ("hp_lt_vec3", Refused(MIX), &[Lowered]),
    ("hp_mat2_mul", Refused(MIX), &[]),
    ("hp_mat3_add", Refused(MIX), &[Lowered]),
    ("hp_mat4_sub", Refused(MIX), &[Lowered]),
    ("hp_pow", Refused(MIX), &[]),
    ("hp_seq_bvec4", Refused(MIX), &[]),
    ("hp_seq_vec4", Refused(MIX), &[]),
    ("hp_sin", Refused(MIX), &[]),
    ("hp_sqrt_float", Refused(MIX), &[]),
    ("hp_sqrt_vec4", Refused(MIX), &[]),
    ("hp_tan", Refused(MIX), &[]),
    ("rnd_block_store", Refused(MIX), &[Refused(FORM)]),
    ("rnd_cond1", Refused(MIX), &[]),
];

const MIX: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";
const FORM: &str = "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("hp_bin_eq_vec3", 13),
    ("hp_cos", 18),
    ("hp_distance", 10),
    ("hp_fract_vec3", 18),
    ("hp_inout_param", 13),
    ("hp_log", 16),
    ("hp_log2", 13),
    ("hp_loop_do_while_uniform", 18),
    ("hp_loop_for_uniform", 17),
    ("hp_loop_while_dynamic", 20),
    ("hp_lt_vec3", 16),
    ("hp_mat2_mul", 7),
    ("hp_mat3_add", 7),
    ("hp_mat4_sub", 7),
    ("hp_pow", 16),
    ("hp_seq_bvec4", 25),
    ("hp_seq_vec4", 21),
    ("hp_sin", 18),
    ("hp_sqrt_float", 12),
    ("hp_sqrt_vec4", 24),
    ("hp_tan", 23),
    ("rnd_block_store", 8),
    ("rnd_cond1", 14),
];

/// `in0 == in1` of two highp vec3s (`operator.binary_operator.equal.highp_vec3_fragment`,
/// step M2-4c): `FCMP_OR.f32` per lane, the three masks reduced.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_bin_eq_vec3() {
    mali_ref::assert_matches("deqp_mixres_hp_bin_eq_vec3");
}

/// `cos(in0)` of a highp float (`operator.angle_and_trigonometry.cos.highp_float_fragment`,
/// step MR2): the fp32 `FSIN_TABLE`/`FCOS_TABLE` kernel, narrowed once.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_cos() {
    mali_ref::assert_matches("deqp_mixres_hp_cos");
}

/// `distance(in0, in1)` of highp floats (`operator.geometric.distance.highp_float_fragment`,
/// step M2-4c): the fragment `FADD.f32` and `|x|`; Mesa's vertex program
/// is 5 `vs.varying` words.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_distance() {
    mali_ref::assert_matches("deqp_mixres_hp_distance");
}

/// `fract(in0)` of a highp vec3 (`operator.common_functions.fract.highp_vec3_fragment`,
/// step M2-4c): `FROUND.f32.rtn` and `FADD.f32` per lane.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_fract_vec3() {
    mali_ref::assert_matches("deqp_mixres_hp_fract_vec3");
}

/// An `inout highp float` parameter (`functions.qualifiers.inout_highp_float_fragment`,
/// step M2-4a): the parameter folds, the rest is fp16.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_inout_param() {
    mali_ref::assert_matches("deqp_mixres_hp_inout_param");
}

/// `log(in0)` of a highp float (`operator.exponential.log.highp_float_fragment`,
/// step MR2): `FREXPE`, `FLOGD.f32` and two `FMA.f32`, narrowed once.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_log() {
    mali_ref::assert_matches("deqp_mixres_hp_log");
}

/// `log2(in0)` of a highp float (`operator.exponential.log2.highp_float_fragment`, step MR2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_log2() {
    mali_ref::assert_matches("deqp_mixres_hp_log2");
}

/// A highp float accumulator in a `do while` loop of `ui_three` iterations
/// (`loops.do_while_uniform_iterations.basic_highp_float_fragment`, step MX3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_loop_do_while_uniform() {
    mali_ref::assert_matches("deqp_mixres_hp_loop_do_while_uniform");
}

/// A highp float accumulator in a `for` loop of `ui_three` iterations
/// (`loops.for_uniform_iterations.basic_highp_float_fragment`, step MX3):
/// the counter `FADD.f32` and `FCMP_OR.f32.ge` against the uniform.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_loop_for_uniform() {
    mali_ref::assert_matches("deqp_mixres_hp_loop_for_uniform");
}

/// A highp float accumulator in a `while` loop of a varying-derived count
/// (`loops.while_dynamic_iterations.basic_highp_float_fragment`, step MX3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_loop_while_dynamic() {
    mali_ref::assert_matches("deqp_mixres_hp_loop_while_dynamic");
}

/// `lessThan(in0, in1)` of highp vec3s (`operator.float_compare.lessThan.highp_vec3_fragment`, step M2-4c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_lt_vec3() {
    mali_ref::assert_matches("deqp_mixres_hp_lt_vec3");
}

/// `const highp mat2` times a `const highp mat2` (`matrix.mul.const_highp_mat2_mat2_fragment`,
/// step M2-4b): a constant colour, two FAU words.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_mat2_mul() {
    mali_ref::assert_matches("deqp_mixres_hp_mat2_mul");
}

/// `const highp mat3` plus `const highp mat3` (`matrix.add.const_highp_mat3_mat3_fragment`,
/// step M2-4b): folded at fp32 and rounded once (FAU `3f9a40cd`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_mat3_add() {
    mali_ref::assert_matches("deqp_mixres_hp_mat3_add");
}

/// `const highp mat4` minus `const highp mat4` (`matrix.sub.const_highp_mat4_mat4_fragment`, step M2-4b).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_mat4_sub() {
    mali_ref::assert_matches("deqp_mixres_hp_mat4_sub");
}

/// `pow(in0, in1)` of highp floats (`operator.exponential.pow.highp_float_fragment`, step MR2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_pow() {
    mali_ref::assert_matches("deqp_mixres_hp_pow");
}

/// A highp vec4, ivec4 and bvec4 sequence with no side effects
/// (`operator.sequence.no_side_effects.highp_vec4_ivec4_bvec4_fragment`, step M2-4a).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_seq_bvec4() {
    mali_ref::assert_matches("deqp_mixres_hp_seq_bvec4");
}

/// A highp vec4 sequence with no side effects
/// (`operator.sequence.no_side_effects.highp_vec4_fragment`, step M2-4a).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_seq_vec4() {
    mali_ref::assert_matches("deqp_mixres_hp_seq_vec4");
}

/// `sin(in0)` of a highp float (`operator.angle_and_trigonometry.sin.highp_float_fragment`, step MR2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sin() {
    mali_ref::assert_matches("deqp_mixres_hp_sin");
}

/// `sqrt(in0)` of a highp float (`operator.exponential.sqrt.highp_float_fragment`,
/// step M2-4c): `FRSQ.f32`, `FRCP.f32`, one narrowing.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sqrt_float() {
    mali_ref::assert_matches("deqp_mixres_hp_sqrt_float");
}

/// `sqrt(in0)` of a highp vec4 (`operator.exponential.sqrt.highp_vec4_fragment`, step M2-4c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sqrt_vec4() {
    mali_ref::assert_matches("deqp_mixres_hp_sqrt_vec4");
}

/// `tan(in0)` of a highp float (`operator.angle_and_trigonometry.tan.highp_float_fragment`,
/// step MR2): the sine and cosine tables, `FRCP.f32` and the product.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_tan() {
    mali_ref::assert_matches("deqp_mixres_hp_tan");
}

/// An output store overwritten by a later store inside a bare block
/// (`fragment_ops.interaction.basic_shader.1`, step M2-1): its fp32
/// product of a mediump uniform and an int uniform leaves no words.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_block_store() {
    mali_ref::assert_matches("deqp_mixres_rnd_block_store");
}

/// A live fp32 sum of a mediump float uniform under `if` arms
/// (`shaders.random.conditionals.fragment.1`, step M2-2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_cond1() {
    mali_ref::assert_matches("deqp_mixres_rnd_cond1");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_mixres_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_mixres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_mixres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_mixres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_mixres_{name}.trace"));
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

/// Each reference's outcome is one of those in [`PINS`] (a refusal by its
/// whole `MaliError` message, not a substring), and [`PINS`] covers every
/// `deqp_mixres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = PINS.iter().map(|(n, _, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_mixres_* reference has no pin here");
    let mut changed = Vec::new();
    for &(name, tip, others) in PINS {
        let pinned = || std::iter::once(tip).chain(others.iter().copied());
        let r = mali_ref::Reference::load(&format!("deqp_mixres_{name}"));
        let (got, why) = match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => match mali_ref::differences(&r, &pair).first() {
                None => (Lowered, String::new()),
                Some(first) => (Differs, first.clone()),
            },
            Err(e) => match pinned().find(|o| matches!(o, Refused(t) if *t == e)) {
                Some(o) => (o, String::new()),
                None => {
                    changed.push(format!("{name}: refused with {e:?}, pinned {:?}", pinned().collect::<Vec<_>>()));
                    continue;
                }
            },
        };
        if !pinned().any(|o| o == got) {
            changed.push(format!("{name}: now {got:?} {why}, pinned {:?}", pinned().collect::<Vec<_>>()));
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
