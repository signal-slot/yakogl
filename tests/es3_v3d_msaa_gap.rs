// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D build has no multisample renderbuffers (plan #380, issue #1261;
//! images M1, FB4, FB1, R1, S10 and SQ of tools/cts/pi4-gles3-tree.txt: 67
//! Fail and 35 NotSupported cases). ES 3.0 needs GL_MAX_SAMPLES >= 4; the build
//! says 1.
//!
//! The cause is in the GL core, not in src/v3d: `msaa::backend_max_samples`
//! (src/gl/msaa.rs:29) returns 0 for every back end ("gl-msaa-cap" is not
//! built; `YakoglBackend` has no `max_samples` field). From it:
//!
//! * GL_MAX_SAMPLES is `max_samples_query(0)`, 1 (src/gl/msaa.rs:34,
//!   src/gl/state.rs:1307);
//! * glRenderbufferStorageMultisample with samples 2.. is GL_INVALID_OPERATION
//!   (`stored_samples`, src/gl/msaa.rs:53, called from src/gl/framebuffer.rs:3554),
//!   samples 1 keeps one sample (GL_RENDERBUFFER_SAMPLES 1, framebuffer.rs:2791),
//!   samples 0 is plain storage;
//! * glGetInternalformativ lists the one count 1 for every non-integer format
//!   and none for the integer ones (`internalformat_sample_counts`,
//!   src/gl/msaa.rs:97-110, called from src/gl/framebuffer.rs:3894).
//!
//! What this pins, through the GL entry points over a stand-in back end (the
//! rig of tests/es3_rgba32f_v3d.rs; the V3D build is only the feature set).
//! The tests whose names end `_flip_when_v3d_msaa_lands` are what the fix
//! flips (max 4, counts [4], 4 samples accepted); the control stays.
//! What it does not show: the dEQP picture or the blit from a multisampled
//! framebuffer.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_msaa_gap
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and a recording back end ---- */

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
unsafe extern "C" fn b_ok_draw(_d: *const GlesDraw) -> i32 {
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

/// A recording stand-in. The storage class the GL core picks does not depend
/// on the layout answers; the sampled classes are the V3D's (`gpu-v3d`).
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-v3d-msaa-gap".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_ok_draw),
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

/// One context for the whole file; the tests take turns on it.
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
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn max_samples() -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call, one value.
    unsafe { glGetIntegerv(GL_MAX_SAMPLES, &mut v) };
    v
}

/// The sample counts glGetInternalformativ lists for a renderbuffer format:
/// (GL_NUM_SAMPLE_COUNTS, GL_SAMPLES).
fn counts(format: GLenum) -> (GLint, Vec<GLint>) {
    let mut n: GLint = -1;
    let mut v = [-1 as GLint; 8];
    // SAFETY: plain GL calls; `v` has room for the 8 asked.
    unsafe {
        glGetInternalformativ(GL_RENDERBUFFER, format, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
        glGetInternalformativ(GL_RENDERBUFFER, format, GL_SAMPLES, 8, v.as_mut_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "{format:#x}");
    let list = v.iter().copied().take_while(|&s| s != -1).collect();
    (n, list)
}

/// glRenderbufferStorageMultisample on a fresh renderbuffer; the error it
/// raised and the GL_RENDERBUFFER_SAMPLES it stored.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLint) {
    let mut rb = 0;
    let mut stored: GLint = -1;
    // SAFETY: plain GL calls on a renderbuffer made here.
    let e = unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 16, 16);
        let e = glGetError();
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut stored);
        while glGetError() != GL_NO_ERROR {}
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &rb);
        e
    };
    (e, stored)
}

/// Stands for dEQP-GLES3 `functional.state_query.integers.max_samples*` and the
/// `fbo.msaa.*`, `rasterization.*` (num_samples) cases that skip or fail
/// without 4 samples: ES 3.0 6.2 has GL_MAX_SAMPLES at least 4. The core
/// reports 1 (src/gl/msaa.rs:34, 29; src/gl/state.rs:1307).
#[test]
fn max_samples_is_one_flip_when_v3d_msaa_lands() {
    let _g = context();
    assert_eq!(max_samples(), 1, "flip to 4");
}

/// Stands for `state_query.internal_format.*_samples` (SQ) and the R1/FB1
/// cases that read the list: RGBA8 and DEPTH24_STENCIL8 list the one count 1
/// (src/gl/msaa.rs:97-110, src/gl/framebuffer.rs:3894); with a 4x back end
/// they list [4] (ES 3.0 6.1.15, largest first).
#[test]
fn internalformat_sample_counts_are_just_one_flip_when_v3d_msaa_lands() {
    let _g = context();
    for f in [GL_RGBA8, GL_DEPTH24_STENCIL8] {
        assert_eq!(counts(f), (1, vec![1]), "{f:#x}: flip to (1, [4])");
    }
}

/// Stands for `negative_api.buffer.renderbuffer_storage_multisample`, the
/// M1/FB4 `fbo.msaa.*` renderbuffers and the blit cases that create a 4x
/// renderbuffer: samples 4 is GL_INVALID_OPERATION for RGBA8 and
/// DEPTH24_STENCIL8 and stores nothing (src/gl/msaa.rs:53,
/// src/gl/framebuffer.rs:3554-3560); flip to GL_NO_ERROR with 4 stored.
#[test]
fn four_samples_are_refused_flip_when_v3d_msaa_lands() {
    let _g = context();
    for f in [GL_RGBA8, GL_DEPTH24_STENCIL8] {
        let (e, stored) = storage(4, f);
        assert_eq!(e, GL_INVALID_OPERATION, "{f:#x}: flip to GL_NO_ERROR");
        assert_eq!(stored, 0, "{f:#x}: flip to 4");
    }
    let (e, _) = storage(2, GL_RGBA8);
    assert_eq!(e, GL_INVALID_OPERATION, "2 samples: flip to GL_NO_ERROR (stored 4)");
}

/// Stands for the integer-target cases of `negative_api` and
/// `fbo.msaa.*` that expect GL_INVALID_OPERATION for GL_RGBA8I with samples:
/// the count is refused today (src/gl/msaa.rs:53 `integer_format`) and
/// stays refused after a fix (ES 3.0 has no multisample integer storage), so
/// this one holds before and after.
#[test]
fn integer_format_samples_are_refused_control() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA8I).0, GL_INVALID_OPERATION);
    assert_eq!(counts(GL_RGBA8I), (0, vec![]));
}

/// The control that must hold before and after a fix: samples 0 is plain
/// storage with GL_RENDERBUFFER_SAMPLES 0 (src/gl/msaa.rs:50, `stored_samples`),
/// a negative count is GL_INVALID_VALUE, and 1 sample is accepted
/// (today it is kept as 1; the fix keeps it valid, the stored count may
/// change to 4, so only the error is pinned).
#[test]
fn zero_samples_is_plain_storage_control() {
    let _g = context();
    assert_eq!(storage(0, GL_RGBA8), (GL_NO_ERROR, 0));
    assert_eq!(storage(0, GL_DEPTH24_STENCIL8), (GL_NO_ERROR, 0));
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
    assert_eq!(storage(1, GL_RGBA8).0, GL_NO_ERROR);
}
