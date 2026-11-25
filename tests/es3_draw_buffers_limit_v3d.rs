// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D back end's draw-buffer and colour-attachment limit of 3 (plan 399,
//! item fr2-pin; #1261).
//!
//! `backend::DRAW_TARGETS` is 3 for the V3D, so the GL core answers
//! GL_MAX_DRAW_BUFFERS = GL_MAX_COLOR_ATTACHMENTS = 3 and a GLSL ES 3.00
//! fragment shader with four outputs (`layout(location = 0..3) out vec4`)
//! does not link: the dEQP `fragment_out` cases of Pi 4 image FR2 that use
//! four outputs end that way. ES 3.0 asks for a minimum of 4; the `#[ignore]`d
//! test holds that result for the mrt4 work (DRAW_TARGETS = 4 for the V3D).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_draw_buffers_limit_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-draw-buffers-limit".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context, on the build's own `DRAW_TARGETS` (3 for the V3D).
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Link a fragment shader of four `vec4` outputs at locations 0..3:
/// (link status, info log).
fn link_four_outputs() -> (GLint, String) {
    let fs = "#version 300 es\nprecision highp float;\n\
        layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
        layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
        void main() { o0 = vec4(0.0); o1 = vec4(1.0); o2 = vec4(2.0); o3 = vec4(3.0); }\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls; the log buffer is 1024 bytes.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut buf = [0 as c_char; 1024];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, 1024, &mut n, buf.as_mut_ptr());
        let bytes: Vec<u8> = buf[..n.max(0) as usize].iter().map(|&c| c as u8).collect();
        (ok, String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn limit(pname: GLenum) -> GLint {
    let mut n: GLint = 0;
    // SAFETY: plain GL call, one GLint.
    unsafe { glGetIntegerv(pname, &mut n) };
    n
}

/// `backend::DRAW_TARGETS` of the V3D is 3, for both queries.
#[test]
fn max_draw_buffers_and_color_attachments_read_3() {
    let _g = context();
    assert_eq!(backend::DRAW_TARGETS, 3);
    assert_eq!(limit(GL_MAX_DRAW_BUFFERS), 3);
    assert_eq!(limit(GL_MAX_COLOR_ATTACHMENTS), 3);
}

/// Four outputs do not link, and the log names the limit (dEQP FR2).
#[test]
fn four_fragment_outputs_fail_to_link_naming_the_draw_buffer_limit() {
    let _g = context();
    let (ok, log) = link_four_outputs();
    assert_eq!(ok, 0, "linked: {log:?}");
    assert!(log.contains("beyond the 3 draw buffers"), "{log:?}");
}

/// ES 3.0's minimum of 4, held for the mrt4 work (DRAW_TARGETS = 4 for the V3D).
#[test]
#[ignore = "mrt4: V3D DRAW_TARGETS is 3 until the mrt4 work raises it to 4 (docs/notes/v3d-fragment-out.md G5)"]
fn es3_minimum_of_four_draw_buffers_and_four_outputs_link() {
    let _g = context();
    assert_eq!(limit(GL_MAX_DRAW_BUFFERS), 4);
    assert_eq!(limit(GL_MAX_COLOR_ATTACHMENTS), 4);
    let (ok, log) = link_four_outputs();
    assert_eq!(ok, 1, "{log:?}");
}
