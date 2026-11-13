// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): ceilings on what one warm frame of glmark2's three slow
//! scenes, terrain, shadow and refract, costs the CPU in the Vivante back end
//! (#1204, #1106, plan #328): kicks, CPU waits for the GPU outside the swap,
//! `finish()` calls outside the swap, CPU tiled copies and CPU depth
//! resolves. Each ceiling is the count at the commit that wrote this file;
//! a change that raises one fails here. A fix that lowers one lowers its
//! ceiling in the same commit. The comment at each ceiling names the planned
//! fix that lowers it, and to what.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL, as `gc_glmark2_mip_offer.rs` does, driven with the GL calls
//! glmark2 (`22c527cb`, `$GLMARK2_SRC`) makes in a frame: `canvas_.clear()`,
//! the scene's `draw()`, then the swap. The models are small and the shaders
//! short (the counts do not depend on either), but every target, attachment,
//! clear, draw, texture binding by stage, blend and `glGenerateMipmap` of a
//! frame is glmark2's, in its order:
//!
//! * terrain: `scene-terrain.cpp:332` (the height/normal chain, then the
//!   terrain chain: terrain, bloom_h, bloom_v, overlay, tilt_h, tilt_v),
//!   `scene-terrain/base-renderer.cpp`, `texture-renderer.cpp:53`,
//!   `terrain-renderer.cpp:43`, `overlay-renderer.cpp:72`;
//!   `docs/notes/gc-slow-terrain.md`, "The 52 kicks".
//! * shadow: `scene-shadow.cpp:404` (`DepthRenderTarget::enable`, the horse,
//!   `disable`, the ground sampling the depth texture, the lit horse);
//!   `docs/notes/gc-slow-shadow-refract.md`, "shadow".
//! * refract: `scene-refract.cpp:394` (`DistanceRenderTarget` 1600x1200
//!   colour and depth, the bunny culled front, `glGenerateMipmap` of the
//!   colour, the bunny into the window with the depth, colour and image
//!   textures); the same note, "refract".
//!
//! The swap is the board's (`src/gl/egl.rs:2396-2413`, a double-buffered
//! window): `flush`, then `finish_previous`. The host harness's window has no
//! present, so its swap is a `finish()`; that difference is the harness's.
//!
//! What is counted, a frame, with what:
//! * kicks: the stand-in's `submit` calls (`frame_submit`);
//! * waits outside the swap: the stand-in's `wait` calls made outside the
//!   swap (`frame_open` on a slot still pending, `finish`, `wait_kick`);
//! * `finish()` calls outside the swap: the back end's `gc_finishes`;
//! * CPU tiled copies: `cache_invd` calls outside the depth resolve: the
//!   CPU tiled copy's read of the rendered map (`tiled_copy.rs` `refresh`)
//!   is the only CPU read of GPU memory in these frames outside the resolve
//!   (the notes' catch-all over `cache_invd`);
//! * CPU depth resolves: calls of the back end table's `depth_resolve`, the
//!   one entry wrapped by this test (its copy of `yakogl_backend_vivante`).
//!
//! Two GPU models, as the harness's GC cost mode has
//! (`docs/notes/gc-slow-host-counts.md`, "Run"): `Lag`, where a kick
//! completes only when the CPU waits for it or for a later one (every check
//! of a pending kick is a wait: the most the board pays), and `Done`, where
//! every kick completes when submitted. Waits are read from `Lag`. The RS
//! path of the tiled copy depends on the model (the same note, "After
//! tiled-copy-rs"), so the tiled copies have a ceiling in each.
//!
//! ```text
//! The library reports ES 2.0, as glmark2's board images
//! (`CONFIG_YAKOGL_REPORT_ES2`, default y) and the host harness
//! (`-DYAKOGL_REPORT_ES2=ON`) build it: under an ES 3.0 report shadow's and
//! refract's unsized `GL_DEPTH_COMPONENT` textures are no attachment
//! (`GlTexture::unsized_depth`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_scene_kicks -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use gc_scenes::*;

/* ---- the ceilings ------------------------------------------------------------------- */

/// terrain, `Lag`. Kicks 14: the canvas clear, the height map, the normal
/// map, the terrain pass, bloom_h, bloom_v, the overlay, tilt_h and tilt_v
/// (9 passes: every render-target change ends the kick), and the 5 GPU
/// mipmap chains, one kick each (`mipmap.rs`: the frame open is submitted
/// first, then the chain's levels share one stream). mip-one-kick took the
/// chains from 43 one-level kicks to 5 (52 -> 14; gc-slow-host-counts.md
/// predicted about 9, from the stream diff's count, which puts each chain in
/// its pass's kick). gc-frames-ring (gc-kick-continue.md C1-C3: a target
/// change does not end the kick) merges the passes between two chains: the
/// canvas clear, height, normal and terrain passes become one kick, 14 -> 11.
/// C5 (the chain stays in the open kick) then takes it to 1, submitted at
/// the swap.
/// Waits outside the swap 14: every `frame_open` (one a kick) reuses one of
/// the 4 stream slots (`GC_FRAMES`) whose kick the lag model still holds.
/// They fall with the kicks: at 2 kicks a frame or fewer the slot reused is
/// the previous frame's, waited for at the swap, so gc-frames-ring with C5
/// takes them to 0 (the harness's lag count was 48.2 before mip-one-kick).
/// No `finish()` and no CPU tiled copy: tiled-copy-rs put the RS copy of the
/// two vertex-stage maps in the stream (finish 1 -> 0, copies 2 -> 0). No
/// depth resolve: terrain's depth is a renderbuffer, never sampled.
const TERRAIN_LAG: Counts =
    Counts { kicks: 14, waits_other: 14, finishes_other: 0, tiled_copies: 0, resolves: 0, waits_swap: 0 };
/// terrain, `Done`: the same kicks, no waits. One of the two maps takes the
/// CPU tiled copy: its producer kick has completed, so `tiled_copy.rs`
/// `refresh` finds nothing pending to tile behind and copies on the CPU
/// (gc-slow-host-counts.md, "After tiled-copy-rs"; the board lies between
/// the two models). gc-frames-ring keeps both maps' producers in the kick
/// being recorded when the terrain draw reads them, which takes it to 0.
const TERRAIN_DONE: Counts =
    Counts { kicks: 14, waits_other: 0, finishes_other: 0, tiled_copies: 1, resolves: 0, waits_swap: 0 };

/// shadow, `Lag`. Kicks 3: the canvas clear (K1), the 1600x1200 depth pass
/// (K2, submitted when the ground's draw resolves the depth it samples), the
/// window's ground and lit horse (K3, at the swap). gc-frames-ring takes them
/// to 2 (K1 and K2 share a stream); depth-sample-gc with C9 to 1.
/// Waits outside the swap 1: the depth resolve's `wait_kick` for K2
/// (`texture.rs` `resolve_depth`; resolve-own-kick made it that kick alone,
/// where it was a `finish()` of every slot: finish 1 -> 0; the harness's
/// lag count was 1.7).
/// depth-sample-gc (the texture unit samples the D24S8 surface, no CPU
/// resolve) takes it to 0.
/// One CPU depth resolve (1600x1200, 7.6 MB read): depth-sample-gc takes it
/// to 0; resolve-wide makes it cheaper, not fewer.
const SHADOW_LAG: Counts =
    Counts { kicks: 3, waits_other: 1, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const SHADOW_DONE: Counts =
    Counts { kicks: 3, waits_other: 0, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };

/// refract, `Lag`. Kicks 4: the canvas clear (K1), the 1600x1200 pass (K2),
/// its 11-level GPU chain in one kick of its own (mip-one-kick: 10 one-level
/// kicks, 13 -> 4; gc-slow-host-counts.md predicted about 3), the window's
/// bunny (at the swap). gc-frames-ring takes them to 3 (K1 shares K2's
/// stream), C5 to 2 (the chain stays in that stream), depth-sample-gc with C9
/// to 1.
/// Waits outside the swap 2: the chain's `frame_open` on a slot the lag model
/// still holds (mip-one-kick took the chain's from 8 to 1; 0 at 2 kicks a
/// frame, gc-frames-ring with C5) and the depth resolve's `wait_kick` for K2
/// (depth-sample-gc takes it to 0). No `finish()` (resolve-own-kick, as in
/// shadow). One CPU depth resolve: depth-sample-gc takes it to 0.
const REFRACT_LAG: Counts =
    Counts { kicks: 4, waits_other: 2, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const REFRACT_DONE: Counts =
    Counts { kicks: 4, waits_other: 0, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };

/// Check `frames` against `ceil` and print them, and the last frame's kicks
/// and waits by step, for the record.
fn check(scene: &str, gpu: Gpu, (frames, events): &(Vec<Counts>, Vec<String>), ceil: &Counts) {
    let max = Counts::max_of(frames);
    eprintln!("{scene} {gpu:?}: a frame at most {max:?}; frames {FIRST_COUNTED}..={FRAMES}: {frames:?}");
    eprintln!("{scene} {gpu:?}: frame {FRAMES}: {}", events.join(", "));
    let over = max.within(ceil);
    assert!(over.is_empty(), "{scene} ({gpu:?}): {over:?} above the ceiling: a frame {max:?}, ceiling {ceil:?}");
}

/* ---- the tests ------------------------------------------------------------------------ */

#[test]
fn terrain_a_frame_within_its_ceilings() {
    check("terrain", Gpu::Lag, &run(Gpu::Lag, terrain), &TERRAIN_LAG);
    check("terrain", Gpu::Done, &run(Gpu::Done, terrain), &TERRAIN_DONE);
}

#[test]
fn shadow_a_frame_within_its_ceilings() {
    check("shadow", Gpu::Lag, &run(Gpu::Lag, shadow), &SHADOW_LAG);
    check("shadow", Gpu::Done, &run(Gpu::Done, shadow), &SHADOW_DONE);
}

#[test]
fn refract_a_frame_within_its_ceilings() {
    check("refract", Gpu::Lag, &run(Gpu::Lag, refract), &REFRACT_LAG);
    check("refract", Gpu::Done, &run(Gpu::Done, refract), &REFRACT_DONE);
}
