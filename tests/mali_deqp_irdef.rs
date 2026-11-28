// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_irdef_*` (plan #387, item
//! irdefect-design; `docs/notes/mali-irdef.md`): the dEQP-GLES2 shapes that
//! still meet `Defect(IrIndex)` at 6e9e515c (rechecked at 14e7a742), of the
//! 216 cases the Verdin iMX95 (Mali-G310) run at e63ab28e refused with it
//! (`tools/cts/log/2026-11-2*-imx95-gles2-*/refusals.tsv`). On the host at
//! 6e9e515c, 42 of the 216 still meet it, in four root causes (the note's
//! groups):
//!
//! | group | cause | cases | references here |
//! |---|---|--:|---|
//! | R1 | an unread lane paired in the mediump lane tree (`lane_tree`, `split_lanes`, `vectorise`) | 20 | `mat4_mat4`, `mat4_addassign`, `mat4_preinc`, `mat4_postdec`, `mat4_float_lowp` |
//! | R2 | a compact `mat3` load never emitted before the node walk reads it (`select_highp`, `src32`) | 18 | `mat3_vertex`, `mat3_combine`, `mat3_linkage`, `mat3_datatypes` |
//! | R3 | the colour lanes not refreshed after `vectorize16` (`select_highp`) | 2 | `not_bvec3`, `not_bvec4` |
//! | R4 | an int uniform pair under `fneg` has no value (`operand`) | 2 | none new: `deqp_irindex2_op_val` |
//!
//! Every pair is the dEQP case's own, verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` (its `.license` names the case). Each was
//! drawn with `tools/mali/deqp_ref.sh <case>.vert <case>.frag
//! tools/mali/testdata/deqp_irdef_<shape> --instrument shim`
//! (`$CONNECTED_DEPS_DIR/mesa-shim`, the G610 drm-shim; the G310's words are
//! the same, `docs/notes/mali-g310-vs-g610.md`), and its `.nir.sha` with
//! `tools/mali/nir_refs.sh 'deqp_irdef_*'`. `tests/mali_ref` does the compare.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The fix of a group moves its
//! references to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops their `#[ignore]` and updates
//! the note. `references_parse` checks every file and the fragment word
//! count ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_irdef
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a fix made byte-equal, without the `deqp_irdef_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

const IR_INDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("mat3_combine", IR_INDEX),
    ("mat3_datatypes", IR_INDEX),
    ("mat3_linkage", IR_INDEX),
    ("mat3_vertex", IR_INDEX),
    ("mat4_addassign", IR_INDEX),
    ("mat4_float_lowp", IR_INDEX),
    ("mat4_mat4", IR_INDEX),
    ("mat4_postdec", IR_INDEX),
    ("mat4_preinc", IR_INDEX),
    ("not_bvec3", IR_INDEX),
    ("not_bvec4", IR_INDEX),
];

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("mat3_combine", 30),
    ("mat3_datatypes", 29),
    ("mat3_linkage", 30),
    ("mat3_vertex", 30),
    ("mat4_addassign", 35),
    ("mat4_float_lowp", 37),
    ("mat4_mat4", 35),
    ("mat4_postdec", 48),
    ("mat4_preinc", 38),
    ("not_bvec3", 13),
    ("not_bvec4", 12),
];

/// R2: `varying mat3 out0` compared with `isOk` against a `mat3` uniform
/// (`conversions.matrix_to_matrix.mat3_to_mat3_vertex`; the fragment shader of
/// all 12 `_vertex` cases of the group).
#[test]
#[ignore = "Defect(IrIndex) R2: a compact mat3 load never emitted (src32 at select_highp's node walk)"]
fn mat3_vertex() {
    mali_ref::assert_matches("deqp_irdef_mat3_vertex");
}

/// R2: `mat3(in0, in1, in2)` of three `vec3` varyings against a `mat3` uniform
/// (`conversions.matrix_combine.vec3_vec3_vec3_to_mat3_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R2: a compact mat3 load never emitted (src32 at select_highp's node walk)"]
fn mat3_combine() {
    mali_ref::assert_matches("deqp_irdef_mat3_combine");
}

/// R2: `out0 = var` of a `varying mat3`, no default int precision
/// (`linkage.varying_type_mat3`).
#[test]
#[ignore = "Defect(IrIndex) R2: a compact mat3 load never emitted (src32 at select_highp's node walk)"]
fn mat3_linkage() {
    mali_ref::assert_matches("deqp_irdef_mat3_linkage");
}

/// R2: the sum of a `varying mat3`'s nine lanes through a function
/// (`functions.datatypes.float_mat3_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R2: a compact mat3 load never emitted (src32 at select_highp's node walk)"]
fn mat3_datatypes() {
    mali_ref::assert_matches("deqp_irdef_mat3_datatypes");
}

/// R1: `v_mat4 + u_in1`, a mediump `mat4` varying and uniform, the column swizzle sum
/// (`matrix.add.dynamic_mediump_mat4_mat4_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R1: an unread lane paired in the lane tree (index from split_lanes)"]
fn mat4_mat4() {
    mali_ref::assert_matches("deqp_irdef_mat4_mat4");
}

/// R1: `res = v_mat4; res += u_in1;` (`matrix.add_assign.dynamic_mediump_mat4_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R1: an unread lane paired in the lane tree (index from split_lanes)"]
fn mat4_addassign() {
    mali_ref::assert_matches("deqp_irdef_mat4_addassign");
}

/// R1: `res = ++tmpValue`, both summed into the colour (`matrix.pre_increment.dynamic_mediump_mat4_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R1: an unread lane paired in the lane tree (index from split_lanes)"]
fn mat4_preinc() {
    mali_ref::assert_matches("deqp_irdef_mat4_preinc");
}

/// R1: `res = tmpValue--`, both summed into the colour (`matrix.post_decrement.dynamic_mediump_mat4_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R1: an unread lane paired in the lane tree (index from split_lanes)"]
fn mat4_postdec() {
    mali_ref::assert_matches("deqp_irdef_mat4_postdec");
}

/// R1: `v_mat4 + v_coords` of a lowp `mat4` and a lowp float varying
/// (`matrix.add.dynamic_lowp_mat4_float_fragment`; the mediump sibling is
/// `deqp_irindex_mat4_pair_unread`).
#[test]
#[ignore = "Defect(IrIndex) R1: an unread lane paired in the lane tree (index from split_lanes)"]
fn mat4_float_lowp() {
    mali_ref::assert_matches("deqp_irdef_mat4_float_lowp");
}

/// R3: `vec3(not(greaterThan(v_in0.zxy, vec3(0.0))))` into the colour
/// (`operator.bool_compare.not.bvec3_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R3: the colour lanes not refreshed after vectorize16 (src32 from select_highp)"]
fn not_bvec3() {
    mali_ref::assert_matches("deqp_irdef_not_bvec3");
}

/// R3: the `bvec4` form (`operator.bool_compare.not.bvec4_fragment`).
#[test]
#[ignore = "Defect(IrIndex) R3: the colour lanes not refreshed after vectorize16 (src32 from select_highp)"]
fn not_bvec4() {
    mali_ref::assert_matches("deqp_irdef_not_bvec4");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_irdef_")?.strip_suffix(".expect.json").map(str::to_string))
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
/// prefix), `vs.position`, `vs.varying` and `fs.fragment` have 64-bit hex
/// words, and `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_irdef_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_irdef_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_irdef_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_irdef_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
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
/// lists cover every `deqp_irdef_*` reference on disk. A fix flips this
/// test: a shape that compiles must move to [`LOWERED`] or [`DIFFERS`].
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_irdef_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_irdef_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_irdef_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_irdef_{name}"));
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
