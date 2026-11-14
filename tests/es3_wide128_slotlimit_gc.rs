// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): companion sampler slots of 128-bit bindings near the
//! 32-slot limit (item gc-w128-slotlimit).
//!
//! The 128-bit bits of the sampled mask (`backend::TEST_EXTRA_SAMPLED`) are a
//! `cfg(test)` hook of the crate, so an integration test cannot draw with
//! them. What it can reach is the slot arithmetic a draw uses:
//! `gl::draw::wide128_companions` (the GL core's companion facts of one
//! draw, fed the predicate `wide128_sampled` is in a draw -- here "the format
//! is a 128-bit class", i.e. the mask with the bits ORed in),
//! `companion_slot` and `stage_slots_fit` (what `emit_textures_inner` calls).
//! This file pins today's answer for a fragment stage of 15 RGBA8 units plus
//! one RGBA32F, of 8 RGBA32F units and of 16, each with and without
//! vertex-stage bindings: a companion on a distinct free slot, or none
//! (sampled black, by name), and the draw refused when the vertex stage's own
//! slots pass slot 31. A draw through the GC back end itself is an in-crate
//! test (src/vivante/backend_tests.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_slotlimit_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::collections::BTreeSet;
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA8};
use yakogl::gl::draw::wide128_companions;
use yakogl::shaderc::ShcShader;
use yakogl::vivante::texture::{companion_slot, is_wide128, stage_slots_fit, GC_HW_SAMPLER_SLOTS};

fn bind(unit: u8, format: u8) -> GlesTextureBind {
    GlesTextureBind { unit, format, ..GlesTextureBind::ZERO }
}

/// Units `0..n`, the ones `wide` accepts RGBA32F, the others RGBA8.
fn units(n: u8, wide: impl Fn(u8) -> bool) -> Vec<GlesTextureBind> {
    (0..n).map(|u| bind(u, if wide(u) { GLES_TEXFMT_RGBA32F } else { GLES_TEXFMT_RGBA8 })).collect()
}

/// The companion facts of a draw as absolute slots, in the order of the
/// bindings given: (vs_base, fragment companions, vertex companions).
fn slots(fs: &[GlesTextureBind], vs: &[GlesTextureBind]) -> (u32, Vec<Option<u32>>, Vec<Option<u32>>) {
    let none = ShcShader::ZERO;
    let c = wide128_companions(vs, &none, fs, &none, |t| is_wide128(t.format));
    let count = |b: &[GlesTextureBind]| b.iter().map(|t| t.unit as u32 + 1).max().unwrap_or(0);
    let fs_wide = fs.iter().filter(|t| is_wide128(t.format)).count() as u32;
    let vs_base = count(fs) + fs_wide;
    let abs = |b: &[GlesTextureBind], stage: usize, base: u32| -> Vec<Option<u32>> {
        b.iter().map(|t| c[stage][t.unit as usize].map(|r| base + r as u32)).collect()
    };
    (vs_base, abs(fs, 1, 0), abs(vs, 0, vs_base))
}

/// Every own slot and companion of both stages: all distinct, all in the array.
fn assert_distinct(fs: &[GlesTextureBind], vs: &[GlesTextureBind]) {
    let (vs_base, f, v) = slots(fs, vs);
    let mut used = BTreeSet::new();
    for t in fs {
        assert!(used.insert(t.unit as u32), "fragment unit {} twice", t.unit);
    }
    for t in vs {
        assert!(used.insert(vs_base + t.unit as u32), "vertex unit {} collides", t.unit);
    }
    for c in f.iter().chain(&v).flatten() {
        assert!(used.insert(*c), "companion {c} collides with a unit or another companion");
    }
    assert!(used.iter().all(|&s| s < GC_HW_SAMPLER_SLOTS), "{used:?}");
    assert!(f.iter().flatten().all(|&c| c < vs_base), "fragment companions precede the vertex stage");
}

#[test]
fn fifteen_rgba8_and_one_rgba32f_take_slot_16() {
    let fs = units(16, |u| u == 15);
    let (vs_base, f, _) = slots(&fs, &[]);
    assert_eq!(vs_base, 17);
    assert_eq!(f[15], Some(16));
    assert!(f[..15].iter().all(|c| c.is_none()));
    assert_distinct(&fs, &[]);
    assert_eq!(stage_slots_fit(16, 1, 0), Ok(()));
    /* the wide unit first: the companion is still after the 16 slots */
    let fs = units(16, |u| u == 0);
    assert_eq!(slots(&fs, &[]).1[0], Some(16));
}

#[test]
fn eight_rgba32f_units_take_slots_8_to_15() {
    let fs = units(8, |_| true);
    let (vs_base, f, _) = slots(&fs, &[]);
    assert_eq!(vs_base, 16);
    assert_eq!(f, (8..16).map(Some).collect::<Vec<_>>());
    assert_distinct(&fs, &[]);
    /* eight RGBA32F then eight RGBA8: the stage is 16 slots, companions 16..23 */
    let fs = units(16, |u| u < 8);
    let (vs_base, f, _) = slots(&fs, &[]);
    assert_eq!(vs_base, 24);
    assert_eq!(&f[..8], &(16..24).map(Some).collect::<Vec<_>>()[..]);
    assert_distinct(&fs, &[]);
}

#[test]
fn sixteen_rgba32f_units_fill_the_array_exactly() {
    let fs = units(16, |_| true);
    let (vs_base, f, _) = slots(&fs, &[]);
    assert_eq!(vs_base, 32);
    assert_eq!(f, (16..32).map(Some).collect::<Vec<_>>());
    assert_distinct(&fs, &[]);
    assert_eq!(stage_slots_fit(16, 16, 0), Ok(()));
}

/// With vertex-stage textures in the same draw: the vertex stage starts after
/// the fragment companions; a companion that would pass slot 31 is none (the
/// binding samples black, by name); the vertex stage's own slot past 31 is a
/// draw refused by `stage_slots_fit`.
#[test]
fn the_vertex_stage_sits_after_the_fragment_companions() {
    /* 8 wide fragment, 8 wide vertex: 0..7, 8..15, vertex 16..23, 24..31 */
    let (fs, vs) = (units(8, |_| true), units(8, |_| true));
    let (vs_base, f, v) = slots(&fs, &vs);
    assert_eq!(vs_base, 16);
    assert_eq!(f, (8..16).map(Some).collect::<Vec<_>>());
    assert_eq!(v, (24..32).map(Some).collect::<Vec<_>>());
    assert_distinct(&fs, &vs);
    assert_eq!(stage_slots_fit(8, 8, 8), Ok(()));

    /* one more vertex unit: own slots 16..24, companions 25 + rank, so ranks
     * 0..6 fit (slots 25..31) and ranks 7 and 8 get none */
    let vs = units(9, |_| true);
    let (_, _, v) = slots(&fs, &vs);
    let want: Vec<_> = (0..9u32).map(|r| (25 + r < 32).then_some(25 + r)).collect();
    assert_eq!(v, want);
    assert_eq!(v.iter().filter(|c| c.is_none()).count(), 2);
    assert_distinct(&fs, &vs);
    assert_eq!(stage_slots_fit(8, 8, 9), Ok(()), "the own slots fit; the companions are refused one by one");

    /* 16 wide fragment: vs_base 32; the vertex stage's wide binding has no
     * companion, and its own slot 32 is past the array: the draw is refused */
    let fs = units(16, |_| true);
    let vs = units(1, |_| true);
    let (vs_base, f, v) = slots(&fs, &vs);
    assert_eq!(vs_base, 32);
    assert_eq!(f, (16..32).map(Some).collect::<Vec<_>>());
    assert_eq!(v, [None]);
    assert_eq!(stage_slots_fit(16, 16, 1), Err(32));

    /* 15 RGBA8 + 1 RGBA32F fragment: vertex own slots 17..31 fit for 15 units,
     * 16 are refused; a wide vertex binding there has no companion */
    let fs = units(16, |u| u == 15);
    assert_eq!(stage_slots_fit(16, 1, 15), Ok(()));
    assert_eq!(stage_slots_fit(16, 1, 16), Err(32));
    let vs = units(15, |u| u == 3);
    let (vs_base, _, v) = slots(&fs, &vs);
    assert_eq!(vs_base, 17);
    assert_eq!(v[3], None);
    assert_eq!(companion_slot(17, 15, 0), None);
    assert_eq!(companion_slot(17, 14, 0), Some(31));
    assert_distinct(&fs, &units(15, |_| false));
}

/// No pattern of wide units over 6 fragment and 5 vertex units puts two
/// bindings on one slot; sparse unit numbers rank in unit order.
#[test]
fn no_pattern_makes_two_bindings_share_a_slot() {
    for fm in 0..64u32 {
        for vm in 0..32u32 {
            let fs = units(6, |u| fm >> u & 1 != 0);
            let vs = units(5, |u| vm >> u & 1 != 0);
            assert_distinct(&fs, &vs);
        }
    }
    /* units 12 and 2: 13 slots, companions 13 (unit 2) and 14 (unit 12);
     * the vertex stage at base 15 */
    let fs = vec![bind(12, GLES_TEXFMT_RGBA32F), bind(2, GLES_TEXFMT_RGBA32F)];
    let vs = vec![bind(0, GLES_TEXFMT_RGBA32F)];
    let (vs_base, f, v) = slots(&fs, &vs);
    assert_eq!(vs_base, 15);
    assert_eq!(f, [Some(14), Some(13)]);
    assert_eq!(v, [Some(16)]);
}
