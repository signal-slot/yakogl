// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2019 Broadcom
// SPDX-FileCopyrightText: 2021 Raspberry Pi Ltd
//
// The limits and the tile-size choice are transcribed from Mesa 26.2.1
// src/broadcom/common/v3d_limits.h (MIT, Copyright © 2019 Broadcom) and
// v3d_util.c (MIT, Copyright © 2021 Raspberry Pi Ltd).
//! The V3D 7.1 limits and tile-size choice (roadmap P9; the Pi 5's V3D
//! 7.1.10, docs/notes/v3d71-gl-table.md item 4).
//!
//! Pure consts and functions; no `crate::` import, so a test includes this
//! file with `#[path]` (tests/v3d71_limits.rs). Nothing hooks it into
//! `src/v3d/mod.rs` yet: the 7.1 table (`backend71.rs`) does that. The 4.2
//! answers stay where they are (`backend.rs` `MAX_TARGET`, `MAX_JOB_TILES`;
//! `clear_mrt.rs` `V3D_MAX_DRAW_BUFFERS`) and are not read here.
//!
//! The sources are Mesa 26.2.1's, under
//! `third_party/mesa-26.2.1/src/broadcom/common/`:
//!
//! * `v3d_limits.h:44` `V3D_MAX_RENDER_TARGETS(ver)` = `ver < 71 ? 4 : 8`;
//! * `v3d_limits.h:63` `V3D_MAX_FRAMEBUFFER_SIZE(ver)` = `ver < 71 ? 4096u :
//!   7680u`;
//! * `v3d_limits.h:62` `V3D_MAX_IMAGE_DIMENSION` = 4096, one value for every
//!   version: what the TMU addresses;
//! * `v3d_util.c` `tile_size_valid` and the `ver >= 71` branch of
//!   `v3d_choose_tile_size`, with `V3D71_TLB_COLOR_SIZE`,
//!   `V3D71_TLB_DETPH_SIZE` and `V3D71_TLB_AUX_DETPH_SIZE`.
//!
//! The GL answers: a texture is at most 4096 a side (the TMU's limit), and
//! the GL core reports 4096 for `GL_MAX_TEXTURE_SIZE`,
//! `GL_MAX_RENDERBUFFER_SIZE` and `GL_MAX_VIEWPORT_DIMS`
//! (`src/gl/state.rs` `MAX_VIEWPORT_DIM` and the `GL_MAX_RENDERBUFFER_SIZE`
//! arm). A renderbuffer or viewport past 4096 would be a target no texture
//! of the same size can back, so 7.1 reports 4096 too; the 7680 px
//! framebuffer limit is headroom the GL core never reaches, and the 7.1
//! table's target guard reads [`MAX_FRAMEBUFFER_SIZE71`].

/// Render targets one 7.1 frame may have (`V3D_MAX_RENDER_TARGETS(71)`).
pub const MAX_RENDER_TARGETS71: u32 = 8;

/// The largest framebuffer side at 7.1, in pixels
/// (`V3D_MAX_FRAMEBUFFER_SIZE(71)`).
pub const MAX_FRAMEBUFFER_SIZE71: u32 = 7680;

/// The largest image side the TMU addresses (`V3D_MAX_IMAGE_DIMENSION`, the
/// same at 4.2 and 7.1).
pub const MAX_IMAGE_DIMENSION71: u32 = 4096;

/// `GL_MAX_TEXTURE_SIZE` at 7.1: the TMU's limit.
pub const GL_MAX_TEXTURE_SIZE71: u32 = MAX_IMAGE_DIMENSION71;

/// `GL_MAX_RENDERBUFFER_SIZE` at 7.1: a renderbuffer no larger than a
/// texture (see the module comment).
pub const GL_MAX_RENDERBUFFER_SIZE71: u32 = MAX_IMAGE_DIMENSION71;

/// `GL_MAX_VIEWPORT_DIMS` at 7.1, both sides.
pub const GL_MAX_VIEWPORT_DIMS71: [u32; 2] = [MAX_IMAGE_DIMENSION71, MAX_IMAGE_DIMENSION71];

/// The colour tile buffer, in bytes (`V3D71_TLB_COLOR_SIZE`).
pub const TLB_COLOR_SIZE71: u32 = 16 * 1024;
/// The main depth tile buffer, in bytes (`V3D71_TLB_DETPH_SIZE`).
pub const TLB_DEPTH_SIZE71: u32 = 16 * 1024;
/// The auxiliary depth tile buffer, in bytes (`V3D71_TLB_AUX_DETPH_SIZE`).
/// A depth tile that fits it frees the main depth buffer for colour.
pub const TLB_AUX_DEPTH_SIZE71: u32 = 8 * 1024;

/// The tile sizes, largest first (`v3d_choose_tile_size`'s `tile_sizes`).
pub const TILE_SIZES71: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];

/// A render target's internal bpp (the `V3D_INTERNAL_BPP_*` values 0, 1, 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bpp71 {
    Bpp32 = 0,
    Bpp64 = 1,
    Bpp128 = 2,
}

impl Bpp71 {
    /// Bytes one pixel of the target takes in the tile buffer.
    pub const fn bytes(self) -> u32 {
        match self {
            Bpp71::Bpp32 => 4,
            Bpp71::Bpp64 => 8,
            Bpp71::Bpp128 => 16,
        }
    }
}

/// Mesa's `tile_size_valid`: a tile of `pixels` fits when depth fits the
/// auxiliary buffer and colour fits colour plus main depth, or when both fit
/// their main buffers.
pub const fn tile_size_valid71(pixels: u32, color_bpp: u32, depth_bpp: u32) -> bool {
    let depth = pixels * depth_bpp;
    let color = pixels * color_bpp;
    (depth <= TLB_AUX_DEPTH_SIZE71 && color <= TLB_COLOR_SIZE71 + TLB_DEPTH_SIZE71)
        || (depth <= TLB_DEPTH_SIZE71 && color <= TLB_COLOR_SIZE71)
}

/// The tile size the `ver >= 71` branch of `v3d_choose_tile_size` picks for
/// `total_color_bytes`, the sum of every render target's bytes per pixel.
///
/// The first table entry that [`tile_size_valid71`] accepts (colour as
/// given, depth at 4 bytes, both times 4 under MSAA), one smaller for a
/// double-buffered frame. `None` where Mesa asserts: no entry fits, or a
/// double-buffered frame that already needs 8x8 tiles.
pub fn choose_tile_size71_total(total_color_bytes: u32, msaa: bool, double_buffer: bool) -> Option<(u32, u32)> {
    let samples = if msaa { 4 } else { 1 };
    let color_bpp = total_color_bytes * samples;
    let depth_bpp = 4 * samples;
    let last = TILE_SIZES71.len() - 1;
    let mut idx = 0;
    while idx < last && !tile_size_valid71(TILE_SIZES71[idx].0 * TILE_SIZES71[idx].1, color_bpp, depth_bpp) {
        idx += 1;
    }
    /* Mesa's do/while runs past the table when even 8x8 is invalid and
     * then asserts; no input in range gets there (8 x 128 bpp with MSAA
     * fits 8x8 exactly), and a larger sum is refused here. */
    if !tile_size_valid71(TILE_SIZES71[idx].0 * TILE_SIZES71[idx].1, color_bpp, depth_bpp) {
        return None;
    }
    if double_buffer {
        idx += 1;
    }
    TILE_SIZES71.get(idx).copied()
}

/// The tile size for `rts` render targets of at most `max_bpp` each, with
/// or without 4x MSAA and double buffering.
///
/// Mesa sums the targets' actual bpp; with one bpp per frame the sum is
/// `rts * max_bpp`, an upper bound that never picks a tile too large.
/// `None` for no target or more than [`MAX_RENDER_TARGETS71`], and where
/// [`choose_tile_size71_total`] refuses.
pub fn choose_tile_size71(rts: u32, max_bpp: Bpp71, msaa: bool, double_buffer: bool) -> Option<(u32, u32)> {
    if rts == 0 || rts > MAX_RENDER_TARGETS71 {
        return None;
    }
    choose_tile_size71_total(rts * max_bpp.bytes(), msaa, double_buffer)
}
