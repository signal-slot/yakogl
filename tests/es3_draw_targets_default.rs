// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 (docs/es3-cts-failures.md): a product build answers THREE draw
//! targets, on every GPU. Only a later `test-knobs` knob may raise the
//! answer; this file runs with `backend::HOST_DRAW_TARGETS` left unset (0)
//! and pins, through the GL API:
//!
//! - `backend::DRAW_TARGETS` and `backend::draw_targets()` are 3;
//! - GL_MAX_COLOR_ATTACHMENTS is 3 (every build), GL_MAX_DRAW_BUFFERS is 3
//!   (ES 3.0 report) or GL_INVALID_ENUM (`report-es2`, where
//!   GL_EXT_draw_buffers is not claimed, so GL_MAX_DRAW_BUFFERS_EXT, the
//!   same token, is refused too), and neither is ever 4;
//! - a fourth colour attachment is refused with the error the GL core gives
//!   today (GL_INVALID_ENUM), and glDrawBuffers of four on a framebuffer
//!   object is GL_INVALID_VALUE (ES 3.0 report);
//! - GLSL ES 3.00's `gl_MaxDrawBuffers` compiles to 3 (`shaderc-online`,
//!   ES 3.0 report).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_draw_targets_default
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_draw_targets_default
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_draw_targets_default
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_draw_targets_default
//! ```

#![cfg(feature = "host-test")]
// the ES 2.0 report and builds without the compiler leave helpers idle
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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
    name: c"es3-draw-targets-default".as_ptr() as *const c_char,
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
    flush: None,
    finish: None,
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

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

/// The context, made once, with `HOST_DRAW_TARGETS` unset; the guard keeps
/// the tests of this file off the library (single-threaded by contract) one
/// at a time.
fn context() -> std::sync::MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
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
    assert_eq!(backend::HOST_DRAW_TARGETS.load(Ordering::Relaxed), 0, "this file runs with the stand-in unset");
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    guard
}

/// glGetIntegerv's answer, or its error.
fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetIntegerv(pname, &mut v);
        match glGetError() {
            GL_NO_ERROR => Ok(v),
            e => Err(e),
        }
    }
}

#[test]
fn the_build_answers_three() {
    let _g = context();
    assert_eq!(backend::DRAW_TARGETS, 3);
    assert_eq!(backend::draw_targets(), 3);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(3));
    #[cfg(not(feature = "report-es2"))]
    {
        assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(3));
        assert_eq!(geti(GL_DRAW_BUFFER0 + 3), Err(GL_INVALID_ENUM), "past GL_MAX_DRAW_BUFFERS");
    }
    // GL_EXT_draw_buffers is not claimed in an ES 2.0 report: its token is
    // GL_MAX_DRAW_BUFFERS (0x8824), refused, never a 4
    #[cfg(feature = "report-es2")]
    {
        assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Err(GL_INVALID_ENUM));
        assert_eq!(geti(GL_DRAW_BUFFER0), Err(GL_INVALID_ENUM));
    }
    assert_eq!(backend::HOST_DRAW_TARGETS.load(Ordering::Relaxed), 0);
}

/// A fourth colour attachment is not an attachment point.
#[test]
fn a_fourth_colour_attachment_is_refused() {
    let _g = context();
    // SAFETY: plain GL calls; names made here.
    unsafe {
        let (mut fb, mut rb) = (0, 0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        while glGetError() != GL_NO_ERROR {}
        for i in 0..3 {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_RENDERBUFFER, rb);
            assert_eq!(glGetError(), GL_NO_ERROR, "GL_COLOR_ATTACHMENT{i}");
        }
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 3, GL_RENDERBUFFER, rb);
        assert_eq!(glGetError(), GL_INVALID_ENUM, "GL_COLOR_ATTACHMENT3");
        #[cfg(not(feature = "report-es2"))]
        {
            let bufs: [GLenum; 4] = core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum);
            glDrawBuffers(4, bufs.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "glDrawBuffers(4)");
            glDrawBuffers(3, bufs.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "glDrawBuffers(3)");
        }
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

/// `gl_MaxDrawBuffers` is the query's 3: an array of `4 - gl_MaxDrawBuffers`
/// has a positive size, one of `gl_MaxDrawBuffers - 3` does not.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
#[test]
fn gl_max_draw_buffers_is_three() {
    use yakogl::gl::shader::*;
    let _g = context();
    let compiles = |decl: &str| {
        let fs = format!(
            "#version 300 es\nprecision mediump float;\nout vec4 o;\n{decl}\nvoid main() {{ o = vec4(a[0], 0.0, 0.0, 1.0); }}\n"
        );
        let text = std::ffi::CString::new(fs).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            ok == 1
        }
    };
    assert!(compiles("const float a[4 - gl_MaxDrawBuffers] = float[1](1.0);"), "gl_MaxDrawBuffers is not 3");
    assert!(!compiles("const float a[gl_MaxDrawBuffers - 3] = float[1](1.0);"), "gl_MaxDrawBuffers - 3 is a size");
}
