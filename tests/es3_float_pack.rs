// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The CPU half-float and R11F_G11F_B10F helpers of `src/gl/float_pack.rs`
//! (plan #360, ReadPixels from float colour surfaces): the exhaustive half
//! round trip, the special values, round-to-nearest-even, the packed 11/10-bit
//! known vectors and the row helpers.
//!
//! Feature set: the file needs none, but the package must build, so any one
//! host GPU set works:
//! `cargo test --no-default-features --features host-test,gpu-powervr --test es3_float_pack`.
//! The file under test is not in the module tree yet, so it is included by path.

#[path = "../src/gl/float_pack.rs"]
#[allow(dead_code)]
mod float_pack;

use float_pack::*;

#[test]
fn every_finite_half_round_trips() {
    for h in 0..=u16::MAX {
        let f = half_to_f32(h);
        if h & 0x7c00 == 0x7c00 && h & 0x3ff != 0 {
            assert!(f.is_nan(), "{h:#06x}");
            assert!(half_to_f32(f32_to_half(f)).is_nan(), "{h:#06x}");
        } else {
            assert_eq!(f32_to_half(f), h, "{h:#06x} -> {f}");
        }
    }
}

#[test]
fn half_decode_is_exact_for_every_value() {
    for h in 0..0x7c00u16 {
        let (e, m) = ((h >> 10) as i32, (h & 0x3ff) as f64);
        let want = if e == 0 { m * 2f64.powi(-24) } else { (1.0 + m / 1024.0) * 2f64.powi(e - 15) };
        assert_eq!(half_to_f32(h) as f64, want, "{h:#06x}");
        assert_eq!(half_to_f32(h | 0x8000) as f64, -want, "{h:#06x}");
    }
}

#[test]
fn half_special_values() {
    assert_eq!(f32_to_half(0.0), 0x0000);
    assert_eq!(f32_to_half(-0.0), 0x8000);
    assert_eq!(f32_to_half(1.0), 0x3c00);
    assert_eq!(f32_to_half(-2.0), 0xc000);
    assert_eq!(f32_to_half(65504.0), 0x7bff);
    assert_eq!(f32_to_half(f32::INFINITY), 0x7c00);
    assert_eq!(f32_to_half(f32::NEG_INFINITY), 0xfc00);
    assert_eq!(half_to_f32(0x7c00), f32::INFINITY);
    assert_eq!(half_to_f32(0xfc00), f32::NEG_INFINITY);
    assert_eq!(half_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
    assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
    for nan in [f32::NAN, -f32::NAN, f32::from_bits(0x7f80_0001), f32::from_bits(0xffff_ffff)] {
        let h = f32_to_half(nan);
        assert_eq!(h & 0x7c00, 0x7c00);
        assert_ne!(h & 0x3ff, 0, "{:#x}", nan.to_bits());
        assert!(half_to_f32(h).is_nan());
    }
    // overflow: past the largest finite half, including the rounding edge
    assert_eq!(f32_to_half(65519.0), 0x7bff);
    assert_eq!(f32_to_half(65520.0), 0x7c00);
    assert_eq!(f32_to_half(1.0e30), 0x7c00);
    assert_eq!(f32_to_half(-1.0e30), 0xfc00);
    // underflow: half the smallest subnormal ties to even (zero)
    assert_eq!(f32_to_half(2f32.powi(-25)), 0x0000);
    assert_eq!(f32_to_half(2f32.powi(-25) * 1.0001), 0x0001);
    assert_eq!(f32_to_half(-1.0e-30), 0x8000);
    assert_eq!(f32_to_half(f32::from_bits(1)), 0x0000);
}

#[test]
fn half_rounds_to_nearest_even() {
    let ulp = 2f32.powi(-10);
    assert_eq!(f32_to_half(1.0 + ulp / 2.0), 0x3c00); // tie, even is 1.0
    assert_eq!(f32_to_half(1.0 + ulp * 1.5), 0x3c02); // tie, even is 1 + 2 ulp
    assert_eq!(f32_to_half(1.0 + ulp / 2.0 + 2f32.powi(-20)), 0x3c01);
    // a mantissa carry runs into the exponent
    assert_eq!(f32_to_half(2.0 - ulp / 4.0), 0x4000);
    // the largest subnormal's upper neighbour tie goes up into the smallest normal
    assert_eq!(f32_to_half(2f32.powi(-14) - 2f32.powi(-25)), 0x0400);
    // every midpoint between neighbours rounds to the even one
    for h in 0..0x7bffu16 {
        let mid = (half_to_f32(h) as f64 + half_to_f32(h + 1) as f64) / 2.0;
        let got = f32_to_half(mid as f32);
        assert_eq!(got, if h & 1 == 0 { h } else { h + 1 }, "{h:#06x}");
    }
}

#[test]
fn r11g11b10f_known_vectors() {
    assert_eq!(pack_r11g11b10f([0.0, 0.0, 0.0]), 0);
    assert_eq!(unpack_r11g11b10f(0), [0.0, 0.0, 0.0]);
    // 1.0: exponent 15, mantissa 0
    assert_eq!(pack_r11g11b10f([1.0, 1.0, 1.0]), (15 << 6) | (15 << 17) | (15 << 27));
    assert_eq!(unpack_r11g11b10f(pack_r11g11b10f([1.0, 1.0, 1.0])), [1.0, 1.0, 1.0]);
    // largest finite: 65024 (11 bits), 64512 (10 bits)
    let max = (30 << 6) | 0x3f | (((30 << 6) | 0x3f) << 11) | (((30 << 5) | 0x1f) << 22);
    assert_eq!(pack_r11g11b10f([65024.0, 65024.0, 64512.0]), max);
    assert_eq!(unpack_r11g11b10f(max), [65024.0, 65024.0, 64512.0]);
    // smallest denormals: 2^-20 (11 bits), 2^-19 (10 bits)
    assert_eq!(pack_r11g11b10f([2f32.powi(-20), 0.0, 0.0]), 1);
    assert_eq!(pack_r11g11b10f([0.0, 2f32.powi(-20), 0.0]), 1 << 11);
    assert_eq!(pack_r11g11b10f([0.0, 0.0, 2f32.powi(-19)]), 1 << 22);
    assert_eq!(unpack_r11g11b10f(1), [2f32.powi(-20), 0.0, 0.0]);
    assert_eq!(unpack_r11g11b10f(1 << 22), [0.0, 0.0, 2f32.powi(-19)]);
    // smallest normal 2^-14
    assert_eq!(pack_r11g11b10f([2f32.powi(-14), 0.0, 0.0]), 1 << 6);
    // negative clamps to 0, -0 too
    assert_eq!(pack_r11g11b10f([-1.0, -0.0, f32::NEG_INFINITY]), 0);
    assert_eq!(pack_r11g11b10f([-1.0e-30, 0.5, -3.0]), (14 << 6) << 11);
    // overflow and infinity
    assert_eq!(pack_r11g11b10f([f32::INFINITY, 1.0e10, 65520.0]) & 0x7ff, 0x7c0);
    assert_eq!(pack_r11g11b10f([0.0, 1.0e10, 0.0]), 0x7c0 << 11);
    assert_eq!(pack_r11g11b10f([0.0, 0.0, 65536.0]), 0x3e0 << 22);
    assert_eq!(pack_r11g11b10f([0.0, 0.0, 64513.0]), ((30 << 5) | 0x1f) << 22);
    assert_eq!(unpack_r11g11b10f(0x7c0), [f32::INFINITY, 0.0, 0.0]);
    assert_eq!(unpack_r11g11b10f(0x3e0 << 22)[2], f32::INFINITY);
    // NaN stays NaN in every channel
    let p = pack_r11g11b10f([f32::NAN; 3]);
    assert_eq!(p & 0x7c0, 0x7c0);
    assert_ne!(p & 0x3f, 0);
    assert_ne!((p >> 11) & 0x3f, 0);
    assert_ne!((p >> 22) & 0x1f, 0);
    assert!(unpack_r11g11b10f(p).iter().all(|v| v.is_nan()));
}

#[test]
fn r11g11b10f_every_field_round_trips() {
    for v in 0..0x800u32 {
        let f = unpack_r11g11b10f(v)[0];
        if (v & 0x7c0) == 0x7c0 && v & 0x3f != 0 {
            assert!(f.is_nan());
        } else {
            assert_eq!(pack_r11g11b10f([f, 0.0, 0.0]), v, "11-bit {v:#x}");
            assert_eq!(unpack_r11g11b10f(v << 11)[1], f);
        }
    }
    for v in 0..0x400u32 {
        let f = unpack_r11g11b10f(v << 22)[2];
        if (v & 0x3e0) == 0x3e0 && v & 0x1f != 0 {
            assert!(f.is_nan());
        } else {
            assert_eq!(pack_r11g11b10f([0.0, 0.0, f]), v << 22, "10-bit {v:#x}");
        }
    }
}

#[test]
fn r11g11b10f_rounds_to_nearest_even() {
    // 11-bit: 1.0 has an ulp of 2^-6; the tie between 1.0 and 1 + ulp is even
    let ulp = 2f32.powi(-6);
    assert_eq!(pack_r11g11b10f([1.0 + ulp / 2.0, 0.0, 0.0]) & 0x7ff, 15 << 6);
    assert_eq!(pack_r11g11b10f([1.0 + ulp * 1.5, 0.0, 0.0]) & 0x7ff, (15 << 6) | 2);
    assert_eq!(pack_r11g11b10f([1.0 + ulp / 2.0 + 2f32.powi(-12), 0.0, 0.0]) & 0x7ff, (15 << 6) | 1);
    // 10-bit: ulp 2^-5
    let ulp = 2f32.powi(-5);
    assert_eq!(pack_r11g11b10f([0.0, 0.0, 1.0 + ulp / 2.0]) >> 22, 15 << 5);
    assert_eq!(pack_r11g11b10f([0.0, 0.0, 1.0 + ulp * 1.5]) >> 22, (15 << 5) | 2);
}

#[test]
fn rows_fill_the_missing_channels() {
    let bytes = |v: &[u16]| v.iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>();
    let (one, two, half) = (0x3c00, 0x4000, 0x3800);

    let mut out = [9.0f32; 8];
    assert_eq!(rgba16f_row_to_rgba_f32(&bytes(&[one, two, half, 0, 0xbc00, 0, 0, one]), &mut out), 2);
    assert_eq!(out, [1.0, 2.0, 0.5, 0.0, -1.0, 0.0, 0.0, 1.0]);

    let mut out = [9.0f32; 8];
    assert_eq!(rg16f_row_to_rgba_f32(&bytes(&[one, two, half, one]), &mut out), 2);
    assert_eq!(out, [1.0, 2.0, 0.0, 1.0, 0.5, 1.0, 0.0, 1.0]);

    let mut out = [9.0f32; 8];
    assert_eq!(r16f_row_to_rgba_f32(&bytes(&[two, 0x7c00]), &mut out), 2);
    assert_eq!(out, [2.0, 0.0, 0.0, 1.0, f32::INFINITY, 0.0, 0.0, 1.0]);

    let mut out = [9.0f32; 4];
    let w = pack_r11g11b10f([1.0, 0.5, 2.0]).to_le_bytes();
    assert_eq!(r11g11b10f_row_to_rgba_f32(&w, &mut out), 1);
    assert_eq!(out, [1.0, 0.5, 2.0, 1.0]);
}

#[test]
fn rows_stop_at_the_shorter_side() {
    let src = [0u8; 16]; // 2 RGBA16F texels
    let mut out = [9.0f32; 4]; // room for 1
    assert_eq!(rgba16f_row_to_rgba_f32(&src, &mut out), 1);
    let mut out = [9.0f32; 12]; // room for 3
    assert_eq!(rgba16f_row_to_rgba_f32(&src, &mut out), 2);
    assert_eq!(out[8..], [9.0; 4], "untouched beyond the converted pixels");
    assert_eq!(half_row_to_rgba_f32(&src, 0, &mut out), 0);
    assert_eq!(half_row_to_rgba_f32(&src, 5, &mut out), 0);
    assert_eq!(r16f_row_to_rgba_f32(&src[..3], &mut out), 1, "a trailing odd byte is ignored");
}
