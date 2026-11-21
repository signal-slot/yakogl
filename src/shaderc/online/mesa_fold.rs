// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The constant fold of GLSL built-ins with Mesa 26.2.1's bits
//! (`docs/notes/mali-libm-fold.md` section 4). Mesa folds a built-in on
//! constants with the host's C library: the GLSL IR folder
//! (`ir_expression_operation.py`, [`fold_glsl`]) and, for what reaches NIR,
//! `nir_opcodes.py`'s const expressions ([`fold_nir`], [`fold_nir_f16`],
//! [`fma32`]). The crate is `no_std` without a libm, so this module computes
//! the same values from `core` arithmetic, at the levels of section 4.3:
//!
//! 1. exact: `Sqrt` (correctly rounded, `sqrtf`), `Rsq` (`1.0f / sqrtf`,
//!    two roundings), [`fma32`] (`fmaf`, one rounding); `Atan`, `Atan2` of
//!    the GLSL folder are Mesa's **double** calls, here an f64 evaluation
//!    (error a few 2^-53) rounded once to f32, as Mesa stores the double;
//! 2. evaluate in f64, round once: `Sin`, `Cos`, `Exp2`, `Log2` are
//!    `fmath.rs`'s; `Exp`, `Log`, `Pow`, `Asin`, `Acos` are this module's
//!    f64 kernels. glibc's binary32 functions are not correctly rounded, so
//!    these differ from the host's on a small fraction of arguments
//!    (section 3.3, counted by `tests/mali_libm_fold.rs`); every constant of
//!    the references (section 3.1) agrees.
//!
//! Level 3 (a port of glibc's `sinf`/`cosf`) is not here: no row of
//! section 3.1 needs it.
//!
//! NaN results are `core`'s quiet NaN (0x7fc00000), not the host's pattern
//! (x86-64 libm gives 0xffc00000 for some): callers refuse non-finite folds
//! (section 1.5, 6.6).
//!
//! Not yet declared in `online/mod.rs`: its first caller (cfold step B / CF1
//! in `mali/nir.rs`, or negzero-lower in `glsl.rs`) adds the `mod` line;
//! until then `tests/mali_libm_fold.rs` builds it by `#[path]`, with
//! `fmath.rs` as the root's `mod fmath` (hence `super::fmath`, which is
//! `online::fmath` in the crate).

use super::fmath;

/// A built-in Mesa folds with the host libm (section 1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Libm {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Exp,
    Exp2,
    Log,
    Log2,
    Pow,
    Sqrt,
    Rsq,
}

/// `float(M_LOG2E)`, the factor of `glsl_to_nir.cpp`'s `exp` expansion.
pub(crate) const LOG2E_F32: f32 = core::f64::consts::LOG2_E as f32;
/// `float(1.0 / M_LOG2E)`, the factor of its `log` expansion.
pub(crate) const LN2_F32: f32 = (1.0 / core::f64::consts::LOG2_E) as f32;

/// The binary32 result of the GLSL IR folder (1.1) on binary32 operands:
/// `ir_expression_operation.py`'s C expression. `y` is the second operand
/// of `Atan2` (`atan2(x, y)`: `x` is GLSL's `y`, the numerator) and `Pow`,
/// ignored otherwise. Never `None`: a domain error gives NaN or an infinity,
/// as Mesa stores it.
pub(crate) fn fold_glsl(f: Libm, x: f32, y: f32) -> f32 {
    match f {
        Libm::Sin => fmath::sin(x),
        Libm::Cos => fmath::cos(x),
        /* the built-in's body `sin(x) / cos(x)`, a true f32 division */
        Libm::Tan => fmath::sin(x) / fmath::cos(x),
        Libm::Asin => asin_f32(x),
        Libm::Acos => acos_f32(x),
        Libm::Atan => {
            if x.is_nan() {
                x
            } else {
                atan64(f64::from(x)) as f32
            }
        }
        Libm::Atan2 => atan2_64(x, y) as f32,
        Libm::Exp => exp_f32(x),
        Libm::Exp2 => fmath::exp2(x),
        Libm::Log => ln_f32(x),
        Libm::Log2 => fmath::log2(x),
        Libm::Pow => pow_f32(x, y),
        Libm::Sqrt => fmath::sqrt(x),
        Libm::Rsq => 1.0 / fmath::sqrt(x),
    }
}

/// The same for a built-in NIR folds (1.2), binary32:
/// - `Exp` is `exp2f(x * LOG2E_F32)`, `Log` is `log2f(x) * LN2_F32`
///   (`glsl_to_nir.cpp:2177-2178`, two f32 roundings around the libm call);
/// - `Tan` has no NIR opcode: the GLSL body's division after `lower_fdiv`,
///   `fmul(fsin, frcp(fcos))`, each op rounded (the rule `cfold2
///   const_other_rcp16` shows for an f16 division; unpinned for `tan`);
/// - `Atan`, `Atan2`, `Asin`, `Acos` have no NIR opcode: NIR expands them
///   (`nir_builtin_builder.c` `nir_atan`, `nir_atan2`, `build_asin`) and folds
///   the expansion op by op, `ffma_weak` as `fmaf` (1.4). This is that
///   expansion's value, Mesa's op order, for a lane whose operand is constant
///   only in NIR (`atan(1.0)` gives `0x3f490fa4`, not `atan`'s `0x3f490fdb`);
/// - the others are as [`fold_glsl`].
pub(crate) fn fold_nir(f: Libm, x: f32, y: f32) -> f32 {
    let w = Width::F32;
    match f {
        Libm::Exp => fmath::exp2(x * LOG2E_F32),
        Libm::Log => fmath::log2(x) * LN2_F32,
        Libm::Tan => fmath::sin(x) * (1.0 / fmath::cos(x)),
        Libm::Atan => nir_atan(w, x),
        Libm::Atan2 => nir_atan2(w, x, y),
        Libm::Asin => build_asin(x, true),
        Libm::Acos => nir_acos(x),
        _ => fold_glsl(f, x, y),
    }
}

/// A 16-bit NIR result (1.3): `x`, `y` and the result are f16 bit patterns.
/// Each NIR op widens its f16 operands, evaluates the f32 expression and
/// rounds once with `_mesa_float_to_float16_rtne`; a chain (the `exp`/`log`
/// expansion's `fmul`, `tan`'s `fmul(fsin, frcp(fcos))`, `nir_atan`'s
/// polynomial with f16 constants) rounds after every op. `asin`/`acos` are
/// evaluated at f32 and converted (`nir_asin`'s promotion).
pub(crate) fn fold_nir_f16(f: Libm, x: u16, y: u16) -> u16 {
    let (a, b) = (f16_to_f32(x), f16_to_f32(y));
    let h = Width::F16;
    let r = |v: f32| f32_to_f16(v);
    match f {
        Libm::Exp => r(fmath::exp2(h.mul(a, h.imm(core::f64::consts::LOG2_E)))),
        Libm::Log => r(h.mul(h.round(fmath::log2(a)), h.imm(1.0 / core::f64::consts::LOG2_E))),
        Libm::Tan => {
            let s = h.round(fmath::sin(a));
            let c = h.round(fmath::cos(a));
            r(h.mul(s, h.rcp(c)))
        }
        Libm::Atan => r(nir_atan(h, a)),
        Libm::Atan2 => r(nir_atan2(h, a, b)),
        Libm::Asin => r(build_asin(a, true)),
        Libm::Acos => r(nir_acos(a)),
        _ => r(fold_glsl(f, a, b)),
    }
}

/// `fmaf`: the single rounding of `a * b + c` (NIR `ffma`, `ffma_weak`).
/// The product of two f32 is exact in an f64; the sum is the f64 sum rounded
/// to odd (TwoSum's error as the sticky bit), whose rounding to f32 is the
/// correctly rounded value of the exact sum (53 >= 24 + 2).
pub(crate) fn fma32(a: f32, b: f32, c: f32) -> f32 {
    let p = f64::from(a) * f64::from(b);
    let cd = f64::from(c);
    let s = p + cd;
    if !s.is_finite() || !p.is_finite() {
        /* an infinity or NaN operand: the f64 sum is the IEEE result */
        return s as f32;
    }
    let bb = s - p;
    let e = (p - (s - bb)) + (cd - bb);
    if e == 0.0 {
        return s as f32;
    }
    let bits = s.to_bits();
    if bits & 1 != 0 {
        return s as f32;
    }
    /* s and e have no common bit; step |s| toward the exact sum */
    let toward_larger = (e > 0.0) == (s > 0.0);
    let odd = if toward_larger { bits + 1 } else { bits - 1 };
    f64::from_bits(odd) as f32
}

/// An f16 bit pattern widened to f32 (exact).
pub(crate) fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let e = u32::from((h >> 10) & 0x1f);
    let m = u32::from(h & 0x3ff);
    let bits = if e == 0x1f {
        sign | 0x7f80_0000 | (m << 13)
    } else if e != 0 {
        sign | ((e + 112) << 23) | (m << 13)
    } else if m == 0 {
        sign
    } else {
        /* subnormal: m * 2^-24, exact in f32 */
        let v = (m as f32) * f32::from_bits(0x3380_0000);
        return if sign != 0 { -v } else { v };
    };
    f32::from_bits(bits)
}

/// `_mesa_float_to_float16_rtne`: round to nearest even (x86-64's F16C
/// `vcvtps2ph $0`, the path of the host that drew the references); a NaN
/// keeps its top payload bits and is quieted.
pub(crate) fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let e = ((bits >> 23) & 0xff) as i32;
    let m = bits & 0x7f_ffff;
    if e == 0xff {
        return if m == 0 { sign | 0x7c00 } else { sign | 0x7e00 | (m >> 13) as u16 };
    }
    let ue = e - 127;
    if ue > 15 {
        return sign | 0x7c00;
    }
    if ue < -14 {
        /* below the f16 normals: |v| * 2^24 rounded to an integer (even) */
        if ue < -26 {
            return sign; /* below a quarter of the smallest subnormal */
        }
        let full = m | 0x80_0000; /* |v| = full * 2^(ue - 23) */
        let shift = (-ue - 1) as u32; /* full * 2^(ue+1) = |v| * 2^24 */
        let q = full >> shift;
        let rem = full & ((1u32 << shift) - 1);
        let half = 1u32 << (shift - 1);
        let q = if rem > half || (rem == half && q & 1 != 0) { q + 1 } else { q };
        return sign | q as u16; /* q = 1024 is the smallest normal */
    }
    let mut he = (ue + 15) as u32;
    let mut q = m >> 13;
    let rem = m & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && q & 1 != 0) {
        q += 1;
        if q == 0x400 {
            q = 0;
            he += 1;
        }
    }
    sign | ((he << 10) | q) as u16
}

/* ---- level 2 kernels: f64 evaluation, one rounding to f32 ---- */

/* ln 2 in two pieces (fdlibm's e_exp.c ln2_hi, ln2_lo): n * LN2_HI is exact
 * for |n| < 2^11 */
const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);

/// 2^k as an f64, k in the normal range.
fn pow2(k: i32) -> f64 {
    f64::from_bits(((1023 + k) as u64) << 52)
}

/// `e^r` for |r| <= 0.36 by its Taylor series (to r^17: the first term left
/// out is below 2^-60).
fn exp_kernel(r: f64) -> f64 {
    let mut term = 1.0f64;
    let mut sum = 1.0f64;
    for k in 1..18 {
        term *= r / f64::from(k);
        sum += term;
    }
    sum
}

/// `e^t` in f64 for |t| < 700: `t = n ln2 + r`, `e^r` by its series.
fn exp64(t: f64) -> f64 {
    let q = t * core::f64::consts::LOG2_E;
    let n = if q >= 0.0 { (q + 0.5) as i32 } else { (q - 0.5) as i32 };
    let nd = f64::from(n);
    let r = (t - nd * LN2_HI) - nd * LN2_LO;
    let half = n / 2;
    exp_kernel(r) * pow2(half) * pow2(n - half)
}

/// `2^t` in f64 for |t| < 1000: `t = n + f`, |f| <= 1/2 (exact), `2^f` by
/// the series of `e^(f ln 2)`.
fn exp2_64(t: f64) -> f64 {
    let n = if t >= 0.0 { (t + 0.5) as i32 } else { (t - 0.5) as i32 };
    let f = t - f64::from(n);
    let half = n / 2;
    exp_kernel(f * core::f64::consts::LN_2) * pow2(half) * pow2(n - half)
}

/// `(e, s)` for a positive finite `x = m 2^e`, m in [sqrt(1/2), sqrt(2)),
/// `ln m = 2 atanh(s)`, `s = (m - 1) / (m + 1)`, and `2 atanh(s)` in f64.
fn log_parts(x: f32) -> (i32, f64) {
    let bits = x.to_bits();
    let ex = ((bits >> 23) & 0xff) as i32;
    let mut frac = bits & 0x7f_ffff;
    let mut e = ex - 127;
    if ex == 0 {
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
    /* |s| < 0.172: the series to s^25, first term left out below 2^-62 */
    let mut term = s;
    let mut sum = 0.0f64;
    let mut k = 1.0f64;
    while k < 26.0 {
        sum += term / k;
        term *= s2;
        k += 2.0;
    }
    (e, 2.0 * sum)
}

/// `expf(x)`, correctly rounded but for ties within the f64 error.
fn exp_f32(x: f32) -> f32 {
    if x.is_nan() {
        return x;
    }
    if x > 100.0 {
        return f32::INFINITY;
    }
    if x < -110.0 {
        return 0.0;
    }
    exp64(f64::from(x)) as f32
}

/// `logf(x)`.
fn ln_f32(x: f32) -> f32 {
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return f32::NEG_INFINITY;
    }
    if x == f32::INFINITY {
        return x;
    }
    let (e, l) = log_parts(x);
    let ed = f64::from(e);
    ((ed * LN2_HI + l) + ed * LN2_LO) as f32
}

/// Whether an f32 is an integer, and an odd one.
fn int_parity(y: f32) -> (bool, bool) {
    if !y.is_finite() || fmath::trunc(y) != y {
        return (false, false);
    }
    let odd = y.abs() < 16_777_216.0 && (y as i64) & 1 != 0;
    (true, odd)
}

/// `powf(x, y)` with C99's special cases (F.10.4.4).
fn pow_f32(x: f32, y: f32) -> f32 {
    if y == 0.0 || x == 1.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f32::NAN;
    }
    let (is_int, odd) = int_parity(y);
    let neg = x.is_sign_negative();
    let ax = x.abs();
    if x == 0.0 {
        let mag = if y < 0.0 { f32::INFINITY } else { 0.0 };
        return if odd && neg { -mag } else { mag };
    }
    if y.is_infinite() {
        if ax == 1.0 {
            return 1.0; /* x = -1 */
        }
        return if (ax < 1.0) == (y < 0.0) { f32::INFINITY } else { 0.0 };
    }
    if ax.is_infinite() {
        let mag = if y < 0.0 { 0.0 } else { f32::INFINITY };
        return if neg && odd { -mag } else { mag };
    }
    if neg && !is_int {
        return f32::NAN;
    }
    let (e, l) = log_parts(ax);
    let log2x = f64::from(e) + l * core::f64::consts::LOG2_E;
    let t = f64::from(y) * log2x;
    let mag = if t >= 129.0 {
        f32::INFINITY
    } else if t < -160.0 {
        0.0
    } else {
        exp2_64(t) as f32
    };
    if neg && odd {
        -mag
    } else {
        mag
    }
}

/// The f64 square root of `a` in [2^-120, 2^120], to an f64 ulp or so:
/// the f32 root as the start, then two Newton steps (24 -> 48 -> 53 bits).
fn sqrt64(a: f64) -> f64 {
    let mut r = f64::from(fmath::sqrt(a as f32));
    r = 0.5 * (r + a / r);
    0.5 * (r + a / r)
}

/// `atan(a)` for `a` in [0, 1]: two halvings `a / (1 + sqrt(1 + a^2))` take
/// it below tan(pi/16), the odd series then has 17 terms (the first left out
/// below 2^-60).
fn atan_core(a: f64) -> f64 {
    let a1 = a / (1.0 + sqrt64(1.0 + a * a));
    let a2 = a1 / (1.0 + sqrt64(1.0 + a1 * a1));
    let t2 = a2 * a2;
    let mut term = a2;
    let mut sum = 0.0f64;
    let mut k = 1.0f64;
    let mut sign = 1.0f64;
    while k < 35.0 {
        sum += sign * term / k;
        term *= t2;
        sign = -sign;
        k += 2.0;
    }
    4.0 * sum
}

/// `atan(t)` in f64, signed zero kept, `atan(+-inf) = +-pi/2`.
fn atan64(t: f64) -> f64 {
    if t == 0.0 || t.is_nan() {
        return t;
    }
    let a = t.abs();
    let r = if a > 1.0 { core::f64::consts::FRAC_PI_2 - atan_core(1.0 / a) } else { atan_core(a) };
    if t < 0.0 {
        -r
    } else {
        r
    }
}

/// C's `atan2(y, x)` on f32 operands, with C99's special cases (F.10.1.4).
fn atan2_64(y: f32, x: f32) -> f64 {
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    let sy = if y.is_sign_negative() { -1.0 } else { 1.0 };
    if y == 0.0 {
        /* +-0 for x > 0 or +0, +-pi for x < 0 or -0 */
        return if x.is_sign_negative() { sy * PI } else { f64::from(y) };
    }
    if x == 0.0 {
        return sy * FRAC_PI_2;
    }
    if y.is_infinite() {
        return sy
            * if x.is_infinite() {
                if x > 0.0 {
                    FRAC_PI_4
                } else {
                    3.0 * FRAC_PI_4
                }
            } else {
                FRAC_PI_2
            };
    }
    if x.is_infinite() {
        return if x > 0.0 { sy * 0.0 } else { sy * PI };
    }
    let q = atan64(f64::from(y) / f64::from(x));
    if x > 0.0 {
        q
    } else {
        q + sy * PI
    }
}

/// `sqrt(1 - x^2)` for |x| <= 1: `(1 - x)(1 + x)`, both factors exact.
fn cos_of_asin(x: f32) -> f64 {
    let xd = f64::from(x);
    let c2 = (1.0 - xd) * (1.0 + xd);
    if c2 == 0.0 {
        0.0
    } else {
        sqrt64(c2)
    }
}

/// `asinf(x)`: `atan(x / sqrt(1 - x^2))` in f64, rounded once.
fn asin_f32(x: f32) -> f32 {
    if x.is_nan() || x.abs() > 1.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return x;
    }
    let c = cos_of_asin(x);
    if c == 0.0 {
        return (f64::from(x) * core::f64::consts::FRAC_PI_2) as f32;
    }
    atan64(f64::from(x) / c) as f32
}

/// `acosf(x)`: `atan2(sqrt(1 - x^2), x)` in f64, rounded once.
fn acos_f32(x: f32) -> f32 {
    if x.is_nan() || x.abs() > 1.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return core::f64::consts::FRAC_PI_2 as f32;
    }
    let q = atan64(cos_of_asin(x) / f64::from(x));
    (if x > 0.0 { q } else { q + core::f64::consts::PI }) as f32
}

/* ---- NIR's polynomial expansions (nir_builtin_builder.c), op by op ---- */

/// The bit size of a NIR expansion: every op's f32 value is rounded to it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Width {
    F32,
    F16,
}

impl Width {
    fn round(self, v: f32) -> f32 {
        match self {
            Width::F32 => v,
            Width::F16 => f16_to_f32(f32_to_f16(v)),
        }
    }
    /// `nir_imm_floatN_t(b, v, bit_size)`; the f32 then f16 rounding equals
    /// `_mesa_double_to_float16_rtne` for the constants used here (none is
    /// within an f32 ulp of an f16 tie).
    fn imm(self, v: f64) -> f32 {
        self.round(v as f32)
    }
    fn mul(self, a: f32, b: f32) -> f32 {
        self.round(a * b)
    }
    fn rcp(self, a: f32) -> f32 {
        self.round(1.0 / a)
    }
    /// `ffma_weak`: `fmaf` at 32 bits; at 16 the double `fma` (exact
    /// product, one f64 rounding of the sum), stored to a float, then f16.
    fn fma(self, a: f32, b: f32, c: f32) -> f32 {
        match self {
            Width::F32 => fma32(a, b, c),
            Width::F16 => self.round((f64::from(a) * f64::from(b) + f64::from(c)) as f32),
        }
    }
}

/// `util_min_num`: the bits of both ORed when equal (-0 below +0), else
/// `fmin` (a NaN operand loses).
fn min_num(a: f32, b: f32) -> f32 {
    if a == b {
        return f32::from_bits(a.to_bits() | b.to_bits());
    }
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a < b {
        a
    } else {
        b
    }
}

fn copysign(x: f32, y: f32) -> f32 {
    f32::from_bits((x.to_bits() & 0x7fff_ffff) | (y.to_bits() & 0x8000_0000))
}

/// `nir_atan(y_over_x)`.
fn nir_atan(w: Width, v: f32) -> f32 {
    /* the C float literals -0.0121323213173444f ... 0.9999793128310355f, by
     * their words (the FAU words of the references, section 1.4) */
    const COEFFS: [u32; 6] = [0xbc46_c6a5, 0x3d5b_e101, 0xbdf0_555d, 0x3e46_8bc1, 0xbeaa_5476, 0x3f7f_fea5];
    let le_1 = v.abs() <= 1.0;
    let u = if le_1 { v } else { w.rcp(v) };
    let x_2 = w.mul(u, u);
    let mut res = w.round(f32::from_bits(COEFFS[0]));
    for &c in &COEFFS[1..] {
        res = w.fma(res, x_2, w.round(f32::from_bits(c)));
    }
    let bias = if le_1 { 0.0 } else { w.imm(-core::f64::consts::FRAC_PI_2) };
    let tmp = w.fma(u.abs(), res, bias);
    copysign(tmp, v)
}

/// `nir_atan2(y, x)`.
fn nir_atan2(w: Width, y: f32, x: f32) -> f32 {
    let flip = 0.0 >= x;
    let s = if flip { x.abs() } else { y };
    let t = if flip { y } else { x.abs() };
    let huge = w.imm(if w == Width::F32 { 1e18 } else { 16384.0 });
    let scale = if t.abs() >= huge { 0.25 } else { 1.0 };
    let rcp_scaled_t = w.rcp(w.mul(t, scale));
    let abs_s_over_t = w.mul(w.mul(s, scale).abs(), rcp_scaled_t.abs());
    let tan = if x.abs() == y.abs() { 1.0 } else { abs_s_over_t };
    let flip_f = if flip { 1.0 } else { 0.0 };
    let arc = w.fma(flip_f, w.imm(core::f64::consts::FRAC_PI_2), nir_atan(w, tan));
    if min_num(y, rcp_scaled_t) < 0.0 {
        -arc
    } else {
        arc
    }
}

/// `fsign`: NaN gives 0, a zero itself, else +-1.
fn fsign(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else if x == 0.0 {
        x
    } else if x > 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// `build_asin(x, piecewise)`, always at 32 bits (`nir_asin` promotes f16).
fn build_asin(x: f32, piecewise: bool) -> f32 {
    let abs_x = x.abs();
    let p0 = fma32(abs_x, -0.0187293f64 as f32, 0.0742610f64 as f32);
    let tail = fma32(abs_x, fma32(abs_x, p0, -0.2121144f64 as f32), 1.5707288f64 as f32);
    let root = fmath::sqrt(1.0 - abs_x);
    let result0 = fsign(x) * fma32(-root, tail, core::f32::consts::FRAC_PI_2);
    if piecewise {
        let x2 = x * x;
        let result1 = x * fma32(x2, (1.0f64 / 6.0) as f32, 1.0);
        if abs_x < 0.21502245f64 as f32 {
            result1
        } else {
            result0
        }
    } else {
        result0
    }
}

/// `nir_acos(x)`: `M_PI_2f - build_asin(x, false)`.
fn nir_acos(x: f32) -> f32 {
    core::f32::consts::FRAC_PI_2 - build_asin(x, false)
}
