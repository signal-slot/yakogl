// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glClear and glClearBuffer{fv,iv,uiv} on the 16F colour targets (R16F,
//! RG16F, RGBA16F) as the tip answers them (plan #360, item clear16f-test).
//!
//! ES 3.0.6 4.2.3: the float form clears a fixed- or floating-point draw
//! buffer, the integer forms are `GL_INVALID_OPERATION` on it. The 16F
//! targets are complete on every GPU today (`GL_EXT_color_buffer_half_float`
//! is claimed), so a clear on one must stay `GL_NO_ERROR` when plan #360's
//! float-target claim lands.
//!
//! GL core, the same on every GPU, so the file runs in each ES 3.0 host build:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_half_float_clearbuffer
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_clearbuffer
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_half_float_clearbuffer
//! ```
//!
//! The stand-in back end draws nothing, so there is no pixel to read back:
//! the cleared value is checked where the GL core hands it over, the
//! `GlesDraw::clear_rgba` of the flush's clear record (the half rounding is
//! the back end's; tests/es3_clearbuffer_formats_v3d.rs has the V3D's).
//! glReadPixels is pinned as to its error only.
//!
//! Not in a `report-es2` build: ES 2.0 has none of the sized float names.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glClear, glClearColor, glFinish, glGetError, glGetFloatv, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host ---------------------------------------------- */

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

/// The clears the back end was handed: (clear_rgba, clear_target_mask).
static DRAWS: Mutex<Vec<([f32; 4], u8)>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core passes a live draw record.
    let d = unsafe { &*d };
    if d.do_clear {
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push((d.clear_rgba, d.clear_target_mask));
    }
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
    name: c"es3-half-float-clearbuffer".as_ptr() as *const c_char,
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
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const NULL: *const c_void = core::ptr::null();

const GPU: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "vivante"
} else if cfg!(feature = "gpu-v3d") {
    "v3d"
} else {
    "other"
};

const FORMATS: [(GLenum, GLenum, &str); 3] =
    [(GL_R16F, GL_RED, "R16F"), (GL_RG16F, GL_RG, "RG16F"), (GL_RGBA16F, GL_RGBA, "RGBA16F")];

/// A complete framebuffer with a 16 x 16 `internalformat` texture as colour 0,
/// bound; returns (framebuffer, texture).
fn target(internalformat: GLenum, format: GLenum, label: &str) -> (GLuint, GLuint) {
    let (mut fb, mut t) = (0, 0);
    // SAFETY: room for one name each; no pixels; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 16, 16, 0, format, GL_HALF_FLOAT, NULL);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} texture");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {label} attached");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {label} complete");
    }
    (fb, t)
}

fn release(fb: GLuint, t: GLuint) {
    // SAFETY: names made by `target`.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &t);
    }
}

/// Flush and return the clear records the back end saw since the last call.
fn flushed() -> Vec<([f32; 4], u8)> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    std::mem::take(&mut *DRAWS.lock().unwrap_or_else(|e| e.into_inner()))
}

#[test]
fn glclear_on_16f_targets_is_no_error() {
    let _g = context();
    for (ifmt, fmt, label) in FORMATS {
        let (fb, t) = target(ifmt, fmt, label);
        flushed();
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: glClear on {label}");
        let seen = flushed();
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: flush of the {label} clear");
        assert!(
            seen.iter().any(|(rgba, _)| *rgba == [0.25, 0.5, 0.75, 1.0]),
            "{GPU}: {label} clear colour reaches the back end: {seen:?}"
        );
        release(fb, t);
    }
}

#[test]
fn clearbufferfv_on_16f_targets_is_no_error() {
    let _g = context();
    // exact in a half; 2.0 and -1.0 are outside a fixed-point target's range
    let v: [GLfloat; 4] = [0.5, 2.0, -1.0, 0.25];
    for (ifmt, fmt, label) in FORMATS {
        let (fb, t) = target(ifmt, fmt, label);
        flushed();
        // SAFETY: four floats; plain GL call.
        unsafe { glClearBufferfv(GL_COLOR, 0, v.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: glClearBufferfv on {label}");
        let seen = flushed();
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: flush of the {label} clear");
        // The values reach the back end whole (no clamp in the GL core), and
        // glClearBuffer* leaves the context's clear colour alone. Tip: a
        // format with no alpha channel (R16F, RG16F) gets alpha 1 in the
        // record, on every GPU; blue is passed on as it is. Draw buffer 0 is
        // target mask 1.
        let mut want = v;
        if ifmt != GL_RGBA16F {
            want[3] = 1.0;
        }
        assert!(
            seen.iter().any(|(rgba, mask)| *rgba == want && *mask == 1),
            "{GPU}: {label} clearbuffer values reach the back end: {seen:?}"
        );
        let mut cc = [0.0f32; 4];
        // SAFETY: room for four floats.
        unsafe { glGetFloatv(GL_COLOR_CLEAR_VALUE, cc.as_mut_ptr()) };
        assert_ne!(cc, v, "{GPU}: {label} glClearBufferfv leaves GL_COLOR_CLEAR_VALUE");
        release(fb, t);
    }
}

#[test]
fn clearbufferiv_uiv_on_16f_targets_is_invalid_operation() {
    let _g = context();
    let iv: [GLint; 4] = [1, 2, 3, 4];
    let uv: [GLuint; 4] = [1, 2, 3, 4];
    for (ifmt, fmt, label) in FORMATS {
        let (fb, t) = target(ifmt, fmt, label);
        flushed();
        // SAFETY: four integers each; plain GL calls.
        unsafe {
            glClearBufferiv(GL_COLOR, 0, iv.as_ptr());
            assert_eq!(err(), GL_INVALID_OPERATION, "{GPU}: glClearBufferiv on {label}");
            glClearBufferuiv(GL_COLOR, 0, uv.as_ptr());
            assert_eq!(err(), GL_INVALID_OPERATION, "{GPU}: glClearBufferuiv on {label}");
        }
        let seen = flushed();
        assert!(seen.is_empty(), "{GPU}: a refused clear on {label} clears nothing: {seen:?}");
        release(fb, t);
    }
}

#[test]
fn readpixels_after_clear_16f() {
    let _g = context();
    for (ifmt, fmt, label) in FORMATS {
        let (fb, t) = target(ifmt, fmt, label);
        let v: [GLfloat; 4] = [0.5, 0.25, 0.125, 1.0];
        let mut px = vec![0.0f32; 16 * 16 * 4];
        // SAFETY: four floats; room for 16 x 16 RGBA floats; plain GL calls.
        unsafe {
            glClearBufferfv(GL_COLOR, 0, v.as_ptr());
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: glClearBufferfv on {label}");
            glReadPixels(0, 0, 16, 16, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void);
        }
        // The stand-in draws nothing, so only the error is pinned: RGBA/FLOAT
        // is the pair ES 3.0 4.3.2 gives a floating-point colour buffer.
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: glReadPixels RGBA/FLOAT after clearing {label}");
        release(fb, t);
    }
}
