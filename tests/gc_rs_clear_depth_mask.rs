// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RS fill of a D24S8 clear, `src/vivante/rs_clear.rs`, run through
//! `rs_interp` (as tests/gc_ts_resolve_interp.rs does for the TS resolve).
//!
//! glmark2 shadow's and refract's depth clears are depth-only: the fill's
//! RS_CLEAR_CONTROL bits are 0xeeee (the stencil byte keeps its value), so no
//! TS fast clear is possible. tests/gc_rs_clear.rs and gc_rs_clear_stream.rs
//! pin the words against Mesa and the stream; this file pins what those words
//! do to memory:
//!
//! * a depth-only clear: bits 0xeeee, the stencil byte of every pixel as it
//!   was, depth bytes the clear word's;
//! * a depth+stencil clear: bits 0xffff, every byte the clear word's;
//! * a colour clear for contrast: bits 0xffff on a 32-bit colour surface.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_rs_clear_depth_mask`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::regs::*;
use yakogl::vivante::rs_clear::{self, *};
use yakogl::vivante::rs_interp::{Memory, RsInterp};

const ADDR: u32 = 0x1000_0000;
const W: u32 = 64;
const H: u32 = 64;

fn surface() -> Surface {
    Surface { addr: ADDR, padded_width: W, padded_height: H, bpp: 4, pitch: 0 }
}

fn ds(depth: f32, stencil: u8, clear_depth: bool, clear_stencil: bool) -> Clear {
    Clear {
        width: W,
        height: H,
        scissor: None,
        colour: None,
        depth_stencil: Some(DepthStencilClear {
            surface: surface(),
            has_stencil: true,
            clear_depth,
            clear_stencil,
            depth,
            stencil,
            depth_write: true,
            stencil_write_mask: 0xff,
        }),
    }
}

/// The surface's stale content: stencil byte (low) = a function of the pixel.
fn stale(i: u32) -> u32 {
    (i.wrapping_mul(0x9e37_79b1) & 0xffff_ff00) | (i & 0xff) | 0x01
}

/// Runs `words` over a stale surface; returns the surface's pixels.
fn run(words: &[u32]) -> (Vec<u32>, usize, Vec<(u32, u32)>) {
    let w: Vec<(u32, u32)> = rs_clear::state_writes(words).collect();
    let mut mem = Memory::new();
    let mut bytes = Vec::new();
    for i in 0..W * H {
        bytes.extend_from_slice(&stale(i).to_le_bytes());
    }
    mem.map(ADDR, bytes);
    let mut rs = RsInterp::new();
    // The fill reads no source, but the interpreter wants an address.
    rs.write(GC_RS_SOURCE_ADDR, ADDR, &mut mem);
    rs.run(&w, &mut mem);
    let px = mem.region(ADDR).chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    (px, rs.jobs.len(), w)
}

fn clear_bits(w: &[(u32, u32)]) -> Vec<u32> {
    w.iter().filter(|&&(r, _)| r == GC_RS_CLEAR_CONTROL).map(|&(_, v)| v).collect()
}

#[test]
fn a_depth_only_clear_keeps_the_stencil_byte() {
    let words = rs_clear(&ds(0.5, 0x5a, true, false)).unwrap();
    let (px, jobs, w) = run(words.as_slice());
    assert_eq!(clear_bits(&w), vec![RS_CLEAR_MODE_ENABLED1 | 0xeeee]);
    assert_eq!(jobs, 1);
    let word = d24s8_word(0.5, 0x5a);
    for (i, &p) in px.iter().enumerate() {
        // depth bytes from the clear word, stencil byte as it was (the
        // stencil value packed into the word is not written)
        assert_eq!(p & 0xffff_ff00, word & 0xffff_ff00, "pixel {i} depth");
        assert_eq!(p & 0xff, stale(i as u32) & 0xff, "pixel {i} stencil kept");
    }
    // the kept stencil really differs from the clear's, or the test shows nothing
    assert!(px.iter().enumerate().any(|(i, _)| stale(i as u32) & 0xff != 0x5a));
}

#[test]
fn a_depth_and_stencil_clear_writes_every_byte() {
    let words = rs_clear(&ds(1.0, 0x5a, true, true)).unwrap();
    let (px, jobs, w) = run(words.as_slice());
    assert_eq!(clear_bits(&w), vec![RS_CLEAR_MODE_ENABLED1 | 0xffff]);
    assert_eq!(jobs, 1);
    assert_eq!(d24s8_word(1.0, 0x5a), 0xffff_ff5a);
    assert!(px.iter().all(|&p| p == 0xffff_ff5a));
}

#[test]
fn a_stencil_only_clear_keeps_the_depth_bytes() {
    let words = rs_clear(&ds(0.25, 0x77, false, true)).unwrap();
    let (px, _, w) = run(words.as_slice());
    assert_eq!(clear_bits(&w), vec![RS_CLEAR_MODE_ENABLED1 | 0x1111]);
    for (i, &p) in px.iter().enumerate() {
        assert_eq!(p & 0xffff_ff00, stale(i as u32) & 0xffff_ff00, "pixel {i} depth kept");
        assert_eq!(p & 0xff, 0x77);
    }
}

#[test]
fn a_colour_clear_writes_all_four_bytes() {
    let c = Clear {
        width: W,
        height: H,
        scissor: None,
        colour: Some(ColourClear { surface: surface(), pixel: 0x8040_20ff, write_mask: 0xf }),
        depth_stencil: None,
    };
    let words = rs_clear(&c).unwrap();
    let (px, jobs, w) = run(words.as_slice());
    assert_eq!(clear_bits(&w), vec![RS_CLEAR_MODE_ENABLED1 | 0xffff]);
    assert_eq!(jobs, 1);
    assert!(px.iter().all(|&p| p == 0x8040_20ff));
}
