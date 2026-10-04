// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Control flow with execution masks: the groups pco builds for a loop or an
//! `if` whose instances may go different ways.
//!
//! The construction is pco's own (Mesa `src/imagination/pco/pco_cf.c`, MIT,
//! Copyright © 2025 Imagination Technologies Ltd.; transcribed here):
//! one temporary, the execution mask counter (`emc`), holds each instance's
//! nesting state, and the `cnd*` operations change it and the instance's
//! execution predicate:
//!
//! - `cndst` (adjust n): enter a level; `pcnd` says which instances stay
//!   active (always, or those whose p0 is false/true).
//! - `cndef`: the other arm of an `if`.
//! - `cndsm` (a value): an instance leaves `value - 2` levels -- `break`
//!   (nesting + 2) and `continue` (nesting + 1) -- when `pcnd` holds.
//! - `cndlt` (adjust 2): a loop's end: p0 set while any instance goes on.
//! - `cndend` (adjust n): leave a level.
//!
//! A forward branch taken only when NO instance is active (`bpred = allp`)
//! skips a section no instance runs; the loop's back edge is the counted
//! `br if(p0)` of `fragment::branch_back`, p0 from `cndlt`.
//!
//! Every group here is built to the byte of pco's in the table's own program
//! for femtovg's filter_image.frag (a loop that leaves on data), which the
//! tests read back: the operation's fields, the header's `cc` (ignorepe for
//! the `cnd*` groups, which must run in every instance) and the operands.

use super::assemble::{close, narrowest_upper_src, BuildError, Parts};
use super::decode::{Group, MAX_INSTRS};
use super::fragment::{control, ev, hset};
use super::instr::instr_region;
use super::isa::{BitSetId, Region, PH_CTRL};
use super::legalise::{choose_src_variant, fill_srcs};
use super::operands::Operand;
use super::sel::Mods;

const ALU_CONTROL: u32 = 3;

/// One of the `cnd*` operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cnd {
    St,
    Ef,
    Sm,
    Lt,
    End,
}

impl Cnd {
    fn op(self) -> &'static str {
        match self {
            Cnd::St => "cndst",
            Cnd::Ef => "cndef",
            Cnd::Sm => "cndsm",
            Cnd::Lt => "cndlt",
            Cnd::End => "cndend",
        }
    }
}

/// A `cnd*` group: `op` with `adjust` (ignored where the operation fixes it)
/// under predicate condition `pcnd` ("always", "p0_true", "never",
/// "p0_false"), reading `srcs` (the emc first; `cndsm` reads a second and a
/// third, the value it leaves by) and writing the emc `emc`. The group runs
/// in every instance (`cc` ignorepe), as pco's do.
pub fn cnd(c: Cnd, adjust: u32, pcnd: &str, srcs: &[Operand], emc: Operand) -> Result<Group, BuildError> {
    let mut extra: [(&'static str, u32); 2] = [("pcnd", ev("pcnd", pcnd)?), ("adjust", adjust)];
    /* the operations that fix a field take none for it */
    let extra: &[(&'static str, u32)] = match c {
        Cnd::Sm => &extra[..1],
        Cnd::End => {
            extra[0] = ("adjust", adjust);
            &extra[..1]
        }
        _ => &extra[..],
    };
    let r = instr_region(c.op(), PH_CTRL, &[Mods::default(); 3], extra).map_err(BuildError::Form)?;
    let slot = super::pack::slot_of(PH_CTRL)
        .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: PH_CTRL }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((PH_CTRL, r));

    if srcs.is_empty() || srcs.len() > 3 {
        return Err(BuildError::Unsupported("a cnd group reads one to three sources"));
    }
    let mut ops: [Option<Operand>; 6] = [None; 6];
    for (o, s) in ops.iter_mut().zip(srcs) {
        *o = Some(*s);
    }
    let v = choose_src_variant(&ops, 0).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, v);
    fill_srcs(&mut lower, &ops, 0).map_err(BuildError::Fit)?;
    let upper = Some(Region::new(BitSetId::Src, narrowest_upper_src()?));

    let dst_ops = [Some(emc), None];
    let dv = super::legalise::choose_dst_variant(&dst_ops).map_err(BuildError::Fit)?;
    let mut d = Region::new(BitSetId::Dst, dv);
    super::legalise::fill_dsts(&mut d, &dst_ops).map_err(BuildError::Fit)?;

    let hv = super::pack::header_variant_of(ALU_CONTROL).ok_or(BuildError::Unsupported("no control header"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    hset(&mut hdr, "cc", ev("cc", "(ignorepe)")?)?;
    hset(&mut hdr, "ctrlop", ev("ctrlop", "cnd")?)?;
    hset(&mut hdr, "w0p", 1)?;
    let parts =
        Parts { alutype: ALU_CONTROL, instrs, lower: Some(lower), upper, iss: None, dst: Some(d), align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// A forward branch taken when no instance is active (`bpred = allp`), by
/// `offset` bytes from the branch group's own start: past a section no
/// instance runs (the loop's body when none enters, as pco's prologue does).
pub fn branch_none_active(offset: i64) -> Result<Group, BuildError> {
    if offset <= 0 {
        return Err(BuildError::Unsupported("a skip that does not go forward"));
    }
    let off = u32::try_from(offset).map_err(|_| BuildError::Unsupported("a skip too long to encode"))?;
    control("br", &[("offset", off), ("bpred", ev("bpred", "allp")?), ("link", 0)], "e1_zx", "b")
}
