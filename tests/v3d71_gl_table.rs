// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Acceptance c3 of P9 (docs/notes/pre-study-rpi5.md section 8): the GL
//! core's public entry points through the hooked 7.1 table
//! (`yakogl_backend_v3d71`, src/v3d/table71.rs, with P10's texture path,
//! src/v3d/table71tex.rs, in the same build) for gl_test's six cases 5, 58,
//! 59, 66, 67 and 102, each job compared with Mesa's tools/v3d/pi5/gl_*.clif,
//! and every refusal, of the cases and of the shapes the table does not
//! draw, checked against the names docs/notes/v3d71-refusals.md lists.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_gl_table
//! ```
//!
//! The cases' GL calls and the comparison with Mesa are
//! tests/v3d71_table_cases.rs's (`recorded`, `refused`: each job decoded
//! with v3d_packet.xml at version 71 and compared field by field with the
//! reference's, every difference named); that file is included here as the
//! module `cases`, so its own tests run in this binary as well. The OSAL and
//! the C builders are tests/v3d71_table_rig/mod.rs's.
//!
//! What this file adds:
//!
//! * the six verdicts on the hooked table with the texture path in: 5, 58,
//!   66 and 102 recorded as Mesa's jobs with no texture copy made, and 59
//!   and 67 refused by the 4.2 table's stencil name (`cases`' tests), which
//!   the note lists (section 7);
//! * the unsupported shapes a GL program can reach, each refused with
//!   `GL_INVALID_OPERATION` and one log line `yakogl/v3d71: draw refused:
//!   <module>: <reason>`, whose reason the note lists in double quotes and
//!   whose module it names: a 3D texture and a 2D array texture
//!   (texglue71). An instanced draw is drawn since backend71 assembles
//!   instanced arrays (tests/v3d71_instanced.rs), and a vertex program
//!   writing `gl_PointSize` since it assembles point-size records
//!   (tests/v3d71_table_points.rs).
#![cfg(all(feature = "gpu-v3d71", feature = "host-test", feature = "shaderc-online"))]

#[path = "v3d71_table_cases.rs"]
mod cases;

use std::ffi::c_void;

use cases::v3d71_table_rig::{log_lines, logs_with};
use cases::{gl, recorded, refusals, solid_quad, Gl, READS_STENCIL};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::v3d::table71::{self, REFUSAL_PREFIX};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn note() -> String {
    let p = format!("{ROOT}/docs/notes/v3d71-refusals.md");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// `why` (a refusal line without the prefix) is listed: the reason in
/// double quotes, and the module it names, in the note.
fn listed(why: &str) {
    let note = note();
    let (module, reason) = match why.split_once(": ") {
        Some((m, r)) if !m.contains(' ') => (Some(m), r),
        _ => (None, why),
    };
    assert!(note.contains(&format!("\"{reason}\"")), "docs/notes/v3d71-refusals.md does not list \"{reason}\"");
    if let Some(m) = module {
        assert!(note.contains(m), "docs/notes/v3d71-refusals.md does not name the module {m}");
    }
}

#[test]
fn the_six_cases_pass_as_mesas_jobs_or_name_a_listed_refusal() {
    for (shape, case, jobs) in [
        ("scissor", gl::case_scissor as cases::CaseFn, 2),
        ("scisbox", gl::case_clearscis, 3),
        ("viewport", gl::case_clearvp, 3),
        ("colormask", gl::case_maskclear, 6),
    ] {
        let (g, ks, _) = recorded(shape, case, jobs);
        assert_eq!(ks.len(), jobs);
        assert!(table71::texture_copies().is_empty(), "{shape}: no texture is sampled");
        drop(g);
    }
    /* 59 and 67 are refused by READS_STENCIL: `cases`' own tests
     * (case_59_..., case_67_..., the board's order) run them here, with
     * `refused`'s checks. Run again they would spend the GL core's budget
     * of "was not recorded" lines (src/gl/draw.rs DRAW_FAIL_LOGS, 8 a
     * process), which those tests and the shapes below use up exactly:
     * 1 + 1 + 2 + 4. The name is the note's. */
    listed(READS_STENCIL);
}

const VS3: &str = "#version 300 es\nin vec4 a_pos;\nout vec2 v_uv;\n\
                   void main() { gl_Position = a_pos; v_uv = a_pos.xy; }\n\0";
const FS_3D: &str = "#version 300 es\nprecision mediump float;\nprecision mediump sampler3D;\n\
                     uniform sampler3D u_a;\nin vec2 v_uv;\nout vec4 o;\n\
                     void main() { o = texture(u_a, vec3(v_uv, 0.5)); }\n\0";
const FS_ARRAY: &str = "#version 300 es\nprecision mediump float;\nprecision mediump sampler2DArray;\n\
                        uniform sampler2DArray u_a;\nin vec2 v_uv;\nout vec4 o;\n\
                        void main() { o = texture(u_a, vec3(v_uv, 1.0)); }\n\0";

unsafe fn program(vs: &str, fs: &str) {
    let compile = |ty: GLenum, src: &str| {
        let s = glCreateShader(ty);
        let p = src.as_ptr() as *const GLchar;
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_ne!(ok, 0, "shader {ty:#x} did not compile: {:?}", log_lines());
        s
    };
    let prog = glCreateProgram();
    glAttachShader(prog, compile(GL_VERTEX_SHADER, vs));
    glAttachShader(prog, compile(GL_FRAGMENT_SHADER, fs));
    glBindAttribLocation(prog, 0, c"a_pos".as_ptr());
    glLinkProgram(prog);
    let mut ok: GLint = 0;
    glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
    assert_ne!(ok, 0, "the program did not link: {:?}", log_lines());
    glUseProgram(prog);
}

/// A 4x4x4 RGBA8 texture of `target` (GL_TEXTURE_3D or _2D_ARRAY).
unsafe fn layered(target: GLenum) {
    let px = vec![0u8; 4 * 4 * 4 * 4];
    let mut t: GLuint = 0;
    glGenTextures(1, &mut t);
    glBindTexture(target, t);
    glTexImage3D(target, 0, GL_RGBA8 as GLint, 4, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
    glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
}

type Shape = unsafe fn();

unsafe fn texture_3d() {
    program(VS3, FS_3D);
    layered(GL_TEXTURE_3D);
    glDrawArrays(GL_TRIANGLES, 0, 6);
}

unsafe fn texture_array() {
    program(VS3, FS_ARRAY);
    layered(GL_TEXTURE_2D_ARRAY);
    glDrawArrays(GL_TRIANGLES, 0, 6);
}

#[test]
fn unsupported_shapes_name_a_listed_refusal() {
    let shapes: [(&str, Shape, &str); 2] = [
        ("3D texture", texture_3d, "texglue71: texture glue 7.1: a 3D texture is not laid out by texture71"),
        ("2D array", texture_array, "texglue71: texture glue 7.1: a 2D array texture is not laid out by texture71"),
    ];
    let _g = Gl::new();
    for (n, (name, shape, why)) in shapes.iter().enumerate() {
        // SAFETY: the context is current.
        let e = unsafe {
            gl::reset_state();
            gl::upload(&solid_quad(1.0, 0.0, 0.0, 0.0));
            shape();
            let e = glGetError();
            glFinish();
            e
        };
        assert_eq!(e, GL_INVALID_OPERATION, "{name}: {:?}", log_lines());
        assert_eq!(table71::refused_draws(), n as u32 + 1, "{name}: {:?}", refusals());
        assert_eq!(logs_with(&format!("{REFUSAL_PREFIX}{why}")).len(), 1, "{name}: {:?}", refusals());
        listed(why);
    }
    assert_eq!(refusals().len(), shapes.len(), "one line per cause: {:?}", refusals());
}
