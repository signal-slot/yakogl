// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// SPDX-FileCopyrightText: 2010 Intel Corporation
// Derived from Mesa 26.2.1 src/broadcom/compiler/qpu_schedule.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Operation order before scheduling: a list scheduler's choice of order,
//! made after register allocation so that every dependency is one between
//! physical registers, and handed to the in-order packer (sched.rs), which
//! keeps all of its own rules.
//!
//! The selected program is in the order selection made it: a long chain
//! (atan's polynomial, a rounded rectangle's distance) and the independent
//! work around it follow each other, and the packer, which places each
//! operation in the word of the one before it or later, cannot pair them.
//! Mesa's scheduler picks, each cycle, the ready instruction with the
//! longest path to the end (qpu_schedule.c choose_instruction_to_schedule,
//! its `delay`). This does the same on our operations, one region at a time
//! (a region ends at a thread switch, a loop head or back edge, a branch or
//! a label, which stay where they are): an order in which an add, a mul
//! and a load signal of independent work can share a word.
//!
//! What must not change order, and so is an edge of the graph:
//! - a register (by its physical name, `Allocation::reg`): a read after
//!   the write it reads (with that write's latency: two for an SFU result
//!   and for an ldvary's C term in r5), a write after the reads of the
//!   value before (same word allowed), a write after the write before;
//! - r5: an ldvary's C term and a plain ldunif (a load into r5) write it;
//! - the flags: a push, and each conditional write that reads it;
//! - rtop: MULTOP writes it, UMUL24 / SMUL24 read it;
//! - every operation with an effect outside the registers, in one chain:
//!   the TMU (its configuration, its writes, LDTMU), the TLB, the VPM,
//!   SETMSF and MSF.
//!
//! The pick follows Mesa's choose_instruction_to_schedule (qpu_schedule.c
//! :1400-1650, Mesa 26.2.1): the longest path first, and in a fragment
//! shader no uniform load while a varying is still to be loaded, unless
//! nothing else is ready (its ldvary pipelining, :1415-1425 and :1633).
//!
//! Uniform loads take no edge among themselves: the uniform stream is built
//! from the words' order afterwards (sched.rs), and a branch's or back
//! edge's stream offset from their positions, which never cross a region.

use super::alloc::{Allocation, Reg};
use super::isa::{AddOp, BranchCond, Cond, MulOp};
use super::select::{is_sfu, Dst, FlagOp, Operand, Selected, VOp, V};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};

#[cfg(test)]
thread_local! {
    /// A test's switch: the chain schedules the selected order only.
    pub(crate) static ORDER_OFF: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether the chain tries [`reorder`]'s order: always, except in a test
/// that switched it off.
pub fn on() -> bool {
    #[cfg(test)]
    {
        !ORDER_OFF.with(|h| h.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// A resource an operation reads or writes: a physical register, the flags,
/// rtop, or the chain of outside effects.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Res {
    Reg(Reg),
    Flags,
    Rtop,
    Side,
}

/// The unit an operation takes in a word, for the pick of a cycle.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Add,
    Mul,
    Sig,
}

/// Whether `op` ends a region (keeps its place, and nothing crosses it).
fn barrier(op: &VOp) -> bool {
    matches!(op, VOp::Thrsw | VOp::LoopHead | VOp::BackEdge { .. } | VOp::FwdBranch { .. } | VOp::Label(_))
}

fn unit(op: &VOp) -> Unit {
    match op {
        VOp::Add { .. } | VOp::Flags { unit: FlagOp::Add(_), .. } | VOp::Setmsf { .. } => Unit::Add,
        VOp::Ldunif { .. } | VOp::Ldvary { .. } | VOp::Ldtmu { .. } | VOp::Wrtmuc { .. } => Unit::Sig,
        _ => Unit::Mul,
    }
}

/// Whether `op` has an effect outside the registers (the side chain).
fn side(op: &VOp) -> bool {
    match *op {
        VOp::Wrtmuc { .. } | VOp::Ldtmu { .. } | VOp::Setmsf { .. } => true,
        VOp::Add { op, dst, .. } => {
            !matches!(dst, Dst::V(_))
                || matches!(op, AddOp::LdvpmvIn | AddOp::Msf | AddOp::Tmuwt | AddOp::Vpmwt | AddOp::Stvpmv)
        }
        VOp::Mul { dst, .. } => !matches!(dst, Dst::V(_)),
        _ => false,
    }
}

/// Up to four resources.
type Set = [Option<Res>; 4];

/// The resources `op` reads and writes.
fn access(op: &VOp, al: &Allocation<'_>) -> (Set, Set) {
    let mut r: Set = [None; 4];
    let mut w: Set = [None; 4];
    let reg = |v: V| Res::Reg(al.reg[v.0 as usize]);
    let mut nr = 0;
    for o in op.uses() {
        let x = match o {
            Operand::V(v) | Operand::Abs(v) | Operand::Lo(v) | Operand::Hi(v) => reg(v),
            Operand::R5 => Res::Reg(Reg::Acc(5)),
            _ => continue,
        };
        r[nr] = Some(x);
        nr += 1;
    }
    let reads_flags = match *op {
        VOp::CondMov { .. } => true,
        VOp::Setmsf { cond, .. } => cond != Cond::None,
        _ => false,
    };
    if reads_flags {
        r[nr] = Some(Res::Flags);
        nr += 1;
    }
    if matches!(op, VOp::Mul { op: MulOp::Umul24 | MulOp::Smul24, .. }) {
        r[nr] = Some(Res::Rtop);
    }
    let mut nw = 0;
    let written = match *op {
        VOp::CondMov { dst, .. } | VOp::Copy { dst, .. } => Some(dst),
        _ => op.def(),
    };
    if let Some(v) = written {
        w[nw] = Some(reg(v));
        nw += 1;
    }
    if matches!(op, VOp::Ldvary { .. }) {
        w[nw] = Some(Res::Reg(Reg::Acc(5)));
        nw += 1;
    }
    if matches!(op, VOp::Flags { .. }) {
        w[nw] = Some(Res::Flags);
        nw += 1;
    }
    if matches!(op, VOp::Mul { op: MulOp::Multop, .. }) {
        w[nw] = Some(Res::Rtop);
        nw += 1;
    }
    if side(op) {
        w[nw] = Some(Res::Side);
    }
    (r, w)
}

/// How many words after `op` its result `res` is readable: an SFU's two, an
/// ldvary's C term (r5) two, anything else one.
fn latency(op: &VOp, res: Res) -> u32 {
    match *op {
        VOp::Add { op, .. } if is_sfu(op) => 2,
        VOp::Ldvary { .. } if res == Res::Reg(Reg::Acc(5)) => 2,
        _ => 1,
    }
}

/// `sel`'s operations in a list scheduler's order, region by region.
pub fn reorder<'a>(sel: &Selected<'_>, al: &Allocation<'_>, a: &Arena<'a>) -> Result<&'a [VOp], ArenaError> {
    let ops = sel.ops;
    let mut out: AVec<'a, VOp> = AVec::with_capacity(a, ops.len())?;
    let mut start = 0;
    while start < ops.len() {
        let end = (start..ops.len()).find(|&i| barrier(&ops[i])).unwrap_or(ops.len());
        /* a conditional branch reads the region's last flag push, and
         * the words after that push can be its delay slots */
        let conditional = ops.get(end).is_some_and(|op| match *op {
            VOp::BackEdge { cond, .. } | VOp::FwdBranch { cond, .. } => cond != BranchCond::Always,
            _ => false,
        });
        region(&ops[start..end], al, a, conditional, &mut out)?;
        if end < ops.len() {
            out.push(a, ops[end])?;
        }
        start = end + 1;
    }
    Ok(out.into_slice())
}

/// A resource's row in the per-resource tables: r0..r5, rf0..rf255 (the
/// field is a byte), the flags, rtop, the chain of outside effects.
const SLOTS: usize = 6 + 256 + 3;

fn slot(r: Res) -> usize {
    match r {
        Res::Reg(Reg::Acc(n)) => usize::from(n).min(5),
        Res::Reg(Reg::Rf(n)) => 6 + usize::from(n),
        Res::Flags => 6 + 256,
        Res::Rtop => 6 + 256 + 1,
        Res::Side => 6 + 256 + 2,
    }
}

const NONE: u32 = u32::MAX;

/// A branch's delay slots (sched.rs branch_back).
const BRANCH_SLOTS: i32 = 3;

/// One region, into `out`.
///
/// The dependencies are built as a list scheduler builds them (Mesa's
/// qpu_schedule.c calculate_forward_deps: per resource, the last write and
/// the reads since): a read depends on the resource's last write, a write
/// on that write and on every read since. So the edges are O(n) in the
/// region's operations and so is the memory, where every dependent PAIR
/// made an edge before (26eca726: 2.99 MB of scratch and 12x the compile
/// time for dEQP-GLES2 uniform_api's nested_structs_arrays mat4/mat2 pair,
/// "Compile failed" past the 4 MiB scratch on larger ones).
///
/// An edge is (from, to, weight, soft), from < to. A soft edge is an
/// ldvary's write of r5 after the read of the previous C term: the ldvary
/// may issue one word before that read (its r5 write lands two words
/// later), so it is picked up to one cycle early and emitted right after
/// the read, where the packer lifts it back (sched.rs ldvary_back; Mesa's
/// fixup_pipelined_ldvary, qpu_schedule.c).
fn region<'a>(
    ops: &[VOp],
    al: &Allocation<'_>,
    a: &Arena<'a>,
    conditional: bool,
    out: &mut AVec<'a, VOp>,
) -> Result<(), ArenaError> {
    let n = ops.len();
    if n < 2 {
        return out.extend_from_slice(a, ops);
    }
    let r5 = slot(Res::Reg(Reg::Acc(5)));
    let mut edges: AVec<'a, (u32, u32, i32, bool)> = AVec::with_capacity(a, 4 * n)?;
    let last_write = a.alloc_filled(SLOTS, NONE)?;
    /* the reads of each resource since its last write, a list through
     * node_next (at most four reads an operation) */
    let head = a.alloc_filled(SLOTS, NONE)?;
    let node_op = a.alloc_filled(4 * n, 0u32)?;
    let node_next = a.alloc_filled(4 * n, NONE)?;
    let mut nodes = 0usize;
    for j in 0..n {
        let (rj, wj) = access(&ops[j], al);
        let writes = |x: usize| wj.iter().flatten().any(|&y| slot(y) == x);
        /* read after write, with the write's latency */
        for &r in rj.iter().flatten() {
            let w = last_write[slot(r)];
            if w != NONE {
                edges.push(a, (w, j as u32, latency(&ops[w as usize], r) as i32, false))?;
            }
        }
        for &r in wj.iter().flatten() {
            let x = slot(r);
            /* write after write */
            if last_write[x] != NONE {
                edges.push(a, (last_write[x], j as u32, 1, false))?;
            }
            /* write after read: the same word reads first; an ldvary's r5
             * after a C term's read may go a word early (soft) */
            let mut k = head[x];
            while k != NONE {
                let i = node_op[k as usize];
                if i != j as u32 {
                    let soft = x == r5
                        && matches!(ops[j], VOp::Ldvary { .. })
                        && ops[i as usize].uses().contains(&Operand::R5);
                    edges.push(a, (i, j as u32, if soft { -1 } else { 0 }, soft))?;
                }
                k = node_next[k as usize];
            }
            last_write[x] = j as u32;
            head[x] = NONE;
        }
        /* the reads this operation makes of values it does not replace */
        for &r in rj.iter().flatten() {
            let x = slot(r);
            if !writes(x) {
                node_op[nodes] = j as u32;
                node_next[nodes] = head[x];
                head[x] = nodes as u32;
                nodes += 1;
            }
        }
    }
    let e = edges.as_slice();
    /* out- and in-edges, CSR */
    let csr = |key: fn(&(u32, u32, i32, bool)) -> u32| -> Result<(&'a [u32], &'a [u32]), ArenaError> {
        let start = a.alloc_filled(n + 1, 0u32)?;
        for x in e {
            start[key(x) as usize + 1] += 1;
        }
        for k in 0..n {
            start[k + 1] += start[k];
        }
        let list = a.alloc_filled(e.len(), 0u32)?;
        let fill = a.alloc_filled(n, 0u32)?;
        fill.copy_from_slice(&start[..n]);
        for (ei, x) in e.iter().enumerate() {
            let k = key(x) as usize;
            list[fill[k] as usize] = ei as u32;
            fill[k] += 1;
        }
        Ok((start, list))
    };
    let (out_start, out_list) = csr(|x| x.0)?;
    let (in_start, in_list) = csr(|x| x.1)?;
    let outs = |k: usize| out_list[out_start[k] as usize..out_start[k + 1] as usize].iter().map(|&ei| e[ei as usize]);
    let ins = |k: usize| in_list[in_start[k] as usize..in_start[k + 1] as usize].iter().map(|&ei| e[ei as usize]);
    /* height: the longest weighted path to the region's end */
    let height = a.alloc_filled(n, 1i32)?;
    /* Before a conditional branch, the flag push it reads has the branch's
     * three delay slots after it: work picked after the push can run there
     * (sched.rs branch_back, Mesa's emit_branch), work picked before it
     * cannot, and the slots then hold NOPs. So the push counts as a path of
     * 1 + 3 to the end, and the operations it waits for (a counter's `sub`)
     * rank above work that could fill the slots. Mesa has no such weight:
     * its back edge is unconditional (the loop test sits at the top). Step
     * (m): es100 loopcarry's body chain outranked the counter, the push
     * came last and the three slots stayed NOPs once its uniform was loaded
     * before the loop. */
    if conditional {
        if let Some(p) = ops.iter().rposition(|op| matches!(op, VOp::Flags { .. })) {
            height[p] = 1 + BRANCH_SLOTS;
        }
    }
    for i in (0..n).rev() {
        let h = outs(i).map(|x| x.2.max(if x.3 { -1 } else { 1 }) + height[x.1 as usize]).max().unwrap_or(1);
        height[i] = height[i].max(h);
    }
    let preds = a.alloc_filled(n, 0u32)?;
    for x in e.iter().filter(|x| !x.3) {
        preds[x.1 as usize] += 1;
    }
    let earliest = a.alloc_filled(n, 0i32)?;
    /* 0: not picked, 1: picked and waiting for its soft predecessors'
     * emission, 2: emitted */
    let state = a.alloc_filled(n, 0u8)?;
    /* the operations whose hard predecessors are all picked, and the picked
     * ones not yet emitted */
    let mut ready_list: AVec<'a, u32> = AVec::with_capacity(a, n)?;
    for (k, _) in preds.iter().enumerate().filter(|(_, &p)| p == 0) {
        ready_list.push(a, k as u32)?;
    }
    let mut pending: AVec<'a, u32> = AVec::with_capacity(a, n)?;
    let mut left = n;
    let mut varyings_left = ops.iter().filter(|op| matches!(op, VOp::Ldvary { .. })).count();
    let mut cycle = 0i32;
    while left > 0 {
        let mut picked = [usize::MAX; 3];
        for (slot_no, u) in [Unit::Add, Unit::Mul, Unit::Sig].into_iter().enumerate() {
            let in_cycle = |k: usize| picked.contains(&k);
            /* a pending ldvary flushes this cycle when each of its soft
             * predecessors is emitted or picked in this cycle */
            let flushes = |l: usize| ins(l).filter(|x| x.3).all(|x| state[x.0 as usize] == 2 || in_cycle(x.0 as usize));
            let ready = |k: usize| {
                state[k] == 0
                    && preds[k] == 0
                    && earliest[k] <= cycle
                    && ins(k).all(|x| {
                        let p = x.0 as usize;
                        if x.3 {
                            /* a soft predecessor: readable by the next cycle */
                            state[p] != 0 || (preds[p] == 0 && earliest[p] <= cycle + 1)
                        } else {
                            state[p] != 1 || flushes(p)
                        }
                    })
            };
            let is_load = |k: usize| matches!(ops[k], VOp::Ldunif { .. });
            let cand = ready_list.as_slice().iter().map(|&k| k as usize);
            /* Mesa's ldvary pipelining: while a varying is still to be
             * loaded, a uniform load waits, unless nothing else is ready */
            let skip = varyings_left > 0 && cand.clone().any(|k| ready(k) && !is_load(k));
            picked[slot_no] = cand
                .filter(|&k| unit(&ops[k]) == u && !(skip && is_load(k)) && ready(k))
                .max_by_key(|&k| (height[k], core::cmp::Reverse(k)))
                .unwrap_or(usize::MAX);
        }
        for &k in picked.iter().filter(|&&k| k != usize::MAX) {
            state[k] = 1;
            left -= 1;
            pending.push(a, k as u32)?;
            if matches!(ops[k], VOp::Ldvary { .. }) {
                varyings_left -= 1;
            }
            for x in outs(k).filter(|x| !x.3) {
                let t = x.1 as usize;
                preds[t] -= 1;
                earliest[t] = earliest[t].max(cycle + x.2);
                if preds[t] == 0 {
                    ready_list.push(a, t as u32)?;
                }
            }
        }
        /* the ready list keeps what is not picked yet */
        let keep = {
            let l = ready_list.as_mut_slice();
            let mut w = 0;
            for r in 0..l.len() {
                if state[l[r] as usize] == 0 {
                    l[w] = l[r];
                    w += 1;
                }
            }
            w
        };
        while ready_list.len() > keep {
            ready_list.pop();
        }
        /* emit, in program order, this cycle's picks and every pending
         * ldvary whose soft predecessors are now emitted or emitted with
         * them: every edge still points forward for the packer */
        let pl = pending.as_mut_slice();
        pl.sort_unstable();
        let mut w = 0;
        for r in 0..pl.len() {
            let k = pl[r] as usize;
            if ins(k).filter(|x| x.3).all(|x| state[x.0 as usize] != 0) {
                state[k] = 2;
                out.push(a, ops[k])?;
            } else {
                pl[w] = pl[r];
                w += 1;
            }
        }
        while pending.len() > w {
            pending.pop();
        }
        cycle += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;
    use crate::shaderc::online::v3d::alloc::RF_FIRST;
    use crate::shaderc::online::v3d::select::Uniform;
    use std::vec::Vec;

    /// `ops` with registers `regs` (indexed by virtual register), reordered.
    fn reordered(ops: &[VOp], regs: &[Reg]) -> Vec<VOp> {
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
            reorder(&sel, &al, a).expect("scratch").to_vec()
        })
        .expect("scratch")
    }

    /// Where each of `ops` went in `out`: a permutation, or a panic.
    fn places(ops: &[VOp], out: &[VOp]) -> Vec<usize> {
        assert_eq!(ops.len(), out.len(), "every operation once: {out:#?}");
        let mut taken = std::vec![false; out.len()];
        ops.iter()
            .map(|op| {
                let k = (0..out.len()).find(|&k| !taken[k] && out[k] == *op).expect("an operation lost");
                taken[k] = true;
                k
            })
            .collect()
    }

    fn fmul(d: u32, a: Operand, b: Operand) -> VOp {
        VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(d)), a, b }
    }
    fn fadd(d: u32, a: Operand, b: Operand) -> VOp {
        VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(d)), a, b }
    }
    fn v(n: u32) -> Operand {
        Operand::V(V(n))
    }

    /// Before a conditional back edge the flag push it reads comes before
    /// work of height 3 or less: work after the push can run in the branch's
    /// three delay slots (sched.rs branch_back), work before it cannot. Here
    /// a three-deep chain (FADD, FMUL, FADD) and the push (an FCMP) want the
    /// add unit at once; by height alone the chain's head (3) beats the push
    /// (1). Revert check: without the weight the push is not first.
    #[test]
    fn a_back_edges_flag_push_goes_before_work_its_slots_can_take() {
        use super::super::isa::{BranchCond, Msfign, Pf, Uf};
        let ops = [
            fadd(1, Operand::Payload(0), Operand::Payload(0)),
            fmul(2, v(1), v(1)),
            fadd(3, v(2), v(2)),
            VOp::Flags { unit: FlagOp::Add(AddOp::Fcmp), a: v(9), b: v(9), pf: Pf::PushC, uf: Uf::None },
            VOp::BackEdge { cond: BranchCond::AnyNA, msfign: Msfign::P },
        ];
        let mut regs = std::vec![Reg::Acc(0); 10];
        (regs[1], regs[2], regs[3], regs[9]) = (Reg::Acc(0), Reg::Acc(1), Reg::Acc(2), Reg::Rf(RF_FIRST));
        let out = reordered(&ops, &regs);
        assert!(matches!(out[0], VOp::Flags { .. }), "the push first: {out:#?}");
        assert!(matches!(out[4], VOp::BackEdge { .. }), "the back edge keeps its place: {out:#?}");
    }

    /// The scratch a region takes grows with its operations, not with
    /// their pairs: 1600 operations take about twice what 800 take (the
    /// ratio is pinned below 2.5). Before (26eca726), every dependent pair
    /// was an edge, dEQP-GLES2 uniform_api's larger programs ran the 4 MiB
    /// scratch out ("Compile failed" on the Pi 4, FBOCU/FBOCU2) and the
    /// compile took 12x its time. Revert check: an edge for every earlier
    /// operation runs the 4 MiB scratch out at 1600 (1.28 M edges): the
    /// test fails on OutOfScratch before the ratio is taken.
    #[test]
    fn a_regions_scratch_grows_with_its_operations_not_their_pairs() {
        let used = |n: u32| {
            /* each operation reads the payload and the value two before it,
             * writing one of 64 registers: RAW, WAR and WAW edges all */
            let ops: Vec<VOp> = (0..n)
                .map(|k| fadd(k, Operand::Payload(0), if k >= 2 { v(k - 2) } else { Operand::Payload(0) }))
                .collect();
            let regs: Vec<Reg> = (0..n).map(|k| Reg::Rf(RF_FIRST + (k % 64) as u8)).collect();
            store::with_scratch(|a| {
                let sel = Selected {
                    ops: &ops,
                    fragment: true,
                    vregs: n,
                    vattr_sizes: [0; 16],
                    vpm_output_rows: 0,
                    uses_vid: false,
                    uses_iid: false,
                    uses_center_w: false,
                    implicit_point_varyings: false,
                    writes_z: false,
                    disable_ez: false,
                };
                let al = Allocation { reg: &regs, rf_end: RF_FIRST + 64 };
                let before = a.used();
                let out = reorder(&sel, &al, a).expect("scratch");
                assert_eq!(out.len(), ops.len());
                a.used() - before
            })
            .expect("scratch")
        };
        let (small, large) = (used(800), used(1600));
        let ratio = large as f64 / small as f64;
        assert!(ratio < 2.5, "800 operations {small} bytes, 1600 {large}: x{ratio:.2}");
    }

    /// Four independent adds selected before a chain of four dependent    /// Four independent adds selected before a chain of four dependent
    /// adds, all on the add unit: the chain's head goes first (its path to
    /// the end is the longest) and the independent adds fill the chain's
    /// waits, and every dependency keeps its order. Revert check: picking
    /// the lowest index instead of the longest path keeps the selected
    /// order and fails the first assertion.
    #[test]
    fn the_longest_chain_starts_first() {
        let w = Operand::Payload(0);
        let ops = [
            fadd(0, w, Operand::SmallImm(1)),
            fadd(1, w, Operand::SmallImm(2)),
            fadd(2, w, Operand::SmallImm(3)),
            fadd(3, w, Operand::SmallImm(4)),
            fadd(4, w, w),
            fadd(5, v(4), v(4)),
            fadd(6, v(5), v(5)),
            fadd(7, v(6), v(6)),
        ];
        let regs: Vec<Reg> = (0..8).map(|k| Reg::Rf(RF_FIRST + k)).collect();
        let out = reordered(&ops, &regs);
        let at = places(&ops, &out);
        assert_eq!(at[4], 0, "the chain's head first: {out:#?}");
        for k in 4..7 {
            assert!(at[k] < at[k + 1], "the chain keeps its order: {out:#?}");
        }
    }

    /// The ldvary pipeline: a varying's C term (r5) is read before the next
    /// ldvary in the order handed to the packer (which lifts the ldvary
    /// back a word, sched.rs ldvary_back), however early the model picked
    /// it. Revert check: emitting an ldvary when it is picked puts the
    /// second ldvary before the first C term's read.
    #[test]
    fn a_c_term_is_read_before_the_next_ldvary() {
        let w = Operand::Payload(0);
        /* each C term read waits only for r5 (two cycles after its
         * ldvary), so the model may pick the next ldvary a cycle before it */
        let ops = [
            VOp::Ldvary { dst: V(0) },
            fmul(1, v(0), w),
            fadd(2, w, Operand::R5),
            VOp::Ldvary { dst: V(3) },
            fmul(4, v(3), w),
            fadd(5, w, Operand::R5),
            VOp::Ldvary { dst: V(6) },
            fmul(7, v(6), w),
            fadd(8, w, Operand::R5),
            fadd(9, v(2), v(5)),
            fadd(10, v(9), v(8)),
        ];
        let regs = [
            Reg::Acc(0),
            Reg::Acc(0),
            Reg::Rf(3),
            Reg::Acc(1),
            Reg::Acc(1),
            Reg::Rf(4),
            Reg::Acc(2),
            Reg::Acc(2),
            Reg::Rf(5),
            Reg::Rf(6),
            Reg::Rf(7),
        ];
        let out = reordered(&ops, &regs);
        let at = places(&ops, &out);
        assert!(at[2] < at[3] && at[5] < at[6], "each C term is read before the next ldvary: {out:#?}");
    }

    /// Nothing crosses a thread switch, and a uniform load waits while a
    /// varying is still to be loaded (Mesa's ldvary pipelining), though its
    /// path is the longest. Revert check: without the wait the load goes
    /// first.
    #[test]
    fn a_load_waits_for_the_varyings_and_nothing_crosses_a_switch() {
        let w = Operand::Payload(0);
        let ops = [
            VOp::Ldvary { dst: V(0) },
            fmul(1, v(0), w),
            fadd(2, v(1), Operand::R5),
            VOp::Ldunif { dst: V(3), u: Uniform::constant(5) },
            fmul(4, v(3), v(3)),
            fmul(5, v(4), v(4)),
            fmul(6, v(5), v(5)),
            fadd(7, v(6), v(2)),
            VOp::Thrsw,
            fadd(8, v(7), v(7)),
        ];
        let regs: Vec<Reg> = (0..9).map(|k| Reg::Rf(RF_FIRST + k)).collect();
        let out = reordered(&ops, &regs);
        let at = places(&ops, &out);
        assert!(at[0] < at[3], "the ldvary before the load: {out:#?}");
        assert_eq!((at[8], at[9]), (8, 9), "the switch and what follows it stay: {out:#?}");
    }
}
