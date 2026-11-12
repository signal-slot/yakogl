// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
// Modelled on VK-GL-CTS framework/common/tcuCompressedTexture.cpp at
// 93bca01 (Apache-2.0), modified: see below.
// See NOTICE.
//! ETC2 / EAC decoding on the CPU for the ten OpenGL ES 3.0 block formats:
//! `RGB8_ETC2`, `SRGB8_ETC2`, `RGBA8_ETC2_EAC`, `SRGB8_ALPHA8_ETC2_EAC`,
//! `RGB8_PUNCHTHROUGH_ALPHA1_ETC2`, `SRGB8_PUNCHTHROUGH_ALPHA1_ETC2`,
//! `R11_EAC`, `SIGNED_R11_EAC`, `RG11_EAC` and `SIGNED_RG11_EAC`. The
//! sRGB formats store the same bytes as their linear twins; the decode is
//! identical and the GL core applies the sRGB meaning to the texels.
//!
//! The functions follow dEQP's `decompressETC2Block` (individual,
//! differential, T, H and planar modes, the punch-through variant),
//! `decompressEAC8Block` and `decompressEAC11Block` (with the 11-to-16-bit
//! extension): the arithmetic of OpenGL ES 3.0 spec Annex C.2/C.3, which
//! dEQP's reference implements. The VK-GL-CTS sources were not at hand when
//! this was written, so the code is written from the spec and checked
//! against Mesa 26.2.1's decoder, not transcribed from
//! tcuCompressedTexture.cpp; a comparison with dEQP's own output is still to
//! do (the host dEQP run of the plan's deqp-after item).
//!
//! Output: colour formats give RGBA8 texels; the 11-bit formats give 16-bit
//! channels, unsigned (`u16`, 0..=65535 for 0..=1.0) or signed (`i16`,
//! -32767..=32767 for -1.0..=1.0). The 11-bit value `v` is widened as
//! `(v << 5) | (v >> 6)` (unsigned) or, on the magnitude of a signed value,
//! `(m << 5) | (m >> 5)` with `m` in 0..=1023, the sign applied afterwards.
//! So all 11 bits are kept (`value >> 5` of an unsigned result is the spec's
//! 11-bit integer) and 0 / 2047 map to 0 / 65535. These are the 16-bit
//! values Mesa writes and, by the spec's allowance to extend to any number of
//! bits, those dEQP compares against. Converting to a GL texel class is left
//! to the GL core.
//!
//! Layout. A block is 4x4 texels; the 4-wide, 4-high blocks of an image are
//! row-major, `ceil(w/4)` per row, and the texels of the right and bottom
//! edge blocks outside the image are dropped. A block's texels come out
//! row-major, (x, y) at `y * 4 + x`. Block bytes are the stream's own: the
//! 64-bit words are big-endian, the pixel indices run down the columns.
//! RGBA8_ETC2_EAC and RG11 blocks are two 8-byte halves: alpha (or red) then
//! the colour (or green).
//!
//! Test vectors. `tests` holds blocks and the texels Mesa 26.2.1's own
//! decoder (src/mesa/main/texcompress_etc.c, read only) makes of them,
//! one per mode (individual, differential, T, H, planar, each with the
//! opaque bit set and clear for punch-through) and EAC blocks with
//! multiplier 0 and 15, every table index and the clamp extremes (signed
//! base -128, results -1023 and 1023). To reproduce: copy
//! texcompress_etc.c next to a throwaway C file, cut it before
//! `_mesa_unpack_etc2_format`, stub the Mesa headers it includes (empty
//! files plus GLubyte/GLshort/... typedefs, `CLAMP`, `MIN2`, `GL_FALSE`),
//! and add `void dec(int fmt, const uint8_t *blk, uint8_t *out)` calling
//! `etc2_unpack_rgb8`, `..._rgb8_punchthrough_alpha1`, `..._rgba8`,
//! `..._r11`, `..._signed_r11`, `..._rg11`, `..._signed_rg11` with
//! width = height = 4 and the matching dst stride (16 bytes per row for RGBA8
//! and RG11, 8 for R11). Build with `gcc -shared -fPIC`, drive it from
//! Python ctypes with random blocks and with hand-built EAC blocks (the
//! 3-bit indices are column-major, texel (x, y) at bit
//! `((3 - y) + (3 - x) * 4) * 3` of the low 48 bits), and print
//! the 11-bit results as the u16 / i16 values Mesa writes. 21000 random
//! blocks (3000 per format, seed 1) agreed with this module while it was
//! written; the committed vectors are the first of each mode in that set.
//! The harness is not committed.

use super::etc1::{extend4_to_8, extend5_to_8, get_bit, get_bits, MODIFIERS};

/// The bytes of one RGB8_ETC2 / PUNCHTHROUGH / R11 block (4x4 texels).
pub const BLOCK_BYTES: usize = 8;
/// The bytes of one RGBA8_ETC2_EAC / RG11 block.
pub const BLOCK16_BYTES: usize = 16;

const DISTANCES: [i32; 8] = [3, 6, 11, 16, 23, 32, 41, 64];

/// EAC modifier tables (spec table C.5); the first index is the table number.
const EAC_MODIFIERS: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

fn clamp255(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Decode one RGB8_ETC2 block (`punchthrough` false) or one
/// RGB8_PUNCHTHROUGH_ALPHA1_ETC2 block (true) to 16 RGBA8 texels. An
/// RGB8_ETC2 block's alpha is 255; a punch-through block's is 255, or 0
/// (with R = G = B = 0) for the texels the format makes transparent.
pub fn decode_rgb_block(block: &[u8; BLOCK_BYTES], punchthrough: bool) -> [[u8; 4]; 16] {
    let src = u64::from_be_bytes(*block);
    /* bit 33 is the diff bit; with punch-through it is the opaque bit */
    let flag = get_bit(src, 33) != 0;
    let opaque = !punchthrough || flag;
    let br = get_bits(src, 59, 63) as i32;
    let bg = get_bits(src, 51, 55) as i32;
    let bb = get_bits(src, 43, 47) as i32;
    let signed3 = |v: u32| if v & 4 != 0 { v as i32 - 8 } else { v as i32 };
    let r2 = br + signed3(get_bits(src, 56, 58));
    let g2 = bg + signed3(get_bits(src, 48, 50));
    let b2 = bb + signed3(get_bits(src, 40, 42));
    let out_of_range = |v: i32| !(0..=31).contains(&v);

    let mut out = [[0u8; 4]; 16];
    let alpha_of = |idx: u32| if !opaque && idx == 2 { 0u8 } else { 255 };
    /* The index of texel (x, y): the column-major bit pair. */
    let index = |x: u32, y: u32| (get_bit(src, 16 + x * 4 + y) << 1) | get_bit(src, x * 4 + y);

    if (flag || punchthrough) && out_of_range(r2) {
        /* T mode */
        let r1 = (get_bits(src, 59, 60) << 2) | get_bits(src, 56, 57);
        let c1 = [extend4_to_8(r1), extend4_to_8(get_bits(src, 52, 55)), extend4_to_8(get_bits(src, 48, 51))];
        let c2 = [
            extend4_to_8(get_bits(src, 44, 47)),
            extend4_to_8(get_bits(src, 40, 43)),
            extend4_to_8(get_bits(src, 36, 39)),
        ];
        let d = DISTANCES[((get_bits(src, 34, 35) << 1) | get_bit(src, 32)) as usize];
        let paint = |i: u32| -> [i32; 3] {
            match i {
                0 => c1,
                1 => [c2[0] + d, c2[1] + d, c2[2] + d],
                2 => c2,
                _ => [c2[0] - d, c2[1] - d, c2[2] - d],
            }
        };
        for y in 0..4 {
            for x in 0..4 {
                let idx = index(x, y);
                let p = paint(idx);
                out[(y * 4 + x) as usize] =
                    if alpha_of(idx) == 0 { [0; 4] } else { [clamp255(p[0]), clamp255(p[1]), clamp255(p[2]), 255] };
            }
        }
    } else if (flag || punchthrough) && out_of_range(g2) {
        /* H mode */
        /* G1 = bits 58..56 (3) + bit 52 (1); B1 = bit 51, bits 49..48 (2), bit 47 */
        let c1 = [
            extend4_to_8(get_bits(src, 59, 62)),
            extend4_to_8((get_bits(src, 56, 58) << 1) | get_bit(src, 52)),
            extend4_to_8((get_bit(src, 51) << 3) | (get_bits(src, 48, 49) << 1) | get_bit(src, 47)),
        ];
        let c2 = [
            extend4_to_8(get_bits(src, 43, 46)),
            extend4_to_8((get_bits(src, 40, 42) << 1) | get_bit(src, 39)),
            extend4_to_8(get_bits(src, 35, 38)),
        ];
        let v1 = ((c1[0] << 16) + (c1[1] << 8) + c1[2]) as u32;
        let v2 = ((c2[0] << 16) + (c2[1] << 8) + c2[2]) as u32;
        let di = (get_bit(src, 34) << 2) | (get_bit(src, 32) << 1) | u32::from(v1 >= v2);
        let d = DISTANCES[di as usize];
        let paint = |i: u32| -> [i32; 3] {
            let (c, s) = match i {
                0 => (c1, d),
                1 => (c1, -d),
                2 => (c2, d),
                _ => (c2, -d),
            };
            [c[0] + s, c[1] + s, c[2] + s]
        };
        for y in 0..4 {
            for x in 0..4 {
                let idx = index(x, y);
                let p = paint(idx);
                out[(y * 4 + x) as usize] =
                    if alpha_of(idx) == 0 { [0; 4] } else { [clamp255(p[0]), clamp255(p[1]), clamp255(p[2]), 255] };
            }
        }
    } else if (flag || punchthrough) && out_of_range(b2) {
        /* planar mode (always opaque) */
        let ro = get_bits(src, 57, 62);
        let go = (get_bit(src, 56) << 6) | get_bits(src, 49, 54);
        let bo = (get_bit(src, 48) << 5) | (get_bits(src, 43, 44) << 3) | get_bits(src, 39, 41);
        let rh = (get_bits(src, 34, 38) << 1) | get_bit(src, 32);
        let gh = get_bits(src, 25, 31);
        let bh = get_bits(src, 19, 24);
        let rv = get_bits(src, 13, 18);
        let gv = get_bits(src, 6, 12);
        let bv = get_bits(src, 0, 5);
        let ext67 = |v: u32| (v << 2) | (v >> 4);
        let ext7 = |v: u32| (v << 1) | (v >> 6);
        let o = [ext67(ro) as i32, ext7(go) as i32, ext67(bo) as i32];
        let h = [ext67(rh) as i32, ext7(gh) as i32, ext67(bh) as i32];
        let v = [ext67(rv) as i32, ext7(gv) as i32, ext67(bv) as i32];
        for y in 0..4i32 {
            for x in 0..4i32 {
                let ch = |c: usize| clamp255((x * (h[c] - o[c]) + y * (v[c] - o[c]) + 4 * o[c] + 2) >> 2);
                out[(y * 4 + x) as usize] = [ch(0), ch(1), ch(2), 255];
            }
        }
    } else {
        /* individual (flag clear, not punch-through) or differential mode */
        let (c0, c1);
        if !punchthrough && !flag {
            c0 = [
                extend4_to_8(get_bits(src, 60, 63)),
                extend4_to_8(get_bits(src, 52, 55)),
                extend4_to_8(get_bits(src, 44, 47)),
            ];
            c1 = [
                extend4_to_8(get_bits(src, 56, 59)),
                extend4_to_8(get_bits(src, 48, 51)),
                extend4_to_8(get_bits(src, 40, 43)),
            ];
        } else {
            c0 = [extend5_to_8(br as u32), extend5_to_8(bg as u32), extend5_to_8(bb as u32)];
            c1 = [extend5_to_8(r2 as u32), extend5_to_8(g2 as u32), extend5_to_8(b2 as u32)];
        }
        let tables = [get_bits(src, 37, 39) as usize, get_bits(src, 34, 36) as usize];
        let flip = get_bit(src, 32) != 0;
        let base = [c0, c1];
        for y in 0..4u32 {
            for x in 0..4u32 {
                let idx = index(x, y);
                let sub = usize::from(if flip { y } else { x } >= 2);
                out[(y * 4 + x) as usize] = if alpha_of(idx) == 0 {
                    [0; 4]
                } else {
                    /* The non-opaque punch-through table is 0, +m, (transparent),
                     * -m, with m the ETC1 table's second entry. */
                    let t = &MODIFIERS[tables[sub]];
                    let m = if opaque { t[idx as usize] } else { [0, t[1], 0, -t[1]][idx as usize] };
                    let b = base[sub];
                    [clamp255(b[0] + m), clamp255(b[1] + m), clamp255(b[2] + m), 255]
                };
            }
        }
    }
    out
}

/// The 16 values of an EAC alpha / single-channel block as (base, multiplier,
/// table, per-texel modifier index), texel (x, y) at `y * 4 + x`.
fn eac_parts(block: &[u8; BLOCK_BYTES]) -> (i32, i32, usize, [usize; 16]) {
    let src = u64::from_be_bytes(*block);
    let base = (src >> 56) as i32;
    let mult = get_bits(src, 52, 55) as i32;
    let table = get_bits(src, 48, 51) as usize;
    let idx = core::array::from_fn(|i| {
        let (x, y) = ((i % 4) as u32, (i / 4) as u32);
        /* 3-bit indices, column-major, texel (0,0) at the top of the 48 bits */
        get_bits(src, (3 - y + (3 - x) * 4) * 3, (3 - y + (3 - x) * 4) * 3 + 2) as usize
    });
    (base, mult, table, idx)
}

/// RGBA8_ETC2_EAC's alpha half (dEQP `decompressEAC8Block`): 16 alpha values.
pub fn decode_eac8_block(block: &[u8; BLOCK_BYTES]) -> [u8; 16] {
    let (base, mult, table, idx) = eac_parts(block);
    idx.map(|i| clamp255(base + EAC_MODIFIERS[table][i] * mult))
}

/// R11_EAC / RG11_EAC channel block (unsigned): 16 values widened to 16 bits.
pub fn decode_eac11_unsigned_block(block: &[u8; BLOCK_BYTES]) -> [u16; 16] {
    let (base, mult, table, idx) = eac_parts(block);
    idx.map(|i| {
        let m = EAC_MODIFIERS[table][i];
        let v = (base << 3) + 4 + if mult != 0 { (m * mult) << 3 } else { m };
        let v = v.clamp(0, 2047) as u32;
        ((v << 5) | (v >> 6)) as u16
    })
}

/// SIGNED_R11_EAC / SIGNED_RG11_EAC channel block: 16 values widened to a
/// signed 16-bit range (-32767..=32767); a base of -128 counts as -127.
pub fn decode_eac11_signed_block(block: &[u8; BLOCK_BYTES]) -> [i16; 16] {
    let (base, mult, table, idx) = eac_parts(block);
    let base = (base as u8 as i8 as i32).max(-127);
    idx.map(|i| {
        let m = EAC_MODIFIERS[table][i];
        let v = (base << 3) + if mult != 0 { (m * mult) << 3 } else { m };
        let v = v.clamp(-1023, 1023);
        let a = v.unsigned_abs();
        let w = ((a << 5) | (a >> 5)) as i16;
        if v < 0 {
            -w
        } else {
            w
        }
    })
}

/// One RGBA8_ETC2_EAC block (alpha half first): 16 RGBA8 texels.
pub fn decode_rgba_block(block: &[u8; BLOCK16_BYTES]) -> [[u8; 4]; 16] {
    let a = decode_eac8_block(block[..8].try_into().unwrap());
    let mut t = decode_rgb_block(block[8..].try_into().unwrap(), false);
    for (t, a) in t.iter_mut().zip(a) {
        t[3] = a;
    }
    t
}

/// Walk an image of `w` x `h` texels stored as rows of `ceil(w/4)` blocks of
/// `bytes` each; `put(block, x0, y0)` gets each block and its texel origin.
/// Returns false (touching nothing) when `data` is shorter than the blocks.
fn for_blocks(w: usize, h: usize, data: &[u8], bytes: usize, mut put: impl FnMut(&[u8], usize, usize)) -> bool {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    if data.len() < bw * bh * bytes {
        return false;
    }
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * bytes;
            put(&data[o..o + bytes], bx * 4, by * 4);
        }
    }
    true
}

fn put_rgba(out: &mut [u8], w: usize, h: usize, x0: usize, y0: usize, t: &[[u8; 4]; 16]) {
    for y in 0..4.min(h - y0) {
        for x in 0..4.min(w - x0) {
            let o = ((y0 + y) * w + x0 + x) * 4;
            out[o..o + 4].copy_from_slice(&t[y * 4 + x]);
        }
    }
}

/// A whole RGB8_ETC2 / SRGB8_ETC2 (`punchthrough` false) or
/// RGB8_PUNCHTHROUGH_ALPHA1_ETC2 / SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 image to
/// `w * h * 4` RGBA8 bytes, rows packed. Returns false when `data` or `out`
/// is too short.
pub fn decode_rgb_image(w: usize, h: usize, data: &[u8], punchthrough: bool, out: &mut [u8]) -> bool {
    if out.len() < w * h * 4 {
        return false;
    }
    for_blocks(w, h, data, BLOCK_BYTES, |b, x0, y0| {
        put_rgba(out, w, h, x0, y0, &decode_rgb_block(b.try_into().unwrap(), punchthrough))
    })
}

/// A whole RGBA8_ETC2_EAC / SRGB8_ALPHA8_ETC2_EAC image to RGBA8 bytes.
pub fn decode_rgba_image(w: usize, h: usize, data: &[u8], out: &mut [u8]) -> bool {
    if out.len() < w * h * 4 {
        return false;
    }
    for_blocks(w, h, data, BLOCK16_BYTES, |b, x0, y0| {
        put_rgba(out, w, h, x0, y0, &decode_rgba_block(b.try_into().unwrap()))
    })
}

/// A whole R11_EAC image (`channels` 1) or RG11_EAC image (2) to `w * h *
/// channels` unsigned 16-bit values, rows packed, red first.
pub fn decode_eac11_unsigned_image(w: usize, h: usize, channels: usize, data: &[u8], out: &mut [u16]) -> bool {
    if !(1..=2).contains(&channels) || out.len() < w * h * channels {
        return false;
    }
    for_blocks(w, h, data, BLOCK_BYTES * channels, |b, x0, y0| {
        for c in 0..channels {
            let t = decode_eac11_unsigned_block(b[c * 8..c * 8 + 8].try_into().unwrap());
            for y in 0..4.min(h - y0) {
                for x in 0..4.min(w - x0) {
                    out[((y0 + y) * w + x0 + x) * channels + c] = t[y * 4 + x];
                }
            }
        }
    })
}

/// As [`decode_eac11_unsigned_image`] for SIGNED_R11_EAC / SIGNED_RG11_EAC.
pub fn decode_eac11_signed_image(w: usize, h: usize, channels: usize, data: &[u8], out: &mut [i16]) -> bool {
    if !(1..=2).contains(&channels) || out.len() < w * h * channels {
        return false;
    }
    for_blocks(w, h, data, BLOCK_BYTES * channels, |b, x0, y0| {
        for c in 0..channels {
            let t = decode_eac11_signed_block(b[c * 8..c * 8 + 8].try_into().unwrap());
            for y in 0..4.min(h - y0) {
                for x in 0..4.min(w - x0) {
                    out[((y0 + y) * w + x0 + x) * channels + c] = t[y * 4 + x];
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    //! Run with `cargo test --no-default-features --features
    //! host-test,gpu-vivante,shaderc-online --lib gl::etc2`; the module has
    //! no feature dependence, any host feature set of the `--lib` rows does.
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }
    /// RGB8_ETC2 blocks (individual, differential, T, H, planar) as (name, block bytes, RGBA8 texels), hex.
    const RGB: &[(&str, &str, &str)] = &[
        ("rgb-H", "74066676cfb0b4eb", "ce6824ffececffffececffffce6824ffce6824ffacacceffececffffce6824ffffa864ffce6824ffacacceffececffffce6824ffacacceffececffffacacceff"),
        ("rgb-T", "fa0ff0169dc95756", "ff0011ffff0b1cfff40006fff40006ffff0b1cffee00ffffff0b1cffee00ffffff0b1cfff40006fff40006ffff0b1cffff0011ffff0011ffff0011ffff0011ff"),
        ("rgb-diff", "887534a20f0b0d04", "745b19ffa48b49ff845208ff8e5c12ff745b19ffa48b49ff8a580eff8e5c12ffdcc381ffa48b49ff845208ff8e5c12ff745b19ffa48b49ff845208ff8e5c12ff"),
        ("rgb-ind", "4420823cfde6f1c2", "49278dff49278dff000000ff000000ff331177ff3f1d83ff732f51ff000000ff3f1d83ff331177ff150000ff000000ff49278dff331177ff150000ff000000ff"),
        ("rgb-planar", "6b30f90ec7dd01e4", "d7b169ffa7b78bff78bcacff48c2ceffca8873ff9a8e95ff6a93b6ff3b99d8ffbd607eff8d659fff5d6bc1ff2d70e2ffaf3788ff803ca9ff5042cbff2047ecff"),
    ];

    /// RGB8_PUNCHTHROUGH_ALPHA1_ETC2: opaque bit set or clear, every mode, with transparent texels (0,0,0,0).
    const PT: &[(&str, &str, &str)] = &[
        ("pt-H-nonopaque", "260ef3c1233dfbff", "e36c7dffe36c7dffe36c7dff39c1d2ff39c1d2ffe36c7dffe36c7dffe36c7dffe36c7dff39c1d2ff4fd7e8ff39c1d2ffe36c7dff39c1d2ff39c1d2ff39c1d2ff"),
        ("pt-H-nonopaque-transp", "51fa68bc546ce0da", "c14ae3ff931cb5ffc14ae3ff00000000931cb5ff00000000c14ae3ff931cb5ff00000000c60060ff00000000c60060ffc60060ff931cb5ffc14ae3ff931cb5ff"),
        ("pt-H-opaque", "d80e883785d2dece", "fb40ffff5140a6ff5140a6ff7b009dff000026fffb40ffff7b009dfffb40ffff7b009dff000026ff000026ff7b009dff7b009dff000026ff7b009dff000026ff"),
        ("pt-T-nonopaque-transp", "1c0fd94969c9fab3", "bd7924fffdb964ff00000000fdb964fffdb964fffdb964fffdb964ffbd7924ffcc00ffff00000000cc00ffffbd7924ff00000000bd7924ffbd7924fffdb964ff"),
        ("pt-T-opaque", "05d360174cd4c26e", "11dd33ff660011ff11dd33ff11dd33ff761021ff761021ff761021ff11dd33ff560001ff560001ff660011ff560001ff761021ff660011ff660011ff761021ff"),
        ("pt-diff-nonopaque", "95f87d7988849b84", "94ff7bff94ff7bffbeffa5ffbeffa5ff94ff7bff94ff7bffbeffa5ff94ff7bff119500ff7bff63ff7bff63ff7bff63ff7bff63ff119500ff119500ff119500ff"),
        ("pt-diff-nonopaque-transp", "52bced75757a66ba", "52bdefff2893c5ff00000000000000002893c5ff2893c5ff7ce7ffff2893c5ff639cd6ff00000000134c86ff134c86ff134c86ffb3ecffff639cd6ff639cd6ff"),
        ("pt-diff-opaque", "629677d705203e27", "cdfeddff84b594ff427352ffcdfeddffcdfeddff002a09ffcdfeddffcdfeddffc3d4bbff8b9c83ff23341bff8b9c83ff8b9c83ff8b9c83ffc3d4bbff8b9c83ff"),
        ("pt-planar-nonopaque", "68a7ebb1d4204c81", "d326beffb85293ff9c7e67ff81a93cffa03690ff856164ff698d39ff4eb90dff6e4561ff527136ff379d0aff1bc800ff3b5533ff1f8007ff04ac00ff00d800ff"),
        ("pt-planar-opaque", "a465f96bde9f9976", "4964ebff6d83c4ff90a29cffb4c075ff747ee7ff979dc0ffbbbb98ffdeda71ff9e98e3ffc2b6bcffe5d594fffff46dffc9b1dfffecd0b8ffffef90ffffff69ff"),
    ];

    /// RGBA8_ETC2_EAC: alpha half first (multiplier 0 and 15, table 13, clamping).
    const RGBA: &[(&str, &str, &str)] = &[
        ("rgba-0", "805310434d5967df887534a20f0b0d04", "745b1976a48b496c845208628e5c123f745b1985a48b498f8a580e998e5c12bcdcc38176a48b496c845208628e5c123f745b1985a48b498f845208998e5c12bc"),
        ("rgba-1", "fff0efbcb2a69820fa0ff0169dc95756", "ff0011ffff0b1cfff40006fff40006ffff0b1c1eee00ff78ff0b1ca5ee00ffd2ff0b1cfff40006fff40006ffff0b1cffff00111eff001178ff0011a5ff0011d2"),
        ("rgba-2", "0a0d10434d5967df6b30f90ec7dd01e4", "d7b1690aa7b78b0a78bcac0a48c2ce0aca88730a9a8e950a6a93b60a3b99d80abd607e0a8d659f0a5d6bc10a2d70e20aaf37880a803ca90a5042cb0a2047ec0a"),
    ];

    /// R11_EAC: u16 little-endian per texel.
    const R11: &[(&str, &str, &str)] = &[
        ("r11-0", "000010434d5967df", "2000000000000000c0002001800140022000000000000000c000200180014002"),
        ("r11-1", "801d10434d5967df", "8f7f8f7e8f7d8e7690809081908291898f7f8f7e8f7d8e769080908190829189"),
        ("r11-2", "fff010434d5967df", "9ad294a58f78831effffffffffffffff9ad294a58f78831effffffffffffffff"),
        ("r11-3", "fff0efbcb2a69820", "ffffffffffffffff831e8f7894a59ad2ffffffffffffffff831e8f7894a59ad2"),
        ("r11-4", "00f010434d5967df", "0000000000000000831e894b8f789ad20000000000000000831e894b8f789ad2"),
        ("r11-5", "6479efbcb2a69820", "94a3929590808d6b831e852c88418a5694a3929590808d6b831e852c88418a56"),
        ("r11-6", "2803efbcb2a69820", "052a2529e528a528e426c42705284528052a2529e528a528e426c42705284528"),
    ];

    /// SIGNED_R11_EAC: i16 little-endian per texel; bases -128 (as -127), 127, multiplier 0 and 15.
    const SR11: &[(&str, &str, &str)] = &[
        ("sr11-0", "80f010434d5967df", "0180018001800180e89ef3cbfff814530180018001800180e89ef3cbfff81453"),
        ("sr11-1", "80f0efbcb2a69820", "1453fff8f3cbe89e01800180018001801453fff8f3cbe89e0180018001800180"),
        ("sr11-2", "7ff010434d5967df", "14520925fef7e89dff7fff7fff7fff7f14520925fef7e89dff7fff7fff7fff7f"),
        ("sr11-3", "7ff0efbcb2a69820", "ff7fff7fff7fff7fe89dfef709251452ff7fff7fff7fff7fe89dfef709251452"),
        ("sr11-4", "810510434d5967df", "81800180018001802181a181e181218281800180018001802181a181e1812182"),
        ("sr11-5", "1037efbcb2a69820", "0b2e0925071c0516fceefef7000101070b2e0925071c0516fceefef700010107"),
        ("sr11-6", "f02d10434d5967df", "fcedfbebfbe9f7dbfceffdf1fdf30002fcedfbebfbe9f7dbfceffdf1fdf30002"),
    ];

    /// RG11_EAC: red then green per texel, u16 little-endian.
    const RG11: &[(&str, &str, &str)] = &[
        ("rg11-0", "fff0efbcb2a6982000f010434d5967df", "ffff0000ffff0000ffff0000ffff0000831e831e8f78894b94a58f789ad29ad2ffff0000ffff0000ffff0000ffff0000831e831e8f78894b94a58f789ad29ad2"),
        ("rg11-1", "647910434d5967dfc812efbcb2a69820", "8a569ad4884199cf852c99cc831e99c98d6b97bb908098c0929598c394a398c68a569ad4884199cf852c99cc831e99c98d6b97bb908098c0929598c394a398c6"),
    ];

    /// SIGNED_RG11_EAC: red then green per texel, i16 little-endian.
    const SRG11: &[(&str, &str, &str)] = &[
        ("srg11-0", "80f010434d5967df7ff0efbcb2a69820", "0180ff7f0180ff7f0180ff7f0180ff7fe89ee89df3cbfef7fff80925145314520180ff7f0180ff7f0180ff7f0180ff7fe89ee89df3cbfef7fff8092514531452"),
        ("srg11-1", "103710434d5967dff02defbcb2a69820", "010700020001fdf3fef7fdf1fceefcef0516f7db071cfbe90925fbeb0b2efced010700020001fdf3fef7fdf1fceefcef0516f7db071cfbe90925fbeb0b2efced"),
    ];

    fn le16(v: impl IntoIterator<Item = [u8; 2]>) -> Vec<u8> {
        v.into_iter().flatten().collect()
    }

    fn check(table: &[(&str, &str, &str)], f: impl Fn(&[u8]) -> Vec<u8>) {
        for (name, block, texels) in table {
            let got: String = f(&unhex(block)).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(got, *texels, "{name} {block}");
        }
    }

    #[test]
    fn rgb8_every_mode_against_mesa() {
        check(RGB, |b| decode_rgb_block(b.try_into().unwrap(), false).concat());
    }

    #[test]
    fn punchthrough_every_mode_against_mesa() {
        check(PT, |b| decode_rgb_block(b.try_into().unwrap(), true).concat());
        /* the vectors do hold transparent texels, and only with the opaque bit clear */
        for (name, block, texels) in PT {
            let t = unhex(texels);
            let transparent = t.chunks(4).any(|p| p == [0, 0, 0, 0]);
            assert_eq!(transparent, name.contains("transp"), "{name}");
            if name.contains("-opaque") {
                assert!(t.chunks(4).all(|p| p[3] == 255), "{name} {block}");
            }
        }
    }

    #[test]
    fn opaque_differential_is_the_same_in_both_formats() {
        /* the same bytes: RGB8_ETC2 reads bit 33 as the diff bit, so an
         * opaque differential block decodes identically in both formats */
        let (_, block, texels) = PT.iter().find(|v| v.0 == "pt-diff-opaque").unwrap();
        let got = decode_rgb_block(&unhex(block).try_into().unwrap(), false).concat();
        assert_eq!(got, unhex(texels));
    }

    #[test]
    fn rgba8_eac_against_mesa() {
        check(RGBA, |b| decode_rgba_block(b.try_into().unwrap()).concat());
    }

    #[test]
    fn r11_against_mesa() {
        check(R11, |b| le16(decode_eac11_unsigned_block(b.try_into().unwrap()).map(u16::to_le_bytes)));
        /* 11-bit extremes widen to the full 16-bit range */
        let all_hi = unhex("fff0efbcb2a69820");
        assert_eq!(decode_eac11_unsigned_block(&all_hi.try_into().unwrap())[0], 0xffff);
    }

    #[test]
    fn signed_r11_against_mesa() {
        check(SR11, |b| le16(decode_eac11_signed_block(b.try_into().unwrap()).map(i16::to_le_bytes)));
        /* -1023 and 1023 clamp to -32767 and 32767, never -32768 */
        let lo = decode_eac11_signed_block(&unhex("80f010434d5967df").try_into().unwrap());
        assert_eq!(lo[0], -32767);
        let hi = decode_eac11_signed_block(&unhex("7ff0efbcb2a69820").try_into().unwrap());
        assert_eq!(hi[0], 32767);
    }

    #[test]
    fn rg11_against_mesa() {
        check(RG11, |b| {
            let (r, g) = (
                decode_eac11_unsigned_block(b[..8].try_into().unwrap()),
                decode_eac11_unsigned_block(b[8..].try_into().unwrap()),
            );
            le16((0..16).flat_map(|i| [r[i].to_le_bytes(), g[i].to_le_bytes()]))
        });
    }

    #[test]
    fn signed_rg11_against_mesa() {
        check(SRG11, |b| {
            let (r, g) = (
                decode_eac11_signed_block(b[..8].try_into().unwrap()),
                decode_eac11_signed_block(b[8..].try_into().unwrap()),
            );
            le16((0..16).flat_map(|i| [r[i].to_le_bytes(), g[i].to_le_bytes()]))
        });
    }

    #[test]
    fn eac8_alpha_by_hand() {
        /* base 128, multiplier 5, table 3 (-2 -4 -6 -13 1 3 5 12), indices
         * cycling 0..7 along the 16 texels in column-major order */
        let a = decode_eac8_block(&unhex("805310434d5967df").try_into().unwrap());
        assert_eq!(a[0], 118); /* texel (0,0): index 0, 128 + -2*5 */
        assert_eq!(a[1], 108); /* texel (1,0): index 1, 128 + -4*5 */
        assert_eq!(a[4], 133); /* texel (0,1): index 4, 128 + 1*5 */
    }

    #[test]
    fn image_helpers_crop_and_pack() {
        /* 5x3 texels = 2x1 blocks; the second block is cropped to 1 column */
        let (_, b0, t0) = RGB[0];
        let (_, b1, t1) = RGB[1];
        let data = [unhex(b0), unhex(b1)].concat();
        let mut out = vec![0u8; 5 * 3 * 4];
        assert!(decode_rgb_image(5, 3, &data, false, &mut out));
        let (t0, t1) = (unhex(t0), unhex(t1));
        for y in 0..3 {
            assert_eq!(out[y * 20..y * 20 + 16], t0[y * 16..y * 16 + 16]);
            assert_eq!(out[y * 20 + 16..y * 20 + 20], t1[y * 16..y * 16 + 4]);
        }
        assert!(!decode_rgb_image(5, 3, &data[..8], false, &mut out), "short data");
        assert!(!decode_rgb_image(5, 3, &data, false, &mut out[..10]), "short out");

        /* RG11: two channels interleaved in one pass over 2x2 texels */
        let (_, b, _) = RG11[1];
        let blk = unhex(b);
        let mut rg = [0u16; 8];
        assert!(decode_eac11_unsigned_image(2, 2, 2, &blk, &mut rg));
        let r = decode_eac11_unsigned_block(blk[..8].try_into().unwrap());
        let g = decode_eac11_unsigned_block(blk[8..].try_into().unwrap());
        assert_eq!(rg, [r[0], g[0], r[1], g[1], r[4], g[4], r[5], g[5]]);
        let mut srg = [0i16; 8];
        let (_, b, _) = SRG11[1];
        let blk = unhex(b);
        assert!(decode_eac11_signed_image(2, 2, 2, &blk, &mut srg));
        let r = decode_eac11_signed_block(blk[..8].try_into().unwrap());
        assert_eq!(srg[0], r[0]);
        assert_eq!(srg[6], r[5]);
    }
}
