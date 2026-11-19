// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The CONST_FOLD and NEG_ZERO families of plan #367 (lower-const-fold) on
//! the Mali, against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_constfold_*` and `gm2_negzero_deep_*`
//! (docs/notes/mali-algebraic-folds.md section 9).
//!
//! The -0.0 rules of a compare (glsl.rs `Tree::neg_zero_compares`) ask what
//! NIR's constant folding can make of each operand (`Tree::fold`). That
//! walk stopped at a depth of 12 and at 8 locals folded at once, and read a
//! value it had not reached as one that may be a constant -0.0: a compare
//! of anything computed through more than that was refused as NEG_ZERO
//! (glmark2 terrain fs 1637's `step(h, vec4(0.0))` of its noise value).
//! The walk now keeps every value's lanes it found whole and folds the
//! stage's values in order (`Tree::fill_folds`), so a deep chain is folded
//! to its end: no constant, no -0.0, and the compare stands as written, as
//! Mesa's does. Then Mesa's rule on a compare of a sum with a constant
//! against a constant (nir_opt_algebraic.py:866-871, `Tree::sum_compares`)
//! moves the last sum's constant into the compare: `flt(fadd(a, #b), #c)`
//! is `flt(a, c - b)`, at any depth.
//!
//! The gm2_constfold references stop before any constant fold now: their
//! look-ups at `TextureCoord + vec2(c, 0.0)` are a mediump computed
//! coordinate of two lanes (Mesa's one `FADD_IMM.v2f16` of the varying and
//! the packed constant pair), TEX_COORD for a lone look-up and TEX_MULTI
//! beside another (plan #367 lower-fma). They are pinned at that refusal.
//!
//! Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region. The references
//! were drawn without `--uniform-values`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_const_fold
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `step(h, 0.0)` of a texel through 7 multiply-adds (14 operations, the
/// local `h` one more): the last `* 2.5 + -0.5` is a product compared with
/// 0.5 (`FMA.f32 r0, r0^, u2.w0, 0x0.neg`, `FCMP_OR.f32.ge.m1 r0,
/// 0x38000000.h1, r0^`).
#[test]
fn gm2_negzero_deep_1_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_negzero_deep_1");
}

/// `if (h < 0.0) discard;` of a texel through 11 multiply-adds: the last
/// `* 0.75 + 0.125` is the product against -0.125 (`DISCARD.f32.lt r0^,
/// r1^`).
#[test]
fn gm2_negzero_deep_2_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_negzero_deep_2");
}

/// `step(vec4(h, h * 2.0, h + 1.0, h - 1.0), vec4(0.0))`, terrain fs
/// 1637's vector form, of the same 11 multiply-adds: `h + 1.0` and `h -
/// 1.0` are `h` against -1.0 and 1.0 through the constructor's local and
/// its copy; `h`, read four times, keeps its last sum.
#[test]
fn gm2_negzero_deep_3_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_negzero_deep_3");
}

/// The gm2_constfold references stop at their look-ups' mediump computed
/// coordinates of two lanes, before any fold on 0.0: TEX_MULTI where a
/// look-up is beside another (desktop blur's passes, the effect2d Laplace
/// kernel and its -0.0 weight, the non-separable blur), TEX_COORD for the
/// lone taps.
#[test]
fn gm2_constfold_stop_at_their_coordinates() {
    let multi = "a fragment shader texture look-up beside another";
    let coord = "a texture coordinate other than a vec2 varying read whole or two constants";
    let pins = [
        ("gm2_constfold_1", multi),
        ("gm2_constfold_2", multi),
        ("gm2_constfold_3", multi),
        ("gm2_constfold_4", multi),
        ("gm2_constfold_5", coord),
        ("gm2_constfold_6", coord),
    ];
    let moved: Vec<String> = pins.iter().filter_map(|(n, t)| mali_ref::refusal_change(n, t)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// A texel through `n - 1` multiply-adds, as one expression.
fn nested(n: usize) -> String {
    let mut e = String::from("texture2D(s, v).x");
    for i in 1..n {
        let k = [1.5, 2.5, 0.75][i % 3];
        let b = [0.25, -0.5, 0.125][i % 3];
        e = format!("({e} * {k:?} + {b:?})");
    }
    e
}

/// A compare of a value at any depth: each of these was NEG_ZERO from 8
/// multiply-adds on (the walk cut at 12 operations), and compiles up to
/// 20 (40 meets GLSL's rebalance of the chain, `Rebalance`, another
/// refusal).
#[test]
fn a_compare_of_a_deep_value_compiles_at_any_depth() {
    const VS: &str =
        "attribute vec2 position; varying vec2 v; void main() { v = position; gl_Position = vec4(position, 0.0, 1.0); }";
    let head = "precision highp float; uniform highp sampler2D s; varying vec2 v; void main() {";
    let tails = [
        "gl_FragColor = vec4(h < 0.0 ? h : 0.25, 0.0, 0.0, 1.0);",
        "if (h < 0.0) discard; gl_FragColor = vec4(1.0);",
        "gl_FragColor = vec4(step(h, 0.0), 0.0, 0.0, 1.0);",
        "gl_FragColor = vec4(h - step(h, 0.0), 0.0, 0.0, 1.0);",
        "vec4 hh = vec4(h, h * 2.0, h + 1.0, h - 1.0); gl_FragColor = step(hh, vec4(0.0));",
    ];
    for n in [4, 8, 12, 20] {
        for tail in tails {
            let fs = format!("{head} float h = {}; {tail} }}", nested(n));
            if let Err(e) = mali_ref::compile_pair(VS, &fs) {
                panic!("{n} deep, `{tail}`: {e}");
            }
        }
    }
}

/// What the walk keeps is what it found whole: a -0.0 a compare reads
/// through a deep chain that does fold to it is still refused as before,
/// and a local whose stores make -0.0 is still read through.
#[test]
fn a_negative_zero_under_a_deep_chain_is_still_seen() {
    const VS: &str = "attribute vec2 position; varying vec2 v; void main() { v = position; gl_Position = vec4(position, 0.0, 1.0); }";
    /* `(.. * 0.0) * -1.0`: the products by 0.0 and -1.0 may fold to -0.0
     * (Mesa's `fmul(a, 0.0)` under nsz), which a compare reads */
    let fs = format!(
        "precision highp float; uniform highp sampler2D s; varying vec2 v; void main() {{ float h = ({} * 0.0) * -1.0; \
         gl_FragColor = vec4(h < v.x ? 1.0 : 0.25, 0.0, 0.0, 1.0); }}",
        nested(20)
    );
    mali_ref::assert_refused(VS, &fs, "a -0.0 that a comparison reads through a variable");
}
