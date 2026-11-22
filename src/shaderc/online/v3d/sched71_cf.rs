// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/compiler/qpu_schedule.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Scheduling at V3D 7.1, the whole program: the straight-line runs packed
//! by [`super::sched71`]'s [`Packer71`] in order, the control flow between
//! them placed here, as the contract in [`super::vop71`]'s header and
//! `docs/notes/v3d71-backend.md` (e) rows 20-21 say. A program with no
//! control flow gives exactly [`super::sched71::pack71`]'s words: the same
//! packer, the same calls, nothing between.
//!
//! The 4.2 scheduler's control flow (sched.rs `schedule`, from `LoopHead` to
//! the branch resolution) is the model, construct for construct:
//!
//! * A [`Op71::LoopHead`] or [`Op71::Label`] is a word boundary
//!   ([`Packer71::boundary`]): nothing before it merges into the word it
//!   names, nothing after it goes before it, and no THRSW moves back across
//!   it. Its index is the target of the branches to it.
//! * A [`Op71::BackEdge`] or [`Op71::FwdBranch`] is a branch word after
//!   every word before it, after the flag push a conditional one reads
//!   (Mesa's `emit_branch` reads the flags as the branch issues,
//!   qpu_schedule.c:2479-2485), and outside a THRSW's delay slots
//!   (qpu_validate.c:157-164). It is REL with `ub` (the uniform stream
//!   branches too, `bdu` REL), its offset counted from the word after its
//!   three delay slots in bytes, and its own stream word the byte offset of
//!   the target's next uniform from the one after the branch's
//!   (`qpu_set_branch_targets`, qpu_schedule.c:2916-2983, as sched.rs).
//! * Its delay slots: Mesa's `emit_branch` (qpu_schedule.c:2445-2502) moves
//!   a branch back over up to three of the words before it, which then run
//!   in its slots instead of NOPs; they ran before the branch and still do,
//!   on both of its paths. [`branch_back`] does that with sched.rs's narrow
//!   rule ([`ok_in_branch_slot71`]), and the rest of the three slots are
//!   NOPs ([`Packer71::insert_branch`]). At 7.1 a branch with `msfign` may
//!   follow a SETMSF whatever its condition (`is_safe_msf_branch`,
//!   qpu_schedule.c:2430-2438, is `ver < 71` only; design note row 20), so
//!   sched.rs's `msf_safe` has no 7.1 counterpart.
//! * The counted loops nest; a back edge closes the innermost open head.
//!   Labels are forward only. Anything else (a back edge with no head, a
//!   head left open, a label defined twice or never, a branch to a label
//!   before its slots) is [`Sched71Error::ControlFlow`] naming the
//!   operation, never a program.
//! * The program ends with [`Packer71::thread_end`], after every branch's
//!   slots and the last boundary, as sched.rs's thrend does.
//!
//! The result passes [`branches_are_structured71`] by construction; chain71
//! runs it after verify71 once the chain71 item wires it in.

use super::alloc71::Allocation71;
use super::isa71::{AddOp71, Branch, BranchCond, BranchDest, Msfign, MulOp71, Sig71, WADDR_NOP};
use super::sched71::{Packer71, Sched71Error, Word71};
use super::verify71::{is_sfu_op, VerifyError};
use super::vop71::{Instr71, Op71, Selected71, Uniform};
use crate::shaderc::online::arena::{AVec, Arena};

/// The scheduled 7.1 program.
#[derive(Clone, Copy, Debug)]
pub struct Scheduled71<'a> {
    pub instrs: &'a [Instr71],
    /// The uniform stream, in the order the program consumes it.
    pub uniforms: &'a [Uniform],
    /// Every counted loop's back edge, as (branch instruction, target
    /// instruction).
    pub back_edges: &'a [(u32, u32)],
    /// Every forward branch, as (branch instruction, target instruction).
    pub fwd_branches: &'a [(u32, u32)],
}

/// Whether word `w` may run in a branch's delay slots, moved there from
/// before the branch: sched.rs `ok_in_branch_slot` at 7.1. Mesa's
/// `qpu_inst_valid_in_branch_delay_slot` (qpu_schedule.c:2396-2412): no
/// branch, no thread switch, no UNIFA write, no uniform (the branch moves
/// the stream: a uniform read after it would read the target's); for a
/// conditional branch, no flag push either (`emit_branch`, :2479-2485).
/// This cut is narrower than Mesa's, so that none of the packer's
/// bookkeeping (the look-up, varying, flag and switch positions, the SFU
/// latency, rf0) sees a word move: no signal at all, no TLB or TMU access,
/// no SFU operation (an add operation at 7.1, read two words later), no VPM
/// access, TMUWT or BARRIERID, no magic write but NOP, no MULTOP or UMUL24
/// (verify71: a MULTOP in a branch delay slot does not set rtop).
fn ok_in_branch_slot71(w: &Word71, conditional: bool) -> bool {
    let a = &w.alu;
    let signal = Sig71 { small_imm_a: false, small_imm_b: false, small_imm_c: false, small_imm_d: false, ..a.sig }
        != Sig71::default();
    let other_magic = |used: bool, magic: bool, waddr: u8| used && magic && waddr != WADDR_NOP;
    let vpm_or_wait = matches!(
        a.add.op,
        AddOp71::Vpmsetup
            | AddOp71::Vpmwt
            | AddOp71::LdvpmvIn
            | AddOp71::LdvpmvOut
            | AddOp71::LdvpmdIn
            | AddOp71::LdvpmdOut
            | AddOp71::Ldvpmp
            | AddOp71::LdvpmgIn
            | AddOp71::LdvpmgOut
            | AddOp71::Stvpmv
            | AddOp71::Stvpmd
            | AddOp71::Stvpmp
            | AddOp71::Tmuwt
            | AddOp71::Barrierid
    );
    w.branch.is_none()
        && w.uniform.is_none()
        && !signal
        && !w.tlb
        && !w.tmu_write
        && (!conditional || !w.flags_push)
        && !is_sfu_op(a.add.op)
        && !vpm_or_wait
        && !other_magic(a.add.op != AddOp71::Nop, a.add.magic, a.add.waddr)
        && !other_magic(a.mul.op != MulOp71::Nop, a.mul.magic, a.mul.waddr)
        && !matches!(a.mul.op, MulOp71::Multop | MulOp71::Umul24)
}

/// Mesa's `emit_branch` (qpu_schedule.c:2445-2502), sched.rs `branch_back`:
/// a branch that would go right after the last word placed (`at ==
/// p.len()`) moves back over up to three of the words before it, which
/// then run in its delay slots instead of NOPs. It stops at a word that may
/// not be in a slot ([`ok_in_branch_slot71`]) and never lands before `lo`,
/// which the caller sets past a THRSW's delay slots and another branch's
/// (:2453-2469), past the word a loop head or a label names (the targets
/// stay where they are), and two words past an `ldvary`, whose C term is
/// still to land in rf0 (:2471-2477). Returns the branch's word.
fn branch_back(p: &Packer71<'_, '_>, at: usize, lo: usize, conditional: bool) -> usize {
    if at != p.len() {
        return at;
    }
    let mut q = at;
    while q > lo && at - q < 3 && p.word(q - 1).is_some_and(|w| ok_in_branch_slot71(w, conditional)) {
        q -= 1;
    }
    q
}

/// The packer and what the branch placement keeps beside it: the last loop
/// head or label word (a branch never moves back over a target) and the
/// last branch word (nor into another branch's slots).
struct Cf<'a, 'r> {
    p: Packer71<'a, 'r>,
    last_target: Option<usize>,
    last_branch: Option<usize>,
}

impl<'a, 'r> Cf<'a, 'r> {
    /// A branch word (REL, the uniform stream moved; offset and stream word
    /// resolved by [`Cf::resolve`]) placed for operation `i`: after every
    /// word before it, after the flag push it reads (a forward conditional
    /// branch with none is [`Sched71Error::CondWithoutFlags`]; a back
    /// edge's counter is always pushed in its body, so as sched.rs it only
    /// waits for the last push), outside the last THRSW's delay slots, then
    /// moved back by [`branch_back`] over the words that can be its delay
    /// slots. Returns the branch's word.
    fn branch(
        &mut self,
        i: usize,
        cond: BranchCond,
        msfign: Msfign,
        forward: bool,
        a: &Arena<'a>,
    ) -> Result<usize, Sched71Error> {
        let p = &mut self.p;
        let conditional = cond != BranchCond::Always;
        let mut at = p.len().max(p.floor());
        match (p.last_flags(), conditional, forward) {
            (Some(f), true, _) | (Some(f), false, false) => at = at.max(f + 1),
            (None, true, true) => return Err(Sched71Error::CondWithoutFlags(i)),
            _ => {}
        }
        if let Some(t) = p.last_thrsw() {
            at = at.max(t + 3);
        }
        p.pad_to(at, a)?;
        let lo = [
            self.last_branch.map(|b| b + 4),
            self.last_target.map(|t| t + 1),
            p.last_thrsw().map(|t| t + 3),
            p.last_ldvary().map(|l| l + 3),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);
        let q = branch_back(p, at, lo, conditional);
        let br = Branch { cond, msfign, bdi: BranchDest::Rel, ub: true, bdu: BranchDest::Rel, raddr_a: 0, offset: 0 };
        p.insert_branch(q, br, a)?;
        self.last_branch = Some(q);
        Ok(q)
    }

    /// The branch at `b` resolved to target `t`: the REL offset from the
    /// word after its delay slots, and the uniform stream moved to the
    /// target's first uniform (`qpu_set_branch_targets`,
    /// qpu_schedule.c:2916-2983: the branch's own stream word is the byte
    /// offset from the one after it).
    fn resolve(&mut self, b: usize, t: usize, mut br: Branch) -> Result<(), Sched71Error> {
        let (bu, tu) = (self.p.uniforms_before(b) as i64, self.p.uniforms_before(t) as i64);
        br.offset = ((t as i64 - (b as i64 + 4)) * 8) as u32;
        self.p.set_branch(b, br, Uniform::constant(((tu - (bu + 1)) * 4) as u32))
    }
}

/// Schedule `sel` with the registers of `al`: the whole program, control
/// flow included (the module header).
pub fn schedule71<'a>(
    sel: &Selected71<'_>,
    al: &Allocation71<'_>,
    a: &Arena<'a>,
) -> Result<Scheduled71<'a>, Sched71Error> {
    let ops = sel.ops;
    let mut cf = Cf { p: Packer71::new(al, a)?, last_target: None, last_branch: None };
    /* each label's word (usize::MAX until placed) */
    let n_labels = ops.iter().filter_map(|o| if let Op71::Label(l) = o { Some(l.0 as usize + 1) } else { None }).max();
    let label_at = a.alloc_filled(n_labels.unwrap_or(0), usize::MAX)?;
    /* the open loops' head words and head operations, innermost last */
    let mut heads: AVec<'a, (usize, usize)> = AVec::new();
    /* every back edge and forward branch: its word, its target (the head
     * word, the label), its operation and its word's contents */
    let mut back: AVec<'a, (usize, usize, usize, Branch)> = AVec::new();
    let mut fwd: AVec<'a, (usize, usize, usize, Branch)> = AVec::new();
    for (i, op) in ops.iter().enumerate() {
        match *op {
            Op71::LoopHead => {
                let at = cf.p.boundary(a)?;
                heads.push(a, (at, i))?;
                cf.last_target = Some(at);
            }
            Op71::Label(l) => {
                let at = cf.p.boundary(a)?;
                let slot = label_at.get_mut(l.0 as usize).ok_or(Sched71Error::ControlFlow(i))?;
                if *slot != usize::MAX {
                    return Err(Sched71Error::ControlFlow(i));
                }
                *slot = at;
                cf.last_target = Some(at);
            }
            Op71::BackEdge { cond, msfign } => {
                let (head, _) = heads.pop().ok_or(Sched71Error::ControlFlow(i))?;
                let at = cf.branch(i, cond, msfign, false, a)?;
                let br = cf.p.word(at).and_then(|w| w.branch).ok_or(Sched71Error::NotPlaceable(i))?;
                back.push(a, (at, head, i, br))?;
            }
            Op71::FwdBranch { cond, msfign, label } => {
                if label.0 as usize >= label_at.len() {
                    return Err(Sched71Error::ControlFlow(i));
                }
                let at = cf.branch(i, cond, msfign, true, a)?;
                let br = cf.p.word(at).and_then(|w| w.branch).ok_or(Sched71Error::NotPlaceable(i))?;
                fwd.push(a, (at, label.0 as usize, i, br))?;
            }
            _ => cf.p.place(i, op, ops.get(i + 1), a)?,
        }
    }
    if let Some(&(_, i)) = heads.as_slice().last() {
        return Err(Sched71Error::ControlFlow(i));
    }
    cf.p.thread_end(a)?;

    let mut back_edges: AVec<'a, (u32, u32)> = AVec::with_capacity(a, back.len())?;
    for &(b, h, i, br) in back.as_slice() {
        if h >= b {
            return Err(Sched71Error::ControlFlow(i));
        }
        cf.resolve(b, h, br)?;
        back_edges.push(a, (b as u32, h as u32))?;
    }
    let mut fwd_branches: AVec<'a, (u32, u32)> = AVec::with_capacity(a, fwd.len())?;
    for &(b, l, i, br) in fwd.as_slice() {
        let t = label_at[l];
        if t == usize::MAX || t < b + 4 {
            return Err(Sched71Error::ControlFlow(i));
        }
        cf.resolve(b, t, br)?;
        fwd_branches.push(a, (b as u32, t as u32))?;
    }
    let packed = cf.p.finish(a)?;
    Ok(Scheduled71 {
        instrs: packed.instrs,
        uniforms: packed.uniforms,
        back_edges: back_edges.into_slice(),
        fwd_branches: fwd_branches.into_slice(),
    })
}

/// Ours, the 4.2 [`super::verify::branches_are_structured`] rule for rule
/// over [`Instr71`], with the same rule strings: every branch is a counted
/// loop's back edge (one of `back_edges`, as (branch, target): conditional,
/// REL with the uniform stream moved, backward to its head) or a forward
/// branch (one of `fwd`: REL with the uniform stream moved, to its label,
/// strictly forward past its three delay slots and inside the program,
/// never into a loop body, out of one only to its exit, `b + 4`). Every
/// recorded edge is a branch, loops nest or follow each other, and the
/// last-thread-switch pair is at the top level.
///
/// Not a conversion to the 4.2 `Instr`: the branch word is the shared isa
/// [`Branch`](super::isa71::Branch) and the 7.1 ALU's signal field is read
/// directly.
pub fn branches_are_structured71(
    prog: &[Instr71],
    back_edges: &[(u32, u32)],
    fwd: &[(u32, u32)],
) -> Result<(), VerifyError> {
    for (ip, instr) in prog.iter().enumerate() {
        let Instr71::Branch(br) = instr else { continue };
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
        if !matches!(prog.get(b as usize), Some(Instr71::Branch(_))) {
            return Err(VerifyError { ip: b as usize, rule: "a recorded back edge that is not a branch" });
        }
    }
    for &(b, _) in fwd {
        if !matches!(prog.get(b as usize), Some(Instr71::Branch(_))) {
            return Err(VerifyError { ip: b as usize, rule: "a recorded forward branch that is not a branch" });
        }
    }
    /* Loops nest or follow each other, never cross (verify.rs). */
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
     * path: not in a loop body and not where a forward branch skips it. */
    let thrsw = |i: &Instr71| matches!(i, Instr71::Alu(a) if a.sig.thrsw);
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
mod branch_tests {
    use super::*;
    use crate::shaderc::online::v3d::isa71::{Alu71, Branch, Msfign};
    use std::vec::Vec;

    /// `n` NOP words with a branch at each `(at, target)`: conditional, REL,
    /// the uniform stream moved, offset counted from the word after the
    /// three delay slots.
    fn prog(n: usize, branches: &[(usize, i64)]) -> Vec<Instr71> {
        let mut p: Vec<Instr71> = (0..n).map(|_| Instr71::Alu(Alu71::nop())).collect();
        for &(at, target) in branches {
            p[at] = Instr71::Branch(Branch {
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
    fn edit(p: &mut [Instr71], at: usize, f: impl FnOnce(&mut Branch)) {
        let Instr71::Branch(b) = &mut p[at] else { panic!("not a branch") };
        f(b);
    }
    fn thrsw_pair(p: &mut [Instr71], at: usize) {
        for w in &mut p[at..at + 2] {
            let Instr71::Alu(a) = w else { panic!("not an ALU word") };
            a.sig.thrsw = true;
        }
    }
    fn rule(p: &[Instr71], edges: &[(u32, u32)], fwd: &[(u32, u32)]) -> Result<(), &'static str> {
        branches_are_structured71(p, edges, fwd).map_err(|e| e.rule)
    }

    #[test]
    fn accepts_a_loop_a_forward_branch_and_a_break() {
        let p = prog(40, &[(20, 10)]);
        assert_eq!(rule(&p, &[(20, 10)], &[]), Ok(()));
        let p = prog(40, &[(5, 30)]);
        assert_eq!(rule(&p, &[], &[(5, 30)]), Ok(()));
        /* a break to the loop's exit, a pair after the loop, at the top */
        let mut p = prog(40, &[(12, 24), (20, 10)]);
        thrsw_pair(&mut p, 30);
        assert_eq!(rule(&p, &[(20, 10)], &[(12, 24)]), Ok(()));
        assert_eq!(rule(&prog(4, &[]), &[], &[]), Ok(()));
    }

    #[test]
    fn forward_branch_rules() {
        let mut p = prog(40, &[(5, 30)]);
        edit(&mut p, 5, |b| b.ub = false);
        assert_eq!(rule(&p, &[], &[(5, 30)]), Err("a forward branch that is not REL with the uniform stream moved"));
        let p = prog(40, &[(5, 30)]);
        assert_eq!(rule(&p, &[], &[(5, 31)]), Err("a forward branch whose target is not its label"));
        let p = prog(40, &[(20, 10)]);
        assert_eq!(rule(&p, &[], &[(20, 10)]), Err("a forward branch that is not forward inside the program"));
        let p = prog(40, &[(5, 50)]);
        assert_eq!(rule(&p, &[], &[(5, 50)]), Err("a forward branch that is not forward inside the program"));
        let p = prog(40, &[(5, 15), (20, 10)]);
        assert_eq!(rule(&p, &[(20, 10)], &[(5, 15)]), Err("a forward branch into a loop body"));
        let p = prog(40, &[(12, 30), (20, 10)]);
        assert_eq!(rule(&p, &[(20, 10)], &[(12, 30)]), Err("a forward branch out of a loop body, not to its exit"));
    }

    #[test]
    fn back_edge_rules() {
        let p = prog(40, &[(20, 10)]);
        assert_eq!(rule(&p, &[], &[]), Err("a branch that is neither a loop's back edge nor a forward branch"));
        let mut p = prog(40, &[(20, 10)]);
        edit(&mut p, 20, |b| b.cond = BranchCond::Always);
        assert_eq!(rule(&p, &[(20, 10)], &[]), Err("an unconditional back edge"));
        let mut p = prog(40, &[(20, 10)]);
        edit(&mut p, 20, |b| b.bdu = BranchDest::Abs);
        assert_eq!(rule(&p, &[(20, 10)], &[]), Err("a back edge that is not REL with the uniform stream moved"));
        let p = prog(40, &[(20, 10)]);
        assert_eq!(rule(&p, &[(20, 11)], &[]), Err("a back edge whose target is not its loop's head"));
        let p = prog(40, &[(20, 30)]);
        assert_eq!(rule(&p, &[(20, 30)], &[]), Err("a back edge that is not backward inside the program"));
    }

    #[test]
    fn recorded_edges_must_be_branches() {
        let p = prog(40, &[]);
        assert_eq!(rule(&p, &[(20, 10)], &[]), Err("a recorded back edge that is not a branch"));
        assert_eq!(rule(&p, &[], &[(5, 30)]), Err("a recorded forward branch that is not a branch"));
    }

    #[test]
    fn loops_nest_or_follow_never_cross() {
        let p = prog(40, &[(20, 10), (30, 15)]);
        assert_eq!(rule(&p, &[(20, 10), (30, 15)], &[]), Err("two loops that cross"));
        let p = prog(40, &[(30, 10), (20, 15)]);
        assert_eq!(rule(&p, &[(30, 10), (20, 15)], &[]), Ok(()));
        let p = prog(40, &[(20, 10), (35, 26)]);
        assert_eq!(rule(&p, &[(20, 10), (35, 26)], &[]), Ok(()));
    }

    #[test]
    fn last_thrsw_pair_is_at_the_top_level() {
        let mut p = prog(40, &[(20, 10)]);
        thrsw_pair(&mut p, 12);
        assert_eq!(rule(&p, &[(20, 10)], &[]), Err("the last-thread-switch pair in a loop body"));
        let mut p = prog(40, &[(5, 30)]);
        thrsw_pair(&mut p, 12);
        assert_eq!(rule(&p, &[], &[(5, 30)]), Err("the last-thread-switch pair where a forward branch skips it"));
        let mut p = prog(40, &[(5, 30)]);
        thrsw_pair(&mut p, 32);
        assert_eq!(rule(&p, &[], &[(5, 30)]), Ok(()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::powervr::unroll::unroll;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::v3d::alloc71::{allocate71, Rf};
    use crate::shaderc::online::v3d::codec71::{decode71, encode71};
    use crate::shaderc::online::v3d::isa::{small_imm_index, Pf, Uf};
    use crate::shaderc::online::v3d::isa71::Alu71;
    use crate::shaderc::online::v3d::listing71::write_instr71;
    use crate::shaderc::online::v3d::prepare::{fragment_inputs, prepare_fragment_with};
    use crate::shaderc::online::v3d::sched71::pack71;
    use crate::shaderc::online::v3d::select::{select_fragment, Pressure, QUNIFORM_CONSTANT, QUNIFORM_UNIFORM};
    use crate::shaderc::online::v3d::select71::select71;
    use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, Varying, LANES};
    use crate::shaderc::online::v3d::sim71::run71;
    use crate::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
    use crate::shaderc::online::v3d::vop71::{Dst71, FlagOp71, Label, Src71, V};
    use crate::shaderc::online::{lower, store};
    use std::boxed::Box;
    use std::string::String;
    use std::vec;
    use std::vec::Vec;

    fn v(n: u32) -> Src71 {
        Src71::V(V(n))
    }
    fn imm(x: u32) -> Src71 {
        Src71::SmallImm(small_imm_index(x).expect("a small immediate"))
    }
    fn u(x: f32) -> Uniform {
        Uniform::constant(x.to_bits())
    }
    fn tlb(n: u32) -> Op71 {
        Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(n), b: Src71::None }
    }
    fn fadd(d: u32, x: Src71, y: Src71) -> Op71 {
        Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(d)), a: x, b: y }
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

    /// What every scheduled program must be: structured, verified at 7.1,
    /// fitting `threads`, and a codec71 round trip to the same
    /// instructions. Returns the words.
    fn check(s: &Scheduled71<'_>, threads: u32, single_seg: bool) -> Vec<u64> {
        let text = listing(s.instrs);
        branches_are_structured71(s.instrs, s.back_edges, s.fwd_branches).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        verify71(s.instrs, single_seg).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        registers_fit_threads71(s.instrs, threads).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
        let words: Vec<u64> =
            s.instrs.iter().map(|i| encode71(i).unwrap_or_else(|e| panic!("{e:?} encoding {i:?}\n{text}"))).collect();
        for (i, w) in s.instrs.iter().zip(&words) {
            assert_eq!(decode71(*w).expect("decodes"), *i, "{text}");
        }
        words
    }

    /// The uniform stream resolved for sim71: constants as they are, user
    /// uniforms (dword `data` of the block) from `user`.
    fn stream(unifs: &[Uniform], user: &[f32]) -> Vec<u32> {
        unifs
            .iter()
            .map(|un| match un.contents {
                QUNIFORM_CONSTANT => un.data,
                QUNIFORM_UNIFORM => user[un.data as usize].to_bits(),
                c => panic!("uniform contents {c} not modelled here"),
            })
            .collect()
    }

    fn fs_inputs(z: [u32; LANES], varyings: Vec<Varying>) -> Inputs {
        let one = [1.0f32.to_bits(); LANES];
        Inputs::Fragment(Box::new(FsInputs {
            w: one,
            w_centroid: one,
            z,
            x: one,
            y: one,
            back_facing: [false; LANES],
            varyings,
        }))
    }

    fn run(words: &[u64], unif: &[u32], inputs: &Inputs, threads: u32) -> Outcome {
        run71(words, unif, inputs, &Env { threads, ..Env::default() }).unwrap_or_else(|e| panic!("{e}"))
    }

    /// The one value every lane of TLB write `n` holds.
    fn tlb_f(o: &Outcome, n: usize) -> f32 {
        let l = o.tlb[n].value;
        assert!(l.iter().all(|x| *x == l[0]), "{l:?}");
        f32::from_bits(l[0].expect("defined"))
    }

    /// RT0's colour per lane from two vfpack writes (r, g) then (b, a), as
    /// tests/v3d71_common reads it.
    fn colour(o: &Outcome) -> [[f32; 4]; LANES] {
        assert_eq!(o.tlb.len(), 2, "two colour TLB writes");
        let h = |w: Option<u32>, hi: bool| {
            let w = w.expect("a defined colour");
            sim::half_to_f32(if hi { (w >> 16) as u16 } else { w as u16 })
        };
        core::array::from_fn(|l| {
            let (p, q) = (o.tlb[0].value[l], o.tlb[1].value[l]);
            [h(p, false), h(p, true), h(q, false), h(q, true)]
        })
    }

    fn close(got: f32, want: f32) -> bool {
        (got - want).abs() <= 1.0 / 1024.0 + want.abs() / 1024.0
    }

    /// `ops`, a fragment shader with `vregs` registers, allocated at
    /// `threads` and scheduled, handed to `f` with its words.
    fn with_ops<R>(ops: &[Op71], vregs: u32, threads: u32, f: impl FnOnce(&Scheduled71<'_>, &[u64]) -> R) -> R {
        store::with_scratch(|a| {
            let sel = Selected71 { ops, fragment: true, vregs };
            let al = allocate71(&sel, threads, a).expect("allocates");
            let s = schedule71(&sel, &al, a).unwrap_or_else(|e| panic!("{e:?}"));
            let words = check(&s, threads, false);
            f(&s, &words)
        })
        .expect("scratch")
    }

    /// The fragment shader `src` through the shared front half (the
    /// unroller, prepare, the 4.2 selection at Pressure::None, as chain71
    /// drives them), select71, allocate71 at 4 threads and schedule71,
    /// handed to `f` with its words; or the stage that refused it, by name.
    fn with_fs<R>(src: &str, f: impl FnOnce(Result<(&Scheduled71<'_>, &[u64]), String>) -> R) -> R {
        store::with_scratch(|a| {
            let s = a.alloc_str(src).unwrap();
            let m = lower::compile_to_ir(a, Stage::Fragment, s).map_err(|e| e.msg).unwrap();
            let (m, trips) = match unroll(&m.module, a) {
                Ok(Some(u)) => (u.module, u.trips),
                Ok(None) => (&m.module, &[][..]),
                Err(e) => return f(Err(std::format!("unroll: {}", e.message()))),
            };
            let p = match prepare_fragment_with(m, trips, 0, a) {
                Ok(p) => p,
                Err(e) => return f(Err(std::format!("prepare: {e:?}"))),
            };
            let inputs = fragment_inputs(&p, a).expect("inputs");
            let sel = match select_fragment(&p, inputs, 0, 0, Pressure::None, a) {
                Ok(s) => s,
                Err(e) => return f(Err(std::format!("select: {e:?}"))),
            };
            let sel71 = match select71(&sel, a) {
                Ok(s) => s,
                Err(e) => return f(Err(std::format!("select71: {e:?}"))),
            };
            let al = match allocate71(&sel71, 4, a) {
                Ok(al) => al,
                Err(e) => return f(Err(std::format!("allocate71: {e:?}"))),
            };
            let s = schedule71(&sel71, &al, a).unwrap_or_else(|e| panic!("schedule71: {e:?}"));
            let words = check(&s, 4, false);
            f(Ok((&s, &words)))
        })
        .expect("scratch")
    }

    fn branch_at(instrs: &[Instr71], at: usize) -> &Branch {
        match &instrs[at] {
            Instr71::Branch(b) => b,
            Instr71::Alu(_) => panic!("word {at} is not a branch\n{}", listing(instrs)),
        }
    }
    fn alu(i: &Instr71) -> &Alu71 {
        match i {
            Instr71::Alu(a) => a,
            Instr71::Branch(_) => panic!("a branch"),
        }
    }

    /// A program with no control flow is pack71's, word for word and
    /// uniform for uniform, with no edges: the pass-through fragment shader
    /// and the uniform colour of sched71's tests.
    #[test]
    fn a_branch_free_program_is_pack71s_words() {
        let pass = [
            Op71::Ldvary { dst: V(0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(1)), a: v(0), b: Src71::Payload(3) },
            fadd(2, v(1), Src71::LdvaryC),
            Op71::Ldvary { dst: V(3) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(4)), a: v(3), b: Src71::Payload(3) },
            fadd(5, v(4), Src71::LdvaryC),
            Op71::Thrsw,
            Op71::Thrsw,
            tlb(2),
            tlb(5),
        ];
        let unif = [
            Op71::Ldunif { dst: V(0), u: u(0.25) },
            Op71::Ldunif { dst: V(1), u: u(0.5) },
            fadd(2, v(0), v(1)),
            Op71::Thrsw,
            Op71::Thrsw,
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlbu(Uniform::constant(0xffff_ff7a)), a: v(2), b: Src71::None },
        ];
        for (ops, regs) in [(&pass[..], &[4u8, 5, 6, 7, 8, 9][..]), (&unif[..], &[4u8, 0, 5][..])] {
            let rfs: Vec<Rf> = regs.iter().map(|&r| Rf(r)).collect();
            store::with_scratch(|a| {
                let al = Allocation71 { reg: &rfs, rf_end: regs.iter().copied().max().unwrap_or(0) + 1 };
                let sel = Selected71 { ops, fragment: true, vregs: regs.len() as u32 };
                let packed = pack71(ops, &al, a).expect("packs");
                let s = schedule71(&sel, &al, a).expect("schedules");
                assert_eq!(s.instrs, packed.instrs, "{}", listing(s.instrs));
                assert_eq!(s.uniforms, packed.uniforms);
                assert!(s.back_edges.is_empty() && s.fwd_branches.is_empty());
                check(&s, 4, false);
            })
            .expect("scratch");
        }
    }

    /// A uniform `if` by hand: `t = 1.0; if (any lane has Z >= u) skip else
    /// t = 2.0; colour = t + (1.0 + 1.0)`. The branch is conditional on the
    /// flags a FCMP pushed; the FADD after the push has no signal, so the
    /// branch moves back over it and it runs in the first delay slot
    /// (Mesa's emit_branch). Both paths in sim71: every lane below the
    /// threshold falls through (4.0), one lane at or above it jumps (3.0).
    #[test]
    fn a_forward_branch_runs_both_paths_in_sim71() {
        let ops = [
            Op71::Ldunif { dst: V(0), u: u(0.25) },
            Op71::Ldunif { dst: V(1), u: u(1.0) },
            Op71::Flags {
                unit: FlagOp71::Add(AddOp71::Fcmp),
                a: Src71::Payload(2),
                b: v(0),
                pf: Pf::PushN,
                uf: Uf::None,
            },
            fadd(3, v(1), v(1)),
            Op71::FwdBranch { cond: BranchCond::AnyNA, msfign: Msfign::None, label: Label(0) },
            Op71::Ldunif { dst: V(2), u: u(2.0) },
            Op71::Copy { dst: V(1), src: v(2) },
            Op71::Label(Label(0)),
            fadd(4, v(1), v(3)),
            Op71::Thrsw,
            Op71::Thrsw,
            tlb(4),
        ];
        with_ops(&ops, 5, 4, |s, words| {
            let text = listing(s.instrs);
            assert!(s.back_edges.is_empty(), "{text}");
            assert_eq!(s.fwd_branches.len(), 1, "{text}");
            let (b, t) = (s.fwd_branches[0].0 as usize, s.fwd_branches[0].1 as usize);
            let br = branch_at(s.instrs, b);
            assert_eq!((br.cond, br.ub, br.bdi, br.bdu), (BranchCond::AnyNA, true, BranchDest::Rel, BranchDest::Rel));
            assert_eq!(t as i64, b as i64 + 4 + (br.offset as i32 as i64) / 8, "{text}");
            /* the FADD moved into the first slot; the push stays before */
            assert_eq!(alu(&s.instrs[b + 1]).add.op, AddOp71::Fadd, "{text}");
            assert_eq!(alu(&s.instrs[b - 1]).flags.apf, Pf::PushN, "{text}");
            assert!(s.instrs[b + 2..b + 4].iter().all(|i| *alu(i) == Alu71::nop()), "{text}");
            /* the stream: two loads, the branch's own word (over the
             * skipped load: +4 bytes), the skipped load, the TLB's */
            let unif = stream(s.uniforms, &[]);
            assert_eq!(unif[2], 4, "{unif:?}\n{text}");
            let z = |x: f32| [x.to_bits(); LANES];
            let o = run(words, &unif, &fs_inputs(z(0.1), Vec::new()), 4);
            assert_eq!(tlb_f(&o, 0), 4.0, "not taken: the then arm ran\n{text}");
            let o = run(words, &unif, &fs_inputs(z(0.5), Vec::new()), 4);
            assert_eq!(tlb_f(&o, 0), 3.0, "taken: the then arm skipped\n{text}");
            let mut mixed = z(0.1);
            mixed[7] = 0.5f32.to_bits();
            let o = run(words, &unif, &fs_inputs(mixed, Vec::new()), 4);
            assert_eq!(tlb_f(&o, 0), 3.0, "one lane at the threshold: taken for all\n{text}");
        });
    }

    /// The P8 `if` shape (tests/v3d71_common `if_branch`): a varying against
    /// a uniform picks one of two colours. Our select if-converts a varying
    /// condition (prepare.rs `if_convert`: only a uniform condition in a
    /// fragment shader with arms over BRANCH_LIMIT or a look-up becomes a
    /// uniform branch), so the program is straight-line code with
    /// conditional moves and schedule71 gives pack71's words; per lane,
    /// both colours come out right.
    #[test]
    fn the_p8_if_shape_at_7_1() {
        const FS: &str = "precision mediump float;\nuniform float u_t;\nvarying float v_x;\nvoid main() {\n    if (v_x > u_t) {\n        gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);\n    } else {\n        gl_FragColor = vec4(0.0, 0.0, 1.0, 1.0);\n    }\n}\n";
        const THEN: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
        const ELSE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
        with_fs(FS, |r| {
            let (s, words) = r.expect("the if shape compiles at 7.1");
            let text = listing(s.instrs);
            assert!(s.fwd_branches.is_empty() && s.back_edges.is_empty(), "if-converted\n{text}");
            let unif = stream(s.uniforms, &[0.5]);
            let x: [f32; LANES] = core::array::from_fn(|l| l as f32 / 16.0);
            let vary = Varying { vary: x.map(f32::to_bits), c: [0; LANES] };
            let o = run(words, &unif, &fs_inputs([0.5f32.to_bits(); LANES], vec![vary]), 4);
            let got = colour(&o);
            for l in 0..LANES {
                let want = if x[l] > 0.5 { THEN } else { ELSE };
                assert!((0..4).all(|c| close(got[l][c], want[c])), "lane {l}: {:?}, want {want:?}\n{text}", got[l]);
            }
        });
    }

    /// An `if` on a uniform with an arm past BRANCH_LIMIT: prepare makes it a
    /// uniform branch (SOp::IfBranch), select a FwdBranch round the then arm,
    /// an unconditional one over the else arm and two labels. Both paths in
    /// sim71: u_t = 1.0 takes the then arm (the polynomial of the varying),
    /// u_t = 0.0 the else arm (blue).
    #[test]
    fn a_uniform_if_branches_both_ways_in_sim71() {
        const FS: &str = "precision mediump float;\nuniform float u_t;\nvarying vec4 v_c;\nvoid main() {\n    if (u_t > 0.5) {\n        vec4 c = v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        c = c * v_c + v_c;\n        gl_FragColor = c;\n    } else {\n        gl_FragColor = vec4(0.0, 0.0, 1.0, 1.0);\n    }\n}\n";
        with_fs(FS, |r| {
            let (s, words) = r.expect("the uniform if compiles at 7.1");
            let text = listing(s.instrs);
            std::println!("uniform if: {} words, branches {:?}\n{text}", s.instrs.len(), s.fwd_branches);
            assert!(s.back_edges.is_empty(), "{text}");
            assert!(!s.fwd_branches.is_empty(), "a uniform branch\n{text}");
            for &(b, t) in s.fwd_branches {
                let br = branch_at(s.instrs, b as usize);
                assert!(br.ub && br.bdi == BranchDest::Rel && br.bdu == BranchDest::Rel, "{text}");
                assert!(t >= b + 4, "{text}");
            }
            let x: [[f32; LANES]; 4] =
                core::array::from_fn(|k| core::array::from_fn(|l| ((l + 3 * k) % 8) as f32 / 32.0));
            let vary: Vec<Varying> = x.iter().map(|c| Varying { vary: c.map(f32::to_bits), c: [0; LANES] }).collect();
            let z = [0.5f32.to_bits(); LANES];
            for (ut, then_) in [(1.0f32, true), (0.0, false)] {
                let unif = stream(s.uniforms, &[ut]);
                let o = run(words, &unif, &fs_inputs(z, vary.clone()), 4);
                let got = colour(&o);
                for l in 0..LANES {
                    let want: [f32; 4] = if then_ {
                        core::array::from_fn(|k| {
                            let v = x[k][l];
                            (0..13).fold(v, |c, _| c * v + v)
                        })
                    } else {
                        [0.0, 0.0, 1.0, 1.0]
                    };
                    assert!(
                        (0..4).all(|c| close(got[l][c], want[c])),
                        "u_t {ut}, lane {l}: {:?}, want {want:?}\n{text}",
                        got[l]
                    );
                }
            }
        });
    }

    /// A counted loop by hand: `acc = 0; n = 3; do { acc += 0.25; n -= 1 }
    /// while (any lane has n != 0)`, the counter's zero test pushed with
    /// XOR (the one Z flag sim describes). The back edge is conditional,
    /// REL with the stream moved, back to the head; the carried values are
    /// Copy's. Three passes: 0.75.
    #[test]
    fn a_counted_loop_runs_its_passes_in_sim71() {
        let ops = [
            Op71::Ldunif { dst: V(0), u: Uniform::constant(3) },
            Op71::Ldunif { dst: V(1), u: u(0.0) },
            Op71::Ldunif { dst: V(2), u: u(0.25) },
            Op71::LoopHead,
            fadd(3, v(1), v(2)),
            Op71::Copy { dst: V(1), src: v(3) },
            Op71::Add { op: AddOp71::Sub, dst: Dst71::V(V(4)), a: v(0), b: imm(1) },
            Op71::Copy { dst: V(0), src: v(4) },
            Op71::Flags { unit: FlagOp71::Add(AddOp71::Xor), a: v(0), b: imm(0), pf: Pf::PushZ, uf: Uf::None },
            Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None },
            Op71::Thrsw,
            Op71::Thrsw,
            tlb(1),
        ];
        with_ops(&ops, 5, 4, |s, words| {
            let text = listing(s.instrs);
            assert!(s.fwd_branches.is_empty(), "{text}");
            assert_eq!(s.back_edges.len(), 1, "{text}");
            let (b, h) = (s.back_edges[0].0 as usize, s.back_edges[0].1 as usize);
            let br = branch_at(s.instrs, b);
            assert_eq!((br.cond, br.ub, br.bdi, br.bdu), (BranchCond::AnyNA, true, BranchDest::Rel, BranchDest::Rel));
            assert_eq!(h as i64, b as i64 + 4 + (br.offset as i32 as i64) / 8, "{text}");
            assert!(h < b, "{text}");
            /* the branch waits for its push and stays after it */
            assert_eq!(alu(&s.instrs[b - 1]).flags.apf, Pf::PushZ, "{text}");
            /* the stream moves back to the head's first uniform: none are
             * consumed in the body, so the branch's own word says -4 (the
             * word after itself, minus one) */
            let unif = stream(s.uniforms, &[]);
            assert_eq!(unif.len(), 4, "{unif:?}");
            assert_eq!(unif[3] as i32, -4, "{unif:?}");
            let o = run(words, &unif, &fs_inputs([0.5f32.to_bits(); LANES], Vec::new()), 4);
            assert_eq!(tlb_f(&o, 0), 0.75, "three passes\n{text}");
            assert_eq!(o.uniforms_read, 3 + 3, "the branch's word read on every pass\n{text}");
        });
    }

    /// A GLSL loop that leaves on data (tools/shaders/es100/loopcarry.frag's
    /// shape): the unroller keeps it as a counted loop with its exit as a
    /// flag, and select gives it a LoopHead, a flag-carried break and a
    /// conditional back edge. Scheduled, checked and run: with u_k = (1,
    /// 0.25) three passes of 0.25 into red. A refusal by an earlier stage
    /// would be named, not a failure here; today none is.
    #[test]
    fn a_glsl_counted_loop_at_7_1() {
        const FS: &str = "precision mediump float;\nuniform vec4 u_k;\nvoid main() {\n    float n = ceil(u_k.x * 3.0);\n    float k = u_k.y;\n    float acc = 0.0;\n    for (float i = 0.0; i < 8.0; i += 1.0) {\n        if (i >= n) {\n            break;\n        }\n        acc += k;\n        float t = acc * 2.0;\n        acc = t * 0.5;\n    }\n    gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);\n}\n";
        with_fs(FS, |r| {
            let (s, words) = r.expect("the counted loop compiles at 7.1");
            let text = listing(s.instrs);
            std::println!(
                "counted loop: {} words, back edges {:?}, forward {:?}\n{text}",
                s.instrs.len(),
                s.back_edges,
                s.fwd_branches
            );
            assert_eq!(s.back_edges.len(), 1, "{text}");
            let (b, h) = s.back_edges[0];
            let br = branch_at(s.instrs, b as usize);
            assert!(br.cond != BranchCond::Always && br.ub, "{text}");
            for &(fb, ft) in s.fwd_branches {
                /* a break: out of the body to its exit */
                assert!(fb > h && fb < b && ft == b + 4, "{text}");
            }
            let unif = stream(s.uniforms, &[1.0, 0.25, 0.0, 0.0]);
            let o = run(words, &unif, &fs_inputs([0.5f32.to_bits(); LANES], Vec::new()), 4);
            let got = colour(&o);
            for l in 0..LANES {
                assert!(close(got[l][0], 0.75) && close(got[l][3], 1.0), "lane {l}: {:?}\n{text}", got[l]);
            }
            /* two trips with u_k.x = 0.5: n = 2 */
            let unif = stream(s.uniforms, &[0.5, 0.25, 0.0, 0.0]);
            let o = run(words, &unif, &fs_inputs([0.5f32.to_bits(); LANES], Vec::new()), 4);
            assert!(close(colour(&o)[0][0], 0.5), "{:?}\n{text}", colour(&o)[0]);
        });
    }

    /// A branch never lands in a THRSW's delay slots, nor does a THRSW's
    /// merge-back reach into a branch's: a look-up's switch right before a
    /// back edge, and the last-thread-switch pair right after a label.
    #[test]
    fn branches_and_thread_switches_keep_out_of_each_others_slots() {
        let ops = [
            Op71::Ldunif { dst: V(0), u: Uniform::constant(2) },
            Op71::Ldunif { dst: V(1), u: u(0.0) },
            Op71::LoopHead,
            fadd(2, v(1), Src71::Payload(2)),
            Op71::Copy { dst: V(1), src: v(2) },
            Op71::Add { op: AddOp71::Sub, dst: Dst71::V(V(3)), a: v(0), b: imm(1) },
            Op71::Copy { dst: V(0), src: v(3) },
            Op71::Flags { unit: FlagOp71::Add(AddOp71::Xor), a: v(0), b: imm(0), pf: Pf::PushZ, uf: Uf::None },
            Op71::Thrsw,
            Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None },
            Op71::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: Label(0) },
            Op71::Ldunif { dst: V(4), u: u(9.0) },
            Op71::Copy { dst: V(1), src: v(4) },
            Op71::Label(Label(0)),
            Op71::Thrsw,
            Op71::Thrsw,
            tlb(1),
        ];
        with_ops(&ops, 5, 4, |s, words| {
            let text = listing(s.instrs);
            let thrsws: Vec<usize> = s
                .instrs
                .iter()
                .enumerate()
                .filter(|(_, i)| matches!(i, Instr71::Alu(a) if a.sig.thrsw))
                .map(|(k, _)| k)
                .collect();
            let branches: Vec<usize> =
                s.instrs.iter().enumerate().filter(|(_, i)| matches!(i, Instr71::Branch(_))).map(|(k, _)| k).collect();
            assert_eq!(branches.len(), 2, "{text}");
            for &b in &branches {
                for &t in &thrsws {
                    assert!(!(b > t && b <= t + 2), "branch {b} in the slots of the THRSW at {t}\n{text}");
                    assert!(!(t > b && t <= b + 3), "THRSW {t} in the slots of the branch at {b}\n{text}");
                }
            }
            let (b, h) = s.back_edges[0];
            let f = s.fwd_branches[0];
            assert!(f.0 >= b + 4 && f.1 >= f.0 + 4 && h < b, "{text}");
            let unif = stream(s.uniforms, &[]);
            let o = run(words, &unif, &fs_inputs([0.5f32.to_bits(); LANES], Vec::new()), 4);
            assert_eq!(tlb_f(&o, 0), 1.0, "two passes of Z, the skipped 9.0 never loaded\n{text}");
        });
    }

    /// What schedule71 refuses, by name, never scheduling: a back edge with
    /// no open head, a head never closed, a label defined twice, a branch to
    /// a label that is never defined, a conditional forward branch with no
    /// flag push before it, a branch to a label before it.
    #[test]
    fn refusals_by_name() {
        let edge = Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None };
        let to = |l: u32| Op71::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: Label(l) };
        let cond_to = |l: u32| Op71::FwdBranch { cond: BranchCond::AnyA, msfign: Msfign::None, label: Label(l) };
        let push = Op71::Flags { unit: FlagOp71::Add(AddOp71::Xor), a: v(0), b: imm(0), pf: Pf::PushZ, uf: Uf::None };
        let ld = Op71::Ldunif { dst: V(0), u: u(1.0) };
        let refusal = |ops: &[Op71]| {
            store::with_scratch(|a| {
                let sel = Selected71 { ops, fragment: true, vregs: 1 };
                let al = Allocation71 { reg: &[Rf(4)], rf_end: 5 };
                schedule71(&sel, &al, a).err()
            })
            .expect("scratch")
        };
        assert_eq!(refusal(&[ld, push, edge]), Some(Sched71Error::ControlFlow(2)));
        assert_eq!(refusal(&[ld, Op71::LoopHead, tlb(0)]), Some(Sched71Error::ControlFlow(1)));
        assert_eq!(
            refusal(&[ld, to(0), Op71::Label(Label(0)), Op71::Label(Label(0))]),
            Some(Sched71Error::ControlFlow(3))
        );
        assert_eq!(refusal(&[ld, to(1), Op71::Label(Label(0))]), Some(Sched71Error::ControlFlow(1)));
        assert_eq!(refusal(&[ld, to(0)]), Some(Sched71Error::ControlFlow(1)));
        assert_eq!(refusal(&[ld, cond_to(0), Op71::Label(Label(0))]), Some(Sched71Error::CondWithoutFlags(1)));
        /* a label is forward only: a branch after it is not a branch to it */
        assert_eq!(refusal(&[ld, Op71::Label(Label(0)), to(0)]), Some(Sched71Error::ControlFlow(2)));
    }
}

/// Unit tests of the delay-slot rules and the error paths, each on its own:
/// [`ok_in_branch_slot71`] on single words, [`branch_back`] on a packer of
/// hand-placed words, `resolve` and `schedule71` on the errors the whole
/// program tests do not reach.
#[cfg(test)]
mod slot_tests {
    use super::*;
    use crate::shaderc::online::arena::Arena;
    use crate::shaderc::online::store;
    use crate::shaderc::online::v3d::alloc71::Rf;
    use crate::shaderc::online::v3d::isa::{small_imm_index, Pf, Uf};
    use crate::shaderc::online::v3d::vop71::{FlagOp71, Label, Src71, V};

    fn word() -> Word71 {
        Word71::empty()
    }
    fn with_word(f: impl FnOnce(&mut Word71)) -> Word71 {
        let mut w = word();
        f(&mut w);
        w
    }
    /// Accepted or refused, for an unconditional then a conditional branch.
    fn verdicts(w: &Word71) -> (bool, bool) {
        (ok_in_branch_slot71(w, false), ok_in_branch_slot71(w, true))
    }
    const BOTH: (bool, bool) = (true, true);
    const NEITHER: (bool, bool) = (false, false);

    fn branch(cond: BranchCond) -> Branch {
        Branch {
            cond,
            msfign: Msfign::None,
            bdi: BranchDest::Rel,
            ub: true,
            bdu: BranchDest::Rel,
            raddr_a: 0,
            offset: 0,
        }
    }

    #[test]
    fn slot_accepts_a_nop_word_and_plain_arithmetic() {
        assert_eq!(verdicts(&word()), BOTH);
        /* adds and muls to the register file (not magic), small immediates */
        let w = with_word(|w| {
            w.alu.add.op = AddOp71::Fadd;
            w.alu.add.magic = false;
            w.alu.add.waddr = 4;
            w.alu.mul.op = MulOp71::Fmul;
            w.alu.mul.magic = false;
            w.alu.mul.waddr = 5;
            w.alu.sig.small_imm_a = true;
            w.alu.sig.small_imm_b = true;
            w.alu.sig.small_imm_c = true;
            w.alu.sig.small_imm_d = true;
        });
        assert_eq!(verdicts(&w), BOTH);
    }

    #[test]
    fn slot_refuses_a_branch_and_a_uniform() {
        assert_eq!(verdicts(&with_word(|w| w.branch = Some(branch(BranchCond::Always)))), NEITHER);
        /* the branch moves the uniform stream: a uniform read after it
         * would read the target's */
        assert_eq!(verdicts(&with_word(|w| w.uniform = Some(Uniform::constant(1)))), NEITHER);
    }

    #[test]
    fn slot_refuses_every_signal_but_the_small_immediates() {
        type Set = fn(&mut Sig71);
        let sigs: [(&str, Set); 12] = [
            ("thrsw", |s| s.thrsw = true),
            ("ldunif", |s| s.ldunif = true),
            ("ldunifa", |s| s.ldunifa = true),
            ("ldunifrf", |s| s.ldunifrf = true),
            ("ldunifarf", |s| s.ldunifarf = true),
            ("ldtmu", |s| s.ldtmu = true),
            ("ldvary", |s| s.ldvary = true),
            ("ldtlb", |s| s.ldtlb = true),
            ("ldtlbu", |s| s.ldtlbu = true),
            ("ucb", |s| s.ucb = true),
            ("rotate", |s| s.rotate = true),
            ("wrtmuc", |s| s.wrtmuc = true),
        ];
        for (name, set) in sigs {
            let w = with_word(|w| set(&mut w.alu.sig));
            assert_eq!(verdicts(&w), NEITHER, "signal {name}");
        }
    }

    #[test]
    fn slot_refuses_tlb_and_tmu_accesses() {
        assert_eq!(verdicts(&with_word(|w| w.tlb = true)), NEITHER);
        assert_eq!(verdicts(&with_word(|w| w.tmu_write = true)), NEITHER);
    }

    #[test]
    fn slot_refuses_a_flag_push_only_before_a_conditional_branch() {
        let w = with_word(|w| w.flags_push = true);
        assert_eq!(verdicts(&w), (true, false));
    }

    #[test]
    fn slot_refuses_sfu_vpm_and_wait_operations() {
        for op in [
            AddOp71::Recip,
            AddOp71::Rsqrt,
            AddOp71::Exp,
            AddOp71::Log,
            AddOp71::Sin,
            AddOp71::Rsqrt2,
            AddOp71::Ballot,
            AddOp71::Bcastf,
            AddOp71::Alleq,
            AddOp71::Allfeq,
            AddOp71::Rotq,
            AddOp71::Rot,
            AddOp71::Shuffle,
            AddOp71::Vpmsetup,
            AddOp71::Vpmwt,
            AddOp71::LdvpmvIn,
            AddOp71::LdvpmvOut,
            AddOp71::LdvpmdIn,
            AddOp71::LdvpmdOut,
            AddOp71::Ldvpmp,
            AddOp71::LdvpmgIn,
            AddOp71::LdvpmgOut,
            AddOp71::Stvpmv,
            AddOp71::Stvpmd,
            AddOp71::Stvpmp,
            AddOp71::Tmuwt,
            AddOp71::Barrierid,
        ] {
            /* not a magic write: the operation alone refuses the word */
            let w = with_word(|w| {
                w.alu.add.op = op;
                w.alu.add.magic = false;
            });
            assert_eq!(verdicts(&w), NEITHER, "{op:?}");
        }
    }

    #[test]
    fn slot_magic_writes_only_to_nop() {
        /* a magic write to NOP is no write */
        let nop = with_word(|w| {
            w.alu.add.op = AddOp71::Fadd;
            w.alu.add.magic = true;
            w.alu.add.waddr = WADDR_NOP;
            w.alu.mul.op = MulOp71::Fmul;
            w.alu.mul.magic = true;
            w.alu.mul.waddr = WADDR_NOP;
        });
        assert_eq!(verdicts(&nop), BOTH);
        /* any other magic address (TLB, TMU, SFU result, ...) on either unit */
        for waddr in [0u8, 1, 7, 9, 20, 36, 42] {
            let w = with_word(|w| {
                w.alu.add.op = AddOp71::Fadd;
                w.alu.add.magic = true;
                w.alu.add.waddr = waddr;
            });
            assert_eq!(verdicts(&w), NEITHER, "add magic {waddr}");
            let w = with_word(|w| {
                w.alu.mul.op = MulOp71::Fmul;
                w.alu.mul.magic = true;
                w.alu.mul.waddr = waddr;
            });
            assert_eq!(verdicts(&w), NEITHER, "mul magic {waddr}");
        }
        /* an unused unit's address is not a write */
        let w = with_word(|w| {
            w.alu.add.waddr = 1;
            w.alu.mul.waddr = 1;
        });
        assert_eq!(verdicts(&w), BOTH);
    }

    #[test]
    fn slot_refuses_multop_and_umul24() {
        for op in [MulOp71::Multop, MulOp71::Umul24] {
            let w = with_word(|w| {
                w.alu.mul.op = op;
                w.alu.mul.magic = false;
                w.alu.mul.waddr = 4;
            });
            assert_eq!(verdicts(&w), NEITHER, "{op:?}");
        }
    }

    /// A packer over one virtual register in rf4, handed to `f` with its
    /// arena.
    fn with_packer<R>(f: impl for<'a> FnOnce(&mut Packer71<'a, '_>, &Arena<'a>) -> R) -> R {
        store::with_scratch(|a| {
            let al = Allocation71 { reg: &[Rf(4)], rf_end: 5 };
            let mut p = Packer71::new(&al, a).expect("packer");
            f(&mut p, a)
        })
        .expect("scratch")
    }
    /// An `ldunif` (a word the slot rules refuse), alone in word `p.len()`.
    fn put_refused<'a>(p: &mut Packer71<'a, '_>, a: &Arena<'a>) {
        p.fence();
        let op = Op71::Ldunif { dst: V(0), u: Uniform::constant(1) };
        p.place(0, &op, None, a).expect("ldunif");
        p.fence();
    }
    /// A flag push, alone in word `p.len()`.
    fn put_push<'a>(p: &mut Packer71<'a, '_>, a: &Arena<'a>) {
        p.fence();
        let imm = Src71::SmallImm(small_imm_index(0).expect("0"));
        let op =
            Op71::Flags { unit: FlagOp71::Add(AddOp71::Xor), a: Src71::V(V(0)), b: imm, pf: Pf::PushZ, uf: Uf::None };
        p.place(0, &op, None, a).expect("push");
        p.fence();
    }

    #[test]
    fn the_helpers_make_the_words_the_tests_mean() {
        with_packer(|p, a| {
            p.pad_to(1, a).unwrap();
            put_refused(p, a);
            put_push(p, a);
            assert_eq!(p.len(), 3);
            assert_eq!(verdicts(p.word(0).unwrap()), BOTH);
            assert_eq!(verdicts(p.word(1).unwrap()), NEITHER);
            assert_eq!(verdicts(p.word(2).unwrap()), (true, false));
        });
    }

    #[test]
    fn branch_back_moves_over_at_most_three_words() {
        with_packer(|p, a| {
            p.pad_to(5, a).unwrap();
            assert_eq!(branch_back(p, 5, 0, false), 2);
            assert_eq!(branch_back(p, 5, 0, true), 2);
        });
        /* fewer than three words: back to the program start */
        with_packer(|p, a| {
            p.pad_to(2, a).unwrap();
            assert_eq!(branch_back(p, 2, 0, false), 0);
        });
        /* an empty program: nowhere to go */
        with_packer(|p, _| assert_eq!(branch_back(p, 0, 0, false), 0));
    }

    #[test]
    fn branch_back_never_goes_before_lo() {
        with_packer(|p, a| {
            p.pad_to(8, a).unwrap();
            assert_eq!(branch_back(p, 8, 5, false), 5);
            assert_eq!(branch_back(p, 8, 6, false), 6);
            assert_eq!(branch_back(p, 8, 7, false), 7);
            assert_eq!(branch_back(p, 8, 8, false), 8);
            /* lo further back than three words: still three */
            assert_eq!(branch_back(p, 8, 1, false), 5);
            assert_eq!(branch_back(p, 8, 0, true), 5);
        });
    }

    #[test]
    fn branch_back_stops_at_a_word_that_may_not_be_in_a_slot() {
        /* word 3 is refused: the branch lands right after it */
        with_packer(|p, a| {
            p.pad_to(3, a).unwrap();
            put_refused(p, a);
            p.pad_to(6, a).unwrap();
            assert_eq!(branch_back(p, 6, 0, false), 4);
            assert_eq!(branch_back(p, 6, 0, true), 4);
        });
        /* the refused word is the last one: no slot is filled */
        with_packer(|p, a| {
            p.pad_to(4, a).unwrap();
            put_refused(p, a);
            assert_eq!(p.len(), 5);
            assert_eq!(branch_back(p, 5, 0, false), 5);
        });
        /* a branch word is not moved over either */
        with_packer(|p, a| {
            p.pad_to(2, a).unwrap();
            p.insert_branch(2, branch(BranchCond::Always), a).unwrap();
            assert_eq!(p.len(), 6);
            /* words 3-5 are its slots (NOPs), which may be moved over; the
             * branch itself at 2 is the stop */
            assert_eq!(branch_back(p, 6, 0, false), 3);
        });
    }

    #[test]
    fn branch_back_a_flag_push_stops_only_a_conditional_branch() {
        with_packer(|p, a| {
            p.pad_to(2, a).unwrap();
            put_push(p, a);
            p.pad_to(4, a).unwrap();
            /* words 0, 1 NOP, 2 the push, 3 NOP */
            assert_eq!(p.len(), 4);
            assert_eq!(branch_back(p, 4, 0, false), 1);
            assert_eq!(branch_back(p, 4, 0, true), 3);
        });
        with_packer(|p, a| {
            p.pad_to(1, a).unwrap();
            put_push(p, a);
            assert_eq!(branch_back(p, 2, 0, false), 0);
            assert_eq!(branch_back(p, 2, 0, true), 2);
        });
    }

    #[test]
    fn branch_back_leaves_a_branch_that_is_not_at_the_end() {
        with_packer(|p, a| {
            p.pad_to(6, a).unwrap();
            assert_eq!(branch_back(p, 4, 0, false), 4);
            assert_eq!(branch_back(p, 7, 0, false), 7);
        });
    }

    /// A `Cf` with a branch word at 3 (slots 4-6) and NOPs up to 12.
    fn with_cf<R>(f: impl FnOnce(&mut Cf<'_, '_>) -> R) -> R {
        store::with_scratch(|a| {
            let al = Allocation71 { reg: &[Rf(4)], rf_end: 5 };
            let mut cf = Cf { p: Packer71::new(&al, a).expect("packer"), last_target: None, last_branch: None };
            cf.p.pad_to(3, a).unwrap();
            cf.p.insert_branch(3, branch(BranchCond::AnyA), a).unwrap();
            cf.p.pad_to(12, a).unwrap();
            f(&mut cf)
        })
        .expect("scratch")
    }

    #[test]
    fn resolve_counts_the_offset_from_after_the_slots_and_the_stream_in_bytes() {
        with_cf(|cf| {
            let br = branch(BranchCond::AnyA);
            cf.resolve(3, 10, br).expect("resolves");
            let w = cf.p.word(3).unwrap();
            assert_eq!(w.branch.unwrap().offset, (10 - (3 + 4)) * 8);
            /* no uniform between the branch and its target but its own:
             * the stream word is (1 - (0 + 1)) * 4 */
            assert_eq!(w.uniform.unwrap().data, 0);
            /* a backward target: negative words in two's complement */
            cf.resolve(3, 1, br).expect("resolves");
            assert_eq!(cf.p.word(3).unwrap().branch.unwrap().offset, ((1i64 - 7) * 8) as u32);
        });
    }

    #[test]
    fn resolve_refuses_a_word_that_is_not_a_branch() {
        with_cf(|cf| {
            let br = branch(BranchCond::AnyA);
            assert_eq!(cf.resolve(2, 4, br), Err(Sched71Error::NotPlaceable(2)));
            /* past the last word */
            assert_eq!(cf.resolve(40, 4, br), Err(Sched71Error::NotPlaceable(40)));
        });
    }

    fn refusal(ops: &[Op71], vregs: u32) -> Option<Sched71Error> {
        store::with_scratch(|a| {
            let sel = Selected71 { ops, fragment: true, vregs };
            let al = Allocation71 { reg: &[Rf(4), Rf(5)][..vregs as usize], rf_end: 6 };
            schedule71(&sel, &al, a).err()
        })
        .expect("scratch")
    }

    #[test]
    fn a_branch_to_a_label_never_defined_is_control_flow() {
        /* label 1 is defined, label 0 (in range) never is */
        let to0 = Op71::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: Label(0) };
        let ld = Op71::Ldunif { dst: V(0), u: Uniform::constant(1) };
        assert_eq!(refusal(&[ld, to0, Op71::Label(Label(1))], 1), Some(Sched71Error::ControlFlow(1)));
    }

    #[test]
    fn a_back_edge_with_no_open_head_is_control_flow_at_the_edge() {
        let edge = Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None };
        assert_eq!(refusal(&[edge], 0), Some(Sched71Error::ControlFlow(0)));
        /* a head closed once: a second edge has none */
        assert_eq!(refusal(&[Op71::LoopHead, edge, edge], 0), Some(Sched71Error::ControlFlow(2)));
    }

    #[test]
    fn nested_loop_heads_left_open_name_the_outer_one() {
        let edge = Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None };
        /* the inner closes; the outer, operation 0, stays open */
        assert_eq!(refusal(&[Op71::LoopHead, Op71::LoopHead, edge], 0), Some(Sched71Error::ControlFlow(0)));
    }

    #[test]
    fn a_forward_conditional_branch_needs_a_flag_push_a_back_edge_does_not() {
        let cond = Op71::FwdBranch { cond: BranchCond::AnyA, msfign: Msfign::None, label: Label(0) };
        assert_eq!(refusal(&[cond, Op71::Label(Label(0))], 0), Some(Sched71Error::CondWithoutFlags(0)));
        /* an unconditional one needs none */
        let always = Op71::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: Label(0) };
        assert_eq!(refusal(&[always, Op71::Label(Label(0))], 0), None);
        /* a back edge only waits for the last push, if there is one */
        let edge = Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None };
        let ld = Op71::Ldunif { dst: V(0), u: Uniform::constant(1) };
        assert_eq!(refusal(&[Op71::LoopHead, ld, edge], 1), None);
    }

    #[test]
    fn a_loop_with_an_empty_body_is_control_flow_at_its_edge() {
        /* the branch cannot move before the word after the head, and with
         * no body that word does not exist: the edge's word would be the
         * head's, not after it */
        let edge = Op71::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::None };
        assert_eq!(refusal(&[Op71::LoopHead, edge], 0), Some(Sched71Error::ControlFlow(1)));
    }

    #[test]
    fn a_scratch_arena_too_small_is_out_of_scratch() {
        let mut buf = [0u8; 0];
        let a = Arena::new(&mut buf);
        let sel = Selected71 { ops: &[], fragment: true, vregs: 1 };
        let al = Allocation71 { reg: &[Rf(4)], rf_end: 5 };
        assert_eq!(schedule71(&sel, &al, &a).err(), Some(Sched71Error::OutOfScratch));
    }

    #[test]
    fn the_packers_error_inside_a_loop_passes_through() {
        /* v1 has no register: the packer's Unallocated, at its operation */
        let ops = [Op71::LoopHead, Op71::Ldunif { dst: V(1), u: Uniform::constant(1) }];
        assert_eq!(refusal(&ops, 1), Some(Sched71Error::Unallocated(1)));
    }
}
