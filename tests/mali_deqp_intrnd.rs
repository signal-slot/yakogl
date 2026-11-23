// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_intrnd_*` (plan #377, item
//! ref-intrnd): the dEQP-GLES2 `shaders.random.*` pairs whose first refusal
//! is `an integer or boolean value` (the measure's shape
//! `intval/random_mixed`), one pair per sub-shape of Mesa's fragment program
//! that the `deqp_intval3_rsg_*` references leave. `tests/mali_ref` does the
//! compare. Each pair is the case's, as VK-GL-CTS 93bca01's
//! `rsg::ProgramGenerator` writes it with the case's parameters and seed
//! (`es2fRandomShaderTests.cpp`), drawn with `tools/mali/deqp_ref.sh
//! --instrument shim` (`drm_shim_ref.sh run ... --uniform-values` on Mesa
//! 26.2.1's G610 drm-shim). Every reference was drawn twice with the same
//! bytes.
//!
//! At the tip every reference is refused, so each byte-compare is
//! `#[ignore]`d and `refusals_pinned` holds the refusal ([`REFUSED`]): the
//! integer/boolean text, or for the three `dead_*` references the next one,
//! since the walk builds nothing for the dead int and bool values
//! (`nir_fs.rs` `ir_live`). The lowering step that lifts one moves it to
//! [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other words than
//! Mesa's) in the same commit and drops its `#[ignore]`. `references_parse`
//! checks every file: the pair is there, each region's words are 64-bit hex,
//! the trace is the reference's own, and the fragment program has the word
//! count of [`FS_WORDS`].
//!
//! `int_uniform_lanes_fs` and `bool_uniform_select_comb` have no varying
//! program: Mesa's link removes every varying the fragment shader leaves
//! dead. `mali_ref` runs the GL core's link, which may not reach the same
//! program (`tests/mali_ref`'s module comment).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intrnd
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_intrnd_` prefix: each has a byte-compare test below.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const INT_VALUE: &str = "Valhall: an integer or boolean value is not lowered yet";
/// The later refusals of the `dead_*` references, their dead int and bool
/// values left out of the walk.
const LANECYCLE: &str = "Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)";
const FS_OPERAND: &str = "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet";
const FRAGCOLOR: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("bool_uniform_select_comb", INT_VALUE),
    ("bool_varying_lane_fs", INT_VALUE),
    /* the pair of a computed lane then a constant asked for again
     * (mali-deqp-defects.md fix C) */
    ("dead_bool_cmp_fs", LANECYCLE),
    ("dead_cond_fs", FS_OPERAND),
    ("dead_ivec_uniform_fs", FRAGCOLOR),
    ("int_uniform_lanes_fs", INT_VALUE),
    ("trunc_atan_fs", INT_VALUE),
    ("trunc_lane_fs", INT_VALUE),
];

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("bool_uniform_select_comb", 12),
    ("bool_varying_lane_fs", 10),
    ("dead_bool_cmp_fs", 16),
    ("dead_cond_fs", 13),
    ("dead_ivec_uniform_fs", 11),
    ("int_uniform_lanes_fs", 10),
    ("trunc_atan_fs", 39),
    ("trunc_lane_fs", 11),
];

/// `shaders.random.conditionals.combined.6`: `if (bvec3(e, i, i).t)` of a
/// `uniform float` is `FCMP_OR.v2f16.eq.m1` of its FAU half against 0, and
/// the colour of the `if` / `else if` is two `MUX.v2i16` of FAU constants
/// (the inner `0 <= h - int(-5.0)` and the `else if` fold). The vertex
/// shader is random too; the fragment shader has no varying left.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn bool_uniform_select_comb() {
    mali_ref::assert_matches("deqp_intrnd_bool_uniform_select_comb");
}

/// `shaders.random.conditionals.fragment.37`: `vec4(bool(c), ...)` of a
/// varying is `FCMP_OR.v2f16.ne.f1` (a 1.0/0.0 lane), then `U16_TO_U32` and
/// one `FMA.v2f16` with the constant pair; the int globals and the `if` on
/// `bool(int(e))` fold.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn bool_varying_lane_fs() {
    mali_ref::assert_matches("deqp_intrnd_bool_varying_lane_fs");
}

/// `shaders.random.comparison_ops.fragment.13`: the `vec3 f` of
/// `i > float(g)` (a `uniform int`) and `3 >= 4`, the `vec4 h` of int and
/// bool lanes and the statement `float(2) < float(4)` are dead; no integer
/// op is left, only `FADD_IMM`/`FMA.v2f16` of the varyings and FAU constants.
#[test]
#[ignore = "refused: a back end defect (LaneCycle); pinned by refusals_pinned"]
fn dead_bool_cmp_fs() {
    mali_ref::assert_matches("deqp_intrnd_dead_bool_cmp_fs");
}

/// `shaders.random.conditionals.fragment.66`: an `int` local, a `bvec2` of
/// `int(e) != i` and nested `if`s on folded bools; only the `uniform vec4`
/// colour times a constant pair and one varying lane are left.
#[test]
#[ignore = "refused: a fragment shader operand of a form or in a place no reference draws; pinned by refusals_pinned"]
fn dead_cond_fs() {
    mali_ref::assert_matches("deqp_intrnd_dead_cond_fs");
}

/// `shaders.random.comparison_ops.fragment.46`: `ivec3` arithmetic of
/// `uniform ivec3`/`int` values and an `ivec3 !=` statement, all dead; the
/// colour is `vec4(0, c * b, a, 0)` (`FMA.v2f16`, `U16_TO_U32` of the lane
/// beside 0.0).
#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn dead_ivec_uniform_fs() {
    mali_ref::assert_matches("deqp_intrnd_dead_ivec_uniform_fs");
}

/// `shaders.random.scalar_conversion.fragment.58`: `vec4(bool(int(h)), ...,
/// h)` of a `uniform int` is `ICMP_OR.u32.ne.m1` of its FAU word, `MUX.v2i16`
/// to 1.0/0.0, and `S32_TO_F32` narrowed by `FADD.f32 .h0`; the `ivec4`
/// locals are dead.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn int_uniform_lanes_fs() {
    mali_ref::assert_matches("deqp_intrnd_int_uniform_lanes_fs");
}

/// `shaders.random.trigonometric.fragment.31`: `int(a)` of a varying read
/// back as a float is `FROUND.f32.rtz` (`i2f(f2i(x))` is `ftrunc`), then
/// `atan` of the lanes at fp32 (`FRCP`, the polynomial `FMA.f32` chain,
/// `CSEL.u32.eq`, the sign put back with `LSHIFT_AND/OR.i32`).
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn trunc_atan_fs() {
    mali_ref::assert_matches("deqp_intrnd_trunc_atan_fs");
}

/// `shaders.random.conditionals.fragment.31`: the last write
/// `vec4(a, b, false, int(a))` leaves `int(a)` as `FROUND.f32.rtz` of the
/// fp32 varying, narrowed by `FADD.f32 .h0`; every earlier write, the int
/// globals and the `if`s are dead or fold.
#[test]
#[ignore = "refused: an integer or boolean value; pinned by refusals_pinned"]
fn trunc_lane_fs() {
    mali_ref::assert_matches("deqp_intrnd_trunc_lane_fs");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_intrnd_")?.strip_suffix(".expect.json").map(str::to_string))
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
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_intrnd_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_intrnd_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_intrnd_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_intrnd_{name}.trace"));
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
/// lists cover every `deqp_intrnd_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_intrnd_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_intrnd_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intrnd_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_intrnd_{name}"));
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
