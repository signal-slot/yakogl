// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Pins the half-float conversions and the coverage model of the host USC
//! byte model (tests/usc_exec) against values worked out by hand from the
//! IEEE 754 binary16 layout (1 sign, 5 exponent with bias 15, 10 mantissa
//! bits), written as literals. Tests such as es3_int_out_alu_pvr use the
//! model through `run_binary` to check what a program writes into its pixel
//! output registers, and the RGBA16F (`pck f16f16`) values rest on these
//! two conversions.
//!
//! Only the public helpers are checked. The pck u8888, f16f16, s32 and u32
//! paths of `run_binary` are private to the interpreter and reachable only
//! through a built USC binary, so they stay untested here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test usc_exec_pck_model
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

mod usc_exec;
use usc_exec::{cov_model, cov_model_of, f16_to_f32, f32_to_f16};

#[test]
fn f32_to_f16_exact_values() {
    // (value, half bits); each pattern derived from sign | exp + 15 | mantissa.
    let cases: [(f32, u16); 9] = [
        (0.0, 0x0000),
        (-0.0, 0x8000),                 // sign bit only
        (1.0, 0x3c00),                  // exp 0 + 15 = 01111, mantissa 0
        (-2.0, 0xc000),                 // sign, exp 1 + 15 = 10000
        (0.5, 0x3800),                  // exp -1 + 15 = 01110
        (65504.0, 0x7bff),              // (2 - 2^-10) * 2^15: exp 11110, mantissa all ones
        (2f32.powi(-24), 0x0001),       // smallest subnormal half, mantissa 1 * 2^-24
        (2f32.powi(-14), 0x0400),       // smallest normal half, exp 00001
        (1.0 + 2f32.powi(-10), 0x3c01), // one mantissa ulp above 1
    ];
    for (x, h) in cases {
        assert_eq!(f32_to_f16(x), h, "{x:e}");
    }
}

#[test]
fn f32_to_f16_overflow_rounds_to_infinity() {
    // 65520 = 65504 + 16 is halfway between 65504 and the would-be 65536; the
    // tie goes to the even mantissa, which is 65536 = infinity (0x7c00).
    assert_eq!(f32_to_f16(65520.0), 0x7c00);
    assert_eq!(f32_to_f16(-65520.0), 0xfc00);
    // The f32 just below 65520 is below the tie and stays 65504.
    assert_eq!(f32_to_f16(f32::from_bits(65520.0f32.to_bits() - 1)), 0x7bff);
    assert_eq!(f32_to_f16(1.0e9), 0x7c00);
    assert_eq!(f32_to_f16(f32::INFINITY), 0x7c00);
    assert_eq!(f32_to_f16(f32::NEG_INFINITY), 0xfc00);
}

#[test]
fn f32_to_f16_ties_round_to_even() {
    // Half ulp at 1.0 is 2^-11. 1 + 2^-11 is halfway between 0x3c00 and
    // 0x3c01: the even one, 0x3c00. 1 + 3*2^-11 is halfway between 0x3c01
    // and 0x3c02: the even one, 0x3c02.
    assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3c00);
    assert_eq!(f32_to_f16(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
    // Just above the first tie rounds up.
    assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11) + 2f32.powi(-20)), 0x3c01);
    // Subnormal ulp is 2^-24: 2^-25 is halfway between 0 and 1 ulp (-> 0),
    // 3*2^-25 halfway between 1 and 2 ulp (-> 2).
    assert_eq!(f32_to_f16(2f32.powi(-25)), 0x0000);
    assert_eq!(f32_to_f16(3.0 * 2f32.powi(-25)), 0x0002);
    assert_eq!(f32_to_f16(-3.0 * 2f32.powi(-25)), 0x8002);
}

#[test]
fn f16_to_f32_exact_values() {
    // Same bit patterns as above, read back by the format's definition.
    assert_eq!(f16_to_f32(0x0000).to_bits(), 0.0f32.to_bits());
    assert_eq!(f16_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
    assert_eq!(f16_to_f32(0x3c00), 1.0);
    assert_eq!(f16_to_f32(0xc000), -2.0);
    assert_eq!(f16_to_f32(0x3800), 0.5);
    assert_eq!(f16_to_f32(0x7bff), 65504.0);
    assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
    assert_eq!(f16_to_f32(0x0400), 2f32.powi(-14));
    assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
    assert_eq!(f16_to_f32(0xfc00), f32::NEG_INFINITY);
}

#[test]
fn nan_stays_nan() {
    // Exponent all ones with a non-zero mantissa is NaN, either sign.
    assert!(f16_to_f32(0x7e00).is_nan());
    assert!(f16_to_f32(0x7c01).is_nan());
    assert!(f16_to_f32(0xfe00).is_nan());
    let h = f32_to_f16(f32::NAN);
    assert_eq!(h & 0x7c00, 0x7c00, "exponent all ones");
    assert_ne!(h & 0x03ff, 0, "mantissa non-zero, not infinity");
    assert!(f16_to_f32(h).is_nan());
}

#[test]
fn every_finite_half_round_trips() {
    // 65536 patterns, 2048 of them (exponent 31) are infinity or NaN; the
    // other 63488 are finite and f32 holds each exactly, so the round trip
    // is the identity on the bits. -0 keeps its sign.
    let mut n = 0u32;
    for h in 0..=u16::MAX {
        if (h >> 10) & 0x1f == 31 {
            continue;
        }
        let x = f16_to_f32(h);
        assert!(x.is_finite(), "{h:#06x}");
        assert_eq!(f32_to_f16(x), h, "{h:#06x} -> {x:e}");
        n += 1;
    }
    assert_eq!(n, 63488);
}

#[test]
fn coverage_of_alpha() {
    let a0 = 0.0f32.to_bits();
    let half = 0.5f32.to_bits();
    let one = 1.0f32.to_bits();
    // Four samples, one per quarter from the first: n = round(alpha * 4)
    // samples, mask (1 << n) - 1.
    assert_eq!(cov_model(a0), 0b0000); // n = 0
    assert_eq!(cov_model(half), 0b0011); // n = 2
    assert_eq!(cov_model(one), 0b1111); // n = 4
    assert_eq!(cov_model_of(a0, 4), 0b0000);
    assert_eq!(cov_model_of(half, 4), 0b0011);
    assert_eq!(cov_model_of(one, 4), 0b1111);
    // Two samples: n = 0, 1, 2.
    assert_eq!(cov_model_of(a0, 2), 0b00);
    assert_eq!(cov_model_of(half, 2), 0b01);
    assert_eq!(cov_model_of(one, 2), 0b11);
    // A single-sampled draw: 1.0 covers its one sample (mask 1), 0.0 none.
    assert_eq!(cov_model_of(a0, 1), 0);
    assert_eq!(cov_model_of(one, 1), 1);
    // Outside [0, 1] clamps; NaN covers nothing.
    assert_eq!(cov_model(2.0f32.to_bits()), 0b1111);
    assert_eq!(cov_model((-1.0f32).to_bits()), 0);
    assert_eq!(cov_model(f32::NAN.to_bits()), 0);
}
