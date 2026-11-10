// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A scalar `int` or `uint` colour output whose value is an ALU result, on
//! the PowerVR online back end: the shape of dEQP-GLES3's
//! `shaders.precision.{int,uint}.*_fragment` cases (`o_out = in0 + in1`)
//! and of `o_out = int(f)`. Selection read all four lanes of a colour output
//! from its value, and a one-lane ALU result writes only lane 0, so the
//! program read registers nothing writes and was refused ("a register read
//! that nothing writes"). The lanes past the output's declared width now
//! repeat lane 0 (select.rs `StoreOutput`).
//!
//! What is checked is the integer that lands in pixel output 0 of the
//! integer render target variant (R32I / R32UI, `shc_blend_key_int_rt`),
//! with the built USC bytes run through the host byte model
//! (tests/usc_exec `run_binary`). HOST-ONLY: no GPU arithmetic, and no GL
//! read-back -- no host row renders and reads back an integer target.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_alu_pvr
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

/// dEQP's `precision.int.*_add_fragment` shape, `o_out = v + 1`: the sum,
/// wrapping at 32 bits, in R32I.
#[test]
fn int_add_lands_in_the_output() {
    let src = fragment("int", "int", "o_out = v + 1;");
    for x in [41i32, -5, 0, -1, i32::MAX] {
        assert_eq!(run(&src, "int", x as u32) as i32, x.wrapping_add(1), "{x} + 1");
    }
}

/// The same in R32UI, `o_out = v + 1u` and `v * 3u`: unsigned wrap.
#[test]
fn uint_arithmetic_lands_in_the_output() {
    let add = fragment("uint", "uint", "o_out = v + 1u;");
    let mul = fragment("uint", "uint", "o_out = v * 3u;");
    for x in [41u32, 0, u32::MAX, 0x8000_0000] {
        assert_eq!(run(&add, "uint", x), x.wrapping_add(1), "{x} + 1u");
        assert_eq!(run(&mul, "uint", x), x.wrapping_mul(3), "{x} * 3u");
    }
}

/// `o_out = int(f)` and `uint(f)`: the conversion truncates toward zero.
#[test]
fn conversion_from_float_lands_in_the_output() {
    let int = fragment("float", "int", "o_out = int(v);");
    for (x, want) in [(2.75f32, 2i32), (-2.75, -2), (0.5, 0), (-100.0, -100), (1.0e6, 1_000_000)] {
        assert_eq!(run(&int, "int", x.to_bits()) as i32, want, "int({x})");
    }
    let uint = fragment("float", "uint", "o_out = uint(v);");
    for (x, want) in [(3.9f32, 3u32), (0.0, 0), (65536.5, 65536)] {
        assert_eq!(run(&uint, "uint", x.to_bits()), want, "uint({x})");
    }
}

/// The copy of the input, which built before, still writes the input.
#[test]
fn copy_still_lands_in_the_output() {
    let src = fragment("int", "int", "o_out = v;");
    assert_eq!(run(&src, "int", (-7i32) as u32) as i32, -7);
}

/// Float colour outputs narrower than a vec4 from an ALU result were refused
/// the same way: `vec2` and `vec3` of a sum build under the default variant.
#[test]
fn narrow_float_outputs_from_arithmetic_build() {
    for ty in ["float", "vec2", "vec3"] {
        let src = format!(
            "#version 300 es\nprecision highp float;\nin vec4 v;\nlayout(location = 0) out {ty} o;\n\
             void main() {{ o = {ty}(v) + {ty}(0.25); }}\n"
        );
        compile_for(&src, 0).unwrap_or_else(|e| panic!("{ty}: {e}"));
    }
}
