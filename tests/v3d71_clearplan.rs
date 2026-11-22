// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 glClear plan (src/v3d/clearplan71.rs) against Mesa's 7.1
//! references under tools/v3d/pi5/ (read as text), and every refusal with
//! synthetic input.
//!
//! Each reference shape's GL clears are the ones tools/v3d/pi5/src/gl_shapes.py
//! issues, written here as `ClearIn`s in order. What the `.clif` shows and the
//! test checks:
//!
//! * the draws (`VERTEX_ARRAY_PRIMS`) of the whole dump are the shape's own
//!   draws plus one per planned clear draw (Mesa's blitter clear is a draw);
//! * a shape with a planned tile clear has `CLEAR_RT` in the dump, and a
//!   multi-target clear has its `Number of Render Targets`;
//! * gl_scissor's scissored clear is a `Clip Window` of 32x32.
//!
//! `CLEAR_RT` is necessary, not sufficient, for a tile clear: Mesa also
//! emits it in jobs whose only clear is a draw (a job's depth/stencil buffer
//! is cleared on first use), so its absence is not asserted per job. A
//! zero-width scissor (gl_scisbox step 3) is a degenerate draw in Mesa and
//! `Nothing` in the plan: the quad would write no pixel.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_clearplan`

#![allow(dead_code)]

#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;

use clearplan71::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

const OFF: ScissorIn = ScissorIn { enabled: false, x: 0, y: 0, w: 0, h: 0 };
const NONE: TargetIn = TargetIn { cleared: false, mask: MASK_RGBA };
const ALL: TargetIn = TargetIn { cleared: true, mask: MASK_RGBA };

/// A 64x64 framebuffer with `n` colour targets, all cleared, full masks, no
/// depth or stencil, no scissor.
fn base(n: usize) -> ClearIn {
    let mut targets = [NONE; MAX_TARGETS];
    for t in targets.iter_mut().take(n) {
        *t = ALL;
    }
    ClearIn {
        fb_w: 64,
        fb_h: 64,
        n_targets: n,
        targets,
        has_depth: false,
        has_stencil: false,
        clear_depth: false,
        clear_stencil: false,
        depth_mask: true,
        stencil_mask: 0xff,
        scissor: OFF,
        ds_valid: false,
        draws_queued: false,
    }
}

/// gl_test's window: RGBA8 and D24S8.
fn window() -> ClearIn {
    ClearIn { has_depth: true, has_stencil: true, ..base(1) }
}

fn scissor(c: ClearIn, x: i32, y: i32, w: i32, h: i32) -> ClearIn {
    ClearIn { scissor: ScissorIn { enabled: true, x, y, w, h }, ..c }
}

fn mask(c: ClearIn, m: u8) -> ClearIn {
    let mut c = c;
    c.targets[0].mask = m;
    c
}

fn is_tile(p: Result<ClearPlan, Refusal>) -> bool {
    matches!(p, Ok(ClearPlan::TileClear { .. }))
}
fn is_draw(p: Result<ClearPlan, Refusal>) -> bool {
    matches!(p, Ok(ClearPlan::Draw(_)))
}

struct Clif {
    draws: usize,
    clear_rt: usize,
    rts: Vec<u32>,
    clips: Vec<(u32, u32)>,
}

fn clif(shape: &str) -> Clif {
    let t = read(&format!("tools/v3d/pi5/gl_{shape}.clif"));
    let mut c = Clif { draws: 0, clear_rt: 0, rts: vec![], clips: vec![] };
    let mut w = 0;
    for l in t.lines() {
        let l = l.strip_prefix("MESA: info: ").unwrap_or(l);
        if l == "VERTEX_ARRAY_PRIMS" {
            c.draws += 1;
        } else if l == "CLEAR_RT" {
            c.clear_rt += 1;
        } else if let Some(v) = l.trim().strip_prefix("/*       Number of Render Targets: */ ") {
            c.rts.push(v.trim().parse().unwrap());
        } else if let Some(v) = l.trim().strip_prefix("/*    Clip Window Width in pixels: */ ") {
            w = v.trim().parse().unwrap();
        } else if let Some(v) = l.trim().strip_prefix("/*   Clip Window Height in pixels: */ ") {
            c.clips.push((w, v.trim().parse().unwrap()));
        }
    }
    c
}

/// Check one shape: `plans` are the planned clears, `explicit` the shape's
/// own draws, `degenerate` the draws Mesa makes for a clear the plan drops.
fn check(shape: &str, plans: &[Result<ClearPlan, Refusal>], explicit: usize, degenerate: usize) -> Clif {
    let c = clif(shape);
    let clear_draws = plans.iter().filter(|p| matches!(p, Ok(ClearPlan::Draw(_)))).count();
    assert_eq!(c.draws, explicit + clear_draws + degenerate, "{shape}: draws in the dump");
    if plans.iter().any(|p| matches!(p, Ok(ClearPlan::TileClear { .. }))) {
        assert!(c.clear_rt > 0, "{shape}: a tile clear needs CLEAR_RT");
    }
    assert!(plans.iter().all(|p| p.is_ok()), "{shape}: a refusal");
    c
}

#[test]
fn gl_scissor() {
    let plans = [plan_clear(&window()), plan_clear(&scissor(window(), 0, 0, 32, 32))];
    assert!(is_tile(plans[0]) && is_draw(plans[1]));
    let c = check("scissor", &plans, 1, 0);
    assert!(c.clips.contains(&(32, 32)), "the scissored clear's clip window");
    let Ok(ClearPlan::Draw(d)) = plans[1] else { unreachable!() };
    assert_eq!(d.rect, Rect { x: 0, y: 0, w: 32, h: 32 });
    assert_eq!((d.targets, d.color_masks[0]), (1, MASK_RGBA));
}

#[test]
fn gl_scisbox() {
    let red = |w| scissor(window(), 16, 16, w, 32);
    let plans = [
        plan_clear(&window()),
        plan_clear(&red(32)),
        // step 1: the blue clear opens the job, the green quad follows, then red
        plan_clear(&window()),
        plan_clear(&ClearIn { draws_queued: true, ..red(32) }),
        // step 2: a zero-width box
        plan_clear(&window()),
        plan_clear(&red(0)),
    ];
    assert!(is_tile(plans[0]) && is_tile(plans[2]) && is_tile(plans[4]));
    assert!(is_draw(plans[1]) && is_draw(plans[3]));
    assert_eq!(plans[5], Ok(ClearPlan::Nothing));
    let Ok(ClearPlan::Draw(d)) = plans[1] else { unreachable!() };
    assert_eq!(d.rect, Rect { x: 16, y: 16, w: 32, h: 32 });
    // the green quad (1 draw), and Mesa's degenerate draw for the zero box (1)
    check("scisbox", &plans, 1, 1);
}

#[test]
fn gl_writemask() {
    // colour: the blue clear, then every channel masked off
    let blue = plan_clear(&window());
    let off = plan_clear(&mask(window(), 0));
    assert!(is_tile(blue));
    assert_eq!(off, Ok(ClearPlan::Nothing));
    // depth: colour|depth full, then a depth clear under glDepthMask(false)
    let cd = ClearIn { clear_depth: true, ..window() };
    let depth_on = plan_clear(&cd);
    let depth_off =
        plan_clear(&ClearIn { clear_depth: false, ds_valid: true, ..ClearIn { depth_mask: false, ..window() } });
    // only the depth asked, through a closed depth mask: nothing to write
    let depth_off_asked = plan_clear(&ClearIn {
        targets: [NONE; MAX_TARGETS],
        depth_mask: false,
        clear_depth: true,
        ds_valid: true,
        ..window()
    });
    assert!(is_tile(depth_on));
    assert_eq!(depth_off_asked, Ok(ClearPlan::Nothing));
    assert!(depth_off.is_ok());
    // stencil: colour|stencil under 0xff, then stencil alone under 0x0f
    let cs = ClearIn { clear_stencil: true, ..window() };
    let st_full = plan_clear(&cs);
    let st_part = plan_clear(&ClearIn {
        targets: [NONE; MAX_TARGETS],
        stencil_mask: 0x0f,
        ds_valid: true,
        draws_queued: false,
        ..cs
    });
    assert!(is_tile(st_full));
    let Ok(ClearPlan::Draw(d)) = st_part else { panic!("{st_part:?}") };
    assert!(d.stencil && !d.depth && d.targets == 0);
    assert_eq!(d.stencil_mask, 0x0f);
    assert_eq!(d.rect, Rect { x: 0, y: 0, w: 64, h: 64 });
    // 4 explicit draws (two depth, two stencil), one clear draw
    check("writemask", &[blue, off, depth_on, depth_off_asked, st_full, st_part], 4, 0);
}

#[test]
fn gl_colormask() {
    let plans = [
        plan_clear(&window()),
        // R and A only
        plan_clear(&mask(window(), 0b1001)),
        plan_clear(&window()),
        // scissored (0,0,32,32), G only
        plan_clear(&scissor(mask(window(), 0b0010), 0, 0, 32, 32)),
        plan_clear(&window()),
        // B only, fused into a draw in Mesa; a draw here
        plan_clear(&mask(window(), 0b0100)),
    ];
    assert!(is_tile(plans[0]) && is_tile(plans[2]) && is_tile(plans[4]));
    let Ok(ClearPlan::Draw(ra)) = plans[1] else { panic!() };
    assert_eq!(ra.color_masks[0], 0b1001);
    let Ok(ClearPlan::Draw(g)) = plans[3] else { panic!() };
    assert_eq!((g.color_masks[0], g.rect.w, g.rect.h), (0b0010, 32, 32));
    let Ok(ClearPlan::Draw(b)) = plans[5] else { panic!() };
    assert_eq!(b.color_masks[0], 0b0100);
    check("colormask", &plans, 1, 0);
}

fn mrt(shape: &str, n: usize) {
    let p = plan_clear(&base(n));
    assert_eq!(
        p,
        Ok(ClearPlan::TileClear { targets: ((1u16 << n) - 1) as u8, depth: false, stencil: false, new_job: false })
    );
    let c = check(shape, &[p], 0, 0);
    assert!(c.rts.contains(&(n as u32)), "{shape}: {n} render targets in the dump");
    // one target of the n masked: a draw writing the other n-1
    let mut m = base(n);
    m.targets[1].mask = 0;
    let Ok(ClearPlan::Draw(d)) = plan_clear(&m) else { panic!() };
    assert_eq!(d.targets.count_ones() as usize, n - 1);
    assert_eq!(d.targets & 2, 0);
    // one target not a draw buffer: the others still need a draw
    let mut m = base(n);
    m.targets[n - 1].cleared = false;
    assert!(is_draw(plan_clear(&m)));
}

#[test]
fn gl_mrt4() {
    mrt("mrt4", 4);
}

#[test]
fn gl_mrt8() {
    mrt("mrt8", 8);
}

#[test]
fn gl_fbods() {
    let col = |c: ClearIn| c;
    // frame 1: a colour-only FBO, depth asked for and not there
    let f1 = plan_clear(&col(ClearIn { clear_depth: true, ..base(1) }));
    // frame 2: stencil asked for and not there
    let f2 = plan_clear(&col(ClearIn { clear_stencil: true, ..base(1) }));
    // frame 3: RGB colour target
    let f3 = plan_clear(&base(1));
    // frames 4 and 5: depth16
    let f4 = plan_clear(&ClearIn { has_depth: true, clear_depth: true, ..base(1) });
    // frame 6: a STENCIL_INDEX8 FBO, colour|stencil, then a scissored stencil-only clear
    let s8 = ClearIn { has_stencil: true, clear_stencil: true, ..base(1) };
    let f6a = plan_clear(&s8);
    let s_only = ClearIn { targets: [NONE; MAX_TARGETS], ds_valid: true, ..scissor(s8, 16, 16, 32, 32) };
    let f6b = plan_clear(&s_only);
    // frames 7 and 8: the same with a draw ahead
    let f7a = plan_clear(&s8);
    let f7b = plan_clear(&ClearIn { draws_queued: true, ..s_only });
    for p in [f1, f2, f3, f4, f6a, f7a] {
        assert!(is_tile(p), "{p:?}");
    }
    let Ok(ClearPlan::TileClear { depth, stencil, .. }) = f1 else { panic!() };
    assert!(!depth && !stencil, "no depth buffer, no depth clear");
    let Ok(ClearPlan::TileClear { depth, stencil, .. }) = f6a else { panic!() };
    assert!(!depth && stencil);
    for p in [f6b, f7b] {
        let Ok(ClearPlan::Draw(d)) = p else { panic!("{p:?}") };
        assert!((d.stencil, d.depth, d.targets) == (true, false, 0));
        assert_eq!(d.rect, Rect { x: 16, y: 16, w: 32, h: 32 });
    }
    // draws of the dump: 2 + 1 + 1 + 1 + 1 + 2 + 2 explicit, three clear draws
    check("fbods", &[f1, f2, f3, f4, f6a, f6b, f7a, f7b, f7a, f7b], 10, 0);
}

#[test]
fn tile_clear_after_draws_opens_a_new_job() {
    let p = plan_clear(&ClearIn { draws_queued: true, ..base(1) });
    assert_eq!(p, Ok(ClearPlan::TileClear { targets: 1, depth: false, stencil: false, new_job: true }));
}

#[test]
fn partial_cover_is_a_draw() {
    // a box covering the framebuffer is no scissor; one that clips past the edge too
    assert!(is_tile(plan_clear(&scissor(base(1), 0, 0, 64, 64))));
    assert!(is_tile(plan_clear(&scissor(base(1), -5, -5, 500, 500))));
    let Ok(ClearPlan::Draw(d)) = plan_clear(&scissor(base(1), 48, 48, 100, 100)) else { panic!() };
    assert_eq!(d.rect, Rect { x: 48, y: 48, w: 16, h: 16 });
    // a box wholly outside writes nothing
    assert_eq!(plan_clear(&scissor(base(1), 64, 0, 8, 8)), Ok(ClearPlan::Nothing));
    // live depth the clear does not touch is kept: a draw
    let w = ClearIn { ds_valid: true, ..window() };
    assert!(is_draw(plan_clear(&w)));
    // dead depth is not
    assert!(is_tile(plan_clear(&window())));
    // a partial depth/stencil of a combined buffer
    let both = ClearIn { clear_depth: true, clear_stencil: true, stencil_mask: 0x0f, ..window() };
    assert!(is_draw(plan_clear(&both)));
    // nothing attached, nothing asked
    let none = ClearIn { targets: [NONE; MAX_TARGETS], n_targets: 0, ..base(0) };
    assert_eq!(plan_clear(&none), Ok(ClearPlan::Nothing));
}

#[test]
fn refusals() {
    let r = plan_clear(&ClearIn { n_targets: 9, ..base(8) }).unwrap_err();
    assert_eq!(r, Refusal::TooManyTargets { n: 9 });
    assert_eq!(r.to_string(), "clear: 9 colour targets, V3D 7.1 has 8");

    let r = plan_clear(&ClearIn { fb_w: 7681, ..base(1) }).unwrap_err();
    assert_eq!(r, Refusal::FramebufferSize { w: 7681, h: 64 });
    assert_eq!(r.to_string(), "clear: framebuffer 7681x64, V3D 7.1 is 1..=7680 a side");
    assert!(plan_clear(&ClearIn { fb_h: 0, ..base(1) }).is_err());
    assert!(plan_clear(&ClearIn { fb_w: 7680, fb_h: 7680, ..base(1) }).is_ok());

    let mut c = base(2);
    c.targets[3].cleared = true;
    let r = plan_clear(&c).unwrap_err();
    assert_eq!(r, Refusal::TargetOutOfRange { index: 3, n: 2 });
    assert_eq!(r.to_string(), "clear: colour target 3 is cleared but the framebuffer has 2");

    let r = plan_clear(&scissor(base(1), 0, 0, -1, 8)).unwrap_err();
    assert_eq!(r, Refusal::NegativeScissor { w: -1, h: 8 });
    assert_eq!(r.to_string(), "clear: scissor -1x8 has a negative side");
    // a disabled scissor's box is not looked at
    assert!(plan_clear(&ClearIn { scissor: ScissorIn { enabled: false, x: 0, y: 0, w: -3, h: -3 }, ..base(1) }).is_ok());
}
