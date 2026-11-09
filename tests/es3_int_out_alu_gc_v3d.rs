// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A scalar `int` or `uint` fragment colour output whose value is an ALU
//! result (`o_out = v_in0 + 1;`, `int(f)`) must build on the GC7000UL and
//! the V3D. The PowerVR online selector refuses that shape (docs/es3/
//! shader-host-repro.md row 5, `int-rt-pvr`); the other two back ends build
//! all 48 `shaders.precision.{int,uint}` programs, and this file pins it so a
//! later change shared with the PowerVR fix cannot break them. Each program
//! is compiled and linked through `SHC_ONLINE`, as glCompileShader and
//! glLinkProgram do.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_out_alu_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_out_alu_gc_v3d
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

/// glsShaderExecUtil's vertex stage for two flat integer inputs.
fn vertex(prec: &str, ty: &str) -> String {
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin {prec} {ty} a_in0;\nin {prec} {ty} a_in1;\n\
         flat out {prec} {ty} v_in0;\nflat out {prec} {ty} v_in1;\n\
         void main() {{ gl_Position = a_position; v_in0 = a_in0; v_in1 = a_in1; }}\n"
    )
}

/// The shapes of `int_output_from_arithmetic_is_the_powervr_shape`
/// (tests/es3_shaderc_repro.rs), for `int` and `uint`.
#[test]
fn scalar_integer_output_shapes_build() {
    for (ty, one, konst, conv) in [("int", "1", "3", "int(f)"), ("uint", "1u", "3u", "uint(f)")] {
        let vs = format!(
            "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\nflat out highp {ty} v_in0;\n\
             void main() {{ gl_Position = a_position; v_in0 = a_in0; }}\n"
        );
        for body in [
            "o_out = v_in0;".to_string(),
            format!("o_out = {konst};"),
            "o_out = u;".to_string(),
            format!("o_out = v_in0 + {one};"),
            format!("o_out = {conv};"),
        ] {
            let fs = format!(
                "#version 300 es\nprecision highp float;\nflat in highp {ty} v_in0;\nuniform highp {ty} u;\n\
                 uniform highp float f;\nlayout(location = 0) out highp {ty} o_out;\nvoid main() {{ {body} }}\n"
            );
            build(&vs, &fs).unwrap_or_else(|e| panic!("{ty} `{body}`: {e}"));
        }
    }
}

/// The 48 `shaders.precision.{int,uint}.{add,sub,mul,div}.{lowp,mediump,highp}_{vertex,fragment}`
/// programs: the fragment ones put the ALU result in the colour output, the
/// vertex ones pass it through a flat varying.
#[test]
fn deqp_precision_int_uint_programs_build() {
    let mut n = 0;
    for ty in ["int", "uint"] {
        for (op, sym) in [("add", "+"), ("sub", "-"), ("mul", "*"), ("div", "/")] {
            for prec in ["lowp", "mediump", "highp"] {
                let fs = format!(
                    "#version 300 es\nflat in {prec} {ty} v_in0;\nflat in {prec} {ty} v_in1;\n\
                     layout(location = 0) out highp {ty} o_out0;\n\nvoid main (void)\n{{\n\
                     \t{prec} {ty} in0 = v_in0;\n\t{prec} {ty} in1 = v_in1;\n\t{prec} {ty} out0;\n\
                     \tout0 = in0 {sym} in1;\n\to_out0 = out0;\n}}\n"
                );
                build(&vertex(prec, ty), &fs).unwrap_or_else(|e| panic!("{ty}.{op}.{prec}_fragment: {e}"));
                let vs = format!(
                    "#version 300 es\nin highp vec4 a_position;\nin {prec} {ty} a_in0;\nin {prec} {ty} a_in1;\n\
                     flat out {prec} {ty} vtx_out_out0;\n\nvoid main (void)\n{{\n\
                     \tgl_Position = a_position;\n\t{prec} {ty} out0 = a_in0 {sym} a_in1;\n\
                     \tvtx_out_out0 = out0;\n}}\n"
                );
                let fs = format!(
                    "#version 300 es\nflat in {prec} {ty} vtx_out_out0;\n\
                     layout(location = 0) out highp {ty} o_out0;\nvoid main (void)\n{{\n\to_out0 = vtx_out_out0;\n}}\n"
                );
                build(&vs, &fs).unwrap_or_else(|e| panic!("{ty}.{op}.{prec}_vertex: {e}"));
                n += 2;
            }
        }
    }
    assert_eq!(n, 48);
}
