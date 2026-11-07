// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the 4x4-tiled and 64x64 super-tiled address maps as
//! pure functions (docs/notes/gc-tiled-targets.md, steps 2-5).
//!
//! Self-contained on purpose: it uses `core` only, so a host test can include
//! it with `#[path]` before it is joined to the module tree.
//!
//! Sources: Mesa 26.2.1 etnaviv, `src/gallium/drivers/etnaviv/`:
//!
//! - `etnaviv_tiling.c:36-64` (`DO_TILE`/`DO_UNTILE`): a 4x4 tile is 16
//!   consecutive pixels, row-major inside the tile; tiles of one tile row
//!   follow each other; a tile row is `4 * stride` bytes.
//! - `etnaviv_rs.c:626-652` (`etna_compute_tileoffset`): the byte offset of a
//!   tile-aligned corner. TILED is `(y & ~3) * stride + bpp * ((x & ~3) << 2)`,
//!   SUPER_TILED is `(y & ~63) * stride + bpp * ((x & ~63) << 6)`.
//! - `etnaviv_resource.c:327-329` (`setup_miptree`): the padded width is
//!   `align(width, paddingX)`, the stride is `padded_width * bpp`.
//! - `etnaviv_resource.c:352-407` (`etna_layout_multiple`): the paddings.
//!
//! # What is and is not here
//!
//! - **LINEAR, TILED, SUPER_TILED** (the `Layout` enum). The GC7000UL
//!   (model 0x7000, rev 0x6204) has one pixel pipe (`NumPixelPipes` 1, hwdb
//!   `gc_feature_database.h:18866`), so etnaviv never picks its
//!   MULTI_TILED/MULTI_SUPERTILED (split) layouts for this part: they pad
//!   the height to `4 * pixel_pipes` / `64 * pixel_pipes` and halve the y
//!   offset (`etnaviv_resource.c:394-403`, `etnaviv_rs.c:637-646`). They are
//!   **not** implemented here.
//! - **Inside a super-tile Mesa has no CPU address map**
//!   (`etnaviv_transfer.c:514`, "TODO supertiling"). Mesa only fixes where a
//!   64x64 super-tile starts: super-tiles are `64 * 64 * bpp` contiguous
//!   bytes, a row of them after another (`etnaviv_rs.c:649`). The order of
//!   the sixteen-by-sixteen 4x4 tiles inside one is **assumed row-major here
//!   (`SUPER_INNER_ROW_MAJOR`), not measured**: gt-tiled-probe is to dump a
//!   super-tiled target and settle it. Change [`super_inner_tile_index`] if
//!   the dump says otherwise; nothing else needs to move.
//!
//! Strides are bytes between two pixel rows of the *linear* padded surface,
//! as in Mesa: `stride(layout, width, bpp, rs_align)`. In a tiled layout the
//! same number is the distance between two pixels one tile row apart divided
//! by four.

/// Pixels along one side of a 4x4 tile.
pub const TILE: u32 = 4;
/// Pixels along one side of a super-tile.
pub const SUPER_TILE: u32 = 64;
/// 4x4 tiles along one side of a super-tile.
pub const TILES_PER_SUPER: u32 = SUPER_TILE / TILE;
/// Whether the 4x4 tiles inside a super-tile are assumed row-major (see the
/// module doc: unmeasured).
pub const SUPER_INNER_ROW_MAJOR: bool = true;

/// The memory layout of a 2D surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Row after row.
    Linear,
    /// 4x4 pixel tiles (ETNA_LAYOUT_TILED).
    Tiled,
    /// 64x64 pixel super-tiles of 4x4 tiles (ETNA_LAYOUT_SUPER_TILED).
    SuperTiled,
}

/// `value` rounded up to a multiple of `to` (`to` > 0).
pub const fn align_up(value: u32, to: u32) -> u32 {
    value.div_ceil(to) * to
}

/// The width and height padding of a level in pixels, `etna_layout_multiple`
/// (`etnaviv_resource.c:380-393`) for one sample and one pixel pipe.
///
/// `rs_align` is Mesa's flag of the same name (`:361-362`): the resolve
/// engine may touch the surface, so a row is a multiple of 16 pixels. On the
/// GC7000UL (no BLT engine) it is true for every render target, and for every
/// texture on a TEXTURE_HALIGN core. LINEAR: `(rs ? 16 : 4, 4)`, TILED:
/// `(rs ? 16 : 4, 4)`, SUPER_TILED: `(64, 64)`.
pub const fn padding(layout: Layout, rs_align: bool) -> (u32, u32) {
    match layout {
        Layout::Linear | Layout::Tiled => (if rs_align { 16 } else { 4 }, 4),
        Layout::SuperTiled => (SUPER_TILE, SUPER_TILE),
    }
}

/// The padded width in pixels (`etnaviv_resource.c:327`).
pub const fn aligned_width(layout: Layout, width: u32, rs_align: bool) -> u32 {
    align_up(width, padding(layout, rs_align).0)
}

/// The padded height in pixels (`etnaviv_resource.c:328`).
pub const fn aligned_height(layout: Layout, height: u32, rs_align: bool) -> u32 {
    align_up(height, padding(layout, rs_align).1)
}

/// The row stride in bytes of a level `width` pixels wide
/// (`etnaviv_resource.c:329`): the padded width times `bpp`.
pub const fn stride(layout: Layout, width: u32, bpp: u32, rs_align: bool) -> u32 {
    aligned_width(layout, width, rs_align) * bpp
}

/// The size in bytes of one level (`etnaviv_resource.c:330-332`,
/// `layer_stride`): the stride times the padded height.
pub const fn size(layout: Layout, width: u32, height: u32, bpp: u32, rs_align: bool) -> usize {
    stride(layout, width, bpp, rs_align) as usize * aligned_height(layout, height, rs_align) as usize
}

/// The index of the 4x4 tile `(tx, ty)` (0..16 each) inside a super-tile.
/// Row-major by assumption, see the module doc.
pub const fn super_inner_tile_index(tx: u32, ty: u32) -> u32 {
    ty * TILES_PER_SUPER + tx
}

/// The byte offset of pixel `(x, y)` in a surface of the given layout whose
/// padded linear row is `stride` bytes (see [`stride`]); `bpp` bytes a pixel.
///
/// LINEAR: `y * stride + x * bpp`. TILED: Mesa's
/// `(y & ~3) * stride + bpp * ((x & ~3) << 2)` (`etnaviv_rs.c:642`) plus the
/// pixel inside the tile, `bpp * ((y & 3) * 4 + (x & 3))`
/// (`etnaviv_tiling.c:41-47`). SUPER_TILED: Mesa's
/// `(y & ~63) * stride + bpp * ((x & ~63) << 6)` (`etnaviv_rs.c:649`) plus
/// the 4x4 tile's place in the super-tile (assumed row-major) and the pixel
/// inside the tile.
pub const fn offset(layout: Layout, x: u32, y: u32, bpp: u32, stride: u32) -> usize {
    let (x, y, bpp, stride) = (x as usize, y as usize, bpp as usize, stride as usize);
    match layout {
        Layout::Linear => y * stride + x * bpp,
        Layout::Tiled => (y & !3) * stride + bpp * ((x & !3) << 2) + bpp * ((y & 3) * 4 + (x & 3)),
        Layout::SuperTiled => {
            let base = (y & !63) * stride + bpp * ((x & !63) << 6);
            let tile = super_inner_tile_index(((x & 63) >> 2) as u32, ((y & 63) >> 2) as u32) as usize;
            base + bpp * (tile * 16 + (y & 3) * 4 + (x & 3))
        }
    }
}

/// Copy the `width` x `height` pixels of a linear surface into `layout`.
///
/// `src` rows are `src_stride` bytes apart; `dst` is a [`size`]d surface
/// with row stride `dst_stride` (use [`stride`]; for `Layout::Linear` any
/// stride at least `width * bpp`). Padding bytes of `dst` are left as they
/// were. Panics if a slice is too short for the area it is asked to cover.
#[allow(clippy::too_many_arguments)]
pub fn linear_to_layout(
    layout: Layout,
    dst: &mut [u8],
    dst_stride: u32,
    src: &[u8],
    src_stride: u32,
    width: u32,
    height: u32,
    bpp: u32,
) {
    let n = check(src_stride, width, bpp);
    for_each_pixel(layout, dst_stride, src_stride, width, height, bpp, |laid, lin| {
        dst[laid..laid + n].copy_from_slice(&src[lin..lin + n]);
    });
}

/// The inverse of [`linear_to_layout`]: read `width` x `height` pixels of a
/// `layout` surface (`src`, row stride `src_stride`) into linear `dst`
/// (row stride `dst_stride`).
#[allow(clippy::too_many_arguments)]
pub fn layout_to_linear(
    layout: Layout,
    dst: &mut [u8],
    dst_stride: u32,
    src: &[u8],
    src_stride: u32,
    width: u32,
    height: u32,
    bpp: u32,
) {
    let n = check(dst_stride, width, bpp);
    for_each_pixel(layout, src_stride, dst_stride, width, height, bpp, |laid, lin| {
        dst[lin..lin + n].copy_from_slice(&src[laid..laid + n]);
    });
}

fn check(lin_stride: u32, width: u32, bpp: u32) -> usize {
    assert!(bpp > 0, "layout: bytes per pixel must be non-zero");
    assert!(lin_stride as usize >= width as usize * bpp as usize, "layout: linear stride below one row");
    bpp as usize
}

// Calls `f(offset in the laid-out surface, offset in the linear one)` for
// every pixel of the area, row by row.
fn for_each_pixel(
    layout: Layout,
    laid_stride: u32,
    lin_stride: u32,
    width: u32,
    height: u32,
    bpp: u32,
    mut f: impl FnMut(usize, usize),
) {
    for y in 0..height {
        for x in 0..width {
            let lin = y as usize * lin_stride as usize + x as usize * bpp as usize;
            f(offset(layout, x, y, bpp, laid_stride), lin);
        }
    }
}
