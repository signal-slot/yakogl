// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_cfold2_*` (plan #387, item
//! cfold-ref: the dEQP-GLES2 cases whose first refusal at the tip is
//! `MaliError::ConstantFold`, "a constant operation on or to a value that is
//! not a finite normal float"; see `docs/notes/mali-deqp-cfold2.md`). One
//! test per reference, named `<script shape>_<construct>`:
//!
//! - integer and boolean operations on constants in the vertex walk
//!   (`nir.rs:2166`, `:2253`): `zero_operand_icmp`, `zero_operand_inot`,
//!   `random_mixed_icmp`, `random_mixed_inot`, `random_mixed_f2i`,
//!   `const_other_inot`, `const_other_ior`, `const_other_iabs`,
//!   `const_call_other_bvec`, `const_call_other_ivec`,
//!   `const_call_transcendental_atan` (the `iand` of atan's sign);
//! - `log2` of a constant (`nir.rs:2253`): `const_call_transcendental_log2`,
//!   `const_call_transcendental_pow`;
//! - `rcp`/`rsq` of 0.0, an infinity Mesa clamps or folds away
//!   (`nir.rs:2274`, `:2214`): `zero_operand_rcp_inf`, `random_mixed_rcp_inf`,
//!   `random_mixed_rsq_inf`;
//! - a mediump `rcp` of a constant not exact in fp16 (`nir_fs.rs:6104`):
//!   `zero_operand_rcp16`, `const_other_rcp16`, `random_mixed_rcp16`.
//!
//! The dEQP pairs are verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` (VK-GL-CTS 93bca018); the references
//! are `tools/mali/deqp_ref.sh --instrument shim`.
//!
//! `const_other_ior` compiles byte-equal (the constrnd lowering's `ior` of
//! constants), and so do `random_mixed_f2i` (its constants fold, and its
//! mediump int uniform is the unitype lowering's) and `zero_operand_rcp_inf`
//! (the viewport transform's clamp of `1/w` of a constant zero w);
//! `random_mixed_rcp_inf` and `random_mixed_rsq_inf` fold their constants and
//! refuse later. `random_mixed_rcp16` compiles since its `rcp` is dead and
//! left out of the walk (`nir_fs.rs` `ir_live`), but its vertex program keeps
//! a varying Mesa's link removes ([`DIFFERS`]). Every other test is ignored
//! with its refusal quoted; the lowering removes the `ignore`, moves the name
//! from `REFUSALS` to `MATCHED` in the same commit. `refusals_pinned` runs
//! now: it asserts each ignored reference's exact current refusal, that each
//! of `DIFFERS` still compiles and differs, names every one that moved, and
//! fails when a `deqp_cfold2_*` reference is in no list.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_cfold2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

macro_rules! cfold_ref {
    ($($test:ident => $name:literal,)*) => {
        $(
            #[test]
            #[ignore = "refused: a constant operation on or to a value that is not a finite normal float"]
            fn $test() {
                mali_ref::assert_matches($name);
            }
        )*
    };
}

cfold_ref! {
    cfold2_zero_operand_icmp => "deqp_cfold2_zero_operand_icmp",
    cfold2_zero_operand_inot => "deqp_cfold2_zero_operand_inot",
    cfold2_zero_operand_rcp16 => "deqp_cfold2_zero_operand_rcp16",
    cfold2_random_mixed_icmp => "deqp_cfold2_random_mixed_icmp",
    cfold2_random_mixed_inot => "deqp_cfold2_random_mixed_inot",
    cfold2_const_other_inot => "deqp_cfold2_const_other_inot",
    cfold2_const_other_iabs => "deqp_cfold2_const_other_iabs",
    cfold2_const_other_rcp16 => "deqp_cfold2_const_other_rcp16",
    cfold2_const_call_transcendental_log2 => "deqp_cfold2_const_call_transcendental_log2",
    cfold2_const_call_transcendental_pow => "deqp_cfold2_const_call_transcendental_pow",
    cfold2_const_call_transcendental_atan => "deqp_cfold2_const_call_transcendental_atan",
    cfold2_const_call_other_bvec => "deqp_cfold2_const_call_other_bvec",
    cfold2_const_call_other_ivec => "deqp_cfold2_const_call_other_ivec",
}

/// `(!false || false) && (true ^^ false)`: the `ior` of constants folds (nir.rs
/// `fold_int_const`), byte-equal.
#[test]
fn cfold2_const_other_ior() {
    mali_ref::assert_matches("deqp_cfold2_const_other_ior");
}

/// Its constant operation folds (the vertex walk's fold of constant lanes,
/// `fold::fold_float`), and its mediump int uniform is lowered: byte-equal.
#[test]
fn cfold2_random_mixed_f2i() {
    mali_ref::assert_matches("deqp_cfold2_random_mixed_f2i");
}

/// Its `rcp` of 0.0 is IEEE's infinity, which the select of the front end's
/// `atan` drops (nir.rs `Nir::alu`): a fused multiply-add is next.
#[test]
#[ignore = "refused: a fused multiply-add other than a matrix product's (a built-in function's)"]
fn cfold2_random_mixed_rcp_inf() {
    mali_ref::assert_matches("deqp_cfold2_random_mixed_rcp_inf");
}

/// Its constant operations fold now: fragment arithmetic is next.
#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)"]
fn cfold2_random_mixed_rsq_inf() {
    mali_ref::assert_matches("deqp_cfold2_random_mixed_rsq_inf");
}

/// dEQP's smallest vertex shader, `gl_Position = vec4(0.0)`: the viewport
/// transform's `frcp` of the zero w is +inf, clamped to 2^15 (issue #11328).
#[test]
fn cfold2_zero_operand_rcp_inf() {
    mali_ref::assert_matches("deqp_cfold2_zero_operand_rcp_inf");
}

/// Its `rcp` is dead and left out of the walk (`nir_fs.rs` `ir_live`); the
/// vertex program keeps a varying Mesa's link removes.
#[test]
#[ignore = "differs: a varying program and the position's FAU words Mesa's link has not; pinned by refusals_pinned"]
fn cfold2_random_mixed_rcp16() {
    mali_ref::assert_matches("deqp_cfold2_random_mixed_rcp16");
}

/// `MaliError::ConstantFold`.
const CONSTANT_FOLD: &str = "a constant operation on or to a value that is not a finite normal float";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_cfold2_zero_operand_icmp", CONSTANT_FOLD),
    ("deqp_cfold2_zero_operand_inot", CONSTANT_FOLD),
    ("deqp_cfold2_zero_operand_rcp16", CONSTANT_FOLD),
    ("deqp_cfold2_random_mixed_icmp", CONSTANT_FOLD),
    ("deqp_cfold2_random_mixed_inot", CONSTANT_FOLD),
    ("deqp_cfold2_random_mixed_rcp_inf", "a fused multiply-add other than a matrix product's (a built-in function's)"),
    ("deqp_cfold2_random_mixed_rsq_inf", "arithmetic other than +, -, * and a dot or matrix product"),
    ("deqp_cfold2_const_other_inot", CONSTANT_FOLD),
    ("deqp_cfold2_const_other_iabs", CONSTANT_FOLD),
    ("deqp_cfold2_const_other_rcp16", CONSTANT_FOLD),
    ("deqp_cfold2_const_call_transcendental_log2", CONSTANT_FOLD),
    ("deqp_cfold2_const_call_transcendental_pow", CONSTANT_FOLD),
    ("deqp_cfold2_const_call_transcendental_atan", CONSTANT_FOLD),
    ("deqp_cfold2_const_call_other_bvec", CONSTANT_FOLD),
    ("deqp_cfold2_const_call_other_ivec", CONSTANT_FOLD),
];

/// The references that compile byte-equal (their tests run).
const MATCHED: &[&str] =
    &["deqp_cfold2_const_other_ior", "deqp_cfold2_random_mixed_f2i", "deqp_cfold2_zero_operand_rcp_inf"];

/// The references that compile with other words than Mesa's: the vertex
/// program of `random_mixed_rcp16` stores a varying the fragment shader
/// reads only in dead code, which Mesa's link removes (`vs.varying`).
const DIFFERS: &[&str] = &["deqp_cfold2_random_mixed_rcp16"];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_cfold2_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> =
        REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).chain(DIFFERS.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_cfold2_* reference is neither pinned nor matched here");
    let mut moved: Vec<String> =
        REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    for name in DIFFERS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) if mali_ref::differences(&r, &pair).is_empty() => {
                moved.push(format!("{name}: now byte-equal; move it to MATCHED"))
            }
            Ok(_) => {}
            Err(e) => moved.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
