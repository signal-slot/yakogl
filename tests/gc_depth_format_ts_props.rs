// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Seeded property test of `depth_format::read_super_tiled_ts` (W12b)
//! against the full read: random D24S8 contents, random per-tile TS entries
//! (clear code or another code), both TS tile sizes (128 and 256 bytes),
//! random clear words, and widths and heights that are and are not
//! multiples of the 64x64 super tile. The expected picture is `read_super_tiled`
//! over a copy of the surface in which every tile marked clear holds the clear
//! word. The generator is a fixed-seed xorshift written here (no dependency).
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_depth_format_ts_props`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::depth_format::{
    depth_surface, read_super_tiled, read_super_tiled_ts, DepthTs, GlDepthAttachment, PeDepthFormat, TS_CLEAR_CODE,
};
use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::ts;

const SEED: u64 = 0x5eed_d24_5_8c1ea4;
const CASES: u32 = 1200;
const SENTINEL: u32 = 0x7fc0_1234;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u32) -> u32 {
        ((self.next() >> 32) % n as u64) as u32
    }
}

/// A dimension: about half the time a multiple of 64, else anything up to 150.
fn dim(r: &mut Rng) -> u32 {
    if r.below(2) == 0 {
        64 * (1 + r.below(2))
    } else {
        1 + r.below(150)
    }
}

fn entry(status: &[u8], tile: usize) -> Option<u8> {
    status.get(tile / 2).map(|b| (b >> (4 * (tile % 2))) & 0xf)
}

fn case(seed: u64) -> Result<(), String> {
    let mut r = Rng(seed);
    let (w, h) = (dim(&mut r), dim(&mut r));
    let mode = if r.below(2) == 0 { ts::TS_MODE_128B } else { ts::TS_MODE_256B };
    let tile_bytes = ts::tile_size(mode);
    let format = PeDepthFormat::D24S8;
    let bpp = depth_surface(GlDepthAttachment::Depth24Stencil8).unwrap().bytes_per_pixel;
    let stride = layout::stride(Layout::SuperTiled, w, bpp, false);
    let size = layout::size(Layout::SuperTiled, w, h, bpp, false);
    let surface: Vec<u8> = (0..size).map(|_| r.next() as u8).collect();
    let entries = size.div_ceil(tile_bytes as usize);
    let clear_prob = [0, 1, 2, 4, 7, 8][r.below(6) as usize]; // of 8
    let mut status = vec![0u8; entries.div_ceil(2)];
    for tile in 0..entries {
        let code = if r.below(8) < clear_prob { TS_CLEAR_CODE } else { (r.below(16) as u8) & !TS_CLEAR_CODE & 0xf };
        status[tile / 2] |= code << (4 * (tile % 2));
    }
    let clear_value = r.next() as u32;
    let t = DepthTs { status: &status, tile_bytes, clear_value };
    let pitch = w + r.below(4);

    let mut mem = surface.clone();
    for (tile, chunk) in mem.chunks_mut(tile_bytes as usize).enumerate() {
        if entry(&status, tile) == Some(TS_CLEAR_CODE) {
            for (i, b) in chunk.iter_mut().enumerate() {
                *b = clear_value.to_le_bytes()[i % 4];
            }
        }
    }
    let len = (h * pitch) as usize;
    let mut want = vec![f32::from_bits(SENTINEL); len];
    read_super_tiled(format, &mem, stride, w, h, &mut want, pitch);
    let mut got = vec![f32::from_bits(SENTINEL); len];
    read_super_tiled_ts(format, &surface, stride, w, h, &mut got, pitch, &t);
    for (i, (a, b)) in got.iter().zip(&want).enumerate() {
        if a.to_bits() != b.to_bits() {
            return Err(format!(
                "seed {seed:#x}: {w}x{h} pitch {pitch} tile {tile_bytes} clear {clear_value:#x}: pixel {i} got {:#x} want {:#x}",
                a.to_bits(),
                b.to_bits()
            ));
        }
    }
    Ok(())
}

#[test]
fn ts_read_equals_full_read_on_random_states() {
    let mut r = Rng(SEED);
    let mut bad = Vec::new();
    for _ in 0..CASES {
        if let Err(e) = case(r.next() | 1) {
            bad.push(e);
        }
    }
    assert!(bad.is_empty(), "{} of {CASES} cases differ, first: {}", bad.len(), bad[0]);
}
