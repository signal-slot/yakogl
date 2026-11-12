// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Precision guard for the integer fragment output on the GC7000UL and the
//! V3D: for each of `lowp`, `mediump` and `highp`, each of `int` and `uint`
//! and each of `+ - * /`, a program whose colour output has that precision
//! and whose value is the ALU result of two flat inputs builds. Lowp and
//! mediump can take other lowering paths than highp (the
//! `shaders.precision.{int,uint}` cases of dEQP-GLES3). Each program is
//! compiled and linked through `SHC_ONLINE`, as glCompileShader and
//! glLinkProgram do.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_out_prec_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_out_prec_gc_v3d
//! ```
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "gpu-powervr")
))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// glCompileShader on both stages, then glLinkProgram.
fn build(vs: &str, fs: &str) -> Result<(), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];
const TYPES: [&str; 2] = ["int", "uint"];
const OPS: [(&str, &str); 4] = [("add", "+"), ("sub", "-"), ("mul", "*"), ("div", "/")];

fn check(ty: &str, prec: &str, op: &str, sym: &str) {
    let vs = format!(
        "#version 300 es\nin highp vec4 a_position;\nin {prec} {ty} a_a;\nin {prec} {ty} a_b;\n\
         flat out {prec} {ty} v_a;\nflat out {prec} {ty} v_b;\n\
         void main() {{ gl_Position = a_position; v_a = a_a; v_b = a_b; }}\n"
    );
    let fs = format!(
        "#version 300 es\nprecision {prec} float;\nflat in {prec} {ty} v_a;\nflat in {prec} {ty} v_b;\n\
         layout(location=0) out {prec} {ty} o_out;\nvoid main(){{ o_out = v_a {sym} v_b; }}\n"
    );
    build(&vs, &fs).unwrap_or_else(|e| panic!("{prec} {ty} {op}: {e}"));
}

/// 3 precisions x 2 types x 4 operators.
#[test]
fn precision_int_uint_alu_outputs_build() {
    let mut n = 0;
    for prec in PRECISIONS {
        for ty in TYPES {
            for (op, sym) in OPS {
                check(ty, prec, op, sym);
                n += 1;
            }
        }
    }
    assert_eq!(n, 24);
}
