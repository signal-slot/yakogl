// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 2D uploads with unpack parameters and from a pixel unpack
//! buffer on the V3D host back end, through the GL entry points only (plan
//! #370, unpack2d-api).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class
//! (`GLES_TEXFMT_RGB10A2`). The source is GL_RGBA /
//! GL_UNSIGNED_INT_2_10_10_10_REV: one 4-byte word per texel, red in bits
//! 0..9, alpha in 30..31. The back end is the smallest stand-in that makes a
//! context (nothing is drawn).
//!
//! Every row reads level 0 back through an FBO with glReadPixels
//! RGBA/UNSIGNED_BYTE, which decodes each field to a byte as
//! `(v * 255 + max / 2) / max` (max 1023 for R/G/B, 3 for A; 8428875e), and
//! compares it with the decoded expected words. The source image sits inside
//! a buffer of poison words, so a wrong skip, stride or offset shows.
//!
//! Covered, for glTexImage2D and glTexSubImage2D alike: GL_UNPACK_ROW_LENGTH
//! beyond the width; GL_UNPACK_SKIP_PIXELS and SKIP_ROWS; GL_UNPACK_ALIGNMENT
//! 1/2/4/8 (texels are 4 bytes, so only alignment 8 with an odd row length
//! pads a row); the source in a GL_PIXEL_UNPACK_BUFFER at a non-zero offset,
//! alone and with row length and skips; a buffer that ends exactly at the last
//! texel; a buffer too small (GL_INVALID_OPERATION, texture unchanged).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_unpack2d_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
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
    name: c"es3-rgb10a2-unpack2d".as_ptr() as *const c_char,
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

fn want_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|&w| decoded(w)).collect()
}

/// Edge words: 0, 1, 511, 512, 1022, 1023 in the colour fields, all four alphas.
const EDGES: [(u32, u32, u32, u32); 8] = [
    (0, 0, 0, 0),
    (1, 1, 1, 1),
    (511, 511, 511, 2),
    (512, 512, 512, 3),
    (1022, 1022, 1022, 0),
    (1023, 1023, 1023, 1),
    (1, 511, 1023, 2),
    (1023, 512, 0, 3),
];

/// A w x h image of edge words; the green field is taken from another edge
/// word so that neighbouring texels and the three colour fields differ.
fn image(w: usize, h: usize) -> Vec<u32> {
    (0..w * h)
        .map(|i| {
            let (r, _, b, a) = EDGES[i % EDGES.len()];
            let (_, g, _, _) = EDGES[(i / 2 + 3) % EDGES.len()];
            a2(r, g, b, a)
        })
        .collect()
}

/// What the source buffer holds outside the image.
const POISON: u32 = 0x2aaa_aaaa;
/// What the texture holds before a sub-image upload.
const BASE: u32 = 0x1234_5678;

/// One upload: image size, unpack state, where the source starts.
#[derive(Clone, Copy)]
struct Case {
    w: usize,
    h: usize,
    row_len: usize,
    skip_px: usize,
    skip_rows: usize,
    align: usize,
    /// Byte offset of the source in the unpack buffer, or in the client array.
    offset: usize,
    pbo: bool,
}

const PLAIN: Case = Case { w: 4, h: 3, row_len: 0, skip_px: 0, skip_rows: 0, align: 4, offset: 0, pbo: false };

impl Case {
    fn stride(&self) -> usize {
        let row = if self.row_len == 0 { self.w } else { self.row_len } * 4;
        let a = self.align.max(4);
        row.div_ceil(a) * a
    }
    /// Bytes the spec requires from the first byte of the buffer.
    fn need(&self) -> usize {
        self.offset + (self.skip_rows + self.h - 1) * self.stride() + (self.skip_px + self.w) * 4
    }
    /// The source bytes: poison everywhere, the image where the state reads it.
    fn source(&self, img: &[u32]) -> Vec<u8> {
        let stride = self.stride();
        let total = self.need() + 8;
        let mut v: Vec<u8> = POISON.to_le_bytes().into_iter().cycle().take(total).collect();
        for y in 0..self.h {
            for x in 0..self.w {
                let at = self.offset + (self.skip_rows + y) * stride + (self.skip_px + x) * 4;
                v[at..at + 4].copy_from_slice(&img[y * self.w + x].to_le_bytes());
            }
        }
        v
    }
}

/// Resets the unpack state and the unpack buffer binding on creation and drop.
struct Unpack;

fn reset_unpack() {
    // SAFETY: plain GL calls.
    unsafe {
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, 0);
    }
}

impl Unpack {
    fn reset() -> Unpack {
        reset_unpack();
        Unpack
    }
}

impl Drop for Unpack {
    fn drop(&mut self) {
        reset_unpack();
    }
}

/// An RGB10_A2 texture bound to GL_TEXTURE_2D, a framebuffer to attach it to,
/// and a pixel unpack buffer name.
struct Tex {
    tex: GLuint,
    fb: GLuint,
    pbo: GLuint,
}

impl Drop for Tex {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
            glDeleteBuffers(1, &self.pbo);
        }
    }
}

const TW: usize = 8;
const TH: usize = 6;

/// A TW x TH RGB10_A2 texture of `BASE` words, default unpack state.
fn base_tex() -> Tex {
    let (mut tex, mut fb, mut pbo) = (0, 0, 0);
    let words = [BASE; TW * TH];
    // SAFETY: room for one name each; `words` holds TW x TH texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB10_A2 as GLint,
            TW as GLsizei,
            TH as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        );
        glGenFramebuffers(1, &mut fb);
        glGenBuffers(1, &mut pbo);
    }
    assert_eq!(err(), GL_NO_ERROR, "base texture");
    Tex { tex, fb, pbo }
}

/// Level 0 through the FBO, glReadPixels RGBA/UNSIGNED_BYTE.
fn read0(t: &Tex, w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0x77u8; w * h * 4];
    // SAFETY: room for w x h RGBA8 pixels.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, t.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t.tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Call {
    Image,
    Sub,
}

/// Sets the unpack state of `c`, puts `src` where the call reads it (a bound
/// buffer of `buf_len` bytes, or client memory) and makes the call. `Sub`
/// writes at (2, 1) of the texture. Returns glGetError right after.
fn upload(t: &Tex, c: &Case, call: Call, src: &[u8], buf_len: usize) -> GLenum {
    // SAFETY: client memory holds `src`; a buffer offset is only an offset.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, t.tex);
        glPixelStorei(GL_UNPACK_ALIGNMENT, c.align as GLint);
        glPixelStorei(GL_UNPACK_ROW_LENGTH, c.row_len as GLint);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, c.skip_px as GLint);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, c.skip_rows as GLint);
        let p: *const c_void = if c.pbo {
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, t.pbo);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, buf_len as isize, src.as_ptr() as *const c_void, GL_STATIC_DRAW);
            assert_eq!(err(), GL_NO_ERROR, "glBufferData");
            c.offset as *const c_void
        } else {
            src.as_ptr().add(c.offset) as *const c_void
        };
        match call {
            Call::Image => glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGB10_A2 as GLint,
                c.w as GLsizei,
                c.h as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                p,
            ),
            Call::Sub => glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                2,
                1,
                c.w as GLsizei,
                c.h as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                p,
            ),
        }
    }
    err()
}

/// Both calls of `c`: the texture then holds the image (Image) or the image
/// at (2, 1) inside BASE words (Sub).
fn check(c: Case) {
    let img = image(c.w, c.h);
    let src = c.source(&img);
    for call in [Call::Image, Call::Sub] {
        let _u = Unpack::reset();
        let t = base_tex();
        assert_eq!(upload(&t, &c, call, &src, src.len()), GL_NO_ERROR, "{call:?}");
        reset_unpack();
        let (got, want) = match call {
            Call::Image => (read0(&t, c.w, c.h), want_bytes(&img)),
            Call::Sub => {
                let mut full = vec![BASE; TW * TH];
                for y in 0..c.h {
                    for x in 0..c.w {
                        full[(1 + y) * TW + 2 + x] = img[y * c.w + x];
                    }
                }
                (read0(&t, TW, TH), want_bytes(&full))
            }
        };
        assert_eq!(got, want, "{call:?}");
    }
}

#[test]
fn plain_client_upload() {
    let _g = context();
    check(PLAIN);
}

#[test]
fn row_length_beyond_width() {
    let _g = context();
    check(Case { row_len: 7, ..PLAIN });
    check(Case { row_len: 5, w: 5, h: 2, ..PLAIN });
}

#[test]
fn skip_pixels_and_rows() {
    let _g = context();
    check(Case { skip_px: 3, ..PLAIN });
    check(Case { skip_rows: 2, ..PLAIN });
    check(Case { skip_px: 2, skip_rows: 1, row_len: 9, ..PLAIN });
}

#[test]
fn alignment_1_2_4_8() {
    let _g = context();
    // texels are 4 bytes: alignments 1, 2 and 4 leave the stride alone
    for align in [1, 2, 4] {
        check(Case { align, ..PLAIN });
        check(Case { align, w: 3, row_len: 5, skip_px: 1, ..PLAIN });
    }
    // 8: even row lengths keep the stride, odd ones pad a row to 8 bytes
    check(Case { align: 8, ..PLAIN });
    check(Case { align: 8, w: 4, row_len: 6, skip_px: 1, ..PLAIN });
    check(Case { align: 8, w: 3, h: 3, ..PLAIN });
    check(Case { align: 8, w: 3, row_len: 5, skip_rows: 1, ..PLAIN });
}

#[test]
fn unpack_buffer_at_an_offset() {
    let _g = context();
    check(Case { pbo: true, ..PLAIN });
    for offset in [4, 8, 12, 20] {
        check(Case { pbo: true, offset, ..PLAIN });
    }
}

#[test]
fn unpack_buffer_with_row_length_and_skips() {
    let _g = context();
    check(Case { pbo: true, offset: 8, row_len: 7, ..PLAIN });
    check(Case { pbo: true, offset: 12, skip_px: 2, skip_rows: 1, ..PLAIN });
    check(Case { pbo: true, offset: 20, row_len: 9, skip_px: 3, skip_rows: 2, ..PLAIN });
    check(Case { pbo: true, offset: 4, align: 8, w: 3, row_len: 5, skip_px: 1, ..PLAIN });
}

#[test]
fn unpack_buffer_ending_at_the_last_texel() {
    let _g = context();
    // the spec needs the bytes up to the last texel, not a whole last stride
    for c in [
        Case { pbo: true, offset: 8, row_len: 7, ..PLAIN },
        Case { pbo: true, offset: 4, skip_px: 2, skip_rows: 1, row_len: 8, ..PLAIN },
    ] {
        let img = image(c.w, c.h);
        let src = c.source(&img);
        for call in [Call::Image, Call::Sub] {
            let _u = Unpack::reset();
            let t = base_tex();
            assert_eq!(upload(&t, &c, call, &src, c.need()), GL_NO_ERROR, "{call:?} need {}", c.need());
            reset_unpack();
            if call == Call::Image {
                assert_eq!(read0(&t, c.w, c.h), want_bytes(&img), "{call:?}");
            }
        }
    }
}

#[test]
fn unpack_buffer_too_small_is_invalid_and_leaves_the_texture() {
    let _g = context();
    let base = want_bytes(&[BASE; TW * TH]);
    for c in [
        PLAIN,
        Case { offset: 8, ..PLAIN },
        Case { offset: 8, row_len: 7, ..PLAIN },
        Case { offset: 4, skip_px: 2, skip_rows: 1, ..PLAIN },
    ] {
        let c = Case { pbo: true, ..c };
        let img = image(c.w, c.h);
        let src = c.source(&img);
        // one texel short, and a whole row short
        for short in [4, c.stride()] {
            for call in [Call::Image, Call::Sub] {
                let _u = Unpack::reset();
                let t = base_tex();
                let e = upload(&t, &c, call, &src, c.need() - short);
                assert_eq!(e, GL_INVALID_OPERATION, "{call:?} short {short}");
                while err() != GL_NO_ERROR {}
                reset_unpack();
                assert_eq!(read0(&t, TW, TH), base, "{call:?} short {short}: texture changed");
            }
        }
    }
}
