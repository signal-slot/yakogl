// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_mixprec2_*` (plan #387, item
//! mixprec2-design, `docs/notes/mali-mixprec2-lowering.md`): the shapes of the
//! `MixedPrecision` refusal ("a fragment shader that mixes highp and mediump
//! or lowp float values") that the Verdin iMX95 (Mali-G310) run of 2026-11-25
//! reported and that still refuse first with that text at 8133663c. They
//! are not pinned by `tests/mali_deqp_mixprec.rs`, `mali_deqp_mixprec3.rs`,
//! `mali_deqp_mixprec4.rs` or `mali_deqp_mixcol.rs`. Every pair is taken verbatim from the
//! `.qpa` of a host deqp-gles2 run (`tools/mali/deqp-host-measure.sh`), and
//! every reference was drawn with `tools/mali/deqp_ref.sh --instrument shim`
//! (Mesa 26.2.1's G610 drm-shim, `--uniform-values`). `tests/mali_ref` does
//! the compare.
//!
//! A reference the compiler refuses has its test `#[ignore]`d, and
//! `refusals_pinned` holds its whole refusal text ([`REFUSED`]). When a
//! lowering step lifts one, the same commit moves it to [`LOWERED`]
//! (byte-equal) or [`DIFFERS`] (compiles with other words than Mesa's),
//! drops its `#[ignore]` and updates the note. `references_parse` checks
//! that every file loads and that the fragment word count matches the one
//! the note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal (or that were byte-equal
/// when drawn), without the `deqp_mixprec2_` prefix.
/// `rnd_sconv36` since plan #387 mali-train-3c (mixprec2 M2-1): a store a
/// later one overwrites is dropped before the walk (`mod.rs`
/// `fragment_live`), and a constant lane times a constant zero lane beside
/// a computed lane is the algebraic rule's 0.0.
/// `seq_side` since the same item (mixprec2 M2-3): an fp32 sum reads a
/// highp varying's lane as loaded, and a highp local may hold an integer's
/// fp32 conversion.
const LOWERED: &[&str] = &["rnd_sconv36", "seq_side"];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("hp_fract", MIX),
    ("hp_length_vec2", MIX),
    ("hp_mat2_add", MIX),
    ("hp_out_param", MIX),
    ("hp_sqrt_vec3", MIX),
    /* past the mixed-precision text since M2-1 (their overwritten stores
     * are dropped): the last store's own forms are refused */
    ("rnd_swz4", OPERAND),
    ("rnd_tex140", LANE),
    ("struct_nested", MIX),
];

const MIX: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";

const OPERAND: &str = "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet";

const LANE: &str = "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] = &[
    ("hp_fract", 12),
    ("hp_length_vec2", 14),
    ("hp_mat2_add", 7),
    ("hp_out_param", 13),
    ("hp_sqrt_vec3", 18),
    ("rnd_sconv36", 9),
    ("rnd_swz4", 12),
    ("rnd_tex140", 10),
    ("seq_side", 15),
    ("struct_nested", 22),
];

/// `fract(in0)` of a highp float local in a mediump shader
/// (`operator.common_functions.fract.highp_float_fragment`): Mesa's
/// `FROUND.f32.rtn`, `FADD.f32` at fp32, narrowed once into the colour.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_fract() {
    mali_ref::assert_matches("deqp_mixprec2_hp_fract");
}

/// `length(in0)` of a highp vec2 (`operator.geometric.length.highp_vec2_fragment`):
/// `FMA.f32` sum of squares, `FRSQ.f32`, `FRCP.f32`, one narrowing.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_length_vec2() {
    mali_ref::assert_matches("deqp_mixprec2_hp_length_vec2");
}

/// `in0 + in1` of two `const highp mat2` (`matrix.add.const_highp_mat2_mat2_fragment`):
/// Mesa folds the whole colour to FAU words (no varying, no arithmetic).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_mat2_add() {
    mali_ref::assert_matches("deqp_mixprec2_hp_mat2_add");
}

/// `func(out highp float a)` storing a constant (`functions.qualifiers.out_highp_float_fragment`):
/// the highp parameter folds away, the rest is fp16.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_out_param() {
    mali_ref::assert_matches("deqp_mixprec2_hp_out_param");
}

/// `sqrt(in0)` of a highp vec3 (`operator.exponential.sqrt.highp_vec3_fragment`):
/// per lane `FRSQ.f32` then `FRCP.f32`, each lane narrowed alone.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn hp_sqrt_vec3() {
    mali_ref::assert_matches("deqp_mixprec2_hp_sqrt_vec3");
}

/// `random.scalar_conversion.fragment.36`: the refused fp32 sum reads a
/// mediump uniform in a `gl_FragColor` store a later store overwrites; Mesa
/// keeps only the last store.
#[test]
fn rnd_sconv36() {
    mali_ref::assert_matches("deqp_mixprec2_rnd_sconv36");
}

/// `random.swizzle.fragment.4`: an overwritten store's `vec4(l, l, k, j) * i`
/// (int uniforms beside a mediump float uniform).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_swz4() {
    mali_ref::assert_matches("deqp_mixprec2_rnd_swz4");
}

/// `random.texture.fragment.140`: three overwritten stores (two of them
/// products of int uniforms), then `gl_FragColor = a.rbag`; Mesa's program is
/// that swizzle alone.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn rnd_tex140() {
    mali_ref::assert_matches("deqp_mixprec2_rnd_tex140");
}

/// `(in1++, in0 = float(in1), in1 = int(in0 + in2))` of highp locals
/// (`operator.sequence.side_effects.highp_float_int_fragment`): an fp32 sum of
/// a converted int and a highp varying lane.
#[test]
fn seq_side() {
    mali_ref::assert_matches("deqp_mixprec2_seq_side");
}

/// `struct.local.nested_struct_array_fragment`: `float(s[0].c) + s[1].b[2].a
/// - s[1].b[1].a`, an int uniform converted plus two mediump float uniforms
///   Mesa widens (`F16_TO_F32 r, u.wN.h0`).
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn struct_nested() {
    mali_ref::assert_matches("deqp_mixprec2_struct_nested");
}

fn on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_mixprec2_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// The number of 64-bit words of `region` in an expect.json, or why not.
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

#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_mixprec2_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_mixprec2_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_mixprec2_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_mixprec2_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                /* hp_mat2_add's colour is constant: the link removes the
                 * vertex shader's varyings */
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

#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_mixprec2_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_mixprec2_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_mixprec2_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_mixprec2_{name}"));
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
