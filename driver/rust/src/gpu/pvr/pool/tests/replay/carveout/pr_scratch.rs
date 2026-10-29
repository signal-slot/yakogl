// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The firmware's partial render needs two blocks the render pool does not
//! hold today (docs/notes/pvr-render-pool.md, "Design: the firmware's
//! partial render", "The risks the sizing carries"): the SPM colour scratch
//! (4 bytes a pixel) and the PR depth/stencil scratch (4 + 1 bytes a
//! pixel), both reserved once at init for the window's size.
//!
//! This replays glmark2's sequence (`glmark2_with`) with the two blocks
//! reserved right after the boot (after the frame sets and the backend's
//! statics, where the backend's init would ask for them), by the two ways
//! the pool offers, and says what that costs. No policy changes: the
//! `Top` arm is a plain GENERAL `alloc` (both blocks are far over
//! `pool_top_above`, so the pool puts them in the top region, at the
//! carveout's tail), the `Bump` arm a driver block from the bump
//! (`bo_alloc`, as the render service's statics, i.e. "the general pool"
//! below the top region).
//!
//! The replay's own sequence keeps the 800x600 window (its depth buffer,
//! its HWRT members) at both sizes: the 1600x1200 variant reserves for a
//! window of that size and leaves the rest as glmark2 on the 800x600
//! window runs, which is the headroom question (what the reservation takes
//! from the later blocks), not a second sequence.

use super::*;

/// 4 + 1 bytes a pixel for the depth/stencil scratch, 4 for the colour.
pub const COLOUR_BPP: u32 = 4;
pub const ZS_BPP: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// A GENERAL `alloc` at init: the top region.
    Top,
    /// The bump (the general pool below the top region).
    Bump,
}

#[derive(Clone, Copy)]
pub struct Reserve {
    pub place: Place,
    pub w: u32,
    pub h: u32,
}

impl Reserve {
    pub fn colour(&self) -> u32 {
        self.w * self.h * COLOUR_BPP
    }

    pub fn zs(&self) -> u32 {
        self.w * self.h * ZS_BPP
    }

    /// Reserve both blocks; false when either is refused.
    pub fn reserve(&self, rp: &mut Replay) -> bool {
        let mut ok = true;
        for (what, bytes) in [("PR colour scratch", self.colour()), ("PR depth/stencil scratch", self.zs())] {
            ok &= match self.place {
                Place::Top => rp.a(LIFE, what, GEN, bytes),
                Place::Bump => rp.drv_ok(GEN, bytes),
            };
        }
        ok
    }
}

/// (scene, what, how many) of the blocks a comparison names.
type Named = Vec<(u32, &'static str, usize)>;

/// What the reservation changed against the run without it.
struct Cost {
    /// Both blocks were served at init.
    reserved: bool,
    /// The pool less the highest scene high-water, KiB.
    headroom: i64,
    /// The refusals in order, folded by (scene, what, KiB): (.., times).
    refused: Vec<(u32, &'static str, u32, usize)>,
    /// Blocks served in both runs at different device addresses.
    moved: Named,
    /// Blocks the base run served that this one does not.
    lost: Named,
}

fn fold(items: impl Iterator<Item = (u32, &'static str)>) -> Named {
    let mut out: Named = Vec::new();
    for (s, w) in items {
        match out.iter_mut().find(|e| (e.0, e.1) == (s, w)) {
            Some(e) => e.2 += 1,
            None => out.push((s, w, 1)),
        }
    }
    out
}

/// Blocks keyed by (scene, what, n-th of that name).
fn keyed(v: &[(u32, &'static str, u64)]) -> Vec<((u32, &'static str, usize), u64)> {
    let mut seen: Vec<((u32, &'static str), usize)> = Vec::new();
    let mut out = Vec::new();
    for &(s, w, a) in v {
        let n = match seen.iter_mut().find(|e| e.0 == (s, w)) {
            Some(e) => {
                e.1 += 1;
                e.1
            }
            None => {
                seen.push(((s, w), 0));
                0
            }
        };
        out.push(((s, w, n), a));
    }
    out
}

fn cost(bytes: usize, pr: Reserve) -> Cost {
    let base = glmark2_on(bytes, Chains::Pot);
    let run = glmark2_with(bytes, Chains::Pot, Some(pr));
    let hw = [21, 22, 23, 24].iter().map(|&n| run.live_hw_of(n)).max().unwrap();
    let mut refused: Vec<(u32, &'static str, u32, usize)> = Vec::new();
    for ((s, w, k), _) in &run.refused {
        match refused.last_mut() {
            Some(e) if (e.0, e.1, e.2) == (*s, *w, *k) => e.3 += 1,
            _ => refused.push((*s, *w, *k, 1)),
        }
    }
    let (kb, kr) = (keyed(&base.placed), keyed(&run.placed));
    let moved = fold(
        kb.iter().filter_map(|(k, a)| kr.iter().find(|(k2, _)| k2 == k).filter(|(_, a2)| a2 != a).map(|_| (k.0, k.1))),
    );
    let lost = fold(kb.iter().filter(|(k, _)| !kr.iter().any(|(k2, _)| k2 == k)).map(|(k, _)| (k.0, k.1)));
    Cost {
        reserved: !run.refused.iter().any(|((_, w, _), _)| w.contains("scratch")),
        headroom: i64::from(run.pool) - i64::from(hw),
        refused,
        moved,
        lost,
    }
}

fn moved_n(c: &Cost) -> usize {
    c.moved.iter().map(|m| m.2).sum()
}

const WINDOW: (u32, u32) = (800, 600);
const BIG: (u32, u32) = (1600, 1200);

fn at(place: Place, (w, h): (u32, u32)) -> Reserve {
    Reserve { place, w, h }
}

/// The two blocks, page rounded, KiB: 800x600 4 + 1 bytes a pixel is
/// 1920000 and 2400000 bytes, 1600x1200 four times that.
#[test]
fn the_scratch_blocks_in_kib() {
    let kib = |b: u32| b.div_ceil(4096) * 4;
    let w = at(Place::Top, WINDOW);
    assert_eq!((kib(w.colour()), kib(w.zs())), (1876, 2344));
    let b = at(Place::Top, BIG);
    assert_eq!((kib(b.colour()), kib(b.zs())), (7500, 9376));
}

/// The reservation is a plain GENERAL `alloc` at init: both blocks are far
/// over `pool_top_above`, so the pool puts them in the top region, at the
/// carveout's tail, mapped above `TOP_VA`; the bump and the frame sets do
/// not move for it. Taken from the bump instead, the bump's end moves up by
/// the two blocks (4220 KiB) and the top region does not grow.
#[test]
fn a_general_reservation_lands_in_the_top_region() {
    for (win, kib) in [(WINDOW, 4220), (BIG, 16876)] {
        let base = glmark2_on(256 << 20, Chains::Pot);
        let top = glmark2_with(256 << 20, Chains::Pot, Some(at(Place::Top, win)));
        let bump = glmark2_with(256 << 20, Chains::Pot, Some(at(Place::Bump, win)));
        let va = |what| top.placed.iter().find(|p| p.1 == what).map(|p| p.2 - GENERAL_HEAP_BASE).unwrap();
        assert!(va("PR colour scratch") >= u64::from(crate::gpu::pvr::mem::TOP_VA));
        assert!(va("PR depth/stencil scratch") >= u64::from(crate::gpu::pvr::mem::TOP_VA));
        assert_eq!(top.end_bump, base.end_bump, "the bump does not move for a top reservation");
        assert_eq!(top.top_hw, base.top_hw + kib, "{win:?}: the top region grows by the two blocks");
        assert_eq!(bump.end_bump, base.end_bump + kib, "{win:?}: the bump grows by the two blocks");
        assert_eq!(bump.top_hw, base.top_hw);
        assert!(top.persistent.abs_diff(base.persistent + kib) <= 1);
    }
}

/// The 256 MiB carve-out of the glmark2 image: both sizes fit, in either
/// place, and nothing is refused or lost. The headroom (the pool less the
/// highest scene high-water, refract's 143576 KiB with the window's
/// blocks, against 139356 without) is 116452 KiB without the scratch, then
/// 112232 at 800x600 and 99576 at 1600x1200. Top moves the 73 large
/// blocks (the top region's: the window's depth buffer, the caustic chains,
/// terrain's vertex buffers and targets, shadow's and refract's) down by
/// the reservation; the bump takes all 90 small ones up.
#[test]
fn the_scratch_fits_the_256_mib_carveout() {
    let base = glmark2_on(256 << 20, Chains::Pot);
    let hw = [21, 22, 23, 24].iter().map(|&n| base.live_hw_of(n)).max().unwrap();
    assert_eq!(i64::from(base.pool) - i64::from(hw), 116452);
    for (win, headroom) in [(WINDOW, 112232), (BIG, 99576)] {
        for (place, moved) in [(Place::Top, 73), (Place::Bump, 90)] {
            let c = cost(256 << 20, at(place, win));
            assert!(c.reserved, "{win:?} {place:?}");
            assert_eq!(c.headroom, headroom, "{win:?} {place:?}");
            assert_eq!(c.refused, [], "{win:?} {place:?}");
            assert_eq!(c.lost, [], "{win:?} {place:?}");
            assert_eq!(moved_n(&c), moved, "{win:?} {place:?}");
        }
    }
}

/// The board's 64 MiB carve-out at the window's 800x600: the two blocks
/// are served at init (4220 KiB of a 59200 KiB pool), and the headroom of
/// the base replay, 2544 KiB, is gone: the run's highest scene
/// high-water, terrain's, leaves 2100 KiB. Terrain's fourth 800x600
/// target and its depth buffer are no longer served, and refract's
/// 1600x1216 depth buffer (7600 KiB) is refused at the first draw, the
/// block the render needs to draw at all; the three POT chains stay
/// refused as before. Top and bump refuse exactly the same.
#[test]
fn the_scratch_at_800x600_starves_the_64_mib_board() {
    let base = glmark2_on(MIB_64, Chains::Pot);
    let hw = [21, 22, 23, 24].iter().map(|&n| base.live_hw_of(n)).max().unwrap();
    assert_eq!(i64::from(base.pool) - i64::from(hw), 2544);
    for (place, moved) in [(Place::Top, 67), (Place::Bump, 90)] {
        let c = cost(MIB_64, at(place, WINDOW));
        assert!(c.reserved, "{place:?}");
        assert_eq!(c.headroom, 2100, "{place:?}");
        assert_eq!(
            c.refused,
            [
                (22, "800x600 target", 1875, 1),
                (22, "800x608 depth buffer", 1900, 1),
                (22, CHAIN_800.0, 5462, 2),
                (24, "1600x1216 depth buffer", 7600, 1),
                (24, CHAIN_1600.0, 21846, 1),
            ],
            "{place:?}"
        );
        assert_eq!(
            c.lost,
            [(22, "800x600 target", 1), (22, "800x608 depth buffer", 1), (24, "1600x1216 depth buffer", 1)],
            "{place:?}"
        );
        assert_eq!(moved_n(&c), moved, "{place:?}");
    }
}

/// The same at 1600x1200 (16876 KiB, over a quarter of the pool): both
/// blocks still fit at init, since nothing else is placed yet, but the
/// reservation takes the room of terrain's vertex indices, two of its
/// 512x512 chains, its four 800x600 targets and depth buffer, 22 of
/// shadow's 24 unnamed blocks and refract's depth buffer: the headroom is
/// 388 KiB.
#[test]
fn the_scratch_at_1600x1200_starves_the_64_mib_board_further() {
    for (place, moved) in [(Place::Top, 61), (Place::Bump, 69)] {
        let c = cost(MIB_64, at(place, BIG));
        assert!(c.reserved, "{place:?}");
        assert_eq!(c.headroom, 388, "{place:?}");
        assert_eq!(
            c.refused,
            [
                (22, "terrain indices", 3072, 1),
                (22, "512x512 mip chain", 1366, 2),
                (22, "800x600 target", 1875, 4),
                (22, "800x608 depth buffer", 1900, 1),
                (22, CHAIN_800.0, 5462, 2),
                (23, "a driver block", 64, 1),
                (23, "a PM block", 4, 1),
                (23, "unnamed", 256, 22),
                (24, "1600x1216 depth buffer", 7600, 1),
                (24, CHAIN_1600.0, 21846, 1),
            ],
            "{place:?}"
        );
        assert_eq!(
            c.lost,
            [
                (22, "terrain indices", 1),
                (22, "512x512 mip chain", 2),
                (22, "800x600 target", 4),
                (22, "800x608 depth buffer", 1),
                (LIFE, "unnamed", 22),
                (24, "1600x1216 depth buffer", 1),
            ],
            "{place:?}"
        );
        assert_eq!(moved_n(&c), moved, "{place:?}");
    }
}
