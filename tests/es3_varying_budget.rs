// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.00 varying budget (`reflect::varying_budget_es3`): a vertex /
//! fragment pair exactly at each limit links, one component past it is
//! refused with a message naming the limit. The limits are parameters, so
//! this knows no GPU; ES 3.0's minimums are 15 / 60 / 60 / 64.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_varying_budget
//! ```
#![cfg(all(feature = "shaderc-online", not(feature = "no-shader-table")))]
use yakogl::shaderc::online::{arena::Arena, reflect, sema::Stage};

const MIN: reflect::Es3VaryingLimits = reflect::Es3VaryingLimits {
    max_varying_vectors: 15,
    max_varying_components: 60,
    max_fragment_input_components: 60,
    max_vertex_output_components: 64,
};

/// `n` vec4 varyings v0.. plus an optional float `extra`; the vertex shader
/// writes `vs_n` of them (and `extra` when `vs_extra`), the fragment shader
/// reads `fs_n` (and `extra` when `fs_extra`).
fn pair(vs_n: usize, vs_extra: bool, fs_n: usize, fs_extra: bool) -> (String, String) {
    let n = vs_n.max(fs_n);
    let decl = |kw: &str| {
        let mut s = String::from("#version 300 es\nprecision mediump float;\n");
        for i in 0..n {
            s += &format!("{kw} vec4 v{i};\n");
        }
        s += &format!("{kw} float extra;\n");
        s
    };
    let mut vs = decl("out");
    vs += "void main() {\n  gl_Position = vec4(0.0);\n";
    for i in 0..vs_n {
        vs += &format!("  v{i} = vec4({i}.0);\n");
    }
    if vs_extra {
        vs += "  extra = 1.0;\n";
    }
    vs += "}\n";
    let mut fs = decl("in");
    fs += "out vec4 c;\nvoid main() {\n  c = vec4(0.0)";
    for i in 0..fs_n {
        fs += &format!(" + v{i}");
    }
    if fs_extra {
        fs += " + vec4(extra)";
    }
    fs += ";\n}\n";
    (vs, fs)
}

fn budget(src: (String, String), limits: reflect::Es3VaryingLimits) -> Result<(), String> {
    let mut s1 = vec![0u8; 8 << 20];
    let mut s2 = vec![0u8; 8 << 20];
    let a = Arena::new(&mut s1);
    let b = Arena::new(&mut s2);
    let vi = reflect::interface(&a, Stage::Vertex, &src.0).unwrap_or_else(|e| panic!("vs: {}\n{}", e.msg, src.0));
    let fi = reflect::interface(&b, Stage::Fragment, &src.1).unwrap_or_else(|e| panic!("fs: {}\n{}", e.msg, src.1));
    reflect::varying_budget_es3(&vi, &fi, limits)
}

fn refused_by(r: Result<(), String>, limit: &str) {
    let e = r.expect_err("must be refused");
    assert!(e.contains(limit), "message {e:?} does not name {limit}");
}

#[test]
fn fifteen_vec4_at_the_minimums_links() {
    assert_eq!(budget(pair(15, false, 15, false), MIN), Ok(()));
}

#[test]
fn vertex_output_64_links_and_65_is_refused() {
    // 16 vec4 written, the fragment shader reads 15: 64 outputs, 60 inputs.
    assert_eq!(budget(pair(16, false, 15, false), MIN), Ok(()));
    // + one float written, not read.
    refused_by(budget(pair(16, true, 15, false), MIN), "MAX_VERTEX_OUTPUT_COMPONENTS = 64");
}

#[test]
fn fragment_input_60_links_and_61_is_refused() {
    refused_by(budget(pair(15, true, 15, true), MIN), "MAX_FRAGMENT_INPUT_COMPONENTS = 60");
}

#[test]
fn varying_components_are_the_matched_ones() {
    // Fragment inputs and vertex outputs are within their own limits (64),
    // the shared 64 are past MAX_VARYING_COMPONENTS = 60.
    let l = reflect::Es3VaryingLimits { max_fragment_input_components: 64, ..MIN };
    refused_by(budget(pair(16, false, 16, false), l), "MAX_VARYING_COMPONENTS = 60");
    let l = reflect::Es3VaryingLimits { max_varying_components: 64, ..l };
    refused_by(budget(pair(16, false, 16, false), l), "MAX_VARYING_VECTORS = 15");
    let l = reflect::Es3VaryingLimits { max_varying_vectors: 16, ..l };
    assert_eq!(budget(pair(16, false, 16, false), l), Ok(()));
}

#[test]
fn es1_pair_is_left_to_varying_budget() {
    let vs = "attribute vec4 p;\nvarying vec4 a;\nvoid main() { gl_Position = p; a = p; }\n";
    let fs = "precision mediump float;\nvarying vec4 a;\nvoid main() { gl_FragColor = a; }\n";
    let zero = reflect::Es3VaryingLimits {
        max_varying_vectors: 0,
        max_varying_components: 0,
        max_fragment_input_components: 0,
        max_vertex_output_components: 0,
    };
    assert_eq!(budget((vs.into(), fs.into()), zero), Ok(()));
}
