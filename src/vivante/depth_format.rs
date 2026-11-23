// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth surface format and size of a GL depth attachment, as a pure
//! function (docs/notes/gc-slow-terrain-stream-diff.md, rank 6).
//!
//! Terrain's `DEPTH_COMPONENT16` renderbuffer is stored and tested as D24S8
//! today (stride 0xd00 at 800 wide, Mesa's 0x680): twice the depth bytes in
//! the terrain draw, its clear and the overlay draw. This file says what the
//! surface should be; nothing calls it yet. Core only: it depends on `super::layout`
//! alone (itself core-only), so a test crate that includes this file by
//! `#[path]` must include `layout.rs` beside it.
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

use super::layout::{self, Layout};

/// The TS entry that marks a tile cleared (`ts::TS_CLEAR_VALUE`'s nibble,
/// `rs_interp::TS_CLEAR_CODE`; local because this file is core-only).
const TS_CLEAR_CODE: u8 = 0x1;

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

const fn bytes_per_pixel(format: PeDepthFormat) -> usize {
    match format {
        PeDepthFormat::D16 => 2,
        PeDepthFormat::D24S8 => 4,
    }
}

/// Read a SUPER_TILED depth surface (W12a groundwork, docs/notes/gc-tiled-wire.md)
/// into `dst`, `h` rows of `pitch` floats: pixel `(x, y)` is at
/// `layout::offset(Layout::SuperTiled, x, y, bpp, stride)`, the float is
/// D24S8 `(word >> 8) / 16777215` or D16 `half / 65535`, rounded to f32 as
/// `resolve_depth`'s linear read does. A texel outside the surface and the
/// pitch's padding keep their value. Panics if `w > pitch` or `dst` is
/// shorter than `h * pitch`.
pub fn read_super_tiled(
    format: PeDepthFormat,
    surface: &[u8],
    stride: u32,
    w: u32,
    h: u32,
    dst: &mut [f32],
    pitch: u32,
) {
    assert!(w <= pitch, "width {w} over pitch {pitch}");
    assert!(dst.len() >= h as usize * pitch as usize, "dst too short");
    let bpp = bytes_per_pixel(format);
    let cols = w.min(stride / bpp as u32);
    let rows = h.min(surface.len().div_ceil(stride.max(1) as usize) as u32);
    for y in 0..rows {
        for x in 0..cols {
            let at = layout::offset(Layout::SuperTiled, x, y, bpp as u32, stride);
            let Some(b) = surface.get(at..at + bpp) else { continue };
            dst[y as usize * pitch as usize + x as usize] = match format {
                PeDepthFormat::D16 => (u16::from_le_bytes([b[0], b[1]]) as f64 / 65535.0) as f32,
                PeDepthFormat::D24S8 => {
                    ((u32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8) as f64 / 16_777_215.0) as f32
                }
            };
        }
    }
}

/// [`read_super_tiled`] through a depth tile-status buffer (W12b groundwork,
/// docs/notes/gc-tiled-wire.md): a pixel whose tile's TS entry is
/// [`TS_CLEAR_CODE`] reads as `clear_value`'s depth (the word the depth fast
/// clear programs, `TS_DEPTH_CLEAR_VALUE`: D24S8 `depth << 8 | stencil`, D16
/// the doubled half), whatever the surface holds there; every other entry
/// reads the surface as [`read_super_tiled`] does. The TS is Mesa's
/// (`ts.rs`): one 4-bit entry per `tile_bytes` of surface (128, or 256 with
/// `TS_MODE_256B`), the low nibble of a byte first, the tile of a pixel being
/// its [`layout::offset`] `/ tile_bytes`, as `rs_interp`'s `ts_entry`. A tile
/// past the end of `ts` counts as not cleared. Compression is not modelled
/// (this part has none outside MSAA): any other entry reads memory.
/// Same bounds and panics as [`read_super_tiled`].
#[allow(clippy::too_many_arguments)]
pub fn read_super_tiled_ts(
    format: PeDepthFormat,
    surface: &[u8],
    stride: u32,
    w: u32,
    h: u32,
    dst: &mut [f32],
    pitch: u32,
    ts: &[u8],
    tile_bytes: u32,
    clear_value: u32,
) {
    assert!(w <= pitch, "width {w} over pitch {pitch}");
    assert!(dst.len() >= h as usize * pitch as usize, "dst too short");
    assert!(tile_bytes > 0, "tile_bytes is zero");
    let bpp = bytes_per_pixel(format);
    let cleared = match format {
        PeDepthFormat::D16 => ((clear_value & 0xffff) as f64 / 65535.0) as f32,
        PeDepthFormat::D24S8 => ((clear_value >> 8) as f64 / 16_777_215.0) as f32,
    };
    let cols = w.min(stride / bpp as u32);
    let rows = h.min(surface.len().div_ceil(stride.max(1) as usize) as u32);
    for y in 0..rows {
        for x in 0..cols {
            let at = layout::offset(Layout::SuperTiled, x, y, bpp as u32, stride);
            let Some(b) = surface.get(at..at + bpp) else { continue };
            let tile = at / tile_bytes as usize;
            let code = ts.get(tile / 2).map(|e| (e >> (4 * (tile % 2))) & 0xf);
            dst[y as usize * pitch as usize + x as usize] = if code == Some(TS_CLEAR_CODE) {
                cleared
            } else {
                match format {
                    PeDepthFormat::D16 => (u16::from_le_bytes([b[0], b[1]]) as f64 / 65535.0) as f32,
                    PeDepthFormat::D24S8 => {
                        ((u32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8) as f64 / 16_777_215.0) as f32
                    }
                }
            };
        }
    }
}

/// The inverse of [`read_super_tiled`]: write `src` (`h` rows of `pitch`
/// floats) into the SUPER_TILED surface. Depth is clamped to [0, 1], NaN is
/// 0 and the value is rounded `+ 0.5`, as [`DepthSurface::clear_value`]. D24S8
/// keeps each word's low (stencil) byte; bytes not covered stay untouched.
/// Panics if `w > pitch` or `src` is shorter than `h * pitch`.
pub fn write_super_tiled(
    format: PeDepthFormat,
    surface: &mut [u8],
    stride: u32,
    w: u32,
    h: u32,
    src: &[f32],
    pitch: u32,
) {
    assert!(w <= pitch, "width {w} over pitch {pitch}");
    assert!(src.len() >= h as usize * pitch as usize, "src too short");
    let bpp = bytes_per_pixel(format);
    let cols = w.min(stride / bpp as u32);
    let rows = h.min(surface.len().div_ceil(stride.max(1) as usize) as u32);
    for y in 0..rows {
        for x in 0..cols {
            let at = layout::offset(Layout::SuperTiled, x, y, bpp as u32, stride);
            let Some(b) = surface.get_mut(at..at + bpp) else { continue };
            let f = src[y as usize * pitch as usize + x as usize];
            let d = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) } as f64;
            match format {
                PeDepthFormat::D16 => b.copy_from_slice(&((d * 65535.0 + 0.5) as u16).to_le_bytes()),
                PeDepthFormat::D24S8 => {
                    let v = (d * 16_777_215.0 + 0.5) as u32;
                    b.copy_from_slice(&(v << 8 | b[0] as u32).to_le_bytes());
                }
            }
        }
    }
}

#[cfg(test)]
mod super_tiled_tests {
    use super::*;

    const CASES: [(u32, u32, u32); 13] = [
        (90, 70, 90),
        (90, 70, 97),
        (128, 128, 129),
        (1, 1, 1),
        (150, 140, 160),
        (3, 5, 64),
        (89, 70, 89),
        (91, 33, 95),
        (7, 3, 9),
        (5, 2, 5),
        (2, 4, 3),
        (127, 70, 127),
        (129, 1, 131),
    ];

    fn pattern(i: usize) -> u32 {
        (i as u32).wrapping_mul(0x9e37_79b1).rotate_left(7) ^ 0x5bd1_e995
    }

    fn surface(format: PeDepthFormat) -> (Vec<u8>, u32, u32, u32) {
        let bpp = bytes_per_pixel(format) as u32;
        let stride = layout::stride(Layout::SuperTiled, 90, bpp, false);
        let rows = layout::aligned_height(Layout::SuperTiled, 70, false);
        (vec![0; (stride * rows) as usize], stride, rows, bpp)
    }

    /// Fill the tiled surface and a linear copy with the same pattern; the
    /// extremes sit at the first pixels.
    fn fill(format: PeDepthFormat) -> (Vec<u8>, u32, Vec<u32>, u32) {
        let (mut s, stride, rows, bpp) = surface(format);
        let tw = stride / bpp;
        let mut lin = vec![0u32; (tw * rows) as usize];
        for y in 0..rows {
            for x in 0..tw {
                let mut v = pattern((y * tw + x) as usize);
                if format == PeDepthFormat::D16 {
                    v &= 0xffff;
                }
                match (x, y, format) {
                    (0, 0, PeDepthFormat::D24S8) => v = 0x0000_00ff,
                    (1, 0, PeDepthFormat::D24S8) => v = 0xffff_ff00,
                    (2, 0, PeDepthFormat::D24S8) => v = 0x8000_0000,
                    (0, 0, _) => v = 0,
                    (1, 0, _) => v = 0xffff,
                    _ => {}
                }
                lin[(y * tw + x) as usize] = v;
                let at = layout::offset(Layout::SuperTiled, x, y, bpp, stride);
                s[at..at + bpp as usize].copy_from_slice(&v.to_le_bytes()[..bpp as usize]);
            }
        }
        (s, stride, lin, tw)
    }

    fn linear_float(format: PeDepthFormat, v: u32) -> f32 {
        match format {
            PeDepthFormat::D16 => (v as f64 / 65535.0) as f32,
            PeDepthFormat::D24S8 => ((v >> 8) as f64 / 16_777_215.0) as f32,
        }
    }

    #[test]
    fn read_is_bit_exact_with_the_linear_read() {
        for format in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
            let (s, stride, lin, tw) = fill(format);
            let rows = lin.len() as u32 / tw;
            for (w, h, pitch) in CASES {
                let mut dst = vec![7.5f32; (h * pitch) as usize];
                read_super_tiled(format, &s, stride, w, h, &mut dst, pitch);
                for y in 0..h {
                    for x in 0..pitch {
                        let got = dst[(y * pitch + x) as usize];
                        let inside = x < w && x < tw && y < rows;
                        let want = if inside { linear_float(format, lin[(y * tw + x) as usize]) } else { 7.5 };
                        assert_eq!(got.to_bits(), want.to_bits(), "{format:?} {w}x{h}/{pitch} at ({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn write_then_read_round_trips_and_keeps_the_rest() {
        for format in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
            let (before, stride, ..) = fill(format);
            let (w, h, pitch) = (89u32, 70u32, 93u32);
            let src: Vec<f32> =
                (0..h * pitch).map(|i| (pattern(i as usize) & 0xff_ffff) as f32 / 16_777_215.0).collect();
            let mut s = before.clone();
            write_super_tiled(format, &mut s, stride, w, h, &src, pitch);
            let mut back = vec![7.5f32; (h * pitch) as usize];
            read_super_tiled(format, &s, stride, w, h, &mut back, pitch);
            let bpp = bytes_per_pixel(format);
            let tol = if format == PeDepthFormat::D16 { 1.0 / 65535.0 } else { 1.0 / 16_777_215.0 };
            let mut covered = vec![false; s.len()];
            for y in 0..h {
                for x in 0..w {
                    let (a, b) = (src[(y * pitch + x) as usize], back[(y * pitch + x) as usize]);
                    assert!((a - b).abs() <= tol, "{format:?} ({x}, {y}): {a} vs {b}");
                    let at = layout::offset(Layout::SuperTiled, x, y, bpp as u32, stride);
                    covered[at..at + bpp].fill(true);
                    if format == PeDepthFormat::D24S8 {
                        assert_eq!(s[at], before[at], "stencil byte at ({x}, {y})");
                    }
                }
                for x in w..pitch {
                    assert_eq!(back[(y * pitch + x) as usize], 7.5);
                }
            }
            for (i, c) in covered.iter().enumerate() {
                if !c {
                    assert_eq!(s[i], before[i], "{format:?} uncovered byte {i}");
                }
            }
        }
    }

    #[test]
    fn write_clamps_and_maps_nan_like_clear_value() {
        let both = [
            (GlDepthAttachment::Depth24Stencil8, PeDepthFormat::D24S8),
            (GlDepthAttachment::DepthComponent16, PeDepthFormat::D16),
        ];
        for (att, format) in both {
            let ds = depth_surface(att).unwrap();
            let vals =
                [-1.0f32, -0.0, 0.0, 1e-9, 0.25, 0.5, 0.999_999, 1.0, 2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
            let (mut s, stride, ..) = surface(format);
            let n = vals.len() as u32;
            write_super_tiled(format, &mut s, stride, n, 1, &vals, n);
            for (x, &v) in vals.iter().enumerate() {
                let at = layout::offset(Layout::SuperTiled, x as u32, 0, ds.bytes_per_pixel, stride);
                let got = u32::from_le_bytes(core::array::from_fn(|i| {
                    if i < ds.bytes_per_pixel as usize {
                        s[at + i]
                    } else {
                        0
                    }
                }));
                let want = match format {
                    PeDepthFormat::D24S8 => ds.clear_value(v, 0),
                    PeDepthFormat::D16 => ds.clear_value(v, 0) & 0xffff,
                };
                assert_eq!(got, want, "{format:?} {v}");
            }
        }
    }

    #[test]
    fn d24_extremes() {
        let (mut s, stride, ..) = surface(PeDepthFormat::D24S8);
        let at = layout::offset(Layout::SuperTiled, 0, 0, 4, stride);
        let mut dst = [0f32; 1];
        for (word, want) in
            [(0x0000_00ffu32, 0.0f32), (0xffff_ff00, 1.0), (0x8000_0000, (0x80_0000 as f64 / 16_777_215.0) as f32)]
        {
            s[at..at + 4].copy_from_slice(&word.to_le_bytes());
            read_super_tiled(PeDepthFormat::D24S8, &s, stride, 1, 1, &mut dst, 1);
            assert_eq!(dst[0].to_bits(), want.to_bits(), "{word:#x}");
        }
    }

    #[test]
    #[should_panic]
    fn read_panics_when_w_exceeds_pitch() {
        let (s, stride, ..) = surface(PeDepthFormat::D16);
        read_super_tiled(PeDepthFormat::D16, &s, stride, 4, 1, &mut [0.0; 8], 3);
    }
}
