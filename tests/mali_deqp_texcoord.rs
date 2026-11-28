// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_texcoord_*` (plan #387, item
//! texcoord-design, `docs/notes/mali-texcoord-lowering.md`): the dEQP-GLES2
//! shapes the "texture coordinate" refusal (`MaliError::TexCoord`) blocks that
//! the four `deqp_fmaloc_tc_*` references (`tests/mali_deqp_fmaloc.rs`) do not
//! cover: the `uniform_api` constant cube directions beside other look-ups,
//! the `shaders.discard` texture coordinate, and the `shaders.random.texture`
//! cube directions (implicit level and bias) and computed 2D coordinates.
//! `tests/mali_ref` does the compare. Every pair is verbatim from the .qpa of
//! `tools/mali/deqp-host-measure.sh` (HOST-ONLY run at 6e9e515c) and was drawn
//! with `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610
//! drm-shim, `--uniform-values`); the five `uapi_*` pairs with a scratch
//! `gl_ref.py` that gives each struct-member sampler its own unit (the note,
//! section 3: the words are the same with the units reversed).
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_texcoord
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_texcoord_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("disc_texture", TEXT_TC),
    ("rnd_2d_const_sub", TEXT_TC),
    ("rnd_2d_splat_mul", TEXT_TC),
    ("rnd_2d_vec_sub", TEXT_TC),
    ("rnd_add_const", TEXT_TC),
    ("rnd_bias_const", TEXT_TC),
    ("rnd_bias_ivec", TEXT_TC),
    ("rnd_bias_mul", TEXT_TC),
    ("rnd_int_lane", TEXT_TC),
    ("rnd_swizzle_const", TEXT_TC),
    ("uapi_array_in_struct", TEXT_TC),
    ("uapi_struct_cube", TEXT_TC),
    ("uapi_struct_cube_both", TEXT_TC),
    ("uapi_struct_in_array", TEXT_TC),
    ("uapi_unused_cube_elem", TEXT_TC),
];

const TEXT_TC: &str =
    "Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("disc_texture", 17),
    ("rnd_2d_const_sub", 14),
    ("rnd_2d_splat_mul", 14),
    ("rnd_2d_vec_sub", 15),
    ("rnd_add_const", 18),
    ("rnd_bias_const", 30),
    ("rnd_bias_ivec", 26),
    ("rnd_bias_mul", 26),
    ("rnd_int_lane", 19),
    ("rnd_swizzle_const", 19),
    ("uapi_array_in_struct", 104),
    ("uapi_struct_cube", 38),
    ("uapi_struct_cube_both", 40),
    ("uapi_struct_in_array", 82),
    ("uapi_unused_cube_elem", 38),
];

/// `texture2D(ut_brick, v_coords.xy*0.25+0.5).x < 0.7` before a `discard`, `v_coords` a mediump vec4 varying whose load Mesa shares with `v_color` (`shaders.discard.basic_texture`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn disc_texture() {
    mali_ref::assert_matches("deqp_texcoord_disc_texture");
}

/// `texture2D(a, vec2(false, true) - b)`, `b` a mediump vec2 varying: one `FADD.v2f16` of the constant pair minus the converted pair (`shaders.random.texture.fragment.43`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_2d_const_sub() {
    mali_ref::assert_matches("deqp_texcoord_rnd_2d_const_sub");
}

/// `texture2D(k, vec2(g, g) * l)` with `g` the constant 2: Mesa's `FADD.v2f16 r0, r0^, r0^` of the converted pair (`shaders.random.texture.fragment.85`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_2d_splat_mul() {
    mali_ref::assert_matches("deqp_texcoord_rnd_2d_splat_mul");
}

/// `texture2D(a, vec2(d, c) - vec2(false, b))`: a scalar varying and a constant int lane paired by `MKVEC.v2i16` with a pushed fp16 word, then `FADD_IMM.v2f16` (`shaders.random.texture.fragment.11`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_2d_vec_sub() {
    mali_ref::assert_matches("deqp_texcoord_rnd_2d_vec_sub");
}

/// `textureCube(a, b + vec3(4.75, 0, 1.5))` whose direction folds to constants and one uniform lane: `CUBEFACE1` of an `IADD_IMM` constant, `-1.0` and `u2.w0` (`shaders.random.texture.fragment.134`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_add_const() {
    mali_ref::assert_matches("deqp_texcoord_rnd_add_const");
}

/// `textureCube(a, d - vec3(0, c, b), 6.5)`: a vec3 varying minus a vector of a constant and two scalar varyings, a constant bias (`.computed_bias`; `shaders.random.texture.fragment.8`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_bias_const() {
    mali_ref::assert_matches("deqp_texcoord_rnd_bias_const");
}

/// `textureCube(a, vec3(e, d, c), b + b)`: an int global, an int uniform and a float uniform as the direction (`S32_TO_F32`, `F16_TO_F32`), a varying sum as the bias (`shaders.random.texture.fragment.0`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_bias_ivec() {
    mali_ref::assert_matches("deqp_texcoord_rnd_bias_ivec");
}

/// `textureCube(a, d * vec3(1, c, b), b - b)`: a vec3 varying times a vector of a constant, a global and a varying; the bias `b - b` folds (`shaders.random.texture.fragment.92`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_bias_mul() {
    mali_ref::assert_matches("deqp_texcoord_rnd_bias_mul");
}

/// `textureCube(a, vec3(d, c, int(b)))`: an int uniform, a varying and `int()` of a varying (`FROUND.f32.rtz`) as the three lanes (`shaders.random.texture.fragment.38`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_int_lane() {
    mali_ref::assert_matches("deqp_texcoord_rnd_int_lane");
}

/// `textureCube(a, vec4(e, d, c, b).arg)` of constants and int globals: the direction is constant words and one pushed word (`shaders.random.texture.fragment.22`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_swizzle_const() {
    mali_ref::assert_matches("deqp_texcoord_rnd_swizzle_const");
}

/// `uniform_api`'s compare harness over a struct of `sampler2D`, `samplerCube` and arrays of both, every look-up at `vec2(0.0)` / `vec3(0.0)`: Mesa computes the constant cube sequence once and every cube look-up reads it (`uniform_api.info_query.array_in_struct.sampler2D_samplerCube_fragment`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_array_in_struct() {
    mali_ref::assert_matches("deqp_texcoord_uapi_array_in_struct");
}

/// The compare harness over `struct { sampler2D m0; samplerCube m1; }`: one 2D look-up at `vec2(0.0)`, one cube look-up at `vec3(0.0)` (`uniform_api.info_query.basic_struct.sampler2D_samplerCube_fragment`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_struct_cube() {
    mali_ref::assert_matches("deqp_texcoord_uapi_struct_cube");
}

/// `uapi_struct_cube` with the same look-ups in the vertex shader too (`..._both`; the fragment shader reads the vertex result through `v_vtxOut`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_struct_cube_both() {
    mali_ref::assert_matches("deqp_texcoord_uapi_struct_cube_both");
}

/// The compare harness over `structType u_var[3]`, a 2D and a cube look-up per element (`uniform_api.info_query.struct_in_array.sampler2D_samplerCube_fragment`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_struct_in_array() {
    mali_ref::assert_matches("deqp_texcoord_uapi_struct_in_array");
}

/// The compare harness reading `u_var.m0` and the cube array element `u_var.m3[1]` only (`uniform_api.info_query.unused_uniforms.sampler2D_samplerCube_fragment`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_unused_cube_elem() {
    mali_ref::assert_matches("deqp_texcoord_uapi_unused_cube_elem");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_texcoord_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_texcoord_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_texcoord_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_texcoord_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_texcoord_{name}.trace"));
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
/// lists cover every `deqp_texcoord_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_texcoord_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_texcoord_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_texcoord_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_texcoord_{name}"));
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
