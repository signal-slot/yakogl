// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 attachment and renderbuffer queries of GL_RGB10_A2 on the V3D
//! host back end, through the GL entry points only (plan #370,
//! rgb10a2-query-api), as dEQP-GLES3 fbo.api and state_query read them.
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class
//! ([`backend::GLES_TEXFMT_RGB10A2`]) and its own render surface
//! ([`backend::GLES_SURF_RGB10_A2`], the 10/10/10/2 word; src/gl/framebuffer.rs
//! `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md). The queries then answer
//!
//! - `glGetRenderbufferParameteriv` (ES 3.0 6.1.14): RED/GREEN/BLUE/
//!   ALPHA_SIZE 10/10/10/2, INTERNAL_FORMAT the requested GL_RGB10_A2,
//! - `glGetFramebufferAttachmentParameteriv` (6.1.13), for a renderbuffer
//!   and for a texture attachment alike: RED/GREEN/BLUE/ALPHA_SIZE
//!   10/10/10/2, DEPTH_SIZE and STENCIL_SIZE 0, COMPONENT_TYPE
//!   GL_UNSIGNED_NORMALIZED, COLOR_ENCODING GL_LINEAR,
//! - `glCheckFramebufferStatus`: GL_FRAMEBUFFER_COMPLETE either way (ES 3.0
//!   table 3.13: GL_RGB10_A2 is colour-renderable).
//!
//! An RGBA8 attachment of the same kinds answers 8/8/8/8 for contrast.
//!
//! `glGetIntegerv(GL_RED_BITS ...)` with the RGB10_A2 framebuffer bound
//! should follow the bound draw framebuffer (6.1.13's sizes, 6.1 state
//! tables); today the answer is the constant 8 of the default framebuffer
//! whatever is bound (src/gl/state.rs, the GL_RED_BITS arm of
//! glGetIntegerv), so the spec row is `#[ignore]`d and a passing test pins
//! today's answer. The fallback builds are pinned by
//! tests/es3_rgb10a2_queries_fallback.rs.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_queries_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glFinish, glGetError, glGetIntegerv};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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
    name: c"es3-rgb10a2-queries".as_ptr() as *const c_char,
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
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glReadBuffer(GL_BACK);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const NULL: *const c_void = core::ptr::null();
const N: GLsizei = 4;

/// The attachment sizes of ES 3.0 6.1.13, the renderbuffer sizes of 6.1.14,
/// and the *_BITS state of the 6.1 state tables.
const ATT_SIZES: [GLenum; 4] = [
    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
];
const RB_SIZES: [GLenum; 4] =
    [GL_RENDERBUFFER_RED_SIZE, GL_RENDERBUFFER_GREEN_SIZE, GL_RENDERBUFFER_BLUE_SIZE, GL_RENDERBUFFER_ALPHA_SIZE];
const BITS: [GLenum; 4] = [GL_RED_BITS, GL_GREEN_BITS, GL_BLUE_BITS, GL_ALPHA_BITS];

/// Which kind of colour attachment a test makes.
#[derive(Clone, Copy)]
enum Kind {
    Texture,
    Renderbuffer,
}

const KINDS: [(Kind, &str); 2] = [(Kind::Texture, "texture"), (Kind::Renderbuffer, "renderbuffer")];

/// A target: an internal format and the channel sizes ES 3.0 gives its
/// storage here (10/10/10/2 for the class's GL_RGB10_A2, 8/8/8/8 for the
/// RGBA8 contrast).
struct Target {
    internalformat: GLenum,
    sizes: [GLint; 4],
    name: &'static str,
}

const TARGETS: [Target; 2] = [
    Target { internalformat: GL_RGB10_A2, sizes: [10, 10, 10, 2], name: "RGB10_A2" },
    Target { internalformat: GL_RGBA8, sizes: [8, 8, 8, 8], name: "RGBA8" },
];

/// The one pair ES 3.0 table 3.2 gives each internal format.
fn upload_pair(internalformat: GLenum) -> (GLenum, GLenum) {
    if internalformat == GL_RGB10_A2 {
        (GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV)
    } else {
        (GL_RGBA, GL_UNSIGNED_BYTE)
    }
}

/// Runs `f` with a new framebuffer bound to GL_FRAMEBUFFER whose colour
/// attachment 0 is a new N x N `t` texture (level 0) or renderbuffer --
/// complete, asserted -- then deletes them.
fn with_target<R>(t: &Target, kind: Kind, f: impl FnOnce() -> R) -> R {
    let (mut fb, mut obj) = (0, 0);
    let (format, type_) = upload_pair(t.internalformat);
    // SAFETY: room for one name each; no pixels; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            Kind::Texture => {
                glGenTextures(1, &mut obj);
                glBindTexture(GL_TEXTURE_2D, obj);
                glTexImage2D(GL_TEXTURE_2D, 0, t.internalformat as GLint, N, N, 0, format, type_, NULL);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, obj, 0);
            }
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut obj);
                glBindRenderbuffer(GL_RENDERBUFFER, obj);
                glRenderbufferStorage(GL_RENDERBUFFER, t.internalformat, N, N);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, obj);
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{} setup", t.name);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{} complete", t.name);
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
    assert_eq!(err(), GL_NO_ERROR, "attachment pname {pname:#x}");
    v
}

fn integer(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv {pname:#x}");
    v
}

/// The premise: this build has the class, so `rgb10a2_surface` is on and the
/// 10/10/10/2 answers below are this build's answers (docs/notes/v3d-rgb10a2.md).
#[test]
fn rgb10a2_build_has_the_class() {
    let _g = context();
    assert!(backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2), "the class's bit makes the surface drawable");
}

/// (a) 6.1.14: an RGB10_A2 renderbuffer's store is the 10/10/10/2 word, and
/// it keeps the internal format it was named by; RGBA8 says eight.
#[test]
fn renderbuffer_sizes_and_internal_format() {
    let _g = context();
    for t in &TARGETS {
        let (mut rb, mut sizes, mut format) = (0, [-1; 4], 0);
        // SAFETY: room for one name and one integer each; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, t.internalformat, N, N);
            assert_eq!(err(), GL_NO_ERROR, "{} storage", t.name);
            for (s, p) in sizes.iter_mut().zip(RB_SIZES) {
                glGetRenderbufferParameteriv(GL_RENDERBUFFER, p, s);
                assert_eq!(err(), GL_NO_ERROR, "{} pname {p:#x}", t.name);
            }
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_INTERNAL_FORMAT, &mut format);
            assert_eq!(err(), GL_NO_ERROR, "{} INTERNAL_FORMAT", t.name);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
        }
        assert_eq!(sizes, t.sizes, "{} RENDERBUFFER_RED/GREEN/BLUE/ALPHA_SIZE", t.name);
        assert_eq!(format as GLenum, t.internalformat, "{} RENDERBUFFER_INTERNAL_FORMAT", t.name);
    }
}

/// (b) 6.1.13: a colour attachment reports its channel sizes, no depth or
/// stencil, GL_UNSIGNED_NORMALIZED and GL_LINEAR -- for an RGB10_A2
/// renderbuffer and an RGB10_A2 texture attachment alike; RGBA8 for contrast.
#[test]
fn attachment_sizes_component_type_and_encoding() {
    let _g = context();
    for t in &TARGETS {
        for (kind, k) in KINDS {
            let got = with_target(t, kind, || {
                let sizes = ATT_SIZES.map(attachment);
                let (depth, stencil) = (
                    attachment(GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE),
                    attachment(GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE),
                );
                let ct = attachment(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE);
                let enc = attachment(GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING);
                (sizes, depth, stencil, ct, enc)
            });
            assert_eq!(got.0, t.sizes, "{} {k} attachment RED/GREEN/BLUE/ALPHA_SIZE", t.name);
            assert_eq!((got.1, got.2), (0, 0), "{} {k} attachment DEPTH/STENCIL_SIZE", t.name);
            assert_eq!(got.3, GL_UNSIGNED_NORMALIZED as GLint, "{} {k} attachment COMPONENT_TYPE", t.name);
            assert_eq!(got.4, GL_LINEAR as GLint, "{} {k} attachment COLOR_ENCODING", t.name);
        }
    }
}

/// (d) 4.4.4: GL_RGB10_A2 is colour-renderable (ES 3.0 table 3.13), so both
/// attachment kinds complete. (`with_target` asserts it on the way in; this
/// names it as the item's own row.)
#[test]
fn check_framebuffer_status_is_complete() {
    let _g = context();
    for t in &TARGETS {
        for (kind, k) in KINDS {
            with_target(t, kind, || {
                // SAFETY: plain GL call on the framebuffer just bound.
                unsafe {
                    assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{} {k}", t.name)
                };
            });
        }
    }
}

/// (c) Today: the *_BITS state with an RGB10_A2 or RGBA8 framebuffer bound
/// is the constant 8 of the default framebuffer (src/gl/state.rs, the
/// GL_RED_BITS arm of glGetIntegerv), whatever READ_BUFFER selects.
#[test]
fn color_bits_with_target_bound_today() {
    let _g = context();
    for t in &TARGETS {
        for (kind, k) in KINDS {
            let got = with_target(t, kind, || {
                // SAFETY: plain GL call on the framebuffer just bound.
                unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0) };
                BITS.map(integer)
            });
            assert_eq!(got, [8; 4], "{} {k} RED/GREEN/BLUE/ALPHA_BITS today", t.name);
        }
    }
}

/// (c) ES 3.0 6.1.13: with a framebuffer object bound, *_BITS take the size
/// of the component of the image attached at READ_BUFFER -- 10/10/10/2 for
/// the class's RGB10_A2, 8/8/8/8 for RGBA8.
#[test]
#[ignore = "6.1.13 (*_BITS of the draw framebuffer): constant 8 today, src/gl/state.rs (glGetIntegerv GL_RED_BITS arm)"]
fn color_bits_follow_the_bound_target() {
    let _g = context();
    for t in &TARGETS {
        for (kind, k) in KINDS {
            let got = with_target(t, kind, || {
                // SAFETY: plain GL call on the framebuffer just bound.
                unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0) };
                BITS.map(integer)
            });
            assert_eq!(got, t.sizes, "{} {k} RED/GREEN/BLUE/ALPHA_BITS", t.name);
        }
    }
}
