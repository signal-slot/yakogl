// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Float operations on constants, folded exactly (the V3D's `prepare` and
//! the USC's `select` both use it). It lived in `v3d::prepare` until the V3D
//! compiler became a `gpu-v3d`-only module; the PowerVR compiler calls it
//! too, so it is shared rather than V3D's.

use super::ir::AluOp;

/// A float operation on constants, evaluated exactly as the IR defines it
/// (eval.rs), where core arithmetic can do so: the crate is `no_std` and
/// has no libm. `None` leaves the operation to the QPU:
/// - any NaN, infinity or denormal input or result (the QPU flushes
///   denormals; the host does not);
/// - `exp2` / `log2` / `sin` / `cos` except where the result is exact
///   (an integer exponent, a power of two, a zero).
///
/// `sqrt` is correctly rounded (IEEE, as the host's and Mesa's `sqrtf`);
/// `rsq` is `1 / sqrt(x)` with two roundings, as eval.rs and Mesa's
/// nir_opcodes.py (`1.0f / sqrtf(src0)`) compute it; `rcp` is `1 / x`.
///
/// The USC folds with it too (`powervr::select` `fold_constants`, dEQP
/// random all_features.fragment.53 and vertex.39 on the AM62P).
pub(crate) fn fold_float(alu: AluOp, x: u32, y: u32, z: u32) -> Option<u32> {
    let (fx, fy, fz) = (f32::from_bits(x), f32::from_bits(y), f32::from_bits(z));
    let plain = |v: f32| v == 0.0 || v.is_normal();
    let n = alu.arity();
    if !plain(fx) || (n >= 2 && !plain(fy)) || (n >= 3 && !plain(fz)) {
        /* a bool or int operand is not a float: only the float ops below
         * read these, and they return None for such bits */
        if !matches!(alu, AluOp::I2f | AluOp::B2f | AluOp::I2b) {
            return None;
        }
    }
    let b = |c: bool| if c { u32::MAX } else { 0 };
    let r = match alu {
        AluOp::Fadd => fx + fy,
        AluOp::Fmul => fx * fy,
        AluOp::Ffma => fx * fy + fz,
        AluOp::Fmin => fx.min(fy),
        AluOp::Fmax => fx.max(fy),
        AluOp::Fneg => -fx,
        AluOp::Fabs => fx.abs(),
        AluOp::Fsign => {
            if fx > 0.0 {
                1.0
            } else if fx < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        AluOp::Frcp => 1.0 / fx,
        AluOp::Fsqrt => sqrt_exact(fx)?,
        AluOp::Frsq => 1.0 / sqrt_exact(fx)?,
        AluOp::Fexp2 => {
            if trunc_exact(fx) != fx || !(-126.0..=127.0).contains(&fx) {
                return None;
            }
            f32::from_bits(((fx as i32 + 127) as u32) << 23)
        }
        AluOp::Flog2 => {
            if fx <= 0.0 || x & 0x7f_ffff != 0 {
                return None;
            }
            ((x >> 23) as i32 - 127) as f32
        }
        AluOp::Fsin if fx == 0.0 => fx,
        AluOp::Fcos if fx == 0.0 => 1.0,
        AluOp::Ftrunc => trunc_exact(fx),
        AluOp::Ffloor => floor_exact(fx),
        AluOp::Fceil => -floor_exact(-fx),
        AluOp::Ffract => fx - floor_exact(fx),
        AluOp::Flt => return Some(b(fx < fy)),
        AluOp::Fle => return Some(b(fx <= fy)),
        AluOp::Feq => return Some(b(fx == fy)),
        AluOp::Fne => return Some(b(fx != fy)),
        /* in range only: the QPU's out-of-range ftoiz is not measured */
        AluOp::F2i if fx.abs() < 2147483648.0 => return Some(fx as i32 as u32),
        AluOp::F2b => return Some(b(fx != 0.0)),
        AluOp::I2f => (x as i32) as f32,
        AluOp::B2f => {
            if x != 0 {
                1.0
            } else {
                0.0
            }
        }
        _ => return None,
    };
    plain(r).then_some(r.to_bits())
}

/// `x` rounded toward zero to an integer, exactly (by clearing the
/// fraction's bits).
fn trunc_exact(x: f32) -> f32 {
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    if e < 0 {
        return f32::from_bits(bits & 0x8000_0000);
    }
    if e >= 23 {
        return x;
    }
    f32::from_bits(bits & !((1u32 << (23 - e)) - 1))
}

/// The largest integer not above `x`, exactly.
fn floor_exact(x: f32) -> f32 {
    let t = trunc_exact(x);
    if x < 0.0 && t != x {
        t - 1.0
    } else {
        t
    }
}

/// The correctly rounded square root of a normal, non-negative `x` (or of
/// zero), from the integer square root of its significand: `x = m * 2^e`
/// with `e` even, `sqrt(x) = isqrt(m << 48) * 2^(e / 2 - 24)`, with the
/// remainder as a sticky bit. The ~37-bit root and the sticky bit are exact
/// in an f64, whose conversion to f32 rounds to nearest-even once.
fn sqrt_exact(x: f32) -> Option<f32> {
    if x == 0.0 {
        return Some(x);
    }
    if x < 0.0 || !x.is_normal() {
        return None;
    }
    let bits = x.to_bits();
    let mut m = u128::from((bits & 0x7f_ffff) | 0x80_0000);
    let mut e = ((bits >> 23) & 0xff) as i32 - 150;
    if e & 1 != 0 {
        m <<= 1;
        e -= 1;
    }
    let n = m << 48;
    /* the integer square root, bit by bit */
    let (mut r, mut rem, mut bit) = (0u128, n, 1u128 << 126);
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
    let root = (r << 1) | u128::from(rem != 0);
    /* root * 2^(e/2 - 25), every step exact in f64 */
    let k = e / 2 - 25;
    let scale = f64::from_bits(((1023 + k) as u64) << 52);
    Some((root as u64 as f64 * scale) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sqrt_exact` is IEEE's correctly rounded square root: checked against
    /// the host's `f32::sqrt` (an independent implementation) on every
    /// 4099th positive normal bit pattern and on exact squares, and the
    /// helpers `trunc_exact`/`floor_exact` against the host's.
    #[test]
    fn the_float_folds_are_the_hosts() {
        let mut n = 0u32;
        let mut bits = 0x0080_0000u32;
        while bits < 0x7f80_0000 {
            let x = f32::from_bits(bits);
            assert_eq!(super::sqrt_exact(x).map(f32::to_bits), Some(x.sqrt().to_bits()), "sqrt({x:e}) {bits:#x}");
            let neg = -x;
            assert_eq!(super::trunc_exact(x).to_bits(), x.trunc().to_bits(), "trunc({x:e})");
            assert_eq!(super::floor_exact(neg).to_bits(), neg.floor().to_bits(), "floor({neg:e})");
            bits += 4099;
            n += 1;
        }
        assert!(n > 500_000, "the sweep covered {n} values");
        for k in 1..=4096u32 {
            let x = (k * k) as f32 / 16.0;
            assert_eq!(super::sqrt_exact(x), Some(k as f32 / 4.0), "sqrt({x})");
        }
        assert_eq!(super::sqrt_exact(12.25), Some(3.5));
        assert_eq!(super::sqrt_exact(-1.0), None);
        assert_eq!(super::sqrt_exact(f32::from_bits(1)), None, "a denormal stays on the QPU");
        /* the exact transcendentals fold, the others do not */
        let fold = |op, x: f32| super::fold_float(op, x.to_bits(), 0, 0).map(f32::from_bits);
        assert_eq!(fold(AluOp::Flog2, 16.0), Some(4.0));
        assert_eq!(fold(AluOp::Flog2, 20.0), None);
        assert_eq!(fold(AluOp::Fexp2, -1.0), Some(0.5));
        assert_eq!(fold(AluOp::Fexp2, 0.5), None);
        assert_eq!(fold(AluOp::Fsin, 0.75), None);
        assert_eq!(fold(AluOp::Frcp, 4.0), Some(0.25));
    }
}
