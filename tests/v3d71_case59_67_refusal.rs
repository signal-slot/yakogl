// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! gl_test cases 59 (`case_clearmask`, driver/tools/pvrlab/gl_test.c:8178) and
//! 67 (`case_fbods`, :8727) on the 7.1 host path: each is refused by name.
//!
//! The GL calls of both cases are replayed through the GL core's entry points
//! with `yakogl_backend_v3d71` (src/v3d/table71.rs) as `g_backend`, on the
//! OSAL mock of tests/v3d71_table_rig/mod.rs, as tests/v3d71_table_cases.rs
//! does (this file keeps only the two cases and what they refuse with; it does
//! not compare jobs with Mesa's clifs). The refusal is the shared 7.1
//! read-lost refusal, `zskeep71`'s `ZsRefusal71::ReadLost(Plane71::Stencil)`,
//! whose message is the 4.2 table's string (src/v3d/backend.rs `fn record`):
//!
//! ```text
//! the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in
//! ```
//!
//! The table logs it as `yakogl/v3d71: draw refused: <reason>`
//! (`table71::REFUSAL_PREFIX`), counts it in `table71::refused_draws()`, and
//! the GL core turns it into `GL_INVALID_OPERATION` with the log line
//! `glDrawArrays was not recorded (-95)`. Per case:
//!
//! * 59: the three jobs before the last draw are submitted; the draw that
//!   tests the stencil the third job wrote is refused, and the case stops at
//!   its "stencil read-back" check;
//! * 67: frames 1 to 7 are eight jobs (the GL_RGB frame included, drawn, so no
//!   "fewer than 8 alpha bits" refusal); frame 8's stencil-testing draw, one
//!   `glFinish` after the stencil clear that wrote its box, is refused.
//!
//! The case-67 refusal "a colour format with fewer than 8 alpha bits" of the
//! Pi 4's first run is not the one given (tests/v3d71_refuse67.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_case59_67_refusal
//! ```
#![cfg(all(feature = "gpu-v3d71", feature = "host-test", feature = "shaderc-online"))]

mod v3d71_table_rig;

use std::ffi::c_void;

use v3d71_table_rig::{log_lines, logs_with, osal_counts, rig, Rig};
use yakogl::backend;
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context::{gl_context_create, gl_context_destroy, GlContext};
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::v3d::table71::{self, REFUSAL_PREFIX};

const TW: GLsizei = 64;
const TH: GLsizei = 64;

/// gl_test's ES 1.00 pair (`gl_test_vs_src`, `gl_test_fs_src`): a_pos 0,
/// a_color 1, a_uv 2; the colour times (u, v, 1, 1).
const COLOR_UV_VS: &str = concat!(include_str!("../tools/shaders/gl_test_es100/color_uv.vert"), "\0");
const COLOR_UV_FS: &str = concat!(include_str!("../tools/shaders/gl_test_es100/color_uv.frag"), "\0");

/// The 4.2 table's string for a read of stencil an earlier job wrote
/// (src/v3d/backend.rs `fn record`), which zskeep71 repeats.
const READS_STENCIL: &str =
    "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in";

/// `struct cuv_vertex`: pos4, colour4, uv2 (40 bytes).
type Vtx = [f32; 10];

/// `cuv_set`.
fn cuv(x: f32, y: f32, z: f32, r: f32, g: f32, b: f32) -> Vtx {
    [x, y, z, 1.0, r, g, b, 1.0, 1.0, 1.0]
}

const FULL: [[f32; 2]; 6] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// `solid_quad`: a full-screen quad of one colour at depth `z`.
fn solid_quad(r: f32, g: f32, b: f32, z: f32) -> Vec<Vtx> {
    FULL.iter().map(|c| cuv(c[0], c[1], z, r, g, b)).collect()
}

/// A case stops at its first failed `check_gl_error`, as gl_test's does
/// (`return -1`): the label of the check and the error.
type Case = Result<(), String>;
/// A case's function ([`gl`]).
type CaseFn = unsafe fn() -> Case;

/// `check_gl_error`.
fn check_gl_error(what: &str) -> Case {
    // SAFETY: a plain GL call.
    match unsafe { glGetError() } {
        GL_NO_ERROR => Ok(()),
        e => Err(format!("{what}: GL error {e:#06x}")),
    }
}

/// The GL calls of the cases; every one needs the current context of [`Gl`].
mod gl {
    use super::*;

    /// `link_program(gl_test_vs_src, gl_test_fs_src)`.
    pub unsafe fn link_program() -> GLuint {
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
        let vs = compile(GL_VERTEX_SHADER, COLOR_UV_VS);
        let fs = compile(GL_FRAGMENT_SHADER, COLOR_UV_FS);
        let prog = glCreateProgram();
        glAttachShader(prog, vs);
        glAttachShader(prog, fs);
        glLinkProgram(prog);
        let mut ok: GLint = 0;
        glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
        assert_ne!(ok, 0, "the program did not link: {:?}", log_lines());
        prog
    }

    /// `upload_vbo` and `cuv_attribs`.
    pub unsafe fn upload(v: &[Vtx]) {
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(v) as GLsizeiptr,
            v.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        for (i, (n, off)) in [(4, 0usize), (4, 16), (2, 32)].into_iter().enumerate() {
            glVertexAttribPointer(i as GLuint, n, GL_FLOAT, 0, 40, off as *const c_void);
            glEnableVertexAttribArray(i as GLuint);
        }
    }

    /// `read_topdown`: the case's glReadPixels (the pixels are not looked at:
    /// nothing renders on the host).
    pub unsafe fn read_topdown() {
        let mut px = vec![0u32; (TW * TH) as usize];
        glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }

    /// `reset_state` (gl_test.c:1472): what gl_test puts back before a case.
    pub unsafe fn reset_state() {
        while glGetError() != GL_NO_ERROR {}
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, TW, TH);
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_CULL_FACE);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_BLEND);
        glDepthMask(1);
        glDepthFunc(GL_LESS);
        glFrontFace(GL_CCW);
        glCullFace(GL_BACK);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glColorMask(1, 1, 1, 1);
        glStencilMask(0xff);
        glDisable(GL_STENCIL_TEST);
        glClearDepthf(1.0);
        glClearStencil(0);
        glUseProgram(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, 0);
        for a in 0..8 {
            glDisableVertexAttribArray(a);
        }
        glStencilFunc(GL_ALWAYS, 0, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
    }

    /// Case 59, `case_clearmask` (gl_test.c:8178).
    pub unsafe fn case_clearmask() -> Case {
        glUseProgram(link_program());
        /* colour: a clear with every channel masked off changes nothing */
        upload(&solid_quad(0.0, 1.0, 0.0, 0.0));
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glColorMask(0, 0, 0, 0);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glColorMask(1, 1, 1, 1);
        glFinish();
        check_gl_error("masked colour clear")?;
        read_topdown();
        /* depth: glDepthMask(GL_FALSE) then a depth clear changes nothing */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearDepthf(1.0);
        glDepthMask(1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        upload(&solid_quad(1.0, 0.0, 0.0, 0.0));
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glDepthMask(0);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glDepthMask(1);
        upload(&solid_quad(0.0, 1.0, 0.0, 0.5));
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        glDisable(GL_DEPTH_TEST);
        check_gl_error("masked depth clear")?;
        read_topdown();
        /* stencil: the write mask says which bits a clear may change */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glStencilMask(0xff);
        glClearStencil(0x00);
        glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glStencilMask(0x0f);
        glClearStencil(0xff);
        glClear(GL_STENCIL_BUFFER_BIT);
        glStencilMask(0xff);
        glEnable(GL_STENCIL_TEST);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        upload(&solid_quad(0.0, 1.0, 0.0, 0.0));
        glStencilFunc(GL_EQUAL, 0xff, 0xff);
        glDrawArrays(GL_TRIANGLES, 0, 6); /* must not draw */
        glFinish();
        check_gl_error("masked stencil clear")?;
        read_topdown();
        glStencilFunc(GL_EQUAL, 0x0f, 0xff);
        glDrawArrays(GL_TRIANGLES, 0, 6); /* must draw */
        glFinish();
        glDisable(GL_STENCIL_TEST);
        check_gl_error("stencil read-back")?;
        read_topdown();
        check_gl_error("clearmask")
    }

    /// `fbods_color_tex`.
    unsafe fn fbods_color_tex(format: GLenum) -> GLuint {
        let mut tex: GLuint = 0;
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, TW, TH, 0, format, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        tex
    }

    unsafe fn complete(what: &str) -> Case {
        match glCheckFramebufferStatus(GL_FRAMEBUFFER) {
            GL_FRAMEBUFFER_COMPLETE => Ok(()),
            s => Err(format!("{what} FBO status {s:#x}")),
        }
    }

    /// Case 67, `case_fbods` (gl_test.c:8727), the build without
    /// `GL_TEST_VIVANTE`: frames 1 to 8.
    pub unsafe fn case_fbods() -> Case {
        let (mut fbo_a, mut fbo_b, mut depth_a, mut depth_b): (GLuint, GLuint, GLuint, GLuint) = (0, 0, 0, 0);
        glUseProgram(link_program());
        /* 0..5: red at z = -0.5 (near). 6..11: green at z = +0.5 (far). */
        let mut v = solid_quad(1.0, 0.0, 0.0, -0.5);
        v.extend(solid_quad(0.0, 1.0, 0.0, 0.5));
        upload(&v);
        glViewport(0, 0, TW, TH);

        /* frame 1: colour only, and GL_DEPTH_TEST enabled */
        let tex = fbods_color_tex(GL_RGBA);
        glGenFramebuffers(1, &mut fbo_a);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        complete("colour-only")?;
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glDrawArrays(GL_TRIANGLES, 6, 6);
        glFinish();
        check_gl_error("no depth attachment")?;
        read_topdown();

        /* frame 2: the same framebuffer, and a stencil test */
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 1, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glClearStencil(0);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        check_gl_error("no stencil attachment")?;
        read_topdown();
        glDisable(GL_STENCIL_TEST);

        /* frame 3: a GL_RGB colour attachment */
        {
            let rgb_tex = fbods_color_tex(GL_RGB);
            let mut rgb_fbo: GLuint = 0;
            glGenFramebuffers(1, &mut rgb_fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, rgb_fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rgb_tex, 0);
            complete("RGB")?;
            glDisable(GL_DEPTH_TEST);
            glClearColor(1.0, 0.0, 0.0, 0.25);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
            check_gl_error("RGB clear")?;
            read_topdown();
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &rgb_fbo);
            glDeleteTextures(1, &rgb_tex);
        }

        /* frames 4 and 5: two framebuffer objects over one colour texture,
         * each with a depth buffer of its own */
        glGenRenderbuffers(1, &mut depth_a);
        glBindRenderbuffer(GL_RENDERBUFFER, depth_a);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);
        glGenRenderbuffers(1, &mut depth_b);
        glBindRenderbuffer(GL_RENDERBUFFER, depth_b);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_a);
        glGenFramebuffers(1, &mut fbo_b);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_b);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_b);
        complete("shared-colour")?;

        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        check_gl_error("shared colour, A")?;
        read_topdown();

        glBindFramebuffer(GL_FRAMEBUFFER, fbo_b);
        glDrawArrays(GL_TRIANGLES, 6, 6);
        glFinish();
        check_gl_error("shared colour, B")?;
        read_topdown();

        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo_b);
        glDeleteRenderbuffers(1, &depth_a);
        glDeleteRenderbuffers(1, &depth_b);
        glDisable(GL_DEPTH_TEST);

        /* frame 6: a scissored stencil-only clear */
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0);
        let mut sten: GLuint = 0;
        glGenRenderbuffers(1, &mut sten);
        glBindRenderbuffer(GL_RENDERBUFFER, sten);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, TW, TH);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, sten);
        complete("stencil")?;
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_SCISSOR_TEST);
        glScissor(16, 16, 32, 32);
        glClearStencil(1);
        glClear(GL_STENCIL_BUFFER_BIT);
        glDisable(GL_SCISSOR_TEST);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 0, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        glDisable(GL_STENCIL_TEST);
        check_gl_error("scissored stencil clear")?;
        read_topdown();

        /* frames 7 and 8: the same clear with a draw ahead of it, and the
         * stencil-testing draw in the same kick (7) or in the next one (8) */
        for step in 0..2 {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClearStencil(0);
            glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 6, 6);
            glEnable(GL_SCISSOR_TEST);
            glScissor(16, 16, 32, 32);
            glClearStencil(1);
            glClear(GL_STENCIL_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
            glClearStencil(0);
            if step == 1 {
                glFinish(); /* the box must survive a kick */
            }
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            glDisable(GL_STENCIL_TEST);
            check_gl_error(["stencil box, one kick", "stencil box across a kick"][step])?;
            read_topdown();
        }
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &sten);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo_a);
        glDeleteTextures(1, &tex);
        check_gl_error("fbods")
    }
}

struct Gl {
    ctx: *mut GlContext,
    _rig: Rig,
}

impl Gl {
    fn new() -> Gl {
        let rig = rig();
        assert_eq!(backend::init(), 0, "{:?}", log_lines());
        // SAFETY: the OSAL and the table are installed; the rig's lock makes
        // this the only context.
        let ctx = unsafe { gl_context_create(TW as u32, TH as u32) };
        assert!(!ctx.is_null());
        // SAFETY: the context is current.
        unsafe { gl::reset_state() };
        Gl { ctx, _rig: rig }
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        // SAFETY: the context `new` made; nothing holds a reference to it.
        unsafe { gl_context_destroy(self.ctx) };
    }
}

/// Run `case`: it stops at the check `stops_at` with `GL_INVALID_OPERATION`,
/// exactly one draw is refused, the log carries the one refusal line with
/// [`READS_STENCIL`] after the prefix, and `jobs` jobs were submitted.
fn refused(case: CaseFn, stops_at: &str, jobs: usize) {
    let _g = Gl::new();
    // SAFETY: the context is current.
    let r = unsafe { case() };
    assert_eq!(r, Err(format!("{stops_at}: GL error 0x0502")), "{:?}", log_lines());
    assert_eq!(table71::refused_draws(), 1, "{:?}", logs_with(REFUSAL_PREFIX));
    assert_eq!(logs_with(REFUSAL_PREFIX), [format!("{REFUSAL_PREFIX}{READS_STENCIL}")]);
    assert_eq!(logs_with("glDrawArrays was not recorded (-95)").len(), 1, "{:?}", log_lines());
    assert!(logs_with("alpha bits").is_empty(), "{:?}", log_lines());
    assert_eq!(osal_counts().0, jobs, "the jobs before the refused draw");
}

#[test]
fn case_59_is_refused_with_the_stencil_read_lost_reason() {
    refused(gl::case_clearmask, "stencil read-back", 3);
}

#[test]
fn case_67_is_refused_with_the_same_reason_at_frame_8() {
    refused(gl::case_fbods, "stencil box across a kick", 8);
}

#[test]
fn the_reason_is_zskeeps_read_lost_stencil_message() {
    let src = include_str!("../src/v3d/zskeep71.rs");
    assert!(src.contains("the draw reads stencil an earlier job wrote"), "zskeep71 changed its reason");
    assert!(include_str!("../src/v3d/backend.rs").contains(READS_STENCIL));
}
