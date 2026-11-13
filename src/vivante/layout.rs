// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the 4x4-tiled and 64x64 super-tiled address maps as
//! pure functions (docs/notes/gc-tiled-targets.md, steps 2-5). [`Layout`],
//! [`padding`] and [`stride`] name the tiled shadow's and the depth surface's
//! layout (`tiled_rt.rs`, `ds_cache.rs`); the address maps are the host
//! tests' (`tests/gc_layout.rs`, the RS interpreter of `backend_tests.rs`).
//!
//! Self-contained on purpose: `core` only, no crate imports; the host test
//! `tests/gc_layout.rs` takes it from the crate as `yakogl::vivante::layout`.
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

// Case 244 `dlayout` (docs/notes/gc-ts-probe.md, "Part (ii)"): the depth ramp
// of the 64x64 draw and the decode of the surface the PE wrote.

/// Pixels along one side of case 244's target.
pub const DLAYOUT_SIDE: u32 = 64;
/// Dwords of case 244's surface (16 KiB).
pub const DLAYOUT_DWORDS: usize = (DLAYOUT_SIDE * DLAYOUT_SIDE) as usize;
/// The reference pixels whose dword offsets the page carries, in page order.
pub const DLAYOUT_REF_PIXELS: [(u32, u32); 12] =
    [(1, 0), (2, 0), (4, 0), (8, 0), (0, 1), (0, 4), (0, 8), (16, 0), (0, 16), (32, 0), (0, 32), (63, 63)];
/// An offset of [`DlayoutResult::offsets`] whose pixel was not found.
pub const DLAYOUT_NOT_FOUND: u16 = 0xffff;
/// A decoded z24 matches a pixel only when strictly closer than this.
const DLAYOUT_TOLERANCE: u32 = 1024;
/// Distance of two neighbouring ids in z24 units (2^-13 of the range).
const DLAYOUT_STEP: u32 = 2048;

/// `dlayout_decode`'s outcome (the low nibble of page A's w1; `F` is the
/// case's own HANG).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DlayoutCode {
    /// The surface is the clear value everywhere.
    NoWrite = 1,
    /// Every id at `y * 64 + x` (stride `align(64, 64) * 4`).
    RowLinear = 2,
    /// Every id at `offset(Tiled, ..)`.
    Tiled4 = 3,
    /// Every id at `offset_in(SuperTiled, Drm, ..)`.
    SuperDrm = 4,
    /// Every id at `offset_in(SuperTiled, Interleaved, ..)`.
    SuperInterleaved = 5,
    /// Each id found once, at none of the four maps.
    Other = 6,
    /// Fewer than 4096 ids found exactly once.
    Partial = 7,
}

/// What [`dlayout_decode`] read from a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DlayoutResult {
    pub code: DlayoutCode,
    /// How many of the 4096 ids were found exactly once.
    pub matched: u32,
    /// Dword offset of each of [`DLAYOUT_REF_PIXELS`] ([`DLAYOUT_NOT_FOUND`]
    /// unless the id was found exactly once).
    pub offsets: [u16; 12],
    /// CRC-32 (IEEE) of the surface bytes, little-endian dwords.
    pub crc: u32,
    /// `(dword index, z byte)` of the first dword that is not the clear
    /// value; the z byte is bits 15:8 of the dword (the low byte of z24).
    pub first_non_clear: Option<(u32, u8)>,
}

/// The 24-bit depth of pixel `(x, y)` on the plane `z = (x + 64 y) / 8192`
/// sampled at the pixel centre, rounded to 24 bits.
pub fn dlayout_expect(x: u32, y: u32) -> u32 {
    // ((x + .5) + 64 (y + .5)) / 8192 * 2^24 = (x + 64 y + 32.5) * 2048,
    // an integer.
    (x + DLAYOUT_SIDE * y) * DLAYOUT_STEP + 65 * DLAYOUT_STEP / 2
}

// The pixel id `x + 64 y` whose `dlayout_expect` is nearest to `z24`, if
// within the tolerance.
fn dlayout_id(z24: u32) -> Option<u32> {
    let base = dlayout_expect(0, 0);
    let id = (z24.saturating_sub(base) + DLAYOUT_STEP / 2) / DLAYOUT_STEP;
    if id >= DLAYOUT_DWORDS as u32 {
        return None;
    }
    let e = dlayout_expect(id % DLAYOUT_SIDE, id / DLAYOUT_SIDE);
    (z24.abs_diff(e) < DLAYOUT_TOLERANCE).then_some(id)
}

// CRC-32 (IEEE 802.3, reflected, as zlib) of `data`.
fn crc32(data: impl Iterator<Item = u8>) -> u32 {
    let mut crc = !0u32;
    for b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// Decode a D24S8 surface the PE wrote for case 244's ramp (`surface` is
/// the 64x64 surface's dwords, at least [`DLAYOUT_DWORDS`], `clear` the
/// dword it was cleared to). Dwords past the first 4096 are ignored.
pub fn dlayout_decode(surface: &[u32], clear: u32) -> DlayoutResult {
    let s = &surface[..DLAYOUT_DWORDS.min(surface.len())];
    // Per id: how often found, and the first dword index it was found at.
    let mut count = [0u8; DLAYOUT_DWORDS];
    let mut at = [0u16; DLAYOUT_DWORDS];
    for (i, &d) in s.iter().enumerate() {
        if let Some(id) = dlayout_id(d >> 8) {
            let id = id as usize;
            if count[id] == 0 {
                at[id] = i as u16;
            }
            count[id] = count[id].saturating_add(1);
        }
    }
    let matched = count.iter().filter(|&&c| c == 1).count() as u32;
    let mut offsets = [DLAYOUT_NOT_FOUND; 12];
    for (o, &(x, y)) in offsets.iter_mut().zip(DLAYOUT_REF_PIXELS.iter()) {
        let id = (x + DLAYOUT_SIDE * y) as usize;
        if count[id] == 1 {
            *o = at[id];
        }
    }
    let first_non_clear = s.iter().position(|&d| d != clear).map(|i| (i as u32, (s[i] >> 8) as u8));
    let stride = align_up(DLAYOUT_SIDE, 64) * 4;
    let fits = |map: &dyn Fn(u32, u32) -> usize| {
        (0..DLAYOUT_DWORDS as u32).all(|id| {
            let (x, y) = (id % DLAYOUT_SIDE, id / DLAYOUT_SIDE);
            count[id as usize] == 1 && at[id as usize] as usize == map(x, y)
        })
    };
    let code = if first_non_clear.is_none() {
        DlayoutCode::NoWrite
    } else if matched < DLAYOUT_DWORDS as u32 {
        DlayoutCode::Partial
    } else if fits(&|x, y| (y * stride / 4 + x) as usize) {
        DlayoutCode::RowLinear
    } else if fits(&|x, y| offset(Layout::Tiled, x, y, 4, stride) / 4) {
        DlayoutCode::Tiled4
    } else if fits(&|x, y| offset_in(Layout::SuperTiled, SuperOrder::Drm, x, y, 4, stride) / 4) {
        DlayoutCode::SuperDrm
    } else if fits(&|x, y| offset_in(Layout::SuperTiled, SuperOrder::Interleaved, x, y, 4, stride) / 4) {
        DlayoutCode::SuperInterleaved
    } else {
        DlayoutCode::Other
    };
    let bytes = s.iter().flat_map(|d| d.to_le_bytes());
    DlayoutResult { code, matched, offsets, crc: crc32(bytes), first_non_clear }
}

/// Tag of mode `mode`'s page A (page B is [`dlayout_tag_b`]).
pub const fn dlayout_tag_a(mode: u32) -> u8 {
    0xc0 + mode as u8
}

/// Tag of mode `mode`'s page B.
pub const fn dlayout_tag_b(mode: u32) -> u8 {
    0xc4 + mode as u8
}

// Two 16-bit offsets in one 32-bit half: the first in the low bits.
fn pair(o: &[u16; 12], i: usize) -> u64 {
    o[i] as u64 | (o[i + 1] as u64) << 16
}

/// The words `w1..w3` of mode `mode`'s page A.
///
/// The note's table puts four offsets (two pairs) in each of w2 and w3, so
/// the record words are taken as 64 bits wide: a pair of 16-bit offsets is
/// the low half (first offset low), the next pair `<< 32`. w1 is `code |
/// mode << 4 | timeouts << 8 | matched << 16`; `timeouts` is 8 bits.
pub fn dlayout_page_a(mode: u32, timeouts: u32, r: &DlayoutResult) -> [u64; 3] {
    let w1 = r.code as u64 | (mode as u64 & 0xf) << 4 | (timeouts as u64 & 0xff) << 8 | (r.matched as u64) << 16;
    [w1, pair(&r.offsets, 0) | pair(&r.offsets, 2) << 32, pair(&r.offsets, 4) | pair(&r.offsets, 6) << 32]
}

/// The words `w1..w3` of mode `mode`'s page B: the CRC, the offsets of
/// (0,16), (32,0) and (0,32), (63,63) packed as in [`dlayout_page_a`], and
/// the first non-clear dword (`index << 16 | z byte`; index `0xffff` if
/// there is none).
pub fn dlayout_page_b(r: &DlayoutResult) -> [u64; 3] {
    let (i, z) = r.first_non_clear.unwrap_or((0xffff, 0));
    [r.crc as u64, pair(&r.offsets, 8) | pair(&r.offsets, 10) << 32, (i as u64) << 16 | z as u64]
}

#[cfg(test)]
mod dlayout_tests {
    use super::*;

    const CLEAR: u32 = 0xffff_ff00;

    fn ramp(map: &dyn Fn(u32, u32) -> usize) -> Vec<u32> {
        let mut s = vec![CLEAR; DLAYOUT_DWORDS];
        for y in 0..64 {
            for x in 0..64 {
                s[map(x, y)] = dlayout_expect(x, y) << 8;
            }
        }
        s
    }

    fn row(x: u32, y: u32) -> usize {
        (y * 64 + x) as usize
    }
    fn tiled(x: u32, y: u32) -> usize {
        offset(Layout::Tiled, x, y, 4, 256) / 4
    }
    fn drm(x: u32, y: u32) -> usize {
        offset_in(Layout::SuperTiled, SuperOrder::Drm, x, y, 4, 256) / 4
    }
    fn inter(x: u32, y: u32) -> usize {
        offset_in(Layout::SuperTiled, SuperOrder::Interleaved, x, y, 4, 256) / 4
    }

    #[test]
    fn expect_values() {
        assert_eq!(dlayout_expect(0, 0), 65 * 1024);
        assert_eq!(dlayout_expect(1, 0) - dlayout_expect(0, 0), 2048);
        assert_eq!(dlayout_expect(0, 1) - dlayout_expect(0, 0), 64 * 2048);
        // z stays near [0, 0.5): the centre adds 32.5 / 8192
        assert!(dlayout_expect(63, 63) < (1 << 23) + (1 << 17));
    }

    fn check_map(map: &dyn Fn(u32, u32) -> usize, code: DlayoutCode) {
        let s = ramp(map);
        let r = dlayout_decode(&s, CLEAR);
        assert_eq!(r.code, code);
        assert_eq!(r.matched, 4096);
        for (o, &(x, y)) in r.offsets.iter().zip(DLAYOUT_REF_PIXELS.iter()) {
            assert_eq!(*o as usize, map(x, y));
        }
        assert_eq!(r.first_non_clear.map(|(i, _)| i as usize), s.iter().position(|&d| d != CLEAR));
    }

    #[test]
    fn code1_no_write() {
        let s = vec![CLEAR; DLAYOUT_DWORDS];
        let r = dlayout_decode(&s, CLEAR);
        assert_eq!(r.code, DlayoutCode::NoWrite);
        assert_eq!(r.matched, 0);
        assert_eq!(r.first_non_clear, None);
        assert_eq!(r.offsets, [DLAYOUT_NOT_FOUND; 12]);
    }

    #[test]
    fn code2_row_linear() {
        check_map(&row, DlayoutCode::RowLinear);
    }

    #[test]
    fn code3_tiled4() {
        check_map(&tiled, DlayoutCode::Tiled4);
    }

    #[test]
    fn code4_super_drm() {
        check_map(&drm, DlayoutCode::SuperDrm);
    }

    #[test]
    fn code5_super_interleaved() {
        check_map(&inter, DlayoutCode::SuperInterleaved);
    }

    #[test]
    fn code6_other() {
        // A permutation of the row-linear ramp that is none of the maps.
        let other = |x: u32, y: u32| row((x + 1) % 64, y);
        let s = ramp(&other);
        let r = dlayout_decode(&s, CLEAR);
        assert_eq!(r.code, DlayoutCode::Other);
        assert_eq!(r.matched, 4096);
        assert_eq!(r.offsets[0] as usize, other(1, 0));
    }

    #[test]
    fn code7_partial() {
        let mut s = ramp(&row);
        s[row(5, 5)] = CLEAR; // one id missing
        let r = dlayout_decode(&s, CLEAR);
        assert_eq!(r.code, DlayoutCode::Partial);
        assert_eq!(r.matched, 4095);
        let mut s = ramp(&row);
        s[row(5, 5)] = s[row(6, 5)]; // one id twice, one missing
        let r = dlayout_decode(&s, CLEAR);
        assert_eq!(r.code, DlayoutCode::Partial);
        assert_eq!(r.matched, 4094);
        assert_eq!(r.offsets[11] as usize, row(63, 63));
    }

    #[test]
    fn decode_tolerance() {
        let mut s = ramp(&row);
        s[row(3, 3)] = (dlayout_expect(3, 3) + 1000) << 8 | 0xff;
        assert_eq!(dlayout_decode(&s, CLEAR).code, DlayoutCode::RowLinear);
        s[row(3, 3)] = (dlayout_expect(3, 3) + 1024) << 8;
        assert_eq!(dlayout_decode(&s, CLEAR).code, DlayoutCode::Partial);
    }

    #[test]
    fn crc_and_pages() {
        // CRC-32 of the ASCII digits 1 to 9 is the standard check value.
        assert_eq!(crc32(b"123456789".iter().copied()), 0xcbf4_3926);
        let s = ramp(&row);
        let r = dlayout_decode(&s, CLEAR);
        let bytes: Vec<u8> = s.iter().flat_map(|d| d.to_le_bytes()).collect();
        assert_eq!(r.crc, crc32(bytes.into_iter()));
        let z0 = dlayout_expect(0, 0) as u8;
        assert_eq!(r.first_non_clear, Some((0, z0)));

        let a = dlayout_page_a(1, 3, &r);
        assert_eq!(a[0], 2 | 1 << 4 | 3 << 8 | 4096 << 16);
        // (1,0) (2,0) (4,0) (8,0)
        assert_eq!(a[1], 1 | 2 << 16 | 4 << 32 | 8 << 48);
        // (0,1) (0,4) (0,8) (16,0)
        assert_eq!(a[2], 64 | 256 << 16 | 512 << 32 | 16 << 48);
        let b = dlayout_page_b(&r);
        assert_eq!(b[0], r.crc as u64);
        // (0,16) (32,0) (0,32) (63,63)
        assert_eq!(b[1], 1024 | 32 << 16 | 2048 << 32 | 4095 << 48);
        assert_eq!(b[2], z0 as u64);
        assert_eq!((dlayout_tag_a(2), dlayout_tag_b(2)), (0xc2, 0xc6));

        let none = dlayout_decode(&vec![CLEAR; DLAYOUT_DWORDS], CLEAR);
        assert_eq!(dlayout_page_b(&none)[2], 0xffff << 16);
    }
}
