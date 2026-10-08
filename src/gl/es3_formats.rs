// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! OpenGL ES 3.0's sized colour texture formats that `texture.rs`' original
//! tables (RGBA8 and the float trio RGBA16F / R16F / R32F) do not hold:
//! the signed normalised, packed-float, shared-exponent, 10-bit and integer
//! ones (ES 3.0.6 table 3.2), the pure functions that decide which
//! (internalformat, format, type) combinations are valid, which storage class
//! a back end can sample them from, and the CPU conversion of an upload into
//! that storage.
//!
//! Nothing here touches a context or a back end: the caller hands in the
//! mask of texel formats its back end samples (`backend::sampled_texfmts`),
//! so the host tests can ask the same question of every GPU. Under
//! `report-es2` [`find`] knows no format at all, which keeps the ES 2.0
//! surface exactly as it was.
//!
//! Storage. Each format lists the storage classes it can live in, best
//! first. The first is the class a sampler reads natively (the new
//! `GLES_TEXFMT_*` of `backend.rs`); a lossless or near-lossless fallback in
//! a class every back end already knows follows where there is one (packed
//! float to RGBA16F is exact, 10-bit to RGBA8 loses two bits). The integer
//! formats have no fallback: an integer cannot be a float texel, so a back end
//! that does not sample the class refuses the format by name
//! ([`Es3Format::name`]) rather than storing texels it would read as colours.
//!
//! The CPU conversion decodes the application's (format, type) texel into
//! four components, floats or raw integers, and encodes them in the storage
//! class. Components the format does not have read as (0, 0, 0, 1), the
//! base-format rule of ES 3.0 3.8.3.

use super::consts::*;
use super::types::*;
use crate::backend::*;

/// What a format's components are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// Unsigned normalised (RGB10_A2).
    Unorm,
    /// Signed normalised, -1..1 (the *_SNORM formats).
    Snorm,
    /// Floating point, of whatever width.
    Float,
    /// Signed integer.
    Int,
    /// Unsigned integer.
    Uint,
}

/// One ES 3.0 sized colour format this module owns.
pub struct Es3Format {
    pub internalformat: GLenum,
    pub name: &'static str,
    /// The base format: GL_RED .. GL_RGBA, or its *_INTEGER twin.
    pub base: GLenum,
    pub class: Class,
    /// Components the format has (1..=4).
    pub comps: u8,
    /// glTexImage's `type` values table 3.2 pairs with it; the first is the
    /// format's own (the one glTexStorage and glCopyTexImage unpack as).
    pub types: &'static [GLenum],
    /// Storage classes, best first (`GLES_TEXFMT_*`).
    pub storage: &'static [u8],
    /// 32-bit float components: not filterable in ES 3.0 core.
    pub f32_texels: bool,
}

const fn f(
    internalformat: GLenum,
    name: &'static str,
    base: GLenum,
    class: Class,
    comps: u8,
    types: &'static [GLenum],
    storage: &'static [u8],
) -> Es3Format {
    Es3Format { internalformat, name, base, class, comps, types, storage, f32_texels: false }
}

const SNORM_ST: &[u8] = &[GLES_TEXFMT_RGBA8_SNORM, GLES_TEXFMT_RGBA16F];
const F16_ST: &[u8] = &[GLES_TEXFMT_RGBA16F];
const F32_ST: &[u8] = &[GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA16F];
const U8_ST: &[u8] = &[GLES_TEXFMT_RGBA8UI];
const I8_ST: &[u8] = &[GLES_TEXFMT_RGBA8I];
const U16_ST: &[u8] = &[GLES_TEXFMT_RGBA16UI];
const I16_ST: &[u8] = &[GLES_TEXFMT_RGBA16I];
const U32_ST: &[u8] = &[GLES_TEXFMT_RGBA32UI];
const I32_ST: &[u8] = &[GLES_TEXFMT_RGBA32I];

/// The formats, in `find`'s index order (index + 1 is what a texture records).
pub static FORMATS: [Es3Format; 35] = [
    f(GL_R8_SNORM, "GL_R8_SNORM", GL_RED, Class::Snorm, 1, &[GL_BYTE], SNORM_ST),
    f(GL_RG8_SNORM, "GL_RG8_SNORM", GL_RG, Class::Snorm, 2, &[GL_BYTE], SNORM_ST),
    f(GL_RGB8_SNORM, "GL_RGB8_SNORM", GL_RGB, Class::Snorm, 3, &[GL_BYTE], SNORM_ST),
    f(GL_RGBA8_SNORM, "GL_RGBA8_SNORM", GL_RGBA, Class::Snorm, 4, &[GL_BYTE], SNORM_ST),
    f(
        GL_R11F_G11F_B10F,
        "GL_R11F_G11F_B10F",
        GL_RGB,
        Class::Float,
        3,
        &[GL_UNSIGNED_INT_10F_11F_11F_REV, GL_HALF_FLOAT, GL_FLOAT],
        &[GLES_TEXFMT_R11G11B10F, GLES_TEXFMT_RGBA16F],
    ),
    f(
        GL_RGB9_E5,
        "GL_RGB9_E5",
        GL_RGB,
        Class::Float,
        3,
        &[GL_UNSIGNED_INT_5_9_9_9_REV, GL_HALF_FLOAT, GL_FLOAT],
        &[GLES_TEXFMT_RGB9E5, GLES_TEXFMT_RGBA16F],
    ),
    f(GL_RGB16F, "GL_RGB16F", GL_RGB, Class::Float, 3, &[GL_HALF_FLOAT, GL_FLOAT], F16_ST),
    Es3Format { f32_texels: true, ..f(GL_RG32F, "GL_RG32F", GL_RG, Class::Float, 2, &[GL_FLOAT], F32_ST) },
    Es3Format { f32_texels: true, ..f(GL_RGB32F, "GL_RGB32F", GL_RGB, Class::Float, 3, &[GL_FLOAT], F32_ST) },
    f(
        GL_RGB10_A2,
        "GL_RGB10_A2",
        GL_RGBA,
        Class::Unorm,
        4,
        &[GL_UNSIGNED_INT_2_10_10_10_REV],
        &[GLES_TEXFMT_RGB10A2, GLES_TEXFMT_RGBA8],
    ),
    f(
        GL_RGB10_A2UI,
        "GL_RGB10_A2UI",
        GL_RGBA_INTEGER,
        Class::Uint,
        4,
        &[GL_UNSIGNED_INT_2_10_10_10_REV],
        &[GLES_TEXFMT_RGB10A2UI],
    ),
    f(GL_R8UI, "GL_R8UI", GL_RED_INTEGER, Class::Uint, 1, &[GL_UNSIGNED_BYTE], U8_ST),
    f(GL_R8I, "GL_R8I", GL_RED_INTEGER, Class::Int, 1, &[GL_BYTE], I8_ST),
    f(GL_R16UI, "GL_R16UI", GL_RED_INTEGER, Class::Uint, 1, &[GL_UNSIGNED_SHORT], U16_ST),
    f(GL_R16I, "GL_R16I", GL_RED_INTEGER, Class::Int, 1, &[GL_SHORT], I16_ST),
    f(GL_R32UI, "GL_R32UI", GL_RED_INTEGER, Class::Uint, 1, &[GL_UNSIGNED_INT], U32_ST),
    f(GL_R32I, "GL_R32I", GL_RED_INTEGER, Class::Int, 1, &[GL_INT], I32_ST),
    f(GL_RG8UI, "GL_RG8UI", GL_RG_INTEGER, Class::Uint, 2, &[GL_UNSIGNED_BYTE], U8_ST),
    f(GL_RG8I, "GL_RG8I", GL_RG_INTEGER, Class::Int, 2, &[GL_BYTE], I8_ST),
    f(GL_RG16UI, "GL_RG16UI", GL_RG_INTEGER, Class::Uint, 2, &[GL_UNSIGNED_SHORT], U16_ST),
    f(GL_RG16I, "GL_RG16I", GL_RG_INTEGER, Class::Int, 2, &[GL_SHORT], I16_ST),
    f(GL_RG32UI, "GL_RG32UI", GL_RG_INTEGER, Class::Uint, 2, &[GL_UNSIGNED_INT], U32_ST),
    f(GL_RG32I, "GL_RG32I", GL_RG_INTEGER, Class::Int, 2, &[GL_INT], I32_ST),
    f(GL_RGB8UI, "GL_RGB8UI", GL_RGB_INTEGER, Class::Uint, 3, &[GL_UNSIGNED_BYTE], U8_ST),
    f(GL_RGB8I, "GL_RGB8I", GL_RGB_INTEGER, Class::Int, 3, &[GL_BYTE], I8_ST),
    f(GL_RGB16UI, "GL_RGB16UI", GL_RGB_INTEGER, Class::Uint, 3, &[GL_UNSIGNED_SHORT], U16_ST),
    f(GL_RGB16I, "GL_RGB16I", GL_RGB_INTEGER, Class::Int, 3, &[GL_SHORT], I16_ST),
    f(GL_RGB32UI, "GL_RGB32UI", GL_RGB_INTEGER, Class::Uint, 3, &[GL_UNSIGNED_INT], U32_ST),
    f(GL_RGB32I, "GL_RGB32I", GL_RGB_INTEGER, Class::Int, 3, &[GL_INT], I32_ST),
    f(GL_RGBA8UI, "GL_RGBA8UI", GL_RGBA_INTEGER, Class::Uint, 4, &[GL_UNSIGNED_BYTE], U8_ST),
    f(GL_RGBA8I, "GL_RGBA8I", GL_RGBA_INTEGER, Class::Int, 4, &[GL_BYTE], I8_ST),
    f(GL_RGBA16UI, "GL_RGBA16UI", GL_RGBA_INTEGER, Class::Uint, 4, &[GL_UNSIGNED_SHORT], U16_ST),
    f(GL_RGBA16I, "GL_RGBA16I", GL_RGBA_INTEGER, Class::Int, 4, &[GL_SHORT], I16_ST),
    f(GL_RGBA32UI, "GL_RGBA32UI", GL_RGBA_INTEGER, Class::Uint, 4, &[GL_UNSIGNED_INT], U32_ST),
    f(GL_RGBA32I, "GL_RGBA32I", GL_RGBA_INTEGER, Class::Int, 4, &[GL_INT], I32_ST),
];

/// The record of `internalformat`: (index + 1, row). None for a name this
/// module does not own, always under `report-es2`.
pub fn find(internalformat: GLenum) -> Option<(u8, &'static Es3Format)> {
    if crate::REPORT_ES2 {
        return None;
    }
    FORMATS.iter().position(|e| e.internalformat == internalformat).map(|i| (i as u8 + 1, &FORMATS[i]))
}

/// The row a texture recorded (`GlTexture::es3_fmt`), 0 being none.
pub fn by_index(idx: u8) -> Option<&'static Es3Format> {
    (idx as usize).checked_sub(1).and_then(|i| FORMATS.get(i))
}

impl Es3Format {
    /// Is (format, type) an upload table 3.2 pairs with this internal format?
    pub fn accepts(&self, format: GLenum, type_: GLenum) -> bool {
        format == self.base && self.types.contains(&type_)
    }

    /// The first storage class of the format that `sampled` (a mask of
    /// `gles_texfmt_bit`) has; None when the back end samples none.
    pub fn storage_in(&self, sampled: u32) -> Option<u8> {
        self.storage.iter().copied().find(|&s| sampled & gles_texfmt_bit(s) != 0)
    }

    /// The storage class on this build's back end.
    pub fn storage_here(&self) -> Option<u8> {
        self.storage_in(sampled_texfmts())
    }

    pub fn is_integer(&self) -> bool {
        matches!(self.class, Class::Int | Class::Uint)
    }

    /// May a sampler filter it? ES 3.0 3.8.13: an integer texture is
    /// complete only with NEAREST magnification and a NEAREST or
    /// NEAREST_MIPMAP_NEAREST minification; so is one of 32-bit float texels
    /// (no OES_texture_float_linear in core).
    pub fn filterable(&self) -> bool {
        !self.is_integer() && !self.f32_texels
    }

    /// Does the sampler state leave a texture of this format incomplete?
    pub fn incomplete_with(&self, min_filter: GLenum, mag_filter: GLenum) -> bool {
        !self.filterable()
            && (mag_filter != GL_NEAREST || !matches!(min_filter, GL_NEAREST | GL_NEAREST_MIPMAP_NEAREST))
    }
}

/// Components `format` carries (colour formats only).
fn format_comps(format: GLenum) -> usize {
    match format {
        GL_RGBA | GL_RGBA_INTEGER => 4,
        GL_RGB | GL_RGB_INTEGER => 3,
        GL_RG | GL_RG_INTEGER => 2,
        _ => 1,
    }
}

/// Is `format` one of the *_INTEGER bases?
pub fn is_integer_format(format: GLenum) -> bool {
    matches!(format, GL_RED_INTEGER | GL_RG_INTEGER | GL_RGB_INTEGER | GL_RGBA_INTEGER)
}

/// Is (format, type) a colour transfer pair of ES 3.0 table 3.2 (any
/// internal format)?
pub fn transfer_pair_valid(format: GLenum, type_: GLenum) -> bool {
    if crate::REPORT_ES2 {
        return false;
    }
    match format {
        GL_RGBA => matches!(
            type_,
            GL_UNSIGNED_BYTE
                | GL_BYTE
                | GL_UNSIGNED_SHORT_4_4_4_4
                | GL_UNSIGNED_SHORT_5_5_5_1
                | GL_UNSIGNED_INT_2_10_10_10_REV
                | GL_HALF_FLOAT
                | GL_FLOAT
        ),
        GL_RGB => matches!(
            type_,
            GL_UNSIGNED_BYTE
                | GL_BYTE
                | GL_UNSIGNED_SHORT_5_6_5
                | GL_UNSIGNED_INT_10F_11F_11F_REV
                | GL_UNSIGNED_INT_5_9_9_9_REV
                | GL_HALF_FLOAT
                | GL_FLOAT
        ),
        GL_RG | GL_RED => matches!(type_, GL_UNSIGNED_BYTE | GL_BYTE | GL_HALF_FLOAT | GL_FLOAT),
        GL_RGBA_INTEGER => matches!(
            type_,
            GL_UNSIGNED_BYTE
                | GL_BYTE
                | GL_UNSIGNED_SHORT
                | GL_SHORT
                | GL_UNSIGNED_INT
                | GL_INT
                | GL_UNSIGNED_INT_2_10_10_10_REV
        ),
        GL_RGB_INTEGER | GL_RG_INTEGER | GL_RED_INTEGER => {
            matches!(type_, GL_UNSIGNED_BYTE | GL_BYTE | GL_UNSIGNED_SHORT | GL_SHORT | GL_UNSIGNED_INT | GL_INT)
        }
        _ => false,
    }
}

/// Is (format, type) a pair only the formats of [`FORMATS`] take (an integer
/// base, or a type the original tables do not unpack)? Such an upload into a
/// texture of any other format is GL_INVALID_OPERATION.
pub fn only_for_new_formats(format: GLenum, type_: GLenum) -> bool {
    if crate::REPORT_ES2 {
        return false;
    }
    is_integer_format(format)
        || matches!(
            type_,
            GL_BYTE
                | GL_SHORT
                | GL_UNSIGNED_SHORT
                | GL_INT
                | GL_UNSIGNED_INT
                | GL_UNSIGNED_INT_2_10_10_10_REV
                | GL_UNSIGNED_INT_10F_11F_11F_REV
                | GL_UNSIGNED_INT_5_9_9_9_REV
        ) && !matches!(format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
}

/// Source bytes of one texel of a colour (format, type); 0 for a pair this
/// module does not unpack.
pub fn texel_bytes(format: GLenum, type_: GLenum) -> usize {
    match type_ {
        GL_UNSIGNED_SHORT_5_6_5 | GL_UNSIGNED_SHORT_4_4_4_4 | GL_UNSIGNED_SHORT_5_5_5_1 => 2,
        GL_UNSIGNED_INT_2_10_10_10_REV | GL_UNSIGNED_INT_10F_11F_11F_REV | GL_UNSIGNED_INT_5_9_9_9_REV => 4,
        GL_UNSIGNED_BYTE | GL_BYTE => format_comps(format),
        GL_UNSIGNED_SHORT | GL_SHORT | GL_HALF_FLOAT | GL_HALF_FLOAT_OES => 2 * format_comps(format),
        GL_UNSIGNED_INT | GL_INT | GL_FLOAT => 4 * format_comps(format),
        _ => 0,
    }
}

/* ---- the number formats ----------------------------------------------------- */

/// IEEE half -> f32, subnormals included.
pub fn half_to_f32(h: u16) -> f32 {
    let s = ((h & 0x8000) as u32) << 16;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;

    if e == 0 {
        /* m * 2^-24, exactly */
        let v = m as f32 * f32::from_bits(0x3380_0000);
        return f32::from_bits(v.to_bits() | s);
    }
    if e == 31 {
        return f32::from_bits(s | 0x7f80_0000 | (m << 13));
    }
    f32::from_bits(s | ((e + 112) << 23) | (m << 13))
}

/// f32 -> IEEE half, to nearest (ties to even); overflow is infinity.
pub fn f32_to_half(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x7f_ffff;

    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 | (mant >> 13) as u16 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        /* subnormal: the implicit one joins the mantissa */
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut r = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && r & 1 == 1) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (mant >> 13);
    let rem = mant & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && r & 1 == 1) {
        r += 1; /* may carry into the exponent, up to infinity */
    }
    sign | r as u16
}

/// An unsigned small float (5 exponent bits, `mb` mantissa bits) -> f32.
pub fn small_float_to_f32(v: u32, mb: u32) -> f32 {
    let e = (v >> mb) & 0x1f;
    let m = v & ((1 << mb) - 1);

    if e == 0 {
        /* m * 2^(-14 - mb) */
        return m as f32 * f32::from_bits((127 - 14 - mb) << 23);
    }
    if e == 31 {
        return f32::from_bits(0x7f80_0000 | (m << (23 - mb)));
    }
    f32::from_bits(((e + 112) << 23) | (m << (23 - mb)))
}

/// f32 -> unsigned small float: negatives and NaN are 0, overflow clamps to
/// the largest finite value, the rest rounds to nearest (halves up).
pub fn f32_to_small_float(v: f32, mb: u32) -> u32 {
    let bits = v.to_bits();
    let max = (30 << mb) | ((1 << mb) - 1);

    if bits & 0x8000_0000 != 0 || v.is_nan() {
        return 0;
    }
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 0xff {
        return max;
    }
    let e = exp - 127 + 15;
    let x = if e <= 0 {
        let shift = (1 - e) as u32 + 23 - mb;
        if shift > 24 {
            return 0;
        }
        let m = mant | 0x80_0000;
        let r = m >> shift;
        r + ((m >> (shift - 1)) & 1)
    } else {
        let r = ((e as u32) << mb) | (mant >> (23 - mb));
        r + ((mant >> (22 - mb)) & 1)
    };
    x.min(max)
}

/// Pack R11F_G11F_B10F (r in the low bits).
pub fn pack_r11g11b10f(r: f32, g: f32, b: f32) -> u32 {
    f32_to_small_float(r, 6) | (f32_to_small_float(g, 6) << 11) | (f32_to_small_float(b, 5) << 22)
}

pub fn unpack_r11g11b10f(p: u32) -> [f32; 3] {
    [small_float_to_f32(p & 0x7ff, 6), small_float_to_f32((p >> 11) & 0x7ff, 6), small_float_to_f32(p >> 22, 5)]
}

/// 2^k as an f32, k in -126..=127.
fn pow2(k: i32) -> f32 {
    f32::from_bits(((k + 127) as u32) << 23)
}

const RGB9E5_MAX: f32 = 65408.0;

/// Pack RGB9_E5 (ES 3.0 3.8.3's EXT_texture_shared_exponent rule).
pub fn pack_rgb9e5(r: f32, g: f32, b: f32) -> u32 {
    let clamp = |v: f32| if v > 0.0 { v.min(RGB9E5_MAX) } else { 0.0 };
    let (r, g, b) = (clamp(r), clamp(g), clamp(b));
    let max = r.max(g).max(b);
    /* floor(log2(max)), -16 for anything below 2^-16 (and 0) */
    let lg = if max <= 0.0 { -16 } else { (((max.to_bits() >> 23) & 0xff) as i32 - 127).max(-16) };
    let mut shared = lg + 1 + 15;
    let mut denom = pow2(shared - 15 - 9);
    let maxm = (max / denom + 0.5) as u32;
    if maxm == 512 {
        shared += 1;
        denom *= 2.0;
    }
    let m = |c: f32| ((c / denom + 0.5) as u32).min(511);
    m(r) | (m(g) << 9) | (m(b) << 18) | ((shared as u32) << 27)
}

pub fn unpack_rgb9e5(p: u32) -> [f32; 3] {
    let scale = pow2((p >> 27) as i32 - 24);
    [(p & 0x1ff) as f32 * scale, ((p >> 9) & 0x1ff) as f32 * scale, ((p >> 18) & 0x1ff) as f32 * scale]
}

/* ---- decoding the application's texel ---------------------------------------- */

/// A decoded texel: floats for the normalised and float formats, raw integer
/// bit patterns (signed ones sign-extended to 32 bits) for the integer ones.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Texel {
    F([f32; 4]),
    I([u32; 4]),
}

fn u16_at(s: &[u8], o: usize) -> u16 {
    u16::from_ne_bytes([s[o], s[o + 1]])
}
fn u32_at(s: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]])
}

/// Decode one texel of (format, type) from `src` (`texel_bytes` of it).
/// Components the pair lacks are (0, 0, 0, 1). None for a pair not unpacked.
pub fn decode(src: &[u8], format: GLenum, type_: GLenum) -> Option<Texel> {
    let n = format_comps(format);
    let ints = is_integer_format(format);

    if src.len() < texel_bytes(format, type_) || texel_bytes(format, type_) == 0 {
        return None;
    }
    match type_ {
        GL_UNSIGNED_INT_2_10_10_10_REV => {
            let p = u32_at(src, 0);
            let c = [p & 0x3ff, (p >> 10) & 0x3ff, (p >> 20) & 0x3ff, p >> 30];
            return Some(if ints {
                Texel::I(c)
            } else {
                Texel::F([c[0] as f32 / 1023.0, c[1] as f32 / 1023.0, c[2] as f32 / 1023.0, c[3] as f32 / 3.0])
            });
        }
        GL_UNSIGNED_INT_10F_11F_11F_REV => {
            let [r, g, b] = unpack_r11g11b10f(u32_at(src, 0));
            return Some(Texel::F([r, g, b, 1.0]));
        }
        GL_UNSIGNED_INT_5_9_9_9_REV => {
            let [r, g, b] = unpack_rgb9e5(u32_at(src, 0));
            return Some(Texel::F([r, g, b, 1.0]));
        }
        GL_UNSIGNED_SHORT_5_6_5 => {
            let p = u16_at(src, 0) as u32;
            return Some(Texel::F([
                (p >> 11) as f32 / 31.0,
                ((p >> 5) & 0x3f) as f32 / 63.0,
                (p & 0x1f) as f32 / 31.0,
                1.0,
            ]));
        }
        GL_UNSIGNED_SHORT_4_4_4_4 => {
            let p = u16_at(src, 0) as u32;
            return Some(Texel::F([
                (p >> 12) as f32 / 15.0,
                ((p >> 8) & 0xf) as f32 / 15.0,
                ((p >> 4) & 0xf) as f32 / 15.0,
                (p & 0xf) as f32 / 15.0,
            ]));
        }
        GL_UNSIGNED_SHORT_5_5_5_1 => {
            let p = u16_at(src, 0) as u32;
            return Some(Texel::F([
                (p >> 11) as f32 / 31.0,
                ((p >> 6) & 0x1f) as f32 / 31.0,
                ((p >> 1) & 0x1f) as f32 / 31.0,
                (p & 1) as f32,
            ]));
        }
        _ => {}
    }
    if ints {
        let mut v = [0u32, 0, 0, 1];
        for (k, vk) in v.iter_mut().enumerate().take(n) {
            *vk = match type_ {
                GL_UNSIGNED_BYTE => src[k] as u32,
                GL_BYTE => src[k] as i8 as i32 as u32,
                GL_UNSIGNED_SHORT => u16_at(src, k * 2) as u32,
                GL_SHORT => u16_at(src, k * 2) as i16 as i32 as u32,
                _ => u32_at(src, k * 4),
            };
        }
        return Some(Texel::I(v));
    }
    let mut v = [0.0f32, 0.0, 0.0, 1.0];
    for (k, vk) in v.iter_mut().enumerate().take(n) {
        *vk = match type_ {
            GL_UNSIGNED_BYTE => src[k] as f32 / 255.0,
            GL_BYTE => (src[k] as i8 as f32 / 127.0).max(-1.0),
            GL_HALF_FLOAT | GL_HALF_FLOAT_OES => half_to_f32(u16_at(src, k * 2)),
            GL_FLOAT => f32::from_bits(u32_at(src, k * 4)),
            _ => return None,
        };
    }
    Some(Texel::F(v))
}

/// The texel as a format with `comps` components stores it: the others are
/// (0, 0, 0, 1) whatever the source carried.
fn masked(t: Texel, comps: u8, one: u32) -> Texel {
    match t {
        Texel::F(mut v) => {
            for (k, vk) in v.iter_mut().enumerate().skip(comps as usize) {
                *vk = if k == 3 { 1.0 } else { 0.0 };
            }
            Texel::F(v)
        }
        Texel::I(mut v) => {
            for (k, vk) in v.iter_mut().enumerate().skip(comps as usize) {
                *vk = if k == 3 { one } else { 0 };
            }
            Texel::I(v)
        }
    }
}

fn unorm(v: f32, max: f32) -> u32 {
    if v > 0.0 {
        (v.min(1.0) * max + 0.5) as u32
    } else {
        0
    }
}

fn snorm8(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    let x = v.clamp(-1.0, 1.0) * 127.0;
    (if x >= 0.0 { x + 0.5 } else { x - 0.5 }) as i32 as i8 as u8
}

/// As floats, whatever the decoder produced (integers only reach a float
/// storage through a pair the caller already refused).
fn as_floats(t: Texel) -> [f32; 4] {
    match t {
        Texel::F(v) => v,
        Texel::I(v) => [v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32],
    }
}

fn as_ints(t: Texel) -> [u32; 4] {
    match t {
        Texel::I(v) => v,
        Texel::F(v) => [v[0] as i32 as u32, v[1] as i32 as u32, v[2] as i32 as u32, v[3] as i32 as u32],
    }
}

/// Bytes per texel of the storage classes this module writes.
pub fn storage_bytes(texfmt: u8) -> usize {
    gles_texfmt_bytes(texfmt) as usize
}

/// Convert one texel into the storage class `texfmt` for the format `fmt`.
/// `dst` holds `storage_bytes(texfmt)` bytes. False for a class this module
/// does not write.
pub fn encode(fmt: &Es3Format, texfmt: u8, t: Texel, dst: &mut [u8]) -> bool {
    let t = masked(t, fmt.comps, 1);
    let ints = as_ints(t);
    let fl = as_floats(t);
    let put16 = |dst: &mut [u8], k: usize, v: u16| dst[k * 2..k * 2 + 2].copy_from_slice(&v.to_ne_bytes());
    let put32 = |dst: &mut [u8], k: usize, v: u32| dst[k * 4..k * 4 + 4].copy_from_slice(&v.to_ne_bytes());

    match texfmt {
        GLES_TEXFMT_RGBA16F => {
            for (k, v) in fl.iter().enumerate() {
                put16(dst, k, f32_to_half(*v));
            }
        }
        GLES_TEXFMT_RGBA32F => {
            for (k, v) in fl.iter().enumerate() {
                put32(dst, k, v.to_bits());
            }
        }
        GLES_TEXFMT_RGBA8 => {
            for (k, v) in fl.iter().enumerate() {
                dst[k] = unorm(*v, 255.0) as u8;
            }
        }
        GLES_TEXFMT_RGBA8_SNORM => {
            for (k, v) in fl.iter().enumerate() {
                dst[k] = snorm8(*v);
            }
        }
        GLES_TEXFMT_R11G11B10F => put32(dst, 0, pack_r11g11b10f(fl[0], fl[1], fl[2])),
        GLES_TEXFMT_RGB9E5 => put32(dst, 0, pack_rgb9e5(fl[0], fl[1], fl[2])),
        GLES_TEXFMT_RGB10A2 => {
            put32(
                dst,
                0,
                unorm(fl[0], 1023.0)
                    | (unorm(fl[1], 1023.0) << 10)
                    | (unorm(fl[2], 1023.0) << 20)
                    | (unorm(fl[3], 3.0) << 30),
            );
        }
        GLES_TEXFMT_RGB10A2UI => {
            put32(
                dst,
                0,
                (ints[0].min(1023)) | (ints[1].min(1023) << 10) | (ints[2].min(1023) << 20) | (ints[3].min(3) << 30),
            );
        }
        GLES_TEXFMT_RGBA8UI | GLES_TEXFMT_RGBA8I => {
            for (k, v) in ints.iter().enumerate() {
                dst[k] = *v as u8;
            }
        }
        GLES_TEXFMT_RGBA16UI | GLES_TEXFMT_RGBA16I => {
            for (k, v) in ints.iter().enumerate() {
                put16(dst, k, *v as u16);
            }
        }
        GLES_TEXFMT_RGBA32UI | GLES_TEXFMT_RGBA32I => {
            for (k, v) in ints.iter().enumerate() {
                put32(dst, k, *v);
            }
        }
        _ => return false,
    }
    true
}

/// Convert `n` texels of (format, type) at `src` into `dst` as `fmt` stored
/// in `texfmt`. False when the pair or the class is not one this module
/// handles (nothing is then promised about `dst`).
pub fn convert_row(
    fmt: &Es3Format,
    texfmt: u8,
    src: &[u8],
    format: GLenum,
    type_: GLenum,
    n: usize,
    dst: &mut [u8],
) -> bool {
    let sb = texel_bytes(format, type_);
    let db = storage_bytes(texfmt);

    if sb == 0 || src.len() < n * sb || dst.len() < n * db {
        return false;
    }
    for x in 0..n {
        let Some(t) = decode(&src[x * sb..(x + 1) * sb], format, type_) else { return false };
        if !encode(fmt, texfmt, t, &mut dst[x * db..(x + 1) * db]) {
            return false;
        }
    }
    true
}
