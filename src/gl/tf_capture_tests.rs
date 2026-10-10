// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests for `tf_capture.rs` (docs/es3-transform-feedback.md 7.2,
//! tf-capture): the bytes a captured draw writes, against values computed
//! here from the inputs, in every draw mode, both buffer modes, with
//! instancing, pause/resume, ranges and guards, overflow, rasterizer
//! discard, a buffer a pending kick still reads, a refused shader, and a
//! captured buffer drawn from again. Each case counts
//! TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN. GLSL ES 3.00 on the in-process
//! compiler, so `shaderc-online` builds only, and never under `report-es2`
//! (tf_tests.rs `es2_answers_are_unchanged` holds that build).
#![allow(clippy::undocumented_unsafe_blocks)]

use super::buffer::*;
use super::consts::*;
use super::draw::*;
use super::es3::*;
use super::program::*;
use super::shader::*;
use super::state::*;
use super::types::*;
use super::vertex::*;
use crate::testutil::TestGl;
use std::ffi::CString;
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
    let be = BackEnd(unsafe { crate::shaderc::g_shaderc });
    unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
    (gl, be)
}

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

/// `vs` with the plain fragment shader, `varyings` captured in `mode`;
/// linked (asserted) and current.
fn program(vs: &str, fs: &str, varyings: &[&str], mode: GLenum) -> GLuint {
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        let names: Vec<CString> = varyings.iter().map(|n| CString::new(*n).unwrap()).collect();
        let ptrs: Vec<*const GLchar> = names.iter().map(|n| n.as_ptr()).collect();
        glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), mode);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{vs}: link failed");
        glUseProgram(p);
        p
    }
}

/// A buffer of `size` bytes filled with the guard byte 0xcd, bound at
/// GL_TRANSFORM_FEEDBACK_BUFFER index `index` (the whole buffer).
fn tf_buffer(index: GLuint, size: usize) -> GLuint {
    let mut b = 0;
    let fill = std::vec![0xcdu8; size];
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, b);
        glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, size as GLsizeiptr, fill.as_ptr().cast(), GL_DYNAMIC_READ);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, index, b);
    }
    b
}

/// What the GPU sees of buffer `name` (its current storage): `len` bytes.
/// The capture's writes must have been flushed for the GPU to see them.
fn gpu_view(gl: &TestGl, name: GLuint, len: usize) -> Vec<u8> {
    let slot = super::names::slot_of(gl.ctx(), super::names::Kind::Buffer, name).expect("a buffer");
    let gpu = gl.ctx().objs_ref().buffers[slot as usize].mem.gpu;
    assert_eq!(gl.unflushed(gpu, len), None, "every captured byte is flushed");
    gl.gpu_bytes(gpu, len)
}

/// The first `n` dwords of buffer `name` as the GPU sees them.
fn words(gl: &TestGl, name: GLuint, n: usize) -> Vec<u32> {
    gpu_view(gl, name, 4 * n).chunks(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn floats(gl: &TestGl, name: GLuint, n: usize) -> Vec<f32> {
    words(gl, name, n).into_iter().map(f32::from_bits).collect()
}

/// A PRIMITIVES_WRITTEN query begun now; [`query_result`] ends it.
fn query_begin() -> GLuint {
    let mut q = 0;
    unsafe {
        glGenQueries(1, &mut q);
        glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
    }
    q
}

fn query_result(q: GLuint) -> u32 {
    let mut r = u32::MAX;
    unsafe {
        glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
        glGetQueryObjectuiv(q, GL_QUERY_RESULT, &mut r);
    }
    r
}

/// Generic attribute `index` as a client array of `comps` floats a vertex.
fn client_floats(index: GLuint, comps: GLint, data: &[f32]) {
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(index);
        glVertexAttribPointer(index, comps, GL_FLOAT, GL_FALSE as GLboolean, 0, data.as_ptr().cast());
    }
}

/// C-basic: a pass-through point capture, interleaved, two varyings. The
/// bytes are the inputs (and their doubles) exactly, the GPU sees them,
/// the guard past the six records is untouched, the draw still reaches the
/// back end, and six primitives are counted.
///
/// Revert check: return before `capture` in `tf::pre_draw` and the records
/// stay 0xcdcdcdcd; drop the cache flush in `buffer::write_at` and
/// `gpu_view` names the unflushed byte.
#[test]
fn points_capture_the_inputs_interleaved() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nin vec4 a;\nout vec4 v;\nvoid main() { v = a * 2.0; gl_Position = a; }\n";
    program(vs, FS, &["v", "gl_Position"], GL_INTERLEAVED_ATTRIBS);
    let input: Vec<f32> = (0..24).map(|k| k as f32 * 0.25 - 1.0).collect();
    client_floats(0, 4, &input);
    let b = tf_buffer(0, 6 * 32 + 32);
    unsafe {
        let q = query_begin();
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 6);
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
        assert_eq!(query_result(q), 6);
    }
    let got = floats(&gl, b, 6 * 8);
    for vtx in 0..6 {
        let a = &input[vtx * 4..vtx * 4 + 4];
        let want: Vec<f32> = a.iter().map(|x| x * 2.0).chain(a.iter().copied()).collect();
        assert_eq!(&got[vtx * 8..vtx * 8 + 8], &want[..], "vertex {vtx}");
    }
    assert_eq!(gpu_view(&gl, b, 6 * 32 + 32)[6 * 32..], [0xcd; 32], "the guard");
    assert_eq!(gl.draws().len(), 1, "the GPU draws it as well");
}

/// C-types: integer, unsigned, matrix, array-element and point-size
/// outputs, separate buffers, with a uniform and gl_VertexID. Integers are
/// their bits, a matrix is column-major, `arr[1]` is the second element.
#[test]
fn separate_buffers_take_every_type() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nin vec2 a;\nuniform float u;\nflat out highp ivec2 iv;\nflat out highp uint uu;\n\
              out mat2 m;\nout float arr[3];\n\
              void main() { iv = ivec2(gl_VertexID, -gl_VertexID - 1); uu = uint(gl_VertexID) * 3u + 4000000000u;\n\
              m = mat2(a.x, a.y, u, -u); for (int i = 0; i < 3; i++) arr[i] = a.x + float(i);\n\
              gl_Position = vec4(a, 0.0, 1.0); gl_PointSize = u * 2.0; }\n";
    let p = program(vs, FS, &["iv", "uu", "m", "arr[1]"], GL_SEPARATE_ATTRIBS);
    let input = [1.5f32, -2.0, 0.25, 8.0, -3.0, 0.5];
    client_floats(0, 2, &input);
    let bufs: Vec<GLuint> = (0..4).map(|i| tf_buffer(i, 256)).collect();
    unsafe {
        let name = CString::new("u").unwrap();
        glUniform1f(glGetUniformLocation(p, name.as_ptr()), 0.75);
        let q = query_begin();
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 1, 2); /* vertices 1 and 2 */
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
        assert_eq!(query_result(q), 2);
    }
    let iv = words(&gl, bufs[0], 4);
    assert_eq!(iv, [1, (-2i32) as u32, 2, (-3i32) as u32]);
    assert_eq!(words(&gl, bufs[1], 2), [4_000_000_003, 4_000_000_006]);
    assert_eq!(floats(&gl, bufs[2], 8), [0.25, 8.0, 0.75, -0.75, -3.0, 0.5, 0.75, -0.75]);
    assert_eq!(floats(&gl, bufs[3], 2), [1.25, -2.0]);
    assert_eq!(words(&gl, bufs[3], 3)[2], 0xcdcd_cdcd, "nothing past the two records");
}

/// The vertex order of each draw mode's primitives (Gallium's decomposition
/// with the last vertex provoking, as draw.rs's restart_to_lists), with an
/// incomplete primitive dropped, read back as gl_VertexID. And the
/// primitive counts.
///
/// Revert check: swap the odd strip triangle's first two vertices in
/// `prim_vertex` and the TRIANGLE_STRIP row fails.
#[test]
fn each_draw_mode_captures_its_decomposed_primitives() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nout float k;\nvoid main() { k = float(gl_VertexID); gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["k"], GL_INTERLEAVED_ATTRIBS);
    let cases: [(GLenum, GLenum, i32, &[f32], u32); 9] = [
        (GL_POINTS, GL_POINTS, 3, &[2.0, 3.0, 4.0], 3),
        (GL_LINES, GL_LINES, 5, &[2.0, 3.0, 4.0, 5.0], 2),
        (GL_LINES, GL_LINE_STRIP, 4, &[2.0, 3.0, 3.0, 4.0, 4.0, 5.0], 3),
        (GL_LINES, GL_LINE_LOOP, 4, &[2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 2.0], 4),
        (GL_LINES, GL_LINE_LOOP, 1, &[], 0),
        (GL_TRIANGLES, GL_TRIANGLES, 7, &[2.0, 3.0, 4.0, 5.0, 6.0, 7.0], 2),
        (GL_TRIANGLES, GL_TRIANGLE_STRIP, 5, &[2.0, 3.0, 4.0, 4.0, 3.0, 5.0, 4.0, 5.0, 6.0], 3),
        (GL_TRIANGLES, GL_TRIANGLE_FAN, 5, &[2.0, 3.0, 4.0, 2.0, 4.0, 5.0, 2.0, 5.0, 6.0], 3),
        (GL_TRIANGLES, GL_TRIANGLE_FAN, 2, &[], 0),
    ];
    for (begin, mode, count, want, prims) in cases {
        let b = tf_buffer(0, 64);
        unsafe {
            let q = query_begin();
            glBeginTransformFeedback(begin);
            glDrawArrays(mode, 2, count);
            assert_eq!(gl.error(), GL_NO_ERROR, "mode 0x{mode:x}");
            glEndTransformFeedback();
            assert_eq!(query_result(q), prims, "mode 0x{mode:x} count {count}: primitives");
        }
        let got = floats(&gl, b, want.len() + 1);
        assert_eq!(&got[..want.len()], want, "mode 0x{mode:x} count {count}");
        assert_eq!(got[want.len()].to_bits(), 0xcdcd_cdcd, "mode 0x{mode:x}: nothing more");
    }
}

/// C-instanced: glDrawArraysInstanced with a divisor-2 array and
/// gl_InstanceID, instance-major, both when the back end takes the divisor
/// (one draw) and when draw_instanced draws instance by instance (the
/// plain mock takes divisor 1 only).
#[test]
fn instanced_draws_capture_instance_by_instance() {
    for gpu_divisors in [false, true] {
        let (gl, _be) = fresh();
        if gpu_divisors {
            gl.backend_takes_divisors();
        }
        let vs = "#version 300 es\nin float per_vtx;\nin float per_inst;\nout vec2 o;\n\
                  void main() { o = vec2(per_vtx + per_inst, float(gl_InstanceID)); gl_Position = vec4(0.0); }\n";
        let p = program(vs, FS, &["o"], GL_INTERLEAVED_ATTRIBS);
        let (lv, li) = unsafe {
            let (a, b) = (CString::new("per_vtx").unwrap(), CString::new("per_inst").unwrap());
            (glGetAttribLocation(p, a.as_ptr()) as GLuint, glGetAttribLocation(p, b.as_ptr()) as GLuint)
        };
        let vtx = [1.0f32, 2.0, 3.0];
        let inst = [10.0f32, 20.0];
        client_floats(lv, 1, &vtx);
        client_floats(li, 1, &inst);
        let b = tf_buffer(0, 128);
        unsafe {
            glVertexAttribDivisor(li, 2);
            let q = query_begin();
            glBeginTransformFeedback(GL_TRIANGLES);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glEndTransformFeedback();
            assert_eq!(query_result(q), 3, "one triangle per instance");
        }
        let want = [11.0, 0.0, 12.0, 0.0, 13.0, 0.0, 11.0, 1.0, 12.0, 1.0, 13.0, 1.0, 21.0, 2.0, 22.0, 2.0, 23.0, 2.0];
        assert_eq!(floats(&gl, b, 18), want, "divisors on the GPU: {gpu_divisors}");
        assert_eq!(gl.draws().len(), if gpu_divisors { 1 } else { 3 });
    }
}

/// C-pause, C-offset, C-overflow: a range at offset 16 inside guards; two
/// points, a paused draw (drawn, not captured), one more point after the
/// resume, back to back; a draw that does not fit is refused whole (no
/// bytes, no draw, no count) and one that exactly fits is not.
#[test]
fn pause_offset_and_overflow() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nout float k;\nvoid main() { k = float(gl_VertexID) + 0.5; gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["k"], GL_INTERLEAVED_ATTRIBS);
    let b = tf_buffer(0, 64);
    unsafe {
        glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b, 16, 16); /* room for four */
        let q = query_begin();
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 2);
        glPauseTransformFeedback();
        glDrawArrays(GL_POINTS, 5, 3);
        glResumeTransformFeedback();
        glDrawArrays(GL_POINTS, 7, 1);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.draws().len(), 3, "the paused draw is drawn");
        glDrawArrays(GL_POINTS, 0, 2);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "five records do not fit four");
        assert!(gl.draws().is_empty(), "nor is it drawn");
        glDrawArrays(GL_POINTS, 9, 1);
        assert_eq!(gl.error(), GL_NO_ERROR, "the fourth record fits exactly");
        glEndTransformFeedback();
        assert_eq!(query_result(q), 4, "2 + 1 + 1: the paused and the refused draws count nothing");
    }
    let w = words(&gl, b, 16);
    assert_eq!(w[..4], [0xcdcd_cdcd; 4], "the guard before the range");
    assert_eq!(w[4..8].iter().map(|&x| f32::from_bits(x)).collect::<Vec<_>>(), [0.5, 1.5, 7.5, 9.5]);
    assert_eq!(w[8..], [0xcdcd_cdcd; 8], "the guard after it");
}

/// C-discard: under rasterizer discard the capture happens and nothing is
/// drawn.
#[test]
fn discard_captures_without_drawing() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nout float k;\nvoid main() { k = float(gl_VertexID); gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["k"], GL_INTERLEAVED_ATTRIBS);
    let b = tf_buffer(0, 64);
    unsafe {
        glEnable(GL_RASTERIZER_DISCARD);
        let q = query_begin();
        glBeginTransformFeedback(GL_LINES);
        glDrawArrays(GL_LINE_STRIP, 0, 3);
        glEndTransformFeedback();
        assert_eq!(query_result(q), 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
    assert_eq!(floats(&gl, b, 4), [0.0, 1.0, 1.0, 2.0]);
    assert!(gl.draws().is_empty(), "discarded");
}

/// C-hazard: a pending kick still reads the buffer, so the capture goes to
/// fresh storage (copy on write, as glBufferSubData does) and the old
/// storage the kick reads keeps its bytes.
///
/// Revert check: drop the `buffer::writable` call in `capture` and the old
/// storage changes under the kick.
#[test]
fn a_buffer_a_pending_kick_reads_is_copied_first() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nout float k;\nvoid main() { k = 3.0; gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["k"], GL_INTERLEAVED_ATTRIBS);
    let b = tf_buffer(0, 16);
    let slot = super::names::slot_of(gl.ctx(), super::names::Kind::Buffer, b).unwrap() as usize;
    let id = gl.kick_id() + 1;
    gl.ctx().objs().buffers[slot].last_use = id;
    gl.set_kick_pending(id, true);
    let old = gl.ctx().objs_ref().buffers[slot].mem.gpu;
    unsafe {
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
    }
    assert_ne!(gl.ctx().objs_ref().buffers[slot].mem.gpu, old, "fresh storage");
    assert_eq!(gl.gpu_bytes(old, 16), [0xcd; 16], "what the pending kick reads is unchanged");
    assert_eq!(floats(&gl, b, 3), [3.0, 3.0, f32::from_bits(0xcdcd_cdcd)]);
}

/// C-refuse: a vertex shader that samples a texture cannot be evaluated:
/// GL_INVALID_OPERATION with the reason, nothing written, drawn or counted.
#[test]
fn a_vertex_shader_that_samples_is_refused() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nuniform sampler2D s;\nout vec4 v;\n\
              void main() { v = textureLod(s, vec2(0.5), 0.0); gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["v"], GL_INTERLEAVED_ATTRIBS);
    let b = tf_buffer(0, 64);
    unsafe {
        let q = query_begin();
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        glEndTransformFeedback();
        assert_eq!(query_result(q), 0);
    }
    assert!(gl.logs().iter().any(|l| l.contains("samples a texture")), "{:?}", gl.logs());
    assert_eq!(words(&gl, b, 1), [0xcdcd_cdcd]);
    assert!(gl.draws().is_empty());
}

/// C-feedback-loop, and the uniform block: capture into A, then draw from
/// A as a vertex buffer (a VBO, not a client array) through a second
/// capturing program whose vertex shader reads a uniform block; the second
/// capture reads the first one's values.
#[test]
fn a_captured_buffer_feeds_the_next_capture() {
    let (gl, _be) = fresh();
    let vs1 =
        "#version 300 es\nout vec2 o;\nvoid main() { o = vec2(float(gl_VertexID), 1.0); gl_Position = vec4(0.0); }\n";
    program(vs1, FS, &["o"], GL_INTERLEAVED_ATTRIBS);
    let a = tf_buffer(0, 3 * 8);
    unsafe {
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 3);
        glEndTransformFeedback();
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
    let vs2 = "#version 300 es\nin vec2 p;\nlayout(std140) uniform Block { vec4 scale; };\nout float r;\n\
               void main() { r = p.x * scale.x + p.y * scale.y; gl_Position = vec4(0.0); }\n";
    let prog2 = program(vs2, FS, &["r"], GL_INTERLEAVED_ATTRIBS);
    let mut ubo = 0;
    let scale = [10.0f32, 0.5, 0.0, 0.0];
    unsafe {
        glGenBuffers(1, &mut ubo);
        glBindBuffer(GL_UNIFORM_BUFFER, ubo);
        glBufferData(GL_UNIFORM_BUFFER, 16, scale.as_ptr().cast(), GL_STATIC_DRAW);
        let name = CString::new("Block").unwrap();
        glUniformBlockBinding(prog2, glGetUniformBlockIndex(prog2, name.as_ptr()), 2);
        glBindBufferBase(GL_UNIFORM_BUFFER, 2, ubo);
        glBindBuffer(GL_ARRAY_BUFFER, a);
        let pname = CString::new("p").unwrap();
        let loc = glGetAttribLocation(prog2, pname.as_ptr()) as GLuint;
        glEnableVertexAttribArray(loc);
        glVertexAttribPointer(loc, 2, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
    }
    let b = tf_buffer(0, 16);
    unsafe {
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
    }
    assert_eq!(floats(&gl, b, 3), [0.5, 10.5, 20.5]);
}

/// A GLSL ES 1.00 program captures too (ES 3.0 applies transform feedback
/// to any program): gl_test case 203's pair, `gl_Position` and a
/// `varying`, from a VBO with glBindAttribLocation.
#[test]
fn an_es100_program_captures_its_varyings() {
    let (gl, _be) = fresh();
    let vs = "#version 100\nattribute vec4 a_pos;\nattribute vec4 a_color;\nvarying vec4 v_color;\n\
              void main() { gl_Position = a_pos; v_color = a_color; }\n";
    let fs = "#version 100\nprecision mediump float;\nvarying vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";
    let p = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        let (pos, col) = (CString::new("a_pos").unwrap(), CString::new("a_color").unwrap());
        glBindAttribLocation(p, 3, pos.as_ptr());
        glBindAttribLocation(p, 5, col.as_ptr());
        let names = [CString::new("gl_Position").unwrap(), CString::new("v_color").unwrap()];
        let ptrs: Vec<*const GLchar> = names.iter().map(|n| n.as_ptr()).collect();
        glTransformFeedbackVaryings(p, 2, ptrs.as_ptr(), GL_INTERLEAVED_ATTRIBS);
        glLinkProgram(p);
        let mut ok = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        p
    };
    let _ = p;
    /* x, y, z, w, r, g, b, a per vertex, interleaved in one VBO */
    let vbo_data: Vec<f32> = (0..3).flat_map(|v| (0..8).map(move |k| (v * 8 + k) as f32 / 8.0)).collect();
    let mut vbo = 0;
    unsafe {
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, 96, vbo_data.as_ptr().cast(), GL_STATIC_DRAW);
        glEnableVertexAttribArray(3);
        glVertexAttribPointer(3, 4, GL_FLOAT, GL_FALSE as GLboolean, 32, core::ptr::null());
        glEnableVertexAttribArray(5);
        glVertexAttribPointer(5, 4, GL_FLOAT, GL_FALSE as GLboolean, 32, 16usize as *const core::ffi::c_void);
    }
    let b = tf_buffer(0, 96);
    unsafe {
        let q = query_begin();
        glBeginTransformFeedback(GL_TRIANGLES);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
        assert_eq!(query_result(q), 1);
    }
    assert_eq!(floats(&gl, b, 24), vbo_data);
}

/// C-perf, run by hand: 10^4 points of a vertex shader of about 20
/// operations through the capture, and one point (the lowering and set-up
/// alone). Prints the time per vertex and per draw for the record
/// (docs/es3-transform-feedback.md section 4.1, "Speed").
///
/// ```text
/// cargo test --release --no-default-features --features host-test,gpu-powervr,shaderc-online \
///     --lib capture_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, run by hand"]
fn capture_cost() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nin vec4 a;\nuniform mat4 m;\nuniform vec4 k;\nout vec4 v;\nout vec3 n;\n\
              void main() { vec4 p = m * a; v = p * k + vec4(a.w); n = normalize(p.xyz + k.xyz);\n\
              gl_Position = p; }\n";
    let p = program(vs, FS, &["v", "n", "gl_Position"], GL_INTERLEAVED_ATTRIBS);
    const N: usize = 10_000;
    let input: Vec<f32> = (0..4 * N).map(|i| (i % 97) as f32 * 0.01 + 0.5).collect();
    client_floats(0, 4, &input);
    let ident = [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    unsafe {
        let (mn, kn) = (CString::new("m").unwrap(), CString::new("k").unwrap());
        glUniformMatrix4fv(glGetUniformLocation(p, mn.as_ptr()), 1, GL_FALSE as GLboolean, ident.as_ptr());
        glUniform4f(glGetUniformLocation(p, kn.as_ptr()), 0.5, 0.25, 2.0, 1.0);
    }
    let _b = tf_buffer(0, N * 44);
    let time = |count: i32| {
        let t = std::time::Instant::now();
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            glDrawArrays(GL_POINTS, 0, count);
            glEndTransformFeedback();
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        t.elapsed().as_secs_f64() * 1e6
    };
    let one = (0..20).map(|_| time(1)).fold(f64::MAX, f64::min);
    let many = time(N as i32);
    std::println!(
        "capture_cost: one-point draw {one:.0} us (lowering and set-up); {N} points {many:.0} us, {:.2} us/vertex",
        (many - one) / N as f64
    );
}

/// A range bound before the buffer was respecified smaller is cut to the
/// buffer as it is at the draw: the capture never writes past the storage.
///
/// Revert check: take the range's size alone in `tf::fits` and the five
/// records are written into a 16-byte buffer.
#[test]
fn a_range_is_cut_to_the_buffer_it_names_now() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nout float k;\nvoid main() { k = 1.0; gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["k"], GL_INTERLEAVED_ATTRIBS);
    let b = tf_buffer(0, 64);
    unsafe {
        glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b, 0, 64);
        glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, 16, core::ptr::null(), GL_DYNAMIC_READ);
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 5);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "20 bytes do not fit the 16 left");
        glDrawArrays(GL_POINTS, 0, 4);
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
    }
    assert_eq!(floats(&gl, b, 4), [1.0; 4]);
}

/// C-separate-room and C-builtins: separate buffers of different sizes, the
/// smallest decides whether a draw fits; gl_PointSize captured beside a
/// user varying.
#[test]
fn the_smallest_separate_buffer_decides_and_point_size_is_captured() {
    let (gl, _be) = fresh();
    let vs = "#version 300 es\nout vec2 o;\nvoid main() { o = vec2(float(gl_VertexID), 2.0);\n\
              gl_PointSize = float(gl_VertexID) + 4.0; gl_Position = vec4(0.0); }\n";
    program(vs, FS, &["o", "gl_PointSize"], GL_SEPARATE_ATTRIBS);
    let wide = tf_buffer(0, 64); /* 8 vertices of 8 bytes */
    let narrow = tf_buffer(1, 12); /* 3 vertices of 4 bytes */
    unsafe {
        glBeginTransformFeedback(GL_POINTS);
        glDrawArrays(GL_POINTS, 0, 4);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "the 12-byte buffer holds three");
        glDrawArrays(GL_POINTS, 0, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        glEndTransformFeedback();
    }
    assert_eq!(floats(&gl, wide, 6), [0.0, 2.0, 1.0, 2.0, 2.0, 2.0]);
    assert_eq!(floats(&gl, narrow, 3), [4.0, 5.0, 6.0]);
}
