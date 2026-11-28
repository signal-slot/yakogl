// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `pvr::mrt_layout::layout` against a first-fit reference allocator written
//! from Mesa 26.2.1's rules for the PowerVR render-target output registers
//! (`pvr_arch_mrt.c`, `pvr_arch_hw_pass.c`; the rule as
//! docs/notes/pvr-int-mixed-targets.md section 1 quotes it, the vendored
//! tree does not carry the files, so the reference was not diffed against
//! them):
//!
//! - each target is placed on its own, in emit order, first fit over a
//!   bitset of the four 32-bit output registers; a 2- or 4-register target
//!   starts on an even register (`pbe2_in_xe`);
//! - a target that finds no room is spilled to a tile buffer and later
//!   targets are still tried against the registers (back-fill);
//! - spilled targets share tile buffers first fit (a buffer holds
//!   `TB_DWORDS` dwords), up to 7 buffers (`PVR_MAX_TILE_BUFFER_COUNT`).
//!
//! `layout` places in order and, from the first target that misses the
//! registers on, spills every target to its own tile buffer. Defect D1
//! (plan #368, mesa-rt4-pvr): 72 of the 120 width lists over {1, 2, 4} with
//! 1-4 targets are placed differently, 20 have no end-of-tile program, e.g.
//! (4,1) and (1,4,1); widths (1,2,1) are registers 0, 2, 1 in Mesa and
//! `layout` spills the third.
//!
//! HOST-ONLY: pure arithmetic, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mrt_layout_mesa
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{layout, Place, OUTPUT_REGS};

/// Dwords one tile buffer holds for spilled targets (one 4-dword pixel slot; this gives the 48 agreeing lists of defect D1).
const TB_DWORDS: u8 = 4;
/// `PVR_MAX_TILE_BUFFER_COUNT`.
const MAX_TB: usize = 7;

type Placed = Vec<Place>;

/// The reference: first fit with back-fill and shared tile buffers.
fn reference(dwords: &[u8]) -> Placed {
    let mut used = 0u8;
    let mut tb = [0u8; MAX_TB];
    dwords
        .iter()
        .map(|&d| {
            let align = if d == 2 || d == 4 { 2 } else { 1 };
            let want = (1u8 << d) - 1;
            let mut s = 0;
            while s + d <= OUTPUT_REGS {
                if used & (want << s) == 0 {
                    used |= want << s;
                    return Place::Register { first: s };
                }
                s += align;
            }
            let i = tb.iter().position(|&u| u + d <= TB_DWORDS).expect("tile buffers exhausted");
            tb[i] += d;
            Place::TileBuffer { index: i as u8 }
        })
        .collect()
}

/// `layout`'s places, `None` when it has no program for the list.
fn actual(dwords: &[u8]) -> Option<Placed> {
    let l = layout(dwords).ok()?;
    Some(l.place[..l.targets as usize].to_vec())
}

/// Every list of 1 to 4 widths from {1, 2, 4}: 3 + 9 + 27 + 81 = 120.
fn all_lists() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for n in 1..=4u32 {
        for k in 0..3usize.pow(n) {
            out.push((0..n).map(|i| [1u8, 2, 4][(k / 3usize.pow(i)) % 3]).collect());
        }
    }
    out
}

fn all_in_registers(p: &Placed) -> bool {
    p.iter().all(|p| matches!(p, Place::Register { .. }))
}

/// The lists where `layout` and the reference agree today, tile buffers
/// included (the other 72 differ).
const AGREE: &[&[u8]] = &[
    &[1],
    &[2],
    &[4],
    &[1, 1],
    &[2, 1],
    &[1, 2],
    &[2, 2],
    &[1, 1, 1],
    &[2, 1, 1],
    &[2, 2, 1],
    &[1, 1, 2],
    &[2, 1, 2],
    &[1, 2, 2],
    &[2, 2, 2],
    &[1, 1, 4],
    &[2, 1, 4],
    &[1, 2, 4],
    &[2, 2, 4],
    &[1, 1, 1, 1],
    &[2, 1, 1, 1],
    &[1, 1, 2, 1],
    &[2, 2, 4, 1],
    &[4, 4, 4, 1],
    &[1, 1, 1, 2],
    &[2, 1, 1, 2],
    &[1, 1, 2, 2],
    &[2, 1, 4, 2],
    &[1, 2, 4, 2],
    &[2, 2, 4, 2],
    &[4, 4, 4, 2],
    &[1, 1, 1, 4],
    &[2, 1, 1, 4],
    &[2, 2, 1, 4],
    &[4, 4, 1, 4],
    &[1, 1, 2, 4],
    &[2, 1, 2, 4],
    &[1, 2, 2, 4],
    &[2, 2, 2, 4],
    &[4, 4, 2, 4],
    &[1, 1, 4, 4],
    &[2, 1, 4, 4],
    &[4, 1, 4, 4],
    &[1, 2, 4, 4],
    &[2, 2, 4, 4],
    &[4, 2, 4, 4],
    &[1, 4, 4, 4],
    &[2, 4, 4, 4],
    &[4, 4, 4, 4],
];

#[test]
fn lists_are_the_120() {
    assert_eq!(all_lists().len(), 120);
}

#[test]
fn agreeing_lists_are_exactly_the_table() {
    let found: Vec<Vec<u8>> = all_lists().into_iter().filter(|d| actual(d).as_ref() == Some(&reference(d))).collect();
    assert_eq!(found.len(), 48, "agreeing lists");
    let table: Vec<Vec<u8>> = AGREE.iter().map(|d| d.to_vec()).collect();
    assert_eq!(found, table);
}

#[test]
fn lists_that_fit_in_registers_agree() {
    let fits: Vec<Vec<u8>> =
        all_lists().into_iter().filter(|d| actual(d).is_some_and(|a| all_in_registers(&a))).collect();
    assert_eq!(fits.len(), 11, "{fits:?}");
    for d in &fits {
        assert_eq!(actual(d), Some(reference(d)), "{d:?}");
    }
}

#[test]
#[ignore = "defect D1 (plan #368, mesa-rt4-pvr): layout places in order with no back-fill and spills every target after the first miss, one tile buffer each; Mesa uses first fit with back-fill and shared tile buffers"]
fn layout_equals_mesa_first_fit_for_every_list() {
    let bad: Vec<_> = all_lists()
        .into_iter()
        .filter(|d| actual(d).as_ref() != Some(&reference(d)))
        .map(|d| (d.clone(), actual(&d), reference(&d)))
        .collect();
    assert!(bad.is_empty(), "{} lists differ, first: {:?}", bad.len(), &bad[..bad.len().min(3)]);
}
