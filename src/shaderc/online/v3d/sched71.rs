// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// SPDX-FileCopyrightText: 2010 Intel Corporation
// Derived from Mesa 26.2.1 src/broadcom/compiler/qpu_schedule.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Scheduling at V3D 7.1, straight-line code: one run of allocated
//! [`Op71`]s with no control flow packed into [`Instr71`] words, thread
//! switches, their delay slots and the program end included, as the
//! contract in [`super::vop71`]'s header and `docs/notes/v3d71-backend.md`
//! (e) say. Branches, counted loops and their delay slots are
//! [`super::sched71_cf`]'s: [`pack71`] refuses them by name
//! ([`Sched71Error::ControlFlow`]), and [`Packer71`] is what sched71_cf
//! builds on.
//!
//! # In order, merged, never reordered
//!
//! As [`super::sched`] on 4.2: each operation goes into the word the previous
//! one went into, or a later one; it shares a word when the word has room
//! (the add unit, the mul unit, the signal, one small immediate, one
//! uniform, one TLB access, one TMU write) and its operands are ready there,
//! and starts a new word otherwise. The uniform stream, the `ldvary`
//! sequence and the TLB writes keep the program order, and alloc71's "last
//! read" is exact. Whether a word can hold an operation is decided by
//! [`encode71`] (two signals with no joint encoding are not made). This
//! first cut has none of sched.rs's `hoist` or `ldvary_back`, and reads an
//! SFU result two instructions later, not one (design note (c), (e)).
//!
//! # The 7.1 rules, from Mesa 26.2.1 (MIT), each cited
//!
//! * No read-port limit: each of add a, add b, mul a, mul b has its own
//!   raddr, and two operations merge unless both use a small immediate
//!   (`qpu_merge_raddrs`, qpu_schedule.c:1013-1032); a/b only with an add
//!   operation, c/d only with a mul one (qpu_validate.c:204-218).
//! * A mul MOV or FMOV takes a free add unit when the mul unit is taken
//!   (`can_do_mul_as_add`, qpu_schedule.c:1174-1183; `qpu_convert_mul_to_add`,
//!   :1200-1233).
//! * A result is readable one instruction later; an SFU result two
//!   (`instruction_latency`, qpu_schedule.c:1870-1871).
//! * One restricted peripheral per word, bar WRTMUC with a TMU write other
//!   than TMUC; one TMU read; one TLB access (qpu_schedule.c:937-975,
//!   [`peripherals_compatible71`]).
//! * `ldunif` into rf0 is the plain signal (no register named); any other
//!   destination is `ldunifrf`, which names it in the condition field
//!   (vir_to_qpu.c:328-350), so that word carries no condition or flag
//!   push. A signal names at most one address (qpu_schedule.c:1372-1374).
//! * `ldvary` writes C to rf0 one instruction after its own destination
//!   (qpu_schedule.c:1739-1742): rf0 (C) is read two instructions after the
//!   `ldvary` (`reads_too_soon`, :623-635), and nothing else writes rf0 in
//!   the instruction right after it (`writes_too_soon_after_write`,
//!   :713-727; "LDUNIF after a LDVARY", qpu_validate.c:220-227).
//! * A thread switch's delay slots: no SFU operation in slot 1 or 2, no
//!   `ldvary` in slot 2 (qpu_schedule.c:2079-2091), and no signal writing the
//!   register file in slot 2 when rf0's last implicit write coincided with
//!   one (`has_rf0_flops_conflict`, :1692-1745, :2110-2119). An operation
//!   from after the THRSW goes into its slots only if it is no THRSW, TLB
//!   access, TMU write, WRTMUC, TMU wait, MULTOP, flag write or BARRIERID
//!   (`qpu_inst_after_thrsw_valid_in_delay_slot`, :2132-2215; no
//!   accumulator rule at 7.1, `v3d_qpu_writes_accum`). Conditional writes
//!   stay out too, as in sched.rs.
//! * A THRSW moves back onto one of the last three words placed when they
//!   are valid in its slots (`emit_thrsw`, qpu_schedule.c:2250-2330); two
//!   THRSW in a row are the last-thread-switch pair, the second in the word
//!   after the first (:2365-2377).
//! * The program end: a THRSW and two delay slots, moved back as above when
//!   the words are valid in a thread-end slot
//!   (`qpu_inst_valid_in_thrend_slot`, qpu_schedule.c:1979-2061, ver >= 71):
//!   slot 0 writes no register through either ALU, no slot reads or writes
//!   rf2/rf3, no uniform after slot 0, no `ldvary`, no TLB Z write in slot 2
//!   (this cut keeps every TLB write out of slot 2: the words do not say
//!   which write is Z). The 4.2-only rules (GFXH-1684's VPM wait, GFXH-1625's
//!   TMUWT, no register-file write in any slot) are not applied.
//! * The TMU, as sched.rs: a look-up's configuration (`wrtmuc`) before its
//!   register writes, the next look-up's configuration after the write that
//!   retired the last one (qpu_schedule.c:249-250, 511-516), an `ldtmu`
//!   after the write that retired its look-up; a look-up is never paired
//!   with an earlier `ldtmu` (design note (d)).

use super::alloc71::{Allocation71, Rf};
use super::codec71::{decode71, encode71};
use super::encode::EncodeError;
use super::isa71::{
    add_op_args, mul_op_args, AddAlu71, Alu71, Cond, Flags, MulAlu71, Operand, OutPack, ARG_D, WADDR_NOP,
};
use super::verify71::{is_sfu_op, peripherals_compatible71};
use super::vop71::{AddOp71, Dst71, FlagOp71, Instr71, MulOp71, Op71, Src71, Uniform, Unpack71, V};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};

/// The magic write addresses this file names (`enum v3d_qpu_waddr`).
const WADDR_TLB: u8 = 7;
const WADDR_TLBU: u8 = 8;

/// A TMU register (`v3d_qpu_magic_waddr_is_tmu`, qpu_instr.c:633-648).
fn is_tmu(waddr: u8) -> bool {
    (11..=13).contains(&waddr) || (32..=46).contains(&waddr)
}

/// A TMU write that ends a look-up's sequence (TMUS, TMUSCM, TMUSF, TMUSLOD,
/// TMUA, TMUAU: `tmu_write_is_sequence_terminator`, qpu_schedule.c:188-202).
fn ends_tmu_sequence(waddr: u8) -> bool {
    matches!(waddr, 33 | 40 | 41 | 42 | 12 | 13)
}

/// Why 7.1 scheduling stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sched71Error {
    /// Not scheduled at 7.1 yet, by name.
    NotYet(&'static str),
    /// Operation `.0` is control flow, which [`pack71`] does not take.
    ControlFlow(usize),
    /// Operation `.0` does not encode even alone in a word.
    NotEncodable(usize, EncodeError),
    /// Operation `.0` is a conditional write with no flag push before it.
    CondWithoutFlags(usize),
    /// Operation `.0` reads [`Src71::LdvaryC`] with no `ldvary` before it.
    LdvaryCWithoutLdvary(usize),
    /// Operation `.0` reads rf0 (an `ldvary`'s C or a value alloc71 put in
    /// rf0) after another write replaced it: an allocation alloc71 does not
    /// make.
    Rf0Clobbered(usize),
    /// Operation `.0` is an `ldtmu` with no TMU look-up written before it.
    LdtmuWithoutLookup(usize),
    /// Operation `.0` names a virtual register the allocation has no
    /// register for.
    Unallocated(usize),
    /// Operation `.0` is not something a word can hold (an `ldvary` into
    /// rf0, where its own C lands).
    NotPlaceable(usize),
    OutOfScratch,
}

impl From<ArenaError> for Sched71Error {
    fn from(_: ArenaError) -> Self {
        Sched71Error::OutOfScratch
    }
}

/// One packed straight-line run.
#[derive(Clone, Copy, Debug)]
pub struct Packed71<'a> {
    pub instrs: &'a [Instr71],
    /// The uniforms the run consumes, in order.
    pub uniforms: &'a [Uniform],
}

/// The ALU's operation has a destination (`v3d_qpu_add_op_has_dst`).
fn add_dst(alu: &Alu71) -> bool {
    alu.add.op != AddOp71::Nop && add_op_args(alu.add.op) & ARG_D != 0
}
fn mul_dst(alu: &Alu71) -> bool {
    alu.mul.op != MulOp71::Nop && mul_op_args(alu.mul.op) & ARG_D != 0
}

/// The register-file registers a word writes by name: the add unit, the mul
/// unit, the signal (`v3d71_qpu_writes_waddr_explicitly`,
/// qpu_instr.c:1066-1093).
fn rf_writes(alu: &Alu71) -> [Option<u8>; 3] {
    [
        (add_dst(alu) && !alu.add.magic).then_some(alu.add.waddr),
        (mul_dst(alu) && !alu.mul.magic).then_some(alu.mul.waddr),
        (alu.sig.writes_address() && !alu.sig_magic).then_some(alu.sig_addr),
    ]
}

fn writes_rf(alu: &Alu71, r: u8) -> bool {
    rf_writes(alu).contains(&Some(r))
}

/// A signal writes the register file (qpu_schedule.c:1696-1698, 2113-2114).
fn sig_writes_rf(alu: &Alu71) -> bool {
    rf_writes(alu)[2].is_some()
}

/// `v3d_qpu_writes_rf0_implicitly` (qpu_instr.c:1030-1040).
fn implicit_rf0(alu: &Alu71) -> bool {
    alu.sig.ldvary || alu.sig.ldunif || alu.sig.ldunifa
}

/// `v3d71_qpu_reads_raddr` (qpu_instr.c:1055-1064).
fn reads_rf(alu: &Alu71, r: u8) -> bool {
    alu.read_operands().any(|(o, imm)| !imm && o.raddr == r)
}

/// The word being filled.
#[derive(Clone, Copy, Debug)]
pub struct Word71 {
    pub(super) alu: Alu71,
    /// The uniform the word consumes (`ldunif`, `ldunifrf`, `wrtmuc`, TLBU).
    pub(super) uniform: Option<Uniform>,
    add_used: bool,
    mul_used: bool,
    sig_addr_used: bool,
    /// A TLB write is in this word (one TLB access per instruction).
    tlb: bool,
    /// A TMU register write is in this word (one per instruction:
    /// qpu_schedule.c:944-962 counts the TMU write once per operation).
    tmu_write: bool,
    /// This word pushes a flag / has a conditional write. The two never
    /// share a word, as in sched.rs.
    flags_push: bool,
    flags_read: bool,
}

/// Why an operation did not go into a word.
enum Refused {
    /// A unit, the signal address, the small immediate, the uniform or a
    /// peripheral is taken, or the condition field is.
    Full,
    /// Never placeable by [`merge`] (a THRSW, control flow).
    Never,
    Unallocated,
    Encode(EncodeError),
}

fn reg(al: &[Rf], v: V) -> Result<u8, Refused> {
    al.get(v.0 as usize).map(|r| r.0).ok_or(Refused::Unallocated)
}

/// An operand: its address (or small-immediate index) and unpack, and
/// whether it is a small immediate.
fn src(al: &[Rf], s: Src71) -> Result<(Operand, bool), Refused> {
    let rf = |raddr: u8, unpack: Unpack71| Operand { raddr, unpack };
    Ok(match s {
        Src71::V(v) => (rf(reg(al, v)?, Unpack71::None), false),
        Src71::Unpacked(v, u) => (rf(reg(al, v)?, u), false),
        Src71::SmallImm(i) => (rf(i, Unpack71::None), true),
        Src71::Payload(n) => (rf(n, Unpack71::None), false),
        Src71::LdvaryC => (rf(0, Unpack71::None), false),
        Src71::None => (Operand::default(), false),
    })
}

/// A destination: waddr, magic, and the uniform a TLBU write consumes.
fn dst(al: &[Rf], d: Dst71) -> Result<(u8, bool, Option<Uniform>), Refused> {
    Ok(match d {
        Dst71::V(v) => (reg(al, v)?, false, None),
        Dst71::Tlb => (WADDR_TLB, true, None),
        Dst71::Tlbu(u) => (WADDR_TLBU, true, Some(u)),
        Dst71::Nop => (WADDR_NOP, true, None),
        Dst71::None => (0, false, None),
        Dst71::Magic(w) => (w, true, None),
    })
}

impl Word71 {
    pub(super) fn empty() -> Word71 {
        Word71 {
            alu: Alu71::nop(),
            uniform: None,
            add_used: false,
            mul_used: false,
            sig_addr_used: false,
            tlb: false,
            tmu_write: false,
            flags_push: false,
            flags_read: false,
        }
    }

    /// The uniform, the TLB access and the TMU write a destination takes.
    fn take_dst(&mut self, d: Dst71, u: Option<Uniform>) -> Result<(), Refused> {
        if let Some(u) = u {
            if self.uniform.is_some() {
                return Err(Refused::Full);
            }
            self.uniform = Some(u);
        }
        match d {
            Dst71::Tlb | Dst71::Tlbu(_) => {
                if self.tlb {
                    return Err(Refused::Full);
                }
                self.tlb = true;
            }
            Dst71::Magic(w) if is_tmu(w) => {
                if self.tmu_write {
                    return Err(Refused::Full);
                }
                self.tmu_write = true;
            }
            _ => {}
        }
        Ok(())
    }

    /// The add unit, its small-immediate flags a and b.
    fn put_add(&mut self, al: &[Rf], op: AddOp71, d: Dst71, a: Src71, b: Src71) -> Result<(), Refused> {
        if self.add_used {
            return Err(Refused::Full);
        }
        let ((oa, ia), (ob, ib)) = (src(al, a)?, src(al, b)?);
        let (waddr, magic, u) = dst(al, d)?;
        self.alu.add = AddAlu71 { op, a: oa, b: ob, waddr, magic, pack: OutPack::None };
        (self.alu.sig.small_imm_a, self.alu.sig.small_imm_b) = (ia, ib);
        self.add_used = true;
        self.take_dst(d, u)
    }

    /// The mul unit, its small-immediate flags c and d.
    fn put_mul(&mut self, al: &[Rf], op: MulOp71, d: Dst71, a: Src71, b: Src71) -> Result<(), Refused> {
        if self.mul_used {
            return Err(Refused::Full);
        }
        let ((oa, ia), (ob, ib)) = (src(al, a)?, src(al, b)?);
        let (waddr, magic, u) = dst(al, d)?;
        self.alu.mul = MulAlu71 { op, a: oa, b: ob, waddr, magic, pack: OutPack::None };
        (self.alu.sig.small_imm_c, self.alu.sig.small_imm_d) = (ia, ib);
        self.mul_used = true;
        self.take_dst(d, u)
    }

    /// A signal that names its destination (ldunifrf, ldvary, ldtmu).
    fn put_sig_addr(&mut self, addr: u8) -> Result<(), Refused> {
        if self.sig_addr_used {
            return Err(Refused::Full);
        }
        self.alu.sig_addr = addr;
        self.alu.sig_magic = false;
        self.sig_addr_used = true;
        Ok(())
    }

    /// What every word must be, whatever it holds.
    fn check(&self) -> Result<(), Refused> {
        let a = &self.alu;
        let s = &a.sig;
        if s.small_imm_a as u32 + s.small_imm_b as u32 + s.small_imm_c as u32 + s.small_imm_d as u32 > 1 {
            return Err(Refused::Full);
        }
        /* a signal's address is the condition field (qpu_pack.c, v3d71
         * cond/sig_addr; Sig71::writes_address) */
        if s.writes_address() && a.flags != Flags::default() {
            return Err(Refused::Full);
        }
        /* two writes of one register in one instruction (a plain ldunif's
         * rf0 among them) */
        let w = rf_writes(a);
        let implicit = (s.ldunif || s.ldunifa).then_some(0u8);
        let all = [w[0], w[1], w[2], implicit];
        for i in 0..all.len() {
            if all[i].is_some() && all[i + 1..].contains(&all[i]) {
                return Err(Refused::Full);
            }
        }
        if !peripherals_compatible71(a) {
            return Err(Refused::Full);
        }
        encode71(&Instr71::Alu(*a)).map_err(Refused::Encode)?;
        Ok(())
    }
}

/// `w` with `op` added, or why not.
fn merge(w: &Word71, al: &[Rf], op: &Op71) -> Result<Word71, Refused> {
    let mut w = *w;
    match *op {
        Op71::Add { op, dst, a, b } => w.put_add(al, op, dst, a, b)?,
        Op71::Mul { op, dst, a, b } => {
            /* can_do_mul_as_add (qpu_schedule.c:1174-1183): a register move
             * takes the add unit when the mul unit is taken */
            let as_add = match op {
                MulOp71::Mov => Some(AddOp71::Mov),
                MulOp71::Fmov => Some(AddOp71::Fmov),
                _ => None,
            };
            match as_add {
                Some(aop) if w.mul_used && matches!(dst, Dst71::V(_)) => w.put_add(al, aop, dst, a, b)?,
                _ => w.put_mul(al, op, dst, a, b)?,
            }
        }
        Op71::Copy { dst, src } => {
            return merge(&w, al, &Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(dst), a: src, b: Src71::None });
        }
        Op71::Sfu { op, dst, a } => w.put_add(al, op.add_op(), Dst71::V(dst), a, Src71::None)?,
        Op71::Ldunif { dst, u } => {
            if w.uniform.is_some() {
                return Err(Refused::Full);
            }
            match reg(al, dst)? {
                /* rf0 is the plain signal's implicit destination; with an
                 * ldvary in the word both would write rf0 */
                0 => {
                    if w.alu.sig.ldvary {
                        return Err(Refused::Full);
                    }
                    w.alu.sig.ldunif = true;
                }
                r => {
                    w.put_sig_addr(r)?;
                    w.alu.sig.ldunifrf = true;
                }
            }
            w.uniform = Some(u);
        }
        Op71::Ldvary { dst } => {
            if w.alu.sig.ldunif {
                return Err(Refused::Full);
            }
            w.put_sig_addr(reg(al, dst)?)?;
            w.alu.sig.ldvary = true;
        }
        Op71::Ldtmu { dst } => {
            w.put_sig_addr(reg(al, dst)?)?;
            w.alu.sig.ldtmu = true;
        }
        Op71::Wrtmuc { u } => {
            if w.uniform.is_some() || w.alu.sig.wrtmuc {
                return Err(Refused::Full);
            }
            w.alu.sig.wrtmuc = true;
            w.uniform = Some(u);
        }
        Op71::Flags { unit, a, b, pf, uf } => {
            if w.flags_push || w.flags_read {
                return Err(Refused::Full);
            }
            match unit {
                FlagOp71::Add(op) => {
                    w.put_add(al, op, Dst71::Nop, a, b)?;
                    (w.alu.flags.apf, w.alu.flags.auf) = (pf, uf);
                }
                FlagOp71::Mul(op) => {
                    w.put_mul(al, op, Dst71::Nop, a, b)?;
                    (w.alu.flags.mpf, w.alu.flags.muf) = (pf, uf);
                }
            }
            w.flags_push = true;
        }
        Op71::CondMov { dst, src, cond } => {
            if w.flags_push {
                return Err(Refused::Full);
            }
            if !w.mul_used {
                w.put_mul(al, MulOp71::Mov, Dst71::V(dst), src, Src71::None)?;
                w.alu.flags.mc = cond;
            } else {
                w.put_add(al, AddOp71::Mov, Dst71::V(dst), src, Src71::None)?;
                w.alu.flags.ac = cond;
            }
            w.flags_read = true;
        }
        Op71::Setmsf { src, cond } => {
            if cond != Cond::None && w.flags_push {
                return Err(Refused::Full);
            }
            w.put_add(al, AddOp71::Setmsf, Dst71::Nop, src, Src71::None)?;
            if cond != Cond::None {
                w.alu.flags.ac = cond;
                w.flags_read = true;
            }
        }
        Op71::Thrsw | Op71::LoopHead | Op71::BackEdge { .. } | Op71::FwdBranch { .. } | Op71::Label(_) => {
            return Err(Refused::Never)
        }
    }
    w.check()?;
    Ok(w)
}

/// The latency of `op`'s result: how many instructions later it is read.
fn latency(op: &Op71) -> usize {
    match op {
        Op71::Sfu { .. } => 2,
        Op71::Add { op, .. } if is_sfu_op(*op) => 2,
        _ => 1,
    }
}

/// `op`, scheduled after a THRSW, may go into its delay slots
/// (`qpu_inst_after_thrsw_valid_in_delay_slot`, qpu_schedule.c:2132-2215).
/// The slot rules shared with the words from before the THRSW (SFU,
/// `ldvary` in slot 2, rf0) are [`window_ok`]'s.
fn ok_after_thrsw(op: &Op71) -> bool {
    match *op {
        /* flags are invalidated across a thread switch (:2203-2207): no push
         * and, as in sched.rs, no conditional write */
        Op71::Thrsw
        | Op71::Flags { .. }
        | Op71::CondMov { .. }
        | Op71::Setmsf { .. }
        | Op71::Wrtmuc { .. }
        | Op71::Ldtmu { .. }
        | Op71::Sfu { .. } => false,
        Op71::Add { op: AddOp71::Tmuwt | AddOp71::Barrierid, .. } | Op71::Mul { op: MulOp71::Multop, .. } => false,
        Op71::Add { op, .. } if is_sfu_op(op) => false,
        /* TLB, TMU and any other magic write but NOP (UNIFA, TSY) */
        Op71::Add { dst, .. } | Op71::Mul { dst, .. } => !matches!(dst, Dst71::Tlb | Dst71::Tlbu(_) | Dst71::Magic(_)),
        Op71::Ldunif { .. } | Op71::Ldvary { .. } | Op71::Copy { .. } => true,
        Op71::LoopHead | Op71::BackEdge { .. } | Op71::FwdBranch { .. } | Op71::Label(_) => false,
    }
}

/// Word `w` may be slot `slot` of the program end
/// (`qpu_inst_valid_in_thrend_slot`, qpu_schedule.c:1979-2061, ver >= 71).
fn ok_in_thrend71(w: &Word71, slot: usize) -> bool {
    let a = &w.alu;
    if slot > 0 && w.uniform.is_some() {
        return false;
    }
    if a.sig.ldvary {
        return false;
    }
    /* is_tlb_z_write in slot 2 (:1984-1985), for every TLB write here */
    if slot == 2 && w.tlb {
        return false;
    }
    /* the thread end instruction writes no register through the ALUs
     * (:2018-2027) */
    if slot == 0 && ((add_dst(a) && !a.add.magic) || (mul_dst(a) && !a.mul.magic)) {
        return false;
    }
    /* rf2/rf3 may be overwritten by the next fragment's setup (:2043-2056) */
    !(reads_rf(a, 2) || reads_rf(a, 3) || writes_rf(a, 2) || writes_rf(a, 3))
}

/// rf0's flops conflict at the end of word `end`, counting from `from`, the
/// first word of the thread section (`update_scoreboard_for_rf0_flops`,
/// qpu_schedule.c:1704-1745: reset by an explicit rf0 write and at the
/// section's start; an implicit write lands in its own word, an `ldvary`'s
/// one later; a conflict when it lands with a signal's register-file
/// write).
fn rf0_flops(get: &dyn Fn(usize) -> Option<Word71>, from: usize, end: usize) -> bool {
    let (mut implicit_at, mut conflict) = (None, false);
    for j in from..=end {
        let Some(w) = get(j) else { continue };
        let a = &w.alu;
        if writes_rf(a, 0) {
            (implicit_at, conflict) = (None, false);
        }
        if implicit_rf0(a) {
            implicit_at = Some(if a.sig.ldvary { j + 1 } else { j });
        }
        if implicit_at == Some(j) && sig_writes_rf(a) {
            conflict = true;
        }
    }
    conflict
}

/// The words of the THRSW in word `t` (slots 0, 1, 2, as far as they exist;
/// `over` replaces one) are valid there
/// (`qpu_inst_before_thrsw_valid_in_delay_slot`, qpu_schedule.c:2070-2122),
/// and in a thread-end slot when `thrend`. `from` is the first word of the
/// section the switch ends. The rf0 rule is the one sim71 runs: the
/// conflict counted through slot 2 itself.
fn window_ok(words: &[Word71], over: Option<(usize, &Word71)>, t: usize, from: usize, thrend: bool) -> bool {
    let get = |j: usize| match over {
        Some((k, w)) if k == j => Some(*w),
        _ => words.get(j).copied(),
    };
    (0..3).all(|slot| {
        let Some(w) = get(t + slot) else { return true };
        let a = &w.alu;
        if slot > 0 && is_sfu_op(a.add.op) {
            return false;
        }
        if slot == 2 && a.sig.ldvary {
            return false;
        }
        if slot == 2 && sig_writes_rf(a) && rf0_flops(&get, from, t + 2) {
            return false;
        }
        !thrend || ok_in_thrend71(&w, slot)
    })
}

/// What rf0 holds, for the reads of it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rf0 {
    Unknown,
    /// A value alloc71 put there.
    Value(V),
    /// The last `ldvary`'s C.
    C,
}

/// The packer: straight-line operations in, words out. [`pack71`] runs it
/// over one run; [`super::sched71_cf`] runs it over the runs of a program,
/// with [`Packer71::fence`] at each control-flow boundary.
pub struct Packer71<'a, 'r> {
    reg: &'r [Rf],
    words: AVec<'a, Word71>,
    /// Per virtual register, the first word its value can be read in.
    ready: &'a mut [usize],
    /// The first word the next operation may go in.
    floor: usize,
    /// No operation or THRSW goes before this word.
    fence: usize,
    last_ldvary: Option<usize>,
    /// The word of the last flag push: a condition reads it from the next
    /// word on.
    last_flags: Option<usize>,
    last_wrtmuc: Option<usize>,
    last_tmu_write: Option<usize>,
    /// The word of the last write that ended a look-up's sequence.
    last_tmu_end: Option<usize>,
    last_ldtmu: Option<usize>,
    /// The word of the last THRSW that opened a delay window (the first of
    /// a pair), and the first word of the section it ends.
    last_thrsw: Option<usize>,
    section: usize,
    prev_was_thrsw: bool,
    rf0: Rf0,
}

impl<'a, 'r> Packer71<'a, 'r> {
    pub fn new(al: &Allocation71<'r>, a: &Arena<'a>) -> Result<Packer71<'a, 'r>, Sched71Error> {
        Ok(Packer71 {
            reg: al.reg,
            words: AVec::new(),
            ready: a.alloc_filled(al.reg.len(), 0usize)?,
            floor: 0,
            fence: 0,
            last_ldvary: None,
            last_flags: None,
            last_wrtmuc: None,
            last_tmu_write: None,
            last_tmu_end: None,
            last_ldtmu: None,
            last_thrsw: None,
            section: 0,
            prev_was_thrsw: false,
            rf0: Rf0::Unknown,
        })
    }

    /// Words placed so far.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// No word placed yet.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The word at `at`.
    pub fn word(&self, at: usize) -> Option<&Word71> {
        self.words.as_slice().get(at)
    }

    /// The next operation starts a new word; nothing merges across, and no
    /// THRSW moves back across.
    pub fn fence(&mut self) {
        self.fence = self.words.len().max(self.floor);
        self.floor = self.fence;
        self.prev_was_thrsw = false;
    }

    /// Where a THRSW, or the program end, may go at the earliest.
    fn barrier(&self) -> usize {
        self.last_thrsw.map_or(0, |t| t + 3).max(self.fence)
    }

    fn pad_to(&mut self, at: usize, a: &Arena<'a>) -> Result<(), Sched71Error> {
        while self.words.len() < at {
            self.words.push(a, Word71::empty())?;
        }
        Ok(())
    }

    fn reg_of(&self, i: usize, v: V) -> Result<u8, Sched71Error> {
        reg(self.reg, v).map_err(|_| Sched71Error::Unallocated(i))
    }

    /// The earliest of the last three words a THRSW can be merged into
    /// (`emit_thrsw`, qpu_schedule.c:2250-2330): it encodes with the signal,
    /// so does the next one for the last-thread-switch pair (:2316-2330),
    /// and the words from there on are valid in its slots.
    fn merge_back(&self, pair: bool) -> Option<usize> {
        let words = self.words.as_slice();
        let n = words.len();
        let from = self.last_thrsw.map_or(0, |t| t + 3);
        let with_thrsw = |w: &Word71| {
            let mut t = *w;
            t.alu.sig.thrsw = true;
            encode71(&Instr71::Alu(t.alu)).is_ok().then_some(t)
        };
        (n.saturating_sub(3).max(self.barrier())..n).find(|&k| {
            let Some(first) = with_thrsw(&words[k]) else { return false };
            (!pair || k + 1 >= n || with_thrsw(&words[k + 1]).is_some())
                && window_ok(words, Some((k, &first)), k, from, false)
        })
    }

    fn thrsw(&mut self, i: usize, pair: bool, a: &Arena<'a>) -> Result<(), Sched71Error> {
        let second = self.prev_was_thrsw;
        let at = if second {
            /* the second of the pair: the word after the first */
            self.last_thrsw.ok_or(Sched71Error::NotPlaceable(i))? + 1
        } else {
            match self.merge_back(pair) {
                Some(k) => k,
                None => self.words.len().max(self.floor).max(self.barrier()),
            }
        };
        self.pad_to(at, a)?;
        if at < self.words.len() {
            let w = &mut self.words.as_mut_slice()[at];
            w.alu.sig.thrsw = true;
            if encode71(&Instr71::Alu(w.alu)).is_err() {
                return Err(Sched71Error::NotPlaceable(i));
            }
        } else {
            let mut w = Word71::empty();
            w.alu.sig.thrsw = true;
            self.words.push(a, w)?;
        }
        if !second {
            self.section = self.last_thrsw.map_or(0, |t| t + 3);
            self.last_thrsw = Some(at);
        }
        /* a third THRSW in a row opens a window of its own */
        self.prev_was_thrsw = !second;
        self.floor = self.floor.max(at + 1);
        Ok(())
    }

    /// Place operation `i`, `op`; `next` is the operation after it (a THRSW
    /// followed by a THRSW is the last-thread-switch pair).
    pub fn place(&mut self, i: usize, op: &Op71, next: Option<&Op71>, a: &Arena<'a>) -> Result<(), Sched71Error> {
        if op.is_control_flow() {
            return Err(Sched71Error::ControlFlow(i));
        }
        if *op == Op71::Thrsw {
            return self.thrsw(i, next == Some(&Op71::Thrsw), a);
        }
        self.prev_was_thrsw = false;
        /* A move the allocator made from a register to itself copies
         * nothing: dropped, as in sched.rs. */
        if let Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(d), a: Src71::V(x), b: Src71::None }
        | Op71::Copy { dst: d, src: Src71::V(x) } = *op
        {
            if self.reg_of(i, d)? == self.reg_of(i, x)? {
                self.ready[d.0 as usize] = self.ready[d.0 as usize].max(self.ready[x.0 as usize]);
                return Ok(());
            }
        }
        let alone = match merge(&Word71::empty(), self.reg, op) {
            Ok(w) => w,
            Err(Refused::Encode(e)) => return Err(Sched71Error::NotEncodable(i, e)),
            Err(Refused::Unallocated) => return Err(Sched71Error::Unallocated(i)),
            Err(Refused::Full | Refused::Never) => return Err(Sched71Error::NotPlaceable(i)),
        };
        let mut at = self.floor.max(self.fence);
        let tmu_write =
            matches!(op, Op71::Add { dst: Dst71::Magic(w), .. } | Op71::Mul { dst: Dst71::Magic(w), .. } if is_tmu(*w));
        if matches!(op, Op71::CondMov { .. }) || matches!(op, Op71::Setmsf { cond, .. } if *cond != Cond::None) {
            at = at.max(self.last_flags.ok_or(Sched71Error::CondWithoutFlags(i))? + 1);
        }
        if tmu_write || matches!(op, Op71::Wrtmuc { .. }) {
            /* no new look-up beside an earlier ldtmu (design note (d)) */
            if let Some(l) = self.last_ldtmu {
                at = at.max(l + 1);
            }
        }
        if tmu_write {
            if let Some(w) = self.last_wrtmuc {
                at = at.max(w + 1);
            }
        }
        if matches!(op, Op71::Wrtmuc { .. }) {
            if let Some(w) = self.last_tmu_end {
                at = at.max(w + 1);
            }
        }
        if matches!(op, Op71::Ldtmu { .. }) {
            at = at.max(self.last_tmu_write.ok_or(Sched71Error::LdtmuWithoutLookup(i))? + 1);
        }
        /* rf0, written by this operation: never in an ldvary's own word or
         * the next one, where its C lands (qpu_schedule.c:713-727;
         * qpu_validate.c:220-227) */
        let def_reg = match op.def() {
            Some(v) => Some(self.reg_of(i, v)?),
            None => None,
        };
        if let Op71::Ldvary { .. } = op {
            if def_reg == Some(0) {
                return Err(Sched71Error::NotPlaceable(i));
            }
        } else if def_reg == Some(0) {
            if let Some(l) = self.last_ldvary {
                at = at.max(l + 2);
            }
        }
        /* every register operand waits for its value; rf0 is read only by
         * the value that is in it */
        for s in op.uses() {
            match s {
                Src71::LdvaryC => {
                    let l = self.last_ldvary.ok_or(Sched71Error::LdvaryCWithoutLdvary(i))?;
                    if self.rf0 != Rf0::C {
                        return Err(Sched71Error::Rf0Clobbered(i));
                    }
                    at = at.max(l + 2);
                }
                _ => {
                    if let Some(v) = s.vreg() {
                        if self.reg_of(i, v)? == 0 && self.rf0 != Rf0::Value(v) {
                            return Err(Sched71Error::Rf0Clobbered(i));
                        }
                        at = at.max(self.ready[v.0 as usize]);
                    }
                }
            }
        }
        loop {
            let words = self.words.as_slice();
            let in_delay = self.last_thrsw.is_some_and(|t| at > t && at <= t + 2);
            if in_delay && !ok_after_thrsw(op) {
                at += 1;
                continue;
            }
            /* an ldvary's C lands in the next word, which then writes no rf0 */
            if matches!(op, Op71::Ldvary { .. }) {
                if let Some(nx) = words.get(at + 1) {
                    if writes_rf(&nx.alu, 0) || nx.alu.sig.ldunif || nx.alu.sig.ldunifa {
                        at += 1;
                        continue;
                    }
                }
            }
            let cand = if at < words.len() {
                match merge(&words[at], self.reg, op) {
                    Ok(w) => w,
                    Err(_) => {
                        at += 1;
                        continue;
                    }
                }
            } else {
                alone
            };
            if in_delay {
                let t = self.last_thrsw.unwrap_or(0);
                if !window_ok(words, Some((at, &cand)), t, self.section, false) {
                    at += 1;
                    continue;
                }
            }
            if at < words.len() {
                self.words.as_mut_slice()[at] = cand;
            } else {
                self.pad_to(at, a)?;
                self.words.push(a, cand)?;
            }
            break;
        }
        if let Some(v) = op.def() {
            self.ready[v.0 as usize] = at + latency(op);
            if def_reg == Some(0) {
                self.rf0 = Rf0::Value(v);
            }
        }
        /* a conditional move writes its destination without defining it */
        if let Op71::CondMov { dst, .. } | Op71::Copy { dst, .. } = *op {
            self.ready[dst.0 as usize] = at + 1;
            if self.reg_of(i, dst)? == 0 {
                self.rf0 = Rf0::Unknown;
            }
        }
        match *op {
            Op71::Ldvary { .. } => {
                self.last_ldvary = Some(at);
                self.rf0 = Rf0::C;
            }
            Op71::Flags { .. } => self.last_flags = Some(at),
            Op71::Wrtmuc { .. } => self.last_wrtmuc = Some(at),
            Op71::Ldtmu { .. } => self.last_ldtmu = Some(at),
            Op71::Add { dst: Dst71::Magic(w), .. } | Op71::Mul { dst: Dst71::Magic(w), .. } if is_tmu(w) => {
                self.last_tmu_write = Some(at);
                if ends_tmu_sequence(w) {
                    self.last_tmu_end = Some(at);
                }
            }
            _ => {}
        }
        self.floor = at;
        Ok(())
    }

    /// The program end: a THRSW on the earliest of the last three words from
    /// which every word is valid in its thread-end slot, outside the last
    /// THRSW's delay slots, then NOPs for the slots no word fills (Mesa's
    /// emit_thrsw with is_thrend, qpu_schedule.c:2250-2385).
    pub fn thread_end(&mut self, a: &Arena<'a>) -> Result<(), Sched71Error> {
        let n = self.words.len();
        let from = self.last_thrsw.map_or(0, |t| t + 3);
        let mut k = n.saturating_sub(3).max(self.barrier());
        let mut merged = false;
        while k < n {
            let mut first = self.words.as_slice()[k];
            first.alu.sig.thrsw = true;
            if encode71(&Instr71::Alu(first.alu)).is_ok()
                && window_ok(self.words.as_slice(), Some((k, &first)), k, from, true)
            {
                self.words.as_mut_slice()[k] = first;
                merged = true;
                break;
            }
            k += 1;
        }
        if !merged {
            self.pad_to(k, a)?;
            let mut w = Word71::empty();
            w.alu.sig.thrsw = true;
            self.words.push(a, w)?;
        }
        self.pad_to(k + 3, a)?;
        self.section = from;
        self.last_thrsw = Some(k);
        self.floor = k + 3;
        Ok(())
    }

    /// The words and the uniform stream, in word order.
    pub fn finish(self, a: &Arena<'a>) -> Result<Packed71<'a>, Sched71Error> {
        let words = self.words.as_slice();
        let mut instrs: AVec<'a, Instr71> = AVec::with_capacity(a, words.len())?;
        let mut uniforms: AVec<'a, Uniform> = AVec::new();
        for (k, w) in words.iter().enumerate() {
            /* the instruction as its word decodes: codec71's canonical form,
             * FADD/FMIN's operands in the order that says which it is */
            let word = encode71(&Instr71::Alu(w.alu)).map_err(|e| Sched71Error::NotEncodable(k, e))?;
            let instr = decode71(word).map_err(|e| Sched71Error::NotEncodable(k, EncodeError::DoesNotDecode(e)))?;
            instrs.push(a, instr)?;
            if let Some(u) = w.uniform {
                uniforms.push(a, u)?;
            }
        }
        Ok(Packed71 { instrs: instrs.into_slice(), uniforms: uniforms.into_slice() })
    }
}

/// Pack `ops`, one straight-line run that ends the program, into words:
/// every operation placed in order, then the program end.
pub fn pack71<'a>(ops: &[Op71], al: &Allocation71<'_>, a: &Arena<'a>) -> Result<Packed71<'a>, Sched71Error> {
    if let Some(i) = ops.iter().position(Op71::is_control_flow) {
        return Err(Sched71Error::ControlFlow(i));
    }
    let mut p = Packer71::new(al, a)?;
    for (i, op) in ops.iter().enumerate() {
        p.place(i, op, ops.get(i + 1), a)?;
    }
    p.thread_end(a)?;
    p.finish(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;
    use crate::shaderc::online::texref::Texture;
    use crate::shaderc::online::v3d::isa::{small_imm_index, Pf, Uf};
    use crate::shaderc::online::v3d::listing71::write_instr71;
    use crate::shaderc::online::v3d::sim::{Env, FsInputs, Inputs, Outcome, Varying, VsInputs, LANES};
    use crate::shaderc::online::v3d::sim71::run71;
    use crate::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
    use crate::shaderc::online::v3d::vop71::{Label, Sfu71};
    use std::boxed::Box;
    use std::string::String;
    use std::vec;
    use std::vec::Vec;

    const W: f32 = 2.0;
    const Z: f32 = 0.5;

    fn v(n: u32) -> Src71 {
        Src71::V(V(n))
    }
    fn imm(x: u32) -> Src71 {
        Src71::SmallImm(small_imm_index(x).expect("a small immediate"))
    }
    fn u(x: f32) -> Uniform {
        Uniform::constant(x.to_bits())
    }

    /// `ops` with registers `regs`, packed, as instructions and uniform
    /// words.
    fn pack(ops: &[Op71], regs: &[u8]) -> Result<(Vec<Instr71>, Vec<u32>), Sched71Error> {
        let rfs: Vec<Rf> = regs.iter().map(|&r| Rf(r)).collect();
        store::with_scratch(|a| {
            let al = Allocation71 { reg: &rfs, rf_end: regs.iter().copied().max().unwrap_or(0) + 1 };
            pack71(ops, &al, a).map(|p| (p.instrs.to_vec(), p.uniforms.iter().map(|u| u.data).collect()))
        })
        .expect("scratch")
    }

    fn listing(instrs: &[Instr71]) -> String {
        let mut s = String::new();
        for (k, i) in instrs.iter().enumerate() {
            s.push_str(&std::format!("{k:3}: "));
            write_instr71(&mut s, i).unwrap();
            s.push('\n');
        }
        s
    }

    /// Pack, encode, decode back equal (the packer returns the decoded form), verify at 7.1 (4 threads), and run
    /// in sim71.
    fn run(ops: &[Op71], regs: &[u8], single_seg: bool, inputs: &Inputs, env: &Env) -> (Vec<Instr71>, Outcome) {
        let (instrs, unif) = pack(ops, regs).expect("packs");
        let text = listing(&instrs);
        let words: Vec<u64> =
            instrs.iter().map(|i| encode71(i).unwrap_or_else(|e| panic!("{e:?} encoding {i:?}\n{text}"))).collect();
        for (i, w) in instrs.iter().zip(&words) {
            assert_eq!(decode71(*w).expect("decodes"), *i, "{text}");
        }
        verify71(&instrs, single_seg).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        registers_fit_threads71(&instrs, 4).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        let o = run71(&words, &unif, inputs, env).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(o.uniforms_read, unif.len(), "{text}");
        (instrs, o)
    }

    fn fs_inputs(z: [u32; LANES], varyings: Vec<Varying>) -> Inputs {
        let f = |x: f32| [x.to_bits(); LANES];
        Inputs::Fragment(Box::new(FsInputs {
            w: f(W),
            w_centroid: f(W),
            z,
            x: f(1.0),
            y: f(1.0),
            back_facing: [false; LANES],
            varyings,
        }))
    }

    fn fs(ops: &[Op71], regs: &[u8], varyings: Vec<Varying>) -> (Vec<Instr71>, Outcome) {
        let env = Env { threads: 4, ..Env::default() };
        run(ops, regs, false, &fs_inputs([Z.to_bits(); LANES], varyings), &env)
    }

    /// The one value every lane of TLB write `n` holds.
    fn tlb_f(o: &Outcome, n: usize) -> f32 {
        let l = o.tlb[n].value;
        assert!(l.iter().all(|x| *x == l[0]), "{l:?}");
        f32::from_bits(l[0].expect("defined"))
    }

    fn alu(i: &Instr71) -> &Alu71 {
        match i {
            Instr71::Alu(a) => a,
            Instr71::Branch(_) => panic!("a branch"),
        }
    }

    /// The first instruction for which `f` holds.
    fn find(instrs: &[Instr71], f: impl Fn(&Alu71) -> bool) -> usize {
        instrs.iter().position(|i| f(alu(i))).expect("found")
    }

    /// interp_color.frag: two smooth varyings (`ldvary * W + C`), the
    /// last-thread-switch pair, both to the TLB. C is read two words after
    /// its `ldvary` (an earlier read gets an undefined rf0 in sim71).
    #[test]
    fn a_pass_through_fragment_shader() {
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(1)), a: v(0), b: Src71::Payload(3) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(2)), a: v(1), b: Src71::LdvaryC },
            Op71::Ldvary { dst: V(3) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(4)), a: v(3), b: Src71::Payload(3) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(5)), a: v(4), b: Src71::LdvaryC },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(2), b: Src71::None },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(5), b: Src71::None },
        ];
        let vary = |x: f32, c: f32| Varying { vary: [x.to_bits(); LANES], c: [c.to_bits(); LANES] };
        let (instrs, o) = fs(&ops, &[4, 5, 6, 7, 8, 9], vec![vary(3.0, 0.25), vary(-1.5, 4.0)]);
        assert_eq!(o.tlb.len(), 2);
        assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25);
        assert_eq!(tlb_f(&o, 1), -1.5 * W + 4.0);
        let ld = find(&instrs, |a| a.sig.ldvary);
        let c = find(&instrs, |a| a.add.op == AddOp71::Fadd);
        assert_eq!(c, ld + 2, "{}", listing(&instrs));
    }

    /// passthrough.vert's shape: two VPM rows in, one scaled by a uniform,
    /// both out. Each VPM access carries a small immediate, so no two share
    /// a word; single segment, ended by the first THRSW.
    #[test]
    fn a_pass_through_vertex_shader() {
        let ops = [
            Op71::Add { op: AddOp71::LdvpmvIn, dst: Dst71::V(V(0)), a: imm(0), b: Src71::None },
            Op71::Add { op: AddOp71::LdvpmvIn, dst: Dst71::V(V(1)), a: imm(1), b: Src71::None },
            Op71::Ldunif { dst: V(2), u: u(2.0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(3)), a: v(0), b: v(2) },
            Op71::Add { op: AddOp71::Stvpmv, dst: Dst71::None, a: imm(0), b: v(1) },
            Op71::Add { op: AddOp71::Stvpmv, dst: Dst71::None, a: imm(1), b: v(3) },
        ];
        let inputs = Inputs::Vertex(VsInputs { vpm: vec![[1.5f32.to_bits(); LANES], [7.0f32.to_bits(); LANES]] });
        let env = Env { threads: 4, ..Env::default() };
        let (instrs, o) = run(&ops, &[4, 5, 6, 7], true, &inputs, &env);
        assert_eq!(o.vpm_out, vec![Some([7.0f32.to_bits(); LANES]), Some([3.0f32.to_bits(); LANES])]);
        for i in &instrs {
            let s = alu(i).sig;
            assert!(s.small_imm_a as u8 + s.small_imm_b as u8 + s.small_imm_c as u8 + s.small_imm_d as u8 <= 1);
        }
        assert_eq!(instrs.iter().filter(|i| alu(i).sig.thrsw).count(), 1, "{}", listing(&instrs));
    }

    /// utri's shape: two uniforms added into a TLBU write, its configuration
    /// from the stream. The uniform alloc71 put in rf0 is a plain `ldunif`,
    /// the other an `ldunifrf`; the stream is in program order.
    #[test]
    fn a_uniform_colour() {
        const CFG: u32 = 0xffff_ff7a;
        let ops = [
            Op71::Ldunif { dst: V(0), u: u(0.25) },
            Op71::Ldunif { dst: V(1), u: u(0.5) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(2)), a: v(0), b: v(1) },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlbu(Uniform::constant(CFG)), a: v(2), b: Src71::None },
        ];
        let (instrs, o) = fs(&ops, &[4, 0, 5], Vec::new());
        assert_eq!(o.tlb.len(), 1);
        assert_eq!(o.tlb[0].config, Some(CFG));
        assert_eq!(tlb_f(&o, 0), 0.75);
        assert!(instrs.iter().any(|i| alu(i).sig.ldunif));
        assert!(instrs.iter().any(|i| alu(i).sig.ldunifrf && alu(i).sig_addr == 4));
        let (_, unif) = pack(&ops, &[4, 0, 5]).unwrap();
        assert_eq!(unif, [0.25f32.to_bits(), 0.5f32.to_bits(), CFG]);
    }

    /// A 2D look-up under one THRSW (the last-thread-switch pair at the
    /// look-up's switch, as select.rs places it): WRTMUC P0 and P1, T, S
    /// retiring it, the switch, LDTMU, the TLB. The THRSW moves back over the
    /// TMU writes, which run in its delay slots.
    #[test]
    fn a_texture_look_up_under_one_thread_switch() {
        let ops = [
            Op71::Ldunif { dst: V(0), u: u(0.25) },
            Op71::Ldunif { dst: V(1), u: u(0.75) },
            Op71::Wrtmuc { u: Uniform::constant(0x1) },
            Op71::Wrtmuc { u: Uniform::constant(0x1) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Magic(34), a: v(0), b: Src71::None }, // TMUT
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Magic(33), a: v(1), b: Src71::None }, // TMUS
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Ldtmu { dst: V(2) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(2), b: Src71::None },
        ];
        let texels = vec![[10.0, 0.0, 0.0, 1.0], [20.0, 0.0, 0.0, 1.0], [30.0, 0.0, 0.0, 1.0], [40.0, 0.0, 0.0, 1.0]];
        let env = Env { textures: vec![Texture::flat(2, 2, texels)], threads: 4, ..Env::default() };
        let (instrs, o) = run(&ops, &[4, 5, 6], false, &fs_inputs([Z.to_bits(); LANES], Vec::new()), &env);
        assert_eq!(o.tmu_lookups, 1);
        assert_eq!(o.ldtmus_without_switch, 0);
        assert_eq!(tlb_f(&o, 0), 20.0); // s 0.75, t 0.25: texel (1, 0)
        let t = find(&instrs, |a| a.sig.thrsw);
        let s = find(&instrs, |a| a.mul.magic && a.mul.waddr == 33);
        assert!(s > t && s <= t + 2, "the S write runs in a delay slot\n{}", listing(&instrs));
        let ld = find(&instrs, |a| a.sig.ldtmu);
        assert!(ld >= t + 3, "{}", listing(&instrs));
    }

    /// An SFU result is read two words later, and no SFU operation sits in a
    /// THRSW's slot 1 or 2: the pair does not move back over the second one.
    #[test]
    fn sfu_operations() {
        let ops = [
            Op71::Sfu { op: Sfu71::Recip, dst: V(0), a: Src71::Payload(3) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(1)), a: v(0), b: v(0) },
            Op71::Sfu { op: Sfu71::Rsqrt, dst: V(2), a: Src71::Payload(2) },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(1), b: Src71::None },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(2), b: Src71::None },
        ];
        let (instrs, o) = fs(&ops, &[4, 5, 6], Vec::new());
        assert_eq!(tlb_f(&o, 0), 2.0 / W);
        assert!((tlb_f(&o, 1) - 1.0 / Z.sqrt()).abs() < 1e-6);
        assert_eq!(o.approximate, ["recip", "rsqrt"]);
        let text = listing(&instrs);
        let r = find(&instrs, |a| a.add.op == AddOp71::Recip);
        assert_eq!(find(&instrs, |a| a.add.op == AddOp71::Fadd), r + 2, "{text}");
        let t = find(&instrs, |a| a.sig.thrsw);
        for (k, i) in instrs.iter().enumerate() {
            if is_sfu_op(alu(i).add.op) {
                assert!(!(t + 1..=t + 2).contains(&k), "{text}");
            }
        }
    }

    /// If-converted code: a flag push, then a conditional move where it
    /// holds (lanes whose Z is below 0.25). An `ldunifrf` names its register
    /// in the condition field, so neither shares its word.
    #[test]
    fn a_flag_push_and_a_conditional_move() {
        let ops = [
            Op71::Ldunif { dst: V(0), u: u(0.25) },
            Op71::Ldunif { dst: V(1), u: u(1.0) },
            Op71::Ldunif { dst: V(2), u: u(2.0) },
            Op71::Flags {
                unit: FlagOp71::Add(AddOp71::Fcmp),
                a: Src71::Payload(2),
                b: v(0),
                pf: Pf::PushN,
                uf: Uf::None,
            },
            Op71::CondMov { dst: V(1), src: v(2), cond: Cond::IfA },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(1), b: Src71::None },
        ];
        let mut z = [0u32; LANES];
        for (l, z) in z.iter_mut().enumerate() {
            *z = (l as f32 / LANES as f32).to_bits();
        }
        let env = Env { threads: 4, ..Env::default() };
        let (instrs, o) = run(&ops, &[4, 5, 6], false, &fs_inputs(z, Vec::new()), &env);
        for (l, x) in o.tlb[0].value.iter().enumerate() {
            let want = if (l as f32 / LANES as f32) < 0.25 { 2.0f32 } else { 1.0 };
            assert_eq!(*x, Some(want.to_bits()), "lane {l}");
        }
        for i in &instrs {
            let a = alu(i);
            assert!(!a.sig.writes_address() || a.flags == Flags::default());
        }
    }

    /// A mul MOV takes the add unit when the mul unit is taken
    /// (can_do_mul_as_add).
    #[test]
    fn a_mul_move_takes_a_free_add_unit() {
        let ops = [
            Op71::Ldunif { dst: V(0), u: u(3.0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(1)), a: v(0), b: Src71::Payload(3) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(V(2)), a: v(0), b: Src71::None },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(1), b: Src71::None },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(2), b: Src71::None },
        ];
        let (instrs, o) = fs(&ops, &[4, 5, 6], Vec::new());
        assert_eq!(tlb_f(&o, 0), 3.0 * W);
        assert_eq!(tlb_f(&o, 1), 3.0);
        let k = find(&instrs, |a| a.mul.op == MulOp71::Fmul);
        assert_eq!(alu(&instrs[k]).add.op, AddOp71::Mov, "{}", listing(&instrs));
    }

    /// What the packer refuses, by name.
    #[test]
    fn refusals() {
        let tlb = |n| Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(n), b: Src71::None };
        assert_eq!(pack(&[Op71::Thrsw, Op71::Label(Label(0))], &[]).err(), Some(Sched71Error::ControlFlow(1)));
        let fadd_c = Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(0)), a: Src71::Payload(3), b: Src71::LdvaryC };
        assert_eq!(pack(&[fadd_c], &[4]).err(), Some(Sched71Error::LdvaryCWithoutLdvary(0)));
        let cmov = Op71::CondMov { dst: V(0), src: Src71::Payload(3), cond: Cond::IfA };
        assert_eq!(pack(&[cmov], &[4]).err(), Some(Sched71Error::CondWithoutFlags(0)));
        assert_eq!(pack(&[Op71::Ldtmu { dst: V(0) }], &[4]).err(), Some(Sched71Error::LdtmuWithoutLookup(0)));
        assert_eq!(pack(&[tlb(3)], &[4]).err(), Some(Sched71Error::Unallocated(0)));
        assert_eq!(pack(&[Op71::Ldvary { dst: V(0) }], &[0]).err(), Some(Sched71Error::NotPlaceable(0)));
        /* a second plain ldunif replaces the first's rf0 before it is read */
        let two = [Op71::Ldunif { dst: V(0), u: u(1.0) }, Op71::Ldunif { dst: V(1), u: u(2.0) }, tlb(0)];
        assert_eq!(pack(&two, &[0, 0]).err(), Some(Sched71Error::Rf0Clobbered(2)));
        /* a plain ldunif after an ldvary, before C is read */
        let c = [Op71::Ldvary { dst: V(0) }, Op71::Ldunif { dst: V(1), u: u(1.0) }, fadd_c];
        assert_eq!(pack(&c, &[4, 0]).err(), Some(Sched71Error::Rf0Clobbered(2)));
        /* two small immediates in one operation do not encode */
        let two_imm = Op71::Add { op: AddOp71::Add, dst: Dst71::V(V(0)), a: imm(1), b: imm(2) };
        assert!(matches!(pack(&[two_imm], &[4]).err(), Some(Sched71Error::NotPlaceable(0))));
    }

    /// rf0 across a thread switch: an `ldvary`'s C lands in the word of an
    /// `ldunifrf` (a signal writing the register file), so the switch's last
    /// delay slot takes no such signal, or C is undefined after the switch
    /// (sim71 runs the hardware rule; qpu_schedule.c:1692-1745, 2110-2119).
    #[test]
    fn the_rf0_flops_conflict_keeps_the_last_delay_slot_free() {
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Ldunif { dst: V(1), u: u(1.0) },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Ldunif { dst: V(2), u: u(0.5) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(3)), a: v(0), b: Src71::Payload(3) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(4)), a: v(3), b: Src71::LdvaryC },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(5)), a: v(4), b: v(1) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(6)), a: v(5), b: v(2) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(6), b: Src71::None },
        ];
        let vary = Varying { vary: [3.0f32.to_bits(); LANES], c: [0.25f32.to_bits(); LANES] };
        let (instrs, o) = fs(&ops, &[4, 5, 6, 7, 8, 9, 10], vec![vary]);
        assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 1.0 + 0.5);
        let text = listing(&instrs);
        let t = find(&instrs, |a| a.sig.thrsw);
        let ld = find(&instrs, |a| a.sig.ldvary);
        assert!(alu(&instrs[ld + 1]).sig.ldunifrf, "the conflict is set up\n{text}");
        assert!(!alu(&instrs[t + 2]).sig.writes_address(), "{text}");
    }

    /// A plain `ldunif` never goes into the word right after an `ldvary`
    /// (rf0 then takes the ldvary's C).
    #[test]
    fn no_plain_ldunif_right_after_an_ldvary() {
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(1)), a: v(0), b: Src71::Payload(3) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(2)), a: v(1), b: Src71::LdvaryC },
            Op71::Ldunif { dst: V(3), u: u(0.5) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(4)), a: v(2), b: v(3) },
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(4), b: Src71::None },
        ];
        let vary = Varying { vary: [1.0f32.to_bits(); LANES], c: [0.125f32.to_bits(); LANES] };
        let (instrs, o) = fs(&ops, &[4, 5, 6, 0, 7], vec![vary]);
        assert_eq!(tlb_f(&o, 0), W + 0.125 + 0.5);
        let ld = find(&instrs, |a| a.sig.ldvary);
        assert!(find(&instrs, |a| a.sig.ldunif) >= ld + 2, "{}", listing(&instrs));
    }
}
