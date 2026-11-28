// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 4.4.2.1 glRenderbufferStorageMultisample: how a requested sample
//! count is rounded and capped, on the 4x stand-in back end (`max_samples` 4)
//! of tests/es3_msaa_gl.rs (its helpers are copied, not shared). No draw is
//! made; every call is followed by GL_RENDERBUFFER_WIDTH / HEIGHT /
//! INTERNAL_FORMAT / SAMPLES.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_gl_rbsamples
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
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

/// The stand-in back end that keeps 4 samples (`max_samples` 4). It draws
/// nothing: these tests only query state.
static BACKEND_MS4: YakoglBackend = YakoglBackend {
    name: c"es3-msaa-gl-query".as_ptr() as *const c_char,
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

/// The context with the 4x stand-in; the tests take turns on it.
fn context_ms4() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND_MS4;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a static table; plain GL calls.
    unsafe {
        backend::g_backend = &BACKEND_MS4;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn rbp(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out; a renderbuffer is bound.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "pname {pname:#x}");
    v
}

/// (width, height, internal format, samples) of the bound renderbuffer.
fn shape() -> (GLint, GLint, GLint, GLint) {
    (
        rbp(GL_RENDERBUFFER_WIDTH),
        rbp(GL_RENDERBUFFER_HEIGHT),
        rbp(GL_RENDERBUFFER_INTERNAL_FORMAT),
        rbp(GL_RENDERBUFFER_SAMPLES),
    )
}

fn bound_rb() -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
    }
    rb
}

fn store(samples: GLsizei, format: GLenum, w: GLsizei, h: GLsizei) {
    // SAFETY: plain GL call.
    unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w, h) };
}

/// The counts glGetInternalformativ lists for GL_RENDERBUFFER and `format`.
fn listed(format: GLenum) -> Vec<GLint> {
    let mut n: GLint = -1;
    // SAFETY: one GLint out.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, GL_NUM_SAMPLE_COUNTS, 1, &mut n) };
    assert_eq!(err(), GL_NO_ERROR);
    let mut v = vec![-1; n.max(0) as usize];
    if n > 0 {
        // SAFETY: n GLints out.
        unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, GL_SAMPLES, n, v.as_mut_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
    }
    v
}

fn del(rb: GLuint) {
    // SAFETY: a name this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &rb);
    }
}

const FORMATS: [GLenum; 4] = [GL_RGBA8, GL_RGB565, GL_DEPTH_COMPONENT16, GL_DEPTH24_STENCIL8];

#[test]
fn max_samples_is_four() {
    let _g = context_ms4();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/// samples 0 is plain storage: GL_RENDERBUFFER_SAMPLES reads 0.
#[test]
fn zero_samples_is_single_sample() {
    let _g = context_ms4();
    for f in FORMATS {
        let rb = bound_rb();
        store(0, f, 13, 7);
        assert_eq!(err(), GL_NO_ERROR, "{f:#x}");
        assert_eq!(shape(), (13, 7, f as GLint, 0), "{f:#x}");
        del(rb);
    }
}

/// samples 1..=4: the smallest listed count >= the request.
#[test]
fn requested_samples_round_up_to_a_listed_count() {
    let _g = context_ms4();
    for f in FORMATS {
        let counts = listed(f);
        assert!(!counts.is_empty(), "{f:#x}: no listed counts");
        for req in 1..=4 {
            let rb = bound_rb();
            store(req, f, 9, 11);
            assert_eq!(err(), GL_NO_ERROR, "{f:#x} samples {req}");
            let (w, h, fmt, s) = shape();
            assert_eq!((w, h, fmt), (9, 11, f as GLint), "{f:#x} samples {req}");
            assert!(s >= req, "{f:#x}: asked {req}, got {s}");
            assert!(counts.contains(&s), "{f:#x}: {s} is not in {counts:?}");
            let want = counts.iter().copied().filter(|&c| c >= req).min().unwrap();
            assert_eq!(s, want, "{f:#x}: asked {req}: not the smallest listed count");
            del(rb);
        }
    }
}

/// samples > GL_MAX_SAMPLES: GL_INVALID_OPERATION, the earlier storage stays.
#[test]
fn more_than_max_samples_is_invalid_operation_and_keeps_storage() {
    let _g = context_ms4();
    let max = geti(GL_MAX_SAMPLES);
    for first in [0, 4] {
        let rb = bound_rb();
        store(first, GL_RGBA8, 5, 6);
        assert_eq!(err(), GL_NO_ERROR);
        let before = shape();
        for over in [max + 1, max + 4, 1000, GLsizei::MAX] {
            store(over, GL_RGBA8, 21, 22);
            assert_eq!(err(), GL_INVALID_OPERATION, "samples {over}");
            assert_eq!(shape(), before, "samples {over} changed the storage");
        }
        del(rb);
    }
}

/// A negative count is GL_INVALID_VALUE, the earlier storage stays.
#[test]
fn negative_samples_is_invalid_value_and_keeps_storage() {
    let _g = context_ms4();
    let rb = bound_rb();
    store(4, GL_RGB565, 5, 6);
    assert_eq!(err(), GL_NO_ERROR);
    let before = shape();
    for neg in [-1, -4, GLsizei::MIN] {
        store(neg, GL_RGB565, 21, 22);
        assert_eq!(err(), GL_INVALID_VALUE, "samples {neg}");
        assert_eq!(shape(), before, "samples {neg} changed the storage");
    }
    del(rb);
}

/// A fresh renderbuffer refused its first storage stays at its defaults.
#[test]
fn refused_first_storage_leaves_defaults() {
    let _g = context_ms4();
    let rb = bound_rb();
    store(5, GL_RGBA8, 8, 8);
    assert_eq!(err(), GL_INVALID_OPERATION);
    store(-1, GL_RGBA8, 8, 8);
    assert_eq!(err(), GL_INVALID_VALUE);
    assert_eq!(shape(), (0, 0, GL_RGBA4 as GLint, 0));
    del(rb);
}

/// Respecifying changes the count both ways and the size with it.
#[test]
fn respecified_storage_takes_the_new_count() {
    let _g = context_ms4();
    let rb = bound_rb();
    store(4, GL_RGBA8, 8, 8);
    assert_eq!(err(), GL_NO_ERROR);
    assert!(rbp(GL_RENDERBUFFER_SAMPLES) >= 4);
    store(0, GL_RGBA8, 3, 2);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(shape(), (3, 2, GL_RGBA8 as GLint, 0));
    store(2, GL_DEPTH_COMPONENT16, 4, 4);
    assert_eq!(err(), GL_NO_ERROR);
    let (w, h, f, s) = shape();
    assert_eq!((w, h, f), (4, 4, GL_DEPTH_COMPONENT16 as GLint));
    assert!(s >= 2);
    del(rb);
}
