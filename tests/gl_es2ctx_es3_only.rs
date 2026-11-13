// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0-context answers of two ES 3.0 changes, pinned on a GLSL ES 1.00
//! program (item gl-es2ctx-es3-only): 2830b218 (GL_MAX_SAMPLES asks
//! src/gl/msaa.rs) and bc200f21 (glUniform*ui checks the type of a GLSL ES
//! 3.00 program). Both were tested only in ES 3.0 (tests/es3_negapi_uniform.rs);
//! this file asks what a client of an ES 2.0 context sees.
//!
//! - GL_MAX_SAMPLES is not an ES 2.0 state variable (it enters with ES 3.0 and
//!   with extensions such as GL_ANGLE_framebuffer_multisample, none of which
//!   GL_EXTENSIONS lists): glGetIntegerv is GL_INVALID_ENUM and writes
//!   nothing. GL_SAMPLES and GL_SAMPLE_BUFFERS are ES 2.0 state (ES 2.0.25
//!   6.1.x): 0 and 0 on a single-sampled default framebuffer, which
//!   functional.state_query and implementation_limits read.
//! - The uniform setters follow ES 2.0 2.10.4: a setter whose type or size
//!   does not match the uniform (a bool takes the 1i/1f family of its size, a
//!   sampler takes 1i only) is GL_INVALID_OPERATION and leaves the value;
//!   location -1 is silently ignored; a location that is not the program's,
//!   or no program in use, is GL_INVALID_OPERATION; glUniformMatrix with
//!   transpose GL_TRUE is GL_INVALID_VALUE. Sources: dEQP-GLES2
//!   functional.uniform_api (es2fUniformApiTests.cpp) and
//!   functional.negative_api.shader (es2fNegativeShaderApiTests.cpp).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test gl_es2ctx_es3_only
//! ```
//!
//! Compiled only with `report-es2`: an ES 3.0 build answers GL_MAX_SAMPLES.
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es2ctx-es3-only".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
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
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

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
        assert_eq!(ok, 1, "shader compiles:\n{text:?}");
        s
    }
}

/// `vs`/`fs` linked and current.
fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "program links");
        glUseProgram(p);
        assert_eq!(err(), GL_NO_ERROR);
        p
    }
}

fn loc(p: GLuint, name: &std::ffi::CStr) -> GLint {
    // SAFETY: plain GL call.
    let l = unsafe { glGetUniformLocation(p, name.as_ptr()) };
    assert!(l >= 0, "{name:?} is active");
    l
}

const VS100: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS100: &str = "precision mediump float;\n\
    uniform float f1;\nuniform vec2 v2;\nuniform vec4 v4;\nuniform int i1;\nuniform ivec4 iv4;\n\
    uniform bool b1;\nuniform bvec2 bv2;\nuniform mat3 m3;\nuniform mat4 m4;\nuniform sampler2D s;\n\
    void main() { gl_FragColor = v4 + vec4(f1 + v2.x + float(i1) + float(iv4.x) + float(b1) + float(bv2.x)\n\
    + m3[0][0] + m4[0][0]) + texture2D(s, vec2(0.5)); }\n";

fn geti(pname: GLenum) -> (GLenum, GLint) {
    let mut v: GLint = 0x5a5a;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    (err(), v)
}

fn get_string(name: GLenum) -> String {
    // SAFETY: plain GL call; the string is NUL-terminated and static.
    unsafe { CStr::from_ptr(glGetString(name) as *const c_char).to_str().unwrap().to_owned() }
}

fn get_f(p: GLuint, l: GLint, n: usize) -> Vec<GLfloat> {
    let mut v = vec![-7.0; n];
    // SAFETY: plain GL call; `v` holds `n` values.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn get_i(p: GLuint, l: GLint, n: usize) -> Vec<GLint> {
    let mut v = vec![-7; n];
    // SAFETY: plain GL call; `v` holds `n` values.
    unsafe { glGetUniformiv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// ES 2.0 6.1.1: GL_MAX_SAMPLES is no ES 2.0 state unless an extension that
/// defines it is listed; the list has none, so GL_INVALID_ENUM and the output
/// untouched (2830b218 must not have opened the name in ES 2.0).
#[test]
fn max_samples_is_invalid_enum_without_a_multisample_extension() {
    let _g = context();
    let ext = get_string(GL_EXTENSIONS);
    assert!(!ext.to_ascii_lowercase().contains("multisample"), "a multisample extension is listed: {ext}");
    assert_eq!(geti(GL_MAX_SAMPLES), (GL_INVALID_ENUM, 0x5a5a));
}

/// ES 2.0 6.1.x: GL_SAMPLE_BUFFERS and GL_SAMPLES of a single-sampled default
/// framebuffer are 0 and 0, with no error.
#[test]
fn samples_and_sample_buffers_are_zero_on_the_default_framebuffer() {
    let _g = context();
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    assert_eq!(geti(GL_SAMPLE_BUFFERS), (GL_NO_ERROR, 0));
    assert_eq!(geti(GL_SAMPLES), (GL_NO_ERROR, 0));
}

/// ES 2.0 2.10.4: matching setters load without error and read back.
#[test]
fn matching_setters_load_and_read_back() {
    let _g = context();
    let p = program(VS100, FS100);
    let (f1, v2, v4, i, iv4, b1, bv2, s) = (
        loc(p, c"f1"),
        loc(p, c"v2"),
        loc(p, c"v4"),
        loc(p, c"i1"),
        loc(p, c"iv4"),
        loc(p, c"b1"),
        loc(p, c"bv2"),
        loc(p, c"s"),
    );
    // SAFETY: plain GL calls.
    unsafe {
        glUniform1f(f1, 2.5);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_f(p, f1, 1), [2.5]);
        glUniform2f(v2, 1.0, 2.0);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_f(p, v2, 2), [1.0, 2.0]);
        glUniform4f(v4, 1.0, 2.0, 3.0, 4.0);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_f(p, v4, 4), [1.0, 2.0, 3.0, 4.0]);
        glUniform1i(i, 9);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, i, 1), [9]);
        glUniform4i(iv4, 1, 2, 3, 4);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, iv4, 4), [1, 2, 3, 4]);
        glUniform1i(s, 3);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, s, 1), [3]);
        /* a bool takes the int and the float family of its size */
        glUniform1i(b1, 1);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, b1, 1), [1]);
        glUniform1f(b1, 0.0);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, b1, 1), [0]);
        glUniform2i(bv2, 1, 0);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, bv2, 2), [1, 0]);
        glUniform2f(bv2, 0.0, 1.0);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_i(p, bv2, 2), [0, 1]);
    }
}

/// ES 2.0 2.10.4: a setter of the wrong type or size is GL_INVALID_OPERATION
/// and the uniform keeps its value (negative_api.shader and uniform_api).
#[test]
fn mismatched_setters_are_invalid_operation_and_leave_the_value() {
    let _g = context();
    let p = program(VS100, FS100);
    let (f1, v2, i, iv4, b1, s, m3) =
        (loc(p, c"f1"), loc(p, c"v2"), loc(p, c"i1"), loc(p, c"iv4"), loc(p, c"b1"), loc(p, c"s"), loc(p, c"m3"));
    // SAFETY: plain GL calls; the arrays hold what the setters read.
    unsafe {
        glUniform1f(f1, 2.5);
        glUniform2f(v2, 1.0, 2.0);
        glUniform1i(i, 9);
        glUniform4i(iv4, 1, 2, 3, 4);
        glUniform1i(b1, 1);
        glUniform1i(s, 3);
        assert_eq!(err(), GL_NO_ERROR);

        glUniform1i(f1, 7); /* int on a float */
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform1f(i, 7.0); /* float on an int */
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform2i(v2, 7, 7); /* int on a vec2 */
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform4f(v2, 7.0, 7.0, 7.0, 7.0); /* wrong size */
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform1f(v2, 7.0);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform1i(iv4, 7);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform4f(iv4, 7.0, 7.0, 7.0, 7.0);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform2i(b1, 7, 7); /* a bool takes its own size only */
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniform1f(s, 7.0); /* a sampler takes 1i only */
        assert_eq!(err(), GL_INVALID_OPERATION);
        let m = [7.0f32; 16];
        glUniformMatrix4fv(m3, 1, 0, m.as_ptr()); /* mat4 on a mat3 */
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUniformMatrix4fv(f1, 1, 0, m.as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION);
    }
    assert_eq!(get_f(p, f1, 1), [2.5]);
    assert_eq!(get_f(p, v2, 2), [1.0, 2.0]);
    assert_eq!(get_i(p, i, 1), [9]);
    assert_eq!(get_i(p, iv4, 4), [1, 2, 3, 4]);
    assert_eq!(get_i(p, b1, 1), [1]);
    assert_eq!(get_i(p, s, 1), [3]);
}

/// ES 2.0 2.10.4: matrices of the right size load; transpose GL_TRUE is
/// GL_INVALID_VALUE (ES 2.0 has no transpose); a wrong-size matrix is
/// GL_INVALID_OPERATION; the values stay on every refusal.
#[test]
fn matrix_setters() {
    let _g = context();
    let p = program(VS100, FS100);
    let (m3, m4) = (loc(p, c"m3"), loc(p, c"m4"));
    let id4: [f32; 16] = core::array::from_fn(|k| if k % 5 == 0 { 1.0 } else { 0.0 });
    let id3: [f32; 9] = core::array::from_fn(|k| if k % 4 == 0 { 2.0 } else { 0.0 });
    // SAFETY: plain GL calls; the arrays hold what the setters read.
    unsafe {
        glUniformMatrix4fv(m4, 1, 0, id4.as_ptr());
        glUniformMatrix3fv(m3, 1, 0, id3.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(get_f(p, m4, 16), id4);
        assert_eq!(get_f(p, m3, 9), id3);
        let other = [5.0f32; 16];
        glUniformMatrix4fv(m4, 1, 1, other.as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glUniformMatrix3fv(m3, 1, 1, other.as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE);
        glUniformMatrix3fv(m4, 1, 0, other.as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION);
    }
    assert_eq!(get_f(p, m4, 16), id4);
    assert_eq!(get_f(p, m3, 9), id3);
}

/// ES 2.0 2.10.4: location -1 is silently ignored by every setter; a
/// location that is no uniform's is GL_INVALID_OPERATION; with no program in
/// use every setter on a real location is GL_INVALID_OPERATION. No value
/// changes.
#[test]
fn location_minus_one_is_ignored_and_other_bad_locations_are_refused() {
    let _g = context();
    let p = program(VS100, FS100);
    let (f1, i, m4) = (loc(p, c"f1"), loc(p, c"i1"), loc(p, c"m4"));
    // SAFETY: plain GL calls; the arrays hold what the setters read.
    unsafe {
        glUniform1f(f1, 2.5);
        glUniform1i(i, 9);
        assert_eq!(err(), GL_NO_ERROR);
        let m = [7.0f32; 16];
        glUniform1i(-1, 7);
        glUniform1f(-1, 7.0);
        glUniform2i(-1, 7, 7);
        glUniform4f(-1, 7.0, 7.0, 7.0, 7.0);
        glUniformMatrix4fv(-1, 1, 0, m.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "location -1 is ignored");
        glUniform1i(1000, 7);
        assert_eq!(err(), GL_INVALID_OPERATION, "a location past the program's");
        glUseProgram(0);
        assert_eq!(err(), GL_NO_ERROR);
        glUniform1i(i, 7);
        assert_eq!(err(), GL_INVALID_OPERATION, "no program in use");
        glUniform1f(f1, 7.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "no program in use");
        glUniformMatrix4fv(m4, 1, 0, m.as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "no program in use");
        glUseProgram(p);
        assert_eq!(err(), GL_NO_ERROR);
    }
    assert_eq!(get_f(p, f1, 1), [2.5]);
    assert_eq!(get_i(p, i, 1), [9]);
}
