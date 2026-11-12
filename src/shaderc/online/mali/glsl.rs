// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2010-2014 Intel Corporation
// Derived from Mesa 26.2.1 src/compiler/glsl/opt_rebalance_tree.cpp and
// opt_algebraic.cpp (MIT). The upstream part stays under its licence
// (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! The two passes of Mesa's GLSL compiler that change the shape of an
//! expression tree before NIR sees it, on the front end's IR: what
//! `do_common_optimization` (glsl_parser_extras.cpp:2597-2612, run once at
//! compile time, :2291) does to the trees the Mali back end takes, in its
//! order.
//!
//! 1. `do_rebalance_tree` (opt_rebalance_tree.cpp): a tree of three or more
//!    additions (or multiplications) of one type with no matrix, no array
//!    or record dereference and at most one constant is turned into a vine
//!    and back (`tree_to_vine`, `vine_to_tree`: the DSW algorithm,
//!    transcribed in [`dsw`]). The visitor is an rvalue enter visitor whose
//!    `visit_enter(ir_assignment)` does not handle the right-hand side
//!    itself (:151-163): a whole assignment's right-hand side is no
//!    candidate, its operands are, and so is every other rvalue, a
//!    constructor's lane among them (`vec_ctor.x = swizzle(arg)`,
//!    ast_function.cpp:1619-1632).
//! 2. `do_algebraic` (opt_algebraic.cpp), of which only
//!    `reassociate_constant` (:175-216, :276-284) changes these trees: for
//!    `op(const, tree)` of one operation it walks the tree, operand 0
//!    first, to the first sub-expression with one constant operand and
//!    swaps the outer constant with that sub-expression's other operand
//!    (`reassociate_operands`, :148-166), so that the two constants meet.
//!    NIR folds them (`nir_opt_constant_folding`); this pass folds them in
//!    place, the value fp32 as NIR computes it.
//!
//! Ahead of them, a pass of Mali's NIR on the front end's run-time index
//! ladders ([`ilt_trees`]): the `ieq` ladders of an element, column or lane
//! read or written at a run-time index become Mesa's `ilt` binary search,
//! the shape nir_lower_indirect_derefs_to_if_else_trees and
//! `nir_vector_extract` give them (docs/notes/mali-indexed-locals.md).
//! The ladders are lower.rs's, which the four back ends share; only this
//! one rewrites them. After them, three of NIR's rules on a negated
//! constant, which both stages' builders would see too late (they fold
//! `fneg(literal)` first): a product with one (`Tree::neg_products`), a
//! compare of a negated value with a constant (`Tree::neg_compares`) and
//! a compare with `-0.0` (`Tree::neg_zero_compares`).
//!
//! The IR keeps what these passes need: a local, a uniform and an input
//! are loads at each read (GLSL's variable dereferences), a value with one
//! use by an ALU operation is an operand expression, and a value stored
//! whole is an assignment's right-hand side. What it does not keep is
//! refused by name rather than guessed: a tree the GLSL passes would
//! change that reads a local `do_tree_grafting` may graft (assigned once,
//! read once: opt_tree_grafting.cpp:389-401) or a value with more than one
//! use is [`MaliError::Rebalance`]; a constant moved across a change of
//! width is [`MaliError::ArithConstant`].

use super::{Defect, MaliError};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir::{
    AluOp, IoKind, IoSlot, Module, Node as IrNode, Op as IrOp, Range, Src, ValueId, VarDecl, SWIZZLE_IDENTITY,
};

/// A value's uses, by kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Use {
    None,
    /// One use, as an operand of this ALU instruction.
    Alu(u32),
    /// One use, as the value this `StoreLocal` or `StoreOutput` writes.
    Store(u32),
    /// One use by anything else (a look-up, an index, an `if`).
    Other,
    Many,
}

/// The IR of one stage and its use tables, rebuilt after each change.
struct Tree<'m, 'a> {
    m: &'m Module<'m>,
    instrs: &'a mut [IrOp],
    comps: &'a mut [u8],
    uses: &'a mut [Use],
    /// Per local: stores, whole stores, loads.
    locals: &'a mut [(u16, u16, u16)],
    /// The position of each value's instruction in `m.nodes`.
    pos: &'a mut [u32],
    changed: bool,
    /// The stage is a fragment shader whose float default is highp
    /// (mod.rs `FsPrecision::highp`).
    highp_fragment: bool,
}

/// A child of an expression record in [`dsw`]: an expression or a leaf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Child {
    E(usize),
    L(Src),
}

/// What GLSL's `is_reduction` (opt_rebalance_tree.cpp:201-257) finds for a
/// tree: a reduction of `n` expressions, none, or a tree whose GLSL form
/// the IR does not tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reduction {
    Yes(usize),
    Not,
    Unknown,
}

/// The most expressions of one tree the passes rebuild.
const MAX_TREE: usize = 64;

/// A stage's instructions, and the rest of its [`Shape`], rewritten.
pub(super) type Rewritten<'s> = (&'s [IrOp], Shape<'s>);

/// What [`passes`] rewrites besides the instructions: their widths, and
/// the node list when [`ilt_trees`] renumbered the stage.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shape<'s> {
    pub comps: &'s [u8],
    pub nodes: Option<(&'s [IrNode], Range)>,
}

impl<'s> Shape<'s> {
    /// `m` with the rewritten `instrs` and this shape.
    pub(super) fn module<'x>(self, instrs: &'x [IrOp], m: &Module<'x>) -> Module<'x>
    where
        's: 'x,
    {
        let (nodes, body) = self.nodes.unwrap_or((m.nodes, m.body));
        Module { instrs, comps: self.comps, nodes, body, ..*m }
    }
}

/// Run the passes on `m`: the run-time index trees ([`ilt_trees`]), then
/// GLSL's two. `None`: they change nothing (and refuse nothing);
/// otherwise the stage's instructions and their shape after them.
pub(super) fn passes<'s>(
    m: &Module<'_>,
    highp_fragment: bool,
    scratch: &Arena<'s>,
) -> Result<Option<Rewritten<'s>>, MaliError> {
    let trees = ilt_trees(m, scratch)?;
    let shaped;
    let m = match trees {
        Some(t) => {
            shaped = Module { instrs: t.instrs, comps: t.comps, nodes: t.nodes, body: t.body, ..*m };
            &shaped
        }
        None => m,
    };
    let n = m.instrs.len();
    let oom = |_| MaliError::OutOfScratch;
    let instrs = scratch.alloc_copy(m.instrs).map_err(oom)?;
    let comps = scratch.alloc_filled(n, 4u8).map_err(oom)?;
    for (i, c) in comps.iter_mut().enumerate() {
        *c = m.comps_of(ValueId(i as u32));
    }
    let uses = scratch.alloc_filled(n, Use::None).map_err(oom)?;
    let locals = scratch.alloc_filled(m.locals.len(), (0u16, 0u16, 0u16)).map_err(oom)?;
    let pos = scratch.alloc_filled(n, u32::MAX).map_err(oom)?;
    let mut t = Tree { m, instrs, comps, uses, locals, pos, changed: false, highp_fragment };
    t.count();
    t.rebalance()?;
    t.reassociate()?;
    t.negneg();
    t.neg_products()?;
    t.neg_compares()?;
    t.neg_zero_compares();
    if !t.changed && trees.is_none() {
        return Ok(None);
    }
    let Tree { instrs, comps, .. } = t;
    Ok(Some((&*instrs, Shape { comps: &*comps, nodes: trees.map(|t| (t.nodes, t.body)) })))
}

/// Locals `a` and `b` (`a < b`) are slots of one variable: the front end
/// gives a matrix's columns, an array's elements and a struct's leaves one
/// local each, consecutive, every one under the declaration's own name
/// (lower.rs `alloc_var`). The same name slice, not equal text: a second
/// declaration of the name (a shadow in a nested scope) is another slice of
/// the source.
fn same_run(locals: &[VarDecl<'_>], a: usize, b: usize) -> bool {
    let Some(d) = locals.get(a) else { return false };
    !d.name.is_empty() && a < b && locals.get(a..=b).is_some_and(|r| r.iter().all(|o| core::ptr::eq(o.name, d.name)))
}

/// The most elements of one run-time index tree: Mesa lowers an array of
/// up to 8 so (bifrost_nir.c:978-994), a longer one goes to scratch.
const MAX_LADDER: usize = 8;
/// The most ladders of one stage [`ilt_trees`] rewrites; the rest stay.
const MAX_LADDERS: usize = 32;
/// The instructions one ladder may add: a constant per compare, and per
/// slot of a write two per level of the tree.
const LADDER_GROWTH: usize = 64;
/// No ladder, no value.
const NONE: u32 = u32::MAX;

/// A stage after [`ilt_trees`]: renumbered, in a node list of its own.
#[derive(Clone, Copy)]
struct Trees<'s> {
    instrs: &'s [IrOp],
    comps: &'s [u8],
    nodes: &'s [IrNode],
    body: Range,
}

/// One `ieq` ladder of the front end (lower.rs) over the `n` elements of a
/// run-time index.
#[derive(Clone, Copy)]
struct Ladder {
    /// Its nodes, `m.nodes[span.0..=span.1]`, in one block: its own
    /// instructions and constants, nothing else.
    span: (u32, u32),
    /// A write (`store_among`, `store_lref` of whole slots): slot `k` of a
    /// local's run takes `v` where the index is `k`, `xs[k]` its old value.
    /// Otherwise a read (`select_among`, `vector_lane_select`): `xs[k]`
    /// where the index is `k`, `xs[n - 1]` past the compares.
    write: bool,
    n: usize,
    /// The index, its one lane splat.
    i: Src,
    /// The selects' width.
    w: u8,
    xs: [Src; MAX_LADDER],
    v: Src,
    /// The `ieq` of element `k` and its select (a read's `sels[0]` its
    /// root, the outermost), a write's load of the old value and store.
    conds: [ValueId; MAX_LADDER],
    sels: [ValueId; MAX_LADDER],
    loads: [ValueId; MAX_LADDER],
    stores: [ValueId; MAX_LADDER],
    /// Its new nodes: `seq[seq.0..seq.0 + seq.1]` of [`ilt_trees`].
    seq: (u32, u32),
}

impl Ladder {
    const EMPTY: Ladder = Ladder {
        span: (0, 0),
        write: false,
        n: 0,
        i: Src { val: ValueId(0), swizzle: SWIZZLE_IDENTITY },
        w: 1,
        xs: [Src { val: ValueId(0), swizzle: SWIZZLE_IDENTITY }; MAX_LADDER],
        v: Src { val: ValueId(0), swizzle: SWIZZLE_IDENTITY },
        conds: [ValueId(0); MAX_LADDER],
        sels: [ValueId(0); MAX_LADDER],
        loads: [ValueId(0); MAX_LADDER],
        stores: [ValueId(0); MAX_LADDER],
        seq: (0, 0),
    };

    /// The `ieq`s the ladder has: one per slot of a write, one per element
    /// but the last of a read.
    fn ncond(&self) -> usize {
        if self.write {
            self.n
        } else {
            self.n - 1
        }
    }

    /// A read of a vector's lanes (`vector_lane_select`): every element a
    /// lane of one value, not a slot of a run of locals.
    fn lanes(&self) -> bool {
        !self.write
            && self.n > 1
            && self.xs[1..self.n].iter().all(|x| x.val == self.xs[0].val && x.swizzle[0] != self.xs[0].swizzle[0])
    }
}

/// The nodes of block `r`.
fn block(nodes: &[IrNode], r: Range) -> &[IrNode] {
    nodes.get(r.first as usize..(r.first as usize).saturating_add(r.len as usize)).unwrap_or(&[])
}

/// Each value of block `r` and its nested blocks once, in ascending order
/// as the stage runs (the front end's numbering, which [`ilt_trees`]
/// keeps).
fn ascending(nodes: &[IrNode], r: Range, last: &mut Option<u32>, count: &mut usize, depth: usize) -> bool {
    if depth > MAX_TREE {
        return false;
    }
    for nd in block(nodes, r) {
        let ok = match *nd {
            IrNode::Instr(id) => {
                let ok = last.is_none_or(|l| id.0 > l);
                *last = Some(id.0);
                *count += 1;
                ok
            }
            IrNode::If { then_, else_, .. } => {
                ascending(nodes, then_, last, count, depth + 1) && ascending(nodes, else_, last, count, depth + 1)
            }
            IrNode::Loop { body } => ascending(nodes, body, last, count, depth + 1),
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// `c`, the condition of a select of `w` lanes, is the front end's `ieq(i,
/// k)` of a run-time `i` and a constant `k`, one compare for every lane of
/// the select: `i` (its lane splat), `k`, the compare. A lane of the
/// condition that reads no lane of `i` is no compare (`vector_lane_select`
/// compares the lanes of a one-lane index past its first, which nothing
/// defines).
fn ieq_of(m: &Module<'_>, c: Src, w: u8) -> Option<(Src, i32, ValueId)> {
    let IrOp::Alu(AluOp::Ieq, [a, b, _]) = *m.instrs.get(c.val.0 as usize)? else { return None };
    let IrOp::ConstI32(k) = *m.instrs.get(b.val.0 as usize)? else { return None };
    if matches!(m.instrs.get(a.val.0 as usize), Some(IrOp::ConstI32(_))) {
        return None;
    }
    let lane = |j: usize| {
        let s = c.swizzle[j] as usize & 3;
        let i = a.swizzle[s] as usize & 3;
        (s < m.comps_of(c.val) as usize && i < m.comps_of(a.val) as usize).then(|| (i, k[b.swizzle[s] as usize & 3]))
    };
    let (i, k) = lane(0)?;
    if (1..w.clamp(1, 4) as usize).any(|j| lane(j).is_some_and(|x| x != (i, k))) {
        return None;
    }
    Some((Src { val: a.val, swizzle: [i as u8; 4] }, k, c.val))
}

/// The read ladder at its root `root` (`select_among`,
/// `vector_lane_select`): `bcsel(ieq(i, 0), x0, bcsel(ieq(i, 1), x1, ..
/// x_{n-1}))`, one `i`, the constants ascending from 0, each link read
/// once and whole. Built from the last element back after every element:
/// each compare's id is below the one before, every element's below them
/// all, and `i` is one value -- a `?:` chain of the source evaluates its
/// compares, each with its own load of `i`, before its arms.
fn read_ladder(m: &Module<'_>, uses: &[u16], root: ValueId) -> Option<Ladder> {
    let w = m.comps_of(root);
    let mut l = Ladder { w, ..Ladder::EMPTY };
    let mut cur = Src::id(root);
    let mut k = 0usize;
    loop {
        let level = match m.instrs.get(cur.val.0 as usize) {
            Some(&IrOp::Alu(AluOp::Bcsel, [c, x, e])) => ieq_of(m, c, w).map(|(i, kk, cv)| (i, kk, cv, x, e)),
            _ => None,
        };
        let Some((i, kk, cv, x, e)) = level else { break };
        let whole = (0..w as usize).all(|j| cur.swizzle[j] as usize == j);
        let link = k == 0 || (uses[cur.val.0 as usize] == 1 && m.comps_of(cur.val) == w && whole);
        let same = k == 0 || (i.val == l.i.val && i.swizzle[0] == l.i.swizzle[0]);
        if kk != k as i32 || !link || !same || uses[cv.0 as usize] != 1 {
            break;
        }
        if k + 1 == MAX_LADDER {
            return None;
        }
        l.i = i;
        l.conds[k] = cv;
        l.sels[k] = cur.val;
        l.xs[k] = x;
        cur = e;
        k += 1;
    }
    if k == 0 {
        return None;
    }
    l.n = k + 1;
    l.xs[k] = cur;
    let low = l.conds[..k].iter().map(|c| c.0).min()?;
    let before = l.xs[..l.n].iter().all(|x| x.val.0 < low) && l.i.val.0 < low;
    let descending = l.conds[..k].windows(2).all(|p| p[0].0 > p[1].0);
    /* slot 0 of a write ([`write_ladder`]): its old value stored back */
    let slot = match m.instrs.get(l.xs[k].val.0 as usize) {
        Some(&IrOp::LoadLocal { local, index: None, .. }) => m.instrs.iter().any(
            |o| matches!(*o, IrOp::StoreLocal { local: to, index: None, src, .. } if to == local && src.val == root),
        ),
        _ => false,
    };
    (before && descending && !slot).then_some(l)
}

/// A write's slot at store `id`: `store(L, bcsel(ieq(i, k), v, load(L)))`
/// -- its local, `k`, `i`, `v`, the old value, and the compare, select
/// and load.
#[allow(clippy::type_complexity)]
fn write_slot(m: &Module<'_>, id: ValueId) -> Option<(u32, i32, Src, Src, Src, ValueId, ValueId, ValueId)> {
    let IrOp::StoreLocal { local, index: None, src, .. } = *m.instrs.get(id.0 as usize)? else { return None };
    let IrOp::Alu(AluOp::Bcsel, [c, v, old]) = *m.instrs.get(src.val.0 as usize)? else { return None };
    let IrOp::LoadLocal { local: from, index: None, .. } = *m.instrs.get(old.val.0 as usize)? else { return None };
    if from != local {
        return None;
    }
    let (i, k, cv) = ieq_of(m, c, m.comps_of(src.val))?;
    Some((local.0, k, i, v, old, cv, src.val, old.val))
}

/// The write ladder whose first store is node `j` of `nodes` (a block):
/// every slot of one local's run (lower.rs `alloc_var`, [`same_run`]) in
/// order, each `bcsel(ieq(i, k), v, old)` with one `i` and one `v` (the
/// front end evaluates both once; a source's statements would load each
/// anew).
fn write_ladder(m: &Module<'_>, uses: &[u16], nodes: &[IrNode], j: usize) -> Option<Ladder> {
    let IrNode::Instr(first) = *nodes.get(j)? else { return None };
    let first = id_of(first);
    let (l0, k0, i, v, old, cv, sel, load) = write_slot(m, first)?;
    let l0u = l0 as usize;
    if k0 != 0 || (l0u > 0 && same_run(m.locals, l0u - 1, l0u)) {
        return None;
    }
    let w = m.comps_of(sel);
    let mut l = Ladder { write: true, i, v, w, ..Ladder::EMPTY };
    let put = |l: &mut Ladder, k: usize, old: Src, cv: ValueId, sel: ValueId, load: ValueId, store: ValueId| {
        l.xs[k] = old;
        l.conds[k] = cv;
        l.sels[k] = sel;
        l.loads[k] = load;
        l.stores[k] = store;
    };
    put(&mut l, 0, old, cv, sel, load, first);
    let mut n = 1usize;
    for nd in &nodes[j + 1..] {
        let IrNode::Instr(id) = *nd else { break };
        let id = id_of(id);
        if !matches!(m.instrs.get(id.0 as usize), Some(IrOp::StoreLocal { .. })) {
            continue;
        }
        let Some((lk, k, ik, vk, old, cv, sel, load)) = write_slot(m, id) else { break };
        let next = k == n as i32 && lk as usize == l0u + n && same_run(m.locals, l0u, l0u + n);
        if !next || ik.val != i.val || ik.swizzle[0] != i.swizzle[0] || vk != v || m.comps_of(sel) != w {
            break;
        }
        if n == MAX_LADDER {
            return None;
        }
        put(&mut l, n, old, cv, sel, load, id);
        n += 1;
    }
    if n < 2 || same_run(m.locals, l0u, l0u + n) {
        return None;
    }
    l.n = n;
    let once = (0..n).all(|k| [l.conds[k], l.sels[k], l.loads[k]].iter().all(|x| uses[x.0 as usize] == 1));
    let low = (0..n).flat_map(|k| [l.conds[k], l.loads[k]]).map(|x| x.0).min()?;
    (once && i.val.0 < low && v.val.0 < low).then_some(l)
}

/// `l`'s nodes: from its first instruction to its last (the root of a
/// read, the last store of a write), all in block `r` of `m`, nothing
/// between them but its own and constants.
fn span_of(m: &Module<'_>, pos: &[u32], r: Range, l: &Ladder) -> Option<(u32, u32)> {
    let k = l.ncond();
    let mut ids = [ValueId(0); 4 * MAX_LADDER];
    let mut nids = 0usize;
    let groups: [&[ValueId]; 4] = if l.write {
        [&l.conds[..k], &l.sels[..l.n], &l.loads[..l.n], &l.stores[..l.n]]
    } else {
        [&l.conds[..k], &l.sels[..k], &[], &[]]
    };
    for g in groups {
        for &id in g {
            ids[nids] = id;
            nids += 1;
        }
    }
    let ids = &ids[..nids];
    let at = |id: ValueId| pos.get(id.0 as usize).copied().unwrap_or(NONE);
    let lo = ids.iter().map(|&id| at(id)).min()?;
    let hi = ids.iter().map(|&id| at(id)).max()?;
    if hi == NONE || lo < r.first || hi >= r.first + r.len {
        return None;
    }
    let own = m.nodes.get(lo as usize..=hi as usize)?.iter().all(|nd| match *nd {
        IrNode::Instr(id) => ids.contains(&id_of(id)) || matches!(m.instrs.get(id.0 as usize), Some(IrOp::ConstI32(_))),
        _ => false,
    });
    own.then_some((lo, hi))
}

fn id_of(id: crate::shaderc::online::ir::InstrId) -> ValueId {
    ValueId(id.0)
}

/// What [`ilt_trees`] finds: the ladders and the node each one owns.
struct Found<'f> {
    ladders: &'f mut [Ladder],
    n: usize,
    owner: &'f mut [u32],
}

/// The ladders of block `r` and its nested blocks.
fn find(m: &Module<'_>, uses: &[u16], pos: &[u32], r: Range, f: &mut Found<'_>, depth: usize) {
    if depth > MAX_TREE {
        return;
    }
    let nodes = block(m.nodes, r);
    for (j, nd) in nodes.iter().enumerate() {
        let l = match *nd {
            IrNode::Instr(id) => match m.instrs.get(id.0 as usize) {
                Some(IrOp::Alu(AluOp::Bcsel, _)) => read_ladder(m, uses, id_of(id)),
                Some(IrOp::StoreLocal { .. }) => write_ladder(m, uses, nodes, j),
                _ => None,
            },
            IrNode::If { then_, else_, .. } => {
                find(m, uses, pos, then_, f, depth + 1);
                find(m, uses, pos, else_, f, depth + 1);
                None
            }
            IrNode::Loop { body } => {
                find(m, uses, pos, body, f, depth + 1);
                None
            }
            _ => None,
        };
        let Some(mut l) = l else { continue };
        let Some(span) = span_of(m, pos, r, &l) else { continue };
        let owned = &mut f.owner[span.0 as usize..=span.1 as usize];
        if f.n == f.ladders.len() || owned.iter().any(|&o| o != NONE) {
            continue;
        }
        owned.fill(f.n as u32);
        l.span = span;
        f.ladders[f.n] = l;
        f.n += 1;
    }
}

/// The instructions [`ilt_trees`] writes, and the new nodes of each
/// ladder.
struct Build<'b> {
    instrs: &'b mut [IrOp],
    comps: &'b mut [u8],
    next: usize,
    seq: &'b mut [u32],
    ns: usize,
}

impl Build<'_> {
    fn push(&mut self, id: ValueId) -> Result<(), MaliError> {
        *self.seq.get_mut(self.ns).ok_or(MaliError::Defect(Defect::IrIndex))? = id.0;
        self.ns += 1;
        Ok(())
    }

    fn set(&mut self, id: ValueId, op: IrOp, w: u8) -> Result<(), MaliError> {
        *self.instrs.get_mut(id.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))? = op;
        self.comps[id.0 as usize] = w;
        self.push(id)
    }

    /// A new instruction, as a node of the ladder.
    fn add(&mut self, op: IrOp, w: u8) -> Result<ValueId, MaliError> {
        let id = ValueId(self.next as u32);
        self.next += 1;
        self.set(id, op, w)?;
        Ok(id)
    }

    /// `ilt(i, mid)` of a new constant `mid` (`nir_ilt_imm`), into `c`.
    fn ilt(&mut self, i: Src, mid: usize, c: ValueId) -> Result<(), MaliError> {
        let k = Src::id(self.add(IrOp::ConstI32([mid as i32; 4]), 4)?);
        self.set(c, IrOp::Alu(AluOp::Ilt, [i, k, k]), 1)
    }

    /// `_nir_select_from_array_helper(xs, i, start, end)`
    /// (nir_builder.h:947-958) as the deref tree's `if` nest leaves it
    /// (nir_lower_indirect_derefs_to_if_else_trees.c:47-63): the compare,
    /// the lower half, the upper half, then the select. The compares take
    /// the ladder's `ieq`s, the selects its selects, the root its root.
    /// A vector's lane (`nir_vector_extract`) is the helper itself, a call
    /// `nir_bcsel(b, nir_ilt_imm(b, idx, mid), helper(start, mid),
    /// helper(mid, end))` whose arguments the compiler Mesa was built with
    /// evaluates last to first: the upper half, the lower half, then the
    /// compare (measured: indexlocal_lanes.stderr, `v[j]`'s `ilt(j, 3)`
    /// and its select, `ilt(j, 1)` and its, then `ilt(j, 2)`).
    fn read_tree(&mut self, l: &Ladder, start: usize, end: usize, used: &mut (usize, usize)) -> Result<Src, MaliError> {
        if end - start == 1 {
            return Ok(l.xs[start]);
        }
        let mid = start + (end - start) / 2;
        let c = l.conds[used.0];
        used.0 += 1;
        let (lo, hi) = if l.lanes() {
            let hi = self.read_tree(l, mid, end, used)?;
            let lo = self.read_tree(l, start, mid, used)?;
            self.ilt(l.i, mid, c)?;
            (lo, hi)
        } else {
            self.ilt(l.i, mid, c)?;
            let lo = self.read_tree(l, start, mid, used)?;
            (lo, self.read_tree(l, mid, end, used)?)
        };
        let s = if start == 0 && end == l.n {
            l.sels[0]
        } else {
            used.1 += 1;
            l.sels[used.1]
        };
        self.set(s, IrOp::Alu(AluOp::Bcsel, [Src { val: c, swizzle: [0; 4] }, lo, hi]), l.w)?;
        Ok(Src::id(s))
    }

    /// The compares of a write's tree, in the order of the deref tree's
    /// `if`s: each internal node's `(start, end, compare)`.
    fn write_cmps(
        &mut self,
        l: &Ladder,
        start: usize,
        end: usize,
        cmps: &mut [(usize, usize, ValueId); MAX_LADDER],
        nc: &mut usize,
    ) -> Result<(), MaliError> {
        if end - start == 1 {
            return Ok(());
        }
        let mid = start + (end - start) / 2;
        let c = l.conds[*nc];
        self.ilt(l.i, mid, c)?;
        cmps[*nc] = (start, end, c);
        *nc += 1;
        self.write_cmps(l, start, mid, cmps, nc)?;
        self.write_cmps(l, mid, end, cmps, nc)
    }

    /// Slot `k`'s condition: the `if` nest's phis
    /// (nir_lower_indirect_derefs_to_if_else_trees.c:54-63, a store in the
    /// leaf only) as `bcsel`s, from the leaf up, each level folded by
    /// nir_opt_algebraic.py:2486-2489 into one select. `true` when the
    /// select takes the stored value where the condition holds, `false`
    /// when it keeps the old one there.
    fn slot_cond(
        &mut self,
        l: &Ladder,
        k: usize,
        cmps: &[(usize, usize, ValueId); MAX_LADDER],
    ) -> Result<(Src, bool), MaliError> {
        let mut path = [(ValueId(0), false); MAX_LADDER];
        let mut np = 0usize;
        let (mut start, mut end) = (0usize, l.n);
        while end - start > 1 {
            let mid = start + (end - start) / 2;
            let c = cmps.iter().find(|x| x.0 == start && x.1 == end).ok_or(MaliError::Defect(Defect::IrIndex))?.2;
            path[np] = (c, k < mid);
            np += 1;
            if k < mid {
                end = mid;
            } else {
                start = mid;
            }
        }
        let splat = |v: ValueId| Src { val: v, swizzle: [0; 4] };
        let mut at: Option<(Src, bool)> = None;
        for &(c, left) in path[..np].iter().rev() {
            let c = splat(c);
            let and = |b: &mut Self, x: Src, y: Src| b.add(IrOp::Alu(AluOp::Iand, [x, y, y]), 1).map(splat);
            let not = |b: &mut Self, x: Src| b.add(IrOp::Alu(AluOp::Inot, [x, x, x]), 1).map(splat);
            at = Some(match (left, at) {
                (true, None) => (c, true),
                (false, None) => (c, false),
                /* bcsel(a, bcsel(b, c, d), d) -> bcsel(a & b, c, d) (:2486) */
                (true, Some((b, true))) => (and(self, c, b)?, true),
                /* bcsel(a, bcsel(b, d, c), d) -> bcsel(a & ~b, c, d) (:2487) */
                (true, Some((b, false))) => {
                    let nb = not(self, b)?;
                    (and(self, c, nb)?, true)
                }
                /* bcsel(a, b, bcsel(c, b, d)) -> bcsel(a | c, b, d) (:2488) */
                (false, Some((b, false))) => (self.add(IrOp::Alu(AluOp::Ior, [c, b, b]), 1).map(splat)?, false),
                /* bcsel(a, b, bcsel(c, d, b)) -> bcsel(c & ~a, d, b) (:2489) */
                (false, Some((b, true))) => {
                    let na = not(self, c)?;
                    (and(self, b, na)?, true)
                }
            });
        }
        at.ok_or(MaliError::Defect(Defect::IrIndex))
    }
}

/// Mesa's binary search for an element at a run-time index, in place of the
/// front end's `ieq` ladders (docs/notes/mali-indexed-locals.md, sections 1
/// and 3): bifrost_nir.c:978-994 lowers a local read or written at a
/// run-time index by nir_lower_indirect_derefs_to_if_else_trees (`ilt(i,
/// mid)`, `mid = start + (end - start) / 2`, a leaf per element), and a
/// vector's component read so is `nir_vector_extract`'s
/// `_nir_select_from_array_helper`, the same search as `bcsel`s. The
/// front end (lower.rs, shared by the four back ends, whose references are
/// made from the ladder) builds `bcsel(ieq(i, k), ..)` instead:
///
/// * a read, `bcsel(ieq(i, 0), x0, bcsel(ieq(i, 1), x1, .. x_{n-1}))`,
///   becomes the tree `bcsel(ilt(i, mid), lower, upper)`;
/// * a write, `bcsel(ieq(i, k), v, old_k)` into each slot `k`, takes the
///   condition of the tree's path to leaf `k` after nir_opt_algebraic.py's
///   select rules (:2486-2489): for three slots `ilt(i, 1)`,
///   `iand(ilt(i, 2), inot(ilt(i, 1)))` and, the old value first,
///   `ior(ilt(i, 1), ilt(i, 2))`.
///
/// An index out of range now reads or writes element 0 below and the last
/// above, as Mesa's tree does (undefined in GLSL ES; the reference is the
/// oracle). A component written at a run-time index stays an `ieq` lane
/// mask (section 3.3: no reference has one). The fragment stage only: the
/// vertex walk takes no `ilt` (nir.rs, `IntValue`). The stage is
/// renumbered in the order it runs, as the front end numbers it, with the
/// new nodes where the ladder was. `None`: no ladder.
fn ilt_trees<'s>(m: &Module<'_>, scratch: &Arena<'s>) -> Result<Option<Trees<'s>>, MaliError> {
    if m.stage != crate::shaderc::SHC_STAGE_FRAGMENT || m.body.len == 0 {
        return Ok(None);
    }
    let n = m.instrs.len();
    let oom = |_| MaliError::OutOfScratch;
    let uses = scratch.alloc_filled(n, 0u16).map_err(oom)?;
    {
        let mut note = |v: ValueId| {
            if let Some(u) = uses.get_mut(v.0 as usize) {
                *u = u.saturating_add(1);
            }
        };
        for op in m.instrs {
            op.for_each_src(|s| note(s.val));
        }
        for nd in m.nodes {
            if let IrNode::If { cond, .. } = *nd {
                note(cond.val);
            }
        }
    }
    let pos = scratch.alloc_filled(n, NONE).map_err(oom)?;
    for (k, nd) in m.nodes.iter().enumerate() {
        if let IrNode::Instr(id) = *nd {
            if let Some(p) = pos.get_mut(id.0 as usize) {
                *p = k as u32;
            }
        }
    }
    let (mut last, mut count) = (None, 0usize);
    if !ascending(m.nodes, m.body, &mut last, &mut count, 0) || count != n {
        return Ok(None);
    }
    let ladders = scratch.alloc_filled(MAX_LADDERS, Ladder::EMPTY).map_err(oom)?;
    let owner = scratch.alloc_filled(m.nodes.len(), NONE).map_err(oom)?;
    let mut f = Found { ladders, n: 0, owner };
    find(m, uses, pos, m.body, &mut f, 0);
    let Found { ladders, n: nl, owner } = f;
    if nl == 0 {
        return Ok(None);
    }
    let ladders = &mut ladders[..nl];

    /* the new nodes of each ladder */
    let cap = n + nl * LADDER_GROWTH;
    let instrs = scratch.alloc_filled(cap, IrOp::ConstI32([0; 4])).map_err(oom)?;
    instrs[..n].copy_from_slice(m.instrs);
    let comps = scratch.alloc_filled(cap, 4u8).map_err(oom)?;
    for (i, c) in comps[..n].iter_mut().enumerate() {
        *c = m.comps_of(ValueId(i as u32));
    }
    let nseq: usize = ladders.iter().map(|l| (l.span.1 - l.span.0 + 1) as usize + LADDER_GROWTH * 2).sum();
    let seq = scratch.alloc_filled(nseq, 0u32).map_err(oom)?;
    let mut b = Build { instrs, comps, next: n, seq, ns: 0 };
    for l in ladders.iter_mut() {
        let first = b.ns;
        let k = l.ncond();
        /* the span's constants first, but those only the ladder's
         * compares read (nir_opt_dce) */
        for p in l.span.0..=l.span.1 {
            let IrNode::Instr(id) = m.nodes[p as usize] else { continue };
            if !matches!(m.instrs[id.0 as usize], IrOp::ConstI32(_)) {
                continue;
            }
            let by_ladder = l.conds[..k]
                .iter()
                .filter(|c| matches!(m.instrs[c.0 as usize], IrOp::Alu(_, [_, s, _]) if s.val.0 == id.0))
                .count();
            if uses[id.0 as usize] as usize != by_ladder {
                b.push(id_of(id))?;
            }
        }
        if l.write {
            let mut cmps = [(0usize, 0usize, ValueId(0)); MAX_LADDER];
            let mut nc = 0usize;
            b.write_cmps(l, 0, l.n, &mut cmps, &mut nc)?;
            for s in 0..l.n {
                b.push(l.loads[s])?;
                let (c, stored) = b.slot_cond(l, s, &cmps)?;
                let arms = if stored { [l.v, l.xs[s]] } else { [l.xs[s], l.v] };
                b.set(l.sels[s], IrOp::Alu(AluOp::Bcsel, [c, arms[0], arms[1]]), l.w)?;
                b.push(l.stores[s])?;
            }
        } else {
            b.read_tree(l, 0, l.n, &mut (0, 0))?;
        }
        l.seq = (first as u32, (b.ns - first) as u32);
    }
    let Build { instrs, comps, next: ext, seq, .. } = b;

    /* renumbered in the order the stage runs */
    let map = scratch.alloc_filled(ext, NONE).map_err(oom)?;
    let r = Renumber { nodes: m.nodes, owner, ladders, seq };
    let mut total = 0u32;
    r.number(m.body, map, &mut total, 0);
    let out = scratch.alloc_filled(m.nodes.len() + nseq, IrNode::Break).map_err(oom)?;
    let mut nout = 0usize;
    let Some(body) = r.layout(m.body, map, out, &mut nout, 0) else { return Ok(None) };
    let fin = scratch.alloc_filled(total as usize, IrOp::ConstI32([0; 4])).map_err(oom)?;
    let fcomps = scratch.alloc_filled(total as usize, 4u8).map_err(oom)?;
    for (old, &new) in map.iter().enumerate() {
        if new == NONE {
            continue;
        }
        let mut op = instrs[old];
        let lost = core::cell::Cell::new(false);
        let to = |s: &mut Src| match map.get(s.val.0 as usize) {
            Some(&v) if v != NONE => s.val = ValueId(v),
            _ => lost.set(true),
        };
        match &mut op {
            IrOp::Alu(a, s) => {
                let k = a.arity();
                for j in 0..3 {
                    if j < k {
                        to(&mut s[j]);
                    } else {
                        s[j] = s[k - 1];
                    }
                }
            }
            o => remap_srcs(o, &to),
        }
        if lost.get() {
            return Ok(None);
        }
        fin[new as usize] = op;
        fcomps[new as usize] = comps[old];
    }
    Ok(Some(Trees { instrs: fin, comps: fcomps, nodes: &out[..nout], body }))
}

/// The stage's nodes with each ladder's span replaced by its new nodes.
struct Renumber<'r> {
    nodes: &'r [IrNode],
    owner: &'r [u32],
    ladders: &'r [Ladder],
    seq: &'r [u32],
}

impl Renumber<'_> {
    /// The ladder whose span node `p` is, and whether `p` is its first.
    fn at(&self, p: usize) -> Option<(&[u32], bool)> {
        let o = *self.owner.get(p)?;
        let l = self.ladders.get(o as usize)?;
        let s = self.seq.get(l.seq.0 as usize..(l.seq.0 + l.seq.1) as usize)?;
        Some((s, p == l.span.0 as usize))
    }

    /// Each value's new id, in the order the stage runs.
    fn number(&self, r: Range, map: &mut [u32], next: &mut u32, depth: usize) {
        if depth > MAX_TREE {
            return;
        }
        for (j, nd) in block(self.nodes, r).iter().enumerate() {
            if let Some((s, first)) = self.at(r.first as usize + j) {
                if first {
                    for &id in s {
                        map[id as usize] = *next;
                        *next += 1;
                    }
                }
                continue;
            }
            match *nd {
                IrNode::Instr(id) => {
                    map[id.0 as usize] = *next;
                    *next += 1;
                }
                IrNode::If { then_, else_, .. } => {
                    self.number(then_, map, next, depth + 1);
                    self.number(else_, map, next, depth + 1);
                }
                IrNode::Loop { body } => self.number(body, map, next, depth + 1),
                _ => {}
            }
        }
    }

    /// Block `r` written at `out[*n..]`, then its nested blocks after it.
    fn layout(&self, r: Range, map: &[u32], out: &mut [IrNode], n: &mut usize, depth: usize) -> Option<Range> {
        if depth > MAX_TREE {
            return None;
        }
        let first = *n;
        let mut put = |nd: IrNode, n: &mut usize| -> Option<()> {
            *out.get_mut(*n)? = nd;
            *n += 1;
            Some(())
        };
        let id = |v: u32| map.get(v as usize).copied().filter(|&x| x != NONE);
        for (j, nd) in block(self.nodes, r).iter().enumerate() {
            if let Some((s, first)) = self.at(r.first as usize + j) {
                if first {
                    for &v in s {
                        put(IrNode::Instr(crate::shaderc::online::ir::InstrId(id(v)?)), n)?;
                    }
                }
                continue;
            }
            let nd = match *nd {
                IrNode::Instr(v) => IrNode::Instr(crate::shaderc::online::ir::InstrId(id(v.0)?)),
                IrNode::If { cond, then_, else_ } => {
                    IrNode::If { cond: Src { val: ValueId(id(cond.val.0)?), swizzle: cond.swizzle }, then_, else_ }
                }
                o => o,
            };
            put(nd, n)?;
        }
        let r2 = Range { first: first as u32, len: (*n - first) as u32 };
        for q in first..*n {
            let nd = match out[q] {
                IrNode::If { cond, then_, else_ } => {
                    let then_ = self.layout(then_, map, out, n, depth + 1)?;
                    let else_ = self.layout(else_, map, out, n, depth + 1)?;
                    IrNode::If { cond, then_, else_ }
                }
                IrNode::Loop { body } => IrNode::Loop { body: self.layout(body, map, out, n, depth + 1)? },
                o => o,
            };
            out[q] = nd;
        }
        Some(r2)
    }
}

fn is_const(op: &IrOp) -> bool {
    matches!(op, IrOp::ConstF32(_))
}

/// `gl_type` names a float matrix.
fn is_matrix(gl_type: u16) -> bool {
    use crate::gl::consts as gl;
    let t = gl_type as u32;
    (gl::GL_FLOAT_MAT2..=gl::GL_FLOAT_MAT4).contains(&t) || (gl::GL_FLOAT_MAT2x3..=gl::GL_FLOAT_MAT4x3).contains(&t)
}

/// The columns of a matrix type (1 for any other).
pub(super) fn columns(gl_type: u16) -> u8 {
    use crate::gl::consts as gl;
    match gl_type as u32 {
        gl::GL_FLOAT_MAT2 | gl::GL_FLOAT_MAT2x3 | gl::GL_FLOAT_MAT2x4 => 2,
        gl::GL_FLOAT_MAT3 | gl::GL_FLOAT_MAT3x2 | gl::GL_FLOAT_MAT3x4 => 3,
        gl::GL_FLOAT_MAT4 | gl::GL_FLOAT_MAT4x2 | gl::GL_FLOAT_MAT4x3 => 4,
        _ => 1,
    }
}

impl Tree<'_, '_> {
    /// Rebuild the use tables from the instructions.
    fn count(&mut self) {
        self.uses.fill(Use::None);
        self.locals.fill((0, 0, 0));
        let n = self.instrs.len();
        let uses = &mut *self.uses;
        let mut note = |v: ValueId, by: Use| {
            if let Some(u) = uses.get_mut(v.0 as usize) {
                *u = if *u == Use::None { by } else { Use::Many };
            }
        };
        for i in 0..n {
            let op = self.instrs[i];
            match op {
                IrOp::Alu(a, s) => s[..a.arity()].iter().for_each(|s| note(s.val, Use::Alu(i as u32))),
                IrOp::StoreLocal { index: None, src, .. } | IrOp::StoreOutput { src, .. } => {
                    note(src.val, Use::Store(i as u32))
                }
                _ => op.for_each_src(|s| note(s.val, Use::Other)),
            }
            match op {
                IrOp::StoreLocal { local, index, write_mask, .. } => {
                    if let Some(l) = self.locals.get_mut(local.0 as usize) {
                        let decl = self.m.locals[local.0 as usize];
                        let whole = index.is_none()
                            && decl.array_len <= 1
                            && write_mask & 0xf == (1u8 << decl.ty.comps.min(4)) - 1;
                        l.0 = l.0.saturating_add(1);
                        l.1 = l.1.saturating_add(whole as u16);
                    }
                }
                IrOp::LoadLocal { local, .. } => {
                    if let Some(l) = self.locals.get_mut(local.0 as usize) {
                        l.2 = l.2.saturating_add(1);
                    }
                }
                _ => {}
            }
        }
        self.pos.fill(u32::MAX);
        for (k, nd) in self.m.nodes.iter().enumerate() {
            match *nd {
                IrNode::Instr(id) => {
                    if let Some(p) = self.pos.get_mut(id.0 as usize) {
                        *p = k as u32;
                    }
                }
                IrNode::If { cond, .. } => note(cond.val, Use::Other),
                _ => {}
            }
        }
    }

    fn op(&self, v: ValueId) -> Option<IrOp> {
        self.instrs.get(v.0 as usize).copied()
    }

    fn width(&self, v: ValueId) -> u8 {
        self.comps.get(v.0 as usize).copied().unwrap_or(4)
    }

    fn uses(&self, v: ValueId) -> Use {
        self.uses.get(v.0 as usize).copied().unwrap_or(Use::None)
    }

    /// `v` is an expression in GLSL's tree: an ALU value whose one use is
    /// an ALU operation's operand.
    fn inline(&self, v: ValueId) -> bool {
        matches!(self.op(v), Some(IrOp::Alu(..))) && matches!(self.uses(v), Use::Alu(_))
    }

    /// The operation of a tree node: `fadd` or `fmul`, not a subtraction
    /// (`a - b` is the front end's `fadd(a, fneg(b))`, GLSL's
    /// `ir_binop_sub`, no reduction).
    fn tree_op(&self, v: ValueId) -> Option<AluOp> {
        match self.op(v)? {
            IrOp::Alu(AluOp::Fadd, s)
                if s[..2].iter().any(|s| matches!(self.op(s.val), Some(IrOp::Alu(AluOp::Fneg, _)))) =>
            {
                None
            }
            IrOp::Alu(a @ (AluOp::Fadd | AluOp::Fmul), _) => Some(a),
            _ => None,
        }
    }

    /// The operand `s` of `parent` is read whole, lane for lane: no
    /// swizzle and no change of width between them.
    fn direct(&self, parent: ValueId, s: Src) -> bool {
        let w = self.width(parent);
        self.width(s.val) == w && (0..w as usize).all(|i| s.swizzle[i] as usize == i)
    }

    /// The nodes of the chain at `v` as the refusal before this pass
    /// counted them: the tree's single-use links of one operation and
    /// width.
    fn chain(&self, v: ValueId) -> usize {
        let Some(op) = self.tree_op(v) else { return 0 };
        let Some(IrOp::Alu(_, s)) = self.op(v) else { return 0 };
        1 + s[..2]
            .iter()
            .filter(|c| self.inline(c.val) && self.tree_op(c.val) == Some(op) && self.width(c.val) == self.width(v))
            .map(|c| self.chain(c.val))
            .sum::<usize>()
    }

    /// The local `l` may be grafted into its one read
    /// (opt_tree_grafting.cpp:389-401: assigned once, whole, and read once).
    fn graftable(&self, l: usize) -> bool {
        matches!(self.locals.get(l), Some(&(1, 1, 1)))
    }

    /// The local `l` is one slot of a variable of several ([`same_run`]
    /// with a neighbour): a matrix's column, an array's element or a
    /// struct's leaf.
    fn run_slot(&self, l: usize) -> bool {
        same_run(self.m.locals, l, l + 1) || l.checked_sub(1).is_some_and(|k| same_run(self.m.locals, k, l))
    }

    /// GLSL's view of a leaf of a tree: a constant (`Some(true)`), a
    /// variable dereference (`Some(false)`), or a dereference that ends the
    /// reduction (`None` in `Err(Not)`), or a value whose GLSL form is not
    /// known (`Err(Unknown)`).
    fn leaf(&self, s: Src) -> Result<bool, Reduction> {
        let m = self.m;
        match self.op(s.val).ok_or(Reduction::Unknown)? {
            IrOp::ConstF32(_) => Ok(true),
            IrOp::LoadUniform { dword_off, .. } => {
                let u = m
                    .uniforms
                    .iter()
                    .find(|u| {
                        u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK
                            && dword_off >= u.dword_off
                            && dword_off < u.dword_off + u.dwords.max(1)
                    })
                    .ok_or(Reduction::Unknown)?;
                /* an array element, a matrix column or a struct member is
                 * an `ir_dereference_array`/`_record` (:219-226) */
                if u.array_len > 1 || u.inner > 1 || is_matrix(u.gl_type) || u.name.contains(['.', '[']) {
                    return Err(Reduction::Not);
                }
                Ok(false)
            }
            IrOp::LoadInput { slot: IoSlot::Attrib(loc) | IoSlot::Varying(loc), .. } => {
                let col = |d: &&crate::shaderc::online::ir::IoDecl<'_>| {
                    let n = if d.name.contains('[') { 1 } else { columns(d.gl_type) };
                    matches!(d.kind, IoKind::Attribute | IoKind::Varying)
                        && loc >= d.location
                        && loc < d.location.saturating_add(n)
                        && is_matrix(d.gl_type)
                };
                if m.inputs.iter().any(|d| col(&d)) {
                    return Err(Reduction::Not);
                }
                Ok(false)
            }
            IrOp::LoadInput { .. } => Ok(false),
            IrOp::LoadLocal { index: Some(_), .. } | IrOp::LoadUniformIdx { .. } => Err(Reduction::Not),
            IrOp::LoadLocal { local, index: None, .. } => {
                let l = local.0 as usize;
                /* a matrix local's column, an array element or a struct
                 * member is an `ir_dereference_array`/`_record` (:219-226),
                 * as a matrix attribute's column is (case 80's
                 * `s[1][1] + d[1][0] + ..`) */
                if m.locals.get(l).is_some_and(|d| d.array_len > 1) || self.run_slot(l) {
                    return Err(Reduction::Not);
                }
                if self.graftable(l) {
                    return Err(Reduction::Unknown);
                }
                Ok(false)
            }
            _ => Err(Reduction::Unknown),
        }
    }

    /// `is_reduction` (opt_rebalance_tree.cpp:201-257) over the tree at
    /// `root`, an `fadd` or `fmul`.
    fn reduction(&self, root: ValueId) -> Reduction {
        let Some(op) = self.tree_op(root) else { return Reduction::Not };
        let w = self.width(root);
        let mut n = 0usize;
        let mut konst = false;
        let mut unknown = false;
        let mut stack = [ValueId(0); MAX_TREE];
        let mut sp = 0usize;
        stack[sp] = root;
        sp += 1;
        while sp > 0 {
            sp -= 1;
            let v = stack[sp];
            /* an expression of another operation or type ends it */
            if self.tree_op(v) != Some(op) || self.width(v) != w {
                return Reduction::Not;
            }
            n += 1;
            let Some(IrOp::Alu(_, s)) = self.op(v) else { return Reduction::Unknown };
            for c in &s[..2] {
                if matches!(self.op(c.val), Some(IrOp::Alu(..))) {
                    if !self.inline(c.val) {
                        unknown = true;
                    } else if self.direct(v, *c) {
                        if sp == MAX_TREE {
                            return Reduction::Unknown;
                        }
                        stack[sp] = c.val;
                        sp += 1;
                    } else if self.width(c.val) != self.width(v) {
                        return Reduction::Not;
                    } else {
                        /* a swizzle of an expression */
                        unknown = true;
                    }
                    continue;
                }
                match self.leaf(*c) {
                    Ok(true) if konst => return Reduction::Not,
                    Ok(true) => konst = true,
                    Ok(false) => {}
                    Err(Reduction::Not) => return Reduction::Not,
                    Err(_) => unknown = true,
                }
            }
        }
        if unknown {
            Reduction::Unknown
        } else {
            Reduction::Yes(n)
        }
    }

    /// The statement of the tree at `v` assigns an `invariant` output:
    /// both passes skip it (opt_rebalance_tree.cpp:151-163,
    /// opt_algebraic.cpp's `visit_enter(ir_assignment)`).
    fn invariant(&self, mut v: ValueId) -> bool {
        for _ in 0..self.instrs.len() {
            match self.uses(v) {
                Use::Alu(u) => v = ValueId(u),
                Use::Store(u) => {
                    return matches!(self.op(ValueId(u)), Some(IrOp::StoreOutput { slot, .. })
                        if self.m.outputs.iter().any(|d| d.slot() == slot && d.invariant));
                }
                _ => return false,
            }
        }
        false
    }

    /// The tree at the root `v` (an ALU value that is no operand
    /// expression) is a whole assignment's right-hand side, which the
    /// rebalance visitor does not handle (:151-163): one use, a store of
    /// every lane of the variable, lane for lane.
    fn whole_rhs(&self, v: ValueId) -> bool {
        let Use::Store(u) = self.uses(v) else { return false };
        let w = self.width(v);
        let identity = |s: Src| (0..w as usize).all(|i| s.swizzle[i] as usize == i);
        match self.op(ValueId(u)) {
            Some(IrOp::StoreLocal { local, index: None, src, write_mask }) => {
                let Some(d) = self.m.locals.get(local.0 as usize) else { return false };
                d.array_len <= 1 && d.ty.comps == w && write_mask & 0xf == (1u8 << w.min(4)) - 1 && identity(src)
            }
            Some(IrOp::StoreOutput { slot, src }) => {
                self.m.outputs.iter().any(|d| d.slot() == slot && d.components == w) && identity(src)
            }
            _ => false,
        }
    }

    /// `do_rebalance_tree` over the stage.
    fn rebalance(&mut self) -> Result<(), MaliError> {
        let n = self.instrs.len();
        for i in 0..n {
            let v = ValueId(i as u32);
            if !matches!(self.op(v), Some(IrOp::Alu(..))) || self.inline(v) || self.invariant(v) {
                continue;
            }
            let handled = match self.uses(v) {
                Use::None => continue,
                Use::Many => {
                    /* a value of several uses: what GLSL had there (a
                     * temporary, a copy) is not kept */
                    if self.chain(v) >= 3 {
                        return Err(MaliError::Rebalance);
                    }
                    continue;
                }
                _ => !self.whole_rhs(v),
            };
            self.visit(v, handled, 0)?;
        }
        Ok(())
    }

    /// The rvalue enter visitor at `v`: `handle_rvalue` (:299-327) when
    /// `handled`, then each operand expression of the (rebuilt) tree.
    fn visit(&mut self, v: ValueId, handled: bool, depth: usize) -> Result<(), MaliError> {
        if depth > MAX_TREE {
            return Err(MaliError::Rebalance);
        }
        if handled && self.tree_op(v).is_some() {
            match self.reduction(v) {
                Reduction::Yes(k) if k > 2 => self.dsw(v)?,
                Reduction::Unknown if self.chain(v) >= 3 => return Err(MaliError::Rebalance),
                _ => {}
            }
        }
        let Some(IrOp::Alu(a, s)) = self.op(v) else { return Ok(()) };
        for c in s.iter().take(a.arity()) {
            if self.inline(c.val) {
                self.visit(c.val, true, depth + 1)?;
            }
        }
        Ok(())
    }

    /// `handle_expression` (:259-281) on the reduction at `root`: the tree
    /// under a pseudo root `0.0 + tree` to a vine and back, then the
    /// result written over the tree's instructions in its post-order (the
    /// order glsl_to_nir emits it), each leaf load where it is first read.
    fn dsw(&mut self, root: ValueId) -> Result<(), MaliError> {
        let mut rec = [(0u32, [Child::L(Src::id(ValueId(0))); 2]); MAX_TREE + 1];
        let mut nrec = 1usize;
        /* build: record 0 the pseudo root */
        let mut work = [(0usize, ValueId(0)); MAX_TREE];
        let mut nw = 0usize;
        rec[1].0 = root.0;
        nrec += 1;
        work[nw] = (1, root);
        nw += 1;
        rec[0].1 = [Child::L(Src::id(ValueId(u32::MAX))), Child::E(1)];
        while nw > 0 {
            nw -= 1;
            let (k, v) = work[nw];
            let Some(IrOp::Alu(_, s)) = self.op(v) else { return Err(MaliError::Defect(Defect::IrIndex)) };
            for j in 0..2 {
                rec[k].1[j] = if self.inline(s[j].val) && matches!(self.op(s[j].val), Some(IrOp::Alu(..))) {
                    if nrec > MAX_TREE || nw == MAX_TREE {
                        return Err(MaliError::Rebalance);
                    }
                    rec[nrec].0 = s[j].val.0;
                    work[nw] = (nrec, s[j].val);
                    nw += 1;
                    nrec += 1;
                    Child::E(nrec - 1)
                } else {
                    Child::L(s[j])
                };
            }
        }
        let size = tree_to_vine(&mut rec);
        vine_to_tree(&mut rec, size);
        let Child::E(top) = rec[0].1[1] else { return Err(MaliError::Defect(Defect::IrIndex)) };
        /* the post-order of the new tree, leaves included */
        let mut seq = [Child::L(Src::id(ValueId(0))); 2 * MAX_TREE + 2];
        let mut nseq = 0usize;
        post_order(&rec, top, &mut seq, &mut nseq)?;
        /* the span the tree's instructions take in the node list: every
         * node there is one of them (an expression or a leaf load), in
         * ascending value order */
        let first = rec[1..nrec].iter().map(|r| r.0).min().ok_or(MaliError::Defect(Defect::IrIndex))?;
        if rec[1..nrec].iter().any(|r| r.0 > root.0) {
            return Err(MaliError::Rebalance);
        }
        let (p0, p1) = (self.pos[first as usize], self.pos[root.0 as usize]);
        if p0 == u32::MAX || p1 == u32::MAX || p0 > p1 {
            return Err(MaliError::Rebalance);
        }
        let span = &self.m.nodes[p0 as usize..=p1 as usize];
        let mut ids = [0u32; 2 * MAX_TREE + 2];
        let mut nids = 0usize;
        for nd in span {
            let IrNode::Instr(id) = *nd else { return Err(MaliError::Rebalance) };
            if nids == ids.len() || (nids > 0 && ids[nids - 1] >= id.0) {
                return Err(MaliError::Rebalance);
            }
            ids[nids] = id.0;
            nids += 1;
        }
        /* the sequence restricted to the span: each id once */
        let mut order = [0u32; 2 * MAX_TREE + 2];
        let mut nord = 0usize;
        for c in &seq[..nseq] {
            let id = match *c {
                Child::E(k) => rec[k].0,
                Child::L(s) => s.val.0,
            };
            if !ids[..nids].contains(&id) || order[..nord].contains(&id) {
                continue;
            }
            if let Child::L(s) = *c {
                if !matches!(
                    self.op(s.val),
                    Some(
                        IrOp::ConstF32(_)
                            | IrOp::LoadUniform { .. }
                            | IrOp::LoadInput { .. }
                            | IrOp::LoadLocal { index: None, .. }
                    )
                ) {
                    return Err(MaliError::Rebalance);
                }
            }
            order[nord] = id;
            nord += 1;
        }
        if nord != nids {
            return Err(MaliError::Rebalance);
        }
        /* an `if` reads no value of the span (the node list stays) */
        if self.m.nodes.iter().any(|nd| matches!(nd, IrNode::If { cond, .. } if ids[..nids].contains(&cond.val.0))) {
            return Err(MaliError::Rebalance);
        }
        /* old id order[j] becomes ids[j] */
        let map = |v: u32| -> u32 { order[..nord].iter().position(|&o| o == v).map_or(v, |j| ids[j]) };
        let old: [IrOp; 2 * MAX_TREE + 2] =
            core::array::from_fn(|j| if j < nord { self.instrs[order[j] as usize] } else { IrOp::ConstF32([0; 4]) });
        let oldc: [u8; 2 * MAX_TREE + 2] =
            core::array::from_fn(|j| if j < nord { self.comps[order[j] as usize] } else { 0 });
        let w = self.width(root);
        for j in 0..nord {
            let to = ids[j] as usize;
            let k = rec[1..nrec].iter().position(|r| r.0 == order[j]).map(|k| k + 1);
            self.instrs[to] = match k {
                Some(k) => {
                    let IrOp::Alu(a, _) = old[j] else { return Err(MaliError::Defect(Defect::IrIndex)) };
                    let src = |c: Child| match c {
                        Child::E(e) => Src::id(ValueId(map(rec[e].0))),
                        Child::L(s) => Src { val: ValueId(map(s.val.0)), swizzle: s.swizzle },
                    };
                    let (x, y) = (src(rec[k].1[0]), src(rec[k].1[1]));
                    IrOp::Alu(a, [x, y, y])
                }
                None => old[j],
            };
            self.comps[to] = if k.is_some() { w } else { oldc[j] };
        }
        /* every other read of a moved leaf; the tree's value stays at the
         * root's id, the last of the span */
        let remap = |s: &mut Src| {
            if s.val != root {
                s.val = ValueId(map(s.val.0));
            }
        };
        for i in 0..self.instrs.len() {
            if !ids[..nids].contains(&(i as u32)) {
                remap_srcs(&mut self.instrs[i], &remap);
            }
        }
        self.changed = true;
        self.count();
        Ok(())
    }

    /// `do_algebraic`'s `reassociate_constant` over the stage, bottom-up
    /// (an rvalue visitor: each expression after its operands).
    fn reassociate(&mut self) -> Result<(), MaliError> {
        for i in 0..self.instrs.len() {
            let v = ValueId(i as u32);
            let Some(op) = self.tree_op(v) else { continue };
            let Some(IrOp::Alu(_, s)) = self.op(v) else { continue };
            if self.invariant(v) {
                continue;
            }
            let c = [0, 1].map(|k| self.op(s[k].val).as_ref().is_some_and(is_const));
            let at = match c {
                [true, false] => 0,
                [false, true] => 1,
                _ => continue,
            };
            let other = s[1 - at];
            /* `op_expr`: an operand expression (one use, no swizzle) */
            if !self.inline(other.val) {
                continue;
            }
            self.reassociate_constant(v, at, op, other, false, 0)?;
        }
        Ok(())
    }

    /// `reassociate_constant(ir1, const_index, constant, ir2)`
    /// (:175-216): `ir1`'s constant operand `at` down the tree at `ir2`
    /// (reached from `ir1` through operands of `op`; `mixed`: a change of
    /// width on the way).
    fn reassociate_constant(
        &mut self,
        ir1: ValueId,
        at: usize,
        op: AluOp,
        ir2s: Src,
        mixed: bool,
        depth: usize,
    ) -> Result<bool, MaliError> {
        let ir2 = ir2s.val;
        if depth > MAX_TREE || !self.inline(ir2) || self.tree_op(ir2) != Some(op) {
            return Ok(false);
        }
        let Some(IrOp::Alu(_, s2)) = self.op(ir2) else { return Ok(false) };
        /* a swizzle is no `as_expression()`; a scalar operand of a vector
         * operation is */
        let parent = match self.uses(ir2) {
            Use::Alu(p) => ValueId(p),
            _ => return Ok(false),
        };
        let mut mixed = mixed;
        if !self.direct(parent, ir2s) {
            if self.width(ir2) == self.width(parent) {
                return Ok(false);
            }
            mixed = true;
        }
        let c2 = [0, 1].map(|k| self.op(s2[k].val).as_ref().is_some_and(is_const));
        let swap = match c2 {
            [true, true] => return Ok(false),
            [true, false] => Some(1),
            [false, true] => Some(0),
            _ => None,
        };
        if let Some(k2) = swap {
            if mixed {
                return Err(MaliError::ArithConstant);
            }
            /* reassociate_operands(ir1, const_index, ir2, k2) */
            let Some(IrOp::Alu(a1, mut s1)) = self.op(ir1) else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let mut s2 = s2;
            core::mem::swap(&mut s1[at], &mut s2[k2]);
            s1[2] = s1[1];
            self.instrs[ir1.0 as usize] = IrOp::Alu(a1, s1);
            /* the two constants meet: nir_opt_constant_folding */
            let lane = |s: Src, i: usize| match self.op(s.val) {
                Some(IrOp::ConstF32(b)) => f32::from_bits(b[s.swizzle[i] as usize & 3]),
                _ => 0.0,
            };
            let folded: [u32; 4] = core::array::from_fn(|i| {
                let (x, y) = (lane(s2[0], i), lane(s2[1], i));
                match op {
                    AluOp::Fmul => (x * y).to_bits(),
                    _ => (x + y).to_bits(),
                }
            });
            self.instrs[ir2.0 as usize] = IrOp::ConstF32(folded);
            self.comps[ir2.0 as usize] = 4;
            /* the folded constant is read whole, lane for lane */
            if let Some(IrOp::Alu(ap, mut sp)) = self.op(parent) {
                for x in sp.iter_mut() {
                    if x.val == ir2 {
                        x.swizzle = SWIZZLE_IDENTITY;
                    }
                }
                self.instrs[parent.0 as usize] = IrOp::Alu(ap, sp);
            }
            self.changed = true;
            self.count();
            return Ok(true);
        }
        for k in 0..2 {
            if self.reassociate_constant(ir1, at, op, s2[k], mixed, depth + 1)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl Tree<'_, '_> {
    /// NIR's `fneg(fneg(a))` -> `fcanonicalize(a)` -> `a`
    /// (nir_opt_algebraic.py:230, :161-167), after the GLSL passes, which
    /// see the negations: each read of the outer negation reads `a`
    /// through both swizzles, and the pair is left without a use for the
    /// builders' dead-code elimination (algfold_negneg.expect.json: the
    /// colour's lanes are the uniforms themselves, `V2F32_TO_V2F16 r0,
    /// u2.w0, u2.w1`).
    fn negneg(&mut self) {
        let n = self.instrs.len();
        let through = |instrs: &[IrOp], s: Src| -> Option<Src> {
            let IrOp::Alu(AluOp::Fneg, [a, ..]) = *instrs.get(s.val.0 as usize)? else { return None };
            let IrOp::Alu(AluOp::Fneg, [b, ..]) = *instrs.get(a.val.0 as usize)? else { return None };
            let swizzle = core::array::from_fn(|i| b.swizzle[a.swizzle[s.swizzle[i] as usize & 3] as usize & 3]);
            Some(Src { val: b.val, swizzle })
        };
        let mut any = false;
        for i in 0..n {
            let mut op = self.instrs[i];
            let mut hit = false;
            remap_srcs_mut(&mut op, &mut |s: &mut Src| {
                if let Some(t) = through(self.instrs, *s) {
                    *s = t;
                    hit = true;
                }
            });
            if hit {
                self.instrs[i] = op;
                any = true;
            }
        }
        if any {
            self.changed = true;
            self.count();
        }
    }
}

impl Tree<'_, '_> {
    /// `fmul(fneg(a), b)` is `fneg(fmul(a, b))` (nir_opt_algebraic.py:2387,
    /// a commutative match: `a` the fneg's source, first, on whichever
    /// side the fneg stood) for a negated literal: glsl_to_nir gives
    /// `x * -3.0` as `fmul(x, fneg(3.0))`, and the GL linker's loop runs
    /// `nir_opt_algebraic` before `nir_opt_constant_folding`
    /// (gl_nir_linker.c:102-103), so the rule takes the product before the
    /// constant is folded. Measured: `vec4(a2 * -3.0, 0.0, 1.0)` is
    /// `FMA.v2f16 r0, u2.w0, r0^` (3.0 first) then `FADD.v2f16 r0,
    /// r0^.neg, 0x0.neg` (tools/mali/testdata/negmul_*). The builders fold
    /// `fneg(3.0)` into `-3.0` before they see the product, so it is
    /// rewritten here, in place: the fneg (read by the product alone)
    /// becomes the product of the literal and the other operand, and the
    /// product its negation. That needs the other operand made before the
    /// fneg; a literal first (`-3.0 * x`) has no place for the product
    /// between the operand and its user, and is refused
    /// ([`MaliError::ArithConstant`]), unless it is 1.0, which both forms
    /// make `fneg(x)` (:292, :296). A product read by `fmin` or `fmax` may be
    /// GLSL's `clamp` that NIR sees as `fsat` (the rule's
    /// `is_used_by_non_fsat`), and a product of a product (`x * u * -3.0`)
    /// takes a reassociation next (`fmul(fmul(a, b), #c)`, measured: Mesa
    /// computes `x * 3.0` first, then `* u`, negmul_three): both refused
    /// the same way.
    fn neg_products(&mut self) -> Result<(), MaliError> {
        let n = self.instrs.len();
        let mut any = false;
        for p in 0..n {
            let IrOp::Alu(AluOp::Fmul, s) = self.instrs[p] else { continue };
            let lit = |t: &Self, q: usize, k: usize| -> Option<(usize, Src)> {
                let IrOp::Alu(AluOp::Fmul, s) = *t.instrs.get(q)? else { return None };
                let id = s[k].val.0 as usize;
                let IrOp::Alu(AluOp::Fneg, [c, ..]) = *t.instrs.get(id)? else { return None };
                matches!(t.instrs.get(c.val.0 as usize)?, IrOp::ConstF32(_)).then_some((id, c))
            };
            let (k, (nid, c)) = match (lit(self, p, 0), lit(self, p, 1)) {
                (Some(_), Some(_)) => return Err(MaliError::ArithConstant),
                (Some(x), None) => (0, x),
                (None, Some(x)) => (1, x),
                (None, None) => continue,
            };
            let w = usize::from(self.comps[p]).clamp(1, 4);
            let swizzle: [u8; 4] = core::array::from_fn(|i| c.swizzle[s[k].swizzle[i] as usize & 3]);
            let IrOp::ConstF32(v) = self.instrs[c.val.0 as usize] else { continue };
            let read = &swizzle[..w];
            let one = read.iter().all(|&l| v[l as usize & 3] == 0x3f80_0000);
            /* a vector whose lanes differ and one of which is 0.0 or 1.0
             * (`x * -vec2(1.0, 2.0)`, `x * -vec2(0.0, 2.0)`) in a highp
             * fragment shader: Mesa folds those lanes away (`FADD.f32 r1,
             * r1^, r1^` for the 2.0 lane alone, a pushed constant for the
             * 0.0 lane), ours keeps the product (negmul_highp_lanes*). The
             * same vectors are byte-equal in a mediump fragment shader and
             * in the vertex stage (negmul_mediump_lanes*, negmul_vs_lanes) */
            let splat = read.iter().all(|&l| v[l as usize & 3] == v[read[0] as usize & 3]);
            let folded_lane = read.iter().any(|&l| matches!(v[l as usize & 3] & !F32_NEG_ZERO, 0 | 0x3f80_0000));
            if self.highp_fragment && !splat && folded_lane {
                return Err(MaliError::ArithConstant);
            }
            let other = s[1 - k];
            /* `is_used_by_non_fsat` (nir_search_helpers.h:477-492): read by
             * `fsat` alone. The rule still fires in that pass, as the
             * replaced `fmin` leaves its `fmax` in the program until DCE
             * (nir_search.c:744-750), but :1105 turns `fsat(fneg(fmul(a,
             * b)))` back into `fsat(fmul(fneg(a), b))`, the product as
             * written (measured: negsat_clamp, negsat_clamp_one). Another
             * product with a negated literal may be the same value in
             * Mesa (CSE: two `a2 * -2.0` are one), read by more than the
             * `fsat`: not taken then (negsat_twice) */
            let twin = self.instrs.iter().enumerate().any(|(q, op)| {
                q != p
                    && matches!(*op, IrOp::Alu(AluOp::Fmul, _))
                    && (lit(self, q, 0).is_some() || lit(self, q, 1).is_some())
            });
            if !twin && self.readers(p).count() > 0 && self.readers(p).all(|(j, _)| self.fsat_of(j, p)) {
                continue;
            }
            let clamped = self.readers(p).any(|(_, op)| matches!(op, IrOp::Alu(AluOp::Fmin | AluOp::Fmax, _)));
            let placed = self.uses[nid] == Use::Alu(p as u32) && (other.val.0 as usize) < nid;
            let chain = matches!(self.instrs.get(other.val.0 as usize), Some(IrOp::Alu(AluOp::Fmul, _)));
            if one && !clamped {
                continue;
            }
            if clamped || chain || !placed {
                return Err(MaliError::ArithConstant);
            }
            self.instrs[nid] = IrOp::Alu(AluOp::Fmul, [Src { val: c.val, swizzle }, other, other]);
            self.comps[nid] = self.comps[p];
            self.instrs[p] = IrOp::Alu(AluOp::Fneg, [Src::id(ValueId(nid as u32)); 3]);
            any = true;
        }
        if any {
            self.changed = true;
            self.count();
        }
        Ok(())
    }

    /// The instructions that read value `id`, with their places.
    fn readers(&self, id: usize) -> impl Iterator<Item = (usize, &IrOp)> + '_ {
        self.instrs.iter().enumerate().filter(move |(_, op)| {
            let mut r = false;
            op.for_each_src(|x| r |= x.val.0 as usize == id);
            r
        })
    }

    /// Instruction `j`, a reader of `x`, is half of a `fsat` of it and
    /// nothing else reads it: `fmax(x, 0.0)` read only by `fmin(.., 1.0)`,
    /// either operand order (nir_opt_algebraic.py:1088, GLSL's `clamp(x,
    /// 0.0, 1.0)`). The other order, `fmax(fmin(x, 1.0), 0.0)` (:1091), is
    /// not taken: left alone it does not give Mesa's words (measured,
    /// negsat_minmax), so it stays refused.
    fn fsat_of(&self, j: usize, x: usize) -> bool {
        let splat = |t: &Self, s: Src, k: u32| matches!(t.instrs.get(s.val.0 as usize), Some(IrOp::ConstF32(v)) if s.swizzle.iter().all(|&l| v[l as usize & 3] == k));
        let pair = |t: &Self, id: usize, op: AluOp, k: u32, of: usize| match t.instrs.get(id) {
            Some(&IrOp::Alu(o, [a, b, _])) if o == op => {
                (a.val.0 as usize == of && splat(t, b, k)) || (b.val.0 as usize == of && splat(t, a, k))
            }
            _ => false,
        };
        let only_reader = |t: &Self, id: usize| match t.uses.get(id) {
            Some(Use::Alu(r)) => Some(*r as usize),
            _ => None,
        };
        let (zero, one) = (0, 0x3f80_0000);
        if pair(self, j, AluOp::Fmax, zero, x) {
            return only_reader(self, j).is_some_and(|r| pair(self, r, AluOp::Fmin, one, j));
        }
        false
    }

    /// The rules of nir_opt_algebraic.py on a compare of a negated value
    /// with a constant (:765-770): `flt(fneg(a), #b)` is `flt(fneg(b), a)`,
    /// `flt(#b, fneg(a))` is `flt(a, fneg(b))`, the same for `fge`;
    /// `feq` either way round is `feq(a, fneg(b))` and `fneu`
    /// `fneu(fneg(b), a)` (both commutative). On the IR (`Fle(x, y)` is
    /// `fge(y, x)`) a `<` or `<=` swaps its operands. The constant is
    /// a literal (`#b`, its `fneg` read through when it is one), negated in
    /// place, so a literal must be read by this compare alone; otherwise
    /// refused
    /// ([`MaliError::ArithConstant`]). Measured: `if (-u.x > 0.5)` around
    /// a kept branch, a highp `if (-v.x < 0.5) discard;`, a vertex select
    /// (tools/mali/testdata/negcmp_*).
    fn neg_compares(&mut self) -> Result<(), MaliError> {
        let n = self.instrs.len();
        let mut any = false;
        for i in 0..n {
            let IrOp::Alu(op @ (AluOp::Flt | AluOp::Fle | AluOp::Feq | AluOp::Fne), s) = self.instrs[i] else {
                continue;
            };
            let neg_of = |t: &Self, x: Src| match t.instrs.get(x.val.0 as usize) {
                Some(&IrOp::Alu(AluOp::Fneg, [a, ..]))
                    if !matches!(t.instrs.get(a.val.0 as usize), Some(IrOp::ConstF32(_))) =>
                {
                    Some(Src { val: a.val, swizzle: core::array::from_fn(|l| a.swizzle[x.swizzle[l] as usize & 3]) })
                }
                _ => None,
            };
            /* the constant `b`, a literal or a negated literal: its value
             * instruction and whether it is negated already */
            let const_of = |t: &Self, x: Src| match t.instrs.get(x.val.0 as usize) {
                Some(IrOp::ConstF32(_)) => Some((x.val.0 as usize, false)),
                Some(&IrOp::Alu(AluOp::Fneg, [c, ..]))
                    if matches!(t.instrs.get(c.val.0 as usize), Some(IrOp::ConstF32(_))) =>
                {
                    Some((c.val.0 as usize, true))
                }
                _ => None,
            };
            let (k, a, (cid, negated)) = match (neg_of(self, s[0]), neg_of(self, s[1])) {
                (Some(a), None) => match const_of(self, s[1]) {
                    Some(c) => (1, a, c),
                    None => continue,
                },
                (None, Some(a)) => match const_of(self, s[0]) {
                    Some(c) => (0, a, c),
                    None => continue,
                },
                _ => continue,
            };
            /* the constant's own source: the literal itself, or the fneg's */
            let b = if negated {
                let IrOp::Alu(AluOp::Fneg, [c, ..]) = self.instrs[s[k].val.0 as usize] else { continue };
                Src { val: c.val, swizzle: core::array::from_fn(|l| c.swizzle[s[k].swizzle[l] as usize & 3]) }
            } else {
                s[k]
            };
            /* a literal is negated in place: this compare must be its only reader */
            let alone = negated || self.uses[cid] == Use::Alu(i as u32);
            if !alone {
                return Err(MaliError::ArithConstant);
            }
            /* `fneg(b)`: a literal flips its sign, a negated literal is its
             * literal */
            if !negated {
                let IrOp::ConstF32(v) = self.instrs[cid] else { continue };
                self.instrs[cid] = IrOp::ConstF32(v.map(|x| x ^ 0x8000_0000));
            }
            let srcs = match op {
                AluOp::Feq => [a, b],
                AluOp::Fne => [b, a],
                /* `<` and `<=` turn round */
                _ if k == 1 => [b, a],
                _ => [a, b],
            };
            self.instrs[i] = IrOp::Alu(op, [srcs[0], srcs[1], srcs[1]]);
            any = true;
        }
        if any {
            self.changed = true;
            self.count();
        }
        Ok(())
    }

    /// The rules of nir_opt_algebraic.py on a compare with `-0.0`
    /// (:771-776): `flt` and `fge` take `0.0` on either side, `feq(-0.0,
    /// a)` is `feq(a, 0.0)` and `fneu(a, -0.0)` is `fneu(0.0, a)`, each
    /// matched both ways round (`feq` and `fneu` are commutative). They
    /// fire in the first `nir_opt_algebraic` after the constant is folded
    /// (`fneg(0.0)` is a GLSL `-0.0`), before any back end rule sees the
    /// compare, so here on the IR, which the vertex and the fragment
    /// builders both read. Measured: `if (v.x < -0.0) discard;` is
    /// `DISCARD.f32.lt r0, 0x0`, `-0.0 == v.x` is `DISCARD.f32.eq r0^.h0,
    /// 0x0` (tools/mali/testdata/negzero_*). The IR's `Fle(a, b)` is NIR's
    /// `fge(b, a)`, so it takes the `fge` rule.
    fn neg_zero_compares(&mut self) {
        let n = self.instrs.len();
        let mut any = false;
        for i in 0..n {
            let IrOp::Alu(op @ (AluOp::Flt | AluOp::Fle | AluOp::Feq | AluOp::Fne), mut s) = self.instrs[i] else {
                continue;
            };
            let w = usize::from(self.comps[i]).clamp(1, 4);
            let mut zero = [false; 2];
            for k in 0..2 {
                if let Some(z) = self.plus_zero(s[k], w) {
                    s[k] = z;
                    zero[k] = true;
                }
            }
            if zero == [false; 2] {
                continue;
            }
            let swap = match op {
                AluOp::Feq => zero == [true, false],
                AluOp::Fne => zero == [false, true],
                _ => false,
            };
            if swap {
                s.swap(0, 1);
            }
            self.instrs[i] = IrOp::Alu(op, s);
            any = true;
        }
        if any {
            self.changed = true;
            self.count();
        }
    }

    /// `s`'s `w` lanes read as `+0.0` where they are `-0.0`: an `fneg` of
    /// a constant zero read through (the GLSL `-0.0`), or a constant whose
    /// lanes are `-0.0` and which only compares read, made `+0.0` in place.
    /// `None`: a lane read is not `-0.0`.
    fn plus_zero(&mut self, s: Src, w: usize) -> Option<Src> {
        let lanes = |v: &[u32; 4], sw: &[u8; 4], k: u32| sw[..w].iter().all(|&c| v[c as usize & 3] == k);
        match *self.instrs.get(s.val.0 as usize)? {
            IrOp::Alu(AluOp::Fneg, [c, ..]) => {
                let IrOp::ConstF32(v) = *self.instrs.get(c.val.0 as usize)? else { return None };
                let swizzle = core::array::from_fn(|i| c.swizzle[s.swizzle[i] as usize & 3]);
                lanes(&v, &swizzle, 0).then_some(Src { val: c.val, swizzle })
            }
            IrOp::ConstF32(v) if lanes(&v, &s.swizzle, 0x8000_0000) => {
                let compares_only = self.instrs.iter().all(|op| match *op {
                    IrOp::Alu(AluOp::Flt | AluOp::Fle | AluOp::Feq | AluOp::Fne, _) => true,
                    o => {
                        let mut reads = false;
                        o.for_each_src(|x| reads |= x.val == s.val);
                        !reads
                    }
                });
                if !compares_only {
                    return None;
                }
                self.instrs[s.val.0 as usize] = IrOp::ConstF32(v.map(|x| if x == 0x8000_0000 { 0 } else { x }));
                Some(s)
            }
            _ => None,
        }
    }
}

/// The bits of `-0.0` in fp32.
const F32_NEG_ZERO: u32 = 0x8000_0000;

/// [`remap_srcs`] with a mutable closure.
fn remap_srcs_mut(op: &mut IrOp, f: &mut impl FnMut(&mut Src)) {
    let cell = core::cell::RefCell::new(f);
    remap_srcs(op, &|s: &mut Src| (cell.borrow_mut())(s));
}

/// Every source `op` reads, through `f`.
fn remap_srcs(op: &mut IrOp, f: &impl Fn(&mut Src)) {
    match op {
        IrOp::Alu(_, s) => s.iter_mut().for_each(f),
        IrOp::StoreOutput { src, .. } => f(src),
        IrOp::StoreLocal { index, src, .. } => {
            if let Some(i) = index {
                f(i);
            }
            f(src);
        }
        IrOp::LoadLocal { index: Some(i), .. } => f(i),
        IrOp::LoadUniformIdx { index, .. } => f(index),
        IrOp::Tex { coord, lod, .. } => {
            f(coord);
            match lod {
                crate::shaderc::online::ir::TexLod::Bias(x)
                | crate::shaderc::online::ir::TexLod::Lod(x)
                | crate::shaderc::online::ir::TexLod::Fetch(x) => f(x),
                crate::shaderc::online::ir::TexLod::Grad { ddx, ddy } => {
                    let (mut a, mut b) = (Src::id(*ddx), Src::id(*ddy));
                    f(&mut a);
                    f(&mut b);
                    *ddx = a.val;
                    *ddy = b.val;
                }
                crate::shaderc::online::ir::TexLod::Implicit => {}
            }
        }
        IrOp::TexSize { lod, .. } => f(lod),
        _ => {}
    }
}

/// The post-order of record `k`'s tree: operand 0, operand 1, the record.
fn post_order(rec: &[(u32, [Child; 2])], k: usize, out: &mut [Child], n: &mut usize) -> Result<(), MaliError> {
    for c in rec[k].1 {
        match c {
            Child::E(e) => post_order(rec, e, out, n)?,
            l => {
                *out.get_mut(*n).ok_or(MaliError::Rebalance)? = l;
                *n += 1;
            }
        }
    }
    *out.get_mut(*n).ok_or(MaliError::Rebalance)? = Child::E(k);
    *n += 1;
    Ok(())
}

/// `tree_to_vine` (opt_rebalance_tree.cpp:70-99) from record 0.
fn tree_to_vine(rec: &mut [(u32, [Child; 2])]) -> usize {
    let mut size = 0usize;
    let mut vine_tail = Child::E(0);
    let mut remainder = Some(rec[0].1[1]);
    while let Some(r) = remainder {
        let left = match r {
            Child::E(k) => match rec[k].1[0] {
                Child::E(l) => Some(l),
                Child::L(_) => None,
            },
            Child::L(_) => None,
        };
        match (left, r) {
            (Some(t), Child::E(rk)) => {
                /* rotate */
                rec[rk].1[0] = rec[t].1[1];
                rec[t].1[1] = r;
                remainder = Some(Child::E(t));
                if let Child::E(vt) = vine_tail {
                    rec[vt].1[1] = Child::E(t);
                }
            }
            _ => {
                /* move vine_tail down one */
                vine_tail = r;
                remainder = match r {
                    Child::E(k) => Some(rec[k].1[1]),
                    Child::L(_) => None,
                };
                size += 1;
            }
        }
    }
    size
}

/// `compression` (:101-113).
fn compression(rec: &mut [(u32, [Child; 2])], count: usize) {
    let mut scanner = 0usize;
    for _ in 0..count {
        let Child::E(child) = rec[scanner].1[1] else { return };
        rec[scanner].1[1] = rec[child].1[1];
        let Child::E(s) = rec[scanner].1[1] else { return };
        scanner = s;
        rec[child].1[1] = rec[scanner].1[0];
        rec[scanner].1[0] = Child::E(child);
    }
}

/// `vine_to_tree` (:115-123).
fn vine_to_tree(rec: &mut [(u32, [Child; 2])], size: usize) {
    let mut n = size as isize - 1;
    let mut m = n / 2;
    while m > 0 {
        compression(rec, m as usize);
        n -= m + 1;
        m = n / 2;
    }
}

#[cfg(test)]
#[path = "glsl_tests.rs"]
mod tests;
