// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_fsintres_*` (plan #387, item
//! fsintres-design, `docs/notes/mali-fsintres-lowering.md`): the fragment
//! shapes that still refuse "an integer or boolean value" after the
//! `lower-fsint` train and that no `deqp_intval*` / `deqp_fsint*` reference
//! covers (dynamic indexing of local arrays and matrices, int and bool array
//! arguments, a mixed int/bool `bvec2`, the two `fragment_ops.interaction`
//! and one `random.scalar_conversion` shader). `tests/mali_ref` does the
//! compare. Every pair is the one dEQP-GLES2 (VK-GL-CTS 93bca01) writes,
//! verbatim from the `.qpa` of a HOST-ONLY `tools/mali/deqp-host-measure.sh`
//! run at 6e9e515c; every reference was drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610 drm-shim,
//! `--uniform-values`). `tools/mali/g310_check.sh` gives the same words for
//! the G310.
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
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsintres
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal (or that were byte-equal when
/// drawn), without the `deqp_fsintres_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("fn_arr_bool", TEXT_INT),
    ("fn_arr_int", TEXT_INT),
    ("idx_float_dyn_dyn", TEXT_INT),
    ("idx_mat4_dyn_static", TEXT_INT),
    ("idx_vec2_static_loop", TEXT_INT),
    ("idx_vec4_dyn_dyn", TEXT_INT),
    ("idx_vec4_dyn_loop", TEXT_INT),
    ("inter_85", TEXT_INT),
    ("inter_96", TEXT_INT),
    ("rnd_sconv68", TEXT_INT),
    ("vcomb_int_bool_bvec2", TEXT_INT),
];

const TEXT_INT: &str = "Valhall: an integer or boolean value is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("fn_arr_bool", 24),
    ("fn_arr_int", 28),
    ("idx_float_dyn_dyn", 65),
    ("idx_mat4_dyn_static", 85),
    ("idx_vec2_static_loop", 59),
    ("idx_vec4_dyn_dyn", 98),
    ("idx_vec4_dyn_loop", 92),
    ("inter_85", 9),
    ("inter_96", 7),
    ("rnd_sconv68", 13),
    ("vcomb_int_bool_bvec2", 18),
];

/// `functions.array_arguments.global_in_bool_fragment`: a global `bool[4]`
/// of `!bvec4(v)` lanes passed by value, compared with a `uniform bvec4`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fn_arr_bool() {
    mali_ref::assert_matches("deqp_fsintres_fn_arr_bool");
}

/// `functions.array_arguments.global_in_int_fragment`: a global `int[4]` of
/// `ivec4(v * 1.0025)` lanes passed by value, `-1 * ivec4(arr)` compared with
/// a `uniform ivec4`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn fn_arr_int() {
    mali_ref::assert_matches("deqp_fsintres_fn_arr_int");
}

/// `indexing.tmp_array.float_dynamic_write_dynamic_read_fragment`: a local
/// `float[4]` written and read at `uniform int` indices.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_float_dyn_dyn() {
    mali_ref::assert_matches("deqp_fsintres_idx_float_dyn_dyn");
}

/// `indexing.matrix_subscript.mat4_dynamic_write_static_read_fragment`: the
/// columns of a local `mat4` written at `uniform int` indices, read at 0..3.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_mat4_dyn_static() {
    mali_ref::assert_matches("deqp_fsintres_idx_mat4_dyn_static");
}

/// `indexing.tmp_array.vec2_dynamic_write_static_loop_read_fragment`: a
/// local `vec2[4]` written at `uniform int` indices, read by a `for` loop of
/// four iterations.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_vec2_static_loop() {
    mali_ref::assert_matches("deqp_fsintres_idx_vec2_static_loop");
}

/// `indexing.tmp_array.vec4_dynamic_write_dynamic_read_fragment`: a local
/// `vec4[4]` written and read at `uniform int` indices.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_vec4_dyn_dyn() {
    mali_ref::assert_matches("deqp_fsintres_idx_vec4_dyn_dyn");
}

/// `indexing.tmp_array.vec4_dynamic_write_dynamic_loop_read_fragment`: a
/// local `vec4[4]` written at `uniform int` indices, read by a loop to
/// `uniform int ui_four` (Mesa unrolls it to the array's four elements).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn idx_vec4_dyn_loop() {
    mali_ref::assert_matches("deqp_fsintres_idx_vec4_dyn_loop");
}

/// `fragment_ops.interaction.basic_shader.85`: `ivec3(true, d, c) != b` of
/// varyings and an int global (Mesa folds it: lane x is 1 against 0), so the
/// colour is `vec4(0, 0, 1, int(e))` of a mediump uniform.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn inter_85() {
    mali_ref::assert_matches("deqp_fsintres_inter_85");
}

/// `fragment_ops.interaction.basic_shader.96`: int and bool values that are
/// all dead (`k != k`, `d - int(h)`); the colour is `b - a`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn inter_96() {
    mali_ref::assert_matches("deqp_fsintres_inter_96");
}

/// `random.scalar_conversion.fragment.68`: `float(b)` of an int uniform
/// sum and `bool(a)` of an int uniform in colour lanes.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_sconv68() {
    mali_ref::assert_matches("deqp_fsintres_rnd_sconv68");
}

/// `conversions.vector_combine.int_bool_to_bvec2_fragment`: `bvec2(int,
/// bool)` of two scalar varyings compared with a `uniform bvec2`.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn vcomb_int_bool_bvec2() {
    mali_ref::assert_matches("deqp_fsintres_vcomb_int_bool_bvec2");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_fsintres_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_fsintres_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_fsintres_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_fsintres_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_fsintres_{name}.trace"));
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
/// lists cover every `deqp_fsintres_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_fsintres_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_fsintres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fsintres_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fsintres_{name}"));
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
