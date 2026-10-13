// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D `glClear` on the host: the TLB/quad decision, the clear quad's state
//! and the scissor geometry, against what dEQP-GLES2 checks
//! (the investigation es2-v3d-clear.md, section 5.1; removed from
//! tools/cts/log by 358ca5324, read it at 358ca5324^).
//!
//! The V3D clears a whole tile buffer, every channel, at the start of a job
//! (the RCL's clear colour, depth and stencil). Every other clear (a scissor
//! box, a colour mask with channels left out, a partial stencil mask) is an
//! ordinary draw of the clear program over the whole target, which keeps the
//! clear's scissor and masks: the clear quad (`v3d::state::clear_quad_draw`).
//!
//! The host has no V3D rasteriser and no tile buffer, so these tests check
//! decisions, state and geometry, not pixels. The last test,
//! `cts_color_clear_reference`, puts the pieces together in a small software
//! model of the two clear paths and compares it with dEQP's own reference
//! for all 19 `functional.color_clear.*` cases. It shows that the decisions
//! and the state give the CTS image *if* the hardware does what the model
//! says; the board (dEQP on the Pi 4) checks that second half.
//!
//! The GL core's `plan_clear` is private, so the clears here are built the
//! way it builds them (`src/gl/draw.rs` plan_clear / apply_clear_plan); its
//! own tests are in that file.

// The V3D back end exists only in a gpu-v3d build; `host-test` is the
// configuration tools/check.sh runs the V3D rows in.
#![cfg(all(feature = "gpu-v3d", feature = "host-test"))]

use yakogl::backend::*;
use yakogl::shaderc::{ShcShader, SHC_BF_ONE};
use yakogl::v3d::ffi::{V3dJobState, V3D_COMPARE_FUNC_ALWAYS, V3D_STENCIL_OP_REPLACE};
use yakogl::v3d::state::{self, ClearProg, QuadBuffers};

/* ---- shared helpers ----------------------------------------------------------- */

/// A small xorshift32. The CTS seeds `de::Random` with `deInt32Hash(iter)`;
/// the doc allows any seed, so the sequence is not dEQP's, only its ranges.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9) | 1)
    }

    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// A colour target of `w` x `h` RGBA8 pixels; `flip_y` is a window (first
/// row in memory is the top of the picture), not an FBO.
fn target(w: u32, h: u32, flip_y: bool) -> GlesDraw {
    GlesDraw {
        target_gpu: 0x0100_0000,
        target_w: w,
        target_h: h,
        target_format: GLES_SURF_RGBA8888,
        target_alpha_bits: 8,
        samples: 1,
        num_targets: 1,
        flip_y,
        ..GlesDraw::ZERO
    }
}

/// One scissor box from dEQP's color_clear generator
/// (es2fColorClearTest.cpp `iterate`): x, y in [-W, W), w in [0, W),
/// h in [0, H). So the box may start outside the target, be empty (w or h
/// 0), or reach past the target's far edge.
fn cts_box(rng: &mut Rng, w: u32, h: u32) -> [i32; 4] {
    let x = (rng.next() % (2 * w)) as i32 - w as i32;
    let y = (rng.next() % (2 * h)) as i32 - h as i32;
    [x, y, (rng.next() % w) as i32, (rng.next() % h) as i32]
}

/// Is GL pixel (x, y) (y counted from the bottom) inside `box_`? dEQP's
/// `de::inBounds(v, lo, hi)` is `lo <= v < hi`.
fn in_box(box_: [i32; 4], x: i32, y: i32) -> bool {
    let [bx, by, bw, bh] = box_;
    bx <= x && x < bx + bw && by <= y && y < by + bh
}

/// The row in memory of GL row `y`.
fn mem_row(y: i32, h: u32, flip_y: bool) -> i32 {
    if flip_y {
        h as i32 - 1 - y
    } else {
        y
    }
}

/// Is memory pixel (col, row) inside the scissor of a job state?
fn in_job_scissor(st: &V3dJobState, col: i32, row: i32) -> bool {
    !st.scissor
        || (st.scissor_minx as i32 <= col
            && col < st.scissor_maxx as i32
            && st.scissor_miny as i32 <= row
            && row < st.scissor_maxy as i32)
}

/* ---- 1. the scissor box ------------------------------------------------------- */

/// `state::scissor` covers exactly the pixels of the CTS rectangle (the
/// box intersected with the target), in memory rows, for a window (y
/// flipped) and for an FBO. The clear quad covers the whole target and
/// relies on this box for its extent.
///
/// An empty intersection is all zero before the flip, as st_update_scissor
/// makes it (st_atom_scissor.c:55-95, mesa-26.2.1); a window then flips the
/// y pair, so it reads [0, H, 0, H]. Both cover no pixel.
#[test]
fn scissor_is_the_cts_rectangle() {
    const W: u32 = 37;
    const H: u32 = 23;
    const BOXES: u32 = 3000;
    let mut rng = Rng::new(0x5c15_5012);
    let mut extra = vec![[0, 0, 0, 0], [-5, -3, 0, 7], [3, 4, 5, 0], [-40, -30, 36, 22], [0, 0, 36, 22]];
    extra.extend([[10, 5, 30, 20], [-3, -2, 8, 6], [W as i32 - 1, H as i32 - 1, 36, 22], [W as i32, 0, 5, 5]]);
    for flip_y in [false, true] {
        let mut empty = 0;
        let boxes = (0..BOXES).map(|_| cts_box(&mut rng, W, H)).collect::<Vec<_>>();
        for &box_ in extra.iter().chain(boxes.iter()) {
            let d = GlesDraw { scissor: true, scissor_rect: box_, ..target(W, H, flip_y) };
            let [minx, miny, maxx, maxy] = state::scissor(&d).expect("the scissor test is on");
            assert!(minx <= maxx && maxx <= W && miny <= maxy && maxy <= H, "{box_:?} -> {minx},{miny},{maxx},{maxy}");
            let mut covered = 0;
            for y in 0..H as i32 {
                for x in 0..W as i32 {
                    let row = mem_row(y, H, flip_y) as u32;
                    let ours = minx <= x as u32 && (x as u32) < maxx && miny <= row && row < maxy;
                    assert_eq!(ours, in_box(box_, x, y), "flip_y {flip_y}, box {box_:?}, pixel ({x}, {y})");
                    covered += u32::from(ours);
                }
            }
            if covered == 0 {
                empty += 1;
                let want = if flip_y { [0, H, 0, H] } else { [0, 0, 0, 0] };
                assert_eq!([minx, miny, maxx, maxy], want, "flip_y {flip_y}, empty box {box_:?}");
            }
        }
        /* the generator does produce empty boxes (about a quarter of them) */
        assert!(empty > BOXES / 10, "flip_y {flip_y}: only {empty} empty boxes");
    }
    /* the scissor test off is no box at all */
    assert_eq!(state::scissor(&GlesDraw { scissor_rect: [1, 2, 3, 4], ..target(W, H, true) }), None);
}

/* ---- 2. TLB or quad ----------------------------------------------------------- */

/// The colour of a clear goes to the quad if and only if the TLB cannot
/// do it: the GL core marked it `clear_quad` (a scissor box, a partial
/// stencil mask), or its colour mask leaves a channel out. A full-mask,
/// unscissored colour clear is the TLB's; no colour clear is no quad.
#[test]
fn colour_needs_quad_is_the_tlb_decision() {
    let mut tlb = 0;
    for do_clear in [false, true] {
        for clear_quad in [false, true] {
            for mask in 0u8..=15 {
                let d = GlesDraw { do_clear, clear_quad, clear_color_mask: mask, ..target(64, 48, true) };
                let want = do_clear && (clear_quad || mask != 0xf);
                assert_eq!(
                    state::colour_needs_quad(&d),
                    want,
                    "do_clear {do_clear}, clear_quad {clear_quad}, mask {mask:#x}"
                );
                tlb += u32::from(do_clear && !want);
            }
        }
    }
    assert_eq!(tlb, 1, "the TLB takes exactly one row: colour, no clear_quad, mask 0xf");
    /* only the low four bits are channels */
    let d = GlesDraw { do_clear: true, clear_color_mask: 0xff, ..target(64, 48, true) };
    assert!(!state::colour_needs_quad(&d));
}

/* ---- 3. the quad's state ------------------------------------------------------ */

/// An application's draw with every state the clear quad must NOT inherit:
/// blend, cull, polygon offset, LESS depth with no write, a stencil that
/// keeps, a colour mask, its own scissor, its own viewport and depth range.
fn busy_draw(w: u32, h: u32, flip_y: bool) -> GlesDraw {
    GlesDraw {
        index_count: 3,
        viewport: [5, 6, 7, 8],
        depth_range: [0.25, 0.5],
        blend: true,
        blend_src_rgb: SHC_BF_ONE as u8,
        blend_dst_rgb: SHC_BF_ONE as u8,
        blend_src_alpha: SHC_BF_ONE as u8,
        blend_dst_alpha: SHC_BF_ONE as u8,
        cull: GLES_CULL_CW,
        polygon_offset: true,
        polygon_offset_factor: 2.0,
        polygon_offset_units: 3.0,
        depth_test: true,
        depth_func: GLES_CMP_LESS,
        depth_write: false,
        stencil: true,
        stencil_func: GLES_CMP_EQUAL,
        stencil_back_func: GLES_CMP_NEVER,
        stencil_ref: 9,
        stencil_back_ref: 9,
        stencil_cmp_mask: 0x0f,
        stencil_back_cmp_mask: 0x0f,
        stencil_write_mask: 0xfe,
        stencil_back_write_mask: 0x7f,
        stencil_sfail: GLES_SOP_KEEP,
        stencil_dpfail: GLES_SOP_INCR_SAT,
        stencil_dppass: GLES_SOP_INVERT,
        stencil_back_sfail: GLES_SOP_ZERO,
        stencil_back_dpfail: GLES_SOP_KEEP,
        stencil_back_dppass: GLES_SOP_DECR_WRAP,
        color_write: true,
        color_mask: 0x9,
        scissor: true,
        scissor_rect: [1, 1, 2, 2],
        ..target(w, h, flip_y)
    }
}

/// `clear_quad_draw` over the 8 colour/depth/stencil combinations, with
/// the clear's scissor and without, through `job_state`: the colour write
/// mask is the clear's or 0; depth is ALWAYS and writes only when depth is
/// cleared; stencil is ALWAYS + REPLACE on all three outcomes with the
/// clear's reference and write mask, the same on both faces; no blend, no
/// cull, no offset; the viewport the whole target at depth range (0, 1);
/// no scissor unless `scissored`.
#[test]
fn quad_state_for_each_buffer_combination() {
    const W: u32 = 64;
    const H: u32 = 48;
    let sh = ShcShader::ZERO;
    let p = ClearProg { fs: &sh, code_gpu: 0x0200_0000, location: 3 };
    let colour = [0.25, 0.5, 0.75, 1.0];
    let vertices = state::clear_quad_vertices(0.5);
    for flip_y in [false, true] {
        let mut d = busy_draw(W, H, flip_y);
        (d.do_clear, d.do_clear_depth, d.do_clear_stencil) = (true, true, true);
        (d.clear_quad, d.clear_color_mask, d.clear_rgba) = (true, 0x6, colour);
        (d.clear_depth, d.clear_stencil, d.clear_stencil_write_mask) = (0.5, 0x5a, 0x3c);
        /* a clear_quad carries the clear's own box in scissor_rect (the GL
         * core's clear-only draw) */
        d.scissor_rect = [-4, 10, 30, 50];
        for bits in 0u8..8 {
            let b = QuadBuffers { colour: bits & 1 != 0, depth: bits & 2 != 0, stencil: bits & 4 != 0 };
            for scissored in [false, true] {
                let why = format!("flip_y {flip_y}, {b:?}, scissored {scissored}");
                let q = state::clear_quad_draw(&d, b, scissored, &p, &vertices, &colour);

                /* the draw itself: the clear program over the six vertices */
                assert_eq!((q.prim, q.index_count, q.num_attribs), (GLES_PRIM_TRIANGLES, 6, 1), "{why}");
                assert!(core::ptr::eq(q.fs, &sh), "{why}");
                assert_eq!((q.fs_usc_gpu, q.attribs[0].vi_reg), (0x0200_0000, 3), "{why}");
                assert_eq!(q.attribs[0].cpu, vertices.as_ptr() as *const core::ffi::c_void, "{why}");
                assert_eq!(
                    (q.attribs[0].components, q.attribs[0].stride, q.attribs[0].cpu_bytes),
                    (4, 16, 96),
                    "{why}"
                );
                assert_eq!(q.fs_uniform_cpu, colour.as_ptr() as *const core::ffi::c_void, "{why}");
                assert_eq!(q.fs_uniform_dwords, 4, "{why}");
                assert_eq!((q.target_gpu, q.target_w, q.target_h, q.flip_y), (d.target_gpu, W, H, flip_y), "{why}");
                assert_eq!((q.viewport, q.depth_range), ([0, 0, W as i32, H as i32], [0.0, 1.0]), "{why}");

                let st = state::job_state(&q).unwrap_or_else(|e| panic!("{why}: {e}"));
                assert_eq!(st.vp, state::viewport(&full_viewport(W, H, flip_y)), "{why}");
                assert_eq!(st.colormask, if b.colour { 0x6 } else { 0 }, "{why}: colour write mask");
                assert!(!st.blend_enabled && !st.cull_front && !st.cull_back && !st.offset_tri, "{why}");

                assert_eq!(st.depth_enabled, b.depth, "{why}: depth test");
                assert_eq!(st.depth_writemask, b.depth, "{why}: depth write");
                assert_eq!(st.depth_func, V3D_COMPARE_FUNC_ALWAYS, "{why}: depth func");

                assert_eq!(st.stencil_enabled, b.stencil, "{why}: stencil");
                if b.stencil {
                    let f = st.stencil_front;
                    assert_eq!(f.func, V3D_COMPARE_FUNC_ALWAYS, "{why}");
                    assert_eq!(
                        (f.sfail_op, f.zfail_op, f.zpass_op),
                        (V3D_STENCIL_OP_REPLACE, V3D_STENCIL_OP_REPLACE, V3D_STENCIL_OP_REPLACE),
                        "{why}"
                    );
                    assert_eq!((f.ref_, f.value_mask, f.write_mask), (0x5a, 0xff, 0x3c), "{why}");
                    /* both faces the same: one STENCIL_CFG for both */
                    assert!(!st.stencil_back_enabled, "{why}: the back face differs");
                    assert_eq!(
                        (q.stencil_back_func, q.stencil_back_ref, q.stencil_back_write_mask),
                        (GLES_CMP_ALWAYS, 0x5a, 0x3c),
                        "{why}"
                    );
                    assert_eq!(
                        (q.stencil_back_sfail, q.stencil_back_dpfail, q.stencil_back_dppass),
                        (GLES_SOP_REPLACE, GLES_SOP_REPLACE, GLES_SOP_REPLACE),
                        "{why}"
                    );
                }

                if scissored {
                    let want = state::scissor(&GlesDraw {
                        scissor: true,
                        scissor_rect: [-4, 10, 30, 50],
                        ..target(W, H, flip_y)
                    });
                    let got =
                        st.scissor.then_some([st.scissor_minx, st.scissor_miny, st.scissor_maxx, st.scissor_maxy]);
                    assert_eq!(got, want, "{why}: the clear's box");
                } else {
                    assert!(!st.scissor && !q.scissor, "{why}: the scissor is off");
                }
            }
        }
    }
}

/// A target whose viewport is the whole target at depth range (0, 1): the
/// viewport the clear quad asks for.
fn full_viewport(w: u32, h: u32, flip_y: bool) -> GlesDraw {
    GlesDraw { viewport: [0, 0, w as i32, h as i32], depth_range: [0.0, 1.0], ..target(w, h, flip_y) }
}

/* ---- 4. the clear depth ------------------------------------------------------- */

/// The quad's z is `2d - 1` for every depth dEQP's depth_stencil_clear
/// asks for (`glClearDepthf` in [-0.2, 1.2], which the GL core clamps to
/// [0, 1]), and the quad's viewport takes it back to `d`: the depth the
/// quad writes is the clear depth. Its six vertices are two triangles over
/// the whole clip-space square, which the viewport maps onto the target.
#[test]
fn depth_clear_quad_z() {
    const W: u32 = 64;
    const H: u32 = 48;
    let sh = ShcShader::ZERO;
    let p = ClearProg { fs: &sh, code_gpu: 0, location: 0 };
    let b = QuadBuffers { colour: false, depth: true, stencil: false };
    for step in 0..=1400 {
        let asked = -0.2 + step as f32 * 0.001;
        let d = asked.clamp(0.0, 1.0);
        let vertices = state::clear_quad_vertices(d);
        let want = 2.0 * d as f64 - 1.0;
        for v in vertices {
            assert!((v[2] as f64 - want).abs() <= 1e-7, "depth {d}: z {} want {want}", v[2]);
            assert_eq!(v[3], 1.0, "depth {d}: w");
        }
        for flip_y in [false, true] {
            let src = GlesDraw { clear_depth: d, do_clear_depth: true, clear_quad: true, ..busy_draw(W, H, flip_y) };
            let q = state::clear_quad_draw(&src, b, false, &p, &vertices, &[0.0; 4]);
            let vp = state::viewport(&q);
            for v in vertices {
                let zw = v[2] * vp.scale[2] + vp.translate[2];
                assert!((zw - d).abs() <= 1e-6, "depth {d}, flip_y {flip_y}: window z {zw}");
            }
        }
    }
    /* the two triangles: (-1,-1) (1,-1) (1,1) and (-1,-1) (1,1) (-1,1) */
    let xy = state::clear_quad_vertices(0.0).map(|v| (v[0], v[1]));
    assert_eq!(xy, [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]);
    /* clip (-1,-1)..(1,1) is the whole target in window coordinates */
    for flip_y in [false, true] {
        let vp = state::viewport(&full_viewport(W, H, flip_y));
        let wx = |x: f32| x * vp.scale[0] + vp.translate[0];
        let wy = |y: f32| y * vp.scale[1] + vp.translate[1];
        let (mut ys, mut xs) = ([wy(-1.0), wy(1.0)], [wx(-1.0), wx(1.0)]);
        ys.sort_by(f32::total_cmp);
        xs.sort_by(f32::total_cmp);
        assert_eq!((xs, ys), ([0.0, W as f32], [0.0, H as f32]), "flip_y {flip_y}");
    }
}

/* ---- 5. a clear fused into a draw --------------------------------------------- */

/// A clear fused into an application's draw (the GL core puts a plain
/// clear into the next draw's `do_clear*`) carries its own masks
/// (`clear_color_mask`, `clear_stencil_write_mask`), while the draw has
/// different ones and its own scissor. The quad takes the clear's values,
/// never the draw's: gl_test 9 check 9 on the Pi 4 (2026-09-24) was the
/// draw's `glStencilMask(0xfe)` keeping bit 0 of a cleared stencil.
#[test]
fn masked_fused_clear_keeps_its_own_masks() {
    const W: u32 = 64;
    const H: u32 = 48;
    let sh = ShcShader::ZERO;
    let p = ClearProg { fs: &sh, code_gpu: 0, location: 0 };
    let vertices = state::clear_quad_vertices(1.0);
    let colour = [1.0, 0.0, 1.0, 0.0];
    for flip_y in [false, true] {
        let mut d = busy_draw(W, H, flip_y); /* color_mask 0x9, stencil masks 0xfe/0x7f, scissor on */
        (d.do_clear, d.clear_quad, d.clear_color_mask, d.clear_rgba) = (true, false, 0x5, colour);
        (d.do_clear_stencil, d.clear_stencil, d.clear_stencil_write_mask) = (true, 0x81, 0x0f);
        assert!(state::colour_needs_quad(&d), "a partial colour mask is the quad's");
        /* what the backend passes: the colour (mask 0x5) and, by
         * GFXH-1461, the stencil half; `scissored` is the draw's
         * clear_quad, false for a fused clear */
        let b = QuadBuffers { colour: true, depth: false, stencil: true };
        let q = state::clear_quad_draw(&d, b, d.clear_quad, &p, &vertices, &colour);
        let st = state::job_state(&q).expect("the quad's state");
        assert_eq!(st.colormask, 0x5, "flip_y {flip_y}: the clear's colour mask, not the draw's 0x9");
        assert_eq!(st.stencil_front.write_mask, 0x0f, "flip_y {flip_y}: the clear's stencil mask, not 0xfe");
        assert!(!st.stencil_back_enabled, "flip_y {flip_y}: not the draw's back mask 0x7f");
        assert_eq!(q.stencil_back_write_mask, 0x0f, "flip_y {flip_y}");
        assert_eq!(st.stencil_front.ref_, 0x81, "flip_y {flip_y}: the clear's value, not the draw's ref 9");
        assert!(!st.scissor, "flip_y {flip_y}: not the draw's scissor");
        assert!(!st.blend_enabled && !st.cull_front && !st.cull_back, "flip_y {flip_y}");

        /* the draw's own state is untouched by the quad: it still gets
         * its masks and scissor when it is drawn after the quad */
        let own = state::job_state(&d).expect("the draw's state");
        assert_eq!((own.colormask, own.stencil_front.write_mask), (0x9, 0xfe), "flip_y {flip_y}");
        assert!(own.scissor, "flip_y {flip_y}");
    }
    /* no write mask given (0) is the GL core's "not given": the quad
     * falls back to the draw's (a clear-only draw carries the clear's
     * state in the draw fields) */
    let mut d = GlesDraw { stencil_write_mask: 0x33, ..target(W, H, true) };
    (d.do_clear_stencil, d.clear_stencil, d.clear_quad) = (true, 7, true);
    let b = QuadBuffers { colour: false, depth: false, stencil: true };
    let q = state::clear_quad_draw(&d, b, true, &p, &vertices, &colour);
    assert_eq!(state::job_state(&q).expect("state").stencil_front.write_mask, 0x33);
}

/* ---- 6. dEQP color_clear on a model of the V3D -------------------------------- */

/// One `functional.color_clear.*` case (es2fColorClearTest.cpp `init`).
struct Case {
    name: &'static str,
    iters: u32,
    clears_min: u32,
    clears_max: u32,
    alpha: bool,
    scissor: bool,
    masks: bool,
    first_full: bool,
}

/// `Case` from the CTS's `addChild` line: name, iterations, clears min and
/// max, alpha, scissor, masks, first clear full.
type CaseArgs = (&'static str, u32, u32, u32, bool, bool, bool, bool);

const fn case(a: CaseArgs) -> Case {
    let (name, iters, clears_min, clears_max, alpha, scissor, masks, first_full) = a;
    Case { name, iters, clears_min, clears_max, alpha, scissor, masks, first_full }
}

/// The 19 cases, in the CTS's order (es2-v3d-clear.md
/// table 1.1).
const CASES: [Case; 19] = [
    case(("single_rgb", 30, 1, 3, false, false, false, true)),
    case(("single_rgba", 30, 1, 3, true, false, false, true)),
    case(("multiple_rgb", 15, 4, 20, false, false, false, true)),
    case(("multiple_rgba", 15, 4, 20, true, false, false, true)),
    case(("long_rgb", 2, 100, 500, false, false, false, true)),
    case(("long_rgba", 2, 100, 500, true, false, false, true)),
    case(("subclears_rgb", 15, 4, 30, false, false, false, false)),
    case(("subclears_rgba", 15, 4, 30, true, false, false, false)),
    case(("short_scissored_rgb", 30, 2, 4, false, true, false, true)),
    case(("scissored_rgb", 15, 4, 30, false, true, false, true)),
    case(("scissored_rgba", 15, 4, 30, true, true, false, true)),
    case(("masked_rgb", 15, 4, 30, false, false, true, true)),
    case(("masked_rgba", 15, 4, 30, true, false, true, true)),
    case(("masked_scissored_rgb", 15, 4, 30, false, true, true, true)),
    case(("masked_scissored_rgba", 15, 4, 30, true, true, true, true)),
    case(("complex_rgb", 15, 5, 50, false, true, true, false)),
    case(("complex_rgba", 15, 5, 50, true, true, true, false)),
    case(("long_masked_rgb", 2, 100, 500, false, true, true, true)),
    case(("long_masked_rgba", 2, 100, 500, true, true, true, true)),
];

/// dEQP's `ClearInfo`: the box (x, y, w, h), the 4-bit mask, the colour.
#[derive(Clone, Copy)]
struct ClearOp {
    rect: [i32; 4],
    mask: u8,
    rgba: [u8; 4],
}

/// The CTS reference (es2fColorClearTest.cpp, "Compute reference image"),
/// transcribed: per GL row, walk spans from the newest clear back, a
/// channel is set by the newest clear that covers it with that channel on.
/// Returns the reference colours and `pixelKnownChannelMask`, indexed by
/// GL (x, y) (y from the bottom). The target has alpha bits, so the
/// reference's alpha is not forced to 0xff.
fn cts_reference(ops: &[ClearOp], w: i32, h: i32) -> (Vec<[u8; 4]>, Vec<u8>) {
    let mut image = vec![[0u8; 4]; (w * h) as usize];
    let mut known = vec![0u8; (w * h) as usize];
    for y in 0..h {
        let line: Vec<ClearOp> =
            ops.iter().copied().filter(|op| op.rect[1] <= y && y < op.rect[1] + op.rect[3]).collect();
        let mut x = 0;
        while x < w {
            let mut span = [0u8; 4];
            let mut span_known = 0u8;
            let mut len = w - x;
            for op in line.iter().rev() {
                if in_box(op.rect, x, y) {
                    len = len.min(op.rect[0] + op.rect[2] - x);
                    for (c, v) in span.iter_mut().enumerate() {
                        if op.mask & (1 << c) != 0 && span_known & (1 << c) == 0 {
                            *v = op.rgba[c];
                        }
                    }
                    span_known |= op.mask;
                    if span_known == 0xf {
                        break;
                    }
                } else if op.rect[0] > x {
                    len = len.min(op.rect[0] - x);
                }
            }
            for i in 0..len {
                let at = (y * w + x + i) as usize;
                image[at] = span;
                known[at] |= span_known;
            }
            x += len;
        }
    }
    (image, known)
}

/// What the GL core makes of one `glClear(GL_COLOR_BUFFER_BIT)` (plan_clear
/// and apply_clear_plan in src/gl/draw.rs): nothing for a zero colour mask
/// or an empty box; `clear_quad` with the box when the box does not hold the
/// target; the mask in `clear_color_mask`.
fn gl_core_clear(t: &GlesDraw, scissor_on: bool, op: &ClearOp) -> Option<GlesDraw> {
    if op.mask == 0 {
        return None;
    }
    let mut d = GlesDraw { do_clear: true, clear_color_mask: op.mask, ..*t };
    d.clear_rgba = op.rgba.map(|v| v as f32 / 255.0);
    if scissor_on {
        let [x0, y0, bw, bh] = op.rect;
        if bw <= 0 || bh <= 0 {
            return None;
        }
        if x0 > 0 || y0 > 0 || x0 + bw < t.target_w as i32 || y0 + bh < t.target_h as i32 {
            (d.clear_quad, d.scissor, d.scissor_rect) = (true, true, op.rect);
        }
    }
    Some(d)
}

/// How the clears of one model run went.
#[derive(Default)]
struct Paths {
    tlb: u32,
    quad: u32,
    quad_empty: u32,
}

/// The V3D model: a TLB clear writes the whole target, every channel; the
/// clear quad writes every pixel inside its job state's scissor (the quad
/// covers the whole target), the channels of its job state's colour write
/// mask. Both write the clear colour as `pack_rgba8` rounds it. `fb` is in
/// memory order (row 0 is the top for a window).
fn v3d_clear(fb: &mut [[u8; 4]], d: &GlesDraw, paths: &mut Paths) {
    let (w, h) = (d.target_w as i32, d.target_h as i32);
    let rgba = state::pack_rgba8(d.clear_rgba).to_le_bytes();
    if !state::colour_needs_quad(d) {
        /* the TLB is only taken for what it can do */
        assert!(!d.clear_quad && d.clear_color_mask == 0xf, "a TLB clear with a box or a mask");
        fb.fill(rgba);
        paths.tlb += 1;
        return;
    }
    let sh = ShcShader::ZERO;
    let p = ClearProg { fs: &sh, code_gpu: 0, location: 0 };
    let vertices = state::clear_quad_vertices(1.0);
    let b = QuadBuffers { colour: true, depth: false, stencil: false };
    let q = state::clear_quad_draw(d, b, d.clear_quad, &p, &vertices, &d.clear_rgba);
    let st = state::job_state(&q).expect("the clear quad's state");
    let mut wrote = false;
    for row in 0..h {
        for col in 0..w {
            if in_job_scissor(&st, col, row) {
                let px = &mut fb[(row * w + col) as usize];
                for c in 0..4 {
                    if st.colormask & (1 << c) != 0 {
                        px[c] = rgba[c];
                    }
                }
                wrote = true;
            }
        }
    }
    paths.quad += 1;
    paths.quad_empty += u32::from(!wrote);
}

/// All 19 dEQP `functional.color_clear.*` cases, each with its iteration
/// count and clear counts, on a model of the V3D (TLB clear or clear quad,
/// as `colour_needs_quad`, `clear_quad_draw`, `job_state` and `scissor`
/// decide) against the CTS reference: every channel the reference knows
/// matches exactly, for a window (y flipped) and for an FBO. The target is
/// smaller than the board's window and not a multiple of the 64x64 tile.
///
/// The model writes exact 8-bit values; dEQP allows one step (two with
/// dithering), so exact equality is the stricter check.
#[test]
fn cts_color_clear_reference() {
    const W: u32 = 61;
    const H: u32 = 45;
    let mut total = Paths::default();
    for flip_y in [false, true] {
        let t = target(W, H, flip_y);
        for (ci, cs) in CASES.iter().enumerate() {
            for iter in 0..cs.iters {
                let mut rng = Rng::new(((ci as u32) << 16) ^ (iter << 1) ^ u32::from(flip_y));
                let clears = rng.next() % (cs.clears_max + 1 - cs.clears_min) + cs.clears_min;
                /* the target's contents before the first clear are undefined:
                 * any bytes; the reference does not compare them */
                let mut fb = vec![[0xa5, 0x3c, 0x5a, 0xc3]; (W * H) as usize];
                let mut ops = Vec::new();
                let mut paths = Paths::default();
                for n in 0..clears {
                    let first_full = n == 0 && cs.first_full;
                    let rect =
                        if !cs.scissor || first_full { [0, 0, W as i32, H as i32] } else { cts_box(&mut rng, W, H) };
                    let r = (rng.next() & 0xff) as u8;
                    let g = (rng.next() & 0xff) as u8;
                    let b = (rng.next() & 0xff) as u8;
                    let a = if cs.alpha { (rng.next() & 0xff) as u8 } else { 0xff };
                    let mask = if !cs.masks || first_full { 0xf } else { (rng.next() & 0xf) as u8 };
                    let op = ClearOp { rect, mask, rgba: [r, g, b, a] };
                    ops.push(op);
                    if let Some(d) = gl_core_clear(&t, cs.scissor, &op) {
                        v3d_clear(&mut fb, &d, &mut paths);
                    }
                }
                let (reference, known) = cts_reference(&ops, W as i32, H as i32);
                for y in 0..H as i32 {
                    for x in 0..W as i32 {
                        let at = (y * W as i32 + x) as usize;
                        let got = fb[(mem_row(y, H, flip_y) * W as i32 + x) as usize];
                        for c in 0..4 {
                            if known[at] & (1 << c) != 0 {
                                assert_eq!(
                                    got[c], reference[at][c],
                                    "{} iter {iter}, flip_y {flip_y}: pixel ({x}, {y}) channel {c}",
                                    cs.name
                                );
                            }
                        }
                    }
                }
                /* each case runs the path it is about */
                if !cs.scissor && !cs.masks {
                    assert_eq!(paths.quad, 0, "{}: a quad without a box or a mask", cs.name);
                }
                if cs.scissor || cs.masks {
                    assert!(paths.quad > 0 || clears < 4, "{} iter {iter}: no quad in {clears} clears", cs.name);
                }
                total.tlb += paths.tlb;
                total.quad += paths.quad;
                total.quad_empty += paths.quad_empty;
            }
        }
    }
    /* both paths, and quads whose box misses the target, were exercised */
    assert!(total.tlb > 1000 && total.quad > 1000, "tlb {}, quad {}", total.tlb, total.quad);
    assert!(total.quad_empty > 0, "no clear quad with an empty scissor");
}

/// The clear colour dEQP gives (`r / 255.0`) comes back as `r` from
/// `pack_rgba8`, the TLB's packing, for every 8-bit value; the model above
/// relies on this.
#[test]
fn cts_clear_colours_pack_exactly() {
    for v in 0..=255u32 {
        let f = v as f32 / 255.0;
        assert_eq!(state::pack_rgba8([f, f, f, f]), v * 0x0101_0101, "{v}");
    }
}
