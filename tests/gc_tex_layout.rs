// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the texture descriptor words of `src/vivante/tex_layout.rs`.
//!
//! The linear half is checked against what `texture.rs` emits today: CONFIG0
//! and CONFIG1 are `texture::format_words`'s own output; LINEAR_STRIDE, SLICE
//! and the level addresses are `build_descriptor`'s formulas (texture.rs:376-447,
//! which needs a frame arena and is private) written out in `texture_rs_linear`.
//! The super-tiled offsets are worked out by hand from Mesa 26.2.1 etnaviv
//! (`etnaviv_resource.c:327-332, 351-352, 389-393`): a level pads to 64x64
//! texels, 4 bytes a texel, and starts on 64 bytes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_tex_layout
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

#[path = "../src/vivante/tex_layout.rs"]
#[allow(dead_code)]
mod tex_layout;

use tex_layout::*;
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_RGBA8};
use yakogl::vivante::regs;
use yakogl::vivante::texture::format_words;

fn input(w: u32, h: u32, levels: u32, layout: Layout, stride_texels: u32) -> Input {
    Input { width: w, height: h, levels, bpp: 4, layout, ts: Ts::None, rs_align: false, stride_texels, base: 0 }
}

fn bind(w: u32, h: u32, stride: u32, levels: u8) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        width: w,
        height: h,
        stride_texels: stride,
        mip_levels: levels,
        layers: 1,
        ..GlesTextureBind::ZERO
    }
}

/// `build_descriptor`'s linear words for a one-layer RGBA8 chain: LINEAR_STRIDE
/// `stride_texels * bpp` (:384), SLICE that times the plain height (:401), each
/// level at 64 bytes after the last, `stride * level height` long (:422-447).
fn texture_rs_linear(h: u32, stride_texels: u32, levels: u32) -> (u32, u32, Vec<u64>) {
    let stride = stride_texels * 4;
    let (mut lh, mut at, mut offs) = (h, 0u64, vec![]);
    for _ in 0..levels {
        at = (at + 63) & !63;
        offs.push(at);
        at += (stride * lh) as u64;
        lh = (lh / 2).max(1);
    }
    (stride, stride * h, offs)
}

fn assert_linear_equals_texture_rs(w: u32, h: u32, stride: u32, levels: u32) {
    let t = bind(w, h, stride, levels as u8);
    let [c0, c1, _] = format_words(&t, false);
    let got = tex_words(&input(w, h, levels, Layout::Linear, stride)).unwrap();
    // CONFIG0's addressing field and CONFIG1's HALIGN are the whole of what
    // the layout adds to the descriptor's format words.
    assert_eq!(got.config0_addressing, c0 & (ADDRESSING_MASK << ADDRESSING_SHIFT), "{w}x{h} addressing");
    assert_eq!(got.config1_halign, c1 & (HALIGN_MASK << HALIGN_SHIFT), "{w}x{h} halign");
    let (stride_b, slice, offs) = texture_rs_linear(h, stride, levels);
    assert_eq!(got.linear_stride, stride_b, "{w}x{h} LINEAR_STRIDE");
    assert_eq!(got.slice, slice, "{w}x{h} SLICE");
    assert_eq!(&got.level_offset[..levels as usize], &offs[..], "{w}x{h} level offsets");
    assert!(got.level_stride[..levels as usize].iter().all(|&s| s == stride_b));
}

#[test]
fn constants_match_regs_rs() {
    assert_eq!(regs::texdesc_addressing_mode(ADDRESSING_MASK), ADDRESSING_MASK << ADDRESSING_SHIFT);
    assert_eq!(regs::texdesc_addressing_mode(1), 1 << ADDRESSING_SHIFT);
    assert_eq!(ADDRESSING_TILED, regs::GC_TEX_ADDRESSING_TILED);
    assert_eq!(ADDRESSING_LINEAR, regs::GC_TEX_ADDRESSING_LINEAR);
    assert_eq!(regs::texdesc_halign(HALIGN_MASK), HALIGN_MASK << HALIGN_SHIFT);
    assert_eq!(regs::texdesc_halign(1), 1 << HALIGN_SHIFT);
    assert_eq!(HALIGN_FOUR, regs::GC_TEXTURE_HALIGN_FOUR);
    assert_eq!(HALIGN_SIXTEEN, regs::GC_TEXTURE_HALIGN_SIXTEEN);
    assert_eq!(LOD_ALIGN, regs::GC_TEXDESC_LOD_ALIGN as u64);
    assert_eq!(MAX_LEVELS as u32 * 4, regs::texdesc_lod_addr(MAX_LEVELS as u32));
}

#[test]
fn linear_matches_texture_rs_rgba8_256_chain() {
    assert_linear_equals_texture_rs(256, 256, 256, 9);
    let got = tex_words(&input(256, 256, 9, Layout::Linear, 256)).unwrap();
    assert_eq!(got.linear_stride, 1024);
    assert_eq!(got.slice, 262144);
    assert_eq!(got.level_offset[1], 262144);
    assert_eq!(got.config1_halign, regs::texdesc_halign(regs::GC_TEXTURE_HALIGN_FOUR));
    assert_eq!(got.config0_addressing, regs::texdesc_addressing_mode(regs::GC_TEX_ADDRESSING_LINEAR));
}

#[test]
fn linear_matches_texture_rs_where_padding_would_show() {
    // not a multiple of 16 wide or 4 high: nothing is padded for a linear
    // texture (texture.rs:384, :401); HALIGN stays FOUR even with rs_align.
    assert_linear_equals_texture_rs(100, 30, 100, 6);
    let mut i = input(100, 30, 6, Layout::Linear, 100);
    i.rs_align = true;
    let got = tex_words(&i).unwrap();
    assert_eq!(got.linear_stride, 400);
    assert_eq!(got.slice, 12000);
    assert_eq!(got.config1_halign, 0);
    // a pitch wider than the row, and a level whose offset needs the 64 round
    assert_linear_equals_texture_rs(100, 30, 128, 6);
    assert_linear_equals_texture_rs(5, 3, 5, 3);
}

#[test]
fn super_tiled_256_chain_offsets() {
    // 256x256 at 4 B: L0 256x256 = 262144; L1 128x128 = 65536; L2 64x64 =
    // 16384; L3..L8 (32 down to 1) pad to 64x64 = 16384 each.
    let w = tex_words(&input(256, 256, 9, Layout::SuperTiled, 0)).unwrap();
    let want = [0, 262144, 327680, 344064, 360448, 376832, 393216, 409600, 425984];
    assert_eq!(&w.level_offset[..9], &want);
    assert_eq!(w.size, 442368);
    assert_eq!(w.linear_stride, 1024);
    assert_eq!(w.slice, 262144);
    assert_eq!(&w.level_stride[..4], &[1024, 512, 256, 256]);
    assert_eq!(w.config0_addressing, regs::texdesc_addressing_mode(regs::GC_TEX_ADDRESSING_TILED));
    assert_eq!(w.config1_halign, regs::texdesc_halign(2));
}

#[test]
fn super_tiled_800x600_chain_offsets() {
    // L0 800x600 pads to 832x640 (3328 B rows) = 2129920; L1 400x300 -> 448x320
    // = 573440; L2 200x150 -> 256x192 = 196608; L3 100x75 -> 128x128 = 65536;
    // L4..L8 (50x37 down to 3x2) -> 64x64 = 16384 each.
    let w = tex_words(&input(800, 600, 9, Layout::SuperTiled, 0)).unwrap();
    let want = [0, 2129920, 2703360, 2899968, 2965504, 2981888, 2998272, 3014656, 3031040];
    assert_eq!(&w.level_offset[..9], &want);
    assert_eq!(w.size, 3047424);
    assert_eq!(w.linear_stride, 3328);
    assert_eq!(w.slice, 3328 * 640);
    assert_eq!(&w.level_stride[..4], &[3328, 1792, 1024, 512]);
}

#[test]
fn tiled_halign_follows_rs_align() {
    let mut i = input(256, 256, 1, Layout::Tiled, 0);
    let four = tex_words(&i).unwrap();
    assert_eq!(four.config1_halign, regs::texdesc_halign(regs::GC_TEXTURE_HALIGN_FOUR));
    assert_eq!(four.align, Alignment { width: 4, height: 4, base: 64 });
    i.rs_align = true;
    i.width = 100;
    i.height = 30;
    let sixteen = tex_words(&i).unwrap();
    assert_eq!(sixteen.config1_halign, regs::texdesc_halign(regs::GC_TEXTURE_HALIGN_SIXTEEN));
    assert_eq!(sixteen.align.width, 16);
    // 100 -> 112 texels, 30 -> 32 rows
    assert_eq!(sixteen.linear_stride, 448);
    assert_eq!(sixteen.slice, 448 * 32);
}

#[test]
fn sampler_ts_carries_the_fill_and_is_level_zero_only() {
    let mut i = input(256, 256, 1, Layout::SuperTiled, 0);
    i.ts = Ts::Sampler { fill: 0xffff_ff00 };
    let w = tex_words(&i).unwrap();
    assert!(w.ts_enable);
    assert_eq!(w.ts_clear_value, 0xffff_ff00);
    assert!(!tex_words(&input(256, 256, 1, Layout::SuperTiled, 0)).unwrap().ts_enable);
    i.levels = 2;
    assert_eq!(tex_words(&i), Err(Error::TsMipped));
    i.levels = 1;
    i.layout = Layout::Linear;
    i.stride_texels = 256;
    assert_eq!(tex_words(&i), Err(Error::TsLinear));
}

#[test]
fn alignment_errors_are_refused() {
    assert!(tex_words(&input(256, 256, 1, Layout::SuperTiled, 0)).is_ok());
    // base off the 64-byte line, every layout
    for layout in [Layout::Linear, Layout::Tiled, Layout::SuperTiled] {
        let mut i = input(64, 64, 1, layout, 64);
        i.base = 32;
        assert_eq!(tex_words(&i), Err(Error::Base), "{layout:?}");
        i.base = 128;
        assert!(tex_words(&i).is_ok(), "{layout:?}");
    }
    // a tiled pitch that is not a multiple of the width padding, or below the row
    assert_eq!(tex_words(&input(100, 64, 1, Layout::SuperTiled, 100)), Err(Error::Stride));
    assert_eq!(tex_words(&input(100, 64, 1, Layout::SuperTiled, 64)), Err(Error::Stride));
    assert!(tex_words(&input(100, 64, 1, Layout::SuperTiled, 128)).is_ok());
    let mut r = input(100, 64, 1, Layout::Tiled, 100);
    r.rs_align = true;
    assert_eq!(tex_words(&r), Err(Error::Stride));
    r.stride_texels = 112;
    assert!(tex_words(&r).is_ok());
    // a linear pitch below the row, none given, and the empty cases
    assert_eq!(tex_words(&input(100, 8, 1, Layout::Linear, 99)), Err(Error::Stride));
    assert_eq!(tex_words(&input(100, 8, 1, Layout::Linear, 0)), Err(Error::Empty));
    assert_eq!(tex_words(&input(0, 8, 1, Layout::Tiled, 0)), Err(Error::Empty));
    assert_eq!(tex_words(&input(8, 8, 0, Layout::Tiled, 0)), Err(Error::Levels));
    assert_eq!(tex_words(&input(8, 8, 17, Layout::Tiled, 0)), Err(Error::Levels));
}
