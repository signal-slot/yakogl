// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2022 Collabora Ltd.
// SPDX-FileCopyrightText: 2026 Arm Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/valhall/
// va_insert_flow.c, va_merge_flow.c, va_mark_last.c and
// bi_helper_invocations.c (MIT). The upstream part stays under its licence
// (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! Message slots, flow control and last-use marks, after RA, over the
//! blocks: `va_assign_slots`, `va_insert_flow_control_nops`,
//! `va_merge_flow` and `va_mark_last`, in Mesa's order
//! (bifrost_compile.c:4589-4594).
//!
//! The rules (va_insert_flow.c:22-41): an instruction waits for the slot of
//! an asynchronous instruction whose staging registers it reads or writes;
//! memory accesses are serialised; ATEST waits for slots 0, 1, 2 and 6
//! before it and slot 0 after it; BLEND waits for everything; a varying
//! load is waited for at the end of its block; a block that ends the
//! program ends with `.end`, a block whose branches may diverge (a
//! conditional branch, or a fall-through into a join) with `.reconverge`.
//! A fragment shader that needs no helper invocations discards them at the
//! start; one that does (a texture look-up with a computed level of
//! detail) discards them after the last instruction that needs them, in
//! every block after which no successor needs them
//! (`bi_analyze_helper_terminate`, bi_helper_invocations.c). The
//! scoreboard and the register liveness flow across the blocks' edges.

use super::bir::{flow, props, Idx, Ins, Msg, Op, Prog, MAX_BLOCKS, MAX_NODES};
use super::ra::{postra_live, postra_liveness};
use super::MaliError;

/// `bi_instr_uses_helpers` (bi_helper_invocations.c) for the ops here:
/// a look-up whose level of detail comes from the derivatives (a level-0
/// one does not).
pub fn uses_helpers(i: &Ins) -> bool {
    matches!(i.op, Op::TexSingle { lod: super::bir::LodMode::Computed, .. })
}

/// `bi_analyze_helper_requirements` (bi_helper_invocations.c) over the
/// blocks, in SSA form (after the constants, before the scheduler,
/// bifrost_compile.c:4548): the values a helper-using instruction reads,
/// and the value a conditional branch tests
/// (`bi_add_branch_compare_values`), and every value they are computed
/// from, are needed by helper invocations; a look-up whose result is none
/// of them sets `skip`.
pub fn analyze_helper_requirements(p: &mut Prog<'_>) {
    let mut deps = [false; MAX_NODES];
    let ssa_srcs = |i: &Ins| {
        let mut v = [None::<u16>; super::bir::MAX_SRCS];
        for (k, x) in i.srcs().iter().enumerate() {
            v[k] = x.ssa_value();
        }
        v
    };
    for i in p.as_slice() {
        if uses_helpers(i) || matches!(i.op, Op::Branchz { .. }) {
            for v in ssa_srcs(i).into_iter().flatten() {
                deps[v as usize] = true;
            }
        }
    }
    let mut progress = true;
    while progress {
        progress = false;
        for i in p.as_slice().iter().rev() {
            if !i.dests().iter().any(|d| matches!(d.idx, Idx::Ssa(v, _) if deps[v as usize])) {
                continue;
            }
            for v in ssa_srcs(i).into_iter().flatten() {
                progress |= !deps[v as usize];
                deps[v as usize] = true;
            }
        }
    }
    for i in p.ins[..p.n].iter_mut() {
        let exec = i.dests().iter().any(|d| matches!(d.idx, Idx::Ssa(v, _) if deps[v as usize]));
        /* `TEX_FETCH` has no skip bit (`bi_has_skip_bit`) */
        if let Op::TexSingle { ref mut skip, lod, .. } = i.op {
            *skip = !exec && lod != super::bir::LodMode::Fetch;
        }
    }
}

/// The general slots (`VA_NUM_GENERAL_SLOTS`).
const SLOTS: usize = 3;

/// `va_assign_slots` (va_insert_flow.c:515-532).
pub fn assign_slots(p: &mut Prog<'_>) {
    let mut counter = 0u8;
    for i in p.ins[..p.n].iter_mut() {
        if matches!(i.op, Op::Atest | Op::ZsEmit { .. }) {
            i.slot = 0;
        } else if props(i.op).msg != Msg::None {
            i.slot = counter;
            counter += 1;
            if counter == 3 {
                counter = 0;
            }
        }
    }
}

fn reg_mask(r: Idx, count: u32) -> u64 {
    match r {
        Idx::Reg(r) => ((1u64 << count) - 1) << r,
        _ => 0,
    }
}

/// `bi_read_mask` / `bi_write_mask` (va_insert_flow.c:56-88).
fn read_mask(i: &Ins) -> u64 {
    i.srcs().iter().enumerate().fold(0, |m, (s, x)| m | reg_mask(x.idx, i.count_read(s)))
}

fn write_mask(i: &Ins) -> u64 {
    i.dests().iter().enumerate().fold(0, |m, (d, x)| m | reg_mask(x.idx, i.count_write(d)))
}

/// `bi_is_memory_access` (va_insert_flow.c:101-125) for the ops here.
fn is_memory_access(i: &Ins) -> bool {
    props(i.op).msg == Msg::Store
}

/// `struct bi_scoreboard_state`: per slot the registers an asynchronous
/// instruction will write, the slots with a varying load or a memory
/// access pending, and (for `va_mark_last`) the staging registers read.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Scoreboard {
    write: [u64; SLOTS],
    read: [u64; SLOTS],
    varying: u8,
    memory: u8,
}

impl Scoreboard {
    fn pop(&mut self, slot: usize) -> u8 {
        self.write[slot] = 0;
        self.varying &= !(1 << slot);
        self.memory &= !(1 << slot);
        1 << slot
    }

    /// `pending_in[s] = sum { p in pred[s] } ( pending_out[p] )`.
    fn union(&mut self, o: &Scoreboard) {
        for s in 0..SLOTS {
            self.write[s] |= o.write[s];
            self.read[s] |= o.read[s];
        }
        self.varying |= o.varying;
        self.memory |= o.memory;
    }
}

/// `bi_reconverge_branches` (bir.c:230-236).
fn reconverge_branches(p: &Prog<'_>, b: u8) -> bool {
    let blk = &p.blocks[b as usize];
    match (blk.nsucc(), blk.succ[0]) {
        (1, Some(s)) => p.blocks[s as usize].npred > 1,
        _ => true,
    }
}

/// `bi_analyze_helper_terminate` (bi_helper_invocations.c:159-181): the
/// blocks that need helper invocations at their end (`pass_flags`): one
/// that uses them, and every predecessor of such a block.
fn helper_terminate(p: &Prog<'_>) -> [bool; MAX_BLOCKS] {
    let mut pass = [false; MAX_BLOCKS];
    for b in (0..p.nblocks).rev().filter(|&b| !p.blocks[b as usize].dead) {
        if pass[b as usize] || !(p.block_start(b)..p.block_end(b)).any(|k| uses_helpers(&p.ins[k])) {
            continue;
        }
        /* bi_propagate_pass_flag: depth first over the predecessors */
        let mut stack = [0u8; MAX_BLOCKS];
        let mut sp = 1;
        stack[0] = b;
        while sp > 0 {
            sp -= 1;
            let x = stack[sp];
            pass[x as usize] = true;
            for &pr in p.blocks[x as usize].preds() {
                if !pass[pr as usize] && sp < stack.len() {
                    stack[sp] = pr;
                    sp += 1;
                }
            }
        }
    }
    pass
}

/// `bi_block_terminates_helpers` (:145-156): no successor needs helpers.
fn terminates_helpers(p: &Prog<'_>, pass: &[bool; MAX_BLOCKS], b: u8) -> bool {
    !p.blocks[b as usize].succs().any(|s| pass[s as usize])
}

/// `va_discard_before_block` (va_insert_flow.c:346-368): a block that
/// needs no helpers, with one predecessor some other successor of which
/// does.
fn discard_before(p: &Prog<'_>, pass: &[bool; MAX_BLOCKS], b: u8) -> bool {
    let blk = &p.blocks[b as usize];
    if pass[b as usize] || blk.npred != 1 {
        return false;
    }
    p.blocks[blk.pred[0] as usize].succs().any(|s| pass[s as usize])
}

/// `va_insert_flow_control_nops` (va_insert_flow.c:387-506) on a program
/// that is not empty: the scoreboard's forward dataflow over the blocks
/// (`va_assign_scoreboard`, :280-302, one pass in layout order, which is
/// topological without loops), then per block the discard at its start,
/// the waits before and after the instructions that need them, the
/// discard after the last helper use, and `.end` or `.reconverge` at its
/// end.
pub fn insert_flow_control_nops(p: &mut Prog<'_>, fragment: bool) -> Result<(), MaliError> {
    if p.n == 0 {
        return Ok(());
    }
    /* va_assign_scoreboard: scoreboard_block_update (:219-278) */
    let mut sb_out = [Scoreboard::default(); MAX_BLOCKS];
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        let mut st = Scoreboard::default();
        for &pr in p.blocks[b as usize].preds() {
            st.union(&sb_out[pr as usize]);
        }
        for k in p.block_start(b)..p.block_end(b) {
            let i = p.ins[k];
            let mut f = 0u8;
            let regs = read_mask(&i) | write_mask(&i);
            for slot in 0..SLOTS {
                if st.write[slot] & regs != 0 {
                    f |= st.pop(slot);
                }
            }
            /* LD_VAR_BUF_IMM with .store writes the hidden register */
            if props(i.op).msg == Msg::Varying {
                for slot in 0..SLOTS {
                    if st.varying & (1 << slot) != 0 {
                        f |= st.pop(slot);
                    }
                }
            }
            if is_memory_access(&i) {
                for slot in 0..SLOTS {
                    if st.memory & (1 << slot) != 0 {
                        f |= st.pop(slot);
                    }
                }
            }
            p.ins[k].flow |= f;
            /* bi_push_instr */
            if props(i.op).sr_write {
                st.write[i.slot as usize] |= write_mask(&i);
            }
            if is_memory_access(&i) {
                st.memory |= 1 << i.slot;
            }
            if props(i.op).msg == Msg::Varying {
                st.varying |= 1 << i.slot;
            }
        }
        if st.varying != 0 {
            let mut f = 0u8;
            for slot in 0..SLOTS {
                if st.varying & (1 << slot) != 0 {
                    f |= st.pop(slot);
                }
            }
            p.insert_end(b, Ins::nop(f))?;
        }
        sb_out[b as usize] = st;
    }
    let pass = if fragment { helper_terminate(p) } else { [false; MAX_BLOCKS] };
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        /* the discards along control flow edges */
        if fragment && discard_before(p, &pass, b) {
            let at = p.block_start(b);
            p.insert_in(at, Ins::nop(flow::DISCARD), b)?;
        }
        /* the per-instruction NOPs */
        let mut k = p.block_start(b);
        while k < p.block_end(b) {
            let i = p.ins[k];
            let mut at = k;
            match i.op {
                /* "the required waits are assumed to already have happened
                 * in blend shaders" (:421-434): a blend shader (compiled with
                 * `fragment` false, blend.rs) waits before neither its BLEND
                 * nor its LD_TILE; no fragment shader here has an LD_TILE */
                Op::Blend { .. } if fragment => {
                    p.insert(at, Ins::nop(flow::WAIT))?;
                    at += 1;
                }
                Op::Atest => {
                    p.insert(at, Ins::nop(flow::WAIT0126))?;
                    at += 1;
                    p.insert_after(at, Ins::nop(flow::WAIT0))?;
                }
                /* not a blend shader: this back end compiles none */
                Op::ZsEmit { .. } => {
                    p.insert(at, Ins::nop(flow::WAIT0126))?;
                    at += 1;
                }
                _ => {}
            }
            if p.ins[at].flow != 0 && p.ins[at].op != Op::Nop {
                if p.ins[at].flow as usize >= 1 << SLOTS {
                    return Err(MaliError::Defect(super::Defect::Flow));
                }
                let f = p.ins[at].flow;
                p.insert(at, Ins::nop(f))?;
                at += 1;
                p.ins[at].flow = 0;
            }
            k = at + 1;
            if i.op == Op::Atest {
                k += 1;
            }
        }
        /* terminate helpers after the last use (:470-480) */
        if fragment && pass[b as usize] && terminates_helpers(p, &pass, b) {
            let (start, end) = (p.block_start(b), p.block_end(b));
            if let Some(k) = (start..end).rev().find(|&k| uses_helpers(&p.ins[k])) {
                p.insert_after(k, Ins::nop(flow::DISCARD))?;
            }
        }
        /* end execution after the block, or reconverge (:482-494) */
        let blk = p.blocks[b as usize];
        if blk.nsucc() == 0 || blk.needs_nop {
            if b == 0 || blk.npred > 0 {
                p.insert_end(b, Ins::nop(flow::END))?;
            }
        } else if reconverge_branches(p, b) && p.block_end(b) > p.block_start(b) {
            p.insert_end(b, Ins::nop(flow::RECONVERGE))?;
        }
    }
    /* helpers unused anywhere: terminated at the start (:497-505) */
    if fragment && !pass[0] {
        p.insert_in(0, Ins::nop(flow::DISCARD), 0)?;
    }
    Ok(())
}

/// `union_waits` (va_merge_flow.c:101-112).
fn union_waits(x: u8, y: u8) -> u8 {
    if x == flow::WAIT || y == flow::WAIT {
        flow::WAIT
    } else if x == flow::WAIT0126 || y == flow::WAIT0126 {
        flow::WAIT0126
    } else {
        x | y
    }
}

/// `va_merge_flow` (va_merge_flow.c:224-241) on each block of two or more
/// instructions.
pub fn merge_flow(p: &mut Prog<'_>, fragment: bool) {
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead || p.block_end(b) < p.block_start(b) + 2 {
            continue;
        }
        merge_end_reconverge(p, b);
        /* merge_resource_waits (:146-173): none is ever inserted here */
        merge_waits(p, b);
        if fragment {
            merge_discard(p, b);
        }
    }
}

/// `merge_end_reconverge` (:59-92): a block's final `NOP.end` or
/// `NOP.reconverge` joins the instruction before it, after the end's
/// blocking NOPs (other than a `.wait` on the barrier slot) go.
fn merge_end_reconverge(p: &mut Prog<'_>, b: u8) {
    let Some(last) = p.last_of(b) else { return };
    let f = p.ins[last].flow;
    if p.ins[last].op != Op::Nop || (f != flow::RECONVERGE && f != flow::END) {
        return;
    }
    let start = p.block_start(b);
    let mut penult = last - 1;
    if f == flow::END {
        loop {
            if p.ins[penult].op == Op::Nop && p.ins[penult].flow != flow::WAIT {
                p.remove(penult);
                if penult == start {
                    return;
                }
                penult -= 1;
                continue;
            }
            break;
        }
    }
    if p.ins[penult].flow != flow::NONE {
        return;
    }
    p.ins[penult].flow = f;
    p.remove(penult + 1);
}

/// `merge_waits` (:114-144).
fn merge_waits(p: &mut Prog<'_>, b: u8) {
    let mut last_free: Option<usize> = None;
    let mut k = p.block_start(b);
    while k < p.block_end(b) {
        let i = p.ins[k];
        if let Some(f) = last_free {
            if i.op == Op::Nop && flow::is_wait_or_none(i.flow) {
                p.ins[f].flow = union_waits(p.ins[f].flow, i.flow);
                p.remove(k);
                continue;
            }
        }
        if props(i.op).msg != Msg::None {
            last_free = None;
        }
        if flow::is_wait_or_none(i.flow) {
            last_free = Some(k);
        }
        k += 1;
    }
}

/// `merge_discard` (:181-222), walking backwards: a `NOP.discard` joins
/// the instruction before it, else the next one with no flow control, and
/// at the end of the program goes.
fn merge_discard(p: &mut Prog<'_>, b: u8) {
    let start = p.block_start(b);
    let mut last_free: Option<usize> = None;
    let mut k = p.block_end(b);
    while k > start {
        k -= 1;
        let i = p.ins[k];
        if i.op == Op::Nop && i.flow == flow::DISCARD {
            if k > start && p.ins[k - 1].flow == flow::NONE {
                p.ins[k - 1].flow = flow::DISCARD;
                p.remove(k);
                last_free = last_free.map(|f| f - 1);
                continue;
            }
            if let Some(f) = last_free {
                p.ins[f].flow = flow::DISCARD;
                p.remove(k);
                last_free = Some(f - 1);
                continue;
            }
            if p.blocks[b as usize].nsucc() == 0 {
                p.remove(k);
                last_free = last_free.map(|f| f - 1);
                continue;
            }
        }
        if i.flow == flow::NONE {
            last_free = Some(k);
        }
    }
}

/// `waits_on_slot` (va_mark_last.c:66-71).
fn waits_on_slot(f: u8, slot: usize) -> bool {
    f == flow::WAIT || f == flow::WAIT0126 || (flow::is_wait_or_none(f) && f & (1 << slot) != 0)
}

/// The registers a staging source reads (`bi_staging_read_mask`).
fn staging_read_mask(i: &Ins) -> u64 {
    i.srcs()
        .iter()
        .enumerate()
        .filter(|(s, x)| i.is_staging_src(*s) && x.idx != Idx::Null)
        .fold(0, |m, (s, x)| m | reg_mask(x.idx, i.count_read(s)))
}

/// `scoreboard_update` (va_mark_last.c:73-84).
fn reads_update(read: &mut [u64; SLOTS], i: &Ins) {
    read[i.slot as usize] |= staging_read_mask(i);
    for (slot, r) in read.iter_mut().enumerate() {
        if waits_on_slot(i.flow, slot) {
            *r = 0;
        }
    }
}

/// Whether `i` writes register `reg` (`bi_writes_reg`).
fn writes_reg(i: &Ins, reg: u8) -> bool {
    i.dests().iter().enumerate().any(|(d, x)| match x.idx {
        Idx::Reg(r) => reg >= r && ((reg - r) as u32) < i.count_write(d),
        _ => false,
    })
}

/// `va_mark_last` (va_mark_last.c:127-194): the register liveness and the
/// staging reads pending (`va_analyze_scoreboard_reads`, :86-125, one
/// forward pass in layout order) flow across the blocks; then each block
/// from its live-out set.
pub fn mark_last(p: &mut Prog<'_>) {
    let (_, reg_out) = postra_liveness(p);
    let mut read_out = [[0u64; SLOTS]; MAX_BLOCKS];
    let mut read_in = [[0u64; SLOTS]; MAX_BLOCKS];
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        let mut st = [0u64; SLOTS];
        for &pr in p.blocks[b as usize].preds() {
            for s in 0..SLOTS {
                st[s] |= read_out[pr as usize][s];
            }
        }
        read_in[b as usize] = st;
        for k in p.block_start(b)..p.block_end(b) {
            reads_update(&mut st, &p.ins[k]);
        }
        read_out[b as usize] = st;
    }
    for b in 0..p.nblocks {
        if p.blocks[b as usize].dead {
            continue;
        }
        let (start, end) = (p.block_start(b), p.block_end(b));
        let mut live = reg_out[b as usize];
        for k in (start..end).rev() {
            let i = p.ins[k];
            for s in 0..i.ns as usize {
                let Idx::Reg(r) = i.src[s].idx else { continue };
                let mask = ((1u64 << i.count_read(s)) - 1) << r;
                let mut discard = live & mask == 0;
                discard |= writes_reg(&i, r);
                p.ins[k].src[s].discard = discard;
            }
            live = postra_live(live, &i);
        }
        let mut read = read_in[b as usize];
        for k in start..end {
            let i = p.ins[k];
            for s in 0..i.ns as usize {
                if !p.ins[k].src[s].discard {
                    continue;
                }
                let Idx::Reg(r) = i.src[s].idx else { continue };
                let pending = (read[0] | read[1] | read[2]) >> r & 1 != 0;
                if i.is_staging_src(s) || pending {
                    p.ins[k].src[s].discard = false;
                }
            }
            for s in 0..(i.ns as usize).min(3) {
                if super::bir::src_info(i.op, s).bits == 64 && i.count_read(s) == 1 && s + 1 < i.ns as usize {
                    let both = p.ins[k].src[s].discard && p.ins[k].src[s + 1].discard;
                    p.ins[k].src[s].discard = both;
                    p.ins[k].src[s + 1].discard = both;
                }
            }
            reads_update(&mut read, &p.ins[k]);
        }
    }
}

/// The registers live at the start: what the hardware must preload
/// (bifrost_compile.c:4685-4701: the first block's `reg_live_in`, with
/// r60 and r61 always preloaded for a fragment shader on Valhall).
pub fn preload_mask(p: &Prog<'_>, fragment: bool) -> u64 {
    let (reg_in, _) = postra_liveness(p);
    let mut live = reg_in[0];
    if fragment {
        live |= (1 << 60) | (1 << 61);
    }
    live
}
