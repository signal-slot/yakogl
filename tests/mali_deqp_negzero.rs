// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_negzero_*` (plan #387, item
//! negzero-ref: the dEQP-GLES2 cases whose first board refusal was
//! `MaliError::NegZeroShared`, "a -0.0 that a comparison reads through a
//! variable"; see `docs/notes/mali-deqp-negzero.md`). One test per
//! reference:
//!
//! - shape `constglobal` (the refusal comes from global initialisers alone):
//!   `constglobal_fs` (`shaders.random.all_features.fragment.13`),
//!   `constglobal_vs` (`.vertex.10`), and `constglobal_reduced`, fragment.13's
//!   three constant globals that keep the refusal, with a constant colour;
//! - shape `mainbuiltin` (a compare in `main` reads values built from
//!   built-ins on constants): `mainbuiltin_vs` (`all_features.vertex.13`),
//!   `mainbuiltin_fs` (`.fragment.14`);
//! - shape `mainplain` (the same without built-ins: int and bool
//!   conversions, locals stored twice): `mainplain_vs`
//!   (`fragment_ops.interaction.basic_shader.91`), `mainplain_fs`
//!   (`shaders.random.comparison_ops.fragment.43`);
//! - `array_inout_fs`, `array_inout_vs`:
//!   `shaders.functions.array_arguments.copy_local_inout_on_call_{fragment,vertex}`
//!   (shape `array_inout`); at the tip their first refusal is the arithmetic
//!   one, no longer the -0.0 one.
//!
//! The dEQP pairs are verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` (VK-GL-CTS 93bca018).
//!
//! `constglobal_reduced` compiles byte-equal (the -0.0 rule walks constant
//! chains of any depth, folds `float()` of int and bool literals and reads a
//! local's reaching store in `main`'s block), and so does `mainplain_fs`
//! since the walk leaves out the dead values whose arithmetic it refused
//! (`nir_fs.rs` `ir_live`). The others are ignored with
//! their refusal quoted; none stops at the -0.0 rule any more (negzero N2:
//! the rule folds `sqrt`, `inversesqrt`, `exp2`, `log2`, `sin` and `cos` of
//! constants through `mesa_fold`, so `mainbuiltin_fs`'s `log2(float(18.625))`
//! and `sqrt(float(6))` are known not to be -0.0); they stop at arithmetic
//! after it. The lowering removes the
//! `ignore` and the reference's entry in `REFUSALS` together.
//! `refusals_pinned` runs now: it asserts each ignored reference's exact
//! current refusal and names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_negzero
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn negzero_constglobal_fs() {
    mali_ref::assert_matches("deqp_negzero_constglobal_fs");
}

#[test]
fn negzero_constglobal_reduced() {
    mali_ref::assert_matches("deqp_negzero_constglobal_reduced");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (its constant fused multiply-adds fold now)"]
fn negzero_constglobal_vs() {
    mali_ref::assert_matches("deqp_negzero_constglobal_vs");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn negzero_mainbuiltin_vs() {
    mali_ref::assert_matches("deqp_negzero_mainbuiltin_vs");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn negzero_mainbuiltin_fs() {
    mali_ref::assert_matches("deqp_negzero_mainbuiltin_fs");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn negzero_mainplain_vs() {
    mali_ref::assert_matches("deqp_negzero_mainplain_vs");
}

#[test]
fn negzero_mainplain_fs() {
    mali_ref::assert_matches("deqp_negzero_mainplain_fs");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn negzero_array_inout_fs() {
    mali_ref::assert_matches("deqp_negzero_array_inout_fs");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn negzero_array_inout_vs() {
    mali_ref::assert_matches("deqp_negzero_array_inout_vs");
}

const ARITH: &str = "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_negzero_constglobal_fs", ARITH),
    ("deqp_negzero_constglobal_vs", ARITH),
    ("deqp_negzero_mainbuiltin_vs", ARITH),
    ("deqp_negzero_mainbuiltin_fs", ARITH),
    ("deqp_negzero_mainplain_vs", ARITH),
    ("deqp_negzero_array_inout_fs", ARITH),
    ("deqp_negzero_array_inout_vs", ARITH),
];

/// The references that compile byte-equal (their tests run).
const MATCHED: &[&str] = &["deqp_negzero_constglobal_reduced", "deqp_negzero_mainplain_fs"];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_negzero_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_negzero_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
