// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_fmares_*` (plan #387, item
//! fmares-refs, `docs/notes/mali-fmares-lowering.md`): the dEQP-GLES2 vertex
//! shapes whose first refusal at the integration tip `349f989f` is
//! `FusedMultiplyAdd` and that no `deqp_constprod_fma_*`, `deqp_kfold_*` or
//! `deqp_fmaloc_*` reference covers: the lowp/mediump and other-size matrix
//! products of constprod K6 (`k6_*`) and the vector, mediump and invariance
//! forms of K7's `reflect`/`refract`/`distance`/`fract` (`k7_*`).
//! `tests/mali_ref` does the compare.
//!
//! Every reference was drawn with `tools/mali/deqp_ref.sh --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`) and its `.nir.sha` with
//! `tools/mali/nir_refs.sh`; each pair is verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` at `349f989f`, and each shader's
//! `.license` names its case.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the word counts the note
//! quotes ([`WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fmares
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_fmares_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("k6_const_highp_mat4_vec4", TEXT_FMA),
    ("k6_const_lowp_mat4_vec4", TEXT_FMA),
    ("k6_const_mediump_mat2_vec2", TEXT_FMA),
    ("k6_dyn_highp_mat2_mat2", TEXT_FMA),
    ("k6_dyn_highp_mat3_mat3", TEXT_FMA),
    ("k6_dyn_lowp_mat2_vec2", TEXT_FMA),
    ("k6_dyn_mediump_mat4_vec4", TEXT_FMA),
    ("k6_mulassign_highp_mat4", TEXT_FMA),
    ("k7_inv_highp_lowp", TEXT_FMA),
    ("k7_inv_lowp_lowp", TEXT_FMA),
    ("k7_inv_mediump_mediump", TEXT_FMA),
    ("k7_reflect_mediump_float", TEXT_FMA),
    ("k7_refract_highp_vec3", TEXT_FMA),
    ("k7_refract_mediump_vec4", TEXT_FMA),
];

const TEXT_FMA: &str =
    "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet";

/// The words of each reference's `vs.varying` and `fs.fragment` regions
/// (Mesa's varying program and fragment program, the tail and the blend
/// branch included).
const WORDS: &[(&str, usize, usize)] = &[
    ("k6_const_highp_mat4_vec4", 15, 8),
    ("k6_const_lowp_mat4_vec4", 29, 7),
    ("k6_const_mediump_mat2_vec2", 21, 7),
    ("k6_dyn_highp_mat2_mat2", 20, 8),
    ("k6_dyn_highp_mat3_mat3", 46, 8),
    ("k6_dyn_lowp_mat2_vec2", 23, 7),
    ("k6_dyn_mediump_mat4_vec4", 39, 7),
    ("k6_mulassign_highp_mat4", 73, 8),
    ("k7_inv_highp_lowp", 69, 15),
    ("k7_inv_lowp_lowp", 66, 15),
    ("k7_inv_mediump_mediump", 73, 15),
    ("k7_reflect_mediump_float", 27, 7),
    ("k7_refract_highp_vec3", 47, 7),
    ("k7_refract_mediump_vec4", 47, 7),
];

/// `const highp mat4 * attribute vec4` (K6a; Mesa: `FMA.f32` with the zero entries folded out).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_const_highp_mat4_vec4() {
    mali_ref::assert_matches("deqp_fmares_k6_const_highp_mat4_vec4");
}

/// `const lowp mat4 * attribute vec4` (K6a; Mesa: fp16, `FMA.v2f16` x6).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_const_lowp_mat4_vec4() {
    mali_ref::assert_matches("deqp_fmares_k6_const_lowp_mat4_vec4");
}

/// `const mediump mat2 * attribute vec2` (K6a: a `const` matrix column; Mesa: `FMA.v2f16`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_const_mediump_mat2_vec2() {
    mali_ref::assert_matches("deqp_fmares_k6_const_mediump_mat2_vec2");
}

/// `attribute highp mat2 * uniform mat2` (K6c: attribute times uniform matrix; Mesa: `FMA.f32` x8).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_dyn_highp_mat2_mat2() {
    mali_ref::assert_matches("deqp_fmares_k6_dyn_highp_mat2_mat2");
}

/// `attribute highp mat3 * uniform mat3` (K6c; Mesa: 27 `FMA.f32`, 6 `FADD.f32`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_dyn_highp_mat3_mat3() {
    mali_ref::assert_matches("deqp_fmares_k6_dyn_highp_mat3_mat3");
}

/// `attribute lowp mat2 * attribute vec2` (K6b; Mesa: fp16).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_dyn_lowp_mat2_vec2() {
    mali_ref::assert_matches("deqp_fmares_k6_dyn_lowp_mat2_vec2");
}

/// `attribute mediump mat4 * attribute vec4` (K6b: an attribute matrix column; Mesa: fp16, `FMA.v2f16` x8).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_dyn_mediump_mat4_vec4() {
    mali_ref::assert_matches("deqp_fmares_k6_dyn_mediump_mat4_vec4");
}

/// `res = a_mat4; res *= u_in1` of highp mat4 (K6c through a local; Mesa: 48 `FMA.f32`, 9 `FADD.f32`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k6_mulassign_highp_mat4() {
    mali_ref::assert_matches("deqp_fmares_k6_mulassign_highp_mat4");
}

/// `invariance.highp.subexpression_precision_lowp`: lowp unrelated output, highp `gl_Position` chain (K7; Mesa: fp32 position, fp16 varying).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k7_inv_highp_lowp() {
    mali_ref::assert_matches("deqp_fmares_k7_inv_highp_lowp");
}

/// `invariance.lowp.subexpression_precision_lowp`: lowp `refract`, `distance`, `fract` into an invariant `gl_Position` (K7; Mesa: fp16).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k7_inv_lowp_lowp() {
    mali_ref::assert_matches("deqp_fmares_k7_inv_lowp_lowp");
}

/// `invariance.mediump.subexpression_precision_mediump` (K7; Mesa: fp16 with `FROUND.v2f16` for `fract`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k7_inv_mediump_mediump() {
    mali_ref::assert_matches("deqp_fmares_k7_inv_mediump_mediump");
}

/// `reflect(float, float)` of mediump attribute lanes (K7: the scalar `reflect`; Mesa: fp16).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k7_reflect_mediump_float() {
    mali_ref::assert_matches("deqp_fmares_k7_reflect_mediump_float");
}

/// `refract(vec3, vec3, float)` of highp attribute lanes (K7: the `refract` `ffma`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k7_refract_highp_vec3() {
    mali_ref::assert_matches("deqp_fmares_k7_refract_highp_vec3");
}

/// `refract(vec4, vec4, float)` of mediump attribute lanes (K7; Mesa: fp16, `FMA.v2f16` x14).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k7_refract_mediump_vec4() {
    mali_ref::assert_matches("deqp_fmares_k7_refract_mediump_vec4");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_fmares_")?.strip_suffix(".expect.json").map(str::to_string))
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
/// prefix), `vs.position`, `vs.varying` and `fs.fragment` have 64-bit hex
/// words, and `vs.varying` and `fs.fragment` have the counts of [`WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = WORDS.iter().map(|(n, _, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "WORDS does not list every deqp_fmares_* reference");
    let mut bad = Vec::new();
    for (name, vs_words, fs_words) in WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_fmares_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_fmares_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_fmares_{name}.trace"));
        }
        for (region, want) in [("vs.position", None), ("vs.varying", Some(*vs_words)), ("fs.fragment", Some(*fs_words))]
        {
            match region_words(&r.json, region) {
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(n)) => {
                    if want.is_some_and(|w| w != n) {
                        bad.push(format!("{name}: {region} has {n} words, WORDS says {want:?}"));
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
/// lists cover every `deqp_fmares_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_fmares_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_fmares_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fmares_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fmares_{name}"));
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
