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
use super::sel::{Bank, Mods, SelDst, SelGroup, SelOp, SelSrc};

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

/// The files a uniform-block load's operand may name, by slot
/// (`docs/notes/pvr-ubo-memory-load.md` §2): what [`choose_src_variant`]
/// encodes for `add64_32` with `is0 = s0` (the mux
/// [`super::assemble::build_add64_32`] sets), narrowed to what the load
/// needs.
///
/// - [`SelOp::Add64`] `srcs[0]`, the address's low dword: a temporary, the
///   shared pair's low register, that register read through `idx0` past
///   sh255 (`prepare::index_high_shared`), or a constant register.
/// - `srcs[1]`, the high dword: a temporary or a constant register only.
///   Slot `s1` has no shared or `idx0` form in any source section, so the
///   shared pair's high register is moved to a temporary first (pco
///   legalises the same source with a move; `tests/usc_exec_ubo_seq.rs`).
/// - `srcs[2]`, the 32-bit byte offset: any of the files `srcs[0]` takes.
///   An immediate is made a constant register or a move first, as for any
///   operand.
/// - [`SelOp::Ld`] `srcs[0]`, the address pair: a temporary pair only. pco
///   always adds into temporaries before an `ld` (§2: whether the silicon
///   takes a shared pair as the address is not known), so this does too.
///
/// Empty for any other operation or slot.
pub fn mem_src_banks(op: SelOp, slot: usize) -> &'static [Bank] {
    match (op, slot) {
        (SelOp::Add64, 0 | 2) => &[Bank::Temp, Bank::Shared, Bank::Idx0, Bank::Special],
        (SelOp::Add64, 1) => &[Bank::Temp, Bank::Special],
        (SelOp::Ld { .. }, 0) => &[Bank::Temp],
        _ => &[],
    }
}

/// Why a uniform-block load group is not one the builder takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemRule {
    /// Operand `slot` is missing, carries a modifier, is an immediate, names
    /// a file [`mem_src_banks`] does not allow there, or is one the
    /// operation does not read.
    Operand { slot: usize },
    /// The destination is missing or not a temporary.
    Destination,
    /// An `ld` burst outside 1..=[`super::sel::MAX_LD_BURST`].
    Burst,
    /// The group is not an [`SelOp::Ld`] or an [`SelOp::Add64`].
    NotAMemOp,
}

/// Is `g`, a uniform-block load group, legal as it stands: every operand a
/// plain register in a file [`mem_src_banks`] allows (a virtual one will be
/// a temporary), and the destination a temporary run? `prepare` makes it so;
/// `assemble::build_group` refuses what is not.
pub fn check_mem_op(g: &SelGroup) -> Result<(), MemRule> {
    let n = match g.op {
        SelOp::Ld { burst } if (1..=super::sel::MAX_LD_BURST).contains(&burst) => 1,
        SelOp::Ld { .. } => return Err(MemRule::Burst),
        SelOp::Add64 => 3,
        _ => return Err(MemRule::NotAMemOp),
    };
    for (slot, s) in g.srcs.iter().enumerate() {
        let ok = match *s {
            _ if slot >= n => s.is_none(),
            Some(SelSrc::Virtual { mods, .. }) => mods == Mods::default(),
            Some(SelSrc::Fixed { bank, mods, .. }) => {
                mods == Mods::default() && mem_src_banks(g.op, slot).contains(&bank)
            }
            Some(SelSrc::Imm(_)) | None => false,
        };
        if !ok {
            return Err(MemRule::Operand { slot });
        }
    }
    match g.dst {
        Some(SelDst::Virtual(_)) | Some(SelDst::Fixed { bank: Bank::Temp, .. }) => Ok(()),
        _ => Err(MemRule::Destination),
    }
}

#[cfg(test)]
mod tests {
    use super::super::sel::{OpType, VReg};
    use super::*;

    fn is0_s0() -> u32 {
        super::super::isa_tables::ENUMS
            .iter()
            .find(|e| e.name == "is0_sel")
            .and_then(|e| e.elems.iter().find(|(_, n)| *n == "s0"))
            .map(|(v, _)| *v)
            .expect("is0_sel s0")
    }

    /// [`mem_src_banks`] for `add64_32` is what the source sections encode:
    /// each allowed file fits slot k with temporaries in the other two, and
    /// the shared and `idx0` files fit slot 1 in no form, which is why the
    /// high dword is moved.
    #[test]
    fn the_add64_banks_are_the_ones_the_source_forms_hold() {
        let t = |i| Some(Operand { bank: Bank::Temp.hw(), index: i });
        for slot in 0..3 {
            for bank in [Bank::Temp, Bank::Shared, Bank::Idx0, Bank::Special, Bank::VtxIn, Bank::Coeff] {
                let mut ops = [t(0), t(1), t(2), None, None, None];
                ops[slot] = Some(Operand { bank: bank.hw(), index: 200 });
                let fits = choose_src_variant(&ops, is0_s0()).is_ok();
                let allowed = mem_src_banks(SelOp::Add64, slot).contains(&bank);
                /* the rule may be narrower than the encoding, never wider */
                assert!(!allowed || fits, "slot {slot} {bank:?} is allowed and does not fit");
                if matches!(bank, Bank::Shared | Bank::Idx0) {
                    assert_eq!(fits, slot != 1, "slot {slot} {bank:?}");
                }
            }
        }
    }

    #[test]
    fn a_mem_op_with_a_shared_high_dword_or_a_shared_address_is_refused() {
        let v = |n| Some(SelSrc::Virtual { v: VReg(n), mods: Mods::default() });
        let sh = |r| Some(SelSrc::Fixed { bank: Bank::Shared, reg: r, mods: Mods::default() });
        let add = SelGroup {
            op: SelOp::Add64,
            ty: OpType::U32,
            dst: Some(SelDst::Virtual(VReg(9))),
            srcs: [sh(8), v(1), v(2)],
        };
        assert_eq!(check_mem_op(&add), Ok(()));
        assert_eq!(check_mem_op(&SelGroup { srcs: [sh(8), sh(9), v(2)], ..add }), Err(MemRule::Operand { slot: 1 }));
        assert_eq!(
            check_mem_op(&SelGroup { srcs: [sh(8), v(1), Some(SelSrc::Imm(4))], ..add }),
            Err(MemRule::Operand { slot: 2 })
        );
        let neg = Some(SelSrc::Virtual { v: VReg(2), mods: Mods { neg: true, ..Mods::default() } });
        assert_eq!(check_mem_op(&SelGroup { srcs: [sh(8), v(1), neg], ..add }), Err(MemRule::Operand { slot: 2 }));
        assert_eq!(
            check_mem_op(&SelGroup { dst: Some(SelDst::Fixed { bank: Bank::Shared, reg: 0 }), ..add }),
            Err(MemRule::Destination)
        );
        let ld = SelGroup {
            op: SelOp::Ld { burst: 4 },
            ty: OpType::U32,
            dst: Some(SelDst::Virtual(VReg(20))),
            srcs: [v(9), None, None],
        };
        assert_eq!(check_mem_op(&ld), Ok(()));
        assert_eq!(check_mem_op(&SelGroup { srcs: [sh(8), None, None], ..ld }), Err(MemRule::Operand { slot: 0 }));
        assert_eq!(check_mem_op(&SelGroup { srcs: [v(9), v(1), None], ..ld }), Err(MemRule::Operand { slot: 1 }));
        assert_eq!(check_mem_op(&SelGroup { op: SelOp::Ld { burst: 5 }, ..ld }), Err(MemRule::Burst));
        assert_eq!(check_mem_op(&SelGroup { op: SelOp::Ld { burst: 0 }, ..ld }), Err(MemRule::Burst));
        assert_eq!(check_mem_op(&SelGroup { op: SelOp::Copy, ..ld }), Err(MemRule::NotAMemOp));
    }
}
