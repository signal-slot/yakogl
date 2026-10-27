// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's integer colour render targets, through the GL API on the host
//! (`b9b2ac00`; the dEQP bucket: every
//! shaders.builtin_functions case renders into an integer renderbuffer and
//! died at glRenderbufferStorage with GL_INVALID_ENUM).
//!
//! What is held here is the GL layer's half: the nineteen colour-renderable
//! integer formats are renderbuffer storage and complete framebuffers;
//! glClearBuffer{f,i,ui}v and glReadPixels tell integer and normalised
//! buffers apart and say GL_INVALID_OPERATION for a mismatch;
//! GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE answers for them; and a back end
//! that has not learned a format refuses to draw into it by name instead of
//! rendering garbage. The pixels come from a stand-in back end whose `draw`
//! stores a clear's integers the way a back end that can render the format
//! must (`backend::HOST_INTEGER_RT_FORMATS` is what tells the GL core it can);
//! no GPU runs here. The back ends' own work is int-rt:
//! #1229, #1230, #1231.
//!
//! An ES 2.0-reporting build (`report-es2`) must still refuse every one of
//! the formats: `es2_build_refuses_the_integer_formats` is the only test it
//! runs. tools/check.sh runs this file in the gpu-powervr, gpu-vivante and
//! V3D rows, with and without report-es2.

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
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

/// Draws the stand-in back end was handed (clears only: the tests set no
/// program), and the ones that stored an integer clear.
static DRAWS: Mutex<Vec<(u8, u8, bool)>> = Mutex::new(Vec::new());

/// The back end that can render every integer surface (for the clears): a
/// clear of an integer target stores the low bits of the four raw values in
/// the surface's layout, little endian, rows at the pair-padded pitch.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for i in 0..d.num_targets as usize {
        let t = &d.targets[i];
        let hit = d.do_clear && (d.clear_target_mask == 0 || d.clear_target_mask & (1 << i) != 0);
        DRAWS.lock().unwrap().push((i as u8, t.format, hit && t.gpu != 0));
        let Some(l) = backend::gles_surf_int_layout(t.format) else { continue };
        if !hit || t.gpu == 0 {
            continue;
        }
        let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
        let pitch = ((t.w + 1) & !1) as usize;
        let v = d.clear_rgba.map(f32::to_bits);
        let mut px = vec![0u8; bpp];
        if l.bits == 10 {
            let w = (v[0] & 0x3ff) | (v[1] & 0x3ff) << 10 | (v[2] & 0x3ff) << 20 | (v[3] & 3) << 30;
            px.copy_from_slice(&w.to_le_bytes());
        } else {
            let step = l.bits as usize / 8;
            for ch in 0..l.channels as usize {
                px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
            }
        }
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
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
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-int-rt".as_ptr() as *const c_char,
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

/// Every integer surface, so the stand-in back end can draw them all.
const ALL_SURFACES: u32 = (1 << (backend::GLES_SURF_INT_LAST - backend::GLES_SURF_INT_FIRST + 1)) - 1;

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
    /* What an earlier test left recorded and unflushed (a pending clear: the
     * clear checks end on the default framebuffer) is settled here, under the
     * previous test's mask, before this test's draws are counted; else it
     * reaches the back end at this test's glFinish, in whatever order the
     * threads took the lock. */
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    DRAWS.lock().unwrap().clear();
    backend::HOST_INTEGER_RT_FORMATS.store(0, Ordering::Relaxed);
    drain_errors();
    g
}

fn drain_errors() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The integer formats ES 3.0 makes colour-renderable (table 3.13), with
/// their channels, bits a channel and signedness.
const INT_FORMATS: [(GLenum, &str, usize, u8, bool); 19] = [
    (GL_R8I, "R8I", 1, 8, true),
    (GL_R8UI, "R8UI", 1, 8, false),
    (GL_R16I, "R16I", 1, 16, true),
    (GL_R16UI, "R16UI", 1, 16, false),
    (GL_R32I, "R32I", 1, 32, true),
    (GL_R32UI, "R32UI", 1, 32, false),
    (GL_RG8I, "RG8I", 2, 8, true),
    (GL_RG8UI, "RG8UI", 2, 8, false),
    (GL_RG16I, "RG16I", 2, 16, true),
    (GL_RG16UI, "RG16UI", 2, 16, false),
    (GL_RG32I, "RG32I", 2, 32, true),
    (GL_RG32UI, "RG32UI", 2, 32, false),
    (GL_RGBA8I, "RGBA8I", 4, 8, true),
    (GL_RGBA8UI, "RGBA8UI", 4, 8, false),
    (GL_RGBA16I, "RGBA16I", 4, 16, true),
    (GL_RGBA16UI, "RGBA16UI", 4, 16, false),
    (GL_RGBA32I, "RGBA32I", 4, 32, true),
    (GL_RGBA32UI, "RGBA32UI", 4, 32, false),
    (GL_RGB10_A2UI, "RGB10_A2UI", 4, 10, false),
];

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

/// A framebuffer with `rbs` on colour attachments 0.., bound.
fn framebuffer(rbs: &[GLuint]) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, rb) in rbs.iter().enumerate() {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, *rb);
        }
    }
    fb
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
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

fn geti(pname: GLenum) -> GLint {
    let mut v = [i32::MIN; 4];
    // SAFETY: room for the widest answer.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    v[0]
}

fn read_u32(w: GLsizei, h: GLsizei, format: GLenum, type_: GLenum) -> Vec<u32> {
    let mut px = vec![0xdead_beefu32; (w * h * 4) as usize];
    // SAFETY: room for w x h pixels of four 32-bit values.
    unsafe { glReadPixels(0, 0, w, h, format, type_, px.as_mut_ptr() as *mut c_void) };
    px
}

/* ---- the ES 2.0 report refuses them ------------------------------------- */

#[test]
#[cfg(feature = "report-es2")]
fn es2_build_refuses_the_integer_formats() {
    let _g = context();
    for (f, name, ..) in INT_FORMATS {
        let mut rb = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f, 4, 4);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "{name}: an ES 2.0 report has no integer renderbuffer");
        assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), 0, "{name}: no storage was made");
        // SAFETY: a name this test made.
        unsafe { glDeleteRenderbuffers(1, &rb) };
    }
}

/* ---- ES 3.0: storage, completeness, queries ------------------------------ */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    #[test]
    fn every_integer_format_is_renderbuffer_storage() {
        let _g = context();
        for (f, name, channels, bits, signed) in INT_FORMATS {
            let rb = renderbuffer(f, 5, 3);
            assert_eq!(err(), GL_NO_ERROR, "{name}");
            assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT) as u32, f, "{name}");
            assert_eq!((rb_param(GL_RENDERBUFFER_WIDTH), rb_param(GL_RENDERBUFFER_HEIGHT)), (5, 3), "{name}");
            let sizes = [
                rb_param(GL_RENDERBUFFER_RED_SIZE),
                rb_param(GL_RENDERBUFFER_GREEN_SIZE),
                rb_param(GL_RENDERBUFFER_BLUE_SIZE),
                rb_param(GL_RENDERBUFFER_ALPHA_SIZE),
            ];
            let b = bits as GLint;
            let want = match (channels, bits) {
                (1, _) => [b, 0, 0, 0],
                (2, _) => [b, b, 0, 0],
                (_, 10) => [10, 10, 10, 2],
                _ => [b; 4],
            };
            assert_eq!(sizes, want, "{name}: channel sizes");

            let fb = framebuffer(&[rb]);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: ES 3.0 4.4.4, colour-renderable");
            let mut ct = 0;
            // SAFETY: room for one value.
            unsafe {
                glGetFramebufferAttachmentParameteriv(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE,
                    &mut ct,
                )
            };
            assert_eq!(ct as u32, if signed { GL_INT } else { GL_UNSIGNED_INT }, "{name}: component type");
            // SAFETY: plain GL call.
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
            release(fb, &[rb]);
        }
    }

    #[test]
    fn storage_errors_follow_the_spec() {
        let _g = context();
        let rb = renderbuffer(GL_RGBA8I, 4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8I, -1, 4);
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8I, 4, 4097);
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            /* an integer format has no multisampled storage (4.4.2) */
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA8I, 4, 4);
            assert_eq!(glGetError(), GL_INVALID_OPERATION);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_RGBA8I, 4, 4);
            assert_eq!(glGetError(), GL_NO_ERROR);
            /* the unsized and the sized-but-not-renderable names stay refused */
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA_INTEGER, 4, 4);
            assert_eq!(glGetError(), GL_INVALID_ENUM);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB32UI, 4, 4);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "RGB32UI is texture-only");
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB8UI, 4, 4);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "RGB8UI is texture-only");
            glDeleteRenderbuffers(1, &rb);
        }
    }

    /// A texture of `f`, 4x4, one level; None when the back end refuses the class.
    fn int_texture(f: GLenum) -> Option<GLuint> {
        let mut tex = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexStorage2D(GL_TEXTURE_2D, 1, f, 4, 4);
        }
        if err() == GL_NO_ERROR {
            return Some(tex);
        }
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &tex) };
        None
    }

    /// dEQP fbo.completeness.renderable.texture.color0.*: an integer texture
    /// level is a complete attachment where the back end writes its surface
    /// (RC1; these were GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT on the PowerVR),
    /// at colour attachment 0 and beyond it.
    ///
    /// The R and RG classes are held widened to four channels (`es3_formats`),
    /// a store that is not the layout of their surface, so they stay
    /// incomplete until the back end writes the narrow surface into it.
    #[test]
    fn completeness_takes_four_channel_integer_texture_attachments() {
        let _g = context();
        let mut made = 0;
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        for (f, name, channels, ..) in INT_FORMATS {
            let Some(tex) = int_texture(f) else { continue };
            made += 1;
            let want = if channels == 4 { GL_FRAMEBUFFER_COMPLETE } else { GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT };
            let mut fb = 0;
            // SAFETY: room for one name; tex is a level the framebuffer takes.
            unsafe {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            }
            assert_eq!(status(), want, "{name}: an integer texture at attachment 0");
            /* beside a normalised renderbuffer */
            let rb = renderbuffer(GL_RGBA8, 4, 4);
            // SAFETY: plain GL calls; the texture moves to attachment 1.
            unsafe {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 1, GL_TEXTURE_2D, tex, 0);
            }
            assert_eq!(status(), want, "{name}: an integer texture at attachment 1");
            release(fb, &[rb]);
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &tex) };
        }
        backend::HOST_INTEGER_RT_FORMATS.store(0, Ordering::Relaxed);
        assert!(made > 0, "some integer texture is storage on every back end");
    }

    /// A clear of a four-channel integer texture on attachment 1 lands in
    /// that attachment, in the layout of its surface.
    #[test]
    fn a_clear_reaches_an_integer_texture_on_attachment_1() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        for (f, name, channels, bits, signed) in INT_FORMATS.iter().copied().filter(|f| f.2 == 4 && !f.4 && f.3 != 10) {
            let Some(tex) = int_texture(f) else { continue };
            let rb = renderbuffer(GL_RGBA8, 4, 4);
            let fb = framebuffer(&[rb]);
            // SAFETY: tex is a level the framebuffer takes.
            unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 1, GL_TEXTURE_2D, tex, 0) };
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
            let max = if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 };
            let v: [GLuint; 4] = [max, 1, max / 2, 3];
            // SAFETY: four values; plain GL calls.
            unsafe {
                glClearBufferuiv(GL_COLOR, 1, v.as_ptr());
                glReadBuffer(GL_COLOR_ATTACHMENT0 + 1);
            }
            let px = read_u32(4, 4, GL_RGBA_INTEGER, GL_UNSIGNED_INT);
            assert_eq!(err(), GL_NO_ERROR, "{name}");
            for (n, p) in px.chunks(4).enumerate() {
                assert_eq!(p, v, "{name}: pixel {n}");
            }
            assert!(channels == 4 && !signed);
            release(fb, &[rb]);
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &tex) };
        }
        backend::HOST_INTEGER_RT_FORMATS.store(0, Ordering::Relaxed);
    }

    /// The integer formats that are texture-only (table 3.13) stay incomplete.
    #[test]
    fn completeness_refuses_texture_only_integer_formats() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        for (f, name) in [(GL_RGB8I, "RGB8I"), (GL_RGB16UI, "RGB16UI"), (GL_RGB32I, "RGB32I")] {
            let Some(tex) = int_texture(f) else { continue };
            let mut fb = 0;
            // SAFETY: room for one name; tex is a level the framebuffer takes.
            unsafe {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            }
            assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name} is not colour-renderable");
            release(fb, &[]);
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &tex) };
        }
        backend::HOST_INTEGER_RT_FORMATS.store(0, Ordering::Relaxed);
    }

    #[test]
    fn completeness_mixes_integer_and_normalised_attachments() {
        let _g = context();
        let (i, f, d) =
            (renderbuffer(GL_RGBA8I, 4, 4), renderbuffer(GL_RGBA8, 4, 4), renderbuffer(GL_DEPTH_COMPONENT16, 4, 4));
        let fb = framebuffer(&[i, f]);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "an integer and a normalised attachment (ES 3.0 4.4.4)");
        // SAFETY: plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, d) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "with a depth buffer");
        /* a depth format is not colour-renderable, an integer one is not depth */
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, i);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "an integer image as depth");
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_NONE, 0);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, d);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "a depth image as colour");
        release(fb, &[i, f, d]);
    }

    #[test]
    fn an_unsized_integer_renderbuffer_has_no_attachment_to_be_complete_with() {
        let _g = context();
        let rb = renderbuffer(GL_R32UI, 0, 0);
        let fb = framebuffer(&[rb]);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "zero size");
        release(fb, &[rb]);
    }

    /* ---- the back end refuses what it cannot render, by name ------------ */

    #[test]
    fn a_back_end_that_cannot_render_the_format_refuses_the_clear() {
        let _g = context();
        let rb = renderbuffer(GL_RGBA8I, 4, 4);
        let fb = framebuffer(&[rb]);
        let v: [GLint; 4] = [1, 2, 3, 4];
        // SAFETY: four values; plain GL calls.
        unsafe {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "the clear is recorded");
            glFinish();
        }
        /* the PowerVR renders every integer surface (INTEGER_RT_FORMATS):
         * there the clear reaches the back end; the refusal is the other
         * GPUs' rows of this file */
        if backend::rt_format_drawable(backend::GLES_SURF_RGBA8I) {
            assert_eq!(err(), GL_NO_ERROR, "a back end that renders the format takes the clear");
            assert!(DRAWS.lock().unwrap().iter().any(|d| d.2), "the clear was handed to the back end");
        } else {
            assert_eq!(err(), GL_INVALID_OPERATION, "the draw is refused when it would reach the back end");
            assert!(DRAWS.lock().unwrap().iter().all(|d| !d.2), "nothing was handed to the back end to store");
        }
        release(fb, &[rb]);
    }

    /// A clear one test leaves pending is not counted against the next: the
    /// refusal test failed whenever the clear checks (which end on a pending
    /// clear of the default framebuffer) took the lock just before it.
    #[test]
    fn a_test_starts_with_nothing_pending() {
        let g = context();
        let fv: [GLfloat; 4] = [0.0; 4];
        // SAFETY: four values; plain GL call.
        unsafe { glClearBufferfv(GL_COLOR, 0, fv.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
        drop(g);
        let _g = context();
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        let draws = DRAWS.lock().unwrap().clone();
        assert_eq!(draws, [], "the previous holder's clear reached this one's back end");
    }

    /* ---- glClearBuffer and glReadPixels ---------------------------------- */

    #[test]
    fn clear_buffer_checks_the_type_of_the_draw_buffer() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        let (i, u, f) = (renderbuffer(GL_RGBA8I, 4, 4), renderbuffer(GL_RGBA8UI, 4, 4), renderbuffer(GL_RGBA8, 4, 4));
        let fb = framebuffer(&[i, u, f]);
        let iv: [GLint; 4] = [0; 4];
        let uv: [GLuint; 4] = [0; 4];
        let fv: [GLfloat; 4] = [0.0; 4];
        // SAFETY: four values each; plain GL calls.
        unsafe {
            /* matching types are no error */
            glClearBufferiv(GL_COLOR, 0, iv.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "iv on signed");
            glClearBufferuiv(GL_COLOR, 1, uv.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "uiv on unsigned");
            glClearBufferfv(GL_COLOR, 2, fv.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "fv on normalised");
            /* every mismatch is GL_INVALID_OPERATION (ES 3.0 4.2.3) */
            let mismatch = |name: &str| assert_eq!(glGetError(), GL_INVALID_OPERATION, "{name}");
            glClearBufferfv(GL_COLOR, 0, fv.as_ptr());
            mismatch("fv on signed");
            glClearBufferuiv(GL_COLOR, 0, uv.as_ptr());
            mismatch("uiv on signed");
            glClearBufferiv(GL_COLOR, 1, iv.as_ptr());
            mismatch("iv on unsigned");
            glClearBufferfv(GL_COLOR, 1, fv.as_ptr());
            mismatch("fv on unsigned");
            glClearBufferiv(GL_COLOR, 2, iv.as_ptr());
            mismatch("iv on normalised");
            glClearBufferuiv(GL_COLOR, 2, uv.as_ptr());
            mismatch("uiv on normalised");
            /* an argument error is still the argument's */
            glClearBufferiv(GL_COLOR, 99, iv.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            glClearBufferiv(GL_DEPTH, 0, iv.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_ENUM);
            /* a draw buffer with nothing behind it is no error and clears nothing */
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT2, GL_RENDERBUFFER, 0);
            glClearBufferiv(GL_COLOR, 2, iv.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR);
            /* the default framebuffer is fixed point */
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glClearBufferiv(GL_COLOR, 0, iv.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_OPERATION);
            glClearBufferuiv(GL_COLOR, 0, uv.as_ptr());
            assert_eq!(glGetError(), GL_INVALID_OPERATION);
            glClearBufferfv(GL_COLOR, 0, fv.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        release(fb, &[i, u, f]);
    }

    #[test]
    fn a_clear_does_not_disturb_the_clear_colour() {
        let _g = context();
        let rb = renderbuffer(GL_R32I, 2, 2);
        let fb = framebuffer(&[rb]);
        let v: [GLint; 4] = [-7, 0, 0, 0];
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
            let mut c = [0.0f32; 4];
            glGetFloatv(GL_COLOR_CLEAR_VALUE, c.as_mut_ptr());
            assert_eq!(c, [0.25, 0.5, 0.75, 1.0]);
        }
        release(fb, &[rb]);
    }

    #[test]
    fn signed_clears_read_back_as_rgba_integer() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        for (f, name, channels, bits, signed) in INT_FORMATS.iter().copied().filter(|f| f.4) {
            let rb = renderbuffer(f, 3, 2);
            let fb = framebuffer(&[rb]);
            let lim = (1i64 << (bits - 1)) - 1;
            let v: [GLint; 4] = [(-lim - 1) as GLint, lim as GLint, -1, 5];
            // SAFETY: four values; plain GL call.
            unsafe { glClearBufferiv(GL_COLOR, 0, v.as_ptr()) };
            let px = read_u32(3, 2, GL_RGBA_INTEGER, GL_INT);
            assert_eq!(err(), GL_NO_ERROR, "{name}");
            let mut want = [0u32, 0, 0, 1];
            for ch in 0..channels {
                want[ch] = v[ch] as u32;
            }
            for (n, p) in px.chunks(4).enumerate() {
                assert_eq!(p, want, "{name}: pixel {n} (sign-extended; the missing channels read 0,0,0,1)");
            }
            assert!(signed);
            release(fb, &[rb]);
        }
    }

    #[test]
    fn unsigned_clears_read_back_as_rgba_integer() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        for (f, name, channels, bits, _) in INT_FORMATS.iter().copied().filter(|f| !f.4) {
            let rb = renderbuffer(f, 3, 3);
            let fb = framebuffer(&[rb]);
            let max = if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 };
            let v: [GLuint; 4] = [max, 1, max / 2, 3];
            // SAFETY: four values; plain GL call.
            unsafe { glClearBufferuiv(GL_COLOR, 0, v.as_ptr()) };
            let px = read_u32(3, 3, GL_RGBA_INTEGER, GL_UNSIGNED_INT);
            assert_eq!(err(), GL_NO_ERROR, "{name}");
            let mut want = [0u32, 0, 0, 1];
            want[..channels].copy_from_slice(&v[..channels]);
            if bits == 10 {
                want[3] = v[3] & 3; /* the 2-bit alpha */
            }
            for (n, p) in px.chunks(4).enumerate() {
                assert_eq!(p, want, "{name}: pixel {n}");
            }
            release(fb, &[rb]);
        }
    }

    #[test]
    fn a_clear_of_one_draw_buffer_leaves_the_other() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        let (a, b) = (renderbuffer(GL_RGBA8UI, 2, 2), renderbuffer(GL_RGBA16I, 2, 2));
        let fb = framebuffer(&[a, b]);
        let (ua, ib): ([GLuint; 4], [GLint; 4]) = ([9, 8, 7, 6], [-9, -8, -7, -6]);
        // SAFETY: four values each; plain GL calls.
        unsafe {
            glClearBufferuiv(GL_COLOR, 0, ua.as_ptr());
            glClearBufferiv(GL_COLOR, 1, ib.as_ptr());
            /* attachment 1 is the read buffer now */
            glReadBuffer(GL_COLOR_ATTACHMENT1);
        }
        assert_eq!(read_u32(2, 2, GL_RGBA_INTEGER, GL_INT)[..4], ib.map(|v| v as u32));
        // SAFETY: plain GL call.
        unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0) };
        assert_eq!(read_u32(2, 2, GL_RGBA_INTEGER, GL_UNSIGNED_INT)[..4], ua);
        assert_eq!(err(), GL_NO_ERROR);
        release(fb, &[a, b]);
    }

    #[test]
    fn read_pixels_takes_only_the_integer_pair_of_an_integer_buffer() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        let (s, u, n) = (renderbuffer(GL_RG16I, 2, 2), renderbuffer(GL_RG16UI, 2, 2), renderbuffer(GL_RGBA8, 2, 2));
        let mut buf = [0u32; 16];
        let p = buf.as_mut_ptr() as *mut c_void;
        // SAFETY: room for a 2 x 2 RGBA read of 32-bit values; plain GL calls.
        unsafe {
            let fb = framebuffer(&[s]);
            glReadPixels(0, 0, 2, 2, GL_RGBA_INTEGER, GL_INT, p);
            assert_eq!(glGetError(), GL_NO_ERROR);
            for (format, type_, what) in [
                (GL_RGBA_INTEGER, GL_UNSIGNED_INT, "unsigned type on a signed buffer"),
                (GL_RGBA, GL_UNSIGNED_BYTE, "RGBA/UNSIGNED_BYTE on an integer buffer"),
                (GL_RGBA, GL_FLOAT, "RGBA/FLOAT on an integer buffer"),
                (GL_RED_INTEGER, GL_INT, "RED_INTEGER is not a read format"),
            ] {
                glReadPixels(0, 0, 2, 2, format, type_, p);
                assert_eq!(glGetError(), GL_INVALID_OPERATION, "{what}");
            }
            glReadPixels(0, 0, 2, 2, GL_RGBA_INTEGER, 0x1234, p);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "an unknown type");
            release(fb, &[]);

            let fb = framebuffer(&[u]);
            glReadPixels(0, 0, 2, 2, GL_RGBA_INTEGER, GL_UNSIGNED_INT, p);
            assert_eq!(glGetError(), GL_NO_ERROR);
            glReadPixels(0, 0, 2, 2, GL_RGBA_INTEGER, GL_INT, p);
            assert_eq!(glGetError(), GL_INVALID_OPERATION, "signed type on an unsigned buffer");
            release(fb, &[]);

            let fb = framebuffer(&[n]);
            glReadPixels(0, 0, 2, 2, GL_RGBA_INTEGER, GL_INT, p);
            assert_eq!(glGetError(), GL_INVALID_OPERATION, "RGBA_INTEGER on a normalised buffer");
            glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, p);
            assert_eq!(glGetError(), GL_NO_ERROR);
            release(fb, &[s, u, n]);
        }
    }

    #[test]
    fn read_pixels_honours_the_pack_state_at_sixteen_bytes_a_pixel() {
        let _g = context();
        backend::HOST_INTEGER_RT_FORMATS.store(ALL_SURFACES, Ordering::Relaxed);
        let rb = renderbuffer(GL_R8UI, 3, 2);
        let fb = framebuffer(&[rb]);
        let v: [GLuint; 4] = [200, 0, 0, 0];
        // SAFETY: plain GL calls; room for 2 rows of 5 pixels (row length) + skip of 1 pixel.
        let px = unsafe {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr());
            glPixelStorei(GL_PACK_ROW_LENGTH, 5);
            glPixelStorei(GL_PACK_SKIP_PIXELS, 1);
            let mut px = vec![0xaaaa_aaaau32; 5 * 2 * 4 + 4];
            glReadPixels(0, 0, 3, 2, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
            glPixelStorei(GL_PACK_ROW_LENGTH, 0);
            glPixelStorei(GL_PACK_SKIP_PIXELS, 0);
            px
        };
        assert_eq!(err(), GL_NO_ERROR);
        let pixel = |row: usize, col: usize| &px[(row * 5 + col + 1) * 4..][..4];
        for row in 0..2 {
            for col in 0..3 {
                assert_eq!(pixel(row, col), [200, 0, 0, 1], "row {row} col {col}");
            }
            if row == 0 {
                assert_eq!(&px[(3 + 1) * 4..(5 + 1) * 4], [0xaaaa_aaaa; 8], "the row padding is the caller's");
            }
        }
        release(fb, &[rb]);
    }

    #[test]
    fn implementation_color_read_format_and_type_follow_the_read_buffer() {
        let _g = context();
        let q = || (geti(GL_IMPLEMENTATION_COLOR_READ_FORMAT) as u32, geti(GL_IMPLEMENTATION_COLOR_READ_TYPE) as u32);
        assert_eq!(q(), (GL_RGBA, GL_UNSIGNED_BYTE), "the default framebuffer");
        let (s, u, n) = (renderbuffer(GL_RGBA8I, 2, 2), renderbuffer(GL_RG32UI, 2, 2), renderbuffer(GL_RGBA8, 2, 2));
        let fb = framebuffer(&[s, u, n]);
        // SAFETY: plain GL calls.
        unsafe {
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            assert_eq!(q(), (GL_RGBA_INTEGER, GL_INT));
            glReadBuffer(GL_COLOR_ATTACHMENT1);
            assert_eq!(q(), (GL_RGBA_INTEGER, GL_UNSIGNED_INT));
            glReadBuffer(GL_COLOR_ATTACHMENT2);
            assert_eq!(q(), (GL_RGBA, GL_UNSIGNED_BYTE));
        }
        assert_eq!(err(), GL_NO_ERROR);
        release(fb, &[s, u, n]);
    }

    #[test]
    fn the_surface_description_a_back_end_maps_is_consistent() {
        use backend::*;
        let mut seen = 0u32;
        for s in GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST {
            assert!(gles_surf_is_integer(s));
            let l = gles_surf_int_layout(s).expect("a layout for every integer surface");
            let want = if l.bits == 10 { 4 } else { l.channels as u32 * l.bits as u32 / 8 };
            assert_eq!(gles_surf_bytes_per_pixel(s), want, "{}", gles_surf_name(s));
            assert_ne!(gles_surf_name(s), "unknown");
            assert_eq!(gles_surf_rt_class(s), 0, "no float-class shader variant");
            seen += 1;
        }
        assert_eq!(seen as usize, INT_FORMATS.len());
        for s in 0..GLES_SURF_INT_FIRST {
            assert!(!gles_surf_is_integer(s) && gles_surf_int_layout(s).is_none());
            assert!(rt_format_drawable(s), "{} always was", gles_surf_name(s));
        }
        let _g = context();
        assert!(!rt_format_drawable(GLES_SURF_R8I) || INTEGER_RT_FORMATS & 1 != 0);
    }
}
