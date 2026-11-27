// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): glmark2's shadow compare holds within the error bound of
//! verdict code 3 of gl_test case 222 `dtpu` (docs/notes/gc-depth-sample.md).
//!
//! Code 3 means the texture unit's D24X8 sample of a depth surface differs
//! from the exact `k / (2^24 - 1)` by at most 255 units of `k`, the model
//! being a 16-bit cut: the low 8 bits of the 24-bit depth are dropped. The
//! test is pure arithmetic on a synthetic shadow map shaped like glmark2's
//! (a ground plane plus a model, a cleared border), so it holds whatever the
//! board's verdict is; it needs no GPU, only the feature set of the other
//! gc_depth tests. It pins:
//!
//! 1. the cut stays within 255 / M24 of the exact value for every 24-bit k;
//! 2. a cleared texel (0xFFFF00 under the cut) stays above glmark2's largest
//!    biased ground z, 0.7215, so it stays lit;
//! 3. the X2 ramp: the cut keeps n * 2^17 and moves n * 2^17 - 1 by exactly
//!    255 (indices 0 and 1 are n = 64 and 65);
//! 4. the compare `d < z + 0.1505` gives the same answer for the exact and
//!    the cut sample whenever |d - (z + 0.1505)| > 255 / M24, and no flip
//!    happens on the synthetic map for z samples with that margin.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_view_precision
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

const M24: u32 = 0xFF_FFFF;
const BOUND: f64 = 255.0 / M24 as f64;
const BIAS: f64 = 0.1505;
/// glmark2's largest biased ground z (shadow.frag, near/far 2 and 50).
const GROUND_Z_MAX: f64 = 0.7215;

fn exact(k: u32) -> f64 {
    k as f64 / M24 as f64
}

/// The sample under code 3: the low 8 bits of the 24-bit depth are lost.
fn cut(k: u32) -> f64 {
    (k & !0xff) as f64 / M24 as f64
}

#[test]
fn cut_stays_within_255_units_for_every_depth() {
    for k in 0..=M24 {
        let lost = k - (k & !0xff);
        assert!(lost <= 255, "k={k:#x}");
        assert!((exact(k) - cut(k)).abs() <= BOUND + 1e-15, "k={k:#x}");
        // the rounded 16-bit variant (a / 65535) is inside the bound too
        let r = (k >> 8) as f64 / 65535.0;
        assert!((exact(k) - r).abs() <= BOUND + 1e-15, "k={k:#x} 16-bit");
    }
}

#[test]
fn cleared_texel_stays_lit() {
    let c = cut(0xFF_FFFF);
    assert_eq!(0xFF_FF00, 0xFF_FFFFu32 & !0xff);
    assert!((c - 0.9999848).abs() < 1e-6, "{c}");
    assert!(c > GROUND_Z_MAX, "cleared texel {c} must exceed ground z");
    assert!(c - GROUND_Z_MAX > 0.27, "margin {}", c - GROUND_Z_MAX);
}

#[test]
fn x2_ramp_keeps_n_shl_17_and_moves_the_one_below_by_255() {
    for n in [64u32, 65] {
        let hi = n << 17;
        assert_eq!(hi & !0xff, hi, "n={n} kept");
        let lo = hi - 1;
        assert_eq!(lo - (lo & !0xff), 255, "n={n} moved by 255");
    }
}

/// A shadow map like the pass makes: a ground plane at depth 0.5..0.7 (a ramp
/// over the map), a model bump nearer the light, the rest cleared (2^24 - 1).
fn shadow_map(w: usize, h: usize) -> Vec<u32> {
    let mut m = vec![M24; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f64 / w as f64, y as f64 / h as f64);
            let inside = (fx - 0.5).abs() < 0.45 && (fy - 0.5).abs() < 0.45;
            if !inside {
                continue;
            }
            let ground = 0.5 + 0.2215 * fy * fx;
            let r2 = (fx - 0.5).powi(2) + (fy - 0.5).powi(2);
            let d = if r2 < 0.04 { ground - 0.3 * (1.0 - r2 / 0.04) } else { ground };
            m[y * w + x] = (d * M24 as f64).round() as u32;
        }
    }
    m
}

#[test]
fn shadow_compare_unchanged_outside_the_bound() {
    let (w, h) = (160, 120);
    let map = shadow_map(w, h);
    // z samples across the ground's biased range plus values near the map's.
    let zs: Vec<f64> = (0..=72).map(|i| i as f64 / 100.0).collect();
    let (mut compared, mut within, mut flips) = (0usize, 0usize, 0usize);
    for &k in &map {
        let (e, c) = (exact(k), cut(k));
        for &z in &zs {
            let t = z + BIAS;
            let (le, lc) = (e < t, c < t);
            compared += 1;
            if (e - t).abs() > BOUND {
                assert_eq!(le, lc, "k={k:#x} z={z}");
            } else {
                within += 1;
                if le != lc {
                    flips += 1;
                }
            }
        }
    }
    assert!(compared > 1_000_000);
    // Flips only happen on pairs inside the bound; report them.
    println!("{compared} pairs, {within} within the bound, {flips} flips");
    // The cleared texels stay lit (not below the threshold) at every ground z.
    assert!(map.contains(&M24));
    for &z in &zs {
        assert!(cut(M24) >= z + BIAS, "cleared texel shadowed at z={z}");
    }
}
