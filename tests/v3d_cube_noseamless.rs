// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Pi 4's 25 dEQP-GLES2 cube failures, reproduced on the host
//! (docs/v3d-noseamless-cube.md): the V3D 4.2 filters every cube map
//! seamlessly, and an ES 2.0-reporting context is judged against dEQP's
//! NON-seamless reference, where each face is an independent 2D texture with
//! the sampler's wrap modes (GL ES 2.0 3.7.5; ES 3.0 Appendix F.2).
//!
//! Our compiler's own vertex program for `textureCube` runs on the QPU
//! interpreter (`v3d::sim`, whose TMU model filters a cube lookup as the
//! hardware does: seamless, wrap modes ignored) against two references
//! computed here from the same texels: the seamless one and ES 2.0's
//! per-face bilinear with CLAMP_TO_EDGE / REPEAT / MIRRORED_REPEAT.
//!
//! Two tests:
//! * `the_compiled_cube_lookup_is_seamless_today` pins what the compiler
//!   emits NOW (a TMUSCM cube lookup) and the seamless value it yields, in
//!   every build. In a build that does not report ES 2.0 this is the right
//!   answer and stays. In a report-es2 build it is the defect: the
//!   assertion carries the wrong-for-ES2 value on purpose so the tree stays
//!   green; item v3d-cube-fix flips it.
//! * `an_es2_build_samples_a_cube_non_seamlessly` is the target: in a
//!   report-es2 build the emitted program must read the non-seamless value.
//!   `#[ignore]` until v3d-cube-fix lands (its reason names the item).
//!
//! The texture is the 4x4 cube `sim.rs` and `texref.rs` use for the same
//! seam (face +X's texel (i, j) is (1, i/4, j/4, 1), face -Z flat blue), the
//! direction (1, 0, -0.875): on +X, u = 3.75, so LINEAR blends columns 3 and
//! 4 (the latter past the edge) 3:1, and rows 1 and 2 equally. Every value is
//! exact in f16, the TMU's 16-bit return.

// The V3D compiler exists only in a gpu-v3d build (tools/check.sh runs the
// gpu-v3d rows with and without report-es2).
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{Face, Filter, Level, MinFilter, Sampler, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::sim::{self, Env, Inputs, Texture, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

/// dEQP's vertex cube cases, in shape: the direction is an attribute, the
/// look-up a vertex-stage `textureCube` (ES 1.00: level 0), the result a
/// varying the fragment shader passes through.
const VS: &str = "attribute vec4 a_dir;\nuniform samplerCube u_cube;\nvarying vec4 v_color;\n\
                  void main() { v_color = textureCube(u_cube, a_dir.xyz); gl_Position = a_dir; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

const N: usize = 4;

/// The 4x4 cube: +X's texel (i, j) is (1, i/4, j/4, 1); -Z (which adjoins
/// +X's s = 1 edge) is flat (0, 0, 1, 1); the other faces are flat f/8.
fn faces() -> Vec<Vec<[f32; 4]>> {
    (0..6)
        .map(|f| match f {
            0 => (0..N * N).map(|k| [1.0, (k % N) as f32 / 4.0, (k / N) as f32 / 4.0, 1.0]).collect(),
            5 => vec![[0.0, 0.0, 1.0, 1.0]; N * N],
            _ => vec![[f as f32 / 8.0; 4]; N * N],
        })
        .collect()
}

fn texture() -> Texture {
    Texture {
        cube: true,
        min: MinFilter::Linear,
        levels: vec![Level { width: N as u32, height: N as u32, faces: faces() }],
    }
}

/// LINEAR both ways, `wrap` on S and T.
fn linear(wrap: Wrap) -> Sampler {
    Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s: wrap, wrap_t: wrap }
}

/// The direction under test: +X, face coordinates (0.9375, 0.5).
const DIR: [f32; 3] = [1.0, 0.0, -0.875];

/// ES 2.0 3.7.5's face selection and Table 3.21's (s, t), by hand for the
/// test's direction: the major axis is +x, sc = -z, tc = -y, ma = x.
fn es2_face_coords(d: [f32; 3]) -> (Face, f32, f32) {
    let [x, y, z] = d;
    assert!(x > 0.0 && x.abs() > y.abs() && x.abs() > z.abs(), "the test's direction is on +X");
    (Face::PosX, ((-z / x) + 1.0) / 2.0, ((-y / x) + 1.0) / 2.0)
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

/// The NON-seamless reference, ES 2.0's rule written out here (not the
/// model's): the selected face as a 2D texture, dEQP's `sampleLinear2D`
/// (x0 = floor(u - 0.5), a = frac(u - 0.5)) with the four taps wrapped by
/// the sampler's modes.
fn non_seamless_reference(d: [f32; 3], mode: Wrap) -> [f32; 4] {
    let (face, s, t) = es2_face_coords(d);
    let tex = faces();
    let n = N as i32;
    let (u, v) = (s * N as f32, t * N as f32);
    let (x0, y0) = ((u - 0.5).floor(), (v - 0.5).floor());
    let (a, b) = (u - 0.5 - x0, v - 0.5 - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |i: i32, j: i32| tex[face.index()][wrap(mode, j, n) * N + wrap(mode, i, n)];
    let (p00, p10, p01, p11) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    core::array::from_fn(|c| {
        p00[c] * (1.0 - a) * (1.0 - b) + p10[c] * a * (1.0 - b) + p01[c] * (1.0 - a) * b + p11[c] * a * b
    })
}

/// The SEAMLESS reference for the test's direction, written out (ES 3.0
/// 3.8.9.1, dEQP's `getCubeLinearSamples`): the same bilinear weights as
/// [`non_seamless_reference`], but a tap past +X's s = 1 edge reads the
/// texel of the neighbouring face, -Z, at its s = 0 edge column and the
/// same row. For this direction the taps are columns 3 and 4 (the latter
/// past the edge) of rows 1 and 2.
fn seamless_reference(d: [f32; 3]) -> [f32; 4] {
    let (face, s, t) = es2_face_coords(d);
    let tex = faces();
    let n = N as i32;
    let (u, v) = (s * N as f32, t * N as f32);
    let (x0, y0) = ((u - 0.5).floor(), (v - 0.5).floor());
    let (a, b) = (u - 0.5 - x0, v - 0.5 - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    assert!(0 <= y0 && y0 + 1 < n && 0 <= x0 && x0 + 1 == n, "the s = 1 edge only: rows inside, column 4 past it");
    let at = |i: i32, j: i32| {
        if i < n {
            tex[face.index()][j as usize * N + i as usize]
        } else {
            /* +X's right edge adjoins -Z's left edge (both are x = +|z|): column 0 of -Z, row j */
            tex[Face::NegZ.index()][j as usize * N]
        }
    };
    let (p00, p10, p01, p11) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    core::array::from_fn(|c| {
        p00[c] * (1.0 - a) * (1.0 - b) + p10[c] * a * (1.0 - b) + p01[c] * (1.0 - a) * b + p11[c] * a * b
    })
}

/// `prog` compiled from [`VS`] and [`FS`], handed to `f`.
fn with_program(f: impl FnOnce(&V3dProgram<'_>)) {
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(FS).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, &a).expect("the program compiles");
    f(&p);
}

/// The vertex program's listing.
fn listing(p: &V3dProgram<'_>) -> Vec<String> {
    p.vs.code
        .instrs
        .iter()
        .map(|i| {
            let mut s = String::new();
            write_instr(&mut s, i).unwrap();
            s
        })
        .collect()
}

/// The vertex program run on the interpreter with every lane's `a_dir` =
/// `dir` and sampler 0 = `sampler`: the four components of `v_color` (the
/// last four VPM output rows, as f32) of lane 0.
fn run_vs(p: &V3dProgram<'_>, dir: [f32; 3], sampler: Sampler) -> [f32; 4] {
    /* the uniform stream: constants, the viewport words (unused here), the
     * TMU P0/P1 words as the compiler wrote them (unit 0) */
    let uniforms: Vec<u32> =
        p.vs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                2..=5 => 1.0f32.to_bits(),
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    /* a_dir is the one attribute: one VPM row per component */
    assert_eq!(p.attributes.len(), 1, "one attribute: {:?}", p.attributes);
    let rec = p.attributes[0].1.expect("a_dir is read") as usize;
    assert_eq!(p.vs.vattr_sizes[rec], 4, "vec4: {:?}", p.vs.vattr_sizes);
    let rows: Vec<[u32; LANES]> = [dir[0], dir[1], dir[2], 1.0].iter().map(|c| [c.to_bits(); LANES]).collect();
    let env = Env { textures: vec![texture()], samplers: vec![sampler], threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &uniforms, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .unwrap_or_else(|e| panic!("the vertex program runs: {e}\n{:#?}", listing(p)));
    assert_eq!(o.tmu_lookups, 1, "one look-up");
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
    let v = &out[out.len() - 4..];
    core::array::from_fn(|c| f32::from_bits(v[c][0]))
}

/// What the compiler emits today, and what it yields: a TMUSCM cube
/// lookup, which the model filters seamlessly whatever the wrap mode. In a
/// build that does not report ES 2.0 this is right (ES 3.0 3.8.9.1) and
/// stays. In a report-es2 build this value is WRONG against dEQP's ES 2.0
/// reference: it is pinned here on purpose (the defect, reproduced) until
/// item v3d-cube-fix makes the ES 2.0 build take the non-seamless path,
/// when this test becomes `#[cfg(not(feature = "report-es2"))]` and the
/// ignored test below takes over for the ES 2.0 build.
#[test]
fn the_compiled_cube_lookup_is_seamless_today() {
    let seamless = seamless_reference(DIR);
    assert_eq!(seamless, [0.75, 0.5625, 0.53125, 1.0]);
    assert_eq!(non_seamless_reference(DIR, Wrap::ClampToEdge), [1.0, 0.75, 0.375, 1.0]);
    assert_eq!(non_seamless_reference(DIR, Wrap::Repeat), [1.0, 0.5625, 0.375, 1.0]);
    assert_eq!(non_seamless_reference(DIR, Wrap::MirroredRepeat), [1.0, 0.75, 0.375, 1.0]);
    with_program(|p| {
        let text = listing(p);
        assert!(text.iter().any(|t| t.contains("tmuscm")), "a cube look-up, retired by TMUSCM: {text:#?}");
        assert!(!text.iter().any(|t| t.contains("tmui")), "no array layer: {text:#?}");
        for mode in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            let got = run_vs(p, DIR, linear(mode));
            assert_eq!(got, seamless, "{mode:?}: the hardware's seamless blend, the wrap mode ignored");
            assert_ne!(
                got,
                non_seamless_reference(DIR, mode),
                "{mode:?}: differs from ES 2.0's per-face rule (the 25 Pi 4 failures)"
            );
        }
    });
}

/// The target of item v3d-cube-fix: a report-es2 build's vertex program
/// reads the cube non-seamlessly, each face a 2D texture with the sampler's
/// wrap modes. Today the program is the cube look-up above and this fails,
/// so it is ignored; v3d-cube-fix removes the ignore (and makes the test
/// above ES 3-only).
#[cfg(feature = "report-es2")]
#[test]
#[ignore = "v3d-cube-fix: the ES 2.0 build does not emulate non-seamless cube sampling yet (docs/v3d-noseamless-cube.md)"]
fn an_es2_build_samples_a_cube_non_seamlessly() {
    with_program(|p| {
        for mode in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            let got = run_vs(p, DIR, linear(mode));
            assert_eq!(got, non_seamless_reference(DIR, mode), "{mode:?}: ES 2.0's per-face rule");
        }
    });
}

/// The two references, checked against each other by hand: the direction
/// is +X at (0.9375, 0.5); column 3 of rows 1 and 2 weighs 3/4, and the
/// fourth tap (1/4) is -Z's blue edge column in the seamless rule, column 3
/// again under CLAMP and MIRROR, column 0 under REPEAT. So the test above
/// does not depend on the interpreter's model for its expectations.
#[test]
fn the_references_differ_only_in_the_tap_past_the_edge() {
    let (face, s, t) = es2_face_coords(DIR);
    assert_eq!((face, s, t), (Face::PosX, 0.9375, 0.5));
    let px = &faces()[0];
    let col = |i: usize, j: usize| px[j * N + i];
    let rows = |i: usize| -> [f32; 4] { core::array::from_fn(|c| 0.5 * col(i, 1)[c] + 0.5 * col(i, 2)[c]) };
    let blend = |fourth: [f32; 4]| -> [f32; 4] { core::array::from_fn(|c| 0.75 * rows(3)[c] + 0.25 * fourth[c]) };
    assert_eq!(seamless_reference(DIR), blend([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(non_seamless_reference(DIR, Wrap::ClampToEdge), blend(rows(3)));
    assert_eq!(non_seamless_reference(DIR, Wrap::MirroredRepeat), blend(rows(3)));
    assert_eq!(non_seamless_reference(DIR, Wrap::Repeat), blend(rows(0)));
}
