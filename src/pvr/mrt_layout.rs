// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Where the colour targets of a render of up to four go: the pixel's output
//! registers or a tile buffer, and which captured end-of-tile program emits
//! that layout (RC14 step 2's shared rule, docs/es3-cts-failures.md). The
//! compiler places a fragment shader's colour outputs by it, and the back end
//! picks its PBE source positions and its end-of-tile program by it. Pure;
//! nothing here is wired into a draw yet.
//!
//! The rule ("layout B"), over the targets the end-of-tile program emits, in
//! emit order:
//!
//! * A target takes its format's dwords of output registers (1 for the 8-bit
//!   formats, 2 for RGBA16F and sRGB, an integer target's packed dwords, 1..4)
//!   at the next free register, and a target of two or more dwords starts at
//!   an even one (a 64-bit PBE source, `pbe2_in_xe`). A pixel has
//!   [`OUTPUT_REGS`] of them. This is how `backend.rs` places the outputs of
//!   today's up to three targets; registers are never back-filled.
//! * The first target that does not fit, and EVERY target after it, is
//!   spilled: the shader stores it to a tile buffer, the k-th spilled target
//!   to tile buffer k, and the end-of-tile program loads it back and emits it
//!   from dword 0 (`eot_tb_upload` in driver/src/pvr_render.c: the k-th
//!   spilled emit reads tile buffer k). The spilled targets are always the
//!   last ones. Target 0 always fits, so at most three spill, one per tile
//!   buffer (`PVR_RENDER_TILE_BUFFERS`).
//!
//! The captured programs (tools/pco_shc/usc_blobs.txt, `capture_usc eot N
//! s..`: N emits, emits s.. from tile buffers) that this rule reaches:
//!
//! | layout | program | capture | static slot |
//! |---|---|---|---|
//! | 1..4 targets, none spilled | `eot1`..`eot4` | `eot N` | `eot_doutu_src0_n[N]` |
//! | 3 targets, target 2 spilled | `eot3_tb` | `eot 3 2` | `eot_doutu_src0_tb_n[3]` |
//! | 4 targets, target 3 spilled | `eot4_tb` | `eot 4 3` | `eot_doutu_src0_tb4_k[1]` (= `tb_n[4]`) |
//! | 4 targets, targets 2, 3 spilled | `eot4_tb2` | `eot 4 2 3` | `eot_doutu_src0_tb4_k[2]` |
//! | 4 targets, targets 1..3 spilled | `eot4_tb3` | `eot 4 1 2 3` | `eot_doutu_src0_tb4_k[3]` |
//!
//! It also reaches two layouts no program was captured for, which are refused
//! ([`LayoutError::NoProgram`]): two targets with target 1 spilled (an RGBA32UI
//! target and any other), and three with targets 1 and 2 spilled. In
//! particular `eot_doutu_src0_tb_n[2]` holds `eot2_tb`, which was captured as
//! `eot 3 1`: THREE emits, the tile buffer on emit 1 and emit 2 from an output
//! register. It is not a two-target program, and this rule never asks for its
//! layout (a register target after a spilled one), so it is never picked.
//!
//! The tile-buffer programs were captured with one sample per pixel
//! (`msaa_samples 1`, `num_output_regs 4`, tools/pco_shc/README.md); a
//! multisampled render's spill is not covered here.

use crate::osal::render::{PvrRenderStatic, PVR_RENDER_EOT_MAX_RT, PVR_RENDER_TILE_BUFFERS};

/// Output registers per pixel (`backend.rs` `PVR_BACKEND_MAX_OUT_REGS`; the
/// tile-buffer programs' `num_output_regs`).
pub const OUTPUT_REGS: u8 = 4;

/// Colour targets a layout holds: the end-of-tile programs' capacity.
pub const MAX_TARGETS: usize = PVR_RENDER_EOT_MAX_RT;

/// Where one target's dwords are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Output registers `first..first + dwords`; `first` is the PBE source
    /// position (PBESTATE_STATE_WORD1 `source_pos`).
    Register { first: u8 },
    /// Tile buffer `index` (the index-th spilled target), from dword 0: its
    /// PBE source position is 0.
    TileBuffer { index: u8 },
}

impl Place {
    /// The PBE source position of the emit: the first register, or 0 for a
    /// target loaded back from a tile buffer.
    pub const fn source_pos(self) -> u8 {
        match self {
            Place::Register { first } => first,
            Place::TileBuffer { .. } => 0,
        }
    }
}

/// A captured end-of-tile USC program (driver/src/pvr_usc_eot_blob.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EotProgram {
    /// `eot1`..`eot4`: `targets` emits, all from output registers.
    Plain { targets: u8 },
    /// `eot3_tb` (`eot 3 2`): three emits, the last from tile buffer 0.
    Eot3Tb,
    /// `eot4_tb`, `eot4_tb2`, `eot4_tb3` (`spilled` 1, 2, 3): four emits,
    /// the last `spilled` from tile buffers 0..spilled.
    Eot4Tb { spilled: u8 },
}

impl EotProgram {
    /// The program's name in tools/pco_shc/usc_blobs.txt.
    pub const fn name(self) -> &'static str {
        match self {
            EotProgram::Plain { targets: 1 } => "eot1",
            EotProgram::Plain { targets: 2 } => "eot2",
            EotProgram::Plain { targets: 3 } => "eot3",
            EotProgram::Plain { .. } => "eot4",
            EotProgram::Eot3Tb => "eot3_tb",
            EotProgram::Eot4Tb { spilled: 1 } => "eot4_tb",
            EotProgram::Eot4Tb { spilled: 2 } => "eot4_tb2",
            EotProgram::Eot4Tb { .. } => "eot4_tb3",
        }
    }

    /// The number of emits.
    pub const fn emits(self) -> u8 {
        match self {
            EotProgram::Plain { targets } => targets,
            EotProgram::Eot3Tb => 3,
            EotProgram::Eot4Tb { .. } => 4,
        }
    }

    /// The emits that read a tile buffer, as a bit mask over emit indices.
    pub const fn spilled_mask(self) -> u8 {
        match self {
            EotProgram::Plain { .. } => 0,
            EotProgram::Eot3Tb => 1 << 2,
            EotProgram::Eot4Tb { spilled } => (0xf << (4 - spilled)) & 0xf,
        }
    }

    /// Its DOUTU task control in the driver's static objects (0 when the
    /// driver did not upload it).
    pub fn doutu(self, st: &PvrRenderStatic) -> u64 {
        match self {
            EotProgram::Plain { targets } => st.eot_doutu_src0_n[targets as usize],
            EotProgram::Eot3Tb => st.eot_doutu_src0_tb_n[3],
            EotProgram::Eot4Tb { spilled } => st.eot_doutu_src0_tb4_k[spilled as usize],
        }
    }

    /// Its CR_EVENT_PIXEL_PDS_INFO in the driver's static objects.
    pub fn pds_info(self, st: &PvrRenderStatic) -> u32 {
        match self {
            EotProgram::Plain { targets } => st.event_pixel_pds_info_n[targets as usize],
            EotProgram::Eot3Tb => st.event_pixel_pds_info_tb_n[3],
            EotProgram::Eot4Tb { .. } => st.event_pixel_pds_info_tb_n[4],
        }
    }
}

/// Why a set of targets has no layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// No targets, or more than [`MAX_TARGETS`].
    TargetCount(usize),
    /// Target `target` takes `dwords` registers: a format takes 1..4.
    Dwords { target: u8, dwords: u8 },
    /// `targets` targets whose last `spilled` spill: no end-of-tile program
    /// was captured for that.
    NoProgram { targets: u8, spilled: u8 },
}

/// The layout of a render's colour targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MrtLayout {
    /// Targets, in emit order.
    pub targets: u8,
    /// Each target's place (`targets` of them are meaningful).
    pub place: [Place; MAX_TARGETS],
    /// Output registers the register targets span (the shader record's
    /// output count).
    pub output_regs: u8,
    /// Spilled targets: the last `tile_buffers`, tile buffers
    /// 0..tile_buffers.
    pub tile_buffers: u8,
    /// The end-of-tile program that emits this layout.
    pub program: EotProgram,
}

/// The program for `targets` targets whose last `spilled` come from tile
/// buffers, if one was captured.
const fn program_for(targets: u8, spilled: u8) -> Option<EotProgram> {
    match (targets, spilled) {
        (1..=4, 0) => Some(EotProgram::Plain { targets }),
        (3, 1) => Some(EotProgram::Eot3Tb),
        (4, 1..=3) => Some(EotProgram::Eot4Tb { spilled }),
        _ => None,
    }
}

/// Lays out targets of `dwords[i]` output-register dwords each, in emit
/// order, by the rule in the module docs.
pub fn layout(dwords: &[u8]) -> Result<MrtLayout, LayoutError> {
    if dwords.is_empty() || dwords.len() > MAX_TARGETS {
        return Err(LayoutError::TargetCount(dwords.len()));
    }
    let targets = dwords.len() as u8;
    let mut place = [Place::Register { first: 0 }; MAX_TARGETS];
    let mut next = 0u8;
    let mut spilled = 0u8;
    for (i, &d) in dwords.iter().enumerate() {
        if d == 0 || d > OUTPUT_REGS {
            return Err(LayoutError::Dwords { target: i as u8, dwords: d });
        }
        let first = if d >= 2 { (next + 1) & !1 } else { next };
        if spilled == 0 && first + d <= OUTPUT_REGS {
            place[i] = Place::Register { first };
            next = first + d;
        } else {
            place[i] = Place::TileBuffer { index: spilled };
            spilled += 1;
        }
    }
    debug_assert!(spilled as usize <= PVR_RENDER_TILE_BUFFERS);
    let program = program_for(targets, spilled).ok_or(LayoutError::NoProgram { targets, spilled })?;
    Ok(MrtLayout { targets, place, output_regs: next, tile_buffers: spilled, program })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(first: u8) -> Place {
        Place::Register { first }
    }
    fn tb(index: u8) -> Place {
        Place::TileBuffer { index }
    }

    fn places(l: &MrtLayout) -> &[Place] {
        &l.place[..l.targets as usize]
    }

    /// The `capture_usc` arguments of `name` in usc_blobs.txt: the emit
    /// count and the mask of emits read from tile buffers.
    fn capture(name: &str) -> (u8, u8) {
        let list = include_str!("../../tools/pco_shc/usc_blobs.txt");
        let line = list
            .lines()
            .find(|l| l.split(" | ").next() == Some(name))
            .unwrap_or_else(|| panic!("{name} is not in usc_blobs.txt"));
        let args: Vec<u8> = line.split(" | ").nth(1).unwrap().split(' ').skip(1).map(|a| a.parse().unwrap()).collect();
        assert_eq!(line.split(" | ").nth(1).unwrap().split(' ').next(), Some("eot"), "{line}");
        (args[0], args[1..].iter().fold(0, |m, &e| m | 1 << e))
    }

    #[test]
    fn registers_in_order_wide_targets_on_even_registers() {
        let l = layout(&[1, 1, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(1), reg(2)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (3, 0, EotProgram::Plain { targets: 3 }));

        let l = layout(&[1, 1, 1, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(1), reg(2), reg(3)]);
        assert_eq!(l.program, EotProgram::Plain { targets: 4 });

        /* an RGBA16F target after an RGBA8 one skips register 1 */
        let l = layout(&[1, 2]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2)]);
        assert_eq!((l.output_regs, l.program), (4, EotProgram::Plain { targets: 2 }));

        let l = layout(&[2, 2]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2)]);
        let l = layout(&[4]).unwrap();
        assert_eq!((places(&l), l.program), (&[reg(0)][..], EotProgram::Plain { targets: 1 }));
        let l = layout(&[3]).unwrap();
        assert_eq!(l.output_regs, 3);
    }

    #[test]
    fn overflow_spills_that_target_and_every_later_one() {
        /* three RGBA16F targets (the Quick 3D G-buffer): the third spills */
        let l = layout(&[2, 2, 2]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2), tb(0)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (4, 1, EotProgram::Eot3Tb));

        /* RGBA8, RGBA16F, RGBA8: the third does not back-fill register 1 */
        let l = layout(&[1, 2, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2), tb(0)]);
        assert_eq!(l.program, EotProgram::Eot3Tb);

        let l = layout(&[1, 1, 2, 2]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(1), reg(2), tb(0)]);
        assert_eq!((l.tile_buffers, l.program), (1, EotProgram::Eot4Tb { spilled: 1 }));

        /* target 3 would fit register 1, but follows a spilled target */
        let l = layout(&[1, 4, 4, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), tb(0), tb(1), tb(2)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (1, 3, EotProgram::Eot4Tb { spilled: 3 }));

        let l = layout(&[2, 2, 2, 2]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2), tb(0), tb(1)]);
        assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 2 });

        let l = layout(&[4, 1, 1, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), tb(0), tb(1), tb(2)]);
        assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 3 });
        assert_eq!(l.place[1].source_pos(), 0);
        assert_eq!(l.place[0].source_pos(), 0);
        assert_eq!(layout(&[1, 2]).unwrap().place[1].source_pos(), 2);
    }

    /// Integer colour attachments beside float/unorm ones, as output-register
    /// dwords (`surf_out_dwords`): RGBA8, R32F, R32I, R32UI and RG16UI take 1,
    /// RGBA16F and RGBA16UI 2, RGBA32UI 4. The placement, and with it the PBE
    /// source positions a later wiring reads off this layout, is the emit
    /// loop's walk (`backend.rs`, the end-of-tile emit) for these mixes:
    /// RGBA8 + R32UI and R32F + R32I at registers 0, 1; R32I + RGBA16F at 0, 2
    /// (the 64-bit source skips register 1); RGBA8 + RG16UI + R32F at 0, 1, 2;
    /// RGBA8 + RGBA16UI + R32UI spills the third target; RGBA32UI + R32F is
    /// refused (docs/notes/pvr-int-mixed-targets.md section 3).
    #[test]
    fn integer_beside_float_unorm_mixes() {
        /* RGBA8 + R32UI: two 1-dword targets, the second's source at 1 */
        let l = layout(&[1, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(1)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (2, 0, EotProgram::Plain { targets: 2 }));
        assert_eq!((l.place[0].source_pos(), l.place[1].source_pos()), (0, 1));

        /* R32F + R32I: the same dwords, the same placement -- the rule is
         * per-target dwords, not per-target class */
        let l = layout(&[1, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(1)]);
        assert_eq!((l.place[0].source_pos(), l.place[1].source_pos()), (0, 1));

        /* R32I + RGBA16F: the 2-dword source starts even, skipping
         * register 1; the span is the four registers, with no spill */
        let l = layout(&[1, 2]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (4, 0, EotProgram::Plain { targets: 2 }));
        assert_eq!((l.place[0].source_pos(), l.place[1].source_pos()), (0, 2));

        /* RGBA8 + RG16UI + R32F: three 1-dword targets fill 0, 1, 2 */
        let l = layout(&[1, 1, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(1), reg(2)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (3, 0, EotProgram::Plain { targets: 3 }));

        /* RGBA8 + RGBA16UI + R32UI: the 2-dword target takes registers 2..4,
         * the 1-dword one finds none left, and register 1 is not back-filled:
         * it spills to tile buffer 0 (eot3_tb), emitted from dword 0 */
        let l = layout(&[1, 2, 1]).unwrap();
        assert_eq!(places(&l), [reg(0), reg(2), tb(0)]);
        assert_eq!((l.output_regs, l.tile_buffers, l.program), (4, 1, EotProgram::Eot3Tb));
        assert_eq!(l.place[2].source_pos(), 0);

        /* RGBA32UI + R32F: the 4-dword target fills the four registers, the
         * second spills as target 1 of two, and eot_doutu_src0_tb_n[2] is
         * eot2_tb, a three-emit program: refused (the family:
         * `spills_without_a_captured_program_are_refused`) */
        assert_eq!(layout(&[4, 1]), Err(LayoutError::NoProgram { targets: 2, spilled: 1 }));
    }

    #[test]
    fn spills_without_a_captured_program_are_refused() {
        /* two targets, the second spilled: eot_doutu_src0_tb_n[2] is
         * eot2_tb, a three-emit program (`eot 3 1`), not this */
        for d in [[4, 1], [3, 2], [2, 4], [4, 4], [1, 4]] {
            assert_eq!(layout(&d), Err(LayoutError::NoProgram { targets: 2, spilled: 1 }), "{d:?}");
        }
        /* three targets, the last two spilled: no `eot 3 1 2` */
        for d in [[4, 1, 1], [1, 4, 1], [2, 3, 1]] {
            assert_eq!(layout(&d), Err(LayoutError::NoProgram { targets: 3, spilled: 2 }), "{d:?}");
        }
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(layout(&[]), Err(LayoutError::TargetCount(0)));
        assert_eq!(layout(&[1; 5]), Err(LayoutError::TargetCount(5)));
        assert_eq!(layout(&[1, 0]), Err(LayoutError::Dwords { target: 1, dwords: 0 }));
        assert_eq!(layout(&[5]), Err(LayoutError::Dwords { target: 0, dwords: 5 }));
    }

    /// Every layout of 1..4 targets of 1..4 dwords: spilled targets are a
    /// suffix in tile-buffer order, registers never overlap or pass the
    /// fourth, and the program picked emits exactly the targets, reading
    /// tile buffers on exactly the spilled ones -- checked against the
    /// capture arguments in usc_blobs.txt, not against this module's tables.
    #[test]
    fn every_layout_matches_its_captured_program() {
        let mut seen = std::collections::BTreeSet::new();
        for n in 1..=MAX_TARGETS as u32 {
            for code in 0..4u32.pow(n) {
                let d: Vec<u8> = (0..n).map(|i| (code / 4u32.pow(i) % 4 + 1) as u8).collect();
                let l = match layout(&d) {
                    Ok(l) => l,
                    Err(LayoutError::NoProgram { targets, spilled }) => {
                        assert!(matches!((targets, spilled), (2, 1) | (3, 2)), "{d:?}");
                        continue;
                    }
                    Err(e) => panic!("{d:?}: {e:?}"),
                };
                let mut used = 0u8;
                let mut mask = 0u8;
                for (i, p) in places(&l).iter().enumerate() {
                    match *p {
                        Place::Register { first } => {
                            assert_eq!(mask, 0, "{d:?}: a register target after a spilled one");
                            let bits = ((1u16 << d[i]) - 1) << first;
                            assert!(bits < 1 << OUTPUT_REGS, "{d:?}");
                            assert_eq!(used & bits as u8, 0, "{d:?}");
                            assert!(d[i] < 2 || first % 2 == 0, "{d:?}");
                            used |= bits as u8;
                        }
                        Place::TileBuffer { index } => {
                            assert_eq!(index as u32, mask.count_ones(), "{d:?}");
                            mask |= 1 << i;
                        }
                    }
                }
                assert!((l.tile_buffers as usize) <= PVR_RENDER_TILE_BUFFERS);
                assert_eq!(l.tile_buffers as u32, mask.count_ones());
                assert_eq!((l.program.emits(), l.program.spilled_mask()), (l.targets, mask), "{d:?}");
                assert_eq!(capture(l.program.name()), (l.targets, mask), "{d:?}: {}", l.program.name());
                seen.insert(l.program.name());
            }
        }
        /* every captured program but eot2_tb is reached */
        let all = ["eot1", "eot2", "eot3", "eot4", "eot3_tb", "eot4_tb", "eot4_tb2", "eot4_tb3"];
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), {
            let mut v = all.to_vec();
            v.sort_unstable();
            v
        });
        /* eot2_tb, in tb_n[2], is three emits with a register emit after a
         * spilled one: a layout this rule never makes */
        assert_eq!(capture("eot2_tb"), (3, 0b010));
    }

    #[test]
    fn programs_read_their_static_slots() {
        let st = PvrRenderStatic {
            eot_doutu_src0_n: [0, 0x1001, 0x1002, 0x1003, 0x1004],
            event_pixel_pds_info_n: [0, 11, 12, 13, 14],
            eot_doutu_src0_tb_n: [0, 0, 0x2002, 0x2003, 0x2004],
            event_pixel_pds_info_tb_n: [0, 0, 22, 23, 24],
            eot_doutu_src0_tb4_k: [0, 0x2004, 0x3002, 0x3003],
            ..Default::default()
        };
        let pick = |d: &[u8]| {
            let p = layout(d).unwrap().program;
            (p.doutu(&st), p.pds_info(&st))
        };
        assert_eq!(pick(&[1]), (0x1001, 11));
        assert_eq!(pick(&[1, 1]), (0x1002, 12));
        assert_eq!(pick(&[1, 1, 1, 1]), (0x1004, 14));
        assert_eq!(pick(&[2, 2, 2]), (0x2003, 23));
        assert_eq!(pick(&[1, 1, 2, 2]), (0x2004, 24));
        assert_eq!(pick(&[2, 2, 2, 2]), (0x3002, 24));
        assert_eq!(pick(&[4, 1, 1, 1]), (0x3003, 24));
    }
}
