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

// ---- case 243 tsprobe (docs/notes/gc-ts-probe.md, Part (iii)) ----

mod probe {
    use super::*;
    use yakogl::vivante::layout::{self, Layout, SuperOrder};

    fn b() -> Vec<u32> {
        (0..PROBE_DWORDS as u32).map(|i| 0xa500_0000 | i).collect()
    }

    /// A readback that is `YES`; each test spoils one thing.
    struct Case {
        b: Vec<u32>,
        k1: Vec<u32>,
        k2: Vec<u32>,
    }

    fn yes() -> Case {
        Case { b: b(), k1: vec![PROBE_FILL; 64], k2: vec![PROBE_CLEAR; PROBE_DWORDS] }
    }

    fn run(c: &Case, k0: &[u32], timeouts: u32, guard: u32, changed: bool) -> Outcome {
        classify(&ProbeReadback {
            fence_timeouts: timeouts,
            guard_changed: guard,
            surface_changed: changed,
            expected_b: &c.b,
            k0_d: k0,
            k1_t: &c.k1,
            k2_d: &c.k2,
            clear: PROBE_CLEAR,
        })
    }

    #[test]
    fn one_readback_per_outcome_code() {
        let c = yes();
        assert_eq!(run(&c, &c.b, 0, 0, false), Outcome::Yes);

        assert_eq!(run(&c, &c.b, 1, 0, false), Outcome::Hang);
        // A hang wins over everything else.
        assert_eq!(run(&c, &[], 2, 5, true), Outcome::Hang);

        assert_eq!(run(&c, &c.b, 0, 1, false), Outcome::Guard);
        assert_eq!(run(&c, &c.b, 0, 0, true), Outcome::Guard);

        let mut bad = c.b.clone();
        bad[100] ^= 1;
        assert_eq!(run(&c, &bad, 0, 0, false), Outcome::NoControl);
        assert_eq!(run(&c, &[PROBE_DEST_INIT; PROBE_DWORDS], 0, 0, false), Outcome::NoControl);

        let mut fm = yes();
        fm.k1[63] = 0;
        assert_eq!(run(&fm, &fm.b, 0, 0, false), Outcome::FillMiss);
        // Dwords of T past the fill are not looked at.
        let mut ok = yes();
        ok.k1.extend([0, 0]);
        assert_eq!(run(&ok, &ok.b, 0, 0, false), Outcome::Yes);

        let mut ig = yes();
        ig.k2 = ig.b.clone();
        assert_eq!(run(&ig, &ig.b, 0, 0, false), Outcome::Ignored);

        let mut nw = yes();
        nw.k2 = vec![PROBE_DEST_INIT; PROBE_DWORDS];
        assert_eq!(run(&nw, &nw.b, 0, 0, false), Outcome::NoWrite);

        let mut mx = yes();
        mx.k2[7] = mx.b[7];
        assert_eq!(run(&mx, &mx.b, 0, 0, false), Outcome::Mixed);
        let mut mx2 = yes();
        mx2.k2[0] = PROBE_DEST_INIT;
        assert_eq!(run(&mx2, &mx2.b, 0, 0, false), Outcome::Mixed);
    }

    #[test]
    fn outcome_values_and_tags() {
        let v = [
            (Outcome::Hang, 0xf),
            (Outcome::Guard, 1),
            (Outcome::NoControl, 2),
            (Outcome::FillMiss, 3),
            (Outcome::Yes, 4),
            (Outcome::Ignored, 5),
            (Outcome::NoWrite, 6),
            (Outcome::Mixed, 7),
        ];
        for (o, n) in v {
            assert_eq!(o as u32, n);
        }
        assert_eq!(probe_tag(0), 0xd0);
        assert_eq!(probe_tag(3), 0xd3);
        assert_eq!(PROBE_TAG_K3 + 3, 0xd7);
        assert_eq!(PROBE_TAG_DIFF + 3, 0xdb);
        assert_eq!(PROBE_CLEAR, color_clear_words(ColorFormat::Argb8888, [0.25, 0.5, 0.75, 1.0]).0);
    }

    #[test]
    fn page_words() {
        assert_eq!(probe_w1(Outcome::Yes, 2, 5, 0), 0x0000_0524);
        assert_eq!(probe_w1(Outcome::Hang, 1, 2, 3), 0x0003_021f);
        assert_eq!(probe_k3_w1(0x55, K3_LOW_NIBBLE_FIRST), 0x155);
        assert_eq!(probe_k3_w1(0xaa, 0), 0xaa);
        assert_eq!(probe_k3_w1(0x33, K3_LOW_NIBBLE_FIRST | K3_INTERLEAVED), 0x333);
        let d = b();
        let mut k2 = vec![PROBE_CLEAR; PROBE_DWORDS];
        assert_eq!(probe_counts(&k2, PROBE_CLEAR, &d), 4096 << 16);
        assert_eq!(probe_counts(&d, PROBE_CLEAR, &d), 4096);
        k2[..10].copy_from_slice(&d[..10]);
        assert_eq!(probe_counts(&k2, PROBE_CLEAR, &d), (4086 << 16) | 10);
        assert_eq!(probe_diff_w1(&k2, PROBE_CLEAR, &d), 0xffff_ffff);
        k2[20] = 0x1234_5678;
        assert_eq!(probe_diff_w1(&k2, PROBE_CLEAR, &d), (20 << 16) | 0x5678);
        let mut t = vec![PROBE_FILL; 64];
        assert_eq!(probe_diff_w3(&t, 0), 0xffff_0000);
        t[9] = 0;
        assert_eq!(probe_diff_w3(&t, 3), (9 << 16) | 3);
    }

    fn k3_ts() -> Vec<u8> {
        let mut t = vec![0u8; 256];
        t[..64].fill(0x01);
        t
    }

    #[test]
    fn k3_offset_matches_layout_rs() {
        for (kl, l, o) in [
            (K3Layout::Tiled, Layout::Tiled, SuperOrder::Drm),
            (K3Layout::SuperTiledDrm, Layout::SuperTiled, SuperOrder::Drm),
            (K3Layout::SuperTiledInterleaved, Layout::SuperTiled, SuperOrder::Interleaved),
        ] {
            for y in 0..64 {
                for x in 0..64 {
                    assert_eq!(k3_offset(kl, x, y, 256), layout::offset_in(l, o, x, y, 4, 256), "{kl:?} {x},{y}");
                }
            }
        }
    }

    #[test]
    fn k3_expected_map_tiled_128b() {
        // The note: low nibble first, row 0 has C in x [0,8), [16,24), ...
        let m = k3_expected_clear(K3Layout::Tiled, 64, 64, TS_MODE_128B, &k3_ts(), true);
        assert_eq!(k3_row0_map(&m, 64), 0x55);
        for x in 0..64u32 {
            assert_eq!(m[x as usize], (x / 8) % 2 == 0, "x {x}");
        }
        // High nibble first: the 0x01 bytes have a zero high nibble, so the
        // odd granules are the cleared ones.
        let h = k3_expected_clear(K3Layout::Tiled, 64, 64, TS_MODE_128B, &k3_ts(), false);
        assert_eq!(k3_row0_map(&h, 64), 0xaa);
        // Every pixel of a granule is in the same state, checked per offset.
        for y in 0..64 {
            for x in 0..64 {
                let g = layout::offset(Layout::Tiled, x, y, 4, 256) / 128;
                assert_eq!(m[(y * 64 + x) as usize], g % 2 == 0);
            }
        }
        // 16384 / 128 = 128 granules, the even ones cleared: half the pixels.
        assert_eq!(m.iter().filter(|&&c| c).count(), 2048);
    }

    #[test]
    fn k3_expected_map_tiled_256b() {
        let m = k3_expected_clear(K3Layout::Tiled, 64, 64, TS_MODE_256B, &k3_ts(), true);
        // Runs 16 wide: map bits over 8-pixel steps are 0x33.
        assert_eq!(k3_row0_map(&m, 64), 0x33);
        let h = k3_expected_clear(K3Layout::Tiled, 64, 64, TS_MODE_256B, &k3_ts(), false);
        assert_eq!(k3_row0_map(&h, 64), 0xcc);
    }

    #[test]
    fn k3_expected_map_super_tiled_both_orders() {
        for (kl, o) in
            [(K3Layout::SuperTiledDrm, SuperOrder::Drm), (K3Layout::SuperTiledInterleaved, SuperOrder::Interleaved)]
        {
            let m = k3_expected_clear(kl, 64, 64, TS_MODE_128B, &k3_ts(), true);
            for y in 0..64 {
                for x in 0..64 {
                    let g = layout::offset_in(Layout::SuperTiled, o, x, y, 4, 256) / 128;
                    assert_eq!(m[(y * 64 + x) as usize], g % 2 == 0, "{kl:?} {x},{y}");
                }
            }
        }
        // Finding: with 128B granules and the 0x01 pattern the parity of the
        // granule is pixel-index bit 5 (y2) in both orders, so the two maps
        // are equal and cannot tell the orders apart; 256B granules (bit 6:
        // y3 against x3) can.
        let map = |kl, mode| k3_expected_clear(kl, 64, 64, mode, &k3_ts(), true);
        assert_eq!(map(K3Layout::SuperTiledDrm, TS_MODE_128B), map(K3Layout::SuperTiledInterleaved, TS_MODE_128B));
        assert_ne!(map(K3Layout::SuperTiledDrm, TS_MODE_256B), map(K3Layout::SuperTiledInterleaved, TS_MODE_256B));
    }
}
