// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Case 244 `dlayout` (docs/notes/gc-ts-probe.md, "Part (ii)"): the depth
//! surface's override plan and the dump step, as pure functions.
//!
//! The hook `gc_debug_depth_dump(mode, out)` (a later item) draws a 64x64
//! depth ramp under one of four layout modes. [`plan`] says what the mode
//! asks of the PE (`PE_LOGIC_OP` SINGLE_BUFFER and `PE_DEPTH_CONFIG`
//! SUPER_TILED) and [`dump`] turns the bytes read back into the page words
//! through [`layout::dlayout_decode`]. Nothing here touches the GPU.

use super::layout::{self, DlayoutResult, Layout};
use super::regs::{
    pe_depth_func, pe_logic_op_single_buffer, GC_PE_DEPTH_FORMAT_D24S8, GC_PE_DEPTH_MODE_Z, GC_PE_DEPTH_SUPER_TILED,
    GC_PE_DEPTH_UNK18, GC_PE_DEPTH_WRITE_ENABLE,
};

/// Number of layout modes, one per boot.
pub const MODES: u32 = 4;
/// Hardware encoding of the compare function GL_ALWAYS (`PE_DEPTH_CONFIG`).
/// The code `F` of page A's w1 (a fence timeout), not in `DlayoutCode`.
pub const HANG: u64 = 0xf;
const DEPTH_FUNC_ALWAYS: u32 = 7;

/// What one mode asks of the depth surface of the ramp draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthOverride {
    /// The mode, 0..=3 (the page tag's low bits).
    pub mode: u32,
    /// The value of `PE_LOGIC_OP`'s SINGLE_BUFFER field.
    pub single_buffer: u32,
    /// Whether `PE_DEPTH_CONFIG` carries `GC_PE_DEPTH_SUPER_TILED`.
    pub super_tiled: bool,
}

/// The override of `mode` (the note's table), `None` past mode 3.
pub const fn plan(mode: u32) -> Option<DepthOverride> {
    let (single_buffer, super_tiled) = match mode {
        0 => (2, false),
        1 => (2, true),
        2 => (1, true),
        3 => (1, false),
        _ => return None,
    };
    Some(DepthOverride { mode, single_buffer, super_tiled })
}

impl DepthOverride {
    /// `PE_DEPTH_CONFIG` of the ramp draw: D24S8, GL_ALWAYS, depth writes
    /// on, no early depth, no DISABLE_ZS, plus SUPER_TILED when asked.
    pub const fn depth_config(&self) -> u32 {
        GC_PE_DEPTH_MODE_Z
            | GC_PE_DEPTH_FORMAT_D24S8
            | GC_PE_DEPTH_UNK18
            | pe_depth_func(DEPTH_FUNC_ALWAYS)
            | GC_PE_DEPTH_WRITE_ENABLE
            | if self.super_tiled { GC_PE_DEPTH_SUPER_TILED } else { 0 }
    }

    /// The SINGLE_BUFFER field of `PE_LOGIC_OP`, in place (OR it into the
    /// register's other bits).
    pub const fn logic_op_bits(&self) -> u32 {
        pe_logic_op_single_buffer(self.single_buffer)
    }

    /// The layout the surface is declared with: super-tiled when
    /// SUPER_TILED is on, else the row-linear one the PE has written so far.
    pub const fn layout(&self) -> Layout {
        if self.super_tiled {
            Layout::SuperTiled
        } else {
            Layout::Linear
        }
    }

    /// Bytes of the surface the draw covers (64x64 D24S8, 4 bytes a pixel).
    pub const fn surface_bytes(&self) -> usize {
        layout::DLAYOUT_DWORDS * 4
    }
}

/// What [`dump`] returns for one mode: the decode and the pages' words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dump {
    pub result: layout::DlayoutResult,
    /// Tag and words of page A, `0xc0 + mode`.
    pub page_a: (u8, [u64; 3]),
    /// Tag and words of page B, `0xc4 + mode`.
    pub page_b: (u8, [u64; 3]),
}

/// Decodes the surface bytes read back after the ramp draw: `bytes` is the
/// surface in memory order (little-endian dwords, at least 16 KiB), `clear`
/// the value the clear quad wrote, `timeouts` the fence timeouts counted.
/// A timeout makes the code HANG, as in the C twin. `None` for a mode past 3 or a short readback.
pub fn dump(mode: u32, bytes: &[u8], clear: u32, timeouts: u32) -> Option<Dump> {
    plan(mode)?;
    if bytes.len() < layout::DLAYOUT_DWORDS * 4 {
        return None;
    }
    let dwords: Vec<u32> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let result: DlayoutResult = layout::dlayout_decode(&dwords, clear);
    // The C twin (pd_pack_a) clamps the timeouts to 255 and makes any of
    // them the code HANG (0xf); the case writes that code itself.
    let timeouts = timeouts.min(255);
    let mut a = layout::dlayout_page_a(mode, timeouts, &result);
    if timeouts != 0 {
        a[0] |= HANG;
    }
    let page_a = (layout::dlayout_tag_a(mode), a);
    let page_b = (layout::dlayout_tag_b(mode), layout::dlayout_page_b(&result));
    Some(Dump { result, page_a, page_b })
}
