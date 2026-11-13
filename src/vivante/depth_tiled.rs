// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! CPU access to a super-tiled depth surface (docs/notes/gc-tiled-wire.md,
//! W12a). Kept apart from `depth_format.rs`, which tests/gc_depth_format.rs
//! includes on its own and so cannot import `layout`. Nothing calls it yet.
//! `resolve_depth` and `seed_depth` of ds_cache.rs read and write linear
//! surfaces only; these are pure functions over a byte slice.

use super::depth_format::PeDepthFormat;
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
    use super::super::depth_format::{depth_surface, GlDepthAttachment};
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
