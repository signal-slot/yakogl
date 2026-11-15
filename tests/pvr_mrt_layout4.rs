// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 on the PowerVR BXS-4-64: four colour targets in the pixel's four
//! 32-bit output registers (`pvr::mrt_layout::register_layout`). A 1-dword
//! (RGBA8-class: RGBA8, R32F, R32I/UI, RG16I/UI, RGBA8I/UI, RGB10_A2UI)
//! target takes one register, a 2-dword one (RGBA16F, RG32F/I/UI,
//! RGBA16I/UI) an even pair, a 4-dword one (RGBA32F/I/UI) all four, in emit
//! order and never back-filled; the first target that does not fit is
//! refused by name (`LayoutError::OutputRegs`) where `layout` would spill it
//! to a tile buffer.
//!
//! The placement is checked against an independent port of Mesa 26.2.1's
//! `pvr_mrt_alloc_from_buffer` (`src/imagination/vulkan/pvr_arch_mrt.c`;
//! the rule as docs/notes/pvr-int-mixed-targets.md section 1 quotes it,
//! since `third_party/mesa-26.2.1` does not vendor that file): first fit
//! over 4 registers, an even start for 2 and 4 registers (`pbe2_in_xe`).
//! And `layout`, the spilling rule, is pinned unchanged for every list of 1
//! to 4 targets against a copy of its walk as it was before
//! `register_layout` joined it.
//!
//! HOST-ONLY: pure arithmetic, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mrt_layout4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{
    layout, register_layout, EotProgram, LayoutError, MrtLayout, Place, MAX_TARGETS, OUTPUT_REGS,
};

/* output-register dwords of a format (`surf_out_dwords`) */
const RGBA8: u8 = 1;
const R32F: u8 = 1;
const R32I: u8 = 1;
const R32UI: u8 = 1;
const RG16UI: u8 = 1;
const RGBA16F: u8 = 2;
const RG32F: u8 = 2;
const RG32I: u8 = 2;
const RG32UI: u8 = 2;
const RGBA16I: u8 = 2;
const RGBA16UI: u8 = 2;
const RGBA32UI: u8 = 4;

/// Each target's first register, or `Err` with `register_layout`'s refusal.
fn regs(dwords: &[u8]) -> Result<(Vec<u8>, u8), LayoutError> {
    let l = register_layout(dwords)?;
    assert_eq!((l.targets as usize, l.tile_buffers), (dwords.len(), 0), "{dwords:?}");
    assert_eq!(l.program, EotProgram::Plain { targets: l.targets }, "{dwords:?}");
    let starts = l.place[..dwords.len()]
        .iter()
        .map(|p| match *p {
            Place::Register { first } => first,
            Place::TileBuffer { .. } => panic!("{dwords:?}: a tile buffer"),
        })
        .collect();
    Ok((starts, l.output_regs))
}

fn refused(target: u8, first: u8, dwords: u8) -> Result<(Vec<u8>, u8), LayoutError> {
    Err(LayoutError::OutputRegs { target, first, dwords })
}

#[test]
fn the_limit_is_four_targets() {
    assert_eq!(MAX_TARGETS, yakogl::backend::GLES_MAX_TARGETS);
    assert_eq!((MAX_TARGETS, OUTPUT_REGS), (4, 4));
    assert_eq!(register_layout(&[1; 5]), Err(LayoutError::TargetCount(5)));
    assert_eq!(register_layout(&[]), Err(LayoutError::TargetCount(0)));
    assert_eq!(register_layout(&[1, 0]), Err(LayoutError::Dwords { target: 1, dwords: 0 }));
    assert_eq!(register_layout(&[1, 5]), Err(LayoutError::Dwords { target: 1, dwords: 5 }));
}

#[test]
fn four_rgba8_take_one_register_each() {
    assert_eq!(regs(&[RGBA8; 4]), Ok((vec![0, 1, 2, 3], 4)));
    for p in 0..4 {
        assert_eq!(register_layout(&[RGBA8; 4]).unwrap().place[p].source_pos(), p as u8);
    }
    /* layout agrees: nothing spills, the four-emit program */
    assert_eq!(layout(&[RGBA8; 4]), register_layout(&[RGBA8; 4]));
}

#[test]
fn rgba8_x2_and_rgba16f_fill_the_four() {
    assert_eq!(regs(&[RGBA8, RGBA8, RGBA16F]), Ok((vec![0, 1, 2], 4)));
    assert_eq!(regs(&[RGBA16F, RGBA8, RGBA8]), Ok((vec![0, 2, 3], 4)));
    assert_eq!(regs(&[RG32F, RGBA8, RGBA8]), Ok((vec![0, 2, 3], 4)));
    assert_eq!(regs(&[RGBA16F, RGBA16F]), Ok((vec![0, 2], 4)));
    /* the pair starts even: after one RGBA8 it skips register 1, and a
     * later RGBA8 does not back-fill it (Mesa would: see the cross-check) */
    assert_eq!(regs(&[RGBA8, RGBA16F]), Ok((vec![0, 2], 4)));
    assert_eq!(regs(&[RGBA8, RGBA16F, RGBA8]), refused(2, 4, 1));
    /* a fourth target finds no register left */
    assert_eq!(regs(&[RGBA8, RGBA8, RGBA16F, RGBA8]), refused(3, 4, 1));
}

#[test]
fn rgba8_x2_and_rgba16f_x2_are_refused() {
    assert_eq!(regs(&[RGBA8, RGBA8, RGBA16F, RGBA16F]), refused(3, 4, 2));
    assert_eq!(regs(&[RGBA16F, RGBA16F, RGBA8, RGBA8]), refused(2, 4, 1));
    assert_eq!(regs(&[RGBA16F, RGBA8, RGBA16F, RGBA8]), refused(2, 4, 2));
    assert_eq!(regs(&[RGBA16F; 4]), refused(2, 4, 2));
    /* layout spills the same target to tile buffer 0 (unchanged) */
    let l = layout(&[RGBA8, RGBA8, RGBA16F, RGBA16F]).unwrap();
    assert_eq!(l.place[3], Place::TileBuffer { index: 0 });
    assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 1 });
}

#[test]
fn integer_mixes() {
    assert_eq!(regs(&[RGBA8, R32UI, R32I, RG16UI]), Ok((vec![0, 1, 2, 3], 4)));
    assert_eq!(regs(&[R32F, R32I, RGBA16UI]), Ok((vec![0, 1, 2], 4)));
    assert_eq!(regs(&[RG32UI, R32I, R32UI]), Ok((vec![0, 2, 3], 4)));
    assert_eq!(regs(&[RGBA16I, RG32UI]), Ok((vec![0, 2], 4)));
    assert_eq!(regs(&[R32UI, RG32I]), Ok((vec![0, 2], 4)));
    assert_eq!(regs(&[RGBA32UI]), Ok((vec![0], 4)));
    assert_eq!(regs(&[RGBA32UI, R32F]), refused(1, 4, 1));
    assert_eq!(regs(&[RGBA8, RGBA32UI]), refused(1, 2, 4));
    assert_eq!(regs(&[R32I, RG32I, R32UI]), refused(2, 4, 1));
    assert_eq!(regs(&[RG32I, RGBA16UI, R32I, R32UI]), refused(2, 4, 1));
}

/// Mesa's `pvr_mrt_alloc_from_buffer` over the 4 output registers: the
/// first start, stepping by the alignment, whose registers are all free;
/// `None` when none is (Mesa then uses a tile buffer).
fn mesa_alloc(used: &mut u8, dwords: u8) -> Option<u8> {
    let align = if dwords == 2 || dwords == 4 { 2 } else { 1 };
    let want = (1u8 << dwords) - 1;
    let mut start = 0;
    while start + dwords <= 4 {
        if *used & (want << start) == 0 {
            *used |= want << start;
            return Some(start);
        }
        start += align;
    }
    None
}

fn mesa(dwords: &[u8]) -> Vec<Option<u8>> {
    let mut used = 0u8;
    dwords.iter().map(|&d| mesa_alloc(&mut used, d)).collect()
}

/// Every list of 1..=4 targets over `set`.
fn lists(set: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for n in 1..=4u32 {
        let k = set.len();
        for code in 0..k.pow(n) {
            out.push((0..n).map(|i| set[code / k.pow(i) % k]).collect());
        }
    }
    out
}

/// Over every list of 1 to 4 targets of 1, 2 and 4 dwords: where
/// `register_layout` places every target, Mesa places them at the same
/// starts; where it refuses target t, Mesa puts t in a tile buffer, except
/// in the pinned lists where Mesa back-fills the hole an even pair left.
#[test]
fn starts_match_mesa_for_four_targets() {
    let mut backfilled = Vec::new();
    for d in lists(&[1, 2, 4]) {
        let m = mesa(&d);
        match regs(&d) {
            Ok((starts, _)) => {
                assert_eq!(m, starts.into_iter().map(Some).collect::<Vec<_>>(), "{d:?}")
            }
            Err(LayoutError::OutputRegs { target, first, dwords }) => {
                let t = target as usize;
                assert_eq!(dwords, d[t], "{d:?}");
                assert!(first + dwords > OUTPUT_REGS, "{d:?}");
                /* the targets before t agree */
                let before = regs(&d[..t]).unwrap().0;
                assert_eq!(m[..t], before.into_iter().map(Some).collect::<Vec<_>>()[..], "{d:?}");
                if let Some(r) = m[t] {
                    assert!(r < first, "{d:?}");
                    backfilled.push(d.clone());
                }
            }
            Err(e) => panic!("{d:?}: {e:?}"),
        }
    }
    let want: Vec<Vec<u8>> = vec![vec![1, 2, 1], vec![1, 2, 1, 1], vec![1, 2, 1, 2], vec![1, 2, 1, 4]];
    assert_eq!(backfilled, want);
}

/// `layout`'s walk as it was before `register_layout` joined it (commit
/// 24c6c9ba): the k-th target that does not fit, and every later one, to
/// tile buffer k.
fn layout_before(dwords: &[u8]) -> Result<MrtLayout, LayoutError> {
    if dwords.is_empty() || dwords.len() > 4 {
        return Err(LayoutError::TargetCount(dwords.len()));
    }
    let targets = dwords.len() as u8;
    let mut place = [Place::Register { first: 0 }; 4];
    let (mut next, mut spilled) = (0u8, 0u8);
    for (i, &d) in dwords.iter().enumerate() {
        if d == 0 || d > 4 {
            return Err(LayoutError::Dwords { target: i as u8, dwords: d });
        }
        let first = if d >= 2 { (next + 1) & !1 } else { next };
        if spilled == 0 && first + d <= 4 {
            place[i] = Place::Register { first };
            next = first + d;
        } else {
            place[i] = Place::TileBuffer { index: spilled };
            spilled += 1;
        }
    }
    let program = match (targets, spilled) {
        (1..=4, 0) => EotProgram::Plain { targets },
        (3, 1) => EotProgram::Eot3Tb,
        (4, 1..=3) => EotProgram::Eot4Tb { spilled },
        _ => return Err(LayoutError::NoProgram { targets, spilled }),
    };
    Ok(MrtLayout { targets, place, output_regs: next, tile_buffers: spilled, program })
}

/// `layout` is unchanged for every list of 1 to 4 targets of 0 to 5
/// dwords, and `register_layout` is `layout` wherever that spills nothing.
#[test]
fn layout_is_unchanged_and_register_layout_is_its_unspilled_part() {
    let mut n = 0;
    for d in lists(&[0, 1, 2, 3, 4, 5]) {
        let l = layout(&d);
        assert_eq!(l, layout_before(&d), "{d:?}");
        match (l, register_layout(&d)) {
            (Ok(l), r) if l.tile_buffers == 0 => assert_eq!(r, Ok(l), "{d:?}"),
            (Ok(_) | Err(LayoutError::NoProgram { .. }), r) => {
                assert!(matches!(r, Err(LayoutError::OutputRegs { .. })), "{d:?}: {r:?}")
            }
            (Err(e), r) => assert_eq!(r, Err(e), "{d:?}"),
        }
        n += 1;
    }
    assert_eq!(n, 6 + 36 + 216 + 1296);
}
