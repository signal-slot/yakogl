// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The V3D 7.1 depth/stencil decision between jobs (src/v3d/zskeep71.rs):
//!
//! * the GFXH-1461 answer at 7.1 and its Mesa citation;
//! * the full history table, row by row, written out here (not recomputed);
//! * the clear routing, with the one case where 7.1 differs from 4.2 (a whole
//!   clear of one plane while the job loads the other);
//! * the frame71 inputs of each layout, fed through `frame71::plan`;
//! * gl_test cases 59 and 67 (driver/tools/pvrlab/gl_test.c `case_clearmask`,
//!   `case_fbods`) as frame sequences, and case 67's jobs against the Mesa
//!   7.1 reference tools/v3d/pi5/gl_fbods.clif (its loads, stores, clear
//!   values and early clear, read as text);
//! * every refusal.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_zskeep`

#![allow(dead_code)]

#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

#[path = "../src/v3d/frame71.rs"]
mod frame71;

use frame71::{Attach71, DepthKind71, EndOp71, Frame71, Op71, Surface71, ZsAttach71};
use std::collections::HashMap;
use zskeep71::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("{p}: {e}"))
}

const ONE: u32 = 0x3f80_0000; // 1.0f32
const HALF: u32 = 0x3f00_0000; // 0.5f32

const D24S8_BOTH: Target71 = Target71 { layout: Layout71::Combined, depth: true, stencil: true };
const D16: Target71 = Target71 { layout: Layout71::Combined, depth: true, stencil: false };
const S_ONLY: Target71 = Target71 { layout: Layout71::Combined, depth: false, stencil: true };
const SPLIT: Target71 = Target71 { layout: Layout71::Split, depth: true, stencil: true };
const TILE: Target71 = Target71 { layout: Layout71::TileOnly, depth: true, stencil: true };
const COLOUR_ONLY: Target71 = Target71 { layout: Layout71::TileOnly, depth: false, stencil: false };

/* ---- GFXH-1461 ----------------------------------------------------------- */

#[test]
fn gfxh1461_is_42_only() {
    assert!(gfxh1461_applies(42));
    assert!(!gfxh1461_applies(71));
    assert_eq!(GFXH1461_MESA, "src/gallium/drivers/v3d/v3dx_draw.c:1676-1687");
    let src = read("src/v3d/zskeep71.rs");
    for cite in [
        "`src/gallium/drivers/v3d/v3dx_draw.c:1676-1687`",
        "`if (devinfo->ver == 42 && ...)` (`:1681`)",
        "This issue is fixed since V3D 4.3.18.",
        "`src/broadcom/vulkan/v3dv_pass.c:133-151`",
        "v3dx_rcl.c:325-341`",
    ] {
        assert!(src.contains(cite), "module docs lack {cite}");
    }
    // the repository's own erratum survey says the same (pre-study U9)
    let pre = read("docs/notes/pre-study-rpi5.md");
    assert!(pre.contains("GFXH-1461 split depth/stencil clear (`v3dx_draw.c:1676-1683`, \"fixed since V3D 4.3.18\")"));
    // and the 4.2 table is the one that works around it
    let b42 = read("src/v3d/backend.rs");
    assert!(b42.contains("TLB while loading the other: GFXH-1461"));
}

/* ---- the history table --------------------------------------------------- */

fn use_of(tlb: Option<u32>, quad: bool, read: bool, written: bool) -> PlaneUse71 {
    PlaneUse71 { tlb_clear: tlb, quad, read, written }
}

#[derive(Clone, Copy, Debug)]
enum Want {
    Out(Start71, bool, Known71),
    No(ZsRefusal71),
}
use Known71::*;
use Start71::{Clear as C, Load as L, Undefined as U};
use Want::*;

/// One plane (depth of a D16, or depth of a tile-only target); the stencil
/// plane is absent or untouched. Columns: layout, history, (tlb, quad, read,
/// written), discard, want.
#[allow(clippy::type_complexity)]
const TABLE: &[(Target71, Known71, (Option<u32>, bool, bool, bool), bool, Want)] = &[
    // a buffer: Untouched
    (D16, Untouched, (None, false, false, false), false, Out(U, false, Untouched)),
    (D16, Untouched, (None, false, true, false), false, Out(U, false, Untouched)),
    (D16, Untouched, (None, false, true, true), false, Out(U, true, Stored)),
    (D16, Untouched, (None, true, false, false), false, Out(U, true, Stored)),
    (D16, Untouched, (Some(HALF), false, false, false), false, Out(C(HALF), false, Cleared(HALF))),
    (D16, Untouched, (Some(HALF), false, true, true), false, Out(C(HALF), true, Stored)),
    (D16, Untouched, (Some(HALF), true, false, false), false, Out(C(HALF), true, Stored)),
    (D16, Untouched, (None, false, true, true), true, Out(U, false, Untouched)),
    // a buffer: Cleared
    (D16, Cleared(ONE), (None, false, false, false), false, Out(C(ONE), false, Cleared(ONE))),
    (D16, Cleared(ONE), (None, false, true, false), false, Out(C(ONE), false, Cleared(ONE))),
    (D16, Cleared(ONE), (None, false, true, true), false, Out(C(ONE), true, Stored)),
    (D16, Cleared(ONE), (None, true, false, false), false, Out(C(ONE), true, Stored)),
    (D16, Cleared(ONE), (Some(HALF), false, false, false), false, Out(C(HALF), false, Cleared(HALF))),
    (D16, Cleared(ONE), (Some(HALF), false, true, true), false, Out(C(HALF), true, Stored)),
    (D16, Cleared(ONE), (None, false, true, true), true, Out(C(ONE), false, Untouched)),
    // a buffer: Stored
    (D16, Stored, (None, false, false, false), false, Out(U, false, Stored)),
    (D16, Stored, (None, false, true, false), false, Out(L, false, Stored)),
    (D16, Stored, (None, false, true, true), false, Out(L, true, Stored)),
    (D16, Stored, (None, false, false, true), false, Out(L, true, Stored)),
    (D16, Stored, (None, true, false, false), false, Out(L, true, Stored)),
    (D16, Stored, (Some(HALF), false, false, false), false, Out(C(HALF), false, Cleared(HALF))),
    (D16, Stored, (Some(HALF), false, true, true), false, Out(C(HALF), true, Stored)),
    (D16, Stored, (None, false, true, true), true, Out(L, false, Untouched)),
    (D16, Stored, (None, false, false, false), true, Out(U, false, Untouched)),
    // a buffer: Written is not a history a buffer has
    (D16, Written, (None, false, false, false), false, No(ZsRefusal71::WrittenWithBuffer(Plane71::Depth))),
    // tile only: Untouched / Cleared
    (TILE, Untouched, (None, false, true, true), false, Out(U, false, Written)),
    (TILE, Untouched, (Some(ONE), false, false, false), false, Out(C(ONE), false, Cleared(ONE))),
    (TILE, Untouched, (Some(ONE), false, true, true), false, Out(C(ONE), false, Written)),
    (TILE, Cleared(ONE), (None, false, true, false), false, Out(C(ONE), false, Cleared(ONE))),
    (TILE, Cleared(ONE), (None, true, false, false), false, Out(C(ONE), false, Written)),
    (TILE, Cleared(ONE), (None, false, true, true), true, Out(C(ONE), false, Untouched)),
    // tile only: Written
    (TILE, Written, (None, false, false, false), false, Out(U, false, Written)),
    (TILE, Written, (None, false, false, true), false, Out(U, false, Written)),
    (TILE, Written, (None, true, false, false), false, Out(U, false, Written)),
    (TILE, Written, (None, false, true, false), false, No(ZsRefusal71::ReadLost(Plane71::Depth))),
    (TILE, Written, (Some(ONE), false, true, true), false, Out(C(ONE), false, Written)),
    (TILE, Written, (Some(ONE), false, false, false), false, Out(C(ONE), false, Cleared(ONE))),
    (TILE, Written, (None, false, false, false), true, Out(U, false, Untouched)),
    // tile only: Stored is not a history it has
    (TILE, Stored, (None, false, false, false), false, No(ZsRefusal71::StoredWithoutBuffer(Plane71::Depth))),
];

#[test]
fn history_table() {
    for (i, &(t, k, (tlb, quad, rd, wr), discard, want)) in TABLE.iter().enumerate() {
        let job = Job71 { z: use_of(tlb, quad, rd, wr), s: PlaneUse71::NONE, discard };
        let got = decide(&t, k, Untouched, &job);
        match want {
            Out(start, store, next) => {
                let d = got.unwrap_or_else(|e| panic!("row {i}: refused: {e}"));
                assert_eq!((d.z.start, d.z.store, d.z.next), (start, store, next), "row {i}");
                // the stencil plane, untouched, stays so
                assert_eq!((d.s.start, d.s.store), (U, false), "row {i}");
            }
            No(r) => assert_eq!(got, Err(r), "row {i}"),
        }
    }
    // the same rows on the stencil plane of a stencil-only buffer
    for (i, &(t, k, (tlb, quad, rd, wr), discard, want)) in TABLE.iter().enumerate() {
        let t = if t == D16 { S_ONLY } else { Target71 { depth: false, ..t } };
        let job = Job71 { z: PlaneUse71::NONE, s: use_of(tlb, quad, rd, wr), discard };
        let got = decide(&t, Untouched, k, &job);
        let flip = |p: Plane71| if p == Plane71::Depth { Plane71::Stencil } else { p };
        match want {
            Out(start, store, next) => {
                let d = got.unwrap_or_else(|e| panic!("stencil row {i}: refused: {e}"));
                assert_eq!((d.s.start, d.s.store, d.s.next), (start, store, next), "stencil row {i}");
            }
            No(ZsRefusal71::ReadLost(p)) => assert_eq!(got, Err(ZsRefusal71::ReadLost(flip(p)))),
            No(ZsRefusal71::WrittenWithBuffer(p)) => assert_eq!(got, Err(ZsRefusal71::WrittenWithBuffer(flip(p)))),
            No(ZsRefusal71::StoredWithoutBuffer(p)) => {
                assert_eq!(got, Err(ZsRefusal71::StoredWithoutBuffer(flip(p))))
            }
            No(r) => panic!("row {i}: {r:?}"),
        }
    }
}

#[test]
fn planes_are_independent_at_71() {
    // D24S8: depth stored, a whole stencil clear, a draw testing both. 4.2
    // draws the stencil clear as a quad and loads Z+S (GFXH-1461); 7.1
    // TLB-clears stencil and loads depth alone.
    let r = route_clear(&D24S8_BOTH, false, false, ClearKind71::None, ClearKind71::Full(7)).unwrap();
    assert_eq!(r, ClearRoute71 { new_job: false, z: Route71::None, s: Route71::Tlb(7) });
    let mut job = Job71::NONE;
    job.s.apply(r.s);
    job.z.draw(true, false);
    job.s.draw(true, false);
    let d = decide(&D24S8_BOTH, Stored, Stored, &job).unwrap();
    assert_eq!((d.z.start, d.z.store, d.z.next), (L, false, Stored));
    assert_eq!((d.s.start, d.s.store, d.s.next), (C(7), false, Cleared(7)));
    let f = d.frame();
    assert!(f.zs_load_z && !f.zs_load_s && !f.zs_store_z && !f.zs_store_s);
    assert_eq!(f.stencil_clear, 7);
    let p = frame71::plan(&frame(&f, D24S8_BOTH, DepthKind71::D24, 43)).unwrap();
    assert_eq!(zs_ops(&p.list_loads), vec![(frame71::BUFFER_Z, 43)]);
    assert_eq!(zs_ops(&p.list_stores), vec![]);
    // the next job draws depth only: stencil starts from its clear again
    let mut job = Job71::NONE;
    job.z.draw(true, true);
    let d = decide(&D24S8_BOTH, d.z.next, d.s.next, &job).unwrap();
    assert_eq!((d.z.start, d.z.store), (L, true));
    assert_eq!((d.s.start, d.s.store, d.s.next), (C(7), false, Cleared(7)));
    // both used: one Z+Stencil load
    let mut job = Job71::NONE;
    job.z.draw(true, true);
    job.s.draw(true, true);
    let d = decide(&D24S8_BOTH, Stored, Stored, &job).unwrap();
    let p = frame71::plan(&frame(&d.frame(), D24S8_BOTH, DepthKind71::D24, 43)).unwrap();
    assert_eq!(zs_ops(&p.list_loads), vec![(frame71::BUFFER_ZSTENCIL, 43)]);
    assert_eq!(zs_ops(&p.list_stores), vec![(frame71::BUFFER_ZSTENCIL, 43)]);
}

#[test]
fn clear_routing() {
    use ClearKind71::*;
    // whole clears go to the TLB; with draws queued the job ends first
    let r = route_clear(&D24S8_BOTH, false, true, Full(ONE), Full(0)).unwrap();
    assert_eq!(r, ClearRoute71 { new_job: false, z: Route71::Tlb(ONE), s: Route71::Tlb(0) });
    let r = route_clear(&D24S8_BOTH, true, false, Full(ONE), None).unwrap();
    assert!(r.new_job);
    let r = route_clear(&D24S8_BOTH, true, true, None, None).unwrap();
    assert!(r.new_job, "a colour TLB clear after draws ends the job too");
    // partial clears are quads, drawn into the open job
    let r = route_clear(&D24S8_BOTH, true, false, Partial, Partial).unwrap();
    assert_eq!(r, ClearRoute71 { new_job: false, z: Route71::Quad, s: Route71::Quad });
    // one whole, one partial: the whole one is not demoted (4.2 would)
    let r = route_clear(&D24S8_BOTH, false, false, Full(ONE), Partial).unwrap();
    assert_eq!((r.z, r.s), (Route71::Tlb(ONE), Route71::Quad));
    // a plane the framebuffer lacks is not cleared (ES 2.0 4.2.3)
    let r = route_clear(&D16, false, false, Full(ONE), Full(1)).unwrap();
    assert_eq!((r.z, r.s), (Route71::Tlb(ONE), Route71::None));
    let r = route_clear(&COLOUR_ONLY, true, false, Full(ONE), Partial).unwrap();
    assert_eq!(r, ClearRoute71 { new_job: false, z: Route71::None, s: Route71::None });
    // a TLB clear after a quad in one use is a later job's
    let mut u = PlaneUse71::NONE;
    u.apply(Route71::Quad);
    assert!(u.needs() && u.tlb_clear.is_none());
}

/* ---- the frame71 inputs -------------------------------------------------- */

fn surface(format: u8) -> Surface71 {
    Surface71 { format, memory_format: 4, height_in_ub_or_stride: 8, address: 0x1_0000, ..Surface71::default() }
}

/// A 64x64, 64 px tile frame with RT 0 stored and the decision's
/// depth/stencil inputs, the way backend71 is to fill it.
fn frame(f: &FrameZs71, t: Target71, kind: DepthKind71, format: u8) -> Frame71 {
    let mut fr = Frame71::new(64, 64, 64, 64);
    fr.rt[0] = Some(Attach71 { surface: surface(27), load: false, end: EndOp71::Store });
    fr.depth_kind = if f.zs_surface { kind } else { DepthKind71::None };
    if f.zs_surface {
        fr.zs = Some(ZsAttach71 {
            surface: surface(format),
            load_z: f.zs_load_z,
            load_s: f.zs_load_s,
            store_z: f.zs_store_z,
            store_s: f.zs_store_s,
        });
    }
    if f.stencil_surface {
        fr.stencil = Some(Attach71 {
            surface: surface(frame71::FORMAT_S8),
            load: f.stencil_load,
            end: if f.stencil_store { EndOp71::Store } else { EndOp71::Invalidate },
        });
    }
    let _ = t;
    fr.early_ds_clear = f.early_ds_clear;
    fr.z_clear_bits = f.z_clear_bits;
    fr.stencil_clear = f.stencil_clear;
    fr.clear_tile_buffers = true;
    fr
}

/// The depth/stencil loads or stores of a tile list: (buffer, format).
fn zs_ops(ops: &[Op71]) -> Vec<(u8, u8)> {
    ops.iter()
        .filter_map(|o| match o {
            Op71::Load(t) | Op71::Store(t) if t.buffer >= frame71::BUFFER_Z => Some((t.buffer, t.surface.format)),
            _ => None,
        })
        .collect()
}

#[test]
fn frame_inputs_per_layout() {
    let mut job = Job71::NONE;
    job.z.draw(true, true);
    job.s.draw(true, true);
    // split: the depth surface carries Z, the separate one stencil
    let d = decide(&SPLIT, Stored, Stored, &job).unwrap();
    let f = d.frame();
    assert_eq!(
        f,
        FrameZs71 {
            zs_surface: true,
            zs_load_z: true,
            zs_store_z: true,
            stencil_surface: true,
            stencil_load: true,
            stencil_store: true,
            z_clear_bits: UNDEFINED_Z_BITS,
            ..FrameZs71::default()
        }
    );
    let p = frame71::plan(&frame(&f, SPLIT, DepthKind71::D16, 42)).unwrap();
    assert_eq!(zs_ops(&p.list_loads), vec![(frame71::BUFFER_STENCIL, frame71::FORMAT_S8), (frame71::BUFFER_Z, 42)]);
    assert_eq!(zs_ops(&p.list_stores), vec![(frame71::BUFFER_Z, 42), (frame71::BUFFER_STENCIL, frame71::FORMAT_S8)]);
    // split, stencil only written: the depth surface is neither loaded nor stored
    let mut job = Job71::NONE;
    job.s.apply(Route71::Tlb(3));
    job.s.draw(true, true);
    let d = decide(&SPLIT, Stored, Untouched, &job).unwrap();
    let f = d.frame();
    assert!(!f.zs_load_z && !f.zs_store_z && !f.stencil_load && f.stencil_store);
    let p = frame71::plan(&frame(&f, SPLIT, DepthKind71::D16, 42)).unwrap();
    assert_eq!(zs_ops(&p.list_loads), vec![]);
    assert_eq!(zs_ops(&p.list_stores), vec![(frame71::BUFFER_STENCIL, frame71::FORMAT_S8)]);
    // tile only: no surface, an early clear of what the TLB clears
    let mut job = Job71::NONE;
    job.z.apply(Route71::Tlb(ONE));
    job.z.draw(true, true);
    let d = decide(&TILE, Untouched, Untouched, &job).unwrap();
    let f = d.frame();
    assert!(!f.zs_surface && !f.stencil_surface && f.early_ds_clear && f.clear_tile_buffers);
    assert_eq!(f.z_clear_bits, ONE);
    let p = frame71::plan(&frame(&f, TILE, DepthKind71::D24, 43)).unwrap();
    assert!(p.common.early_ds_clear);
    assert_eq!(zs_ops(&p.list_loads), vec![]);
    // nothing used, nothing cleared: no early clear, no tile clear asked
    let d = decide(&D24S8_BOTH, Stored, Stored, &Job71::NONE).unwrap();
    let f = d.frame();
    assert!(!f.early_ds_clear && !f.clear_tile_buffers && !f.zs_load_z && !f.zs_load_s);
}

/* ---- gl_test as frame sequences ------------------------------------------ */

/// A GL context's view of the jobs: the open job, the histories per
/// depth/stencil buffer, and every decision in order.
struct Sim {
    name: &'static str,
    t: Target71,
    hist: HashMap<&'static str, (Known71, Known71)>,
    job: Option<(Job71, bool)>,
    log: Vec<(&'static str, Target71, Decision71)>,
}

impl Sim {
    fn new(name: &'static str, t: Target71) -> Sim {
        Sim { name, t, hist: HashMap::new(), job: None, log: Vec::new() }
    }

    fn bind(&mut self, name: &'static str, t: Target71) -> Result<(), ZsRefusal71> {
        self.finish()?;
        self.name = name;
        self.t = t;
        Ok(())
    }

    fn clear(&mut self, colour: bool, z: ClearKind71, s: ClearKind71) -> Result<(), ZsRefusal71> {
        let draws = self.job.is_some_and(|j| j.1);
        let r = route_clear(&self.t, draws, colour, z, s)?;
        if r.new_job {
            self.finish()?;
        }
        let j = self.job.get_or_insert((Job71::NONE, false));
        j.0.z.apply(r.z);
        j.0.s.apply(r.s);
        if r.z == Route71::Quad || r.s == Route71::Quad {
            j.1 = true;
        }
        Ok(())
    }

    /// A draw: (reads, writes) of depth and of stencil, after the GL core
    /// (a test on a plane the framebuffer lacks is off).
    fn draw(&mut self, z: (bool, bool), s: (bool, bool)) {
        let j = self.job.get_or_insert((Job71::NONE, false));
        j.0.z.draw(z.0, z.1);
        j.0.s.draw(s.0, s.1);
        j.1 = true;
    }

    /// glFinish (and glReadPixels after it): the open job ends.
    fn finish(&mut self) -> Result<(), ZsRefusal71> {
        if let Some((j, _)) = self.job.take() {
            let (z, s) = self.hist.get(self.name).copied().unwrap_or((Untouched, Untouched));
            let d = decide(&self.t, z, s, &j)?;
            self.hist.insert(self.name, (d.z.next, d.s.next));
            self.log.push((self.name, self.t, d));
        }
        Ok(())
    }
}

const NO: (bool, bool) = (false, false);
const TEST: (bool, bool) = (true, false);
const TEST_WRITE: (bool, bool) = (true, true);

/// case_clearmask (gl_test.c:8153-8250) on the window's framebuffer.
/// The context, and the refusal that stopped it, if any.
fn case59(t: Target71) -> (Sim, Option<ZsRefusal71>) {
    use ClearKind71::*;
    let mut g = Sim::new("window", t);
    let run = |g: &mut Sim| -> Result<(), ZsRefusal71> {
        // colour: a clear, then one with every channel masked (the core drops it)
        g.clear(true, None, None)?;
        g.finish()?;
        // depth: glClear(COLOR | DEPTH) to 1.0; red at window z 0.5, LESS,
        // writes; glDepthMask(FALSE) + glClear(DEPTH) asks nothing; green
        // behind, LESS, writes; glFinish
        g.clear(true, Full(ONE), None)?;
        g.draw(TEST_WRITE, NO);
        g.clear(false, None, None)?;
        g.draw(TEST_WRITE, NO);
        g.finish()?;
        // stencil: glClear(COLOR | STENCIL) to 0 under mask 0xff; 0xff under
        // mask 0x0f (partial: the quad); a draw EQUAL 0xff, KEEP; glFinish
        g.clear(true, None, Full(0))?;
        g.clear(false, None, Partial)?;
        g.draw(NO, TEST);
        g.finish()?;
        // then EQUAL 0x0f: a new job reads the stencil the last one wrote
        g.draw(NO, TEST);
        g.finish()
    };
    let r = run(&mut g).err();
    (g, r)
}

#[test]
fn case59_with_the_windows_d24s8() {
    // the window surface keeps a D24S8 (src/gl/draw.rs resolve_ds_storage:
    // ds_persist for the default framebuffer)
    let (g, e) = case59(D24S8_BOTH);
    assert_eq!(e, Option::None);
    let d: Vec<Decision71> = g.log.iter().map(|l| l.2).collect();
    assert_eq!(d.len(), 4);
    // colour only: depth and stencil untouched
    assert_eq!((d[0].z.start, d[0].z.store, d[0].s.store), (U, false, false));
    // depth: TLB 1.0, stored as Z alone
    assert_eq!((d[1].z.start, d[1].z.store, d[1].z.next), (C(ONE), true, Stored));
    assert_eq!((d[1].s.start, d[1].s.store), (U, false));
    let f = d[1].frame();
    assert!(!f.zs_load_z && !f.zs_load_s && f.zs_store_z && !f.zs_store_s);
    // stencil: TLB 0, the quad, stored as stencil alone; depth not loaded
    assert_eq!((d[2].s.start, d[2].s.store, d[2].s.next), (C(0), true, Stored));
    assert_eq!((d[2].z.start, d[2].z.store, d[2].z.next), (U, false, Stored));
    let f = d[2].frame();
    assert!(!f.zs_load_z && !f.zs_load_s && !f.zs_store_z && f.zs_store_s);
    // the read-back draw loads the stencil the last job stored
    assert_eq!((d[3].s.start, d[3].s.store, d[3].s.next), (L, false, Stored));
    let f = d[3].frame();
    assert!(!f.zs_load_z && f.zs_load_s && !f.zs_store_s);
    let p = frame71::plan(&frame(&f, D24S8_BOTH, DepthKind71::D24, 43)).unwrap();
    assert_eq!(zs_ops(&p.list_loads), vec![(frame71::BUFFER_STENCIL, 43)]);
}

#[test]
fn case59_tile_only_is_the_42_refusal() {
    // with no buffer to keep them in (the 4.2 table's first run, Pi 4
    // bring-up), the last draw is refused with the 4.2 string
    let (g, e) = case59(TILE);
    let e = e.expect("refused");
    assert_eq!(e, ZsRefusal71::ReadLost(Plane71::Stencil));
    assert_eq!(g.log.len(), 3);
    assert!(read("src/v3d/backend.rs").contains(&format!("\"{e}\"")));
    assert!(read("src/v3d/backend.rs").contains(&format!("\"{}\"", ZsRefusal71::ReadLost(Plane71::Depth))));
}

/// case_fbods (gl_test.c:8675-8956) frames 1 to 8, as the Mesa reference
/// gl_shapes.py `fbods` runs them: one job per frame, frame 8 two.
fn case67() -> Sim {
    use ClearKind71::*;
    let mut g = Sim::new("fbo_a", COLOUR_ONLY);
    // frame 1: colour only; glClear(COLOR | DEPTH) clears no depth, the
    // depth test is off for want of a buffer (ES 2.0 4.1.6)
    g.clear(true, Full(ONE), None).unwrap();
    g.draw(NO, NO);
    g.draw(NO, NO);
    g.finish().unwrap();
    // frame 2: the stencil test on, no stencil buffer (4.1.4)
    g.clear(true, None, Full(0)).unwrap();
    g.draw(NO, NO);
    g.finish().unwrap();
    // frame 3: a GL_RGB target, a colour clear
    g.bind("rgb_fbo", COLOUR_ONLY).unwrap();
    g.clear(true, None, None).unwrap();
    g.finish().unwrap();
    // frame 4: fbo_a with depth_a (DEPTH_COMPONENT16): clear to 1.0, red
    // near, LESS, writes
    g.bind("depth_a", D16).unwrap();
    g.clear(true, Full(ONE), None).unwrap();
    g.draw(TEST_WRITE, NO);
    g.finish().unwrap();
    // frame 5: fbo_b with depth_b, never cleared: green far, LESS, writes
    g.bind("depth_b", D16).unwrap();
    g.draw(TEST_WRITE, NO);
    g.finish().unwrap();
    // frame 6: fbo_a, depth detached, STENCIL_INDEX8 (Mesa keeps it as
    // D24S8): clear colour and stencil 0, a scissored stencil clear to 1
    // (the quad), red EQUAL 0, KEEP
    g.bind("sten", S_ONLY).unwrap();
    g.clear(true, None, Full(0)).unwrap();
    g.clear(false, None, Partial).unwrap();
    g.draw(NO, TEST);
    g.finish().unwrap();
    // frames 7 and 8: the same with green ahead, in one job and across a
    // glFinish
    for step in 0..2 {
        g.clear(true, None, Full(0)).unwrap();
        g.draw(NO, NO);
        g.clear(false, None, Partial).unwrap();
        if step == 1 {
            g.finish().unwrap();
        }
        g.draw(NO, TEST);
        g.finish().unwrap();
    }
    g
}

#[test]
fn case67_frames() {
    let g = case67();
    let d: Vec<(&str, Decision71)> = g.log.iter().map(|l| (l.0, l.2)).collect();
    assert_eq!(d.len(), 9);
    for (_, x) in &d[0..3] {
        assert_eq!(x.frame(), FrameZs71::default(), "colour-only frames carry no depth/stencil");
    }
    // frame 4: depth_a cleared and written: stored
    assert_eq!(d[3].0, "depth_a");
    assert_eq!((d[3].1.z.start, d[3].1.z.store, d[3].1.z.next), (C(ONE), true, Stored));
    // frame 5: depth_b undefined (starts at the far plane), written, stored
    assert_eq!(d[4].0, "depth_b");
    assert_eq!((d[4].1.z.start, d[4].1.z.store, d[4].1.z.next), (U, true, Stored));
    assert_eq!(d[4].1.frame().z_clear_bits, UNDEFINED_Z_BITS);
    assert!(d[4].1.clear_tile_buffers);
    // depth_a keeps its own history
    assert_eq!(g.hist["depth_a"], (Stored, Untouched));
    // frames 6, 7, 8a: stencil cleared + quad: stored
    for (n, x) in d[5..8].iter() {
        assert_eq!(*n, "sten");
        assert_eq!((x.s.start, x.s.store, x.s.next), (C(0), true, Stored));
    }
    // frame 8b: the stencil-testing draw after the kick loads it
    assert_eq!((d[8].1.s.start, d[8].1.s.store, d[8].1.s.next), (L, false, Stored));
    // an absent plane the core did not mask is refused
    let mut job = Job71::NONE;
    job.z.draw(true, false);
    assert_eq!(decide(&COLOUR_ONLY, Untouched, Untouched, &job), Err(ZsRefusal71::AbsentPlane(Plane71::Depth)));
}

/* ---- case 67 against Mesa's 7.1 reference -------------------------------- */

#[derive(Debug, Default)]
struct ClifJob {
    /// depth/stencil loads and stores: (buffer, format)
    loads: Vec<(u8, u8)>,
    stores: Vec<(u8, u8)>,
    z_clear: f32,
    stencil_clear: u8,
    early: u8,
    depth_type: u8,
}

fn value(line: &str) -> &str {
    line.split("*/").nth(1).unwrap().split_whitespace().next().unwrap()
}

/// The jobs of a CLIF dump, one per `@createbuf_aligned` run.
fn clif_jobs(text: &str) -> Vec<ClifJob> {
    let mut jobs: Vec<ClifJob> = Vec::new();
    let mut prev_create = false;
    let mut pending: Option<(bool, u8)> = None; // (load?, buffer)
    for line in text.lines() {
        let create = line.contains("@createbuf_aligned");
        if create && !prev_create {
            jobs.push(ClifJob::default());
        }
        prev_create = create;
        let Some(j) = jobs.last_mut() else { continue };
        if line.contains("Buffer to Load:") {
            pending = Some((true, value(line).parse().unwrap()));
        } else if line.contains("Buffer to Store:") {
            pending = Some((false, value(line).parse().unwrap()));
        } else if line.contains("Input Image Format:") || line.contains("Output Image Format:") {
            let (load, buffer) = pending.take().expect("a format after its buffer");
            let f: u8 = value(line).parse().unwrap();
            if buffer >= frame71::BUFFER_Z {
                if load { &mut j.loads } else { &mut j.stores }.push((buffer, f));
            }
        } else if line.contains("Z Clear Value:") {
            j.z_clear = value(line).parse().unwrap();
        } else if line.contains("Stencil Clear Value:") {
            j.stencil_clear = value(line).parse().unwrap();
        } else if line.contains("Early Depth/Stencil Clear:") {
            j.early = value(line).parse().unwrap();
        } else if line.contains("Internal Depth Type:") {
            j.depth_type = value(line).parse().unwrap();
        }
    }
    jobs
}

#[test]
fn case67_matches_gl_fbods_clif() {
    let jobs = clif_jobs(&read("tools/v3d/pi5/gl_fbods.clif"));
    assert_eq!(jobs.len(), 9, "gl_fbods.clif: one job per frame, two for frame 8");
    // the reference's own shape: D16 Z stored in 4 and 5, stencil of a
    // D24S8 stored in 6 to 8 and loaded in 9 (read as text)
    assert_eq!(jobs[3].stores, vec![(9, 42)]);
    assert_eq!(jobs[8].loads, vec![(10, 43)]);
    let g = case67();
    for (i, ((name, t, d), m)) in g.log.iter().zip(&jobs).enumerate() {
        let (kind, format) = match *name {
            "depth_a" | "depth_b" => (DepthKind71::D16, 42),
            "sten" => (DepthKind71::StencilOnly, frame71::FORMAT_D24S8),
            _ => (DepthKind71::None, 0),
        };
        assert_eq!(kind.internal_type(), m.depth_type, "job {}: depth type", i + 1);
        let f = d.frame();
        let p = frame71::plan(&frame(&f, *t, kind, format)).unwrap_or_else(|e| panic!("job {}: {e:?}", i + 1));
        assert_eq!(zs_ops(&p.list_loads), m.loads, "job {}: loads", i + 1);
        assert_eq!(zs_ops(&p.list_stores), m.stores, "job {}: stores", i + 1);
        assert_eq!(p.common.early_ds_clear as u8, m.early, "job {}: early clear", i + 1);
        assert_eq!(p.zs_clear.stencil, m.stencil_clear, "job {}: stencil clear", i + 1);
        if i == 4 {
            // depth_b is undefined: Mesa clears it to 0.0 (its job's
            // default), this table to the far plane as the 4.2 table does
            assert_eq!(m.z_clear, 0.0);
            assert_eq!(f32::from_bits(p.zs_clear.z_bits), 1.0);
        } else {
            assert_eq!(f32::from_bits(p.zs_clear.z_bits), m.z_clear, "job {}: Z clear", i + 1);
        }
    }
}

/* ---- refusals ------------------------------------------------------------ */

#[test]
fn every_refusal() {
    use ClearKind71::*;
    let bad_split = Target71 { layout: Layout71::Split, depth: true, stencil: false };
    let empty = Target71 { layout: Layout71::Combined, depth: false, stencil: false };
    let mut rd_s = Job71::NONE;
    rd_s.s.draw(true, false);
    let mut clr_z = Job71::NONE;
    clr_z.z.apply(Route71::Tlb(ONE));
    let cases: Vec<(Result<(), ZsRefusal71>, ZsRefusal71)> = vec![
        (decide(&TILE, Untouched, Written, &rd_s).map(|_| ()), ZsRefusal71::ReadLost(Plane71::Stencil)),
        (decide(&TILE, Written, Untouched, &rd_s.swap()).map(|_| ()), ZsRefusal71::ReadLost(Plane71::Depth)),
        (decide(&D16, Untouched, Untouched, &rd_s).map(|_| ()), ZsRefusal71::AbsentPlane(Plane71::Stencil)),
        (decide(&S_ONLY, Untouched, Untouched, &clr_z).map(|_| ()), ZsRefusal71::AbsentPlane(Plane71::Depth)),
        (decide(&D16, Untouched, Stored, &Job71::NONE).map(|_| ()), ZsRefusal71::AbsentPlane(Plane71::Stencil)),
        (decide(&bad_split, Untouched, Untouched, &Job71::NONE).map(|_| ()), ZsRefusal71::SplitNeedsBoth),
        (route_clear(&bad_split, false, false, None, None).map(|_| ()), ZsRefusal71::SplitNeedsBoth),
        (decide(&empty, Untouched, Untouched, &Job71::NONE).map(|_| ()), ZsRefusal71::EmptyBuffer),
        (route_clear(&empty, false, true, Full(ONE), None).map(|_| ()), ZsRefusal71::EmptyBuffer),
        (decide(&TILE, Stored, Untouched, &Job71::NONE).map(|_| ()), ZsRefusal71::StoredWithoutBuffer(Plane71::Depth)),
        (
            decide(&TILE, Untouched, Stored, &Job71::NONE).map(|_| ()),
            ZsRefusal71::StoredWithoutBuffer(Plane71::Stencil),
        ),
        (
            decide(&D24S8_BOTH, Written, Untouched, &Job71::NONE).map(|_| ()),
            ZsRefusal71::WrittenWithBuffer(Plane71::Depth),
        ),
        (
            decide(&SPLIT, Untouched, Written, &Job71::NONE).map(|_| ()),
            ZsRefusal71::WrittenWithBuffer(Plane71::Stencil),
        ),
    ];
    let mut messages = std::collections::HashSet::new();
    for (i, (got, want)) in cases.into_iter().enumerate() {
        assert_eq!(got, Err(want), "refusal {i}");
        assert_eq!(want.to_string(), want.message());
        assert!(!want.message().is_empty());
        messages.insert(want.message());
    }
    // ten variants (with their plane), ten distinct messages
    assert_eq!(messages.len(), 10);
    for m in &messages {
        assert!(m.starts_with("V3D 7.1: ") || m.starts_with("the draw reads "), "{m}");
    }
}

trait Swap {
    fn swap(self) -> Self;
}

impl Swap for Job71 {
    fn swap(self) -> Job71 {
        Job71 { z: self.s, s: self.z, discard: self.discard }
    }
}
