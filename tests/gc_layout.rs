// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the tiled and super-tiled address maps of `src/vivante/layout.rs`.
//!
//! The expected offsets are worked out by hand from Mesa 26.2.1 etnaviv
//! (`src/gallium/drivers/etnaviv/`):
//!
//! - `etnaviv_rs.c:642`: TILED `(y & ~3) * stride + bpp * ((x & ~3) << 2)`;
//! - `etnaviv_rs.c:649`: SUPER_TILED
//!   `(y & ~63) * stride + bpp * ((x & ~63) << 6)`;
//! - `etnaviv_tiling.c:41-47`: the pixel inside a 4x4 tile is
//!   `(y & 3) * 4 + (x & 3)`;
//! - `etnaviv_resource.c:327-329`: padded width `align(w, paddingX)`, stride
//!   `padded_width * bpp`; `:384-393`: paddings 16x4 (RS) / 4x4 (TILED),
//!   64x64 (SUPER_TILED).
//!
//! The order of the 4x4 tiles inside a super-tile is not in Mesa (the module
//! doc says so); the super-tiled in-tile cases below pin the row-major
//! assumption, and the corner cases hold whatever the order is.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_layout
//! ```

#[path = "../src/vivante/layout.rs"]
#[allow(dead_code)]
mod layout;

use layout::*;

#[test]
fn tiled_offsets_follow_etnaviv_s_formula() {
    // 4 bytes a pixel, 16 pixels a row: stride 64.
    let (bpp, s) = (4, 64);
    assert_eq!(offset(Layout::Tiled, 0, 0, bpp, s), 0);
    assert_eq!(offset(Layout::Tiled, 1, 0, bpp, s), 4);
    // second row of the tile: 4 pixels on
    assert_eq!(offset(Layout::Tiled, 0, 1, bpp, s), 16);
    assert_eq!(offset(Layout::Tiled, 3, 3, bpp, s), 60);
    // next tile in the row: 16 pixels * 4 bytes
    assert_eq!(offset(Layout::Tiled, 4, 0, bpp, s), 64);
    // next tile row: 4 * stride
    assert_eq!(offset(Layout::Tiled, 0, 4, bpp, s), 256);
    // (5,5): tile row 1 = 256, tile column 1 = 64, pixel (1,1) = 5 * 4 = 20
    assert_eq!(offset(Layout::Tiled, 5, 5, bpp, s), 340);
    // last pixel of 16x4: tile column 3 (192) + pixel 15 (60)
    assert_eq!(offset(Layout::Tiled, 15, 3, bpp, s), 252);
}

#[test]
fn tiled_offsets_for_two_byte_pixels() {
    // 16 pixels * 2 bytes: stride 32. (5,5) = 4*32 + 2*(4<<2) + 2*5 = 170.
    assert_eq!(offset(Layout::Tiled, 5, 5, 2, 32), 170);
    assert_eq!(offset(Layout::Tiled, 1, 0, 2, 32), 2);
    assert_eq!(offset(Layout::Tiled, 0, 1, 2, 32), 8);
    assert_eq!(offset(Layout::Tiled, 0, 4, 2, 32), 128);
}

#[test]
fn super_tiled_offsets_follow_etnaviv_s_formula() {
    // 64 pixels * 4 bytes: stride 256, a super-tile is 16384 bytes.
    let (bpp, s) = (4, 256);
    assert_eq!(offset(Layout::SuperTiled, 0, 0, bpp, s), 0);
    assert_eq!(offset(Layout::SuperTiled, 3, 3, bpp, s), 60);
    // row-major tiles (assumed): the next tile is 64 bytes on, the first
    // tile of the next tile row is 16 tiles on.
    assert_eq!(offset(Layout::SuperTiled, 4, 0, bpp, s), 64);
    assert_eq!(offset(Layout::SuperTiled, 0, 4, bpp, s), 1024);
    // the last pixel of a super-tile is the last word of its 16384 bytes
    assert_eq!(offset(Layout::SuperTiled, 63, 63, bpp, s), 16380);

    // 128 pixels wide: stride 512.
    let s = 512;
    // x = 64 starts the next super-tile: 4 * (64 << 6) = 16384
    assert_eq!(offset(Layout::SuperTiled, 64, 0, bpp, s), 16384);
    // y = 64 starts the next super-tile row: 64 * stride = 32768
    assert_eq!(offset(Layout::SuperTiled, 0, 64, bpp, s), 32768);
    // (65,66): 16384 + 32768 + tile 0 + pixel (1,2) = 9 * 4 = 36
    assert_eq!(offset(Layout::SuperTiled, 65, 66, bpp, s), 49188);
}

#[test]
fn super_tiled_two_byte_pixels() {
    // 64 pixels * 2 bytes = stride 128; a super-tile is 8192 bytes.
    assert_eq!(offset(Layout::SuperTiled, 63, 63, 2, 128), 8190);
    // two super-tiles wide: stride 256; x = 64 -> 2 * (64 << 6) = 8192
    assert_eq!(offset(Layout::SuperTiled, 64, 0, 2, 256), 8192);
    assert_eq!(offset(Layout::SuperTiled, 0, 64, 2, 256), 64 * 256);
}

#[test]
fn linear_offsets() {
    assert_eq!(offset(Layout::Linear, 0, 0, 4, 64), 0);
    assert_eq!(offset(Layout::Linear, 3, 2, 4, 64), 2 * 64 + 12);
}

#[test]
fn padding_and_alignment_edge_cases() {
    // etnaviv_resource.c:384-393
    assert_eq!(padding(Layout::Tiled, true), (16, 4));
    assert_eq!(padding(Layout::Tiled, false), (4, 4));
    assert_eq!(padding(Layout::Linear, true), (16, 4));
    assert_eq!(padding(Layout::SuperTiled, true), (64, 64));
    assert_eq!(padding(Layout::SuperTiled, false), (64, 64));

    // 1x1
    assert_eq!((aligned_width(Layout::Tiled, 1, true), aligned_height(Layout::Tiled, 1, true)), (16, 4));
    assert_eq!(stride(Layout::Tiled, 1, 4, true), 64);
    assert_eq!(size(Layout::Tiled, 1, 1, 4, true), 256);
    assert_eq!(stride(Layout::Tiled, 1, 4, false), 16);
    assert_eq!(size(Layout::Tiled, 1, 1, 4, false), 64);
    assert_eq!(size(Layout::SuperTiled, 1, 1, 4, true), 16384);
    assert_eq!(size(Layout::SuperTiled, 1, 1, 2, true), 8192);

    // exact multiples stay put
    assert_eq!(aligned_width(Layout::SuperTiled, 64, true), 64);
    assert_eq!(aligned_width(Layout::SuperTiled, 65, true), 128);
    assert_eq!(aligned_height(Layout::SuperTiled, 64, true), 64);
    assert_eq!(aligned_height(Layout::Tiled, 4, true), 4);
    assert_eq!(aligned_height(Layout::Tiled, 5, true), 8);
    assert_eq!(aligned_width(Layout::Tiled, 16, true), 16);
    assert_eq!(aligned_width(Layout::Tiled, 17, true), 32);
    assert_eq!(aligned_width(Layout::Tiled, 17, false), 20);

    // 100x30, 2 bytes: 112 x 32, stride 224
    assert_eq!(stride(Layout::Tiled, 100, 2, true), 224);
    assert_eq!(size(Layout::Tiled, 100, 30, 2, true), 224 * 32);
    // glmark2's 1024x1024 D24S8 / RGBA8 surfaces need no padding
    assert_eq!(stride(Layout::SuperTiled, 1024, 4, true), 4096);
    assert_eq!(size(Layout::SuperTiled, 1024, 1024, 4, true), 4 << 20);
}

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    let mut v = 0x9e37_79b9u32 ^ seed;
    (0..n)
        .map(|_| {
            v = v.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (v >> 24) as u8
        })
        .collect()
}

#[test]
fn offsets_are_a_bijection_onto_the_padded_surface() {
    for layout in [Layout::Tiled, Layout::SuperTiled] {
        for bpp in [1u32, 2, 4] {
            let (w, h) = (aligned_width(layout, 70, true), aligned_height(layout, 70, true));
            let s = stride(layout, 70, bpp, true);
            let total = size(layout, 70, 70, bpp, true);
            let mut seen = vec![false; total];
            for y in 0..h {
                for x in 0..w {
                    let o = offset(layout, x, y, bpp, s);
                    for b in 0..bpp as usize {
                        assert!(!seen[o + b], "{:?} bpp {}: ({},{}) overlaps", layout, bpp, x, y);
                        seen[o + b] = true;
                    }
                }
            }
            assert!(seen.iter().all(|&b| b), "{:?} bpp {}: holes", layout, bpp);
        }
    }
}

#[test]
fn round_trips_linear_tiled_linear() {
    let sizes = [(1, 1), (3, 5), (4, 4), (15, 3), (16, 4), (17, 9), (63, 65), (64, 64), (65, 66), (100, 30), (130, 70)];
    for layout in [Layout::Tiled, Layout::SuperTiled] {
        for rs in [true, false] {
            for bpp in [1u32, 2, 4, 8] {
                for &(w, h) in &sizes {
                    let lin_stride = w * bpp;
                    let src = pattern((lin_stride * h) as usize, w * 31 + h);
                    let s = stride(layout, w, bpp, rs);
                    let mut laid = vec![0xa5u8; size(layout, w, h, bpp, rs)];
                    linear_to_layout(layout, &mut laid, s, &src, lin_stride, w, h, bpp);
                    let mut back = vec![0u8; src.len()];
                    layout_to_linear(layout, &mut back, lin_stride, &laid, s, w, h, bpp);
                    assert_eq!(back, src, "{:?} rs {} bpp {} {}x{}", layout, rs, bpp, w, h);
                    // the padding was not written
                    let written = laid.iter().filter(|&&b| b != 0xa5).count();
                    assert!(written <= src.len(), "{:?} {}x{}: wrote past the area", layout, w, h);
                }
            }
        }
    }
}

#[test]
fn a_tiled_copy_puts_pixels_where_the_formula_says() {
    // 6x5 pixels of 2 bytes: byte 0 is x*16+y, byte 1 is 0x80 | linear index.
    let (w, h, bpp) = (6u32, 5u32, 2u32);
    let mut src = vec![0u8; (w * h * bpp) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * bpp) as usize;
            src[i] = (x * 16 + y) as u8;
            src[i + 1] = 0x80 | (y * w + x) as u8;
        }
    }
    let s = stride(Layout::Tiled, w, bpp, true);
    assert_eq!(s, 32);
    let mut laid = vec![0u8; size(Layout::Tiled, w, h, bpp, true)];
    linear_to_layout(Layout::Tiled, &mut laid, s, &src, w * bpp, w, h, bpp);
    // pixel (5,4): tile row 1 = 4*32 = 128, tile column 1 = 2*(4<<2) = 32,
    // inside the tile (1,0) = 1 * 2 = 2 -> 162
    assert_eq!(&laid[162..164], &[5 * 16 + 4, 0x80 | (4 * 6 + 5)]);
    // pixel (2,3): tile row 0, column 0, inside (2,3) = 14 * 2 = 28
    assert_eq!(&laid[28..30], &[2 * 16 + 3, 0x80 | (3 * 6 + 2)]);
}

#[test]
fn a_short_linear_stride_is_refused() {
    let r = std::panic::catch_unwind(|| {
        let mut d = vec![0u8; 256];
        linear_to_layout(Layout::Tiled, &mut d, 64, &[0u8; 8], 4, 2, 1, 4);
    });
    assert!(r.is_err());
}
