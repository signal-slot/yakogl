// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2-es2 `refract` on the PowerVR: which mip LOD it samples from its
//! mipmapped target, and how much three levels a render
//! (`pvr_backend_set_mip_three_levels`, off by default,
//! docs/notes/pvr-mip-three-levels.md) can move its picture. The last open
//! refract item of docs/notes/pvr-mip-three-levels-es2.md part (c).
//!
//! The scene (glmark2 `22c527cb`, `src/scene-refract.cpp`,
//! `data/shaders/light-refract.{vert,frag}`, `depth.{vert,frag}`; the
//! glmark2 tree is not in this repository, so its numbers are constants
//! here, each with where it was read):
//!
//! * One chain a frame: `tex_[COLOR]`, the colour target of the light-view
//!   depth pass, twice the canvas: 1600x1200 RGBA8 at 800x600
//!   (`DistanceRenderTarget::setup`; yakogl's GL_MAX_TEXTURE_SIZE is 4096,
//!   src/gl/texture.rs, so no clamp), `GL_LINEAR_MIPMAP_LINEAR`,
//!   `glGenerateMipmap` after the depth pass (`scene-refract.cpp:430`).
//!   Levels 0..=10. It holds `depth.frag`'s `vec4(Normal, 1.0)`: the
//!   model-space normal, clamped to [0, 1] by the UNORM store.
//! * The camera pass samples it as `NormalMap` at
//!   `normcoord = back_position.st * 0.5 + 0.5`, where
//!   `back_position = vertex_position.xyz + front_refraction * distance`
//!   is an eye-space point (`light-refract.frag`). The other two samplers
//!   have no chain (`DistanceMap` the depth texture, NEAREST; `ImageMap`
//!   nasa1, 512x512, LINEAR), so the knob reaches the picture only through
//!   `back_normal = texture2D(NormalMap, normcoord).xyz`, which then goes
//!   through `refract(back_position, back_normal, 1/1.2)` into the nasa1
//!   coordinate.
//!
//! What the tests pin:
//!
//! 1. `refract_samples_lod_1_5_to_2_5_face_on_and_level_1_up`: the LOD
//!    (ES 2.0 3.7.7) of the position term of `normcoord` over the bunny's
//!    depth range: 1.48 to 2.53, so face-on the sample reads levels 1, 2
//!    and 3 (level 3 with a weight of at most 0.53), never level 0. Slopes,
//!    the refraction term and the depth texture's edges only add to rho
//!    (or, where they cancel the position term, are not bounded here: the
//!    mesh is not on the host), so at silhouettes every level up to 10 is
//!    reachable: refract's range is levels 1..=10, centred on 1..=3.
//! 2. `levels` (shaderc-online): the 1600x1200 chain with the knob on,
//!    in the model of tests/pvr_mip_three_levels.rs (the reference IR
//!    evaluator runs mipgen3.frag, look-ups by a model of the TPU's
//!    bilinear sample, round-half-up pack): levels 1 and 2 byte-identical
//!    to the knob-off chain, every level from 3 on within 1, on a
//!    normal-map content and on random bytes.
//! 3. `the_normal_sample_moves_by_1_lsb_at_most_but_the_picture_is_not_bound_by_it`:
//!    the bound the plan asked for, the chain levels' worst LSB difference
//!    weighted by the trilinear weights, is <= 1 LSB of the NormalMap
//!    sample at every LOD (0.53 LSB face-on), under tolerance 8. That
//!    bound is the sample's, not the picture's: the refract of the shader
//!    turns 1 LSB of `back_normal` into up to ~34 texels of nasa1
//!    coordinate, so a pixel of the bunny can move by up to nasa1's whole
//!    contrast. What does hold exactly: the knob changes no coverage, so
//!    every background pixel (the four corner probes of the am62p
//!    reference, `000000`) is the same byte for byte. The mean's change is
//!    printed as an estimate, not asserted (it needs the mesh).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_refract_lod
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_refract_lod   (1 and 3 only)
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr"))]

/// The picture tolerance of tools/cts/glmark2-pictures.txt (`tolerance 8`;
/// `judge_picture`, tools/cts/common-tally.py: a mismatch is `d > tol`).
const TOLERANCE: f64 = 8.0;

/// glmark2's default window (`GM2_SIZE`, the board runs' 800x600).
const CANVAS: (f64, f64) = (800.0, 600.0);
/// `tex_[COLOR]`: twice the canvas.
const TEX: (u32, u32) = (1600, 1200);
/// Levels of the 1600x1200 chain, level 0 included: floor(log2 1600) + 1.
const LEVELS: usize = 11;

/// `Model::maxVec`/`minVec` of `data/models/bunny.obj` (34835 vertices,
/// read once at glmark2 `22c527cb`): the bounding box `RefractPrivate::setup`
/// fits the projection to.
const BUNNY_MIN: [f64; 3] = [-1.0, -0.991233, -0.775047];
const BUNNY_MAX: [f64; 3] = [1.0, 0.991233, 0.775047];

/// `RefractiveIndex` (option `index`, default 1.2): the front refract's eta;
/// the back one is its inverse.
const INDEX: f64 = 1.2;

/// nasa1.png (512x512), read once at glmark2 `22c527cb` by decoding the PNG:
/// the largest step between two neighbouring texels in a channel, and the
/// mean change of the largest channel of a bilinear sample moved by 34
/// texels (`worst_shift_per_lsb`) in a random direction (20000 samples,
/// REPEAT).
const NASA1_SIZE: f64 = 512.0;
const NASA1_MAX_STEP: f64 = 219.0;
const NASA1_MEAN_CHANGE_AT_34: f64 = 48.5;
/// nasa1's mean RGB, the same read: with the am62p reference mean (23 18
/// 30, tools/cts/glmark2-pictures.txt) it estimates the share of the grid
/// the bunny covers.
const NASA1_MEAN: [f64; 3] = [127.5, 96.6, 132.9];
const AM62P_REFRACT_MEAN: [f64; 3] = [23.0, 18.0, 30.0];

/// The scene's fit (`RefractPrivate::setup`): the bounding sphere's radius
/// r = |max - min| / 2, its centre, and tan(fovy / 2) = r / (2 + r)
/// (`fovy = 2 atan(r / (2 + r))`), near 2, the centre at eye depth 2 + r
/// (`modelview_.translate(-c.x, -c.y, -(c.z + 2 + r))`).
fn fit() -> (f64, [f64; 3], f64) {
    let d: Vec<f64> = (0..3).map(|i| BUNNY_MAX[i] - BUNNY_MIN[i]).collect();
    let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0;
    let c = [0, 1, 2].map(|i| (BUNNY_MAX[i] + BUNNY_MIN[i]) / 2.0);
    (r, c, r / (2.0 + r))
}

/// The bunny's eye-space depth range: the model turns about y
/// (`modelview_.rotate(rotation_, 0, 1, 0)`), so a box point's depth is the
/// centre's 2 + r plus at most the box's half diagonal in x and z.
fn depth_range() -> (f64, f64) {
    let (r, _, _) = fit();
    let hx = (BUNNY_MAX[0] - BUNNY_MIN[0]) / 2.0;
    let hz = (BUNNY_MAX[2] - BUNNY_MIN[2]) / 2.0;
    let e = (hx * hx + hz * hz).sqrt();
    (2.0 + r - e, 2.0 + r + e)
}

/// rho (ES 2.0 3.7.7: the larger of the two screen-axis derivative lengths,
/// in texels) of `normcoord`'s position term at eye depth `z`, a surface
/// facing the camera. A pixel is 2 z tan(fovy/2) / 600 eye units high and
/// 2 z tan(fovy/2) aspect / 800 wide: the same, square pixels. `normcoord`
/// is half the eye-space st, so d s / d x = 1600 * 0.5 * fp texels a pixel
/// and d t / d y = 1200 * 0.5 * fp; rho is the s one.
fn rho_face_on(z: f64) -> f64 {
    let (_, _, tan) = fit();
    let fp = 2.0 * z * tan / CANVAS.1;
    let fpx = 2.0 * z * tan * (CANVAS.0 / CANVAS.1) / CANVAS.0;
    assert!((fp - fpx).abs() < 1e-12, "square pixels");
    (TEX.0 as f64 * 0.5 * fpx).max(TEX.1 as f64 * 0.5 * fp)
}

/// The knob-on groups of a chain of `n` levels (level 0 included): level 1
/// alone, then up to three from the level above them
/// (src/pvr/backend.rs `mip_chain_gpu`, as tests/pvr_mip_three_levels.rs
/// has it). The first level of each group is a TPU sample of its source.
fn groups(n: usize) -> Vec<Vec<usize>> {
    let mut g = vec![vec![1]];
    let mut l = 2;
    while l < n {
        let m = (n - l).min(3);
        g.push((l..l + m).collect());
        l += m;
    }
    g
}

/// The trilinear weight of level `l` at LOD `lambda` (ES 2.0 3.7.7,
/// LINEAR_MIPMAP_LINEAR: levels floor(lambda) and floor(lambda) + 1; level 0
/// alone at lambda <= 0, the last level alone past it).
fn weight(l: usize, lambda: f64) -> f64 {
    let last = (LEVELS - 1) as f64;
    let lam = lambda.clamp(0.0, last);
    let k = lam.floor();
    let f = lam - k;
    if l as f64 == k {
        if k == last {
            1.0
        } else {
            1.0 - f
        }
    } else if l as f64 == k + 1.0 {
        f
    } else {
        0.0
    }
}

/// GLSL ES 1.00 8.4 `refract(I, N, eta)`.
fn refract(i: [f64; 3], n: [f64; 3], eta: f64) -> [f64; 3] {
    let d = i[0] * n[0] + i[1] * n[1] + i[2] * n[2];
    let k = 1.0 - eta * eta * (1.0 - d * d);
    if k < 0.0 {
        return [0.0; 3];
    }
    let s = eta * d + k.sqrt();
    [0, 1, 2].map(|c| eta * i[c] - s * n[c])
}

#[test]
fn refract_samples_lod_1_5_to_2_5_face_on_and_level_1_up() {
    let (r, c, tan) = fit();
    assert!((r - 1.607246).abs() < 1e-5, "r = {r}");
    assert!(c.iter().all(|v| v.abs() < 1e-9), "the box is centred: {c:?}");
    assert!((tan - 0.445560).abs() < 1e-5, "tan(fovy/2) = {tan}");
    assert_eq!((usize::BITS - (TEX.0 as usize).leading_zeros()) as usize, LEVELS);
    let (zn, zf) = depth_range();
    assert!(zn > 2.0, "the near plane (2) clips nothing: {zn}");
    let (lo, hi) = (rho_face_on(zn).log2(), rho_face_on(zf).log2());
    println!(
        "refract NormalMap: eye depth {zn:.3}..{zf:.3}, face-on rho {:.3}..{:.3}, lambda {lo:.3}..{hi:.3}",
        rho_face_on(zn),
        rho_face_on(zf)
    );
    assert!((lo - 1.4765).abs() < 1e-3, "face-on LOD at the nearest point: {lo}");
    assert!((hi - 2.5334).abs() < 1e-3, "face-on LOD at the farthest point: {hi}");
    /* Face-on, levels 1, 2 and 3 are read; level 0 never (lambda > 0), and
     * level 3 at a weight of at most lambda - 2. */
    let lam: Vec<f64> = (0..=1000).map(|k| lo + (hi - lo) * k as f64 / 1000.0).collect();
    let read: Vec<usize> = (0..LEVELS).filter(|&l| lam.iter().any(|&x| weight(l, x) > 0.0)).collect();
    assert_eq!(read, vec![1, 2, 3], "levels read face-on");
    let w3 = lam.iter().map(|&x| weight(3, x)).fold(0.0, f64::max);
    assert!((w3 - (hi - 2.0)).abs() < 1e-9 && w3 < 0.54, "level 3's weight face-on: {w3}");
    /* The knob's groups of an 11-level chain: [1], [2, 3, 4], [5, 6, 7],
     * [8, 9, 10]. Levels 1 and 2 are TPU samples of level 0 and level 1,
     * which are the same in both chains: the knob cannot move them. From
     * level 3 on a level is made by the ALU or sampled from one that was. */
    assert_eq!(groups(LEVELS), vec![vec![1], vec![2, 3, 4], vec![5, 6, 7], vec![8, 9, 10]]);
    /* The refraction term: front_refraction is a unit vector (|refract| = 1
     * for unit I and N) or 0 past the critical angle, times the depth
     * texture's value in [0, 1]. Its screen derivative is the normal's
     * curvature times the pixel footprint times dT/dN, plus T times the
     * depth texture's step, which is unbounded where `dcoord` crosses a
     * light-view edge (NEAREST). Not bounded on the host (no mesh): at those
     * pixels and at silhouettes rho has no upper bound, so every level up to
     * 10 is reachable. */
    let t = refract([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], INDEX);
    assert!((t[0].hypot(t[1]).hypot(t[2]) - 1.0).abs() < 1e-12, "|front_refraction| = 1 face-on");
}

/* ---- the level contents --------------------------------------------------------- */

#[cfg(all(
    feature = "shaderc-online",
    not(feature = "no-shader-table"),
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod levels {
    use super::*;
    use yakogl::pvr::internal::{PVR_INTERNAL_MIPGEN3, PVR_INTERNAL_TABLE};
    use yakogl::shaderc::online::arena::Arena;
    use yakogl::shaderc::online::eval::{self, Fault, Frame, Inputs, Sink, TexLevel, TexLookup, TexelSource};
    use yakogl::shaderc::online::ir::{IoSlot, Module, TexDim};
    use yakogl::shaderc::online::lower::compile_to_ir;
    use yakogl::shaderc::online::sema::Stage;

    const MIPGEN3: &str = include_str!("../tools/shaders/internal/es/mipgen3.frag");

    /// The most a level from 3 on may differ from the knob-off chain
    /// (tests/pvr_mip_three_levels.rs `MAX_D`, measured there at 800x600).
    pub(super) const MAX_D: u8 = 1;

    /// One RGBA8 level, tightly packed.
    #[derive(Clone)]
    struct Level {
        w: u32,
        h: u32,
        px: Vec<[u8; 4]>,
    }

    impl Level {
        fn at(&self, x: i64, y: i64) -> [u8; 4] {
            let x = x.clamp(0, self.w as i64 - 1) as u32;
            let y = y.clamp(0, self.h as i64 - 1) as u32;
            self.px[(y * self.w + x) as usize]
        }
    }

    /* The model below is tests/pvr_mip_three_levels.rs's, copied (a test
     * file cannot share another's items). */

    /// The 8-bit pack: round half up.
    fn q(v: f32) -> u8 {
        (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
    }

    /// The TPU model's bilinear sample (CLAMP_TO_EDGE) at the normalised `c`.
    fn sample(l: &Level, c: [f32; 2]) -> [f32; 4] {
        let u = c[0] * l.w as f32 - 0.5;
        let v = c[1] * l.h as f32 - 0.5;
        let (i0, j0) = (u.floor(), v.floor());
        let (a, b) = (u - i0, v - j0);
        let (i0, j0) = (i0 as i64, j0 as i64);
        let t = [l.at(i0, j0), l.at(i0 + 1, j0), l.at(i0, j0 + 1), l.at(i0 + 1, j0 + 1)];
        let mut out = [0.0f32; 4];
        for (ch, o) in out.iter_mut().enumerate() {
            let g = |k: usize| t[k][ch] as f32 / 255.0;
            *o = (1.0 - a) * (1.0 - b) * g(0) + a * (1.0 - b) * g(1) + (1.0 - a) * b * g(2) + a * b * g(3);
        }
        out
    }

    /// A level kick (mipgen.frag): each texel the sample of `l` at its centre.
    fn level_kick(l: &Level, w: u32, h: u32) -> Level {
        let s = [1.0 / w as f32, 1.0 / h as f32];
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(sample(l, [(x as f32 + 0.5) * s[0], (y as f32 + 0.5) * s[1]]).map(q));
            }
        }
        Level { w, h, px }
    }

    struct Src<'a> {
        l: &'a Level,
    }

    impl TexelSource for Src<'_> {
        fn sample(&mut self, t: &TexLookup) -> Result<[u32; 4], Fault> {
            if t.sampler != 0 || t.dim != TexDim::D2 || t.level != TexLevel::Lod(0.0) {
                return Err(Fault::new(format_args!("a look-up mipgen3.frag does not make: {t:?}")));
            }
            Ok(eval::f32x4(sample(self.l, [f32::from_bits(t.coord[0]), f32::from_bits(t.coord[1])])))
        }
        fn size(&mut self, _: u8, _: TexDim, _: i32) -> Result<[u32; 4], Fault> {
            Ok([self.l.w, self.l.h, 1, 0])
        }
    }

    #[derive(Default)]
    struct Outs {
        o: [Option<[u32; 4]>; 3],
    }

    impl Sink for Outs {
        fn store(&mut self, slot: IoSlot, value: [u32; 4], _: [bool; 4]) {
            if let IoSlot::FragColor(k) = slot {
                if let Some(o) = self.o.get_mut(k as usize) {
                    *o = Some(value);
                }
            }
        }
    }

    fn sides(w: u32, h: u32, n: usize) -> Vec<(u32, u32)> {
        (0..n).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect()
    }

    /// Level 0: `normals`, what depth.frag stores (a sphere's normal,
    /// clamped to [0, 1], alpha 1, on a black clear: a smooth field with a
    /// hard silhouette, refract's kind of content), or `random` bytes, the
    /// worst case of tests/pvr_mip_three_levels.rs.
    fn level0(w: u32, h: u32, kind: &str) -> Level {
        let mut s: u32 = 0x1234_5678;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let r = rnd().to_le_bytes();
                px.push(if kind == "random" {
                    r
                } else {
                    let nx = (x as f32 + 0.5 - w as f32 / 2.0) / (0.4 * h as f32);
                    let ny = (y as f32 + 0.5 - h as f32 / 2.0) / (0.4 * h as f32);
                    let rr = nx * nx + ny * ny;
                    if rr >= 1.0 {
                        [0, 0, 0, 255]
                    } else {
                        let nz = (1.0 - rr).sqrt();
                        [q(nx.max(0.0)), q(ny.max(0.0)), q(nz), 255]
                    }
                });
            }
        }
        Level { w, h, px }
    }

    /// mipgen3.frag's render of `dst` from `src`, every pixel of `dst[0]`.
    fn three_level_render(m: &Module<'_>, src: &Level, dst: &[(u32, u32)]) -> Vec<Level> {
        let mut block = [0u32; 12];
        for (k, &(w, h)) in dst.iter().enumerate() {
            let v = [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32];
            for c in 0..4 {
                block[4 * k + c] = v[c].to_bits();
            }
        }
        let mut values = vec![[0u32; 4]; m.instrs.len()];
        let mut def = vec![[false; 4]; m.instrs.len()];
        let mut locals = vec![[0u32; 4]; m.locals.len()];
        let mut ldef = vec![[false; 4]; m.locals.len()];
        let mut out: Vec<Level> =
            dst.iter().map(|&(w, h)| Level { w, h, px: vec![[0; 4]; (w * h) as usize] }).collect();
        let mut texels = Src { l: src };
        let (w1, h1) = dst[0];
        for y in 0..h1 {
            for x in 0..w1 {
                let io = [(IoSlot::FragCoord, eval::f32x4([x as f32 + 0.5, y as f32 + 0.5, 0.5, 1.0]))];
                let mut o = Outs::default();
                let frame = Frame { values: &mut values, def: &mut def, locals: &mut locals, ldef: &mut ldef };
                eval::run_in_with_textures(m, &Inputs { io: &io, uniforms: &block }, frame, &mut o, &mut texels)
                    .unwrap_or_else(|f| panic!("pixel ({x}, {y}): {}", f.as_str()));
                for (k, lv) in out.iter_mut().enumerate() {
                    if x < lv.w && y < lv.h {
                        let v = o.o[k].unwrap_or_else(|| panic!("pixel ({x}, {y}): output {k} not written"));
                        lv.px[(y * lv.w + x) as usize] = v.map(|b| q(f32::from_bits(b)));
                    }
                }
            }
        }
        out
    }

    /// Per level of refract's 1600x1200 chain, the largest knob-on minus
    /// knob-off difference in a channel and the share of texels that differ,
    /// the worst of the two contents.
    pub(super) fn chain_diffs() -> Vec<(u8, f64)> {
        static ONCE: std::sync::OnceLock<Vec<(u8, f64)>> = std::sync::OnceLock::new();
        ONCE.get_or_init(measure).clone()
    }

    fn measure() -> Vec<(u8, f64)> {
        let rec = &PVR_INTERNAL_TABLE[PVR_INTERNAL_MIPGEN3].shader;
        assert_eq!(rec.num_outputs, 3, "mipgen3.frag writes three levels");
        let mut buf = vec![0u8; 8 << 20];
        let a = Arena::new(&mut buf);
        let lowered = compile_to_ir(&a, Stage::Fragment, MIPGEN3).unwrap_or_else(|e| panic!("{}: {}", e.line, e.msg));
        let m = &lowered.module;
        let (w, h) = TEX;
        let sz = sides(w, h, LEVELS);
        let mut worst = vec![(0u8, 0.0f64); LEVELS];
        for kind in ["normals", "random"] {
            let mut off = vec![level0(w, h, kind)];
            for l in 1..LEVELS {
                let next = level_kick(&off[l - 1], sz[l].0, sz[l].1);
                off.push(next);
            }
            let mut on: Vec<Level> = vec![off[0].clone()];
            for g in groups(LEVELS) {
                let src = on[g[0] - 1].clone();
                if g.len() == 1 {
                    on.push(level_kick(&src, sz[g[0]].0, sz[g[0]].1));
                } else {
                    let dst: Vec<(u32, u32)> = g.iter().map(|&l| sz[l]).collect();
                    on.extend(three_level_render(m, &src, &dst));
                }
            }
            assert_eq!(on.len(), LEVELS);
            for l in 1..LEVELS {
                let (mut texd, mut maxd) = (0usize, 0u8);
                for (a, b) in on[l].px.iter().zip(&off[l].px) {
                    let d = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                    texd += usize::from(d != 0);
                    maxd = maxd.max(d);
                }
                let share = texd as f64 / on[l].px.len() as f64;
                println!(
                    "1600x1200 {kind} level {l} ({}x{}): {texd} of {} texels differ, max |d| {maxd}",
                    sz[l].0,
                    sz[l].1,
                    on[l].px.len()
                );
                worst[l] = (worst[l].0.max(maxd), worst[l].1.max(share));
            }
        }
        worst
    }

    #[test]
    fn refracts_chain_with_three_levels_keeps_levels_1_and_2_and_moves_the_rest_by_1_at_most() {
        let d = chain_diffs();
        for (l, &(maxd, _)) in d.iter().enumerate().skip(1) {
            if l <= 2 {
                assert_eq!(maxd, 0, "level {l}: a TPU sample of a level both chains share");
            } else {
                assert!(maxd <= MAX_D, "level {l}: {maxd} from the knob-off chain");
            }
        }
    }
}

/// The largest shift of nasa1's coordinate, in nasa1 texels, that one LSB
/// (1/255 in each channel, either sign) of `back_normal` makes in
/// `imagecoord = refract(back_position, back_normal, 1/1.2).st * 0.5 + 0.5`,
/// over the inputs the scene can give: `back_position` in the bunny's
/// eye-space box widened by 1 (`vertex_position + front_refraction * d`,
/// |front_refraction| <= 1, d in [0, 1]), `back_normal` in [0, 1]^3 (the
/// UNORM store). A grid of 9 points an axis on both; pairs where either
/// refract is past the critical angle (0) are left out.
fn worst_shift_per_lsb() -> f64 {
    let (zn, zf) = depth_range();
    let hx = ((BUNNY_MAX[0] - BUNNY_MIN[0]) / 2.0).hypot((BUNNY_MAX[2] - BUNNY_MIN[2]) / 2.0) + 1.0;
    let hy = (BUNNY_MAX[1] - BUNNY_MIN[1]) / 2.0 + 1.0;
    let g = |lo: f64, hi: f64, k: usize| lo + (hi - lo) * k as f64 / 8.0;
    let mut best = 0.0f64;
    for (ix, iy, iz) in (0..9).flat_map(|a| (0..9).flat_map(move |b| (0..9).map(move |c| (a, b, c)))) {
        let i = [g(-hx, hx, ix), g(-hy, hy, iy), -g(zn - 1.0, zf + 1.0, iz)];
        for (nx, ny, nz) in (0..9).flat_map(|a| (0..9).flat_map(move |b| (0..9).map(move |c| (a, b, c)))) {
            let n = [g(0.0, 1.0, nx), g(0.0, 1.0, ny), g(0.0, 1.0, nz)];
            let r0 = refract(i, n, 1.0 / INDEX);
            if r0 == [0.0; 3] {
                continue;
            }
            for sg in 0..8 {
                let s = |b: usize| if sg >> b & 1 == 1 { 1.0 } else { -1.0 };
                let n2 = [0, 1, 2].map(|c| (n[c] + s(c) / 255.0).clamp(0.0, 1.0));
                let r1 = refract(i, n2, 1.0 / INDEX);
                if r1 == [0.0; 3] {
                    continue;
                }
                best = best.max((r1[0] - r0[0]).hypot(r1[1] - r0[1]) * 0.5 * NASA1_SIZE);
            }
        }
    }
    best
}

#[test]
fn the_normal_sample_moves_by_1_lsb_at_most_but_the_picture_is_not_bound_by_it() {
    /* Each level's worst difference: 0 for levels 0, 1 and 2 (the knob's
     * groups, test 1); from 3 on the measured value with shaderc-online, or
     * MAX_D (tests/pvr_mip_three_levels.rs) without it. */
    #[cfg(all(
        feature = "shaderc-online",
        not(feature = "no-shader-table"),
        not(feature = "report-es2"),
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    let d: Vec<(u8, f64)> = levels::chain_diffs();
    #[cfg(not(all(
        feature = "shaderc-online",
        not(feature = "no-shader-table"),
        not(feature = "report-es2"),
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    /* (the share of differing texels is not measured: NaN, and the mean
     * estimate below falls back to every bunny point touching one) */
    let d: Vec<(u8, f64)> = (0..LEVELS).map(|l| if l <= 2 { (0, 0.0) } else { (1, f64::NAN) }).collect();
    assert!(d[0].0 == 0 && d[1].0 == 0 && d[2].0 == 0, "levels 0..=2 cannot move: {d:?}");

    /* The NormalMap sample's change at LOD lambda: sum over levels of the
     * trilinear weight times the level's worst difference. The weights sum
     * to 1, so it is at most the largest difference of the two levels read. */
    let at = |lambda: f64| (0..LEVELS).map(|l| weight(l, lambda) * d[l].0 as f64).sum::<f64>();
    let all = (0..=10_000).map(|k| at(10.0 * k as f64 / 10_000.0)).fold(0.0, f64::max);
    let (zn, zf) = depth_range();
    let (lo, hi) = (rho_face_on(zn).log2(), rho_face_on(zf).log2());
    let face = (0..=1000).map(|k| at(lo + (hi - lo) * k as f64 / 1000.0)).fold(0.0, f64::max);
    println!("NormalMap sample change: <= {all} LSB at any LOD, <= {face:.3} LSB face-on (lambda {lo:.3}..{hi:.3})");
    assert!(all <= 1.0, "the weighted bound: {all} LSB");
    assert!(face <= hi - 2.0 + 1e-9, "face-on only level 3 moves, at weight lambda - 2: {face}");
    assert!(all <= TOLERANCE, "the NormalMap sample stays within tolerance 8");

    /* That is the sample's bound. The picture is nasa1 at a coordinate the
     * sample steers: one LSB of back_normal moves it by many texels. */
    let shift = worst_shift_per_lsb();
    let pixel = (NASA1_MAX_STEP * shift * all).min(255.0);
    println!("one LSB of back_normal moves nasa1's coordinate by up to {shift:.1} texels: a bunny pixel can move by up to {pixel:.0} levels");
    assert!(shift > 20.0 && shift < 40.0, "the refract's gain, nasa1 texels per LSB: {shift}");
    assert!(pixel > TOLERANCE, "a bunny pixel is not bound by tolerance 8 through the sample bound");

    /* What the board judge reads (five probes and the 40x30 grid's mean).
     * Coverage and depth do not depend on NormalMap, so every pixel the
     * bunny does not cover is the same byte for byte: the four corner probes
     * of the am62p reference (000000) cannot move. Probe c is on the bunny
     * and is not bounded (it already moves between runs of one image: the
     * scene is sampled at its last, time-dependent frame).
     * The mean, an estimate, not a bound: the bunny covers about
     * reference mean / nasa1 mean of the grid; a grid point moves only if
     * its trilinear footprint touches a texel that differs, and then by
     * nasa1's mean change at that shift. Face-on that is the 4 taps of
     * level 3 (levels 2 and 3 are read); at a silhouette 8 taps of any two
     * levels from 3 on, at the worst share of any of them. */
    let cover = AM62P_REFRACT_MEAN.iter().sum::<f64>() / NASA1_MEAN.iter().sum::<f64>();
    let share3 = d[3].1;
    let share = if share3.is_nan() { f64::NAN } else { d.iter().skip(3).map(|x| x.1).fold(0.0, f64::max) };
    let touch = |taps: f64, s: f64| if s.is_nan() { 1.0 } else { (taps * s).min(1.0) };
    let est_face = cover * touch(4.0, share3) * NASA1_MEAN_CHANGE_AT_34;
    let est_edge = cover * touch(8.0, share) * NASA1_MEAN_CHANGE_AT_34;
    let worst = cover * 255.0;
    println!(
        "grid mean: the bunny covers ~{:.0}% of the grid; level 3 differs in {:.4} of its texels, the worst level from 3 on in {share:.4}; estimated mean change {est_face:.2} (all face-on) to {est_edge:.2} (all at the worst level), worst case (every bunny point by 255) {worst:.0}",
        cover * 100.0,
        share3
    );
    assert!(worst > TOLERANCE, "the mean is not bound by tolerance 8 without the mesh either");
}
