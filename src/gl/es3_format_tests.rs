// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 sized texture formats of `es3_formats` through the GL entry
//! points (glTexImage2D/3D, glTexStorage2D/3D, glTexSubImage2D/3D, the
//! copies, completeness), on the mock back end. The pure tables and
//! conversions are tested by tests/es3_sized_formats.rs; this checks the
//! wiring in `texture.rs`, and that under `report-es2` none of it exists.

use core::ffi::c_void;
use core::sync::atomic::Ordering;
use std::string::String;
use std::vec;
use std::vec::Vec;

use super::consts::*;
use super::draw::texture_bind_2d;
use super::es3_formats::{self, FORMATS};
use super::texture::*;
use super::types::*;
use crate::backend::{
    gles_texfmt_bit, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8, TEST_EXTRA_SAMPLED, TEST_HIDDEN_SAMPLED,
};
use crate::testutil::TestGl;

/// Every texel format of the new classes (ids 11 and up), as the mask a back
/// end that samples all of them would state.
const ALL_NEW: u32 = (!0 << 11) | (1 << GLES_TEXFMT_RGBA16F);

/// Holds `TEST_EXTRA_SAMPLED` / `TEST_HIDDEN_SAMPLED` for a test and puts
/// them back.
struct Sampled;
impl Sampled {
    fn also(mask: u32) -> Sampled {
        TEST_EXTRA_SAMPLED.store(mask, Ordering::Relaxed);
        Sampled
    }
    /// A back end that samples none of the new classes (ids 11 and up),
    /// whatever this build's samples.
    fn none_new() -> Sampled {
        TEST_HIDDEN_SAMPLED.store(!0 << 11, Ordering::Relaxed);
        Sampled
    }
}
impl Drop for Sampled {
    fn drop(&mut self) {
        TEST_EXTRA_SAMPLED.store(0, Ordering::Relaxed);
        TEST_HIDDEN_SAMPLED.store(0, Ordering::Relaxed);
    }
}

fn new_tex(target: GLenum) -> GLuint {
    let mut name: GLuint = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

fn bytes_of(e: &es3_formats::Es3Format, w: usize, h: usize) -> Vec<u8> {
    let n = w * h * es3_formats::texel_bytes(e.base, e.types[0]);
    (0..n).map(|i| (i * 7 + 3) as u8).collect()
}

/// The first `texels` texels of texture `name`'s storage, and its texfmt.
fn first_row(gl: &TestGl, name: GLuint, texels: usize) -> (Vec<u8>, u8) {
    let t = &gl.ctx().objs().textures[name as usize];
    let (m, tf, bpp) = (t.mem, t.texfmt, t.bpp as usize);
    (gl.gpu_bytes(m.gpu, texels * bpp), tf)
}

fn tex_image_2d(internalformat: GLenum, w: GLsizei, h: GLsizei, format: GLenum, type_: GLenum, data: &[u8]) {
    let p = if data.is_empty() { core::ptr::null() } else { data.as_ptr() as *const c_void };
    // SAFETY: `data` holds the image for (format, type), or is null.
    unsafe { glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, w, h, 0, format, type_, p) };
}

/// ES 2.0 reporting: no format of the table exists, as before.
#[test]
fn report_es2_knows_none_of_them() {
    if !crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    new_tex(GL_TEXTURE_2D);
    for e in FORMATS.iter() {
        assert!(es3_formats::find(e.internalformat).is_none());
        let data = bytes_of(e, 2, 2);
        tex_image_2d(e.internalformat, 2, 2, e.base, e.types[0], &data);
        let err = gl.error();
        /* an integer base or a type ES 2.0 lacks is GL_INVALID_ENUM (the
         * types of ES 2.0 3.6.2 Table 3.2 and of the claimed
         * GL_OES_depth_texture / GL_OES_packed_depth_stencil are the ones
         * listed here); a pair of known enums ES 2.0 lacks is
         * GL_INVALID_OPERATION */
        let es2_type = matches!(
            e.types[0],
            GL_UNSIGNED_BYTE
                | GL_UNSIGNED_SHORT_5_6_5
                | GL_UNSIGNED_SHORT_4_4_4_4
                | GL_UNSIGNED_SHORT_5_5_5_1
                | GL_UNSIGNED_SHORT
                | GL_UNSIGNED_INT
                | GL_UNSIGNED_INT_24_8
        );
        let want =
            if es3_formats::is_integer_format(e.base) || !es2_type { GL_INVALID_ENUM } else { GL_INVALID_OPERATION };
        assert_eq!(err, want, "{} under ES 2.0 reporting", e.name);
    }
}

/// glTexImage2D takes every row of the table in its own (format, type), the
/// storage is the first class the back end samples, and the texels are the
/// converted ones.
#[test]
fn every_format_uploads_into_a_sampled_class() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    for e in FORMATS.iter() {
        let name = new_tex(GL_TEXTURE_2D);
        let data = bytes_of(e, 2, 2);
        tex_image_2d(e.internalformat, 2, 2, e.base, e.types[0], &data);
        assert_eq!(gl.error(), GL_NO_ERROR, "{} glTexImage2D", e.name);
        let (row, tf) = first_row(&gl, name, 2);
        assert_eq!(Some(tf), e.storage_here(), "{} storage", e.name);
        let mut want = vec![0u8; es3_formats::storage_bytes(tf) * 2];
        let sb = es3_formats::texel_bytes(e.base, e.types[0]);
        assert!(es3_formats::convert_row(e, tf, &data[..2 * sb], e.base, e.types[0], 2, &mut want));
        assert_eq!(row, want, "{} first row", e.name);
        // SAFETY: plain GL call.
        unsafe { glDeleteTextures(1, &name) };
    }
}

/// An integer texture's integers survive: R8I -5, R16UI 0xBEEF, RGBA32I -1.
#[test]
fn integer_texels_are_stored_as_integers() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    let name = new_tex(GL_TEXTURE_2D);
    tex_image_2d(GL_R8I, 1, 1, GL_RED_INTEGER, GL_BYTE, &[0xFB]);
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_eq!(first_row(&gl, name, 1).0, vec![0xFB, 0, 0, 1]);
    tex_image_2d(GL_R16UI, 1, 1, GL_RED_INTEGER, GL_UNSIGNED_SHORT, &0xBEEFu16.to_ne_bytes());
    assert_eq!(gl.error(), GL_NO_ERROR);
    let mut want = Vec::new();
    for v in [0xBEEFu16, 0, 0, 1] {
        want.extend(v.to_ne_bytes());
    }
    assert_eq!(first_row(&gl, name, 1).0, want);
    let m1: Vec<u8> = (0..16).map(|_| 0xFF).collect();
    tex_image_2d(GL_RGBA32I, 1, 1, GL_RGBA_INTEGER, GL_INT, &m1);
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_eq!(first_row(&gl, name, 1).0, m1);
}

/// Table 3.2 pairs: an internal format takes its own and refuses the rest.
#[test]
fn a_pair_of_another_row_is_invalid_operation() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    new_tex(GL_TEXTURE_2D);
    let z = [0u8; 64];
    tex_image_2d(GL_R8UI, 1, 1, GL_RED_INTEGER, GL_BYTE, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION, "R8UI with BYTE");
    tex_image_2d(GL_R8UI, 1, 1, GL_RED, GL_UNSIGNED_BYTE, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION, "R8UI with RED");
    tex_image_2d(GL_RGBA8, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION, "RGBA8 with RGBA_INTEGER");
    tex_image_2d(GL_RGBA8, 1, 1, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION, "RGBA8 with the 10-bit word");
    tex_image_2d(GL_R11F_G11F_B10F, 1, 1, GL_RGB, GL_UNSIGNED_BYTE, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION, "R11F with bytes");
    tex_image_2d(GL_R11F_G11F_B10F, 1, 1, GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION, "R11F with the shared exponent word");
    /* and the pairs it does take */
    tex_image_2d(GL_R11F_G11F_B10F, 1, 1, GL_RGB, GL_FLOAT, &z);
    assert_eq!(gl.error(), GL_NO_ERROR);
    tex_image_2d(GL_RGB9_E5, 1, 1, GL_RGB, GL_HALF_FLOAT, &z);
    assert_eq!(gl.error(), GL_NO_ERROR);
    /* a name that is no format at all stays what it was */
    tex_image_2d(0x1234, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &z);
    assert_eq!(gl.error(), GL_INVALID_VALUE);
}

/// A back end that samples no class of a format refuses it by name, with an
/// error and a log line, in every specification call; one that has a
/// fallback class stores the texels there.
#[test]
fn a_format_no_class_of_which_is_sampled_is_refused_by_name() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::none_new();
    new_tex(GL_TEXTURE_2D);
    let z = [0u8; 64];
    gl.logs();
    tex_image_2d(GL_RGBA16UI, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_SHORT, &z);
    assert_eq!(gl.error(), GL_INVALID_OPERATION);
    let logs: String = gl.logs().join("\n");
    assert!(logs.contains("GL_RGBA16UI is not supported"), "{logs}");
    // SAFETY: plain GL calls; no texels are read.
    unsafe {
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_R32I, 4, 4);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        let a = new_tex(GL_TEXTURE_2D_ARRAY);
        glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RG8UI, 4, 4, 2);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        let p = z.as_ptr() as *const c_void;
        glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RG8UI as GLint, 4, 4, 2, 0, GL_RG_INTEGER, GL_UNSIGNED_BYTE, p);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        glDeleteTextures(1, &a);
    }
    /* R11F falls back to halves wherever RGBA16F is sampled, which the PowerVR
     * and the GC7000 do and the V3D does not (yet) */
    let half = crate::backend::sampled_texfmts() & gles_texfmt_bit(GLES_TEXFMT_RGBA16F) != 0;
    let name = new_tex(GL_TEXTURE_2D);
    let one = [1.0f32, 0.5, 2.0].iter().flat_map(|f| f.to_ne_bytes()).collect::<Vec<u8>>();
    tex_image_2d(GL_R11F_G11F_B10F, 1, 1, GL_RGB, GL_FLOAT, &one);
    if half {
        assert_eq!(gl.error(), GL_NO_ERROR);
        let mut want = Vec::new();
        for v in [0x3C00u16, 0x3800, 0x4000, 0x3C00] {
            want.extend(v.to_ne_bytes());
        }
        assert_eq!(first_row(&gl, name, 1), (want, GLES_TEXFMT_RGBA16F));
    } else {
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }
    /* RGB10_A2 falls back to RGBA8 on every back end */
    let name = new_tex(GL_TEXTURE_2D);
    let word: u32 = 1023 | (512 << 10) | (3 << 30);
    tex_image_2d(GL_RGB10_A2, 1, 1, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, &word.to_ne_bytes());
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_eq!(first_row(&gl, name, 1), (vec![255, 128, 0, 255], GLES_TEXFMT_RGBA8));
}

/// glTexStorage2D/3D + glTexSubImage2D/3D: the sub-image converts into the
/// class chosen at specification, and takes only the format's own pairs.
#[test]
fn storage_and_sub_images_convert() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    let name = new_tex(GL_TEXTURE_2D);
    // SAFETY: plain GL calls; the source holds what each upload reads.
    unsafe {
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8_SNORM, 2, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let px: [i8; 4] = [127, -127, 0, 64];
        let p = px.as_ptr() as *const c_void;
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_BYTE, p);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(first_row(&gl, name, 1).0, vec![127, 0x81, 0, 64]);
        /* the wrong pair for the texture */
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        /* an integer pair into an ordinary RGBA8 texture */
        let rgba = new_tex(GL_TEXTURE_2D);
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 2, 2);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, p);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        glDeleteTextures(1, &rgba);

        let arr = new_tex(GL_TEXTURE_2D_ARRAY);
        glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RG16UI, 2, 2, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let v: Vec<u8> = [7u16, 9].iter().flat_map(|x| x.to_ne_bytes()).collect();
        let vp = v.as_ptr() as *const c_void;
        glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 1, 1, 1, 1, 1, 1, GL_RG_INTEGER, GL_UNSIGNED_SHORT, vp);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[arr as usize];
        let (m, pitch, bpp) = (t.mem, t.level_pitch(2) as usize, t.bpp as usize);
        let all = gl.gpu_bytes(m.gpu, m.size);
        /* layer 1, row 1, column 1 of a stack of 2-row layers */
        let o = ((2 + 1) * pitch + 1) * bpp;
        let mut want = Vec::new();
        for x in [7u16, 9, 0, 1] {
            want.extend(x.to_ne_bytes());
        }
        assert_eq!(&all[o..o + 8], &want[..]);
    }
}

/// ES 3.0 3.8.5: the component types of the colour buffer and the image
/// must agree. The default framebuffer is unsigned normalised, so a copy into
/// an integer, signed normalised or floating-point image is
/// GL_INVALID_OPERATION, and into RGB10_A2 (unsigned normalised) is fine.
#[test]
fn copies_follow_the_component_types() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    new_tex(GL_TEXTURE_2D);
    // SAFETY: plain GL calls.
    unsafe {
        for f in [
            GL_RGBA8UI,
            GL_R32I,
            GL_RGBA8_SNORM,
            GL_R8_SNORM,
            GL_RGB16F,
            GL_R11F_G11F_B10F,
            GL_RGB9_E5,
            GL_RG32F,
            GL_RGB32F,
        ] {
            glCopyTexImage2D(GL_TEXTURE_2D, 0, f, 0, 0, 2, 2, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "copy into 0x{f:x} from a fixed-point buffer");
        }
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGB10_A2, 0, 0, 2, 2, 0);
        assert_eq!(gl.error(), GL_NO_ERROR, "RGB10_A2");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, 2, 2, 0);
        assert_eq!(gl.error(), GL_NO_ERROR, "RGBA8");
        /* the sub-image copy into existing storage of each kind */
        for f in [GL_R32UI, GL_RGBA8_SNORM, GL_RGB16F] {
            let s = new_tex(GL_TEXTURE_2D);
            glTexStorage2D(GL_TEXTURE_2D, 1, f, 2, 2);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "sub-copy into 0x{f:x}");
            glDeleteTextures(1, &s);
        }
    }
}

/// ES 3.0 table 3.2: RGB5_A1 also unpacks UNSIGNED_INT_2_10_10_10_REV (its
/// alpha bit is set by 2 and 3), through the original RGBA8 store; no other
/// internal format of the original tables takes the word.
#[test]
fn rgb5_a1_takes_the_ten_bit_word() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    let name = new_tex(GL_TEXTURE_2D);
    /* r 1023, g 512, b 0, a 2 | r 0, g 1023, b 1023, a 1 */
    let w = [1023u32 | (512 << 10) | (2 << 30), (1023 << 10) | (1023 << 20) | (1 << 30)];
    let bytes: Vec<u8> = w.iter().flat_map(|v| v.to_ne_bytes()).collect();
    tex_image_2d(GL_RGB5_A1, 2, 1, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, &bytes);
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_eq!(first_row(&gl, name, 2), (vec![255, 128, 0, 255, 0, 255, 255, 0], GLES_TEXFMT_RGBA8));
    /* a sub-image of the word into it */
    // SAFETY: the source holds one texel.
    unsafe {
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 0, 1, 1, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, bytes.as_ptr() as _);
    }
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_eq!(first_row(&gl, name, 2).0[4..], [255, 128, 0, 255]);
    /* RGBA8 and RGBA4 do not take it */
    tex_image_2d(GL_RGBA8, 2, 1, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, &bytes);
    assert_eq!(gl.error(), GL_INVALID_OPERATION);
    tex_image_2d(GL_RGBA4, 2, 1, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, &bytes);
    assert_eq!(gl.error(), GL_INVALID_OPERATION);
}

/// ES 3.0 3.8.13: an integer texture is complete with NEAREST filtering
/// only; a float one of 16-bit texels filters.
#[test]
fn an_integer_texture_is_complete_only_with_nearest_filters() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    let name = new_tex(GL_TEXTURE_2D);
    let z = [0u8; 16];
    tex_image_2d(GL_RGBA8UI, 2, 2, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, &z);
    assert_eq!(gl.error(), GL_NO_ERROR);
    let incomplete = |gl: &TestGl, n: GLuint| texture_bind_2d(&gl.ctx().objs().textures[n as usize]).incomplete;
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(incomplete(&gl, name), 0, "NEAREST / NEAREST");
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint) };
    assert_eq!(incomplete(&gl, name), 1, "a LINEAR magnification filter");
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
    }
    assert_eq!(incomplete(&gl, name), 1, "a LINEAR minification filter");
    /* a float texture of 16-bit texels filters */
    let f = new_tex(GL_TEXTURE_2D);
    let h = [0u8; 24];
    tex_image_2d(GL_RGB16F, 2, 2, GL_RGB, GL_HALF_FLOAT, &h);
    assert_eq!(gl.error(), GL_NO_ERROR);
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint) };
    assert_eq!(incomplete(&gl, f), 0);
}

/// glGenerateMipmap of a format that is not renderable and filterable is
/// GL_INVALID_OPERATION (ES 3.0 3.8.14); RGB10_A2 stored as RGBA8 has one.
#[test]
fn generate_mipmap_follows_renderable_and_filterable() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(0);
    new_tex(GL_TEXTURE_2D);
    let z = [0u8; 64];
    tex_image_2d(GL_RGB10_A2, 2, 2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, &z);
    // SAFETY: plain GL calls.
    unsafe {
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
    new_tex(GL_TEXTURE_2D);
    tex_image_2d(GL_RGB9_E5, 2, 2, GL_RGB, GL_HALF_FLOAT, &z);
    if gl.error() == GL_NO_ERROR {
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }
}

/// A level of another ES 3.0 format than level 0 leaves the chain bad,
/// though both are stored in the same texel class.
#[test]
fn levels_of_another_es3_format_are_a_mismatch() {
    if crate::REPORT_ES2 {
        return;
    }
    let gl = TestGl::new(4, 4);
    let _s = Sampled::also(ALL_NEW);
    let name = new_tex(GL_TEXTURE_2D);
    let z = [0u8; 64];
    tex_image_2d(GL_RGBA8UI, 2, 2, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, &z);
    // SAFETY: the source holds one texel.
    unsafe {
        let p = z.as_ptr() as *const c_void;
        glTexImage2D(GL_TEXTURE_2D, 1, GL_RGBA8I as GLint, 1, 1, 0, GL_RGBA_INTEGER, GL_BYTE, p);
    }
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_ne!(gl.ctx().objs().textures[name as usize].level_bad & 2, 0);
}
