// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_intval5_*` (plan #387, item
//! intval5-ref, section 12 of `docs/notes/mali-intval-rest-lowering.md` and
//! section 3.1 of `docs/notes/mali-deqp-cfold2.md`): the patterns that steps 2
//! to 4 of the `intval-rest` lowering marked "(no reference)" (the fragment
//! uniform compare chains, the three-mask `iand`, a 16-bit mask with a 32-bit
//! one) and the `light_amount` shaders. `tests/mali_ref` does the compare.
//! Every reference was drawn with `tools/mali/deqp_ref.sh --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`); the pairs are written
//! after the dEQP-GLES2 generator templates, the `light_amount` pairs are
//! `tools/shaders/es100/light_amount_*.vert` verbatim.
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
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intval5
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal (or that were byte-equal when
/// drawn), without the `deqp_intval5_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("c_bool3", TEXT_INT),
    ("c_bool_not", TEXT_INT),
    ("c_bvec2", TEXT_LANE),
    ("c_bvec3", TEXT_LANE),
    ("c_bvec4", TEXT_LANE),
    ("c_int3", TEXT_INT),
    ("c_int_k", TEXT_LANE),
    ("c_int_lt", TEXT_LANE),
    ("c_int_two", TEXT_INT),
    ("c_ivec4", TEXT_LANE),
    ("light_amount_0_16", TEXT_CONST),
    ("light_amount_2_2", TEXT_CONST),
    ("mask1632", TEXT_INT),
];

const TEXT_INT: &str = "Valhall: an integer or boolean value is not lowered yet";
const TEXT_LANE: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const TEXT_CONST: &str =
    "Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("c_bool3", 10),
    ("c_bool_not", 11),
    ("c_bvec2", 9),
    ("c_bvec3", 10),
    ("c_bvec4", 12),
    ("c_int3", 14),
    ("c_int_k", 15),
    ("c_int_lt", 11),
    ("c_int_two", 10),
    ("c_ivec4", 15),
    ("light_amount_0_16", 8),
    ("light_amount_2_2", 8),
    ("mask1632", 13),
];

/// `u_a && u_b && u_c` of three `uniform bool`s (the scalar three-mask `iand`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_bool3() {
    mali_ref::assert_matches("deqp_intval5_c_bool3");
}

/// `!u_a && u_b` and `!u_b` of `uniform bool`s.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_bool_not() {
    mali_ref::assert_matches("deqp_intval5_c_bool_not");
}

/// `u_a == bvec2(..)` of a `uniform bvec2`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_bvec2() {
    mali_ref::assert_matches("deqp_intval5_c_bvec2");
}

/// `u_a == bvec3(..)` of a `uniform bvec3`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_bvec3() {
    mali_ref::assert_matches("deqp_intval5_c_bvec3");
}

/// `u_a == bvec4(..)` of a `uniform bvec4`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_bvec4() {
    mali_ref::assert_matches("deqp_intval5_c_bvec4");
}

/// `(u_a == -9) && (u_b == 2) && (u_c == -2)` of three `uniform int`s (the three-mask `iand`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_int3() {
    mali_ref::assert_matches("deqp_intval5_c_int3");
}

/// `u == 4`, `u == 9`, `u == 17` of `uniform int`s (the byte-immediate table, k = 4 and up).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_int_k() {
    mali_ref::assert_matches("deqp_intval5_c_int_k");
}

/// `u_a < u_b` and `u_a >= 3` of `uniform int`s.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_int_lt() {
    mali_ref::assert_matches("deqp_intval5_c_int_lt");
}

/// `(u_a == 2) && (u_b == -3)` of two `uniform int`s.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_int_two() {
    mali_ref::assert_matches("deqp_intval5_c_int_two");
}

/// `u_a == ivec4(-1, 8, -4, 10)` of a `uniform ivec4`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn c_ivec4() {
    mali_ref::assert_matches("deqp_intval5_c_ivec4");
}

/// `light_amount` with 0 directional and 16 omni lights.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn light_amount_0_16() {
    mali_ref::assert_matches("deqp_intval5_light_amount_0_16");
}

/// `light_amount` with 2 directional and 2 omni lights.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn light_amount_2_2() {
    mali_ref::assert_matches("deqp_intval5_light_amount_2_2");
}

/// `(v.x > 0.0) && (int(v.y) < 1)`: a 16-bit compare mask with a 32-bit one.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn mask1632() {
    mali_ref::assert_matches("deqp_intval5_mask1632");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_intval5_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_intval5_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_intval5_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_intval5_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_intval5_{name}.trace"));
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
/// lists cover every `deqp_intval5_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_intval5_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_intval5_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intval5_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intval5_{name}"));
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
