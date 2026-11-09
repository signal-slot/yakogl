// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Texture swizzle and wrap R in the PowerVR texture state (docs/es3-cts-failures.md
//! RC18): `GlesTextureBind::swizzle` is composed with the format's own swizzle
//! into TEXSTATE_IMAGE_WORD0 swiz0..3 (Mesa image.c:166-191), ZERO and ONE
//! staying constants, and `wrap_r` is the native 3D texture's third address
//! mode.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_swizzle
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::backend::*;

const HEAP: u64 = 0x80_0000_0000;
/// texstate.xml SWIZ: SRCCHAN_0..3 are 0..3, SRC_ONE 4, SRC_ZERO 5.
const ONE: u64 = 4;
const ZERO: u64 = 5;

fn bind(format: u8) -> GlesTextureBind {
    GlesTextureBind {
        format,
        type_: GLES_TEXTYPE_2D,
        base_gpu: HEAP + 0x1000,
        width: 4,
        height: 4,
        stride_texels: 4,
        bytes: 4096,
        ..GlesTextureBind::ZERO
    }
}

fn words(t: &GlesTextureBind) -> Vec<u32> {
    yakogl::pvr::backend::texture_descriptor(t).to_vec()
}

/// IMAGE_WORD0 swiz0..3 (bits 14, 11, 8, 5).
fn swiz(t: &GlesTextureBind) -> [u64; 4] {
    let w = words(t);
    let w0 = w[0] as u64 | (w[1] as u64) << 32;
    [(w0 >> 14) & 7, (w0 >> 11) & 7, (w0 >> 8) & 7, (w0 >> 5) & 7]
}

/// IMAGE_WORD0 without the swizzle bits.
fn rest(t: &GlesTextureBind) -> u64 {
    let w = words(t);
    (w[0] as u64 | (w[1] as u64) << 32) & !0x1ffe0
}

fn with(t: &GlesTextureBind, s: [GlesSwizzle; 4]) -> GlesTextureBind {
    GlesTextureBind { swizzle: s, ..*t }
}

const FORMATS: [u8; 5] = [GLES_TEXFMT_RGBA8, GLES_TEXFMT_R8UI, GLES_TEXFMT_L8, GLES_TEXFMT_LA8, GLES_TEXFMT_R32F];

#[test]
fn an_unset_swizzle_keeps_the_formats_own() {
    let own = |f| swiz(&bind(f));
    assert_eq!(own(GLES_TEXFMT_RGBA8), [0, 1, 2, 3]);
    assert_eq!(own(GLES_TEXFMT_L8), [0, 0, 0, ONE]);
    assert_eq!(own(GLES_TEXFMT_LA8), [0, 0, 0, 1]);
    assert_eq!(own(GLES_TEXFMT_R32F), [0, ZERO, ZERO, ONE]);
    for f in FORMATS {
        let t = bind(f);
        assert_eq!(swiz(&with(&t, [GLES_SWIZZLE_SAME; 4])), swiz(&t), "format {f}");
    }
}

#[test]
fn each_source_reads_what_the_format_puts_in_that_channel() {
    for f in FORMATS {
        let t = bind(f);
        let own = swiz(&t);
        for (src, name) in [
            (GLES_SWIZZLE_RED, "red"),
            (GLES_SWIZZLE_GREEN, "green"),
            (GLES_SWIZZLE_BLUE, "blue"),
            (GLES_SWIZZLE_ALPHA, "alpha"),
        ] {
            let got = swiz(&with(&t, [src; 4]));
            let c = (src - 1) as usize;
            assert_eq!(got, [own[c]; 4], "format {f}, every component {name}");
        }
        assert_eq!(swiz(&with(&t, [GLES_SWIZZLE_ZERO; 4])), [ZERO; 4], "format {f} zero");
        assert_eq!(swiz(&with(&t, [GLES_SWIZZLE_ONE; 4])), [ONE; 4], "format {f} one");
    }
}

#[test]
fn components_are_composed_independently_and_nothing_else_moves() {
    let t = bind(GLES_TEXFMT_RGBA8);
    let s = with(&t, [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_SAME, GLES_SWIZZLE_ZERO, GLES_SWIZZLE_RED]);
    assert_eq!(swiz(&s), [3, 1, ZERO, 0]);
    assert_eq!(words(&s)[2..], words(&t)[2..], "image word 1, meta and sampler unchanged");
    for f in FORMATS {
        let t = bind(f);
        let s = with(&t, [GLES_SWIZZLE_BLUE, GLES_SWIZZLE_ONE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_SAME]);
        assert_eq!(rest(&s), rest(&t), "format {f}: only the swizzle bits");
    }
    /* L8's green is the format's second component, L; its alpha is the constant 1 */
    let l =
        with(&bind(GLES_TEXFMT_L8), [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_ZERO, GLES_SWIZZLE_ALPHA]);
    assert_eq!(swiz(&l), [ONE, 0, ZERO, ONE]);
    /* LA8 (L,L,L,A): a GL red reads L, a GL alpha reads A */
    let la = with(&bind(GLES_TEXFMT_LA8), [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_RED, GLES_SWIZZLE_SAME, GLES_SWIZZLE_SAME]);
    assert_eq!(swiz(&la), [1, 0, 0, 1]);
}

#[test]
fn a_depth_texture_composes_over_its_luminance_swizzle() {
    let t = GlesTextureBind { depth_luminance: 1, ..bind(GLES_TEXFMT_R32F) };
    assert_eq!(swiz(&t), [0, 0, 0, ONE]);
    let s = with(&t, [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_ZERO, GLES_SWIZZLE_RED, GLES_SWIZZLE_ONE]);
    assert_eq!(swiz(&s), [ONE, ZERO, 0, ONE]);
}

#[test]
fn the_swizzle_reaches_the_native_textypes_too() {
    for ty in [GLES_TEXTYPE_3D_TWIDDLED, GLES_TEXTYPE_2D_ARRAY_TWIDDLED, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE] {
        let t = GlesTextureBind { type_: ty, layers: 6, ..bind(GLES_TEXFMT_RGBA8) };
        let s = with(&t, [GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED, GLES_SWIZZLE_ONE]);
        assert_eq!(swiz(&s), [2, 1, 0, ONE], "type {ty}");
    }
}

fn addrmode_w(t: &GlesTextureBind) -> u32 {
    (words(t)[9] >> 24) & 7
}

#[test]
fn wrap_r_is_the_native_3d_textures_third_address_mode() {
    let t = GlesTextureBind { type_: GLES_TEXTYPE_3D_TWIDDLED, layers: 4, ..bind(GLES_TEXFMT_RGBA8) };
    for f in [GLES_TEXFMT_RGBA8, GLES_TEXFMT_R8UI, GLES_TEXFMT_LA8, GLES_TEXFMT_R32F] {
        for (wrap, mode) in [(GLES_WRAP_REPEAT, 0), (GLES_WRAP_MIRRORED_REPEAT, 1), (GLES_WRAP_CLAMP_TO_EDGE, 2)] {
            let d = GlesTextureBind { format: f, wrap_r: wrap, ..t };
            assert_eq!(addrmode_w(&d), mode, "format {f}, wrap {wrap}");
        }
    }
    /* the stacked and array targets keep CLAMP_TO_EDGE on the third axis */
    for ty in [GLES_TEXTYPE_2D, GLES_TEXTYPE_2D_ARRAY_TWIDDLED] {
        let d = GlesTextureBind { type_: ty, wrap_r: GLES_WRAP_REPEAT, ..bind(GLES_TEXFMT_RGBA8) };
        assert_eq!(addrmode_w(&d), 2, "type {ty}");
    }
}
