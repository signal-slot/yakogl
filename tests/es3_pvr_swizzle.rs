// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Texture swizzle on the PowerVR texture state (docs/es3-cts-failures.md
//! RC18): the bind's GL_TEXTURE_SWIZZLE_* is composed with the format's own
//! swizzle into TEXSTATE_IMAGE_WORD0 swiz0..3, ZERO and ONE included, as
//! Mesa composes a view's swizzle with the format's. Wrap R reaches the
//! sampler's third address mode for the native 3D texture only.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_swizzle
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::backend::*;
use yakogl::pvr::backend::texture_descriptor;

/// TEXSTATE_SWIZ values: SRCCHAN_0..3 are 0..3, SRC_ONE 4, SRC_ZERO 5.
const ONE: u64 = 4;
const ZERO: u64 = 5;

fn bind(format: u8, swizzle: [u8; 4]) -> GlesTextureBind {
    GlesTextureBind {
        format,
        type_: GLES_TEXTYPE_2D,
        base_gpu: 0x1_0000_0000,
        width: 4,
        height: 4,
        stride_texels: 4,
        bytes: 64,
        swizzle,
        ..GlesTextureBind::ZERO
    }
}

fn swiz(t: &GlesTextureBind) -> [u64; 4] {
    let w = texture_descriptor(t);
    let q = w[0] as u64 | (w[1] as u64) << 32;
    [(q >> 14) & 7, (q >> 11) & 7, (q >> 8) & 7, (q >> 5) & 7]
}

#[test]
fn the_default_swizzle_is_the_formats_own() {
    assert_eq!(swiz(&bind(GLES_TEXFMT_RGBA8, [GLES_SWIZZLE_SAME; 4])), [0, 1, 2, 3]);
}

#[test]
fn a_channel_permutation_reads_the_named_channels() {
    let s = [GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED, GLES_SWIZZLE_ALPHA];
    assert_eq!(swiz(&bind(GLES_TEXFMT_RGBA8, s)), [2, 1, 0, 3]);
}

#[test]
fn zero_and_one_are_constants() {
    let s = [GLES_SWIZZLE_ZERO, GLES_SWIZZLE_ONE, GLES_SWIZZLE_SAME, GLES_SWIZZLE_ONE];
    assert_eq!(swiz(&bind(GLES_TEXFMT_RGBA8, s)), [ZERO, ONE, 2, ONE]);
}

#[test]
fn the_bind_composes_with_the_formats_swizzle() {
    // A8 reads (0, 0, 0, A): GL's red is the format's first channel, which
    // the format's own swizzle makes the constant zero.
    let id = swiz(&bind(GLES_TEXFMT_A8, [GLES_SWIZZLE_SAME; 4]));
    assert_eq!(id[3], swiz(&bind(GLES_TEXFMT_A8, [GLES_SWIZZLE_ALPHA; 4]))[0]);
    let s = [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_SAME, GLES_SWIZZLE_SAME, GLES_SWIZZLE_RED];
    let got = swiz(&bind(GLES_TEXFMT_A8, s));
    assert_eq!(got, [id[3], id[1], id[2], id[0]]);
    // L8 reads (L, L, L, 1): swizzling alpha into red gives the constant one.
    let l = swiz(&bind(GLES_TEXFMT_L8, [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_SAME, GLES_SWIZZLE_SAME, GLES_SWIZZLE_SAME]));
    assert_eq!(l[0], ONE);
}

#[test]
fn wrap_r_reaches_only_the_native_3d_sampler() {
    let w_mode = |t: &GlesTextureBind| {
        let w = texture_descriptor(t);
        ((w[8] as u64 | (w[9] as u64) << 32) >> 56) & 7
    };
    let flat = bind(GLES_TEXFMT_RGBA8, [GLES_SWIZZLE_SAME; 4]);
    let a = w_mode(&GlesTextureBind { wrap_r: GLES_WRAP_REPEAT, ..flat });
    let b = w_mode(&GlesTextureBind { wrap_r: GLES_WRAP_CLAMP_TO_EDGE, ..flat });
    assert_eq!(a, b, "a 2D texture never reads the third axis");
    let vol = GlesTextureBind { type_: GLES_TEXTYPE_3D_TWIDDLED, layers: 4, ..flat };
    let a = w_mode(&GlesTextureBind { wrap_r: GLES_WRAP_REPEAT, ..vol });
    let b = w_mode(&GlesTextureBind { wrap_r: GLES_WRAP_CLAMP_TO_EDGE, ..vol });
    assert_ne!(a, b, "a native 3D texture takes WRAP_R");
}
