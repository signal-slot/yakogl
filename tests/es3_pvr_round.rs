// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `round` and `roundEven` on the PowerVR online back end (the 16
//! `shaders.builtin_functions.common.round` and `roundeven` `*_highp_*` cases
//! of dEQP-GLES3 that failed on the AM62P): the programs dEQP's shader
//! executor writes (float..vec4, lowp, mediump and highp, vertex and
//! fragment) compile through the PowerVR chain; the IR the chain receives
//! gives the half-way values on the CPU evaluator (`round` takes either
//! neighbour, `roundEven` the even one, and `roundEven(-0.5)` is +0.0 as
//! dEQP's reference has it); and the operation sequence the PowerVR emits
//! for each function is pinned.
//!
//! HOST-ONLY: the GPU's own arithmetic is for the board.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_round
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::eval::{f32x4, run, Inputs};
use yakogl::shaderc::online::ir::{IoDecl, IoKind, IoSlot};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/// The compiler keeps its output in one arena that every compile resets:
/// one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

const SHAPES: [(&str, &str, usize); 4] =
    [("float", "uint", 1), ("vec2", "uvec2", 2), ("vec3", "uvec3", 3), ("vec4", "uvec4", 4)];

/// `generateVertexShader`: input `in0`, output `out0`, flat varying out.
fn vertex_executor(prec: &str, ty: &str, source: &str) -> String {
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin {prec} {ty} a_in0;\n\
         flat out {prec} {ty} vtx_out_out0;\n\
         void main (void)\n{{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n\
         \t{prec} {ty} in0 = a_in0;\n\t{prec} {ty} out0;\n\t{source}\n\tvtx_out_out0 = out0;\n}}\n"
    )
}

/// `generateFragmentShader` with integer outputs.
fn fragment_executor(prec: &str, ty: &str, uty: &str, source: &str) -> String {
    format!(
        "#version 300 es\nflat in {prec} {ty} vtx_out_in0;\n\
         layout(location = 0) out highp {uty} o_out0;\n\
         void main (void)\n{{\n\t{prec} {ty} in0 = vtx_out_in0;\n\t{prec} {ty} out0;\n\t{source}\n\
         \x20   o_out0 = floatBitsToUint(out0);\n}}\n"
    )
}

/// The program's operation names, in order.
fn op_names(image: &[u8]) -> Vec<&'static str> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if let Some(op) = ops_for(r, ph).next() {
                out.push(op.name);
            }
        }
        off += g.total;
    }
    out
}

/// One shader through the PowerVR chain: its operation names.
fn pvr_ops(stage: Stage, src: &str) -> Vec<&'static str> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
        let rec = match stage {
            Stage::Vertex => compile_vertex(&l.module, &l.reflection, a),
            _ => compile_fragment(&l.module, &l.reflection, a),
        }
        .unwrap_or_else(|e| panic!("chain: {e:?}\n{src}"));
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        op_names(&image)
    })
    .unwrap()
}

fn slot(name: &str, decls: &[IoDecl<'_>]) -> IoSlot {
    let d = decls.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no {name} in {decls:?}"));
    match d.kind {
        IoKind::Attribute => IoSlot::Attrib(d.location),
        IoKind::Varying => IoSlot::Varying(d.location),
        IoKind::Colour => IoSlot::FragColor(d.location),
        k => panic!("{name} is a {k:?}"),
    }
}

/// `out0` bits of the executor for `in0`, on the IR evaluator.
fn eval(stage: Stage, src: &str, in0: [f32; 4]) -> [u32; 4] {
    let mut buf = vec![0u8; 1 << 21];
    let a = Arena::new(&mut buf);
    let l = lower::compile_to_ir(&a, stage, src).unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
    let m = &l.module;
    if stage == Stage::Vertex {
        let io = [(slot("a_in0", m.inputs), f32x4(in0)), (slot("a_position", m.inputs), f32x4([0.0, 0.0, 0.0, 1.0]))];
        let got = run(m, &Inputs { io: &io, uniforms: &[] }).expect("runs");
        got.output(slot("vtx_out_out0", m.outputs)).expect("stored")
    } else {
        let io = [(slot("vtx_out_in0", m.inputs), f32x4(in0))];
        let got = run(m, &Inputs { io: &io, uniforms: &[] }).expect("runs");
        got.output(slot("o_out0", m.outputs)).expect("stored")
    }
}

/// Every precision, shape and stage compiles through the PowerVR chain.
#[test]
fn programs_compile() {
    for func in ["round", "roundEven"] {
        let source = format!("out0 = {func}(in0);");
        for prec in ["lowp", "mediump", "highp"] {
            for (ty, uty, _) in SHAPES {
                let v = pvr_ops(Stage::Vertex, &vertex_executor(prec, ty, &source));
                assert!(!v.is_empty(), "{func} {prec} {ty} vertex: empty program");
                let f = pvr_ops(Stage::Fragment, &fragment_executor(prec, ty, uty, &source));
                assert!(!f.is_empty(), "{func} {prec} {ty} fragment: empty program");
            }
        }
    }
}

/// The half-way values on the evaluator, for the IR the back end receives.
#[test]
fn half_way_values() {
    // (input, round's allowed values, roundEven's value)
    let cases: [(f32, [f32; 2], f32); 8] = [
        (0.5, [0.0, 1.0], 0.0),
        (1.5, [1.0, 2.0], 2.0),
        (2.5, [2.0, 3.0], 2.0),
        (-0.5, [-1.0, -0.0], 0.0),
        (-1.5, [-2.0, -1.0], -2.0),
        (-2.5, [-3.0, -2.0], -2.0),
        (3.5, [3.0, 4.0], 4.0),
        (-3.5, [-4.0, -3.0], -4.0),
    ];
    for (ty, uty, n) in SHAPES {
        for stage in [Stage::Vertex, Stage::Fragment] {
            let exec = |func: &str| {
                let source = format!("out0 = {func}(in0);");
                if stage == Stage::Vertex {
                    vertex_executor("highp", ty, &source)
                } else {
                    fragment_executor("highp", ty, uty, &source)
                }
            };
            let (rs, es) = (exec("round"), exec("roundEven"));
            for (x, round_ok, even) in cases {
                let r = eval(stage, &rs, [x; 4]);
                let e = eval(stage, &es, [x; 4]);
                for lane in 0..n {
                    assert!(
                        round_ok.iter().any(|v| v.to_bits() == r[lane]),
                        "{stage:?} {ty} round({x}) = {:e}",
                        f32::from_bits(r[lane])
                    );
                    assert_eq!(
                        e[lane],
                        even.to_bits(),
                        "{stage:?} {ty} roundEven({x}) = {:e}",
                        f32::from_bits(e[lane])
                    );
                }
            }
        }
    }
}

/// The operation sequence of `out0 = f(in0)` (float, highp, fragment).
#[test]
fn emitted_sequence_is_pinned() {
    let seq = |func: &str| {
        pvr_ops(Stage::Fragment, &fragment_executor("highp", "float", "uint", &format!("out0 = {func}(in0);")))
    };
    assert_eq!(
        seq("round"),
        [
            "mbyp", "mbyp", "mbyp", "pck", "tst", "movc", "fadd", "fadd", "tst", "movc", "fadd", "mbyp", "mbyp", "tst",
            "movc", "fadd", "fmul", "mbyp", "tst", "movc", "fadd", "mbyp", "mbyp", "mbyp", "pck", "tst", "movc",
            "mbyp", "mbyp", "tst", "movc", "fadd", "mbyp", "mbyp", "mbyp", "mbyp", "pck", "mbyp"
        ]
    );
    assert_eq!(
        seq("roundEven"),
        [
            "mbyp", "mbyp", "mbyp", "pck", "tst", "movc", "fadd", "fadd", "tst", "movc", "fadd", "mbyp", "mbyp", "tst",
            "movc", "fadd", "fmul", "mbyp", "tst", "movc", "fadd", "mbyp", "mbyp", "mbyp", "pck", "tst", "movc",
            "mbyp", "mbyp", "tst", "movc", "mbyp", "mbyp", "pck", "tst", "movc", "fmul", "fadd", "fadd", "mbyp",
            "mbyp", "pck", "tst", "movc", "mbyp", "mbyp", "tst", "movc", "mbyp", "mbyp", "tst", "movc", "fadd", "mbyp",
            "mbyp", "mbyp", "mbyp", "pck", "mbyp"
        ]
    );
}
