// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Integer colour outputs from conversions and uniform operands, on the
//! PowerVR online back end: `int(u)`, `uint(i)`, `int(v > 3)`, `uint(v > 3)`
//! and `v * 2 + u_k` with a uniform `u_k`. Regression guard for the
//! `StoreOutput` fix of docs/notes/pvr-int-output-alu.md; `int(f)` and
//! `uint(f)` are in es3_int_out_alu_pvr.rs.
//!
//! What is checked is that each program builds and the 32-bit integer in
//! pixel output 0 of the integer render target variant (R32I / R32UI,
//! `shc_blend_key_int_rt`), with the built USC bytes run through the host
//! byte model (tests/usc_exec `run_binary`). The model feeds a uniform
//! through the shared file; the uniform case sets every word of it to the
//! uniform's value, wherever the program placed it. HOST-ONLY: no GPU
//! arithmetic, and no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_conv_pvr
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

/// A fragment shader of one flat varying `v` of type `vty`, the declarations
/// `extra`, and one integer colour output `o_out` of type `ty` at location 0.
fn fragment(vty: &str, ty: &str, extra: &str, body: &str) -> String {
    format!(
        "#version 300 es\nflat in highp {vty} v;\n{extra}layout(location = 0) out highp {ty} o_out;\n\
         void main() {{ {body} }}\n"
    )
}

/// `src` under the R32 variant of `ty`, run once with the one varying's
/// every coefficient `bits` and, when `shared` is set, every word of the
/// shared file that value: the dword in pixel output 0.
fn run_with(src: &str, ty: &str, bits: u32, shared: Option<u32>) -> u32 {
    let rec = compile_for(src, r32_key(ty)).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs, 1, "{src}: one dword");
    assert_eq!(rec.num_varyings, 1, "{src}: the one varying");
    /* pixel output 0 is special register 32 (usc_exec: PIXOUT0) */
    assert_eq!(pixel_outputs(&rec.image), [32], "{src}: written to pixel output 0 once");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff.fill(bits);
    if let Some(k) = shared {
        inp.shared.fill(k);
    }
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    *out.get(&0).unwrap_or_else(|| panic!("{src}: nothing in pixel output 0: {out:?}"))
}

fn run(src: &str, ty: &str, bits: u32) -> u32 {
    run_with(src, ty, bits, None)
}

/// `o_out = int(u)` from a flat uint varying: the bits are kept.
#[test]
fn int_from_uint() {
    let src = fragment("uint", "int", "", "o_out = int(v);");
    for (x, want) in [(5u32, 5i32), (0xFFFF_FFFF, -1), (0, 0)] {
        assert_eq!(run(&src, "int", x) as i32, want, "int({x}u)");
    }
}

/// `o_out = uint(i)` from a flat int varying: the bits are kept.
#[test]
fn uint_from_int() {
    let src = fragment("int", "uint", "", "o_out = uint(v);");
    for (x, want) in [(5i32, 5u32), (-1, 0xFFFF_FFFF), (0, 0)] {
        assert_eq!(run(&src, "uint", x as u32), want, "uint({x})");
    }
}

/// `o_out = int(v > 3)` and `uint(v > 3)`: a bool is 0 or 1.
#[test]
fn bool_to_int_and_uint() {
    let int = fragment("int", "int", "", "o_out = int(v > 3);");
    let uint = fragment("int", "uint", "", "o_out = uint(v > 3);");
    for (x, want) in [(2i32, 0u32), (7, 1)] {
        assert_eq!(run(&int, "int", x as u32) as i32, want as i32, "int({x} > 3)");
        assert_eq!(run(&uint, "uint", x as u32), want, "uint({x} > 3)");
    }
}

/// `o_out = v * 2 + u_k` with a uniform `u_k`: the product and the uniform
/// meet in one sum.
#[test]
fn uniform_operand() {
    let src = fragment("int", "int", "uniform highp int u_k;\n", "o_out = v * 2 + u_k;");
    for (x, k) in [(5i32, 100i32), (-3, 7), (0, -1)] {
        assert_eq!(run_with(&src, "int", x as u32, Some(k as u32)) as i32, x * 2 + k, "{x} * 2 + {k}");
    }
}
