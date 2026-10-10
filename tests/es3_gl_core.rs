// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 GL-core conformance fixes that need no GPU (item gl-core-misc of
//! plan #47): one test per root cause, through the GL API on the host, with
//! the dEQP-GLES3 cases each one stands for.
//!
//!   - GL_RGB10_A2 is renderbuffer storage (ES 3.0 table 3.13):
//!     state_query.rbo.renderbuffer_internal_format, the RGB10_A2 rows of
//!     fbo.completeness.renderable.renderbuffer.color0 and of fbo.color.*.
//!   - A one-sample renderbuffer says it has one sample (ES 3.0 4.4.2.1),
//!     with ES 3.0's multisample completeness and read rules:
//!     state_query.rbo.renderbuffer_samples.
//!   - A texture of an ES 3.0 sized format (integer, snorm, packed float,
//!     RGB10_A2) answers the attachment queries with its own component type
//!     and sizes: state_query.fbo.framebuffer_attachment_component_type and
//!     framebuffer_attachment_x_size_texture.
//!   - glUnmapBufferOES and glGetBufferPointervOES take every ES 3.0 target,
//!     as the core calls do (dEQP's loader calls them in their place):
//!     state_query.buffer_object.buffer_mapped_*, buffer_access_flags_*,
//!     buffer_map_offset_length_*, negative_api.buffer.copy_buffer_sub_data.
//!   - glDrawBuffers on a framebuffer object checks n against
//!     GL_MAX_DRAW_BUFFERS before the enums: negative_api.buffer.draw_buffers.
//!   - glReadPixels into a pixel pack buffer stays inside the buffer and
//!     writes the rows back from the CPU cache: pbo.* (the GL half; the
//!     integer rows also need the back ends' integer render targets).
//!
//! The context is the smallest OSAL and back end that make one, as in
//! tests/es3_int_rt.rs; no pixel is rendered. An ES 2.0-reporting build
//! (`report-es2`) runs `es2_report_is_unchanged` only: none of the above
//! exists on that surface.

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::{
    glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers, glGetBufferPointervOES, glMapBufferRange, glUnmapBuffer,
    glUnmapBufferOES,
};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glDeleteRenderbuffers,
    glDrawBuffers, glFramebufferRenderbuffer, glFramebufferTexture2D, glGenFramebuffers, glGenRenderbuffers,
    glGetFramebufferAttachmentParameteriv, glGetRenderbufferParameteriv, glReadPixels, glRenderbufferStorage,
    glRenderbufferStorageMultisample,
};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexStorage2D};
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-gl-core".as_ptr() as *const c_char,
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

/// A renderbuffer with `samples` samples of `format` (glRenderbufferStorage
/// for None), bound.
fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei, samples: Option<GLsizei>) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        match samples {
            None => glRenderbufferStorage(GL_RENDERBUFFER, format, w, h),
            Some(s) => glRenderbufferStorageMultisample(GL_RENDERBUFFER, s, format, w, h),
        }
    }
    rb
}

/// A framebuffer, bound, with nothing attached.
fn framebuffer() -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    fb
}

fn attach_rb(attachment: GLenum, rb: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb) };
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn rb_param(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    v
}

fn fb_param(attachment: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, attachment, pname, &mut v) };
    v
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
}

/* ---- the ES 2.0 report ---------------------------------------------------- */

/// None of the four changes reaches the ES 2.0 surface: GL_RGB10_A2 is no
/// renderbuffer storage there, GL_RENDERBUFFER_SAMPLES is no pname, and a
/// one-sample storage is plain storage that reads like any other.
#[test]
#[cfg(feature = "report-es2")]
fn es2_report_is_unchanged() {
    let _g = context();
    let rb = renderbuffer(GL_RGB10_A2, 4, 4, None);
    assert_eq!(err(), GL_INVALID_ENUM, "RGB10_A2 is not an ES 2.0 renderbuffer format");
    assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), 0);
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };

    let rb = renderbuffer(GL_RGBA8, 4, 4, Some(1));
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(rb_param(GL_RENDERBUFFER_SAMPLES), -1);
    assert_eq!(err(), GL_INVALID_ENUM, "no GL_RENDERBUFFER_SAMPLES in ES 2.0");
    let fb = framebuffer();
    attach_rb(GL_COLOR_ATTACHMENT0, rb);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    let mut px = [0u8; 16];
    // SAFETY: room for 2 x 2 RGBA8 pixels.
    unsafe { glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "an ES 2.0 framebuffer has no sample buffers to refuse a read for");
    release(fb, &[rb]);

    /* GL_OES_mapbuffer's unmap takes only the extension's two targets */
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glUnmapBufferOES(GL_COPY_READ_BUFFER) }, GL_FALSE as GLboolean);
    assert_eq!(err(), GL_INVALID_ENUM);
}

/* ---- ES 3.0 --------------------------------------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The colour formats ES 3.0 requires to be renderbuffer storage (table
    /// 3.13), in the order dEQP-GLES3 state_query.rbo.renderbuffer_internal_format
    /// stores them one after the other into a single renderbuffer.
    const REQUIRED_COLOR: [GLenum; 28] = [
        GL_R8,
        GL_RG8,
        GL_RGB8,
        GL_RGB565,
        GL_RGBA4,
        GL_RGB5_A1,
        GL_RGBA8,
        GL_RGB10_A2,
        GL_RGB10_A2UI,
        GL_SRGB8_ALPHA8,
        GL_R8I,
        GL_R8UI,
        GL_R16I,
        GL_R16UI,
        GL_R32I,
        GL_R32UI,
        GL_RG8I,
        GL_RG8UI,
        GL_RG16I,
        GL_RG16UI,
        GL_RG32I,
        GL_RG32UI,
        GL_RGBA8I,
        GL_RGBA8UI,
        GL_RGBA16I,
        GL_RGBA16UI,
        GL_RGBA32I,
        GL_RGBA32UI,
    ];

    /// Root cause 1: GL_RGB10_A2 was refused as renderbuffer storage with
    /// GL_INVALID_ENUM (is_color_format did not know it), which failed
    /// state_query.rbo.renderbuffer_internal_format (and every RGB10_A2
    /// renderbuffer row of fbo.*). It is storage now, held as RGBA8 like its
    /// texture, colour-renderable, unsigned normalised, and its sizes say
    /// the eight bits it really has.
    #[test]
    fn rgb10_a2_is_colour_renderable_renderbuffer_storage() {
        let _g = context();
        let rb = renderbuffer(GL_RGBA4, 4, 4, None);
        for f in REQUIRED_COLOR {
            // SAFETY: plain GL call.
            unsafe { glRenderbufferStorage(GL_RENDERBUFFER, f, 16, 16) };
            assert_eq!(err(), GL_NO_ERROR, "0x{f:x} is required renderbuffer storage");
            assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT) as GLenum, f);
        }

        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, 5, 3) };
        assert_eq!(err(), GL_NO_ERROR);
        let sizes = [
            rb_param(GL_RENDERBUFFER_RED_SIZE),
            rb_param(GL_RENDERBUFFER_GREEN_SIZE),
            rb_param(GL_RENDERBUFFER_BLUE_SIZE),
            rb_param(GL_RENDERBUFFER_ALPHA_SIZE),
        ];
        assert_eq!(sizes, [8; 4], "held as RGBA8, and said so (not 10/10/10/2)");
        let fb = framebuffer();
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "colour-renderable (ES 3.0 4.4.4)");
        assert_eq!(
            fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE) as GLenum,
            GL_UNSIGNED_NORMALIZED
        );
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE), 8);
        /* read back as any normalised buffer is */
        assert_eq!(
            (geti(GL_IMPLEMENTATION_COLOR_READ_FORMAT) as GLenum, geti(GL_IMPLEMENTATION_COLOR_READ_TYPE) as GLenum),
            (GL_RGBA, GL_UNSIGNED_BYTE)
        );
        assert_eq!(err(), GL_NO_ERROR);
        release(fb, &[rb]);
    }

    /// Root cause 2: GL_RENDERBUFFER_SAMPLES was always 0, so storage made
    /// with one sample (GL_MAX_SAMPLES is 1) read back below the count asked
    /// for (dEQP-GLES3 state_query.rbo.renderbuffer_samples; ES 3.0 4.4.2.1:
    /// at least `samples`). The count is kept now, and the rules that come
    /// with it hold: the framebuffer has sample buffers, mixing counts is
    /// GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE (4.4.4.2), and glReadPixels from
    /// it is GL_INVALID_OPERATION (4.3.2).
    #[test]
    fn a_one_sample_renderbuffer_has_one_sample() {
        let _g = context();
        assert_eq!(geti(GL_MAX_SAMPLES), 1);
        let ms = renderbuffer(GL_RGBA8, 4, 4, Some(0));
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(rb_param(GL_RENDERBUFFER_SAMPLES), 0, "0 is plain storage");
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA8, 4, 4) };
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(rb_param(GL_RENDERBUFFER_SAMPLES), 1, "at least the one asked for");
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 4, 4) };
        assert_eq!(rb_param(GL_RENDERBUFFER_SAMPLES), 0, "glRenderbufferStorage is zero samples");
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA8, 4, 4) };

        let fb = framebuffer();
        attach_rb(GL_COLOR_ATTACHMENT0, ms);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
        let mut px = [0u8; 16];
        // SAFETY: room for 2 x 2 RGBA8 pixels.
        unsafe { glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_INVALID_OPERATION, "no read from a framebuffer with sample buffers");

        /* a zero-sample depth buffer beside it: the counts differ */
        let depth = renderbuffer(GL_DEPTH_COMPONENT16, 4, 4, None);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
        assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "an incomplete framebuffer has none");
        /* the same count: complete again */
        // SAFETY: plain GL call (the depth renderbuffer is bound).
        unsafe { glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_DEPTH_COMPONENT16, 4, 4) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        /* a texture beside a one-sample renderbuffer is incomplete too */
        let mut tex = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 4, 4);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
            glDeleteTextures(1, &tex);
        }
        /* plain storage on both: an ordinary framebuffer, read as before */
        attach_rb(GL_COLOR_ATTACHMENT0, ms);
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, ms);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 4, 4);
            glBindRenderbuffer(GL_RENDERBUFFER, depth);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 4, 4);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
        // SAFETY: room for 2 x 2 RGBA8 pixels.
        unsafe { glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR);
        release(fb, &[ms, depth]);
    }

    /// Root cause 3: a texture of an es3_formats format answered
    /// GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE with GL_NONE and every size
    /// with 0 (texture_bits read the base format, GL_RGBA_INTEGER and such,
    /// which its table did not know): dEQP-GLES3 state_query.fbo.
    /// framebuffer_attachment_component_type and _x_size_texture. Each
    /// format the back end of this build stores is checked; a format it
    /// refuses by name (glTexStorage2D's GL_INVALID_OPERATION) is the
    /// formats-* items' and is skipped. RGB10_A2 every back end stores.
    #[test]
    fn es3_texture_attachments_answer_their_type_and_sizes() {
        let _g = context();
        let fb = framebuffer();
        let (u, s, i, ui, f) = (GL_UNSIGNED_NORMALIZED, GL_SIGNED_NORMALIZED, GL_INT, GL_UNSIGNED_INT, GL_FLOAT);
        /* (format, component type, at least these sizes) -- dEQP's minimums */
        let cases: [(GLenum, GLenum, [GLint; 4]); 12] = [
            (GL_RGB10_A2, u, [8, 8, 8, 2]),
            (GL_RGB10_A2UI, ui, [10, 10, 10, 2]),
            (GL_RGBA8_SNORM, s, [8, 8, 8, 8]),
            (GL_R11F_G11F_B10F, f, [10, 10, 10, 0]),
            (GL_R8I, i, [8, 0, 0, 0]),
            (GL_R16UI, ui, [16, 0, 0, 0]),
            (GL_RG32UI, ui, [32, 32, 0, 0]),
            (GL_RG32I, i, [32, 32, 0, 0]),
            (GL_RGBA8I, i, [8, 8, 8, 8]),
            (GL_RGBA8UI, ui, [8, 8, 8, 8]),
            (GL_RGBA16I, i, [16, 16, 16, 16]),
            (GL_RGBA32UI, ui, [32, 32, 32, 32]),
        ];
        let mut checked = 0;
        for (format, ct, min) in cases {
            let mut tex = 0;
            // SAFETY: room for one name; plain GL calls.
            let stored = unsafe {
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexStorage2D(GL_TEXTURE_2D, 1, format, 8, 8);
                glGetError() == GL_NO_ERROR
            };
            if stored {
                // SAFETY: plain GL call.
                unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0) };
                assert_eq!(err(), GL_NO_ERROR, "0x{format:x}");
                assert_eq!(
                    fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE) as GLenum,
                    ct,
                    "0x{format:x}: component type"
                );
                let sizes = [
                    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
                    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
                    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
                    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
                ]
                .map(|p| fb_param(GL_COLOR_ATTACHMENT0, p));
                for (&got, &want) in sizes.iter().zip(&min) {
                    assert!(got >= want, "0x{format:x}: sizes {sizes:?}, at least {min:?}");
                    /* a channel the format lacks is 0 */
                    assert!(want != 0 || got == 0, "0x{format:x}: sizes {sizes:?}");
                }
                assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE), 0);
                assert_eq!(err(), GL_NO_ERROR, "0x{format:x}");
                checked += 1;
            }
            // SAFETY: plain GL calls.
            unsafe {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
                glDeleteTextures(1, &tex);
            }
        }
        assert!(checked >= 1, "RGB10_A2 at least (its RGBA8 fallback is on every back end)");
        release(fb, &[]);
    }

    /// Root cause 5: glUnmapBufferOES and glGetBufferPointervOES took only
    /// GL_ARRAY_BUFFER and GL_ELEMENT_ARRAY_BUFFER. dEQP's GL loader
    /// (glwInitExtES.inl) calls them in place of glUnmapBuffer and
    /// glGetBufferPointerv whenever GL_OES_mapbuffer is listed, so every
    /// unmap of another target read GL_INVALID_ENUM and left the buffer
    /// mapped: state_query.buffer_object.buffer_mapped_*,
    /// buffer_access_flags_*, buffer_map_offset_length_* and
    /// negative_api.buffer.copy_buffer_sub_data. In an ES 3.0 build they are
    /// the core commands under another name.
    #[test]
    fn the_oes_unmap_and_pointer_calls_take_every_es3_target() {
        let _g = context();
        let mut buf = 0;
        let data = [0u8; 16];
        for target in [GL_COPY_READ_BUFFER, GL_UNIFORM_BUFFER, GL_PIXEL_PACK_BUFFER, GL_ARRAY_BUFFER] {
            // SAFETY: room for one name; `data` holds the 16 bytes; plain GL calls.
            unsafe {
                glGenBuffers(1, &mut buf);
                glBindBuffer(target, buf);
                glBufferData(target, 16, data.as_ptr() as *const c_void, GL_DYNAMIC_COPY);
                let mapped = glMapBufferRange(target, 4, 8, GL_MAP_WRITE_BIT);
                assert!(!mapped.is_null(), "0x{target:x}");
                let mut p = core::ptr::null_mut();
                glGetBufferPointervOES(target, GL_BUFFER_MAP_POINTER, &mut p);
                assert_eq!(glGetError(), GL_NO_ERROR, "0x{target:x}: pointer");
                assert_eq!(p, mapped, "0x{target:x}");
                assert_eq!(glUnmapBufferOES(target), GL_TRUE as GLboolean, "0x{target:x}");
                assert_eq!(glGetError(), GL_NO_ERROR, "0x{target:x}: unmap");
                assert_eq!(glUnmapBuffer(target), GL_FALSE as GLboolean, "0x{target:x}: no longer mapped");
                assert_eq!(glGetError(), GL_INVALID_OPERATION);
                glBindBuffer(target, 0);
                glDeleteBuffers(1, &buf);
            }
        }
        // SAFETY: plain GL call.
        unsafe { glUnmapBufferOES(0x1234) };
        assert_eq!(err(), GL_INVALID_ENUM, "no target at all");
    }

    /// Root cause 6: glDrawBuffers checked the enums before n on a
    /// framebuffer object, so GL_MAX_DRAW_BUFFERS + 1 buffers with a
    /// GL_DEPTH_ATTACHMENT among them read GL_INVALID_ENUM where
    /// dEQP-GLES3 negative_api.buffer.draw_buffers wants GL_INVALID_VALUE.
    /// The calls before it in that case keep their answers.
    #[test]
    fn draw_buffers_on_an_object_checks_the_count_first() {
        let _g = context();
        let max = geti(GL_MAX_DRAW_BUFFERS);
        let mut values = vec![GL_NONE; max as usize + 1];
        values[..4].copy_from_slice(&[GL_NONE, GL_BACK, GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT]);
        let rb = renderbuffer(GL_RGBA8, 4, 4, None);
        let fb = framebuffer();
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        // SAFETY: each call reads at most `values.len()` enums from it.
        unsafe {
            glDrawBuffers(2, values[2..].as_ptr());
            assert_eq!(glGetError(), GL_INVALID_ENUM, "{{COLOR_ATTACHMENT0, DEPTH_ATTACHMENT}}");
            glDrawBuffers(1, values[1..].as_ptr());
            assert_eq!(glGetError(), GL_INVALID_OPERATION, "GL_BACK on an object");
            glDrawBuffers(-1, values[1..].as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            glDrawBuffers(max + 1, values.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE, "n past GL_MAX_DRAW_BUFFERS, a bad enum among them");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDrawBuffers(2, values.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_OPERATION, "the window, n = 2");
            glDrawBuffers(1, values[2..].as_ptr());
            assert_eq!(glGetError(), GL_INVALID_OPERATION, "the window, COLOR_ATTACHMENT0");
        }
        release(fb, &[rb]);
    }

    /// Root cause 4: glReadPixels with a pixel pack buffer bound checked the
    /// buffer as if the read touched no byte, so a read larger than the
    /// buffer wrote past its store; and the rows it wrote stayed in the CPU
    /// cache, which a later glMapBufferRange(GL_MAP_READ_BIT) invalidates
    /// (dEQP-GLES3 pbo.*: read into a buffer, map it for reading). ES 3.0
    /// 4.3.2: the whole pack footprint, skips and row padding included,
    /// lies inside the buffer or the read is GL_INVALID_OPERATION.
    #[test]
    fn read_pixels_into_a_pack_buffer_stays_inside_it_and_reaches_memory() {
        let _g = context();
        let rb = renderbuffer(GL_RGBA8, 4, 4, None);
        let fb = framebuffer();
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let mut pbo = 0;
        let fill = [0xAAu8; 64];
        // SAFETY: room for one name; `fill` holds the 64 bytes the store takes.
        unsafe {
            glGenBuffers(1, &mut pbo);
            glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
            glBufferData(GL_PIXEL_PACK_BUFFER, 64, fill.as_ptr() as *const c_void, GL_STREAM_READ);
        }
        assert_eq!(err(), GL_NO_ERROR);

        /* 2 x 2 RGBA8 at offset 4: 16 bytes, inside the 64 */
        FLUSHES.lock().unwrap().clear();
        // SAFETY: with a pack buffer bound the pointer is an offset.
        unsafe { glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, 4 as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR);
        let flushed = FLUSHES.lock().unwrap().clone();
        assert!(flushed.iter().any(|&(_, n)| n == 16), "the 16 bytes written are written back: {flushed:?}");

        // SAFETY: a mapping of the 64 bytes, read while it lasts.
        let bytes = unsafe {
            let p = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, 64, GL_MAP_READ_BIT) as *const u8;
            assert!(!p.is_null());
            let v = core::slice::from_raw_parts(p, 64).to_vec();
            glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
            v
        };
        assert_eq!(bytes[..4], [0xAA; 4], "before the offset: untouched");
        assert_eq!(bytes[4..20], [0u8; 16], "the four pixels of the cleared storage");
        assert_eq!(bytes[20..], [0xAA; 44], "after the rows: untouched");

        /* the footprint past the end: 4 x 4 RGBA8 is 64 bytes, at offset 4 */
        // SAFETY: as above.
        unsafe { glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, 4 as *mut c_void) };
        assert_eq!(err(), GL_INVALID_OPERATION, "a read larger than the buffer");
        /* the same read at offset 0 fits exactly */
        // SAFETY: as above.
        unsafe { glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null_mut()) };
        assert_eq!(err(), GL_NO_ERROR, "64 bytes into 64");
        /* the pack skips count: one skipped row of 4 pixels makes it 80 */
        // SAFETY: plain GL calls; the pointer is an offset.
        unsafe {
            glPixelStorei(GL_PACK_SKIP_ROWS, 1);
            glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null_mut());
            glPixelStorei(GL_PACK_SKIP_ROWS, 0);
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "the skipped row is part of the footprint");
        /* the last row is not padded: 3 x 2 at alignment 8 is 16 + 12 = 28 bytes */
        // SAFETY: plain GL calls; the pointer is an offset.
        unsafe {
            glPixelStorei(GL_PACK_ALIGNMENT, 8);
            glReadPixels(0, 0, 3, 2, GL_RGBA, GL_UNSIGNED_BYTE, 36 as *mut c_void);
            glPixelStorei(GL_PACK_ALIGNMENT, 4);
        }
        assert_eq!(err(), GL_NO_ERROR, "36 + 28 = 64");

        // SAFETY: names this test made.
        unsafe {
            glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
            glDeleteBuffers(1, &pbo);
        }
        release(fb, &[rb]);
    }
}
