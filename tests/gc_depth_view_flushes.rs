// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the flushes and stalls the stream already holds in
//! front of a draw that samples a depth view (`gc_debug_set_depth_tpu`, mode
//! 1, src/vivante/depth_sample.rs), pinned. This is the evidence for verdict
//! code 2 of docs/notes/gc-depth-sample.md ("stale texture cache", check 7)
//! and for docs/notes/gc-depth-sample-wire.md (a).9-(a).10: if the board
//! serves old depth, these are the words that already stood between the depth
//! pass and the sample, so the fix is a new word and not a missing old one.
//!
//! glmark2's shadow and refract frames run through the real GL core and the
//! Vivante back end over the stand-in OSAL of `common/gc_scenes.rs`, wrapped
//! here so that each submitted stream is decoded at the moment of its submit
//! into a list of register writes, front-end stalls and draws. A descriptor's
//! CONFIG0 is read from the stand-in's memory in the same call, at the
//! `NTE_DESCRIPTOR_ADDR(0)` write that names it: the frame arena is not
//! reused before the kick is submitted, and nothing runs concurrently, so the
//! value read is the one the draw sampled. A *sampling draw* is a draw
//! whose own window (the words since the previous draw, or since the kick's
//! start) wrote slot 0's descriptor address, and whose descriptor's format is
//! D24X8 (a depth view, `GC_TEX_FORMAT_D24X8`).
//!
//! Pinned for every sampling draw of the settled frame (frame 6):
//!
//! * `emit_flushes`, in this order inside the draw's window: FLUSH
//!   COLOR|DEPTH|TEXTURE|SHADER_L1, FLUSH TEXTUREVS, the RA to PE semaphore
//!   and stall, the TS flush;
//! * after them, the front end held until the pixel engine drained (FE to PE
//!   stall), then slot 0's descriptor address, then slot 0's descriptor-cache
//!   INVALIDATE, then the draw;
//! * at the kick's start, before its first draw: the descriptor-cache flush
//!   (`NTE_DESCRIPTOR_FLUSH`, then FLUSH DESCRIPTOR);
//! * the sampler words carry no compare bits (SAMP_CTRL0 bit 17, 18-20).
//!
//! The same test pins that with the knob off no draw samples a D24X8
//! descriptor (so the pin is not vacuous), and that the shadow frame has
//! exactly one sampling draw.
//!
//! The runs are in one `#[test]`, in a fixed order (see its comment): the
//! back end carries state from one run of a process to the next, so an order
//! that follows thread timing is not deterministic. The capture is taken
//! inside the run, under `SERIAL`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_view_flushes
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;
#[path = "common/gc_stream.rs"]
mod gc_stream;

use gc_scenes::*;
use gc_stream::{frames_with, Ev};
use yakogl::vivante::depth_sample::{DEPTH_TPU_D24X8, DEPTH_TPU_OFF, GC_TEX_FORMAT_D24X8};
use yakogl::vivante::regs::*;

/// Run `scene` with the depth-view knob in `mode` and return the decoded
/// kicks of the last frame (the harness is tests/common/gc_stream.rs).
fn last_frame(mode: i32, scene: fn() -> Box<dyn FnMut()>) -> Vec<Vec<Ev>> {
    frames_with(mode, scene, |body| body()).pop().map(|f| f.0).unwrap_or_default()
}

/// Index of the first event at or after `from` satisfying `f`.
fn find(evs: &[Ev], from: usize, f: impl Fn(&Ev) -> bool) -> Option<usize> {
    evs.get(from..)?.iter().position(f).map(|p| p + from)
}

const DESC_FORMAT_MASK: u32 = 0x1f << 13;

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

fn is_view(cfg: Option<u32>) -> bool {
    cfg.is_some_and(|c| c & DESC_FORMAT_MASK == texdesc_format(GC_TEX_FORMAT_D24X8))
}

/// Every sampling draw of `kicks`, checked; the count of them.
fn check_sampling_draws(name: &str, kicks: &[Vec<Ev>]) -> usize {
    let st = |reg: u32, val: u32| move |e: &Ev| *e == Ev::State(reg, val);
    let mut n = 0;
    for (k, evs) in kicks.iter().enumerate() {
        for (start, draw, cfg) in windows(evs) {
            if !is_view(cfg) {
                continue;
            }
            n += 1;
            let at = format!("{name} kick {k} draw at {draw}");
            let win = &evs[..draw];
            let flush = GC_FLUSH_COLOR | GC_FLUSH_DEPTH | GC_FLUSH_TEXTURE | GC_FLUSH_SHADER_L1;
            let i1 =
                find(win, start, st(GC_GL_FLUSH_CACHE, flush)).unwrap_or_else(|| panic!("{at}: no FLUSH C|D|T|L1"));
            let i2 = find(win, i1 + 1, st(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS))
                .unwrap_or_else(|| panic!("{at}: no TEXTUREVS"));
            let ra_pe = stall_token(GC_SYNC_RA, GC_SYNC_PE);
            let i3 = find(win, i2 + 1, st(GC_GL_SEMAPHORE_TOKEN, ra_pe))
                .unwrap_or_else(|| panic!("{at}: no RA-PE semaphore"));
            assert_eq!(
                win.get(i3 + 1),
                Some(&Ev::State(GC_GL_STALL_TOKEN, ra_pe)),
                "{at}: RA-PE stall not behind its semaphore"
            );
            let i4 =
                find(win, i3 + 2, st(GC_TS_FLUSH_CACHE, GC_TS_FLUSH)).unwrap_or_else(|| panic!("{at}: no TS flush"));
            let i5 = find(win, i4 + 1, |e| *e == Ev::FeStall(stall_token(GC_SYNC_FE, GC_SYNC_PE)))
                .unwrap_or_else(|| panic!("{at}: no FE-PE stall behind the flushes"));
            let i6 = find(win, i5 + 1, |e| matches!(e, Ev::Slot0(_)))
                .unwrap_or_else(|| panic!("{at}: no slot 0 write behind the stall"));
            let inval = GC_NTE_INVALIDATE_UNK29 | nte_invalidate_idx(0);
            find(win, i6 + 1, st(GC_NTE_DESCRIPTOR_INVALIDATE, inval))
                .unwrap_or_else(|| panic!("{at}: no slot 0 INVALIDATE before the draw"));
            /* the kick's start: the descriptor cache holds nothing from before */
            let first = evs.iter().position(|e| *e == Ev::Draw).unwrap();
            let d0 = find(&evs[..first], 0, st(GC_NTE_DESCRIPTOR_FLUSH, 0))
                .unwrap_or_else(|| panic!("{at}: no descriptor flush at the kick's start"));
            find(&evs[..first], d0 + 1, st(GC_GL_FLUSH_CACHE, GC_FLUSH_DESCRIPTOR))
                .unwrap_or_else(|| panic!("{at}: no FLUSH DESCRIPTOR at the kick's start"));
            /* the sampler state: no compare bits */
            let s0 = find(win, start, |e| matches!(e, Ev::State(r, _) if *r == nte_descriptor_samp_ctrl0(0)));
            if let Some(Ev::State(_, v)) = s0.map(|i| win[i]) {
                assert_eq!(v & (0x1 << 17 | 0x7 << 18), 0, "{at}: SAMP_CTRL0 {v:#x} has compare bits");
            }
        }
    }
    n
}

/// One test, in a fixed order: the back end keeps state across runs of one
/// process that `gc_shutdown` does not clear, and refract's frame 6 under
/// mode 1 samples the depth view only on a run that no earlier run in the
/// process preceded (after a refract run, with the knob off or on, frame 6
/// takes the plain texture again; its frames 1 to 5 still sample the view).
/// With two `#[test]`s libtest's threads took `SERIAL` in either order, and
/// the result followed. The mode 1 runs come first; the knob-off runs, which
/// assert an absence, follow.
#[test]
fn depth_view_draw_is_preceded_by_the_flushes_and_stalls() {
    let kicks = last_frame(DEPTH_TPU_D24X8, shadow);
    assert!(!kicks.is_empty(), "no kick in the frame");
    assert_eq!(check_sampling_draws("shadow", &kicks), 1, "shadow: one draw samples the depth view");
    let kicks = last_frame(DEPTH_TPU_D24X8, refract);
    assert!(check_sampling_draws("refract", &kicks) >= 1, "refract: a draw samples the depth view");
    /* the knob off: no draw samples a D24X8 descriptor, so the checks above
     * are not vacuous */
    for (name, scene) in [("shadow", shadow as fn() -> Box<dyn FnMut()>), ("refract", refract)] {
        let kicks = last_frame(DEPTH_TPU_OFF, scene);
        assert!(!kicks.is_empty(), "{name}: no kick");
        assert_eq!(check_sampling_draws(name, &kicks), 0, "{name}: a D24X8 descriptor with the knob off");
    }
}
