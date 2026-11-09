// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL tile-status sizes and fast-clear words (`src/vivante/ts.rs`).
//!
//! Expected values are worked out by hand from Mesa 26.2.1 etnaviv
//! (`src/gallium/drivers/etnaviv/`), cited per test. The golden stream of
//! item ts-golden (`tools/vivante/testdata/ts-clear*`) was not on the
//! integration tip when this was written, so no check against it is made.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::ts::*;

#[test]
fn register_addresses_match_regs_rs() {
    let regs = include_str!("../src/vivante/regs.rs");
    for (name, val) in [
        ("GC_PE_MEM_CONFIG", PE_MEM_CONFIG),
        ("GC_TS_FLUSH_CACHE", TS_FLUSH_CACHE),
        ("GC_TS_MEM_CONFIG", TS_MEM_CONFIG),
        ("GC_RS_CLEAR_CONTROL", RS_CLEAR_CONTROL),
    ] {
        let want = format!("pub const {name}: u32 = 0x{val:08x};");
        assert!(regs.contains(&want), "regs.rs lacks `{want}`");
    }
}

#[test]
fn no_crate_or_super_paths() {
    let src = include_str!("../src/vivante/ts.rs");
    for l in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
        assert!(!l.contains("crate::") && !l.contains("super::"), "{l}");
    }
}

#[test]
fn ts_size_matches_alloc_ts() {
    // etnaviv_resource.c:241-249: ceil(stride / (128*8/4 = 256)), aligned 0x100.
    // 1600x1200 D24S8, TILED (16x4 padding keeps the size): 7_680_000 / 256
    // = 30000 -> 30208. The note says "30 KB".
    assert_eq!(ts_size(1600, 1200, 4, false), 30208);
    // 800x600 RGBA8: 1_920_000 / 256 = 7500 -> 7680.
    assert_eq!(ts_size(800, 600, 4, false), 7680);
    // SUPER_TILED pads to 64x64 (:389-393): 1600x1216x4 / 256 = 30400 -> 30464.
    assert_eq!(padded_dims(1600, 1200, true), (1600, 1216));
    assert_eq!(ts_size(1600, 1200, 4, true), 30464);
    // TILED pads to 16x4: 30x5 -> 32x8.
    assert_eq!(padded_dims(30, 5, false), (32, 8));
    // 256B tiles (compression only): 512 bytes of surface per TS byte.
    assert_eq!(ts_layer_stride(7_680_000, TS_MODE_256B), 15104);
    // Tiny surfaces round up to the 0x100 alignment; none gives none (:252).
    assert_eq!(ts_layer_stride(1, TS_MODE_128B), 0x100);
    assert_eq!(ts_layer_stride(0, TS_MODE_128B), 0);
    assert_eq!(tile_size(TS_MODE_128B), 128);
    assert_eq!(tile_size(TS_MODE_256B), 256);
}

#[test]
fn ts_clear_job_matches_rs_gen_ts_clear_cmd() {
    // etnaviv_rs.c:339-358: width 16, stride 0x40, height align(ts/0x40, 4),
    // fill 0x11111111 (etnaviv_screen.c:884-886), clear 0xffff ENABLED1.
    let j = ts_clear_job(30208);
    assert_eq!(j.width, 16);
    assert_eq!(j.dest_stride, 0x40);
    assert_eq!(j.height, 472);
    assert_eq!(j.fill, 0x1111_1111);
    assert_eq!(j.clear_control, 0x0001_ffff);
    // The job covers exactly the TS buffer: 16 * 4 bytes * rows = 64 * rows.
    assert_eq!(j.width * 4 * j.height, 30208);
    // etnaviv_rs.c:382-387: tiles = padded w * h / 16.
    assert_eq!(auto_disable_count(1600, 1200), 120_000);
}

#[test]
fn colour_words_match_pack_rgba() {
    // etnaviv_clear_blit.c:91-111: pack, then repeat 1/2-byte texels to 32 bits.
    let red = [1.0, 0.0, 0.0, 1.0];
    assert_eq!(color_clear_words(ColorFormat::Argb8888, red), (0xffff_0000, 0xffff_0000));
    assert_eq!(color_clear_words(ColorFormat::Argb8888, [0.2, 0.4, 0.6, 0.8]).0, 0xcc33_6699);
    assert_eq!(color_clear_words(ColorFormat::Argb8888, [0.0; 4]).0, 0);
    assert_eq!(color_clear_words(ColorFormat::Xrgb8888, [0.0; 4]).0, 0xff00_0000);
    assert_eq!(color_clear_words(ColorFormat::Rgb565, red).0, 0xf800_f800);
    assert_eq!(color_clear_words(ColorFormat::Rgb565, [1.0; 4]).0, 0xffff_ffff);
    assert_eq!(color_clear_words(ColorFormat::Argb4444, red).0, 0xff00_ff00);
    assert_eq!(color_clear_words(ColorFormat::Argb1555, red).0, 0xfc00_fc00);
    assert_eq!(color_clear_words(ColorFormat::R8, [0.5, 0.0, 0.0, 0.0]).0, 0x8080_8080);
    // Out-of-range and NaN inputs clamp.
    assert_eq!(color_clear_words(ColorFormat::R8, [2.0, 0.0, 0.0, 0.0]).0, 0xffff_ffff);
    assert_eq!(color_clear_words(ColorFormat::R8, [f32::NAN, 0.0, 0.0, 0.0]).0, 0);
}

#[test]
fn depth_words_match_pack_z_stencil() {
    // util/u_pack_color.h:546-580 and :613-632, etnaviv_translate.h:481-492.
    assert_eq!(depth_clear_word(DepthFormat::S8Z24, 1.0, 0), 0xffff_ff00);
    assert_eq!(depth_clear_word(DepthFormat::S8Z24, 0.0, 0), 0);
    assert_eq!(depth_clear_word(DepthFormat::S8Z24, 1.0, 0x5a), 0xffff_ff5a);
    // 0.5 * 0xffffff = 8388607.5, lrint ties to even -> 0x800000.
    assert_eq!(depth_clear_word(DepthFormat::S8Z24, 0.5, 1), 0x8000_0001);
    // X8Z24 ignores stencil.
    assert_eq!(depth_clear_word(DepthFormat::X8Z24, 1.0, 0x5a), 0xffff_ff00);
    // Z16 is repeated in both halves; 0.5 * 0xffff = 32767.5 -> 32768.
    assert_eq!(depth_clear_word(DepthFormat::Z16, 1.0, 0), 0xffff_ffff);
    assert_eq!(depth_clear_word(DepthFormat::Z16, 0.5, 0), 0x8000_8000);
    assert_eq!(depth_clear_word(DepthFormat::Z16, 0.0, 0), 0);
}

#[test]
fn depth_clear_bits_and_ts_path() {
    // etnaviv_rs.c:446-467.
    let d = ClearMask { depth: true, stencil: false };
    let s = ClearMask { depth: false, stencil: true };
    let ds = ClearMask { depth: true, stencil: true };
    let none = ClearMask { depth: false, stencil: false };
    assert_eq!(depth_clear_bits(DepthFormat::Z16, d), 0xffff);
    assert_eq!(depth_clear_bits(DepthFormat::X8Z24, d), 0xffff);
    assert_eq!(depth_clear_bits(DepthFormat::S8Z24, d), 0xeeee);
    assert_eq!(depth_clear_bits(DepthFormat::S8Z24, s), 0x1111);
    assert_eq!(depth_clear_bits(DepthFormat::S8Z24, ds), 0xffff);
    assert_eq!(depth_clear_bits(DepthFormat::S8Z24, none), 0);
    assert!(depth_clear_uses_ts(DepthFormat::Z16, d));
    assert!(depth_clear_uses_ts(DepthFormat::S8Z24, ds));
    assert!(!depth_clear_uses_ts(DepthFormat::S8Z24, d));
    // The depth-only D24S8 RS fill: RS_CLEAR_CONTROL 0x0001eeee
    // (tools/vivante/README.md, "How the two drivers clear depth").
    assert_eq!(rs_clear_control(0xeeee), 0x0001_eeee);
}

#[test]
fn register_values() {
    // etnaviv_state.c:233-235, :458; hw/state_3d.xml.h:1180-1185.
    assert_eq!(pe_mem_config(None, None), 0);
    assert_eq!(pe_mem_config(Some(TS_MODE_128B), Some(TS_MODE_128B)), 0);
    assert_eq!(pe_mem_config(Some(TS_MODE_256B), None), 0x0100_0000);
    assert_eq!(pe_mem_config(None, Some(TS_MODE_256B)), 0x0400_0000);
    assert_eq!(pe_mem_config(Some(1), Some(1)), 0x0500_0000);

    // TS_MEM_CONFIG: etnaviv_state.c:313, :468, :903, :936; etnaviv_rs.c:385, :473.
    let base = TsMemConfig::default();
    assert_eq!(ts_mem_config(base), 0);
    // Colour fast-cleared RGBA8 + D24S8 fast-cleared, both with auto disable.
    let c = TsMemConfig {
        color_valid: true,
        depth_valid: true,
        color_auto_disable: true,
        depth_auto_disable: true,
        ..base
    };
    assert_eq!(ts_mem_config(c), 0x33);
    // Z16 depth only, cleared: DEPTH_16BPP | DEPTH_FAST_CLEAR | DEPTH_AUTO_DISABLE.
    let z = TsMemConfig { depth_16bpp: true, depth_valid: true, depth_auto_disable: true, ..base };
    assert_eq!(ts_mem_config(z), 0x19);
    let w = TsMemConfig { color_64bpp: true, color_valid: true, ..base };
    assert_eq!(ts_mem_config(w), 0x4000_0002);
}
