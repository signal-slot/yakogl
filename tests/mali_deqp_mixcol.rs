// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_mixcol_*` (plan #387, item
//! mixcol-ref, `docs/notes/mali-deqp-mixcol.md`): the dEQP-GLES2 fragment
//! shaders whose first refusal at the tip (bee6896d, HOST-ONLY,
//! `tools/mali/deqp-host-measure.sh`) is still one of the mixprec or
//! colourpair texts:
//!
//! * `a fragment shader that mixes highp and mediump or lowp float values`
//!   (166 cases): a declared `highp` value written into the mediump colour
//!   (`hp_*`), a mediump int uniform read as a float (`int_uniform*`), and a
//!   mediump float converted to int (`f2i_*`);
//! * `a colour pair of a constant then a varying lane` (61) and `a colour
//!   pair of a varying lane and 0.0` (16): the pair as an operand of an
//!   operation, the operator harness's `color * 0.5 + 0.5` (`ck_scale`,
//!   `lz_scale`) and the random shaders' sums and products (`ck_rnd_*`,
//!   `lz_rnd*`).
//!
//! `ck_scale`, `ck_sum` and `lz_scale` are byte-equal since plan #377's
//! spike-colourpair-rest ([`LOWERED`]): the pair `MKVEC.v2i16` (a
//! constant first) or `U16_TO_U32` (a load's lane and 0.0) as the
//! operation's operand. `f2i_seq` is byte-equal and `f2i_dead` compiles
//! with other words ([`DIFFERS`]) since the walk builds nothing for a dead
//! value (`nir_fs.rs` `ir_live`: the sequence's first operand, the dead
//! `int d` and global `a`). Every other reference is refused, so its test
//! is `#[ignore]`d with its refusal and [`refusals_pinned`] asserts the whole
//! `MaliError` message of each one exactly, that each of [`LOWERED`] and
//! [`MATCHED`] is still byte-equal and each of [`DIFFERS`] still compiles and
//! differs, and that every `deqp_mixcol_*` reference on disk is in one of
//! the four lists. The lowering that lifts one removes its `ignore` and
//! moves its entry in the same commit.
//! The draws are `tools/mali/deqp_ref.sh --instrument shim` (drm-shim,
//! `--uniform-values`, GPU g610; the G310's words are the same,
//! `docs/notes/mali-g310-vs-g610.md`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixcol
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const MIX: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";
/// The later refusals of the colourpair rows spike-colourpair-rest lifted
/// past their pair: the colour's lane forms and the conversions.
const FORM: &str = "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";

/// The references byte-equal now, their tests running.
const LOWERED: &[&str] = &["ck_scale", "ck_sum", "lz_scale"];

/// The references that compile byte-equal (their tests run): `hp_mat`, a
/// shader with no default precision whose floats are all declared highp
/// (mod.rs `FsPrecision::implicit_highp`), `hp_loop`, the highp counter
/// folded away with the unrolled loop (`nir_fs.rs` `folded_counter`), and
/// `f2i_seq`, the conversions of the sequence's dead first operand left out
/// of the walk (`nir_fs.rs` `ir_live`).
const MATCHED: &[&str] = &["hp_mat", "hp_loop", "f2i_seq"];

/// The references that compile with other words than Mesa's: `f2i_dead`'s
/// colour holds a `-0.0` half (`80003c00`) where Mesa's FAU word has `0.0`
/// (`00003c00`).
const DIFFERS: &[&str] = &["f2i_dead"];

/// Every reference (without the `deqp_mixcol_` prefix) and the whole
/// `MaliError` message the compiler refuses its pair with now.
const REFUSALS: &[(&str, &str)] = &[
    /* mixprec, declared highp: Mesa computes the highp values at fp32 and
     * narrows each where it meets a mediump value (FADD.f32 .h0) */
    ("hp_scale", MIX),
    ("hp_bool", MIX),
    ("hp_param", MIX),
    ("hp_seq", MIX),
    /* mixprec, an int read as a float: S32_TO_F32 at 32 bits */
    ("int_uniform", MIX),
    ("int_uniform_struct", MIX),
    /* mixprec, a mediump float converted to int: F32_TO_S32.rtz at 32 bits,
     * or folded in Mesa (`* vec4(0, 0, 1, a)`) */
    ("f2i_rnd", MIX),
    /* colourpair, the pair as an operand past its pair: a highp varying's
     * lanes in the mediump colour, and the random shaders' other lanes */
    ("ck_scale_highp", LOAD),
    ("ck_rnd_other", FORM),
    ("lz_rnd", FORM),
    ("lz_rnd_other", LOAD),
];

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (hp_scale)"]
fn mixcol_hp_scale() {
    mali_ref::assert_matches("deqp_mixcol_hp_scale");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (hp_bool)"]
fn mixcol_hp_bool() {
    mali_ref::assert_matches("deqp_mixcol_hp_bool");
}

#[test]
fn mixcol_hp_mat() {
    mali_ref::assert_matches("deqp_mixcol_hp_mat");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (hp_param)"]
fn mixcol_hp_param() {
    mali_ref::assert_matches("deqp_mixcol_hp_param");
}

#[test]
fn mixcol_hp_loop() {
    mali_ref::assert_matches("deqp_mixcol_hp_loop");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (hp_seq)"]
fn mixcol_hp_seq() {
    mali_ref::assert_matches("deqp_mixcol_hp_seq");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (int_uniform)"]
fn mixcol_int_uniform() {
    mali_ref::assert_matches("deqp_mixcol_int_uniform");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (int_uniform_struct)"]
fn mixcol_int_uniform_struct() {
    mali_ref::assert_matches("deqp_mixcol_int_uniform_struct");
}

#[test]
fn mixcol_f2i_seq() {
    mali_ref::assert_matches("deqp_mixcol_f2i_seq");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (f2i_rnd)"]
fn mixcol_f2i_rnd() {
    mali_ref::assert_matches("deqp_mixcol_f2i_rnd");
}

#[test]
#[ignore = "differs: fs.fragment's FAU word 80003c00 ours, 00003c00 Mesa's (a -0.0 half); pinned by refusals_pinned"]
fn mixcol_f2i_dead() {
    mali_ref::assert_matches("deqp_mixcol_f2i_dead");
}

#[test]
fn mixcol_ck_scale() {
    mali_ref::assert_matches("deqp_mixcol_ck_scale");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws (ck_scale_highp)"]
fn mixcol_ck_scale_highp() {
    mali_ref::assert_matches("deqp_mixcol_ck_scale_highp");
}

#[test]
fn mixcol_ck_sum() {
    mali_ref::assert_matches("deqp_mixcol_ck_sum");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (ck_rnd_other)"]
fn mixcol_ck_rnd_other() {
    mali_ref::assert_matches("deqp_mixcol_ck_rnd_other");
}

#[test]
fn mixcol_lz_scale() {
    mali_ref::assert_matches("deqp_mixcol_lz_scale");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (lz_rnd)"]
fn mixcol_lz_rnd() {
    mali_ref::assert_matches("deqp_mixcol_lz_rnd");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws (lz_rnd_other)"]
fn mixcol_lz_rnd_other() {
    mali_ref::assert_matches("deqp_mixcol_lz_rnd_other");
}

/// Each reference is refused with exactly its text in [`REFUSALS`] (the
/// whole `MaliError` message, not a substring), each of [`LOWERED`] and
/// [`MATCHED`] is byte-equal, each of [`DIFFERS`] compiles and differs, and
/// the four lists cover every `deqp_mixcol_*` reference on disk.
#[test]
fn refusals_pinned() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_mixcol_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = REFUSALS
        .iter()
        .map(|(n, _)| *n)
        .chain(LOWERED.iter().copied())
        .chain(MATCHED.iter().copied())
        .chain(DIFFERS.iter().copied())
        .collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_mixcol_* reference has no entry in REFUSALS, LOWERED, MATCHED or DIFFERS");
    let mut changed = Vec::new();
    for (name, refused) in REFUSALS {
        let r = mali_ref::Reference::load(&format!("deqp_mixcol_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; remove its #[ignore] and its entry here")),
            Err(e) if e != *refused => changed.push(format!("{name}: the refusal changed to {e:?}")),
            Err(_) => {}
        }
    }
    for name in LOWERED.iter().chain(MATCHED) {
        let r = mali_ref::Reference::load(&format!("deqp_mixcol_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if let Some(d) = mali_ref::differences(&r, &pair).first() {
                    changed.push(format!("{name}: no longer byte-equal: {d}"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_mixcol_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) if mali_ref::differences(&r, &pair).is_empty() => {
                changed.push(format!("{name}: now byte-equal; move it to MATCHED"))
            }
            Ok(_) => {}
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
