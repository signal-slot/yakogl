// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth surface format and size of a GL depth attachment, as a pure
//! function (docs/notes/gc-slow-terrain-stream-diff.md, rank 6).
//!
//! Terrain's `DEPTH_COMPONENT16` renderbuffer is stored and tested as D24S8
//! today (stride 0xd00 at 800 wide, Mesa's 0x680): twice the depth bytes in
//! the terrain draw, its clear and the overlay draw. This file says what the
//! surface should be; nothing calls it yet. Core only, no crate imports.
//!
//! Mesa 26.2.1 etnaviv, which this follows (line numbers are those of the
//! tree the notes cite; the etnaviv driver is not vendored, so they are not
//! checkable from this repository):
//! - `translate_depth_format` (etnaviv_translate.h): `Z16_UNORM` is
//!   `ETNA_DEPTH_FORMAT D16`; `Z24X8`, `Z24_UNORM_S8_UINT` and `S8_UINT`
//!   (no D16 stencil form exists) are `D24S8`.
//! - `etna_resource` sizing (etnaviv_resource.c, `etna_resource_alloc`):
//!   stride = align(width, layout padding) * bytes a pixel, size =
//!   stride * align(height, layout padding), the padding of the layout
//!   (4x4 tiled 4x4, super-tiled 64x64 here).
//! - `translate_clear_depth_stencil` (etnaviv_translate.h): D16 clears with
//!   `depth * 0xffff` in both halves of the word; D24S8 with
//!   `(depth * 0xffffff) << 8 | stencil`.
//!
//! Consumers that must change before the format can switch (all still assume
//! 4 bytes a pixel and D24S8):
//! - `ds_cache.rs`: `select` sizes every surface `align64(w) * 4` and keeps
//!   the plane bookkeeping (`stride`, `rows`, retention) in those units;
//!   `resolve_depth` and `depth-sample-gc` read 4-byte words
//!   (`resolve_read_bytes` below says 2 for D16);
//! - the depth sample path that turns a surface into a depth texture;
//! - `backend.rs`: `GC_PE_DEPTH_FORMAT_D24S8` in `PE_DEPTH_CONFIG`, the
//!   surface's stride/size registers, and the clear's value;
//! - polygon offset: `GC_SE_DEPTH_BIAS` is scaled by 1 / 16777215 (24 bits);
//!   a 16-bit buffer needs 1 / 65535.

/// What the pixel engine is told the depth buffer holds. The caller maps it
/// to `GC_PE_DEPTH_FORMAT_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeDepthFormat {
    D16,
    D24S8,
}

/// A GL depth/stencil attachment format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlDepthAttachment {
    /// No depth or stencil attachment.
    None,
    DepthComponent16,
    DepthComponent24,
    Depth24Stencil8,
    /// `STENCIL_INDEX8` alone.
    StencilIndex8,
}

/// What a depth attachment is stored as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthSurface {
    pub format: PeDepthFormat,
    /// Bytes a pixel in memory.
    pub bytes_per_pixel: u32,
    /// Bytes a depth resolve or depth sample must read a pixel: 2 for D16,
    /// 4 for D24S8.
    pub resolve_read_bytes: u32,
}

const fn align_up(v: u32, to: u32) -> u32 {
    v.div_ceil(to) * to
}

/// The surface for an attachment; `None` for no attachment (nothing to
/// allocate).
pub const fn depth_surface(a: GlDepthAttachment) -> Option<DepthSurface> {
    match a {
        GlDepthAttachment::None => None,
        GlDepthAttachment::DepthComponent16 => {
            Some(DepthSurface { format: PeDepthFormat::D16, bytes_per_pixel: 2, resolve_read_bytes: 2 })
        }
        GlDepthAttachment::DepthComponent24 | GlDepthAttachment::Depth24Stencil8 | GlDepthAttachment::StencilIndex8 => {
            Some(DepthSurface { format: PeDepthFormat::D24S8, bytes_per_pixel: 4, resolve_read_bytes: 4 })
        }
    }
}

impl DepthSurface {
    /// Bytes a row with each row padded to `pad` pixels (64 for the
    /// super-tiled layout).
    pub const fn stride(&self, width: u32, pad: u32) -> u32 {
        align_up(width, pad) * self.bytes_per_pixel
    }

    /// Bytes the surface takes: stride times the height padded to `pad`.
    pub const fn size(&self, width: u32, height: u32, pad: u32) -> u32 {
        self.stride(width, pad) * align_up(height, pad)
    }

    /// The 32-bit word the clear writes to every pixel, for a depth in
    /// [0, 1] (clamped) and a stencil value (D24S8 only; ignored by D16).
    pub fn clear_value(&self, depth: f32, stencil: u8) -> u32 {
        let d = if depth.is_nan() { 0.0 } else { depth.clamp(0.0, 1.0) };
        match self.format {
            PeDepthFormat::D16 => {
                let v = (d as f64 * 65535.0 + 0.5) as u32;
                v | v << 16
            }
            PeDepthFormat::D24S8 => {
                let v = (d as f64 * 16_777_215.0 + 0.5) as u32;
                v << 8 | stencil as u32
            }
        }
    }
}

/// Bytes of D24S8 surface a TS tile covers (`ts::tile_size` in the 128B mode).
const TS_TILE_BYTES: usize = super::ts::tile_size(super::ts::TS_MODE_128B) as usize;

/// The TS code of a tile the RS fast clear left cleared: one nibble of
/// [`super::ts::TS_CLEAR_VALUE`].
const TS_CLEARED: u8 = (super::ts::TS_CLEAR_VALUE & 0xf) as u8;

/// A `w` x `h` region (from pixel 0, 0) of a D24S8 surface as row-major
/// depth floats in `out` (`w * h` long, panics if shorter), reading no
/// surface word of a tile the tile status says is cleared.
///
/// `surface` is the D24S8 words (depth in bits 31..8) in `layout` with a
/// padded row of `stride` bytes (`layout::stride`). `ts` is the tile status,
/// four bits a tile as `ts::ts_size` sizes it, tile `n` in the low (even `n`)
/// or high (odd `n`) nibble of byte `n / 2`; tile `n` covers surface bytes
/// `n * 128 ..`, in memory order, so a pixel's tile is its
/// `layout::offset` / 128. A tile whose nibble is the clear code takes the
/// float of `clear_word` (the word the depth clear wrote, as
/// `ts::depth_clear_word`); any other tile, and a tile past the end of `ts`,
/// converts its words with `ds_retention::unorm24_depth`, the conversion of
/// `ds_cache::resolve_depth`: the result equals a full read of a surface
/// whose cleared tiles hold `clear_word`. Groundwork: nothing calls it until
/// the X2 tile-status probe says the TS is usable with the MMU off.
#[allow(clippy::too_many_arguments)]
pub fn read_depth_ts(
    surface: &[u32],
    layout: super::layout::Layout,
    stride: u32,
    ts: &[u8],
    clear_word: u32,
    w: u32,
    h: u32,
    out: &mut [f32],
) {
    let clear = crate::ds_retention::unorm24_depth(clear_word >> 8);
    for y in 0..h {
        for x in 0..w {
            let off = super::layout::offset(layout, x, y, 4, stride);
            let tile = off / TS_TILE_BYTES;
            let code = ts.get(tile / 2).map(|b| (b >> (4 * (tile & 1))) & 0xf);
            out[(y * w + x) as usize] = if code == Some(TS_CLEARED) {
                clear
            } else {
                crate::ds_retention::unorm24_depth(surface[off / 4] >> 8)
            };
        }
    }
}

#[cfg(test)]
mod ts_read_tests {
    use super::super::layout::{self, Layout};
    use super::super::ts;
    use super::read_depth_ts;
    use crate::ds_retention::unorm24_depth;

    const CLEAR: u32 = 0xffff_ff00;
    const POISON: u32 = 0x1234_5600;

    /// A deterministic scatter of words (xorshift), never equal to `CLEAR`.
    fn word(i: u32) -> u32 {
        let mut s = i.wrapping_mul(0x9e37_79b9) | 1;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        if s >> 8 == CLEAR >> 8 {
            s ^ 0x100
        } else {
            s
        }
    }

    /// Every third tile cleared, the rest written: `read_depth_ts` over a
    /// surface whose cleared tiles are poisoned (never read) equals the full
    /// read of the surface whose cleared tiles hold `CLEAR`.
    fn run(layout: Layout, w: u32, h: u32) {
        let super_tiled = layout == Layout::SuperTiled;
        let stride = layout::stride(layout, w, 4, true);
        let (_, ph) = ts::padded_dims(w, h, super_tiled);
        let words = (layout::size(layout, w, h, 4, true) / 4).max((stride * ph / 4) as usize);
        let n_tiles = words * 4 / 128 + 1;
        let mut ts_buf = vec![0u8; ts::ts_size(w, h, 4, super_tiled) as usize];
        let mut full: Vec<u32> = (0..words as u32).map(word).collect();
        let mut seen = full.clone();
        for t in (0..n_tiles).filter(|t| t % 3 == 1) {
            for (f, s) in full.iter_mut().zip(seen.iter_mut()).skip(t * 32).take(32) {
                *f = CLEAR;
                *s = POISON;
            }
        }
        for t in 0..n_tiles {
            let code = if t % 3 == 1 { 1u8 } else { (t % 14) as u8 + 2 };
            if let Some(b) = ts_buf.get_mut(t / 2) {
                *b |= code << (4 * (t & 1));
            }
        }
        let mut want = vec![0f32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                want[(y * w + x) as usize] = unorm24_depth(full[layout::offset(layout, x, y, 4, stride) / 4] >> 8);
            }
        }
        let mut got = vec![0f32; (w * h) as usize];
        read_depth_ts(&seen, layout, stride, &ts_buf, CLEAR, w, h, &mut got);
        for (i, (a, b)) in got.iter().zip(&want).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "{layout:?} {w}x{h} pixel {i}");
        }
    }

    #[test]
    fn linear_mix_is_bit_exact() {
        run(Layout::Linear, 64, 32);
    }

    #[test]
    fn linear_odd_size_and_pitch() {
        run(Layout::Linear, 37, 19);
        run(Layout::Linear, 5, 3);
    }

    #[test]
    fn super_tiled_mix_is_bit_exact() {
        run(Layout::SuperTiled, 128, 64);
    }

    #[test]
    fn super_tiled_odd_size_and_pitch() {
        run(Layout::SuperTiled, 100, 70);
        run(Layout::SuperTiled, 67, 65);
    }

    #[test]
    fn all_cleared_reads_no_surface() {
        let ts_buf = vec![0x11u8; 64];
        let mut out = vec![0f32; 9 * 7];
        read_depth_ts(&[], Layout::Linear, 9 * 4, &ts_buf, 0x8000_0000, 9, 7, &mut out);
        assert!(out.iter().all(|v| v.to_bits() == unorm24_depth(0x80_0000).to_bits()));
    }
}
