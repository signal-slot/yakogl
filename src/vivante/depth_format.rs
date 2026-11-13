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

// CPU access to a super-tiled depth surface (docs/notes/gc-tiled-wire.md, W12a:
// `resolve_depth` and `seed_depth` of ds_cache.rs read and write linear
// surfaces only). Pure functions over a byte slice; nothing calls them yet.

use super::layout::{self, Layout};

const D24_MAX: f64 = 16_777_215.0;
const D16_MAX: f64 = 65_535.0;

const fn depth_bpp(format: PeDepthFormat) -> u32 {
    match format {
        PeDepthFormat::D16 => 2,
        PeDepthFormat::D24S8 => 4,
    }
}

/// The depth float of the pixel at `px` (its `bytes_per_pixel` bytes): D24S8
/// is `(word >> 8) / 0xffffff`, D16 `half / 0xffff`, computed in f64 and
/// rounded to f32 as `resolve_depth` does.
fn depth_of(format: PeDepthFormat, px: &[u8]) -> f32 {
    match format {
        PeDepthFormat::D24S8 => ((u32::from_ne_bytes([px[0], px[1], px[2], px[3]]) >> 8) as f64 / D24_MAX) as f32,
        PeDepthFormat::D16 => (u16::from_ne_bytes([px[0], px[1]]) as f64 / D16_MAX) as f32,
    }
}

/// Store `d` ([0, 1] clamped, NaN is 0, rounded as `clear_value` does) into
/// the pixel at `px`; D24S8 keeps the stencil byte.
fn store_depth(format: PeDepthFormat, px: &mut [u8], d: f32) {
    let d = if d.is_nan() { 0.0 } else { d.clamp(0.0, 1.0) as f64 };
    match format {
        PeDepthFormat::D24S8 => {
            let old = u32::from_ne_bytes([px[0], px[1], px[2], px[3]]);
            let v = ((d * D24_MAX + 0.5) as u32) << 8 | (old & 0xff);
            px[..4].copy_from_slice(&v.to_ne_bytes());
        }
        PeDepthFormat::D16 => px[..2].copy_from_slice(&((d * D16_MAX + 0.5) as u16).to_ne_bytes()),
    }
}

/// The area both a surface and a `w` x `h` request cover: columns up to the
/// stride, rows up to the bytes `surface_len` holds.
fn covered(w: u32, h: u32, stride: u32, surface_len: usize, bpp: u32) -> (u32, u32) {
    let cols = w.min(stride / bpp);
    let rows = if stride == 0 { 0 } else { (surface_len / stride as usize).min(h as usize) as u32 };
    (cols, rows)
}

/// Read the depth of a SUPER_TILED surface (`surface`, row stride `stride`
/// bytes, [`layout::stride`]) into `dst`, `pitch` floats a row: pixel
/// `(x, y)` is at [`layout::offset`]`(SuperTiled, ..)`. The floats equal
/// `resolve_depth`'s read of the same pixels laid out linearly; a texel
/// outside the surface, or in the pitch's padding, keeps its value.
///
/// Panics if `w > pitch`, if `dst` is shorter than `h` rows of `pitch`
/// floats, or if `surface` ends inside a super-tile it must read.
pub fn read_super_tiled(
    format: PeDepthFormat,
    surface: &[u8],
    stride: u32,
    w: u32,
    h: u32,
    dst: &mut [f32],
    pitch: u32,
) {
    assert!(w <= pitch && dst.len() >= h as usize * pitch as usize, "depth_format: dst too short");
    let bpp = depth_bpp(format);
    let (cols, rows) = covered(w, h, stride, surface.len(), bpp);
    for y in 0..rows {
        for x in 0..cols {
            let at = layout::offset(Layout::SuperTiled, x, y, bpp, stride);
            dst[y as usize * pitch as usize + x as usize] = depth_of(format, &surface[at..at + bpp as usize]);
        }
    }
}

/// Write `src` (`h` rows of `pitch` floats) into a SUPER_TILED surface the
/// way [`read_super_tiled`] reads it. The stencil of a D24S8 word and every
/// byte not covered stay. The inverse of the read up to the depth's
/// quantisation (24 or 16 bits).
pub fn write_super_tiled(
    format: PeDepthFormat,
    surface: &mut [u8],
    stride: u32,
    w: u32,
    h: u32,
    src: &[f32],
    pitch: u32,
) {
    assert!(w <= pitch && src.len() >= h as usize * pitch as usize, "depth_format: src too short");
    let bpp = depth_bpp(format);
    let (cols, rows) = covered(w, h, stride, surface.len(), bpp);
    for y in 0..rows {
        for x in 0..cols {
            let at = layout::offset(Layout::SuperTiled, x, y, bpp, stride);
            store_depth(format, &mut surface[at..at + bpp as usize], src[y as usize * pitch as usize + x as usize]);
        }
    }
}

#[cfg(test)]
mod super_tiled_tests {
    use super::*;

    /// The (w, h, pitch) asks of tests/gc_depth_resolve.rs.
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
        let mut x = (i as u32).wrapping_mul(0x9e37_79b9) | 1;
        x ^= x << 13;
        x ^= x >> 17;
        x ^ x << 5
    }

    /// A 90 x 70 surface as the cache sizes it: the linear words and the same
    /// words placed by `layout::offset`.
    fn surfaces(format: PeDepthFormat) -> (Vec<u8>, Vec<u8>, u32) {
        let bpp = depth_bpp(format);
        let stride = layout::stride(Layout::SuperTiled, 90, bpp, false);
        let rows = layout::aligned_height(Layout::SuperTiled, 70, false);
        let mut lin = vec![0u8; stride as usize * rows as usize];
        let mut tiled = vec![0xa5u8; lin.len()];
        for y in 0..rows {
            for x in 0..stride / bpp {
                let v = pattern((y * (stride / bpp) + x) as usize).to_ne_bytes();
                let l = (y * stride + x * bpp) as usize;
                lin[l..l + bpp as usize].copy_from_slice(&v[..bpp as usize]);
                let t = layout::offset(Layout::SuperTiled, x, y, bpp, stride);
                tiled[t..t + bpp as usize].copy_from_slice(&v[..bpp as usize]);
            }
        }
        (lin, tiled, stride)
    }

    /// The linear read: the rule of gc_depth_resolve.rs, per word.
    fn linear_read(format: PeDepthFormat, lin: &[u8], stride: u32, w: u32, h: u32, dst: &mut [f32], pitch: u32) {
        let bpp = depth_bpp(format);
        let (cols, rows) = covered(w, h, stride, lin.len(), bpp);
        for y in 0..rows {
            for x in 0..cols {
                let l = (y * stride + x * bpp) as usize;
                dst[(y * pitch + x) as usize] = depth_of(format, &lin[l..l + bpp as usize]);
            }
        }
    }

    #[test]
    fn super_tiled_read_is_the_linear_read() {
        for format in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
            let (lin, tiled, stride) = surfaces(format);
            for (w, h, pitch) in CASES {
                let mut want = vec![7.5f32; (h * pitch) as usize];
                let mut got = want.clone();
                linear_read(format, &lin, stride, w, h, &mut want, pitch);
                read_super_tiled(format, &tiled, stride, w, h, &mut got, pitch);
                for (i, (g, e)) in got.iter().zip(&want).enumerate() {
                    assert_eq!(g.to_bits(), e.to_bits(), "{format:?} {w}x{h}/{pitch} at {i}");
                }
            }
        }
    }

    #[test]
    fn d24_rule_and_extremes() {
        for (word, want) in
            [(0x0000_00ffu32, 0.0f32), (0xffff_ff00, 1.0), (0x8000_0000, (0x80_0000 as f64 / D24_MAX) as f32)]
        {
            assert_eq!(depth_of(PeDepthFormat::D24S8, &word.to_ne_bytes()).to_bits(), want.to_bits());
        }
        assert_eq!(depth_of(PeDepthFormat::D16, &0xffffu16.to_ne_bytes()), 1.0);
    }

    #[test]
    fn super_tiled_write_lands_where_the_read_looks() {
        for format in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
            let (lin, tiled, stride) = surfaces(format);
            let bpp = depth_bpp(format);
            for (w, h, pitch) in CASES {
                let mut floats = vec![7.5f32; (h * pitch) as usize];
                linear_read(format, &lin, stride, w, h, &mut floats, pitch);
                let mut surf = tiled.clone();
                write_super_tiled(format, &mut surf, stride, w, h, &floats, pitch);
                let mut back = vec![7.5f32; (h * pitch) as usize];
                read_super_tiled(format, &surf, stride, w, h, &mut back, pitch);
                let (cols, rows) = covered(w, h, stride, surf.len(), bpp);
                for y in 0..rows {
                    for x in 0..cols {
                        let i = (y * pitch + x) as usize;
                        assert_eq!(back[i].to_bits(), floats[i].to_bits(), "{format:?} {w}x{h}/{pitch} at ({x}, {y})");
                    }
                }
                // Bytes outside the covered pixels, and every stencil byte, stay.
                for y in 0..layout::aligned_height(Layout::SuperTiled, 70, false) {
                    for x in 0..stride / bpp {
                        let t = layout::offset(Layout::SuperTiled, x, y, bpp, stride);
                        let (a, b) = (&surf[t..t + bpp as usize], &tiled[t..t + bpp as usize]);
                        if x >= cols || y >= rows {
                            assert_eq!(a, b, "{format:?} {w}x{h}/{pitch} touched ({x}, {y})");
                        } else if format == PeDepthFormat::D24S8 {
                            assert_eq!(a[0], b[0], "{format:?} {w}x{h}/{pitch} stencil at ({x}, {y})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn write_clamps_and_rounds_like_clear_value() {
        let s = depth_surface(GlDepthAttachment::Depth24Stencil8).unwrap();
        let mut surf = vec![0x5au8; 64 * 64 * 4];
        write_super_tiled(PeDepthFormat::D24S8, &mut surf, 64 * 4, 2, 1, &[2.0, f32::NAN], 2);
        for (x, d) in [(0, 1.0), (1, 0.0)] {
            let at = layout::offset(Layout::SuperTiled, x, 0, 4, 256);
            assert_eq!(&surf[at..at + 4], &s.clear_value(d, 0x5a).to_ne_bytes());
        }
    }
}
