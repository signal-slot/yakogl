// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A float `vec2` or `vec3` fragment colour output whose value is an ALU
//! result (`o = v_in.xy * 2.0 + 1.0;`, `o = vec3(v_in0) + u;`) must build on
//! the GC7000UL and the V3D, the counterpart of
//! tests/es3_float_vec_out_alu_pvr.rs. Each program is compiled and linked
//! through `SHC_ONLINE`, as glCompileShader and glLinkProgram do.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_float_vec_out_alu_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_float_vec_out_alu_gc_v3d
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

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec4 a_in;\nout highp vec4 v_in;\n\
                  void main() { gl_Position = a_position; v_in = a_in; }\n";

fn check(ty: &str, body: &str) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nin highp vec4 v_in;\nuniform highp {ty} u;\n\
         layout(location = 0) out highp {ty} o;\nvoid main() {{ {body} }}\n"
    );
    build(VS, &fs).unwrap_or_else(|e| panic!("{ty} `{body}`: {e}"));
}

#[test]
fn vec2_output_from_alu_builds() {
    for body in [
        "o = v_in.xy * 2.0 + 1.0;",
        "o = vec2(v_in.x) + u;",
        "o = v_in.xy + u;",
        "o = vec2(v_in.x + v_in.y, v_in.z * 0.5);",
        "o = v_in.yx - u * 3.0;",
    ] {
        check("vec2", body);
    }
}

#[test]
fn vec3_output_from_alu_builds() {
    for body in [
        "o = v_in.xyz * 2.0 + 1.0;",
        "o = vec3(v_in.x) + u;",
        "o = v_in.xyz * u;",
        "o = vec3(v_in.x + v_in.y, v_in.z * 0.5, v_in.w - 1.0);",
        "o = v_in.zyx - u * 3.0;",
    ] {
        check("vec3", body);
    }
}
