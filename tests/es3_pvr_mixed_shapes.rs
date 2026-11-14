// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the online PowerVR compiler does today with fragment shaders of
//! several colour outputs of mixed types: the shapes of dEQP-GLES3's
//! `fragment_out` (`layout(location = N)` outputs of int/uint/float types)
//! and of the `builtin_functions.common.modf` executor (a highp float output
//! and an int output). Each source is compiled through `compile_fragment_for`
//! under key 0 and under the integer key of its first integer output
//! (`shc_blend_key_int_rt`), and the outcome is pinned literally: Ok with its
//! `num_outputs` and the pixel output registers the bytes write (through
//! tests/usc_exec `run_binary`), or the exact refusal. The test documents the
//! current behaviour and does not judge it: a change of the compiler that
//! moves a row moves it here on purpose.
//! HOST-ONLY: no GPU arithmetic.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_mixed_shapes
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Rec {
    num_outputs: u8,
    image: Vec<u8>,
}

/// One fragment shader through the chain under the variant `key`, or the
/// refusal.
fn compile_for(src: &str, key: u32) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(Rec { num_outputs: rec.num_outputs, image: code.iter().flat_map(|w| w.to_le_bytes()).collect() })
    })
    .unwrap()
}

/// The program's operations, in order: (name, the first destination's bank
/// and index).
fn ops(image: &[u8]) -> Vec<(&'static str, Option<(u32, u32)>)> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let Some(op) = ops_for(r, ph).next() else { continue };
            let d0 = g.dst.as_ref().and_then(|d| dsts(d)[0]).map(|o| (o.bank, o.index));
            out.push((op.name, d0));
        }
        off += g.total;
    }
    out
}

/// The pixel output registers a program writes, in order: the destinations
/// of the operations into the bank the last operation writes (the program
/// ends on its last pixel output write).
fn pixel_outputs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i)).collect()
}

fn key_int(channels: u8, bits: u8, signed: bool) -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels, bits, signed })
}

/// A fragment shader of one flat varying `v` with the output declarations
/// `decls` and the statements `body`.
fn shader(decls: &[&str], body: &str) -> String {
    let outs = decls.join("\n");
    format!("#version 300 es\nprecision highp float;\nprecision highp int;\nflat in highp vec4 v;\n{outs}\nvoid main() {{ {body} }}\n")
}

/// The outcome of `src` under `key`, as text: the refusal, or the
/// `num_outputs`, the pixel output registers the program writes (in order)
/// and the registers the byte model leaves a value in.
fn outcome(src: &str, key: u32) -> String {
    match compile_for(src, key) {
        Err(e) => format!("err {e}"),
        Ok(rec) => {
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            inp.coeff.fill(0x4000_0000);
            let ran = match usc_exec::run_binary(&rec.image, &inp, true) {
                Ok(m) => format!("{:?}", m.keys().collect::<Vec<_>>()),
                Err(e) => format!("model: {e}"),
            };
            format!("ok n={} pixout={:?} ran={ran}", rec.num_outputs, pixel_outputs(&rec.image))
        }
    }
}

struct Shape {
    name: &'static str,
    src: String,
    int_key: u32,
    key0: &'static str,
    keyint: &'static str,
}

fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "vec4 + uint",
            src: shader(
                &["layout(location = 0) out highp vec4 o0;", "layout(location = 1) out highp uint o1;"],
                "o0 = v; o1 = uint(v.x);",
            ),
            int_key: key_int(1, 32, false),
            key0: "ok n=2 pixout=[32, 33] ran=[0, 1]",
            keyint: "ok n=2 pixout=[32, 33] ran=[0, 1]",
        },
        Shape {
            name: "modf: float + int",
            src: shader(
                &["layout(location = 0) out highp float o0;", "layout(location = 1) out highp int o1;"],
                "o0 = v.x; o1 = floatBitsToInt(v.y);",
            ),
            int_key: key_int(1, 32, true),
            key0: "ok n=2 pixout=[32, 33] ran=[0, 1]",
            keyint: "ok n=2 pixout=[32, 33] ran=[0, 1]",
        },
        Shape {
            name: "modf: float as floatBitsToUint + int",
            src: shader(
                &["layout(location = 0) out highp uint o0;", "layout(location = 1) out highp int o1;"],
                "o0 = floatBitsToUint(v.x); o1 = floatBitsToInt(v.y);",
            ),
            int_key: key_int(1, 32, false),
            key0: "ok n=2 pixout=[32, 33] ran=[0, 1]",
            keyint: "ok n=2 pixout=[32, 33] ran=[0, 1]",
        },
        Shape {
            name: "vec4 + int + vec4",
            src: shader(
                &[
                    "layout(location = 0) out highp vec4 o0;",
                    "layout(location = 1) out highp int o1;",
                    "layout(location = 2) out highp vec4 o2;",
                ],
                "o0 = v; o1 = int(v.x); o2 = v.wzyx;",
            ),
            int_key: key_int(1, 32, true),
            key0: "ok n=3 pixout=[32, 33, 34] ran=[0, 1, 2]",
            keyint: "ok n=3 pixout=[32, 33, 34] ran=[0, 1, 2]",
        },
        Shape {
            name: "ivec4 + uvec2 + vec4",
            src: shader(
                &[
                    "layout(location = 0) out highp ivec4 o0;",
                    "layout(location = 1) out highp uvec2 o1;",
                    "layout(location = 2) out highp vec4 o2;",
                ],
                "o0 = ivec4(v); o1 = uvec2(v.xy); o2 = v;",
            ),
            int_key: key_int(4, 32, true),
            key0: "ok n=3 pixout=[32, 33, 34] ran=[0, 1, 2]",
            keyint: "err chain: Build(Unsupported(\"integer colour outputs past the pixel's four output registers\"))",
        },
        Shape {
            name: "int + uint (two integer outputs)",
            src: shader(
                &["layout(location = 0) out highp int o0;", "layout(location = 1) out highp uint o1;"],
                "o0 = int(v.x); o1 = uint(v.y);",
            ),
            int_key: key_int(1, 32, true),
            key0: "ok n=2 pixout=[32, 33] ran=[0, 1]",
            keyint: "ok n=2 pixout=[32, 33] ran=[0, 1]",
        },
        Shape {
            name: "uint at location 2 only",
            src: shader(&["layout(location = 2) out highp uint o2;"], "o2 = uint(v.x);"),
            int_key: key_int(1, 32, false),
            key0: "ok n=3 pixout=[34] ran=[2]",
            keyint:
                "err chain: Build(Unsupported(\"an integer render target written from other than colour outputs 0..\"))",
        },
    ]
}

#[test]
fn mixed_output_shapes_today() {
    let rows: Vec<_> = shapes().into_iter().map(|s| (outcome(&s.src, 0), outcome(&s.src, s.int_key), s)).collect();
    for (a, b, s) in &rows {
        println!("SHAPE {} | key0: {a} | keyint: {b}", s.name);
    }
    for (a, b, s) in &rows {
        assert_eq!(a, s.key0, "{} under key 0", s.name);
        assert_eq!(b, s.keyint, "{} under the integer key", s.name);
    }
}
