// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 reads from a multisample read framebuffer, on the host
//! (docs/notes/es3-msaa-renderbuffers.md; ES 3.0 4.3.2 and 3.8.5).
//!
//! glReadPixels from a read framebuffer whose GL_SAMPLE_BUFFERS is 1 is
//! GL_INVALID_OPERATION (4.3.2), and so are glCopyTexImage2D,
//! glCopyTexSubImage2D and glCopyTexSubImage3D (3.8.5). The GL core checks
//! the first today (src/gl/framebuffer.rs); the passing tests pin that and
//! the single-sample reads next to it. The glCopyTex* ones have no check yet:
//! their tests are `#[ignore]`d with the owner named (gl-msaa-read-errors,
//! src/gl/texture.rs).
//!
//! The 4x framebuffers need a back end that says `max_samples` 4 (the
//! capability on); the plain stand-in table, 0, pins the capability off.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_read_errors
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

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

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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
    name: c"es3-msaa-read-errors".as_ptr() as *const c_char,
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

/// [`BACKEND`] with the multisample capability on: it keeps 4 samples.
static BACKEND_MS4: YakoglBackend = YakoglBackend { max_samples: 4, ..BACKEND };

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context with the stand-in back end of today: no multisample capability.
fn context() -> MutexGuard<'static, ()> {
    context_on(&BACKEND)
}

/// The context with a stand-in back end that says `max_samples` 4.
fn context_ms4() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_MS4)
}

fn context_on(table: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a static table; the lock above serialises the tests. Plain GL calls.
    unsafe {
        backend::g_backend = table;
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A framebuffer of one 8x8 RGBA8 renderbuffer, and what
/// glRenderbufferStorageMultisample said.
struct Fbo {
    fb: GLuint,
    rb: GLuint,
    storage_error: GLenum,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

/// A renderbuffer of `samples` as colour attachment 0 of a new framebuffer
/// bound to `target`.
fn rbo_fbo(target: GLenum, samples: GLsizei) -> Fbo {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls; one name each out.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(target, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 8, 8);
    }
    let storage_error = err();
    // SAFETY: names made above.
    unsafe { glFramebufferRenderbuffer(target, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
    assert_eq!(err(), GL_NO_ERROR);
    Fbo { fb, rb, storage_error }
}

/// A complete single-sample 8x8 RGBA8 framebuffer (a texture), bound to `target`.
fn single_fbo(target: GLenum) -> (GLuint, GLuint) {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: plain GL calls; one name each out.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(target, fb);
        glFramebufferTexture2D(target, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(target), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR);
    (fb, tex)
}

const SENTINEL: u8 = 0x11;

/// glReadPixels of the 8x8 of the read framebuffer: its error and the bytes
/// (SENTINEL where nothing was written).
fn read_pixels() -> (GLenum, Vec<[u8; 4]>) {
    let mut px = vec![[SENTINEL; 4]; 64];
    // SAFETY: room for 8x8 RGBA/UNSIGNED_BYTE pixels (rows of 32 bytes).
    unsafe {
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    (err(), px)
}

fn untouched(px: &[[u8; 4]]) -> bool {
    px.iter().all(|p| *p == [SENTINEL; 4])
}

/* ---- passing pins: glReadPixels -------------------------------------------- */

/// (a) A 4x read framebuffer, a complete single-sample draw framebuffer:
/// GL_INVALID_OPERATION, nothing written.
#[test]
fn read_pixels_from_4x_read_framebuffer_is_invalid_operation() {
    let _g = context_ms4();
    let (draw_fb, tex) = single_fbo(GL_DRAW_FRAMEBUFFER);
    let src = rbo_fbo(GL_READ_FRAMEBUFFER, 4);
    assert_eq!(src.storage_error, GL_NO_ERROR);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_READ_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
    let (e, px) = read_pixels();
    assert_eq!(e, GL_INVALID_OPERATION);
    assert!(untouched(&px), "nothing is written");
    drop(src);
    // SAFETY: names this test made.
    unsafe {
        glDeleteFramebuffers(1, &draw_fb);
        glDeleteTextures(1, &tex);
    }
}

/// (b) The same with the 4x framebuffer bound to GL_FRAMEBUFFER.
#[test]
fn read_pixels_from_4x_framebuffer_is_invalid_operation() {
    let _g = context_ms4();
    let src = rbo_fbo(GL_FRAMEBUFFER, 4);
    assert_eq!(src.storage_error, GL_NO_ERROR);
    let (e, px) = read_pixels();
    assert_eq!(e, GL_INVALID_OPERATION);
    assert!(untouched(&px), "nothing is written");
}

/// (c) A one-sample renderbuffer (samples 0) reads back without an error,
/// on the 4x-capable table too.
#[test]
fn read_pixels_from_1x_renderbuffer_fbo_is_fine() {
    let _g = context_ms4();
    let src = rbo_fbo(GL_FRAMEBUFFER, 0);
    assert_eq!(src.storage_error, GL_NO_ERROR);
    let (e, px) = read_pixels();
    assert_eq!(e, GL_NO_ERROR);
    assert!(!untouched(&px), "the 8x8 is written");
}

/// (d) The capability off (`max_samples` 0): 4 samples are
/// GL_INVALID_OPERATION as tests/es3_msaa_gl.rs pins it; the renderbuffer is
/// left without storage, so its framebuffer is not complete and a read of it
/// is refused. 0 samples read fine; 1 sample is read as a sample buffer.
#[test]
fn capability_off_storage_and_read() {
    let _g = context();
    let src = rbo_fbo(GL_FRAMEBUFFER, 4);
    assert_eq!(src.storage_error, GL_INVALID_OPERATION);
    // SAFETY: plain GL call.
    let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_ne!(status, GL_FRAMEBUFFER_COMPLETE, "a renderbuffer without storage");
    let (e, px) = read_pixels();
    assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION);
    assert!(untouched(&px));
    drop(src);

    /* samples 0 is plain storage and reads fine */
    let zero = rbo_fbo(GL_FRAMEBUFFER, 0);
    assert_eq!(zero.storage_error, GL_NO_ERROR);
    let (e, px) = read_pixels();
    assert_eq!(e, GL_NO_ERROR);
    assert!(!untouched(&px));
    drop(zero);

    /* samples 1 stores GL_RENDERBUFFER_SAMPLES 1, which the read check
     * (`framebuffer_samples(c) > 0`) takes for a sample buffer: today it is
     * GL_INVALID_OPERATION, with nothing written */
    let one = rbo_fbo(GL_FRAMEBUFFER, 1);
    assert_eq!(one.storage_error, GL_NO_ERROR);
    let (e, px) = read_pixels();
    assert_eq!(e, GL_INVALID_OPERATION);
    assert!(untouched(&px));
}

/* ---- the glCopyTex* errors, owned by the GL core's texture code ----------- */

const FILL: u8 = 0x7f;

/// An 8x8 RGBA8 2D texture filled with FILL, left bound.
fn filled_2d() -> GLuint {
    let data = vec![FILL; 8 * 8 * 4];
    let mut tex = 0;
    // SAFETY: plain GL calls; 8x8 RGBA bytes.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            8,
            8,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            data.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR);
    tex
}

/// An 8x8x2 RGBA8 texture of `target` (3D or 2D array) filled with FILL, left bound.
fn filled_3d(target: GLenum) -> GLuint {
    let data = vec![FILL; 8 * 8 * 2 * 4];
    let mut tex = 0;
    // SAFETY: plain GL calls; 8x8x2 RGBA bytes.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(target, tex);
        glTexImage3D(
            target,
            0,
            GL_RGBA8 as GLint,
            8,
            8,
            2,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            data.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR);
    tex
}

/// Is the 8x8 of a 2D texture (layer `layer` of a layered one) still all
/// FILL, read back through a single-sample framebuffer?
fn texture_is_unchanged(target: GLenum, tex: GLuint, layer: GLint) -> bool {
    let mut fb = 0;
    let mut px = vec![[0u8; 4]; 64];
    // SAFETY: plain GL calls; room for 8x8 RGBA bytes.
    let complete = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if target == GL_TEXTURE_2D {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        } else {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer);
        }
        let complete = glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE;
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        complete
    };
    complete && err() == GL_NO_ERROR && px.iter().all(|p| *p == [FILL; 4])
}

/// A 4x read framebuffer whose colour is the copy source.
fn ms4_source() -> Fbo {
    let src = rbo_fbo(GL_READ_FRAMEBUFFER, 4);
    assert_eq!(src.storage_error, GL_NO_ERROR);
    // SAFETY: plain GL calls.
    unsafe {
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
    }
    src
}

/// ES 3.0 3.8.5: glCopyTexImage2D from a 4x read framebuffer.
#[test]
#[ignore = "owner: GL core, src/gl/texture.rs (gl-msaa-read-errors)"]
fn copy_tex_image_2d_from_4x_is_invalid_operation() {
    let _g = context_ms4();
    let tex = filled_2d();
    let src = ms4_source();
    // SAFETY: plain GL call.
    unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, 4, 4, 0) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    drop(src);
    assert!(texture_is_unchanged(GL_TEXTURE_2D, tex, 0), "the texture is as it was (8x8)");
}

/// ES 3.0 3.8.5: glCopyTexSubImage2D from a 4x read framebuffer.
#[test]
#[ignore = "owner: GL core, src/gl/texture.rs (gl-msaa-read-errors)"]
fn copy_tex_sub_image_2d_from_4x_is_invalid_operation() {
    let _g = context_ms4();
    let tex = filled_2d();
    let src = ms4_source();
    // SAFETY: plain GL call.
    unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 8, 8) };
    assert_eq!(err(), GL_INVALID_OPERATION);
    drop(src);
    assert!(texture_is_unchanged(GL_TEXTURE_2D, tex, 0));
}

/// ES 3.0 3.8.5: glCopyTexSubImage3D onto a 3D texture and onto a 2D array.
#[test]
#[ignore = "owner: GL core, src/gl/texture.rs (gl-msaa-read-errors)"]
fn copy_tex_sub_image_3d_from_4x_is_invalid_operation() {
    let _g = context_ms4();
    for target in [GL_TEXTURE_3D, GL_TEXTURE_2D_ARRAY] {
        let tex = filled_3d(target);
        let src = ms4_source();
        // SAFETY: plain GL call.
        unsafe { glCopyTexSubImage3D(target, 0, 0, 0, 1, 0, 0, 8, 8) };
        assert_eq!(err(), GL_INVALID_OPERATION, "{target:#x}");
        drop(src);
        assert!(texture_is_unchanged(target, tex, 1), "{target:#x}: layer 1 is as it was");
    }
}
