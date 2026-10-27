// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Choosing the encoding a group's operands fit in, and saying so when none
//! does.
//!
//! Selection says *what* to compute and the allocator says *where* the values
//! live. Neither knows that this machine encodes a source slot's register file
//! in one, two or three bits depending on the form of the section, so a group
//! that is perfectly well formed above this line can be unencodable below it:
//! a uniform in a slot whose variant can only name `special` and `temp` has
//! nowhere to go.
//!
//! That is what legalisation is for, and it is the reason the seam document
//! put the constraint on this side: selection states the operand and the
//! machine half takes the consequences. The consequence is either a wider
//! variant, or -- when even the widest cannot hold it -- an `mbyp` moving the
//! value into a file the slot can name.
//!
//! This file does the first half: pick the narrowest section that fits, and
//! name the slot that defeats every section so the caller can insert the move.

use super::isa::{bit_set, BitSetId, Region};
use super::operands::{dst_capability, src_capability, Operand, SLOT_BANK, SLOT_IDX};

/// Why no source section can hold these operands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoFit {
    /// This slot's operand fits no variant: the widest bank field is 3 bits
    /// and the widest index field is bounded too, so an operand outside both
    /// needs a move rather than a wider section.
    Slot { slot: usize, op: Operand },
    /// Operands were given for both halves at once. The two sections are
    /// chosen independently, so a caller must ask twice.
    BothHalves,
}

/// The narrowest source-section variant that can hold every operand given.
///
/// Operands are indexed by TRUE SLOT: 0..2 are the lower section's and 3..5
/// the upper's. A slot left `None` is one the group does not use, and a
/// variant that has more slots than are needed is still a candidate -- the
/// unused slots simply carry whatever the builder puts there -- so "narrowest"
/// means fewest bytes, not fewest slots.
///
/// `mux` is the section's `is0` field. Only some variants carry it -- the
/// `_Nm` family -- so a nonzero mux is a width requirement exactly as a wide
/// bank is: it rules out every variant without the field, however well the
/// registers would fit. A zero mux constrains nothing, because a variant
/// without the field encodes the same zero.
///
/// The re-encode gate measured this: ignoring `mux` left 4,682 of 416,991 of
/// the reference compiler's own groups wrong, 3,800 by taking a variant
/// narrower than pco's and 882 by matching the variant and dropping the value.
pub fn choose_src_variant(ops: &[Option<Operand>; 6], mux: u32) -> Result<u16, NoFit> {
    let lower = ops[0..3].iter().any(Option::is_some);
    let upper = ops[3..6].iter().any(Option::is_some);
    if lower && upper {
        return Err(NoFit::BothHalves);
    }
    let set = bit_set(BitSetId::Src);
    let want_upper = upper;

    let mut best: Option<(u16, u8)> = None;
    for (vi, v) in set.variants.iter().enumerate() {
        if (v.upper_src == Some(true)) != want_upper {
            continue;
        }
        let cap = src_capability(vi as u16);
        let fits = ops.iter().enumerate().all(|(slot, op)| match (op, cap[slot]) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(op), Some(c)) => c.holds(op),
        });
        if fits && holds_mux(vi as u16, mux) && best.is_none_or(|(_, bytes)| v.bytes < bytes) {
            best = Some((vi as u16, v.bytes));
        }
    }
    if let Some((vi, _)) = best {
        return Ok(vi);
    }

    /* Name the slot that defeated every variant, so the caller knows which
     * operand needs the move rather than being told only that something did. */
    for (slot, op) in ops.iter().enumerate() {
        let Some(op) = op else { continue };
        let any = set.variants.iter().enumerate().any(|(vi, v)| {
            (v.upper_src == Some(true)) == want_upper && src_capability(vi as u16)[slot].is_some_and(|c| c.holds(op))
        });
        if !any {
            return Err(NoFit::Slot { slot, op: *op });
        }
    }
    /* Every operand fits some variant but no single variant fits them all. */
    Err(NoFit::Slot { slot: 0, op: ops.iter().flatten().next().copied().unwrap_or(Operand { bank: 0, index: 0 }) })
}

/// Can this source-section variant carry this `is0` value?
///
/// A variant without the field carries only zero; one with it carries what the
/// field's width and transform allow, which `Region::set` decides rather than
/// this function guessing at it.
fn holds_mux(vi: u16, mux: u32) -> bool {
    mux == 0 || Region::new(BitSetId::Src, vi).set("is0", mux).is_ok()
}

/// The narrowest destination-section variant that can hold these writes.
///
/// `ops[1]` is `None` for the common single-destination group. The two-slot
/// forms can still be chosen for it if they happen to be narrower, which they
/// are not on this machine -- but the rule is "narrowest that fits" rather
/// than "the form with the matching slot count", because the encoding decides
/// that, not this file's idea of tidiness.
pub fn choose_dst_variant(ops: &[Option<Operand>; 2]) -> Result<u16, NoFit> {
    let set = bit_set(BitSetId::Dst);
    let mut best: Option<(u16, u8)> = None;
    for (vi, v) in set.variants.iter().enumerate() {
        let cap = dst_capability(vi as u16);
        let fits = ops.iter().enumerate().all(|(slot, op)| match (op, cap[slot]) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(op), Some(c)) => c.holds(op),
        });
        if fits && best.is_none_or(|(_, bytes)| v.bytes < bytes) {
            best = Some((vi as u16, v.bytes));
        }
    }
    if let Some((vi, _)) = best {
        return Ok(vi);
    }
    for (slot, op) in ops.iter().enumerate() {
        let Some(op) = op else { continue };
        let any =
            set.variants.iter().enumerate().any(|(vi, _)| dst_capability(vi as u16)[slot].is_some_and(|c| c.holds(op)));
        if !any {
            return Err(NoFit::Slot { slot, op: *op });
        }
    }
    Err(NoFit::Slot { slot: 0, op: ops.iter().flatten().next().copied().unwrap_or(Operand { bank: 0, index: 0 }) })
}

/// Write operands into a chosen source section.
///
/// The counterpart of [`super::operands::srcs`], which reads them out. The
/// variant must be one [`choose_src_variant`] returned for these operands, so
/// every `set` here is expected to succeed; a failure means the capability
/// model and the field widths disagree, which is this file's bug rather than
/// the caller's, and it surfaces as `NoFit::Slot` rather than a wrong group.
pub fn fill_srcs(r: &mut Region, ops: &[Option<Operand>; 6], mux: u32) -> Result<(), NoFit> {
    /* Zero is the default every variant encodes, so it is written only when it
     * is asked for: setting it unconditionally would refuse the narrow
     * variants that do not carry the field at all. */
    if mux != 0 && r.set("is0", mux).is_err() {
        return Err(NoFit::Slot { slot: 0, op: ops[0].unwrap_or(Operand { bank: 0, index: 0 }) });
    }
    let base = if r.variant().upper_src == Some(true) { 3 } else { 0 };
    for i in 0..3 {
        let slot = base + i;
        let Some(op) = ops[slot] else { continue };
        let (bn, sn) = (SLOT_BANK[slot], SLOT_IDX[slot]);
        if r.set(bn, op.bank).is_err() || r.set(sn, op.index).is_err() {
            return Err(NoFit::Slot { slot, op });
        }
    }
    Ok(())
}

/// Write operands into a chosen destination section.
pub fn fill_dsts(r: &mut Region, ops: &[Option<Operand>; 2]) -> Result<(), NoFit> {
    /* The single-destination forms spell it dN/dbN and the pair d0/d1; try the
     * pair first because a variant carrying d0 never also carries dN. */
    for (slot, (bn, dn)) in [("db0", "d0"), ("db1", "d1")].into_iter().enumerate() {
        let Some(op) = ops[slot] else { continue };
        if r.set(bn, op.bank).is_ok() && r.set(dn, op.index).is_ok() {
            continue;
        }
        if slot == 0 && r.set("dbN", op.bank).is_ok() && r.set("dN", op.index).is_ok() {
            continue;
        }
        return Err(NoFit::Slot { slot, op });
    }
    Ok(())
}
