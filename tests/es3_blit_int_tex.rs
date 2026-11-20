// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer between integer *texture* colour attachments (ES 3.0.6
//! 4.3.3; dEQP-GLES3 fbo.color.repeated_clear.blit.tex2d.<integer format>,
//! VK-GL-CTS es3fFboColorbufferTests.cpp at 93bca01).
//!
//! Until this file the GL core's copy (`src/gl/blit.rs` `attachment_image`)
//! had no case for an integer texture image: the blit dropped
//! GL_COLOR_BUFFER_BIT and answered GL_NO_ERROR with nothing copied, while
//! the same formats as renderbuffers were copied. An integer texture is now
//! copied in the layout of the class it is stored in, the way an integer
//! renderbuffer is: NEAREST only, values kept within one signedness and
//! saturated to the destination's width. An R or RG integer format that
//! es3_formats widened (R8UI kept in RGBA8UI) is copied in the wide store,
//! the channels the format lacks reading and written as 0, 0, 1; where the
//! build keeps such a texture incomplete (the V3D, `WIDE_INT_RT`) the blit
//! answers GL_INVALID_FRAMEBUFFER_OPERATION instead.
//!
//! Every value is read back through glReadPixels(GL_RGBA_INTEGER), never
//! from the storage. The expected pictures come from the spec's NEAREST rule
//! written apart from the code: destination pixel x of the destination
//! rectangle takes source texel floor(s0 + (x + 0.5 - d0) * (s1 - s0) /
//! (d1 - d0)) per axis.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_blit_int_tex
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_blit_int_tex
//! ```
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there (gl/framebuffer.rs `blit_framebuffer_is_es3_only`).
//!
//! The V3D and PowerVR builds only: the GC7000 renders no 32-bit integer
//! texture (`GC_INT_RT_128BPP`), so the RGBA32 cases have no attachment there
//! (tests/es3_blit_int_err.rs narrows them; the copy path is the same).

#![cfg(all(feature = "host-test", not(feature = "report-es2"), any(feature = "gpu-v3d", feature = "gpu-powervr")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host ---------------------------------------------- */

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
    name: c"es3-blit-int-tex".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glDisable(GL_SCISSOR_TEST);
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- formats and their values ------------------------------------------- */

/// An integer format: internal format, upload format and type, channel
/// count, channel width and signedness.
#[derive(Clone, Copy, Debug)]
struct Fmt {
    internal: GLenum,
    format: GLenum,
    ty: GLenum,
    channels: usize,
    bits: u32,
    signed: bool,
}

const fn fmt(internal: GLenum, format: GLenum, ty: GLenum, channels: usize, bits: u32, signed: bool) -> Fmt {
    Fmt { internal, format, ty, channels, bits, signed }
}

const RGBA8UI: Fmt = fmt(GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, 4, 8, false);
const RGBA8I: Fmt = fmt(GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE, 4, 8, true);
const RGBA16I: Fmt = fmt(GL_RGBA16I, GL_RGBA_INTEGER, GL_SHORT, 4, 16, true);
const RGBA16UI: Fmt = fmt(GL_RGBA16UI, GL_RGBA_INTEGER, GL_UNSIGNED_SHORT, 4, 16, false);
const RGBA32UI: Fmt = fmt(GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 4, 32, false);
const RGBA32I: Fmt = fmt(GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, 4, 32, true);
const R8UI: Fmt = fmt(GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, 1, 8, false);
const RG16I: Fmt = fmt(GL_RG16I, GL_RG_INTEGER, GL_SHORT, 2, 16, true);
const RGBA8: Fmt = fmt(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 4, 8, false);

/// The value of channel `ch` of texel (x, y) of image `seed`, in range of
/// `f` (signed values negative on odd texels; the widths' extremes appear).
fn value(f: Fmt, seed: u32, x: u32, y: u32, ch: usize) -> i64 {
    let n = (seed * 97 + y * 31 + x * 7 + ch as u32 * 13) as i64;
    let (lo, hi) =
        if f.signed { (-(1i64 << (f.bits - 1)), (1i64 << (f.bits - 1)) - 1) } else { (0, (1i64 << f.bits) - 1) };
    match (x + y + ch as u32) % 5 {
        0 => hi,
        1 => lo,
        _ if f.signed && (x + y) % 2 == 1 => -(n % (1 << (f.bits - 1).min(20))),
        _ if f.signed => n % (1 << (f.bits - 1).min(20)),
        _ => n % (1 << f.bits.min(20)),
    }
}

/// The bytes of a w x h image of `f` whose texel (x, y) is `value(seed..)`.
fn pixels(f: Fmt, seed: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for ch in 0..f.channels {
                let v = value(f, seed, x, y, ch);
                match f.bits {
                    8 => out.push(v as u8),
                    16 => out.extend_from_slice(&(v as u16).to_le_bytes()),
                    _ => out.extend_from_slice(&(v as u32).to_le_bytes()),
                }
            }
        }
    }
    out
}

/// What glReadPixels(GL_RGBA_INTEGER) answers for texel (x, y) of image
/// `seed` of `f`: its channels, the ones the format lacks 0, 0, 1.
fn expected(f: Fmt, seed: u32, x: u32, y: u32) -> [i64; 4] {
    core::array::from_fn(|ch| if ch < f.channels { value(f, seed, x, y, ch) } else { (ch == 3) as i64 })
}

/* ---- attachments --------------------------------------------------------- */

/// A colour attachment: an image of a texture at attachment 0 of its own
/// framebuffer, `w` x `h`. Dropped, it deletes its framebuffer and `tex`
/// (0 for a second view of a texture another `Att` owns).
struct Att {
    fb: GLuint,
    tex: GLuint,
    f: Fmt,
    w: u32,
    h: u32,
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

/// Attach an image of texture `tex` through `attach` to a new framebuffer
/// (owning `tex` when `owns`).
fn framebuffer(tex: GLuint, owns: bool, f: Fmt, w: u32, h: u32, attach: impl FnOnce()) -> Att {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        attach();
    }
    assert_eq!(err(), GL_NO_ERROR, "attaching a {:#x} texture", f.internal);
    Att { fb, tex: if owns { tex } else { 0 }, f, w, h }
}

/// Is the attachment's framebuffer complete?
fn complete(a: &Att) -> bool {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, a.fb);
        glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE
    }
}

/// [`framebuffer`], which this build must make complete.
fn complete_fb(tex: GLuint, owns: bool, f: Fmt, w: u32, h: u32, what: &str, attach: impl FnOnce()) -> Att {
    let a = framebuffer(tex, owns, f, w, h, attach);
    assert!(complete(&a), "{what} ({:#x}) is not complete", f.internal);
    a
}

/// A w x h 2D texture of `f` holding image `seed` at level 0, attached.
fn tex2d(f: Fmt, seed: u32, w: u32, h: u32) -> Att {
    let px = pixels(f, seed, w, h);
    let mut tex = 0;
    // SAFETY: room for one name; `px` holds w x h texels of `f`.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        let p = px.as_ptr() as *const c_void;
        glTexImage2D(GL_TEXTURE_2D, 0, f.internal as GLint, w as GLsizei, h as GLsizei, 0, f.format, f.ty, p);
    }
    assert_eq!(err(), GL_NO_ERROR, "a {:#x} texture", f.internal);
    framebuffer(tex, true, f, w, h, || unsafe {
        // SAFETY: plain GL call.
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0)
    })
}

/// [`tex2d`], which this build must make complete.
fn tex2d_complete(f: Fmt, seed: u32, w: u32, h: u32) -> Att {
    let a = tex2d(f, seed, w, h);
    assert!(complete(&a), "a {:#x} texture framebuffer is not complete", f.internal);
    a
}

/// glReadPixels(GL_RGBA_INTEGER) of the whole attachment, as signed values.
fn read(a: &Att) -> Vec<[i64; 4]> {
    let ty = if a.f.signed { GL_INT } else { GL_UNSIGNED_INT };
    let mut px = vec![0x5a5a_5a5au32; (a.w * a.h * 4) as usize];
    // SAFETY: room for w x h RGBA 32-bit values.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, a.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, a.w as GLsizei, a.h as GLsizei, GL_RGBA_INTEGER, ty, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels(GL_RGBA_INTEGER) of a {:#x} texture", a.f.internal);
    px.chunks(4)
        .map(|c| core::array::from_fn(|ch| if a.f.signed { c[ch] as i32 as i64 } else { c[ch] as i64 }))
        .collect()
}

/// A colour blit from `src` into `dst` with `filter`; its GL error.
fn blit(src: &Att, s: [GLint; 4], dst: &Att, d: [GLint; 4], filter: GLenum) -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], GL_COLOR_BUFFER_BIT, filter);
    }
    err()
}

/// The source texel NEAREST takes for destination pixel `o` of the axis
/// from `s0..s1` to `d0..d1` (None: outside the source, left alone).
fn nearest(s0: i32, s1: i32, d0: i32, d1: i32, o: i32, size: u32) -> Option<u32> {
    let (lo, hi) = (d0.min(d1), d0.max(d1));
    if o < lo || o >= hi {
        return None;
    }
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let t = p.floor();
    (t >= 0.0 && t < size as f64 && t >= s0.min(s1) as f64 && t < s0.max(s1) as f64).then_some(t as u32)
}

/// The picture of `dst` (holding image `dst_seed`) after a NEAREST blit of
/// rectangle `s` of `src` (image `src_seed`) into rectangle `d`.
fn model(src: &Att, src_seed: u32, s: [GLint; 4], dst: &Att, dst_seed: u32, d: [GLint; 4]) -> Vec<[i64; 4]> {
    let mut out = Vec::new();
    for y in 0..dst.h {
        for x in 0..dst.w {
            let sx = nearest(s[0], s[2], d[0], d[2], x as i32, src.w);
            let sy = nearest(s[1], s[3], d[1], d[3], y as i32, src.h);
            out.push(match (sx, sy) {
                (Some(sx), Some(sy)) => expected(src.f, src_seed, sx, sy),
                _ => expected(dst.f, dst_seed, x, y),
            });
        }
    }
    out
}

fn whole(a: &Att) -> [GLint; 4] {
    [0, 0, a.w as GLint, a.h as GLint]
}

/* ---- copies ----------------------------------------------------------------- */

/// The picture of a same-size NEAREST blit of a 2D integer texture: every
/// value arrives, read back with glReadPixels(GL_RGBA_INTEGER).
fn same_size(f: Fmt) {
    let src = tex2d_complete(f, 1, 8, 6);
    let dst = tex2d_complete(f, 2, 8, 6);
    assert_eq!(read(&dst), model(&dst, 2, whole(&dst), &dst, 2, whole(&dst)), "{:#x} before", f.internal);
    assert_eq!(blit(&src, whole(&src), &dst, whole(&dst), GL_NEAREST), GL_NO_ERROR, "{:#x}", f.internal);
    assert_eq!(read(&dst), model(&src, 1, whole(&src), &dst, 2, whole(&dst)), "{:#x} after", f.internal);
    assert_eq!(read(&dst), read(&src), "{:#x}: the copy equals its source", f.internal);
}

#[test]
fn rgba8ui_textures_copy_their_values() {
    let _g = context();
    same_size(RGBA8UI);
}

#[test]
fn rgba16i_textures_copy_their_values() {
    let _g = context();
    same_size(RGBA16I);
}

#[test]
fn rgba32ui_textures_copy_their_values() {
    let _g = context();
    same_size(RGBA32UI);
}

#[test]
fn the_other_four_channel_integer_textures_copy_their_values() {
    let _g = context();
    for f in [RGBA8I, RGBA16UI, RGBA32I] {
        same_size(f);
    }
}

/// A scaled, flipped and partial NEAREST blit takes the texels the spec's
/// rule names and leaves the rest of the destination alone.
#[test]
fn a_scaled_nearest_blit_takes_the_nearest_texels() {
    let _g = context();
    for f in [RGBA8UI, RGBA16I, RGBA32UI] {
        let src = tex2d_complete(f, 3, 8, 8);
        for (s, d) in [
            ([0, 0, 4, 4], [0, 0, 8, 8]),    /* up 2x */
            ([0, 0, 8, 8], [1, 2, 5, 6]),    /* down 2x into a part */
            ([1, 1, 7, 5], [7, 6, 0, 1]),    /* mirrored, 6:7 and 4:5 */
            ([-2, 0, 6, 8], [0, 0, 16, 16]), /* the source cut, the scale kept */
        ] {
            let dst = tex2d_complete(f, 4, 9, 7);
            assert_eq!(blit(&src, s, &dst, d, GL_NEAREST), GL_NO_ERROR, "{:#x} {s:?} -> {d:?}", f.internal);
            assert_eq!(read(&dst), model(&src, 3, s, &dst, 4, d), "{:#x} {s:?} -> {d:?}", f.internal);
        }
    }
}

/// Values a narrower destination cannot hold are saturated to its width
/// (dEQP's reference saturates too), within one signedness.
#[test]
fn a_narrower_destination_saturates() {
    let _g = context();
    let src = tex2d_complete(RGBA32UI, 5, 4, 4);
    let dst = tex2d_complete(RGBA8UI, 6, 4, 4);
    assert_eq!(blit(&src, whole(&src), &dst, whole(&dst), GL_NEAREST), GL_NO_ERROR);
    let want: Vec<[i64; 4]> =
        model(&src, 5, whole(&src), &src, 5, whole(&src)).into_iter().map(|p| p.map(|v| v.min(255))).collect();
    assert_eq!(read(&dst), want);
    let src = tex2d_complete(RGBA16I, 7, 4, 4);
    let dst = tex2d_complete(RGBA8I, 8, 4, 4);
    assert_eq!(blit(&src, whole(&src), &dst, whole(&dst), GL_NEAREST), GL_NO_ERROR);
    let want: Vec<[i64; 4]> =
        model(&src, 7, whole(&src), &src, 7, whole(&src)).into_iter().map(|p| p.map(|v| v.clamp(-128, 127))).collect();
    assert_eq!(read(&dst), want);
}

/// A layer of a 2D array texture (the source) and level 1 of a 2D texture
/// and a cube map face (the destinations) are the images attached, not
/// image 0 of level 0.
#[test]
fn an_array_layer_a_level_and_a_cube_face_are_the_images_attached() {
    let _g = context();
    let f = RGBA16I;
    let (w, h, layers) = (4u32, 4u32, 3u32);
    /* the 2D array: layer l holds image 10 + l */
    let mut arr = 0;
    let all: Vec<u8> = (0..layers).flat_map(|l| pixels(f, 10 + l, w, h)).collect();
    // SAFETY: room for one name; `all` holds w x h x layers texels of `f`.
    unsafe {
        glGenTextures(1, &mut arr);
        glBindTexture(GL_TEXTURE_2D_ARRAY, arr);
        let p = all.as_ptr() as *const c_void;
        let (wi, hi, li) = (w as GLsizei, h as GLsizei, layers as GLsizei);
        glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, f.internal as GLint, wi, hi, li, 0, f.format, f.ty, p);
    }
    assert_eq!(err(), GL_NO_ERROR, "the 2D array texture");
    let src = complete_fb(arr, true, f, w, h, "layer 1 of a 2D array", || unsafe {
        // SAFETY: plain GL call.
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 1)
    });

    /* level 1 of a 2D texture whose level 0 is twice the size */
    let mut t2 = 0;
    let (l0, l1) = (pixels(f, 20, 2 * w, 2 * h), pixels(f, 21, w, h));
    // SAFETY: room for one name; the levels hold their texels of `f`.
    unsafe {
        glGenTextures(1, &mut t2);
        glBindTexture(GL_TEXTURE_2D, t2);
        let (fi, ty, fo) = (f.internal as GLint, f.ty, f.format);
        glTexImage2D(GL_TEXTURE_2D, 0, fi, 2 * w as GLsizei, 2 * h as GLsizei, 0, fo, ty, l0.as_ptr() as _);
        glTexImage2D(GL_TEXTURE_2D, 1, fi, w as GLsizei, h as GLsizei, 0, fo, ty, l1.as_ptr() as _);
    }
    assert_eq!(err(), GL_NO_ERROR, "the two-level 2D texture");
    let lvl = complete_fb(t2, true, f, w, h, "level 1 of a 2D texture", || unsafe {
        // SAFETY: plain GL call.
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t2, 1)
    });
    assert_eq!(blit(&src, whole(&src), &lvl, whole(&lvl), GL_NEAREST), GL_NO_ERROR, "layer 1 into level 1");
    assert_eq!(read(&lvl), model(&src, 11, whole(&src), &lvl, 21, whole(&lvl)), "level 1 holds layer 1");

    /* the cube map: face i holds image 30 + i; +Y is written */
    let mut cube = 0;
    // SAFETY: room for one name; each face holds w x h texels of `f`.
    unsafe {
        glGenTextures(1, &mut cube);
        glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
        for i in 0..6 {
            let px = pixels(f, 30 + i, w, h);
            let target = GL_TEXTURE_CUBE_MAP_POSITIVE_X + i;
            let (wi, hi) = (w as GLsizei, h as GLsizei);
            glTexImage2D(target, 0, f.internal as GLint, wi, hi, 0, f.format, f.ty, px.as_ptr() as _);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the cube map");
    let face = complete_fb(cube, true, f, w, h, "a cube map face", || unsafe {
        // SAFETY: plain GL call.
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_Y, cube, 0)
    });
    assert_eq!(blit(&src, whole(&src), &face, [0, 0, 2, 2], GL_NEAREST), GL_NO_ERROR, "layer 1 into +Y");
    assert_eq!(read(&face), model(&src, 11, whole(&src), &face, 32, [0, 0, 2, 2]), "+Y holds layer 1, scaled");

    /* the other layers of the source, and the other faces, are untouched */
    for l in [0u32, 2] {
        let other = complete_fb(arr, false, f, w, h, "another layer", || unsafe {
            // SAFETY: plain GL call.
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, l as GLint)
        });
        untouched(&other, 10 + l);
    }
    let px = complete_fb(cube, false, f, w, h, "+X", || unsafe {
        // SAFETY: plain GL call.
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X, cube, 0)
    });
    untouched(&px, 30);
    /* level 0 of the two-level texture too */
    let l0 = complete_fb(t2, false, f, 2 * w, 2 * h, "level 0", || unsafe {
        // SAFETY: plain GL call.
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t2, 0)
    });
    untouched(&l0, 20);
}

/// An R or RG integer texture where this build makes it complete (es3_formats
/// keeps it in the four-channel store of its width) copies its channels; the
/// ones it lacks read 0, 0, 1. Where the build keeps it incomplete the blit
/// is GL_INVALID_FRAMEBUFFER_OPERATION, never a silent no-op.
#[test]
fn r_and_rg_integer_textures_copy_or_are_incomplete() {
    let _g = context();
    for (f, wide) in [(R8UI, RGBA8UI), (RG16I, RGBA16I)] {
        let src = tex2d(f, 40, 5, 3);
        if !complete(&src) {
            let other = tex2d_complete(wide, 42, 5, 3);
            let e = blit(&src, whole(&src), &other, whole(&other), GL_NEAREST);
            assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION, "{:#x} incomplete", f.internal);
            untouched(&other, 42);
            continue;
        }
        let dst = tex2d_complete(f, 41, 5, 3);
        assert_eq!(blit(&src, whole(&src), &dst, whole(&dst), GL_NEAREST), GL_NO_ERROR, "{:#x}", f.internal);
        assert_eq!(read(&dst), model(&src, 40, whole(&src), &dst, 41, whole(&dst)), "{:#x}", f.internal);
        /* into the four-channel format of its store: the lacking channels 0, 0, 1 */
        let other = tex2d_complete(wide, 43, 5, 3);
        assert_eq!(blit(&src, whole(&src), &other, whole(&other), GL_NEAREST), GL_NO_ERROR, "{:#x}", f.internal);
        assert_eq!(read(&other), model(&src, 40, whole(&src), &other, 43, whole(&other)), "{:#x}", f.internal);
    }
}

/* ---- errors (ES 3.0.6 4.3.3) -------------------------------------------- */

/// The attachment's values, still image `seed`.
fn untouched(a: &Att, seed: u32) {
    assert_eq!(read(a), model(a, seed, whole(a), a, seed, whole(a)), "{:#x} untouched", a.f.internal);
}

#[test]
fn linear_with_an_integer_read_buffer_is_invalid_operation() {
    let _g = context();
    for f in [RGBA8UI, RGBA16I, RGBA32UI] {
        let src = tex2d_complete(f, 50, 4, 4);
        let dst = tex2d_complete(f, 51, 8, 8);
        assert_eq!(blit(&src, whole(&src), &dst, whole(&dst), GL_LINEAR), GL_INVALID_OPERATION, "{:#x}", f.internal);
        /* the same size too: the rule is about the read buffer, not the scale */
        assert_eq!(blit(&src, whole(&src), &dst, [0, 0, 4, 4], GL_LINEAR), GL_INVALID_OPERATION, "{:#x}", f.internal);
        untouched(&dst, 51);
    }
}

#[test]
fn integer_into_non_integer_and_back_is_invalid_operation() {
    let _g = context();
    let int = tex2d_complete(RGBA16I, 52, 4, 4);
    let uint = tex2d_complete(RGBA8UI, 53, 4, 4);
    let norm = tex2d_complete(RGBA8, 54, 4, 4);
    assert_eq!(blit(&int, whole(&int), &norm, whole(&norm), GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit(&uint, whole(&uint), &norm, whole(&norm), GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit(&norm, whole(&norm), &int, whole(&int), GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit(&norm, whole(&norm), &uint, whole(&uint), GL_NEAREST), GL_INVALID_OPERATION);
    untouched(&int, 52);
    untouched(&uint, 53);
}

#[test]
fn signed_into_unsigned_and_back_is_invalid_operation() {
    let _g = context();
    let int = tex2d_complete(RGBA32I, 55, 4, 4);
    let uint = tex2d_complete(RGBA32UI, 56, 4, 4);
    assert_eq!(blit(&int, whole(&int), &uint, whole(&uint), GL_NEAREST), GL_INVALID_OPERATION);
    assert_eq!(blit(&uint, whole(&uint), &int, whole(&int), GL_NEAREST), GL_INVALID_OPERATION);
    untouched(&int, 55);
    untouched(&uint, 56);
}

/// An integer read buffer into two draw buffers, one of them of the other
/// signedness: GL_INVALID_OPERATION, nothing written; both of one kind:
/// each is written.
#[test]
fn every_draw_buffer_is_checked_and_written() {
    let _g = context();
    let src = tex2d_complete(RGBA16I, 60, 4, 4);
    let a = tex2d_complete(RGBA16I, 61, 4, 4);
    let b = tex2d_complete(RGBA32I, 62, 4, 4);
    let c = tex2d_complete(RGBA32UI, 63, 4, 4);
    let mut fb = 0;
    let mrt = |fb: GLuint, second: &Att| {
        // SAFETY: plain GL calls on names made here.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, a.tex, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, second.tex, 0);
            let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
            glDrawBuffers(2, bufs.as_ptr());
            glCheckFramebufferStatus(GL_FRAMEBUFFER)
        }
    };
    // SAFETY: room for one name; plain GL calls.
    unsafe { glGenFramebuffers(1, &mut fb) };
    if mrt(fb, &c) == GL_FRAMEBUFFER_COMPLETE {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb);
            glBlitFramebuffer(0, 0, 4, 4, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "a signed read buffer into an unsigned draw buffer");
        untouched(&a, 61);
        untouched(&c, 63);
    }
    assert_eq!(mrt(fb, &b), GL_FRAMEBUFFER_COMPLETE, "RGBA16I and RGBA32I textures together");
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb);
        glBlitFramebuffer(0, 0, 4, 4, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR, "an RGBA16I read buffer into RGBA16I and RGBA32I");
    let want = model(&src, 60, whole(&src), &src, 60, whole(&src));
    assert_eq!(read(&a), want, "draw buffer 0");
    assert_eq!(read(&b), want, "draw buffer 1 (wider: the values kept)");
    // SAFETY: a name made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}
