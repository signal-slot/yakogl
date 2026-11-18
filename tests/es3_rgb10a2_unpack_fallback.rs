// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 uploads with the unpack parameters and from a pixel unpack
//! buffer where the build has no class for the format (plan #370 item
//! unpack-fallback-pin; the class build's rows are
//! tests/es3_rgb10a2_unpack2d_v3d.rs and tests/es3_rgb10a2_unpack3d_v3d.rs).
//!
//! On the V3D 4.2 build GL_RGB10_A2 is its own texture class and render
//! surface (`GLES_TEXFMT_RGB10A2` / `GLES_SURF_RGB10_A2`, the 10/10/10/2 word:
//! src/gl/framebuffer.rs `rgb10a2_surface`, docs/notes/v3d-rgb10a2.md); there
//! the word is stored and the unpack state moves whole words around it.
//! Everywhere else a GL_RGB10_A2 texture is in the RGBA8 class
//! (es3_formats' fallback): the PowerVR and GC7000 builds and every
//! `report-es2` build. This file holds what an upload with the ES 3.0 unpack
//! state answers there today, so that the class build's word rules cannot
//! reach them (sibling of tests/es3_rgb10a2_texclear_fallback.rs,
//! tests/es3_rgb10a2_layers_fallback.rs and tests/es3_rgb10a2_copytex_fallback.rs,
//! same stand-in OSAL and back end, no GPU):
//!
//! - GL_UNPACK_ROW_LENGTH, GL_UNPACK_SKIP_PIXELS, GL_UNPACK_SKIP_ROWS,
//!   GL_UNPACK_IMAGE_HEIGHT and GL_UNPACK_SKIP_IMAGES move the source texels
//!   of a GL_RGB10_A2 upload exactly as ES 3.0 3.7.4 lays them out (the row
//!   pitch GL_UNPACK_ROW_LENGTH texels wide or the upload's width, rounded up
//!   to GL_UNPACK_ALIGNMENT; src/gl/texture.rs `src_row_bytes`,
//!   `src_slice_bytes`, `unpack_skip_bytes`), and every texel that reaches
//!   the storage does so as the RGBA8 bytes the generic path makes of the
//!   10/10/10/2 word: the field decoded to a float (c / 1023, alpha c / 3)
//!   and encoded as (v * 255 + 0.5) truncated (es3_formats' `decode` /
//!   `encode`, `GLES_TEXFMT_RGBA8`) - the same bytes a plain upload of the
//!   word stores (texclear_fallback), no word and no field rules;
//! - GL_UNPACK_ALIGNMENT is a row *padding* rule for this pair, not a
//!   packing one: a GL_RGBA / GL_UNSIGNED_INT_2_10_10_10_REV row is whole
//!   4-byte words, so alignments 1, 2 and 4 lay out the same bytes, while
//!   alignment 8 rounds a 12-byte 3-word row up to 16 and skips the padding
//!   word (both pinned);
//! - with GL_PIXEL_UNPACK_BUFFER bound the `pixels` argument is an offset
//!   into the buffer (a multiple of the 4-byte datum), the skips count from
//!   that offset, and an offset that is not word-aligned or a read past the
//!   buffer's end is GL_INVALID_OPERATION with the texture untouched
//!   (src/gl/texture.rs `unpack_source`, src/gl/buffer.rs
//!   `pixel_buffer_resolve`);
//! - all of it on GL_TEXTURE_3D and GL_TEXTURE_2D_ARRAY as well: the slice
//!   pitch GL_UNPACK_IMAGE_HEIGHT rows moves whole RGBA8-stored texels
//!   between slices (as the layered attachments of
//!   tests/es3_rgb10a2_layers_fallback.rs read them).
//!
//! An ES 2.0-reporting build cannot allocate GL_RGB10_A2 at all (ES 2.0 names
//! no such format), so no upload of it exists there; worse, the unpack state
//! itself is ES 3.0: the five GL_UNPACK_* names are refused GL_INVALID_ENUM
//! (tests/es2_error_rules_state.rs row 19), GL_PIXEL_UNPACK_BUFFER is no
//! buffer target and GL_TEXTURE_3D no texture target. The rows of that cfg
//! pin the exact error each entry gives today.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_rgb10a2_unpack_fallback
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test es3_rgb10a2_unpack_fallback
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es3_rgb10a2_unpack_fallback
//! ```
//!
//! Compiled only where the class falls back: the V3D build without
//! `report-es2` stores the class and runs zero tests of this file (its answers
//! are tests/es3_rgb10a2_unpack2d_v3d.rs and tests/es3_rgb10a2_unpack3d_v3d.rs).

#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "report-es2")))]
// the ES 2.0 report runs the refusal pins only; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
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
/// No draw reaches the stand-in GPU in this file (no clear, no draw call).
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
    name: c"es3-rgb10a2-unpack-fallback".as_ptr() as *const c_char,
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
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A 10/10/10/2 word (R in bits 0..9, G 10..19, B 20..29, A 30..31).
const fn word(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

/// The word of source texel `i`: every field a byte no other tag's and no
/// sentinel's rounds to, so a texel placed at the wrong spot shows.
fn tag(i: usize) -> u32 {
    let i = i as u32;
    word(8 * i + 3, 1023 - 8 * i, 300 + 11 * i, i % 4)
}

/// `n` tag words, texel `i` tagged `i` (`tag(0)`, `tag(1)`, ...).
fn tags(n: usize) -> Vec<u32> {
    (0..n).map(tag).collect()
}

/// The word a texture is pre-filled with before a partial upload.
const SENTINEL: u32 = word(7, 70, 700, 2);

/// A field of `max` as the RGBA8 byte the fallback stores:
/// trunc(c / max * 255 + 0.5), alpha over 3 (es3_formats' decode to floats
/// and the `GLES_TEXFMT_RGBA8` encode).
fn byte(c: u32, max: u32) -> u8 {
    (c as f32 / max as f32 * 255.0 + 0.5) as u8
}

/// The RGBA8 bytes the fallback stores for a word.
fn stored(w: u32) -> [u8; 4] {
    [byte(w & 0x3ff, 1023), byte(w >> 10 & 0x3ff, 1023), byte(w >> 20 & 0x3ff, 1023), byte(w >> 30, 3)]
}

/// Words as the RGBA8 bytes the fallback stores them in, read order.
fn stored_bytes(ws: &[u32]) -> Vec<u8> {
    ws.iter().flat_map(|w| stored(*w)).collect()
}

/// The five ES 3.0 unpack parameters and the alignment back to their
/// initial values (the one context is shared by every test).
fn unpack_reset() {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, 0);
        glPixelStorei(GL_UNPACK_SKIP_IMAGES, 0);
        glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "resetting the unpack state");
}

/// The unpack state of one upload.
#[derive(Clone, Copy)]
struct Unpack {
    row_len: GLint,
    skip_px: GLint,
    skip_rows: GLint,
    skip_images: GLint,
    image_height: GLint,
    align: GLint,
}

const DEFAULTS: Unpack = Unpack { row_len: 0, skip_px: 0, skip_rows: 0, skip_images: 0, image_height: 0, align: 4 };

fn set_unpack(u: &Unpack) {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_UNPACK_ROW_LENGTH, u.row_len);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, u.skip_px);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, u.skip_rows);
        glPixelStorei(GL_UNPACK_SKIP_IMAGES, u.skip_images);
        glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, u.image_height);
        glPixelStorei(GL_UNPACK_ALIGNMENT, u.align);
    }
    assert_eq!(err(), GL_NO_ERROR, "glPixelStorei");
}

/* ---- ES 2.0 report: the unpack state, the buffer and the 3D are refused ------ */

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// ES 2.0 3.7.4 / 6.1.11 name no ES 3.0 unpack parameter: each of the
    /// five is GL_INVALID_ENUM and stores nothing (as
    /// tests/es2_error_rules_state.rs row 19 holds for their names), so no
    /// upload of any format - least of all GL_RGB10_A2, which no image of an
    /// ES 2.0 report can allocate (tests/es3_rgb10a2_texclear_fallback.rs) -
    /// can carry unpack state there.
    #[test]
    fn unpack_fallback_es2_unpack_parameters_are_refused() {
        let _g = context();
        for pname in [
            GL_UNPACK_ROW_LENGTH,
            GL_UNPACK_SKIP_PIXELS,
            GL_UNPACK_SKIP_ROWS,
            GL_UNPACK_SKIP_IMAGES,
            GL_UNPACK_IMAGE_HEIGHT,
        ] {
            // SAFETY: plain GL call.
            unsafe { glPixelStorei(pname, 2) };
            assert_eq!(err(), GL_INVALID_ENUM, "glPixelStorei({pname:#x})");
        }
    }

    /// ES 2.0 2.9 names no GL_PIXEL_UNPACK_BUFFER: the bind and the store are
    /// GL_INVALID_ENUM, so a GL_RGB10_A2 upload can never read from a pixel
    /// unpack buffer on an ES 2.0 report.
    #[test]
    fn unpack_fallback_es2_pixel_unpack_buffer_is_no_target() {
        let _g = context();
        let mut b = 0;
        // SAFETY: room for one name.
        unsafe { glGenBuffers(1, &mut b) };
        assert_eq!(err(), GL_NO_ERROR, "glGenBuffers");
        let src = tags(4);
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, b);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glBindBuffer(GL_PIXEL_UNPACK_BUFFER)");
        // SAFETY: the source holds four words.
        unsafe {
            glBufferData(GL_PIXEL_UNPACK_BUFFER, 16, src.as_ptr() as *const c_void, GL_STATIC_DRAW);
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glBufferData(GL_PIXEL_UNPACK_BUFFER)");
        // SAFETY: a name this test made.
        unsafe { glDeleteBuffers(1, &b) };
    }

    /// ES 2.0 3.7.1 names no 3D or 2D-array texture target and no third
    /// image dimension, so the slice unpack state of a GL_RGB10_A2 upload is
    /// unreachable there. The errors asserted are what the build gives today:
    /// both 3D entry points refuse GL_TEXTURE_3D with GL_INVALID_OPERATION
    /// (the ES 3.0 target itself is refused - an ES 2.0 internal format does
    /// not make it legal), while glTexSubImage3D on an ES 2.0 target is
    /// refused GL_INVALID_ENUM on the target mismatch first, before the
    /// missing texture storage is asked about.
    #[test]
    fn unpack_fallback_es2_three_dimensional_uploads_are_refused() {
        let _g = context();
        let mut tex = 0;
        // SAFETY: room for one name; the null pixels allocate only.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA as GLint, 2, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "glTexImage3D(GL_TEXTURE_3D)");
        // SAFETY: plain GL call.
        unsafe {
            glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 2, 2, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "glTexSubImage3D(GL_TEXTURE_3D)");
        // SAFETY: plain GL call.
        unsafe {
            glTexSubImage3D(GL_TEXTURE_2D, 0, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glTexSubImage3D(GL_TEXTURE_2D)");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    /// The 2D entry stays open but the type does not:
    /// GL_UNSIGNED_INT_2_10_10_10_REV is no ES 2.0 pixel type, so even the
    /// partial upload of the pair this file pins is refused there - on an
    /// ordinary ES 2.0 RGBA texture (and GL_RGB10_A2 itself is no internal
    /// format, as tests/es3_rgb10a2_texclear_fallback.rs pins).
    #[test]
    fn unpack_fallback_es2_upload_pair_is_refused() {
        let _g = context();
        let mut tex = 0;
        let src = tags(4);
        // SAFETY: room for one name; the texture is an ES 2.0 RGBA one.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        assert_eq!(err(), GL_NO_ERROR, "ES 2.0 RGBA8 storage");
        // SAFETY: the source holds four words.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                2,
                2,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                src.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_INVALID_ENUM, "glTexSubImage2D(.., GL_UNSIGNED_INT_2_10_10_10_REV)");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
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
    fn unpack_fallback_build_has_no_class() {
        assert!(!backend::rt_format_drawable(backend::GLES_SURF_RGB10_A2));
    }

    /// A new GL_RGB10_A2 texture of `w` x `h`, every texel pre-filled with
    /// [`SENTINEL`], so that a partial upload leaves its untouched texels
    /// recognisable.
    fn filled2(w: GLsizei, h: GLsizei) -> GLuint {
        let px = vec![SENTINEL; (w * h) as usize];
        let mut t = 0;
        // SAFETY: room for one name; the pixels hold w x h words.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGB10_A2 as GLint,
                w,
                h,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                px.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "pre-fill {w}x{h}");
        t
    }

    /// Read the whole level 0 of the bound 2D texture through a new
    /// framebuffer (left attached) as GL_RGBA / GL_UNSIGNED_BYTE.
    fn read2(tex: GLuint, w: GLsizei, h: GLsizei) -> Vec<u8> {
        let mut fb = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(err(), GL_NO_ERROR, "attach {tex}");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let mut px = vec![0x77u8; (w * h * 4) as usize];
        // SAFETY: room for w x h RGBA8 pixels.
        unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels {w}x{h}");
        px
    }

    /// Slice (layer) `i` of the texture, read the same way, `w` x `h`.
    fn read_layer(tex: GLuint, i: GLint, w: GLsizei, h: GLsizei) -> Vec<u8> {
        let mut fb = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, i);
            assert_eq!(err(), GL_NO_ERROR, "attach layer {i}");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let mut px = vec![0x77u8; (w * h * 4) as usize];
        // SAFETY: room for w x h RGBA8 pixels.
        unsafe { glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels layer {i}");
        px
    }

    /// An 18-word tag source; the rows below take their texels from it.
    const SRC18: usize = 18;

    /// glTexSubImage2D with GL_UNPACK_ROW_LENGTH, GL_UNPACK_SKIP_PIXELS and
    /// GL_UNPACK_SKIP_ROWS: the rectangle (1, 1) 2 x 1 of a 4 x 2 texture
    /// taken from an 18-word source whose rows are
    /// GL_UNPACK_ROW_LENGTH = 6 texels wide, starting one skipped row and one
    /// skipped pixel in - so its two texels are source words 7 and 8, laid
    /// out by the row pitch alone. They store as the RGBA8 bytes of those
    /// words; the other six texels keep their sentinel's bytes.
    #[test]
    fn unpack_fallback_texsubimage2d_row_length_and_skips() {
        let _g = context();
        unpack_reset();
        // pre-fill under the initial state, then move to the row length and
        // the skips: the upload, not the pre-fill, must carry them.
        let tex = filled2(4, 2);
        set_unpack(&Unpack { row_len: 6, skip_px: 1, skip_rows: 1, ..DEFAULTS });
        let src = tags(SRC18);
        // SAFETY: the source holds 18 words.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                1,
                1,
                2,
                1,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                src.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D with row length and skips");
        let mut want = vec![SENTINEL; 8];
        want[5] = tag(7); // (1, 1)
        want[6] = tag(8); // (2, 1)
        assert_eq!(read2(tex, 4, 2), stored_bytes(&want));
        unpack_reset();
        // SAFETY: names this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    /// GL_UNPACK_ALIGNMENT never pads this pair below 8: a 3-wide row of
    /// 4-byte words needs no padding at alignment 1, 2 or 4, and a
    /// GL_UNPACK_ROW_LENGTH = 5 pitch needs none at alignment 1 (its texels
    /// 5, 6 and 7 of the source land in row 1). Alignment 8 with no row
    /// length pads the 12-byte row up to 16 and skips the fourth word.
    #[test]
    fn unpack_fallback_alignment_pads_only_at_eight_and_row_length_sets_the_pitch() {
        let _g = context();
        unpack_reset();
        let src = tags(SRC18);
        // Alignments 1, 2 and 4, no row length: the identical tight 3 x 2
        // upload.
        for align in [1, 2, 4] {
            let tex = filled2(3, 2);
            set_unpack(&Unpack { align, ..DEFAULTS });
            // SAFETY: the source holds the 6 words read.
            unsafe {
                glTexSubImage2D(
                    GL_TEXTURE_2D,
                    0,
                    0,
                    0,
                    3,
                    2,
                    GL_RGBA,
                    GL_UNSIGNED_INT_2_10_10_10_REV,
                    src.as_ptr() as *const c_void,
                );
            }
            assert_eq!(err(), GL_NO_ERROR, "alignment {align}");
            assert_eq!(read2(tex, 3, 2), stored_bytes(&tags(6)), "alignment {align}");
            // SAFETY: a name this test made.
            unsafe { glDeleteTextures(1, &tex) };
        }
        // Row length 5 at alignment 1: the second row is source words 5..8.
        let tex = filled2(3, 2);
        set_unpack(&Unpack { row_len: 5, align: 1, ..DEFAULTS });
        // SAFETY: the source holds 8 of the 18 words.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                3,
                2,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                src.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "row length 5, alignment 1");
        let want = [tag(0), tag(1), tag(2), tag(5), tag(6), tag(7)];
        assert_eq!(read2(tex, 3, 2), stored_bytes(&want), "row length 5");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
        // Alignment 8 with no row length: a 3-word row is 12 bytes and the
        // row pitch rounds up to 16, so the fourth word of each source row
        // is padding and is not stored.
        let tex = filled2(3, 2);
        set_unpack(&Unpack { align: 8, ..DEFAULTS });
        // SAFETY: the source holds 8 of the 18 words.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                3,
                2,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                src.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "alignment 8");
        let want = [tag(0), tag(1), tag(2), tag(4), tag(5), tag(6)];
        assert_eq!(read2(tex, 3, 2), stored_bytes(&want), "alignment 8 pads the row to 16 bytes");
        unpack_reset();
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    /// A GL_PIXEL_UNPACK_BUFFER as the source: `pixels` is an offset into it,
    /// a multiple of the 4-byte datum; GL_UNPACK_SKIP_PIXELS counts from that
    /// offset; and the two refusals - an offset that is not word aligned and
    /// an upload that would read past the buffer's end - are
    /// GL_INVALID_OPERATION with the texture untouched. The texels that do
    /// upload are the RGBA8 bytes of the words.
    #[test]
    fn unpack_fallback_uploads_from_pixel_unpack_buffer() {
        let _g = context();
        unpack_reset();
        let src = tags(SRC18);
        let mut buf = 0;
        // (a) the 4 x 2 texture is pre-filled before the buffer is bound:
        // with it bound, the client pointer would be a buffer offset.
        let tex = filled2(4, 2);
        // SAFETY: room for one name; the buffer holds the 18 words.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
            let bytes = (SRC18 * 4) as isize;
            glBufferData(GL_PIXEL_UNPACK_BUFFER, bytes, src.as_ptr() as *const c_void, GL_STATIC_DRAW);
            assert_eq!(err(), GL_NO_ERROR, "glBufferData");
        }
        // (a) glTexSubImage2D at offset 16 bytes (word 4): a 2 x 2 at (0, 0)
        // of the texture takes source words 4, 5, 6, 7.
        // SAFETY: plain GL call; the offset is a word multiple.
        unsafe {
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 16 as *const c_void);
        }
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage2D from the buffer");
        let mut want = vec![SENTINEL; 8];
        want[0] = tag(4); // (0, 0)
        want[1] = tag(5); // (1, 0)
        want[4] = tag(6); // (0, 1)
        want[5] = tag(7); // (1, 1)
        assert_eq!(read2(tex, 4, 2), stored_bytes(&want), "offset 16");
        // (b) glTexImage2D of a new 3 x 1 texture at offset 0 with
        // GL_UNPACK_SKIP_PIXELS 2: the first texel read is word 2, the row
        // is words 2, 3, 4.
        set_unpack(&Unpack { skip_px: 2, ..DEFAULTS });
        let mut tex2 = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut tex2);
            glBindTexture(GL_TEXTURE_2D, tex2);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGB10_A2 as GLint,
                3,
                1,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                core::ptr::null(),
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "glTexImage2D from the buffer with a skip");
        assert_eq!(read2(tex2, 3, 1), stored_bytes(&[tag(2), tag(3), tag(4)]), "skip 2 from offset 0");
        // (c) an offset that is not a multiple of the 4-byte datum is
        // GL_INVALID_OPERATION and changes nothing: tex (a) still holds (a)'s
        // bytes.
        // SAFETY: plain GL call.
        unsafe {
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 6 as *const c_void);
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "offset 6 (not word aligned)");
        assert_eq!(read2(tex, 4, 2), stored_bytes(&want), "the refused upload changed nothing");
        // (d) an upload that reads past the buffer's end is the same
        // refusal, the skip counted in: 5 x 4 = 20 words plus 2 skipped, so
        // 88 bytes from a 72-byte buffer.
        let mut tex3 = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut tex3);
            glBindTexture(GL_TEXTURE_2D, tex3);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGB10_A2 as GLint,
                5,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                core::ptr::null(),
            );
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "88 bytes read from a 72-byte buffer");
        unpack_reset();
        // SAFETY: names this test made.
        unsafe {
            glDeleteTextures(1, &tex);
            glDeleteTextures(1, &tex2);
            glDeleteTextures(1, &tex3);
            glDeleteBuffers(1, &buf);
        }
    }

    /// glTexImage3D of a GL_TEXTURE_3D volume with
    /// GL_UNPACK_IMAGE_HEIGHT = 4 (the upload is 2 rows high, so every slice
    /// occupies 4 rows of source words and the third of each is skipped):
    /// slice z, row y, texel x is source word 8z + 2y + x, and each slice
    /// attaches and reads back as the RGBA8 bytes of those words.
    #[test]
    fn unpack_fallback_teximage3d_image_height() {
        let _g = context();
        unpack_reset();
        set_unpack(&Unpack { image_height: 4, ..DEFAULTS });
        // Texel (x, y, z) of the layout the source must hold.
        let at = |z: usize, y: usize, x: usize| 8 * z + 2 * y + x;
        let src = tags(24);
        let mut tex = 0;
        // SAFETY: room for one name; the source holds the 24 words the
        // layout reaches.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_3D, tex);
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_RGB10_A2 as GLint,
                2,
                2,
                3,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                src.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D with image height 4");
        for z in 0..3 {
            let want = [tag(at(z, 0, 0)), tag(at(z, 0, 1)), tag(at(z, 1, 0)), tag(at(z, 1, 1))];
            assert_eq!(read_layer(tex, z as GLint, 2, 2), stored_bytes(&want), "slice {z}");
        }
        unpack_reset();
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    /// glTexSubImage3D into slice 1 of a pre-filled volume with the full 3D
    /// unpack state: GL_UNPACK_ROW_LENGTH 4, GL_UNPACK_IMAGE_HEIGHT 3,
    /// GL_UNPACK_SKIP_IMAGES 2, GL_UNPACK_SKIP_ROWS 1 and
    /// GL_UNPACK_SKIP_PIXELS 1. The slice pitch is 3 rows of 4 words, so the
    /// first texel read is source word 2 * 12 + 1 * 4 + 1 = 29; slice 0 and
    /// slice 2 keep their sentinel.
    #[test]
    fn unpack_fallback_texsubimage3d_skip_images_and_rows() {
        let _g = context();
        unpack_reset();
        let pre = [SENTINEL; 12];
        let mut tex = 0;
        // SAFETY: room for one name; the pre-fill holds 12 words.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_3D, tex);
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_RGB10_A2 as GLint,
                2,
                2,
                3,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                pre.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "pre-filled volume");
        set_unpack(&Unpack { row_len: 4, skip_px: 1, skip_rows: 1, skip_images: 2, image_height: 3, ..DEFAULTS });
        let src = tags(36);
        // SAFETY: the source holds the 36 words the layout reaches.
        unsafe {
            glTexSubImage3D(
                GL_TEXTURE_3D,
                0,
                0,
                0,
                1,
                2,
                2,
                1,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                src.as_ptr() as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "glTexSubImage3D with the full 3D unpack state");
        let at = |y: usize, x: usize| 29 + y * 4 + x;
        let sentry = [SENTINEL; 4];
        assert_eq!(read_layer(tex, 0, 2, 2), stored_bytes(&sentry), "slice 0 untouched");
        let want = [tag(at(0, 0)), tag(at(0, 1)), tag(at(1, 0)), tag(at(1, 1))];
        assert_eq!(read_layer(tex, 1, 2, 2), stored_bytes(&want), "slice 1");
        assert_eq!(read_layer(tex, 2, 2, 2), stored_bytes(&sentry), "slice 2 untouched");
        unpack_reset();
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    /// A GL_TEXTURE_2D_ARRAY uploaded by glTexImage3D from a
    /// GL_PIXEL_UNPACK_BUFFER with GL_UNPACK_IMAGE_HEIGHT and
    /// GL_UNPACK_SKIP_IMAGES together: an 8-byte offset into the buffer, a
    /// 3-row slice pitch (one row more than the 2 rows the layers use) and
    /// one skipped leading slice: the upload starts at buffer word 2 + 6
    /// (skip) = 8, so layer 0 takes words 8..11 and layer 1 words 14..17.
    #[test]
    fn unpack_fallback_teximage3d_array_from_buffer_with_image_height() {
        let _g = context();
        unpack_reset();
        set_unpack(&Unpack { skip_images: 1, image_height: 3, ..DEFAULTS });
        let src = tags(SRC18);
        let (mut buf, mut tex) = (0, 0);
        // SAFETY: room for one name each; the buffer holds the 18 words.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, (SRC18 * 4) as isize, src.as_ptr() as *const c_void, GL_STATIC_DRAW);
            assert_eq!(err(), GL_NO_ERROR, "glBufferData");
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                GL_RGB10_A2 as GLint,
                2,
                2,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_INT_2_10_10_10_REV,
                8 as *const c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "glTexImage3D of an array from the buffer");
        let at = |k: usize, y: usize, x: usize| 8 + 6 * k + 2 * y + x;
        for k in 0..2 {
            let want = [tag(at(k, 0, 0)), tag(at(k, 0, 1)), tag(at(k, 1, 0)), tag(at(k, 1, 1))];
            assert_eq!(read_layer(tex, k as GLint, 2, 2), stored_bytes(&want), "layer {k}");
        }
        unpack_reset();
        // SAFETY: names this test made.
        unsafe {
            glDeleteTextures(1, &tex);
            glDeleteBuffers(1, &buf);
        }
    }
}
