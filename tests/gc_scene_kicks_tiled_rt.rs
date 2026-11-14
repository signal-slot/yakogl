// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): what the tiled render targets and early depth
//! (`gc_tiled_rt`, docs/notes/gc-tiled-wire.md W4 and W5, plan #355) change
//! in a warm frame of glmark2's terrain, shadow and refract, on the harness
//! of `gc_scene_kicks.rs` (`tests/common/gc_scenes.rs`). Each scene runs
//! with the knob off and then on, under both GPU models, and the test pins:
//!
//! * the counts of `gc_scene_kicks.rs` (kicks, waits outside the swap,
//!   `finish()` calls, CPU tiled copies, CPU depth resolves) are the same
//!   with the knob on: the tile-in and the copy back are RS jobs in the
//!   stream being recorded, never a kick, a wait or a CPU copy of their own;
//! * the RS jobs and early-depth draws the knob adds to the submitted
//!   streams ([`StreamTally`]), frame by frame.
//!
//! What the counts say (2026-11-14, main at d49a8045):
//!
//! * terrain: two draws a frame render into the tiled shadow of the
//!   800x600 terrain target and take early depth: the terrain pass (it opens
//!   with a whole colour clear, so no tile-in) and the overlay (blended into
//!   the same target in a later pass: one RS tile-in). Each pass ends with
//!   its copy back before the mipmap chain reads the target: 2 copies back
//!   and 1 tile-in, 3 RS jobs a frame more. Its depth is a 16-bit
//!   renderbuffer, private to the pixel engine (`ds_cache::pe_private`).
//! * shadow and refract: nothing changes. Their offscreen depth is a depth
//!   *texture*, which the next pass samples, so it is not private to the
//!   pixel engine and no draw qualifies (`backend.rs` `tiled_rt_spec`); the
//!   window's draws do not qualify either. W4 and W5 cannot make these two
//!   scenes faster: their gain has to come from elsewhere (depth-sample-gc,
//!   gc-depth-sample.md), and session A on the X2 measures only terrain's.
//!
//! A change that makes another draw qualify, or one stop, fails here and
//! moves the pins in the same commit, with the note's account.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2,test-knobs --test gc_scene_kicks_tiled_rt -- --nocapture
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-vivante",
    feature = "shaderc-online",
    feature = "report-es2",
    feature = "test-knobs"
))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use gc_scenes::*;
use std::cell::RefCell;
use yakogl::vivante::backend::gc_debug_set_tiled_rt;

type Scene = fn() -> Box<dyn FnMut()>;

/// `scene` under `gpu` with the knob set to `on` from `gc_init` on: the
/// counts of frames FIRST_COUNTED..=FRAMES, and the stream tally of each.
/// The knob is off again when it returns, whatever happened.
fn run_knob(gpu: Gpu, on: bool, scene: Scene) -> (Vec<Counts>, Vec<StreamTally>) {
    let tallies = RefCell::new(Vec::new());
    let hooks = Hooks {
        after_init: Some(Box::new(|| gc_debug_set_tiled_rt(i32::from(on)))),
        each_frame: Some(Box::new(|_| tallies.borrow_mut().push(stream_tally()))),
        after_frame: Some(Box::new(|_| {
            let mut t = tallies.borrow_mut();
            let before = t.pop().expect("each_frame's mark");
            t.push(stream_tally().since(&before));
        })),
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_with(gpu, scene, hooks)));
    gc_debug_set_tiled_rt(0);
    let (counts, _) = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    (counts, tallies.into_inner().split_off(FIRST_COUNTED - 1))
}

/// Run `scene` off and on under both models: the counts are the same, and
/// every counted frame's tally with the knob on is the one off plus `added`.
fn check(name: &str, scene: Scene, added: StreamTally) {
    for gpu in [Gpu::Lag, Gpu::Done] {
        let (counts_off, tally_off) = run_knob(gpu, false, scene);
        let (counts_on, tally_on) = run_knob(gpu, true, scene);
        eprintln!("{name} {gpu:?}: off {:?} {:?}", counts_off.last(), tally_off.last());
        eprintln!("{name} {gpu:?}: on  {:?} {:?}", counts_on.last(), tally_on.last());
        assert_eq!(counts_on, counts_off, "{name} ({gpu:?}): the knob changed the frame's counts");
        assert_eq!(tally_off.len(), FRAMES + 1 - FIRST_COUNTED);
        for (n, (off, on)) in tally_off.iter().zip(&tally_on).enumerate() {
            let frame = FIRST_COUNTED + n;
            assert_eq!(off.early_z_draws, 0, "{name} ({gpu:?}) frame {frame}: early depth with the knob off: {off:?}");
            assert_eq!(on.draws, off.draws, "{name} ({gpu:?}) frame {frame}: the knob changed the draws");
            assert_eq!(on.since(off), added, "{name} ({gpu:?}) frame {frame}: off {off:?}, on {on:?}");
        }
    }
}

/// terrain: the terrain pass and the overlay render into the shadow with
/// early depth; 2 copies back, 1 tile-in (the overlay's).
#[test]
fn terrain_with_the_knob_takes_two_shadow_draws_and_no_kick_more() {
    let added = StreamTally { rs_kicks: 3, rs_from_tiled: 2, rs_to_tiled: 1, draws: 0, early_z_draws: 2 };
    check("terrain", terrain, added);
}

/// shadow: the depth pass writes a depth texture, which no shadow takes.
#[test]
fn shadow_with_the_knob_is_unchanged() {
    check("shadow", shadow, StreamTally::default());
}

/// refract: the distance pass's depth is a texture too.
#[test]
fn refract_with_the_knob_is_unchanged() {
    check("refract", refract, StreamTally::default());
}
