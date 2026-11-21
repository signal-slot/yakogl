// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): what the depth-view path (`gc_debug_set_depth_tpu`,
//! src/vivante/depth_sample.rs, docs/notes/gc-depth-sample-wire.md outcome
//! (a)) saves in a warm frame of glmark2's shadow and refract, on the harness
//! of `gc_scene_kicks.rs` (`tests/common/gc_scenes.rs`). Each scene runs with
//! the knob off and then with mode 1 (D24X8 view), under both GPU models, and
//! the test pins the counts of a warm frame: kicks, waits outside the swap,
//! `finish()` calls, CPU tiled copies and CPU depth resolves (and the waits
//! in the swap, which the Lag model moves).
//!
//! What the counts say (2026-11-21, tip 104748bb):
//!
//! * shadow: knob off, 2 kicks, 1 CPU depth resolve (and under Lag 1 wait
//!   outside the swap); mode 1: 1 kick, 0 resolves, 0 waits outside the swap
//!   (under Lag the wait moves into the swap). The resolve and the kick it
//!   forced are gone.
//!   Mode 1's frames 3 to 5 still take 2 kicks; frame 6's count is the
//!   settled one and is what is pinned (no frame resolves).
//! * refract: off, 3 kicks, 1 resolve (Lag: 1 wait outside the swap); mode 1:
//!   3 kicks, 0 resolves (Lag: 2 waits outside the swap, 1 in it).
//! * both: 0 `finish()` calls and 0 CPU tiled copies either way. The knob
//!   path refuses neither scene's depth texture: `view_binding` never fails,
//!   no draw returns -ENOTSUP and `no_error` holds in every frame.
//! * terrain samples no depth (its depth is a renderbuffer): mode 1 changes
//!   nothing and it resolves nothing either way.
//!
//! The headline: CPU depth resolves 1 -> 0 a frame in shadow and refract.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2,test-knobs --test gc_scene_kicks_depth_tpu -- --nocapture
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
use yakogl::vivante::depth_sample::gc_debug_set_depth_tpu;

type Scene = fn() -> Box<dyn FnMut()>;

/// `scene` under `gpu` with the depth-view mode set from `gc_init` on. The
/// knob is written only inside the hooks, where the harness's lock is held
/// (libtest runs the tests on parallel threads), and put back to 0 after the
/// last frame; every run sets it again at its start, so a panicked run leaves
/// nothing to the next.
fn run_mode(gpu: Gpu, mode: i32, scene: Scene) -> Vec<Counts> {
    let hooks = Hooks {
        after_init: Some(Box::new(move || {
            gc_debug_set_depth_tpu(mode);
        })),
        after_frame: Some(Box::new(|n| {
            if n == FRAMES {
                gc_debug_set_depth_tpu(0);
            }
        })),
        ..Hooks::default()
    };
    run_with(gpu, scene, hooks).0
}

/// A warm frame's counts: kicks, waits outside the swap, CPU tiled copies,
/// CPU depth resolves, waits in the swap (`finish()` calls are 0 throughout).
fn c(kicks: u32, waits_other: u32, tiled_copies: u32, resolves: u32, waits_swap: u32) -> Counts {
    Counts { kicks, waits_other, finishes_other: 0, tiled_copies, resolves, waits_swap }
}

/// `scene` under `gpu` and `mode`: every mode 0 frame has the counts `want`;
/// with mode 1 no frame resolves and the settled (last) frame has `want`
/// (shadow's first counted frames with the view still take 2 kicks).
fn check(name: &str, scene: Scene, gpu: Gpu, mode: i32, want: Counts) {
    let frames = run_mode(gpu, mode, scene);
    eprintln!("{name} {gpu:?} mode {mode}: {frames:?}");
    assert_eq!(frames.len(), FRAMES + 1 - FIRST_COUNTED);
    if mode == 0 {
        for (n, got) in frames.iter().enumerate() {
            assert_eq!(*got, want, "{name} ({gpu:?}) mode 0 frame {}", FIRST_COUNTED + n);
        }
    } else {
        assert!(frames.iter().all(|f| f.resolves == 0), "{name} ({gpu:?}): a CPU resolve with mode 1: {frames:?}");
        assert_eq!(frames.last(), Some(&want), "{name} ({gpu:?}) mode 1, last frame");
    }
}

#[test]
fn shadow_depth_view_drops_the_cpu_resolve_and_a_kick() {
    check("shadow", shadow, Gpu::Lag, 0, c(2, 1, 0, 1, 0));
    check("shadow", shadow, Gpu::Lag, 1, c(1, 0, 0, 0, 1));
    check("shadow", shadow, Gpu::Done, 0, c(2, 0, 0, 1, 0));
    check("shadow", shadow, Gpu::Done, 1, c(1, 0, 0, 0, 0));
}

#[test]
fn refract_depth_view_drops_the_cpu_resolve() {
    check("refract", refract, Gpu::Lag, 0, c(3, 1, 0, 1, 0));
    check("refract", refract, Gpu::Lag, 1, c(3, 2, 0, 0, 1));
    check("refract", refract, Gpu::Done, 0, c(3, 0, 0, 1, 0));
    check("refract", refract, Gpu::Done, 1, c(3, 0, 0, 0, 0));
}

/// terrain samples no depth (its depth is a renderbuffer): mode 1 changes
/// nothing, and no frame resolves.
#[test]
fn terrain_samples_no_depth_so_mode_1_changes_nothing() {
    for gpu in [Gpu::Lag, Gpu::Done] {
        let off = run_mode(gpu, 0, terrain);
        let on = run_mode(gpu, 1, terrain);
        assert_eq!(off, on, "terrain ({gpu:?})");
        assert!(off.iter().all(|f| f.resolves == 0), "terrain ({gpu:?}): {off:?}");
    }
}
