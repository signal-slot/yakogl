// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_constrnd_*` (plan #377, the dEQP
//! shape constop/random_mixed: `shaders.random.*` cases whose first refusal
//! is 'a constant operation on or to a value that is not a finite normal
//! float'; see `docs/notes/mali-deqp-constop.md` and
//! `tools/mali/testdata/deqp_shapes/index.tsv`). One test per reference. A
//! reference the compiler still refuses is `#[ignore]`d with its refusal
//! quoted, and `refusals_pinned` pins that refusal; the lowering removes the
//! ignore and the pin together.
//!
//! Every pair is a dEQP-GLES2 `shaders.random.*` program verbatim (VK-GL-CTS
//! 93bca01's rsg generator at the case's parameters and seed). At 0f0b1ec9,
//! 52 of the 1,900 random programs have this refusal first, all but two of
//! them in the vertex shader (highp, `nir.rs`'s fold of constants). Reduced
//! line by line, they fall into four sub-shapes, one reference each:
//!
//! * `sqrt_zero_vs` (`exponential.vertex.99`) and `atan_zero_lanes_vs`
//!   (`trigonometric.vertex.87`): 22 cases. `sqrt`, `atan` or `exp` of a
//!   constant 0 (a `0`, `false` or `0.0` lane, or a sum that is 0); the
//!   lowering's reciprocal of 0.0 is an infinity.
//! * `int_bits_vs` (`conditionals.vertex.8`): 24 cases. Integer and boolean
//!   constants (`int(1) * 1 == int(int(1.0))`, `ivec ==`, `true`) whose
//!   bits, read as a float, are a subnormal or a NaN.
//! * `lod_const_vs` (`texture.vertex.8`): 4 cases. A constant level of
//!   detail (`textureCubeLod(a, d, 6.5)`, `texture2DLod(a, c, float(b))`).
//! * `atan_const_vs` (`all_features.vertex.95`): 2 cases. `atan` of non-zero
//!   constants, and `sqrt` of a negative constant (a NaN).
//!
//! `rnd_fragment43` is the shape's example in `deqp_shapes`
//! (`all_features.fragment.43`). Its constant fold now compiles, and its
//! first refusal is #367's fragment ARITH.
//!
//! The lowering (nir.rs: the vertex walk's fold of constant lanes, `sqrt`
//! and `ffma` of constants, `frcp(0.0)` as IEEE's infinity, an `if` of a
//! constant condition taken inline, the explicit level's `f2i16` of a
//! constant) makes `lod_const_vs` byte-equal, and `atan_zero_lanes_vs`
//! with the unitype lowering of its mediump int uniform; `sqrt_zero_vs` and
//! `atan_const_vs` fold their constants and refuse later. `int_bits_vs` folds
//! its int and bool compares of constants too (ConstantFold step A, nir.rs
//! `fold_int_alu`) and refuses by name at its `f + e` of a select of two
//! constants, which Mesa's algebraic rules distribute into the select
//! (`CSEL.u32.eq` of two FAU floats) and the vertex walk does not.
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
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (log2(35.625) of a constant, Mesa's libm fold; then exp of an attribute, plan #367's vertex ARITH)"]
fn constrnd_sqrt_zero_vs() {
    mali_ref::assert_matches("deqp_constrnd_sqrt_zero_vs");
}

#[test]
fn constrnd_atan_zero_lanes_vs() {
    mali_ref::assert_matches("deqp_constrnd_atan_zero_lanes_vs");
}

#[test]
#[ignore = "refused: a shape Mesa's algebraic rules rewrite without a reference (an integer sum of a constant and a select of two constants, which Mesa distributes into the select)"]
fn constrnd_int_bits_vs() {
    mali_ref::assert_matches("deqp_constrnd_int_bits_vs");
}

#[test]
fn constrnd_lod_const_vs() {
    mali_ref::assert_matches("deqp_constrnd_lod_const_vs");
}

#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (its cos and log of an attribute are lowered, arith step 2; atan's polynomial of an attribute is next)"]
fn constrnd_atan_const_vs() {
    mali_ref::assert_matches("deqp_constrnd_atan_const_vs");
}

/// The vertex walk's arithmetic refusal (a libm fold of a constant, or
/// #367's vertex ARITH).
const ARITH: &str = "arithmetic other than +, -, * and a dot or matrix product";

/// Every ignored reference above, and the refusal the compiler gives it now.
const REFUSED: &[(&str, &str)] = &[
    (
        "deqp_constrnd_rnd_fragment43",
        "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)",
    ),
    ("deqp_constrnd_sqrt_zero_vs", ARITH),
    ("deqp_constrnd_int_bits_vs", "a shape Mesa's algebraic rules rewrite without a reference"),
    ("deqp_constrnd_atan_const_vs", "a fused multiply-add other than a matrix product's (a built-in function's)"),
];

#[test]
fn refusals_pinned() {
    let changed: Vec<String> = REFUSED.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
