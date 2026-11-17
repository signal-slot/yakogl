// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 attachment and renderbuffer queries of GL_RGB10_A2 where the
//! build has no class for it, through the GL entry points only (plan #370
//! item rgb10a2-query-api, the fallback half of tests/es3_rgb10a2_queries_v3d.rs).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is the 10/10/10/2 surface and answers
//! 10/10/10/2 there (src/gl/framebuffer.rs `rgb10a2_surface`,
//! docs/notes/v3d-rgb10a2.md). Everywhere else it must stay away:
//!
//! - the PowerVR and the GC7000 builds: the class is not in their sampled
//!   set (`backend::sampled_texfmts`), so a GL_RGB10_A2 renderbuffer is
//!   RGBA8888 and a texture of the name is in the RGBA8 class. Their queries
//!   answer eight bits a channel and GL_UNSIGNED_NORMALIZED, and
//!   GL_RENDERBUFFER_INTERNAL_FORMAT keeps the requested GL_RGB10_A2 name;
//!   the attachment completes. That is what the 6.1.13/6.1.14 rows of
//!   `renderbuffer_bits` and `es3_texture_bits` answer, and this file pins
//!   it, so a later change cannot move the 10/10/10/2 answers of the class
//!   build onto these;
//! - an ES 2.0-reporting build names no GL_RGB10_A2 at all: neither the
//!   renderbuffer nor the texture is allocated (as tests/es3_rgb10a2_fallback.rs
//!   holds), the queries see a renderbuffer with no data store, and the ES
//!   3.0 attachment pnames are unknown here (GL_INVALID_ENUM).
//!
//! The rows say what these builds answer today, matching or not; what the
//! spec asks is in the messages.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_queries_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_queries_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_queries_fallback
//! ```
//!
//! Compiled only where the class falls back; the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its
//! answers are pinned by tests/es3_rgb10a2_queries_v3d.rs). No GPU runs: the
//! stand-in back end draws nothing.

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

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
    name: c"es3-rgb10a2-queries-fallback".as_ptr() as *const c_char,
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
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

const ATT_SIZES: [GLenum; 4] = [
    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
];
const RB_SIZES: [GLenum; 4] =
    [GL_RENDERBUFFER_RED_SIZE, GL_RENDERBUFFER_GREEN_SIZE, GL_RENDERBUFFER_BLUE_SIZE, GL_RENDERBUFFER_ALPHA_SIZE];
const BITS: [GLenum; 4] = [GL_RED_BITS, GL_GREEN_BITS, GL_BLUE_BITS, GL_ALPHA_BITS];

#[derive(Clone, Copy)]
enum Kind {
    Texture,
    Renderbuffer,
}

const KINDS: [(Kind, &str); 2] = [(Kind::Texture, "texture"), (Kind::Renderbuffer, "renderbuffer")];

/// A colour attachment name: its internal format, what the fallback build
/// answers for its channel sizes, and its name.
struct Target {
    internalformat: GLenum,
    sizes: [GLint; 4],
    name: &'static str,
}

/// The one pair ES 3.0 table 3.2 gives each internal format.
fn upload_pair(internalformat: GLenum) -> (GLenum, GLenum) {
    if internalformat == GL_RGB10_A2 {
        (GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV)
    } else {
        (GL_RGBA, GL_UNSIGNED_BYTE)
    }
}

fn attachment(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "attachment pname {pname:#x}");
    v
}

fn rb_param(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "renderbuffer pname {pname:#x}");
    v
}

fn integer(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one integer.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv {pname:#x}");
    v
}

/* ---- ES 3.0 on a build without the class -------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    const TARGETS: [Target; 2] = [
        Target { internalformat: GL_RGB10_A2, sizes: [8, 8, 8, 8], name: "RGB10_A2" },
        Target { internalformat: GL_RGBA8, sizes: [8, 8, 8, 8], name: "RGBA8" },
    ];

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

    /// The premise: this build's back end does not sample the class, so the
    /// 10/10/10/2 surface is not drawable and GL_RGB10_A2 falls back to
    /// RGBA8 (framebuffer.rs `rgb10a2_surface`).
    #[test]
    fn rgb10a2_fallback_build_has_no_class() {
        let _g = context();
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2), "no bit, no surface");
    }

    /// 6.1.14: an RGB10_A2 renderbuffer is stored as RGBA8, so the sizes
    /// answer eight -- while GL_RENDERBUFFER_INTERNAL_FORMAT keeps the
    /// requested GL_RGB10_A2 (`glGetRenderbufferParameteriv`).
    #[test]
    fn renderbuffer_sizes_are_eight() {
        let _g = context();
        for t in &TARGETS {
            let (mut rb, mut sizes) = (0, [-1; 4]);
            // SAFETY: room for one name; plain GL calls.
            unsafe {
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, t.internalformat, N, N);
                assert_eq!(err(), GL_NO_ERROR, "{} storage", t.name);
            }
            for (s, p) in sizes.iter_mut().zip(RB_SIZES) {
                *s = rb_param(p);
            }
            let format = rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT);
            // SAFETY: plain GL calls.
            unsafe {
                glBindRenderbuffer(GL_RENDERBUFFER, 0);
                glDeleteRenderbuffers(1, &rb);
            }
            assert_eq!(sizes, t.sizes, "{} RENDERBUFFER_RED/GREEN/BLUE/ALPHA_SIZE", t.name);
            assert_eq!(format as GLenum, t.internalformat, "{} RENDERBUFFER_INTERNAL_FORMAT", t.name);
        }
    }

    /// 6.1.13: an RGB10_A2 renderbuffer or texture attachment reports the
    /// eight bits its RGBA8 store holds, no depth or stencil,
    /// GL_UNSIGNED_NORMALIZED, GL_LINEAR, and completes.
    #[test]
    fn attachment_sizes_are_eight() {
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

    /// The *_BITS state with an RGB10_A2 framebuffer bound: today the
    /// constant 8 whatever is bound (src/gl/state.rs, the GL_RED_BITS arm)
    /// -- and on these builds 8 is what an RGB10_A2 attachment really holds,
    /// so no row can tell the fallback from the class rule.
    #[test]
    fn color_bits_with_target_bound() {
        let _g = context();
        for t in &TARGETS {
            for (kind, k) in KINDS {
                let got = with_target(t, kind, || {
                    // SAFETY: plain GL call on the framebuffer just bound.
                    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0) };
                    BITS.map(integer)
                });
                assert_eq!(got, [8; 4], "{} {k} RED/GREEN/BLUE/ALPHA_BITS", t.name);
            }
        }
    }
}

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ------------------------ */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 names no GL_RGB10_A2 (as tests/es3_rgb10a2_fallback.rs holds):
    /// neither object is allocated, and with them attached the framebuffer
    /// is incomplete, so the queries below run on objects with no data
    /// store.
    #[test]
    fn rgb10a2_is_not_allocated_and_nothing_completes() {
        let _g = context();
        let (mut rb, mut tex, mut fb) = (0, 0, 0);
        // SAFETY: plain GL calls; room for one name each.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, N, N);
            assert_eq!(err(), GL_INVALID_ENUM, "glRenderbufferStorage(GL_RGB10_A2)");
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB10_A2 as GLint, N, N, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
            assert_eq!(err(), GL_INVALID_VALUE, "glTexImage2D(GL_RGB10_A2)");
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(
                glCheckFramebufferStatus(GL_FRAMEBUFFER),
                GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
                "renderbuffer with no storage"
            );
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(
                glCheckFramebufferStatus(GL_FRAMEBUFFER),
                GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
                "texture with no image"
            );
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &tex);
            glDeleteRenderbuffers(1, &rb);
        }
    }

    /// What a renderbuffer with no data store answers today: no format of
    /// its own but GL_RGBA4, every size 0. (ES 2.0 6.1.13 would ask GL_NONE
    /// for the format; the answer is recorded as it is, not as it should
    /// be.)
    #[test]
    fn unallocated_renderbuffer_answers_today() {
        let _g = context();
        let mut rb = 0;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, N, N);
            assert_eq!(err(), GL_INVALID_ENUM);
        }
        let mut sizes = [-1; 4];
        for (s, p) in sizes.iter_mut().zip(RB_SIZES) {
            *s = rb_param(p);
        }
        let (depth, stencil) = (rb_param(GL_RENDERBUFFER_DEPTH_SIZE), rb_param(GL_RENDERBUFFER_STENCIL_SIZE));
        let format = rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT);
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
        }
        assert_eq!(sizes, [0; 4], "no storage: no bits");
        assert_eq!((depth, stencil), (0, 0), "no storage: no depth or stencil");
        assert_eq!(format as GLenum, GL_RGBA4, "no storage answers GL_RGBA4 today");
    }

    /// The ES 3.0 attachment pnames are not on the ES 2.0 surface: with a
    /// no-image texture attached they answer GL_INVALID_ENUM, while the one
    /// ES 2.0 pname still names the attachment.
    #[test]
    fn attachment_bit_pnames_are_unknown_here() {
        let _g = context();
        let (mut tex, mut fb) = (0, 0);
        // SAFETY: plain GL calls; room for one name each.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB10_A2 as GLint, N, N, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
            assert_eq!(err(), GL_INVALID_VALUE);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(err(), GL_NO_ERROR);
        }
        let mut v = 0;
        for p in ATT_SIZES
            .into_iter()
            .chain([GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE, GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE])
            .chain([GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE, GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING])
        {
            // SAFETY: room for one integer.
            unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, p, &mut v) };
            assert_eq!(err(), GL_INVALID_ENUM, "ES 2.0 surface does not know attachment pname {p:#x}");
        }
        // SAFETY: room for one integer.
        unsafe {
            glGetFramebufferAttachmentParameteriv(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
                &mut v,
            )
        };
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(v as GLenum, GL_TEXTURE, "the ES 2.0 pname still answers");
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &tex);
        }
    }

    /// The RGBA8 contrast on this surface: an ES 2.0 renderbuffer of
    /// GL_RGBA8 (GL_OES_rgb8_rgba8) stores and answers eight, completes,
    /// and keeps its name.
    #[test]
    fn rgba8_renderbuffer_and_attachment() {
        let _g = context();
        let (mut rb, mut fb) = (0, 0);
        // SAFETY: plain GL calls; room for one name each.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, N, N);
            assert_eq!(err(), GL_NO_ERROR, "GL_OES_rgb8_rgba8 is listed here");
        }
        let mut sizes = [-1; 4];
        for (s, p) in sizes.iter_mut().zip(RB_SIZES) {
            *s = rb_param(p);
        }
        assert_eq!(sizes, [8; 4], "RENDERBUFFER_RED/GREEN/BLUE/ALPHA_SIZE");
        assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT) as GLenum, GL_RGBA8);
        // SAFETY: plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
        }
    }
}
