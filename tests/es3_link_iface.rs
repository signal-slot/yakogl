// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 stage-interface link rules (sections 4.3.7 and 4.3.9): the
//! program link refuses a varying whose interpolation qualifier differs
//! between the vertex and fragment shaders, and a uniform block of one name
//! declared differently in the two. The sources are rebuilt from the shape of
//! dEQP-GLES3 `linkage.varying.rules.interpolation_mismatch_1`,
//! `linkage.uniform.block.members_mismatch` and
//! `layout_qualifier_mismatch_1..5`, plus controls that must still link.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_link_iface
//! ```
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

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

/// Both stages compile; the link's result: `Ok` or the info log.
fn link(vs: &str, fs: &str) -> Result<(), String> {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).unwrap_or_else(|m| panic!("vertex shader does not compile: {m}"));
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).unwrap_or_else(|m| panic!("fragment shader does not compile: {m}"));
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(text_of(&log))
    }
}

fn must_fail(vs: &str, fs: &str, needle: &str) {
    match link(vs, fs) {
        Ok(()) => panic!("wrongly accepted:\n{vs}\n{fs}"),
        Err(log) => {
            assert!(!log.is_empty(), "empty info log");
            assert!(log.contains(needle), "log {log:?} lacks {needle:?}");
        }
    }
}

fn must_link(vs: &str, fs: &str) {
    if let Err(log) = link(vs, fs) {
        panic!("refused: {log}\n{vs}\n{fs}");
    }
}

// ---- varyings --------------------------------------------------------------

fn varying_vs(q: &str) -> String {
    format!("#version 300 es\n{q} out mediump vec4 v;\nvoid main() {{ v = vec4(1.0); gl_Position = vec4(0.0); }}\n")
}

fn varying_fs(q: &str) -> String {
    format!(
        "#version 300 es\nprecision mediump float;\n{q} in mediump vec4 v;\nlayout(location = 0) out mediump vec4 o;\nvoid main() {{ o = v; }}\n"
    )
}

#[test]
fn interpolation_mismatch_1_flat_against_smooth() {
    must_fail(&varying_vs("flat"), &varying_fs(""), "interpolation");
    must_fail(&varying_vs(""), &varying_fs("flat"), "interpolation");
    must_fail(&varying_vs("flat"), &varying_fs("smooth"), "interpolation");
}

#[test]
fn matching_interpolation_links() {
    must_link(&varying_vs("flat"), &varying_fs("flat"));
    must_link(&varying_vs(""), &varying_fs(""));
    must_link(&varying_vs("smooth"), &varying_fs(""));
    must_link(&varying_vs(""), &varying_fs("smooth"));
}

// ---- uniform blocks --------------------------------------------------------

fn block_vs(decl: &str) -> String {
    format!("#version 300 es\n{decl}\nin vec4 p;\nvoid main() {{ gl_Position = p; }}\n")
}

fn block_fs(decl: &str) -> String {
    format!(
        "#version 300 es\nprecision mediump float;\n{decl}\nlayout(location = 0) out mediump vec4 o;\nvoid main() {{ o = vec4(1.0); }}\n"
    )
}

const BLOCK: &str = "layout(std140) uniform B { mediump vec4 a; highp float b; mediump mat3 m; };";

#[test]
fn identical_and_one_sided_blocks_link() {
    must_link(&block_vs(BLOCK), &block_fs(BLOCK));
    must_link(&block_vs(BLOCK), &block_fs(""));
    must_link(&block_vs(""), &block_fs(BLOCK));
    // the instance name is not part of the match
    must_link(
        &block_vs("layout(std140) uniform B { mediump vec4 a; } x;"),
        &block_fs("layout(std140) uniform B { mediump vec4 a; } y;"),
    );
    // a different block name is a different block
    must_link(
        &block_vs("layout(std140) uniform B { mediump vec4 a; };"),
        &block_fs("layout(std140) uniform C { mediump float z; };"),
    );
    // matching explicit layout at both levels
    let r = "layout(std140, row_major) uniform B { mediump mat3 m; layout(column_major) mediump mat2 n; };";
    must_link(&block_vs(r), &block_fs(r));
}

#[test]
fn members_mismatch() {
    let n = "uniform block 'B'";
    must_fail(&block_vs(BLOCK), &block_fs("layout(std140) uniform B { mediump vec4 a; highp float b; };"), n);
    must_fail(
        &block_vs(BLOCK),
        &block_fs("layout(std140) uniform B { mediump vec4 a; highp float c; mediump mat3 m; };"),
        n,
    );
    must_fail(
        &block_vs(BLOCK),
        &block_fs("layout(std140) uniform B { highp float b; mediump vec4 a; mediump mat3 m; };"),
        n,
    );
    must_fail(
        &block_vs(BLOCK),
        &block_fs("layout(std140) uniform B { mediump vec4 a; highp vec2 b; mediump mat3 m; };"),
        // the member is a default-block uniform too: the older type rule says it first
        "uniform 'b'",
    );
    must_fail(
        &block_vs(BLOCK),
        &block_fs("layout(std140) uniform B { lowp vec4 a; highp float b; mediump mat3 m; };"),
        n,
    );
    must_fail(
        &block_vs("layout(std140) uniform B { mediump vec4 a[2]; };"),
        &block_fs("layout(std140) uniform B { mediump vec4 a[3]; };"),
        "uniform 'a'",
    );
}

#[test]
fn layout_qualifier_mismatch() {
    let n = "uniform block 'B'";
    let b = |q: &str| format!("layout({q}) uniform B {{ mediump mat3 m; mediump vec4 a; }};");
    // 1: shared against packed, 2: std140 against shared, 3: std140 against packed
    must_fail(&block_vs(&b("shared")), &block_fs(&b("packed")), n);
    must_fail(&block_vs(&b("std140")), &block_fs(&b("shared")), n);
    must_fail(&block_vs(&b("std140")), &block_fs(&b("packed")), n);
    // 4: block-level row_major against column_major (the default)
    must_fail(&block_vs(&b("std140, row_major")), &block_fs(&b("std140")), n);
    must_fail(&block_vs(&b("std140, row_major")), &block_fs(&b("std140, column_major")), n);
    // 5: member-level row_major against none
    must_fail(
        &block_vs("layout(std140) uniform B { layout(row_major) mediump mat3 m; mediump vec4 a; };"),
        &block_fs("layout(std140) uniform B { mediump mat3 m; mediump vec4 a; };"),
        n,
    );
}
