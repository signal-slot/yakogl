// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_TEXTURE_SRGB_DECODE_EXT (EXT_texture_sRGB_decode) in the GL core's
//! texture and sampler state: DECODE_EXT or SKIP_DECODE_EXT, default
//! DECODE_EXT, on both objects, the bound sampler's value overriding the
//! texture's. No back end reads it yet (a 7.1 table hooks it later).
//!
//! No build's extension list names the extension yet, so the cases that need
//! it exposed turn on the `host-test` knob `SRGB_DECODE_FORCED`.
//!
//! - `not_exposed_is_invalid_enum`: set and get on a texture and a sampler.
//! - `defaults_are_decode`: texture and sampler, and the bound view.
//! - `texture_set_get_roundtrip`: SKIP and DECODE, a bad value is INVALID_ENUM.
//! - `sampler_set_get_roundtrip`: the same on a sampler object.
//! - `bound_sampler_overrides_the_texture`: and unbinding restores it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_srgb_decode_state
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::state::glGetError;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"gl-srgb-decode-state".as_ptr() as *const c_char,
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

unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
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

static SERIAL: Mutex<()> = Mutex::new(());

fn on_fresh_context(exposed: bool, case: impl Fn()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    SRGB_DECODE_FORCED.store(exposed, Ordering::Relaxed);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    SRGB_DECODE_FORCED.store(false, Ordering::Relaxed);
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn new_texture() {
    let mut t: GLuint = 0;
    // SAFETY: one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
    }
}

fn new_sampler() -> GLuint {
    let mut s: GLuint = 0;
    // SAFETY: one name.
    unsafe { glGenSamplers(1, &mut s) };
    s
}

fn tex_get() -> (GLint, GLenum) {
    let mut v: GLint = -1;
    // SAFETY: one GLint.
    unsafe { glGetTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, &mut v) };
    (v, err())
}

fn smp_get(s: GLuint) -> (GLint, GLenum) {
    let mut v: GLint = -1;
    // SAFETY: one GLint.
    unsafe { glGetSamplerParameteriv(s, GL_TEXTURE_SRGB_DECODE_EXT, &mut v) };
    (v, err())
}

#[test]
fn not_exposed_is_invalid_enum() {
    on_fresh_context(false, || {
        new_texture();
        let s = new_sampler();
        assert_eq!(err(), GL_NO_ERROR);
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint) };
        assert_eq!(err(), GL_INVALID_ENUM);
        assert_eq!(tex_get().1, GL_INVALID_ENUM);
        // SAFETY: plain GL call.
        unsafe { glSamplerParameteri(s, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint) };
        assert_eq!(err(), GL_INVALID_ENUM);
        assert_eq!(smp_get(s).1, GL_INVALID_ENUM);
        assert_eq!(srgb_decode_bound_2d(0), GL_DECODE_EXT);
    });
}

#[test]
fn defaults_are_decode() {
    on_fresh_context(true, || {
        new_texture();
        let s = new_sampler();
        assert_eq!(tex_get(), (GL_DECODE_EXT as GLint, GL_NO_ERROR));
        assert_eq!(smp_get(s), (GL_DECODE_EXT as GLint, GL_NO_ERROR));
        assert_eq!(srgb_decode_bound_2d(0), GL_DECODE_EXT);
    });
}

#[test]
fn texture_set_get_roundtrip() {
    on_fresh_context(true, || {
        new_texture();
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint) };
        assert_eq!(tex_get(), (GL_SKIP_DECODE_EXT as GLint, GL_NO_ERROR));
        assert_eq!(srgb_decode_bound_2d(0), GL_SKIP_DECODE_EXT);
        // a second texture keeps its own value
        new_texture();
        assert_eq!(tex_get(), (GL_DECODE_EXT as GLint, GL_NO_ERROR));
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_NEAREST as GLint) };
        assert_eq!(err(), GL_INVALID_ENUM);
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint) };
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_DECODE_EXT as GLint) };
        assert_eq!(tex_get(), (GL_DECODE_EXT as GLint, GL_NO_ERROR));
    });
}

#[test]
fn sampler_set_get_roundtrip() {
    on_fresh_context(true, || {
        let s = new_sampler();
        // SAFETY: plain GL calls.
        unsafe { glSamplerParameteri(s, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint) };
        assert_eq!(smp_get(s), (GL_SKIP_DECODE_EXT as GLint, GL_NO_ERROR));
        unsafe { glSamplerParameteri(s, GL_TEXTURE_SRGB_DECODE_EXT, 0x1234) };
        assert_eq!(err(), GL_INVALID_ENUM);
        assert_eq!(smp_get(s), (GL_SKIP_DECODE_EXT as GLint, GL_NO_ERROR));
        unsafe { glSamplerParameteri(s, GL_TEXTURE_SRGB_DECODE_EXT, GL_DECODE_EXT as GLint) };
        assert_eq!(smp_get(s), (GL_DECODE_EXT as GLint, GL_NO_ERROR));
    });
}

#[test]
fn bound_sampler_overrides_the_texture() {
    on_fresh_context(true, || {
        new_texture();
        let s = new_sampler();
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint);
            glBindSampler(0, s);
        }
        assert_eq!(err(), GL_NO_ERROR);
        // the sampler holds the default DECODE and wins over the texture's SKIP
        assert_eq!(srgb_decode_bound_2d(0), GL_DECODE_EXT);
        // SAFETY: plain GL calls.
        unsafe { glSamplerParameteri(s, GL_TEXTURE_SRGB_DECODE_EXT, GL_SKIP_DECODE_EXT as GLint) };
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SRGB_DECODE_EXT, GL_DECODE_EXT as GLint) };
        assert_eq!(srgb_decode_bound_2d(0), GL_SKIP_DECODE_EXT);
        // unbound: the texture's own value again
        unsafe { glBindSampler(0, 0) };
        assert_eq!(srgb_decode_bound_2d(0), GL_DECODE_EXT);
        // the getters stay per object
        assert_eq!(tex_get().0, GL_DECODE_EXT as GLint);
        assert_eq!(smp_get(s).0, GL_SKIP_DECODE_EXT as GLint);
    });
}
