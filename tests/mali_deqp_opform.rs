// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_opform_*` (plan #387, item
//! opform-design, `docs/notes/mali-opform-lowering.md`): the fragment
//! operand forms (`MaliError::FsOperandForm`) of the Verdin iMX95 (Mali-G310)
//! dEQP-GLES2 run that no earlier reference pins. Each pair is one dEQP case
//! of the run; the note's section 3 maps the cases to the form classes and
//! section 4 the classes to these references. `tests/mali_ref` does the
//! compare. Every reference was drawn with `tools/mali/deqp_ref.sh
//! --instrument shim` (Mesa 26.2.1's G610 drm-shim, `--uniform-values`) and is
//! identical on the G310v2 (`tools/mali/g310_check.sh`).
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_opform
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_opform_` prefix.
/// `matadd_uniform` and `matsub_uniform` since plan #387 mali-train-3a
/// (opform O2, class OB with NS): the column sums of a matrix uniform plus
/// or minus a lane alone are admitted (nir_fs.rs `select`'s gate,
/// `uniform_lane_sum`).
const LOWERED: &[&str] = &["matadd_uniform", "matsub_uniform"];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("clamp_vec3_float", TEXT_OPERAND),
    ("conv_vec3", TEXT_OPERAND),
    ("matmul_const", TEXT_OPERAND),
    ("matneg_mat3", TEXT_OPERAND),
    ("min_vec3", TEXT_OPERAND),
    ("mul_float_vec4", TEXT_OPERAND),
    ("ret_dynamic", TEXT_OPERAND),
    ("ret_never", TEXT_OPERAND),
    ("rnd_constneg85", TEXT_OPERAND),
    ("rnd_splat18", TEXT_OPERAND),
];

const TEXT_OPERAND: &str = "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("clamp_vec3_float", 19),
    ("conv_vec3", 29),
    ("matadd_uniform", 17),
    ("matmul_const", 10),
    ("matneg_mat3", 18),
    ("matsub_uniform", 17),
    ("min_vec3", 20),
    ("mul_float_vec4", 15),
    ("ret_dynamic", 16),
    ("ret_never", 8),
    ("rnd_constneg85", 19),
    ("rnd_splat18", 10),
];

/// `clamp(v_in0.zxy, v_in1.x, v_in2.y)`: a vec3 clamp to two scalar
/// varying bounds (class MM, `vectorise`'s `Fmax | Fmin` arm).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn clamp_vec3_float() {
    mali_ref::assert_matches("deqp_opform_clamp_vec3_float");
}

/// `vec3(bool, float, int)` of the shader library's conversions: the `b2f`
/// of a mask that is not an fp16 compare (class BM).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn conv_vec3() {
    mali_ref::assert_matches("deqp_opform_conv_vec3");
}

/// `u_in0 + v_coords` of a `uniform mediump mat3`: an operation's result
/// second in a sum (class OB).
#[test]
fn matadd_uniform() {
    mali_ref::assert_matches("deqp_opform_matadd_uniform");
}

/// `in0 * v_coords` of a `const mediump mat3`: a lane splat as a fused
/// multiply-add's addend (class SC).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn matmul_const() {
    mali_ref::assert_matches("deqp_opform_matmul_const");
}

/// `-v_mat3`: the negation of a cross-slot column pair, and a negation
/// first in a sum (class NM).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn matneg_mat3() {
    mali_ref::assert_matches("deqp_opform_matneg_mat3");
}

/// `u_in0 - v_coords` of a `uniform mediump mat3`: a negated splat and an
/// operation second in a sum (class OB with NS).
#[test]
fn matsub_uniform() {
    mali_ref::assert_matches("deqp_opform_matsub_uniform");
}

/// `min(v_in0.zxy, v_in1.yzx)`: a vec3 min of two varying lanes (class MM).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn min_vec3() {
    mali_ref::assert_matches("deqp_opform_min_vec3");
}

/// `v_in0.z * v_in1.wzyx`: a swapped half second in a product (class HS).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mul_float_vec4() {
    mali_ref::assert_matches("deqp_opform_mul_float_vec4");
}

/// `return` between two `gl_FragColor` writes under `v.x + v.y >= 0.0`:
/// a flattened `if` whose compare reads a computed lane (class IF).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ret_dynamic() {
    mali_ref::assert_matches("deqp_opform_ret_dynamic");
}

/// `vec4(v_coords.wzy, 1.0)` after a dead `return`: a varying's swapped
/// pair as an output word (class HO).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ret_never() {
    mali_ref::assert_matches("deqp_opform_ret_never");
}

/// random.scalar_conversion.fragment.85: `i - vec4(-8.0, 4.0, d, 7.5)`, the
/// negation of a constant/lane pair and an operation second in a sum
/// (class CN).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_constneg85() {
    mali_ref::assert_matches("deqp_opform_rnd_constneg85");
}

/// random.basic_expression.fragment.18: `vec4(c, c, b, 0.0)`, a uniform's
/// half in both lanes of an output word (class SO).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_splat18() {
    mali_ref::assert_matches("deqp_opform_rnd_splat18");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_opform_")?.strip_suffix(".expect.json").map(str::to_string))
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

/// Every reference on disk loads: both shaders are there and not empty,
/// the trace is the reference's own (`deqp_ref.sh` names it after the
/// prefix), `vs.position` and `fs.fragment` have 64-bit hex words, and
/// `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_opform_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_opform_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_opform_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_opform_{name}.trace"));
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
/// lists cover every `deqp_opform_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_opform_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_opform_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_opform_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_opform_{name}"));
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
