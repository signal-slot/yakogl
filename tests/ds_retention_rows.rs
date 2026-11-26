// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The slice-based D24S8 row helpers of `ds_retention` (`fill_depth_rows`,
//! `fill_stencil_rows`, `d24s8_depth_rows`, `d24s8_stencil_rows`) against
//! the per-texel rule: `write_depth_texels` driven by the closure the
//! PowerVR resolve uses today (`unorm24_depth(words[y*sy*stride + x*sx])`
//! inside `x < sw && y < sh`), bit for bit, padding untouched.
//!
//! Features: `--no-default-features --features
//! host-test,gpu-powervr,shaderc-online`.
#![cfg(feature = "host-test")]

use yakogl::ds_retention::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn words(n: usize, seed: u64) -> Vec<u32> {
    let mut r = Rng(seed);
    let mut v: Vec<u32> = (0..n).map(|_| r.next()).collect();
    let edge = [0, 0xff_ffff, 0xff00_0000, 0xffff_ffff, 0xff00_0001, 0x0080_0000, 1];
    for (i, e) in edge.iter().enumerate() {
        if i < n {
            v[i] = *e;
        }
    }
    v
}

const SENT: u32 = 0xdead_beef;

#[allow(clippy::too_many_arguments)]
fn check(sw: u32, sh: u32, w: u32, h: u32, pitch: u32, sx: usize, sy: usize, pad_rows: usize) {
    let stride = (sw as usize * sx).div_ceil(4) * 4 + 4 * (pad_rows % 2);
    let n = (sh.max(1) as usize * sy) * stride + 8;
    let src = words(n, 0x9e37_79b9_7f4a_7c15 ^ (sw as u64 * 131 + sh as u64));
    let len = pitch as usize * h as usize;
    let ctx = format!("src {sw}x{sh} dst {w}x{h} pitch {pitch} step {sx},{sy}");

    // depth
    let mut want = vec![f32::from_bits(SENT); len];
    let mut got = want.clone();
    unsafe {
        write_depth_texels(want.as_mut_ptr(), w, h, pitch, |x, y| {
            (x < sw && y < sh).then(|| unorm24_depth(src[y as usize * sy * stride + x as usize * sx]))
        })
    };
    d24s8_depth_rows(&src, stride, sx, sy, sw, sh, &mut got, w, h, pitch);
    let (a, b): (Vec<u32>, Vec<u32>) =
        (want.iter().map(|f| f.to_bits()).collect(), got.iter().map(|f| f.to_bits()).collect());
    assert_eq!(a, b, "depth {ctx}");

    // stencil: the same rule on bytes
    let mut want = vec![0xa5u8; len];
    let mut got = want.clone();
    for y in 0..h.min(sh) {
        for x in 0..w.min(sw) {
            want[y as usize * pitch as usize + x as usize] =
                (src[y as usize * sy * stride + x as usize * sx] >> 24) as u8;
        }
    }
    d24s8_stencil_rows(&src, stride, sx, sy, sw, sh, &mut got, w, h, pitch);
    assert_eq!(want, got, "stencil {ctx}");
}

#[test]
fn steps() {
    for (sx, sy) in [(1, 1), (1, 2), (2, 2)] {
        for pad in 0..2 {
            // aligned, padded pitch, 1x1, wider / taller / both than source
            check(16, 8, 16, 8, 16, sx, sy, pad);
            check(16, 8, 13, 7, 20, sx, sy, pad);
            check(1, 1, 1, 1, 1, sx, sy, pad);
            check(1, 1, 1, 1, 5, sx, sy, pad);
            check(8, 8, 12, 8, 16, sx, sy, pad);
            check(8, 8, 8, 11, 8, sx, sy, pad);
            check(8, 8, 19, 13, 24, sx, sy, pad);
            check(33, 17, 30, 15, 33, sx, sy, pad);
        }
    }
}

#[test]
fn empty_requests_write_nothing() {
    let src = words(64, 7);
    let mut d = vec![1.5f32; 16];
    d24s8_depth_rows(&src, 8, 1, 1, 8, 8, &mut d, 0, 4, 4);
    d24s8_depth_rows(&src, 8, 1, 1, 8, 8, &mut d, 4, 0, 4);
    d24s8_depth_rows(&src, 8, 1, 1, 0, 8, &mut d, 4, 4, 4);
    assert!(d.iter().all(|&f| f == 1.5));
    let mut s = vec![9u8; 16];
    d24s8_stencil_rows(&src, 8, 1, 1, 8, 0, &mut s, 4, 4, 4);
    fill_stencil_rows(&mut s, 0, 4, 4, 1);
    assert!(s.iter().all(|&b| b == 9));
}

#[test]
fn fills() {
    for (w, h, pitch) in [(4u32, 3u32, 4u32), (3, 3, 7), (1, 1, 1), (5, 2, 9)] {
        let len = (pitch * h) as usize;
        for v in [0.0f32, 1.0, 0.123_456_79, -0.0] {
            let mut want = vec![f32::from_bits(SENT); len];
            let mut got = want.clone();
            unsafe { write_depth_texels(want.as_mut_ptr(), w, h, pitch, |_, _| Some(v)) };
            fill_depth_rows(&mut got, w, h, pitch, v);
            assert_eq!(
                want.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
                got.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
            );
        }
        for v in [0u8, 0xff, 0x5a] {
            let mut want = vec![0xa5u8; len];
            let mut got = want.clone();
            for y in 0..h {
                for x in 0..w {
                    want[(y * pitch + x) as usize] = v;
                }
            }
            fill_stencil_rows(&mut got, w, h, pitch, v);
            assert_eq!(want, got);
        }
    }
}
