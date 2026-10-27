// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Building one instruction of a group: which encoded form an operation takes,
//! and whether that form can carry the source modifiers the operands need.
//!
//! The inverse of [`super::isa::ops_for`], which reads an operation out of an
//! encoded instruction. Here the operation and its modifiers are known and the
//! form has to be chosen.
//!
//! # Why an operation can have several forms, and why that matters
//!
//! `isa_tables::OPS` lists `fmad` twice in phase 0, at variants 15 and 16, and
//! `frcp` twice likewise. The forms differ in width and in which modifier
//! fields they carry: the narrow `sngl` form of a single-source operation has
//! no `s0neg` at all, while `sngl_ext` does. So a `frcp` of a negated operand
//! cannot use the narrow form -- not because the operation differs, but
//! because the negate has nowhere to live.
//!
//! That is the same shape as the source-section choice in [`super::legalise`]:
//! take the narrowest form that can express what is needed, and when none can,
//! say which requirement defeated them so the caller can emit a separate
//! `mbyp` carrying the modifier instead.

use super::isa::{BitSetId, Region, SetError};
use super::isa_tables::OPS;
use super::sel::Mods;

/// Why no encoded form of an operation will do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoForm {
    /// The tables list no form of this operation in this phase. A selection
    /// bug or a `builtins::shape` entry naming a phase the ISA does not allow.
    UnknownOp { op: &'static str, phase: u8 },
    /// Every form exists but none carries this modifier on this source, so the
    /// modifier has to become an instruction of its own.
    Modifier { op: &'static str, slot: usize, modifier: &'static str },
    /// No form carries a modifier that belongs to the instruction rather than
    /// to one of its sources -- a saturate, a pack format, a rounding mode.
    /// Unlike a source modifier there is no separate instruction to move it
    /// to, so this is a refusal and not a lowering.
    InstrModifier { op: &'static str, modifier: &'static str },
}

/// The modifier field names, by source slot. `flr` exists only on some slots
/// of some forms, which is exactly what makes this a choice rather than a
/// lookup.
pub(super) const NEG: [&str; 3] = ["s0neg", "s1neg", "s2neg"];
pub(super) const ABS: [&str; 3] = ["s0abs", "s1abs", "s2abs"];
pub(super) const FLR: [&str; 3] = ["s0flr", "s1flr", "s2flr"];

/// The narrowest encoded form of `op` in `phase` that carries every modifier
/// the sources need.
///
/// `mods[i]` is the modifier set of source slot `i`; a slot with no modifiers
/// constrains nothing. The returned region has the operation's pinned
/// constants and the modifiers set, and nothing else -- the registers live in
/// the source and destination sections, not here.
///
/// `extra` carries the modifiers that belong to the instruction rather than to
/// a source: `sat`, `pck_format`, `scale_rtz`, `red_part`, `red_type`, `s`.
/// The re-encode gate measured them -- rebuilding the reference compiler's own
/// groups with an empty `extra` left 21,570 of 416,991 groups wrong, and every
/// named difference was one of these six fields. A form that does not carry a
/// field encodes its default, so a zero value never constrains the choice and
/// a caller may pass the whole set without knowing which form will be picked.
pub fn instr_region(
    op: &'static str,
    phase: u8,
    mods: &[Mods; 3],
    extra: &[(&'static str, u32)],
) -> Result<Region, NoForm> {
    let mut any = false;
    let mut first_missing: Option<NoForm> = None;
    let mut best: Option<(Region, u8)> = None;

    for o in OPS.iter().filter(|o| o.name == op && o.phase == phase) {
        any = true;
        let set = set_of(o);
        let bytes = super::isa::bit_set(set).variants[o.variant as usize].bytes;
        /* Nothing narrower is possible than what is already held. */
        if best.as_ref().is_some_and(|(_, b)| *b <= bytes) {
            continue;
        }
        let mut r = Region::new(set, o.variant);
        let mut ok = true;
        for (f, v) in o.consts {
            if r.set(f, *v).is_err() {
                ok = false;
            }
        }
        for (slot, m) in mods.iter().enumerate() {
            for (want, name, label) in [(m.neg, NEG[slot], "neg"), (m.abs, ABS[slot], "abs"), (m.flr, FLR[slot], "flr")]
            {
                if want && r.set(name, 1).is_err() {
                    ok = false;
                    first_missing.get_or_insert(NoForm::Modifier { op, slot, modifier: label });
                }
            }
        }
        for (name, value) in extra {
            match r.set(name, *value) {
                Ok(()) => {}
                /* A form without the field encodes the default, which is the
                 * zero this caller is asking for: no constraint. */
                Err(SetError::NoSuchField { .. }) if *value == 0 => {}
                Err(_) => {
                    ok = false;
                    first_missing.get_or_insert(NoForm::InstrModifier { op, modifier: name });
                }
            }
        }
        if ok {
            best = Some((r, bytes));
        }
    }

    if !any {
        return Err(NoForm::UnknownOp { op, phase });
    }
    match best {
        Some((r, _)) => Ok(r),
        None => Err(first_missing.unwrap_or(NoForm::UnknownOp { op, phase })),
    }
}

/// The bit set an operation's form lives in.
fn set_of(o: &super::isa::OpEnc) -> BitSetId {
    match o.set {
        4 => BitSetId::Main,
        5 => BitSetId::Backend,
        6 => BitSetId::Bitwise,
        7 => BitSetId::Ctrl,
        _ => BitSetId::Main,
    }
}

/// The source-section variant an operation pins, whatever its operand fits in.
///
/// Normally the section is chosen from the operands alone: the narrowest form
/// that can name every register ([`super::legalise::choose_src_variant`]).
/// `uvsw.write` and its fused form `uvsw.write.emit.endtask` are the operations
/// in the reference compiler's corpus that contradict it. They take
/// `1lo_3b11i_2m` in all 796 of their groups, and in most of those the only
/// source is a low-numbered temporary that the 1-byte `1lo_1b6i` form names
/// perfectly well -- so the width is not coming from the operand.
///
/// The backend phase is not the axis: the same phase uses `1lo_1b6i` 480 times
/// for `savmsk`, `st.tiled` and `uvsw.emit.endtask`. Nor is it a free choice,
/// because a free choice would vary and this one has no exception in the
/// corpus. `alphaf` and `smp` also use one form each, but their operands
/// require it, so they are not evidence of pinning and are not listed here.
///
/// Returning the name rather than an index keeps this readable against the
/// tables; the lookup that resolves it is the same one
/// [`super::pack::header_variant_of`] uses.
pub fn pinned_src_variant(op: &str) -> Option<u16> {
    let want = match op {
        "uvsw.write" | "uvsw.write.emit.endtask" => "1lo_3b11i_2m",
        _ => return None,
    };
    let set = super::isa::bit_set(BitSetId::Src);
    set.variants.iter().position(|v| v.name == want).map(|i| i as u16)
}
