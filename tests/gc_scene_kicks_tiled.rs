// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): what one warm frame of glmark2's terrain, shadow and
//! refract scenes costs in the Vivante back end with the tiled render targets
//! and early depth on (`gc_tiled_rt`, `gc_debug_set_tiled_rt`,
//! docs/notes/gc-tiled-wire.md W4 and W5), beside the same frame with the
//! knob off. The scenes, the stand-in OSAL and the counts are
//! `gc_scene_kicks.rs`'s (the shared harness, `common/gc_scenes.rs`; read
//! that file's header for what each count is); this file adds what the
//! stand-in reads out of each kick's command stream:
//!
//! * RS tile-in: an RS kick from a linear source into a TILED destination
//!   (a tiled shadow's tile-in, and the RS copy of a vertex-stage map,
//!   `tiled_copy.rs`: both are this shape);
//! * RS copy back: an RS kick from a TILED source into a linear destination
//!   (a tiled shadow's resolve at the end of its span);
//! * RS other: fills (`rs_clear.rs`) and linear copies;
//! * draws that took a tiled shadow: draw commands issued with
//!   SUPER_TILED in PE_DEPTH_CONFIG (only a shadow draw sets it);
//! * draws with early depth: draw commands issued with EARLY_Z in
//!   PE_DEPTH_CONFIG.
//!
//! A frame, the most over frames 3..=6, at the commit that wrote this file
//! (`Lag`; `Done` differs only in the waits, which the tests pin too):
//!
//! ```text
//!                          terrain         shadow          refract
//!                          off     on      off     on      off     on
//! kicks                    11      11      2       2       3       3
//! waits outside the swap   11      11      1       1       1       1      (Lag; 0 in Done)
//! finish() outside swap    0       0       0       0       0       0
//! CPU tiled copies         0       0       0       0       0       0      (Done: terrain 0)
//! CPU depth resolves       0       0       1       1       1       1
//! RS tile-in               2       3       0       0       0       0
//! RS copy back             0       2       0       0       0       0
//! RS other                 11      11      3       3       4       4
//! draws                    51      51      3       3       12      12
//! draws, tiled shadow      0       2       0       0       0       0
//! draws, early depth       0       2       0       0       0       0
//! ```
//!
//! What it shows: with the knob on only terrain takes a tiled shadow, two
//! draws (which two is the census's to say, tiled-qualify-census: depth-tested
//! draws into a colour target with a renderbuffer depth); shadow's and refract's depth
//! targets are textures, which `tiled_rt::spec_of` refuses, so their streams
//! are the knob-off ones word for word in count. The kicks do not change: a
//! shadow is tiled in and copied back inside the stream that draws into it.
//! Terrain's extra RS kicks are one tile-in and two copy backs.
//!
//! The knob-off column is `gc_scene_kicks.rs`'s run on the feature set below
//! (no `report-es2`: the harness's depth textures are sized there, see
//! `depth_internalformat`). Its counts are at or under that file's ceilings,
//! which are copied here ([`OFF_TERRAIN_LAG`] ...) and asserted; the
//! ceilings stand as they are (terrain 14, shadow 3, refract 4 kicks: the
//! frame merging of gc-frames-ring has since lowered the counts below them),
//! so each test also pins today's knob-off counts exactly.
//!
//! The knob is set before the first context of each run and put back to off
//! after it; [`KNOB`] keeps another test of this file out meanwhile.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_scene_kicks_tiled -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use gc_scenes::*;
use std::sync::Mutex;
use yakogl::vivante::backend::gc_debug_set_tiled_rt;

/* ---- gc_scene_kicks.rs's ceilings, copied: the knob off stays under them ---------------- */

const OFF_TERRAIN_LAG: Counts =
    Counts { kicks: 14, waits_other: 14, finishes_other: 0, tiled_copies: 0, resolves: 0, waits_swap: 0 };
const OFF_TERRAIN_DONE: Counts =
    Counts { kicks: 14, waits_other: 0, finishes_other: 0, tiled_copies: 1, resolves: 0, waits_swap: 0 };
const OFF_SHADOW_LAG: Counts =
    Counts { kicks: 3, waits_other: 1, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const OFF_SHADOW_DONE: Counts =
    Counts { kicks: 3, waits_other: 0, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const OFF_REFRACT_LAG: Counts =
    Counts { kicks: 4, waits_other: 2, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const OFF_REFRACT_DONE: Counts =
    Counts { kicks: 4, waits_other: 0, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };

/* ---- the counts, off and on ----------------------------------------------------------- */

const fn c(kicks: u32, waits_other: u32, tiled_copies: u32, resolves: u32) -> Counts {
    Counts { kicks, waits_other, finishes_other: 0, tiled_copies, resolves, waits_swap: 0 }
}

const fn s(
    rs_tile_in: u32,
    rs_copy_back: u32,
    rs_other: u32,
    draws: u32,
    draws_tiled_shadow: u32,
    draws_early_z: u32,
) -> StreamCounts {
    StreamCounts { rs_tile_in, rs_copy_back, rs_other, draws, draws_tiled_shadow, draws_early_z }
}

/// One scene's frame, a GPU model and the knob: the most over the counted
/// frames, and a check that every counted frame is the same.
fn frame(on: bool, gpu: Gpu, scene: fn() -> Box<dyn FnMut()>) -> (Counts, StreamCounts) {
    let _k = KNOB.lock().unwrap_or_else(|e| e.into_inner());
    gc_debug_set_tiled_rt(i32::from(on));
    let r = std::panic::catch_unwind(|| run_full(gpu, scene, Hooks::default()));
    gc_debug_set_tiled_rt(0);
    let (frames, _, streams) = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(frames.windows(2).all(|w| w[0] == w[1]), "the counted frames differ: {frames:?}");
    assert!(streams.windows(2).all(|w| w[0] == w[1]), "the counted frames' streams differ: {streams:?}");
    (Counts::max_of(&frames), StreamCounts::max_of(&streams))
}

/// Serialises the knob across this file's tests (they are threads of one
/// process, and the knob and the back end are process-global).
static KNOB: Mutex<()> = Mutex::new(());

/// A scene with the knob off and on, in both GPU models: off, under
/// `gc_scene_kicks.rs`'s ceilings and exactly `off`; on, exactly `on`.
/// `(lag, done)` for the GC counts of each model and the stream counts the
/// models share (the GPU model changes waits and the CPU tiled copy, not
/// what a stream holds).
struct Scene {
    name: &'static str,
    run: fn() -> Box<dyn FnMut()>,
    ceilings: (Counts, Counts),
    off: (Counts, Counts, StreamCounts),
    on: (Counts, Counts, StreamCounts),
}

fn check(sc: &Scene) {
    for (knob, gpu_counts, ceil) in [(false, &sc.off, Some(&sc.ceilings)), (true, &sc.on, None)] {
        for (gpu, want, ceiling) in
            [(Gpu::Lag, gpu_counts.0, ceil.map(|c| c.0)), (Gpu::Done, gpu_counts.1, ceil.map(|c| c.1))]
        {
            let (got, stream) = frame(knob, gpu, sc.run);
            eprintln!("{} knob {} {gpu:?}: {got:?} {stream:?}", sc.name, if knob { "on" } else { "off" });
            assert_eq!(got, want, "{} (knob {knob}, {gpu:?}): the GC counts", sc.name);
            assert_eq!(stream, gpu_counts.2, "{} (knob {knob}, {gpu:?}): the stream counts", sc.name);
            if let Some(ceiling) = ceiling {
                let over = got.within(&ceiling);
                assert!(
                    over.is_empty(),
                    "{} ({gpu:?}): {over:?} above gc_scene_kicks.rs's ceiling {ceiling:?}",
                    sc.name
                );
            }
        }
    }
}

/* ---- the tests ------------------------------------------------------------------------ */

/// terrain. Knob off, `gc_scene_kicks.rs`'s frame: 11 kicks (the canvas clear
/// with the height, normal and terrain passes in one stream, then the chains
/// and passes after them, gc-frames-ring: under the ceiling of 14); the RS
/// kicks are 2 tiled copies of the vertex-stage maps (tile-in shape) and 11
/// fills; no early depth.
/// Knob on, the same 11 kicks and the same waits (a shadow is tiled in and
/// copied back inside the stream that draws into it): 2 depth-tested draws
/// into a target with a depth renderbuffer take a shadow (both with early
/// depth, `early_z.rs`: their fragment shaders neither discard nor write
/// depth). The RS kicks gain 1 tile-in and 2 copy backs (one at the end of
/// each shadow's span; a span that opens with a whole colour clear tiles
/// nothing in, `backend_tests_tiled_rt.rs`). The fills and the 2
/// copies of the maps are the ones of knob off. No CPU tiled copy and no CPU
/// depth resolve, as off: terrain's depth is a renderbuffer, never sampled.
#[test]
fn terrain_with_gc_tiled_rt_on_beside_off() {
    check(&Scene {
        name: "terrain",
        run: terrain,
        ceilings: (OFF_TERRAIN_LAG, OFF_TERRAIN_DONE),
        off: (c(11, 11, 0, 0), c(11, 0, 0, 0), s(2, 0, 11, 51, 0, 0)),
        on: (c(11, 11, 0, 0), c(11, 0, 0, 0), s(3, 2, 11, 51, 2, 2)),
    });
}

/// shadow. Knob off, `gc_scene_kicks.rs`'s frame: 2 kicks (the canvas clear
/// and the 1600x1200 depth pass in one stream, then the window's ground and
/// lit horse at the swap; the ceiling is 3), 1 wait (the depth resolve's
/// `wait_kick`, `Lag`), 1 CPU depth resolve (depth-sample-gc takes it to 0);
/// 3 draws, 3 RS fills.
/// Knob on: every count the same and no draw takes a shadow or early depth.
/// The depth pass renders into a depth texture, and the window's draws
/// into the window: `tiled_rt::spec_of` refuses a target whose depth is a
/// texture (and the window), so W4 and W5 do not reach this scene today. That
/// is the scene's finding for the board: the knob moves nothing in shadow's
/// frame, so a session A gain in shadow would not come from it.
#[test]
fn shadow_with_gc_tiled_rt_on_beside_off() {
    check(&Scene {
        name: "shadow",
        run: shadow,
        ceilings: (OFF_SHADOW_LAG, OFF_SHADOW_DONE),
        off: (c(2, 1, 0, 1), c(2, 0, 0, 1), s(0, 0, 3, 3, 0, 0)),
        on: (c(2, 1, 0, 1), c(2, 0, 0, 1), s(0, 0, 3, 3, 0, 0)),
    });
}

/// refract. Knob off, `gc_scene_kicks.rs`'s frame: 3 kicks (the canvas clear
/// with the 1600x1200 pass, the 11-level chain in a stream of its own, the
/// window's bunny at the swap; the ceiling is 4), 1 wait outside the swap
/// (`Lag`: the depth resolve's `wait_kick`), 1 CPU depth resolve; 12 draws, 4
/// RS fills.
/// Knob on: every count the same and no draw takes a shadow or early depth,
/// for shadow's reason: the 1600x1200 pass has a depth texture, the bunny
/// draws into the window.
#[test]
fn refract_with_gc_tiled_rt_on_beside_off() {
    check(&Scene {
        name: "refract",
        run: refract,
        ceilings: (OFF_REFRACT_LAG, OFF_REFRACT_DONE),
        off: (c(3, 1, 0, 1), c(3, 0, 0, 1), s(0, 0, 4, 12, 0, 0)),
        on: (c(3, 1, 0, 1), c(3, 0, 0, 1), s(0, 0, 4, 12, 0, 0)),
    });
}
