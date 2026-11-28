// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/vivante/ts.rs`'s tile-status buffer sizes and tile modes against
//! etnaviv's, for the glmark2 surfaces: the 800x600 and 1920x1080 windows,
//! the offscreen shadow and refract targets (RGBA8 colour, D24S8 depth) and
//! odd sizes that are not a multiple of the supertile.
//!
//! The reference is a direct transcription of Mesa 26.2.1's
//! `src/gallium/drivers/etnaviv/` (the tree is not under `third_party/`; the
//! line numbers are those of `mesa-26.2.1`), written out here in the test
//! and not shared with `ts.rs`:
//!
//! * `etna_screen_get_tile_size`, etnaviv_screen.h:113-126: with
//!   CACHE128B256BPERLINE, 256 for `TS_MODE_256B`, else 128.
//! * `etna_screen_resource_alloc_ts`, etnaviv_resource.c:196-249: the mode is
//!   256B only if `ts_compress_fmt >= 0` (compression, which needs
//!   `specs.v4_compression`, :196-200, or MSAA without a modifier) and the
//!   layout is not linear or the stride is a multiple of 256 (:211-215);
//!   `ts_layer_stride = align(DIV_ROUND_UP(lvl->layer_stride,
//!   tile_size * 8 / bits_per_tile), 0x100 * pixel_pipes)` (:218-220).
//! * `bits_per_tile` is 4 with CACHE128B256BPERLINE (etnaviv_screen.c:880-882).
//! * `lvl->layer_stride` is the padded surface: `etna_layout_multiple`
//!   (etnaviv_resource.c:384-400) pads TILED to 16x4 and SUPER_TILED to
//!   64x64.
//!
//! The GC7000UL has no V4 compression (hwdb `V4Compression` 0) and these
//! surfaces are not MSAA, so the mode is always 128B and a TS byte covers
//! 256 surface bytes. The register words are out of scope (see
//! `tests/gc_tiled_regs_mesa.rs`).
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_ts_sizes`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::ts;

const BITS_PER_TILE: u32 = 4;
const PIXEL_PIPES: u32 = 1;

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

/// etnaviv_screen.h:113-126 (CACHE128B256BPERLINE set).
fn mesa_tile_size(mode: u32) -> u32 {
    if mode == 1 {
        256
    } else {
        128
    }
}

/// etnaviv_resource.c:206-217 with no compression format and no modifier.
fn mesa_mode(compress: bool, linear: bool, stride: u32) -> u32 {
    if compress && (!linear || stride % 256 == 0) {
        1
    } else {
        0
    }
}

/// etnaviv_resource.c:384-400 (`etna_layout_multiple`).
fn mesa_padded(w: u32, h: u32, super_tiled: bool) -> (u32, u32) {
    if super_tiled {
        (align(w, 64), align(h, 64))
    } else {
        (align(w, 16), align(h, 4))
    }
}

/// etnaviv_resource.c:218-220.
fn mesa_ts_layer_stride(layer_stride: u32, mode: u32) -> u32 {
    align(layer_stride.div_ceil(mesa_tile_size(mode) * 8 / BITS_PER_TILE), 0x100 * PIXEL_PIPES)
}

/// (name, width, height, bytes a pixel): RGBA8 colour is 4, D24S8 is 4.
const SURFACES: &[(&str, u32, u32, u32)] = &[
    ("window 800x600 RGBA8", 800, 600, 4),
    ("window 800x600 D24S8", 800, 600, 4),
    ("window 1920x1080 RGBA8", 1920, 1080, 4),
    ("window 1920x1080 D24S8", 1920, 1080, 4),
    // glmark2 shadow: a 1024x1024 map, refract: 512x512 (and the window-sized
    // offscreen of the default size).
    ("shadow map 1024x1024 RGBA8", 1024, 1024, 4),
    ("shadow map 1024x1024 D24S8", 1024, 1024, 4),
    ("refract 512x512 RGBA8", 512, 512, 4),
    ("refract 512x512 D24S8", 512, 512, 4),
    ("odd 1x1", 1, 1, 4),
    ("odd 63x63", 63, 63, 4),
    ("odd 65x65", 65, 65, 4),
    ("odd 100x100", 100, 100, 4),
    ("odd 333x77", 333, 77, 4),
    ("odd 1921x1081", 1921, 1081, 4),
    ("odd 800x601", 800, 601, 4),
];

#[test]
fn tile_modes_and_sizes() {
    // No compression on this part: 128B, 128 bytes a tile.
    assert_eq!(ts::TS_MODE_128B, 0);
    assert_eq!(ts::TS_MODE_256B, 1);
    assert_eq!(ts::tile_size(ts::TS_MODE_128B), mesa_tile_size(0));
    assert_eq!(ts::tile_size(ts::TS_MODE_256B), mesa_tile_size(1));
    assert_eq!(ts::BITS_PER_TILE, BITS_PER_TILE);
    assert_eq!(ts::PIXEL_PIPES, PIXEL_PIPES);
    // The choice for these surfaces (no compression, not MSAA): 128B in
    // every layout, linear stride or not.
    for (_, w, _, bpp) in SURFACES {
        assert_eq!(mesa_mode(false, false, 0), ts::TS_MODE_128B);
        assert_eq!(mesa_mode(false, true, w * bpp), ts::TS_MODE_128B);
    }
}

#[test]
fn padded_dims_match_layout_multiple() {
    for &(name, w, h, _) in SURFACES {
        for st in [false, true] {
            assert_eq!(ts::padded_dims(w, h, st), mesa_padded(w, h, st), "{name} super_tiled={st}");
        }
    }
}

#[test]
fn ts_buffer_size_matches_etnaviv() {
    for &(name, w, h, bpp) in SURFACES {
        for st in [false, true] {
            let (pw, ph) = mesa_padded(w, h, st);
            let want = mesa_ts_layer_stride(pw * ph * bpp, 0);
            assert_eq!(ts::ts_size(w, h, bpp, st), want, "{name} super_tiled={st}");
            assert_eq!(ts::ts_layer_stride(pw * ph * bpp, ts::TS_MODE_128B), want, "{name}");
            assert_eq!(want % 0x100, 0, "{name}: 0x100 aligned");
        }
    }
}

#[test]
fn ts_buffer_size_256b_mode() {
    // The 256B mode is never chosen here, but the formula covers it
    // (etnaviv_resource.c:218-220): one TS byte per 512 surface bytes.
    for &(name, w, h, bpp) in SURFACES {
        let (pw, ph) = mesa_padded(w, h, true);
        assert_eq!(
            ts::ts_layer_stride(pw * ph * bpp, ts::TS_MODE_256B),
            mesa_ts_layer_stride(pw * ph * bpp, 1),
            "{name}"
        );
    }
}

#[test]
fn known_sizes() {
    // Worked by hand from the formula: 800x600x4 TILED is 800x600 (16x4
    // aligned already) = 1_920_000 bytes, / 256 = 7500, aligned to 0x100 =
    // 7680. SUPER_TILED 832x640x4 = 2_129_920, / 256 = 8320 (0x2080 ->
    // 8448).
    assert_eq!(ts::ts_size(800, 600, 4, false), 7680);
    assert_eq!(ts::ts_size(800, 600, 4, true), 8448);
    // 1024x1024x4 = 4 MiB, / 256 = 16384 (already aligned).
    assert_eq!(ts::ts_size(1024, 1024, 4, false), 16384);
    assert_eq!(ts::ts_size(1024, 1024, 4, true), 16384);
    // 1x1 pads to 16x4x4 = 256 bytes -> 1 TS byte -> 0x100.
    assert_eq!(ts::ts_size(1, 1, 4, false), 0x100);
}
