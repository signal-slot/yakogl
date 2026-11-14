// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `pvr::mrt_layout::layout` against an independent port of Mesa 26.2.1's
//! render-target output allocator (`pvr_mrt_alloc_from_buffer` and
//! `pvr_alloc_mrt`, `src/imagination/vulkan/pvr_arch_mrt.c`) for the
//! BXS-4-64 (AM62P): 4 output registers per pixel, first fit, and an even
//! start for a 2- or 4-register pixel because the core has `pbe2_in_xe`
//! (`device_info/bxs-4-64.h`). The mixed integer / float targets design
//! (docs/notes/pvr-int-mixed-targets.md, sections 1 and 3) relies on
//! yakogl's placement being Mesa's wherever a mix fits in the output
//! registers, so a PBE source position computed here is the one Mesa's
//! driver would give the same render.
//!
//! Every list of 1 to 3 targets whose dword counts are in {1, 2, 4} runs
//! through both. Where both place every target in the output registers the
//! starts must be equal. The lists where the two differ are pinned in
//! `differences()` with each side's outcome: yakogl never back-fills a
//! register hole left by the even alignment (`layout` walks the targets in
//! emit order and spills the first that does not fit and all after it), Mesa
//! fills the hole with a later 1-register target and keeps placing after a
//! target that did not fit.
//!
//! The port is written from the allocator's rule as the design note quotes
//! it (`:57-80`, `:116-137`); the vendored tree under `third_party/mesa-26.2.1`
//! does not carry `pvr_arch_mrt.c`, so the port was not diffed against the
//! file.
//!
//! HOST-ONLY: pure arithmetic, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mrt_layout_mesa
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{layout, Place, OUTPUT_REGS};

/// Where each target lands: `Some(first register)` or `None` (not in the
/// output registers: a tile buffer).
type Starts = Vec<Option<u8>>;

/// Mesa's `pvr_mrt_alloc_from_buffer` over a buffer of `OUTPUT_REGS`
/// registers: the first start, stepping by the alignment, whose `dwords`
/// registers are all free. `None` when none is (Mesa then uses a tile
/// buffer).
fn mesa_alloc_from_buffer(used: &mut u8, dwords: u8, pbe2_in_xe: bool) -> Option<u8> {
    let align = if pbe2_in_xe && (dwords == 2 || dwords == 4) { 2 } else { 1 };
    let want = (1u8 << dwords) - 1;
    let mut start = 0;
    while start + dwords <= OUTPUT_REGS {
        if *used & (want << start) == 0 {
            *used |= want << start;
            return Some(start);
        }
        start += align;
    }
    None
}

/// Mesa's `pvr_alloc_mrt` over a list of targets: each allocated in turn
/// from the registers left, whatever an earlier one did.
fn mesa_layout(dwords: &[u8]) -> Starts {
    let mut used = 0u8;
    dwords.iter().map(|&d| mesa_alloc_from_buffer(&mut used, d, true)).collect()
}

/// `layout`'s outcome; `Err` when it refuses the list (no end-of-tile
/// program was captured for it).
fn yakogl_layout(dwords: &[u8]) -> Result<Starts, ()> {
    let l = layout(dwords).map_err(|_| ())?;
    Ok(l.place[..l.targets as usize]
        .iter()
        .map(|p| match *p {
            Place::Register { first } => Some(first),
            Place::TileBuffer { .. } => None,
        })
        .collect())
}

/// Every list of 1 to 3 dword counts from {1, 2, 4}.
fn all_lists() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for n in 1..=3u32 {
        for k in 0..3usize.pow(n) {
            out.push((0..n).map(|i| [1u8, 2, 4][(k / 3usize.pow(i)) % 3]).collect());
        }
    }
    out
}

type Diff = (Vec<u8>, Result<Starts, ()>, Starts);

const fn s(v: u8) -> Option<u8> {
    Some(v)
}

/// The lists where the two differ: (dwords, yakogl, Mesa). `None` is a
/// target outside the output registers (a tile buffer); `Err` a list
/// `layout` refuses (`NoProgram`: no end-of-tile program was captured for it).
///
/// Two causes. (a) `[1, 2, 1]`: the even start of the 2-register target
/// leaves register 1 free; Mesa gives it to the third target, `layout`
/// continues at register 4, finds no room and spills it (a three-target
/// program with target 2 spilled exists, `eot3_tb`). (b) Every list with a
/// 4-register target after another target (and `[4, ..]` followed by
/// anything): Mesa spills that target to a tile buffer and still places
/// later ones that fit in the free registers; `layout` spills it and every
/// later target, which leaves target 1 spilled in a two- or three-target
/// list, a layout no program was captured for, so it refuses.
fn differences() -> Vec<Diff> {
    let e = || Err(());
    vec![
        (vec![4, 1], e(), vec![s(0), None]),
        (vec![4, 2], e(), vec![s(0), None]),
        (vec![1, 4], e(), vec![s(0), None]),
        (vec![2, 4], e(), vec![s(0), None]),
        (vec![4, 4], e(), vec![s(0), None]),
        (vec![4, 1, 1], e(), vec![s(0), None, None]),
        (vec![1, 2, 1], Ok(vec![s(0), s(2), None]), vec![s(0), s(2), s(1)]),
        (vec![4, 2, 1], e(), vec![s(0), None, None]),
        (vec![1, 4, 1], e(), vec![s(0), None, s(1)]),
        (vec![2, 4, 1], e(), vec![s(0), None, s(2)]),
        (vec![4, 4, 1], e(), vec![s(0), None, None]),
        (vec![4, 1, 2], e(), vec![s(0), None, None]),
        (vec![4, 2, 2], e(), vec![s(0), None, None]),
        (vec![1, 4, 2], e(), vec![s(0), None, s(2)]),
        (vec![2, 4, 2], e(), vec![s(0), None, s(2)]),
        (vec![4, 4, 2], e(), vec![s(0), None, None]),
        (vec![4, 1, 4], e(), vec![s(0), None, None]),
        (vec![4, 2, 4], e(), vec![s(0), None, None]),
        (vec![1, 4, 4], e(), vec![s(0), None, None]),
        (vec![2, 4, 4], e(), vec![s(0), None, None]),
        (vec![4, 4, 4], e(), vec![s(0), None, None]),
    ]
}

#[test]
fn starts_agree_where_both_fit_in_registers() {
    for d in all_lists() {
        let (Ok(y), m) = (yakogl_layout(&d), mesa_layout(&d)) else { continue };
        if y.iter().chain(m.iter()).all(|p| p.is_some()) {
            assert_eq!(y, m, "{d:?}");
        }
    }
}

#[test]
fn differing_lists_are_exactly_the_pinned_table() {
    let mut found = Vec::new();
    for d in all_lists() {
        let (y, m) = (yakogl_layout(&d), mesa_layout(&d));
        if y.as_ref() != Ok(&m) {
            found.push((d, y, m));
        }
    }
    assert_eq!(found, differences());
}

#[test]
fn plan_mixes_place_the_same_in_both() {
    // RGBA8 + R32UI, R32UI + R32I, RGBA8 + R32I + RGBA16F, RGBA8 + RG32UI.
    for (d, want) in [
        (vec![1u8, 1], vec![0u8, 1]),
        (vec![1, 1], vec![0, 1]),
        (vec![1, 1, 2], vec![0, 1, 2]),
        (vec![1, 2], vec![0, 2]),
    ] {
        let want: Starts = want.into_iter().map(Some).collect();
        assert_eq!(yakogl_layout(&d), Ok(want.clone()), "yakogl {d:?}");
        assert_eq!(mesa_layout(&d), want, "mesa {d:?}");
    }
}
