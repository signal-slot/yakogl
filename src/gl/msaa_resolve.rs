// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The CPU reference of a multisample resolve: N samples of one pixel in, the
//! resolved pixel out, per renderbuffer format
//! (docs/notes/es3-msaa-renderbuffers.md, (b) items 7-9 and (e) 6-7). The host
//! tests of the resolve blit and of each back end's 4x FBO compare the pixels
//! they read back against it. Nothing calls it yet: `gl-msaa-resolve` and the
//! `*-msaa-fbo` items do.
//!
//! A format is named by its sized internal format, the GLenum the GL core
//! keys its tables on; the integer formats' rows are [`es3_formats::FORMATS`].
//! A pixel is the format's packed bytes as its own transfer type lays them out
//! (ES 3.0.6 table 3.2's first type, native endian): RGBA8 is four bytes,
//! RGB565 one `GL_UNSIGNED_SHORT_5_6_5` u16 (red in the top bits), RGB10_A2
//! one `GL_UNSIGNED_INT_2_10_10_10_REV` u32 (red in the low bits), RGBA16F four
//! halves, R11F_G11F_B10F one `GL_UNSIGNED_INT_10F_11F_11F_REV` u32, depth and
//! stencil as glReadPixels-style words (DEPTH_COMPONENT24 and DEPTH24_STENCIL8
//! one u32, DEPTH32F_STENCIL8 eight bytes). A caller whose storage is another
//! class (the GL core keeps RGB565 renderbuffers as RGBA8888) converts first,
//! or resolves the storage's own format: [`stored`] and [`resolve_rect`]
//! resolve a rectangle in the layout the GL core stores
//! ([`renderbuffer_surf_format`]), for the resolve blit.
//!
//! The rules:
//!  - UNORM (the 8-bit R/RG/RGB/RGBA, RGB565, RGBA4, RGB5_A1, RGB10_A2): each
//!    field is averaged in its own bit width and rounded to the nearest value,
//!    a half rounding up: `(sum + n / 2) / n` in integers, which is
//!    `round(mean)` exactly (no float). 0, 0, 255, 255 -> 510 / 4 = 127.5 -> 128.
//!  - SRGB8_ALPHA8: red, green and blue decoded to linear ([`srgb::decode`]),
//!    averaged in f32, encoded back ([`srgb::encode`], the nearest byte); alpha
//!    is linear and resolves as UNORM.
//!  - Float (R16F, RG16F, RGB16F, RGBA16F, R32F, RG32F, RGBA32F,
//!    R11F_G11F_B10F): the samples decoded to f64, averaged, and stored back
//!    with the format's own rounding (half: nearest, ties to even; 11/10-bit:
//!    nearest, halves up; f32: nearest).
//!  - Integer, depth and stencil: sample 0. ES 3.0 refuses an integer resolve
//!    at the API (4.3.3) and leaves which sample a depth or stencil resolve
//!    takes to the implementation; sample 0 is the CPU copy's choice (note (e)6).
// Nothing calls it until gl-msaa-resolve lands.
#![cfg_attr(not(test), allow(dead_code))]

use super::consts::*;
use super::es3_formats::{self, Class};
use super::framebuffer::renderbuffer_surf_format;
use super::srgb;
use super::types::GLenum;

/// How a format's samples combine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Rule {
    /// `n` unsigned bytes, each averaged.
    Unorm8(usize),
    /// One u16 of UNORM fields, (shift, bits) each.
    Unorm16(&'static [(u32, u32)]),
    /// RGB10_A2: one u32, three 10-bit fields and a 2-bit one.
    Rgb10A2,
    /// SRGB8_ALPHA8: three sRGB bytes averaged in linear, a linear alpha.
    Srgb8A8,
    /// `n` IEEE halves.
    Half(usize),
    /// `n` IEEE floats.
    F32(usize),
    /// R11F_G11F_B10F, one u32.
    R11G11B10F,
    /// Sample 0 of a pixel this many bytes wide (integer, depth, stencil).
    Sample0(usize),
}

const RGB565: &[(u32, u32)] = &[(11, 5), (5, 6), (0, 5)];
const RGBA4: &[(u32, u32)] = &[(12, 4), (8, 4), (4, 4), (0, 4)];
const RGB5_A1: &[(u32, u32)] = &[(11, 5), (6, 5), (1, 5), (0, 1)];

/// The rule of a sized internal format; None for one with no multisample
/// storage to resolve (an unsized or compressed name, RGB9_E5, the SNORMs).
pub(crate) fn rule(internalformat: GLenum) -> Option<Rule> {
    Some(match internalformat {
        GL_R8 => Rule::Unorm8(1),
        GL_RG8 => Rule::Unorm8(2),
        GL_RGB8 => Rule::Unorm8(3),
        GL_RGBA8 => Rule::Unorm8(4),
        GL_RGB565 => Rule::Unorm16(RGB565),
        GL_RGBA4 => Rule::Unorm16(RGBA4),
        GL_RGB5_A1 => Rule::Unorm16(RGB5_A1),
        GL_RGB10_A2 => Rule::Rgb10A2,
        GL_SRGB8_ALPHA8 => Rule::Srgb8A8,
        GL_R16F => Rule::Half(1),
        GL_RG16F => Rule::Half(2),
        GL_RGB16F => Rule::Half(3),
        GL_RGBA16F => Rule::Half(4),
        GL_R32F => Rule::F32(1),
        GL_RG32F => Rule::F32(2),
        GL_RGBA32F => Rule::F32(4),
        GL_R11F_G11F_B10F => Rule::R11G11B10F,
        GL_STENCIL_INDEX8 => Rule::Sample0(1),
        GL_DEPTH_COMPONENT16 => Rule::Sample0(2),
        GL_DEPTH_COMPONENT24 | GL_DEPTH_COMPONENT32F | GL_DEPTH24_STENCIL8 => Rule::Sample0(4),
        GL_DEPTH32F_STENCIL8 => Rule::Sample0(8),
        f => {
            /* the integer rows of es3_formats, whatever report-es2 says of
             * `find`: the helper stays total */
            let e = es3_formats::FORMATS.iter().find(|e| e.internalformat == f)?;
            if !matches!(e.class, Class::Int | Class::Uint) {
                return None;
            }
            Rule::Sample0(es3_formats::texel_bytes(e.base, e.types[0]))
        }
    })
}

/// Bytes of one pixel (one sample) of the format; None as [`rule`].
pub(crate) fn pixel_bytes(internalformat: GLenum) -> Option<usize> {
    rule(internalformat).map(|r| match r {
        Rule::Unorm8(n) => n,
        Rule::Unorm16(_) => 2,
        Rule::Rgb10A2 | Rule::Srgb8A8 | Rule::R11G11B10F => 4,
        Rule::Half(n) => 2 * n,
        Rule::F32(n) => 4 * n,
        Rule::Sample0(b) => b,
    })
}

/// The mean of `n` integer samples rounded to the nearest, a half up.
fn avg_round(sum: u64, n: u64) -> u64 {
    (sum + n / 2) / n
}

fn u16_at(s: &[u8], o: usize) -> u16 {
    u16::from_ne_bytes([s[o], s[o + 1]])
}
fn u32_at(s: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]])
}

/// Average the packed fields (shift, bits) of `n` words.
fn avg_fields(words: impl Iterator<Item = u32> + Clone, fields: &[(u32, u32)], n: u64) -> u32 {
    let mut out = 0u32;
    for &(shift, bits) in fields {
        let mask = (1u32 << bits) - 1;
        let sum: u64 = words.clone().map(|w| u64::from((w >> shift) & mask)).sum();
        out |= (avg_round(sum, n) as u32) << shift;
    }
    out
}

/// Resolve one pixel of `internalformat`: `samples` holds `n` pixels of
/// [`pixel_bytes`] each, sample 0 first; the resolved pixel goes to the front
/// of `out`. False (nothing written) for a format [`rule`] does not know, for
/// `n` 0, or for slices too short.
pub(crate) fn resolve_pixel(internalformat: GLenum, samples: &[u8], n: usize, out: &mut [u8]) -> bool {
    let (Some(r), Some(b)) = (rule(internalformat), pixel_bytes(internalformat)) else { return false };
    if n == 0 || samples.len() < n * b || out.len() < b {
        return false;
    }
    resolve_with(r, b, samples, n, out);
    true
}

/// Resolve one pixel of `b` bytes by rule `r`; the caller checked `n` > 0 and
/// the slice lengths.
fn resolve_with(r: Rule, b: usize, samples: &[u8], n: usize, out: &mut [u8]) {
    let px = |i: usize| &samples[i * b..(i + 1) * b];
    let nn = n as u64;
    match r {
        Rule::Unorm8(c) => {
            for (k, o) in out.iter_mut().enumerate().take(c) {
                let sum: u64 = (0..n).map(|i| u64::from(px(i)[k])).sum();
                *o = avg_round(sum, nn) as u8;
            }
        }
        Rule::Unorm16(fields) => {
            let w = avg_fields((0..n).map(|i| u32::from(u16_at(px(i), 0))), fields, nn) as u16;
            out[..2].copy_from_slice(&w.to_ne_bytes());
        }
        Rule::Rgb10A2 => {
            let w = avg_fields((0..n).map(|i| u32_at(px(i), 0)), &[(0, 10), (10, 10), (20, 10), (30, 2)], nn);
            out[..4].copy_from_slice(&w.to_ne_bytes());
        }
        Rule::Srgb8A8 => {
            for (k, o) in out.iter_mut().enumerate().take(3) {
                let sum: f32 = (0..n).map(|i| srgb::decode(px(i)[k])).sum();
                *o = srgb::encode(sum / n as f32);
            }
            let sum: u64 = (0..n).map(|i| u64::from(px(i)[3])).sum();
            out[3] = avg_round(sum, nn) as u8;
        }
        Rule::Half(c) => {
            for k in 0..c {
                let sum: f64 = (0..n).map(|i| f64::from(es3_formats::half_to_f32(u16_at(px(i), 2 * k)))).sum();
                let h = es3_formats::f32_to_half((sum / n as f64) as f32);
                out[2 * k..2 * k + 2].copy_from_slice(&h.to_ne_bytes());
            }
        }
        Rule::F32(c) => {
            for k in 0..c {
                let sum: f64 = (0..n).map(|i| f64::from(f32::from_bits(u32_at(px(i), 4 * k)))).sum();
                out[4 * k..4 * k + 4].copy_from_slice(&((sum / n as f64) as f32).to_bits().to_ne_bytes());
            }
        }
        Rule::R11G11B10F => {
            let mut m = [0f64; 3];
            for i in 0..n {
                for (mk, v) in m.iter_mut().zip(es3_formats::unpack_r11g11b10f(u32_at(px(i), 0))) {
                    *mk += f64::from(v);
                }
            }
            let [r, g, b] = m.map(|s| (s / n as f64) as f32);
            out[..4].copy_from_slice(&es3_formats::pack_r11g11b10f(r, g, b).to_ne_bytes());
        }
        Rule::Sample0(_) => out[..b].copy_from_slice(px(0)),
    }
}

/// How a renderbuffer's samples are resolved in the layout the GL core
/// stores it in: [`renderbuffer_surf_format`]'s surface, which is wider than
/// the format's own pixel for the narrow names (R8, RG8, RGB8, RGB565, RGBA4,
/// RGB5_A1, RGB10_A2 where its 10/10/10/2 surface is not drawn, and the other
/// names of the RGBA8888 fallback are four bytes RGBA; RGB16F and RG16F are
/// four halves).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Stored {
    /// Bytes of one stored pixel (one sample).
    pub bytes: usize,
    /// The rule applied to the stored channels.
    pub rule: Rule,
}

/// The stored-layout resolve of `internalformat`; None as [`rule`].
///
/// A colour format's surface decides it: RGBA8888 (and XRGB8888) four UNORM
/// bytes averaged, the sRGB surface as [`Rule::Srgb8A8`], the 10/10/10/2
/// surface as [`Rule::Rgb10A2`], the half and float surfaces averaged as
/// floats, RGB565 in its fields. An integer format takes sample 0 of its
/// stored pixel. Depth and stencil take sample 0 of the format's own word
/// ([`pixel_bytes`]): the GL core keeps no storage of its own for them, the
/// back end's resolve does (note (e)6), so the CPU reference stays in the
/// read-back layout.
pub(crate) fn stored(internalformat: GLenum) -> Option<Stored> {
    use crate::backend::*;
    let own = rule(internalformat)?;
    if matches!(
        internalformat,
        GL_DEPTH_COMPONENT16
            | GL_DEPTH_COMPONENT24
            | GL_DEPTH_COMPONENT32F
            | GL_DEPTH24_STENCIL8
            | GL_DEPTH32F_STENCIL8
            | GL_STENCIL_INDEX8
    ) {
        return Some(Stored { bytes: pixel_bytes(internalformat)?, rule: own });
    }
    let surf = renderbuffer_surf_format(internalformat);
    let bytes = gles_surf_bytes_per_pixel(surf) as usize;
    /* integer formats keep sample 0 whatever surface holds them (an ES 2.0
     * report maps them to RGBA8888, where they are not colour-renderable) */
    let rule = if matches!(own, Rule::Sample0(_)) || gles_surf_is_integer(surf) {
        Rule::Sample0(bytes)
    } else {
        match surf {
            GLES_SURF_RGB565 => Rule::Unorm16(RGB565),
            GLES_SURF_RGBA16F => Rule::Half(4),
            GLES_SURF_R16F => Rule::Half(1),
            GLES_SURF_R32F => Rule::F32(1),
            GLES_SURF_SRGB8_A8 => Rule::Srgb8A8,
            GLES_SURF_RGB10_A2 => Rule::Rgb10A2,
            /* RGBA8888, XRGB8888 */
            _ => Rule::Unorm8(4),
        }
    };
    Some(Stored { bytes, rule })
}

/// Resolve a `w` x `h` rectangle of a multisample renderbuffer of
/// `internalformat`, in its stored layout ([`stored`]).
///
/// `src` row `y` starts at `y * src_pitch` and holds `w` pixels of `samples`
/// stored pixels each, back to back, sample 0 first (`samples * bytes` a
/// pixel). `dst` row `y` starts at `y * dst_pitch` and gets `w` resolved
/// stored pixels; the bytes between the rows and past the rectangle are not
/// written. A 0-wide or 0-high rectangle writes nothing and is true. False,
/// with nothing written, for a format [`stored`] does not know, `samples` 0,
/// a pitch shorter than its row or a slice shorter than its rectangle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_rect(
    internalformat: GLenum,
    src: &[u8],
    src_pitch: usize,
    samples: usize,
    w: usize,
    h: usize,
    dst: &mut [u8],
    dst_pitch: usize,
) -> bool {
    let Some(Stored { bytes: b, rule: r }) = stored(internalformat) else { return false };
    if samples == 0 {
        return false;
    }
    if w == 0 || h == 0 {
        return true;
    }
    /* the last row ends at (h - 1) * pitch + row bytes */
    let span = |pitch: usize, row: usize| -> Option<usize> {
        (row <= pitch).then_some(())?;
        (h - 1).checked_mul(pitch)?.checked_add(row)
    };
    let (Some(src_row), Some(dst_row)) = (w.checked_mul(samples).and_then(|p| p.checked_mul(b)), w.checked_mul(b))
    else {
        return false;
    };
    match (span(src_pitch, src_row), span(dst_pitch, dst_row)) {
        (Some(s), Some(d)) if s <= src.len() && d <= dst.len() => {}
        _ => return false,
    }
    let px = samples * b;
    for y in 0..h {
        let (s, d) = (&src[y * src_pitch..y * src_pitch + src_row], &mut dst[y * dst_pitch..y * dst_pitch + dst_row]);
        for (sp, dp) in s.chunks_exact(px).zip(d.chunks_exact_mut(b)) {
            resolve_with(r, b, sp, samples, dp);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve `samples` (one Vec of bytes each) of `fmt`.
    fn res(fmt: GLenum, samples: &[Vec<u8>]) -> Vec<u8> {
        let b = pixel_bytes(fmt).unwrap();
        let flat: Vec<u8> = samples.iter().flat_map(|s| s.iter().copied()).collect();
        let mut out = vec![0xeeu8; b];
        assert!(resolve_pixel(fmt, &flat, samples.len(), &mut out), "{fmt:#x}");
        out
    }

    fn w16(v: u16) -> Vec<u8> {
        v.to_ne_bytes().to_vec()
    }
    fn w32(v: u32) -> Vec<u8> {
        v.to_ne_bytes().to_vec()
    }
    fn halves(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| es3_formats::f32_to_half(*x).to_ne_bytes()).collect()
    }
    fn floats(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_bits().to_ne_bytes()).collect()
    }

    #[test]
    fn unorm8_rounds_the_mean_to_nearest_halves_up() {
        /* 0 + 0 + 255 + 255 = 510, / 4 = 127.5 -> 128 */
        assert_eq!(res(GL_R8, &[vec![0], vec![0], vec![255], vec![255]]), [128]);
        /* 1 + 2 = 3, / 2 = 1.5 -> 2; 1 + 1 + 1 + 2 = 5, / 4 = 1.25 -> 1 */
        assert_eq!(res(GL_RG8, &[vec![1, 1], vec![2, 1]]), [2, 1]);
        assert_eq!(res(GL_RG8, &[vec![1, 0], vec![1, 0], vec![1, 0], vec![2, 3]]), [1, 1]);
        /* 3 / 4 = 0.75 -> 1 */
        assert_eq!(
            res(GL_RGB8, &[vec![0, 10, 255], vec![0, 20, 255], vec![0, 30, 255], vec![3, 40, 255]]),
            [1, 25, 255]
        );
        /* one sample is itself */
        assert_eq!(res(GL_RGBA8, &[vec![9, 8, 7, 6]]), [9, 8, 7, 6]);
        assert_eq!(res(GL_RGBA8, &[vec![255, 0, 0, 255], vec![0, 0, 255, 0]]), [128, 0, 128, 128]);
    }

    #[test]
    fn packed16_fields_average_in_their_own_width() {
        /* RGB565: red 31 + 0 = 15.5 -> 16, green 63 + 0 = 31.5 -> 32, blue 0 + 1 = 0.5 -> 1 */
        let a = (31 << 11) | (63 << 5);
        assert_eq!(res(GL_RGB565, &[w16(a), w16(1)]), w16((16 << 11) | (32 << 5) | 1));
        /* RGBA4: 15,15,0,0 x2 and 0,0,0,0 x2: 7.5 -> 8 */
        let s = 0xff00;
        assert_eq!(res(GL_RGBA4, &[w16(s), w16(s), w16(0), w16(0)]), w16(0x8800));
        /* RGB5_A1: alpha 1,1,1,0 -> 0.75 -> 1; red 31,0,0,0 -> 7.75 -> 8 */
        let p = (31 << 11) | 1;
        assert_eq!(res(GL_RGB5_A1, &[w16(p), w16(1), w16(1), w16(0)]), w16((8 << 11) | 1));
        /* alpha 1,1,0,0 -> 0.5 -> 1 (halves up) */
        assert_eq!(res(GL_RGB5_A1, &[w16(1), w16(1), w16(0), w16(0)]), w16(1));
    }

    #[test]
    fn rgb10_a2_averages_each_field() {
        let w = |r: u32, g: u32, b: u32, a: u32| w32(r | (g << 10) | (b << 20) | (a << 30));
        /* 1023 + 0 = 511.5 -> 512; 1 + 2 = 1.5 -> 2; alpha 3 + 0 = 1.5 -> 2 */
        assert_eq!(res(GL_RGB10_A2, &[w(1023, 1, 100, 3), w(0, 2, 100, 0)]), w(512, 2, 100, 2));
        /* four: 1023, 1023, 0, 0 -> 511.5 -> 512; alpha 3,3,3,0 -> 2.25 -> 2 */
        assert_eq!(
            res(GL_RGB10_A2, &[w(1023, 0, 0, 3), w(1023, 0, 0, 3), w(0, 0, 0, 3), w(0, 0, 0, 0)]),
            w(512, 0, 0, 2)
        );
    }

    #[test]
    fn srgb_averages_colour_in_linear_and_alpha_as_is() {
        /* the midpoint: linear 0 and 1 average to 0.5, which encodes to
         * 1.055 * 0.5^(1/2.4) - 0.055 = 0.7354 -> 187.5 -> 188, not 128;
         * alpha 0 and 255 -> 127.5 -> 128 */
        assert_eq!(res(GL_SRGB8_ALPHA8, &[vec![0, 0, 0, 0], vec![255, 255, 255, 255]]), [188, 188, 188, 128]);
        /* four samples, two of each: the same linear mean */
        let (z, f) = (vec![0, 255, 0, 255], vec![255, 255, 0, 0]);
        assert_eq!(res(GL_SRGB8_ALPHA8, &[z.clone(), z, f.clone(), f]), [188, 255, 0, 128]);
        /* equal samples are kept exactly */
        assert_eq!(res(GL_SRGB8_ALPHA8, &vec![vec![37, 120, 201, 7]; 4]), [37, 120, 201, 7]);
        /* 1 of 4 white: linear 0.25 -> 0.5371 -> 136.97 -> 137 */
        let k = vec![0, 0, 0, 0];
        assert_eq!(res(GL_SRGB8_ALPHA8, &[vec![255, 0, 0, 0], k.clone(), k.clone(), k])[0], 137);
    }

    #[test]
    fn half_floats_average() {
        assert_eq!(res(GL_R16F, &[halves(&[0.0]), halves(&[1.0])]), halves(&[0.5]));
        assert_eq!(res(GL_RG16F, &[halves(&[1.0, -2.0]), halves(&[2.0, 2.0])]), halves(&[1.5, 0.0]));
        assert_eq!(res(GL_RGB16F, &[halves(&[4.0, 0.25, 8.0]), halves(&[0.0, 0.25, 8.0])]), halves(&[2.0, 0.25, 8.0]));
        /* 0, 0, 1, 1 -> 0.5; 1, 1, 1, 0 -> 0.75; beyond 1 is kept (no clamp) */
        let s = |a: f32, b: f32| halves(&[a, b, 3.0, 1.0]);
        assert_eq!(
            res(GL_RGBA16F, &[s(0.0, 1.0), s(0.0, 1.0), s(1.0, 1.0), s(1.0, 0.0)]),
            halves(&[0.5, 0.75, 3.0, 1.0])
        );
    }

    #[test]
    fn f32_floats_average() {
        assert_eq!(res(GL_R32F, &[floats(&[0.0]), floats(&[0.0]), floats(&[1.0]), floats(&[1.0])]), floats(&[0.5]));
        assert_eq!(res(GL_RG32F, &[floats(&[-1.0, 10.0]), floats(&[3.0, 20.0])]), floats(&[1.0, 15.0]));
        /* 0.1 + 0.2 in f64 is 0.30000000000000004 -> /2 -> 0.15 as f32 */
        assert_eq!(
            res(GL_RGBA32F, &[floats(&[0.1, 1.0, 2.0, 1.0]), floats(&[0.2, 3.0, 2.0, 0.0])]),
            floats(&[0.15, 2.0, 2.0, 0.5])
        );
    }

    #[test]
    fn r11f_g11f_b10f_averages_unpacked() {
        let p = |r: f32, g: f32, b: f32| w32(es3_formats::pack_r11g11b10f(r, g, b));
        assert_eq!(res(GL_R11F_G11F_B10F, &[p(0.0, 2.0, 1.0), p(1.0, 4.0, 1.0)]), p(0.5, 3.0, 1.0));
        assert_eq!(
            res(GL_R11F_G11F_B10F, &[p(1.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 0.0, 8.0)]),
            p(0.5, 0.0, 2.0)
        );
    }

    #[test]
    fn integer_formats_take_sample_0() {
        assert_eq!(res(GL_R8UI, &[vec![7], vec![255], vec![0], vec![1]]), [7]);
        assert_eq!(res(GL_RGBA8I, &[vec![0x80, 1, 2, 3], vec![0, 0, 0, 0]]), [0x80, 1, 2, 3]);
        assert_eq!(res(GL_RG16UI, &[w32(0x1234_5678), w32(0)]), w32(0x1234_5678));
        assert_eq!(res(GL_RGB10_A2UI, &[w32(0xc000_03ff), w32(0)]), w32(0xc000_03ff));
        let big: Vec<u8> = (0..16).collect();
        assert_eq!(res(GL_RGBA32I, &[big.clone(), vec![0; 16], vec![9; 16], vec![1; 16]]), big);
        /* every integer row of es3_formats has a rule, sized as its type says */
        for e in es3_formats::FORMATS.iter().filter(|e| e.is_integer()) {
            assert_eq!(rule(e.internalformat), Some(Rule::Sample0(es3_formats::texel_bytes(e.base, e.types[0]))));
            assert!(pixel_bytes(e.internalformat).unwrap() > 0, "{}", e.name);
        }
    }

    #[test]
    fn depth_and_stencil_take_sample_0() {
        assert_eq!(res(GL_DEPTH_COMPONENT16, &[w16(0x1234), w16(0xffff)]), w16(0x1234));
        assert_eq!(res(GL_DEPTH_COMPONENT24, &[w32(0x0012_3456), w32(0), w32(0), w32(0)]), w32(0x0012_3456));
        assert_eq!(res(GL_DEPTH_COMPONENT32F, &[floats(&[0.25]), floats(&[1.0])]), floats(&[0.25]));
        assert_eq!(res(GL_DEPTH24_STENCIL8, &[w32(0xabcd_ef01), w32(0x0000_00ff)]), w32(0xabcd_ef01));
        let ds: Vec<u8> = (1..=8).collect();
        assert_eq!(res(GL_DEPTH32F_STENCIL8, &[ds.clone(), vec![0; 8], vec![0; 8], vec![0; 8]]), ds);
        assert_eq!(res(GL_STENCIL_INDEX8, &[vec![3], vec![200]]), [3]);
    }

    #[test]
    fn unknown_formats_and_bad_arguments_are_refused() {
        let mut out = [0u8; 16];
        for f in [GL_RGB9_E5, GL_RGBA8_SNORM, GL_RGBA, 0] {
            assert_eq!(rule(f), None, "{f:#x}");
            assert!(!resolve_pixel(f, &[0; 64], 1, &mut out));
        }
        /* n 0, too few sample bytes, too small an output */
        assert!(!resolve_pixel(GL_RGBA8, &[0; 16], 0, &mut out));
        assert!(!resolve_pixel(GL_RGBA8, &[0; 15], 4, &mut out));
        assert!(!resolve_pixel(GL_RGBA8, &[0; 16], 4, &mut out[..3]));
        assert_eq!(out, [0; 16], "nothing written");
    }

    #[test]
    fn pixel_sizes() {
        for (f, b) in [
            (GL_R8, 1),
            (GL_RGB8, 3),
            (GL_RGB565, 2),
            (GL_RGB10_A2, 4),
            (GL_SRGB8_ALPHA8, 4),
            (GL_RGB16F, 6),
            (GL_RGBA32F, 16),
            (GL_R11F_G11F_B10F, 4),
            (GL_DEPTH32F_STENCIL8, 8),
            (GL_RGB8UI, 3),
            (GL_RGBA16I, 8),
        ] {
            assert_eq!(pixel_bytes(f), Some(b), "{f:#x}");
        }
    }

    /// `w` x `h` pixels of `samples` stored pixels each from `px(x, y, s)`,
    /// rows `pitch` bytes apart, the padding 0xaa.
    fn ms_rect(
        w: usize,
        h: usize,
        samples: usize,
        pitch: usize,
        px: impl Fn(usize, usize, usize) -> Vec<u8>,
    ) -> Vec<u8> {
        let mut v = vec![0xaau8; pitch * h];
        for y in 0..h {
            let mut o = y * pitch;
            for x in 0..w {
                for s in 0..samples {
                    let p = px(x, y, s);
                    v[o..o + p.len()].copy_from_slice(&p);
                    o += p.len();
                }
            }
        }
        v
    }

    #[test]
    fn stored_layout_follows_renderbuffer_surf_format() {
        use crate::backend::*;
        /* the narrow UNORMs are RGBA8888, four bytes averaged */
        for f in [GL_R8, GL_RG8, GL_RGB8, GL_RGBA8, GL_RGB565, GL_RGBA4, GL_RGB5_A1] {
            assert_eq!(renderbuffer_surf_format(f), GLES_SURF_RGBA8888, "{f:#x}");
            assert_eq!(stored(f), Some(Stored { bytes: 4, rule: Rule::Unorm8(4) }), "{f:#x}");
        }
        /* the half surfaces: RGB16F and RG16F are held as four halves */
        for f in [GL_RG16F, GL_RGB16F, GL_RGBA16F] {
            assert_eq!(stored(f), Some(Stored { bytes: 8, rule: Rule::Half(4) }), "{f:#x}");
        }
        assert_eq!(stored(GL_R16F), Some(Stored { bytes: 2, rule: Rule::Half(1) }));
        assert_eq!(stored(GL_R32F), Some(Stored { bytes: 4, rule: Rule::F32(1) }));
        /* sRGB and RGB10_A2 as their surface is in this build */
        let srgb = if crate::REPORT_ES2 { Rule::Unorm8(4) } else { Rule::Srgb8A8 };
        assert_eq!(stored(GL_SRGB8_ALPHA8), Some(Stored { bytes: 4, rule: srgb }));
        let rgb10a2 =
            if renderbuffer_surf_format(GL_RGB10_A2) == GLES_SURF_RGB10_A2 { Rule::Rgb10A2 } else { Rule::Unorm8(4) };
        assert_eq!(stored(GL_RGB10_A2), Some(Stored { bytes: 4, rule: rgb10a2 }));
        /* integer: sample 0 of the stored pixel, which is the format's own
         * where the build has the integer surfaces */
        for e in es3_formats::FORMATS.iter().filter(|e| e.is_integer()) {
            let st = stored(e.internalformat).unwrap();
            let surf = renderbuffer_surf_format(e.internalformat);
            assert_eq!(st, Stored { bytes: gles_surf_bytes_per_pixel(surf) as usize, rule: Rule::Sample0(st.bytes) });
            if gles_surf_is_integer(surf) {
                assert_eq!(Some(st.bytes), pixel_bytes(e.internalformat), "{}", e.name);
            }
        }
        /* depth and stencil: the format's own word */
        for (f, b) in [
            (GL_DEPTH_COMPONENT16, 2),
            (GL_DEPTH_COMPONENT24, 4),
            (GL_DEPTH_COMPONENT32F, 4),
            (GL_DEPTH24_STENCIL8, 4),
            (GL_DEPTH32F_STENCIL8, 8),
            (GL_STENCIL_INDEX8, 1),
        ] {
            assert_eq!(stored(f), Some(Stored { bytes: b, rule: Rule::Sample0(b) }), "{f:#x}");
        }
        for f in [GL_RGB9_E5, GL_RGBA8_SNORM, GL_RGBA, 0] {
            assert_eq!(stored(f), None, "{f:#x}");
        }
    }

    #[test]
    fn rect_rgba8_4x_averages_each_pixel_and_keeps_padding() {
        /* 3 x 2, distinct samples: channel k of sample s of (x, y) is
         * 10 * s + 40 * x + 100 * y + k, so the mean is 15 + 40x + 100y + k */
        let (w, h, n) = (3, 2, 4);
        let src_pitch = w * n * 4 + 5;
        let src =
            ms_rect(w, h, n, src_pitch, |x, y, s| (0..4).map(|k| (10 * s + 40 * x + 100 * y + k) as u8).collect());
        let dst_pitch = w * 4 + 3;
        let mut dst = vec![0xeeu8; dst_pitch * h];
        assert!(resolve_rect(GL_RGBA8, &src, src_pitch, n, w, h, &mut dst, dst_pitch));
        for y in 0..h {
            for x in 0..w {
                let o = y * dst_pitch + x * 4;
                let want: Vec<u8> = (0..4).map(|k| (15 + 40 * x + 100 * y + k) as u8).collect();
                assert_eq!(dst[o..o + 4], want[..], "({x}, {y})");
            }
            /* the bytes after the row are untouched */
            assert_eq!(dst[y * dst_pitch + w * 4..(y + 1) * dst_pitch], [0xee; 3]);
        }
        /* rounding: 0, 0, 255, 255 -> 128 in every byte */
        let src = ms_rect(1, 1, 4, 16, |_, _, s| vec![if s < 2 { 0 } else { 255 }; 4]);
        let mut d = [0u8; 4];
        assert!(resolve_rect(GL_RGBA8, &src, 16, 4, 1, 1, &mut d, 4));
        assert_eq!(d, [128; 4]);
    }

    #[test]
    fn rect_narrow_format_averages_the_stored_bytes() {
        /* RGB565 is stored RGBA8888: the four stored bytes are averaged, not
         * a 5/6/5 word (two samples of 0xff and two of 0 or 1 give 128 in
         * each byte) */
        let src =
            ms_rect(2, 1, 4, 32, |x, _, s| if s % 2 == 0 { vec![255, 255, 255, 255] } else { vec![x as u8, 0, 0, 0] });
        let mut dst = [0u8; 8];
        assert!(resolve_rect(GL_RGB565, &src, 32, 4, 2, 1, &mut dst, 8));
        assert_eq!(dst, [128; 8]);
        /* RGBA4: 1 + 2 + 3 + 6 = 12 / 4 = 3 in the red byte of the stored pixel */
        let src = ms_rect(1, 1, 4, 16, |_, _, s| vec![[1, 2, 3, 6][s], 0, 9, 255]);
        let mut d = [0u8; 4];
        assert!(resolve_rect(GL_RGBA4, &src, 16, 4, 1, 1, &mut d, 4));
        assert_eq!(d, [3, 0, 9, 255]);
        /* RGB16F is stored as four halves: the fourth is averaged too */
        let src = ms_rect(1, 1, 2, 16, |_, _, s| halves(&[s as f32, 1.0, 2.0, 2.0 * s as f32]));
        let mut d = [0u8; 8];
        assert!(resolve_rect(GL_RGB16F, &src, 16, 2, 1, 1, &mut d, 8));
        assert_eq!(d.to_vec(), halves(&[0.5, 1.0, 2.0, 1.0]));
    }

    #[test]
    fn rect_integer_takes_sample_0() {
        let b = stored(GL_RGBA16UI).unwrap().bytes;
        let src = ms_rect(2, 2, 4, 2 * 4 * b, |x, y, s| (0..b).map(|k| (s * 50 + x * 7 + y * 3 + k) as u8).collect());
        let mut dst = vec![0u8; 2 * 2 * b];
        assert!(resolve_rect(GL_RGBA16UI, &src, 2 * 4 * b, 4, 2, 2, &mut dst, 2 * b));
        for y in 0..2 {
            for x in 0..2 {
                let want: Vec<u8> = (0..b).map(|k| (x * 7 + y * 3 + k) as u8).collect();
                assert_eq!(dst[(y * 2 + x) * b..(y * 2 + x + 1) * b], want[..], "({x}, {y})");
            }
        }
    }

    #[test]
    fn rect_depth24_stencil8_takes_sample_0() {
        let src = ms_rect(2, 1, 4, 32, |x, _, s| w32(if s == 0 { 0xabcd_ef00 | x as u32 } else { 0x0000_00ff }));
        let mut dst = [0u8; 8];
        assert!(resolve_rect(GL_DEPTH24_STENCIL8, &src, 32, 4, 2, 1, &mut dst, 8));
        assert_eq!(dst[..4], w32(0xabcd_ef00)[..]);
        assert_eq!(dst[4..], w32(0xabcd_ef01)[..]);
    }

    #[test]
    fn rect_one_sample_is_a_copy() {
        let src: Vec<u8> = (0..2 * 12).map(|i| (i * 11) as u8).collect();
        let mut dst = vec![0u8; 2 * 8];
        /* 2 x 2 of RGBA8, the source rows 12 bytes apart, the destination 8 */
        assert!(resolve_rect(GL_RGBA8, &src, 12, 1, 2, 2, &mut dst, 8));
        assert_eq!(dst[..8], src[..8]);
        assert_eq!(dst[8..], src[12..20]);
    }

    #[test]
    fn rect_refuses_bad_arguments_and_writes_nothing() {
        let src = [7u8; 64];
        let mut dst = [0u8; 64];
        /* unknown formats, samples 0 */
        for f in [GL_RGB9_E5, GL_RGBA8_SNORM, GL_RGBA, 0] {
            assert!(!resolve_rect(f, &src, 16, 1, 1, 1, &mut dst, 16), "{f:#x}");
        }
        assert!(!resolve_rect(GL_RGBA8, &src, 16, 0, 1, 1, &mut dst, 16));
        /* a pitch shorter than its row: 2 x 4 x 4 = 32 source bytes a row */
        assert!(!resolve_rect(GL_RGBA8, &src, 31, 4, 2, 1, &mut dst, 8));
        assert!(!resolve_rect(GL_RGBA8, &src, 32, 4, 2, 1, &mut dst, 7));
        /* slices shorter than the rectangle: 2 rows of 32 fit in 64, 3 do not */
        assert!(!resolve_rect(GL_RGBA8, &src, 32, 4, 2, 3, &mut dst, 8));
        assert!(!resolve_rect(GL_RGBA8, &src, 32, 4, 2, 2, &mut dst[..15], 8));
        /* sizes that overflow */
        assert!(!resolve_rect(GL_RGBA8, &src, usize::MAX, usize::MAX, usize::MAX, 1, &mut dst, 8));
        assert!(!resolve_rect(GL_RGBA8, &src, 32, 4, 2, usize::MAX, &mut dst, 8));
        assert_eq!(dst, [0; 64], "nothing written");
        /* the exact fit is accepted */
        assert!(resolve_rect(GL_RGBA8, &src, 32, 4, 2, 2, &mut dst[..16], 8));
        /* an empty rectangle is done */
        assert!(resolve_rect(GL_RGBA8, &[], 0, 4, 0, 5, &mut [], 0));
        assert!(resolve_rect(GL_RGBA8, &[], 0, 4, 5, 0, &mut [], 0));
    }
}
