// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Regression pin: dEQP-GLES3 `shaders.precision.{int,uint,float}.*_fragment`
//! on the PowerVR online back end. Commit 3efd7b30 (select.rs `StoreOutput`
//! fills the lanes past a colour output's declared width with lane 0) removed
//! their 24 refusals ("a register read that nothing writes"); the host dEQP
//! run of `shaders.precision.` has no refusal and no link failure left. This
//! file keeps it that way with dEQP's own fragment shapes: two flat inputs
//! copied into locals of the case's precision, `o_out = in0 <op> in1` for add,
//! sub, mul and div at highp, mediump and lowp, and the float cases'
//! `o_out = floatBitsToUint(in0 + in1)`; then the same arithmetic into `ivec2`
//! and `uvec2` outputs.
//!
//! What is checked is the integer in each pixel output of the integer render
//! target variant (R32I / R32UI as the dEQP cases render to, RG32I / RG32UI
//! for the `vec2` outputs; `shc_blend_key_int_rt`), with the built USC bytes
//! run through the host byte model (tests/usc_exec `run_binary`). A mediump
//! or lowp result is compared in its low 16 or 8 bits, as dEQP masks it.
//! HOST-ONLY: no GPU arithmetic and no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_int_output
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One fragment shader through the chain under the variant `key`: the pixel
/// output dwords it declares and the USC bytes, or the refusal.
fn compile_for(src: &str, key: u32) -> Result<(u8, Vec<u8>), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok((rec.num_outputs, code.iter().flat_map(|w| w.to_le_bytes()).collect()))
    })
    .unwrap()
}

/// The 32-bit integer variant of `channels` channels.
fn key(channels: u8, signed: bool) -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels, bits: 32, signed })
}

/// dEQP's fragment shader of a precision case (`es3fShaderPrecisionTests.cpp`
/// `createIntUintPrecisionEvalProgram`): flat inputs `v_in0`, `v_in1` of
/// type `ty`, copied into locals, and `o_out = <expr>` into an output of
/// type `out_ty`.
fn precision_case(prec: &str, ty: &str, out_ty: &str, expr: &str) -> String {
    format!(
        "#version 300 es\nlayout(location = 0) out highp {out_ty} o_out;\n\
         flat in {prec} {ty} v_in0;\nflat in {prec} {ty} v_in1;\n\nvoid main (void)\n{{\n\
         \t{prec} {ty} in0 = v_in0;\n\t{prec} {ty} in1 = v_in1;\n\to_out = {expr};\n}}\n"
    )
}

/// `src`, built under `key`, run with every coefficient set by `coeff`
/// (index -> dword): the pixel output dwords, in order.
fn run(src: &str, key: u32, coeff: impl Fn(usize) -> u32) -> Vec<u32> {
    let (n, image) = compile_for(src, key).unwrap_or_else(|e| panic!("{src}: {e}"));
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    for (i, c) in inp.coeff.iter_mut().enumerate() {
        *c = coeff(i);
    }
    let out = usc_exec::run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    (0..u32::from(n))
        .map(|k| *out.get(&k).unwrap_or_else(|| panic!("{src}: nothing in pixel output {k}: {out:?}")))
        .collect()
}

/// Where component `c` of `v_in0` and of `v_in1` is read from (the
/// coefficient each iterates to): a run with every coefficient its own index
/// of `o_out = in0 * 4096 + in1` (component-wise for a vector).
fn locate(prec: &str, ty: &str, out_ty: &str, k: u32) -> Vec<(usize, usize)> {
    let src = precision_case(prec, ty, out_ty, &format!("in0 * {ty}(4096) + in1"));
    run(&src, k, |i| i as u32).into_iter().map(|v| ((v / 4096) as usize, (v % 4096) as usize)).collect()
}

/// Coefficient `i0` holding `a`, `i1` holding `b`, every other 0.
fn two(i0: usize, i1: usize, a: u32, b: u32) -> impl Fn(usize) -> u32 {
    move |i| match i {
        _ if i == i0 => a,
        _ if i == i1 => b,
        _ => 0,
    }
}

/// The bits dEQP compares at a precision.
fn mask(prec: &str) -> u32 {
    match prec {
        "highp" => u32::MAX,
        "mediump" => 0xffff,
        _ => 0xff,
    }
}

/// `int` at each precision into R32I: add, sub, mul and div of pairs that
/// wrap, cross zero and divide toward zero.
#[test]
fn int_precision_cases_land_in_r32i() {
    type Op = fn(i32, i32) -> i32;
    let ops: [(&str, Op); 4] = [
        ("in0 + in1", i32::wrapping_add),
        ("in0 - in1", i32::wrapping_sub),
        ("in0 * in1", i32::wrapping_mul),
        ("in0 / in1", i32::wrapping_div),
    ];
    let pairs = [(41, 7), (-5, 3), (i32::MAX, 2), (-100, -7), (12345, -10000), (i32::MIN + 1, -1)];
    for prec in ["highp", "mediump", "lowp"] {
        let at = locate(prec, "int", "int", key(1, true));
        assert_eq!(at.len(), 1, "{prec}: one dword");
        let (i0, i1) = at[0];
        assert_ne!(i0, i1, "{prec}: the two inputs read distinct coefficients");
        for (expr, f) in ops {
            let src = precision_case(prec, "int", "int", expr);
            for (a, b) in pairs {
                let got = run(&src, key(1, true), two(i0, i1, a as u32, b as u32));
                let want = f(a, b) as u32;
                assert_eq!(got[0] & mask(prec), want & mask(prec), "{prec} {expr} with ({a}, {b})");
            }
        }
    }
}

/// `uint` at each precision into R32UI.
#[test]
fn uint_precision_cases_land_in_r32ui() {
    type Op = fn(u32, u32) -> u32;
    let ops: [(&str, Op); 4] = [
        ("in0 + in1", u32::wrapping_add),
        ("in0 - in1", u32::wrapping_sub),
        ("in0 * in1", u32::wrapping_mul),
        ("in0 / in1", u32::wrapping_div),
    ];
    let pairs = [(41u32, 7u32), (3, 5), (u32::MAX, 2), (0x8000_0000, 10000), (1000, 1)];
    for prec in ["highp", "mediump", "lowp"] {
        let at = locate(prec, "uint", "uint", key(1, false));
        assert_eq!(at.len(), 1, "{prec}: one dword");
        let (i0, i1) = at[0];
        for (expr, f) in ops {
            let src = precision_case(prec, "uint", "uint", expr);
            for (a, b) in pairs {
                let got = run(&src, key(1, false), two(i0, i1, a, b));
                assert_eq!(got[0] & mask(prec), f(a, b) & mask(prec), "{prec} {expr} with ({a}, {b})");
            }
        }
    }
}

/// The float cases' shape into R32UI: `o_out = floatBitsToUint(in0 + in1)`,
/// a float ALU result reinterpreted into a one-lane integer output.
#[test]
fn float_precision_case_lands_in_r32ui() {
    let at = locate("highp", "uint", "uint", key(1, false));
    let (i0, i1) = at[0];
    let src = precision_case("highp", "float", "uint", "floatBitsToUint(in0 + in1)");
    for (a, b) in [(1.5f32, 2.25f32), (-3.0, 0.125), (1.0e6, -1.0e6)] {
        let got = run(&src, key(1, false), two(i0, i1, a.to_bits(), b.to_bits()));
        assert_eq!(got[0], (a + b).to_bits(), "floatBitsToUint({a} + {b})");
    }
}

/// `ivec2` and `uvec2` outputs of the same arithmetic into RG32I / RG32UI:
/// each lane lands in its own pixel output.
#[test]
fn vec2_outputs_land_in_both_lanes() {
    for (ty, out_ty, signed) in [("ivec2", "ivec2", true), ("uvec2", "uvec2", false)] {
        let k = key(2, signed);
        let at = locate("highp", ty, out_ty, k);
        assert_eq!(at.len(), 2, "{ty}: two dwords");
        let ins: [[u32; 2]; 2] = [[41, (-5i32) as u32], [7, 3]];
        let coeff = |i: usize| {
            for (c, &(i0, i1)) in at.iter().enumerate() {
                if i == i0 {
                    return ins[0][c];
                }
                if i == i1 {
                    return ins[1][c];
                }
            }
            0
        };
        type Op = fn(u32, u32) -> u32;
        let ops: [(&str, Op); 3] =
            [("in0 + in1", u32::wrapping_add), ("in0 - in1", u32::wrapping_sub), ("in0 * in1", u32::wrapping_mul)];
        for (expr, f) in ops {
            let src = precision_case("highp", ty, out_ty, expr);
            let got = run(&src, k, coeff);
            let want: Vec<u32> = (0..2).map(|c| f(ins[0][c], ins[1][c])).collect();
            assert_eq!(got, want, "{ty} {expr}");
        }
    }
}
