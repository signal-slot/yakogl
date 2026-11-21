// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_intval4_*` (plan #387, item
//! intval4-ref, section 7 of `docs/notes/mali-intval-rest-lowering.md`): the
//! fragment integer/boolean shapes that step 1 to 4 of the `intval-rest`
//! lowering needs and that no `deqp_intval_*` / `deqp_intval3_*` reference
//! holds. `tests/mali_ref` does the compare. Every reference was drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610 drm-shim,
//! `--uniform-values`); the pairs follow the dEQP-GLES2 generator's templates
//! (VK-GL-CTS 93bca01), the shapes of the CTS `.qpa` the earlier references
//! quote, written out by hand where the note says so.
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
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intval4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal (or that were byte-equal when
/// drawn), without the `deqp_intval4_` prefix.
const LOWERED: &[&str] = &["bvec3_cmp_fs"];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &["not_eq_int_fs"];

const TEXT0: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";
const TEXT1: &str = "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const TEXT2: &str = "Valhall: an integer or boolean value is not lowered yet";
const TEXT3: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("and_icmp_fs", TEXT2),
    ("bvec2_of_ivec2_fs", TEXT2),
    ("bvec4_cmp_fs", TEXT0),
    ("logical_or_fs", TEXT2),
    ("ne_int_fs", TEXT2),
    ("ne_ivec2_fs", TEXT2),
    ("sel_int_fs", TEXT3),
    ("sel_ivec2_fs", TEXT3),
    ("uapi_bool_fs", TEXT1),
    ("uapi_bvec2_fs", TEXT1),
    ("uapi_bvec3_fs", TEXT1),
    ("uapi_bvec4_fs", TEXT1),
    ("uapi_int_fs", TEXT1),
    ("uapi_ivec2_fs", TEXT1),
    ("uapi_ivec3_fs", TEXT1),
    ("uapi_ivec4_fs", TEXT1),
    ("uni_bool_and_or_fs", TEXT2),
];

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("and_icmp_fs", 11),
    ("bvec2_of_ivec2_fs", 18),
    ("bvec3_cmp_fs", 28),
    ("bvec4_cmp_fs", 30),
    ("logical_or_fs", 12),
    ("ne_int_fs", 11),
    ("ne_ivec2_fs", 15),
    ("not_eq_int_fs", 10),
    ("sel_int_fs", 17),
    ("sel_ivec2_fs", 23),
    ("uapi_bool_fs", 10),
    ("uapi_bvec2_fs", 11),
    ("uapi_bvec3_fs", 12),
    ("uapi_bvec4_fs", 14),
    ("uapi_int_fs", 11),
    ("uapi_ivec2_fs", 12),
    ("uapi_ivec3_fs", 15),
    ("uapi_ivec4_fs", 17),
    ("uni_bool_and_or_fs", 15),
];

/// `(i > 1) && (i < 3)` of a run-time mediump int (a `random` shape, the smallest pair with the form).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn and_icmp_fs() {
    mali_ref::assert_matches("deqp_intval4_and_icmp_fs");
}

/// `bvec2(ivec2)`, the `conversions.vector_to_vector` template of the `intval3` reslib references at width 2.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn bvec2_of_ivec2_fs() {
    mali_ref::assert_matches("deqp_intval4_bvec2_of_ivec2_fs");
}

/// `all(equal(a, b))` and `any(notEqual(a, b))` of two bvec3 (one pair for the `bool_compare.{equal,notEqual,all}.bvec3` forms).
#[test]
fn bvec3_cmp_fs() {
    mali_ref::assert_matches("deqp_intval4_bvec3_cmp_fs");
}

/// `all(equal(a, b))` and `any(notEqual(a, b))` of two bvec4 (one pair for the `bool_compare.{equal,notEqual,all}.bvec4` forms).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn bvec4_cmp_fs() {
    mali_ref::assert_matches("deqp_intval4_bvec4_cmp_fs");
}

/// `binary_operator.logical_or.bool_fragment`: `||` of two `> 0.0` compares.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn logical_or_fs() {
    mali_ref::assert_matches("deqp_intval4_logical_or_fs");
}

/// `binary_operator.not_equal.mediump_int_fragment`: `!=` of two mediump ints.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ne_int_fs() {
    mali_ref::assert_matches("deqp_intval4_ne_int_fs");
}

/// `binary_operator.not_equal.mediump_ivec2_fragment`: whole-vector `!=` of two ivec2.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn ne_ivec2_fs() {
    mali_ref::assert_matches("deqp_intval4_ne_ivec2_fs");
}

/// `!(i == 2)` of a run-time mediump int (a `random` shape, the smallest pair with the form).
#[test]
#[ignore = "compiles with other words than Mesa's; pinned by refusals_pinned"]
fn not_eq_int_fs() {
    mali_ref::assert_matches("deqp_intval4_not_eq_int_fs");
}

/// `selection.mediump_int_fragment`: `bool ? int : int`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sel_int_fs() {
    mali_ref::assert_matches("deqp_intval4_sel_int_fs");
}

/// `selection.mediump_ivec2_fragment`: `bool ? ivec2 : ivec2`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn sel_ivec2_fs() {
    mali_ref::assert_matches("deqp_intval4_sel_ivec2_fs");
}

/// `uniform_api` `compare_bool` harness of a scalar `u_var` of type bool (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_bool_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_bool_fs");
}

/// `uniform_api` `compare_bvec2` harness of a scalar `u_var` of type bvec2 (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_bvec2_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_bvec2_fs");
}

/// `uniform_api` `compare_bvec3` harness of a scalar `u_var` of type bvec3 (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_bvec3_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_bvec3_fs");
}

/// `uniform_api` `compare_bvec4` harness of a scalar `u_var` of type bvec4 (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_bvec4_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_bvec4_fs");
}

/// `uniform_api` `compare_int` harness of a scalar `u_var` of type int (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_int_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_int_fs");
}

/// `uniform_api` `compare_ivec2` harness of a scalar `u_var` of type ivec2 (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_ivec2_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_ivec2_fs");
}

/// `uniform_api` `compare_ivec3` harness of a scalar `u_var` of type ivec3 (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_ivec3_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_ivec3_fs");
}

/// `uniform_api` `compare_ivec4` harness of a scalar `u_var` of type ivec4 (fragment stage).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uapi_ivec4_fs() {
    mali_ref::assert_matches("deqp_intval4_uapi_ivec4_fs");
}

/// `uniform bool u_a, u_b`: `u_a && u_b`, `u_a || u_b`, `u_a != u_b` (a hand-written shape, no dEQP case).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn uni_bool_and_or_fs() {
    mali_ref::assert_matches("deqp_intval4_uni_bool_and_or_fs");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_intval4_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_intval4_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_intval4_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_intval4_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_intval4_{name}.trace"));
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
/// lists cover every `deqp_intval4_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_intval4_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_intval4_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intval4_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intval4_{name}"));
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
