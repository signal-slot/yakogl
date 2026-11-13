// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): which float colour formats are complete attachments, as
//! dEQP fbo.color, fbo.render and fbo.completeness.renderable judge them
//! (ES 3.0 4.4.4.1, table 3.13, and the extensions this build lists).
//!
//! The build claims GL_EXT_color_buffer_half_float and not
//! GL_EXT_color_buffer_float, so:
//!
//! - RGBA16F, RGB16F, RG16F and R16F are complete, as textures and as
//!   renderbuffers (RGB16F as a texture was INCOMPLETE_ATTACHMENT because it
//!   is an es3_formats name; both are held as the PBE's four-half surface);
//! - R32F, RG32F and RGBA32F textures are INCOMPLETE_ATTACHMENT, and their
//!   renderbuffer storage is GL_INVALID_ENUM.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_fbo_float
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", not(feature = "report-es2")))]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
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

/// What the back end was handed, one entry a draw (or clear).
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    color_mask: u8,
    /// The fragment program's (`ShcShader::blend_key`), 0 without one.
    blend_key: u32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each draw; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout, little endian, rows at the
/// pair-padded pitch (every channel the surface has, as the PBE would).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets.max(1) as usize;
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
        blend_key: d.fs.as_ref().map_or(0, |fs| fs.blend_key),
    });
    let t = &d.targets[0];
    let Some(l) = backend::gles_surf_int_layout(t.format) else { return 0 };
    if !d.do_clear || t.gpu == 0 {
        return 0;
    }
    let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
    let pitch = ((t.w + 1) & !1) as usize;
    let v = d.clear_rgba.map(f32::to_bits);
    let step = l.bits as usize / 8;
    let mut px = vec![0u8; bpp];
    for ch in 0..l.channels as usize {
        px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
    }
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
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
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    /* what an earlier test left recorded is settled before this one counts */
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

fn release(fb: GLuint, tex: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

const HALF: [(GLenum, &str); 4] =
    [(GL_RGBA16F, "RGBA16F"), (GL_RGB16F, "RGB16F"), (GL_RG16F, "RG16F"), (GL_R16F, "R16F")];
const SINGLE: [(GLenum, &str); 3] = [(GL_RGBA32F, "RGBA32F"), (GL_RG32F, "RG32F"), (GL_R32F, "R32F")];

fn renderbuffer(f: GLenum) -> (GLuint, GLenum) {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, f, 8, 8);
    }
    (rb, err())
}

fn framebuffer_rb(rb: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    fb
}

/// dEQP fbo.completeness.renderable.texture.color0 of rgba16f, rgb16f, rg16f
/// and r16f under GL_EXT_color_buffer_half_float: complete at levels 0 and 1.
#[test]
fn half_float_textures_are_complete_attachments() {
    let _g = context();
    for (f, name) in HALF {
        for (levels, level) in [(1, 0), (3, 1)] {
            let tex = texture_2d(f, 8, levels);
            let fb = framebuffer_2d(tex, level);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: level {level}");
            release(fb, tex);
        }
    }
}

#[test]
fn half_float_renderbuffers_are_complete_attachments() {
    let _g = context();
    for (f, name) in HALF {
        let (rb, e) = renderbuffer(f);
        assert_eq!(e, GL_NO_ERROR, "{name}: glRenderbufferStorage");
        let fb = framebuffer_rb(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
    }
}

/// dEQP fbo.completeness.renderable.texture.color0 of rgba32f, rg32f and
/// r32f: not colour-renderable without GL_EXT_color_buffer_float, which this
/// build does not list.
#[test]
fn single_float_textures_are_incomplete_without_color_buffer_float() {
    let _g = context();
    // SAFETY: a NUL-terminated string the GL core owns.
    let ext = unsafe { std::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_string_lossy();
    assert!(!ext.contains("GL_EXT_color_buffer_float") && ext.contains("GL_EXT_color_buffer_half_float"));
    for (f, name) in SINGLE {
        let tex = texture_2d(f, 8, 1);
        let fb = framebuffer_2d(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name}");
        release(fb, tex);
    }
}

/// The sized single-precision names are no renderbuffer formats without the
/// extension (ES 3.0 4.4.2).
#[test]
fn single_float_renderbuffers_are_refused() {
    let _g = context();
    for (f, name) in SINGLE {
        let (rb, e) = renderbuffer(f);
        assert_eq!(e, GL_INVALID_ENUM, "{name}");
        // SAFETY: a name made above.
        unsafe { glDeleteRenderbuffers(1, &rb) };
    }
}

/// An RGB16F attachment names three half-float channels and no alpha.
#[test]
fn rgb16f_texture_reports_three_half_channels() {
    let _g = context();
    let tex = texture_2d(GL_RGB16F, 8, 1);
    let fb = framebuffer_2d(tex, 0);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let q = |pname| {
        let mut v = -1;
        // SAFETY: one GLint.
        unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
        v
    };
    assert_eq!(
        [
            q(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE),
            q(GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE),
            q(GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE),
            q(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE)
        ],
        [16, 16, 16, 0]
    );
    assert_eq!(q(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), GL_FLOAT as GLint);
    release(fb, tex);
}
