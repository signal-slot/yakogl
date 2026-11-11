// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An `ivec*` or `uvec*` fragment colour output whose value is an ALU result
//! (`o_out = v_in0 + ivec4(1);`, `o_out = ivec4(v_f);`) must build on the
//! GC7000UL and the V3D. This is the vector case of
//! `tests/es3_int_out_alu_gc_v3d.rs`, which pins the scalar one the PowerVR
//! online selector refuses (docs/es3/shader-host-repro.md row 5,
//! `int-rt-pvr`); pinning the vectors on the other two back ends too lets the
//! PowerVR results be compared against them, and keeps a later change shared
//! with the PowerVR fix from breaking them. Each program is compiled and
//! linked through `SHC_ONLINE`, as glCompileShader and glLinkProgram do.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_vec_out_alu_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_vec_out_alu_gc_v3d
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

/// A vertex stage for the vector case: a flat integer vector input and a
/// float vector of the same width, for the conversion shape.
fn vertex(ty: &str, n: u32) -> String {
    let swizzle = match n {
        2 => "xy",
        3 => "xyz",
        _ => "xyzw",
    };
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\n\
         flat out highp {ty} v_in0;\nout highp vec{n} v_f;\n\
         void main() {{ gl_Position = a_position; v_in0 = a_in0; v_f = a_position.{swizzle}; }}\n"
    )
}

/// One program per type: the fragment output `o_out` takes `body`'s value.
fn build_shape(ty: &str, n: u32, body: String) {
    let vs = vertex(ty, n);
    let fs = format!(
        "#version 300 es\nprecision highp float;\nflat in highp {ty} v_in0;\nin highp vec{n} v_f;\n\
         layout(location = 0) out highp {ty} o_out;\nvoid main() {{ {body} }}\n"
    );
    build(&vs, &fs).unwrap_or_else(|e| panic!("{ty} `{body}`: {e}"));
}

/// `o_out = v_in0 + ivec4(1);` and `o_out = uvec4(v_in0 + 1u);`'s shape: the
/// ALU result, an add of the flat input and a constant, lands in the vector
/// integer colour output.
#[test]
fn vector_integer_output_from_add_builds() {
    for (base, one) in [("ivec", "1"), ("uvec", "1u")] {
        for n in 2..=4u32 {
            let ty = format!("{base}{n}");
            build_shape(&ty, n, format!("o_out = v_in0 + {ty}({one});"));
        }
    }
}

/// The conversion from a float vector: `o_out = ivec3(v_f);` and
/// `o_out = uvec3(v_f);`, component for component.
#[test]
fn vector_integer_output_from_conversion_builds() {
    for base in ["ivec", "uvec"] {
        for n in 2..=4u32 {
            let ty = format!("{base}{n}");
            build_shape(&ty, n, format!("o_out = {ty}(v_f);"));
        }
    }
}

/// The control: a plain copy of the flat input builds, as it does for the
/// scalar case (tests/es3_int_out_alu_gc_v3d.rs).
#[test]
fn vector_integer_output_copy_builds() {
    for base in ["ivec", "uvec"] {
        for n in 2..=4u32 {
            let ty = format!("{base}{n}");
            build_shape(&ty, n, "o_out = v_in0;".to_string());
        }
    }
}
