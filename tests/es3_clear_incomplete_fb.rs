// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glClear and glClearBuffer{iv,uiv,fv,fi} on a framebuffer object that is not
//! framebuffer-complete (ES 3.0.6 4.4.4: the call generates
//! GL_INVALID_FRAMEBUFFER_OPERATION and changes nothing). Only conditions that
//! ES 3.0 calls incomplete are used (it dropped the dimensions rule):
//!
//! - no attachment at all (GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
//! - a texture level that does not exist (GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
//! - a texture of a format that is not colour-renderable, RGBA8_SNORM
//!   (GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT).
//!
//! Every case is checked with each of the five calls. The control cases run
//! the same calls on a complete framebuffer and expect GL_NO_ERROR.
//!
//! The contents cannot be observed: the stand-in back end draws nothing, and a
//! read from an incomplete framebuffer is an error itself. "Changes nothing"
//! is therefore checked only as far as the error code goes.
//!
//! The cases that fail on today's code are `#[ignore]`d for the plan item
//! `fix-clear-incomplete` (src/gl/state.rs glClear, src/gl/es3.rs): run them
//! with `-- --ignored` to see them fail.
//!
//! Not in a `report-es2` build: glClearBuffer* is ES 3.0 only.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_clear_incomplete_fb
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_clear_incomplete_fb
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_clear_incomplete_fb
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glClear, glGetError};
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
    name: c"es3-clear-incomplete-fb".as_ptr() as *const c_char,
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
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A framebuffer object bound to GL_FRAMEBUFFER; deleted with the default
/// framebuffer bound on drop.
struct Fbo {
    fb: GLuint,
    tex: GLuint,
}

impl Fbo {
    fn new() -> Fbo {
        let mut fb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(err(), GL_NO_ERROR);
        Fbo { fb, tex: 0 }
    }

    /// A 32 x 32 level-0 texture of `internalformat`, attached at `level` of
    /// colour 0.
    fn texture(&mut self, internalformat: GLenum, format: GLenum, type_: GLenum, level: GLint) {
        // SAFETY: room for one name; no pixels; plain GL calls.
        unsafe {
            glGenTextures(1, &mut self.tex);
            glBindTexture(GL_TEXTURE_2D, self.tex);
            glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 32, 32, 0, format, type_, core::ptr::null());
            assert_eq!(glGetError(), GL_NO_ERROR, "glTexImage2D");
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, self.tex, level);
            assert_eq!(glGetError(), GL_NO_ERROR, "glFramebufferTexture2D");
        }
    }

    fn status(&self) -> GLenum {
        // SAFETY: plain GL call.
        unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made (0 is ignored).
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

/* ---- the five calls: each returns the error it left ---------------------- */

fn call_clear() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) };
    err()
}
fn call_iv() -> GLenum {
    let v: [GLint; 4] = [1, 2, 3, 4];
    // SAFETY: four GLints.
    unsafe { glClearBufferiv(GL_COLOR, 0, v.as_ptr()) };
    err()
}
fn call_uiv() -> GLenum {
    let v: [GLuint; 4] = [1, 2, 3, 4];
    // SAFETY: four GLuints.
    unsafe { glClearBufferuiv(GL_COLOR, 0, v.as_ptr()) };
    err()
}
fn call_fv() -> GLenum {
    let v: [GLfloat; 4] = [0.25, 0.5, 0.75, 1.0];
    // SAFETY: four GLfloats.
    unsafe { glClearBufferfv(GL_COLOR, 0, v.as_ptr()) };
    err()
}
fn call_fi() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glClearBufferfi(GL_DEPTH_STENCIL, 0, 1.0, 0) };
    err()
}

/* ---- the incomplete framebuffers ----------------------------------------- */

/// No attachment: GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT.
fn missing_attachment() -> Fbo {
    let f = Fbo::new();
    assert_eq!(f.status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    f
}

/// Level 1 of a texture that has only level 0.
fn missing_level() -> Fbo {
    let mut f = Fbo::new();
    f.texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 1);
    assert_eq!(f.status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    f
}

/// A texture of a format ES 3.0 does not make colour-renderable.
fn unrenderable_format() -> Fbo {
    let mut f = Fbo::new();
    f.texture(GL_RGBA8_SNORM, GL_RGBA, GL_BYTE, 0);
    assert_eq!(f.status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    f
}

/// One ignored test per (case, call): the call must leave
/// GL_INVALID_FRAMEBUFFER_OPERATION on the framebuffer `$setup` builds.
macro_rules! incomplete_cases {
    ($setup:ident, $reason:literal: $( $name:ident => $call:ident ),+ $(,)?) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {
                let _g = context();
                let _f = $setup();
                assert_eq!($call(), GL_INVALID_FRAMEBUFFER_OPERATION);
            }
        )+
    };
}

/* A few of these pass in a whole-file `--ignored` run (a pending clear left by
 * an earlier case answers for them) but fail alone: none is a control. */
incomplete_cases!(missing_attachment, "fix-clear-incomplete: ES 3.0 4.4.4 wants GL_INVALID_FRAMEBUFFER_OPERATION":
    missing_attachment_clear => call_clear,
    missing_attachment_clear_buffer_iv => call_iv,
    missing_attachment_clear_buffer_uiv => call_uiv,
    missing_attachment_clear_buffer_fv => call_fv,
    missing_attachment_clear_buffer_fi => call_fi,
);
/* These also fail earlier: glCheckFramebufferStatus answers COMPLETE for
 * level 1 of a texture that has only level 0 (a separate cause, see the
 * report of this item). */
incomplete_cases!(missing_level, "fix-clear-incomplete: level 1 of a level-0-only texture reads as complete; ES 3.0 4.4.4 wants GL_INVALID_FRAMEBUFFER_OPERATION":
    missing_level_clear => call_clear,
    missing_level_clear_buffer_iv => call_iv,
    missing_level_clear_buffer_uiv => call_uiv,
    missing_level_clear_buffer_fv => call_fv,
    missing_level_clear_buffer_fi => call_fi,
);
incomplete_cases!(unrenderable_format, "fix-clear-incomplete: ES 3.0 4.4.4 wants GL_INVALID_FRAMEBUFFER_OPERATION":
    unrenderable_format_clear => call_clear,
    unrenderable_format_clear_buffer_iv => call_iv,
    unrenderable_format_clear_buffer_uiv => call_uiv,
    unrenderable_format_clear_buffer_fv => call_fv,
    unrenderable_format_clear_buffer_fi => call_fi,
);

/* ---- the control: complete framebuffers ----------------------------------- */

/// The calls on a complete RGBA8 framebuffer. The integer forms are left out
/// (an RGBA8 target is a fixed-point buffer: GL_INVALID_OPERATION, ES 3.0
/// 4.2.3), and glClearBufferfi has no depth-stencil buffer to clear here.
#[test]
fn complete_framebuffer_clears_without_error() {
    let _g = context();
    let mut f = Fbo::new();
    f.texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 0);
    assert_eq!(f.status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(call_clear(), GL_NO_ERROR);
    assert_eq!(call_fv(), GL_NO_ERROR);
}

/// The default framebuffer is complete.
#[test]
fn default_framebuffer_clears_without_error() {
    let _g = context();
    assert_eq!(call_clear(), GL_NO_ERROR);
    assert_eq!(call_fv(), GL_NO_ERROR);
    assert_eq!(call_fi(), GL_NO_ERROR);
}
