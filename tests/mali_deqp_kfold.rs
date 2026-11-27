// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_kfold_*` (plan #387, item
//! kfold-refs, `docs/notes/mali-kfold-lowering.md`): the dEQP-GLES2 shapes of
//! the constant-fold steps the second lowering train left open, constprod K2
//! (`k2_*`: products of negated literals and literal chains), the rest of K4
//! (`k4_*`: built-ins on constants that still reach a walk's `ffma`), K5
//! (`k5_*`: `asin`/`acos`/`atan`/`atan2` of a vertex value) and fix C of
//! `docs/notes/mali-deqp-defects.md` (`c_*`: the `lane_tree`/`vectorise`
//! cycle, `Defect(LaneCycle)`). `tests/mali_ref` does the compare.
//!
//! Every reference was drawn with `tools/mali/deqp_ref.sh --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`). The `k4_*` pairs are
//! generated from the dEQP templates (`es2fShaderConstExprTests.cpp`,
//! `glsShaderConstExprTests.cpp`, `glsShaderLibraryCase.cpp` at VK-GL-CTS
//! 93bca01; the generator reproduces the `.qpa`-verbatim
//! `deqp_constprod_foldcall_*` pairs byte for byte); the others are verbatim
//! from the `.qpa` of `tools/mali/deqp-host-measure.sh` at 4d8a0b40. Each
//! shader's `.license` names its case.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the word counts the note
//! quotes ([`WORDS`]); `folded_bits` checks that the constants Mesa folded
//! into the K4 and K5 references are the values of the shared helper
//! `src/shaderc/online/mesa_fold.rs` (included by path, as
//! `tests/mali_libm_fold.rs` does), so the lowering has its bits.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_kfold
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[path = "../src/shaderc/online/fmath.rs"]
#[allow(dead_code)]
mod fmath;
#[path = "../src/shaderc/online/mesa_fold.rs"]
#[allow(dead_code)]
mod mesa_fold;

/// The references a lowering step made byte-equal, without the
/// `deqp_kfold_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("c_cmp29_fs", TEXT_LANECYCLE),
    ("c_conv29_fs", TEXT_LANECYCLE),
    ("c_conv74_fs", TEXT_LANECYCLE),
    ("c_expr58_fs", TEXT_LANECYCLE),
    ("k2_chain_cond_vs", TEXT_CONST),
    ("k2_chain_conv_combined", TEXT_CONST),
    ("k2_chain_swizzle_fs", TEXT_CONST),
    ("k2_neglit_conv_fs", TEXT_CONST),
    ("k2_neglit_expr_vs", TEXT_CONST),
    ("k2_neglit_texture_fs", TEXT_CONST),
    ("k2_neglit_trig_vs", TEXT_CONST),
    ("k4_acos_vec2_fs", TEXT_CONST),
    ("k4_atan2_vec4_fs", TEXT_CONST),
    ("k4_mod_vec3_float_vs", TEXT_FMA),
    ("k4_mod_vec4_vs", TEXT_FMA),
    ("k4_reflect_vec3_fs", TEXT_CONST),
    ("k4_refract_float_fs", TEXT_CONST),
    ("k4_refract_vec4_fs", TEXT_CONST),
    ("k5_acos_highp_vec4_vs", TEXT_FMA),
    ("k5_asin_mediump_vs", TEXT_FMA),
    ("k5_atan2_mediump_vs", TEXT_FMA),
    ("k5_atan_mediump_vec2_vs", TEXT_FMA),
    ("k5_random_allfeat51_vs", TEXT_FMA),
    ("k5_random_trig17_vs", TEXT_FMA),
];

const TEXT_CONST: &str = "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet";
const TEXT_FMA: &str =
    "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet";
const TEXT_LANECYCLE: &str = "Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included) and of its `vs.varying`
/// region (0: none, the varyings are constants linked into the fragment
/// shader).
const WORDS: &[(&str, usize, usize)] = &[
    ("c_cmp29_fs", 9, 3),
    ("c_conv29_fs", 9, 3),
    ("c_conv74_fs", 17, 13),
    ("c_expr58_fs", 13, 8),
    ("k2_chain_cond_vs", 7, 29),
    ("k2_chain_conv_combined", 7, 0),
    ("k2_chain_swizzle_fs", 9, 15),
    ("k2_neglit_conv_fs", 10, 3),
    ("k2_neglit_expr_vs", 7, 20),
    ("k2_neglit_texture_fs", 18, 0),
    ("k2_neglit_trig_vs", 7, 91),
    ("k4_acos_vec2_fs", 11, 0),
    ("k4_atan2_vec4_fs", 13, 0),
    ("k4_mod_vec3_float_vs", 12, 0),
    ("k4_mod_vec4_vs", 12, 0),
    ("k4_reflect_vec3_fs", 12, 0),
    ("k4_refract_float_fs", 12, 0),
    ("k4_refract_vec4_fs", 12, 0),
    ("k5_acos_highp_vec4_vs", 7, 62),
    ("k5_asin_mediump_vs", 7, 35),
    ("k5_atan2_mediump_vs", 7, 58),
    ("k5_atan_mediump_vec2_vs", 7, 49),
    ("k5_random_allfeat51_vs", 7, 144),
    ("k5_random_trig17_vs", 7, 47),
];

/* ---- fix C: the lane_tree / vectorise cycle (nir_fs.rs pair_node) ---- */

/// `shaders.random.comparison_ops.fragment.29`: `vec4(-1, g, 0, -0.25) *
/// vec4(f, c, e, d)` with constant-false `f`, `c`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_cmp29_fs() {
    mali_ref::assert_matches("deqp_kfold_c_cmp29_fs");
}

/// `shaders.random.scalar_conversion.fragment.29`: the same colour product as
/// `c_cmp29_fs` with other constant globals (the same Mesa words).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_conv29_fs() {
    mali_ref::assert_matches("deqp_kfold_c_conv29_fs");
}

/// `shaders.random.scalar_conversion.fragment.74`: a three-factor colour
/// product, `vec4(g, f, e, -0.125) * d * vec4(c, b, 0, a)`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_conv74_fs() {
    mali_ref::assert_matches("deqp_kfold_c_conv74_fs");
}

/// `shaders.random.basic_expression.fragment.58`: `vec4(o, n, m, l) *
/// vec4(m, l, 0.5, k)` of constant globals and varyings.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_expr58_fs() {
    mali_ref::assert_matches("deqp_kfold_c_expr58_fs");
}

/* ---- K2: products of negated literals, literal chains ---- */

/// `shaders.random.conditionals.vertex.12`: `float(0) * float(-1) * -0.5`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_chain_cond_vs() {
    mali_ref::assert_matches("deqp_kfold_k2_chain_cond_vs");
}

/// `shaders.random.scalar_conversion.combined.20`: `float(61.75) *
/// float(0.5) * -0.25`; Mesa's fragment program is a constant colour.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_chain_conv_combined() {
    mali_ref::assert_matches("deqp_kfold_k2_chain_conv_combined");
}

/// `shaders.random.swizzle.fragment.2`: `const float f = 16.0 * 0.25 * -0.25`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_chain_swizzle_fs() {
    mali_ref::assert_matches("deqp_kfold_k2_chain_swizzle_fs");
}

/// `shaders.random.scalar_conversion.fragment.10`: `(float(5.5)) - -5.0 * -1.0`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_neglit_conv_fs() {
    mali_ref::assert_matches("deqp_kfold_k2_neglit_conv_fs");
}

/// `shaders.random.basic_expression.vertex.42`: `(-8.0 * -1.5)`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_neglit_expr_vs() {
    mali_ref::assert_matches("deqp_kfold_k2_neglit_expr_vs");
}

/// `shaders.random.texture.fragment.64`: `float(1.5) - -0.25 * -2.0` beside a
/// texture look-up.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_neglit_texture_fs() {
    mali_ref::assert_matches("deqp_kfold_k2_neglit_texture_fs");
}

/// `shaders.random.trigonometric.vertex.34`: `-2.0 * float(float(0.25))`
/// beside an `acos` of a vertex value (K5 next).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k2_neglit_trig_vs() {
    mali_ref::assert_matches("deqp_kfold_k2_neglit_trig_vs");
}

/* ---- K4: built-ins on constants that reach a walk's ffma ---- */

/// `constant_expressions.builtin_functions.trigonometry.acos_vec2_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_acos_vec2_fs() {
    mali_ref::assert_matches("deqp_kfold_k4_acos_vec2_fs");
}

/// `constant_expressions.builtin_functions.trigonometry.atan_separate_vec4_fragment`
/// (the float case is `deqp_constop_ce_atan_separate_fs`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_atan2_vec4_fs() {
    mali_ref::assert_matches("deqp_kfold_k4_atan2_vec4_fs");
}

/// `constant_expressions.builtin_functions.common.mod_vec3_float_vertex`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_mod_vec3_float_vs() {
    mali_ref::assert_matches("deqp_kfold_k4_mod_vec3_float_vs");
}

/// `constant_expressions.builtin_functions.common.mod_vec4_vertex`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_mod_vec4_vs() {
    mali_ref::assert_matches("deqp_kfold_k4_mod_vec4_vs");
}

/// `constant_expressions.builtin_functions.geometric.reflect_vec3_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_reflect_vec3_fs() {
    mali_ref::assert_matches("deqp_kfold_k4_reflect_vec3_fs");
}

/// `constant_expressions.builtin_functions.geometric.refract_float_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_refract_float_fs() {
    mali_ref::assert_matches("deqp_kfold_k4_refract_float_fs");
}

/// `constant_expressions.builtin_functions.geometric.refract_vec4_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k4_refract_vec4_fs() {
    mali_ref::assert_matches("deqp_kfold_k4_refract_vec4_fs");
}

/* ---- K5: asin, acos, atan, atan2 of a vertex value ---- */

/// `shaders.operator.angle_and_trigonometry.acos.highp_vec4_vertex`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k5_acos_highp_vec4_vs() {
    mali_ref::assert_matches("deqp_kfold_k5_acos_highp_vec4_vs");
}

/// `shaders.operator.angle_and_trigonometry.asin.mediump_float_vertex`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k5_asin_mediump_vs() {
    mali_ref::assert_matches("deqp_kfold_k5_asin_mediump_vs");
}

/// `shaders.operator.angle_and_trigonometry.atan2.mediump_float_vertex`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k5_atan2_mediump_vs() {
    mali_ref::assert_matches("deqp_kfold_k5_atan2_mediump_vs");
}

/// `shaders.operator.angle_and_trigonometry.atan.mediump_vec2_vertex`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k5_atan_mediump_vec2_vs() {
    mali_ref::assert_matches("deqp_kfold_k5_atan_mediump_vec2_vs");
}

/// `shaders.random.all_features.vertex.51`: `sin`/`cos` and `acos` of
/// attributes in one vertex shader.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k5_random_allfeat51_vs() {
    mali_ref::assert_matches("deqp_kfold_k5_random_allfeat51_vs");
}

/// `shaders.random.trigonometric.vertex.17`: an `atan` of an attribute.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn k5_random_trig17_vs() {
    mali_ref::assert_matches("deqp_kfold_k5_random_trig17_vs");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_kfold_")?.strip_suffix(".expect.json").map(str::to_string))
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
/// prefix), `vs.position`, `vs.varying` (when there is one) and `fs.fragment`
/// have 64-bit hex words, and the counts are those of [`WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = WORDS.iter().map(|(n, ..)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "WORDS does not list every deqp_kfold_* reference");
    let mut bad = Vec::new();
    for (name, fs_words, varying_words) in WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_kfold_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_kfold_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_kfold_{name}.trace"));
        }
        for (region, want) in
            [("vs.position", None), ("vs.varying", Some(*varying_words)), ("fs.fragment", Some(*fs_words))]
        {
            match (region_words(&r.json, region), want) {
                (None, Some(0)) => {}
                (None, _) => bad.push(format!("{name}: no {region}")),
                (Some(Err(e)), _) => bad.push(format!("{name}: {e}")),
                (Some(Ok(0)), _) => bad.push(format!("{name}: {region} has no words")),
                (Some(Ok(n)), Some(w)) if n != w => bad.push(format!("{name}: {region} has {n} words, WORDS says {w}")),
                (Some(Ok(_)), _) => {}
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] is byte-equal, and the three
/// lists cover every `deqp_kfold_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_kfold_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_kfold_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_kfold_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_kfold_{name}"));
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

/// The expect.json of reference `name` (without the prefix).
fn json(name: &str) -> String {
    mali_ref::Reference::load(&format!("deqp_kfold_{name}")).json
}

/// The constants Mesa folded into the K4 and K5 references, from the helper
/// the lowering is to call (`docs/notes/mali-kfold-lowering.md` sections 3
/// and 4): each value is computed here and found among the reference's words.
#[test]
fn folded_bits() {
    use mesa_fold::{f32_to_f16, fold_glsl, Libm};
    /* K4 atan(y, x) of (-1.0, -1.0): GLSL IR's double atan2, rounded once
     * (mali-libm-fold.md 1.1), the FAU word of the fragment compare */
    let atan2 = fold_glsl(Libm::Atan2, -1.0, -1.0).to_bits();
    assert_eq!(atan2, 0xc016_cbe4);
    assert!(json("k4_atan2_vec4_fs").contains(&format!("\"{atan2:08x}\"")), "atan2 word");
    /* K4 mod(87.65, 3.7): `x - y * floorf(x / y)` unfused in f32, then the
     * mediump varying's fp16, as an IADD_IMM pair of the fragment shader */
    let (x, y) = (87.65f32, 3.7f32);
    let m = f32_to_f16(x - y * (x / y).floor());
    assert_eq!(m, 0x411a);
    for name in ["k4_mod_vec4_vs", "k4_mod_vec3_float_vs"] {
        assert!(json(name).contains(&format!("#0x{m:04X}{m:04X}")), "{name}: mod word");
    }
    /* K4 reflect and refract of the constants are exactly -1.0, the
     * immediate `0x3F800000.neg` of the compare; acos(1.0) is +0.0 and has
     * no word (the compare reads the reference alone) */
    let reflect = 1.0f32 - 2.0 * (1.0 * 1.0) * 1.0;
    let eta = 0.5f32;
    let k = 1.0 - eta * eta * (1.0 - 1.0 * 1.0);
    let refract = eta * 1.0 - (eta * 1.0 + fold_glsl(Libm::Sqrt, k, 0.0)) * 1.0;
    assert_eq!((reflect.to_bits(), refract.to_bits()), (0xbf80_0000, 0xbf80_0000));
    for name in ["k4_reflect_vec3_fs", "k4_refract_float_fs", "k4_refract_vec4_fs"] {
        assert!(json(name).contains("0x3F800000.neg"), "{name}: -1.0 immediate");
    }
    assert_eq!(fold_glsl(Libm::Acos, 1.0, 0.0).to_bits(), 0);
    /* K5: build_asin's coefficients as f32 (fp32 at both precisions) and
     * nir_atan's as fp16 pairs in the mediump vertex shader */
    let asin = [-0.0187293f64, 0.0742610, -0.2121144, 1.5707288].map(|c| c as f32);
    for name in ["k5_acos_highp_vec4_vs", "k5_asin_mediump_vs"] {
        let j = json(name);
        for c in asin {
            assert!(j.contains(&format!("\"{:08x}\"", c.to_bits())), "{name}: asin coefficient {c}");
        }
        assert!(j.contains(&format!("\"{:08x}\"", core::f32::consts::FRAC_PI_2.to_bits())), "{name}: pi/2");
    }
    let atan = [
        -0.012_132_321_317_344_4f64,
        0.053_681_378_431_040_6,
        -0.117_350_319_478_685_1,
        0.193_892_497_711_561,
        -0.332_675_641_809_124_6,
    ];
    for name in ["k5_atan_mediump_vec2_vs", "k5_atan2_mediump_vs"] {
        let j = json(name);
        for c in atan {
            let h = f32_to_f16(c as f32);
            assert!(j.contains(&format!("\"{h:04x}{h:04x}\"")), "{name}: atan coefficient {c}");
        }
    }
    let j = json("k5_random_trig17_vs");
    for c in atan.iter().copied().chain([0.999_979_312_831_035_5]) {
        assert!(j.contains(&format!("\"{:08x}\"", (c as f32).to_bits())), "trig17: atan coefficient {c}");
    }
}
