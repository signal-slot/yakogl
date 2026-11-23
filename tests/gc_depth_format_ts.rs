// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! W12b's host test (docs/notes/gc-tiled-wire.md): `read_super_tiled_ts`, the
//! tile-status-aware read of a SUPER_TILED depth surface, against
//! `read_super_tiled`, bit for bit, over a synthetic depth TS:
//! (a) no tile cleared: the same as the plain read;
//! (b) every tile cleared: the clear value's depth, whatever the surface holds;
//! (c) a mixed TS: the plain read of the surface with the cleared tiles
//!     overwritten by the clear word.
//! Sizes include ones that are not a multiple of the 64x64 super tile.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_depth_format_ts`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::depth_format::{read_super_tiled, read_super_tiled_ts, PeDepthFormat};
use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::rs_interp::TS_CLEAR_CODE;
use yakogl::vivante::ts;

const SIZES: [(u32, u32); 5] = [(64, 64), (128, 64), (90, 70), (37, 13), (200, 130)];
const TILE: u32 = 128;

fn bpp(f: PeDepthFormat) -> u32 {
    match f {
        PeDepthFormat::D16 => 2,
        PeDepthFormat::D24S8 => 4,
    }
}

/// A padded surface of pseudo-random bytes and its stride.
fn surface(f: PeDepthFormat, w: u32, h: u32) -> (Vec<u8>, u32) {
    let (pw, ph) = ts::padded_dims(w, h, true);
    let stride = pw * bpp(f);
    let mut x = 0x1234_5678u32 ^ (w << 8) ^ h;
    let s = (0..stride * ph)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    (s, stride)
}

fn ts_buffer(len: usize, cleared: impl Fn(usize) -> bool) -> Vec<u8> {
    let tiles = len.div_ceil(TILE as usize);
    let mut t = vec![0u8; tiles.div_ceil(2) + 2];
    for tile in 0..tiles {
        if cleared(tile) {
            t[tile / 2] |= TS_CLEAR_CODE << (4 * (tile % 2));
        }
    }
    t
}

fn clear_word(f: PeDepthFormat) -> u32 {
    match f {
        PeDepthFormat::D24S8 => 0x00ab_cd7f,
        PeDepthFormat::D16 => 0x3456_3456,
    }
}

/// Overwrite every pixel in a cleared tile with the clear word.
fn apply_clear(f: PeDepthFormat, s: &mut [u8], stride: u32, w: u32, h: u32, ts: &[u8]) {
    let b = bpp(f) as usize;
    for y in 0..h {
        for x in 0..w {
            let at = layout::offset(Layout::SuperTiled, x, y, b as u32, stride);
            let tile = at / TILE as usize;
            if (ts[tile / 2] >> (4 * (tile % 2))) & 0xf == TS_CLEAR_CODE {
                s[at..at + b].copy_from_slice(&clear_word(f).to_le_bytes()[..b]);
            }
        }
    }
}

fn check(f: PeDepthFormat, cleared: impl Fn(usize) -> bool + Copy) {
    for (w, h) in SIZES {
        let (mut s, stride) = surface(f, w, h);
        let t = ts_buffer(s.len(), cleared);
        let pitch = w + 3;
        let mut got = vec![-1.0f32; (h * pitch) as usize];
        read_super_tiled_ts(f, &s, stride, w, h, &mut got, pitch, &t, TILE, clear_word(f));
        apply_clear(f, &mut s, stride, w, h, &t);
        let mut want = vec![-1.0f32; (h * pitch) as usize];
        read_super_tiled(f, &s, stride, w, h, &mut want, pitch);
        let g: Vec<u32> = got.iter().map(|v| v.to_bits()).collect();
        let e: Vec<u32> = want.iter().map(|v| v.to_bits()).collect();
        assert_eq!(g, e, "{f:?} {w}x{h}");
    }
}

#[test]
fn a_ts_with_no_cleared_tile_is_the_plain_read() {
    for f in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
        check(f, |_| false);
    }
}

#[test]
fn an_all_cleared_ts_reads_the_clear_value() {
    for f in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
        check(f, |_| true);
    }
    let (w, h) = (90, 70);
    let f = PeDepthFormat::D24S8;
    let (s, stride) = surface(f, w, h);
    let t = ts_buffer(s.len(), |_| true);
    let mut got = vec![0.0f32; (w * h) as usize];
    read_super_tiled_ts(f, &s, stride, w, h, &mut got, w, &t, TILE, clear_word(f));
    let want = ((clear_word(f) >> 8) as f64 / 16_777_215.0) as f32;
    assert!(got.iter().all(|v| v.to_bits() == want.to_bits()));
}

#[test]
fn a_mixed_ts_reads_cleared_tiles_as_clear_and_the_rest_from_memory() {
    for f in [PeDepthFormat::D24S8, PeDepthFormat::D16] {
        check(f, |t| t % 5 < 2);
        check(f, |t| t % 2 == 1);
    }
}

#[test]
fn an_entry_that_is_not_the_clear_code_reads_memory() {
    let f = PeDepthFormat::D24S8;
    let (s, stride) = surface(f, 64, 64);
    let t = vec![0x22u8; s.len() / 256 + 2];
    let mut got = vec![0.0f32; 64 * 64];
    let mut want = got.clone();
    read_super_tiled_ts(f, &s, stride, 64, 64, &mut got, 64, &t, TILE, clear_word(f));
    read_super_tiled(f, &s, stride, 64, 64, &mut want, 64);
    assert_eq!(got, want);
}
