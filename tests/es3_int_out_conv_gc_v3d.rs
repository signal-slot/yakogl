// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A scalar `int` or `uint` fragment colour output computed by a conversion
//! (`int(u)` from a flat uint, `uint(i)` from a flat int, `int(v > 3)`,
//! `uint(v > 3)`) or from arithmetic with a uniform operand (`v * 2 + u_k`)
//! must build on the GC7000UL and the V3D. This is the counterpart of the
//! PowerVR value checks; each program is compiled and linked through
//! `SHC_ONLINE`, as glCompileShader and glLinkProgram do.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_out_conv_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_out_conv_gc_v3d
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

/// A vertex shader passing a `vty` attribute to a flat varying, and a
/// fragment shader writing `body` to an output of type `oty`.
fn shape(vty: &str, oty: &str, body: &str) -> Result<(), String> {
    let vs = format!(
        "#version 300 es\nin highp vec4 a_position;\nin highp {vty} a_in0;\nflat out highp {vty} v_in0;\n\
         void main() {{ gl_Position = a_position; v_in0 = a_in0; }}\n"
    );
    let fs = format!(
        "#version 300 es\nprecision highp float;\nflat in highp {vty} v_in0;\nuniform highp int u_k;\n\
         layout(location = 0) out highp {oty} o_out;\nvoid main() {{ {body} }}\n"
    );
    build(&vs, &fs)
}

#[test]
fn int_from_uint() {
    shape("uint", "int", "o_out = int(v_in0);").unwrap();
}

#[test]
fn uint_from_int() {
    shape("int", "uint", "o_out = uint(v_in0);").unwrap();
}

#[test]
fn int_from_bool() {
    shape("int", "int", "o_out = int(v_in0 > 3);").unwrap();
}

#[test]
fn uint_from_bool() {
    shape("int", "uint", "o_out = uint(v_in0 > 3);").unwrap();
}

#[test]
fn int_from_uniform_operand() {
    shape("int", "int", "o_out = v_in0 * 2 + u_k;").unwrap();
}
