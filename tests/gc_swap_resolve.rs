// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! W13's swap resolve (`src/vivante/rs_clear.rs` `swap_resolve`,
//! docs/notes/gc-tiled-window.md) run on the host: its words go through
//! `rs_interp` over a synthetic 4x4 TILED window shadow, and the linear
//! window buffer it writes must equal an untile through `layout::offset`
//! byte for byte: every pixel of the RS window (the GL size widened to
//! 16 x 4), R and B exchanged with `swap_rb`, nothing else of the
//! destination touched.
//!
//! The words themselves are pinned against a hand derivation from Mesa
//! 26.2.1 in rs_clear.rs's own tests (the `--lib` row). This file is the
//! semantic check: the interpreter models Mesa's RS, and the shadow's
//! stride is `layout::stride`'s for a TILED, RS-aligned target. Nothing in
//! the back end calls `swap_resolve` yet, so no stream moves.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante --test gc_swap_resolve`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::regs::*;
use yakogl::vivante::rs_clear::{
    state_writes, swap_resolve, swap_resolve_window, window_shadow_stride, NotSwapResolve, WindowResolve, WINDOW_BPP,
};
use yakogl::vivante::rs_interp::{Memory, RsInterp};

const SRC: u32 = 0x1000_0000;
const DST: u32 = 0x2000_0000;
/// What the destination holds before the resolve: a pixel the RS window
/// does not cover must still read this.
const UNTOUCHED: u8 = 0xee;

/// The destination after an untile of `src` (a TILED shadow `stride` bytes
/// a row) over the RS window `ww` x `wh`, through `layout::offset`.
fn untiled(src: &[u8], stride: u32, ww: u32, wh: u32, pitch: usize, rows: usize, swap_rb: bool) -> Vec<u8> {
    let mut out = vec![UNTOUCHED; pitch * rows];
    for y in 0..wh {
        for x in 0..ww {
            let s = layout::offset(Layout::Tiled, x, y, WINDOW_BPP, stride);
            let mut px = [src[s], src[s + 1], src[s + 2], src[s + 3]];
            if swap_rb {
                px.swap(0, 2);
            }
            let d = y as usize * pitch + x as usize * 4;
            out[d..d + 4].copy_from_slice(&px);
        }
    }
    out
}

#[test]
fn the_swap_resolve_untiles_byte_exact_through_rs_interp() {
    // (width, height, format, swap_rb, dst_pitch, dst_rows): the note's
    // 800x600 and 1920x1080 windows, 1366x768 widened to 1376 in a padded
    // destination, and 50x30 widened in both directions with padding the
    // RS window does not reach (pitch 288 > 64 * 4, 33 rows > 32).
    let cases = [
        (800, 600, GC_RS_FORMAT_A8R8G8B8, false, 0, 0),
        (1920, 1080, GC_RS_FORMAT_X8R8G8B8, true, 0, 0),
        (1366, 768, GC_RS_FORMAT_A8R8G8B8, false, 1376 * 4, 0),
        (50, 30, GC_RS_FORMAT_A8R8G8B8, true, 64 * 4 + 32, 33),
    ];
    for (width, height, format, swap_rb, dst_pitch, dst_rows) in cases {
        let r = WindowResolve { src_addr: SRC, dst_addr: DST, width, height, format, swap_rb, dst_pitch, dst_rows };
        let (ww, wh) = swap_resolve_window(&r).unwrap();
        let stride = window_shadow_stride(width);
        assert_eq!(stride, layout::stride(Layout::Tiled, width, WINDOW_BPP, true), "{width}x{height}: stride");
        let pitch = if dst_pitch != 0 { dst_pitch } else { width * 4 } as usize;
        let rows = if dst_rows != 0 { dst_rows } else { height } as usize;
        let src: Vec<u8> = (0..stride as usize * wh as usize).map(|i| (i * 7 + i / 251 + 3) as u8).collect();
        let expect = untiled(&src, stride, ww, wh, pitch, rows, swap_rb);

        let mut mem = Memory::new();
        mem.map(SRC, src);
        mem.map(DST, vec![UNTOUCHED; pitch * rows]);
        let words = swap_resolve(&r).unwrap();
        let writes: Vec<(u32, u32)> = state_writes(words.as_slice()).collect();
        let mut rs = RsInterp::new();
        rs.run(&writes, &mut mem);
        assert_eq!(rs.jobs.len(), 1, "{width}x{height}: one RS job");
        let job = rs.jobs[0];
        assert_eq!((job.w, job.h, job.swap_rb, job.fill), (ww, wh, swap_rb, None), "{width}x{height}: the job");
        assert!(mem.region(DST) == &expect[..], "{width}x{height}: the resolve is not the untile");
    }
}

#[test]
fn a_refused_window_writes_no_words() {
    // The caller keeps the linear path: no partial stream to run.
    let r = WindowResolve {
        src_addr: SRC,
        dst_addr: DST,
        width: 1366,
        height: 768,
        format: GC_RS_FORMAT_A8R8G8B8,
        swap_rb: false,
        dst_pitch: 0,
        dst_rows: 0,
    };
    assert_eq!(swap_resolve(&r).err(), Some(NotSwapResolve::Unaligned));
}
