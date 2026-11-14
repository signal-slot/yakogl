// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `usc_exec::take_pixout_writes`: the log of every pixel output register
//! write of `run_binary`, in program order, beside the map of last values.
//! A one-vec4 float output and a one-int output are compiled through the
//! online compiler and run through the host byte model. HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test usc_exec_writes
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn compile_for(src: &str, key: u32) -> Vec<u8> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).expect("front end");
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).expect("chain");
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        code.iter().flat_map(|w| w.to_le_bytes()).collect()
    })
    .unwrap()
}

/// Run `image`, check the log against the map's keys, return the log.
fn check(image: &[u8]) -> Vec<u32> {
    usc_exec::take_pixout_writes();
    let inp = usc_exec::sentinel_inputs(1.0);
    let out = usc_exec::run_binary(image, &inp, true).expect("byte model");
    let writes = usc_exec::take_pixout_writes();
    assert!(usc_exec::take_pixout_writes().is_empty(), "a second take is empty");
    let logged: std::collections::BTreeSet<u32> = writes.iter().copied().collect();
    assert_eq!(logged, out.keys().copied().collect(), "writes match the map's keys");
    writes
}

#[test]
fn vec4_output_writes_each_register_once_in_order() {
    let src = "#version 300 es\nprecision highp float;\nin vec4 v;\nlayout(location = 0) out vec4 o;\n\
               void main() { o = v + vec4(0.25); }\n";
    let writes = check(&compile_for(src, 0));
    assert!(!writes.is_empty());
    assert!(writes.windows(2).all(|w| w[0] < w[1]), "ascending, each once: {writes:?}");
}

#[test]
fn int_output_writes_register_zero_once() {
    let src = "#version 300 es\nflat in highp int v;\nlayout(location = 0) out highp int o;\n\
               void main() { o = v + 1; }\n";
    let key = shc_blend_key_int_rt(ShcIntRt { channels: 1, bits: 32, signed: true });
    assert_eq!(check(&compile_for(src, key)), [0]);
}
