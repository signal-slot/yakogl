// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 getters of the half-float colour targets that
//! `GL_EXT_color_buffer_half_float` makes complete (src/gl/state.rs): RGBA16F,
//! RG16F and R16F as a texture and as a renderbuffer attachment.
//!
//! - `glGetFramebufferAttachmentParameteriv`: COMPONENT_TYPE is `GL_FLOAT`,
//!   the RED/GREEN/BLUE/ALPHA sizes are 16 for a channel the format has and
//!   0 for one it has not, COLOR_ENCODING is `GL_LINEAR` (ES 3.0.6 6.1.13).
//! - `glGetRenderbufferParameteriv`: the same sizes (6.1.14).
//! - `glGetInternalformativ`: GL_NUM_SAMPLE_COUNTS and GL_SAMPLES (6.1.15).
//!   GL_MAX_SAMPLES is 1 without `msaa-fbo`, so the default build is tested:
//!   one count, 1.
//! - `glGetIntegerv(GL_RED_BITS ...)` with the target bound: the bits of the
//!   bound draw framebuffer's colour buffer (6.1.13's sizes, 6.1 state
//!   tables). Today the answer is a constant 8 (src/gl/state.rs, the
//!   GL_RED_BITS arm), so that test is `#[ignore]`d; a passing test pins
//!   today's answer.
//!
//! The rule is GL core and the same on every GPU:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_half_float_getters
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_getters
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_half_float_getters
//! ```
//!
//! No GPU runs: the stand-in back end draws nothing. Not in a `report-es2`
//! build: ES 2.0 has none of the sized float names.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
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
    name: c"es3-half-float-getters".as_ptr() as *const c_char,
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

/// The GPU this build is for, as the messages name it.
const GPU: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "vivante"
} else if cfg!(feature = "gpu-v3d") {
    "v3d"
} else {
    "other"
};

/// A half-float colour format: internalformat, texture (format, type), the
/// channel sizes ES 3.0 6.1.13 gives it, and its name.
struct Half {
    internalformat: GLenum,
    format: GLenum,
    type_: GLenum,
    sizes: [GLint; 4],
    name: &'static str,
}

const HALF: [Half; 3] = [
    Half {
        internalformat: GL_RGBA16F,
        format: GL_RGBA,
        type_: GL_HALF_FLOAT,
        sizes: [16, 16, 16, 16],
        name: "RGBA16F",
    },
    Half { internalformat: GL_RG16F, format: GL_RG, type_: GL_HALF_FLOAT, sizes: [16, 16, 0, 0], name: "RG16F" },
    Half { internalformat: GL_R16F, format: GL_RED, type_: GL_HALF_FLOAT, sizes: [16, 0, 0, 0], name: "R16F" },
];

const ATT_SIZES: [GLenum; 4] = [
    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
];
const RB_SIZES: [GLenum; 4] =
    [GL_RENDERBUFFER_RED_SIZE, GL_RENDERBUFFER_GREEN_SIZE, GL_RENDERBUFFER_BLUE_SIZE, GL_RENDERBUFFER_ALPHA_SIZE];
const BITS: [GLenum; 4] = [GL_RED_BITS, GL_GREEN_BITS, GL_BLUE_BITS, GL_ALPHA_BITS];

/// Which kind of attachment a test makes.
#[derive(Clone, Copy)]
enum Kind {
    Texture,
    Renderbuffer,
}

const KINDS: [(Kind, &str); 2] = [(Kind::Texture, "texture"), (Kind::Renderbuffer, "renderbuffer")];

/// Runs `f` with a new framebuffer bound whose colour attachment 0 is a new
/// 16 x 16 `h` texture or renderbuffer (complete: asserted), then cleans up.
fn with_target<R>(h: &Half, kind: Kind, f: impl FnOnce() -> R) -> R {
    let (mut fb, mut obj) = (0, 0);
    // SAFETY: room for one name each; no pixels; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            Kind::Texture => {
                glGenTextures(1, &mut obj);
                glBindTexture(GL_TEXTURE_2D, obj);
                glTexImage2D(GL_TEXTURE_2D, 0, h.internalformat as GLint, 16, 16, 0, h.format, h.type_, NULL);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, obj, 0);
            }
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut obj);
                glBindRenderbuffer(GL_RENDERBUFFER, obj);
                glRenderbufferStorage(GL_RENDERBUFFER, h.internalformat, 16, 16);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, obj);
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} setup", h.name);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} complete", h.name);
    }
    let r = f();
    // SAFETY: the names are the ones made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        match kind {
            Kind::Texture => glDeleteTextures(1, &obj),
            Kind::Renderbuffer => glDeleteRenderbuffers(1, &obj),
        }
    }
    r
}

fn attachment(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: attachment pname {pname:#x}");
    v
}

fn integer(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: glGetIntegerv {pname:#x}");
    v
}

#[test]
fn attachment_component_type_is_float() {
    let _g = context();
    for h in &HALF {
        for (kind, k) in KINDS {
            let v = with_target(h, kind, || attachment(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE));
            assert_eq!(v, GL_FLOAT as GLint, "{GPU}: {} {k} COMPONENT_TYPE", h.name);
        }
    }
}

#[test]
fn attachment_channel_sizes() {
    let _g = context();
    for h in &HALF {
        for (kind, k) in KINDS {
            let got = with_target(h, kind, || ATT_SIZES.map(attachment));
            assert_eq!(got, h.sizes, "{GPU}: {} {k} RED/GREEN/BLUE/ALPHA_SIZE", h.name);
        }
    }
}

#[test]
fn attachment_color_encoding_is_linear() {
    let _g = context();
    for h in &HALF {
        for (kind, k) in KINDS {
            let v = with_target(h, kind, || attachment(GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING));
            assert_eq!(v, GL_LINEAR as GLint, "{GPU}: {} {k} COLOR_ENCODING", h.name);
        }
    }
}

#[test]
fn renderbuffer_channel_sizes() {
    let _g = context();
    for h in &HALF {
        let (mut rb, mut got, mut format) = (0, [0; 4], 0);
        // SAFETY: room for one name and one integer each; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, h.internalformat, 16, 16);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} storage", h.name);
            for (g, p) in got.iter_mut().zip(RB_SIZES) {
                glGetRenderbufferParameteriv(GL_RENDERBUFFER, p, g);
                assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} pname {p:#x}", h.name);
            }
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_INTERNAL_FORMAT, &mut format);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
        }
        assert_eq!(got, h.sizes, "{GPU}: {} RENDERBUFFER_RED/GREEN/BLUE/ALPHA_SIZE", h.name);
        assert_eq!(format as GLenum, h.internalformat, "{GPU}: {} RENDERBUFFER_INTERNAL_FORMAT", h.name);
    }
}

/// 6.1.15: one sample count, 1 (GL_MAX_SAMPLES is 1 without `msaa-fbo`).
#[test]
fn internalformat_sample_counts_default_build() {
    let _g = context();
    assert_eq!(integer(GL_MAX_SAMPLES), 1, "{GPU}: GL_MAX_SAMPLES");
    for h in &HALF {
        let (mut n, mut s) = (-1, -1);
        // SAFETY: room for one integer each.
        unsafe {
            glGetInternalformativ(GL_RENDERBUFFER, h.internalformat, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} NUM_SAMPLE_COUNTS", h.name);
            glGetInternalformativ(GL_RENDERBUFFER, h.internalformat, GL_SAMPLES, 1, &mut s);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} SAMPLES", h.name);
        }
        assert_eq!((n, s), (1, 1), "{GPU}: {} (NUM_SAMPLE_COUNTS, SAMPLES)", h.name);
    }
}

/// Today's answer of the *_BITS queries with a 16F target bound: the
/// constant 8 of the default framebuffer, whatever is bound.
#[test]
fn color_bits_with_target_bound_today() {
    let _g = context();
    for h in &HALF {
        for (kind, k) in KINDS {
            let got = with_target(h, kind, || BITS.map(integer));
            assert_eq!(got, [8; 4], "{GPU}: {} {k} RED/GREEN/BLUE/ALPHA_BITS today", h.name);
        }
    }
}

/// The *_BITS state follows the bound draw framebuffer's colour attachment:
/// 16 for a channel of a 16F target and 0 for one it has not.
#[test]
#[ignore = "6.1.13 (*_BITS of the draw framebuffer): constant 8 today, src/gl/state.rs glGetIntegerv GL_RED_BITS arm"]
fn color_bits_follow_the_bound_target() {
    let _g = context();
    for h in &HALF {
        for (kind, k) in KINDS {
            let got = with_target(h, kind, || BITS.map(integer));
            assert_eq!(got, h.sizes, "{GPU}: {} {k} RED/GREEN/BLUE/ALPHA_BITS", h.name);
        }
    }
}
