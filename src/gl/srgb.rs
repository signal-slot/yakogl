// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The sRGB transfer function on the CPU, for the 8-bit sRGB storage class
//! (`GLES_TEXFMT_SRGB8_A8`, `GLES_SURF_SRGB8_A8`): what the GL core computes
//! itself where the GPU does not -- glGenerateMipmap's box filter, which
//! averages in linear space -- and the oracle the host tests hold the GPU
//! paths' expectations to (docs/es3/srgb.md, appendix).
//!
//! The formulas are ES 3.0.6's: decode (3.8.16, eq. 3.24)
//!
//! ```text
//!   c = i / 255;  linear = c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055)^2.4
//! ```
//!
//! and encode (4.1.8, eq. 4.1-4.2), the linear value clamped to [0, 1] first
//! (NaN to 0) and the result rounded to the nearest byte:
//!
//! ```text
//!   s = l <= 0.0031308 ? 12.92 * l : 1.055 * l^(1/2.4) - 0.055;  i = round(255 * s)
//! ```
//!
//! Only red, green and blue are converted; alpha is linear. `core` has no
//! `powf`, so both tables are made at compile time from the curve evaluated
//! in f64 with a Newton fifth root (x^2.4 = x^2 * (x^2)^(1/5)): 256 decoded
//! values, and the 255 linear values where the encoded value crosses k + 1/2,
//! which make the encode an exact nearest-byte search with no `powf` and no
//! rounding of its own.

/// x^(1/5) for x in [0, 1], in f64: Newton's iteration from 1, which comes
/// down monotonically onto the root of the convex x^5 - y and stops when a
/// step no longer lowers it (the last bit).
const fn fifth_root(y: f64) -> f64 {
    if y <= 0.0 {
        return 0.0;
    }
    let mut x = 1.0f64;
    let mut i = 0;
    while i < 200 {
        let x4 = x * x * x * x;
        let next = x - (x4 * x - y) / (5.0 * x4);
        if next >= x {
            break;
        }
        x = next;
        i += 1;
    }
    x
}

/// The decode curve at any sRGB value `c` in [0, 1] (not only k / 255).
const fn decode_f64(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        let t = (c + 0.055) / 1.055;
        let t2 = t * t;
        t2 * fifth_root(t2)
    }
}

/// The 256 decoded values, f32.
static DECODE: [f32; 256] = {
    let mut t = [0.0f32; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = decode_f64(i as f64 / 255.0) as f32;
        i += 1;
    }
    t
};

/// Where the encoded byte steps from k to k + 1: the linear value whose
/// encoded value is (k + 1/2) / 255 -- the encode is monotone, so a linear
/// value encodes to the number of these it reaches (round half up).
static STEP: [f64; 255] = {
    let mut t = [0.0f64; 255];
    let mut k = 0;
    while k < 255 {
        t[k] = decode_f64((k as f64 + 0.5) / 255.0);
        k += 1;
    }
    t
};

/// The linear value of an sRGB-encoded byte.
pub fn decode(i: u8) -> f32 {
    DECODE[i as usize]
}

/// The sRGB-encoded byte of a linear value: clamped to [0, 1] (NaN to 0),
/// then the nearest byte on the curve.
pub fn encode(linear: f32) -> u8 {
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(linear > 0.0) {
        return 0;
    }
    let l = linear as f64;
    /* the number of steps at or below l: a binary search over 255 */
    let (mut lo, mut hi) = (0usize, STEP.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if STEP[mid] <= l {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ES 3.0 formulas in f64 with std's `powf`, the reference.
    fn decode_ref(i: u8) -> f64 {
        let c = i as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn encode_ref(l: f64) -> u8 {
        let l = if l.is_nan() { 0.0 } else { l.clamp(0.0, 1.0) };
        let s = if l <= 0.0031308 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5).floor() as u8
    }

    #[test]
    fn every_byte_decodes_as_the_es3_formula_says() {
        for i in 0..=255u8 {
            let want = decode_ref(i);
            let got = decode(i) as f64;
            assert!((got - want).abs() <= want * 1e-6 + 1e-9, "{i}: {got} vs {want}");
        }
        assert_eq!(decode(0), 0.0);
        assert_eq!(decode(255), 1.0);
        /* the knee: 10 / 255 is on the linear segment, 11 / 255 past it */
        assert!((decode(10) as f64 - 10.0 / 255.0 / 12.92).abs() < 1e-9);
        assert!((decode(188) - 0.5029).abs() < 1e-4, "188 is about half: {}", decode(188));
    }

    #[test]
    fn decoding_is_strictly_monotone_and_encoding_inverts_it() {
        for i in 1..=255u8 {
            assert!(decode(i) > decode(i - 1), "{i}");
        }
        for i in 0..=255u8 {
            assert_eq!(encode(decode(i)), i, "encode(decode({i}))");
        }
    }

    #[test]
    fn encoding_matches_the_es3_formula_over_a_dense_sweep() {
        /* 2^20 values across [0, 1], the ends and beyond, NaN */
        let n = 1u32 << 20;
        let mut last = 0u8;
        for k in 0..=n {
            let l = k as f32 / n as f32;
            let got = encode(l);
            assert_eq!(got, encode_ref(l as f64), "encode({l})");
            assert!(got >= last, "monotone at {l}");
            last = got;
        }
        assert_eq!(encode(-0.5), 0);
        assert_eq!(encode(2.0), 255);
        assert_eq!(encode(f32::INFINITY), 255);
        assert_eq!(encode(f32::NEG_INFINITY), 0);
        assert_eq!(encode(f32::NAN), 0);
        /* docs/es3/srgb.md B1: 0.2 linear is byte 124, 0.4 is 170 */
        assert_eq!((encode(0.2), encode(0.4)), (124, 170));
    }
}
