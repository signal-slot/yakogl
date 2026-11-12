// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ETC2/EAC decoder against Mesa 26.2.1's own: `etc2_vectors.rs`
//! (tools/etc2_vectors.py) holds blocks and the images
//! `_mesa_unpack_etc2_format` makes of them.

use super::*;
use crate::gl::etc2_vectors as v;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
}

fn flat(texels: &[[u8; 4]; 16]) -> Vec<u8> {
    texels.iter().flatten().copied().collect()
}

#[test]
fn rgb8_matches_mesa() {
    for (i, (blk, want)) in v::RGB8.iter().enumerate() {
        let b: [u8; 8] = unhex(blk).try_into().unwrap();
        assert_eq!(flat(&rgb_block(&b, false)), unhex(want), "block {i} {blk}");
    }
}

#[test]
fn punchthrough_matches_mesa() {
    let mut transparent = 0;
    for (i, (blk, want)) in v::RGB8_PT.iter().enumerate() {
        let b: [u8; 8] = unhex(blk).try_into().unwrap();
        let got = rgb_block(&b, true);
        assert_eq!(flat(&got), unhex(want), "block {i} {blk}");
        transparent += got.iter().filter(|t| t[3] == 0).count();
        for t in got.iter().filter(|t| t[3] == 0) {
            assert_eq!(*t, [0; 4], "a transparent texel is (0, 0, 0, 0)");
        }
    }
    assert!(transparent > 0, "the vectors hold transparent texels");
}

#[test]
fn rgba8_eac_matches_mesa() {
    for (i, (blk, want)) in v::RGBA8_EAC.iter().enumerate() {
        let b = unhex(blk);
        let alpha = eac_alpha(b[..8].try_into().unwrap());
        let mut rgba = rgb_block(b[8..].try_into().unwrap(), false);
        for (t, a) in rgba.iter_mut().zip(alpha) {
            t[3] = a;
        }
        assert_eq!(flat(&rgba), unhex(want), "block {i} {blk}");
    }
}

/// Mesa widens the 11-bit codeword to 16 bits by bit replication; the
/// codeword is what the decoder returns.
fn widen_unsigned(c: i16) -> u16 {
    let c = c as u16;
    (c << 5) | (c >> 6)
}

fn widen_signed(c: i16) -> i16 {
    let m = c.unsigned_abs();
    let w = ((m << 5) | (m >> 5)) as i16;
    if c < 0 {
        -w
    } else {
        w
    }
}

fn words(bytes: &[u8]) -> Vec<u16> {
    bytes.chunks(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect()
}

#[test]
fn r11_matches_mesa() {
    for (signed, vectors) in [(false, v::R11), (true, v::SIGNED_R11)] {
        for (i, (blk, want)) in vectors.iter().enumerate() {
            let b: [u8; 8] = unhex(blk).try_into().unwrap();
            let got: Vec<u16> = eac11(&b, signed)
                .iter()
                .map(|&c| if signed { widen_signed(c) as u16 } else { widen_unsigned(c) })
                .collect();
            assert_eq!(got, words(&unhex(want)), "signed {signed} block {i} {blk}");
        }
    }
}

#[test]
fn rg11_matches_mesa() {
    for (signed, vectors) in [(false, v::RG11), (true, v::SIGNED_RG11)] {
        for (i, (blk, want)) in vectors.iter().enumerate() {
            let b = unhex(blk);
            let (r, g) = (eac11(b[..8].try_into().unwrap(), signed), eac11(b[8..].try_into().unwrap(), signed));
            let got: Vec<u16> = (0..16)
                .flat_map(|t| [r[t], g[t]])
                .map(|c| if signed { widen_signed(c) as u16 } else { widen_unsigned(c) })
                .collect();
            assert_eq!(got, words(&unhex(want)), "signed {signed} block {i} {blk}");
        }
    }
}

/// The vectors reach every colour mode, so a decoder that matched Mesa on
/// the individual and differential blocks alone would not pass above; this
/// pins that the stratification is still in the file.
#[test]
fn the_vectors_cover_every_mode() {
    let mut seen = [false; 3];
    for (blk, _) in v::RGB8_PT {
        let b = unhex(blk);
        let (r, g, bl) = (diff_sum(b[0]), diff_sum(b[1]), diff_sum(b[2]));
        seen[0] |= !(0..=31).contains(&r);
        seen[1] |= (0..=31).contains(&r) && !(0..=31).contains(&g);
        seen[2] |= (0..=31).contains(&r) && (0..=31).contains(&g) && !(0..=31).contains(&bl);
    }
    assert_eq!(seen, [true; 3], "T, H and planar blocks");
}

/// The values GL defines (ES 3.0.6 3.8.5: the 11-bit codeword over 2047, the
/// signed one over 1023, in [-1, 1]).
#[test]
fn eac_codewords_map_to_the_gl_range() {
    assert_eq!(unorm11_to_float(0), 0.0);
    assert_eq!(unorm11_to_float(2047), 1.0);
    assert_eq!(snorm11_to_float(-1023), -1.0);
    assert_eq!(snorm11_to_float(1023), 1.0);
    assert_eq!(snorm11_to_float(0), 0.0);
    /* an all-zero block: base 4 (unsigned), 0 (signed), table 0, index 0 */
    assert_eq!(eac11(&[0; 8], false)[0], 4 - 3);
    assert_eq!(eac11(&[0; 8], true)[0], -3);
    /* a base of 255 with modifier +14 (index 7, table 0, multiplier 15) clamps */
    assert_eq!(eac11(&[255, 0xf0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], false)[0], 2047);
    /* the signed -128 base is -127 */
    assert_eq!(eac11(&[0x80, 0, 0, 0, 0, 0, 0, 0], true)[0], -127 * 8 - 3);
}
