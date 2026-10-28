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
    AluOp, IoKind, IoSlot, Module, Node as IrNode, Op as IrOp, Src, ValueId, SWIZZLE_IDENTITY,
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

/// A stage's instructions and their widths, rewritten.
pub(super) type Rewritten<'s> = (&'s [IrOp], &'s [u8]);

/// Run the two passes on `m`. `None`: they change nothing (and refuse
/// nothing); otherwise the stage's instructions and widths after them,
/// in the same node order.
pub(super) fn passes<'s>(m: &Module<'_>, scratch: &Arena<'s>) -> Result<Option<Rewritten<'s>>, MaliError> {
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
    let mut t = Tree { m, instrs, comps, uses, locals, pos, changed: false };
    t.count();
    t.rebalance()?;
    t.reassociate()?;
    t.negneg();
    if !t.changed {
        return Ok(None);
    }
    let Tree { instrs, comps, .. } = t;
    Ok(Some((&*instrs, &*comps)))
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
fn columns(gl_type: u16) -> u8 {
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
                if m.locals.get(l).is_some_and(|d| d.array_len > 1) {
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
