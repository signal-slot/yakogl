// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An integer fragment output whose value is an ALU result, next to a float
//! colour output (MRT), must build on the GC7000UL and the V3D. The PowerVR
//! counterpart is tests/es3_int_mrt_alu_pvr.rs. Each program is compiled and
//! linked through `SHC_ONLINE`, as glCompileShader and glLinkProgram do. Build
//! checks only: no pixel value is asserted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_mrt_alu_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_mrt_alu_gc_v3d
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

/// A flat integer vertex input forwarded to the fragment stage.
fn vertex(ty: &str) -> String {
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\nflat out highp {ty} v_in0;\n\
         void main() {{ gl_Position = a_position; v_in0 = a_in0; }}\n"
    )
}

/// A fragment shader with an integer output `o_i` at `loc_i` computed by
/// `expr`, and a `vec4` output `o_c` at `loc_c`.
fn mrt(ty: &str, expr: &str, loc_i: u32, loc_c: u32) -> Result<(), String> {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nflat in highp {ty} v_in0;\n\
         layout(location = {loc_c}) out vec4 o_c;\nlayout(location = {loc_i}) out highp {ty} o_i;\n\
         void main() {{ o_c = vec4(0.25, 0.5, 0.75, 1.0); o_i = {expr}; }}\n"
    );
    build(&vertex(ty), &fs)
}

/// (a) `vec4` at 0, `int` from `v_in0 + 1` at 1.
#[test]
fn vec4_then_int_from_arithmetic_builds() {
    mrt("int", "v_in0 + 1", 1, 0).unwrap_or_else(|e| panic!("{e}"));
}

/// (b) the same with `uint`.
#[test]
fn vec4_then_uint_from_arithmetic_builds() {
    mrt("uint", "v_in0 + 1u", 1, 0).unwrap_or_else(|e| panic!("{e}"));
}

/// (c) `int` at 0, `vec4` at 1.
#[test]
fn int_then_vec4_from_arithmetic_builds() {
    mrt("int", "v_in0 + 1", 0, 1).unwrap_or_else(|e| panic!("{e}"));
}

/// (d) `ivec2` from arithmetic next to a `vec4`.
#[test]
fn ivec2_from_arithmetic_next_to_vec4_builds() {
    mrt("ivec2", "v_in0 + ivec2(1, 2)", 1, 0).unwrap_or_else(|e| panic!("{e}"));
}
