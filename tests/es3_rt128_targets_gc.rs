// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the hardware target list of a draw with 128-bit
//! (RGBA32I/UI) colour attachments, and the register words it gives (item
//! gc-rt128-hwtargets; docs/notes/gc-128bit-rt-shader.md sections 3 and 4,
//! "Sequence of reviewable items" step 4, and its section 5 tests 2, 4, 5).
//!
//! A 128-bit attachment is two 64-bit hardware targets, as Mesa's etnaviv
//! draws one (`etna_fb_expand_128bit`): its R,G plane in its own place, its
//! B,A plane appended after the ordinary targets, in attachment order, up to
//! eight. `vivante::backend::hw_target_list` makes the list,
//! `vivante::shader::ps_target_words` the PS_OUTPUT_REG / PS_OUTPUT_REG2,
//! PS_CONTROL (RT_COUNT) and PS_CONTROL_EXT words from it, and
//! `hw_target_mask` / `hw_target_surface` each target's colour mask and bank
//! words. All of it is inert: `GC_INT_RT_128BPP` is still refused by name
//! (the GL core's `rt_format_drawable`, the back end's `draw_one`), so these
//! tests hand the functions a `GlesDraw` directly and no draw reaches them.
//!
//! What it cannot see: whether the GC7000UL's 8-RT bank takes RT4..RT7 and
//! keeps 32 bits on a plane (the board probe, step 6 of the note).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_rt128_targets_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", not(feature = "report-es2")))]

use yakogl::backend::*;
use yakogl::shaderc::{ShcViv, ShcVivIo};
use yakogl::vivante::backend::{
    hw_target_list, hw_target_mask, hw_target_surface, mrt_targets, HwTarget, GC_INTEGER_RT_FORMATS, GC_INT_RT_128BPP,
    GC_MAX_HW_TARGETS,
};
use yakogl::vivante::regs::*;
use yakogl::vivante::shader::ps_target_words;
use yakogl::vivante::wide_target::{padded, plane_offset, stride};

const W: u32 = 20;
const H: u32 = 10;

/// A draw of `formats` in draw-buffer slots 0.. (`None`: a GL_NONE slot, `gpu`
/// 0), each `W` x `H` at its own address.
fn draw(formats: &[Option<u8>], shift: u8) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    for (i, f) in formats.iter().enumerate() {
        let gpu = f.map_or(0, |_| 0x1000_0000 + 0x10_0000 * i as u64);
        d.targets[i] = GlesDrawTarget { gpu, w: W, h: H, format: f.unwrap_or(0), out_shift: 0 };
    }
    d.targets[0].out_shift = shift;
    d.target_gpu = d.targets[0].gpu;
    d.target_w = W;
    d.target_h = H;
    d.target_format = d.targets[0].format;
    d.num_targets = formats.len() as u8;
    d
}

fn tags(d: &GlesDraw) -> Vec<(u8, u8)> {
    hw_target_list(d).list().iter().map(|t| (t.slot, t.plane)).collect()
}

/// A fragment shader with RT0's register `rt0` and an output row per
/// `(output index, register)`: the row's `slot` is 4 + output.
fn fs(rt0: u8, rows: &[(u8, u8)]) -> ShcViv {
    let mut v = ShcViv::ZERO;
    v.ps_color_out_reg = rt0;
    for (i, &(out, reg)) in rows.iter().enumerate() {
        v.out[i] = ShcVivIo { reg, slot: 4 + out, ..ShcVivIo::ZERO };
    }
    v.num_out = rows.len() as u8;
    v
}

fn rt_count(control: u32) -> u32 {
    (control >> 8) & 7
}

fn mode(control_ext: u32, rt: u32) -> u32 {
    (control_ext >> (4 * rt)) & 7
}

const U: Option<u8> = Some(GLES_SURF_RGBA32UI);
const I: Option<u8> = Some(GLES_SURF_RGBA32I);
const P: Option<u8> = Some(GLES_SURF_RGBA8888);

/// The precondition of everything here: the formats are still refused, so
/// no shipped draw has a plane.
#[test]
fn rgba32_targets_are_still_refused() {
    let bits = GC_INT_RT_128BPP;
    assert_ne!(bits, 0);
    assert_eq!(GC_INTEGER_RT_FORMATS & bits, 0, "RGBA32I/UI are not drawable on the GC");
    assert_eq!(bits.count_ones(), 2);
    assert_eq!(GC_MAX_HW_TARGETS, 8);
}

/// Ordinary draws keep today's list: slots in order, a GL_NONE slot skipped,
/// no plane, at most four.
#[test]
fn plain_draws_keep_their_list() {
    assert_eq!(tags(&draw(&[P], 0)), [(0, 0)]);
    assert_eq!(tags(&draw(&[P, P, P, P], 0)), [(0, 0), (1, 0), (2, 0), (3, 0)]);
    assert_eq!(tags(&draw(&[P, None, P], 0)), [(0, 0), (2, 0)]);
    let rg32 = Some(GLES_SURF_RG32UI);
    assert_eq!(tags(&draw(&[rg32], 0)), [(0, 0)], "a 64-bit target is one target");
    for n in 1..=4 {
        let d = draw(&[P; 4][..n], 0);
        let w = ps_target_words(&d, &fs(3, &[(1, 4), (2, 5), (3, 6)]));
        assert_eq!(w.targets, n);
        assert_eq!(rt_count(w.control), n as u32 - 1);
        assert_eq!(w.output_reg[1], 0, "no second word below five targets");
    }
}

/// One 128-bit, one plain and one 128-bit attachment in slots 0, 1, 2: the
/// hardware targets are [0, 1, 2, 0p, 2p], RT_COUNT(4).
#[test]
fn rgba32_plain_rgba32_is_five_targets() {
    let d = draw(&[U, P, I], 0);
    assert_eq!(tags(&d), [(0, 0), (1, 0), (2, 0), (0, 1), (2, 1)]);
    let hw = hw_target_list(&d);
    assert_eq!((0..5).map(|rt| hw.output(rt, 0)).collect::<Vec<_>>(), [0, 1, 2, 4, 5], "B,A halves are outputs 4, 5");

    /* outputs 0..2 in registers 3, 4, 5; the companions 4 and 5 in 6 and 7 */
    let w = ps_target_words(&d, &fs(3, &[(1, 4), (2, 5), (4, 6), (5, 7)]));
    assert_eq!(w.targets, 5);
    assert_eq!(rt_count(w.control), 4, "RT_COUNT(4): five targets, planes counted");
    assert_eq!(w.output_reg[0], 3 | 4 << 8 | 5 << 16 | 6 << 24, "RT0..3: outputs 0, 1, 2 and 0's B,A");
    assert_eq!(w.output_reg[1], 7, "PS_OUTPUT_REG2 byte 0: 2's B,A, unsaturated");
    for rt in [0, 2, 3, 4] {
        assert_eq!(mode(w.control_ext, rt), GC_PS_OUTPUT_MODE_UIF32, "plane RT{rt}: UIF32");
    }
    assert_eq!(mode(w.control_ext, 1), GC_PS_OUTPUT_MODE_NORMAL, "the RGBA8 target");
    assert_eq!(w.control & !(7 << 8), ps_control_saturate_rt(1), "only the unorm target saturates");

    /* the bank words of each plane: G32R32F, the 8-byte tiled stride, plane 1
     * at the half-way point of the two-plane image */
    let (pw, ph) = padded(W, H);
    let half = plane_offset(pw, ph) as u64;
    let surf: Vec<_> = hw.list().iter().map(|&t| hw_target_surface(&d, t)).collect();
    assert_eq!(surf[0], (d.target_gpu, stride(W), GC_PE_FORMAT_G32R32F));
    assert_eq!(surf[2], (d.targets[2].gpu, stride(W), GC_PE_FORMAT_G32R32F));
    assert_eq!(surf[3], (d.target_gpu + half, stride(W), GC_PE_FORMAT_G32R32F), "0p");
    assert_eq!(surf[4], (d.targets[2].gpu + half, stride(W), GC_PE_FORMAT_G32R32F), "2p");
    assert_eq!(
        surf[1],
        (d.targets[1].gpu, W.next_multiple_of(2) * 4, GC_PE_FORMAT_A8R8G8B8),
        "the plain one, as before"
    );
    assert!(half > 0);
}

/// A null middle draw buffer and `out_shift` move the user outputs, not the
/// companions: the B,A output is the attachment's rank among the 128-bit
/// ones, 4 + k.
#[test]
fn companion_output_ignores_null_slots_and_shift() {
    let d = draw(&[U, None, U], 1);
    assert_eq!(tags(&d), [(0, 0), (2, 0), (0, 1), (2, 1)]);
    let hw = hw_target_list(&d);
    assert_eq!((0..4).map(|rt| hw.output(rt, 1)).collect::<Vec<_>>(), [1, 3, 4, 5]);
    let w = ps_target_words(&d, &fs(2, &[(1, 9), (3, 10), (4, 11), (5, 12)]));
    assert_eq!(w.output_reg[0], 9 | 10 << 8 | 11 << 16 | 12 << 24);
    assert_eq!(w.output_reg[1], 0);
    assert_eq!(rt_count(w.control), 3);
}

/// Four 128-bit attachments are eight hardware targets in the back end's
/// list; the GL core hands over at most `DRAW_TARGETS` attachments, so a GL
/// draw has at most twice that.
#[test]
fn four_rgba32_attachments() {
    let d = draw(&[U, I, U, I], 0);
    assert_eq!(mrt_targets(&d), 4);
    assert_eq!(tags(&d), [(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (1, 1), (2, 1), (3, 1)]);
    let rows: Vec<(u8, u8)> = (1..8).map(|o| (o, 10 + o)).collect();
    let w = ps_target_words(&d, &fs(10, &rows));
    assert_eq!(w.targets, GC_MAX_HW_TARGETS);
    assert_eq!(rt_count(w.control), 7, "RT_COUNT(7): all eight");
    assert_eq!(w.output_reg[0], 10 | 11 << 8 | 12 << 16 | 13 << 24);
    assert_eq!(w.output_reg[1], 14 | 15 << 8 | 16 << 16 | 17 << 24);
    for rt in 0..8 {
        assert_eq!(mode(w.control_ext, rt), GC_PS_OUTPUT_MODE_UIF32);
    }

    /* what the GL core can hand over today */
    let n = DRAW_TARGETS;
    let d = draw(&[U; 4][..n], 0);
    assert_eq!(hw_target_list(&d).n, 2 * n);
    assert!(2 * n <= GC_MAX_HW_TARGETS);
    assert_eq!(rt_count(ps_target_words(&d, &fs(10, &rows)).control), 2 * n as u32 - 1);
}

/// The 16 user masks: plane 0 takes R,G (`mask & 3`), plane 1 B,A in its
/// R,G lanes (`(mask >> 2) & 3`); an ordinary target keeps the mask.
#[test]
fn plane_colour_masks() {
    let d = draw(&[U, P], 0);
    for m in 0u32..16 {
        assert_eq!(hw_target_mask(&d, HwTarget { slot: 0, plane: 0 }, m), m & 3, "R,G plane, mask {m:#06b}");
        assert_eq!(hw_target_mask(&d, HwTarget { slot: 0, plane: 1 }, m), (m >> 2) & 3, "B,A plane, mask {m:#06b}");
        assert_eq!(hw_target_mask(&d, HwTarget { slot: 1, plane: 0 }, m), m, "RGBA8, mask {m:#06b}");
    }
}

/// No colour surface: slot 0 is listed as ever, but a 128-bit format left in
/// `target_format` gets no plane.
#[test]
fn no_colour_surface_has_no_plane() {
    let mut d = draw(&[U], 0);
    d.target_gpu = 0;
    d.targets[0].gpu = 0;
    assert_eq!(tags(&d), [(0, 0)]);
}
