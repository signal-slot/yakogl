// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_arithres_*` (plan #387, item
//! arithres-refs, `docs/notes/mali-arithres-lowering.md`): the forms of the
//! `MaliError::Arithmetic` residue (fragment and vertex) that no earlier
//! design draws, measured at main `56b5878a` after trains 2 and 3a and
//! lower-train-2. Fragment vectors of a built-in kernel on a mediump varying's lanes
//! (`fsvec_*`, step AR1), vertex vec3/vec4 kernels (`vswide_*`, AR2), the
//! reciprocal of a computed mediump lane (`fsrcp_*`, AR3) and a highp `mat3`
//! divided in a fragment shader (`matdiv_*`, AR4). `tests/mali_ref` does the
//! compare. Every reference was drawn with `tools/mali/deqp_ref.sh
//! --instrument shim` (Mesa 26.2.1's G610 drm-shim, `--uniform-values`); the
//! pairs are the dEQP-GLES2 programs verbatim (the host measure's `.qpa`),
//! but for `fsrcp_*`, written after the operator template (their
//! `.frag.license` says so).
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. A reference the compiler already compiles byte-equal is in [`LOWERED`]
//! and its test runs. `references_parse` checks every file and the fragment
//! word count the note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_arithres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal (or that were byte-equal when
/// drawn), without the `deqp_arithres_` prefix.
const LOWERED: &[&str] = &["fsrcp_diff"];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("fsrcp_prod", ARITH),
    ("fsvec_cos_vec3", ARITH),
    ("fsvec_log2_vec3", ARITH),
    ("fsvec_log_vec2", ARITH),
    ("fsvec_pow_vec4", ARITH),
    ("fsvec_sin_lowp_vec3", ARITH),
    ("fsvec_sin_vec2", ARITH),
    ("fsvec_tan_vec4", ARITH),
    ("matdiv_hp_mat3_float", ARITH),
    ("matdiv_hp_mat3_mat3", ARITH),
    ("vswide_cos_vec4", ARITH),
    ("vswide_exp2_vec4", ARITH),
    ("vswide_log2_vec4", ARITH),
    ("vswide_sin_vec3", ARITH),
    ("vswide_sin_vec4", ARITH),
    ("vswide_tan_vec3", ARITH),
];

/// `MaliError::Arithmetic`.
const ARITH: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("fsrcp_diff", 11),
    ("fsrcp_prod", 11),
    ("fsvec_cos_vec3", 41),
    ("fsvec_log2_vec3", 26),
    ("fsvec_log_vec2", 26),
    ("fsvec_pow_vec4", 46),
    ("fsvec_sin_lowp_vec3", 41),
    ("fsvec_sin_vec2", 29),
    ("fsvec_tan_vec4", 70),
    ("matdiv_hp_mat3_float", 21),
    ("matdiv_hp_mat3_mat3", 27),
    ("vswide_cos_vec4", 7),
    ("vswide_exp2_vec4", 7),
    ("vswide_log2_vec4", 7),
    ("vswide_sin_vec3", 7),
    ("vswide_sin_vec4", 7),
    ("vswide_tan_vec3", 7),
];

/// `1.0 / (in0.x - in0.y)` of a mediump varying's lanes (AR3's boundary): the
/// difference is `smoothstep`'s denominator form (`FADD.v2f16`, `FRCP.f16`), byte-equal.
#[test]
fn fsrcp_diff() {
    mali_ref::assert_matches("deqp_arithres_fsrcp_diff");
}

/// `1.0 / (in0.x * in0.y)`: `FRCP.f16` of a computed mediump lane (a product).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsrcp_prod() {
    mali_ref::assert_matches("deqp_arithres_fsrcp_prod");
}

/// `cos` of a mediump `vec3` (`v_in0.zxy`): three fp32 kernels on the load's words.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_cos_vec3() {
    mali_ref::assert_matches("deqp_arithres_fsvec_cos_vec3");
}

/// `log2` of a mediump `vec3`: three fp32 `log2` kernels, z narrowed alone, x and y as a pair.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_log2_vec3() {
    mali_ref::assert_matches("deqp_arithres_fsvec_log2_vec3");
}

/// `log` of a mediump `vec2`: two `log2` kernels, narrowed, times ln 2 at fp16.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_log_vec2() {
    mali_ref::assert_matches("deqp_arithres_fsvec_log_vec2");
}

/// `pow` of two mediump `vec4`s: four `log2`/`exp2` kernel pairs.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_pow_vec4() {
    mali_ref::assert_matches("deqp_arithres_fsvec_pow_vec4");
}

/// `sin` of a lowp `vec3`: the mediump form (lowp is mediump in Mesa).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_sin_lowp_vec3() {
    mali_ref::assert_matches("deqp_arithres_fsvec_sin_lowp_vec3");
}

/// `sin` of a mediump `vec2` (`v_in0.wy` into `color.yz`): two kernels, each lane narrowed alone.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_sin_vec2() {
    mali_ref::assert_matches("deqp_arithres_fsvec_sin_vec2");
}

/// `tan` of a mediump `vec4`: four shared sin/cos reductions and `FRCP.f16` of each cosine.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fsvec_tan_vec4() {
    mali_ref::assert_matches("deqp_arithres_fsvec_tan_vec4");
}

/// highp `mat3 / float` of uniforms in a fragment shader: one `FRCP.f32`, the
/// column sum factored over it.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn matdiv_hp_mat3_float() {
    mali_ref::assert_matches("deqp_arithres_matdiv_hp_mat3_float");
}

/// highp `mat3 / mat3` of uniforms in a fragment shader: nine `FRCP.f32`, fused
/// into the column sums' `FMA.f32` chains.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn matdiv_hp_mat3_mat3() {
    mali_ref::assert_matches("deqp_arithres_matdiv_hp_mat3_mat3");
}

/// vertex `cos` of a mediump `vec4`: four fp32 kernels, the lanes narrowed in pairs.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vswide_cos_vec4() {
    mali_ref::assert_matches("deqp_arithres_vswide_cos_vec4");
}

/// vertex `exp2` of a mediump `vec4` (`a_in0` whole): four `FMA_RSCALE`/`F32_TO_S32`/`FEXP` kernels.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vswide_exp2_vec4() {
    mali_ref::assert_matches("deqp_arithres_vswide_exp2_vec4");
}

/// vertex `log2` of a mediump `vec4`: four `log2` kernels.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vswide_log2_vec4() {
    mali_ref::assert_matches("deqp_arithres_vswide_log2_vec4");
}

/// vertex `sin` of a mediump `vec3` (`a_in0.zxy`): kernels in the swizzle's order, the
/// bias word moved to a register (`MOV.i32`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vswide_sin_vec3() {
    mali_ref::assert_matches("deqp_arithres_vswide_sin_vec3");
}

/// vertex `sin` of a mediump `vec4`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vswide_sin_vec4() {
    mali_ref::assert_matches("deqp_arithres_vswide_sin_vec4");
}

/// vertex `tan` of a mediump `vec3`: three shared reductions, an `FRCP.f32` of each cosine.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vswide_tan_vec3() {
    mali_ref::assert_matches("deqp_arithres_vswide_tan_vec3");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_arithres_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_arithres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_arithres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_arithres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_arithres_{name}.trace"));
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
/// lists cover every `deqp_arithres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_arithres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_arithres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_arithres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_arithres_{name}"));
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
