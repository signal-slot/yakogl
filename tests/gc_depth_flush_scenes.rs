// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): dtpu knob mode 4 (`DEPTH_TPU_D24X8_FLUSH`,
//! src/vivante/depth_sample.rs) puts its texture-cache flush block only in
//! front of draws that sample a depth view, in glmark2's three scenes
//! (docs/notes/gc-depth-sample-wire.md (a).10). The mode is set explicitly.
//! Each scene runs its frames through the real GL core and the Vivante back
//! end over the stand-in OSAL of `common/gc_scenes.rs`; every submitted
//! stream is decoded into register writes, front-end stalls and draws (the
//! harness of tests/common/gc_stream.rs). A *flush block* is
//! the seven events `emit_view_flush` writes (FLUSH_CACHE DEPTH; SEMAPHORE
//! FE-PE; STALL FE-PE; FLUSH_CACHE TEXTURE; FLUSH_CACHE TEXTUREVS; SEMAPHORE
//! RA-PE; STALL_TOKEN RA-PE).
//!
//! Pinned, for each of the six frames of each scene in mode 4:
//!
//! * shadow and refract: as many flush blocks as draws whose slot 0
//!   descriptor is D24X8 (a depth view), at least one; each block lies in
//!   the window of such a draw (after the previous draw, before this one);
//!   every such draw's window holds exactly one block (one depth write a
//!   frame, so one block per sampling draw) and no other window holds one;
//! * terrain: no flush block and no depth-view draw;
//! * all three: the CPU depth resolve (`depth_resolve` of the table) runs
//!   0 times, so no resolve sits behind those draws.
//!
//! The runs are in one `#[test]` in a fixed order: the back end keeps state
//! across runs of one process.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_flush_scenes
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;
#[path = "common/gc_stream.rs"]
mod gc_stream;

use gc_scenes::*;
use gc_stream::{frames_with, Ev};
use yakogl::vivante::depth_sample::{DEPTH_TPU_D24X8_FLUSH, DEPTH_TPU_OFF, GC_TEX_FORMAT_D24X8};
use yakogl::vivante::regs::*;

/// Run `scene` with the depth-view knob in `mode` and return the decoded
/// kicks of every frame with the CPU depth resolves the frame ran (the
/// harness is tests/common/gc_stream.rs).
fn frames(mode: i32, scene: fn() -> Box<dyn FnMut()>) -> Vec<(Vec<Vec<Ev>>, u32)> {
    frames_with(mode, scene, |body| {
        let before = snapshot().resolves;
        body();
        snapshot().resolves - before
    })
}

/// The windows of one kick: (start of the window, index of its draw, slot-0
/// CONFIG0 written in it).
fn windows(evs: &[Ev]) -> Vec<(usize, usize, Option<u32>)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut cfg = None;
    for (i, e) in evs.iter().enumerate() {
        match e {
            Ev::Slot0(c) => cfg = *c,
            Ev::Draw => {
                out.push((start, i, cfg.take()));
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

const DESC_FORMAT_MASK: u32 = 0x1f << 13;

fn is_view(cfg: Option<u32>) -> bool {
    cfg.is_some_and(|c| c & DESC_FORMAT_MASK == texdesc_format(GC_TEX_FORMAT_D24X8))
}

/// The seven events of `emit_view_flush`.
fn block() -> [Ev; 7] {
    let fe_pe = stall_token(GC_SYNC_FE, GC_SYNC_PE);
    let ra_pe = stall_token(GC_SYNC_RA, GC_SYNC_PE);
    [
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_DEPTH),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, fe_pe),
        Ev::FeStall(fe_pe),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTURE),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, ra_pe),
        Ev::State(GC_GL_STALL_TOKEN, ra_pe),
    ]
}

/// Where the flush blocks of one kick start.
fn blocks(evs: &[Ev]) -> Vec<usize> {
    let b = block();
    (0..evs.len()).filter(|&i| evs[i..].starts_with(&b)).collect()
}

/// One frame's flush blocks as (kick, block start), and its sampling draws
/// as (kick, window start, draw index).
type Found = (Vec<(usize, usize)>, Vec<(usize, usize, usize)>);

fn found(kicks: &[Vec<Ev>]) -> Found {
    let mut bl = Vec::new();
    let mut sd = Vec::new();
    for (k, evs) in kicks.iter().enumerate() {
        bl.extend(blocks(evs).into_iter().map(|s| (k, s)));
        sd.extend(windows(evs).into_iter().filter(|w| is_view(w.2)).map(|(s, d, _)| (k, s, d)));
    }
    (bl, sd)
}

/// One frame's flush blocks as (kick, block start), and its sampling draws
/// Frames of one scene in mode 4: the flush blocks and the depth-view draws
/// agree, window by window; `views` says whether the scene samples a depth
/// view at all.
fn check(name: &str, all: &[(Vec<Vec<Ev>>, u32)], views: bool) {
    assert!(all.len() >= 2, "{name}: need two frames");
    for (f, (kicks, resolves)) in all.iter().enumerate() {
        let at = format!("{name} frame {}", f + 1);
        assert_eq!(*resolves, 0, "{at}: {resolves} CPU depth resolves, want none");
        let mut sampling = 0usize;
        let mut flushed = 0usize;
        for (k, evs) in kicks.iter().enumerate() {
            let at_blocks = blocks(evs);
            for (start, draw, cfg) in windows(evs) {
                let here: Vec<usize> = at_blocks.iter().copied().filter(|&b| start <= b && b < draw).collect();
                if is_view(cfg) {
                    sampling += 1;
                    assert_eq!(here.len(), 1, "{at} kick {k}: the depth-view draw at {draw} has {} blocks", here.len());
                    flushed += 1;
                } else {
                    assert!(here.is_empty(), "{at} kick {k}: a flush block at {here:?} before the draw at {draw}, which samples no depth view");
                }
            }
            /* every block is in some draw's window */
            let inside = at_blocks.iter().filter(|&&b| windows(evs).iter().any(|w| w.0 <= b && b < w.1)).count();
            assert_eq!(inside, at_blocks.len(), "{at} kick {k}: a flush block outside every draw window");
        }
        let (bl, sd) = found(kicks);
        assert_eq!(bl.len(), sd.len(), "{at}: {} blocks, {} depth-view draws", bl.len(), sd.len());
        assert_eq!(flushed, sampling);
        if views {
            assert!(sampling >= 1, "{at}: no draw samples the depth view");
        } else {
            assert!(bl.is_empty() && sd.is_empty(), "{at}: blocks {bl:?}, depth-view draws {sd:?}");
        }
    }
}

#[test]
fn mode4_flushes_only_in_front_of_depth_view_draws() {
    for (name, scene, views) in
        [("shadow", shadow as fn() -> Box<dyn FnMut()>, true), ("refract", refract, true), ("terrain", terrain, false)]
    {
        let all = frames(DEPTH_TPU_D24X8_FLUSH, scene);
        assert_eq!(all.len(), FRAMES);
        check(name, &all, views);
    }
    /* the knob is back off */
    let off = frames(DEPTH_TPU_OFF, shadow);
    assert!(off.iter().all(|(k, _)| found(k).0.is_empty()), "mode 0 wrote a flush block");
}
