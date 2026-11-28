// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_texmiscres_*` (plan #387, item
//! texmiscres-refs, `docs/notes/mali-texmiscres-lowering.md`): the dEQP-GLES2
//! shapes whose first refusal at the integration tip `349f989f` is
//! `MaliError::TexVertex`, `MaliError::TexKind` or `MaliError::FragCoordMediump`
//! and that no `deqp_texmisc_*` or `deqp_fmaloc_*` reference covers. Every
//! reference was drawn with `tools/mali/deqp_ref.sh --instrument shim` (Mesa
//! 26.2.1's G610 drm-shim, `--uniform-values`) from the pair the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` logs (VK-GL-CTS 93bca01); each `.license`
//! sidecar names the case, and `tools/mali/nir_refs.sh` wrote each `.nir.sha`.
//! `tests/mali_ref` does the compare.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_texmiscres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_texmiscres_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("fcm_bool_ivec2_to_vec3", TEXT_FCM),
    ("fcm_int_to_mat3", TEXT_FCM),
    ("fcm_int_to_mat4", TEXT_FCM),
    ("fcm_ivec3_to_vec3", TEXT_FCM),
    ("fcm_ivec4_to_vec4", TEXT_FCM),
    ("fcm_rnd_cmp_ops", TEXT_FCM),
    ("fcm_rnd_conv_const", TEXT_FCM),
    ("fcm_rnd_cube_bias_neg", TEXT_FCM),
    ("tk_allf_proj_fma", TEXT_TK),
    ("tk_proj_vec3_bias", TEXT_TK),
    ("tk_proj_vec4_bias", TEXT_TK),
    ("tk_rnd_bias_dead_proj", TEXT_TK),
    ("tk_rnd_bias_zero", TEXT_TK),
    ("tk_rnd_bias_zero_dead_cube", TEXT_TK),
    ("tk_rnd_cube_bias_const", TEXT_TK),
    ("tk_rnd_cube_bias_dead2d", TEXT_TK),
    ("tk_rnd_proj_bias_comp", TEXT_TK),
    ("tk_rnd_proj_bias_whole", TEXT_TK),
    ("tk_rnd_proj_const_lane", TEXT_TK),
    ("tk_rnd_proj_mul", TEXT_TK),
    ("tv_rnd_cube_mixed", TEXT_TV),
];

const TEXT_TV: &str = "Valhall: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum, is not lowered yet";
const TEXT_TK: &str = "Valhall: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query) is not lowered yet";
const TEXT_FCM: &str = "Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("fcm_bool_ivec2_to_vec3", 32),
    ("fcm_int_to_mat3", 26),
    ("fcm_int_to_mat4", 33),
    ("fcm_ivec3_to_vec3", 35),
    ("fcm_ivec4_to_vec4", 40),
    ("fcm_rnd_cmp_ops", 9),
    ("fcm_rnd_conv_const", 13),
    ("fcm_rnd_cube_bias_neg", 29),
    ("tk_allf_proj_fma", 25),
    ("tk_proj_vec3_bias", 18),
    ("tk_proj_vec4_bias", 15),
    ("tk_rnd_bias_dead_proj", 19),
    ("tk_rnd_bias_zero", 11),
    ("tk_rnd_bias_zero_dead_cube", 9),
    ("tk_rnd_cube_bias_const", 27),
    ("tk_rnd_cube_bias_dead2d", 31),
    ("tk_rnd_proj_bias_comp", 25),
    ("tk_rnd_proj_bias_whole", 19),
    ("tk_rnd_proj_const_lane", 12),
    ("tk_rnd_proj_mul", 18),
    ("tv_rnd_cube_mixed", 10),
];

/// `shaders.conversions.vector_combine.bool_ivec2_to_vec3_fragment`: a three-lane fp16 harness of a bool lane and two `ftrunc` lanes (step 2, width 3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_bool_ivec2_to_vec3() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_bool_ivec2_to_vec3");
}

/// `shaders.conversions.scalar_to_matrix.int_to_mat3_fragment`: `mat3(int)`, the `ftrunc` lane beside zero halves, the column distances joined by `FMAX.v2f16` (step 2, width 3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_int_to_mat3() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_int_to_mat3");
}

/// `shaders.conversions.scalar_to_matrix.int_to_mat4_fragment`: `mat4(int)`, refused at the pair arm with no texel or narrowed operand (`nir_fs.rs:3794`; step 2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_int_to_mat4() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_int_to_mat4");
}

/// `shaders.conversions.vector_to_vector.ivec3_to_vec3_fragment`: three `F16_TO_S32.rtz` / `S32_TO_F32` lanes, the harness as a pair and a lone lane (step 2, width 3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_ivec3_to_vec3() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_ivec3_to_vec3");
}

/// `shaders.conversions.vector_to_vector.ivec4_to_vec4_fragment`: four int round-trip lanes, two pair harnesses joined by `LSHIFT_AND.v2i16` (step 2, width 4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_ivec4_to_vec4() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_ivec4_to_vec4");
}

/// `shaders.random.comparison_ops.fragment.26`: no int word left in Mesa's program, a pair of uniform halves (`nir_fs.rs:3794`; step 6).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_rnd_cmp_ops() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_rnd_cmp_ops");
}

/// `shaders.random.scalar_conversion.fragment.40`: `S32_TO_F32` of an int uniform beside a constant half (`nir_fs.rs:3799`; step 6).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_rnd_conv_const() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_rnd_conv_const");
}

/// `shaders.random.texture.fragment.37`: a lone fp16 `Fneg` (`nir_fs.rs:3858`) before a cube look-up with a computed bias (step 6, then step 5).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fcm_rnd_cube_bias_neg() {
    mali_ref::assert_matches("deqp_texmiscres_fcm_rnd_cube_bias_neg");
}

/// `shaders.random.all_features.fragment.60`: `texture2DProj(k, n + m * l.yxz)`, the coordinate as two `FMA.v2f16` (step 3, computed coordinate).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_allf_proj_fma() {
    mali_ref::assert_matches("deqp_texmiscres_tk_allf_proj_fma");
}

/// `shaders.texture_functions.fragment.texture2dproj_vec3_bias`: projection and a varying bias, `2d.computed_bias` (step 4, projection with bias).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_proj_vec3_bias() {
    mali_ref::assert_matches("deqp_texmiscres_tk_proj_vec3_bias");
}

/// `shaders.texture_functions.fragment.texture2dproj_vec4_bias`: the same with a `vec4` coordinate (step 4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_proj_vec4_bias() {
    mali_ref::assert_matches("deqp_texmiscres_tk_proj_vec4_bias");
}

/// `shaders.random.texture.fragment.99`: a dead bias look-up before a live projection of `c + vec3(true, b, -5)` (step 4 dead look-up, then step 3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_bias_dead_proj() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_bias_dead_proj");
}

/// `shaders.random.texture.fragment.70`: `texture2DProj(.., float(bool(b)))` whose bias folds to 0.0: Mesa's plain `2d.computed` (TM1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_bias_zero() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_bias_zero");
}

/// `shaders.random.texture.fragment.26`: a dead cube look-up and a bias `float(false) * x` that folds to 0.0 on a constant coordinate (TM1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_bias_zero_dead_cube() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_bias_zero_dead_cube");
}

/// `shaders.random.texture.fragment.77`: `textureCube(a, vec3(d, c, -3), b + 0.75)` after a dead 2D look-up (step 5).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_cube_bias_const() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_cube_bias_const");
}

/// `shaders.random.texture.fragment.139`: a dead 2D projection with bias, then a cube look-up of a computed direction and bias (step 5).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_cube_bias_dead2d() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_cube_bias_dead2d");
}

/// `shaders.random.texture.fragment.76`: `texture2DProj(a, e - vec3(d, b, c), b - -8.25)`, computed coordinate and bias (step 4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_proj_bias_comp() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_proj_bias_comp");
}

/// `shaders.random.texture.fragment.81`: `texture2DProj(a, d, c + b)`, a varying read whole and a bias sum (step 4).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_proj_bias_whole() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_proj_bias_whole");
}

/// `shaders.random.texture.fragment.123`: `texture2DProj(a, vec3(c, true, b))`, a constant lane in the coordinate (step 3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_proj_const_lane() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_proj_const_lane");
}

/// `shaders.random.texture.fragment.119`: `texture2DProj(a, vec3(e, d, c) * b)` with an int lane, fp32 products (step 3).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tk_rnd_proj_mul() {
    mali_ref::assert_matches("deqp_texmiscres_tk_rnd_proj_mul");
}

/// `uniform_api.random.6`: the cube `compare_vec4` harness joined with int, bvec4, vec3 and struct mat4 uniform compares (step 1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn tv_rnd_cube_mixed() {
    mali_ref::assert_matches("deqp_texmiscres_tv_rnd_cube_mixed");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_texmiscres_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_texmiscres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_texmiscres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_texmiscres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_texmiscres_{name}.trace"));
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
/// lists cover every `deqp_texmiscres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_texmiscres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_texmiscres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_texmiscres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_texmiscres_{name}"));
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
