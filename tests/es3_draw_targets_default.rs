// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 (docs/es3-cts-failures.md): what a product build answers for its
//! draw targets, on every GPU: THREE, except the PowerVR's ES 3.0 report,
//! which answers FOUR since its four-target half landed (plan #325, item
//! mrt-frameload; `backend::DRAW_TARGETS`). Raising the answer of the
//! others is a later item's `test-knobs` knob (`backend::HOST_DRAW_TARGETS`,
//! the stand-in a host test sets); this file runs with that knob left UNSET
//! and pins, through the GL API, that a product build reaches no further
//! than its count (`PRODUCT` below, 3 or 4):
//!
//! - `backend::DRAW_TARGETS` and `backend::draw_targets()` are `PRODUCT`;
//! - GL_MAX_COLOR_ATTACHMENTS is `PRODUCT` in every build (3 in every ES 2.0
//!   report, the PowerVR's included), and GL_MAX_DRAW_BUFFERS is `PRODUCT`
//!   in the ES 3.0 report. The ES 2.0 report (`report-es2`) does not claim
//!   GL_EXT_draw_buffers, so its GL_MAX_DRAW_BUFFERS_EXT — the same token,
//!   0x8824 — is answered as GL_INVALID_ENUM: a refusal, never a 4;
//! - the colour attachment past `PRODUCT` is refused with the error the GL
//!   core gives (no attachment point this GL names: GL_INVALID_ENUM), and a
//!   glDrawBuffers of `PRODUCT + 1` on a framebuffer object is
//!   GL_INVALID_VALUE (ES 3.0 report, where the draw-buffer state is public
//!   at all);
//! - GLSL ES 3.00's `gl_MaxDrawBuffers` compiles to `PRODUCT`
//!   (`shaderc-online`, ES 3.0 report).
//!
//! The gate runs this binary in the powervr and vivante CONFIGS rows of
//! tools/check.sh (with and without `shaderc-online` and `report-es2`); the
//! lines below are what the V3D half is checked with, the same ones
//! tests/es3_mrt_gl.rs names.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_draw_targets_default
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_draw_targets_default
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_draw_targets_default
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_draw_targets_default
//! ```

#![cfg(feature = "host-test")]

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
    max_samples: 0,
};

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

/// The context, made once with `HOST_DRAW_TARGETS` unset — the product
/// answer; the guard keeps the tests of this file off the library
/// (single-threaded by contract) one at a time.
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

/// glGetIntegerv's answer and the error it raised: Ok(value) when it
/// answered, Err(error) when it refused (leaving `out` — never a 4 there).
fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetIntegerv(pname, &mut v);
        match glGetError() {
            GL_NO_ERROR => Ok(v),
            e => {
                assert_eq!(v, -777, "glGetIntegerv(0x{pname:x}) answered {v} and raised its error at once");
                Err(e)
            }
        }
    }
}

/// The draw targets of a product build: four in the PowerVR's ES 3.0
/// report, three everywhere else (until each back end's four-target half
/// lands).
const PRODUCT: GLint = if cfg!(all(feature = "gpu-powervr", not(feature = "report-es2"))) { 4 } else { 3 };

#[test]
fn the_product_build_answers_its_count() {
    let _g = context();
    assert_eq!(backend::DRAW_TARGETS as GLint, PRODUCT, "the back end's own count");
    assert_eq!(backend::draw_targets() as GLint, PRODUCT);
    assert_eq!(backend::HOST_DRAW_TARGETS.load(Ordering::Relaxed), 0);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(PRODUCT));
    /* the ES 3.0 report answers the query with the back end's count, and
     * GL_DRAW_BUFFERi stops with it (ES 3.0 6.1.14: i below
     * GL_MAX_DRAW_BUFFERS, past that GL_INVALID_ENUM) */
    #[cfg(not(feature = "report-es2"))]
    {
        assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(PRODUCT));
        assert_eq!(geti(GL_DRAW_BUFFER0 + PRODUCT as GLenum), Err(GL_INVALID_ENUM), "past GL_MAX_DRAW_BUFFERS");
    }
    /* GL_EXT_draw_buffers is not claimed in an ES 2.0 report, so its token
     * — GL_MAX_DRAW_BUFFERS_EXT, GL_MAX_DRAW_BUFFERS, 0x8824 — is refused,
     * as is GL_DRAW_BUFFERi; a refusal is not a 4 (gl/state.rs) */
    #[cfg(feature = "report-es2")]
    {
        assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Err(GL_INVALID_ENUM), "GL_MAX_DRAW_BUFFERS_EXT is not listed");
        for i in 0..4 {
            assert_eq!(geti(GL_DRAW_BUFFER0 + i), Err(GL_INVALID_ENUM), "GL_DRAW_BUFFER{i}");
        }
    }
}

/// The colour attachment past `PRODUCT` is no attachment point this GL
/// names, and `PRODUCT + 1` draw buffers are past the answer of
/// GL_MAX_DRAW_BUFFERS.
#[test]
fn the_target_past_the_count_is_refused() {
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
        let n = PRODUCT as GLenum;
        for i in 0..n {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_RENDERBUFFER, rb);
            assert_eq!(glGetError(), GL_NO_ERROR, "GL_COLOR_ATTACHMENT{i}");
        }
        // the points the build advertises complete the framebuffer
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + n, GL_RENDERBUFFER, rb);
        assert_eq!(glGetError(), GL_INVALID_ENUM, "GL_COLOR_ATTACHMENT{n}");
        /* the draw-buffer state is public in the ES 3.0 report only; one
         * more than the count on a framebuffer object is past
         * GL_MAX_DRAW_BUFFERS (ES 3.0 4.2.1, checked before the enums) */
        #[cfg(not(feature = "report-es2"))]
        {
            let bufs: [GLenum; 5] = core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum);
            glDrawBuffers(PRODUCT + 1, bufs.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "glDrawBuffers(PRODUCT + 1)");
            glDrawBuffers(PRODUCT, bufs.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "glDrawBuffers(PRODUCT)");
        }
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

/// `gl_MaxDrawBuffers` is the query's `PRODUCT`: an array of
/// `PRODUCT + 1 - gl_MaxDrawBuffers` elements has a positive size, one of
/// `gl_MaxDrawBuffers - PRODUCT` does not (GLSL ES 3.00 4.1.9: an array
/// size is a positive constant expression). The same shape tests/es3_mrt_gl.rs uses
/// to pin the other side, the stand-in's 4.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
#[test]
fn gl_max_draw_buffers_is_the_count() {
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
            glDeleteShader(s);
            ok == 1
        }
    };
    let one = format!("const float a[{} - gl_MaxDrawBuffers] = float[1](1.0);", PRODUCT + 1);
    assert!(compiles(&one), "gl_MaxDrawBuffers is not {PRODUCT}");
    let none = format!("const float a[gl_MaxDrawBuffers - {PRODUCT}] = float[1](1.0);");
    assert!(!compiles(&none), "gl_MaxDrawBuffers - {PRODUCT} is an array size");
}
