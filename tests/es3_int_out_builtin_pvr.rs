// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Scalar `int` and `uint` colour outputs from the integer built-ins `abs`,
//! `sign`, `min`, `max` and `clamp`, on the PowerVR online back end: a
//! regression guard for the `StoreOutput` fix (docs/notes/pvr-int-output-alu.md).
//! Each case must build under the integer render target variant
//! (R32I / R32UI, `shc_blend_key_int_rt`), and the dword in pixel output 0,
//! run through the host byte model (tests/usc_exec `run_binary`), must be the
//! built-in's value. HOST-ONLY: no GPU arithmetic and no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_builtin_pvr
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

/// A fragment shader of one flat varying `v` of type `vty` and one integer
/// colour output `o_out` of type `ty` at location 0.
fn fragment(vty: &str, ty: &str, body: &str) -> String {
    format!(
        "#version 300 es\nflat in highp {vty} v;\nlayout(location = 0) out highp {ty} o_out;\n\
         void main() {{ {body} }}\n"
    )
}

/// `src` under the R32 variant of `ty`, run once with the one varying's
/// every coefficient `bits` (a flat varying is its value whichever plane it
/// is read from): the dword in pixel output 0.
fn run(src: &str, ty: &str, bits: u32) -> u32 {
    let rec = compile_for(src, r32_key(ty)).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs, 1, "{src}: one dword");
    assert_eq!(rec.num_varyings, 1, "{src}: the one varying");
    /* pixel output 0 is special register 32 (usc_exec: PIXOUT0) */
    assert_eq!(pixel_outputs(&rec.image), [32], "{src}: written to pixel output 0 once");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff.fill(bits);
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    *out.get(&0).unwrap_or_else(|| panic!("{src}: nothing in pixel output 0: {out:?}"))
}

/// `o_out = expr` with a flat varying `v` of type `ty` holding `v`: the dword
/// in pixel output 0 against `want`.
fn check(ty: &str, expr: &str, v: u32, want: u32) {
    let src = fragment(ty, ty, &format!("o_out = {expr};"));
    assert_eq!(run(&src, ty, v), want, "{expr} with v = {v:#x}");
}

#[test]
fn int_abs_sign() {
    for v in [-7i32, 0, 4] {
        check("int", "abs(v)", v as u32, v.abs() as u32);
        check("int", "sign(v)", v as u32, v.signum() as u32);
    }
}

#[test]
fn int_min_max() {
    for v in [-7i32, 0, 4] {
        check("int", "min(v, 3)", v as u32, v.min(3) as u32);
        check("int", "max(v, 3)", v as u32, v.max(3) as u32);
    }
}

#[test]
fn int_clamp() {
    for v in [-7i32, 0, 4] {
        check("int", "clamp(v, -2, 5)", v as u32, v.clamp(-2, 5) as u32);
    }
}

#[test]
fn uint_min_max_clamp() {
    for v in [0u32, 2, 9] {
        check("uint", "min(v, 3u)", v, v.min(3));
        check("uint", "max(v, 3u)", v, v.max(3));
        check("uint", "clamp(v, 1u, 5u)", v, v.clamp(1, 5));
    }
}
