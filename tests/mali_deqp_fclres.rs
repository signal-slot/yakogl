// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_fclres_*` (plan #387, item
//! fclres-refs, `docs/notes/mali-fclres-lowering.md`): the residue of the
//! refusal "a gl_FragColor lane other than the varying's own lane or a
//! constant" after fclane-lower-1, the shapes the earlier `deqp_fclane_*`,
//! `deqp_fclane2_*` and other pinned references do not already cover.
//! `tests/mali_ref` does the compare. Every pair is a dEQP-GLES2 case's
//! verbatim (the host `deqp-gles2` .qpa of `tools/mali/deqp-host-measure.sh`),
//! drawn with `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610
//! drm-shim, `--uniform-values`). The shape letters (S1..S11) are the note's.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fclres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_fclres_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("ceil_vec4_mp", TEXT_LANE),
    ("div_vec3_lp", TEXT_LANE),
    ("exp_vec4_mp", TEXT_LANE),
    ("f2f32_swizzle", TEXT_LANE),
    ("k0_min_vec2_mp", TEXT_LANE),
    ("k0_mul_vec2_lp", TEXT_LANE),
    ("k0_sub_vec2_mp", TEXT_LANE),
    ("kneg_random", TEXT_LANE),
    ("link_conv_random", TEXT_LANE),
    ("link_dup_mat2", TEXT_LANE),
    ("mat2_add_mp", TEXT_LANE),
    ("mat3_add_hp", TEXT_LANE),
    ("mat3_mul_const_mp", TEXT_LANE),
    ("matsub_loop", TEXT_LANE),
    ("mod_vec3_mp", TEXT_LANE),
    ("sign_vec4_mp", TEXT_LANE),
    ("struct_local_int", TEXT_LANE),
    ("swz_add_vec4_lp", TEXT_LANE),
    ("swz_inc_vec4_mp", TEXT_LANE),
    ("swz_max_vec4_mp", TEXT_LANE),
    ("vec2_mat2_mul_mp", TEXT_LANE),
    ("vecsub_loop", TEXT_LANE),
];

const TEXT_LANE: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("ceil_vec4_mp", 17),
    ("div_vec3_lp", 16),
    ("exp_vec4_mp", 30),
    ("f2f32_swizzle", 17),
    ("k0_min_vec2_mp", 15),
    ("k0_mul_vec2_lp", 18),
    ("k0_sub_vec2_mp", 18),
    ("kneg_random", 12),
    ("link_conv_random", 7),
    ("link_dup_mat2", 7),
    ("mat2_add_mp", 12),
    ("mat3_add_hp", 27),
    ("mat3_mul_const_mp", 7),
    ("matsub_loop", 22),
    ("mod_vec3_mp", 32),
    ("sign_vec4_mp", 26),
    ("struct_local_int", 12),
    ("swz_add_vec4_lp", 15),
    ("swz_inc_vec4_mp", 17),
    ("swz_max_vec4_mp", 15),
    ("vec2_mat2_mul_mp", 17),
    ("vecsub_loop", 14),
];

/// `ceil(in0)` of a mediump vec4 in the operator harness (`.yzwx` swizzle): a pair of `Fceil` lanes (S8, `FROUND.v2f16.rtp`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ceil_vec4_mp() {
    mali_ref::assert_matches("deqp_fclres_ceil_vec4_mp");
}

/// `in0 /= in1` of a lowp vec3 by a float: a pair of `Frcp` of the one scalar (S8, `FRCP.f16` once, `.h00` twice).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn div_vec3_lp() {
    mali_ref::assert_matches("deqp_fclres_div_vec3_lp");
}

/// `exp(in0)` of a mediump vec4: a pair of `Fexp2` lanes (S8, fp32 `FEXP.f32` per lane, then pair converts).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn exp_vec4_mp() {
    mali_ref::assert_matches("deqp_fclres_exp_vec4_mp");
}

/// shaders.random.swizzle.fragment.26: a pair of `F2f32` lanes of mediump arithmetic (S8).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn f2f32_swizzle() {
    mali_ref::assert_matches("deqp_fclres_f2f32_swizzle");
}

/// `min(in0, in1)` of a mediump vec2 and a float: the (0.0, computed lane) word (S6, `MKVEC.v2i16 r0, 0x0.h0, r0^.h0`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k0_min_vec2_mp() {
    mali_ref::assert_matches("deqp_fclres_k0_min_vec2_mp");
}

/// `in0 *= in1` of lowp vec2s: the (0.0, computed lane) word (S6).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k0_mul_vec2_lp() {
    mali_ref::assert_matches("deqp_fclres_k0_mul_vec2_lp");
}

/// `in0 -= in1` of mediump vec2s: the (0.0, computed lane) word (S6).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k0_sub_vec2_mp() {
    mali_ref::assert_matches("deqp_fclres_k0_sub_vec2_mp");
}

/// shaders.random.basic_expression.combined.76: a (constant, `-uniform`) pair (S6, a uniform in a word).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn kneg_random() {
    mali_ref::assert_matches("deqp_fclres_kneg_random");
}

/// shaders.random.*.vertex.16: `v_color = vec4(b, a, float(a), 1.0)` and a
/// plain `gl_FragColor = v_color`, refused when the program relinks the
/// fragment shader (S14b: Mesa's z is the pair's `.h1`, `MKVEC.v2i16 r1,
/// r0.h1, u2.w0.h0`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_conv_random() {
    mali_ref::assert_matches("deqp_fclres_link_conv_random");
}

/// matrix.mul const_mediump_vec2_mat2_vertex: `v_color = vec4(res.x, res.y,
/// res.x, 1.0)`, refused when the program relinks the fragment shader (S14a:
/// Mesa loads the four lanes, two pair converts).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn link_dup_mat2() {
    mali_ref::assert_matches("deqp_fclres_link_dup_mat2");
}

/// `m0 + m1` of dynamic mediump mat2s: the colour pair reads lanes of two loads (S2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mat2_add_mp() {
    mali_ref::assert_matches("deqp_fclres_mat2_add_mp");
}

/// `m0 + m1` of dynamic highp mat3s: two lanes of a matrix column the link keeps as one varying (S11 lane/lane).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mat3_add_hp() {
    mali_ref::assert_matches("deqp_fclres_mat3_add_hp");
}

/// `in0 * in1` of two constant mediump mat3s: the product is not folded (S11 `Ffma`/`Undef`; Mesa folds it to two FAU words).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mat3_mul_const_mp() {
    mali_ref::assert_matches("deqp_fclres_mat3_mul_const_mp");
}

/// indexing.matrix_subscript mat4 static loop write, static read: (y, z) of one load (S1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn matsub_loop() {
    mali_ref::assert_matches("deqp_fclres_matsub_loop");
}

/// `mod(in0, in1)` of mediump vec3s: a pair of `Ffloor` lanes (S8).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mod_vec3_mp() {
    mali_ref::assert_matches("deqp_fclres_mod_vec3_mp");
}

/// `sign(in0)` of a mediump vec4: a pair of `B2f` of compares (S8, `FCMP_OR.v2f16.lt.m1` and `MUX.v2i16`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sign_vec4_mp() {
    mali_ref::assert_matches("deqp_fclres_sign_vec4_mp");
}

/// struct.local return_nested: (a varying lane, `float(int + int)` of uniforms) (S11 lane/`I2f`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn struct_local_int() {
    mali_ref::assert_matches("deqp_fclres_struct_local_int");
}

/// `in0 += in1` of a lowp vec4 (`.yzwx`) by a float: (y, z) of one load (S1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn swz_add_vec4_lp() {
    mali_ref::assert_matches("deqp_fclres_swz_add_vec4_lp");
}

/// `++in0` of a mediump vec4 (`.yzwx`): (y, z) of one load (S1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn swz_inc_vec4_mp() {
    mali_ref::assert_matches("deqp_fclres_swz_inc_vec4_mp");
}

/// `max(in0, in1)` of a mediump vec4 (`.yzwx`) and a float: (y, z) of one load (S1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn swz_max_vec4_mp() {
    mali_ref::assert_matches("deqp_fclres_swz_max_vec4_mp");
}

/// `v * m` of a dynamic mediump vec2 and mat2: the colour pair reads lanes of two loads (S2).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vec2_mat2_mul_mp() {
    mali_ref::assert_matches("deqp_fclres_vec2_mat2_mul_mp");
}

/// indexing.vector_subscript vec4 static write, static loop read: (y, z) of one load (S1).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vecsub_loop() {
    mali_ref::assert_matches("deqp_fclres_vecsub_loop");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_fclres_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_fclres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_fclres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_fclres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_fclres_{name}.trace"));
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
/// lists cover every `deqp_fclres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_fclres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_fclres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fclres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fclres_{name}"));
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
