// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 2.12.6: glUniform{1,2,3,4}ui[v] loads uint, uvecN and bool
//! uniforms only, and a mismatch is GL_INVALID_OPERATION that leaves the
//! uniform alone (dEQP-GLES3 negative_api.shader.uniformui_incompatible_type
//! and uniformuiv_incompatible_type; docs/notes/es3-negative-api.md, 2).
//! A GLSL ES 1.00 program keeps the unsigned setter on an int uniform, the
//! deviation Qt's RHI relies on (`setter_accepts` in src/gl/program.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_negapi_uniform
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3::glGetUniformuiv;
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
    name: c"es3-negapi-uniform".as_ptr() as *const c_char,
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

/// The dEQP program (vec4_v, ivec4_f, uvec4_f, sampler_f) plus a bool, a
/// float and a uint, all read.
const VS300: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS300: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n\
    uniform vec4 vec4_v;\nuniform ivec4 ivec4_f;\nuniform uvec4 uvec4_f;\nuniform bvec4 bvec4_f;\n\
    uniform float float_f;\nuniform uint uint_f;\nuniform sampler2D sampler_f;\n\
    layout(location = 0) out vec4 o;\n\
    void main() { o = vec4_v + vec4(ivec4_f) + vec4(uvec4_f) + vec4(bvec4_f) + vec4(float_f)\n\
    + vec4(float(uint_f)) + texture(sampler_f, vec2(0.5)); }\n";

/// The errors of glUniform{n}ui and glUniform{n}uiv at `l`, as dEQP issues them.
fn ui_calls(l: GLint, n: usize) -> [GLenum; 2] {
    let d: [GLuint; 4] = [0; 4];
    // SAFETY: plain GL calls; `d` holds four values.
    unsafe {
        match n {
            1 => glUniform1ui(l, 0),
            2 => glUniform2ui(l, 0, 0),
            3 => glUniform3ui(l, 0, 0, 0),
            _ => glUniform4ui(l, 0, 0, 0, 0),
        }
        let a = err();
        match n {
            1 => glUniform1uiv(l, 1, d.as_ptr()),
            2 => glUniform2uiv(l, 1, d.as_ptr()),
            3 => glUniform3uiv(l, 1, d.as_ptr()),
            _ => glUniform4uiv(l, 1, d.as_ptr()),
        }
        [a, err()]
    }
}

fn ok_when(matches: bool) -> [GLenum; 2] {
    [if matches { GL_NO_ERROR } else { GL_INVALID_OPERATION }; 2]
}

#[test]
fn an_es300_program_refuses_the_unsigned_setters_on_other_types() {
    let _g = context();
    let p = program(VS300, FS300);
    let (v, i, u, b, f, u1, s) = (
        loc(p, c"vec4_v"),
        loc(p, c"ivec4_f"),
        loc(p, c"uvec4_f"),
        loc(p, c"bvec4_f"),
        loc(p, c"float_f"),
        loc(p, c"uint_f"),
        loc(p, c"sampler_f"),
    );
    for n in 1..=4 {
        assert_eq!(ui_calls(u, n), ok_when(n == 4), "uvec4 / {n}");
        assert_eq!(ui_calls(i, n), ok_when(false), "ivec4 / {n}");
        assert_eq!(ui_calls(v, n), ok_when(false), "vec4 / {n}");
        assert_eq!(ui_calls(b, n), ok_when(n == 4), "bvec4 / {n}: bool takes it, with its size");
        assert_eq!(ui_calls(f, n), ok_when(false), "float / {n}");
        assert_eq!(ui_calls(s, n), ok_when(false), "sampler / {n}");
        assert_eq!(ui_calls(u1, n), ok_when(n == 1), "uint / {n}");
    }
}

#[test]
fn a_refused_unsigned_setter_leaves_the_uniform_unchanged() {
    let _g = context();
    let p = program(VS300, FS300);
    let i = loc(p, c"ivec4_f");
    let first: [GLint; 4] = [1, 2, 3, 4];
    let mut got: [GLint; 4] = [0; 4];
    // SAFETY: plain GL calls; four-element arrays.
    unsafe {
        glUniform4iv(i, 1, first.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        glUniform4ui(i, 9, 9, 9, 9);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glGetUniformiv(p, i, got.as_mut_ptr());
        assert_eq!(err(), GL_NO_ERROR);
    }
    assert_eq!(got, first);
}

#[test]
fn matching_unsigned_setters_load_and_read_back() {
    let _g = context();
    let p = program(VS300, FS300);
    let u = loc(p, c"uvec4_f");
    let val: [GLuint; 4] = [5, 6, 7, 8];
    let mut got: [GLuint; 4] = [0; 4];
    // SAFETY: plain GL calls; four-element arrays.
    unsafe {
        glUniform4uiv(u, 1, val.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        glGetUniformuiv(p, u, got.as_mut_ptr());
        assert_eq!(err(), GL_NO_ERROR);
    }
    assert_eq!(got, val);
}

#[test]
fn an_es100_program_keeps_the_unsigned_setter_on_an_int_uniform() {
    let _g = context();
    let p = program(
        "attribute vec4 p;\nvoid main() { gl_Position = p; }\n",
        "precision mediump float;\nuniform int n;\nvoid main() { gl_FragColor = vec4(float(n)); }\n",
    );
    let n = loc(p, c"n");
    // SAFETY: plain GL call.
    unsafe { glUniform1ui(n, 3) };
    assert_eq!(err(), GL_NO_ERROR, "Qt's RHI sets an ES 1.00 int with the unsigned setter");
}
