// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the 4x4-tiled and 64x64 super-tiled address maps as
//! pure functions (docs/notes/gc-tiled-targets.md, steps 2-5). **Not wired**:
//! not yet in `mod.rs`, nothing in the backend calls it.
//!
//! Self-contained on purpose: `core` only, no crate imports, so a host test
//! can include it with `#[path]` (tests/gc_layout.rs) before it is joined to
//! the module tree.
//!
//! # Sources
//!
//! Mesa 26.2.1 (the tree `tools/vivante/README.md` builds `etna_shc` in; the
//! vendored `third_party/mesa-26.2.1` holds no etnaviv), read for this file:
//!
//! - `src/gallium/drivers/etnaviv/etnaviv_tiling.c:36-64` (`DO_TILE`,
//!   `DO_UNTILE`): a 4x4 tile is 16 consecutive pixels, row-major inside the
//!   tile; the tiles of one tile row follow each other; a tile row is
//!   `4 * stride` bytes.
//! - `etnaviv_rs.c:626-652` (`etna_compute_tileoffset`): where a tile or a
//!   super-tile starts. TILED `(y & ~3) * stride + bpp * ((x & ~3) << 2)`
//!   (`:642`), SUPER_TILED `(y & ~63) * stride + bpp * ((x & ~63) << 6)`
//!   (`:649`): a super-tile is `64 * 64 * bpp` contiguous bytes.
//! - `etnaviv_resource.c:327-331` (`setup_miptree`): padded width and height
//!   `align(w, paddingX)`, `align(h, paddingY)`; stride `padded_width * bpp`;
//!   layer stride `stride * padded_height`.
//! - `etnaviv_resource.c:352-407` (`etna_layout_multiple`): the paddings,
//!   `rs_align` at `:361-362`.
//! - `include/drm-uapi/drm_fourcc.h:855-865` (`DRM_FORMAT_MOD_VIVANTE_SUPER_TILED`):
//!   inside a 64x64 super-tile, "8x4 groups of 2x4 tiles of 4x4 pixels
//!   (like above) each, all in row-major layout". Mesa itself has no CPU map
//!   of a super-tile's inside (`etnaviv_transfer.c:514`, "TODO supertiling").
//!
//! # Which variants this part uses
//!
//! - **No split (multi-pipe) layout.** The GC7000UL has one pixel pipe
//!   (hwdb `NumPixelPipes` 1) and SINGLE_BUFFER, so `pe_multitiled` is false
//!   (`etnaviv_screen.c:978-979`) and `etna_resource_alloc` never adds
//!   `ETNA_LAYOUT_BIT_MULTI` (`etnaviv_resource.c:572-588`). MULTI_TILED and
//!   MULTI_SUPERTILED (`DRM_FORMAT_MOD_VIVANTE_SPLIT_*`, height padded by
//!   the pipe count, y halved, `etnaviv_rs.c:637-646`) are not implemented.
//! - **Super-tiled is the SUPER_TILED_NEW kind.** This part has
//!   CACHE128B256BPERLINE (docs/notes/gc-tiled-targets.md, hwdb table), so
//!   etnaviv sets `*_SUPER_TILED_NEW` next to `SUPER_TILED` on every
//!   super-tiled PE and RS surface (`etnaviv_state.c:296`, `:336`,
//!   `etnaviv_rs.c:108-110`, `:134-135`). Neither Mesa nor drm_fourcc.h says
//!   whether the NEW bit changes the order of pixels inside a super-tile.
//!   [`SuperOrder`] therefore keeps both candidates: [`SuperOrder::Drm`], the
//!   documented one and the default, and [`SuperOrder::Interleaved`], a
//!   hypothesis (x and y bits alternating from the 4x4 tile up; it is how the
//!   Vivante driver is recalled to address its "mode 2" super-tiles, a
//!   recollection, not a source read here). A board dump of a super-tiled
//!   target (gt-tiled-probe) settles which one is the GC7000UL's; only
//!   [`super_index`] needs to change.
//!
//! Strides are bytes between two pixel rows of the *padded linear* surface,
//! as in Mesa: [`stride`]. In a tiled layout four rows (one tile row) are
//! `4 * stride` bytes apart, in a super-tiled one 64 rows are `64 * stride`.

/// Pixels along one side of a 4x4 tile.
pub const TILE: u32 = 4;
/// Pixels along one side of a super-tile.
pub const SUPER_TILE: u32 = 64;

/// The memory layout of a 2D surface (`enum etna_surface_layout`, single
/// pipe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Row after row (ETNA_LAYOUT_LINEAR).
    Linear,
    /// 4x4 pixel tiles, row-major (ETNA_LAYOUT_TILED).
    Tiled,
    /// 64x64 pixel super-tiles of 4x4 tiles (ETNA_LAYOUT_SUPER_TILED).
    SuperTiled,
}

/// The order of the 4096 pixels inside one 64x64 super-tile (see the module
/// doc: which one the GC7000UL writes under SUPER_TILED_NEW is unmeasured).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuperOrder {
    /// `drm_fourcc.h:855-865`: 8x4 groups of 2x4 tiles, each level
    /// row-major. Pixel index bits, low to high: x0 x1 y0 y1 | x2 | y2 y3 |
    /// x3 x4 x5 | y4 y5.
    Drm,
    /// Hypothesis: x and y alternate from the tile up. Bits, low to high:
    /// x0 x1 y0 y1 | x2 y2 x3 y3 x4 y4 x5 y5.
    Interleaved,
}

/// The order [`offset`] uses.
pub const SUPER_ORDER: SuperOrder = SuperOrder::Drm;

/// `value` rounded up to a multiple of `to` (`to` > 0).
pub const fn align_up(value: u32, to: u32) -> u32 {
    value.div_ceil(to) * to
}

/// The width and height padding of a level in pixels, `etna_layout_multiple`
/// (`etnaviv_resource.c:378-393`) for one sample and no BLT engine (the
/// GC7000UL has none, so `paddingY` of LINEAR is 4).
///
/// `rs_align` is Mesa's flag of the same name (`:361-362`): the resolve
/// engine may touch the surface, so a row is a multiple of 16 pixels.
/// Without the BLT it holds for every render target, and on a TEXTURE_HALIGN
/// core (this one) for every texture. LINEAR and TILED: `(rs ? 16 : 4, 4)`;
/// SUPER_TILED: `(64, 64)` whatever `rs_align`.
pub const fn padding(layout: Layout, rs_align: bool) -> (u32, u32) {
    match layout {
        Layout::Linear | Layout::Tiled => (if rs_align { 16 } else { TILE }, TILE),
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

/// The size in bytes of one level, Mesa's `layer_stride`
/// (`etnaviv_resource.c:331`): the stride times the padded height.
pub const fn size(layout: Layout, width: u32, height: u32, bpp: u32, rs_align: bool) -> usize {
    stride(layout, width, bpp, rs_align) as usize * aligned_height(layout, height, rs_align) as usize
}

/// The index (0..16) of pixel `(x, y)` inside its 4x4 tile
/// (`etnaviv_tiling.c:41-47`): row-major.
pub const fn tile_index(x: u32, y: u32) -> u32 {
    (y & 3) * TILE + (x & 3)
}

/// The index (0..4096) of pixel `(x, y)` inside its 64x64 super-tile, in
/// pixels; only the low six bits of `x` and `y` count.
pub const fn super_index(order: SuperOrder, x: u32, y: u32) -> u32 {
    let (x, y) = (x & 63, y & 63);
    tile_index(x, y)
        | match order {
            // drm_fourcc.h: tile column in the 2x4 group (x2), tile row in
            // the group (y2 y3), group column of 8 (x3 x4 x5), group row
            // of 4 (y4 y5).
            SuperOrder::Drm => ((x & 0x04) << 2) | ((y & 0x0c) << 3) | ((x & 0x38) << 4) | ((y & 0x30) << 6),
            SuperOrder::Interleaved => {
                ((x & 0x04) << 2)
                    | ((y & 0x04) << 3)
                    | ((x & 0x08) << 3)
                    | ((y & 0x08) << 4)
                    | ((x & 0x10) << 4)
                    | ((y & 0x10) << 5)
                    | ((x & 0x20) << 5)
                    | ((y & 0x20) << 6)
            }
        }
}

/// The byte offset of pixel `(x, y)` in a surface of `layout` whose padded
/// linear row is `stride` bytes (see [`stride`]), `bpp` bytes a pixel, with
/// super-tiles in [`SUPER_ORDER`].
pub const fn offset(layout: Layout, x: u32, y: u32, bpp: u32, stride: u32) -> usize {
    offset_in(layout, SUPER_ORDER, x, y, bpp, stride)
}

/// [`offset`] with the super-tile order given.
///
/// LINEAR: `y * stride + x * bpp`. TILED: Mesa's tile start
/// `(y & ~3) * stride + bpp * ((x & ~3) << 2)` (`etnaviv_rs.c:642`) plus
/// `bpp * tile_index`. SUPER_TILED: Mesa's super-tile start
/// `(y & ~63) * stride + bpp * ((x & ~63) << 6)` (`etnaviv_rs.c:649`) plus
/// `bpp * super_index`.
pub const fn offset_in(layout: Layout, order: SuperOrder, x: u32, y: u32, bpp: u32, stride: u32) -> usize {
    let (xs, ys, b, s) = (x as usize, y as usize, bpp as usize, stride as usize);
    match layout {
        Layout::Linear => ys * s + xs * b,
        Layout::Tiled => (ys & !3) * s + b * ((xs & !3) << 2) + b * tile_index(x, y) as usize,
        Layout::SuperTiled => (ys & !63) * s + b * ((xs & !63) << 6) + b * super_index(order, x, y) as usize,
    }
}

/// Copy the `width` x `height` pixels of a linear surface into `layout`.
///
/// `src` rows are `src_stride` bytes apart; `dst` is a [`size`]d surface
/// with row stride `dst_stride` ([`stride`]; for `Layout::Linear` any stride
/// of at least `width * bpp`). Padding bytes of `dst` keep their value.
/// Panics if `bpp` is 0, if `src_stride` is below one row, or if a slice is
/// too short for the area.
#[allow(clippy::too_many_arguments)]
pub fn linear_to_layout(
    layout: Layout,
    order: SuperOrder,
    dst: &mut [u8],
    dst_stride: u32,
    src: &[u8],
    src_stride: u32,
    width: u32,
    height: u32,
    bpp: u32,
) {
    let n = check(src_stride, width, bpp);
    for_each_pixel(layout, order, dst_stride, src_stride, width, height, bpp, |laid, lin| {
        dst[laid..laid + n].copy_from_slice(&src[lin..lin + n]);
    });
}

/// The inverse of [`linear_to_layout`]: read the `width` x `height` pixels
/// of a `layout` surface (`src`, row stride `src_stride`) into linear `dst`
/// (row stride `dst_stride`). Bytes of `dst` past `width * bpp` in a row
/// keep their value.
#[allow(clippy::too_many_arguments)]
pub fn layout_to_linear(
    layout: Layout,
    order: SuperOrder,
    dst: &mut [u8],
    dst_stride: u32,
    src: &[u8],
    src_stride: u32,
    width: u32,
    height: u32,
    bpp: u32,
) {
    let n = check(dst_stride, width, bpp);
    for_each_pixel(layout, order, src_stride, dst_stride, width, height, bpp, |laid, lin| {
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
#[allow(clippy::too_many_arguments)]
fn for_each_pixel(
    layout: Layout,
    order: SuperOrder,
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
            f(offset_in(layout, order, x, y, bpp, laid_stride), lin);
        }
    }
}
