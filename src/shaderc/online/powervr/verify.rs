// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Reading a built group back and checking it says what was meant.
//!
//! The builder's own "decode back" used to compare a decoded register file
//! with the expected one translated by the same cast the builder encoded it
//! with, so a wrong mapping agreed with itself: 29 of 29 groups "decoded
//! back" while every destination was a special register (usc-backend,
//! 2026-09-23). Everything here goes the other way round: the decoded bytes
//! are named through the generated ISA table's own enumerations, and
//! compared with what the [`SelGroup`] says in the selection seam's own
//! words. Nothing the builder computed is reused.
//!
//! Two questions, kept apart:
//!
//! * [`check_group`] / [`check_write`]: do the bytes mean the operation,
//!   operands and modifiers that were asked for?
//! * [`skeleton`]: everything a group encodes EXCEPT its registers and
//!   literal values, as text. The gate asks whether pco ever emitted a group
//!   with the same skeleton; a group whose wiring (ISS, header flags,
//!   operation fields) no reference group shares is a guess, however well it
//!   decodes.

use super::decode::{decode_group, Group};
use super::isa::{bit_set, ops_for, Region};
use super::isa_tables::ENUMS;
use super::operands::{dsts, srcs, Operand};
use super::sel::{MachineOp, Mods, SelDst, SelGroup, SelOp, SelSrc};
use core::fmt::Write;

/// What differed, named once per cause.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mismatch {
    DoesNotDecode,
    /// The group holds other operations than the one meant.
    Operation {
        got: &'static str,
        want: &'static str,
    },
    MoreThanOneOperation,
    Destination {
        slot: u8,
    },
    Source {
        slot: u8,
    },
    Modifier {
        slot: u8,
    },
    Immediate,
    Address,
    Repeat,
    /// The meaning names something this checker cannot state (an unresolved
    /// virtual register), so it cannot say yes.
    Unresolved,
}

fn bank_name(v: u32) -> Option<&'static str> {
    ENUMS.iter().find(|e| e.name == "regbank").and_then(|e| e.name_of(v))
}

/// Does decoded `op` name the register `bank`/`reg`, by the table's name?
fn same(op: Option<Operand>, bank: super::sel::Bank, reg: u16) -> bool {
    op.is_some_and(|o| bank_name(o.bank) == Some(bank.isa_name()) && o.index == reg as u32)
}

fn field_or_zero(r: &Region, name: &str) -> u32 {
    r.get(name).unwrap_or(0)
}

/// A resolved sample (`chain::expand_samples`): `smp` with the image
/// descriptor in s0, the data block in s1 and the sampler state in s2 (as
/// `srcs` name them), the result block in s4, special 0 in s3, and the
/// extended form's flags as the sample asks -- `proj`, the level replaced
/// or biased per pixel (`lodm`, `pplod`), `nncoords` for a fetch, `soo`
/// for an offset, `sno` for a multisampled image's sample, `tao` for a 2D
/// array's layer address. It writes through s4, so the destination region
/// holds nothing.
fn check_smp(
    meant: &SelGroup,
    (proj, lod, fetch, offset, layer, gather, ms): (bool, super::sel::SmpLod, bool, bool, bool, bool, bool),
    g: &Group,
) -> Result<usize, Mismatch> {
    use super::sel::Bank;
    let (op, r) = only_op(g)?;
    if op != "smp" {
        return Err(Mismatch::Operation { got: op, want: "smp" });
    }
    let fixed = |s: Option<SelSrc>| match s {
        Some(SelSrc::Fixed { bank, reg, mods }) if mods == Mods::default() => Ok((bank, reg)),
        _ => Err(Mismatch::Unresolved),
    };
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
    for (slot, s) in [(0u8, meant.srcs[0]), (1, meant.srcs[1]), (2, meant.srcs[2])] {
        let (bank, reg) = fixed(s)?;
        if !same(lo[slot as usize], bank, reg) {
            return Err(Mismatch::Source { slot });
        }
    }
    if !same(up[3], Bank::Special, 0) {
        return Err(Mismatch::Source { slot: 3 });
    }
    match meant.dst {
        Some(SelDst::Fixed { bank, reg }) if same(up[4], bank, reg) => {}
        Some(SelDst::Virtual(_)) => return Err(Mismatch::Unresolved),
        _ => return Err(Mismatch::Destination { slot: 0 }),
    }
    if g.dst.as_ref().map(dsts).is_some_and(|d| d.iter().any(|x| x.is_some())) {
        return Err(Mismatch::Destination { slot: 0 });
    }
    let lodm = match lod {
        super::sel::SmpLod::Implicit => "normal",
        super::sel::SmpLod::Replace => "replace",
        super::sel::SmpLod::Bias => "bias",
        super::sel::SmpLod::Gradients => "gradients",
    };
    let lodm = super::shapes::value(super::isa::BitSetId::Backend, "lodm", lodm).map_err(|_| Mismatch::Unresolved)?;
    let rawdata =
        super::shapes::value(super::isa::BitSetId::Backend, "sbmode", "rawdata").map_err(|_| Mismatch::Unresolved)?;
    let flags = [
        ("proj", u32::from(proj)),
        ("pplod", u32::from(lod.in_block())),
        ("nncoords", u32::from(fetch)),
        ("soo", u32::from(offset)),
        ("sno", u32::from(ms)),
        ("tao", u32::from(layer)),
        ("sbmode", if gather { rawdata } else { 0 }),
        ("chan", if gather { 1 } else { 4 }),
        ("lodm", lodm),
    ];
    for (f, want) in flags {
        if field_or_zero(r, f) != want {
            return Err(Mismatch::Operation { got: "smp", want: "smp with the sample's flags" });
        }
    }
    Ok(g.total)
}

/// The single operation a group holds.
fn only_op(g: &Group) -> Result<(&'static str, &Region), Mismatch> {
    let mut it = g.instrs();
    let (_, phase, r) = it.next().ok_or(Mismatch::Operation { got: "<none>", want: "?" })?;
    if it.next().is_some() {
        return Err(Mismatch::MoreThanOneOperation);
    }
    let op = ops_for(r, phase).next().map(|o| o.name).unwrap_or("<unknown>");
    Ok((op, r))
}

/// An execution-mask group (`powervr::cf`) read back: the operation and the
/// fields `assemble` builds for it (cndst 1 / cndst 2 / cndsm p0_false by
/// special 2 / cndlt 2 for a loop; cndst 1 p0_false / cndef 1 /
/// cndend 1 for a branched `if`), the emc's temporary as its first source and its
/// destination (special 0 as the source of the set-up), and the group run in
/// every instance (`cc` ignorepe).
fn check_cnd(meant: &SelGroup, m: MachineOp, g: &Group) -> Result<usize, Mismatch> {
    let (op, r) = only_op(g)?;
    let (want, adjust, pcnd) = match m {
        MachineOp::CndInit => ("cndst", 1, "always"),
        MachineOp::CndEnter => ("cndst", 2, "always"),
        MachineOp::CndLeave => ("cndsm", 0, "p0_false"),
        MachineOp::CndIf | MachineOp::CndIfMasked => ("cndst", 1, "p0_false"),
        MachineOp::CndElse => ("cndef", 1, "always"),
        MachineOp::CndIfEnd => ("cndend", 1, "always"),
        _ => ("cndlt", 2, "always"),
    };
    if op != want {
        return Err(Mismatch::Operation { got: op, want });
    }
    let pcnd = super::shapes::value(super::isa::BitSetId::Ctrl, "pcnd", pcnd).map_err(|_| Mismatch::Unresolved)?;
    let ignorepe =
        super::shapes::value(super::isa::BitSetId::IgrpHdr, "cc", "(ignorepe)").map_err(|_| Mismatch::Unresolved)?;
    if field_or_zero(r, "adjust") != adjust
        || field_or_zero(r, "pcnd") != pcnd
        || field_or_zero(&g.hdr, "cc") != ignorepe
    {
        return Err(Mismatch::Operation { got: op, want: "the execution mask's fields" });
    }
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    match meant.srcs[0] {
        Some(SelSrc::Fixed { bank, reg, .. }) if same(lo[0], bank, reg) => {}
        Some(SelSrc::Virtual { .. }) => return Err(Mismatch::Unresolved),
        _ => return Err(Mismatch::Source { slot: 0 }),
    }
    if m == MachineOp::CndLeave
        && !(same(lo[1], super::sel::Bank::Special, 0) && same(lo[2], super::sel::Bank::Special, 2))
    {
        return Err(Mismatch::Source { slot: 2 });
    }
    let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
    match meant.dst {
        Some(SelDst::Fixed { bank, reg }) if same(d[0], bank, reg) => {}
        Some(SelDst::Virtual(_)) => return Err(Mismatch::Unresolved),
        _ => return Err(Mismatch::Destination { slot: 0 }),
    }
    Ok(g.total)
}

/// A resolved [`SelOp::Ld`] read back: `ld` in form `ld_immbl` (pco's
/// `group_map(O_LD)`), the burst asked for, DRC 0, cache mode normal, the
/// address pair's first register in s0 (`srcseladd` s0), the destination
/// run's first register in s3, no destination section (the backend writes
/// it itself) and no write port.
fn check_ld(meant: &SelGroup, burst: u8, g: &Group) -> Result<usize, Mismatch> {
    let (op, r) = only_op(g)?;
    if op != "ld" || r.variant().name != "ld_immbl" {
        return Err(Mismatch::Operation { got: op, want: "ld" });
    }
    let normal = super::shapes::value(super::isa::BitSetId::Backend, "cachemode_ld", "normal")
        .map_err(|_| Mismatch::Unresolved)?;
    if r.get("burstlen") != Some(u32::from(burst))
        || field_or_zero(r, "drc") != 0
        || field_or_zero(r, "srcseladd") != 0
        || field_or_zero(r, "cachemode_ld") != normal
    {
        return Err(Mismatch::Operation { got: op, want: "ld with the load's fields" });
    }
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
    match meant.srcs[0] {
        Some(SelSrc::Fixed { bank, reg, mods }) if mods == Mods::default() && same(lo[0], bank, reg) => {}
        Some(SelSrc::Virtual { .. }) => return Err(Mismatch::Unresolved),
        _ => return Err(Mismatch::Source { slot: 0 }),
    }
    match meant.dst {
        Some(SelDst::Fixed { bank, reg }) if same(up[3], bank, reg) => {}
        Some(SelDst::Virtual(_)) => return Err(Mismatch::Unresolved),
        _ => return Err(Mismatch::Destination { slot: 0 }),
    }
    if g.dst.is_some() || field_or_zero(&g.hdr, "w0p") != 0 || field_or_zero(&g.hdr, "w1p") != 0 {
        return Err(Mismatch::Destination { slot: 0 });
    }
    Ok(g.total)
}

/// A resolved [`SelOp::Add64`] read back: `add64_32` with `.s`, the low
/// dword, high dword and offset in s0, s1 and s2, and the pair written
/// through w0 and w1 to `dst`, `dst + 1`.
fn check_add64(meant: &SelGroup, g: &Group) -> Result<usize, Mismatch> {
    let (op, r) = only_op(g)?;
    if op != "add64_32" {
        return Err(Mismatch::Operation { got: op, want: "add64_32" });
    }
    if field_or_zero(r, "s") != 1 {
        return Err(Mismatch::Operation { got: op, want: "add64_32.s" });
    }
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    for (slot, s) in meant.srcs.iter().enumerate() {
        match s {
            Some(SelSrc::Fixed { bank, reg, mods }) if *mods == Mods::default() && same(lo[slot], *bank, *reg) => {}
            Some(SelSrc::Virtual { .. }) => return Err(Mismatch::Unresolved),
            _ => return Err(Mismatch::Source { slot: slot as u8 }),
        }
    }
    let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
    match meant.dst {
        Some(SelDst::Fixed { bank, reg }) => {
            if !same(d[0], bank, reg) {
                return Err(Mismatch::Destination { slot: 0 });
            }
            if !same(d[1], bank, reg + 1) {
                return Err(Mismatch::Destination { slot: 1 });
            }
        }
        Some(SelDst::Virtual(_)) => return Err(Mismatch::Unresolved),
        None => return Err(Mismatch::Destination { slot: 0 }),
    }
    Ok(g.total)
}

/// Why a program's `ld`s and waits are out of order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LdWait {
    /// Group `at` reads or writes a register of the `ld` at `ld` -- its
    /// destination run or its address pair -- before the `wdf` that makes
    /// the data land.
    Early { ld: usize, at: usize },
    /// The `ld` at `ld` reaches a loop or `if` edge, or the end of the
    /// program, with no `wdf` after it.
    NoWait { ld: usize },
    /// More `ld`s waiting on one `wdf` than this check tracks.
    TooMany { at: usize },
}

/// The first register of a run a pending `ld` holds: a virtual value or a
/// temporary (the run's width travels beside it).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    V(u32),
    T(u32),
}

fn held_of(s: &SelSrc) -> Option<Held> {
    match *s {
        SelSrc::Virtual { v, .. } => Some(Held::V(v.0)),
        SelSrc::Fixed { bank: super::sel::Bank::Temp, reg, .. } => Some(Held::T(u32::from(reg))),
        _ => None,
    }
}

fn held_dst(d: &SelDst) -> Option<Held> {
    match *d {
        SelDst::Virtual(v) => Some(Held::V(v.0)),
        SelDst::Fixed { bank: super::sel::Bank::Temp, reg } => Some(Held::T(u32::from(reg))),
        SelDst::Fixed { .. } => None,
    }
}

fn overlaps((a, an): (Held, u32), (b, bn): (Held, u32)) -> bool {
    match (a, b) {
        (Held::V(x), Held::V(y)) | (Held::T(x), Held::T(y)) => x < y + bn && y < x + an,
        _ => false,
    }
}

/// The rule `docs/notes/pvr-ubo-memory-load.md` §4 sets for the allocator,
/// checked on a program before or after allocation: an [`SelOp::Ld`]'s
/// destination counts as written at the next [`SelOp::WaitData`], not at
/// the `ld`. Between the two, no group reads or writes the destination run
/// or the address pair the `ld` reads, and no loop or `if` edge (a
/// [`SelOp::LoopBegin`], [`SelOp::LoopEnd`] or [`SelOp::Pred`]) and no end
/// of program comes first. pco puts the `wdf` right after each `ld`
/// (`pco_schedule.c`), and so does the sequence the selector emits; this
/// holds every other order to the same rule.
pub fn check_ld_waits(groups: &[SelGroup]) -> Result<(), LdWait> {
    const MAX_PENDING: usize = 8;
    /* (the ld's index, its register run) */
    let mut pending: [Option<(usize, (Held, u32))>; 2 * MAX_PENDING] = [None; 2 * MAX_PENDING];
    for (at, g) in groups.iter().enumerate() {
        match g.op {
            SelOp::WaitData => {
                pending = [None; 2 * MAX_PENDING];
                continue;
            }
            SelOp::LoopBegin | SelOp::LoopEnd | SelOp::Pred(_) => {
                if let Some((ld, _)) = pending.iter().flatten().next() {
                    return Err(LdWait::NoWait { ld: *ld });
                }
                continue;
            }
            _ => {}
        }
        let mut touched = g.dst.as_ref().and_then(held_dst).map(|h| (h, g.op.dst_width())).into_iter().chain(
            g.srcs
                .iter()
                .enumerate()
                .filter_map(|(slot, s)| s.as_ref().and_then(held_of).map(|h| (h, g.op.src_width(slot)))),
        );
        if let Some((ld, _)) = touched.find_map(|t| pending.iter().flatten().find(|(_, p)| overlaps(t, *p)).copied()) {
            return Err(LdWait::Early { ld, at });
        }
        if let SelOp::Ld { .. } = g.op {
            let regs = [
                g.dst.as_ref().and_then(held_dst).map(|h| (h, g.op.dst_width())),
                g.srcs[0].as_ref().and_then(held_of).map(|h| (h, g.op.src_width(0))),
            ];
            for r in regs.into_iter().flatten() {
                let slot = pending.iter_mut().find(|p| p.is_none()).ok_or(LdWait::TooMany { at })?;
                *slot = Some((at, r));
            }
        }
    }
    match pending.iter().flatten().next() {
        Some((ld, _)) => Err(LdWait::NoWait { ld: *ld }),
        None => Ok(()),
    }
}

/// Check one built group against the resolved [`SelGroup`] it was built
/// from. `want_op` is the operation the builder was to emit (the shape's
/// single step, or `mbyp` for a copy). Returns the group's length.
pub fn check_group(meant: &SelGroup, want_op: &'static str, bytes: &[u8]) -> Result<usize, Mismatch> {
    let g = decode_group(bytes, 0).map_err(|_| Mismatch::DoesNotDecode)?;
    if let SelOp::Sample { proj, lod, fetch, offset, layer, gather, ms, .. } = meant.op {
        return check_smp(meant, (proj, lod, fetch, offset.is_some(), layer, gather, ms), &g);
    }
    if let SelOp::Machine(m) = meant.op {
        if m.is_cnd() {
            return check_cnd(meant, m, &g);
        }
    }
    match meant.op {
        SelOp::Ld { burst } => return check_ld(meant, burst, &g),
        SelOp::Add64 => return check_add64(meant, &g),
        _ => {}
    }
    if let Some(t) = super::shapes::template_for(meant.op) {
        return check_template(t, meant, &g);
    }
    let (op, r) = only_op(&g)?;

    let (want_op, imm) = match (meant.op, meant.srcs[0]) {
        (SelOp::Copy, Some(SelSrc::Imm(v))) => ("bbyp0bm_imm32", Some(v)),
        _ => (want_op, None),
    };
    if op != want_op {
        return Err(Mismatch::Operation { got: op, want: want_op });
    }

    let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
    match meant.dst {
        Some(SelDst::Fixed { bank, reg }) => {
            if !same(d[0], bank, reg) {
                return Err(Mismatch::Destination { slot: 0 });
            }
        }
        Some(SelDst::Virtual(_)) => return Err(Mismatch::Unresolved),
        None if d[0].is_some() => return Err(Mismatch::Destination { slot: 0 }),
        None => {}
    }
    if d[1].is_some() {
        return Err(Mismatch::Destination { slot: 1 });
    }

    if let Some(v) = imm {
        if r.get("imm32") != Some(v) {
            return Err(Mismatch::Immediate);
        }
        return Ok(g.total);
    }

    let s = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let nm = ["s0", "s1", "s2"];
    for i in 0..meant.op.arity() {
        let Some(want) = meant.srcs[i] else { return Err(Mismatch::Source { slot: i as u8 }) };
        let mods = match want {
            SelSrc::Fixed { bank, reg, mods } => {
                if !same(s[i], bank, reg) {
                    return Err(Mismatch::Source { slot: i as u8 });
                }
                mods
            }
            SelSrc::Virtual { .. } => return Err(Mismatch::Unresolved),
            SelSrc::Imm(_) => return Err(Mismatch::Source { slot: i as u8 }),
        };
        let got = Mods {
            neg: field_or_zero(r, neg_name(nm[i])) != 0,
            abs: field_or_zero(r, abs_name(nm[i])) != 0,
            flr: field_or_zero(r, flr_name(nm[i])) != 0,
        };
        if got != mods {
            return Err(Mismatch::Modifier { slot: i as u8 });
        }
    }
    /* A modifier on a slot the operation does not read would change nothing,
     * but a set one means the builder and this checker disagree about which
     * slot is which. */
    for (i, n) in nm.iter().enumerate().skip(meant.op.arity()) {
        if field_or_zero(r, neg_name(n)) | field_or_zero(r, abs_name(n)) | field_or_zero(r, flr_name(n)) != 0 {
            return Err(Mismatch::Modifier { slot: i as u8 });
        }
    }
    Ok(g.total)
}

/* The modifier field names, spelled out rather than formatted so this file
 * needs no allocator. */
fn neg_name(s: &str) -> &'static str {
    match s {
        "s0" => "s0neg",
        "s1" => "s1neg",
        _ => "s2neg",
    }
}
fn abs_name(s: &str) -> &'static str {
    match s {
        "s0" => "s0abs",
        "s1" => "s1abs",
        _ => "s2abs",
    }
}
fn flr_name(s: &str) -> &'static str {
    match s {
        "s0" => "s0flr",
        "s1" => "s1flr",
        _ => "s2flr",
    }
}

/// Check one epilogue group: `uvsw.write` of `rpt` consecutive registers
/// starting at `first`, landing at `first.addr`. Returns the group's length.
pub fn check_write(first: super::assemble::OutWrite, rpt: u32, bytes: &[u8]) -> Result<usize, Mismatch> {
    let g = decode_group(bytes, 0).map_err(|_| Mismatch::DoesNotDecode)?;
    let (op, r) = only_op(&g)?;
    if op != "uvsw.write" {
        return Err(Mismatch::Operation { got: op, want: "uvsw.write" });
    }
    if r.get("imm_addr") != Some(first.addr as u32) {
        return Err(Mismatch::Address);
    }
    if g.hdr.get("rpt") != Some(rpt) {
        return Err(Mismatch::Repeat);
    }
    let s = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    if !same(s[0], first.bank, first.reg) {
        return Err(Mismatch::Source { slot: 0 });
    }
    Ok(g.total)
}

/// Fields that are operands or literals, not wiring: left out of the
/// skeleton so that two groups differing only in which registers and which
/// value they use share one.
fn is_operand_field(name: &str) -> bool {
    matches!(name, "imm32" | "imm_addr" | "length" | "da")
        || name.starts_with("sb")
        || name.starts_with("db")
        || (name.len() >= 2
            && (name.starts_with('s') || name.starts_with('d'))
            && name[1..].bytes().all(|b| b.is_ascii_digit()))
        || name == "dN"
}

fn region_skeleton(w: &mut impl Write, label: &str, r: &Region, with_variant: bool) -> core::fmt::Result {
    write!(w, " {label}")?;
    if with_variant {
        write!(w, ":{}", r.variant().name)?;
    }
    let set = bit_set(r.set);
    /* An iterator's destination, coefficient and count are operands, like
     * a source's register number: which temporaries and which varying. */
    let iter_operand = |n: &str| r.variant().name == "ditr" && matches!(n, "dest" | "coff" | "count");
    /* A branch's offset is a literal: where the target is, not what the
     * branch is. (The byte model follows it.) */
    let branch_operand = |n: &str| r.variant().name == "branch" && n == "offset";
    for (i, u) in r.variant().uses.iter().enumerate() {
        if u.fixed.is_some()
            || u.name.is_empty()
            || is_operand_field(u.name)
            || iter_operand(u.name)
            || branch_operand(u.name)
        {
            continue;
        }
        /* A source section's mux at `s0` wires what a section without the
         * field wires: which form carries the field is a question of the
         * registers' widths, not of the group's meaning. */
        if u.name == "is0" && r.vals[i] == 0 {
            continue;
        }
        /* A main or bitwise group's execution condition is a gate on the
         * whole group, not part of what its operations are: pco puts one on
         * any instruction that takes it (every ALU operation's OM_EXEC_CND,
         * set by pco_cf.c lower_if_pred_exec for a predicated `if`). So
         * if(p0) and if(!p0) read as e1_zx here, and a skeleton attested
         * unconditionally is attested under either. `(ignorepe)` stays its
         * own value: it is what makes a `cnd*` group one. */
        if u.name == "cc" && label == "hdr" && matches!(r.vals[i], 1 | 3) {
            write!(w, " cc=0")?;
            continue;
        }
        write!(w, " {}={}", u.name, set.fields[u.field].xform.decode(r.vals[i]))?;
    }
    Ok(())
}

/// A group's skeleton: its ALU type, header content, each operation with its
/// phase, form and every non-operand field, the source sections' mux, the
/// ISS routing, and whether it writes a destination -- everything but
/// registers and literals.
pub fn skeleton(w: &mut impl Write, g: &Group) -> core::fmt::Result {
    write!(w, "alu{}", g.alutype)?;
    region_skeleton(w, "hdr", &g.hdr, false)?;
    for (_, phase, r) in g.instrs() {
        let op = ops_for(r, phase).next().map(|o| o.name).unwrap_or("<unknown>");
        write!(w, " | p{phase} {op}")?;
        region_skeleton(w, "", r, true)?;
    }
    for (label, r) in [("lo", &g.lower), ("up", &g.upper), ("iss", &g.iss)] {
        if let Some(r) = r {
            region_skeleton(w, label, r, false)?;
        }
    }
    write!(w, " dst={}", g.dst.is_some())
}

/// A group built from a template: its operations in phase order are the
/// template's, its destination is the meant one, and each source slot names
/// the operand or constant the template assigns it.
fn check_template(t: &super::shapes::Template, meant: &SelGroup, g: &Group) -> Result<usize, Mismatch> {
    use super::shapes::Arg;
    /* A group decodes in SLOT order (phase 2's pack sits in slot 2, its
     * test in slot 3), so the template's steps are compared by slot. */
    /* Two operation names can share one encoding (`bbyp0bm` and `bbyp0s1`
     * pin the same constants), so an operation matches when ANY name its
     * encoding decodes as is the template's. */
    let mut ops = g.instrs().map(|(_, ph, r)| (ph, r));
    let mut want: [(u8, &'static str); super::decode::MAX_INSTRS] = [(u8::MAX, ""); super::decode::MAX_INSTRS];
    for (w, st) in want.iter_mut().zip(t.steps) {
        *w = (super::pack::slot_of(st.phase).unwrap_or(u8::MAX), st.op);
    }
    let want = &mut want[..t.steps.len()];
    want.sort_unstable_by_key(|w| w.0);
    for (_, name) in want.iter() {
        match ops.next() {
            Some((ph, r)) if ops_for(r, ph).any(|o| o.name == *name) => {}
            Some((ph, r)) => {
                let got = ops_for(r, ph).next().map(|o| o.name).unwrap_or("<unknown>");
                return Err(Mismatch::Operation { got, want: name });
            }
            None => return Err(Mismatch::Operation { got: "<none>", want: name }),
        }
    }
    if ops.next().is_some() {
        return Err(Mismatch::MoreThanOneOperation);
    }
    let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
    match meant.dst {
        Some(SelDst::Fixed { bank, reg }) if same(d[0], bank, reg) => {}
        Some(SelDst::Virtual(_)) => return Err(Mismatch::Unresolved),
        /* A template that writes no register (a loop's test into p0,
         * `KILL_TEST`): none meant, none built. It read as a destination
         * mismatch until a vertex shader kept a loop, the first program this
         * readback met one in. */
        None if !t.dst && d.iter().all(Option::is_none) => {}
        _ => return Err(Mismatch::Destination { slot: 0 }),
    }
    let mut s = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
    s[3..6].copy_from_slice(&up[3..6]);
    for (slot, a) in t.slots.iter().enumerate() {
        let ok = match a {
            None => true,
            Some(Arg::Special(r)) => same(s[slot], super::sel::Bank::Special, *r),
            Some(Arg::Op(i)) => match meant.srcs[*i] {
                Some(SelSrc::Fixed { bank, reg, mods }) => mods == Mods::default() && same(s[slot], bank, reg),
                Some(SelSrc::Virtual { .. }) => return Err(Mismatch::Unresolved),
                _ => false,
            },
        };
        if !ok {
            return Err(Mismatch::Source { slot: slot as u8 });
        }
    }
    Ok(g.total)
}

#[cfg(test)]
mod ld_tests {
    use super::super::assemble::build_group;
    use super::super::encode::encode_group;
    use super::super::sel::{Bank, OpType, PredArm, VReg};
    use super::*;

    fn t(r: u16) -> Option<SelSrc> {
        Some(SelSrc::Fixed { bank: Bank::Temp, reg: r, mods: Mods::default() })
    }
    fn sh(r: u16) -> Option<SelSrc> {
        Some(SelSrc::Fixed { bank: Bank::Shared, reg: r, mods: Mods::default() })
    }
    fn tdst(r: u16) -> Option<SelDst> {
        Some(SelDst::Fixed { bank: Bank::Temp, reg: r })
    }
    fn add64(d: u16) -> SelGroup {
        SelGroup { op: SelOp::Add64, ty: OpType::U32, dst: tdst(d), srcs: [sh(8), t(0), t(1)] }
    }
    fn ld(d: u16, burst: u8, a: u16) -> SelGroup {
        SelGroup { op: SelOp::Ld { burst }, ty: OpType::U32, dst: tdst(d), srcs: [t(a), None, None] }
    }
    fn marker(op: SelOp) -> SelGroup {
        SelGroup { op, ty: OpType::U32, dst: None, srcs: [None; 3] }
    }
    fn copy(d: u16, s: u16) -> SelGroup {
        SelGroup { op: SelOp::Copy, ty: OpType::F32, dst: tdst(d), srcs: [t(s), None, None] }
    }
    fn bytes(g: &SelGroup) -> std::vec::Vec<u8> {
        let b = build_group(g).expect("builds");
        let mut out = std::vec![0u8; b.total];
        encode_group(&b, &mut out, 0).expect("encodes");
        out
    }

    #[test]
    fn a_read_after_the_wait_is_in_order() {
        let wdf = marker(SelOp::WaitData);
        assert_eq!(check_ld_waits(&[add64(2), ld(4, 4, 2), wdf, copy(9, 7), copy(10, 2)]), Ok(()));
    }

    /// Before the `wdf`, the destination run (any lane of it) and the
    /// address pair (either half) are the `ld`'s: neither read nor written.
    #[test]
    fn a_register_the_ld_holds_touched_before_the_wait_is_refused() {
        let wdf = marker(SelOp::WaitData);
        for (k, g) in [copy(9, 7), copy(9, 4), copy(5, 0), copy(3, 0), copy(9, 2)].into_iter().enumerate() {
            assert_eq!(
                check_ld_waits(&[add64(2), ld(4, 4, 2), g, wdf]),
                Err(LdWait::Early { ld: 1, at: 2 }),
                "case {k}: {g:?}"
            );
        }
        /* outside both runs: fine */
        assert_eq!(check_ld_waits(&[add64(2), ld(4, 4, 2), copy(9, 8), wdf]), Ok(()));
        /* the same rule before allocation, on virtual values */
        let v = |n| Some(SelSrc::Virtual { v: VReg(n), mods: Mods::default() });
        let vld = SelGroup {
            op: SelOp::Ld { burst: 2 },
            ty: OpType::U32,
            dst: Some(SelDst::Virtual(VReg(20))),
            srcs: [v(10), None, None],
        };
        let read21 = SelGroup {
            op: SelOp::Copy,
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(VReg(30))),
            srcs: [v(21), None, None],
        };
        assert_eq!(check_ld_waits(&[vld, read21, wdf]), Err(LdWait::Early { ld: 0, at: 1 }));
        assert_eq!(check_ld_waits(&[vld, wdf, read21]), Ok(()));
    }

    /// A `wdf` must come before a loop or `if` edge and before the end.
    #[test]
    fn an_ld_with_no_wait_is_refused() {
        assert_eq!(check_ld_waits(&[add64(2), ld(4, 1, 2)]), Err(LdWait::NoWait { ld: 1 }));
        for m in [SelOp::LoopBegin, SelOp::LoopEnd, SelOp::Pred(PredArm::Then)] {
            let g = [add64(2), ld(4, 1, 2), marker(m), marker(SelOp::WaitData)];
            assert_eq!(check_ld_waits(&g), Err(LdWait::NoWait { ld: 1 }), "{m:?}");
        }
    }

    /// The built `ld` and `add64_32` read back as what was meant, and a
    /// different destination, address or burst does not.
    #[test]
    fn the_built_groups_read_back() {
        let l = ld(4, 4, 2);
        let b = bytes(&l);
        assert_eq!(check_group(&l, "", &b), Ok(b.len()));
        assert_eq!(check_group(&ld(5, 4, 2), "", &b), Err(Mismatch::Destination { slot: 0 }));
        assert_eq!(check_group(&ld(4, 4, 3), "", &b), Err(Mismatch::Source { slot: 0 }));
        assert!(matches!(check_group(&ld(4, 2, 2), "", &b), Err(Mismatch::Operation { .. })));
        let a = add64(2);
        let b = bytes(&a);
        assert_eq!(check_group(&a, "", &b), Ok(b.len()));
        assert_eq!(check_group(&add64(4), "", &b), Err(Mismatch::Destination { slot: 0 }));
        let other = SelGroup { srcs: [sh(8), t(0), t(3)], ..a };
        assert_eq!(check_group(&other, "", &b), Err(Mismatch::Source { slot: 2 }));
        assert!(matches!(check_group(&l, "", &b), Err(Mismatch::Operation { .. })));
    }

    /// The builder refuses what `prepare` should have legalised: an
    /// address that is not a temporary pair, an unresolved operand, a
    /// burst past a vec4.
    #[test]
    fn the_builder_refuses_an_unlegalised_load() {
        assert!(build_group(&SelGroup { srcs: [sh(8), None, None], ..ld(4, 1, 2) }).is_err());
        assert!(build_group(&ld(4, 5, 2)).is_err());
        let v =
            SelGroup { srcs: [Some(SelSrc::Virtual { v: VReg(1), mods: Mods::default() }), None, None], ..ld(4, 1, 2) };
        assert!(build_group(&v).is_err());
        assert!(build_group(&SelGroup { srcs: [sh(8), t(0), Some(SelSrc::Imm(4))], ..add64(2) }).is_err());
        assert!(build_group(&SelGroup { dst: Some(SelDst::Virtual(VReg(2))), ..add64(2) }).is_err());
    }
}
