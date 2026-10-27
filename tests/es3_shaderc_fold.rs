// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 `round` and `roundEven` (section 8.3) give the exact
//! result: the dEQP-GLES3 `shaders.builtin_functions.common.round.*_highp_*`
//! and `roundeven.*_highp_*` cases (16), which failed on the AM62P with
//! "Result comparison failed" (docs/es3/am62p-gles3-notpass.tsv).
//!
//! The shaders are the ones dEQP's shader executor writes for these cases
//! (VK-GL-CTS `modules/glshared/glsShaderExecUtil.cpp`,
//! `generateVertexShader` and `generateFragmentShader` with integer
//! outputs, as ES 3.0 without float colour buffers gets). The inputs and
//! the expected values come from the case sources
//! (`modules/gles3/functional/es3fShaderCommonFunctionTests.cpp`,
//! `RoundCase` and `RoundEvenCase`): their special inputs, values near the
//! two places floor(x + 0.5) goes wrong, and seeded values over dEQP's
//! highp range of +-1e7. The expected value is dEQP's reference, ported
//! below with its comparison rule: highp wants the exact bits, the sign of
//! zero included, and `round` may take either neighbour at a .5.
//!
//! The values are those of the CPU evaluator (eval.rs) running the front
//! end's IR, the IR every back end compiles. Each program also compiles and
//! links on this build's back end wherever the same program with
//! `out0 = in0;` does (`builds_where_a_copy_builds`). HOST-ONLY: the GPUs'
//! own arithmetic is for the board.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_fold
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
use yakogl::shaderc::online::ir::{IoDecl, IoKind, IoSlot};
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that `forget_compiled` resets,
/// and the tests of this binary run on parallel threads. `build` holds this
/// lock across the reset, both compiles and the link, and reads every result
/// before it lets go: the serialiser `src/test_rules.rs` asks of a test
/// binary that resets the compiler and holds more than one test (the shape
/// of `tests/online_oracle.rs`).
static COMPILING: Mutex<()> = Mutex::new(());

// ---- dEQP's programs ------------------------------------------------------

/// `generateVertexShader(spec, "a_", "vtx_out_")`: one input `in0`, one
/// output `out0`, both `<prec> <ty>`.
fn vertex_executor(prec: &str, ty: &str, source: &str) -> String {
    format!(
        "#version 300 es\n\
         in highp vec4 a_position;\n\
         in {prec} {ty} a_in0;\n\
         flat out {prec} {ty} vtx_out_out0;\n\
         \n\
         void main (void)\n\
         {{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n\n\
         \t{prec} {ty} in0 = a_in0;\n\
         \t{prec} {ty} out0;\n\
         \t{source}\n\
         \tvtx_out_out0 = out0;\n\
         }}\n"
    )
}

/// `generatePassthroughFragmentShader` with integer outputs: the vertex
/// executor's fragment half.
fn passthrough_fragment(prec: &str, ty: &str, uty: &str) -> String {
    format!(
        "#version 300 es\n\
         flat in {prec} {ty} vtx_out_out0;\n\
         layout(location = 0) out highp {uty} o_out0;\n\
         \n\
         void main (void)\n\
         {{\n    o_out0 = floatBitsToUint(vtx_out_out0);\n}}\n"
    )
}

/// `generatePassthroughVertexShader(spec, "a_", "vtx_out_")`: the fragment
/// executor's vertex half.
fn passthrough_vertex(prec: &str, ty: &str) -> String {
    format!(
        "#version 300 es\n\
         in highp vec4 a_position;\n\
         in {prec} {ty} a_in0;\n\
         flat out {prec} {ty} vtx_out_in0;\n\
         \n\
         void main (void)\n\
         {{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n\
         \tvtx_out_in0 = a_in0;\n\
         }}\n"
    )
}

/// `generateFragmentShader(spec, true, .., "vtx_out_", "o_")`.
fn fragment_executor(prec: &str, ty: &str, uty: &str, source: &str) -> String {
    format!(
        "#version 300 es\n\
         flat in {prec} {ty} vtx_out_in0;\n\
         layout(location = 0) out highp {uty} o_out0;\n\
         \n\
         void main (void)\n\
         {{\n\
         \t{prec} {ty} in0 = vtx_out_in0;\n\
         \t{prec} {ty} out0;\n\
         \t{source}\n\
         \x20   o_out0 = floatBitsToUint(out0);\n\
         }}\n"
    )
}

// ---- running them ----------------------------------------------------------

fn slot(name: &str, decls: &[IoDecl<'_>]) -> IoSlot {
    let d = decls.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no {name} in {decls:?}"));
    match d.kind {
        IoKind::Attribute => IoSlot::Attrib(d.location),
        IoKind::Varying => IoSlot::Varying(d.location),
        IoKind::Colour => IoSlot::FragColor(d.location),
        k => panic!("{name} is a {k:?}"),
    }
}

/// `out0` of the vertex executor for `in0`, through the IR evaluator.
fn eval_vertex(src: &str, in0: [f32; 4]) -> [u32; 4] {
    let mut buf = vec![0u8; 1 << 21];
    let a = Arena::new(&mut buf);
    let l = compile_to_ir(&a, Stage::Vertex, src).unwrap_or_else(|e| panic!("refused: {}: {}\n{src}", e.tok, e.msg));
    let m = &l.module;
    let io = [(slot("a_in0", m.inputs), f32x4(in0)), (slot("a_position", m.inputs), f32x4([0.0, 0.0, 0.0, 1.0]))];
    let got = run(m, &Inputs { io: &io, uniforms: &[] }).unwrap_or_else(|e| panic!("does not run: {e}"));
    got.output(slot("vtx_out_out0", m.outputs)).expect("vtx_out_out0 stored")
}

/// `o_out0` (the bits of `out0`) of the fragment executor for `in0`.
fn eval_fragment(src: &str, in0: [f32; 4]) -> [u32; 4] {
    let mut buf = vec![0u8; 1 << 21];
    let a = Arena::new(&mut buf);
    let l = compile_to_ir(&a, Stage::Fragment, src).unwrap_or_else(|e| panic!("refused: {}: {}\n{src}", e.tok, e.msg));
    let m = &l.module;
    let io = [(slot("vtx_out_in0", m.inputs), f32x4(in0))];
    let got = run(m, &Inputs { io: &io, uniforms: &[] }).unwrap_or_else(|e| panic!("does not run: {e}"));
    got.output(slot("o_out0", m.outputs)).expect("o_out0 stored")
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// glCompileShader on both stages and glLinkProgram, on this build's back
/// end, as dEQP's executor does.
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
            return Err(format!("compile: {}\n{src}", text_of(&log)));
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

/// Both executor programs of `func` (vertex `vs`, fragment `fs`) build on
/// this back end wherever the same program with `out0 = in0;` builds: the
/// function adds no refusal. The copy is the baseline because the V3D
/// back end refuses the vec2 and vec3 executor programs whatever their
/// body (`Program(Prepare(Undefined(..)))` at link; the
/// "Shader compilation failed (glsBuiltinPrecisionTests)" row of
/// ab43a20c5:docs/es3-host-run.md), which is not this function's to fix.
fn builds_where_a_copy_builds(func: &str, ty: &str, uty: &str, vs: &str, fs: &str) {
    let copy = "out0 = in0;";
    let (pvs, pfs) = (passthrough_vertex("highp", ty), passthrough_fragment("highp", ty, uty));
    let (cvs, cfs) = (vertex_executor("highp", ty, copy), fragment_executor("highp", ty, uty, copy));
    let both: [(&str, [&str; 2], [&str; 2]); 2] =
        [("vertex", [vs, &pfs], [&cvs, &pfs]), ("fragment", [&pvs, fs], [&pvs, &cfs])];
    for (stage, [v, f], [cv, cf]) in both {
        if let Err(e) = build(v, f) {
            let copy_too = build(cv, cf).err();
            assert!(copy_too.is_some(), "{func} {ty} {stage}: refused where a copy builds: {e}");
            println!("{func} {ty} {stage}: not built here, nor is a copy: {}", copy_too.unwrap_or_default());
        }
    }
}

// ---- dEQP's reference --------------------------------------------------------

/// es3fShaderCommonFunctionTests.cpp `roundEven`, as written there:
/// `deFloatFrac(v)` is v - floor(v), and the result is float(int), so a
/// zero result is +0.0.
fn deqp_round_even(v: f32) -> f32 {
    let q = v - v.floor();
    let truncated = (v - q) as i32;
    let rounded = if q > 0.5 || (q == 0.5 && truncated % 2 != 0) { truncated + 1 } else { truncated };
    rounded as f32
}

/// `RoundEvenCase::compare` at highp: the exact bits.
fn deqp_round_even_ok(x: f32, got: u32) -> bool {
    got == deqp_round_even(x).to_bits()
}

/// `RoundCase::compare` at highp: at a fraction of exactly 0.5, floor(x)
/// or ceil(x); elsewhere `roundEven`'s exact bits.
fn deqp_round_ok(x: f32, got: u32) -> bool {
    if x - x.floor() == 0.5 {
        got == x.floor().to_bits() || got == x.ceil().to_bits()
    } else {
        got == deqp_round_even(x).to_bits()
    }
}

// ---- the inputs --------------------------------------------------------------

/// dEQP's special inputs at highp: `float(ndx) - 5.5` for ndx 0..10
/// (`RoundCase`), `float(ndx) - 10.5` for ndx 0..20 (`RoundEvenCase`).
fn special(n: i32, off: f32) -> Vec<f32> {
    (0..n).map(|i| i as f32 - off).collect()
}

/// Where floor(x + 0.5) was wrong: the sum rounds before the floor sees it
/// (0.5 - 2^-25 + 0.5 is 1.0; past 2^23 every odd integer + 0.5 rounds to
/// the even one above), and the halfway points just below 2^23.
fn edges() -> Vec<f32> {
    vec![
        0.49999997,
        -0.49999997,
        1.4999999,
        -1.4999999,
        2.4999998,
        8388609.0,
        -8388609.0,
        8388611.0,
        9999999.0,
        -9999999.0,
        4194304.5,
        4194305.5,
        -4194304.5,
        -4194305.5,
        8388607.5,
        -8388607.5,
        -0.3,
        0.3,
        -0.0,
        0.0,
    ]
}

/// Seeded values over dEQP's highp range, [-1e7, 1e7] (xorshift32; dEQP
/// draws its own with de::Random, which this does not reproduce).
fn seeded(n: usize) -> Vec<f32> {
    let mut s = 0x2545_f491u32;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f64 / u32::MAX as f64 * 2.0e7 - 1.0e7) as f32
        })
        .collect()
}

fn inputs(special_inputs: Vec<f32>) -> Vec<f32> {
    let mut v = special_inputs;
    v.extend(edges());
    v.extend(seeded(4000));
    v
}

// ---- the cases ------------------------------------------------------------------

const SHAPES: [(&str, &str, usize); 4] =
    [("float", "uint", 1), ("vec2", "uvec2", 2), ("vec3", "uvec3", 3), ("vec4", "uvec4", 4)];

/// Every lane of every input, in vec4-sized groups (lanes past the type's
/// width are not read).
fn check(func: &str, values: &[f32], ok: fn(f32, u32) -> bool) {
    for (ty, uty, n) in SHAPES {
        let source = format!("out0 = {func}(in0);");
        let vs = vertex_executor("highp", ty, &source);
        let fs = fragment_executor("highp", ty, uty, &source);
        builds_where_a_copy_builds(func, ty, uty, &vs, &fs);
        let mut bad = Vec::new();
        for chunk in values.chunks(n) {
            let mut in0 = [0.0f32; 4];
            in0[..chunk.len()].copy_from_slice(chunk);
            for (stage, got) in [("vertex", eval_vertex(&vs, in0)), ("fragment", eval_fragment(&fs, in0))] {
                for (lane, &x) in chunk.iter().enumerate() {
                    if !ok(x, got[lane]) {
                        bad.push(format!(
                            "{stage} {func}({x:e}) = {:e} ({:#010x}), dEQP's roundEven reference {:e}",
                            f32::from_bits(got[lane]),
                            got[lane],
                            deqp_round_even(x)
                        ));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{func} {ty}: {} wrong, first: {:#?}", bad.len(), &bad[..bad.len().min(8)]);
    }
}

/// dEQP-GLES3.functional.shaders.builtin_functions.common.round.
/// {float,vec2,vec3,vec4}_highp_{vertex,fragment}.
///
/// Revert check: put back floor(x + 0.5) and round(0.49999997) is 1.0 and
/// round(8388609.0) is 8388610.0.
///
/// -0.49999997 is left out here: dEQP's frac of it, -0.49999997 + 1.0,
/// rounds to 0.5, so its rule asks for floor or ceil (-1.0 or -0.0) where
/// the answer is zero and every other input of (-0.5, 0) must give +0.0.
/// dEQP draws its inputs over +-1e7, so it does not meet that input.
#[test]
fn round_highp_is_exact() {
    let values: Vec<f32> = inputs(special(10, 5.5)).into_iter().filter(|&x| x != -0.49999997).collect();
    check("round", &values, deqp_round_ok);
}

/// dEQP-GLES3.functional.shaders.builtin_functions.common.roundeven.
/// {float,vec2,vec3,vec4}_highp_{vertex,fragment}.
///
/// Revert check: as `round_highp_is_exact`, and drop the final `+ 0.0`
/// and roundEven(-0.5) is -0.0, where dEQP wants +0.0.
#[test]
fn round_even_highp_is_exact() {
    check("roundEven", &inputs(special(20, 10.5)), deqp_round_even_ok);
}

/// The values dEQP's reference gives for a few inputs, written out, so the
/// port of the reference above is pinned too (es3fShaderCommonFunctionTests
/// `roundEven`, and `RoundCase`'s two allowed values at a .5).
#[test]
fn the_reference_values() {
    let re = |x: f32| deqp_round_even(x).to_bits();
    assert_eq!(re(-0.5), 0.0f32.to_bits(), "+0.0, not -0.0");
    assert_eq!(re(-0.3), 0.0f32.to_bits());
    assert_eq!(re(0.5), 0.0f32.to_bits());
    assert_eq!(re(1.5), 2.0f32.to_bits());
    assert_eq!(re(2.5), 2.0f32.to_bits());
    assert_eq!(re(-2.5), (-2.0f32).to_bits());
    assert_eq!(re(-3.5), (-4.0f32).to_bits());
    assert_eq!(re(0.49999997), 0.0f32.to_bits());
    assert_eq!(re(8388609.0), 8388609.0f32.to_bits());
    assert!(deqp_round_ok(-0.5, (-0.0f32).to_bits()) && deqp_round_ok(-0.5, (-1.0f32).to_bits()));
    assert!(!deqp_round_ok(0.49999997, 1.0f32.to_bits()));
    assert!(!deqp_round_ok(-0.3, (-0.0f32).to_bits()), "highp compares the sign of zero");
}

/// The mediump and lowp cases passed on the AM62P and must keep passing:
/// the same programs at those precisions still build and give the exact
/// value on dEQP's special inputs (mediump's inputs are exact in f16).
/// Below highp dEQP ignores the sign of a zero (`supportsSignedZero`).
#[test]
fn mediump_and_lowp_still_build_and_round() {
    let unsigned_zero = |bits: u32| if bits == (-0.0f32).to_bits() { 0 } else { bits };
    for prec in ["mediump", "lowp"] {
        for (func, ok) in [("round", deqp_round_ok as fn(f32, u32) -> bool), ("roundEven", deqp_round_even_ok)] {
            let source = format!("out0 = {func}(in0);");
            let vs = vertex_executor(prec, "float", &source);
            build(&vs, &passthrough_fragment(prec, "float", "uint")).unwrap_or_else(|e| panic!("{prec} {func}: {e}"));
            for x in special(20, 10.5) {
                let got = eval_vertex(&vs, [x; 4])[0];
                assert!(
                    ok(x, got) || ok(x, unsigned_zero(got)) || (got == 0 && ok(x, (-0.0f32).to_bits())),
                    "{prec} {func}({x}) = {}",
                    f32::from_bits(got)
                );
            }
        }
    }
}
