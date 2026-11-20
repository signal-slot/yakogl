// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! `deqp_varylink2_*` references (`tools/mali/testdata/`): the dEQP-GLES2
//! shapes that still meet one of the varying layout refusals (`two varying
//! lanes of equal value`, `a varying lane computed from no attribute`, `a
//! gl_Position lane left unwritten`) first at cc89a1cf, after the lowering of
//! `docs/notes/mali-deqp-varylink.md`. Counts, Mesa's link per shape and the
//! proposed lowering are in `docs/notes/mali-deqp-varylink2.md`.
//!
//! Every reference is refused at the tip: each test is `#[ignore]`d with its
//! refusal quoted. `refusals_pinned` runs now and asserts each reference's
//! exact current refusal, so the lowering that reaches one moves its pin and
//! removes its ignore. Shape U (`a varying lane left unwritten`) and the
//! struct sampler lane are pinned by `tests/mali_deqp_varylink.rs`
//! (`deqp_varylink_unwritten`) and `varymerge_structsampler`, not here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varylink2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const EQUAL: &str = "Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet";
const NO_ATTRIBUTE: &str = "Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet";
const POSITION: &str = "Valhall: a gl_Position lane left unwritten is not lowered yet";

/// Each reference and the refusal it meets at the tip.
const PINS: [(&str, &str); 8] = [
    ("swz_mixed", EQUAL),
    ("prec_mixed", EQUAL),
    ("mat2", EQUAL),
    ("loop_bcast", EQUAL),
    ("vec_uniform", NO_ATTRIBUTE),
    ("scalar_uniform", NO_ATTRIBUTE),
    ("loop_uniform", NO_ATTRIBUTE),
    ("fract_vec2", POSITION),
];

/// `out0 = in0.grg`: Mesa merges lane 2 into lane 0 at the link and stores
/// two lanes (`VAR0.xy`).
#[test]
#[ignore = "refused at the link: two varying lanes of equal value (Mesa's link merges them); pinned by refusals_pinned"]
fn swz_mixed() {
    mali_ref::assert_matches("deqp_varylink2_swz_mixed");
}

/// `var = vec4(in0, 2.0*in0, -in0, -in0)`: Mesa merges `w` into `z` and
/// stores three lanes (`VAR0.xyz`).
#[test]
#[ignore = "refused at the link: two varying lanes of equal value (Mesa's link merges them); pinned by refusals_pinned"]
fn prec_mixed() {
    mali_ref::assert_matches("deqp_varylink2_prec_mixed");
}

/// `out0 = mat2(in0)`: the matrix is one packed vec4; Mesa merges nothing
/// and stores all four lanes, the two 0.0 included.
#[test]
#[ignore = "refused at the link: two varying lanes of equal value (Mesa's link merges them); pinned by refusals_pinned"]
fn mat2() {
    mali_ref::assert_matches("deqp_varylink2_mat2");
}

/// `v_color = vec3(res)` of a run-time loop's sum: Mesa stores the one
/// converted value three times (`VAR0.xyz`).
#[test]
#[ignore = "refused: two varying lanes of equal value (Mesa's link merges them), the vertex shader alone; pinned by refusals_pinned"]
fn loop_bcast() {
    mali_ref::assert_matches("deqp_varylink2_loop_bcast");
}

/// `v_color = vec4(i + h, g, f, b)` with `g` a uniform: Mesa stores `g` as a
/// plain interpolated lane of the one smooth vec4.
#[test]
#[ignore = "refused: a varying lane computed from no attribute, the vertex shader alone; pinned by refusals_pinned"]
fn vec_uniform() {
    mali_ref::assert_matches("deqp_varylink2_vec_uniform");
}

/// `a = 3.0 * n` beside interpolated varyings: the fragment shader reads
/// only `a`, which Mesa keeps as one flat lane (`ffma(x, 0.0, x)`).
#[test]
#[ignore = "refused: a varying lane computed from no attribute, the vertex shader alone; pinned by refusals_pinned"]
fn scalar_uniform() {
    mali_ref::assert_matches("deqp_varylink2_scalar_uniform");
}

/// `v_color = vec4(res, 0.0, 0.0, 1.0)` with `res` a loop over constants:
/// Mesa stores all four lanes, smooth.
#[test]
#[ignore = "refused: a varying lane computed from no attribute, the vertex shader alone; pinned by refusals_pinned"]
fn loop_uniform() {
    mali_ref::assert_matches("deqp_varylink2_loop_uniform");
}

/// `color.yz = fract(in0); color * 0.80 + 0.10`: Mesa computes the four
/// lanes at fp16 and stores each.
#[test]
#[ignore = "refused: a gl_Position lane left unwritten (an undefined operand lane, not gl_Position); pinned by refusals_pinned"]
fn fract_vec2() {
    mali_ref::assert_matches("deqp_varylink2_fract_vec2");
}

/// The refusal each reference meets at the tip; one message lists every
/// reference that moved.
#[test]
fn refusals_pinned() {
    let moved: Vec<String> = PINS
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_varylink2_{name}"), text))
        .collect();
    assert!(moved.is_empty(), "pins moved:\n{}", moved.join("\n"));
}
