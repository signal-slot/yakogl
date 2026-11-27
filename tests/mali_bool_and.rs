// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 shadow fs cf4b's `&&` of two comparisons on the Mali (plan #367
//! lower-train-2, step BOOL_AND), against the Mesa 26.2.1 drm-shim
//! references `tools/mali/testdata/gm2_nbshrf_1` and `_2`, byte for byte
//! (docs/notes/mali-bool-and.md):
//!
//! - `a && b` of two mediump compares is a 16-bit `iand`
//!   (`LSHIFT_AND.v2i16`), and the flattened `if` selects two constants on
//!   it (`MUX.v2i16`).
//! - A compare of two varyings' lanes is fp32 in Mesa (the late
//!   `flt(f2fmp(a), f2fmp(b))` -> `flt(a, b)`). The `iand` then reads the
//!   32-bit mask's low half, and that compare stands first
//!   (`pan_nir_lower_bool_to_bitsize`'s `i2i16` before the second
//!   `nir_move_comparisons` run).
//!
//! Neither reference has a uniform: both are byte-equal when drawn with
//! and without `--uniform-values`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_bool_and
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `if (sc.w > 0.0 && sc.x < sc.z) s = 0.5;` and `vec4(s, s, s, 1.0)`: the
/// fp32 compare of two varying lanes, the fp16 one of `w` and 0.0, the
/// 16-bit `iand`, one `MUX.v2i16` the colour's words read.
#[test]
fn gm2_nbshrf_1_an_and_of_an_fp32_and_an_fp16_compare_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_nbshrf_1");
    mali_ref::assert_matches("gm2_nbshrf_1");
}

/// The same `&&` with a texel on one side (`d < sc.z`): two fp16 compares,
/// the select scaling `col.rgb` (`FMA.v2f16 r1, r0.h00, r1^, 0x0.neg`).
#[test]
fn gm2_nbshrf_2_an_and_of_two_fp16_compares_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_nbshrf_2");
    mali_ref::assert_matches("gm2_nbshrf_2");
}

const VS: &str = "attribute highp vec4 a_pos;
attribute highp vec4 a_c;
varying vec4 sc;
void main()
{
	gl_Position = a_pos;
	sc = a_pos * 0.5 + a_c;
}";

/// The fp32 compare first in the `&&`: Mesa's `iand` is then 32 bits wide
/// (its first source's width), which no reference draws.
#[test]
fn an_fp32_compare_first_in_the_and_stays_refused() {
    let fs = "precision mediump float;
varying vec4 sc;
void main()
{
	float s = 1.0;
	if (sc.x < sc.z && sc.w > 0.0)
		s = 0.5;
	gl_FragColor = vec4(s, s, s, 1.0);
}";
    mali_ref::assert_refused(VS, fs, "an integer or boolean value is not lowered yet");
}

/// Two different selects on one `&&` in the colour (`vec4(o0, o1, ..)` of
/// one `if`, dEQP's conditionals.if.complex_condition): no reference
/// draws the pair of selects. Refused before the selection.
#[test]
fn two_selects_on_one_and_stay_refused() {
    let fs = "precision mediump float;
varying vec4 sc;
void main()
{
	float o0 = 0.25;
	float o1 = 0.5;
	if (sc.w > 0.0 && sc.x < sc.z) {
		o0 = 0.5;
		o1 = 0.25;
	}
	gl_FragColor = vec4(o0, o1, 0.0, 1.0);
}";
    mali_ref::assert_refused(VS, fs, "a fragment shader operand of a form or in a place no reference draws");
}
