// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 Broadcom
// Ported from Mesa 26.2.1 src/broadcom/compiler/qpu_validate.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The rules between instructions, checked on every program the chain emits.
//!
//! Most of them are Mesa's `qpu_validate` (src/broadcom/compiler/
//! qpu_validate.c, Mesa 26.2.1, Copyright (c) 2014 Broadcom, MIT licence),
//! transcribed for V3D 4.2: "the instruction restrictions from page 37
//! (Summary of Instruction Restrictions)" that Mesa checks after its own
//! scheduling. The branch, rtop and SFU rules are here although this cut
//! emits no branch, MULTOP or magic SFU write, so that a later cut that
//! emits one is checked from its first program.
//!
//! Not transcribed, because they need facts the words do not carry and
//! nothing in this cut produces them: the TLB-Z-write rules (Mesa marks an
//! instruction `is_tlb_z_write` at selection; the branch/MSF, SETMSF and MSF
//! checks, qpu_validate.c:134-191), and GFXH-1633, which is for versions
//! below 4.2 (:202-226). They are named so the gap is visible.
//!
//! One rule is ours, not Mesa's, and says so: a thread switch invalidates
//! the accumulators (vir_register_allocate.c:1340-1347 is how Mesa's
//! allocator avoids it, not a check), so this file refuses a read of r0..r5
//! that no write in the same thread section put a value in.

use super::isa::*;

/// A rule a program broke, at an instruction index. The rule is Mesa's
/// message where the rule is Mesa's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VerifyError {
    pub ip: usize,
    pub rule: &'static str,
}

fn is_sfu(waddr: u8) -> bool {
    (19..=24).contains(&waddr)
}
fn is_tmu(waddr: u8) -> bool {
    /* ver >= 40: TMUD..TMUAU and TMUC..TMUHSLOD (qpu_instr.c:634-648) */
    (11..=13).contains(&waddr) || (32..=46).contains(&waddr)
}
fn is_tlb(waddr: u8) -> bool {
    waddr == 7 || waddr == 8
}
fn is_vpm(waddr: u8) -> bool {
    waddr == 14 || waddr == 15
}
fn is_tsy(waddr: u8) -> bool {
    (16..=18).contains(&waddr)
}
/// The reserved magic waddr ranges on V3D 4.2 (qpu_validate.c:87-97).
fn is_reserved(waddr: u8) -> bool {
    waddr == 10 || (14..=15).contains(&waddr) || (25..=31).contains(&waddr) || (47..=54).contains(&waddr) || waddr >= 56
}

/// One instruction's peripheral accesses, as Mesa's scheduler counts them
/// (qpu_peripherals, qpu_schedule.c:854-893). A bit per kind.
const P_VPM_READ: u32 = 1 << 0;
const P_VPM_WRITE: u32 = 1 << 1;
const P_VPM_WAIT: u32 = 1 << 2;
const P_SFU: u32 = 1 << 3;
const P_TMU_WRITE: u32 = 1 << 4;
const P_TMU_READ: u32 = 1 << 5;
const P_TMU_WAIT: u32 = 1 << 6;
const P_WRTMUC: u32 = 1 << 7;
const P_TSY: u32 = 1 << 8;
const P_TLB_READ: u32 = 1 << 9;
const P_TLB_WRITE: u32 = 1 << 10;
/// V3D_QPU_WADDR_TMUC.
const WADDR_TMUC: u8 = 32;

fn peripherals(alu: &Alu) -> u32 {
    let mut p = 0;
    let add = alu.add.op;
    /* v3d_qpu_add_op_reads_vpm / _writes_vpm (qpu_instr.c:695-724) */
    if matches!(
        add,
        AddOp::Vpmsetup
            | AddOp::LdvpmvIn
            | AddOp::LdvpmvOut
            | AddOp::LdvpmdIn
            | AddOp::LdvpmdOut
            | AddOp::Ldvpmp
            | AddOp::LdvpmgIn
            | AddOp::LdvpmgOut
    ) {
        p |= P_VPM_READ;
    }
    if matches!(add, AddOp::Vpmsetup | AddOp::Stvpmv | AddOp::Stvpmd | AddOp::Stvpmp) {
        p |= P_VPM_WRITE;
    }
    if add == AddOp::Vpmwt {
        p |= P_VPM_WAIT;
    }
    if add == AddOp::Tmuwt {
        p |= P_TMU_WAIT;
    }
    /* v3d_qpu_uses_sfu: the 4.2 SFU opcodes and the legacy magic writes */
    if crate::shaderc::online::v3d::select::is_sfu(add) {
        p |= P_SFU;
    }
    for w in magic_writes(alu).into_iter().flatten() {
        if is_vpm(w) {
            p |= P_VPM_WRITE;
        }
        if is_sfu(w) {
            p |= P_SFU;
        }
        if is_tmu(w) {
            p |= P_TMU_WRITE;
        }
        if is_tlb(w) {
            p |= P_TLB_WRITE;
        }
        if is_tsy(w) {
            p |= P_TSY;
        }
    }
    if alu.sig.ldtmu {
        p |= P_TMU_READ;
    }
    if alu.sig.wrtmuc {
        p |= P_WRTMUC;
    }
    if alu.sig.ldtlb || alu.sig.ldtlbu {
        p |= P_TLB_READ;
    }
    p
}

/// Does one instruction make only the peripheral accesses V3D 4.2 allows
/// together? One access per instruction, except a WRTMUC signal with a TMU
/// register write other than TMUC, and a TMU read with a VPM read or write
/// (qpu_compatible_peripheral_access, qpu_schedule.c:895-930, ver 42).
///
/// Mesa's validator does not check this (it counts magic writes only,
/// qpu_validate.c:250-343): its scheduler never builds such an instruction.
/// Ours did: the first `wrtmuc` of a look-up merged into an `rsqrt`
/// (dEQP random all_features.fragment.1) or an `ldvpmv_in` (gl_test 126's
/// and struct.uniform sampler_in_array_function_arg's vertex shaders),
/// and the Pi 4's TMU then fetched from a wild address (MMU fault, L2T).
pub fn peripherals_compatible(alu: &Alu) -> bool {
    let p = peripherals(alu);
    if p.count_ones() <= 1 {
        return true;
    }
    let tmu_not_tmuc = magic_writes(alu).into_iter().flatten().all(|w| w != WADDR_TMUC);
    (p == P_WRTMUC | P_TMU_WRITE && tmu_not_tmuc) || p == P_TMU_READ | P_VPM_READ || p == P_TMU_READ | P_VPM_WRITE
}

/// Does the instruction make any peripheral access (VPM, SFU, TMU, TSY,
/// TLB, by a unit or a signal)? The same classes as
/// [`peripherals_compatible`] counts.
pub fn touches_peripheral(alu: &Alu) -> bool {
    peripherals(alu) != 0
}

/// The magic addresses an ALU instruction's add and mul units write.
fn magic_writes(alu: &Alu) -> [Option<u8>; 2] {
    [
        (alu.add.op != AddOp::Nop && alu.add.magic).then_some(alu.add.waddr),
        (alu.mul.op != MulOp::Nop && alu.mul.magic).then_some(alu.mul.waddr),
    ]
}

/// Ours: the registers an instruction's add, mul and signal write one
/// instruction later, as (magic, address): the accumulators (magic 0..5) and
/// the register file. A magic peripheral address, the NOP address, an op with
/// no destination and an SFU op (its result lands two later) are left out.
fn same_cycle_dests(alu: &Alu) -> [Option<(bool, u8)>; 3] {
    let reg = |magic: bool, addr: u8| (!magic || addr < 6).then_some((magic, addr));
    let sfu = matches!(alu.add.op, AddOp::Recip | AddOp::Rsqrt | AddOp::Exp | AddOp::Log | AddOp::Sin | AddOp::Rsqrt2);
    [
        (alu.add.op != AddOp::Nop && add_op_args(alu.add.op) & ARG_D != 0 && !sfu)
            .then(|| reg(alu.add.magic, alu.add.waddr))
            .flatten(),
        (alu.mul.op != MulOp::Nop && mul_op_args(alu.mul.op) & ARG_D != 0)
            .then(|| reg(alu.mul.magic, alu.mul.waddr))
            .flatten(),
        alu.sig.writes_address().then(|| reg(alu.sig_magic, alu.sig_addr)).flatten(),
    ]
}

/// `v3d_qpu_uses_mux` (qpu_instr.c:1043-1052).
fn uses_mux(alu: &Alu, m: Mux) -> bool {
    let add_n = num_src(add_op_args(alu.add.op));
    let mul_n = num_src(mul_op_args(alu.mul.op));
    (add_n > 0 && alu.add.a.mux == m)
        || (add_n > 1 && alu.add.b.mux == m)
        || (mul_n > 0 && alu.mul.a.mux == m)
        || (mul_n > 1 && alu.mul.b.mux == m)
}

/// `v3d_qpu_writes_r4` (qpu_instr.c:962-992).
fn writes_r4(alu: &Alu) -> bool {
    if magic_writes(alu).into_iter().flatten().any(|w| w == 4 || is_sfu(w)) {
        return true;
    }
    if alu.sig.writes_address() {
        alu.sig_magic && alu.sig_addr == 4
    } else {
        alu.sig.ldtmu
    }
}

/// Ours: the accumulators an instruction writes, and how many instructions
/// later each write lands (r5 from `ldvary` two later, from `ldunif` one;
/// qpu_validate.c:193-200).
fn acc_writes(alu: &Alu) -> [Option<isize>; 6] {
    let mut out = [None; 6];
    for w in magic_writes(alu).into_iter().flatten() {
        if w < 6 {
            out[w as usize] = Some(1);
        }
    }
    if alu.sig.writes_address() && alu.sig_magic && alu.sig_addr < 6 {
        out[alu.sig_addr as usize] = Some(1);
    }
    if writes_r4(alu) && out[4].is_none() {
        out[4] = Some(1);
    }
    if alu.sig.ldvary {
        out[5] = Some(2);
    }
    if alu.sig.ldunif || alu.sig.ldunifa {
        out[5] = Some(1);
    }
    out
}

/// Check `prog`. `single_seg`: the program starts in its last thread
/// section (a vertex or coordinate shader on 4.x, which has no last-thread-
/// switch pair; Mesa starts with `last_thrsw_found = !c->last_thrsw`,
/// qpu_validate.c:461-468).
pub fn verify(prog: &[Instr], single_seg: bool) -> Result<(), VerifyError> {
    let mut last: Option<&Instr> = None;
    let mut last_sfu_write: isize = -10;
    let mut last_branch_ip: isize = -10;
    let mut last_thrsw_ip: isize = -10;
    let mut last_thrsw_found = single_seg;
    let mut thrend_found = false;
    let mut thrsw_count = 0;
    let mut rtop_hazard = false;
    let mut rtop_valid = false;
    /* Ours: which accumulators hold a value of this thread section, the
     * writes still in flight, and where the pending switch lands. */
    let mut acc_valid = [false; 6];
    let mut pending: [Option<isize>; 6] = [None; 6];
    let mut switch_at: Option<isize> = None;

    for (i, instr) in prog.iter().enumerate() {
        let ip = i as isize;
        let fail = |rule| Err(VerifyError { ip: i, rule });
        let in_branch_delay = ip - last_branch_ip < 4;
        let in_thrsw_delay = ip - last_thrsw_ip < 3;

        /* Ours: the switch happens after a THRSW's two delay slots and
         * clears the accumulators; writes land after their latency. */
        if switch_at == Some(ip) {
            acc_valid = [false; 6];
            pending = [None; 6];
            switch_at = None;
        }
        for (v, p) in acc_valid.iter_mut().zip(pending.iter_mut()) {
            if *p == Some(ip) {
                *v = true;
                *p = None;
            }
        }

        let alu = match instr {
            Instr::Branch(_) => {
                if in_branch_delay {
                    return fail("branch in a branch delay slot.");
                }
                if in_thrsw_delay {
                    return fail("branch in a THRSW delay slot.");
                }
                last_branch_ip = ip;
                last = Some(instr);
                continue;
            }
            Instr::Alu(alu) => alu,
        };

        if alu.mul.op == MulOp::Multop && !in_branch_delay {
            rtop_valid = true;
        }
        if alu.mul.op == MulOp::Umul24 {
            if rtop_hazard {
                return fail("UMUL24 reads rtop from MULTOP but it got cleared by a previous THRSW");
            }
            rtop_valid = false;
            rtop_hazard = false;
        }

        if let Some(Instr::Alu(prev)) = last {
            if prev.sig.ldvary && (alu.sig.ldunif || alu.sig.ldunifa) {
                return fail("LDUNIF after a LDVARY");
            }
        }

        let (mut tmu, mut sfu, mut vpm, mut tlb, mut tsy) = (0u32, 0u32, 0u32, 0u32, 0u32);
        for w in magic_writes(alu).into_iter().flatten() {
            tmu += is_tmu(w) as u32;
            sfu += is_sfu(w) as u32;
            vpm += is_vpm(w) as u32;
            tlb += is_tlb(w) as u32;
            tsy += is_tsy(w) as u32;
        }

        if in_thrsw_delay {
            if sfu > 0 {
                return fail("SFU write started during THRSW delay slots ");
            }
            if alu.sig.ldvary {
                return fail("LDVARY during THRSW delay slots");
            }
        }

        if magic_writes(alu).into_iter().flatten().any(is_reserved) {
            return fail("write to a reserved magic waddr");
        }

        /* Ours: two writes that land on one register in one cycle (the
         * add and the mul, or either and the signal). Which one wins is not
         * described by Mesa, so it is a defect in whoever emitted it. */
        let d = same_cycle_dests(alu);
        for x in 0..3 {
            for y in x + 1..3 {
                if d[x].is_some() && d[x] == d[y] {
                    return fail("two writes to one register in one instruction (ours)");
                }
            }
        }

        if ip - last_sfu_write < 2 {
            if uses_mux(alu, Mux::R4) {
                return fail("R4 read too soon after SFU");
            }
            if writes_r4(alu) {
                return fail("R4 write too soon after SFU");
            }
            if sfu > 0 {
                return fail("SFU write too soon after SFU");
            }
        }

        let loads = alu.sig.ldtmu as u32 + alu.sig.ldtlb as u32 + alu.sig.ldtlbu as u32;
        if tmu + sfu + vpm + tlb + tsy + loads > 1 {
            return fail("Only one of [TMU, SFU, TSY, TLB read, VPM] allowed");
        }
        /* Ours: Mesa's scheduler rule, which its validator leaves out */
        if !peripherals_compatible(alu) {
            return fail("more than one peripheral access (qpu_compatible_peripheral_access, ours)");
        }
        if sfu > 0 {
            last_sfu_write = ip;
        }

        if alu.sig.thrsw {
            if in_branch_delay {
                return fail("THRSW in a branch delay slot.");
            }
            if last_thrsw_found {
                thrend_found = true;
            }
            if last_thrsw_ip == ip - 1 {
                if last_thrsw_found {
                    return fail("Two last-THRSW signals");
                }
                last_thrsw_found = true;
            } else {
                if in_thrsw_delay {
                    return fail("THRSW too close to another THRSW.");
                }
                thrsw_count += 1;
                last_thrsw_ip = ip;
                switch_at = Some(ip + 3);
            }
        }

        if thrend_found && ip - last_thrsw_ip <= 2 {
            if (alu.add.op != AddOp::Nop && !alu.add.magic) || (alu.mul.op != MulOp::Nop && !alu.mul.magic) {
                return fail("RF write after THREND");
            }
            if alu.sig.writes_address() && !alu.sig_magic {
                return fail("RF write after THREND");
            }
            if ip - last_thrsw_ip == 2 && alu.add.op == AddOp::Tmuwt {
                return fail("TMUWT in last instruction");
            }
        }

        if rtop_valid && ip == last_thrsw_ip + 2 {
            rtop_hazard = true;
            rtop_valid = false;
        }

        /* Ours: an accumulator read must see a value of this section. */
        for (r, valid) in acc_valid.iter().enumerate() {
            if !valid && uses_mux(alu, Mux::from_bits(r as u32)) {
                return fail("accumulator read with no value written in this thread section (ours)");
            }
        }
        /* The earliest landing wins: a write only ever makes a register
         * valid, and a later write to it must not cancel an earlier one
         * still in flight (two ldvary in a row: r5 lands at ip+2 for each). */
        for (p, d) in pending.iter_mut().zip(acc_writes(alu)) {
            if let Some(d) = d {
                let at = ip + d;
                *p = Some(p.map_or(at, |q| q.min(at)));
            }
        }

        last = Some(instr);
    }

    let end = prog.len().saturating_sub(1);
    if thrsw_count > 1 && !last_thrsw_found {
        return Err(VerifyError { ip: end, rule: "thread switch found without last-THRSW in program" });
    }
    if !thrend_found {
        return Err(VerifyError { ip: end, rule: "No program-end THRSW found" });
    }
    Ok(())
}

/// Ours: the register-file registers `prog` reads or writes all exist at
/// `threads` threads. A thread has rf0..rf31 at 4 threads and rf0..rf63 at 1
/// or 2 on V3D 4.2 (vir_register_allocate.c:1428-1435); an address past its
/// share is another thread's register. Mesa's validator has no such rule, and
/// the interpreter gives every thread 64 registers, so without this a 4-thread
/// program that used rf32 would agree on the host.
pub fn registers_fit_threads(prog: &[Instr], threads: u32) -> Result<(), VerifyError> {
    const RULE: &str = "a register-file register past what a thread has at this thread count";
    let end = match threads {
        4 => 32,
        1 | 2 => 64,
        _ => return Err(VerifyError { ip: 0, rule: "a thread count the V3D does not run (not 1, 2 or 4)" }),
    };
    for (ip, instr) in prog.iter().enumerate() {
        let mut regs: [Option<u8>; 5] = [None; 5];
        match instr {
            Instr::Alu(alu) => {
                if uses_mux(alu, Mux::A) {
                    regs[0] = Some(alu.raddr_a);
                }
                if uses_mux(alu, Mux::B) && !alu.sig.small_imm_b {
                    regs[1] = Some(alu.raddr_b);
                }
                if alu.add.op != AddOp::Nop && add_op_args(alu.add.op) & ARG_D != 0 && !alu.add.magic {
                    regs[2] = Some(alu.add.waddr);
                }
                if alu.mul.op != MulOp::Nop && mul_op_args(alu.mul.op) & ARG_D != 0 && !alu.mul.magic {
                    regs[3] = Some(alu.mul.waddr);
                }
                if alu.sig.writes_address() && !alu.sig_magic {
                    regs[4] = Some(alu.sig_addr);
                }
            }
            Instr::Branch(br) => {
                if br.reads_raddr_a() {
                    regs[0] = Some(br.raddr_a);
                }
            }
        }
        if regs.into_iter().flatten().any(|r| r >= end) {
            return Err(VerifyError { ip, rule: RULE });
        }
    }
    Ok(())
}

/// [`branches_are_structured`] for a program with no forward branches.
pub fn branches_are_counted_loops(prog: &[Instr], back_edges: &[(u32, u32)]) -> Result<(), VerifyError> {
    branches_are_structured(prog, back_edges, &[])
}

/// Ours: the compiler never emits a program that does not terminate
/// (CLAUDE.md). This back end emits two kinds of branch, and every branch
/// in `prog` must be one of them:
/// - a counted loop's back edge, one of `back_edges` (branch, target)
///   (sched.rs, select.rs VOp::BackEdge, whose condition includes the
///   loop's pass counter): conditional, backward to its loop's head;
/// - a forward branch, one of `fwd` (branch, target) (select.rs
///   VOp::FwdBranch: a uniform branch's, or a loop's uniform break):
///   strictly forward, past its own three delay slots, to an instruction
///   inside the program, never into a loop body (a loop is entered only at
///   its head, where its counter was set), and out of one only to its exit,
///   the word after its back edge's delay slots (a break leaves the loop it
///   is in, and only that one: with loops nested, the target must be the
///   exit of every loop the branch leaves, so it leaves one).
///
/// Both are REL with the uniform stream moved by the same rule, and every
/// recorded branch must be a branch. Forward branches cannot make a cycle,
/// so the program still ends: each loop by its counter, the rest by moving
/// forward.
///
/// And the last-thread-switch pair is at the top level: in no loop body
/// and in no forward branch's range, so every path runs it once.
pub fn branches_are_structured(
    prog: &[Instr],
    back_edges: &[(u32, u32)],
    fwd: &[(u32, u32)],
) -> Result<(), VerifyError> {
    for (ip, instr) in prog.iter().enumerate() {
        let Instr::Branch(br) = instr else { continue };
        if let Some(&(_, t)) = fwd.iter().find(|e| e.0 as usize == ip) {
            if br.bdi != BranchDest::Rel || !br.ub || br.bdu != BranchDest::Rel {
                return Err(VerifyError { ip, rule: "a forward branch that is not REL with the uniform stream moved" });
            }
            let target = ip as i64 + 4 + (br.offset as i32 as i64) / 8;
            if target != t as i64 {
                return Err(VerifyError { ip, rule: "a forward branch whose target is not its label" });
            }
            if target < ip as i64 + 4 || target as usize >= prog.len() {
                return Err(VerifyError { ip, rule: "a forward branch that is not forward inside the program" });
            }
            for &(b, h) in back_edges {
                let inside = |x: usize| x >= h as usize && x <= b as usize;
                match (inside(ip), inside(target as usize)) {
                    (false, true) => return Err(VerifyError { ip, rule: "a forward branch into a loop body" }),
                    /* a break: to the word after the back edge's delay slots */
                    (true, false) if target != b as i64 + 4 => {
                        return Err(VerifyError { ip, rule: "a forward branch out of a loop body, not to its exit" })
                    }
                    _ => {}
                }
            }
            continue;
        }
        let Some(&(_, head)) = back_edges.iter().find(|e| e.0 as usize == ip) else {
            return Err(VerifyError { ip, rule: "a branch that is neither a loop's back edge nor a forward branch" });
        };
        if br.cond == BranchCond::Always {
            return Err(VerifyError { ip, rule: "an unconditional back edge" });
        }
        if br.bdi != BranchDest::Rel || !br.ub || br.bdu != BranchDest::Rel {
            return Err(VerifyError { ip, rule: "a back edge that is not REL with the uniform stream moved" });
        }
        let target = ip as i64 + 4 + (br.offset as i32 as i64) / 8;
        if target != head as i64 {
            return Err(VerifyError { ip, rule: "a back edge whose target is not its loop's head" });
        }
        if target < 0 || target as usize >= ip {
            return Err(VerifyError { ip, rule: "a back edge that is not backward inside the program" });
        }
    }
    for &(b, _) in back_edges {
        if !matches!(prog.get(b as usize), Some(Instr::Branch(_))) {
            return Err(VerifyError { ip: b as usize, rule: "a recorded back edge that is not a branch" });
        }
    }
    for &(b, _) in fwd {
        if !matches!(prog.get(b as usize), Some(Instr::Branch(_))) {
            return Err(VerifyError { ip: b as usize, rule: "a recorded forward branch that is not a branch" });
        }
    }
    /* Loops nest or follow each other, never cross: each one's counter is
     * loaded before its head, so a loop entered from outside its body (a
     * crossing range) could run on a counter nobody set. */
    for (k, &(b1, h1)) in back_edges.iter().enumerate() {
        for &(b2, h2) in &back_edges[k + 1..] {
            let (a, b) = if h1 <= h2 { ((h1, b1), (h2, b2)) } else { ((h2, b2), (h1, b1)) };
            let nested = b.0 >= a.0 && b.1 <= a.1;
            let disjoint = b.0 > a.1;
            if !nested && !disjoint {
                return Err(VerifyError { ip: b.1 as usize, rule: "two loops that cross" });
            }
        }
    }
    /* The last-thread-switch pair (two THRSW in a row) runs once on every
     * path: not in a loop body, which runs it again, and not between a
     * forward branch's delay slots and its target, which a path skips
     * (Mesa emits it only at the top level, last_thrsw_at_top_level,
     * nir_to_vir.c:5007; select.rs Sel::last_thrsw). */
    let thrsw = |i: &Instr| matches!(i, Instr::Alu(a) if a.sig.thrsw);
    if let Some(p) = prog.windows(2).position(|w| thrsw(&w[0]) && thrsw(&w[1])) {
        if back_edges.iter().any(|&(b, h)| p >= h as usize && p <= b as usize) {
            return Err(VerifyError { ip: p, rule: "the last-thread-switch pair in a loop body" });
        }
        if fwd.iter().any(|&(b, t)| p >= b as usize + 4 && p < t as usize) {
            return Err(VerifyError { ip: p, rule: "the last-thread-switch pair where a forward branch skips it" });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// `n` NOP words with a REL branch, the uniform stream moved too, at
    /// each `(at, target)`: the offset counts from the word after the three
    /// delay slots, as sched.rs sets it.
    fn prog(n: usize, branches: &[(usize, i64)]) -> Vec<Instr> {
        let mut p: Vec<Instr> = (0..n).map(|_| Instr::Alu(Alu::nop())).collect();
        for &(at, target) in branches {
            p[at] = Instr::Branch(Branch {
                cond: BranchCond::AnyA,
                msfign: Msfign::P,
                bdi: BranchDest::Rel,
                ub: true,
                bdu: BranchDest::Rel,
                raddr_a: 0,
                offset: ((target - (at as i64 + 4)) * 8) as u32,
            });
        }
        p
    }

    fn rule(p: &[Instr], edges: &[(u32, u32)], fwd: &[(u32, u32)]) -> Result<(), &'static str> {
        branches_are_structured(p, edges, fwd).map_err(|e| e.rule)
    }

    /// CLAUDE.md's guarantee: a backward branch target not produced by a
    /// loop node is refused. A loop's back edge, recorded, passes (the known
    /// positive); the same branch not recorded, or recorded as a forward
    /// branch (a break), is refused. Revert check: with the forward rule's
    /// `target < ip + 4` switched off, the forward case passes and this
    /// fails; with a branch that is no recorded kind let through, the
    /// unrecorded case passes and this fails.
    #[test]
    fn a_backward_target_not_made_by_a_loop_node_is_refused() {
        let p = prog(40, &[(20, 10)]);
        assert_eq!(rule(&p, &[(20, 10)], &[]), Ok(()));
        assert_eq!(rule(&p, &[], &[]), Err("a branch that is neither a loop's back edge nor a forward branch"));
        assert_eq!(rule(&p, &[], &[(20, 10)]), Err("a forward branch that is not forward inside the program"));
        /* a break's branch aimed back at its own loop's head */
        let p = prog(40, &[(12, 10), (20, 10)]);
        assert_eq!(rule(&p, &[(20, 10)], &[(12, 10)]), Err("a forward branch that is not forward inside the program"));
    }

    /// A forward branch leaves a loop body only for that loop's exit, the
    /// word after its back edge's delay slots (a uniform `break`,
    /// prepare.rs SOp::LoopBreak), and enters none. With two loops nested
    /// (outer 5..30, inner 10..20), a break of the inner loop to the outer
    /// one's exit leaves the inner loop to somewhere else and is refused;
    /// each loop's own break passes. Revert check: with the out-of-a-body
    /// arm switched off, the break one word past its exit passes and this
    /// fails.
    #[test]
    fn a_target_outside_the_enclosing_loop_is_refused() {
        let one = |brk: (usize, i64)| prog(40, &[brk, (20, 10)]);
        assert_eq!(rule(&one((12, 24)), &[(20, 10)], &[(12, 24)]), Ok(()), "a break to its exit");
        assert_eq!(
            rule(&one((12, 25)), &[(20, 10)], &[(12, 25)]),
            Err("a forward branch out of a loop body, not to its exit")
        );
        assert_eq!(rule(&one((2, 12)), &[(20, 10)], &[(2, 12)]), Err("a forward branch into a loop body"));
        let nest = |brk: (usize, i64)| prog(40, &[brk, (20, 10), (30, 5)]);
        let edges = [(20, 10), (30, 5)];
        assert_eq!(rule(&nest((12, 24)), &edges, &[(12, 24)]), Ok(()), "the inner loop's break");
        assert_eq!(rule(&nest((6, 34)), &edges, &[(6, 34)]), Ok(()), "the outer loop's break");
        assert_eq!(
            rule(&nest((12, 34)), &edges, &[(12, 34)]),
            Err("a forward branch out of a loop body, not to its exit"),
            "the inner loop's break to the outer loop's exit"
        );
    }

    /// A target past the program's end is refused, a loop's exit included:
    /// a loop whose back edge's delay slots are the program's last words has
    /// no word to break to. Revert check: with `target >= prog.len()`
    /// switched off, both refused cases pass and this fails.
    #[test]
    fn a_target_past_the_programs_end_is_refused() {
        let p = prog(36, &[(30, 35)]);
        assert_eq!(rule(&p, &[], &[(30, 35)]), Ok(()), "the last word");
        let p = prog(36, &[(30, 36)]);
        assert_eq!(rule(&p, &[], &[(30, 36)]), Err("a forward branch that is not forward inside the program"));
        let p = prog(34, &[(12, 34), (30, 10)]);
        assert_eq!(
            rule(&p, &[(30, 10)], &[(12, 34)]),
            Err("a forward branch that is not forward inside the program"),
            "a break to an exit past the end"
        );
    }
}
