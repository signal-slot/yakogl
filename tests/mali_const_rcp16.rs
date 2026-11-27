// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! CONST_RCP16 (plan #367 lower-train-2, from proto-const-rcp16): the
//! mediump reciprocal of a constant that fp16 does not hold exactly, refract
//! fs 3fae's `refract(.., 1.0 / RefractiveIndex)` of a global `const float`
//! 1.2, against the Mesa 26.2.1 drm-shim reference
//! `tools/mali/testdata/gm2_nbshrf_7`, byte for byte
//! (docs/notes/mali-const-rcp16.md):
//!
//! - `1.0 / c` is `frcp(c)` in NIR; lower_precision rounds `c` to fp16 and
//!   `nir_opt_constant_folding` takes `1.0f / c` in `float` on the half,
//!   then rounds to nearest even: 1.2 is 0x3ccd (1.2001953), its reciprocal
//!   0x3aaa, where the fp32 quotient rounded once is 0x3aab;
//! - an operand or a result that is not zero or a normal half (a reciprocal
//!   of 0.0, an fp16 subnormal, an overflow) has no reference and stays
//!   refused, and so does a highp constant in a mediump shader (Mesa's fp32
//!   reciprocal, refused as a shader that mixes precisions).
//!
//! The `fmul(a, frcp(c))` of a constant `a` is the 16-bit product fold
//! after it (`deqp_cfold2_const_other_rcp16`, byte-equal with this fold,
//! tests/mali_deqp_cfold2.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_const_rcp16
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use yakogl::mali::frame::f32_to_f16;
use yakogl::shaderc::online::mali::fau::FauWord;

/// `vec4(sc.xyz * (1.0 / ratio), 1.0)` of `const float ratio = 1.2`: the
/// FAU word 0x3aaa3aaa, `FMA.v2f16 r0, r0^, u2.w0, 0x0.neg`.
#[test]
fn gm2_nbshrf_7_a_constant_reciprocal_matches_its_reference() {
    mali_ref::assert_matches("gm2_nbshrf_7");
}

/// The rule's arithmetic: round the operand to fp16, divide in f32, round
/// to nearest even; not the fp32 quotient rounded once.
#[test]
fn the_reciprocal_is_of_the_rounded_half() {
    let half = |b: u32| f32::from_bits(yakogl::shaderc::online::mali::select::f16_bits(f32_to_f16(b)));
    let rcp16 = |c: f32| f32_to_f16((1.0 / half(c.to_bits())).to_bits());
    assert_eq!(f32_to_f16(1.2f32.to_bits()), 0x3ccd);
    assert_eq!(rcp16(1.2), 0x3aaa, "gm2_nbshrf_7");
    assert_eq!(f32_to_f16((1.0f32 / 1.2).to_bits()), 0x3aab, "the fp32 quotient, which Mesa does not take");
}

const VS: &str = "attribute vec4 p; varying vec4 sc; void main() { gl_Position = p; sc = p; }";

fn fs(k: &str, expr: &str) -> String {
    format!(
        "precision mediump float; const float k = {k}; varying vec4 sc; \
         void main() {{ gl_FragColor = vec4(sc.xyz * {expr}, 1.0); }}"
    )
}

/// The fragment program's promoted constant word of `sc.xyz * expr`.
fn constant(k: &str, expr: &str) -> u32 {
    let pair = mali_ref::compile_pair(VS, &fs(k, expr)).unwrap_or_else(|e| panic!("k = {k}: refused: {e}"));
    let words: Vec<u32> = pair
        .fs
        .fau
        .iter()
        .filter_map(|w| match *w {
            FauWord::Constant(c) if c != 0x3c00_3c00 => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(words.len(), 1, "k = {k}: constants {words:x?}");
    words[0]
}

/// Other operands the same way, in both lanes of the pair: 1/3, 1/-1.2, and
/// 0.001 (0x1419, 1.0004e-3), whose reciprocal 999.6 is 999.5, 0x63cf;
/// `sc / k` is `sc * frcp(k)` too.
#[test]
fn constant_reciprocals_are_folded_at_fp16() {
    assert_eq!(constant("1.2", "(1.0 / k)"), 0x3aaa_3aaa);
    assert_eq!(constant("-1.2", "(1.0 / k)"), 0xbaaa_baaa);
    assert_eq!(constant("3.0", "(1.0 / k)"), 0x3555_3555);
    assert_eq!(constant("0.001", "(1.0 / k)"), 0x63cf_63cf);
    let pair = mali_ref::compile_pair(VS, &fs("1.2", "(1.0 / k)")).expect("compiles");
    let div = mali_ref::compile_pair(
        VS,
        "precision mediump float; const float k = 1.2; varying vec4 sc; \
         void main() { gl_FragColor = vec4(sc.xyz / k, 1.0); }",
    )
    .expect("compiles");
    assert_eq!(pair.fs.code, div.fs.code, "`sc / k` and `sc * (1.0 / k)` are one program");
}

/// No reference: a reciprocal of 0.0 (an infinity), of an fp16 subnormal
/// (1.0e-5), to an fp16 subnormal (1/30000) and of a value past fp16's
/// range (1.0e6).
#[test]
fn reciprocals_out_of_the_normal_halves_stay_refused() {
    for k in ["0.0", "1.0e-5", "30000.0", "1.0e6"] {
        mali_ref::assert_refused(VS, &fs(k, "(1.0 / k)"), "not a finite normal float");
    }
}

/// A highp constant's reciprocal is fp32 in Mesa (0x3f555555): not the fp16
/// fold, refused with the shader that mixes precisions.
#[test]
fn a_highp_constant_reciprocal_is_not_folded_at_fp16() {
    let fs = "precision mediump float; const highp float k = 1.2; varying vec4 sc; \
              void main() { gl_FragColor = vec4(sc.xyz * (1.0 / k), 1.0); }";
    mali_ref::assert_refused(VS, fs, "mixes highp and mediump");
}
