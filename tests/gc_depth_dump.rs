// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Case 244's depth-layout override plan and dump pages (`src/vivante/depth_dump.rs`).
//!
//! Feature set: `cargo test --test gc_depth_dump --no-default-features
//! --features host-test,gpu-vivante`.
//!
//! The pages are checked against a Rust transcription of the C twin's
//! packing (`driver/tools/pvrlab/gl_test_probe_decode.inc`: `pd_pack16`,
//! `pd_pack_offsets`, `pd_pack_a`, `pd_pack_b`, `pd_pack_c`): the C words are
//! 32 bits and the Rust pages 64, the same stream of words, A w2 and w3 then
//! B w2 (offset words 0..=5 in order, pages carrying them low half first).

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::depth_dump::*;
use yakogl::vivante::layout::{self, Layout, SuperOrder};
use yakogl::vivante::regs::*;

const CLEAR: u32 = 0xffff_ff00;

fn ramp_bytes(map: &dyn Fn(u32, u32) -> usize) -> Vec<u8> {
    let mut s = vec![CLEAR; layout::DLAYOUT_DWORDS];
    for y in 0..64 {
        for x in 0..64 {
            s[map(x, y)] = layout::dlayout_expect(x, y) << 8;
        }
    }
    s.iter().flat_map(|d| d.to_le_bytes()).collect()
}

fn row(x: u32, y: u32) -> usize {
    (y * 64 + x) as usize
}
fn sup(order: SuperOrder) -> impl Fn(u32, u32) -> usize {
    move |x, y| layout::offset_in(Layout::SuperTiled, order, x, y, 4, 256) / 4
}

// ---- the C twin, transcribed ----
fn pd_pack16(lo: u32, hi: u32) -> u32 {
    (lo & 0xffff) | (hi & 0xffff) << 16
}
fn pd_pack_offsets(o: &[u16; 12]) -> [u32; 6] {
    std::array::from_fn(|k| pd_pack16(o[2 * k] as u32, o[2 * k + 1] as u32))
}
fn pd_pack_a(d: &Dump, mode: u32, timeouts: u32) -> [u32; 3] {
    let o = pd_pack_offsets(&d.result.offsets);
    let code = if timeouts != 0 { 0xf } else { d.result.code as u32 };
    [(code & 0xf) | (mode & 0xf) << 4 | timeouts.min(255) << 8 | d.result.matched << 16, o[0], o[1]]
}
fn pd_pack_b(d: &Dump) -> [u32; 3] {
    let o = pd_pack_offsets(&d.result.offsets);
    let (i, z) = d.result.first_non_clear.unwrap_or((0xffff, 0));
    [d.result.crc, o[2], pd_pack16(z as u32, i)]
}
fn pd_pack_c(d: &Dump) -> [u32; 3] {
    let o = pd_pack_offsets(&d.result.offsets);
    [o[3], o[4], o[5]]
}

/// The Rust pages as the C twin's 32-bit words: A w1, B w1 and B w3 are
/// the same word; the 64-bit w2/w3 words of A and B hold, low half first,
/// the C words A w2, A w3, B w2 and C w1..w3.
fn as_c_words(d: &Dump) -> ([u32; 3], [u32; 3], [u32; 3]) {
    let (a, b) = (d.page_a.1, d.page_b.1);
    let lo = |w: u64| w as u32;
    let hi = |w: u64| (w >> 32) as u32;
    ([a[0] as u32, lo(a[1]), hi(a[1])], [b[0] as u32, lo(a[2]), b[2] as u32], [hi(a[2]), lo(b[1]), hi(b[1])])
}

#[test]
fn plan_is_the_notes_table() {
    let want = [(2, false), (2, true), (1, true), (1, false)];
    for (m, (sb, st)) in want.into_iter().enumerate() {
        let p = plan(m as u32).unwrap();
        assert_eq!((p.mode, p.single_buffer, p.super_tiled), (m as u32, sb, st));
        assert_eq!(p.depth_config() & GC_PE_DEPTH_SUPER_TILED != 0, st);
        assert_eq!(p.logic_op_bits(), pe_logic_op_single_buffer(sb));
        assert_eq!(p.layout(), if st { Layout::SuperTiled } else { Layout::Linear });
        // D24S8, GL_ALWAYS, writes on; no early depth, no DISABLE_ZS.
        let c = p.depth_config() & !GC_PE_DEPTH_SUPER_TILED;
        assert_eq!(c, 0x0004_1711);
        assert_eq!(c & (GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS), 0);
    }
    assert!(plan(MODES).is_none());
    assert_eq!(layout::stride(Layout::Linear, 64, 4, false), 256);
}

#[test]
fn ramp_under_each_mode_gives_its_pages() {
    // (mode, the map the PE is assumed to write, the code)
    let cases: [(u32, Box<dyn Fn(u32, u32) -> usize>, u64); 4] = [
        (3, Box::new(row), 2),
        (0, Box::new(row), 2),
        (1, Box::new(sup(SuperOrder::Drm)), 4),
        (2, Box::new(sup(SuperOrder::Interleaved)), 5),
    ];
    for (mode, map, code) in cases {
        let bytes = ramp_bytes(&*map);
        let d = dump(mode, &bytes, CLEAR, 0).unwrap();
        assert_eq!(d.page_a.0, 0xc0 + mode as u8);
        assert_eq!(d.page_b.0, 0xc4 + mode as u8);
        assert_eq!(d.page_a.1[0], code | (mode as u64) << 4 | 4096 << 16, "mode {mode}");
        // offsets are where the references sit in the map
        for (o, &(x, y)) in d.result.offsets.iter().zip(layout::DLAYOUT_REF_PIXELS.iter()) {
            assert_eq!(*o as usize, map(x, y), "mode {mode} ({x},{y})");
        }
        // the first dword that is not the clear value, and its z byte
        let (i, z) = d.result.first_non_clear.unwrap();
        assert_eq!(i, 0);
        assert_eq!(z as u32, (layout::dlayout_expect(0, 0) << 8) & 0xff, "z byte of the first dword");
        assert_eq!(d.page_b.1[2], (i as u64) << 16 | z as u64);
        assert_eq!(d.page_b.1[0], d.result.crc as u64);
    }
}

#[test]
fn pages_match_the_c_twin_packing() {
    let maps: [Box<dyn Fn(u32, u32) -> usize>; 3] =
        [Box::new(row), Box::new(sup(SuperOrder::Drm)), Box::new(sup(SuperOrder::Interleaved))];
    let mut surfaces: Vec<Vec<u8>> = maps.iter().map(|m| ramp_bytes(&**m)).collect();
    surfaces.push(CLEAR.to_le_bytes().repeat(layout::DLAYOUT_DWORDS)); // nothing written
    let mut odd = surfaces[0].clone();
    odd[4 * 100..4 * 100 + 4].copy_from_slice(&CLEAR.to_le_bytes()); // a missing id
    surfaces.push(odd);
    for (n, bytes) in surfaces.iter().enumerate() {
        for mode in 0..MODES {
            for timeouts in [0, 3, 300] {
                let d = dump(mode, bytes, CLEAR, timeouts).unwrap();
                let (a, b, c) = as_c_words(&d);
                assert_eq!(a, pd_pack_a(&d, mode, timeouts), "surface {n} mode {mode} page A");
                assert_eq!(b, pd_pack_b(&d), "surface {n} mode {mode} page B");
                assert_eq!(c, pd_pack_c(&d), "surface {n} mode {mode} page C");
            }
        }
    }
}

#[test]
fn nothing_written_and_timeouts() {
    let bytes = CLEAR.to_le_bytes().repeat(layout::DLAYOUT_DWORDS);
    let d = dump(0, &bytes, CLEAR, 0).unwrap();
    assert_eq!(d.page_a.1[0] & 0xf, 1, "NO_WRITE");
    assert_eq!(d.result.first_non_clear, None);
    assert_eq!(d.page_b.1[2], 0xffff << 16);
    let h = dump(1, &bytes, CLEAR, 2).unwrap();
    assert_eq!(h.page_a.1[0] & 0xfff, 0xf | 1 << 4 | 2 << 8, "HANG with 2 timeouts");
}

#[test]
fn refuses_a_bad_mode_or_short_readback() {
    let bytes = vec![0u8; layout::DLAYOUT_DWORDS * 4];
    assert!(dump(MODES, &bytes, CLEAR, 0).is_none());
    assert!(dump(0, &bytes[..bytes.len() - 4], CLEAR, 0).is_none());
}
