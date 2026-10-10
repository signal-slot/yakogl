// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Pixel buffer objects through the GL entry points on the mock back end:
//! uploads that read a bound PIXEL_UNPACK_BUFFER under the unpack state
//! (dEQP-GLES3 texture.specification teximage2d_pbo, texsubimage2d_pbo,
//! teximage3d_pbo, texsubimage3d_pbo, *_unpack_params, *_align).
//!
//! Each case is the dEQP case's sizes and pixel-store state; the texture it
//! makes is compared with one uploaded from a tight copy of the same texels
//! (alignment 1, no row length, no skips) taken out of the source by this
//! file's own reading of ES 3.0 3.7.4, so the unpack arithmetic is checked
//! against an independent statement of the rule, not against itself.
//! The rest: what the verification draw of those cases samples (mipmap
//! completeness under GL_TEXTURE_MAX_LEVEL), and the transfers a mapped
//! pixel buffer refuses.

use core::ffi::c_void;
use std::format;
use std::string::String;
use std::vec;
use std::vec::Vec;

use super::buffer::*;
use super::consts::*;
use super::state::glPixelStorei;
use super::texture::*;
use super::types::*;
use crate::testutil::TestGl;

/// The dEQP colour formats with the transfer (format, type) glu picks for
/// each (`glu::getTransferFormat`) and the bytes of one source pixel.
const COLOR_FORMATS: &[(&str, GLenum, GLenum, GLenum, usize)] = &[
    ("rgba32f", GL_RGBA32F, GL_RGBA, GL_FLOAT, 16),
    ("rgba32i", GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, 16),
    ("rgba32ui", GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 16),
    ("rgba16f", GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, 8),
    ("rgba16i", GL_RGBA16I, GL_RGBA_INTEGER, GL_SHORT, 8),
    ("rgba16ui", GL_RGBA16UI, GL_RGBA_INTEGER, GL_UNSIGNED_SHORT, 8),
    ("rgba8", GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 4),
    ("rgba8i", GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE, 4),
    ("rgba8ui", GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, 4),
    ("srgb8_alpha8", GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE, 4),
    ("rgb10_a2", GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4),
    ("rgb10_a2ui", GL_RGB10_A2UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT_2_10_10_10_REV, 4),
    ("rgba4", GL_RGBA4, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, 2),
    ("rgb5_a1", GL_RGB5_A1, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1, 2),
    ("rgba8_snorm", GL_RGBA8_SNORM, GL_RGBA, GL_BYTE, 4),
    ("rgb8", GL_RGB8, GL_RGB, GL_UNSIGNED_BYTE, 3),
    ("rgb565", GL_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, 2),
    ("r11f_g11f_b10f", GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV, 4),
    ("rgb32f", GL_RGB32F, GL_RGB, GL_FLOAT, 12),
    ("rgb32i", GL_RGB32I, GL_RGB_INTEGER, GL_INT, 12),
    ("rgb32ui", GL_RGB32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT, 12),
    ("rgb16f", GL_RGB16F, GL_RGB, GL_HALF_FLOAT, 6),
    ("rgb16i", GL_RGB16I, GL_RGB_INTEGER, GL_SHORT, 6),
    ("rgb16ui", GL_RGB16UI, GL_RGB_INTEGER, GL_UNSIGNED_SHORT, 6),
    ("rgb8_snorm", GL_RGB8_SNORM, GL_RGB, GL_BYTE, 3),
    ("rgb8i", GL_RGB8I, GL_RGB_INTEGER, GL_BYTE, 3),
    ("rgb8ui", GL_RGB8UI, GL_RGB_INTEGER, GL_UNSIGNED_BYTE, 3),
    ("srgb8", GL_SRGB8, GL_RGB, GL_UNSIGNED_BYTE, 3),
    ("rgb9_e5", GL_RGB9_E5, GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV, 4),
    ("rg32f", GL_RG32F, GL_RG, GL_FLOAT, 8),
    ("rg32i", GL_RG32I, GL_RG_INTEGER, GL_INT, 8),
    ("rg32ui", GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, 8),
    ("rg16f", GL_RG16F, GL_RG, GL_HALF_FLOAT, 4),
    ("rg16i", GL_RG16I, GL_RG_INTEGER, GL_SHORT, 4),
    ("rg16ui", GL_RG16UI, GL_RG_INTEGER, GL_UNSIGNED_SHORT, 4),
    ("rg8", GL_RG8, GL_RG, GL_UNSIGNED_BYTE, 2),
    ("rg8i", GL_RG8I, GL_RG_INTEGER, GL_BYTE, 2),
    ("rg8ui", GL_RG8UI, GL_RG_INTEGER, GL_UNSIGNED_BYTE, 2),
    ("rg8_snorm", GL_RG8_SNORM, GL_RG, GL_BYTE, 2),
    ("r32f", GL_R32F, GL_RED, GL_FLOAT, 4),
    ("r32i", GL_R32I, GL_RED_INTEGER, GL_INT, 4),
    ("r32ui", GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, 4),
    ("r16f", GL_R16F, GL_RED, GL_HALF_FLOAT, 2),
    ("r16i", GL_R16I, GL_RED_INTEGER, GL_SHORT, 2),
    ("r16ui", GL_R16UI, GL_RED_INTEGER, GL_UNSIGNED_SHORT, 2),
    ("r8", GL_R8, GL_RED, GL_UNSIGNED_BYTE, 1),
    ("r8i", GL_R8I, GL_RED_INTEGER, GL_BYTE, 1),
    ("r8ui", GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, 1),
    ("r8_snorm", GL_R8_SNORM, GL_RED, GL_BYTE, 1),
];

/// The unpack state of one case (ES 3.0 table 3.1's UNPACK_* values).
#[derive(Clone, Copy, Default)]
struct Unpack {
    row_length: i32,
    image_height: i32,
    skip_images: i32,
    skip_rows: i32,
    skip_pixels: i32,
    alignment: i32,
    offset: usize,
}

impl Unpack {
    fn aligned(alignment: i32) -> Unpack {
        Unpack { alignment, ..Unpack::default() }
    }

    fn apply(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ROW_LENGTH, self.row_length);
            glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, self.image_height);
            glPixelStorei(GL_UNPACK_SKIP_IMAGES, self.skip_images);
            glPixelStorei(GL_UNPACK_SKIP_ROWS, self.skip_rows);
            glPixelStorei(GL_UNPACK_SKIP_PIXELS, self.skip_pixels);
            glPixelStorei(GL_UNPACK_ALIGNMENT, self.alignment);
        }
    }

    /// ES 3.0 3.7.4: the bytes from one row to the next (`k` of equation
    /// 3.3, times the component size) and from one image to the next.
    fn strides(&self, w: usize, h: usize, bpp: usize) -> (usize, usize) {
        let l = if self.row_length > 0 { self.row_length as usize } else { w };
        let a = self.alignment as usize;
        let row = (l * bpp).div_ceil(a) * a;
        let rows = if self.image_height > 0 { self.image_height as usize } else { h };
        (row, row * rows)
    }

    /// A source of `w x h x d` pixels under this state, every byte distinct
    /// enough that a misplaced read shows (`seed` varies it per face).
    fn source(&self, w: usize, h: usize, d: usize, bpp: usize, seed: usize) -> Vec<u8> {
        let (row, image) = self.strides(w, h, bpp);
        let n = self.offset
            + image * (self.skip_images as usize + d - 1)
            + row * (self.skip_rows as usize + h - 1)
            + bpp * (self.skip_pixels as usize + w);
        (0..n).map(|i| ((i * 131 + seed * 17 + (i >> 8) * 7) % 251) as u8).collect()
    }

    /// The `w x h x d` pixels the state selects from `src`, tight.
    fn tight(&self, src: &[u8], w: usize, h: usize, d: usize, bpp: usize) -> Vec<u8> {
        let (row, image) = self.strides(w, h, bpp);
        let mut out = Vec::with_capacity(w * h * d * bpp);
        for z in 0..d {
            for y in 0..h {
                let at = self.offset
                    + image * (self.skip_images as usize + z)
                    + row * (self.skip_rows as usize + y)
                    + bpp * self.skip_pixels as usize;
                out.extend_from_slice(&src[at..at + w * bpp]);
            }
        }
        out
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

/// A buffer bound to PIXEL_UNPACK_BUFFER holding `data`.
fn unpack_buffer(data: &[u8]) -> GLuint {
    let mut buf: GLuint = 0;
    // SAFETY: plain GL calls; `data` is valid for its length.
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
        glBufferData(GL_PIXEL_UNPACK_BUFFER, data.len() as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
    buf
}

fn unbind_and_delete(buf: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        glDeleteBuffers(1, &buf);
    }
}

/// The texture's storage as the GPU sees it.
fn texels(gl: &TestGl, name: GLuint) -> Vec<u8> {
    let m = gl.ctx().objs().textures[name as usize].mem;
    gl.gpu_bytes(m.gpu, m.size)
}

/// What one case does to a texture: its target, and a function that
/// specifies it from `p` (an offset into the bound buffer, or a client
/// address) for face / image `i`.
struct Case {
    name: String,
    target: GLenum,
    /// Images specified one after another (6 for a cube's faces).
    faces: usize,
    /// The source extent of one call.
    w: usize,
    h: usize,
    d: usize,
    bpp: usize,
    unpack: Unpack,
    /// Specify face (or image) `i` from `p`.
    upload: Box<dyn Fn(usize, *const c_void)>,
    /// A sub-image case's base image, uploaded before the case's own call.
    base: Option<Box<dyn Fn()>>,
}

/// How a case's texels reach the call.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    /// A pixel unpack buffer, under the case's unpack state.
    Buffer,
    /// Client memory, under the case's unpack state.
    Client,
    /// Client memory holding the tight copy, alignment 1 and nothing else.
    Tight,
}

/// Run `case` from a pixel unpack buffer and from client memory under its
/// unpack state, and from the tight copy, and compare the first two textures'
/// storage and errors with the third's.
fn check(gl: &TestGl, case: &Case) {
    let mut names = [0 as GLuint; 3];
    let mut errors = [GL_NO_ERROR; 3];
    for (k, how) in [Source::Buffer, Source::Client, Source::Tight].into_iter().enumerate() {
        Unpack::aligned(4).apply();
        names[k] = new_tex(case.target);
        if let Some(base) = &case.base {
            base();
            /* a format this back end does not store: every way refuses it */
            errors[k] = gl.error();
        }
        for f in 0..case.faces {
            let src = case.unpack.source(case.w, case.h, case.d, case.bpp, f);
            let tight = case.unpack.tight(&src, case.w, case.h, case.d, case.bpp);
            let mut buf = 0;
            match how {
                Source::Buffer => {
                    buf = unpack_buffer(&src);
                    case.unpack.apply();
                    (case.upload)(f, case.unpack.offset as *const c_void);
                }
                Source::Client => {
                    case.unpack.apply();
                    (case.upload)(f, src[case.unpack.offset..].as_ptr() as *const c_void);
                }
                Source::Tight => {
                    Unpack::aligned(1).apply();
                    (case.upload)(f, tight.as_ptr() as *const c_void);
                }
            }
            let e = gl.error();
            if e != GL_NO_ERROR {
                errors[k] = e;
            }
            if buf != 0 {
                unbind_and_delete(buf);
            }
        }
    }
    Unpack::aligned(4).apply();
    let reference = if errors[2] == GL_NO_ERROR { texels(gl, names[2]) } else { Vec::new() };
    for (k, how) in [Source::Buffer, Source::Client].into_iter().enumerate() {
        assert_eq!(
            errors[k], errors[2],
            "{} from {how:?}: error 0x{:x}, the tight upload's 0x{:x}",
            case.name, errors[k], errors[2]
        );
        if errors[2] != GL_NO_ERROR {
            continue; /* a format this back end does not store: refused every way */
        }
        let got = texels(gl, names[k]);
        assert_eq!(got.len(), reference.len(), "{} from {how:?}: storage sizes", case.name);
        if let Some(i) = got.iter().zip(&reference).position(|(x, y)| x != y) {
            panic!(
                "{} from {how:?}: storage differs from byte {i} of {} ({:?}, tight {:?})",
                case.name,
                got.len(),
                &got[i..(i + 8).min(got.len())],
                &reference[i..(i + 8).min(reference.len())]
            );
        }
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteTextures(3, names.as_ptr()) };
}

const CUBE_FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

fn image_2d(
    name: String,
    cube: bool,
    ifmt: GLenum,
    fmt: GLenum,
    ty: GLenum,
    bpp: usize,
    w: usize,
    h: usize,
    unpack: Unpack,
) -> Case {
    Case {
        name,
        target: if cube { GL_TEXTURE_CUBE_MAP } else { GL_TEXTURE_2D },
        faces: if cube { 6 } else { 1 },
        w,
        h,
        d: 1,
        bpp,
        unpack,
        upload: Box::new(move |f, p| {
            let t = if cube { CUBE_FACES[f] } else { GL_TEXTURE_2D };
            // SAFETY: `p` is an offset into the bound buffer or a tight client copy.
            unsafe { glTexImage2D(t, 0, ifmt as GLint, w as GLsizei, h as GLsizei, 0, fmt, ty, p) };
        }),
        base: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn sub_image_2d(
    name: String,
    cube: bool,
    ifmt: GLenum,
    fmt: GLenum,
    ty: GLenum,
    bpp: usize,
    size: (usize, usize),
    sub: (i32, i32, usize, usize),
    unpack: Unpack,
) -> Case {
    let (w, h) = size;
    let (x, y, sw, sh) = sub;
    Case {
        name,
        target: if cube { GL_TEXTURE_CUBE_MAP } else { GL_TEXTURE_2D },
        faces: if cube { 6 } else { 1 },
        w: sw,
        h: sh,
        d: 1,
        bpp,
        unpack,
        upload: Box::new(move |f, p| {
            let t = if cube { CUBE_FACES[f] } else { GL_TEXTURE_2D };
            // SAFETY: `p` is an offset into the bound buffer or a tight client copy.
            unsafe { glTexSubImage2D(t, 0, x, y, sw as GLsizei, sh as GLsizei, fmt, ty, p) };
        }),
        base: Some(Box::new(move || {
            let base = vec![0x5au8; w * h * bpp];
            for f in 0..if cube { 6 } else { 1 } {
                let t = if cube { CUBE_FACES[f] } else { GL_TEXTURE_2D };
                // SAFETY: `base` holds w x h tight pixels, read with alignment 1.
                unsafe {
                    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
                    glTexImage2D(
                        t,
                        0,
                        ifmt as GLint,
                        w as GLsizei,
                        h as GLsizei,
                        0,
                        fmt,
                        ty,
                        base.as_ptr() as *const c_void,
                    );
                }
            }
        })),
    }
}

#[allow(clippy::too_many_arguments)]
fn image_3d(
    name: String,
    array: bool,
    ifmt: GLenum,
    fmt: GLenum,
    ty: GLenum,
    bpp: usize,
    size: (usize, usize, usize),
    unpack: Unpack,
) -> Case {
    let (w, h, d) = size;
    Case {
        name,
        target: if array { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_3D },
        faces: 1,
        w,
        h,
        d,
        bpp,
        unpack,
        upload: Box::new(move |_, p| {
            let t = if array { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_3D };
            // SAFETY: `p` is an offset into the bound buffer or a tight client copy.
            unsafe { glTexImage3D(t, 0, ifmt as GLint, w as GLsizei, h as GLsizei, d as GLsizei, 0, fmt, ty, p) };
        }),
        base: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn sub_image_3d(
    name: String,
    array: bool,
    ifmt: GLenum,
    fmt: GLenum,
    ty: GLenum,
    bpp: usize,
    size: (usize, usize, usize),
    sub: (i32, i32, i32, usize, usize, usize),
    unpack: Unpack,
) -> Case {
    let (w, h, d) = size;
    let (x, y, z, sw, sh, sd) = sub;
    let t = if array { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_3D };
    Case {
        name,
        target: t,
        faces: 1,
        w: sw,
        h: sh,
        d: sd,
        bpp,
        unpack,
        upload: Box::new(move |_, p| {
            // SAFETY: `p` is an offset into the bound buffer or a tight client copy.
            unsafe { glTexSubImage3D(t, 0, x, y, z, sw as GLsizei, sh as GLsizei, sd as GLsizei, fmt, ty, p) };
        }),
        base: Some(Box::new(move || {
            let base = vec![0x5au8; w * h * d * bpp];
            // SAFETY: `base` holds w x h x d tight pixels.
            unsafe {
                glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
                glTexImage3D(
                    t,
                    0,
                    ifmt as GLint,
                    w as GLsizei,
                    h as GLsizei,
                    d as GLsizei,
                    0,
                    fmt,
                    ty,
                    base.as_ptr() as *const c_void,
                );
            }
        })),
    }
}

#[test]
fn teximage2d_pbo_cases_store_what_a_tight_upload_stores() {
    if crate::REPORT_ES2 {
        return; /* no pixel buffer targets nor ES 3.0 formats on the ES 2.0 surface */
    }
    let gl = TestGl::new(4, 4);
    for &(n, ifmt, fmt, ty, bpp) in COLOR_FORMATS {
        check(&gl, &image_2d(format!("{n}_2d"), false, ifmt, fmt, ty, bpp, 65, 37, Unpack::aligned(4)));
        check(&gl, &image_2d(format!("{n}_cube"), true, ifmt, fmt, ty, bpp, 64, 64, Unpack::aligned(4)));
    }
}

/// (name, internal format, width, height, row length, skip rows, skip
/// pixels, alignment, offset).
type Params2d = (&'static str, GLenum, usize, usize, i32, i32, i32, i32, usize);

/// teximage2d_pbo's parameter cases.
const PBO_2D_PARAMS: &[Params2d] = &[
    ("rgb8_offset", GL_RGB8, 31, 30, 0, 0, 0, 4, 67),
    ("rgb8_alignment", GL_RGB8, 31, 30, 0, 0, 0, 2, 0),
    ("rgb8_row_length", GL_RGB8, 31, 30, 50, 0, 0, 4, 0),
    ("rgb8_skip_rows", GL_RGB8, 31, 30, 0, 3, 0, 4, 0),
    ("rgb8_skip_pixels", GL_RGB8, 31, 30, 36, 0, 5, 4, 0),
];

/// teximage2d_unpack_params (from client memory, the same rule).
const UNPACK_2D_PARAMS: &[Params2d] = &[
    ("rgb8_alignment", GL_RGB8, 31, 30, 0, 0, 0, 2, 0),
    ("rgb8_row_length", GL_RGB8, 31, 30, 50, 0, 0, 4, 0),
    ("rgb8_skip_rows", GL_RGB8, 31, 30, 0, 3, 0, 4, 0),
    ("rgb8_skip_pixels", GL_RGB8, 31, 30, 36, 0, 5, 4, 0),
    ("r8_complex1", GL_R8, 31, 30, 64, 1, 3, 1, 0),
    ("r8_complex2", GL_R8, 31, 30, 64, 1, 3, 2, 0),
    ("r8_complex3", GL_R8, 31, 30, 64, 1, 3, 4, 0),
    ("r8_complex4", GL_R8, 31, 30, 64, 1, 3, 8, 0),
    ("rgba8_complex1", GL_RGBA8, 56, 61, 69, 0, 0, 8, 0),
    ("rgba8_complex2", GL_RGBA8, 56, 61, 69, 0, 7, 8, 0),
    ("rgba8_complex3", GL_RGBA8, 56, 61, 69, 3, 0, 8, 0),
    ("rgba8_complex4", GL_RGBA8, 56, 61, 69, 3, 7, 8, 0),
    ("rgba32f_complex", GL_RGBA32F, 19, 10, 27, 1, 7, 8, 0),
];

fn format_of(ifmt: GLenum) -> (GLenum, GLenum, usize) {
    let &(_, _, f, t, b) = COLOR_FORMATS.iter().find(|e| e.1 == ifmt).expect("a dEQP colour format");
    (f, t, b)
}

#[test]
fn teximage2d_parameter_cases_store_what_a_tight_upload_stores() {
    if crate::REPORT_ES2 {
        return; /* no pixel buffer targets nor ES 3.0 formats on the ES 2.0 surface */
    }
    let gl = TestGl::new(4, 4);
    for &(n, ifmt, w, h, row_length, skip_rows, skip_pixels, alignment, offset) in
        PBO_2D_PARAMS.iter().chain(UNPACK_2D_PARAMS)
    {
        let (f, t, b) = format_of(ifmt);
        let u = Unpack { row_length, skip_rows, skip_pixels, alignment, offset, ..Unpack::default() };
        check(&gl, &image_2d(format!("{n}_2d"), false, ifmt, f, t, b, w, h, u));
        check(&gl, &image_2d(format!("{n}_cube"), true, ifmt, f, t, b, w, w, u));
    }
    /* teximage2d_align: (internal format, width, height, alignment) */
    for (ifmt, w, h) in [(GL_R8, 4, 8), (GL_R8, 63, 30), (GL_RGBA4, 51, 30), (GL_RGB8, 39, 43), (GL_RGBA8, 47, 27)] {
        for a in [1, 2, 4, 8] {
            let (f, t, b) = format_of(ifmt);
            check(&gl, &image_2d(format!("align {ifmt:x} {w} {a}"), false, ifmt, f, t, b, w, h, Unpack::aligned(a)));
            check(
                &gl,
                &image_2d(format!("align cube {ifmt:x} {w} {a}"), true, ifmt, f, t, b, w, w, Unpack::aligned(a)),
            );
        }
    }
}

#[test]
fn texsubimage2d_pbo_cases_store_what_a_tight_upload_stores() {
    if crate::REPORT_ES2 {
        return; /* no pixel buffer targets nor ES 3.0 formats on the ES 2.0 surface */
    }
    let gl = TestGl::new(4, 4);
    for &(n, ifmt, fmt, ty, bpp) in COLOR_FORMATS {
        let u = Unpack::aligned(4);
        check(&gl, &sub_image_2d(format!("{n}_2d"), false, ifmt, fmt, ty, bpp, (54, 60), (11, 7, 31, 30), u));
        check(&gl, &sub_image_2d(format!("{n}_cube"), true, ifmt, fmt, ty, bpp, (64, 64), (11, 7, 31, 30), u));
    }
    /* the parameter cases (texsubimage2d_pbo and texsubimage2d_unpack_params) */
    for &(n, ifmt, row_length, skip_rows, skip_pixels, alignment, offset) in &[
        ("rgb8_offset", GL_RGB8, 0, 0, 0, 4, 67usize),
        ("rgb8_alignment", GL_RGB8, 0, 0, 0, 2, 0),
        ("rgb8_row_length", GL_RGB8, 50, 0, 0, 4, 0),
        ("rgb8_skip_rows", GL_RGB8, 0, 3, 0, 4, 0),
        ("rgb8_skip_pixels", GL_RGB8, 36, 0, 5, 4, 0),
        ("r8_complex1", GL_R8, 64, 1, 3, 1, 0),
        ("r8_complex4", GL_R8, 64, 1, 3, 8, 0),
    ] {
        let (f, t, b) = format_of(ifmt);
        let u = Unpack { row_length, skip_rows, skip_pixels, alignment, offset, ..Unpack::default() };
        check(&gl, &sub_image_2d(format!("{n}_2d"), false, ifmt, f, t, b, (54, 60), (11, 7, 31, 30), u));
        check(&gl, &sub_image_2d(format!("{n}_cube"), true, ifmt, f, t, b, (64, 64), (11, 7, 31, 30), u));
    }
    for &(n, ifmt, row_length, skip_rows, skip_pixels) in &[
        ("rgba8_complex1", GL_RGBA8, 69, 0, 0),
        ("rgba8_complex4", GL_RGBA8, 69, 3, 7),
        ("rgba32f_complex", GL_RGBA32F, 69, 3, 7),
    ] {
        let (f, t, b) = format_of(ifmt);
        let u = Unpack { row_length, skip_rows, skip_pixels, alignment: 8, ..Unpack::default() };
        check(&gl, &sub_image_2d(format!("{n}_2d"), false, ifmt, f, t, b, (92, 84), (13, 19, 56, 61), u));
    }
    /* texsubimage2d_align */
    for (ifmt, sub) in [
        (GL_R8, (13, 17, 1, 6)),
        (GL_R8, (1, 9, 63, 30)),
        (GL_RGBA4, (7, 29, 51, 30)),
        (GL_RGB8, (11, 8, 39, 43)),
        (GL_RGBA8, (10, 1, 47, 27)),
    ] {
        for a in [1, 2, 4, 8] {
            let (f, t, b) = format_of(ifmt);
            check(
                &gl,
                &sub_image_2d(
                    format!("sub align {ifmt:x} {a}"),
                    false,
                    ifmt,
                    f,
                    t,
                    b,
                    (64, 64),
                    sub,
                    Unpack::aligned(a),
                ),
            );
            check(
                &gl,
                &sub_image_2d(
                    format!("sub align cube {ifmt:x} {a}"),
                    true,
                    ifmt,
                    f,
                    t,
                    b,
                    (64, 64),
                    sub,
                    Unpack::aligned(a),
                ),
            );
        }
    }
}

/// (name, internal format, width, height, depth, image height, row length,
/// skip images, skip rows, skip pixels, alignment, offset).
type Params3d = (&'static str, GLenum, usize, usize, usize, i32, i32, i32, i32, i32, i32, usize);

const PARAMS_3D: &[Params3d] = &[
    ("rgb8_offset", GL_RGB8, 23, 19, 8, 0, 0, 0, 0, 0, 1, 67),
    ("rgb8_alignment", GL_RGB8, 23, 19, 8, 0, 0, 0, 0, 0, 2, 0),
    ("rgb8_image_height", GL_RGB8, 23, 19, 8, 26, 0, 0, 0, 0, 4, 0),
    ("rgb8_row_length", GL_RGB8, 23, 19, 8, 0, 27, 0, 0, 0, 4, 0),
    ("rgb8_skip_images", GL_RGB8, 23, 19, 8, 0, 0, 3, 0, 0, 4, 0),
    ("rgb8_skip_rows", GL_RGB8, 23, 19, 8, 22, 0, 0, 3, 0, 4, 0),
    ("rgb8_skip_pixels", GL_RGB8, 23, 19, 8, 0, 25, 0, 0, 2, 4, 0),
    ("r8_complex1", GL_R8, 13, 17, 11, 23, 15, 2, 3, 1, 1, 0),
    ("r8_complex4", GL_R8, 13, 17, 11, 23, 15, 2, 3, 1, 8, 0),
    ("rgba8_complex1", GL_RGBA8, 11, 20, 8, 25, 14, 0, 0, 0, 8, 0),
    ("rgba8_complex4", GL_RGBA8, 11, 20, 8, 25, 14, 0, 2, 3, 8, 0),
    ("rgba32f_complex", GL_RGBA32F, 11, 20, 8, 25, 14, 0, 2, 3, 8, 0),
];

#[test]
fn teximage3d_pbo_cases_store_what_a_tight_upload_stores() {
    if crate::REPORT_ES2 {
        return; /* no pixel buffer targets nor ES 3.0 formats on the ES 2.0 surface */
    }
    let gl = TestGl::new(4, 4);
    for &(n, ifmt, fmt, ty, bpp) in COLOR_FORMATS {
        for array in [true, false] {
            check(
                &gl,
                &image_3d(format!("{n} array {array}"), array, ifmt, fmt, ty, bpp, (11, 20, 8), Unpack::aligned(4)),
            );
        }
    }
    for &(n, ifmt, w, h, d, image_height, row_length, skip_images, skip_rows, skip_pixels, alignment, offset) in
        PARAMS_3D
    {
        let (f, t, b) = format_of(ifmt);
        let u = Unpack { row_length, image_height, skip_images, skip_rows, skip_pixels, alignment, offset };
        for array in [true, false] {
            check(&gl, &image_3d(format!("{n} array {array}"), array, ifmt, f, t, b, (w, h, d), u));
        }
    }
}

#[test]
fn texsubimage3d_pbo_cases_store_what_a_tight_upload_stores() {
    if crate::REPORT_ES2 {
        return; /* no pixel buffer targets nor ES 3.0 formats on the ES 2.0 surface */
    }
    let gl = TestGl::new(4, 4);
    for &(n, ifmt, fmt, ty, bpp) in COLOR_FORMATS {
        for array in [true, false] {
            let c = sub_image_3d(
                format!("{n} array {array}"),
                array,
                ifmt,
                fmt,
                ty,
                bpp,
                (26, 25, 10),
                (1, 2, 0, 23, 19, 8),
                Unpack::aligned(4),
            );
            check(&gl, &c);
        }
    }
    for &(n, ifmt, _, _, _, image_height, row_length, skip_images, skip_rows, skip_pixels, alignment, offset) in
        PARAMS_3D
    {
        let (f, t, b) = format_of(ifmt);
        let u = Unpack { row_length, image_height, skip_images, skip_rows, skip_pixels, alignment, offset };
        let (size, sub) =
            if ifmt == GL_RGB8 { ((26, 25, 10), (1, 2, 1, 23, 19, 8)) } else { ((15, 20, 11), (1, 1, 0, 13, 17, 11)) };
        for array in [true, false] {
            check(&gl, &sub_image_3d(format!("{n} array {array}"), array, ifmt, f, t, b, size, sub, u));
        }
    }
}

/* ---- what the verification draw samples ------------------------------------ */

const TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];

/// Draw one triangle sampling the 2D texture bound to unit 0 and return the
/// binding the back end received.
fn draw_2d(gl: &TestGl) -> crate::backend::GlesTextureBind {
    let prog = gl.program("uv.vert", "texture.frag");
    gl.clear_recordings();
    // SAFETY: plain GL calls; TRI is a static.
    unsafe {
        use crate::gl::{draw::*, program::*, vertex::*};
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    assert_eq!(gl.error(), GL_NO_ERROR);
    let d = gl.draws();
    assert_eq!(d.len(), 1);
    d[0].draw.textures[0]
}

fn tex_parameter(pname: GLenum, v: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_2D, pname, v as GLint) };
}

/// dEQP-GLES3 texture.specification verifies a texture of N levels with
/// NEAREST_MIPMAP_NEAREST and GL_TEXTURE_MAX_LEVEL N - 1 (Texture2DSpecCase::
/// verifyTexture). ES 3.0 3.8.13: the levels a mipmap filter needs run from
/// the base level to q = min(p, MAX_LEVEL), so one level with MAX_LEVEL 0
/// is complete. The draw called it incomplete -- "fewer levels uploaded
/// than the chain needs" -- and every one-level case (teximage2d_pbo,
/// texsubimage2d_pbo, *_unpack_params, *_align, texstorage2d with a short
/// chain) read (0,0,0,1) on the AM62P.
#[test]
fn a_mipmap_filter_needs_the_levels_up_to_max_level_only() {
    let gl = TestGl::new(16, 16);
    let px = vec![0x40u8; 65 * 37 * 4];
    new_tex(GL_TEXTURE_2D);
    // SAFETY: `px` holds the 65 x 37 RGBA8 image (rows of 260 bytes).
    unsafe {
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            65,
            37,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        )
    };
    tex_parameter(GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
    tex_parameter(GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    assert_eq!(draw_2d(&gl).incomplete, 1, "one level of seven, no MAX_LEVEL: incomplete");
    tex_parameter(GL_TEXTURE_MAX_LEVEL, 0);
    let b = draw_2d(&gl);
    assert_eq!((b.incomplete, b.mip_levels), (u8::from(crate::REPORT_ES2), 1), "MAX_LEVEL 0: level 0 is the chain");
    tex_parameter(GL_TEXTURE_MAX_LEVEL, 1);
    assert_eq!(draw_2d(&gl).incomplete, 1, "MAX_LEVEL 1: level 1 is missing");
}

/// An immutable texture's chain ends at its last level whatever MAX_LEVEL
/// says (ES 3.0 3.8.4: "levels-1" clamps the maximum level of an immutable
/// texture): texstorage2d.size.2d_32x64_4_levels, cube_64x64_4_levels.
#[test]
fn an_immutable_texture_needs_only_its_levels() {
    if crate::REPORT_ES2 {
        return; /* glTexStorage2D is not on the ES 2.0 surface */
    }
    let gl = TestGl::new(16, 16);
    new_tex(GL_TEXTURE_2D);
    // SAFETY: plain GL call.
    unsafe { glTexStorage2D(GL_TEXTURE_2D, 4, GL_RGBA8, 32, 64) };
    assert_eq!(gl.error(), GL_NO_ERROR);
    tex_parameter(GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
    tex_parameter(GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    let px = vec![0x40u8; 32 * 64 * 4];
    for l in 0..4 {
        // SAFETY: `px` holds level 0's texels, more than any level needs.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                l,
                0,
                0,
                32 >> l,
                64 >> l,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            )
        };
    }
    assert_eq!(draw_2d(&gl).incomplete, 0, "four levels of seven, MAX_LEVEL left at 1000");
    tex_parameter(GL_TEXTURE_MAX_LEVEL, 3);
    let b = draw_2d(&gl);
    assert_eq!((b.incomplete, b.mip_levels, b.mip_filter), (0, 4, 1));
}

/// ES 3.0 2.9.4 / 3.7.1 / 4.3.2: a pixel transfer whose buffer is mapped is
/// GL_INVALID_OPERATION, on every entry point that reads or writes one, and
/// leaves the texture or the buffer as they were.
#[test]
fn a_mapped_pixel_buffer_is_refused_by_every_transfer() {
    if crate::REPORT_ES2 {
        return; /* no pixel buffer targets on the ES 2.0 surface */
    }
    let gl = TestGl::new(8, 8);
    let tex = new_tex(GL_TEXTURE_2D);
    let px = [7u8; 16];
    // SAFETY: `px` holds the 2 x 2 RGBA8 image.
    unsafe {
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            2,
            2,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        )
    };
    let before = texels(&gl, tex);
    let buf = unpack_buffer(&[1u8; 64]);
    // SAFETY: plain GL calls; every pointer is an offset into the mapped buffer.
    unsafe {
        assert!(!glMapBufferRange(GL_PIXEL_UNPACK_BUFFER, 0, 64, GL_MAP_WRITE_BIT).is_null());
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "glTexImage2D");
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "glTexSubImage2D");
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 4, 4, 0, 8, core::ptr::null());
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "glCompressedTexImage2D");
        let t3 = new_tex(GL_TEXTURE_3D);
        glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8 as GLint, 2, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "glTexImage3D");
        glDeleteTextures(1, &t3);
        glBindTexture(GL_TEXTURE_2D, tex);
        glUnmapBuffer(GL_PIXEL_UNPACK_BUFFER);
    }
    assert_eq!(texels(&gl, tex), before, "nothing was read");
    /* unmapped, the same upload reads the buffer */
    // SAFETY: plain GL call; offset 0 of the 64-byte buffer.
    unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null()) };
    assert_eq!(gl.error(), GL_NO_ERROR);
    assert_eq!(texels(&gl, tex)[..4], [1, 1, 1, 1]);
    unbind_and_delete(buf);

    /* glReadPixels into a mapped pack buffer */
    let mut pack: GLuint = 0;
    // SAFETY: plain GL calls; the read's pointer is an offset into the buffer.
    unsafe {
        glGenBuffers(1, &mut pack);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pack);
        glBufferData(GL_PIXEL_PACK_BUFFER, 64, [0xEEu8; 64].as_ptr() as *const c_void, GL_STREAM_READ);
        assert!(!glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, 64, GL_MAP_READ_BIT).is_null());
        crate::gl::framebuffer::glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null_mut());
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "glReadPixels");
        glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
        let p = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, 64, GL_MAP_READ_BIT) as *const u8;
        assert_eq!(core::slice::from_raw_parts(p, 64), &[0xEE; 64], "nothing was written");
        glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
        glDeleteBuffers(1, &pack);
    }
}

/// The same rule for the targets glTexImage3D specifies: teximage3d_pbo and
/// texsubimage3d_pbo verify one-level 2D arrays and 3D textures with
/// NEAREST_MIPMAP_NEAREST and GL_TEXTURE_MAX_LEVEL 0 (Texture2DArraySpecCase
/// / Texture3DSpecCase::verifyTexture).
#[test]
fn a_one_level_array_or_3d_texture_with_max_level_0_is_complete() {
    if crate::REPORT_ES2 {
        return; /* glTexImage3D is not on the ES 2.0 surface */
    }
    let gl = TestGl::new(4, 4);
    let px = vec![0x40u8; 11 * 20 * 8 * 4];
    for target in [GL_TEXTURE_2D_ARRAY, GL_TEXTURE_3D] {
        let name = new_tex(target);
        // SAFETY: `px` holds the 11 x 20 x 8 RGBA8 image; plain GL calls.
        unsafe {
            glTexImage3D(
                target,
                0,
                GL_RGBA8 as GLint,
                11,
                20,
                8,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
            glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let incomplete = || super::draw::texture_bind_2d(&gl.ctx().objs().textures[name as usize]).incomplete;
        assert_eq!(incomplete(), 1, "0x{target:x}: one level, no MAX_LEVEL");
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(target, GL_TEXTURE_MAX_LEVEL, 0) };
        assert_eq!(incomplete(), 0, "0x{target:x}: MAX_LEVEL 0");
    }
}
