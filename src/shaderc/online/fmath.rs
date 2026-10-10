// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The float functions the evaluator needs where `core` has none: the crate
//! is `no_std` and has no libm (`Cargo.toml`, no dependencies). Used by the
//! transform-feedback capture, which runs the evaluator in the product
//! (`docs/es3-transform-feedback.md` section 4.2); the host's test oracles
//! keep the host's `std` functions so that their pins do not move.
//!
//! What each one promises, checked against the host's `std` in the tests
//! below (an independent implementation):
//!
//! - `sqrt`, `trunc`, `floor`, `ceil`: exact (IEEE's correctly rounded
//!   square root), every input, signed zeros, subnormals, infinities and NaN
//!   included -- bit for bit what `std` gives.
//! - `exp2`, `log2`: within 1 ULP of the exact result (evaluated in f64,
//!   rounded once to f32); exact where the result is (an integer argument of
//!   `exp2`, a power of two for `log2`). GLSL ES 3.00 section 4.5.1 asks for
//!   3 ULP (`exp2`: 3 + 2|x|; `log2`: 3 ULP outside [0.5, 2], absolute 2^-21
//!   inside).
//! - `sin`, `cos`: within 2^-23 absolute for |x| < 2^19, where the argument
//!   reduction by pi/2 (three f64 pieces, fdlibm's) is exact; past that the
//!   reduction loses bits and the result, still in [-1, 1], drifts. Section
//!   4.5.1 leaves their precision undefined.
//!
//! Every computation is plain `core` arithmetic on f32/f64/integers, so it is
//! the same on the host and on the A53/A72 (an f64 operation is IEEE on both).

/// `x` rounded toward zero, exactly: the fraction's bits cleared.
pub fn trunc(x: f32) -> f32 {
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    if e < 0 {
        return f32::from_bits(bits & 0x8000_0000);
    }
    if e >= 23 {
        return x; /* an integer already, an infinity or a NaN */
    }
    f32::from_bits(bits & !((1u32 << (23 - e)) - 1))
}

/// The largest integer not above `x` (-0.5 gives -1, -0.0 stays -0.0).
pub fn floor(x: f32) -> f32 {
    let t = trunc(x);
    if x < t {
        t - 1.0
    } else {
        t
    }
}

/// The smallest integer not below `x` (-0.5 gives -0.0, as `std` does).
pub fn ceil(x: f32) -> f32 {
    let t = trunc(x);
    if x > t {
        t + 1.0
    } else {
        t
    }
}

/// The integer square root of `n` and whether it was exact.
fn isqrt(n: u64) -> (u64, bool) {
    let (mut r, mut rem, mut bit) = (0u64, n, 1u64 << 62);
    while bit > rem {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= r + bit {
            rem -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    (r, rem == 0)
}

/// The correctly rounded square root: `x = m * 2^e` with `m` normalised to
/// 24 or 25 bits and `e` even, `sqrt(x) = isqrt(m << 32) * 2^(e/2 - 16)`;
/// the root has 28 or 29 bits, and with the remainder as a sticky bit it and
/// the scale are exact in an f64, whose conversion to f32 rounds once.
pub fn sqrt(x: f32) -> f32 {
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 || x == f32::INFINITY {
        return x;
    }
    let bits = x.to_bits();
    let ex = ((bits >> 23) & 0xff) as i32;
    let frac = u64::from(bits & 0x7f_ffff);
    let (mut m, mut e) = if ex == 0 { (frac, -149) } else { (frac | 0x80_0000, ex - 150) };
    while m < 0x80_0000 {
        m <<= 1;
        e -= 1;
    }
    if e & 1 != 0 {
        m <<= 1;
        e -= 1;
    }
    let (r, exact) = isqrt(m << 32);
    /* the sticky bit below the root's last bit: never a tie when inexact */
    let root = (r << 1) | u64::from(!exact);
    let k = e / 2 - 17;
    (root as f64 * pow2(k)) as f32
}

/// 2^k as an f64, for k in the normal range.
fn pow2(k: i32) -> f64 {
    f64::from_bits(((1023 + k) as u64) << 52)
}

/// 2^x: `x = n + f` with |f| <= 1/2, `2^f = e^(f ln 2)` by its Taylor series
/// in f64 (14 terms: the first left out is below 2^-60), scaled by 2^n.
pub fn exp2(x: f32) -> f32 {
    if x.is_nan() {
        return x;
    }
    if x >= 128.0 {
        return f32::INFINITY;
    }
    if x < -151.0 {
        return 0.0;
    }
    let mut n = x as i32; /* toward zero; |x| < 152 */
    let mut f = f64::from(x) - f64::from(n);
    if f > 0.5 {
        n += 1;
        f -= 1.0;
    } else if f < -0.5 {
        n -= 1;
        f += 1.0;
    }
    let y = f * core::f64::consts::LN_2;
    let mut term = 1.0f64;
    let mut sum = 1.0f64;
    for k in 1..14 {
        term *= y / f64::from(k);
        sum += term;
    }
    /* 2^n in two steps, so that n down to -151 stays a normal f64 product */
    let half = n / 2;
    (sum * pow2(half) * pow2(n - half)) as f32
}

/// log2(x): `x = m * 2^e` with m in [sqrt(1/2), sqrt(2)), `ln m = 2 atanh(s)`
/// with `s = (m - 1) / (m + 1)`, |s| < 0.172, by its series in f64 (to s^19).
pub fn log2(x: f32) -> f32 {
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return f32::NEG_INFINITY;
    }
    if x == f32::INFINITY {
        return x;
    }
    let bits = x.to_bits();
    let ex = ((bits >> 23) & 0xff) as i32;
    let mut frac = bits & 0x7f_ffff;
    let mut e = ex - 127;
    if ex == 0 {
        /* subnormal: shift the leading bit up to the implicit position */
        e = -126;
        while frac & 0x80_0000 == 0 {
            frac <<= 1;
            e -= 1;
        }
        frac &= 0x7f_ffff;
    }
    let mut m = 1.0 + f64::from(frac) / 8_388_608.0;
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut term = s;
    let mut sum = 0.0f64;
    let mut k = 1.0f64;
    while k < 20.0 {
        sum += term / k;
        term *= s2;
        k += 2.0;
    }
    (f64::from(e) + 2.0 * sum * core::f64::consts::LOG2_E) as f32
}

/* pi/2 in three pieces of 33 bits plus a tail (fdlibm's e_rem_pio2.c
 * pio2_1, pio2_2, pio2_3, pio2_3t, given here by their bits): k * PIO2_1
 * is exact for k < 2^20 */
const PIO2_1: f64 = f64::from_bits(0x3ff9_21fb_5440_0000);
const PIO2_2: f64 = f64::from_bits(0x3dd0_b461_1a60_0000);
const PIO2_3: f64 = f64::from_bits(0x3ba3_198a_2e00_0000);
const PIO2_3T: f64 = f64::from_bits(0x397b_839a_2520_49c1);

/// `x - k pi/2` for the k nearest to `x / (pi/2)`, and k mod 4.
fn reduce(x: f32) -> (f64, u32) {
    let xd = f64::from(x);
    let q = xd * core::f64::consts::FRAC_2_PI;
    let k = if q >= 0.0 { (q + 0.5) as i64 } else { (q - 0.5) as i64 };
    let kd = k as f64;
    let r = ((xd - kd * PIO2_1) - kd * PIO2_2) - kd * PIO2_3 - kd * PIO2_3T;
    (r, (k & 3) as u32)
}

/// sin(r) and cos(r) for |r| <= pi/4, by their Taylor series in f64 (to
/// r^17 and r^18: the first term left out is below 2^-60).
fn sincos_kernel(r: f64) -> (f64, f64) {
    let r2 = r * r;
    let (mut s, mut st) = (r, r);
    let (mut c, mut ct) = (1.0f64, 1.0f64);
    let mut k = 1.0f64;
    for _ in 0..9 {
        st *= -r2 / ((k + 1.0) * (k + 2.0));
        s += st;
        ct *= -r2 / (k * (k + 1.0));
        c += ct;
        k += 2.0;
    }
    (s, c)
}

/// sin(x).
pub fn sin(x: f32) -> f32 {
    if !x.is_finite() {
        return f32::NAN;
    }
    if x == 0.0 {
        return x; /* the sign of zero */
    }
    let (r, q) = reduce(x);
    let (s, c) = sincos_kernel(r);
    (match q {
        0 => s,
        1 => c,
        2 => -s,
        _ => -c,
    }) as f32
}

/// cos(x).
pub fn cos(x: f32) -> f32 {
    if !x.is_finite() {
        return f32::NAN;
    }
    let (r, q) = reduce(x);
    let (s, c) = sincos_kernel(r);
    (match q {
        0 => c,
        1 => -s,
        2 => -c,
        _ => s,
    }) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every 997th bit pattern, both signs, and the special values.
    fn sweep() -> impl Iterator<Item = f32> {
        let specials = [0.0f32, -0.0, 1.0, -1.0, 0.5, f32::MIN_POSITIVE, 1.0e-45, f32::MAX, f32::INFINITY, f32::NAN];
        (0..=u32::MAX / 997).map(|i| f32::from_bits(i * 997)).chain(specials)
    }

    fn same(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    /// Distance in ULPs between two finite floats of one sign.
    fn ulps(a: f32, b: f32) -> u32 {
        if a == b {
            return 0;
        }
        let key = |x: f32| {
            let b = x.to_bits() as i64;
            if b & 0x8000_0000 != 0 {
                0x8000_0000 - b
            } else {
                b
            }
        };
        (key(a) - key(b)).unsigned_abs() as u32
    }

    /// sqrt, trunc, floor and ceil are std's, bit for bit, over every 997th
    /// bit pattern (subnormals, both zeros, infinities, NaNs among them).
    /// Revert check: drop the subnormal normalisation in `sqrt` and the
    /// subnormal patterns fail.
    #[test]
    fn the_exact_functions_are_std_bit_for_bit() {
        for x in sweep() {
            assert!(same(sqrt(x), x.sqrt()), "sqrt({x:e}) {:#x}: {:e} vs {:e}", x.to_bits(), sqrt(x), x.sqrt());
            assert!(same(trunc(x), x.trunc()), "trunc({x:e})");
            assert!(same(floor(x), x.floor()), "floor({x:e})");
            assert!(same(ceil(x), x.ceil()), "ceil({x:e})");
        }
        for k in 1..4096u32 {
            let sq = (k * k) as f32;
            assert_eq!(sqrt(sq), k as f32, "an exact square");
        }
    }

    /// exp2 and log2 within 1 ULP of std (itself correctly rounded or
    /// nearly), exact at integers and powers of two.
    #[test]
    fn exp2_and_log2_are_within_one_ulp() {
        for x in sweep() {
            let (a, b) = (exp2(x), x.exp2());
            if b.is_nan() {
                assert!(a.is_nan(), "exp2({x:e})");
            } else if b.is_infinite() || b == 0.0 || b < f32::MIN_POSITIVE {
                /* past the ends: the same infinity or zero, a subnormal within one */
                assert!(same(a, b) || ulps(a, b) <= 1, "exp2({x:e}) {a:e} vs {b:e}");
            } else {
                assert!(ulps(a, b) <= 1, "exp2({x:e}) {a:e} vs {b:e}: {} ulps", ulps(a, b));
            }
            let (a, b) = (log2(x), x.log2());
            if b.is_nan() || b.is_infinite() {
                assert!(same(a, b), "log2({x:e}) {a:e} vs {b:e}");
            } else {
                assert!(ulps(a, b) <= 1, "log2({x:e}) {a:e} vs {b:e}: {} ulps", ulps(a, b));
            }
        }
        for n in -149..128 {
            assert_eq!(exp2(n as f32), (n as f32).exp2(), "exp2({n})");
            assert_eq!(log2((n as f32).exp2()), n as f32, "log2(2^{n})");
        }
    }

    /// sin and cos within 2^-23 of std for |x| < 2^19.
    #[test]
    fn sin_and_cos_are_within_their_bound() {
        let bound = 1.0 / 8_388_608.0;
        for x in sweep().filter(|x| x.abs() < 524_288.0) {
            assert!((sin(x) - x.sin()).abs() <= bound, "sin({x:e}) {} vs {}", sin(x), x.sin());
            assert!((cos(x) - x.cos()).abs() <= bound, "cos({x:e}) {} vs {}", cos(x), x.cos());
        }
        assert!(same(sin(-0.0), -0.0));
        assert!(sin(f32::INFINITY).is_nan() && cos(f32::NAN).is_nan());
    }
}
