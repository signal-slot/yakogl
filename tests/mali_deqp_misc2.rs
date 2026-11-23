// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_misc2_*` (plan #387, item
//! misc2-ref: the small refusal texts no other item owns; see
//! `docs/notes/mali-deqp-misc2.md`). One test per reference:
//!
//! - shape `invariant_arith` (`MaliError::Invariant`, an invariant output
//!   computed with arithmetic): `invariant_cse`, both shaders of
//!   `shaders.invariance.highp.common_subexpression_0` verbatim (its first
//!   program); `invariant_posonly`, that case's invariant `gl_Position`
//!   alone, with a constant varying and a constant colour (our text).
//! - `MaliError::Rebalance` (a chain of three or more additions or
//!   multiplications): `rebalance_funcsum_fs`
//!   (`shaders.functions.misc.multi_arg_float_fragment`, a `sum()` of four
//!   lanes through a function; its `_vertex` twin is arith-ref's
//!   `deqp_arith_other_x_vs_mediump`, the same pair and words, pinned in
//!   `tests/mali_deqp_arith.rs`), `rebalance_invariant`
//!   (`shaders.invariance.highp.common_subexpression_1`, the chain under an
//!   invariant `gl_Position`), `rebalance_random_fs` / `_vs`
//!   (`shaders.random.all_features.{fragment.24,vertex.35}`).
//! - `MaliError::AlgebraicForm`, the two `shaders.random` cases:
//!   `algebraic_random13` / `algebraic_random46`
//!   (`shaders.random.comparison_ops.fragment.{13,46}`).
//! - `MaliError::AttributeNumbering`: `attrnum_zero_lanes`
//!   (`shaders.random.scalar_conversion.vertex.97`, attribute lanes times a
//!   constant 0) and `attrnum_dead_attrs`
//!   (`shaders.random.basic_expression.vertex.49`, attributes read only by
//!   dead code or through a constant -0.0 product).
//!
//! Every dEQP pair is verbatim from the `.qpa` of
//! `tools/mali/deqp-host-measure.sh` (`.license` sidecars). The other texts
//! of the item already have references of other items' tests, or no Mesa
//! reference could be drawn here: the note says which and why.
//!
//! Only `invariant_posonly` compiles (byte-equal), and `invariant_cse`'s
//! vertex shader (`misc2_invariant_cse_vertex`, beside a plain fragment
//! shader); each other test is ignored with its refusal quoted;
//! the lowering removes the `ignore` and the reference's entry in
//! `REFUSALS` together. `refusals_pinned` runs now: it asserts each ignored
//! reference's exact current refusal and names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_misc2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane (the fragment shader's `u_color` lanes beside `blue`; its vertex shader matches, misc2_invariant_cse_vertex)"]
fn misc2_invariant_cse() {
    mali_ref::assert_matches("deqp_misc2_invariant_cse");
}

#[test]
fn misc2_invariant_posonly() {
    mali_ref::assert_matches("deqp_misc2_invariant_posonly");
}

#[test]
#[ignore = "refused: three or more additions or multiplications in a chain"]
fn misc2_rebalance_funcsum_fs() {
    mali_ref::assert_matches("deqp_misc2_rebalance_funcsum_fs");
}

#[test]
#[ignore = "refused: three or more additions or multiplications in a chain"]
fn misc2_rebalance_invariant() {
    mali_ref::assert_matches("deqp_misc2_rebalance_invariant");
}

#[test]
#[ignore = "refused: three or more additions or multiplications in a chain"]
fn misc2_rebalance_random_fs() {
    mali_ref::assert_matches("deqp_misc2_rebalance_random_fs");
}

#[test]
#[ignore = "refused: three or more additions or multiplications in a chain"]
fn misc2_rebalance_random_vs() {
    mali_ref::assert_matches("deqp_misc2_rebalance_random_vs");
}

#[test]
#[ignore = "refused: a back end defect (LaneCycle)"]
fn misc2_algebraic_random13() {
    mali_ref::assert_matches("deqp_misc2_algebraic_random13");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn misc2_algebraic_random46() {
    mali_ref::assert_matches("deqp_misc2_algebraic_random46");
}

#[test]
#[ignore = "refused: a vertex shader whose attribute numbering waits for a fold"]
fn misc2_attrnum_zero_lanes() {
    mali_ref::assert_matches("deqp_misc2_attrnum_zero_lanes");
}

#[test]
#[ignore = "refused: a vertex shader whose attribute numbering waits for a fold"]
fn misc2_attrnum_dead_attrs() {
    mali_ref::assert_matches("deqp_misc2_attrnum_dead_attrs");
}

/// `invariant_cse`'s vertex shader, byte-equal in both vertex programs: the
/// term the invariant `gl_Position` shares with `v_unrelated` exact in both
/// (nir.rs `Nir::mark_exact`), the rest of `v_unrelated` fused. Its own
/// fragment shader is refused (its uniform lanes, [`UNITYPE`]), so the
/// pair here reads `v_unrelated` whole, every lane as the reference's
/// `dot` does; only the vertex regions are compared.
#[test]
fn misc2_invariant_cse_vertex() {
    let r = mali_ref::Reference::load("deqp_misc2_invariant_cse");
    let fs = "varying mediump vec4 v_unrelated;\nvoid main ()\n{\n    gl_FragColor = v_unrelated;\n}\n";
    let pair = mali_ref::compile_pair(&r.vs, fs).unwrap_or_else(|e| panic!("refused: {e}"));
    let d: Vec<String> = mali_ref::differences(&r, &pair)
        .into_iter()
        .filter(|d| d.starts_with("vs") || d.starts_with("secondary"))
        .collect();
    assert!(d.is_empty(), "differs from deqp_misc2_invariant_cse:\n{}", d.join("\n"));
}

/// `invariant_cse`'s next refusal after the per-value exactness (nir.rs
/// `Nir::mark_exact`): its fragment shader's uniform lanes (unitype).
const UNITYPE: &str = "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane";
const REBALANCE: &str =
    "three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree)";
/// The factored sum of constant products (nir_fs.rs `Fs::op`) moved these
/// two past `AlgebraicForm`, and the walk's liveness (nir_fs.rs `ir_live`:
/// their int and bool values are dead) past `an integer or boolean value`,
/// to their next refusal: the pair of a computed lane then a constant asked
/// for again (`random13`, mali-deqp-defects.md fix C) and the colour's
/// lanes (`random46`).
const LANECYCLE: &str = "an invariant of the pipeline did not hold (LaneCycle)";
const FRAGCOLOR: &str = "a gl_FragColor lane other than the varying's own lane or a constant";
const ATTRNUM: &str =
    "a vertex shader whose attribute numbering waits for a fold Mesa's back end makes after it numbers the locations";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_misc2_invariant_cse", UNITYPE),
    ("deqp_misc2_rebalance_funcsum_fs", REBALANCE),
    ("deqp_misc2_rebalance_invariant", REBALANCE),
    ("deqp_misc2_rebalance_random_fs", REBALANCE),
    ("deqp_misc2_rebalance_random_vs", REBALANCE),
    ("deqp_misc2_algebraic_random13", LANECYCLE),
    ("deqp_misc2_algebraic_random46", FRAGCOLOR),
    ("deqp_misc2_attrnum_zero_lanes", ATTRNUM),
    ("deqp_misc2_attrnum_dead_attrs", ATTRNUM),
];

/// The references that compile byte-equal (their tests run): the exact
/// build of an invariant output whose program has no other arithmetic
/// (nir.rs `build`, `Nir::exact`).
const MATCHED: &[&str] = &["deqp_misc2_invariant_posonly"];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_misc2_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_misc2_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
