// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/compiler/vir_register_allocate.c
// and qpu/qpu_instr.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Register allocation at V3D 7.1 (`docs/notes/v3d71-backend.md`, (b)): a
//! linear scan over the [`Op71`]s in program order, as [`super::alloc`] does
//! at 4.2, which is also the order sched71 keeps (it merges operations into
//! words but never reorders them, so "last read at operation i" is exact).
//!
//! # The register file at 7.1
//!
//! There are no accumulators (`has_accumulators = ver < 71`,
//! common/v3d_device_info.c:76): Mesa's allocator has no accumulator nodes at
//! 7.1, only one fixed node for rf0, the register written implicitly
//! (vir_register_allocate.c:1398-1404, :1454-1460), and every class is the
//! register file (:67-75). So there is one register kind, and the 4.2 class
//! rules (an SFU or `ldvpmv_in` result, a value live across a thread switch:
//! physical, :1182-1210, :1340-1347) hold for every value already.
//!
//! * rf0..rf31 at 4 threads, rf0..rf63 at 1 or 2 (`PHYS_COUNT >> threads`,
//!   vir_register_allocate.c:1054-1072, the thread index :1427-1435; no
//!   version branch), as at 4.2.
//! * rf1, rf2 and rf3 are never handed out, in any stage: at 7.1 they hold
//!   the fragment payload, centroid W, Z and W (nir_to_vir.c:4851-4858;
//!   `reg_is_payload`, vir_register_allocate.c:1103-1124), read through
//!   [`Src71::Payload`]. Mesa starts at rf4 "to prevent TLB writes from using
//!   RF2-3" (:1415-1421) and keeps the last instructions off rf0..rf3
//!   (`safe_rf_start`, :977-987). Every value goes to rf4 and up, lowest
//!   first ([`RF_FIRST71`]).
//! * rf0 is the implicit destination of `ldunif`, `ldunifa` and `ldvary`'s C
//!   term (`v3d_qpu_writes_rf0_implicitly`, qpu/qpu_instr.c:1030-1040). Mesa
//!   keeps every temp live across such a write out of rf0
//!   (vir_register_allocate.c:1167-1177), an `ldvary`'s destination out of it
//!   (:1314-1322), and a temp live from the last `ldvary` to an explicit rf0
//!   read (:1245-1261, the tracking :1474-1491). It gives rf0 to an `ldunif`
//!   destination when it can (`try_rf0`, :1323-1336, :965-975), because a load
//!   into rf0 stays a plain `ldunif`, while any other destination makes it an
//!   `ldunifrf` whose address takes the condition field (vir_to_qpu.c:329-350);
//!   rf0 is otherwise kept out (:993-995) and a last resort (:1006-1012).
//!
//! Here rf0 goes to an [`Op71::Ldunif`] destination only, under the rule
//! alloc.rs keeps for r5 (the note's choice): a fragment shader, the value
//! read within [`R0_SPAN`] operations, no implicit rf0 write, `ldvary` C
//! read, thread switch or control flow inside its life, no `ldvary` or C read
//! in the [`R0_AFTER`] operations before the load (Mesa's
//! `ip > last_ldvary_ip + 4`, :1330-1332), and no `ldvary` C live across it.
//! One value holds rf0 at a time. Nothing else is ever given rf0, so no
//! implicit write can land on a live value, an `ldvary` never writes rf0 as
//! its destination, and sched71 never sees an ALU write of rf0. Every
//! [`Op71::Ldunif`] counts as an implicit rf0 write here, whatever its
//! destination: so does Mesa's allocator, which runs before vir_to_qpu.c
//! turns a non-rf0 one into `ldunifrf`.
//!
//! # The read ports
//!
//! At 7.1 each ALU operand has its own address (raddr_a..raddr_d,
//! qpu_pack.c:87-111), and two operations merge unless both use a small
//! immediate (`qpu_merge_raddrs`, qpu_schedule.c:1001-1032). The note assigns
//! no read-port rule to the allocator: there is nothing to optimise here.
//!
//! # What alloc.rs does that this keeps, and leaves out
//!
//! Kept: the loop rule (a value read in a counted loop's body lives to its
//! back edge), the lifetime holes of a [`Op71::Copy`] target (its register is
//! lent inside the hole), a Copy's source written straight into its
//! destination's register, and a plain MOV that ends its source taking the
//! source's register. Left out: the round-robin for loads and the
//! hoist-aware choice, which serve sched.rs's `hoist` and `ldvary_back`
//! (sched71 has neither yet).
//!
//! There is no spilling: a program that does not fit is
//! [`Alloc71Error::DoesNotFit`], by name, and chain71 retries at 2 threads
//! (and later with chain.rs's `Pressure` levels).

use super::vop71::{MulOp71, Op71, Selected71, Src71, V};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};

/// A register-file register, rf0..rf63.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rf(pub u8);

/// rf0, the implicit destination of `ldunif` and `ldvary`'s C term: given
/// only to an [`Op71::Ldunif`] destination, which sched71 then writes as a
/// plain `ldunif` (any other destination is an `ldunifrf`).
pub const RF_IMPLICIT: Rf = Rf(0);
/// The first register-file register handed out (rf1..rf3 are the payload;
/// rf3 on 4.2).
pub const RF_FIRST71: u8 = 4;
/// One past the last register a thread has at 4 threads.
pub const RF_END71_4_THREADS: u8 = 32;
/// One past the last register a thread has at 1 or 2 threads.
pub const RF_END71_2_THREADS: u8 = 64;
/// A uniform load takes rf0 only when it is read within this many
/// operations (alloc.rs `R5_SPAN`).
pub const R0_SPAN: usize = 2;
/// Nor when an `ldvary` or a read of its C term is among the operations
/// this far before it (alloc.rs `R5_AFTER`; Mesa's `last_ldvary_ip + 4`).
pub const R0_AFTER: usize = 4;

/// One past the last register-file register at `threads` threads; `None`
/// for a count the V3D does not run (1, 2 and 4 only).
pub const fn rf_end71(threads: u32) -> Option<u8> {
    match threads {
        4 => Some(RF_END71_4_THREADS),
        1 | 2 => Some(RF_END71_2_THREADS),
        _ => None,
    }
}

/// Why 7.1 allocation stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alloc71Error {
    /// Not allocated at 7.1 yet, by name.
    NotYet(&'static str),
    /// More values are live at one point than there are registers at the
    /// thread count asked for (no spilling).
    DoesNotFit,
    /// A thread count the V3D does not run (not 1, 2 or 4).
    Threads,
    /// An operation reads a virtual register nothing defined before it (or
    /// one at or past `Selected71::vregs`).
    UseBeforeDef,
    /// A virtual register is defined twice (selection hands each out once).
    DefinedTwice,
    /// A [`Op71::BackEdge`] with no open [`Op71::LoopHead`].
    UnmatchedBackEdge,
    OutOfScratch,
}

impl From<ArenaError> for Alloc71Error {
    fn from(_: ArenaError) -> Self {
        Alloc71Error::OutOfScratch
    }
}

/// Where each virtual register lives at 7.1.
#[derive(Clone, Copy, Debug)]
pub struct Allocation71<'a> {
    /// Indexed by virtual register. [`RF_IMPLICIT`] only for an
    /// [`Op71::Ldunif`] destination; a value never defined holds rf0 too
    /// (nothing reads it).
    pub reg: &'a [Rf],
    /// One past the highest register used from [`RF_FIRST71`] on
    /// (RF_FIRST71 when none; rf0 does not count).
    pub rf_end: u8,
}

const NONE: u32 = u32::MAX;

fn reads_vary_c(o: &Op71) -> bool {
    o.uses().contains(&Src71::LdvaryC)
}

/// Allocate `sel`'s virtual registers for `threads` threads.
pub fn allocate71<'a>(sel: &Selected71<'_>, threads: u32, a: &Arena<'a>) -> Result<Allocation71<'a>, Alloc71Error> {
    let end = rf_end71(threads).ok_or(Alloc71Error::Threads)?;
    let n = sel.vregs as usize;
    let ops = sel.ops;
    let def = a.alloc_filled(n, NONE)?;
    let last = a.alloc_filled(n, NONE)?;
    for (i, op) in ops.iter().enumerate() {
        for o in op.uses() {
            if let Some(v) = o.vreg() {
                let v = v.0 as usize;
                if def.get(v).copied().unwrap_or(NONE) == NONE {
                    return Err(Alloc71Error::UseBeforeDef);
                }
                last[v] = i as u32;
            }
        }
        if let Some(v) = op.def() {
            let v = v.0 as usize;
            if def.get(v).copied().ok_or(Alloc71Error::UseBeforeDef)? != NONE {
                return Err(Alloc71Error::DefinedTwice);
            }
            def[v] = i as u32;
            last[v] = i as u32;
        }
    }
    /* A counted loop (LoopHead .. BackEdge): a value defined before the head
     * and read inside the body is read again on every pass, so it lives to
     * the back edge (alloc.rs). Loops nest: the innermost open head is the
     * one a back edge closes, and an inner loop is closed first, so a value
     * it extends is then extended to the outer back edge by the same rule. */
    let mut heads: AVec<'a, usize> = AVec::new();
    for (e, op) in ops.iter().enumerate() {
        match op {
            Op71::LoopHead => heads.push(a, e)?,
            Op71::BackEdge { .. } => {
                let h = heads.pop().ok_or(Alloc71Error::UnmatchedBackEdge)?;
                for v in 0..n {
                    if def[v] != NONE && (def[v] as usize) < h && (last[v] as usize) >= h && (last[v] as usize) < e {
                        last[v] = e as u32;
                    }
                }
            }
            _ => {}
        }
    }

    /* A Copy `m <- e` that is e's last read can write e straight into m's
     * register when e is defined after m and nothing reads m between e's
     * definition and the Copy (alloc.rs `into`): the Copy is then
     * `mov rfN, rfN`, which the scheduler drops. */
    let into = a.alloc_filled(n, NONE)?;
    for (ci, op) in ops.iter().enumerate() {
        let Op71::Copy { dst: m, src: Src71::V(e) } = *op else { continue };
        let (m, e) = (m.0 as usize, e.0 as usize);
        if last[e] as usize != ci || def[e] == NONE || def[m] == NONE || def[e] <= def[m] {
            continue;
        }
        let between = &ops[def[e] as usize + 1..ci];
        if !between.iter().any(|o| o.uses().iter().any(|u| u.vreg() == Some(V(m as u32)))) {
            into[e] = m as u32;
        }
    }
    let (hole_start, need_from) = holes(sel, def, into, a)?;

    let reg = a.alloc_filled(n, RF_IMPLICIT)?;
    /* Per register, the operation index of the last read of the value in it
     * (NONE: never held anything); the value that holds it until then; and
     * the last read of the value lent it inside that value's hole. */
    let mut busy = [NONE; RF_END71_2_THREADS as usize];
    let mut owner = [NONE; RF_END71_2_THREADS as usize];
    let mut lent = [NONE; RF_END71_2_THREADS as usize];
    let mut rf0_busy = NONE;
    let mut rf_end = RF_FIRST71;
    /* What ends a uniform's stay in rf0: another implicit write (every
     * Ldunif and Ldvary), a read of ldvary's C (which is rf0), a thread
     * switch (the rf0 flops rule, qpu_schedule.c:1692-1745, is sched71's;
     * no rf0 value crosses one so it never matters), control flow. */
    let ends_rf0 = |o: &Op71| o.writes_rf0_implicitly() || reads_vary_c(o) || *o == Op71::Thrsw || o.is_control_flow();
    let near_ldvary = |o: &Op71| matches!(o, Op71::Ldvary { .. }) || reads_vary_c(o);
    for (i, op) in ops.iter().enumerate() {
        let Some(v) = op.def() else { continue };
        let v = v.0 as usize;
        /* A register whose value is last read by this very operation, or
         * earlier, is free: an instruction reads its operands before it
         * writes, and the scheduler never moves a later write before an
         * earlier read. */
        let free = |b: u32| b == NONE || b <= i as u32;
        /* m's register is m's until its last read, past this value's (not
         * rf0: no arithmetic result and no ldvary goes there). */
        if into[v] != NONE && reg[into[v] as usize] != RF_IMPLICIT {
            reg[v] = reg[into[v] as usize];
            continue;
        }
        if matches!(op, Op71::Ldunif { .. })
            && sel.fragment
            && free(rf0_busy)
            && (last[v] as usize) <= i + R0_SPAN
            && !ops[i + 1..=(last[v] as usize).max(i)].iter().any(ends_rf0)
            && !ops[i.saturating_sub(R0_AFTER)..i].iter().any(near_ldvary)
            && !vary_c_live_after(ops, i)
        {
            rf0_busy = last[v];
            reg[v] = RF_IMPLICIT;
            continue;
        }
        /* A register is free for this value when its holder's last read is
         * past, or when this value lives inside the holder's hole: defined
         * at or after the hole's start, last read no later than where the
         * holder needs the register again, and no other value lent it
         * still read (alloc.rs). */
        let lendable = |o: u32| {
            o != NONE
                && hole_start[o as usize] != NONE
                && hole_start[o as usize] <= i as u32
                && last[v] <= need_from[o as usize]
        };
        let ok = |r: u8, lend: bool| {
            let r = r as usize;
            if lend {
                lendable(owner[r]) && free(lent[r])
            } else {
                free(busy[r])
            }
        };
        /* A plain move that is its source's last read takes the source's
         * register when the source holds it in the plain sense: the move is
         * then `mov rfN, rfN`, which the scheduler drops (alloc.rs). */
        let same = match *op {
            Op71::Mul { op: MulOp71::Mov, a: Src71::V(x), b: Src71::None, .. } if last[x.0 as usize] == i as u32 => {
                let r = reg[x.0 as usize].0;
                (r >= RF_FIRST71 && r < end && free(busy[r as usize])).then_some(r)
            }
            _ => None,
        };
        let pick = |lend: bool| (RF_FIRST71..end).find(|&r| ok(r, lend));
        /* First a register free in the plain sense; only when there is none,
         * one lent inside its holder's hole. */
        let (r, l) = match (same, pick(false)) {
            (Some(r), _) | (None, Some(r)) => (r, false),
            (None, None) => (pick(true).ok_or(Alloc71Error::DoesNotFit)?, true),
        };
        let k = r as usize;
        if l {
            lent[k] = last[v];
        } else {
            (busy[k], owner[k], lent[k]) = (last[v], v as u32, NONE);
        }
        rf_end = rf_end.max(r + 1);
        reg[v] = Rf(r);
    }
    Ok(Allocation71 { reg, rf_end })
}

/// The C term of an earlier `ldvary` is still to be read after operation
/// `i`: the first operation after it that is an `ldvary` or reads C reads C.
/// A plain `ldunif` at `i` would overwrite it in rf0.
fn vary_c_live_after(ops: &[Op71], i: usize) -> bool {
    ops[i + 1..].iter().find(|o| matches!(o, Op71::Ldvary { .. }) || reads_vary_c(o)).is_some_and(reads_vary_c)
}

/// Lifetime holes, as alloc.rs `holes` finds them at 4.2. A value `m` that
/// one [`Op71::Copy`] rewrites (a uniform branch's merge register at the end
/// of its else arm, a loop's carried value at the end of a pass) holds
/// nothing anyone reads between the point from which no path reaches a read
/// of `m` before that Copy, and the Copy. Per value, `(hole_start,
/// need_from)`: the first operation of the hole and the operation from which
/// the register is `m`'s again (NONE: no hole).
fn holes<'a>(
    sel: &Selected71<'_>,
    def: &[u32],
    into: &[u32],
    a: &Arena<'a>,
) -> Result<(&'a mut [u32], &'a mut [u32]), Alloc71Error> {
    let n = def.len();
    let ops = sel.ops;
    let hole_start = a.alloc_filled(n, NONE)?;
    let need_from = a.alloc_filled(n, NONE)?;
    let labels = ops.iter().filter_map(|o| if let Op71::Label(l) = o { Some(l.0 as usize + 1) } else { None }).max();
    let label_at = a.alloc_filled(labels.unwrap_or(0), NONE)?;
    let head_of = a.alloc_filled(ops.len(), NONE)?;
    let copies = a.alloc_filled(n, 0u32)?;
    let copy_at = a.alloc_filled(n, NONE)?;
    let mut heads: AVec<'a, usize> = AVec::new();
    for (i, op) in ops.iter().enumerate() {
        match *op {
            Op71::Label(l) => label_at[l.0 as usize] = i as u32,
            Op71::LoopHead => heads.push(a, i)?,
            Op71::BackEdge { .. } => head_of[i] = heads.pop().ok_or(Alloc71Error::UnmatchedBackEdge)? as u32,
            Op71::Copy { dst, .. } => {
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
            let reads = op.uses().iter().any(|u| u.vreg() == Some(V(m as u32)));
            let skips = matches!(*op, Op71::FwdBranch { label, .. }
                if label_at.get(label.0 as usize).is_none_or(|&at| at == NONE || at as usize > ci));
            if reads || skips {
                hs = j + 1;
                for b in j + 1..ci {
                    if matches!(ops[b], Op71::BackEdge { .. }) && (head_of[b] as usize) < j {
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
    use super::super::isa::{BranchCond, Cond, Msfign, Pf, Uf};
    use super::super::vop71::{AddOp71, Dst71, FlagOp71, Label, Uniform};
    use super::*;
    use crate::shaderc::online::store;
    use std::vec::Vec;

    fn v(n: u32) -> Src71 {
        Src71::V(V(n))
    }
    fn fadd(d: u32, x: u32, y: u32) -> Op71 {
        Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(d)), a: v(x), b: v(y) }
    }
    fn fmul(d: u32, x: u32, y: u32) -> Op71 {
        Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(d)), a: v(x), b: v(y) }
    }
    fn ldunif(d: u32) -> Op71 {
        Op71::Ldunif { dst: V(d), u: Uniform::constant(d) }
    }
    fn tlb(x: u32) -> Op71 {
        Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(x), b: Src71::None }
    }

    fn run(ops: &[Op71], fragment: bool, vregs: u32, threads: u32) -> Result<(Vec<Rf>, u8), Alloc71Error> {
        store::with_scratch(|a| {
            let sel = Selected71 { ops, fragment, vregs };
            allocate71(&sel, threads, a).map(|al| (al.reg.to_vec(), al.rf_end))
        })
        .unwrap()
    }

    /// Every register handed out is rf0 (a uniform load) or rf4 and up,
    /// below the thread count's end: rf1..rf3 are the payload.
    fn assert_in_file(reg: &[Rf], end: u8) {
        for r in reg {
            assert!(*r == RF_IMPLICIT || (RF_FIRST71..end).contains(&r.0), "{reg:?}");
        }
    }

    /// `n` uniforms loaded, then all read by one chain of adds, so all `n`
    /// are live at once.
    fn live_at_once(n: u32) -> Vec<Op71> {
        let mut ops: Vec<Op71> = (0..n).map(ldunif).collect();
        let mut acc = 0;
        for i in 1..n {
            ops.push(fadd(n + i, acc, i));
            acc = n + i;
        }
        ops
    }

    #[test]
    fn straight_line_code_takes_rf4_and_up_lowest_first() {
        /* a varying times W, plus a uniform loaded far from its read */
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(1)), a: v(0), b: Src71::Payload(3) },
            fadd(2, 1, 1),
            fmul(3, 2, 1),
            fadd(4, 3, 3),
            tlb(4),
        ];
        let (reg, end) = run(&ops, true, 5, 4).unwrap();
        assert_in_file(&reg, 32);
        assert_eq!(reg[0], Rf(4), "an ldvary destination is never rf0: {reg:?}");
        /* v0 is last read by op 1: its register is free for v1 */
        assert_eq!(reg[1], Rf(4), "{reg:?}");
        /* v1 is still read by op 3: v2 takes the next one */
        assert_eq!(reg[2], Rf(5), "{reg:?}");
        assert_eq!(reg[3], Rf(4), "{reg:?}");
        assert_eq!(end, 6);
        /* a uniform load read within two operations in a fragment shader
         * takes rf0 (a plain ldunif); in a vertex shader it does not */
        let ops = [ldunif(0), fadd(1, 0, 0), tlb(1)];
        assert_eq!(run(&ops, true, 2, 4).unwrap().0[0], RF_IMPLICIT);
        assert_eq!(run(&ops, false, 2, 4).unwrap().0[0], Rf(4));
        /* read three operations later: an ldunifrf */
        let ops = [ldunif(0), ldunif(1), fadd(2, 1, 1), fadd(3, 2, 2), fadd(4, 0, 3), tlb(4)];
        let reg = run(&ops, true, 5, 4).unwrap().0;
        assert_ne!(reg[0], RF_IMPLICIT, "{reg:?}");
        assert_eq!(reg[1], RF_IMPLICIT, "{reg:?}");
    }

    #[test]
    fn rf0_implicit_writes_back_to_back() {
        /* v0's life holds v1's load, an implicit rf0 write: v0 is an
         * ldunifrf; v1 is read next and takes rf0 */
        let ops = [ldunif(0), ldunif(1), fadd(2, 0, 1), tlb(2)];
        let reg = run(&ops, true, 3, 4).unwrap().0;
        assert_eq!((reg[0], reg[1]), (Rf(4), RF_IMPLICIT), "{reg:?}");
        /* one after the other, each read before the next load: both rf0 */
        let ops = [ldunif(0), fadd(1, 0, 0), ldunif(2), fadd(3, 2, 1), tlb(3)];
        let reg = run(&ops, true, 4, 4).unwrap().0;
        assert_eq!((reg[0], reg[2]), (RF_IMPLICIT, RF_IMPLICIT), "{reg:?}");
        /* an ldvary inside the life, or its C read: not rf0 */
        let ops = [ldunif(0), Op71::Ldvary { dst: V(1) }, fadd(2, 0, 1), tlb(2)];
        let reg = run(&ops, true, 3, 4).unwrap().0;
        assert_ne!(reg[0], RF_IMPLICIT, "{reg:?}");
        assert_ne!(reg[1], RF_IMPLICIT, "{reg:?}");
        /* within R0_AFTER operations after an ldvary: not rf0 (Mesa's
         * last_ldvary_ip + 4) */
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(1)), a: v(0), b: Src71::LdvaryC },
            ldunif(2),
            fadd(3, 1, 2),
            tlb(3),
        ];
        let reg = run(&ops, true, 4, 4).unwrap().0;
        assert_ne!(reg[2], RF_IMPLICIT, "{reg:?}");
        assert!(!reg.contains(&RF_IMPLICIT), "{reg:?}");
        /* an ldvary's C still to be read after the load, however far: a plain
         * ldunif would overwrite it */
        let mut ops = std::vec![Op71::Ldvary { dst: V(0) }];
        for k in 0..R0_AFTER as u32 + 1 {
            ops.push(fadd(10 + k, 0, 0));
        }
        ops.extend([
            ldunif(1),
            fadd(2, 1, 1),
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(3)), a: v(2), b: Src71::LdvaryC },
            tlb(3),
        ]);
        let reg = run(&ops, true, 16, 4).unwrap().0;
        assert_ne!(reg[1], RF_IMPLICIT, "C is live in rf0 across the load: {reg:?}");
        /* the same without the C read: rf0 */
        let k = ops.len() - 2;
        ops[k] = fadd(3, 2, 2);
        assert_eq!(run(&ops, true, 16, 4).unwrap().0[1], RF_IMPLICIT);
    }

    #[test]
    fn values_live_across_a_thread_switch() {
        let ops = [ldunif(0), ldunif(1), Op71::Thrsw, fadd(2, 0, 1), ldunif(3), Op71::Thrsw, fadd(4, 2, 3), tlb(4)];
        let reg = run(&ops, true, 5, 4).unwrap().0;
        assert_in_file(&reg, 32);
        assert_ne!(reg[0], reg[1], "{reg:?}");
        assert!(reg[..4].iter().all(|r| *r != RF_IMPLICIT), "no rf0 value lives across a switch: {reg:?}");
        /* a load after the switch, read at once, takes rf0 */
        let ops = [ldunif(0), Op71::Thrsw, ldunif(1), fadd(2, 0, 1), tlb(2)];
        let reg = run(&ops, true, 3, 4).unwrap().0;
        assert_eq!((reg[0], reg[1]), (Rf(4), RF_IMPLICIT), "{reg:?}");
    }

    #[test]
    fn cond_mov_and_copy_read_their_destination() {
        /* v0 = a, then v0 = v1 where the flag holds: the CondMov reads v0,
         * so v2, made between, cannot take v0's register */
        let ops = [
            Op71::Ldvary { dst: V(0) },
            Op71::Ldvary { dst: V(1) },
            Op71::Flags { unit: FlagOp71::Mul(MulOp71::Fmov), a: v(1), b: Src71::None, pf: Pf::PushZ, uf: Uf::None },
            fadd(2, 1, 1),
            Op71::CondMov { dst: V(0), src: v(2), cond: Cond::IfA },
            fadd(3, 0, 1),
            tlb(3),
        ];
        let reg = run(&ops, true, 4, 4).unwrap().0;
        assert_ne!(reg[2], reg[0], "{reg:?}");
        assert_ne!(reg[2], reg[1], "{reg:?}");
        /* a CondMov or Copy into a register never defined */
        let ops = [ldunif(0), Op71::CondMov { dst: V(1), src: v(0), cond: Cond::IfA }];
        assert_eq!(run(&ops, true, 2, 4), Err(Alloc71Error::UseBeforeDef));
        let ops = [ldunif(0), Op71::Copy { dst: V(1), src: v(0) }];
        assert_eq!(run(&ops, true, 2, 4), Err(Alloc71Error::UseBeforeDef));
        assert_eq!(run(&[ldunif(0), ldunif(0)], true, 1, 4), Err(Alloc71Error::DefinedTwice));
        assert_eq!(run(&[ldunif(1)], true, 1, 4), Err(Alloc71Error::UseBeforeDef), "past vregs");
        /* a Copy that ends its source writes it into the destination's
         * register; read in between, separate registers */
        let ops = [ldunif(0), fadd(1, 0, 0), fadd(2, 0, 0), Op71::Copy { dst: V(1), src: v(2) }, fadd(3, 1, 1)];
        let reg = run(&ops, false, 4, 4).unwrap().0;
        assert_eq!(reg[2], reg[1], "{reg:?}");
        let ops = [
            ldunif(0),
            fadd(1, 0, 0),
            fadd(2, 0, 0),
            fadd(4, 1, 0),
            Op71::Copy { dst: V(1), src: v(2) },
            fadd(3, 1, 4),
        ];
        let reg = run(&ops, false, 5, 4).unwrap().0;
        assert_ne!(reg[2], reg[1], "{reg:?}");
        /* a MOV that ends its source takes the source's register, even when
         * a lower one is free */
        let ops = [
            ldunif(0),
            ldunif(1),
            fadd(2, 0, 0),
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(V(3)), a: v(1), b: Src71::None },
            fadd(4, 3, 2),
        ];
        let reg = run(&ops, false, 5, 4).unwrap().0;
        assert_eq!((reg[0], reg[1]), (Rf(4), Rf(5)), "{reg:?}");
        assert_eq!(reg[3], Rf(5), "{reg:?}");
    }

    #[test]
    fn a_loop_carried_value_lives_to_the_back_edge() {
        /* v0 carried round the loop, v1 invariant and read at the top of the
         * body: v1 is read again on the next pass, so v2 and v3, made after
         * its last read in program order, cannot take its register */
        let ops = [
            ldunif(0),
            ldunif(1),
            Op71::LoopHead,
            fadd(2, 0, 1),
            fmul(3, 2, 2),
            Op71::Flags { unit: FlagOp71::Add(AddOp71::Fsub), a: v(3), b: v(3), pf: Pf::PushN, uf: Uf::None },
            Op71::Copy { dst: V(0), src: v(3) },
            Op71::BackEdge { cond: BranchCond::AnyA, msfign: Msfign::None },
            tlb(0),
        ];
        let reg = run(&ops, true, 4, 4).unwrap().0;
        assert_in_file(&reg, 32);
        assert!(!reg.contains(&RF_IMPLICIT), "{reg:?}");
        assert_ne!(reg[2], reg[1], "{reg:?}");
        assert_ne!(reg[3], reg[1], "{reg:?}");
        assert_ne!(reg[2], reg[0], "{reg:?}");
        assert_eq!(reg[3], reg[0], "the carried value is written where it is kept: {reg:?}");
        /* a back edge with no head */
        let ops = [ldunif(0), Op71::BackEdge { cond: BranchCond::AnyA, msfign: Msfign::None }];
        assert_eq!(run(&ops, true, 1, 4), Err(Alloc71Error::UnmatchedBackEdge));
    }

    #[test]
    fn a_merge_register_is_lent_inside_the_else_arm() {
        /* the merge register v1 holds nothing in the else arm (its then arm
         * wrote it and jumped over): with 28 values live at once in the arm
         * (rf4..rf31) it fits only when v1's register is lent */
        const K: u32 = 28;
        let mut ops = std::vec![
            ldunif(0),
            Op71::FwdBranch { cond: BranchCond::AnyNA, msfign: Msfign::P, label: Label(0) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(V(1)), a: v(0), b: Src71::None },
            Op71::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: Label(1) },
            Op71::Label(Label(0)),
        ];
        ops.extend((0..K).map(|i| ldunif(2 + i)));
        let mut acc = 2;
        for i in 1..K {
            ops.push(fadd(2 + K + i, acc, 2 + i));
            acc = 2 + K + i;
        }
        ops.push(Op71::Copy { dst: V(1), src: v(acc) });
        ops.push(Op71::Label(Label(1)));
        ops.push(fadd(2 + 2 * K, 1, 1));
        let reg = run(&ops, false, 3 + 2 * K, 4).expect("fits with the merge register lent").0;
        assert!((2..2 + K).any(|x| reg[x as usize] == reg[1]), "{reg:?}");
    }

    #[test]
    fn a_program_that_does_not_fit_is_refused_by_name() {
        /* rf4..rf31 = 28 registers at 4 threads; the chain needs one free
         * for each add's result only once the operand it replaces dies, so
         * 28 live uniforms fit and 29 do not (a vertex shader: no rf0) */
        assert_eq!(run(&live_at_once(28), false, 56, 4).map(|r| r.1), Ok(32));
        assert_eq!(run(&live_at_once(29), false, 58, 4), Err(Alloc71Error::DoesNotFit));
        /* rf4..rf63 = 60 at 1 or 2 threads */
        assert_eq!(run(&live_at_once(29), false, 58, 2).map(|r| r.1), Ok(33));
        assert_eq!(run(&live_at_once(60), false, 120, 2).map(|r| r.1), Ok(64));
        assert_eq!(run(&live_at_once(61), false, 122, 2), Err(Alloc71Error::DoesNotFit));
        assert_eq!(run(&live_at_once(60), false, 120, 1).map(|r| r.1), Ok(64));
        assert_eq!(run(&live_at_once(1), false, 2, 3), Err(Alloc71Error::Threads));
        assert_eq!(run(&live_at_once(1), false, 2, 8), Err(Alloc71Error::Threads));
        /* nothing allocated: rf_end is RF_FIRST71 */
        assert_eq!(run(&[], true, 0, 4), Ok((std::vec![], RF_FIRST71)));
    }
}
