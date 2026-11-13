// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The stored texels of the six ETC2/EAC formats the GL core decodes at
//! upload (docs/notes/etc2-eac.md): `R11_EAC`, `SIGNED_R11_EAC`, `RG11_EAC`,
//! `SIGNED_RG11_EAC`, `RGB8_PUNCHTHROUGH_ALPHA1_ETC2` and
//! `SRGB8_PUNCHTHROUGH_ALPHA1_ETC2`. Pure functions over bytes, no GL state:
//! a compressed image or a block-aligned sub-rectangle goes in, tightly
//! packed texels of the stored class come out.
//!
//! Stored classes: R11 -> R16F (2 bytes), RG11 -> RGBA16F (8 bytes, (r, g,
//! 0, 1.0)), punch-through -> RGBA8 / SRGB8_ALPHA8 (4 bytes). Half floats are
//! little-endian, converted with `es3_formats::f32_to_half`; unsigned 11-bit
//! values are `u16 / 65535`, signed `max(i16 / 32767, -1)`.

use super::es3_formats::f32_to_half;
use super::etc2;

const GL_R11_EAC: u32 = 0x9270;
const GL_SIGNED_R11_EAC: u32 = 0x9271;
const GL_RG11_EAC: u32 = 0x9272;
const GL_SIGNED_RG11_EAC: u32 = 0x9273;
const GL_RGB8_PUNCHTHROUGH_ALPHA1_ETC2: u32 = 0x9276;
const GL_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2: u32 = 0x9277;

const GL_R16F: u32 = 0x822D;
const GL_RGBA16F: u32 = 0x881A;
const GL_RGBA8: u32 = 0x8058;
const GL_SRGB8_ALPHA8: u32 = 0x8C43;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    R11,
    SignedR11,
    Rg11,
    SignedRg11,
    Punch,
}

/// One decoded format: what it is stored as and its block geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedClass {
    kind: Kind,
    /// The compressed name this class was made for.
    pub compressed: u32,
    /// The stored GL internal format.
    pub stored_format: u32,
    /// Bytes per stored texel.
    pub texel_bytes: usize,
    /// Bytes per compressed 4x4 block.
    pub block_bytes: usize,
}

/// The class of a decoded-at-upload name; `None` for every other name,
/// including ETC1 and the four ETC2 names the GL core stores natively.
pub fn decoded_class(internalformat: u32) -> Option<DecodedClass> {
    let (kind, stored_format, texel_bytes, block_bytes) = match internalformat {
        GL_R11_EAC => (Kind::R11, GL_R16F, 2, 8),
        GL_SIGNED_R11_EAC => (Kind::SignedR11, GL_R16F, 2, 8),
        GL_RG11_EAC => (Kind::Rg11, GL_RGBA16F, 8, 16),
        GL_SIGNED_RG11_EAC => (Kind::SignedRg11, GL_RGBA16F, 8, 16),
        GL_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 => (Kind::Punch, GL_RGBA8, 4, 8),
        GL_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 => (Kind::Punch, GL_SRGB8_ALPHA8, 4, 8),
        _ => return None,
    };
    Some(DecodedClass { kind, compressed: internalformat, stored_format, texel_bytes, block_bytes })
}

/// Bytes of a compressed w x h image: `ceil(w/4) * ceil(h/4) * block_bytes`.
pub fn compressed_size(class: DecodedClass, w: usize, h: usize) -> usize {
    w.div_ceil(4) * h.div_ceil(4) * class.block_bytes
}

fn unsigned_half(v: u16) -> [u8; 2] {
    f32_to_half(v as f32 / 65535.0).to_le_bytes()
}

fn signed_half(v: i16) -> [u8; 2] {
    f32_to_half((v as f32 / 32767.0).max(-1.0)).to_le_bytes()
}

/// The 16 stored texels of one block, row-major; each is `texel_bytes` long.
fn block_texels(class: DecodedClass, blk: &[u8]) -> [[u8; 8]; 16] {
    let mut t = [[0u8; 8]; 16];
    let b8 = |o: usize| -> [u8; 8] { blk[o..o + 8].try_into().unwrap() };
    match class.kind {
        Kind::R11 => {
            for (i, v) in etc2::decode_eac11_unsigned_block(&b8(0)).iter().enumerate() {
                t[i][..2].copy_from_slice(&unsigned_half(*v));
            }
        }
        Kind::SignedR11 => {
            for (i, v) in etc2::decode_eac11_signed_block(&b8(0)).iter().enumerate() {
                t[i][..2].copy_from_slice(&signed_half(*v));
            }
        }
        Kind::Rg11 => {
            let r = etc2::decode_eac11_unsigned_block(&b8(0));
            let g = etc2::decode_eac11_unsigned_block(&b8(8));
            for i in 0..16 {
                t[i][..2].copy_from_slice(&unsigned_half(r[i]));
                t[i][2..4].copy_from_slice(&unsigned_half(g[i]));
            }
        }
        Kind::SignedRg11 => {
            let r = etc2::decode_eac11_signed_block(&b8(0));
            let g = etc2::decode_eac11_signed_block(&b8(8));
            for i in 0..16 {
                t[i][..2].copy_from_slice(&signed_half(r[i]));
                t[i][2..4].copy_from_slice(&signed_half(g[i]));
            }
        }
        Kind::Punch => {
            for (i, px) in etc2::decode_rgb_block(&b8(0), true).iter().enumerate() {
                t[i][..4].copy_from_slice(px);
            }
        }
    }
    if matches!(class.kind, Kind::Rg11 | Kind::SignedRg11) {
        // blue stays 0, alpha is 1.0
        let one = f32_to_half(1.0).to_le_bytes();
        for px in t.iter_mut() {
            px[6..8].copy_from_slice(&one);
        }
    }
    t
}

/// Decode a whole w x h image into `out`, tightly packed stored texels.
/// Edge blocks decode whole and keep the texels inside the image. Returns
/// false and writes nothing when `data` or `out` is too short.
pub fn decode_into(class: DecodedClass, w: usize, h: usize, data: &[u8], out: &mut [u8]) -> bool {
    if data.len() < compressed_size(class, w, h) || out.len() < w * h * class.texel_bytes {
        return false;
    }
    decode_blocks(class, data, (0, 0), (w, h), out, (w, h));
    true
}

/// Decode a block-aligned sub-rectangle (`xoff`, `yoff` multiples of 4; `w`,
/// `h` multiples of 4 or reaching the level's edge) into the `dst_w` x
/// `dst_h` level buffer `dst`. Texels outside the level are dropped. Returns
/// false and writes nothing on a misaligned rectangle or a short buffer.
#[allow(clippy::too_many_arguments)]
pub fn decode_sub_into(
    class: DecodedClass,
    data: &[u8],
    xoff: usize,
    yoff: usize,
    w: usize,
    h: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) -> bool {
    if xoff % 4 != 0
        || yoff % 4 != 0
        || data.len() < compressed_size(class, w, h)
        || dst.len() < dst_w * dst_h * class.texel_bytes
        || (w % 4 != 0 && xoff + w < dst_w)
        || (h % 4 != 0 && yoff + h < dst_h)
    {
        return false;
    }
    decode_blocks(class, data, (xoff, yoff), (w, h), dst, (dst_w, dst_h));
    true
}

fn decode_blocks(
    class: DecodedClass,
    data: &[u8],
    off: (usize, usize),
    size: (usize, usize),
    dst: &mut [u8],
    dst_size: (usize, usize),
) {
    let tb = class.texel_bytes;
    let (w, h) = size;
    let bw = w.div_ceil(4);
    for by in 0..h.div_ceil(4) {
        for bx in 0..bw {
            let o = (by * bw + bx) * class.block_bytes;
            let texels = block_texels(class, &data[o..o + class.block_bytes]);
            for ty in 0..4 {
                let y = off.1 + by * 4 + ty;
                if by * 4 + ty >= h || y >= dst_size.1 {
                    break;
                }
                for tx in 0..4 {
                    let x = off.0 + bx * 4 + tx;
                    if bx * 4 + tx >= w || x >= dst_size.0 {
                        break;
                    }
                    let d = (y * dst_size.0 + x) * tb;
                    dst[d..d + tb].copy_from_slice(&texels[ty * 4 + tx][..tb]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Run with `cargo test --no-default-features --features
    //! host-test,gpu-vivante,shaderc-online --lib gl::etc2_store`; the module
    //! has no feature dependence, any host feature set of the `--lib` rows does.
    use super::*;
    use crate::gl::es3_formats::half_to_f32;

    fn pseudo(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

    const ALL: [u32; 6] = [0x9270, 0x9271, 0x9272, 0x9273, 0x9276, 0x9277];

    fn half_at(b: &[u8], i: usize) -> f32 {
        half_to_f32(u16::from_le_bytes([b[2 * i], b[2 * i + 1]]))
    }

    #[test]
    fn classes_and_none() {
        let c = decoded_class(0x9270).unwrap();
        assert_eq!((c.stored_format, c.texel_bytes, c.block_bytes), (0x822D, 2, 8));
        let c = decoded_class(0x9273).unwrap();
        assert_eq!((c.stored_format, c.texel_bytes, c.block_bytes), (0x881A, 8, 16));
        assert_eq!(decoded_class(0x9276).unwrap().stored_format, 0x8058);
        assert_eq!(decoded_class(0x9277).unwrap().stored_format, 0x8C43);
        for n in [0x8D64, 0x9274, 0x9275, 0x9278, 0x9279, 0x8058, 0] {
            assert!(decoded_class(n).is_none(), "{n:#x}");
        }
    }

    #[test]
    fn sizes() {
        let r = decoded_class(0x9270).unwrap();
        let rg = decoded_class(0x9272).unwrap();
        let p = decoded_class(0x9276).unwrap();
        assert_eq!(compressed_size(r, 1, 1), 8);
        assert_eq!(compressed_size(r, 3, 5), 16);
        assert_eq!(compressed_size(r, 4, 4), 8);
        assert_eq!(compressed_size(r, 17, 9), 5 * 3 * 8);
        assert_eq!(compressed_size(rg, 17, 9), 5 * 3 * 16);
        assert_eq!(compressed_size(p, 3, 5), 16);
    }

    #[test]
    fn r11_extremes() {
        let r = decoded_class(0x9270).unwrap();
        let mut out = [0u8; 32];
        assert!(decode_into(r, 4, 4, &[0u8; 8], &mut out));
        let zero = etc2::decode_eac11_unsigned_block(&[0u8; 8]);
        assert!((half_at(&out, 0) - zero[0] as f32 / 65535.0).abs() < 2.4e-4);
        let blk = [255u8, 0, 0, 0, 0, 0, 0, 0];
        assert!(decode_into(r, 4, 4, &blk, &mut out));
        let want = etc2::decode_eac11_unsigned_block(&blk);
        for i in 0..16 {
            assert!((half_at(&out, i) - want[i] as f32 / 65535.0).abs() < 2.4e-4);
        }
        let s = decoded_class(0x9271).unwrap();
        let blk = [0x80u8, 0, 0, 0, 0, 0, 0, 0];
        assert!(decode_into(s, 4, 4, &blk, &mut out));
        let want = etc2::decode_eac11_signed_block(&blk);
        for i in 0..16 {
            let w = (want[i] as f32 / 32767.0).max(-1.0);
            assert!((half_at(&out, i) - w).abs() < 2.4e-4);
        }
        // the extremes through the conversion
        assert_eq!(signed_half(-32767), f32_to_half(-1.0).to_le_bytes());
        assert_eq!(signed_half(-32768), f32_to_half(-1.0).to_le_bytes());
        assert_eq!(unsigned_half(65535), f32_to_half(1.0).to_le_bytes());
        assert_eq!(unsigned_half(0), [0, 0]);
        // 11-bit 2047 (65535), 0, and signed -1023 / -1024 (clamped) stay within 2.4e-4
        for v in [0u16, 2047 << 5 | 31, 65535] {
            assert!((half_to_f32(u16::from_le_bytes(unsigned_half(v))) - v as f32 / 65535.0).abs() < 2.4e-4);
        }
        for v in [-32767i16, -32736, 32767] {
            let w = (v as f32 / 32767.0).max(-1.0);
            assert!((half_to_f32(u16::from_le_bytes(signed_half(v))) - w).abs() < 2.4e-4);
        }
    }

    #[test]
    fn rg11_layout() {
        let rg = decoded_class(0x9272).unwrap();
        let data = pseudo(16, 7);
        let mut out = vec![0u8; 16 * 8];
        assert!(decode_into(rg, 4, 4, &data, &mut out));
        let r = etc2::decode_eac11_unsigned_block(data[0..8].try_into().unwrap());
        let g = etc2::decode_eac11_unsigned_block(data[8..16].try_into().unwrap());
        for i in 0..16 {
            assert_eq!(&out[i * 8..i * 8 + 2], &unsigned_half(r[i]));
            assert_eq!(&out[i * 8 + 2..i * 8 + 4], &unsigned_half(g[i]));
            assert_eq!(half_at(&out[i * 8..], 2), 0.0);
            assert_eq!(half_at(&out[i * 8..], 3), 1.0);
        }
    }

    #[test]
    fn punchthrough_zeroes_rgb() {
        let p = decoded_class(0x9276).unwrap();
        let mut seen_clear = false;
        for seed in 0..64 {
            let data = pseudo(8, seed);
            let mut out = [0u8; 64];
            assert!(decode_into(p, 4, 4, &data, &mut out));
            let want = etc2::decode_rgb_block(data[..8].try_into().unwrap(), true);
            for i in 0..16 {
                assert_eq!(&out[i * 4..i * 4 + 4], &want[i]);
                if out[i * 4 + 3] == 0 {
                    seen_clear = true;
                    assert_eq!(&out[i * 4..i * 4 + 3], &[0, 0, 0]);
                } else {
                    assert_eq!(out[i * 4 + 3], 255);
                }
            }
        }
        assert!(seen_clear);
    }

    #[test]
    fn edge_blocks_equal_cropped_full_decode() {
        for f in ALL {
            let c = decoded_class(f).unwrap();
            let (w, h) = (17, 9);
            let data = pseudo(compressed_size(c, w, h), f);
            let mut small = vec![0u8; w * h * c.texel_bytes];
            assert!(decode_into(c, w, h, &data, &mut small));
            let (fw, fh) = (20, 12);
            let mut full = vec![0u8; fw * fh * c.texel_bytes];
            assert!(decode_into(c, fw, fh, &data, &mut full));
            for y in 0..h {
                for x in 0..w {
                    let a = (y * w + x) * c.texel_bytes;
                    let b = (y * fw + x) * c.texel_bytes;
                    assert_eq!(small[a..a + c.texel_bytes], full[b..b + c.texel_bytes], "{f:#x} {x},{y}");
                }
            }
        }
    }

    #[test]
    fn short_buffers_write_nothing() {
        let c = decoded_class(0x9272).unwrap();
        let data = pseudo(64, 1);
        let mut out = vec![0xAAu8; 8 * 8 * 8];
        assert!(!decode_into(c, 8, 8, &data[..63], &mut out[..]));
        assert!(!decode_into(c, 8, 8, &data, &mut out[..8 * 8 * 8 - 1]));
        assert!(out.iter().all(|b| *b == 0xAA));
    }

    #[test]
    fn sub_rect_equals_window_of_full_decode() {
        for f in ALL {
            let c = decoded_class(f).unwrap();
            let (lw, lh) = (18, 14);
            let tb = c.texel_bytes;
            let full_data = pseudo(compressed_size(c, lw, lh), f + 1);
            let mut full = vec![0u8; lw * lh * tb];
            assert!(decode_into(c, lw, lh, &full_data, &mut full));
            // 8x4 texels at (4, 8): blocks 1..3 of block row 2
            let bw = lw.div_ceil(4);
            let mut sub = Vec::new();
            for bx in 1..3 {
                let o = (2 * bw + bx) * c.block_bytes;
                sub.extend_from_slice(&full_data[o..o + c.block_bytes]);
            }
            let mut dst = vec![0u8; lw * lh * tb];
            assert!(decode_sub_into(c, &sub, 4, 8, 8, 4, &mut dst, lw, lh));
            for y in 8..12 {
                for x in 4..12 {
                    let a = (y * lw + x) * tb;
                    assert_eq!(dst[a..a + tb], full[a..a + tb], "{f:#x} {x},{y}");
                }
            }
            assert!(dst[..8 * lw * tb].iter().all(|b| *b == 0));
            // a 2x6 rect reaching the right and bottom edge drops outside texels
            let mut sub = Vec::new();
            for by in 2..4 {
                let o = (by * bw + 4) * c.block_bytes;
                sub.extend_from_slice(&full_data[o..o + c.block_bytes]);
            }
            let mut dst = vec![0u8; lw * lh * tb];
            assert!(decode_sub_into(c, &sub, 16, 8, 2, 6, &mut dst, lw, lh));
            for y in 8..lh {
                for x in 16..lw {
                    let a = (y * lw + x) * tb;
                    assert_eq!(dst[a..a + tb], full[a..a + tb], "{f:#x} edge {x},{y}");
                }
            }
            // misaligned offset refused
            assert!(!decode_sub_into(c, &sub, 2, 8, 2, 6, &mut dst, lw, lh));
        }
    }
}
