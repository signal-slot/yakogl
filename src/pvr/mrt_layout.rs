// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Where each of up to four colour targets leaves the PowerVR's fragment
//! program, and which end-of-tile program stores them (RC14, plan #325).
//!
//! The one place the rule is written: the compiler (`shaderc::online::powervr`,
//! which `#[path]`-includes this file in a build without `gpu-powervr`) and
//! the back end (`pvr::backend`) both call [`layout`] with the dwords each
//! bound target takes in its pixel output, and must agree with the programs of
//! `driver/src/pvr_usc_eot_blob.h` (`tools/pco_shc/usc_blobs.txt`).
//!
//! The rule ("layout B", as `tools/pco_shc/pco_shc.c` places outputs): the
//! targets go in order. A pixel has [`OUT_REGS`] output registers; a target
//! of two or more dwords starts at an even register. A target that does not
//! fit after the registers taken so far is spilled to the next tile buffer
//! (index in spill order, offset 0) and takes no register, so a later,
//! smaller target can still fit in the registers. The end-of-tile programs
//! exist for 1..4 targets plain, and for these spill sets only (each target
//! named by index): 3 targets {1} or {2}; 4 targets {3}, {2,3} or {1,2,3}. Any
//! other spill set has no program and is refused.
//!
//! This file depends on nothing outside `core`: it is included by path.

/// Colour targets one draw can carry: `PVR_RENDER_EOT_MAX_RT`.
pub const MAX_TARGETS: usize = 4;
/// 32-bit pixel output registers of the BXS-4-64 (no eight_output_registers).
pub const OUT_REGS: u32 = 4;
/// Tile buffers (`PVR_RENDER_TILE_BUFFERS`), one per spilled target.
pub const TILE_BUFFERS: usize = 3;

/// Where one target's pixel output lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Output registers `offset..offset + dwords` (the PBE's `source_pos`).
    Regs { offset: u32 },
    /// Tile buffer `index`, from dword `offset` (always 0: one per target).
    TileBuffer { index: u32, offset: u32 },
}

/// One target's slot: its place and the dwords of its pixel output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub place: Place,
    pub dwords: u32,
}

/// The end-of-tile program a layout needs, as the driver's tables name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EotProgram {
    /// `eot_doutu_src0_n[n]` / `event_pixel_pds_info_n[n]`: no tile buffer.
    Plain(u32),
    /// `eot_doutu_src0_tb_n[n]` with n = 3: `eot3_tb` (target 2 spilled) or
    /// `eot2_tb` (target 1 spilled; the capture's "eot 3 1"). `spilled` is
    /// the index of the spilled target.
    TbN { n: u32, spilled: u32 },
    /// `eot_doutu_src0_tb4_k[k]`: four targets, the last `k` (1..=3) spilled.
    Tb4K(u32),
}

/// A layout the programs cannot carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No target.
    None,
    /// More than [`MAX_TARGETS`].
    TooMany(usize),
    /// Target `.0` takes `.1` dwords: not 1..=4.
    BadDwords(usize, u32),
    /// The spill set (bit i = target i) has no end-of-tile program for this
    /// many targets. Includes two targets with the last one spilled: the
    /// driver has no "eot 2 1" program.
    NoProgram { targets: usize, spilled: u32 },
}

/// The result: a slot per target, the program, and what the shader uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MrtLayout {
    pub targets: usize,
    pub slots: [Slot; MAX_TARGETS],
    pub eot: EotProgram,
    /// Output registers the outputs in registers take (to the end of the last).
    pub out_regs: u32,
    /// Tile buffers in use (0..=3).
    pub tile_buffers: u32,
}

impl MrtLayout {
    /// The slots of the targets, in order.
    pub fn slots(&self) -> &[Slot] {
        &self.slots[..self.targets]
    }

    /// Bit i set: target i is in a tile buffer.
    pub fn spilled_mask(&self) -> u32 {
        self.slots().iter().enumerate().fold(0, |m, (i, s)| match s.place {
            Place::TileBuffer { .. } => m | 1 << i,
            Place::Regs { .. } => m,
        })
    }
}

/// Lays out targets whose pixel outputs take `dwords[i]` dwords (1 for an
/// 8-bit target, R16F, R32F; 2 for RGBA16F and sRGB; an integer target's
/// packed dwords, `ShcIntRt::dwords`).
pub fn layout(dwords: &[u32]) -> Result<MrtLayout, Refusal> {
    let n = dwords.len();
    if n == 0 {
        return Err(Refusal::None);
    }
    if n > MAX_TARGETS {
        return Err(Refusal::TooMany(n));
    }
    let blank = Slot { place: Place::Regs { offset: 0 }, dwords: 0 };
    let mut slots = [blank; MAX_TARGETS];
    let (mut reg, mut tbs, mut mask) = (0u32, 0u32, 0u32);

    for (i, &dw) in dwords.iter().enumerate() {
        if !(1..=OUT_REGS).contains(&dw) {
            return Err(Refusal::BadDwords(i, dw));
        }
        let start = if dw >= 2 { reg.next_multiple_of(2) } else { reg };
        let place = if start + dw > OUT_REGS {
            mask |= 1 << i;
            tbs += 1;
            Place::TileBuffer { index: tbs - 1, offset: 0 }
        } else {
            reg = start + dw;
            Place::Regs { offset: start }
        };
        slots[i] = Slot { place, dwords: dw };
    }
    let eot = match (n, mask) {
        (_, 0) => EotProgram::Plain(n as u32),
        (3, 0b010) | (3, 0b100) => EotProgram::TbN { n: 3, spilled: mask.trailing_zeros() },
        (4, 0b1000) => EotProgram::Tb4K(1),
        (4, 0b1100) => EotProgram::Tb4K(2),
        (4, 0b1110) => EotProgram::Tb4K(3),
        _ => return Err(Refusal::NoProgram { targets: n, spilled: mask }),
    };
    Ok(MrtLayout { targets: n, slots, eot, out_regs: reg, tile_buffers: tbs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs(o: u32, d: u32) -> Slot {
        Slot { place: Place::Regs { offset: o }, dwords: d }
    }
    fn tb(i: u32, d: u32) -> Slot {
        Slot { place: Place::TileBuffer { index: i, offset: 0 }, dwords: d }
    }
    fn ok(d: &[u32]) -> MrtLayout {
        layout(d).unwrap()
    }

    #[test]
    fn one_to_three_8bit_targets_take_one_register_each() {
        for n in 1..=3u32 {
            let l = ok(&vec![1; n as usize]);
            let want: Vec<_> = (0..n).map(|i| regs(i, 1)).collect();
            assert_eq!(l.slots(), &want[..]);
            assert_eq!(l.eot, EotProgram::Plain(n));
            assert_eq!((l.out_regs, l.tile_buffers), (n, 0));
        }
    }

    #[test]
    fn rgba16f_targets_take_even_pairs() {
        // backend.rs: "a 64-bit source starts at an even register".
        let l = ok(&[2, 2]);
        assert_eq!(l.slots(), &[regs(0, 2), regs(2, 2)]);
        assert_eq!(l.eot, EotProgram::Plain(2));
        let l = ok(&[1, 2]);
        assert_eq!(l.slots(), &[regs(0, 1), regs(2, 2)]);
        assert_eq!(l.out_regs, 4);
    }

    #[test]
    fn three_rgba16f_spill_the_third() {
        // the Qt Quick 3D G-buffer case of GL_CORE_NOTES.md: six registers.
        let l = ok(&[2, 2, 2]);
        assert_eq!(l.slots(), &[regs(0, 2), regs(2, 2), tb(0, 2)]);
        assert_eq!(l.eot, EotProgram::TbN { n: 3, spilled: 2 });
        assert_eq!((l.out_regs, l.tile_buffers, l.spilled_mask()), (4, 1, 0b100));
    }

    #[test]
    fn later_small_target_fits_after_a_spill() {
        // pco_shc.c does not advance the register on a spill: eot "3 1".
        let l = ok(&[2, 4, 2]);
        assert_eq!(l.slots(), &[regs(0, 2), tb(0, 4), regs(2, 2)]);
        assert_eq!(l.eot, EotProgram::TbN { n: 3, spilled: 1 });
    }

    #[test]
    fn four_targets_spill_suffixes() {
        let l = ok(&[1, 1, 1, 1]);
        assert_eq!(l.eot, EotProgram::Plain(4));
        assert_eq!(l.slots()[3], regs(3, 1));
        let l = ok(&[2, 1, 1, 2]);
        assert_eq!(l.slots(), &[regs(0, 2), regs(2, 1), regs(3, 1), tb(0, 2)]);
        assert_eq!(l.eot, EotProgram::Tb4K(1));
        let l = ok(&[2, 2, 2, 2]);
        assert_eq!(l.slots(), &[regs(0, 2), regs(2, 2), tb(0, 2), tb(1, 2)]);
        assert_eq!(l.eot, EotProgram::Tb4K(2));
        let l = ok(&[4, 2, 2, 4]);
        assert_eq!(l.slots(), &[regs(0, 4), tb(0, 2), tb(1, 2), tb(2, 4)]);
        assert_eq!(l.eot, EotProgram::Tb4K(3));
        assert_eq!((l.spilled_mask(), l.tile_buffers), (0b1110, 3));
    }

    #[test]
    fn spill_sets_without_a_program_are_refused() {
        // target 2 spills, target 3 fits again: "eot 4 2" does not exist.
        assert_eq!(layout(&[2, 1, 4, 1]), Err(Refusal::NoProgram { targets: 4, spilled: 0b0100 }));
        // two targets, the last spilled: no "eot 2 1".
        assert_eq!(layout(&[4, 1]), Err(Refusal::NoProgram { targets: 2, spilled: 0b10 }));
        // target 1 spilled of four (the later ones fit), and targets 1 and 3.
        assert_eq!(layout(&[2, 4, 1, 1]), Err(Refusal::NoProgram { targets: 4, spilled: 0b0010 }));
        assert_eq!(layout(&[2, 4, 2, 2]), Err(Refusal::NoProgram { targets: 4, spilled: 0b1010 }));
    }

    #[test]
    fn bad_inputs() {
        assert_eq!(layout(&[]), Err(Refusal::None));
        assert_eq!(layout(&[1; 5]), Err(Refusal::TooMany(5)));
        assert_eq!(layout(&[1, 0]), Err(Refusal::BadDwords(1, 0)));
        assert_eq!(layout(&[5]), Err(Refusal::BadDwords(0, 5)));
    }
}
