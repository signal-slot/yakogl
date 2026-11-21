// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What an ES 2.0 context sees on a back end that says `max_samples` 4
//! (item gl-msaa-es2ctx-tests; docs/notes/es3-msaa-renderbuffers.md). The
//! table is the 4x stand-in of tests/es3_msaa_gl.rs; the context is the
//! `report-es2` build's, as in tests/gl_es2ctx_es3_only.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_msaa_gl_es2ctx
//! ```
//!
//! Compiled only with `report-es2`.
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
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
    name: c"es2ctx-msaa-gl".as_ptr() as *const c_char,
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
    max_samples: 4,
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
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> (GLenum, GLint) {
    let mut v: GLint = 0x5a5a;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    (err(), v)
}

/// A fresh renderbuffer, bound.
fn new_rb() -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
    }
    rb
}

fn rb_param(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// ES 2.0 has no GL_MAX_SAMPLES; the back end's 4 must not open the name.
#[test]
fn max_samples_is_invalid_enum_with_a_4x_back_end() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), (GL_INVALID_ENUM, 0x5a5a));
}

/// The 1x window: GL_SAMPLE_BUFFERS and GL_SAMPLES are 0, whatever the table says.
#[test]
fn window_samples_are_zero() {
    let _g = context();
    assert_eq!(geti(GL_SAMPLE_BUFFERS), (GL_NO_ERROR, 0));
    assert_eq!(geti(GL_SAMPLES), (GL_NO_ERROR, 0));
}

/// 1x renderbuffers still work: storage, parameters, a complete framebuffer,
/// and the framebuffer's GL_SAMPLES is 0. (No clear or read back: the
/// stand-in has no `draw`, so a clear is not carried out.)
#[test]
fn single_sample_renderbuffers_work() {
    let _g = context();
    let rb = new_rb();
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 8, 8);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!((rb_param(GL_RENDERBUFFER_WIDTH), rb_param(GL_RENDERBUFFER_HEIGHT)), (8, 8));
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(geti(GL_SAMPLES), (GL_NO_ERROR, 0));
    assert_eq!(geti(GL_SAMPLE_BUFFERS), (GL_NO_ERROR, 0));
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

/// ES 2.0 has no glBlitFramebuffer: GL_INVALID_OPERATION, nothing drawn.
#[test]
fn blit_framebuffer_is_invalid_operation() {
    let _g = context();
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(0, 0, 4, 4, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST) };
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/// Today: the ES 2.0 report has glRenderbufferStorageMultisample, and with a
/// 4x back end it succeeds for 0..=4 samples (GL_INVALID_OPERATION past 4), but
/// the stored count is not observable (GL_RENDERBUFFER_SAMPLES is no ES 2.0
/// name: GL_INVALID_ENUM). Pins today's behaviour; see the ignored test.
#[test]
fn multisample_storage_today_is_accepted() {
    let _g = context();
    for s in [0, 1, 4] {
        let rb = new_rb();
        // SAFETY: plain GL calls.
        unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, s, GL_RGBA4, 8, 8) };
        assert_eq!(err(), GL_NO_ERROR, "samples {s}");
        let mut v = -777;
        // SAFETY: one GLint out.
        unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v) };
        assert_eq!((err(), v), (GL_INVALID_ENUM, -777));
        // SAFETY: a name this test made.
        unsafe { glDeleteRenderbuffers(1, &rb) };
    }
    let rb = new_rb();
    // SAFETY: plain GL call.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 5, GL_RGBA4, 8, 8) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

/// Expected by ES 2.0 (no such command, as glBlitFramebuffer): a 4-sample
/// request must not hand out multisampled storage. A defect today, see the
/// report: the entry is not refused under `report-es2`.
#[test]
#[ignore = "defect: glRenderbufferStorageMultisample is not refused in an ES 2.0 context (glBlitFramebuffer is)"]
fn multisample_storage_is_refused_in_es2() {
    let _g = context();
    let rb = new_rb();
    // SAFETY: plain GL call.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA4, 8, 8) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}
