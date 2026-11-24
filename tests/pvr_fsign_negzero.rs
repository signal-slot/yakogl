// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL `sign()` on the PowerVR online back end: `sign(-0.0)` is 0, not -1.0
//! (AM62P dEQP-GLES3 builtin_functions.precision.sign QualityWarnings, item
//! pvr-fsign). The FSIGN group (src/shaderc/online/powervr/shapes.rs) computes
//! `sat(x * +inf)` and selects -1.0 by a `tst gez`; that test was typed s32
//! (the raw sign bit), so -0.0 took the -1.0 arm. It is typed f32 now, and an
//! IEEE compare takes -0.0 >= 0 as true.
//!
//! What is checked: `o = vec4(sign(v.x))` is built for the PowerVR (RGBA16F
//! variant, so -1.0 is kept) and its USC bytes are run through the host byte
//! model (tests/usc_exec `run_binary`) for x = -0.0, +0.0, +-1e-30, +-1,
//! +-inf. Zero of either sign comes out zero of either sign; +-1 and +-inf
//! come out +-1; +-1e-30 come out +-1, or 0 where the hardware flushes it.
//! Revert check: with `("type", "s32")` put back in FSIGN, -0.0 gives
//! 0xbf80_0000 (-1.0) and the first test fails. HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_fsign_negzero
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_rt, SHC_BLEND_KEY_REPLACE, SHC_RT_F16X4};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn compile(src: &str, key: u32) -> Vec<u8> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).unwrap_or_else(|e| panic!("front end: {}", e.msg));
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).unwrap_or_else(|e| panic!("chain: {e:?}"));
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        code.iter().flat_map(|w| w.to_le_bytes()).collect()
    })
    .unwrap()
}

/// `sign(x)` as the PowerVR computes it, lane 0 of pixel output 0 as a float.
fn sign_of(x: u32) -> f32 {
    let src = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
               layout(location = 0) out vec4 o;\nvoid main() { o = vec4(sign(v.x)); }\n";
    let img = compile(src, SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F16X4));
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff[0] = 1.0f32.to_bits();
    inp.coeff[4] = x;
    let out = usc_exec::run_binary(&img, &inp, true).unwrap_or_else(|e| panic!("byte model: {e}"));
    let w = *out.get(&0).unwrap_or_else(|| panic!("nothing in pixel output 0: {out:?}"));
    usc_exec::f16_to_f32(w as u16)
}

#[test]
fn negative_zero_gives_zero_not_minus_one() {
    for x in [0x8000_0000u32, 0] {
        let r = sign_of(x);
        assert!(r == 0.0, "sign({x:#010x}) = {r}, want 0");
    }
}

#[test]
fn other_values_keep_their_sign() {
    for (x, want) in [(1.0f32, 1.0f32), (-1.0, -1.0), (f32::INFINITY, 1.0), (f32::NEG_INFINITY, -1.0)] {
        assert_eq!(sign_of(x.to_bits()), want, "sign({x})");
    }
    for (x, want) in [(1e-30f32, 1.0f32), (-1e-30, -1.0)] {
        let r = sign_of(x.to_bits());
        assert!(r == want || r == 0.0, "sign({x}) = {r}, want {want} or a flushed 0");
    }
}
