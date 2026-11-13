// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Reference model of the PowerVR per-stream instance divide the way Mesa
//! programs it, checked against plain integer division.
//!
//! The model follows Mesa 26.2.1 `src/imagination/vulkan/pds/pvr_pipeline_pds.c`
//! lines 771-885 (the `vertex_dma->divisor > 1` branch of the vertex PDS
//! program generator): there is no power-of-two special case and no SFTLP32
//! path in that code, every divisor above 1 takes the multiply and shift:
//!
//! ```text
//! shift      = (31 - clz(d - 1)) + 1            (line 800, ceil(log2 d))
//! multiplier = (2^(shift + 31) + (d - 1)) / d   (lines 811-815, 64-bit math,
//!                                                stored as a 32-bit literal)
//! out        = (instance * multiplier) >> (shift + 31)   (MAD, then SFTLP64)
//! ```
//!
//! Width: the MAD (`pvr_rogue_inst_encode_mad`) multiplies two 32-bit values
//! and adds a 64-bit zero register, so the product is the full 64-bit
//! `u32 * u32`; SFTLP64 then shifts that 64-bit value right by `shift + 31`
//! (in two steps when it exceeds 31, lines 886-912) and the low 32 bits are
//! the divided index. Divisor 0 and 1 take no multiply (0: base instance,
//! 1: the instance id itself), so the model reports them as such.
//!
//! Range: with e = multiplier * d - 2^(shift + 31) < d, the result is exact
//! for every instance id below 2^(shift + 31) / e, which is above 2^31 for
//! every d (d <= 2^shift). So the divide is exact for all ids a GL draw can
//! have (the instance count is a signed `GLsizei`, ids <= 0x7fff_ffff); the
//! last test pins that it is NOT exact up to `u32::MAX` (d = 7 is off by one
//! at 4294967284), so the exact range is not stretched to the full word.
//!
//! The test uses no crate items. The pinned literals (second test) are what
//! the C test of the PDS data words in driver/src/pvr_pds_render_gen.c
//! compares its multiplier constant and shift against.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_divisor_magic
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Kind {
    /// Divisor 0: the stream is constant, the index is the base instance.
    Zero,
    /// Divisor 1: the instance id is used as is.
    Identity,
    /// Divisor above 1: multiply by the 32-bit constant, shift right by `shift + 31`.
    MulShift,
}

/// `(kind, multiplier, shift)`; `shift` is Mesa's `shift` before the +31.
fn magic(d: u32) -> (Kind, u32, u32) {
    match d {
        0 => (Kind::Zero, 0, 0),
        1 => (Kind::Identity, 0, 0),
        _ => {
            let shift = (31 - (d - 1).leading_zeros()) + 1;
            let m = (1u64 << (shift + 31)).div_ceil(d as u64);
            (Kind::MulShift, m as u32, shift)
        }
    }
}

/// The PDS arithmetic: 32x32 -> 64-bit product, 64-bit right shift, low 32 bits.
fn apply(d: u32, instance: u32) -> u32 {
    let (kind, m, shift) = magic(d);
    match kind {
        Kind::Zero => 0,
        Kind::Identity => instance,
        Kind::MulShift => ((instance as u64 * m as u64) >> (shift + 31)) as u32,
    }
}

fn lcg(state: &mut u64) -> u32 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*state >> 32) as u32
}

#[test]
fn magic_divide_matches_integer_division() {
    let mut rng = 0x5eed_u64;
    for d in 1..=0xffffu32 {
        let check = |i: u32| {
            assert_eq!(apply(d, i), i / d, "d={d} instance={i}");
        };
        let (kind, m, shift) = magic(d);
        if d > 1 {
            assert_eq!(kind, Kind::MulShift);
            assert!((1..=16).contains(&shift), "d={d} shift={shift}");
            assert!(m != 0, "d={d}");
        }
        for i in 0..=0x1_0000u32 {
            check(i);
        }
        for c in [1u32 << 16, 1 << 24, 1 << 30, i32::MAX as u32] {
            for i in [d - 1, d, d + 1] {
                check(i);
            }
            // The multiples of d around the corner value c.
            let k = c / d;
            for kk in [k.saturating_sub(1), k, k.saturating_add(1)] {
                if let Some(p) = kk.checked_mul(d) {
                    check(p.saturating_sub(1));
                    check(p);
                }
            }
            check(c.saturating_sub(1));
            check(c);
        }
        for _ in 0..4096 {
            check(lcg(&mut rng) & 0x7fff_ffff);
        }
    }
}

#[test]
fn magic_literals_are_pinned() {
    let pinned: [(u32, u32, u32); 6] = [
        (2, 0x8000_0000, 1),
        (3, 0xaaaa_aaab, 2),
        (4, 0x8000_0000, 2),
        (5, 0xcccc_cccd, 3),
        (7, 0x9249_2493, 3),
        (0xffff, 0x8000_8001, 16),
    ];
    for (d, m, s) in pinned {
        assert_eq!(magic(d), (Kind::MulShift, m, s), "d={d}");
    }
    assert_eq!(magic(0).0, Kind::Zero);
    assert_eq!(magic(1).0, Kind::Identity);
}

#[test]
fn magic_is_not_exact_above_i32_max() {
    // d = 7: multiplier 0x9249_2493, shift 3; the rounded-up constant carries
    // an error that shows once instance * error reaches 2^34.
    assert_eq!(apply(7, 4294967284), 4294967284 / 7 + 1);
    assert_eq!(apply(7, 0x7fff_ffff), 0x7fff_ffff / 7);
}
