// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's sequence with the partial render's scratch blocks reserved at
//! the window's size before the first scene: the design's third risk
//! (docs/notes/pvr-render-pool.md, "Design: the firmware's partial render",
//! part 2 "The SPM scratch" and "The risks the sizing carries": "the render
//! pool must hold one more 1.9 MB colour scratch, plus about 2.4 MB for the
//! PR depth and stencil, at the window size, where glmark2 already runs it
//! dry").
//!
//! The two blocks and where each stands:
//! - **the SPM colour scratch**, `src/pvr/spm.rs` `scratch_size` for one
//!   sample, one output dword and no tile buffer (the load shader's
//!   configuration): the width aligned to the PBE line-stride alignment (2)
//!   x height x 4 bytes. The landed backend (`src/pvr/backend.rs`,
//!   `pr_scratch_for`) takes it through `osal::gpu_alloc(HEAP_GENERAL_BASE)`,
//!   which on the board is `pvr_render_alloc(GENERAL)` (src/osal/zephyr.c)
//!   and so `YdPvrPool::alloc`: a GENERAL request above the top region's
//!   threshold, placed in the top region. It is taken at the first kick of
//!   the size, which for the window is before the first scene's depth
//!   buffer, and never freed: here, right after the boot.
//! - **the PR depth/stencil scratch**, the design's 4 + 1 bytes a pixel.
//!   `docs/notes/pvr-pr-zs.md` decided that the partial render stores into
//!   the frame's own depth/stencil buffer, so the landed code allocates no
//!   such block. It is replayed all the same, as the risk names it, so the
//!   cost of that decision's reversal is on record.
//!
//! Two placements are compared: the pool's own (the landed path above), and
//! a driver block from the bump (`bo_alloc`, the way the freelists and the
//! HWRT members are taken), to answer where the reservation should come
//! from. The freelist grow's on-demand refill is a `bo_alloc` from the bump
//! too (render.rs, the FWCCB service's `src`), so a refill is served exactly
//! when the gap holds it; each scene's frames are kicked in the state
//! `Replay::frame_shots` records.
//!
//! The runs are on the 64 MiB board's layout (the gl_test image's), with the
//! top region at the driver's threshold and the model's board-calibrated
//! unnamed blocks (`UNNAMED_AT_THE_BOARD`), against the run without the
//! reservation. The glmark2 image's own carve-out is the 256 MiB of its
//! overlay (`carveout.rs`), where these blocks are not in question.

use super::*;

/// The SPM colour scratch of a `w` x `h` render (spm.rs `scratch_size`,
/// one sample, one output dword, no tile buffer).
const fn colour_scratch(w: u32, h: u32) -> u32 {
    w.next_multiple_of(2) * h * 4
}

/// The design's PR depth/stencil scratch: 4 + 1 bytes a pixel.
const fn zs_scratch(w: u32, h: u32) -> u32 {
    w * h * 5
}

/// Page-rounded KiB of a block, what the pool takes for it.
const fn pages_kib(bytes: u32) -> u32 {
    bytes.div_ceil(4096) * 4
}

/// One on-demand refill of a global freelist's one-grow reserve: 256 pages
/// and 3 more (`freelist::ondemand_bytes`), the board's "the render pool
/// refused 1036 KiB".
const REFILL_KIB: u32 = (256 * 4096 + 0x3000) / 1024;

const COLOUR: &str = "SPM colour scratch";
const ZS: &str = "PR depth/stencil scratch";

/// The blocks a `w` x `h` window reserves: the colour scratch, and the
/// depth/stencil one when `zs`.
fn pr_blocks(w: u32, h: u32, zs: bool) -> Vec<(&'static str, u32)> {
    let mut v = std::vec![(COLOUR, colour_scratch(w, h))];
    if zs {
        v.push((ZS, zs_scratch(w, h)));
    }
    v
}

/// Where the reservation is taken from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Place {
    /// A GENERAL request through the pool, the landed path: large, so the
    /// top region.
    Pool,
    /// A driver block from the bump, as the freelists and HWRT members.
    Bump,
}

/// What the replay shows with the reservation.
struct PrRun {
    /// The reserved blocks' heap offsets, in `pr_blocks` order, and their
    /// page-rounded KiB together.
    offs: Vec<u32>,
    reserved_kib: u32,
    /// The frame sets' four 1 MiB GENERAL arenas, heap offsets.
    arenas: Vec<u32>,
    g: Glmark2,
    /// GENERAL at each scene's frames (after its ops): (scene, state).
    frames: Vec<(u32, Shot)>,
}

impl PrRun {
    /// GENERAL at scene `n`'s frames: the last state recorded for it
    /// (refract's is the one with its depth buffer).
    fn frame(&self, n: u32) -> Shot {
        self.frames.iter().rev().find(|(s, _)| *s == n).map_or_else(|| panic!("scene {n}'s frames"), |(_, s)| *s)
    }

    /// The scenes at whose frames a refill from the bump is refused.
    fn no_refill(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.frames.iter().filter(|(_, s)| s.gap < REFILL_KIB).map(|(n, _)| *n).collect();
        v.dedup();
        v
    }

    /// What the tests compare between two placements.
    fn outcome(&self) -> Outcome {
        Outcome {
            before_refract: self.g.before_refract,
            at_depth_buffer: self.g.at_depth_buffer,
            depth_buffer_served: self.g.depth_buffer_served,
            refused: self.g.refused.clone(),
            frame_gaps: self.frames.iter().map(|(n, s)| (*n, s.gap)).collect(),
        }
    }
}

/// Everything a placement decides for the run: GENERAL before refract and
/// at its depth buffer, whether that was served, every refusal, and the gap
/// at every scene's frames (scene, KiB).
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    before_refract: Shot,
    at_depth_buffer: Shot,
    depth_buffer_served: bool,
    refused: Vec<(u32, &'static str, u32)>,
    frame_gaps: Vec<(u32, u32)>,
}

/// glmark2's entries 0-24 on the 64 MiB board with the top region, with
/// `blocks` reserved from `place` right after the boot (kept for the run),
/// and `unnamed` 256 KiB blocks kept from shadow on.
fn glmark2_reserving(blocks: &[(&'static str, u32)], place: Place, unnamed: u32) -> PrRun {
    let mut rp = Replay::new(YD_PVR_POOL_TOP_ABOVE);
    rp.boot();
    let arenas: Vec<u32> =
        rp.live.iter().filter(|b| b.size == 1024 * K && b.heap == GEN).map(|b| b.bo.heap_off).collect();
    let mut offs = Vec::new();
    let mut reserved_kib = 0;
    for &(what, bytes) in blocks {
        let off = match place {
            Place::Pool => {
                assert!(rp.a(LIFE, what, GEN, bytes), "{what} of {bytes} bytes refused at the boot");
                rp.live.last().expect("the block").bo.heap_off
            }
            Place::Bump => {
                let bo = bo_alloc(&mut rp.r.k, GENERAL_HEAP_BASE, &mut rp.r.bumps[GEN], bytes, L0_FLAGS_DATA);
                assert!(!bo.cpu.is_null(), "{what} of {bytes} bytes refused by the bump at the boot");
                rp.note_live();
                bo.heap_off
            }
        };
        offs.push(off);
        reserved_kib += pages_kib(bytes);
    }
    let g = glmark2_scenes(&mut rp, unnamed);
    PrRun { offs, reserved_kib, arenas, g, frames: rp.frame_shots }
}

/// The run without the reservation, the baseline (the top-region arm of
/// `glmark2_with_the_top_region_fits_refract_after_shadow_and_terrain`).
fn baseline(unnamed: u32) -> PrRun {
    glmark2_reserving(&[], Place::Pool, unnamed)
}

fn say(name: &str, r: &PrRun) {
    let g = &r.g;
    let late: Vec<(u32, u32)> = r.frames.iter().filter(|(n, _)| *n >= 21).map(|(n, s)| (*n, s.gap)).collect();
    eprintln!(
        "glmark2 replay, PR scratch {name}: reserved {} KiB at {:x?}; before refract {:?} (contiguous {} KiB, refract needs {} KiB); at its depth buffer {:?}, served {}; refused {:?}; gap at the frames of jellyfish..refract {:?} KiB, no refill at {:?}",
        r.reserved_kib,
        r.offs,
        g.before_refract,
        g.before_refract.contiguous(),
        REFRACT_NEEDS,
        g.at_depth_buffer,
        g.depth_buffer_served,
        g.refused,
        late,
        r.no_refill()
    );
}

/// The blocks' sizes, each a literal checked by hand: 800x600 RGBA8 is
/// 800 x 600 x 4 = 1,920,000 bytes (the design's "1.92 MB"), 469 pages,
/// 1876 KiB; its depth/stencil 800 x 600 x 5 = 2,400,000 bytes ("about
/// 2.4 MB"), 586 pages, 2344 KiB. At 1600x1200 four times each: 7,680,000
/// bytes (7500 KiB) and 9,600,000 (9376 KiB). A refill is 1036 KiB.
#[test]
fn the_pr_scratch_sizes_are_the_designs() {
    assert_eq!((colour_scratch(800, 600), pages_kib(colour_scratch(800, 600))), (1_920_000, 1876));
    assert_eq!((zs_scratch(800, 600), pages_kib(zs_scratch(800, 600))), (2_400_000, 2344));
    assert_eq!((colour_scratch(1600, 1200), pages_kib(colour_scratch(1600, 1200))), (7_680_000, 7500));
    assert_eq!((zs_scratch(1600, 1200), pages_kib(zs_scratch(1600, 1200))), (9_600_000, 9376));
    assert_eq!(REFILL_KIB, 1036);
}

/// The window's colour scratch alone (1876 KiB, the landed backend's one
/// block) is served from the top region right under the frame sets' four
/// arenas, which do not move. It costs the run its tightest point, terrain:
/// the gap at terrain's frames is 1816 KiB without it (one 1036 KiB refill
/// fits there) and 4 KiB with it, so terrain's last HWRT member loses its
/// 64 KiB region-header block (the only refusal; `hwrt_get` then keeps an
/// unbuilt member) and no refill fits at terrain's frames any more, as on
/// the board. Refract is still served: its depth buffer with 1864 KiB of
/// gap left (3676 without the scratch), and before its set-up one range of
/// 24464 KiB against its 22600 (26276 before). Each is 1812 KiB less: the
/// scratch, less the 64 KiB terrain did not get. The terrain refusal does
/// not depend on the unnamed blocks, which come at shadow: it is the same
/// with none.
#[test]
fn the_windows_colour_scratch_fits_but_takes_terrains_last_hwrt_block_and_its_refill() {
    let b = baseline(UNNAMED_AT_THE_BOARD);
    say("none", &b);
    assert_eq!(b.g.refused, []);
    assert_eq!(b.frame(22).gap, 1816, "terrain's frames without the scratch: one refill fits");
    assert_eq!(b.frame(24).gap, 3676, "refract's frames, its depth buffer in place");
    assert_eq!(b.g.before_refract.contiguous(), 26276);
    assert_eq!(b.no_refill(), []);

    let r = glmark2_reserving(&pr_blocks(800, 600, false), Place::Pool, UNNAMED_AT_THE_BOARD);
    say("800x600 colour, from the pool", &r);
    let end = CARVEOUT as u32;
    assert_eq!(r.arenas, (1..=4).map(|i| TOP_VA + end - i * 1024 * K).collect::<Vec<u32>>(), "the frame sets stay");
    assert_eq!(r.offs, [TOP_VA + end - 4096 * K - 1876 * K], "the top region, under the arenas");
    assert_eq!(r.offs, [0x43A2_B000]);
    assert_eq!(r.reserved_kib, 1876);
    assert_eq!(r.g.refused, [(22, "a driver block", 64)], "terrain's last HWRT member's region headers");
    assert_eq!(r.frame(22).gap, 4, "terrain's frames: no refill");
    assert_eq!(r.no_refill(), [22]);
    assert!(r.g.depth_buffer_served);
    assert_eq!(r.frame(24).gap, 1864, "refract's frames, its depth buffer in place");
    assert_eq!(r.g.before_refract.contiguous(), 24464);
    assert_eq!(b.g.before_refract.contiguous() - r.g.before_refract.contiguous(), 1876 - 64);
    assert_eq!(b.frame(24).gap - r.frame(24).gap, 1876 - 64);

    let r0 = glmark2_reserving(&pr_blocks(800, 600, false), Place::Pool, 0);
    assert_eq!(r0.g.refused, [(22, "a driver block", 64)], "no unnamed blocks: terrain is refused all the same");
    assert_eq!((r0.frame(22).gap, r0.no_refill()), (4, std::vec![22]));
}

/// With the design's depth/stencil scratch too (4220 KiB in all), terrain
/// loses one of its four 800x600 targets and its 800x608 depth buffer (the
/// scene cannot render), and refract its depth buffer: before refract's
/// set-up the one range is 22056 KiB, short of its 22600. Without the
/// unnamed blocks refract is served again; terrain's two refusals stay.
#[test]
fn the_designs_depth_stencil_scratch_too_refuses_terrains_target_and_refracts_depth_buffer() {
    let r = glmark2_reserving(&pr_blocks(800, 600, true), Place::Pool, UNNAMED_AT_THE_BOARD);
    say("800x600 colour + depth/stencil, from the pool", &r);
    assert_eq!(r.reserved_kib, 1876 + 2344);
    assert_eq!(r.offs, [0x43A2_B000, 0x437E_1000], "both in the top region, under the arenas");
    assert_eq!(
        r.g.refused,
        [(22, "800x600 target", 1875), (22, "800x608 depth buffer", 1900), (24, "1600x1216 depth buffer", 7600)]
    );
    assert!(!r.g.depth_buffer_served);
    assert_eq!(r.g.before_refract.contiguous(), 22056);
    assert!(r.g.before_refract.contiguous() < REFRACT_NEEDS);
    let r0 = glmark2_reserving(&pr_blocks(800, 600, true), Place::Pool, 0);
    assert_eq!(r0.g.refused, [(22, "800x600 target", 1875), (22, "800x608 depth buffer", 1900)]);
    assert!(r0.g.depth_buffer_served);
}

/// A 1600x1200 window's blocks do not fit the 64 MiB layout's glmark2: the
/// colour scratch alone (7500 KiB) refuses two of terrain's 800x600 targets,
/// its depth buffer and an HWRT block, and refract's depth buffer (the one
/// range before refract 18840 KiB of its 22600); with the depth/stencil
/// scratch (16876 KiB in all) terrain's indices, two mip chains, all four
/// targets and its depth buffer go, then shadow's HWRT member and every
/// unnamed block, then refract's depth buffer: 33 refusals, 11 without the
/// unnamed blocks, and the gap at terrain's, shadow's and refract's frames
/// is under a refill.
#[test]
fn a_1600x1200_scratch_does_not_fit_the_64_mib_layouts_glmark2() {
    let r = glmark2_reserving(&pr_blocks(1600, 1200, false), Place::Pool, UNNAMED_AT_THE_BOARD);
    say("1600x1200 colour, from the pool", &r);
    assert_eq!(r.offs, [0x434A_D000]);
    assert_eq!(
        r.g.refused,
        [
            (22, "800x600 target", 1875),
            (22, "800x600 target", 1875),
            (22, "800x608 depth buffer", 1900),
            (22, "a driver block", 64),
            (24, "1600x1216 depth buffer", 7600),
        ]
    );
    assert_eq!((r.g.before_refract.contiguous(), r.frame(22).gap), (18840, 32));
    assert_eq!(r.no_refill(), [22]);

    let r = glmark2_reserving(&pr_blocks(1600, 1200, true), Place::Pool, UNNAMED_AT_THE_BOARD);
    say("1600x1200 colour + depth/stencil, from the pool", &r);
    assert_eq!(r.reserved_kib, 7500 + 9376);
    assert_eq!(r.g.refused.len(), 33);
    assert_eq!(r.g.refused[0], (22, "terrain indices", 3072));
    assert_eq!(r.g.refused.iter().filter(|(_, what, _)| *what == "unnamed").count(), 22);
    assert_eq!(r.g.refused[32], (24, "1600x1216 depth buffer", 7600));
    assert_eq!((r.g.before_refract.contiguous(), r.frame(22).gap, r.frame(23).gap), (15112, 152, 12));
    assert_eq!(r.no_refill(), [22, 23, 24]);
    let r0 = glmark2_reserving(&pr_blocks(1600, 1200, true), Place::Pool, 0);
    assert_eq!(r0.g.refused.len(), 11);
    assert!(!r0.g.depth_buffer_served);
}

/// Where the reservation comes from makes no difference to the run: a
/// permanent block taken before any scene's lies at one end of the gap or
/// the other, and the gap is the one piece both ends draw on. For every
/// size and both placements, GENERAL before refract and at its depth buffer,
/// every refusal and the gap at every scene's frames are the same; only the
/// block's address differs (the top region's `TOP_VA` range, or the bump's
/// first free offset). So the landed path, the pool's own GENERAL request
/// (the top region), is the right one: it needs no new entry point, and a
/// bump block would buy nothing.
#[test]
fn the_top_region_and_the_bump_give_the_reservation_the_same_headroom() {
    for (w, h) in [(800, 600), (1600, 1200)] {
        for zs in [false, true] {
            for unnamed in [0, UNNAMED_AT_THE_BOARD] {
                let blocks = pr_blocks(w, h, zs);
                let pool = glmark2_reserving(&blocks, Place::Pool, unnamed);
                let bump = glmark2_reserving(&blocks, Place::Bump, unnamed);
                assert_eq!(pool.outcome(), bump.outcome(), "{w}x{h}, zs {zs}, {unnamed} unnamed");
                assert_eq!(pool.reserved_kib, bump.reserved_kib);
                assert!(pool.offs.iter().all(|&o| o >= TOP_VA), "{w}x{h}: the pool places it in the top region");
                assert!(bump.offs.iter().all(|&o| o < TOP_VA), "{w}x{h}: the bump places it below");
                assert_eq!(pool.arenas, bump.arenas);
            }
        }
    }
}
