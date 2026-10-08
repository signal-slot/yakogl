// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D `glClear` on the host (docs/investigations/es2-v3d-clear.md, 5.1): the
//! decisions, the clear quad's state and the scissor geometry.
//!
//! The host has no V3D rasteriser or tile buffer, so nothing here checks a
//! pixel the hardware wrote. The tests pin what the Pi 4 board runs of
//! dEQP-GLES2 `color_clear`, `depth_stencil_clear` and `fragment_ops` rely
//! on: when a clear is a TLB clear and when a quad ([`state::colour_needs_quad`]),
//! the state the quad is drawn with ([`state::clear_quad_draw`] through
//! [`state::job_state`]), and the scissor box ([`state::scissor`]) against the
//! CTS reference rectangle. The last test drives a small software model of
//! the V3D semantics with the colour_clear generator.

#![cfg(all(feature = "gpu-v3d", feature = "host-test"))]

use yakogl::backend::*;
use yakogl::shaderc::{ShcShader, SHC_BF_ONE, SHC_BF_SRC_ALPHA};
use yakogl::v3d::state::{self, ClearProg, QuadBuffers};

/// xorshift64*: any fixed seed will do, the CTS's own hash is not reproduced.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11
    }
    /// A value in `lo..hi`.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo) as u64) as i32
    }
}

/// dEQP's colour_clear box: x, y in [-W, W), w < W, h < H.
fn cts_box(rng: &mut Rng, w: i32, h: i32) -> [i32; 4] {
    [rng.range(-w, w), rng.range(-h, h), rng.range(0, w), rng.range(0, h)]
}

/// The CTS reference rectangle (the intersection of the box with the target),
/// as a half-open `[x0, y0, x1, y1)` in GL's rows (row 0 at the bottom), or
/// `None` when it is empty. Plain arithmetic, not `state::scissor`.
fn cts_rect(b: [i32; 4], w: i32, h: i32) -> Option<[i32; 4]> {
    let [x, y, bw, bh] = b;
    let (x0, y0, x1, y1) = (x.max(0), y.max(0), (x + bw).min(w), (y + bh).min(h));
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

fn target(w: u32, h: u32, flip_y: bool) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    (d.target_w, d.target_h, d.flip_y) = (w, h, flip_y);
    (d.target_format, d.target_alpha_bits) = (GLES_SURF_RGBA8888, 8);
    d.viewport = [0, 0, w as i32, h as i32];
    d.depth_range = [0.0, 1.0];
    d
}

#[test]
fn scissor_is_the_cts_rectangle() {
    let (w, h) = (16i32, 12i32);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut empty, mut covered_total) = (0, 0u64);
    for flip_y in [false, true] {
        for _ in 0..4000 {
            let b = cts_box(&mut rng, w, h);
            let mut d = target(w as u32, h as u32, flip_y);
            (d.scissor, d.scissor_rect) = (true, b);
            let [minx, miny, maxx, maxy] = state::scissor(&d).expect("the scissor test is on");
            let want = cts_rect(b, w, h);
            if want.is_none() {
                empty += 1;
                /* the empty box is all zero before the window flip; after
                 * it the rows are the target height twice (an empty range) */
                let rows = if flip_y { (h as u32, h as u32) } else { (0, 0) };
                assert_eq!([minx, miny, maxx, maxy], [0, rows.0, 0, rows.1], "flip {flip_y} box {b:?}");
            }
            for gl_row in 0..h {
                /* memory row of GL row `gl_row` */
                let row = if flip_y { h - 1 - gl_row } else { gl_row } as u32;
                for x in 0..w {
                    let got = (minx..maxx).contains(&(x as u32)) && (miny..maxy).contains(&row);
                    let exp = want.is_some_and(|r| x >= r[0] && x < r[2] && gl_row >= r[1] && gl_row < r[3]);
                    assert_eq!(got, exp, "flip {flip_y} box {b:?} pixel ({x},{gl_row})");
                    covered_total += u64::from(got);
                }
            }
        }
    }
    /* the generator reaches the empty boxes */
    assert!(empty > 500, "empty boxes: {empty}");
    assert!(covered_total > 0);
    /* test off: no scissor at all */
    assert_eq!(state::scissor(&target(16, 12, false)), None);
}

#[test]
fn colour_needs_quad_is_the_tlb_decision() {
    for do_clear in [false, true] {
        for clear_quad in [false, true] {
            for mask in 0u8..=15 {
                let mut d = target(8, 8, false);
                (d.do_clear, d.clear_quad, d.clear_color_mask) = (do_clear, clear_quad, mask);
                let quad = do_clear && (clear_quad || mask != 0xf);
                assert_eq!(
                    state::colour_needs_quad(&d),
                    quad,
                    "do_clear {do_clear} clear_quad {clear_quad} mask {mask:#x}"
                );
            }
        }
    }
    /* only the low four bits are channels */
    let mut d = target(8, 8, false);
    (d.do_clear, d.clear_color_mask) = (true, 0xff);
    assert!(!state::colour_needs_quad(&d));
}

/// A fused clear's draw state, all different from the clear's.
fn clear_over_draw(scissored: bool) -> GlesDraw {
    let mut d = target(64, 48, true);
    d.do_clear = true;
    d.clear_quad = scissored;
    d.clear_color_mask = 0b0101;
    d.clear_stencil = 0x5a;
    d.clear_stencil_write_mask = 0x0f;
    /* the application's draw state */
    d.color_write = true;
    d.color_mask = 0b1010;
    d.stencil = true;
    d.stencil_write_mask = 0xf0;
    d.stencil_back_write_mask = 0x3c;
    d.blend = true;
    (d.blend_src_rgb, d.blend_dst_rgb) = (SHC_BF_SRC_ALPHA as u8, SHC_BF_ONE as u8);
    (d.blend_src_alpha, d.blend_dst_alpha) = (SHC_BF_ONE as u8, SHC_BF_ONE as u8);
    d.cull = GLES_CULL_CW;
    d.depth_test = true;
    d.depth_func = GLES_CMP_LESS;
    d.scissor = scissored;
    d.scissor_rect = [3, 4, 20, 10];
    d
}

#[test]
fn quad_state_for_each_buffer_combination() {
    let sh = ShcShader::ZERO;
    let prog = ClearProg { fs: &sh, code_gpu: 0x1000, location: 0 };
    let colour = [0.1, 0.2, 0.3, 0.4];
    let verts = state::clear_quad_vertices(0.25);
    for scissored in [false, true] {
        let src = clear_over_draw(scissored);
        for bits in 0u8..8 {
            let b = QuadBuffers { colour: bits & 1 != 0, depth: bits & 2 != 0, stencil: bits & 4 != 0 };
            let q = state::clear_quad_draw(&src, b, scissored, &prog, &verts, &colour);
            let ctx = format!("{b:?} scissored {scissored}");
            assert_eq!(q.index_count, 6, "{ctx}");
            assert_eq!(q.viewport, [0, 0, 64, 48], "{ctx}");
            assert_eq!((q.target_w, q.target_h, q.flip_y), (64, 48, true), "{ctx}");
            assert!(!q.blend, "{ctx}");
            assert_eq!(q.cull, GLES_CULL_NONE, "{ctx}");
            assert!(!q.polygon_offset, "{ctx}");
            assert_eq!(q.color_mask, if b.colour { 0b0101 } else { 0 }, "{ctx}");
            assert_eq!(q.depth_test, b.depth, "{ctx}");
            assert_eq!(q.depth_write, b.depth, "{ctx}");
            assert_eq!(q.depth_func, GLES_CMP_ALWAYS, "{ctx}");
            assert_eq!(q.stencil, b.stencil, "{ctx}");
            if b.stencil {
                let front = (q.stencil_func, q.stencil_ref, q.stencil_write_mask, q.stencil_cmp_mask);
                let back =
                    (q.stencil_back_func, q.stencil_back_ref, q.stencil_back_write_mask, q.stencil_back_cmp_mask);
                for face in [front, back] {
                    assert_eq!(face, (GLES_CMP_ALWAYS, 0x5a, 0x0f, 0xff), "{ctx}");
                }
                for ops in [
                    [q.stencil_sfail, q.stencil_dpfail, q.stencil_dppass],
                    [q.stencil_back_sfail, q.stencil_back_dpfail, q.stencil_back_dppass],
                ] {
                    assert_eq!(ops, [GLES_SOP_REPLACE; 3], "{ctx}");
                }
            }
            assert_eq!(q.scissor, scissored, "{ctx}");
            assert_eq!(state::scissor(&q).is_some(), scissored, "{ctx}");
            /* and through the job state the V3D is given */
            let st = state::job_state(&q).expect("a clear quad is drawable");
            assert_eq!(st.colormask, if b.colour { 0b0101 } else { 0 }, "{ctx}");
            assert_eq!((st.depth_enabled, st.depth_writemask), (b.depth, b.depth), "{ctx}");
            assert!(!st.blend_enabled && !st.cull_front && !st.cull_back && !st.offset_tri, "{ctx}");
            assert_eq!(st.scissor, scissored, "{ctx}");
            assert_eq!(st.stencil_enabled, b.stencil, "{ctx}");
            /* both faces are the same, so the back state stays off */
            assert!(!st.stencil_back_enabled, "{ctx}");
        }
    }
}

#[test]
fn depth_clear_quad_z() {
    let mut rng = Rng(7);
    let mut depths = vec![0.0f32, 0.5, 1.0];
    for _ in 0..200 {
        /* dEQP draws glClearDepthf in [-0.2, 1.2]; the GL core clamps */
        let raw = -0.2 + 1.4 * (rng.next() % 1_000_001) as f32 / 1_000_000.0;
        depths.push(raw.clamp(0.0, 1.0));
    }
    let d = target(32, 32, false);
    let vp = state::viewport(&d);
    for dep in depths {
        for v in state::clear_quad_vertices(dep) {
            assert_eq!(v[2], 2.0 * dep - 1.0);
            assert_eq!(v[3], 1.0);
            let z = vp.scale[2] * v[2] + vp.translate[2];
            assert!((z - dep).abs() <= 1e-6, "depth {dep} became {z}");
        }
    }
}

#[test]
fn masked_fused_clear_keeps_its_own_masks() {
    let sh = ShcShader::ZERO;
    let prog = ClearProg { fs: &sh, code_gpu: 0x1000, location: 0 };
    let verts = state::clear_quad_vertices(1.0);
    let src = clear_over_draw(false);
    let b = QuadBuffers { colour: true, depth: false, stencil: true };
    let q = state::clear_quad_draw(&src, b, false, &prog, &verts, &[0.0; 4]);
    assert_eq!(q.color_mask, 0b0101);
    assert_eq!((q.stencil_write_mask, q.stencil_back_write_mask), (0x0f, 0x0f));
    assert!(!q.scissor, "the draw's scissor is not the clear's");
    let st = state::job_state(&q).unwrap();
    assert_eq!(st.colormask, 0b0101);
    /* equal faces: one STENCIL_CFG covers both, the back state stays off */
    assert_eq!(st.stencil_front.write_mask, 0x0f);
    assert!(!st.stencil_back_enabled);
    /* the same draw's own state is still the draw's */
    let own = state::job_state(&src).unwrap();
    assert_eq!(own.colormask, 0b1010);
    assert_eq!(own.stencil_front.write_mask, 0xf0);
    /* a clear that carries no stencil mask of its own falls back to the draw's */
    let mut d = clear_over_draw(false);
    d.clear_stencil_write_mask = 0;
    let q = state::clear_quad_draw(&d, b, false, &prog, &verts, &[0.0; 4]);
    assert_eq!(q.stencil_write_mask, 0xf0);
}

/* ---- the colour_clear cases against a model ------------------------------ */

struct Case {
    name: &'static str,
    iters: u32,
    clears: (i32, i32),
    scissor: bool,
    mask: bool,
    first_full: bool,
}

const fn case(name: &'static str, iters: u32, lo: i32, hi: i32, scissor: bool, mask: bool, first_full: bool) -> Case {
    Case { name, iters, clears: (lo, hi), scissor, mask, first_full }
}

/// Table 1.1 of the diagnosis (es2fColorClearTest.cpp).
const CASES: [Case; 19] = [
    case("single_rgb", 30, 1, 3, false, false, true),
    case("single_rgba", 30, 1, 3, false, false, true),
    case("multiple_rgb", 15, 4, 20, false, false, true),
    case("multiple_rgba", 15, 4, 20, false, false, true),
    case("long_rgb", 2, 100, 500, false, false, true),
    case("long_rgba", 2, 100, 500, false, false, true),
    case("subclears_rgb", 15, 4, 30, false, false, false),
    case("subclears_rgba", 15, 4, 30, false, false, false),
    case("short_scissored_rgb", 30, 2, 4, true, false, true),
    case("scissored_rgb", 15, 4, 30, true, false, true),
    case("scissored_rgba", 15, 4, 30, true, false, true),
    case("masked_rgb", 15, 4, 30, false, true, true),
    case("masked_rgba", 15, 4, 30, false, true, true),
    case("masked_scissored_rgb", 15, 4, 30, true, true, true),
    case("masked_scissored_rgba", 15, 4, 30, true, true, true),
    case("complex_rgb", 15, 5, 50, true, true, false),
    case("complex_rgba", 15, 5, 50, true, true, false),
    case("long_masked_rgb", 2, 100, 500, true, true, true),
    case("long_masked_rgba", 2, 100, 500, true, true, true),
];

/// An RGBA8 image in memory rows, and the channels each pixel is known in
/// (the CTS's `pixelKnownChannelMask`).
struct Image {
    px: Vec<[u8; 4]>,
    known: Vec<u8>,
}

impl Image {
    fn new(n: usize) -> Image {
        /* contents the hardware would not promise: a pattern, not zero */
        Image { px: (0..n).map(|i| [0xa5 ^ i as u8; 4]).collect(), known: vec![0; n] }
    }

    fn write(&mut self, p: usize, mask: u8, c: [u8; 4]) {
        for (ch, (dst, &src)) in self.px[p].iter_mut().zip(c.iter()).enumerate() {
            if mask >> ch & 1 != 0 {
                *dst = src;
                self.known[p] |= 1 << ch;
            }
        }
    }
}

#[test]
fn cts_color_clear_reference() {
    assert_eq!(CASES.len(), 19);
    let (w, h) = (24i32, 18i32);
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for flip_y in [false, true] {
        for case in &CASES {
            let rgba = case.name.ends_with("_rgba");
            for iter in 0..case.iters {
                let n = rng.range(case.clears.0, case.clears.1 + 1);
                let mut want = Image::new((w * h) as usize);
                let mut got = Image::new((w * h) as usize);
                for i in 0..n {
                    let full = i == 0 && case.first_full;
                    let scissor = case.scissor && !full;
                    let mask = if case.mask && !full { rng.range(0, 16) as u8 } else { 0xf };
                    let b = if scissor { cts_box(&mut rng, w, h) } else { [0, 0, w, h] };
                    let alpha = if rgba { rng.range(0, 256) as u8 } else { 255 };
                    let c = [rng.range(0, 256) as u8, rng.range(0, 256) as u8, rng.range(0, 256) as u8, alpha];

                    /* the CTS reference: every pixel of the box takes the
                     * channels of the mask, and knows them from then on */
                    if let Some([x0, y0, x1, y1]) = cts_rect(b, w, h) {
                        for y in y0..y1 {
                            for x in x0..x1 {
                                want.write((y * w + x) as usize, mask, c);
                            }
                        }
                    }

                    /* the V3D's: the GL core's plan_clear (a box that holds
                     * the target is dropped, an empty box or mask 0 clears
                     * nothing), then the back end's TLB-or-quad decision */
                    let mut d = target(w as u32, h as u32, flip_y);
                    d.do_clear = true;
                    d.clear_color_mask = mask;
                    if scissor && cts_rect(b, w, h) != Some([0, 0, w, h]) {
                        (d.scissor, d.scissor_rect, d.clear_quad) = (true, b, true);
                    }
                    if mask == 0 {
                        continue;
                    }
                    let area = if state::colour_needs_quad(&d) {
                        match state::scissor(&d) {
                            Some([x0, y0, x1, y1]) => [x0 as i32, y0 as i32, x1 as i32, y1 as i32],
                            None => [0, 0, w, h], /* a masked clear of the whole target */
                        }
                    } else {
                        [0, 0, w, h] /* the tile buffer clear: whole target, every channel */
                    };
                    for row in area[1]..area[3] {
                        let gl_row = if flip_y { h - 1 - row } else { row };
                        for x in area[0]..area[2] {
                            got.write((gl_row * w + x) as usize, mask, c);
                        }
                    }
                }
                for p in 0..(w * h) as usize {
                    let ctx = format!("{} iter {iter} flip {flip_y} pixel {p}", case.name);
                    assert_eq!(got.known[p], want.known[p], "{ctx}: known channels");
                    for (ch, (g, w)) in got.px[p].iter().zip(want.px[p].iter()).enumerate() {
                        if want.known[p] >> ch & 1 != 0 {
                            assert_eq!(g, w, "{ctx} channel {ch}");
                        }
                    }
                }
            }
        }
    }
}
