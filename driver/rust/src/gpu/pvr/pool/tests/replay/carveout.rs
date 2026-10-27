// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's sequence with the NPOT mip chains at their power-of-two
//! extent, on the AM62P carve-outs that matter: the board's 64 MiB, where
//! terrain and refract were refused (cd738b1a, f8741799), and the glmark2
//! image's own carve-out (`504d4fae`, read from its overlay), where every
//! scene ran (e3749a25, c854e0ae). The freelists are sized per carve-out as
//! `driver/src/pvr_render.c` sizes them (`Cfg::for_carveout`).
//!
//! What the chains are: the GL core lays an NPOT texture with mipmaps out
//! at its power-of-two extent (`src/gl/texture.rs` `pot_chain_layout`,
//! Mesa pvr's rule), so terrain's two mipmapped 800x600 targets become
//! 1024x1024 chains of 5592404 bytes each, and refract's 1600x1200 colour
//! texture a 2048x2048 chain of 22369620 bytes: the byte counts of the
//! board's `out of GPU memory` lines. The first `glGenerateMipmap` grows the
//! one-level storage to the chain, releasing the old storage first
//! (`Replay::grow`).
//!
//! The measured values the tests hold the model to come from
//! docs/notes/am62p-render-pool-glmark2.md: the glmark2 host harness's
//! per-scene high-water on the PowerVR build (terrain 38732 KiB, shadow
//! 17580, refract 45160 above the back end's fixed part), the driver's
//! persistent set of 25224 KiB at 64 MiB, and the 256 MiB board's end bump
//! of 91356 KiB and refract-end live of 139212 KiB.

use super::*;
use std::{eprintln, format, string::String};

/// The AM62P render pool for a carve-out of `bytes`, KiB: the carve-out
/// less the firmware heap and the page tables (`POOL_AM62P`).
fn pool_kib(bytes: usize) -> u32 {
    ((bytes - POOL_AM62P as usize) / 1024) as u32
}

/// A 1024x1024 chain for an 800x600 RGBA8 texture and a 2048x2048 one for
/// 1600x1200: 4 x (n² + (n/2)² + ... + 1) bytes.
const CHAIN_800: (&str, u32) = ("800x600 POT mip chain (1024x1024)", 5_592_404);
const CHAIN_1600: (&str, u32) = ("1600x1200 POT mip chain (2048x2048)", 22_369_620);

/// The 64 MiB board and the glmark2 image's overlay.
const MIB_64: usize = 64 << 20;
const OVERLAY: &str =
    include_str!("../../../../../../../samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.overlay");

/// The glmark2 image's AM62P carve-out: the size of the `reg` of
/// `gpu_carveout_glmark2` in its overlay, which must be the GPU's
/// `memory-region`. A change to the overlay changes what the tests below
/// run on.
fn glmark2_image_carveout() -> usize {
    assert!(
        OVERLAY.contains("memory-region = <&gpu_carveout_glmark2>;"),
        "the overlay no longer gives the GPU the gpu_carveout_glmark2 region"
    );
    let node = OVERLAY.find("gpu_carveout_glmark2:").expect("the overlay's gpu_carveout_glmark2 node");
    let reg = &OVERLAY[node..];
    let reg = &reg[reg.find("reg = <").expect("its reg") + "reg = <".len()..];
    let reg = &reg[..reg.find('>').expect("the reg's end")];
    let mut cells = reg
        .split_whitespace()
        .map(|c| usize::from_str_radix(c.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("reg cell {c}: {e}")));
    let (_base, size) = (cells.next().expect("the reg's base"), cells.next().expect("the reg's size"));
    assert!(cells.next().is_none(), "reg has more than two cells");
    size
}

/// The scenes with their chains, as the test variants want them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Chains {
    /// NPOT textures stay at their level 0 (before the npot change).
    Level0Only,
    /// The POT-extent chains (main since the npot change).
    Pot,
}

/// What the replay shows for one carve-out.
struct Run {
    /// The render pool, KiB.
    pool: u32,
    /// The driver's persistent set: live after the boot, the GL leftovers
    /// and the window's depth buffer, before jellyfish, KiB.
    persistent: u32,
    /// Each scene's live high-water, KiB.
    live_hw: Vec<(u32, u32)>,
    /// The physical cursor at the run's end, KiB (the board's "bump").
    end_bump: u32,
    /// The top region's high-water, KiB.
    top_hw: u32,
    /// Every refusal: (scene, what, KiB) with GENERAL at the request.
    refused: Vec<((u32, &'static str, u32), Shot)>,
}

impl Run {
    fn live_hw_of(&self, scene: u32) -> u32 {
        self.live_hw.iter().find(|(s, _)| *s == scene).map_or(0, |(_, hw)| *hw)
    }
}

/// glmark2's entries 0-24 on a carve-out of `bytes` with the driver's top
/// region, the board's unnamed blocks from shadow on
/// (`UNNAMED_AT_THE_BOARD`), and the NPOT chains as `chains` says: terrain's
/// two mipmapped 800x600 targets grown after its first draw (glmark2
/// `scene-terrain/base-renderer.cpp` generates them each frame), refract's
/// colour texture grown after its depth buffer (`scene-refract.cpp` draws
/// into it, then generates). Both as `Replay::grow`: old storage out first.
fn glmark2_on(bytes: usize, chains: Chains) -> Run {
    let mut rp = Replay::with_carveout(bytes, YD_PVR_POOL_TOP_ABOVE);
    rp.boot_for(&Cfg::for_carveout(bytes));
    before_jellyfish(&mut rp);
    let persistent = rp.live();
    rp.scene = 21;
    for _ in 0..32 {
        rp.a(21, "caustic 256x256 mip chain", GEN, 349_525);
    }
    rp.scene(21, JELLYFISH);
    rp.teardown();
    rp.scene(22, TERRAIN);
    if chains == Chains::Pot {
        for _ in 0..2 {
            rp.grow(22, "800x600 target", CHAIN_800);
        }
    }
    rp.teardown();
    rp.scene(23, SHADOW);
    for _ in 0..UNNAMED_AT_THE_BOARD {
        rp.a(LIFE, "unnamed", GEN, 256 * K);
    }
    rp.teardown();
    rp.scene(24, REFRACT_SETUP);
    rp.a(24, "1600x1216 depth buffer", GEN, DS_1600);
    if chains == Chains::Pot {
        rp.grow(24, "1600x1200 colour texture (fmt 0)", CHAIN_1600);
    }
    rp.teardown();
    let end_bump = rp.r.pool_used() / K;
    let top_hw = rp.r.k.pool_top_hw / K;
    let refused = rp.refused.iter().copied().zip(rp.refused_at.iter().copied()).collect();
    Run { pool: pool_kib(bytes), persistent, live_hw: rp.live_hw, end_bump, top_hw, refused }
}

fn say(name: &str, run: &Run) {
    let scenes: Vec<String> = [21, 22, 23, 24].iter().map(|&n| format!("{n}: {}", run.live_hw_of(n))).collect();
    eprintln!(
        "glmark2 replay, {name}: pool {} KiB, persistent set {} KiB, live high-water KiB [{}], end bump {} KiB, top high-water {} KiB, refused {:?}",
        run.pool,
        run.persistent,
        scenes.join(", "),
        run.end_bump,
        run.top_hw,
        run.refused
    );
}

/// The harness's per-scene needs above the back end's fixed part, KiB
/// (docs/notes/am62p-render-pool-glmark2.md, "Harness"), and the driver's
/// persistent set at 64 MiB.
const HARNESS_TERRAIN: u32 = 38732;
const HARNESS_SHADOW: u32 = 17580;
const HARNESS_REFRACT: u32 = 45160;
const PERSISTENT_64: u32 = 25224;

/// The 256 MiB board run (am62p-common-carveout.md at e3749a25, read as
/// `ab43a20c5:docs/board-sessions/am62p-common-carveout.md`): the bump at
/// the end, and live at refract's end with its frees still held.
const BOARD_256_END_BUMP: u32 = 91356;
const BOARD_256_REFRACT_LIVE: u32 = 139212;

/// The pin of cd738b1a and f8741799: at 64 MiB the POT-extent chains are
/// refused, and only they: terrain's two 1024x1024 chains and refract's
/// 2048x2048 one. It is a shortfall in total, not fragmentation: at
/// terrain's first chain and at refract's, the free lists and the gap
/// together hold less than the request (terrain: 2604 + 1816 KiB against
/// 5462; refract: 7736 + 3676 against 21846, the board's "21848 KiB
/// requested, 2744 KiB left"), and each scene's peak with its chains in
/// place (its level-0 peak plus what the chains add) is more than the
/// whole 59200 KiB pool: terrain 63832 KiB, refract 69596. Terrain's
/// second chain is refused with more bytes free than it asks for only
/// because the first refusal gave the first target's level 0 back; the
/// board served one of the two. The model's persistent set is the board's
/// 25224 KiB, and the harness's per-scene needs put terrain and refract
/// over the pool and shadow under it (served, as on the board).
#[test]
fn glmark2_npot_chains_at_64_mib_are_refused_for_want_of_memory() {
    let run = glmark2_on(MIB_64, Chains::Pot);
    let level0 = glmark2_on(MIB_64, Chains::Level0Only);
    say("64 MiB, POT chains", &run);
    assert_eq!(run.pool, 59200, "the 64 MiB board's render pool");
    assert!(run.persistent.abs_diff(PERSISTENT_64) <= 64, "persistent set {} KiB", run.persistent);
    let chain_800 = CHAIN_800.1.div_ceil(K);
    let chain_1600 = CHAIN_1600.1.div_ceil(K);
    let refused: Vec<(u32, &str, u32)> = run.refused.iter().map(|(r, _)| *r).collect();
    assert_eq!(
        refused,
        [(22, CHAIN_800.0, chain_800), (22, CHAIN_800.0, chain_800), (24, CHAIN_1600.0, chain_1600)],
        "exactly the three POT chains are refused"
    );
    for i in [0, 2] {
        let ((scene, what, kib), s) = &run.refused[i];
        assert!(s.free + s.gap < *kib, "scene {scene} {what}: {kib} KiB refused with {s:?}: that is fragmentation");
    }
    /* each scene's peak with its chains: the level-0 run's peak plus what
     * the chains add over the level-0 storage they replace, in pages */
    let pages_kib = |bytes: u32| bytes.div_ceil(4096) * 4;
    let terrain = level0.live_hw_of(22) + 2 * (pages_kib(CHAIN_800.1) - pages_kib(800 * 600 * 4));
    let refract = level0.live_hw_of(24) + pages_kib(CHAIN_1600.1) - pages_kib(TEX_1600);
    assert_eq!((terrain, refract), (63832, 69596));
    assert!(terrain > run.pool && refract > run.pool, "no layout of the pool holds either scene");
    assert!(PERSISTENT_64 + HARNESS_TERRAIN > run.pool, "terrain is over the pool by the harness's measure");
    assert!(PERSISTENT_64 + HARNESS_REFRACT > run.pool, "refract is over the pool by the harness's measure");
    assert!(PERSISTENT_64 + HARNESS_SHADOW < run.pool, "shadow fits by the harness's measure");
    assert!(run.live_hw_of(23) < run.pool);
}

/// The baseline the npot change broke: the same run with the NPOT textures
/// at their level 0 refuses nothing at 64 MiB (the common sets before
/// 2026-10-14 ran every scene there).
#[test]
fn glmark2_before_npot_chains_fits_64_mib() {
    let run = glmark2_on(MIB_64, Chains::Level0Only);
    say("64 MiB, level 0 only", &run);
    assert_eq!(run.refused, []);
    for n in [21, 22, 23, 24] {
        assert!(run.live_hw_of(n) < run.pool, "scene {n}: live high-water {} of {} KiB", run.live_hw_of(n), run.pool);
    }
}

/// The fix of `504d4fae`, pinned to the overlay: on the glmark2 image's
/// carve-out nothing is refused and every scene's live high-water stays
/// under 3/4 of the pool (the margin the GC7000's glmark2 row uses). With
/// the overlay at 256 MiB: refract 139356 of 255808 KiB. Shrinking the
/// overlay's carve-out below what glmark2 needs fails this test; at the
/// board's 64 MiB it fails as the test above shows.
#[test]
fn glmark2_fits_the_glmark2_images_carveout() {
    let bytes = glmark2_image_carveout();
    let run = glmark2_on(bytes, Chains::Pot);
    say(&format!("the overlay's {} MiB, POT chains", bytes >> 20), &run);
    assert_eq!(run.refused, [], "refused on the overlay's {} MiB carve-out", bytes >> 20);
    for n in [21, 22, 23, 24] {
        let hw = run.live_hw_of(n);
        assert!(
            hw * 4 < run.pool * 3,
            "scene {n}: live high-water {hw} KiB is not under 3/4 of the {} KiB pool",
            run.pool
        );
    }
}

/// The reconstruction against the 256 MiB board run, so that a change to a
/// scene's list keeps the model honest. Live at refract's peak is within
/// 1 MiB of the board's 139212 KiB (model: 139356). The bump at the end is
/// within 5 MiB of the board's 91356 KiB (model: 96176): the model's 24
/// unnamed blocks are 256 KiB each, not above the top region's threshold,
/// so they sit on the bump (6144 KiB of it), where the board's 6 MiB
/// mostly did not (without them the model's bump is 90032 KiB). Where the
/// board put them does not change any scene's live bytes.
#[test]
fn the_replay_matches_the_256_mib_board_run() {
    let run = glmark2_on(256 << 20, Chains::Pot);
    say("256 MiB, POT chains", &run);
    assert_eq!(run.pool, 255808);
    assert_eq!(run.refused, []);
    let refract = run.live_hw_of(24);
    assert!(
        refract.abs_diff(BOARD_256_REFRACT_LIVE) <= 1024,
        "refract's peak {refract} KiB, the board's {BOARD_256_REFRACT_LIVE}"
    );
    assert!(
        run.end_bump.abs_diff(BOARD_256_END_BUMP) <= 5 * 1024,
        "end bump {} KiB, the board's {BOARD_256_END_BUMP}",
        run.end_bump
    );
}

/// `pvr_render.c`'s sizing step: from 128 MiB the render service takes a
/// quarter of the carve-out as the freelists' grow pool and backs 2048
/// pages per global list with no reserve, so the persistent set at 128 MiB
/// is about 36928 KiB above that at 120 MiB (32768 of pool, 4096 of list
/// pages, 64 of PFN arrays), and at 256 MiB about 94984 KiB.
#[test]
fn the_freelist_grow_pool_takes_a_quarter_from_128_mib() {
    let at_120 = glmark2_on(120 << 20, Chains::Pot);
    let at_128 = glmark2_on(128 << 20, Chains::Pot);
    let at_256 = glmark2_on(256 << 20, Chains::Pot);
    eprintln!(
        "persistent set: 120 MiB {} KiB, 128 MiB {} KiB, 256 MiB {} KiB",
        at_120.persistent, at_128.persistent, at_256.persistent
    );
    assert!(at_120.persistent.abs_diff(PERSISTENT_64) <= 64, "below 128 MiB the set is the 64 MiB board's");
    let step = at_128.persistent - at_120.persistent;
    assert!(step.abs_diff(36928) <= 64, "the step at 128 MiB is {step} KiB");
    assert!(at_256.persistent.abs_diff(94984) <= 64, "at 256 MiB the set is {} KiB", at_256.persistent);
}
