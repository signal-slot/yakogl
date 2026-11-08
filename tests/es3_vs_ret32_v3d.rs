// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Vertex-stage look-ups of the 32-bit texture classes on the V3D 4.2
//! (R32F, RGBA32F, RGBA32UI, RGBA32I and the depth textures the GL core
//! stores as R32F: `gles_texfmt_return_32`), item v3d-qpu-grad, designed in
//! docs/notes/v3d-vs-ret32.md.
//!
//! The TMU returns these classes at 32 bits a channel (TMU Config Parameter
//! 1 bit 0, "Output Type 32-bit"), one word per channel. A program's vertex
//! record is sensitive to the format of each binding below 16 its look-ups
//! read (`LodKey::ret32`, `v3d_sensitivity`), the GL core keys the draw by
//! the bound formats (`stage_lod_key`), and the vertex key reaches the vertex
//! and the coordinate shader (`compile_program_keyed` -> `select_vertex`).
//! Before, a vertex look-up was always compiled for the 16-bit output and
//! the back end refused the draw of a 32-bit texture with it
//! (`v3d::texture::refusal_for`).
//!
//! Here: the forms build through `SHC_ONLINE` (glCompileShader and
//! glLinkProgram); the vertex record's sensitivity and its keyed variant go
//! through the back end's own entry points, as the GL core calls them; and
//! the keyed programs run on the QPU interpreter (`v3d::sim`) against the
//! texels themselves, values no f16 holds, so a 16-bit read would show.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_vs_ret32_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, Code, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema, SHC_ONLINE};
use yakogl::shaderc::{LodKey, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

const FS_C: &str = "#version 300 es\nprecision mediump float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n";

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

/// glLinkProgram of `vs` and `fs` (`fs` becomes the program record).
fn link(vs: &ShcShader, fs: &mut ShcShader) -> Result<(), String> {
    let link = SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(vs, fs, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

fn vs_of(sampler: &str, expr: &str) -> String {
    format!(
        "#version 300 es\nuniform highp {sampler} t;\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 c;\n\
         void main() {{\n  highp vec4 p = a_pos;\n  c = {expr};\n  gl_Position = a_pos;\n}}\n"
    )
}

/// The vertex-stage explicit-level forms of the 32-bit classes' samplers,
/// as (sampler, expression of `t` and `p`).
const FORMS: [(&str, &str); 9] = [
    ("sampler2D", "textureLod(t, p.xy, 0.0)"),
    ("sampler2D", "vec4(textureLod(t, p.xy, 0.0).x)"),
    ("sampler2D", "texelFetch(t, ivec2(p.xy), 0)"),
    ("sampler2D", "texture(t, p.xy)"),
    ("isampler2D", "vec4(texelFetch(t, ivec2(p.xy), 0))"),
    ("usampler2D", "vec4(textureLod(t, p.xy, 0.0))"),
    ("sampler2DArray", "textureLod(t, p.xyz, 0.0)"),
    ("sampler3D", "textureLod(t, p.xyz, 0.0)"),
    ("samplerCube", "textureLod(t, p.xyz, 0.0)"),
];

/// Every form builds and links (the program's vertex key is 0 at link: the
/// key comes with the draw), its vertex record is sensitive at binding 0
/// (`ret32`, whatever the sampler type: the format decides at the draw),
/// and its variant for that key, linked with the base fragment stage (the
/// GL core's `lod_variant_for` with key [k, 0]), is a program whose vertex
/// and coordinate shaders ask for the 32-bit output: P1 bit 0 set, where
/// the base program has it clear.
#[test]
fn vertex_ret32_forms_build_and_key() {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let p1 = |s: &[yakogl::shaderc::ShcV3dUniform]| -> Vec<u32> {
        s.iter().filter(|u| u.contents == 41).map(|u| u.data & 0xff).collect()
    };
    let sense = SHC_ONLINE.lod_sensitivity.expect("the online back end reports a sensitivity");
    let variant = SHC_ONLINE.sampler_variant.expect("the online back end builds variants");
    for (sampler, expr) in FORMS {
        yakogl::shaderc::online::backend::forget_compiled();
        /* the position reads the look-up, so the coordinate shader samples too */
        let vs = vs_of(sampler, expr).replace("gl_Position = a_pos;", "gl_Position = a_pos + c;");
        let what = format!("{sampler} {expr}");
        let vso = compile(SHC_STAGE_VERTEX, &vs).unwrap_or_else(|e| panic!("{what}: {e}"));
        let mut prog = compile(SHC_STAGE_FRAGMENT, FS_C).expect("the fragment shader compiles");
        link(&vso, &mut prog).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(p1(prog.v3d.vs.uniform_stream()).iter().all(|&w| w == 4), "{what}: the base vertex shader, 16-bit");
        // SAFETY: a vertex shader record.
        let k = LodKey::from_bits(unsafe { sense(&vso) });
        assert_eq!(k.ret32, 1, "{what}: the vertex record is sensitive at binding 0");
        let key = LodKey { ret32: 1, ..LodKey::NONE };
        let mut vvar = ShcShader::ZERO;
        // SAFETY: a vertex record and a real output.
        assert_eq!(unsafe { variant(&vso, key.bits(), &mut vvar, core::ptr::null_mut(), 0) }, 0, "{what}");
        let mut base = ShcShader::ZERO;
        // SAFETY: the program record and a real output.
        assert_eq!(unsafe { variant(&prog, 0, &mut base, core::ptr::null_mut(), 0) }, 0, "{what}");
        link(&vvar, &mut base).unwrap_or_else(|e| panic!("{what}: the keyed link: {e}"));
        for (stage, s) in [("vs", base.v3d.vs.uniform_stream()), ("cs", base.v3d.cs.uniform_stream())] {
            let words = p1(s);
            assert!(!words.is_empty() && words.iter().all(|&w| w == 5), "{what}: {stage} P1 {words:?}");
        }
        for sh in [&base, &vvar, &vso, &prog] {
            yakogl::shaderc::online::backend::release(sh);
        }
    }
}

// ---- values, on the QPU interpreter ----------------------------------------

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

/// `vs` (+ [`FS_C`]) compiled with the vertex key `ret32`, handed to `f`.
fn with_program<R>(vs: &str, ret32: u32, f: impl FnOnce(&V3dProgram<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(FS_C).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let key = LodKey { ret32, ..LodKey::NONE };
    let p = compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, key, LodKey::NONE, &a)
        .expect("the program compiles");
    f(&p)
}

/// One NEAREST texture per binding, by index (P0's unit).
struct Bound {
    tex: Texture,
    integer: bool,
}

/// The program's vertex shader (`coord` false) or coordinate shader on the
/// interpreter with every lane's `a_pos` = `pos` and `textures` bound by
/// binding, `user` the user uniforms: `c`'s raw words (the last four VPM
/// output rows of lane 0) of the vertex shader; the run's look-up count.
fn run(p: &V3dProgram<'_>, coord: bool, pos: [f32; 4], textures: &[Bound], user: &[f32]) -> ([u32; 4], usize) {
    let prog = if coord { &p.cs } else { &p.vs };
    let u: Vec<u32> = prog
        .code
        .uniforms
        .iter()
        .map(|u| match u.contents {
            0 | 40 | 41 => u.data,
            1 => user[u.data as usize].to_bits(),
            2..=5 => 1.0f32.to_bits(),
            44 | 45 => {
                let (w, h) = textures[u.data as usize].tex.size();
                if u.contents == 44 {
                    w
                } else {
                    h
                }
            }
            c => panic!("uniform contents {c} (data {}) not modelled by this test", u.data),
        })
        .collect();
    let rec = p.attributes.iter().find(|e| e.0 == 0).and_then(|e| e.1).expect("a_pos is read") as usize;
    let rows: Vec<[u32; LANES]> =
        pos.iter().take(prog.vattr_sizes[rec] as usize).map(|c| [c.to_bits(); LANES]).collect();
    let integer_textures = textures.iter().enumerate().filter(|(_, b)| b.integer).fold(0, |m, (i, _)| m | 1 << i);
    let env = Env {
        textures: textures.iter().map(|b| b.tex.clone()).collect(),
        samplers: vec![Sampler::NEAREST_CLAMP; textures.len()],
        threads: prog.threads,
        integer_textures,
        ..Env::default()
    };
    let o = sim::run(prog.code.words, &u, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .unwrap_or_else(|e| panic!("the program runs: {e}\n{:#?}", listing(&prog.code)));
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
    let v = &out[out.len() - 4..];
    (core::array::from_fn(|c| v[c][0]), o.tmu_lookups)
}

/// A 4 x 4 texture whose every texel is `t`.
fn flat(t: [f32; 4], integer: bool) -> Bound {
    Bound { tex: Texture::flat(4, 4, vec![t; 16]), integer }
}

/// Floats no f16 holds: a 16-bit read rounds every one of them.
const R32: [f32; 4] = [1.000_122_1, -3.217_891_2, 65_537.5, 1.0e-7];

const POS: [f32; 4] = [0.4, 0.6, 0.5, 1.0];

/// R32F and RGBA32F read in the vertex stage with the vertex key: each
/// channel is its texel's float exactly; one channel read asks for one
/// word. Without the key (the 16-bit read the back end refuses at the
/// draw) the same program would return f16-rounded halves: the base
/// program differs. Revert check: `Sel::ret32` left 0 in select_vertex
/// gives the rounded values.
#[test]
fn vertex_ret32_float_texels_are_exact() {
    let vs4 = vs_of("sampler2D", "textureLod(t, p.xy, 0.0)");
    let got = with_program(&vs4, 1, |p| run(p, false, POS, &[flat(R32, false)], &[]).0);
    assert_eq!(got.map(f32::from_bits), R32, "RGBA32F: four exact words");
    let vs1 = vs_of("sampler2D", "vec4(textureLod(t, p.xy, 0.0).x, 0.0, 0.0, 1.0)");
    let (got, p0) = with_program(&vs1, 1, |p| {
        let p0: Vec<u32> = p.vs.code.uniforms.iter().filter(|u| u.contents == 40).map(|u| u.data & 0xf).collect();
        (run(p, false, POS, &[flat([R32[0], 0.0, 0.0, 1.0], false)], &[]).0, p0)
    });
    assert_eq!(f32::from_bits(got[0]), R32[0], "R32F: the one channel exact");
    assert_eq!(p0, [0x1], "one word asked for one channel");
    let (rounded, _) = with_program(&vs4, 0, |p| run(p, false, POS, &[flat(R32, false)], &[]));
    assert_ne!(rounded.map(f32::from_bits), R32, "the unkeyed 16-bit read rounds");
}

/// RGBA32UI and RGBA32I: the raw words, no 16-bit sign or zero extension
/// (values past 16 bits, and negative ones), through texelFetch and
/// textureLod, read back as bits.
#[test]
fn vertex_ret32_integer_texels_are_whole_words() {
    let ui: [u32; 4] = [100_000, 4_000_000_000, 65_536, 7];
    let si: [i32; 4] = [-100_000, 2_000_000_000, -1, 70_000];
    let vs = vs_of("usampler2D", "uintBitsToFloat(texelFetch(t, ivec2(p.xy * 4.0), 0))");
    let got = with_program(&vs, 1, |p| run(p, false, POS, &[flat(ui.map(|v| v as f32), true)], &[]).0);
    assert_eq!(got, ui.map(|v| (v as f32) as u32), "RGBA32UI");
    let vs = vs_of("isampler2D", "intBitsToFloat(textureLod(t, p.xy, 0.0))");
    let got = with_program(&vs, 1, |p| run(p, false, POS, &[flat(si.map(|v| v as f32), true)], &[]).0);
    assert_eq!(got, si.map(|v| v as u32), "RGBA32I");
}

/// The coordinate shader carries the same key: its look-up returns the
/// exact words too (the position reads the texel).
#[test]
fn the_coordinate_shader_reads_32_bit_words_too() {
    let vs = "#version 300 es\nuniform highp sampler2D t;\nlayout(location = 0) in highp vec4 a_pos;\n\
              out highp vec4 c;\nvoid main() {\n  c = textureLod(t, a_pos.xy, 0.0);\n  gl_Position = c;\n}\n";
    with_program(vs, 1, |p| {
        let p1: Vec<u32> = p.cs.code.uniforms.iter().filter(|u| u.contents == 41).map(|u| u.data & 0xff).collect();
        assert_eq!(p1, [5], "the coordinate shader's P1");
        let (_, n) = run(p, true, POS, &[flat(R32, false)], &[]);
        assert_eq!(n, 1, "the coordinate shader samples");
    });
}

/// Two 32-bit vec4 look-ups in one vertex shader fill the TMU output FIFO
/// (4 words each, `TMU_OUTPUT_WORDS`): both are read in full and summed,
/// on the interpreter that models the FIFO. And a 16-bit binding beside a
/// 32-bit one: the key is per binding, the RGBA8-class texture (bit clear)
/// read as f16 halves, the R32F one (bit set) as words.
#[test]
fn two_vertex_look_ups_and_a_mixed_pair() {
    let vs = "#version 300 es\nuniform highp sampler2D t0;\nuniform highp sampler2D t1;\n\
              layout(location = 0) in highp vec4 a_pos;\nout highp vec4 c;\n\
              void main() {\n  c = textureLod(t0, a_pos.xy, 0.0) * 2.0 + textureLod(t1, a_pos.xy, 0.0);\n  \
              gl_Position = a_pos;\n}\n";
    let b: [f32; 4] = [0.000_976_7, 1.0e6, -0.333_333_34, 5.000_001];
    let want: [f32; 4] = core::array::from_fn(|i| R32[i] * 2.0 + b[i]);
    let (got, n) = with_program(vs, 3, |p| run(p, false, POS, &[flat(R32, false), flat(b, false)], &[]));
    assert_eq!(n, 2);
    assert_eq!(got.map(f32::from_bits), want, "both RGBA32F look-ups, exact");
    let half: [f32; 4] = [0.5, 0.25, 0.125, 1.0];
    let want: [f32; 4] = core::array::from_fn(|i| half[i] * 2.0 + b[i]);
    let got = with_program(vs, 2, |p| run(p, false, POS, &[flat(half, false), flat(b, false)], &[]).0);
    assert_eq!(got.map(f32::from_bits), want, "binding 0 at 16 bits, binding 1 at 32");
}

/// textureGrad in the vertex stage of an R32F class texture: the level from
/// the gradients (docs/notes/v3d-qpu-grad.md) and the 32-bit read compose.
/// A 4 x 4 texture of three levels, each its own exact float; the
/// gradients pick level 1 (lambda = 1.26).
#[test]
fn a_vertex_texture_grad_of_a_32_bit_texture() {
    let level = |l: u32, v: f32| {
        let n = 4 >> l;
        Level { width: n, height: n, faces: vec![vec![[v, 0.0, 0.0, 1.0]; (n * n) as usize]] }
    };
    let vals = [1.000_122_1f32, 2.000_244_1, 3.000_366_2];
    let tex = Texture {
        cube: false,
        min: MinFilter::NearestMipmapNearest,
        levels: (0..3).map(|l| level(l, vals[l as usize])).collect(),
    };
    let vs = "#version 300 es\nuniform highp sampler2D t;\nuniform highp vec2 gx, gy;\n\
              layout(location = 0) in highp vec4 a_pos;\nout highp vec4 c;\n\
              void main() {\n  c = textureGrad(t, a_pos.xy, gx, gy);\n  gl_Position = a_pos;\n}\n";
    let got = with_program(vs, 1, |p| {
        let prog = &p.vs;
        let u: Vec<u32> = prog
            .code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                1 => [0.6f32, 0.0, 0.0, 0.0][u.data as usize].to_bits(),
                2..=5 => 1.0f32.to_bits(),
                44 | 45 => 4,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
        let rows: Vec<[u32; LANES]> = POS.iter().map(|c| [c.to_bits(); LANES]).collect();
        let sampler = Sampler {
            mag: Filter::Nearest,
            min: MinFilter::NearestMipmapNearest,
            wrap_s: Wrap::ClampToEdge,
            wrap_t: Wrap::ClampToEdge,
        };
        let env = Env { textures: vec![tex.clone()], samplers: vec![sampler], threads: prog.threads, ..Env::default() };
        let o = sim::run(prog.code.words, &u, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
            .unwrap_or_else(|e| panic!("the program runs: {e}\n{:#?}", listing(&prog.code)));
        let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
        f32::from_bits(out[out.len() - 4][0])
    });
    /* |dPdx * size| = 0.6 * 4 = 2.4: lambda 1.26, level 1 */
    assert_eq!(got, vals[1]);
}
