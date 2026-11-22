// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shadow look-ups on the V3D 4.2 (`sampler2DShadow`, `samplerCubeShadow`,
//! `sampler2DArrayShadow`), item pi4-results, designed in
//! docs/notes/v3d-shadow.md.
//!
//! The reference depth is one more TMU write, TMUDREF, after the coordinates
//! and the bias or level and before the S write that retires the look-up
//! (Mesa's `nir_tex_src_comparator`, v3d_tex.c); no P1 or P2 bit names it.
//! The compare function is the sampler record's (`GlesTextureBind::
//! compare_op`, `v3d::texture::pack_sampler`): the TMU compares every texel
//! the filter reads with the reference, then filters (percentage-closer,
//! ES 3.0.6 3.8.15.1). A depth texture returns 32 bits (every depth row of
//! Mesa's format table), so a draw keys the look-up `ret32` and one `ldtmu`
//! word comes back. Before this item the compiler refused every shadow form
//! by name (`NotYet("a shadow look-up")`).
//!
//! Here: every shadow form of GLSL ES 3.00 8.8 builds through `SHC_ONLINE`
//! (glCompileShader and glLinkProgram), in the fragment and the vertex
//! stage; and the keyed programs run on the QPU interpreter (`v3d::sim`,
//! whose TMUDREF model is `texref::Compare`) against values computed HERE
//! from ES 3.0.6 3.8.15.1: reference OP depth per texel, 0 or 1 under
//! NEAREST, the bilinear blend of the four compares under LINEAR, for each
//! of the eight functions, a projected reference, an offset, a cube face, an
//! array layer, and a vertex-stage look-up.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{CompareFunc, Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, Code, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema, SHC_ONLINE};
use yakogl::shaderc::{LodKey, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

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
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
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
    let link = SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

/// The shadow forms of GLSL ES 3.00 8.8: (sampler, expression of `t`, `p`
/// (a vec4), a float `b` and the gradients `gx`, `gy` (vec3); whether the
/// form is for the fragment stage only (a bias)).
const FORMS: [(&str, &str, bool); 20] = [
    ("sampler2DShadow", "texture(t, p.xyz)", false),
    ("sampler2DShadow", "texture(t, p.xyz, b)", true),
    ("sampler2DShadow", "textureProj(t, p)", false),
    ("sampler2DShadow", "textureProj(t, p, b)", true),
    ("sampler2DShadow", "textureLod(t, p.xyz, b)", false),
    ("sampler2DShadow", "textureOffset(t, p.xyz, ivec2(1, -1))", false),
    ("sampler2DShadow", "textureOffset(t, p.xyz, ivec2(-8, 7), b)", true),
    ("sampler2DShadow", "textureProjOffset(t, p, ivec2(2, 2))", false),
    ("sampler2DShadow", "textureProjOffset(t, p, ivec2(2, 2), b)", true),
    ("sampler2DShadow", "textureLodOffset(t, p.xyz, b, ivec2(-1, 0))", false),
    ("sampler2DShadow", "textureProjLod(t, p, b)", false),
    ("sampler2DShadow", "textureProjLodOffset(t, p, b, ivec2(1, 1))", false),
    ("sampler2DShadow", "textureGrad(t, p.xyz, gx.xy, gy.xy)", false),
    ("sampler2DShadow", "textureGradOffset(t, p.xyz, gx.xy, gy.xy, ivec2(1, 2))", false),
    ("sampler2DShadow", "textureProjGrad(t, p, gx.xy, gy.xy)", false),
    ("sampler2DShadow", "textureProjGradOffset(t, p, gx.xy, gy.xy, ivec2(-2, 3))", false),
    ("samplerCubeShadow", "texture(t, p)", false),
    ("samplerCubeShadow", "texture(t, p, b)", true),
    ("samplerCubeShadow", "textureGrad(t, p, gx, gy)", false),
    ("sampler2DArrayShadow", "texture(t, p)", false),
];

/// Every shadow form builds in the fragment stage, and every form without a
/// bias in the vertex stage too (a vertex shader's implicit look-up is level
/// 0). Revert check: the old arm refused each with "a shadow look-up".
#[test]
fn every_shadow_form_builds() {
    let vs_pass = "#version 300 es\nin highp vec4 a_pos;\nout highp vec4 p;\n\
                   void main() {\n  p = a_pos;\n  gl_Position = a_pos;\n}\n";
    let fs_c = "#version 300 es\nprecision mediump float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n";
    for (sampler, expr, fragment_only) in FORMS {
        let decl = format!("uniform highp {sampler} t;\nuniform highp float b;\nuniform highp vec3 gx, gy;\n");
        let fs = format!(
            "#version 300 es\nprecision highp float;\n{decl}in highp vec4 p;\nout vec4 o;\n\
             void main() {{\n  o = vec4({expr});\n}}\n"
        );
        build(vs_pass, &fs).unwrap_or_else(|e| panic!("fragment {sampler} {expr}: {e}\n{fs}"));
        if fragment_only {
            continue;
        }
        let vs = format!(
            "#version 300 es\n{decl}in highp vec4 a_pos;\nout highp vec4 c;\n\
             void main() {{\n  highp vec4 p = a_pos;\n  c = vec4({expr});\n  gl_Position = a_pos;\n}}\n"
        );
        build(&vs, fs_c).unwrap_or_else(|e| panic!("vertex {sampler} {expr}: {e}\n{vs}"));
    }
    /* the two forms sampler2DArrayShadow has beside texture(): with
     * gradients (GLSL ES 3.00 8.8 lists no textureLod, textureOffset or bias
     * for it) */
    let fs = "#version 300 es\nprecision highp float;\nuniform highp sampler2DArrayShadow t;\n\
              uniform highp vec2 gx, gy;\nin highp vec4 p;\nout vec4 o;\n\
              void main() {\n  o = vec4(textureGrad(t, p, gx, gy));\n}\n";
    build(vs_pass, fs).expect("textureGrad of a 2D-array shadow sampler");
}

// ---- values, on the QPU interpreter ----------------------------------------

/// A depth texel: the depth in R (the GL core keeps depth as R32F).
fn depth(d: f32) -> [f32; 4] {
    [d, 0.0, 0.0, 1.0]
}

/// The 2x2 depth texture of the tests: 0.25, 0.5 on row 0, 0.5, 0.75 on
/// row 1; with `layers` images, layer k's depths k/4 higher.
fn depth_2x2(layers: usize) -> Texture {
    let faces = (0..layers).map(|k| [0.25, 0.5, 0.5, 0.75].map(|d| depth(d + k as f32 / 4.0)).to_vec()).collect();
    Texture { cube: false, min: MinFilter::Nearest, levels: vec![Level { width: 2, height: 2, faces }] }
}

/// A cube whose six faces (+X, -X, +Y, -Y, +Z, -Z) are flat depths.
fn depth_cube(depths: [f32; 6]) -> Texture {
    let faces = depths.iter().map(|&d| vec![depth(d); 4]).collect();
    Texture { cube: true, min: MinFilter::Nearest, levels: vec![Level { width: 2, height: 2, faces }] }
}

const LINEAR: Sampler =
    Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };

/// ES 3.0.6 3.8.15.1: `reference OP depth` (GL_LESS passes when the
/// reference is below the depth), 1.0 when it passes.
fn compare(func: CompareFunc, reference: f32, d: f32) -> f32 {
    let pass = match func {
        CompareFunc::Never => false,
        CompareFunc::Less => reference < d,
        CompareFunc::Equal => reference == d,
        CompareFunc::Lequal => reference <= d,
        CompareFunc::Greater => reference > d,
        CompareFunc::NotEqual => reference != d,
        CompareFunc::Gequal => reference >= d,
        CompareFunc::Always => true,
    };
    if pass {
        1.0
    } else {
        0.0
    }
}

const FUNCS: [CompareFunc; 8] = [
    CompareFunc::Never,
    CompareFunc::Less,
    CompareFunc::Equal,
    CompareFunc::Lequal,
    CompareFunc::Greater,
    CompareFunc::NotEqual,
    CompareFunc::Gequal,
    CompareFunc::Always,
];

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

/// `vs` + `fs` compiled for the V3D with the vertex key `vs_ret32` and the
/// fragment key `fs_ret32` (the bindings whose texture returns 32 bits: a
/// depth texture's does), handed to `f`.
fn with_program<R>(vs: &str, fs: &str, vs_ret32: u32, fs_ret32: u32, f: impl FnOnce(&V3dProgram<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let vk = LodKey { ret32: vs_ret32, ..LodKey::NONE };
    let fk = LodKey { ret32: fs_ret32, ..LodKey::NONE };
    let p = compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, vk, fk, &a);
    let p = p.unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    f(&p)
}

/// NEAREST within a level, the nearest level of a mipmapped texture.
const MIP_NEAREST: Sampler = Sampler { min: MinFilter::NearestMipmapNearest, ..Sampler::NEAREST_CLAMP };

/// The uniform stream of `c`: constants and the TMU words as compiled,
/// `user` by dword, the viewport 1.0, the size words of sampler 0 from
/// `size`.
fn uniforms(c: &Code<'_>, user: &[f32], size: [u32; 4]) -> Vec<u32> {
    c.uniforms
        .iter()
        .map(|u| match u.contents {
            0 | 40 | 41 => u.data,
            1 => user.get(u.data as usize).copied().unwrap_or(0.0).to_bits(),
            2..=5 => 1.0f32.to_bits(),
            44..=47 if u.data == 0 => size[(u.contents - 44) as usize],
            /* QUNIFORM_FB_WPOS_Y_SCALE: a window target */
            0x1_0000 => (-1.0f32).to_bits(),
            c => panic!("uniform contents {c} (data {}) not modelled by this test", u.data),
        })
        .collect()
}

/// A fragment shader `fs` (its varying `v`, a vec4, every lane's `v` =
/// `vary`; `o` its colour) linked with a pass-through VS, compiled with
/// the fragment key `ret32` = 1 (binding 0, a depth texture), run on the
/// interpreter with sampler 0 = `tex` under `sampler` and the compare
/// function `func`, `user` the user uniforms: the colour's red, from the
/// f16 the FS writes.
fn run_fs(fs: &str, vary: [f32; 4], tex: &Texture, sampler: Sampler, func: CompareFunc, user: &[f32]) -> f32 {
    const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                      void main() { v = a_pos; gl_Position = a_pos; }\n";
    with_program(VS, fs, 0, 1, |p| {
        let u = uniforms(&p.fs.code, user, [2, 2, 1, 1]);
        let one = [1.0f32.to_bits(); LANES];
        let z = [0u32; LANES];
        let varyings = vary.iter().map(|c| sim::Varying { vary: [c.to_bits(); LANES], c: z }).collect();
        let inputs = FsInputs { w: one, w_centroid: one, z, x: z, y: z, back_facing: [false; LANES], varyings };
        let env = Env {
            textures: vec![tex.clone()],
            samplers: vec![sampler],
            compare: vec![Some(func)],
            threads: p.fs.threads,
            ..Env::default()
        };
        let o = sim::run(p.fs.code.words, &u, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("the fragment program runs: {e}\n{:#?}", listing(&p.fs.code)));
        assert_eq!(o.tmu_lookups, 1, "one look-up");
        let word = o.tlb[0].value[0].expect("a defined value");
        sim::half_to_f32((word & 0xffff) as u16)
    })
}

fn fs_of(sampler: &str, expr: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nuniform highp {sampler} t;\nuniform highp float u_ref, u_q;\n\
         uniform highp vec3 gx, gy;\nin highp vec4 v;\nout vec4 o;\nvoid main() {{ o = vec4({expr}); }}\n"
    )
}

/// The fragment stage's `texture(sampler2DShadow, vec3)`: at each texel
/// centre of the 2x2 depth texture, reference 0.5, every compare function
/// gives `reference OP depth` (NEAREST: 0 or 1), which pins the direction
/// of the comparison the DREF write asks for; under LINEAR at the centre
/// the four compares are blended (1/4, 3/4: never the compare of the
/// blended depth, 0.5). Revert checks: the reference written after S (or
/// not at all) fails every row but ALWAYS and NEVER; a compare of the
/// filtered depth fails the LINEAR rows.
#[test]
fn a_2d_shadow_look_up_compares_each_texel_then_filters() {
    let tex = depth_2x2(1);
    let fs = fs_of("sampler2DShadow", "texture(t, vec3(v.xy, u_ref))");
    let depths = [0.25, 0.5, 0.5, 0.75];
    let centres = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    for func in FUNCS {
        for (k, (s, t)) in centres.into_iter().enumerate() {
            let got = run_fs(&fs, [s, t, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, func, &[0.5]);
            assert_eq!(got, compare(func, 0.5, depths[k]), "{func:?} at texel {k}");
        }
        let got = run_fs(&fs, [0.5, 0.5, 0.0, 1.0], &tex, LINEAR, func, &[0.5]);
        let want = depths.iter().map(|&d| compare(func, 0.5, d)).sum::<f32>() / 4.0;
        assert_eq!(got, want, "{func:?} LINEAR at the centre");
    }
    /* a reference between the depths of a LINEAR footprint: a fraction */
    let got = run_fs(&fs, [0.5, 0.5, 0.0, 1.0], &tex, LINEAR, CompareFunc::Greater, &[0.6]);
    assert_eq!(got, 0.75, "0.6 > 0.25, 0.5, 0.5 and not 0.75");
}

/// textureProj: s, t and the reference are divided by q (the fourth
/// component); textureOffset moves the texel before the wrap; textureLod
/// reads its level; a bias form builds and reads level 0 of a one-level
/// texture.
#[test]
fn the_projected_offset_and_level_forms_read_the_right_texel() {
    let tex = depth_2x2(1);
    /* (s, t, ref) = (0.25, 0.75, 0.5) * 4 over q = 4: texel 2 (0.5): LESS 0, LEQUAL 1 */
    let fs = fs_of("sampler2DShadow", "textureProj(t, vec4(v.xy, u_ref, u_q))");
    assert_eq!(run_fs(&fs, [1.0, 3.0, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Less, &[2.0, 4.0]), 0.0);
    assert_eq!(run_fs(&fs, [1.0, 3.0, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Lequal, &[2.0, 4.0]), 1.0);
    /* reference 0.6 / 4 = 0.15 against texel 2's 0.5: GREATER 0, the
     * undivided 0.6 would pass */
    assert_eq!(run_fs(&fs, [1.0, 3.0, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Greater, &[0.6, 4.0]), 0.0);
    /* Offset (1, 0) from texel 0's centre reads texel 1 (0.5): LESS of
     * 0.3 is 1 either way, EQUAL tells them apart */
    let fs = fs_of("sampler2DShadow", "textureOffset(t, vec3(v.xy, u_ref), ivec2(1, 0))");
    assert_eq!(run_fs(&fs, [0.25, 0.25, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Equal, &[0.5]), 1.0);
    assert_eq!(run_fs(&fs, [0.25, 0.25, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Equal, &[0.25]), 0.0);
    /* textureLod at level 1 of a two-level texture (1x1, depth 0.5) */
    let two = Texture {
        cube: false,
        min: MinFilter::NearestMipmapNearest,
        levels: vec![tex.levels[0].clone(), Level { width: 1, height: 1, faces: vec![vec![depth(0.5)]] }],
    };
    let fs = fs_of("sampler2DShadow", "textureLod(t, vec3(v.xy, u_ref), u_q)");
    assert_eq!(run_fs(&fs, [0.25, 0.25, 0.0, 1.0], &two, MIP_NEAREST, CompareFunc::Equal, &[0.5, 1.0]), 1.0);
    assert_eq!(run_fs(&fs, [0.25, 0.25, 0.0, 1.0], &two, MIP_NEAREST, CompareFunc::Equal, &[0.5, 0.0]), 0.0);
    /* a bias on a one-level texture: level 0 */
    let fs = fs_of("sampler2DShadow", "texture(t, vec3(v.xy, u_ref), u_q)");
    assert_eq!(run_fs(&fs, [0.75, 0.75, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Less, &[0.5, 0.0]), 1.0);
}

/// samplerCubeShadow: the face the direction selects is compared, the
/// reference is not divided by the direction's major axis (a direction of
/// length 6 with reference 0.5 compares 0.5, not 0.5 / 6).
#[test]
fn a_cube_shadow_look_up_compares_the_face_its_direction_selects() {
    let tex = depth_cube([0.25, 0.75, 0.5, 0.9, 0.1, 0.6]);
    let fs = fs_of("samplerCubeShadow", "texture(t, vec4(v.xyz, u_ref))");
    let dirs: [([f32; 3], f32); 6] = [
        ([6.0, 0.6, 0.3], 0.25),
        ([-1.0, 0.2, 0.1], 0.75),
        ([0.1, 1.0, 0.2], 0.5),
        ([0.2, -1.0, 0.1], 0.9),
        ([0.1, 0.2, 1.0], 0.1),
        ([0.2, 0.1, -1.0], 0.6),
    ];
    for (d, face_depth) in dirs {
        for func in [CompareFunc::Less, CompareFunc::Gequal, CompareFunc::Equal] {
            let got = run_fs(&fs, [d[0], d[1], d[2], 1.0], &tex, Sampler::NEAREST_CLAMP, func, &[0.5]);
            assert_eq!(got, compare(func, 0.5, face_depth), "{func:?} towards {d:?}");
        }
    }
    /* the face depth equals the reference only when it is not divided */
    let got = run_fs(&fs, [6.0, 0.6, 0.3, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Equal, &[0.25]);
    assert_eq!(got, 1.0, "the reference is compared as it is");
}

/// sampler2DArrayShadow: the layer (the coordinate's z, rounded and clamped
/// as GL ES 3.0 3.8.9) is read, the reference is the coordinate's w.
#[test]
fn an_array_shadow_look_up_reads_its_layer() {
    let tex = depth_2x2(3);
    let fs = fs_of("sampler2DArrayShadow", "texture(t, vec4(v.xyz, u_ref))");
    /* texel 0 of layer k has depth 0.25 + k / 4 */
    for (layer, want_less) in [(0.0, 0.0), (1.0, 0.0), (2.0, 1.0), (1.4, 0.0), (1.6, 1.0), (9.0, 1.0), (-3.0, 0.0)] {
        let got = run_fs(&fs, [0.25, 0.25, layer, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Less, &[0.5]);
        assert_eq!(got, want_less, "LESS 0.5 on layer {layer}");
    }
    let got = run_fs(&fs, [0.25, 0.25, 1.0, 1.0], &tex, Sampler::NEAREST_CLAMP, CompareFunc::Equal, &[0.5]);
    assert_eq!(got, 1.0, "layer 1's texel 0 is 0.5");
}

/// A vertex shader reading `a_pos` (location 0, vec4) and writing the
/// varying `c` (vec4), compiled with the vertex key `ret32` = 1, run on the
/// interpreter with every lane's `a_pos` = `pos`, sampler 0 = `tex` under
/// `sampler`: `c`'s red.
fn run_vs(vs: &str, pos: [f32; 4], tex: &Texture, sampler: Sampler, func: CompareFunc, user: &[f32]) -> f32 {
    const FS: &str = "#version 300 es\nprecision mediump float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n";
    with_program(vs, FS, 1, 0, |p| {
        let u = uniforms(&p.vs.code, user, [2, 2, 1, 1]);
        let rec = p.attributes.iter().find(|e| e.0 == 0).and_then(|e| e.1).expect("a_pos is read") as usize;
        let rows: Vec<[u32; LANES]> =
            pos.iter().take(p.vs.vattr_sizes[rec] as usize).map(|c| [c.to_bits(); LANES]).collect();
        let env = Env {
            textures: vec![tex.clone()],
            samplers: vec![sampler],
            compare: vec![Some(func)],
            threads: p.vs.threads,
            ..Env::default()
        };
        let o = sim::run(p.vs.code.words, &u, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
            .unwrap_or_else(|e| panic!("the vertex program runs: {e}\n{:#?}", listing(&p.vs.code)));
        assert_eq!(o.tmu_lookups, 1, "one look-up");
        let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
        f32::from_bits(out[out.len() - 4][0])
    })
}

/// The vertex stage: `texture()` on a shadow sampler is level 0 (no
/// derivatives), `textureLod` its level, both with the reference on
/// TMUDREF and the 32-bit result of the keyed vertex look-up.
#[test]
fn a_vertex_shadow_look_up_is_level_0_with_the_reference() {
    let tex = depth_2x2(1);
    let vs = |expr: &str| {
        format!(
            "#version 300 es\nuniform highp sampler2DShadow t;\nuniform highp float u_ref, u_lod;\n\
             layout(location = 0) in highp vec4 a_pos;\nout highp vec4 c;\n\
             void main() {{\n  c = vec4({expr});\n  gl_Position = a_pos;\n}}\n"
        )
    };
    let plain = vs("texture(t, vec3(a_pos.xy, u_ref))");
    let depths = [0.25, 0.5, 0.5, 0.75];
    let centres = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    for func in FUNCS {
        for (k, (s, t)) in centres.into_iter().enumerate() {
            let got = run_vs(&plain, [s, t, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, func, &[0.5]);
            assert_eq!(got, compare(func, 0.5, depths[k]), "{func:?} at texel {k}");
        }
    }
    let two = Texture {
        cube: false,
        min: MinFilter::NearestMipmapNearest,
        levels: vec![tex.levels[0].clone(), Level { width: 1, height: 1, faces: vec![vec![depth(0.5)]] }],
    };
    let lod = vs("textureLod(t, vec3(a_pos.xy, u_ref), u_lod)");
    assert_eq!(
        run_vs(&lod, [0.25, 0.25, 0.0, 1.0], &two, MIP_NEAREST, CompareFunc::Equal, &[0.5, 1.0]),
        1.0,
        "level 1"
    );
    assert_eq!(
        run_vs(&lod, [0.25, 0.25, 0.0, 1.0], &two, MIP_NEAREST, CompareFunc::Equal, &[0.5, 0.0]),
        0.0,
        "level 0"
    );
    /* an implicit look-up on the mipmapped texture is level 0 */
    assert_eq!(run_vs(&plain, [0.25, 0.25, 0.0, 1.0], &two, MIP_NEAREST, CompareFunc::Equal, &[0.25]), 1.0, "level 0");
}

/// The programs are the sequences docs/notes/v3d-shadow.md (c) gives: the
/// fragment look-up writes TMUDREF once, between T and S, P1 carries the
/// 32-bit output bit under the key, one `ldtmu` returns the result; a cube
/// shadow textureGrad (T, R, B, DREF, S: five TMU writes) runs at 2 threads
/// as Mesa compiles it (tests/fixtures/v3d_mesa_pins.txt, shadowcube_grad).
#[test]
fn the_shadow_programs_are_the_designed_sequences() {
    const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                      void main() { v = a_pos; gl_Position = a_pos; }\n";
    let fs = fs_of("sampler2DShadow", "texture(t, vec3(v.xy, u_ref))");
    with_program(VS, &fs, 0, 1, |p| {
        let ops = listing(&p.fs.code);
        let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
        assert_eq!((n("tmut"), n("tmudref"), n("tmus,"), n("ldtmu"), n("wrtmuc")), (1, 1, 1, 1, 2), "{ops:#?}");
        let at = |s: &str| ops.iter().position(|o| o.contains(s)).unwrap();
        assert!(at("tmut") < at("tmudref") && at("tmudref") < at("tmus,") && at("tmus,") < at("ldtmu"), "{ops:#?}");
        let p1 = p.fs.code.uniforms.iter().find(|u| u.contents == 41).expect("P1");
        assert_eq!(p1.data, 0x5, "unit 0, per-pixel mask, 32-bit output");
        assert_eq!(p.fs.threads, 4);
    });
    let fs = fs_of("samplerCubeShadow", "textureGrad(t, vec4(v.xyz, u_ref), gx, gy)");
    with_program(VS, &fs, 0, 1, |p| {
        let ops = listing(&p.fs.code);
        let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
        assert_eq!((n("tmur"), n("tmub"), n("tmudref"), n("tmuscm")), (1, 1, 1, 1), "{ops:#?}");
        assert_eq!(p.fs.threads, 2, "five TMU writes: the input FIFO's 16 slots over 4 threads do not hold them");
    });
}

/// The draw-time half: a depth binding's `compare_op` (the GL core's
/// TEXTURE_COMPARE_FUNC, GL's order) is the compare function of its sampler
/// record (`v3d::texture::compare_function`, what `sampler_for` packs and
/// keys its cache on), and that function, which the QPU interpreter's TMUDREF
/// takes in the same order as the hardware field, makes the look-up compare as
/// GL says. Before plan #354 `sampler_for` packed with the compare-less
/// `v3d_tex_pack_sampler`, the record held NEVER whatever the texture said,
/// and a shadow draw compared never (dEQP texture.shadow, RC2).
///
/// Per function: the texels 0.25, 0.5 and 0.75 against the reference 0.5. A
/// compare that is off (no TEXTURE_COMPARE_MODE, `COMPARE_OFF`) packs NEVER,
/// as Mesa's compare-off word does, and a colour binding carries none.
#[test]
fn the_draw_states_compare_op_is_the_look_ups_compare() {
    use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F, GLES_TEXFMT_RGBA8};
    use yakogl::v3d::texture::{compare_function, sampler_compare_bits, COMPARE_OFF, SAMPLER_COMPARE_SHIFT};

    let tex = depth_2x2(1);
    let fs = fs_of("sampler2DShadow", "texture(t, vec3(v.xy, u_ref))");
    let depths = [0.25, 0.5, 0.75];
    let centres = [(0.25, 0.25), (0.75, 0.25), (0.75, 0.75)];
    let binding = |format: u8, compare_op: u8| GlesTextureBind { format, compare_op, ..GlesTextureBind::ZERO };
    for (op, func) in FUNCS.into_iter().enumerate() {
        let b = binding(GLES_TEXFMT_R32F, op as u8);
        let f = compare_function(&b);
        assert_eq!(f, op as u32, "{func:?}: the record's field is GL's order");
        assert_eq!(sampler_compare_bits(&b), f << SAMPLER_COMPARE_SHIFT);
        for (k, (s, t)) in centres.into_iter().enumerate() {
            let got = run_fs(&fs, [s, t, 0.0, 1.0], &tex, Sampler::NEAREST_CLAMP, FUNCS[f as usize], &[0.5]);
            assert_eq!(got, compare(func, 0.5, depths[k]), "{func:?} against {}", depths[k]);
        }
    }
    /* off: NEVER in the record, so a look-up that does not compare reads 0 */
    let off = binding(GLES_TEXFMT_R32F, COMPARE_OFF as u8);
    assert_eq!((compare_function(&off), sampler_compare_bits(&off)), (0, 0));
    let colour = binding(GLES_TEXFMT_RGBA8, 3);
    assert_eq!((compare_function(&colour), sampler_compare_bits(&colour)), (0, 0), "no compare on a colour texture");
}

/* ---- from the GL entry points: the sampler record a draw holds ----------
 *
 * The cases above run the look-up forms; these start at glTexParameteri and
 * glSamplerParameteri, draw through the real GL core and V3D back end (the
 * job builder stood in for, as in tests/es3_shadow_v3d_draw.rs, which pins
 * only the compare field) and read the whole six-word sampler record that
 * each draw's FS binding of unit 0 points at. The words are Mesa's
 * v3dx_state.c (docs/notes/v3d-shadow.md, compare-words), as
 * src/v3d/backend_tests.rs pins them from the back end down: word 0 =
 * 0x00100007 | fn << 4 for a NEAREST sampler and 0x00100004 | fn << 4 for a
 * LINEAR one, word 1 0x00490000, words 2-5 zero, for a one-level texture
 * clamped on every axis (WRAP_R too: GL's default REPEAT changes word 1);
 * compare off and a non-depth format (bypass) give
 * fn 0. */
#[cfg(feature = "host-test")]
mod gl_records {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::gl::consts::*;
    use yakogl::gl::context;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::yakogl_backend_v3d;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the job builder: records each draw's FS sampler bindings ---- */

    static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// The FS sampler bindings of every draw handed to the job builder.
    static DRAWS: Mutex<Vec<Vec<V3dJobTmu>>> = Mutex::new(Vec::new());

    fn log_lines() -> Vec<String> {
        LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn draws() -> Vec<Vec<V3dJobTmu>> {
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_begin(
        j: *mut V3dJob,
        t: *const V3dJobTarget,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool {
        // SAFETY: the backend passes its live job, target and lists.
        let (j, t) = unsafe { (&mut *j, *t) };
        *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
        // SAFETY: as above.
        unsafe {
            (*bcl).used = 16;
            (*ind).used = 256;
        }
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let d = unsafe { &*d };
        let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
            Vec::new()
        } else {
            // SAFETY: `num_tmu` records, live for the call.
            unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
        };
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(tmu);
        // SAFETY: the backend passes its live job.
        unsafe {
            (*j).draws += 1;
            (*(*j).bcl).used += 120;
        }
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        _c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job; its lists are live.
        unsafe {
            (*(*j).bcl).used += 1;
            (*(*j).rcl).used = 200;
        }
        true
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, 1, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
    }

    /// The tile-size table of v3d_frame.c, as the backend's unit tests fake it.
    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_bpp(
        width: u32,
        height: u32,
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        t: *mut V3dTiling,
    ) {
        const SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
        let idx = match rts {
            0 | 1 => 0,
            2 => 1,
            _ => 2,
        } + if msaa { 2 } else { 0 }
            + max_bpp.min(2) as usize;
        let (tw, th) = SIZES[idx];
        let tiles = V3dTiling {
            tile_w: tw,
            tile_h: th,
            tiles_x: width.div_ceil(tw),
            tiles_y: height.div_ceil(th),
            supertile_w: 1,
            supertile_h: 1,
            frame_w_supertiles: width.div_ceil(tw),
            frame_h_supertiles: height.div_ceil(th),
        };
        // SAFETY: the backend's out-parameter.
        unsafe { *t = tiles };
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _d: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
        // SAFETY: the backend's parameters.
        unsafe {
            let n = (*t).tiles_x * (*t).tiles_y;
            *ta = n * 128 + 8192;
            *ts = n * 256;
        }
    }

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
    /// object of this test's own, reached through the handle only (RTLD_LOCAL).
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_shadow_v3d_records");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object */
            let st = Command::new("cc")
                .current_dir(&root)
                .env("TMPDIR", &dir)
                .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
                .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
                .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
                .arg("-Ithird_party/mesa-26.2.1/include")
                .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
                .arg(&so)
                .status()
                .expect("cc runs");
            assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
            let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
            // SAFETY: a path of a library this test just built; RTLD_NOW.
            let h = unsafe { dlopen(path.as_ptr(), 2) };
            assert!(!h.is_null(), "the layout code loads");
            let _ = std::fs::remove_file(&so);
            h as usize
        }) as *mut c_void
    }

    fn sym(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        // SAFETY: a handle `lib` made and a NUL-terminated name.
        let p = unsafe { dlsym(lib(), c.as_ptr()) };
        assert!(!p.is_null(), "{name} in v3d_tex.c");
        p
    }

    /// The trampolines: the library's `extern "C"` texture functions
    /// (src/v3d/ffi.rs), each handed to the real C.
    macro_rules! trampoline {
        ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
            #[no_mangle]
            unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
                // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
                let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                    unsafe { std::mem::transmute(sym(stringify!($name))) };
                // SAFETY: the library's call, passed on.
                unsafe { f($($arg),*) }
            }
        };
    }

    /* The signatures are src/v3d/ffi.rs's, kept as written (see
     * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
    #[rustfmt::skip]
    mod trampolines {
        use super::*;

        trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
        trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                     j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
        trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
        trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                      src: *const u8, stride: u32) -> bool);
        trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                     dst: *mut u8, stride: u32) -> bool);
        trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
        trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
        trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
        trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
    }

    /* ---- the platform: memory that answers, jobs that finish at once ---- */

    struct Alloc {
        mem: Box<[u64]>,
        size: usize,
    }
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        submitted: usize,
    }
    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0 }))
    }

    /// `n` bytes of GPU memory at `gpu`, inside one allocation.
    fn gpu_bytes(gpu: u32, n: usize) -> Vec<u8> {
        with_platform(|p| {
            let (base, a) = p.allocs.range(..=u64::from(gpu)).next_back().expect("an allocation at the address");
            let off = (u64::from(gpu) - base) as usize;
            assert!(off + n <= a.size, "{n} bytes at {gpu:#x} inside the allocation");
            // SAFETY: inside the allocation (checked above).
            unsafe { std::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
        })
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = 0x0100_0000 + p.next;
            p.next += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, Alloc { mem, size });
            0
        })
    }
    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if !m.impl_.is_null() {
            with_platform(|p| p.allocs.remove(&m.gpu));
        }
        0
    }
    unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
            return -22;
        }
        // SAFETY: inside the allocation (checked above).
        unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
        0
    }
    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
    unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        with_platform(|p| {
            p.submitted += 1;
            p.submitted as i64
        })
    }
    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }
    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.submitted as i64)
    }
    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
        if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = 65536 };
            return 0;
        }
        if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
                Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                    // SAFETY: inside the allocation.
                    q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                    0
                }
                _ => -22,
            });
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61;
        }
        -22
    }
    /// The library logs every line as `log("%s", line)` (`osal::log_str`).
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
    }
    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    fn osal() -> &'static OsalOps {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        Box::leak(Box::new(OsalOps {
            init: Some(p_init),
            shutdown: None,
            gpu_alloc: Some(p_gpu_alloc),
            gpu_free: Some(p_gpu_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: Some(p_gpu_write),
            gpu_read: None,
            cache_flush: Some(p_cache),
            cache_invd: Some(p_cache),
            submit: Some(p_submit),
            wait: Some(p_wait),
            completed: Some(p_completed),
            reg_read: None,
            reg_write: None,
            query: Some(p_query),
            log: Some(log),
            wait_vblank: None,
            present: None,
            lock: None,
            unlock: None,
            thread_id: None,
            display_busy: None,
        }))
    }

    /* ---- the context ---- */

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The one context and back end of the module; the tests take turns and
    /// each starts with nothing recorded and nothing pending. The lock also
    /// serialises the interpreter's own compiles with the GL core's.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the module.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", log_lines());
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", log_lines());
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        g
    }

    /* ---- helpers ---- */

    use yakogl::gl::buffer::glBindBuffer;
    use yakogl::gl::draw::*;
    use yakogl::gl::es3::{glBindSampler, glDeleteSamplers, glGenSamplers, glSamplerParameteri};
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

    const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                      void main() { v = a_pos; gl_Position = a_pos; }\n";
    const FS_SHADOW: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler2DShadow t;\n\
                             in highp vec4 v;\nout vec4 o;\nvoid main() { o = vec4(texture(t, vec3(v.xy, 0.5))); }\n";
    const FS_COLOUR: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler2D t;\n\
                             in highp vec4 v;\nout vec4 o;\nvoid main() { o = texture(t, v.xy); }\n";

    /// GL's compare functions in the order of the record's field.
    const GL_FUNCS: [GLenum; 8] =
        [GL_NEVER, GL_LESS, GL_EQUAL, GL_LEQUAL, GL_GREATER, GL_NOTEQUAL, GL_GEQUAL, GL_ALWAYS];

    fn err() -> GLenum {
        // SAFETY: plain GL calls.
        unsafe {
            let e = glGetError();
            while glGetError() != GL_NO_ERROR {}
            e
        }
    }

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let c = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [c.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text} did not compile");
            s
        }
    }

    /// `VS` + `fs` as the current program, its sampler `t` on unit 0.
    fn program(fs: &str) -> GLuint {
        let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
        // SAFETY: plain GL calls; NUL-terminated names.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "{fs} did not link");
            glUseProgram(p);
            glUniform1i(glGetUniformLocation(p, c"t".as_ptr()), 0);
            glDeleteShader(v);
            glDeleteShader(f);
            p
        }
    }

    /// A 2x2 texture on unit 0, one level, clamped, `filter` (GL_NEAREST or
    /// GL_LINEAR) for minification and magnification: GL_DEPTH_COMPONENT24
    /// when `depth`, GL_RGBA8 otherwise, with GL_COMPARE_REF_TO_TEXTURE and
    /// `func` either way (state a colour texture keeps but ignores).
    fn texture(depth: bool, filter: GLenum, func: GLenum) -> GLuint {
        let mut t = 0;
        // SAFETY: plain GL calls; the texels outlive the upload.
        unsafe {
            glGenTextures(1, &mut t);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, t);
            if depth {
                let texels: [u32; 4] = [0x4000_0000, 0x8000_0000, 0x8000_0000, 0xc000_0000];
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_DEPTH_COMPONENT24 as GLint,
                    2,
                    2,
                    0,
                    GL_DEPTH_COMPONENT,
                    GL_UNSIGNED_INT,
                    texels.as_ptr() as *const c_void,
                );
            } else {
                let texels = [0x80u8; 16];
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8 as GLint,
                    2,
                    2,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr() as *const c_void,
                );
            }
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, func as GLint);
        }
        assert_eq!(err(), GL_NO_ERROR, "the texture: {:?}", log_lines());
        t
    }

    fn set_func(func: GLenum) {
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, func as GLint) };
    }

    fn set_mode(mode: GLenum) {
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, mode as GLint) };
    }

    fn draw_triangle(what: &str) {
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        // SAFETY: TRI is static and outlives the draw.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(err(), GL_NO_ERROR, "{what}: {:?}", log_lines());
    }

    /// The (address, six words) of the sampler record of unit 0 of each draw
    /// since the last call, once the job is done; the draws are forgotten.
    fn records() -> Vec<(u32, [u32; 6])> {
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        let got = draws()
            .iter()
            .filter(|tmu| !tmu.is_empty())
            .map(|tmu| {
                let u = tmu.iter().find(|u| u.unit == 0).expect("unit 0 sampled");
                let b = gpu_bytes(u.sampler, V3D_SAMPLER_STATE_BYTES);
                let w = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
                (u.sampler, [w(0), w(1), w(2), w(3), w(4), w(5)])
            })
            .collect();
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        got
    }

    /// Mesa's words (v3dx_state.c, docs/notes/v3d-shadow.md): `func` 0 for
    /// compare off and for a non-depth format.
    fn mesa(nearest: bool, func: u32) -> [u32; 6] {
        [if nearest { 0x0010_0007 } else { 0x0010_0004 } | func << 4, 0x0049_0000, 0, 0, 0, 0]
    }

    fn delete(p: GLuint, ts: &[GLuint]) {
        // SAFETY: names this test made.
        unsafe {
            glBindSampler(0, 0);
            glDeleteTextures(ts.len() as GLsizei, ts.as_ptr());
            if p != 0 {
                glDeleteProgram(p);
            }
        }
    }

    /// Each of the eight GL compare functions set with glTexParameteri on a
    /// DEPTH_COMPONENT24 texture, under NEAREST and under LINEAR: the draw's
    /// whole sampler record is Mesa's, one record per function. Revert
    /// check: packing with the compare-off packer gives 0x00100007 or
    /// 0x00100004 for every function.
    #[test]
    fn each_gl_compare_function_packs_mesas_record() {
        let _g = rig();
        let p = program(FS_SHADOW);
        for (nearest, filter) in [(true, GL_NEAREST), (false, GL_LINEAR)] {
            let t = texture(true, filter, GL_NEVER);
            for func in GL_FUNCS {
                set_func(func);
                draw_triangle(&format!("compare {func:#x}"));
            }
            let got = records();
            assert_eq!(got.len(), 8, "eight textured draws: {:?}", log_lines());
            for (k, (rec, words)) in got.iter().enumerate() {
                assert_eq!(*words, mesa(nearest, k as u32), "nearest {nearest}, GL function {:#x}", GL_FUNCS[k]);
                assert!(got[..k].iter().all(|(r, _)| r != rec), "GL function {:#x}: a record of its own", GL_FUNCS[k]);
            }
            delete(0, &[t]);
        }
        delete(p, &[]);
    }

    /// COMPARE_MODE = NONE on a depth texture whose function is GREATER is
    /// the compare-off record, NEAREST and LINEAR (V3D 4.2 has no enable
    /// bit; a look-up that does not compare is the NEVER field).
    #[test]
    fn compare_off_packs_the_compare_off_record() {
        let _g = rig();
        let p = program(FS_SHADOW);
        for (nearest, filter) in [(true, GL_NEAREST), (false, GL_LINEAR)] {
            let t = texture(true, filter, GL_GREATER);
            set_mode(GL_NONE);
            draw_triangle("compare off");
            let got = records();
            assert_eq!(got.len(), 1, "{:?}", log_lines());
            assert_eq!(got[0].1, mesa(nearest, 0), "nearest {nearest}");
            delete(0, &[t]);
        }
        delete(p, &[]);
    }

    /// A sampler object's compare mode and function replace the texture's
    /// (ES 3.0.6 3.8.2), whichever way they differ, and unbinding it brings
    /// the texture's back.
    #[test]
    fn a_sampler_objects_compare_overrides_the_textures() {
        let _g = rig();
        let p = program(FS_SHADOW);
        let t = texture(true, GL_NEAREST, GL_EQUAL);
        let mut s = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenSamplers(1, &mut s);
            for (pname, value) in [
                (GL_TEXTURE_MIN_FILTER, GL_LINEAR),
                (GL_TEXTURE_MAG_FILTER, GL_LINEAR),
                (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
                (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
                (GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE),
                (GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE),
                (GL_TEXTURE_COMPARE_FUNC, GL_GREATER),
            ] {
                glSamplerParameteri(s, pname, value as GLint);
            }
            glBindSampler(0, s);
        }
        draw_triangle("the sampler object's GREATER (LINEAR)");
        // SAFETY: plain GL call.
        unsafe { glSamplerParameteri(s, GL_TEXTURE_COMPARE_FUNC, GL_NOTEQUAL as GLint) };
        draw_triangle("the sampler object's NOTEQUAL");
        // SAFETY: plain GL call.
        unsafe { glSamplerParameteri(s, GL_TEXTURE_COMPARE_MODE, GL_NONE as GLint) };
        draw_triangle("the sampler object's compare off over the texture's EQUAL");
        // SAFETY: plain GL call.
        unsafe { glBindSampler(0, 0) };
        draw_triangle("the texture's EQUAL (NEAREST)");
        let got: Vec<[u32; 6]> = records().into_iter().map(|(_, w)| w).collect();
        assert_eq!(got, [mesa(false, 4), mesa(false, 5), mesa(false, 0), mesa(true, 2)], "{:?}", log_lines());
        // SAFETY: plain GL call.
        unsafe { glDeleteSamplers(1, &s) };
        delete(p, &[t]);
    }

    /// An RGBA8 texture with the compare on and a function set is not
    /// compared (ES 3.0.6 3.8.15): the bypass record is compare off's,
    /// whichever function GL holds, one record for both draws of the job.
    #[test]
    fn a_colour_texture_with_compare_on_packs_the_bypass_record() {
        let _g = rig();
        let p = program(FS_COLOUR);
        for (nearest, filter) in [(true, GL_NEAREST), (false, GL_LINEAR)] {
            let t = texture(false, filter, GL_GREATER);
            draw_triangle("RGBA8, GREATER");
            set_func(GL_LEQUAL);
            draw_triangle("RGBA8, LEQUAL");
            let got = records();
            assert_eq!(got.len(), 2, "{:?}", log_lines());
            assert_eq!(got[0].1, mesa(nearest, 0), "nearest {nearest}, GREATER");
            assert_eq!(got[1].1, mesa(nearest, 0), "nearest {nearest}, LEQUAL");
            assert_eq!(got[0].0, got[1].0, "one bypass record per job");
            delete(0, &[t]);
        }
        delete(p, &[]);
    }

    /// Two draws of one job with different functions hold two records, and a
    /// third with the first's function finds the first again.
    #[test]
    fn two_functions_in_one_job_are_two_records() {
        let _g = rig();
        let p = program(FS_SHADOW);
        let t = texture(true, GL_NEAREST, GL_LESS);
        draw_triangle("LESS");
        set_func(GL_GEQUAL);
        draw_triangle("GEQUAL");
        set_func(GL_LESS);
        draw_triangle("LESS again");
        let got = records();
        assert_eq!(got.len(), 3, "{:?}", log_lines());
        assert_eq!((got[0].1, got[1].1, got[2].1), (mesa(true, 1), mesa(true, 6), mesa(true, 1)));
        assert_ne!(got[0].0, got[1].0, "LESS and GEQUAL are two records");
        assert_eq!(got[0].0, got[2].0, "LESS is found again");
        delete(p, &[t]);
    }
}
