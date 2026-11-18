// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's vertex arithmetic (the vertex ARITH family of plan #367) on
//! the Mali: the steps of the conditionals, function and loop scenes'
//! vertex shaders, against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_vsarith_1..6` (docs/notes/mali-vs-alu-ops.md,
//! "glmark2's vertex steps"), built as glCompileShader and glLinkProgram
//! build them (`tests/mali_ref/mod.rs`) and compared byte for byte in every
//! region. The references were drawn without gl_ref.py's
//! `--uniform-values`: every uniform word of their FAU is 0
//! ([`mali_ref::assert_matches_unset`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vs_arith
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The base every step of the three scenes starts from: `d = fract(x)` and
/// `0.1 * d * fract(x)`, one `fract` after CSE (`FROUND.f32.rtn` and
/// `FADD.f32 x, r.neg`).
#[test]
fn gm2_vsarith_1_fract_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_vsarith_1");
}

/// conditionals' `if (d >= 0.5) d = fract(2.0 * d); else d = fract(3.0 *
/// d);`: both sides computed, one `CSEL` (`nir_opt_peephole_select`).
#[test]
fn gm2_vsarith_2_a_step_conditional_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_vsarith_2");
}

/// conditionals' simple steps: five `fract(3.0 * d)` in a row.
#[test]
fn gm2_vsarith_3_steps_in_a_row_match_their_reference() {
    mali_ref::assert_matches_unset("gm2_vsarith_3");
}

/// function's `process(d)`, whose `fract(3.0 * d) * fract(3.0 * d)` is one
/// `fract` `g` squared: `0.1 * (g * g)` is `(g * 0.1) * g`, NIR's
/// "propagate constants up multiplication chains"
/// (nir_opt_algebraic.py:2393) on the product the local `d` holds, read
/// once (`FMA.f32 r5, r4, 0x3DCCCCCD, 0x0.neg`, then `FMA.f32 r4, r5^,
/// r4^, 0x0.neg`).
#[test]
fn gm2_vsarith_4_a_function_step_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_vsarith_4");
}

/// function's medium step `fract(sqrt(d))`: `sqrt` is `frcp(frsq(d))`
/// (`lower_fsqrt`), `FRSQ.f32` then `FRCP.f32`.
#[test]
fn gm2_vsarith_6_sqrt_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_vsarith_6");
}

/// loop's `for (int i = 0; i < 5; i++) d = fract(3.0 * d);` stays refused
/// at the loop (the int-loop family, not this one): its body is case 3's.
#[test]
fn gm2_vsarith_5_the_loop_stays_refused_at_the_loop() {
    mali_ref::assert_refused_ref("gm2_vsarith_5", "a loop in a vertex shader");
}

/// The constant moves up only over a product read once: a product a later
/// statement reads again keeps its place (`is_used_once`), and the program
/// still compiles.
#[test]
fn a_constant_does_not_move_over_a_product_read_twice() {
    let vs = "attribute vec3 position; varying vec4 v;
        void main() {
            float g = fract(position.x);
            float d = g * g;
            float e = 0.1 * d;
            v = vec4(e, d, 0.0, 1.0);
            gl_Position = vec4(position, 1.0);
        }";
    let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
    mali_ref::compile_pair(vs, fs).unwrap_or_else(|e| panic!("refused: {e}"));
}
