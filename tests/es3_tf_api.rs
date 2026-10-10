// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 transform feedback and rasterizer discard through the GL API on the
//! host (`16ac8641`, docs/es3-transform-feedback.md section 5): the objects
//! and their indexed bindings, `glTransformFeedbackVaryings` resolved at link
//! against the vertex outputs, `glGetTransformFeedbackVarying`, the
//! begin/end/pause/resume rules, the primitives-written query, and
//! GL_RASTERIZER_DISCARD stopping draws and clears before the back end.
//!
//! The capture itself is tested byte by byte in `src/gl/tf_capture_tests.rs`;
//! here one capture is read back through glMapBufferRange, as dEQP reads it
//! (`capture_writes_the_bound_buffer`). The back end is a stand-in that counts
//! the draws it is handed.
//!
//! An ES 2.0-reporting build (`report-es2`) must answer as before:
//! `es2_build_keeps_its_answers` is the only test it runs. The shader-level
//! tests need the in-process compiler (`shaderc-online`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tf_api
//! ```

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// Draws the stand-in back end was handed.
static DRAWS: AtomicU32 = AtomicU32::new(0);
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    DRAWS.fetch_add(1, Ordering::Relaxed);
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-tf-api".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    reset();
    g
}

/// Back to a known state after the previous test: no capture, the default
/// transform feedback object, no program, discard off, the error flag clear.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        if geti(GL_TRANSFORM_FEEDBACK_ACTIVE) == 1 {
            glEndTransformFeedback();
        }
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        glUseProgram(0);
        glDisable(GL_RASTERIZER_DISCARD);
        while glGetError() != GL_NO_ERROR {}
    }
    DRAWS.store(0, Ordering::Relaxed);
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn geti_i(pname: GLenum, i: GLuint) -> GLint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegeri_v(pname, i, &mut v) };
    v
}

/* ---------------------------------------------------------------- ES 2.0 -- */

/// The ES 2.0 report answers as before: every limit 0, nothing ever active,
/// the same refusals, and discard only recorded.
#[cfg(feature = "report-es2")]
#[test]
fn es2_build_keeps_its_answers() {
    let _g = context();
    assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS), 0);
    assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS), 0);
    assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS), 0);
    assert_eq!((geti(GL_TRANSFORM_FEEDBACK_ACTIVE), geti(GL_TRANSFORM_FEEDBACK_PAUSED)), (0, 0));
    let mut b = 0;
    // SAFETY: plain GL calls; one name.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 64, core::ptr::null(), GL_DYNAMIC_COPY);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
        assert_eq!(err(), GL_NO_ERROR, "zero is an unbind");
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
        assert_eq!(err(), GL_INVALID_OPERATION, "a real buffer cannot be honoured");
        glBeginTransformFeedback(GL_POINTS);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glBeginTransformFeedback(GL_LINE_STRIP);
        assert_eq!(err(), GL_INVALID_ENUM);
        glEndTransformFeedback();
        assert_eq!(err(), GL_INVALID_OPERATION);
        glPauseTransformFeedback();
        assert_eq!(err(), GL_INVALID_OPERATION);
        let mut q = 0;
        glGenQueries(1, &mut q);
        glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
        assert_eq!(err(), GL_INVALID_OPERATION);
        assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0), 0);
        assert_eq!(err(), GL_NO_ERROR);
        glEnable(GL_RASTERIZER_DISCARD);
        assert_eq!(glIsEnabled(GL_RASTERIZER_DISCARD), GL_TRUE_B, "recorded, as before");
    }
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(all(not(feature = "report-es2"), feature = "shaderc-online"))]
mod es3 {
    use super::*;

    const VS: &str = "#version 300 es\nin vec4 a;\nout vec4 v;\nout float f;\nout mediump vec2 arr[3];\n\
        void main() {\n v = a; f = a.x;\n for (int i = 0; i < 3; i++) arr[i] = a.xy;\n gl_Position = a; }\n";
    const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let text = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    }

    /// A linked program with `varyings` asked for; (name, linked, info log).
    fn link(varyings: &[&str], mode: GLenum) -> (GLuint, bool, String) {
        // SAFETY: plain GL calls; every pointer outlives its call.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            let names: Vec<CString> = varyings.iter().map(|n| CString::new(*n).unwrap()).collect();
            let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
            glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), mode);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            let mut log = [0u8; 512];
            let mut len: GLsizei = 0;
            glGetProgramInfoLog(p, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char);
            (p, ok == 1, String::from_utf8_lossy(&log[..len.max(0) as usize]).into_owned())
        }
    }

    fn buffer(size: usize) -> GLuint {
        let mut b = 0;
        // SAFETY: plain GL calls; one name.
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_ARRAY_BUFFER, b);
            glBufferData(GL_ARRAY_BUFFER, size as GLsizeiptr, core::ptr::null(), GL_DYNAMIC_COPY);
        }
        b
    }

    fn gen_tf() -> GLuint {
        let mut t = 0;
        // SAFETY: one name.
        unsafe { glGenTransformFeedbacks(1, &mut t) };
        t
    }

    /// A program with `varyings`, current, and `bufs` buffers on indices 0...
    fn ready(varyings: &[&str], mode: GLenum, bufs: u32) -> GLuint {
        let (p, ok, log) = link(varyings, mode);
        assert!(ok, "{log}");
        for i in 0..bufs {
            let b = buffer(1024);
            // SAFETY: plain GL call.
            unsafe { glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, i, b) };
        }
        // SAFETY: plain GL call.
        unsafe { glUseProgram(p) };
        p
    }

    #[test]
    fn limits_are_the_es3_minimums() {
        let _g = context();
        assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS), 64);
        assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS), 4);
        assert_eq!(geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS), 64);
        assert_eq!(err(), GL_NO_ERROR);
    }

    #[test]
    fn objects_have_the_binding_and_lifetime_rules() {
        let _g = context();
        let (a, b) = (gen_tf(), gen_tf());
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsTransformFeedback(a), GL_FALSE_B);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 1000);
            assert_eq!(err(), GL_INVALID_OPERATION, "a name nobody generated");
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            assert_eq!((err(), geti(GL_TRANSFORM_FEEDBACK_BINDING) as GLuint), (GL_NO_ERROR, a));
            assert_eq!(glIsTransformFeedback(a), GL_TRUE_B);
            glDeleteTransformFeedbacks(1, &a);
            assert_eq!(geti(GL_TRANSFORM_FEEDBACK_BINDING), 0);
            assert_eq!(glIsTransformFeedback(a), GL_FALSE_B);
            glDeleteTransformFeedbacks(1, &b);
            assert_eq!(err(), GL_NO_ERROR);
        }
    }

    #[test]
    fn the_indexed_bindings_belong_to_the_bound_object() {
        let _g = context();
        let (a, b) = (gen_tf(), gen_tf());
        let (x, y) = (buffer(64), buffer(128));
        // SAFETY: plain GL calls.
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, a);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x);
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 2, y, 4, 32);
            assert_eq!(err(), GL_NO_ERROR);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0) as GLuint, x);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_START, 2), 4);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_SIZE, 2), 32);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, b);
            assert_eq!(geti_i(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0), 0);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 4, x);
            assert_eq!(err(), GL_INVALID_VALUE);
            glBindBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x, 2, 8);
            assert_eq!(err(), GL_INVALID_VALUE);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glDeleteTransformFeedbacks(1, &a);
            glDeleteTransformFeedbacks(1, &b);
        }
    }

    #[test]
    fn varyings_link_and_are_reported() {
        let _g = context();
        let (p, ok, log) = link(&["v", "arr[1]", "gl_Position"], GL_INTERLEAVED_ATTRIBS);
        assert!(ok, "{log}");
        let mut v = -1;
        // SAFETY: plain GL calls with room for the answers.
        unsafe {
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_VARYINGS, &mut v);
            assert_eq!(v, 3);
            glGetProgramiv(p, GL_TRANSFORM_FEEDBACK_BUFFER_MODE, &mut v);
            assert_eq!(v as GLenum, GL_INTERLEAVED_ATTRIBS);
            let (mut len, mut size, mut ty) = (0, 0, 0);
            let mut name = [0 as c_char; 32];
            glGetTransformFeedbackVarying(p, 1, 32, &mut len, &mut size, &mut ty, name.as_mut_ptr());
            assert_eq!((len, size, ty), (6, 1, GL_FLOAT_VEC2));
            assert_eq!(&name[..7], b"arr[1]\0".map(|b| b as c_char));
            glGetTransformFeedbackVarying(p, 3, 32, &mut len, &mut size, &mut ty, name.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_VALUE);
        }
        for (names, mode, needle) in [
            (&["nosuch"][..], GL_INTERLEAVED_ATTRIBS, "undeclared"),
            (&["v", "v"][..], GL_INTERLEAVED_ATTRIBS, "more than once"),
        ] {
            let (_, ok, log) = link(names, mode);
            assert!(!ok && log.contains(needle), "{names:?}: {log:?}");
        }
        let (_, ok, log) = link(&["f", "v", "arr[0]", "arr[1]"], GL_SEPARATE_ATTRIBS);
        assert!(ok, "four separate varyings: {log}");
        let (_, ok, log) = link(&["arr", "v"], GL_SEPARATE_ATTRIBS);
        assert!(ok, "a whole array to one buffer (separate components 64): {log}");
    }

    #[test]
    fn begin_end_pause_resume_and_their_interlocks() {
        let _g = context();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(err(), GL_INVALID_OPERATION, "no program");
            let p = ready(&["v"], GL_INTERLEAVED_ATTRIBS, 1);
            glBeginTransformFeedback(GL_LINE_LOOP);
            assert_eq!(err(), GL_INVALID_ENUM);
            glEndTransformFeedback();
            assert_eq!(err(), GL_INVALID_OPERATION);
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(err(), GL_NO_ERROR);
            assert_eq!(geti(GL_TRANSFORM_FEEDBACK_ACTIVE), 1);
            glUseProgram(0);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glLinkProgram(p);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glPauseTransformFeedback();
            assert_eq!((err(), geti(GL_TRANSFORM_FEEDBACK_PAUSED)), (GL_NO_ERROR, 1));
            glUseProgram(0);
            assert_eq!(err(), GL_NO_ERROR, "a paused capture may change program");
            glResumeTransformFeedback();
            assert_eq!(err(), GL_INVALID_OPERATION, "not the program Begin saw");
            glUseProgram(p);
            glResumeTransformFeedback();
            glEndTransformFeedback();
            assert_eq!((err(), geti(GL_TRANSFORM_FEEDBACK_ACTIVE)), (GL_NO_ERROR, 0));
        }
    }

    /// The capture through the API (tf-capture): three points of a client
    /// array, `v` and `f` interleaved, read back with glMapBufferRange as
    /// dEQP reads them; the draw reaches the back end too, and the rules
    /// still come first.
    #[test]
    fn capture_writes_the_bound_buffer() {
        let _g = context();
        ready(&["v", "f"], GL_INTERLEAVED_ATTRIBS, 1);
        let input: [f32; 12] = [1.0, 2.0, 3.0, 4.0, -1.0, -2.0, -3.0, -4.0, 0.5, 0.25, 0.125, 0.0625];
        // SAFETY: plain GL calls; the client array outlives the draw.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, input.as_ptr().cast());
            glBeginTransformFeedback(GL_POINTS);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(err(), GL_INVALID_OPERATION, "mode does not match");
            glDrawElements(GL_POINTS, 1, GL_UNSIGNED_SHORT, core::ptr::null());
            assert_eq!(err(), GL_INVALID_OPERATION, "indexed");
            assert_eq!(DRAWS.load(Ordering::Relaxed), 0);
            glDrawArrays(GL_POINTS, 0, 3);
            assert_eq!(err(), GL_NO_ERROR, "captured");
            assert_eq!(DRAWS.load(Ordering::Relaxed), 1, "and drawn");
            glEndTransformFeedback();
            glDisableVertexAttribArray(0);
            let mut b: GLint = 0;
            glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &mut b);
            glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, b as GLuint);
            let p = glMapBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 60, GL_MAP_READ_BIT) as *const f32;
            assert!(!p.is_null(), "mapped");
            let got = core::slice::from_raw_parts(p, 15).to_vec();
            glUnmapBuffer(GL_TRANSFORM_FEEDBACK_BUFFER);
            assert_eq!(err(), GL_NO_ERROR);
            let want: Vec<f32> = input.chunks(4).flat_map(|a| [a[0], a[1], a[2], a[3], a[0]]).collect();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn primitives_written_query_runs_its_state_machine() {
        let _g = context();
        let mut q = 0;
        let (mut v, mut r) = (-1, 99);
        // SAFETY: plain GL calls.
        unsafe {
            glGenQueries(1, &mut q);
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
            assert_eq!(err(), GL_NO_ERROR);
            glGetQueryiv(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, GL_CURRENT_QUERY, &mut v);
            assert_eq!(v as GLuint, q);
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            glGetQueryObjectuiv(q, GL_QUERY_RESULT_AVAILABLE, &mut r);
            assert_eq!((err(), r), (GL_NO_ERROR, GL_TRUE));
            glGetQueryObjectuiv(q, GL_QUERY_RESULT, &mut r);
            assert_eq!((err(), r), (GL_NO_ERROR, 0));
            glBeginQuery(0x8c87, q);
            assert_eq!(err(), GL_INVALID_ENUM, "GL_PRIMITIVES_GENERATED is not ES 3.0");
            glDeleteQueries(1, &q);
        }
    }

    #[test]
    fn rasterizer_discard_keeps_draws_and_clears_from_the_back_end() {
        let _g = context();
        let (p, ok, log) = link(&[], GL_INTERLEAVED_ATTRIBS);
        assert!(ok, "{log}");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glEnable(GL_RASTERIZER_DISCARD);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null());
            glDrawRangeElements(GL_TRIANGLES, 0, 2, 3, GL_UNSIGNED_SHORT, core::ptr::null());
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            assert_eq!(err(), GL_NO_ERROR);
            glFinish();
            assert_eq!(DRAWS.load(Ordering::Relaxed), 0, "nothing reached the back end");
            glDrawArrays(GL_TRIANGLES, 0, -1);
            assert_eq!(err(), GL_INVALID_VALUE, "argument errors still fire");
            glDisable(GL_RASTERIZER_DISCARD);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
            assert!(DRAWS.load(Ordering::Relaxed) > 0, "without discard the draw is submitted");
        }
    }
}
