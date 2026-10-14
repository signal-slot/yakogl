// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 error rules, framebuffer part (item cts-tree-diff-readme; the audit
//! is docs/investigations/es2-error-rules.md, Table A "Framebuffers and
//! renderbuffers" rows 34-39 and section D).
//!
//! Only a `report-es2` build refuses; the ES 3.0 build keeps taking these
//! values. Run with
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_error_rules_fbo
//! ```

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es2-error-rules-fbo".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    reset();
    g
}

/// Back to the default framebuffer, no renderbuffer, empty error queue.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
    }
    drain();
}

fn drain() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

/// The first error; the queue is empty again afterwards.
fn err() -> GLenum {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    drain();
    e
}

/// A framebuffer bound to GL_FRAMEBUFFER and a renderbuffer bound to
/// GL_RENDERBUFFER with 16x16 RGBA4 storage.
fn fbo_with_renderbuffer() -> (GLuint, GLuint) {
    let (mut f, mut r) = (0, 0);
    // SAFETY: plain GL calls; one name each written.
    unsafe {
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
        glGenRenderbuffers(1, &mut r);
        glBindRenderbuffer(GL_RENDERBUFFER, r);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 16, 16);
    }
    assert_eq!(err(), GL_NO_ERROR);
    (f, r)
}

/* ---- row 34: targets ------------------------------------------------------ */

#[test]
fn row34_read_and_draw_framebuffer_are_invalid_enum() {
    let _g = context();
    let (f, r) = fbo_with_renderbuffer();
    for t in [GL_READ_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER] {
        // SAFETY: plain GL calls; the pointers hold room for one value.
        unsafe {
            glBindFramebuffer(t, f);
            assert_eq!(err(), GL_INVALID_ENUM, "glBindFramebuffer 0x{t:x}");
            assert_eq!(glCheckFramebufferStatus(t), 0);
            assert_eq!(err(), GL_INVALID_ENUM, "glCheckFramebufferStatus 0x{t:x}");
            glFramebufferRenderbuffer(t, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, r);
            assert_eq!(err(), GL_INVALID_ENUM, "glFramebufferRenderbuffer 0x{t:x}");
            let mut v = -1;
            glGetFramebufferAttachmentParameteriv(
                t,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
                &mut v,
            );
            assert_eq!(err(), GL_INVALID_ENUM, "glGetFramebufferAttachmentParameteriv 0x{t:x}");
            assert_eq!(v, -1);
        }
    }
}

#[test]
fn row34_framebuffer_still_works() {
    let _g = context();
    let (f, r) = fbo_with_renderbuffer();
    // SAFETY: plain GL calls.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, r);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let mut v = 0;
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            &mut v,
        );
        assert_eq!(v as GLenum, GL_RENDERBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/* ---- row 35: attachment point --------------------------------------------- */

#[test]
fn row35_depth_stencil_attachment_is_invalid_enum() {
    let _g = context();
    let (_f, r) = fbo_with_renderbuffer();
    let mut tex = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, 16, 16);
        // the packed format needs GL_OES_packed_depth_stencil, which is listed
        assert_eq!(err(), GL_NO_ERROR);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, r);
        assert_eq!(err(), GL_INVALID_ENUM, "glFramebufferRenderbuffer");
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 16, 16, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        assert_eq!(err(), GL_NO_ERROR);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(err(), GL_INVALID_ENUM, "glFramebufferTexture2D");
        let mut v = -1;
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_DEPTH_STENCIL_ATTACHMENT,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            &mut v,
        );
        assert_eq!(err(), GL_INVALID_ENUM, "glGetFramebufferAttachmentParameteriv");
        assert_eq!(v, -1);
        /* the separate points take the packed renderbuffer */
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, r);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, r);
        assert_eq!(err(), GL_NO_ERROR);
    }
}

/* ---- rows 36-39: renderbuffer formats ------------------------------------- */

fn storage_error(f: GLenum) -> GLenum {
    let (_f, _r) = fbo_with_renderbuffer();
    // SAFETY: plain GL call on the bound renderbuffer.
    unsafe { glRenderbufferStorage(GL_RENDERBUFFER, f, 16, 16) };
    err()
}

#[test]
fn row36_es3_only_renderbuffer_formats_are_invalid_enum() {
    let _g = context();
    for f in [GL_DEPTH_COMPONENT32F, GL_DEPTH32F_STENCIL8, GL_R32F] {
        assert_eq!(storage_error(f), GL_INVALID_ENUM, "format 0x{f:x}");
    }
}

#[test]
fn rows37_39_formats_outside_the_advertised_set_stay_invalid_enum() {
    let _g = context();
    for f in [GL_DEPTH_COMPONENT24, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGBA32F, GL_RGBA8UI, GL_R11F_G11F_B10F] {
        assert_eq!(storage_error(f), GL_INVALID_ENUM, "format 0x{f:x}");
    }
}

#[test]
fn es2_and_advertised_renderbuffer_formats_are_kept() {
    let _g = context();
    for f in [
        GL_RGBA4,
        GL_RGB5_A1,
        GL_RGB565,
        GL_DEPTH_COMPONENT16,
        GL_STENCIL_INDEX8,
        GL_RGB8,
        GL_RGBA8,
        GL_DEPTH24_STENCIL8,
    ] {
        assert_eq!(storage_error(f), GL_NO_ERROR, "format 0x{f:x}");
    }
}
