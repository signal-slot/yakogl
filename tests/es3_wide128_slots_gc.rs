// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the companion sampler slot of a 128-bit texture
//! (item wide128-slots-test).
//!
//! An RGBA32F/UI/I binding is sampled as two G32R32 planes; the second plane
//! takes a companion slot after its stage's own slots, in unit order among
//! the stage's 128-bit bindings (Mesa's unified packing,
//! etnaviv_texture.c:301-322, 434-438). `companion_slot(base, count, rank)`
//! is the allocation. A draw cannot reach it from an integration test (the
//! sampled mask that opens the 128-bit classes is a `cfg(test)` hook of the
//! crate), so this file drives the function itself, laid out as
//! `emit_textures_inner` calls it (src/vivante/texture.rs): the
//! fragment stage at base 0 with `fs_count` slots, the vertex stage at base
//! `fs_count + fs_wide` (= VS_SAMPLER_BASE). `stage_slots_fit` is the check
//! that refuses a draw whose vertex stage passes slot 31.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_slots_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use yakogl::vivante::state::GC_MAX_SAMPLERS;
use yakogl::vivante::texture::{companion_slot, stage_slots_fit, GC_HW_SAMPLER_SLOTS};

/// A stage's slots: per binding, `(own slot, companion slot)`.
type Slots = Vec<(u32, Option<u32>)>;

/// The layout of one draw from the units each stage binds, `(unit, is 128-bit)`.
/// Returns (VS_SAMPLER_BASE, fragment slots, vertex slots).
fn layout(fs: &[(u32, bool)], vs: &[(u32, bool)]) -> (u32, Slots, Slots) {
    let count = |b: &[(u32, bool)]| b.iter().map(|t| t.0 + 1).max().unwrap_or(0).max(b.len() as u32);
    let stage = |b: &[(u32, bool)], base: u32, count: u32| -> Slots {
        b.iter()
            .map(|&(unit, wide)| {
                let comp = wide.then(|| {
                    let rank = b.iter().filter(|o| o.1 && o.0 < unit).count() as u32;
                    companion_slot(base, count, rank)
                });
                (base + unit, comp.flatten())
            })
            .collect()
    };
    let fs_count = count(fs);
    let vs_base = fs_count + fs.iter().filter(|t| t.1).count() as u32;
    (vs_base, stage(fs, 0, fs_count), stage(vs, vs_base, count(vs)))
}

#[test]
fn the_function_is_pure_slot_arithmetic() {
    assert_eq!(GC_HW_SAMPLER_SLOTS, 32);
    assert_eq!(GC_MAX_SAMPLERS, 16);
    for base in [0, 3, 17] {
        for count in 0..6 {
            for rank in 0..6 {
                let want = base + count + rank;
                assert_eq!(companion_slot(base, count, rank), (want < 32).then_some(want));
            }
        }
    }
}

#[test]
fn n_wide_fragment_bindings_take_the_slots_right_after_the_units() {
    for n in 1..=GC_MAX_SAMPLERS as u32 {
        let fs: Vec<_> = (0..n).map(|u| (u, true)).collect();
        let (vs_base, f, _) = layout(&fs, &[]);
        assert_eq!(vs_base, 2 * n, "VS_SAMPLER_BASE past the companions");
        for (rank, (own, comp)) in f.iter().enumerate() {
            assert_eq!(*own, rank as u32);
            assert_eq!(*comp, Some(n + rank as u32), "{n} wide bindings, rank {rank}");
        }
    }
}

#[test]
fn n_wide_vertex_bindings_take_the_slots_after_the_vertex_units() {
    for n in 1..=GC_MAX_SAMPLERS as u32 {
        let vs: Vec<_> = (0..n).map(|u| (u, true)).collect();
        let (vs_base, _, v) = layout(&[(0, false)], &vs);
        assert_eq!(vs_base, 1, "no fragment companion");
        for (rank, (own, comp)) in v.iter().enumerate() {
            assert_eq!(*own, 1 + rank as u32);
            let want = 1 + n + rank as u32;
            assert_eq!(*comp, (want < GC_HW_SAMPLER_SLOTS).then_some(want), "{n} wide, rank {rank}");
        }
    }
}

#[test]
fn rank_is_the_position_among_the_wide_bindings_in_unit_order() {
    /* units 0, 2, 5 wide; 1 and 3 ordinary; the table given out of order */
    let fs = [(5, true), (1, false), (0, true), (3, false), (2, true)];
    let (vs_base, f, _) = layout(&fs, &[]);
    assert_eq!(vs_base, 6 + 3);
    let comp = |unit: u32| f.iter().find(|s| s.0 == unit).unwrap().1;
    assert_eq!([comp(0), comp(2), comp(5)], [Some(6), Some(7), Some(8)]);
    assert_eq!([comp(1), comp(3)], [None, None]);
}

#[test]
fn companions_never_collide_with_a_unit_a_companion_or_the_other_stage() {
    /* every pattern of wide units over 5 fragment and 4 vertex units */
    for fm in 0..32u32 {
        for vm in 0..16u32 {
            let fs: Vec<_> = (0..5).map(|u| (u, fm >> u & 1 != 0)).collect();
            let vs: Vec<_> = (0..4).map(|u| (u, vm >> u & 1 != 0)).collect();
            let (vs_base, f, v) = layout(&fs, &vs);
            let mut used = std::collections::BTreeSet::new();
            for (own, comp) in f.iter().chain(&v) {
                assert!(used.insert(*own), "slot {own} twice ({fm:#x}, {vm:#x})");
                if let Some(c) = comp {
                    assert!(used.insert(*c), "companion {c} overlaps ({fm:#x}, {vm:#x})");
                }
            }
            assert!(f.iter().all(|s| s.0 < vs_base && s.1.is_none_or(|c| c < vs_base)));
            assert!(v.iter().all(|s| s.0 >= vs_base));
            assert!(used.iter().all(|&s| s < GC_HW_SAMPLER_SLOTS));
        }
    }
}

/// Past the 32-slot array a companion is refused by name (`None`: the binding
/// samples the black texel, texture.rs:182-194), never wrapped onto another
/// slot.
#[test]
fn a_companion_past_the_hardware_slots_is_refused_not_overlapped() {
    /* 16 fragment units, all wide: companions 16..31, the array exactly full */
    let fs: Vec<_> = (0..16).map(|u| (u, true)).collect();
    let (vs_base, f, _) = layout(&fs, &[]);
    assert_eq!(vs_base, 32);
    assert!(f.iter().all(|s| s.1.is_some()));
    assert_eq!(f.last().unwrap().1, Some(31));

    /* after 16 fragment units, one of them wide, the vertex stage's four wide
     * bindings: own slots 17..20, companions 21..24, all in the array */
    let fs16: Vec<_> = (0..16).map(|u| (u, u == 0)).collect();
    let vs: Vec<_> = (0..4).map(|u| (u, true)).collect();
    let (vs_base, _, v) = layout(&fs16, &vs);
    assert_eq!(vs_base, 17);
    assert_eq!(v.iter().map(|s| s.1).collect::<Vec<_>>(), [Some(21), Some(22), Some(23), Some(24)]);

    /* ranks around the limit: 30, 31 fit, 32 and on are refused */
    for rank in 0..8 {
        let got = companion_slot(28, 2, rank);
        match 30 + rank {
            s if s < 32 => assert_eq!(got, Some(s)),
            _ => assert_eq!(got, None, "rank {rank} is refused"),
        }
    }
    assert_eq!(companion_slot(0, 16, 16), None);
}

/// The vertex stage's *own* slots start at `vs_base = fs_count + fs_wide`;
/// `emit_textures_inner` refuses the draw (-ENOTSUP, logged by name) when
/// `stage_slots_fit` says the last slot is past 31, before any slot is
/// written. 16 fragment units with one wide binding put vertex unit 15 at
/// slot 32: refused. One vertex unit fewer ends at slot 31: accepted.
#[test]
fn the_vertex_own_slot_overflow_is_refused() {
    let mut fs: Vec<_> = (0..16).map(|u| (u, false)).collect();
    fs[0].1 = true;
    let vs: Vec<_> = (0..16).map(|u| (u, false)).collect();
    let (vs_base, _, v) = layout(&fs, &vs);
    assert_eq!(vs_base, 17);
    assert_eq!(v.last().unwrap().0, 32, "one past the last slot of the array");
    assert_eq!(stage_slots_fit(16, 1, 16), Err(32));

    /* exactly at the limit: 15 vertex units, the last at slot 31 */
    let (_, _, v) = layout(&fs, &vs[..15]);
    assert_eq!(v.last().unwrap().0, 31);
    assert_eq!(stage_slots_fit(16, 1, 15), Ok(()));
}
