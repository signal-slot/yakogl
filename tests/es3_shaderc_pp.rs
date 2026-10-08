// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 line continuation (section 3.1/3.2): a backslash right before
//! a newline is removed before tokenisation, inside a directive name, an
//! identifier, a number or an operator too. The dEQP-GLES3
//! `preprocessor.line_continuation` bodies, as inline sources. GLSL ES 1.00
//! has no line continuation and keeps refusing the same splice.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_pp
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

fn compile(src: &str) -> Result<(), String> {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc =
        unsafe { compile(SHC_STAGE_FRAGMENT, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect())
    }
}

/// A `#version 300 es` fragment shader: 3 header lines, then `body` from line 4.
fn frag(body: &str) -> String {
    format!("#version 300 es\nprecision mediump float;\nout mediump vec4 o;\n{body}\n")
}

fn ok(body: &str) {
    let src = frag(body);
    if let Err(e) = compile(&src) {
        panic!("refused: {e}\n---\n{src}");
    }
}

fn directive_name_split() {
    ok("#def\\\nine X 1\nvoid main() { o = vec4(float(X)); }");
    ok("#\\\ndefine X 1\n#if\\\ndef X\nvoid main() { o = vec4(float(X)); }\n#endif");
}

fn identifier_split() {
    ok("void main() { float f\\\noo = 1.0; o = vec4(foo); }");
    ok("#define F\\\nOO 1.0\nvoid main() { o = vec4(FOO); }");
}

fn number_split() {
    ok("void main() { o = vec4(1\\\n.5, 0.\\\n25, 1\\\n0.0, 1e\\\n1); }");
    ok("void main() { int i = 0x\\\n1F; o = vec4(float(i)); }");
}

fn operator_split() {
    ok("void main() { float a = 1.0; a +\\\n= 2.0; o = vec4(a); }");
    ok("void main() { int a = 1; a <\\\n<= 1; o = vec4(float(a)); }");
    ok("void main() { int a = 1; if (a =\\\n= 1) o = vec4(1.0); else o = vec4(0.0); }");
}

fn crlf_split() {
    ok("void main() { float f\\\r\noo = 1.0; o = vec4(foo); }");
}

fn spliced_line_still_counts() {
    // __LINE__ after one spliced line (body starts at line 4): `#if` is on 6.
    ok("int x\\\n;\n#if __LINE__ != 6\n#error wrong line\n#endif\nvoid main() { o = vec4(0.0); }");
    // A preprocessor diagnostic after a spliced directive is on its real line
    // (the sema errors carry no line number in this build, so use #error).
    let e = compile(&frag("#def\\\nine X 1\n#error boom")).unwrap_err();
    assert!(e.contains("boom"), "{e}");
    assert!(e.starts_with("0:6:") || e.contains(":6:"), "line 6 expected: {e}");
}

fn es100_has_no_continuation() {
    // Same splices in a #version 100 source stay refused, as before.
    assert!(compile("#def\\\nine X 1\nvoid main() { gl_FragColor = vec4(1.0); }\n").is_err());
    assert!(compile("void main() { float f\\\noo = 1.0; gl_FragColor = vec4(foo); }\n").is_err());
}

/// The dEQP bodies as they are (macro use, and a backslash in the middle of a line).
fn deqp_bodies() {
    ok("#def\\\nine A(X) (-1.0*(X))\nvoid main() { o = vec4(A(-1.0)); }");
    ok("void main()\n{\n    float f\\\noo = 1.0;\n    o = vec4(foo);\n}");
    ok("#define A(X) \\\n    (-1.0*(X))\nvoid main() { o = vec4(A(-1.0)); }");
    ok("#define A a \\ b\n#define B 1.0\nvoid main() { o = vec4(B); }");
}

/// One `#[test]`: the compiler's keep arena is reset per compile and is not
/// reentrant, so a binary that resets it holds a single test (src/test_rules.rs).
#[test]
fn line_continuation() {
    directive_name_split();
    identifier_split();
    number_split();
    operator_split();
    crlf_split();
    spliced_line_still_counts();
    es100_has_no_continuation();
    deqp_bodies();
}
