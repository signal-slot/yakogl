// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D's shadow reference clamp (plan #354 item v3d-shadow-clamp-key,
//! docs/notes/v3d-shadow.md (d) and (f)): "If the texture's internal format
//! indicates a fixed-point depth texture, then Dt and Dref are clamped to the
//! range [0, 1]; otherwise no clamping is performed" (ES 3.0.6 3.8.15.1).
//!
//! The TMU compares the reference as it is written to TMUDREF (Mesa writes
//! the comparator unchanged, v3d_tex.c handle_tex_src), and the QPU
//! interpreter models that (`texref::Compare { clamp_ref: false }`). So the
//! compiler clamps: a binding keyed `LodKey::fixed_depth` (the GL core sets
//! it for a DEPTH_COMPONENT16/24 or DEPTH24_STENCIL8 texture, never for
//! DEPTH_COMPONENT32F or DEPTH32F_STENCIL8) has its reference taken through
//! fmax(ref, 0) and fmin(.., 1) before TMUDREF (`online::v3d::select`).
//!
//! Here: a fragment and a vertex `texture(sampler2DShadow, vec3)` compiled
//! with and without the bit, run on the interpreter against a D16 texture
//! (texels on 16-bit steps) and a D32F one, with references -0.5, 0.5 and
//! 1.5 under GL_LEQUAL and GL_GEQUAL. With the bit the verdict is the
//! compare of the clamped reference, without it the raw one; and the key as
//! the GL core sets it (D16 set, D32F clear) gives what 3.8.15.1 and dEQP's
//! verifier (`clampValues = isFixedPoint`, tcuTexCompareVerifier.cpp) expect.
//! The draw-level half (the GL core sets the bit) is
//! tests/es3_shadow_clamp_draw_v3d.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_clamp_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{CompareFunc, Level, MinFilter, Sampler, Texture};
use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, Code, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::LodKey;

/// The compiler keeps its output in one arena per compile here, but the
/// compile key is a process-wide setting: one compile at a time.
static COMPILING: Mutex<()> = Mutex::new(());

/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

const VS_PASS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                       void main() { v = a_pos; gl_Position = a_pos; }\n";
const FS_SHADOW: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler2DShadow t;\n\
                         uniform highp float u_ref;\nin highp vec4 v;\nout vec4 o;\n\
                         void main() { o = vec4(texture(t, vec3(v.xy, u_ref))); }\n";
const VS_SHADOW: &str = "#version 300 es\nuniform highp sampler2DShadow t;\nuniform highp float u_ref;\n\
                         layout(location = 0) in highp vec4 a_pos;\nout highp vec4 c;\n\
                         void main() { c = vec4(texture(t, vec3(a_pos.xy, u_ref))); gl_Position = a_pos; }\n";
const FS_PASS: &str = "#version 300 es\nprecision mediump float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n";

/// A depth texel: the depth in R (the GL core keeps depth as R32F).
fn texture_of(depths: [f32; 4]) -> Texture {
    let faces = vec![depths.map(|d| [d, 0.0, 0.0, 1.0]).to_vec()];
    Texture { cube: false, min: MinFilter::Nearest, levels: vec![Level { width: 2, height: 2, faces }] }
}

/// A DEPTH_COMPONENT16 texture as the GL core stores it: unorm16 values as
/// floats, 0 and 1 at the ends (where a clamped reference meets them).
fn d16() -> [f32; 4] {
    [0, 16384, 49151, 65535].map(|v| v as f32 / 65535.0)
}

/// A DEPTH_COMPONENT32F texture: the floats as given.
const D32F: [f32; 4] = [0.0, 0.25, 0.75, 1.0];

/// The texel centres, in the texel order of `texture_of`.
const CENTRES: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

/// ES 3.0.6 3.8.15.1, `reference OP depth`: 1.0 when it passes.
fn compare(func: CompareFunc, reference: f32, d: f32) -> f32 {
    let pass = match func {
        CompareFunc::Lequal => reference <= d,
        CompareFunc::Gequal => reference >= d,
        f => panic!("not a function of this test: {f:?}"),
    };
    f32::from(u8::from(pass))
}

/// `vs` + `fs` compiled with the vertex key `vk` and the fragment key `fk`
/// (the fragment stage reads its key from the compile key, which the GL
/// core sets around a keyed compile; set here the same way and put back).
fn with_program<R>(vs: &str, fs: &str, vk: LodKey, fk: LodKey, f: impl FnOnce(&V3dProgram<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let old = yakogl::shaderc::set_compile_lod_key(fk);
    let p = compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, vk, fk, &a);
    yakogl::shaderc::set_compile_lod_key(old);
    let p = p.unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    f(&p)
}

/// Binding 0 a depth texture (its 32-bit return), and `fixed` its
/// fixed-point bit.
fn key(fixed: bool) -> LodKey {
    LodKey { ret32: 1, fixed_depth: u32::from(fixed), ..LodKey::NONE }
}

/// The uniform stream of `c`: constants and the TMU words as compiled, the
/// one user uniform `u_ref`, the viewport 1.0, the 2x2 size of sampler 0.
fn uniforms(c: &Code<'_>, reference: f32) -> Vec<u32> {
    c.uniforms
        .iter()
        .map(|u| match u.contents {
            0 | 40 | 41 => u.data,
            1 if u.data == 0 => reference.to_bits(),
            2..=5 => 1.0f32.to_bits(),
            44..=47 if u.data == 0 => [2, 2, 1, 1][(u.contents - 44) as usize],
            /* QUNIFORM_FB_WPOS_Y_SCALE: a window target */
            0x1_0000 => (-1.0f32).to_bits(),
            c => panic!("uniform contents {c} (data {}) not modelled by this test", u.data),
        })
        .collect()
}

fn env(tex: &Texture, func: CompareFunc, threads: u32) -> Env {
    Env {
        textures: vec![tex.clone()],
        samplers: vec![Sampler::NEAREST_CLAMP],
        compare: vec![Some(func)],
        threads,
        ..Env::default()
    }
}

/// The fragment look-up compiled with `fixed`, at each texel centre of
/// `depths` with `reference` under `func`: the four results.
fn run_fs(fixed: bool, depths: [f32; 4], func: CompareFunc, reference: f32) -> [f32; 4] {
    let tex = texture_of(depths);
    with_program(VS_PASS, FS_SHADOW, LodKey::NONE, key(fixed), |p| {
        let u = uniforms(&p.fs.code, reference);
        CENTRES.map(|(s, t)| {
            let one = [1.0f32.to_bits(); LANES];
            let z = [0u32; LANES];
            let varyings = [s, t, 0.0, 1.0].iter().map(|c| sim::Varying { vary: [c.to_bits(); LANES], c: z }).collect();
            let inputs = FsInputs { w: one, w_centroid: one, z, x: z, y: z, back_facing: [false; LANES], varyings };
            let o = sim::run(p.fs.code.words, &u, &Inputs::Fragment(Box::new(inputs)), &env(&tex, func, p.fs.threads))
                .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
            assert_eq!(o.tmu_lookups, 1, "one look-up");
            let word = o.tlb[0].value[0].expect("a defined value");
            sim::half_to_f32((word & 0xffff) as u16)
        })
    })
}

/// The vertex look-up compiled with `fixed` in the vertex key: as
/// [`run_fs`], `c`'s red.
fn run_vs(fixed: bool, depths: [f32; 4], func: CompareFunc, reference: f32) -> [f32; 4] {
    let tex = texture_of(depths);
    with_program(VS_SHADOW, FS_PASS, key(fixed), LodKey::NONE, |p| {
        let u = uniforms(&p.vs.code, reference);
        let rec = p.attributes.iter().find(|e| e.0 == 0).and_then(|e| e.1).expect("a_pos is read") as usize;
        CENTRES.map(|(s, t)| {
            let rows: Vec<[u32; LANES]> =
                [s, t, 0.0, 1.0].iter().take(p.vs.vattr_sizes[rec] as usize).map(|c| [c.to_bits(); LANES]).collect();
            let o =
                sim::run(p.vs.code.words, &u, &Inputs::Vertex(VsInputs { vpm: rows }), &env(&tex, func, p.vs.threads))
                    .unwrap_or_else(|e| panic!("the vertex program runs: {e}"));
            assert_eq!(o.tmu_lookups, 1, "one look-up");
            let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
            f32::from_bits(out[out.len() - 4][0])
        })
    })
}

const REFS: [f32; 3] = [-0.5, 0.5, 1.5];
const FUNCS: [CompareFunc; 2] = [CompareFunc::Lequal, CompareFunc::Gequal];

/// Each (texture, bit, function, reference): the bit set compares the
/// clamped reference, the bit clear the raw one; the cases where the two
/// differ (1.5 LEQUAL a texel of 1, -0.5 GEQUAL a texel of 0) are among
/// them, so a compiler that ignored the bit, or clamped without it, fails.
/// Revert check: the clamp removed from `select.rs` fails every bit-set row
/// at -0.5 GEQUAL and 1.5 LEQUAL.
fn check(stage: &str, run: impl Fn(bool, [f32; 4], CompareFunc, f32) -> [f32; 4]) {
    let mut differs = 0;
    for (name, depths) in [("D16", d16()), ("D32F", D32F)] {
        for fixed in [false, true] {
            for func in FUNCS {
                for reference in REFS {
                    let r = if fixed { reference.clamp(0.0, 1.0) } else { reference };
                    let want = depths.map(|d| compare(func, r, d));
                    let got = run(fixed, depths, func, reference);
                    assert_eq!(got, want, "{stage} {name}, fixed_depth {fixed}, {func:?}, reference {reference}");
                    if want != depths.map(|d| compare(func, reference, d)) {
                        differs += 1;
                    }
                }
            }
        }
    }
    assert_eq!(differs, 4, "{stage}: -0.5 GEQUAL and 1.5 LEQUAL, on both textures, tell the bit apart");
}

#[test]
fn a_fragment_shadow_look_up_clamps_its_reference_when_keyed_fixed_depth() {
    check("fragment", run_fs);
}

#[test]
fn a_vertex_shadow_look_up_clamps_its_reference_when_keyed_fixed_depth() {
    check("vertex", run_vs);
}

/// The key as the GL core sets it (`stage_lod_key`: D16 set, D32F clear)
/// gives 3.8.15.1's answer: on D16 the reference (and the texel, already in
/// range) clamped, on D32F the raw compare. dEQP's out-of-range rows (1.1
/// and -0.1, es3fTextureShadowTests.cpp) are the same two sides; here the
/// texel 1.0 under LEQUAL and 0.0 under GEQUAL decide them.
#[test]
fn the_gl_cores_key_gives_the_es_3_0_answer_for_each_format() {
    for (reference, func, texel) in [(1.1f32, CompareFunc::Lequal, 3), (-0.1, CompareFunc::Gequal, 0)] {
        let fixed = run_fs(true, d16(), func, reference)[texel];
        assert_eq!(fixed, 1.0, "D16, {func:?} {reference}: the clamped reference meets the texel");
        let float = run_fs(false, D32F, func, reference)[texel];
        assert_eq!(float, 0.0, "D32F, {func:?} {reference}: compared as it is");
    }
}

/// The clamp costs a binding without the bit nothing: the program of a
/// look-up keyed clear is the program of the plain ret32 key (its words
/// unchanged), and the bit set changes the program (the fmax / fmin pair;
/// still one look-up, `run_fs`).
#[test]
fn the_bit_clear_compiles_the_unclamped_program() {
    let words = |k: LodKey| with_program(VS_PASS, FS_SHADOW, LodKey::NONE, k, |p| p.fs.code.words.to_vec());
    let plain = words(LodKey { ret32: 1, ..LodKey::NONE });
    assert_eq!(words(key(false)), plain, "fixed_depth clear: the program as before");
    assert_ne!(words(key(true)), plain, "fixed_depth set: the clamp is in the program");
    /* a bit for another binding changes nothing at binding 0 */
    assert_eq!(words(LodKey { fixed_depth: 2, ..key(false) }), plain, "binding 1's bit");
}
