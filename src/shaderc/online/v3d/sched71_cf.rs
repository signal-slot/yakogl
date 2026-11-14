// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Scheduling at V3D 7.1, the whole program: split at control flow, each
//! straight-line run packed by [`super::sched71::pack71`], the branches,
//! counted loops and their delay slots placed, and the uniform stream moved
//! with them, as the contract in [`super::vop71`]'s header says. A stub: it
//! refuses every program by name.

use super::alloc71::Allocation71;
use super::isa71::{BranchCond, BranchDest};
use super::sched71::Sched71Error;
use super::verify71::VerifyError;
use super::vop71::{Instr71, Selected71, Uniform, NOT_YET_71};
use crate::shaderc::online::arena::Arena;

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

/// Schedule `sel` with the registers of `al`.
pub fn schedule71<'a>(
    sel: &Selected71<'_>,
    al: &Allocation71<'_>,
    a: &Arena<'a>,
) -> Result<Scheduled71<'a>, Sched71Error> {
    let _ = (sel, al, a);
    Err(Sched71Error::NotYet(NOT_YET_71))
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

    #[test]
    fn refuses_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = Selected71 { ops: &[], fragment: true, vregs: 0 };
        let al = Allocation71 { reg: &[], rf_end: 0 };
        assert_eq!(schedule71(&sel, &al, &a).err(), Some(Sched71Error::NotYet(NOT_YET_71)));
    }
}
