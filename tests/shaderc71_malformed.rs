// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Malformed and edge-case GLSL ES 1.00 through the V3D 7.1 online back end
//! gives `Err`, never a panic (the fuzz71 study's finding, pinned).
//!
//! A table of inline vertex/fragment pairs: syntax errors, type errors, too
//! many varyings, uniforms and attributes, deep nesting, huge loops, an empty
//! or missing `main`, unsupported extensions and 0-length arrays. Each pair
//! goes through the front end (`lower::compile_to_ir`) and, if both stages
//! lower, `chain71::compile_program71`, under `std::panic::catch_unwind`.
//! The outcome is one of `FrontErr` (the front end refused), `LinkErr`
//! (chain71 refused) or `Ok` (compiled). A panic fails the test; so does any
//! outcome other than the one the table expects, and the three counts are
//! pinned. The front end is lenient about a missing default float precision,
//! `#version 300 es` and a loop that never ends (all `Ok`, pinned as such);
//! the varying and attribute limits are chain71's (`LinkErr`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_malformed
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, TARGET71};
use yakogl::shaderc::online::{lower, store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Out {
    /// the front end refused a stage
    FrontErr,
    /// both stages lowered, chain71 refused
    LinkErr,
    /// compiled
    Ok,
}
use Out::{FrontErr as F, LinkErr as L, Ok as K};

/// Pins: the outcomes over the table.
const FRONT_ERRS: usize = 27;
const LINK_ERRS: usize = 3;
const OKS: usize = 6;

const VS: &str = "attribute vec4 p; void main() { gl_Position = p; }";
const FS: &str = "precision mediump float; void main() { gl_FragColor = vec4(1.0); }";

/// Compile `vs` + `fs` at 7.1.
fn run(vs: &str, fs: &str) -> Out {
    /* the scratch region is one per process: the tests take turns */
    static SCRATCH: Mutex<()> = Mutex::new(());
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs), (Stage::Fragment, fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(_) => return Out::FrontErr,
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        match compile_program71(v, f, Prim::Triangles, TARGET71, a) {
            Ok(_) => Out::Ok,
            Err(_) => Out::LinkErr,
        }
    })
    .expect("scratch")
}

fn nested(n: usize) -> String {
    let mut s = String::from("precision mediump float; uniform float u; void main() { float x = 0.0; ");
    for _ in 0..n {
        s.push_str("if (u > 0.5) { ");
    }
    s.push_str("x = 1.0;");
    for _ in 0..n {
        s.push_str(" }");
    }
    s.push_str(" gl_FragColor = vec4(x); }");
    s
}

fn parens(n: usize) -> String {
    format!(
        "precision mediump float; uniform float u; void main() {{ gl_FragColor = vec4({}u{}); }}",
        "(".repeat(n),
        ")".repeat(n)
    )
}

fn many(decl: &str, n: usize, vs_side: bool) -> (String, String) {
    let mut d = String::new();
    let mut u = String::new();
    for i in 0..n {
        d.push_str(&format!("{decl} t{i};\n"));
        u.push_str(&format!("+ t{i}.x"));
    }
    if vs_side {
        (format!("{d}attribute vec4 p; void main() {{ gl_Position = p {u}; }}"), FS.to_string())
    } else {
        (VS.to_string(), format!("precision mediump float;\n{d} void main() {{ gl_FragColor = vec4(0.0 {u}); }}"))
    }
}

#[test]
fn malformed_gives_err_never_panic() {
    let mut cases: Vec<(String, String, String, Out)> = Vec::new();
    let mut add = |name: &str, vs: &str, fs: &str, want: Out| cases.push((name.into(), vs.into(), fs.into(), want));

    // sanity: valid
    add("valid", VS, FS, K);
    // syntax errors
    add("fs-missing-semicolon", VS, "precision mediump float; void main() { gl_FragColor = vec4(1.0) }", F);
    add("fs-unbalanced-brace", VS, "precision mediump float; void main() { gl_FragColor = vec4(1.0);", F);
    add("fs-extra-brace", VS, "precision mediump float; void main() { gl_FragColor = vec4(1.0); } }", F);
    add("vs-garbage", "@#$ %^&", FS, F);
    add("fs-empty-source", VS, "", F);
    add("vs-empty-source", "", FS, F);
    add("fs-unterminated-comment", VS, "precision mediump float; void main() { gl_FragColor = vec4(1.0); } /* open", F);
    add("fs-bad-number", VS, "precision mediump float; void main() { gl_FragColor = vec4(1.0.0); }", F);
    add("fs-nul-char", VS, "precision mediump float; void main() { gl_FragColor = vec4(1.0); }\0 x", F);
    // type errors
    add("fs-vec3-to-vec4", VS, "precision mediump float; void main() { gl_FragColor = vec3(1.0); }", F);
    add("fs-int-float-mix", VS, "precision mediump float; void main() { float x = 1; gl_FragColor = vec4(x); }", F);
    add("fs-undeclared", VS, "precision mediump float; void main() { gl_FragColor = vec4(nope); }", F);
    add("fs-bool-cond-float", VS, "precision mediump float; void main() { if (1.0) { gl_FragColor = vec4(1.0); } }", F);
    add(
        "fs-assign-const",
        VS,
        "precision mediump float; const float c = 1.0; void main() { c = 2.0; gl_FragColor = vec4(c); }",
        F,
    );
    add("fs-no-float-precision", VS, "void main() { gl_FragColor = vec4(1.0); }", K);
    // main
    add("fs-empty-main", VS, "precision mediump float; void main() { }", K);
    add("fs-missing-main", VS, "precision mediump float; uniform float u;", F);
    add("vs-missing-main", "attribute vec4 p;", FS, F);
    add("fs-main-returns-value", VS, "precision mediump float; float main() { return 1.0; }", F);
    // extensions and directives
    add(
        "fs-ext-unknown-require",
        VS,
        "#extension GL_FOO_bar : require\nprecision mediump float; void main() { gl_FragColor = vec4(1.0); }",
        F,
    );
    add("fs-ext-bad-behavior", VS, "#extension GL_OES_standard_derivatives : maybe\nprecision mediump float; void main() { gl_FragColor = vec4(1.0); }", F);
    add(
        "fs-version-300",
        VS,
        "#version 300 es\nprecision mediump float; out vec4 c; void main() { c = vec4(1.0); }",
        K,
    );
    add("fs-if-without-endif", VS, "#if 1\nprecision mediump float; void main() { gl_FragColor = vec4(1.0); }", F);
    // arrays
    add(
        "fs-array-len-0",
        VS,
        "precision mediump float; uniform float a[0]; void main() { gl_FragColor = vec4(a[0]); }",
        F,
    );
    add(
        "fs-array-len-neg",
        VS,
        "precision mediump float; uniform float a[-1]; void main() { gl_FragColor = vec4(1.0); }",
        F,
    );
    add(
        "fs-array-oob-const",
        VS,
        "precision mediump float; uniform float a[2]; void main() { gl_FragColor = vec4(a[5]); }",
        F,
    );
    // limits
    let (v, f) = many("varying vec4", 40, false);
    add("fs-40-varyings", &v, &f, L);
    let (v, f) = many("varying vec4", 40, true);
    add("vs-40-varyings", &v, &f, L);
    let (v, f) = many("uniform vec4", 5000, false);
    add("fs-5000-uniforms", &v, &f, F);
    let (v, f) = many("attribute vec4", 40, true);
    add("vs-40-attributes", &v, &f, L);
    // nesting and size
    let n = nested(2000);
    add("fs-nested-if-2000", VS, &n, F);
    let n = parens(5000);
    add("fs-parens-5000", VS, &n, F);
    add(
        "fs-huge-loop",
        VS,
        "precision mediump float; void main() { float x = 0.0; for (int i = 0; i < 100000; i++) { x += 1.0; } gl_FragColor = vec4(x); }",
        K,
    );
    add(
        "fs-infinite-loop",
        VS,
        "precision mediump float; void main() { float x = 0.0; for (int i = 0; i < 4; ) { x += 1.0; } gl_FragColor = vec4(x); }",
        K,
    );
    add(
        "fs-recursion",
        VS,
        "precision mediump float; float f(float x) { return f(x); } void main() { gl_FragColor = vec4(f(1.0)); }",
        F,
    );

    let mut counts = [0usize; 3];
    let mut bad = Vec::new();
    for (name, vs, fs, want) in &cases {
        let got = match catch_unwind(AssertUnwindSafe(|| run(vs, fs))) {
            Ok(o) => o,
            Err(_) => {
                bad.push(format!("{name}: PANIC"));
                continue;
            }
        };
        println!("{name}: {got:?}");
        counts[match got {
            Out::FrontErr => 0,
            Out::LinkErr => 1,
            Out::Ok => 2,
        }] += 1;
        if got != *want {
            bad.push(format!("{name}: got {got:?}, want {want:?}"));
        }
    }
    println!("counts front/link/ok = {counts:?}");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!(counts, [FRONT_ERRS, LINK_ERRS, OKS], "outcome counts");
}
