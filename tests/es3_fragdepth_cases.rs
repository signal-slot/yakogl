// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The nine shapes of dEQP-GLES3 `shaders.fragdepth.{write,compare}.*`
//! (es3fFragDepthTests.cpp: no_write, const, uniform, dynamic, fragcoord_z,
//! uniform_conditional_write, dynamic_conditional_write, uniform_loop_write,
//! write_in_function) on the GC7000UL and V3D online compilers. Each is
//! compiled and linked through `SHC_ONLINE`, the path glCompileShader and
//! glLinkProgram reach, and the verdict is pinned: it builds, and the
//! program data says the depth is written (GC7000UL: `viv.ps_depth_out_reg`
//! is 0 and `writes_depth` 1; V3D: `writes_z` and `disable_ez` 1), or for
//! no_write says it is not (`SHC_VIV_NO_REG` and 0; 0 and 0).
//!
//! The simple writes have their own files (`es3_fragdepth_gc`,
//! `es3_fragdepth_v3d`); this one adds the conditional, loop and function
//! shapes and the no-write control on both back ends.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_fragdepth_cases
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_fragdepth_cases
//! ```
#![cfg(all(
    feature = "shaderc-online",
    any(feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "report-es2")
))]

use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const VS: &str = "#version 300 es\n\
in highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

fn fragment(body: &str) -> String {
    format!(
        "#version 300 es\nuniform highp vec4 u_color;\nuniform highp float uf_half;\nuniform bool ub_true;\n\
         uniform int ui_two;\nuniform highp float uf_fourth;\nin highp vec2 v_coord;\n\
         layout(location = 0) out mediump vec4 o_color;\n{body}\n"
    )
}

/// (shape, fragment body, writes depth). `.compare.*` shares these sources.
const SHAPES: [(&str, &str, bool); 9] = [
    ("no_write", "void main (void)\n{\n    o_color = u_color;\n}", false),
    ("const", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.5;\n}", true),
    ("uniform", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = uf_half;\n}", true),
    ("dynamic", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}", true),
    ("fragcoord_z", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = gl_FragCoord.z;\n}", true),
    (
        "uniform_conditional_write",
        "void main (void)\n{\n    o_color = u_color;\n    if (ub_true)\n        gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}",
        true,
    ),
    (
        "dynamic_conditional_write",
        "void main (void)\n{\n    o_color = u_color;\n    mediump float d = (v_coord.x+v_coord.y)*0.5f;\n    if (v_coord.y < 0.5)\n        gl_FragDepth = d;\n    else\n        gl_FragDepth = 1.0 - d;\n}",
        true,
    ),
    (
        "uniform_loop_write",
        "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.0;\n    for (int i = 0; i < ui_two; i++)\n        gl_FragDepth += uf_fourth;\n}",
        true,
    ),
    (
        "write_in_function",
        "void myfunc (highp vec2 coord)\n{\n    gl_FragDepth = (coord.x+coord.y)*0.5;\n}\nvoid main (void)\n{\n    o_color = u_color;\n    myfunc(v_coord);\n}",
        true,
    ),
];

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

/// glCompileShader on both stages, then glLinkProgram; the linked fragment
/// record, passed to `f` while the compiler's arena is held.
fn build<R>(fs: &str, f: impl FnOnce(&ShcShader) -> R) -> Result<R, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, VS).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(f(&fso))
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

#[cfg(feature = "gpu-vivante")]
mod gc7000ul {
    use super::*;
    use yakogl::shaderc::SHC_VIV_NO_REG;

    #[test]
    fn every_fragdepth_shape_builds_with_its_depth_register() {
        for (name, body, writes) in SHAPES {
            let fs = fragment(body);
            let (reg, flag) = build(&fs, |s| (s.viv.ps_depth_out_reg, s.writes_depth))
                .unwrap_or_else(|e| panic!("fragdepth.{name}: {e}\n{fs}"));
            if writes {
                assert_eq!((reg, flag), (0, 1), "fragdepth.{name}: (ps_depth_out_reg, writes_depth)");
            } else {
                assert_eq!((reg, flag), (SHC_VIV_NO_REG, 0), "fragdepth.{name}: (ps_depth_out_reg, writes_depth)");
            }
        }
    }
}

#[cfg(feature = "gpu-v3d")]
mod v3d {
    use super::*;

    #[test]
    fn every_fragdepth_shape_builds_with_writes_z_and_disable_ez() {
        for (name, body, writes) in SHAPES {
            let fs = fragment(body);
            let got = build(&fs, |s| (s.v3d.fs.writes_z, s.v3d.fs.disable_ez))
                .unwrap_or_else(|e| panic!("fragdepth.{name}: {e}\n{fs}"));
            let want = u8::from(writes);
            assert_eq!(got, (want, want), "fragdepth.{name}: (writes_z, disable_ez)");
        }
    }
}
