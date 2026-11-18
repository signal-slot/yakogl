// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer from and into GL_RGB10_A2 *texture* attachments on the
//! V3D host back end, through the GL entry points only (plan #370,
//! rgb10a2-blit-api).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class
//! (`GLES_TEXFMT_RGB10A2`) and render surface (`GLES_SURF_RGB10_A2`): a
//! texel is the little-endian 10/10/10/2 word, red in bits 0..9, alpha in
//! 30..31. The blit is the GL core's CPU copy (`src/gl/blit.rs`): a source of
//! the class decodes R, G, B over 1023 and A over 3, a destination encodes
//! round(clamp(v) * (2^b - 1)), class to class keeps the words, and LINEAR
//! filters the decoded values (18d83492 the renderbuffer arm, 4082f91a the
//! texture arm, 019aa841 the LINEAR and fallback lib tests
//! `gl::blit::tests::rgb10a2_*`, which hold the same through the internals). The
//! back end is the smallest stand-in that makes a context (nothing is drawn).
//!
//! An RGBA8 destination is read with glReadPixels. An RGB10_A2 destination
//! is read with glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) too, which since
//! 8428875e decodes the word as round(R * 255 / 1023), round(G * 255 / 1023),
//! round(B * 255 / 1023), round(A * 255 / 3), and is also looked at in its
//! storage (the host's "GPU memory" is CPU memory, rows of W texels) and by
//! blitting it back into RGBA8. The read pair and
//! GL_UNSIGNED_INT_2_10_10_10_REV are not used here.
//!
//! Covered: an RGB10_A2 texture into an RGBA8 texture, RGBA8 into RGB10_A2,
//! RGB10_A2 into RGB10_A2 word for word (and mirrored), and a LINEAR 2:1
//! reduction out of RGB10_A2 into both RGBA8 and RGB10_A2.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_blit_v3d
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
    name: c"es3-rgb10a2-blit".as_ptr() as *const c_char,
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

/// The word's channels as unit floats: R, G, B over 1023, A over 3.
fn decode(w: u32) -> [f64; 4] {
    let c = |s: u32, m: u32| ((w >> s) & m) as f64 / m as f64;
    [c(0, 0x3ff), c(10, 0x3ff), c(20, 0x3ff), c(30, 3)]
}

/// Unit floats as an RGBA8 pixel, rounded to nearest.
fn to_rgba8(v: [f64; 4]) -> [u8; 4] {
    v.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Unit floats as the 10/10/10/2 word, rounded to nearest.
fn to_a2(v: [f64; 4]) -> u32 {
    let q = |x: f64, m: f64| (x.clamp(0.0, 1.0) * m).round() as u32;
    a2(q(v[0], 1023.0), q(v[1], 1023.0), q(v[2], 1023.0), q(v[3], 3.0))
}

/// A W x H source image of the class: every channel varies with x and y,
/// and the values sit off the rounding midpoints of the 8-bit encode.
fn src_words() -> Vec<u32> {
    let px = [
        (1023, 512, 0, 3),
        (341, 1, 1022, 1),
        (100, 900, 700, 2),
        (0, 1023, 255, 0),
        (64, 128, 960, 1),
        (513, 300, 3, 3),
        (777, 20, 1000, 2),
        (900, 640, 33, 0),
    ];
    px.iter().map(|&(r, g, b, a)| a2(r, g, b, a)).collect()
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

/// An RGBA8 texture of `px` (w x h of them, or none: a fill of 0x5a).
fn tex8(w: GLsizei, h: GLsizei, px: Option<&[[u8; 4]]>) -> Att {
    let fill = vec![[0x5a; 4]; (w * h) as usize];
    let px = px.unwrap_or(&fill);
    assert_eq!(px.len(), (w * h) as usize);
    attach(GL_RGBA8, w, h, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void)
}

/// The words an RGB10_A2 texture holds, row by row, out of its storage.
fn words(a: &Att) -> Vec<u32> {
    let c = yakogl::gl::context::current().expect("a context");
    let t = &c.objs_ref().textures[a.tex as usize];
    let n = (a.w * a.h) as usize;
    assert!(t.mem.size >= n * 4, "storage of {} bytes for {n} texels", t.mem.size);
    // SAFETY: the storage holds at least n words (checked above).
    let b = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, n * 4) };
    b.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// An RGBA8 attachment through glReadPixels.
fn read8(a: &Att) -> Vec<[u8; 4]> {
    let mut px = vec![0x77u8; (a.w * a.h * 4) as usize];
    // SAFETY: room for w x h RGBA bytes.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, a.w, a.h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels of an RGBA8 texture");
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

/// What glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) hands out for RGB10_A2 words.
fn read_bytes_of(w: &[u32]) -> Vec<[u8; 4]> {
    w.iter().map(|&x| to_rgba8(decode(x))).collect()
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

/// An RGB10_A2 attachment as RGBA8 bytes: blitted (NEAREST, same size) into
/// a fresh RGBA8 texture and read back from there.
fn via_rgba8(a: &Att) -> Vec<[u8; 4]> {
    let o = tex8(a.w, a.h, None);
    blit(a, whole(a), &o, whole(&o), GL_NEAREST);
    read8(&o)
}

/// The upload keeps the words: GL_UNSIGNED_INT_2_10_10_10_REV data into the
/// class is stored as given (what every case below starts from).
#[test]
fn upload_stores_the_words() {
    let _g = context();
    let w = src_words();
    let s = tex10(W, H, Some(&w));
    assert_eq!(words(&s), w);
}

/// An RGB10_A2 texture source into an RGBA8 texture decodes the word: each
/// channel is round(c / 1023 * 255), alpha round(a / 3 * 255). The source
/// keeps its words.
#[test]
fn rgb10a2_texture_into_rgba8_texture() {
    let _g = context();
    let w = src_words();
    let s = tex10(W, H, Some(&w));
    let d = tex8(W, H, None);
    blit(&s, whole(&s), &d, whole(&d), GL_NEAREST);
    let want: Vec<[u8; 4]> = w.iter().map(|&x| to_rgba8(decode(x))).collect();
    assert_eq!(read8(&d), want);
    assert_eq!(words(&s), w, "the source keeps its words");
}

/// An RGBA8 texture source into an RGB10_A2 texture encodes: byte v is
/// round(v / 255 * 1023), alpha round(v / 255 * 3). Ten bits hold every
/// byte, so the destination blitted back into RGBA8 gives the bytes again.
#[test]
fn rgba8_texture_into_rgb10a2_texture() {
    let _g = context();
    let px: Vec<[u8; 4]> = vec![
        [255, 128, 0, 255],
        [85, 1, 254, 85],
        [200, 17, 64, 170],
        [0, 255, 99, 0],
        [12, 34, 56, 78],
        [250, 3, 129, 200],
        [127, 128, 126, 43],
        [9, 240, 180, 128],
    ];
    let s = tex8(W, H, Some(&px));
    let d = tex10(W, H, None);
    blit(&s, whole(&s), &d, whole(&d), GL_NEAREST);
    let want: Vec<u32> = px.iter().map(|p| to_a2(p.map(|v| v as f64 / 255.0))).collect();
    assert_eq!(words(&d), want, "the encoded words");
    assert_eq!(read8(&d), read_bytes_of(&want), "glReadPixels of the destination");
    /* alpha keeps two bits: byte v comes back as round(round(v / 255 * 3) / 3 * 255) */
    let a = |v: u8| ((v as f64 / 255.0 * 3.0).round() * 85.0) as u8;
    let back: Vec<[u8; 4]> = px.iter().map(|p| [p[0], p[1], p[2], a(p[3])]).collect();
    assert_eq!(via_rgba8(&d), back, "back into RGBA8: colour exact, alpha at two bits");
}

/// RGB10_A2 into RGB10_A2 with NEAREST is exact: every word survives,
/// including the low bits RGBA8 cannot hold; mirrored in x, word x lands
/// at W - 1 - x.
#[test]
fn rgb10a2_round_trip_is_exact() {
    let _g = context();
    let w = src_words();
    let s = tex10(W, H, Some(&w));
    let d = tex10(W, H, None);
    blit(&s, whole(&s), &d, whole(&d), GL_NEAREST);
    assert_eq!(words(&d), w, "class to class keeps the words");
    assert_eq!(read8(&d), read_bytes_of(&w), "glReadPixels of the destination");
    assert_eq!(via_rgba8(&d), via_rgba8(&s), "and reads as the source does");

    let m = tex10(W, H, None);
    blit(&d, whole(&d), &m, [W, 0, 0, H], GL_NEAREST);
    let want: Vec<u32> = (0..(W * H) as usize)
        .map(|i| {
            let (x, y) = (i % W as usize, i / W as usize);
            w[y * W as usize + (W as usize - 1 - x)]
        })
        .collect();
    assert_eq!(words(&m), want, "mirrored in x");
    assert_eq!(read8(&m), read_bytes_of(&want), "glReadPixels of the mirrored destination");
}

/// GL_LINEAR filters the decoded values, then the destination encodes: a
/// 2:1 reduction of W x H to W/2 x H/2 lands each destination centre on the
/// corner of a 2 x 2 block, so each pixel is the block's mean, for an RGBA8
/// destination and an RGB10_A2 one.
#[test]
fn linear_reduction_filters_after_decode() {
    let _g = context();
    let w = src_words();
    let s = tex10(W, H, Some(&w));
    let (dw, dh) = (W / 2, H / 2);
    let mean = |dx: usize, dy: usize| -> [f64; 4] {
        let mut m = [0.0; 4];
        for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let v = decode(w[(dy * 2 + sy) * W as usize + dx * 2 + sx]);
            for ch in 0..4 {
                m[ch] += v[ch] / 4.0;
            }
        }
        m
    };
    let means: Vec<[f64; 4]> = (0..(dw * dh) as usize).map(|i| mean(i % dw as usize, i / dw as usize)).collect();

    let o = tex8(dw, dh, None);
    blit(&s, whole(&s), &o, whole(&o), GL_LINEAR);
    assert_eq!(read8(&o), means.iter().map(|&m| to_rgba8(m)).collect::<Vec<_>>(), "into RGBA8");

    let d = tex10(dw, dh, None);
    blit(&s, whole(&s), &d, whole(&d), GL_LINEAR);
    let mw: Vec<u32> = means.iter().map(|&m| to_a2(m)).collect();
    assert_eq!(words(&d), mw, "into RGB10_A2");
    assert_eq!(read8(&d), read_bytes_of(&mw), "glReadPixels of the RGB10_A2 destination");

    /* the two-texel pair of the lib test: (0, 0, 1023, 0) and
     * (1023, 0, 1023, 3) average to 0.5, 0, 1, 0.5 */
    let p = tex10(2, 1, Some(&[a2(0, 0, 1023, 0), a2(1023, 0, 1023, 3)]));
    let o1 = tex8(1, 1, None);
    blit(&p, whole(&p), &o1, whole(&o1), GL_LINEAR);
    assert_eq!(read8(&o1), vec![[128, 0, 255, 128]]);
    let d1 = tex10(1, 1, None);
    blit(&p, whole(&p), &d1, whole(&d1), GL_LINEAR);
    assert_eq!(words(&d1), vec![a2(512, 0, 1023, 2)]);
    assert_eq!(read8(&d1), vec![[128, 0, 255, 170]]);
}
