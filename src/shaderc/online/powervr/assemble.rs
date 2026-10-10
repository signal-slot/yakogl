// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Ported from Mesa 26.2.1 src/imagination/pco/pco_map.py (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Putting a group together: the header's derived fields, the padding, and the
//! length the whole thing states about itself.
//!
//! The choosers say which section and which form each part takes
//! ([`super::legalise`], [`super::instr`]). This closes the group over them:
//! everything in the header that is a FUNCTION of the parts is computed here
//! rather than supplied, because a header that disagrees with its own body is a
//! group the hardware will read past the end of.
//!
//! The counterpart of [`super::decode`], which reads these same fields out of
//! bytes, and the step before [`super::encode`], which writes an already
//! complete group and derives nothing.
//!
//! The four rules were established over the whole offline table before any
//! group was built (`roundtrip::the_group_length_and_da_rules_hold_for_every_group`),
//! which is why this file can apply them rather than guess them:
//!
//! 1. the word pad is present exactly when the body is odd;
//! 2. the total is header + instructions + sections + word pad + alignment;
//! 3. the header's `length` states that total in 2-byte words, alignment
//!    included;
//! 4. `da`, the decode assist, is the header plus the instructions from slot 2
//!    for a main-ALU group, the header plus all of them for a bitwise one, and
//!    zero for a control group with no sources, destinations or destination
//!    section at all.

use super::decode::{Group, MAX_INSTRS};

/// The only ALU type the builder emits today. Named rather than written as a
/// bare 0 because three of the rules below are functions of it.
const ALUTYPE_MAIN: u32 = 0;
use super::instr::{self, instr_region};
use super::isa::{BitSetId, Region};
use super::legalise::{choose_src_variant, fill_srcs};
use super::operands::Operand;
use super::sel::Mods;
use crate::shaderc::online::ir::AluOp;

/// Why a group cannot be closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoGroup {
    /// The body is longer than the header's word count can state. `length` is
    /// four bits under `WrapMod16`, so sixteen two-byte words is the most a
    /// group can be; a group needing more has to be split, and splitting is
    /// the caller's decision rather than this file's.
    TooLong { bytes: usize },
    /// The total came out odd after its own pad, which can only mean the parts
    /// do not tile. A bug here, not in the caller.
    NotWhole { bytes: usize },
}

/// The parts of a group, before the header closes over them.
pub struct Parts {
    pub alutype: u32,
    /// By slot: the index IS the slot, and the tuple does not repeat it.
    pub instrs: [Option<(u8, Region)>; MAX_INSTRS],
    pub lower: Option<Region>,
    pub upper: Option<Region>,
    pub iss: Option<Region>,
    pub dst: Option<Region>,
    /// Bytes of the i-cache alignment block that closes a shader: zero for
    /// every group but the last.
    pub align_pad: u8,
}

/// The decode-assist value a group's parts imply.
///
/// Split out because it is the one derived field whose rule is not arithmetic,
/// and because the control-group case reads as an exception until one knows it
/// is pco's `no_srcs_dests`. Note that the ISS section is deliberately not one
/// of the three that case looks at.
pub fn decode_assist(p: &Parts, hdr_bytes: usize) -> usize {
    let from = |slot_min: u8| -> usize {
        p.instrs
            .iter()
            .enumerate()
            .filter(|(slot, _)| *slot >= slot_min as usize)
            .filter_map(|(_, o)| o.as_ref())
            .map(|(_, r)| r.bytes())
            .sum()
    };
    const ALU_MAIN: u32 = 0;
    const ALU_BITWISE: u32 = 2;
    match p.alutype {
        ALU_MAIN => hdr_bytes + from(2),
        ALU_BITWISE => hdr_bytes + from(0),
        _ if p.lower.is_none() && p.upper.is_none() && p.dst.is_none() => 0,
        _ => hdr_bytes + from(0),
    }
}

/// Close a group over its parts: pad it, measure it, and fill the header
/// fields the body determines.
///
/// The header region is supplied rather than built because which header
/// variant a group takes is a choice like the others; this fills only what the
/// body decides.
pub fn close(p: &Parts, mut hdr: Region) -> Result<Group, NoGroup> {
    let hdr_bytes = hdr.bytes();
    let instrs: usize = p.instrs.iter().flatten().map(|(_, r)| r.bytes()).sum();
    let sections: usize = [&p.lower, &p.upper, &p.iss, &p.dst].into_iter().flatten().map(|r| r.bytes()).sum();
    let body = hdr_bytes + instrs + sections;
    let word_pad = u8::from(!body.is_multiple_of(2));
    let total = body + word_pad as usize + p.align_pad as usize;

    if !total.is_multiple_of(2) {
        return Err(NoGroup::NotWhole { bytes: total });
    }
    let words = total / 2;
    if words == 0 || words > 16 {
        return Err(NoGroup::TooLong { bytes: total });
    }

    /* `Region::set` refuses a value the field cannot hold, so neither of these
     * can silently truncate: `words` is bounded above by the check just made,
     * and `da` is a byte count inside a group that is itself bounded. A
     * failure would be this file's bug rather than the caller's, which is what
     * the debug assertions say. */
    let da = decode_assist(p, hdr_bytes);
    debug_assert!(hdr.set("length", words as u32).is_ok(), "length {words} does not fit the header");
    debug_assert!(hdr.set("da", da as u32).is_ok(), "da {da} does not fit the header");
    let _ = hdr.set("length", words as u32);
    let _ = hdr.set("da", da as u32);

    Ok(Group {
        hdr,
        alutype: p.alutype,
        instrs: p.instrs,
        lower: p.lower,
        upper: p.upper,
        iss: p.iss,
        dst: p.dst,
        word_pad,
        align_pad: p.align_pad,
        total,
    })
}

/// Why one selected group could not be built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildError {
    /// No encoded form of the operation carries what it needs.
    Form(super::instr::NoForm),
    /// No section can hold the operands.
    Fit(super::legalise::NoFit),
    /// The phases do not make a legal group.
    Phases(super::pack::NotPackable),
    /// The parts do not close.
    Group(NoGroup),
    /// Something the builder does not do yet, named rather than guessed at.
    /// An immediate is the common one: the USC encodes it inside the
    /// instruction (`bbyp0bm_imm32`), not in a source section, so it is a
    /// different path rather than a wider variant.
    Unsupported(&'static str),
    /// An operation whose shape is wired through internal registers or
    /// operation fields (tst/movc/bitwise) and is not transcribed yet. It
    /// carries the operation: one cause per operation, so the survey says
    /// which transcription would carry which programs.
    UntranscribedShape(AluOp),
}

/// The single-operation phase-0 shapes whose whole encoding is operands and
/// source modifiers: nothing in the group is wired through an internal
/// register, and the operation has no field of its own to set. Each is
/// checked against the corpus by the chain gate (`tests/usc_limits.rs`).
pub const TRANSCRIBED_SINGLE: &[&str] =
    &["fadd", "fmul", "fmad", "frcp", "frsq", "fexp", "flog", "mbyp", "fdsx", "fdsy"];

/// A 32-bit immediate moved into a register: pco's `group_map(O_MOVI32)`
/// (Mesa `src/imagination/pco/pco_map.py`, MIT, read and transcribed).
///
/// A bitwise-pipeline group: header `opcnt = p0`, `w0p`, one
/// `bbyp0bm_imm32` in phase 0 with `count_src = s2`, `count_op = byp`,
/// `shift1_op = byp` (the operation's pinned constants, which
/// [`instr_region`] sets) and the value in `imm32`; `s0` names special
/// register 0 because the operation's first source is `s0` and the value
/// never reads it; the destination is `w0`. No ISS section, which the
/// corpus's 28,325 groups of this exact shape agree with.
pub fn build_imm_move(dst: Operand, value: u32) -> Result<Group, BuildError> {
    const ALUTYPE_BITWISE: u32 = 2;
    let op = "bbyp0bm_imm32";
    let mut r = instr_region(op, super::isa::PH_0, &[Mods::default(); 3], &[]).map_err(BuildError::Form)?;
    r.set("imm32", value).map_err(|_| BuildError::Unsupported("the immediate does not fit imm32"))?;
    let slot = super::pack::slot_of(super::isa::PH_0)
        .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: super::isa::PH_0 }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((super::isa::PH_0, r));

    const SPECIAL: u32 = 0;
    let src_ops = [Some(Operand { bank: SPECIAL, index: 0 }), None, None, None, None, None];
    let v = choose_src_variant(&src_ops, 0).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, v);
    fill_srcs(&mut lower, &src_ops, 0).map_err(BuildError::Fit)?;
    let upper =
        super::pack::upper_present(true).then(narrowest_upper_src).transpose()?.map(|v| Region::new(BitSetId::Src, v));
    let iss = super::pack::iss_present(ALUTYPE_BITWISE, true).then(|| Region::new(BitSetId::Iss, 0));

    let dst_ops = [Some(dst), None];
    let dv = super::legalise::choose_dst_variant(&dst_ops).map_err(BuildError::Fit)?;
    let mut d = Region::new(BitSetId::Dst, dv);
    super::legalise::fill_dsts(&mut d, &dst_ops).map_err(BuildError::Fit)?;

    let hv = super::pack::header_variant_of(ALUTYPE_BITWISE)
        .ok_or(BuildError::Unsupported("no header variant for this ALU type"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let opcnt = super::pack::opcnt_for(&[super::isa::PH_0])
        .ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hdr.set("opcnt", opcnt).map_err(|_| BuildError::Unsupported("opcnt does not fit the header"))?;
    hdr.set("w0p", 1).map_err(|_| BuildError::Unsupported("no w0p in the header"))?;

    let parts = Parts { alutype: ALUTYPE_BITWISE, instrs, lower: Some(lower), upper, iss, dst: Some(d), align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// Build one resolved group: choose the forms and sections, fill them, pack by
/// phase and close.
///
/// `g` must be resolved -- every source and destination `Fixed`, which is what
/// [`super::alloc::allocate`] returns. A `Virtual` here is a caller error and
/// is refused rather than assigned a register on the spot, because guessing
/// one would produce a group that reads a register nobody wrote.
pub fn build_group(g: &super::sel::SelGroup) -> Result<Group, BuildError> {
    use super::builtins::{shape, Shape};
    use super::isa::{BitSetId, Region, PH_0};
    use super::legalise::{choose_dst_variant, choose_src_variant, fill_dsts, fill_srcs};
    use super::operands::Operand;
    use super::pack::{check, oporg_for};
    use super::sel::{Mods, SelDst, SelOp, SelSrc};

    /* An immediate is not an operand of any group this builder emits: it is
     * its own group, a bitwise-pipeline move (`bbyp0bm_imm32`) into the
     * destination. `prepare` turns every immediate a group reads into one of
     * these, or into a constant register. */
    if let (SelOp::Copy, Some(SelSrc::Imm(v))) = (g.op, g.srcs[0]) {
        return match g.dst {
            Some(SelDst::Fixed { bank, reg }) => build_imm_move(Operand { bank: bank.hw(), index: reg as u32 }, v),
            Some(SelDst::Virtual(_)) => Err(BuildError::Unsupported("an unresolved virtual destination")),
            None => Err(BuildError::Unsupported("an immediate move with no destination")),
        };
    }

    /* A resolved sample and the wait after it (`chain::expand_samples`
     * produced both). */
    match g.op {
        SelOp::Sample { proj, lod, fetch, offset, cube, .. } => {
            let fixed = |s: Option<SelSrc>, bank: super::sel::Bank| match s {
                Some(SelSrc::Fixed { bank: b, reg, mods }) if b == bank && mods == Mods::default() => Ok(reg),
                _ => Err(BuildError::Unsupported("a sample that was not expanded")),
            };
            /* a descriptor word: a shared register, or one read through idx0
             * past sh255 (`prepare::index_high_shared`) */
            let state = |s: Option<SelSrc>| match s {
                Some(SelSrc::Fixed { bank: b @ (super::sel::Bank::Shared | super::sel::Bank::Idx0), reg, mods })
                    if mods == Mods::default() =>
                {
                    Ok(Operand { bank: b.hw(), index: u32::from(reg) })
                }
                _ => Err(BuildError::Unsupported("a sample that was not expanded")),
            };
            let image = state(g.srcs[0])?;
            let data = fixed(g.srcs[1], super::sel::Bank::Temp)?;
            let sampler = state(g.srcs[2])?;
            let Some(SelDst::Fixed { bank: super::sel::Bank::Temp, reg: dest }) = g.dst else {
                return Err(BuildError::Unsupported("a sample that was not expanded"));
            };
            /* an isampler / usampler look-up (`select::smp_type`): no fcnorm */
            let integer = g.ty != super::sel::OpType::F32;
            return super::fragment::build_smp(
                image,
                data,
                sampler,
                dest,
                super::fragment::SmpForm { proj, lod, fetch, offset: offset.is_some(), cube, integer },
            );
        }
        SelOp::WaitData => return super::fragment::wait_data(),
        /* The execution mask (`powervr::cf`), on the emc the allocator gave
         * a temporary: each reads it (special 0 for the set-up) and writes it
         * back. */
        SelOp::Machine(op) if op.is_cnd() => {
            use super::cf::{cnd, Cnd};
            use super::sel::{Bank, MachineOp};
            const SPECIAL: u32 = 0;
            const TEMP: u32 = 1;
            let Some(SelDst::Fixed { bank: Bank::Temp, reg: emc }) = g.dst else {
                return Err(BuildError::Unsupported("an execution mask not in a temporary"));
            };
            let emc = Operand { bank: TEMP, index: u32::from(emc) };
            let src = match g.srcs[0] {
                Some(SelSrc::Fixed { bank: Bank::Special, reg: 0, mods }) if mods == Mods::default() => {
                    Operand { bank: SPECIAL, index: 0 }
                }
                Some(SelSrc::Fixed { bank: Bank::Temp, reg, mods }) if mods == Mods::default() => {
                    Operand { bank: TEMP, index: u32::from(reg) }
                }
                _ => return Err(BuildError::Unsupported("an execution mask read from elsewhere")),
            };
            let special = |i: u32| Operand { bank: SPECIAL, index: i };
            return match op {
                MachineOp::CndInit => cnd(Cnd::St, 1, "always", &[src], emc),
                MachineOp::CndEnter => cnd(Cnd::St, 2, "always", &[src], emc),
                /* by 2: out of this loop's level (pco_cf.c, `break`) */
                MachineOp::CndLeave => cnd(Cnd::Sm, 0, "p0_false", &[src, special(0), special(2)], emc),
                MachineOp::CndLoopEnd => cnd(Cnd::Lt, 2, "always", &[src], emc),
                /* a branched `if` (pco_cf.c lower_if_cond_exec), each by 1 */
                MachineOp::CndIf => cnd(Cnd::St, 1, "p0_false", &[src], emc),
                MachineOp::CndElse => cnd(Cnd::Ef, 1, "always", &[src], emc),
                MachineOp::CndIfEnd => cnd(Cnd::End, 1, "always", &[src], emc),
                _ => Err(BuildError::Unsupported("an execution-mask operation the builder does not know")),
            };
        }
        _ => {}
    }

    /* A group made of several operations wired through the internal
     * registers is built from its template (`shapes.rs`). Its operands must
     * be plain registers: `prepare` has moved anything carrying a modifier
     * or an immediate. */
    {
        if let Some(t) = super::shapes::template_for(g.op) {
            let mut ops: [Option<Operand>; 3] = [None; 3];
            for (o, s) in ops.iter_mut().zip(g.srcs.iter()).take(g.op.arity()) {
                *o = match s {
                    Some(SelSrc::Fixed { bank, reg, mods }) if *mods == Mods::default() => {
                        Some(Operand { bank: bank.hw(), index: *reg as u32 })
                    }
                    _ => return Err(BuildError::Unsupported("a template operand that is not a plain register")),
                };
            }
            /* A template with no destination (a loop's `tstz.p0`) writes
             * only the predicate. */
            if !t.dst {
                if g.dst.is_some() {
                    return Err(BuildError::Unsupported("a destination on a template that writes none"));
                }
                return super::shapes::build_opt(t, &ops, None);
            }
            let Some(SelDst::Fixed { bank, reg }) = g.dst else {
                return Err(BuildError::Unsupported("an unresolved virtual destination"));
            };
            return super::shapes::build(t, &ops, Operand { bank: bank.hw(), index: reg as u32 });
        }
    }

    /* The operations this group's one kernel op becomes. A copy is `mbyp` in
     * phase 0; everything else comes from the shape table. */
    let steps: &[super::builtins::Step] = match g.op {
        SelOp::Copy => &[super::builtins::Step { op: "mbyp", phase: PH_0, corpus: 0 }],
        SelOp::Sample { .. } | SelOp::WaitData => unreachable!("built above"),
        /* the chain resolved every one into a copy (`expand_samples`) */
        SelOp::TexWord { .. } => return Err(BuildError::Unsupported("a texture state word that was not resolved")),
        SelOp::SampleZ => return Err(BuildError::Unsupported("a cube coordinate the chain did not consume")),
        SelOp::SampleGrad => return Err(BuildError::Unsupported("gradients the chain did not consume")),
        SelOp::PlaneCoeff { .. } => return Err(BuildError::Unsupported("a plane coefficient that was not resolved")),
        /* The program builder places these: a label, and the branch to it,
         * whose offset only it knows (`Loops`). */
        SelOp::LoopBegin | SelOp::LoopEnd | SelOp::Pred(_) => {
            return Err(BuildError::Unsupported("a loop or `if` marker outside the program builder"))
        }
        SelOp::Machine(_) => unreachable!("every machine operation is a template"),
        SelOp::Alu(a) => match shape(a) {
            /* ONE operation in phase 0 whose form carries nothing but its
             * operands and their modifiers. Everything else in the shape
             * table -- `min` as mbyp/mbyp/tst/movc, a compare's `tst_op`, the
             * bitwise pipeline -- is wired through internal registers (`ft0`,
             * `ftt`, `is1..is4`) and operation fields that this builder does
             * not yet set, so it used to emit them with every one of those
             * left zero: a group that decodes and computes something else.
             * Refused until each is transcribed from its `group_map` and
             * checked against the corpus. */
            Shape::Group(steps) if steps.len() == 1 && TRANSCRIBED_SINGLE.contains(&steps[0].op) => {
                if g.ty != super::sel::OpType::F32 {
                    return Err(BuildError::Unsupported("a float operation on a non-float type"));
                }
                steps
            }
            Shape::Group(_) => return Err(BuildError::UntranscribedShape(a)),
            Shape::Modifier(_) => return Err(BuildError::Unsupported("a modifier has no group of its own")),
            Shape::Lower(_) => return Err(BuildError::Unsupported("lowering into several groups")),
            Shape::SelectionExpands(_) => return Err(BuildError::Unsupported("selection should have expanded this")),
            Shape::Refused(_) => return Err(BuildError::Unsupported("this machine cannot")),
        },
    };

    /* Operands, and the modifiers each carries. */
    let mut mods = [Mods::default(); 3];
    let mut src_ops: [Option<Operand>; 6] = [None; 6];
    for (i, s) in g.srcs.iter().enumerate().take(3) {
        match s {
            None => {}
            Some(SelSrc::Fixed { bank, reg, mods: m }) => {
                src_ops[i] = Some(Operand { bank: bank.hw(), index: *reg as u32 });
                mods[i] = *m;
            }
            Some(SelSrc::Virtual { .. }) => return Err(BuildError::Unsupported("an unresolved virtual source")),
            Some(SelSrc::Imm(_)) => return Err(BuildError::Unsupported("an immediate operand")),
        }
    }
    let dst_ops: [Option<Operand>; 2] = match g.dst {
        None => [None; 2],
        Some(SelDst::Fixed { bank, reg }) => [Some(Operand { bank: bank.hw(), index: reg as u32 }), None],
        Some(SelDst::Virtual(_)) => return Err(BuildError::Unsupported("an unresolved virtual destination")),
    };

    /* One instruction per step, in its own slot. */
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    let mut phases: [u8; MAX_INSTRS] = [0; MAX_INSTRS];
    let mut nph = 0;
    /* An operation may pin the source section (see `instr::pinned_src_variant`).
     * Two that pin different ones cannot share a group, and that is refused
     * here rather than resolved by taking the wider: the two would disagree
     * about where the registers sit, not merely about width. */
    let mut pinned: Option<u16> = None;
    for st in steps {
        if let Some(v) = instr::pinned_src_variant(st.op) {
            if pinned.is_some_and(|p| p != v) {
                return Err(BuildError::Unsupported("two operations pinning different source sections"));
            }
            pinned = Some(v);
        }
        let slot = super::pack::slot_of(st.phase)
            .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: st.phase }))?;
        /* No instruction-level modifier yet: selection produces none, and a
         * saturate or pack format will arrive with the operation that needs
         * it rather than as an empty channel nobody fills. */
        let r = instr_region(st.op, st.phase, &mods, &[]).map_err(BuildError::Form)?;
        instrs[slot as usize] = Some((st.phase, r));
        phases[nph] = st.phase;
        nph += 1;
    }
    let mut set = phases[..nph].to_vec_in_place();
    check(&set).map_err(BuildError::Phases)?;

    let lower = if src_ops[0..3].iter().any(Option::is_some) {
        /* Selection produces no source mux yet; when it does, the value
         * arrives here beside the operands rather than as a field written
         * after the variant has already been chosen. */
        let v = match pinned {
            Some(v) => v,
            None => choose_src_variant(&src_ops, 0).map_err(BuildError::Fit)?,
        };
        let mut r = Region::new(BitSetId::Src, v);
        fill_srcs(&mut r, &src_ops, 0).map_err(BuildError::Fit)?;
        Some(r)
    } else {
        None
    };
    let dst = if dst_ops[0].is_some() {
        let v = choose_dst_variant(&dst_ops).map_err(BuildError::Fit)?;
        let mut r = Region::new(BitSetId::Dst, v);
        fill_dsts(&mut r, &dst_ops).map_err(BuildError::Fit)?;
        Some(r)
    } else {
        None
    };

    /* THE UPPER SECTION AND THE ISS SECTION ARE NOT OPTIONAL.
     *
     * Measured, because the first version of this omitted both and every group
     * it emitted was unparseable. Of the 439,209 phase-0-only groups in the
     * table with a source section, a destination and no alignment padding,
     * ZERO lack an upper source section
     * (`roundtrip::which_predicate_empties_the_simplest_shape`), and pco's own
     * group for this very shader carries both.
     *
     * What they hold is nothing: pco writes `sb3 = 0, s3 = 0` in the upper
     * section and zero in every `is1..is5` of the ISS section. So they are
     * present and empty -- placeholders the layout requires rather than
     * operands -- and the narrowest variant of each, left zero, is exactly
     * what to emit. */
    let has_lower = lower.is_some();
    let upper = super::pack::upper_present(has_lower)
        .then(narrowest_upper_src)
        .transpose()?
        .map(|v| Region::new(BitSetId::Src, v));
    let iss = super::pack::iss_present(ALUTYPE_MAIN, has_lower).then(|| Region::new(BitSetId::Iss, 0));

    /* The header: the class its phases imply, and the fields `close` fills. */
    let oporg = oporg_for(&set).ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    let hdr_variant = super::pack::header_variant_of(ALUTYPE_MAIN)
        .ok_or(BuildError::Unsupported("no header variant for this ALU type"))?;
    let _ = oporg;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hdr_variant);
    let _ = hdr.set("oporg", oporg);
    /* `w0p` and `w1p` say a destination section is present, and the corpus
     * says how (`roundtrip::what_w0p_and_w1p_track`): both clear means no
     * destination, EITHER set means one, both set means two. So they are
     * per-phase write flags rather than per-slot -- (0,1) and (1,0) both
     * occur and both mean a single write -- and the destination count is
     * their popcount.
     *
     * Leaving them zero is what made the first 37 groups undecodable: the
     * bytes of a destination section were present and the header said there
     * was none, so the decoder's accounting ran off the end. */
    if dst_ops[0].is_some() {
        let _ = hdr.set("w0p", 1);
    }
    if dst_ops[1].is_some() {
        let _ = hdr.set("w1p", 1);
    }
    set.clear();

    let parts = Parts { alutype: ALUTYPE_MAIN, instrs, lower, upper, iss, dst, align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// The narrowest upper source section, for the placeholder every group needs.
pub(super) fn narrowest_upper_src() -> Result<u16, BuildError> {
    let set = super::isa::bit_set(super::isa::BitSetId::Src);
    set.variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.upper_src == Some(true))
        .min_by_key(|(_, v)| v.bytes)
        .map(|(i, _)| i as u16)
        .ok_or(BuildError::Unsupported("no upper source variant in the tables"))
}

/// A tiny helper so the phase list can be checked without an allocator.
trait ToVecInPlace {
    fn to_vec_in_place(&self) -> heapless_set::PhaseSet;
}
impl ToVecInPlace for [u8] {
    fn to_vec_in_place(&self) -> heapless_set::PhaseSet {
        let mut s = heapless_set::PhaseSet::default();
        for &p in self {
            s.push(p);
        }
        s.sort_dedup();
        s
    }
}

/// A sorted, deduplicated phase set of at most [`MAX_INSTRS`] entries, without
/// an allocator: the builder runs in `no_std` with only the caller's arena,
/// and a group can never have more phases than it has slots.
pub mod heapless_set {
    use super::MAX_INSTRS;

    #[derive(Default)]
    pub struct PhaseSet {
        buf: [u8; MAX_INSTRS],
        len: usize,
    }
    impl PhaseSet {
        pub fn push(&mut self, p: u8) {
            if self.len < MAX_INSTRS {
                self.buf[self.len] = p;
                self.len += 1;
            }
        }
        pub fn sort_dedup(&mut self) {
            self.buf[..self.len].sort_unstable();
            let mut w = 0;
            for i in 0..self.len {
                if w == 0 || self.buf[w - 1] != self.buf[i] {
                    self.buf[w] = self.buf[i];
                    w += 1;
                }
            }
            self.len = w;
        }
        pub fn clear(&mut self) {
            self.len = 0;
        }
    }
    impl core::ops::Deref for PhaseSet {
        type Target = [u8];
        fn deref(&self) -> &[u8] {
            &self.buf[..self.len]
        }
    }
}

/// The largest run of registers one `uvsw.write` can carry, measured from the
/// header field rather than assumed: `rpt` is narrow and `Region::set` refuses
/// a value it cannot hold, so the cap is whatever that field accepts.
fn max_repeat() -> u32 {
    let Some(hv) = super::pack::header_variant_of(ALUTYPE_MAIN) else { return 1 };
    (1..=16).rev().find(|n| Region::new(BitSetId::IgrpHdr, hv).set("rpt", *n).is_ok()).unwrap_or(1)
}

/// One backend-phase group with a single source and no destination.
///
/// Both halves of the output epilogue have this shape. `end` marks the last
/// group of the shader and `rpt` the number of consecutive registers the
/// write covers.
fn backend_group(
    op: &'static str,
    bank: u32,
    index: u32,
    rpt: u32,
    end: bool,
    align_pad: u8,
    extra: &[(&'static str, u32)],
    iss4_fte: bool,
) -> Result<Group, BuildError> {
    const PH_BACKEND: u8 = 6;
    let r = instr_region(op, PH_BACKEND, &[Mods::default(); 3], extra).map_err(BuildError::Form)?;
    let slot = super::pack::slot_of(PH_BACKEND)
        .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: PH_BACKEND }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((PH_BACKEND, r));

    let src_ops = [Some(Operand { bank, index }), None, None, None, None, None];
    let v = match instr::pinned_src_variant(op) {
        Some(v) => v,
        None => choose_src_variant(&src_ops, 0).map_err(BuildError::Fit)?,
    };
    let mut lower = Region::new(BitSetId::Src, v);
    fill_srcs(&mut lower, &src_ops, 0).map_err(BuildError::Fit)?;

    let hv = super::pack::header_variant_of(ALUTYPE_MAIN).ok_or(BuildError::Unsupported("no main header variant"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let oporg =
        super::pack::oporg_for(&[PH_BACKEND]).ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hdr.set("oporg", oporg).map_err(|_| BuildError::Unsupported("oporg does not fit the header"))?;
    hdr.set("rpt", rpt).map_err(|_| BuildError::Unsupported("the repeat count does not fit the header"))?;
    if end {
        hdr.set("end", 1).map_err(|_| BuildError::Unsupported("no end bit in the header"))?;
    }

    /* The upper section is present whenever the lower one is, and the ISS
     * whenever the group is main-ALU with operands. Both are empty here, and
     * both are what the corpus measurement says, not what looks tidy. */
    let upper = match super::pack::upper_present(true) {
        true => Some(Region::new(BitSetId::Src, narrowest_upper_src()?)),
        false => None,
    };
    let mut iss = super::pack::iss_present(ALUTYPE_MAIN, true).then(|| Region::new(BitSetId::Iss, 0));
    /* pco's `group_map(O_UVSW_WRITE)` sets `is[4] = fte`: the write takes
     * its data through the feed-through, and every one of the corpus's 796
     * `uvsw.write` groups carries it. Left at `ft0` the group decodes, and
     * writes whatever phase 0 left behind. */
    if iss4_fte {
        let r = iss.as_mut().ok_or(BuildError::Unsupported("no ISS section to route the write through"))?;
        r.set("is4", IS4_FTE).map_err(|_| BuildError::Unsupported("is4 does not take fte"))?;
    }

    let parts = Parts { alutype: ALUTYPE_MAIN, instrs, lower: Some(lower), upper, iss, dst: None, align_pad };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// `fte` in the `is4_sel` enum (`ft0 ft1 ft2 fte`).
const IS4_FTE: u32 = 3;

/// One output dword the epilogue writes: where it comes from and where it
/// goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutWrite {
    pub bank: super::sel::Bank,
    pub reg: u16,
    /// The dword of the vertex output this lands in: `uvsw.write`'s
    /// `imm_addr`.
    pub addr: u8,
}

/// The files `uvsw.write` is seen reading in the corpus: temp 683, vtxin 106,
/// special 7 of 796. A write from any other file (a uniform) takes a move
/// first, which `prepare` inserts.
pub const UVSW_WRITE_BANKS: &[super::sel::Bank] =
    &[super::sel::Bank::Temp, super::sel::Bank::VtxIn, super::sel::Bank::Special];

/// The output writes a vertex shader's outputs ask for, with their addresses.
///
/// `gl_Position` is dwords 0..3 and the varyings follow in the order the
/// module declares them, at the offsets [`vertex_output_offsets`] gives --
/// the same function the record's `vtxout_off` comes from, so the two cannot
/// disagree. A component is written from the register `assignment` gave its
/// value, or from the fixed register selection named.
///
/// `gl_PointSize` follows the varyings. Refuses a fragment output.
pub fn vertex_output_writes(
    outputs: &[(crate::shaderc::online::ir::IoSlot, [Option<super::sel::SelSrc>; 4])],
    assignment: &[u16],
    io_out: &[crate::shaderc::online::ir::IoDecl<'_>],
    out: &mut [OutWrite],
) -> Result<usize, BuildError> {
    use super::sel::{Bank, SelSrc};
    use crate::shaderc::online::ir::{IoKind, IoSlot};
    let mut widths = [0u8; 16];
    let mut interps = [0u8; 16];
    let mut locs = [0u8; 16];
    let mut nv = 0usize;
    for io in io_out.iter().filter(|io| io.kind == IoKind::Varying) {
        if nv >= widths.len() {
            return Err(BuildError::Unsupported("more varyings than the record holds"));
        }
        widths[nv] = io.components;
        interps[nv] = io.interp;
        locs[nv] = io.location;
        nv += 1;
    }
    let mut offs = [0u8; 16];
    let psize_at = vertex_output_offsets(&widths[..nv], &interps[..nv], &mut offs)?;

    let mut n = 0usize;
    for (slot, comps) in outputs {
        let base = match slot {
            IoSlot::Position => 0u8,
            IoSlot::Varying(loc) => {
                let i = locs[..nv]
                    .iter()
                    .position(|l| l == loc)
                    .ok_or(BuildError::Unsupported("an output varying the module does not declare"))?;
                offs[i]
            }
            IoSlot::FragColor(_) | IoSlot::FragDepth => {
                return Err(BuildError::Unsupported("a fragment output: the pixel output write is not built yet"))
            }
            /* After the varyings: pco's record for a shader writing it puts
             * `vs_psize_off` at 4 + the varyings' dwords (measured with
             * pco_shc: 6 behind one vec2), the dword the record states. */
            IoSlot::PointSize => psize_at,
            _ => return Err(BuildError::Unsupported("an output slot a vertex shader does not write")),
        };
        for (lane, c) in comps.iter().enumerate() {
            let Some(c) = c else { continue };
            let (bank, reg) = match *c {
                SelSrc::Virtual { v, mods } if mods == Mods::default() => {
                    let r = *assignment
                        .get(v.0 as usize)
                        .ok_or(BuildError::Unsupported("an output value the allocator did not place"))?;
                    if r == super::alloc::UNASSIGNED {
                        return Err(BuildError::Unsupported("an output value the allocator did not place"));
                    }
                    (Bank::Temp, r)
                }
                SelSrc::Fixed { bank, reg, mods } if mods == Mods::default() && UVSW_WRITE_BANKS.contains(&bank) => {
                    (bank, reg)
                }
                _ => return Err(BuildError::Unsupported(OUTPUT_NEEDS_A_MOVE)),
            };
            if n >= out.len() {
                return Err(BuildError::Unsupported("more output components than the caller made room for"));
            }
            out[n] = OutWrite { bank, reg, addr: base + lane as u8 };
            n += 1;
        }
    }
    Ok(n)
}

/// An output component the write cannot take as it stands: a modifier, an
/// immediate, or a file `uvsw.write` is never seen reading. `prepare` moves
/// these into a temporary first, so reaching this is a caller that skipped it.
pub const OUTPUT_NEEDS_A_MOVE: &str = "an output component that needs a move first (prepare was not run)";

/// The output-write epilogue: what a vertex shader does after its arithmetic.
///
/// Each write names its source register and the output dword it lands in.
/// `uvsw.write` carries the dword in `imm_addr`, and a group with repeat
/// count `n` writes `n` consecutive registers to `n` consecutive dwords, so
/// a run consecutive in both becomes one group exactly as the reference
/// compiler emits it (`texturefill.batchable.vert` ends `uvsw.write` rpt 2
/// from Temp0 at `imm_addr` 4, `uvsw.write` rpt 4 from Temp2 at 0,
/// `uvsw.emit.endtask` with `end`).
///
/// This file used to read the opposite: that the write carried no
/// destination and an implicit pointer advanced by the repeat count. It
/// never set `imm_addr`, so every component went to dword 0, and it wrote in
/// selection's output order, which puts a varying before `gl_Position`.
/// pvrgpu-platform's corpus measurement (no shader's `(imm_addr, rpt)` spans
/// ever overlap; `roundtrip::uvsw_writes_immediate_address_never_overlaps`)
/// and pco's `encode_map(O_UVSW_WRITE)` (`imm_addr` from the operation's
/// second source) say it is an address.
///
/// Returns how many groups were written into `out`; `out` takes `Option`
/// slots so the caller need not invent a placeholder group to fill it with.
pub fn build_output_writes(writes: &[OutWrite], out: &mut [Option<Group>]) -> Result<usize, BuildError> {
    let cap = max_repeat();
    let mut n = 0usize;
    let mut i = 0usize;
    while i < writes.len() {
        let w = writes[i];
        /* A run is consecutive in BOTH the register and the address: the
         * repeat advances the two together. */
        let mut run = 1u32;
        while (i + run as usize) < writes.len() && run < cap {
            let x = writes[i + run as usize];
            if x.bank != w.bank || x.reg as u32 != w.reg as u32 + run || x.addr as u32 != w.addr as u32 + run {
                break;
            }
            run += 1;
        }
        if n >= out.len() {
            return Err(BuildError::Unsupported("more output-write groups than the caller made room for"));
        }
        /* `dsel = w0` is the encoding's zero and pco's constant. */
        out[n] = Some(backend_group(
            "uvsw.write",
            w.bank.hw(),
            w.reg as u32,
            run,
            false,
            0,
            &[("imm_addr", w.addr as u32)],
            true,
        )?);
        n += 1;
        i += run as usize;
    }
    if n >= out.len() {
        return Err(BuildError::Unsupported("no room for the end-of-task group"));
    }
    /* The task must end even when the shader wrote nothing. */
    out[n] = Some(end_task_group(0)?);
    Ok(n + 1)
}

/// The group that ends the task, optionally carrying the shader's closing
/// alignment block.
///
/// `align_pad` is the block's size in bytes. It is a trailer inside this
/// group -- the marker `0xf0 | words` followed by `0xff` filler -- and the
/// group's own length covers it, which is why it is chosen here rather than
/// appended afterwards.
pub fn end_task_group(align_pad: u8) -> Result<Group, BuildError> {
    /* Special register 0, the source the reference compiler gives it. */
    const BANK_SPECIAL: u32 = 0;
    backend_group("uvsw.emit.endtask", BANK_SPECIAL, 0, 1, true, align_pad, &[], false)
}

/// The boundary a shader's closing alignment block leaves it on.
///
/// Measured over the offline table: all 1,518 alignment blocks in it leave
/// the shader at a multiple of 8 bytes (763 of them also at a multiple of 16,
/// which is why 16 would have fitted the data and been wrong). 1,493 have
/// further bytes after them, so a blob holds several programs and the block
/// is what separates them.
pub const SHADER_ALIGN: usize = 8;

/// Add `pad` bytes of alignment block to a closed group, keeping its header
/// length true. The block is a trailer inside the group (`0xf0 | words`, then
/// `0xff` filler), counted by `length` and by nothing else -- not by `da`.
pub fn pad_group(g: &mut Group, pad: usize) -> Result<(), BuildError> {
    if pad == 0 {
        return Ok(());
    }
    let total = g.total + pad;
    if !total.is_multiple_of(2) || total / 2 > 16 {
        return Err(BuildError::Group(NoGroup::TooLong { bytes: total }));
    }
    let ap = g.align_pad as usize + pad;
    g.align_pad = u8::try_from(ap).map_err(|_| BuildError::Group(NoGroup::TooLong { bytes: total }))?;
    g.total = total;
    g.hdr.set("length", (total / 2) as u32).map_err(|_| BuildError::Group(NoGroup::TooLong { bytes: total }))?;
    Ok(())
}

/// The arithmetic of a program, group by group, with its loops placed: a
/// [`SelOp::LoopBegin`](super::sel::SelOp::LoopBegin) is the address of the
/// group after it and emits nothing; a
/// [`SelOp::LoopEnd`](super::sel::SelOp::LoopEnd) is `br if(p0)` back to it.
///
/// The one place a branch is made, and it holds the rule that makes a plain
/// branch sound here: the back edge must directly follow its loop's own
/// `LoopTest`, the only group that sets p0 for it (`select::verify_loops`
/// checks that the test reads the loop's counter). Anything else is refused.
/// The other branches are forward, past an arm of a branched `if` that no
/// instance runs (`bpred = allp`, [`super::cf::branch_none_active`]), where
/// the mask decides and every instance agrees by construction.
struct Arithmetic<'s> {
    groups: &'s [super::sel::SelGroup],
    next: usize,
    /// Where each open loop's body starts, in bytes from the program's start.
    open: [usize; 4],
    depth: usize,
    /// The group just built was a `LoopTest`.
    after_test: bool,
    /// The group just built entered a branched `if` (`CndIf`).
    after_cnd_if: bool,
    /// Inside an `if` arm ([`SelOp::Pred`](super::sel::SelOp::Pred)): the
    /// arm, and whether the `if` is branched. A predicated arm's groups take
    /// its `cc` until the next marker; a branched arm's take none.
    arm: Option<(super::sel::PredArm, bool)>,
    /// A group built ahead of its turn: the branch past the else arm, after
    /// the `cndef` that switched the mask to it.
    pending: Option<Group>,
}

impl<'s> Arithmetic<'s> {
    fn new(groups: &'s [super::sel::SelGroup]) -> Self {
        Arithmetic {
            groups,
            next: 0,
            open: [0; 4],
            depth: 0,
            after_test: false,
            after_cnd_if: false,
            arm: None,
            pending: None,
        }
    }

    /// The branch past the arm that starts at group `from`, when it has
    /// groups: `br` taken when no instance is active
    /// ([`super::cf::branch_none_active`]), landing on the group after the
    /// arm's last, the arm's own groups measured by building them here
    /// (each is built again in its turn; an arm holds no marker, so neither
    /// build depends on where it lands).
    fn skip_arm(&self, from: usize) -> Result<Option<Group>, BuildError> {
        use super::sel::SelOp;
        let mut bytes = 0usize;
        for g in &self.groups[from..] {
            if matches!(g.op, SelOp::Pred(_)) {
                break;
            }
            bytes += build_group(g)?.total;
        }
        if bytes == 0 {
            return Ok(None);
        }
        /* The branch's own length does not depend on the offset's value
         * (a fixed field): built once to measure, then with the offset. */
        let probe = super::cf::branch_none_active(bytes as i64)?;
        let br = super::cf::branch_none_active((bytes + probe.total) as i64)?;
        if br.total != probe.total {
            return Err(BuildError::Unsupported("a branch whose length depends on its offset"));
        }
        Ok(Some(br))
    }

    /// The next group, to be placed at byte `at`; `None` after the last.
    fn group(&mut self, at: usize) -> Result<Option<Group>, BuildError> {
        use super::sel::{MachineOp, PredArm, SelOp};
        if let Some(br) = self.pending.take() {
            return Ok(Some(br));
        }
        while let Some(g) = self.groups.get(self.next) {
            self.next += 1;
            match g.op {
                /* An `if` with arithmetic arms: the then arm, the else arm
                 * after it, then the end; never nested, never across a loop
                 * edge, and no group in an arm touches p0 or the mask
                 * (selection's `predicable` holds that; this refuses what
                 * slips past). Predicated (pco_cf.c lower_if_pred_exec): the
                 * then arm right after the condition's tstz.p0. Branched
                 * (lower_if_cond_exec): right after the `cndst` that keeps
                 * its instances, a `br` past each arm with groups, the else
                 * arm's after the `cndef` right after its marker, and the
                 * `cndend` right after the end. */
                SelOp::Pred(m) => {
                    let branched = match (self.arm, m) {
                        (None, PredArm::Then) if self.after_test => false,
                        (None, PredArm::Then) if self.after_cnd_if => true,
                        (Some((PredArm::Then, b)), PredArm::Else) => b,
                        (Some((PredArm::Then | PredArm::Else, b)), PredArm::End) => b,
                        _ => return Err(BuildError::Unsupported("an `if` arm marker out of order")),
                    };
                    self.arm = (m != PredArm::End).then_some((m, branched));
                    self.after_test = false;
                    self.after_cnd_if = false;
                    if !branched {
                        continue;
                    }
                    let next = self.groups.get(self.next).map(|x| x.op);
                    match m {
                        PredArm::Then => {
                            if let Some(br) = self.skip_arm(self.next)? {
                                return Ok(Some(br));
                            }
                        }
                        /* the mask switched to the else arm, then the branch
                         * past it; an `if` with an empty arm has no `cndef` */
                        PredArm::Else => {
                            let from = self.next + usize::from(next == Some(SelOp::Machine(MachineOp::CndElse)));
                            let br = self.skip_arm(from)?;
                            if from > self.next {
                                let ef = build_group(&self.groups[self.next])?;
                                self.next += 1;
                                self.pending = br;
                                return Ok(Some(ef));
                            }
                            if let Some(br) = br {
                                return Ok(Some(br));
                            }
                        }
                        PredArm::End => {
                            if next != Some(SelOp::Machine(MachineOp::CndIfEnd)) {
                                return Err(BuildError::Unsupported("a branched `if` that does not leave its mask"));
                            }
                        }
                    }
                }
                SelOp::LoopBegin
                | SelOp::LoopEnd
                | SelOp::Machine(MachineOp::LoopTest | MachineOp::Fsinc(_) | MachineOp::PselTrig)
                    if self.arm.is_some() =>
                {
                    return Err(BuildError::Unsupported("a loop edge or a p0 user inside an `if` arm"));
                }
                SelOp::Machine(m) if m.is_cnd() && self.arm.is_some() => {
                    return Err(BuildError::Unsupported("an execution-mask operation inside an `if` arm"));
                }
                SelOp::LoopBegin => {
                    let slot =
                        self.open.get_mut(self.depth).ok_or(BuildError::Unsupported("loops nested too deeply"))?;
                    *slot = at;
                    self.depth += 1;
                    self.after_test = false;
                }
                SelOp::LoopEnd => {
                    if !self.after_test {
                        return Err(BuildError::Unsupported("a back edge that does not follow its loop's test"));
                    }
                    self.depth =
                        self.depth.checked_sub(1).ok_or(BuildError::Unsupported("a back edge with no loop"))?;
                    self.after_test = false;
                    return super::fragment::branch_back(self.open[self.depth] as i64 - at as i64).map(Some);
                }
                SelOp::Machine(MachineOp::CndIf)
                    if self.groups.get(self.next).map(|x| x.op) != Some(SelOp::Pred(PredArm::Then)) =>
                {
                    return Err(BuildError::Unsupported("a branched `if`'s mask with no arm after it"));
                }
                _ => {
                    /* The p0 the back edge reads: the loop's test, or under
                     * the execution mask the loop's end (`cndlt`). */
                    self.after_test = matches!(g.op, SelOp::Machine(MachineOp::LoopTest | MachineOp::CndLoopEnd));
                    self.after_cnd_if = matches!(g.op, SelOp::Machine(MachineOp::CndIf));
                    let mut built = build_group(g)?;
                    if let Some((arm, false)) = self.arm {
                        /* the then arm runs where the condition held: p0, set
                         * by the tstz where it is zero, is clear there */
                        let cc = match arm {
                            PredArm::Then => "if(!p0)",
                            _ => "if(p0)",
                        };
                        let v = super::shapes::value(super::isa::BitSetId::IgrpHdr, "cc", cc)?;
                        built.hdr.set("cc", v).map_err(|_| {
                            BuildError::Unsupported("a group whose header takes no execution condition")
                        })?;
                    }
                    return Ok(Some(built));
                }
            }
        }
        if self.depth != 0 {
            return Err(BuildError::Unsupported("a loop with no back edge"));
        }
        if self.arm.is_some() {
            return Err(BuildError::Unsupported("a predicated `if` arm with no end"));
        }
        Ok(None)
    }
}

/// Encode a program's groups with pco's closing rule, from
/// `calc_align_padding` in Mesa `src/imagination/pco/pco_group_instrs.c`
/// (MIT, read and transcribed), `ROGUE_ICACHE_ALIGN = 8`: the LAST group's
/// length is padded to a multiple of 8, and its START is brought to a
/// multiple of 8 by padding the group before it.
///
/// This file used to do only the first half, padding inside the last group
/// so that the shader ENDED aligned; pco's `gl_Position = a_pos` puts six
/// bytes in the `uvsw.write` and none in the end of task, and ours put six
/// in the end of task, starting it at byte 10.
///
/// `next` yields the groups in order; the last one is held back until the
/// padding before it is known. Returns the bytes written.
fn encode_closed(
    mut next: impl FnMut() -> Result<Option<Group>, BuildError>,
    buf: &mut [u8],
) -> Result<usize, BuildError> {
    let enc = |g: &Group, buf: &mut [u8], at: usize| {
        super::encode::encode_group(g, buf, at)
            .map_err(|_| BuildError::Unsupported("the shader does not fit the buffer"))
    };
    let mut off = 0usize;
    /* The group before the one currently held, and where it starts. */
    let mut prev: Option<(usize, Group)> = None;
    let mut held: Option<Group> = None;
    while let Some(g) = next()? {
        if let Some(h) = held.take() {
            if let Some((at, p)) = prev.take() {
                off = at + enc(&p, buf, at)?;
            }
            prev = Some((off, h));
            off += h.total;
        }
        held = Some(g);
    }
    let mut last = held.ok_or(BuildError::Unsupported("a shader with no groups"))?;
    /* The start of the last group: pad the one before it. */
    if let Some((at, mut p)) = prev.take() {
        let start = at + p.total;
        let lead = (SHADER_ALIGN - start % SHADER_ALIGN) % SHADER_ALIGN;
        pad_group(&mut p, lead)?;
        off = at + enc(&p, buf, at)?;
    }
    /* Its length. */
    let tail = (SHADER_ALIGN - last.total % SHADER_ALIGN) % SHADER_ALIGN;
    pad_group(&mut last, tail)?;
    off += enc(&last, buf, off)?;
    debug_assert_eq!(off % SHADER_ALIGN, 0);
    Ok(off)
}

/// A whole vertex shader: the arithmetic, the output writes and the end of
/// task, closed by [`encode_closed`]'s alignment rule.
///
/// Returns the bytes written into `buf`.
///
/// `writes` are the output components and their addresses, from
/// [`vertex_output_writes`]; see [`build_output_writes`].
pub fn build_shader(
    arithmetic: &[super::sel::SelGroup],
    writes: &[OutWrite],
    buf: &mut [u8],
) -> Result<usize, BuildError> {
    let mut epi: [Option<Group>; 32] = [None; 32];
    let n = build_output_writes(writes, &mut epi)?;
    let mut arith = Arithmetic::new(arithmetic);
    let mut at = 0usize;
    let mut j = 0usize;
    encode_closed(
        || {
            if let Some(g) = arith.group(at)? {
                at += g.total;
                return Ok(Some(g));
            }
            if j < n {
                j += 1;
                return Ok(epi[j - 1]);
            }
            Ok(None)
        },
        buf,
    )
}

/// The value `name` has in the generated enum `enum_name`. Looked up rather
/// than written as a number so a reader can check it against the table.
fn enum_val(enum_name: &str, name: &str) -> Result<u32, BuildError> {
    super::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == enum_name)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .ok_or(BuildError::Unsupported("an enum value the ISA tables do not have"))
}

/// The pixel output register the first colour lands in: special register 32,
/// `PCO_SR_PIXOUT0` (Mesa `pco_map.h.py`, `pco_map_reg_index`, a PIXOUT class
/// register maps to `val + PCO_SR_PIXOUT0` in the special bank). Every
/// fragment blob of the offline table ends by writing it.
pub const PIXOUT0: u16 = 32;

/// Four float colour components in `first..first+3` packed to one RGBA8
/// dword in `dst`: pco's `group_map(O_PCK)` (Mesa `pco_map.py`, MIT, read and
/// transcribed) with `pck_format = u8888`, `scale` set (0..1 to 0..255) and a
/// repeat of 4 walking the source.
///
/// The operation is phase 2's `pck`, which reads its operand through the ISS:
/// `is0 = s2` puts the lower section's third slot on the internal bus, `is3 =
/// fte` hands it to the packer, and `is4 = ft2` routes the packer's result to
/// the write port. `s0` and `s1` name special register 0 as placeholders, as
/// pco's do: the operation's only source is `s2`.
pub fn build_pck_u8888(first: u16, dst: u16) -> Result<Group, BuildError> {
    build_pck("u8888", 1, 4, first, dst)
}

/// [`build_pck_u8888`]'s group in any format: `rpt` components from `first`
/// packed into `dst` -- `f16f16` repeat 2 for two halves of an RGBA16F
/// target's dword, repeat 1 for an R16F one (pco's float-class variants).
pub fn build_pck(format: &str, scale: u32, rpt: u32, first: u16, dst: u16) -> Result<Group, BuildError> {
    use super::isa::PH_2_PCK;
    let extra = [("pck_format", enum_val("pck_format", format)?), ("scale", scale), ("rtz", 0), ("prog", 0)];
    let r = instr_region("pck", PH_2_PCK, &[Mods::default(); 3], &extra).map_err(BuildError::Form)?;
    let slot = super::pack::slot_of(PH_2_PCK)
        .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: PH_2_PCK }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((PH_2_PCK, r));

    const SPECIAL: u32 = 0;
    const TEMP: u32 = 1;
    let ops = [
        Some(Operand { bank: SPECIAL, index: 0 }),
        Some(Operand { bank: SPECIAL, index: 0 }),
        Some(Operand { bank: TEMP, index: first as u32 }),
        None,
        None,
        None,
    ];
    let mux = enum_val("is0_sel", "s2")?;
    let v = choose_src_variant(&ops, mux).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, v);
    fill_srcs(&mut lower, &ops, mux).map_err(BuildError::Fit)?;
    let upper = Some(Region::new(BitSetId::Src, narrowest_upper_src()?));
    let mut iss = Region::new(BitSetId::Iss, 0);
    iss.set("is3", enum_val("is3_sel", "fte")?).map_err(|_| BuildError::Unsupported("is3 does not take fte"))?;
    iss.set("is4", enum_val("is4_sel", "ft2")?).map_err(|_| BuildError::Unsupported("is4 does not take ft2"))?;

    let dst_ops = [Some(Operand { bank: TEMP, index: dst as u32 }), None];
    let dv = super::legalise::choose_dst_variant(&dst_ops).map_err(BuildError::Fit)?;
    let mut d = Region::new(BitSetId::Dst, dv);
    super::legalise::fill_dsts(&mut d, &dst_ops).map_err(BuildError::Fit)?;

    let hv = super::pack::header_variant_of(ALUTYPE_MAIN).ok_or(BuildError::Unsupported("no main header variant"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let oporg =
        super::pack::oporg_for(&[PH_2_PCK]).ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hdr.set("oporg", oporg).map_err(|_| BuildError::Unsupported("oporg does not fit the header"))?;
    hdr.set("w0p", 1).map_err(|_| BuildError::Unsupported("no w0p in the header"))?;
    hdr.set("rpt", rpt).map_err(|_| BuildError::Unsupported("the repeat count does not fit the header"))?;

    let parts =
        Parts { alutype: ALUTYPE_MAIN, instrs, lower: Some(lower), upper, iss: Some(iss), dst: Some(d), align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// The inverse of [`build_pck_u8888`]: the RGBA8 dword in `src` unpacked to
/// four floats in `dst..dst+3`, 0..255 to 0..1 -- pco's `unpck` of format
/// u8888 with `scale_rtz` and a repeat of 4, as its blend variants unpack
/// the colour already in the pixel.
pub fn build_unpck_u8888(src: u16, dst: u16) -> Result<Group, BuildError> {
    build_unpck("u8888", 1, 4, src, dst)
}

/// [`build_unpck_u8888`]'s group in any format: `f16f16` with no scale,
/// repeat 2 (or 1 for R16F), as pco's float-class blend variants unpack the
/// pixel.
pub fn build_unpck(format: &str, scale_rtz: u32, rpt: u32, src: u16, dst: u16) -> Result<Group, BuildError> {
    use super::isa::PH_0;
    let extra = [
        ("elem", enum_val("upck_elem", "e0")?),
        ("scale_rtz", scale_rtz),
        ("pck_format", enum_val("pck_format", format)?),
    ];
    let r = instr_region("unpck", PH_0, &[Mods::default(); 3], &extra).map_err(BuildError::Form)?;
    let slot =
        super::pack::slot_of(PH_0).ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: PH_0 }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((PH_0, r));
    const TEMP: u32 = 1;
    let ops = [Some(Operand { bank: TEMP, index: src as u32 }), None, None, None, None, None];
    let v = choose_src_variant(&ops, 0).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, v);
    fill_srcs(&mut lower, &ops, 0).map_err(BuildError::Fit)?;
    let upper = Some(Region::new(BitSetId::Src, narrowest_upper_src()?));
    let iss = Some(Region::new(BitSetId::Iss, 0));
    let dst_ops = [Some(Operand { bank: TEMP, index: dst as u32 }), None];
    let dv = super::legalise::choose_dst_variant(&dst_ops).map_err(BuildError::Fit)?;
    let mut d = Region::new(BitSetId::Dst, dv);
    super::legalise::fill_dsts(&mut d, &dst_ops).map_err(BuildError::Fit)?;
    let hv = super::pack::header_variant_of(ALUTYPE_MAIN).ok_or(BuildError::Unsupported("no main header variant"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let oporg = super::pack::oporg_for(&[PH_0]).ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hdr.set("oporg", oporg).map_err(|_| BuildError::Unsupported("oporg does not fit the header"))?;
    hdr.set("w0p", 1).map_err(|_| BuildError::Unsupported("no w0p in the header"))?;
    hdr.set("rpt", rpt).map_err(|_| BuildError::Unsupported("the repeat count does not fit the header"))?;
    let parts = Parts { alutype: ALUTYPE_MAIN, instrs, lower: Some(lower), upper, iss, dst: Some(d), align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// The colour already in the pixel, read into temporary `dst`: a move from
/// the pixel output register with the output-lock check (`olchk`), as pco's
/// blend variants read it (`ref_needs_olchk`).
pub fn build_pixout_read(dst: u16) -> Result<Group, BuildError> {
    build_pixout_read_from(0, dst)
}

/// [`build_pixout_read`] of pixel output register `index` (an RGBA16F
/// target spans two).
pub fn build_pixout_read_from(index: u16, dst: u16) -> Result<Group, BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    let mut g = build_group(&SelGroup {
        op: SelOp::Copy,
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dst }),
        srcs: [Some(SelSrc::Fixed { bank: Bank::Special, reg: PIXOUT0 + index, mods: Mods::default() }), None, None],
    })?;
    g.hdr.set("olchk", 1).map_err(|_| BuildError::Unsupported("no olchk in the header"))?;
    Ok(g)
}

/// The fragment shader's last group: the packed colour moved to the pixel
/// output, with `olchk` (the output-lock check pco sets on a pixel output
/// write, `pco_legalize.c` `ref_needs_olchk`) and `end`.
pub fn build_pixout_write(src: u16) -> Result<Group, BuildError> {
    build_pixout_write_to(0, src, true)
}

/// [`build_pixout_write`] to pixel output register `index`, ending the
/// program or not: an RGBA16F target's first dword goes to pixout0 without
/// `end`, its second to pixout1 with it (pco's f16x4 variants).
pub fn build_pixout_write_to(index: u16, src: u16, end: bool) -> Result<Group, BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    let mut g = build_group(&SelGroup {
        op: SelOp::Copy,
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Special, reg: PIXOUT0 + index }),
        srcs: [Some(SelSrc::Fixed { bank: Bank::Temp, reg: src, mods: Mods::default() }), None, None],
    })?;
    g.hdr.set("olchk", 1).map_err(|_| BuildError::Unsupported("no olchk in the header"))?;
    if end {
        g.hdr.set("end", 1).map_err(|_| BuildError::Unsupported("no end bit in the header"))?;
    }
    Ok(g)
}

/// A whole fragment shader writing one RGBA8 colour: the arithmetic, a move
/// of each colour component into four consecutive temporaries from `free`
/// up (above everything the allocator placed, so nothing live is
/// overwritten), the pack into `free + 4`, and the pixel output write.
/// Returns the bytes written and the temporaries the program declares.
///
/// The moves are unconditional: the allocator has no way to ask for four
/// consecutive registers, and pco's own program for a constant colour makes
/// the same four moves.
pub fn build_fragment_shader(
    iterated: &[super::fragment::Iterated],
    arithmetic: &[super::sel::SelGroup],
    colour: [(super::sel::Bank, u16); 4],
    discard: Option<u16>,
    free: u16,
    buf: &mut [u8],
) -> Result<(usize, u16), BuildError> {
    build_fragment_shader_blended(iterated, arithmetic, colour, discard, free, None, buf)
}

/// [`build_fragment_shader`] for a blend variant: the blend lowered between
/// the colour and its pack (`blend::step`), reading the constants from the
/// shared register given when a factor needs them.
pub fn build_fragment_shader_blended(
    iterated: &[super::fragment::Iterated],
    arithmetic: &[super::sel::SelGroup],
    colour: [(super::sel::Bank, u16); 4],
    discard: Option<u16>,
    free: u16,
    blend: Option<(super::blend::Blend, Option<u16>, Option<u16>)>,
    buf: &mut [u8],
) -> Result<(usize, u16), BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    /* Already four consecutive temporaries -- a varying passed straight
     * through -- and the pack reads them where they are, as pco's does. */
    let in_place = colour.iter().enumerate().all(|(k, (b, r))| *b == Bank::Temp && *r == colour[0].1 + k as u16);
    /* The render target's class: an RGBA16F target packs two dwords (the
     * second one temporary past the first), R16F one, R32F none. */
    use crate::shaderc::{SHC_RT_F16, SHC_RT_F16X4, SHC_RT_F32, SHC_RT_RGBA8};
    let class = blend.map_or(SHC_RT_RGBA8, |(b, _, _)| b.class);
    let wide = u16::from(class == SHC_RT_F16X4);
    let (first, packed, temps) = if in_place {
        (colour[0].1, free, free.checked_add(1 + wide))
    } else {
        (free, free + 4, free.checked_add(5 + wide))
    };
    let temps = temps.ok_or(BuildError::Unsupported("no temporaries left for the colour"))?;
    /* The blend works in temporaries above everything else. */
    let regs = blend.map(|(_, consts, a2one)| super::blend::Regs { colour: first, scratch: temps, consts, a2one });
    let temps = match regs {
        Some(_) => temps
            .checked_add(super::blend::Regs::SCRATCH)
            .ok_or(BuildError::Unsupported("no temporaries left for the blend"))?,
        None => temps,
    };
    let mut bstep = 0usize;
    let mut pro = Prologue::new(iterated, arithmetic);
    let mut k = if in_place { 4 } else { 0 };
    let mut tail = 0u8;
    let n = encode_closed(
        || {
            if let Some(g) = pro.group()? {
                return Ok(Some(g));
            }
            if k < 4 {
                let (bank, reg) = colour[k];
                k += 1;
                return build_group(&SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::F32,
                    dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: free + k as u16 - 1 }),
                    srcs: [Some(SelSrc::Fixed { bank, reg, mods: Mods::default() }), None, None],
                })
                .map(Some);
            }
            if let (Some((b, _, _)), Some(r)) = (blend, regs) {
                if let Some(g) = super::blend::step(&b, &r, bstep)? {
                    bstep += 1;
                    return Ok(Some(g));
                }
            }
            tail += 1;
            /* The pack in the target's format; in a shader that discards,
             * the kill flag tested into p0, the pass feedback under p0 and
             * the wait for it, as pco orders them; then the pixel write(s),
             * the last one ending the program. */
            const TEMP: u32 = 1;
            let packs: u8 = match class {
                SHC_RT_F16X4 => 2,
                SHC_RT_F32 => 0,
                _ => 1,
            };
            let kills: u8 = if discard.is_some() { 3 } else { 0 };
            let t = tail;
            if t <= packs {
                return match (class, t) {
                    (SHC_RT_F16X4, 1) => build_pck("f16f16", 0, 2, first, packed),
                    (SHC_RT_F16X4, _) => build_pck("f16f16", 0, 2, first + 2, packed + 1),
                    (SHC_RT_F16, _) => build_pck("f16f16", 0, 1, first, packed),
                    _ => build_pck_u8888(first, packed),
                }
                .map(Some);
            }
            if t <= packs + kills {
                let flag = discard.ok_or(BuildError::Unsupported("a kill test with no flag"))?;
                return match t - packs {
                    1 => super::shapes::build_opt(
                        &super::shapes::KILL_TEST,
                        &[Some(Operand { bank: TEMP, index: flag as u32 }), None, None],
                        None,
                    ),
                    2 => super::shapes::build_opt(&super::shapes::ALPHA_FEEDBACK, &[None; 3], None),
                    _ => super::fragment::wait_data(),
                }
                .map(Some);
            }
            match (class, t - packs - kills) {
                (SHC_RT_F16X4, 1) => build_pixout_write_to(0, packed, false).map(Some),
                (SHC_RT_F16X4, 2) => build_pixout_write_to(1, packed + 1, true).map(Some),
                (SHC_RT_F32, 1) => build_pixout_write_to(0, first, true).map(Some),
                (_, 1) => build_pixout_write(packed).map(Some),
                _ => Ok(None),
            }
        },
        buf,
    )?;
    Ok((n, temps))
}

/// A fragment shader's groups before its colour: the iterated varyings
/// first, then the flat ones' moves, as pco orders them, then the
/// arithmetic.
struct Prologue<'s> {
    iterated: &'s [super::fragment::Iterated],
    n_smooth: usize,
    it: usize,
    /// The next flat varying (among the flat ones) and its component.
    flat: (usize, u8),
    arith: Arithmetic<'s>,
    /// Bytes placed so far, for the loops' back edges.
    at: usize,
}

impl<'s> Prologue<'s> {
    fn new(iterated: &'s [super::fragment::Iterated], arithmetic: &'s [super::sel::SelGroup]) -> Self {
        use super::fragment::IterMode;
        let n_smooth = iterated.iter().filter(|v| v.mode != IterMode::Flat).count();
        Prologue { iterated, n_smooth, it: 0, flat: (0, 0), arith: Arithmetic::new(arithmetic), at: 0 }
    }

    /// The flat varying the next move is for, past any with no components.
    fn next_flat(&mut self) -> Option<super::fragment::Iterated> {
        use super::fragment::IterMode;
        loop {
            let v = self.iterated.iter().filter(|v| v.mode == IterMode::Flat).nth(self.flat.0).copied()?;
            if self.flat.1 < v.count {
                return Some(v);
            }
            self.flat = (self.flat.0 + 1, 0);
        }
    }

    /// The next group; `None` once the arithmetic is done.
    fn group(&mut self) -> Result<Option<Group>, BuildError> {
        use super::fragment::{iterate_group, IterMode, GROUPS_PER_VARYING};
        let g = if self.it < self.n_smooth * GROUPS_PER_VARYING {
            self.it += 1;
            let v = self
                .iterated
                .iter()
                .filter(|v| v.mode != IterMode::Flat)
                .nth((self.it - 1) / GROUPS_PER_VARYING)
                .copied()
                .ok_or(BuildError::Unsupported("a varying miscounted"))?;
            iterate_group(v, (self.it - 1) % GROUPS_PER_VARYING)?
        } else if let Some(v) = self.next_flat() {
            let c = self.flat.1;
            self.flat = if c + 1 < v.count { (self.flat.0, c + 1) } else { (self.flat.0 + 1, 0) };
            super::fragment::flat_group(v, c)?
        } else {
            match self.arith.group(self.at)? {
                Some(g) => g,
                None => return Ok(None),
            }
        };
        self.at += g.total;
        Ok(Some(g))
    }
}

/// The most colour outputs [`build_fragment_shader_outputs`] writes: one
/// RGBA8 output per pixel output register, and a pixel has four.
pub const MAX_RGBA8_OUTPUTS: usize = 4;

/// A fragment shader writing several RGBA8 colours, none blended: output
/// (`index`, colour) is packed and written to pixel output register
/// `index`, which is where the end-of-tile program emits attachment
/// `index` from when every attachment is 8 bits per channel
/// (src/pvr/backend.rs: one register per 8-bit attachment, in order).
/// Each colour is moved into four consecutive temporaries and packed into
/// the fifth, above everything the allocator placed; the kill test and the
/// feedback of a shader that discards come after the packs, and the last
/// write ends the program. Returns the bytes written and the temporaries.
pub fn build_fragment_shader_outputs(
    iterated: &[super::fragment::Iterated],
    arithmetic: &[super::sel::SelGroup],
    outputs: &[(u16, [(super::sel::Bank, u16); 4])],
    discard: Option<u16>,
    free: u16,
    buf: &mut [u8],
) -> Result<(usize, u16), BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    if outputs.is_empty() || outputs.len() > MAX_RGBA8_OUTPUTS {
        return Err(BuildError::Unsupported("a colour output count outside 1..=4"));
    }
    if outputs.iter().any(|(i, _)| usize::from(*i) >= MAX_RGBA8_OUTPUTS) {
        return Err(BuildError::Unsupported("a colour output past the pixel's four output registers"));
    }
    let n = outputs.len() as u16;
    let temps = free.checked_add(5 * n).ok_or(BuildError::Unsupported("no temporaries left for the colours"))?;
    let base = |j: u16| free + 5 * j;
    let mut pro = Prologue::new(iterated, arithmetic);
    /* moves: output j component c at step 4j + c; then packs, kills, writes */
    let mut step = 0u16;
    let kills: u16 = if discard.is_some() { 3 } else { 0 };
    let len = encode_closed(
        || {
            if let Some(g) = pro.group()? {
                return Ok(Some(g));
            }
            let s = step;
            step += 1;
            if s < 4 * n {
                let (j, c) = (s / 4, s % 4);
                let (bank, reg) = outputs[j as usize].1[c as usize];
                return build_group(&SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::F32,
                    dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: base(j) + c }),
                    srcs: [Some(SelSrc::Fixed { bank, reg, mods: Mods::default() }), None, None],
                })
                .map(Some);
            }
            let s = s - 4 * n;
            if s < n {
                return build_pck_u8888(base(s), base(s) + 4).map(Some);
            }
            let s = s - n;
            if s < kills {
                const TEMP: u32 = 1;
                let flag = discard.ok_or(BuildError::Unsupported("a kill test with no flag"))?;
                return match s {
                    0 => super::shapes::build_opt(
                        &super::shapes::KILL_TEST,
                        &[Some(Operand { bank: TEMP, index: flag as u32 }), None, None],
                        None,
                    ),
                    1 => super::shapes::build_opt(&super::shapes::ALPHA_FEEDBACK, &[None; 3], None),
                    _ => super::fragment::wait_data(),
                }
                .map(Some);
            }
            let s = s - kills;
            if s < n {
                return build_pixout_write_to(outputs[s as usize].0, base(s) + 4, s + 1 == n).map(Some);
            }
            Ok(None)
        },
        buf,
    )?;
    Ok((len, temps))
}

/// A fragment shader writing one integer render target's packed dwords
/// (`chain::pack_int_output`; Mesa pco_nir_pvfio.c pack_to_format): dword
/// `k` is written to pixel output register `k` as it is, with no pack
/// instruction -- the bits are already the target's, and the PBE's integer
/// pack mode reads them from there (src/pvr/backend.rs `pbe_emit_words`).
/// A dword not already in a temporary is moved into one above everything
/// the allocator placed first; the kill test and the feedback of a shader
/// that discards come before the writes, and the last write ends the
/// program. Returns the bytes written and the temporaries.
pub fn build_fragment_shader_packed(
    iterated: &[super::fragment::Iterated],
    arithmetic: &[super::sel::SelGroup],
    dwords: &[(super::sel::Bank, u16)],
    discard: Option<u16>,
    free: u16,
    buf: &mut [u8],
) -> Result<(usize, u16), BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    if dwords.is_empty() || dwords.len() > MAX_RGBA8_OUTPUTS {
        return Err(BuildError::Unsupported("an integer output of other than 1..=4 dwords"));
    }
    let n = dwords.len() as u16;
    let temps = free.checked_add(n).ok_or(BuildError::Unsupported("no temporaries left for the colour"))?;
    /* where dword k is written from: its own temporary, or free + k */
    let src = |k: u16| match dwords[k as usize] {
        (Bank::Temp, r) => r,
        _ => free + k,
    };
    let mut pro = Prologue::new(iterated, arithmetic);
    let kills: u16 = if discard.is_some() { 3 } else { 0 };
    let mut step = 0u16;
    let len = encode_closed(
        || {
            if let Some(g) = pro.group()? {
                return Ok(Some(g));
            }
            loop {
                let s = step;
                step += 1;
                if s < n {
                    let (bank, reg) = dwords[s as usize];
                    if bank == Bank::Temp {
                        continue;
                    }
                    return build_group(&SelGroup {
                        op: SelOp::Copy,
                        ty: OpType::U32,
                        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: free + s }),
                        srcs: [Some(SelSrc::Fixed { bank, reg, mods: Mods::default() }), None, None],
                    })
                    .map(Some);
                }
                let s = s - n;
                if s < kills {
                    const TEMP: u32 = 1;
                    let flag = discard.ok_or(BuildError::Unsupported("a kill test with no flag"))?;
                    return match s {
                        0 => super::shapes::build_opt(
                            &super::shapes::KILL_TEST,
                            &[Some(Operand { bank: TEMP, index: flag as u32 }), None, None],
                            None,
                        ),
                        1 => super::shapes::build_opt(&super::shapes::ALPHA_FEEDBACK, &[None; 3], None),
                        _ => super::fragment::wait_data(),
                    }
                    .map(Some);
                }
                let s = s - kills;
                if s < n {
                    return build_pixout_write_to(s, src(s), s + 1 == n).map(Some);
                }
                return Ok(None);
            }
        },
        buf,
    )?;
    Ok((len, temps))
}

/// The background object's clear program for `dwords` output dwords: shared
/// register k moved to pixel output register k, the last move ending the
/// program. Mesa's pvr_uscgen_clear_attach with `.dword_count = dwords,
/// .offset = 0` compiles to this (the tests below hold it to Mesa's captures,
/// tests/reference/pvr_internal.rs); the driver's three programs are this
/// builder's output, which yakogl-precompile --internal writes into
/// src/pvr/internal.rs.
pub fn build_clear_attach(dwords: u16, buf: &mut [u8]) -> Result<usize, BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    if !(1..=4).contains(&dwords) {
        return Err(BuildError::Unsupported("a clear of other than 1..=4 output dwords"));
    }
    let mut k = 0u16;
    encode_closed(
        || {
            if k == dwords {
                return Ok(None);
            }
            k += 1;
            let mut g = build_group(&SelGroup {
                op: SelOp::Copy,
                ty: OpType::F32,
                dst: Some(SelDst::Fixed { bank: Bank::Special, reg: PIXOUT0 + k - 1 }),
                srcs: [Some(SelSrc::Fixed { bank: Bank::Shared, reg: k - 1, mods: Mods::default() }), None, None],
            })?;
            g.hdr.set("olchk", 1).map_err(|_| BuildError::Unsupported("no olchk in the header"))?;
            if k == dwords {
                g.hdr.set("end", 1).map_err(|_| BuildError::Unsupported("no end bit in the header"))?;
            }
            Ok(Some(g))
        },
        buf,
    )
}

/// Does this group end the program? A main or bitwise group says so in
/// `end`; a control `nop` in `miscctl` (pco's `group_map(O_NOP)`:
/// `miscctl = OM_END`).
pub fn ends_program(g: &Group) -> bool {
    if g.hdr.get("end").unwrap_or(0) != 0 {
        return true;
    }
    let nop = super::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == "ctrlop")
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == "nop"))
        .map(|(v, _)| *v);
    nop.is_some() && g.hdr.get("ctrlop") == nop && g.hdr.get("miscctl") == Some(1)
}

/// A fragment shader that writes nothing: pco's one control `nop` ending
/// the program, closed as every program is. Returns the bytes written.
pub fn build_empty_fragment_shader(buf: &mut [u8]) -> Result<usize, BuildError> {
    let mut done = false;
    encode_closed(
        || {
            if done {
                return Ok(None);
            }
            done = true;
            super::fragment::end_nop().map(Some)
        },
        buf,
    )
}

/// Walk a built shader back and say what it contains.
///
/// A shader is a composition of parts that were each validated separately,
/// and a composition of validated parts is not validated until the whole
/// round-trips: the lengths that chain the groups together are exactly what
/// per-group checks cannot see.
///
/// Returns (groups, bytes consumed, whether the last group carries `end`).
/// `None` if any group fails to decode, which for our own output is a defect
/// in this file rather than in the caller.
pub fn walk_shader(buf: &[u8]) -> Option<(usize, usize, bool)> {
    let mut off = 0usize;
    let mut n = 0usize;
    let mut ended = false;
    while off < buf.len() {
        let g = super::decode::decode_group(buf, off).ok()?;
        if g.total == 0 {
            return None;
        }
        ended = ends_program(&g);
        off += g.total;
        n += 1;
    }
    Some((n, off, ended))
}

/// `gl_Position` occupies the first four dwords of the vertex output.
pub const VTXOUT_POSITION_DWORDS: u8 = 4;

/// Where each varying sits in the vertex output, and how large that output is.
///
/// `components[i]` is varying `i`'s width; `offs[i]` receives its dword
/// offset. Returns `vs_output_size`, the total the record reports and the
/// link rule checks.
///
/// Measured over the offline table: of 214 vertex blobs, the 203 that have
/// varyings ALL place `gl_Position` at dwords 0..3 and then pack the varyings
/// in offset order with no holes -- 24 distinct shapes, 0 exceptions.
///
/// # Why this is the same function the epilogue uses
///
/// `uvsw.write` carries no destination: the output pointer starts at zero and
/// advances by each group's repeat count, so the ORDER the epilogue writes in
/// IS this layout. If the two were computed separately they could disagree,
/// and the disagreement would be invisible -- every group would encode, the
/// program would link, and the fragment shader would read the wrong dwords.
/// That is the same shape as the slot that `decode_assist` read from a second
/// copy, so it gets the same treatment: one place computes it, and the caller
/// concatenates its registers in this order.
pub fn vertex_output_offsets(components: &[u8], interps: &[u8], offs: &mut [u8]) -> Result<u8, BuildError> {
    use crate::shaderc::online::ir::{INTERP_FLAT, INTERP_NOPERSPECTIVE};
    if offs.len() < components.len() || interps.len() < components.len() {
        return Err(BuildError::Unsupported("more varyings than the caller made room for"));
    }
    /* pco's order (pco_shc.c alloc_vs_varyings, as Mesa's
     * pvr_postprocess_shader_data): the smooth varyings, then the flat, then
     * the noperspective, each in declaration order. Centroid is a
     * qualifier, not a mode: smooth. */
    let group = |i: u8| match i {
        INTERP_FLAT => 1,
        INTERP_NOPERSPECTIVE => 2,
        _ => 0,
    };
    let mut at = VTXOUT_POSITION_DWORDS;
    for g in 0..3 {
        for (i, &c) in components.iter().enumerate() {
            if group(interps[i]) != g {
                continue;
            }
            if c == 0 || c > 4 {
                return Err(BuildError::Unsupported("a varying that is not one to four components"));
            }
            offs[i] = at;
            at = at
                .checked_add(c)
                .ok_or(BuildError::Unsupported("the vertex output is larger than a byte of dwords"))?;
        }
    }
    Ok(at)
}

#[cfg(all(test, feature = "gpu-powervr"))]
mod clear_attach_tests {
    use super::*;

    fn words(n: u16) -> Vec<u32> {
        let mut buf = [0u8; 256];
        let len = build_clear_attach(n, &mut buf).expect("built");
        buf[..len].chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// The driver's three programs (src/pvr/internal.rs, which
    /// yakogl-precompile --internal writes from this builder) are this
    /// builder's words: a builder change that moves them moves the committed
    /// file with it (tools/check.sh regenerates it).
    #[test]
    fn the_clear_attach_programs_are_the_drivers() {
        use crate::pvr::internal::*;
        assert_eq!(words(1), PVR_INTERNAL_CLEAR_ATTACH1, "one dword");
        assert_eq!(words(2), PVR_INTERNAL_CLEAR_ATTACH2, "two dwords");
        assert_eq!(words(3), PVR_INTERNAL_CLEAR_ATTACH3, "three dwords");
        let mut buf = [0u8; 256];
        assert!(build_clear_attach(0, &mut buf).is_err() && build_clear_attach(5, &mut buf).is_err());
    }

    /// And Mesa's: one and two dwords byte-identical to the programs
    /// captured from pvr_uscgen_clear_attach, three to what it makes for
    /// three dwords (usc_blobs.txt `clear3`).
    #[test]
    #[cfg(mesa_reference)]
    fn the_clear_attach_programs_are_mesas() {
        use crate::pvr::mesa_internal::*;
        assert_eq!(words(1), PVR_USC_CLEAR_FS_WORDS, "one dword: the captured program");
        assert_eq!(words(2), PVR_USC_CLEAR_FS2_WORDS, "two dwords: the captured program");
        assert_eq!(words(3), PVR_USC_CLEAR_FS3_WORDS, "three dwords: Mesa's for three");
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::shaderc::online::powervr::sel::{Bank, MachineOp, OpType, SelDst, SelGroup, SelOp, SelSrc};

    fn temp(r: u16) -> SelSrc {
        SelSrc::Fixed { bank: Bank::Temp, reg: r, mods: Mods::default() }
    }
    fn mov(d: u16, s: u16) -> SelGroup {
        SelGroup {
            op: SelOp::Copy,
            ty: OpType::F32,
            dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: d }),
            srcs: [Some(temp(s)), None, None],
        }
    }
    fn marker(op: SelOp) -> SelGroup {
        SelGroup { op, ty: OpType::U32, dst: None, srcs: [None; 3] }
    }
    fn test() -> SelGroup {
        SelGroup {
            op: SelOp::Machine(MachineOp::LoopTest),
            ty: OpType::U32,
            dst: None,
            srcs: [Some(temp(3)), None, None],
        }
    }

    /// Every group the arithmetic yields, placed one after another.
    fn place(groups: &[SelGroup]) -> Result<std::vec::Vec<(usize, Group)>, BuildError> {
        let mut a = Arithmetic::new(groups);
        let (mut at, mut out) = (0usize, std::vec::Vec::new());
        while let Some(g) = a.group(at)? {
            out.push((at, g));
            at += g.total;
        }
        Ok(out)
    }

    /// The back edge lands on the loop's first group, backward, under
    /// `if(p0)`; a `LoopBegin` takes no bytes.
    #[test]
    fn a_back_edge_branches_to_the_first_group_of_its_body() {
        let g = [mov(0, 1), marker(SelOp::LoopBegin), mov(1, 2), mov(2, 1), test(), marker(SelOp::LoopEnd), mov(4, 0)];
        let placed = place(&g).expect("builds");
        assert_eq!(placed.len(), 6, "five groups and the branch");
        let (br_at, br) = placed[4];
        let (_, _, r) = br.instrs().next().unwrap();
        let off = r.get("offset").unwrap() as i32 as i64;
        assert_eq!(br_at as i64 + off, placed[1].0 as i64, "to the body's first group");
        let cc = super::super::isa_tables::ENUMS.iter().find(|e| e.name == "cc").unwrap();
        assert_eq!(cc.name_of(br.hdr.get("cc").unwrap()), Some("if(p0)"));
    }

    /// A back edge anywhere but directly after its loop's test is refused,
    /// as are unpaired markers: the branch must read the counter's p0.
    #[test]
    fn a_back_edge_not_after_its_test_is_refused() {
        let no_test = [marker(SelOp::LoopBegin), mov(1, 2), marker(SelOp::LoopEnd)];
        assert!(place(&no_test).is_err());
        let late = [marker(SelOp::LoopBegin), test(), mov(1, 2), marker(SelOp::LoopEnd)];
        assert!(place(&late).is_err(), "a group between the test and the branch");
        let open = [marker(SelOp::LoopBegin), mov(1, 2)];
        assert!(place(&open).is_err(), "a loop with no back edge");
        let stray = [mov(1, 2), test(), marker(SelOp::LoopEnd)];
        assert!(place(&stray).is_err(), "a back edge with no loop");
        assert!(super::super::fragment::branch_back(0).is_err() && super::super::fragment::branch_back(8).is_err());
    }

    /// A predicated `if`: the then arm's groups carry if(!p0), the else
    /// arm's if(p0), everything else e1_zx; the markers take no bytes.
    #[test]
    fn an_if_arm_s_groups_carry_its_execution_condition() {
        use super::super::sel::PredArm::{Else, End, Then};
        let g = [
            mov(0, 1),
            test(),
            marker(SelOp::Pred(Then)),
            mov(1, 2),
            mov(2, 1),
            marker(SelOp::Pred(Else)),
            mov(4, 0),
            marker(SelOp::Pred(End)),
            mov(5, 4),
        ];
        let placed = place(&g).expect("builds");
        let e = super::super::isa_tables::ENUMS.iter().find(|e| e.name == "cc").unwrap();
        let cc: std::vec::Vec<_> = placed.iter().map(|(_, g)| e.name_of(g.hdr.get("cc").unwrap()).unwrap()).collect();
        assert_eq!(cc, ["e1_zx", "e1_zx", "if(!p0)", "if(!p0)", "if(p0)", "e1_zx"]);
    }

    /// The arm markers out of order are refused, as is anything in an arm
    /// that would read or write p0: the arms' condition.
    #[test]
    fn if_arm_markers_out_of_order_are_refused() {
        use super::super::sel::PredArm::{Else, End, Then};
        let m = |a| marker(SelOp::Pred(a));
        assert!(place(&[mov(0, 1), m(Then), mov(1, 2), m(Else), m(End)]).is_err(), "a then arm not after its test");
        assert!(place(&[test(), m(Then), mov(1, 2)]).is_err(), "an arm with no end");
        assert!(place(&[test(), m(Else), m(End)]).is_err(), "an else with no then");
        assert!(place(&[test(), m(Then), test(), m(Else), m(End)]).is_err(), "a p0 write in an arm");
        assert!(place(&[test(), m(Then), marker(SelOp::LoopBegin), m(Else), m(End)]).is_err(), "a loop edge in an arm");
        assert!(place(&[test(), m(Then), m(Else), m(End)]).is_ok(), "empty arms are fine");
    }

    fn mask(op: MachineOp) -> SelGroup {
        SelGroup {
            op: SelOp::Machine(op),
            ty: OpType::U32,
            dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: 9 }),
            srcs: [Some(temp(9)), None, None],
        }
    }

    /// The name of `g`'s execution condition, operation and (a branch's)
    /// landing byte, placed at `at`.
    fn read(at: usize, g: &Group) -> (&'static str, &'static str, Option<usize>) {
        let e = super::super::isa_tables::ENUMS.iter().find(|e| e.name == "cc").unwrap();
        let (_, ph, r) = g.instrs().next().unwrap();
        let op = super::super::isa::ops_for(r, ph).next().unwrap().name;
        let to = (op == "br").then(|| at + r.get("offset").unwrap() as usize);
        (e.name_of(g.hdr.get("cc").unwrap()).unwrap(), op, to)
    }

    /// A branched `if` (pco_cf.c lower_if_cond_exec): after the `cndst`, a
    /// `br` past the then arm landing on the `cndef`; after the `cndef`, a
    /// `br` past the else arm landing on the `cndend`; the arms' groups
    /// carry no condition, the mask's run in every instance.
    #[test]
    fn a_branched_if_puts_a_branch_past_each_arm() {
        use super::super::sel::PredArm::{Else, End, Then};
        let g = [
            test(),
            mask(MachineOp::CndIf),
            marker(SelOp::Pred(Then)),
            mov(1, 2),
            mov(2, 1),
            marker(SelOp::Pred(Else)),
            mask(MachineOp::CndElse),
            mov(4, 0),
            marker(SelOp::Pred(End)),
            mask(MachineOp::CndIfEnd),
            mov(5, 4),
        ];
        let placed = place(&g).expect("builds");
        let parts: std::vec::Vec<_> = placed.iter().map(|(at, g)| read(*at, g)).collect();
        let at = |i: usize| Some(placed[i].0);
        assert_eq!(parts.len(), 10, "eight groups and two branches");
        assert_eq!(
            parts[1..],
            [
                ("(ignorepe)", "cndst", None),
                ("e1_zx", "br", at(5)),
                ("e1_zx", "mbyp", None),
                ("e1_zx", "mbyp", None),
                ("(ignorepe)", "cndef", None),
                ("e1_zx", "br", at(8)),
                ("e1_zx", "mbyp", None),
                ("(ignorepe)", "cndend", None),
                ("e1_zx", "mbyp", None),
            ]
        );
    }

    /// An arm with no groups gets no branch: an `if` with no else arm has
    /// no `cndef` (selection leaves it out) and one branch, and one whose
    /// then arm is empty branches only past its else arm.
    #[test]
    fn a_branched_if_skips_only_the_arms_it_has() {
        use super::super::sel::PredArm::{Else, End, Then};
        let m = |a| marker(SelOp::Pred(a));
        let no_else = [test(), mask(MachineOp::CndIf), m(Then), mov(1, 2), m(Else), m(End), mask(MachineOp::CndIfEnd)];
        let placed = place(&no_else).expect("builds");
        let ops: std::vec::Vec<_> = placed.iter().map(|(at, g)| read(*at, g)).map(|x| (x.1, x.2)).collect();
        assert_eq!(ops[1..], [("cndst", None), ("br", Some(placed[4].0)), ("mbyp", None), ("cndend", None)]);
        let no_then = [
            test(),
            mask(MachineOp::CndIf),
            m(Then),
            m(Else),
            mask(MachineOp::CndElse),
            mov(1, 2),
            m(End),
            mask(MachineOp::CndIfEnd),
        ];
        let placed = place(&no_then).expect("builds");
        let ops: std::vec::Vec<_> = placed.iter().map(|(at, g)| read(*at, g)).map(|x| (x.1, x.2)).collect();
        assert_eq!(
            ops[1..],
            [("cndst", None), ("cndef", None), ("br", Some(placed[5].0)), ("mbyp", None), ("cndend", None)]
        );
    }

    /// The branched form's order is held: the `cndst` right before the then
    /// marker, the `cndend` right after the end, no mask group inside an
    /// arm.
    #[test]
    fn a_branched_if_out_of_shape_is_refused() {
        use super::super::sel::PredArm::{Else, End, Then};
        let m = |a| marker(SelOp::Pred(a));
        let (st, ef, end) = (mask(MachineOp::CndIf), mask(MachineOp::CndElse), mask(MachineOp::CndIfEnd));
        assert!(place(&[test(), st, mov(1, 2), m(Then), m(Else), m(End), end]).is_err(), "a group after the cndst");
        assert!(place(&[test(), st, m(Then), mov(1, 2), m(Else), m(End), mov(2, 1), end]).is_err(), "no cndend");
        assert!(place(&[test(), st, m(Then), ef, m(Else), m(End), end]).is_err(), "a cndef in the then arm");
        assert!(place(&[test(), st, m(Then), m(Else), ef, ef, mov(1, 2), m(End), end]).is_err(), "two cndef");
        assert!(place(&[test(), st, m(Then), mov(1, 2), m(Else), m(End), end]).is_ok(), "the shape");
    }
}
