// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 clear plans (src/v3d/clearplan71.rs) of the gl_test cases the
//! Raspberry Pi 5's first board run clears with: 58, 59, 66 and 102
//! (driver/tools/pvrlab/gl_test.c, `case_clearscis`, `case_clearmask`,
//! `case_clearvp`, `case_maskclear`), each `glClear` of the case written as
//! a `ClearIn` on gl_test's 64x64 RGBA8 + D24S8 window, in order.
//!
//! Also pinned: case 59's shared refusal string, and that case 67's
//! alpha-bits refusal (docs/notes/v3d71-gl-table.md) is gone from the shared
//! sources. The sources are read with `include_str!`, which the
//! tree-key scan skips because the compiler reads them.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_clear_cases`

#![allow(dead_code)]

#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;

use clearplan71::*;

const STATE_RS: &str = include_str!("../src/v3d/state.rs");
const BACKEND_RS: &str = include_str!("../src/v3d/backend.rs");

const OFF: ScissorIn = ScissorIn { enabled: false, x: 0, y: 0, w: 0, h: 0 };
const NONE: TargetIn = TargetIn { cleared: false, mask: MASK_RGBA };

/// gl_test's window: 64x64, one RGBA8 target, D24S8, no scissor, all masks
/// open, nothing drawn yet. `glClear(COLOR)` only; the caller sets the rest.
fn window() -> ClearIn {
    let mut targets = [NONE; MAX_TARGETS];
    targets[0] = TargetIn { cleared: true, mask: MASK_RGBA };
    ClearIn {
        fb_w: 64,
        fb_h: 64,
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

fn scissor(c: ClearIn, x: i32, y: i32, w: i32, h: i32) -> ClearIn {
    ClearIn { scissor: ScissorIn { enabled: true, x, y, w, h }, ..c }
}

fn cmask(c: ClearIn, m: u8) -> ClearIn {
    let mut c = c;
    c.targets[0].mask = m;
    c
}

fn plan(c: ClearIn) -> ClearPlan {
    plan_clear(&c).expect("the case's clear is planned")
}

fn draw(p: ClearPlan) -> ClearDraw {
    match p {
        ClearPlan::Draw(d) => d,
        other => panic!("want a clear draw, got {other:?}"),
    }
}

const FULL: Rect = Rect { x: 0, y: 0, w: 64, h: 64 };

fn is_tile(p: ClearPlan) -> bool {
    matches!(p, ClearPlan::TileClear { .. })
}

#[test]
fn case58_scissored_clear_is_a_quad_in_the_box() {
    // the whole target blue: a tile clear
    assert!(is_tile(plan(window())));
    // red inside a 32x32 box at (16,16): a draw of that box, colour only
    let d = draw(plan(scissor(window(), 16, 16, 32, 32)));
    assert_eq!(d.rect, Rect { x: 16, y: 16, w: 32, h: 32 });
    assert_eq!(d.targets, 1);
    assert_eq!(d.color_masks[0], MASK_RGBA);
    assert!(!d.depth && !d.stencil);
    // after a draw the full clear opens a new job; the box clear stays a draw
    let after = ClearIn { draws_queued: true, ..window() };
    assert_eq!(plan(after), ClearPlan::TileClear { targets: 1, depth: false, stencil: false, new_job: true });
    assert!(matches!(plan(scissor(after, 16, 16, 32, 32)), ClearPlan::Draw(_)));
    // an empty box (width 0) clears nothing
    assert_eq!(plan(scissor(window(), 16, 16, 0, 32)), ClearPlan::Nothing);
}

#[test]
fn case59_masks_decide_what_a_clear_writes() {
    // colour: every channel masked off writes nothing
    assert_eq!(plan(cmask(window(), 0)), ClearPlan::Nothing);
    // depth: glDepthMask(FALSE) with a depth-only clear writes nothing
    let depth_only =
        ClearIn { targets: [NONE; MAX_TARGETS], clear_depth: true, ds_valid: true, depth_mask: false, ..window() };
    assert_eq!(plan(depth_only), ClearPlan::Nothing);
    // stencil: clear to 0xff through mask 0x0f is a draw with that mask,
    // because a tile clear would write every bit and the depth is kept
    let stencil =
        ClearIn { targets: [NONE; MAX_TARGETS], clear_stencil: true, stencil_mask: 0x0f, ds_valid: true, ..window() };
    let d = draw(plan(stencil));
    assert!(d.stencil && !d.depth);
    assert_eq!(d.stencil_mask, 0x0f);
    assert_eq!(d.targets, 0);
    assert_eq!(d.rect, FULL);
}

#[test]
fn case59_refusal_text_is_still_the_shared_one() {
    // the draw that reads the stencil an earlier job wrote (case 59's last
    // step) is refused by the shared decision, on 4.2 and 7.1 alike
    assert!(BACKEND_RS.contains(
        "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in"
    ));
}

#[test]
fn case66_viewport_is_not_an_input() {
    // ClearIn has no viewport: the plan of a clear under a quarter viewport
    // is the plan of the full one, a tile clear of the whole target
    let p = plan(window());
    assert_eq!(p, ClearPlan::TileClear { targets: 1, depth: false, stencil: false, new_job: false });
    // step 3: the scissor does bound it, to the bottom-left quarter
    let d = draw(plan(scissor(window(), 0, 0, 32, 32)));
    assert_eq!(d.rect, Rect { x: 0, y: 0, w: 32, h: 32 });
}

#[test]
fn case102_partial_colour_masks() {
    // 1. full screen, R and A on: a draw of the whole target with R|A
    let d = draw(plan(cmask(window(), 0b1001)));
    assert_eq!(d.rect, FULL);
    assert_eq!(d.targets, 1);
    assert_eq!(d.color_masks[0], 0b1001);
    // 2. scissored to the bottom-left 32x32, G only
    let d = draw(plan(scissor(cmask(window(), 0b0010), 0, 0, 32, 32)));
    assert_eq!(d.rect, Rect { x: 0, y: 0, w: 32, h: 32 });
    assert_eq!(d.color_masks[0], 0b0010);
    // 3. B only, full screen
    let d = draw(plan(cmask(window(), 0b0100)));
    assert_eq!(d.rect, FULL);
    assert_eq!(d.color_masks[0], 0b0100);
    // the mask's bits above RGBA never reach the plan
    let d = draw(plan(cmask(window(), 0xf1)));
    assert_eq!(d.color_masks[0], 0b0001);
}

#[test]
fn case67_no_longer_carries_the_alpha_bits_refusal() {
    // docs/notes/v3d71-gl-table.md still lists "a colour format with fewer
    // than 8 alpha bits" as case 67's refusal; state.rs draws such formats
    // now (its test `a_format_with_fewer_alpha_bits_is_drawn_...`), so the
    // shared refusal is gone and the doc row is stale.
    assert!(STATE_RS.contains("fn a_format_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha"));
    assert!(!BACKEND_RS.contains("refuses a colour format with fewer than 8 alpha bits"));
}
