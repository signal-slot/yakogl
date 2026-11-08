// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the tiled and super-tiled address maps of `src/vivante/layout.rs`.
//!
//! The expected numbers are worked out by hand from Mesa 26.2.1:
//!
//! - `src/gallium/drivers/etnaviv/etnaviv_rs.c:642`: a TILED tile starts at
//!   `(y & ~3) * stride + bpp * ((x & ~3) << 2)`; `:649`: a SUPER_TILED
//!   super-tile at `(y & ~63) * stride + bpp * ((x & ~63) << 6)`;
//! - `etnaviv_tiling.c:41-47`: the pixel inside a 4x4 tile is
//!   `(y % 4) * 4 + x % 4`;
//! - `etnaviv_resource.c:327-331`: padded `align(w, paddingX)` and
//!   `align(h, paddingY)`, stride `padded_width * bpp`, size
//!   `stride * padded_height`; `:379-393`: paddings 16x4 (`rs_align`) or 4x4
//!   for LINEAR and TILED, 64x64 for SUPER_TILED;
//! - `include/drm-uapi/drm_fourcc.h:855-865`: a super-tile is 8x4 groups of
//!   2x4 tiles, all row-major (`SuperOrder::Drm`).
//!
//! `SuperOrder::Interleaved` is the module's unmeasured hypothesis for
//! SUPER_TILED_NEW; its cases pin the formula the module states, not a
//! source.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_layout
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

#[path = "../src/vivante/layout.rs"]
#[allow(dead_code)]
mod layout;

use layout::*;

/// `etna_compute_tileoffset` (`etnaviv_rs.c:626-652`), single pipe, written
/// out again: the byte offset of a tile- or super-tile-aligned corner.
fn mesa_tileoffset(layout: Layout, x: u32, y: u32, bpp: u32, stride: u32) -> usize {
    let (x, y, b, s) = (x as usize, y as usize, bpp as usize, stride as usize);
    match layout {
        Layout::Linear => y * s + x * b,
        Layout::Tiled => {
            assert!(x & 3 == 0 && y & 3 == 0);
            (y & !3) * s + b * ((x & !3) << 2)
        }
        Layout::SuperTiled => {
            assert!(x & 63 == 0 && y & 63 == 0);
            (y & !63) * s + b * ((x & !63) << 6)
        }
    }
}

#[test]
fn tiled_offsets_by_hand() {
    // 16 pixels a row, 4 bytes a pixel: stride 64, a tile row 256 bytes.
    let (bpp, s) = (4, 64);
    assert_eq!(offset(Layout::Tiled, 0, 0, bpp, s), 0);
    assert_eq!(offset(Layout::Tiled, 1, 0, bpp, s), 4);
    // second row inside the first tile: pixel 4 of the tile
    assert_eq!(offset(Layout::Tiled, 0, 1, bpp, s), 16);
    assert_eq!(offset(Layout::Tiled, 3, 3, bpp, s), 60);
    // the next tile of the row: 16 pixels * 4 bytes
    assert_eq!(offset(Layout::Tiled, 4, 0, bpp, s), 64);
    assert_eq!(offset(Layout::Tiled, 5, 1, bpp, s), 64 + 4 * 5);
    // the next tile row: 4 * stride
    assert_eq!(offset(Layout::Tiled, 0, 4, bpp, s), 256);
    // (6, 5): tile row 1 (256) + tile 1 (64) + pixel 1 * 4 + 2 (24)
    assert_eq!(offset(Layout::Tiled, 6, 5, bpp, s), 344);
    // 2 bytes a pixel, 16 pixels a row: stride 32: 4 * 32 + 2 * 16 + 2 * 6
    assert_eq!(offset(Layout::Tiled, 6, 5, 2, 32), 172);
}

#[test]
fn super_tiled_offsets_by_hand_drm_order() {
    // 128 pixels a row (two super-tiles), 4 bytes a pixel: stride 512.
    let (bpp, s) = (4, 512);
    let o = |x, y| offset(Layout::SuperTiled, x, y, bpp, s);
    assert_eq!(o(0, 0), 0);
    assert_eq!(o(3, 3), 4 * 15);
    // the second tile of a 2x4 group: 16 pixels on
    assert_eq!(o(4, 0), 4 * 16);
    // the group's second tile row: two tiles on
    assert_eq!(o(0, 4), 4 * 32);
    assert_eq!(o(4, 4), 4 * 48);
    // the second group of a group row: one 2x4 group (128 pixels) on
    assert_eq!(o(8, 0), 4 * 128);
    // the second group row: eight groups (1024 pixels) on
    assert_eq!(o(0, 16), 4 * 1024);
    // (13, 22): 1 + 2*4 | x2 16 | y2y3 4<<3=32 | group col 1 (128) | group row 1 (1024)
    assert_eq!(o(13, 22), 4 * 1209);
    // the last pixel of the first super-tile
    assert_eq!(o(63, 63), 4 * 4095);
    // the second super-tile of the row starts 64 * 64 pixels on
    assert_eq!(o(64, 0), 4 * 4096);
    // the second super-tile row starts 64 rows of stride on
    assert_eq!(o(0, 64), 64 * 512);
    // (65, 66): super-tile (1, 1) at 32768 + 16384, pixel 2 * 4 + 1 inside
    assert_eq!(o(65, 66), 49152 + 4 * 9);
    // 2 bytes a pixel
    assert_eq!(offset(Layout::SuperTiled, 13, 22, 2, 256), 2 * 1209);
}

#[test]
fn super_tiled_offsets_interleaved_hypothesis() {
    let i = |x, y| super_index(SuperOrder::Interleaved, x, y);
    assert_eq!(i(0, 0), 0);
    assert_eq!(i(4, 0), 16);
    assert_eq!(i(0, 4), 32);
    assert_eq!(i(8, 0), 64);
    assert_eq!(i(0, 8), 128);
    // (13, 22): 1 + 8 | x2 16 | y2 32 | x3 64 | y4 512
    assert_eq!(i(13, 22), 633);
    assert_eq!(i(63, 63), 4095);
    // the super-tile start is the same as Drm's
    assert_eq!(offset_in(Layout::SuperTiled, SuperOrder::Interleaved, 65, 66, 4, 512), 49152 + 4 * 9);
    // where the two orders part: Drm puts (8, 0) after one whole 2x4 group
    // (8 tiles), and (0, 8) after two tile rows of a group
    assert_eq!(super_index(SuperOrder::Drm, 8, 0), 128);
    assert_eq!(super_index(SuperOrder::Drm, 0, 8), 64);
}

#[test]
fn tile_corners_match_etna_compute_tileoffset() {
    for &bpp in &[1u32, 2, 4, 8] {
        let s = stride(Layout::Tiled, 100, bpp, true);
        for y in (0..40).step_by(4) {
            for x in (0..100).step_by(4) {
                assert_eq!(offset(Layout::Tiled, x, y, bpp, s), mesa_tileoffset(Layout::Tiled, x, y, bpp, s));
            }
        }
        let s = stride(Layout::SuperTiled, 200, bpp, true);
        for y in (0..256).step_by(64) {
            for x in (0..256).step_by(64) {
                for order in [SuperOrder::Drm, SuperOrder::Interleaved] {
                    assert_eq!(
                        offset_in(Layout::SuperTiled, order, x, y, bpp, s),
                        mesa_tileoffset(Layout::SuperTiled, x, y, bpp, s)
                    );
                }
            }
        }
    }
}

#[test]
fn alignment_and_stride_edge_cases() {
    // 1x1
    assert_eq!(aligned_width(Layout::Tiled, 1, true), 16);
    assert_eq!(aligned_width(Layout::Tiled, 1, false), 4);
    assert_eq!(aligned_height(Layout::Tiled, 1, true), 4);
    assert_eq!(aligned_width(Layout::Linear, 1, true), 16);
    assert_eq!(aligned_height(Layout::Linear, 1, false), 4);
    assert_eq!(aligned_width(Layout::SuperTiled, 1, false), 64);
    assert_eq!(aligned_height(Layout::SuperTiled, 1, true), 64);
    assert_eq!(stride(Layout::Tiled, 1, 2, true), 32);
    assert_eq!(size(Layout::Tiled, 1, 1, 2, true), 128);
    assert_eq!(stride(Layout::SuperTiled, 1, 4, true), 256);
    assert_eq!(size(Layout::SuperTiled, 1, 1, 4, true), 16384);
    // exact multiples stay, one more goes to the next
    assert_eq!(aligned_width(Layout::Tiled, 4, false), 4);
    assert_eq!(aligned_width(Layout::Tiled, 5, false), 8);
    assert_eq!(aligned_width(Layout::Tiled, 16, true), 16);
    assert_eq!(aligned_width(Layout::Tiled, 17, true), 32);
    assert_eq!(aligned_height(Layout::Tiled, 7, true), 8);
    assert_eq!(aligned_width(Layout::SuperTiled, 64, true), 64);
    assert_eq!(aligned_width(Layout::SuperTiled, 65, true), 128);
    assert_eq!(aligned_height(Layout::SuperTiled, 63, false), 64);
    // 100x100, 4 bytes: TILED 112 wide (448 bytes) by 100; SUPER 128x128
    assert_eq!(stride(Layout::Tiled, 100, 4, true), 448);
    assert_eq!(size(Layout::Tiled, 100, 100, 4, true), 448 * 100);
    assert_eq!(stride(Layout::SuperTiled, 100, 4, true), 512);
    assert_eq!(size(Layout::SuperTiled, 100, 100, 4, true), 512 * 128);
    // the glmark2 panel, 800x480 RGBA8 and D16
    assert_eq!(size(Layout::SuperTiled, 800, 480, 4, true), 832 * 4 * 512);
    assert_eq!(size(Layout::SuperTiled, 800, 480, 2, true), 832 * 2 * 512);
    assert_eq!(align_up(0, 4), 0);
}

/// Every pixel of the padded surface maps to its own `bpp`-byte slot and the
/// slots fill the surface exactly.
fn assert_bijection(layout: Layout, order: SuperOrder, w: u32, h: u32, bpp: u32, rs: bool) {
    let (pw, ph) = (aligned_width(layout, w, rs), aligned_height(layout, h, rs));
    let s = stride(layout, w, bpp, rs);
    let total = size(layout, w, h, bpp, rs);
    assert_eq!(total, (pw * ph * bpp) as usize);
    let mut seen = vec![false; total / bpp as usize];
    for y in 0..ph {
        for x in 0..pw {
            let o = offset_in(layout, order, x, y, bpp, s);
            assert_eq!(o % bpp as usize, 0);
            let slot = o / bpp as usize;
            assert!(slot < seen.len(), "{layout:?} {order:?} ({x}, {y}) {o} past {total}");
            assert!(!seen[slot], "{layout:?} {order:?} ({x}, {y}) maps twice to {o}");
            seen[slot] = true;
        }
    }
}

#[test]
fn padded_surfaces_are_bijections() {
    for &(w, h) in &[(1, 1), (5, 3), (17, 9), (64, 64), (65, 63), (130, 70)] {
        for &bpp in &[1, 2, 4, 8] {
            for rs in [false, true] {
                assert_bijection(Layout::Linear, SuperOrder::Drm, w, h, bpp, rs);
                assert_bijection(Layout::Tiled, SuperOrder::Drm, w, h, bpp, rs);
                assert_bijection(Layout::SuperTiled, SuperOrder::Drm, w, h, bpp, rs);
                assert_bijection(Layout::SuperTiled, SuperOrder::Interleaved, w, h, bpp, rs);
            }
        }
    }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(131) ^ (i >> 8)) as u8).collect()
}

fn round_trip(layout: Layout, order: SuperOrder, w: u32, h: u32, bpp: u32) {
    let lin_stride = w * bpp + 3; // a source row with trailing slack
    let src = pattern(lin_stride as usize * h as usize);
    let s = stride(layout, w, bpp, true);
    let mut laid = vec![0xa5u8; size(layout, w, h, bpp, true)];
    linear_to_layout(layout, order, &mut laid, s, &src, lin_stride, w, h, bpp);
    // the padding stays as it was: exactly w * h pixels were written
    let written = laid.chunks(bpp as usize).filter(|p| p.iter().any(|&b| b != 0xa5)).count();
    let untouched_src = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let o = (y * lin_stride + x * bpp) as usize;
            src[o..o + bpp as usize].iter().all(|&b| b == 0xa5)
        })
        .count();
    assert_eq!(written + untouched_src, (w * h) as usize);
    let mut back = vec![0u8; src.len()];
    layout_to_linear(layout, order, &mut back, lin_stride, &laid, s, w, h, bpp);
    for y in 0..h as usize {
        let row = y * lin_stride as usize;
        let n = (w * bpp) as usize;
        assert_eq!(back[row..row + n], src[row..row + n], "{layout:?} {order:?} {w}x{h} bpp {bpp} row {y}");
    }
}

#[test]
fn linear_to_tiled_to_linear_round_trips() {
    for &(w, h) in &[(1, 1), (3, 5), (4, 4), (17, 9), (64, 64), (65, 63), (100, 70)] {
        for &bpp in &[1, 2, 4, 8] {
            round_trip(Layout::Linear, SuperOrder::Drm, w, h, bpp);
            round_trip(Layout::Tiled, SuperOrder::Drm, w, h, bpp);
            round_trip(Layout::SuperTiled, SuperOrder::Drm, w, h, bpp);
            round_trip(Layout::SuperTiled, SuperOrder::Interleaved, w, h, bpp);
        }
    }
}

#[test]
fn conversion_puts_pixels_where_offset_says() {
    // 6x5, 2 bytes a pixel, pixel value = (y << 8) | x
    let (w, h, bpp) = (6u32, 5u32, 2u32);
    let mut src = Vec::new();
    for y in 0..h {
        for x in 0..w {
            src.extend_from_slice(&(((y << 8) | x) as u16).to_le_bytes());
        }
    }
    for layout in [Layout::Tiled, Layout::SuperTiled] {
        let s = stride(layout, w, bpp, false);
        let mut laid = vec![0u8; size(layout, w, h, bpp, false)];
        linear_to_layout(layout, SUPER_ORDER, &mut laid, s, &src, w * bpp, w, h, bpp);
        for y in 0..h {
            for x in 0..w {
                let o = offset(layout, x, y, bpp, s);
                assert_eq!(u16::from_le_bytes([laid[o], laid[o + 1]]), ((y << 8) | x) as u16);
            }
        }
    }
    // tiled, not rs-aligned: 8 pixels a row, so (5, 1) is tile 1, pixel 5:
    // 2 * (16 + 5) bytes
    let s = stride(Layout::Tiled, w, bpp, false);
    assert_eq!(s, 16);
    assert_eq!(offset(Layout::Tiled, 5, 1, bpp, s), 42);
}

#[test]
#[should_panic(expected = "bytes per pixel")]
fn zero_bpp_panics() {
    let mut dst = [0u8; 64];
    linear_to_layout(Layout::Tiled, SuperOrder::Drm, &mut dst, 16, &[0u8; 4], 4, 1, 1, 0);
}

#[test]
#[should_panic(expected = "linear stride")]
fn short_linear_stride_panics() {
    let mut dst = [0u8; 64];
    layout_to_linear(Layout::Tiled, SuperOrder::Drm, &mut dst, 3, &[0u8; 64], 16, 1, 1, 4);
}
