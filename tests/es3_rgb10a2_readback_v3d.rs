// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels of GL_RGB10_A2 renderbuffer and texture attachments on the
//! V3D host back end, through the GL entry points only (plan #370,
//! readback-api).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own render surface
//! (`GLES_SURF_RGB10_A2`): a pixel is the little-endian 10/10/10/2 word, red
//! in bits 0..9, alpha in 30..31. glReadPixels (src/gl/framebuffer.rs) then
//! hands out
//!
//! - GL_RGBA/GL_UNSIGNED_INT_2_10_10_10_REV: the word unchanged (ES 3.0
//!   table 3.2),
//! - GL_RGBA/GL_UNSIGNED_BYTE: each field scaled to a byte as
//!   (v * 255 + max / 2) / max, max 1023 for R, G, B and 3 for A,
//! - any other known pair (here RGBA/FLOAT): GL_INVALID_OPERATION, and the
//!   destination is not written.
//!
//! The buffers are filled by uploading words into a texture and blitting
//! them in (class to class keeps the words, see `es3_rgb10a2_blit_v3d`).
//!
//! `GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE` of such a read buffer answers
//! GL_RGBA/GL_UNSIGNED_INT_2_10_10_10_REV, the pair Mesa answers for a
//! normalized fixed-point 10/10/10/2 buffer and ES 3.0 6.1.7 allows, and
//! glReadPixels with the answered pair returns the words unchanged.
//! `implementation_color_read` (src/gl/framebuffer.rs) answers that pair.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_readback_v3d
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
    name: c"es3-rgb10a2-readback".as_ptr() as *const c_char,
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

const W: GLsizei = 4;
const H: GLsizei = 2;

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

/// A colour attachment: a texture at attachment 0 of its own framebuffer.
struct Att {
    fb: GLuint,
    tex: GLuint,
    w: GLsizei,
    h: GLsizei,
}

impl Drop for Att {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

fn attach(internal: GLenum, w: GLsizei, h: GLsizei, ty: GLenum, px: *const c_void) -> Att {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; `px` is null or holds w x h texels of `ty`.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, w, h, 0, GL_RGBA, ty, px);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "texture {internal:#x} {w}x{h}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{internal:#x}");
    }
    Att { fb, tex, w, h }
}

/// An RGB10_A2 texture of `words` (w x h of them, or none: zeros).
fn tex10(w: GLsizei, h: GLsizei, words: Option<&[u32]>) -> Att {
    let zero = vec![0u32; (w * h) as usize];
    let words = words.unwrap_or(&zero);
    assert_eq!(words.len(), (w * h) as usize);
    attach(GL_RGB10_A2, w, h, GL_UNSIGNED_INT_2_10_10_10_REV, words.as_ptr() as *const c_void)
}

fn blit(src: &Att, s: [GLint; 4], dst: &Att, d: [GLint; 4], filter: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], GL_COLOR_BUFFER_BIT, filter);
    }
    assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer");
}

fn whole(a: &Att) -> [GLint; 4] {
    [0, 0, a.w, a.h]
}

/// Edge words first: all zero, all ones, each field alone, alpha 0..3, and
/// values off the byte rounding midpoints.
fn edge_words() -> Vec<u32> {
    let px = [
        (0, 0, 0, 0),
        (1023, 1023, 1023, 3),
        (1, 0, 0, 1),
        (0, 1, 0, 2),
        (0, 0, 1, 3),
        (512, 513, 2, 0),
        (341, 682, 1022, 1),
        (100, 900, 700, 2),
    ];
    px.iter().map(|&(r, g, b, a)| a2(r, g, b, a)).collect()
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

/// An RGB10_A2 colour attachment (a texture, or a renderbuffer in `rb`).
struct Buf {
    att: Att,
    rb: GLuint,
}

impl Drop for Buf {
    fn drop(&mut self) {
        if self.rb != 0 {
            // SAFETY: a name this test made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteRenderbuffers(1, &self.rb);
            }
        }
    }
}

/// A texture attachment holding `words`, filled by a class to class blit.
fn filled_texture(words: &[u32]) -> Buf {
    let s = tex10(W, H, Some(words));
    let d = tex10(W, H, None);
    blit(&s, whole(&s), &d, whole(&d), GL_NEAREST);
    Buf { att: d, rb: 0 }
}

/// A renderbuffer attachment holding `words`, filled the same way.
fn filled_renderbuffer(words: &[u32]) -> Buf {
    let s = tex10(W, H, Some(words));
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, W, H);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer storage");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    let att = Att { fb, tex: 0, w: W, h: H };
    blit(&s, whole(&s), &att, whole(&att), GL_NEAREST);
    Buf { att, rb }
}

/// glReadPixels of the whole attachment into 0x77-filled bytes; returns the
/// bytes and the error.
fn read(b: &Buf, format: GLenum, ty: GLenum, bytes_per_px: usize) -> (Vec<u8>, GLenum) {
    let a = &b.att;
    let mut px = vec![0x77u8; (a.w * a.h) as usize * bytes_per_px];
    // SAFETY: room for w x h pixels of bytes_per_px bytes.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, a.w, a.h, format, ty, px.as_mut_ptr() as *mut c_void);
    }
    (px, err())
}

fn check(b: &Buf, words: &[u32]) {
    let (px, e) = read(b, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4);
    assert_eq!(e, GL_NO_ERROR, "RGBA/UNSIGNED_INT_2_10_10_10_REV");
    let got: Vec<u32> = px.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    assert_eq!(got, words, "the words, unchanged");

    let (px, e) = read(b, GL_RGBA, GL_UNSIGNED_BYTE, 4);
    assert_eq!(e, GL_NO_ERROR, "RGBA/UNSIGNED_BYTE");
    let want: Vec<u8> = words.iter().flat_map(|&w| decoded(w)).collect();
    assert_eq!(px, want, "the decoded bytes");

    let (px, e) = read(b, GL_RGBA, GL_FLOAT, 16);
    assert_eq!(e, GL_INVALID_OPERATION, "RGBA/FLOAT");
    assert!(px.iter().all(|&v| v == 0x77), "a refused read writes nothing");
}

#[test]
fn renderbuffer_reads_words_and_decoded_bytes() {
    let _g = context();
    let w = edge_words();
    check(&filled_renderbuffer(&w), &w);
}

#[test]
fn texture_reads_words_and_decoded_bytes() {
    let _g = context();
    let w = edge_words();
    check(&filled_texture(&w), &w);
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE with `b` as the read buffer.
fn read_pair(b: &Buf) -> (GLenum, GLenum) {
    let (mut format, mut ty) = (0, 0);
    // SAFETY: plain GL calls; one GLint of room each.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, b.att.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut format);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
    }
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv read pair");
    (format as GLenum, ty as GLenum)
}

/// The implementation answers the word pair for `b`, and glReadPixels with
/// that answered pair returns the words unchanged. Both are 4-byte pairs,
/// so `read` keeps working for either.
fn check_pair(b: &Buf, words: &[u32]) {
    let (format, ty) = read_pair(b);
    assert_eq!(
        (format, ty),
        (GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV),
        "the implementation read pair of an RGB10_A2 read buffer"
    );
    let (px, e) = read(b, format, ty, 4);
    assert_eq!(e, GL_NO_ERROR, "glReadPixels with the answered pair");
    let got: Vec<u32> = px.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    assert_eq!(got, words, "the answered pair returns the words unchanged");
}

#[test]
fn renderbuffer_read_pair_is_the_word_pair() {
    let _g = context();
    let w = edge_words();
    let b = filled_renderbuffer(&w);
    check_pair(&b, &w);
}

#[test]
fn texture_read_pair_is_the_word_pair() {
    let _g = context();
    let w = edge_words();
    let b = filled_texture(&w);
    check_pair(&b, &w);
}
