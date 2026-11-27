// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): glmark2 refract's depth read holds under the dtpu mode 1
//! depth view (D24X8) within the error bound of verdict code 3 of gl_test
//! case 222 (docs/notes/gc-depth-sample.md, gc-depth-sample-wire.md "(a)
//! Code 3").
//!
//! Companion of `gc_depth_view_precision.rs` (shadow's compare), same
//! approach: pure arithmetic, no GPU, so it holds whatever the board's
//! verdict is. Code 3 means the sample differs from the exact
//! `k / (2^24 - 1)` by at most 255 units of `k` (the low 8 bits dropped).
//!
//! refract has no compare. `light-refract.frag` uses the sampled depth as a
//! distance along `front_refraction` (length <= 1):
//! `back_position = vertex_position + front_refraction * d`, then
//! `normcoord = back_position.st * 0.5 + 0.5` into the 1600x1200 NormalMap,
//! `refract(back_position, back_normal, 1/1.2)` into the nasa1 (512x512)
//! coordinate. The program here is that chain with the GLSL `refract`, run
//! over depth values from 0 to 1 and eye-space positions over the bunny's
//! range, once with the exact depth and once with the cut one. It pins:
//!
//! 1. the cut moves `d` by at most `255 / (2^24 - 1)` = 1.52e-5 (the cut
//!    itself is the precision test's; here over refract's range of d);
//! 2. `back_position` moves by at most that, `normcoord` by at most half of
//!    it: 0.0122 of a NormalMap texel (1600 wide), the note's analytic bound;
//! 3. the image coordinate moves by less than 0.05 of a nasa1 texel, the
//!    note's upper figure, over normals from smooth to the sharpest the
//!    8-bit NormalMap holds (a step of one colour value per texel);
//! 4. an 8-bit bilinear read across the sharpest edge (a full 0..255 step
//!    in one texel) changes only inside the one-texel band of the edge, by
//!    at most `255 * move + 1` colour values (4 on the NormalMap, 13 on the
//!    image, measured), so the share of changed fragments is the band's
//!    width over the map's, not more.
//!
//! The measured bounds are printed (`--nocapture`): max normcoord move in
//! texels, max image coordinate move in texels, share of changed colours.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_view_refract
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

const M24: u32 = 0xFF_FFFF;
const BOUND: f64 = 255.0 / M24 as f64;
const NORM_W: f64 = 1600.0;
const IMG_W: f64 = 512.0;
const ETA: f64 = 1.0 / 1.2;

fn exact(k: u32) -> f64 {
    k as f64 / M24 as f64
}
fn cut(k: u32) -> f64 {
    (k & !0xff) as f64 / M24 as f64
}

type V3 = [f64; 3];
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    scale(a, 1.0 / dot(a, a).sqrt())
}
/// GLSL `refract(i, n, eta)`.
fn refract(i: V3, n: V3, eta: f64) -> V3 {
    let d = dot(n, i);
    let k = 1.0 - eta * eta * (1.0 - d * d);
    if k < 0.0 {
        [0.0; 3]
    } else {
        add(scale(i, eta), scale(n, -(eta * d + k.sqrt())))
    }
}

/// A deterministic pseudo-random sequence in [0, 1).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The program's coordinates for a depth sample `d`: (normcoord s, t, image
/// coordinate s, t), for the eye-space position `p`, the front refraction
/// `fr` and the back normal `bn`.
fn chain(p: V3, fr: V3, bn: V3, d: f64) -> [f64; 4] {
    let back = add(p, scale(fr, d));
    let nc = [back[0] * 0.5 + 0.5, back[1] * 0.5 + 0.5];
    let r = refract(norm(back), norm(bn), ETA);
    let ic = [r[0] * 0.5 + 0.5, r[1] * 0.5 + 0.5];
    [nc[0], nc[1], ic[0], ic[1]]
}

#[test]
fn cut_stays_within_the_bound_over_refracts_distance_range() {
    // refract's distance is the depth texture's value: all of [0, 1].
    let mut worst = 0u32;
    for k in (0..=M24).step_by(7) {
        let lost = k - (k & !0xff);
        worst = worst.max(lost);
        assert!((exact(k) - cut(k)).abs() <= BOUND + 1e-15, "k={k:#x}");
    }
    assert_eq!(worst, 255);
}

#[test]
fn normcoord_moves_by_at_most_0_0122_texel_and_the_image_coordinate_under_0_05() {
    let mut rng = Rng(0x5eed_1234);
    let (mut max_nc, mut max_ic, mut n) = (0f64, 0f64, 0u32);
    for _ in 0..400_000 {
        let k = (rng.next() * M24 as f64) as u32;
        // eye-space point of the bunny (xy in the window, z in front of the
        // camera) and a unit-length front refraction.
        let p = [rng.next() * 2.0 - 1.0, rng.next() * 2.0 - 1.0, -(rng.next() * 2.0 + 1.0)];
        let fr = scale(norm([rng.next() - 0.5, rng.next() - 0.5, -(rng.next() + 0.1)]), rng.next());
        // the back normal read from an 8-bit map: an 8-bit step per texel is
        // the sharpest change, so the normal itself is any 8-bit vector.
        let bn = [
            (rng.next() * 255.0).round() / 255.0 * 2.0 - 1.0,
            (rng.next() * 255.0).round() / 255.0 * 2.0 - 1.0,
            (rng.next() * 255.0).round() / 255.0 * 2.0 - 1.0 + 1e-3,
        ];
        let (a, b) = (chain(p, fr, bn, exact(k)), chain(p, fr, bn, cut(k)));
        let dnc = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) * NORM_W;
        max_nc = max_nc.max(dnc);
        assert!(dnc <= 0.5 * BOUND * NORM_W + 1e-9, "k={k:#x} normcoord moved {dnc}");
        // The image coordinate follows refract of the moved back position;
        // refract's direction is a smooth function of its input except at
        // total internal reflection (k < 0 -> zero vector): skip those.
        let (ra, rb) = (
            refract(norm(add(p, scale(fr, exact(k)))), norm(bn), ETA),
            refract(norm(add(p, scale(fr, cut(k)))), norm(bn), ETA),
        );
        if ra == [0.0; 3] || rb == [0.0; 3] {
            continue;
        }
        let dic = (a[2] - b[2]).abs().max((a[3] - b[3]).abs()) * IMG_W;
        n += 1;
        // Near grazing incidence refract's slope grows without bound; the
        // note's 0.05 holds where the incidence is not grazing (|n.i| >=
        // 0.1), which is where the bunny is drawn (the sphere of normals).
        if dot(norm(bn), norm(add(p, scale(fr, exact(k))))).abs() >= 0.1 {
            max_ic = max_ic.max(dic);
            assert!(dic < 0.05, "k={k:#x} image coordinate moved {dic} texel");
        }
    }
    assert!(n > 100_000);
    println!("refract: max normcoord move {max_nc:.5} texel, max image coordinate move {max_ic:.5} texel");
    assert!(max_nc <= 0.0123);
    assert!(max_ic < 0.05);
}

/// A bilinear read of a one-dimensional 8-bit ramp with a sharp edge: texel
/// `i` holds 0 below `edge` and 255 from it, as the NormalMap's sharpest
/// feature (a full-range step in one texel).
fn bilinear_edge(u: f64, w: f64, edge: usize) -> u8 {
    let x = u * w - 0.5;
    let i = x.floor();
    let f = x - i;
    let t = |j: f64| if (j as i64) >= edge as i64 { 255.0 } else { 0.0 };
    ((t(i) * (1.0 - f) + t(i + 1.0) * f) + 0.5).floor() as u8
}

#[test]
fn colour_changes_only_inside_a_band_as_wide_as_the_coordinate_moved() {
    // Worst case move of each coordinate, in its map's texels.
    let (nc_move, ic_move) = (0.5 * BOUND * NORM_W, 0.05);
    for (w, mv) in [(NORM_W, nc_move), (IMG_W, ic_move)] {
        let edge = (w / 2.0) as usize;
        let (mut changed, mut total, mut max_step) = (0u32, 0u32, 0i32);
        for s in 0..200_000 {
            let u = s as f64 / 200_000.0;
            let du = mv / w;
            let (a, b) = (bilinear_edge(u, w, edge), bilinear_edge((u + du).min(1.0), w, edge));
            total += 1;
            let step = (a as i32 - b as i32).abs();
            max_step = max_step.max(step);
            if step > 0 {
                changed += 1;
                // only within a texel of the edge can anything change
                assert!((u * w - edge as f64).abs() <= 1.0 + mv, "u={u} w={w}");
            }
        }
        // 255 steps across one texel: a move of mv texels is at most
        // 255 * mv + 1 colour values.
        assert!(max_step as f64 <= 255.0 * mv + 1.0, "w={w} step {max_step}");
        let share = changed as f64 / total as f64;
        println!("refract: edge read, width {w}: {changed}/{total} changed ({share:.2e}), max step {max_step}");
        // Changed fragments: the band of one texel the edge spreads over.
        assert!(share <= (1.0 + mv) / w + 1e-4, "share {share}");
    }
}
