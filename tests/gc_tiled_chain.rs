// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: W11b's pure pieces (docs/notes/gc-tiled-wire.md), unwired:
//! `tex_layout::tiled_chain` over `tex_words`, and the CPU untile of a chain
//! level (`Chain::untile_level`, `layout::layout_to_linear` in the default
//! super-tile order, which is `layout::offset`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test gc_tiled_chain
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::layout;
use yakogl::vivante::tex_layout::*;

const TILED: [Layout; 2] = [Layout::Tiled, Layout::SuperTiled];

fn input(w: u32, h: u32, levels: u32, layout: Layout, rs_align: bool) -> Input {
    Input { width: w, height: h, levels, bpp: 4, layout, ts: Ts::None, rs_align, stride_texels: 0, base: 0 }
}

fn full_levels(w: u32, h: u32) -> u32 {
    32 - w.max(h).leading_zeros()
}

/// Every level's offset, pitch and span agree with `tex_words` of the chain,
/// and each level alone (`tex_words` of that size, one level) has the same
/// pitch and span.
#[test]
fn chain_levels_agree_with_tex_words() {
    for (w, h) in [(256, 256), (100, 37), (1, 1), (300, 7)] {
        for layout in TILED {
            for rs in [false, true] {
                let n = full_levels(w, h);
                let i = input(w, h, n, layout, rs);
                let c = tiled_chain(&i).unwrap();
                let words = tex_words(&i).unwrap();
                assert_eq!(c.words, words);
                assert_eq!((c.levels, c.bpp, c.layout), (n, 4, layout));
                for l in 0..n as usize {
                    let lv = c.level[l];
                    assert_eq!((lv.width, lv.height), ((w >> l).max(1), (h >> l).max(1)), "{w}x{h} {layout:?} l{l}");
                    assert_eq!(lv.offset, words.level_offset[l]);
                    assert_eq!(lv.stride, words.level_stride[l]);
                    assert_eq!(lv.offset % LOD_ALIGN, 0);
                    let one = tex_words(&input(lv.width, lv.height, 1, layout, rs)).unwrap();
                    assert_eq!(lv.stride, one.linear_stride, "{w}x{h} {layout:?} {rs} l{l}");
                    assert_eq!(lv.bytes, one.size, "{w}x{h} {layout:?} {rs} l{l}");
                    assert_eq!(lv.bytes, layout::size(layout, lv.width, lv.height, 4, rs) as u64);
                    if l + 1 < n as usize {
                        assert!(lv.offset + lv.bytes <= words.level_offset[l + 1]);
                    } else {
                        assert_eq!(lv.offset + lv.bytes, words.size);
                    }
                }
                for l in n as usize..MAX_LEVELS {
                    assert_eq!(c.level[l].bytes, 0);
                }
            }
        }
    }
}

/// Level 0 of a chain is the single-level layout: same words but the level
/// count, and the same placement.
#[test]
fn level_zero_is_the_single_level_layout() {
    for (w, h) in [(256, 256), (100, 37)] {
        for layout in TILED {
            for rs in [false, true] {
                let c = tiled_chain(&input(w, h, full_levels(w, h), layout, rs)).unwrap();
                let one = tiled_chain(&input(w, h, 1, layout, rs)).unwrap();
                assert_eq!(c.level[0], one.level[0]);
                assert_eq!(c.words.linear_stride, one.words.linear_stride);
                assert_eq!(c.words.slice, one.words.slice);
                assert_eq!(c.words.config0_addressing, one.words.config0_addressing);
                assert_eq!(c.words.config1_halign, one.words.config1_halign);
                assert_eq!(one.words.size, one.level[0].bytes);
            }
        }
    }
}

fn texel(level: u32, x: u32, y: u32) -> [u8; 4] {
    let v = (level << 24) ^ (y << 12) ^ x ^ 0x5a5a_0000u32.wrapping_mul(level + 1);
    v.to_le_bytes()
}

/// Tile then untile every level through the chain gives the texels back, and
/// each texel sits where `layout::offset` says inside its level.
#[test]
fn tile_untile_round_trips_every_level() {
    for (w, h) in [(256, 256), (100, 37)] {
        for layout in TILED {
            for rs in [false, true] {
                let c = tiled_chain(&input(w, h, full_levels(w, h), layout, rs)).unwrap();
                let mut buf = vec![0xeeu8; c.words.size as usize];
                let mut src = Vec::new();
                for l in 0..c.levels {
                    let lv = c.level[l as usize];
                    let mut lin = Vec::with_capacity((lv.width * lv.height * 4) as usize);
                    for y in 0..lv.height {
                        for x in 0..lv.width {
                            lin.extend_from_slice(&texel(l, x, y));
                        }
                    }
                    c.tile_level(l, &mut buf, &lin, lv.width * 4);
                    src.push(lin);
                }
                for l in 0..c.levels {
                    let lv = c.level[l as usize];
                    for (x, y) in [(0, 0), (lv.width - 1, lv.height - 1), (lv.width / 2, lv.height / 3)] {
                        let at = lv.offset as usize + layout::offset(layout, x, y, 4, lv.stride);
                        assert_eq!(buf[at..at + 4], texel(l, x, y), "{w}x{h} {layout:?} l{l} ({x},{y})");
                    }
                    let mut out = vec![0u8; (lv.width * lv.height * 4) as usize];
                    c.untile_level(l, &buf, &mut out, lv.width * 4);
                    assert_eq!(out, src[l as usize], "{w}x{h} {layout:?} {rs} l{l}");
                }
            }
        }
    }
}

#[test]
fn refusals_are_tex_words_and_linear() {
    let lin = Input { stride_texels: 16, ..input(16, 16, 1, Layout::Linear, false) };
    assert_eq!(tiled_chain(&lin), Err(Error::NotTiled));
    assert_eq!(tiled_chain(&input(16, 16, 0, Layout::Tiled, false)), Err(Error::Levels));
    assert_eq!(tiled_chain(&input(0, 16, 1, Layout::SuperTiled, false)), Err(Error::Empty));
    let ts = Input { ts: Ts::Sampler { fill: 0 }, ..input(16, 16, 2, Layout::Tiled, true) };
    assert_eq!(tiled_chain(&ts), Err(Error::TsMipped));
}
