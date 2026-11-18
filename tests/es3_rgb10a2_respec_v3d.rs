// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An attached texture re-specified between GL_RGBA8 and GL_RGB10_A2 (a
//! glTexImage2D on the attached level, the attachment left as it is) on the
//! V3D host back end, through the GL entry points only (plan #370,
//! respec-api): what glReadPixels, the CPU colour blit and the framebuffer
//! queries then report about that attachment.
//!
//! The attachment follows the texture's *current* format, not one cached at
//! attach time. While the texture is in the RGB10A2 class its attachment is
//! the 10/10/10/2 surface: glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) decodes
//! the stored words (8428875e), the attachment queries answer 10, 10, 10, 2,
//! and glBlitFramebuffer (src/gl/blit.rs `attachment_image`, the arms of
//! 18d83492 / 4082f91a) hands the decoded words to an RGBA8 destination.
//! The same call re-specifying the level back to GL_RGBA8 gives raw RGBA8
//! bytes and sizes of 8 again. A glTexImage2D at a level *above* the attached
//! one leaves the attached level's format and texels alone. The expected
//! bytes are rounded as in es3_rgb10a2_readback_v3d (read-back) and as the
//! CPU blit rounds (blit). The implementation read pair row runs since
//! 654b0620, with the rows of es3_rgb10a2_readback_v3d. Builds where the
//! class falls back to RGBA8 are
//! pinned by es3_rgb10a2_fallback and its siblings; this file runs only on
//! the build that stores the class.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_respec_v3d
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
    name: c"es3-rgb10a2-respec".as_ptr() as *const c_char,
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
    // SAFETY: room for one name each; `px` holds w x h texels of `ty`.
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

/// An RGBA8 texture of `px` (w x h pixels).
fn tex8(w: GLsizei, h: GLsizei, px: &[[u8; 4]]) -> Att {
    assert_eq!(px.len(), (w * h) as usize);
    attach(GL_RGBA8, w, h, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void)
}

/// An RGB10_A2 texture of `words` (w x h words).
fn tex10(w: GLsizei, h: GLsizei, words: &[u32]) -> Att {
    assert_eq!(words.len(), (w * h) as usize);
    attach(GL_RGB10_A2, w, h, GL_UNSIGNED_INT_2_10_10_10_REV, words.as_ptr() as *const c_void)
}

/// An RGBA8 renderbuffer attachment of its own framebuffer, W x H.
struct Rb {
    att: Att,
    rb: GLuint,
}

impl Drop for Rb {
    fn drop(&mut self) {
        // SAFETY: a name this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteRenderbuffers(1, &self.rb);
        }
    }
}

fn rgba8_renderbuffer() -> Rb {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, W, H);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer storage");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    Rb { att: Att { fb, tex: 0, w: W, h: H }, rb }
}

/// glTexImage2D on this texture again, at `level`: re-specification with no
/// new attachment call. `px` holds `w` x `h` texels of `ty`.
fn respec(a: &Att, level: GLint, internal: GLenum, ty: GLenum, w: GLsizei, h: GLsizei, px: *const c_void) {
    // SAFETY: plain GL calls; `px` holds w x h texels of `ty`.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, a.tex);
        glTexImage2D(GL_TEXTURE_2D, level, internal as GLint, w, h, 0, GL_RGBA, ty, px);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexImage2D level {level} {internal:#x}");
    // SAFETY: plain GL call.
    let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "the framebuffer stays complete through the re-specification");
}

/// The attachment sizes of `a`'s framebuffer: R, G, B, A.
fn att_bits(fb: GLuint) -> [GLint; 4] {
    const PNAME: [GLenum; 4] = [
        GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
        GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
        GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
        GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
    ];
    // SAFETY: plain GL calls; one GLint of room per query.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    core::array::from_fn(|i| {
        let mut v: GLint = -1;
        unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, PNAME[i], &mut v) };
        assert_eq!(err(), GL_NO_ERROR, "attachment query {:#x}", PNAME[i]);
        v
    })
}

fn blit(src: &Att, dst: &Att, filter: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(0, 0, src.w, src.h, 0, 0, dst.w, dst.h, GL_COLOR_BUFFER_BIT, filter);
    }
    assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer");
}

/// glReadPixels of the whole attachment into 0x77-filled bytes; returns the
/// bytes and the error.
fn read(a: &Att, format: GLenum, ty: GLenum, bytes_per_px: usize) -> (Vec<u8>, GLenum) {
    let mut px = vec![0x77u8; (a.w * a.h) as usize * bytes_per_px];
    // SAFETY: room for w x h pixels of bytes_per_px bytes.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, a.w, a.h, format, ty, px.as_mut_ptr() as *mut c_void);
    }
    let e = err();
    assert_eq!(e, GL_NO_ERROR, "glReadPixels {format:#x}/{ty:#x}");
    (px, e)
}

/// glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) of the whole attachment.
fn read_bytes(a: &Att) -> Vec<u8> {
    read(a, GL_RGBA, GL_UNSIGNED_BYTE, 4).0
}

/// glReadPixels(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV) words.
fn read_words(a: &Att) -> Vec<u32> {
    read(a, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4)
        .0
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE with `a` as the read buffer.
fn read_pair(a: &Att) -> (GLenum, GLenum) {
    let (mut format, mut ty) = (0, 0);
    // SAFETY: plain GL calls; one GLint of room each.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut format);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
    }
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv read pair");
    (format as GLenum, ty as GLenum)
}

/// A field read back as a byte: (v * 255 + max / 2) / max, the rounding of
/// es3_rgb10a2_readback_v3d.
fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

/// What glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) hands out for one word.
fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

/// What the CPU blit hands an RGBA8 destination for one RGB10_A2 word: the
/// source decodes v / max and the destination encodes round(x * 255) --
/// the same byte as `scale` for every 10-bit and 2-bit field.
fn blitted(w: u32) -> [u8; 4] {
    let f = |v: u32, m: u32| ((v as f64 / m as f64) * 255.0).round() as u8;
    [f(w & 0x3ff, 1023), f((w >> 10) & 0x3ff, 1023), f((w >> 20) & 0x3ff, 1023), f(w >> 30, 3)]
}

fn flat(px: &[[u8; 4]]) -> Vec<u8> {
    px.iter().flatten().copied().collect()
}

fn of_words(words: &[u32], f: fn(u32) -> [u8; 4]) -> Vec<u8> {
    words.iter().flat_map(|&w| f(w)).collect()
}

/// Known RGBA8 texels to fill an attachment with.
fn rgba8_px() -> Vec<[u8; 4]> {
    vec![
        [255, 0, 0, 255],
        [0, 255, 0, 128],
        [0, 0, 255, 0],
        [1, 2, 3, 4],
        [127, 128, 129, 130],
        [10, 20, 30, 40],
        [200, 100, 50, 25],
        [255, 255, 255, 255],
    ]
}

/// Known 10/10/10/2 words to re-specify an attachment with, on the edges
/// and off the byte-rounding midpoints.
fn words10() -> Vec<u32> {
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

/// (a): an RGBA8 texture attachment re-specified at its attached level with
/// RGB10_A2 words, the attachment left as it is. The framebuffer stays
/// complete, the attachment queries follow the new format, and glReadPixels
/// hands out the decoded words -- not the old RGBA8 bytes.
#[test]
fn respec_to_rgb10a2_reads_the_decoded_words() {
    let _g = context();
    let old = rgba8_px();
    let a = tex8(W, H, &old);
    assert_eq!(att_bits(a.fb), [8, 8, 8, 8], "the RGBA8 attachment");
    assert_eq!(read_bytes(&a), flat(&old), "the fill, raw");

    let w = words10();
    respec(&a, 0, GL_RGB10_A2, GL_UNSIGNED_INT_2_10_10_10_REV, W, H, w.as_ptr() as *const c_void);
    assert_eq!(att_bits(a.fb), [10, 10, 10, 2], "the re-specified attachment");
    let got = read_bytes(&a);
    assert_eq!(got, of_words(&w, decoded), "the words, decoded to bytes");
    assert_ne!(got, flat(&old), "not the old RGBA8 bytes");
    assert_eq!(read_words(&a), w, "and the word pair returns the words unchanged");
}

/// (b): the reverse. The same level re-specified back to RGBA8 reads back as
/// raw bytes and the sizes answer 8 again.
#[test]
fn respec_to_rgba8_reads_raw_bytes() {
    let _g = context();
    let w = words10();
    let a = tex10(W, H, &w);
    assert_eq!(att_bits(a.fb), [10, 10, 10, 2], "the RGB10_A2 attachment");
    assert_eq!(read_bytes(&a), of_words(&w, decoded), "the upload, decoded");

    let px = rgba8_px();
    respec(&a, 0, GL_RGBA8, GL_UNSIGNED_BYTE, W, H, px.as_ptr() as *const c_void);
    assert_eq!(att_bits(a.fb), [8, 8, 8, 8], "the re-specified attachment");
    assert_eq!(read_bytes(&a), flat(&px), "raw bytes, nothing decoded");
}

/// (c): after (a)'s re-specification, glBlitFramebuffer (NEAREST) into an
/// RGBA8 renderbuffer gives the decoded words.
#[test]
fn respec_to_rgb10a2_blits_decoded_into_rgba8() {
    let _g = context();
    let old = rgba8_px();
    let a = tex8(W, H, &old);
    let w = words10();
    respec(&a, 0, GL_RGB10_A2, GL_UNSIGNED_INT_2_10_10_10_REV, W, H, w.as_ptr() as *const c_void);
    assert_eq!(att_bits(a.fb), [10, 10, 10, 2], "the source attachment follows its re-specification");
    let d = rgba8_renderbuffer();
    blit(&a, &d.att, GL_NEAREST);
    assert_eq!(read_bytes(&d.att), of_words(&w, blitted), "the decoded words in an RGBA8 renderbuffer");
    assert_eq!(read_words(&a), w, "the source keeps its words");
}

/// (d): a glTexImage2D at a level *above* the attached one (level 1 of the
/// chain, another format) leaves the attachment as it is: complete, its
/// format, its words.
#[test]
fn respec_a_level_above_the_attached_one_leaves_it_alone() {
    let _g = context();
    let w = words10();
    let a = tex10(W, H, &w);
    assert_eq!(att_bits(a.fb), [10, 10, 10, 2], "the attached level 0");
    let lvl1: Vec<[u8; 4]> = vec![[9, 10, 11, 12], [200, 100, 50, 25]];
    respec(&a, 1, GL_RGBA8, GL_UNSIGNED_BYTE, W / 2, H / 2, lvl1.as_ptr() as *const c_void);
    assert_eq!(att_bits(a.fb), [10, 10, 10, 2], "the attachment keeps its format");
    assert_eq!(read_bytes(&a), of_words(&w, decoded), "the attached level still reads as RGB10_A2");
    assert_eq!(read_words(&a), w, "and its words are untouched");
}

/// (e): after (a), GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE answer the word
/// pair for the re-specified attachment.
#[test]
fn respec_to_rgb10a2_read_pair_is_the_word_pair() {
    let _g = context();
    let old = rgba8_px();
    let a = tex8(W, H, &old);
    let w = words10();
    respec(&a, 0, GL_RGB10_A2, GL_UNSIGNED_INT_2_10_10_10_REV, W, H, w.as_ptr() as *const c_void);
    assert_eq!(
        read_pair(&a),
        (GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV),
        "the implementation read pair of the re-specified attachment"
    );
    let (px, _) = read(&a, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4);
    let got: Vec<u32> = px.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    assert_eq!(got, w, "the answered pair returns the words unchanged");
}
