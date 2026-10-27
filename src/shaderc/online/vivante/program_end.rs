// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How a GC7000 program ends -- measured over the shipped table, because the
//! answer is "it does not".
//!
//! This file exists because a GPU wedge looked like a vertex program running
//! past its own end, and the first question was whether the reference marks
//! the end of a program with something we omit. It does not, and that is
//! worth pinning rather than re-deriving: the absence is what says a program's
//! extent is carried **solely by `usc_dw`**, the length the driver programs,
//! and therefore that a program running on is a statement about that length
//! and its placement rather than about the instruction words.
//!
//! Measured 2026-09-22 over all 324 blobs of the shipped table:
//!
//! - `usc_dw` is exactly 4 x the instruction count in 324 of 324.
//! - No trailing nop and no appended all-zero word after real code. 15 blobs
//!   end in a nop and all 15 have exactly one instruction -- the nop *is* the
//!   program. (Those are the same 15 empty rows established elsewhere as
//!   correct pass-throughs, reached here from a different direction.)
//! - No end-of-program flag: no operation has any bit set only when it is the
//!   last instruction, including bits outside every named field.
//! - No difference between the stages in any of the above.

#![cfg(all(test, feature = "gpu-vivante"))]

use super::decode::decode_instr;
use crate::shaderc::mesa_reference::SHC_BLOB_TABLE;

/// Every bit this operation accounts for: each field's span, plus the bits the
/// opcode match itself tested. Anything outside is a bit the decoder ignores --
/// which is exactly where an end-of-program flag would hide.
fn covered_bits(op: &super::isa::Op) -> u128 {
    let mut m = op.care;
    for f in op.fields {
        for b in f.low..=f.high {
            m |= 1u128 << b;
        }
    }
    m
}

fn as_u128(q: &[u32; 4]) -> u128 {
    (q[0] as u128) | (q[1] as u128) << 32 | (q[2] as u128) << 64 | (q[3] as u128) << 96
}

/// A blob's length and its instruction count are the same fact, so a program
/// can neither be cut short nor carry a word the count does not cover.
#[test]
fn every_blob_is_a_whole_number_of_instructions() {
    let mut n = 0usize;
    for blob in SHC_BLOB_TABLE.iter() {
        let dw = blob.shader.usc_dw as usize;
        assert!(dw.is_multiple_of(4), "{}: usc_dw {dw} is not a whole number of instructions", blob.source_name());
        n += 1;
    }
    assert!(n > 0, "no blob in the table: this test would pass vacuously");
    println!("{n} blobs, every usc_dw a whole number of instructions");
}

/// Accumulates the evidence for "this bit is an end-of-program flag".
///
/// The signature is **always set on a final instruction and never set
/// anywhere else** -- an intersection on one side and a union on the other.
/// My first version compared the union of last-instruction bits against the
/// union of the rest, which cannot fire: with thousands of mid-program
/// instructions the second union covers nearly every bit position, so
/// `last & !rest` is zero whatever the truth is. It passed identically with
/// the bit-coverage subtraction removed, which is what exposed it -- a test
/// that gives the same answer however you change it is not measuring.
#[derive(Clone, Copy)]
struct FlagEvidence {
    name: &'static str,
    /// Bits set on EVERY final instance of this operation. `None` until the
    /// first is seen, so that an intersection never starts from zero.
    always_last: Option<u128>,
    /// Bits set on ANY non-final instance.
    ever_rest: u128,
}

impl FlagEvidence {
    fn observe(&mut self, bits: u128, is_last: bool) {
        if is_last {
            self.always_last = Some(self.always_last.map_or(bits, |a| a & bits));
        } else {
            self.ever_rest |= bits;
        }
    }

    /// Bits that behave exactly as a flag would.
    fn suspect(&self) -> u128 {
        self.always_last.unwrap_or(0) & !self.ever_rest
    }
}

/// The detector finds a planted flag. Without this the corpus result below is
/// "no flag found", which is not the same claim as "there is no flag".
#[test]
fn the_flag_detector_finds_one_that_is_there() {
    let mut e = FlagEvidence { name: "planted", always_last: None, ever_rest: 0 };
    // Mid-program instances, varied, none carrying bit 40.
    e.observe(0x0000_0000_0000_00ff, false);
    e.observe(0x0000_00ff_ff00_0000, false);
    /* Final instances, differing from each other but both carrying bit 40.
     * Their other bits are drawn from what the mid-program instances already
     * showed, so bit 40 is the only candidate. My first attempt used
     * 0x1234 and 0x9876_5432, which share bit 12 -- a bit no mid-program
     * observation covered, so the detector correctly reported TWO suspects
     * and the assertion failed. The detector was right and the fixture was
     * wrong, which is the failure a positive control is for. */
    e.observe((1 << 40) | 0x00ff, true);
    e.observe((1 << 40) | 0x000f, true);
    assert_eq!(e.suspect(), 1 << 40, "the detector missed a bit set on every final instance and no other");

    /* And it does not cry wolf: every bit of this final instance was also
     * seen mid-program, so nothing is a candidate. (My first version left
     * 0x1234 on the final only, which makes those bits genuine suspects --
     * the detector said so and was right again. Twice in one test the
     * fixture was wrong and the logic was not, which is the argument for
     * writing the control at all rather than trusting the corpus result.) */
    let mut f = FlagEvidence { name: "innocent", always_last: None, ever_rest: 0 };
    f.observe((1 << 40) | 0x1234, false);
    f.observe((1 << 40) | 0x1234, true);
    assert_eq!(f.suspect(), 0, "the detector flagged a bit that occurs mid-program");
}

/// The load-bearing absence. If the reference set a bit on its final
/// instruction, a program of ours without it would run on -- so this looks
/// outside the named fields too, where a flag the decoder ignores would live.
#[test]
fn no_operation_carries_an_end_of_program_flag() {
    let mut ops: [FlagEvidence; 64] = [FlagEvidence { name: "", always_last: None, ever_rest: 0 }; 64];
    let mut k = 0usize;
    let mut instrs = 0usize;
    let mut finals = 0usize;
    for blob in SHC_BLOB_TABLE.iter() {
        let code = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
        let quads = code.as_chunks::<4>().0;
        let Some(n) = quads.len().checked_sub(1) else { continue };
        finals += 1;
        for (idx, q) in quads.iter().enumerate() {
            let Ok(i) = decode_instr(q) else { continue };
            instrs += 1;
            let extra = as_u128(q) & !covered_bits(i.op());
            let slot = ops[..k].iter().position(|e| e.name == i.op().name).unwrap_or_else(|| {
                assert!(k < ops.len(), "more operations than this table holds");
                ops[k].name = i.op().name;
                k += 1;
                k - 1
            });
            ops[slot].observe(extra, idx == n);
        }
    }
    assert!(instrs > 0 && finals > 0, "nothing decoded: this test would pass vacuously");
    let mut with_finals = 0usize;
    for e in &ops[..k] {
        if e.always_last.is_some() {
            with_finals += 1;
        }
        assert_eq!(
            e.suspect(),
            0,
            "{} sets {:#x} on every final instance and never elsewhere: a flag",
            e.name,
            e.suspect()
        );
    }
    assert!(with_finals > 0, "no operation was ever final: this test would pass vacuously");
    println!("{instrs} instructions over {finals} programs, {k} operations ({with_finals} ever final): no end-of-program flag");
}

/// The 15 nop-terminated blobs are not real code with a terminator appended:
/// each one *is* a single nop. Without this the test above could be read as
/// "the reference ends with a nop sometimes", which is a different claim.
#[test]
fn a_blob_ending_in_nop_is_a_one_instruction_program() {
    let mut nops = 0usize;
    for blob in SHC_BLOB_TABLE.iter() {
        let code = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
        let quads = code.as_chunks::<4>().0;
        let Some(tail) = quads.last() else { continue };
        let Ok(i) = decode_instr(tail) else { continue };
        if i.op().name != "nop" {
            continue;
        }
        nops += 1;
        assert_eq!(quads.len(), 1, "{}: ends in a nop after {} instructions", blob.source_name(), quads.len() - 1);
    }
    assert!(nops > 0, "no nop-terminated blob: this test would pass vacuously");
    println!("{nops} blobs end in a nop, every one of them a single-instruction program");
}
