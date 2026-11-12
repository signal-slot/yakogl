// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A scalar `int` or `uint` fragment colour output whose value is an integer
//! built-in (`abs`, `sign`, `min`, `max`, `clamp`) of a flat varying must
//! build on the GC7000UL and the V3D, the counterpart of the PowerVR
//! built-in guard. Each program is compiled and linked through
//! `SHC_ONLINE`, as glCompileShader and glLinkProgram do.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_out_builtin_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_out_builtin_gc_v3d
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

/// A flat integer varying passed through by the matching vertex shader.
fn vertex(ty: &str) -> String {
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\nflat out highp {ty} v_in0;\n\
         void main() {{ gl_Position = a_position; v_in0 = a_in0; }}\n"
    )
}

fn check(ty: &str, expr: &str) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nflat in highp {ty} v_in0;\n\
         layout(location = 0) out highp {ty} o_out;\nvoid main() {{ o_out = {expr}; }}\n"
    );
    build(&vertex(ty), &fs).unwrap_or_else(|e| panic!("{ty} `{expr}`: {e}"));
}

#[test]
fn int_output_from_builtins_builds() {
    for expr in ["abs(v_in0)", "sign(v_in0)", "min(v_in0, 3)", "max(v_in0, 3)", "clamp(v_in0, -2, 5)"] {
        check("int", expr);
    }
}

#[test]
fn uint_output_from_builtins_builds() {
    for expr in ["min(v_in0, 3u)", "max(v_in0, 3u)", "clamp(v_in0, 1u, 5u)"] {
        check("uint", expr);
    }
}
