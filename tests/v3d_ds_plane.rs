// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D D24S8 / S8 plane read-modify-write helpers (plan item
//! v3d-ds-plane; docs/notes/v3d-fbo-blit.md 3.4), on their own.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ds_plane` (the test file needs no feature itself, the module is
//! included by path and does not depend on the crate; the features only let
//! the crate build, which cargo does for any test). The back end includes
//! the same file (src/v3d/backend.rs, `v3d_ds_write` / `v3d_stencil_resolve`).
//!
//! The expected values come from a model written apart from the module:
//! `v3d_depth_resolve` reads a word's depth as `(word >> 8) as f32 /
//! 16777215.0`, so a depth written must read back as the nearest 24-bit word.

#[path = "../src/v3d/ds_plane.rs"]
mod ds_plane;

use ds_plane::*;

/// `v3d_depth_resolve`'s conversion of one word.
fn resolve(word: u32) -> f32 {
    (word >> 8) as f32 / 16_777_215.0
}

/// The word nearest to `v`, from the f64 product (independent of the
/// module's formula).
fn nearest(v: f32) -> u32 {
    let t = (v as f64).clamp(0.0, 1.0) * 16_777_215.0;
    let lo = t.floor();
    if t - lo >= 0.5 {
        lo as u32 + 1
    } else {
        lo as u32
    }
}

#[test]
fn depth_round_trips_through_resolve_for_24_bit_values() {
    let (w, h) = (8u32, 4u32);
    for base in [0u32, 1, 0x123456, 0x7f_ffff, 0xff_fff0] {
        let words24: Vec<u32> = (0..w * h).map(|i| (base + i).min(0xff_ffff)).collect();
        let src: Vec<f32> = words24.iter().map(|&v| resolve(v << 8)).collect();
        let mut buf = vec![0u32; (w * h) as usize];
        write_depth_d24s8(&mut buf, w, h, &src, 0, 0, w, h, w).unwrap();
        for (i, &word) in buf.iter().enumerate() {
            assert_eq!(word >> 8, words24[i], "base {base:#x} px {i}");
            assert_eq!(resolve(word), src[i]);
        }
    }
}

#[test]
fn depth_of_many_24_bit_words_is_exact() {
    let mut buf = [0u32; 1];
    for v in (0..=0xff_ffffu32).step_by(257).chain([0xff_ffff]) {
        write_depth_d24s8(&mut buf, 1, 1, &[resolve(v << 8)], 0, 0, 1, 1, 1).unwrap();
        assert_eq!(buf[0] >> 8, v);
    }
}

#[test]
fn d32f_values_round_to_the_nearest_word_and_clamp() {
    let vals = [0.0f32, 1.0, 0.5, 0.25, 1.0 / 3.0, 0.999_999_9, 1e-8, 0.123_456_79, -1.0, 2.0, f32::NAN];
    let n = vals.len() as u32;
    let mut buf = vec![0u32; vals.len()];
    write_depth_d24s8(&mut buf, n, 1, &vals, 0, 0, n, 1, n).unwrap();
    for (i, &v) in vals.iter().enumerate() {
        let want = if v.is_nan() { 0 } else { nearest(v) };
        assert_eq!(buf[i] >> 8, want, "value {v}");
    }
    assert_eq!(buf[0] >> 8, 0);
    assert_eq!(buf[1] >> 8, 0xff_ffff);
}

#[test]
fn depth_write_keeps_the_stencil_byte() {
    let mut buf: Vec<u32> = (0..16).map(|i| 0x00ab_cd00 | (i * 13 + 1)).collect();
    let stencil: Vec<u32> = buf.iter().map(|w| w & 0xff).collect();
    write_depth_d24s8(&mut buf, 4, 4, &[0.5; 16], 0, 0, 4, 4, 4).unwrap();
    for (i, w) in buf.iter().enumerate() {
        assert_eq!(w & 0xff, stencil[i]);
        assert_eq!(w >> 8, nearest(0.5));
    }
}

#[test]
fn stencil_write_keeps_the_depth_bits() {
    let mut buf: Vec<u32> = (0..16).map(|i| (0x10_0000 + i * 977) << 8 | 0x5a).collect();
    let depth: Vec<u32> = buf.iter().map(|w| w >> 8).collect();
    let src: Vec<u8> = (0..16).map(|i| 200 + i as u8).collect();
    write_stencil_d24s8(&mut buf, 4, 4, &src, 0, 0, 4, 4, 4).unwrap();
    for (i, w) in buf.iter().enumerate() {
        assert_eq!(w >> 8, depth[i]);
        assert_eq!((w & 0xff) as u8, src[i]);
    }
}

#[test]
fn s8_write_and_read_round_trip() {
    let (bw, bh) = (5u32, 3u32);
    let mut buf = vec![0u8; 15];
    let src: Vec<u8> = (1..=15).collect();
    write_stencil_s8(&mut buf, bw, bh, &src, 0, 0, bw, bh, bw).unwrap();
    assert_eq!(buf, src);
    // a sub-rectangle with a wider source pitch
    let mut buf = vec![9u8; 15];
    let src = [1u8, 2, 99, 99, 3, 4, 99, 99];
    write_stencil_s8(&mut buf, bw, bh, &src, 2, 1, 2, 2, 4).unwrap();
    assert_eq!(buf, [9, 9, 9, 9, 9, 9, 9, 1, 2, 9, 9, 9, 3, 4, 9]);
    let mut dst = vec![0xeeu8; 4 * 2];
    read_stencil_s8(&buf, bw, bh, &mut dst, 3, 2, 4).unwrap();
    assert_eq!(dst, [9, 9, 9, 0xee, 9, 9, 1, 0xee]);
}

#[test]
fn d24s8_stencil_read_takes_the_low_byte_with_a_pitch() {
    let words: Vec<u32> = (0..6).map(|i| 0xffff_ff00 | (i + 10)).collect();
    let mut dst = vec![0u8; 3 * 2 + 2];
    read_stencil_d24s8(&words, 3, 2, &mut dst, 3, 2, 4).unwrap();
    assert_eq!(&dst[..3], [10, 11, 12]);
    assert_eq!(dst[3], 0);
    assert_eq!(&dst[4..7], [13, 14, 15]);
    // a read larger than the buffer is clipped like v3d_depth_resolve
    let mut dst = vec![0xeeu8; 4 * 3];
    read_stencil_d24s8(&words, 3, 2, &mut dst, 4, 3, 4).unwrap();
    assert_eq!(&dst[..4], [10, 11, 12, 0xee]);
    assert_eq!(&dst[8..], [0xee; 4]);
}

#[test]
fn rectangles_at_the_edges() {
    let (bw, bh) = (4u32, 3u32);
    let mut buf = vec![0u32; 12];
    // bottom-right corner pixel
    write_stencil_d24s8(&mut buf, bw, bh, &[7], 3, 2, 1, 1, 1).unwrap();
    assert_eq!(buf[11], 7);
    assert_eq!(buf.iter().filter(|&&w| w != 0).count(), 1);
    // top row, full width
    write_depth_d24s8(&mut buf, bw, bh, &[1.0; 4], 0, 0, 4, 1, 4).unwrap();
    assert!(buf[..4].iter().all(|&w| w >> 8 == 0xff_ffff));
    assert!(buf[4..11].iter().all(|&w| w == 0));
    // the whole buffer
    write_depth_d24s8(&mut buf, bw, bh, &[0.0; 12], 0, 0, bw, bh, bw).unwrap();
    assert_eq!(buf[11], 7);
}

#[test]
fn out_of_bounds_rectangles_are_refused_and_write_nothing() {
    let mut buf = vec![0x1234_5678u32; 12];
    let copy = buf.clone();
    let f = [0.5f32; 64];
    let s = [1u8; 64];
    let e = Err(PlaneError::OutOfBounds);
    assert_eq!(write_depth_d24s8(&mut buf, 4, 3, &f, 1, 0, 4, 1, 4), e);
    assert_eq!(write_depth_d24s8(&mut buf, 4, 3, &f, 0, 2, 4, 2, 4), e);
    assert_eq!(write_stencil_d24s8(&mut buf, 4, 3, &s, 4, 0, 1, 1, 1), e);
    assert_eq!(write_stencil_d24s8(&mut buf, 4, 3, &s, u32::MAX, 0, 2, 1, 2), e);
    assert_eq!(write_stencil_d24s8(&mut buf, 4, 3, &s, 0, 0, 4, 1, 3), e); // w > pitch
    let mut bytes = vec![0u8; 12];
    assert_eq!(write_stencil_s8(&mut bytes, 4, 3, &s, 0, 3, 1, 1, 1), e);
    assert_eq!(buf, copy);
    assert_eq!(bytes, [0u8; 12]);
    // too-short slices are refused apart from the bounds
    assert_eq!(write_stencil_s8(&mut bytes, 4, 3, &s[..3], 0, 0, 2, 2, 2), Err(PlaneError::ShortSlice));
    assert_eq!(write_depth_d24s8(&mut buf[..5], 4, 3, &f, 0, 0, 1, 1, 1), Err(PlaneError::ShortSlice));
}

#[test]
fn the_empty_rectangle_is_accepted_and_writes_nothing() {
    let mut buf = vec![0xdead_beefu32; 12];
    let copy = buf.clone();
    assert_eq!(write_depth_d24s8(&mut buf, 4, 3, &[], 0, 0, 0, 0, 0), Ok(()));
    assert_eq!(write_stencil_d24s8(&mut buf, 4, 3, &[], 4, 3, 0, 0, 0), Ok(()));
    assert_eq!(write_stencil_d24s8(&mut buf, 4, 3, &[], 1, 1, 3, 0, 3), Ok(()));
    assert_eq!(write_stencil_d24s8(&mut buf, 4, 3, &[], 2, 1, 0, 2, 0), Ok(()));
    let mut bytes = vec![5u8; 12];
    assert_eq!(write_stencil_s8(&mut bytes, 4, 3, &[], 1, 1, 0, 0, 0), Ok(()));
    assert_eq!(read_stencil_s8(&bytes, 4, 3, &mut [], 0, 0, 0), Ok(()));
    assert_eq!(read_stencil_d24s8(&buf, 4, 3, &mut [], 0, 0, 0), Ok(()));
    assert_eq!(buf, copy);
    assert_eq!(bytes, [5u8; 12]);
    // an empty rectangle past the buffer is still out of bounds
    assert_eq!(write_stencil_s8(&mut bytes, 4, 3, &[], 5, 0, 0, 0, 0), Err(PlaneError::OutOfBounds));
}
