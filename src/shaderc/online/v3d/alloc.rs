// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// SPDX-FileCopyrightText: 2010 Intel Corporation
// Derived from Mesa 26.2.1 src/broadcom/compiler/ (nir_to_vir.c, vir_register_allocate.c) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Register allocation for the selected program ([`super::select`]): a
//! linear scan over the operations in program order, which is also the order
//! the scheduler keeps ([`super::sched`] merges operations into words but
//! never reorders them, so "last read at operation i" and "last read in the
//! word operation i lands in" say the same thing).
//!
//! # The register file this cut allocates from
//!
//! A program runs 4 threads (4-way threadable in the shader record) when it
//! fits, else 2, as Mesa's thread loop goes (nir_to_vir.c:5160-5200: 4, then
//! 2, never 1). On V3D 4.2 a thread has 32 physical registers, rf0..rf31, at
//! 4 threads and 64 at 1 or 2 (vir_register_allocate.c:1428-1435, Mesa
//! 26.2.1, MIT), and the accumulators r0..r5 at any count. Of those this cut
//! uses:
//!
//! * r0..r4 for anything that may live in an accumulator. r5 is left out:
//!   `ldvary` writes its C term there (qpu_validate.c:193-200), and Mesa keeps
//!   r5 for `ldunif` alone (vir_register_allocate.c:1290-1311).
//! * rf3 up to the thread count's end for the rest. rf0..rf2 hold the fragment shader's payload (W,
//!   centroid W, Z; nir_to_vir.c:4853-4858) and are never handed out, in any
//!   stage, which costs three registers nobody needs yet.
//!
//! Mesa's class rules, each followed:
//!
//! * an `ldvpmv_in` result is physical (vir_register_allocate.c:1182-1196);
//! * an SFU result (`recip`, `rsqrt`, `exp`, `log`, `sin`) is physical
//!   (:1198-1210);
//! * a value live across a thread switch is physical, because a switch
//!   invalidates the accumulators (:1340-1347).
//!
//! There is no spilling to memory: a program that does not fit 2 threads is
//! [`AllocError::DoesNotFit`], by name, and `chain.rs` retries with the
//! selection levels that shorten live ranges (select.rs `Pressure`).

use super::isa::{AddOp, MulOp};
use super::sched::hoistable;
use super::select::{is_sfu, Dst, Operand, Selected, VOp};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};

/// A physical register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reg {
    /// Accumulator r0..r4.
    Acc(u8),
    /// Register-file register rf3..rf31.
    Rf(u8),
}

/// The first register-file register handed out.
pub const RF_FIRST: u8 = 3;
/// One past the last register-file register a thread has at 4 threads.
pub const RF_END_4_THREADS: u8 = 32;
/// One past the last register-file register a thread has at 1 or 2 threads.
pub const RF_END_2_THREADS: u8 = 64;

/// One past the last register-file register at `threads` threads; `None`
/// for a count the V3D does not run (1, 2 and 4 only).
pub const fn rf_end(threads: u32) -> Option<u8> {
    match threads {
        4 => Some(RF_END_4_THREADS),
        1 | 2 => Some(RF_END_2_THREADS),
        _ => None,
    }
}
/// Accumulators handed out: r0..r4.
pub const ACCS: u8 = 5;

/// Why allocation stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllocError {
    /// More values are live at one point than there are registers at the
    /// thread count asked for. No spilling exists yet.
    DoesNotFit,
    /// A thread count the V3D does not run (not 1, 2 or 4).
    Threads,
    /// An operation reads a virtual register nothing defined before it.
    UseBeforeDef,
    /// A virtual register is defined twice (selection hands each out once).
    DefinedTwice,
    OutOfScratch,
}

impl From<ArenaError> for AllocError {
    fn from(_: ArenaError) -> Self {
        AllocError::OutOfScratch
    }
}

/// Where each virtual register lives.
#[derive(Clone, Copy, Debug)]
pub struct Allocation<'a> {
    /// Indexed by virtual register.
    pub reg: &'a [Reg],
    /// One past the highest register-file register used (RF_FIRST when none).
    pub rf_end: u8,
}

const NONE: u32 = u32::MAX;

/// Allocate `sel`'s virtual registers.
pub fn allocate<'a>(sel: &Selected<'_>, threads: u32, a: &Arena<'a>) -> Result<Allocation<'a>, AllocError> {
    let end = rf_end(threads).ok_or(AllocError::Threads)?;
    let n = sel.vregs as usize;
    let def = a.alloc_filled(n, NONE)?;
    let last = a.alloc_filled(n, NONE)?;
    let phys = a.alloc_filled(n, false)?;
    for (i, op) in sel.ops.iter().enumerate() {
        for o in op.uses() {
            if let Some(v) = o.vreg() {
                let v = v.0 as usize;
                if def.get(v).copied().unwrap_or(NONE) == NONE {
                    return Err(AllocError::UseBeforeDef);
                }
                last[v] = i as u32;
            }
        }
        if let Some(v) = op.def() {
            let v = v.0 as usize;
            if def.get(v).copied().ok_or(AllocError::UseBeforeDef)? != NONE {
                return Err(AllocError::DefinedTwice);
            }
            def[v] = i as u32;
            last[v] = i as u32;
            if let VOp::Add { op, .. } = op {
                if *op == AddOp::LdvpmvIn || is_sfu(*op) {
                    phys[v] = true;
                }
            }
        }
    }
    /* A counted loop (select.rs VOp::LoopHead .. VOp::BackEdge): a value
     * defined before the head and read inside the body is read again on
     * every pass, so it lives to the back edge; a value carried round the
     * loop (a phi, the counter) is defined before the head and written by a
     * Copy at the end, which this also covers. Before the thread-switch
     * rule, so a value this extends across a switch in the body is kept out
     * of the accumulators. */
    /* Loops nest: the innermost open head is the one a back edge closes.
     * An inner loop is closed first, so a value it extends to its back
     * edge is then extended to the outer one's by the same rule. */
    let mut heads: AVec<'a, usize> = AVec::new();
    for (e, op) in sel.ops.iter().enumerate() {
        match op {
            VOp::LoopHead => heads.push(a, e)?,
            VOp::BackEdge { .. } => {
                let h = heads.pop().ok_or(AllocError::UseBeforeDef)?;
                for v in 0..n {
                    if def[v] != NONE && (def[v] as usize) < h && (last[v] as usize) >= h && (last[v] as usize) < e {
                        last[v] = e as u32;
                    }
                }
            }
            _ => {}
        }
    }
    /* Live across a thread switch: defined before it, read after it. */
    for (t, op) in sel.ops.iter().enumerate() {
        if *op == VOp::Thrsw {
            for v in 0..n {
                if def[v] != NONE && (def[v] as usize) < t && (last[v] as usize) > t {
                    phys[v] = true;
                }
            }
        }
    }

    /* A plain MOV `v <- x` that is x's last read, where v must be in the
     * register file (it lives across a thread switch: a loop's carried
     * value, select.rs SOp::Phi's initial `mov phi, x`), puts x in the
     * register file too, so the MOV can take x's register below (the
     * MOV-source rule) and the scheduler drops it as `mov rfN, rfN`: the
     * value is written where it is kept, as Mesa's out-of-SSA coalesces a
     * copy's source with its destination when they do not interfere
     * (nir_from_ssa.c aggressive_coalesce_parallel_copy :403-440, phis
     * :385-401, Mesa 26.2.1). Otherwise x takes an accumulator and the MOV
     * stays: femtovg filter_image's five loop-carried sums cost five
     * words before the loop (step (m)). Backwards, so a chain of such MOVs
     * passes it on. Not to a uniform load: in the register file it is an
     * `ldunifrf`, whose address takes the condition field (Mesa keeps r5
     * for ldunif so that "it keeps the QPU cond field free from being
     * occupied by ldunifrf", vir_register_allocate.c v3d_ra_select_accum),
     * so it cannot share a word with a flag push or a conditional move,
     * where `ldunif` + MOV cost nothing (es300 idxtex's select chains
     * 31 -> 32 words with it). */
    let loaded = a.alloc_filled(n, false)?;
    for op in sel.ops {
        if let VOp::Ldunif { dst, .. } = *op {
            loaded[dst.0 as usize] = true;
        }
    }
    for (i, op) in sel.ops.iter().enumerate().rev() {
        if let VOp::Mul { op: MulOp::Mov, dst: Dst::V(v), a: Operand::V(x), b: Operand::None } = *op {
            let x = x.0 as usize;
            if phys[v.0 as usize] && last[x] == i as u32 && !loaded[x] {
                phys[x] = true;
            }
        }
    }

    /* A Copy `m <- e` that is e's last read (a uniform branch's else value
     * into its merge register, select.rs SOp::IfEnd; a loop's carried value
     * at the end of a pass) can write e straight into m's register when e
     * is defined after m and nothing reads or writes m between e's
     * definition and the Copy: m's register then holds nothing anyone reads
     * until the Copy (on a branch's else path the then arm's definition of
     * m did not run), and the Copy becomes `mov rX, rX`, which the
     * scheduler drops. */
    let into = a.alloc_filled(n, NONE)?;
    for (ci, op) in sel.ops.iter().enumerate() {
        let VOp::Copy { dst: m, src: Operand::V(e) } = *op else { continue };
        let (m, e) = (m.0 as usize, e.0 as usize);
        if last[e] as usize != ci || def[e] == NONE || def[m] == NONE || def[e] <= def[m] {
            continue;
        }
        let between = &sel.ops[def[e] as usize + 1..ci];
        if !between.iter().any(|o| o.uses().iter().any(|u| u.vreg() == Some(super::select::V(m as u32)))) {
            into[e] = m as u32;
        }
    }
    let (hole_start, need_from) = holes(sel, def, into, a)?;
    let reg = a.alloc_filled(n, Reg::Acc(0))?;
    /* Per register, the operation index of the last read of the value in it
     * (NONE: never held anything). */
    let mut acc_busy = [NONE; ACCS as usize];
    let mut rf_busy = [NONE; RF_END_2_THREADS as usize];
    /* Per register, the value that holds it until its `busy` read (NONE:
     * none), and the last read of the value lent it inside that value's
     * hole (NONE: none). */
    let mut acc_owner = [NONE; ACCS as usize];
    let mut rf_owner = [NONE; RF_END_2_THREADS as usize];
    let mut acc_lent = [NONE; ACCS as usize];
    let mut rf_lent = [NONE; RF_END_2_THREADS as usize];
    let mut rf_end = RF_FIRST;
    /* A load's accumulator (ldvary, a uniform load) is taken round-robin,
     * from the one after the last taken (Mesa's v3d_ra_select_accum takes
     * every accumulator so, "to give post-RA instruction selection more
     * options", vir_register_allocate.c:936-947). A load then seldom lands in
     * the register the value before it was just read from, which is what
     * lets the scheduler move an ldvary into the word before (sched.rs
     * ldvary_back) and a uniform load into an earlier free signal slot
     * (sched.rs hoist): back to back loads into one register cannot pass
     * each other. Every other value keeps the lowest free one, which often
     * gives a move its source's register (the dropped `mov rX, rX`):
     * round-robin for all measured worse (static 6850 against 6833,
     * executed 16382 against 16360, 2026-09-26). Loads too: executed
     * 15182 -> 15168, vertex shaders 3860 -> 3561. A load that goes to the
     * register file takes it round-robin too (below); every other value
     * there stays lowest-first. */
    let mut next_acc = 0u8;
    let mut next_rf = RF_FIRST;
    /* r5, for a uniform load (Mesa's v3d_ra_select_accum takes it first for
     * one, vir_register_allocate.c: "nobody else can load to that reg, and
     * it keeps the QPU cond field free from being occupied by ldunifrf"):
     * the scheduler then writes a plain `ldunif`, which names no register
     * and so can share a word with a conditional move or a flag push. Only
     * where nothing else touches r5 while the value lives: no `ldvary` (its C
     * term lands there), no read of that C term, and no thread switch (phys
     * marks those). r5 holds one such value at a time, so only a value read
     * within the next R5_SPAN operations takes it, and only in a fragment
     * shader: a vertex shader's short loads (the VPM row numbers before each
     * STVPMV) then queued behind the one r5 where round-robin accumulators
     * let them move early (vertex shaders 3551 -> 3724), and it has few
     * conditional writes to gain the field for. Measured 2026-09-26: fs
     * 6600 -> 6502, executed 15062 -> 14922; span 1, 2, 4, 8 and 64 within
     * 10 of each other, 2 the least. Nor does a load within R5_AFTER
     * operations after an ldvary or a read of its C term: the prologue's
     * first loads then went into r5 and waited two words behind the last
     * ldvary, where an ldunifrf into an accumulator shares the varyings'
     * words (fill_gradient.nodiscard 96 -> 98 until this; 2, 4, 8 and 16
     * measured, 4 the least: executed 14874 -> 14836). */
    let mut r5_busy = NONE;
    const R5_SPAN: usize = 2;
    const R5_AFTER: usize = 4;
    let touches_r5 = |o: &VOp| matches!(o, VOp::Ldvary { .. }) || o.uses().contains(&Operand::R5);
    /* The hoist-aware choice (below, `stop`) needs three facts, kept as the
     * scan goes. `barrier`: the last operation hoist never moves anything
     * over (sched.rs schedule's barrier: a thread switch, a branch or back
     * edge, a loop head, a label). `early`: per value, the earliest
     * operation it could be made after: its own operation when hoist cannot
     * move it (sched.rs hoistable), else one after each operand's earliest,
     * and not before the barrier (an operand that hoist moves may itself
     * land above operations before it). `*_touch`: per register, the last
     * operation before this one that read or wrote it, and the value when
     * that was a definition (NONE for a read). */
    let mut barrier = 0u32;
    let early = a.alloc_filled(n, 0u32)?;
    let mut acc_touch = [(NONE, NONE); ACCS as usize + 1];
    let mut rf_touch = [(NONE, NONE); RF_END_2_THREADS as usize];
    for (i, op) in sel.ops.iter().enumerate() {
        if let Some(p) = i.checked_sub(1) {
            let before = &sel.ops[p];
            let reads = before.uses().into_iter().filter_map(|u| u.vreg()).map(|x| (x, NONE));
            for (x, by) in reads.chain(before.def().map(|d| (d, d.0))) {
                match reg[x.0 as usize] {
                    Reg::Acc(r) => acc_touch[r as usize] = (p as u32, by),
                    Reg::Rf(r) => rf_touch[r as usize] = (p as u32, by),
                }
            }
        }
        if matches!(op, VOp::Thrsw | VOp::FwdBranch { .. } | VOp::BackEdge { .. } | VOp::LoopHead | VOp::Label(_)) {
            barrier = i as u32;
        }
        let Some(v) = op.def() else { continue };
        let v = v.0 as usize;
        let earliest = if hoistable(op).is_some() {
            let after_operands = op.uses().into_iter().filter_map(|u| u.vreg()).map(|u| early[u.0 as usize] + 1);
            after_operands.fold(barrier, u32::max).min(i as u32)
        } else {
            i as u32
        };
        early[v] = earliest;
        /* A register whose value is last read by this very operation, or
         * earlier, is free: an instruction reads its operands before it
         * writes, and the scheduler never moves a later write before an
         * earlier read. */
        let free = |busy: u32| busy == NONE || busy <= i as u32;
        /* A register is free for this value when its holder's last read is
         * past, or when this value lives inside the holder's hole: defined
         * at or after the hole's start, last read no later than where the
         * holder needs the register again, and no other value lent it
         * still read. */
        let lendable = |owner: u32| {
            owner != NONE
                && hole_start[owner as usize] != NONE
                && hole_start[owner as usize] <= i as u32
                && last[v] <= need_from[owner as usize]
        };
        /* m's register is m's until its last read, past this value's */
        if into[v] != NONE {
            let r = reg[into[v] as usize];
            if !phys[v] || matches!(r, Reg::Rf(_)) {
                reg[v] = r;
                continue;
            }
        }
        if matches!(op, VOp::Ldunif { .. })
            && sel.fragment
            && !phys[v]
            && free(r5_busy)
            && (last[v] as usize) <= i + R5_SPAN
            && !sel.ops[i + 1..=(last[v] as usize).max(i)].iter().any(touches_r5)
            && !sel.ops[i.saturating_sub(R5_AFTER)..i].iter().any(touches_r5)
        {
            r5_busy = last[v];
            reg[v] = Reg::Acc(5);
            continue;
        }
        let is_load = matches!(op, VOp::Ldvary { .. } | VOp::Ldunif { .. });
        let start = if is_load { next_acc } else { 0 };
        /* First a register free in the plain sense, as a scan without holes
         * takes it; only when there is none, one lent inside its holder's
         * hole. So a program that fits without lending keeps its registers:
         * lending first moved the packing of five loop programs with no
         * look-up (deadloop executed 161 -> 176, 2026-09-26). */
        /* Hoist-aware choice. sched.rs hoist moves a value back to a word p
         * only when no word from p on reads or writes the value's register
         * (sched.rs hoist, the register rules), never before its operands
         * are made and never over the barrier. So a value hoist may move can
         * go back as far as `earliest` in a register nobody touched since,
         * but only to the last touch in one touched later: `stop` is that
         * point, and among the free registers of the class chosen below the
         * one with the earliest stop is taken, the existing order (lowest
         * first, round-robin for a load) breaking ties. A touch that is the
         * definition of this operation's own operand never stops it (the
         * value cannot go above its operand anyway), so an operand's
         * register is as good as an untouched one when nothing else read it
         * since. A value hoist cannot move has no stop: the order alone. The
         * class is not changed for it: a register-file entry in place of an
         * accumulator costs a read port, and Mesa keeps the accumulators for
         * short values (vir_register_allocate.c:881-917). Mesa has no such
         * rule: its list scheduler orders by the dependency graph, and its
         * allocator spreads reuse round-robin "to give post-RA instruction
         * selection more options" (:936-947, :990-1003); this scheduler
         * keeps program order and moves one operation back, so the register
         * fact it needs is known here. Measured 2026-09-29 against 5fff6d69
         * (step (l)): fs 5928 -> 5723, vs 3522 -> 3287, cs 2804 -> 2623,
         * femtovg executed 5486 -> 5311, no pair or run longer in either
         * build; without the operand-definition clause fill_gradient and
         * five others were 1-2 words longer, with the operation's own index
         * as `earliest` (no chain through hoisted operands) main.vert's
         * three `recip rf7` stayed serial. */
        let hoists = hoistable(op).is_some();
        let stop = |(touch, by): (u32, u32)| {
            if !hoists {
                0
            } else if touch == NONE || (by != NONE && op.uses().iter().any(|u| u.vreg().is_some_and(|x| x.0 == by))) {
                earliest
            } else {
                touch.max(earliest)
            }
        };
        let acc_ok = |r: u8, lend: bool| {
            let r = r as usize;
            if lend {
                lendable(acc_owner[r]) && free(acc_lent[r])
            } else {
                free(acc_busy[r])
            }
        };
        let rf_ok = |r: u8, lend: bool| {
            let r = r as usize;
            if lend {
                lendable(rf_owner[r]) && free(rf_lent[r])
            } else {
                free(rf_busy[r])
            }
        };
        let acc_pick = |lend: bool| {
            if phys[v] {
                None
            } else {
                let order = (0..ACCS).map(|k| (start + k) % ACCS);
                order.filter(|&r| acc_ok(r, lend)).min_by_key(|&r| if lend { 0 } else { stop(acc_touch[r as usize]) })
            }
        };
        /* A load takes the register-file entry after the last one a load
         * took (round-robin, as its accumulator, and as Mesa's
         * v3d_ra_select_rf does with next_phys), anywhere below this thread
         * count's limit: a linear scan fits when no more values are live at
         * once than registers exist, whichever free register each takes, so
         * this fails exactly when lowest-first fails and the thread count
         * cannot change (chain.rs pins the two-thread stages). */
        let rf_pick = |lend: bool| {
            let first = if is_load { next_rf.max(RF_FIRST) - RF_FIRST } else { 0 };
            let order = (0..end - RF_FIRST).map(|k| RF_FIRST + (first + k) % (end - RF_FIRST));
            order.filter(|&r| rf_ok(r, lend)).min_by_key(|&r| if lend { 0 } else { stop(rf_touch[r as usize]) })
        };
        /* A plain move that is its source's last read takes the source's
         * register when the source holds it in the plain sense (not lent
         * inside another value's hole): the move is then `mov rX, rX`, which
         * the scheduler drops (sched.rs). It is select.rs's select: the else
         * value moved into the register a conditional move then writes.
         * Lowest-first gave this only when the source's register was the
         * lowest free one: once a look-up's words were read through L/H by
         * other readers (step (h)), they held r0/r1 longer, and
         * texture_copy_unclipped's texType selects kept real MOVs. Not r5:
         * only a short uniform load takes it (above). */
        let same = match *op {
            VOp::Mul { op: MulOp::Mov, a: Operand::V(x), b: Operand::None, .. } if last[x.0 as usize] == i as u32 => {
                match reg[x.0 as usize] {
                    Reg::Acc(r) if r < ACCS && !phys[v] && free(acc_busy[r as usize]) => Some(Reg::Acc(r)),
                    Reg::Rf(r) if free(rf_busy[r as usize]) => Some(Reg::Rf(r)),
                    _ => None,
                }
            }
            _ => None,
        };
        /* (register, lent) */
        let pick = match (same, acc_pick(false), rf_pick(false)) {
            (Some(r), _, _) => (r, false),
            (None, Some(r), _) => (Reg::Acc(r), false),
            (None, None, Some(r)) => (Reg::Rf(r), false),
            (None, None, None) => match (acc_pick(true), rf_pick(true)) {
                (Some(r), _) => (Reg::Acc(r), true),
                (None, Some(r)) => (Reg::Rf(r), true),
                (None, None) => return Err(AllocError::DoesNotFit),
            },
        };
        let r = match pick {
            (Reg::Acc(r), lent) => {
                next_acc = (r + 1) % ACCS;
                let k = r as usize;
                if lent {
                    acc_lent[k] = last[v];
                } else {
                    (acc_busy[k], acc_owner[k], acc_lent[k]) = (last[v], v as u32, NONE);
                }
                Reg::Acc(r)
            }
            (Reg::Rf(r), lent) => {
                if is_load {
                    next_rf = r + 1;
                }
                let k = r as usize;
                if lent {
                    rf_lent[k] = last[v];
                } else {
                    (rf_busy[k], rf_owner[k], rf_lent[k]) = (last[v], v as u32, NONE);
                }
                rf_end = rf_end.max(r + 1);
                Reg::Rf(r)
            }
        };
        reg[v] = r;
    }
    Ok(Allocation { reg, rf_end })
}

/// Lifetime holes. A value `m` that a [`VOp::Copy`] rewrites (a uniform
/// branch's merge register at the end of its else arm, a loop's carried
/// value at the end of a pass) holds nothing anyone reads between the
/// point from which no path reaches a read of `m` before that Copy, and
/// the Copy. For a merge register that is the whole else arm: its then
/// arm writes it and jumps over the else arm. A linear scan keeps the
/// register busy from the definition to the last read; inside the hole
/// it can lend the register to a value that lives there only.
///
/// Per value, `(hole_start, need_from)`: the first operation of the hole,
/// and the operation from which the register is `m`'s again (the Copy, or
/// the definition of a value written straight into it, `into`). NONE: no
/// hole. The hole is found scanning back from the Copy: it starts after
/// the nearest read of `m` (a use: a Copy or CondMov into `m` reads it
/// too), or after a forward branch to a label past the Copy (a path that
/// skips the Copy and reads `m` after it); a counted loop round that
/// point moves it after the loop's back edge, which leads back to it.
/// Only a value with exactly one Copy into it has a hole.
fn holes<'a>(
    sel: &Selected<'_>,
    def: &[u32],
    into: &[u32],
    a: &Arena<'a>,
) -> Result<(&'a mut [u32], &'a mut [u32]), AllocError> {
    let n = def.len();
    let ops = sel.ops;
    let hole_start = a.alloc_filled(n, NONE)?;
    let need_from = a.alloc_filled(n, NONE)?;
    let labels = ops.iter().filter_map(|o| if let VOp::Label(l) = o { Some(*l as usize + 1) } else { None }).max();
    let label_at = a.alloc_filled(labels.unwrap_or(0), NONE)?;
    let head_of = a.alloc_filled(ops.len(), NONE)?;
    let copies = a.alloc_filled(n, 0u32)?;
    let copy_at = a.alloc_filled(n, NONE)?;
    let mut heads: AVec<'a, usize> = AVec::new();
    for (i, op) in ops.iter().enumerate() {
        match *op {
            VOp::Label(l) => label_at[l as usize] = i as u32,
            VOp::LoopHead => heads.push(a, i)?,
            VOp::BackEdge { .. } => head_of[i] = heads.pop().ok_or(AllocError::UseBeforeDef)? as u32,
            VOp::Copy { dst, .. } => {
                let m = dst.0 as usize;
                copies[m] += 1;
                copy_at[m] = i as u32;
            }
            _ => {}
        }
    }
    for m in 0..n {
        let (d, ci) = (def[m], copy_at[m]);
        if copies[m] != 1 || d == NONE || ci <= d {
            continue;
        }
        let (d, ci) = (d as usize, ci as usize);
        let mut hs = d + 1;
        /* the back edges passed so far, scanning back: the latest one whose
         * loop holds the stopping point decides */
        let mut outer_edge: Option<usize> = None;
        for j in (d + 1..ci).rev() {
            let op = &ops[j];
            let reads = op.uses().iter().any(|u| u.vreg() == Some(super::select::V(m as u32)));
            let skips = matches!(*op, VOp::FwdBranch { label, .. }
                if label_at.get(label as usize).is_none_or(|&at| at == NONE || at as usize > ci));
            if reads || skips {
                hs = j + 1;
                for b in j + 1..ci {
                    if matches!(ops[b], VOp::BackEdge { .. }) && (head_of[b] as usize) < j {
                        outer_edge = Some(outer_edge.map_or(b, |e: usize| e.max(b)));
                    }
                }
                break;
            }
        }
        if let Some(b) = outer_edge {
            hs = hs.max(b + 1);
        }
        if hs < ci {
            hole_start[m] = hs as u32;
            need_from[m] = ci as u32;
        }
    }
    for (e, &m) in into.iter().enumerate() {
        if m != NONE && need_from[m as usize] != NONE {
            need_from[m as usize] = need_from[m as usize].min(def[e]);
        }
    }
    Ok((hole_start, need_from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;
    use crate::shaderc::online::v3d::select::{Operand, Uniform, V};
    use std::vec::Vec;

    /// `n` uniforms loaded, then all read by one chain of adds, so all `n`
    /// are live at once.
    fn live_at_once(n: u32) -> Vec<VOp> {
        let mut ops: Vec<VOp> = (0..n).map(|i| VOp::Ldunif { dst: V(i), u: Uniform::constant(i) }).collect();
        let mut acc = V(0);
        for i in 1..n {
            let d = V(n + i);
            ops.push(VOp::Add {
                op: AddOp::Fadd,
                dst: super::super::select::Dst::V(d),
                a: Operand::V(acc),
                b: Operand::V(V(i)),
            });
            acc = d;
        }
        ops
    }

    fn alloc(n: u32) -> Result<(u8, usize), AllocError> {
        alloc_at(n, 4)
    }

    fn alloc_at(n: u32, threads: u32) -> Result<(u8, usize), AllocError> {
        store::with_scratch(|a| {
            let ops = live_at_once(n);
            let sel = Selected {
                fragment: false,
                ops: &ops,
                vregs: 2 * n,
                vattr_sizes: [0; 16],
                vpm_output_rows: 0,
                uses_center_w: false,
                implicit_point_varyings: false,
                uses_vid: false,
                uses_iid: false,
                writes_z: false,
                disable_ez: false,
            };
            allocate(&sel, threads, a).map(|al| (al.rf_end, al.reg.iter().filter(|r| matches!(r, Reg::Acc(_))).count()))
        })
        .unwrap()
    }

    /// A Copy `m <- e` that is e's last read gives e m's register when
    /// nothing reads or writes m between e's definition and the Copy (a
    /// uniform branch's else value into its merge register; the Copy then
    /// is `mov rX, rX` and the scheduler drops it). Where m is read in
    /// between, e keeps a register of its own. Revert checks: without the
    /// rule the first pair differs; without the "read between" check the
    /// second pair shares.
    #[test]
    fn a_copy_that_ends_its_source_writes_it_into_the_destination_register() {
        use super::super::select::Dst;
        let run = |ops: &[VOp]| -> Vec<Reg> {
            store::with_scratch(|a| {
                let sel = Selected {
                    ops,
                    fragment: true,
                    vregs: 6,
                    vattr_sizes: [0; 16],
                    vpm_output_rows: 0,
                    uses_center_w: false,
                    implicit_point_varyings: false,
                    uses_vid: false,
                    uses_iid: false,
                    writes_z: false,
                    disable_ez: false,
                };
                allocate(&sel, 4, a).expect("allocates").reg.to_vec()
            })
            .unwrap()
        };
        let fadd = |d: u32, x: u32, y: u32| VOp::Add {
            op: AddOp::Fadd,
            dst: Dst::V(V(d)),
            a: Operand::V(V(x)),
            b: Operand::V(V(y)),
        };
        /* m = V(1) defined, e = V(2) defined later and last read by the Copy */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(1) },
            fadd(1, 0, 0),
            fadd(2, 0, 0),
            VOp::Copy { dst: V(1), src: Operand::V(V(2)) },
            fadd(3, 1, 1),
        ];
        let reg = run(&ops);
        assert_eq!(reg[2], reg[1], "e is written into m's register");
        /* m read between e's definition and the Copy: its value is still
         * needed there, so e takes another register */
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(1) },
            fadd(1, 0, 0),
            fadd(2, 0, 0),
            fadd(4, 1, 0),
            VOp::Copy { dst: V(1), src: Operand::V(V(2)) },
            fadd(3, 1, 4),
        ];
        let reg = run(&ops);
        assert_ne!(reg[2], reg[1], "m is read after e is made: separate registers");
    }

    /// A MOV that is its source's last read takes the source's register,
    /// even when a lower one is free (the MOV is then `mov rX, rX`, which
    /// the scheduler drops); a MOV whose source is read after it takes the
    /// lowest free one as before. Revert check: without the preference the
    /// first MOV takes r0.
    #[test]
    fn a_move_that_ends_its_source_takes_the_source_register() {
        use super::super::isa::{Pf, Uf};
        use super::super::select::{Dst, FlagOp};
        let run = |ops: &[VOp]| -> Vec<Reg> {
            store::with_scratch(|a| {
                let sel = Selected {
                    ops,
                    fragment: true,
                    vregs: 6,
                    vattr_sizes: [0; 16],
                    vpm_output_rows: 0,
                    uses_center_w: false,
                    implicit_point_varyings: false,
                    uses_vid: false,
                    uses_iid: false,
                    writes_z: false,
                    disable_ez: false,
                };
                allocate(&sel, 4, a).expect("allocates").reg.to_vec()
            })
            .unwrap()
        };
        let v = |n: u32| Operand::V(V(n));
        let mov = |d: u32, x: u32| VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(d)), a: v(x), b: Operand::None };
        let pack = |x: u32, y: u32| VOp::Add { op: AddOp::Vfpack, dst: Dst::Tlb, a: v(x), b: v(y) };
        /* v0 in r0, v1 in r1 (loads take accumulators round-robin); v0's
         * last read frees r0 before the MOV of v1 */
        let head = [
            VOp::Ldvary { dst: V(0) },
            VOp::Ldvary { dst: V(1) },
            VOp::Flags { unit: FlagOp::Mul(MulOp::Fmov), a: v(0), b: Operand::None, pf: Pf::PushZ, uf: Uf::None },
        ];
        let ops = [head[0], head[1], head[2], mov(2, 1), pack(2, 2)];
        let reg = run(&ops);
        assert_eq!((reg[0], reg[1]), (Reg::Acc(0), Reg::Acc(1)));
        assert_eq!(reg[2], reg[1], "the MOV ends v1: it takes v1's register, not the free r0");
        /* v1 read after the MOV: the lowest free register */
        let ops = [head[0], head[1], head[2], mov(2, 1), pack(2, 1)];
        assert_eq!(run(&ops)[2], Reg::Acc(0), "v1 is still read: the MOV takes r0");
    }

    /// A value hoist may move takes the free register that lets it move
    /// furthest (step (l)). Here r0 holds `u0` until op 4 and the chain
    /// `v2 = v1 + v1; v3 = v2 + v2` takes r1 and reads it in op 4; the late
    /// uniform load `u5` feeds only the FMUL of op 6. Round-robin gives the
    /// load r1 (the accumulator after the last one taken), which op 4 reads,
    /// so hoist cannot move it above op 4's word, and the FMUL follows it.
    /// The hoist-aware choice gives it r2, which nothing touched: the load
    /// goes into word 2's free signal slot and the FMUL into word 3's free
    /// mul slot, one word fewer. Revert check: with `stop` always 0 the load
    /// takes r1 and the second assertion fails.
    #[test]
    fn a_late_load_takes_a_register_nothing_reads_inside_its_hoist_window() {
        use super::super::sched::schedule;
        use super::super::select::Dst;
        let v = |n: u32| Operand::V(V(n));
        let fadd = |d: u32, x: u32, y: u32| VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(d)), a: v(x), b: v(y) };
        let ops = [
            VOp::Ldunif { dst: V(0), u: Uniform::constant(0) },
            VOp::Ldunif { dst: V(1), u: Uniform::constant(1) },
            fadd(2, 1, 1),
            fadd(3, 2, 2),
            fadd(4, 0, 3),
            VOp::Ldunif { dst: V(5), u: Uniform::constant(5) },
            VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(6)), a: v(5), b: v(5) },
            VOp::Add { op: AddOp::Vfpack, dst: Dst::Tlb, a: v(6), b: v(4) },
        ];
        /* a vertex-stage selection: no load takes r5 (alloc's r5 rule is
         * for fragment shaders), so the accumulators decide */
        let sel = Selected {
            ops: &ops,
            fragment: false,
            vregs: 7,
            vattr_sizes: [0; 16],
            vpm_output_rows: 0,
            uses_center_w: false,
            implicit_point_varyings: false,
            uses_vid: false,
            uses_iid: false,
            writes_z: false,
            disable_ez: false,
        };
        let (reg, words, words_rr) = store::with_scratch(|a| {
            let al = allocate(&sel, 4, a).expect("allocates");
            let reg = al.reg.to_vec();
            let words = schedule(&sel, &al, a).expect("schedules").instrs.len();
            let mut rr = reg.clone();
            rr[5] = Reg::Acc(1);
            let al_rr = Allocation { reg: &rr, rf_end: al.rf_end };
            let words_rr = schedule(&sel, &al_rr, a).expect("schedules").instrs.len();
            (reg, words, words_rr)
        })
        .unwrap();
        assert_eq!((reg[0], reg[2], reg[3], reg[4]), (Reg::Acc(0), Reg::Acc(1), Reg::Acc(1), Reg::Acc(0)), "{reg:?}");
        assert_eq!(reg[5], Reg::Acc(2), "the late load takes r2, which nothing reads after its hoist point: {reg:?}");
        assert_eq!(words + 1, words_rr, "in r1 the load and its FMUL wait behind op 4's read: a word more");
    }

    /// A MOV that ends its source and writes a value that must be in the
    /// register file (live across a thread switch: a loop's carried value
    /// set before the loop) puts its source in the register file too, in
    /// the same register: the MOV is `mov rfN, rfN` and the scheduler drops
    /// it (femtovg filter_image's five loop-carried sums, step (m)). A
    /// uniform load keeps its accumulator (as an ldunifrf it would take the
    /// condition field). Revert check: without the rule the FMUL result
    /// takes an accumulator and the first assertion fails.
    #[test]
    fn a_move_into_the_register_file_puts_its_source_there() {
        use super::super::select::Dst;
        let v = |n: u32| Operand::V(V(n));
        let run = |x_op: VOp| -> Vec<Reg> {
            let ops = [
                VOp::Ldunif { dst: V(0), u: Uniform::constant(0) },
                x_op,
                VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(2)), a: v(1), b: Operand::None },
                VOp::Thrsw,
                VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(3)), a: v(2), b: v(0) },
            ];
            store::with_scratch(|a| {
                let sel = Selected {
                    ops: &ops,
                    fragment: false,
                    vregs: 4,
                    vattr_sizes: [0; 16],
                    vpm_output_rows: 0,
                    uses_center_w: false,
                    implicit_point_varyings: false,
                    uses_vid: false,
                    uses_iid: false,
                    writes_z: false,
                    disable_ez: false,
                };
                allocate(&sel, 4, a).expect("allocates").reg.to_vec()
            })
            .unwrap()
        };
        let reg = run(VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(1)), a: v(0), b: v(0) });
        assert!(matches!(reg[2], Reg::Rf(_)), "the carried value is in the register file: {reg:?}");
        assert_eq!(reg[1], reg[2], "the FMUL writes it where it is kept: {reg:?}");
        let reg = run(VOp::Ldunif { dst: V(1), u: Uniform::constant(1) });
        assert!(matches!(reg[1], Reg::Acc(_)), "a uniform load keeps its accumulator: {reg:?}");
    }

    #[test]
    fn a_program_that_does_not_fit_four_threads_is_refused() {
        /* 5 accumulators + rf3..rf31 (29) = 34 values. The chain needs one
         * register free for each add's result only once the operand it
         * replaces dies, so 34 live uniforms fit and 35 do not. (Not a
         * fragment shader, and long-lived: no load takes r5.) */
        assert_eq!(alloc(34).map(|(end, _)| end), Ok(32), "34 values fill r0..r4 and rf3..rf31");
        assert_eq!(alloc(35), Err(AllocError::DoesNotFit));
    }

    #[test]
    fn two_threads_have_twice_the_register_file() {
        /* 5 accumulators + rf3..rf63 (61) = 66 values at 2 threads, the
         * same bound as 4 threads' 34 with the file doubled; 1 thread has
         * no more registers than 2 on 4.x (vir_register_allocate.c:1428-1435). */
        assert_eq!(alloc_at(35, 2).map(|(end, _)| end), Ok(33), "the 35th value takes rf32");
        assert_eq!(alloc_at(66, 2).map(|(end, _)| end), Ok(64));
        assert_eq!(alloc_at(67, 2), Err(AllocError::DoesNotFit));
        assert_eq!(alloc_at(66, 1).map(|(end, _)| end), Ok(64));
        assert_eq!(alloc_at(1, 3), Err(AllocError::Threads));
        assert_eq!(alloc_at(1, 8), Err(AllocError::Threads));
    }

    #[test]
    fn a_value_live_across_a_thread_switch_is_not_in_an_accumulator() {
        store::with_scratch(|a| {
            let ops = [
                VOp::Ldunif { dst: V(0), u: Uniform::constant(1) },
                VOp::Ldunif { dst: V(1), u: Uniform::constant(2) },
                VOp::Thrsw,
                VOp::Add {
                    op: AddOp::Fadd,
                    dst: super::super::select::Dst::V(V(2)),
                    a: Operand::V(V(0)),
                    b: Operand::V(V(1)),
                },
            ];
            let sel = Selected {
                fragment: false,
                ops: &ops,
                vregs: 3,
                vattr_sizes: [0; 16],
                vpm_output_rows: 0,
                uses_center_w: false,
                implicit_point_varyings: false,
                uses_vid: false,
                uses_iid: false,
                writes_z: false,
                disable_ez: false,
            };
            let al = allocate(&sel, 4, a).unwrap();
            assert!(matches!(al.reg[0], Reg::Rf(_)) && matches!(al.reg[1], Reg::Rf(_)), "{:?}", al.reg);
            /* Defined after the switch: free to use an accumulator. */
            assert!(matches!(al.reg[2], Reg::Acc(_)), "{:?}", al.reg);
        })
        .unwrap();
    }

    /// A uniform branch's merge register holds nothing between its else
    /// label and the Copy that ends the else arm: the then arm wrote it and
    /// jumped over that arm (holes). An else arm with 34 values live at
    /// once (4 threads' 5 accumulators and rf3..rf31) fits only when the
    /// merge register is lent to one of them. Where a path could skip the
    /// Copy (a branch in the arm to a label past it) or the arm reads the
    /// merge register, nothing is lent and the arm does not fit; nor where
    /// the arm reads it at the top of a loop (the back edge leads back to
    /// that read, so the hole starts after the loop).
    /// filter_image's break guards (step (b)) went from 4 threads to 2
    /// without this: two merge registers per carried value, each held
    /// across the whole body. An arm that fits without lending is not lent
    /// the register (the fallback order). Revert checks: without the lending the first
    /// case is DoesNotFit; without the skip rule the second fits; without
    /// the loop rule the fourth fits.
    #[test]
    fn a_merge_register_is_lent_inside_the_else_arm_it_is_not_live_in() {
        use super::super::isa::{BranchCond, Msfign, MulOp};
        use super::super::select::Dst;
        /* 0 the value before the `if`, 1 the merge register, 2.. the arm */
        const K: u32 = 34;
        let build = |k: u32, skip: bool, read: bool, looped: bool| -> Vec<VOp> {
            let mut ops = vec![
                VOp::Ldunif { dst: V(0), u: Uniform::constant(0) },
                VOp::FwdBranch { cond: BranchCond::AnyNA, msfign: Msfign::P, label: 0 },
                VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(1)), a: Operand::V(V(0)), b: Operand::None },
                VOp::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: 1 },
                VOp::Label(0),
            ];
            if looped {
                /* a read of the merge register at the top of a loop in the
                 * arm: the back edge leads to it again */
                ops.push(VOp::LoopHead);
                ops.push(VOp::Add {
                    op: AddOp::Fadd,
                    dst: Dst::V(V(4 + 2 * k)),
                    a: Operand::V(V(1)),
                    b: Operand::V(V(1)),
                });
            }
            for i in 0..k {
                ops.push(VOp::Ldunif { dst: V(2 + i), u: Uniform::constant(i) });
            }
            if skip {
                ops.push(VOp::FwdBranch { cond: BranchCond::AnyA, msfign: Msfign::P, label: 1 });
            }
            let mut acc = V(2);
            for i in 1..k {
                let d = V(2 + k + i);
                let b = if read && i == 1 { V(1) } else { V(2 + i) };
                ops.push(VOp::Add { op: AddOp::Fadd, dst: Dst::V(d), a: Operand::V(acc), b: Operand::V(b) });
                if read && i == 1 {
                    let d2 = V(2 + 2 * k);
                    ops.push(VOp::Add { op: AddOp::Fadd, dst: Dst::V(d2), a: Operand::V(d), b: Operand::V(V(3)) });
                    acc = d2;
                } else {
                    acc = d;
                }
            }
            if looped {
                ops.push(VOp::BackEdge { cond: BranchCond::AnyA, msfign: Msfign::P });
            }
            ops.push(VOp::Copy { dst: V(1), src: Operand::V(acc) });
            ops.push(VOp::Label(1));
            ops.push(VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(3 + 2 * k)), a: Operand::V(V(1)), b: Operand::V(V(1)) });
            ops
        };
        let run = |ops: &[VOp]| -> Result<Vec<Reg>, AllocError> {
            store::with_scratch(|a| {
                let sel = Selected {
                    ops,
                    fragment: true,
                    vregs: 5 + 2 * K,
                    vattr_sizes: [0; 16],
                    vpm_output_rows: 0,
                    uses_center_w: false,
                    implicit_point_varyings: false,
                    uses_vid: false,
                    uses_iid: false,
                    writes_z: false,
                    disable_ez: false,
                };
                allocate(&sel, 4, a).map(|al| al.reg.to_vec())
            })
            .unwrap()
        };
        let reg = run(&build(K, false, false, false)).expect("fits 4 threads with the merge register lent");
        assert!((2..2 + K).any(|v| reg[v as usize] == reg[1]), "an else-arm value takes the merge register: {reg:?}");
        /* an arm that fits without lending keeps the registers a scan
         * without holes gives it: none takes the merge register */
        let reg = run(&build(8, false, false, false)).expect("fits");
        assert!(!(2..2 + 8).any(|v| reg[v as usize] == reg[1]), "lent only when nothing else is free: {reg:?}");
        assert_eq!(
            run(&build(K, true, false, false)),
            Err(AllocError::DoesNotFit),
            "a path skips the Copy: nothing lent"
        );
        assert_eq!(run(&build(K, false, true, false)), Err(AllocError::DoesNotFit), "the arm reads it: nothing lent");
        assert_eq!(
            run(&build(K, false, false, true)),
            Err(AllocError::DoesNotFit),
            "read at the top of a loop in the arm: nothing lent inside the loop"
        );
    }
}
