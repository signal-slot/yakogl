// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Writing decoded instruction groups back out as USC binary.
//!
//! The order is the one Mesa's `pco_encode_igrp` writes: the header, then the
//! instructions in reverse phase order, then the lower and upper source
//! sections, the ISS byte and the destination section, and last the padding.
//! Every field of a variant is written - fixed, reserved and settable alike -
//! and a variant's fields tile its bytes exactly, so a group that came from
//! [`super::decode`] comes back byte for byte.

use super::decode::{DecodeError, Group, MAX_INSTRS};
use super::isa::encode_region;

/// Write one group into `out[off..]`. Returns the bytes written, which is the
/// length the group's own header states.
pub fn encode_group(g: &Group, out: &mut [u8], off: usize) -> Result<usize, DecodeError> {
    if off + g.total > out.len() {
        return Err(DecodeError::Truncated);
    }
    let mut p = off;
    p += encode_region(&g.hdr, out, p);

    // Instructions are written highest phase first.
    for slot in (0..MAX_INSTRS).rev() {
        if let Some((_, r)) = &g.instrs[slot] {
            p += encode_region(r, out, p);
        }
    }
    for r in [&g.lower, &g.upper, &g.iss, &g.dst].into_iter().flatten() {
        p += encode_region(r, out, p);
    }
    if g.word_pad != 0 {
        out[p] = 0xff;
        p += 1;
    }
    if g.align_pad != 0 {
        out[p] = 0xf0 | (g.align_pad / 2);
        p += 1;
        for b in &mut out[p..off + g.total] {
            *b = 0xff;
        }
        p = off + g.total;
    }
    if p != off + g.total {
        return Err(DecodeError::NoParse);
    }
    Ok(g.total)
}
