// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `GlesTextureBind::no_alpha` for the integer base formats: GL_RGB_INTEGER,
//! GL_RG_INTEGER and GL_RED_INTEGER have no alpha, so a look-up returns
//! alpha 1 (ES 3.0.6 3.8.15 table 3.24; docs/notes/gc-128bit-swizzle.md).
//! The bind the back end receives is observed on a stand-in back end that
//! records each draw's texture bindings and draws nothing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_no_alpha_int
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gl_no_alpha_int
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
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

/// The `no_alpha` of each binding of each draw since the last take.
static DRAWS: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    let binds = d.textures[..d.num_textures as usize].iter().map(|b| b.no_alpha).collect();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(binds);
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
    name: c"gl-no-alpha-int".as_ptr() as *const c_char,
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
        glActiveTexture(GL_TEXTURE0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";

fn program(sampler: &str, out: &str) {
    let fs = format!(
        "#version 300 es\nprecision mediump float;\nprecision mediump int;\nuniform highp {sampler} s;\nin vec2 v;\nout {out} o;\n\
         void main() {{ o = texture(s, v); }}\n"
    );
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let p = glCreateProgram();
        for (ty, text) in [(GL_VERTEX_SHADER, VS.to_string()), (GL_FRAGMENT_SHADER, fs)] {
            let text = CString::new(text).unwrap();
            let s = glCreateShader(ty);
            let src: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, src.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            if ok != 1 {
                let mut log = [0u8; 512];
                let mut len: GLsizei = 0;
                glGetShaderInfoLog(s, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char);
                panic!("compile: {}", String::from_utf8_lossy(&log[..len.max(0) as usize]));
            }
            glAttachShader(p, s);
        }
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

const QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// A 2x2 texture of `internal` uploaded with `format`/`type_`, bound on unit
/// 0 with NEAREST (integer textures are not filterable); one draw; the
/// `no_alpha` of its binding; None when this back end refuses the format.
fn no_alpha_of(internal: GLenum, format: GLenum, type_: GLenum, bytes: usize, sampler: &str, out: &str) -> Option<u8> {
    let px = vec![1u8; 4 * bytes];
    let mut name = 0;
    // SAFETY: plain GL calls; `px` holds the texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, format, type_, px.as_ptr() as *const c_void);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    let e = err();
    if e == GL_INVALID_OPERATION {
        /* a back end that samples none of the format's storage classes
         * refuses it by name (the GC7000UL's 32-bit integers, RC17) */
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
        return None;
    }
    assert_eq!(e, GL_NO_ERROR, "texture {internal:#x}");
    program(sampler, out);
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 16, 16);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glDeleteTextures(1, &name);
    }
    assert_eq!(err(), GL_NO_ERROR, "draw {internal:#x}");
    let d = core::mem::take(&mut *DRAWS.lock().unwrap_or_else(|e| e.into_inner()));
    assert_eq!(d.len(), 1, "one draw: {d:?}");
    assert_eq!(d[0].len(), 1, "one binding: {d:?}");
    Some(d[0][0])
}

/// `want` where the back end takes the format; a refused one (None) is that
/// back end's, counted so a back end cannot refuse them all.
fn check(got: Option<u8>, want: u8, what: &str, taken: &mut u32) {
    if let Some(v) = got {
        assert_eq!(v, want, "{what}");
        *taken += 1;
    }
}

#[test]
fn integer_base_formats_without_alpha_say_so() {
    let _g = context();
    let mut taken = 0;
    let got = no_alpha_of(GL_RGB32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT, 12, "usampler2D", "uvec4");
    check(got, 1, "RGB32UI", &mut taken);
    let got = no_alpha_of(GL_RG16I, GL_RG_INTEGER, GL_SHORT, 4, "isampler2D", "ivec4");
    check(got, 1, "RG16I", &mut taken);
    let got = no_alpha_of(GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, 1, "usampler2D", "uvec4");
    check(got, 1, "R8UI", &mut taken);
    assert!(taken >= 2, "RG16I and R8UI are sampled on every back end");
}

#[test]
fn formats_with_alpha_do_not() {
    let _g = context();
    let mut taken = 0;
    let got = no_alpha_of(GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, 4, "usampler2D", "uvec4");
    check(got, 0, "RGBA8UI", &mut taken);
    let got = no_alpha_of(GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, 4, "sampler2D", "vec4");
    check(got, 0, "RGBA", &mut taken);
    assert_eq!(taken, 2);
}
