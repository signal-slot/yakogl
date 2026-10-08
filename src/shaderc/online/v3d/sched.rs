// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// SPDX-FileCopyrightText: 2010 Intel Corporation
// Derived from Mesa 26.2.1 src/broadcom/compiler/qpu_schedule.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Scheduling: the allocated operations ([`super::select`],
//! [`super::alloc`]) into 64-bit QPU words.
//!
//! # In order, merged, never reordered
//!
//! Each operation goes into the word the previous one went into, or a later
//! one. It shares a word with the previous operations when the word has room
//! (the add unit, the mul unit, the signal, the two register-file read ports,
//! the one small immediate, one uniform) and its operands are ready there;
//! otherwise it starts a new word, after as many NOPs as its operands need.
//! That is far less pairing than Mesa's list scheduler (qpu_schedule.c)
//! finds, and it is the point: the three orders the hardware depends on (the
//! uniform stream, the `ldvary` sequence, the TLB writes) are the program
//! order by construction, and the register allocator's "last read" is exact.
//!
//! One exception, [`hoist`] (2026-09-26): a plain result into a register
//! (arithmetic, a move, an SFU result, a uniform load) may go into an
//! earlier word's free unit, up to 32 words back, where its operands are
//! ready and no word from there on writes a register it reads or the
//! register it writes, and none after reads the register it writes (the
//! allocator's last read of the value before). It never crosses a THRSW
//! window, a branch, a label or a loop head. Nothing of the ordered
//! sequences moves; the uniform stream follows, as it is built from the
//! words' order afterwards.
//!
//! Whether a word can hold an operation is decided by the encoder: the
//! candidate word is encoded ([`super::encode::encode`]), and a combination
//! it refuses (two signals with no joint encoding, an operand pair its opcode
//! cannot say) is not made.
//!
//! # The rules, from Mesa 26.2.1 (MIT), each cited
//!
//! * A result is readable one instruction later; an SFU result (`recip`) two
//!   (instruction_latency, qpu_schedule.c:1859-1871). `ldvary`'s value is in
//!   its register the next instruction and its C term in r5 two later
//!   (qpu_validate.c:193-200).
//! * One uniform per instruction (an instruction has one `uniform` index),
//!   one of {TMU, SFU, VPM, TLB, TSY} (qpu_validate.c:333-353).
//! * A thread switch has two delay slots. Nothing that writes an
//!   accumulator, touches the TLB, issues `ldvary`, starts an SFU operation
//!   or is a thread switch goes into them
//!   (qpu_inst_after_thrsw_valid_in_delay_slot, qpu_schedule.c:2132-2215;
//!   this cut also keeps `recip` out, which Mesa refuses on 4.2 only for the
//!   legacy magic-register SFU, :2082-2086). An operation scheduled after a
//!   THRSW goes into its delay slots, never into the THRSW's own word
//!   (:2139-2143). The second of two consecutive THRSW is the last-thread-
//!   switch signal, the one thread switch a delay slot may hold
//!   (qpu_validate.c:357-373).
//! * A THRSW moves back onto one of the last three words placed, when they are
//!   valid in its delay slots ([`merge_back`]; emit_thrsw,
//!   qpu_schedule.c:2250-2330): a look-up's switch then takes the TMU
//!   writes before it as its delay slots instead of two NOPs.
//! * The program ends with a THRSW and its two delay slots (the "thrend").
//!   The THRSW moves back onto the last instructions when they are all valid
//!   in a thrend slot (emit_thrsw, qpu_schedule.c:2250-2330;
//!   qpu_inst_valid_in_thrend_slot, :1979-2061): on 4.2 no register-file
//!   write by either unit or a signal, no read of rf0..rf2, no uniform after
//!   slot 0, no `vpmwt`, no `ldvary`, no TMUWT in slot 2.

use super::alloc::{Allocation, Reg};
use super::encode::{encode, EncodeError};
use super::isa::*;
use super::select::{is_sfu, Dst, FlagOp, Operand, Selected, Uniform, VOp, V};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};

/// Magic write addresses this file writes (`enum v3d_qpu_waddr`).
/// Whether a TMU write ends a look-up's sequence (Mesa's
/// tmu_write_is_sequence_terminator, qpu_schedule.c:188-202: TMUS, TMUSCM,
/// TMUSF, TMUSLOD, TMUA, TMUAU; qpu_instr.h's numbers).
fn ends_tmu_sequence(waddr: u8) -> bool {
    matches!(waddr, 33 | 40 | 41 | 42 | 12 | 13)
}
const WADDR_TLB: u8 = 7;
const WADDR_TLBU: u8 = 8;

/// Why scheduling stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchedError {
    /// Operation `.0` does not encode even alone in a word.
    NotEncodable(usize, EncodeError),
    /// Operation `.0` reads more register-file registers than a word has
    /// read ports, or two different small immediates.
    TooManyReads(usize),
    /// Operation `.0` reads r5 with no `ldvary` before it.
    R5WithoutLdvary(usize),
    /// Operation `.0` is a conditional write with no flag push before it.
    CondWithoutFlags(usize),
    /// Operation `.0` is an `ldtmu` with no TMU look-up written before it.
    LdtmuWithoutLookup(usize),
    /// Operation `.0` is not something one word can hold (a TLB write on the
    /// mul unit).
    NotPlaceable(usize),
    OutOfScratch,
}

impl From<ArenaError> for SchedError {
    fn from(_: ArenaError) -> Self {
        SchedError::OutOfScratch
    }
}

/// The scheduled program.
#[derive(Clone, Copy, Debug)]
pub struct Scheduled<'a> {
    pub instrs: &'a [Instr],
    /// The uniform stream, in the order the program consumes it.
    pub uniforms: &'a [Uniform],
    /// Every counted loop's back edge, as (branch instruction, target
    /// instruction) (verify.rs branches_are_counted_loops).
    pub back_edges: &'a [(u32, u32)],
    /// Every uniform branch's forward branch, as (branch instruction,
    /// target instruction): the only other branches this back end emits.
    pub fwd_branches: &'a [(u32, u32)],
    /// Operations [`hoist`] moved into an earlier word.
    pub hoists: u32,
    /// Operations [`hoist`] would have moved further back without its
    /// [`HOIST_WINDOW`]: moves the cap cut (a measurement, step (o); counted
    /// in host-test builds only, 0 elsewhere).
    pub window_cuts: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortB {
    Rf(u8),
    Imm(u8),
}

#[derive(Clone, Copy)]
struct Word {
    alu: Alu,
    add_used: bool,
    mul_used: bool,
    sig_addr_used: bool,
    raddr_a: Option<u8>,
    raddr_b: Option<PortB>,
    uniform: Option<Uniform>,
    /// A TLB write is in this word (one of {TMU, SFU, VPM, TLB, TSY}).
    special: bool,
    /// This word pushes a flag / has a conditional write. The two never
    /// share a word here: which flags a condition sees in the word that
    /// also pushes is not something this cut relies on.
    flags_push: bool,
    flags_read: bool,
    /// A counted loop's back edge: this word is the branch (and nothing
    /// else); its offset and its uniform are fixed once every word is placed.
    branch: Option<Branch>,
}

/// Why an operation did not go into a word.
enum Refused {
    /// The unit, the signal address, the uniform or the special slot is
    /// taken.
    Full,
    /// The read ports cannot serve its operands.
    Ports,
    /// Never placeable (see [`SchedError::NotPlaceable`]).
    Never,
    Encode(EncodeError),
}

impl Word {
    fn empty() -> Word {
        Word {
            alu: Alu::nop(),
            add_used: false,
            mul_used: false,
            sig_addr_used: false,
            raddr_a: None,
            raddr_b: None,
            uniform: None,
            special: false,
            flags_push: false,
            flags_read: false,
            branch: None,
        }
    }

    /// An operand as a mux, claiming a read port for a register-file read.
    fn input(&mut self, al: &Allocation<'_>, o: Operand) -> Result<Input, Refused> {
        let rf = |w: &mut Word, n: u8| -> Result<Mux, Refused> {
            if w.raddr_a == Some(n) {
                Ok(Mux::A)
            } else if w.raddr_b == Some(PortB::Rf(n)) {
                Ok(Mux::B)
            } else if w.raddr_a.is_none() {
                w.raddr_a = Some(n);
                Ok(Mux::A)
            } else if w.raddr_b.is_none() {
                w.raddr_b = Some(PortB::Rf(n));
                Ok(Mux::B)
            } else {
                Err(Refused::Ports)
            }
        };
        let mux = match o {
            Operand::None => return Ok(Input::default()),
            Operand::V(v) | Operand::Abs(v) | Operand::Lo(v) | Operand::Hi(v) => match al.reg[v.0 as usize] {
                Reg::Acc(n) => Mux::from_bits(n as u32),
                Reg::Rf(n) => rf(self, n)?,
            },
            Operand::Payload(n) => rf(self, n)?,
            Operand::R5 => Mux::R5,
            Operand::SmallImm(i) => {
                match self.raddr_b {
                    None => self.raddr_b = Some(PortB::Imm(i)),
                    Some(PortB::Imm(j)) if j == i => {}
                    _ => return Err(Refused::Ports),
                }
                Mux::B
            }
        };
        let unpack = match o {
            Operand::Abs(_) => Unpack::Abs,
            Operand::Lo(_) => Unpack::L,
            Operand::Hi(_) => Unpack::H,
            _ => Unpack::None,
        };
        Ok(Input { mux, unpack })
    }

    /// The word's model with its ports written in.
    fn finish(&self) -> Alu {
        let mut alu = self.alu;
        alu.raddr_a = self.raddr_a.unwrap_or(0);
        match self.raddr_b {
            Some(PortB::Rf(n)) => alu.raddr_b = n,
            Some(PortB::Imm(i)) => {
                alu.raddr_b = i;
                alu.sig.small_imm_b = true;
            }
            None => alu.raddr_b = 0,
        }
        alu
    }
}

fn dst_of(al: &Allocation<'_>, d: Dst) -> (u8, bool, Option<Uniform>) {
    match d {
        Dst::V(v) => match al.reg[v.0 as usize] {
            Reg::Acc(n) => (n, true, None),
            Reg::Rf(n) => (n, false, None),
        },
        Dst::Tlb => (WADDR_TLB, true, None),
        Dst::Tlbu(u) => (WADDR_TLBU, true, Some(u)),
        Dst::Nop => (WADDR_NOP, true, None),
        Dst::None => (0, false, None),
        Dst::Magic(w) => (w, true, None),
    }
}

/// `w` with `op` added, or why not.
fn merge(w: &Word, al: &Allocation<'_>, op: &VOp) -> Result<Word, Refused> {
    if w.branch.is_some() {
        return Err(Refused::Full);
    }
    let mut w = *w;
    match *op {
        VOp::Copy { dst, src } => {
            return merge(&w, al, &VOp::Mul { op: MulOp::Mov, dst: Dst::V(dst), a: src, b: Operand::None });
        }
        VOp::LoopHead | VOp::BackEdge { .. } | VOp::FwdBranch { .. } | VOp::Label(_) => return Err(Refused::Never),
        VOp::Add { op, dst, a, b } => {
            if w.add_used {
                return Err(Refused::Full);
            }
            let ia = w.input(al, a)?;
            let ib = w.input(al, b)?;
            let (waddr, magic, u) = dst_of(al, dst);
            if u.is_some() {
                if w.uniform.is_some() {
                    return Err(Refused::Full);
                }
                w.uniform = u;
            }
            if matches!(dst, Dst::Tlb | Dst::Tlbu(_)) {
                if w.special {
                    return Err(Refused::Full);
                }
                w.special = true;
            }
            w.alu.add = AddAlu { op, a: ia, b: ib, waddr, magic, pack: OutPack::None };
            w.add_used = true;
        }
        VOp::Mul { op, dst, a, b } => {
            if w.mul_used {
                return Err(Refused::Full);
            }
            /* A TMU or TLB write takes the word's one peripheral access (one
             * of TMU, SFU, TSY, TLB, VPM: qpu_validate.c:333-343). A TLB
             * write here is the pass-through Z (Mesa's vir_MOV_dest(tlbu),
             * a mul-unit MOV); the colour writes are VFPACK on the add unit. */
            if matches!(dst, Dst::Magic(_) | Dst::Tlb | Dst::Tlbu(_)) {
                if w.special {
                    return Err(Refused::Full);
                }
                w.special = true;
            }
            let ia = w.input(al, a)?;
            let ib = w.input(al, b)?;
            let (waddr, magic, u) = dst_of(al, dst);
            if u.is_some() {
                if w.uniform.is_some() {
                    return Err(Refused::Full);
                }
                w.uniform = u;
            }
            w.alu.mul = MulAlu { op, a: ia, b: ib, waddr, magic, pack: OutPack::None };
            w.mul_used = true;
        }
        /* A load into r5 is a plain `ldunif` (alloc.rs gives r5 to a load
         * nothing else disturbs): no register named, so the condition field
         * stays free for a conditional move or a flag push in the word. */
        VOp::Ldunif { dst, u } if al.reg[dst.0 as usize] == Reg::Acc(5) => {
            if w.uniform.is_some() || w.alu.sig.ldvary || w.alu.sig.ldunif {
                return Err(Refused::Full);
            }
            w.alu.sig.ldunif = true;
            w.uniform = Some(u);
        }
        VOp::Ldunif { dst, u } => {
            if w.sig_addr_used || w.uniform.is_some() {
                return Err(Refused::Full);
            }
            let (addr, magic, _) = dst_of(al, Dst::V(dst));
            w.alu.sig.ldunifrf = true;
            w.alu.sig_addr = addr;
            w.alu.sig_magic = magic;
            w.sig_addr_used = true;
            w.uniform = Some(u);
        }
        VOp::Ldvary { dst } => {
            if w.sig_addr_used {
                return Err(Refused::Full);
            }
            let (addr, magic, _) = dst_of(al, Dst::V(dst));
            w.alu.sig.ldvary = true;
            w.alu.sig_addr = addr;
            w.alu.sig_magic = magic;
            w.sig_addr_used = true;
        }
        VOp::Flags { unit, a, b, pf, uf } => {
            if w.flags_push || w.flags_read {
                return Err(Refused::Full);
            }
            let ia = w.input(al, a)?;
            let ib = w.input(al, b)?;
            match unit {
                FlagOp::Add(op) => {
                    if w.add_used {
                        return Err(Refused::Full);
                    }
                    w.alu.add = AddAlu { op, a: ia, b: ib, waddr: WADDR_NOP, magic: true, pack: OutPack::None };
                    (w.alu.flags.apf, w.alu.flags.auf) = (pf, uf);
                    w.add_used = true;
                }
                FlagOp::Mul(op) => {
                    if w.mul_used {
                        return Err(Refused::Full);
                    }
                    w.alu.mul = MulAlu { op, a: ia, b: ib, waddr: WADDR_NOP, magic: true, pack: OutPack::None };
                    (w.alu.flags.mpf, w.alu.flags.muf) = (pf, uf);
                    w.mul_used = true;
                }
            }
            w.flags_push = true;
        }
        VOp::CondMov { dst, src, cond } => {
            if w.mul_used || w.flags_push {
                return Err(Refused::Full);
            }
            let ia = w.input(al, src)?;
            let (waddr, magic, _) = dst_of(al, Dst::V(dst));
            w.alu.mul = MulAlu { op: MulOp::Mov, a: ia, b: Input::default(), waddr, magic, pack: OutPack::None };
            w.alu.flags.mc = cond;
            w.mul_used = true;
            w.flags_read = true;
        }
        VOp::Setmsf { src, cond } => {
            if w.add_used || (cond != Cond::None && w.flags_push) {
                return Err(Refused::Full);
            }
            let ia = w.input(al, src)?;
            w.alu.add = AddAlu {
                op: AddOp::Setmsf,
                a: ia,
                b: Input::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            };
            if cond != Cond::None {
                w.alu.flags.ac = cond;
                w.flags_read = true;
            }
            w.add_used = true;
        }
        VOp::Wrtmuc { u } => {
            if w.uniform.is_some() || w.alu.sig.wrtmuc {
                return Err(Refused::Full);
            }
            w.alu.sig.wrtmuc = true;
            w.uniform = Some(u);
        }
        VOp::Ldtmu { dst } => {
            if w.sig_addr_used {
                return Err(Refused::Full);
            }
            let (addr, magic, _) = dst_of(al, Dst::V(dst));
            w.alu.sig.ldtmu = true;
            w.alu.sig_addr = addr;
            w.alu.sig_magic = magic;
            w.sig_addr_used = true;
        }
        VOp::Thrsw => return Err(Refused::Never),
    }
    /* One peripheral access per word, bar the two pairs 4.2 allows
     * (verify::peripherals_compatible): the `special` slot above covers the
     * magic writes, not an SFU op, a VPM access, `ldtmu` or `wrtmuc`. */
    let fin = w.finish();
    if !super::verify::peripherals_compatible(&fin) {
        return Err(Refused::Full);
    }
    encode(&Instr::Alu(fin)).map_err(Refused::Encode)?;
    Ok(w)
}

/// The latency of `op`'s result: how many instructions later it is readable.
fn latency(op: &VOp) -> usize {
    match op {
        VOp::Add { op, .. } if is_sfu(*op) => 2,
        _ => 1,
    }
}

/// `op` may go into a delay slot of a thread switch it comes after.
fn ok_after_thrsw(op: &VOp, al: &Allocation<'_>) -> bool {
    let writes_acc = |d: Dst| matches!(d, Dst::V(v) if matches!(al.reg[v.0 as usize], Reg::Acc(_)));
    match *op {
        /* Flags are invalidated across a thread switch (qpu_schedule.c:
         * 2201-2205): neither a push nor a conditional write goes here. */
        VOp::Thrsw
        | VOp::Ldvary { .. }
        | VOp::Add { op: AddOp::Recip | AddOp::Rsqrt | AddOp::Exp | AddOp::Log | AddOp::Sin | AddOp::Rsqrt2, .. }
        | VOp::Flags { .. }
        | VOp::CondMov { .. }
        | VOp::Setmsf { .. }
        | VOp::Wrtmuc { .. }
        | VOp::Ldtmu { .. } => false,
        VOp::Mul { dst: Dst::Magic(_), .. } => false,
        VOp::Add { dst, .. } | VOp::Mul { dst, .. } => !writes_acc(dst) && !matches!(dst, Dst::Tlb | Dst::Tlbu(_)),
        VOp::Ldunif { dst, .. } => !writes_acc(Dst::V(dst)),
        VOp::Copy { dst, .. } => !writes_acc(Dst::V(dst)),
        VOp::LoopHead | VOp::BackEdge { .. } | VOp::FwdBranch { .. } | VOp::Label(_) => false,
    }
}

/// Word `alu` may be thrend slot `slot` (qpu_inst_valid_in_thrend_slot and
/// qpu_inst_before_thrsw_valid_in_delay_slot, the 4.2 rules).
fn ok_in_thrend(alu: &Alu, uniform: bool, slot: usize) -> bool {
    if slot > 0 && uniform {
        return false;
    }
    if alu.add.op == AddOp::Vpmwt || alu.sig.ldvary {
        return false;
    }
    if slot > 0 && is_sfu(alu.add.op) {
        return false;
    }
    if slot == 2 && alu.add.op == AddOp::Tmuwt {
        return false;
    }
    /* No writing physical registers at the end. */
    if (alu.add.op != AddOp::Nop && !alu.add.magic) || (alu.mul.op != MulOp::Nop && !alu.mul.magic) {
        return false;
    }
    if alu.sig.writes_address() && !alu.sig_magic {
        return false;
    }
    /* rf0..rf2 may be overwritten during the delay slots by the next
     * fragment's setup. */
    let add_n = num_src(add_op_args(alu.add.op));
    let mul_n = num_src(mul_op_args(alu.mul.op));
    let uses = |m: Mux| {
        (add_n > 0 && alu.add.a.mux == m)
            || (add_n > 1 && alu.add.b.mux == m)
            || (mul_n > 0 && alu.mul.a.mux == m)
            || (mul_n > 1 && alu.mul.b.mux == m)
    };
    if alu.raddr_a < 3 && uses(Mux::A) {
        return false;
    }
    if alu.raddr_b < 3 && !alu.sig.small_imm_b && uses(Mux::B) {
        return false;
    }
    true
}

/// The earliest of the last three words placed that a THRSW can be merged
/// into (emit_thrsw, qpu_schedule.c:2250-2330, which walks back until
/// `slots_filled == 3`): the switch takes effect after two delay slots, so
/// the words already placed after it keep running before the switch, and
/// the slots hold work instead of NOPs. Words `k..` are then slots 0, 1
/// and 2 of the THRSW, as far as they exist; each must be valid
/// there (qpu_inst_before_thrsw_valid_in_delay_slot, :2070-2123: on 4.2 no
/// `ldvary` and no SFU operation after slot 0; Mesa refuses only the legacy
/// magic-register SFU there, this cut refuses every SFU, as
/// [`ok_after_thrsw`] does), no branch, and word `k` must encode with the
/// signal. For the last-thread-switch pair (`pair`) word `k + 1`, when it
/// exists, takes the second signal and must encode with it too
/// (:2316-2330, 2359-2370). Nothing before `barrier` (the previous THRSW's
/// window, a branch's delay slots, the head of the loop being built) is a
/// candidate. None when no placed word qualifies: the THRSW takes a word
/// of its own.
fn merge_back(words: &[Word], barrier: usize, pair: bool) -> Option<usize> {
    let n = words.len();
    let with_thrsw = |w: &Word| {
        let mut t = *w;
        t.alu.sig.thrsw = true;
        w.branch.is_none() && encode(&Instr::Alu(t.finish())).is_ok()
    };
    (n.saturating_sub(3).max(barrier)..n).find(|&k| {
        let slots_ok = words[k..].iter().enumerate().all(|(slot, w)| {
            let alu = w.finish();
            w.branch.is_none() && (slot == 0 || (!alu.sig.ldvary && !is_sfu(alu.add.op)))
        });
        slots_ok && with_thrsw(&words[k]) && (!pair || k + 1 >= n || with_thrsw(&words[k + 1]))
    })
}

/// Whether word `w` reads register `r` (an accumulator by its mux, a
/// register-file entry through a read port) in an operand of its units.
fn reads_reg(w: &Word, r: Reg) -> bool {
    let alu = w.finish();
    let ins = [
        (num_src(add_op_args(alu.add.op)) > 0, alu.add.a.mux),
        (num_src(add_op_args(alu.add.op)) > 1, alu.add.b.mux),
        (num_src(mul_op_args(alu.mul.op)) > 0, alu.mul.a.mux),
        (num_src(mul_op_args(alu.mul.op)) > 1, alu.mul.b.mux),
    ];
    ins.iter().any(|&(used, m)| {
        used && match r {
            Reg::Acc(n) => m as u8 == n,
            Reg::Rf(n) => {
                (m == Mux::A && alu.raddr_a == n) || (m == Mux::B && !alu.sig.small_imm_b && alu.raddr_b == n)
            }
        }
    })
}

/// Whether word `w` writes register `r`: a unit's result, a signal that
/// writes an address (ldunifrf, ldvary, ldtmu), or r5, which an `ldvary` or
/// a plain `ldunif` writes without naming it.
fn writes_reg(w: &Word, r: Reg) -> bool {
    let alu = w.finish();
    let (addr, magic) = match r {
        Reg::Acc(n) => (n, true),
        Reg::Rf(n) => (n, false),
    };
    let unit = |used: bool, waddr: u8, m: bool| used && waddr == addr && m == magic;
    unit(alu.add.op != AddOp::Nop, alu.add.waddr, alu.add.magic)
        || unit(alu.mul.op != MulOp::Nop, alu.mul.waddr, alu.mul.magic)
        || (alu.sig.writes_address() && alu.sig_addr == addr && alu.sig_magic == magic)
        || (r == Reg::Acc(5) && (alu.sig.ldvary || alu.sig.ldunif))
}

/// How far back [`hoist`] looks: words before the next operation's floor.
const HOIST_WINDOW: usize = 32;
#[cfg(test)]
std::thread_local! {
    /// Tests only: [`hoist`] off on this thread, for an A/B in one process
    /// (per thread, so tests running in parallel do not see each other's).
    pub(crate) static HOIST_OFF: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether [`hoist`] runs: always, except in a test that switched it off.
fn hoist_on() -> bool {
    #[cfg(test)]
    {
        !HOIST_OFF.with(|h| h.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// The value `op` writes when [`hoist`] may move it, or None: what may
/// move, by name (a list of what may not would be short by one: MULTOP /
/// UMUL24 share the hidden rtop, MSF reads the flags SETMSF sets, FLAFIRST
/// the condition flags, FDX / FDY the other lanes of the quad, the VPM
/// reads the segment the writes share): arithmetic, logic, conversions and
/// moves of registers, the SFU results, uniform loads; and none that reads
/// r5 (an ldvary's C term or a plain ldunif's value: the load before it
/// must not be passed). The allocator reads it too (alloc.rs: a value hoist
/// may move takes the register that lets it move furthest).
pub(super) fn hoistable(op: &VOp) -> Option<V> {
    if op.uses().contains(&Operand::R5) {
        return None;
    }
    let dst = match *op {
        VOp::Add {
            op:
                AddOp::Fadd
                | AddOp::Faddnf
                | AddOp::Fsub
                | AddOp::Fmin
                | AddOp::Fmax
                | AddOp::Add
                | AddOp::Sub
                | AddOp::Min
                | AddOp::Max
                | AddOp::Umin
                | AddOp::Umax
                | AddOp::Shl
                | AddOp::Shr
                | AddOp::Asr
                | AddOp::Ror
                | AddOp::And
                | AddOp::Or
                | AddOp::Xor
                | AddOp::Not
                | AddOp::Neg
                | AddOp::Itof
                | AddOp::Ftoin
                | AddOp::Ftoiz
                | AddOp::Ftouz
                | AddOp::Ftoc
                | AddOp::Ffloor
                | AddOp::Fceil
                | AddOp::Ftrunc
                | AddOp::Fround
                | AddOp::Recip
                | AddOp::Rsqrt
                | AddOp::Rsqrt2
                | AddOp::Exp
                | AddOp::Log
                | AddOp::Sin,
            dst: Dst::V(d),
            ..
        } => d,
        VOp::Mul { op: MulOp::Fmul | MulOp::Mov | MulOp::Fmov | MulOp::Add | MulOp::Sub, dst: Dst::V(d), .. } => d,
        VOp::Ldunif { dst, .. } => dst,
        _ => return None,
    };
    Some(dst)
}

/// An earlier word `op` can go into, before `floor` (the in-order place), or
/// None: a list scheduler's freedom inside this in-order one. Only a plain
/// result into a register moves (an ALU operation, a move, or a uniform
/// load: the uniform stream is built from the words' order afterwards, and
/// a branch's or back edge's stream offset from their positions). Nothing
/// that pushes or reads flags, touches r5, a TMU, the TLB or the VPM, or
/// is a thread switch, a load signal of the look-up or varying sequences,
/// or a loop's or branch's structure moves. Word `p` must be at or after
/// `barrier` (the last THRSW's window, a branch's delay slots, a loop head,
/// a label), where every operand is ready (`ready_at`), and hold the
/// operation (it encodes). Then the registers: nothing from word `p` on may
/// write a register it reads (an earlier write it must see, or the same
/// word's, which it would read before) or the register it writes, and
/// nothing after `p` may read the register it writes (an earlier read of
/// the old value, which the allocator's last read counts on).
///
/// A flag push (FCMP, XOR, SUB, MIN, MOV or FMOV with pushz/n/c) moves the
/// same way, where no word from `p` on pushes or reads the flags: a push
/// in between would reorder the A/B shift, a conditional write in between
/// would read this push instead of the one before (Mesa orders the flags
/// like a register, the last push against every read and push,
/// qpu_schedule.c:539-542). It writes no register. Moving it back lets the
/// words after it be a back edge's delay slots (branch_back).
fn hoist(
    words: &[Word],
    al: &Allocation<'_>,
    op: &VOp,
    floor: usize,
    barrier: usize,
    ready_at: usize,
    window: usize,
) -> Option<(usize, Word)> {
    if let VOp::Flags {
        unit: FlagOp::Add(AddOp::Fcmp | AddOp::Xor | AddOp::Sub | AddOp::Min) | FlagOp::Mul(MulOp::Mov | MulOp::Fmov),
        ..
    } = *op
    {
        let uses = op.uses();
        if uses.contains(&Operand::R5) {
            return None;
        }
        let reads: [Option<Reg>; 2] = uses.map(|o| o.vreg().map(|v| al.reg[v.0 as usize]));
        let flags =
            |w: &Word| w.flags_push || w.flags_read || matches!(w.alu.add.op, AddOp::Flafirst | AddOp::Flnafirst);
        let lo = barrier.max(ready_at).max(floor.saturating_sub(window));
        for p in lo..floor.min(words.len()) {
            let later = &words[p..];
            if later.iter().any(flags) || reads.iter().flatten().any(|&r| later.iter().any(|w| writes_reg(w, r))) {
                continue;
            }
            if let Ok(w) = merge(&words[p], al, op) {
                return Some((p, w));
            }
        }
        return None;
    }
    let dst = hoistable(op)?;
    let uses = op.uses();
    let d = al.reg[dst.0 as usize];
    /* r5 holds only a uniform load's value (alloc.rs); a load into it is a
     * plain ldunif, which the register rules below cover (an ldvary and
     * an ldunif write r5, `writes_reg`), and the caller's barrier keeps it
     * two words after the last ldvary. Nothing else writes r5. */
    if d == Reg::Acc(5) && !matches!(op, VOp::Ldunif { .. }) {
        return None;
    }
    let reads: [Option<Reg>; 2] = uses.map(|o| o.vreg().map(|v| al.reg[v.0 as usize]));
    let lo = barrier.max(ready_at).max(floor.saturating_sub(window));
    for p in lo..floor.min(words.len()) {
        let later = &words[p..];
        if later.iter().any(|w| writes_reg(w, d)) || later[1..].iter().any(|w| reads_reg(w, d)) {
            continue;
        }
        if reads.iter().flatten().any(|&r| later.iter().any(|w| writes_reg(w, r))) {
            continue;
        }
        if let Ok(w) = merge(&words[p], al, op) {
            return Some((p, w));
        }
    }
    None
}

/// Mesa's fixup_pipelined_ldvary (qpu_schedule.c): an `ldvary` just placed
/// in word `at` goes one word earlier, where that is safe, so that a
/// varying's FMUL shares a word with the next `ldvary` and its FADD with
/// the next FMUL, one word per varying instead of two. Its C term then
/// lands in r5 one word after the FADD of the varying before it, which
/// read r5 in word `at` at the latest. Word `at - 1` must be at or after
/// `barrier` (a loop head, a label, a branch's delay slots) and outside a
/// THRSW's delay slots, hold no signal that writes an address, read or
/// write no flags (ldvary takes the condition field), take the signal (it
/// encodes), and not write the ldvary's register. No word from `at` on may
/// read that register (it would see the new value early), none after `at`
/// may read r5 (the new C term lands there; word `at` still reads the one
/// before), and word `at` may hold no other load signal.
fn ldvary_back(words: &[Word], at: usize, barrier: usize, reg: Reg) -> Option<usize> {
    let k = at.checked_sub(1)?;
    if k < barrier {
        return None;
    }
    let prev = &words[k];
    if prev.branch.is_some() || prev.sig_addr_used || prev.flags_push || prev.flags_read {
        return None;
    }
    let alu = prev.finish();
    let (addr, magic) = match reg {
        Reg::Acc(n) => (n, true),
        Reg::Rf(n) => (n, false),
    };
    let writes = |op_used: bool, waddr: u8, m: bool| op_used && waddr == addr && m == magic;
    if writes(alu.add.op != AddOp::Nop, alu.add.waddr, alu.add.magic)
        || writes(alu.mul.op != MulOp::Nop, alu.mul.waddr, alu.mul.magic)
    {
        return None;
    }
    let here = words[at].alu.sig;
    if here.ldunif || here.ldunifrf {
        return None;
    }
    if words[at..].iter().any(|w| reads_reg(w, reg)) || words[at + 1..].iter().any(|w| reads_reg(w, Reg::Acc(5))) {
        return None;
    }
    let mut moved = *prev;
    moved.alu.sig.ldvary = true;
    moved.alu.sig_addr = words[at].alu.sig_addr;
    moved.alu.sig_magic = words[at].alu.sig_magic;
    moved.sig_addr_used = true;
    encode(&Instr::Alu(moved.finish())).ok()?;
    Some(k)
}

/// Whether word `w` may run in a branch's delay slots, moved there from
/// before the branch. Mesa's qpu_inst_valid_in_branch_delay_slot
/// (qpu_schedule.c:2396-2412): no branch, no thread switch, no UNIFA write,
/// no uniform (the branch moves the uniform stream: a uniform read after it
/// would read the target's). For a conditional branch, no flag push either
/// (emit_branch, :2479-2485: the branch reads the flags as it issues). This
/// cut is narrower than Mesa's, so that none of the placement's bookkeeping
/// (the look-up, varying and switch positions, the SFU latency) sees a word
/// move: no signal at all, no peripheral access (TMU, SFU, VPM, TLB, TSY),
/// no magic write but an accumulator or NOP (UNIFA among the rest), no
/// MULTOP or UMUL24 (verify.rs: a MULTOP in a branch delay slot does not
/// set rtop).
fn ok_in_branch_slot(w: &Word, conditional: bool) -> bool {
    let alu = w.finish();
    /* any signal but the small immediate, which is a read port */
    let signal = Sig { small_imm_b: false, ..alu.sig } != Sig::default();
    let other_magic = |used: bool, magic: bool, waddr: u8| used && magic && waddr > WADDR_NOP;
    /* Mesa's uniform rule; today every word with a uniform also has a
     * signal (ldunif, ldunifrf, wrtmuc) or a peripheral write (TLBU), which
     * the next lines refuse too */
    w.branch.is_none()
        && w.uniform.is_none()
        && !signal
        && !w.special
        && (!conditional || !w.flags_push)
        && !super::verify::touches_peripheral(&alu)
        && !other_magic(alu.add.op != AddOp::Nop, alu.add.magic, alu.add.waddr)
        && !other_magic(alu.mul.op != MulOp::Nop, alu.mul.magic, alu.mul.waddr)
        && !matches!(alu.mul.op, MulOp::Multop | MulOp::Umul24)
}

/// Mesa's emit_branch (qpu_schedule.c:2445-2502): a branch that would go
/// right after the last word placed (`at == words.len()`) moves back over up
/// to three of the words before it, which then run in its delay slots
/// instead of NOPs: they ran before the branch and still do, on both of its
/// paths. It stops at a word that may not be in a slot
/// ([`ok_in_branch_slot`]) and never lands before `lo`, which the caller
/// sets past a THRSW's delay slots and another branch's (:2453-2469), past
/// the word a loop head or a label names (the targets stay where they are),
/// and two words past an `ldvary`, whose C term is still to land in r5
/// (:2471-2477). A branch with msfign that tests anything but A never lands
/// right after a SETMSF (4.x, :2430-2440 and :2490-2498). Returns the
/// branch's word.
fn branch_back(words: &[Word], at: usize, lo: usize, conditional: bool, msf_safe: bool) -> usize {
    if at != words.len() {
        return at;
    }
    let mut p = at;
    while p > lo && at - p < 3 && ok_in_branch_slot(&words[p - 1], conditional) {
        if !msf_safe && p >= 2 && words[p - 2].alu.add.op == AddOp::Setmsf {
            break;
        }
        p -= 1;
    }
    p
}

#[cfg(test)]
std::thread_local! {
    /// Tests only: [`branch_back`] off on this thread (every branch keeps
    /// three NOP slots), for an A/B in one process.
    pub(crate) static BRANCH_BACK_OFF: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether [`branch_back`] runs: always, except in a test that switched it
/// off.
fn branch_back_on() -> bool {
    #[cfg(test)]
    {
        !BRANCH_BACK_OFF.with(|h| h.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// The earliest word a branch may move back to ([`branch_back`]): after a
/// THRSW's two delay slots and another branch's three, after the word the
/// open loop's head or the last label names, and three words after the
/// last `ldvary`.
fn branch_lo(
    last_thrsw: Option<usize>,
    last_branch: Option<usize>,
    head: Option<usize>,
    last_label: Option<usize>,
    last_ldvary: Option<usize>,
) -> usize {
    [
        last_thrsw.map(|t| t + 3),
        last_branch.map(|b| b + 4),
        head.map(|h| h + 1),
        last_label.map(|l| l + 1),
        last_ldvary.map(|l| l + 3),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0)
}

/// 4.x: a branch with msfign may follow a SETMSF only when it is
/// unconditional or tests A (emit_branch's is_safe_msf_branch,
/// qpu_schedule.c:2430-2438).
fn msf_safe(cond: BranchCond, msfign: Msfign) -> bool {
    msfign == Msfign::None || matches!(cond, BranchCond::Always | BranchCond::A0 | BranchCond::NA0)
}

/// Puts branch word `w` at word `at` (== the words placed so far), moved
/// back by [`branch_back`], then NOPs for the delay slots no moved word
/// fills. Returns the branch's word.
fn place_branch<'a>(
    words: &mut AVec<'a, Word>,
    a: &Arena<'a>,
    w: Word,
    at: usize,
    lo: usize,
    conditional: bool,
    msf_safe: bool,
) -> Result<usize, SchedError> {
    let p = if branch_back_on() { branch_back(words.as_slice(), at, lo, conditional, msf_safe) } else { at };
    words.push(a, w)?;
    words.as_mut_slice()[p..].rotate_right(1);
    while words.len() < p + 4 {
        words.push(a, Word::empty())?;
    }
    Ok(p)
}

/// Schedule `sel` with registers `al`.
pub fn schedule<'a>(sel: &Selected<'_>, al: &Allocation<'_>, a: &Arena<'a>) -> Result<Scheduled<'a>, SchedError> {
    let mut words: AVec<'a, Word> = AVec::new();
    /* Per virtual register, the first word its value can be read in. */
    let ready = a.alloc_filled(sel.vregs as usize, 0usize)?;
    /* The first word the next operation may go in. */
    let mut floor = 0usize;
    let mut last_ldvary: Option<usize> = None;
    /* The word of the last flag push: a condition reads it from the next
     * word on. */
    let mut last_flags: Option<usize> = None;
    /* The words of the last `wrtmuc` and of the last TMU register write: the
     * configuration comes before the look-up's writes, and its result is
     * read (ldtmu) only after the write that retires it (v3d_tex.c's
     * order: the config words, the coordinates with S last, then ldtmu). */
    let mut last_wrtmuc: Option<usize> = None;
    let mut last_tmu_write: Option<usize> = None;
    /* The word of the last write that ended a look-up's sequence: the next
     * look-up's configuration goes in a later word (Mesa: a wrtmuc reads
     * last_tmu_config, which a terminator writes, qpu_schedule.c:249-250,
     * 511-516, so the two are never merged into one instruction). With a
     * switch between every two look-ups this never bound; queued look-ups
     * (select.rs Sel::flush_tmu) put the next P0 right after an S write. */
    let mut last_tmu_end: Option<usize> = None;
    /* The word of the last THRSW that opened a delay window (the first of a
     * pair). */
    let mut last_thrsw: Option<usize> = None;
    let mut prev_was_thrsw = false;
    /* Counted loops: the open loop's head word, and every back edge as
     * (branch word, head word). */
    /* the heads of the loops open at this point, innermost last */
    let mut loop_heads: AVec<'a, usize> = AVec::new();
    let (mut hoists, mut window_cuts) = (0u32, 0u32);
    let mut back_edges: AVec<'a, (usize, usize)> = AVec::new();
    let mut last_branch: Option<usize> = None;
    /* Uniform branches: each label's word (usize::MAX until placed), each
     * forward branch as (branch word, label), and the last label's word: a
     * THRSW merged back, and the thrend, never reach across it. */
    let label_at = a.alloc_filled(sel.ops.iter().filter(|o| matches!(o, VOp::Label(_))).count(), usize::MAX)?;
    let mut fwd: AVec<'a, (usize, u32)> = AVec::new();
    let mut last_label: Option<usize> = None;
    for (i, op) in sel.ops.iter().enumerate() {
        if *op == VOp::LoopHead {
            /* A word of its own at the top of the body: nothing before the
             * loop is merged into it, nothing after it goes before it. */
            let at = words.len().max(floor);
            while words.len() < at {
                words.push(a, Word::empty())?;
            }
            loop_heads.push(a, at)?;
            floor = at;
            prev_was_thrsw = false;
            continue;
        }
        if let VOp::BackEdge { cond, msfign } = *op {
            let head = loop_heads.pop().ok_or(SchedError::NotPlaceable(i))?;
            /* After every word of the body and the flag push it reads, and
             * outside the last THRSW's delay slots (qpu_validate.c: no branch
             * in a THRSW's delay slots). */
            let mut at = words.len().max(floor);
            if let Some(f) = last_flags {
                at = at.max(f + 1);
            }
            if let Some(t) = last_thrsw {
                at = at.max(t + 3);
            }
            while words.len() < at {
                words.push(a, Word::empty())?;
            }
            let mut w = Word::empty();
            w.branch = Some(Branch {
                cond,
                msfign,
                bdi: BranchDest::Rel,
                ub: true,
                bdu: BranchDest::Rel,
                raddr_a: 0,
                offset: 0,
            });
            w.uniform = Some(Uniform::constant(0));
            /* The branch moves back over the words that can be its delay
             * slots (branch_back), after the head's word, and the rest of
             * the three slots are NOPs. */
            let lo = branch_lo(last_thrsw, last_branch, Some(head), last_label, last_ldvary);
            let at = place_branch(&mut words, a, w, at, lo, cond != BranchCond::Always, msf_safe(cond, msfign))?;
            back_edges.push(a, (at, head))?;
            last_branch = Some(at);
            floor = at + 4;
            prev_was_thrsw = false;
            continue;
        }
        if let VOp::Label(l) = *op {
            /* A word of its own starts here, as at a loop head: the
             * branches to it skip or join the code before it. */
            let at = words.len().max(floor);
            while words.len() < at {
                words.push(a, Word::empty())?;
            }
            *label_at.get_mut(l as usize).ok_or(SchedError::NotPlaceable(i))? = at;
            last_label = Some(at);
            floor = at;
            prev_was_thrsw = false;
            continue;
        }
        if let VOp::FwdBranch { cond, msfign, label } = *op {
            /* After every word before it and the flag push it reads, and
             * outside the last THRSW's delay slots; the words it moves back
             * over (branch_back) and NOPs are its three delay slots, as a
             * back edge's. */
            let mut at = words.len().max(floor);
            if cond != BranchCond::Always {
                at = at.max(last_flags.ok_or(SchedError::CondWithoutFlags(i))? + 1);
            }
            if let Some(t) = last_thrsw {
                at = at.max(t + 3);
            }
            while words.len() < at {
                words.push(a, Word::empty())?;
            }
            let mut w = Word::empty();
            w.branch = Some(Branch {
                cond,
                msfign,
                bdi: BranchDest::Rel,
                ub: true,
                bdu: BranchDest::Rel,
                raddr_a: 0,
                offset: 0,
            });
            w.uniform = Some(Uniform::constant(0));
            let head = loop_heads.as_slice().last().copied();
            let lo = branch_lo(last_thrsw, last_branch, head, last_label, last_ldvary);
            let at = place_branch(&mut words, a, w, at, lo, cond != BranchCond::Always, msf_safe(cond, msfign))?;
            fwd.push(a, (at, label))?;
            last_branch = Some(at);
            floor = at + 4;
            prev_was_thrsw = false;
            continue;
        }
        if *op == VOp::Thrsw {
            let at = if prev_was_thrsw {
                /* The second of the pair: the word after the first. */
                last_thrsw.ok_or(SchedError::NotPlaceable(i))? + 1
            } else {
                let pair = sel.ops.get(i + 1) == Some(&VOp::Thrsw);
                let barrier = [
                    last_thrsw.map(|t| t + 3),
                    last_branch.map(|b| b + 4),
                    loop_heads.as_slice().last().copied(),
                    last_label,
                ]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(0);
                match merge_back(words.as_slice(), barrier, pair) {
                    Some(k) => k,
                    None => words.len().max(floor).max(barrier),
                }
            };
            while words.len() < at {
                words.push(a, Word::empty())?;
            }
            if at < words.len() {
                words.as_mut_slice()[at].alu.sig.thrsw = true;
            } else {
                let mut w = Word::empty();
                w.alu.sig.thrsw = true;
                words.push(a, w)?;
            }
            if !prev_was_thrsw {
                last_thrsw = Some(at);
            }
            floor = floor.max(at + 1);
            prev_was_thrsw = true;
            continue;
        }
        /* A plain move whose source and destination the allocator put in
         * one register copies nothing: it is dropped, and its value is ready
         * when the source's is. It is select.rs's select (the else value
         * moved into the destination a conditional move then writes): after
         * coalescing, `mov r2, r2` (FemtoVG's texType selects, 2026-09-26).
         * Only MOV, a bit copy: FMOV applies an unpack or a float rule. */
        if let VOp::Mul { op: MulOp::Mov, dst: Dst::V(d), a: Operand::V(x), b: Operand::None }
        | VOp::Copy { dst: d, src: Operand::V(x) } = *op
        {
            if al.reg[d.0 as usize] == al.reg[x.0 as usize] {
                ready[d.0 as usize] = ready[d.0 as usize].max(ready[x.0 as usize]);
                continue;
            }
        }
        prev_was_thrsw = false;
        let alone = match merge(&Word::empty(), al, op) {
            Ok(w) => w,
            Err(Refused::Ports) => return Err(SchedError::TooManyReads(i)),
            Err(Refused::Encode(e)) => return Err(SchedError::NotEncodable(i, e)),
            Err(Refused::Full | Refused::Never) => return Err(SchedError::NotPlaceable(i)),
        };
        let mut at = floor;
        if matches!(op, VOp::CondMov { .. }) || matches!(op, VOp::Setmsf { cond, .. } if *cond != Cond::None) {
            at = at.max(last_flags.ok_or(SchedError::CondWithoutFlags(i))? + 1);
        }
        if matches!(op, VOp::Mul { dst: Dst::Magic(_), .. }) {
            if let Some(w) = last_wrtmuc {
                at = at.max(w + 1);
            }
        }
        if matches!(op, VOp::Ldtmu { .. }) {
            at = at.max(last_tmu_write.ok_or(SchedError::LdtmuWithoutLookup(i))? + 1);
        }
        if matches!(op, VOp::Wrtmuc { .. }) {
            if let Some(w) = last_tmu_end {
                at = at.max(w + 1);
            }
        }
        /* A plain ldunif writes r5 one word later; an ldvary's C term lands
         * there two words after it and is read in that word, so the ldunif
         * goes two words after the last ldvary or later (qpu_validate.c:
         * "LDUNIF after a LDVARY"). */
        if let VOp::Ldunif { dst, .. } = op {
            if al.reg[dst.0 as usize] == Reg::Acc(5) {
                if let Some(l) = last_ldvary {
                    at = at.max(l + 2);
                }
            }
        }
        /* Every register operand, whatever its unpack (vreg(): V, Abs, Lo, Hi),
         * waits for its value: a match on the variants by name missed Lo/Hi
         * when they were added, and an f16 unpack read its ldtmu in the
         * ldtmu's own word. */
        for o in op.uses() {
            if let Some(v) = o.vreg() {
                at = at.max(ready[v.0 as usize]);
            } else if o == Operand::R5 {
                at = at.max(last_ldvary.ok_or(SchedError::R5WithoutLdvary(i))? + 2);
            }
        }
        let barrier =
            [last_thrsw.map(|t| t + 3), last_branch.map(|b| b + 4), loop_heads.as_slice().last().copied(), last_label]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(0);
        /* a plain ldunif (into r5) stays two words after the last ldvary */
        let barrier = match (op, last_ldvary) {
            (VOp::Ldunif { dst, .. }, Some(l)) if al.reg[dst.0 as usize] == Reg::Acc(5) => barrier.max(l + 2),
            _ => barrier,
        };
        let ready_at = op.uses().iter().filter_map(|o| o.vreg()).map(|v| ready[v.0 as usize]).max().unwrap_or(0);
        let hoisted =
            if hoist_on() { hoist(words.as_slice(), al, op, at, barrier, ready_at, HOIST_WINDOW) } else { None };
        hoists += u32::from(hoisted.is_some());
        /* The window's cost, measured (step (o)): where the window, not the
         * barrier nor an operand, bounds the search, the same search without
         * it finding an earlier word is a move the cap cut. Host builds only
         * (the tests read it): the second search costs compile time, which
         * a board spends at glLinkProgram. */
        if cfg!(feature = "host-test") && hoist_on() && at.saturating_sub(HOIST_WINDOW) > barrier.max(ready_at) {
            let free = hoist(words.as_slice(), al, op, at, barrier, ready_at, usize::MAX);
            if free.is_some_and(|(pf, _)| hoisted.is_none_or(|(ph, _)| pf < ph)) {
                window_cuts += 1;
            }
        }
        if let Some((p, w)) = hoisted {
            words.as_mut_slice()[p] = w;
            at = p;
        }
        loop {
            if hoisted.is_some() {
                break;
            }
            let in_delay = last_thrsw.is_some_and(|t| at > t && at <= t + 2);
            if in_delay && !ok_after_thrsw(op, al) {
                at += 1;
                continue;
            }
            if at < words.len() {
                match merge(&words.as_slice()[at], al, op) {
                    Ok(w) => {
                        words.as_mut_slice()[at] = w;
                        break;
                    }
                    Err(_) => {
                        at += 1;
                        continue;
                    }
                }
            }
            while words.len() < at {
                words.push(a, Word::empty())?;
            }
            words.push(a, alone)?;
            break;
        }
        /* The word the next operation may go in: the one this went into,
         * even when its ldvary signal moves one word back below (a later
         * operation must never land before an earlier one: the allocator's
         * last read is exact only in that order). */
        /* a hoisted operation leaves the floor where it stood */
        let placed = if hoisted.is_some() { floor } else { at };
        if let VOp::Ldvary { dst } = *op {
            let barrier = [
                last_thrsw.map(|t| t + 3),
                last_branch.map(|b| b + 4),
                loop_heads.as_slice().last().copied(),
                last_label,
            ]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(0);
            if let Some(k) = ldvary_back(words.as_slice(), at, barrier, al.reg[dst.0 as usize]) {
                let (addr, magic) = (words.as_slice()[at].alu.sig_addr, words.as_slice()[at].alu.sig_magic);
                let w = &mut words.as_mut_slice()[at];
                w.alu.sig.ldvary = false;
                w.alu.sig_addr = 0;
                w.alu.sig_magic = false;
                w.sig_addr_used = false;
                let p = &mut words.as_mut_slice()[k];
                p.alu.sig.ldvary = true;
                p.alu.sig_addr = addr;
                p.alu.sig_magic = magic;
                p.sig_addr_used = true;
                at = k;
            }
        }
        if let Some(v) = op.def() {
            ready[v.0 as usize] = at + latency(op);
        }
        /* A conditional move writes its destination without defining it (the
         * plain move before it did): a reader must still wait for it, or it
         * reads the value from before the move, and its result, landing in
         * the same cycle, collides with the move's (two writes to one
         * register in one instruction, verify.rs). */
        if let VOp::CondMov { dst, .. } | VOp::Copy { dst, .. } = op {
            ready[dst.0 as usize] = at + 1;
        }
        if matches!(op, VOp::Ldvary { .. }) {
            last_ldvary = Some(at);
        }
        if matches!(op, VOp::Flags { .. }) {
            last_flags = Some(at);
        }
        if matches!(op, VOp::Wrtmuc { .. }) {
            last_wrtmuc = Some(at);
        }
        if matches!(op, VOp::Mul { dst: Dst::Magic(_), .. }) {
            last_tmu_write = Some(at);
        }
        if matches!(op, VOp::Mul { dst: Dst::Magic(w), .. } if ends_tmu_sequence(*w)) {
            last_tmu_end = Some(at);
        }
        floor = placed;
    }

    /* The thrend: a THRSW on the earliest word from which every word to the
     * end is valid in its thrend slot, outside the last THRSW's delay slots,
     * and no more than two words from the end. */
    let n = words.len();
    let mut k = n
        .saturating_sub(2)
        .max(last_thrsw.map_or(0, |t| t + 3))
        .max(last_branch.map_or(0, |b| b + 4))
        .max(last_label.unwrap_or(0));
    while k < n {
        let ws = &words.as_slice()[k..n];
        let slots_ok = ws.iter().enumerate().all(|(s, w)| ok_in_thrend(&w.finish(), w.uniform.is_some(), s));
        let mut first = words.as_slice()[k];
        first.alu.sig.thrsw = true;
        if slots_ok && encode(&Instr::Alu(first.finish())).is_ok() {
            words.as_mut_slice()[k] = first;
            break;
        }
        k += 1;
    }
    if k >= n {
        while words.len() < k {
            words.push(a, Word::empty())?;
        }
        let mut w = Word::empty();
        w.alu.sig.thrsw = true;
        words.push(a, w)?;
    }
    while words.len() < k + 3 {
        words.push(a, Word::empty())?;
    }

    /* Each back edge: the REL offset from the word after its delay slots
     * back to the head, and the uniform stream moved back to the head's
     * first uniform (qpu_set_branch_targets, qpu_schedule.c:2916-2983: the
     * branch's own uniform is the byte offset from the one after it). */
    let uniforms_before = |words: &[Word], w: usize| words[..w].iter().filter(|x| x.uniform.is_some()).count() as i64;
    let mut edges: AVec<'a, (u32, u32)> = AVec::new();
    for &(b, h) in back_edges.as_slice() {
        let hu = uniforms_before(words.as_slice(), h);
        let bu = uniforms_before(words.as_slice(), b);
        let w = &mut words.as_mut_slice()[b];
        let br = w.branch.as_mut().ok_or(SchedError::NotPlaceable(0))?;
        br.offset = ((h as i64 - (b as i64 + 4)) * 8) as u32;
        w.uniform = Some(Uniform::constant(((hu - (bu + 1)) * 4) as u32));
        edges.push(a, (b as u32, h as u32))?;
    }
    /* Each forward branch: the same, to its label's word. */
    let mut fwd_out: AVec<'a, (u32, u32)> = AVec::new();
    for &(b, l) in fwd.as_slice() {
        let t = *label_at.get(l as usize).ok_or(SchedError::NotPlaceable(0))?;
        if t == usize::MAX || t <= b + 3 {
            return Err(SchedError::NotPlaceable(0));
        }
        let tu = uniforms_before(words.as_slice(), t);
        let bu = uniforms_before(words.as_slice(), b);
        let w = &mut words.as_mut_slice()[b];
        let br = w.branch.as_mut().ok_or(SchedError::NotPlaceable(0))?;
        br.offset = ((t as i64 - (b as i64 + 4)) * 8) as u32;
        w.uniform = Some(Uniform::constant(((tu - (bu + 1)) * 4) as u32));
        fwd_out.push(a, (b as u32, t as u32))?;
    }

    let mut instrs: AVec<'a, Instr> = AVec::with_capacity(a, words.len())?;
    let mut uniforms: AVec<'a, Uniform> = AVec::new();
    for w in words.as_slice() {
        match w.branch {
            Some(br) => {
                encode(&Instr::Branch(br)).map_err(|e| SchedError::NotEncodable(0, e))?;
                instrs.push(a, Instr::Branch(br))?;
            }
            None => instrs.push(a, Instr::Alu(w.finish()))?,
        }
        if let Some(u) = w.uniform {
            uniforms.push(a, u)?;
        }
    }
    Ok(Scheduled {
        instrs: instrs.into_slice(),
        uniforms: uniforms.into_slice(),
        back_edges: edges.into_slice(),
        fwd_branches: fwd_out.into_slice(),
        hoists,
        window_cuts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;
    use crate::shaderc::online::v3d::alloc::RF_FIRST;
    use crate::shaderc::online::v3d::listing::write_instr;
    use crate::shaderc::online::v3d::select::V;
    use std::string::String;
    use std::vec::Vec;

    /// `ops` with registers `regs` (indexed by virtual register), scheduled,
    /// as listing lines.
    fn listing(ops: &[VOp], regs: &[Reg]) -> Vec<String> {
        store::with_scratch(|a| {
            let sel = Selected {
                ops,
                fragment: true,
                vregs: regs.len() as u32,
                vattr_sizes: [0; 16],
                vpm_output_rows: 0,
                uses_vid: false,
                uses_iid: false,
                uses_center_w: false,
                implicit_point_varyings: false,
                writes_z: false,
                disable_ez: false,
            };
            let al = Allocation { reg: regs, rf_end: RF_FIRST };
            let s = schedule(&sel, &al, a).expect("schedules");
            s.instrs
                .iter()
                .map(|i| {
                    let mut t = String::new();
                    write_instr(&mut t, i).unwrap();
                    t
                })
                .collect()
        })
        .expect("scratch")
    }

    /// A uniform load in a fragment shader whose value is read at once takes
    /// r5 (alloc.rs) and is a plain `ldunif` (no register named, so the word
    /// keeps its condition field: here the load shares the word of a
    /// conditional move); one whose value lives across an `ldvary` (its C
    /// term lands in r5) does not, and neither does one in a vertex shader.
    /// Revert checks: without the ldvary rule the second load takes r5; with
    /// no r5 at all the first is an `ldunifrf`.
    #[test]
    fn a_short_uniform_load_in_a_fragment_shader_is_a_plain_ldunif() {
        use crate::shaderc::online::v3d::alloc::allocate;
        /* the varying, four operations (so the load is past R5_AFTER), the
         * load, its reader */
        let ops = [
            VOp::Ldvary { dst: V(0) },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(1)), a: Operand::V(V(0)), b: Operand::Payload(0) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(1)), b: Operand::R5 },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(5)), a: Operand::V(V(2)), b: Operand::V(V(2)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(6)), a: Operand::V(V(5)), b: Operand::V(V(5)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(7)), a: Operand::V(V(6)), b: Operand::V(V(6)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(8)), a: Operand::V(V(7)), b: Operand::V(V(7)) },
            VOp::Ldunif { dst: V(3), u: Uniform::constant(7) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(4)), a: Operand::V(V(8)), b: Operand::V(V(3)) },
        ];
        let listing_of = |ops: &[VOp], fragment: bool| -> (Vec<Reg>, Vec<String>) {
            store::with_scratch(|a| {
                let sel = Selected {
                    ops,
                    fragment,
                    vregs: 9,
                    vattr_sizes: [0; 16],
                    vpm_output_rows: 0,
                    uses_vid: false,
                    uses_iid: false,
                    uses_center_w: false,
                    implicit_point_varyings: false,
                    writes_z: false,
                    disable_ez: false,
                };
                let al = allocate(&sel, 4, a).expect("allocates");
                let s = schedule(&sel, &al, a).expect("schedules");
                let text = s
                    .instrs
                    .iter()
                    .map(|i| {
                        let mut t = String::new();
                        write_instr(&mut t, i).unwrap();
                        t
                    })
                    .collect();
                (al.reg.to_vec(), text)
            })
            .expect("scratch")
        };
        let (regs, text) = listing_of(&ops, true);
        assert_eq!(regs[3], Reg::Acc(5), "read at once: r5");
        assert!(text.iter().any(|t| t.ends_with("; ldunif")), "a plain ldunif: {text:#?}");
        /* a load read after an ldvary, within R5_SPAN: only the ldvary rule
         * keeps it out of r5 */
        let ops2 = [
            VOp::Ldunif { dst: V(3), u: Uniform::constant(7) },
            VOp::Ldvary { dst: V(0) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(4)), a: Operand::V(V(3)), b: Operand::Payload(0) },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(1)), a: Operand::V(V(0)), b: Operand::Payload(0) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(1)), b: Operand::R5 },
        ];
        let (regs, _) = listing_of(&ops2, true);
        assert_ne!(regs[3], Reg::Acc(5), "live across an ldvary: not r5");
        let (regs, _) = listing_of(&ops, false);
        assert_ne!(regs[3], Reg::Acc(5), "a vertex shader's load: not r5");
        /* a load right after the varyings (within R5_AFTER operations of a
         * read of the C term): an ldunifrf, which can share their words */
        let ops3 = [
            VOp::Ldvary { dst: V(0) },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(1)), a: Operand::V(V(0)), b: Operand::Payload(0) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(1)), b: Operand::R5 },
            VOp::Ldunif { dst: V(3), u: Uniform::constant(7) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(4)), a: Operand::V(V(3)), b: Operand::V(V(3)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(5)), a: Operand::V(V(2)), b: Operand::V(V(4)) },
        ];
        let (regs, _) = listing_of(&ops3, true);
        assert_ne!(regs[3], Reg::Acc(5), "right after the varyings: not r5");
    }

    /// An operation moves into an earlier word only where no word from
    /// there on still reads the register it writes (sched.rs hoist; the
    /// allocator gave the register to it because the value before was last
    /// read earlier in program order). Here `mov r0, rf0` comes after a FADD
    /// in word 2 that reads the old r0 (it waits for rf3, loaded in word 1);
    /// word 1 has a free mul unit and a free read port, so encoding would
    /// allow the move there, and only the read rule keeps it in word 2 (the
    /// same word as the read, which reads before it writes). And where
    /// nothing reads r0 later, the move does go back into a free unit.
    /// Revert check: without the read rule the move lands in word 1.
    #[test]
    fn a_hoisted_write_waits_for_the_last_read_of_the_old_value() {
        let (r0, r1, r2, r3) = (Reg::Acc(0), Reg::Acc(1), Reg::Acc(2), Reg::Rf(3));
        let one = Operand::SmallImm(1);
        let ops = [
            VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(0)), a: Operand::SmallImm(2), b: Operand::None },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(5)), a: Operand::V(V(0)), b: Operand::V(V(0)) },
            VOp::Ldunif { dst: V(1), u: Uniform::constant(11) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(0)), b: Operand::V(V(1)) },
            VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(3)), a: Operand::Payload(0), b: Operand::None },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(4)), a: Operand::V(V(2)), b: Operand::V(V(3)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(6)), a: Operand::V(V(4)), b: Operand::V(V(5)) },
        ];
        let regs = [r0, r3, r2, r0, r2, r1, r1];
        let text = listing(&ops, &regs);
        let read = text.iter().position(|t| t.starts_with("fadd r2, r0, rf3")).expect("the read of the old r0");
        let mov = text.iter().position(|t| t.contains("mov r0, rf0")).expect("the move");
        assert!(mov >= read, "the move writes r0 at or after its last read: {text:#?}");
        /* the write rule: an earlier FADD whose value nothing reads (word
         * 1) leaves r1 free for the move after it, whose in-order place is
         * word 2 (after a FADD that waits for a load in word 1); hoisted
         * above the dead FADD, into word 0, the move would be overwritten
         * by it. Revert check: without the write rule it lands in word 0.
         * (A dead value reaches the scheduler only in a hand-made program:
         * prepare removes them.) */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(11) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(1)), a: Operand::V(V(0)), b: Operand::V(V(0)) },
            VOp::Ldunif { dst: V(4), u: Uniform::constant(12) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(5)), a: Operand::V(V(4)), b: Operand::V(V(4)) },
            VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(2)), a: Operand::Payload(0), b: Operand::None },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(3)), a: Operand::V(V(2)), b: Operand::V(V(5)) },
        ];
        let regs = [r3, r1, r1, Reg::Acc(3), Reg::Rf(4), r2];
        let text = listing(&ops, &regs);
        let dead = text.iter().position(|t| t.starts_with("fadd r1, rf3, rf3")).expect("the dead write");
        let mov = text.iter().position(|t| t.contains("mov r1, rf0")).expect("the move");
        assert!(mov > dead, "the move writes r1 after the earlier write: {text:#?}");
        /* where nothing reads r0 after its in-order place's earlier words,
         * the move goes back into a free unit: its in-order place is word 3
         * (after the FADD that waits for the first FADD's result), and it
         * lands in an earlier word */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(10) },
            VOp::Ldunif { dst: V(1), u: Uniform::constant(11) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(1)), b: Operand::V(V(1)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(5)), a: Operand::V(V(2)), b: Operand::V(V(2)) },
            VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(3)), a: one, b: Operand::None },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(4)), a: Operand::V(V(0)), b: Operand::V(V(3)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(6)), a: Operand::V(V(4)), b: Operand::V(V(5)) },
        ];
        let regs = [r2, r3, r1, r0, r1, Reg::Acc(3), r2];
        let text = listing(&ops, &regs);
        let second = text.iter().position(|t| t.starts_with("fadd r3, r1, r1")).expect("the second FADD");
        let mov = text.iter().position(|t| t.contains("mov r0, ")).expect("the move");
        assert!(mov < second, "hoisted before its in-order place: {text:#?}");
    }

    /// An empty word, as the listing prints it (trimmed).
    const NOP: &str = "nop                           ; nop";

    /// `listing` with branch_back switched off or on.
    fn listing_with(ops: &[VOp], regs: &[Reg], branch_back: bool) -> Vec<String> {
        BRANCH_BACK_OFF.with(|h| h.set(!branch_back));
        let text = listing(ops, regs);
        BRANCH_BACK_OFF.with(|h| h.set(false));
        text
    }

    /// The window's cut, in the shape the corpus shows it (step (o)): a
    /// chain of 40 dependent FADDs fills words 0..39, then a uniform load
    /// into a register nothing touched could go to word 0 (no barrier, no
    /// operand), but hoist looks back only HOIST_WINDOW words from its place
    /// (40): it lands in word 8, and window_cuts counts the move the window
    /// stopped. Revert check: without the count the second assertion fails.
    #[test]
    fn a_load_the_window_stops_short_is_counted_as_a_cut() {
        use crate::shaderc::online::v3d::select::{Dst, Uniform};
        let mut ops: Vec<VOp> = (0..40u32)
            .map(|k| VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(k + 1)), a: Operand::V(V(k)), b: Operand::V(V(k)) })
            .collect();
        ops.insert(0, VOp::Ldunif { dst: V(0), u: Uniform::constant(1) });
        ops.push(VOp::Ldunif { dst: V(41), u: Uniform::constant(2) });
        ops.push(VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(42)), a: Operand::V(V(41)), b: Operand::V(V(40)) });
        let mut regs = std::vec![Reg::Rf(RF_FIRST); 43];
        (regs[41], regs[42]) = (Reg::Acc(1), Reg::Acc(2));
        let (words, hoists, cuts) = store::with_scratch(|a| {
            let sel = Selected {
                ops: &ops,
                fragment: false,
                vregs: regs.len() as u32,
                vattr_sizes: [0; 16],
                vpm_output_rows: 0,
                uses_vid: false,
                uses_iid: false,
                uses_center_w: false,
                implicit_point_varyings: false,
                writes_z: false,
                disable_ez: false,
            };
            let al = Allocation { reg: &regs, rf_end: RF_FIRST + 1 };
            let s = schedule(&sel, &al, a).expect("schedules");
            let text: Vec<String> = s
                .instrs
                .iter()
                .map(|i| {
                    let mut t = String::new();
                    write_instr(&mut t, i).unwrap();
                    t
                })
                .collect();
            (text, s.hoists, s.window_cuts)
        })
        .expect("scratch");
        let load = words.iter().position(|w| w.contains("ldunifrf.r1")).expect("the late load");
        assert_eq!(load, 40 - HOIST_WINDOW, "hoisted as far as the window allows: {words:#?}");
        assert_eq!((hoists, cuts), (1, 1), "one move, cut by the window: {words:#?}");
    }

    /// A counted loop's back edge (Mesa's emit_branch, qpu_schedule.c:
    /// 2445-2502): the counter's flag push comes last in program order, and
    /// it goes back into the free mul unit of the FADD's word (a two-op word:
    /// `fadd ; mov.pushz`, hoist's flag rule), so the FMUL after it is not
    /// needed by the branch and runs in its first delay slot, one NOP fewer.
    /// The words: the counter load, `sub`, `fadd ; mov.pushz`, the branch,
    /// `fmul`, two NOPs. Revert checks: with branch_back off the FMUL is
    /// before the branch and three NOPs follow it (the second listing); with
    /// hoist's flag rule off the push takes a word of its own after the
    /// FMUL and the branch cannot move over it.
    #[test]
    fn a_back_edge_takes_the_work_after_its_flag_push_as_a_delay_slot() {
        let (c, r0, r1) = (Reg::Rf(3), Reg::Acc(0), Reg::Acc(1));
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(24) },
            VOp::LoopHead,
            VOp::Add { op: AddOp::Sub, dst: Dst::V(V(1)), a: Operand::V(V(0)), b: Operand::SmallImm(1) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::Payload(0), b: Operand::Payload(0) },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(3)), a: Operand::V(V(2)), b: Operand::V(V(2)) },
            VOp::Flags {
                unit: FlagOp::Mul(MulOp::Mov),
                a: Operand::V(V(1)),
                b: Operand::None,
                pf: Pf::PushZ,
                uf: Uf::None,
            },
            VOp::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None },
        ];
        let regs = [c, c, r0, r1];
        let text = listing_with(&ops, &regs, true);
        let br = text.iter().position(|t| t.starts_with("bu.anyna")).expect("the back edge");
        assert!(
            text[br - 1].starts_with("fadd r0, rf0, rf0") && text[br - 1].contains("mov.pushz"),
            "the push shares the FADD's word, just before the branch: {text:#?}"
        );
        assert!(text[br + 1].contains("fmul r1, r0, r0"), "the FMUL in the first delay slot: {text:#?}");
        assert!(text[br + 2..br + 4].iter().all(|t| t.trim() == NOP), "then two NOPs: {text:#?}");
        let off = listing_with(&ops, &regs, false);
        let br_off = off.iter().position(|t| t.starts_with("bu.anyna")).expect("the back edge");
        assert!(off[br_off - 1].contains("fmul r1, r0, r0"), "without branch_back the FMUL is before: {off:#?}");
        assert_eq!(off.len(), text.len() + 1, "one word more: {off:#?}");
    }

    /// What a branch does not move over: its flag push (a conditional branch
    /// reads the flags as it issues), a word with a uniform (the branch moves
    /// the stream), the word its loop's head names (the target stays), and
    /// never more than three words. An unconditional branch moves over a
    /// flag push (Mesa: only a conditional one stops there, :2479-2485).
    #[test]
    fn a_branch_never_moves_over_its_flags_a_uniform_or_its_loops_head() {
        let (c, r0, r1, r2) = (Reg::Rf(3), Reg::Acc(0), Reg::Acc(1), Reg::Acc(2));
        let push = |v: u32| VOp::Flags {
            unit: FlagOp::Mul(MulOp::Mov),
            a: Operand::V(V(v)),
            b: Operand::None,
            pf: Pf::PushZ,
            uf: Uf::None,
        };
        let fadd =
            |d: u32| VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(d)), a: Operand::Payload(0), b: Operand::Payload(0) };
        /* the body's one word is the head's: the branch stays after it */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(24) },
            push(0),
            VOp::LoopHead,
            fadd(1),
            VOp::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None },
        ];
        let text = listing_with(&ops, &[c, r0], true);
        let br = text.iter().position(|t| t.starts_with("bu.anyna")).expect("the back edge");
        assert!(text[br - 1].starts_with("fadd r0"), "the head's word before the branch: {text:#?}");
        /* a uniform load in the last word: the branch stays after it */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(24) },
            VOp::LoopHead,
            push(0),
            fadd(1),
            VOp::Ldunif { dst: V(2), u: Uniform::constant(7) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(3)), a: Operand::V(V(1)), b: Operand::V(V(2)) },
            VOp::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None },
        ];
        let text = listing_with(&ops, &[c, r0, Reg::Rf(4), r1], true);
        let br = text.iter().position(|t| t.starts_with("bu.anyna")).expect("the back edge");
        let unif = text.iter().position(|t| t.contains("ldunifrf.rf4")).expect("the load");
        assert!(unif < br, "the uniform load stays before the branch: {text:#?}");
        /* at most three words: the push shares the first FADD's word, four
         * FADD words follow, the branch moves over the last three */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(24) },
            VOp::LoopHead,
            push(0),
            fadd(1),
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(1)), b: Operand::V(V(1)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(3)), a: Operand::V(V(2)), b: Operand::V(V(2)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(4)), a: Operand::V(V(3)), b: Operand::V(V(3)) },
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(5)), a: Operand::V(V(4)), b: Operand::V(V(4)) },
            VOp::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None },
        ];
        let text = listing_with(&ops, &[c, r0, r1, r2, r0, r1], true);
        let br = text.iter().position(|t| t.starts_with("bu.anyna")).expect("the back edge");
        assert!(text[br - 1].starts_with("fadd r1, r0, r0"), "the second FADD stays before: {text:#?}");
        assert!(text[br + 1..br + 4].iter().all(|t| t.starts_with("fadd ")), "three FADDs in the slots: {text:#?}");
        /* an unconditional forward branch moves over the push */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(24) },
            fadd(1),
            push(0),
            VOp::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: 0 },
            VOp::Label(0),
            VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: Operand::V(V(1)), b: Operand::V(V(1)) },
        ];
        let text = listing_with(&ops, &[c, r0, r1], true);
        let br = text.iter().position(|t| t.starts_with("bu ")).expect("the branch");
        assert!(text[br + 1].contains("mov.pushz"), "the push in a slot of the unconditional branch: {text:#?}");
    }

    /// hoist's flag rule: a flag push goes back into an earlier word's free
    /// unit, but not over a conditional write that reads the push before it
    /// (it would read this one) nor over another push (the A/B shift would
    /// change). Revert check: with the flags test in hoist dropped, the
    /// push lands before the conditional move.
    #[test]
    fn a_flag_push_is_not_hoisted_over_a_flag_read_or_push() {
        let (r0, r1, r2, r3) = (Reg::Acc(0), Reg::Acc(1), Reg::Acc(2), Reg::Acc(3));
        let push = |v: u32| VOp::Flags {
            unit: FlagOp::Mul(MulOp::Mov),
            a: Operand::V(V(v)),
            b: Operand::None,
            pf: Pf::PushZ,
            uf: Uf::None,
        };
        let fadd =
            |d: u32, s: u32| VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(d)), a: Operand::V(V(s)), b: Operand::V(V(s)) };
        /* Words: `mov r0`, `fadd r2 ; mov.pushz r0`, `fadd r3` (a free mul
         * unit), `fadd r4 ; mov.ifa r1` (the read of the first push),
         * `fmul`, then the second push, which reads a payload register
         * (ready from word 0): word 2's free mul unit is before the read,
         * so it must stay after it. */
        let ops = [
            VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(0)), a: Operand::Payload(0), b: Operand::None },
            push(0),
            fadd(2, 0),
            fadd(3, 2),
            fadd(4, 3),
            VOp::CondMov { dst: V(1), src: Operand::Payload(1), cond: Cond::IfA },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(5)), a: Operand::V(V(4)), b: Operand::V(V(4)) },
            VOp::Flags {
                unit: FlagOp::Mul(MulOp::Mov),
                a: Operand::Payload(2),
                b: Operand::None,
                pf: Pf::PushZ,
                uf: Uf::None,
            },
            VOp::CondMov { dst: V(1), src: Operand::Payload(1), cond: Cond::IfA },
        ];
        let text = listing_with(&ops, &[r0, r1, r2, r3, Reg::Acc(4), r2], true);
        let first = text.iter().position(|t| t.contains("mov.ifa r1, rf1")).expect("the first conditional move");
        let second = text.iter().position(|t| t.contains("mov.pushz -, rf2")).expect("the second push");
        assert!(second > first, "the second push stays after the read of the first: {text:#?}");
        assert!(
            text[2].starts_with("fadd r3, r2, r2") && text[2].ends_with("; nop"),
            "word 2's mul unit free: {text:#?}"
        );
        /* with nothing between, it goes back to the free mul unit */
        let ops = [
            VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(0)), a: Operand::Payload(0), b: Operand::None },
            fadd(2, 0),
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(3)), a: Operand::V(V(2)), b: Operand::V(V(2)) },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(1)), a: Operand::V(V(3)), b: Operand::V(V(3)) },
            push(0),
        ];
        let text = listing_with(&ops, &[r0, r1, r2, r3], true);
        let push_at = text.iter().position(|t| t.contains("mov.pushz -, r0")).expect("the push");
        assert!(text[push_at].starts_with("fadd r2, r0, r0"), "the push in the FADD's word: {text:#?}");
    }
}
