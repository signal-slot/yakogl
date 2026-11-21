// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/shaderc/online/mesa_fold.rs`, the constant fold of built-ins with
//! Mesa 26.2.1's bits (`docs/notes/mali-libm-fold.md` sections 3 to 5): every
//! row of the note's tables 3.1 and 3.2 as bits, the two `exp`/`log` paths
//! apart, `fma32` against `f32::mul_add`, NIR's `atan(1.0)` polynomial, and
//! the count of arguments where the helper differs from the host's libm
//! (section 3.3's counts as upper bounds).
//!
//! Feature set: the file itself needs none, but the package must build, and
//! the plain `cargo test --test mali_libm_fold` does not (the default
//! features select no GPU). Use the Mali tests' set:
//! `cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_libm_fold`.
//! The module is not yet declared in `src/shaderc/online/mod.rs`, so this
//! file includes it and `fmath.rs` by path (`fmath.rs`'s own unit tests run
//! here too); its first caller declares it (note section 5).
//!
//! The counts compare with `std`, which is the host's glibc: they were
//! measured on glibc 2.43 (x86-64). Another libm can move them; a count
//! above its bound means the helper drifted or the host differs (section
//! 4.3, level 3).

#![allow(dead_code)]

#[path = "../src/shaderc/online/fmath.rs"]
mod fmath;
#[path = "../src/shaderc/online/mesa_fold.rs"]
mod mesa_fold;

use mesa_fold::{f16_to_f32, f32_to_f16, fma32, fold_glsl, fold_nir, fold_nir_f16, Libm};

fn bits(v: f32) -> u32 {
    v.to_bits()
}

/* ---- section 3.1: the libm results that reach a word ---- */

#[test]
fn row_sin_3() {
    let v = fold_glsl(Libm::Sin, 3.0, 0.0);
    assert_eq!(bits(v), 0x3e10_81c3, "arith_constexpr_fsin_fs_highp");
    assert_eq!(f32_to_f16(v), 0x3084, "arith_constexpr_fsin_vs_highp");
}

#[test]
fn row_cos_3_2() {
    let v = fold_glsl(Libm::Cos, 3.2, 0.0);
    assert_eq!(bits(v), 0xbf7f_903f, "arith_constexpr_fcos_fs_highp");
    assert_eq!(f32_to_f16(v), 0xbbfd, "arith_constexpr_fcos_vs_highp");
}

#[test]
fn row_exp2_6_7() {
    let v = fold_glsl(Libm::Exp2, 6.7, 0.0);
    assert_eq!(bits(v), 0x42cf_efc4, "arith_constexpr_fexp2_fs_highp");
    assert_eq!(f32_to_f16(v), 0x567f, "arith_constexpr_fexp2_vs_highp");
}

#[test]
fn row_log2_100() {
    let v = fold_glsl(Libm::Log2, 100.0, 0.0);
    assert_eq!(bits(v), 0x40d4_9a78, "cfold2_const_call_transcendental_log2");
    assert_eq!(f32_to_f16(v), 0x46a5, "arith_constexpr_flog2_vs_highp");
}

#[test]
fn row_atan_2() {
    assert_eq!(bits(fold_glsl(Libm::Atan, 2.0, 0.0)), 0x3f8d_b70d, "cfold2_const_call_transcendental_atan");
}

#[test]
fn row_pow_1_7_3_5() {
    assert_eq!(bits(fold_glsl(Libm::Pow, 1.7, 3.5)), 0x40cc_fc0f, "cfold2_const_call_transcendental_pow");
}

#[test]
fn row_atan2_minus_1_minus_1() {
    let v = fold_glsl(Libm::Atan2, -1.0, -1.0);
    assert_eq!(bits(v), 0xc016_cbe4, "constprod_fma_foldcall_atan_vs, f32");
    assert_eq!(f32_to_f16(v), 0xc0b6, "constprod_fma_foldcall_atan_vs, f16");
}

#[test]
fn row_tan_sqrt_sqrt_quarter() {
    let s = fold_glsl(Libm::Sqrt, fold_glsl(Libm::Sqrt, 0.25, 0.0), 0.0);
    let v = fold_glsl(Libm::Tan, s, 0.0);
    assert_eq!(bits(v), 0x3f5a_c132, "cfold2_random_mixed_rsq_inf, f32");
    assert_eq!(f32_to_f16(v), 0x3ad6, "cfold2_random_mixed_rsq_inf, f16");
}

#[test]
fn row_asin_0_375() {
    let v = fold_glsl(Libm::Asin, 0.375, 0.0);
    assert_eq!(bits(v), 0x3ec4_cfa7, "constprod_reassoc_mixed_vs");
    assert_eq!(bits(-v), 0xbec4_cfa7, "constprod_reassoc_mixed_vs, negated");
}

/// `negzero_mainbuiltin_vs`: the lane `atan(g)` at 1.0 is NIR's
/// polynomial (`nir_atan`, `ffma_weak` folded as `fmaf`), not `atan`.
#[test]
fn row_atan_1_nir_polynomial() {
    /* nir_atan's coefficients as Mesa's f32 words (FAU words of the refs) */
    let c = [0xbc46_c6a5u32, 0x3d5b_e101, 0xbdf0_555d, 0x3e46_8bc1, 0xbeaa_5476, 0x3f7f_fea5].map(f32::from_bits);
    let lits = [
        -0.012_132_321_317_344_4f64,
        0.053_681_378_431_040_6,
        -0.117_350_319_478_685_1,
        0.193_892_497_711_561,
        -0.332_675_641_809_124_6,
        0.999_979_312_831_035_5,
    ];
    for (w, l) in c.iter().zip(lits) {
        assert_eq!(bits(*w), bits(l as f32), "coefficient {l}");
    }
    let u = 1.0f32;
    let x2 = u * u;
    let mut res = c[0];
    for &k in &c[1..] {
        res = fma32(res, x2, k);
    }
    let tmp = fma32(u, res, 0.0);
    assert_eq!(bits(tmp), 0x3f49_0fa4, "the polynomial through fma32");
    assert_eq!(bits(fold_nir(Libm::Atan, 1.0, 0.0)), 0x3f49_0fa4, "fold_nir");
    assert_eq!(bits(fold_glsl(Libm::Atan, 1.0, 0.0)), 0x3f49_0fdb, "the GLSL folder's atan(1.0)");
}

/// `build_asin`'s coefficients are the FAU words of
/// `deqp_constprod_fma_trig_acos_vs` (section 3.4).
#[test]
fn asin_coefficients_are_the_reference_words() {
    let got = [-0.0187293f64, 0.0742610, -0.2121144, 1.5707288].map(|v| bits(v as f32));
    assert_eq!(got, [0xbc99_6e30, 0x3d98_1627, 0xbe59_3484, 0x3fc9_0da4]);
    assert_eq!(bits(core::f32::consts::FRAC_PI_2), 0x3fc9_0fdb);
}

/* ---- section 3.2: the non-libm folds that fix the rules ---- */

#[test]
fn rule_mod_is_unfused() {
    let (x, y) = (87.65f32, 3.7f32);
    let q = fmath::floor(x / y);
    let unfused = x - y * q;
    assert_eq!(bits(unfused), 0x4023_3340, "ce_mod_vec2_float_fs: x - y * floor(x / y)");
    assert_eq!(bits(fma32(-y, q, x)), 0x4023_3335, "the fused value");
    assert_eq!(f32_to_f16(unfused), 0x411a, "constprod_fma_foldcall_mod_vs");
    assert_eq!(f32_to_f16(fma32(-y, q, x)), 0x411a, "the f16 cannot tell");
}

/// One f16 NIR op: widened operands, the f32 expression, one RTNE.
fn h(v: f32) -> f32 {
    f16_to_f32(f32_to_f16(v))
}

/// mediump `a / b` after `lower_fdiv`: `fmul(a, frcp(b))`, each rounded.
fn div16(a: f32, b: f32) -> f32 {
    h(h(a) * h(1.0 / h(b)))
}

#[test]
fn rule_rcp16_rounds_after_every_op() {
    assert_eq!(f32_to_f16(div16(0.6, -0.5)), 0xbccd);
    assert_eq!(f32_to_f16(div16(-1.0, -0.4)), 0x4100);
    let (d1, d2) = (div16(0.7, 0.7), div16(0.4, -0.8));
    assert_eq!((f32_to_f16(d1), f32_to_f16(d2)), (0x3bff, 0xb800));
    assert_eq!(f32_to_f16(h(d1 + d2)), 0x37fe, "cfold2_const_other_rcp16");
    let true_div = h(h(0.7) / h(0.7));
    assert_eq!(f32_to_f16(h(true_div + d2)), 0x3800, "an f16 divide would give 0x3800");
}

#[test]
fn rule_reflect_1_1() {
    let (i, n) = (1.0f32, 1.0f32);
    assert_eq!(i - 2.0 * (n * i * n), -1.0, "constprod_fma_foldcall_reflect_*");
}

/* ---- the two paths of exp and log (section 1.2) ---- */

#[test]
fn exp_and_log_glsl_and_nir_paths_differ() {
    assert_eq!(bits(fold_glsl(Libm::Exp, 5.5, 0.0)), 0x4374_b122, "expf(5.5)");
    assert_eq!(bits(fold_nir(Libm::Exp, 5.5, 0.0)), 0x4374_b121, "exp2f(5.5 * float(M_LOG2E))");
    assert_eq!(bits(fold_glsl(Libm::Log, 42.12, 0.0)), 0x406f_64b9, "logf(42.12)");
    let nir = fold_nir(Libm::Log, 42.12, 0.0);
    assert_eq!(bits(nir), bits(fmath::log2(42.12) * mesa_fold::LN2_F32), "log2f(x) * float(1 / M_LOG2E)");
    assert_eq!(bits(mesa_fold::LOG2E_F32), 0x3fb8_aa3b);
    assert_eq!(bits(mesa_fold::LN2_F32), 0x3f31_7218);
}

/// The f16 forms: the f32 function of the widened operand, one RTNE.
#[test]
fn f16_folds_round_the_f32_value_once() {
    let x = f32_to_f16(3.0);
    assert_eq!(fold_nir_f16(Libm::Sin, x, 0), 0x3084);
    assert_eq!(fold_nir_f16(Libm::Log2, f32_to_f16(100.0), 0), 0x46a5);
    assert_eq!(fold_nir_f16(Libm::Sqrt, f32_to_f16(0.25), 0), f32_to_f16(0.5));
    /* acos(-1) at f32 is pi; as f16 0x4248 */
    assert_eq!(fold_nir_f16(Libm::Acos, f32_to_f16(-1.0), 0), f32_to_f16(fold_nir(Libm::Acos, -1.0, 0.0)));
}

/// The f16 conversions: every f16 round-trips, ties go to even, the ends
/// saturate to infinity and zero, the subnormals round.
#[test]
fn f16_conversions_are_ieee() {
    for v in 0..=u16::MAX {
        let f = f16_to_f32(v);
        if v & 0x7c00 == 0x7c00 && v & 0x3ff != 0 {
            assert!(f.is_nan());
        } else {
            assert_eq!(f32_to_f16(f), v, "{v:#06x}");
        }
    }
    assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3c00, "a tie to even, down");
    assert_eq!(f32_to_f16(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02, "a tie to even, up");
    assert_eq!(f32_to_f16(65520.0), 0x7c00, "rounds to infinity");
    assert_eq!(f32_to_f16(65519.0), 0x7bff);
    assert_eq!(f32_to_f16(2f32.powi(-25)), 0x0000, "half the smallest subnormal: even");
    assert_eq!(f32_to_f16(3.0 * 2f32.powi(-25)), 0x0002, "a subnormal tie, up");
    assert_eq!(f32_to_f16(-2f32.powi(-24)), 0x8001);
    assert_eq!(f32_to_f16(2f32.powi(-14) - 2f32.powi(-26)), 0x0400, "the largest subnormal rounds up");
    assert_eq!(f32_to_f16(f32::NAN) & 0x7e00, 0x7e00);
}

/* ---- fma32 ---- */

struct Rng(u64);

impl Rng {
    /// xorshift64 (13, 7, 17), the note's generator (section 7).
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// A uniform f32 in [lo, hi).
    fn range(&mut self, lo: f64, hi: f64) -> f32 {
        let u = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        (lo + (hi - lo) * u) as f32
    }
}

fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[test]
fn fma32_is_mul_add() {
    let mut r = Rng(0x1234_5678_9abc_def0);
    for i in 0..1_000_000u32 {
        let (a, b, c) = if i % 2 == 0 {
            /* random bit patterns: subnormals, infinities, NaNs, cancellation */
            (f32::from_bits(r.next() as u32), f32::from_bits(r.next() as u32), f32::from_bits(r.next() as u32))
        } else {
            /* near cancellation: c close to -a*b */
            let (a, b) = (r.range(-4.0, 4.0), r.range(-4.0, 4.0));
            let c = -(a * b) + f32::from_bits((r.next() as u32) & 0x807f_ffff) * 1e-6;
            (a, b, c)
        };
        let (got, want) = (fma32(a, b, c), a.mul_add(b, c));
        assert!(same(got, want), "fma32({a:e}, {b:e}, {c:e}) = {got:e}, mul_add {want:e}");
    }
    /* exact cancellation is +0, the double-rounding trap rounds once */
    assert_eq!(bits(fma32(2.0, 3.0, -6.0)), 0);
    let (a, b, c) = (1.0 + 2f32.powi(-12), 1.0 + 2f32.powi(-12), -(1.0 + 2f32.powi(-11)));
    assert!(same(fma32(a, b, c), a.mul_add(b, c)));
    let (a, b, c) = (1.0 + 2f32.powi(-23), 1.0 - 2f32.powi(-23), 2f32.powi(-70));
    assert!(same(fma32(a, b, c), a.mul_add(b, c)), "a product just below 1 and a tiny addend");
}

/* ---- section 3.3: where the helper differs from the host's libm ---- */

const N: usize = 2_000_000;

impl Rng {
    /// The note's sampling (section 7's scratch program; with one generator
    /// through the rows this reproduces its sin, cos, exp2 and log2 counts
    /// exactly): 24 random bits as a fraction, scaled in f32.
    fn sample(&mut self, lo: f64, hi: f64) -> f32 {
        let u = (self.next() >> 40) as f32 / 16_777_216.0;
        lo as f32 + (hi - lo) as f32 * u
    }
}

/// The count of N arguments in `[lo, hi)` where `ours` and `host` differ.
fn count1(r: &mut Rng, (lo, hi): (f64, f64), ours: impl Fn(f32) -> f32, host: impl Fn(f32) -> f32) -> usize {
    (0..N)
        .filter(|_| {
            let x = std::hint::black_box(r.sample(lo, hi));
            !same(ours(x), host(x))
        })
        .count()
}

/// The same for two operands, `x` drawn before `y`.
fn count2(
    r: &mut Rng,
    (xl, xh): (f64, f64),
    (yl, yh): (f64, f64),
    ours: impl Fn(f32, f32) -> f32,
    host: impl Fn(f32, f32) -> f32,
) -> usize {
    (0..N)
        .filter(|_| {
            let x = std::hint::black_box(r.sample(xl, xh));
            let y = std::hint::black_box(r.sample(yl, yh));
            !same(ours(x, y), host(x, y))
        })
        .count()
}

fn bounded(name: &str, n: usize, bound: usize) {
    eprintln!("{name}: {n} of {N} differ from std (bound {bound})");
    assert!(n <= bound, "{name}: {n} of {N} differ from the host libm, more than the bound {bound}");
}

/// One generator through the rows in the table's order. The first four
/// rows draw the note's arguments exactly. pow, exp and log came from another
/// scratch program, so their arguments differ and the note's counts bound a
/// rate. The `atan` rows are 0 on the note's host.
#[test]
fn counts_against_the_host_libm_stay_within_section_3_3() {
    let g = |f| move |x| fold_glsl(f, x, 0.0);
    let mut r = Rng(0x1234_5678_9abc_def0);
    bounded("sin", count1(&mut r, (-8.0, 8.0), g(Libm::Sin), f32::sin), 25_493);
    bounded("cos", count1(&mut r, (-8.0, 8.0), g(Libm::Cos), f32::cos), 25_810);
    bounded("exp2", count1(&mut r, (-10.0, 10.0), g(Libm::Exp2), f32::exp2), 1_302);
    bounded("log2", count1(&mut r, (0.001, 1000.0), g(Libm::Log2), f32::log2), 437);
    let pow = |x, y| fold_glsl(Libm::Pow, x, y);
    bounded("pow", count2(&mut r, (0.01, 20.0), (-8.0, 8.0), pow, f32::powf), 1_282);
    bounded("exp", count1(&mut r, (-8.0, 8.0), g(Libm::Exp), f32::exp), 1_296);
    bounded("log", count1(&mut r, (0.01, 100.0), g(Libm::Log), f32::ln), 3_480);
    bounded("atan", count1(&mut r, (-8.0, 8.0), g(Libm::Atan), f32::atan), 0);
    bounded("asin", count1(&mut r, (-1.0, 1.0), g(Libm::Asin), f32::asin), 0);
    bounded("acos", count1(&mut r, (-1.0, 1.0), g(Libm::Acos), f32::acos), 0);
    let atan2 = |y, x| fold_glsl(Libm::Atan2, y, x);
    bounded("atan2", count2(&mut r, (-8.0, 8.0), (-1.0, 1.0), atan2, f32::atan2), 0);
    /* tan is sin / cos in f32: it can differ only where sin or cos does */
    let (mut tan, mut either) = (0, 0);
    for _ in 0..N {
        let x = std::hint::black_box(r.sample(-8.0, 8.0));
        let sc = same(fold_glsl(Libm::Sin, x, 0.0), x.sin()) && same(fold_glsl(Libm::Cos, x, 0.0), x.cos());
        either += usize::from(!sc);
        tan += usize::from(!same(fold_glsl(Libm::Tan, x, 0.0), x.sin() / x.cos()));
    }
    bounded("tan", tan, either);
}

/// The special values the C functions define, against `std`.
#[test]
fn special_values_are_the_c_functions() {
    let vals = [0.0f32, -0.0, 1.0, -1.0, 2.0, -2.0, 0.5, -0.5, 3.0, -3.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    for &x in &vals {
        assert!(same(fold_glsl(Libm::Exp, x, 0.0), x.exp()), "exp({x})");
        assert!(same(fold_glsl(Libm::Log, x, 0.0), x.ln()) || x.ln().is_nan(), "log({x})");
        assert!(same(fold_glsl(Libm::Atan, x, 0.0), x.atan()), "atan({x})");
        if x.abs() <= 1.0 || x.is_nan() {
            assert!(same(fold_glsl(Libm::Asin, x, 0.0), x.asin()), "asin({x})");
            assert!(same(fold_glsl(Libm::Acos, x, 0.0), x.acos()), "acos({x})");
        }
        for &y in &vals {
            assert!(
                same(fold_glsl(Libm::Pow, x, y), x.powf(y)),
                "pow({x}, {y}): {} vs {}",
                fold_glsl(Libm::Pow, x, y),
                x.powf(y)
            );
            assert!(same(fold_glsl(Libm::Atan2, x, y), x.atan2(y)), "atan2({x}, {y})");
        }
    }
    assert_eq!(fold_glsl(Libm::Rsq, 0.0, 0.0), f32::INFINITY, "inversesqrt(0.0)");
    assert_eq!(bits(fold_glsl(Libm::Rsq, 2.0, 0.0)), bits(1.0 / 2f32.sqrt()));
}
