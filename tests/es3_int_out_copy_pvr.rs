// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A scalar `int` or `uint` colour output that is not an ALU result, on the
//! PowerVR online back end: a copy of a flat input, a constant and a uniform.
//! These built before the ALU case did (tests/es3_int_out_alu_pvr.rs); this
//! file pins the integer that lands in pixel output 0 of the R32I / R32UI
//! variant (`shc_blend_key_int_rt`), with the built USC bytes run through the
//! host byte model (tests/usc_exec `run_binary`). HOST-ONLY: no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_copy_pvr
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

/// A fragment shader of one flat varying `v` and one uniform `u`, both of
/// type `ty`, and one integer colour output `o_out` of type `ty` at
/// location 0. Unused ones are dropped by the compiler.
fn fragment(ty: &str, body: &str) -> String {
    format!(
        "#version 300 es\nflat in highp {ty} v;\nuniform highp {ty} u;\n\
         layout(location = 0) out highp {ty} o_out;\nvoid main() {{ {body} }}\n"
    )
}

/// `src` under the R32 variant of `ty`, run once with every iterated
/// coefficient (a flat varying is its value whichever plane it is read from)
/// and every uniform dword set to `bits`: the dword in pixel output 0.
/// `varyings` is the count the program must declare.
fn run(src: &str, ty: &str, varyings: u8, bits: u32) -> u32 {
    let rec = compile_for(src, r32_key(ty)).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs, 1, "{src}: one dword");
    assert_eq!(rec.num_varyings, varyings, "{src}: varyings");
    /* pixel output 0 is special register 32 (usc_exec: PIXOUT0) */
    assert_eq!(pixel_outputs(&rec.image), [32], "{src}: written to pixel output 0 once");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff.fill(bits);
    inp.shared.fill(bits);
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    *out.get(&0).unwrap_or_else(|| panic!("{src}: nothing in pixel output 0: {out:?}"))
}

/// `o_out = v;` copies the flat input, signed and unsigned.
#[test]
fn input_copy_lands_in_the_output() {
    let int = fragment("int", "o_out = v;");
    let uint = fragment("uint", "o_out = v;");
    for x in [-7i32, 0, 1, i32::MIN, i32::MAX] {
        assert_eq!(run(&int, "int", 1, x as u32) as i32, x, "int {x}");
    }
    for x in [0u32, 1, 4_000_000_000, u32::MAX] {
        assert_eq!(run(&uint, "uint", 1, x), x, "uint {x}");
    }
}

/// A constant is the same dword whatever the inputs hold.
#[test]
fn constant_lands_in_the_output() {
    let int = fragment("int", "o_out = -7;");
    let uint = fragment("uint", "o_out = 4000000000u;");
    for junk in [0u32, 0x1234_5678, u32::MAX] {
        assert_eq!(run(&int, "int", 0, junk) as i32, -7, "int, inputs {junk:#x}");
        assert_eq!(run(&uint, "uint", 0, junk), 4_000_000_000, "uint, inputs {junk:#x}");
    }
}

/// A uniform is read from the uniform file, not from an iterated value.
#[test]
fn uniform_lands_in_the_output() {
    let int = fragment("int", "o_out = u;");
    let uint = fragment("uint", "o_out = u;");
    for x in [-123_456i32, 0, 77, i32::MIN] {
        assert_eq!(run(&int, "int", 0, x as u32) as i32, x, "int {x}");
    }
    for x in [0u32, 4_000_000_000, u32::MAX] {
        assert_eq!(run(&uint, "uint", 0, x), x, "uint {x}");
    }
}
