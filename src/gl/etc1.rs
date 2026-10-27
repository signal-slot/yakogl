// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
// Ported from VK-GL-CTS framework/common/tcuCompressedTexture.cpp at
// 93bca01 (Apache-2.0), modified: see below.
// See NOTICE.
//! ETC1 decoding on the CPU, for a backend whose texture unit has no ETC1
//! (`YakoglBackend::etc1_decoded_at_upload`): `glCompressedTexImage2D` turns a
//! `GL_ETC1_RGB8_OES` image into RGB texels here, as Mesa's state tracker does
//! for a driver with no ETC1 format (`st_format.c`, `st->has_etc1`).
//!
//! The block decoder is dEQP's own, so the texels are the ones dEQP's
//! reference is built from: `decompressETC1Block` with `getBit`, `getBits`,
//! `extend4To8`, `extend5To8`, `extendSigned3To8` and `extend5Delta3To8`,
//! transcribed from VK-GL-CTS framework/common/tcuCompressedTexture.cpp at
//! 93bca01 (:709-719, :748-757, :771-793, :818-876; Apache-2.0,
//! "Copyright 2014 The Android Open Source Project").
//!
//! The test vectors are not from dEQP: two blocks derived by hand from the
//! format, and four blocks of dEQP's photo_helsinki_mip_0.pkm decoded by
//! Mesa 26.2.1's own decoder (`etc1_parse_block` / `etc1_fetch_texel`,
//! src/util/format/texcompress_etc_tmp.h), a second implementation.

/// The bytes of one ETC1 block (4x4 texels).
pub(crate) const BLOCK_BYTES: usize = 8;

fn get_bit(src: u64, bit: u32) -> u32 {
    ((src >> bit) & 1) as u32
}

fn get_bits(src: u64, low: u32, high: u32) -> u32 {
    ((src >> low) & ((1u64 << (high - low + 1)) - 1)) as u32
}

fn extend4_to_8(v: u32) -> i32 {
    ((v << 4) | v) as i32
}

fn extend5_to_8(v: u32) -> i32 {
    ((v << 3) | (v >> 2)) as i32
}

/// dEQP's extend5Delta3To8: a 5-bit base plus a signed 3-bit delta. A valid
/// ETC1 block keeps the sum in 0..=31 (the format says so); the `& 31` only
/// keeps an invalid one from reading outside 5 bits, where dEQP asserts.
fn extend5_delta3_to_8(base5: u32, delta3: u32) -> i32 {
    let delta = if delta3 & 4 != 0 { delta3 as i32 - 8 } else { delta3 as i32 };
    extend5_to_8(((base5 as i32 + delta) as u32) & 31)
}

const MODIFIERS: [[i32; 4]; 8] = [
    [2, 8, -2, -8],
    [5, 17, -5, -17],
    [9, 29, -9, -29],
    [13, 42, -13, -42],
    [18, 60, -18, -60],
    [24, 80, -24, -80],
    [33, 106, -33, -106],
    [47, 183, -47, -183],
];

/// Decode one block: 16 texels as R, G, B, A (A = 255), row-major
/// (texel (x, y) at `y * 4 + x`), row 0 being the block's first row in
/// memory, as dEQP's `decompressETC1Block` writes them.
pub(crate) fn decode_block(block: &[u8; BLOCK_BYTES]) -> [[u8; 4]; 16] {
    let src = u64::from_be_bytes(*block); /* get64BitBlock: big-endian */
    let diff = get_bit(src, 33);
    let flip = get_bit(src, 32);
    let table = [get_bits(src, 37, 39) as usize, get_bits(src, 34, 36) as usize];
    let (base_r, base_g, base_b);

    if diff == 0 {
        /* individual mode: two 4-bit colours */
        base_r = [extend4_to_8(get_bits(src, 60, 63)), extend4_to_8(get_bits(src, 56, 59))];
        base_g = [extend4_to_8(get_bits(src, 52, 55)), extend4_to_8(get_bits(src, 48, 51))];
        base_b = [extend4_to_8(get_bits(src, 44, 47)), extend4_to_8(get_bits(src, 40, 43))];
    } else {
        /* differential mode: a 5-bit colour and a 3-bit signed delta */
        let (br, dr) = (get_bits(src, 59, 63), get_bits(src, 56, 58));
        let (bg, dg) = (get_bits(src, 51, 55), get_bits(src, 48, 50));
        let (bb, db) = (get_bits(src, 43, 47), get_bits(src, 40, 42));
        base_r = [extend5_to_8(br), extend5_delta3_to_8(br, dr)];
        base_g = [extend5_to_8(bg), extend5_delta3_to_8(bg, dg)];
        base_b = [extend5_to_8(bb), extend5_delta3_to_8(bb, db)];
    }

    let mut out = [[0u8; 4]; 16];
    for pixel in 0..16u32 {
        /* the index bits run down the columns: pixel = x * 4 + y */
        let (x, y) = (pixel / 4, pixel % 4);
        let sub = usize::from((if flip != 0 { y } else { x }) >= 2);
        let m = MODIFIERS[table[sub]][((get_bit(src, 16 + pixel) << 1) | get_bit(src, pixel)) as usize];
        out[(y * 4 + x) as usize] = [
            (base_r[sub] + m).clamp(0, 255) as u8,
            (base_g[sub] + m).clamp(0, 255) as u8,
            (base_b[sub] + m).clamp(0, 255) as u8,
            255,
        ];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16 texels as R, G, B, row-major, as the vectors below list them.
    fn rgb(block: [u8; 8]) -> [[u8; 3]; 16] {
        let d = decode_block(&block);
        core::array::from_fn(|i| [d[i][0], d[i][1], d[i][2]])
    }

    #[test]
    fn alpha_is_opaque() {
        assert!(decode_block(&[0x3e, 0x31, 0x43, 0x33, 0x77, 0x75, 0x22, 0x88]).iter().all(|t| t[3] == 255));
    }

    #[test]
    fn individual_mode_by_hand() {
        /* R 8/2, G 4/C, B 0/F (x17 each); tables 0 and 7; diff 0, flip 0;
         * every index 01 (MSB word 0, LSB word FFFF): +8 on the left two
         * columns, +183 on the right two, which clamps G and B.
         * Left: (0x88, 0x44, 0x00) + 8 = (144, 76, 8).
         * Right: (0x22, 0xCC, 0xFF) + 183 = (217, 387 -> 255, 438 -> 255). */
        let l = [144, 76, 8];
        let r = [217, 255, 255];
        assert_eq!(
            rgb([0x82, 0x4c, 0x0f, 0x1c, 0x00, 0x00, 0xff, 0xff]),
            [l, l, r, r, l, l, r, r, l, l, r, r, l, l, r, r]
        );
    }

    #[test]
    fn differential_flipped_by_hand() {
        /* R 16 + 3, G 31 - 4, B 0 + 1 (5-bit + 3-bit signed); tables 2 and
         * 5; diff 1, flip 1 (the top two rows are sub-block 0).
         * Base 0 = (132, 255, 0), base 1 = (156, 222, 8).
         * Texel (0, 0): pixel 0, MSB bit 1 and LSB bit 0: index 2, -9 of
         * table 2: (123, 246, 0). Texel (1, 0): pixel 4, index 01, +29:
         * (161, 284 -> 255, 29). The rest are Mesa's decode of the same
         * bytes (texcompress_etc_tmp.h), which agrees with those two. */
        assert_eq!(
            rgb([0x83, 0xfc, 0x01, 0x57, 0xa5, 0xc3, 0x3c, 0x96]),
            [
                [123, 246, 0],
                [161, 255, 29],
                [123, 246, 0],
                [161, 255, 29],
                [103, 226, 0],
                [141, 255, 9],
                [141, 255, 9],
                [103, 226, 0],
                [236, 255, 88],
                [132, 198, 0],
                [76, 142, 0],
                [180, 246, 32],
                [180, 246, 32],
                [76, 142, 0],
                [236, 255, 88],
                [132, 198, 0],
            ]
        );
    }

    /// Blocks 276, 45, 275 and 76 of dEQP's data/etc1/photo_helsinki_mip_0.pkm
    /// (the texture of 37 of the 54 ETC1 cases), one per mode: (diff, flip)
    /// = (0, 0), (0, 1), (1, 0), (1, 1). Expected: Mesa 26.2.1's decoder.
    #[test]
    fn real_blocks_against_mesa() {
        let cases: [([u8; 8], [[u8; 3]; 16]); 4] = [
            (
                [0x24, 0x37, 0x5b, 0x0c, 0x0c, 0x21, 0x7f, 0x96],
                [
                    [32, 49, 83],
                    [42, 59, 93],
                    [110, 161, 229],
                    [110, 161, 229],
                    [42, 59, 93],
                    [32, 49, 83],
                    [110, 161, 229],
                    [110, 161, 229],
                    [42, 59, 93],
                    [36, 53, 87],
                    [26, 77, 145],
                    [110, 161, 229],
                    [36, 53, 87],
                    [42, 59, 93],
                    [26, 77, 145],
                    [81, 132, 200],
                ],
            ),
            (
                [0x45, 0x75, 0xd8, 0xad, 0x08, 0xcc, 0x48, 0x8c],
                [
                    [92, 143, 245],
                    [92, 143, 245],
                    [92, 143, 245],
                    [92, 143, 245],
                    [92, 143, 245],
                    [92, 143, 245],
                    [92, 143, 245],
                    [92, 143, 245],
                    [43, 43, 94],
                    [72, 72, 123],
                    [98, 98, 149],
                    [127, 127, 178],
                    [43, 43, 94],
                    [43, 43, 94],
                    [43, 43, 94],
                    [98, 98, 149],
                ],
            ),
            (
                [0x6d, 0x84, 0xa4, 0x4e, 0xf9, 0xf8, 0xf2, 0x9f],
                [
                    [136, 161, 194],
                    [78, 103, 136],
                    [69, 86, 119],
                    [40, 57, 90],
                    [136, 161, 194],
                    [98, 123, 156],
                    [124, 141, 174],
                    [40, 57, 90],
                    [136, 161, 194],
                    [98, 123, 156],
                    [95, 112, 145],
                    [40, 57, 90],
                    [78, 103, 136],
                    [78, 103, 136],
                    [69, 86, 119],
                    [40, 57, 90],
                ],
            ),
            (
                [0x3e, 0x31, 0x43, 0x33, 0x77, 0x75, 0x22, 0x88],
                [
                    [52, 44, 61],
                    [52, 44, 61],
                    [52, 44, 61],
                    [52, 44, 61],
                    [62, 54, 71],
                    [52, 44, 61],
                    [40, 32, 49],
                    [40, 32, 49],
                    [23, 39, 72],
                    [23, 39, 72],
                    [23, 39, 72],
                    [23, 39, 72],
                    [101, 117, 150],
                    [101, 117, 150],
                    [59, 75, 108],
                    [59, 75, 108],
                ],
            ),
        ];
        for (i, (block, want)) in cases.iter().enumerate() {
            assert_eq!(rgb(*block), *want, "block {i}");
        }
    }
}
