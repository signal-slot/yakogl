// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests for `tf.rs` (`16ac8641`, docs/es3-transform-feedback.md 7.1):
//! the transform feedback objects, indexed bindings, varying resolution at
//! link, the begin/end rules, the primitives-written query and rasterizer
//! discard. The shader-level cases compile GLSL ES 3.00 on the in-process
//! compiler, so they exist only in `shaderc-online` builds. Under
//! `report-es2` the one test is `es2_answers_are_unchanged`.
#![allow(clippy::undocumented_unsafe_blocks)]

#[cfg(any(feature = "report-es2", feature = "shaderc-online"))]
use super::consts::*;
#[cfg(any(feature = "report-es2", feature = "shaderc-online"))]
use super::es3::*;
#[cfg(any(feature = "report-es2", feature = "shaderc-online"))]
use super::types::*;
#[cfg(any(feature = "report-es2", feature = "shaderc-online"))]
use crate::testutil::TestGl;

#[cfg(all(not(feature = "report-es2"), feature = "shaderc-online"))]
mod online {
    use super::*;
    use crate::gl::buffer::*;
    use crate::gl::draw::*;
    use crate::gl::program::*;
    use crate::gl::shader::*;
    use crate::gl::state::*;
    use std::ffi::CString;
    use std::string::String;
    use std::vec::Vec;

    /// The in-process compiler for the length of a test.
    struct BackEnd(*const crate::shaderc::ShcBackend);
    impl Drop for BackEnd {
        fn drop(&mut self) {
            // SAFETY: restoring what was saved, under the suite's guard.
            unsafe { crate::shaderc::g_shaderc = self.0 };
        }
    }

    fn fresh() -> (TestGl, BackEnd) {
        let gl = TestGl::new(8, 8);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        (gl, be)
    }

    const VS: &str = "#version 300 es\nin vec4 a;\nout vec4 v;\nout float f;\nout mediump vec2 arr[3];\n\
        flat out highp ivec2 iv;\nout mat2 m;\nvoid main() {\n v = a; f = a.x; iv = ivec2(1);\n m = mat2(1.0);\n\
        for (int i = 0; i < 3; i++) arr[i] = a.xy;\n gl_Position = a; gl_PointSize = 2.0; }\n";
    const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let text = CString::new(text).unwrap();
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const core::ffi::c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    }

    /// A program of `vs` and the plain fragment shader; `varyings` are asked
    /// for before the link. Returns the name, the link status and the log.
    fn link_with(vs: &str, varyings: &[&str], mode: GLenum) -> (GLuint, bool, String) {
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            let names: Vec<CString> = varyings.iter().map(|n| CString::new(*n).unwrap()).collect();
            let ptrs: Vec<*const GLchar> = names.iter().map(|n| n.as_ptr()).collect();
            glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), mode);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            let mut log = [0u8; 512];
            let mut len: GLsizei = 0;
            glGetProgramInfoLog(p, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut GLchar);
            (p, ok == 1, String::from_utf8_lossy(&log[..len.max(0) as usize]).into_owned())
        }
    }

    fn buffer(size: usize) -> GLuint {
        let mut b = 0;
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_ARRAY_BUFFER, b);
            glBufferData(GL_ARRAY_BUFFER, size as GLsizeiptr, core::ptr::null(), GL_DYNAMIC_COPY);
        }
        b
    }

    fn gen_tf() -> GLuint {
        let mut t = 0;
        unsafe { glGenTransformFeedbacks(1, &mut t) };
        assert_ne!(t, 0);
        t
    }

    fn geti(pname: GLenum) -> GLint {
        let mut v = -1;
        unsafe { glGetIntegerv(pname, &mut v) };
        v
    }

    fn geti_i(pname: GLenum, i: GLuint) -> GLint {
        let mut v = -1;
        unsafe { glGetIntegeri_v(pname, i, &mut v) };
        v
    }

    /// A linked program with `varyings` (all linking is asserted) and the
    /// buffers bound for Begin: `bufs` buffers of 1 KiB at indices 0...
    fn ready(varyings: &[&str], mode: GLenum, bufs: u32) -> GLuint {
        let (p, ok, log) = link_with(VS, varyings, mode);
        assert!(ok, "{log}");
        for i in 0..bufs {
            let b = buffer(1024);
            unsafe { glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, i, b) };
        }
        unsafe { glUseProgram(p) };
        p
    }

    #[test]
    fn limits_are_the_es3_minimums() {
        let (gl, _be) = fresh();
        assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS), 64);
        assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS), 4);
        assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS), 4);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn objects_are_generated_bound_and_deleted() {
        let (gl, _be) = fresh();
        let (a, b) = (gen_tf(), gen_tf());
        unsafe {
            assert_eq!(glIsTransformFeedback(a), GL_FALSE_B, "a name is an object once bound");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 99);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "never generated");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK_BUFFER, a);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            assert_eq!((gl.error(), geti(GL_TRANSFORM_FEEDBACK_BINDING) as GLuint), (GL_NO_ERROR, a));
            assert_eq!(glIsTransformFeedback(a), GL_TRUE_B);
            glDeleteTransformFeedbacks(1, &a);
            assert_eq!(
                (gl.error(), geti(GL_TRANSFORM_FEEDBACK_BINDING)),
                (GL_NO_ERROR, 0),
                "deleting the bound one rebinds 0"
            );
            assert_eq!(glIsTransformFeedback(a), GL_FALSE_B);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "deleted");
            glDeleteTransformFeedbacks(-1, &b);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            let mut many = [0u32; MAX_FEEDBACKS_FOR_TEST];
            glGenTransformFeedbacks(many.len() as GLsizei, many.as_mut_ptr());
            assert_eq!(
                many.iter().filter(|n| **n != 0).count(),
                MAX_FEEDBACKS_FOR_TEST - 2,
                "slot 0 and `b` are taken"
            );
        }
    }

    const MAX_FEEDBACKS_FOR_TEST: usize = super::super::tf::MAX_FEEDBACKS;

    #[test]
    fn deleting_an_active_object_is_an_error_and_it_survives() {
        let (gl, _be) = fresh();
        let (a, b) = (gen_tf(), gen_tf());
        ready(&["v"], GL_INTERLEAVED_ATTRIBS, 1);
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, b);
            let buf = buffer(256);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buf);
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let ids = [a, b];
            glDeleteTransformFeedbacks(2, ids.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the bound object is active");
            assert_eq!(glIsTransformFeedback(b), GL_TRUE_B, "it survives");
            assert_eq!(geti(GL_TRANSFORM_FEEDBACK_ACTIVE), 1);
            glEndTransformFeedback();
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteTransformFeedbacks(2, ids.as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn indexed_bindings_are_per_object() {
        let (gl, _be) = fresh();
        let (a, b) = (gen_tf(), gen_tf());
        let (x, y) = (buffer(64), buffer(128));
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x);
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 3, y, 8, 16);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(geti(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING) as GLuint, y, "the generic binding follows");
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0) as GLuint, x);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_START, 3), 8);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_SIZE, 3), 16);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_SIZE, 0), 0, "Base records size 0");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, b);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0), 0, "another object, nothing bound");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 3) as GLuint, y);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 4, x);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "one past the points");
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x, 2, 16);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "offset not a multiple of 4");
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x, 4, 6);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "size not a multiple of 4");
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            let mut v = 0;
            glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 4, &mut v);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 1, 0);
            assert_eq!(gl.error(), GL_NO_ERROR, "zero unbinds");
        }
    }

    #[test]
    fn a_bound_range_cannot_change_while_active_or_paused() {
        let (gl, _be) = fresh();
        ready(&["v"], GL_INTERLEAVED_ATTRIBS, 1);
        let other = buffer(64);
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, other);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 9, other, 0, 4);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the active check comes before the index");
            glPauseTransformFeedback();
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, other);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "paused is still active");
            glEndTransformFeedback();
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, other);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn a_deleted_buffer_lives_on_while_another_object_holds_it() {
        let (gl, _be) = fresh();
        let a = gen_tf();
        let buf = buffer(256);
        let allocs = gl.live_allocs();
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buf);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glDeleteBuffers(1, &buf);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glIsBuffer(buf), GL_FALSE_B, "the name is gone");
            assert_eq!(gl.live_allocs(), allocs, "the storage is kept for the object");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0) as GLuint, buf, "it still answers");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glDeleteTransformFeedbacks(1, &a);
            assert_eq!(gl.live_allocs(), allocs - 1, "the last holder let go: freed");
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn deleting_a_buffer_unbinds_it_from_the_current_object() {
        let (gl, _be) = fresh();
        let buf = buffer(256);
        let allocs = gl.live_allocs();
        unsafe {
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 1, buf);
            glDeleteBuffers(1, &buf);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 1), 0);
            assert_eq!(geti(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING), 0);
            assert_eq!(gl.live_allocs(), allocs - 1);
        }
    }

    /// (name, GL type, size) of the linked program's varying `i`.
    fn varying(p: GLuint, i: GLuint) -> (String, GLenum, GLsizei) {
        let mut name = [0 as GLchar; 64];
        let (mut len, mut size, mut ty) = (0, 0, 0);
        unsafe { glGetTransformFeedbackVarying(p, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr()) };
        let s: String = name[..len as usize].iter().map(|c| *c as u8 as char).collect();
        (s, ty, size)
    }

    #[test]
    fn varyings_resolve_against_the_vertex_outputs() {
        let (gl, _be) = fresh();
        let (p, ok, log) = link_with(
            VS,
            &["v", "arr", "arr[1]", "gl_Position", "gl_PointSize", "iv", "m", "f"],
            GL_INTERLEAVED_ATTRIBS,
        );
        assert!(ok, "{log}");
        let want = [
            ("v", GL_FLOAT_VEC4, 1),
            ("arr", GL_FLOAT_VEC2, 3),
            ("arr[1]", GL_FLOAT_VEC2, 1),
            ("gl_Position", GL_FLOAT_VEC4, 1),
            ("gl_PointSize", GL_FLOAT, 1),
            ("iv", GL_INT_VEC2, 1),
            ("m", GL_FLOAT_MAT2, 1),
            ("f", GL_FLOAT, 1),
        ];
        for (i, (n, t, s)) in want.iter().enumerate() {
            assert_eq!(varying(p, i as GLuint), (n.to_string(), *t, *s), "row {i}");
        }
        let mut v = -1;
        unsafe {
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_VARYINGS, &mut v);
            assert_eq!(v, 8);
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_VARYING_MAX_LENGTH, &mut v);
            assert_eq!(v, "gl_PointSize".len() as GLint + 1);
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_BUFFER_MODE, &mut v);
            assert_eq!(v as GLenum, GL_INTERLEAVED_ATTRIBS);
            // bufSize truncates, the call after the last row errs
            let mut name = [0x55 as GLchar; 8];
            let mut len = 0;
            glGetTransformFeedbackVarying(
                p,
                4,
                5,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr(),
            );
            assert_eq!((len, name[4]), (4, 0));
            glGetTransformFeedbackVarying(
                p,
                8,
                8,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr(),
            );
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetTransformFeedbackVarying(
                p,
                0,
                -1,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr(),
            );
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn link_errors_follow_es3_2_11_8() {
        let (_gl, _be) = fresh();
        let cases: &[(&[&str], GLenum, &str)] = &[
            (&["nosuch"], GL_INTERLEAVED_ATTRIBS, "Transform feedback varying nosuch undeclared."),
            (&["V"], GL_INTERLEAVED_ATTRIBS, "Transform feedback varying V undeclared."),
            (&["arr[3]"], GL_INTERLEAVED_ATTRIBS, "arr[3]"),
            (&["v", "v"], GL_INTERLEAVED_ATTRIBS, "Transform feedback varying v specified more than once."),
            (&["arr[0]", "arr[0]"], GL_SEPARATE_ATTRIBS, "specified more than once"),
            (&["arr"], GL_SEPARATE_ATTRIBS, "arr exceeds MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS"),
        ];
        for (names, mode, needle) in cases {
            let (_, ok, log) = link_with(VS, names, *mode);
            assert!(!ok, "{names:?} linked");
            assert!(log.contains(needle), "{names:?}: {log:?} lacks {needle:?}");
        }
        /* separate: elements apart, each within 4 components; the same name again after is fine */
        let (_, ok, log) = link_with(VS, &["arr[0]", "arr[1]", "arr[2]", "v"], GL_SEPARATE_ATTRIBS);
        assert!(ok, "{log}");
        #[cfg(not(feature = "gpu-vivante"))] // the GC7000 compiler refuses 16 vec4 varyings
        interleaved_limit();
    }

    #[cfg(not(feature = "gpu-vivante"))]
    fn interleaved_limit() {
        /* interleaved over 64 components: 17 vec4s are 68, but only the sum counts */
        let mut big = String::from("#version 300 es\nin vec4 a;\n");
        for i in 0..16 {
            big.push_str(&format!("out vec4 o{i};\n"));
        }
        big.push_str("void main() {\n");
        for i in 0..16 {
            big.push_str(&format!(" o{i} = a;\n"));
        }
        big.push_str(" gl_Position = a; }\n");
        let all: Vec<String> = (0..16).map(|i| format!("o{i}")).collect();
        let mut all: Vec<&str> = all.iter().map(String::as_str).collect();
        let (_, ok, log) = link_with(&big, &all, GL_INTERLEAVED_ATTRIBS);
        assert!(ok, "64 components fit: {log}");
        all.push("gl_PointSize");
        let (_, ok, log) = link_with(&big, &all, GL_INTERLEAVED_ATTRIBS);
        assert!(!ok);
        assert!(log.contains("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS limit has been exceeded"), "{log}");
    }

    #[test]
    fn varyings_apply_at_the_next_link_and_a_failed_link_keeps_none() {
        let (gl, _be) = fresh();
        let (p, ok, _) = link_with(VS, &["v"], GL_INTERLEAVED_ATTRIBS);
        assert!(ok);
        unsafe {
            let n = CString::new("f").unwrap();
            let names = [n.as_ptr()];
            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(varying(p, 0).0, "v", "recorded now, applied at the next link");
            glLinkProgram(p);
            assert_eq!(varying(p, 0).0, "f");
            let bad = CString::new("nope").unwrap();
            let names = [bad.as_ptr()];
            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_SEPARATE_ATTRIBS);
            glLinkProgram(p);
            let mut v = -1;
            glGetProgramiv(p, GL_LINK_STATUS, &mut v);
            assert_eq!(v, 0);
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_VARYINGS, &mut v);
            assert_eq!(v, 0, "a failed link keeps none");
            glTransformFeedbackVaryings(p, 0, core::ptr::null(), GL_INTERLEAVED_ATTRIBS);
            glLinkProgram(p);
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_VARYINGS, &mut v);
            assert_eq!(v, 0);
            // the call's own errors
            glTransformFeedbackVaryings(p, -1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_POINTS);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            let five = [n.as_ptr(); 5];
            glTransformFeedbackVaryings(p, 5, five.as_ptr(), GL_SEPARATE_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "more than MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS");
            glTransformFeedbackVaryings(p, 1, core::ptr::null(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTransformFeedbackVaryings(p + 100, 0, core::ptr::null(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTransformFeedbackVaryings(shader(GL_VERTEX_SHADER, VS), 0, core::ptr::null(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a shader is not a program");
        }
    }

    #[test]
    fn begin_end_pause_resume_rules() {
        let (gl, _be) = fresh();
        let (p0, ok, _) = link_with(VS, &[], GL_INTERLEAVED_ATTRIBS);
        assert!(ok);
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no program");
            glUseProgram(p0);
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no varyings");
            glEndTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not active");
            glPauseTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glResumeTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        let p = ready(&["v", "f"], GL_SEPARATE_ATTRIBS, 1);
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "separate needs a buffer for each varying");
            glBeginTransformFeedback(GL_LINE_STRIP);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 1, buffer(64));
            glBeginTransformFeedback(GL_TRIANGLES);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!((geti(GL_TRANSFORM_FEEDBACK_ACTIVE), geti(GL_TRANSFORM_FEEDBACK_PAUSED)), (1, 0));
            glBeginTransformFeedback(GL_TRIANGLES);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "already active");
            glResumeTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not paused");
            glPauseTransformFeedback();
            assert_eq!((gl.error(), geti(GL_TRANSFORM_FEEDBACK_PAUSED)), (GL_NO_ERROR, 1));
            glPauseTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "already paused");
            glResumeTransformFeedback();
            assert_eq!((gl.error(), geti(GL_TRANSFORM_FEEDBACK_PAUSED)), (GL_NO_ERROR, 0));
            glEndTransformFeedback();
            assert_eq!((gl.error(), geti(GL_TRANSFORM_FEEDBACK_ACTIVE)), (GL_NO_ERROR, 0));
            // resume needs the program Begin saw
            glBeginTransformFeedback(GL_POINTS);
            glPauseTransformFeedback();
            glUseProgram(p0);
            assert_eq!(gl.error(), GL_NO_ERROR, "a paused capture may change program");
            glResumeTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "another program is current");
            glUseProgram(p);
            glResumeTransformFeedback();
            assert_eq!(gl.error(), GL_NO_ERROR);
            glEndTransformFeedback();
        }
    }

    #[test]
    fn use_program_link_and_bind_are_locked_while_capturing() {
        let (gl, _be) = fresh();
        let p = ready(&["v"], GL_INTERLEAVED_ATTRIBS, 1);
        let (q, ok, _) = link_with(VS, &[], GL_INTERLEAVED_ATTRIBS);
        assert!(ok);
        let t = gen_tf();
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            glUseProgram(q);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "UseProgram while capturing");
            glLinkProgram(p);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "LinkProgram of the program in use");
            glLinkProgram(q);
            assert_eq!(gl.error(), GL_NO_ERROR, "another program links");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, t);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "BindTransformFeedback while capturing");
            let n = CString::new("v").unwrap();
            let names = [n.as_ptr()];
            glTransformFeedbackVaryings(q, 1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "varyings while the object is active");
            glPauseTransformFeedback();
            glUseProgram(q);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, t);
            assert_eq!(gl.error(), GL_NO_ERROR, "paused: bind allowed");
            glLinkProgram(p);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a paused object still uses its program (any object)");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glUseProgram(p);
            glResumeTransformFeedback();
            glEndTransformFeedback();
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn draws_while_capturing_are_refused_and_the_rules_come_first() {
        let (gl, _be) = fresh();
        ready(&["v"], GL_INTERLEAVED_ATTRIBS, 1);
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "mode does not match POINTS");
            glDrawElements(GL_POINTS, 1, GL_UNSIGNED_SHORT, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "indexed");
            glDrawArrays(GL_POINTS, 0, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "argument errors first");
            glDrawArrays(GL_POINTS, 0, 1000);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "does not fit the 1 KiB buffer");
            assert!(gl.logs().iter().any(|l| l.contains("too small")), "{:?}", gl.logs());
            glDrawArrays(GL_POINTS, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "capture is not built yet");
            assert!(gl.logs().iter().any(|l| l.contains("tf-capture")), "named in the log");
            glDrawArraysInstanced(GL_POINTS, 0, 3, 2);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert!(gl.draws().is_empty(), "nothing reached the back end");
            glPauseTransformFeedback();
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_NO_ERROR, "paused: an ordinary draw");
            glEndTransformFeedback();
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(!gl.draws().is_empty());
        }
    }

    #[test]
    fn captured_vertex_counts_follow_the_draw_mode() {
        use super::super::tf::captured_vertices as cv;
        assert_eq!(cv(GL_POINTS, 5), 5);
        assert_eq!((cv(GL_LINES, 5), cv(GL_LINE_STRIP, 5), cv(GL_LINE_LOOP, 5), cv(GL_LINE_LOOP, 1)), (4, 8, 10, 0));
        assert_eq!(
            (cv(GL_TRIANGLES, 7), cv(GL_TRIANGLE_STRIP, 5), cv(GL_TRIANGLE_FAN, 5), cv(GL_TRIANGLE_FAN, 2)),
            (6, 9, 9, 0)
        );
    }

    #[test]
    fn primitives_written_queries_are_objects_with_one_active_per_target() {
        let (gl, _be) = fresh();
        let mut q = [0u32; 2];
        let mut v = -1;
        unsafe {
            glGenQueries(2, q.as_mut_ptr());
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, 77);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not generated");
            glBeginQuery(0x8c87, q[0]);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "PRIMITIVES_GENERATED is not ES 3.0");
            glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "none active");
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[0]);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glGetQueryiv(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, GL_CURRENT_QUERY, &mut v);
            assert_eq!(v as GLuint, q[0]);
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[1]);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "one active per target");
            let mut r = 99;
            glGetQueryObjectuiv(q[0], GL_QUERY_RESULT, &mut r);
            assert_eq!((gl.error(), r), (GL_INVALID_OPERATION, 99), "an active query has no result");
            glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glGetQueryiv(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, GL_CURRENT_QUERY, &mut v);
            assert_eq!(v, 0);
            glGetQueryObjectuiv(q[0], GL_QUERY_RESULT_AVAILABLE, &mut r);
            assert_eq!((gl.error(), r), (GL_NO_ERROR, GL_TRUE), "available at once");
            glGetQueryObjectuiv(q[0], GL_QUERY_RESULT, &mut r);
            assert_eq!((gl.error(), r), (GL_NO_ERROR, 0), "nothing was captured");
            glGetQueryObjectuiv(q[0], GL_CURRENT_QUERY, &mut r);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            super::super::es3::add_primitives_written(gl.ctx(), 5); /* no active query: ignored */
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[0]);
            super::super::es3::add_primitives_written(gl.ctx(), 5);
            super::super::es3::add_primitives_written(gl.ctx(), 2);
            glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            glGetQueryObjectuiv(q[0], GL_QUERY_RESULT, &mut r);
            assert_eq!(r, 7, "a new Begin restarts the count, the captured draws add to it");
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q[1]);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "occlusion counting is not built (HANDOFF occlusion-query)");
            glDeleteQueries(2, q.as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    fn pending_clear(gl: &TestGl) -> bool {
        has_pending_clear(gl.ctx())
    }

    #[test]
    fn rasterizer_discard_stops_draws_and_clears() {
        let (gl, _be) = fresh();
        let (p, ok, _) = link_with(VS, &[], GL_INTERLEAVED_ATTRIBS);
        assert!(ok);
        unsafe {
            glUseProgram(p);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(gl.draws().len(), 1, "without discard the draw is recorded");
            glEnable(GL_RASTERIZER_DISCARD);
            assert_eq!(glIsEnabled(GL_RASTERIZER_DISCARD), GL_TRUE_B);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null());
            glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null(), 2);
            glDrawRangeElements(GL_TRIANGLES, 0, 2, 3, GL_UNSIGNED_SHORT, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(gl.draws().is_empty(), "no back-end draw under discard");
            glDrawArrays(GL_POINTS + 50, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "argument errors still fire");
            glDrawArrays(GL_TRIANGLES, 0, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            // clears
            for bit in [GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT] {
                glClear(bit);
                assert!(!pending_clear(&gl), "glClear(0x{bit:x}) under discard");
            }
            glClear(0x1);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "an unknown bit is still an error");
            let c = [0.5f32; 4];
            glClearBufferfv(GL_COLOR, 0, c.as_ptr());
            glClearBufferfv(GL_DEPTH, 0, c.as_ptr());
            glClearBufferfi(GL_DEPTH_STENCIL, 0, 1.0, 0);
            let iv = [0i32; 4];
            glClearBufferiv(GL_STENCIL, 0, iv.as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(!pending_clear(&gl), "glClearBuffer* under discard");
            glClearBufferfv(GL_COLOR, 9, c.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "their argument errors still fire");
            glDisable(GL_RASTERIZER_DISCARD);
            glClear(GL_COLOR_BUFFER_BIT);
            assert!(pending_clear(&gl), "without discard the clear is recorded");
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert!(!gl.draws().is_empty());
        }
    }

    #[test]
    fn discard_does_not_stop_a_capturing_draw_from_being_checked() {
        let (gl, _be) = fresh();
        ready(&["v"], GL_INTERLEAVED_ATTRIBS, 1);
        unsafe {
            glEnable(GL_RASTERIZER_DISCARD);
            glBeginTransformFeedback(GL_POINTS);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the mode rule applies under discard");
            glEndTransformFeedback();
        }
    }
}

/// The ES 2.0 report keeps its answers (the new code is never reached).
#[cfg(feature = "report-es2")]
#[test]
fn es2_answers_are_unchanged() {
    use crate::gl::buffer::*;
    use crate::gl::state::*;
    let gl = TestGl::new(8, 8);
    let geti = |pname| {
        let mut v = -1;
        unsafe { glGetIntegerv(pname, &mut v) };
        v
    };
    assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS), 0);
    assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS), 0);
    assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS), 0);
    assert_eq!((geti(GL_TRANSFORM_FEEDBACK_ACTIVE), geti(GL_TRANSFORM_FEEDBACK_PAUSED)), (0, 0));
    let mut b = 0;
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 64, core::ptr::null(), GL_DYNAMIC_COPY);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
        assert_eq!(gl.error(), GL_NO_ERROR, "binding zero is an unbind");
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        glBeginTransformFeedback(GL_POINTS);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        glBeginTransformFeedback(GL_LINE_STRIP);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        glEndTransformFeedback();
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        let mut q = 0;
        glGenQueries(1, &mut q);
        glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        let mut v = -1;
        glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &mut v);
        assert_eq!((gl.error(), v), (GL_NO_ERROR, 0));
        glEnable(GL_RASTERIZER_DISCARD);
        assert_eq!(glIsEnabled(GL_RASTERIZER_DISCARD), GL_TRUE_B, "recorded, as before");
    }
}
