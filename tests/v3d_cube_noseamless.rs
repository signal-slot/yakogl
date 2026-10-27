// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Pi 4's 25 dEQP-GLES2 cube failures on the host
//! (docs/v3d-noseamless-cube.md): the V3D 4.2 filters every cube map
//! seamlessly, and an ES 2.0-reporting context is judged against dEQP's
//! NON-seamless reference, where each face is an independent 2D texture with
//! the sampler's wrap modes (GL ES 2.0 3.7.5; ES 3.0 Appendix F.2).
//!
//! The fix is a `LodKey::cube_noseam` variant of the program: a keyed cube
//! binding's `textureCube` is compiled as a 2D-array look-up of the face the
//! compiler selects (TMUI = face), so the TMU applies the sampler's wrap
//! modes inside the face. The GL core asks for the key only in an ES 2.0
//! build (`REPORT_ES2`) and only for a cube sampler whose filters can see the
//! difference; an unkeyed program is the hardware's seamless `TMUSCM` look-up,
//! unchanged in every build (`an_unkeyed_program_is_unchanged`).
//!
//! The programs run on the QPU interpreter (`v3d::sim`, whose TMU model
//! filters a cube look-up as the hardware does and a layer look-up as one
//! 2D texture) against a NON-seamless reference computed HERE from the same
//! texels, never by the model: Table 3.21's face and (s, t), dEQP's
//! `sampleLinear2D` with the sampler's wrap modes, mipmap-LINEAR blends.
//!
//! A keyed fragment look-up computes its own level from the derivatives of
//! the direction (FDX / FDY) and retires with TMUSLOD, so the fragment
//! cases run on a full mip chain; the interpreter's derivatives need its
//! assumed quad layout (`Env::quads`), which the tests' inputs follow.

// The V3D compiler exists only in a gpu-v3d build (tools/check.sh runs the
// gpu-v3d rows with and without report-es2).
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::LodKey;

/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

/// The key a draw asks for a cube sampler at binding 0 under ES 2.0.
const KEYED: LodKey = LodKey { cube_noseam: 1, ..LodKey::NONE };

/// dEQP's vertex cube cases, in shape: the direction is an attribute, the
/// look-up a vertex-stage `textureCube` (ES 1.00: level 0), the result a
/// varying the fragment shader passes through.
const VS: &str = "attribute vec4 a_dir;\nuniform samplerCube u_cube;\nvarying vec4 v_color;\n\
                  void main() { v_color = textureCube(u_cube, a_dir.xyz); gl_Position = a_dir; }\n";
/// `textureCubeLod` at the level the single uniform gives.
const VS_LOD: &str =
    "attribute vec4 a_dir;\nuniform samplerCube u_cube;\nuniform float u_lod;\nvarying vec4 v_color;\n\
                      void main() { v_color = textureCubeLod(u_cube, a_dir.xyz, u_lod); gl_Position = a_dir; }\n";
/// The direction computed by a division, as dEQP's projected cases do.
const VS_PROJ: &str = "attribute vec4 a_dir;\nuniform samplerCube u_cube;\nvarying vec4 v_color;\n\
                       void main() { v_color = textureCube(u_cube, a_dir.xyz / a_dir.w); gl_Position = a_dir; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

/// A fragment look-up with a bias, at a constant direction.
const VS_PASS: &str = "attribute vec4 a_dir;\nvoid main() { gl_Position = a_dir; }\n";
const FS_CONST_BIAS: &str = "precision mediump float;\nuniform samplerCube u_cube;\n\
                             void main() { gl_FragColor = textureCube(u_cube, vec3(1.0, 0.0, -0.875), 0.5); }\n";

const N: usize = 4;
const LEVELS: usize = 3;

/// Texel channel `c` of (level, face, i, j): distinct per face, column and
/// row, and exact in f16 (multiples of 1/16 below 1).
fn texel(l: usize, f: usize, i: usize, j: usize) -> [f32; 4] {
    core::array::from_fn(|c| ((f * 7 + i * 3 + j * 5 + c * 2 + l * 11) % 16) as f32 / 16.0)
}

fn levels(count: usize) -> Vec<Level> {
    (0..count)
        .map(|l| {
            let n = N >> l;
            Level {
                width: n as u32,
                height: n as u32,
                faces: (0..6).map(|f| (0..n * n).map(|k| texel(l, f, k % n, k / n)).collect()).collect(),
            }
        })
        .collect()
}

fn texture(count: usize) -> Texture {
    Texture { cube: true, min: MinFilter::Linear, levels: levels(count) }
}

/// ES 2.0 3.7.5 Table 3.21, written out: the face (+X -X +Y -Y +Z -Z = 0..5)
/// of the largest component and (s, t) of the direction.
fn face_st(d: [f32; 3]) -> (usize, f32, f32) {
    let [x, y, z] = d;
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, sc, tc, ma) = if ax > ay && ax > az {
        if x > 0.0 {
            (0, -z, -y, ax)
        } else {
            (1, z, -y, ax)
        }
    } else if ay > az {
        if y > 0.0 {
            (2, x, z, ay)
        } else {
            (3, x, -z, ay)
        }
    } else if z > 0.0 {
        (4, x, -y, az)
    } else {
        (5, -x, -y, az)
    };
    (face, (sc / ma + 1.0) / 2.0, (tc / ma + 1.0) / 2.0)
}

/// tcuTexture.cpp `wrap`, the three GL ES 2.0 modes.
fn wrap(mode: Wrap, c: i32, n: i32) -> usize {
    let imod = |a: i32, b: i32| ((a % b) + b) % b;
    let mirror = |a: i32| if a >= 0 { a } else { -(1 + a) };
    (match mode {
        Wrap::ClampToEdge => c.clamp(0, n - 1),
        Wrap::Repeat => imod(c, n),
        Wrap::MirroredRepeat => (n - 1) - mirror(imod(c, 2 * n) - n),
    }) as usize
}

/// One level of face `face` read at (s, t): dEQP's `sampleLinear2D` (x0 =
/// floor(u - 0.5), a = frac(u - 0.5)), the four taps wrapped by the sampler,
/// or the NEAREST texel.
fn sample_level(lv: &[Level], l: usize, face: usize, s: f32, t: f32, filter: Filter, sp: &Sampler) -> [f32; 4] {
    let n = (N >> l) as i32;
    let tex = &lv[l].faces[face];
    let (u, v) = (s * n as f32, t * n as f32);
    if filter == Filter::Nearest {
        let at = |c: f32| (c.floor() as i32).clamp(0, n - 1) as usize;
        return tex[at(v) * n as usize + at(u)];
    }
    let (x0, y0) = ((u - 0.5).floor(), (v - 0.5).floor());
    let (a, b) = (u - 0.5 - x0, v - 0.5 - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |i: i32, j: i32| tex[wrap(sp.wrap_t, j, n) * n as usize + wrap(sp.wrap_s, i, n)];
    let (p00, p10, p01, p11) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    core::array::from_fn(|c| {
        p00[c] * (1.0 - a) * (1.0 - b) + p10[c] * a * (1.0 - b) + p01[c] * (1.0 - a) * b + p11[c] * a * b
    })
}

/// The NON-seamless reference of an explicit-level look-up (GL ES 2.0 3.7.7,
/// 3.7.8): the face of `d` as an independent 2D texture; magnified (lod <=
/// 0) with the mag filter at level 0, else the min filter, a mipmap-LINEAR
/// one blending floor(lod) and the next level by frac(lod).
fn reference(count: usize, sp: &Sampler, d: [f32; 3], lod: f32) -> [f32; 4] {
    let lv = levels(count);
    let (face, s, t) = face_st(d);
    if lod <= 0.0 {
        return sample_level(&lv, 0, face, s, t, sp.mag, sp);
    }
    let filter = sp.min.within_level();
    match sp.min {
        MinFilter::Nearest | MinFilter::Linear => sample_level(&lv, 0, face, s, t, filter, sp),
        MinFilter::NearestMipmapLinear | MinFilter::LinearMipmapLinear => {
            let l0 = (lod.floor() as usize).min(count - 1);
            let l1 = (l0 + 1).min(count - 1);
            let w = if l0 == l1 { 0.0 } else { lod - lod.floor() };
            let (a, b) = (sample_level(&lv, l0, face, s, t, filter, sp), sample_level(&lv, l1, face, s, t, filter, sp));
            core::array::from_fn(|c| a[c] * (1.0 - w) + b[c] * w)
        }
        _ => unreachable!("not used by these tests"),
    }
}

/// The SEAMLESS value of the test's edge direction (1, 0, -0.875): the tap
/// past +X's s = 1 edge reads -Z's s = 0 edge column at the same row (ES 3.0
/// 3.8.9.1), hand-computed from [`texel`]: columns 3 (weight 3/4) of rows 1
/// and 2 of +X and column 0 of those rows of -Z (weight 1/4).
fn seamless_edge() -> [f32; 4] {
    let at = |f: usize, i: usize, j: usize| texel(0, f, i, j);
    core::array::from_fn(|c| {
        0.375 * at(0, 3, 1)[c] + 0.375 * at(0, 3, 2)[c] + 0.125 * at(5, 0, 1)[c] + 0.125 * at(5, 0, 2)[c]
    })
}

fn linear(wrap_s: Wrap, wrap_t: Wrap) -> Sampler {
    Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s, wrap_t }
}

/// Directions on every face, most of them within a quarter texel of an
/// edge (+X's is the edge case of the doc), none near a tie; scaled so that
/// the normalisation matters.
const DIRS: [[f32; 3]; 8] = [
    [1.0, 0.0, -0.875],
    [1.0, -0.875, 0.0],
    [-3.7, 0.9, 3.3],
    [0.5, 4.0, -3.6],
    [0.2, -2.0, 1.9],
    [3.4, -1.3, 3.9],
    [-1.8, 0.7, -2.0],
    [-0.9, 0.0, 0.4],
];

fn close(got: [f32; 4], want: [f32; 4], what: &str) {
    for c in 0..4 {
        assert!((got[c] - want[c]).abs() <= 2e-3, "{what}: channel {c}: got {got:?}, want {want:?}");
    }
}

/// `vs` + `fs` compiled with the stages' keys, handed to `f`.
fn with_program(vs: &str, fs: &str, vs_key: LodKey, fs_key: LodKey, f: impl FnOnce(&V3dProgram<'_>)) {
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, vs_key, fs_key, &a)
        .expect("the program compiles");
    f(&p);
}

fn text(c: &yakogl::shaderc::online::v3d::chain::Code<'_>) -> Vec<String> {
    c.instrs
        .iter()
        .map(|i| {
            let mut s = String::new();
            write_instr(&mut s, i).unwrap();
            s
        })
        .collect()
}

/// The vertex program run on the interpreter with every lane's `a_dir` =
/// `dir` (w as given) and sampler 0 = `sampler` on `tex`; `user` are the
/// user uniforms' values. The components of `v_color` (the last four VPM
/// output rows, as f32) of lane 0.
fn run_vs(p: &V3dProgram<'_>, tex: &Texture, dir: [f32; 4], sampler: Sampler, user: &[f32]) -> [f32; 4] {
    let uniforms: Vec<u32> =
        p.vs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                1 => user[u.data as usize].to_bits(),
                2..=5 => 1.0f32.to_bits(),
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    assert_eq!(p.attributes.len(), 1, "one attribute: {:?}", p.attributes);
    let rec = p.attributes[0].1.expect("a_dir is read") as usize;
    assert_eq!(p.vs.vattr_sizes[rec], 4, "vec4: {:?}", p.vs.vattr_sizes);
    let rows: Vec<[u32; LANES]> = dir.iter().map(|c| [c.to_bits(); LANES]).collect();
    let env = Env { textures: vec![tex.clone()], samplers: vec![sampler], threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &uniforms, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .unwrap_or_else(|e| panic!("the vertex program runs: {e}\n{:#?}", text(&p.vs.code)));
    assert_eq!(o.tmu_lookups, 1, "one look-up");
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
    let v = &out[out.len() - 4..];
    core::array::from_fn(|c| f32::from_bits(v[c][0]))
}

fn xyz1(d: [f32; 3]) -> [f32; 4] {
    [d[0], d[1], d[2], 1.0]
}

/// The design item's reproduction, now passing: the keyed program reads the
/// NON-seamless value for every wrap pair, on every face, where the unkeyed
/// one (the hardware's cube look-up, whatever the wrap) read the seamless
/// blend. The reference is the test's own.
#[test]
fn a_keyed_cube_lookup_is_non_seamless_for_every_wrap_pair() {
    let tex = texture(1);
    let modes = [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat];
    with_program(VS, FS, KEYED, LodKey::NONE, |p| {
        let l = text(&p.vs.code);
        assert!(l.iter().any(|t| t.contains("tmui")), "the face goes to TMUI: {l:#?}");
        assert!(!l.iter().any(|t| t.contains("tmuscm")), "no cube look-up: {l:#?}");
        for ws in modes {
            for wt in modes {
                for d in DIRS {
                    let sp = linear(ws, wt);
                    close(
                        run_vs(p, &tex, xyz1(d), sp, &[]),
                        reference(1, &sp, d, 0.0),
                        &format!("{ws:?}/{wt:?} {d:?}"),
                    );
                }
            }
        }
    });
}

/// The wrap modes on the +X face's s = 1 edge: the tap past it clamps,
/// mirrors (both the edge column) or repeats (column 0 of the SAME face),
/// each different from the seamless value of the hardware's cube look-up.
#[test]
fn clamp_mirror_and_repeat_differ_on_a_face_edge_and_from_the_seamless_value() {
    let tex = texture(1);
    let d = DIRS[0];
    let mut seen = Vec::new();
    with_program(VS, FS, KEYED, LodKey::NONE, |p| {
        for w in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            let got = run_vs(p, &tex, xyz1(d), linear(w, w), &[]);
            assert_ne!(got, seamless_edge(), "{w:?} is not the seamless blend");
            seen.push(got);
        }
    });
    assert_ne!(seen[0], seen[1], "CLAMP and REPEAT differ");
    assert_eq!(seen[0], seen[2], "CLAMP and MIRRORED_REPEAT agree one texel past the edge");
    /* the unkeyed program: seamless whatever the wrap (the defect, in an ES 3 build the rule) */
    with_program(VS, FS, LodKey::NONE, LodKey::NONE, |p| {
        assert!(text(&p.vs.code).iter().any(|t| t.contains("tmuscm")));
        for w in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            close(run_vs(p, &tex, xyz1(d), linear(w, w), &[]), seamless_edge(), &format!("unkeyed {w:?}"));
        }
    });
}

/// A vertex-stage `textureCubeLod` at an explicit level, LINEAR_MIPMAP_LINEAR
/// and NEAREST_MIPMAP_LINEAR with mag LINEAR (dEQP's texture.vertex.cube
/// filtering): the two levels blended by frac(lod), each level read
/// non-seamlessly; also magnified (lod < 0).
#[test]
fn a_vertex_explicit_level_blends_two_non_seamless_levels() {
    let tex = texture(LEVELS);
    with_program(VS_LOD, FS, KEYED, LodKey::NONE, |p| {
        let l = text(&p.vs.code);
        assert!(l.iter().any(|t| t.contains("tmuslod")) && l.iter().any(|t| t.contains("tmui")), "{l:#?}");
        assert!(!l.iter().any(|t| t.contains("tmuscm")), "no cube look-up: {l:#?}");
        for min in [MinFilter::LinearMipmapLinear, MinFilter::NearestMipmapLinear, MinFilter::Linear] {
            for w in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
                let sp = Sampler { mag: Filter::Linear, min, wrap_s: w, wrap_t: w };
                for lod in [-1.0f32, 0.25, 1.0, 1.5] {
                    /* NEAREST within a level is refused by the model on a texel edge
                     * (DIRS[0] and [1] sit at 0.5): those two read LINEAR only */
                    let dirs: &[[f32; 3]] = if min == MinFilter::NearestMipmapLinear {
                        &[DIRS[2], DIRS[5]]
                    } else {
                        &[DIRS[0], DIRS[1], DIRS[2], DIRS[5]]
                    };
                    for &d in dirs {
                        let got = run_vs(p, &tex, xyz1(d), sp, &[lod]);
                        close(got, reference(LEVELS, &sp, d, lod), &format!("{min:?} {w:?} lod {lod} {d:?}"));
                    }
                }
            }
        }
    });
}

/// A direction computed by a division (dEQP's projected cases): the face is
/// chosen on the divided direction.
#[test]
fn a_projected_direction_selects_its_face_after_the_division() {
    let tex = texture(1);
    with_program(VS_PROJ, FS, KEYED, LodKey::NONE, |p| {
        for w in [Wrap::ClampToEdge, Wrap::Repeat] {
            let sp = linear(w, w);
            for d in DIRS {
                let q = 2.5;
                let got = run_vs(p, &tex, [d[0] * q, d[1] * q, d[2] * q, q], sp, &[]);
                close(got, reference(1, &sp, d, 0.0), &format!("{w:?} {d:?}"));
            }
        }
    });
}

/// The fragment cases' texture: `FN` x `FN` faces, a full chain, each
/// (level, face) one colour (dEQP's mipmap.cube fill, in shape), exact in
/// f16: what a pixel reads names its level (and the blend of two).
const FN: usize = 16;
const FLEVELS: usize = 5;

fn solid(l: usize, f: usize) -> [f32; 4] {
    [l as f32 / 8.0, f as f32 / 8.0, 0.25, 1.0]
}

fn solid_texture() -> Texture {
    Texture {
        cube: true,
        min: MinFilter::LinearMipmapLinear,
        levels: (0..FLEVELS)
            .map(|l| {
                let n = FN >> l;
                Level { width: n as u32, height: n as u32, faces: (0..6).map(|f| vec![solid(l, f); n * n]).collect() }
            })
            .collect(),
    }
}

/// dEQP's `computeCubeLodFromDerivates` (glsTextureTestUtil.cpp, LODMODE_EXACT)
/// for direction `d` and its screen derivatives, in f64: the face by the
/// largest component, du/dx = W/2 (dsc/dx ma - sc |dma/dx|) / ma^2, and
/// log2 of the larger of the two derivative vectors' lengths.
fn deqp_cube_lod(d: [f32; 3], dx: [f32; 3], dy: [f32; 3]) -> f64 {
    let (face, _, _) = face_st(d);
    let (ma, sn, tn) = match face {
        0 | 1 => (0, 2, 1),
        2 | 3 => (1, 0, 2),
        _ => (2, 0, 1),
    };
    let w = FN as f64;
    let g = |c: [f32; 3]| -> (f64, f64) {
        let (sc, tc, m) = (d[sn] as f64, d[tn] as f64, (d[ma] as f64).abs());
        let (scd, tcd, md) = (c[sn] as f64, c[tn] as f64, (c[ma] as f64).abs());
        (w * 0.5 * (scd * m - sc * md) / (m * m), w * 0.5 * (tcd * m - tc * md) / (m * m))
    };
    let ((ux, vx), (uy, vy)) = (g(dx), g(dy));
    (ux * ux + vx * vx).sqrt().max((uy * uy + vy * vy).sqrt()).log2()
}

/// The non-seamless reference of [`solid_texture`] at level of detail `lod`
/// (GL ES 2.0 3.7.7-3.7.9, mag LINEAR or NEAREST alike on a solid face):
/// level 0 when magnified, else LINEAR_MIPMAP_LINEAR's blend of floor(lod)
/// and the next level, or LINEAR_MIPMAP_NEAREST's level ceil(lod + 0.5) - 1,
/// clamped to the chain.
fn solid_reference(min: MinFilter, d: [f32; 3], lod: f64) -> [f32; 4] {
    let (face, _, _) = face_st(d);
    let top = FLEVELS - 1;
    if lod <= 0.0 {
        return solid(0, face);
    }
    match min {
        MinFilter::LinearMipmapLinear => {
            let l0 = (lod.floor() as usize).min(top);
            let l1 = (l0 + 1).min(top);
            let w = if l0 == l1 { 0.0 } else { (lod - lod.floor()) as f32 };
            let (a, b) = (solid(l0, face), solid(l1, face));
            core::array::from_fn(|c| a[c] * (1.0 - w) + b[c] * w)
        }
        MinFilter::LinearMipmapNearest => solid((((lod + 0.5).ceil() as usize).max(1) - 1).min(top), face),
        _ => unreachable!("not used by these tests"),
    }
}

/// The fragment shaders: dEQP's textureCube of an interpolated direction,
/// implicit level (mipmap.cube.basic) and with a bias (mipmap.cube.bias).
const VS_DIR: &str =
    "attribute vec4 a_dir;\nvarying vec3 v_dir;\nvoid main() { v_dir = a_dir.xyz; gl_Position = a_dir; }\n";
const FS_IMPLICIT: &str = "precision mediump float;\nuniform samplerCube u_cube;\nvarying vec3 v_dir;\n\
                           void main() { gl_FragColor = textureCube(u_cube, v_dir); }\n";
const FS_BIAS: &str = "precision mediump float;\nuniform samplerCube u_cube;\nuniform float u_bias;\n\
                       varying vec3 v_dir;\nvoid main() { gl_FragColor = textureCube(u_cube, v_dir, u_bias); }\n";

/// The per-pixel change of the direction across the +X face: dEQP's quad
/// spans the face, s and t move by K / 2 a pixel (16 x 2.4 / 16: level
/// 1.263 inside the face).
const K: f32 = 0.3;

/// The direction of the pixel at (px, py) of the +X-face quad, extended
/// past its edge: z = 1 at px = -0.5, so the pixels at px <= -1 select +Z.
fn quad_dir(px: i32, py: i32) -> [f32; 3] {
    [1.0, 1.0 - (py as f32 + 0.5) * K, 1.0 - (px as f32 + 0.5) * K]
}

/// The 16 lanes as four 2x2 quads (the interpreter's `Env::quads` layout:
/// lane 4q + i at column i & 1, row i >> 1 of quad q), each quad's top-left
/// pixel and whether its direction follows [`quad_dir`] (else a constant
/// one): q0 inside +X; q1 straddling +X's z = 1 edge, its left column (a
/// helper pixel in dEQP's draw of the face) on +Z; q2 entirely on +Z (the
/// major axis z varies there); q3 a constant direction (zero derivatives).
const QUADS: [(i32, i32, bool); 4] = [(2, 2, true), (-1, 4, true), (-4, 0, true), (0, 0, false)];
const CONSTANT_DIR: [f32; 3] = [1.0, 0.3, 0.2];

/// Each lane's direction and its screen derivatives (exact: linear).
fn lanes() -> Vec<([f32; 3], [f32; 3], [f32; 3])> {
    (0..LANES)
        .map(|l| {
            let (ox, oy, moving) = QUADS[l / 4];
            let (px, py) = (ox + (l & 1) as i32, oy + ((l >> 1) & 1) as i32);
            if moving {
                (quad_dir(px, py), [0.0, 0.0, -K], [0.0, -K, 0.0])
            } else {
                (CONSTANT_DIR, [0.0; 3], [0.0; 3])
            }
        })
        .collect()
}

/// The fragment program run on the interpreter over [`lanes`], with the
/// quad layout assumed (`Env::quads`): every lane's RGBA.
fn run_fs(p: &V3dProgram<'_>, tex: &Texture, sp: Sampler, user: &[f32]) -> Vec<[f32; 4]> {
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                1 => user[u.data as usize].to_bits(),
                /* QUNIFORM_TEXTURE_WIDTH of binding 0: the face size */
                44 if u.data == 0 => FN as u32,
                /* QUNIFORM_FB_WPOS_Y_SCALE: a window target (dFdy's sign) */
                0x1_0000 => (-1.0f32).to_bits(),
                c => panic!("fragment uniform contents {c} (data {}) not modelled by this test", u.data),
            })
            .collect();
    let ls = lanes();
    let one = [1.0f32.to_bits(); LANES];
    let z = [0u32; LANES];
    let varyings =
        (0..3).map(|c| sim::Varying { vary: core::array::from_fn(|l| ls[l].0[c].to_bits()), c: z }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z, x: z, y: z, back_facing: [false; LANES], varyings };
    let env =
        Env { textures: vec![tex.clone()], samplers: vec![sp], threads: p.fs.threads, quads: true, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}\n{:#?}", text(&p.fs.code)));
    let h = |x: u32| sim::half_to_f32(x as u16);
    (0..LANES)
        .map(|l| {
            let word = |k: usize| o.tlb[k].value[l].expect("a defined value");
            [h(word(0) & 0xffff), h(word(0) >> 16), h(word(1) & 0xffff), h(word(1) >> 16)]
        })
        .collect()
}

/// dEQP's mipmap.cube.{basic,bias}.linear_* and texture.size.cube.15x15_*
/// (the Pi 4's last nine dEQP-GLES2 failures at a27d69c0): a fragment
/// `textureCube` on a keyed (non-seamless) binding takes its level from
/// the derivatives of the DIRECTION, per pixel, on the pixel's own face,
/// as dEQP's reference does: the program computes it (FDX / FDY, the face
/// size word) and retires the look-up with TMUSLOD. The TMU's own level,
/// from the quad's differences of the per-face (s, t), was what the board
/// ran: in quad q1, whose left column selects +Z, the jump across the face
/// edge made the level of the +X pixels the smallest level's (mipmap.cube:
/// the edge pixels of every cell) or a minification where GL magnifies
/// (texture.size 15x15, min LINEAR, mag NEAREST: LINEAR on the edge row).
/// Every lane is checked against [`deqp_cube_lod`] and
/// [`solid_reference`], with the implicit level, with biases (one to below
/// zero, one past the chain) and with both LINEAR mip filters.
#[test]
fn a_fragment_lookup_takes_each_pixels_level_from_the_direction() {
    let tex = solid_texture();
    let ls = lanes();
    for (fs, biases) in [(FS_IMPLICIT, &[0.0f32][..]), (FS_BIAS, &[0.6, -3.0, 10.0][..])] {
        with_program(VS_DIR, fs, LodKey::NONE, KEYED, |p| {
            let l = text(&p.fs.code);
            for w in ["fdx", "fdy", "tmui", "tmub", "tmuslod"] {
                assert!(l.iter().any(|t| t.contains(w)), "{w}: {l:#?}");
            }
            assert!(!l.iter().any(|t| t.contains("tmuscm") || t.contains("tmus ") || t.ends_with("tmus")), "{l:#?}");
            assert!(p.fs.code.uniforms.iter().any(|u| (u.contents, u.data) == (44, 0)), "the face size word");
            for &bias in biases {
                for min in [MinFilter::LinearMipmapLinear, MinFilter::LinearMipmapNearest] {
                    /* mag LINEAR (dEQP's is NEAREST): equal on a solid face,
                     * and the model refuses NEAREST on a texel edge */
                    let sp = Sampler { mag: Filter::Linear, min, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };
                    let got = run_fs(p, &tex, sp, &[bias]);
                    for (lane, (d, dx, dy)) in ls.iter().enumerate() {
                        let lod = deqp_cube_lod(*d, *dx, *dy) + f64::from(bias);
                        close(
                            got[lane],
                            solid_reference(min, *d, lod),
                            &format!("{min:?} bias {bias} lane {lane} {d:?} lod {lod}"),
                        );
                    }
                }
            }
        });
    }
    /* the quad q1 pins the straddle: its +X pixels sit at level 1.263, not
     * at the level the per-face (s, t) difference would give */
    let lod = deqp_cube_lod(ls[5].0, ls[5].1, ls[5].2);
    assert!((lod - 2.4f64.log2()).abs() < 1e-5, "{lod}");
    assert_eq!(face_st(ls[4].0).0, 4, "q1's left column is on +Z");
    assert_eq!(face_st(ls[5].0).0, 0, "q1's right column is on +X");
}

/// The level reaches the clamp: a constant direction (q3, zero
/// derivatives) magnifies (log2(0) clamped to -16), at level 0 for every
/// bias; the TMU reads the face with the sampler's wrap there too.
#[test]
fn a_fragment_lookup_with_a_constant_direction_magnifies() {
    let tex = texture(1);
    with_program(VS_DIR, FS_BIAS, LodKey::NONE, KEYED, |p| {
        for w in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            let sp = Sampler { mag: Filter::Linear, min: MinFilter::LinearMipmapLinear, wrap_s: w, wrap_t: w };
            for bias in [0.0f32, 3.0] {
                let got = run_fs(p, &tex, sp, &[bias]);
                for (lane, v) in got.iter().enumerate().skip(12) {
                    close(*v, reference(1, &sp, CONSTANT_DIR, 0.0), &format!("{w:?} bias {bias} lane {lane}"));
                }
            }
        }
    });
}

/// A key for a binding the program does not read as a cube changes nothing,
/// and no key is the hardware's cube look-up; NEAREST cubes are never keyed
/// (the GL core's `stage_lod_key`), so their program is the unkeyed one.
/// Golden hashes (FNV-1a of the code and uniform streams of the vertex,
/// fragment and coordinate programs) taken from the tree BEFORE the
/// non-seamless change (commit 0252e9d): the unkeyed compile is
/// byte-identical, in this build and in the other (the hash does not depend
/// on `report-es2`).
#[test]
fn an_unkeyed_program_is_unchanged() {
    fn fnv(h: &mut u64, x: u64) {
        for b in x.to_le_bytes() {
            *h ^= b as u64;
            *h = h.wrapping_mul(0x100000001b3);
        }
    }
    let pass = "attribute vec4 a_dir;\nvarying vec4 v_d;\nvoid main() { v_d = a_dir; gl_Position = a_dir; }\n";
    let cases: [(&str, String, &str, u64); 4] = [
        ("vs-cube", VS.to_string(), FS, 0xcb9920d97e8dbde9),
        (
            "fs-cube-bias",
            pass.to_string(),
            "precision mediump float;\nvarying vec4 v_d;\nuniform samplerCube u_cube;\nuniform float u_b;\n\
             void main() { gl_FragColor = textureCube(u_cube, v_d.xyz, u_b); }\n",
            0x11f9c323ba17211f,
        ),
        (
            "vs-cube-lod",
            "attribute vec4 a_dir;\nuniform samplerCube u_cube;\nuniform float u_l;\nvarying vec4 v_color;\n\
             void main() { v_color = textureCubeLod(u_cube, a_dir.xyz, u_l); gl_Position = a_dir; }\n"
                .to_string(),
            FS,
            0x137016607c7fd7b8,
        ),
        (
            "fs-cube-2d",
            pass.to_string(),
            "precision mediump float;\nvarying vec4 v_d;\nuniform samplerCube u_cube;\nuniform sampler2D u_2d;\n\
             void main() { gl_FragColor = textureCube(u_cube, v_d.xyz) + texture2D(u_2d, v_d.xy); }\n",
            0x9d127fcf6ec38ed0,
        ),
    ];
    for (name, vs, fs, want) in cases {
        /* a key naming a binding the program has no cube at is no key */
        for key in [LodKey::NONE, LodKey { cube_noseam: 1 << 7, ..LodKey::NONE }] {
            with_program(&vs, fs, key, key, |p| {
                let mut h = 0xcbf29ce484222325u64;
                for c in [&p.fs.code, &p.vs.code, &p.cs.code] {
                    c.words.iter().for_each(|w| fnv(&mut h, *w));
                    for u in c.uniforms {
                        fnv(&mut h, u.contents as u64);
                        fnv(&mut h, u.data as u64);
                    }
                }
                assert_eq!(h, want, "{name}: the unkeyed program changed ({h:016x})");
            });
        }
    }
}

/// The cube rule is what the key asks, in the stage the key is given for:
/// a fragment key does not touch the vertex program, and the reverse.
#[test]
fn a_key_changes_only_its_own_stage() {
    with_program(VS, FS, LodKey::NONE, KEYED, |p| {
        assert!(text(&p.vs.code).iter().any(|t| t.contains("tmuscm")), "the vertex stage was not keyed");
    });
    with_program(VS_PASS, FS_CONST_BIAS, KEYED, LodKey::NONE, |p| {
        assert!(text(&p.fs.code).iter().any(|t| t.contains("tmuscm")), "the fragment stage was not keyed");
    });
}
