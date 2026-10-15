// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's sRGB colour buffers and textures through the GL API on the host
//! (docs/es3-srgb.md; the AM62P dEQP-GLES3 failures it answers are listed
//! there and in docs/HANDOFF.md, srgb).
//!
//! What is held here is the GL layer's contract with a back end: an
//! `SRGB8_ALPHA8` renderbuffer or texture level is a complete colour buffer
//! whose encoding is GL_SRGB and whose draws and clears reach the back end
//! as `GLES_SURF_SRGB8_A8` with the colour linear; glReadPixels returns the
//! bytes the back end stored, encoded; `SRGB8` is texture-only; a back end
//! that cannot encode (`backend::SRGB_RT`, the GC7000 and the V3D today)
//! refuses the draw by name. The stand-in back end's `draw` stores a clear
//! the way a back end that renders the class must -- red, green and blue
//! clamped and encoded (`gl::srgb::encode`), alpha clamped and linear -- and
//! claims the class through `backend::HOST_SRGB_RT`; no GPU runs here. The
//! PowerVR's own half (the PBE's gamma, the TPU's) is pinned in
//! src/pvr/backend.rs and src/pvr/backend_tests.rs.
//!
//! An ES 2.0-reporting build (`report-es2`) must still refuse the sRGB
//! names: `es2_build_has_no_srgb_format` is the only test it runs.
//! tools/check.sh runs this file in the gpu-powervr, gpu-vivante and V3D
//! rows, with and without report-es2.

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
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

/// Draws the stand-in back end was handed: (target format, stored a clear).
static DRAWS: Mutex<Vec<(u8, bool)>> = Mutex::new(Vec::new());

/// A clear as a back end that renders the sRGB class stores it: the linear
/// colour clamped to [0, 1], red, green and blue sRGB-encoded, alpha a
/// linear byte, every pixel, rows at the pair-padded pitch. Every other
/// target is left alone.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for i in 0..d.num_targets as usize {
        let t = &d.targets[i];
        let hit = d.do_clear && (d.clear_target_mask == 0 || d.clear_target_mask & (1 << i) != 0) && t.gpu != 0;
        let srgb = backend::gles_surf_is_srgb(t.format);
        DRAWS.lock().unwrap().push((t.format, hit && srgb));
        if !hit || !srgb {
            continue;
        }
        let c = d.clear_rgba;
        let unorm = |v: f32| {
            if v >= 1.0 {
                255
            } else if v > 0.0 {
                (v * 255.0 + 0.5) as u8
            } else {
                0
            }
        };
        let px = [
            yakogl::gl::srgb::encode(c[0]),
            yakogl::gl::srgb::encode(c[1]),
            yakogl::gl::srgb::encode(c[2]),
            unorm(c[3]),
        ];
        let pitch = ((t.w + 1) & !1) as usize;
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * 4), 4);
            }
        }
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-srgb".as_ptr() as *const c_char,
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
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    DRAWS.lock().unwrap().clear();
    backend::HOST_SRGB_RT.store(false, Ordering::Relaxed);
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
    }
    rb
}

fn texture(internalformat: GLenum, format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, w, h, 0, format, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    t
}

/// A framebuffer, bound, with renderbuffer `rb` on colour attachment 0.
fn framebuffer(rb: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    fb
}

fn unbind(fb: GLuint) {
    // SAFETY: a name this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn encoding(attachment: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe {
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            attachment,
            GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING,
            &mut v,
        )
    };
    v
}

fn read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h * 4) as usize];
    // SAFETY: room for w x h RGBA pixels.
    unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    px
}

/* ---- the ES 2.0 report has none ------------------------------------------ */

#[test]
#[cfg(feature = "report-es2")]
fn es2_build_has_no_srgb_format() {
    let _g = context();
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_SRGB8_ALPHA8, 4, 4);
    }
    assert_eq!(err(), GL_INVALID_ENUM, "GL_EXT_sRGB is not claimed: no sRGB renderbuffer");
    let _t = texture(GL_SRGB8_ALPHA8, GL_RGBA, 4, 4);
    assert_eq!(err(), GL_INVALID_VALUE, "nor an sRGB texture");
    let _t = texture(GL_SRGB8, GL_RGB, 4, 4);
    assert_eq!(err(), GL_INVALID_VALUE);
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

/* ---- ES 3.0 --------------------------------------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    #[test]
    fn srgb8_alpha8_is_a_complete_srgb_colour_buffer_and_srgb8_is_not() {
        let _g = context();
        assert_eq!(encoding(GL_BACK), GL_LINEAR as GLint, "the window is linear");
        let rb = renderbuffer(GL_SRGB8_ALPHA8, 4, 4);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(encoding(GL_COLOR_ATTACHMENT0), GL_SRGB as GLint);
        let tex = texture(GL_SRGB8_ALPHA8, GL_RGBA, 4, 4);
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(encoding(GL_COLOR_ATTACHMENT0), GL_SRGB as GLint);
        /* dEQP fbo.completeness.renderable.texture.color0.srgb8 */
        let rgb = texture(GL_SRGB8, GL_RGB, 4, 4);
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rgb, 0) };
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "SRGB8 is texture-only (ES 3.0 table 3.13)");
        /* nor renderbuffer storage */
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_SRGB8, 4, 4) };
        assert_eq!(err(), GL_INVALID_ENUM);
        unbind(fb);
    }

    /// The clear reaches a back end that renders the class as the sRGB
    /// surface with the linear colour, and what it stored reads back
    /// encoded: 0.5 is byte 188, 1.5 clamps to 255, alpha 0.5 is 128.
    #[test]
    fn a_clear_reads_back_encoded() {
        let _g = context();
        backend::HOST_SRGB_RT.store(true, Ordering::Relaxed);
        let rb = renderbuffer(GL_SRGB8_ALPHA8, 3, 2);
        let fb = framebuffer(rb);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.5, 1.5, 0.2, 0.5);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(err(), GL_NO_ERROR);
        let px = read(3, 2);
        assert_eq!(err(), GL_NO_ERROR);
        assert!(DRAWS.lock().unwrap().iter().any(|&(f, stored)| f == backend::GLES_SURF_SRGB8_A8 && stored));
        for p in px.as_chunks::<4>().0 {
            assert_eq!(*p, [188, 255, 124, 128]);
        }
        unbind(fb);
    }

    /// A back end that cannot encode refuses the clear by name instead of
    /// storing the linear colour as if it were encoded; the PowerVR renders
    /// the class (`SRGB_RT`), the GC7000 and the V3D rows of this file see
    /// the refusal.
    #[test]
    fn a_back_end_that_cannot_encode_refuses_the_draw() {
        let _g = context();
        let rb = renderbuffer(GL_SRGB8_ALPHA8, 4, 4);
        let fb = framebuffer(rb);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(glGetError(), GL_NO_ERROR, "the clear is recorded");
            glFinish();
        }
        if backend::SRGB_RT {
            assert_eq!(err(), GL_NO_ERROR);
            assert!(DRAWS.lock().unwrap().iter().any(|d| d.1), "the clear reached the back end");
        } else {
            assert_eq!(err(), GL_INVALID_OPERATION, "refused where it would reach the back end");
            assert!(DRAWS.lock().unwrap().iter().all(|d| !d.1), "nothing was stored");
        }
        assert_eq!(backend::SRGB_RT, cfg!(feature = "gpu-powervr"));
        unbind(fb);
    }

    /// Every back end samples the sRGB class: an SRGB8_ALPHA8 texture is
    /// stored in it, not refused.
    #[test]
    fn every_back_end_samples_the_srgb_class() {
        let _g = context();
        assert_ne!(backend::sampled_texfmts() & backend::gles_texfmt_bit(backend::GLES_TEXFMT_SRGB8_A8), 0);
        let _t = texture(GL_SRGB8_ALPHA8, GL_RGBA, 4, 4);
        assert_eq!(err(), GL_NO_ERROR);
        let mut t = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexStorage2D(GL_TEXTURE_2D, 3, GL_SRGB8, 4, 4);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }
}
