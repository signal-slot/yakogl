// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2022-2023 Imagination Technologies Ltd.
//! The smart parameter mode (SPM) scratch of a partial render: its size, and
//! the PBE words that make the partial render's end-of-tile store the tiles'
//! raw output dwords to it (docs/notes/pvr-render-pool.md, "Design: the
//! firmware's partial render", part 2). Pure functions; nothing here
//! allocates or kicks.
//!
//! Transcribed from Mesa 26.2.1 (MIT, Copyright © 2022-2023 Imagination
//! Technologies Ltd.):
//! - [`scratch_size`]: `pvr_arch_spm_scratch_buffer_calc_required_size`,
//!   src/imagination/vulkan/pvr_arch_spm.c:39-75;
//! - [`pbe_packmode`]: `pvr_spm_get_pbe_packmode`, pvr_arch_spm.c:258-273;
//! - [`pbe_state`]: `pvr_spm_setup_pbe_state`, pvr_arch_spm.c:285-326, with
//!   the part of `pvr_arch_pbe_pack_state` it reaches
//!   (src/imagination/vulkan/pvr_arch_job_common.c:82-297: not z-only,
//!   linear, swizzle X,Y,Z,W, gamma none, not normalised, no downscale,
//!   MRT index 0, min clips and slice 0) and `pvr_pbestate_source_pos`
//!   (pvr_csb_enum_helpers.h:130-152).
//!
//! Our core (BXS-4-64, 36.53.104.796) has no `eight_output_registers`
//! feature, so a pixel has at most four output dwords, the source start is
//! one of the first 128 bits and STATE_WORD1.source_pos_offset_128 stays 0
//! (pvr_arch_job_common.c:125-130); Mesa's eight-dword path
//! (`pvr_spm_setup_pbe_eight_dword_write`) is not taken here.

use super::csbgen as csb;

/// CR_PBE_WORD0_MRT0.linestride's ALIGNMENT (Mesa csbgen/rogue/cr.xml:612-614):
/// the scratch's rows are this many pixels apart at least.
pub const PBE_LINESTRIDE_ALIGNMENT: u32 = 2;
/// PBESTATE_REG_WORD0.linestride's UNIT_SIZE (Mesa csbgen/rogue/pbestate.xml
/// :258-260): the field counts the row pitch in units of this many pixels.
pub const PBE_LINESTRIDE_UNIT_SIZE: u32 = 2;
/// The output dwords a pixel has on this core (no `eight_output_registers`;
/// backend.rs `PVR_BACKEND_MAX_OUT_REGS`).
pub const SPM_MAX_OUTPUT_DWORDS: u32 = 4;

/// The bytes of SPM scratch a render of `width` x `height` pixels at
/// `samples` samples needs, for a pass whose renders use at most
/// `output_dwords` output registers a pixel and `tile_buffers` tile buffers.
///
/// Mesa takes the maximum over the pass's hardware renders, starting the
/// output dword count at 1 (pvr_arch_spm.c:49-62); the caller hands that
/// maximum here, and a count of 0 is taken as Mesa's 1. One more than the
/// tile buffers accounts for the output registers, which are always stored
/// (:64-67). The row is `width` aligned to the PBE's line-stride alignment
/// (:69-72).
pub fn scratch_size(width: u32, height: u32, samples: u32, output_dwords: u32, tile_buffers: u32) -> u64 {
    let max_used_dwords = output_dwords.max(1);
    let dwords_per_pixel = u64::from(samples) * u64::from(max_used_dwords) * (1 + u64::from(tile_buffers));
    let row = u64::from(width.next_multiple_of(PBE_LINESTRIDE_ALIGNMENT));
    row * u64::from(height) * dwords_per_pixel * 4
}

/// The PBE pack mode that stores `dword_count` raw dwords a pixel: U32 to
/// U32U32U32U32 (pvr_arch_spm.c:258-273). Mesa has no other count
/// (`UNREACHABLE`); `None` here.
pub fn pbe_packmode(dword_count: u32) -> Option<csb::ROGUE_PBESTATE_PACKMODE> {
    match dword_count {
        1 => Some(csb::ROGUE_PBESTATE_PACKMODE_U32),
        2 => Some(csb::ROGUE_PBESTATE_PACKMODE_U32U32),
        3 => Some(csb::ROGUE_PBESTATE_PACKMODE_U32U32U32),
        4 => Some(csb::ROGUE_PBESTATE_PACKMODE_U32U32U32U32),
        _ => None,
    }
}

/// One PR store's PBE state: Mesa's `pbe_state_words_out` (the
/// PBESTATE_STATE_WORD0/1 pair the end-of-tile program hands the PBE),
/// `pbe_reg_words_out` (PBESTATE_REG_WORD0..2, the CR_PBE_WORD0..2 of the
/// emit; word 2 is always 0, pvr_arch_job_common.c:100) and the bytes the
/// store takes in the scratch from `scratch_addr` (the function's return).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpmPbe {
    pub state: [u32; 2],
    pub reg: [u64; 3],
    pub bytes: u64,
}

/// The PBE words that store `dword_count` raw output dwords a pixel of a
/// `width` x `height` render at `samples` samples into the scratch at
/// `scratch_addr`, starting at output dword `source_dword` (0 to 3: Mesa's
/// PVR_PBE_STARTPOS_BIT0/32/64/96; the PR EOT uses 0, pvr_arch_spm.c:545,
/// :568). `pvr_spm_setup_pbe_state` (pvr_arch_spm.c:285-326): a linear
/// surface whose row is `width` aligned to the line-stride unit, the source
/// 8 bits a channel (raw, not normalised), pack mode by dword count, clipped
/// to the render. `None` for a dword count or start Mesa does not have on
/// this core.
pub fn pbe_state(
    width: u32,
    height: u32,
    dword_count: u32,
    source_dword: u32,
    samples: u32,
    scratch_addr: u64,
) -> Option<SpmPbe> {
    let packmode = pbe_packmode(dword_count)?;
    let source_pos = match source_dword {
        0 => csb::ROGUE_PBESTATE_SOURCE_POS_START_BIT0,
        1 => csb::ROGUE_PBESTATE_SOURCE_POS_START_BIT32,
        2 => csb::ROGUE_PBESTATE_SOURCE_POS_START_BIT64,
        3 => csb::ROGUE_PBESTATE_SOURCE_POS_START_BIT96,
        _ => return None,
    };
    let stride = width.next_multiple_of(PBE_LINESTRIDE_UNIT_SIZE);
    let addr = csb::pvr_dev_addr_t { addr: scratch_addr };

    let w0 = csb_pack!(ROGUE_PBESTATE_STATE_WORD0_pack, 1, csb::ROGUE_PBESTATE_STATE_WORD0 { address_low: addr });
    let w1 = csb_pack!(
        ROGUE_PBESTATE_STATE_WORD1_pack,
        1,
        csb::ROGUE_PBESTATE_STATE_WORD1 {
            address_high: addr,
            source_format: csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL,
            source_pos,
            mrt_index: 0,
            norm: false,
            packmode,
            ..Default::default()
        }
    );
    let r0 = csb_pack!(
        ROGUE_PBESTATE_REG_WORD0_pack,
        2,
        csb::ROGUE_PBESTATE_REG_WORD0 {
            tilerelative: true,
            memlayout: csb::ROGUE_PBESTATE_MEMLAYOUT_LINEAR,
            rotation: csb::ROGUE_PBESTATE_ROTATION_TYPE_0_DEG,
            y_flip: csb::ROGUE_PBESTATE_Y_FLIP_DISABLED,
            linestride: (stride - 1) / PBE_LINESTRIDE_UNIT_SIZE,
            minclip_x: 0,
            /* swizzle X, Y, Z, W: channel n from source channel n */
            swiz_chan0: csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN0,
            swiz_chan1: csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN1,
            swiz_chan2: csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN2,
            swiz_chan3: csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN3,
            downscale: false,
            ..Default::default()
        }
    );
    let r1 = csb_pack!(
        ROGUE_PBESTATE_REG_WORD1_pack,
        2,
        csb::ROGUE_PBESTATE_REG_WORD1 {
            minclip_y: 0,
            maxclip_x: width - 1,
            zslice: 0,
            maxclip_y: height - 1,
            ..Default::default()
        }
    );
    let dw64 = |dw: [u32; 2]| u64::from(dw[0]) | u64::from(dw[1]) << 32;
    Some(SpmPbe {
        state: [w0[0], w1[0]],
        reg: [dw64(r0), dw64(r1), 0],
        bytes: u64::from(stride) * u64::from(height) * u64::from(samples) * 4 * u64::from(dword_count),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_size_per_render_size() {
        /* 256x256, RGBA8 (1 output dword), 1 sample, no tile buffers:
         * 256 (already a multiple of 2) x 256 x 4 B x 1 x 1 x (1 + 0) */
        assert_eq!(scratch_size(256, 256, 1, 1, 0), 262_144);
        /* 800x600, the design's 1.92 MB. By hand: the row is 800 pixels
         * (800 is even, so the alignment to 2 adds nothing); dwords per
         * pixel = samples 1 x dwords 1 x (1 + 0 tile buffers) = 1, so 4 B a
         * pixel; 800 x 600 = 480 000 pixels x 4 B = 1 920 000 B. */
        assert_eq!(scratch_size(800, 600, 1, 1, 0), 1_920_000);
        /* 1600x1200, the design's 7.7 MB */
        assert_eq!(scratch_size(1600, 1200, 1, 1, 0), 7_680_000);
        /* 4x MSAA at 800x600: four samples a pixel */
        assert_eq!(scratch_size(800, 600, 4, 1, 0), 7_680_000);
        /* 4x MSAA at 256x256 with 2 output dwords and 1 tile buffer:
         * 256 x 256 x 4 B x 4 x 2 x 2 */
        assert_eq!(scratch_size(256, 256, 4, 2, 1), 4_194_304);
        /* an odd width rounds the row up to 2 pixels: 802 x 600 x 4 */
        assert_eq!(scratch_size(801, 600, 1, 1, 0), 1_924_800);
        /* Mesa counts at least one output dword */
        assert_eq!(scratch_size(800, 600, 1, 0, 0), 1_920_000);
    }

    #[test]
    fn packmode_by_dword_count() {
        assert_eq!(pbe_packmode(1), Some(0x34));
        assert_eq!(pbe_packmode(2), Some(0x28));
        assert_eq!(pbe_packmode(3), Some(0x1a));
        assert_eq!(pbe_packmode(4), Some(0x0b));
        assert_eq!(pbe_packmode(0), None);
        assert_eq!(pbe_packmode(5), None);
    }

    #[test]
    fn pbe_words_800x600_rgba8() {
        /* The fields, pbestate.xml:226-282, at scratch 0x4_8010_0000:
         * STATE_WORD0 address_low = addr >> 2 (low 32 bits) = 0x2004_0000;
         * STATE_WORD1 source_format 8_PER_CHANNEL (1 << 19), packmode U32
         * (0x34 << 7), address_high = addr >> 34 = 1;
         * REG_WORD0 tilerelative (bit 57), linestride (800 - 1) / 2 = 399 at
         * bit 34, swizzle chan3/2/1/0 = 3/2/1/0 at bits 29/26/23/20;
         * REG_WORD1 maxclip_x 799 at bit 32, maxclip_y 599 at bit 0. */
        let pbe = pbe_state(800, 600, 1, 0, 1, 0x4_8010_0000).unwrap();
        assert_eq!(pbe.state, [0x2004_0000, 0x0008_1a01]);
        assert_eq!(pbe.reg, [0x0200_063c_6880_0000, 0x0000_031f_0000_0257, 0]);
        assert_eq!(pbe.bytes, 1_920_000);
    }

    #[test]
    fn pbe_words_odd_width_msaa_two_dwords() {
        /* 801x601, 2 dwords from output dword 2, 4 samples, scratch at
         * 0x1000: the row is 802 pixels, linestride (802 - 1) / 2 = 400;
         * packmode U32U32 (0x28 << 7), source_pos START_BIT64 (2 << 14). */
        let pbe = pbe_state(801, 601, 2, 2, 4, 0x1000).unwrap();
        assert_eq!(pbe.state, [0x0000_0400, 0x0008_9400]);
        assert_eq!(pbe.reg, [0x0200_0640_6880_0000, 0x0000_0320_0000_0258, 0]);
        /* 802 x 601 x 4 samples x 4 B x 2 dwords */
        assert_eq!(pbe.bytes, 15_424_064);
    }

    #[test]
    fn pbe_state_rejects_what_mesa_does_not_have() {
        assert_eq!(pbe_state(800, 600, 0, 0, 1, 0), None);
        assert_eq!(pbe_state(800, 600, 8, 0, 1, 0), None);
        assert_eq!(pbe_state(800, 600, 1, 4, 1, 0), None);
    }

    #[test]
    fn scratch_size_holds_the_pr_store() {
        /* what the PR store writes fits the scratch Mesa sizes for it */
        for (w, h, s, d) in [(800, 600, 1, 1), (801, 601, 4, 2), (1600, 1200, 1, 4)] {
            assert_eq!(pbe_state(w, h, d, 0, s, 0).unwrap().bytes, scratch_size(w, h, s, d, 0));
        }
    }
}
