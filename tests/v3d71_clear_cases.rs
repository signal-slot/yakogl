// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 clear plans (src/v3d/clearplan71.rs, src/v3d/zskeep71.rs) fed with
//! the clears of the four gl_test cases of the Raspberry Pi 5's P9 set that
//! are clears (driver/tools/pvrlab/gl_test.c):
//!
//! * case 58 `case_clearscis`, "glClear honours the scissor box" (:8087);
//! * case 59 `case_clearmask`, "glClear honours the write masks" (:8171);
//! * case 66 `case_clearvp`, "glClear is not bounded by the viewport" (:9215);
//! * case 102 `case_maskclear`, "a colour clear under a partial glColorMask"
//!   (:23096).
//!
//! Each test writes the clears of one case as `ClearIn`s in gl_test.c's
//! order (the target is the window: 64x64 RGBA8 with D24S8, `TW`/`TH` at
//! gl_test.c:1088) and pins the decision: a full TLB clear (`TileClear`), a
//! clear drawn as a quad (`Draw`, with its rectangle and channels), nothing,
//! or a named refusal. The comment on each assertion names the gl_test.c line
//! of the clear and the readback the decision has to deliver.
//!
//! The 4.2 back end's decision for the same clear is stated in a comment
//! where it is documented (src/v3d/state.rs `colour_needs_quad`,
//! src/gl/draw.rs `plan_clear`, src/v3d/backend.rs `draw_clear_quad`); the
//! last test reads those sources as text, so a change there shows here.
//!
//! No decision here fails its case's readback, so no test is `#[ignore]`d.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_clear_cases`

#![allow(dead_code)]

#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;

#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use clearplan71::*;
use zskeep71::{ClearKind71, Job71, Known71, Layout71, Plane71, Route71, Start71, Target71, ZsRefusal71};

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// gl_test.c:1088, `TW` and `TH`.
const SIDE: u32 = 64;
const R: u8 = 1;
const G: u8 = 2;
const B: u8 = 4;
const A: u8 = 8;
const OFF: ScissorIn = ScissorIn { enabled: false, x: 0, y: 0, w: 0, h: 0 };
const FULL: Rect = Rect { x: 0, y: 0, w: SIDE, h: SIDE };

/// The window's glClear(GL_COLOR_BUFFER_BIT): RGBA8 + D24S8, one target, the
/// colour mask `mask`, no depth or stencil in the clear, no scissor, no draw
/// queued, no depth/stencil content a clear must keep.
fn colour_clear(mask: u8) -> ClearIn {
    let mut targets = [TargetIn { cleared: false, mask: MASK_RGBA }; MAX_TARGETS];
    targets[0] = TargetIn { cleared: true, mask };
    ClearIn {
        fb_w: SIDE,
        fb_h: SIDE,
        n_targets: 1,
        targets,
        has_depth: true,
        has_stencil: true,
        clear_depth: false,
        clear_stencil: false,
        depth_mask: true,
        stencil_mask: 0xff,
        scissor: OFF,
        ds_valid: false,
        draws_queued: false,
    }
}

fn boxed(c: ClearIn, x: i32, y: i32, w: i32, h: i32) -> ClearIn {
    ClearIn { scissor: ScissorIn { enabled: true, x, y, w, h }, ..c }
}

const TILE_COLOUR: ClearPlan = ClearPlan::TileClear { targets: 1, depth: false, stencil: false, new_job: false };

fn quad(mask: u8, rect: Rect) -> ClearPlan {
    let mut color_masks = [0u8; MAX_TARGETS];
    color_masks[0] = mask;
    ClearPlan::Draw(ClearDraw { targets: 1, color_masks, depth: false, stencil: false, stencil_mask: 0, rect })
}

// ---- case 58 -------------------------------------------------------------

#[test]
fn case58_scissored_clear_is_a_quad_in_the_box() {
    // gl_test.c:8122 glClear(COLOR) blue, no scissor, nothing queued: a whole
    // clear of the one target, so the TLB does it. Readback: blue everywhere
    // (the "left of the box" etc. pixels, 0xffff0000 in ABGR at :8118-8122).
    assert_eq!(plan_clear(&colour_clear(MASK_RGBA)), Ok(TILE_COLOUR));
    // gl_test.c:8124-8126 glScissor(16,16,32,32) then glClear(COLOR) red: a
    // quad of the box, all four channels. Readback: red 0xff0000ff at (32,32)
    // and (17,17) (:8133-8134), blue outside (:8135-8138). The rectangle is in
    // GL's bottom-left origin and the planner keeps it so: the clip window
    // is the one of the draw (the case's 32x32 box, as the Mesa reference
    // gl_scissor shows).
    let r = Rect { x: 16, y: 16, w: 32, h: 32 };
    assert_eq!(plan_clear(&boxed(colour_clear(MASK_RGBA), 16, 16, 32, 32)), Ok(quad(MASK_RGBA, r)));
}

#[test]
fn case58_clear_between_draws_stays_in_sequence() {
    // gl_test.c:8142 glClear(COLOR) blue after the previous step's glFinish:
    // no draw queued, a TLB clear. Then :8143 glDrawArrays queues a draw.
    assert_eq!(plan_clear(&colour_clear(MASK_RGBA)), Ok(TILE_COLOUR));
    // gl_test.c:8145-8147, the scissored red clear with the green draw queued:
    // still a quad (the scissor makes it one), never a TLB clear, so it lands
    // AFTER the draw. Readback: red at (32,32) (:8142), the draw's green
    // elsewhere (:8143). The quad does not need a new job (`draws_queued`
    // only matters for a TLB clear).
    let c = ClearIn { draws_queued: true, ..boxed(colour_clear(MASK_RGBA), 16, 16, 32, 32) };
    assert_eq!(plan_clear(&c), Ok(quad(MASK_RGBA, Rect { x: 16, y: 16, w: 32, h: 32 })));
    // A whole clear with the draw queued would end the job first: the plan
    // says so (the case does not do it; it pins `new_job`, the 4.2 rule).
    let c = ClearIn { draws_queued: true, ..colour_clear(MASK_RGBA) };
    assert_eq!(plan_clear(&c), Ok(ClearPlan::TileClear { targets: 1, depth: false, stencil: false, new_job: true }));
}

#[test]
fn case58_empty_box_clears_nothing() {
    // gl_test.c:8161-8163 glScissor(16,16,0,32) then a red clear: a box with
    // no width writes no pixel. Readback: blue stays at (32,32), :8167.
    assert_eq!(plan_clear(&boxed(colour_clear(MASK_RGBA), 16, 16, 0, 32)), Ok(ClearPlan::Nothing));
    // a zero height likewise
    assert_eq!(plan_clear(&boxed(colour_clear(MASK_RGBA), 16, 16, 32, 0)), Ok(ClearPlan::Nothing));
}

// ---- case 59 -------------------------------------------------------------

#[test]
fn case59_colour_half_a_fully_masked_clear_does_nothing() {
    // gl_test.c:8195-8197 glColorMask(FALSE x4) then glClear(COLOR) red: no
    // channel is writable. Readback: blue stays, 0xffff0000 at (32,32)
    // (:8203). The GL core hands the planner a target with mask 0.
    assert_eq!(plan_clear(&colour_clear(0)), Ok(ClearPlan::Nothing));
    // the clear before it (:8194), colour only, is the TLB's
    assert_eq!(plan_clear(&colour_clear(MASK_RGBA)), Ok(TILE_COLOUR));
}

/// A glClear of depth and/or stencil of the window; `colour` adds the
/// colour buffer.
fn zs_clear(colour: bool, depth: bool, stencil: bool, stencil_mask: u8, depth_mask: bool) -> ClearIn {
    let mut c = colour_clear(MASK_RGBA);
    c.targets[0].cleared = colour;
    ClearIn { clear_depth: depth, clear_stencil: stencil, stencil_mask, depth_mask, ..c }
}

#[test]
fn case59_depth_half_plans() {
    // gl_test.c:8212 glClear(COLOR | DEPTH) to 1.0 under depth mask TRUE:
    // everything cleared whole, the TLB's, depth only (stencil not asked,
    // nothing in it to keep).
    assert_eq!(
        plan_clear(&zs_clear(true, true, false, 0xff, true)),
        Ok(ClearPlan::TileClear { targets: 1, depth: true, stencil: false, new_job: false })
    );
    // gl_test.c:8219-8221 glDepthMask(FALSE), glClear(DEPTH): nothing to
    // write. Readback: red still owns the depth, the green quad behind loses,
    // 0xff0000ff at (32,32) (:8233).
    assert_eq!(plan_clear(&zs_clear(false, true, false, 0xff, false)), Ok(ClearPlan::Nothing));
}

#[test]
fn case59_stencil_half_plans() {
    // gl_test.c:8239-8241 glClear(COLOR | STENCIL) to 0x00 under stencil
    // mask 0xff, with the previous step's depth already kept by the buffer
    // (`ds_valid` false: the job has not written or loaded it): the TLB
    // clears colour and stencil.
    assert_eq!(
        plan_clear(&zs_clear(true, false, true, 0xff, true)),
        Ok(ClearPlan::TileClear { targets: 1, depth: false, stencil: true, new_job: false })
    );
    // gl_test.c:8242-8244 glStencilMask(0x0f), glClear(STENCIL) to 0xff: only
    // the low four bits may change, so a quad that replaces stencil through
    // the mask, colour untouched (the target is not cleared). Readback: the
    // stencil is 0x0f, a draw EQUAL 0xff does not draw (red stays, :8257) and
    // one EQUAL 0x0f does (green, :8259).
    let mut masks = [0u8; MAX_TARGETS];
    masks[0] = 0;
    assert_eq!(
        plan_clear(&zs_clear(false, false, true, 0x0f, true)),
        Ok(ClearPlan::Draw(ClearDraw {
            targets: 0,
            color_masks: masks,
            depth: false,
            stencil: true,
            stencil_mask: 0x0f,
            rect: FULL,
        }))
    );
    // a stencil mask of 0 asks for nothing
    assert_eq!(plan_clear(&zs_clear(false, false, true, 0, true)), Ok(ClearPlan::Nothing));
    // zskeep71 routes the same two stencil clears: whole -> the TLB, partial
    // -> the quad, and the depth plane is left alone (GFXH-1461 does not
    // apply at 7.1, so neither is a quad for the other plane's sake).
    let t = Target71 { layout: Layout71::Combined, depth: true, stencil: true };
    let r = zskeep71::route_clear(&t, false, true, ClearKind71::None, ClearKind71::Full(0)).unwrap();
    assert_eq!((r.z, r.s, r.new_job), (Route71::None, Route71::Tlb(0), false));
    let r = zskeep71::route_clear(&t, false, false, ClearKind71::None, ClearKind71::Partial).unwrap();
    assert_eq!((r.z, r.s, r.new_job), (Route71::None, Route71::Quad, false));
}

#[test]
fn case59_depth_stencil_half_is_refused_by_name_without_a_buffer() {
    // The refusal docs/notes/v3d71-gl-table.md names: with a tile-only target
    // (no buffer to keep depth/stencil between jobs) the read-back draw of
    // the stencil half (gl_test.c:8258-8259, `glStencilFunc(EQUAL, 0x0f)`
    // after the glFinish at :8253) reads stencil an earlier job wrote, and the
    // plan says ReadLost. With the window's D24S8 (src/gl/draw.rs
    // resolve_ds_storage keeps it) the same sequence loads it and passes:
    // tests/v3d71_zskeep.rs case59_with_the_windows_d24s8 walks that.
    let tile = Target71 { layout: Layout71::TileOnly, depth: true, stencil: true };
    let kept = Target71 { layout: Layout71::Combined, depth: true, stencil: true };
    for (t, want) in [(tile, false), (kept, true)] {
        // job 1: the stencil TLB-cleared to 0 and the partial clear's quad,
        // then a test draw (:8238-8254), depth untouched
        let mut j1 = Job71::NONE;
        j1.s.apply(Route71::Tlb(0));
        j1.s.apply(Route71::Quad);
        j1.s.draw(true, false);
        let d1 = zskeep71::decide(&t, Known71::Untouched, Known71::Untouched, &j1).unwrap();
        assert_eq!(d1.s.start, Start71::Clear(0));
        // job 2: the second test draw reads the stencil job 1 wrote
        let mut j2 = Job71::NONE;
        j2.s.draw(true, false);
        let r = zskeep71::decide(&t, d1.z.next, d1.s.next, &j2);
        if want {
            let d2 = r.unwrap();
            assert_eq!(d2.s.start, Start71::Load);
        } else {
            let e = r.unwrap_err();
            assert_eq!(e, ZsRefusal71::ReadLost(Plane71::Stencil));
            // 4.2 says the same, word for word
            assert!(read("src/v3d/backend.rs").contains(&format!("\"{e}\"")));
        }
    }
    // the depth half has no such refusal here: the depth is written in one
    // job (:8211-8228, both quads in one job before the glFinish) and read
    // back as colour, never loaded by a later job.
}

// ---- case 66 -------------------------------------------------------------

#[test]
fn case66_the_viewport_is_not_an_input_of_a_clear() {
    // gl_test.c:9255 glViewport(0,0,TW,TH), glClear(COLOR) white; the plan
    // takes no viewport, so the quarter viewport of step 2 (:9264-9266,
    // glViewport(0,0,TW/2,TH/2)) cannot bound it. ClearIn has no viewport
    // field: this struct literal (every field) fails to compile if one is added
    // without this test being told.
    let ClearIn {
        fb_w: _,
        fb_h: _,
        n_targets: _,
        targets: _,
        has_depth: _,
        has_stencil: _,
        clear_depth: _,
        clear_stencil: _,
        depth_mask: _,
        stencil_mask: _,
        scissor: _,
        ds_valid: _,
        draws_queued: _,
    } = colour_clear(MASK_RGBA);
    // step 1 (:9255) and step 2 (:9264-9266): the same plan, a whole TLB clear of
    // the whole 64x64 surface. Readback: blue at (63,63), (63,16), (16,63) and
    // (32,0), i.e. outside the quarter viewport too (:9272-9279).
    assert_eq!(plan_clear(&colour_clear(MASK_RGBA)), Ok(TILE_COLOUR));
}

#[test]
fn case66_scissor_still_bounds_the_clear() {
    // gl_test.c:9283-9287 glViewport(0,0,TW,TH), glScissor(0,0,TW/2,TH/2),
    // glClear(COLOR) green: a quad of the box, with GL's bottom-left origin.
    // Readback: green inside (the picture's bottom-left quarter, :9295), blue
    // outside at (63,16) (:9296).
    let r = Rect { x: 0, y: 0, w: SIDE / 2, h: SIDE / 2 };
    assert_eq!(plan_clear(&boxed(colour_clear(MASK_RGBA), 0, 0, 32, 32)), Ok(quad(MASK_RGBA, r)));
    // a scissor that covers the whole surface is the whole clear (not what the
    // case does; it shows the quarter box is what makes it a quad)
    assert_eq!(plan_clear(&boxed(colour_clear(MASK_RGBA), 0, 0, 64, 64)), Ok(TILE_COLOUR));
}

// ---- case 102 ------------------------------------------------------------

#[test]
fn case102_partial_masks_are_quads_with_their_channels() {
    // 1. gl_test.c:23158-23166. The black fill under the full mask (:23160) is
    // the TLB's; R+A (:23164) is a quad of the whole target writing R and A.
    // Readback over every pixel: (1,0,0,0.5) = 0x800000ff (:23170).
    assert_eq!(plan_clear(&colour_clear(MASK_RGBA)), Ok(TILE_COLOUR));
    assert_eq!(plan_clear(&colour_clear(R | A)), Ok(quad(R | A, FULL)));

    // 2. :23174-23183. Black again (TLB), then the scissor 0,0,32,32 with G
    // only: a quad of the box, G. Readback: green 0xff00ff00 in the box (the
    // top-down rows 32..63 of the left half), black 0xff000000 elsewhere,
    // tolerance 0 (:23188).
    let r = Rect { x: 0, y: 0, w: 32, h: 32 };
    assert_eq!(plan_clear(&boxed(colour_clear(G), 0, 0, 32, 32)), Ok(quad(G, r)));

    // 3. :23200-23205. Black (TLB), then B only over the whole target, then a
    // draw (yellow, the bottom-left quadrant) with the mask back at all: the
    // clear keeps its own mask, a quad of the whole target writing B. The
    // quad is first in the job, the draw after it: the order is the case's.
    // Readback: yellow 0xff00ffff in the quadrant, blue 0xffff0000 elsewhere
    // (:23211).
    assert_eq!(plan_clear(&colour_clear(B)), Ok(quad(B, FULL)));
}

#[test]
fn case102_every_partial_mask_is_never_a_tile_clear() {
    // The TLB clear writes every channel, so any mask short of RGBA must be a
    // quad (the switch the case measures: with a partial mask written as four
    // channels every shape fails). All fifteen non-empty masks, full and
    // scissored; the empty one is nothing.
    for m in 1..MASK_RGBA {
        match plan_clear(&colour_clear(m)).unwrap() {
            ClearPlan::Draw(d) => {
                assert_eq!((d.targets, d.color_masks[0], d.rect), (1, m, FULL), "mask {m:#x}");
                assert!(!d.depth && !d.stencil);
            }
            p => panic!("mask {m:#x}: {p:?}"),
        }
    }
    assert_eq!(plan_clear(&colour_clear(0)), Ok(ClearPlan::Nothing));
}

// ---- the 4.2 back end ----------------------------------------------------

#[test]
fn the_42_back_end_decides_the_same_way() {
    // Colour: a partial mask or a scissor is a quad, otherwise the TLB's
    // (src/v3d/state.rs colour_needs_quad: `d.clear_quad || d.clear_color_mask
    // & 0xf != 0xf`). 7.1 matches for cases 58, 66 and 102.
    let st = read("src/v3d/state.rs");
    assert!(st.contains("d.do_clear && (d.clear_quad || d.clear_color_mask & 0xf != 0xf)"));
    // The quad is drawn into the whole target whatever the application's
    // viewport (src/v3d/state.rs clear_quad_draw), so case 66's quarter
    // viewport does not bound it: 7.1's planner has no viewport input, and
    // the draw that carries it must set the viewport to the target too.
    assert!(st.contains("q.viewport = [0, 0, d.target_w as i32, d.target_h as i32];"));
    // Case 59's refusals are the 4.2 strings (checked above); case 58's empty
    // box: 4.2 plan_clear in src/gl/draw.rs returns None for a clear that
    // changes nothing, and 7.1 says Nothing.
    assert!(read("src/gl/draw.rs").contains("or `None`"));
    // One difference, not exercised by these cases: 4.2 marks every scissored
    // clear `clear_quad`, even a box that covers the surface; 7.1's planner
    // sends a covering box to the TLB (tested in case 66 above).
}
