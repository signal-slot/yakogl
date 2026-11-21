// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context against 82dcceba (glClear on an incomplete framebuffer is
//! GL_INVALID_FRAMEBUFFER_OPERATION and records nothing) and 8428875e
//! (glReadPixels checks completeness before the format/type pair). Both are
//! in src/gl (state.rs `draw_framebuffer_complete`, framebuffer.rs
//! `read_pixels`) and were pinned only by ES 3.0 tests. Every dEQP-GLES2
//! case that clears or reads pixels goes through them. This drives the real
//! GL core in a `report-es2` build and asserts, in the ES 2.0 context:
//!
//! (a) glClear on an incomplete FBO (missing attachment, mismatched
//!     dimensions, an unrenderable LUMINANCE texture, a zero-size
//!     renderbuffer) is GL_INVALID_FRAMEBUFFER_OPERATION and submits nothing
//!     (no draw reaches the back end, not even at glFinish).
//! (b) glReadPixels on an incomplete FBO is GL_INVALID_FRAMEBUFFER_OPERATION.
//!     Order, from ES 2.0 4.3.1 and dEQP-GLES2 functional.negative_api.buffer
//!     read_pixels_* (invalid_enum / invalid_value on the default
//!     framebuffer) and fbo.completeness: a negative size is GL_INVALID_VALUE
//!     and comes first (also on an incomplete FBO); on a complete target an
//!     unknown format/type is GL_INVALID_ENUM and a known but unsupported
//!     pair GL_INVALID_OPERATION; an incomplete FBO with valid arguments is
//!     GL_INVALID_FRAMEBUFFER_OPERATION. No dEQP case combines a bad pair
//!     with an incomplete FBO and the spec leaves that order open; since
//!     8428875e completeness wins, before it the pair did. That one answer
//!     moved and is pinned as it is now.
//! (c) On complete RGBA8, RGB565, RGBA4 and RGB5_A1 FBOs and the default
//!     framebuffer, glReadPixels RGBA/UNSIGNED_BYTE is GL_NO_ERROR, and
//!     GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE is RGBA/UNSIGNED_BYTE.
//! (d) glClear on a complete FBO is no error. The stand-in back end executes
//!     nothing, so no pixel is compared; the clear is asserted by its error
//!     and by the draws it hands the back end (`clear_reaches_the_backend`).
//!
//! The GL core is the same under every back end; the stand-in back end is
//! the smallest that makes a context (as tests/gl_es2ctx_buffer_rename.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_incomplete_fb
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_incomplete_fb
//! cargo clippy --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_incomplete_fb -- -D warnings
//! cargo clippy --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_incomplete_fb -- -D warnings
//! ```
#![cfg(all(feature = "host-test", feature = "report-es2", any(feature = "gpu-vivante", feature = "gpu-v3d")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
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

static DRAWS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    DRAWS.fetch_add(1, Ordering::SeqCst);
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
    name: c"es2ctx-incomplete-fb".as_ptr() as *const c_char,
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
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn draws() -> usize {
    DRAWS.load(Ordering::SeqCst)
}

fn geti(pname: GLenum) -> GLint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn bind_fbo() -> GLuint {
    let mut fb = 0;
    // SAFETY: one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    fb
}

fn rb_attach(attachment: GLenum, format: GLenum, w: GLsizei, h: GLsizei) {
    let mut rb = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
    }
}

fn tex_attach(format: GLenum, ty: GLenum, w: GLsizei, h: GLsizei) {
    let mut t = 0;
    // SAFETY: one name; null pixels allocate storage.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, w, h, 0, format, ty, core::ptr::null());
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn read(w: GLsizei, h: GLsizei, format: GLenum, ty: GLenum) -> (GLenum, [u8; 16]) {
    let mut px = [0x5au8; 16];
    // SAFETY: room for up to 4 RGBA pixels; callers read at most 2x2.
    unsafe { glReadPixels(0, 0, w, h, format, ty, px.as_mut_ptr().cast::<c_void>()) };
    (err(), px)
}

/// glClear on the bound target: its error, and the draws that reached the
/// back end up to and including glFinish.
fn clear_and_finish(mask: GLbitfield) -> (GLenum, usize) {
    let before = draws();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(mask);
    }
    let e = err();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let late = err();
    assert_eq!(late, GL_NO_ERROR, "nothing more is raised at glFinish");
    (e, draws() - before)
}

#[test]
fn the_context_is_es2() {
    let _g = context();
    // SAFETY: plain GL call; the string is static.
    let v = unsafe { std::ffi::CStr::from_ptr(glGetString(GL_VERSION).cast::<c_char>()) };
    assert!(v.to_str().unwrap().starts_with("OpenGL ES 2.0"), "{v:?}");
}

/// Each of the four ES 2.0 ways a framebuffer is incomplete, bound for `f`.
fn incomplete_cases(f: impl Fn(&str)) {
    for case in ["missing attachment", "dimensions", "luminance texture", "zero-size renderbuffer"] {
        let _fb = bind_fbo();
        match case {
            "missing attachment" => {
                assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
            }
            "dimensions" => {
                rb_attach(GL_COLOR_ATTACHMENT0, GL_RGBA4, 4, 4);
                rb_attach(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, 8, 8);
                assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
            }
            "luminance texture" => {
                tex_attach(GL_LUMINANCE, GL_UNSIGNED_BYTE, 4, 4);
                assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
            }
            _ => {
                rb_attach(GL_COLOR_ATTACHMENT0, GL_RGBA4, 0, 0);
                assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{case}: setting up raised no error");
        f(case);
    }
}

#[test]
fn clear_on_an_incomplete_fbo_is_refused_and_submits_nothing() {
    let _g = context();
    incomplete_cases(|case| {
        let (e, d) = clear_and_finish(GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{case}");
        assert_eq!(d, 0, "{case}: nothing reaches the back end");
        let (e, d) = clear_and_finish(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{case}: all planes");
        assert_eq!(d, 0, "{case}: all planes submit nothing");
    });
}

#[test]
fn clear_mask_error_comes_before_completeness() {
    let _g = context();
    incomplete_cases(|case| {
        // SAFETY: plain GL call; a mask bit that is none of the three.
        unsafe { glClear(0x0000_0001) };
        assert_eq!(err(), GL_INVALID_VALUE, "{case}");
    });
}

#[test]
fn read_pixels_on_an_incomplete_fbo_is_refused() {
    let _g = context();
    incomplete_cases(|case| {
        let (e, px) = read(1, 1, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{case}");
        assert_eq!(px, [0x5a; 16], "{case}: nothing written");
        // a negative size is GL_INVALID_VALUE first, as dEQP negative_api
        // read_pixels_invalid_value expects
        let (e, _) = read(-1, 1, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_VALUE, "{case}: negative width");
        let (e, _) = read(1, -1, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_VALUE, "{case}: negative height");
        // the answer 8428875e moved: completeness before the pair
        let (e, _) = read(1, 1, 0x1234, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{case}: bad format, incomplete");
        let (e, _) = read(1, 1, GL_RGBA, GL_UNSIGNED_SHORT_5_6_5);
        assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{case}: odd pair, incomplete");
    });
}

/// The default framebuffer, then FBOs with each ES 2.0 colour format.
fn complete_targets(f: impl Fn(&str)) {
    f("default framebuffer");
    for (name, format) in [("RGBA4", GL_RGBA4), ("RGB5_A1", GL_RGB5_A1), ("RGB565", GL_RGB565)] {
        let _fb = bind_fbo();
        rb_attach(GL_COLOR_ATTACHMENT0, format, 4, 4);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
        assert_eq!(err(), GL_NO_ERROR);
        f(name);
    }
    let _fb = bind_fbo();
    tex_attach(GL_RGBA, GL_UNSIGNED_BYTE, 4, 4);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "RGBA8");
    f("RGBA8 texture");
}

#[test]
fn read_pixels_on_complete_targets_keeps_its_answers() {
    let _g = context();
    complete_targets(|case| {
        assert_eq!(geti(GL_IMPLEMENTATION_COLOR_READ_FORMAT), GL_RGBA as GLint, "{case}");
        assert_eq!(geti(GL_IMPLEMENTATION_COLOR_READ_TYPE), GL_UNSIGNED_BYTE as GLint, "{case}");
        assert_eq!(err(), GL_NO_ERROR, "{case}: queries");
        let (e, _) = read(2, 2, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_NO_ERROR, "{case}: RGBA/UNSIGNED_BYTE");
        let (e, _) = read(0, 0, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_NO_ERROR, "{case}: empty read");
        let (e, _) = read(1, 1, 0x1234, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_ENUM, "{case}: unknown format");
        let (e, _) = read(1, 1, GL_RGBA, 0x1234);
        assert_eq!(e, GL_INVALID_ENUM, "{case}: unknown type");
        let (e, _) = read(1, 1, GL_RGB, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_OPERATION, "{case}: RGB/UNSIGNED_BYTE");
        let (e, _) = read(1, 1, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4);
        assert_eq!(e, GL_INVALID_OPERATION, "{case}: RGBA/4_4_4_4");
        let (e, _) = read(-1, 1, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_INVALID_VALUE, "{case}: negative width");
    });
}

#[test]
fn clear_reaches_the_backend_on_complete_targets() {
    let _g = context();
    complete_targets(|case| {
        let (e, d) = clear_and_finish(GL_COLOR_BUFFER_BIT);
        assert_eq!(e, GL_NO_ERROR, "{case}");
        // the stand-in executes nothing; a clear is a draw or a pending clear,
        // never an error and never more than one draw
        assert!(d <= 1, "{case}: {d} draws for one clear");
        let (e, _) = read(1, 1, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_NO_ERROR, "{case}: read after the clear");
    });
}
