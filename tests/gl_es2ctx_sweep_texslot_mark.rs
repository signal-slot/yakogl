// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0-context guard for c6b16f71 ("gl: clear a texture slot's render-into
//! mark when the slot is freed"), item es2ctx-guards-more.
//!
//! c6b16f71 makes `glDeleteTextures` clear `TARGET_USE` and `DEPTH_TARGET` of
//! the freed slot, so the next `glGenTextures` texture is not taken for a
//! render target by the V3D back end. Render-to-texture is ES 2.0 core (4.4.2
//! `glFramebufferTexture2D`), and so is the delete rule of 3.7.13: a texture
//! attached to the bound framebuffer is detached at once, one attached to
//! another framebuffer stays alive until that framebuffer lets go. What an
//! ES 2.0 context answers must not move:
//!
//! - a texture rendered into, deleted and made again in the same slot: the
//!   new texture has no render-into mark, and a framebuffer built on it is
//!   complete and can be cleared;
//! - an orphan (deleted while another framebuffer holds it) keeps its mark
//!   and its attachment (`FRAMEBUFFER_ATTACHMENT_OBJECT_NAME` still the old
//!   name, status complete) until the last framebuffer is deleted, then the
//!   slot is free and unmarked;
//! - no GL error on any of this.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test gl_es2ctx_sweep_texslot_mark
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test gl_es2ctx_sweep_texslot_mark
//! ```
//!
//! Compiled only with `report-es2`: an ES 3.0 build is not the context this
//! guards.

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
unsafe extern "C" fn b_kick() -> u32 {
    7
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
    name: c"es2ctx-sweep-texslot-mark".as_ptr() as *const c_char,
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
    kick_id: Some(b_kick),
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
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, 0);
    }
    drain();
    g
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

/// A 4x4 RGBA texture with storage, left bound.
fn new_texture() -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; the null pixels allocate storage only.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR);
    t
}

/// A framebuffer with `tex` as colour attachment 0, left bound.
fn new_fbo(tex: GLuint) -> GLuint {
    let mut f = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    f
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn attached_name() -> GLint {
    let mut v = -1;
    // SAFETY: plain GL call, room for one value.
    unsafe {
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
            &mut v,
        )
    };
    v
}

fn attached_type() -> GLint {
    let mut v = -1;
    // SAFETY: plain GL call, room for one value.
    unsafe {
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            &mut v,
        )
    };
    v
}

fn clear_red() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFlush();
    }
}

fn mark(tex: GLuint) -> u32 {
    // SAFETY: the current context is the one `context()` made.
    let c = yakogl::gl::context::current().expect("a current context");
    texture_target_use(c, tex as usize)
}

#[test]
fn a_texture_rendered_into_deleted_and_made_again_is_no_render_target() {
    let _g = context();
    let a = new_texture();
    let f = new_fbo(a);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    clear_red();
    assert_eq!(err(), GL_NO_ERROR);
    assert_ne!(mark(a), 0, "the clear into the attachment marks the texture");
    // ES 2.0 3.7.13: deleting the texture of the bound framebuffer detaches it
    // SAFETY: plain GL calls.
    unsafe { glDeleteTextures(1, &a) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(attached_type(), GL_NONE as GLint, "detached from the bound framebuffer");
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    let b = new_texture();
    assert_eq!(b, a, "glGenTextures hands out the freed slot again");
    assert_eq!(mark(b), 0, "the new texture carries no render-into mark");
    // the new texture is an ordinary attachment: complete, clearable
    // SAFETY: plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, b, 0) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    clear_red();
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &f);
        glDeleteTextures(1, &b);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn an_orphan_keeps_its_attachment_and_mark_until_its_last_framebuffer_goes() {
    let _g = context();
    let a = new_texture();
    let f1 = new_fbo(a);
    let f2 = new_fbo(a); // f2 is bound, f1 holds the texture too
    clear_red();
    assert_eq!(err(), GL_NO_ERROR);
    let m = mark(a);
    assert_ne!(m, 0);
    // ES 2.0 3.7.13: detached from the bound f2, still held by f1, so alive
    // SAFETY: plain GL calls.
    unsafe { glDeleteTextures(1, &a) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(attached_type(), GL_NONE as GLint, "the bound framebuffer lost it");
    assert_eq!(mark(a), m, "the orphan is still f1's render target");
    // SAFETY: plain GL calls.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f1) };
    assert_eq!(attached_name(), a as GLint, "f1 still holds the deleted name");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &f1);
        glDeleteFramebuffers(1, &f2);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(mark(a), 0, "freed with its last framebuffer");
    let b = new_texture();
    assert_eq!(b, a);
    assert_eq!(mark(b), 0);
    // SAFETY: plain GL call.
    unsafe { glDeleteTextures(1, &b) };
    assert_eq!(err(), GL_NO_ERROR);
}
