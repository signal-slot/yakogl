// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_constres_*` (plan #387, item
//! constres-refs; the design is `docs/notes/mali-constres-lowering.md`):
//! the dEQP-GLES2 shapes of the `ConstantFold`, `NegZeroShared`,
//! `AlgebraicForm` and `Rebalance` first refusals at `349f989f` that no
//! earlier reference pins (CF-D's random and mat4 shapes, the constant dot
//! CR-1, negzero N3 and its residue CR-2/CR-3, the shared-factor sum CR-4,
//! the zero constant sum CR-5, the select distribution CR-6 and arith A8's
//! unpinned chains). `tests/mali_ref` does the compare. Every reference was
//! drawn with `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610
//! drm-shim, `--uniform-values`); each pair is verbatim from the host
//! measure's `.qpa`, and its `.license` names the case.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. A row's alternatives are the outcomes measured with the unlanded
//! results of other items merged on the tip (`2c0c6596` lower-vsconst, step
//! D; `81b3ce19` lower-mixprec3; `8180095c`): either the text the refusal
//! moves to, or [`EQUAL`], a byte-equal compile. So the pin stays green
//! whichever lands first; it fails when a reference moves anywhere else.
//! `references_parse` checks every file and the fragment word count the note
//! quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_constres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_constres_` prefix. None at `349f989f`.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// An alternative outcome of [`REFUSED`]: the pair compiles byte-equal.
const EQUAL: &str = "(compiles byte-equal)";

/// Every ignored reference: its whole `MaliError` message at the tip, and the
/// other outcomes the unlanded results of other items give it.
const REFUSED: &[(&str, &str, &[&str])] = &[
    ("bcd_cond_vertex_8", TEXT_ALG, &[]),
    ("cdot_cse0_highp", TEXT_CF, &[]),
    ("cdot_cse3_mediump", TEXT_CF, &[]),
    ("cfd_allfeat_fragment_40", TEXT_CF, &[]),
    /* lower-vsconst 2c0c6596 (vsconst step D) folds the highp tree at fp32 */
    ("cfd_div_highp_mat4", TEXT_CF, &[EQUAL]),
    ("cfd_trig_fragment_3", TEXT_CF, &[TEXT_ARITH]),
    ("n3_allfeat_vertex_87", TEXT_NZ, &[]),
    ("n3_cond_combined_10", TEXT_NZ, &[]),
    ("n3_cond_fragment_17", TEXT_NZ, &[]),
    ("n3_cond_vertex_17", TEXT_NZ, &[]),
    ("nsz_basic_expression_fragment_4", TEXT_ALG, &[TEXT_LANECYCLE]),
    ("nzb_allfeat_fragment_5", TEXT_NZ, &[]),
    ("nzb_allfeat_vertex_40", TEXT_NZ, &[]),
    ("nzd_allfeat_fragment_71", TEXT_NZ, &[]),
    ("rb_allfeat_vertex_14", TEXT_RB, &[]),
    ("rb_allfeat_vertex_54", TEXT_RB, &[]),
    ("rb_invariance_lowp_cse1", TEXT_RB, &[]),
    ("sf_div_dynamic_mediump_mat3", TEXT_ALG, &[]),
    ("sf_mul_dynamic_mediump_mat2", TEXT_ALG, &[]),
    ("sf_mul_uniform_highp_mat3", TEXT_ALG, &[]),
    ("sf_mul_uniform_lowp_mat4", TEXT_ALG, &[]),
];

const TEXT_CF: &str =
    "Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet";
const TEXT_NZ: &str = "Valhall: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), is not lowered yet";
const TEXT_ALG: &str = "Valhall: a shape Mesa's algebraic rules rewrite without a reference (max(x, 0.0), clamp(x, -1.0, 1.0), a negated float(bool) in a product) is not lowered yet";
const TEXT_RB: &str = "Valhall: three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered yet";
const TEXT_ARITH: &str =
    "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";
const TEXT_LANECYCLE: &str = "Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("bcd_cond_vertex_8", 7),
    ("cdot_cse0_highp", 7),
    ("cdot_cse3_mediump", 7),
    ("cfd_allfeat_fragment_40", 39),
    ("cfd_div_highp_mat4", 7),
    ("cfd_trig_fragment_3", 7),
    ("n3_allfeat_vertex_87", 7),
    ("n3_cond_combined_10", 7),
    ("n3_cond_fragment_17", 16),
    ("n3_cond_vertex_17", 7),
    ("nsz_basic_expression_fragment_4", 17),
    ("nzb_allfeat_fragment_5", 164),
    ("nzb_allfeat_vertex_40", 7),
    ("nzd_allfeat_fragment_71", 81),
    ("rb_allfeat_vertex_14", 7),
    ("rb_allfeat_vertex_54", 7),
    ("rb_invariance_lowp_cse1", 15),
    ("sf_div_dynamic_mediump_mat3", 21),
    ("sf_mul_dynamic_mediump_mat2", 17),
    ("sf_mul_uniform_highp_mat3", 19),
    ("sf_mul_uniform_lowp_mat4", 20),
];

/// CR-6: `int c = f + e;` where a run-time `if` stores one of two constants
/// to `e` (`conditionals.vertex.8`): Mesa distributes the `iadd` into the
/// select of constants (`CSEL.u32.eq` of two FAU floats, no `S32_TO_F32`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn bcd_cond_vertex_8() {
    mali_ref::assert_matches("deqp_constres_bcd_cond_vertex_8");
}

/// CR-1: `dot(v_unrelated, vec4(1.0))` of a varying the link made constant
/// (`invariance.highp.common_subexpression_0`): Mesa folds it, FS 7 words.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn cdot_cse0_highp() {
    mali_ref::assert_matches("deqp_constres_cdot_cse0_highp");
}

/// CR-1, the mediump `common_subexpression_3`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn cdot_cse3_mediump() {
    mali_ref::assert_matches("deqp_constres_cdot_cse3_mediump");
}

/// CF-D: a mediump `rcp` of a constant not exact in fp16 among live code
/// (`random.all_features.fragment.40`, FS 39 words).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn cfd_allfeat_fragment_40() {
    mali_ref::assert_matches("deqp_constres_cfd_allfeat_fragment_40");
}

/// CF-D: `matrix.div.const_highp_mat4_mat4_fragment`, a constant colour.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn cfd_div_highp_mat4() {
    mali_ref::assert_matches("deqp_constres_cfd_div_highp_mat4");
}

/// CF-D: `random.trigonometric.fragment.3`, a constant colour.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn cfd_trig_fragment_3() {
    mali_ref::assert_matches("deqp_constres_cfd_trig_fragment_3");
}

/// N3: int/bool constant globals in a vertex compare (`all_features.vertex.87`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn n3_allfeat_vertex_87() {
    mali_ref::assert_matches("deqp_constres_n3_allfeat_vertex_87");
}

/// N3: `-0.0 * float(u)` and int constant globals (`conditionals.combined.10`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn n3_cond_combined_10() {
    mali_ref::assert_matches("deqp_constres_n3_cond_combined_10");
}

/// N3: `vec3(b, 0, -1.25) * vec3(f, m, l)` with `m` an int global
/// (`conditionals.fragment.17`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn n3_cond_fragment_17() {
    mali_ref::assert_matches("deqp_constres_n3_cond_fragment_17");
}

/// N3: the vertex twin (`conditionals.vertex.17`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn n3_cond_vertex_17() {
    mali_ref::assert_matches("deqp_constres_n3_cond_vertex_17");
}

/// CR-5: `#a + -(b + #c)` whose constants sum to 0.0
/// (`random.basic_expression.fragment.4`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn nsz_basic_expression_fragment_4() {
    mali_ref::assert_matches("deqp_constres_nsz_basic_expression_fragment_4");
}

/// CR-2: `atan`/`asin`/`tan` of constants beside a compare
/// (`all_features.fragment.5`, FS 164 words).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn nzb_allfeat_fragment_5() {
    mali_ref::assert_matches("deqp_constres_nzb_allfeat_fragment_5");
}

/// CR-2: the vertex form (`all_features.vertex.40`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn nzb_allfeat_vertex_40() {
    mali_ref::assert_matches("deqp_constres_nzb_allfeat_vertex_40");
}

/// CR-3: a constant tree deeper than the -0.0 fold's 12 levels
/// (`all_features.fragment.71`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn nzd_allfeat_fragment_71() {
    mali_ref::assert_matches("deqp_constres_nzd_allfeat_fragment_71");
}

/// A8: a chain of one use (`all_features.vertex.14`, `glsl.rs` visit).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rb_allfeat_vertex_14() {
    mali_ref::assert_matches("deqp_constres_rb_allfeat_vertex_14");
}

/// A8: a chain of several uses (`all_features.vertex.54`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rb_allfeat_vertex_54() {
    mali_ref::assert_matches("deqp_constres_rb_allfeat_vertex_54");
}

/// A8: the lowp `invariance.lowp.common_subexpression_1`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rb_invariance_lowp_cse1() {
    mali_ref::assert_matches("deqp_constres_rb_invariance_lowp_cse1");
}

/// CR-4: `mat3 / float` of varyings (`matrix.div.dynamic_mediump_mat3_float_fragment`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sf_div_dynamic_mediump_mat3() {
    mali_ref::assert_matches("deqp_constres_sf_div_dynamic_mediump_mat3");
}

/// CR-4: `v_mat2 * v_coords` with `res[0][1] + res[1][1]` summed: Mesa adds
/// the columns first and multiplies once.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sf_mul_dynamic_mediump_mat2() {
    mali_ref::assert_matches("deqp_constres_sf_mul_dynamic_mediump_mat2");
}

/// CR-4: the highp `matrix.mul.uniform_highp_mat3_float_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sf_mul_uniform_highp_mat3() {
    mali_ref::assert_matches("deqp_constres_sf_mul_uniform_highp_mat3");
}

/// CR-4: `matrix.mul.uniform_lowp_mat4_float_fragment`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sf_mul_uniform_lowp_mat4() {
    mali_ref::assert_matches("deqp_constres_sf_mul_uniform_lowp_mat4");
}

fn on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_constres_")?.strip_suffix(".expect.json").map(str::to_string))
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
/// each has its `.license`, the trace is the reference's own (`deqp_ref.sh`
/// names it after the prefix), `vs.position` and `fs.fragment` have 64-bit
/// hex words, and `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_constres_* reference");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_constres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        for ext in ["vert", "frag"] {
            if !dir.join(format!("deqp_constres_{name}.{ext}.license")).is_file() {
                bad.push(format!("{name}: no .{ext}.license"));
            }
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_constres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_constres_{name}.trace"));
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

/// Each refused reference is refused with exactly its text in [`REFUSED`] or
/// one of its alternatives (the whole `MaliError` message, not a substring;
/// [`EQUAL`] accepts a byte-equal compile), each of [`DIFFERS`] compiles and
/// differs, each of [`LOWERED`] is byte-equal, and the three lists cover every
/// `deqp_constres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> =
        LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_constres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused, alt) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_constres_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let equal = mali_ref::differences(&r, &pair).is_empty();
                if !(equal && alt.contains(&EQUAL)) {
                    let to = if equal { "LOWERED" } else { "DIFFERS" };
                    changed.push(format!("{name}: now compiles; move it to {to}"));
                }
            }
            Err(e) => {
                if e != *refused && !alt.contains(&e.as_str()) {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_constres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_constres_{name}"));
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
