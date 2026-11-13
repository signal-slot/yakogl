// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! CPU conversions for the float colour formats glReadPixels has to decode:
//! IEEE half floats (RGBA16F, RG16F, R16F) and the unsigned packed
//! R11F_G11F_B10F (ES 3.0.6 section 2.1.3: 5-bit exponent, 6-bit mantissa for
//! R and G, 5-bit mantissa for B, no sign bit, bias 15, no negative values).
//!
//! Plan #360 (GL_EXT_color_buffer_float). The module is pure `no_std` code
//! (no `alloc`, no `std`) and is **not yet in the module tree**: it is pulled
//! into `tests/es3_float_pack.rs` with `#[path]`. The core-surf item adds
//! `mod float_pack;` to `src/gl/mod.rs` and wires it into ReadPixels.
//!
//! The GL core already has two half-float helpers; this file mirrors them and
//! differs where ReadPixels needs it:
//! `framebuffer::half_to_float` (exact, subnormals normalised; [`half_to_f32`]
//! is the same function) and `texture::float_to_half` (round toward zero, used
//! for uploads; [`f32_to_half`] rounds to nearest even instead).
//!
//! Conversion rules shared by every function here:
//! - float to packed: rounds to nearest even, overflow (including a value that
//!   rounds past the largest finite one) goes to infinity, NaN stays NaN;
//! - the unsigned 11/10-bit formats clamp negative values and -inf to 0
//!   (Mesa `util/format_r11g11b10f.h` does the same); -0 packs to 0;
//! - f32 subnormals are far below the smallest 11-bit denormal and pack to 0.

/// Round `x >> shift` to nearest even (`shift` in 1..=31).
#[inline]
fn shr_rne(x: u32, shift: u32) -> u32 {
    let half = 1u32 << (shift - 1);
    (x + (half - 1) + ((x >> shift) & 1)) >> shift
}

/// Pack a non-negative, non-NaN f32 (given as bits without the sign) into an
/// unsigned float with a 5-bit exponent (bias 15) and `mant` mantissa bits.
/// Returns the `5 + mant` bit field; infinity is `0x1f << mant`.
fn pack_ufloat(abs_bits: u32, mant: u32) -> u32 {
    let inf = 0x1f << mant;
    let shift = 23 - mant;
    let exp = (abs_bits >> 23) as i32 - 127;
    if abs_bits >= 0x7f80_0000 || exp > 15 {
        return inf;
    }
    let r = if exp >= -14 {
        // re-bias the exponent from 127 to 15; a mantissa carry runs into it
        shr_rne(abs_bits - (112 << 23), shift)
    } else {
        // target denormal: the 24-bit significand shifted down further
        let s = shift + (-14 - exp) as u32;
        if s > 24 {
            return 0;
        }
        shr_rne((1 << 23) | (abs_bits & 0x7f_ffff), s)
    };
    r.min(inf)
}

/// Unpack an unsigned float with a 5-bit exponent and `mant` mantissa bits.
fn unpack_ufloat(v: u32, mant: u32) -> f32 {
    let exp = (v >> mant) & 0x1f;
    let m = v & ((1 << mant) - 1);
    if exp == 0x1f {
        return f32::from_bits(0x7f80_0000 | if m != 0 { 0x40_0000 | (m << (23 - mant)) } else { 0 });
    }
    if exp == 0 {
        // m * 2^(-14 - mant), exact
        return m as f32 * f32::from_bits((127 - 14 - mant) << 23);
    }
    f32::from_bits(((exp + 112) << 23) | (m << (23 - mant)))
}

/// IEEE half to f32, exact: subnormals are normalised, infinity and NaN keep
/// their class (a NaN keeps its payload, shifted up).
pub fn half_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let abs = (h & 0x7fff) as u32;
    f32::from_bits(sign | unpack_ufloat(abs, 10).to_bits())
}

/// f32 to IEEE half, round to nearest even; overflow gives infinity, values
/// below half the smallest subnormal give a signed zero, NaN stays NaN.
pub fn f32_to_half(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let abs = bits & 0x7fff_ffff;
    if abs > 0x7f80_0000 {
        // NaN: keep the top payload bits, force the quiet bit so it stays NaN
        return sign | 0x7e00 | ((abs >> 13) & 0x1ff) as u16;
    }
    sign | pack_ufloat(abs, 10) as u16
}

/// Unpack R11F_G11F_B10F (R in bits 0..=10, G in 11..=21, B in 22..=31) to
/// `[r, g, b]`. Infinity and NaN keep their class.
pub fn unpack_r11g11b10f(v: u32) -> [f32; 3] {
    [unpack_ufloat(v & 0x7ff, 6), unpack_ufloat((v >> 11) & 0x7ff, 6), unpack_ufloat(v >> 22, 5)]
}

/// Pack `[r, g, b]` into R11F_G11F_B10F: negative values and -inf clamp to 0,
/// NaN stays NaN, overflow goes to infinity, rounding is to nearest even.
pub fn pack_r11g11b10f(rgb: [f32; 3]) -> u32 {
    let one = |f: f32, mant: u32| -> u32 {
        let bits = f.to_bits();
        let abs = bits & 0x7fff_ffff;
        if abs > 0x7f80_0000 {
            return (0x1f << mant) | (1 << (mant - 1)); // NaN
        }
        if bits >> 31 != 0 {
            return 0;
        }
        pack_ufloat(abs, mant)
    };
    one(rgb[0], 6) | (one(rgb[1], 6) << 11) | (one(rgb[2], 5) << 22)
}

/// Convert a row of little-endian half-float texels with `channels` (1 = R16F,
/// 2 = RG16F, 4 = RGBA16F) channels to RGBA f32 as ReadPixels RGBA/FLOAT
/// returns them: missing G and B are 0, a missing A is 1. Converts
/// `min(src texels, out pixels)` pixels and returns that count; `out` holds 4
/// floats per pixel. `channels` outside 1..=4 converts nothing.
pub fn half_row_to_rgba_f32(src: &[u8], channels: usize, out: &mut [f32]) -> usize {
    if !(1..=4).contains(&channels) {
        return 0;
    }
    let n = (src.len() / (2 * channels)).min(out.len() / 4);
    for (texel, px) in src.chunks_exact(2 * channels).zip(out.chunks_exact_mut(4)).take(n) {
        px.copy_from_slice(&[0.0, 0.0, 0.0, 1.0]);
        for (c, h) in texel.chunks_exact(2).enumerate() {
            px[c] = half_to_f32(u16::from_le_bytes([h[0], h[1]]));
        }
    }
    n
}

/// [`half_row_to_rgba_f32`] for RGBA16F.
pub fn rgba16f_row_to_rgba_f32(src: &[u8], out: &mut [f32]) -> usize {
    half_row_to_rgba_f32(src, 4, out)
}

/// [`half_row_to_rgba_f32`] for RG16F (B = 0, A = 1).
pub fn rg16f_row_to_rgba_f32(src: &[u8], out: &mut [f32]) -> usize {
    half_row_to_rgba_f32(src, 2, out)
}

/// [`half_row_to_rgba_f32`] for R16F (G = B = 0, A = 1).
pub fn r16f_row_to_rgba_f32(src: &[u8], out: &mut [f32]) -> usize {
    half_row_to_rgba_f32(src, 1, out)
}

/// Convert a row of little-endian R11F_G11F_B10F words to RGBA f32 (A = 1).
/// Same count rule as [`half_row_to_rgba_f32`].
pub fn r11g11b10f_row_to_rgba_f32(src: &[u8], out: &mut [f32]) -> usize {
    let n = (src.len() / 4).min(out.len() / 4);
    for (w, px) in src.chunks_exact(4).zip(out.chunks_exact_mut(4)).take(n) {
        let [r, g, b] = unpack_r11g11b10f(u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
        px.copy_from_slice(&[r, g, b, 1.0]);
    }
    n
}
