// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2020-2025 Collabora Ltd.
// SPDX-FileCopyrightText: 2025 Arm Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/bi_ra.c and
// valhall/va_lower_split_64bit.c (MIT). The upstream part stays under its
// licence (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! Register allocation: `bi_register_allocate` (bi_ra.c:1218-1326) on a
//! program of blocks, no spilling.
//!
//! In Mesa's order: `va_lower_split_64bit` (a 64-bit source given as two
//! words gets a COLLECT and a SPLIT), the out-of-SSA pass (each phi a
//! register written by a move at the end of every predecessor, the move
//! then propagated into the writer, [`out_of_ssa`]), `bi_lower_vector`
//! (COLLECT and SPLIT become word moves, the dead ones are removed),
//! `squeeze_index` (values renumbered by first appearance), then LCRA,
//! "Linearly Constrained Register Allocation" (bi_ra.c:36-172), over the
//! split register file r0-r15 and r48-r63 that gives 32 work registers
//! (bi_ra.c:1276-1285), with the interference of each block marked from
//! its live-out set ([`liveness_ra`], `bi_compute_liveness_ra`). A program
//! that does not fit is refused: the full-file retry and spilling are not
//! transcribed.

use super::bir::{self, props, Dest, Idx, Ins, Op, Prog, Src, MAX_BLOCKS, MAX_NODES};
use super::MaliError;
use crate::shaderc::online::arena::Arena;

/// Each block's live-in and live-out words per value
/// (`bi_block::live_in`/`live_out`), `MAX_NODES` bytes a block.
pub struct LiveRa<'s> {
    live_in: &'s mut [u8],
    live_out: &'s mut [u8],
}

impl<'s> LiveRa<'s> {
    fn new(scratch: &Arena<'s>) -> Result<LiveRa<'s>, MaliError> {
        let oom = |_| MaliError::OutOfScratch;
        Ok(LiveRa {
            live_in: scratch.alloc_filled(MAX_BLOCKS * MAX_NODES, 0u8).map_err(oom)?,
            live_out: scratch.alloc_filled(MAX_BLOCKS * MAX_NODES, 0u8).map_err(oom)?,
        })
    }

    pub fn live_out(&self, b: u8) -> &[u8] {
        &self.live_out[b as usize * MAX_NODES..(b as usize + 1) * MAX_NODES]
    }

    fn live_in(&self, b: u8) -> &[u8] {
        &self.live_in[b as usize * MAX_NODES..(b as usize + 1) * MAX_NODES]
    }
}

/// `bi_compute_liveness_ra` (bi_ra.c:215-279): per block, the words of
/// each value live out (the union of the successors' live-in) and live in
/// (live-out through the instructions backwards), to a fixed point over
/// the blocks in reverse layout order.
pub fn liveness_ra(p: &Prog<'_>, l: &mut LiveRa<'_>) {
    for x in l.live_in.iter_mut().chain(l.live_out.iter_mut()) {
        *x = 0;
    }
    let mut live = [0u8; MAX_NODES];
    loop {
        let mut changed = false;
        for b in (0..p.nblocks).rev().filter(|&b| !p.blocks[b as usize].dead) {
            live.fill(0);
            for s in p.blocks[b as usize].succs() {
                for (x, y) in live.iter_mut().zip(l.live_in(s)) {
                    *x |= *y;
                }
            }
            let out = b as usize * MAX_NODES;
            if l.live_out[out..out + MAX_NODES] != live {
                l.live_out[out..out + MAX_NODES].copy_from_slice(&live);
                changed = true;
            }
            for k in (p.block_start(b)..p.block_end(b)).rev() {
                live_update(&mut live, &p.ins[k]);
            }
            if l.live_in[out..out + MAX_NODES] != live {
                l.live_in[out..out + MAX_NODES].copy_from_slice(&live);
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

/// `bi_postra_liveness` (bi_opt_dce.c:86-137): each block's registers live
/// in and out, to a fixed point over the blocks in reverse layout order.
pub fn postra_liveness(p: &Prog<'_>) -> ([u64; MAX_BLOCKS], [u64; MAX_BLOCKS]) {
    let (mut reg_in, mut reg_out) = ([0u64; MAX_BLOCKS], [0u64; MAX_BLOCKS]);
    loop {
        let mut changed = false;
        for b in (0..p.nblocks).rev().filter(|&b| !p.blocks[b as usize].dead) {
            let mut live = 0u64;
            for s in p.blocks[b as usize].succs() {
                live |= reg_in[s as usize];
            }
            reg_out[b as usize] = live;
            for k in (p.block_start(b)..p.block_end(b)).rev() {
                live = postra_live(live, &p.ins[k]);
            }
            if reg_in[b as usize] != live {
                reg_in[b as usize] = live;
                changed = true;
            }
        }
        if !changed {
            return (reg_in, reg_out);
        }
    }
}

/// `bi_after_block_logical` (compiler.h:1805-1817): before the block's
/// final branch, or at its end.
fn logical_end(p: &Prog<'_>, b: u8) -> usize {
    match p.last_of(b) {
        Some(k) if props(p.ins[k].op).branch => k,
        _ => p.block_end(b),
    }
}

/// `bi_out_of_ssa` (bi_ra.c:993-1151): each phi becomes a fresh value
/// written at the logical end of every predecessor (a `MOV.i32` of the
/// phi's source for it, or `IADD_IMM.i32 #0, c` of a constant, the
/// Valhall form of a move of an immediate, :1033-1040) and read by a
/// `MOV.i32` where the phi stood, which the rest of the block then reads
/// directly (:1059-1073). Then each `reg = ssa` move whose source has one
/// use and is written in the move's block takes the writer's destination
/// (:1077-1146: the block scanned from its end, a write of the register
/// after a read of it or before the writer stops the search). Returns the
/// first of the new values (`first_reg`).
fn out_of_ssa(p: &mut Prog<'_>) -> Result<u16, MaliError> {
    let first_reg = p.ssa_alloc;
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        while let Some(k) = (p.block_start(b)..p.block_end(b)).find(|&k| p.ins[k].op == Op::Phi) {
            let phi = p.ins[k];
            let reg = temp(p)?;
            for i in 0..p.blocks[b as usize].npred as usize {
                let pred = p.blocks[b as usize].pred[i];
                let src = phi.src[i];
                let mov = match src.idx {
                    Idx::Const(c) => Ins::new(Op::IaddImmI32(c), &[Dest::ssa(reg)], &[Src::of(Idx::Lut(0))]),
                    _ => Ins::new(Op::MovI32, &[Dest::ssa(reg)], &[src]),
                };
                let at = logical_end(p, pred);
                p.insert_in(at, mov, pred)?;
            }
            /* the phi, where it now stands */
            let k = (p.block_start(b)..p.block_end(b)).find(|&k| p.ins[k].op == Op::Phi).unwrap_or(k);
            let mov = Ins::new(Op::MovI32, &[phi.dest[0]], &[Src::ssa(reg)]);
            p.ins[k] = Ins { block: b, ..mov };
            let Idx::Ssa(dest, _) = phi.dest[0].idx else { return Err(MaliError::Defect(super::Defect::Vector)) };
            for j in (p.block_start(b)..p.block_end(b)).rev() {
                if p.ins[j].op == Op::Phi {
                    break;
                }
                for s in 0..p.ins[j].ns as usize {
                    if let Idx::Ssa(v, off) = p.ins[j].src[s].idx {
                        if v == dest {
                            p.ins[j].src[s].idx = Idx::Ssa(reg, off);
                        }
                    }
                }
            }
        }
    }
    /* the local propagation of the moves made */
    let mut used = [false; MAX_NODES];
    let mut multiple = [false; MAX_NODES];
    for i in p.as_slice() {
        for s in i.srcs() {
            if let Some(v) = s.ssa_value() {
                if used[v as usize] {
                    multiple[v as usize] = true;
                }
                used[v as usize] = true;
            }
        }
    }
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        let mut j = p.block_end(b);
        while j > p.block_start(b) {
            j -= 1;
            let mov = p.ins[j];
            let (Op::MovI32, Idx::Ssa(dst, _), Some(src)) = (mov.op, mov.dest[0].idx, mov.src[0].ssa_value()) else {
                continue;
            };
            if dst < first_reg || src >= first_reg || multiple[src as usize] {
                continue;
            }
            let mut found = false;
            for i in (p.block_start(b)..p.block_end(b)).rev() {
                /* bail: write-after-read */
                if p.ins[i].srcs().iter().any(|s| s.ssa_value() == Some(dst)) {
                    break;
                }
                for d in 0..p.ins[i].nd as usize {
                    let Idx::Ssa(v, _) = p.ins[i].dest[d].idx else { continue };
                    /* bail: write-after-write */
                    if v == dst {
                        break;
                    }
                    if v != src {
                        continue;
                    }
                    /* found: the writer takes the register (`bi_replace_index`
                     * keeps the destination's own offset and swizzle) */
                    if let Idx::Ssa(_, off) = p.ins[i].dest[d].idx {
                        p.ins[i].dest[d].idx = Idx::Ssa(dst, off);
                    }
                    found = true;
                    break;
                }
                if found {
                    break;
                }
            }
            if found {
                p.remove(j);
            }
        }
    }
    Ok(first_reg)
}

/// The registers the hardware preloads that the RA pins
/// (`bi_preload_reg`, compiler.h, for v9+): the blend shader's colour
/// input, its link register and the cumulative coverage.
pub const BLEND_SRC0_C0: u8 = 0;
pub const BLEND_LINK: u8 = 48;
pub const CUMULATIVE_COVERAGE: u8 = 60;
/// `BI_PRELOAD_POSITION_XY` (compiler.h:1138, `bi_preload_reg`
/// :1207-1208): a fragment shader's pixel position, x in the low half and
/// y in the high half. Read through the `MOV.i32` `bi_preload` makes,
/// which the RA coalesces like any preload (no pin).
pub const POSITION_XY: u8 = 59;
/// `BI_PRELOAD_VERTEX_ID` and `BI_PRELOAD_INSTANCE_ID` for arch >= 9
/// (compiler.h:1192-1195): an IDVS vertex shader's gl_VertexID and
/// gl_InstanceID ([`super::nir::Kind::Preload`]).
pub const VERTEX_ID: u8 = 60;
pub const INSTANCE_ID: u8 = 61;
/// `BI_PRELOAD_PRIMITIVE_FLAGS` (compiler.h:1205-1206): a fragment shader's
/// primitive flags, bit 0 set for a back-facing primitive.
pub const PRIMITIVE_FLAGS: u8 = 58;

const NO: u32 = u32::MAX;
const EVEN_BITS_MASK: u64 = 0x5555_5555_5555_5555;

/// The work register count of a program allocated in the split file.
pub const WORK_REGS_SPLIT: u8 = 32;

fn temp(p: &mut Prog<'_>) -> Result<u16, MaliError> {
    Ok(p.temp()?)
}

/// `va_lower_split_64bit` (va_lower_split_64bit.c:17-91).
fn lower_split_64bit(p: &mut Prog<'_>) -> Result<(), MaliError> {
    /* producers, recorded before the pass adds any */
    let mut lut_op = [None::<Op>; MAX_NODES];
    let mut lut_src0 = [Src::NULL; MAX_NODES];
    for i in p.as_slice() {
        for d in i.dests() {
            if let Idx::Ssa(v, _) = d.idx {
                lut_op[v as usize] = Some(i.op);
                lut_src0[v as usize] = i.src[0];
            }
        }
    }
    let mut k = 0;
    while k < p.n {
        let mut added = 0;
        for s in 0..(p.ins[k].ns as usize).min(4) {
            let i = p.ins[k + added];
            if i.src[s].idx == Idx::Null {
                continue;
            }
            if !(bir::src_info(i.op, s).bits == 64 && i.count_read(s) == 1) {
                continue;
            }
            let (a, b) = (i.src[s], i.src[s + 1]);
            /* already a FAU pair */
            let pair = match (a.idx, b.idx) {
                (Idx::Special(x, false), Idx::Special(y, true)) => x == y,
                (Idx::Uniform(x), Idx::Uniform(y)) => x % 2 == 0 && y == x + 1,
                _ => false,
            };
            if pair {
                continue;
            }
            if let (Idx::Ssa(va, _), Idx::Ssa(vb, _)) = (a.idx, b.idx) {
                if lut_op[va as usize] == Some(Op::Split) && same_producer(p, va, vb) {
                    let split_src = lut_src0[va as usize];
                    match split_src.idx {
                        Idx::Ssa(w, _) if lut_op[w as usize] != Some(Op::Collect) => {}
                        _ => continue,
                    }
                }
            }
            let vec = temp(p)?;
            let collect = Ins::new(Op::Collect, &[Dest::ssa(vec)], &[a, b]);
            let (t0, t1) = (temp(p)?, temp(p)?);
            let split = Ins::new(Op::Split, &[Dest::ssa(t0), Dest::ssa(t1)], &[Src::ssa(vec)]);
            let at = k + added;
            p.insert_in(at, collect, i.block)?;
            p.insert_in(at + 1, split, i.block)?;
            added += 2;
            let i = &mut p.ins[k + added];
            i.src[s] = Src { idx: Idx::Ssa(t0, 0), ..a };
            i.src[s + 1] = Src { idx: Idx::Ssa(t1, 0), ..b };
        }
        k += added + 1;
    }
    Ok(())
}

/// Whether two values are dests of the same SPLIT.
fn same_producer(p: &Prog<'_>, a: u16, b: u16) -> bool {
    p.as_slice().iter().any(|i| {
        i.op == Op::Split
            && i.dests().iter().any(|d| d.idx == Idx::Ssa(a, 0))
            && i.dests().iter().any(|d| d.idx == Idx::Ssa(b, 0))
    })
}

/// `bi_liveness_ins_update_ra` (bi_ra.c:198-213): per value, the words live.
fn live_update(live: &mut [u8], i: &Ins) {
    for (d, x) in i.dests().iter().enumerate() {
        if let Idx::Ssa(v, _) = x.idx {
            live[v as usize] &= !(i.writemask(d) as u8);
        }
    }
    for (s, x) in i.srcs().iter().enumerate() {
        if let Idx::Ssa(v, off) = x.idx {
            live[v as usize] |= (((1u32 << i.count_read(s)) - 1) << off) as u8;
        }
    }
}

/// `bi_lower_vector` (bi_ra.c:819-895): a SPLIT's destinations below
/// `first_reg` (SSA values, not a phi's register) read the source's words
/// directly; the dead moves of each block go, from its live-out set.
fn lower_vector(p: &mut Prog<'_>, first_reg: u16, l: &mut LiveRa<'_>) -> Result<(), MaliError> {
    let mut remap = [None::<(u16, u8)>; MAX_NODES];
    let mut k = 0;
    while k < p.n {
        let i = p.ins[k];
        match i.op {
            Op::Split => {
                let Idx::Ssa(src, 0) = i.src[0].idx else {
                    return Err(MaliError::Defect(super::Defect::Vector));
                };
                p.remove(k);
                for (d, x) in i.dests().iter().enumerate() {
                    let mov = Ins::new(Op::MovI32, &[*x], &[Src::of(Idx::Ssa(src, d as u8))]);
                    p.insert_in(k + d, mov, i.block)?;
                    if let Idx::Ssa(v, _) = x.idx {
                        if v < first_reg {
                            remap[v as usize] = Some((src, d as u8));
                        }
                    }
                }
                k += i.nd as usize;
            }
            Op::Collect => {
                let Idx::Ssa(dst, 0) = i.dest[0].idx else {
                    return Err(MaliError::Defect(super::Defect::Vector));
                };
                p.remove(k);
                let mut at = k;
                for (s, x) in i.srcs().iter().enumerate() {
                    if x.idx == Idx::Null {
                        continue;
                    }
                    let d = Dest { idx: Idx::Ssa(dst, s as u8), swz: bir::Swz::H01 };
                    p.insert_in(at, Ins::new(Op::MovI32, &[d], &[*x]), i.block)?;
                    at += 1;
                }
                k = at;
            }
            _ => k += 1,
        }
    }
    for i in 0..p.n {
        for s in 0..p.ins[i].ns as usize {
            if let Idx::Ssa(v, _) = p.ins[i].src[s].idx {
                if v < first_reg {
                    if let Some((w, off)) = remap[v as usize] {
                        p.ins[i].src[s].idx = Idx::Ssa(w, off);
                    }
                }
            }
        }
    }
    /* the dead moves, per block from its successors' live-in (:868-895) */
    liveness_ra(p, l);
    let mut live = [0u8; MAX_NODES];
    for b in (0..p.nblocks).rev() {
        if p.blocks[b as usize].dead {
            continue;
        }
        live.copy_from_slice(l.live_out(b));
        let mut k = p.block_end(b);
        let start = p.block_start(b);
        while k > start {
            k -= 1;
            let i = p.ins[k];
            let all_null = i.dests().iter().enumerate().all(|(d, x)| match x.idx {
                Idx::Ssa(v, _) => live[v as usize] as u32 & i.writemask(d) == 0,
                _ => true,
            });
            if all_null && !i.side_effects() {
                p.remove(k);
            } else {
                live_update(&mut live, &i);
            }
        }
    }
    Ok(())
}

/// `squeeze_index` (bi_ra.c:957-977).
fn squeeze(p: &mut Prog<'_>) -> u16 {
    let mut map = [0u16; MAX_NODES];
    let mut alloc = 0u16;
    let mut get = |map: &mut [u16; MAX_NODES], v: u16| -> u16 {
        if map[v as usize] == 0 {
            alloc += 1;
            map[v as usize] = alloc;
        }
        map[v as usize] - 1
    };
    for k in 0..p.n {
        for d in 0..p.ins[k].nd as usize {
            if let Idx::Ssa(v, off) = p.ins[k].dest[d].idx {
                p.ins[k].dest[d].idx = Idx::Ssa(get(&mut map, v), off);
            }
        }
        for s in 0..p.ins[k].ns as usize {
            if let Idx::Ssa(v, off) = p.ins[k].src[s].idx {
                p.ins[k].src[s].idx = Idx::Ssa(get(&mut map, v), off);
            }
        }
    }
    p.ssa_alloc = alloc;
    alloc
}

struct Lcra<'s> {
    n: usize,
    affinity: &'s mut [u64],
    /// `linear[i * n + j]`: the forbidden `solution[i] - solution[j] + 7`.
    linear: &'s mut [u16],
    solutions: &'s mut [u32],
}

impl Lcra<'_> {
    /// `lcra_add_node_interference` (bi_ra.c:70-99).
    fn interfere(&mut self, i: usize, cmask_i: u32, j: usize, cmask_j: u32) {
        if i == j {
            return;
        }
        let (mut fw, mut bw) = (0u16, 0u16);
        for d in 0..8 {
            if cmask_i & (cmask_j << d) != 0 {
                fw |= 1 << (7 + d);
                bw |= 1 << (7 - d);
            }
            if cmask_i & (cmask_j >> d) != 0 {
                bw |= 1 << (7 + d);
                fw |= 1 << (7 - d);
            }
        }
        self.linear[j * self.n + i] |= fw;
        self.linear[i * self.n + j] |= bw;
    }

    /// `lcra_test_linear` (bi_ra.c:101-142).
    fn test(&self, i: usize) -> bool {
        let c = self.solutions[i] as i64;
        for j in 0..self.n {
            if self.solutions[j] == NO {
                continue;
            }
            let lhs = c - self.solutions[j] as i64;
            if !(-7..=7).contains(&lhs) {
                continue;
            }
            if self.linear[i * self.n + j] & (1 << (lhs + 7)) != 0 {
                return false;
            }
        }
        true
    }

    /// `lcra_solve` (bi_ra.c:144-172).
    fn solve(&mut self) -> bool {
        for step in 0..self.n {
            if self.solutions[step] != NO || self.affinity[step] == 0 {
                continue;
            }
            let mut ok = false;
            for r in 0..64 {
                if self.affinity[step] >> r & 1 == 0 {
                    continue;
                }
                self.solutions[step] = r;
                if self.test(step) {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return false;
            }
        }
        true
    }
}

/// `bi_make_affinity` (bi_ra.c:296-320).
fn make_affinity(clobber: u64, count: u32, split: bool) -> u64 {
    let mut clobbered = 0u64;
    for i in 0..count {
        clobbered |= clobber >> i;
    }
    if count > 1 {
        let excess = count - 1;
        let mask = (1u64 << excess) - 1;
        clobbered |= mask << (64 - excess);
        if split {
            clobbered |= mask << (16 - excess);
        }
    }
    if split {
        clobbered |= 0xffff_ffffu64 << 16;
    }
    !clobbered
}

/// `bi_postra_liveness_ins` (bi_opt_dce.c:64-84): hardware registers.
pub fn postra_live(mut live: u64, i: &Ins) -> u64 {
    for (d, x) in i.dests().iter().enumerate() {
        if let Idx::Reg(r) = x.idx {
            live &= !(((1u64 << i.count_write(d)) - 1) << r);
        }
    }
    for (s, x) in i.srcs().iter().enumerate() {
        if let Idx::Reg(r) = x.idx {
            live |= ((1u64 << i.count_read(s)) - 1) << r;
        }
    }
    live
}

/// `bi_register_allocate` for this back end's programs. Returns the work
/// register count.
pub fn allocate(p: &mut Prog<'_>, scratch: &Arena<'_>) -> Result<u8, MaliError> {
    allocate_in(p, scratch, false)
}

/// [`allocate`] for a blend shader (`inputs->is_blend`, blend.rs): r0-r15
/// only ("Blend shaders are restricted to R0-R15", bi_ra.c:431-437), its
/// BLEND neither precoloured nor clobbering (:390, :450), and the moves
/// from the link register and the coverage free to coalesce into them
/// (:507-513).
pub fn allocate_blend(p: &mut Prog<'_>, scratch: &Arena<'_>) -> Result<u8, MaliError> {
    allocate_in(p, scratch, true)
}

fn allocate_in(p: &mut Prog<'_>, scratch: &Arena<'_>, blend: bool) -> Result<u8, MaliError> {
    lower_split_64bit(p)?;
    let first_reg = out_of_ssa(p)?;
    let mut lr = LiveRa::new(scratch)?;
    lower_vector(p, first_reg, &mut lr)?;
    let n = squeeze(p) as usize;
    let oom = |_| MaliError::OutOfScratch;
    let mut l = Lcra {
        n,
        affinity: scratch.alloc_filled(n, 0u64).map_err(oom)?,
        linear: scratch.alloc_filled(n * n, 0u16).map_err(oom)?,
        solutions: scratch.alloc_filled(n, NO).map_err(oom)?,
    };
    /* the split file: r0-r15 and r48-r63 (bi_ra.c:435-438); a blend
     * shader r0-r15 */
    let default_affinity = if blend { 0xffffu64 } else { 0xffffu64 | (0xffffu64 << 48) };
    for i in p.as_slice() {
        for d in i.dests() {
            if let Idx::Ssa(v, _) = d.idx {
                l.affinity[v as usize] = default_affinity;
            }
        }
        if let (Op::Blend { .. }, false) = (i.op, blend) {
            if let Idx::Ssa(v, _) = i.src[0].idx {
                l.solutions[v as usize] = BLEND_SRC0_C0 as u32;
            }
            if let Idx::Ssa(v, _) = i.src[4].idx {
                return Err(MaliError::Defect(super::Defect::DualSource(v)));
            }
            if let Idx::Ssa(v, _) = i.dest[0].idx {
                l.solutions[v as usize] = BLEND_LINK as u32;
            }
        }
        if matches!(i.op, Op::Atest | Op::ZsEmit { .. }) {
            if let Idx::Ssa(v, _) = i.dest[0].idx {
                l.solutions[v as usize] = CUMULATIVE_COVERAGE as u32;
            }
        }
        if i.op == Op::Atest {
            if let Idx::Ssa(v, _) = i.src[0].idx {
                l.solutions[v as usize] = CUMULATIVE_COVERAGE as u32;
            }
        }
    }
    /* bi_compute_interference (bi_ra.c:410-424): each block in reverse,
     * from its live-out set (bi_mark_interference, :322-408) */
    liveness_ra(p, &mut lr);
    let (_, reg_out) = postra_liveness(p);
    let mut live = [0u8; MAX_NODES];
    for b in (0..p.nblocks).rev().filter(|&b| !p.blocks[b as usize].dead) {
        live.copy_from_slice(lr.live_out(b));
        let mut preload_live = reg_out[b as usize];
        for i in p.as_slice()[p.block_start(b)..p.block_end(b)].iter().rev() {
            for (d, x) in i.dests().iter().enumerate() {
                let Idx::Ssa(node, offset) = x.idx else { continue };
                let count = i.count_write(d);
                let mut aff = make_affinity(preload_live, count, true) >> offset;
                if count >= 2 || offset != 0 {
                    aff &= EVEN_BITS_MASK;
                }
                l.affinity[node as usize] &= aff;
                for j in 0..n {
                    let mut r = live[j] as u32;
                    if i.op == Op::MovI32 {
                        if let Idx::Ssa(sv, soff) = i.src[0].idx {
                            if sv as usize == j {
                                r &= !(1 << soff);
                            }
                        }
                    }
                    if r != 0 {
                        l.interfere(node as usize, i.writemask(d), j, r);
                    }
                }
                if d == 1 {
                    if let Idx::Ssa(first, _) = i.dest[0].idx {
                        l.interfere(node as usize, i.writemask(1), first as usize, i.writemask(0));
                    }
                }
            }
            for (s, x) in i.srcs().iter().enumerate() {
                let Idx::Ssa(v, _) = x.idx else { continue };
                if i.count_read(s) >= 2 || (s < bir::nr_srcs(i.op) && bir::src_info(i.op, s).bits > 32) {
                    l.affinity[v as usize] &= EVEN_BITS_MASK;
                }
            }
            if let (Op::Blend { .. }, false) = (i.op, blend) {
                let clobber = 0xffffu64 | (1u64 << BLEND_LINK);
                for j in 0..n {
                    if live[j] != 0 {
                        l.affinity[j] &= !clobber;
                    }
                }
            }
            preload_live = postra_live(preload_live, i);
            live_update(&mut live, i);
        }
    }
    /* coalesce the moves from preloaded registers (bi_ra.c:490-522) */
    for k in 0..p.n {
        let i = p.ins[k];
        let (Op::MovI32, Idx::Reg(reg), Idx::Ssa(node, _)) = (i.op, i.src[0].idx, i.dest[0].idx) else { continue };
        let node = node as usize;
        if l.solutions[node] != NO {
            continue;
        }
        let mut affinity = l.affinity[node];
        if blend {
            affinity |= (1u64 << BLEND_LINK) | (1u64 << CUMULATIVE_COVERAGE);
        }
        if affinity >> reg & 1 != 0 {
            l.solutions[node] = reg as u32;
            if !l.test(node) {
                l.solutions[node] = NO;
            }
        }
    }
    if !l.solve() {
        return Err(MaliError::Spill);
    }
    /* bi_install_registers (bi_ra.c:577-590) */
    let reg = |idx: Idx| -> Result<Idx, MaliError> {
        match idx {
            Idx::Ssa(v, off) => {
                let s = l.solutions[v as usize];
                if s == NO {
                    return Err(MaliError::Defect(super::Defect::Unallocated));
                }
                Ok(Idx::Reg((s + off as u32) as u8))
            }
            other => Ok(other),
        }
    };
    for k in 0..p.n {
        for d in 0..p.ins[k].nd as usize {
            p.ins[k].dest[d].idx = reg(p.ins[k].dest[d].idx)?;
        }
        for s in 0..p.ins[k].ns as usize {
            p.ins[k].src[s].idx = reg(p.ins[k].src[s].idx)?;
        }
    }
    /* bi_opt_post_ra (bifrost_compile.c:4200-4208) */
    let mut k = 0;
    while k < p.n {
        let i = p.ins[k];
        if i.op == Op::MovI32 && i.dest[0].idx == i.src[0].idx {
            p.remove(k);
        } else {
            k += 1;
        }
    }
    Ok(WORK_REGS_SPLIT)
}
