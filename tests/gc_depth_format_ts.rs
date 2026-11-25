// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! W12b's host test (docs/notes/gc-tiled-wire.md):
//! `depth_format::read_super_tiled_ts`, the read of a SUPER_TILED depth
//! surface through its tile status, bit for bit against
//! `depth_format::read_super_tiled`.
//!
//! The reference is built without the function's own tile arithmetic: every
//! tile the synthetic TS marks cleared has its whole byte range
//! (`tile * tile_bytes ..`, the TS granule of `ts.rs`) overwritten with the
//! clear word in a copy of the surface, and that copy is read with
//! `read_super_tiled`. A cleared tile must read as if memory held the clear
//! value, every other tile as memory. Covered: (a) a TS with no tile cleared
//! (zero entries and a non-clear code), (b) every tile cleared, (c) a mixed
//! TS, and a TS shorter than the surface (its missing entries not cleared),
//! for D24S8 and D16, the 128-byte and 256-byte TS modes, and sizes that are
//! and are not multiples of the 64x64 super tile. The destination's pitch is
//! wider than the image, so the padding must keep its sentinel.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_depth_format_ts`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::depth_format::{
    depth_surface, read_super_tiled, read_super_tiled_ts, DepthTs, GlDepthAttachment, PeDepthFormat, TS_CLEAR_CODE,
};
use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::{rs_interp, ts};

const SIZES: [(u32, u32); 7] = [(1, 1), (37, 13), (64, 64), (90, 70), (129, 65), (200, 3), (800, 600)];
const MODES: [u32; 2] = [ts::TS_MODE_128B, ts::TS_MODE_256B];
const SENTINEL: u32 = 0x7fc0_1234;

fn attachment(format: PeDepthFormat) -> GlDepthAttachment {
    match format {
        PeDepthFormat::D16 => GlDepthAttachment::DepthComponent16,
        PeDepthFormat::D24S8 => GlDepthAttachment::Depth24Stencil8,
    }
}

fn pattern(i: usize) -> u8 {
    ((i as u32).wrapping_mul(0x9e37_79b1).rotate_left(11) >> 3) as u8
}

/// A surface of `w` x `h` with Mesa's super-tiled padding, stale bytes
/// throughout; its stride; the TS bytes for `mode` (`ts::ts_layer_stride`).
fn surface(format: PeDepthFormat, w: u32, h: u32, mode: u32) -> (Vec<u8>, u32, usize) {
    let bpp = depth_surface(attachment(format)).unwrap().bytes_per_pixel;
    let stride = layout::stride(Layout::SuperTiled, w, bpp, false);
    let size = layout::size(Layout::SuperTiled, w, h, bpp, false);
    let s = (0..size).map(pattern).collect();
    (s, stride, ts::ts_layer_stride(size as u32, mode) as usize)
}

fn entry(status: &[u8], tile: usize) -> Option<u8> {
    status.get(tile / 2).map(|b| (b >> (4 * (tile % 2))) & 0xf)
}

fn set_entry(status: &mut [u8], tile: usize, code: u8) {
    let sh = 4 * (tile % 2);
    status[tile / 2] = (status[tile / 2] & !(0xf << sh)) | (code << sh);
}

/// The reference: the clear word written over each cleared tile of a copy,
/// then the plain read.
fn reference(format: PeDepthFormat, s: &[u8], stride: u32, w: u32, h: u32, pitch: u32, t: &DepthTs) -> Vec<u32> {
    let mut m = s.to_vec();
    let tb = t.tile_bytes as usize;
    for (tile, chunk) in m.chunks_mut(tb).enumerate() {
        if entry(t.status, tile) == Some(TS_CLEAR_CODE) {
            for (i, b) in chunk.iter_mut().enumerate() {
                *b = t.clear_value.to_le_bytes()[i % 4];
            }
        }
    }
    let mut dst = vec![f32::from_bits(SENTINEL); (h * pitch) as usize];
    read_super_tiled(format, &m, stride, w, h, &mut dst, pitch);
    dst.iter().map(|f| f.to_bits()).collect()
}

fn read_ts(format: PeDepthFormat, s: &[u8], stride: u32, w: u32, h: u32, pitch: u32, t: &DepthTs) -> Vec<u32> {
    let mut dst = vec![f32::from_bits(SENTINEL); (h * pitch) as usize];
    read_super_tiled_ts(format, s, stride, w, h, &mut dst, pitch, t);
    dst.iter().map(|f| f.to_bits()).collect()
}

fn plain(format: PeDepthFormat, s: &[u8], stride: u32, w: u32, h: u32, pitch: u32) -> Vec<u32> {
    let mut dst = vec![f32::from_bits(SENTINEL); (h * pitch) as usize];
    read_super_tiled(format, s, stride, w, h, &mut dst, pitch);
    dst.iter().map(|f| f.to_bits()).collect()
}

fn clear_word(format: PeDepthFormat) -> u32 {
    depth_surface(attachment(format)).unwrap().clear_value(0.625, 0x5a)
}

/// Every size, format and mode, with the TS `make` builds from the TS size.
fn each(mut make: impl FnMut(usize) -> Vec<u8>, mut check: impl FnMut(PeDepthFormat, &[u8], u32, u32, u32, &DepthTs)) {
    for format in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
        for mode in MODES {
            for (w, h) in SIZES {
                let (s, stride, ts_len) = surface(format, w, h, mode);
                let status = make(ts_len);
                let t = DepthTs { status: &status, tile_bytes: ts::tile_size(mode), clear_value: clear_word(format) };
                check(format, &s, stride, w, h, &t);
            }
        }
    }
}

#[test]
fn the_clear_code_is_the_trees() {
    assert_eq!(TS_CLEAR_CODE, rs_interp::TS_CLEAR_CODE);
    assert_eq!(ts::TS_CLEAR_VALUE & 0xf, TS_CLEAR_CODE as u32);
    assert_eq!(ts::tile_size(ts::TS_MODE_128B), 128);
    assert_eq!(ts::tile_size(ts::TS_MODE_256B), 256);
}

/// (a) No tile cleared: the plain read, for zero entries and for a code that
/// is not the clear code.
#[test]
fn no_tile_cleared_reads_memory() {
    for fill in [0x00u8, 0xee] {
        each(
            |n| vec![fill; n],
            |format, s, stride, w, h, t| {
                let pitch = w + 3;
                let got = read_ts(format, s, stride, w, h, pitch, t);
                assert_eq!(got, plain(format, s, stride, w, h, pitch), "{format:?} {w}x{h} tile {}", t.tile_bytes);
            },
        );
    }
}

/// (b) Every tile cleared: every pixel is the clear value's depth, the
/// padding keeps its sentinel.
#[test]
fn every_tile_cleared_reads_the_clear_value() {
    each(
        |n| vec![(ts::TS_CLEAR_VALUE & 0xff) as u8; n],
        |format, s, stride, w, h, t| {
            let pitch = w + 5;
            let depth = match format {
                PeDepthFormat::D16 => ((t.clear_value & 0xffff) as f64 / 65535.0) as f32,
                PeDepthFormat::D24S8 => ((t.clear_value >> 8) as f64 / 16_777_215.0) as f32,
            };
            let got = read_ts(format, s, stride, w, h, pitch, t);
            for y in 0..h {
                for x in 0..pitch {
                    let want = if x < w { depth.to_bits() } else { SENTINEL };
                    assert_eq!(got[(y * pitch + x) as usize], want, "{format:?} {w}x{h} ({x}, {y})");
                }
            }
            assert_eq!(got, reference(format, s, stride, w, h, pitch, t));
        },
    );
}

/// (c) A mixed TS: cleared, zero and other codes scattered over the tiles.
#[test]
fn mixed_tile_status_matches_the_reference() {
    let mut seed = 0x1234_5678u32;
    each(
        |n| {
            let mut v = vec![0u8; n];
            for tile in 0..2 * n {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let code = match seed >> 29 {
                    0..=3 => TS_CLEAR_CODE,
                    4 | 5 => 0,
                    _ => ((seed >> 16) & 0xf) as u8,
                };
                set_entry(&mut v, tile, code);
            }
            v
        },
        |format, s, stride, w, h, t| {
            let pitch = w + 1;
            let got = read_ts(format, s, stride, w, h, pitch, t);
            assert_eq!(got, reference(format, s, stride, w, h, pitch, t), "{format:?} {w}x{h} tile {}", t.tile_bytes);
            // The mix must reach the read for the check to mean anything: a
            // whole super tile (16 or 32 TS tiles) is read from 64x64 up.
            if w >= 64 && h >= 64 {
                assert_ne!(got, plain(format, s, stride, w, h, pitch));
            }
        },
    );
}

/// A TS shorter than the surface: the tiles it does not reach read memory.
#[test]
fn entries_past_the_tile_status_read_memory() {
    each(
        |n| vec![(ts::TS_CLEAR_VALUE & 0xff) as u8; n / 3],
        |format, s, stride, w, h, t| {
            let pitch = w;
            let got = read_ts(format, s, stride, w, h, pitch, t);
            assert_eq!(got, reference(format, s, stride, w, h, pitch, t), "{format:?} {w}x{h}");
        },
    );
}

#[test]
#[should_panic(expected = "tile_bytes")]
fn a_tile_size_the_tree_does_not_have_panics() {
    let (s, stride, _) = surface(PeDepthFormat::D24S8, 8, 8, ts::TS_MODE_128B);
    let t = DepthTs { status: &[0; 4], tile_bytes: 64, clear_value: 0 };
    read_ts(PeDepthFormat::D24S8, &s, stride, 8, 8, 8, &t);
}
