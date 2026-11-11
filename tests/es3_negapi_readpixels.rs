// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels from a framebuffer object it may not read (ES 3.0.6 4.3.2;
//! dEQP-GLES3 negative_api.buffer.read_pixels_fbo_format_mismatch,
//! docs/notes/es3-negative-api.md section 3): a (format, type) pair the read
//! buffer's class does not take is GL_INVALID_OPERATION (RGBA8, RGBA32I and
//! RGBA32UI read as `GL_RGBA`/`GL_FLOAT`), and so is any read from a complete
//! framebuffer whose GL_SAMPLE_BUFFERS is 1. The two integer cases follow the
//! back end's sampled classes: where RGBA32I/RGBA32UI are not stored (the
//! GC7000UL today) the texture's refusal is checked instead.
//!
//! dEQP's last step makes its sample buffers with
//! `glRenderbufferStorageMultisample(.., 4, GL_RGBA8, 32, 32)`. That needs a
//! back end that keeps 4 samples (GL_MAX_SAMPLES >= 4, the `max_samples`
//! capability of docs/notes/es3-msaa-renderbuffers.md). The 4-sample test
//! reads GL_MAX_SAMPLES and checks whichever rule the context is under: the
//! whole dEQP sequence at 4 or more, the GL_INVALID_OPERATION of ES 3.0 4.4.2
//! below it; neither is skipped. The rule of glReadPixels itself is pinned
//! without the capability through one-sample storage, which every ES 3.0
//! context keeps (GL_RENDERBUFFER_SAMPLES 1, GL_SAMPLE_BUFFERS 1).
//!
//! The stand-in table below cannot say 4 yet: `YakoglBackend` has no
//! `max_samples` field until plan #339's `gl-msaa-cap` adds it to every
//! literal (TODO(gl-msaa-cap) at the table), and the one `#[ignore]`d test
//! names that item.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Not in a `report-es2` build: the ES 2.0 surface has no integer formats and
//! no sample buffers on a framebuffer object.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_negapi_readpixels
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, gles_texfmt_bit, sampled_texfmts, GlesDraw, GlesHeap, YakoglBackend, GLES_TEXFMT_RGBA32I,
    GLES_TEXFMT_RGBA32UI,
};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glGetError, glGetIntegerv};
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

/// TODO(gl-msaa-cap): give this table `max_samples: 4` when plan #339 adds
/// the field, and un-ignore `stand_in_keeps_four_samples`; the 4-sample test
/// then runs dEQP's whole sequence here.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-negapi-readpixels".as_ptr() as *const c_char,
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

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv({pname:#x})");
    v
}

/// A framebuffer object bound to GL_FRAMEBUFFER, and what is attached to its
/// colour 0; all of it deleted and the default framebuffer bound on drop.
struct Fbo {
    fb: GLuint,
    tex: GLuint,
    rb: GLuint,
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
        Fbo { fb, tex: 0, rb: 0 }
    }

    /// A 32 x 32 texture of `internalformat` (dEQP's sizes) as colour 0: the
    /// error of its glTexImage2D.
    fn texture(&mut self, internalformat: GLenum, format: GLenum, type_: GLenum) -> GLenum {
        // SAFETY: room for one name; no pixels; plain GL calls.
        unsafe {
            glGenTextures(1, &mut self.tex);
            glBindTexture(GL_TEXTURE_2D, self.tex);
            glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 32, 32, 0, format, type_, core::ptr::null());
            let e = glGetError();
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, self.tex, 0);
            e
        }
    }

    /// A 32 x 32 RGBA8 renderbuffer of `samples` as colour 0: the error of
    /// its glRenderbufferStorageMultisample.
    fn renderbuffer(&mut self, samples: GLsizei) -> GLenum {
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut self.rb);
            glBindRenderbuffer(GL_RENDERBUFFER, self.rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 32, 32);
            let e = glGetError();
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, self.rb);
            e
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
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

/// The error of a one-pixel glReadPixels at the origin.
fn read(format: GLenum, type_: GLenum) -> GLenum {
    let mut px = [0u8; 16];
    // SAFETY: one pixel of at most 16 bytes into `px`.
    unsafe { glReadPixels(0, 0, 1, 1, format, type_, px.as_mut_ptr() as *mut c_void) };
    err()
}

/* ---- the format mismatches (passed before #343, pinned) ------------------- */

#[test]
fn rgba8_read_as_float_is_invalid_operation() {
    let _g = context();
    let mut f = Fbo::new();
    assert_eq!(f.texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE), GL_NO_ERROR);
    assert_eq!(f.status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(read(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION);
    /* the control: the pair every normalised target takes */
    assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE), GL_NO_ERROR);
}

/// The 32-bit integer case of dEQP for `internalformat`, stored in texel
/// class `class`. A back end that does not sample the class gets no texture
/// (glTexImage2D GL_INVALID_OPERATION; the GC7000UL until its 128-bit classes
/// open, docs/notes/gc-128bit-texture-sampling.md), which is checked instead.
fn int32_read_as_float(internalformat: GLenum, class: u8, type_: GLenum) {
    let _g = context();
    let mut f = Fbo::new();
    let e = f.texture(internalformat, GL_RGBA_INTEGER, type_);
    if sampled_texfmts() & gles_texfmt_bit(class) == 0 {
        assert_eq!(e, GL_INVALID_OPERATION, "texture {internalformat:#x}, class {class} not sampled");
        eprintln!("texture {internalformat:#x}: class {class} not sampled by this back end, its refusal checked");
        return;
    }
    assert_eq!(e, GL_NO_ERROR, "texture {internalformat:#x}");
    assert_eq!(f.status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(read(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION);
    assert_eq!(read(GL_RGBA_INTEGER, type_), GL_NO_ERROR);
}

#[test]
fn rgba32i_read_as_float_is_invalid_operation() {
    int32_read_as_float(GL_RGBA32I, GLES_TEXFMT_RGBA32I, GL_INT);
}

#[test]
fn rgba32ui_read_as_float_is_invalid_operation() {
    int32_read_as_float(GL_RGBA32UI, GLES_TEXFMT_RGBA32UI, GL_UNSIGNED_INT);
}

/* ---- a framebuffer with sample buffers ------------------------------------ */

/// The rule of glReadPixels without the capability: one-sample storage is a
/// framebuffer with GL_SAMPLE_BUFFERS 1, and it is not read from.
#[test]
fn one_sample_renderbuffer_read_is_invalid_operation() {
    let _g = context();
    let mut f = Fbo::new();
    assert_eq!(f.renderbuffer(1), GL_NO_ERROR);
    assert_eq!(f.status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(geti(GL_SAMPLE_BUFFERS), 1);
    assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE), GL_INVALID_OPERATION);
}

/// The same through GL_READ_FRAMEBUFFER alone, the draw framebuffer being
/// the default one (glReadPixels reads the read binding, ES 3.0 4.4.1).
#[test]
fn sample_buffers_on_the_read_binding_alone_refuse_the_read() {
    let _g = context();
    let mut f = Fbo::new();
    assert_eq!(f.renderbuffer(1), GL_NO_ERROR);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        assert_eq!(glCheckFramebufferStatus(GL_READ_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE), GL_INVALID_OPERATION);
    /* the control: the default framebuffer as the read one is read */
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, 0) };
    assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE), GL_NO_ERROR);
}

/// dEQP's last step, under the context's GL_MAX_SAMPLES: with 4 or more the
/// whole sequence (no error, complete, GL_SAMPLE_BUFFERS 1, the read
/// GL_INVALID_OPERATION); below 4 the storage request is the error
/// (ES 3.0 4.4.2: more samples than GL_MAX_SAMPLES), and dEQP reports Fail.
#[test]
fn four_sample_renderbuffer_read_is_invalid_operation() {
    let _g = context();
    let max = geti(GL_MAX_SAMPLES);
    assert!(max >= 1, "GL_MAX_SAMPLES {max}");
    let mut f = Fbo::new();
    let e = f.renderbuffer(4);
    if max < 4 {
        assert_eq!(e, GL_INVALID_OPERATION, "4 samples past GL_MAX_SAMPLES {max}");
        assert_eq!(geti(GL_SAMPLE_BUFFERS), 0);
        return;
    }
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(f.status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(geti(GL_SAMPLE_BUFFERS), 1);
    assert_eq!(read(GL_RGBA, GL_UNSIGNED_BYTE), GL_INVALID_OPERATION);
}

/// The stand-in back end keeps 4 samples, so the test above runs dEQP's
/// whole sequence on the host.
#[test]
#[ignore = "plan #339 gl-msaa-cap: YakoglBackend has no max_samples field, so the stand-in cannot say 4 (GL_MAX_SAMPLES is 1)"]
fn stand_in_keeps_four_samples() {
    let _g = context();
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}
