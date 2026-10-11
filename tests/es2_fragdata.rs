// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2.functional.shaders.fragdata.*, replayed through the front end.
//!
//! The cases (VK-GL-CTS modules/gles2/functional/es2fShaderFragDataTests.cpp
//! and data/gles2/shaders/fragdata.test) compile one vertex and one fragment
//! shader and either draw (`valid_*`) or expect a compile failure
//! (`invalid_assign_to_1`, `write_fragcolor_and_fragdata_*`). The compile half
//! is replayed here. The GL query half, `glGetIntegerv(GL_MAX_DRAW_BUFFERS)`,
//! needs a GL context, which `testutil::TestGl` gives only to the library's
//! own unit tests (it is `cfg(test)`): it is pinned in src/gl/state.rs
//! `draw_buffer_queries_are_es3_only`. tools/cts/log/investigations/es2-fragdata.md has
//! the whole path.
//!
//! ```text
//! cargo test --no-default-features \
//!     --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_fragdata
//! ```
#![cfg(feature = "shaderc-online")]

use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, ir, sema::Stage};

/// The vertex shader of the three valid_* cases (es2fShaderFragDataTests.cpp:125).
const VALID_VS: &str =
    "attribute highp vec4 a_position;\nattribute highp float a_index;\nattribute highp vec4 a_color;\n\
varying mediump float v_index;\nvarying mediump vec4 v_color;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_color = a_color;\n    v_index = a_index;\n}\n";

/// The vertex shader of every negative case (fragdata.test).
const NEG_VS: &str = "void main (void)\n{\n    gl_Position = vec4(1.0);\n}\n";

/// genSources' fragment shader (es2fShaderFragDataTests.cpp:137).
fn valid_fs(index: &str) -> String {
    format!(
        "varying mediump vec4 v_color;\nvarying mediump float v_index;\nuniform mediump int u_index;\n\
         void main (void)\n{{\n    gl_FragData[{index}] = v_color;\n}}\n"
    )
}

/// Compile; Ok is the number of stores to colour output 0, Err the message
/// glGetShaderInfoLog would carry.
fn compile(stage: Stage, src: &str) -> Result<usize, String> {
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    match compile_to_ir(&a, stage, src) {
        Ok(l) => Ok(l
            .module
            .instrs
            .iter()
            .filter(|o| matches!(o, ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. }))
            .count()),
        Err(e) => Err(e.msg.to_string()),
    }
}

fn ok(stage: Stage, src: &str) -> usize {
    compile(stage, src).unwrap_or_else(|m| panic!("expected success for {src:?}, got: {m}"))
}

fn refused(stage: Stage, src: &str) -> String {
    match compile(stage, src) {
        Ok(_) => panic!("expected a compile failure for {src:?}"),
        Err(m) => m,
    }
}

#[test]
fn valid_static_index() {
    ok(Stage::Vertex, VALID_VS);
    assert_eq!(ok(Stage::Fragment, &valid_fs("0")), 1);
}

#[test]
fn valid_uniform_index() {
    ok(Stage::Vertex, VALID_VS);
    assert_eq!(ok(Stage::Fragment, &valid_fs("u_index")), 1);
}

#[test]
fn valid_dynamic_index() {
    ok(Stage::Vertex, VALID_VS);
    assert_eq!(ok(Stage::Fragment, &valid_fs("int(v_index)")), 1);
}

/// fragdata.test:2. The case is NotSupported on a context whose
/// GL_MAX_DRAW_BUFFERS query answers more than 1; with the query refused
/// (report-es2) it runs, and the compile must fail.
#[test]
fn invalid_assign_to_1() {
    ok(Stage::Vertex, NEG_VS);
    let m = refused(Stage::Fragment, "void main (void)\n{\n    gl_FragData[1] = vec4(1.0);\n}\n");
    assert!(m.contains("one draw buffer"), "{m}");
}

/// The shader-side half of the 'exactly one draw buffer' premise: in an ES 2.0
/// report the highest index gl_MaxDrawBuffers allows is 0.
#[cfg(feature = "report-es2")]
#[test]
fn gl_max_draw_buffers_is_one_in_the_es2_report() {
    let src = "void main (void)\n{\n    gl_FragData[gl_MaxDrawBuffers - 1] = vec4(1.0);\n}\n";
    assert_eq!(ok(Stage::Fragment, src), 1);
    let src = "void main (void)\n{\n    gl_FragData[gl_MaxDrawBuffers] = vec4(1.0);\n}\n";
    refused(Stage::Fragment, src);
}

/// fragdata.test:21 -- ES 1.00 section 7.2: a shader may not write both.
#[test]
fn write_fragcolor_and_fragdata_simple() {
    refused(
        Stage::Fragment,
        "void main (void)\n{\n    gl_FragColor = vec4(1.0);\n    gl_FragData[0] = vec4(1.0);\n}\n",
    );
}

/// fragdata.test:39 -- the writes are in the two arms of a static `if`.
#[test]
fn write_fragcolor_and_fragdata_static_if() {
    refused(
        Stage::Fragment,
        "void main (void)\n{\n    if (false)\n        gl_FragColor = vec4(1.0);\n    else\n        gl_FragData[0] = vec4(1.0);\n}\n",
    );
}

/// fragdata.test:59 -- one of the writes is in a function main never calls.
#[test]
fn write_fragcolor_and_fragdata_unused_func() {
    refused(
        Stage::Fragment,
        "void unused (void)\n{\n    gl_FragData[0] = vec4(1.0);\n}\nvoid main (void)\n{\n    gl_FragColor = vec4(1.0);\n}\n",
    );
}
