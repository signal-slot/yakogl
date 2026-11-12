// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000UL and V3D counterpart of the PowerVR integer-output fix
//! (3efd7b30, docs/notes/pvr-int-output-alu.md, docs/es3/shader-host-repro.md
//! row 5): a scalar `int` or `uint` fragment colour output whose value comes
//! from control flow -- written on both arms of an `if`/`else` on a varying,
//! overwritten in one arm after a default, accumulated in a constant-bound
//! `for` loop -- from a helper function's return, or from bitwise (`&`, `|`,
//! `^`, `<<`, `>>`, `~`) and `%` arithmetic must compile and link on both
//! back ends. The PowerVR selector refused the arithmetic shapes because it
//! read a colour output's lanes from its value and a one-lane ALU result
//! writes only lane 0 ("a register read that nothing writes"); the control-flow
//! shapes are the same question in a harder form, so they are pinned here
//! while they build. tests/es3_int_out_alu_gc_v3d.rs already pins the plain
//! ALU shapes; this file adds the rest, plus one `ivec2` and one `uvec4`
//! output, each written under control flow or returned from a helper. Each
//! program is compiled and linked through `SHC_ONLINE`, as glCompileShader
//! and glLinkProgram do. A shape a back end refuses stays as an
//! `#[ignore = "..."]` test carrying its message; the open issues keep the
//! list (docs/es3-cts-failures.md RC13).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_out_cf_gc_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_out_cf_gc_v3d
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
fn vertex(ty: &str) -> String {
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\nin highp {ty} a_in1;\n\
         flat out highp {ty} v_in0;\nflat out highp {ty} v_in1;\n\
         void main() {{ gl_Position = a_position; v_in0 = a_in0; v_in1 = a_in1; }}\n"
    )
}

/// `int` and `uint` with the literals that share their type.
const TYPES: [(&str, &str, &str, &str); 2] = [("int", "0", "1", "2"), ("uint", "0u", "1u", "2u")];

/// One program: the vertex stage above and a fragment stage with two flat
/// inputs of `ty`, one uniform of `ty`, the colour output declaration `decl`
/// and `main`'s body `body`.
fn build_frag(ty: &str, decl: &str, body: &str) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nflat in highp {ty} v_in0;\nflat in highp {ty} v_in1;\n\
         uniform highp {ty} u;\n{decl}\nvoid main() {{ {body} }}\n"
    );
    build(&vertex(ty), &fs).unwrap_or_else(|e| panic!("{ty} `{body}`: {e}"));
}

/// The output is written on both arms of an `if`/`else` whose condition is a
/// varying comparison.
#[test]
fn if_else_on_a_varying_writes_the_output_on_both_paths() {
    for (ty, zero, one, _) in TYPES {
        let body = format!("if (v_in0 > {zero}) {{ o_out = v_in0 + {one}; }} else {{ o_out = v_in1 - {one}; }}");
        build_frag(ty, &format!("layout(location = 0) out highp {ty} o_out;"), &body);
    }
}

/// A default write, overwritten in one arm: the output's value reaches the
/// store from a conditional path and from a straight path in the same program.
#[test]
fn a_default_is_overwritten_in_one_branch() {
    for (ty, zero, one, _) in TYPES {
        let body = format!("o_out = v_in0; if (v_in0 > {zero}) {{ o_out = v_in1 + {one}; }}");
        build_frag(ty, &format!("layout(location = 0) out highp {ty} o_out;"), &body);
    }
}

/// The output accumulates a varying in a `for` loop with a constant bound.
#[test]
fn a_constant_bound_loop_accumulates_into_the_output() {
    for (ty, zero, _, _) in TYPES {
        let body = format!("{{ {ty} acc = {zero}; for (int i = 0; i < 4; i++) {{ acc += v_in0; }} o_out = acc; }}");
        build_frag(ty, &format!("layout(location = 0) out highp {ty} o_out;"), &body);
    }
}

/// The output is the return value of a helper function.
#[test]
fn the_output_is_a_helper_function_return() {
    for (ty, _, _, two) in TYPES {
        let fs = format!(
            "#version 300 es\nprecision highp float;\nflat in highp {ty} v_in0;\nflat in highp {ty} v_in1;\n\
             uniform highp {ty} u;\n\
             {ty} helper({ty} a, {ty} b) {{ return a * {two} + b; }}\n\
             layout(location = 0) out highp {ty} o_out;\nvoid main() {{ o_out = helper(v_in0, v_in1); }}\n"
        );
        build(&vertex(ty), &fs).unwrap_or_else(|e| panic!("{ty} helper return: {e}"));
    }
}

/// Each bitwise operator and `%`, the uniform as the second operand so both
/// operands share the output's signedness (`~` is unary).
#[test]
fn the_output_is_a_bitwise_or_modulo_result() {
    for (ty, _, _, _) in TYPES {
        for op in ["&", "|", "^", "<<", ">>", "%"] {
            let body = format!("o_out = v_in0 {op} u;");
            build_frag(ty, &format!("layout(location = 0) out highp {ty} o_out;"), &body);
        }
        build_frag(ty, &format!("layout(location = 0) out highp {ty} o_out;"), "o_out = ~v_in0;");
    }
}

/// Output from uniform-plus-varying arithmetic, plain and compounded.
#[test]
fn the_output_is_uniform_plus_varying_arithmetic() {
    for (ty, _, _, two) in TYPES {
        for body in ["o_out = u + v_in0;".to_string(), format!("o_out = (u + v_in0) * v_in1 - {two};")] {
            build_frag(ty, &format!("layout(location = 0) out highp {ty} o_out;"), &body);
        }
    }
}

/// An `ivec2` output, written with bitwise results on both arms of an
/// `if`/`else`.
#[test]
fn ivec2_output_from_control_flow_and_bitwise_results() {
    let body = "if (v_in0 > 0) { o_out = ivec2(v_in0 & v_in1, v_in0 + 1); } else { o_out = ivec2(v_in1 >> 1, v_in0 ^ v_in1); }";
    build_frag("int", "layout(location = 0) out highp ivec2 o_out;", body);
}

/// A `uvec4` output, returned from a helper whose lanes are bitwise results.
#[test]
fn uvec4_output_from_a_helper_of_bitwise_results() {
    let fs = "#version 300 es\nprecision highp float;\nflat in highp uint v_in0;\nflat in highp uint v_in1;\n\
             uniform highp uint u;\n\
             uvec4 compose(uint a, uint b) { return uvec4(a & b, a | b, a ^ b, ~(a % (b | 1u))); }\n\
             layout(location = 0) out highp uvec4 o_out;\nvoid main() { o_out = compose(v_in0, v_in1); }\n";
    build(&vertex("uint"), fs).unwrap_or_else(|e| panic!("uvec4 helper: {e}"));
}
