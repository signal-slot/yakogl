// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! textureGrad and the 3D texture offset on the V3D 4.2 (item v3d-qpu-grad,
//! docs/notes/v3d-qpu-grad.md), on the host.
//!
//! The V3D 4.2 TMU has no gradient operand: the compiler computes the level
//! from the two gradients and the texture's size words (`prepare.rs`
//! `grad_lod`, a cube's through `cube_lod`) and the look-up takes it as an
//! explicit level, as `textureLod`'s does (Mesa's `lower_txd`). A 3D
//! texture offset's r goes to TMU Config Parameter 2's Offset R (bits
//! 16..19, Mesa's `offset_r`).
//!
//! Each form is compiled and linked through `SHC_ONLINE`, which is what
//! glCompileShader and glLinkProgram reach; the values run on the QPU
//! interpreter (`v3d::sim`) against a level and a texel computed HERE on
//! the CPU from the GL ES 3.0 formulas (3.8.10: lambda = log2(rho), rho the
//! larger length of the two gradients in texels of level 0; the cube's
//! face-projected derivatives as dEQP's `computeCubeLodFromDerivates`; the
//! NEAREST_MIPMAP_NEAREST level ceil(lambda + 0.5) - 1). Every level of a
//! mipmapped texture has its own colour, so the colour names the level the
//! TMU read; the levels are chosen away from the level boundaries
//! (lambda = k + 0.26), where an SFU approximation could not move them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_grad_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program, Code, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

// ---- glCompileShader / glLinkProgram through SHC_ONLINE ---------------------

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// glCompileShader on both stages, then glLinkProgram.
fn build(vs: &str, fs: &str) -> Result<(), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

/// The forms of GLSL ES 3.00 8.8 with gradients, every sampler type but the
/// shadow ones (item v3d-qpu-shadow's), and the 3D offsets: (sampler,
/// gradient width, expression of `t`, `p` (a vec4) and the gradients `gx`,
/// `gy`).
const FORMS: [(&str, u8, &str); 16] = [
    ("sampler2D", 2, "textureGrad(t, p.xy, gx, gy)"),
    ("sampler2D", 2, "textureGradOffset(t, p.xy, gx, gy, ivec2(-8, 7))"),
    ("sampler2D", 2, "textureProjGrad(t, p.xyz, gx, gy)"),
    ("sampler2D", 2, "textureProjGrad(t, p, gx, gy)"),
    ("sampler2D", 2, "textureProjGradOffset(t, p, gx, gy, ivec2(1, -1))"),
    ("isampler2D", 2, "vec4(textureGrad(t, p.xy, gx, gy))"),
    ("usampler2D", 2, "vec4(textureGradOffset(t, p.xy, gx, gy, ivec2(2, 3)))"),
    ("sampler2DArray", 2, "textureGrad(t, p.xyz, gx, gy)"),
    ("sampler2DArray", 2, "textureGradOffset(t, p.xyz, gx, gy, ivec2(1, -2))"),
    ("sampler3D", 3, "textureGrad(t, p.xyz, gx, gy)"),
    ("sampler3D", 3, "textureGradOffset(t, p.xyz, gx, gy, ivec3(1, -2, 3))"),
    ("sampler3D", 3, "textureProjGradOffset(t, p, gx, gy, ivec3(-8, 7, -8))"),
    ("sampler3D", 3, "textureOffset(t, p.xyz, ivec3(0, 0, 7))"),
    ("sampler3D", 3, "textureLodOffset(t, p.xyz, 1.0, ivec3(-1, 0, -1))"),
    ("isampler3D", 3, "vec4(texelFetchOffset(t, ivec3(p.xyz), 0, ivec3(0, 1, -2)))"),
    ("samplerCube", 3, "textureGrad(t, p.xyz, gx, gy)"),
];

/// Every form builds in the fragment and the vertex stage (a vertex shader
/// may sample with gradients: they are its own values, no derivative is
/// involved). The shadow forms stay refused by name, another item's.
#[test]
fn grad_and_3d_offset_forms_build() {
    let vs_pass = "#version 300 es\nin highp vec4 a_pos;\nout highp vec4 p;\n\
                   void main() {\n  p = a_pos;\n  gl_Position = a_pos;\n}\n";
    for (sampler, gw, expr) in FORMS {
        let decl = format!("uniform highp {sampler} t;\nuniform highp vec{gw} gx, gy;\n");
        let fs = format!(
            "#version 300 es\nprecision highp float;\n{decl}in highp vec4 p;\nout vec4 o;\n\
             void main() {{\n  o = {expr};\n}}\n"
        );
        build(vs_pass, &fs).unwrap_or_else(|e| panic!("fragment {sampler} {expr}: {e}\n{fs}"));
        let vs = format!(
            "#version 300 es\n{decl}in highp vec4 a_pos;\nout highp vec4 c;\n\
             void main() {{\n  highp vec4 p = a_pos;\n  c = {expr};\n  gl_Position = a_pos;\n}}\n"
        );
        let fs_c = "#version 300 es\nprecision mediump float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n";
        if expr.starts_with("textureOffset(") {
            continue; /* an implicit level: the vertex stage's is level 0, built above as textureLod */
        }
        build(&vs, fs_c).unwrap_or_else(|e| panic!("vertex {sampler} {expr}: {e}\n{vs}"));
    }
    let fs = "#version 300 es\nprecision highp float;\nuniform highp sampler2DShadow t;\n\
              uniform highp vec2 gx, gy;\nin highp vec4 p;\nout vec4 o;\n\
              void main() {\n  o = vec4(textureGrad(t, p.xyz, gx, gy));\n}\n";
    let vs =
        "#version 300 es\nin highp vec4 a_pos;\nout highp vec4 p;\nvoid main() { p = a_pos; gl_Position = a_pos; }\n";
    let e = build(vs, fs).expect_err("a shadow look-up is still refused");
    assert!(e.contains("shadow"), "{e}");
}

// ---- values, on the QPU interpreter ----------------------------------------

/// The colour of level `l`: exact in f16, a different red per level.
fn level_colour(l: usize) -> [f32; 4] {
    [l as f32 / 8.0, 1.0 - l as f32 / 8.0, 0.5, 1.0]
}

/// A mipmapped texture of `w` x `h` (`faces` images a level: 1 for 2D, 6
/// for a cube; a 3D texture's are its slices, `depth`, minified with the
/// level), every texel of a level its [`level_colour`].
fn mip_texture(w: u32, h: u32, depth: Option<u32>, cube: bool) -> Texture {
    let n = 32 - w.max(h).max(depth.unwrap_or(1)).leading_zeros();
    let levels = (0..n)
        .map(|l| {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let faces = match (cube, depth) {
                (true, _) => 6,
                (false, Some(d)) => (d >> l).max(1),
                (false, None) => 1,
            };
            Level {
                width: lw,
                height: lh,
                faces: vec![vec![level_colour(l as usize); (lw * lh) as usize]; faces as usize],
            }
        })
        .collect();
    Texture { cube, min: MinFilter::NearestMipmapNearest, levels }
}

const MIP_NEAREST: Sampler = Sampler {
    mag: Filter::Nearest,
    min: MinFilter::NearestMipmapNearest,
    wrap_s: Wrap::ClampToEdge,
    wrap_t: Wrap::ClampToEdge,
};

/// GL ES 3.0 3.8.11, NEAREST_MIPMAP_NEAREST: the magnification filter's
/// level 0 at lambda <= 0 (c = 0: the magnification filter is NEAREST),
/// else ceil(lambda + 0.5) - 1, clamped to the chain.
fn nearest_level(lambda: f64, levels: usize) -> usize {
    if lambda <= 0.0 {
        0
    } else {
        ((lambda + 0.5).ceil() as usize).saturating_sub(1).min(levels - 1)
    }
}

/// GL ES 3.0 3.8.10 for a 2D, 2D-array or 3D texture: log2 of the larger
/// length of the gradients in texels of level 0 (lanes past `size.len()`
/// are not part of the gradient).
fn lambda_2d(size: &[u32], gx: [f32; 3], gy: [f32; 3]) -> f64 {
    let len = |g: [f32; 3]| size.iter().zip(g).map(|(&s, d)| (s as f64 * d as f64).powi(2)).sum::<f64>().sqrt();
    len(gx).max(len(gy)).log2()
}

/// The cube's lambda as dEQP's `computeCubeLodFromDerivates` (the face of
/// the direction's major axis; signs do not matter for a length), with
/// the face width `n`.
fn lambda_cube(n: u32, d: [f32; 3], gx: [f32; 3], gy: [f32; 3]) -> f64 {
    let a = d.map(|c| (c as f64).abs());
    let (ma, s, t) = if a[0] >= a[1] && a[0] >= a[2] {
        (0, 2, 1)
    } else if a[1] >= a[2] {
        (1, 0, 2)
    } else {
        (2, 0, 1)
    };
    let duv = |g: [f32; 3]| {
        let (m, dm) = (a[ma], (g[ma] as f64).abs());
        let du = n as f64 * 0.5 * (g[s] as f64 * m - d[s] as f64 * dm) / (m * m);
        let dv = n as f64 * 0.5 * (g[t] as f64 * m - d[t] as f64 * dm) / (m * m);
        (du * du + dv * dv).sqrt()
    };
    duv(gx).max(duv(gy)).log2()
}

fn listing(c: &Code<'_>) -> Vec<String> {
    c.instrs
        .iter()
        .map(|i| {
            let mut s = String::new();
            write_instr(&mut s, i).unwrap();
            s
        })
        .collect()
}

/// `vs` + `fs` compiled for the V3D (no key), handed to `f`.
fn with_program<R>(vs: &str, fs: &str, f: impl FnOnce(&V3dProgram<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, &a).expect("the program compiles");
    f(&p)
}

/// The uniform stream of `c`: constants and the TMU words as compiled,
/// `user` by dword, the viewport 1.0, and the size words (QUNIFORM_TEXTURE_
/// WIDTH .. _ARRAY_SIZE, 44..47) of sampler 0 from `size`.
fn uniforms(c: &Code<'_>, user: &[f32], size: [u32; 4]) -> Vec<u32> {
    c.uniforms
        .iter()
        .map(|u| match u.contents {
            0 | 40 | 41 => u.data,
            1 => user[u.data as usize].to_bits(),
            2..=5 => 1.0f32.to_bits(),
            44..=47 if u.data == 0 => size[(u.contents - 44) as usize],
            /* QUNIFORM_FB_WPOS_Y_SCALE: a window target */
            0x1_0000 => (-1.0f32).to_bits(),
            c => panic!("uniform contents {c} (data {}) not modelled by this test", u.data),
        })
        .collect()
}

/// A vertex shader reading `a_pos` (location 0, vec4) and writing the
/// varying `c` (vec4), run on the interpreter with every lane's `a_pos` =
/// `pos`, sampler 0 = `tex` with `sampler`, `user` the user uniforms: `c`.
fn run_vs(vs: &str, pos: [f32; 4], tex: &Texture, sampler: Sampler, user: &[f32], size: [u32; 4]) -> [f32; 4] {
    const FS: &str = "#version 300 es\nprecision mediump float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n";
    with_program(vs, FS, |p| {
        let u = uniforms(&p.vs.code, user, size);
        let rec = p.attributes.iter().find(|e| e.0 == 0).and_then(|e| e.1).expect("a_pos is read") as usize;
        let rows: Vec<[u32; LANES]> =
            pos.iter().take(p.vs.vattr_sizes[rec] as usize).map(|c| [c.to_bits(); LANES]).collect();
        let env = Env { textures: vec![tex.clone()], samplers: vec![sampler], threads: p.vs.threads, ..Env::default() };
        let o = sim::run(p.vs.code.words, &u, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
            .unwrap_or_else(|e| panic!("the vertex program runs: {e}\n{:#?}", listing(&p.vs.code)));
        assert_eq!(o.tmu_lookups, 1, "one look-up");
        let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
        let v = &out[out.len() - 4..];
        core::array::from_fn(|c| f32::from_bits(v[c][0]))
    })
}

fn vs_grad(sampler: &str, gw: u8, expr: &str) -> String {
    format!(
        "#version 300 es\nuniform highp {sampler} t;\nuniform highp vec{gw} gx, gy;\n\
         layout(location = 0) in highp vec4 a_pos;\nout highp vec4 c;\n\
         void main() {{\n  highp vec4 p = a_pos;\n  c = {expr};\n  gl_Position = a_pos;\n}}\n"
    )
}

/// The gradient cases of a 2D texture of 16 x 8 (non-square: a swapped
/// width and height shows), as (dPdx, dPdy): each axis alone, the larger
/// of the two chosen, both, a negative lane, a zero gradient (log2(0),
/// clamped: the magnified base level) and one past the chain.
fn grads_2d() -> Vec<([f32; 3], [f32; 3])> {
    let k = |l: i32| 1.2 * 2f32.powi(l); /* lambda = l + 0.263 */
    vec![
        ([k(0) / 16.0, 0.0, 0.0], [0.0; 3]),
        ([k(1) / 16.0, 0.0, 0.0], [0.0; 3]),
        ([0.0, k(2) / 8.0, 0.0], [0.0; 3]),
        ([0.0; 3], [k(3) / 16.0, 0.0, 0.0]),
        ([k(1) / 16.0, 0.0, 0.0], [0.0, -k(2) / 8.0, 0.0]),
        ([-k(2) / 16.0 * 0.6, k(2) / 8.0 * 0.8, 0.0], [0.0; 3]),
        ([0.0; 3], [0.0; 3]),
        ([k(0) / 64.0, 0.0, 0.0], [0.0, k(0) / 32.0, 0.0]),
        ([4.0, 0.0, 0.0], [0.0; 3]),
    ]
}

fn close(got: [f32; 4], want: [f32; 4], what: &str) {
    for c in 0..4 {
        assert!((got[c] - want[c]).abs() <= 1e-3, "{what}: channel {c}: got {got:?}, want {want:?}");
    }
}

/// textureGrad of a 2D texture in the vertex stage: the level the TMU read
/// is the one GL ES 3.0 3.8.10 gives for the gradients. Revert checks: the
/// size lanes swapped (height for s) moves the third case to level 3; the
/// sum of the two lengths instead of the larger moves the fifth.
#[test]
fn texture_grad_2d_picks_the_level_of_its_gradients() {
    let tex = mip_texture(16, 8, None, false);
    let vs = vs_grad("sampler2D", 2, "textureGrad(t, p.xy, gx, gy)");
    for (gx, gy) in grads_2d() {
        let lambda = lambda_2d(&[16, 8], gx, gy);
        let want = level_colour(nearest_level(lambda, tex.levels.len()));
        let got = run_vs(&vs, [0.3, 0.3, 0.0, 1.0], &tex, MIP_NEAREST, &[gx[0], gx[1], gy[0], gy[1]], [16, 8, 1, 1]);
        close(got, want, &format!("dPdx {gx:?} dPdy {gy:?} lambda {lambda}"));
    }
}

/// textureProjGrad: the coordinate is divided by q, the gradients are not
/// (Mesa projects first and uses them as given).
#[test]
fn texture_proj_grad_does_not_divide_the_gradients() {
    let tex = mip_texture(16, 8, None, false);
    let vs = vs_grad("sampler2D", 2, "textureProjGrad(t, p, gx, gy)");
    for (gx, gy) in grads_2d() {
        let want = level_colour(nearest_level(lambda_2d(&[16, 8], gx, gy), tex.levels.len()));
        let got = run_vs(&vs, [1.2, 1.2, 0.0, 4.0], &tex, MIP_NEAREST, &[gx[0], gx[1], gy[0], gy[1]], [16, 8, 1, 1]);
        close(got, want, &format!("dPdx {gx:?} dPdy {gy:?}"));
    }
}

/// textureGrad of a 3D texture of 8 x 8 x 16: the r lane of the gradients
/// scales by the depth (QUNIFORM_TEXTURE_DEPTH), not the height. Revert
/// check: a two-lane length reads level 0 for the r-only cases.
#[test]
fn texture_grad_3d_scales_r_by_the_depth() {
    let tex = mip_texture(8, 8, Some(16), false);
    let vs = vs_grad("sampler3D", 3, "textureGrad(t, p.xyz, gx, gy)");
    let k = |l: i32| 1.2 * 2f32.powi(l);
    for (gx, gy) in [
        ([0.0, 0.0, k(1) / 16.0], [0.0; 3]),
        ([0.0, 0.0, k(3) / 16.0], [0.0; 3]),
        ([k(1) / 8.0, 0.0, 0.0], [0.0, 0.0, k(2) / 16.0]),
        ([k(0) / 8.0 * 0.6, 0.0, k(0) / 16.0 * 0.8], [0.0; 3]),
    ] {
        let lambda = lambda_2d(&[8, 8, 16], gx, gy);
        let want = level_colour(nearest_level(lambda, tex.levels.len()));
        let user = [gx[0], gx[1], gx[2], 0.0, gy[0], gy[1], gy[2]];
        let got = run_vs(&vs, [0.3, 0.3, 0.3, 1.0], &tex, MIP_NEAREST, &user, [8, 8, 16, 1]);
        close(got, want, &format!("dPdx {gx:?} dPdy {gy:?} lambda {lambda}"));
    }
}

/// textureGrad of a cube with faces of 16: the level of the direction's
/// face from the gradients (dEQP's formula), on each major axis, with a
/// direction whose major axis is far from 1 (the gradients are not
/// normalised with it). A direction on an edge (where either face gives
/// the level) is not run: the interpreter refuses a cube direction within
/// 1e-3 of a face edge.
#[test]
fn texture_grad_cube_picks_the_level_of_its_face() {
    let tex = mip_texture(16, 16, None, true);
    let vs = vs_grad("samplerCube", 3, "textureGrad(t, p.xyz, gx, gy)");
    let cases: [([f32; 3], [f32; 3], [f32; 3]); 5] = [
        ([1.0, 0.2, 0.1], [0.0, 0.15, 0.0], [0.0; 3]),
        ([-0.1, 1.0, 0.3], [0.3, 0.0, 0.0], [0.0, 0.0, 0.05]),
        ([0.2, -0.1, -1.0], [0.0; 3], [0.0, 0.6, 0.0]),
        ([6.0, 1.0, -2.0], [0.0, 0.0, 1.8], [0.0; 3]),
        ([1.0, 0.2, 0.1], [0.0; 3], [0.0; 3]),
    ];
    for (d, gx, gy) in cases {
        let lambda = lambda_cube(16, d, gx, gy);
        let want = level_colour(nearest_level(lambda, tex.levels.len()));
        let user = [gx[0], gx[1], gx[2], 0.0, gy[0], gy[1], gy[2]];
        let got = run_vs(&vs, [d[0], d[1], d[2], 1.0], &tex, MIP_NEAREST, &user, [16, 16, 1, 1]);
        close(got, want, &format!("direction {d:?} dPdx {gx:?} dPdy {gy:?} lambda {lambda}"));
    }
}

/// A texture of `w` x `h` x `d`, one level, whose texel (x, y, z) names
/// itself: (x, y, z) / 16, exact in f16.
fn coded_3d(w: u32, h: u32, d: u32) -> Texture {
    let slice = |z: u32| {
        (0..h)
            .flat_map(move |y| (0..w).map(move |x| [x as f32 / 16.0, y as f32 / 16.0, z as f32 / 16.0, 1.0]))
            .collect()
    };
    Texture {
        cube: false,
        min: MinFilter::Nearest,
        levels: vec![Level { width: w, height: h, faces: (0..d).map(slice).collect() }],
    }
}

/// The 3D texel offset (GL ES 3.0 3.8.10: added to the texel coordinates
/// before the wrap): Offset R moves the slice, S and T the texel, in a
/// textureLodOffset, a textureGradOffset and a texelFetchOffset of the
/// vertex stage. The expected texel is computed here: the offset added to
/// floor(coord * size), clamped to the edge (a fetch outside the texture is undefined
/// in GL, so the fetch runs only for the offsets that stay inside). Revert
/// check: Offset R left 0 reads the slice of the coordinate.
#[test]
fn a_3d_offset_moves_the_slice() {
    let tex = coded_3d(8, 8, 8);
    let pos: [f32; 4] = [0.32, 0.45, 0.58, 1.0];
    let texel = |off: [i32; 3]| -> [f32; 4] {
        let c: [i32; 3] = core::array::from_fn(|i| ((pos[i] * 8.0).floor() as i32 + off[i]).clamp(0, 7));
        [c[0] as f32 / 16.0, c[1] as f32 / 16.0, c[2] as f32 / 16.0, 1.0]
    };
    for off in [[0, 0, 3], [0, 0, -4], [1, -2, 2], [-8, 7, -8], [7, 0, 7]] {
        let o = format!("ivec3({}, {}, {})", off[0], off[1], off[2]);
        let inside = (0..3).all(|i| (0..8).contains(&((pos[i] * 8.0).floor() as i32 + off[i])));
        let mut forms =
            vec![format!("textureLodOffset(t, p.xyz, 0.0, {o})"), format!("textureGradOffset(t, p.xyz, gx, gy, {o})")];
        if inside {
            forms.push(format!("texelFetchOffset(t, ivec3(p.xyz * 8.0), 0, {o})"));
        }
        for expr in forms {
            let vs = vs_grad("sampler3D", 3, &expr);
            let got = run_vs(&vs, pos, &tex, Sampler::NEAREST_CLAMP, &[0.0; 8], [8, 8, 8, 1]);
            close(got, texel(off), &expr);
        }
    }
}

/// The fragment stage's textureGrad: the same level from uniform
/// gradients, no derivative read (every lane its own explicit level),
/// written to the TLB as f16 pairs.
#[test]
fn texture_grad_in_the_fragment_stage() {
    let tex = mip_texture(16, 8, None, false);
    const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec2 v;\n\
                      void main() { v = a_pos.xy; gl_Position = a_pos; }\n";
    const FS: &str = "#version 300 es\nprecision mediump float;\nuniform highp sampler2D t;\n\
                      uniform highp vec2 gx, gy;\nin highp vec2 v;\nout vec4 o;\n\
                      void main() { o = textureGrad(t, v, gx, gy); }\n";
    for (gx, gy) in grads_2d() {
        let want = level_colour(nearest_level(lambda_2d(&[16, 8], gx, gy), tex.levels.len()));
        let got = with_program(VS, FS, |p| {
            assert!(!listing(&p.fs.code).iter().any(|i| i.contains("fdx") || i.contains("fdy")), "no derivative");
            let u = uniforms(&p.fs.code, &[gx[0], gx[1], gy[0], gy[1]], [16, 8, 1, 1]);
            let one = [1.0f32.to_bits(); LANES];
            let z = [0u32; LANES];
            let varyings = (0..2).map(|_| sim::Varying { vary: [0.3f32.to_bits(); LANES], c: z }).collect();
            let inputs = FsInputs { w: one, w_centroid: one, z, x: z, y: z, back_facing: [false; LANES], varyings };
            let env = Env {
                textures: vec![tex.clone()],
                samplers: vec![MIP_NEAREST],
                threads: p.fs.threads,
                ..Env::default()
            };
            let o = sim::run(p.fs.code.words, &u, &Inputs::Fragment(Box::new(inputs)), &env)
                .unwrap_or_else(|e| panic!("the fragment program runs: {e}\n{:#?}", listing(&p.fs.code)));
            let h = |x: u32| sim::half_to_f32(x as u16);
            let word = |k: usize| o.tlb[k].value[0].expect("a defined value");
            [h(word(0) & 0xffff), h(word(0) >> 16), h(word(1) & 0xffff), h(word(1) >> 16)]
        });
        close(got, want, &format!("dPdx {gx:?} dPdy {gy:?}"));
    }
}
