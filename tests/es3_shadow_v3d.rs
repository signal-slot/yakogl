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
/// depth texture's does), handed to `f`. The fragment stage reads its key
/// from the compile key the GL core sets around a keyed compile
/// (`set_compile_lod_key`, `select_fragment`), so it is set here the same
/// way and put back after.
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
    let old = yakogl::shaderc::set_compile_lod_key(fk);
    let p = compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, vk, fk, &a);
    yakogl::shaderc::set_compile_lod_key(old);
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
