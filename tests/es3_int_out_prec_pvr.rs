// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `shaders.precision.{int,uint}.*_fragment` as it is declared:
//! the scalar integer colour output and its two flat operands carry
//! `lowp`, `mediump` or `highp`, and the value is `v_a OP v_b` for add, sub,
//! mul and div. Guards the PowerVR `StoreOutput` fix (docs/notes/pvr-int-output-alu.md)
//! for every precision; the other tests of the family use `highp` only.
//!
//! What is checked is that each of the 24 shaders builds under the integer
//! render target variant (R32I / R32UI, `shc_blend_key_int_rt`), and the
//! integer that lands in pixel output 0 when the built USC bytes run through
//! the host byte model (tests/usc_exec `run_binary`). HOST-ONLY: no GPU
//! arithmetic, no GL read-back. The inputs keep every result in the lowp
//! range (7 and 3; for `int` also -7 and 3, division truncating).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_prec_pvr
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

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    num_varyings: u8,
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
        Ok(Rec {
            num_outputs: rec.num_outputs,
            num_varyings: rec.num_varyings,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
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
/// of the moves into the bank the last operation writes (the program ends
/// on its last pixel output write).
fn pixel_outputs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i)).collect()
}

/// The 32-bit one-channel integer variant: R32I for `int`, R32UI for `uint`.
fn r32_key(ty: &str) -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels: 1, bits: 32, signed: ty == "int" })
}

/// `precision <p> float; flat in <p> T v_a, v_b; out <p> T o_out = v_a OP v_b`.
fn shader(p: &str, ty: &str, op: &str) -> String {
    format!(
        "#version 300 es\nprecision {p} float;\nflat in {p} {ty} v_a;\nflat in {p} {ty} v_b;\n\
         layout(location = 0) out {p} {ty} o_out;\nvoid main() {{ o_out = v_a {op} v_b; }}\n"
    )
}

/// `src` under the R32 variant of `ty`, with `v_a` and `v_b` (the varyings
/// start at coefficient 4, after the W plane; a scalar takes one 4-wide
/// plane): the dword in pixel output 0.
fn run(src: &str, ty: &str, a: u32, b: u32) -> u32 {
    let rec = compile_for(src, r32_key(ty)).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs, 1, "{src}: one dword");
    assert_eq!(rec.num_varyings, 2, "{src}: two varyings");
    /* pixel output 0 is special register 32 (usc_exec: PIXOUT0) */
    assert_eq!(pixel_outputs(&rec.image), [32], "{src}: written to pixel output 0 once");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff[4..8].fill(a);
    inp.coeff[8..12].fill(b);
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    *out.get(&0).unwrap_or_else(|| panic!("{src}: nothing in pixel output 0: {out:?}"))
}

fn check_int(p: &str) {
    for op in ["+", "-", "*", "/"] {
        let src = shader(p, "int", op);
        for (a, b) in [(7i32, 3i32), (-7, 3)] {
            let want = match op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                _ => a / b,
            };
            assert_eq!(run(&src, "int", a as u32, b as u32) as i32, want, "{p} int {a} {op} {b}");
        }
    }
}

fn check_uint(p: &str) {
    for op in ["+", "-", "*", "/"] {
        let src = shader(p, "uint", op);
        let want = match op {
            "+" => 7 + 3,
            "-" => 7 - 3,
            "*" => 7 * 3,
            _ => 7 / 3,
        };
        assert_eq!(run(&src, "uint", 7, 3), want, "{p} uint 7 {op} 3");
    }
}

#[test]
fn lowp_int_ops() {
    check_int("lowp");
}

#[test]
fn mediump_int_ops() {
    check_int("mediump");
}

#[test]
fn highp_int_ops() {
    check_int("highp");
}

#[test]
fn lowp_uint_ops() {
    check_uint("lowp");
}

#[test]
fn mediump_uint_ops() {
    check_uint("mediump");
}

#[test]
fn highp_uint_ops() {
    check_uint("highp");
}
