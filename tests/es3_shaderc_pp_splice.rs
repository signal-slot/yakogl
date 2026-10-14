// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 line continuation inside a token: the four dEQP-GLES3
//! `shaders.preprocessor.line_continuation` cases that the coverage table once
//! listed as refused (`preprocessing_token_*`, `token_*`).
//!
//! The sources are the `both ""..""` bodies of
//! `data/gles3/shaders/preprocessor.test` (VK-GL-CTS, Apache-2.0, Copyright
//! (c) 2014 The Android Open Source Project and The Khronos Group Inc.), after
//! the two steps dEQP's `ShaderParser::parseShaderSource` does: the `\\`
//! escape is resolved, and `removeExtraIndentation` takes the indentation of
//! the first line off every line. The second step is the one that matters:
//! `#def\` + newline + `            ine` joins to `#define` only when the
//! second line starts in column 0. The compiler was right to refuse the
//! indented text (`#def            ine`); the corpus harness
//! (tests/es3_shaderc_corpus.rs) did not remove the indentation.
//! `indented_text_is_still_refused` pins that, so the splice is not
//! "fixed" by gluing across white space.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_pp_splice
//! ```
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

fn compile(stage: u32, src: &str) -> Result<(), String> {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect())
    }
}

/// `${DECLARATIONS}` and `${OUTPUT}` of dEQP's `specializeVertexShader` and
/// `specializeFragmentShader` for `values { output float out0 = 1.0; }`.
const VERTEX_DECLS: &str = "in highp vec4 dEQP_Position;\nout mediump float out0;\n";
const VERTEX_OUTPUT: &str = "gl_Position = dEQP_Position;";
const FRAGMENT_DECLS: &str =
    "layout(location = 0) out mediump vec4 dEQP_FragColor;\nuniform float ref_out0;\nfloat out0;\n";
const FRAGMENT_OUTPUT: &str = "dEQP_FragColor = vec4(out0 == ref_out0);";

/// `line_continuation.preprocessing_token`, indentation removed.
const PREPROCESSING_TOKEN: &str = "#version 300 es\nprecision mediump float;\n@DECLARATIONS@\n#def\\\nine A(X) (-1.0*(X))\n\nvoid main ()\n{\n    out0 = A(-1.0);\n    @OUTPUT@\n}\n";
/// `line_continuation.token`, indentation removed.
const TOKEN: &str = "#version 300 es\nprecision mediump float;\n@DECLARATIONS@\n\nvoid main ()\n{\n    float f\\\noo = 1.0;\n    out0 = foo;\n    @OUTPUT@\n}\n";

/// Stands in for dEQP's `${DECLARATIONS}` / `${OUTPUT}` template parameters.
fn expand(src: &str, decls: &str, output: &str) -> String {
    src.replace("@DECLARATIONS@", decls).replace("@OUTPUT@", output)
}

fn both_stages(name: &str, src: &str) {
    for (stage, decls, output) in
        [(SHC_STAGE_VERTEX, VERTEX_DECLS, VERTEX_OUTPUT), (SHC_STAGE_FRAGMENT, FRAGMENT_DECLS, FRAGMENT_OUTPUT)]
    {
        let text = expand(src, decls, output);
        if let Err(e) = compile(stage, &text) {
            panic!("{name} (stage {stage}) refused: {e}\n---\n{text}");
        }
    }
}

fn deqp_cases() {
    both_stages("preprocessing_token", PREPROCESSING_TOKEN);
    both_stages("token", TOKEN);
}

/// With the indentation left on the second line the splice yields white space
/// inside the name, which is a real error: this is the text the corpus harness
/// used to build.
fn indented_text_is_still_refused() {
    let indented = PREPROCESSING_TOKEN.replace("\\\nine", "\\\n            ine");
    let e = compile(SHC_STAGE_FRAGMENT, &expand(&indented, FRAGMENT_DECLS, FRAGMENT_OUTPUT)).unwrap_err();
    assert!(e.contains("def"), "{e}");
    let indented = TOKEN.replace("\\\noo", "\\\n            oo");
    assert!(compile(SHC_STAGE_FRAGMENT, &expand(&indented, FRAGMENT_DECLS, FRAGMENT_OUTPUT)).is_err());
}

/// Positions after a splice (section 3.2: the line numbers are those of the
/// unspliced text). `#def\` / `ine` is lines 4-5 here, so `#error` is line 6.
fn lines_after_a_splice() {
    let src = "#version 300 es\nprecision mediump float;\nout mediump vec4 o;\n#def\\\nine X 1\n#error boom\nvoid main() { o = vec4(0.0); }\n";
    let e = compile(SHC_STAGE_FRAGMENT, src).unwrap_err();
    assert!(e.contains("boom") && e.contains(":6:"), "{e}");
    let src = "#version 300 es\nprecision mediump float;\nout mediump vec4 o;\nint f\\\noo = 1;\n#if __LINE__ != 6\n#error wrong line\n#endif\nvoid main() { o = vec4(0.0); }\n";
    compile(SHC_STAGE_FRAGMENT, src).expect("__LINE__ counts the spliced line");
}

/// ES 1.00 has no line continuation: the same splices stay as they were.
fn es100_is_unchanged() {
    let directive = "#def\\\nine X 1.0\nvoid main() { gl_FragColor = vec4(X); }\n";
    assert!(compile(SHC_STAGE_FRAGMENT, directive).is_err());
    let token = "void main() {\n    float f\\\noo = 1.0;\n    gl_FragColor = vec4(foo);\n}\n";
    assert!(compile(SHC_STAGE_FRAGMENT, token).is_err());
}

/// One `#[test]`: the compiler's keep arena is reset per compile and is not
/// reentrant, so a binary that resets it holds a single test (src/test_rules.rs).
#[test]
fn pp_splice() {
    deqp_cases();
    indented_text_is_still_refused();
    lines_after_a_splice();
    es100_is_unchanged();
}
