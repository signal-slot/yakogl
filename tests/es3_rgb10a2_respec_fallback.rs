// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An attached texture re-specified between GL_RGBA8 and GL_RGB10_A2 where
//! the build has no class for the latter (plan #370 item
//! respec-fallback-pin).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, the 10/10/10/2
//! word: src/gl/framebuffer.rs `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md);
//! there a glTexImage2D on the attached level moves the attachment to the
//! word rules (tests/es3_rgb10a2_respec_v3d.rs). Everywhere else a texture
//! of the name GL_RGB10_A2 is in the RGBA8 class and a GL_RGB10_A2
//! renderbuffer is RGBA8888 (es3_formats' fallback): the PowerVR and
//! GC7000 builds and every `report-es2` build. This file holds what those
//! builds answer when the attached level of a texture is re-specified, so
//! that the class build's word rules cannot reach them (sibling of
//! tests/es3_rgb10a2_fallback.rs and tests/es3_rgb10a2_texclear_fallback.rs,
//! same stand-in OSAL and back end, no GPU):
//!
//! - an RGBA8 texture attached to a framebuffer and re-specified at its
//!   attached level as GL_RGB10_A2 with GL_RGBA /
//!   GL_UNSIGNED_INT_2_10_10_10_REV (no error on the ES 3.0 fallback builds;
//!   the pair the fallback takes) stays the RGBA8 class: each word is stored
//!   as the RGBA8 bytes the generic ES 3.0 path makes of it (the field
//!   decoded to a float, c / 1023 and alpha c / 3, encoded as
//!   (v * 255 + 0.5) truncated), a glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE)
//!   of the attachment hands those bytes out unchanged and a clear of it
//!   reads the clear's RGBA8 bytes (alpha 0.4 is 102, where the class
//!   build's 2-bit alpha would decode 85); no `GLES_SURF_RGB10_A2` target is
//!   ever handed out;
//! - the same in the other direction: an RGB10_A2 attachment re-specified as
//!   GL_RGBA8 reads back the raw bytes uploaded with it;
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE stay GL_RGBA /
//!   GL_UNSIGNED_BYTE for the attachment across the re-specification
//!   (framebuffer.rs `implementation_color_read`: an RGBA8 buffer that is
//!   neither integer is that pair);
//! - glBlitFramebuffer NEAREST from a re-specified texture into an RGBA8
//!   renderbuffer copies those RGBA8 bytes unchanged, and
//!   glCheckFramebufferStatus stays GL_FRAMEBUFFER_COMPLETE after every
//!   re-specification.
//!
//! An ES 2.0-reporting build cannot allocate GL_RGB10_A2 at all (ES 2.0
//! names no such format and no ES 3.0 type): the re-specification into it is
//! GL_INVALID_ENUM and leaves the texture's RGBA8 image alone, and no
//! texture starts in the class (as tests/es3_rgb10a2_fallback.rs holds its
//! refusals).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_respec_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_respec_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_respec_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its
//! answers are tests/es3_rgb10a2_respec_v3d.rs).

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the refusal pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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

/// Targets the stand-in back end was handed: (surface, stored a clear).
static DRAWS: Mutex<Vec<(u8, bool)>> = Mutex::new(Vec::new());

/// A clear as a back end stores it into an RGBA8888 target: the colour
/// clamped to [0, 1] as one byte a channel, R, G, B, A, every pixel, rows at
/// the pair-padded pitch. Every other target is left alone.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for i in 0..d.num_targets as usize {
        let t = &d.targets[i];
        let hit = d.do_clear && (d.clear_target_mask == 0 || d.clear_target_mask & (1 << i) != 0) && t.gpu != 0;
        let rgba8 = t.format == backend::GLES_SURF_RGBA8888;
        DRAWS.lock().unwrap().push((t.format, hit && rgba8));
        if !hit || !rgba8 {
            continue;
        }
        let px = d.clear_rgba.map(unorm8);
        let pitch = ((t.w + 1) & !1) as usize;
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * 4), 4);
            }
        }
    }
    0
}

/// A colour channel as the byte an RGBA8 target stores: clamped, rounded.
fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
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
    name: c"es3-rgb10a2-respec-fallback".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    DRAWS.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
    }
    rb
}

/// A texture of `w` x `h`, its level 0 from `pixels` (null: only allocate),
/// left bound as GL_TEXTURE_2D.
fn texture(
    internalformat: GLenum,
    format: GLenum,
    ty: GLenum,
    w: GLsizei,
    h: GLsizei,
    pixels: *const c_void,
) -> GLuint {
    let mut t = 0;
    // SAFETY: room for one name; `pixels` is null or holds w x h texels.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, w, h, 0, format, ty, pixels);
    }
    t
}

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

/// An RGB10_A2 texture of `w` x `h`, uploaded through the pair the fallback
/// takes for it (GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV).
fn rgb10a2_texture(w: GLsizei, h: GLsizei, words: Option<&[u32]>) -> GLuint {
    let p = words.map_or(core::ptr::null(), |w| w.as_ptr() as *const c_void);
    texture(GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, w, h, p)
}

/// An RGBA8 texture of `w` x `h`, its level 0 from `px` (null: only allocate).
fn rgba8_texture(w: GLsizei, h: GLsizei, px: Option<&[[u8; 4]]>) -> GLuint {
    let p = px.map_or(core::ptr::null(), |b| b.as_ptr() as *const c_void);
    texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, w, h, p)
}

/// glTexImage2D on the *bound* texture's level `level`: re-specification, no
/// new attachment call. The error is the caller's to check.
fn respec(internalformat: GLenum, ty: GLenum, level: GLint, w: GLsizei, h: GLsizei, pixels: *const c_void) {
    // SAFETY: plain GL call; `pixels` is null or holds w x h texels of `ty`.
    unsafe { glTexImage2D(GL_TEXTURE_2D, level, internalformat as GLint, w, h, 0, GL_RGBA, ty, pixels) };
}

/// Re-specify the bound texture as GL_RGB10_A2 with `words`.
fn respec_rgb10a2(words: &[u32], w: GLsizei, h: GLsizei) {
    respec(GL_RGB10_A2, GL_UNSIGNED_INT_2_10_10_10_REV, 0, w, h, words.as_ptr() as *const c_void);
}

/// Re-specify the bound texture as GL_RGBA8 with `px`.
fn respec_rgba8(px: &[[u8; 4]], w: GLsizei, h: GLsizei) {
    respec(GL_RGBA8, GL_UNSIGNED_BYTE, 0, w, h, px.as_ptr() as *const c_void);
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with renderbuffer `rb` on colour
/// attachment 0.
fn rb_framebuffer(rb: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    fb
}

/// A framebuffer, bound to GL_FRAMEBUFFER, with level `level` of texture `tex`
/// on colour attachment 0.
fn tex_framebuffer(tex: GLuint, level: GLint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
    }
    fb
}

fn release(fbs: &[GLuint], rbs: &[GLuint], texs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as GLsizei, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
        glDeleteTextures(texs.len() as GLsizei, texs.as_ptr());
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// The bound read framebuffer's w x h pixels as GL_RGBA / GL_UNSIGNED_BYTE,
/// four bytes a pixel, the buffer prefilled with 0x77 so that a read that
/// writes nothing shows.
fn read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    let mut px = vec![0x77u8; (w * h * 4) as usize];
    // SAFETY: room for w x h pixels of four bytes.
    unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    px
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT and _TYPE of the bound read buffer.
fn implementation_pair() -> (GLenum, GLenum) {
    let (mut f, mut t) = (0, 0);
    // SAFETY: room for one value each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    (f as GLenum, t as GLenum)
}

/// The clear of the re-fill rows and the RGBA8 bytes it is stored as:
/// 0.2 * 255 = 51, 0.6 * 255 = 153, 1.5 clamps to 255, alpha 0.4 is 102.
/// As a 10/10/10/2 word decoded per channel the same clear would read
/// 51, 153, 255, 85 (alpha 0.4 is 1 of 3): the alpha tells the two apart.
const CLEAR: [f32; 4] = [0.2, 0.6, 1.5, 0.4];
const CLEAR_BYTES: [u8; 4] = [51, 153, 255, 102];

/// Clear the bound framebuffer to [`CLEAR`] and read its `w` x `h` pixels
/// back as GL_RGBA / GL_UNSIGNED_BYTE.
fn clear_and_read(w: GLsizei, h: GLsizei) -> Vec<u8> {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
    let px = read(w, h);
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels after glClear");
    px
}

/// What every pixel of `px` must be, `want` a single pixel.
fn assert_uniform(px: &[u8], want: &[u8; 4]) {
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        assert_eq!(*p, *want, "pixel {i}");
    }
}

/// Known RGBA8 texels to fill an attachment with; GL_RGBA8 stores the bytes
/// of GL_RGBA / GL_UNSIGNED_BYTE uploads unchanged.
const BYTES8: [[u8; 4]; 8] = [
    [255, 0, 0, 255],
    [0, 255, 0, 128],
    [0, 0, 255, 0],
    [1, 2, 3, 4],
    [127, 128, 129, 130],
    [10, 20, 30, 40],
    [200, 100, 50, 25],
    [255, 255, 255, 255],
];

/// 4 x 2 fields as 10/10/10/2 words: edges (0, 1, 2, 3, 341, 511, 512, 1022,
/// 1023, all four alphas) and the values of tests/es3_rgb10a2_texclear_fallback.rs'
/// grid whose RGBA8 bytes are pinned there.
const GRID: [[u32; 4]; 8] = [
    [0, 0, 0, 0],
    [341, 341, 341, 1],
    [875, 249, 207, 0],
    [249, 697, 640, 0],
    [511, 512, 1022, 2],
    [1023, 1023, 1023, 3],
    [207, 138, 461, 0],
    [697, 894, 258, 3],
];

/// [`GRID`] as 10/10/10/2 words.
fn grid_words() -> Vec<u32> {
    GRID.iter().map(|c| word(c[0], c[1], c[2], c[3])).collect()
}

/// [`GRID`] as the RGBA8 class stores it: the field decoded to c / 1023
/// (alpha c / 3) and encoded as (v * 255 + 0.5) truncated (es3_formats'
/// decode to floats and `GLES_TEXFMT_RGBA8` store, the table of
/// tests/es3_rgb10a2_texclear_fallback.rs).
const WANTED_WORDS: [[u8; 4]; 8] = [
    [0, 0, 0, 0],
    [85, 85, 85, 85],
    [218, 62, 52, 0],
    [62, 174, 160, 0],
    [127, 128, 255, 170],
    [255, 255, 255, 255],
    [52, 34, 115, 0],
    [174, 223, 64, 255],
];

/// Concatenate the wanted bytes of a pixel list.
fn want_bytes(pixels: &[[u8; 4]]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.iter().copied()).collect()
}

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format ---------------------------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 3.7.1 names no GL_RGB10_A2 internal format and no ES 3.0 type:
    /// re-specifying an attached RGBA8 texture as the class is refused with
    /// GL_INVALID_ENUM (as the fresh allocation of
    /// tests/es3_rgb10a2_fallback.rs), the texture keeps its RGBA8 image and
    /// bytes, the framebuffer stays complete and the implementation read
    /// pair stays GL_RGBA / GL_UNSIGNED_BYTE. Nothing of the class build's
    /// re-specification can be reached.
    #[test]
    fn respec_fallback_es2_respec_into_rgb10a2_is_refused() {
        let _g = context();
        let tex = rgba8_texture(4, 2, Some(&BYTES8));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGBA8)");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(read(4, 2), want_bytes(&BYTES8), "the RGBA8 fill");
        assert_eq!(err(), GL_NO_ERROR);
        let words = grid_words();
        respec_rgb10a2(&words, 4, 2);
        assert_eq!(err(), GL_INVALID_ENUM, "re-specification into GL_RGB10_A2 on an ES 2.0 report");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the attachment untouched by the refusal");
        assert_eq!(read(4, 2), want_bytes(&BYTES8), "the RGBA8 image untouched by the refusal");
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);
        release(&[fb], &[], &[tex]);
    }

    /// The reverse row cannot start on this build: GL_RGB10_A2 is no format
    /// (GL_INVALID_ENUM with the class type, GL_INVALID_VALUE with an ES 2.0
    /// type), so no texture is ever in it; a texture of the failed
    /// allocation re-specified to GL_RGBA8 is a plain RGBA8 attachment that
    /// fills, clears and reads as RGBA8.
    #[test]
    fn respec_fallback_es2_rgb10a2_is_not_allocatable() {
        let _g = context();
        let tex = rgb10a2_texture(4, 2, None);
        assert_eq!(err(), GL_INVALID_ENUM, "glTexImage2D(GL_RGB10_A2, .., GL_UNSIGNED_INT_2_10_10_10_REV)");
        respec_rgba8(&BYTES8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "re-specification to GL_RGBA8");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(read(4, 2), want_bytes(&BYTES8), "the raw bytes of the fill");
        assert_eq!(err(), GL_NO_ERROR);
        assert_uniform(&clear_and_read(4, 2), &CLEAR_BYTES);
        let draws = DRAWS.lock().unwrap().clone();
        assert!(draws.contains(&(backend::GLES_SURF_RGBA8888, true)), "the clear went out as RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);
        release(&[fb], &[], &[tex]);
    }
}

/* ---- ES 3.0 on a build without the class -------------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The premise: this build's back end does not draw the 10/10/10/2
    /// surface, so GL_RGB10_A2 falls back to RGBA8 (framebuffer.rs
    /// `rgb10a2_surface`).
    #[test]
    fn respec_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// (a): an RGBA8 texture attached at level 0, filled and read back, then
    /// re-specified at the attached level as GL_RGB10_A2 with GL_RGBA /
    /// GL_UNSIGNED_INT_2_10_10_10_REV. The pair is taken without error, and
    /// the attachment stays the RGBA8 class: the words are stored as their
    /// RGBA8 bytes and read back as those bytes, a clear of the attachment
    /// reaches the back end as an RGBA8888 target and reads the clear's
    /// RGBA8 bytes, and the implementation read pair stays GL_RGBA /
    /// GL_UNSIGNED_BYTE.
    #[test]
    fn respec_fallback_rgba8_into_rgb10a2_stays_rgba8() {
        let _g = context();
        let tex = rgba8_texture(4, 2, Some(&BYTES8));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGBA8)");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(read(4, 2), want_bytes(&BYTES8), "the RGBA8 fill before the re-specification");
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);

        let words = grid_words();
        respec_rgb10a2(&words, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "re-specification into GL_RGB10_A2 on a build without the class");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "complete after the re-specification");
        let px = read(4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels after the re-specification");
        assert_eq!(px, want_bytes(&WANTED_WORDS), "the RGBA8 bytes of the words, not a 10/10/10/2 read-back");
        assert_ne!(px, want_bytes(&BYTES8), "the re-specification did replace the texels");
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE), "the pair stays the RGBA8 one");
        assert_eq!(err(), GL_NO_ERROR);

        assert_uniform(&clear_and_read(4, 2), &CLEAR_BYTES);
        let draws = DRAWS.lock().unwrap().clone();
        assert!(draws.contains(&(backend::GLES_SURF_RGBA8888, true)), "the clear went out as RGBA8888: {draws:?}");
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        release(&[fb], &[], &[tex]);
    }

    /// (b): the reverse. An RGB10_A2 texture attached at level 0 reads the
    /// RGBA8 bytes of its words; re-specified as GL_RGBA8 it reads the raw
    /// uploaded bytes, stays complete and keeps the RGBA8 implementation
    /// read pair.
    #[test]
    fn respec_fallback_rgb10a2_into_rgba8_stays_rgba8() {
        let _g = context();
        let words = grid_words();
        let tex = rgb10a2_texture(4, 2, Some(&words));
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D(GL_RGB10_A2) with words");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(read(4, 2), want_bytes(&WANTED_WORDS), "the RGBA8 bytes before the re-specification");
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);

        respec_rgba8(&BYTES8, 4, 2);
        assert_eq!(err(), GL_NO_ERROR, "re-specification into GL_RGBA8");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "complete after the re-specification");
        let px = read(4, 2);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels after the re-specification");
        assert_eq!(px, want_bytes(&BYTES8), "the raw bytes uploaded as RGBA8, nothing decoded");
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(err(), GL_NO_ERROR);
        release(&[fb], &[], &[tex]);
    }

    /// (c): glBlitFramebuffer NEAREST from a re-specified texture into an
    /// RGBA8 renderbuffer copies the RGBA8 bytes the attachment holds,
    /// unchanged (the blit row of tests/es3_rgb10a2_fallback.rs, with the
    /// source re-specified first). The source is uploaded as words whose
    /// channels are multiples of a third (341 of 1023, 1 of 3), so the RGBA8
    /// store holds 0, 85, 170 or 255 exactly; every pixel differs from its
    /// neighbours.
    #[test]
    fn respec_fallback_blit_nearest_into_rgba8_renderbuffer() {
        let _g = context();
        let (w, h) = (4, 2);
        let level = |v: u32| v * 341;
        let byte = |v: u32| (v * 85) as u8;
        let chans: Vec<[u32; 4]> =
            (0..w * h).map(|i| [i % 4, (i / 4 + i) % 4, 3 - i % 4, (i + 1) % 4]).map(|c| c.map(|v| v as u32)).collect();
        let words: Vec<u32> = chans.iter().map(|c| word(level(c[0]), level(c[1]), level(c[2]), c[3])).collect();
        let want: Vec<u8> = chans.iter().flat_map(|c| c.map(byte)).collect();

        let src = rgba8_texture(w, h, None);
        assert_eq!(err(), GL_NO_ERROR);
        let s_fb = tex_framebuffer(src, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        respec_rgb10a2(&words, w, h);
        assert_eq!(err(), GL_NO_ERROR, "re-specification into GL_RGB10_A2");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the source stays complete through it");
        assert_eq!(read(w, h), want, "the source holds the RGBA8 bytes of the words");
        assert_eq!(err(), GL_NO_ERROR);

        let rb = renderbuffer(GL_RGBA8, w, h);
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = rb_framebuffer(rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        // SAFETY: plain GL calls on framebuffers this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, s_fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, d_fb);
            glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer(NEAREST)");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, d_fb) };
        assert_eq!(read(w, h), want, "the RGBA8 renderbuffer holds the same bytes");
        assert_eq!(err(), GL_NO_ERROR);
        let draws = DRAWS.lock().unwrap().clone();
        assert!(draws.iter().all(|d| d.0 != backend::GLES_SURF_RGB10_A2), "no 10/10/10/2 surface: {draws:?}");
        release(&[s_fb, d_fb], &[rb], &[src]);
    }

    /// (d): glCheckFramebufferStatus stays GL_FRAMEBUFFER_COMPLETE after
    /// every re-specification of the attached level, in both directions.
    #[test]
    fn respec_fallback_status_complete_after_each_respec() {
        let _g = context();
        let tex = rgba8_texture(4, 2, None);
        assert_eq!(err(), GL_NO_ERROR);
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let words = grid_words();
        let empty: [[u8; 4]; 8] = [[0; 4]; 8];
        for round in 0..2 {
            respec_rgb10a2(&words, 4, 2);
            assert_eq!(err(), GL_NO_ERROR, "re-specification {round} into GL_RGB10_A2");
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "complete after re-specification {round}");
            respec_rgba8(&empty, 4, 2);
            assert_eq!(err(), GL_NO_ERROR, "re-specification {round} into GL_RGBA8");
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "complete after re-specification {round}");
        }
        release(&[fb], &[], &[tex]);
    }
}
