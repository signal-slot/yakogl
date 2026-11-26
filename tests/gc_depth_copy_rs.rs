// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: outcome (b).1's resolve-engine copy of a row-linear D24S8 surface
//! into a super-tiled D24X8 depth view (src/vivante/depth_sample.rs,
//! docs/notes/gc-depth-sample-wire.md (b).1), at glmark2's sizes: the 1600x1200
//! depth texture of shadow and refract (docs/notes/gc-slow-shadow-refract.md),
//! a 90x70 one, and the 64-pixel alignment edges.
//!
//! For each surface `ds_cache` would allocate (64-pixel aligned stride and
//! rows) it checks the register writes `copy_rs_writes` returns (formats,
//! DEST_TILED, the super-tiled stride, the window, the addresses, one kick),
//! `copy_bytes`, and that a CPU decode of the copy through the sampler words'
//! `texel_addr` returns the source surface's depth bit for bit. What the
//! hardware does with them is the board's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_copy_rs
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use yakogl::backend::{GlesTextureBind, GLES_TEXTYPE_2D};
use yakogl::util::ENOTSUP;
use yakogl::vivante::depth_sample::*;
use yakogl::vivante::layout::{self, Layout};
use yakogl::vivante::regs::*;
use yakogl::vivante::rs_interp::{Memory, RsInterp};

const SURF: u64 = 0x0010_0000;
const COPY: u64 = 0x0200_0000;

/// `ds_cache`'s surface for a `w` x `h` depth texture.
fn surface(w: u32, h: u32) -> DepthView {
    DepthView { gpu: SURF, stride: layout::align_up(w, 64) * 4, rows: layout::align_up(h, 64) }
}

fn texture(w: u32, h: u32) -> GlesTextureBind {
    GlesTextureBind { width: w, height: h, type_: GLES_TEXTYPE_2D, layers: 1, mip_levels: 1, ..GlesTextureBind::ZERO }
}

/// A D24S8 word of varied depth and stencil.
fn word(x: u32, y: u32) -> u32 {
    let depth = (x.wrapping_mul(0x9e37) ^ y.wrapping_mul(0x7f4a_7c15) ^ (x << 13)) & 0xff_ffff;
    (depth << 8) | ((x * 3 + y * 5) & 0xff)
}

fn reg(w: &[(u32, u32)], r: u32) -> u32 {
    let all: Vec<u32> = w.iter().filter(|(k, _)| *k == r).map(|(_, v)| *v).collect();
    assert_eq!(all.len(), 1, "register {r:#x} is written once");
    all[0]
}

fn check(w: u32, h: u32) {
    let v = surface(w, h);
    let bytes = (v.stride * v.rows) as usize;
    assert_eq!(copy_bytes(&v), Some(bytes), "{w}x{h}: copy_bytes");
    let wr = copy_rs_writes(&v, COPY).unwrap();

    let a8 = rs_source_format(GC_RS_FORMAT_A8R8G8B8) | rs_dest_format(GC_RS_FORMAT_A8R8G8B8);
    assert_eq!(reg(&wr, GC_RS_CONFIG), a8 | GC_RS_DEST_TILED, "{w}x{h}: A8R8G8B8 both sides, DEST_TILED only");
    assert_eq!(reg(&wr, GC_RS_SOURCE_STRIDE), v.stride, "{w}x{h}: linear source stride");
    assert_eq!(
        reg(&wr, GC_RS_DEST_STRIDE),
        (v.stride << 2) | GC_RS_DEST_STRIDE_TILING | GC_RS_DEST_STRIDE_SUPER_TILED_NEW,
        "{w}x{h}: super-tiled stride"
    );
    assert_eq!(
        reg(&wr, GC_RS_WINDOW_SIZE),
        rs_window_width(v.stride / 4) | rs_window_height(v.rows),
        "{w}x{h}: window"
    );
    assert_eq!(reg(&wr, rs_pipe_source_addr(0)), SURF as u32);
    assert_eq!(reg(&wr, rs_pipe_dest_addr(0)), COPY as u32);
    assert_eq!(reg(&wr, GC_RS_CLEAR_CONTROL), 0, "a copy, not a fill");
    assert_eq!(wr.iter().filter(|(r, _)| *r == GC_RS_KICKER).count(), 1, "one RS job");

    let mut lin = vec![0u8; bytes];
    for y in 0..v.rows {
        for x in 0..v.stride / 4 {
            let o = layout::offset(Layout::Linear, x, y, 4, v.stride);
            lin[o..o + 4].copy_from_slice(&word(x, y).to_le_bytes());
        }
    }
    let mut mem = Memory::new();
    mem.map(SURF as u32, lin.clone());
    mem.map(COPY as u32, vec![0xcd; bytes]);
    let mut rs = RsInterp::new();
    rs.run(&wr, &mut mem);
    assert_eq!(rs.jobs.len(), 1);
    assert_eq!(rs.jobs[0].dst.layout, Layout::SuperTiled, "{w}x{h}");
    assert_eq!(mem.region(SURF as u32), &lin[..], "{w}x{h}: the source is untouched");

    let s = copy_sampler_words(&texture(w, h), &v, COPY).unwrap();
    assert_eq!(s.layout(), Some(Layout::SuperTiled));
    assert_eq!(s.linear_stride, v.stride);
    for y in 0..h {
        for x in 0..w {
            let a = s.texel_addr(x, y).unwrap();
            let got = u32::from_le_bytes(mem.read(a, 4).try_into().unwrap());
            // D24X8 reads bits 31..8: the depth, bit for bit; the stencil byte is X
            assert_eq!(got >> 8, word(x, y) >> 8, "{w}x{h}: depth at ({x}, {y})");
        }
    }
}

#[test]
fn glmark2_shadow_and_refract_1600x1200() {
    let v = surface(1600, 1200);
    assert_eq!((v.stride, v.rows), (6400, 1216));
    check(1600, 1200);
}

#[test]
fn an_odd_size() {
    check(90, 70);
    check(1, 1);
}

#[test]
fn the_64_alignment_edges() {
    for (w, h) in [(63, 63), (64, 64), (65, 65), (64, 65), (65, 64), (128, 63), (129, 1)] {
        check(w, h);
    }
}

#[test]
fn a_refused_copy_has_no_writes() {
    let ok = surface(64, 64);
    assert_eq!(copy_rs_writes(&DepthView { stride: 4 * 48, ..ok }, COPY), Err(-ENOTSUP));
    assert_eq!(copy_rs_writes(&DepthView { rows: 60, ..ok }, COPY), Err(-ENOTSUP));
    assert_eq!(copy_rs_writes(&ok, COPY + 32), Err(-ENOTSUP));
    assert_eq!(copy_bytes(&DepthView { rows: 60, ..ok }), None);
}
