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
//! Every reference is refused now, so each test is `#[ignore]`d with its
//! refusal and [`refusals_pinned`] asserts the whole `MaliError` message of
//! each one exactly, and that every `deqp_mixcol_*` reference on disk is in
//! [`REFUSALS`]. The lowering that lifts one removes its `ignore` and its
//! entry in the same commit. The draws are `tools/mali/deqp_ref.sh
//! --instrument shim` (drm-shim, `--uniform-values`, GPU g610; the G310's
//! words are the same, `docs/notes/mali-g310-vs-g610.md`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixcol
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const MIX: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";
const CK: &str = "Valhall: a colour pair of a constant then a varying lane is not lowered yet";
const LZ: &str = "Valhall: a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32) is not lowered yet";

/// The references that compile byte-equal (their tests run): `hp_mat`, a
/// shader with no default precision whose floats are all declared highp
/// (mod.rs `FsPrecision::implicit_highp`).
const MATCHED: &[&str] = &["hp_mat"];

/// Every reference (without the `deqp_mixcol_` prefix) and the whole
/// `MaliError` message the compiler refuses its pair with now.
const REFUSALS: &[(&str, &str)] = &[
    /* mixprec, declared highp: Mesa computes the highp values at fp32 and
     * narrows each where it meets a mediump value (FADD.f32 .h0) */
    ("hp_scale", MIX),
    ("hp_bool", MIX),
    ("hp_param", MIX),
    ("hp_loop", MIX),
    ("hp_seq", MIX),
    /* mixprec, an int read as a float: S32_TO_F32 at 32 bits */
    ("int_uniform", MIX),
    ("int_uniform_struct", MIX),
    /* mixprec, a mediump float converted to int: F32_TO_S32.rtz at 32 bits,
     * or folded or dead in Mesa */
    ("f2i_seq", MIX),
    ("f2i_rnd", MIX),
    ("f2i_dead", MIX),
    /* colourpair, the pair as an operand: MKVEC.v2i16 or U16_TO_U32, then
     * the operation on the word */
    ("ck_scale", CK),
    ("ck_scale_highp", CK),
    ("ck_sum", CK),
    ("ck_rnd_other", CK),
    ("lz_scale", LZ),
    ("lz_rnd", LZ),
    ("lz_rnd_other", LZ),
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
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (hp_loop)"]
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
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (f2i_seq)"]
fn mixcol_f2i_seq() {
    mali_ref::assert_matches("deqp_mixcol_f2i_seq");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (f2i_rnd)"]
fn mixcol_f2i_rnd() {
    mali_ref::assert_matches("deqp_mixcol_f2i_rnd");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) (f2i_dead)"]
fn mixcol_f2i_dead() {
    mali_ref::assert_matches("deqp_mixcol_f2i_dead");
}

#[test]
#[ignore = "refused: a colour pair of a constant then a varying lane (ck_scale)"]
fn mixcol_ck_scale() {
    mali_ref::assert_matches("deqp_mixcol_ck_scale");
}

#[test]
#[ignore = "refused: a colour pair of a constant then a varying lane (ck_scale_highp)"]
fn mixcol_ck_scale_highp() {
    mali_ref::assert_matches("deqp_mixcol_ck_scale_highp");
}

#[test]
#[ignore = "refused: a colour pair of a constant then a varying lane (ck_sum)"]
fn mixcol_ck_sum() {
    mali_ref::assert_matches("deqp_mixcol_ck_sum");
}

#[test]
#[ignore = "refused: a colour pair of a constant then a varying lane (ck_rnd_other)"]
fn mixcol_ck_rnd_other() {
    mali_ref::assert_matches("deqp_mixcol_ck_rnd_other");
}

#[test]
#[ignore = "refused: a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32) (lz_scale)"]
fn mixcol_lz_scale() {
    mali_ref::assert_matches("deqp_mixcol_lz_scale");
}

#[test]
#[ignore = "refused: a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32) (lz_rnd)"]
fn mixcol_lz_rnd() {
    mali_ref::assert_matches("deqp_mixcol_lz_rnd");
}

#[test]
#[ignore = "refused: a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32) (lz_rnd_other)"]
fn mixcol_lz_rnd_other() {
    mali_ref::assert_matches("deqp_mixcol_lz_rnd_other");
}

/// Each reference is refused with exactly its text in [`REFUSALS`] (the
/// whole `MaliError` message, not a substring), and the table covers every
/// `deqp_mixcol_*` reference on disk.
#[test]
fn refusals_pinned() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_mixcol_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_mixcol_* reference has no entry in REFUSALS");
    let mut changed = Vec::new();
    for (name, refused) in REFUSALS {
        let r = mali_ref::Reference::load(&format!("deqp_mixcol_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; remove its #[ignore] and its entry here")),
            Err(e) if e != *refused => changed.push(format!("{name}: the refusal changed to {e:?}")),
            Err(_) => {}
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
