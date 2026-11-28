// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_varyres_*` (plan #387, item
//! varyres-refs, `docs/notes/mali-varyres-lowering.md`): the dEQP-GLES2
//! shapes whose first refusal at `349f989f` is `FsVaryingLoad` ("a fragment
//! varying load or conversion of a form no reference draws") and that no
//! earlier reference pins: the highp varyings packed across a slot beside a
//! mediump colour (steps VR1a-VR1c, `nir_fs.rs` `layout`), a colour read
//! through a function's dead return (VR2, `colour_keeps_whole`), and the
//! random, unary and varying-array shapes that `varyload-lower-1`
//! (`conversion_order`) reaches. `tests/mali_ref` does the compare. Every
//! reference was drawn with `tools/mali/deqp_ref.sh --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`) from the verbatim
//! shaders of a HOST-ONLY `tools/mali/deqp-host-measure.sh` run; each
//! `.license` names its case, and `tools/mali/nir_refs.sh` wrote the
//! `.nir.sha`.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varyres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_varyres_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("interact17", TEXT_LOAD),
    ("mp_vec3_postinc", TEXT_LOAD),
    ("ret_never", TEXT_LOAD),
    ("rnd_basic30", TEXT_LOAD),
    ("rnd_basic71", TEXT_LOAD),
    ("rnd_cmp15", TEXT_LOAD),
    ("rnd_cmp6", TEXT_LOAD),
    ("rnd_cond75", TEXT_LOAD),
    ("rnd_sconv42", TEXT_LOAD),
    ("rnd_sconv6", TEXT_LOAD),
    ("rnd_sconv65", TEXT_LOAD),
    ("rnd_swz15", TEXT_LOAD),
    ("rnd_swz36", TEXT_LOAD),
    ("rnd_swz47", TEXT_LOAD),
    ("st_clamp_vec2", TEXT_LOAD),
    ("st_clamp_vec3", TEXT_LOAD),
    ("st_float_vec2", TEXT_LOAD),
    ("st_int_ivec3", TEXT_LOAD),
    ("st_ivec2_int", TEXT_LOAD),
    ("st_ivec3_int", TEXT_LOAD),
    ("st_mix_vec2", TEXT_LOAD),
    ("st_mix_vec3", TEXT_LOAD),
    ("st_smooth_vec2", TEXT_LOAD),
    ("st_vec2_float", TEXT_LOAD),
    ("st_vec3_float", TEXT_LOAD),
    ("vary_array_vec2", TEXT_LOAD),
];

const TEXT_LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("interact17", 10),
    ("mp_vec3_postinc", 14),
    ("ret_never", 8),
    ("rnd_basic30", 14),
    ("rnd_basic71", 15),
    ("rnd_cmp15", 14),
    ("rnd_cmp6", 11),
    ("rnd_cond75", 11),
    ("rnd_sconv42", 12),
    ("rnd_sconv6", 11),
    ("rnd_sconv65", 8),
    ("rnd_swz15", 18),
    ("rnd_swz36", 9),
    ("rnd_swz47", 11),
    ("st_clamp_vec2", 19),
    ("st_clamp_vec3", 19),
    ("st_float_vec2", 13),
    ("st_int_ivec3", 25),
    ("st_ivec2_int", 19),
    ("st_ivec3_int", 23),
    ("st_mix_vec2", 19),
    ("st_mix_vec3", 19),
    ("st_smooth_vec2", 26),
    ("st_vec2_float", 14),
    ("st_vec3_float", 16),
    ("vary_array_vec2", 14),
];

/// `fragment_ops.interaction.basic_shader.17` (varyload-lower-1, byte-equal with lower-mixprec3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn interact17() {
    mali_ref::assert_matches("deqp_varyres_interact17");
}

/// `shaders.operator.unary_operator.post_increment_result.mediump_vec3_fragment` (varyload-lower-1, byte-equal with lower-mixprec3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mp_vec3_postinc() {
    mali_ref::assert_matches("deqp_varyres_mp_vec3_postinc");
}

/// `shaders.return.conditional_return_never_fragment` (VR2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ret_never() {
    mali_ref::assert_matches("deqp_varyres_ret_never");
}

/// `shaders.random.basic_expression.fragment.30` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_basic30() {
    mali_ref::assert_matches("deqp_varyres_rnd_basic30");
}

/// `shaders.random.basic_expression.fragment.71` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_basic71() {
    mali_ref::assert_matches("deqp_varyres_rnd_basic71");
}

/// `shaders.random.comparison_ops.fragment.15` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_cmp15() {
    mali_ref::assert_matches("deqp_varyres_rnd_cmp15");
}

/// `shaders.random.comparison_ops.fragment.6` (varyload-lower-1, then the link-offsets refusal).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_cmp6() {
    mali_ref::assert_matches("deqp_varyres_rnd_cmp6");
}

/// `shaders.random.conditionals.fragment.75` (varyload-lower-1, then the link-offsets refusal).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_cond75() {
    mali_ref::assert_matches("deqp_varyres_rnd_cond75");
}

/// `shaders.random.scalar_conversion.combined.42` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_sconv42() {
    mali_ref::assert_matches("deqp_varyres_rnd_sconv42");
}

/// `shaders.random.scalar_conversion.fragment.6` (varyload-lower-1, then the link-offsets refusal).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_sconv6() {
    mali_ref::assert_matches("deqp_varyres_rnd_sconv6");
}

/// `shaders.random.scalar_conversion.fragment.65` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_sconv65() {
    mali_ref::assert_matches("deqp_varyres_rnd_sconv65");
}

/// `shaders.random.swizzle.fragment.15` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_swz15() {
    mali_ref::assert_matches("deqp_varyres_rnd_swz15");
}

/// `shaders.random.swizzle.fragment.36` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_swz36() {
    mali_ref::assert_matches("deqp_varyres_rnd_swz36");
}

/// `shaders.random.swizzle.fragment.47` (VR3, compiles with lower-mixprec3 but differs).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_swz47() {
    mali_ref::assert_matches("deqp_varyres_rnd_swz47");
}

/// `shaders.operator.common_functions.clamp.highp_vec2_float_fragment` (VR1c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_clamp_vec2() {
    mali_ref::assert_matches("deqp_varyres_st_clamp_vec2");
}

/// `shaders.operator.common_functions.clamp.highp_vec3_float_fragment` (VR1c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_clamp_vec3() {
    mali_ref::assert_matches("deqp_varyres_st_clamp_vec3");
}

/// `shaders.operator.binary_operator.add.highp_float_vec2_fragment` (VR1b).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_float_vec2() {
    mali_ref::assert_matches("deqp_varyres_st_float_vec2");
}

/// `shaders.operator.binary_operator.add.highp_int_ivec3_fragment` (VR1b).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_int_ivec3() {
    mali_ref::assert_matches("deqp_varyres_st_int_ivec3");
}

/// `shaders.operator.binary_operator.add.highp_ivec2_int_fragment` (VR1a).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_ivec2_int() {
    mali_ref::assert_matches("deqp_varyres_st_ivec2_int");
}

/// `shaders.operator.binary_operator.add.highp_ivec3_int_fragment` (VR1a).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_ivec3_int() {
    mali_ref::assert_matches("deqp_varyres_st_ivec3_int");
}

/// `shaders.operator.common_functions.mix.highp_vec2_float_fragment` (VR1c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_mix_vec2() {
    mali_ref::assert_matches("deqp_varyres_st_mix_vec2");
}

/// `shaders.operator.common_functions.mix.highp_vec3_float_fragment` (VR1c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_mix_vec3() {
    mali_ref::assert_matches("deqp_varyres_st_mix_vec3");
}

/// `shaders.operator.common_functions.smoothstep.highp_float_vec2_fragment` (VR1c).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_smooth_vec2() {
    mali_ref::assert_matches("deqp_varyres_st_smooth_vec2");
}

/// `shaders.operator.binary_operator.add.highp_vec2_float_fragment` (VR1a).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_vec2_float() {
    mali_ref::assert_matches("deqp_varyres_st_vec2_float");
}

/// `shaders.operator.binary_operator.add.highp_vec3_float_fragment` (VR1a).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn st_vec3_float() {
    mali_ref::assert_matches("deqp_varyres_st_vec3_float");
}

/// `shaders.indexing.varying_array.vec2_static_write_static_read` (varyload-lower-1, byte-equal with lower-mixprec3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vary_array_vec2() {
    mali_ref::assert_matches("deqp_varyres_vary_array_vec2");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_varyres_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_varyres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_varyres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_varyres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_varyres_{name}.trace"));
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
/// lists cover every `deqp_varyres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_varyres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_varyres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_varyres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_varyres_{name}"));
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
