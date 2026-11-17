// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A multisampled GL_RGB10_A2 renderbuffer and its resolve blit on the V3D
//! host back end, through the GL entry points only (plan #370,
//! rgb10a2-msaa-api).
//!
//! What the tip does, pinned: GL_MAX_SAMPLES is 1 while the back end keeps no
//! samples (`msaa::backend_max_samples` is 0), so glGetInternalformativ
//! lists the one count 1 for GL_RGB10_A2 and glRenderbufferStorageMultisample
//! takes 1 (one sample in fact, but `GL_SAMPLE_BUFFERS` 1 and a resolve-style
//! read framebuffer) and refuses 2 or more with GL_INVALID_OPERATION.
//! `glClear` then reaches a `GLES_SURF_RGB10_A2` target through the
//! stand-in back end (same rule as es3_rgb10a2_clear_v3d), and
//! glBlitFramebuffer's CPU colour path resolves it word for word into a
//! single-sample RGB10_A2 renderbuffer or texture. A resolve into RGBA8 (or
//! into a multisampled target) is GL_INVALID_OPERATION as ES 3.0 4.3.3 says
//! (formats must be identical); a single-sample RGB10_A2 source blits into
//! RGBA8 with each field decoded to a byte. glReadPixels of the multisampled
//! buffer itself is GL_INVALID_OPERATION. glReadPixels
//! GL_RGBA/GL_UNSIGNED_BYTE decodes a field as (v * 255 + max / 2) / max
//! (8428875e). The read pair is not asserted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_msaa_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
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
/// The RGB10_A2 clear draws the stand-in back end applied.
static CLEARS: AtomicU32 = AtomicU32::new(0);

/// The stand-in GPU: a clear draw into a `GLES_SURF_RGB10_A2` target writes
/// the clear's 10/10/10/2 word inside the draw's scissor box, for the
/// channels of `clear_color_mask` only. That is the V3D back end's job (the
/// TLB clear or the clear quad), which does not run on the host; the test
/// judges what the GL core hands it and what glReadPixels makes of the words
/// that result.
///
/// Quantisation rule: red, green and blue are `clamp(v, 0, 1) * 1023`
/// rounded half up, alpha `clamp(v, 0, 1) * 3` rounded half up (the GL core
/// has already rounded alpha to the target's 2 bits, `quantise_unorm`).
/// glReadPixels then decodes a field to a byte as (v * 255 + max / 2) / max.
unsafe extern "C" fn b_draw(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    if d.do_clear && d.target_format == backend::GLES_SURF_RGB10_A2 && d.target_gpu != 0 {
        CLEARS.fetch_add(1, Ordering::SeqCst);
        let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n + 0.5) as u32;
        let c = d.clear_rgba;
        let new = [q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0)];
        let pitch = backend::draw_pitch(d) as usize;
        let base = d.target_gpu as *mut u32;
        let [sx, sy, sw, sh] = if d.scissor { d.scissor_rect } else { [0, 0, d.target_w as i32, d.target_h as i32] };
        for y in sy.max(0)..(sy + sh).min(d.target_h as i32) {
            for x in sx.max(0)..(sx + sw).min(d.target_w as i32) {
                let p = base.add(y as usize * pitch + x as usize);
                let w = *p;
                let old = [w & 0x3ff, w >> 10 & 0x3ff, w >> 20 & 0x3ff, w >> 30];
                let f: Vec<u32> =
                    (0..4).map(|i| if d.clear_color_mask >> i & 1 != 0 { new[i] } else { old[i] }).collect();
                *p = f[0] | f[1] << 10 | f[2] << 20 | f[3] << 30;
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
    name: c"es3-rgb10a2-msaa".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const N: GLsizei = 4;

/// The 10/10/10/2 word of (r, g, b, a), as GL_UNSIGNED_INT_2_10_10_10_REV
/// packs it.
const fn a2(r: u32, g: u32, b: u32, a: u32) -> u32 {
    assert!(r < 1024 && g < 1024 && b < 1024 && a < 4);
    r | g << 10 | b << 20 | a << 30
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

/// What glReadPixels RGBA/UNSIGNED_BYTE makes of a word.
fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

/// An N x N colour attachment of its own framebuffer: a renderbuffer (of
/// `samples` samples, 0 for glRenderbufferStorage) or a texture.
struct Buf {
    fb: GLuint,
    rb: GLuint,
    tex: GLuint,
}

impl Drop for Buf {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(1, &self.rb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

/// A renderbuffer of `internal`; `samples` None is glRenderbufferStorage.
fn renderbuffer(internal: GLenum, samples: Option<GLsizei>) -> Buf {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        match samples {
            Some(n) => glRenderbufferStorageMultisample(GL_RENDERBUFFER, n, internal, N, N),
            None => glRenderbufferStorage(GL_RENDERBUFFER, internal, N, N),
        }
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer {internal:#x} samples {samples:?}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    Buf { fb, rb, tex: 0 }
}

/// A texture of `internal`; `px` holds N x N texels of `ty`.
fn texture(internal: GLenum, ty: GLenum, px: &[u32]) -> Buf {
    assert_eq!(px.len(), (N * N) as usize);
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; `px` holds N x N 32-bit texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, N, N, 0, GL_RGBA, ty, px.as_ptr() as *const c_void);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "texture {internal:#x}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{internal:#x}");
    }
    Buf { fb, rb: 0, tex }
}

fn rgb10a2_texture() -> Buf {
    texture(GL_RGB10_A2, GL_UNSIGNED_INT_2_10_10_10_REV, &[0; (N * N) as usize])
}

fn rgba8_texture() -> Buf {
    texture(GL_RGBA8, GL_UNSIGNED_BYTE, &[0x7f7f_7f7f; (N * N) as usize])
}

fn clear(b: &Buf, c: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, b.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
}

/// glBlitFramebuffer of the whole N x N colour buffer; returns the error.
fn blit(src: &Buf, dst: &Buf, filter: GLenum) -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(0, 0, N, N, 0, 0, N, N, GL_COLOR_BUFFER_BIT, filter);
    }
    err()
}

/// glReadPixels RGBA/UNSIGNED_BYTE of the whole buffer, row 0 first.
fn read_bytes(b: &Buf) -> Vec<u8> {
    let mut px = vec![0x77u8; (N * N) as usize * 4];
    // SAFETY: room for N x N RGBA8 pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, b.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

fn uniform(b: &Buf, word: u32, what: &str) {
    let want: Vec<u8> = (0..N * N).flat_map(|_| decoded(word)).collect();
    assert_eq!(read_bytes(b), want, "{what}");
}

/// Clear values whose words the stand-in stores exactly (R/G/B v * 1023 half
/// up, A v * 3 half up): R 1023, G 0, B 512, A 2.
const CLEAR: [f32; 4] = [1.0, 0.0, 512.0 / 1023.0, 2.0 / 3.0];
const WORD: u32 = a2(1023, 0, 512, 2);

/// ES 3.0 4.3.2: glReadPixels from a multisampled read framebuffer is
/// GL_INVALID_OPERATION, so the source of a resolve is judged by the
/// destination only.
fn read_multisample_is_invalid_operation(b: &Buf) {
    let mut px = [0x77u8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, b.fb);
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "glReadPixels of a multisampled buffer");
    assert_eq!(px, [0x77; 4]);
}

// (a) the sample counts and the storage

#[test]
fn sample_counts_of_rgb10_a2() {
    let _g = context();
    let (mut num, mut counts) = (-1, [-1; 4]);
    // SAFETY: room for 1 and 4 values.
    unsafe {
        glGetInternalformativ(GL_RENDERBUFFER, GL_RGB10_A2, GL_NUM_SAMPLE_COUNTS, 1, &mut num);
        glGetInternalformativ(GL_RENDERBUFFER, GL_RGB10_A2, GL_SAMPLES, 4, counts.as_mut_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!((num, counts), (1, [1, -1, -1, -1]), "one count, 1 (GL_MAX_SAMPLES 1)");
}

#[test]
fn multisample_storage_of_the_listed_count_is_complete() {
    let _g = context();
    let b = renderbuffer(GL_RGB10_A2, Some(1));
    let mut s = -1;
    // SAFETY: one int of room.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, b.rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut s);
    }
    assert_eq!(s, 1);
}

/// 4 is not listed: ES 3.0 4.4.2.1 refuses a count past GL_MAX_SAMPLES.
#[test]
fn multisample_storage_past_the_listed_count_is_invalid_operation() {
    let _g = context();
    let b = renderbuffer(GL_RGB10_A2, None);
    for n in [2, 4, 8] {
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, b.rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, n, GL_RGB10_A2, N, N);
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "{n} samples");
    }
}

// (b) clear and resolve into RGB10_A2

#[test]
fn clear_and_resolve_into_a_renderbuffer() {
    let _g = context();
    let (src, dst) = (renderbuffer(GL_RGB10_A2, Some(1)), renderbuffer(GL_RGB10_A2, None));
    clear(&src, CLEAR);
    read_multisample_is_invalid_operation(&src);
    assert_eq!(blit(&src, &dst, GL_NEAREST), GL_NO_ERROR);
    uniform(&dst, WORD, "resolved into a renderbuffer");
}

#[test]
fn clear_and_resolve_into_a_texture() {
    let _g = context();
    let (src, dst) = (renderbuffer(GL_RGB10_A2, Some(1)), rgb10a2_texture());
    clear(&src, CLEAR);
    assert_eq!(blit(&src, &dst, GL_NEAREST), GL_NO_ERROR);
    uniform(&dst, WORD, "resolved into a texture");
}

// (c) a resolve into RGBA8, and the plain blit it is not

/// A single-sample RGB10_A2 renderbuffer into RGBA8 is a plain blit, each
/// field decoded to a byte.
#[test]
fn single_sample_blit_into_rgba8() {
    let _g = context();
    let (src, dst) = (renderbuffer(GL_RGB10_A2, None), rgba8_texture());
    clear(&src, CLEAR);
    assert_eq!(blit(&src, &dst, GL_NEAREST), GL_NO_ERROR);
    uniform(&dst, WORD, "into RGBA8");
    uniform(&src, WORD, "the source");
}

// (d) the mismatched resolve

/// ES 3.0 4.3.3: a multisampled read buffer resolves only into a draw buffer
/// of the identical format, so RGBA8 is GL_INVALID_OPERATION and the
/// destination keeps its pixels.
#[test]
fn resolve_into_another_format_is_invalid_operation() {
    let _g = context();
    let (src, dst) = (renderbuffer(GL_RGB10_A2, Some(1)), rgba8_texture());
    clear(&src, CLEAR);
    assert_eq!(blit(&src, &dst, GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(read_bytes(&dst), vec![0x7f; (N * N) as usize * 4], "untouched");
}

/// The same rule with the destination an RGBA8 renderbuffer, and into a
/// multisampled destination (INVALID_OPERATION whatever the format).
#[test]
fn resolve_into_rgba8_renderbuffer_and_into_multisample_are_invalid_operation() {
    let _g = context();
    let src = renderbuffer(GL_RGB10_A2, Some(1));
    clear(&src, CLEAR);
    assert_eq!(blit(&src, &renderbuffer(GL_RGBA8, None), GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit(&src, &renderbuffer(GL_RGB10_A2, Some(1)), GL_NEAREST), GL_INVALID_OPERATION);
}
