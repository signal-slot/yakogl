// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_constrnd_*` (plan #377, the dEQP
//! shape constop/random_mixed: `shaders.random.*` cases whose first refusal
//! was 'a constant operation on or to a value that is not a finite normal
//! float'; see `docs/notes/mali-deqp-constop.md` and
//! `tools/mali/testdata/deqp_shapes/index.tsv`). One test per reference. A
//! reference the compiler still refuses is `#[ignore]`d with its refusal
//! quoted, and `refusals_pinned` pins that refusal; the lowering removes the
//! ignore and the pin together.
//!
//! `rnd_fragment43` is the shape's own pair
//! (`shaders.random.all_features.fragment.43`, verbatim). The other four are
//! hand-reduced mediump pairs of the random shaders' form (a non-const
//! global read in `main`, so the GLSL front end leaves the fold to NIR), one
//! per constant fold the tip still refuses. Mesa folds each at fp16 and keeps
//! the result as IEEE gives it: an overflow is `0x7c00` (`b * b` of 300.0,
//! `b + b` of 60000.0), a product below the fp16 normal range stays a
//! subnormal (`0x0011` for 0.001 * 0.001, not flushed), and `1.0 / 0.0` is
//! `0x7c00`. The constant colour lane x and the varying lane y are one fp16
//! pair (`MKVEC.v2i16 r0, u2.w0.h0, r0.h0`, FAU word `7c007c00`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_constrnd
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (exp2 feeding texture2DProj; its constant operation is no longer the first refusal), plan #367's fragment ARITH"]
fn constrnd_rnd_fragment43() {
    mali_ref::assert_matches("deqp_constrnd_rnd_fragment43");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (a mediump product folding past 65504 to fp16 infinity)"]
fn constrnd_fp16_mul_inf() {
    mali_ref::assert_matches("deqp_constrnd_fp16_mul_inf");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (a mediump product folding to an fp16 subnormal)"]
fn constrnd_fp16_mul_subnormal() {
    mali_ref::assert_matches("deqp_constrnd_fp16_mul_subnormal");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (a mediump sum folding past 65504 to fp16 infinity)"]
fn constrnd_fp16_add_inf() {
    mali_ref::assert_matches("deqp_constrnd_fp16_add_inf");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (1.0 / 0.0 folding to infinity)"]
fn constrnd_rcp_zero() {
    mali_ref::assert_matches("deqp_constrnd_rcp_zero");
}

/// The refusal text of the constant folds this file's shape is named after.
const CONSTANT_FOLD: &str = "a constant operation on or to a value that is not a finite normal float";

/// Every ignored reference above, and the refusal the compiler gives it now.
const REFUSED: &[(&str, &str)] = &[
    (
        "deqp_constrnd_rnd_fragment43",
        "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)",
    ),
    ("deqp_constrnd_fp16_mul_inf", CONSTANT_FOLD),
    ("deqp_constrnd_fp16_mul_subnormal", CONSTANT_FOLD),
    ("deqp_constrnd_fp16_add_inf", CONSTANT_FOLD),
    ("deqp_constrnd_rcp_zero", CONSTANT_FOLD),
];

#[test]
fn refusals_pinned() {
    let changed: Vec<String> = REFUSED.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
