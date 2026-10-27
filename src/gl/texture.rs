// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Texture objects: 2D textures, cube maps and 2D texture arrays.
//!
//! Texels are stored GPU-resident as linear RGBA8888 (what the PowerVR TPU image
//! state the backend builds expects), or as linear float texels (RGBA16F /
//! R16F / R32F) for the ES 3.0 float formats. glTexImage2D converts the
//! incoming format/type on upload; glTexParameteri records filter/wrap that
//! the draw resolver maps onto `GlesTextureBind`. A cube map (6 faces) or a
//! 2D array (N layers) keeps its faces/layers stacked vertically in one
//! linear texture, which the shaders sample as a tall 2D texture
//! (the sampler emulation of `yakogl-tools glsl2usc`).
#![allow(non_snake_case)]

use core::ffi::c_void;

use super::consts::*;
use super::context::{
    self, gl_trace, heap_data, one_wide_level_pitch, pot_level_side, tex_pitch, GlAttachment, GlContext, GlTexture,
    CAP_COMBINED_TEXTURE_UNITS, CAP_MIP_LEVELS, MAX_TEXTURES, RGBA8_BPP, TEXTURE_SLOTS,
};
use super::es3_formats;
use super::names::{self, Kind};
use super::types::*;
use super::{draw, framebuffer};
use crate::backend::{
    self, gles_texfmt_bytes, GLES_SURF_RGB565, GLES_SURF_XRGB8888, GLES_TEXFMT_A8, GLES_TEXFMT_ETC2_RGB,
    GLES_TEXFMT_ETC2_RGBA, GLES_TEXFMT_ETC2_SRGB_RGB, GLES_TEXFMT_ETC2_SRGB_RGBA, GLES_TEXFMT_L8, GLES_TEXFMT_LA8,
    GLES_TEXFMT_R16F, GLES_TEXFMT_R32F, GLES_TEXFMT_RGB565, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8,
    GLES_TEXFMT_SRGB8_A8,
};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::{self, Global};

/// The texture targets ES 3.0 adds to the two of ES 2.0: a `report-es2` build
/// refuses them with GL_INVALID_ENUM in every call that takes a texture target
/// (through [`binding_for`], and in glBindTexture).
///
/// ES 2.0.25 (read from the extracted build/es_full_spec_2.0.25.txt), 3.7.13:
///
/// ```text
/// A texture object is created by binding an unused name to TEXTURE_2D or
/// TEXTURE_CUBE_MAP.
/// ```
///
/// 3.7.4 and 3.7.11 say the same of TexParameter and GenerateMipmap ("target
/// is the target, which must be TEXTURE_2D or TEXTURE_CUBE_MAP"), 6.1.3 of
/// GetTexParameter; any other value is GL_INVALID_ENUM (2.5).
fn es3_only_texture_target(target: GLenum) -> bool {
    crate::REPORT_ES2 && matches!(target, GL_TEXTURE_3D | GL_TEXTURE_2D_ARRAY)
}

/// The binding slot of the active unit for a texture target (a cube map face
/// target names the cube map binding); None for a target this GL lacks.
fn binding_for(c: &mut GlContext, target: GLenum) -> Option<&mut GLuint> {
    if es3_only_texture_target(target) {
        return None;
    }
    let unit = c.active_texture as usize;
    if target == GL_TEXTURE_2D {
        return Some(&mut c.bound_texture_2d[unit]);
    }
    if target == GL_TEXTURE_CUBE_MAP || is_face_target(target) {
        return Some(&mut c.bound_texture_cube[unit]);
    }
    if target == GL_TEXTURE_2D_ARRAY {
        return Some(&mut c.bound_texture_2d_array[unit]);
    }
    if target == GL_TEXTURE_3D {
        return Some(&mut c.bound_texture_3d[unit]);
    }
    None
}

/// Table index of the texture bound to `target` of the active unit.
/// Slot of a target's default texture object in the tail of the texture
/// table (see [`context::DEFAULT_TEXTURES`]).
pub(crate) fn default_slot(target: GLenum) -> Option<usize> {
    let i = match target {
        GL_TEXTURE_2D => 0,
        t if t == GL_TEXTURE_CUBE_MAP || is_face_target(t) => 1,
        GL_TEXTURE_2D_ARRAY => 2,
        GL_TEXTURE_3D => 3,
        _ => return None,
    };
    Some(MAX_TEXTURES + i)
}

fn active_texture(c: &mut GlContext, target: GLenum) -> Option<usize> {
    let bound = *binding_for(c, target)?;
    if bound == 0 {
        /* The default texture object of this target, created on demand with
         * GL's initial parameter values (ES 3.0 3.8). Without one, every
         * glTexParameteri against a fresh context answered
         * GL_INVALID_OPERATION -- which is what a conformance run does
         * between test cases to put the state back, so the run ended there. */
        let i = default_slot(target)?;
        let t = &mut c.objs().textures[i];

        if !t.used {
            init_texture(t);
            t.target = if is_face_target(target) { GL_TEXTURE_CUBE_MAP } else { target };
            t.layers = if t.target == GL_TEXTURE_CUBE_MAP { 6 } else { 1 };
        }
        return Some(i);
    }
    c.texture_ref(bound).map(|_| bound as usize)
}

/// Table index of the texture bound to TEXTURE_2D of the active unit.
fn active_2d_texture(c: &mut GlContext) -> Option<usize> {
    active_texture(c, GL_TEXTURE_2D)
}

fn is_face_target(target: GLenum) -> bool {
    (GL_TEXTURE_CUBE_MAP_POSITIVE_X..=GL_TEXTURE_CUBE_MAP_NEGATIVE_Z).contains(&target)
}

/// Cube map face index of a face target, else 0 (2D and array targets).
fn face_of(target: GLenum) -> u32 {
    if is_face_target(target) {
        target - GL_TEXTURE_CUBE_MAP_POSITIVE_X
    } else {
        0
    }
}

fn half_to_float(h: u16) -> f32 {
    let s = ((h & 0x8000) as u32) << 16;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;

    let bits = if e == 0 {
        0 /* zero; a subnormal is treated as 0 too (far below 1/255) */
    } else if e == 31 {
        0x7f80_0000 | (m << 13)
    } else {
        ((e + 112) << 23) | (m << 13)
    };
    f32::from_bits(s | bits)
}

/// float -> IEEE half (round toward zero, overflow to infinity).
pub(crate) fn float_to_half(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = (x >> 16) & 0x8000;
    let exp = (x >> 23) & 0xff;
    let mut mant = x & 0x7f_ffff;

    if exp == 0xff {
        return (sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 }) as u16; /* inf / nan */
    }
    if exp > 142 {
        return (sign | 0x7c00) as u16; /* overflow -> inf */
    }
    if exp < 103 {
        return sign as u16; /* underflow -> 0 */
    }
    if exp < 113 {
        /* subnormal half */
        mant |= 0x80_0000;
        return (sign | (mant >> (126 - exp))) as u16;
    }
    (sign | ((exp - 112) << 10) | (mant >> 13)) as u16
}

/// Source texel (format, type) -> the float channels (r, g, b, a). `src`
/// holds the texel's `src_bpp(format, type)` bytes.
fn src_texel_float(src: &[u8], format: GLenum, type_: GLenum) -> [f32; 4] {
    if format == GL_DEPTH_COMPONENT || format == GL_DEPTH_STENCIL {
        return [depth_texel(src, type_), 0.0, 0.0, 1.0];
    }
    let comps = match format {
        GL_RGBA => 4,
        GL_RGB => 3,
        GL_RG | GL_LUMINANCE_ALPHA => 2,
        _ => 1,
    };
    let mut v = [0.0f32, 0.0, 0.0, 1.0];

    for (c, vc) in v.iter_mut().enumerate().take(comps) {
        *vc = if type_ == GL_FLOAT {
            f32::from_ne_bytes([src[c * 4], src[c * 4 + 1], src[c * 4 + 2], src[c * 4 + 3]])
        } else if is_half_float(type_) {
            half_to_float(u16::from_ne_bytes([src[c * 2], src[c * 2 + 1]]))
        } else {
            src[c] as f32 / 255.0
        };
    }
    if format == GL_LUMINANCE || format == GL_LUMINANCE_ALPHA {
        v[3] = if comps == 2 { v[1] } else { 1.0 };
        v[1] = v[0];
        v[2] = v[0];
    } else if format == GL_ALPHA {
        v[3] = v[0];
        v[0] = 0.0;
        v[1] = 0.0;
        v[2] = 0.0;
    }
    v
}

/// One depth texel's value in [0, 1] (ES 3.0.6 3.8.3, "For depth component
/// groups, the depth value is clamped to [0, 1]"; the types of table 3.2): an
/// unsigned integer divided by its type's maximum (the 24-bit depth of a
/// UNSIGNED_INT_24_8 texel is its top 24 bits), a float clamped to [0, 1]
/// (the 32F + 8 pair: its first word). The stencil byte is not kept: no
/// sampler reads it here.
fn depth_texel(src: &[u8], type_: GLenum) -> f32 {
    let u32_at = |o: usize| u32::from_ne_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);
    match type_ {
        GL_UNSIGNED_SHORT => u16::from_ne_bytes([src[0], src[1]]) as f32 / 65535.0,
        GL_UNSIGNED_INT => (u32_at(0) as f64 / u32::MAX as f64) as f32,
        GL_UNSIGNED_INT_24_8 => (u32_at(0) >> 8) as f32 / 16_777_215.0,
        GL_FLOAT | GL_FLOAT_32_UNSIGNED_INT_24_8_REV => f32::from_bits(u32_at(0)).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Expand one source row into a float texel format (RGBA16F / R16F / R32F).
/// `src` holds `w` texels of `src_bpp(format, type)` bytes, `dst` room for
/// `w` texels of the texel format.
fn convert_row_float(dst: &mut [u8], src: &[u8], w: u32, format: GLenum, type_: GLenum, texfmt: u8) {
    let sbpp = src_bpp(format, type_) as usize;

    for x in 0..w as usize {
        let v = src_texel_float(&src[x * sbpp..(x + 1) * sbpp], format, type_);

        if texfmt == GLES_TEXFMT_RGBA16F {
            for (c, vc) in v.iter().enumerate() {
                dst[x * 8 + c * 2..x * 8 + c * 2 + 2].copy_from_slice(&float_to_half(*vc).to_ne_bytes());
            }
        } else if texfmt == GLES_TEXFMT_R16F {
            dst[x * 2..x * 2 + 2].copy_from_slice(&float_to_half(v[0]).to_ne_bytes());
        } else {
            dst[x * 4..x * 4 + 4].copy_from_slice(&v[0].to_ne_bytes());
        }
    }
}

fn unorm8(f: f32) -> u8 {
    /* also NaN */
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(f > 0.0) {
        return 0;
    }
    if f >= 1.0 {
        return 255;
    }
    (f * 255.0 + 0.5) as u8
}

/// Components per texel a float upload of `format` carries.
fn float_comps(format: GLenum) -> usize {
    match format {
        GL_RGBA => 4,
        GL_RGB => 3,
        GL_RG | GL_LUMINANCE_ALPHA => 2,
        _ => 1,
    }
}

/// Source bytes [`convert_row`] reads for `w` texels of (format,type): the
/// length of the slice made of the application's pointer. Pairs it leaves
/// opaque black read nothing.
fn src_used_bytes(w: u32, format: GLenum, type_: GLenum) -> usize {
    let per_texel = match type_ {
        GL_UNSIGNED_BYTE => match format {
            GL_RGBA | GL_BGRA_EXT => 4,
            GL_RGB => 3,
            GL_LUMINANCE_ALPHA | GL_RG => 2,
            GL_LUMINANCE | GL_ALPHA | GL_RED => 1,
            _ => 0,
        },
        GL_FLOAT => float_comps(format) * 4,
        GL_HALF_FLOAT | GL_HALF_FLOAT_OES => float_comps(format) * 2,
        GL_UNSIGNED_SHORT_5_6_5 | GL_UNSIGNED_SHORT_4_4_4_4 | GL_UNSIGNED_SHORT_5_5_5_1 => 2,
        GL_UNSIGNED_INT_2_10_10_10_REV if !crate::REPORT_ES2 => 4,
        _ => 0,
    };
    w as usize * per_texel
}

/// Expand one source row of (format,type) into dst RGBA8888. Supports the common
/// GLES2 unsized formats with UNSIGNED_BYTE, plus the 16-bit packed RGB/RGBA
/// types. Unhandled combos fall back to opaque black (and the caller has already
/// flagged GL_INVALID_ENUM).
fn convert_row(dst: &mut [u8], src: &[u8], w: u32, format: GLenum, type_: GLenum) {
    let w = w as usize;

    if type_ == GL_UNSIGNED_BYTE {
        match format {
            GL_RGBA => {
                dst[..w * 4].copy_from_slice(&src[..w * 4]);
                return;
            }
            /* GL_EXT_texture_format_BGRA8888: the application's bytes are
             * B,G,R,A and the store is R,G,B,A, so red and blue swap. The
             * colour is then right for everything downstream -- sampling,
             * rendering into it, reading it back -- because only the upload
             * ever sees the external order. */
            GL_BGRA_EXT => {
                for x in 0..w {
                    dst[x * 4] = src[x * 4 + 2];
                    dst[x * 4 + 1] = src[x * 4 + 1];
                    dst[x * 4 + 2] = src[x * 4];
                    dst[x * 4 + 3] = src[x * 4 + 3];
                }
                return;
            }
            GL_RGB => {
                for x in 0..w {
                    dst[x * 4] = src[x * 3];
                    dst[x * 4 + 1] = src[x * 3 + 1];
                    dst[x * 4 + 2] = src[x * 3 + 2];
                    dst[x * 4 + 3] = 255;
                }
                return;
            }
            GL_LUMINANCE => {
                for x in 0..w {
                    let l = src[x];
                    dst[x * 4] = l;
                    dst[x * 4 + 1] = l;
                    dst[x * 4 + 2] = l;
                    dst[x * 4 + 3] = 255;
                }
                return;
            }
            GL_LUMINANCE_ALPHA => {
                for x in 0..w {
                    let (l, a) = (src[x * 2], src[x * 2 + 1]);
                    dst[x * 4] = l;
                    dst[x * 4 + 1] = l;
                    dst[x * 4 + 2] = l;
                    dst[x * 4 + 3] = a;
                }
                return;
            }
            GL_ALPHA => {
                for x in 0..w {
                    dst[x * 4] = 0;
                    dst[x * 4 + 1] = 0;
                    dst[x * 4 + 2] = 0;
                    dst[x * 4 + 3] = src[x];
                }
                return;
            }
            /* ES 3.0: a RED texture reads (r, 0, 0, 1) */
            GL_RED => {
                for x in 0..w {
                    dst[x * 4] = src[x];
                    dst[x * 4 + 1] = 0;
                    dst[x * 4 + 2] = 0;
                    dst[x * 4 + 3] = 255;
                }
                return;
            }
            GL_RG => {
                for x in 0..w {
                    dst[x * 4] = src[x * 2];
                    dst[x * 4 + 1] = src[x * 2 + 1];
                    dst[x * 4 + 2] = 0;
                    dst[x * 4 + 3] = 255;
                }
                return;
            }
            _ => {}
        }
    } else if type_ == GL_FLOAT || is_half_float(type_) {
        /* Float texels into the 8-bit store: clamped to [0, 1]. This is
         * what an ES 3.0 float texture gets until the float texel formats
         * (RGBA16F/R16F/R32F: HDR light probes, shadow maps) land. */
        let comps = float_comps(format);

        for x in 0..w {
            let mut v = [0.0f32, 0.0, 0.0, 1.0];

            for (k, vk) in v.iter_mut().enumerate().take(comps) {
                *vk = if type_ == GL_FLOAT {
                    let o = (x * comps + k) * 4;
                    f32::from_ne_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]])
                } else {
                    let o = (x * comps + k) * 2;
                    half_to_float(u16::from_ne_bytes([src[o], src[o + 1]]))
                };
            }
            if format == GL_LUMINANCE || format == GL_LUMINANCE_ALPHA {
                v[3] = if comps == 2 { v[1] } else { 1.0 };
                v[1] = v[0];
                v[2] = v[0];
            } else if format == GL_ALPHA {
                v[3] = v[0];
                v[0] = 0.0;
                v[1] = 0.0;
                v[2] = 0.0;
            }
            for k in 0..4 {
                dst[x * 4 + k] = unorm8(v[k]);
            }
        }
        return;
    } else {
        let s16 = |x: usize| u16::from_ne_bytes([src[x * 2], src[x * 2 + 1]]);

        if type_ == GL_UNSIGNED_SHORT_5_6_5 {
            for x in 0..w {
                let p = s16(x);
                let r = ((p >> 11) & 0x1F) as u8;
                let g = ((p >> 5) & 0x3F) as u8;
                let b = (p & 0x1F) as u8;
                dst[x * 4] = (r << 3) | (r >> 2);
                dst[x * 4 + 1] = (g << 2) | (g >> 4);
                dst[x * 4 + 2] = (b << 3) | (b >> 2);
                dst[x * 4 + 3] = 255;
            }
            return;
        }
        if type_ == GL_UNSIGNED_SHORT_4_4_4_4 {
            for x in 0..w {
                let p = s16(x);
                let r = ((p >> 12) & 0xF) as u8;
                let g = ((p >> 8) & 0xF) as u8;
                let b = ((p >> 4) & 0xF) as u8;
                let a = (p & 0xF) as u8;
                dst[x * 4] = r * 17;
                dst[x * 4 + 1] = g * 17;
                dst[x * 4 + 2] = b * 17;
                dst[x * 4 + 3] = a * 17;
            }
            return;
        }
        if type_ == GL_UNSIGNED_SHORT_5_5_5_1 {
            for x in 0..w {
                let p = s16(x);
                let r = ((p >> 11) & 0x1F) as u8;
                let g = ((p >> 6) & 0x1F) as u8;
                let b = ((p >> 1) & 0x1F) as u8;
                let a = (p & 0x1) as u8;
                dst[x * 4] = (r << 3) | (r >> 2);
                dst[x * 4 + 1] = (g << 3) | (g >> 2);
                dst[x * 4 + 2] = (b << 3) | (b >> 2);
                dst[x * 4 + 3] = if a != 0 { 255 } else { 0 };
            }
            return;
        }
        /* ES 3.0 table 3.2: RGB5_A1 also unpacks the 10-bit word (the only
         * internal format of the original tables that takes it, RGB10_A2
         * being es3_formats'). Its alpha has one bit: 2 and 3 are set. */
        if type_ == GL_UNSIGNED_INT_2_10_10_10_REV && format == GL_RGBA {
            for x in 0..w {
                let p = u32::from_ne_bytes([src[x * 4], src[x * 4 + 1], src[x * 4 + 2], src[x * 4 + 3]]);
                dst[x * 4] = unorm8((p & 0x3ff) as f32 / 1023.0);
                dst[x * 4 + 1] = unorm8(((p >> 10) & 0x3ff) as f32 / 1023.0);
                dst[x * 4 + 2] = unorm8(((p >> 20) & 0x3ff) as f32 / 1023.0);
                dst[x * 4 + 3] = if p >> 30 >= 2 { 255 } else { 0 };
            }
            return;
        }
    }
    /* Unhandled: leave opaque black. */
    for x in 0..w {
        dst[x * 4] = 0;
        dst[x * 4 + 1] = 0;
        dst[x * 4 + 2] = 0;
        dst[x * 4 + 3] = 255;
    }
}

/// GL_MAX_TEXTURE_SIZE (glGetIntegerv reports it): larger 2D levels are
/// GL_INVALID_VALUE.
const MAX_TEXTURE_SIZE: GLsizei = 4096;

/// A pixel transfer type enum GL ES 3.0 knows (any combination): another
/// value is GL_INVALID_ENUM, a known one in a combination the call does not
/// take GL_INVALID_OPERATION.
pub(crate) fn is_pixel_type_enum(type_: GLenum) -> bool {
    const GL_HALF_FLOAT_OES: GLenum = 0x8D61;
    matches!(
        type_,
        GL_UNSIGNED_BYTE
            | GL_BYTE
            | GL_UNSIGNED_SHORT
            | GL_SHORT
            | GL_UNSIGNED_INT
            | GL_INT
            | GL_HALF_FLOAT
            | GL_HALF_FLOAT_OES
            | GL_FLOAT
            | GL_UNSIGNED_SHORT_5_6_5
            | GL_UNSIGNED_SHORT_4_4_4_4
            | GL_UNSIGNED_SHORT_5_5_5_1
            | GL_UNSIGNED_INT_2_10_10_10_REV
            | GL_UNSIGNED_INT_10F_11F_11F_REV
            | GL_UNSIGNED_INT_5_9_9_9_REV
            | GL_UNSIGNED_INT_24_8
            | GL_FLOAT_32_UNSIGNED_INT_24_8_REV
    )
}

/// A pixel type ES 3.0 knows that the ES 2.0 surface does not take: a
/// `report-es2` build refuses it with GL_INVALID_ENUM in every unpacking call
/// (`check_format_type`: glTexImage2D, glTexSubImage2D, glTexImage3D,
/// glTexSubImage3D).
///
/// ES 2.0.25 (read from the extracted build/es_full_spec_2.0.25.txt), 3.6.2
/// Table 3.2 "TexImage2D and ReadPixels type parameter values" lists
/// UNSIGNED_BYTE, UNSIGNED_SHORT_5_6_5, UNSIGNED_SHORT_4_4_4_4 and
/// UNSIGNED_SHORT_5_5_5_1; 3.7.1 takes `type` "as described in section 3.6.2"
/// for TexImage2D, and 3.7.2 for TexSubImage2D. 2.5:
///
/// ```text
/// If a command that requires an enumerated value is passed a symbolic con-
/// stant that is not one of those specified as allowable for that command, the er-
/// ror INVALID_ENUM error is generated.
/// ```
///
/// The extensions this surface claims add types: GL_OES_texture_half_float
/// HALF_FLOAT_OES (0x8D61, not ES 3.0's HALF_FLOAT 0x140B),
/// GL_OES_depth_texture UNSIGNED_SHORT and UNSIGNED_INT, and
/// GL_OES_packed_depth_stencil UNSIGNED_INT_24_8_OES. A type one of them adds
/// is allowable, so a format it does not pair with is GL_INVALID_OPERATION
/// (the pair check), not GL_INVALID_ENUM. GL_FLOAT is GL_OES_texture_float's,
/// which this surface does not claim (`state::ALL_EXTENSION_NAMES`).
fn es3_only_pixel_type(type_: GLenum) -> bool {
    const GL_HALF_FLOAT_OES: GLenum = 0x8D61;
    let es2 = matches!(
        type_,
        GL_UNSIGNED_BYTE | GL_UNSIGNED_SHORT_5_6_5 | GL_UNSIGNED_SHORT_4_4_4_4 | GL_UNSIGNED_SHORT_5_5_5_1
    );
    let extension = type_ == GL_HALF_FLOAT_OES
        || (super::state::DEPTH_TEXTURE_EXTENSIONS
            && matches!(type_, GL_UNSIGNED_SHORT | GL_UNSIGNED_INT | GL_UNSIGNED_INT_24_8));
    crate::REPORT_ES2 && is_pixel_type_enum(type_) && !es2 && !extension
}

/// A pixel transfer format enum GL ES 3.0 knows (see [`is_pixel_type_enum`]).
pub(crate) fn is_pixel_format_enum(format: GLenum) -> bool {
    is_texture_format(format)
        || matches!(
            format,
            GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL | GL_RED_INTEGER | GL_RG_INTEGER | GL_RGB_INTEGER | GL_RGBA_INTEGER
        )
}

/// Source bytes per texel for the (format,type) pair (for advancing rows).
pub(crate) fn src_bpp(format: GLenum, type_: GLenum) -> u32 {
    if type_ == GL_UNSIGNED_SHORT_5_6_5 || type_ == GL_UNSIGNED_SHORT_4_4_4_4 || type_ == GL_UNSIGNED_SHORT_5_5_5_1 {
        return 2;
    }
    /* depth (and depth-stencil) texels: one component of the type's size;
     * the 32F + 8 pair is 8 bytes (ES 3.0 table 3.2) */
    if format == GL_DEPTH_COMPONENT || format == GL_DEPTH_STENCIL {
        return match type_ {
            GL_UNSIGNED_SHORT => 2,
            GL_FLOAT_32_UNSIGNED_INT_24_8_REV => 8,
            _ => 4,
        };
    }
    /* ES 3.0's integer, signed and packed 32-bit uploads (es3_formats) */
    if es3_formats::only_for_new_formats(format, type_) {
        return es3_formats::texel_bytes(format, type_) as u32;
    }
    let comps: u32 = match format {
        GL_RGBA => 4,
        GL_RGB => 3,
        GL_LUMINANCE_ALPHA | GL_RG => 2,
        GL_LUMINANCE | GL_ALPHA | GL_RED => 1,
        _ => 4,
    };
    comps
        * match type_ {
            GL_FLOAT => 4,
            GL_HALF_FLOAT | GL_HALF_FLOAT_OES => 2,
            _ => 1,
        }
}

/* --------------------------------------------------------------------------- */

/// Zero a table slot (`memset(t, 0, sizeof(*t))`).
fn zero_texture(t: &mut GlTexture) {
    // SAFETY: every field of GlTexture is valid when all-zero.
    unsafe { core::ptr::write_bytes(t as *mut GlTexture, 0, 1) };
}

/// A fresh texture object with the GL defaults.
fn init_texture(t: &mut GlTexture) {
    zero_texture(t);
    t.used = true;
    t.serial = context::next_object_stamp();
    t.min_filter = GL_NEAREST_MIPMAP_LINEAR; /* GL defaults */
    t.mag_filter = GL_LINEAR;
    t.wrap_s = GL_REPEAT;
    t.wrap_t = GL_REPEAT;
    t.max_level = MAX_LEVEL_DEFAULT; /* GL's default is "no clamp" */
    /* ES 3.0 table 6.10's initial values */
    t.max_level_set = 1000;
    t.min_lod = -1000.0;
    t.max_lod = 1000.0;
    t.wrap_r = GL_REPEAT;
    t.compare_mode = GL_NONE;
    t.compare_func = GL_LEQUAL;
    t.swizzle = [GL_RED as u16, GL_GREEN as u16, GL_BLUE as u16, GL_ALPHA as u16];
}

/// `GL_TEXTURE_MAX_LEVEL`'s default: the sampler is not clamped (the chain
/// this implementation can hold is `CAP_MIP_LEVELS` levels).
const MAX_LEVEL_DEFAULT: u8 = (CAP_MIP_LEVELS - 1) as u8;

#[no_mangle]
pub unsafe extern "C" fn glGenTextures(n: GLsizei, textures: *mut GLuint) {
    gl_trace!(glGenTextures);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glGenTextures");
        return;
    }
    let Some(c) = context::current() else { return };
    if textures.is_null() {
        return;
    }
    let n = n as usize;
    let mut made = 0;
    for h in 1..MAX_TEXTURES {
        if made >= n {
            break;
        }
        if !c.objs().textures[h].used {
            init_texture(&mut c.objs().textures[h]);
            // SAFETY: the application passes room for n names (GL
            // contract); a GLuint array is not assumed aligned.
            textures.add(made).write_unaligned(h as GLuint);
            made += 1;
        }
    }
    if made < n {
        /* the fixed pool is full: the rest are 0, which a client would
         * otherwise take for a usable name (upstream 7219228) */
        c.set_error(GL_OUT_OF_MEMORY, "glGenTextures");
    }
    for i in made..n {
        // SAFETY: as above.
        textures.add(i).write_unaligned(0);
    }
}

/// Before texels of texture `ti` are written (or read) in place: let every
/// kick that samples or renders into them run first, so an earlier draw
/// sees the earlier texels and a CPU write never races the GPU. Storage an
/// EGLImage shares is always settled (a sibling in another table may be
/// the one in use). The glClear waiting to be merged into the next draw
/// goes first when the texture is its target. False when the backend
/// failed to run the kicks (the write must not happen).
/// The next texel write stamp: one counter for every texture of every
/// context, so a stamp is never reused for other texels at an address that
/// was freed and handed out again. Never 0 (0 is "no stamp").
fn next_content_gen() -> u32 {
    static NEXT: Global<u32> = Global::new(0);
    // SAFETY: single-threaded by contract; no other reference is live.
    let n = unsafe { NEXT.get() };
    *n = n.wrapping_add(1).max(1);
    *n
}

/// The texels of `t` are about to change on the CPU (or its storage was laid
/// out again): give them a new stamp (`GlesTextureBind::content_gen`), which
/// tells a backend that keeps a copy of them to make it again. Storage an
/// EGLImage shares gets 0 instead: a sibling can write it without passing
/// here, so no copy of it may be kept.
fn stamp_texels(t: &mut GlTexture) {
    t.content_gen = if t.external { 0 } else { next_content_gen() };
    t.cpu_wrote = true;
}

/// Bring the depth a kick rendered into depth texture `ti` into its texels.
///
/// A draw with a depth texture at its depth attachment writes depth into the
/// backend's own depth/stencil buffer, keyed by the texture (draw.rs
/// ds_identity), not into the texels: the PowerVR's ISP stores packed D24S8
/// row-linear in its ZLS layout, the GC7000's pixel engine D24S8 row-linear
/// at its surface stride (both measured, gl_test 169). The GC7000's is not
/// a format its texture unit is known to sample; the PowerVR's TPU reads
/// its buffer as X8U24 and the V3D's TMU its own as DEPTH24_X8, so a draw
/// on those samples the buffer itself ([`depth_for_draw`]). Otherwise the
/// depth is resolved here, into the R32F texels the texture unit does
/// sample, when it is next read: before a draw samples the texture
/// (draw.rs `resolve_sampled_depth`, unless the backend samples the buffer
/// itself), and before the CPU writes it ([`settle_texture`], so a
/// sub-image keeps the rendered rest).
/// The kick that rendered it is waited for, then the backend writes the
/// level the draw rendered (`backend::depth_resolve`, which answers 1 when
/// it keeps nothing for the texture: the texels stay as they are). The
/// texels' stamp moves, so a backend copy of them is made again.
///
/// The other way round, depth the CPU writes into the texels
/// ([`note_depth_texels_written`]) goes into the backend's buffer with the
/// next draw that has the texture at its depth attachment ([`depth_seed`]).
pub(crate) fn resolve_depth(c: &mut GlContext, ti: usize) {
    let kick = framebuffer::texture_target_use(c, ti);
    let target = framebuffer::depth_target(c, ti);
    if kick == 0 || kick == target.resolved {
        return;
    }
    let Some(t) = c.objs_ref().textures.get(ti) else { return };
    let level = target.level as usize;
    if !depth_texture_rendered(t) || level >= t.chain_levels as usize || level >= CAP_MIP_LEVELS {
        return;
    }
    if backend::kick_pending(kick) && (backend::flush() != 0 || backend::finish() != 0) {
        glog!("texture {}: the kick rendering its depth did not complete; its texels keep what they held", ti);
        return;
    }
    let t = &mut c.objs().textures[ti];
    /* a deferred store is allocated by its first reader: this one */
    if !ensure_storage(t) {
        glog!("texture {}: no storage to resolve its depth into", ti);
        return;
    }
    let (w, h) = ((t.width >> level).max(1), (t.height >> level).max(1));
    let pitch = t.level_pitch_at(level as u32);
    let off = t.level_off[level] as usize;
    let bytes = (h as usize - 1) * pitch as usize * 4 + w as usize * 4;
    if off + bytes > t.mem.size {
        return;
    }
    let dst = (t.mem.cpu as *mut u8).wrapping_add(off);
    // SAFETY: `dst` is the level's storage, `h` rows of `pitch` floats
    // (checked against the allocation above); the kicks using it completed.
    let rc = unsafe { backend::depth_resolve(context::ds_object_id(t.serial, true), dst as *mut f32, w, h, pitch) };
    if rc == 0 {
        // SAFETY: the bytes just written, inside the allocation.
        unsafe { osal::cache_flush(dst as *mut c_void, bytes) };
        stamp_texels(t);
        t.levels_written = t.levels_written.max(level as u8);
    } else if rc < 0 {
        glog!("texture {}: the backend did not resolve its depth ({})", ti, rc);
    }
    framebuffer::set_depth_resolved(c, ti, kick);
}

/// A live depth texture whose rendered depth lives in the backend's buffer
/// until it is resolved ([`resolve_depth`]): GL_DEPTH_COMPONENT or
/// GL_DEPTH_STENCIL kept as R32F texels the CPU can reach, or will reach
/// once its deferred storage is allocated ([`ensure_storage`]).
fn depth_texture_rendered(t: &GlTexture) -> bool {
    t.used
        && matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
        && t.texfmt == GLES_TEXFMT_R32F
        && (!t.mem.cpu.is_null() || t.storage_deferred)
}

/// Does glTexImage2D / glTexStorage2D lay a `w` x `h` image of `format`
/// (held as `texfmt`) out without its storage (`GlTexture::storage_deferred`)?
/// A 2D depth texture of one level 0 given no pixels, on a backend that
/// samples rendered depth from its own buffer
/// (`backend::defers_depth_storage`, the V3D): glmark2's shadow map, whose
/// R32F texels nobody reads (docs/notes/v3d-depth-storage.md). Cube faces,
/// arrays, 3D, block formats, colour and any upload with data allocate at
/// once, as before.
fn defers_storage(target: GLenum, format: GLenum, texfmt: u8, w: u32, h: u32, pixels: *const c_void) -> bool {
    target == GL_TEXTURE_2D
        && matches!(format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
        && texfmt == GLES_TEXFMT_R32F
        && pixels.is_null()
        && w != 0
        && h != 0
        && backend::defers_depth_storage()
}

/// The bytes the deferred store of `t` takes: the layout
/// [`texture_alloc_storage_deferred`] recorded, made again (it depends on
/// the size and the backend's constants alone).
fn deferred_storage_bytes(t: &GlTexture) -> usize {
    let bpp = gles_texfmt_bytes(t.texfmt);
    let (pot_stack, lvl_align) = (!backend::native_layered_textures(), backend::mip_level_align());
    chain_layout_npot(t.width, t.height, 1, bpp, false, lvl_align, pot_stack, false).map_or(0, |l| l.2 as usize)
}

/// Texture `t`'s texels are about to be read or written: a store left
/// unallocated (`GlTexture::storage_deferred`) is allocated now, cleared, as
/// [`texture_alloc_laid_out`] would have at the glTexImage2D -- the same
/// bytes, the same zero texels, one pass later. False when the platform has
/// no memory for it (the caller's GL_OUT_OF_MEMORY or no-image branch); the
/// texture stays deferred. Nothing for any other texture.
pub(crate) fn ensure_storage(t: &mut GlTexture) -> bool {
    if !t.storage_deferred {
        return true;
    }
    let bytes = deferred_storage_bytes(t);
    if bytes == 0 || osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, &mut t.mem) != 0 {
        glog!("texture {}x{} (depth, {} bytes): out of GPU memory for its deferred storage", t.width, t.height, bytes);
        t.mem = OsalGpuMem::default();
        return false;
    }
    t.storage_deferred = false;
    t.cpu_wrote = !context::clear_storage(&mut t.mem);
    /* new texels at a new address: a backend copy of them is made again */
    stamp_texels(t);
    true
}

/// [`ensure_storage`] of texture slot `ti`, for a reader that holds the
/// context; a failure is logged and the reader takes its no-image branch.
pub(crate) fn ensure_storage_at(c: &mut GlContext, ti: usize) -> bool {
    match c.objs().textures.get_mut(ti) {
        Some(t) => ensure_storage(t),
        None => true,
    }
}

/// A draw is about to sample 2D texture `ti`: the depth a kick rendered into
/// it, if any, reaches the draw one of two ways. The backend samples it
/// from its own depth buffer (`backend::samples_rendered_depth`): the
/// texels are left as they are, stale, and the draw's binding carries the
/// depth object's id (the answer, `GlesTextureBind::depth_id`); the next CPU
/// read or write of the texels resolves them then (every [`settle_texture`],
/// the copies and blits). Or the backend does not, and the depth is
/// resolved into the texels now ([`resolve_depth`], as before): 0.
///
/// The kick that rendered the depth is submitted before the question
/// (`backend::flush`, no wait): the backend's record of what that job
/// stored is then complete, and the answer is read from it. That submit is
/// the one the draw's target change would make a moment later (glmark2
/// binds the canvas back right after its depth pass), and it is what the
/// resolve did before reading the buffer, less the wait.
///
/// Only the whole texture is read from the buffer: its level 0 is the level
/// rendered and its only level, so the draw samples that level alone
/// whatever its filters (draw.rs `sampling_fields`: a one-level texture
/// never samples a chain). A level above 0 rendered, or a chain, resolves.
/// The V3D's buffer is D24S8, which its texture unit reads as a 24-bit
/// depth: the same bits the resolve converts (v3d/backend.rs
/// `v3d_depth_resolve`), so a draw reads the same depth either way, less
/// the CPU's detile of the buffer and the copy of the texels for the
/// texture unit (docs/notes/pi4-glmark2-cpu.md: all of glmark2 shadow's
/// frame, 39% of refract's).
pub(crate) fn depth_for_draw(c: &mut GlContext, ti: usize) -> u32 {
    let kick = framebuffer::texture_target_use(c, ti);
    let target = framebuffer::depth_target(c, ti);
    if kick == 0 || kick == target.resolved {
        /* the draw samples the texels: a deferred store that was never
         * rendered is allocated, cleared, as it would have been */
        ensure_storage_at(c, ti);
        return 0;
    }
    if let Some(t) = c.objs_ref().textures.get(ti) {
        if depth_texture_rendered(t) && t.target == GL_TEXTURE_2D && target.level == 0 && t.levels <= 1 {
            let id = context::ds_object_id(t.serial, true);
            let (w, h) = (t.width, t.height);
            let submitted = !backend::kick_pending(kick) || backend::flush() == 0;
            if submitted && backend::samples_rendered_depth(id, w, h) {
                return id;
            }
        }
    }
    resolve_depth(c, ti);
    ensure_storage_at(c, ti);
    0
}

/// A glClear waiting to be merged into the next draw renders into texture
/// `ti` when it is the bound framebuffer's colour or depth attachment: it
/// goes out now, before a CPU access of the texels that comes after it in
/// GL's order (it then is one of the kicks waited for).
fn flush_clear_into(c: &mut GlContext, ti: usize) {
    let is_target = c.bound_framebuffer != 0
        && c.framebuffer_ref(c.bound_framebuffer).is_some_and(|fb| {
            [&fb.color, &fb.depth].into_iter().any(|a| a.type_ == GL_TEXTURE && a.name as usize == ti)
        });
    if is_target {
        draw::flush_pending_clear(c);
    }
}

/// The CPU is about to read or write levels 1.. of texture `ti`, whose last
/// glGenerateMipmap the backend made in its own image
/// (`GlTexture::chain_in_backend`): the backend writes the chain back into
/// the storage first (`backend::return_mipmap_chain`), and the storage is
/// GL's chain again. Nothing for any other texture.
pub(super) fn chain_back(c: &mut GlContext, ti: usize) {
    let Some(t) = c.objs_ref().textures.get(ti) else { return };
    if !t.used || !t.chain_in_backend {
        return;
    }
    backend::return_mipmap_chain(&draw::texture_bind_2d(t), t.chain_levels as u32);
    c.objs().textures[ti].chain_in_backend = false;
}

/// The CPU is about to read or write the texels of texture `ti`: a backend
/// whose jobs stored level 0 of a render target into its own image alone
/// writes it into the storage first (`backend::storage_current`, the V3D's
/// lazy raster store, docs/notes/v3d-lazy-raster.md).
pub(crate) fn storage_back(c: &GlContext, ti: usize) {
    let Some(t) = c.objs_ref().textures.get(ti) else { return };
    if t.used && t.target == GL_TEXTURE_2D && !t.mem.impl_.is_null() {
        backend::storage_current(&draw::texture_bind_2d(t));
    }
}

fn settle_texture(c: &mut GlContext, ti: usize) -> bool {
    flush_clear_into(c, ti);
    resolve_depth(c, ti);
    /* every CPU access of the texels finds storage (a deferred depth store) */
    if !ensure_storage(&mut c.objs().textures[ti]) {
        return false;
    }
    let t = &c.objs_ref().textures[ti];
    /* A pending kick that only SAMPLED the texture reads a copy on a backend
     * that samples texel copies (the V3D: tiled copies made when the draw
     * was recorded), so the write need not wait for it: under the async
     * swap that wait submitted the frame being drawn and waited out the
     * previous frame's render at every refill (DEMO14, uploads 21 ms). A
     * kick that renders into the texture (texture_target_use) and storage an
     * EGLImage shares still wait. */
    let sampled_pending = !backend::samples_texel_copies() && backend::kick_pending(t.last_use);
    let busy = t.external || sampled_pending || backend::kick_pending(framebuffer::texture_target_use(c, ti));
    if busy && (backend::flush() != 0 || backend::finish() != 0) {
        glog!("texture {}: the kicks using it did not complete; the update is dropped", ti);
        return false;
    }
    /* a chain the backend made in its own image comes back into the
     * storage before the storage is written (the write may be into one of
     * its levels, or into level 0 under them) */
    chain_back(c, ti);
    /* and level 0 a backend's jobs stored in its image alone */
    storage_back(c, ti);
    /* every CPU write of the texels comes through here first (a fresh
     * upload apart: `texture_write_rect` notes that one) */
    let t = &mut c.objs().textures[ti];
    stamp_texels(t);
    note_depth_texels_written(t);
    true
}

/// glBlitFramebuffer wrote texels of `level` of texture `ti` on the CPU,
/// with every kick already complete (`framebuffer::gpu_idle_for_cpu`): the
/// texels take a new stamp, so a backend copy of them is made again, and
/// the level counts as written. A depth texture's texels are newer than
/// the backend's plane now ([`note_depth_texels_written`]).
pub(crate) fn note_blit_written(c: &mut GlContext, ti: usize, level: GLint) {
    let t = &mut c.objs().textures[ti];
    stamp_texels(t);
    note_level_written(t, level);
    note_depth_texels_written(t);
}

/// The CPU wrote texels of `t` (an upload with data, a depth blit). For a
/// depth texture the depth plane a backend keeps for it is then older than
/// the texels: rendered depth reached them first ([`resolve_depth`], which
/// every such write calls before it writes), and a fresh storage gave the
/// object a new generation, whose plane holds nothing. The next draw with
/// the texture at its depth attachment hands the texels to the backend
/// ([`depth_seed`]), so it tests against them, and a later resolve does
/// not write the old plane over them.
fn note_depth_texels_written(t: &mut GlTexture) {
    if depth_texture_rendered(t) {
        t.depth_texels_newer = true;
    }
}

/// What a draw whose depth attachment is `a` hands its backend as
/// `GlesDraw::depth_texels` / `depth_texels_pitch`: the attached level's
/// texels of a depth texture the CPU wrote since the backend's plane
/// matched them ([`note_depth_texels_written`]), or (null, 0). The level's
/// rows are `ds_w` x `ds_h` (draw.rs `ds_identity` sizes the draw by the
/// same level).
pub(crate) fn depth_seed(c: &GlContext, a: &GlAttachment) -> (*const f32, u32) {
    const NONE: (*const f32, u32) = (core::ptr::null(), 0);
    if a.type_ != GL_TEXTURE {
        return NONE;
    }
    let Some(t) = c.texture_ref(a.name).filter(|t| t.depth_texels_newer && depth_texture_rendered(t)) else {
        return NONE;
    };
    let level = a.level.max(0) as usize;
    if level >= t.chain_levels as usize || level >= CAP_MIP_LEVELS || t.mem.cpu.is_null() {
        return NONE;
    }
    let (w, h) = ((t.width >> level).max(1), (t.height >> level).max(1));
    let pitch = t.level_pitch_at(level as u32);
    let off = t.level_off[level] as usize;
    let bytes = (h as usize - 1) * pitch as usize * 4 + w as usize * 4;
    if pitch < w || off + bytes > t.mem.size {
        return NONE;
    }
    ((t.mem.cpu as *const u8).wrapping_add(off) as *const f32, pitch)
}

/// The backend recorded a draw that carried the texels of the bound draw
/// framebuffer's depth texture ([`depth_seed`]): its plane holds them, and
/// later draws carry nothing until the CPU writes the texels again.
pub(crate) fn depth_seeded(c: &mut GlContext) {
    let Some(a) = c.framebuffer_ref(c.bound_framebuffer).filter(|_| c.bound_framebuffer != 0).map(|fb| fb.depth) else {
        return;
    };
    if a.type_ == GL_TEXTURE {
        if let Some(t) = c.objs().textures.get_mut(a.name as usize) {
            t.depth_texels_newer = false;
        }
    }
}

/// A draw is about to render into texture `ti` (a colour attachment of the
/// bound framebuffer). On a backend whose RGBA8888 target is B,G,R,A the
/// pixel engine writes the pixels it covers in that order and leaves the
/// rest, so an RGBA8 texture uploaded by the CPU (R,G,B,A) would end up half
/// in each: the first time, every texel is converted to B,G,R,A here and the
/// texture says so (`pe_order`), which the binding's `bgra` carries to the
/// backend and every later upload follows. A kick still sampling the old
/// texels is waited for first; the texels' stamp moves, so a backend copy
/// of them is rebuilt. An XRGB8888 image (`bgra`) is already B,G,R,X.
///
/// The GC7000 backend used to guess this from a ring of the last 8
/// render-target addresses, which forgot a texture after 8 other targets,
/// kept a freed block's address as "rendered" for the CPU upload that
/// reused it, and left a sub-image upload into a rendered texture in the
/// other order (dEQP fbo.render on the i.MX8MP, order-dependent).
pub(crate) fn take_pe_order(c: &mut GlContext, ti: usize) {
    if !backend::rt_rgba8888_is_bgra() {
        return;
    }
    let t = &c.objs_ref().textures[ti];
    if t.pe_order
        || t.bgra
        || t.texfmt != GLES_TEXFMT_RGBA8
        || t.mem.impl_.is_null()
        || matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
    {
        return;
    }
    /* Nothing the CPU wrote since the storage was cleared: the texels are
     * undefined (glTexImage2D with no data -- a render target, a layer, a
     * shadow map), and a pass over them would cost 10-100 ms for a 1080p
     * target for nothing. Only the flag changes. */
    if t.cpu_wrote {
        if backend::kick_pending(t.last_use) && (backend::flush() != 0 || backend::finish() != 0) {
            glog!("texture {}: the kicks sampling it did not complete; its texels are not converted", ti);
        }
        let t = &mut c.objs().textures[ti];
        if !t.mem.cpu.is_null() {
            // SAFETY: the texture's whole CPU mapping, RGBA8 texels (4 bytes
            // each, the padding included); no kick uses it any more.
            unsafe {
                osal::cache_invd(t.mem.cpu, t.mem.size);
                swap_red_blue(core::slice::from_raw_parts_mut(t.mem.cpu as *mut u8, t.mem.size & !3));
                osal::cache_flush(t.mem.cpu, t.mem.size);
            }
        } else if note_unconverted(ti) {
            /* no CPU view (gpu_write only): flagged without converting, so
             * texels the CPU wrote read with red and blue swapped */
            glog!("texture {}: no CPU view to convert its texels to the pixel engine's order", ti);
        }
    }
    let t = &mut c.objs().textures[ti];
    t.pe_order = true;
    stamp_texels(t);
    /* An EGLImage's storage is shared: every texture of THIS context on it,
     * and the image, take the same order. A texture of another share group
     * on the same image keeps its own flag -- a known limit (it would read
     * red and blue swapped until something re-specifies it). */
    if t.external {
        let gpu = t.mem.gpu;
        for other in c.objs().textures.iter_mut() {
            if other.external && other.mem.gpu == gpu && other.texfmt == GLES_TEXFMT_RGBA8 && !other.bgra {
                other.pe_order = true;
            }
        }
        super::egl::image_note_pe_order(gpu);
    }
}

/// True the first time for texture slot `ti` (a log line once per texture).
fn note_unconverted(ti: usize) -> bool {
    static SEEN: Global<[u64; TEXTURE_SLOTS.div_ceil(64)]> = Global::zeroed();
    // SAFETY: single-threaded by contract; no other reference is live.
    let seen = unsafe { SEEN.get() };
    let (w, bit) = (ti / 64, 1u64 << (ti % 64));
    if w >= seen.len() || seen[w] & bit != 0 {
        return false;
    }
    seen[w] |= bit;
    true
}

/// Is the texture's storage something the upload converters write (RGBA8,
/// the float texel formats or the narrow luminance/alpha ones; not
/// block-compressed, not an RGB565 image)?
fn texels_writable(t: &GlTexture) -> bool {
    t.has_image()
        && t.block_bytes == 0
        && (matches!(
            t.texfmt,
            GLES_TEXFMT_RGBA8 | GLES_TEXFMT_SRGB8_A8 | GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F
        ) || is_narrow_la(t.texfmt)
            || es3_formats::by_index(t.es3_fmt).is_some())
}

#[no_mangle]
pub unsafe extern "C" fn glActiveTexture(texture: GLenum) {
    gl_trace!(glActiveTexture);
    let Some(c) = context::current() else { return };
    if texture < GL_TEXTURE0 || texture >= GL_TEXTURE0 + CAP_COMBINED_TEXTURE_UNITS as u32 {
        c.set_error(GL_INVALID_ENUM, "glActiveTexture");
        return;
    }
    c.active_texture = texture - GL_TEXTURE0;
}

#[no_mangle]
pub unsafe extern "C" fn glBindTexture(target: GLenum, texture: GLuint) {
    gl_trace!(glBindTexture);
    let Some(c) = context::current() else { return };
    if !matches!(target, GL_TEXTURE_2D | GL_TEXTURE_CUBE_MAP | GL_TEXTURE_2D_ARRAY | GL_TEXTURE_3D)
        || es3_only_texture_target(target)
    {
        c.set_error(GL_INVALID_ENUM, "glBindTexture");
        return;
    }
    /* the slot the name lives in; a name past the pool borrows one */
    let Some(texture) = names::bind_slot(c, Kind::Texture, texture, |s| !c.objs_ref().textures[s].used) else {
        c.set_error(GL_OUT_OF_MEMORY, "glBindTexture");
        return;
    };
    if texture != 0 {
        if c.objs().textures[texture as usize].orphan {
            /* the deleted name bound again: a new object under the name;
             * the orphan moves to a free slot and the framebuffers that
             * hold it follow (framebuffer.rs, the orphan rule) */
            let free = (1..MAX_TEXTURES).find(|&i| !c.objs().textures[i].used);
            if let Some(to) = free {
                let objs = c.objs();
                // SAFETY: GlTexture is valid all-zero (see zero_texture).
                let moved = core::mem::replace(&mut objs.textures[texture as usize], unsafe { core::mem::zeroed() });
                objs.textures[to] = moved;
                framebuffer::texture_orphan_moved(c, texture, to as GLuint);
            } else {
                glog!("texture {}: no free slot for its orphan; the framebuffers follow the name", texture);
                free_texture(c, texture);
            }
        }
        let t = &mut c.objs().textures[texture as usize];
        if !t.used {
            init_texture(t);
        }
        if t.target != 0 && t.target != target {
            c.set_error(GL_INVALID_OPERATION, "glBindTexture");
            return;
        }
        t.target = target;
        if t.layers == 0 {
            t.layers = if target == GL_TEXTURE_CUBE_MAP { 6 } else { 1 };
        }
    }
    if let Some(bind) = binding_for(c, target) {
        *bind = texture;
    }
}

fn is_pot(v: u32) -> bool {
    v != 0 && (v & (v - 1)) == 0
}

/// ES 2.0's rules for a non-power-of-two texture apply -- a level above 0
/// that is not a power of two is GL_INVALID_VALUE (3.7.1), such a texture is
/// complete only clamped to the edge and without a mipmap filter (3.7.10),
/// and glGenerateMipmap of one is GL_INVALID_OPERATION -- in a build that
/// reports ES 2.0 without claiming GL_OES_texture_npot. None does since
/// 2026-10-07 (`state::NPOT_EXTENSION`); the rules stay for a GPU that would
/// have to leave the extension out. ES 3.0 has no such rules.
pub(crate) const NPOT_RESTRICTED: bool = crate::REPORT_ES2 && !super::state::NPOT_EXTENSION;

/// Does the backend hold the mip chain of a non-power-of-two texture of
/// `layers` stacked faces? Where it places every level from the GL core's
/// own offsets: the GC7000 names each level's address in its descriptor
/// (vivante/texture.rs), the V3D tiles each level into a copy laid out by
/// Mesa's v3d_setup_slices rules, which cover NPOT chains
/// (driver/src/v3d/v3d_tex.c). Those are the two backends that walk a
/// layered texture themselves. The PowerVR's TPU is handed level 0 alone and
/// finds level i+1 by its own power-of-two arithmetic: a 2D NPOT chain is
/// laid out as the chain of its power-of-two extent, as Mesa's pvr driver
/// lays a multi-level image out ([`chain_layout_npot`]). A stacked NPOT cube
/// there keeps its base level: the emulated cube samples level l as one tall
/// image of (6h) >> l rows, which is not six faces of h >> l rows once h is
/// not a power of two. GL's chain is then recorded as present and the draw
/// samples level 0.
fn npot_mip_chains(layers: u32) -> bool {
    backend::native_layered_textures() || (backend::mip_level_align() == 0 && layers <= 1)
}

/// Level-0 storage. A power-of-two texture gets room for its whole chain
/// (levels consecutive, each tightly packed at its own size), so later
/// glTexImage2D(level > 0) / glGenerateMipmap uploads need no reallocation.
/// A non-power-of-two one gets one level, until glGenerateMipmap or a level
/// above 0 asks for its chain ([`grow_npot_chain`]): a NPOT image nobody
/// mipmaps (a UI image, a glyph atlas) costs no more than its level.
fn texture_alloc_storage(t: &mut GlTexture, w: u32, h: u32, layers: u32, texfmt: u8) -> i32 {
    texture_alloc_storage_ex(t, w, h, layers, texfmt, false, false)
}

/// Lay a one-level non-power-of-two 2D texture or cube map out again with
/// room for its whole chain (GL_OES_texture_npot; ES 3.0), keeping level 0's
/// texels and record and the texture's state, where the backend holds such
/// a chain ([`npot_mip_chains`]). False when there is nothing to grow --
/// the texture already has its chain, is a power of two, is an image, a
/// block format, a 3D grid or has no CPU view, or the backend keeps one
/// level -- or the storage could not be had (`oom`). The caller has
/// settled the texture: no kick still writes the texels copied here.
fn grow_npot_chain(t: &mut GlTexture, oom: &mut bool) -> bool {
    *oom = false;
    let (w, h) = (t.width, t.height);
    if !npot_mip_chains(t.layers as u32)
        || !matches!(t.target, GL_TEXTURE_2D | GL_TEXTURE_CUBE_MAP | GL_TEXTURE_2D_ARRAY)
        || t.chain_levels != 1
        || (is_pot(w) && is_pot(h))
        || (w <= 1 && h <= 1)
        || t.mem.cpu.is_null()
        || t.block_bytes != 0
        || t.external
        || t.grid_cols != 0
    {
        return false;
    }
    let (faces_set, max_level, immutable, immutable_levels) =
        (t.faces_set, t.max_level, t.immutable, t.immutable_levels);
    // SAFETY: the whole CPU mapping of the allocation; the kicks writing it
    // completed (the caller settled it).
    unsafe { osal::cache_invd(t.mem.cpu, t.mem.size) };
    if !relayout_keeping_levels(t, w, h, t.layers as u32, t.texfmt, true) {
        *oom = true;
        return false;
    }
    (t.faces_set, t.max_level, t.immutable, t.immutable_levels) = (faces_set, max_level, immutable, immutable_levels);
    t.chain_levels > 1
}

/// Lay a chain out again for a level 0 of `w x h` while level 0 itself has
/// not been specified yet, keeping the levels already uploaded that have the
/// same size in the new chain (their texels copied across, their record
/// kept). A level uploaded before level 0 lays out the chain its size implies
/// (level 0 = it doubled `level` times), and a side of 1 there could be any
/// level-0 size: dEQP random_teximage2d.2d_5 uploads level 5 of 32x4 as 1x1
/// first (read as 32x32), then level 2 as 8x1, which shows the chain is
/// 32x4. Laying that out afresh lost level 5 (AM62P 2d_5 and 2d_7 until
/// 2026-09-25). Only for storage with a CPU view; without one the levels go.
/// False when the new storage could not be had: refused (too large, out of
/// GPU memory), the texture is left as it was, its levels and their record
/// with it (the new storage is allocated before the old one goes, so the
/// peak holds both); allocated but not cleared, the texture has the new
/// storage and layout and no level. `npot_chain` as in
/// [`texture_alloc_storage_ex`] ([`grow_npot_chain`] lays a level 0 that
/// is specified out again this way).
fn relayout_keeping_levels(t: &mut GlTexture, w: u32, h: u32, layers: u32, texfmt: u8, npot_chain: bool) -> bool {
    /* the new storage before the texture changes at all: with the old one
     * released first, a refused glGenerateMipmap left the texture without
     * level 0 and every later frame reported
     * GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT (refract at a 64 MiB carve-out) */
    let Some(layout) = storage_layout(w, h, layers, texfmt, false, npot_chain) else {
        return false;
    };
    let (_, levels, bytes, _) = layout;
    let mut storage = NewStorage::Alloc;
    if bytes > 0 && storage_made_anew(t, bytes as usize, texfmt) {
        let mut next = OsalGpuMem::ZERO;
        if !alloc_texels(w, h, layers, texfmt, levels, bytes as usize, &mut next) {
            return false;
        }
        storage = NewStorage::Given(next);
    }
    let bpp = gles_texfmt_bytes(t.texfmt) as usize;
    let keep = !t.mem.cpu.is_null() && t.texfmt == texfmt && t.layers as u32 == layers && t.block_bytes == 0;
    /* (level, offset, pitch bytes, row bytes, rows) of each level that fits */
    let mut kept: [(u32, usize, usize, usize, usize); CAP_MIP_LEVELS] = [(0, 0, 0, 0, 0); CAP_MIP_LEVELS];
    let mut n = 0;
    let mut total = 0usize;
    if keep {
        for l in 0..(t.chain_levels as u32).min(CAP_MIP_LEVELS as u32) {
            let bit = 1u16 << l;
            if t.level_defined & bit == 0 || t.level_bad & bit != 0 {
                continue;
            }
            let (ow, oh) = level_size(t, l);
            if (ow, oh) != ((w >> l).max(1), (h >> l).max(1)) {
                continue;
            }
            let pitch = t.level_pitch_at(l) as usize * bpp;
            let rows = oh as usize * layers as usize;
            kept[n] = (l, t.level_off[l as usize] as usize, pitch, ow as usize * bpp, rows);
            total += ow as usize * bpp * rows;
            n += 1;
        }
    }
    // SAFETY: the host heap; freed below on every path.
    let buf = if total > 0 { (unsafe { util::malloc(total) }) as *mut u8 } else { core::ptr::null_mut() };
    if total > 0 && buf.is_null() {
        n = 0;
    }
    let mut at = 0usize;
    let old_faces = t.level_faces;
    for &(_, off, pitch, row, rows) in &kept[..n] {
        for r in 0..rows {
            // SAFETY: the level lies inside the old storage (its layout said
            // so); `buf` holds `total` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping((t.mem.cpu as *const u8).add(off + r * pitch), buf.add(at), row);
            }
            at += row;
        }
    }
    /* the kept levels' bytes go back as they were: in the pixel engine's
     * order if the texture was (frontend4's review of ee1d5894) */
    let pe = t.pe_order;
    /* only the clear can fail here: the new storage and layout are in place
     * and hold no level */
    let ok = texture_alloc_laid_out_ex(t, w, h, layers, texfmt, layout, storage) == 0;
    t.level_defined = 0;
    if ok && !t.mem.cpu.is_null() {
        at = 0;
        let mut top = 0u32;
        for &(l, _, _, row, rows) in &kept[..n] {
            let pitch = t.level_pitch_at(l) as usize * bpp;
            let off = t.level_off[l as usize] as usize;
            for r in 0..rows {
                // SAFETY: the level lies inside the new storage (same size
                // in the new chain); `buf` holds `total` bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(buf.add(at), (t.mem.cpu as *mut u8).add(off + r * pitch), row);
                }
                at += row;
            }
            t.level_defined |= 1u16 << l;
            t.level_faces[l as usize] = old_faces[l as usize];
            top = top.max(l + 1);
        }
        t.levels = (top as u8).max(1);
        t.levels_written = t.levels;
        if n > 0 {
            t.pe_order = pe;
            t.cpu_wrote = true;
        }
        // SAFETY: the whole allocation.
        unsafe { osal::cache_flush(t.mem.cpu, t.mem.size) };
    }
    if !buf.is_null() {
        // SAFETY: from util::malloc above.
        unsafe { util::free(buf as *mut c_void) };
    }
    ok
}

/// The grid a 3D texture's slices are tiled in (`GlTexture::grid_cols`).
fn grid_cols_for(depth: u32) -> u32 {
    let mut c = 1u32;

    while (c as u64) * (c as u64) < depth as u64 {
        c += 1;
    }
    c
}

/// The levels of a 3D texture's chain: log2 of its largest side, plus one
/// (ES 3.0 3.8.14, where the depth halves with the sides), at most
/// [`MAX_3D_LEVELS`].
pub(crate) fn chain_levels_3d(w: u32, h: u32, depth: u32) -> u32 {
    u32::from(context::gl_mip_levels(w.max(depth), h)).min(MAX_3D_LEVELS as u32)
}

/// The slices level `level` of 3D texture `t` has: its depth halved
/// `level` times, at least one.
pub(crate) fn depth_at(t: &GlTexture, level: u32) -> u32 {
    pot_level_side(u32::from(t.layers), level)
}

/// Storage of a 3D texture: w x h slices, `depth` of them, with room for
/// every level of its chain ([`chain_levels_3d`]), whether its sides are
/// powers of two or not -- tiled, or simply stacked on a backend whose
/// texture unit walks the stack itself.
///
/// Stacked: a 2D array's layout, `depth` slots of a level's size at every
/// level, level l's (depth >> l) slices in its first slots. That is the
/// layout the GC7000 walks (vivante/texture.rs places level l at the end of
/// level l-1's `layers` slices, all of one SLICE stride), and the one
/// gl/framebuffer.rs renders a layer of a level into. A chain whose depth
/// outlasts its sides (4x4x16) goes on past 1x1 with 1x1 levels of the same
/// rule. The V3D refuses to sample 3D textures (v3d/texture.rs `refusal`),
/// so it has no layout of its own yet.
///
/// The grid is the PowerVR's arrangement, where the shader addresses the
/// slices and the TPU caps a texture at 16384 texels in each direction. The
/// GC7000's texture unit is told one slice's size and the SLICE stride between
/// them instead, and it would look for slice k at k * SLICE -- which a grid is
/// not. Stacked is what a 2D array already does here, and its layer stride is
/// that same pitch * height, so the two agree by construction.
///
/// The grid is level 0 alone as far as the backend knows (`chain_levels`
/// and `levels` stay 1: the draw hands the TPU that one image, and a TPU
/// told of more levels would walk them by its own power-of-two arithmetic
/// over the grid, which is not where they are). Levels 1 and up are held
/// after it, each a grid of its own slices with the same column count, at
/// `level_off[l]` (non-zero: [`grid_level_held`]), rows at
/// [`level_row_pitch`] of the grid's width. The GL core keeps their texels
/// (uploads, copies, glGenerateMipmap); the PowerVR samples level 0 until
/// its shader back end learns 3D mip levels.
fn texture_alloc_storage_3d(t: &mut GlTexture, w: u32, h: u32, depth: u32, texfmt: u8) -> i32 {
    let bpp = gles_texfmt_bytes(texfmt);
    let lvl_align = backend::mip_level_align();
    /* an image with a zero side has no texels and no chain */
    let levels3 =
        if w == 0 || h == 0 || depth == 0 { 0 } else { chain_levels_3d(w, h, depth).clamp(1, CAP_MIP_LEVELS as u32) };
    let align =
        |b: u32| if lvl_align != 0 { b.checked_add(lvl_align - 1).map(|v| v & !(lvl_align - 1)) } else { Some(b) };

    if backend::native_layered_textures() {
        /* the whole 2D chain of `depth` slots a level, then 1x1 levels while
         * the depth still halves */
        let Some((mut off, mut levels, mut bytes, chain_pitch)) =
            chain_layout_npot(w, h, depth, bpp, false, lvl_align, false, true)
        else {
            return -1;
        };
        /* GlTexture::level_pitch of a 1-wide level of a chain */
        let pitch1 = if chain_pitch != 0 { chain_pitch } else { one_wide_level_pitch(w, 1, true) };
        while u32::from(levels) < levels3 {
            let Some(at) = align(bytes) else { return -1 };
            let Some(end) = pitch1.checked_mul(depth).and_then(|n| n.checked_mul(bpp)).and_then(|n| at.checked_add(n))
            else {
                return -1;
            };
            off[levels as usize] = at;
            bytes = end;
            levels += 1;
        }
        let r = texture_alloc_laid_out(t, w, h, depth, texfmt, (off, levels, bytes, chain_pitch));
        if r == 0 {
            t.grid_cols = 0;
        }
        return r;
    }
    if depth > 1024 {
        return -1; /* beyond the TPU's texture size */
    }
    let cols = grid_cols_for(depth);
    let rows = depth.div_ceil(cols);

    if w as u64 * cols as u64 > 16384 || h as u64 * rows as u64 > 16384 {
        return -1; /* beyond the TPU's texture size */
    }
    /* level 0: one level of cols*w x rows*h, a single "layer" the size of
     * the grid; then each further level's grid */
    let (gw, gh) = (w.wrapping_mul(cols), h.wrapping_mul(rows));
    let Some((mut off, levels, mut bytes, chain_pitch)) =
        chain_layout_npot(gw, gh, 1, bpp, true, lvl_align, true, false)
    else {
        return -1;
    };
    for l in 1..levels3 {
        let (lw, lh) = (pot_level_side(w, l), pot_level_side(h, l));
        let lrows = pot_level_side(depth, l).div_ceil(cols);
        /* level_row_pitch of a one-level texture: its own pair-padded width */
        let pitch = if chain_pitch != 0 { chain_pitch } else { tex_pitch(lw * cols) };
        let Some(at) = align(bytes) else { return -1 };
        let Some(end) = pitch.checked_mul(lh * lrows).and_then(|n| n.checked_mul(bpp)).and_then(|n| at.checked_add(n))
        else {
            return -1;
        };
        off[l as usize] = at;
        bytes = end;
    }
    if texture_alloc_laid_out(t, gw, gh, 1, texfmt, (off, levels, bytes, chain_pitch)) != 0 {
        return -1;
    }
    t.width = w;
    t.height = h;
    t.layers = depth as u16;
    t.grid_cols = cols as u8;
    0
}

/// [`relayout_keeping_levels`] for a 3D texture whose level 0 has not been
/// specified yet: lay its chain out again for a level 0 of `w x h x d`,
/// keeping the levels already uploaded that have the same size in the new
/// chain (their texels copied across, slice by slice, and their record).
/// A level uploaded before level 0 implies a chain whose 1-long sides could
/// be any size: the 1x1x1 level 3 of an 8x8x4 chain reads as 8x8x8 until
/// level 2 (2x2x1) says otherwise. False when the new storage could not be
/// had.
fn relayout_3d_keeping_levels(t: &mut GlTexture, w: u32, h: u32, d: u32, texfmt: u8) -> bool {
    let bpp = t.bpp.max(1) as usize;
    let keep = !t.mem.cpu.is_null() && t.texfmt == texfmt && t.block_bytes == 0 && !t.external;
    let new_levels = chain_levels_3d(w, h, d).min(CAP_MIP_LEVELS as u32);
    /* (level, row bytes, rows) of each level that fits the new chain */
    let mut kept: [(u32, usize, u32, u32); CAP_MIP_LEVELS] = [(0, 0, 0, 0); CAP_MIP_LEVELS];
    let mut n = 0;
    let mut total = 0usize;
    if keep {
        for l in 0..new_levels {
            let bit = 1u16 << l;
            if t.level_defined & bit == 0 || t.level_bad & bit != 0 || !level_held(t, l) {
                continue;
            }
            let (lw, lh) = level_size(t, l);
            let ld = depth_at(t, l);
            if (lw, lh, ld) != (pot_level_side(w, l), pot_level_side(h, l), pot_level_side(d, l)) {
                continue;
            }
            kept[n] = (l, lw as usize * bpp, lh, ld);
            total += lw as usize * bpp * (lh * ld) as usize;
            n += 1;
        }
    }
    // SAFETY: the host heap; freed below on every path.
    let buf = if total > 0 { (unsafe { util::malloc(total) }) as *mut u8 } else { core::ptr::null_mut() };
    if total > 0 && buf.is_null() {
        n = 0;
    }
    if n > 0 {
        // SAFETY: the whole CPU mapping of the allocation.
        unsafe { osal::cache_invd(t.mem.cpu, t.mem.size) };
    }
    let mut at = 0usize;
    for &(l, row, lh, ld) in &kept[..n] {
        for z in 0..ld {
            for y in 0..lh {
                // SAFETY: the row lies inside the old storage (its layout said
                // so); `buf` holds `total` bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        (t.mem.cpu as *const u8).add(texel_offset_3d(t, l, 0, y, z)),
                        buf.add(at),
                        row,
                    );
                }
                at += row;
            }
        }
    }
    /* the kept levels' bytes go back as they were (relayout_keeping_levels) */
    let pe = t.pe_order;
    let ok = texture_alloc_storage_3d(t, w, h, d, texfmt) == 0;
    t.level_defined = 0;
    if ok && !t.mem.cpu.is_null() {
        at = 0;
        for &(l, row, lh, ld) in &kept[..n] {
            for z in 0..ld {
                for y in 0..lh {
                    // SAFETY: the level lies inside the new storage (same size
                    // in the new chain); `buf` holds `total` bytes.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            buf.add(at),
                            (t.mem.cpu as *mut u8).add(texel_offset_3d(t, l, 0, y, z)),
                            row,
                        );
                    }
                    at += row;
                }
            }
            t.level_defined |= 1u16 << l;
            if t.grid_cols == 0 {
                t.levels = t.levels.max(l as u8 + 1);
                t.levels_written = t.levels_written.max(l as u8 + 1);
            }
        }
        if n > 0 {
            t.pe_order = pe;
            t.cpu_wrote = true;
        }
        // SAFETY: the whole allocation.
        unsafe { osal::cache_flush(t.mem.cpu, t.mem.size) };
    }
    if !buf.is_null() {
        // SAFETY: from util::malloc above.
        unsafe { util::free(buf as *mut c_void) };
    }
    ok
}

/// Does the store hold the texels of level `level` of `t`: a level of its
/// chain, or of a 3D grid ([`texture_alloc_storage_3d`]), whose levels above
/// 0 the backend is not told of?
pub(crate) fn level_held(t: &GlTexture, level: u32) -> bool {
    if t.grid_cols != 0 {
        level == 0 || grid_level_held(t, level)
    } else {
        level < u32::from(t.chain_levels)
    }
}

/// A level above 0 of a 3D grid that has storage: its offset is past level
/// 0's grid, so never 0 ([`texture_alloc_storage_3d`]).
fn grid_level_held(t: &GlTexture, level: u32) -> bool {
    t.grid_cols != 0 && level >= 1 && (level as usize) < CAP_MIP_LEVELS && t.level_off[level as usize] != 0
}

/// Where slice `z` of level `level` of a 3D texture (or layer `z` of an
/// array) starts in that level's rows: (the width of the rows
/// [`texture_write_rect`] is told, column, row). A grid tiles the slices in
/// `grid_cols` columns; a stack puts them one under the other.
fn slice_origin(t: &GlTexture, level: u32, z: u32) -> (u32, u32, u32) {
    let (lw, lh) = level_size(t, level);
    if t.grid_cols != 0 {
        let cols = u32::from(t.grid_cols);
        (lw.wrapping_mul(cols), (z % cols).wrapping_mul(lw), (z / cols).wrapping_mul(lh))
    } else {
        (lw, 0, z.wrapping_mul(lh))
    }
}

/// Where slice `z` of level `level` of 3D texture `t` is a render target: its
/// byte offset in the storage and the row pitch in texels, the grid's (a slice
/// is a sub-rectangle of the grid, [`slice_origin`]). None when `t` is not
/// tiled in a grid, or has no such slice.
pub(super) fn grid_slice_target(t: &GlTexture, level: u32, z: u32) -> Option<(u32, u32)> {
    if t.grid_cols <= 1 || z >= depth_at(t, level) {
        return None;
    }
    let (rw, _, _) = slice_origin(t, level, z);
    let pitch = level_row_pitch(t, level, rw);
    let off = texel_offset_3d(t, level, 0, 0, z);
    u32::try_from(off).ok().map(|o| (o, pitch))
}

/// The row pitch, in texels, of a level of grid-tiled 3D texture `t` that is
/// `lw` wide: what [`grid_slice_target`] returns for it, whichever slice.
pub(super) fn grid_level_pitch(t: &GlTexture, lw: u32) -> u32 {
    let mut level = 0;
    while level + 1 < CAP_MIP_LEVELS as u32 && level_size(t, level).0 > lw {
        level += 1;
    }
    level_row_pitch(t, level, lw.wrapping_mul(u32::from(t.grid_cols)))
}

/// Byte offset of texel (x, y) of slice `z` of level `level` of an
/// uncompressed 3D texture `t` in its storage.
pub(crate) fn texel_offset_3d(t: &GlTexture, level: u32, x: u32, y: u32, z: u32) -> usize {
    let (rw, x0, y0) = slice_origin(t, level, z);
    let pitch = level_row_pitch(t, level, rw) as usize;
    t.level_off[level as usize] as usize + (((y0 + y) as usize) * pitch + (x0 + x) as usize) * t.bpp.max(1) as usize
}

/// [`chain_layout_npot`] without `npot_chain`: the layout a level-0 upload
/// gets, a non-power-of-two size one level (the tests' name for it).
#[cfg(test)]
pub(crate) fn chain_layout(
    w: u32,
    h: u32,
    layers: u32,
    bpp: u32,
    one_level: bool,
    lvl_align: u32,
    pot_stack: bool,
) -> Option<([u32; CAP_MIP_LEVELS], u8, u32, u32)> {
    chain_layout_npot(w, h, layers, bpp, one_level, lvl_align, pot_stack, false)
}

/// The byte layout of a w x (h * layers) chain: (level offsets, levels,
/// total bytes); None when it does not fit the u32 offsets (checked: the
/// shipped profile has no overflow checks). `pot_stack`: the backend walks a
/// stacked chain from level 0's address by the power-of-two-rounded height
/// (the PowerVR TPU, see below); false lays every level out packed.
///
/// A non-power-of-two size gets one level, unless `npot_chain` asks for its
/// whole chain ([`npot_mip_chains`]): level i is then max(w >> i, 1) x
/// max(h >> i, 1) (ES 2.0 3.7.7 with GL_OES_texture_npot, floor at every
/// step). A backend that is not `pot_stack` lays it out by the same rules as
/// a power-of-two chain. The PowerVR (`pot_stack`, `lvl_align` 0, one layer)
/// lays it out as the chain of the power-of-two extent, which is how Mesa
/// 26.2.1's pvr driver lays out every image with more than one level and
/// what its TPU walks from level 0's address ([`pot_chain_layout`]); the
/// levels are GL's count, the bytes the whole power-of-two chain's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chain_layout_npot(
    w: u32,
    h: u32,
    layers: u32,
    bpp: u32,
    one_level: bool,
    lvl_align: u32,
    pot_stack: bool,
    npot_chain: bool,
) -> Option<([u32; CAP_MIP_LEVELS], u8, u32, u32)> {
    if npot_chain
        && pot_stack
        && lvl_align == 0
        && layers == 1
        && !one_level
        && !(is_pot(w) && is_pot(h))
        && (w > 1 || h > 1)
    {
        return pot_chain_layout(w, h, bpp);
    }
    /* The layout the backend reads a chain with: one row pitch for every
     * level, and every level starting on a boundary it can fetch from; a
     * backend that wants its linear rows on `linear_row_align` bytes gets
     * that pitch (`context::linear_pitch`) */
    let row_align = backend::linear_row_align();
    let chain_pitch =
        if lvl_align != 0 || row_align != 0 { context::row_aligned(tex_pitch(w), bpp, row_align) } else { 0 };
    let multi = !one_level && ((is_pot(w) && is_pot(h)) || (npot_chain && !pot_stack)) && (w > 1 || h > 1);
    /* GlTexture::level_pitch, which every reader and writer uses */
    let level_pitch = |lw: u32| if chain_pitch != 0 { chain_pitch } else { one_wide_level_pitch(w, lw, multi) };
    let mut off = [0u32; CAP_MIP_LEVELS];
    let mut bytes: u32 = 0;
    let mut levels: u8 = 0;

    if w != 0 && h != 0 && layers != 0 {
        /* The chain belongs to ONE face: level i is `layers` faces of
         * (w >> i) x (h >> i), which is the layout gl/framebuffer.rs renders
         * into and glsl2usc's pvr_cubeUV samples. Halving the stacked height
         * instead gave a 512x512 cube twelve levels rather than ten, and the
         * two tails (3 rows, then 1) cannot hold six faces at all: the
         * backend was told to sample levels that do not exist. */
        let (mut lw, mut lfh) = (w, h);
        /* The PowerVR TPU is handed level 0's address only and finds level
         * i+1 by its own arithmetic, which Mesa spells out in
         * pvr_image_plane_setup_mip_levels: a chain is laid out from the
         * level-0 extent rounded up to a power of two. The stacked height
         * h * layers is not one for a cube map (a 512 cube is 512 x 3072,
         * next_pot 4096), so the step to the next level is that rounded
         * height halved per level; the rows inside a level stay packed.
         * Packed steps put level 1 of a 512 RGBA16F cube at 512*3072*8 where
         * the TPU reads 512*4096*8, and Qt Quick 3D's prefiltered light
         * probe sampled black above its base level (the C-era fix, measured
         * on the AM62P 2026-09-22). The GC7000 names every level's address
         * and keeps its own layout (lvl_align != 0). A texture with one level
         * has no next level to find and is not rounded (neither is Mesa's).
         * A backend whose texture unit walks the layers itself
         * (native_layered_textures: the V3D) finds each level from its own
         * offset and is not rounded either (`pot_stack` false): the rounding
         * cost a 512 cube 11,184,800 bytes where 8,388,600 hold it. */
        let chain = multi;
        let mut prow = if lvl_align == 0 && pot_stack && chain { npot2_u32(h.checked_mul(layers)?) } else { 0 };

        loop {
            if lvl_align != 0 {
                bytes = bytes.checked_add(lvl_align - 1)? & !(lvl_align - 1);
            }
            off[levels as usize] = bytes;
            /* Never fewer rows than the level holds: an array wider than it
             * is tall keeps `layers` rows per level once its height is one,
             * where the rounded height has gone below that; the TPU cannot
             * address layers smaller than a row there anyway, and the
             * uploads must stay inside the storage. */
            let rows = lfh.checked_mul(layers)?.max(prow);
            let level = level_pitch(lw).checked_mul(rows)?.checked_mul(bpp)?;
            bytes = bytes.checked_add(level)?;
            levels += 1;
            if !chain || (lw == 1 && lfh == 1) || levels as usize == CAP_MIP_LEVELS {
                break;
            }
            lw = if lw > 1 { lw / 2 } else { 1 };
            lfh = if lfh > 1 { lfh / 2 } else { 1 };
            prow /= 2;
        }
    }
    Some((off, levels, bytes, chain_pitch))
}

/// The PowerVR's layout of the mip chain of a non-power-of-two `w x h` 2D
/// image (`GlTexture::pot_chain`), as [`chain_layout_npot`] returns it.
///
/// Mesa 26.2.1's pvr driver (src/imagination/vulkan/pvr_image.c; not
/// vendored here, its rules are recorded in
/// docs/notes/pvr-render-pool.md, "What the PowerVR's texture
/// unit needs"):
/// - `pvr_image_plane_init_physical_extent`: an image with more than one mip
///   level has the physical extent `util_next_power_of_two` of its extent on
///   each axis (a one-level linear image keeps its extent);
/// - `pvr_image_plane_setup_mip_levels`: level l has the pitch
///   `cpp * u_minify(physical_width, l)` and `u_minify(physical_height, l)`
///   rows, the levels follow each other from offset 0 (`offset =
///   layer_size; layer_size += size`), and the layer is sized as if the chain
///   went to 1x1 ("the hw calculates layer strides as if a full mip chain up
///   until 1x1x1 were present");
/// - the image view keeps the API extent (`pvr_arch_tex_state.c` packs
///   `width - 1`, `height - 1` into TEXSTATE_IMAGE_WORD0) and states the
///   stride `u_minify(physical_width, base_level)` (TEXSTATE_STRIDE_IMAGE_WORD1
///   `stride`, texstate.xml).
///
/// So an 800x600 chain is laid out as the 1024x1024 one, level l's GL image
/// (800x600, 400x300, ... 1x1, floor at every step) in the top-left of that
/// level: exactly the power-of-two chain this core already lays out for the
/// PowerVR (the layout measured on the AM62P, driver/docs/gpu-recipes/
/// mesa_texture_layout.md), of the rounded extent. The levels are GL's
/// count, which is never more than the rounded chain's.
fn pot_chain_layout(w: u32, h: u32, bpp: u32) -> Option<([u32; CAP_MIP_LEVELS], u8, u32, u32)> {
    let (pw, ph) = (npot2_u32(w), npot2_u32(h));
    let (off, levels, bytes, _) = chain_layout_npot(pw, ph, 1, bpp, false, 0, true, false)?;
    Some((off, levels.min(context::gl_mip_levels(w, h)), bytes, 0))
}

/// [`texture_alloc_storage`], or one level only (`one_level`). New storage
/// is cleared (see `context::clear_storage`); storage of the same texture
/// is reused when it fits and no kick uses it (`last_use`, which
/// `note_target_use` folds the render-target use into).
/// `npot_chain`: a non-power-of-two size gets its whole chain too, where the
/// backend holds one ([`chain_layout_npot`]).
fn texture_alloc_storage_ex(
    t: &mut GlTexture,
    w: u32,
    h: u32,
    layers: u32,
    texfmt: u8,
    one_level: bool,
    npot_chain: bool,
) -> i32 {
    let Some(layout) = storage_layout(w, h, layers, texfmt, one_level, npot_chain) else {
        return -1;
    };
    texture_alloc_laid_out(t, w, h, layers, texfmt, layout)
}

/// The layout [`texture_alloc_storage_ex`] allocates, or `None` (logged)
/// when the texture is too large.
fn storage_layout(
    w: u32,
    h: u32,
    layers: u32,
    texfmt: u8,
    one_level: bool,
    npot_chain: bool,
) -> Option<([u32; CAP_MIP_LEVELS], u8, u32, u32)> {
    let bpp = gles_texfmt_bytes(texfmt);
    let pot_stack = !backend::native_layered_textures();
    let lvl_align = backend::mip_level_align();
    let npot_chain = npot_chain && npot_mip_chains(layers);
    let layout = chain_layout_npot(w, h, layers, bpp, one_level, lvl_align, pot_stack, npot_chain);
    if layout.is_none() {
        glog!("texture {}x{} x{} fmt {}: too large", w, h, layers, texfmt);
    }
    layout
}

/// [`texture_alloc_storage`] of a level 0 that [`defers_storage`]: the
/// layout is laid out and recorded, the storage is not allocated
/// (`GlTexture::storage_deferred`) until [`ensure_storage`].
fn texture_alloc_storage_deferred(t: &mut GlTexture, w: u32, h: u32, texfmt: u8) -> i32 {
    let bpp = gles_texfmt_bytes(texfmt);
    let (pot_stack, lvl_align) = (!backend::native_layered_textures(), backend::mip_level_align());
    let Some(layout) = chain_layout_npot(w, h, 1, bpp, false, lvl_align, pot_stack, false) else {
        glog!("texture {}x{} fmt {}: too large", w, h, texfmt);
        return -1;
    };
    texture_alloc_laid_out_ex(t, w, h, 1, texfmt, layout, NewStorage::Deferred)
}

/// [`texture_alloc_storage_ex`] with the layout already made: (level
/// offsets, levels, total bytes, chain pitch) as [`chain_layout_npot`]
/// returns it, or a 3D texture's ([`texture_alloc_storage_3d`]).
fn texture_alloc_laid_out(
    t: &mut GlTexture,
    w: u32,
    h: u32,
    layers: u32,
    texfmt: u8,
    layout: ([u32; CAP_MIP_LEVELS], u8, u32, u32),
) -> i32 {
    texture_alloc_laid_out_ex(t, w, h, layers, texfmt, layout, NewStorage::Alloc)
}

/// What [`texture_alloc_laid_out_ex`] puts under the layout when the
/// texture's storage has to be made anew ([`storage_made_anew`]).
#[derive(Clone, Copy)]
enum NewStorage {
    /// The old storage is released, then the new one allocated: a refusal
    /// leaves the texture without storage.
    Alloc,
    /// Nothing is allocated (nor cleared) but left to [`ensure_storage`]
    /// ([`texture_alloc_storage_deferred`]).
    Deferred,
    /// Storage the caller allocated for the layout before it touched the
    /// texture ([`relayout_keeping_levels`]); it always replaces the old one.
    Given(OsalGpuMem),
}

/// Whether `t` needs new storage for `bytes` of `texfmt`: its own is too
/// small, missing, an image's, of another format or a block format, or a
/// pending kick still samples it (or renders into it: the caller folded
/// that into `last_use`).
fn storage_made_anew(t: &GlTexture, bytes: usize, texfmt: u8) -> bool {
    t.mem.size < bytes
        || t.mem.impl_.is_null()
        || t.external
        || t.texfmt != texfmt
        || t.block_bytes != 0
        || backend::kick_pending(t.last_use)
}

/// Allocate `bytes` of texture storage into `mem`; false (logged) when the
/// heap refuses them.
fn alloc_texels(w: u32, h: u32, layers: u32, texfmt: u8, levels: u8, bytes: usize, mem: &mut OsalGpuMem) -> bool {
    if osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, mem) == 0 {
        return true;
    }
    /* the C prints the byte count as unsigned (truncated) */
    glog!(
        "texture {}x{} x{} fmt {} ({} levels, {} bytes): out of GPU memory",
        w,
        h,
        layers,
        texfmt,
        levels,
        bytes as u32
    );
    false
}

/// [`texture_alloc_laid_out`]; `storage`: what storage that has to be made
/// anew becomes. Storage of the texture that is kept stays.
fn texture_alloc_laid_out_ex(
    t: &mut GlTexture,
    w: u32,
    h: u32,
    layers: u32,
    texfmt: u8,
    layout: ([u32; CAP_MIP_LEVELS], u8, u32, u32),
    storage: NewStorage,
) -> i32 {
    let bpp = gles_texfmt_bytes(texfmt);
    let pot_stack = !backend::native_layered_textures();
    let lvl_align = backend::mip_level_align();
    let (level_off, levels, bytes, chain_pitch) = layout;
    let bytes = bytes as usize;
    let mut cleared = true;
    let mut fresh = false;
    t.level_off = level_off;
    t.chain_pitch = chain_pitch;
    /* the PowerVR's NPOT chain, laid out as its power-of-two extent's
     * (pot_chain_layout) */
    t.pot_chain = pot_stack && lvl_align == 0 && levels > 1 && !(is_pot(w) && is_pot(h));
    if matches!(storage, NewStorage::Given(_)) || storage_made_anew(t, bytes, texfmt) {
        context::release_storage(&mut t.mem, &mut t.external); /* an image's texture gets its own storage again */
        t.bgra = false;
        t.pe_order = false;
        t.chain_in_backend = false; /* the backend dropped its image with the storage */
        /* deferred: nothing allocated, nothing to clear; ensure_storage does
         * both when something first reads or writes the texels */
        t.storage_deferred = matches!(storage, NewStorage::Deferred) && bytes > 0;
        match storage {
            NewStorage::Given(mem) => t.mem = mem,
            NewStorage::Alloc if bytes > 0 => {
                if !alloc_texels(w, h, layers, texfmt, levels, bytes, &mut t.mem) {
                    return -1;
                }
            }
            NewStorage::Alloc | NewStorage::Deferred => {}
        }
        /* pool memory holds what its last user left: GL leaves new texels
         * undefined, but they must not be another object's (one pass over
         * the new storage per allocation) */
        cleared = t.storage_deferred || context::clear_storage(&mut t.mem);
        fresh = true;
    }
    t.storage_gen = context::respecified(t.serial, true);
    /* laid out again, and a fresh upload skips settle_texture */
    stamp_texels(t);
    /* nothing the CPU wrote is in new, cleared storage yet (the uploads that
     * follow set it again, `texture_write_rect`) */
    t.cpu_wrote = !(fresh && cleared);
    t.width = w;
    t.height = h;
    t.layers = layers as u16;
    t.grid_cols = 0;
    t.texfmt = texfmt;
    t.bpp = bpp as u8;
    t.chain_levels = if levels != 0 { levels } else { 1 };
    t.levels = 1;
    /* the layout is level 0's image; one with a zero side has no texels to
     * sample and leaves the texture incomplete (ES 2.0 3.7.10) */
    t.level_defined = 1;
    t.level_bad = u16::from(w == 0 || h == 0);
    /* fresh storage: no cube face has been specified at this size yet */
    t.faces_set = 0;
    t.level_faces = [0; CAP_MIP_LEVELS];
    t.levels_written = 1;
    t.max_level = MAX_LEVEL_DEFAULT;
    t.immutable = false;
    /* the storage exists and is described; the caller reports the platform's
     * failed write as an error */
    if cleared {
        0
    } else {
        -1
    }
}

/// Source row pitch of a (format,type) upload of w texels under the current
/// unpack state: GL_UNPACK_ROW_LENGTH texels when it is set (w otherwise),
/// rounded up to GL_UNPACK_ALIGNMENT.
fn src_row_bytes(c: &GlContext, mut w: u32, format: GLenum, type_: GLenum) -> usize {
    let a = c.unpack_alignment as usize;

    if c.unpack_row_length > 0 {
        w = c.unpack_row_length as u32;
    }
    (w as usize).wrapping_mul(src_bpp(format, type_) as usize).wrapping_add(a).wrapping_sub(1) & !a.wrapping_sub(1)
}

/// Source bytes from one slice of an upload to the next: GL_UNPACK_IMAGE_HEIGHT
/// rows when it is set, the upload's height otherwise.
fn src_slice_bytes(c: &GlContext, w: u32, h: u32, format: GLenum, type_: GLenum) -> usize {
    let rows = if c.unpack_image_height > 0 { c.unpack_image_height as usize } else { h as usize };
    src_row_bytes(c, w, format, type_).saturating_mul(rows)
}

/// Where the first texel an upload reads lies from `pixels`: the skipped
/// images, rows and pixels of the unpack state (ES 3.0 3.7.4).
fn unpack_skip_bytes(c: &GlContext, w: u32, h: u32, format: GLenum, type_: GLenum) -> usize {
    src_slice_bytes(c, w, h, format, type_)
        .saturating_mul(c.unpack_skip_images as usize)
        .saturating_add(src_row_bytes(c, w, format, type_).saturating_mul(c.unpack_skip_rows as usize))
        .saturating_add((src_bpp(format, type_) as usize).saturating_mul(c.unpack_skip_pixels as usize))
}

/// The bytes an upload of `w x h x d` (format, type) texels reads from its
/// source under the unpack state, from the first texel it reads: whole rows
/// and slices up to the last, and the last row's texels. 0 when any size is
/// 0 or negative.
fn unpack_bytes(c: &GlContext, w: GLsizei, h: GLsizei, d: GLsizei, format: GLenum, type_: GLenum) -> usize {
    if w <= 0 || h <= 0 || d <= 0 {
        return 0;
    }
    let row = src_row_bytes(c, w as u32, format, type_);
    let slice = src_slice_bytes(c, w as u32, h as u32, format, type_);
    slice
        .saturating_mul(d as usize - 1)
        .saturating_add(row.saturating_mul(h as usize - 1))
        .saturating_add((w as usize).saturating_mul(src_bpp(format, type_) as usize))
}

/// The bytes of one datum of `type_`: what a PIXEL_UNPACK_BUFFER offset
/// must be a multiple of (ES 3.0 3.7.1).
fn type_datum_bytes(type_: GLenum) -> usize {
    match type_ {
        GL_UNSIGNED_SHORT_5_6_5
        | GL_UNSIGNED_SHORT_4_4_4_4
        | GL_UNSIGNED_SHORT_5_5_5_1
        | GL_HALF_FLOAT
        | GL_HALF_FLOAT_OES
        | GL_SHORT
        | GL_UNSIGNED_SHORT => 2,
        GL_FLOAT
        | GL_INT
        | GL_UNSIGNED_INT
        | GL_UNSIGNED_INT_2_10_10_10_REV
        | GL_UNSIGNED_INT_10F_11F_11F_REV
        | GL_UNSIGNED_INT_5_9_9_9_REV
        | GL_UNSIGNED_INT_24_8 => 4,
        GL_FLOAT_32_UNSIGNED_INT_24_8_REV => 8,
        _ => 1,
    }
}

/// `pixels` of an uncompressed upload, resolved against PIXEL_UNPACK_BUFFER
/// (ES 3.0 3.7.1): GL_INVALID_OPERATION when the buffer is mapped, when the
/// offset is not a multiple of the type's datum, or when the texels would
/// be read past the buffer's end. The size was passed as 0 until
/// 2026-09-24, so no read was bounded: dEQP-GLES3
/// negative_api.texture.teximage3d_invalid_buffer_target unpacked 64 x 64 x
/// 64 texels from a 512-byte buffer and the process died.
#[allow(clippy::too_many_arguments)]
fn unpack_source(
    c: &mut GlContext,
    pixels: *const c_void,
    w: GLsizei,
    h: GLsizei,
    d: GLsizei,
    format: GLenum,
    type_: GLenum,
    whom: &str,
) -> Option<*const c_void> {
    if c.bound_pixel_unpack_buffer != 0 && !(pixels as usize).is_multiple_of(type_datum_bytes(type_)) {
        c.set_error(GL_INVALID_OPERATION, whom);
        return None;
    }
    /* the skips move the first texel read; the buffer must hold it all */
    let skip = if w > 0 && h > 0 { unpack_skip_bytes(c, w as u32, h as u32, format, type_) } else { 0 };
    let bytes = match unpack_bytes(c, w, h, d, format, type_) {
        0 => 0,
        n => n.saturating_add(skip),
    };
    let resolved = super::buffer::pixel_buffer_resolve(c, false, pixels, bytes, whom)?;
    Some(if resolved.is_null() || bytes == 0 {
        resolved
    } else {
        (resolved as *const u8).wrapping_add(skip) as *const c_void
    })
}

/// Fold "a kick renders into it" into the texture's `last_use`, so the
/// storage decisions that look at `last_use` see it too.
fn note_target_use(c: &mut GlContext, ti: usize) {
    let tu = framebuffer::texture_target_use(c, ti);
    if backend::kick_pending(tu) {
        c.objs().textures[ti].last_use = tu;
    }
}

/// Tell the backend that glTexImage2D / glTexSubImage2D just stored the
/// caller's rows into a box of a 2D texture's level
/// (`backend::texture_uploaded`), when those rows ARE the stored texels:
/// GL_RGBA / GL_UNSIGNED_BYTE into storage with no red/blue swap (an XRGB8888
/// image stores B,G,R,X). A backend that keeps a copy of the texels can then
/// build it from the caller's (cacheable) memory instead of reading the
/// storage back: the V3D tiles 6.2 times faster from a cacheable source
/// (docs/online-shaderc/v3d-backend.md, the second tiling bench, caa6e91a).
/// A 2D texture only (the binding is the one a sampler2D draw gives it);
/// every other write reaches the backend only through `content_gen`. Called
/// after the write succeeded and the level's stamp was taken.
///
/// # Safety
/// `pixels` holds the h rows the write just read, at the current unpack pitch.
#[allow(clippy::too_many_arguments)]
unsafe fn note_uploaded(
    c: &mut GlContext,
    ti: usize,
    level: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pixels: *const c_void,
    format: GLenum,
    type_: GLenum,
) {
    let stride = src_row_bytes(c, w, format, type_) as u32;
    let t = &c.objs_ref().textures[ti];
    if format == GL_RGBA && type_ == GL_UNSIGNED_BYTE && !t.bgra && !t.pe_order && t.target == GL_TEXTURE_2D {
        // SAFETY: `pixels` holds the h rows of `stride` bytes the write just read.
        unsafe { backend::texture_uploaded(&super::draw::texture_bind_2d(t), level, x, y, w, h, pixels, stride) };
    }
}

/// The row pitch, in texels, of `level`, whose rows are `lw` texels wide (a
/// 3D texture's grid of slices is wider than its level): the level's own,
/// or the power-of-two chain's of a PowerVR NPOT chain
/// ([`GlTexture::level_pitch_at`]).
fn level_row_pitch(t: &GlTexture, level: u32, lw: u32) -> u32 {
    if t.pot_chain {
        t.level_pitch_at(level)
    } else {
        t.level_pitch(lw)
    }
}

/// Write a w x h block of texels at (x, y) of a level (converted to
/// RGBA8888 by convert_row). lw is the level's width; `ti` the texture's
/// table index. False when the platform failed a write (the texels are
/// then partly stale).
///
/// # Safety
/// `pixels` must hold h rows of w (format,type) texels at the current unpack
/// row pitch, (x, y, w, h) must lie inside the level's storage, and the
/// storage must be [`texels_writable`].
unsafe fn texture_write_rect(
    c: &mut GlContext,
    ti: usize,
    level: u32,
    lw: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pixels: *const c_void,
    format: GLenum,
    type_: GLenum,
) -> bool {
    /* every caller settled the texture; a deferred store is allocated here
     * all the same rather than written through a null mapping */
    if !ensure_storage(&mut c.objs().textures[ti]) {
        return false;
    }
    let pitch = level_row_pitch(&c.objs().textures[ti], level, lw) as usize; /* rows are pair-padded */
    let mut ok = true;
    let sbpp = src_bpp(format, type_) as usize;
    let srow = src_row_bytes(c, w, format, type_);
    let t = &mut c.objs().textures[ti];
    t.cpu_wrote = true; /* a fresh upload does not pass settle_texture */
    note_depth_texels_written(t);
    let bpp = if t.bpp != 0 { t.bpp as usize } else { RGBA8_BPP as usize };
    let off = t.level_off[level as usize] as usize;
    let src = pixels as *const u8;
    let (x, y) = (x as usize, y as usize);

    if let Some(e) = es3_formats::by_index(t.es3_fmt).filter(|_| t.block_bytes == 0) {
        /* an ES 3.0 sized format of es3_formats: the application's texels
         * decoded and encoded into the storage class chosen at specification,
         * a chunk of a row at a time. A pair the format does not take was
         * refused by the entry point; one that slips through leaves the
         * texels as they were. */
        const EROW_MAX: u32 = 64;
        let mut row = [0u8; EROW_MAX as usize * 16];
        let gpu_write = osal::ops().and_then(|o| o.gpu_write);
        let texfmt = t.texfmt;

        for j in 0..h as usize {
            let mut x0 = 0u32;
            while x0 < w {
                let n = (w - x0).min(EROW_MAX);
                let doff = off + ((y + j) * pitch + x + x0 as usize) * bpp;
                // SAFETY: n texels of the application's row j from texel x0.
                let s = core::slice::from_raw_parts(src.add(j * srow + x0 as usize * sbpp), n as usize * sbpp);
                if es3_formats::convert_row(e, texfmt, s, format, type_, n as usize, &mut row) {
                    if !t.mem.cpu.is_null() {
                        // SAFETY: row y + j of the level holds w texels from x
                        // (caller's contract); `row` holds the n converted ones.
                        core::ptr::copy_nonoverlapping(
                            row.as_ptr(),
                            (t.mem.cpu as *mut u8).add(doff),
                            n as usize * bpp,
                        );
                    } else if let Some(gpu_write) = gpu_write {
                        // SAFETY: `row` holds n converted texels; the OSAL bounds the write.
                        ok &= gpu_write(&mut t.mem, doff, row.as_ptr() as *const c_void, n as usize * bpp) == 0;
                    } else {
                        ok = false;
                    }
                }
                x0 += EROW_MAX;
            }
        }
        if !t.mem.cpu.is_null() {
            // SAFETY: the flushed range is the rows just written.
            osal::cache_flush(
                (t.mem.cpu as *mut u8).add(off + (y * pitch + x) * bpp) as *mut c_void,
                ((h as usize).wrapping_sub(1) * pitch + w as usize) * bpp,
            );
        }
        return ok;
    }
    if matches!(t.texfmt, GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F) {
        /* float texels: converted a row at a time through the CPU view (or
         * a stack row when there is none) */
        const FROW_MAX: u32 = 64;
        let mut row = [0u8; FROW_MAX as usize * 8];
        let gpu_write = osal::ops().and_then(|o| o.gpu_write);

        for j in 0..h as usize {
            let mut x0 = 0u32;
            while x0 < w {
                let n = (w - x0).min(FROW_MAX);
                let doff = off + ((y + j) * pitch + x + x0 as usize) * bpp;
                // SAFETY: n texels of the application's row j from texel x0.
                let s = core::slice::from_raw_parts(src.add(j * srow + x0 as usize * sbpp), n as usize * sbpp);
                convert_row_float(&mut row, s, n, format, type_, t.texfmt);
                if !t.mem.cpu.is_null() {
                    // SAFETY: row y + j of the level holds w texels from x
                    // (caller's contract); `row` holds the n converted ones.
                    core::ptr::copy_nonoverlapping(row.as_ptr(), (t.mem.cpu as *mut u8).add(doff), n as usize * bpp);
                } else if let Some(gpu_write) = gpu_write {
                    // SAFETY: `row` holds n converted texels; the OSAL bounds the write.
                    ok &= gpu_write(&mut t.mem, doff, row.as_ptr() as *const c_void, n as usize * bpp) == 0;
                } else {
                    ok = false;
                }
                x0 += FROW_MAX;
            }
        }
        if !t.mem.cpu.is_null() {
            // SAFETY: the flushed range is the rows just written.
            osal::cache_flush(
                (t.mem.cpu as *mut u8).add(off + (y * pitch + x) * bpp) as *mut c_void,
                ((h as usize).wrapping_sub(1) * pitch + w as usize) * bpp,
            );
        }
        return ok;
    }
    if is_narrow_la(t.texfmt) {
        /* L8/A8/LA8: each chunk converted to RGBA8 on the stack, then packed
         * to its 1 or 2 bytes a texel */
        const NROW_MAX: u32 = 64;
        let mut rgba = [0u8; NROW_MAX as usize * 4];
        let mut row = [0u8; NROW_MAX as usize * 2];
        let gpu_write = osal::ops().and_then(|o| o.gpu_write);
        let texfmt = t.texfmt;

        for j in 0..h as usize {
            let mut x0 = 0u32;
            while x0 < w {
                let n = (w - x0).min(NROW_MAX);
                let doff = off + ((y + j) * pitch + x + x0 as usize) * bpp;
                // SAFETY: n texels of the application's row j from texel x0.
                let s = core::slice::from_raw_parts(
                    src.add(j * srow + x0 as usize * sbpp),
                    src_used_bytes(n, format, type_),
                );
                convert_row(&mut rgba, s, n, format, type_);
                pack_narrow_la(&mut row, &rgba, n as usize, texfmt);
                if !t.mem.cpu.is_null() {
                    // SAFETY: row y + j of the level holds w texels from x
                    // (caller's contract); `row` holds the n packed ones.
                    core::ptr::copy_nonoverlapping(row.as_ptr(), (t.mem.cpu as *mut u8).add(doff), n as usize * bpp);
                } else if let Some(gpu_write) = gpu_write {
                    // SAFETY: `row` holds n packed texels; the OSAL bounds the write.
                    ok &= gpu_write(&mut t.mem, doff, row.as_ptr() as *const c_void, n as usize * bpp) == 0;
                } else {
                    ok = false;
                }
                x0 += NROW_MAX;
            }
        }
        if !t.mem.cpu.is_null() {
            // SAFETY: the flushed range is the rows just written.
            osal::cache_flush(
                (t.mem.cpu as *mut u8).add(off + (y * pitch + x) * bpp) as *mut c_void,
                ((h as usize).wrapping_sub(1) * pitch + w as usize) * bpp,
            );
        }
        return ok;
    }
    /* an XRGB8888 image stores B,G,R,X, and a texture the pixel engine
     * rendered into B,G,R,A (`pe_order`) */
    let bgra = t.bgra || t.pe_order;
    if !t.mem.cpu.is_null() {
        // SAFETY: the level lies inside the texture's CPU mapping.
        let base = (t.mem.cpu as *mut u8).add(off);
        let used = src_used_bytes(w, format, type_);

        for j in 0..h as usize {
            // SAFETY: row y + j of the level holds w texels from x (caller's
            // contract); the application's row j holds the bytes convert_row reads.
            let d = core::slice::from_raw_parts_mut(base.add(((y + j) * pitch + x) * 4), w as usize * 4);
            let s = core::slice::from_raw_parts(src.add(j * srow), used);
            convert_row(d, s, w, format, type_);
            if bgra {
                swap_red_blue(d);
            }
        }
        // SAFETY: the flushed range is the rows just written.
        osal::cache_flush(
            base.add((y * pitch + x) * 4) as *mut c_void,
            ((h as usize).wrapping_sub(1) * pitch + w as usize) * 4,
        );
    } else if let Some(gpu_write) = osal::ops().and_then(|o| o.gpu_write) {
        /* No CPU mapping: convert through a small stack buffer, a chunk of
         * a row at a time. Keep it small: this runs on kernel/RTOS thread
         * stacks (a 4096-texel buffer overflowed the 16 KiB arm64 kernel
         * stack in the pvrlab harness and rebooted the board). */
        const ROW_MAX: u32 = 64;
        let mut row = [0u8; ROW_MAX as usize * 4];
        let chunk = w.min(ROW_MAX);

        for j in 0..h as usize {
            let mut x0 = 0u32;
            while x0 < w {
                let n = (w - x0).min(chunk);
                // SAFETY: n texels of the application's row j from texel x0.
                let s = core::slice::from_raw_parts(
                    src.add(j * srow + x0 as usize * sbpp),
                    src_used_bytes(n, format, type_),
                );
                convert_row(&mut row, s, n, format, type_);
                if bgra {
                    swap_red_blue(&mut row[..n as usize * 4]);
                }
                // SAFETY: `row` holds n converted texels; the OSAL bounds the write.
                ok &= gpu_write(
                    &mut t.mem,
                    off + ((y + j) * pitch + x + x0 as usize) * 4,
                    row.as_ptr() as *const c_void,
                    n as usize * 4,
                ) == 0;
                x0 += chunk;
            }
        }
    } else {
        ok = false; /* no way to reach the storage */
    }
    ok
}

fn swap_red_blue(texels: &mut [u8]) {
    for px in texels.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
}

/// Width/height of a level of t.
pub(crate) fn level_size(t: &GlTexture, level: u32) -> (u32, u32) {
    ((t.width >> level).max(1), (t.height >> level).max(1))
}

/// A level GL has (glTexStorage*, or a glTexImage* that fit its chain) but
/// the backend is not told of: a chain the backend cannot hold (a NPOT array
/// or cube on the PowerVR, `npot_mip_chains`), whose sub-images are checked
/// and then dropped, as its images are; or a level above 0 of a 3D grid,
/// which the store does hold ([`grid_level_held`]).
fn recorded_unheld(t: &GlTexture, level: GLint) -> bool {
    level as u32 >= t.levels as u32
        && level < 16
        && t.level_defined & (1u16 << level) != 0
        && t.level_bad & (1u16 << level) == 0
}

fn is_texture_format(f: GLenum) -> bool {
    matches!(f, GL_RGBA | GL_RGB | GL_LUMINANCE | GL_ALPHA | GL_LUMINANCE_ALPHA | GL_RED | GL_RG | GL_BGRA_EXT)
}

/// GL_MAX_3D_TEXTURE_SIZE, and the levels a 3D image may have (log2 + 1).
const MAX_3D_SIDE: GLsizei = 256;
const MAX_3D_LEVELS: GLint = 9;

/// The shape rules every glTex*Image2D-family call shares: a level in range,
/// a border of 0, a size in range, and square faces for a cube map. Returns
/// the GL error, or GL_NO_ERROR.
///
/// These come before the texture is looked up and before anything is
/// allocated, so a malformed call is reported as GL says whether or not the
/// rest of the call is something this core implements.
fn image_shape_error(target: GLenum, level: GLint, border: GLint, width: GLsizei, height: GLsizei) -> GLenum {
    if level < 0 || level >= CAP_MIP_LEVELS as GLint || border != 0 {
        return GL_INVALID_VALUE;
    }
    /* GL_MAX_TEXTURE_SIZE, and GL_MAX_CUBE_MAP_TEXTURE_SIZE for a face:
     * state.rs reports MAX_TEXTURE_SIZE for both. */
    if !(0..=MAX_TEXTURE_SIZE).contains(&width) || !(0..=MAX_TEXTURE_SIZE).contains(&height) {
        return GL_INVALID_VALUE;
    }
    if is_face_target(target) && width != height {
        return GL_INVALID_VALUE; /* a cube map face is square */
    }
    GL_NO_ERROR
}

/// GL_OES_texture_half_float's type and ES 3.0's are the same texels under two
/// names (0x8D61 and 0x140B). Everything that unpacks a half takes both: the
/// extension is advertised, and while only the ES 3.0 name was recognised a
/// GL_HALF_FLOAT_OES upload was accepted and then read as unsigned bytes --
/// a luminance-alpha pair of 1.0 and 0.5 landed as (0, 0, 0, 255).
fn is_half_float(type_: GLenum) -> bool {
    type_ == GL_HALF_FLOAT || type_ == GL_HALF_FLOAT_OES
}

/// The (format, type) pairs glTexImage2D/glTexSubImage2D can unpack (GL ES
/// 2.0 table 3.4 plus the ES 3.0 RED/RG and float uploads this core takes).
fn valid_format_type(format: GLenum, type_: GLenum) -> bool {
    /* ES 3.0 table 3.2's pairs of the integer, signed and packed formats
     * (never, under report-es2) */
    if es3_formats::transfer_pair_valid(format, type_) {
        return true;
    }
    match type_ {
        GL_UNSIGNED_BYTE => is_texture_format(format),
        GL_UNSIGNED_SHORT_5_6_5 => format == GL_RGB,
        GL_UNSIGNED_SHORT_4_4_4_4 | GL_UNSIGNED_SHORT_5_5_5_1 => format == GL_RGBA,
        /* GL_EXT_texture_format_BGRA8888 defines BGRA with UNSIGNED_BYTE and
         * nothing else: there is no half or float BGRA in it. */
        GL_HALF_FLOAT | GL_HALF_FLOAT_OES | GL_FLOAT => is_texture_format(format) && format != GL_BGRA_EXT,
        _ => false,
    }
}

/// Copy a w x h block of the bound framebuffer (window x, y; bottom-up) into
/// the level's (dx, dy), through glReadPixels one 64-texel chunk at a time.
/// `format` (the texture's) drops channels the framebuffer has and the
/// texture does not, as GL specifies for glCopyTex*Image2D. Pixels outside
/// the framebuffer come out as zero. False when a write failed.
///
/// # Safety
/// (dx, dy, w, h) must lie inside the level's storage, which must be
/// [`texels_writable`].
unsafe fn texture_copy_from_fb(
    c: &mut GlContext,
    ti: usize,
    level: u32,
    lw: u32,
    dx: u32,
    dy: u32,
    x: GLint,
    y: GLint,
    w: u32,
    h: u32,
    format: GLenum,
) -> bool {
    /* One chunk at a time, but the GPU is made idle -- and the target brought
     * back into the CPU's view -- ONCE for the whole copy. Through
     * glReadPixels every chunk would submit the frame, wait for the fence and
     * walk the whole surface again: a screen-sized copy (Quick 3D's
     * SCREEN_TEXTURE, which custommaterial samples every frame) is hundreds
     * of rows, and that cost turned one frame into seconds. */
    const CHUNK: u32 = 1024;
    let mut px = [0u8; CHUNK as usize * 4];
    let mut ok = true;

    /* Lost GPU work leaves the texture as it was, with no GL error: none in
     * ES 2.0/3.0 says it, and GL_OUT_OF_MEMORY (what `false` raises) would
     * be a false one. The backend counted it. */
    if !framebuffer::read_pixels_sync(c) {
        return true;
    }
    for j in 0..h {
        let mut x0 = 0u32;
        while x0 < w {
            let n = (w - x0).min(CHUNK);

            px[..n as usize * 4].fill(0);
            gl_trace!(glReadPixels); /* the C calls the entry point */
            // SAFETY: px holds CHUNK >= n RGBA texels and one row is read.
            unsafe {
                framebuffer::read_pixels(
                    c,
                    x.wrapping_add(x0 as GLint),
                    y.wrapping_add(j as GLint),
                    n as GLsizei,
                    1,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_mut_ptr() as *mut c_void,
                    false, /* synced once for the whole copy */
                    1,     /* a GL-to-GL copy never sees the pack state */
                    0,
                );
            }
            for p in px[..n as usize * 4].as_chunks_mut::<4>().0 {
                match format {
                    GL_RGB => p[3] = 0xff,
                    GL_LUMINANCE => {
                        p[1] = p[0];
                        p[2] = p[0];
                        p[3] = 0xff;
                    }
                    GL_LUMINANCE_ALPHA => {
                        p[1] = p[0];
                        p[2] = p[0];
                    }
                    GL_ALPHA => {
                        p[0] = 0;
                        p[1] = 0;
                        p[2] = 0;
                    }
                    _ => {}
                }
            }
            ok &= texture_write_rect(
                c,
                ti,
                level,
                lw,
                dx + x0,
                dy + j,
                n,
                1,
                px.as_ptr() as *const c_void,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
            );
            x0 += CHUNK;
        }
    }
    ok
}

/// An ES 3.0 *_INTEGER format under ES 2.0 reporting: no such enum there,
/// so GL_INVALID_ENUM as before 2026-09-24 (ES 3.0 makes it a known enum).
fn es2_integer_format(format: GLenum) -> bool {
    crate::REPORT_ES2 && matches!(format, GL_RED_INTEGER | GL_RG_INTEGER | GL_RGB_INTEGER | GL_RGBA_INTEGER)
}

/// The format/type check of the unpacking calls: unknown enums are
/// GL_INVALID_ENUM, known ones in a combination the core does not unpack
/// GL_INVALID_OPERATION (ES 3.0 3.7.1 / table 3.2). Depth uploads: the
/// ES 3.0 pairs (the texels are not modelled: they read as black).
fn check_format_type(c: &mut GlContext, format: GLenum, type_: GLenum, func: &str) -> bool {
    if !is_pixel_format_enum(format) || !is_pixel_type_enum(type_) || es3_only_pixel_type(type_) {
        c.set_error(GL_INVALID_ENUM, func);
        return false;
    }
    let ok = match format {
        GL_DEPTH_COMPONENT => matches!(type_, GL_UNSIGNED_SHORT | GL_UNSIGNED_INT | GL_FLOAT),
        GL_DEPTH_STENCIL => matches!(type_, GL_UNSIGNED_INT_24_8 | GL_FLOAT_32_UNSIGNED_INT_24_8_REV),
        _ => valid_format_type(format, type_),
    };
    if !ok {
        c.set_error(GL_INVALID_OPERATION, func);
    }
    ok
}

/// The depth base format (GL_DEPTH_COMPONENT or GL_DEPTH_STENCIL) of a
/// glTexImage internal format, or None for a colour one.
fn depth_base_of(internalformat: GLenum) -> Option<GLenum> {
    match internalformat {
        GL_DEPTH_COMPONENT | GL_DEPTH_COMPONENT16 | GL_DEPTH_COMPONENT24 | GL_DEPTH_COMPONENT32F => {
            Some(GL_DEPTH_COMPONENT)
        }
        GL_DEPTH_STENCIL | GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 => Some(GL_DEPTH_STENCIL),
        _ => None,
    }
}

/// The error of a glTexImage2D whose internal format or `format` is a depth
/// one (GL_NO_ERROR when it is fine), after `check_format_type`.
///
/// Both surfaces: the internal format's base must be `format` ("the
/// internalformat, format and type must match an entry of table 3.2",
/// ES 3.0.6 3.8.3; OES_packed_depth_stencil "INVALID_OPERATION is generated if
/// the base internal format is DEPTH_COMPONENT or DEPTH_STENCIL_OES and
/// <format> is not" the same). A colour internal format with a depth `format`
/// was taken until 2026-09-30, and so was the reverse, making an R32F
/// "colour" texture out of a GL_DEPTH_COMPONENT internal format.
///
/// The ES 2.0 surface adds the two extensions' own rules
/// (GL_OES_depth_texture, GL_OES_packed_depth_stencil): only GL_TEXTURE_2D
/// ("INVALID_OPERATION is generated if <target> is not TEXTURE_2D"), a
/// GL_DEPTH_COMPONENT image of GL_UNSIGNED_SHORT or GL_UNSIGNED_INT ("...
/// and <type> is not UNSIGNED_SHORT, or UNSIGNED_INT"), a GL_DEPTH_STENCIL one
/// of GL_UNSIGNED_INT_24_8, and of the sized names only those
/// GL_OES_required_internalformat gives these bases (GL_DEPTH_COMPONENT16,
/// GL_DEPTH24_STENCIL8); GL_DEPTH_COMPONENT24 (GL_OES_depth24 is not
/// claimed) and the float ones are ES 3.0's, GL_INVALID_VALUE as the other
/// ES 3.0-only sized names are.
fn depth_image_error(target: GLenum, internalformat: GLenum, format: GLenum, type_: GLenum) -> GLenum {
    let depth_format = matches!(format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL);
    let base = depth_base_of(internalformat);
    if !depth_format && base.is_none() {
        return GL_NO_ERROR;
    }
    if crate::REPORT_ES2
        && matches!(internalformat, GL_DEPTH_COMPONENT24 | GL_DEPTH_COMPONENT32F | GL_DEPTH32F_STENCIL8)
    {
        return GL_INVALID_VALUE;
    }
    if base != Some(format) {
        return GL_INVALID_OPERATION;
    }
    if crate::REPORT_ES2 {
        let type_ok = match format {
            GL_DEPTH_COMPONENT => matches!(type_, GL_UNSIGNED_SHORT | GL_UNSIGNED_INT),
            _ => type_ == GL_UNSIGNED_INT_24_8,
        };
        if !type_ok || target != GL_TEXTURE_2D {
            return GL_INVALID_OPERATION;
        }
    }
    GL_NO_ERROR
}

/// glTexImage2D on the already fetched context (glCopyTexImage2D defines its
/// level through it; the callers trace glTexImage2D). False when the call
/// raised an error (whatever error was pending before).
///
/// # Safety
/// `pixels` is null or holds the image, as for glTexImage2D.
#[allow(clippy::too_many_arguments)]
unsafe fn tex_image_2d(
    c: &mut GlContext,
    target: GLenum,
    level: GLint,
    internalformat: GLint,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) -> bool {
    const FUNC: &str = "glTexImage2D";

    if target != GL_TEXTURE_2D && !is_face_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return false;
    }
    /* the *_INTEGER formats are known enums; their pairing is checked below */
    if !is_pixel_format_enum(format) || es2_integer_format(format) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return false;
    }
    if !check_format_type(c, format, type_, FUNC) {
        return false;
    }
    let depth_err = depth_image_error(target, internalformat as GLenum, format, type_);
    if depth_err != GL_NO_ERROR {
        c.set_error(depth_err, FUNC);
        return false;
    }
    /* GL ES 2.0 3.7.1 tells three errors apart that are easy to confuse: an
     * internal format this GL does not name is GL_INVALID_VALUE, while an
     * unsized one that names a different base format than `format` is
     * GL_INVALID_OPERATION. (The ES 3.0 sized formats carry their own
     * pairing, which sized_format() resolved.) */
    let depth_fmt = format == GL_DEPTH_COMPONENT || format == GL_DEPTH_STENCIL;
    if !depth_fmt {
        if !es3_image_check(c, internalformat as GLenum, format, type_, FUNC) {
            return false;
        }
        /* An ES 2.0 surface names only the sized formats its extensions
         * legitimise: GL_OES_required_internalformat's RGBA8, RGB8, RGBA4,
         * RGB5_A1 and RGB565, and GL_EXT_texture_rg's R8 and RG8. The sRGB
         * and float names are ES 3.0's, and this GL claims neither
         * GL_EXT_sRGB (the sRGB class is ES 3.0's) nor GL_OES_texture_float
         * (GL_RGBA32F is stored as halves), so glTexImage2D does not take
         * them -- dEQP's fbo.completeness read a complete framebuffer built
         * on one as a texture format this GL does not have.
         *
         * Only here, in glTexImage2D's own argument check. `sized_format`
         * is shared with glTexStorage2D, an ES 3.0 entry point for which
         * these names are correct, and refusing them there breaks it. */
        if crate::REPORT_ES2
            && matches!(
                internalformat as GLenum,
                GL_SRGB8 | GL_SRGB8_ALPHA8 | GL_RGBA16F | GL_RGBA32F | GL_R16F | GL_R32F | GL_RG16F | GL_RG32F
            )
        {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return false;
        }
        if sized_format(internalformat as GLenum).is_none() {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return false;
        }
        if is_texture_format(internalformat as GLenum) && internalformat as GLenum != format {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return false;
        }
    }
    let shape_err = image_shape_error(target, level, border, width, height);
    if shape_err != GL_NO_ERROR {
        c.set_error(shape_err, FUNC);
        return false;
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return false;
    };
    if c.objs().textures[ti].immutable {
        c.set_error(GL_INVALID_OPERATION, FUNC); /* ES 3.0 3.8.4 */
        return false;
    }
    note_target_use(c, ti);
    let t = &mut c.objs().textures[ti];
    let layers: u32 = if is_face_target(target) { 6 } else { 1 };
    let face = face_of(target);
    let mut fresh = false;
    let es3_idx = if depth_fmt { 0 } else { es3_index(internalformat as GLenum) };

    if level == 0 {
        /* A cube map's six faces are specified one call each: the first
         * lays out the whole (stacked) storage, the others find it. */
        let texfmt = texfmt_of(internalformat);
        let differs = t.layers as u32 != layers
            || t.width != width as u32
            || t.height != height as u32
            || !t.has_image()
            || t.block_bytes != 0
            || t.external
            || t.texfmt != texfmt
            || t.es3_fmt != es3_idx
            || t.format != format;
        /* Level 0 of the same size and format keeps the layout and the
         * levels above it, a 2D texture's as a cube face's. Laying it out
         * again dropped them: dEQP (glu::TextureCube) uploads each face's
         * whole chain before the next, so +X's level 0 came after -X's
         * levels 1.. (AM62P texture.filtering.cube.*_mipmap_*, 36, fixed in
         * 652000b4), and texture.specification.random_teximage2d uploads 2D
         * levels in random order (AM62P 11 on Linux; fixed 2026-09-24).
         * A 2D level 0 still gets fresh storage while a kick samples the old
         * texels, rather than waiting for it (the chain goes with it). */
        if differs || (!is_face_target(target) && backend::kick_pending(t.last_use)) {
            let before = t.mem;
            /* When the storage survives a re-layout, so do the OTHER faces'
             * texels, and their record must survive with them: an
             * application refreshing one face of a dynamic environment map
             * every frame would otherwise make its own cube map incomplete.
             * Storage that was laid out again is cleared, so there the
             * record goes too. So does a face
             * of another size or format, whether or not the storage was
             * reused: the faces specified before it no longer match, and the
             * cube map is incomplete until they are specified again (dEQP
             * texture.completeness.cube.npot_size_level_0_pos_x and
             * format_mismatch_rgba_rgb_level_0_neg_z read a complete cube
             * when a reused allocation kept the record, and an RGB face over
             * RGBA storage was no difference at all). */
            let had_faces = t.faces_set;
            /* levels uploaded before any level 0 survive a level 0 that
             * shows the chain is another shape (relayout_keeping_levels) */
            let ok = if t.level_defined & 1 == 0 && !t.mem.impl_.is_null() && t.format == format {
                relayout_keeping_levels(t, width as u32, height as u32, layers, texfmt, false)
            } else if defers_storage(target, format, texfmt, width as u32, height as u32, pixels) {
                /* a shadow map: laid out, its storage left to ensure_storage */
                texture_alloc_storage_deferred(t, width as u32, height as u32, texfmt) == 0
            } else {
                texture_alloc_storage(t, width as u32, height as u32, layers, texfmt) == 0
            };
            if !ok {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return false;
            }
            fresh = t.mem.gpu != before.gpu || t.mem.impl_ != before.impl_;
            t.faces_set = if fresh || differs { 0 } else { had_faces };
        }
        t.format = format;
        t.type_ = type_;
        t.es3_fmt = es3_idx;
        t.rgba32f = is_rgba32f(internalformat as GLenum);
        t.unsized_depth = is_unsized_depth(internalformat as GLenum);
        /* kept storage: level 0 is (again) specified and fits */
        t.level_defined |= 1;
        t.level_bad &= !u16::from(width != 0 && height != 0);
        if is_face_target(target) {
            t.faces_set |= 1u8 << face;
        }
    } else {
        /* A mip level. ES 2.0 3.7.1 has ONE error for it: without
         * GL_OES_texture_npot, a level above 0 that is not a power of two
         * (NPOT_RESTRICTED, which no build is any more: dEQP
         * texture.completeness.2d.npot_size expects GL_NO_ERROR when the
         * extension is listed). A level that
         * does not fit the chain the base level laid out is no error in
         * either version: it is recorded, gets no storage, and leaves the
         * texture incomplete while it stands (3.7.10 / ES 3.0 3.8.13). Until
         * 2026-09-24 it was GL_INVALID_VALUE, which failed dEQP's
         * not_positive_level_0 (levels after a 0x0 level 0) and extra_level
         * (a 1x1 past the chain's last, which 3.7.10 ignores) on the error
         * alone. */
        if NPOT_RESTRICTED && (!is_pot(width as u32) || !is_pot(height as u32)) {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return false;
        }
        let bit = 1u16 << (level as u32).min(15);
        /* A 2D level before any level 0: the chain it belongs to is the one
         * whose level 0 is this image doubled `level` times, so that is laid
         * out now and level 0 stays unspecified (the texture incomplete)
         * until it comes. Held nowhere until 2026-09-24, so a level uploaded
         * before level 0 was lost (dEQP texture.specification.
         * random_teximage2d uploads levels in random order). A 1-texel side
         * could be any level-0 size; the later level 0 lays the chain out
         * again if it disagrees. */
        let (w0, h0) = ((width as u32) << level, (height as u32) << level);
        /* a cube face the same: dEQP's random_teximage2d.cube_* start with a
         * level above 0 of some face (AM62P cube_1/2/8 at 33a24bd7) */
        if (target == GL_TEXTURE_2D || is_face_target(target))
            && !t.has_image()
            && t.level_defined & 1 == 0
            && width > 0
            && height > 0
            && w0 <= MAX_TEXTURE_SIZE as u32
            && h0 <= MAX_TEXTURE_SIZE as u32
        {
            if texture_alloc_storage(t, w0, h0, layers, texfmt_of(internalformat)) != 0 {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return false;
            }
            fresh = true;
            t.level_defined = 0;
            t.format = format;
            t.type_ = type_;
            t.es3_fmt = es3_idx;
            t.rgba32f = is_rgba32f(internalformat as GLenum);
            t.unsized_depth = is_unsized_depth(internalformat as GLenum);
        }
        /* a later level before level 0 that does not fit the chain the first
         * one implied: the chain is the one THIS level implies, and the
         * levels that fit it too are kept (relayout_keeping_levels) */
        let (lw, lh) = level_size(t, level as u32);
        if (target == GL_TEXTURE_2D || is_face_target(target))
            && !t.mem.impl_.is_null()
            && t.level_defined & 1 == 0
            && t.block_bytes == 0
            && t.format == format
            && (level as u32 >= t.chain_levels as u32 || (width as u32, height as u32) != (lw, lh))
            && width > 0
            && height > 0
            && w0 <= MAX_TEXTURE_SIZE as u32
            && h0 <= MAX_TEXTURE_SIZE as u32
        {
            if !relayout_keeping_levels(t, w0, h0, layers, texfmt_of(internalformat), false) {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return false;
            }
            fresh = true;
        }
        /* A level of a non-power-of-two image's chain, at that level's size
         * (GL_OES_texture_npot; ES 3.0): the store has one level of such an
         * image and lays the chain out now where the backend holds one
         * (grow_npot_chain), after the kicks using the texture. */
        let npot_level = t.level_defined & 1 != 0
            && t.level_bad & 1 == 0
            && t.chain_levels == 1
            && !(is_pot(t.width) && is_pot(t.height))
            && (level as u32) < u32::from(context::gl_mip_levels(t.width, t.height))
            && (width as u32, height as u32) == level_size(t, level as u32)
            && t.format == format;
        if npot_level && npot_mip_chains(t.layers as u32) {
            if !settle_texture(c, ti) {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return false;
            }
            let mut oom = false;
            fresh = grow_npot_chain(&mut c.objs().textures[ti], &mut oom);
            if oom {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return false;
            }
        }
        let t = &mut c.objs().textures[ti];
        let (lw, lh) = level_size(t, level as u32);
        /* GL's level, which a backend that keeps one level of a NPOT image
         * (a PowerVR NPOT cube, npot_mip_chains) cannot hold: recorded as there, its
         * texels dropped, so the chain is complete and samples its base, as
         * after glGenerateMipmap. */
        if npot_level && t.chain_levels == 1 {
            t.level_defined |= bit;
            t.level_bad &= !bit;
            if let Some(f) = t.level_faces.get_mut(level as usize) {
                *f |= 1u8 << face;
            }
            return true;
        }
        /* a level of another format over a compressed chain: held nowhere */
        if t.block_bytes != 0 {
            t.level_defined |= bit;
            t.level_bad |= bit;
            return true;
        }
        if level as u32 >= t.chain_levels as u32 || width as u32 != lw || height as u32 != lh {
            /* past a chain that ran down to 1x1: 3.7.10 ignores such a level;
             * NOT past a one-level NPOT chain, whose levels are real: one of
             * another size or format than GL's chain is recorded, so the
             * texture reads incomplete (the right one was taken above) */
            let past_the_chain =
                level as u32 >= t.chain_levels as u32 && (lw, lh) == (1, 1) && (width, height) == (1, 1);
            if !past_the_chain {
                t.level_defined |= bit;
                t.level_bad |= bit;
                if let Some(f) = t.level_faces.get_mut(level as usize) {
                    *f &= !(1u8 << face);
                }
            }
            return true;
        }
        t.level_defined |= bit;
        /* every level of a complete texture has level 0's format; a cube
         * map's faces each keep their own bit (`level_faces`) */
        if format != t.format || es3_idx != t.es3_fmt {
            t.level_bad |= bit;
            t.level_faces[level as usize] &= !(1u8 << face);
        } else {
            t.level_bad &= !bit;
            t.level_faces[level as usize] |= 1u8 << face;
        }
        if level as u32 + 1 > t.levels as u32 {
            t.levels = (level + 1) as u8;
        }
        if (t.levels_written as u32) < level as u32 + 1 {
            t.levels_written = (level + 1) as u8;
        }
    }

    let t = &c.objs_ref().textures[ti]; /* the level's branch may have laid it out again */
    if pixels.is_null() || width == 0 || height == 0 || !t.has_image() {
        return true; /* storage allocated, cleared, kept or deferred */
    }
    if !texels_writable(t) {
        c.set_error(GL_INVALID_OPERATION, FUNC); /* a level of an RGB565 image */
        return false;
    }
    /* texels written in place: storage kept from before may still be in use
     * (settle_texture also allocates a deferred store; so does
     * ensure_storage for one laid out fresh) */
    if (!fresh && !settle_texture(c, ti)) || !ensure_storage(&mut c.objs().textures[ti]) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return false;
    }
    let t = &c.objs_ref().textures[ti];
    let y = if face != 0 { face * ((t.height >> level).max(1)) } else { 0 };
    if !texture_write_rect(c, ti, level as u32, width as u32, 0, y, width as u32, height as u32, pixels, format, type_)
    {
        c.set_error(GL_OUT_OF_MEMORY, FUNC); /* the platform failed the upload */
        return false;
    }
    note_uploaded(c, ti, level as u32, 0, y, width as u32, height as u32, pixels, format, type_);
    true
}

#[no_mangle]
pub unsafe extern "C" fn glTexImage2D(
    target: GLenum,
    level: GLint,
    internalformat: GLint,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) {
    gl_trace!(glTexImage2D);
    let Some(c) = context::current() else { return };
    /* With PIXEL_UNPACK_BUFFER bound, `pixels` is an offset into that buffer
     * rather than a client address (ES 3.0 3.7.1). */
    let Some(pixels) = unpack_source(c, pixels, width, height, 1, format, type_, "glTexImage2D") else {
        return;
    };
    tex_image_2d(c, target, level, internalformat, width, height, border, format, type_, pixels);
}

#[no_mangle]
pub unsafe extern "C" fn glTexSubImage2D(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) {
    const FUNC: &str = "glTexSubImage2D";
    gl_trace!(glTexSubImage2D);
    let Some(c) = context::current() else { return };
    /* With PIXEL_UNPACK_BUFFER bound, `pixels` is an offset into that buffer
     * rather than a client address (ES 3.0 3.7.1). */
    let Some(pixels) = unpack_source(c, pixels, width, height, 1, format, type_, "glTexSubImage2D") else {
        return;
    };

    if target != GL_TEXTURE_2D && !is_face_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* an ES 3.0 *_INTEGER format is a known enum: the pairing it is in
     * is GL_INVALID_OPERATION (check_format_type), not GL_INVALID_ENUM
     * (dEQP-GLES3 negative_api.texture.texsubimage2d/3d) */
    /* A depth image is ES 3.0's (table 3.2), and on the ES 2.0 surface
     * GL_OES_depth_texture's and GL_OES_packed_depth_stencil's: where those
     * are not claimed (state::DEPTH_TEXTURE_EXTENSIONS) it is not a format
     * this call takes. On the ES 3.0 surface it was refused too until
     * 2026-09-26, and dEQP-GLES3 texture.specification texsubimage2d_depth /
     * texstorage2d (depth) failed on it. */
    let depth_format = matches!(format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL);
    if !is_pixel_format_enum(format)
        || (crate::REPORT_ES2 && !super::state::DEPTH_TEXTURE_EXTENSIONS && depth_format)
        || es2_integer_format(format)
    {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if !check_format_type(c, format, type_, FUNC) {
        return;
    }
    /* the extensions' types on the ES 2.0 surface (depth_image_error) */
    if crate::REPORT_ES2
        && depth_format
        && !matches!(
            (format, type_),
            (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT | GL_UNSIGNED_INT) | (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8)
        )
    {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    if level < 0 || level >= CAP_MIP_LEVELS as GLint || width < 0 || height < 0 || xoffset < 0 || yoffset < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let ti = active_texture(c, target).filter(|&ti| {
        let t = &c.objs_ref().textures[ti];
        texels_writable(t) && ((level as u32) < t.levels as u32 || recorded_unheld(t, level))
    });
    let Some(ti) = ti else {
        /* no storage at that level, or storage the converters cannot write
         * (compressed, an RGB565 image) */
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if !es3_sub_image_check(c, ti, format, type_, FUNC) {
        return;
    }
    /* A depth image goes into a texture of that depth format, and nothing
     * else does (ES 3.0.6 3.8.5: the format must match the texture's base
     * format; OES_packed_depth_stencil the same): written as the other kind,
     * the texels became floats in an RGBA8 store or bytes in an R32F one. */
    let tex_format = c.objs_ref().textures[ti].format;
    if (depth_format || matches!(tex_format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)) && format != tex_format {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let (lw, lh) = level_size(&c.objs_ref().textures[ti], level as u32);
    if xoffset as u32 + width as u32 > lw || yoffset as u32 + height as u32 > lh {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if pixels.is_null() || width == 0 || height == 0 || recorded_unheld(&c.objs_ref().textures[ti], level) {
        return;
    }
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    let ok = texture_write_rect(
        c,
        ti,
        level as u32,
        lw,
        xoffset as u32,
        (yoffset as u32).wrapping_add(face_of(target).wrapping_mul(lh)),
        width as u32,
        height as u32,
        pixels,
        format,
        type_,
    );
    if !ok {
        c.set_error(GL_OUT_OF_MEMORY, FUNC); /* the platform failed the upload */
        return;
    }
    note_level_written(&mut c.objs().textures[ti], level);
    note_uploaded(
        c,
        ti,
        level as u32,
        xoffset as u32,
        yoffset as u32,
        width as u32,
        height as u32,
        pixels,
        format,
        type_,
    );
}

/// A level the application just put texels in may be sampled (see
/// `GlTexture::levels_written`).
///
/// Upstream marks this in glTexImage2D/3D, glGenerateMipmap and the FBO
/// attachment only, which loses every level of an immutable (glTexStorage*)
/// texture but the base: its levels can only be filled by glTexSubImage*.
fn note_level_written(t: &mut GlTexture, level: GLint) {
    if level >= 0 && (t.levels_written as GLint) < level + 1 {
        t.levels_written = (level + 1) as u8;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glCopyTexImage2D(
    target: GLenum,
    level: GLint,
    internalformat: GLenum,
    x: GLint,
    y: GLint,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
) {
    const FUNC: &str = "glCopyTexImage2D";
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if let Some(c) = context::current() {
        if super::framebuffer::as_read(c, || unsafe {
            glCopyTexImage2D(target, level, internalformat, x, y, width, height, border)
        })
        .is_some()
        {
            return;
        }
    }
    /* the read framebuffer's glReadBuffer attachment */
    if let Some(c) = context::current() {
        if super::framebuffer::as_read_attachment(c, || unsafe {
            glCopyTexImage2D(target, level, internalformat, x, y, width, height, border)
        })
        .is_some()
        {
            return;
        }
    }
    gl_trace!(glCopyTexImage2D);
    let Some(c) = context::current() else { return };

    if target != GL_TEXTURE_2D && !is_face_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* ES 3.0 3.8.5 takes the sized names too (GL_RGBA8; dEQP
     * negative_api.texture.copyteximage2d_incomplete_framebuffer), and a
     * depth or stencil one is GL_INVALID_OPERATION there (no colour to
     * copy); ES 2.0 3.7.2 has no such name at all, GL_INVALID_ENUM. */
    if matches!(
        internalformat,
        GL_DEPTH_COMPONENT
            | GL_DEPTH_STENCIL
            | GL_DEPTH_COMPONENT16
            | GL_DEPTH_COMPONENT24
            | GL_DEPTH_COMPONENT32F
            | GL_DEPTH24_STENCIL8
            | GL_DEPTH32F_STENCIL8
            | GL_STENCIL_INDEX8
    ) {
        c.set_error(if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_INVALID_OPERATION }, FUNC);
        return;
    }
    if let Some((_, e)) = es3_formats::find(internalformat) {
        if !es3_copy_compatible(c, e) {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
        if es3_storage_or_refuse(c, e, FUNC).is_none() {
            return;
        }
    }
    let Some((copy_format, copy_type, _)) = sized_format(internalformat) else {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    };
    let shape_err = image_shape_error(target, level, border, width, height);
    if shape_err != GL_NO_ERROR {
        c.set_error(shape_err, FUNC);
        return;
    }
    if !framebuffer::fb_target_ok(c) {
        c.set_error(GL_INVALID_FRAMEBUFFER_OPERATION, FUNC);
        return;
    }
    /* define the level as glTexImage2D(level, w, h, NULL) would ... */
    gl_trace!(glTexImage2D);
    let defined = tex_image_2d(
        c,
        target,
        level,
        internalformat as GLint,
        width,
        height,
        0,
        copy_format,
        copy_type,
        core::ptr::null(),
    );
    /* (an error an earlier call left pending does not stop the copy) */
    if !defined {
        return;
    }
    let Some(ti) = active_texture(c, target) else { return };
    if c.objs().textures[ti].mem.impl_.is_null() || width == 0 || height == 0 {
        return;
    }
    /* the level tex_image_2d just defined is writable (it checked); a kick
     * may still be reading the storage it kept */
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    /* ... then fill it from the framebuffer. A cube map keeps its six faces
     * stacked, so the face's rows start at face * the level's height. */
    let (lw, lh) = level_size(&c.objs_ref().textures[ti], level as u32);
    let dy = face_of(target) * lh;
    if !texture_copy_from_fb(c, ti, level as u32, lw, 0, dy, x, y, width as u32, height as u32, internalformat) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glCopyTexSubImage2D(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    x: GLint,
    y: GLint,
    width: GLsizei,
    height: GLsizei,
) {
    const FUNC: &str = "glCopyTexSubImage2D";
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if let Some(c) = context::current() {
        if super::framebuffer::as_read(c, || unsafe {
            glCopyTexSubImage2D(target, level, xoffset, yoffset, x, y, width, height)
        })
        .is_some()
        {
            return;
        }
    }
    /* the read framebuffer's glReadBuffer attachment */
    if let Some(c) = context::current() {
        if super::framebuffer::as_read_attachment(c, || unsafe {
            glCopyTexSubImage2D(target, level, xoffset, yoffset, x, y, width, height)
        })
        .is_some()
        {
            return;
        }
    }
    gl_trace!(glCopyTexSubImage2D);
    let Some(c) = context::current() else { return };

    if target != GL_TEXTURE_2D && !is_face_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if level < 0 || level >= CAP_MIP_LEVELS as GLint || width < 0 || height < 0 || xoffset < 0 || yoffset < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* Reading from an incomplete framebuffer is checked before the texture:
     * GL_INVALID_FRAMEBUFFER_OPERATION outranks whatever is bound (ES 2.0
     * 4.4.5), and dEQP asks for it with no texture at all. */
    if !framebuffer::fb_target_ok(c) {
        c.set_error(GL_INVALID_FRAMEBUFFER_OPERATION, FUNC);
        return;
    }
    let ti = active_texture(c, target).filter(|&ti| {
        let t = &c.objs_ref().textures[ti];
        texels_writable(t) && (level as u32) < t.levels as u32
    });
    let Some(ti) = ti else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if es3_formats::by_index(c.objs_ref().textures[ti].es3_fmt).is_some_and(|e| !es3_copy_compatible(c, e)) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let (lw, lh) = level_size(&c.objs_ref().textures[ti], level as u32);
    if xoffset as u32 + width as u32 > lw || yoffset as u32 + height as u32 > lh {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if width == 0 || height == 0 {
        return;
    }
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    let format = match c.objs().textures[ti].format {
        0 => GL_RGBA,
        f => f,
    };
    /* the face's rows start at face * the level's height (stacked faces) */
    let ok = texture_copy_from_fb(
        c,
        ti,
        level as u32,
        lw,
        xoffset as u32,
        yoffset as u32 + face_of(target) * lh,
        x,
        y,
        width as u32,
        height as u32,
        format,
    );
    if !ok {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    note_level_written(&mut c.objs().textures[ti], level);
}

/* ---- compressed textures: ETC1 / ETC2, decoded by the GPU ------------------
 * There are two block layouts, and the backend says which one it reads
 * (`YakoglBackend::compressed_blocks_in_rows`).
 *
 * The PowerVR TPU wants the twiddled 2D layout: the 4x4 blocks of a
 * power-of-two block grid in Morton order with the y bit in the lower
 * position and the longer dimension's spare bits above (its "TwiddleUV"
 * order). The Vivante GC7000 wants plain raster rows of blocks -- ceil(w/4)
 * of them per row, no padding -- with the row pitch in its descriptor.
 *
 * The storage holds GL's whole chain, level-major: level l starts at
 * `level_off[l]` (on the backend's `mip_level_align` boundary) and holds
 * `layers` faces back to back, each its own block grid of (w >> l) x
 * (h >> l) texels -- the layout the GC7000's descriptor walks
 * (vivante/texture.rs build_descriptor). `block_w`/`block_h` are level 0's
 * grid of one face. The PowerVR backend samples level 0 only. */

/// True when the blocks lie in raster rows rather than the twiddled grid.
fn blocks_in_rows() -> bool {
    backend::compressed_blocks_in_rows()
}

/// (GLES_TEXFMT_*, bytes per block) of a compressed internal format the
/// storage has a class for, whether or not this build stores it
/// ([`stored_compressed_format`] says that).
fn compressed_format_info(internalformat: GLenum) -> Option<(u8, u8)> {
    match internalformat {
        /* ETC1 blocks are ETC2 blocks */
        GL_ETC1_RGB8_OES | GL_COMPRESSED_RGB8_ETC2 => Some((GLES_TEXFMT_ETC2_RGB, 8)),
        GL_COMPRESSED_RGBA8_ETC2_EAC => Some((GLES_TEXFMT_ETC2_RGBA, 16)),
        /* the same blocks, sampled through the sRGB decode */
        GL_COMPRESSED_SRGB8_ETC2 => Some((GLES_TEXFMT_ETC2_SRGB_RGB, 8)),
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC => Some((GLES_TEXFMT_ETC2_SRGB_RGBA, 16)),
        _ => None,
    }
}

/// Every compressed internal format [`compressed_format_info`] has a class
/// for, in the order GL_COMPRESSED_TEXTURE_FORMATS lists the stored ones.
pub(crate) const COMPRESSED_FORMATS: [GLenum; 5] = [
    GL_ETC1_RGB8_OES,
    GL_COMPRESSED_RGB8_ETC2,
    GL_COMPRESSED_RGBA8_ETC2_EAC,
    GL_COMPRESSED_SRGB8_ETC2,
    GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
];

/// [`compressed_format_info`] of a format this build stores, None for one
/// it does not. The linear ETC2 classes are stored everywhere. The sRGB
/// ones only in an ES 3.0 report (ES 2.0 has no sRGB format) and where the
/// back end samples them ([`backend::sampled_texfmts`]: the PowerVR, whose
/// TPU decodes the ETC2 blocks with its gamma bit, docs/es3/srgb.md 4.3).
/// glCompressedTex*Image and GL_COMPRESSED_TEXTURE_FORMATS both ask here, so
/// the list names exactly the formats an upload accepts.
pub(crate) fn stored_compressed_format(internalformat: GLenum) -> Option<(u8, u8)> {
    let (fmt, bb) = compressed_format_info(internalformat)?;
    let srgb = matches!(fmt, GLES_TEXFMT_ETC2_SRGB_RGB | GLES_TEXFMT_ETC2_SRGB_RGBA);
    if srgb && (crate::REPORT_ES2 || backend::sampled_texfmts() & backend::gles_texfmt_bit(fmt) == 0) {
        return None;
    }
    Some((fmt, bb))
}

/// The compressed internal formats this build stores
/// (GL_COMPRESSED_TEXTURE_FORMATS), written into `out`; returns how many.
pub(crate) fn stored_compressed_formats(out: &mut [GLint]) -> usize {
    let mut n = 0;
    for f in COMPRESSED_FORMATS.iter().copied().filter(|&f| stored_compressed_format(f).is_some()) {
        if let Some(slot) = out.get_mut(n) {
            *slot = f as GLint;
        }
        n += 1;
    }
    n
}

fn npot2_u32(v: u32) -> u32 {
    let mut p: u32 = 1;

    while p < v {
        p <<= 1;
    }
    p
}

/// The block grid one face of a w x h level is held in: the one place a
/// level's pitch (grid width x block bytes) and size are decided, for the
/// layout and for the writes. The twiddled order interleaves the two block
/// coordinates' bits, so its grid has to be a power of two in each; raster
/// rows do not. The GC7000 addresses a block format TILED and derives each
/// level's pitch itself (Mesa's etnaviv lays the levels out at their own
/// pitch and programs level 0's only), so each level gets its own rows;
/// unmeasured on silicon for levels above 0. A row of raster blocks (`bb`
/// bytes each) is as wide as the backend's linear row pitch wants it
/// (`YakoglBackend::linear_row_align`: the Mali's 64 bytes, Mesa's raster
/// rows of a block format, pan_mod.c:741-746).
fn block_grid(w: u32, h: u32, bb: u32) -> (u32, u32) {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    if blocks_in_rows() {
        (context::row_aligned(bw, bb, backend::linear_row_align()), bh)
    } else {
        (npot2_u32(bw), npot2_u32(bh))
    }
}

/// Level `level`'s block grid of a compressed image whose level 0 is
/// w0 x h0. Raster rows: the level's own size's rows ([`block_grid`]).
/// Twiddled (the PowerVR): level 0's power-of-two grid halved per level,
/// at least one block -- the TPU's walk, which minifies the power-of-two
/// extent in BLOCKS (Mesa pvr_image_plane_setup_mip_levels over the
/// physical extent: max(1, (POT(W)/4) >> l)). The level's own size rounded
/// up is the same for power-of-two images and smaller for some NPOT ones
/// (w0 = 17: level 1 is 8 texels, 2 blocks, where the TPU steps 4), which
/// put every later level at an offset the TPU does not read.
fn level_block_grid(w0: u32, h0: u32, level: u32, bb: u32) -> (u32, u32) {
    if w0 == 0 || h0 == 0 {
        return block_grid(w0, h0, bb);
    }
    if blocks_in_rows() {
        return block_grid((w0 >> level).max(1), (h0 >> level).max(1), bb);
    }
    let (gw, gh) = block_grid(w0, h0, bb);
    ((gw >> level).max(1), (gh >> level).max(1))
}

/// The byte layout of a compressed chain of a w x h image with `layers`
/// faces: (level offsets, levels, total bytes); None when it does not fit
/// the u32 offsets. Every level of GL's chain, power of two or not: each is
/// its own block grid, so a NPOT level needs nothing the layout lacks.
fn compressed_chain_layout(w: u32, h: u32, layers: u32, bb: u32) -> Option<([u32; CAP_MIP_LEVELS], u8, u32)> {
    let align = backend::mip_level_align();
    let levels = if w == 0 || h == 0 { 1 } else { context::gl_mip_levels(w, h).min(CAP_MIP_LEVELS as u8) };
    let mut off = [0u32; CAP_MIP_LEVELS];
    let mut bytes: u32 = 0;

    for l in 0..levels as u32 {
        if align != 0 {
            bytes = bytes.checked_add(align - 1)? & !(align - 1);
        }
        off[l as usize] = bytes;
        /* a 0x0 level 0 has no blocks (raster rows) or one (the grid rounds up) */
        let (gw, gh) = level_block_grid(w, h, l, bb);
        bytes = bytes.checked_add(gw.checked_mul(gh)?.checked_mul(bb)?.checked_mul(layers)?)?;
    }
    Some((off, levels, bytes))
}

/// Byte offset of face `face` of level `level` of a compressed texture, and
/// that level's block grid.
pub(crate) fn compressed_face_base(t: &GlTexture, level: u32, face: u32) -> (usize, u32, u32) {
    let (gw, gh) = level_block_grid(t.width, t.height, level, t.block_bytes as u32);
    let face_bytes = gw as usize * gh as usize * t.block_bytes as usize;
    (t.level_off[level as usize] as usize + face as usize * face_bytes, gw, gh)
}

/// Block (bx, by) of a bw x bh block grid -> block index in twiddled order.
pub(crate) fn twiddle_index(bx: u32, by: u32, bw: u32, bh: u32) -> u32 {
    let min_dim = bw.min(bh);
    let (mut src, mut dst, mut shift, mut out) = (1u32, 1u32, 0u32, 0u32);

    while src < min_dim {
        if by & src != 0 {
            out |= dst;
        }
        if bx & src != 0 {
            out |= dst << 1;
        }
        src <<= 1;
        dst <<= 2;
        shift += 1;
    }
    /* the larger dimension's remaining bits, linear, above the interleave */
    let max_val = (if bw > bh { bx } else { by }) >> shift;
    out | (max_val << (2 * shift))
}

/// Copy a w x h texel rectangle of blocks (data: rows of ceil(w/4) blocks) to
/// blocks (x0/4, y0/4)... of face `face` of level `level`. False when the
/// platform failed a write.
///
/// # Safety
/// The rectangle must lie inside the level; `data` is trusted as the
/// application's imageSize-checked pointer.
#[allow(clippy::too_many_arguments)]
unsafe fn texture_write_blocks(
    t: &mut GlTexture,
    level: u32,
    face: u32,
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    data: *const u8,
) -> bool {
    t.cpu_wrote = true;
    let (bw, bh, bb) = (w.div_ceil(4), h.div_ceil(4), t.block_bytes as usize);
    let (gx0, gy0) = (x0 / 4, y0 / 4);
    let (base, gw, gh) = compressed_face_base(t, level, face);
    // SAFETY: data holds bw * bh blocks (the callers checked imageSize against it).
    let blocks = core::slice::from_raw_parts(data, bw as usize * bh as usize * bb);
    let gpu_write = osal::ops().and_then(|o| o.gpu_write);
    let mut ok = true;

    for by in 0..bh {
        for bx in 0..bw {
            let idx =
                if blocks_in_rows() { (gy0 + by) * gw + (gx0 + bx) } else { twiddle_index(gx0 + bx, gy0 + by, gw, gh) };
            let dst = base + idx as usize * bb;
            let src = &blocks[(by as usize * bw as usize + bx as usize) * bb..][..bb];

            if !t.mem.cpu.is_null() {
                // SAFETY: the block index is inside the level's grid, which the storage holds.
                core::slice::from_raw_parts_mut((t.mem.cpu as *mut u8).add(dst), bb).copy_from_slice(src);
            } else if let Some(gpu_write) = gpu_write {
                // SAFETY: `src` holds one block; the OSAL bounds the write.
                ok &= gpu_write(&mut t.mem, dst, src.as_ptr() as *const c_void, bb) == 0;
            } else {
                ok = false;
            }
        }
    }
    if !t.mem.cpu.is_null() {
        // SAFETY: the face's grid lies inside the allocation.
        osal::cache_flush((t.mem.cpu as *mut u8).add(base) as *mut c_void, gw as usize * gh as usize * bb);
    }
    ok
}

/// Bytes per 4x4 block of every ES 3.0 compressed format (ES 3.0 table
/// 3.19), stored or not: the imageSize rule holds for all of them
/// (dEQP-GLES3 negative_api.texture.compressedteximage2d_invalid_size gives
/// GL_COMPRESSED_SIGNED_R11_EAC a wrong size and expects GL_INVALID_VALUE).
fn compressed_block_bytes(internalformat: GLenum) -> Option<u8> {
    match internalformat {
        GL_ETC1_RGB8_OES
        | GL_COMPRESSED_R11_EAC
        | GL_COMPRESSED_SIGNED_R11_EAC
        | GL_COMPRESSED_RGB8_ETC2
        | GL_COMPRESSED_SRGB8_ETC2
        | GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2
        | GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 => Some(8),
        GL_COMPRESSED_RG11_EAC
        | GL_COMPRESSED_SIGNED_RG11_EAC
        | GL_COMPRESSED_RGBA8_ETC2_EAC
        | GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC => Some(16),
        _ => None,
    }
}

/// The bytes of a w x h x d compressed image of `bb`-byte blocks; None
/// when a size is negative or the product overflows.
fn compressed_image_bytes(w: GLsizei, h: GLsizei, d: GLsizei, bb: u8) -> Option<usize> {
    if w < 0 || h < 0 || d < 0 {
        return None;
    }
    (w as usize).div_ceil(4).checked_mul((h as usize).div_ceil(4))?.checked_mul(d as usize)?.checked_mul(bb as usize)
}

/// Which images of a compressed texture a glCompressedTex*Image call names:
/// `layers` in the storage (1 for a 2D texture, 6 for a cube map, the depth
/// of a 2D array), and the `count` of them from `first` it writes (a cube
/// face, or a run of array layers).
#[derive(Clone, Copy)]
struct CompressedDest {
    layers: u32,
    first: u32,
    count: u32,
    /// A cube map face: its level record is per face (`faces_set`,
    /// `level_faces`).
    face: bool,
}

/// glCompressedTexImage2D / glCompressedTexImage3D once the arguments are
/// checked: lay the storage out (level 0) or record the level, then write the
/// blocks. `data` holds `dst.count` images of w x h, one after the other.
///
/// # Safety
/// `data` is null or holds the imageSize the caller checked.
#[allow(clippy::too_many_arguments)]
unsafe fn compressed_tex_image(
    c: &mut GlContext,
    ti: usize,
    dst: CompressedDest,
    level: GLint,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
    data: *const c_void,
    func: &str,
) {
    let Some((fmt, bb)) = stored_compressed_format(internalformat) else {
        /* a format the storage does not hold (the EAC and punch-through
         * ones, and the sRGB ones where the back end does not sample them):
         * its arguments were right, so this is all that is left to say */
        c.set_error(GL_INVALID_ENUM, func);
        return;
    };
    note_target_use(c, ti);
    let t = &mut c.objs().textures[ti];
    let layers = dst.layers;
    let face = dst.first;
    let mut fresh = false;

    if level == 0 {
        /* As glTexImage2D: a cube map's six faces are one call each; the
         * first (and a face of another size or format) lays the storage
         * out, the others find it. A 2D texture or an array is one call. */
        let differs = t.layers as u32 != layers
            || t.width != width as u32
            || t.height != height as u32
            || t.mem.impl_.is_null()
            || t.block_bytes != bb
            || t.format != internalformat
            || t.external;
        /* a cube face of the same size and format keeps the layout and the
         * other faces' chains (as glTexImage2D) */
        if differs || !dst.face {
            let Some((level_off, chain_levels, bytes)) =
                compressed_chain_layout(width as u32, height as u32, layers, bb as u32)
            else {
                c.set_error(GL_OUT_OF_MEMORY, func);
                return;
            };
            let bytes = bytes as usize;
            let had_faces = t.faces_set;
            let before = t.mem;
            if t.mem.size < bytes || t.mem.impl_.is_null() || t.external || backend::kick_pending(t.last_use) {
                context::release_storage(&mut t.mem, &mut t.external);
                t.storage_deferred = false;
                /* `bytes > 0` for the same reason texture_alloc_storage_ex
                 * has it: a 0x0 level is legal and has no texels, and the
                 * OSAL rejects a zero-byte allocation (EINVAL). */
                if bytes > 0 && osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, &mut t.mem) != 0 {
                    c.set_error(GL_OUT_OF_MEMORY, func);
                    return;
                }
                /* see texture_alloc_storage_ex: no earlier user's bytes */
                if !context::clear_storage(&mut t.mem) {
                    c.set_error(GL_OUT_OF_MEMORY, func);
                    return;
                }
            }
            fresh = t.mem.gpu != before.gpu || t.mem.impl_ != before.impl_;
            let (bw, bh) = block_grid(width as u32, height as u32, u32::from(bb));
            t.width = width as u32;
            t.height = height as u32;
            t.layers = layers as u16;
            t.grid_cols = 0;
            t.texfmt = fmt;
            t.es3_fmt = 0;
            t.rgba32f = false;
            t.unsized_depth = false;
            t.bpp = 0;
            t.block_bytes = bb;
            t.block_w = bw;
            t.block_h = bh;
            t.level_off = level_off;
            t.storage_gen = context::respecified(t.serial, true);
            t.chain_pitch = 0; /* every level its own block rows */
            t.pot_chain = false;
            t.chain_in_backend = false;
            t.chain_levels = chain_levels;
            t.levels = 1;
            t.levels_written = 1;
            t.level_defined = 1;
            t.level_bad = u16::from(width == 0 || height == 0);
            t.max_level = MAX_LEVEL_DEFAULT;
            /* storage that survived keeps the other faces' blocks, and their
             * record, unless this face is of another size or format (as
             * glTexImage2D) */
            t.faces_set = if fresh || differs { 0 } else { had_faces };
            t.level_faces = [0; CAP_MIP_LEVELS];
        }
        /* laid out again, and a fresh upload skips settle_texture */
        stamp_texels(t);
        t.format = internalformat;
        t.type_ = 0;
        t.bgra = false;
        t.pe_order = false;
        if dst.face {
            t.faces_set |= 1u8 << face;
        }
    } else {
        /* A mip level, recorded as glTexImage2D records one: a level that
         * does not fit the chain level 0 laid out -- another size, another
         * format, no compressed level 0 at all -- is no error, gets no
         * storage, and leaves the texture incomplete while it stands
         * (ES 2.0 3.7.10). Until 2026-09-24 every level above 0 and every
         * cube face was GL_INVALID_OPERATION: dEQP-GLES2's etc1 filtering,
         * format and cube cases could not upload their chains. */
        let bit = 1u16 << (level as u32).min(15);
        let (lw, lh) = level_size(t, level as u32);
        if t.block_bytes == 0
            || t.mem.impl_.is_null()
            || level as u32 >= t.chain_levels as u32
            || width as u32 != lw
            || height as u32 != lh
            || t.format != internalformat
            || t.layers as u32 != layers
        {
            /* a 1x1 past a chain that ran down to 1x1 is ignored (3.7.10) */
            let past_the_chain = t.block_bytes != 0
                && level as u32 >= t.chain_levels as u32
                && (lw, lh) == (1, 1)
                && (width, height) == (1, 1);
            if !past_the_chain {
                t.level_defined |= bit;
                t.level_bad |= bit;
                if let (true, Some(f)) = (dst.face, t.level_faces.get_mut(level as usize)) {
                    *f &= !(1u8 << face);
                }
            }
            return;
        }
        t.level_defined |= bit;
        t.level_bad &= !bit;
        if dst.face {
            t.level_faces[level as usize] |= 1u8 << face;
        }
        if level as u32 + 1 > t.levels as u32 {
            t.levels = (level + 1) as u8;
        }
        note_level_written(t, level);
    }

    if data.is_null() || width == 0 || height == 0 || t.mem.impl_.is_null() {
        return;
    }
    /* blocks written in place: storage kept from before may still be in use */
    if !fresh && !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, func);
        return;
    }
    let t = &mut c.objs().textures[ti];
    let image = compressed_image_bytes(width, height, 1, bb).unwrap_or(0);
    for i in 0..dst.count {
        let src = (data as *const u8).add(i as usize * image);
        if !texture_write_blocks(t, level as u32, face + i, 0, 0, width as u32, height as u32, src) {
            c.set_error(GL_OUT_OF_MEMORY, func);
            return;
        }
    }
}

/// glCompressedTexImage2D of a GL_ETC1_RGB8_OES image on a backend that
/// decodes ETC1 at upload: the level is laid out as glTexImage2D(GL_RGB,
/// GL_UNSIGNED_BYTE) lays it out, with its errors (a level that does not fit
/// the chain, an immutable texture), and the blocks are decoded into it.
/// The caller checked the target, the shape and imageSize.
///
/// # Safety
/// `data` is null or holds the image's ceil(w/4) x ceil(h/4) blocks.
unsafe fn etc1_image_decoded(
    c: &mut GlContext,
    target: GLenum,
    level: GLint,
    width: GLsizei,
    height: GLsizei,
    data: *const c_void,
) {
    const FUNC: &str = "glCompressedTexImage2D";

    if !tex_image_2d(c, target, level, GL_RGB as GLint, width, height, 0, GL_RGB, GL_UNSIGNED_BYTE, core::ptr::null()) {
        return; /* its error is set */
    }
    if data.is_null() || width == 0 || height == 0 {
        return;
    }
    let Some(ti) = active_texture(c, target) else { return };
    if c.objs_ref().textures[ti].mem.impl_.is_null() {
        return;
    }
    /* storage kept from before may still be in use; this also stamps it */
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    let t = &mut c.objs().textures[ti];
    let (lw, lh) = level_size(t, level as u32);
    /* only where glTexImage2D writes texels: a level that does not fit the
     * chain is recorded (the texture reads incomplete) and holds none */
    if level as u32 >= t.chain_levels as u32 || (width as u32, height as u32) != (lw, lh) || !texels_writable(t) {
        return;
    }
    let y0 = face_of(target) * lh; /* a cube map's faces are stacked in each level */
    if !texture_write_etc1(t, level as u32, lw, y0, width as u32, height as u32, data as *const u8) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC); /* the platform failed the upload */
        return;
    }
    note_level_written(t, level);
}

/// Decode the w x h ETC1 image `data` (rows of ceil(w/4) blocks) into rows
/// y0.. of an RGBA8 level `lw` texels wide. False when the platform failed a
/// write.
///
/// # Safety
/// `data` holds the image's blocks; the level holds rows y0..y0 + h of at
/// least w texels.
unsafe fn texture_write_etc1(t: &mut GlTexture, level: u32, lw: u32, y0: u32, w: u32, h: u32, data: *const u8) -> bool {
    t.cpu_wrote = true;
    let pitch = level_row_pitch(t, level, lw) as usize; /* texels */
    let off = t.level_off[level as usize] as usize;
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    // SAFETY: data holds bw * bh blocks (the caller checked imageSize against it).
    let blocks = core::slice::from_raw_parts(data, bw as usize * bh as usize * super::etc1::BLOCK_BYTES);
    let gpu_write = osal::ops().and_then(|o| o.gpu_write);
    let mut ok = true;

    for by in 0..bh {
        for bx in 0..bw {
            let i = (by * bw + bx) as usize * super::etc1::BLOCK_BYTES;
            let mut block = [0u8; super::etc1::BLOCK_BYTES];
            block.copy_from_slice(&blocks[i..i + super::etc1::BLOCK_BYTES]);
            let texels = super::etc1::decode_block(&block);
            /* a block past the image's right or bottom edge is cut there */
            let n = (w - bx * 4).min(4) as usize;
            for r in 0..(h - by * 4).min(4) {
                let mut row = [0u8; 16];
                for x in 0..n {
                    row[x * 4..x * 4 + 4].copy_from_slice(&texels[(r * 4) as usize + x]);
                }
                let dst = off + ((y0 + by * 4 + r) as usize * pitch + bx as usize * 4) * RGBA8_BPP as usize;
                if !t.mem.cpu.is_null() {
                    // SAFETY: the row lies inside the level (caller's contract).
                    core::ptr::copy_nonoverlapping(row.as_ptr(), (t.mem.cpu as *mut u8).add(dst), n * 4);
                } else if let Some(gpu_write) = gpu_write {
                    // SAFETY: `row` holds n texels; the OSAL bounds the write.
                    ok &= gpu_write(&mut t.mem, dst, row.as_ptr() as *const c_void, n * 4) == 0;
                } else {
                    ok = false;
                }
            }
        }
    }
    if !t.mem.cpu.is_null() && h != 0 {
        // SAFETY: the flushed range is the rows just written.
        osal::cache_flush(
            (t.mem.cpu as *mut u8).add(off + y0 as usize * pitch * RGBA8_BPP as usize) as *mut c_void,
            ((h as usize - 1) * pitch + w as usize) * RGBA8_BPP as usize,
        );
    }
    ok
}

#[no_mangle]
pub unsafe extern "C" fn glCompressedTexImage2D(
    target: GLenum,
    level: GLint,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
    imageSize: GLsizei,
    data: *const c_void,
) {
    const FUNC: &str = "glCompressedTexImage2D";
    gl_trace!(glCompressedTexImage2D);
    let Some(c) = context::current() else { return };
    /* With PIXEL_UNPACK_BUFFER bound, `data` is an offset into that buffer
     * rather than a client address (ES 3.0 3.7.1). */
    let Some(data) =
        super::buffer::pixel_buffer_resolve(c, false, data, imageSize.max(0) as usize, "glCompressedTexImage2D")
    else {
        return;
    };

    if target != GL_TEXTURE_2D && !is_face_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let Some(bb) = compressed_block_bytes(internalformat) else {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    };
    let shape_err = image_shape_error(target, level, border, width, height);
    if shape_err != GL_NO_ERROR {
        c.set_error(shape_err, FUNC);
        return;
    }
    if imageSize < 0 || compressed_image_bytes(width, height, 1, bb) != Some(imageSize as usize) {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* A backend whose texture unit has no ETC1 gets the image decoded here,
     * into an RGB level like glTexImage2D's; mip levels and cube faces are
     * then the uncompressed path's (YakoglBackend::etc1_decoded_at_upload). */
    if internalformat == GL_ETC1_RGB8_OES && backend::etc1_decoded_at_upload() {
        etc1_image_decoded(c, target, level, width, height, data);
        return;
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if c.objs().textures[ti].immutable {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let face = is_face_target(target);
    let dst = CompressedDest { layers: if face { 6 } else { 1 }, first: face_of(target), count: 1, face };
    compressed_tex_image(c, ti, dst, level, internalformat, width, height, data, FUNC);
}

/// glCompressedTexSubImage2D / 3D once the target and format are known: the
/// level must hold a compressed image of this format, the box must lie
/// inside it and be whole blocks (or reach the level's edge), and imageSize
/// must be the box's; then `layers` images of the box from `first`.
///
/// # Safety
/// `data` is null or holds the imageSize the caller passed.
#[allow(clippy::too_many_arguments)]
unsafe fn compressed_tex_sub_image(
    c: &mut GlContext,
    target: GLenum,
    level: GLint,
    (xoffset, yoffset, zoffset): (GLint, GLint, GLint),
    (width, height, depth): (GLsizei, GLsizei, GLsizei),
    format: GLenum,
    imageSize: GLsizei,
    data: *const c_void,
    func: &str,
) {
    let fmt_bb = compressed_format_info(format);
    /* the level must hold a compressed image of this format; a face target
     * reaches only a cube map, and level 0 only a face that was specified */
    let face = face_of(target);
    let ti = active_texture(c, target).filter(|&ti| {
        let t = &c.objs_ref().textures[ti];
        let bit = 1u16 << level as u32;
        fmt_bb.is_some_and(|(fmt, _)| t.texfmt == fmt && t.format == format)
            && !t.mem.impl_.is_null()
            && t.block_bytes != 0
            && (level as u32) < t.chain_levels as u32
            && t.level_defined & bit != 0
            && t.level_bad & bit == 0
            && (level != 0 || !is_face_target(target) || t.faces_set & (1u8 << face) != 0)
    });
    let (Some(ti), Some((_, bb))) = (ti, fmt_bb) else {
        c.set_error(GL_INVALID_OPERATION, func);
        return;
    };
    let t = &c.objs().textures[ti];
    let (tw, th) = level_size(t, level as u32);
    let td = if target == GL_TEXTURE_2D_ARRAY { t.layers as u32 } else { 1 };
    /* inside the level (GL_INVALID_VALUE), then whole blocks only or up to
     * the level's edge (GL_INVALID_OPERATION, ES 3.0 3.8.6 for ETC2/EAC;
     * dEQP-GLES3 negative_api.texture.compressedtexsubimage2d, which was
     * answered GL_INVALID_VALUE until 2026-09-24) */
    let (xend, yend, zend) =
        (xoffset.wrapping_add(width) as u32, yoffset.wrapping_add(height) as u32, zoffset.wrapping_add(depth) as u32);
    if xoffset < 0
        || yoffset < 0
        || zoffset < 0
        || width < 0
        || height < 0
        || depth < 0
        || xend > tw
        || yend > th
        || zend > td
    {
        c.set_error(GL_INVALID_VALUE, func);
        return;
    }
    if (xoffset & 3) != 0 || (yoffset & 3) != 0 || ((width & 3) != 0 && xend != tw) || ((height & 3) != 0 && yend != th)
    {
        c.set_error(GL_INVALID_OPERATION, func);
        return;
    }
    if imageSize < 0 || compressed_image_bytes(width, height, depth, bb) != Some(imageSize as usize) {
        c.set_error(GL_INVALID_VALUE, func);
        return;
    }
    if data.is_null() || width == 0 || height == 0 || depth == 0 {
        return;
    }
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, func);
        return;
    }
    let t = &mut c.objs().textures[ti];
    let image = compressed_image_bytes(width, height, 1, bb).unwrap_or(0);
    for i in 0..depth as u32 {
        let src = (data as *const u8).add(i as usize * image);
        let layer = face + zoffset as u32 + i;
        if !texture_write_blocks(
            t,
            level as u32,
            layer,
            xoffset as u32,
            yoffset as u32,
            width as u32,
            height as u32,
            src,
        ) {
            c.set_error(GL_OUT_OF_MEMORY, func);
            return;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn glCompressedTexSubImage2D(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    format: GLenum,
    imageSize: GLsizei,
    data: *const c_void,
) {
    const FUNC: &str = "glCompressedTexSubImage2D";
    gl_trace!(glCompressedTexSubImage2D);
    let Some(c) = context::current() else { return };
    /* With PIXEL_UNPACK_BUFFER bound, `data` is an offset into that buffer
     * rather than a client address (ES 3.0 3.7.1). */
    let Some(data) =
        super::buffer::pixel_buffer_resolve(c, false, data, imageSize.max(0) as usize, "glCompressedTexSubImage2D")
    else {
        return;
    };

    if target != GL_TEXTURE_2D && !is_face_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if compressed_block_bytes(format).is_none() {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* a level past the chain any texture may have is GL_INVALID_VALUE
     * before the storage is asked (dEQP compressedtexsubimage2d_max_level,
     * _neg_level, on a cube face too) */
    if level < 0 || level >= CAP_MIP_LEVELS as GLint {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    compressed_tex_sub_image(
        c,
        target,
        level,
        (xoffset, yoffset, 0),
        (width, height, 1),
        format,
        imageSize,
        data,
        FUNC,
    );
}

/// Box-filter the chain down from level 0 (CPU; the chain is CPU-visible).
#[no_mangle]
pub unsafe extern "C" fn glGenerateMipmap(target: GLenum) {
    const FUNC: &str = "glGenerateMipmap";
    gl_trace!(glGenerateMipmap);
    let Some(c) = context::current() else { return };

    if binding_for(c, target).is_none() {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let ti = active_texture(c, target)
        .filter(|&ti| !c.objs().textures[ti].mem.impl_.is_null() && c.objs().textures[ti].block_bytes == 0);
    let Some(ti) = ti else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    /* A cube map must have all six faces specified at one size, in both
     * versions (ES 2.0 3.7.10): a face given a different size laid the
     * storage out again and cleared the record. */
    let t = &c.objs_ref().textures[ti];
    if t.target == GL_TEXTURE_CUBE_MAP && t.faces_set != 0x3f {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* ES 2.0 without GL_OES_texture_npot: a level-0 image that is not a power
     * of two has no mip chain, so asking for one is an error rather than the
     * no-op below (NPOT_RESTRICTED). With the extension, as in ES 3.0, it has
     * one: glmark2's terrain asks for the chain of an 800x600 texture. */
    let npot = !is_pot(t.width) || !is_pot(t.height);
    if NPOT_RESTRICTED && npot {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* A depth texture has no generated chain: GL_OES_depth_texture
     * ("GenerateMipmap will generate INVALID_OPERATION error for a depth
     * texture"), and ES 3.0.6 3.8.9 (the base level must be colour-renderable
     * and filterable). It was a silent no-op. */
    if matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* ES 3.0 3.8.14: only a colour-renderable, texture-filterable base level
     * has a generated chain; of the formats of es3_formats that is RGB10_A2,
     * and only while it is stored as RGBA8 (the fallback): a box filter over
     * the packed word has no CPU path here. */
    if let Some(e) = es3_formats::by_index(t.es3_fmt) {
        if e.internalformat != GL_RGB10_A2 || t.texfmt != GLES_TEXFMT_RGBA8 {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
    }
    let filtered = !t.mem.cpu.is_null()
        && (matches!(
            t.texfmt,
            GLES_TEXFMT_RGBA8 | GLES_TEXFMT_SRGB8_A8 | GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F
        ) || is_narrow_la(t.texfmt))
        && !t.external;
    if t.target == GL_TEXTURE_3D {
        generate_mipmap_3d(c, ti, filtered);
        return;
    }
    /* A NPOT image has one level until its chain is asked for: laid out now
     * where the backend holds one (grow_npot_chain), after the kicks using
     * the texture, whose texels are copied across. Elsewhere nothing is
     * waited for: glmark2's terrain asks again every frame for a texture it
     * has just rendered into. */
    if npot && filtered && t.chain_levels < 2 && npot_mip_chains(t.layers as u32) {
        if !settle_texture(c, ti) {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        }
        let mut oom = false;
        grow_npot_chain(&mut c.objs().textures[ti], &mut oom);
        if oom {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        }
    }
    let t = &mut c.objs().textures[ti];
    if t.chain_levels < 2 {
        /* No chain in the store: a NPOT image where the backend keeps one
         * level (a PowerVR NPOT cube, npot_mip_chains), or a 1x1. GL's chain exists
         * all the same (ES 2.0 3.7.11), so the texture is complete under a
         * mipmap filter and samples its base level. It was left incomplete,
         * and an ES 3.0 build sampled such a texture black. */
        t.level_defined = ((1u32 << u32::from(context::gl_mip_levels(t.width, t.height).min(15))) - 1) as u16;
        t.level_bad = 0;
        t.level_faces = [0x3f; CAP_MIP_LEVELS];
        return;
    }
    if !filtered {
        /* no CPU view of the texels, a packed format, or an image (one
         * level): the texture keeps its base level and samples without
         * mipmaps */
        return;
    }
    /* A backend that keeps level 0 of a render target as its jobs rendered
     * it (the V3D's render image), or that draws the levels into this
     * storage itself (the PowerVR's per-level quad, src/pvr/backend.rs
     * pvr_generate_mipmap), may make the chain on the GPU
     * (backend::generate_mipmap): nothing is filtered or stamped here, the
     * storage's levels 1.. stay as they are until the backend hands the
     * chain back (chain_back, before any CPU read or write of them), and the
     * record says the chain exists, as below. A PowerVR NPOT chain laid out
     * as its power-of-two extent (pot_chain: glmark2's 800x600 terrain) is
     * offered too; the backend knows that layout, it is the one its TPU
     * walks. So is the GC7000's (src/vivante/mipmap.rs gc_generate_mipmap):
     * every level at level 0's pitch (chain_pitch, mip_level_align), and a
     * texture a draw rendered into holds the pixel engine's B,G,R,A
     * (pe_order) -- glmark2's terrain and refract targets, rendered into
     * every frame -- which the bind carries as stride_texels and bgra; the
     * PowerVR and the V3D have neither (no mip_level_align, an RGBA8888
     * target in R,G,B,A order). A level above 0 attached to a
     * framebuffer is read and written by the GPU through the storage, so
     * such a texture keeps the CPU's chain. The clear waiting to be merged
     * into the next draw renders into the texture before this call in GL's
     * order, so it goes out first, as settle_texture sends it. */
    let t = &c.objs_ref().textures[ti];
    if t.target == GL_TEXTURE_2D
        && t.texfmt == GLES_TEXFMT_RGBA8
        && t.es3_fmt == 0
        && t.layers <= 1
        && t.chain_levels >= 2
        && !t.bgra
        && !framebuffer::level_above_zero_attached(c, ti)
    {
        flush_clear_into(c, ti);
        let t = &c.objs_ref().textures[ti];
        if backend::generate_mipmap(&draw::texture_bind_2d(t), t.chain_levels as u32) {
            let t = &mut c.objs().textures[ti];
            t.chain_in_backend = true;
            t.levels = t.chain_levels;
            t.levels_written = t.chain_levels;
            t.level_defined = ((1u32 << u32::from(context::gl_mip_levels(t.width, t.height).min(15))) - 1) as u16;
            t.level_bad = 0;
            t.level_faces = [0x3f; CAP_MIP_LEVELS];
            return;
        }
    }
    /* the chain is rewritten in place: no kick may still read or write it */
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    let t = &mut c.objs().textures[ti];
    // SAFETY: the texture owns its live allocation and no kick uses it any
    // more (settled above); nothing else refers to the bytes while the
    // slice lives.
    let Some(texels) = t.mem.bytes_mut() else {
        return;
    };

    /* Each face/layer filters on its own: the destination row y belongs to
     * face y / dfh, and its source rows start at that face's base. Walking
     * the stacked image as one tall texture blends two faces together once
     * the face height reaches one row. Rows sit at the pair-padded pitch,
     * which matters for the 1-texel-wide tail levels. Float texels (Quick
     * 3D's HDR environment maps) average as floats (upstream 2c48538). */
    let bpp = gles_texfmt_bytes(t.texfmt) as usize;
    /* sRGB texels average in linear space: red, green and blue decoded,
     * averaged and encoded again (ES 3.0.6 3.8.9 leaves the filter to the
     * implementation, which samples in linear space, 3.8.16); alpha is
     * linear already */
    let srgb = t.texfmt == GLES_TEXFMT_SRGB8_A8;
    let comps = if matches!(t.texfmt, GLES_TEXFMT_RGBA8 | GLES_TEXFMT_SRGB8_A8 | GLES_TEXFMT_RGBA16F) {
        4usize
    } else if is_narrow_la(t.texfmt) {
        bpp /* one byte a channel: L, A, or L and A */
    } else {
        1
    };
    let csize = bpp / comps; /* 1 (u8), 2 (half), 4 (float) */
    let layers = t.layers.max(1) as usize;
    let (mut w, mut fh) = (t.width as usize, t.height as usize);

    // SAFETY: the whole CPU mapping of the allocation.
    osal::cache_invd(t.mem.cpu, t.mem.size);
    for l in 1..(t.chain_levels as usize).min(CAP_MIP_LEVELS) {
        let src = t.level_off[l - 1] as usize;
        let dst = t.level_off[l] as usize;
        let (dw, dfh) = (if w > 1 { w / 2 } else { 1 }, if fh > 1 { fh / 2 } else { 1 });
        let (sx, sy) = (if w > 1 { 2usize } else { 1 }, if fh > 1 { 2usize } else { 1 });
        let (sp, dp) = (t.level_pitch_at(l as u32 - 1) as usize, t.level_pitch_at(l as u32) as usize);

        for y in 0..dfh * layers {
            /* the face this destination row belongs to, and the source row
             * its 2x2 box starts at inside that face */
            let sy0 = (y / dfh) * fh + (y % dfh) * sy;

            for x in 0..dw {
                for ch in 0..comps {
                    let d = dst + (y * dp + x) * bpp + ch * csize;
                    let box_src = |j: usize, i: usize| src + ((sy0 + j) * sp + x * sx + i) * bpp + ch * csize;

                    if srgb && ch < 3 {
                        let mut sum = 0.0f32;

                        for j in 0..sy {
                            for i in 0..sx {
                                sum += super::srgb::decode(texels[box_src(j, i)]);
                            }
                        }
                        texels[d] = super::srgb::encode(sum / (sx * sy) as f32);
                        continue;
                    }
                    if csize == 1 {
                        let mut sum: u32 = 0;

                        for j in 0..sy {
                            for i in 0..sx {
                                sum += texels[box_src(j, i)] as u32;
                            }
                        }
                        let n = (sx * sy) as u32;
                        texels[d] = ((sum + n / 2) / n) as u8;
                        continue;
                    }
                    let mut sum = 0.0f32;

                    for j in 0..sy {
                        for i in 0..sx {
                            let p = box_src(j, i);

                            sum += if csize == 2 {
                                draw::half_to_float(u16::from_le_bytes([texels[p], texels[p + 1]]))
                            } else {
                                f32::from_le_bytes([texels[p], texels[p + 1], texels[p + 2], texels[p + 3]])
                            };
                        }
                    }
                    sum /= (sx * sy) as f32;
                    if csize == 2 {
                        texels[d..d + 2].copy_from_slice(&float_to_half(sum).to_le_bytes());
                    } else {
                        texels[d..d + 4].copy_from_slice(&sum.to_le_bytes());
                    }
                }
            }
        }
        w = dw;
        fh = dfh;
    }
    // SAFETY: as above.
    osal::cache_flush(t.mem.cpu, t.mem.size);
    t.levels = t.chain_levels;
    t.levels_written = t.chain_levels; /* glGenerateMipmap filled the chain */
    /* GL's whole chain now exists as far as completeness goes; a NPOT
     * chain this store cannot hold is sampled from its base */
    t.level_defined = ((1u32 << u32::from(super::context::gl_mip_levels(t.width, t.height).min(15))) - 1) as u16;
    t.level_bad = 0;
    t.level_faces = [0x3f; CAP_MIP_LEVELS];
}

/// glGenerateMipmap of a 3D texture (the caller checked the format): each
/// level a 2x2x2 box over the one before it -- a side already 1 long is not
/// halved and its box is one texel along it -- into every level the store
/// holds: the stacked chain, or a grid's levels after level 0
/// (`texture_alloc_storage_3d`). The slices filter into each other, which
/// the per-layer 2D filter of `glGenerateMipmap` must not do for an array.
/// Without a CPU view or a format the box reads (`filtered` false) the chain
/// is recorded as there and keeps what it held, as a 2D one does.
fn generate_mipmap_3d(c: &mut GlContext, ti: usize, filtered: bool) {
    const FUNC: &str = "glGenerateMipmap";
    let t = &c.objs_ref().textures[ti];
    let n = chain_levels_3d(t.width, t.height, u32::from(t.layers)).min(CAP_MIP_LEVELS as u32);
    let held = 1 + (1..n).take_while(|&l| level_held(t, l)).count() as u32;
    if filtered && held > 1 {
        /* the chain is rewritten in place: no kick may still read or write it */
        if !settle_texture(c, ti) {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        }
        let t = &c.objs_ref().textures[ti];
        // SAFETY: the texture owns its live allocation and no kick uses it any
        // more (settled above); nothing else refers to the bytes while the
        // slice lives (`t` is read for the layout only).
        let Some(texels) = (unsafe { t.mem.bytes_mut() }) else {
            return;
        };
        let bpp = gles_texfmt_bytes(t.texfmt) as usize;
        /* sRGB red, green and blue average in linear space, as in the 2D filter */
        let srgb = t.texfmt == GLES_TEXFMT_SRGB8_A8;
        let comps = if matches!(t.texfmt, GLES_TEXFMT_RGBA8 | GLES_TEXFMT_SRGB8_A8 | GLES_TEXFMT_RGBA16F) {
            4usize
        } else if is_narrow_la(t.texfmt) {
            bpp
        } else {
            1
        };
        let csize = bpp / comps; /* 1 (u8), 2 (half), 4 (float) */
        // SAFETY: the whole CPU mapping of the allocation.
        unsafe { osal::cache_invd(t.mem.cpu, t.mem.size) };
        for l in 1..held {
            let (sw, sh, sd) = (pot_level_side(t.width, l - 1), pot_level_side(t.height, l - 1), depth_at(t, l - 1));
            let (dw, dh, dd) = (pot_level_side(t.width, l), pot_level_side(t.height, l), depth_at(t, l));
            /* the box's extent along each axis: 2 where the side halved */
            let (bx, by, bz) = (1 + usize::from(sw > 1), 1 + usize::from(sh > 1), 1 + usize::from(sd > 1));
            let mut srcs = [0usize; 8];
            for z in 0..dd {
                for y in 0..dh {
                    for x in 0..dw {
                        let mut k = 0;
                        for kz in 0..bz as u32 {
                            for ky in 0..by as u32 {
                                for kx in 0..bx as u32 {
                                    srcs[k] = texel_offset_3d(
                                        t,
                                        l - 1,
                                        x * bx as u32 + kx,
                                        y * by as u32 + ky,
                                        z * bz as u32 + kz,
                                    );
                                    k += 1;
                                }
                            }
                        }
                        let d = texel_offset_3d(t, l, x, y, z);
                        for ch in 0..comps {
                            box_average(texels, d + ch * csize, &srcs[..k], ch * csize, csize, srgb && ch < 3);
                        }
                    }
                }
            }
        }
        // SAFETY: as above.
        unsafe { osal::cache_flush(t.mem.cpu, t.mem.size) };
    }
    let t = &mut c.objs().textures[ti];
    /* a grid's levels above 0 are not the backend's (`levels` stays 1) */
    if filtered && held > 1 && t.grid_cols == 0 {
        t.levels = held as u8;
        t.levels_written = held as u8;
    }
    /* GL's whole chain exists as far as completeness goes, held or not */
    t.level_defined = ((1u32 << n.min(15)) - 1) as u16;
    t.level_bad = 0;
}

/// One channel of the box filter: the mean of the `csize`-byte channel at
/// `ch` of the texels at `srcs`, written at `dst` -- rounded to nearest for
/// a byte, as a half or a float otherwise. An `srgb` byte channel is decoded,
/// averaged in linear space and encoded again.
fn box_average(texels: &mut [u8], dst: usize, srcs: &[usize], ch: usize, csize: usize, srgb: bool) {
    if srgb {
        let sum: f32 = srcs.iter().map(|&s| super::srgb::decode(texels[s + ch])).sum();
        texels[dst] = super::srgb::encode(sum / srcs.len() as f32);
        return;
    }
    if csize == 1 {
        let sum: u32 = srcs.iter().map(|&s| u32::from(texels[s + ch])).sum();
        let n = srcs.len() as u32;
        texels[dst] = ((sum + n / 2) / n) as u8;
        return;
    }
    let mut sum = 0.0f32;
    for &s in srcs {
        let p = s + ch;
        sum += if csize == 2 {
            draw::half_to_float(u16::from_le_bytes([texels[p], texels[p + 1]]))
        } else {
            f32::from_le_bytes([texels[p], texels[p + 1], texels[p + 2], texels[p + 3]])
        };
    }
    sum /= srcs.len() as f32;
    if csize == 2 {
        texels[dst..dst + 2].copy_from_slice(&float_to_half(sum).to_le_bytes());
    } else {
        texels[dst..dst + 4].copy_from_slice(&sum.to_le_bytes());
    }
}

/// Is `param` a value GL ES 3.0 accepts for `pname` (the modelled ones)?
pub(super) fn valid_tex_param(pname: GLenum, param: GLint) -> bool {
    let p = param as GLenum;
    match pname {
        GL_TEXTURE_WRAP_R => matches!(p, GL_REPEAT | GL_CLAMP_TO_EDGE | GL_MIRRORED_REPEAT),
        GL_TEXTURE_COMPARE_MODE => matches!(p, GL_NONE | GL_COMPARE_REF_TO_TEXTURE),
        GL_TEXTURE_COMPARE_FUNC => (GL_NEVER..=GL_ALWAYS).contains(&p),
        GL_TEXTURE_MIN_FILTER => matches!(
            p,
            GL_NEAREST
                | GL_LINEAR
                | GL_NEAREST_MIPMAP_NEAREST
                | GL_LINEAR_MIPMAP_NEAREST
                | GL_NEAREST_MIPMAP_LINEAR
                | GL_LINEAR_MIPMAP_LINEAR
        ),
        GL_TEXTURE_MAG_FILTER => matches!(p, GL_NEAREST | GL_LINEAR),
        GL_TEXTURE_WRAP_S | GL_TEXTURE_WRAP_T => matches!(p, GL_REPEAT | GL_CLAMP_TO_EDGE | GL_MIRRORED_REPEAT),
        GL_TEXTURE_SWIZZLE_R | GL_TEXTURE_SWIZZLE_G | GL_TEXTURE_SWIZZLE_B | GL_TEXTURE_SWIZZLE_A => {
            matches!(p, GL_RED | GL_GREEN | GL_BLUE | GL_ALPHA | GL_ZERO | GL_ONE)
        }
        _ => true,
    }
}

/// The texture parameter names ES 3.0 adds to the four of ES 2.0: a
/// `report-es2` build refuses them with GL_INVALID_ENUM in glTexParameter*
/// and glGetTexParameter* alike (the immutable-state names included: they
/// are not in the table either).
///
/// ES 2.0.25 (read from the extracted build/es_full_spec_2.0.25.txt), 3.7.4:
///
/// ```text
/// pname is a symbolic constant indicating the parameter to be set; the
/// possible constants and corresponding parameters are summarized in table 3.10.
/// ```
///
/// Table 3.10 lists TEXTURE_WRAP_S, TEXTURE_WRAP_T, TEXTURE_MIN_FILTER and
/// TEXTURE_MAG_FILTER. 6.1.3, GetTexParameter: "value must be one of the
/// symbolic values in table 3.10."
fn es3_only_tex_param(pname: GLenum) -> bool {
    crate::REPORT_ES2
        && !matches!(pname, GL_TEXTURE_WRAP_S | GL_TEXTURE_WRAP_T | GL_TEXTURE_MIN_FILTER | GL_TEXTURE_MAG_FILTER)
}

/// glTexParameteri on the fetched context (the callers trace glTexParameteri).
fn tex_parameter_i(c: &mut GlContext, target: GLenum, pname: GLenum, param: GLint) {
    tex_parameter(c, target, pname, param, param as GLfloat);
}

/// glTexParameter{i,f}: `iv` the value as an integer, `fv` as a float (the
/// LOD parameters keep the fraction, ES 3.0 3.8.7).
fn tex_parameter(c: &mut GlContext, target: GLenum, pname: GLenum, param: GLint, fv: GLfloat) {
    const FUNC: &str = "glTexParameteri";

    if binding_for(c, target).is_none() || es3_only_tex_param(pname) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if !valid_tex_param(pname, param) {
        c.set_error(GL_INVALID_ENUM, FUNC); /* the value is not one the parameter takes */
        return;
    }
    /* ES 3.0 3.8.7: a negative base or max level is a value error */
    if matches!(pname, GL_TEXTURE_BASE_LEVEL | GL_TEXTURE_MAX_LEVEL) && param < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let t = &mut c.objs().textures[ti];

    match pname {
        GL_TEXTURE_MIN_FILTER => t.min_filter = param as GLenum,
        GL_TEXTURE_MAG_FILTER => t.mag_filter = param as GLenum,
        GL_TEXTURE_WRAP_S => t.wrap_s = param as GLenum,
        GL_TEXTURE_WRAP_T => t.wrap_t = param as GLenum,
        /* The application knows how much of the chain it filled: Qt
         * prefilters a 512 light probe down to 16x16 and caps the sampler
         * here, and sampling past the cap reads memory nobody wrote. */
        GL_TEXTURE_MAX_LEVEL => {
            t.max_level = param.min(MAX_LEVEL_DEFAULT as GLint) as u8;
            t.max_level_set = param;
        }
        /* ES 3.0 parameters: stored and read back, not applied (the sampler
         * state the backends build has no LOD clamps, base level, swizzles
         * or depth compare yet). They were accepted and dropped until
         * 2026-09-24, and 80 dEQP-GLES3 state_query.texture cases read back
         * an error. */
        GL_TEXTURE_WRAP_R => t.wrap_r = param as GLenum,
        GL_TEXTURE_MIN_LOD => t.min_lod = fv,
        GL_TEXTURE_MAX_LOD => t.max_lod = fv,
        GL_TEXTURE_BASE_LEVEL => t.base_level = param,
        GL_TEXTURE_COMPARE_MODE => t.compare_mode = param as GLenum,
        GL_TEXTURE_COMPARE_FUNC => t.compare_func = param as GLenum,
        GL_TEXTURE_SWIZZLE_R => t.swizzle[0] = param as u16,
        GL_TEXTURE_SWIZZLE_G => t.swizzle[1] = param as u16,
        GL_TEXTURE_SWIZZLE_B => t.swizzle[2] = param as u16,
        GL_TEXTURE_SWIZZLE_A => t.swizzle[3] = param as u16,
        _ => c.set_error(GL_INVALID_ENUM, FUNC),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glTexParameteri(target: GLenum, pname: GLenum, param: GLint) {
    gl_trace!(glTexParameteri);
    let Some(c) = context::current() else { return };
    tex_parameter_i(c, target, pname, param);
}

#[no_mangle]
pub unsafe extern "C" fn glTexParameterf(target: GLenum, pname: GLenum, param: GLfloat) {
    gl_trace!(glTexParameterf);
    gl_trace!(glTexParameteri);
    let Some(c) = context::current() else { return };
    tex_parameter(c, target, pname, float_param_as_int(param), param);
}

#[no_mangle]
pub unsafe extern "C" fn glTexParameteriv(target: GLenum, pname: GLenum, params: *const GLint) {
    gl_trace!(glTexParameteriv);
    if params.is_null() {
        return;
    }
    gl_trace!(glTexParameteri);
    let Some(c) = context::current() else { return };
    // SAFETY: non-null; the application passes at least one value (not
    // necessarily aligned).
    tex_parameter_i(c, target, pname, params.read_unaligned());
}

#[no_mangle]
pub unsafe extern "C" fn glTexParameterfv(target: GLenum, pname: GLenum, params: *const GLfloat) {
    gl_trace!(glTexParameterfv);
    if params.is_null() {
        return;
    }
    gl_trace!(glTexParameteri);
    let Some(c) = context::current() else { return };
    // SAFETY: non-null; the application passes at least one value (not
    // necessarily aligned).
    let v = params.read_unaligned();
    tex_parameter(c, target, pname, float_param_as_int(v), v);
}

/// A float given to an integer parameter: rounded to the nearest (ES 3.0
/// 2.3.1), and a float that names an enum is that enum exactly.
fn float_param_as_int(v: GLfloat) -> GLint {
    round_to_int(v)
}

/// The nearest integer, halves away from zero, saturating (no libm here:
/// the crate builds for targets with no operating system).
fn round_to_int(v: GLfloat) -> GLint {
    (if v >= 0.0 { v + 0.5 } else { v - 0.5 }) as GLint
}

/// glGetTexParameteriv's value, or `None` after raising the error.
fn get_tex_parameter_i(c: &mut GlContext, target: GLenum, pname: GLenum) -> Option<GLint> {
    /* the LODs are floats, returned rounded to the nearest integer (ES 3.0 6.1.2) */
    get_tex_parameter(c, target, pname).map(|v| match v {
        TexParam::I(i) => i,
        TexParam::F(f) => round_to_int(f),
    })
}

/// A texture parameter's value in the type it is kept in.
enum TexParam {
    I(GLint),
    F(GLfloat),
}

/// glGetTexParameter{i,f}v's value, or `None` after raising the error.
fn get_tex_parameter(c: &mut GlContext, target: GLenum, pname: GLenum) -> Option<TexParam> {
    const FUNC: &str = "glGetTexParameteriv";

    if binding_for(c, target).is_none() || es3_only_tex_param(pname) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return None;
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return None;
    };
    let t = &c.objs_ref().textures[ti];

    let v = match pname {
        GL_TEXTURE_MIN_FILTER => t.min_filter as GLint,
        GL_TEXTURE_MAG_FILTER => t.mag_filter as GLint,
        GL_TEXTURE_WRAP_S => t.wrap_s as GLint,
        GL_TEXTURE_WRAP_T => t.wrap_t as GLint,
        GL_TEXTURE_IMMUTABLE_FORMAT => t.immutable as GLint,
        /* ES 3.0 6.1.3: the levels glTexStorage* was given; chain_levels is
         * what the store could hold, levels what it has filled */
        GL_TEXTURE_IMMUTABLE_LEVELS => {
            if t.immutable {
                t.immutable_levels as GLint
            } else {
                0
            }
        }
        GL_TEXTURE_WRAP_R => t.wrap_r as GLint,
        GL_TEXTURE_BASE_LEVEL => t.base_level,
        GL_TEXTURE_MAX_LEVEL => t.max_level_set,
        GL_TEXTURE_COMPARE_MODE => t.compare_mode as GLint,
        GL_TEXTURE_COMPARE_FUNC => t.compare_func as GLint,
        GL_TEXTURE_SWIZZLE_R => t.swizzle[0] as GLint,
        GL_TEXTURE_SWIZZLE_G => t.swizzle[1] as GLint,
        GL_TEXTURE_SWIZZLE_B => t.swizzle[2] as GLint,
        GL_TEXTURE_SWIZZLE_A => t.swizzle[3] as GLint,
        GL_TEXTURE_MIN_LOD => return Some(TexParam::F(t.min_lod)),
        GL_TEXTURE_MAX_LOD => return Some(TexParam::F(t.max_lod)),
        _ => {
            c.set_error(GL_INVALID_ENUM, FUNC);
            return None;
        }
    };
    Some(TexParam::I(v))
}

#[no_mangle]
pub unsafe extern "C" fn glGetTexParameteriv(target: GLenum, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetTexParameteriv);
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if let Some(v) = get_tex_parameter_i(c, target, pname) {
        // SAFETY: non-null; the application passes room for one value (not
        // necessarily aligned).
        params.write_unaligned(v);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetTexParameterfv(target: GLenum, pname: GLenum, params: *mut GLfloat) {
    gl_trace!(glGetTexParameterfv);
    if params.is_null() {
        return;
    }
    gl_trace!(glGetTexParameteriv);
    /* on an error (or without a context) nothing is written: "If the
     * generating command modifies values through a pointer argument, no
     * change is made to these values" (ES 2.0.25 2.5) */
    let v = match context::current().and_then(|c| get_tex_parameter(c, target, pname)) {
        Some(TexParam::I(i)) => i as GLfloat,
        Some(TexParam::F(f)) => f,
        None => return,
    };
    // SAFETY: non-null; the application passes room for one value (not
    // necessarily aligned).
    params.write_unaligned(v);
}

/// EGLImage target: the bound TEXTURE_2D takes `mem` (w x h, rows at the
/// pair-padded pitch, `surf_format` = GLES_SURF_*) as its storage without
/// owning it. -1 (texture unchanged) when there is no such texture, it is
/// immutable, or `mem` is smaller than those rows: the draws, uploads and
/// read-backs of the texture all walk that pitch, so a buffer with packed
/// odd-width rows would be sampled skewed and written past its end.
pub fn texture_attach_external(c: &mut GlContext, mem: &OsalGpuMem, w: u32, h: u32, surf_format: u8) -> i32 {
    texture_attach_external_pitched(c, mem, w, h, surf_format, 0)
}

/// [`texture_attach_external`] of storage laid out `pitch` pixels a row (0:
/// the pair-padded pitch), as an EGLImage's source left it: the texture
/// reads it at that pitch (`GlTexture::chain_pitch`).
pub fn texture_attach_external_pitched(
    c: &mut GlContext,
    mem: &OsalGpuMem,
    w: u32,
    h: u32,
    surf_format: u8,
    pitch: u32,
) -> i32 {
    let Some(ti) = active_2d_texture(c) else { return -1 };
    let bpp = if surf_format == GLES_SURF_RGB565 { 2 } else { 4 };
    let rows = if pitch == 0 { tex_pitch(w) } else { pitch };
    if w == 0
        || h == 0
        || w > context::MAX_SURFACE_SIDE
        || h > context::MAX_SURFACE_SIDE
        || mem.impl_.is_null()
        || rows < tex_pitch(w)
        || (mem.size as u64) < rows as u64 * h as u64 * bpp
        || c.objs().textures[ti].immutable
    {
        return -1;
    }
    let t = &mut c.objs().textures[ti];

    context::release_storage(&mut t.mem, &mut t.external);
    t.mem = *mem;
    t.external = true;
    t.storage_deferred = false;
    stamp_texels(t); /* 0: an EGLImage's storage is never cached */
    t.storage_gen = context::respecified(t.serial, true);
    t.chain_in_backend = false;
    t.bgra = surf_format == GLES_SURF_XRGB8888;
    /* the image's own order says whether a render wrote it (the caller sets
     * it from the image, egl.rs); a native buffer's is the CPU's */
    t.pe_order = false;
    t.texfmt = if surf_format == GLES_SURF_RGB565 { GLES_TEXFMT_RGB565 } else { GLES_TEXFMT_RGBA8 };
    t.es3_fmt = 0;
    t.rgba32f = false;
    t.unsized_depth = false;
    t.block_w = 0;
    t.block_h = 0;
    t.block_bytes = 0;
    t.width = w;
    t.height = h;
    t.format = if surf_format == GLES_SURF_RGB565 { GL_RGB } else { GL_RGBA };
    t.type_ = if surf_format == GLES_SURF_RGB565 { GL_UNSIGNED_SHORT_5_6_5 } else { GL_UNSIGNED_BYTE };
    t.level_off[0] = 0;
    /* one level, at the pitch its source laid it out (0: the pair-padded
     * one) */
    t.chain_pitch = if rows == tex_pitch(w) { 0 } else { rows };
    t.pot_chain = false;
    t.layers = 1;
    t.bpp = gles_texfmt_bytes(t.texfmt) as u8;
    t.chain_levels = 1; /* one level: the image is a single 2D buffer */
    t.levels = 1;
    t.levels_written = 1;
    t.level_defined = 1;
    t.level_bad = 0;
    t.max_level = MAX_LEVEL_DEFAULT;
    0
}

/// The texture [`texture_attach_external`] just gave an image's storage
/// holds that image's channel order (`EglImage::pe_order`, egl.rs).
pub(crate) fn set_bound_2d_pe_order(c: &mut GlContext, pe_order: bool) {
    if let Some(ti) = active_2d_texture(c) {
        c.objs().textures[ti].pe_order = pe_order;
    }
}

/// The C name of [`texture_attach_external`], on the current context.
///
/// `_owner` is the EGLImage, for source compatibility with the C library's
/// signature; this port finds the storage record from `mem` (see
/// `gl_internal.h`), so the argument is not used.
///
/// # Safety
/// `mem` is null or a valid descriptor; `_owner` is not dereferenced.
#[no_mangle]
pub unsafe extern "C" fn gl_texture_attach_external(
    mem: *const OsalGpuMem,
    w: u32,
    h: u32,
    surf_format: u8,
    _owner: *mut c_void,
) -> i32 {
    // SAFETY: null or a valid descriptor (the caller's contract).
    let (Some(c), Some(mem)) = (context::current(), mem.as_ref()) else {
        return -1;
    };
    texture_attach_external(c, mem, w, h, surf_format)
}

#[no_mangle]
pub unsafe extern "C" fn glIsTexture(texture: GLuint) -> GLboolean {
    gl_trace!(glIsTexture);
    /* a generated name is a texture only once bound (it then has a target) */
    match context::current().and_then(|c| names::slot_of(c, Kind::Texture, texture).and_then(|s| c.texture_object(s))) {
        Some(_) => GL_TRUE_B,
        None => GL_FALSE_B,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteTextures(n: GLsizei, textures: *const GLuint) {
    gl_trace!(glDeleteTextures);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glDeleteTextures");
        return;
    }
    let Some(c) = context::current() else { return };
    if textures.is_null() {
        return;
    }
    for i in 0..n as usize {
        // SAFETY: the application passes n names (GL contract), n >= 0; a
        // GLuint array is not assumed aligned.
        let Some(h) = names::slot_of(c, Kind::Texture, textures.add(i).read_unaligned()) else {
            continue;
        };
        if !c.objs().textures[h as usize].used {
            continue;
        }
        names::forget(c, Kind::Texture, h);
        unbind_texture(c, h);
        if framebuffer::texture_deleted(c, h) {
            /* still held by a framebuffer that is not bound: the image
             * stays, the name goes (framebuffer.rs, the orphan rule) */
            c.objs().textures[h as usize].orphan = true;
        } else {
            free_texture(c, h);
        }
    }
}

/// Take `h` off every unit it is bound to.
fn unbind_texture(c: &mut GlContext, h: GLuint) {
    for u in 0..CAP_COMBINED_TEXTURE_UNITS {
        if c.bound_texture_2d[u] == h {
            c.bound_texture_2d[u] = 0;
        }
        if c.bound_texture_cube[u] == h {
            c.bound_texture_cube[u] = 0;
        }
        if c.bound_texture_2d_array[u] == h {
            c.bound_texture_2d_array[u] = 0;
        }
        if c.bound_texture_3d[u] == h {
            c.bound_texture_3d[u] = 0;
        }
    }
}

/// Free a texture's slot: its storage and its name (the units it was on
/// were cleared by the delete).
pub(super) fn free_texture(c: &mut GlContext, h: GLuint) {
    let t = &mut c.objs().textures[h as usize];
    if !t.used {
        return;
    }
    crate::backend::attachment_gone(context::ds_object_id(t.serial, true));
    /* the OSAL defers the free past the kicks that still sample it */
    context::release_storage(&mut t.mem, &mut t.external);
    zero_texture(t);
    t.max_level = MAX_LEVEL_DEFAULT; /* GL's default is "no clamp" */
}

/// glTexImage3D's record of level `level` of a 3D texture, and its storage:
/// level 0 lays the chain out ([`texture_alloc_storage_3d`]); a level above
/// it of the chain's size and level 0's format is recorded as defined, and
/// the caller writes its texels. A level of another size or format is
/// recorded as there and wrong (`level_bad`: the texture is incomplete under
/// a mipmap filter while it stands), its texels held nowhere; a 1x1x1 level
/// past a chain that already ran down to 1x1x1 is ignored, as for 2D.
///
/// A level before any level 0 lays out the chain whose level 0 is it doubled
/// `level` times, as glTexImage2D does (dEQP random_teximage2d uploads levels
/// in random order), and a later level 0 of that chain keeps the levels
/// already there. False when the storage could not be had.
fn tex_image_3d_level(
    t: &mut GlTexture,
    level: u32,
    internalformat: GLint,
    (w, h, d): (u32, u32, u32),
    format: GLenum,
    type_: GLenum,
) -> bool {
    let texfmt = texfmt_of(internalformat);
    let es3 = es3_index(internalformat as GLenum);
    let bit = 1u16 << level.min(15);
    let same_format = |t: &GlTexture| t.format == format && t.es3_fmt == es3 && t.texfmt == texfmt;

    if level == 0 {
        let laid_out_for_it = !t.mem.impl_.is_null()
            && t.level_defined & 1 == 0
            && !t.immutable
            && !t.external
            && t.block_bytes == 0
            && (w, h, d) == (t.width, t.height, u32::from(t.layers))
            && w > 0
            && h > 0
            && d > 0
            && same_format(t);
        /* levels before it that laid out another chain: the ones that fit
         * this level 0's chain stay */
        let before_it = !t.mem.impl_.is_null()
            && t.level_defined & 1 == 0
            && t.level_defined != 0
            && !t.immutable
            && same_format(t);
        if !laid_out_for_it {
            let ok = if before_it {
                relayout_3d_keeping_levels(t, w, h, d, texfmt)
            } else {
                texture_alloc_storage_3d(t, w, h, d, texfmt) == 0
            };
            if !ok {
                return false;
            }
            t.format = format;
            t.es3_fmt = es3;
            t.rgba32f = is_rgba32f(internalformat as GLenum);
            t.unsized_depth = is_unsized_depth(internalformat as GLenum);
        }
        t.level_defined |= 1;
        t.level_bad = (t.level_bad & !1) | u16::from(w == 0 || h == 0 || d == 0);
        t.type_ = type_;
        return true;
    }
    let (w0, h0, d0) = (w << level, h << level, d << level);
    let side = MAX_3D_SIDE as u32;
    let implied = w > 0 && h > 0 && d > 0 && w0 <= side && h0 <= side && d0 <= side;
    if implied && t.mem.impl_.is_null() && t.level_defined & 1 == 0 {
        if texture_alloc_storage_3d(t, w0, h0, d0, texfmt) != 0 {
            return false;
        }
        t.level_defined = 0;
        t.format = format;
        t.type_ = type_;
        t.es3_fmt = es3;
        t.rgba32f = is_rgba32f(internalformat as GLenum);
        t.unsized_depth = is_unsized_depth(internalformat as GLenum);
    } else if implied
        && !t.mem.impl_.is_null()
        && t.level_defined & 1 == 0
        && !t.immutable
        && same_format(t)
        && (!level_held(t, level) || (w, h, d) != (level_size(t, level).0, level_size(t, level).1, depth_at(t, level)))
    {
        /* a later level before level 0 that does not fit the chain the first
         * one implied: the chain is the one THIS level implies, keeping the
         * levels that fit it too */
        if !relayout_3d_keeping_levels(t, w0, h0, d0, texfmt) {
            return false;
        }
    }
    let (lw, lh) = level_size(t, level);
    let ld = depth_at(t, level);
    if !t.mem.impl_.is_null()
        && t.block_bytes == 0
        && level_held(t, level)
        && (w, h, d) == (lw, lh, ld)
        && same_format(t)
    {
        t.level_defined |= bit;
        t.level_bad &= !bit;
        /* a grid's levels above 0 are not the backend's (`levels` stays 1) */
        if t.grid_cols == 0 {
            if level + 1 > u32::from(t.levels) {
                t.levels = (level + 1) as u8;
            }
            if u32::from(t.levels_written) < level + 1 {
                t.levels_written = (level + 1) as u8;
            }
        }
        return true;
    }
    let past_the_chain = level >= chain_levels_3d(t.width, t.height, u32::from(t.layers))
        && (lw, lh, ld) == (1, 1, 1)
        && (w, h, d) == (1, 1, 1);
    if !past_the_chain {
        t.level_defined |= bit;
        t.level_bad |= bit;
    }
    true
}

/// 2D texture arrays: `depth` layers stacked in one linear texture; 3D
/// textures: `depth` slices tiled in a grid, or stacked where the backend
/// walks the stack, with every level of the chain
/// ([`texture_alloc_storage_3d`]; `GlTexture::layers` / `grid_cols`).
#[no_mangle]
pub unsafe extern "C" fn glTexImage3D(
    target: GLenum,
    level: GLint,
    internalformat: GLint,
    width: GLsizei,
    height: GLsizei,
    depth: GLsizei,
    border: GLint,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) {
    const FUNC: &str = "glTexImage3D";
    gl_trace!(glTexImage3D);
    let Some(c) = context::current() else { return };
    /* With PIXEL_UNPACK_BUFFER bound, `pixels` is an offset into that buffer
     * rather than a client address (ES 3.0 3.7.1). */
    let Some(pixels) = unpack_source(c, pixels, width, height, depth, format, type_, "glTexImage3D") else {
        return;
    };
    if target != GL_TEXTURE_2D_ARRAY && target != GL_TEXTURE_3D {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* ES 3.0 3.8.3: a level up to log2 of the target's largest size, sides
     * up to that size, and up to MAX_ARRAY_TEXTURE_LAYERS layers (dEQP
     * negative_api.texture.teximage3d_max_level, _max_width_height_depth) */
    let (max_side, max_level) =
        if target == GL_TEXTURE_3D { (MAX_3D_SIDE, MAX_3D_LEVELS) } else { (4096, CAP_MIP_LEVELS as GLint) };
    if level < 0
        || level >= max_level
        || width < 0
        || height < 0
        || width > max_side
        || height > max_side
        || !(0..=MAX_3D_SIDE).contains(&depth)
        || border != 0
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if !check_format_type(c, format, type_, FUNC) {
        return;
    }
    /* the internal format's rules, as glTexImage2D applies them: a name this
     * GL does not have is GL_INVALID_VALUE, an unsized one naming another
     * base format than `format` is GL_INVALID_OPERATION, and a depth format
     * has no 3D image at all (3.8.3; dEQP negative_api.texture.teximage3d) */
    if format == GL_DEPTH_COMPONENT || format == GL_DEPTH_STENCIL {
        if target == GL_TEXTURE_3D {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
    } else {
        if !es3_image_check(c, internalformat as GLenum, format, type_, FUNC) {
            return;
        }
        if sized_format(internalformat as GLenum).is_none() {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return;
        }
        if is_texture_format(internalformat as GLenum) && internalformat as GLenum != format {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if c.objs().textures[ti].immutable {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    note_target_use(c, ti);
    let t = &mut c.objs().textures[ti];
    if target == GL_TEXTURE_3D {
        if !tex_image_3d_level(
            t,
            level as u32,
            internalformat,
            (width as u32, height as u32, depth as u32),
            format,
            type_,
        ) {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        }
        if !level_held(&c.objs_ref().textures[ti], level as u32)
            || c.objs_ref().textures[ti].level_bad & (1u16 << level) != 0
        {
            return; /* recorded, of no size or format the chain has */
        }
    } else if level == 0 {
        if texture_alloc_storage(t, width as u32, height as u32, depth as u32, texfmt_of(internalformat)) != 0 {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        }
        t.format = format;
        t.type_ = type_;
        t.es3_fmt = es3_index(internalformat as GLenum);
        t.rgba32f = is_rgba32f(internalformat as GLenum);
        t.unsized_depth = is_unsized_depth(internalformat as GLenum);
    } else {
        let (lw, lh) = level_size(t, level as u32);

        /* A level of a non-power-of-two array's chain (ES 3.0 3.8.14: floor
         * sizes down to 1x1, the layer count the same at every level). The
         * store has one level of such an array until a backend that holds
         * the chain is asked for it (grow_npot_chain); one that cannot (the
         * PowerVR's stacked NPOT layers, npot_mip_chains) records the level
         * as GL has it and keeps no texels, as for a NPOT cube. */
        let npot_level = t.level_defined & 1 != 0
            && t.level_bad & 1 == 0
            && t.chain_levels == 1
            && t.grid_cols == 0
            && !(is_pot(t.width) && is_pot(t.height))
            && (level as u32) < u32::from(context::gl_mip_levels(t.width, t.height))
            && (width as u32, height as u32, depth as u32) == (lw, lh, t.layers as u32)
            && t.format == format;
        if npot_level && npot_mip_chains(t.layers as u32) {
            if !settle_texture(c, ti) {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return;
            }
            let mut oom = false;
            grow_npot_chain(&mut c.objs().textures[ti], &mut oom);
            if oom {
                c.set_error(GL_OUT_OF_MEMORY, FUNC);
                return;
            }
        }
        let t = &mut c.objs().textures[ti];
        /* Only the PowerVR's tall stacked storage gets here (npot_mip_chains
         * false: layers > 1, no native layers): a stacked NPOT array cannot
         * hold its chain, so the level is recorded and its texels dropped,
         * and draws sample level 0. Left as it is on purpose: plan item
         * pvr-2darray (docs/es3/pvr-2darray.md) lays PowerVR arrays out
         * layer-major, and their levels >= 1 get storage there. Every other
         * array keeps the texels of each level (grown above, or a
         * power-of-two chain). */
        if npot_level && t.chain_levels == 1 {
            t.level_defined |= 1u16 << (level as u32).min(15);
            t.level_bad &= !(1u16 << (level as u32).min(15));
            return;
        }
        if level as u32 >= t.chain_levels as u32
            || width as u32 != lw
            || height as u32 != lh
            || depth as u32 != t.layers as u32
        {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return;
        }
        if level as u32 + 1 > t.levels as u32 {
            t.levels = (level + 1) as u8;
        }
        if (t.levels_written as u32) < level as u32 + 1 {
            t.levels_written = (level + 1) as u8;
        }
        t.level_defined |= 1u16 << (level as u32).min(15);
    }
    if pixels.is_null() || width == 0 || height == 0 || depth == 0 || c.objs_ref().textures[ti].mem.impl_.is_null() {
        return;
    }
    gl_trace!(glTexSubImage3D); /* the C uploads through the entry point */
    tex_sub_image_3d(c, target, level, 0, 0, 0, width, height, depth, format, type_, pixels);
}

/// glTexSubImage3D on the fetched context (the callers trace it).
///
/// # Safety
/// `pixels` is null or holds `depth` slices of `height` rows, as for
/// glTexSubImage3D.
#[allow(clippy::too_many_arguments)]
unsafe fn tex_sub_image_3d(
    c: &mut GlContext,
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    zoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    depth: GLsizei,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) {
    const FUNC: &str = "glTexSubImage3D";
    if target != GL_TEXTURE_2D_ARRAY && target != GL_TEXTURE_3D {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* an ES 3.0 *_INTEGER format is a known enum: the pairing it is in
     * is GL_INVALID_OPERATION (check_format_type), not GL_INVALID_ENUM
     * (dEQP-GLES3 negative_api.texture.texsubimage2d/3d) */
    if !is_pixel_format_enum(format) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* ES 3.0.6 3.8.3: "Textures with a base internal format of
     * DEPTH_COMPONENT or DEPTH_STENCIL are supported by texture image
     * specification commands only if target is TEXTURE_2D,
     * TEXTURE_2D_ARRAY, or TEXTURE_CUBE_MAP. Using these formats in
     * conjunction with any other target will result in an
     * INVALID_OPERATION error." This refused every depth format with
     * GL_INVALID_ENUM until 2026-09-26, and dEQP's 2D array shadow
     * textures are uploaded here: 144 of the AM62P's texture.shadow cases
     * (RESULTS 1r). */
    if matches!(format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL) && target == GL_TEXTURE_3D {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    if !check_format_type(c, format, type_, FUNC) {
        return;
    }
    /* a 3D texture's chain ends at log2(GL_MAX_3D_TEXTURE_SIZE): a level
     * past it is GL_INVALID_VALUE before the storage is asked (dEQP-GLES3
     * negative_api.texture.texsubimage3d_max_level, level 9 on an unbound
     * 3D target, was GL_INVALID_OPERATION) */
    if level < 0
        || level >= CAP_MIP_LEVELS as GLint
        || (target == GL_TEXTURE_3D && level >= MAX_3D_LEVELS)
        || width < 0
        || height < 0
        || depth < 0
        || xoffset < 0
        || yoffset < 0
        || zoffset < 0
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let ti = active_texture(c, target).filter(|&ti| {
        let t = &c.objs_ref().textures[ti];
        texels_writable(t) && ((level as u32) < t.levels as u32 || recorded_unheld(t, level))
    });
    let Some(ti) = ti else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if !es3_sub_image_check(c, ti, format, type_, FUNC) {
        return;
    }
    let t = &c.objs_ref().textures[ti];
    let grid_cols = t.grid_cols as u32;
    /* a level the store keeps no texels for (a PowerVR NPOT array's above
     * 0, see glTexImage3D); a 3D grid's levels above 0 are held after level
     * 0's grid though the backend is told of level 0 alone */
    let unheld = recorded_unheld(t, level) && !grid_level_held(t, level as u32);
    let (lw, lh) = level_size(t, level as u32);
    /* a 3D texture's depth halves with its sides, an array's layers do not */
    let layers = if target == GL_TEXTURE_3D { depth_at(t, level as u32) } else { u32::from(t.layers) };
    /* u32 sums that wrap like the C */
    if (xoffset as u32).wrapping_add(width as u32) > lw
        || (yoffset as u32).wrapping_add(height as u32) > lh
        || (zoffset as u32).wrapping_add(depth as u32) > layers
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if pixels.is_null() || width == 0 || height == 0 || depth == 0 || unheld {
        return;
    }
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    /* the slices follow each other in the source (GL_UNPACK_IMAGE_HEIGHT 0) */
    let slice = src_slice_bytes(c, width as u32, height as u32, format, type_);
    let mut ok = true;

    /* each slice into its place in the level: under the previous one, or in
     * the level's grid (slice_origin) */
    for z in 0..depth as u32 {
        let (rw, x0, y0) = slice_origin(&c.objs_ref().textures[ti], level as u32, (zoffset as u32).wrapping_add(z));
        ok &= texture_write_rect(
            c,
            ti,
            level as u32,
            rw,
            (xoffset as u32).wrapping_add(x0),
            (yoffset as u32).wrapping_add(y0),
            width as u32,
            height as u32,
            (pixels as *const u8).wrapping_add((z as usize).wrapping_mul(slice)) as *const c_void,
            format,
            type_,
        );
    }
    if !ok {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    /* a grid's levels above 0 are not the backend's (`levels_written` is
     * the backend's sampling bound) */
    if grid_cols != 0 {
        return;
    }
    note_level_written(&mut c.objs().textures[ti], level);
}

#[no_mangle]
pub unsafe extern "C" fn glTexSubImage3D(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    zoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    depth: GLsizei,
    format: GLenum,
    type_: GLenum,
    pixels: *const c_void,
) {
    gl_trace!(glTexSubImage3D);
    let Some(c) = context::current() else { return };
    /* With PIXEL_UNPACK_BUFFER bound, `pixels` is an offset into that buffer
     * rather than a client address (ES 3.0 3.7.1). */
    let Some(pixels) = unpack_source(c, pixels, width, height, depth, format, type_, "glTexSubImage3D") else {
        return;
    };
    tex_sub_image_3d(c, target, level, xoffset, yoffset, zoffset, width, height, depth, format, type_, pixels);
}

#[no_mangle]
pub unsafe extern "C" fn glCompressedTexImage3D(
    target: GLenum,
    level: GLint,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
    depth: GLsizei,
    border: GLint,
    imageSize: GLsizei,
    data: *const c_void,
) {
    gl_trace!(glCompressedTexImage3D);
    const FUNC: &str = "glCompressedTexImage3D";
    let Some(c) = context::current() else { return };
    /* ES 3.0 3.8.6's argument errors come before the storage question
     * (dEQP negative_api.texture.compressedteximage3d*) */
    if target != GL_TEXTURE_2D_ARRAY && target != GL_TEXTURE_3D {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let Some(bb) = compressed_block_bytes(internalformat) else {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    };
    let max_level = if target == GL_TEXTURE_3D { MAX_3D_LEVELS } else { CAP_MIP_LEVELS as GLint };
    if level < 0
        || level >= max_level
        || width < 0
        || height < 0
        || depth < 0
        || border != 0
        || imageSize < 0
        || width > 4096
        || height > 4096
        || depth > MAX_3D_SIDE
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if compressed_image_bytes(width, height, depth, bb) != Some(imageSize as usize) {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* the ETC2/EAC formats have no 3D form (ES 3.0 3.8.6) */
    if target == GL_TEXTURE_3D {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let Some(data) = super::buffer::pixel_buffer_resolve(c, false, data, imageSize as usize, FUNC) else {
        return;
    };
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if c.objs().textures[ti].immutable {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* A 2D array: its layers are laid out as a cube map's faces are, one
     * after the other within each level (compressed_chain_layout). Until
     * 2026-09-24 there was no compressed array storage and every call ended
     * in GL_INVALID_OPERATION; ten dEQP-GLES3 negative_api cases could not
     * set up their texture. */
    let dst = CompressedDest { layers: depth as u32, first: 0, count: depth as u32, face: false };
    compressed_tex_image(c, ti, dst, level, internalformat, width, height, data, FUNC);
}

#[no_mangle]
pub unsafe extern "C" fn glCompressedTexSubImage3D(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    zoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    depth: GLsizei,
    format: GLenum,
    imageSize: GLsizei,
    data: *const c_void,
) {
    gl_trace!(glCompressedTexSubImage3D);
    const FUNC: &str = "glCompressedTexSubImage3D";
    let Some(c) = context::current() else { return };
    /* the argument rules first, as for glCompressedTexImage3D */
    if target != GL_TEXTURE_2D_ARRAY && target != GL_TEXTURE_3D {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if compressed_block_bytes(format).is_none() {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let max_level = if target == GL_TEXTURE_3D { MAX_3D_LEVELS } else { CAP_MIP_LEVELS as GLint };
    if level < 0
        || level >= max_level
        || width < 0
        || height < 0
        || depth < 0
        || xoffset < 0
        || yoffset < 0
        || zoffset < 0
        || imageSize < 0
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* no ETC2/EAC 3D texture exists to write into */
    if target == GL_TEXTURE_3D {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let Some(data) = super::buffer::pixel_buffer_resolve(c, false, data, imageSize as usize, FUNC) else {
        return;
    };
    compressed_tex_sub_image(
        c,
        target,
        level,
        (xoffset, yoffset, zoffset),
        (width, height, depth),
        format,
        imageSize,
        data,
        FUNC,
    );
}

/// One slice of an array or 3D texture from the read framebuffer. The slices
/// are stacked, so the destination is the 2D copy's with the slice's rows
/// added to yoffset -- the same arithmetic gl/framebuffer.rs uses to render
/// into one.
#[no_mangle]
pub unsafe extern "C" fn glCopyTexSubImage3D(
    target: GLenum,
    level: GLint,
    xoffset: GLint,
    yoffset: GLint,
    zoffset: GLint,
    x: GLint,
    y: GLint,
    width: GLsizei,
    height: GLsizei,
) {
    const FUNC: &str = "glCopyTexSubImage3D";
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if let Some(c) = context::current() {
        if super::framebuffer::as_read(c, || unsafe {
            glCopyTexSubImage3D(target, level, xoffset, yoffset, zoffset, x, y, width, height)
        })
        .is_some()
        {
            return;
        }
    }
    /* the read framebuffer's glReadBuffer attachment */
    if let Some(c) = context::current() {
        if super::framebuffer::as_read_attachment(c, || unsafe {
            glCopyTexSubImage3D(target, level, xoffset, yoffset, zoffset, x, y, width, height)
        })
        .is_some()
        {
            return;
        }
    }
    gl_trace!(glCopyTexSubImage3D);
    let Some(c) = context::current() else { return };

    if target != GL_TEXTURE_3D && target != GL_TEXTURE_2D_ARRAY {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* a 3D image has log2(GL_MAX_3D_TEXTURE_SIZE) + 1 levels (dEQP
     * negative_api.texture.*3d_max_level) */
    if target == GL_TEXTURE_3D && level >= MAX_3D_LEVELS {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* a 3D image has log2(GL_MAX_3D_TEXTURE_SIZE) + 1 levels (dEQP
     * negative_api.texture.*3d_max_level) */
    if target == GL_TEXTURE_3D && level >= MAX_3D_LEVELS {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if level < 0
        || level >= CAP_MIP_LEVELS as GLint
        || width < 0
        || height < 0
        || xoffset < 0
        || yoffset < 0
        || zoffset < 0
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let ti = active_texture(c, target).filter(|&ti| {
        let t = &c.objs_ref().textures[ti];
        /* a level the backend samples, or a 3D grid's level above 0, which
         * the store holds (texture_alloc_storage_3d) */
        texels_writable(t)
            && t.block_bytes == 0
            && ((level as u32) < t.levels as u32
                || (grid_level_held(t, level as u32) && t.level_defined & (1u16 << level) != 0))
    });
    let Some(ti) = ti else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if es3_formats::by_index(c.objs_ref().textures[ti].es3_fmt).is_some_and(|e| !es3_copy_compatible(c, e)) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let (lw, lh) = level_size(&c.objs_ref().textures[ti], level as u32);
    /* a 3D level's depth halves with its sides, an array's layers do not */
    let layers = if target == GL_TEXTURE_3D {
        depth_at(&c.objs_ref().textures[ti], level as u32)
    } else {
        c.objs().textures[ti].layers.max(1) as u32
    };
    if xoffset as u32 + width as u32 > lw || yoffset as u32 + height as u32 > lh || zoffset as u32 >= layers {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if !framebuffer::fb_target_ok(c) {
        c.set_error(GL_INVALID_FRAMEBUFFER_OPERATION, FUNC);
        return;
    }
    if width == 0 || height == 0 {
        return;
    }
    if !settle_texture(c, ti) {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    let format = match c.objs().textures[ti].format {
        0 => GL_RGBA,
        f => f,
    };
    /* the slice's place in the level: under the previous one, or in the
     * level's grid */
    let (rw, x0, y0) = slice_origin(&c.objs_ref().textures[ti], level as u32, zoffset as u32);
    // SAFETY: the destination lies inside the level's storage (checked
    // above), which is writable (`texels_writable`).
    let ok = unsafe {
        texture_copy_from_fb(
            c,
            ti,
            level as u32,
            rw,
            x0 + xoffset as u32,
            y0 + yoffset as u32,
            x,
            y,
            width as u32,
            height as u32,
            format,
        )
    };
    if !ok {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
    }
}

/// The (format, type) an ES 3.0 sized internal format unpacks from, and the
/// texel format it is stored in (RGBA8 for everything but the float and sRGB
/// ones).
fn sized_format(internalformat: GLenum) -> Option<(GLenum, GLenum, u8)> {
    let (u8_, rgba8) = (GL_UNSIGNED_BYTE, GLES_TEXFMT_RGBA8);
    /* ES 3.0's sRGB pair: the RGBA8 bytes, sampled decoded (SRGB8 widened
     * with alpha 255, as RGB8 is). Every back end samples the class
     * (`backend::sampled_texfmts`). The ES 2.0 report has no sRGB format
     * (glTexImage2D refuses the names) and glTexStorage2D's keep RGBA8 there,
     * as they always did. */
    let srgb = if crate::REPORT_ES2 { rgba8 } else { GLES_TEXFMT_SRGB8_A8 };
    Some(match internalformat {
        GL_SRGB8_ALPHA8 => (GL_RGBA, u8_, srgb),
        GL_SRGB8 => (GL_RGB, u8_, srgb),
        GL_RGBA8 | GL_RGBA4 | GL_RGB5_A1 | GL_RGBA => (GL_RGBA, u8_, rgba8),
        GL_RGB8 | GL_RGB565 | GL_RGB => (GL_RGB, u8_, rgba8),
        GL_R8 => (GL_RED, u8_, rgba8),
        GL_RG8 => (GL_RG, u8_, rgba8),
        GL_RGBA16F => (GL_RGBA, GL_HALF_FLOAT, GLES_TEXFMT_RGBA16F),
        GL_RGBA32F => (GL_RGBA, GL_FLOAT, GLES_TEXFMT_RGBA16F), /* stored as halves */
        GL_R16F => (GL_RED, GL_HALF_FLOAT, GLES_TEXFMT_R16F),
        GL_R32F => (GL_RED, GL_FLOAT, GLES_TEXFMT_R32F),
        GL_RG16F => (GL_RG, GL_HALF_FLOAT, GLES_TEXFMT_RGBA16F),
        /* GL_EXT_texture_rg's unsized pair, alongside ES 2.0's own unsized
         * names. The texels go into the RGBA8 store as (r, 0, 0, 1) and
         * (r, g, 0, 1), which is what the extension says they sample as
         * (convert_row already did it for the upload); a half-float upload
         * into one is clamped into those bytes, as an unsized GL_RGBA
         * half-float upload is. Without these `sized_format` returned None
         * and glTexImage2D answered GL_INVALID_VALUE to a format the
         * advertised extension requires it to take -- twelve
         * fbo.completeness.renderable cases. */
        GL_ALPHA | GL_LUMINANCE | GL_LUMINANCE_ALPHA | GL_RED | GL_RG => (internalformat, u8_, rgba8),
        /* GL_EXT_texture_format_BGRA8888: an unsized internal format of its
         * own, so glTexImage2D's "internalformat must equal format" rule
         * takes it as it takes GL_RGBA. The texels are stored R,G,B,A like
         * every other colour texture -- convert_row does the swap -- so
         * nothing downstream of the upload knows about it. */
        /* GL_BGRA8_EXT is the same thing sized. dEQP's FBO completeness
         * requires both names from an implementation that advertises the
         * extension, and we advertise it: rejecting the sized one gave
         * GL_INVALID_VALUE where a complete framebuffer was required. */
        GL_BGRA_EXT | GL_BGRA8_EXT => (GL_BGRA_EXT, u8_, rgba8),
        /* Depth textures: one float per texel, the depth in [0, 1] (R32F
         * storage, which every backend samples as (d, 0, 0, 1): "Depth
         * textures ... can be treated as RED textures during texture
         * filtering and application", ES 3.0.6 3.8.8), so a shadow look-up compares
         * the value that was uploaded. The type is the format's own transfer
         * type (ES 3.0 table 3.2), which also tells a fixed-point depth
         * (whose compare reference is clamped) from a float one. What makes
         * one a depth attachment is the format recorded here (a render writes
         * depth through the ISP's own buffer, keyed by the texture). Until
         * 2026-09-26 the texels were not stored at all and read as 0. */
        GL_DEPTH_COMPONENT16 => (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, GLES_TEXFMT_R32F),
        GL_DEPTH_COMPONENT24 | GL_DEPTH_COMPONENT => (GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, GLES_TEXFMT_R32F),
        GL_DEPTH_COMPONENT32F => (GL_DEPTH_COMPONENT, GL_FLOAT, GLES_TEXFMT_R32F),
        GL_DEPTH24_STENCIL8 | GL_DEPTH_STENCIL => (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, GLES_TEXFMT_R32F),
        GL_DEPTH32F_STENCIL8 => (GL_DEPTH_STENCIL, GL_FLOAT_32_UNSIGNED_INT_24_8_REV, GLES_TEXFMT_R32F),
        /* the ES 3.0 formats of es3_formats: None here also when the back
         * end samples none of the format's storage classes
         * (`es3_storage_or_refuse` says so by name at the entry points) */
        _ => {
            let (_, e) = es3_formats::find(internalformat)?;
            (e.base, e.types[0], e.storage_here()?)
        }
    })
}

/// The ES 3.0 formats' share of an image specification call
/// (glTexImage2D/3D): the internal format of es3_formats takes only the
/// (format, type) pairs of table 3.2 for it (GL_INVALID_OPERATION) and a
/// back end that samples none of its storage classes refuses it by name; any
/// other internal format refuses the pairs only those formats take. False
/// when the call failed.
fn es3_image_check(c: &mut GlContext, internalformat: GLenum, format: GLenum, type_: GLenum, func: &str) -> bool {
    if let Some((_, e)) = es3_formats::find(internalformat) {
        if !e.accepts(format, type_) {
            c.set_error(GL_INVALID_OPERATION, func);
            return false;
        }
        return es3_storage_or_refuse(c, e, func).is_some();
    }
    /* RGB5_A1 takes the 10-bit word too (table 3.2) */
    let rgb5_a1_word = internalformat == GL_RGB5_A1 && is_rgb5_a1_word(format, type_);
    if es3_formats::only_for_new_formats(format, type_) && !rgb5_a1_word && sized_format(internalformat).is_some() {
        c.set_error(GL_INVALID_OPERATION, func);
        return false;
    }
    true
}

/// May the read framebuffer's colour buffer be copied into an image of this
/// format (ES 3.0 3.8.5, "Alternate Texture Image Specification Commands":
/// the component types must agree)? Colour buffers here are unsigned
/// normalised or floating point (the RGBA16F / R16F / R32F texture
/// attachments), never integer or signed normalised: an integer or snorm image
/// is never a copy target, a float image needs a float buffer, and
/// RGB10_A2 (unsigned normalised) a fixed-point one.
fn es3_copy_compatible(c: &GlContext, e: &es3_formats::Es3Format) -> bool {
    let float_buffer = matches!(
        framebuffer::fb_target_format(c),
        backend::GLES_SURF_RGBA16F | backend::GLES_SURF_R16F | backend::GLES_SURF_R32F
    );
    match e.class {
        es3_formats::Class::Int | es3_formats::Class::Uint | es3_formats::Class::Snorm => false,
        es3_formats::Class::Float => float_buffer,
        es3_formats::Class::Unorm => !float_buffer,
    }
}

/// The pair table 3.2 gives RGB5_A1 besides its own: the 10-bit word.
fn is_rgb5_a1_word(format: GLenum, type_: GLenum) -> bool {
    format == GL_RGBA && type_ == GL_UNSIGNED_INT_2_10_10_10_REV
}

/// The storage class of an ES 3.0 format on this back end, or the refusal
/// by name: a log line and GL_INVALID_OPERATION.
fn es3_storage_or_refuse(c: &mut GlContext, e: &es3_formats::Es3Format, func: &str) -> Option<u8> {
    let st = e.storage_here();
    if st.is_none() {
        glog!("{}: {} is not supported: this back end samples none of its storage classes", func, e.name);
        c.set_error(GL_INVALID_OPERATION, func);
    }
    st
}

/// Is `internalformat` GL_RGBA32F, the one sized name the original tables
/// store as another (`sized_format`: GL_RGBA32F is held as halves)? A
/// texture records it (`GlTexture::rgba32f`) so a framebuffer attachment can
/// report the sized format it was made with, which is what ES 3.0 4.4.4.1
/// judges. Never under `report-es2`, which keeps its answers.
fn is_rgba32f(internalformat: GLenum) -> bool {
    !crate::REPORT_ES2 && internalformat == GL_RGBA32F
}

/// Is `internalformat` an unsized depth format, GL_DEPTH_COMPONENT or
/// GL_DEPTH_STENCIL? ES 3.0 has them only through GL_OES_depth_texture and
/// GL_OES_packed_depth_stencil, which this build claims on the ES 2.0
/// surface alone; ES 3.0 4.4.4.2 makes only a sized depth format
/// depth-renderable. A texture records it (`GlTexture::unsized_depth`) so a
/// framebuffer attachment of it is incomplete (dEQP-GLES3
/// fbo.completeness.renderable.texture.depth.depth_component_unsigned_short
/// and three more). It still samples, as ES 3.0 core's GL_DEPTH_COMPONENT
/// does. Never under `report-es2`, which keeps its answers.
fn is_unsized_depth(internalformat: GLenum) -> bool {
    !crate::REPORT_ES2 && matches!(internalformat, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
}

/// `GlTexture::es3_fmt` of an internal format: its es3_formats index + 1,
/// 0 for any other.
fn es3_index(internalformat: GLenum) -> u8 {
    es3_formats::find(internalformat).map_or(0, |(i, _)| i)
}

/// A sub-image (format, type) against the texture that takes it (ES 3.0
/// 3.8.5): an ES 3.0 format takes the pairs of its table 3.2 row, any other
/// texture the pairs the original tables unpack. False (GL_INVALID_OPERATION
/// set) when it does not.
fn es3_sub_image_check(c: &mut GlContext, ti: usize, format: GLenum, type_: GLenum, func: &str) -> bool {
    if crate::REPORT_ES2 {
        return true;
    }
    let ok = match es3_formats::by_index(c.objs_ref().textures[ti].es3_fmt) {
        Some(e) => e.accepts(format, type_),
        /* (a texture does not record RGB5_A1 apart from RGBA8, so the word
         * is taken into either) */
        None => !es3_formats::only_for_new_formats(format, type_) || is_rgb5_a1_word(format, type_),
    };
    if !ok {
        c.set_error(GL_INVALID_OPERATION, func);
    }
    ok
}

/// The texel format a glTexImage2D/3D internal format is stored in.
fn texfmt_of(internalformat: GLint) -> u8 {
    let tf = sized_format(internalformat as GLenum).map_or(GLES_TEXFMT_RGBA8, |(_, _, tf)| tf);
    /* ES 2.0's luminance/alpha formats go narrow where the backend samples
     * them so: a quarter (L8, A8) or half (LA8) of the RGBA8 store */
    if tf == GLES_TEXFMT_RGBA8 && backend::narrow_la_texels() {
        match internalformat as GLenum {
            GL_LUMINANCE => return GLES_TEXFMT_L8,
            GL_ALPHA => return GLES_TEXFMT_A8,
            GL_LUMINANCE_ALPHA => return GLES_TEXFMT_LA8,
            _ => {}
        }
    }
    tf
}

/// Is `f` one of the narrow luminance/alpha texel formats?
fn is_narrow_la(f: u8) -> bool {
    matches!(f, GLES_TEXFMT_L8 | GLES_TEXFMT_A8 | GLES_TEXFMT_LA8)
}

/// Pack `n` RGBA8 texels (what `convert_row` makes) into `dst` as the
/// narrow format `f` stores them: L = R, A = A (GL's luminance is the red
/// channel of an RGBA source, ES 2.0 3.7.2 and the copy rules of 3.7.2's
/// table 3.9).
fn pack_narrow_la(dst: &mut [u8], rgba: &[u8], n: usize, f: u8) {
    for i in 0..n {
        let px = &rgba[i * 4..i * 4 + 4];
        match f {
            GLES_TEXFMT_L8 => dst[i] = px[0],
            GLES_TEXFMT_A8 => dst[i] = px[3],
            _ => {
                dst[i * 2] = px[0];
                dst[i * 2 + 1] = px[3];
            }
        }
    }
}

/// Immutable storage: the whole chain at once. The store is RGBA8 for every
/// sized format but the float ones (see convert_row); the format/type pair
/// recorded is what the sized format unpacks from, so later glTexSubImage2D
/// calls convert.
#[no_mangle]
pub unsafe extern "C" fn glTexStorage2D(
    target: GLenum,
    levels: GLsizei,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
) {
    const FUNC: &str = "glTexStorage2D";
    gl_trace!(glTexStorage2D);
    let Some(c) = context::current() else { return };

    if target != GL_TEXTURE_2D && target != GL_TEXTURE_CUBE_MAP {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if levels < 1 || width < 1 || height < 1 || width > MAX_TEXTURE_SIZE || height > MAX_TEXTURE_SIZE {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if let Some((_, e)) = es3_formats::find(internalformat) {
        if es3_storage_or_refuse(c, e, FUNC).is_none() {
            return;
        }
    }
    let Some((format, type_, texfmt)) = sized_format(internalformat) else {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    };
    /* ES 3.0 3.8.4: GL_INVALID_OPERATION when the default texture object
     * (name 0) is bound. It was made immutable instead until 2026-09-24,
     * and dEQP-GLES3's state reset after
     * negative_api.texture.texstorage2d_invalid_binding then failed on it
     * and ended the run. */
    if binding_for(c, target).is_some_and(|b| *b == 0) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* ES 3.0 3.8.4: more levels than the chain has (dEQP-GLES3
     * negative_api.texture.texstorage2d_invalid_levels) */
    if levels > i32::from(context::gl_mip_levels(width as u32, height as u32)) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if c.objs().textures[ti].immutable {
        c.set_error(GL_INVALID_OPERATION, FUNC); /* ES 3.0 3.8.4 */
        return;
    }
    note_target_use(c, ti);
    let t = &mut c.objs().textures[ti];
    let layers = if target == GL_TEXTURE_CUBE_MAP { 6 } else { 1 };
    /* a one-level depth texture's storage waits for a reader (defers_storage);
     * a NPOT texture gets its whole chain where the backend holds one */
    let rc = if levels == 1 && defers_storage(target, format, texfmt, width as u32, height as u32, core::ptr::null()) {
        texture_alloc_storage_deferred(t, width as u32, height as u32, texfmt)
    } else {
        texture_alloc_storage_ex(t, width as u32, height as u32, layers, texfmt, false, levels > 1)
    };
    if rc != 0 {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    t.format = format;
    t.type_ = type_;
    t.es3_fmt = es3_index(internalformat);
    t.rgba32f = is_rgba32f(internalformat);
    t.unsized_depth = is_unsized_depth(internalformat);
    t.immutable = true;
    t.immutable_levels = levels as u8;
    /* Immutable storage defines every face at once, so a cube map laid out
     * this way is complete before a single texel is uploaded (ES 3.0 3.8.4).
     * That is the difference from glTexImage2D, which specifies one face per
     * call and may leave the set incomplete. */
    if target == GL_TEXTURE_CUBE_MAP {
        t.faces_set = 0x3f;
    }
    /* every level exists (undefined texels); a chain the store cannot hold
     * (NPOT) keeps the base level only */
    t.levels = levels.min(t.chain_levels as GLsizei) as u8;
    /* every level the application asked for exists (the store keeps what
     * it can; a NPOT chain samples from its base) */
    t.level_defined = ((1u32 << (levels as u32).min(15)) - 1) as u16;
    t.level_bad = 0;
    t.level_faces = [0x3f; CAP_MIP_LEVELS];
}

#[no_mangle]
pub unsafe extern "C" fn glTexStorage3D(
    target: GLenum,
    levels: GLsizei,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
    depth: GLsizei,
) {
    const FUNC: &str = "glTexStorage3D";
    gl_trace!(glTexStorage3D);
    let Some(c) = context::current() else { return };

    if target != GL_TEXTURE_2D_ARRAY && target != GL_TEXTURE_3D {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if levels < 1 || width < 1 || height < 1 || !(1..=1024).contains(&depth) {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if let Some((_, e)) = es3_formats::find(internalformat) {
        if es3_storage_or_refuse(c, e, FUNC).is_none() {
            return;
        }
    }
    let Some((format, type_, texfmt)) = sized_format(internalformat) else {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    };
    /* ES 3.0 3.8.4, as in glTexStorage2D; a 3D chain halves the depth
     * too, an array's does not */
    if binding_for(c, target).is_some_and(|b| *b == 0) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let deepest = if target == GL_TEXTURE_3D { width.max(depth) } else { width };
    if levels > i32::from(context::gl_mip_levels(deepest as u32, height as u32)) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let Some(ti) = active_texture(c, target) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if c.objs().textures[ti].immutable {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    note_target_use(c, ti);
    let t = &mut c.objs().textures[ti];
    if target == GL_TEXTURE_3D {
        if texture_alloc_storage_3d(t, width as u32, height as u32, depth as u32, texfmt) != 0 {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        }
        t.format = format;
        t.type_ = type_;
        t.es3_fmt = es3_index(internalformat);
        t.rgba32f = is_rgba32f(internalformat);
        t.unsized_depth = is_unsized_depth(internalformat);
        /* every level exists (3.8.4) and the store holds each one's texels;
         * a grid's levels above 0 are not the backend's (`levels` 1, see
         * texture_alloc_storage_3d) */
        t.levels = if t.grid_cols != 0 { 1 } else { levels.min(t.chain_levels as GLsizei) as u8 };
        t.immutable = true;
        t.immutable_levels = levels as u8;
        t.level_defined = ((1u32 << (levels as u32).min(15)) - 1) as u16;
        t.level_bad = 0;
        return;
    }
    if texture_alloc_storage_ex(t, width as u32, height as u32, depth as u32, texfmt, false, levels > 1) != 0 {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    t.format = format;
    t.type_ = type_;
    t.es3_fmt = es3_index(internalformat);
    t.rgba32f = is_rgba32f(internalformat);
    t.unsized_depth = is_unsized_depth(internalformat);
    t.immutable = true;
    t.immutable_levels = levels as u8;
    t.levels = levels.min(t.chain_levels as GLsizei) as u8;
    /* every level the application asked for exists (the store keeps what
     * it can; a NPOT chain samples from its base) */
    t.level_defined = ((1u32 << (levels as u32).min(15)) - 1) as u16;
    t.level_bad = 0;
    t.level_faces = [0x3f; CAP_MIP_LEVELS];
}

/// Unit tests: nothing process-global is left here to re-arm (the ES 3.0
/// "not available" log went with the last entry point that used it);
/// testutil::TestGl still calls this when it is created and dropped.
#[cfg(test)]
pub(crate) fn reset_for_tests() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg_attr(feature = "report-es2", allow(unused_imports))]
    use crate::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
    use crate::gl::framebuffer::{
        glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers,
    };
    use crate::gl::state::glPixelStorei;
    use crate::testutil::{CacheOp, MemEvent, TestGl, HEAP_DATA};
    use std::vec;
    use std::vec::Vec;

    /* ---- helpers ------------------------------------------------------------ */

    fn ptr<T>(v: &[T]) -> *const c_void {
        v.as_ptr() as *const c_void
    }

    fn u16_bytes(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|p| p.to_ne_bytes()).collect()
    }

    fn f32_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|p| p.to_ne_bytes()).collect()
    }

    /// convert_row of `w` texels into a fresh buffer.
    fn conv(src: &[u8], w: u32, format: GLenum, type_: GLenum) -> Vec<u8> {
        let mut dst = vec![0xAAu8; w as usize * 4];
        convert_row(&mut dst, src, w, format, type_);
        dst
    }

    /// A new texture name bound to TEXTURE_2D of the active unit.
    fn new_tex() -> GLuint {
        let mut name: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D, name);
        }
        assert_ne!(name, 0);
        name
    }

    fn tex_image(level: GLint, w: GLsizei, h: GLsizei, format: GLenum, type_: GLenum, data: &[u8]) {
        let p = if data.is_empty() { core::ptr::null() } else { ptr(data) };
        // SAFETY: `data` holds the image the caller sized for (format, type, unpack state), or is null.
        unsafe { glTexImage2D(GL_TEXTURE_2D, level, format as GLint, w, h, 0, format, type_, p) };
    }

    fn tex_sub_image(level: GLint, x: GLint, y: GLint, w: GLsizei, h: GLsizei, format: GLenum, data: &[u8]) {
        // SAFETY: `data` holds the w x h UNSIGNED_BYTE image.
        unsafe { glTexSubImage2D(GL_TEXTURE_2D, level, x, y, w, h, format, GL_UNSIGNED_BYTE, ptr(data)) };
    }

    /// The whole storage of texture `name`.
    fn storage(gl: &TestGl, name: GLuint) -> Vec<u8> {
        let m = gl.ctx().objs().textures[name as usize].mem;
        assert!(!m.impl_.is_null(), "texture {name} has no storage");
        gl.gpu_bytes(m.gpu, m.size)
    }

    /// `n` RGBA texels of 4 distinct bytes each: texel i = [4i+1, 4i+2, 4i+3, 4i+4].
    fn ramp(n: usize) -> Vec<u8> {
        (0..n * 4).map(|i| (i + 1) as u8).collect()
    }

    /// Fill the (top-down, pair-pitched) default surface: storage texel
    /// (col, row) = [10*col+1, 10*row+2, 0x55, 0x80+col].
    fn fill_surface(gl: &TestGl, w: u32, h: u32) {
        let s = gl.ctx().default_surface;
        let pitch = tex_pitch(w) as usize;
        assert_eq!(s.size, pitch * h as usize * 4);
        /* rendered by the GPU: the CPU view is stale until invalidated */
        let mut px = gl.gpu_bytes(s.gpu, s.size);
        for row in 0..h as usize {
            for col in 0..w as usize {
                let o = (row * pitch + col) * 4;
                px[o..o + 4].copy_from_slice(&surf_texel(col as u32, row as u32));
            }
        }
        gl.gpu_render(s.gpu, &px);
    }

    /// `store` (rows of `pitch` texels) with the last texel of each row, the
    /// pad texel of an odd width that no upload writes. New storage is
    /// cleared, so it reads as zero.
    fn pad_texels_unwritten<const N: usize>(mut store: [u8; N], pitch: usize) -> [u8; N] {
        for row in store.chunks_exact_mut(pitch * 4) {
            row[(pitch - 1) * 4..].fill(0);
        }
        store
    }

    fn surf_texel(col: u32, row: u32) -> [u8; 4] {
        [(10 * col + 1) as u8, (10 * row + 2) as u8, 0x55, (0x80 + col) as u8]
    }

    /// Backing for an "EGLImage": a descriptor over memory the test owns,
    /// mapped for the GPU at 0x1234_0000.
    fn external_mem(backing: &mut Vec<u8>) -> OsalGpuMem {
        external_mem_at(backing, 0x1234_0000)
    }

    fn external_mem_at(backing: &mut Vec<u8>, gpu: u64) -> OsalGpuMem {
        let cpu = backing.as_mut_ptr() as *mut c_void;
        crate::testutil::mock_map_phys(gpu, backing.len(), cpu);
        OsalGpuMem { cpu, gpu, phys: 0x1234_0000, size: backing.len(), impl_: cpu }
    }

    /* ---- pure: half_to_float / unorm8 ---------------------------------------- */

    #[test]
    fn half_to_float_decodes_normal_values() {
        assert_eq!(half_to_float(0x3C00), 1.0);
        assert_eq!(half_to_float(0x3800), 0.5);
        assert_eq!(half_to_float(0xC000), -2.0);
        assert_eq!(half_to_float(0x7BFF), 65504.0); /* largest finite half */
        assert_eq!(half_to_float(0x0400), 6.103_515_6e-5); /* smallest normal: 2^-14 */
        assert_eq!(half_to_float(0x3555), 0.333_251_95); /* nearest half to 1/3 */
    }

    #[test]
    fn half_to_float_zero_keeps_its_sign() {
        assert_eq!(half_to_float(0x0000).to_bits(), 0.0f32.to_bits());
        assert_eq!(half_to_float(0x8000).to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn half_to_float_subnormals_flush_to_zero() {
        /* documented: a subnormal (< 2^-14, far below 1/255) is treated as 0 */
        assert_eq!(half_to_float(0x0001), 0.0);
        assert_eq!(half_to_float(0x03FF), 0.0);
        assert_eq!(half_to_float(0x83FF), 0.0);
    }

    #[test]
    fn half_to_float_infinity_and_nan() {
        assert_eq!(half_to_float(0x7C00), f32::INFINITY);
        assert_eq!(half_to_float(0xFC00), f32::NEG_INFINITY);
        assert!(half_to_float(0x7E00).is_nan());
        assert!(half_to_float(0xFC01).is_nan());
    }

    #[test]
    fn unorm8_clamps_to_0_1() {
        assert_eq!(unorm8(0.0), 0);
        assert_eq!(unorm8(-0.0), 0);
        assert_eq!(unorm8(-3.5), 0);
        assert_eq!(unorm8(1.0), 255);
        assert_eq!(unorm8(1.5), 255);
        assert_eq!(unorm8(f32::INFINITY), 255);
        assert_eq!(unorm8(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn ext_texture_rg_takes_its_unsized_names() {
        /* GL_EXT_texture_rg is advertised, so glTexImage2D has to take
         * GL_RED and GL_RG as internal formats, with GL_UNSIGNED_BYTE and
         * with GL_HALF_FLOAT_OES (dEQP fbo.completeness.renderable.texture.
         * *.{red,rg}_{unsigned_byte,half_float_oes}). They are stored RGBA8,
         * reading (r, 0, 0, 1) and (r, g, 0, 1). */
        const GL_HALF_FLOAT_OES: GLenum = 0x8D61;
        let gl = TestGl::new(8, 8);
        let mut tex: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            for (format, type_) in [
                (GL_RED, GL_UNSIGNED_BYTE),
                (GL_RG, GL_UNSIGNED_BYTE),
                (GL_RED, GL_HALF_FLOAT_OES),
                (GL_RG, GL_HALF_FLOAT_OES),
            ] {
                glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, 4, 4, 0, format, type_, core::ptr::null());
                assert_eq!(gl.error(), GL_NO_ERROR, "glTexImage2D(0x{format:x}, 0x{type_:x})");
            }
            /* and the "internalformat must equal format" rule still holds */
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RED as GLint, 4, 4, 0, GL_RG, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn a_red_texel_reads_r_zero_zero_one() {
        let gl = TestGl::new(8, 8);
        let mut tex: GLuint = 0;
        let px: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
        // SAFETY: room for one name; `px` holds 4 GL_RED texels.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RED as GLint, 4, 1, 0, GL_RED, GL_UNSIGNED_BYTE, px.as_ptr().cast());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let mem = gl.ctx().objs().textures[tex as usize].mem;
        let store = gl.gpu_bytes(mem.gpu, 4 * 4);
        assert_eq!(&store[..8], &[0x11, 0, 0, 255, 0x22, 0, 0, 255]);
    }

    #[test]
    fn unorm8_nan_is_zero() {
        assert_eq!(unorm8(f32::NAN), 0);
    }

    #[test]
    fn unorm8_rounds_to_nearest() {
        assert_eq!(unorm8(0.5), 128); /* 127.5 rounds up */
        assert_eq!(unorm8(0.0019), 0); /* 0.4845 */
        assert_eq!(unorm8(0.002), 1); /* 0.51 */
        assert_eq!(unorm8(0.998), 254); /* 254.49 */
        assert_eq!(unorm8(0.9985), 255); /* 254.6 */
    }

    #[test]
    fn unorm8_is_exact_for_every_k_over_255() {
        for k in 0..=255u32 {
            assert_eq!(unorm8(k as f32 / 255.0) as u32, k);
        }
    }

    /* ---- pure: source sizes -------------------------------------------------- */

    #[test]
    fn float_comps_per_format() {
        assert_eq!(float_comps(GL_RGBA), 4);
        assert_eq!(float_comps(GL_RGB), 3);
        assert_eq!(float_comps(GL_RG), 2);
        assert_eq!(float_comps(GL_LUMINANCE_ALPHA), 2);
        for f in [GL_RED, GL_LUMINANCE, GL_ALPHA] {
            assert_eq!(float_comps(f), 1);
        }
    }

    #[test]
    fn src_bpp_of_every_accepted_pair() {
        assert_eq!(src_bpp(GL_RGBA, GL_UNSIGNED_BYTE), 4);
        assert_eq!(src_bpp(GL_RGB, GL_UNSIGNED_BYTE), 3);
        assert_eq!(src_bpp(GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE), 2);
        assert_eq!(src_bpp(GL_RG, GL_UNSIGNED_BYTE), 2);
        assert_eq!(src_bpp(GL_LUMINANCE, GL_UNSIGNED_BYTE), 1);
        assert_eq!(src_bpp(GL_ALPHA, GL_UNSIGNED_BYTE), 1);
        assert_eq!(src_bpp(GL_RED, GL_UNSIGNED_BYTE), 1);
        /* the packed types are 2 bytes whatever the format */
        assert_eq!(src_bpp(GL_RGB, GL_UNSIGNED_SHORT_5_6_5), 2);
        assert_eq!(src_bpp(GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4), 2);
        assert_eq!(src_bpp(GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1), 2);
        assert_eq!(src_bpp(GL_RGBA, GL_FLOAT), 16);
        assert_eq!(src_bpp(GL_RGB, GL_FLOAT), 12);
        assert_eq!(src_bpp(GL_RED, GL_FLOAT), 4);
        assert_eq!(src_bpp(GL_RGBA, GL_HALF_FLOAT), 8);
        assert_eq!(src_bpp(GL_RG, GL_HALF_FLOAT), 4);
        assert_eq!(src_bpp(GL_LUMINANCE_ALPHA, GL_HALF_FLOAT), 4);
    }

    #[test]
    fn src_used_bytes_matches_what_a_row_carries() {
        assert_eq!(src_used_bytes(3, GL_RGBA, GL_UNSIGNED_BYTE), 12);
        assert_eq!(src_used_bytes(3, GL_RGB, GL_UNSIGNED_BYTE), 9);
        assert_eq!(src_used_bytes(3, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE), 6);
        assert_eq!(src_used_bytes(3, GL_RG, GL_UNSIGNED_BYTE), 6);
        assert_eq!(src_used_bytes(3, GL_ALPHA, GL_UNSIGNED_BYTE), 3);
        assert_eq!(src_used_bytes(3, GL_RGB, GL_UNSIGNED_SHORT_5_6_5), 6);
        assert_eq!(src_used_bytes(3, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4), 6);
        assert_eq!(src_used_bytes(3, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1), 6);
        assert_eq!(src_used_bytes(3, GL_RGB, GL_FLOAT), 36);
        assert_eq!(src_used_bytes(3, GL_RGBA, GL_HALF_FLOAT), 24);
        assert_eq!(src_used_bytes(0, GL_RGBA, GL_UNSIGNED_BYTE), 0);
    }

    #[test]
    fn src_used_bytes_reads_nothing_for_pairs_left_black() {
        assert_eq!(src_used_bytes(7, GL_DEPTH_COMPONENT, GL_UNSIGNED_BYTE), 0);
        assert_eq!(src_used_bytes(7, GL_RGBA, GL_UNSIGNED_INT), 0);
    }

    /* ---- pure: convert_row --------------------------------------------------- */

    #[test]
    fn convert_rgba_ubyte_is_a_copy() {
        let src = ramp(3);
        assert_eq!(conv(&src, 3, GL_RGBA, GL_UNSIGNED_BYTE), src);
    }

    #[test]
    fn convert_rgba_ubyte_touches_only_w_texels() {
        let mut dst = [0xAAu8; 12];
        convert_row(&mut dst, &ramp(2), 2, GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(dst, [1, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xAA, 0xAA, 0xAA]);
    }

    #[test]
    fn convert_rgb_ubyte_gets_opaque_alpha() {
        assert_eq!(conv(&[1, 2, 3, 4, 5, 6], 2, GL_RGB, GL_UNSIGNED_BYTE), [1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn convert_luminance_replicates_to_rgb() {
        assert_eq!(conv(&[7, 200], 2, GL_LUMINANCE, GL_UNSIGNED_BYTE), [7, 7, 7, 255, 200, 200, 200, 255]);
    }

    #[test]
    fn convert_luminance_alpha_ubyte() {
        assert_eq!(conv(&[7, 9, 200, 0], 2, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE), [7, 7, 7, 9, 200, 200, 200, 0]);
    }

    #[test]
    fn convert_alpha_has_black_rgb() {
        assert_eq!(conv(&[7, 255], 2, GL_ALPHA, GL_UNSIGNED_BYTE), [0, 0, 0, 7, 0, 0, 0, 255]);
    }

    #[test]
    fn convert_red_and_rg_fill_missing_channels_with_0_0_1() {
        assert_eq!(conv(&[7, 9], 2, GL_RED, GL_UNSIGNED_BYTE), [7, 0, 0, 255, 9, 0, 0, 255]);
        assert_eq!(conv(&[7, 9, 11, 13], 2, GL_RG, GL_UNSIGNED_BYTE), [7, 9, 0, 255, 11, 13, 0, 255]);
    }

    #[test]
    fn convert_565_replicates_high_bits() {
        /* r=16 -> 10000|100 = 132, g=32 -> 100000|10 = 130, b=16 -> 132 */
        let src = u16_bytes(&[0xFFFF, 0x0000, 0x8410, 0xF800, 0x07E0, 0x001F]);
        assert_eq!(
            conv(&src, 6, GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
            [
                255, 255, 255, 255, /**/ 0, 0, 0, 255, /**/ 132, 130, 132, 255, /**/ 255, 0, 0, 255,
                /**/ 0, 255, 0, 255, /**/ 0, 0, 255, 255
            ]
        );
    }

    #[test]
    fn convert_565_lowest_steps() {
        /* 1/31 -> 8 (00001|000 | 00001>>2), 1/63 -> 4 */
        let src = u16_bytes(&[0x0800, 0x0020, 0x0001]);
        assert_eq!(
            conv(&src, 3, GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
            [8, 0, 0, 255, /**/ 0, 4, 0, 255, /**/ 0, 0, 8, 255]
        );
    }

    #[test]
    fn convert_4444_scales_nibbles_by_17() {
        let src = u16_bytes(&[0x1234, 0xFFFF, 0x0000, 0xF00F]);
        assert_eq!(
            conv(&src, 4, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
            [17, 34, 51, 68, /**/ 255, 255, 255, 255, /**/ 0, 0, 0, 0, /**/ 255, 0, 0, 255]
        );
    }

    #[test]
    fn convert_5551_one_bit_alpha() {
        /* 0x8420|1: r=16,g=16,b=16,a=1 */
        let src = u16_bytes(&[0xFFFE, 0x0001, 0x8421, 0xF801, 0x07C0, 0x003E]);
        assert_eq!(
            conv(&src, 6, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1),
            [
                255, 255, 255, 0, /**/ 0, 0, 0, 255, /**/ 132, 132, 132, 255, /**/ 255, 0, 0, 255,
                /**/ 0, 255, 0, 0, /**/ 0, 0, 255, 0
            ]
        );
    }

    #[test]
    fn convert_float_rgba_clamps_and_rounds() {
        let src = f32_bytes(&[0.0, 0.5, 1.0, 2.0, -1.0, f32::NAN, 0.25, 1.0]);
        /* 0.25 * 255 = 63.75 -> 64 */
        assert_eq!(conv(&src, 2, GL_RGBA, GL_FLOAT), [0, 128, 255, 255, 0, 0, 64, 255]);
    }

    #[test]
    fn convert_float_rgb_gets_opaque_alpha() {
        let src = f32_bytes(&[1.0, 0.0, 0.5]);
        assert_eq!(conv(&src, 1, GL_RGB, GL_FLOAT), [255, 0, 128, 255]);
    }

    #[test]
    fn convert_float_red_and_rg() {
        assert_eq!(conv(&f32_bytes(&[0.5, 1.0]), 2, GL_RED, GL_FLOAT), [128, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(conv(&f32_bytes(&[0.5, 1.0]), 1, GL_RG, GL_FLOAT), [128, 255, 0, 255]);
    }

    #[test]
    fn convert_float_luminance_alpha_and_alpha() {
        assert_eq!(conv(&f32_bytes(&[0.5]), 1, GL_LUMINANCE, GL_FLOAT), [128, 128, 128, 255]);
        assert_eq!(conv(&f32_bytes(&[1.0, 0.5]), 1, GL_LUMINANCE_ALPHA, GL_FLOAT), [255, 255, 255, 128]);
        assert_eq!(conv(&f32_bytes(&[0.5]), 1, GL_ALPHA, GL_FLOAT), [0, 0, 0, 128]);
    }

    #[test]
    fn convert_half_float_rgba() {
        /* 1.0, 0.5, 0.0, 65504 (clamped) ; -2.0, +inf, NaN, 1.0 */
        let src = u16_bytes(&[0x3C00, 0x3800, 0x0000, 0x7BFF, 0xC000, 0x7C00, 0x7E00, 0x3C00]);
        assert_eq!(conv(&src, 2, GL_RGBA, GL_HALF_FLOAT), [255, 128, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn convert_half_float_single_channel_formats() {
        assert_eq!(conv(&u16_bytes(&[0x3800]), 1, GL_RED, GL_HALF_FLOAT), [128, 0, 0, 255]);
        assert_eq!(conv(&u16_bytes(&[0x3800]), 1, GL_LUMINANCE, GL_HALF_FLOAT), [128, 128, 128, 255]);
        assert_eq!(conv(&u16_bytes(&[0x3800]), 1, GL_ALPHA, GL_HALF_FLOAT), [0, 0, 0, 128]);
        assert_eq!(conv(&u16_bytes(&[0x3C00, 0x3800]), 1, GL_LUMINANCE_ALPHA, GL_HALF_FLOAT), [255, 255, 255, 128]);
    }

    #[test]
    fn convert_unhandled_pair_is_opaque_black() {
        assert_eq!(conv(&[], 2, GL_DEPTH_COMPONENT, GL_UNSIGNED_BYTE), [0, 0, 0, 255, 0, 0, 0, 255]);
        assert_eq!(conv(&[], 1, GL_RGBA, GL_UNSIGNED_INT), [0, 0, 0, 255]);
    }

    #[test]
    fn convert_zero_width_writes_nothing() {
        let mut dst = [0xAAu8; 4];
        convert_row(&mut dst, &[], 0, GL_RGB, GL_UNSIGNED_BYTE);
        convert_row(&mut dst, &[], 0, GL_RGBA, GL_FLOAT);
        convert_row(&mut dst, &[], 0, GL_RGB, GL_UNSIGNED_SHORT_5_6_5);
        assert_eq!(dst, [0xAA; 4]);
    }

    /* ---- pure: layout arithmetic --------------------------------------------- */

    #[test]
    fn is_pot_cases() {
        assert!(!is_pot(0));
        for v in [1u32, 2, 4, 1024, 4096, 0x8000_0000] {
            assert!(is_pot(v), "{v}");
        }
        for v in [3u32, 5, 6, 1000, 4095, 0xFFFF_FFFF] {
            assert!(!is_pot(v), "{v}");
        }
    }

    #[test]
    fn npot2_rounds_up_to_a_power_of_two() {
        assert_eq!(npot2_u32(0), 1);
        assert_eq!(npot2_u32(1), 1);
        assert_eq!(npot2_u32(2), 2);
        assert_eq!(npot2_u32(3), 4);
        assert_eq!(npot2_u32(5), 8);
        assert_eq!(npot2_u32(1024), 1024);
        assert_eq!(npot2_u32(1025), 2048);
    }

    #[test]
    fn level_size_halves_and_clamps_to_1() {
        // SAFETY: GlTexture is valid all-zero (see zero_texture).
        let mut t: GlTexture = unsafe { core::mem::zeroed() };
        t.width = 8;
        t.height = 2;
        assert_eq!(level_size(&t, 0), (8, 2));
        assert_eq!(level_size(&t, 1), (4, 1));
        assert_eq!(level_size(&t, 2), (2, 1));
        assert_eq!(level_size(&t, 3), (1, 1));
        assert_eq!(level_size(&t, 12), (1, 1));
    }

    #[test]
    fn is_texture_format_accepts_the_unsized_formats_only() {
        for f in [GL_RGBA, GL_RGB, GL_LUMINANCE, GL_ALPHA, GL_LUMINANCE_ALPHA, GL_RED, GL_RG] {
            assert!(is_texture_format(f));
        }
        for f in [0, GL_RGBA8, GL_DEPTH_COMPONENT, GL_COMPRESSED_RGB8_ETC2, GL_UNSIGNED_BYTE] {
            assert!(!is_texture_format(f));
        }
    }

    #[test]
    fn compressed_format_info_block_sizes() {
        assert_eq!(compressed_format_info(GL_ETC1_RGB8_OES), Some((GLES_TEXFMT_ETC2_RGB, 8)));
        assert_eq!(compressed_format_info(GL_COMPRESSED_RGB8_ETC2), Some((GLES_TEXFMT_ETC2_RGB, 8)));
        assert_eq!(compressed_format_info(GL_COMPRESSED_RGBA8_ETC2_EAC), Some((GLES_TEXFMT_ETC2_RGBA, 16)));
        assert_eq!(compressed_format_info(GL_COMPRESSED_SRGB8_ETC2), Some((GLES_TEXFMT_ETC2_SRGB_RGB, 8)));
        assert_eq!(compressed_format_info(GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC), Some((GLES_TEXFMT_ETC2_SRGB_RGBA, 16)));
        assert_eq!(compressed_format_info(GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2), None);
        assert_eq!(compressed_format_info(GL_RGBA), None);
        assert_eq!(compressed_format_info(0), None);
        /* the list of names is the table's, and every block size agrees with
         * the imageSize rule's */
        for f in COMPRESSED_FORMATS {
            let (_, bb) = compressed_format_info(f).expect("a listed format has a class");
            assert_eq!(compressed_block_bytes(f), Some(bb), "format {f:#x}");
        }
    }

    #[test]
    fn twiddle_square_grid_is_morton_with_y_in_the_low_bit() {
        /* 2x2: (0,0) (0,1) (1,0) (1,1) */
        assert_eq!(twiddle_index(0, 0, 2, 2), 0);
        assert_eq!(twiddle_index(0, 1, 2, 2), 1);
        assert_eq!(twiddle_index(1, 0, 2, 2), 2);
        assert_eq!(twiddle_index(1, 1, 2, 2), 3);
        /* 4x4: x = 0b10, y = 0b01 -> bits y0=1, x0=0, y1=0, x1=1 -> 0b1001 */
        assert_eq!(twiddle_index(2, 1, 4, 4), 0b1001);
        assert_eq!(twiddle_index(1, 2, 4, 4), 0b0110);
        assert_eq!(twiddle_index(3, 3, 4, 4), 15);
    }

    #[test]
    fn twiddle_is_a_bijection_onto_the_grid() {
        for (bw, bh) in [(1u32, 1u32), (4, 4), (8, 2), (2, 8), (16, 1), (1, 16)] {
            let mut seen = vec![false; (bw * bh) as usize];
            for by in 0..bh {
                for bx in 0..bw {
                    let i = twiddle_index(bx, by, bw, bh) as usize;
                    assert!(i < seen.len() && !seen[i], "{bw}x{bh} ({bx},{by}) -> {i}");
                    seen[i] = true;
                }
            }
        }
    }

    #[test]
    fn twiddle_wide_grid_puts_spare_x_bits_above_the_interleave() {
        /* 4x2: one interleaved bit pair, then x >> 1 from bit 2 */
        assert_eq!(twiddle_index(1, 1, 4, 2), 3);
        assert_eq!(twiddle_index(2, 0, 4, 2), 4);
        assert_eq!(twiddle_index(3, 1, 4, 2), 7);
        /* 4x1: nothing to interleave: linear in x */
        assert_eq!(twiddle_index(3, 0, 4, 1), 3);
    }

    #[test]
    fn twiddle_tall_grid_puts_spare_y_bits_above_the_interleave() {
        assert_eq!(twiddle_index(1, 0, 2, 4), 2);
        assert_eq!(twiddle_index(0, 2, 2, 4), 4);
        assert_eq!(twiddle_index(1, 3, 2, 4), 7);
        assert_eq!(twiddle_index(0, 3, 1, 4), 3);
    }

    /* ---- src_row_bytes (needs a context for the unpack state) ------------------ */

    #[test]
    fn src_row_bytes_rounds_up_to_the_unpack_alignment() {
        let gl = TestGl::new(4, 4);
        let c = gl.ctx();
        assert_eq!(c.unpack_alignment, 4, "GL default");
        assert_eq!(src_row_bytes(c, 3, GL_RGB, GL_UNSIGNED_BYTE), 12);
        assert_eq!(src_row_bytes(c, 4, GL_RGB, GL_UNSIGNED_BYTE), 12);
        assert_eq!(src_row_bytes(c, 3, GL_RGB, GL_UNSIGNED_SHORT_5_6_5), 8);
        assert_eq!(src_row_bytes(c, 5, GL_LUMINANCE, GL_UNSIGNED_BYTE), 8);
        c.unpack_alignment = 1;
        assert_eq!(src_row_bytes(c, 3, GL_RGB, GL_UNSIGNED_BYTE), 9);
        c.unpack_alignment = 2;
        assert_eq!(src_row_bytes(c, 3, GL_RGB, GL_UNSIGNED_BYTE), 10);
        c.unpack_alignment = 8;
        assert_eq!(src_row_bytes(c, 3, GL_RGBA, GL_UNSIGNED_BYTE), 16);
        assert_eq!(src_row_bytes(c, 0, GL_RGBA, GL_UNSIGNED_BYTE), 0);
    }

    #[test]
    fn src_row_bytes_uses_unpack_row_length_when_set() {
        let gl = TestGl::new(4, 4);
        let c = gl.ctx();
        c.unpack_row_length = 5;
        assert_eq!(src_row_bytes(c, 2, GL_RGB, GL_UNSIGNED_BYTE), 16); /* 15 -> 16 */
        assert_eq!(src_row_bytes(c, 2, GL_RGBA, GL_UNSIGNED_BYTE), 20);
        c.unpack_alignment = 1;
        assert_eq!(src_row_bytes(c, 2, GL_RGB, GL_UNSIGNED_BYTE), 15);
    }

    /* ---- names, binding ------------------------------------------------------- */

    #[test]
    fn gen_textures_returns_distinct_nonzero_names() {
        let gl = TestGl::new(4, 4);
        let mut names = [0u32; 5];
        // SAFETY: room for 5 names.
        unsafe { glGenTextures(5, names.as_mut_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        for (i, n) in names.iter().enumerate() {
            assert_ne!(*n, 0);
            assert!(!names[..i].contains(n));
        }
    }

    #[test]
    fn gen_textures_negative_count_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        let mut name = 77u32;
        // SAFETY: n < 0 writes nothing.
        unsafe { glGenTextures(-1, &mut name) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(name, 77);
    }

    #[test]
    fn gen_textures_zero_and_null_are_no_ops() {
        let gl = TestGl::new(4, 4);
        let mut name = 77u32;
        // SAFETY: n == 0 writes nothing; a null array is tolerated.
        unsafe {
            glGenTextures(0, &mut name);
            glGenTextures(3, core::ptr::null_mut());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(name, 77);
    }

    #[test]
    fn gen_textures_new_object_has_gl_defaults() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(t.min_filter, GL_NEAREST_MIPMAP_LINEAR);
        assert_eq!(t.mag_filter, GL_LINEAR);
        assert_eq!((t.wrap_s, t.wrap_t), (GL_REPEAT, GL_REPEAT));
        assert_eq!((t.width, t.height), (0, 0));
        assert!(t.mem.impl_.is_null());
    }

    #[test]
    fn gen_textures_skips_names_created_by_bind() {
        let gl = TestGl::new(4, 4);
        let mut names = [0u32; 3];
        // SAFETY: plain GL calls; room for 3 names.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, 2);
            glGenTextures(3, names.as_mut_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(!names.contains(&2));
        assert!(names.iter().all(|n| *n != 0));
    }

    #[test]
    fn gen_textures_table_exhaustion_returns_zero_names() {
        let gl = TestGl::new(4, 4);
        let mut names = vec![0xFFFF_FFFFu32; MAX_TEXTURES + 1];
        // SAFETY: room for every name asked for.
        unsafe { glGenTextures(names.len() as GLsizei, names.as_mut_ptr()) };
        /* name 0 is reserved: MAX_TEXTURES - 1 objects exist */
        assert!(names[..MAX_TEXTURES - 1].iter().all(|n| *n != 0));
        assert_eq!(names[MAX_TEXTURES - 1..], [0, 0]);
        /* an exhausted pool says so instead of handing back zeros silently
         * (upstream 7219228) */
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    #[test]
    fn active_texture_selects_the_unit() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL call.
        unsafe { glActiveTexture(GL_TEXTURE0 + CAP_COMBINED_TEXTURE_UNITS as u32 - 1) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().active_texture, CAP_COMBINED_TEXTURE_UNITS as u32 - 1);
    }

    #[test]
    fn active_texture_out_of_range_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe { glActiveTexture(GL_TEXTURE0 + 1) };
        unsafe { glActiveTexture(GL_TEXTURE0 + CAP_COMBINED_TEXTURE_UNITS as u32) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        unsafe { glActiveTexture(GL_TEXTURE0 - 1) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(gl.ctx().active_texture, 1, "the unit is unchanged");
    }

    #[test]
    fn bind_texture_is_per_unit() {
        let gl = TestGl::new(4, 4);
        let a = new_tex();
        // SAFETY: plain GL calls.
        unsafe { glActiveTexture(GL_TEXTURE0 + 3) };
        let b = new_tex();
        let c = gl.ctx();
        assert_eq!(c.bound_texture_2d[0], a);
        assert_eq!(c.bound_texture_2d[3], b);
        assert_eq!(c.bound_texture_2d[1], 0);
        assert_eq!(c.objs().textures[a as usize].target, GL_TEXTURE_2D);
    }

    #[test]
    fn bind_texture_creates_an_unknown_name() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe { glBindTexture(GL_TEXTURE_2D, 42) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(unsafe { glIsTexture(42) }, GL_TRUE_B);
        assert_eq!(gl.ctx().objs().textures[42].min_filter, GL_NEAREST_MIPMAP_LINEAR);
    }

    #[test]
    fn bind_texture_rebinding_keeps_the_object_state() {
        let gl = TestGl::new(4, 4);
        let a = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glBindTexture(GL_TEXTURE_2D, 0);
            glBindTexture(GL_TEXTURE_2D, a);
        }
        let t = &gl.ctx().objs_ref().textures[a as usize];
        assert_eq!(t.wrap_s, GL_CLAMP_TO_EDGE);
        assert_eq!((t.width, t.height), (2, 2));
        assert_eq!(&storage(&gl, a)[..16], &ramp(4)[..]);
    }

    /// A texture target this core does not have (GL_TEXTURE_2D_MULTISAMPLE, ES 3.1).
    const BAD_TARGET: GLenum = 0x9100;

    #[test]
    fn bind_texture_bad_target_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL call.
        unsafe { glBindTexture(BAD_TARGET, 1) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(gl.ctx().bound_texture_2d[0], 0);
        assert_eq!(unsafe { glIsTexture(1) }, GL_FALSE_B);
    }

    #[test]
    fn bind_texture_name_beyond_the_table_is_a_far_name() {
        /* ES 2.0 binds names nobody generated (gl/names.rs): the name
         * borrows the top slot, below the default textures */
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, MAX_TEXTURES as GLuint);
            assert_eq!(glIsTexture(MAX_TEXTURES as GLuint), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().bound_texture_2d[0], (MAX_TEXTURES - 1) as GLuint);
    }

    #[test]
    fn is_texture_false_for_zero_unknown_and_out_of_range() {
        let _gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsTexture(0), GL_FALSE_B);
            assert_eq!(glIsTexture(9), GL_FALSE_B);
            assert_eq!(glIsTexture(MAX_TEXTURES as GLuint), GL_FALSE_B);
            assert_eq!(glIsTexture(u32::MAX), GL_FALSE_B);
        }
    }

    #[test]
    fn is_texture_false_until_first_bind() {
        /* GL ES 2.0 glIsTexture: "A name returned by glGenTextures, but not
         * yet associated with a texture by calling glBindTexture, is not the
         * name of a texture." */
        let _gl = TestGl::new(4, 4);
        let mut name = 0u32;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenTextures(1, &mut name);
            assert_eq!(glIsTexture(name), GL_FALSE_B);
            glBindTexture(GL_TEXTURE_2D, name);
            assert_eq!(glIsTexture(name), GL_TRUE_B);
        }
    }

    /* ---- delete ---------------------------------------------------------------- */

    #[test]
    fn delete_frees_storage_and_the_name() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let gpu = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.mem_events();
        // SAFETY: one valid name.
        unsafe { glDeleteTextures(1, &name) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
        assert_eq!(unsafe { glIsTexture(name) }, GL_FALSE_B);
        assert!(!gl.ctx().objs().textures[name as usize].used);
    }

    #[test]
    fn delete_unbinds_from_every_unit() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls; one valid name.
        unsafe {
            glActiveTexture(GL_TEXTURE0 + 5);
            glBindTexture(GL_TEXTURE_2D, name);
            glActiveTexture(GL_TEXTURE0 + 2);
        }
        let other = new_tex();
        unsafe { glDeleteTextures(1, &name) };
        let c = gl.ctx();
        assert_eq!(c.bound_texture_2d[0], 0);
        assert_eq!(c.bound_texture_2d[5], 0);
        assert_eq!(c.bound_texture_2d[2], other, "another texture's binding is untouched");
    }

    #[test]
    fn delete_ignores_zero_unknown_and_out_of_range_names() {
        let gl = TestGl::new(4, 4);
        let keep = new_tex();
        let names = [0, 200, MAX_TEXTURES as GLuint, u32::MAX];
        // SAFETY: 4 names.
        unsafe { glDeleteTextures(4, names.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(unsafe { glIsTexture(keep) }, GL_TRUE_B);
    }

    #[test]
    fn delete_negative_count_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: n < 0 reads nothing.
        unsafe { glDeleteTextures(-1, &name) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(unsafe { glIsTexture(name) }, GL_TRUE_B);
    }

    #[test]
    fn delete_null_array_is_a_no_op() {
        let gl = TestGl::new(4, 4);
        // SAFETY: a null array is tolerated.
        unsafe { glDeleteTextures(2, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn delete_same_name_twice_in_one_call_frees_once() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        gl.mem_events();
        let names = [name, name];
        // SAFETY: 2 names.
        unsafe { glDeleteTextures(2, names.as_ptr()) };
        assert_eq!(gl.mem_events().len(), 1);
    }

    #[test]
    fn deleted_name_is_reused_with_fresh_state() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glDeleteTextures(1, &name);
        }
        let again = new_tex();
        assert_eq!(again, name, "the lowest free name is handed out");
        let t = &gl.ctx().objs_ref().textures[again as usize];
        assert_eq!(t.min_filter, GL_NEAREST_MIPMAP_LINEAR);
        assert_eq!((t.width, t.height), (0, 0));
        assert!(t.mem.impl_.is_null());
    }

    #[test]
    fn delete_detaches_from_the_framebuffer() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let mut fb = 0u32;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.name, name);
        unsafe { glDeleteTextures(1, &name) };
        let a = gl.ctx().framebuffers[fb as usize].color;
        assert_eq!((a.type_, a.name), (GL_NONE, 0));
    }

    #[test]
    fn delete_external_texture_does_not_free_the_image() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        gl.mem_events();
        // SAFETY: one valid name.
        unsafe { glDeleteTextures(1, &name) };
        assert_eq!(gl.mem_events(), []);
        assert_eq!(unsafe { glIsTexture(name) }, GL_FALSE_B);
    }

    /* ---- glTexImage2D: argument errors ------------------------------------------ */

    #[test]
    fn tex_image_bad_target_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        new_tex();
        // SAFETY: null pixels.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            )
        };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn tex_image_negative_or_too_deep_level_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        new_tex();
        tex_image(-1, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_image(CAP_MIP_LEVELS as GLint, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
    }

    #[test]
    fn tex_image_negative_size_or_border_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, -1, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_image(0, 4, -1, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        // SAFETY: null pixels.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null())
        };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null(), "nothing was allocated");
    }

    #[test]
    fn tex_image_without_a_binding_defines_the_default_texture_object() {
        /* ES 3.0 3.8: name 0 is not "no texture". Every target has a default
         * texture object, and glTexImage2D addresses it while nothing else
         * is bound; refusing made a conformance run's state reset fail. */
        let gl = TestGl::new(4, 4);
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = &gl.ctx().objs_ref().textures[MAX_TEXTURES];
        assert_eq!((d.used, d.target, d.width, d.height), (true, GL_TEXTURE_2D, 4, 4));
        assert!(!d.mem.impl_.is_null(), "the default object has storage of its own");
        /* the names are untouched: the default is not one of them */
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glIsTexture(0) }, GL_FALSE_B);
    }

    #[test]
    fn tex_image_unknown_format_or_type_is_invalid_enum() {
        /* an enum that is no pixel type at all is GL_INVALID_ENUM; a known
         * one in a combination table 3.2 does not list is
         * GL_INVALID_OPERATION (the same rule as glTexSubImage2D and
         * glReadPixels) */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 1, 1, GL_RGBA, 0x1234, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "no such type");
        tex_image(0, 1, 1, GL_DEPTH_STENCIL, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "DEPTH_STENCIL takes UNSIGNED_INT_24_8");
        tex_image(0, 1, 1, GL_DEPTH_COMPONENT, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "DEPTH_COMPONENT takes the integer and float types");
        assert_eq!(gl.ctx().objs().textures[name as usize].width, 0, "nothing was defined");
        /* the depth pairs the spec lists are accepted (the texels are
         * stored as floats: a_depth_upload_is_one_float_per_texel) */
        tex_image(0, 1, 1, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, &[1, 2]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        tex_image(0, 1, 1, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// A depth upload is stored as one float per texel in [0, 1] (R32F
    /// storage): an unsigned integer over its type's maximum, the top 24
    /// bits of a UNSIGNED_INT_24_8 texel, a float clamped to [0, 1]. A
    /// shadow look-up compares these values; until 2026-09-26 none were
    /// stored and every depth texel read 0.
    /// Revert check: with the depth arm of src_texel_float removed the
    /// middle values read 0 (the colour path's single byte).
    #[test]
    fn a_depth_upload_is_one_float_per_texel() {
        let gl = TestGl::new(4, 4);
        let f32s = |b: Vec<u8>, n: usize| -> Vec<f32> {
            (0..n).map(|i| f32::from_ne_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])).collect()
        };
        let u16s = |v: &[u16]| v.iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<u8>>();
        let u32s = |v: &[u32]| v.iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<u8>>();
        let fs = |v: &[f32]| v.iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<u8>>();
        /* (internal format, format, type, two texels, the floats stored) */
        type Case = (GLenum, GLenum, GLenum, Vec<u8>, [f32; 2]);
        let cases: [Case; 5] = [
            (
                GL_DEPTH_COMPONENT16,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                u16s(&[0x4000, 0xffff]),
                [16384.0 / 65535.0, 1.0],
            ),
            (GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, u32s(&[0x8000_0000, 0]), [0.5, 0.0]),
            (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT, fs(&[0.25, 7.0]), [0.25, 1.0]),
            (
                GL_DEPTH24_STENCIL8,
                GL_DEPTH_STENCIL,
                GL_UNSIGNED_INT_24_8,
                u32s(&[(0x40_0000 << 8) | 0x77, 0xffff_ff00]),
                [4194304.0 / 16777215.0, 1.0],
            ),
            (
                GL_DEPTH32F_STENCIL8,
                GL_DEPTH_STENCIL,
                GL_FLOAT_32_UNSIGNED_INT_24_8_REV,
                [fs(&[0.75]), u32s(&[0x55]), fs(&[-1.0]), u32s(&[0x55])].concat(),
                [0.75, 0.0],
            ),
        ];
        for (ifmt, fmt, ty, data, want) in cases {
            if crate::REPORT_ES2 && matches!(ifmt, GL_DEPTH_COMPONENT24 | GL_DEPTH_COMPONENT32F | GL_DEPTH32F_STENCIL8)
            {
                continue; /* ES 3.0's names (depth_image_error) */
            }
            let name = new_tex();
            // SAFETY: `data` holds a 2x1 image of (fmt, ty).
            unsafe { glTexImage2D(GL_TEXTURE_2D, 0, ifmt as GLint, 2, 1, 0, fmt, ty, ptr(&data)) };
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{ifmt:x}");
            assert_eq!(gl.ctx().objs().textures[name as usize].texfmt, GLES_TEXFMT_R32F, "0x{ifmt:x}");
            let got = f32s(storage(&gl, name), 2);
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-6, "0x{ifmt:x}: {got:?}, want {want:?}");
            }
        }
    }

    /// glTexSubImage3D takes a depth image into a 2D array (dEQP uploads
    /// its 2D array shadow textures so: 144 AM62P texture.shadow cases were
    /// GL_INVALID_ENUM) and refuses one into a 3D texture with
    /// GL_INVALID_OPERATION, as glTexImage3D does (ES 3.0.6 3.8.3).
    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn a_depth_sub_image_goes_into_a_2d_array_and_not_a_3d_texture() {
        let gl = TestGl::new(4, 4);
        let one = 0x8000u16.to_ne_bytes();
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL calls; `one` is one USHORT depth texel.
        unsafe {
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_DEPTH_COMPONENT16, 2, 2, 2);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 1, 0, 1, 1, 1, 1, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, ptr(&one));
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "a depth image into layer 1 of a 2D array");
        let b = storage(&gl, arr);
        let t = &gl.ctx().objs().textures[arr as usize];
        /* layer 1, texel (1, 0): after layer 0's rows, at the layer's pitch */
        let at = ((t.level_pitch(2) * 2) as usize + 1) * 4;
        let v = f32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        assert!((v - 32768.0 / 65535.0).abs() < 1e-6, "stored {v}");
        let _vol = bind_new(GL_TEXTURE_3D);
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 2, 2, 2);
            glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 1, 1, 1, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, ptr(&one));
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "no depth image in a 3D texture");
    }

    /// glTexSubImage2D takes a depth image on the ES 3.0 surface (stored
    /// as the float it names, like a full upload) and refuses one on the
    /// ES 2.0 surface, which claims no GL_OES_depth_texture.
    #[test]
    fn a_depth_sub_image_2d_is_es3s() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let zero = [0u8; 2 * 2 * 2];
        let one = 0xffffu16.to_ne_bytes();
        // SAFETY: plain GL calls; `zero` is a 2x2 USHORT image, `one` one texel.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT16 as GLint,
                2,
                2,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                ptr(&zero),
            );
            assert_eq!(gl.error(), GL_NO_ERROR);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 0, 1, 1, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, ptr(&one));
        }
        if crate::REPORT_ES2 && !super::super::state::DEPTH_TEXTURE_EXTENSIONS {
            assert_eq!(gl.error(), GL_INVALID_ENUM, "no depth sub-image without GL_OES_depth_texture");
            return;
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let b = storage(&gl, name);
        let v = f32::from_ne_bytes([b[4], b[5], b[6], b[7]]);
        assert_eq!(v, 1.0, "texel (1, 0) now holds 1.0");
    }

    /// glTexImage2D's depth rules (`depth_image_error`). Both surfaces: the
    /// internal format's base is the format, either way round. The ES 2.0
    /// surface: GL_OES_depth_texture and GL_OES_packed_depth_stencil's
    /// types, GL_TEXTURE_2D only, and no ES 3.0 sized depth names.
    #[test]
    fn a_depth_image_follows_the_depth_texture_rules() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let img = |target: GLenum, ifmt: GLenum, fmt: GLenum, ty: GLenum| {
            // SAFETY: null pixels only allocate.
            unsafe { glTexImage2D(target, 0, ifmt as GLint, 2, 2, 0, fmt, ty, core::ptr::null()) };
            gl.error()
        };
        let es2 = |e2: GLenum, e3: GLenum| if crate::REPORT_ES2 { e2 } else { e3 };
        /* the pairs every surface takes */
        for (ifmt, fmt, ty) in [
            (GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT),
            (GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
            (GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT),
            (GL_DEPTH_STENCIL, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8),
            (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8),
        ] {
            assert_eq!(img(GL_TEXTURE_2D, ifmt, fmt, ty), GL_NO_ERROR, "0x{ifmt:x} 0x{fmt:x} 0x{ty:x}");
        }
        /* the base and the format disagree */
        for (ifmt, fmt, ty) in [
            (GL_RGBA, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT),
            (GL_DEPTH_COMPONENT, GL_RGBA, GL_UNSIGNED_BYTE),
            (GL_DEPTH_STENCIL, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
            (GL_DEPTH_COMPONENT, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8),
            (GL_DEPTH24_STENCIL8, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
        ] {
            assert_eq!(img(GL_TEXTURE_2D, ifmt, fmt, ty), GL_INVALID_OPERATION, "0x{ifmt:x} 0x{fmt:x} 0x{ty:x}");
        }
        /* a float depth: ES 3.0 has it; the ES 2.0 surface has no GL_FLOAT
         * type at all (`es3_only_pixel_type`), so the type is refused first */
        assert_eq!(
            img(GL_TEXTURE_2D, GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_FLOAT),
            es2(GL_INVALID_ENUM, GL_NO_ERROR)
        );
        assert_eq!(
            img(GL_TEXTURE_2D, GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT),
            es2(GL_INVALID_ENUM, GL_NO_ERROR)
        );
        assert_eq!(
            img(GL_TEXTURE_2D, GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
            es2(GL_INVALID_VALUE, GL_NO_ERROR)
        );
        /* a depth cube face: ES 3.0's, not the extension's */
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        assert_eq!(
            img(GL_TEXTURE_CUBE_MAP_POSITIVE_X, GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
            es2(GL_INVALID_OPERATION, GL_NO_ERROR)
        );
        assert_ne!(cube, name);
        /* a level above 0: the extension does not forbid one */
        // SAFETY: plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_2D, name) };
        tex_image(0, 4, 4, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, &[]);
        tex_image(1, 2, 2, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, &u16_bytes(&[0xffff; 4]));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// glTexSubImage2D into a depth texture takes that texture's own depth
    /// format, with the extension's types on the ES 2.0 surface, and a colour
    /// texture takes no depth image.
    #[test]
    fn a_depth_sub_image_matches_the_textures_format() {
        let gl = TestGl::new(4, 4);
        let sub = |fmt: GLenum, ty: GLenum, data: &[u8]| {
            // SAFETY: `data` holds a 1x1 image of (fmt, ty).
            unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, fmt, ty, ptr(data)) };
            gl.error()
        };
        let four = [0u8; 8];
        let ds = new_tex();
        tex_image(0, 2, 2, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, &[]);
        if crate::REPORT_ES2 && !crate::gl::state::DEPTH_TEXTURE_EXTENSIONS {
            assert_eq!(sub(GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, &four), GL_INVALID_ENUM);
            return;
        }
        assert_eq!(sub(GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, &four), GL_NO_ERROR);
        assert_eq!(sub(GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, &four), GL_INVALID_OPERATION, "another depth format");
        assert_eq!(sub(GL_RGBA, GL_UNSIGNED_BYTE, &four), GL_INVALID_OPERATION, "a colour image");
        let d = new_tex();
        tex_image(0, 2, 2, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, &[]);
        assert_eq!(sub(GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, &four), GL_NO_ERROR, "either integer type");
        /* the ES 2.0 surface has no GL_FLOAT type (`es3_only_pixel_type`) */
        assert_eq!(
            sub(GL_DEPTH_COMPONENT, GL_FLOAT, &four),
            if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_NO_ERROR }
        );
        let colour = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(sub(GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, &four), GL_INVALID_OPERATION, "into a colour texture");
        assert!(ds != d && d != colour);
    }

    /// glGenerateMipmap of a depth texture is GL_INVALID_OPERATION
    /// (GL_OES_depth_texture; ES 3.0.6 3.8.9), not a silent no-op.
    #[test]
    fn generate_mipmap_of_a_depth_texture_is_invalid_operation() {
        let gl = TestGl::new(4, 4);
        for (fmt, ty) in [(GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT), (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8)] {
            new_tex();
            tex_image(0, 4, 4, fmt, ty, &[]);
            assert_eq!(gl.error(), GL_NO_ERROR);
            // SAFETY: plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "0x{fmt:x}");
        }
    }

    #[test]
    fn half_float_oes_is_unpacked_like_half_float() {
        /* GL_OES_texture_half_float's type (0x8D61) and ES 3.0's (0x140B) are
         * the same texels under two names. This test used to pin the
         * opposite -- "the converter has no path for it (known quirk, as in
         * C)", so an upload was accepted and read as bytes, and every texel
         * came out opaque black. The extension is advertised; taking the
         * type and then not unpacking it is not a quirk, it is wrong. */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* 1.0, 0.5, 0.0, 1.0 */
        tex_image(0, 1, 1, GL_RGBA, GL_HALF_FLOAT_OES, &u16_bytes(&[0x3C00, 0x3800, 0x0000, 0x3C00]));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(&storage(&gl, name)[..4], [255, 128, 0, 255]);
        /* and the ES 3.0 name gives the same bytes, where it is a type at
         * all: the ES 2.0 surface refuses it, and asserting the same bytes
         * there would pass on the ones already in the store */
        #[cfg(not(feature = "report-es2"))]
        {
            tex_image(0, 1, 1, GL_RGBA, GL_HALF_FLOAT, &u16_bytes(&[0x0000, 0x3C00, 0x3800, 0x3C00]));
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(&storage(&gl, name)[..4], [0, 255, 128, 255]);
        }
    }

    #[test]
    fn tex_image_format_type_pair_outside_table_3_4_is_invalid_operation() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 1, GL_RGBA, GL_UNSIGNED_INT, &[9; 8]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        tex_image(0, 2, 1, GL_RGB, GL_UNSIGNED_SHORT_4_4_4_4, &[9; 8]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        tex_image(0, 2, 1, GL_RGBA, GL_UNSIGNED_SHORT_5_6_5, &[9; 8]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        tex_image(0, 2, 1, 0x1234, GL_UNSIGNED_BYTE, &[9; 8]);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "unknown format");
        assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null(), "nothing defined");
    }

    #[test]
    fn tex_image_out_of_memory() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        gl.fail_alloc_in(1);
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null());
        assert_eq!(gl.live_allocs(), 1, "only the default surface");
        /* and it recovers */
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..64], ramp(16)[..]);
    }

    /* ---- glTexImage2D: uploads ---------------------------------------------------- */

    #[test]
    fn tex_image_records_size_format_and_type() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 8, 2, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, &[]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.format, t.type_), (8, 2, GL_RGB, GL_UNSIGNED_SHORT_5_6_5));
        assert_eq!(t.texfmt, GLES_TEXFMT_RGBA8);
        assert!(!t.external && !t.bgra);
    }

    #[test]
    fn tex_image_allocates_from_the_data_heap() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 6, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let gpu = gl.ctx().objs().textures[name as usize].mem.gpu;
        assert_eq!(gl.mem_events(), [MemEvent::Alloc { heap: HEAP_DATA, size: 6 * 4 * 4, gpu }]);
    }

    #[test]
    fn tex_image_rgba_ubyte_is_stored_verbatim() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(&storage(&gl, name)[..16], &ramp(4)[..]);
    }

    #[test]
    fn tex_image_rgb565_expands_to_rgba8() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 1, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, &u16_bytes(&[0xF800, 0x8410]));
        assert_eq!(&storage(&gl, name)[..8], [255, 0, 0, 255, 132, 130, 132, 255]);
    }

    #[test]
    fn tex_image_rgba4444_and_5551_expand_to_rgba8() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 1, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, &u16_bytes(&[0x1234, 0xF00F]));
        assert_eq!(&storage(&gl, name)[..8], [17, 34, 51, 68, 255, 0, 0, 255]);
        tex_image(0, 2, 1, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1, &u16_bytes(&[0xF801, 0x07C0]));
        assert_eq!(&storage(&gl, name)[..8], [255, 0, 0, 255, 0, 255, 0, 0]);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn tex_image_float_rgba_is_clamped_to_unorm8() {
        /* An unsized GL_FLOAT upload is GL_OES_texture_float or ES 3.0; the
         * ES 2.0 surface claims neither and refuses it (the test below). */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 1, 1, GL_RGBA, GL_FLOAT, &f32_bytes(&[0.0, 0.5, 1.0, 2.0]));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(&storage(&gl, name)[..4], [0, 128, 255, 255]);
    }

    #[test]
    #[cfg(feature = "report-es2")]
    fn the_es2_surface_refuses_an_unsized_float_upload() {
        /* GL_OES_texture_float is not advertised, and ES 3.0's own
         * GL_HALF_FLOAT (0x140B) is not a type this surface has -- the one it
         * has is GL_HALF_FLOAT_OES (0x8D61), from the extension it does
         * advertise. Taking the first two made a framebuffer built on such a
         * texture COMPLETE, where dEQP's fbo.completeness requires it not to
         * be (texture.color0.rgb_float, .rgba_float). */
        let gl = TestGl::new(4, 4);
        let _t = bind_new(GL_TEXTURE_2D);
        for (format, type_) in [(GL_RGBA, GL_FLOAT), (GL_RGB, GL_FLOAT), (GL_RGBA, GL_HALF_FLOAT)] {
            // SAFETY: null pixels only allocate.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, 2, 2, 0, format, type_, core::ptr::null());
            }
            /* not a type of this surface (`es3_only_pixel_type`); the
             * INVALID_OPERATION of the (unsized, float) pair it gave until
             * issue #128 is the same refusal under the ES 2.0 error */
            assert_eq!(gl.error(), GL_INVALID_ENUM, "0x{format:x} with type 0x{type_:x}");
        }
        /* the type the advertised extension does define is taken */
        // SAFETY: as above.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_HALF_FLOAT_OES, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* The V3D stores L8/A8/LA8 narrow and tiled for its TMU (board cases
     * 132 narrowla and 135 texnarrow PASS), so its storage bytes are not
     * these. The GC7000 stores them narrow and linear (`narrow_la_texels`),
     * the PowerVR test build expanded into RGBA8: both are checked. */
    #[cfg(not(feature = "gpu-v3d"))]
    #[test]
    fn tex_image_half_float_luminance_alpha() {
        /* GL_HALF_FLOAT_OES, not ES 3.0's GL_HALF_FLOAT: the conversion is
         * the same one and this type is valid on both surfaces (the ES 2.0
         * one advertises GL_OES_texture_half_float and not ES 3.0). */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 1, 1, GL_LUMINANCE_ALPHA, GL_HALF_FLOAT_OES, &u16_bytes(&[0x3C00, 0x3800]));
        assert_eq!(gl.error(), GL_NO_ERROR);
        if crate::backend::narrow_la_texels() {
            assert_eq!(&storage(&gl, name)[..2], [255, 128]);
        } else {
            assert_eq!(&storage(&gl, name)[..4], [255, 255, 255, 128]);
        }
    }

    /* Every GPU stores L8/A8/LA8 narrow, one or two bytes a texel; these are
     * the bytes of the RGBA8 store, the PowerVR's test-knobs arm
     * (pvr_debug_set_narrow_la(0), gl_test 132). */
    #[cfg(all(feature = "gpu-powervr", feature = "test-knobs"))]
    #[test]
    fn tex_image_alpha_and_luminance_rows() {
        let gl = TestGl::new(4, 4);
        crate::pvr::backend::pvr_debug_set_narrow_la(0);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
        tex_image(0, 2, 2, GL_ALPHA, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(&storage(&gl, name)[..16], [0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4]);
        tex_image(0, 2, 2, GL_LUMINANCE, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(&storage(&gl, name)[..16], [1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]);
    }

    /* L8/A8/LA8 stored narrow, as the PowerVR stores them */
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn narrow_luminance_alpha_uploads_are_packed_one_or_two_bytes_a_texel() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
        for (format, texfmt, data, want) in [
            (GL_LUMINANCE, GLES_TEXFMT_L8, &[1u8, 2, 3, 4][..], &[1u8, 2, 3, 4][..]),
            (GL_ALPHA, GLES_TEXFMT_A8, &[5, 6, 7, 8][..], &[5, 6, 7, 8][..]),
            (GL_LUMINANCE_ALPHA, GLES_TEXFMT_LA8, &[1, 9, 2, 8, 3, 7, 4, 6][..], &[1, 9, 2, 8, 3, 7, 4, 6][..]),
        ] {
            let name = new_tex();
            tex_image(0, 2, 2, format, GL_UNSIGNED_BYTE, data);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let t = &gl.ctx().objs_ref().textures[name as usize];
            assert_eq!((t.texfmt, t.bpp as u32), (texfmt, gles_texfmt_bytes(texfmt)), "0x{format:x}");
            assert_eq!(&storage(&gl, name)[..want.len()], want, "0x{format:x}");
        }
        /* a sub-image lands at its narrow offset: L8 2x2, texel (1,1) is byte 3 */
        let name = new_tex();
        tex_image(0, 2, 2, GL_LUMINANCE, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        tex_sub_image(0, 1, 1, 1, 1, GL_LUMINANCE, &[77]);
        assert_eq!(&storage(&gl, name)[..4], [1, 2, 3, 77]);
    }

    /* the GC7000 stores them narrow, packed as the PowerVR does */
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_gc7000_stores_luminance_alpha_uploads_narrow() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
        for (format, texfmt, data) in [
            (GL_LUMINANCE, GLES_TEXFMT_L8, &[1u8, 2, 3, 4][..]),
            (GL_ALPHA, GLES_TEXFMT_A8, &[5, 6, 7, 8][..]),
            (GL_LUMINANCE_ALPHA, GLES_TEXFMT_LA8, &[1, 9, 2, 8, 3, 7, 4, 6][..]),
        ] {
            let name = new_tex();
            tex_image(0, 2, 2, format, GL_UNSIGNED_BYTE, data);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(gl.ctx().objs_ref().textures[name as usize].texfmt, texfmt, "0x{format:x}");
            assert_eq!(&storage(&gl, name)[..data.len()], data, "0x{format:x}");
        }
    }

    /* the GC7000's test knob (gc_debug_narrow_la 0, gl_test 132's other arm):
     * the RGBA8 store again */
    #[cfg(all(feature = "gpu-vivante", feature = "test-knobs"))]
    #[test]
    fn the_gc7000_narrow_la_knob_selects_the_rgba8_store() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
        assert_eq!(crate::vivante::backend::gc_debug_set_narrow_la(0), 1, "narrow by default");
        let name = new_tex();
        tex_image(0, 2, 2, GL_LUMINANCE, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        crate::vivante::backend::gc_debug_set_narrow_la(1);
        assert_eq!(gl.ctx().objs_ref().textures[name as usize].texfmt, GLES_TEXFMT_RGBA8, "0: expanded");
        assert_eq!(&storage(&gl, name)[..16], [1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]);
    }

    /* the PowerVR's RGBA8 arm, a test knob (pvr_debug_set_narrow_la(0),
     * gl_test 132) */
    #[cfg(all(feature = "gpu-powervr", feature = "test-knobs"))]
    #[test]
    fn narrow_off_keeps_luminance_in_rgba8() {
        let gl = TestGl::new(4, 4);
        crate::pvr::backend::pvr_debug_set_narrow_la(0);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
        tex_image(0, 2, 2, GL_LUMINANCE, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(gl.ctx().objs_ref().textures[name as usize].texfmt, GLES_TEXFMT_RGBA8);
        assert_eq!(&storage(&gl, name)[..8], [1, 1, 1, 255, 2, 2, 2, 255]);
    }

    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn generate_mipmap_of_a_narrow_la8_texture_averages_each_channel() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* 2x2 LA8: L {10, 20, 30, 41}, A {200, 100, 0, 101} */
        tex_image(0, 2, 2, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE, &[10, 200, 20, 100, 30, 0, 41, 101]);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let l1 = t.level_off[1] as usize;
        /* L (10+20+30+41)/4 = 25.25 -> 25, A (200+100+0+101)/4 = 100.25 -> 100 */
        assert_eq!(&storage(&gl, name)[l1..l1 + 2], [25, 100]);
    }

    #[test]
    fn tex_image_null_pixels_allocates_without_writing() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(!t.mem.impl_.is_null());
        assert!(t.mem.size >= 64);
    }

    #[test]
    fn tex_image_zero_size_is_legal_and_allocates_nothing() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 0, 0, GL_RGBA, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.mem_events(), []);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.chain_levels, t.levels), (0, 0, 1, 1));
    }

    #[test]
    fn tex_image_default_unpack_alignment_pads_rgb_rows_to_4() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* 3 RGB texels = 9 bytes, the next row starts at byte 12 */
        let mut src = vec![0xEEu8; 24];
        src[..9].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        src[12..21].copy_from_slice(&[11, 12, 13, 14, 15, 16, 17, 18, 19]);
        tex_image(0, 3, 2, GL_RGB, GL_UNSIGNED_BYTE, &src);
        let s = storage(&gl, name);
        /* odd width: rows sit at the even pitch of 4 texels */
        assert_eq!(&s[..12], [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255]);
        assert_eq!(&s[16..28], [11, 12, 13, 255, 14, 15, 16, 255, 17, 18, 19, 255]);
    }

    #[test]
    fn tex_image_unpack_alignment_1_reads_tight_rows() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
        let src: Vec<u8> = (1..=18).collect();
        tex_image(0, 3, 2, GL_RGB, GL_UNSIGNED_BYTE, &src);
        let s = storage(&gl, name);
        assert_eq!(&s[..12], [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255]);
        assert_eq!(&s[16..28], [10, 11, 12, 255, 13, 14, 15, 255, 16, 17, 18, 255]);
    }

    #[test]
    fn tex_image_unpack_alignment_8() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 8) };
        /* 3 RGBA texels = 12 bytes, rows 16 bytes apart */
        let mut src = vec![0xEEu8; 32];
        src[..12].copy_from_slice(&ramp(3));
        src[16..28].copy_from_slice(&[101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112]);
        tex_image(0, 3, 2, GL_RGBA, GL_UNSIGNED_BYTE, &src);
        let s = storage(&gl, name);
        assert_eq!(s[..12], ramp(3)[..]);
        assert_eq!(s[16..28], src[16..28]);
    }

    #[test]
    fn tex_image_odd_width_uses_even_row_pitch_and_leaves_the_pad_texel() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(15));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(t.mem.size, 6 * 3 * 4, "tex_pitch(5) = 6 texels a row");
        let s = storage(&gl, name);
        let src = ramp(15);
        for row in 0..3 {
            assert_eq!(s[row * 24..row * 24 + 20], src[row * 20..row * 20 + 20], "row {row}");
            assert_eq!(s[row * 24 + 20..row * 24 + 24], [0; 4], "pad texel of row {row} (cleared, never written)");
        }
    }

    #[test]
    fn tex_image_width_1_rows_are_a_texel_pair_apart() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 1, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(3));
        let s = storage(&gl, name);
        assert_eq!(&s[0..4], [1, 2, 3, 4]);
        assert_eq!(&s[8..12], [5, 6, 7, 8]);
        assert_eq!(&s[16..20], [9, 10, 11, 12]);
    }

    /* ---- mip chain layout ------------------------------------------------------------ */

    #[test]
    fn pot_texture_lays_out_its_whole_chain() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 8, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        /* 8x4 (128 bytes), 4x2 (32), 2x1 (8), 1x1 */
        assert_eq!(t.chain_levels, 4);
        assert_eq!(t.levels, 1, "only the base level has texels");
        assert_eq!(t.level_off[..4], [0, 128, 160, 168]);
        /* the 1x1 level above a wider base is one texel: the TPU walks a
         * 1-wide level at its own width, not at the pair pitch */
        assert_eq!(t.mem.size, 168 + 4);
    }

    #[test]
    fn pot_1x1_texture_has_a_single_level() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.chain_levels, t.levels, t.level_off[0]), (1, 1, 0));
    }

    #[test]
    fn pot_4096_wide_chain_has_13_levels() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4096, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(t.chain_levels as usize, CAP_MIP_LEVELS);
        /* 4 * (4096 + 2048 + ... + 2) bytes precede the 1x1 level */
        assert_eq!(t.level_off[12], 4 * (8192 - 2));
        assert_eq!(t.level_off[1], 4 * 4096);
    }

    #[test]
    fn a_level_past_max_texture_size_is_invalid_value() {
        /* GL_MAX_TEXTURE_SIZE is 4096 (glGetIntegerv), and 4096 is the
         * largest chain CAP_MIP_LEVELS holds */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 8192, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(gl.ctx().objs().textures[name as usize].width, 0);
        assert_eq!(gl.live_allocs(), 1, "nothing was allocated");
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 1, 8192) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(gl.live_allocs(), 1);
    }

    #[test]
    fn npot_texture_has_no_chain() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 6, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.chain_levels, t.levels), (1, 1));
        assert_eq!(t.mem.size, 6 * 4 * 4);
        /* one NPOT dimension is enough */
        tex_image(0, 8, 3, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.ctx().objs().textures[name as usize].chain_levels, 1);
    }

    #[test]
    fn tex_image_level_1_lands_behind_level_0() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let l1: Vec<u8> = (101..=116).collect();
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &l1);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[64..80], l1[..]);
        assert_eq!(gl.ctx().objs().textures[name as usize].levels, 2);
        tex_image(2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[201, 202, 203, 204]);
        assert_eq!(storage(&gl, name)[80..84], [201, 202, 203, 204]);
        assert_eq!(gl.ctx().objs().textures[name as usize].levels, 3);
    }

    #[test]
    fn tex_image_level_upload_does_not_reallocate() {
        let gl = TestGl::new(4, 4);
        new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        gl.mem_events();
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        assert_eq!(gl.mem_events(), []);
    }

    /// `backend::texture_uploaded` is called after the GL core's own store,
    /// once per upload, only when the caller's rows ARE the stored texels
    /// (GL_RGBA / GL_UNSIGNED_BYTE into RGBA8), with the caller's pointer, the
    /// box and the row pitch after GL_UNPACK_ROW_LENGTH and
    /// GL_UNPACK_ALIGNMENT (`src_row_bytes`); never for a
    /// conversion (LUMINANCE, BGRA_EXT), a NULL upload or a cube face.
    #[test]
    fn the_upload_hook_sees_the_callers_rgba_rows_and_nothing_else() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        let px = ramp(16);
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &px);
        let up = gl.uploads();
        assert_eq!(up.len(), 1);
        assert_eq!((up[0].level, up[0].rect, up[0].stride), (0, [0, 0, 4, 4], 16));
        assert_eq!(up[0].src, px.as_ptr() as usize, "the caller's own rows, not the storage");
        assert_eq!(up[0].first_row, px[..16]);
        assert_eq!(up[0].b.content_gen, gl.ctx().objs_ref().textures[name as usize].content_gen);
        let gen0 = up[0].b.content_gen;
        /* a sub-box, 3 texels a row at alignment 8: 16 bytes, not 12 */
        let sub = ramp(8);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 8) };
        tex_sub_image(0, 1, 2, 3, 2, GL_RGBA, &sub);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 4) };
        let up = gl.uploads();
        assert_eq!((up.len(), up[0].rect, up[0].stride), (1, [1, 2, 3, 2], 16));
        /* the key names the texels AFTER this write: a copy filed under the
         * old stamp would be found by no draw, one filed under the new stamp
         * with stale texels would be drawn */
        let gen1 = gl.ctx().objs_ref().textures[name as usize].content_gen;
        assert_eq!(up[0].b.content_gen, gen1);
        assert_ne!(gen1, gen0);
        /* not the stored bytes: no call */
        tex_image(0, 4, 4, GL_LUMINANCE, GL_UNSIGNED_BYTE, &ramp(4)[..16]);
        tex_image(0, 4, 4, GL_BGRA_EXT, GL_UNSIGNED_BYTE, &px);
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, &px[..32]);
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.uploads().is_empty(), "a conversion or a NULL upload is not the stored texels");
        /* an XRGB8888 image stores B,G,R,X: RGBA rows are swapped on the way */
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, GLES_SURF_XRGB8888), 0);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.uploads().is_empty(), "an XRGB image's storage is not the caller's bytes");
        /* an RGBA8888 image: the bytes are the caller's, the stamp is 0 (an
         * EGLImage's texels are never cached), and the backend is told both */
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        let up = gl.uploads();
        assert_eq!((up.len(), up[0].b.content_gen), (1, 0));
        /* a cube face: RGBA / UNSIGNED_BYTE, but not a 2D level */
        let mut cube: GLuint = 0;
        // SAFETY: valid out pointer; `px` holds a 4x4 RGBA image.
        unsafe {
            glGenTextures(1, &mut cube);
            glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_POSITIVE_X,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr(&px),
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.uploads().is_empty(), "a cube face is not a 2D texture");
        // SAFETY: valid names.
        unsafe {
            glDeleteTextures(1, &name);
            glDeleteTextures(1, &cube);
        }
    }

    /// The texel write stamp (`GlTexture::content_gen`, passed to the
    /// backend as `GlesTextureBind::content_gen`): a new, non-zero value at
    /// every CPU write entry point, so a backend keeping a copy of the texels
    /// keyed on it makes the copy again; 0 for storage an EGLImage shares,
    /// which no backend may keep a copy of.
    #[test]
    fn the_texel_stamp_moves_at_every_cpu_write_and_is_zero_for_an_eglimage() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        let gen = || gl.ctx().objs_ref().textures[name as usize].content_gen;
        let mut seen = std::vec::Vec::new();
        let mut step = |what: &str, g: u32| {
            assert_ne!(g, 0, "{what}: a written texture is stamped");
            assert!(!seen.contains(&g), "{what}: a new stamp ({g}), not one of {seen:?}");
            seen.push(g);
        };
        assert_eq!(gen(), 0, "no texels yet");
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        step("a fresh glTexImage2D", gen());
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        step("glTexImage2D over kept storage", gen());
        tex_sub_image(0, 1, 1, 2, 2, GL_RGBA, &ramp(4));
        step("glTexSubImage2D", gen());
        // SAFETY: plain GL calls on the bound texture.
        unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 2, 2) };
        step("glCopyTexSubImage2D", gen());
        // SAFETY: as above.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        step("glGenerateMipmap", gen());
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        step("glCompressedTexImage2D", gen());
        let b = blocks(1, 8);
        // SAFETY: as above, 8 bytes for one block.
        unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b)) };
        step("glCompressedTexSubImage2D", gen());
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        assert_eq!(gen(), 0, "an EGLImage's storage has no stamp");
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gen(), 0, "a write into an EGLImage's storage leaves it unstamped");
        // SAFETY: one valid name, freed while the context is live.
        unsafe { glDeleteTextures(1, &name) };
    }

    #[test]
    fn tex_image_level_keeps_the_base_format() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_image(1, 1, 1, GL_LUMINANCE, GL_UNSIGNED_BYTE, &[7]);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.format, t.width, t.height), (GL_RGBA, 2, 2));
        assert_eq!(storage(&gl, name)[16..20], [7, 7, 7, 255]);
    }

    /// A level whose size is not the chain's for that level is no error in
    /// ES 2.0 (3.7.1 has none for it) or ES 3.0: it is recorded, gets no
    /// storage, and the texture is incomplete under a mipmap filter
    /// (`draw::incomplete_reason`). This pinned GL_INVALID_VALUE until
    /// 2026-09-24, which failed dEQP texture.completeness's
    /// not_positive_level_0 on the error alone.
    #[test]
    fn tex_image_level_of_the_wrong_size_is_recorded_not_an_error() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_image(1, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        assert_eq!(gl.error(), GL_NO_ERROR);
        tex_image(1, 2, 1, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(2));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        assert_eq!((t.levels, t.level_defined, t.level_bad), (1, 0b11, 0b10), "level 1 recorded as not fitting");
        /* the right size takes its place */
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        let t = &gl.ctx().objs().textures[name as usize];
        assert_eq!((t.levels, t.level_defined, t.level_bad), (2, 0b11, 0));
    }

    /// A 1x1 image past the chain's last level is ignored (ES 2.0 3.7.10
    /// ignores levels past q), with no error. Pinned GL_INVALID_VALUE until
    /// 2026-09-24 (dEQP texture.completeness.2d.extra_level).
    #[test]
    fn tex_image_level_beyond_the_chain_is_ignored() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_image(3, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(1));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        assert_eq!((t.levels, t.level_defined, t.level_bad), (1, 0b1, 0), "not part of the chain");
    }

    /// glTexImage3D's limits and pairings (ES 3.0 3.8.3; dEQP-GLES3
    /// negative_api.texture.teximage3d, _max_level,
    /// _max_width_height_depth): a 3D image has 9 levels of up to 256 a
    /// side, an array up to 4096 a side and 256 layers, an unsized internal
    /// format names `format`'s base, and a depth format has no 3D image.
    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_image_3d_limits_and_pairings() {
        let gl = TestGl::new(4, 4);
        let mut n: GLuint = 0;
        let null = core::ptr::null();
        // SAFETY: plain GL calls; `n` is one name; no pixels are read.
        unsafe {
            glGenTextures(1, &mut n);
            glBindTexture(GL_TEXTURE_3D, n);
            glTexImage3D(GL_TEXTURE_3D, 9, GL_RGB as GLint, 1, 1, 1, 0, GL_RGB, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "level 9 of a 3D image");
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA as GLint, 257, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a 3D side past 256");
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGB as GLint, 1, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "RGB internal format with RGBA data");
            glTexImage3D(GL_TEXTURE_3D, 0, 0, 1, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "no such internal format");
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_DEPTH_COMPONENT as GLint,
                1,
                1,
                1,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                null,
            );
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no 3D depth image");
            let mut arr: GLuint = 0;
            glGenTextures(1, &mut arr);
            glBindTexture(GL_TEXTURE_2D_ARRAY, arr);
            glTexImage3D(GL_TEXTURE_2D_ARRAY, 13, GL_RGB as GLint, 1, 1, 1, 0, GL_RGB, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA as GLint, 1, 1, 257, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "257 layers");
            glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA as GLint, 4097, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a side past 4096");
        }
    }

    /// The compressed 3D entry points check their arguments before saying
    /// the storage does not exist (ES 3.0 3.8.6; dEQP-GLES3
    /// negative_api.texture.compressedtex*image3d*), and
    /// glCompressedTexSubImage2D takes a cube face and refuses a level past
    /// any chain with GL_INVALID_VALUE (compressedtexsubimage2d_max_level).
    /// Every one was GL_INVALID_OPERATION or GL_INVALID_ENUM until 2026-09-24.
    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn compressed_3d_and_cube_sub_image_arguments_come_first() {
        let gl = TestGl::new(4, 4);
        let etc2 = GL_COMPRESSED_RGBA8_ETC2_EAC;
        let null = core::ptr::null();
        // SAFETY: plain GL calls; no data is read.
        unsafe {
            glCompressedTexImage3D(0, 0, etc2, 0, 0, 0, 0, 0, null);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "the target");
            glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA8, 0, 0, 0, 0, 0, null);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a compressed format");
            glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, -1, etc2, 0, 0, 0, 0, 0, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "the level");
            glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, 0, etc2, 0, 0, 0, 1, 0, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "the border");
            glCompressedTexImage3D(GL_TEXTURE_3D, 0, etc2, 4, 4, 1, 0, 16, null);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "ETC2 has no 3D form");
            glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, 0, etc2, 4, 4, 1, 0, 16, null);
            assert_eq!(gl.error(), GL_NO_ERROR, "a compressed 2D array (GL_INVALID_OPERATION until 2026-09-24)");
            glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, -4, 0, 0, 4, 4, 1, etc2, 16, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "the offset");
            glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 0, 4, 4, 1, etc2, 16, null);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glCompressedTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 1, etc2, 16, null);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no ETC2 3D texture to write");
            let mut cube: GLuint = 0;
            glGenTextures(1, &mut cube);
            glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
            glCompressedTexSubImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X, 13, 0, 0, 0, 0, etc2, 0, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "level 13 on a face");
            glCompressedTexSubImage2D(GL_TEXTURE_CUBE_MAP_NEGATIVE_Z, -1, 0, 0, 0, 0, etc2, 0, null);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCompressedTexSubImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X, 0, 0, 0, 0, 0, etc2, 0, null);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no compressed storage on the face");
        }
    }

    /// glCopyTexImage2D takes ES 3.0's sized names (3.8.5), and a 3D image's
    /// level cap holds for glCopyTexSubImage3D (dEQP-GLES3 negative_api.
    /// texture.copyteximage2d_incomplete_framebuffer, copytexsubimage3d_max_level).
    #[test]
    fn copy_tex_image_takes_sized_formats_and_the_3d_level_cap_holds() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls; the default framebuffer is complete.
        unsafe {
            glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 0, 0, 2, 2, 0);
            assert_eq!(gl.error(), GL_NO_ERROR, "a sized internal format");
            let t = &gl.ctx().objs_ref().textures[name as usize];
            assert_eq!((t.width, t.height), (2, 2));
        }
        /* a 3D texture is ES 3.0's: the ES 2.0 surface cannot bind one */
        #[cfg(not(feature = "report-es2"))]
        // SAFETY: plain GL calls; no pixels are read.
        unsafe {
            let mut n: GLuint = 0;
            glGenTextures(1, &mut n);
            glBindTexture(GL_TEXTURE_3D, n);
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA as GLint, 4, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glCopyTexSubImage3D(GL_TEXTURE_3D, 9, 0, 0, 0, 0, 0, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "level 9 of a 3D image");
        }
    }

    /// ES 2.0 3.7.1 without GL_OES_texture_npot: a level above 0 that is not
    /// a power of two is GL_INVALID_VALUE (NPOT_RESTRICTED; dEQP
    /// texture.completeness's npot_size expects GL_NO_ERROR, since the
    /// extension is listed). With the extension, as in ES
    /// 3.0, the level is accepted: on this PowerVR-shaped mock it lays the
    /// chain out as the 8x8 one (`pot_chain_layout`) and its texels land at
    /// level 1's power-of-two pitch; one of another size than GL's chain is
    /// recorded as not fitting. Pinned GL_INVALID_VALUE for both versions
    /// until 2026-09-24; recorded and not held on the PowerVR until
    /// 2026-10-08.
    #[test]
    fn tex_image_level_on_npot_texture_follows_the_version() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 6, 6, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_image(1, 3, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(9));
        if NPOT_RESTRICTED {
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        } else {
            assert_eq!(gl.error(), GL_NO_ERROR);
            let t = &gl.ctx().objs().textures[name as usize];
            assert_eq!((t.level_defined, t.level_bad, t.chain_levels), (0b11, 0, 3), "GL's level 1, held");
            assert!(t.pot_chain);
            assert_eq!((t.level_off[1], t.level_pitch_at(1)), (8 * 8 * 4, 4), "level 1 of the 8x8 chain");
            let s = storage(&gl, name);
            let l1 = t.level_off[1] as usize;
            for y in 0..3 {
                assert_eq!(s[l1 + y * 16..l1 + y * 16 + 12], ramp(9)[y * 12..y * 12 + 12], "level 1 row {y}");
            }
            tex_image(2, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
            assert_eq!(gl.error(), GL_NO_ERROR);
            let t = &gl.ctx().objs().textures[name as usize];
            assert_eq!((t.level_defined, t.level_bad), (0b111, 0b100), "2x2 is not level 2 of 6x6 (1x1)");
        }
    }

    #[test]
    fn respecifying_level_0_resets_the_level_count() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.levels, t.chain_levels), (1, 2));
        assert_eq!(t.level_off[..2], [0, 16]);
    }

    /* ---- storage reallocation ------------------------------------------------------------ */

    #[test]
    fn respecifying_same_or_smaller_size_reuses_the_storage() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let gpu = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.mem_events();
        tex_image(0, 8, 8, GL_RGB, GL_UNSIGNED_BYTE, &[]);
        tex_image(0, 3, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(9));
        assert_eq!(gl.mem_events(), []);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.mem.gpu, t.width, t.height, t.chain_levels), (gpu, 3, 3, 1));
        assert_eq!(storage(&gl, name)[16..28], ramp(9)[12..24], "row 1 at the new pitch");
    }

    #[test]
    fn respecifying_larger_size_frees_and_reallocates() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let old = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.mem_events();
        tex_image(0, 6, 6, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let new = gl.ctx().objs().textures[name as usize].mem.gpu;
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Free { gpu: old }, MemEvent::Alloc { heap: HEAP_DATA, size: 144, gpu: new }]
        );
        assert_eq!(gl.live_allocs(), 2);
    }

    #[test]
    fn respecifying_while_a_kick_samples_it_gets_fresh_storage() {
        /* copy-on-write: the pending kick still samples the old texels */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let old = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.ctx().objs().textures[name as usize].last_use = 7;
        gl.set_kick_pending(7, true);
        gl.mem_events();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        let new = gl.ctx().objs().textures[name as usize].mem.gpu;
        assert_ne!(new, old);
        let ev = gl.mem_events();
        assert!(ev.contains(&MemEvent::Free { gpu: old }));
        assert!(ev.iter().any(|e| matches!(e, MemEvent::Alloc { gpu, .. } if *gpu == new)));
        assert_eq!(storage(&gl, name)[..64], ramp(16)[..]);
    }

    #[test]
    fn respecifying_after_the_kick_completed_reuses_the_storage() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        gl.ctx().objs().textures[name as usize].last_use = 7;
        gl.set_kick_pending(8, true); /* another kick */
        gl.mem_events();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        assert_eq!(gl.mem_events(), []);
    }

    #[test]
    fn tex_image_over_external_storage_allocates_its_own() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0x77u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, GLES_SURF_XRGB8888), 0);
        gl.mem_events();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(!t.external && !t.bgra);
        assert_ne!(t.mem.gpu, mem.gpu);
        let ev = gl.mem_events();
        assert_eq!(ev.len(), 1, "the image is not freed: {ev:?}");
        assert!(matches!(ev[0], MemEvent::Alloc { .. }));
        assert_eq!(backing, vec![0x77u8; 64], "the image is not written");
    }

    #[test]
    fn tex_image_over_compressed_storage_reallocates_as_rgba8() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let blocks = [0u8; 16 * 16];
        // SAFETY: 16 blocks of 16 bytes.
        unsafe {
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGBA8_ETC2_EAC, 16, 16, 0, 256, ptr(&blocks));
        }
        let old = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.mem_events();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(t.texfmt, GLES_TEXFMT_RGBA8);
        assert!(gl.mem_events().contains(&MemEvent::Free { gpu: old }));
    }

    /* ---- glTexSubImage2D ------------------------------------------------------------------ */

    #[test]
    fn tex_sub_image_places_the_block_at_its_offset() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 64]);
        tex_sub_image(0, 1, 2, 2, 2, GL_RGBA, &ramp(4));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        let mut want = [0u8; 64];
        want[(2 * 4 + 1) * 4..][..8].copy_from_slice(&ramp(4)[..8]);
        want[(3 * 4 + 1) * 4..][..8].copy_from_slice(&ramp(4)[8..]);
        assert_eq!(s[..64], want);
    }

    #[test]
    fn tex_sub_image_on_odd_width_texture_uses_the_even_pitch() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 60]);
        tex_sub_image(0, 3, 1, 2, 2, GL_RGBA, &ramp(4));
        let s = storage(&gl, name);
        let mut want = pad_texels_unwritten([0u8; 72], 6);
        want[(6 + 3) * 4..][..8].copy_from_slice(&ramp(4)[..8]);
        want[(2 * 6 + 3) * 4..][..8].copy_from_slice(&ramp(4)[8..]);
        assert_eq!(s[..72], want);
    }

    #[test]
    fn tex_sub_image_converts_from_a_different_format() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 16]);
        // SAFETY: two 565 texels.
        unsafe {
            let px = [0xF800u16, 0x001F];
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 1, 2, 1, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, ptr(&px));
        }
        assert_eq!(storage(&gl, name)[8..16], [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn tex_sub_image_honours_unpack_row_length() {
        /* a 2x2 sub-rectangle at (1,1) of a 4-wide client image (the Qt RHI pattern) */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 16]);
        let img = ramp(16);
        // SAFETY: the pointer names texel (1,1) of the 4x4 image; 2 rows of 2 texels follow at row length 4.
        unsafe {
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 4);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&img[5 * 4..]));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        assert_eq!(s[..8], img[5 * 4..7 * 4]);
        assert_eq!(s[8..16], img[9 * 4..11 * 4]);
    }

    /// The same sub-rectangle named by GL_UNPACK_SKIP_ROWS / SKIP_PIXELS from
    /// the image's start (ES 3.0 3.7.4), and a PIXEL_UNPACK_BUFFER bound past
    /// the skipped texels too. The skips were read back as 0 and ignored
    /// until 2026-09-24.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn tex_sub_image_honours_the_unpack_skips() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 16]);
        let img = ramp(16);
        // SAFETY: `img` is the whole 4x4 image; the skips land inside it.
        unsafe {
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 4);
            glPixelStorei(GL_UNPACK_SKIP_ROWS, 1);
            glPixelStorei(GL_UNPACK_SKIP_PIXELS, 1);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&img));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        assert_eq!(s[..8], img[5 * 4..7 * 4]);
        assert_eq!(s[8..16], img[9 * 4..11 * 4]);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn tex_image_honours_unpack_row_length_with_alignment() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* RGB rows of a 5-texel-wide image: 15 bytes, padded to 16 */
        let mut img = vec![0u8; 32];
        img[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        img[16..22].copy_from_slice(&[7, 8, 9, 10, 11, 12]);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 5) };
        tex_image(0, 2, 2, GL_RGB, GL_UNSIGNED_BYTE, &img);
        assert_eq!(storage(&gl, name)[..16], [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
    }

    #[test]
    fn tex_sub_image_into_a_mip_level() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 16]);
        tex_sub_image(1, 1, 1, 1, 1, GL_RGBA, &[9, 8, 7, 6]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[64 + 12..64 + 16], [9, 8, 7, 6]);
    }

    #[test]
    fn tex_sub_image_level_without_texels_is_invalid_operation() {
        let gl = TestGl::new(4, 4);
        new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        tex_sub_image(1, 0, 0, 1, 1, GL_RGBA, &[9, 8, 7, 6]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn tex_sub_image_without_storage_or_binding_is_invalid_operation() {
        let gl = TestGl::new(4, 4);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 8, 7, 6]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        new_tex();
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 8, 7, 6]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn tex_sub_image_outside_the_level_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 64]);
        tex_sub_image(0, 3, 0, 2, 1, GL_RGBA, &ramp(2));
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_sub_image(0, 0, 4, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_sub_image(0, -1, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_sub_image(0, 0, 0, -1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_sub_image(0, i32::MAX, 0, i32::MAX, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(storage(&gl, name)[..64], [0u8; 64], "nothing was written");
    }

    #[test]
    fn tex_sub_image_bad_target_format_and_level() {
        let gl = TestGl::new(4, 4);
        new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        // SAFETY: one RGBA texel.
        unsafe {
            let px = [0u8; 4];
            glTexSubImage2D(GL_TEXTURE_CUBE_MAP, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px));
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA8, GL_UNSIGNED_BYTE, ptr(&px));
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexSubImage2D(GL_TEXTURE_2D, -1, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px));
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTexSubImage2D(GL_TEXTURE_2D, CAP_MIP_LEVELS as GLint, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px));
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn tex_sub_image_null_or_empty_is_a_no_op() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        // SAFETY: null pixels / empty rectangles read nothing.
        unsafe {
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glTexSubImage2D(GL_TEXTURE_2D, 0, 2, 2, 0, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&[0u8; 4]));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..16], ramp(4)[..]);
    }

    #[test]
    fn tex_sub_image_writes_into_an_external_rgba_image() {
        let gl = TestGl::new(4, 4);
        new_tex();
        let mut backing = vec![0u8; 4 * 2 * 4];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 2, backend::GLES_SURF_RGBA8888), 0);
        tex_sub_image(0, 2, 1, 2, 1, GL_RGBA, &ramp(2));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(backing[(4 + 2) * 4..], ramp(2)[..]);
        assert_eq!(backing[..(4 + 2) * 4], [0u8; 24]);
    }

    #[test]
    fn tex_sub_image_into_an_external_rgb565_image_is_invalid_operation() {
        /* the converter only produces RGBA8 texels */
        let gl = TestGl::new(4, 4);
        new_tex();
        let mut backing = vec![0u8; 4 * 2 * 2];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 2, GLES_SURF_RGB565), 0);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(backing, vec![0u8; 16]);
    }

    /* ---- glCopyTexImage2D / glCopyTexSubImage2D -------------------------------------------- */

    fn copy_tex_image(format: GLenum, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
        // SAFETY: plain GL call.
        unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, format, x, y, w, h, 0) };
    }

    /// With GL_UNPACK_ROW_LENGTH wider than the upload (femtovg's ES 3.0 path
    /// uploads a sub-rectangle of a larger image this way) the hook's stride
    /// is the row length's, not the upload width's: the caller's rows are
    /// that far apart, and a backend tiling from them at w*4 would read the
    /// wrong texels.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn the_upload_hooks_stride_follows_unpack_row_length() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        tex_image(0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(64));
        gl.uploads();
        /* a 3x2 box out of rows 7 texels long: 28 bytes, not 12 */
        let src = ramp(7 * 2);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 7) };
        tex_sub_image(0, 1, 1, 3, 2, GL_RGBA, &src);
        let up = gl.uploads();
        assert_eq!((up.len(), up[0].rect, up[0].stride), (1, [1, 1, 3, 2], 28));
        assert_eq!(up[0].first_row, src[..12], "the box's first row is the caller's first 3 texels");
        /* and the alignment still rounds it: 5 texels, 20 bytes, at 8: 24 */
        // SAFETY: plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 5);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
        }
        tex_sub_image(0, 0, 0, 2, 2, GL_RGBA, &ramp(6 + 2));
        let up = gl.uploads();
        assert_eq!((up.len(), up[0].stride), (1, 24));
        // SAFETY: plain GL calls; one valid name.
        unsafe {
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
            glDeleteTextures(1, &name);
        }
    }

    /// A copy from the framebuffer does not reach `texture_uploaded`: it is
    /// not the caller's rows, and a backend keeps its lazy path for it (the
    /// tile-at-upload decision, docs/online-shaderc/v3d-backend.md).
    #[test]
    fn a_copy_does_not_reach_the_upload_hook() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        new_tex();
        copy_tex_image(GL_RGBA, 0, 0, 4, 2);
        // SAFETY: plain GL call on the bound texture.
        unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 4, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.uploads().is_empty());
    }

    #[test]
    fn copy_tex_image_reads_the_default_surface_bottom_up() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        copy_tex_image(GL_RGBA, 1, 0, 2, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.format, t.type_), (2, 2, GL_RGBA, GL_UNSIGNED_BYTE));
        /* GL row 0 is the surface's last stored row */
        let want: Vec<u8> = [surf_texel(1, 3), surf_texel(2, 3), surf_texel(1, 2), surf_texel(2, 2)].concat();
        assert_eq!(storage(&gl, name)[..16], want[..]);
    }

    #[test]
    fn copy_tex_image_into_an_odd_width_texture_uses_the_even_pitch() {
        /* (an odd-width surface from gl_context_create cannot be used here: it is allocated
         * w * h texels but read at the pair pitch) */
        let gl = TestGl::new(6, 3);
        fill_surface(&gl, 6, 3);
        let name = new_tex();
        copy_tex_image(GL_RGBA, 1, 0, 5, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        assert_eq!(s.len(), 6 * 3 * 4);
        for y in 0..3u32 {
            for x in 0..5u32 {
                let o = ((y * 6 + x) * 4) as usize;
                assert_eq!(s[o..o + 4], surf_texel(x + 1, 2 - y), "texel ({x},{y})");
            }
        }
    }

    #[test]
    fn copy_tex_image_wider_than_one_chunk() {
        let gl = TestGl::new(100, 2);
        fill_surface(&gl, 100, 2);
        let name = new_tex();
        copy_tex_image(GL_RGBA, 0, 0, 100, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        for x in [0u32, 63, 64, 65, 99] {
            assert_eq!(s[(x * 4) as usize..][..4], surf_texel(x, 1), "texel ({x},0)");
            assert_eq!(s[((100 + x) * 4) as usize..][..4], surf_texel(x, 0), "texel ({x},1)");
        }
    }

    #[test]
    fn copy_tex_image_rgb_drops_alpha() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        copy_tex_image(GL_RGB, 2, 3, 1, 1);
        let [r, g, b, _] = surf_texel(2, 0);
        assert_eq!(storage(&gl, name)[..4], [r, g, b, 255]);
        assert_eq!(gl.ctx().objs().textures[name as usize].format, GL_RGB);
    }

    /* The V3D stores L8/A8/LA8 narrow and tiled for its TMU (board cases
     * 132 narrowla and 135 texnarrow PASS), so its storage bytes are not
     * these. The GC7000 stores them narrow and linear (`narrow_la_texels`),
     * the PowerVR test build expanded into RGBA8: both are checked. */
    #[cfg(not(feature = "gpu-v3d"))]
    #[test]
    fn copy_tex_image_luminance_takes_red() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        copy_tex_image(GL_LUMINANCE, 2, 3, 1, 1);
        let [r, _, _, a] = surf_texel(2, 0);
        let narrow = crate::backend::narrow_la_texels();
        if narrow {
            assert_eq!(storage(&gl, name)[..1], [r]);
        } else {
            assert_eq!(storage(&gl, name)[..4], [r, r, r, 255]);
        }
        copy_tex_image(GL_LUMINANCE_ALPHA, 2, 3, 1, 1);
        if narrow {
            assert_eq!(storage(&gl, name)[..2], [r, a]);
        } else {
            assert_eq!(storage(&gl, name)[..4], [r, r, r, a]);
        }
    }

    /* The V3D stores L8/A8/LA8 narrow and tiled for its TMU (board cases
     * 132 narrowla and 135 texnarrow PASS), so its storage bytes are not
     * these. The GC7000 stores them narrow and linear (`narrow_la_texels`),
     * the PowerVR test build expanded into RGBA8: both are checked. */
    #[cfg(not(feature = "gpu-v3d"))]
    #[test]
    fn copy_tex_image_alpha_keeps_alpha_only() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        copy_tex_image(GL_ALPHA, 3, 0, 1, 1);
        if crate::backend::narrow_la_texels() {
            assert_eq!(storage(&gl, name)[..1], [surf_texel(3, 3)[3]]);
        } else {
            assert_eq!(storage(&gl, name)[..4], [0, 0, 0, surf_texel(3, 3)[3]]);
        }
    }

    #[test]
    fn copy_tex_image_partly_outside_the_surface_copies_the_inside() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        copy_tex_image(GL_RGBA, -1, 3, 2, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* texel (1,0) = window (0,3) = stored row 0; the rest is undefined */
        assert_eq!(storage(&gl, name)[4..8], surf_texel(0, 0));
    }

    #[test]
    fn copy_tex_image_argument_errors() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls.
        unsafe {
            glCopyTexImage2D(GL_TEXTURE_CUBE_MAP, 0, GL_RGBA, 0, 0, 2, 2, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            /* a depth name: no such copy format in ES 2.0 (GL_INVALID_ENUM),
             * a name with no colour to copy in ES 3.0 (3.8.5, GL_INVALID_OPERATION) */
            glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT, 0, 0, 2, 2, 0);
            assert_eq!(gl.error(), if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_INVALID_OPERATION });
            glCopyTexImage2D(GL_TEXTURE_2D, -1, GL_RGBA, 0, 0, 2, 2, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, -2, 2, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 2, 2, 1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null());
    }

    #[test]
    fn copy_tex_image_without_a_binding_defines_the_default_texture_object() {
        let gl = TestGl::new(4, 4);
        copy_tex_image(GL_RGBA, 0, 0, 2, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = &gl.ctx().objs_ref().textures[MAX_TEXTURES];
        assert_eq!((d.used, d.width, d.height), (true, 2, 2));
    }

    #[test]
    fn copy_tex_image_from_incomplete_framebuffer() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut fb = 0u32;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        copy_tex_image(GL_RGBA, 0, 0, 2, 2);
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null());
    }

    #[test]
    fn copy_tex_image_zero_size_defines_an_empty_level() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        copy_tex_image(GL_RGBA, 0, 0, 0, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().textures[name as usize].width, 0);
    }

    #[test]
    fn copy_tex_image_is_not_affected_by_a_stale_error() {
        /* GL: an error flag set by an earlier command does not change what later commands do */
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        tex_image(0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[0, 0, 0, 0]);
        // SAFETY: plain GL call (raises GL_INVALID_ENUM, not fetched).
        unsafe { glBindTexture(BAD_TARGET, 0) };
        copy_tex_image(GL_RGBA, 0, 0, 1, 1);
        assert_eq!(storage(&gl, name)[..4], surf_texel(0, 3));
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn copy_tex_sub_image_places_the_block() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 60]);
        // SAFETY: plain GL call.
        unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 3, 1, 2, 1, 2, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        let mut want = pad_texels_unwritten([0u8; 72], 6);
        /* window rows 1, 2 = stored rows 2, 1; the texture's pitch is 6 */
        want[(6 + 3) * 4..][..8].copy_from_slice(&[surf_texel(2, 2), surf_texel(3, 2)].concat());
        want[(12 + 3) * 4..][..8].copy_from_slice(&[surf_texel(2, 1), surf_texel(3, 1)].concat());
        assert_eq!(s[..72], want);
    }

    #[test]
    fn copy_tex_sub_image_converts_to_the_textures_format() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGB, GL_UNSIGNED_BYTE, &[]);
        // SAFETY: plain GL call.
        unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 3, 1, 1) };
        let [r, g, b, _] = surf_texel(1, 0);
        assert_eq!(storage(&gl, name)[..4], [r, g, b, 255]);
    }

    #[test]
    fn copy_tex_sub_image_errors() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no texture bound");
            new_tex();
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no storage");
            tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
            glCopyTexSubImage2D(GL_TEXTURE_CUBE_MAP, 0, 0, 0, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "level without texels");
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 3, 0, 0, 0, 2, 1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, -1, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            let mut fb = 0u32;
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        }
    }

    /* ---- compressed textures ------------------------------------------------------------------ */

    /// `n` blocks of `bb` bytes, block i filled with the byte i + 1.
    fn blocks(n: usize, bb: usize) -> Vec<u8> {
        (0..n * bb).map(|i| (i / bb + 1) as u8).collect()
    }

    /// Level 0's blocks, every face: the storage holds the whole chain after them.
    fn level0_blocks(gl: &TestGl, name: GLuint) -> Vec<u8> {
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let n = (t.block_w * t.block_h * t.block_bytes as u32 * t.layers as u32) as usize;
        storage(gl, name)[..n].to_vec()
    }

    fn compressed_image(format: GLenum, w: GLsizei, h: GLsizei, data: &[u8]) {
        // SAFETY: `data` holds imageSize bytes.
        unsafe { glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, w, h, 0, data.len() as GLsizei, ptr(data)) };
    }

    /* ---- ETC1 decoded at upload (YakoglBackend::etc1_decoded_at_upload) ---- */

    /// Two ETC1 blocks with known texels (src/gl/etc1.rs's hand vectors):
    /// A is (144, 76, 8) in its left two columns and (217, 255, 255) in its
    /// right two; B is a differential, flipped block.
    const ETC1_A: [u8; 8] = [0x82, 0x4c, 0x0f, 0x1c, 0x00, 0x00, 0xff, 0xff];
    const ETC1_B: [u8; 8] = [0x83, 0xfc, 0x01, 0x57, 0xa5, 0xc3, 0x3c, 0x96];

    fn etc1_image(target: GLenum, level: GLint, w: GLsizei, h: GLsizei, data: &[u8]) {
        // SAFETY: `data` holds imageSize bytes.
        unsafe { glCompressedTexImage2D(target, level, GL_ETC1_RGB8_OES, w, h, 0, data.len() as GLsizei, ptr(data)) };
    }

    /// Texel (x, y) of face `face` of level `level` of texture `name`, as stored.
    fn stored_texel(gl: &TestGl, name: GLuint, level: u32, face: u32, x: u32, y: u32) -> [u8; 4] {
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (lw, lh) = level_size(t, level);
        let o = t.level_off[level as usize] as usize + (((face * lh + y) * t.level_pitch(lw) + x) * 4) as usize;
        let s = storage(gl, name);
        [s[o], s[o + 1], s[o + 2], s[o + 3]]
    }

    /// Every texel of a w x h level (face `face`) against the decode of its
    /// block in `blocks` (rows of ceil(w/4) blocks).
    fn assert_decoded(gl: &TestGl, name: GLuint, level: u32, face: u32, w: u32, h: u32, blocks: &[[u8; 8]]) {
        let bw = w.div_ceil(4);
        for y in 0..h {
            for x in 0..w {
                let want = super::super::etc1::decode_block(&blocks[((y / 4) * bw + x / 4) as usize]);
                assert_eq!(
                    stored_texel(gl, name, level, face, x, y),
                    want[((y % 4) * 4 + x % 4) as usize],
                    "level {level} face {face} texel ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn etc1_is_decoded_into_an_rgb_level_when_the_backend_asks() {
        let gl = TestGl::new(4, 4);
        gl.backend_decodes_etc1();
        let name = new_tex();
        etc1_image(GL_TEXTURE_2D, 0, 8, 4, &[ETC1_A, ETC1_B].concat());
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.texfmt, t.block_bytes), (8, 4, GLES_TEXFMT_RGBA8, 0));
        assert_eq!((t.format, t.type_), (GL_RGB, GL_UNSIGNED_BYTE), "an RGB texture like glTexImage2D's");
        assert_eq!(stored_texel(&gl, name, 0, 0, 0, 0), [144, 76, 8, 255]);
        assert_eq!(stored_texel(&gl, name, 0, 0, 3, 3), [217, 255, 255, 255]);
        assert_decoded(&gl, name, 0, 0, 8, 4, &[ETC1_A, ETC1_B]);
    }

    #[test]
    fn etc1_decoded_npot_image_cuts_its_edge_blocks() {
        /* the path of the three mip-less cases (format.etc1_2d_npot and
         * wrap.clamp_clamp_*_npot_etc1: one 113x89 level): a size that is a
         * multiple of 4 on neither axis, here 13x9 = 4x3 blocks */
        let gl = TestGl::new(4, 4);
        gl.backend_decodes_etc1();
        let name = new_tex();
        let grid: Vec<[u8; 8]> = (0..12).map(|i| if i % 3 == 0 { ETC1_B } else { ETC1_A }).collect();
        etc1_image(GL_TEXTURE_2D, 0, 13, 9, &grid.concat());
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.levels), (13, 9, 1));
        assert_decoded(&gl, name, 0, 0, 13, 9, &grid);
        /* nothing is written past the image: the pair-padded 14th column
         * keeps the cleared storage */
        assert_eq!(t.level_pitch(13), 14);
        for y in 0..9 {
            assert_eq!(stored_texel(&gl, name, 0, 0, 13, y), [0; 4], "padding texel of row {y}");
        }
    }

    #[test]
    fn etc1_decoded_mip_levels_and_cube_faces_are_taken() {
        let gl = TestGl::new(4, 4);
        gl.backend_decodes_etc1();
        /* a full chain, 8x8 down to 1x1, as filtering.2d.*_etc1 uploads it */
        let name = new_tex();
        etc1_image(GL_TEXTURE_2D, 0, 8, 8, &[ETC1_A, ETC1_B, ETC1_B, ETC1_A].concat());
        for (level, size) in [(1, 4), (2, 2), (3, 1)] {
            etc1_image(GL_TEXTURE_2D, level, size, size, &ETC1_B);
            assert_eq!(gl.error(), GL_NO_ERROR, "level {level}");
        }
        assert_eq!(gl.ctx().objs_ref().textures[name as usize].levels, 4);
        assert_decoded(&gl, name, 0, 0, 8, 8, &[ETC1_A, ETC1_B, ETC1_B, ETC1_A]);
        for (level, size) in [(1, 4), (2, 2), (3, 1)] {
            assert_decoded(&gl, name, level, 0, size, size, &[ETC1_B]);
        }
        /* a cube map, one call per face (filtering.cube.*_etc1) */
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        for f in 0..6 {
            let block = if f % 2 == 0 { ETC1_A } else { ETC1_B };
            etc1_image(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, 4, 4, &block);
            assert_eq!(gl.error(), GL_NO_ERROR, "face {f}");
        }
        for f in 0..6 {
            let block = if f % 2 == 0 { ETC1_A } else { ETC1_B };
            assert_decoded(&gl, cube, 0, f, 4, 4, &[block]);
        }
    }

    #[test]
    fn etc1_decode_keeps_the_upload_errors() {
        let gl = TestGl::new(4, 4);
        gl.backend_decodes_etc1();
        let name = new_tex();
        etc1_image(GL_TEXTURE_2D, 0, 8, 4, &ETC1_A);
        assert_eq!(gl.error(), GL_INVALID_VALUE, "imageSize of an 8x4 image is two blocks");
        etc1_image(GL_TEXTURE_2D, 0, 8, 8, &[ETC1_A; 4].concat());
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* level 1 of an 8x8 chain is 4x4: a 2x2 one is recorded, holds no
         * texels and leaves the texture incomplete, as glTexImage2D takes it
         * (ES 2.0 3.7.10); its texels must not land in the chain's level 1 */
        etc1_image(GL_TEXTURE_2D, 1, 2, 2, &ETC1_B);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(stored_texel(&gl, name, 1, 0, x, y), [0; 4], "chain level 1 texel ({x}, {y})");
            }
        }
        /* OES_compressed_ETC1_RGB8_texture: no CompressedTexSubImage2D */
        // SAFETY: `ETC1_A` holds one block.
        unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_ETC1_RGB8_OES, 8, ptr(&ETC1_A)) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_decoded(&gl, name, 0, 0, 8, 8, &[ETC1_A; 4]);
    }

    #[test]
    fn etc1_stays_compressed_on_a_backend_that_samples_it() {
        /* the PowerVR-shaped mock (etc1_decoded_at_upload false): the path
         * this change leaves as it was -- blocks stored as ETC2 RGB, levels
         * and cube faces included, never decoded */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        etc1_image(GL_TEXTURE_2D, 0, 4, 4, &ETC1_A);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.texfmt, t.block_bytes), (GLES_TEXFMT_ETC2_RGB, 8));
        assert_eq!(storage(&gl, name)[..8], ETC1_A, "the block itself, not texels");
        etc1_image(GL_TEXTURE_2D, 1, 2, 2, &ETC1_B);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.texfmt, t.block_bytes), (GLES_TEXFMT_ETC2_RGB, 8), "level 1 kept as blocks");
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        etc1_image(GL_TEXTURE_CUBE_MAP_POSITIVE_X, 0, 4, 4, &ETC1_A);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.texfmt, t.block_bytes), (GLES_TEXFMT_ETC2_RGB, 8), "a cube face kept as blocks");
    }

    #[test]
    fn compressed_etc2_rgb_blocks_are_stored_twiddled() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.texfmt, t.block_bytes), (8, 8, GLES_TEXFMT_ETC2_RGB, 8));
        assert_eq!((t.block_w, t.block_h, t.chain_levels, t.levels), (2, 2, 4, 1));
        assert_eq!((t.format, t.type_), (GL_COMPRESSED_RGB8_ETC2, 0));
        /* level 0's 2x2 blocks, then one block for each of 4x4, 2x2 and 1x1 */
        assert_eq!(t.mem.size, 32 + 3 * 8);
        /* twiddled order: (0,0) (0,1) (1,0) (1,1) = source blocks 1, 3, 2, 4 */
        let s = level0_blocks(&gl, name);
        assert_eq!(s, [[1u8; 8], [3; 8], [2; 8], [4; 8]].concat());
    }

    #[test]
    fn compressed_etc2_rgba_uses_16_byte_blocks() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGBA8_ETC2_EAC, 8, 4, &blocks(2, 16));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.texfmt, t.block_bytes, t.block_w, t.block_h), (GLES_TEXFMT_ETC2_RGBA, 16, 2, 1));
        assert_eq!(level0_blocks(&gl, name), [[1u8; 16], [2; 16]].concat());
    }

    /// The compressed sRGB ETC2 formats are stored as blocks in their own
    /// texel classes where the back end samples them (the PowerVR, in an
    /// ES 3.0 report), and stay GL_INVALID_ENUM, with nothing stored,
    /// everywhere else. The RGB one has no alpha, as GL_COMPRESSED_RGB8_ETC2.
    #[test]
    fn compressed_srgb_etc2_is_stored_as_blocks_in_the_srgb_class() {
        let gl = TestGl::new(4, 4);
        let stored = cfg!(feature = "gpu-powervr") && !crate::REPORT_ES2;
        for (format, bb, class, no_alpha) in [
            (GL_COMPRESSED_SRGB8_ETC2, 8usize, GLES_TEXFMT_ETC2_SRGB_RGB, true),
            (GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, 16, GLES_TEXFMT_ETC2_SRGB_RGBA, false),
        ] {
            let name = new_tex();
            compressed_image(format, 8, 4, &blocks(2, bb));
            let t = &gl.ctx().objs_ref().textures[name as usize];
            if !stored {
                assert_eq!(gl.error(), GL_INVALID_ENUM, "format {format:#x}");
                assert_eq!((t.block_bytes, t.mem.impl_.is_null()), (0, true), "format {format:#x}: nothing stored");
                continue;
            }
            assert_eq!(gl.error(), GL_NO_ERROR, "format {format:#x}");
            assert_eq!((t.texfmt, t.block_bytes, t.format), (class, bb as u8, format));
            assert_eq!(draw::texture_has_no_alpha(t), no_alpha, "format {format:#x}");
            assert_eq!(level0_blocks(&gl, name), blocks(2, bb));
            /* a sub-image of the same format replaces a block */
            let b = [9u8; 16];
            // SAFETY: one block of `bb` bytes.
            unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 0, 4, 4, format, bb as GLsizei, ptr(&b[..bb])) };
            assert_eq!(gl.error(), GL_NO_ERROR, "format {format:#x}: sub-image");
            assert_eq!(level0_blocks(&gl, name)[bb..], b[..bb], "format {format:#x}: the second block");
        }
    }

    #[test]
    fn compressed_etc1_is_stored_as_etc2_rgb() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_ETC1_RGB8_OES, 4, 4, &blocks(1, 8));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().textures[name as usize].texfmt, GLES_TEXFMT_ETC2_RGB);
        assert_eq!(level0_blocks(&gl, name), [1u8; 8]);
    }

    #[test]
    fn compressed_npot_size_gets_a_power_of_two_block_grid() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* 10x5 texels = 3x2 blocks, held in a 4x2 grid */
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 10, 5, &blocks(6, 8));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.block_w, t.block_h), (10, 5, 4, 2));
        /* then 5x2 (2 blocks, a 2x1 grid), 2x1 and 1x1 */
        assert_eq!(t.mem.size, 8 * 8 + 2 * 8 + 8 + 8);
        let s = level0_blocks(&gl, name);
        /* index = y | x0 << 1 | (x >> 1) << 2: source block (bx,by) = by * 3 + bx + 1 */
        /* slots 6 and 7 are never written: the cleared storage */
        let want_blocks: [u8; 8] = [1, 4, 2, 5, 3, 6, 0, 0];
        for (i, b) in want_blocks.iter().enumerate() {
            assert_eq!(s[i * 8..i * 8 + 8], [*b; 8], "grid slot {i}");
        }
    }

    /// The twiddled chain's level offsets are the TPU's: Mesa
    /// pvr_image_plane_setup_mip_levels lays level l out as
    /// max(1, (POT(W)/4) >> l) x max(1, (POT(H)/4) >> l) blocks, packed.
    /// Written here from that formula, independently of `level_block_grid`,
    /// for a NPOT size where a level's own size rounds differently (17x5:
    /// level 1 is 8 texels, 2 blocks, where the TPU steps 4), a wide POT
    /// and a square POT one.
    #[test]
    fn the_twiddled_etc_chain_is_laid_out_as_the_tpu_walks_it() {
        let _gl = TestGl::new(4, 4);
        let mesa = |w: u32, h: u32, levels: u32| -> (Vec<u32>, u32) {
            let (bw0, bh0) = (w.next_power_of_two().div_ceil(4), h.next_power_of_two().div_ceil(4));
            let (mut off, mut at) = (Vec::new(), 0u32);
            for l in 0..levels {
                off.push(at);
                at += (bw0 >> l).max(1) * (bh0 >> l).max(1) * 8;
            }
            (off, at)
        };
        for (w, h) in [(17u32, 5u32), (64, 16), (16, 16)] {
            let (off, levels, bytes) = compressed_chain_layout(w, h, 1, 8).expect("fits");
            let (want, want_bytes) = mesa(w, h, levels as u32);
            assert_eq!(&off[..levels as usize], &want[..], "{w}x{h}: level offsets");
            assert_eq!(bytes, want_bytes, "{w}x{h}: chain bytes");
        }
    }

    #[test]
    fn a_backend_that_reads_block_rows_gets_rows_and_no_padded_grid() {
        /* upstream d8279b8: Mesa gives a compressed resource
         * ETNA_LAYOUT_LINEAR and pads it by nothing, so the GC7000 reads 4x4
         * blocks as plain raster rows; only the twiddled order needs a
         * power-of-two grid, because it interleaves the coordinates' bits. */
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = new_tex();
        /* 10x5 texels = 3x2 blocks, and 3x2 is what the storage holds */
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 10, 5, &blocks(6, 8));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.block_w, t.block_h), (3, 2), "no power-of-two padding");
        /* level 0 48 bytes; 5x2 (2x1 blocks), 2x1 and 1x1 each start on
         * this backend's 64-byte level boundary */
        assert_eq!(t.level_off[..4], [0, 64, 128, 192]);
        assert_eq!(t.mem.size, 192 + 8);
        let s = level0_blocks(&gl, name);
        /* block (bx, by) at by * 3 + bx, which is the order they arrived in */
        for i in 0..6 {
            assert_eq!(s[i * 8..i * 8 + 8], [(i + 1) as u8; 8], "block {i}");
        }
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn a_3d_texture_is_stacked_for_a_backend_that_walks_the_stack() {
        /* upstream b0e3e4a: the GC7000's texture unit looks for slice k at
         * k * SLICE, which a grid is not; stacked is what a 2D array already
         * does here, and its layer stride is the same pitch * height. */
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = bind_new(GL_TEXTURE_3D);
        // SAFETY: plain GL call; no texels are read.
        unsafe {
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8 as GLint, 2, 2, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null())
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.layers), (2, 2, 4));
        assert_eq!(t.grid_cols, 0, "no grid: the slices are stacked");
        /* the 3D chain: 2x2x4, 1x1x2, 1x1x1 -- the depth outlasts the sides,
         * so the last level is past the 2D chain of 2x2 (tex3d-finish) */
        assert_eq!(t.chain_levels, 3, "2x2x4 gets room for its whole 3D chain");
        /* level 0 is four stacked 2x2 slices, and each level after it starts
         * on the 64-byte boundary this backend fetches levels from, with
         * four slots of the level's size (the GC7000's walk) */
        /* pitch 2 x (2 rows, then 1, then 1) x 4 slices x 4 bytes */
        let (level0, level1, level2): (u32, u32, u32) = (2 * 2 * 4 * 4, 2 * 4 * 4, 2 * 4 * 4);
        assert_eq!(t.level_off[1], level0);
        assert_eq!(t.level_off[2], (level0 + level1).next_multiple_of(64));
        assert_eq!(t.mem.size as u32, 128 + level2);
    }

    #[test]
    fn compressed_null_data_allocates_only() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: null data.
        unsafe {
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 8, 8, 0, 32, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().textures[name as usize].mem.size, 32 + 3 * 8, "room for the chain");
    }

    #[test]
    fn compressed_image_errors() {
        let gl = TestGl::new(4, 4);
        let data = blocks(4, 8);
        // SAFETY: `data` holds 32 bytes; the failing calls read nothing.
        unsafe {
            /* nothing bound is the default texture object (ES 3.0 3.8), and
             * a compressed image lands in it like any other */
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 8, 8, 0, 32, ptr(&data));
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(gl.ctx().objs().textures[MAX_TEXTURES].block_bytes != 0);
            let name = new_tex();
            glCompressedTexImage2D(GL_TEXTURE_CUBE_MAP, 0, GL_COMPRESSED_RGB8_ETC2, 8, 8, 0, 32, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, 32, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a compressed format");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 8, 8, 0, 31, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "imageSize");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGBA8_ETC2_EAC, 8, 8, 0, 32, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "imageSize of the RGBA format is 64");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, -8, 8, 0, 32, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 8, 8, 1, 32, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "border");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 4100, 4, 0, 1025 * 8, ptr(&data));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "beyond the maximum texture size");
            assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null());
        }
    }

    #[test]
    fn compressed_image_out_of_memory() {
        let gl = TestGl::new(4, 4);
        new_tex();
        gl.fail_alloc_in(1);
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), 1);
    }

    #[test]
    fn compressed_respecify_same_size_reuses_the_storage() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        gl.mem_events();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &[9u8; 32]);
        assert_eq!(gl.mem_events(), []);
        assert_eq!(level0_blocks(&gl, name), [9u8; 32]);
    }

    #[test]
    fn compressed_respecify_under_a_pending_kick_reallocates() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        let old = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.ctx().objs().textures[name as usize].last_use = 3;
        gl.set_kick_pending(3, true);
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &[9u8; 32]);
        assert_ne!(gl.ctx().objs().textures[name as usize].mem.gpu, old);
    }

    #[test]
    fn compressed_sub_image_replaces_whole_blocks() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        let b = [0x99u8; 8];
        // SAFETY: one 8-byte block.
        unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* block (1,0) is grid slot 2 */
        assert_eq!(level0_blocks(&gl, name), [[1u8; 8], [3; 8], [0x99; 8], [4; 8]].concat());
    }

    #[test]
    fn compressed_sub_image_partial_block_at_the_texture_edge() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 6, 6, &blocks(4, 8));
        let b = [0x99u8; 8];
        // SAFETY: one 8-byte block.
        unsafe {
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 2, 2, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_NO_ERROR);
            /* a partial block that does not reach the edge is refused */
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert_eq!(level0_blocks(&gl, name), [[1u8; 8], [3; 8], [2; 8], [0x99; 8]].concat());
    }

    #[test]
    fn compressed_sub_image_errors() {
        let gl = TestGl::new(4, 4);
        let b = [0x99u8; 16];
        // SAFETY: the failing calls read nothing.
        unsafe {
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no texture");
            let name = new_tex();
            compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
            glCompressedTexSubImage2D(GL_TEXTURE_CUBE_MAP, 0, 0, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGB, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_RGBA8_ETC2_EAC, 16, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "format differs from the texture's");
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "level");
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "offset inside a block");
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 8, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "outside");
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, -4, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 16, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "imageSize");
            assert_eq!(level0_blocks(&gl, name), [[1u8; 8], [3; 8], [2; 8], [4; 8]].concat(), "untouched");
        }
    }

    #[test]
    fn uncompressed_updates_of_a_compressed_texture_are_invalid_operation() {
        let gl = TestGl::new(4, 4);
        new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        // SAFETY: plain GL calls.
        unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn compressed_sub_image_on_an_uncompressed_texture_is_invalid_operation() {
        let gl = TestGl::new(4, 4);
        new_tex();
        tex_image(0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let b = [0u8; 8];
        // SAFETY: the failing call reads nothing.
        unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&b)) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    /* ---- glGenerateMipmap ------------------------------------------------------------------------ */

    #[test]
    fn generate_mipmap_box_filters_with_round_half_up() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* R per 2x2 block: {1,2,2,2} {1,1,1,2} / {1,1,2,2} {255 x4}; G = 100; B = 16 x; A = 255 */
        let r: [[u8; 4]; 4] = [[1, 2, 1, 1], [2, 2, 1, 2], [1, 1, 255, 255], [2, 2, 255, 255]];
        let mut img = Vec::new();
        for row in &r {
            for (x, rv) in row.iter().enumerate() {
                img.extend_from_slice(&[*rv, 100, 16 * x as u8, 255]);
            }
        }
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        /* R: 7/4 = 1.75 -> 2, 5/4 = 1.25 -> 1, 6/4 = 1.5 -> 2, 255; B: (0+16)/2 = 8, (32+48)/2 = 40 */
        assert_eq!(s[64..80], [2, 100, 8, 255, 1, 100, 40, 255, 2, 100, 8, 255, 255, 100, 40, 255]);
        /* level 2 filters level 1: R (2+1+2+255)/4 = 65, B (8+40+8+40)/4 = 24 */
        assert_eq!(s[80..84], [65, 100, 24, 255]);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.levels, t.chain_levels), (3, 3));
    }

    /// A backend that makes the chain itself (`YakoglBackend::generate_mipmap`
    /// answering 1: the V3D's render image, filled by the TFU): glGenerateMipmap
    /// filters nothing and moves no stamp, the record says the whole chain
    /// exists, and the chain is asked back (`return_mipmap_chain`) once,
    /// before the next CPU write of the texels. A backend answering 0 keeps
    /// the CPU filter and is asked nothing back.
    #[test]
    fn generate_mipmap_delegated_to_the_backend_filters_nothing_and_asks_the_chain_back_once() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let img: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        let before = storage(&gl, name);
        let stamp = gl.ctx().objs_ref().textures[name as usize].content_gen;
        gl.answer_generate_mipmap(1);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let calls = gl.mipmap_generates();
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].levels, calls[0].b.width, calls[0].b.height, calls[0].b.content_gen), (3, 4, 4, stamp));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(calls[0].b.base_gpu, t.mem.gpu);
        assert!(t.chain_in_backend);
        assert_eq!((t.levels, t.levels_written, t.chain_levels), (3, 3, 3));
        assert_eq!((t.level_defined, t.level_bad), (0b111, 0));
        assert_eq!(t.content_gen, stamp, "nothing was written: the stamp stays");
        assert_eq!(storage(&gl, name)[64..84], before[64..84], "levels 1 and 2 of the storage are untouched");
        assert!(gl.chain_returns().is_empty());
        /* a CPU write asks for the chain first, once */
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let back = gl.chain_returns();
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].levels, back[0].b.width, back[0].b.base_gpu), (3, 4, calls[0].b.base_gpu));
        assert!(!gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert!(gl.chain_returns().is_empty(), "nothing to ask back twice");
        /* answering 0: the CPU filters as before, and nothing is asked back */
        gl.answer_generate_mipmap(0);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.mipmap_generates().len(), 1, "asked, and refused");
        assert!(!gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
        assert_ne!(storage(&gl, name)[64..68], before[64..68], "the CPU filtered level 1");
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert!(gl.chain_returns().is_empty());
        /* a new level 0 lays the storage out again: the backend's chain is
         * forgotten with it (the backend dropped its image with the storage) */
        gl.answer_generate_mipmap(1);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert!(gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
        tex_image(0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert!(!gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
    }

    /// The PowerVR's chain of a non-power-of-two image, laid out as its
    /// power-of-two extent's (`pot_chain`: glmark2's 800x600 terrain), is
    /// offered to the backend too (the PowerVR draws the levels into this
    /// storage, src/pvr/backend.rs pvr_generate_mipmap): the bind carries
    /// level 0's physical pitch, and a backend that answers 1 leaves the
    /// storage's levels 1.. untouched until they are asked back.
    #[test]
    fn generate_mipmap_of_a_powervr_npot_chain_is_offered_to_the_backend() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let base: Vec<u8> = (0..30u8).flat_map(|i| [i * 8, 240 - i * 8, i, 255 - i]).collect();
        tex_image(0, 6, 5, GL_RGBA, GL_UNSIGNED_BYTE, &base);
        gl.answer_generate_mipmap(1);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(t.pot_chain && t.chain_in_backend);
        assert_eq!((t.chain_levels, t.levels, t.levels_written), (3, 3, 3));
        let calls = gl.mipmap_generates();
        assert_eq!(calls.len(), 1);
        let b = &calls[0].b;
        assert_eq!((calls[0].levels, b.width, b.height, b.stride_texels, b.base_gpu), (3, 6, 5, 8, t.mem.gpu));
        assert_eq!((b.format, b.bgra, b.layers), (GLES_TEXFMT_RGBA8, 0, 1));
        let s = storage(&gl, name);
        let l1 = t.level_off[1] as usize;
        assert!(s[l1..l1 + 16].iter().all(|&x| x == 0), "level 1 of the storage is as the clear left it");
        /* a CPU write asks the chain back first */
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.chain_returns().len(), 1);
        assert!(!gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
    }

    /// A level above 0 attached to a framebuffer is rendered into and read
    /// through the storage: attaching one asks a delegated chain back, and
    /// glGenerateMipmap is not delegated while one is attached (ES 3.0
    /// attaches levels above 0; ES 2.0 cannot).
    #[test]
    fn a_level_above_zero_attached_keeps_the_mip_chain_on_the_cpu() {
        if crate::REPORT_ES2 {
            return;
        }
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        gl.answer_generate_mipmap(1);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.mipmap_generates().len(), 1, "delegated with no level above 0 attached");
        assert!(gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
        let mut fb = 0;
        // SAFETY: plain GL calls; `fb` receives one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 1);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.chain_returns().len(), 1, "attaching level 1 asks the chain back");
        assert!(!gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.mipmap_generates().is_empty(), "not delegated while level 1 is attached");
        assert!(!gl.ctx().objs_ref().textures[name as usize].chain_in_backend);
    }

    /// An sRGB texture's chain is filtered in linear space: red, green and
    /// blue decoded, averaged and encoded (`gl::srgb`), alpha averaged as it
    /// is -- a 2x2 of black and white is 188 (0.5 linear), not the 128 of
    /// the RGBA8 box filter, which is what dEQP's linear-space reference
    /// expects of a mipmapped sRGB texture.
    #[test]
    fn generate_mipmap_of_an_srgb_texture_averages_in_linear_space() {
        if crate::REPORT_ES2 {
            return;
        }
        let gl = TestGl::new(4, 4);
        let img = [0u8, 0, 0, 0, 255, 255, 255, 255, 255, 64, 0, 255, 0, 64, 255, 0];
        let mut level1 = [[0u8; 4]; 2];
        for (k, internal) in [GL_SRGB8_ALPHA8, GL_RGBA8].into_iter().enumerate() {
            let name = new_tex();
            // SAFETY: plain GL calls; `img` holds the 2x2 image.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&img));
                glGenerateMipmap(GL_TEXTURE_2D);
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            let t = &gl.ctx().objs_ref().textures[name as usize];
            let want = if k == 0 { GLES_TEXFMT_SRGB8_A8 } else { GLES_TEXFMT_RGBA8 };
            assert_eq!((t.texfmt, t.levels), (want, 2));
            let off = t.level_off[1] as usize;
            level1[k].copy_from_slice(&storage(&gl, name)[off..off + 4]);
        }
        let lin = |v: [u8; 4]| {
            super::super::srgb::encode(v.iter().map(|&b| super::super::srgb::decode(b)).sum::<f32>() / 4.0)
        };
        assert_eq!(level1[0], [lin([0, 255, 255, 0]), lin([0, 255, 64, 64]), lin([0, 255, 0, 255]), 128]);
        assert_eq!(level1[0], [188, 143, 188, 128], "worked: (0 + 1 + 1 + 0) / 4 = 0.5 linear is 188");
        assert_eq!(level1[1], [128, 96, 128, 128], "the RGBA8 box filter, for comparison");
    }

    /// The sRGB pair's storage: the RGBA8 bytes as uploaded (no conversion:
    /// they are encoded already), in the sRGB class the samplers decode;
    /// SRGB8 widened with alpha 255 and bound as having no alpha. Under the
    /// ES 2.0 report glTexImage2D refuses both names, as it always did.
    #[test]
    fn srgb_textures_keep_their_bytes_in_the_srgb_class() {
        let gl = TestGl::new(4, 4);
        let rgba = [10u8, 120, 250, 77, 0, 1, 2, 3];
        let rgb = [10u8, 120, 250, 0, 1, 2];
        let a = new_tex();
        // SAFETY: plain GL calls; the arrays hold the 2x1 images.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_SRGB8_ALPHA8 as GLint, 2, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&rgba))
        };
        if crate::REPORT_ES2 {
            assert_eq!(gl.error(), GL_INVALID_VALUE, "no sRGB format in ES 2.0 (GL_EXT_sRGB is not claimed)");
            return;
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let b = new_tex();
        // SAFETY: as above.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_SRGB8 as GLint, 2, 1, 0, GL_RGB, GL_UNSIGNED_BYTE, ptr(&rgb)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let ta = &gl.ctx().objs_ref().textures[a as usize];
        assert_eq!((ta.texfmt, ta.format), (GLES_TEXFMT_SRGB8_A8, GL_RGBA));
        assert_eq!(storage(&gl, a)[..8], rgba);
        let tb = &gl.ctx().objs_ref().textures[b as usize];
        assert_eq!((tb.texfmt, tb.format), (GLES_TEXFMT_SRGB8_A8, GL_RGB));
        assert_eq!(storage(&gl, b)[..8], [10, 120, 250, 255, 0, 1, 2, 255]);
        let bind = super::super::draw::texture_bind_2d(tb);
        assert_eq!((bind.format, bind.no_alpha), (GLES_TEXFMT_SRGB8_A8, 1));
    }

    #[test]
    fn generate_mipmap_of_a_one_row_texture_averages_pairs() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let img: Vec<u8> = [10u8, 21, 30, 50].iter().flat_map(|v| [*v, 0, 255, 7]).collect();
        tex_image(0, 4, 1, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        let s = storage(&gl, name);
        /* (10+21)/2 = 15.5 -> 16, (30+50)/2 = 40; then (16+40)/2 = 28 */
        assert_eq!(s[16..24], [16, 0, 255, 7, 40, 0, 255, 7]);
        assert_eq!(s[24..28], [28, 0, 255, 7]);
        assert_eq!(gl.ctx().objs().textures[name as usize].levels, 3);
    }

    #[test]
    fn generate_mipmap_fills_every_level_of_a_32x64_chain_one_wide_tail_included() {
        /* dEQP's texture.mipmap.2d.generate.*_non_square: a 32x64 chain ends
         * 1x2, 1x1. glGenerateMipmap fills levels with the CPU box filter
         * through the texture's CPU view at GlTexture::level_pitch, never
         * through the PBE, so the 1-wide levels' one-texel rows are written
         * where the TPU reads them (and the attachment rule for a 1-wide
         * multi-row level does not apply). Offsets and pitches here are
         * spelled out, not taken from the code under test. */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let (w0, h0) = (32usize, 64usize);
        let mut lvl: Vec<Vec<[u8; 4]>> = (0..h0)
            .map(|y| (0..w0).map(|x| [((x * 7 + y * 3) % 256) as u8, y as u8, x as u8, 255]).collect())
            .collect();
        let img: Vec<u8> = lvl.iter().flatten().flatten().copied().collect();
        tex_image(0, w0 as i32, h0 as i32, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.levels, t.chain_levels), (7, 7));
        let s = storage(&gl, name);
        let (mut w, mut h, mut off) = (w0, h0, 0usize);
        for l in 0..7 {
            /* one texel a row at every level: 32..2 are their own width and
             * the 1x2, 1x1 tail is one texel, not a pair */
            let pitch = w;
            for (y, row) in lvl.iter().enumerate() {
                for (x, px) in row.iter().enumerate() {
                    let o = off + (y * pitch + x) * 4;
                    assert_eq!(&s[o..o + 4], px, "level {l} ({w}x{h}) texel ({x},{y})");
                }
            }
            off += pitch * h * 4;
            /* the next level: 2x2 (or 1x2 once one texel wide) box, rounded half up */
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let (sx, sy) = (if w > 1 { 2 } else { 1 }, if h > 1 { 2 } else { 1 });
            lvl = (0..nh)
                .map(|y| {
                    (0..nw)
                        .map(|x| {
                            let mut px = [0u8; 4];
                            for (c, out) in px.iter_mut().enumerate() {
                                let mut sum = 0u32;
                                for j in 0..sy {
                                    for i in 0..sx {
                                        sum += lvl[y * sy + j][x * sx + i][c] as u32;
                                    }
                                }
                                let n = (sx * sy) as u32;
                                *out = ((sum + n / 2) / n) as u8;
                            }
                            px
                        })
                        .collect()
                })
                .collect();
            (w, h) = (nw, nh);
        }
    }

    #[test]
    fn generate_mipmap_keeps_level_0() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        let s = storage(&gl, name);
        assert_eq!(s[..16], ramp(4)[..]);
        /* (1+5+9+13)/4 = 7 ... */
        assert_eq!(s[16..20], [7, 8, 9, 10]);
    }

    #[test]
    fn generate_mipmap_of_a_one_column_texture_averages_rows() {
        /* uploads place the rows of a 1-wide level tex_pitch(1) = 2 texels
         * apart (tex_image_width_1_rows_are_a_texel_pair_apart); the filter
         * must read them there */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let img: Vec<u8> = [10u8, 30, 50, 70].iter().flat_map(|v| [*v, *v, *v, 255]).collect();
        tex_image(0, 1, 4, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        let off = gl.ctx().objs().textures[name as usize].level_off[1] as usize;
        assert_eq!(storage(&gl, name)[off..off + 4], [20, 20, 20, 255]);
    }

    #[test]
    fn generated_and_uploaded_one_column_levels_share_a_layout() {
        let gl = TestGl::new(4, 4);
        /* 2x4 of constant rows 10, 30, 50, 70: level 1 (1x2) is 20, 60; level 2 is 40 */
        let img: Vec<u8> = [10u8, 30, 50, 70].iter().flat_map(|v| [*v; 8]).collect();
        let a = new_tex();
        tex_image(0, 2, 4, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        let b = new_tex();
        tex_image(0, 2, 4, GL_RGBA, GL_UNSIGNED_BYTE, &img);
        tex_image(1, 1, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[20, 20, 20, 20, 60, 60, 60, 60]);
        tex_image(2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[40, 40, 40, 40]);
        assert_eq!(storage(&gl, a), storage(&gl, b));
    }

    /// The PowerVR's NPOT chain on this PowerVR-shaped mock: glGenerateMipmap
    /// of a 6x5 image lays its chain out as the 8x8 chain (`pot_chain_layout`:
    /// levels 8x8, 4x4, 2x2 back to back, pitches 8, 4, 2), keeps level 0's
    /// texels at the 8-texel pitch and box-filters 3x2 and 1x1 into the top
    /// left of levels 1 and 2. In both versions: the ES 2.0 surface claims
    /// GL_OES_texture_npot. Until 2026-10-08 the store kept level 0 alone
    /// and GL's chain was only recorded (glmark2 terrain aliased on the
    /// AM62P).
    ///
    /// Revert check: `npot_mip_chains` without its PowerVR arm leaves
    /// `chain_levels` 1 here.
    #[test]
    fn generate_mipmap_lays_the_powervr_npot_chain_out_as_its_pot_extent() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let base: Vec<u8> = (0..30u8).flat_map(|i| [i * 8, 240 - i * 8, i, 255 - i]).collect();
        tex_image(0, 6, 5, GL_RGBA, GL_UNSIGNED_BYTE, &base);
        assert_eq!(gl.ctx().objs().textures[name as usize].chain_levels, 1, "one level until asked");
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        assert!(t.pot_chain);
        assert_eq!((t.chain_levels, t.levels, t.levels_written, t.chain_pitch), (3, 3, 3, 0));
        assert_eq!((t.level_defined, t.level_bad), (0b111, 0), "6x5, 3x2, 1x1");
        assert_eq!(t.level_off[..3], [0, 8 * 8 * 4, 8 * 8 * 4 + 4 * 4 * 4]);
        assert_eq!([t.level_pitch_at(0), t.level_pitch_at(1), t.level_pitch_at(2)], [8, 4, 2]);
        assert_eq!((t.level_pitch(6), t.level_pitch(3), t.level_pitch(1)), (8, 4, 2), "by GL's width");
        assert!(t.mem.size >= (8 * 8 + 4 * 4 + 2 * 2 + 1) * 4, "the whole 8x8 chain");
        let s = storage(&gl, name);
        for y in 0..5 {
            assert_eq!(s[y * 32..y * 32 + 24], base[y * 24..y * 24 + 24], "level 0 row {y} kept");
        }
        /* level 1 texel (x, y): the 2x2 box at (2x, 2y) of level 0, rounded
         * half up (generate_mipmap_box_filters_with_round_half_up) */
        let px = |x: usize, y: usize| &base[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4];
        let l1 = t.level_off[1] as usize;
        let mut lv1 = [[0u8; 4]; 6];
        for y in 0..2 {
            for x in 0..3 {
                for c in 0..4 {
                    let quad = [px(2 * x, 2 * y), px(2 * x + 1, 2 * y), px(2 * x, 2 * y + 1), px(2 * x + 1, 2 * y + 1)];
                    let sum: u32 = quad.iter().map(|p| p[c] as u32).sum();
                    lv1[y * 3 + x][c] = ((sum + 2) / 4) as u8;
                    assert_eq!(s[l1 + y * 16 + x * 4 + c], lv1[y * 3 + x][c], "level 1 ({x}, {y}) channel {c}");
                }
            }
        }
        /* level 2 (1x1): the 2x2 box at (0, 0) of level 1 */
        let l2 = t.level_off[2] as usize;
        for c in 0..4 {
            let sum: u32 = [lv1[0], lv1[1], lv1[3], lv1[4]].iter().map(|p| p[c] as u32).sum();
            assert_eq!(s[l2 + c] as u32, (sum + 2) / 4, "level 2 channel {c}");
        }
    }

    /// glmark2 terrain's 800x600 RGBA8 chain, laid out by glGenerateMipmap
    /// on the PowerVR-shaped mock, is Mesa's: its level offsets and pitches
    /// equal [`mesa_pvr_chain`] level for level, and the storage holds the
    /// whole 1024x1024 chain.
    #[test]
    fn the_powervr_800x600_chain_has_mesas_offsets() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 800, 600, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        let (off, pitch, size) = mesa_pvr_chain(800, 600, 4);
        assert_eq!((t.chain_levels, t.pot_chain), (10, true));
        for l in 0..10 {
            assert_eq!(t.level_off[l], off[l], "level {l} offset");
            assert_eq!(t.level_pitch_at(l as u32), pitch[l], "level {l} pitch");
        }
        assert!(t.mem.size >= size as usize);
    }

    /// A NPOT cube map keeps its base level on the PowerVR's shape: the
    /// emulated cube samples level l as one tall image (6h) >> l rows high,
    /// which is not six faces of h >> l rows once h is not a power of two
    /// (`npot_mip_chains`). GL's chain is recorded, the texture complete.
    #[test]
    fn a_powervr_npot_cube_keeps_its_base_level() {
        let gl = TestGl::new(4, 4);
        let mut name: GLuint = 0;
        let face = ramp(36);
        // SAFETY: room for one name; plain GL calls with a 6x6 image per face.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_CUBE_MAP, name);
            for f in 0..6 {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    6,
                    6,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    ptr(&face),
                );
            }
            glGenerateMipmap(GL_TEXTURE_CUBE_MAP);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        assert_eq!((t.chain_levels, t.pot_chain), (1, false));
        assert_eq!((t.level_defined, t.level_bad), (0b111, 0), "GL's chain recorded");
    }

    /// The level sizes of glmark2 terrain's 800x600 chain (GL_OES_texture_npot:
    /// floor at every step, ten levels) and where each backend's layout puts
    /// them. The GC7000's (`mip_level_align` 64): one 800-texel pitch, every
    /// level 64-aligned, which 3200-byte rows already are. The V3D's
    /// (packed, `native_layered_textures`): each level at its own pair-padded
    /// pitch, a 1-wide level one texel a row. The PowerVR's (`pot_stack`):
    /// Mesa pvr's layout of the 1024x1024 physical extent ([`mesa_pvr_chain`],
    /// written from Mesa's rules and not from `chain_layout_npot`), for
    /// 800x600 and the odd shapes 100x60 and 3x5 too.
    /// Mesa 26.2.1 pvr's layout of a `w x h` image with a full mip chain, cpp
    /// `bpp`, written from Mesa's rules (src/imagination/vulkan/pvr_image.c,
    /// not vendored: docs/notes/pvr-render-pool.md records
    /// them):
    /// `pvr_image_plane_init_physical_extent` rounds the extent of a
    /// multi-level image up to `util_next_power_of_two` per axis;
    /// `pvr_image_plane_setup_mip_levels` gives level l `pitch = cpp *
    /// u_minify(pw, l)` and `height_pitch = u_minify(ph, l)`, `offset =
    /// layer_size` (then `layer_size += size`), and adds the levels to 1x1 to
    /// the layer ("the hw calculates layer strides as if a full mip chain up
    /// until 1x1x1 were present"). Returns (offset of each level, its pitch
    /// in texels, the layer size).
    fn mesa_pvr_chain(w: u32, h: u32, bpp: u32) -> (Vec<u32>, Vec<u32>, u32) {
        let u_minify = |v: u32, l: u32| (v >> l).max(1);
        let (pw, ph) = (w.next_power_of_two(), h.next_power_of_two());
        let (mut off, mut pitch, mut layer_size) = (Vec::new(), Vec::new(), 0u32);
        let mut l = 0;
        loop {
            let (lw, lh) = (u_minify(pw, l), u_minify(ph, l));
            off.push(layer_size);
            pitch.push(lw);
            layer_size += bpp * lw * lh;
            if lw == 1 && lh == 1 {
                return (off, pitch, layer_size);
            }
            l += 1;
        }
    }

    #[test]
    fn an_800x600_npot_chain_is_laid_out_per_backend() {
        let sizes =
            [(800, 600), (400, 300), (200, 150), (100, 75), (50, 37), (25, 18), (12, 9), (6, 4), (3, 2), (1, 1)];
        assert_eq!(context::gl_mip_levels(800, 600), 10);
        /* GC7000 */
        let (off, levels, bytes, pitch) = chain_layout_npot(800, 600, 1, 4, false, 64, false, true).unwrap();
        assert_eq!((levels, pitch), (10, 800));
        let mut rows = 0;
        for (i, &(_, h)) in sizes.iter().enumerate() {
            assert_eq!(off[i], 3200 * rows, "GC7000 level {i}");
            rows += h;
        }
        assert_eq!(bytes, 3200 * rows);
        assert_eq!(rows, 1196);
        /* V3D */
        let (off, levels, bytes, pitch) = chain_layout_npot(800, 600, 1, 4, false, 0, false, true).unwrap();
        assert_eq!((levels, pitch), (10, 0));
        let mut at = 0;
        for (i, &(w, h)) in sizes.iter().enumerate() {
            assert_eq!(off[i], at, "V3D level {i}");
            let p = if w == 1 { 1 } else { tex_pitch(w) };
            at += p * h * 4;
        }
        assert_eq!(bytes, at);
        assert_eq!(
            at,
            4 * (800 * 600 + 400 * 300 + 200 * 150 + 100 * 75 + 50 * 37 + 26 * 18 + 12 * 9 + 6 * 4 + 4 * 2 + 1)
        );
        /* PowerVR: Mesa's offsets and sizes, GL's level count */
        for (w, h, levels) in [(800, 600, 10u8), (100, 60, 7), (3, 5, 3)] {
            let (off, lv, bytes, pitch) = chain_layout_npot(w, h, 1, 4, false, 0, true, true).unwrap();
            let (moff, mpitch, msize) = mesa_pvr_chain(w, h, 4);
            assert_eq!((lv, pitch), (levels, 0), "PowerVR {w}x{h}: levels, no chain pitch");
            assert_eq!(lv, context::gl_mip_levels(w, h));
            assert_eq!(off[..lv as usize], moff[..lv as usize], "PowerVR {w}x{h}: offsets");
            assert_eq!(bytes, msize, "PowerVR {w}x{h}: the layer, as if the chain went to 1x1");
            for l in 0..lv as u32 {
                let p = context::pot_chain_level_pitch(w, l);
                assert_eq!(p, mpitch[l as usize], "PowerVR {w}x{h} level {l} pitch");
                let (gw, gh) = ((w >> l).max(1), (h >> l).max(1));
                let (pw, ph) = ((w.next_power_of_two() >> l).max(1), (h.next_power_of_two() >> l).max(1));
                assert!(gw <= pw && gh <= ph, "GL's level {l} in the top left of the physical one");
            }
        }
        assert_eq!(mesa_pvr_chain(800, 600, 4).2, 5_592_404, "4 * (1024^2 + 512^2 + ... + 1)");
        /* one level without npot_chain, or for a stacked cube (6 layers) */
        assert_eq!(chain_layout_npot(800, 600, 1, 4, false, 0, true, false).unwrap().1, 1);
        assert_eq!(chain_layout_npot(6, 6, 6, 4, false, 0, true, true).unwrap().1, 1);
        assert_eq!(chain_layout(800, 600, 1, 4, false, 64, false).unwrap().1, 1);
        assert_eq!(chain_layout(800, 600, 1, 4, false, 0, false).unwrap().1, 1);
        /* a power-of-two size is the chain it always was */
        assert_eq!(chain_layout_npot(64, 32, 1, 4, false, 0, false, true), chain_layout(64, 32, 1, 4, false, 0, false));
    }

    /// A heap that refuses the chain: glGenerateMipmap of a 5x3 image is
    /// GL_OUT_OF_MEMORY and the texture keeps level 0 (its storage, texels
    /// and record), so it still samples and stays framebuffer-complete.
    #[test]
    fn generate_mipmap_refused_the_chain_keeps_level_0() {
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = new_tex();
        let base: Vec<u8> = (0..15u8).flat_map(|i| [i * 10, 200 - i * 10, 7, 255]).collect();
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &base);
        let before = storage(&gl, name);
        let (gpu, defined) = {
            let t = &gl.ctx().objs_ref().textures[name as usize];
            (t.mem.gpu, t.level_defined)
        };
        let mut fb = 0u32;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
        }
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
        let allocs = gl.live_allocs();
        gl.fail_alloc_in(1);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), allocs, "nothing allocated, nothing freed");
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(!t.mem.impl_.is_null() && t.mem.gpu == gpu, "level 0's storage stays");
        assert_eq!((t.width, t.height, t.chain_levels, t.level_defined), (5, 3, 1, defined));
        assert_eq!(storage(&gl, name), before, "level 0's texels stay");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
    }

    /// A level above 0 uploaded before level 0 that lays the chain out again
    /// (relayout_keeping_levels) tells its two failures apart: a refused
    /// allocation leaves the texture and the level already uploaded as they
    /// were; new storage that could not be cleared (no CPU view, a failed
    /// gpu_write) holds no level, as it may hold another object's bytes.
    #[test]
    fn a_relayout_keeps_its_levels_when_refused_and_drops_them_when_not_cleared() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        /* level 1 of 2x2: a 4x4 chain */
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        let (gpu, defined) = {
            let t = &gl.ctx().objs_ref().textures[name as usize];
            (t.mem.gpu, t.level_defined)
        };
        assert_eq!(defined, 0b10);
        let before = storage(&gl, name);
        gl.fail_alloc_in(1);
        /* level 1 of 4x2: an 8x4 chain, refused */
        tex_image(1, 4, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(8));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.mem.gpu, t.level_defined, t.width, t.height), (gpu, defined, 4, 4), "the texture as it was");
        assert_eq!(storage(&gl, name), before, "level 1's texels stay");

        gl.set_cpu_mapping(false);
        let unmapped = new_tex();
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let gpu = gl.ctx().objs_ref().textures[unmapped as usize].mem.gpu;
        gl.fail_gpu_write_in(1);
        tex_image(1, 4, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(8));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        let t = &gl.ctx().objs_ref().textures[unmapped as usize];
        assert!(!t.mem.impl_.is_null() && t.mem.gpu != gpu, "the new storage is in place");
        assert_eq!((t.width, t.height, t.level_defined), (8, 4, 0), "no level over uncleared storage");
    }

    /// On a backend that holds a NPOT chain (the GC7000's shape here; the
    /// V3D's in v3d/backend_tests.rs): glGenerateMipmap of a 5x3 image lays
    /// its chain out (5x3, 2x1, 1x1), keeps level 0's texels and
    /// box-filters the rest from it; the texture's state survives.
    #[test]
    fn generate_mipmap_lays_a_npot_chain_out_where_the_backend_holds_one() {
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = new_tex();
        let base: Vec<u8> = (0..15u8).flat_map(|i| [i * 10, 200 - i * 10, 7, 255]).collect();
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &base);
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_MIRRORED_REPEAT as GLint);
        }
        assert_eq!(gl.ctx().objs().textures[name as usize].chain_levels, 1, "one level until asked");
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        assert_eq!((t.chain_levels, t.levels, t.levels_written), (3, 3, 3));
        assert_eq!((t.level_defined, t.level_bad, t.max_level), (0b111, 0, MAX_LEVEL_DEFAULT));
        assert_eq!((t.min_filter, t.wrap_s), (GL_NEAREST_MIPMAP_NEAREST, GL_MIRRORED_REPEAT));
        let pitch = t.level_pitch(5) as usize * 4;
        assert_eq!(pitch, 24, "the GC7000's one chain pitch, pair-padded");
        let (l1, l2) = (t.level_off[1] as usize, t.level_off[2] as usize);
        assert_eq!((l1 % 64, l2 % 64), (0, 0));
        let s = storage(&gl, name);
        for y in 0..3 {
            assert_eq!(s[y * pitch..y * pitch + 20], base[y * 20..y * 20 + 20], "level 0 row {y} kept");
        }
        /* level 1 texel (x, 0): the 2x2 box at (2x, 0) of level 0 */
        let px = |x: usize, y: usize| &base[(y * 5 + x) * 4..(y * 5 + x) * 4 + 4];
        for x in 0..2 {
            for c in 0..4 {
                let sum: u32 =
                    [px(2 * x, 0), px(2 * x + 1, 0), px(2 * x, 1), px(2 * x + 1, 1)].iter().map(|p| p[c] as u32).sum();
                assert_eq!(s[l1 + x * 4 + c] as u32, (sum + 2) / 4, "level 1 ({x}, 0) channel {c}");
            }
        }
        assert_eq!(s[l2 + 3], 255, "level 2's alpha");
    }

    /// The application's own levels of a NPOT image (GL_OES_texture_npot):
    /// on a backend that holds the chain, level 1 at GL's size lays it out
    /// and its texels land there; on the PowerVR's shape it is recorded and
    /// not held (`tex_image_level_on_npot_texture_follows_the_version`).
    #[test]
    fn a_npot_level_above_0_grows_the_chain_where_the_backend_holds_one() {
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = new_tex();
        tex_image(0, 6, 6, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(36));
        tex_image(1, 3, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(9));
        tex_image(2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs().textures[name as usize];
        assert_eq!((t.chain_levels, t.levels, t.level_defined, t.level_bad), (3, 3, 0b111, 0));
        let (pitch, l1, l2) = (t.level_pitch(3) as usize * 4, t.level_off[1] as usize, t.level_off[2] as usize);
        let s = storage(&gl, name);
        assert_eq!(s[..24], ramp(36)[..24], "level 0 kept across the new layout");
        for y in 0..3 {
            assert_eq!(s[l1 + y * pitch..l1 + y * pitch + 12], ramp(9)[y * 12..y * 12 + 12], "level 1 row {y}");
        }
        assert_eq!(s[l2..l2 + 4], [1, 2, 3, 4]);
    }

    #[test]
    fn generate_mipmap_errors() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no texture bound");
            new_tex();
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no storage");
            glGenerateMipmap(BAD_TARGET);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGenerateMipmap(GL_TEXTURE_3D);
            /* not an ES 2.0 target (`es3_only_texture_target`) */
            let want = if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_INVALID_OPERATION };
            assert_eq!(gl.error(), want, "no 3D texture bound");
            glGenerateMipmap(GL_TEXTURE_CUBE_MAP);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no cube map bound");
        }
    }

    /* ---- parameters ---------------------------------------------------------------------------------- */

    fn get_param(pname: GLenum) -> GLint {
        let mut v: GLint = -1;
        // SAFETY: room for one value.
        unsafe { glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut v) };
        v
    }

    #[test]
    fn get_tex_parameter_reports_the_defaults() {
        let gl = TestGl::new(4, 4);
        new_tex();
        assert_eq!(get_param(GL_TEXTURE_MIN_FILTER), GL_NEAREST_MIPMAP_LINEAR as GLint);
        assert_eq!(get_param(GL_TEXTURE_MAG_FILTER), GL_LINEAR as GLint);
        assert_eq!(get_param(GL_TEXTURE_WRAP_S), GL_REPEAT as GLint);
        assert_eq!(get_param(GL_TEXTURE_WRAP_T), GL_REPEAT as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn tex_parameteri_round_trips() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT as GLint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(get_param(GL_TEXTURE_MIN_FILTER), GL_LINEAR_MIPMAP_NEAREST as GLint);
        assert_eq!(get_param(GL_TEXTURE_MAG_FILTER), GL_NEAREST as GLint);
        assert_eq!(get_param(GL_TEXTURE_WRAP_S), GL_CLAMP_TO_EDGE as GLint);
        assert_eq!(get_param(GL_TEXTURE_WRAP_T), GL_MIRRORED_REPEAT as GLint);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.min_filter, t.mag_filter), (GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST));
    }

    #[test]
    fn tex_parameter_f_iv_fv_variants_set_the_same_state() {
        let gl = TestGl::new(4, 4);
        new_tex();
        // SAFETY: one value behind each pointer.
        unsafe {
            glTexParameterf(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLfloat);
            let wrap = GL_CLAMP_TO_EDGE as GLint;
            glTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, &wrap);
            let mag = GL_NEAREST as GLfloat;
            glTexParameterfv(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, &mag);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(get_param(GL_TEXTURE_MIN_FILTER), GL_NEAREST as GLint);
        assert_eq!(get_param(GL_TEXTURE_WRAP_S), GL_CLAMP_TO_EDGE as GLint);
        assert_eq!(get_param(GL_TEXTURE_MAG_FILTER), GL_NEAREST as GLint);
        let mut f: GLfloat = -1.0;
        unsafe { glGetTexParameterfv(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, &mut f) };
        assert_eq!(f, GL_CLAMP_TO_EDGE as GLfloat);
    }

    #[test]
    fn tex_parameters_belong_to_the_bound_object() {
        let gl = TestGl::new(4, 4);
        let a = new_tex();
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint) };
        let _b = new_tex();
        assert_eq!(get_param(GL_TEXTURE_MAG_FILTER), GL_LINEAR as GLint);
        unsafe { glBindTexture(GL_TEXTURE_2D, a) };
        assert_eq!(get_param(GL_TEXTURE_MAG_FILTER), GL_NEAREST as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_parameter_es3_names_are_accepted_and_ignored() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls.
        unsafe {
            for p in [
                GL_TEXTURE_WRAP_R,
                GL_TEXTURE_MIN_LOD,
                GL_TEXTURE_MAX_LOD,
                GL_TEXTURE_BASE_LEVEL,
                GL_TEXTURE_MAX_LEVEL,
                GL_TEXTURE_COMPARE_MODE,
                GL_TEXTURE_COMPARE_FUNC,
                GL_TEXTURE_SWIZZLE_R,
                GL_TEXTURE_SWIZZLE_G,
                GL_TEXTURE_SWIZZLE_B,
                GL_TEXTURE_SWIZZLE_A,
            ] {
                /* a value the parameter takes: WRAP_R, COMPARE_MODE and
                 * COMPARE_FUNC are validated since 2026-09-24 (this set every
                 * one to 1 before) */
                let v = match p {
                    GL_TEXTURE_WRAP_R => GL_CLAMP_TO_EDGE as GLint,
                    GL_TEXTURE_COMPARE_MODE => GL_NONE as GLint,
                    GL_TEXTURE_COMPARE_FUNC => GL_LEQUAL as GLint,
                    _ => 1,
                };
                glTexParameteri(GL_TEXTURE_2D, p, v);
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a wrap mode");
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a compare function");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(
            (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t),
            (GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR, GL_REPEAT, GL_REPEAT)
        );
    }

    #[test]
    fn tex_parameter_unknown_pname_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        new_tex();
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_2D, 0) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn tex_parameter_bad_target_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(BAD_TARGET, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(gl.ctx().objs().textures[name as usize].mag_filter, GL_LINEAR);
    }

    #[test]
    fn tex_parameter_without_a_binding_addresses_the_default_texture_object() {
        /* This is what a conformance run does between test cases to put the
         * state back; every such call used to answer GL_INVALID_OPERATION,
         * so the run ended there (ES 3.0 3.8). */
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(get_param(GL_TEXTURE_MAG_FILTER), GL_NEAREST as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* GL's initial values are the rest of the default object's state */
        assert_eq!(get_param(GL_TEXTURE_MIN_FILTER), GL_NEAREST_MIPMAP_LINEAR as GLint);
        assert_eq!(get_param(GL_TEXTURE_WRAP_S), GL_REPEAT as GLint);
        /* each target has its own, and a cube one is six faces */
        let (d2d, dcube) =
            (&gl.ctx().objs_ref().textures[MAX_TEXTURES], &gl.ctx().objs_ref().textures[MAX_TEXTURES + 1]);
        assert_eq!(d2d.mag_filter, GL_NEAREST);
        assert!(!dcube.used, "an untouched target has no object yet");
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_CUBE_MAP_NEGATIVE_X, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint) };
        let dcube = &gl.ctx().objs_ref().textures[MAX_TEXTURES + 1];
        assert_eq!((dcube.used, dcube.target, dcube.layers), (true, GL_TEXTURE_CUBE_MAP, 6));
        assert_eq!(gl.ctx().objs().textures[MAX_TEXTURES].mag_filter, GL_NEAREST, "the 2D one is untouched");
    }

    #[test]
    fn tex_parameter_null_vectors_are_ignored() {
        let gl = TestGl::new(4, 4);
        new_tex();
        // SAFETY: null pointers are tolerated.
        unsafe {
            glTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, core::ptr::null());
            glTexParameterfv(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, core::ptr::null());
            glGetTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, core::ptr::null_mut());
            glGetTexParameterfv(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, core::ptr::null_mut());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_tex_parameter_unknown_pname_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        new_tex();
        assert_eq!(get_param(GL_TEXTURE_2D), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: room for one value.
        unsafe {
            let mut v: GLint = -1;
            glGetTexParameteriv(BAD_TARGET, GL_TEXTURE_WRAP_S, &mut v);
            assert_eq!(v, -1);
        }
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn get_tex_parameterfv_writes_nothing_on_an_error() {
        /* ES 2.0.25 2.5: "no change is made to these values" (0 was written
         * until issue #128) */
        let gl = TestGl::new(4, 4);
        new_tex();
        let mut f: GLfloat = -1.0;
        // SAFETY: room for one value.
        unsafe { glGetTexParameterfv(GL_TEXTURE_2D, GL_TEXTURE_2D, &mut f) };
        assert_eq!(f, -1.0);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn tex_parameter_invalid_value_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_REPEAT as GLint);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            /* the mipmap filters are minification-only */
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_LINEAR as GLint);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(
            (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t),
            (GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR, GL_REPEAT, GL_REPEAT)
        );
    }

    /* ---- texture_attach_external ------------------------------------------------------------------------ */

    #[test]
    fn attach_external_rgba_image() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 8 * 4 * 4];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 8, 4, backend::GLES_SURF_RGBA8888), 0);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(t.external && !t.bgra);
        assert_eq!((t.mem.gpu, t.mem.size), (mem.gpu, mem.size));
        assert_eq!((t.width, t.height, t.format, t.type_), (8, 4, GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(t.texfmt, GLES_TEXFMT_RGBA8);
        /* a single 2D buffer: no chain even though 8x4 is a power of two */
        assert_eq!((t.chain_levels, t.levels, t.level_off[0]), (1, 1, 0));
        assert_eq!(gl.mem_events(), []);
    }

    #[test]
    fn attach_external_xrgb_image_is_flagged_bgra() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, GLES_SURF_XRGB8888), 0);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(t.bgra);
        assert_eq!((t.texfmt, t.format, t.type_), (GLES_TEXFMT_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE));
    }

    #[test]
    fn attach_external_rgb565_image() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 32];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, GLES_SURF_RGB565), 0);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert!(t.external && !t.bgra);
        assert_eq!((t.texfmt, t.format, t.type_), (GLES_TEXFMT_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5));
    }

    #[test]
    fn attach_external_frees_owned_storage_but_not_a_previous_image() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let owned = gl.ctx().objs().textures[name as usize].mem.gpu;
        gl.mem_events();
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu: owned }]);
        let mut backing2 = vec![0u8; 64];
        let mem2 = external_mem_at(&mut backing2, 0x5678_0000);
        assert_eq!(texture_attach_external(gl.ctx(), &mem2, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        assert_eq!(gl.mem_events(), []);
        assert_eq!(gl.ctx().objs().textures[name as usize].mem.gpu, 0x5678_0000);
    }

    #[test]
    fn attach_external_over_a_compressed_texture_clears_the_block_layout() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.texfmt, t.block_w, t.block_h, t.block_bytes), (GLES_TEXFMT_RGBA8, 0, 0, 0));
    }

    #[test]
    fn attach_external_fails_without_texture_or_with_empty_size() {
        let gl = TestGl::new(4, 4);
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        /* nothing bound is the default texture object, which takes an image
         * like any other (ES 3.0 3.8); an empty size is still refused */
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        assert!(gl.ctx().objs().textures[MAX_TEXTURES].external);
        let name = new_tex();
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 0, 4, backend::GLES_SURF_RGBA8888), -1);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 0, backend::GLES_SURF_RGBA8888), -1);
        assert!(!gl.ctx().objs().textures[name as usize].external);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attach_external_c_entry_point() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 64];
        let mem = external_mem(&mut backing);
        // SAFETY: null or a valid descriptor.
        unsafe {
            assert_eq!(gl_texture_attach_external(core::ptr::null(), 4, 4, 0, core::ptr::null_mut()), -1);
            assert_eq!(gl_texture_attach_external(&mem, 4, 4, backend::GLES_SURF_RGBA8888, core::ptr::null_mut()), 0);
        }
        assert!(gl.ctx().objs().textures[name as usize].external);
    }

    #[test]
    fn generate_mipmap_on_an_external_image_is_a_no_op() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0x11u8; 64];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 4, backend::GLES_SURF_RGBA8888), 0);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().textures[name as usize].levels, 1);
        assert_eq!(backing, vec![0x11u8; 64]);
    }

    /* ---- glTexStorage2D ------------------------------------------------------------------------------------ */

    #[test]
    fn tex_storage_allocates_the_whole_chain_with_every_level_defined() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 3, GL_RGBA8, 4, 4) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.height, t.levels, t.chain_levels), (4, 4, 3, 3));
        assert_eq!((t.format, t.type_), (GL_RGBA, GL_UNSIGNED_BYTE));
        assert_eq!(t.level_off[..3], [0, 64, 80]);
        /* so a sub-image upload into a level needs no glTexImage2D first */
        tex_sub_image(2, 0, 0, 1, 1, GL_RGBA, &[5, 6, 7, 8]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[80..84], [5, 6, 7, 8]);
    }

    #[test]
    fn tex_storage_fewer_levels_than_the_chain() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 8, 8) };
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.levels, t.chain_levels), (1, 4));
        tex_sub_image(1, 0, 0, 1, 1, GL_RGBA, &[5, 6, 7, 8]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn tex_storage_npot_holds_the_chain_where_the_backend_does() {
        /* ES 3.0 3.8.4: every level of glTexStorage2D exists. A 2D NPOT
         * texture gets room for its chain (the PowerVR's as its power-of-two
         * extent's); es3_level_tests covers the arrays and cubes a backend
         * keeps one level of. */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 3, GL_RGBA8, 6, 5) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.levels, t.chain_levels), (3, 3));
        assert!(t.mem.size > 6 * 5 * 4);
    }

    #[test]
    fn tex_storage_sized_formats_map_to_their_unpack_pair() {
        let gl = TestGl::new(4, 4);
        let cases = [
            (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE),
            (GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE),
            (GL_RGBA4, GL_RGBA, GL_UNSIGNED_BYTE),
            (GL_RGB5_A1, GL_RGBA, GL_UNSIGNED_BYTE),
            (GL_RGB8, GL_RGB, GL_UNSIGNED_BYTE),
            (GL_RGB565, GL_RGB, GL_UNSIGNED_BYTE),
            (GL_SRGB8, GL_RGB, GL_UNSIGNED_BYTE),
            (GL_R8, GL_RED, GL_UNSIGNED_BYTE),
            (GL_RG8, GL_RG, GL_UNSIGNED_BYTE),
            (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT),
            (GL_RGBA32F, GL_RGBA, GL_FLOAT),
            (GL_R16F, GL_RED, GL_HALF_FLOAT),
            (GL_R32F, GL_RED, GL_FLOAT),
            (GL_RG16F, GL_RG, GL_HALF_FLOAT),
            (GL_ALPHA, GL_ALPHA, GL_UNSIGNED_BYTE),
            (GL_LUMINANCE, GL_LUMINANCE, GL_UNSIGNED_BYTE),
            (GL_LUMINANCE_ALPHA, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE),
        ];
        for (internal, format, type_) in cases {
            /* one texture per case: glTexStorage2D storage is immutable */
            let name = new_tex();
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, internal, 2, 2) };
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{internal:x}");
            let t = &gl.ctx().objs_ref().textures[name as usize];
            assert_eq!((t.format, t.type_), (format, type_), "0x{internal:x}");
            /* the store is RGBA8 for all but the float formats (RGBA32F and
             * RG16F are kept as four halves) and the sRGB pair (the same
             * bytes, sampled decoded; RGBA8 under the ES 2.0 report) */
            let (texfmt, bpp) = match internal {
                GL_RGBA16F | GL_RGBA32F | GL_RG16F => (GLES_TEXFMT_RGBA16F, 8),
                GL_R16F => (GLES_TEXFMT_R16F, 2),
                GL_R32F => (GLES_TEXFMT_R32F, 4),
                GL_SRGB8_ALPHA8 | GL_SRGB8 if !crate::REPORT_ES2 => (GLES_TEXFMT_SRGB8_A8, 4),
                _ => (GLES_TEXFMT_RGBA8, 4),
            };
            assert_eq!((t.texfmt, t.bpp), (texfmt, bpp), "0x{internal:x}");
            /* 2x2 then the 1x1 level at its pair-padded pitch of 2 */
            assert_eq!(t.level_off[1], 2 * 2 * bpp as u32, "0x{internal:x}");
        }
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_storage_rgba16f_then_half_float_sub_image() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: four half floats = one RGBA texel.
        unsafe {
            glTexStorage2D(GL_TEXTURE_2D, 2, GL_RGBA16F, 2, 2);
            let px = [0x3C00u16, 0x3800, 0x0000, 0x3C00];
            glTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, 1, 1, GL_RGBA, GL_HALF_FLOAT, ptr(&px));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* half-float texels: level 1 follows the 2x2 base level of 8-byte texels */
        assert_eq!(gl.ctx().objs().textures[name as usize].level_off[1], 32);
        assert_eq!(storage(&gl, name)[32..40], [0x00, 0x3C, 0x00, 0x38, 0x00, 0x00, 0x00, 0x3C]);
    }

    #[test]
    fn tex_storage_r8_then_red_sub_image() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_R8, 2, 1);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        }
        tex_sub_image(0, 0, 0, 2, 1, GL_RED, &[7, 9]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..8], [7, 0, 0, 255, 9, 0, 0, 255]);
    }

    #[test]
    fn tex_storage_errors() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            /* Nothing bound is the default texture object (ES 3.0 3.8), and
             * 3.8.4 refuses immutable storage for it. This pinned the
             * opposite (accepted, and the default texture made immutable)
             * until 2026-09-24, which ended dEQP-GLES3 runs at
             * negative_api.texture.texstorage2d_invalid_binding.
             * Revert check: drop the name-0 check in glTexStorage2D/3D. */
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the default texture");
            assert!(!gl.ctx().objs().textures[MAX_TEXTURES].immutable);
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 4, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the default 3D texture");
            let name = new_tex();
            glTexStorage2D(GL_TEXTURE_3D, 1, GL_RGBA8, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexStorage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "levels < 1");
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 0, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "width < 1");
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 4, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "height < 1");
            /* known quirk (upstream): the unsized RGBA/RGB are accepted as
             * storage formats (they share the sized-format table) */
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA, 4, 4);
            assert_eq!(gl.error(), GL_NO_ERROR, "unsized RGBA accepted");
            glDeleteTextures(1, &name);
            let name = new_tex();
            /* upstream 6a1c6df: a depth texture is a sized format, stored in
             * the same RGBA8-sized memory (a render writes its depth through
             * the ISP's own buffer) */
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_DEPTH_COMPONENT16, 4, 4);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(!gl.ctx().objs().textures[name as usize].mem.impl_.is_null());
            let name = new_tex();
            /* (GL_RGB10_A2 was the example until es3_formats took it) */
            glTexStorage2D(GL_TEXTURE_2D, 1, 0x1234, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "an internal format the table has not");
            assert!(gl.ctx().objs().textures[name as usize].mem.impl_.is_null());
        }
    }

    #[test]
    fn tex_storage_out_of_memory() {
        let gl = TestGl::new(4, 4);
        new_tex();
        gl.fail_alloc_in(1);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 4, 4) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), 1);
    }

    #[test]
    fn tex_storage_makes_the_texture_immutable() {
        /* ES 3.0 3.8.4: TEXTURE_IMMUTABLE_FORMAT is set; glTexStorage2D and glTexImage2D then fail */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 4, 4);
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 8, 8);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(gl.ctx().objs().textures[name as usize].width, 4);
    }

    /* ---- ES 3.0 entry points that are not available --------------------------------------------------------- */

    #[test]
    fn tex_3d_entry_points_reject_2d_targets() {
        let gl = TestGl::new(4, 4);
        // SAFETY: no pixels are read.
        unsafe {
            glTexImage3D(GL_TEXTURE_2D, 0, 0, 1, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexSubImage3D(GL_TEXTURE_2D, 0, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexStorage3D(GL_TEXTURE_2D, 1, GL_RGBA8, 1, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            /* a 2D target is GL_INVALID_ENUM (ES 3.0 3.8.6; this pinned
             * GL_INVALID_OPERATION until 2026-09-24) */
            glCompressedTexImage3D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 4, 4, 1, 0, 8, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
        assert_eq!(gl.live_allocs(), 1);
    }

    /// glCompressedTexImage3D on GL_TEXTURE_2D_ARRAY stores its layers as a
    /// cube map's faces are, back to back within each level, and
    /// glCompressedTexSubImage3D reaches a run of layers (dEQP-GLES3
    /// negative_api.texture.compressedtexsubimage3d*, which could not set
    /// up their texture until 2026-09-24). imageSize is checked for every
    /// ES 3.0 format, stored or not (compressedteximage2d_invalid_size).
    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn a_compressed_2d_array_holds_its_layers_level_by_level() {
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        let etc2 = GL_COMPRESSED_RGBA8_ETC2_EAC;
        /* three 8x8 layers of 2x2 blocks, block i of layer l = l * 4 + i + 1 */
        let data: Vec<u8> = (0..12u8).flat_map(|b| [b + 1; 16]).collect();
        // SAFETY: `data` holds the 3 x 4 blocks imageSize names.
        unsafe { glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, 0, etc2, 8, 8, 3, 0, 192, ptr(&data)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[arr as usize];
        assert_eq!((t.layers, t.block_bytes, t.chain_levels), (3, 16, 4));
        assert_eq!(storage(&gl, arr)[..192], data[..]);
        /* layer 2's block (1,1) replaced */
        // SAFETY: one 16-byte block.
        unsafe { glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 4, 4, 2, 4, 4, 1, etc2, 16, ptr(&[0x99u8; 16])) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, arr)[2 * 64 + 48..2 * 64 + 64], [0x99u8; 16]);
        // SAFETY: the failing calls read nothing.
        unsafe {
            glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 2, 4, 4, 2, etc2, 32, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "layers 2 and 3 of three");
            glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 0, 8, 8, 1, etc2, 63, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "imageSize");
            glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 1, 0, 0, 4, 4, 1, etc2, 16, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "an offset inside a block");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_SIGNED_R11_EAC, 16, 16, 0, 256, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "an EAC R11 16x16 image is 128 bytes");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_SIGNED_R11_EAC, 16, 16, 0, 128, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "right size, a format not stored");
        }
    }
    /* ---- cube maps, 2D arrays and float texels (upstream 7e76640) ---------------------------------- */

    fn bind_new(target: GLenum) -> GLuint {
        let mut name: GLuint = 0;
        // SAFETY: valid out pointer, plain GL call.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(target, name);
        }
        name
    }

    fn solid(n: usize, v: u8) -> Vec<u8> {
        vec![v; n * 4]
    }

    #[test]
    fn face_helpers_name_the_six_cube_faces() {
        assert!(is_face_target(GL_TEXTURE_CUBE_MAP_POSITIVE_X) && is_face_target(GL_TEXTURE_CUBE_MAP_NEGATIVE_Z));
        assert!(!is_face_target(GL_TEXTURE_CUBE_MAP) && !is_face_target(GL_TEXTURE_2D));
        assert_eq!(face_of(GL_TEXTURE_CUBE_MAP_POSITIVE_X), 0);
        assert_eq!(face_of(GL_TEXTURE_CUBE_MAP_NEGATIVE_Z), 5);
        assert_eq!(face_of(GL_TEXTURE_2D_ARRAY), 0);
    }

    #[test]
    fn float_to_half_encodes_ieee_halves() {
        assert_eq!(float_to_half(0.0), 0x0000);
        assert_eq!(float_to_half(-0.0), 0x8000);
        assert_eq!(float_to_half(1.0), 0x3C00);
        assert_eq!(float_to_half(-2.0), 0xC000);
        assert_eq!(float_to_half(0.5), 0x3800);
        assert_eq!(float_to_half(65504.0), 0x7BFF, "the largest half");
        assert_eq!(float_to_half(65520.0), 0x7BFF, "rounds toward zero (no round-to-nearest)");
        assert_eq!(float_to_half(65536.0), 0x7C00, "overflow is infinity");
        assert_eq!(float_to_half(f32::INFINITY), 0x7C00);
        assert_eq!(float_to_half(f32::NEG_INFINITY), 0xFC00);
        assert_eq!(float_to_half(f32::NAN) & 0x7E00, 0x7E00, "a quiet NaN stays a NaN");
        assert_eq!(float_to_half(1e-9), 0, "underflow is zero");
        /* the smallest normal half is 2^-14; below it come subnormals */
        assert_eq!(float_to_half(2f32.powi(-14)), 0x0400);
        assert_eq!(float_to_half(2f32.powi(-15)), 0x0200);
        assert_eq!(float_to_half(2f32.powi(-23)), 0x0002);
        assert_eq!(float_to_half(2f32.powi(-24)), 0x0001, "the smallest subnormal half");
    }

    #[test]
    fn half_float_decode_round_trips_normal_values() {
        for v in [0.0f32, 1.0, -1.0, 0.25, 1024.0, 65504.0] {
            assert_eq!(half_to_float(float_to_half(v)), v);
        }
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn bind_sets_the_target_and_layer_count_once() {
        let gl = TestGl::new(4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        {
            let c = gl.ctx();
            assert_eq!(
                (c.objs().textures[cube as usize].target, c.objs().textures[cube as usize].layers),
                (GL_TEXTURE_CUBE_MAP, 6)
            );
            assert_eq!(
                (c.objs().textures[arr as usize].target, c.objs().textures[arr as usize].layers),
                (GL_TEXTURE_2D_ARRAY, 1)
            );
            assert_eq!((c.bound_texture_cube[0], c.bound_texture_2d_array[0], c.bound_texture_2d[0]), (cube, arr, 0));
        }
        // SAFETY: plain GL calls.
        unsafe { glBindTexture(GL_TEXTURE_2D, cube) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "a cube map cannot become a 2D texture");
        assert_eq!(gl.ctx().bound_texture_2d[0], 0);
        /* the bindings are per unit */
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE3);
            glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
            glDeleteTextures(1, &cube);
        }
        let c = gl.ctx();
        assert_eq!((c.bound_texture_cube[0], c.bound_texture_cube[3]), (0, 0), "deleting unbinds every unit");
        assert_eq!(c.bound_texture_2d_array[0], arr);
    }

    #[test]
    fn cube_faces_are_stacked_rows_of_one_storage() {
        let gl = TestGl::new(4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        gl.mem_events();
        for f in 0..6u32 {
            let px = solid(4, 0x10 + f as u8);
            // SAFETY: `px` holds the 2x2 RGBA image.
            unsafe {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    2,
                    2,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    ptr(&px),
                );
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        let allocs = gl.mem_events().iter().filter(|e| matches!(e, crate::testutil::MemEvent::Alloc { .. })).count();
        assert_eq!(allocs, 1, "the first face lays out the stack, the others find it");
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.width, t.height, t.layers, t.texfmt), (2, 2, 6, GLES_TEXFMT_RGBA8));
        /* the chain is one face's: 2x2 then 1x1, six faces at each level */
        assert_eq!(t.chain_levels, 2);
        let s = storage(&gl, cube);
        for f in 0..6usize {
            assert_eq!(s[f * 2 * 2 * 4..(f + 1) * 2 * 2 * 4], solid(4, 0x10 + f as u8)[..], "face {f}");
        }
        /* a face upload of another size lays the stack out again */
        let px = solid(16, 1);
        // SAFETY: `px` holds the 4x4 image.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr(&px),
            )
        };
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.width, t.height), (4, 4));
        assert_eq!(storage(&gl, cube)[3 * 16 * 4..4 * 16 * 4], px[..], "face 3 at rows 12..16");
    }

    #[test]
    fn cube_face_sub_image_lands_in_its_face() {
        let gl = TestGl::new(4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL calls; null pixels allocate only.
        unsafe {
            glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 2, 2);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let px = [9u8, 8, 7, 6];
            glTexSubImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_Z, 0, 1, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* face 4, row 1, column 1: stacked row 4 * 2 + 1 */
        assert_eq!(storage(&gl, cube)[(9 * 2 + 1) * 4..(9 * 2 + 2) * 4], [9, 8, 7, 6]);
        assert_eq!(gl.ctx().objs().textures[cube as usize].layers, 6);
    }

    /* ---- 3D textures: slices tiled in a grid (upstream c1e6084) ------------------------------------- */

    #[cfg(not(feature = "report-es2"))]
    fn binding_3d() -> GLint {
        let mut v: GLint = -1;
        // SAFETY: room for one value.
        unsafe { crate::gl::state::glGetIntegerv(GL_TEXTURE_BINDING_3D, &mut v) };
        v
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn a_3d_texture_has_its_own_binding() {
        let gl = TestGl::new(4, 4);
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        let vol = bind_new(GL_TEXTURE_3D);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let c = gl.ctx();
        assert_eq!((c.bound_texture_2d_array[0], c.bound_texture_3d[0]), (arr, vol));
        assert_eq!(
            (c.objs().textures[vol as usize].target, c.objs().textures[vol as usize].layers),
            (GL_TEXTURE_3D, 1)
        );
        assert_eq!(binding_3d(), vol as GLint);
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D_ARRAY, vol);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a 3D texture is not an array");
            glActiveTexture(GL_TEXTURE3);
            assert_eq!(binding_3d(), 0, "per unit");
            glActiveTexture(GL_TEXTURE0);
            glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteTextures(1, &vol);
        }
        assert!(!gl.ctx().objs().textures[vol as usize].used);
        assert_eq!((binding_3d(), gl.ctx().bound_texture_2d_array[0]), (0, arr));
    }

    #[test]
    fn grid_columns_are_the_ceiling_of_the_square_root() {
        let got: Vec<u32> = [0, 1, 2, 4, 5, 9, 10, 16, 17, 256, 1024].iter().map(|&d| grid_cols_for(d)).collect();
        assert_eq!(got, [1, 1, 2, 2, 3, 3, 4, 4, 5, 16, 32]);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_storage_3d_lays_out_the_grid_then_each_levels_grid() {
        let gl = TestGl::new(4, 4);
        let vol = bind_new(GL_TEXTURE_3D);
        // SAFETY: plain GL call.
        /* 3 levels: 5 -> 2 -> 1 (4 is more than the chain, GL_INVALID_OPERATION) */
        unsafe { glTexStorage3D(GL_TEXTURE_3D, 3, GL_RGBA16F, 3, 2, 5) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[vol as usize];
        /* 5 slices: 3 columns, 2 rows of 3x2 -> a 9x4 level 0, the one level
         * the backend is told of; then level 1 (1x1x2) and level 2 (1x1x1),
         * each a grid of the same 3 columns, one row: 3 texels at pitch 4
         * (tex3d-finish) */
        assert_eq!((t.width, t.height, t.layers, t.grid_cols), (3, 2, 5, 3));
        assert_eq!((t.levels, t.chain_levels, t.texfmt), (1, 1, GLES_TEXFMT_RGBA16F));
        assert_eq!((t.format, t.type_), (GL_RGBA, GL_HALF_FLOAT));
        let level0 = tex_pitch(9) as usize * 4 * 8;
        let level = tex_pitch(3) as usize * 8;
        assert_eq!(t.level_off[..3], [0, level0 as u32, (level0 + level) as u32]);
        assert_eq!(t.mem.size, level0 + 2 * level);
        /* a power-of-two grid: no chain the backend is told of either (a
         * fresh texture: the storage above is immutable); levels 1 (4x2 grid
         * of two 2x2 slices) and 2 (1x1x1, a 2-texel pitch) after level 0 */
        let vol = bind_new(GL_TEXTURE_3D);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_3D, 3, GL_RGBA8, 4, 4, 4) };
        let t = &gl.ctx().objs_ref().textures[vol as usize];
        assert_eq!((t.grid_cols, t.chain_levels, t.levels), (2, 1, 1));
        assert_eq!(t.mem.size, 8 * 8 * 4 + 4 * 2 * 4 + 2 * 4);
        /* storage of a 2D array afterwards is laid out as stacked layers again */
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 4, 4, 4) };
        assert_eq!(gl.ctx().objs().textures[arr as usize].grid_cols, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// The ES 3.0 *_INTEGER formats are known enums: an upload in a pairing
    /// this GL does not unpack is GL_INVALID_OPERATION, not GL_INVALID_ENUM
    /// (dEQP-GLES3 negative_api.texture.teximage2d, texsubimage2d/3d), and
    /// glGetInternalformativ knows the integer renderable formats, with no
    /// sample counts (state_query.internal_format.*_samples).
    #[test]
    fn integer_formats_are_known_enums() {
        let gl = TestGl::new(4, 4);
        new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 64]);
        let null = core::ptr::null();
        let mut n: GLint = -1;
        // SAFETY: no texels are read; `n` is one GLint.
        unsafe {
            /* ES 2.0 has no such enum: GL_INVALID_ENUM there, as before */
            let want = if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_INVALID_OPERATION };
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA32UI as GLint, 1, 1, 0, GL_RGBA_INTEGER, GL_INT, null);
            assert_eq!(gl.error(), want);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGBA_INTEGER, GL_UNSIGNED_INT, null);
            assert_eq!(gl.error(), want);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, 0x1234, GL_UNSIGNED_BYTE, null);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "still an enum error for a name that is no format");
            crate::gl::framebuffer::glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8UI, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
            assert_eq!((gl.error(), n), (GL_NO_ERROR, 0));
        }
    }

    /// ES 3.0 3.8.4: more levels than floor(log2(max side)) + 1 is
    /// GL_INVALID_OPERATION; a 3D chain counts the depth, an array's does not
    /// (dEQP-GLES3 negative_api.texture.texstorage{2d,3d}_invalid_levels).
    /// A 3D texture's level past log2(GL_MAX_3D_TEXTURE_SIZE) is
    /// GL_INVALID_VALUE in glTexSubImage3D before the storage is looked for
    /// (texsubimage3d_max_level). All three passed or answered
    /// GL_INVALID_OPERATION until 2026-09-24.
    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_storage_level_counts_and_the_3d_level_cap() {
        let gl = TestGl::new(4, 4);
        bind_new(GL_TEXTURE_2D);
        // SAFETY: plain GL calls; no texels are read.
        unsafe {
            glTexStorage2D(GL_TEXTURE_2D, 6, GL_RGBA8, 16, 4);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "16x4 has 5 levels");
            glTexStorage2D(GL_TEXTURE_2D, 5, GL_RGBA8, 16, 4);
            assert_eq!(gl.error(), GL_NO_ERROR);
            bind_new(GL_TEXTURE_3D);
            glTexStorage3D(GL_TEXTURE_3D, 5, GL_RGBA8, 8, 2, 2);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "8x2x2 has 4 levels");
            bind_new(GL_TEXTURE_2D_ARRAY);
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 2, GL_RGBA8, 2, 2, 16);
            assert_eq!(gl.error(), GL_NO_ERROR, "an array's layers are no level");
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 3, GL_RGBA8, 2, 2, 16);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glTexSubImage3D(GL_TEXTURE_3D, 9, 0, 0, 0, 0, 0, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "level 9 of a 3D texture");
        }
    }

    /// ES 3.0's texture parameters read back what was set, with table
    /// 6.10's initial values (dEQP-GLES3 state_query.texture.*, 80 cases
    /// that read an error until 2026-09-24): the LODs as floats through the
    /// float calls and rounded through the integer ones, a negative base or
    /// max level GL_INVALID_VALUE, a swizzle outside RGBA01 GL_INVALID_ENUM.
    #[test]
    fn es3_texture_parameters_read_back_what_was_set() {
        let gl = TestGl::new(4, 4);
        bind_new(GL_TEXTURE_2D);
        let geti = |p| {
            let mut v: GLint = -7;
            // SAFETY: one GLint.
            unsafe { glGetTexParameteriv(GL_TEXTURE_2D, p, &mut v) };
            v
        };
        let getf = |p| {
            let mut v: GLfloat = -7.0;
            // SAFETY: one GLfloat.
            unsafe { glGetTexParameterfv(GL_TEXTURE_2D, p, &mut v) };
            v
        };
        if crate::REPORT_ES2 {
            assert_eq!(geti(GL_TEXTURE_SWIZZLE_R), -7, "not an ES 2.0 parameter");
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            return;
        }
        assert_eq!(
            [GL_TEXTURE_SWIZZLE_R, GL_TEXTURE_SWIZZLE_A, GL_TEXTURE_WRAP_R, GL_TEXTURE_COMPARE_FUNC].map(geti),
            [GL_RED as GLint, GL_ALPHA as GLint, GL_REPEAT as GLint, GL_LEQUAL as GLint]
        );
        assert_eq!((geti(GL_TEXTURE_BASE_LEVEL), geti(GL_TEXTURE_MAX_LEVEL)), (0, 1000));
        assert_eq!((getf(GL_TEXTURE_MIN_LOD), getf(GL_TEXTURE_MAX_LOD)), (-1000.0, 1000.0));
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_G, GL_ONE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_BASE_LEVEL, 44524);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, 44524);
            glTexParameterf(GL_TEXTURE_2D, GL_TEXTURE_MIN_LOD, -74.5457);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_BASE_LEVEL, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_B, GL_RGBA as GLint);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
        assert_eq!(geti(GL_TEXTURE_SWIZZLE_G), GL_ONE as GLint);
        assert_eq!((geti(GL_TEXTURE_BASE_LEVEL), geti(GL_TEXTURE_MAX_LEVEL)), (44524, 44524));
        assert_eq!((getf(GL_TEXTURE_MIN_LOD), geti(GL_TEXTURE_MIN_LOD)), (-74.5457, -75));
        assert_eq!(geti(GL_TEXTURE_COMPARE_MODE), GL_COMPARE_REF_TO_TEXTURE as GLint);
        assert_eq!(gl.ctx().objs_ref().textures[gl.ctx().bound_texture_2d[0] as usize].max_level, MAX_LEVEL_DEFAULT);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_storage_3d_limits() {
        let gl = TestGl::new(4, 4);
        let vol = bind_new(GL_TEXTURE_3D);
        // SAFETY: plain GL calls; nothing is uploaded.
        unsafe {
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 1, 1, 1025);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "at most 1024 slices");
            /* 4 slices: 2 columns; 8193 * 2 texels exceed the TPU's 16384 */
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 8193, 1, 4);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 1, 8193, 3);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            assert!(gl.ctx().objs().textures[vol as usize].mem.impl_.is_null(), "nothing allocated");
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 1, 1, 1024);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        let t = &gl.ctx().objs_ref().textures[vol as usize];
        /* level 0's 32x32 grid, then levels 1..8 (MAX_3D_LEVELS): 512 down to
         * 4 slices of 1x1 in 32 columns, 16, 8, 4, 2 rows and then one */
        let rows = [32u32, 16, 8, 4, 2, 1, 1, 1, 1];
        assert_eq!((t.grid_cols, t.layers), (32, 1024));
        assert_eq!(t.mem.size, rows.iter().map(|r| 32 * *r as usize * 4).sum::<usize>());
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_image_3d_writes_each_slice_into_its_tile() {
        let gl = TestGl::new(4, 4);
        let vol = bind_new(GL_TEXTURE_3D);
        let mut px = Vec::new();
        for z in 0..5u8 {
            px.extend([0x10 + z, 0x20 + z, 0x30 + z, 0x40 + z, 0x50 + z, 0x60 + z, 0x70 + z, 0x80 + z]);
        }
        // SAFETY: `px` holds 5 slices of one 2x1 row.
        unsafe { glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8 as GLint, 2, 1, 5, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[vol as usize];
        assert_eq!((t.width, t.height, t.layers, t.grid_cols, t.levels), (2, 1, 5, 3, 1));
        /* a 6x2 grid: slices 0..2 on row 0, 3..4 on row 1, the last tile
         * untouched; then the grids of levels 1 (1x1x2) and 2 (1x1x1), one
         * row of 3 columns at pitch 4 each */
        let st = storage(&gl, vol);
        assert_eq!(st.len(), 6 * 2 * 4 + 2 * 4 * 4);
        assert_eq!(st[..24], px[..24]);
        assert_eq!(st[24..40], px[24..40]);
        assert_eq!(st[40..48], [0; 8], "the cleared storage");
        /* one texel of slice 4, column 1: x = 1 + 1 * 2 */
        let one = [1u8, 2, 3, 4];
        // SAFETY: `one` holds one texel.
        unsafe { glTexSubImage3D(GL_TEXTURE_3D, 0, 1, 0, 4, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&one)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, vol)[(6 + 3) * 4..(6 + 4) * 4], one);
        /* a level above 0 of another size than the chain's (level 1 is
         * 1x1x2) is no error, and recorded as not fitting */
        // SAFETY: as above.
        unsafe { glTexImage3D(GL_TEXTURE_3D, 1, GL_RGBA8 as GLint, 1, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&one)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_ne!(gl.ctx().objs().textures[vol as usize].level_bad & 2, 0);
        /* level 1 of the chain's size is held in its own grid after level 0,
         * and the backend is still told of level 0 alone */
        let two = [9u8, 8, 7, 6, 5, 4, 3, 2];
        // SAFETY: `two` holds two 1x1 slices.
        unsafe { glTexImage3D(GL_TEXTURE_3D, 1, GL_RGBA8 as GLint, 1, 1, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&two)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, vol)[48..56], two);
        let t = &gl.ctx().objs_ref().textures[vol as usize];
        assert_eq!((t.levels, t.level_off[1], t.level_bad & 2), (1, 48, 0));
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_sub_image_3d_checks_the_slice_size_not_the_grid() {
        let gl = TestGl::new(4, 4);
        bind_new(GL_TEXTURE_3D);
        let two = solid(2, 9);
        // SAFETY: plain GL calls; `two` holds two texels.
        unsafe {
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 2, 2, 3);
            glTexSubImage3D(GL_TEXTURE_3D, 0, 1, 0, 0, 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&two));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past the slice's width, though inside the grid");
            glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 2, 2, 1, 2, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&two));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past the last slice");
            glTexSubImage3D(GL_TEXTURE_3D, 1, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&two));
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "one level only");
            glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 1, 2, 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&two));
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_image_3d_stacks_the_layers_of_an_array() {
        let gl = TestGl::new(4, 4);
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        let mut px = Vec::new();
        for z in 0..3u8 {
            px.extend(solid(2, 0x30 + z));
        }
        // SAFETY: `px` holds 3 slices of one 2x1 row (4-byte texels: no padding).
        unsafe {
            glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA8 as GLint, 2, 1, 3, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px))
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[arr as usize];
        assert_eq!((t.width, t.height, t.layers), (2, 1, 3));
        assert_eq!(storage(&gl, arr)[..24], px[..]);
        /* one layer replaced */
        let one = solid(2, 0x77);
        // SAFETY: `one` holds one 2x1 slice.
        unsafe { glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 2, 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&one)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, arr)[16..24], one[..]);
        // SAFETY: plain GL calls (nothing is read on the error paths).
        unsafe {
            glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 2, 2, 1, 2, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&one));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past the last layer");
            glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 0, 1, 1, 1, 0x1234, GL_UNSIGNED_BYTE, ptr(&one));
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                GL_RGBA8 as GLint,
                2,
                1,
                1025,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            assert_eq!(gl.error(), GL_INVALID_VALUE, "at most 1024 layers");
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                1,
                GL_RGBA8 as GLint,
                1,
                1,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a level must keep the layer count");
            /* nothing bound to TEXTURE_3D is that target's default texture
             * object (ES 3.0 3.8), which takes the image */
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8 as GLint, 1, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(gl.ctx().objs().textures[MAX_TEXTURES + 3].width, 1);
        }
        /* "glTexImage3D(3D): not available on OpenGL ES 2.0" is not asserted:
         * no code logs that line (this build takes TEXTURE_3D, as above), so
         * a check on it could never fail. */
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn tex_storage_3d_lays_out_the_array_chain() {
        let gl = TestGl::new(4, 4);
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL calls.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 2, GL_R16F, 4, 4, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[arr as usize];
        assert_eq!(
            (t.layers, t.texfmt, t.bpp, t.levels, t.format, t.type_),
            (2, GLES_TEXFMT_R16F, 2, 2, GL_RED, GL_HALF_FLOAT)
        );
        /* level 0 is 4 x (4 * 2) of 2-byte texels */
        assert_eq!(t.level_off[1], 4 * 8 * 2);
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_R16F, 4, 4, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            /* upstream 6a1c6df: a depth texture is a sized format now, so
             * this one is refused for being immutable already, not for its
             * internal format */
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_DEPTH_COMPONENT16, 4, 4, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            /* Nothing bound to TEXTURE_3D is that target's default object,
             * which ES 3.0 3.8.4 refuses immutable storage (this pinned it
             * accepted until 2026-09-24; see `tex_storage_errors`). */
            glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 4, 4, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert!(!gl.ctx().objs().textures[MAX_TEXTURES + 3].immutable);
        }
    }

    #[test]
    /* The sized float internal formats are ES 3.0's; the report-es2
     * surface refuses them at glTexImage2D, so this asks about a
     * capability that build does not offer. */
    #[cfg(not(feature = "report-es2"))]
    fn float_internal_formats_store_float_texels() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let texel = [1.0f32, 0.5, -2.0, 0.25];
        // SAFETY: one RGBA float texel.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F as GLint, 1, 1, 0, GL_RGBA, GL_FLOAT, ptr(&texel)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().textures[name as usize].texfmt, GLES_TEXFMT_RGBA16F);
        let halves: Vec<u8> = [0x3C00u16, 0x3800, 0xC000, 0x3400].iter().flat_map(|h| h.to_ne_bytes()).collect();
        assert_eq!(storage(&gl, name)[..8], halves[..]);

        /* R32F keeps the red float; R16F its half; bytes normalise to [0, 1] */
        // SAFETY: one RGBA float texel.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_R32F as GLint, 1, 1, 0, GL_RGBA, GL_FLOAT, ptr(&texel)) };
        assert_eq!(gl.ctx().objs().textures[name as usize].texfmt, GLES_TEXFMT_R32F);
        assert_eq!(storage(&gl, name)[..4], 1.0f32.to_ne_bytes());
        // SAFETY: plain GL calls; one LUMINANCE_ALPHA byte texel.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_R16F as GLint,
                1,
                1,
                0,
                GL_LUMINANCE_ALPHA,
                GL_UNSIGNED_BYTE,
                ptr(&[255u8, 0]),
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..2], 0x3C00u16.to_ne_bytes());
    }

    #[test]
    /* The sized float internal formats are ES 3.0's; the report-es2
     * surface refuses them at glTexImage2D, so this asks about a
     * capability that build does not offer. */
    #[cfg(not(feature = "report-es2"))]
    fn half_float_upload_treats_subnormals_as_zero() {
        /* the upload's decoder drops subnormal halves (far below 1/255) */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let texel = [0x0001u16, 0x3C00, 0x0000, 0x3C00];
        // SAFETY: one RGBA half texel.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F as GLint, 1, 1, 0, GL_RGBA, GL_HALF_FLOAT, ptr(&texel)) };
        let want: Vec<u8> = [0x0000u16, 0x3C00, 0x0000, 0x3C00].iter().flat_map(|h| h.to_ne_bytes()).collect();
        assert_eq!(storage(&gl, name)[..8], want[..]);
    }

    #[test]
    /* The sized float internal formats are ES 3.0's; the report-es2
     * surface refuses them at glTexImage2D, so this asks about a
     * capability that build does not offer. */
    #[cfg(not(feature = "report-es2"))]
    fn switching_the_texel_format_reallocates() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let first = gl.ctx().objs().textures[name as usize].mem.gpu;
        // SAFETY: plain GL call.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F as GLint, 2, 2, 0, GL_RGBA, GL_FLOAT, core::ptr::null()) };
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_ne!(t.mem.gpu, first);
        assert_eq!((t.texfmt, t.bpp), (GLES_TEXFMT_RGBA16F, 8));
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn out_of_memory_storage_logs_the_layout() {
        let gl = TestGl::new(4, 4);
        bind_new(GL_TEXTURE_2D_ARRAY);
        gl.logs();
        gl.fail_alloc_in(1);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA16F, 3, 2, 4) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        /* NPOT: one level of 4 x 8 texels of 8 bytes */
        assert_eq!(
            gl.logs(),
            ["texture 3x2 x4 fmt 5 (1 levels, 256 bytes): out of GPU memory", "GL error 0x505 in glTexStorage3D"]
        );
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn generate_mipmap_fills_the_chain_of_a_cube_map_and_of_a_float_array() {
        let gl = TestGl::new(4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL calls.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 2, 2) };
        for f in 0..6u32 {
            let px = solid(4, 20 * f as u8);
            // SAFETY: the 2x2 face image.
            unsafe {
                glTexSubImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px))
            };
        }
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!(t.levels, t.chain_levels);
        /* level 1 is 1 x 6: one texel per face, one texel a row (the TPU
         * reads a 1-wide level at its own width) */
        let off = t.level_off[1] as usize;
        let s = storage(&gl, cube);
        for f in 0..6usize {
            assert_eq!(s[off + f * 4], 20 * f as u8, "face {f}");
        }

        /* upstream 2c48538: float texels get a chain too (Quick 3D generates
         * the mip chain of its HDR environment textures) */
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA16F, 2, 2, 2);
            glGenerateMipmap(GL_TEXTURE_2D_ARRAY);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[arr as usize];
        assert_eq!(t.levels, t.chain_levels);
        assert!(t.chain_levels > 1);
    }

    #[test]
    fn the_mip_chain_of_a_cube_map_is_one_faces() {
        /* upstream 6ced57d: a cube map keeps its six faces stacked in one
         * linear texture, and halving that whole height each level gave a
         * 512x512 cube twelve levels instead of ten. The two tails (3 rows,
         * then 1) cannot hold six faces at all, and the backend was told to
         * sample levels that do not exist. */
        assert_eq!(chain_layout(512, 512, 6, 4, false, 0, true).unwrap().1, 10);
        assert_eq!(chain_layout(512, 512, 1, 4, false, 0, true).unwrap().1, 10, "and a 2D texture is unchanged");
        /* level i is `layers` faces of (w >> i) x (h >> i), and the step to
         * the next level is the rounded stacked height (24 -> 32 rows) halved
         * per level, see the next test */
        let (off, levels, bytes, pitch) = chain_layout(4, 4, 6, 4, false, 0, true).unwrap();
        assert_eq!((levels, pitch), (3, 0), "the packed layout has no chain pitch");
        assert_eq!(&off[..3], [0, 4 * 32 * 4, 4 * 32 * 4 + 2 * 16 * 4]);
        /* the 1x1 tail: eight rows (six faces) of one texel, one texel a row */
        assert_eq!(bytes, off[2] + 8 * 4);
    }

    #[test]
    fn a_backend_that_walks_the_layers_gets_the_chain_packed() {
        /* pot_stack false (native_layered_textures, the V3D): every level is
         * its six faces packed, no rounded height -- a 512 RGBA8 cube is six
         * times the 2D 512 chain, 8,388,600 bytes, not 11,184,800 */
        let (off, levels, bytes, pitch) = chain_layout(512, 512, 6, 4, false, 0, false).unwrap();
        let (_, _, bytes_2d, _) = chain_layout(512, 512, 1, 4, false, 0, false).unwrap();
        assert_eq!((levels, pitch), (10, 0));
        assert_eq!(bytes, 6 * bytes_2d);
        assert_eq!(bytes, 8_388_600);
        let (mut lw, mut o) = (512u32, 0u32);
        for (i, &got) in off.iter().enumerate().take(levels as usize) {
            assert_eq!(got, o, "level {i}");
            o += tex_pitch(lw) * lw * 6 * 4;
            lw = (lw / 2).max(1);
        }
        /* the 1x1 tail: six faces, six one-texel rows */
        assert_eq!(bytes, off[9] + 6 * 4);
        /* the rule the PowerVR keeps: the same cube, rounded */
        assert_eq!(chain_layout(512, 512, 6, 4, false, 0, true).unwrap().2, 11_184_800);
        /* one level, and 2D textures, are the same either way */
        assert_eq!(chain_layout(512, 512, 6, 4, true, 0, false), chain_layout(512, 512, 6, 4, true, 0, true));
        assert_eq!(chain_layout(64, 32, 1, 4, false, 0, false), chain_layout(64, 32, 1, 4, false, 0, true));
    }

    #[test]
    fn a_stacked_chain_steps_by_the_power_of_two_rounded_height() {
        /* The PowerVR TPU finds level i+1 from level 0's address with Mesa's
         * pvr_image_plane_setup_mip_levels arithmetic: the level-0 extent
         * rounded up to a power of two whenever there is more than one level.
         * A 512 RGBA16F cube is 512 x 3072 stacked, rounded to 4096, so the
         * TPU reads level 1 at 512*4096*8; the packed 512*3072*8 made Qt
         * Quick 3D's prefiltered light probe black above level 0 (the C-era
         * fix, board-measured 2026-09-22: mean abs error 88.6 -> 3.4 /255). */
        let (off, levels, bytes, _) = chain_layout(512, 512, 6, 8, false, 0, true).unwrap();
        assert_eq!(levels, 10);
        assert_eq!(off[1], 512 * 4096 * 8);
        /* level i: (512 >> i) wide, 4096 >> i rows, every step */
        for i in 1..levels as usize {
            let step = tex_pitch(512 >> (i - 1)) * (4096 >> (i - 1)) * 8;
            assert_eq!(off[i] - off[i - 1], step, "level {i}");
        }
        /* the last level holds six faces of one texel in eight one-texel rows */
        assert_eq!(bytes, off[9] + 8 * 8);

        /* a height that is already a power of two is laid out as before:
         * 2D textures, and arrays of 1, 2 or 4 layers */
        for &(w, h, layers) in &[(8u32, 4u32, 1u32), (4096, 1, 1), (512, 512, 1), (16, 16, 2), (16, 16, 4)] {
            let (off, levels, _, _) = chain_layout(w, h, layers, 4, false, 0, true).unwrap();
            let (mut lw, mut lh, mut o) = (w, h, 0u32);
            for (i, &got) in off.iter().enumerate().take(levels as usize) {
                assert_eq!(got, o, "{w}x{h} x{layers} level {i}");
                o += tex_pitch(lw) * lh * layers * 4;
                lw = (lw / 2).max(1);
                lh = (lh / 2).max(1);
            }
        }

        /* one level, or an NPOT texture (which has one), is not padded */
        assert_eq!(chain_layout(512, 512, 6, 8, true, 0, true).unwrap().2, 512 * 3072 * 8);
        assert_eq!(chain_layout(6, 6, 6, 4, false, 0, true).unwrap().2, 6 * 36 * 4);

        /* an array wider than tall keeps every layer's row once its height is
         * one: the storage never holds fewer rows than a level writes */
        let (off, levels, bytes, _) = chain_layout(8, 1, 3, 4, false, 0, true).unwrap();
        assert_eq!(levels, 4);
        for i in 1..levels as usize {
            assert!(off[i] - off[i - 1] >= tex_pitch(8 >> (i - 1)) * 3 * 4, "level {i}");
        }
        assert!(bytes - off[3] >= 3 * 4, "three one-texel rows");

        /* the GC7000's one-stride layout is untouched by the rounding */
        let (off, _, _, pitch) = chain_layout(4, 4, 6, 4, false, 64, false).unwrap();
        assert_eq!((pitch, off[1]), (4, 4 * 4 * 6 * 4));
    }

    #[test]
    fn a_one_wide_level_above_a_wider_base_is_one_texel_a_row() {
        /* dEQP's non-square mipmap texture is 32x64: its chain ends 1x2,
         * 1x1. The TPU is told level 0's stride only and reads level L at its
         * own width (Mesa pvr_image_plane_setup_mip_levels: pitch = cpp *
         * width); pair-padded rows put the 1x2 level's second row in the
         * padding, and a cube's 1x6 tail read black and neighbouring faces
         * on the board (texture.mipmap.cube.basic images, 2026-09-23). */
        let (off, levels, bytes, _) = chain_layout(32, 64, 1, 4, false, 0, true).unwrap();
        assert_eq!(levels, 7);
        /* 32x64, 16x32, 8x16, 4x8, 2x4 at their widths, then 1x2 and 1x1 */
        let before_1x2: u32 = [(32, 64), (16, 32), (8, 16), (4, 8), (2, 4)].iter().map(|&(w, h)| w * h * 4).sum();
        assert_eq!(off[5], before_1x2);
        assert_eq!(off[6], off[5] + 2 * 4, "1x2: two one-texel rows");
        assert_eq!(bytes, off[6] + 4);
        assert_eq!(one_wide_level_pitch(32, 1, true), 1);
        /* the base of a 1-wide texture keeps its pair pitch (the descriptor
         * states it), and so does a texture with no chain */
        assert_eq!(one_wide_level_pitch(1, 1, true), 2);
        assert_eq!(one_wide_level_pitch(32, 1, false), 2);
        assert_eq!(one_wide_level_pitch(32, 2, true), 2);

        /* an upload into the 1x2 level writes its rows one texel apart */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 2, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let px: Vec<u8> = [0x11u8, 0x22].iter().flat_map(|v| [*v; 4]).collect();
        // SAFETY: the 1x2 image.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 1, GL_RGBA as GLint, 1, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!(t.level_pitch(1), 1);
        let o = t.level_off[1] as usize;
        assert_eq!(&storage(&gl, name)[o..o + 8], &[0x11, 0x11, 0x11, 0x11, 0x22, 0x22, 0x22, 0x22]);
    }

    #[test]
    fn a_backend_that_wants_one_stride_gets_a_chain_laid_out_that_way() {
        /* upstream 8ffe61c and 85dd12f: the GC7000's descriptor names all
         * sixteen level addresses but carries one LINEAR_STRIDE for the whole
         * chain, and fetches a level from a 64-byte boundary. With the packed
         * layout it read level 1's second row at level 0's stride and landed
         * in level 2. */
        let (off, levels, bytes, pitch) = chain_layout(4, 4, 1, 4, false, 64, false).unwrap();
        assert_eq!((levels, pitch), (3, 4), "every level at level 0's pitch");
        /* 4x4 at 16 bytes a row is 64; level 1 is two rows of that pitch (32
         * bytes) rounded up to the next 64-byte boundary */
        assert_eq!(&off[..3], [0, 64, 128]);
        assert_eq!(bytes, 128 + 4 * 4, "the 1x1 tail is one row of the chain pitch");
        /* the packed layout is what it was */
        let (off, _, _, pitch) = chain_layout(4, 4, 1, 4, false, 0, true).unwrap();
        assert_eq!((pitch, &off[..3]), (0, &[0u32, 64, 80][..]));
    }

    #[test]
    fn the_chain_pitch_reaches_the_levels_a_texture_writes_and_filters() {
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = new_tex();
        /* level 0 red, then the chain from it */
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &solid(16, 0x11));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.chain_pitch, t.chain_levels), (4, 3));
        assert_eq!(t.level_off[1], 64, "level 1 starts on a 64-byte boundary");
        assert_eq!(t.level_off[2], 128);
        assert_eq!(t.level_pitch(2), 4, "level 1 is two texels wide but four apart");
        /* an upload into level 1 writes its rows at the chain's pitch */
        // SAFETY: the 2x2 image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 1, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&solid(4, 0x22)))
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, name);
        assert_eq!(&s[64..72], &[0x22; 8], "row 0 of level 1");
        assert_eq!(&s[80..88], &[0x22; 8], "row 1, one chain pitch (16 bytes) later");
        assert_eq!(&s[72..80], &[0; 8], "the pad between them is untouched");
    }

    #[test]
    fn generate_mipmap_filters_at_the_chain_pitch() {
        let gl = TestGl::new(4, 4);
        gl.backend_like_gc7000();
        let name = new_tex();
        /* level 0: the left half 0x40, the right half 0x80 */
        let mut px = Vec::new();
        for _ in 0..4 {
            px.extend(solid(2, 0x40));
            px.extend(solid(2, 0x80));
        }
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &px);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (o1, o2) = (t.level_off[1] as usize, t.level_off[2] as usize);
        let s = storage(&gl, name);
        /* level 1: 2x2, its two rows one chain pitch apart, halves kept */
        assert_eq!(&s[o1..o1 + 8], &[0x40, 0x40, 0x40, 0x40, 0x80, 0x80, 0x80, 0x80]);
        assert_eq!(&s[o1 + 16..o1 + 24], &[0x40, 0x40, 0x40, 0x40, 0x80, 0x80, 0x80, 0x80]);
        /* level 2: one texel, the mean of the two halves */
        assert_eq!(&s[o2..o2 + 4], &[0x60; 4]);
    }

    #[test]
    fn generate_mipmap_filters_each_cube_face_on_its_own() {
        /* upstream 6ced57d: walking the stacked image as one tall texture
         * blends two faces together once a face is one row high. Measured on
         * the board as a light probe whose faces bled into one another. */
        let gl = TestGl::new(4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 3, GL_RGBA8, 4, 4) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        for f in 0..6u32 {
            let px = solid(16, 20 * f as u8);
            // SAFETY: the 4x4 face image.
            unsafe {
                glTexSubImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, 0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px))
            };
        }
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.chain_levels, t.levels), (3, 3), "4x4 faces: 4x4, 2x2, 1x1");
        let (off1, off2) = (t.level_off[1] as usize, t.level_off[2] as usize);
        let s = storage(&gl, cube);
        for f in 0..6usize {
            /* level 1: 2x2 per face, four texels of the face's own value */
            assert_eq!(s[off1 + f * 2 * 2 * 4], 20 * f as u8, "level 1, face {f}");
            /* level 2: one texel per face, one texel a row. A filter that
             * read across faces would average two of them here. */
            assert_eq!(s[off2 + f * 4], 20 * f as u8, "level 2, face {f}");
        }
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn copy_tex_sub_image_3d_writes_one_slice_of_an_array() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let arr = bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 2, 2, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        unsafe { glCopyTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 1, 1, 0, 2, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* slice 1 holds the copy, the slices around it are untouched */
        let s = storage(&gl, arr);
        let want: Vec<u8> = [surf_texel(1, 3), surf_texel(2, 3), surf_texel(1, 2), surf_texel(2, 2)].concat();
        assert_eq!(s[16..32], want[..]);
        assert!(s[..16].iter().all(|&b| b == 0) && s[32..48].iter().all(|&b| b == 0));
        // SAFETY: plain GL calls; nothing is read on the error paths.
        unsafe {
            glCopyTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 3, 0, 0, 2, 2);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past the last slice");
            glCopyTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 1, 0, 0, 0, 0, 2, 2);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past the width of the level");
            glCopyTexSubImage3D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 0, 2, 2);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not an array or 3D target");
        }
        /* a 3D texture tiles its slices in a grid, not a stack: slice 1 of
         * 2x2x2 is the right half of a 4x2 grid (refused with
         * GL_INVALID_OPERATION until tex3d-finish) */
        let vol = bind_new(GL_TEXTURE_3D);
        // SAFETY: plain GL calls; nothing is read.
        unsafe {
            glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8 as GLint, 2, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glCopyTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 1, 0, 0, 2, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = storage(&gl, vol);
        assert_eq!(s[8..16], [surf_texel(0, 3), surf_texel(1, 3)].concat()[..]);
        assert_eq!(s[24..32], [surf_texel(0, 2), surf_texel(1, 2)].concat()[..]);
        assert!(s[..8].iter().chain(&s[16..24]).all(|&b| b == 0), "slice 0 untouched");
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn compressed_tex_sub_image_3d_has_no_storage_to_write_into() {
        let gl = TestGl::new(4, 4);
        bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call; nothing is read.
        unsafe {
            glCompressedTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                0,
                0,
                0,
                4,
                4,
                1,
                GL_COMPRESSED_RGB8_ETC2,
                8,
                core::ptr::null(),
            )
        };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 buffer target */
    fn a_texture_upload_reads_a_bound_unpack_buffer_at_its_offset() {
        /* ES 3.0 3.7.1: with PIXEL_UNPACK_BUFFER bound the pointer argument
         * is a byte offset into that buffer. Reading it as a pointer
         * dereferenced the offset. */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut src = vec![0u8; 32];
        src[16..32].copy_from_slice(&ramp(4));
        let mut buf: GLuint = 0;
        // SAFETY: plain GL calls; `src` is valid for its length.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, 32, ptr(&src), GL_STATIC_DRAW);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, 16 as *const c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..16], ramp(4)[..]);
        // SAFETY: plain GL call; the offset is past the end of the buffer.
        unsafe {
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, 64 as *const c_void);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "an offset past the end of the buffer");
        // SAFETY: plain GL call.
        unsafe { glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0) };
    }

    /// An upload that would read past the end of the bound unpack buffer,
    /// or from an offset that is not a multiple of the type's datum, is
    /// GL_INVALID_OPERATION and reads nothing (ES 3.0 3.7.1). The buffer's
    /// size bounded no read until 2026-09-24: dEQP-GLES3
    /// negative_api.texture.teximage3d_invalid_buffer_target unpacked
    /// 64 x 64 x 64 texels from 512 bytes and the process died.
    ///
    /// Revert check: pass 0 for the size in `unpack_source` again and the
    /// 3D upload below reads 1 MiB past a 512-byte buffer (a crash here, as
    /// in dEQP); drop the datum check and the offset-3 upload succeeds.
    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn an_upload_past_the_unpack_buffer_or_misaligned_is_refused() {
        let gl = TestGl::new(4, 4);
        let mut buf: GLuint = 0;
        let mut tex: GLuint = 0;
        let data = vec![0u8; 512];
        // SAFETY: plain GL calls; `data` is valid for its length.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, 512, ptr(&data), GL_DYNAMIC_COPY);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_3D, tex);
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_RGBA as GLint,
                64,
                64,
                64,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "1 MiB of texels from 512 bytes");
        // SAFETY: as above.
        unsafe {
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_RGB5_A1 as GLint,
                4,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_SHORT_5_5_5_1,
                3 as *const c_void,
            );
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "offset 3 is not a multiple of a 16-bit datum");
        // SAFETY: as above.
        unsafe {
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_RGBA as GLint,
                4,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                256 as *const c_void,
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "exactly 256 bytes from 256: the last byte is the buffer's last");
        // SAFETY: as above.
        unsafe {
            glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, 257 as *const c_void);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "one byte past the end");
        // SAFETY: plain GL call.
        unsafe { glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0) };
    }

    /// The four halves of an RGBA16F texel at byte offset `off`.
    #[cfg(not(feature = "report-es2"))] /* only the ES 3.0 tests use it */
    fn halves4(s: &[u8], off: usize) -> [f32; 4] {
        core::array::from_fn(|c| draw::half_to_float(u16::from_le_bytes([s[off + c * 2], s[off + c * 2 + 1]])))
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn generate_mipmap_averages_rgba16f_texels_as_floats() {
        /* upstream 2c48538: the box filter averages halves as floats */
        let gl = TestGl::new(4, 4);
        let t = bind_new(GL_TEXTURE_2D);
        let mut px = [0u8; 2 * 2 * 8];
        /* the four texels of level 0: r = 1, 2, 4, 8; g/b/a = 0 */
        for (i, r) in [1.0f32, 2.0, 4.0, 8.0].into_iter().enumerate() {
            px[i * 8..i * 8 + 2].copy_from_slice(&float_to_half(r).to_le_bytes());
        }
        // SAFETY: px holds the 2x2 RGBA16F image.
        unsafe {
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA16F, 2, 2);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_HALF_FLOAT, ptr(&px));
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let off = gl.ctx().objs().textures[t as usize].level_off[1] as usize;
        let s = storage(&gl, t);
        assert_eq!(halves4(&s, off), [3.75, 0.0, 0.0, 0.0], "(1 + 2 + 4 + 8) / 4");
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn generate_mipmap_averages_r32f_rows_at_the_padded_pitch() {
        /* a 4 x 1 R32F texture: level 1 is 2 x 1, level 2 is 1 x 1 but
         * stored 2 texels wide (GL_TEX_PITCH) */
        let gl = TestGl::new(4, 4);
        let t = bind_new(GL_TEXTURE_2D);
        let px: [u8; 16] = core::array::from_fn(|i| [1.0f32, 3.0, 10.0, 20.0][i / 4].to_le_bytes()[i % 4]);
        // SAFETY: px holds the 4x1 R32F image.
        unsafe {
            glTexStorage2D(GL_TEXTURE_2D, 1, GL_R32F, 4, 1);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 1, GL_RED, GL_FLOAT, ptr(&px));
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let ctx = gl.ctx();
        let (o1, o2) = (
            ctx.objs().textures[t as usize].level_off[1] as usize,
            ctx.objs().textures[t as usize].level_off[2] as usize,
        );
        let s = storage(&gl, t);
        let f = |o: usize| f32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]);
        assert_eq!([f(o1), f(o1 + 4)], [2.0, 15.0]);
        assert_eq!(f(o2), 8.5, "the 2-wide level-1 row is read at its own pitch");
    }

    #[test]
    fn tex_parameters_and_queries_follow_the_target_binding() {
        let gl = TestGl::new(4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let mut v: GLint = 0;
            glGetTexParameteriv(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, &mut v);
            assert_eq!(v, GL_CLAMP_TO_EDGE as GLint);
            glGetTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, &mut v);
            assert_eq!(gl.error(), GL_NO_ERROR, "the 2D target's default object answers");
            assert_eq!(v, GL_REPEAT as GLint, "which still has GL's initial values");
            /* a face target names the cube map binding here too */
            glTexParameteri(GL_TEXTURE_CUBE_MAP_NEGATIVE_X, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(gl.ctx().objs().textures[cube as usize].mag_filter, GL_NEAREST);
    }

    #[test]
    fn attach_external_is_one_layer_with_the_surface_texel_size() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 4 * 2 * 2];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 4, 2, GLES_SURF_RGB565), 0);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.layers, t.bpp), (1, 2));
    }

    /* ---- cache maintenance ---------------------------------------------------------------------------- */

    fn tex_mem(gl: &TestGl, name: GLuint) -> OsalGpuMem {
        gl.ctx().objs().textures[name as usize].mem
    }

    #[test]
    fn tex_image_flushes_the_rows_it_wrote() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(8));
        let gpu = tex_mem(&gl, name).gpu;
        /* the clear of the fresh storage, then the upload */
        /* 4x2 (32 bytes), 2x1 (8), 1x1 (4: one texel, not a pair) */
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 44 }, CacheOp::Flush { gpu, off: 0, len: 32 }]);
        /* odd width: rows 6 texels apart, the last row ends at its last texel */
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(15));
        let gpu = tex_mem(&gl, name).gpu;
        assert_eq!(
            gl.cache_ops(),
            [CacheOp::Flush { gpu, off: 0, len: 72 }, CacheOp::Flush { gpu, off: 0, len: (2 * 6 + 5) * 4 }]
        );
        assert_eq!(gl.unflushed(gpu, 72), None);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn tex_image_of_a_mip_level_flushes_that_level() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let gpu = tex_mem(&gl, name).gpu;
        let size = tex_mem(&gl, name).size;
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: size }], "no pixels: only the clear");
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 64, len: 16 }]);
        assert_eq!(gl.gpu_bytes(gpu + 64, 16), ramp(4));
    }

    #[test]
    fn tex_sub_image_flushes_exactly_its_rectangle() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 64]);
        gl.cache_ops();
        tex_sub_image(0, 1, 2, 2, 1, GL_RGBA, &ramp(2));
        let gpu = tex_mem(&gl, name).gpu;
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: (2 * 4 + 1) * 4, len: 8 }]);
        tex_sub_image(0, 0, 1, 3, 2, GL_RGBA, &ramp(6));
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 16, len: (4 + 3) * 4 }]);
        assert_eq!(gl.unflushed(gpu, 64), None);
    }

    #[test]
    fn compressed_uploads_flush_the_block_grid() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 8, 8, &blocks(4, 8));
        let gpu = tex_mem(&gl, name).gpu;
        /* the clear of the fresh storage (the whole chain), then the upload (level 0) */
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 56 }, CacheOp::Flush { gpu, off: 0, len: 32 }]);
        // SAFETY: 8 bytes of block data.
        unsafe {
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 4, 4, GL_COMPRESSED_RGB8_ETC2, 8, ptr(&[9u8; 8]));
        }
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 32 }]);
        assert_eq!(gl.gpu_bytes(gpu + 24, 8), [9; 8], "block (1,1) is twiddle index 3");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn generate_mipmap_invalidates_the_chain_then_flushes_it() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 64]);
        let m = tex_mem(&gl, name);
        gl.cache_ops();
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(
            gl.cache_ops(),
            [CacheOp::Invd { gpu: m.gpu, off: 0, len: m.size }, CacheOp::Flush { gpu: m.gpu, off: 0, len: m.size }]
        );
        assert_eq!(gl.unflushed(m.gpu, m.size), None);
        assert_eq!(gl.gpu_bytes(m.gpu + 64, 20), [0; 20], "the levels reach the GPU");
    }

    #[test]
    fn generate_mipmap_filters_what_the_gpu_rendered_into_level_0() {
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[0u8; 64]);
        let m = tex_mem(&gl, name);
        gl.gpu_render(m.gpu, &[40; 64]); /* the texture was a render target */
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.gpu_bytes(m.gpu + 64, 16), [40; 16]);
        assert_eq!(gl.gpu_bytes(m.gpu + 80, 4), [40; 4]);
    }

    #[test]
    fn a_draw_samples_only_published_texels() {
        let gl = TestGl::new(16, 16);
        let prog = gl.program("uv.vert", "texture.frag");
        let tri: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let name = new_tex();
        tex_image(0, 3, 2, GL_RGB, GL_UNSIGNED_BYTE, &[7u8; 18]);
        // SAFETY: plain GL calls; `tri` outlives the draw.
        unsafe {
            use crate::gl::{draw::*, program::*, vertex::*};
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, ptr(&tri));
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            /* respecified while the frame is recorded: fresh storage, the kick keeps the old texels */
            tex_image(0, 3, 2, GL_RGB, GL_UNSIGNED_BYTE, &[8u8; 18]);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            crate::gl::state::glFinish();
        }
        let d = gl.draws();
        assert_eq!(d.len(), 2);
        let (a, b) = (d[0].draw.textures[0].base_gpu, d[1].draw.textures[0].base_gpu);
        assert_ne!(a, b);
        assert_eq!(gl.gpu_bytes(a, 4), [7, 7, 7, 255]);
        assert_eq!(gl.gpu_bytes(b, 4), [8, 8, 8, 255]);
        assert_eq!(gl.ctx().objs().textures[name as usize].last_use, 1);
    }

    /* ---- no CPU view of GPU memory ---------------------------------------------------------------------- */

    #[test]
    fn tex_image_without_a_cpu_view_converts_in_chunks_through_gpu_write() {
        let gl = TestGl::new(4, 4);
        gl.set_cpu_mapping(false);
        let name = new_tex();
        /* 70 texels a row: one 64-texel chunk and a 6-texel one per row; the
         * client rows are 212 bytes apart (GL_UNPACK_ALIGNMENT 4) */
        let rgb: Vec<u8> = (0..212 * 2).map(|i| (i % 251) as u8).collect();
        let texel = |x: usize, y: usize| {
            let o = y * 212 + x * 3;
            [rgb[o], rgb[o + 1], rgb[o + 2], 255]
        };
        tex_image(0, 70, 2, GL_RGB, GL_UNSIGNED_BYTE, &rgb);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let m = tex_mem(&gl, name);
        assert!(m.cpu.is_null());
        let s = storage(&gl, name);
        for y in 0..2 {
            for x in 0..70 {
                assert_eq!(s[(y * 70 + x) * 4..][..4], texel(x, y), "texel {x},{y}");
            }
        }
        assert!(gl.cache_ops().is_empty());
        /* and a sub-rectangle across the chunk boundary */
        tex_sub_image(0, 60, 1, 8, 1, GL_RGBA, &ramp(8));
        assert_eq!(storage(&gl, name)[(70 + 60) * 4..][..32], ramp(8)[..]);
        assert_eq!(storage(&gl, name)[(70 + 59) * 4..][..4], texel(59, 1));
    }

    #[test]
    fn tex_image_without_a_cpu_view_keeps_the_even_row_pitch() {
        let gl = TestGl::new(4, 4);
        gl.set_cpu_mapping(false);
        let name = new_tex();
        tex_image(0, 5, 3, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(15));
        let s = storage(&gl, name);
        let src = ramp(15);
        for row in 0..3 {
            assert_eq!(s[row * 24..row * 24 + 20], src[row * 20..row * 20 + 20], "row {row}");
            assert_eq!(s[row * 24 + 20..row * 24 + 24], [0; 4], "pad texel of row {row}");
        }
    }

    #[test]
    fn compressed_upload_without_a_cpu_view_writes_each_block() {
        let gl = TestGl::new(4, 4);
        let mapped = new_tex();
        compressed_image(GL_COMPRESSED_RGBA8_ETC2_EAC, 10, 5, &blocks(6, 16));
        gl.set_cpu_mapping(false);
        let unmapped = new_tex();
        compressed_image(GL_COMPRESSED_RGBA8_ETC2_EAC, 10, 5, &blocks(6, 16));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(tex_mem(&gl, unmapped).cpu.is_null());
        assert_eq!(storage(&gl, unmapped)[..6 * 16], storage(&gl, mapped)[..6 * 16]);
    }

    #[test]
    fn generate_mipmap_without_a_cpu_view_keeps_the_base_level_only() {
        let gl = TestGl::new(4, 4);
        gl.set_cpu_mapping(false);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[3u8; 64]);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.levels, t.chain_levels), (1, 3));
        /* 4x4 (64 bytes), 2x2 (16), 1x1 (4) */
        assert_eq!(storage(&gl, name)[64..], [0; 20], "no level was written (the cleared storage)");
        assert!(gl.cache_ops().is_empty());
    }

    #[test]
    fn a_failing_texture_upload_is_reported() {
        let gl = TestGl::new(4, 4);
        gl.set_cpu_mapping(false);
        new_tex();
        gl.fail_gpu_write_in(1);
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "glTexImage2D");
        gl.fail_gpu_write_in(1);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &ramp(1));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "glTexSubImage2D");
        new_tex();
        gl.fail_gpu_write_in(1);
        compressed_image(GL_COMPRESSED_RGB8_ETC2, 4, 4, &blocks(1, 8));
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "glCompressedTexImage2D");
    }

    #[test]
    fn updating_a_texture_a_pending_kick_samples_leaves_that_kick_the_old_texels() {
        let gl = TestGl::new(16, 16);
        let prog = gl.program("uv.vert", "texture.frag");
        let tri: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[1u8; 16]);
        // SAFETY: plain GL calls; `tri` outlives the draw.
        unsafe {
            use crate::gl::{draw::*, program::*, vertex::*};
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, ptr(&tri));
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[2u8; 4]);
        // SAFETY: plain GL call.
        unsafe { crate::gl::state::glFinish() };
        /* the mock reports "a texture ... changed between the draw and kick 1" */
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- regression tests for the round-2 fixes ------------------------------------------ */

    #[test]
    fn gen_and_delete_textures_through_a_misaligned_array() {
        /* a GLuint* from the application carries no alignment guarantee
         * (aarch64-unknown-none is +strict-align; a typed slice over it is
         * undefined behaviour and aborts a dev build) */
        let gl = TestGl::new(4, 4);
        let mut raw = [0u8; 2 * 4 + 1];
        // SAFETY: one byte into the buffer: room for two unaligned names.
        let p = unsafe { raw.as_mut_ptr().add(1) } as *mut GLuint;
        assert_ne!(p as usize % 4, 0, "a deliberately misaligned array");
        // SAFETY: `p` has room for two names.
        unsafe { glGenTextures(2, p) };
        // SAFETY: as above.
        let names = unsafe { [p.read_unaligned(), p.add(1).read_unaligned()] };
        assert!(names[0] != 0 && names[1] != 0 && names[0] != names[1]);
        // SAFETY: as above.
        unsafe { glDeleteTextures(2, p) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().textures.iter().all(|t| !t.used));
    }

    #[test]
    fn new_storage_is_cleared_instead_of_showing_the_pools_last_contents() {
        /* the zephyr / bare-metal pools hand out memory as the previous
         * user left it (the mock fills it with FILL); GL leaves new texels
         * undefined, but they must not be another object's */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        let s = storage(&gl, name);
        assert!(s.iter().all(|&b| b == 0), "no byte of the pool's last user is visible");
        /* a level uploaded afterwards is still the application's */
        tex_image(1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(4));
        let off = gl.ctx().objs().textures[name as usize].level_off[1] as usize;
        assert_eq!(storage(&gl, name)[off..off + 16], ramp(4)[..]);
    }

    #[test]
    fn copy_tex_image_outside_the_framebuffer_reads_zeros() {
        /* GL leaves pixels outside the read buffer undefined; they must not
         * be the pool's last contents either */
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let name = new_tex();
        copy_tex_image(GL_RGBA, 8, 8, 2, 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..16], [0u8; 16], "wholly outside the framebuffer");
    }

    #[test]
    fn an_external_image_must_carry_rows_at_the_pair_pitch() {
        /* every reader and writer of a texture walks tex_pitch(w) texels a
         * row: a 3 x 2 image with packed rows would be written 8 bytes past
         * its end (and sampled skewed) */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut packed = vec![0x11u8; 3 * 2 * 4];
        let mem = external_mem(&mut packed);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 3, 2, backend::GLES_SURF_RGBA8888), -1);
        assert!(!gl.ctx().objs().textures[name as usize].external, "the texture is untouched");
        tex_sub_image(0, 0, 0, 3, 2, GL_RGBA, &ramp(6));
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "no storage to write");
        assert_eq!(packed, vec![0x11u8; 24], "not one byte of the application's buffer");
        /* the same image with the pad texel is accepted */
        let mut padded = vec![0x11u8; 4 * 2 * 4];
        let mem = external_mem_at(&mut padded, 0x1235_0000);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 3, 2, backend::GLES_SURF_RGBA8888), 0);
        tex_sub_image(0, 0, 0, 3, 2, GL_RGBA, &ramp(6));
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(padded[..12], ramp(6)[..12]);
        assert_eq!(padded[12..16], [0x11; 4], "the pad texel is not written");
        gl.ctx().objs().textures[name as usize].external = false;
        gl.ctx().objs().textures[name as usize].mem = OsalGpuMem::ZERO;
    }

    #[test]
    fn tex_sub_image_into_an_xrgb8888_image_stores_bgrx() {
        /* the converters produce R,G,B,A; an XRGB8888 buffer (the scan-out
         * order) holds B,G,R,X, and the TPU / PBE read it that way */
        let gl = TestGl::new(4, 4);
        let name = new_tex();
        let mut backing = vec![0u8; 2 * 2 * 4];
        let mem = external_mem(&mut backing);
        assert_eq!(texture_attach_external(gl.ctx(), &mem, 2, 2, GLES_SURF_XRGB8888), 0);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[1, 2, 3, 4]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(backing[..4], [3, 2, 1, 4], "B,G,R,X");
        gl.ctx().objs().textures[name as usize].external = false;
        gl.ctx().objs().textures[name as usize].mem = OsalGpuMem::ZERO;
    }

    #[test]
    fn generate_mipmap_waits_for_the_kick_that_samples_the_texture() {
        /* the box filter rewrites the chain in place: a kick that samples
         * the texture must have run, or it sees half-filtered texels (and
         * the invalidate would discard what the GPU is writing) */
        let gl = TestGl::new(16, 16);
        let prog = gl.program("uv.vert", "texture.frag");
        let tri: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[1u8; 16]);
        // SAFETY: plain GL calls; `tri` outlives the draw.
        unsafe {
            use crate::gl::{draw::*, program::*, vertex::*};
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, ptr(&tri));
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert!(gl.kick_pending(1), "the draw's kick is open");
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert!(!gl.kick_pending(1), "glGenerateMipmap let it run first");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* an ES 3.0 target, entry point or type */
    fn a_texture_whose_chain_does_not_fit_the_offsets_is_out_of_memory() {
        /* checked arithmetic: the level offsets are u32, and the shipped
         * profile has no overflow checks */
        let gl = TestGl::new(4, 4);
        bind_new(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call; no pixels are read.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA16F, 4096, 4096, 1024) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), 1, "nothing was allocated");
        // SAFETY: plain GL call.
        unsafe {
            /* 256 layers: GL_MAX_ARRAY_TEXTURE_LAYERS, past which the call
             * is GL_INVALID_VALUE before any allocation (ES 3.0 3.8.3; this
             * asked for 1024 until 2026-09-24). 16 GiB still does not fit. */
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                GL_RGBA8 as GLint,
                4096,
                4096,
                256,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            )
        };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), 1);
    }

    #[test]
    fn a_texture_write_under_a_kick_that_renders_into_it_waits_too() {
        /* the texture is the colour attachment of an open frame: the CPU
         * write must not race the PBE */
        let gl = TestGl::new(16, 16);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[7u8; 64]);
        let mut fb: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            crate::gl::state::glClearColor(0.0, 0.0, 0.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            crate::gl::state::glFlush();
        }
        assert_eq!(gl.draws().len(), 1, "the clear kick renders into the texture");
        gl.set_kick_pending(1, true);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 9, 9, 9]);
        assert!(gl.finishes() > 0, "the update waited for the kick");
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.set_kick_pending(1, false);
    }

    /// A texture whose storage kick 7, still pending, sampled; the finishes
    /// counted so far.
    fn sampled_by_a_pending_kick(gl: &TestGl) -> (GLuint, u32) {
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[7u8; 64]);
        gl.ctx().objs().textures[name as usize].last_use = 7;
        gl.set_kick_pending(7, true);
        (name, gl.finishes())
    }

    #[test]
    fn a_sub_image_under_a_kick_that_sampled_it_waits_when_the_draws_read_the_storage() {
        /* the PowerVR and the GC7000 sample the GL core's storage itself */
        let gl = TestGl::new(16, 16);
        let (name, finishes) = sampled_by_a_pending_kick(&gl);
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 9, 9, 9]);
        assert!(gl.finishes() > finishes, "the write waited for the kick sampling the storage");
        assert_eq!(storage(&gl, name)[..4], [9, 9, 9, 9]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.set_kick_pending(7, false);
    }

    #[test]
    fn a_sub_image_under_a_kick_that_sampled_it_does_not_wait_when_the_draws_read_copies() {
        /* the V3D samples a tiled copy made when the draw was recorded: the
         * write cannot change what the pending kick reads, and waiting for
         * it under the async swap submitted the frame being drawn and
         * waited out the previous one at every refill (DEMO14) */
        let gl = TestGl::new(16, 16);
        gl.backend_samples_texel_copies();
        let (name, finishes) = sampled_by_a_pending_kick(&gl);
        let gen = gl.ctx().objs_ref().textures[name as usize].content_gen;
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 9, 9, 9]);
        assert_eq!(gl.finishes(), finishes, "no finish for a kick that sampled a copy");
        assert_eq!(storage(&gl, name)[..4], [9, 9, 9, 9]);
        assert_ne!(
            gl.ctx().objs_ref().textures[name as usize].content_gen,
            gen,
            "the stamp moved, so the next draw makes a new copy"
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.set_kick_pending(7, false);
    }

    #[test]
    fn a_texture_write_under_a_kick_that_renders_into_it_waits_when_the_draws_read_copies() {
        /* the copies are of what a draw samples; a kick that renders into
         * the texture writes the storage itself, and the CPU write must not
         * race it on any backend */
        let gl = TestGl::new(16, 16);
        gl.backend_samples_texel_copies();
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[7u8; 64]);
        let mut fb: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            crate::gl::state::glClearColor(0.0, 0.0, 0.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            crate::gl::state::glFlush();
        }
        assert_eq!(gl.draws().len(), 1, "the clear kick renders into the texture");
        gl.set_kick_pending(1, true);
        let finishes = gl.finishes();
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 9, 9, 9]);
        assert!(gl.finishes() > finishes, "the update waited for the kick rendering into it");
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.set_kick_pending(1, false);
    }

    #[test]
    fn generate_mipmap_waits_for_the_kicks_that_render_into_the_texture() {
        /* upstream 6ced57d: Qt builds a light probe by drawing a cube's six
         * faces into a texture and then asking for its chain. The levels are
         * filtered here, on the CPU, from level 0's texels, and those kicks
         * were still in flight: the last face's whole chain was built from
         * zeros and never corrected, because the wrong values had already
         * been written. Upstream drains unconditionally; this port drains
         * exactly when a kick may still be writing the texture, which
         * `settle_texture` knows from the render-target use below. */
        let gl = TestGl::new(16, 16);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[7u8; 64]);
        let mut fb: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            crate::gl::state::glClearColor(0.0, 0.0, 0.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            crate::gl::state::glFlush();
        }
        assert_eq!(gl.draws().len(), 1, "the clear kick renders into the texture");
        gl.set_kick_pending(1, true);
        let finishes = gl.finishes();
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert!(gl.finishes() > finishes, "the chain was built after the GPU, not before it");
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.set_kick_pending(1, false);
    }

    #[test]
    fn a_pending_clear_of_the_texture_goes_out_before_an_upload_into_it() {
        /* glClear is merged into the next draw's kick; a glTexSubImage2D
         * into the cleared texture must not end up under it */
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        tex_image(0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, &[1u8; 16]);
        let mut fb: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            crate::gl::state::glClearColor(1.0, 1.0, 1.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(gl.draws().len(), 0, "the clear waits for a draw to join");
        tex_sub_image(0, 0, 0, 1, 1, GL_RGBA, &[9, 9, 9, 9]);
        assert_eq!(gl.draws().len(), 1, "it was emitted before the upload");
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.take_faults().is_empty());
    }

    /* ---- the errors GL ES 2.0 names, as dEQP-GLES2 negative_api asks for
     * them. Each test says which case body it mirrors; the group is
     * tests/negative_api (functional.negative_api.texture.*). ------------- */

    /// `teximage2d_invalid_internalformat`: an internal format this GL does
    /// not name is GL_INVALID_VALUE -- not GL_INVALID_OPERATION, which is
    /// what a lookup failure further down would have reported.
    #[test]
    #[cfg(feature = "report-es2")]
    fn the_es2_surface_refuses_the_sized_names_no_extension_of_its_claims() {
        /* GL_EXTENSIONS names GL_OES_required_internalformat and
         * GL_EXT_texture_rg, so the sized names those cover are taken. It
         * does NOT name GL_EXT_sRGB or GL_OES_texture_float -- the sRGB
         * class is ES 3.0's alone (docs/es3/srgb.md), and GL_RGBA32F would
         * be stored as halves -- so those names are refused rather than accepted and then
         * reported as a complete framebuffer. dEQP's fbo.completeness reads
         * the second as a texture format this GL does not have. */
        let gl = TestGl::new(8, 8);
        let t = bind_new(GL_TEXTURE_2D);
        let _ = t;
        /* the float names with the extension's GL_HALF_FLOAT_OES: with ES 3.0's
         * GL_HALF_FLOAT or GL_FLOAT the type is refused first, GL_INVALID_ENUM
         * (`es3_only_pixel_type`) */
        const GL_HALF_FLOAT_OES: GLenum = 0x8D61;
        for (internal, format, type_, want) in [
            (GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE, GL_INVALID_VALUE),
            (GL_SRGB8, GL_RGB, GL_UNSIGNED_BYTE, GL_INVALID_VALUE),
            (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT_OES, GL_INVALID_VALUE),
            (GL_R16F, GL_RED, GL_HALF_FLOAT_OES, GL_INVALID_VALUE),
            (GL_RGBA32F, GL_RGBA, GL_HALF_FLOAT_OES, GL_INVALID_VALUE),
            (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, GL_INVALID_ENUM),
            (GL_R16F, GL_RED, GL_HALF_FLOAT, GL_INVALID_ENUM),
            (GL_RGBA32F, GL_RGBA, GL_FLOAT, GL_INVALID_ENUM),
        ] {
            // SAFETY: null pixels only allocate.
            unsafe { glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, format, type_, core::ptr::null()) };
            assert_eq!(gl.error(), want, "0x{internal:x} type 0x{type_:x}");
        }
        /* and the ones an advertised extension does cover are taken */
        for (internal, format) in [(GL_RGBA8, GL_RGBA), (GL_RGB565, GL_RGB), (GL_R8, GL_RED), (GL_RG8, GL_RG)] {
            // SAFETY: as above.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, format, GL_UNSIGNED_BYTE, core::ptr::null())
            };
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{internal:x}");
        }
    }

    #[test]
    fn tex_image_2d_unknown_internalformat_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        new_tex();
        // SAFETY: plain GL call, null image.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, 0, 1, 1, 0, GL_RGB, GL_UNSIGNED_BYTE, core::ptr::null()) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
    }

    /// `teximage2d_format_mismatch`: an unsized internal format must name the
    /// same base format as `format`.
    #[test]
    fn tex_image_2d_unsized_internalformat_must_match_the_format() {
        let gl = TestGl::new(8, 8);
        new_tex();
        // SAFETY: plain GL call, null image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null())
        };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);

        /* the sized ES 3.0 formats carry their own pairing and are not
         * compared this way */
        // SAFETY: plain GL call, null image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null())
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// `teximage2d_inequal_width_height_cube`: a cube map face is square.
    #[test]
    fn tex_image_2d_cube_face_must_be_square() {
        let gl = TestGl::new(8, 8);
        bind_new(GL_TEXTURE_CUBE_MAP);
        for f in 0..6u32 {
            // SAFETY: plain GL call, null image.
            unsafe {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGB as GLint,
                    1,
                    2,
                    0,
                    GL_RGB,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                )
            };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "face {f}");
        }
    }

    /// `teximage2d_max_width_height_*`: above GL_MAX_TEXTURE_SIZE is
    /// GL_INVALID_VALUE, and the limit is the one glGetIntegerv reports.
    #[test]
    fn tex_image_2d_beyond_max_texture_size_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        new_tex();
        let mut max: GLint = 0;
        // SAFETY: room for one value.
        unsafe { crate::gl::state::glGetIntegerv(GL_MAX_TEXTURE_SIZE, &mut max) };
        assert_eq!(max, MAX_TEXTURE_SIZE, "the limit the shape check uses is the one GL reports");
        tex_image(0, max + 1, 1, GL_RGB, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        tex_image(0, 1, max + 1, GL_RGB, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
    }

    /// `copyteximage2d_*_cube_*`: the copy calls take a cube map face. They
    /// used to reject every face on GL_INVALID_ENUM, which hid the error each
    /// case was really asking about.
    #[test]
    fn copy_tex_image_2d_takes_a_cube_face_and_checks_its_shape() {
        let gl = TestGl::new(8, 8);
        fill_surface(&gl, 8, 8);
        bind_new(GL_TEXTURE_CUBE_MAP);
        for f in 0..6u32 {
            let face = GL_TEXTURE_CUBE_MAP_POSITIVE_X + f;
            // SAFETY: plain GL calls.
            unsafe {
                /* a well-formed face copy is accepted */
                glCopyTexImage2D(face, 0, GL_RGBA, 0, 0, 4, 4, 0);
                assert_eq!(gl.error(), GL_NO_ERROR, "face {f}");
                /* a non-square face is GL_INVALID_VALUE, not GL_INVALID_ENUM */
                glCopyTexImage2D(face, 0, GL_RGBA, 0, 0, 4, 2, 0);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "face {f}");
                /* a border is GL_INVALID_VALUE */
                glCopyTexImage2D(face, 0, GL_RGBA, 0, 0, 4, 4, 1);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "face {f}");
            }
        }
    }

    /// A cube face copied from the framebuffer lands in that face's rows of
    /// the stacked image, not over face 0 (the faces are stacked
    /// vertically).
    #[test]
    fn copy_tex_image_2d_writes_the_face_it_was_given() {
        let gl = TestGl::new(4, 4);
        fill_surface(&gl, 4, 4);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        for f in 0..6u32 {
            // SAFETY: plain GL call.
            unsafe { glCopyTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, GL_RGBA, 0, 0, 4, 4, 0) };
            assert_eq!(gl.error(), GL_NO_ERROR, "face {f}");
        }
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.layers, t.width, t.height), (6, 4, 4));
        let s = storage(&gl, cube);
        /* every face's first row was written (the surface fill is non-zero) */
        for f in 0..6usize {
            let row = &s[f * 4 * 4 * 4..][..16];
            assert!(row.iter().any(|&b| b != 0), "face {f} was not written");
        }
    }

    /// `copytexsubimage2d_incomplete_framebuffer`: reading from an incomplete
    /// framebuffer outranks whatever texture is bound.
    #[test]
    fn copy_tex_sub_image_2d_checks_the_framebuffer_first() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let mut fb: GLuint = 0;
        // SAFETY: room for one name; an FBO with no attachment is incomplete.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glBindTexture(GL_TEXTURE_2D, name);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
        }
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
    }

    /// `compressedteximage2d_*_cube_*` / `_neg_level_*`: the shape of the call
    /// is checked before "this core does not build that", so a bad level or
    /// size is reported as GL says. A well-formed cube or mip-level call is
    /// GL_INVALID_OPERATION, because only one 2D level is laid out.
    #[test]
    fn a_zero_sized_compressed_level_is_legal_and_allocates_nothing() {
        /* dEQP negative_api.texture.generatemipmap_zero_level_array_compressed
         * opens with exactly this call and expects GL_NO_ERROR: a 0x0 level
         * with imageSize 0 and no data is well formed, it simply has no
         * texels. The block grid is then 0 x 0 blocks and the storage is zero
         * bytes, which is not an allocation failure -- it is nothing to
         * allocate. (The PowerVR twiddled grid rounds 0 up to a 1 x 1 block,
         * so it allocated one block and never noticed; the GC7000 keeps
         * blocks in raster rows, computed 0, and asked the OSAL for a
         * zero-byte buffer.) */
        let gl = TestGl::new(8, 8);
        /* the raster-row block layout is the one that computes 0 blocks; the
         * twiddled grid rounds up to 1 and hides it */
        gl.backend_like_gc7000();
        bind_new(GL_TEXTURE_2D);
        // SAFETY: imageSize 0 with a null pointer is what the size says.
        unsafe { glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 0, 0, 0, 0, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR, "a 0x0 compressed level is well formed");
    }

    #[test]
    fn compressed_tex_image_2d_reports_the_shape_on_every_face() {
        let gl = TestGl::new(8, 8);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        let blocks = [0u8; 8];
        for f in 0..6u32 {
            let face = GL_TEXTURE_CUBE_MAP_POSITIVE_X + f;
            // SAFETY: `blocks` holds the one 4x4 ETC1 block imageSize names.
            unsafe {
                glCompressedTexImage2D(face, -1, GL_ETC1_RGB8_OES, 4, 4, 0, 8, ptr(&blocks));
                assert_eq!(gl.error(), GL_INVALID_VALUE, "negative level, face {f}");
                glCompressedTexImage2D(face, 0, GL_ETC1_RGB8_OES, 4, 4, 1, 8, ptr(&blocks));
                assert_eq!(gl.error(), GL_INVALID_VALUE, "border, face {f}");
                glCompressedTexImage2D(face, 0, GL_ETC1_RGB8_OES, -4, 4, 0, 8, ptr(&blocks));
                assert_eq!(gl.error(), GL_INVALID_VALUE, "negative width, face {f}");
                /* well formed: the face is stored (GL_INVALID_OPERATION until
                 * 2026-09-24, when a compressed texture had one 2D level) */
                glCompressedTexImage2D(face, 0, GL_ETC1_RGB8_OES, 4, 4, 0, 8, ptr(&blocks));
                assert_eq!(gl.error(), GL_NO_ERROR, "cube face, face {f}");
            }
        }
        assert_eq!(gl.ctx().objs().textures[cube as usize].faces_set, 0x3f);
    }

    /// ETC1 `n` blocks, every byte `v`.
    fn etc1(n: usize, v: u8) -> Vec<u8> {
        std::vec![v; n * 8]
    }

    fn compressed_level(target: GLenum, level: GLint, format: GLenum, side: GLsizei, data: &[u8]) {
        // SAFETY: `data` holds imageSize bytes.
        unsafe { glCompressedTexImage2D(target, level, format, side, side, 0, data.len() as GLsizei, ptr(data)) };
    }

    /// dEQP-GLES2 texture.filtering.2d.*_etc1 and format.etc1_2d_pot upload
    /// an ETC1 chain one glCompressedTexImage2D per level; every level above
    /// 0 was GL_INVALID_OPERATION until 2026-09-24. On the GC7000 each level
    /// is its own block rows, on the 64-byte boundary the descriptor's
    /// LOD addresses name (vivante/texture.rs build_descriptor).
    #[test]
    fn an_etc1_mip_chain_is_accepted_level_by_level_and_is_complete() {
        let gl = TestGl::new(8, 8);
        gl.backend_like_gc7000();
        let name = new_tex();
        /* 16x16: 4x4 blocks, 2x2, then one block each for 4x4, 2x2, 1x1 */
        let sizes = [(16, 16usize), (8, 4), (4, 1), (2, 1), (1, 1)];
        for (l, &(side, n)) in sizes.iter().enumerate() {
            compressed_level(GL_TEXTURE_2D, l as GLint, GL_ETC1_RGB8_OES, side, &etc1(n, l as u8 + 1));
            assert_eq!(gl.error(), GL_NO_ERROR, "level {l}");
            let t = &gl.ctx().objs_ref().textures[name as usize];
            let complete = draw::texture_bind_2d(t).incomplete == 0;
            assert_eq!(complete, l == 4, "the default mipmap filter wants the whole chain (level {l})");
        }
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.chain_levels, t.levels, t.levels_written), (5, 5, 5));
        assert_eq!((t.level_defined, t.level_bad), (0x1f, 0));
        assert_eq!(t.level_off[..5], [0, 128, 192, 256, 320]);
        assert_eq!(t.mem.size, 328);
        let s = storage(&gl, name);
        for (l, &(_, n)) in sizes.iter().enumerate() {
            let o = t.level_off[l] as usize;
            assert_eq!(s[o..o + n * 8], etc1(n, l as u8 + 1)[..], "level {l}");
        }
        let (_, _, mip_filter, mip_levels) = draw::sampling_fields(t);
        assert_eq!((mip_filter, mip_levels), (2, 5));
    }

    /// The twiddled layout keeps the chain too: level 1 of an 8x8 texture
    /// (one block) right after level 0's 2x2 grid.
    #[test]
    fn a_twiddled_etc1_level_lands_after_level_0() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        compressed_level(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 8, &etc1(4, 1));
        compressed_level(GL_TEXTURE_2D, 1, GL_ETC1_RGB8_OES, 4, &etc1(1, 2));
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.level_off[1], t.levels), (32, 2));
        assert_eq!(storage(&gl, name)[32..40], [2u8; 8]);
    }

    /// A level that does not fit the chain is recorded, as glTexImage2D
    /// records one (ES 2.0 3.7.10): no error, no storage, incomplete while it
    /// stands; the right image clears it.
    #[test]
    fn an_etc1_level_that_does_not_fit_is_recorded_not_refused() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        compressed_level(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 16, &etc1(16, 1));
        let bad = |gl: &TestGl| gl.ctx().objs_ref().textures[name as usize].level_bad;
        compressed_level(GL_TEXTURE_2D, 1, GL_ETC1_RGB8_OES, 4, &etc1(1, 2));
        assert_eq!((gl.error(), bad(&gl)), (GL_NO_ERROR, 0b10), "the wrong size");
        compressed_level(GL_TEXTURE_2D, 1, GL_COMPRESSED_RGBA8_ETC2_EAC, 8, &etc1(8, 2));
        assert_eq!((gl.error(), bad(&gl)), (GL_NO_ERROR, 0b10), "another format");
        compressed_level(GL_TEXTURE_2D, 1, GL_ETC1_RGB8_OES, 8, &etc1(4, 2));
        assert_eq!((gl.error(), bad(&gl)), (GL_NO_ERROR, 0));
        /* an uncompressed level over the block chain is another format too */
        tex_image(2, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &ramp(16));
        assert_eq!((gl.error(), bad(&gl)), (GL_NO_ERROR, 0b100));
        /* a 1x1 past the chain is ignored */
        compressed_level(GL_TEXTURE_2D, 5, GL_ETC1_RGB8_OES, 1, &etc1(1, 3));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((gl.error(), t.level_defined & 0x20), (GL_NO_ERROR, 0));
    }

    /// dEQP-GLES2 texture.format.etc1_cube_* and filtering.cube.*_etc1: six
    /// faces, each with its chain. A level holds its six faces back to back
    /// (level-major, the layout the GC7000 descriptor walks).
    #[test]
    fn etc1_cube_faces_and_levels_are_stored_level_by_level() {
        let gl = TestGl::new(8, 8);
        gl.backend_like_gc7000();
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        /* 8x8 faces: 2x2 blocks, then one block for each of 4x4, 2x2, 1x1 */
        for l in 0..4u32 {
            let (side, n) = if l == 0 { (8, 4) } else { (8 >> l, 1) };
            for f in 0..6u32 {
                let v = (l * 6 + f + 1) as u8;
                compressed_level(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, l as GLint, GL_ETC1_RGB8_OES, side, &etc1(n, v));
                assert_eq!(gl.error(), GL_NO_ERROR, "level {l} face {f}");
            }
        }
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.faces_set, t.layers, t.levels, t.level_defined, t.level_bad), (0x3f, 6, 4, 0xf, 0));
        /* level 0 is 6 x 32 bytes; each later level 6 x 8, on 64 bytes */
        assert_eq!(t.level_off[..4], [0, 192, 256, 320]);
        assert_eq!(draw::texture_bind_2d(t).incomplete, 0);
        let s = storage(&gl, cube);
        for l in 0..4u32 {
            let face = if l == 0 { 32 } else { 8 };
            for f in 0..6u32 {
                let o = t.level_off[l as usize] as usize + f as usize * face;
                assert_eq!(s[o..o + face], std::vec![(l * 6 + f + 1) as u8; face][..], "level {l} face {f}");
            }
        }
        /* a sub-image reaches the face and level it names */
        // SAFETY: one 8-byte block.
        unsafe {
            glCompressedTexSubImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
                1,
                0,
                0,
                4,
                4,
                GL_ETC1_RGB8_OES,
                8,
                ptr(&[0x99u8; 8]),
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let o = 192 + 5 * 8;
        assert_eq!(storage(&gl, cube)[o..o + 8], [0x99u8; 8]);
        assert_eq!(storage(&gl, cube)[o - 8..o], [11u8; 8], "face 4 of level 1 untouched");
    }

    /// glCompressedTexSubImage2D into a level: its own size bounds the
    /// rectangle, and a level with no image is GL_INVALID_OPERATION.
    #[test]
    fn compressed_sub_image_takes_a_level_at_its_own_size() {
        let gl = TestGl::new(8, 8);
        gl.backend_like_gc7000();
        let name = new_tex();
        compressed_level(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 16, &etc1(16, 1));
        let b = [0x99u8; 32];
        // SAFETY: `b` holds up to four blocks; the failing calls read nothing.
        unsafe {
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, 4, 4, GL_ETC1_RGB8_OES, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "level 1 has no image yet");
            compressed_level(GL_TEXTURE_2D, 1, GL_ETC1_RGB8_OES, 8, &etc1(4, 2));
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 1, 4, 4, 4, 4, GL_ETC1_RGB8_OES, 8, ptr(&b));
            assert_eq!(gl.error(), GL_NO_ERROR);
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 1, 8, 0, 4, 4, GL_ETC1_RGB8_OES, 8, ptr(&b));
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past level 1's 8 texels");
        }
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let o = t.level_off[1] as usize;
        /* block (1,1) of a 2-block-wide level is its fourth */
        assert_eq!(storage(&gl, name)[o..o + 32], [[2u8; 24].as_slice(), &[0x99; 8]].concat());
    }

    /* ---- cube map completeness, face by face: dEQP-GLES2 texture.completeness.cube.* ---- */

    /// dEQP's face order (s_cubeTargets): -X, +X, -Y, +Y, -Z, +Z.
    const DEQP_FACES: [GLenum; 6] = [
        GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
        GL_TEXTURE_CUBE_MAP_POSITIVE_X,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    ];

    /// An 8x8 cube map's whole chain, as dEQP's IncompleteCube*Case builds
    /// one: level by level, the six faces in dEQP's order; `spec(face,
    /// level)` gives each image's side and format. The error of each call is
    /// not looked at (with GL_OES_texture_npot listed, a NPOT level above 0
    /// is GL_NO_ERROR, which is what dEQP expects). True when complete.
    fn cube_chain_is_complete(gl: &TestGl, spec: impl Fn(usize, i32) -> (GLsizei, GLenum)) -> bool {
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        let px = solid(64, 9);
        for level in 0..4 {
            for (f, &target) in DEQP_FACES.iter().enumerate() {
                let (side, format) = spec(f, level);
                // SAFETY: `px` holds 8x8 texels of 4 bytes, more than any image here reads.
                unsafe {
                    glTexImage2D(target, level, format as GLint, side, side, 0, format, GL_UNSIGNED_BYTE, ptr(&px))
                };
                gl.error();
            }
        }
        draw::texture_bind_2d(&gl.ctx().objs_ref().textures[cube as usize]).incomplete == 0
    }

    #[test]
    fn a_whole_cube_chain_is_complete() {
        let gl = TestGl::new(8, 8);
        assert!(cube_chain_is_complete(&gl, |_, l| (8 >> l, GL_RGBA)));
    }

    /// dEQP random_teximage2d.2d_5 and 2d_7 (AM62P): a non-square chain's
    /// first upload is a level whose short side is already 1, which reads
    /// as a square chain; the next level shows the real shape, and the
    /// chain is laid out again keeping the level that fits it. The orders
    /// are the qpa's: 32x4 levels 5, 2, 4, 3, 1, 0 and 256x64 levels 8, 5,
    /// 2, 7, 0, 6, 1, 4, 3.
    #[test]
    fn a_non_square_chain_whose_first_level_has_a_side_of_1_is_complete() {
        for (w, h, order) in [(32, 4, &[5, 2, 4, 3, 1, 0][..]), (256, 64, &[8, 5, 2, 7, 0, 6, 1, 4, 3][..])] {
            let gl = TestGl::new(8, 8);
            let name = new_tex();
            for &level in order {
                let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
                tex_image(level, lw, lh, GL_RGBA, GL_UNSIGNED_BYTE, &solid((lw * lh) as usize, level as u8 + 1));
                assert_eq!(gl.error(), GL_NO_ERROR, "{w}x{h} level {level}");
            }
            let t = &gl.ctx().objs_ref().textures[name as usize];
            let levels = order.len() as u32;
            assert_eq!((t.width, t.height), (w as u32, h as u32));
            assert_eq!((t.level_defined, t.level_bad), (((1u32 << levels) - 1) as u16, 0), "{w}x{h}");
            assert_eq!(draw::texture_bind_2d(t).incomplete, 0, "{w}x{h}: complete");
            let s = storage(&gl, name);
            for l in 0..levels {
                let o = t.level_off[l as usize] as usize;
                assert_eq!(s[o..o + 4], [l as u8 + 1; 4], "{w}x{h}: level {l}'s texels");
            }
        }
    }

    /// dEQP-GLES2/3 texture.specification.random_teximage2d uploads a 2D
    /// chain's levels in random order: levels uploaded before level 0 are
    /// kept (the chain is laid out from the first of them), and a level 0 of
    /// the same size and format keeps the levels already there. Both were
    /// lost until 2026-09-24 (AM62P: every failing case uploaded a level
    /// above 0 first). A level 0 of another size still lays the chain out
    /// again.
    #[test]
    fn a_2d_chain_uploaded_out_of_order_is_complete() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        for (level, side, v) in [(2, 2, 3u8), (1, 4, 2), (0, 8, 1), (3, 1, 4)] {
            tex_image(level, side, side, GL_RGBA, GL_UNSIGNED_BYTE, &solid((side * side) as usize, v));
            assert_eq!(gl.error(), GL_NO_ERROR, "level {level}");
        }
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.level_defined, t.level_bad, t.levels), (8, 0xf, 0, 4));
        assert_eq!(draw::texture_bind_2d(t).incomplete, 0, "every level present, mipmap filter");
        let s = storage(&gl, name);
        for (level, v) in [(1usize, 2u8), (2, 3), (3, 4)] {
            let o = t.level_off[level] as usize;
            assert_eq!(s[o..o + 4], [v; 4], "level {level}'s texels kept");
        }
        /* level 0 again at the same size keeps the chain */
        tex_image(0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, &solid(64, 5));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.level_defined, draw::texture_bind_2d(t).incomplete), (0xf, 0));
        /* at another size it starts a new chain: incomplete until filled */
        tex_image(0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, &solid(16, 6));
        let t = &gl.ctx().objs_ref().textures[name as usize];
        assert_eq!((t.width, t.level_defined, draw::texture_bind_2d(t).incomplete), (4, 1, 1));
    }

    /// dEQP random_teximage2d.cube_*: a cube's first upload is a level above
    /// 0 of some face, then the rest in random order. The first lays out the
    /// cube's chain (level 0 unspecified until it comes); every level of every
    /// face is kept. They were lost until 2026-09-24 (AM62P cube_1/2/8).
    #[test]
    fn a_cube_uploaded_in_random_order_is_complete() {
        let gl = TestGl::new(8, 8);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        let px = solid(64, 9);
        /* face 2 levels 2, 1, 0 first, then every other (face, level) in a scrambled order */
        let mut order: Vec<(usize, i32)> = std::vec![(2, 2), (2, 1), (2, 0)];
        for k in 0..24usize {
            let (f, l) = ((k * 5) % 6, ((k * 7) % 4) as i32);
            if f != 2 && !order.contains(&(f, l)) {
                order.push((f, l));
            }
        }
        for f in 0..6 {
            for l in 0..4 {
                if !order.contains(&(f, l)) {
                    order.push((f, l));
                }
            }
        }
        assert_eq!(order.len(), 24);
        for (f, level) in order {
            let side = 8 >> level;
            // SAFETY: `px` holds 8x8 RGBA texels, more than any level reads.
            unsafe {
                glTexImage2D(DEQP_FACES[f], level, GL_RGBA as GLint, side, side, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px))
            };
            assert_eq!(gl.error(), GL_NO_ERROR, "face {f} level {level}");
        }
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.faces_set, t.level_faces[1], t.level_faces[2], t.level_faces[3]), (0x3f, 0x3f, 0x3f, 0x3f));
        assert_eq!(draw::texture_bind_2d(t).incomplete, 0);
    }

    /// dEQP's glu::TextureCube uploads face by face, each face's whole chain
    /// (-X first), so +X's level 0 arrives after -X's levels 1..: it must
    /// not throw them away. The storage was laid out again for every +X
    /// level 0 and -X's per-face record (level_faces, aae1c448) went with
    /// it; AM62P texture.filtering.cube.*_mipmap_* (36) read incomplete.
    /// Before aae1c448 the per-level record was put back by the later
    /// faces, which hid it.
    #[test]
    fn a_cube_uploaded_face_by_face_whole_chains_is_complete() {
        let gl = TestGl::new(8, 8);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        let px = solid(64, 9);
        for &target in DEQP_FACES.iter() {
            for level in 0..4 {
                let side = 8 >> level;
                // SAFETY: `px` holds 8x8 RGBA texels, more than any level reads.
                unsafe {
                    glTexImage2D(target, level, GL_RGBA as GLint, side, side, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px))
                };
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.faces_set, t.level_faces[1], t.level_faces[3]), (0x3f, 0x3f, 0x3f));
        assert_eq!(draw::texture_bind_2d(t).incomplete, 0);
        /* the -X face's level 1 texels are still there (not a cleared re-layout) */
        let (lw, pitch) = (4u32, t.level_pitch(4) as usize);
        let off = t.level_off[1] as usize + lw as usize * pitch * 4; /* -X is face 1: one face of lw rows in */
        assert_eq!(storage(&gl, cube)[off..off + 4], [9u8; 4]);
    }

    /// Level 0 of all six faces, then glGenerateMipmap: complete.
    #[test]
    fn a_cube_with_generated_mipmaps_is_complete() {
        let gl = TestGl::new(8, 8);
        let cube = bind_new(GL_TEXTURE_CUBE_MAP);
        let px = solid(64, 9);
        for &target in DEQP_FACES.iter() {
            // SAFETY: `px` holds the 8x8 RGBA image.
            unsafe { glTexImage2D(target, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr(&px)) };
        }
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(draw::texture_bind_2d(&gl.ctx().objs_ref().textures[cube as usize]).incomplete, 0);
    }

    /// npot_size_level_0_pos_x: +X's level 0 is 7x7 among 8x8 faces. It
    /// is the first face to lay the storage out again, and when the
    /// allocation was reused the record of it survived the next face's
    /// layout: the cube read complete.
    #[test]
    fn a_cube_face_of_another_size_at_level_0_leaves_it_incomplete() {
        let gl = TestGl::new(8, 8);
        let spec = |f, l| (if f == 1 && l == 0 { 7 } else { 8 >> l }, GL_RGBA);
        assert!(!cube_chain_is_complete(&gl, spec));
    }

    /// format_mismatch_rgba_rgb_level_0_neg_z: an RGB -Z level 0 among
    /// RGBA faces. RGB and RGBA share the stored texel format, so the face
    /// was no difference to the layout and the last face's RGBA hid it.
    #[test]
    fn a_cube_face_of_another_format_at_level_0_leaves_it_incomplete() {
        let gl = TestGl::new(8, 8);
        let spec = |f, l| (8 >> l, if f == 4 && l == 0 { GL_RGB } else { GL_RGBA });
        assert!(!cube_chain_is_complete(&gl, spec));
    }

    /// npot_size_level_1_neg_x: -X's level 1 is 3x3 where the chain has
    /// 4x4. Refused (ES 2.0 reporting) or recorded as not fitting, the
    /// other five faces' level 1 must not make the level whole: the record
    /// is per face (`level_faces`).
    #[test]
    fn a_cube_face_missing_or_misfit_at_level_1_leaves_it_incomplete() {
        let gl = TestGl::new(8, 8);
        let spec = |f, l| (if f == 0 && l == 1 { 3 } else { 8 >> l }, GL_RGBA);
        assert!(!cube_chain_is_complete(&gl, spec));
        /* the face given its right image makes it whole again */
        let px = solid(16, 9);
        // SAFETY: `px` holds the 4x4 image.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
                1,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr(&px),
            )
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let cube = gl.ctx().bound_texture_cube[0];
        assert_eq!(draw::texture_bind_2d(&gl.ctx().objs_ref().textures[cube as usize]).incomplete, 0);
    }

    /// `generatemipmap_incomplete_cube`: a cube map needs all six faces at one
    /// size. A face respecified at another size lays the storage out again,
    /// which is what clears the record.
    #[test]
    fn generate_mipmap_needs_every_cube_face() {
        let gl = TestGl::new(8, 8);
        bind_new(GL_TEXTURE_CUBE_MAP);
        let px = solid(16, 7);
        for f in 0..5u32 {
            // SAFETY: `px` holds the 2x2 RGBA image.
            unsafe {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    2,
                    2,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    ptr(&px),
                )
            };
            assert_eq!(gl.error(), GL_NO_ERROR, "face {f}");
        }
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "five faces of six");

        /* the sixth face completes it */
        // SAFETY: `px` holds the 2x2 RGBA image.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
                0,
                GL_RGBA as GLint,
                2,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr(&px),
            );
            glGenerateMipmap(GL_TEXTURE_CUBE_MAP);
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "six faces at one size");
    }

    /// Immutable storage defines all six faces at once, so a cube map laid out
    /// by glTexStorage2D is complete before any texel is uploaded.
    #[test]
    fn generate_mipmap_accepts_an_immutable_cube_map() {
        let gl = TestGl::new(8, 8);
        bind_new(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage2D(GL_TEXTURE_CUBE_MAP, 2, GL_RGBA8, 2, 2);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glGenerateMipmap(GL_TEXTURE_CUBE_MAP);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// `generatemipmap_npot_wdt_hgt`: ES 2.0 without GL_OES_texture_npot has
    /// no mip chain for a non-power-of-two image, so asking for one is an
    /// error (NPOT_RESTRICTED). ES 3.0 dropped the rule, and the ES 2.0
    /// surface claims GL_OES_texture_npot, which drops it too (what dEQP's
    /// case is predicted to do then: docs/es2-cts-failures.md).
    #[test]
    fn generate_mipmap_of_a_npot_texture_follows_the_reported_version() {
        let gl = TestGl::new(8, 8);
        new_tex();
        for (w, h) in [(4, 5), (5, 4), (5, 5)] {
            tex_image(0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, &[]);
            assert_eq!(gl.error(), GL_NO_ERROR, "{w}x{h}");
            // SAFETY: plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            let want = if NPOT_RESTRICTED { GL_INVALID_OPERATION } else { GL_NO_ERROR };
            assert_eq!(gl.error(), want, "{w}x{h}");
        }
    }

    /// Re-uploading one face of a complete cube map at the same size leaves
    /// it complete. The storage is kept when the size and format match, so
    /// the other five faces' texels are still there and their record must
    /// stand -- an application refreshing one face of a dynamic environment
    /// map every frame is doing nothing wrong.
    #[test]
    fn respecifying_one_face_of_a_cube_map_keeps_the_others() {
        let gl = TestGl::new(8, 8);
        bind_new(GL_TEXTURE_CUBE_MAP);
        let px = solid(4, 3);
        let face_image = |f: u32, v: u8| {
            let px = solid(4, v);
            // SAFETY: `px` holds the 2x2 RGBA face image.
            unsafe {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    2,
                    2,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    ptr(&px),
                )
            };
        };
        let _ = px;
        for f in 0..6u32 {
            face_image(f, 10 * f as u8);
        }
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        assert_eq!(gl.error(), GL_NO_ERROR, "six faces at one size: complete");

        /* face 0 again, same size and format */
        face_image(0, 99);
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        assert_eq!(gl.error(), GL_NO_ERROR, "the other five faces are still specified");

        /* and they really are still there: face 5's texels survived */
        let name = gl.ctx().bound_texture_cube[0];
        let t = &gl.ctx().objs_ref().textures[name as usize];
        let (w, h) = (t.width, t.height);
        assert_eq!((w, h), (2, 2));
        let s = storage(&gl, name);
        let pitch = t.level_pitch(w) as usize * 4;
        assert_eq!(s[5 * h as usize * pitch], 50, "face 5 keeps its texels");
        assert_eq!(s[0], 99, "face 0 has the new ones");

        /* a face at a DIFFERENT size does lay the storage out again, and the
         * cube map is incomplete until every face is given the new size */
        // SAFETY: `px` holds the 4x4 RGBA face image.
        unsafe {
            let px = solid(16, 7);
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_POSITIVE_X,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr(&px),
            );
            glGenerateMipmap(GL_TEXTURE_CUBE_MAP);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "one face at 4x4, five at nothing");
    }

    /* ---- GL_EXT_texture_format_BGRA8888 ----------------------------------- */

    /// The upload swaps red and blue once, so the stored texel is the colour
    /// the application meant and everything downstream is ordinary RGBA8.
    #[test]
    fn a_bgra_upload_is_stored_as_the_colour_it_names() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        /* one texel, B=1 G=2 R=3 A=4 */
        let px: [u8; 4] = [1, 2, 3, 4];
        // SAFETY: `px` holds the 1x1 BGRA image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_BGRA_EXT as GLint, 1, 1, 0, GL_BGRA_EXT, GL_UNSIGNED_BYTE, ptr(&px))
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..4], [3, 2, 1, 4], "R,G,B,A: red and blue swapped");

        /* glTexSubImage2D takes it too, and swaps the same way */
        let px2: [u8; 4] = [9, 8, 7, 6];
        // SAFETY: `px2` holds the 1x1 BGRA image.
        unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_BGRA_EXT, GL_UNSIGNED_BYTE, ptr(&px2)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(storage(&gl, name)[..4], [7, 8, 9, 6]);
    }

    /// The errors GL ES 2.0 names still apply to it: an unsized internal
    /// format must equal the format, and the extension defines BGRA with
    /// GL_UNSIGNED_BYTE only.
    #[test]
    fn bgra_keeps_the_internalformat_and_type_rules() {
        let gl = TestGl::new(8, 8);
        new_tex();
        // SAFETY: plain GL calls, null image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 1, 1, 0, GL_BGRA_EXT, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "internalformat must match the format");
            glTexImage2D(GL_TEXTURE_2D, 0, GL_BGRA_EXT as GLint, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "and the other way round");
            glTexImage2D(GL_TEXTURE_2D, 0, GL_BGRA_EXT as GLint, 1, 1, 0, GL_BGRA_EXT, GL_FLOAT, core::ptr::null());
            /* the ES 2.0 surface has no GL_FLOAT type at all (`es3_only_pixel_type`) */
            let want = if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_INVALID_OPERATION };
            assert_eq!(gl.error(), want, "the extension has no float BGRA");
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_BGRA_EXT as GLint,
                1,
                1,
                0,
                GL_BGRA_EXT,
                GL_UNSIGNED_SHORT_4_4_4_4,
                core::ptr::null(),
            );
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "nor a packed one");
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_BGRA_EXT as GLint,
                1,
                1,
                0,
                GL_BGRA_EXT,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            assert_eq!(gl.error(), GL_NO_ERROR, "the pair the extension defines");
        }
    }

    /// A BGRA texture is a colour attachment like any other: its store is
    /// RGBA8, so the framebuffer is complete and the PBE writes it natively.
    #[test]
    fn a_bgra_texture_is_a_complete_colour_attachment() {
        let gl = TestGl::new(8, 8);
        let name = new_tex();
        let px = std::vec![0u8; 4 * 4 * 4];
        let mut fb: GLuint = 0;
        // SAFETY: room for one name; `px` holds the 4x4 BGRA image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_BGRA_EXT as GLint, 4, 4, 0, GL_BGRA_EXT, GL_UNSIGNED_BYTE, ptr(&px));
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        let status = unsafe { crate::gl::framebuffer::glCheckFramebufferStatus(GL_FRAMEBUFFER) };
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "a BGRA texture is renderable");
    }
}
