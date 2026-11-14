// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): which float colour formats are complete framebuffer
//! attachments (dEQP fbo.completeness.renderable.*, fbo.color.*rgb16f*,
//! fbo.render.*rgb16f*).
//!
//! The build claims GL_EXT_color_buffer_half_float and not
//! GL_EXT_color_buffer_float. So (ES 3.0.6 4.4.2, 4.4.4.1, table 3.13):
//!
//! - RGBA16F, RGB16F, RG16F and R16F are complete, as a texture (level 0,
//!   level 1 of a mipmapped one, layer 1 of a 2D array) and as a
//!   renderbuffer; an RGB16F or RG16F attachment is drawn as the four-half
//!   RGBA16F surface its store is;
//! - R32F, RG32F, RGBA32F and R11F_G11F_B10F are not colour-renderable: a
//!   texture of them is GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT and
//!   glRenderbufferStorage of them is GL_INVALID_ENUM;
//! - the extension list says the same: GL_EXT_color_buffer_float absent,
//!   GL_EXT_color_buffer_half_float present.
//!
//! What it cannot see: the GPU. The stand-in back end draws nothing; whether
//! the PBE writes the pictures is a board result.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_fbo_float
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
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

/// The surface of each draw (clear) the back end was handed.
static SEEN: Mutex<Vec<u8>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    SEEN.lock().unwrap().push(d.targets[0].format);
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
    name: c"es3-pvr-fbo-float".as_ptr() as *const c_char,
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
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// The half-float formats: (format, name, the surface a draw into it uses).
const HALF: [(GLenum, &str, u8); 4] = [
    (GL_RGBA16F, "RGBA16F", backend::GLES_SURF_RGBA16F),
    (GL_RGB16F, "RGB16F", backend::GLES_SURF_RGBA16F),
    (GL_RG16F, "RG16F", backend::GLES_SURF_RGBA16F),
    (GL_R16F, "R16F", backend::GLES_SURF_R16F),
];

/// The 32-bit float formats and R11F_G11F_B10F, which need
/// GL_EXT_color_buffer_float.
const NEEDS_FLOAT_EXT: [(GLenum, &str); 4] =
    [(GL_R32F, "R32F"), (GL_RG32F, "RG32F"), (GL_RGBA32F, "RGBA32F"), (GL_R11F_G11F_B10F, "R11F_G11F_B10F")];

/// A `w` x `w` 2D texture of `f` with `levels` levels.
fn texture_2d(f: GLenum, w: GLsizei, levels: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, levels, f, w, w);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D {f:#x}");
    tex
}

/// A framebuffer with `tex` level `level` at GL_COLOR_ATTACHMENT0, bound.
fn framebuffer_2d(tex: GLuint, level: GLint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; tex is a level the framebuffer takes.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
    }
    fb
}

/// A framebuffer with a `w` x `w` renderbuffer of `f` at attachment 0, bound:
/// (framebuffer, renderbuffer, the glRenderbufferStorage error).
fn framebuffer_rb(f: GLenum, w: GLsizei) -> (GLuint, GLuint, GLenum) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    let e = unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, f, w, w);
        let e = glGetError();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        e
    };
    (fb, rb, e)
}

fn release(fb: GLuint, tex: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
    }
}

fn release_rb(fb: GLuint, rb: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

/// dEQP fbo.completeness.renderable.texture.color0.{rgba16f,rgb16f,rg16f,
/// r16f}: complete at level 0, at level 1 of a mipmapped texture and at
/// layer 1 of a 2D array.
#[test]
fn half_float_textures_are_complete_attachments() {
    let _g = context();
    for (f, name, _) in HALF {
        let tex = texture_2d(f, 8, 1);
        let fb = framebuffer_2d(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: level 0 of a 2D texture");
        release(fb, tex);

        let tex = texture_2d(f, 8, 3);
        let fb = framebuffer_2d(tex, 1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: level 1 of a mipmapped 2D texture");
        release(fb, tex);

        let (mut tex, mut fb) = (0, 0);
        // SAFETY: room for one name each; layer 1 of 3 is there.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, f, 8, 8, 3);
            assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glTexStorage3D");
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: layer 1 of a 2D array");
        release(fb, tex);
    }
}

/// dEQP fbo.completeness.renderable.renderbuffer.color0.*16f.
#[test]
fn half_float_renderbuffers_are_complete_attachments() {
    let _g = context();
    for (f, name, _) in HALF {
        let (fb, rb, e) = framebuffer_rb(f, 8);
        assert_eq!(e, GL_NO_ERROR, "{name}: glRenderbufferStorage");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
        release_rb(fb, rb);
    }
}

/// dEQP fbo.completeness.renderable.{texture,renderbuffer}.color0.{r32f,
/// rg32f,rgba32f,r11f_g11f_b10f}: without GL_EXT_color_buffer_float a
/// texture of them is an incomplete attachment and renderbuffer storage of
/// them is GL_INVALID_ENUM.
#[test]
fn formats_needing_color_buffer_float_are_refused() {
    let _g = context();
    for (f, name) in NEEDS_FLOAT_EXT {
        let tex = texture_2d(f, 8, 1);
        let fb = framebuffer_2d(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name}: texture");
        release(fb, tex);

        let (fb, rb, e) = framebuffer_rb(f, 8);
        assert_eq!(e, GL_INVALID_ENUM, "{name}: glRenderbufferStorage");
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name}: renderbuffer");
        release_rb(fb, rb);
    }
}

/// A clear of a half-float texture reaches the back end as the surface of the
/// format's store: an RGB16F and an RG16F texture as the four-half RGBA16F
/// one, R16F as its own (dEQP fbo.color.clear.rgb16f, fbo.render.*.rgb16f).
#[test]
fn half_float_textures_are_cleared_as_their_store() {
    let _g = context();
    for (f, name, surf) in HALF {
        let tex = texture_2d(f, 8, 1);
        let fb = framebuffer_2d(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
        SEEN.lock().unwrap().clear();
        let v = [0.25f32, 0.5, 0.75, 1.0];
        // SAFETY: four floats for a colour clear of attachment 0.
        unsafe { glClearBufferfv(GL_COLOR, 0, v.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "{name}: glClearBufferfv");
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        let seen = SEEN.lock().unwrap().clone();
        assert!(!seen.is_empty() && seen.iter().all(|s| *s == surf), "{name}: {seen:?}");
        release(fb, tex);
    }
}

/// The extension list: half-float renderability is claimed, 32-bit is not.
#[test]
fn the_extension_list_matches() {
    let _g = context();
    let mut n = 0;
    // SAFETY: one GLint.
    unsafe { glGetIntegerv(GL_NUM_EXTENSIONS, &mut n) };
    let listed: Vec<String> = (0..n as u32)
        // SAFETY: indices below GL_NUM_EXTENSIONS name NUL-terminated strings.
        .map(|i| {
            unsafe { CStr::from_ptr(glGetStringi(GL_EXTENSIONS, i) as *const c_char) }.to_string_lossy().into_owned()
        })
        .collect();
    assert!(listed.iter().any(|e| e == "GL_EXT_color_buffer_half_float"), "{listed:?}");
    assert!(!listed.iter().any(|e| e == "GL_EXT_color_buffer_float"), "{listed:?}");
}
