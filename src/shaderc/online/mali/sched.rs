// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2022 Collabora Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/bi_pressure_schedule.c,
// bi_liveness.c and src/util/dag.c (MIT). The upstream part stays under its
// licence (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! The pre-RA scheduler: `bi_pressure_schedule`, a bottom-up list scheduler
//! per block that takes, among the instructions whose users are all
//! scheduled, the one that lowers register pressure most, and keeps the
//! new order only if its peak pressure is lower than the original's
//! (bi_pressure_schedule.c:226-293). A block's branch stays at its end
//! (:52-53), its phis at its start (every later instruction depends on the
//! last phi, as on a preload move, :154-158).
//!
//! The DAG is util/dag.c's: a node's edges point at the earlier
//! instructions it depends on, the heads are the nodes nothing depends on
//! (in creation order, a node leaving the list when it gains a parent), and
//! pruning a head appends each child whose last parent it was to the tail.
//!
//! The live-out set of each block comes from `bi_compute_liveness_ssa`
//! (bi_liveness.c:26-104, [`liveness_ssa`]): a phi's write is killed and
//! its source for the predecessor made live on that edge.

use super::bir::{props, Idx, Ins, Msg, Op, Prog, MAX_BLOCKS, MAX_INSTRS, MAX_NODES};
use super::MaliError;
use crate::shaderc::online::arena::Arena;

const WORDS: usize = MAX_NODES.div_ceil(64);

/// A set of SSA values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Live([u64; WORDS]);

impl Live {
    pub const EMPTY: Live = Live([0; WORDS]);

    fn test(&self, v: u16) -> bool {
        self.0[v as usize / 64] >> (v % 64) & 1 != 0
    }
    fn set(&mut self, v: u16) {
        self.0[v as usize / 64] |= 1 << (v % 64);
    }
    fn clear(&mut self, v: u16) {
        self.0[v as usize / 64] &= !(1 << (v % 64));
    }
    fn or(&mut self, o: &Live) {
        for (a, b) in self.0.iter_mut().zip(o.0.iter()) {
            *a |= *b;
        }
    }
}

fn ssa_dests(i: &Ins) -> impl Iterator<Item = (usize, u16)> + '_ {
    i.dests().iter().enumerate().filter_map(|(d, x)| match x.idx {
        Idx::Ssa(v, _) => Some((d, v)),
        _ => None,
    })
}

fn ssa_srcs(i: &Ins) -> impl Iterator<Item = (usize, u16)> + '_ {
    i.srcs().iter().enumerate().filter_map(|(s, x)| x.ssa_value().map(|v| (s, v)))
}

/// `bi_liveness_ins_update_ssa` (bi_liveness.c:11-24).
fn update(live: &mut Live, i: &Ins) {
    for (_, v) in ssa_dests(i) {
        live.clear(v);
    }
    for (_, v) in ssa_srcs(i) {
        live.set(v);
    }
}

/// `bi_compute_liveness_ssa` (bi_liveness.c:26-104): each block's live-out
/// set, to a fixed point over the blocks in reverse layout order (the
/// worklist's order for a graph without back edges). A block's live-in
/// is its live-out through its instructions backwards, stopping at its
/// phis (:53-64); a predecessor's live-out gains that live-in with the
/// phis' writes killed and the phis' sources for the predecessor made
/// live (:73-99).
pub fn liveness_ssa(p: &Prog<'_>) -> [Live; MAX_BLOCKS] {
    let mut live_in = [Live::EMPTY; MAX_BLOCKS];
    let mut live_out = [Live::EMPTY; MAX_BLOCKS];
    let nb = p.nblocks as usize;
    loop {
        let mut changed = false;
        for b in (0..nb).rev().filter(|&b| !p.blocks[b].dead) {
            let mut out = Live::EMPTY;
            for s in p.blocks[b].succs() {
                let mut l = live_in[s as usize];
                let (start, end) = (p.block_start(s), p.block_end(s));
                let phis = (start..end).take_while(|&k| p.ins[k].op == Op::Phi);
                for k in phis.clone() {
                    for (_, v) in ssa_dests(&p.ins[k]) {
                        l.clear(v);
                    }
                }
                if let Some(i) = p.blocks[s as usize].pred_index(b as u8) {
                    for k in phis {
                        if let Some(v) = p.ins[k].src.get(i).and_then(|x| x.ssa_value()) {
                            l.set(v);
                        }
                    }
                }
                out.or(&l);
            }
            let mut l = out;
            for k in (p.block_start(b as u8)..p.block_end(b as u8)).rev() {
                if p.ins[k].op == Op::Phi {
                    break;
                }
                update(&mut l, &p.ins[k]);
            }
            if out != live_out[b] || l != live_in[b] {
                changed = true;
                live_out[b] = out;
                live_in[b] = l;
            }
        }
        if !changed {
            return live_out;
        }
    }
}

/// `calculate_pressure_delta` (bi_pressure_schedule.c:175-202).
fn delta(i: &Ins, live: &Live) -> i32 {
    let mut d = 0i32;
    for (k, v) in ssa_dests(i) {
        if live.test(v) {
            d -= i.count_write(k) as i32;
        }
    }
    for (s, v) in ssa_srcs(i) {
        let dupe = i.srcs()[..s].iter().any(|x| x.ssa_value() == Some(v));
        if !dupe && !live.test(v) {
            d += i.count_read(s) as i32;
        }
    }
    d
}

struct Dag<'s> {
    /// Children (earlier instructions) of each node, in the scratch arena.
    edges: &'s mut [[u8; MAX_INSTRS]],
    nedges: [u8; MAX_INSTRS],
    parents: [u8; MAX_INSTRS],
    heads: [u8; MAX_INSTRS],
    nheads: usize,
}

impl Dag<'_> {
    fn remove_head(&mut self, n: u8) {
        if let Some(k) = self.heads[..self.nheads].iter().position(|&h| h == n) {
            self.heads.copy_within(k + 1..self.nheads, k);
            self.nheads -= 1;
        }
    }

    /// `dag_add_edge` (dag.c): once per (parent, child).
    fn add(&mut self, parent: Option<u8>, child: Option<u8>) {
        let (Some(p), Some(c)) = (parent, child) else { return };
        let (p, c) = (p as usize, c as usize);
        if self.edges[p][..self.nedges[p] as usize].contains(&(c as u8)) {
            return;
        }
        self.remove_head(c as u8);
        self.edges[p][self.nedges[p] as usize] = c as u8;
        self.nedges[p] += 1;
        self.parents[c] += 1;
    }

    /// `dag_prune_head` (dag.c).
    fn prune(&mut self, n: u8) {
        self.remove_head(n);
        for k in 0..self.nedges[n as usize] as usize {
            let c = self.edges[n as usize][k] as usize;
            self.parents[c] -= 1;
            if self.parents[c] == 0 {
                self.heads[self.nheads] = c as u8;
                self.nheads += 1;
            }
        }
    }
}

/// `create_dag` (bi_pressure_schedule.c:33-164) over one block's
/// instructions before its branch, for the messages here.
fn create_dag(ins: &[Ins], dag: &mut Dag<'_>) {
    let mut last_write = [None::<u8>; MAX_NODES];
    /* no LOAD is selected here, so only a DISCARD ever records a memory
     * load (:141-149) */
    let (mut coverage, mut preload, mut memory_load, mut memory_store) = (None, None, None::<u8>, None);
    for (k, i) in ins.iter().enumerate() {
        let node = Some(k as u8);
        dag.heads[dag.nheads] = k as u8;
        dag.nheads += 1;
        for (_, v) in ssa_srcs(i) {
            dag.add(node, last_write[v as usize]);
        }
        for (_, v) in ssa_dests(i) {
            last_write[v as usize] = node;
        }
        dag.add(node, preload);
        match props(i.op).msg {
            Msg::Store => {
                dag.add(node, memory_load);
                dag.add(node, memory_store);
                memory_store = node;
            }
            Msg::Blend | Msg::ZStencil | Msg::Tile => {
                dag.add(node, coverage);
                coverage = node;
            }
            Msg::Atest => {
                dag.add(node, memory_store);
                memory_store = node;
                dag.add(node, coverage);
                coverage = node;
            }
            /* attribute and varying loads reorder freely (they are not
             * LD_TEX), and so do texture look-ups (the `default` arm); the
             * one LOAD selected here reads a UBO, "read-only so it can be
             * moved around freely" (:70-82) */
            Msg::Attribute | Msg::Varying | Msg::Tex | Msg::Load | Msg::None => {}
        }
        /* a DISCARD is serialised against ATEST (the coverage chain,
         * :136-139) and against memory operations (:141-149) */
        if matches!(i.op, Op::DiscardF32 { .. }) {
            dag.add(node, coverage);
            coverage = node;
            dag.add(node, memory_load);
            dag.add(node, memory_store);
            memory_load = node;
            memory_store = node;
        }
        if i.op == Op::Phi || (i.op == Op::MovI32 && matches!(i.src[0].idx, Idx::Reg(_))) {
            preload = node;
        }
    }
}

/// `pressure_schedule_block` (:226-275) over `p.ins[start..end]`, whose
/// DAG holds `start..stop` (the instructions before the block's branch).
fn schedule_block(
    p: &mut Prog<'_>,
    (start, stop, end): (usize, usize, usize),
    live_out: &Live,
    edges: &mut [[u8; MAX_INSTRS]],
    out: &mut [Ins],
) -> Result<(), MaliError> {
    let (mut pressure, mut orig_max) = (0i32, 0i32);
    let mut live = *live_out;
    for k in (start..end).rev() {
        pressure += delta(&p.ins[k], &live);
        orig_max = orig_max.max(pressure);
        update(&mut live, &p.ins[k]);
    }
    let n = stop - start;
    if n == 0 {
        return Ok(());
    }
    let mut dag = Dag { edges, nedges: [0; MAX_INSTRS], parents: [0; MAX_INSTRS], heads: [0; MAX_INSTRS], nheads: 0 };
    create_dag(&p.ins[start..stop], &mut dag);
    let mut live = *live_out;
    let (mut pressure, mut max) = (0i32, 0i32);
    let mut schedule = [0u8; MAX_INSTRS];
    let mut ns = 0usize;
    while dag.nheads > 0 {
        /* choose_instr: the first head with the least delta */
        let mut best = None;
        let mut min = i32::MAX;
        for &h in &dag.heads[..dag.nheads] {
            let d = delta(&p.ins[start + h as usize], &live);
            if d < min {
                min = d;
                best = Some(h);
            }
        }
        let b = best.ok_or(MaliError::Defect(super::Defect::Schedule))?;
        pressure += delta(&p.ins[start + b as usize], &live);
        max = max.max(pressure);
        dag.prune(b);
        schedule[ns] = b;
        ns += 1;
        update(&mut live, &p.ins[start + b as usize]);
    }
    if ns != n {
        return Err(MaliError::Defect(super::Defect::Schedule));
    }
    if max >= orig_max {
        return Ok(());
    }
    /* list_add at the head, one by one: the bottom-up order reversed */
    for (k, &s) in schedule[..ns].iter().enumerate() {
        out[ns - 1 - k] = p.ins[start + s as usize];
    }
    p.ins[start..stop].copy_from_slice(&out[..n]);
    Ok(())
}

/// `bi_pressure_schedule` (:278-293): each block, with its live-out set.
pub fn pressure_schedule(p: &mut Prog<'_>, scratch: &Arena<'_>) -> Result<(), MaliError> {
    if p.n > MAX_INSTRS {
        return Err(MaliError::Defect(super::Defect::ScheduleTable));
    }
    let live_out = liveness_ssa(p);
    let edges = scratch.alloc_filled(MAX_INSTRS, [0u8; MAX_INSTRS]).map_err(|_| MaliError::OutOfScratch)?;
    let out = scratch.alloc_filled(MAX_INSTRS, Ins::NOP).map_err(|_| MaliError::OutOfScratch)?;
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        let (start, end) = (p.block_start(b), p.block_end(b));
        /* "Leave branches at the end" (:52-53) */
        let stop = (start..end).find(|&k| props(p.ins[k].op).branch).unwrap_or(end);
        schedule_block(p, (start, stop, end), &live_out[b as usize], edges, out)?;
    }
    Ok(())
}
