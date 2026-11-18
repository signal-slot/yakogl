// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer from and into the fourth colour attachment (plan #368,
//! RC14) on the V3D host back end, under `HOST_DRAW_TARGETS` = 4.
//!
//! The blit is the GL core's (`src/gl/blit.rs`, a CPU copy over the
//! attachments' storage), so the pixels are checked through glReadPixels;
//! the back end here is the smallest stand-in that makes a context (nothing
//! is drawn). The attachments are textures filled by glTexImage2D, each
//! attachment k with its own values, so a blit that reads or writes the
//! wrong attachment shows.
//!
//! Covered, for RGBA8 and for RGBA8UI (NEAREST):
//! * `glReadBuffer(GL_COLOR_ATTACHMENT3)` of a four-attachment framebuffer
//!   blitted into a single-attachment one;
//! * a blit into a draw framebuffer whose glDrawBuffers enables only
//!   attachment 3 (the others keep their contents), and all four (each
//!   takes the source);
//! * a component-type mismatch with the read buffer 3 (GL_INVALID_OPERATION).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_blit_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
unsafe extern "C" fn b_ok_draw(_d: *const backend::GlesDraw) -> i32 {
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
    name: c"es3-mrt4-blit".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_ok_draw),
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const W: GLsizei = 4;

fn context() -> MutexGuard<'static, ()> {
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
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fmt {
    Rgba8,
    Rgba8ui,
}

/// A framebuffer of attachments: textures for RGBA8, renderbuffers for
/// RGBA8UI (the GL core's blit copies an integer renderbuffer; an RGBA8UI
/// texture has no blit image today, see the last test).
struct Fbo {
    fb: GLuint,
    texs: Vec<GLuint>,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.texs.len() as GLsizei, self.texs.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
        }
    }
}

/// The bytes attachment `k` holds at pixel (x, y) in the four-attachment
/// framebuffers.
fn val(k: usize, x: usize, y: usize) -> [u8; 4] {
    [(k * 40 + x * 8 + 1) as u8, (y * 16 + k) as u8, (200 - k * 20) as u8, (x + y * 4 + k * 3 + 10) as u8]
}

/// `n` attachments, attachment k filled with `fill(k, x, y)`.
fn fbo(fmt: Fmt, n: usize, fill: impl Fn(usize, usize, usize) -> [u8; 4]) -> Fbo {
    let (mut fb, mut texs, mut rbs) = (0, vec![0; n], vec![0; n]);
    let w = W as usize;
    // SAFETY: room for the names; the pixel array holds W x W RGBA bytes.
    unsafe {
        glGenTextures(n as GLsizei, texs.as_mut_ptr());
        glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for k in 0..n {
            let px: Vec<u8> = (0..w * w).flat_map(|i| fill(k, i % w, i / w)).collect();
            match fmt {
                Fmt::Rgba8 => {
                    glBindTexture(GL_TEXTURE_2D, texs[k]);
                    let p = px.as_ptr() as *const c_void;
                    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, W, W, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);
                    glFramebufferTexture2D(GL_FRAMEBUFFER, att(k), GL_TEXTURE_2D, texs[k], 0);
                }
                Fmt::Rgba8ui => {
                    glBindRenderbuffer(GL_RENDERBUFFER, rbs[k]);
                    glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8UI, W, W);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, att(k), GL_RENDERBUFFER, rbs[k]);
                    // the host's "GPU memory" is CPU memory; rows at the pair-padded pitch (W here)
                    let c = yakogl::gl::context::current().expect("a context");
                    let mem = c.objs_ref().renderbuffers[rbs[k] as usize].mem.cpu as *mut u8;
                    core::ptr::copy_nonoverlapping(px.as_ptr(), mem, px.len());
                }
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{fmt:?} fbo");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{fmt:?} x{n}");
    }
    Fbo { fb, texs, rbs }
}

fn att(k: usize) -> GLenum {
    GL_COLOR_ATTACHMENT0 + k as GLenum
}

/// Attachment `k` of `f` as RGBA bytes.
fn read(f: &Fbo, fmt: Fmt, k: usize) -> Vec<[u8; 4]> {
    let mut px = vec![0x77u8; (W * W * 4) as usize];
    // SAFETY: room for W x W RGBA pixels (bytes, or words for the integer format).
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        glReadBuffer(att(k));
        match fmt {
            Fmt::Rgba8 => glReadPixels(0, 0, W, W, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void),
            Fmt::Rgba8ui => {
                let mut u = vec![0xdead_u32; (W * W * 4) as usize];
                glReadPixels(0, 0, W, W, GL_RGBA_INTEGER, GL_UNSIGNED_INT, u.as_mut_ptr() as *mut c_void);
                for (d, s) in px.iter_mut().zip(&u) {
                    *d = *s as u8;
                }
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "read {fmt:?} {k}");
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

fn expect(f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<[u8; 4]> {
    (0..(W * W) as usize).map(|i| f(i % W as usize, i / W as usize)).collect()
}

fn blit(src: &Fbo, read_k: usize, dst: &Fbo, draw: &[GLenum]) {
    // SAFETY: plain GL calls; `draw` outlives glDrawBuffers.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(att(read_k));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glDrawBuffers(draw.len() as GLsizei, draw.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "state");
        glBlitFramebuffer(0, 0, W, W, 0, 0, W, W, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
}

fn read_att3_into_single(fmt: Fmt) {
    let _g = context();
    let src = fbo(fmt, 4, val);
    let dst = fbo(fmt, 1, |_, _, _| [9, 9, 9, 9]);
    blit(&src, 3, &dst, &[att(0)]);
    assert_eq!(err(), GL_NO_ERROR, "blit");
    assert_eq!(read(&dst, fmt, 0), expect(|x, y| val(3, x, y)), "{fmt:?}: attachment 3 reaches the single target");
    for k in 0..4 {
        assert_eq!(read(&src, fmt, k), expect(|x, y| val(k, x, y)), "{fmt:?}: source {k} is untouched");
    }
}

fn only_att3_is_written(fmt: Fmt) {
    let _g = context();
    let src = fbo(fmt, 1, |_, x, y| val(7, x, y));
    let dst = fbo(fmt, 4, val);
    blit(&src, 0, &dst, &[GL_NONE, GL_NONE, GL_NONE, att(3)]);
    assert_eq!(err(), GL_NO_ERROR, "blit");
    assert_eq!(read(&dst, fmt, 3), expect(|x, y| val(7, x, y)), "{fmt:?}: attachment 3 takes the source");
    for k in 0..3 {
        assert_eq!(read(&dst, fmt, k), expect(|x, y| val(k, x, y)), "{fmt:?}: attachment {k} keeps its pixels");
    }
}

fn all_four_are_written(fmt: Fmt) {
    let _g = context();
    let src = fbo(fmt, 4, val);
    let dst = fbo(fmt, 4, |_, _, _| [3, 3, 3, 3]);
    blit(&src, 3, &dst, &[att(0), att(1), att(2), att(3)]);
    assert_eq!(err(), GL_NO_ERROR, "blit");
    for k in 0..4 {
        assert_eq!(read(&dst, fmt, k), expect(|x, y| val(3, x, y)), "{fmt:?}: attachment {k} takes attachment 3");
    }
}

#[test]
fn rgba8_read_attachment3_into_single() {
    read_att3_into_single(Fmt::Rgba8);
}

#[test]
fn rgba8_draw_only_attachment3() {
    only_att3_is_written(Fmt::Rgba8);
}

#[test]
fn rgba8_draw_all_four() {
    all_four_are_written(Fmt::Rgba8);
}

#[test]
fn rgba8ui_read_attachment3_into_single() {
    read_att3_into_single(Fmt::Rgba8ui);
}

#[test]
fn rgba8ui_draw_only_attachment3() {
    only_att3_is_written(Fmt::Rgba8ui);
}

#[test]
fn rgba8ui_draw_all_four() {
    all_four_are_written(Fmt::Rgba8ui);
}

/// Pinned as today: an RGBA8UI *texture* attachment has no blit image in the
/// GL core (`attachment_image` handles RGBA8/16F/32F/sRGB textures only), so
/// a blit from or into one is a silent no-op with no error. A defect of the
/// GL core (src/gl/blit.rs), not of the V3D back end; a working blit would
/// copy attachment 3's 10s over the destination's 20s.
#[test]
fn rgba8ui_texture_attachment3_blit_is_a_silent_noop_today() {
    let _g = context();
    let (mut fb, mut tex) = ([0; 2], [0; 2]);
    // SAFETY: plain GL calls; the pixel array holds W x W RGBA bytes.
    unsafe {
        glGenFramebuffers(2, fb.as_mut_ptr());
        glGenTextures(2, tex.as_mut_ptr());
        for i in 0..2 {
            let px = vec![(i as u8 + 1) * 10; (W * W * 4) as usize];
            let p = px.as_ptr() as *const c_void;
            glBindTexture(GL_TEXTURE_2D, tex[i]);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8UI as GLint, W, W, 0, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, p);
            glBindFramebuffer(GL_FRAMEBUFFER, fb[i]);
            glFramebufferTexture2D(GL_FRAMEBUFFER, att(if i == 0 { 3 } else { 0 }), GL_TEXTURE_2D, tex[i], 0);
        }
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb[0]);
        glReadBuffer(att(3));
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb[1]);
        glBlitFramebuffer(0, 0, W, W, 0, 0, W, W, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR);
        let mut u = vec![0u32; (W * W * 4) as usize];
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb[1]);
        glReadBuffer(att(0));
        glReadPixels(0, 0, W, W, GL_RGBA_INTEGER, GL_UNSIGNED_INT, u.as_mut_ptr() as *mut c_void);
        assert_eq!(err(), GL_NO_ERROR);
        assert!(u.iter().all(|&v| v == 20), "destination keeps its pixels today: {u:?}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, fb.as_ptr());
        glDeleteTextures(2, tex.as_ptr());
    }
}

/// An integer read buffer 3 into a normalised draw buffer is refused, and
/// nothing is written.
#[test]
fn integer_attachment3_into_rgba8_is_refused() {
    let _g = context();
    let src = fbo(Fmt::Rgba8ui, 4, val);
    let dst = fbo(Fmt::Rgba8, 4, |_, _, _| [5, 5, 5, 5]);
    blit(&src, 3, &dst, &[GL_NONE, GL_NONE, GL_NONE, att(3)]);
    assert_eq!(err(), GL_INVALID_OPERATION);
    assert_eq!(read(&dst, Fmt::Rgba8, 3), vec![[5, 5, 5, 5]; (W * W) as usize]);
}
