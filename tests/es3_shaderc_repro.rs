// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The dEQP-GLES3 `shaders.*` failures of the AM62P per-case run
//! (docs/es3/am62p-gles3-notpass.tsv) that are not a GL API verdict, each as
//! the smallest program that shows it, put through the front end and this
//! build's back end (docs/es3/shader-host-repro.md).
//!
//! What is pinned: the front end (parser, sema, lower, fold) takes every one
//! of these programs and gives the IR the case needs -- a `gl_FragDepth`
//! output, an integer colour output, two colour outputs, a `textureSize`.
//! Where this build's back end then refuses, the refusal is the code
//! generator's and says so, never a front-end parse or type error. The
//! CPU evaluator (eval.rs) runs the IR for the arithmetic cases
//! (`modf`), against dEQP's own reference: a wrong board pixel with a right
//! IR value is the back end's. HOST-ONLY: the GPUs' pixels are for the board.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_repro
//! ```
//! (and with gpu-vivante, gpu-v3d). GLSL ES 3.00 is refused by a
//! `report-es2` build (pp.rs out_of_scope), so the file is empty there.
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::eval::{f32x4, run, Inputs};
use yakogl::shaderc::online::ir::{self, IoDecl, IoKind, IoSlot, Op};
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that `forget_compiled` resets,
/// and the tests of this binary run on parallel threads (`src/test_rules.rs`).
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// glCompileShader on both stages and glLinkProgram, on this build's back end.
fn build(vs: &str, fs: &str) -> Result<(), String> {
    let _serial = COMPILING.lock().unwrap_or_else(|p| p.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    let mut shaders = [ShcShader::ZERO, ShcShader::ZERO];
    for (i, (stage, src)) in [(SHC_STAGE_VERTEX, vs), (SHC_STAGE_FRAGMENT, fs)].into_iter().enumerate() {
        let mut log = [0 as c_char; 1024];
        let text = format!("{src}\0");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc =
            unsafe { compile(stage, text.as_ptr() as *const c_char, &mut shaders[i], log.as_mut_ptr(), log.len()) };
        if rc != 0 {
            return Err(format!("compile: {}", text_of(&log)));
        }
    }
    let Some(link) = yakogl::shaderc::online::SHC_ONLINE.link else {
        return Ok(());
    };
    let [vso, mut fso] = shaders;
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(format!("link: {}", text_of(&log)));
    }
    Ok(())
}

/// The front end alone: the IR of one stage, or its error.
fn lower(stage: Stage, src: &str) -> Result<Vec<IoDecl<'static>>, String> {
    let mut buf = vec![0u8; 1 << 21];
    let a = Arena::new(&mut buf);
    match compile_to_ir(&a, stage, src) {
        Ok(l) => Ok(l.module.outputs.iter().map(|d| IoDecl { name: "", ..*d }).collect()),
        Err(e) => Err(format!("{}: {}", e.tok, e.msg)),
    }
}

/// The front end takes the program; the back end's refusal, if there is one,
/// is the code generator's and named as such.
fn front_end_accepts(name: &str, vs: &str, fs: &str) {
    lower(Stage::Vertex, vs).unwrap_or_else(|e| panic!("{name}: the front end refused the vertex shader: {e}\n{vs}"));
    lower(Stage::Fragment, fs)
        .unwrap_or_else(|e| panic!("{name}: the front end refused the fragment shader: {e}\n{fs}"));
    match build(vs, fs) {
        Ok(()) => println!("{name}: builds here"),
        Err(e) => {
            assert!(
                e.contains("the code generator refused") || e.contains("link:"),
                "{name}: refused by the front end after lowering: {e}"
            );
            println!("{name}: {}", e.lines().next().unwrap_or(""));
        }
    }
}

// ---- shaders.fragdepth (es3fFragDepthTests.cpp) -----------------------------

const VS_COORD: &str = "#version 300 es\n\
in highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

fn fragdepth(body: &str) -> String {
    format!(
        "#version 300 es\nuniform highp vec4 u_color;\nuniform highp float uf_half;\nuniform bool ub_true;\n\
         uniform int ui_two;\nuniform highp float uf_fourth;\nin highp vec2 v_coord;\n\
         layout(location = 0) out mediump vec4 o_color;\n{body}\n"
    )
}

/// The eight `gl_FragDepth` writers of `fragdepth.write.*` and
/// `fragdepth.compare.*` (15 AM62P "Compile failed"): each lowers to a
/// colour output and a `Depth` output. Refused on every back end today
/// (PowerVR `Usc(FragmentOutputsNotBuilt)`, GC7000UL "this output is not
/// selected yet", V3D `NotYet("gl_FragDepth")`).
#[test]
fn fragdepth_writes_lower_to_a_depth_output() {
    let bodies = [
        ("const", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.5;\n}"),
        ("uniform", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = uf_half;\n}"),
        ("dynamic", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}"),
        ("fragcoord_z", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = gl_FragCoord.z;\n}"),
        (
            "uniform_conditional_write",
            "void main (void)\n{\n    o_color = u_color;\n    if (ub_true)\n        gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}",
        ),
        (
            "dynamic_conditional_write",
            "void main (void)\n{\n    o_color = u_color;\n    mediump float d = (v_coord.x+v_coord.y)*0.5f;\n    if (v_coord.y < 0.5)\n        gl_FragDepth = d;\n    else\n        gl_FragDepth = 1.0 - d;\n}",
        ),
        (
            "uniform_loop_write",
            "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.0;\n    for (int i = 0; i < ui_two; i++)\n        gl_FragDepth += uf_fourth;\n}",
        ),
        (
            "write_in_function",
            "void myfunc (highp vec2 coord)\n{\n    gl_FragDepth = (coord.x+coord.y)*0.5;\n}\nvoid main (void)\n{\n    o_color = u_color;\n    myfunc(v_coord);\n}",
        ),
    ];
    for (name, body) in bodies {
        let fs = fragdepth(body);
        let outs = lower(Stage::Fragment, &fs).unwrap_or_else(|e| panic!("fragdepth.{name}: {e}\n{fs}"));
        assert!(
            outs.iter().any(|o| o.kind == IoKind::Depth && o.slot() == IoSlot::FragDepth),
            "fragdepth.{name}: no depth output in {outs:?}"
        );
        assert!(outs.iter().any(|o| o.kind == IoKind::Colour), "fragdepth.{name}: no colour output in {outs:?}");
        front_end_accepts(&format!("fragdepth.{name}"), VS_COORD, &fs);
    }
    // `no_write` is the control: it passed on the board and builds everywhere.
    build(VS_COORD, &fragdepth("void main (void)\n{\n    o_color = u_color;\n}")).expect("fragdepth.no_write builds");
}

// ---- shaders.precision.{int,uint} (es3fShaderPrecisionTests.cpp) -----------

/// `createIntUintPrecisionEvalProgram`: two flat integer inputs, one
/// integer output of the case's precision.
fn precision_program(ty: &str, prec: &str, op: &str, vertex: bool) -> (String, String) {
    let mut vs = format!("#version 300 es\nin highp vec4 a_position;\nin {prec} {ty} a_in0;\nin {prec} {ty} a_in1;\n");
    let mut fs = format!("#version 300 es\nlayout(location = 0) out {prec} {ty} o_out;\n");
    if vertex {
        vs += &format!("flat out {prec} {ty} v_out;\n");
        fs += &format!("flat in {prec} {ty} v_out;\n");
    } else {
        vs += &format!("flat out {prec} {ty} v_in0;\nflat out {prec} {ty} v_in1;\n");
        fs += &format!("flat in {prec} {ty} v_in0;\nflat in {prec} {ty} v_in1;\n");
    }
    vs += "\nvoid main (void)\n{\n    gl_Position = a_position;\n";
    fs += "\nvoid main (void)\n{\n";
    let (p, o) = if vertex { ("a_", "v_") } else { ("v_", "o_") };
    let body = format!("\t{prec} {ty} in0 = {p}in0;\n\t{prec} {ty} in1 = {p}in1;\n\t{o}out = {op};\n");
    if vertex {
        vs += &body;
        fs += "    o_out = v_out;\n";
    } else {
        vs += "    v_in0 = a_in0;\n    v_in1 = a_in1;\n";
        fs += &body;
    }
    (vs + "}\n", fs + "}\n")
}

/// The 48 `precision.{int,uint}.{lowp,mediump,highp}_{add,sub,mul,div}_
/// {vertex,fragment}` programs (24 fragment ones "m_program->isOk()" on the
/// AM62P). Every one lowers, with one integer colour output. The PowerVR
/// refuses the fragment ones ("a register read that nothing writes", a
/// scalar integer output computed from an ALU result); the GC7000UL and the
/// V3D build all 48.
#[test]
fn precision_int_programs_lower() {
    for ty in ["int", "uint"] {
        for prec in ["lowp", "mediump", "highp"] {
            for (name, op) in [("add", "in0 + in1"), ("sub", "in0 - in1"), ("mul", "in0 * in1"), ("div", "in0 / in1")] {
                for vertex in [true, false] {
                    let (vs, fs) = precision_program(ty, prec, op, vertex);
                    let case = format!("precision.{ty}.{prec}_{name}_{}", if vertex { "vertex" } else { "fragment" });
                    let outs = lower(Stage::Fragment, &fs).unwrap_or_else(|e| panic!("{case}: {e}\n{fs}"));
                    assert!(
                        matches!(outs.as_slice(), [IoDecl { kind: IoKind::Colour, location: 0, .. }]),
                        "{case}: {outs:?}"
                    );
                    front_end_accepts(&case, &vs, &fs);
                }
            }
        }
    }
}

/// The smallest fragment shader the PowerVR refuses of that family: an
/// integer output from an ALU result. A copy of the input, a constant and a
/// uniform build. On the GC7000UL and the V3D all four build.
#[test]
fn int_output_from_arithmetic_is_the_powervr_shape() {
    let vs = "#version 300 es\nin highp vec4 a_position;\nin highp int a_in0;\nflat out highp int v_in0;\n\
              void main() { gl_Position = a_position; v_in0 = a_in0; }\n";
    let fs = |body: &str| {
        format!("#version 300 es\nflat in highp int v_in0;\nuniform highp int u;\nlayout(location = 0) out highp int o_out;\nvoid main() {{ {body} }}\n")
    };
    for body in ["o_out = v_in0;", "o_out = 3;", "o_out = u;"] {
        build(vs, &fs(body)).unwrap_or_else(|e| panic!("{body}: {e}"));
    }
    front_end_accepts("int output from an add", vs, &fs("o_out = v_in0 + 1;"));
}

// ---- shaders.builtin_functions.common.modf (glsShaderExecUtil) ------------

/// `generateFragmentShader` with integer outputs, two outputs (`modf`'s
/// value and its whole part), at locations 0 and 1.
fn modf_fragment(prec: &str, ty: &str, uty: &str) -> String {
    format!(
        "#version 300 es\n\
         flat in {prec} {ty} vtx_out_in0;\n\
         layout(location = 0) out highp {uty} o_out0;\n\
         layout(location = 1) out highp {uty} o_out1;\n\
         \n\
         void main (void)\n\
         {{\n\
         \t{prec} {ty} in0 = vtx_out_in0;\n\
         \t{prec} {ty} out0;\n\
         \t{prec} {ty} out1;\n\
         \tout0 = modf(in0, out1);\n\
         \x20   o_out0 = floatBitsToUint(out0);\n\
         \x20   o_out1 = floatBitsToUint(out1);\n\
         }}\n"
    )
}

fn slot(name: &str, decls: &[IoDecl<'_>]) -> IoSlot {
    decls.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no {name} in {decls:?}")).slot()
}

/// `ModfCase::compare`: out0 + out1 is in0 to the precision's ULP bound
/// (exact at highp, where the sign of zero counts too), and out1 is
/// `float(int(in0))`.
fn deqp_modf_ok(in0: f32, out0: u32, out1: u32) -> bool {
    let (o0, o1) = (f32::from_bits(out0), f32::from_bits(out1));
    o1 == (in0 as i32) as f32 && (o0 + o1).to_bits() == in0.to_bits()
}

/// The 24 `modf.{float,vec2,vec3,vec4}_{lowp,mediump,highp}_{vertex,fragment}`
/// cases ("Result comparison failed" on the AM62P): the IR's value for
/// dEQP's inputs is exact, so the wrong pixel is the back end's, and the
/// program is the only one of `common` with two colour outputs.
///
/// Revert check: make `modf` in lower.rs return `x` (drop the Fadd) and
/// `modf(2.5)` is 2.5 where 0.5 is wanted.
#[test]
fn modf_is_exact_in_the_ir() {
    let mut s = 0x2545_f491u32;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s as f64 / u32::MAX as f64 * 2.0e7 - 1.0e7) as f32
    };
    // No -0.0: dEQP's own reference gives (-0.0) + (+0.0) = +0.0 for it,
    // which its highp rule (the exact bits) would refuse too; dEQP draws
    // its inputs over +-1e7 and never meets it.
    let mut values: Vec<f32> = vec![0.0, 0.5, -0.5, 1.5, -2.25, 5.5, 8388609.0, -9999999.0, 1e7, -1e7];
    values.extend((0..2000).map(|_| next()));
    for (ty, uty, n) in [("float", "uint", 1usize), ("vec2", "uvec2", 2), ("vec3", "uvec3", 3), ("vec4", "uvec4", 4)] {
        let fs = modf_fragment("highp", ty, uty);
        let mut buf = vec![0u8; 1 << 21];
        let a = Arena::new(&mut buf);
        let l =
            compile_to_ir(&a, Stage::Fragment, &fs).unwrap_or_else(|e| panic!("refused: {}: {}\n{fs}", e.tok, e.msg));
        let m = &l.module;
        assert_eq!(m.outputs.iter().filter(|o| o.kind == IoKind::Colour).count(), 2, "{:?}", m.outputs);
        let (s0, s1) = (slot("o_out0", m.outputs), slot("o_out1", m.outputs));
        let mut bad = Vec::new();
        for chunk in values.chunks(n) {
            let mut in0 = [0.0f32; 4];
            in0[..chunk.len()].copy_from_slice(chunk);
            let io = [(slot("vtx_out_in0", m.inputs), f32x4(in0))];
            let got = run(m, &Inputs { io: &io, uniforms: &[] }).unwrap_or_else(|e| panic!("does not run: {e}"));
            let (o0, o1) = (got.output(s0).expect("o_out0 stored"), got.output(s1).expect("o_out1 stored"));
            for (lane, &x) in chunk.iter().enumerate() {
                if !deqp_modf_ok(x, o0[lane], o1[lane]) {
                    bad.push(format!("modf({x:e}) = {:e} + {:e}", f32::from_bits(o0[lane]), f32::from_bits(o1[lane])));
                }
            }
        }
        assert!(bad.is_empty(), "modf {ty}: {} wrong, first: {:#?}", bad.len(), &bad[..bad.len().min(8)]);
        let vs = format!(
            "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\nflat out highp {ty} vtx_out_in0;\n\
             void main (void)\n{{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n\tvtx_out_in0 = a_in0;\n}}\n"
        );
        front_end_accepts(&format!("modf.{ty}_highp_fragment"), &vs, &fs);
    }
}

// ---- shaders.texture_functions.texturesize ---------------------------------

/// `TextureSizeCase::genFragmentShader` as dEQP writes it: the size at
/// `u_lod` against `u_texSize`.
fn texture_size_fragment(sampler: &str, three: bool) -> String {
    let (ivec, cast) = if three { ("ivec3", "ivec3") } else { ("ivec2", "ivec2") };
    format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out mediump vec4 o_color;\n\
         uniform highp {sampler} u_sampler;\nuniform highp {ivec} u_texSize;\nuniform highp int u_lod;\n\
         void main()\n{{\n\tif (textureSize(u_sampler, u_lod) == {cast}(u_texSize))\n\t\to_color = vec4(1.0);\n\telse\n\t\to_color = vec4(0.0);\n}}\n"
    )
}

/// The 38 `texturesize.*` cases ("Got unexpected texture size" on the
/// AM62P, a wrong pixel): each sampler type lowers to one `TexSize` of its
/// dimension. The value is the back end's (the PowerVR reads the texture
/// words; the V3D refuses 3D and array sizes).
#[test]
fn texture_size_lowers_for_every_sampler() {
    let vs = "#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    let samplers: [(&str, ir::TexDim, bool); 10] = [
        ("sampler2D", ir::TexDim::D2, false),
        ("isampler2D", ir::TexDim::D2, false),
        ("usampler2D", ir::TexDim::D2, false),
        ("sampler2DShadow", ir::TexDim::D2, false),
        ("samplerCube", ir::TexDim::Cube, false),
        ("usamplerCube", ir::TexDim::Cube, false),
        ("samplerCubeShadow", ir::TexDim::Cube, false),
        ("sampler3D", ir::TexDim::D3, true),
        ("sampler2DArray", ir::TexDim::D2Array, true),
        ("isampler2DArrayShadow", ir::TexDim::D2Array, true),
    ];
    for (sampler, dim, three) in samplers {
        let sampler = sampler.replace("isampler2DArrayShadow", "sampler2DArrayShadow");
        let fs = texture_size_fragment(&sampler, three);
        let mut buf = vec![0u8; 1 << 21];
        let a = Arena::new(&mut buf);
        let l =
            compile_to_ir(&a, Stage::Fragment, &fs).unwrap_or_else(|e| panic!("{sampler}: {}: {}\n{fs}", e.tok, e.msg));
        let sizes: Vec<_> = l.module.instrs.iter().filter(|o| matches!(o, Op::TexSize { .. })).collect();
        assert!(matches!(sizes.as_slice(), [Op::TexSize { dim: d, .. }] if *d == dim), "{sampler}: {sizes:?}");
        front_end_accepts(&format!("texturesize.{sampler}"), vs, &fs);
    }
}
