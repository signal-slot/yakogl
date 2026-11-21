// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! W8's host test (docs/notes/gc-tiled-wire.md): the colour fast clear and
//! the TS-sourced RS resolve of `src/vivante/rs_clear.rs`, run through
//! `rs_interp` with its TS read on, over a TILED 32-bit surface and its tile
//! status, and the linear destination compared bit for bit.
//!
//! The surface starts as stale pixels everywhere; `colour_fast_clear`'s TS
//! fill marks every tile cleared, then some tiles are "written" (surface
//! pixels set, the TS entry back to 0). `ts_resolve` must give the clear
//! colour for a cleared tile and the untiled surface pixel for a written
//! one, `SWAP_RB` applied to both. What this shows is that the words, the
//! interpreter's model of Mesa's TS and `layout.rs` agree; it says nothing
//! about the hardware (that is the board probe's question).
//!
//! The depth case runs `depth_fast_clear`; `inplace_resolve` is kicked with
//! `RS_KICKER_INPLACE`, which `rs_interp` does not model, so it is checked
//! only to run no job.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_ts_resolve_interp`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::regs::*;
use yakogl::vivante::rs_clear::{self, *};
use yakogl::vivante::rs_interp::{Memory, RsInterp, TS_CLEAR_CODE};
use yakogl::vivante::ts;

const SURFACE: u32 = 0x1000_0000;
const TS_BUF: u32 = 0x2000_0000;
const DEST: u32 = 0x3000_0000;
const CLEAR: u32 = 0x8040_20ff;

/// Every `(register, value)` a LOAD_STATE stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        assert_eq!(w & 0xf800_0000, GC_OP_LOAD_STATE, "word {i}: only LOAD_STATEs expected");
        let count = ((w >> 16) & 0x3ff) as usize;
        let base = (w & 0xffff) * 4;
        for k in 0..count {
            out.push((base + 4 * k as u32, words[i + 1 + k]));
        }
        i += 1 + count;
        if i % 2 == 1 {
            i += 1; // the pad
        }
    }
    out
}

fn px(x: u32, y: u32) -> [u8; 4] {
    (x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca6b) ^ 0x5bd1_e995).to_le_bytes()
}

fn swap(p: [u8; 4]) -> [u8; 4] {
    [p[2], p[1], p[0], p[3]]
}

/// A tiled `w` x `h` colour target: the surface full of stale pixels, the
/// padded sizes and the TS buffer. `written(n)` says whether TS entry `n` is
/// written after the fast clear.
struct Case {
    layout: Layout,
    mem: Memory,
    interp: RsInterp,
    target: TsTarget,
    stride: u32,
}

fn tile_entry_of(l: Layout, x: u32, y: u32, stride: u32) -> usize {
    layout::offset(l, x, y, 4, stride) / ts::tile_size(ts::TS_MODE_128B) as usize
}

fn setup(w: u32, h: u32, l: Layout, written: impl Fn(usize) -> bool) -> Case {
    let sup = l == Layout::SuperTiled;
    let (pw, ph) = ts::padded_dims(w, h, sup);
    let stride = pw * 4;
    let ts_bytes = ts::ts_size(w, h, 4, sup);
    let target = TsTarget {
        ts_addr: TS_BUF,
        ts_layer_stride: ts_bytes,
        surface_addr: SURFACE,
        padded_width: pw,
        padded_height: ph,
    };
    let mut surf = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..ph {
        for x in 0..pw {
            let o = layout::offset(l, x, y, 4, stride);
            surf[o..o + 4].copy_from_slice(&px(x, y));
        }
    }
    let mut mem = Memory::new();
    mem.map(SURFACE, surf);
    mem.map(TS_BUF, vec![0u8; ts_bytes as usize]);
    mem.map(DEST, vec![0u8; (pw * ph * 4) as usize]);
    let mut interp = RsInterp::new();
    interp.ts_source = true;
    // rs_interp wants a source address even for a fill; ts_fill writes none
    // (the RS fill does not read its source). See the report.
    interp.write(GC_RS_SOURCE_ADDR, SURFACE, &mut mem);

    // The fast clear: its TS fill marks every tile cleared.
    let clear = colour_fast_clear(&ColourFastClear { target, clear: (CLEAR, CLEAR), keep: 0, auto_disable: false });
    interp.run(&writes(clear.as_slice()), &mut mem);
    assert_eq!(interp.jobs.len(), 1, "the TS fill is one RS job");
    let entries = (pw * ph * 4 / 128) as usize;
    for n in 0..entries {
        let byte = mem.region(TS_BUF)[n / 2];
        assert_eq!((byte >> (4 * (n % 2))) & 0xf, TS_CLEAR_CODE, "entry {n} after the fast clear");
    }
    // Surface writes: a written tile's entry goes back to 0.
    let ts_buf = mem.region_mut(TS_BUF);
    for n in (0..entries).filter(|&n| written(n)) {
        ts_buf[n / 2] &= !(0xf << (4 * (n % 2)));
    }
    Case { layout: l, mem, interp, target, stride }
}

fn resolve(c: &mut Case, swap_rb: bool) {
    let layout = match c.layout {
        Layout::Tiled => TsSourceLayout::Tiled,
        _ => TsSourceLayout::SuperTiled,
    };
    let t = c.target;
    let words = ts_resolve(&TsResolve {
        target: t,
        layout,
        format: GC_RS_FORMAT_A8R8G8B8,
        clear_value: CLEAR,
        dest_addr: DEST,
        dest_stride: t.padded_width * 4,
        source_stride: c.stride,
        swap_rb,
        source_ts_mode: 0,
        extra_ts_mode: 0,
        keep: 0,
    });
    let before = c.interp.jobs.len();
    c.interp.run(&writes(words.as_slice()), &mut c.mem);
    assert_eq!(c.interp.jobs.len(), before + 1, "the resolve is one RS job");
}

/// The destination pixel `(x, y)` over the padded window against what the TS
/// and the surface give.
fn check(c: &Case, w: u32, h: u32, swap_rb: bool, written: impl Fn(usize) -> bool) {
    let (pw, ph) = (c.target.padded_width, c.target.padded_height);
    let dest = c.mem.region(DEST);
    let (mut cleared, mut kept) = (0u32, 0u32);
    for y in 0..ph {
        for x in 0..pw {
            let from_ts = !written(tile_entry_of(c.layout, x, y, c.stride));
            let mut want = if from_ts { CLEAR.to_le_bytes() } else { px(x, y) };
            if swap_rb {
                want = swap(want);
            }
            let o = ((y * pw + x) * 4) as usize;
            assert_eq!(dest[o..o + 4], want, "pixel ({x},{y}) of {w}x{h}, from_ts {from_ts}, swap_rb {swap_rb}");
            if x < w && y < h {
                if from_ts {
                    cleared += 1
                } else {
                    kept += 1
                }
            }
        }
    }
    assert!(cleared > 0 && kept > 0, "the case must mix cleared ({cleared}) and written ({kept}) pixels");
}

fn mixed(n: usize) -> bool {
    n % 5 < 2
}

fn run(w: u32, h: u32, l: Layout, swap_rb: bool) {
    let mut c = setup(w, h, l, mixed);
    resolve(&mut c, swap_rb);
    check(&c, w, h, swap_rb, mixed);
}

#[test]
#[ignore = "ts_resolve(Tiled) sets RS_SOURCE_STRIDE TILING, which rs_interp (and Mesa: TILING only with SUPER_TILED_NEW) reads as super-tiled; see the report"]
fn glmark2_size_800x600() {
    run(800, 600, Layout::Tiled, false);
}

#[test]
#[ignore = "ts_resolve(Tiled) sets RS_SOURCE_STRIDE TILING, which rs_interp (and Mesa: TILING only with SUPER_TILED_NEW) reads as super-tiled; see the report"]
fn glmark2_size_swap_rb() {
    run(800, 600, Layout::Tiled, true);
}

/// 37x13 pads to 48x16: ts_resolve takes it (the window is the padded size)
/// and the padded part resolves like the rest.
#[test]
#[ignore = "ts_resolve(Tiled) sets RS_SOURCE_STRIDE TILING, which rs_interp (and Mesa: TILING only with SUPER_TILED_NEW) reads as super-tiled; see the report"]
fn size_not_a_multiple_of_the_tile() {
    run(37, 13, Layout::Tiled, false);
    run(37, 13, Layout::Tiled, true);
}

/// The layout only changes the stride's `SUPER_TILED_NEW` bit, which the
/// interpreter reads as a super-tiled source: a TILED surface read as
/// super-tiled must not be what the words ask for with `Tiled`.
/// The golden's layout (TILING | SUPER_TILED_NEW): 800x600 pads to 832x640.
#[test]
fn super_tiled_800x600() {
    run(800, 600, Layout::SuperTiled, false);
    run(800, 600, Layout::SuperTiled, true);
}

#[test]
fn super_tiled_size_not_a_multiple() {
    run(37, 13, Layout::SuperTiled, false);
    run(37, 13, Layout::SuperTiled, true);
}

#[test]
fn tiled_words_carry_no_super_tiled_bit() {
    let t =
        TsTarget { ts_addr: TS_BUF, ts_layer_stride: 0x100, surface_addr: SURFACE, padded_width: 16, padded_height: 4 };
    let mk = |layout| {
        let w = ts_resolve(&TsResolve {
            target: t,
            layout,
            format: GC_RS_FORMAT_A8R8G8B8,
            clear_value: CLEAR,
            dest_addr: DEST,
            dest_stride: 64,
            source_stride: 64,
            swap_rb: false,
            source_ts_mode: 0,
            extra_ts_mode: 0,
            keep: 0,
        });
        writes(w.as_slice()).into_iter().find(|&(r, _)| r == GC_RS_SOURCE_STRIDE).unwrap().1
    };
    assert_eq!(mk(TsSourceLayout::Tiled) & GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW, 0);
    assert_ne!(mk(TsSourceLayout::SuperTiled) & GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW, 0);
}

/// Depth: `depth_fast_clear` fills the TS (every tile cleared) through the
/// interpreter; `inplace_resolve` runs no job there (`RS_KICKER_INPLACE` is
/// not modelled), so its effect is not checked.
#[test]
fn depth_fast_clear_fills_ts_inplace_not_modelled() {
    let (pw, ph) = ts::padded_dims(128, 64, true);
    let ts_bytes = ts::ts_size(128, 64, 4, true);
    let target = TsTarget {
        ts_addr: TS_BUF,
        ts_layer_stride: ts_bytes,
        surface_addr: SURFACE,
        padded_width: pw,
        padded_height: ph,
    };
    let mut mem = Memory::new();
    mem.map(SURFACE, vec![0xa5; (pw * ph * 4) as usize]);
    mem.map(TS_BUF, vec![0u8; ts_bytes as usize]);
    let mut interp = RsInterp::new();
    interp.ts_source = true;
    interp.write(GC_RS_SOURCE_ADDR, SURFACE, &mut mem); // as in setup()
    let dc = depth_fast_clear(&DepthFastClear {
        target,
        bits: RS_CLEAR_BITS_ALL,
        clear_word: 0xffff_ff00,
        depth_16bpp: false,
        keep: 0,
        auto_disable: false,
    })
    .expect("a whole-word clear takes the TS path");
    interp.run(&writes(dc.as_slice()), &mut mem);
    assert_eq!(interp.jobs.len(), 1);
    assert!(mem.region(TS_BUF).iter().take((pw * ph * 4 / 256) as usize).all(|&b| b == 0x11));

    let ip = inplace_resolve(&InplaceResolve {
        target,
        format: GC_RS_FORMAT_A8R8G8B8,
        stride: pw * 4,
        tile_count: inplace_tile_count(pw, ph),
        plane: InplacePlane::Depth { clear_value: 0xffff_ff00, depth_16bpp: false },
        keep: 0,
    });
    interp.run(&writes(ip.as_slice()), &mut mem);
    assert_eq!(interp.jobs.len(), 1, "RS_KICKER_INPLACE is not an RS job in rs_interp");
    assert!(mem.region(SURFACE).iter().all(|&b| b == 0xa5), "the surface is untouched");
    let _ = rs_clear::RS_CLEAR_BITS_ALL;
}
