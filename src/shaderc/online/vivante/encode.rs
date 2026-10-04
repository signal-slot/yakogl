// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Writing decoded GC7000 instructions back out.
//!
//! An instruction is built from nothing: start at zero, set the bits the
//! operation pins, then write every field. Nothing of the original word is
//! carried over, so a bit no field and no pattern covers would come back
//! zero - which is exactly what makes the round trip a test of the tables.

use super::decode::DecodeError;
use super::isa::*;

/// Write one instruction as four little-endian dwords.
pub fn encode_instr(instr: &Instr) -> [u32; 4] {
    let op = instr.op();
    let mut w: u128 = (op.bits & op.care) | (instr.dont_care & op.dont_care);
    for (n, f) in op.fields.iter().enumerate() {
        write_field(&mut w, f, instr.vals[n]);
    }
    dwords_of(w)
}

/// Write a sequence of instructions into `out`, which must have room.
pub fn encode(instrs: &[Instr], out: &mut [u32]) -> Result<usize, DecodeError> {
    if out.len() < instrs.len() * 4 {
        return Err(DecodeError::Truncated);
    }
    for (i, instr) in instrs.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&encode_instr(instr));
    }
    Ok(instrs.len() * 4)
}
