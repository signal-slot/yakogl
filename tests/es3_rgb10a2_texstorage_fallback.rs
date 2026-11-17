// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 immutable textures (glTexStorage2D / glTexStorage3D) where the
//! build has no class for the format (plan #370 item texstorage-fallback-pin).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, the 10/10/10/2
//! word: src/gl/framebuffer.rs `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md);
//! its immutable storage belongs to that build (plan #370 item texstorage-api,
//! tests/es3_rgb10a2_texstorage_v3d.rs). Everywhere else a GL_RGB10_A2 texture
//! is in the RGBA8 class (the es3_formats row's storage list
//! `[GLES_TEXFMT_RGB10A2, GLES_TEXFMT_RGBA8]`, `storage_here` taking the first
//! sampled one): the PowerVR and GC7000 builds and every `report-es2` build.
//! glTexStorage2D and glTexStorage3D are an entry point of their own
//! (src/gl/texture.rs 3.8.4): they lay the whole chain out at once, mark the
//! texture immutable, and record the same (format, type, texfmt) as
//! `sized_format` gives glTexImage2D/3D — so an RGB10_A2 texture made through
//! them must answer exactly what the pinned glTexImage attachments answer.
//! This file holds what those paths do today (sibling of
//! tests/es3_rgb10a2_texclear_fallback.rs and tests/es3_rgb10a2_layers_fallback.rs,
//! same stand-in OSAL and back end, no GPU):
//!
//! - glTexStorage2D(GL_TEXTURE_2D, .., GL_RGB10_A2, ..) and
//!   glTexStorage3D(GL_TEXTURE_2D_ARRAY, .., GL_RGB10_A2, ..) are taken with
//!   no error: the store is the RGBA8 one, GL_TEXTURE_IMMUTABLE_FORMAT is 1
//!   and GL_TEXTURE_IMMUTABLE_LEVELS the levels given (glGetTexParameteriv),
//!   and every allocated level and every layer attaches
//!   GL_FRAMEBUFFER_COMPLETE (immutable storage defines them all, ES 3.0
//!   3.8.4);
//! - the attachment size queries answer 8/8/8/8 as GL_UNSIGNED_NORMALIZED
//!   with GL_LINEAR encoding (framebuffer.rs `texture_bits` ->
//!   `es3_texture_bits`: "RGB10_A2 held as RGBA8 reads eight") and the
//!   implementation read pair stays GL_RGBA / GL_UNSIGNED_BYTE — the answers
//!   the glTexImage attachments of tests/es3_rgb10a2_fallback.rs and
//!   tests/es3_rgb10a2_layers_fallback.rs give;
//! - a glTexSubImage2D / glTexSubImage3D upload with GL_RGBA /
//!   GL_UNSIGNED_INT_2_10_10_10_REV (no error; the pair the fallback takes)
//!   stores each word as the RGBA8 bytes the generic ES 3.0 path makes of it
//!   (byte = trunc(c / 1023 * 255 + 0.5), alpha trunc(a / 3 * 255 + 0.5), the
//!   texclear_fallback rule), not kept as a word, and glReadPixels GL_RGBA /
//!   GL_UNSIGNED_BYTE returns those bytes unchanged (no decode);
//! - glGenerateMipmap on such a texture is the RGBA8 class box filter again:
//!   every stored byte averaged on its own, (sum + 2) / 4 (`mip_box_row_u8`),
//!   not the class build's field-by-field average of the word; the wanted
//!   level-1 bytes are texclear_fallback's, which differ from the field rule
//!   at the alphas (128 and 64 where the word rule would answer 170 and 85)
//!   and at a red (127 vs 126);
//! - one glBlitFramebuffer NEAREST from such an immutable texture attachment
//!   into an RGBA8 texture copies those bytes (both arms are texture
//!   attachments; on this build the source's RGBA8 class has its image in
//!   `attachment_image`, where the class build maps its own class).
//!
//! An ES 2.0-reporting build names no GL_RGB10_A2 (es3_formats::find answers
//! none, texture.rs `sized_format` returns None for it): both entry points
//! exist but refuse the internal format with GL_INVALID_ENUM, before they
//! look at the target or the texture, and the texture stays mutable. The
//! refusal is what its rows run (as in tests/es3_rgb10a2_fallback.rs); the
//! immutable storage itself is never allocated there, so the rows of the
//! fallback behaviour — the array of glTexStorage3D first, which ES 2.0 has
//! no such texture target for and which is refused for its format alone —
//! live in the es3 module, compiled out under `report-es2` (as in
//! tests/es3_rgb10a2_layers_fallback.rs). No row of the fallback behaviour
//! needs a cfg of its own beyond that.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_texstorage_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_texstorage_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_texstorage_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its answers
//! are tests/es3_rgb10a2_texstorage_v3d.rs).

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
/// No draw reaches the stand-in GPU in this file (no clear, no draw call; the
/// blit is the CPU copy of the attachment bytes).
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-rgb10a2-texstorage-fallback".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// glGetTexParameteriv of the texture bound to `target`; the initial -1 shows
/// when the query wrote nothing.
fn texparam(target: GLenum, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: room for one value.
    unsafe { glGetTexParameteriv(target, pname, &mut v) };
    v
}

/// The bound framebuffer's colour attachment 0 answers one image pname
/// (GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE &c).
fn fb_bits(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: room for one value.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
    v
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

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

/// A field of `max` as the RGBA8 byte the fallback stores:
/// trunc(c / max * 255 + 0.5).
fn byte(c: u32, max: u32) -> u8 {
    (c as f32 / max as f32 * 255.0 + 0.5) as u8
}

/// The RGBA8 bytes the fallback stores for a word.
fn stored(w: u32) -> [u8; 4] {
    [byte(w & 0x3ff, 1023), byte(w >> 10 & 0x3ff, 1023), byte(w >> 20 & 0x3ff, 1023), byte(w >> 30, 3)]
}

/// An RGB10_A2 texture of `w` x `h` made by glTexStorage2D with `levels`
/// levels; the texture ends up bound to GL_TEXTURE_2D.
fn rgb10a2_storage2d(w: GLsizei, h: GLsizei, levels: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, levels, GL_RGB10_A2, w, h);
    }
    tex
}

/// An RGB10_A2 texture array of 4 x 4 x `layers` made by glTexStorage3D with
/// `levels` levels; the texture ends up bound to GL_TEXTURE_2D_ARRAY.
fn rgb10a2_storage3d_array(layers: GLsizei, levels: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
        glTexStorage3D(GL_TEXTURE_2D_ARRAY, levels, GL_RGB10_A2, 4, 4, layers);
    }
    tex
}

/// An RGBA8 texture of `w` x `h`, allocated only (the blit destination).
fn rgba8_texture(w: GLsizei, h: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; null pixels allocate only.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
    }
    tex
}

/// Upload level 0 of the texture bound to GL_TEXTURE_2D with the pair the
/// fallback takes for GL_RGB10_A2 (GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV).
fn upload_words_2d(w: GLsizei, h: GLsizei, words: &[u32]) {
    // SAFETY: `words` holds w x h words.
    unsafe {
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            0,
            0,
            w,
            h,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        )
    };
}

/// The same for the array bound to GL_TEXTURE_2D_ARRAY: one
/// glTexSubImage3D covering all its layers (glTexSubImage3D is no call for
/// a GL_TEXTURE_2D texture; tex_sub_image_3d refuses that target).
fn upload_words_array(d: GLsizei, words: &[u32]) {
    // SAFETY: `words` holds 4 x 4 x d words.
    unsafe {
        glTexSubImage3D(
            GL_TEXTURE_2D_ARRAY,
            0,
            0,
            0,
            0,
            4,
            4,
            d,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            words.as_ptr() as *const c_void,
        )
    };
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

/// Re-attach level `level` of texture `tex` to colour attachment 0 of the
/// bound framebuffer.
fn attach_level(tex: GLuint, level: GLint) {
    // SAFETY: plain GL call on a framebuffer the caller made.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level) };
}

/// Attach layer `layer` of texture `tex` to colour attachment 0 of the bound
/// framebuffer.
fn attach_layer(tex: GLuint, layer: GLint) {
    // SAFETY: plain GL call on a framebuffer the caller made.
    unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer) };
}

fn release(fbs: &[GLuint], texs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as GLsizei, fbs.as_ptr());
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

/// Concatenate the wanted bytes of a pixel list.
fn want_bytes(pixels: &[[u8; 4]]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.iter().copied()).collect()
}

/* ---- the ES 2.0 report: GL_RGB10_A2 is no format, immutable or not ---------- */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 3.0 3.8.4 laid out under an ES 2.0 report: ES 2.0 names no
    /// GL_RGB10_A2, so `sized_format` answers no (format, type, texfmt) for
    /// it and both entry points refuse the internal format with
    /// GL_INVALID_ENUM before they look at the target or the texture (as
    /// tests/es3_rgb10a2_fallback.rs holds for glTexImage2D), so no immutable
    /// storage of the format exists there. glTexStorage3D is refused the same
    /// way for the array and the 3D target (ES 2.0 has no such texture target
    /// either, tests/es3_rgb10a2_layers_fallback.rs). The immutable-state
    /// names are ES 3.0's too: the getter refuses them with GL_INVALID_ENUM
    /// and writes nothing (texture.rs `es3_only_tex_param`, pinned by
    /// tests/es2_error_rules_texture.rs).
    #[test]
    fn texstorage_fallback_es2_storage_is_not_allocated() {
        let _g = context();
        let mut tex = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexStorage2D(GL_TEXTURE_2D, 3, GL_RGB10_A2, 4, 4);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage2D(GL_RGB10_A2)");
        let v = texparam(GL_TEXTURE_2D, GL_TEXTURE_IMMUTABLE_FORMAT);
        assert_eq!(v, -1, "the query wrote nothing");
        assert_eq!(err(), GL_INVALID_ENUM, "GL_TEXTURE_IMMUTABLE_FORMAT is ES 3.0's name");
        // SAFETY: plain GL call; the refusal is the format, before the target.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 2, GL_RGB10_A2, 4, 4, 3) };
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage3D(GL_TEXTURE_2D_ARRAY, GL_RGB10_A2)");
        // SAFETY: plain GL call, as above.
        unsafe { glTexStorage3D(GL_TEXTURE_3D, 2, GL_RGB10_A2, 4, 4, 3) };
        assert_eq!(err(), GL_INVALID_ENUM, "glTexStorage3D(GL_TEXTURE_3D, GL_RGB10_A2)");
        // SAFETY: name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }
}

/* ---- ES 3.0 on a build without the class -------------------------------------- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    /// The premise: this build's back end does not draw the 10/10/10/2
    /// surface, so GL_RGB10_A2 falls back to RGBA8 (framebuffer.rs
    /// `rgb10a2_surface`, es3_formats' `storage_here`).
    #[test]
    fn texstorage_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// glTexStorage2D(GL_RGB10_A2) is taken with no error and lays out the
    /// RGBA8 store: the texture is immutable with the levels it was given,
    /// every allocated level attaches complete, and the attachment answers
    /// the size queries of the RGBA8 store and the implementation read pair
    /// of an RGBA8 buffer — the answers the glTexImage attachments give
    /// (tests/es3_rgb10a2_fallback.rs).
    #[test]
    fn texstorage_fallback_texstorage2d_is_rgba8_storage() {
        let _g = context();
        let tex = rgb10a2_storage2d(4, 4, 3);
        assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D(GL_RGB10_A2)");
        assert_eq!(texparam(GL_TEXTURE_2D, GL_TEXTURE_IMMUTABLE_FORMAT), 1, "immutable");
        assert_eq!(texparam(GL_TEXTURE_2D, GL_TEXTURE_IMMUTABLE_LEVELS), 3, "the levels given");
        let fb = tex_framebuffer(tex, 0);
        for level in 0..3 {
            attach_level(tex, level);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "level {level} of immutable storage");
            assert_eq!(
                (
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE),
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE),
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE),
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE),
                ),
                (8, 8, 8, 8),
                "level {level}: the RGBA8 store's sizes, where the class build answers 10/10/10/2"
            );
            assert_eq!(fb_bits(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), GL_UNSIGNED_NORMALIZED as GLint);
            assert_eq!(fb_bits(GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING), GL_LINEAR as GLint);
            assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE), "level {level}: the RGBA8 read pair");
            assert_eq!(err(), GL_NO_ERROR, "the attachment queries of level {level}");
        }
        release(&[fb], &[tex]);
    }

    /// A 4 x 4 texture's fields, row 0 first — texclear_fallback's grid, with
    /// the edge values (0, 1, 2, 3, 341, 511, 512, 1022, 1023, all four
    /// alphas) and one box (x 2..3, y 0..1) whose byte average differs from
    /// the class build's field average at its red channel.
    const GRID: [[u32; 4]; 16] = [
        [0, 0, 0, 0],
        [341, 341, 341, 1],
        [875, 249, 207, 0],
        [249, 697, 640, 0],
        [511, 512, 1022, 2],
        [1023, 1023, 1023, 3],
        [207, 138, 461, 0],
        [697, 894, 258, 3],
        [1, 2, 3, 1],
        [2, 3, 1, 1],
        [0, 1023, 0, 3],
        [1023, 0, 1023, 0],
        [3, 1, 2, 2],
        [1022, 1021, 1020, 2],
        [512, 511, 512, 1],
        [341, 682, 1023, 2],
    ];

    /// [`GRID`] as 10/10/10/2 words.
    fn grid_words() -> Vec<u32> {
        GRID.iter().map(|c| word(c[0], c[1], c[2], c[3])).collect()
    }

    /// Level 0 as the RGBA8 class stores [`GRID`]: the field decoded to
    /// c / 1023 (alpha c / 3) and encoded as (v * 255 + 0.5) truncated
    /// (es3_formats' decode to floats and `GLES_TEXFMT_RGBA8` store). The
    /// table tests/es3_rgb10a2_texclear_fallback.rs pins for glTexImage.
    const WANTED_L0: [[u8; 4]; 16] = [
        [0, 0, 0, 0],
        [85, 85, 85, 85],
        [218, 62, 52, 0],
        [62, 174, 160, 0],
        [127, 128, 255, 170],
        [255, 255, 255, 255],
        [52, 34, 115, 0],
        [174, 223, 64, 255],
        [0, 0, 1, 85],
        [0, 1, 0, 85],
        [0, 255, 0, 255],
        [255, 0, 255, 0],
        [1, 0, 0, 170],
        [255, 255, 254, 170],
        [128, 127, 128, 85],
        [85, 170, 255, 170],
    ];

    /// Level 1 (2 x 2) after glGenerateMipmap: each byte the (sum + 2) / 4
    /// average of the four stored bytes of its 2 x 2 box (`mip_box_row_u8`).
    /// The class build's word rules would answer alpha 170, 85, 170, 170 and
    /// a red of 126 in the second box; the byte rule answers 128, 64, 128,
    /// 128 and 127. The table texclear_fallback pins for glTexImage.
    const WANTED_L1: [[u8; 4]; 4] = [[117, 117, 149, 128], [127, 123, 98, 64], [64, 64, 64, 128], [117, 138, 160, 128]];

    /// The word every texel of layer `i` of the array starts with; one per
    /// layer, with a 10-bit field that no byte rounding confuses with a
    /// neighbour's (the sentinel tests/es3_rgb10a2_layers_fallback.rs uses).
    fn sentinel(i: u32) -> u32 {
        word(1000 - i, 5 + i, 600, 1 + i % 3)
    }

    /// Upload level 0 of a glTexStorage2D RGB10_A2 texture with [`GRID`] as
    /// words: the pair the fallback takes without error, its texels stored as
    /// the RGBA8 bytes of the words and read back unchanged (no 10/10/10/2
    /// word, no decode).
    #[test]
    fn texstorage_fallback_upload_reads_rgba8_bytes() {
        let _g = context();
        let words = grid_words();
        let tex = rgb10a2_storage2d(4, 4, 3);
        assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D(GL_RGB10_A2)");
        upload_words_2d(4, 4, &words);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D into immutable storage");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let px = read(4, 4);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
        assert_eq!(px, want_bytes(&WANTED_L0), "the stored RGBA8 bytes of the words");
        release(&[fb], &[tex]);
    }

    /// glGenerateMipmap on such a texture is allowed and is the RGBA8 class
    /// box filter: level 0 untouched, level 1 the byte average of each 2 x 2
    /// box, not the class build's average of the 10/10/10/2 words (the same
    /// level-1 bytes texclear_fallback pins for a glTexImage texture).
    #[test]
    fn texstorage_fallback_generate_mipmap_averages_bytes() {
        let _g = context();
        let words = grid_words();
        let tex = rgb10a2_storage2d(4, 4, 3);
        assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D(GL_RGB10_A2)");
        upload_words_2d(4, 4, &words);
        assert_eq!(err(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap on immutable storage");
        let fb = tex_framebuffer(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let px = read(4, 4);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(px, want_bytes(&WANTED_L0), "level 0 untouched");
        attach_level(tex, 1);
        assert_eq!(err(), GL_NO_ERROR, "attach level 1");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "level 1 of immutable storage");
        let px = read(2, 2);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels of level 1");
        assert_eq!(px, want_bytes(&WANTED_L1), "the byte box average");
        release(&[fb], &[tex]);
    }

    /// glTexStorage3D(GL_TEXTURE_2D_ARRAY, GL_RGB10_A2): taken with no error
    /// as RGBA8 storage, immutable with the levels given; every layer
    /// attaches complete, answers the RGBA8 size queries and the RGBA8 read
    /// pair, and reads back the stored bytes of the words its own layer was
    /// uploaded as through one glTexSubImage3D covering all layers.
    #[test]
    fn texstorage_fallback_texstorage3d_array_is_rgba8_storage() {
        let _g = context();
        let layers = 3;
        let tex = rgb10a2_storage3d_array(layers, 2);
        assert_eq!(err(), GL_NO_ERROR, "glTexStorage3D(GL_TEXTURE_2D_ARRAY, GL_RGB10_A2)");
        assert_eq!(texparam(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_IMMUTABLE_FORMAT), 1, "immutable");
        assert_eq!(texparam(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_IMMUTABLE_LEVELS), 2, "the levels given");
        let mut words = Vec::new();
        for i in 0..layers as u32 {
            words.extend(vec![sentinel(i); 16]);
        }
        upload_words_array(layers, &words);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D into immutable storage");
        // SAFETY: room for one name.
        let fb = unsafe {
            let mut fb = 0;
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            fb
        };
        for layer in 0..layers {
            attach_layer(tex, layer);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "layer {layer} of immutable storage");
            assert_eq!(
                (
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE),
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE),
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE),
                    fb_bits(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE),
                ),
                (8, 8, 8, 8),
                "layer {layer}: the RGBA8 store's sizes"
            );
            assert_eq!(fb_bits(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), GL_UNSIGNED_NORMALIZED as GLint);
            assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE), "layer {layer}: the RGBA8 read pair");
            assert_eq!(err(), GL_NO_ERROR, "the attachment queries of layer {layer}");
            let want = stored(sentinel(layer as u32));
            let px = read(4, 4);
            assert_eq!(err(), GL_NO_ERROR, "glReadPixels of layer {layer}");
            assert!(
                px.as_chunks::<4>().0.iter().all(|p| *p == want),
                "layer {layer}: {:?} want {want:?}",
                px.as_chunks::<4>().0[0]
            );
        }
        release(&[fb], &[tex]);
    }

    /// One NEAREST glBlitFramebuffer from the immutable texture attachment
    /// (level 0, [`GRID`] words uploaded) into an RGBA8 texture copies the
    /// RGBA8 bytes the source holds, unchanged — as tests/es3_rgb10a2_fallback.rs
    /// pins for a glTexImage texture: both arms are texture attachments and
    /// the fallback class's image is in `attachment_image`.
    #[test]
    fn texstorage_fallback_blit_nearest_copies_the_bytes() {
        let _g = context();
        let (w, h) = (4, 4);
        let words = grid_words();
        let want = want_bytes(&WANTED_L0);

        let src = rgb10a2_storage2d(w, h, 3);
        assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D(GL_RGB10_A2)");
        upload_words_2d(w, h, &words);
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D");
        let s_fb = tex_framebuffer(src, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(read(w, h), want, "the source holds the RGBA8 bytes");
        assert_eq!(err(), GL_NO_ERROR);

        let dst = rgba8_texture(w, h);
        assert_eq!(err(), GL_NO_ERROR);
        let d_fb = tex_framebuffer(dst, 0);
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
        assert_eq!(read(w, h), want, "the RGBA8 destination holds the same bytes");
        assert_eq!(err(), GL_NO_ERROR);
        release(&[s_fb, d_fb], &[src, dst]);
    }
}
