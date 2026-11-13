// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The TS fast-clear and in-place-resolve word builders of
//! `src/vivante/rs_clear.rs` (W8/W9 groundwork, nothing calls them yet),
//! compared with Mesa's words in `tools/vivante/testdata/ts-clear.golden`
//! (cs_golden mode 29: a 64x64 RGBA8 target and a 64x64 D24S8 one, both TS
//! backed, cleared in one glClear to colour 0.25, 0.5, 0.75, 1.0, depth 0.75,
//! stencil 0x5a).
//!
//! The golden's core has no AUTO_DISABLE, so its records carry no
//! `TS_*_AUTO_DISABLE_COUNT`; the AUTO_DISABLE words are pinned separately.
//! The in-place resolve has no golden (the golden's resolve goes to a linear
//! staging buffer): it is pinned by structure only.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test gc_ts_fast_clear
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

#[path = "../src/vivante/regs.rs"]
mod regs;

#[path = "../src/vivante/rs_clear.rs"]
#[allow(dead_code)]
mod rs_clear;

use regs::*;
use rs_clear::*;
use yakogl::vivante::ts;

const GOLDEN: &str = include_str!("../tools/vivante/testdata/ts-clear.golden");

/// TS_MEM_CONFIG bits that are not the fast clear's: 0x40 DEPTH_COMPRESSION,
/// 0x80 COLOR_COMPRESSION, 0xf00 and 0x4000 are what Mesa's shim core writes
/// (0x43c3 against our 0x3), the compression open point of gc-tiled-wire.md
/// W8: the hwdb row says V4Compression 0 and this design programs none. They
/// are masked until ts-golden settles which row is right.
const COMPRESSION_BITS: u32 = 0x40 | 0x80 | 0xf00 | 0x4000;

/// One LOAD_STATE: the first register and its values.
type Rec = (u32, Vec<u32>);

fn decode(words: &[u32]) -> Vec<Rec> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let h = words[i];
        assert_eq!(h >> 27, 1, "word {i}: {h:#010x} is not a LOAD_STATE");
        let count = ((h >> 16) & 0x3ff) as usize;
        let reg = (h & 0xffff) << 2;
        out.push((reg, words[i + 1..i + 1 + count].to_vec()));
        i += 1 + count;
        if i % 2 == 1 {
            assert_eq!(words[i], PAD, "word {i} is not the pad");
            i += 1;
        }
    }
    out
}

/// The golden's LOAD_STATE records of its one submit (the `FIXP` marker and
/// the truncated `...` of long runs are not in the ones used here).
fn golden() -> Vec<Rec> {
    GOLDEN
        .lines()
        .filter(|l| l.starts_with("LOAD_STATE"))
        .map(|l| {
            let mut it = l.split_whitespace().skip(1);
            let reg = u32::from_str_radix(it.next().unwrap().trim_start_matches("0x"), 16).unwrap();
            let vals = it
                .skip_while(|t| !t.starts_with("count="))
                .skip(1)
                .filter(|t| t.starts_with("0x"))
                .map(|t| u32::from_str_radix(&t[2..], 16).unwrap())
                .collect();
            (reg, vals)
        })
        .collect()
}

/// Every (register, value) of the records, one entry each.
fn flat(recs: &[Rec]) -> Vec<(u32, u32)> {
    recs.iter().flat_map(|(r, v)| v.iter().enumerate().map(move |(i, x)| (r + 4 * i as u32, *x))).collect()
}

/// The first value the golden writes to `reg`.
fn golden_reg(g: &[(u32, u32)], reg: u32) -> u32 {
    g.iter().find(|(r, _)| *r == reg).unwrap_or_else(|| panic!("golden lacks {reg:#x}")).1
}

fn colour_target() -> TsTarget {
    TsTarget {
        ts_addr: 0xffff_6000,
        ts_layer_stride: ts::ts_size(64, 64, 4, true),
        surface_addr: 0xffff_7000,
        padded_width: 64,
        padded_height: 64,
    }
}

fn depth_target() -> TsTarget {
    TsTarget { ts_addr: 0xffff_1000, surface_addr: 0xffff_2000, ..colour_target() }
}

fn colour(auto_disable: bool, keep: u32) -> ColourFastClear {
    ColourFastClear {
        target: colour_target(),
        clear: ts::color_clear_words(ts::ColorFormat::Argb8888, [0.25, 0.5, 0.75, 1.0]),
        keep,
        auto_disable,
    }
}

fn depth(fmt: ts::DepthFormat, mask: ts::ClearMask, auto_disable: bool, keep: u32) -> DepthFastClear {
    DepthFastClear {
        target: depth_target(),
        bits: ts::depth_clear_bits(fmt, mask),
        clear_word: ts::depth_clear_word(fmt, 0.75, 0x5a),
        depth_16bpp: fmt == ts::DepthFormat::Z16,
        keep,
        auto_disable,
    }
}

const BOTH: ts::ClearMask = ts::ClearMask { depth: true, stencil: true };

/// The records of the golden between the first record after its `n`th
/// TS_FLUSH_CACHE or GL_FLUSH_CACHE-after-fill and the fill's closing
/// RS_SINGLE_BUFFER 0: one RS fill block.
fn golden_fill(g: &[Rec], from: usize) -> &[Rec] {
    let end = from + g[from..].iter().position(|(r, v)| *r == GC_RS_SINGLE_BUFFER && v[0] == 0).unwrap() + 1;
    &g[from..end]
}

#[test]
fn colour_fast_clear_matches_golden() {
    let g = golden();
    let gf = flat(&g);
    let w = colour_fast_clear(&colour(false, 1));
    let mine = decode(w.as_slice());

    // The prologue: GL_FLUSH_CACHE COLOR|DEPTH, the RA->PE stall, TS_FLUSH_CACHE.
    assert_eq!(&mine[..4], &g[..4]);
    // The TS fill of the colour TS: the golden's first RS block.
    let colour_fill = golden_fill(&g, 4);
    assert_eq!(colour_fill[0].0, GC_RS_CONFIG);
    assert_eq!(&mine[4..4 + colour_fill.len()], colour_fill);
    // The stall, then the TS registers: every one the golden writes.
    let n = 4 + colour_fill.len();
    assert_eq!((mine[n].0, &mine[n].1[..]), (GC_GL_SEMAPHORE_TOKEN, &[stall_token(GC_SYNC_RA, GC_SYNC_PE)][..]));
    assert_eq!(mine[n + 1].0, GC_GL_STALL_TOKEN);
    let regs = flat(&mine[n + 2..]);
    let order: Vec<u32> = regs.iter().map(|(r, _)| *r).collect();
    assert_eq!(
        order,
        [
            GC_TS_MEM_CONFIG,
            GC_TS_COLOR_STATUS_BASE,
            GC_TS_COLOR_SURFACE_BASE,
            GC_TS_COLOR_CLEAR_VALUE,
            GC_TS_COLOR_CLEAR_VALUE_EXT
        ]
    );
    for (reg, v) in &regs {
        let want = golden_reg(&gf, *reg);
        if *reg == GC_TS_MEM_CONFIG {
            // 0x43c3 (colour and depth fast clear) is the golden's; the
            // colour clear alone writes the colour bit, `keep` the depth one.
            assert_eq!(*v, want & !COMPRESSION_BITS, "TS_MEM_CONFIG");
        } else {
            assert_eq!(*v, want, "reg {reg:#x}");
        }
    }
    // The register run is one LOAD_STATE of four, then the EXT alone, as in the golden.
    assert_eq!(mine[n + 2].1.len(), 4);
    assert_eq!(mine[n + 3].0, GC_TS_COLOR_CLEAR_VALUE_EXT);
    assert_eq!(&g.iter().find(|r| r.0 == GC_TS_COLOR_CLEAR_VALUE_EXT).unwrap().1, &mine[n + 3].1);
}

#[test]
fn depth_fast_clear_matches_golden() {
    let g = golden();
    let gf = flat(&g);
    let w = depth_fast_clear(&depth(ts::DepthFormat::S8Z24, BOTH, false, GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR)).unwrap();
    let mine = decode(w.as_slice());

    assert_eq!(&mine[..4], &g[..4]);
    // The golden's second RS block, after the GL_FLUSH_CACHE between the fills.
    let first = golden_fill(&g, 4);
    let second_at = 4 + first.len();
    assert_eq!(g[second_at].0, GC_GL_FLUSH_CACHE);
    let depth_fill = golden_fill(&g, second_at + 1);
    assert_eq!(&mine[4..4 + depth_fill.len()], depth_fill);
    assert_eq!(depth_fill[3], (rs_pipe_dest_addr(0), vec![0xffff_1000]));

    let n = 4 + depth_fill.len();
    assert_eq!(mine[n].0, GC_GL_SEMAPHORE_TOKEN);
    assert_eq!(mine[n + 1].0, GC_GL_STALL_TOKEN);
    let tail = &mine[n + 2..];
    // TS_MEM_CONFIG alone, then the depth status, surface and clear value as one run.
    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0].0, GC_TS_MEM_CONFIG);
    assert_eq!(tail[0].1[0], golden_reg(&gf, GC_TS_MEM_CONFIG) & !COMPRESSION_BITS);
    assert_eq!(tail[1].0, GC_TS_DEPTH_STATUS_BASE);
    for (reg, v) in flat(&tail[1..]) {
        assert_eq!(v, golden_reg(&gf, reg), "reg {reg:#x}");
    }
    assert_eq!(tail[1].1, vec![0xffff_1000, 0xffff_2000, 0xbfff_ff5a]);
}

#[test]
fn colour_alone_and_depth_alone_or_to_the_golden_config() {
    let c = colour_fast_clear(&colour(false, 0));
    let d = depth_fast_clear(&depth(ts::DepthFormat::S8Z24, BOTH, false, 0)).unwrap();
    let cfg = |w: &Words| flat(&decode(w.as_slice())).into_iter().find(|(r, _)| *r == GC_TS_MEM_CONFIG).unwrap().1;
    assert_eq!(cfg(&c), GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR);
    assert_eq!(cfg(&d), GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR);
    assert_eq!(cfg(&c) | cfg(&d), golden_reg(&flat(&golden()), GC_TS_MEM_CONFIG) & !COMPRESSION_BITS);
}

#[test]
fn depth_needs_the_whole_word() {
    // D24S8, depth only: 0xeeee, a plain RS fill (golden mode 8), not TS.
    let depth_only = ts::ClearMask { depth: true, stencil: false };
    assert!(depth_fast_clear(&depth(ts::DepthFormat::S8Z24, depth_only, false, 0)).is_none());
    let stencil_only = ts::ClearMask { depth: false, stencil: true };
    assert!(depth_fast_clear(&depth(ts::DepthFormat::S8Z24, stencil_only, false, 0)).is_none());
    // Z16 and X8Z24 have 0xffff for depth alone.
    assert!(depth_fast_clear(&depth(ts::DepthFormat::Z16, depth_only, false, 0)).is_some());
    assert!(depth_fast_clear(&depth(ts::DepthFormat::X8Z24, depth_only, false, 0)).is_some());
}

#[test]
fn z16_sets_the_16bpp_bit_and_the_doubled_word() {
    let depth_only = ts::ClearMask { depth: true, stencil: false };
    let w = depth_fast_clear(&depth(ts::DepthFormat::Z16, depth_only, false, 0)).unwrap();
    let f = flat(&decode(w.as_slice()));
    let get = |r| f.iter().find(|(x, _)| *x == r).unwrap().1;
    assert_eq!(get(GC_TS_MEM_CONFIG), GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR | GC_TS_MEM_CONFIG_DEPTH_16BPP);
    // 0.75 * 65535 = 49151.25 -> 0xbfff, in both halves.
    assert_eq!(get(GC_TS_DEPTH_CLEAR_VALUE), 0xbfff_bfff);
}

#[test]
fn auto_disable_words() {
    let count = 64 * 64 / 16;
    let c = flat(&decode(colour_fast_clear(&colour(true, 0)).as_slice()));
    let get = |f: &[(u32, u32)], r| f.iter().find(|(x, _)| *x == r).unwrap().1;
    assert_eq!(get(&c, GC_TS_MEM_CONFIG), GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR | GC_TS_MEM_CONFIG_COLOR_AUTO_DISABLE);
    assert_eq!(get(&c, GC_TS_COLOR_AUTO_DISABLE_COUNT), count);

    let w = depth_fast_clear(&depth(ts::DepthFormat::S8Z24, BOTH, true, 0)).unwrap();
    let d = decode(w.as_slice());
    let f = flat(&d);
    assert_eq!(get(&f, GC_TS_MEM_CONFIG), GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR | GC_TS_MEM_CONFIG_DEPTH_AUTO_DISABLE);
    assert_eq!(get(&f, GC_TS_DEPTH_AUTO_DISABLE_COUNT), count);
    // 0x1664..0x1670 are one run of four.
    assert_eq!(d.last().unwrap().1.len(), 4);
}

#[test]
fn copied_formulas_match_ts_rs() {
    for (w, h) in [(16, 4), (64, 64), (800, 600), (1920, 1080), (256, 256)] {
        assert_eq!(auto_disable_count(w, h), ts::auto_disable_count(w, h));
        let stride = ts::ts_size(w, h, 4, true);
        let job = ts::ts_clear_job(stride);
        let mut words = Words::new();
        ts_fill(&mut words, 0x1000, stride);
        let f = flat(&decode(words.as_slice()));
        let get = |r| f.iter().find(|(x, _)| *x == r).unwrap().1;
        assert_eq!(get(GC_RS_DEST_STRIDE), job.dest_stride << 2);
        assert_eq!(get(GC_RS_WINDOW_SIZE), rs_window_width(job.width) | rs_window_height(job.height));
        assert_eq!(get(GC_RS_CLEAR_CONTROL), job.clear_control);
        assert_eq!(get(rs_fill_value(0)), job.fill);
        assert_eq!(words.len, FILL_WORDS);
    }
}

#[test]
fn inplace_resolve_is_a_super_tiled_self_copy_kicked_inplace() {
    let t = colour_target();
    let c = InplaceResolve {
        target: t,
        format: GC_RS_FORMAT_A8R8G8B8,
        stride: 64 * 4,
        tile_count: inplace_tile_count(64, 64),
        plane: InplacePlane::Colour { clear_value: 0xff40_80bf },
        keep: 0,
    };
    let w = inplace_resolve(&c);
    let recs = decode(w.as_slice());
    let f = flat(&recs);
    let get = |r| f.iter().find(|(x, _)| *x == r).unwrap().1;

    assert_eq!(recs[0], (GC_TS_FLUSH_CACHE, vec![GC_TS_FLUSH]));
    assert_eq!(get(GC_TS_MEM_CONFIG), GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR);
    assert_eq!(get(GC_TS_COLOR_STATUS_BASE), t.ts_addr);
    assert_eq!(get(GC_TS_COLOR_CLEAR_VALUE), 0xff40_80bf);
    // Source = destination, super-tiled, tile count as the kick.
    assert_eq!(get(rs_pipe_source_addr(0)), t.surface_addr);
    assert_eq!(get(rs_pipe_dest_addr(0)), t.surface_addr);
    assert_eq!(get(GC_RS_SOURCE_STRIDE) & 0xffff, 0x400);
    assert_eq!(get(GC_RS_SOURCE_STRIDE) & GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW, GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW);
    assert_eq!(get(GC_RS_DEST_STRIDE) & GC_RS_DEST_STRIDE_SUPER_TILED_NEW, GC_RS_DEST_STRIDE_SUPER_TILED_NEW);
    assert_eq!(get(GC_RS_CONFIG) & (GC_RS_SOURCE_TILED | GC_RS_DEST_TILED), GC_RS_SOURCE_TILED | GC_RS_DEST_TILED);
    assert_eq!(get(GC_RS_WINDOW_SIZE), 0x0040_0040);
    assert_eq!(get(GC_RS_CLEAR_CONTROL), 0);
    assert_eq!(get(GC_RS_KICKER_INPLACE), 256);
    assert!(!f.iter().any(|(r, _)| *r == GC_RS_KICKER || *r == GC_RS_SINGLE_BUFFER));
    assert_eq!(recs.last().unwrap().0, GC_RS_KICKER_INPLACE);

    let d = InplaceResolve {
        plane: InplacePlane::Depth { clear_value: 0xbfff_ff5a, depth_16bpp: true },
        target: depth_target(),
        ..c
    };
    let f = flat(&decode(inplace_resolve(&d).as_slice()));
    let get = |r| f.iter().find(|(x, _)| *x == r).unwrap().1;
    assert_eq!(get(GC_TS_MEM_CONFIG), GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR | GC_TS_MEM_CONFIG_DEPTH_16BPP);
    assert_eq!(get(GC_TS_DEPTH_STATUS_BASE), 0xffff_1000);
    assert_eq!(get(GC_TS_DEPTH_CLEAR_VALUE), 0xbfff_ff5a);
}
