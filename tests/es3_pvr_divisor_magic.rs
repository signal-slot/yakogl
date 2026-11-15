// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR hardware instance divisors (docs/notes/instanceid-per-draw.md,
//! fix P3; issue #18): a reference model of the numbers Mesa's vertex PDS
//! program uses to divide the instance id by a stream's divisor, checked
//! against integer division.
//!
//! The model follows Mesa 26.2.1
//! `src/imagination/vulkan/pds/pvr_pipeline_pds.c:771-1008` (Imagination
//! Technologies Ltd., MIT; vendored in `third_party/mesa-26.2.1`). It is
//! written from the formula there, not copied from it. For an instance-rate
//! stream with divisor d > 1 Mesa:
//!
//! - takes `shift` = ceil(log2 d), the bit of the next power of two
//!   (lines 845-846),
//! - stores one 32-bit literal in the PDS data segment, `multiplier` =
//!   (2^(shift+31) + d - 1) / d in 64-bit integer arithmetic, truncated to
//!   32 bits (lines 858-875); it never needs the truncation (the test below),
//! - subtracts the base instance, multiplies in a 64-bit MAD (lines 878-887),
//!   then shifts right by shift + 31 as two SFTLP64 with immediates: first by
//!   31 (`0xFFFE1`, lines 893-909), then by `shift` (lines 911-924); the
//!   encoder keeps the low 8 bits of the immediate
//!   (`PVR_ROGUE_PDSINST_REGS32_MASK`, pvr_rogue_pds_encode.h:77), a signed
//!   count where negative is a right shift,
//! - adds the base instance back (lines 926-951).
//!
//! Mesa has no power-of-two shortcut: d = 2^k gives multiplier 2^31 and
//! shift k, and the product shifted by 31 + k is the instance shifted by k.
//! So one SFTLP32 by k gives the same index for every 32-bit instance id,
//! which is what P3 may emit instead (pinned below). Divisor 0 at instance
//! rate is Vulkan's "every instance reads the base instance's element"
//! (lines 956-1003); GL ES
//! divisor 0 is a per-vertex stream and never reaches this code.
//!
//! The quotient is exact for every instance id below 2^31: with the rounding
//! error e = multiplier * d - 2^(shift+31), 0 <= e < d <= 2^shift, so
//! i * e < 2^(shift+31) for i < 2^31. GL's instance count is a GLsizei, so
//! that is every id a draw has. Above 2^31 it can be wrong (d = 7,
//! i = 0xfffffffb, pinned below).
//!
//! P1's chunked draws (`instance_id_modifier`) add the chunk's first
//! instance to the id before the divide, so the divide sees the draw's id;
//! moving
//! the stream's base address by first / d elements instead is right only when
//! first % d == 0 (pinned below).
//!
//! Pure arithmetic, no back end: it runs in the PowerVR rows of the suite.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_pvr_divisor_magic
//! ```

#![cfg(feature = "gpu-powervr")]

/// How the vertex PDS program turns the instance id into a stream index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Divisor 0 at instance rate (Vulkan only): every instance reads the
    /// base instance's element.
    Constant,
    /// Divisor 1: the instance id itself, no data word.
    PerInstance,
    /// Divisor > 1: the 64-bit MAD by `multiplier`, then a right shift by
    /// 31 + `shift`.
    MulShift,
}

/// Mesa's numbers for an instance-rate stream of divisor `d`:
/// (kind, the 32-bit data word, the shift after the fixed 31).
fn magic(d: u32) -> (Kind, u32, u32) {
    match d {
        0 => (Kind::Constant, 0, 0),
        1 => (Kind::PerInstance, 0, 0),
        _ => {
            let (m, shift) = magic_wide(d);
            (Kind::MulShift, m as u32, shift)
        }
    }
}

/// The multiplier before Mesa's cast to 32 bits, and the shift.
fn magic_wide(d: u32) -> (u64, u32) {
    assert!(d > 1);
    let shift = 32 - (d - 1).leading_zeros();
    // Mesa's (2^(shift+31) + d - 1) / d
    let m = (1u64 << (shift + 31)).div_ceil(d as u64);
    (m, shift)
}

/// The two SFTLP64 immediates as the encoder stores them (8 bits, signed,
/// negative = right): by 31, then by `shift`.
fn shift_immediates(shift: u32) -> [u8; 2] {
    [(0xFFFE1u32 & 0xff) as u8, (shift as i32).wrapping_neg() as u8]
}

/// Decode one stored immediate back to a right-shift count.
fn right_shift_of(imm: u8) -> u32 {
    let v = imm as i8;
    assert!(v <= 0, "a left shift: {imm:#x}");
    v.unsigned_abs() as u32
}

/// The index the PDS program fetches the stream at, for instance id `id`
/// (already including any base) and the base instance `base`, the way the
/// program runs it: subtract, MAD, two shifts, add.
fn index(d: u32, id: u32, base: u32) -> u32 {
    let (kind, m, shift) = magic(d);
    match kind {
        Kind::Constant => base,
        Kind::PerInstance => id,
        Kind::MulShift => {
            let rel = id.wrapping_sub(base);
            let [a, b] = shift_immediates(shift);
            let prod = rel as u64 * m as u64;
            let q = (prod >> right_shift_of(a)) >> right_shift_of(b);
            (q as u32).wrapping_add(base)
        }
    }
}

/// The divisors the tests walk: every one up to 4096, the powers of two and
/// their neighbours, the GL limit P3 sets (0xffff) and the 32-bit ends.
fn divisors() -> Vec<u32> {
    let mut v: Vec<u32> = (2..=4096).collect();
    for k in 1..32 {
        let p = 1u32 << k;
        v.extend([p - 1, p, p + 1]);
    }
    v.extend([0xfffe, 0xffff, 0x1_0000, 0x1_0001, 0x7fff_ffff, u32::MAX]);
    v.retain(|&d| d > 1);
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn the_data_words_for_the_divisors_the_c_test_pins() {
    // (d, multiplier, shift, the two immediates)
    let want: [(u32, u32, u32, [u8; 2]); 4] = [
        (2, 0x8000_0000, 1, [0xe1, 0xff]),
        (3, 0xaaaa_aaab, 2, [0xe1, 0xfe]),
        (4, 0x8000_0000, 2, [0xe1, 0xfe]),
        (0xffff, 0x8000_8001, 16, [0xe1, 0xf0]),
    ];
    for (d, m, shift, imm) in want {
        assert_eq!(magic(d), (Kind::MulShift, m, shift), "d = {d:#x}");
        assert_eq!(shift_immediates(shift), imm, "d = {d:#x}");
    }
    assert_eq!(magic(5), (Kind::MulShift, 0xcccc_cccd, 3));
    assert_eq!(magic(7), (Kind::MulShift, 0x9249_2493, 3));
    assert_eq!(magic(0x1_0000), (Kind::MulShift, 0x8000_0000, 16));
    assert_eq!(magic(u32::MAX), (Kind::MulShift, 0x8000_0001, 32));
    assert_eq!(magic(1), (Kind::PerInstance, 0, 0));
    assert_eq!(magic(0), (Kind::Constant, 0, 0));
}

#[test]
fn the_multiplier_always_fits_the_32_bit_data_word() {
    for d in divisors() {
        let (m, shift) = magic_wide(d);
        assert!(m < 1u64 << 32, "d = {d:#x}: multiplier {m:#x}");
        assert!(m >= 1u64 << 31, "d = {d:#x}: multiplier {m:#x}");
        // the total right shift stays within the encoder's signed 8 bits
        assert!(shift <= 32, "d = {d:#x}");
    }
}

#[test]
fn the_rounding_error_keeps_every_id_below_2_31_exact() {
    // The proof in the header, per divisor: e = m * d - 2^(shift+31) is in
    // [0, 2^shift), so i * e < 2^(shift+31) for every i < 2^31.
    for d in divisors() {
        let (m, shift) = magic_wide(d);
        let e = m as u128 * d as u128 - (1u128 << (shift + 31));
        assert!(e < d as u128 && e < 1u128 << shift, "d = {d:#x}: e = {e}");
    }
}

#[test]
fn the_quotient_is_integer_division() {
    // Every id up to 4095 for every divisor up to 4096; for every divisor,
    // the ids around each of its multiples near 0, 2^16, 2^24 and 2^31.
    for d in divisors() {
        if d <= 4096 {
            for i in 0..4096u32 {
                assert_eq!(index(d, i, 0), i / d, "d = {d}, i = {i}");
            }
        }
        for top in [1u64 << 16, 1 << 24, (1 << 31) - 1] {
            let q = top / d as u64;
            for k in q.saturating_sub(2)..=q + 1 {
                let mul = k * d as u64;
                for i in [mul.wrapping_sub(1), mul, mul + 1, mul + d as u64 - 1] {
                    if i < 1 << 31 {
                        let i = i as u32;
                        assert_eq!(index(d, i, 0), i / d, "d = {d:#x}, i = {i:#x}");
                    }
                }
            }
        }
        let i = (1u32 << 31) - 1;
        assert_eq!(index(d, i, 0), i / d, "d = {d:#x}, i = {i:#x}");
    }
}

#[test]
fn ids_above_2_31_are_outside_the_exact_range() {
    // The bound is real: a 32-bit id this large is wrong for d = 7. GL never
    // asks for it (the instance count is a GLsizei).
    let i = 0xffff_fffbu32;
    assert_ne!(index(7, i, 0), i / 7);
}

#[test]
fn a_power_of_two_is_one_right_shift_by_log2() {
    // P3's SFTLP32 shortcut gives Mesa's index for every 32-bit id.
    for k in 1..32u32 {
        let d = 1u32 << k;
        let (kind, m, shift) = magic(d);
        assert_eq!((kind, m, shift), (Kind::MulShift, 1 << 31, k));
        for i in [0u32, 1, d - 1, d, d + 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
            assert_eq!(index(d, i, 0), i >> k, "d = {d:#x}, i = {i:#x}");
        }
    }
}

#[test]
fn the_base_instance_is_taken_off_before_the_divide() {
    for d in [2u32, 3, 4, 7, 0xffff] {
        for base in [0u32, 1, 5, 64] {
            for rel in 0..256u32 {
                assert_eq!(index(d, base + rel, base), base + rel / d, "d = {d}, base = {base}");
            }
        }
    }
}

#[test]
fn a_chunk_goes_through_the_modifier_never_a_base_move() {
    // P1 draws instances [first, first + n) as one chunk whose ids restart
    // at 0; instance_id_modifier adds `first` back before the divide, which
    // is the index of the whole draw. Moving the stream base by first / d
    // elements and dividing the chunk's own id is the same only when
    // first % d == 0.
    for d in [2u32, 3, 4, 0xffff] {
        for first in 0..(2 * d.min(64)) {
            // the chunk's ids: the first few and those around the next
            // multiples of d after `first`
            let cross = d - first % d;
            let mut ids: Vec<u32> = (0..3 * d.min(64)).collect();
            ids.extend([cross - 1, cross, cross + 1, cross + d - 1, cross + d]);
            let mut base_move_ok = true;
            for j in ids {
                let whole = (first + j) / d;
                assert_eq!(index(d, first + j, 0), whole, "d = {d}, first = {first}");
                base_move_ok &= first / d + index(d, j, 0) == whole;
            }
            assert_eq!(base_move_ok, first % d == 0, "d = {d}, first = {first}");
        }
    }
}
