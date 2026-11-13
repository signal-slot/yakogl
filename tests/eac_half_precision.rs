// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every 11-bit EAC value survives the half-float store of the decode-at-upload
//! design (`docs/notes/etc2-eac.md` sections 2 and 3) within dEQP's colour
//! threshold. R11_EAC / RG11_EAC are decoded to R16F / RG16F with
//! `gl::es3_formats::f32_to_half` (round to nearest even).
//!
//! The decoded value is the spec's: unsigned `u / 2047`, signed `s / 1023`
//! (the signed decode clamps -1024 to -1023, so s is in -1023..=1023).
//!
//! Thresholds (docs/notes/etc2-eac.md section 3; the line numbers are that
//! note's reading of VK-GL-CTS 93bca01, not re-read here: the CTS checkout was
//! not in the deps cache of the machine this was written on):
//! - `es3fTextureWrapTests.cpp:388-399`: `computeColorBitsThreshold(8, colorBits)
//!   / colorScale`, colorBits 7 (nearest only) or 6; with
//!   `tcuTexVerifierUtil.cpp:49-56` `(2^(b-n) + 1/256) / (2^b - 1)` that is
//!   0.0078585 (nearest) / 0.015702 (linear), doubled for the signed formats.
//! - `es3fTextureFormatTests.cpp:192/413/590`: 2/255 = 0.00784, NEAREST.
//!
//! Both signs are held against the tightest, 2/255, and the wrap nearest bound.
//!
//!   cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test eac_half_precision

#![cfg(feature = "host-test")]

use yakogl::gl::es3_formats::{f32_to_half, half_to_f32};

/// dEQP texture.format tolerance, the tightest of the three.
const FORMAT_THRESHOLD: f32 = 2.0 / 255.0;
/// texture.wrap, nearest only, unsigned (signed is double).
const WRAP_NEAREST_THRESHOLD: f32 = 0.0078585;

fn roundtrip(v: f32) -> f32 {
    half_to_f32(f32_to_half(v))
}

/// Returns the maximum error; asserts the stored values are monotonic.
fn sweep(range: core::ops::RangeInclusive<i32>, scale: f32) -> f32 {
    let mut max_err = 0f32;
    let mut prev = f32::NEG_INFINITY;
    for i in range {
        let v = i as f32 / scale;
        let h = roundtrip(v);
        max_err = max_err.max((h - v).abs());
        assert!(h >= prev, "not monotonic at {i}: {h} < {prev}");
        prev = h;
    }
    max_err
}

#[test]
fn unsigned_11bit_survives_r16f() {
    let max_err = sweep(0..=2047, 2047.0);
    println!("unsigned max error {max_err:e}");
    assert!(max_err < FORMAT_THRESHOLD);
    assert!(max_err < WRAP_NEAREST_THRESHOLD);
}

#[test]
fn signed_11bit_survives_r16f() {
    // -1024 decodes as -1023 (spec clamp), so the range is symmetric.
    let max_err = sweep(-1023..=1023, 1023.0);
    println!("signed max error {max_err:e}");
    assert!(max_err < FORMAT_THRESHOLD);
    assert!(max_err < 2.0 * WRAP_NEAREST_THRESHOLD);
}

#[test]
fn end_points_are_exact() {
    assert_eq!(roundtrip(0.0), 0.0);
    assert_eq!(roundtrip(1.0), 1.0);
    assert_eq!(roundtrip(-1.0), -1.0);
    assert_eq!(2047.0f32 / 2047.0, 1.0);
    assert_eq!(-1023.0f32 / 1023.0, -1.0);
}
