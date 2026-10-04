// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Decoding a GC7000 shader binary into instructions.
//!
//! There is no framing to work out: a shader is a whole number of 128-bit
//! instructions and each one names its own operation in its pinned bits. The
//! only question is which operation, and the decoder answers it the same way
//! the PowerVR one does - by finding the operation whose pinned bits all hold
//! and refusing to guess if more than one does.

use super::isa::*;
use super::isa_tables::OPS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer is not a whole number of instructions.
    Truncated,
    /// No operation's pinned bits match.
    NoParse,
    /// More than one does; the decoder will not guess.
    Ambiguous,
}

/// Decode one 128-bit instruction.
pub fn decode_instr(dw: &[u32; 4]) -> Result<Instr, DecodeError> {
    let w = word_of(dw);
    let mut found: Option<usize> = None;
    for (i, op) in OPS.iter().enumerate() {
        if w & op.care != op.bits {
            continue;
        }
        if found.is_some() {
            return Err(DecodeError::Ambiguous);
        }
        found = Some(i);
    }
    let i = found.ok_or(DecodeError::NoParse)?;
    let op = &OPS[i];
    let mut instr = Instr { op: i as u16, vals: [0; MAX_FIELDS], dont_care: w & op.dont_care };
    for (n, f) in op.fields.iter().enumerate() {
        instr.vals[n] = read_field(w, f);
    }
    Ok(instr)
}

/// Walk a whole shader, calling `f` with each instruction and its index.
pub fn for_each_instr(
    code: &[u32],
    mut f: impl FnMut(usize, &Instr) -> Result<(), DecodeError>,
) -> Result<usize, DecodeError> {
    if !code.len().is_multiple_of(4) {
        return Err(DecodeError::Truncated);
    }
    for (i, dw) in code.as_chunks::<4>().0.iter().enumerate() {
        let quad = [dw[0], dw[1], dw[2], dw[3]];
        f(i, &decode_instr(&quad)?)?;
    }
    Ok(code.len() / 4)
}
