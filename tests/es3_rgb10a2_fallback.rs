// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Where GL_RGB10_A2 is not its own render surface (`rgb10a2_surface()` in
//! gl/framebuffer.rs is false: the PowerVR, the GC7000UL, and the V3D with
//! `report-es2`), the format is held as RGBA8 and nothing about it changes
//! when the V3D class build learns to decode the 10/10/10/2 word
//! (docs/notes/v3d-rgb10a2.md). This file pins that:
//!
//!   - a GL_RGB10_A2 renderbuffer and texture attachment read back with
//!     glReadPixels GL_RGBA/GL_UNSIGNED_BYTE as the RGBA8 bytes they hold;
//!   - GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE answer GL_RGBA/GL_UNSIGNED_BYTE;
//!   - glReadPixels with GL_UNSIGNED_INT_2_10_10_10_REV is GL_INVALID_OPERATION;
//!   - glBlitFramebuffer NEAREST from such a texture into an RGBA8 texture
//!     copies the bytes unchanged.
//!
//! No GPU runs, so the pixels are the ones the CPU wrote: the texture is
//! specified from a packed word (the core converts it to RGBA8 bytes), the
//! renderbuffer is not drawn and only its read-back answers are pinned. A
//! `report-es2` build has no GL_RGB10_A2 renderbuffer and no glBlitFramebuffer:
//! it pins those errors (GL_INVALID_ENUM, GL_INVALID_OPERATION) and the
//! RGBA8 read pair.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_fallback
//! ```
//!
//! Not run (gated out) in the V3D build without `report-es2`: there the class
//! is its own surface.

#![cfg(all(
    feature = "host-test",
    any(feature = "report-es2", feature = "gpu-powervr", feature = "gpu-vivante"),
    not(all(feature = "gpu-v3d", not(feature = "report-es2")))
))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
#[cfg(not(feature = "report-es2"))]
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

/// Every range the GL wrote back from the CPU cache (address, bytes).
static FLUSHES: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_flush(cpu: *mut c_void, n: usize) {
    FLUSHES.lock().unwrap().push((cpu as usize, n));
}
unsafe extern "C" fn m_invd(_cpu: *mut c_void, _n: usize) {}
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
    cache_flush: Some(m_flush),
    cache_invd: Some(m_invd),
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
    name: c"es3-rgb10a2-fallback".as_ptr() as *const c_char,
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
        while glGetError() != GL_NO_ERROR {}
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    FLUSHES.lock().unwrap().clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = [i32::MIN; 4];
    // SAFETY: room for the widest answer.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    v[0]
}

/// The word of an RGB10_A2 texel and the RGBA8 bytes the fallback holds for it.
#[cfg(not(feature = "report-es2"))]
const WORD: u32 = 1023 | (512 << 10) | (3 << 30);
#[cfg(not(feature = "report-es2"))]
const BYTES: [u8; 4] = [255, 128, 0, 255];

/// An RGB10_A2 renderbuffer cleared to (1, 0.5, 0, 1) on a bound framebuffer:
/// (framebuffer, renderbuffer).
#[cfg(not(feature = "report-es2"))]
fn rb_target() -> (GLuint, GLuint) {
    let (mut rb, mut fb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, 2, 2);
        assert_eq!(err(), GL_NO_ERROR);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearColor(1.0, 0.5, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    (fb, rb)
}

/// A 2 x 2 RGB10_A2 texture of [`WORD`] on a bound framebuffer.
#[cfg(not(feature = "report-es2"))]
fn tex_target() -> (GLuint, GLuint) {
    let (mut tex, mut fb) = (0, 0);
    let words = [WORD; 4];
    // SAFETY: room for one name each; `words` holds 2 x 2 texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB10_A2 as GLint,
            2,
            2,
            0,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        );
        assert_eq!(err(), GL_NO_ERROR);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (tex, fb)
}

#[cfg(not(feature = "report-es2"))]
fn read_ubyte() -> [u8; 16] {
    let mut px = [0u8; 16];
    // SAFETY: room for 2 x 2 RGBA8 pixels.
    unsafe { glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    px
}

fn read_pair_is_rgba8() {
    assert_eq!(geti(GL_IMPLEMENTATION_COLOR_READ_FORMAT) as GLenum, GL_RGBA);
    assert_eq!(geti(GL_IMPLEMENTATION_COLOR_READ_TYPE) as GLenum, GL_UNSIGNED_BYTE);
    assert_eq!(err(), GL_NO_ERROR);
}

#[cfg(not(feature = "report-es2"))]
fn packed_read_is_refused() {
    let mut px = [0u8; 16];
    // SAFETY: room for 2 x 2 pixels of 4 bytes.
    unsafe { glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_INVALID_OPERATION, "no packed read-back where the class is RGBA8");
    assert_eq!(px, [0u8; 16], "nothing written");
}

/// Without a GL_RGB10_A2 attachment the read pair is the RGBA8 one too.
#[test]
fn rgb10a2_fallback_read_pair_without_attachment() {
    let _g = context();
    read_pair_is_rgba8();
}

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The renderbuffer: complete, an RGBA8 read pair, the packed read
    /// refused. The stand-in back end draws nothing, so the cleared bytes
    /// are the GPU's to write; the bytes are pinned on the texture below.
    #[test]
    fn rgb10a2_fallback_renderbuffer_read_pair_and_packed_read() {
        let _g = context();
        let (fb, rb) = rb_target();
        read_pair_is_rgba8();
        let _ = read_ubyte();
        assert_eq!(err(), GL_NO_ERROR, "GL_RGBA/GL_UNSIGNED_BYTE is the read it takes");
        packed_read_is_refused();
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
    }

    #[test]
    fn rgb10a2_fallback_texture_reads_back_rgba8_bytes() {
        let _g = context();
        let (tex, fb) = tex_target();
        let px = read_ubyte();
        assert_eq!(err(), GL_NO_ERROR);
        for p in px.chunks(4) {
            assert_eq!(p, BYTES, "the RGBA8 bytes of the word, not the word");
        }
        read_pair_is_rgba8();
        packed_read_is_refused();
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &tex);
        }
    }

    /// NEAREST blit from the RGB10_A2 texture into an RGBA8 texture copies
    /// the bytes.
    #[test]
    fn rgb10a2_fallback_blit_nearest_copies_bytes() {
        let _g = context();
        let (src, rfb) = tex_target();
        let (mut dst, mut dfb) = (0, 0);
        let zero = [0u8; 16];
        // SAFETY: room for one name each; plain GL calls.
        unsafe {
            glGenTextures(1, &mut dst);
            glBindTexture(GL_TEXTURE_2D, dst);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, zero.as_ptr() as _);
            glGenFramebuffers(1, &mut dfb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dfb);
            glFramebufferTexture2D(GL_DRAW_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, dst, 0);
            glBindFramebuffer(GL_READ_FRAMEBUFFER, rfb);
            assert_eq!(glCheckFramebufferStatus(GL_DRAW_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            assert_eq!(err(), GL_NO_ERROR);
            glBlitFramebuffer(0, 0, 2, 2, 0, 0, 2, 2, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            assert_eq!(err(), GL_NO_ERROR);
            glBindFramebuffer(GL_READ_FRAMEBUFFER, dfb);
        }
        let px = read_ubyte();
        for p in px.chunks(4) {
            assert_eq!(p, BYTES, "the bytes of the source, unchanged");
        }
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &rfb);
            glDeleteFramebuffers(1, &dfb);
            glDeleteTextures(1, &src);
            glDeleteTextures(1, &dst);
        }
    }
}

/// ES 2.0 reporting: GL_RGB10_A2 is no renderbuffer storage, and there is no
/// glBlitFramebuffer.
#[cfg(feature = "report-es2")]
#[test]
fn rgb10a2_fallback_es2_report_refuses_the_format() {
    let _g = context();
    let mut rb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, 2, 2);
        assert_eq!(err(), GL_INVALID_ENUM);
        glDeleteRenderbuffers(1, &rb);
        glBlitFramebuffer(0, 0, 2, 2, 0, 0, 2, 2, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_INVALID_OPERATION);
    }
    read_pair_is_rgba8();
}
