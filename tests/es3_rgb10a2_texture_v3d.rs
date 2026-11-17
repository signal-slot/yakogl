// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 texture definition and mipmaps on the V3D host back end,
//! through the GL entry points only (plan #370, rgb10a2-tex-api).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class
//! (`GLES_TEXFMT_RGB10A2`): a texel is the little-endian 10/10/10/2 word, red
//! in bits 0..9, alpha in 30..31. The back end is the smallest stand-in that
//! makes a context (nothing is drawn).
//!
//! Every row reads the texture back through an FBO with glReadPixels
//! RGBA/UNSIGNED_BYTE, which decodes each field to a byte as
//! `(v * 255 + max / 2) / max` (max 1023 for R/G/B, 3 for A): round half up.
//! `glGenerateMipmap` averages field by field with `(sum + n / 2) / n`
//! (`box_average_rgb10a2`), n = 4 for a 2x2 box: round half up in the 10-bit
//! (2-bit alpha) domain, before the byte decode.
//!
//! Covered: glTexImage2D with edge words, glTexSubImage2D of a sub-rectangle,
//! glGenerateMipmap of an 8x8 level 0 (levels 1 and 2), and completeness of
//! the texture and its framebuffer.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_texture_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
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
    name: c"es3-rgb10a2-texture".as_ptr() as *const c_char,
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
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The 10/10/10/2 word of (r, g, b, a), as GL_UNSIGNED_INT_2_10_10_10_REV
/// packs it.
fn a2(r: u32, g: u32, b: u32, a: u32) -> u32 {
    assert!(r < 1024 && g < 1024 && b < 1024 && a < 4);
    r | g << 10 | b << 20 | a << 30
}

fn fields(w: u32) -> [u32; 4] {
    [w & 0x3ff, (w >> 10) & 0x3ff, (w >> 20) & 0x3ff, w >> 30]
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    let f = fields(w);
    [scale(f[0], 1023), scale(f[1], 1023), scale(f[2], 1023), scale(f[3], 3)]
}

/// The 2x2 box average of four words, field by field, round half up.
fn box4(ws: [u32; 4]) -> u32 {
    let mut m = [0u32; 4];
    for (k, mk) in m.iter_mut().enumerate() {
        let sum: u32 = ws.iter().map(|&w| fields(w)[k]).sum();
        *mk = (sum + 2) / 4;
    }
    a2(m[0], m[1], m[2], m[3])
}

/// An RGB10_A2 texture bound to GL_TEXTURE_2D, level 0 defined from words,
/// and a framebuffer to attach its levels to.
struct Tex {
    tex: GLuint,
    fb: GLuint,
}

impl Drop for Tex {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

fn define(w: GLsizei, h: GLsizei, words: &[u32]) -> Tex {
    assert_eq!(words.len(), (w * h) as usize);
    let (mut tex, mut fb) = (0, 0);
    // SAFETY: room for one name each; `words` holds w x h texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB10_A2 as GLint,
            w,
            h,
            0,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        );
        glGenFramebuffers(1, &mut fb);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexImage2D {w}x{h}");
    Tex { tex, fb }
}

/// glReadPixels RGBA/UNSIGNED_BYTE of `level` of the texture through its FBO.
fn read_level(t: &Tex, level: GLint, w: GLsizei, h: GLsizei) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h) as usize * 4];
    // SAFETY: room for w x h RGBA8 pixels.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, t.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t.tex, level);
        assert_eq!(err(), GL_NO_ERROR, "glFramebufferTexture2D level {level}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "level {level}");
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels level {level}");
    px
}

fn want_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|&w| decoded(w)).collect()
}

/// Edge words: 0, 1, 511, 512, 1022, 1023 in the colour fields, all four alphas.
fn edge_words() -> Vec<u32> {
    let px = [
        (0, 0, 0, 0),
        (1, 1, 1, 1),
        (511, 511, 511, 2),
        (512, 512, 512, 3),
        (1022, 1022, 1022, 0),
        (1023, 1023, 1023, 1),
        (1, 511, 1023, 2),
        (1023, 512, 0, 3),
    ];
    px.iter().map(|&(r, g, b, a)| a2(r, g, b, a)).collect()
}

#[test]
fn tex_image_reads_back_decoded() {
    let _g = context();
    let w = edge_words();
    let t = define(4, 2, &w);
    assert_eq!(read_level(&t, 0, 4, 2), want_bytes(&w));
    // the rule on known values: 1 -> 0, 511 -> 127, 512 -> 128, 1022 -> 255;
    // alpha 1 -> 85, 2 -> 170
    assert_eq!(decoded(a2(1, 511, 512, 1)), [0, 127, 128, 85]);
    assert_eq!(decoded(a2(1022, 1023, 0, 2)), [255, 255, 0, 170]);
}

#[test]
fn texture_is_complete_with_one_level() {
    let _g = context();
    let w = edge_words();
    let t = define(4, 2, &w);
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glBindFramebuffer(GL_FRAMEBUFFER, t.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t.tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn tex_sub_image_replaces_a_rectangle() {
    let _g = context();
    let mut w = vec![a2(10, 20, 30, 1); 16];
    let t = define(4, 4, &w);
    // a 2x2 block at (1, 2)
    let sub = [a2(0, 0, 0, 0), a2(1023, 1023, 1023, 3), a2(511, 512, 1022, 2), a2(1, 2, 3, 0)];
    // SAFETY: four words for a 2x2 rectangle.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, t.tex);
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            1,
            2,
            2,
            2,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            sub.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D");
    for (i, s) in sub.iter().enumerate() {
        w[(2 + i / 2) * 4 + 1 + i % 2] = *s;
    }
    assert_eq!(read_level(&t, 0, 4, 4), want_bytes(&w));
}

#[test]
fn generate_mipmap_box_averages_field_by_field() {
    let _g = context();
    // 8x8: every field varies with x and y so each box differs
    let w: Vec<u32> = (0..64u32)
        .map(|i| {
            let (x, y) = (i % 8, i / 8);
            a2(
                (x * 131 + y * 7 + 1) % 1024,
                (x * 17 + y * 211 + 3) % 1024,
                (1023 - x * 100 - y * 5) % 1024,
                (x + y) % 4,
            )
        })
        .collect();
    let t = define(8, 8, &w);
    // SAFETY: plain GL call.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, t.tex);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");

    let level = |src: &[u32], n: usize| -> Vec<u32> {
        let m = n / 2;
        (0..m * m)
            .map(|i| {
                let (x, y) = (i % m * 2, i / m * 2);
                box4([src[y * n + x], src[y * n + x + 1], src[(y + 1) * n + x], src[(y + 1) * n + x + 1]])
            })
            .collect()
    };
    let l1 = level(&w, 8);
    let l2 = level(&l1, 4);
    assert_eq!(read_level(&t, 0, 8, 8), want_bytes(&w), "level 0 untouched");
    assert_eq!(read_level(&t, 1, 4, 4), want_bytes(&l1), "level 1");
    assert_eq!(read_level(&t, 2, 2, 2), want_bytes(&l2), "level 2");
}

#[test]
fn generate_mipmap_rounds_half_up_per_field() {
    let _g = context();
    // field sums over the four texels: r 2, g 1, b 3, a 2: means 0.5, 0.25, 0.75, 0.5
    let w = [a2(1, 0, 1, 1), a2(1, 1, 1, 0), a2(0, 0, 1, 1), a2(0, 0, 0, 0)];
    let t = define(2, 2, &w);
    // SAFETY: plain GL call.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, t.tex);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    assert_eq!(err(), GL_NO_ERROR);
    // (sum + 2) / 4: r 1, g 0, b 1, a 1
    assert_eq!(box4(w), a2(1, 0, 1, 1));
    assert_eq!(read_level(&t, 1, 1, 1), want_bytes(&[a2(1, 0, 1, 1)]));
}
