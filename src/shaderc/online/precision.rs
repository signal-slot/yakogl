// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES precision lowering's class of every expression, from the tree.
//!
//! Mesa 26.2.1 decides the width of every value of an ES shader on a GPU
//! with the `fp16` cap in `lower_precision` (src/compiler/glsl/
//! lower_precision.cpp), right after `ast_to_hir` and before any inlining
//! or optimisation. This module is that pass's first half on our syntax
//! tree: `find_lowerable_rvalues` gives each expression one of three
//! states, and the topmost Medium node of a joined chain is the root of a
//! tree computed at 16 bits (docs/notes/mali-precision.md, Part A).
//!
//! It runs on the tree, before the lowering, for the reason Mesa runs it
//! before inlining: the class of an expression depends on the declarations
//! of its scope, a user function's declared return precision and its
//! parameters' precision, and on which constructors and `?:` the source
//! wrote. The lowering inlines calls, aliases parameters and turns
//! constructors into lanes, and after that none of these boundaries has a
//! node left. A body is classed once, as Mesa classes a function once:
//! every inlined copy of it reads the same table.
//!
//! The result is two bytes of [`Prec`] per expression, read by
//! `lower.rs`, which records them per instruction (`Module::prec`) and per
//! local (`Module::local_prec`). No back end reads those but the one that
//! lowers precision (the Mali's); nothing here changes the instructions.
//!
//! The rule, as `lower_precision.cpp` has it (line numbers of 26.2.1):
//!
//! - A constant is None; an int or uint value is High (no `int16` cap,
//!   `can_lower_type` :153-182); a read of a variable, a struct member or
//!   an array element is the declared precision (:349-382).
//! - An operation joins its operands' classes: a High operand makes it
//!   High, else a Medium one Medium (`pop_stack_entry`, :224-283).
//! - A dereference's children (the array index, the struct) are separate
//!   trees (`get_parent_relation`, :315-333).
//! - A call's arguments are separate trees; the result is the declared
//!   return precision of a user function, the sampler's for a look-up, and
//!   for another built-in Medium when every argument that is not a
//!   constant is a Medium root (`handle_call`, :419-541).
//! - A compiler temporary (a constructor's `vec_ctor`, a `?:`'s
//!   `conditional_tmp`, `&&`'s `and_tmp`) is None until assigned, Medium
//!   after a Medium assignment, High after any other assignment that is
//!   not a constant (`visit_leave(ir_assignment)`, :585-608).
//! - A Medium root and every node below it on joined edges is lowered,
//!   unless the root is a bare dereference (`handle_rvalue`, :835-843).
//!
//! What `ast_to_hir` folds matters as much: an argument of a call or a
//! constructor that is a constant expression is folded to a constant
//! there, at 32 bits (`process_parameters`, ast_function.cpp:61; and
//! :1198, :2442), and a call whose arguments are all constant is folded
//! whole (:659). An operator is not folded (`c * c` of a `const mediump
//! float` is computed at 16 bits, probe p1 of the note). A folded tree is
//! classed High: its value is the 32-bit fold.

use super::arena::{AVec, Arena};
use super::ast::{Ast, BaseType, BinOp, Cond, Decl, DeclId, Expr, ExprId, Precision, SamplerType, Stmt, StmtId};
use super::ast::{SamplerDim, SamplerRet, StorageQualifier, UnOp};
use super::ir::Prec;
use super::sema::{self, ExprTypes, Stage};
use super::R;

/// The two answers for one expression, as [`Precs::instr`] and
/// [`Precs::temp`] give them.
const INSTR_SHIFT: u8 = 0;
const CLASS_SHIFT: u8 = 3;
/// One [`Prec`] in a table byte.
const PREC_MASK: u8 = 7;
/// The scan reached the expression and recorded its class: an entry
/// without it is [`Prec::Unknown`].
const SEEN: u8 = 0x40;
/// The expression is a constant expression (`constant_expression_value`
/// folds it): [`Precs::folds`].
const FOLDS_BIT: u8 = 0x80;

/// What the lowering asks: the class of each expression, of each declared
/// variable, and of each function's result.
#[derive(Clone, Copy)]
pub struct Precs<'a> {
    /// Per [`ExprId`]: the [`Prec`] of the instructions that compute the
    /// expression (bits 0-2), and its A.3 class (bits 3-5).
    exprs: &'a [u8],
    /// Per [`DeclId`]: a variable's declared class, a function's result's,
    /// and a struct's default float (bits 0-1) and int (bits 2-3)
    /// precision at its declaration, as [`Precision`] numbers.
    decls: &'a [u8],
}

impl<'a> Precs<'a> {
    /// No table: every answer is [`Prec::None`].
    pub const EMPTY: Precs<'static> = Precs { exprs: &[], decls: &[] };

    /// The [`Prec`] the instructions that compute `e` record: Medium when
    /// `e` is inside a tree Mesa computes at 16 bits (a lowered tree, or a
    /// lowered built-in's body), High for any other float or bool value,
    /// None for an int, a struct or no value.
    ///
    /// [`Prec::Unknown`] for an expression the scan did not reach: the
    /// lowering records it, and a reader of precision refuses it.
    pub fn instr(&self, e: ExprId) -> Prec {
        match self.exprs.get(e.0 as usize).copied() {
            Some(b) if b & SEEN != 0 => prec_of((b >> INSTR_SHIFT) & PREC_MASK),
            _ => Prec::Unknown,
        }
    }

    /// `e`'s class (A.3): what a compiler temporary holding its value is
    /// (Mesa's `vec_ctor`, `conditional_tmp`, a call's return value).
    /// The scan reached `e` and recorded its class. An expression the
    /// lowering lowers but the scan did not reach is a defect of the scan:
    /// its values record [`Prec::Unknown`] for no rule's reason.
    pub fn seen(&self, e: ExprId) -> bool {
        self.exprs.get(e.0 as usize).is_some_and(|b| b & SEEN != 0)
    }

    /// `e` is a constant expression Mesa's `ast_to_hir` folds where it is
    /// a call's or a constructor's argument (at 32 bits).
    pub fn folds(&self, e: ExprId) -> bool {
        self.exprs.get(e.0 as usize).is_some_and(|b| b & FOLDS_BIT != 0)
    }

    pub fn class(&self, e: ExprId) -> Prec {
        match self.exprs.get(e.0 as usize).copied() {
            Some(b) if b & SEEN != 0 => prec_of((b >> CLASS_SHIFT) & PREC_MASK),
            _ => Prec::Unknown,
        }
    }

    /// The class a variable `d` was declared with (`handle_precision` of
    /// its type and precision): Medium for a mediump or lowp float or bool,
    /// High for a highp one and for an int, uint or struct, None for a bool
    /// (which has no precision). For a function, its result's class.
    pub fn decl(&self, d: DeclId) -> Prec {
        prec_of(self.decls.get(d.0 as usize).copied().unwrap_or(0) & 3)
    }

    /// The class of the top-level variable `path` names (`v`, `s.a.b`,
    /// `v[2]`, `s[1].a`: the reflection's spelling), or of a member it
    /// reaches. None when no declaration has the name.
    pub fn global_path(&self, ast: &Ast<'a>, path: &str) -> Prec {
        let root = path.split(['.', '[']).next().unwrap_or(path);
        let Some(id) =
            ast.top.iter().copied().find(|&d| matches!(*ast.decl(d), Decl::Var { name, .. } if name == root))
        else {
            return Prec::None;
        };
        let Decl::Var { ty, .. } = *ast.decl(id) else { return Prec::None };
        let mut class = self.decl(id);
        let mut base = ty.base;
        for seg in path.split('.').skip(1) {
            let member = seg.split('[').next().unwrap_or(seg);
            let BaseType::Struct(sname) = base else { return class };
            let Some((m, d)) = self.member(ast, sname, member) else { return Prec::None };
            class = handle(m.ty.base, resolved(m.ty.precision, m.ty.base, &d));
            base = m.ty.base;
        }
        class
    }

    /// A struct's member and the defaults in force where the struct was
    /// declared (the struct found by name, as `lower.rs` finds it).
    fn member(&self, ast: &Ast<'a>, sname: &str, member: &str) -> Option<(super::ast::Member<'a>, Defaults)> {
        let (i, members) = ast.decls.iter().enumerate().find_map(|(i, d)| match *d {
            Decl::Struct { name, members, .. } if name == sname => Some((i, members)),
            _ => None,
        })?;
        let m = ast.members[members.indices()].iter().find(|m| m.name == member).copied()?;
        let b = self.decls.get(i).copied().unwrap_or(0);
        let d = Defaults { float: precision_of(b & 3), int: precision_of((b >> 2) & 3), ..Defaults::NONE };
        Some((m, d))
    }
}

/// The class Mesa gives each expression of `ast`, and the instructions'
/// [`Prec`] that follows from it, for a shader of `stage`.
pub fn classify<'a>(a: &Arena<'a>, ast: &Ast<'a>, types: &ExprTypes<'a>, stage: Stage) -> R<'a, Precs<'a>> {
    let exprs = a.alloc_filled(ast.exprs.len(), 0u8)?;
    let decls = a.alloc_filled(ast.decls.len(), 0u8)?;
    let info = a.alloc_filled(ast.exprs.len(), 0u8)?;
    let mut s = Scan {
        a,
        ast,
        types,
        stage,
        exprs,
        decls,
        info,
        vars: AVec::new(),
        defaults: Defaults::of(stage),
        structs: AVec::new(),
    };
    for &id in ast.top {
        s.decl(id)?;
    }
    /* The scan's own tables are garbage now; the two answers stay. */
    s.vars.release(a);
    s.structs.release(a);
    Ok(Precs { exprs: s.exprs, decls: s.decls })
}

fn prec_of(b: u8) -> Prec {
    match b {
        1 => Prec::High,
        2 => Prec::Medium,
        3 => Prec::Unknown,
        _ => Prec::None,
    }
}

/// Three bits of a table entry.
const fn bits_of(p: Prec) -> u8 {
    match p {
        Prec::None => 0,
        Prec::High => 1,
        Prec::Medium => 2,
        Prec::Unknown => 3,
    }
}

fn precision_bits(p: Precision) -> u8 {
    match p {
        Precision::Default => 0,
        Precision::Low => 1,
        Precision::Medium => 2,
        Precision::High => 3,
    }
}

fn precision_of(b: u8) -> Precision {
    match b {
        1 => Precision::Low,
        2 => Precision::Medium,
        3 => Precision::High,
        _ => Precision::Default,
    }
}

/// `pop_stack_entry`'s join of two classes over a combined edge.
///
/// An [`Prec::Unknown`] operand (a class this module could not settle)
/// leaves the result Unknown unless a High one decides it.
fn join(x: Prec, y: Prec) -> Prec {
    match (x, y) {
        (Prec::High, _) | (_, Prec::High) => Prec::High,
        (Prec::Unknown, _) | (_, Prec::Unknown) => Prec::Unknown,
        (Prec::Medium, _) | (_, Prec::Medium) => Prec::Medium,
        _ => Prec::None,
    }
}

fn is_int(b: BaseType<'_>) -> bool {
    matches!(b, BaseType::Int | BaseType::IVec(_) | BaseType::Uint | BaseType::UVec(_))
}

/// `can_lower_type` (lower_precision.cpp:153-182) with panfrost's caps:
/// a float, a bool and a sampler, never an int (no `int16`) or a struct.
fn lowerable(b: BaseType<'_>) -> bool {
    matches!(
        b,
        BaseType::Float
            | BaseType::Vec(_)
            | BaseType::Mat(..)
            | BaseType::Bool
            | BaseType::BVec(_)
            | BaseType::Sampler(_)
    )
}

/// `handle_precision` (:295-312): the class of a value of type `b` declared
/// with precision `p` (already resolved against the defaults).
fn handle(b: BaseType<'_>, p: Precision) -> Prec {
    if matches!(b, BaseType::Void) {
        return Prec::None;
    }
    if !lowerable(b) {
        return Prec::High;
    }
    match p {
        Precision::Default => Prec::None,
        Precision::High => Prec::High,
        Precision::Medium | Precision::Low => Prec::Medium,
    }
}

/// The precision of a declaration of `base` that writes `p`, under the
/// defaults in force (GLSL ES 1.00 4.5.3, ES 3.00 4.5.4; Mesa's
/// `select_gles_precision`, ast_to_hir.cpp:2764-2800). A bool, a struct and
/// a sampler type with no default have none.
fn resolved(p: Precision, base: BaseType<'_>, d: &Defaults) -> Precision {
    if p != Precision::Default {
        return p;
    }
    match base {
        BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..) => d.float,
        BaseType::Int | BaseType::IVec(_) | BaseType::Uint | BaseType::UVec(_) => d.int,
        BaseType::Sampler(t) => d.samplers[sampler_index(t)],
        _ => Precision::Default,
    }
}

fn sampler_index(t: SamplerType) -> usize {
    let dim = match t.dim {
        SamplerDim::D2 => 0,
        SamplerDim::D3 => 1,
        SamplerDim::Cube => 2,
        SamplerDim::D2Array => 3,
    };
    let ret = match t.ret {
        SamplerRet::Float => 0,
        SamplerRet::Int => 1,
        SamplerRet::Uint => 2,
    };
    dim * 6 + ret * 2 + usize::from(t.shadow)
}

/// The default precisions in force (`precision mediump float;` and the
/// predeclared ones).
#[derive(Clone, Copy)]
struct Defaults {
    float: Precision,
    int: Precision,
    samplers: [Precision; 24],
}

impl Defaults {
    const NONE: Defaults =
        Defaults { float: Precision::Default, int: Precision::Default, samplers: [Precision::Default; 24] };

    /// The predeclared defaults of `stage` (ES 1.00 4.5.3, ES 3.00 4.5.4):
    /// float and int highp in the vertex stage, int mediump and float none
    /// in the fragment stage, `sampler2D` and `samplerCube` lowp in both.
    fn of(stage: Stage) -> Defaults {
        let mut d = Defaults::NONE;
        let (f, i) = match stage {
            Stage::Vertex => (Precision::High, Precision::High),
            Stage::Fragment => (Precision::Default, Precision::Medium),
        };
        d.float = f;
        d.int = i;
        d.samplers[sampler_index(SamplerType::new(SamplerDim::D2, SamplerRet::Float, false))] = Precision::Low;
        d.samplers[sampler_index(SamplerType::new(SamplerDim::Cube, SamplerRet::Float, false))] = Precision::Low;
        d
    }

    /// The defaults after a scope that began at `outer`. Mesa's
    /// `add_default_precision_qualifier` (glsl_symbol_table.cpp:194-207)
    /// replaces a default that exists in any scope, so a `precision`
    /// statement in a block or a function changes it for the rest of the
    /// shader (review probes v_inner2, v_inner4, f_inner2). Only a type
    /// that had no default when the scope began loses the scope's one.
    fn leave(&mut self, outer: Defaults) {
        if outer.float == Precision::Default {
            self.float = Precision::Default;
        }
        if outer.int == Precision::Default {
            self.int = Precision::Default;
        }
        for (s, o) in self.samplers.iter_mut().zip(outer.samplers) {
            if o == Precision::Default {
                *s = Precision::Default;
            }
        }
    }

    /// `precision p base;`
    fn set(&mut self, p: Precision, base: BaseType<'_>) {
        match base {
            BaseType::Float => self.float = p,
            BaseType::Int => self.int = p,
            BaseType::Sampler(t) => self.samplers[sampler_index(t)] = p,
            _ => {}
        }
    }
}

/// How Mesa's IR has an expression at `lower_precision` time, which decides
/// how its class reaches its parent and its instructions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An `ir_constant`: a literal, or a constructor or built-in call
    /// `ast_to_hir` folded.
    Const = 0,
    /// A dereference: a variable, an array element, a struct member.
    Deref = 1,
    /// An expression (an operator, a swizzle, a conversion) joined with its
    /// operands.
    Op = 2,
    /// The read of a compiler temporary that assignments or a call fill: a
    /// constructor's, a `?:`'s, a call's result, an assignment's value.
    Temp = 3,
}

/// The scan's own facts about one expression, kept in `Scan::info`.
const KIND_MASK: u8 = 3;
/// The expression is a constant expression: `constant_expression_value`
/// folds it, so as a call's or a constructor's argument it is a constant.
const FOLDS: u8 = 4;
/// Mesa's `ast_to_hir` emits instructions for it (a call, a temporary), so
/// `&&` with it on the right is an `if` into a temporary (:1672).
const EMITS: u8 = 8;
/// A call or constructor `ast_to_hir` folded whole: its own operations are
/// the fold's, at 32 bits.
const FOLDED: u8 = 16;
/// A `?:` of a constant condition that is its first side ([`CHOSE_F`]:
/// its second).
const CHOSE_T: u8 = 32;
const CHOSE_F: u8 = 64;

#[derive(Clone, Copy)]
struct Var<'a> {
    name: &'a str,
    /// The variable's class (`handle_precision`).
    class: Prec,
    /// A `const` whose initialiser folds: a read of it folds.
    folds: bool,
    /// A sampler (or an array of them) declared lowp or mediump: a look-up
    /// through it is Medium (`handle_call`, :471-487).
    medium_sampler: bool,
    /// A `const` scalar whose value [`Scan::constant`] works out.
    value: Option<Konst>,
}

/// A scalar constant's value, as far as [`Scan::constant`] evaluates.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Konst {
    Bool(bool),
    /// A float: an f32 value, held exactly.
    Num(f64),
}

struct Scan<'x, 'a> {
    a: &'x Arena<'a>,
    ast: &'x Ast<'a>,
    types: &'x ExprTypes<'a>,
    stage: Stage,
    exprs: &'a mut [u8],
    decls: &'a mut [u8],
    info: &'a mut [u8],
    /// The variables in scope, innermost last.
    vars: AVec<'a, Var<'a>>,
    defaults: Defaults,
    /// Every struct declared so far, by name, innermost last.
    structs: AVec<'a, (&'a str, DeclId)>,
}

impl<'x, 'a> Scan<'x, 'a> {
    fn ty(&self, e: ExprId) -> BaseType<'a> {
        let t = self.types.of(e);
        if t.is_unknown() {
            BaseType::Void
        } else {
            t.base
        }
    }

    fn info(&self, e: ExprId) -> u8 {
        self.info.get(e.0 as usize).copied().unwrap_or(0)
    }

    fn kind(&self, e: ExprId) -> Kind {
        match self.info(e) & KIND_MASK {
            0 => Kind::Const,
            1 => Kind::Deref,
            2 => Kind::Op,
            _ => Kind::Temp,
        }
    }

    fn folds(&self, e: ExprId) -> bool {
        self.info(e) & FOLDS != 0
    }

    fn emits(&self, e: ExprId) -> bool {
        self.info(e) & EMITS != 0
    }

    fn class(&self, e: ExprId) -> Prec {
        prec_of((self.exprs.get(e.0 as usize).copied().unwrap_or(0) >> CLASS_SHIFT) & PREC_MASK)
    }

    fn set(&mut self, e: ExprId, kind: Kind, class: Prec, flags: u8) {
        if let Some(i) = self.info.get_mut(e.0 as usize) {
            *i = kind as u8 | flags;
        }
        if let Some(x) = self.exprs.get_mut(e.0 as usize) {
            *x = (*x & !(PREC_MASK << CLASS_SHIFT)) | (bits_of(class) << CLASS_SHIFT);
        }
    }

    fn set_instr(&mut self, e: ExprId, p: Prec) {
        if let Some(x) = self.exprs.get_mut(e.0 as usize) {
            *x = (*x & !(PREC_MASK << INSTR_SHIFT)) | (bits_of(p) << INSTR_SHIFT) | SEEN;
        }
        if self.folds(e) {
            if let Some(x) = self.exprs.get_mut(e.0 as usize) {
                *x |= FOLDS_BIT;
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<Var<'a>> {
        self.vars.as_slice().iter().rev().find(|v| v.name == name).copied()
    }

    fn cut(&mut self, n: usize) {
        while self.vars.len() > n {
            self.vars.pop();
        }
    }

    /* ---- declarations and statements ---------------------------------- */

    fn decl(&mut self, id: DeclId) -> R<'a, ()> {
        match *self.ast.decl(id) {
            Decl::Precision { precision, base, .. } => self.defaults.set(precision, base),
            Decl::Var { storage, ty, name, init, .. } => {
                if let Some(e) = init {
                    self.root(e)?;
                }
                let p = resolved(ty.precision, ty.base, &self.defaults);
                let class = handle(ty.base, p);
                if let Some(d) = self.decls.get_mut(id.0 as usize) {
                    *d = bits_of(class);
                }
                let folds = storage == StorageQualifier::Const && init.is_some_and(|e| self.folds(e));
                let medium_sampler = ty.base.is_sampler() && matches!(p, Precision::Low | Precision::Medium);
                let value = if storage == StorageQualifier::Const && !ty.is_array() {
                    init.and_then(|e| self.constant(e))
                } else {
                    None
                };
                self.vars.push(self.a, Var { name, class, folds, medium_sampler, value })?;
            }
            Decl::Struct { name, .. } => {
                if let Some(d) = self.decls.get_mut(id.0 as usize) {
                    *d = precision_bits(self.defaults.float) | (precision_bits(self.defaults.int) << 2);
                }
                self.structs.push(self.a, (name, id))?;
            }
            Decl::Function { ret, params, body, .. } => {
                let class = handle(ret.base, resolved(ret.precision, ret.base, &self.defaults));
                if let Some(d) = self.decls.get_mut(id.0 as usize) {
                    *d = bits_of(class);
                }
                if let Some(b) = body {
                    let (mark, saved) = (self.vars.len(), self.defaults);
                    for p in &self.ast.params[params.indices()] {
                        if let Some(name) = p.name {
                            let pp = resolved(p.ty.precision, p.ty.base, &self.defaults);
                            let medium_sampler =
                                p.ty.base.is_sampler() && matches!(pp, Precision::Low | Precision::Medium);
                            let class = handle(p.ty.base, pp);
                            self.vars.push(self.a, Var { name, class, folds: false, medium_sampler, value: None })?;
                        }
                    }
                    self.stmt(b)?;
                    self.cut(mark);
                    self.defaults.leave(saved);
                }
            }
            Decl::Invariant { .. } => {}
        }
        Ok(())
    }

    fn stmt(&mut self, id: StmtId) -> R<'a, ()> {
        match *self.ast.stmt(id) {
            Stmt::Expr(Some(e)) | Stmt::Return(Some(e)) | Stmt::Case(Some(e)) => self.root(e)?,
            Stmt::Expr(None) | Stmt::Return(None) | Stmt::Case(None) => {}
            Stmt::Break | Stmt::Continue | Stmt::Discard => {}
            Stmt::Decl(r) => {
                for &d in &self.ast.decl_ids[r.indices()] {
                    self.decl(d)?;
                }
            }
            Stmt::Block(r) => {
                let (mark, saved) = (self.vars.len(), self.defaults);
                for &s in &self.ast.stmt_ids[r.indices()] {
                    self.stmt(s)?;
                }
                self.cut(mark);
                self.defaults.leave(saved);
            }
            Stmt::If { cond, then_s, else_s } => {
                self.root(cond)?;
                self.scoped(then_s)?;
                if let Some(e) = else_s {
                    self.scoped(e)?;
                }
            }
            Stmt::While { cond, body } => {
                let mark = self.vars.len();
                self.cond(cond)?;
                self.scoped(body)?;
                self.cut(mark);
            }
            Stmt::DoWhile { body, cond } => {
                self.scoped(body)?;
                self.root(cond)?;
            }
            Stmt::For { init, cond, step, body } => {
                let (mark, saved) = (self.vars.len(), self.defaults);
                if let Some(i) = init {
                    self.stmt(i)?;
                }
                if let Some(c) = cond {
                    self.cond(c)?;
                }
                if let Some(e) = step {
                    self.root(e)?;
                }
                self.scoped(body)?;
                self.cut(mark);
                self.defaults.leave(saved);
            }
            Stmt::Switch { sel, body } => {
                self.root(sel)?;
                self.scoped(body)?;
            }
        }
        Ok(())
    }

    fn scoped(&mut self, id: StmtId) -> R<'a, ()> {
        let (mark, saved) = (self.vars.len(), self.defaults);
        self.stmt(id)?;
        self.cut(mark);
        self.defaults.leave(saved);
        Ok(())
    }

    fn cond(&mut self, c: Cond) -> R<'a, ()> {
        match c {
            Cond::Expr(e) => self.root(e),
            Cond::Decl(d) => self.decl(d),
        }
    }

    /// An expression that is a tree of its own: an initialiser, a
    /// condition, a statement, a returned value. Its class first, bottom
    /// up; then which of its nodes are lowered, top down.
    fn root(&mut self, e: ExprId) -> R<'a, ()> {
        self.scan(e)?;
        self.mark(e, false, false);
        Ok(())
    }

    /* ---- the class, bottom up ----------------------------------------- */

    /// The class of a built-in variable (GLSL ES 1.00 and 3.00 section 7;
    /// Mesa's builtin_variables.cpp), `None` for a name that is not one.
    fn builtin_var(&self, name: &str) -> Option<Prec> {
        let es3 = self.ast.es3;
        Some(match (self.stage, name) {
            (Stage::Vertex, "gl_Position") => Prec::High,
            /* mediump in ES 1.00, highp in 3.00 (:1587-1591) */
            (Stage::Vertex, "gl_PointSize") => {
                if es3 {
                    Prec::High
                } else {
                    Prec::Medium
                }
            }
            (Stage::Vertex, "gl_VertexID" | "gl_InstanceID") => Prec::High,
            (Stage::Fragment, "gl_FragCoord") => {
                if es3 {
                    Prec::High
                } else {
                    Prec::Medium
                }
            }
            (Stage::Fragment, "gl_PointCoord" | "gl_FragColor" | "gl_FragData") => Prec::Medium,
            (Stage::Fragment, "gl_FragDepth") => Prec::High,
            /* a bool, which has no precision */
            (Stage::Fragment, "gl_FrontFacing") => Prec::None,
            /* a struct; its members are highp (builtin_types.cpp:37-41) */
            (_, "gl_DepthRange") => Prec::High,
            _ => return None,
        })
    }

    /// A struct member's class: its precision, or the default in force
    /// where the struct was declared.
    fn member_class(&self, sname: &str, member: &str) -> Prec {
        if sname == sema::DEPTH_RANGE_PARAMETERS {
            return Prec::High;
        }
        let Some(&(_, id)) = self.structs.as_slice().iter().rev().find(|s| s.0 == sname) else {
            /* not declared before its use: the type checker refuses that */
            return Prec::High;
        };
        let Decl::Struct { members, .. } = *self.ast.decl(id) else { return Prec::High };
        let Some(m) = self.ast.members[members.indices()].iter().find(|m| m.name == member) else {
            return Prec::High;
        };
        let b = self.decls.get(id.0 as usize).copied().unwrap_or(0);
        let d = Defaults { float: precision_of(b & 3), int: precision_of((b >> 2) & 3), ..Defaults::NONE };
        handle(m.ty.base, resolved(m.ty.precision, m.ty.base, &d))
    }

    /// The precision a dereference chain reads (`ir_dereference::
    /// precision()`, ir.h): a variable's, an array element's array's, a
    /// member's; a temporary's class. None when `e` is no dereference.
    fn deref_class(&self, e: ExprId) -> Prec {
        match self.kind(e) {
            Kind::Deref | Kind::Temp => self.class(e),
            _ => Prec::None,
        }
    }

    /// The value of the scalar constant expression `e`, for the forms
    /// measured against Mesa 26.2.1 (review probes p01, p11, u3, u7): bool
    /// and float literals and `const` scalars of them; `!`, `&&`, `||`,
    /// `^^`, `==` and `!=` of bools; unary `-` and `+`, `+ - *` in 32-bit
    /// floats and the compares of floats; one component of a `vecN` of
    /// float scalars. `None` for every other form: an int or uint (wrapping,
    /// division, shifts, `0u - 1u`, third review B-C), a conversion or a
    /// converting constructor (`ivec2(1.5, 0.0).x`, B-D), a bool constructor,
    /// a matrix, a built-in. `ast_to_hir` may still fold those, and the
    /// caller records [`Prec::Unknown`] rather than guess.
    fn constant(&self, e: ExprId) -> Option<Konst> {
        let num = |k: Option<Konst>| match k {
            Some(Konst::Num(x)) => Some(x),
            _ => None,
        };
        let boolean = |k: Option<Konst>| match k {
            Some(Konst::Bool(b)) => Some(b),
            _ => None,
        };
        Some(match *self.ast.expr(e) {
            Expr::BoolLit(b) => Konst::Bool(b),
            Expr::FloatLit(x) => Konst::Num(f64::from(x)),
            Expr::Ident(n) => self.lookup(n)?.value?,
            Expr::Unary(UnOp::Not, x) => Konst::Bool(!boolean(self.constant(x))?),
            Expr::Unary(UnOp::Minus, x) => Konst::Num(-num(self.constant(x))?),
            Expr::Unary(UnOp::Plus, x) => Konst::Num(num(self.constant(x))?),
            /* a component of a constructor of scalar literals,
             * `vec2(1.0, 0.0).x` */
            Expr::Field(b, field) if field.len() == 1 => {
                let Expr::Call { name, args } = *self.ast.expr(b) else { return None };
                /* a float vector of float scalars: no conversion */
                if !matches!(sema::constructor_type(name, self.ast.es3)?, BaseType::Vec(_)) {
                    return None;
                }
                let k = match field.as_bytes()[0] {
                    b'x' | b'r' | b's' => 0,
                    b'y' | b'g' | b't' => 1,
                    b'z' | b'b' | b'p' => 2,
                    b'w' | b'a' | b'q' => 3,
                    _ => return None,
                };
                let ids = &self.ast.expr_ids[args.indices()];
                if ids.iter().any(|&x| self.ty(x) != BaseType::Float) {
                    return None;
                }
                let x = if ids.len() == 1 { ids[0] } else { *ids.get(k)? };
                self.constant(x)?
            }
            Expr::Binary(op, x, y) => {
                let (a, b) = (self.constant(x)?, self.constant(y)?);
                match (op, a, b) {
                    /* 32-bit floats, as ast_to_hir folds them
                     * (ir_expression_operation.py:646, the C operator on
                     * the operand type) */
                    (BinOp::Add | BinOp::Sub | BinOp::Mul, Konst::Num(a), Konst::Num(b)) => {
                        let (a, b) = (a as f32, b as f32);
                        Konst::Num(f64::from(match op {
                            BinOp::Add => a + b,
                            BinOp::Sub => a - b,
                            _ => a * b,
                        }))
                    }
                    (BinOp::AndAnd, Konst::Bool(a), Konst::Bool(b)) => Konst::Bool(a && b),
                    (BinOp::OrOr, Konst::Bool(a), Konst::Bool(b)) => Konst::Bool(a || b),
                    (BinOp::XorXor | BinOp::Ne, Konst::Bool(a), Konst::Bool(b)) => Konst::Bool(a != b),
                    (BinOp::Eq, Konst::Bool(a), Konst::Bool(b)) => Konst::Bool(a == b),
                    (BinOp::Lt, Konst::Num(a), Konst::Num(b)) => Konst::Bool(a < b),
                    (BinOp::Gt, Konst::Num(a), Konst::Num(b)) => Konst::Bool(a > b),
                    (BinOp::Le, Konst::Num(a), Konst::Num(b)) => Konst::Bool(a <= b),
                    (BinOp::Ge, Konst::Num(a), Konst::Num(b)) => Konst::Bool(a >= b),
                    (BinOp::Eq, Konst::Num(a), Konst::Num(b)) => Konst::Bool(a == b),
                    (BinOp::Ne, Konst::Num(a), Konst::Num(b)) => Konst::Bool(a != b),
                    _ => return None,
                }
            }
            _ => return None,
        })
    }

    /// `e` reads a `const` mediump variable whose initialiser folds
    /// (directly, or an element or member of one).
    fn folded_medium_const(&self, e: ExprId) -> bool {
        match *self.ast.expr(e) {
            Expr::Ident(n) => self.lookup(n).is_some_and(|v| v.folds && v.class == Prec::Medium),
            Expr::Index(b, _) | Expr::Field(b, _) => self.folded_medium_const(b),
            _ => false,
        }
    }

    /// Whether a look-up through the sampler `e` returns a Medium value:
    /// the precision of the variable it reads (`variable_referenced`). A
    /// sampler in a struct reads the struct's variable, which has no
    /// precision: High.
    fn medium_sampler(&self, e: ExprId) -> bool {
        match *self.ast.expr(e) {
            Expr::Ident(n) => self.lookup(n).is_some_and(|v| v.medium_sampler),
            Expr::Index(b, _) => self.medium_sampler(b),
            _ => false,
        }
    }

    /// The class (and the scan's facts) of `e` and every node below it.
    fn scan(&mut self, e: ExprId) -> R<'a, ()> {
        let ty = self.ty(e);
        let typed = |c: Prec| if lowerable(ty) || matches!(ty, BaseType::Void) { c } else { Prec::High };
        match *self.ast.expr(e) {
            Expr::IntLit(_) | Expr::UintLit(_) | Expr::Length(_) => self.set(e, Kind::Const, Prec::High, FOLDS),
            Expr::FloatLit(_) | Expr::BoolLit(_) => self.set(e, Kind::Const, Prec::None, FOLDS),
            Expr::Ident(n) => {
                if super::reflect::builtin_constant(n, self.ast.es3).is_some() {
                    self.set(e, Kind::Const, Prec::High, FOLDS);
                } else if let Some(v) = self.lookup(n) {
                    self.set(e, Kind::Deref, typed(v.class), if v.folds { FOLDS } else { 0 });
                } else {
                    let c = self.builtin_var(n).unwrap_or(Prec::High);
                    self.set(e, Kind::Deref, typed(c), 0);
                }
            }
            Expr::Unary(op, x) => {
                self.scan(x)?;
                let f = self.info(x) & (FOLDS | EMITS);
                match op {
                    UnOp::Plus | UnOp::Minus | UnOp::Not | UnOp::BitNot => {
                        self.set(e, Kind::Op, typed(self.class(x)), f)
                    }
                    /* `x = x + 1` and a temporary (or `x` itself) for the
                     * value: the value is `x`'s class */
                    _ => self.set(e, Kind::Temp, typed(self.class(x)), EMITS),
                }
            }
            Expr::Binary(op, x, y) => {
                self.scan(x)?;
                self.scan(y)?;
                let folds = if self.folds(x) && self.folds(y) { FOLDS } else { 0 };
                if matches!(op, BinOp::AndAnd | BinOp::OrOr) && self.emits(y) {
                    /* `if (x) and_tmp = y; else and_tmp = false;` (:1672-1700) */
                    let c = if self.class(y) == Prec::Medium { Prec::Medium } else { Prec::High };
                    self.set(e, Kind::Temp, c, EMITS);
                } else {
                    let emits = (self.info(x) | self.info(y)) & EMITS;
                    self.set(e, Kind::Op, typed(join(self.class(x), self.class(y))), folds | emits);
                }
            }
            Expr::Assign(op, l, r) => {
                self.scan(r)?;
                self.scan(l)?;
                let c = match op {
                    /* `l = l op r`: the value is the operation's */
                    Some(_) => {
                        if join(self.class(l), self.class(r)) == Prec::Medium {
                            Prec::Medium
                        } else {
                            Prec::High
                        }
                    }
                    None if self.kind(r) == Kind::Const => Prec::None,
                    None if self.class(r) == Prec::Medium => Prec::Medium,
                    None => Prec::High,
                };
                self.set(e, Kind::Temp, typed(c), EMITS);
            }
            Expr::Select(c, t, f) => {
                self.scan(c)?;
                self.scan(t)?;
                self.scan(f)?;
                if self.folds(c) && !self.emits(t) && !self.emits(f) {
                    /* `ast_to_hir` takes the chosen side itself when the
                     * condition is a constant and neither side emits an
                     * instruction (:1973-1978): the `?:` is that side, its
                     * kind and its class. A condition not evaluated here is
                     * a class not settled: Unknown. */
                    match self.constant(c) {
                        Some(Konst::Bool(b)) => {
                            let x = if b { t } else { f };
                            let flags = self.info(x) & (KIND_MASK | FOLDS | FOLDED);
                            let chose = if b { CHOSE_T } else { CHOSE_F };
                            let kind = self.kind(x);
                            self.set(e, kind, self.class(x), (flags & !KIND_MASK) | chose);
                        }
                        _ => self.set(e, Kind::Op, typed(Prec::Unknown), 0),
                    }
                } else {
                    let c = self.temp_rule(&[t, f], |s, x| s.kind(x) == Kind::Const);
                    self.set(e, Kind::Temp, typed(c), EMITS);
                }
            }
            Expr::Index(b, i) => {
                self.scan(b)?;
                self.scan(i)?;
                let folds = if self.folds(b) && self.folds(i) { FOLDS } else { 0 };
                let emits = (self.info(b) | self.info(i)) & EMITS;
                /* A run-time index into a folded `const` mediump vector or
                 * array reads its 16-bit storage in Mesa, while every
                 * folded read is the 32-bit value; the lowering keeps one
                 * slot, at 32 bits (lower.rs `declare_var`), so this read
                 * has no class that is right: Unknown. */
                let c = if !self.folds(i) && self.folded_medium_const(b) { Prec::Unknown } else { self.deref_class(b) };
                self.set(e, Kind::Deref, typed(c), folds | emits);
            }
            Expr::Field(b, name) => {
                self.scan(b)?;
                let f = self.info(b) & (FOLDS | EMITS);
                match self.ty(b) {
                    BaseType::Struct(sname) => {
                        let c = self.member_class(sname, name);
                        self.set(e, Kind::Deref, typed(c), f)
                    }
                    /* a swizzle: an expression of its operand */
                    _ => self.set(e, Kind::Op, typed(self.class(b)), f),
                }
            }
            Expr::Comma(x, y) => {
                self.scan(x)?;
                self.scan(y)?;
                let emits = (self.info(x) | self.info(y)) & EMITS;
                self.set(e, Kind::Op, self.class(y), emits);
            }
            Expr::ArrayCtor { args, .. } => {
                let ids = &self.ast.expr_ids[args.indices()];
                for &x in ids {
                    self.scan(x)?;
                }
                if ids.iter().all(|&x| self.folds(x)) {
                    self.set(e, Kind::Const, typed(Prec::None), FOLDS | FOLDED);
                } else {
                    let c = self.temp_rule(ids, |s, x| s.folds(x));
                    self.set(e, Kind::Temp, typed(c), EMITS);
                }
            }
            Expr::Call { name, args } => self.call(e, name, args, ty)?,
        }
        Ok(())
    }

    /// A temporary's class from the values assigned to it
    /// (`visit_leave(ir_assignment)`): High if one that is not a constant
    /// is not Medium, else Medium if one is, else None. `constant` says
    /// which values `ast_to_hir` left as constants there. An int value
    /// assigned is High (its read is, `can_lower_type`).
    fn temp_rule(&self, vals: &[ExprId], constant: impl Fn(&Self, ExprId) -> bool) -> Prec {
        let mut c = Prec::None;
        for &x in vals {
            if constant(self, x) {
                continue;
            }
            let cx = if is_int(self.ty(x)) { Prec::High } else { self.class(x) };
            c = match (c, cx) {
                (Prec::High, _) | (_, Prec::High | Prec::None) => Prec::High,
                (Prec::Unknown, _) | (_, Prec::Unknown) => Prec::Unknown,
                _ => Prec::Medium,
            };
        }
        c
    }

    fn call(&mut self, e: ExprId, name: &'a str, args: super::ast::Range, ty: BaseType<'a>) -> R<'a, ()> {
        let ids: &'a [ExprId] = &self.ast.expr_ids[args.indices()];
        for &x in ids {
            self.scan(x)?;
        }
        let typed = |c: Prec| if lowerable(ty) || matches!(ty, BaseType::Void) { c } else { Prec::High };
        let all_fold = ids.iter().all(|&x| self.folds(x));
        let is_struct = self.structs.as_slice().iter().any(|s| s.0 == name);
        if let Some(t) = sema::constructor_type(name, self.ast.es3).filter(|_| !is_struct) {
            if all_fold {
                /* `all_parameters_are_constant` (ast_function.cpp:2457) */
                self.set(e, Kind::Const, typed(Prec::None), FOLDS | FOLDED);
                return Ok(());
            }
            let scalar_arg = ids.len() == 1 && self.ty(ids[0]).components() == 1 && !self.types.of(ids[0]).is_array();
            if t.components() == 1 && !matches!(t, BaseType::Mat(..)) || scalar_arg && !matches!(t, BaseType::Mat(..)) {
                /* a scalar's constructor is its argument's component,
                 * converted (`dereference_component`, `convert_component`);
                 * a vector's from one scalar a swizzle of it
                 * (`emit_inline_vector_constructor`, :1528-1530). No
                 * temporary: joined with the argument. */
                let x = ids[0];
                let c = if is_int(self.ty(x)) { Prec::High } else { self.class(x) };
                let emits = self.info(x) & EMITS;
                self.set(e, Kind::Op, typed(c), emits);
                return Ok(());
            }
            /* `vec_ctor` / `mat_ctor`: an assignment per argument that is
             * not a constant */
            let c = self.temp_rule(ids, |s, x| s.folds(x));
            self.set(e, Kind::Temp, typed(c), EMITS);
            return Ok(());
        }
        if is_struct {
            let flags = if all_fold { FOLDS | FOLDED } else { EMITS };
            let kind = if all_fold { Kind::Const } else { Kind::Temp };
            self.set(e, kind, Prec::High, flags);
            return Ok(());
        }
        let mut tys = [sema::Type::UNKNOWN; 16];
        let user = if ids.len() <= tys.len() {
            for (t, &x) in tys.iter_mut().zip(ids) {
                *t = self.types.of(x);
            }
            sema::resolve_overload(self.ast, name, &tys[..ids.len()], false)
        } else {
            None
        };
        if let Some(f) = user {
            /* the declared return precision (`handle_call`, :461-463); a
             * result that is not Medium is High (`visit_leave(ir_call)`) */
            let c = match prec_of(self.decls.get(f.0 as usize).copied().unwrap_or(0) & 3) {
                Prec::Medium => Prec::Medium,
                _ if matches!(ty, BaseType::Void) => Prec::None,
                _ => Prec::High,
            };
            self.set(e, Kind::Temp, c, EMITS);
            return Ok(());
        }
        let sampler = ids.first().is_some_and(|&x| self.ty(x).is_sampler());
        if all_fold && !sampler && !matches!(name, "dFdx" | "dFdy" | "fwidth") {
            /* a built-in of constant arguments is a constant
             * (ast_function.cpp:659) */
            self.set(e, Kind::Const, typed(Prec::None), FOLDS | FOLDED);
            return Ok(());
        }
        let c = match name {
            /* a fixed return precision (builtin_functions.cpp) */
            "unpackHalf2x16" => Prec::Medium,
            "unpackUnorm2x16" | "unpackSnorm2x16" | "floatBitsToInt" | "floatBitsToUint" | "intBitsToFloat"
            | "uintBitsToFloat" | "textureSize" | "packHalf2x16" | "packUnorm2x16" | "packSnorm2x16" => Prec::High,
            /* a look-up: the sampler's precision (:471-487) */
            _ if sampler => {
                if self.medium_sampler(ids[0]) {
                    Prec::Medium
                } else {
                    Prec::High
                }
            }
            /* Medium when every argument that is not a constant is a
             * Medium root (:519-541) */
            _ => {
                let open = |x: &ExprId| !self.folds(*x);
                if ids.iter().filter(|x| open(x)).all(|&x| self.class(x) == Prec::Medium) {
                    Prec::Medium
                } else if ids.iter().filter(|x| open(x)).all(|&x| matches!(self.class(x), Prec::Medium | Prec::Unknown))
                {
                    Prec::Unknown
                } else {
                    Prec::High
                }
            }
        };
        self.set(e, Kind::Temp, typed(c), EMITS);
        Ok(())
    }

    /* ---- what is lowered, top down ------------------------------------ */

    /// Record the instructions' [`Prec`] of `e` and below. `joined`: `e`'s
    /// parent is lowered and the edge to it is combined, so `e` is inside
    /// the parent's 16-bit tree. `folded`: `e` is inside a tree
    /// `ast_to_hir` folded, whose operations are the 32-bit fold's.
    fn mark(&mut self, e: ExprId, joined: bool, folded: bool) {
        let ty = self.ty(e);
        let class = self.class(e);
        let kind = self.kind(e);
        let folded = folded || self.info(e) & FOLDED != 0;
        /* the (value's) lowered state of `e` itself */
        let lowered = !folded
            && match kind {
                /* a constant and a read are lowered only below a root: a
                 * bare read is no root (:835-843) */
                Kind::Const | Kind::Deref => joined,
                Kind::Op => joined || class == Prec::Medium,
                /* a temporary is filled by its own trees: Medium when its
                 * class is */
                Kind::Temp => class == Prec::Medium,
            };
        /* An int, a sampler and no value have no float width; a struct's
         * value is its members, copied at full width (a struct is never
         * lowered, `can_lower_type`). */
        let p = if matches!(ty, BaseType::Struct(_)) {
            Prec::High
        } else if !lowerable(ty) || ty.is_sampler() {
            Prec::None
        } else if class == Prec::Unknown && !folded {
            Prec::Unknown
        } else if lowered {
            Prec::Medium
        } else {
            Prec::High
        };
        self.set_instr(e, p);
        /* an argument of a call or a constructor that folds is a constant
         * there (`process_parameters`, :61) */
        let arg = |s: &mut Self, x: ExprId| {
            let f = folded || s.folds(x);
            s.mark(x, false, f);
        };
        match *self.ast.expr(e) {
            Expr::IntLit(_) | Expr::UintLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) | Expr::Ident(_) => {}
            Expr::Length(x) => self.mark(x, false, folded),
            Expr::Unary(UnOp::Plus | UnOp::Minus | UnOp::Not | UnOp::BitNot, x) => self.mark(x, lowered, folded),
            /* `x++`: `x = x + 1` is a tree of its own, Medium when `x` is;
             * the value read back is `x`'s (or its copy's) */
            Expr::Unary(_, x) => {
                self.mark(x, false, folded);
                self.set_instr(x, p);
            }
            Expr::Binary(_, x, y) => {
                let j = kind == Kind::Op && lowered;
                self.mark(x, j, folded);
                self.mark(y, j, folded);
            }
            Expr::Assign(op, l, r) => {
                /* `l op= r` is `l = l op r`: one tree, lowered when the
                 * operation's class is Medium */
                let j = op.is_some() && lowered;
                self.mark(l, j, folded);
                self.mark(r, j, folded);
            }
            Expr::Select(c, t, f) => {
                self.mark(c, false, folded);
                /* the chosen side of a constant condition is this node:
                 * it gets this node's place; the other side is dropped by
                 * Mesa and is a tree of its own here */
                let info = self.info(e);
                let (jt, jf) = match (info & CHOSE_T != 0, info & CHOSE_F != 0) {
                    (true, _) => (joined, false),
                    (_, true) => (false, joined),
                    _ => {
                        let j = kind == Kind::Op && lowered;
                        (j, j)
                    }
                };
                self.mark(t, jt, folded);
                self.mark(f, jf, folded);
            }
            /* a dereference's children are separate trees; the array (or
             * struct) it reads is the same read */
            Expr::Index(b, i) => {
                self.mark(b, lowered, folded);
                self.mark(i, false, folded);
            }
            Expr::Field(b, _) => self.mark(b, lowered, folded),
            Expr::Comma(x, y) => {
                self.mark(x, false, folded);
                self.mark(y, lowered, folded);
            }
            Expr::ArrayCtor { args, .. } => {
                for &x in &self.ast.expr_ids[args.indices()] {
                    arg(self, x);
                }
            }
            Expr::Call { args, .. } => {
                let ids: &'a [ExprId] = &self.ast.expr_ids[args.indices()];
                if kind == Kind::Op {
                    /* a conversion or a swizzle of the one argument: joined */
                    for &x in ids {
                        let f = folded || self.folds(x);
                        self.mark(x, lowered, f);
                    }
                } else {
                    for &x in ids {
                        arg(self, x);
                    }
                }
            }
        }
    }
}

/// An f16 holds `x` exactly (a zero, a normal or a subnormal f16 value):
/// then narrowing `x` and computing at 16 bits agree on it.
pub fn f16_exact(x: f32) -> bool {
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0 {
        /* a zero; an f32 subnormal is far below the smallest f16 */
        return man == 0;
    }
    if exp == 0xff {
        return false;
    }
    let e = exp - 127;
    match e {
        /* normal: ten fraction bits */
        -14..=15 => man & 0x1fff == 0,
        /* subnormal: a multiple of 2^-24 */
        -24..=-15 => (man | 0x80_0000) & ((1u32 << (13 + (-14 - e) as u32)) - 1) == 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! The class of each form of docs/notes/mali-precision.md A.3, as the
    //! lowering records it per instruction: the A.9 probes (whose Mesa NIR
    //! the note quotes) and the `vsmp_*` forms of dEQP's vertex shaders.
    use super::super::arena::Arena;
    use super::super::ir::{AluOp, Module, Op, Prec, ValueId, ValueKind};
    use super::super::lower::compile_to_ir;
    use super::super::sema::Stage;
    use std::string::String;
    use std::vec::Vec;

    /// The A.9 probes' vertex shader head.
    const VS: &str = "attribute highp vec4 p;\nattribute highp float a;\nattribute highp float b;\n";

    struct Got {
        ops: Vec<Op>,
        prec: Vec<Prec>,
        /// Each local's name and storage.
        locals: Vec<(String, Prec)>,
    }

    fn compile(src: &str, stage: Stage) -> Got {
        let mut buf = std::vec![0u8; 1 << 21];
        let a = Arena::new(&mut buf);
        let text = a.alloc_str(src).expect("arena");
        let l = match compile_to_ir(&a, stage, text) {
            Ok(l) => l,
            Err(e) => panic!("{src}\nline {}: {:?}: {}", e.line, e.tok, e.msg),
        };
        let m: &Module = &l.module;
        Got {
            ops: m.instrs.to_vec(),
            prec: (0..m.instrs.len()).map(|i| m.prec_of(ValueId(i as u32))).collect(),
            locals: m
                .locals
                .iter()
                .enumerate()
                .map(|(i, d)| (String::from(d.name), m.local_prec_of(super::super::ir::LocalId(i as u32))))
                .collect(),
        }
    }

    fn vs(body: &str) -> Got {
        compile(&std::format!("{VS}{body}"), Stage::Vertex)
    }

    /// The [`Prec`] of every instruction `f` picks, in program order.
    fn of(g: &Got, f: impl Fn(&Op) -> bool) -> Vec<Prec> {
        g.ops.iter().zip(&g.prec).filter(|(o, _)| f(o)).map(|(_, p)| *p).collect()
    }

    fn alu(g: &Got, op: AluOp) -> Vec<Prec> {
        of(g, |o| matches!(o, Op::Alu(x, _) if *x == op))
    }

    fn local(g: &Got, name: &str) -> Prec {
        g.locals.iter().find(|(n, _)| n == name).map(|l| l.1).unwrap_or_else(|| panic!("no local {name}"))
    }

    const M: Prec = Prec::Medium;
    const H: Prec = Prec::High;

    /// p1: `c * c` of a `const mediump float` is a 16-bit multiply (Mesa
    /// folds it to 0x3ec2, the fp16 product); the front end does not fold
    /// it (open question E.5): the multiply of the two reads is there.
    #[test]
    fn a_mediump_const_times_itself_is_lowered_and_not_folded() {
        let g = vs("varying highp float v;\nconst mediump float c = 1.3;\nvoid main(){ v = c * c; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert!(g.ops.iter().all(|o| !matches!(o, Op::ConstF32(k) if f32::from_bits(k[0]) > 1.6)), "{:?}", g.ops);
        assert_eq!(of(&g, |o| matches!(o, Op::LoadLocal { .. })), [M, M], "the reads below the root");
        assert_eq!(local(&g, "c"), H, "a const whose initialiser folds holds its 32-bit value");
        /* p1b: a local, the same */
        let g = vs("varying highp float v;\nvoid main(){ mediump float c = 1.3; v = c * c; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(local(&g, "c"), M);
    }

    /// p1c, p1d: with a highp operand the multiply is 32-bit.
    #[test]
    fn a_highp_operand_makes_the_operation_high() {
        let g = vs("varying highp float v;\nconst mediump float c = 1.3;\nvoid main(){ v = c * a; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        let g = vs("varying highp float v;\nvoid main(){ mediump float c = 1.3; v = c * a; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
    }

    /// p2: a call's result is the function's declared return precision,
    /// whatever its body computes: the highp `x * x` is 32-bit, the
    /// caller's `* 1.5` of the mediump result 16-bit.
    #[test]
    fn a_call_s_result_is_its_declared_return_precision() {
        let g = vs("varying highp float v;\n\
                    mediump float g(highp float x) { return x * x; }\n\
                    void main(){ v = g(a) * 1.5; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [H, M]);
    }

    /// p3, p3b: a constructor is a temporary, High once a highp value is
    /// assigned to it, Medium when only a mediump one and constants are.
    #[test]
    fn a_constructor_is_classed_by_what_it_is_assigned() {
        let g = vs("varying highp vec2 v;\nvoid main(){ mediump float m = a; v = vec2(m, b) * 1.5; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        assert_eq!(local(&g, "m"), M);
        let g =
            vs("varying highp vec2 v;\nvoid main(){ mediump float m = a; v = vec2(m, 2.0) * 1.5; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        /* the constructor's slot is the temporary Mesa stores at 16 bits */
        assert!(g.locals.iter().any(|(n, p)| n.is_empty() && *p == M), "{:?}", g.locals);
    }

    /// p4: a look-up is its sampler's precision, lowp by default.
    #[test]
    fn a_look_up_is_its_sampler_s_precision() {
        let g = vs("varying highp float v;\nuniform sampler2D s;\n\
                    void main(){ v = texture2D(s, p.xy).x * 1.5; gl_Position = p; }");
        assert_eq!(of(&g, |o| matches!(o, Op::Tex { .. })), [M]);
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        let g = vs("varying highp float v;\nuniform highp sampler2D s;\n\
                    void main(){ v = texture2D(s, p.xy).x * 1.5; gl_Position = p; }");
        assert_eq!(of(&g, |o| matches!(o, Op::Tex { .. })), [H]);
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
    }

    /// p6a, p6b: `?:` is a temporary; its condition a tree of its own.
    #[test]
    fn a_select_is_classed_by_its_sides() {
        let g = vs("varying highp float v;\n\
                    void main(){ mediump float m = a; v = ((b > 0.5) ? m : b) * 1.5; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Flt), [H]);
        assert_eq!(alu(&g, AluOp::Bcsel), [H]);
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        let g = vs("varying highp float v;\n\
                    void main(){ mediump float m = a; v = ((b > 0.5) ? m : 2.0) * 1.5; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Flt), [H]);
        assert_eq!(alu(&g, AluOp::Bcsel), [M]);
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        /* two constants leave the temporary None: not stored at 16 bits,
         * so the select is 32-bit, and its read joins the Medium product */
        let g = vs("varying highp float v;\n\
                    void main(){ mediump float m = a; v = ((b > 0.5) ? 1.0 : 2.0) * m; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Bcsel), [H]);
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
    }

    /// vsmp_if_else: an `if`'s condition is a root; under `precision
    /// mediump float` the attribute is mediump and the compare 16-bit.
    #[test]
    fn a_condition_is_a_root() {
        let src = |p: &str| {
            std::format!(
                "precision {p} float;\nattribute highp vec4 dEQP_Position;\nattribute float in0;\nvarying float out0;\n\
                 void main(){{ if (in0 >= 1.0) out0 = 1.0; else out0 = 0.0; gl_Position = dEQP_Position; }}"
            )
        };
        let g = compile(&src("mediump"), Stage::Vertex);
        assert_eq!(of(&g, |o| matches!(o, Op::Alu(AluOp::Fle | AluOp::Flt, _))), [M]);
        let g = compile(&src("highp"), Stage::Vertex);
        assert_eq!(of(&g, |o| matches!(o, Op::Alu(AluOp::Fle | AluOp::Flt, _))), [H]);
    }

    /// vsmp_vec3, vsmp_in_float: a parameter is read at its precision
    /// (the default in force, mediump); a parameter the body assigns has a
    /// slot, stored at 32 bits; a local at its own precision.
    #[test]
    fn a_parameter_is_read_at_its_precision_and_stored_at_full_width() {
        let head = "precision mediump float;\nattribute highp vec4 dEQP_Position;\n";
        let g = compile(
            &std::format!(
                "{head}attribute vec3 in0;\nvarying float out0;\n\
                 float func(vec3 a) {{ return -(a.x + a.y + a.z); }}\n\
                 void main(){{ out0 = func(in0); gl_Position = dEQP_Position; }}"
            ),
            Stage::Vertex,
        );
        assert_eq!(alu(&g, AluOp::Fadd), [M, M]);
        assert_eq!(alu(&g, AluOp::Fneg), [M]);
        let g = compile(
            &std::format!(
                "{head}attribute float in0;\nvarying float out0;\n\
                 float func(in float a) {{ a = -a; return 2.0 * a; }}\n\
                 void main(){{ float f = in0; float g = func(f); out0 = f + g; gl_Position = dEQP_Position; }}"
            ),
            Stage::Vertex,
        );
        assert_eq!(alu(&g, AluOp::Fneg), [M]);
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(alu(&g, AluOp::Fadd), [M]);
        assert_eq!(local(&g, "a"), H, "a parameter's slot");
        assert_eq!(local(&g, "f"), M);
    }

    /// A built-in of mediump arguments is lowered with them (its body
    /// mediump); one highp argument makes it 32-bit; a constant argument
    /// does not count.
    #[test]
    fn a_builtin_is_medium_when_its_arguments_are() {
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = abs(m) * 2.0; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fabs), [M]);
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = max(m, 1.0); gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmax), [M]);
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = max(m, b); gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmax), [H]);
    }

    /// `pow` is one 32-bit `fpow` in Mesa whatever its arguments
    /// (precrule_e3_pow, branch mali-prec-width): its expansion records
    /// High; the Medium product that reads it stays Medium. A user's own
    /// `exp2(y * log2(x))` of mediump values is Mesa's 16-bit product.
    #[test]
    fn pow_s_expansion_is_full_width() {
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = pow(m, 2.5) * m; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Flog2), [H]);
        assert_eq!(alu(&g, AluOp::Fexp2), [H]);
        assert_eq!(alu(&g, AluOp::Fmul), [H, M], "the expansion's product, then the reader's");
        let g =
            vs("varying highp float v;\nvoid main(){ mediump float m = a; v = exp2(2.5 * log2(m)); gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
    }

    /// A call's argument that is a constant expression is folded by
    /// `ast_to_hir`, at 32 bits: `c * c` there is High although the same
    /// product alone (p1) is Medium; it counts as a constant argument.
    #[test]
    fn a_constant_argument_is_folded_at_full_width() {
        let g = vs("varying highp float v;\nconst mediump float c = 1.3;\n\
                    void main(){ mediump float m = a; v = max(c * c, m); gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        assert_eq!(alu(&g, AluOp::Fmax), [M]);
    }

    /// A uniform, an attribute, a struct member: each read is its declared
    /// precision; an int is High and its instructions record None.
    #[test]
    fn a_read_is_its_declaration_s_precision() {
        let g = vs("varying highp float v;\nuniform mediump float u;\nvoid main(){ v = u * 2.0; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(of(&g, |o| matches!(o, Op::LoadUniform { .. })), [M]);
        let g = vs("varying highp float v;\nuniform mediump float u;\nvoid main(){ v = u * a; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        let g = vs("varying highp float v;\nstruct S { mediump float x; highp float y; };\nuniform S s;\n\
                    void main(){ v = s.x * 2.0 + s.y; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(alu(&g, AluOp::Fadd), [H]);
        let g = vs("varying highp float v;\nvoid main(){ int i = int(a); v = float(i + 1) * 2.0; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Iadd), [Prec::None]);
        assert_eq!(alu(&g, AluOp::I2f), [H]);
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        assert_eq!(local(&g, "i"), Prec::None);
    }

    /// A root that is a bare read is not lowered (`handle_rvalue`): `v = m`
    /// reads `m` at full width.
    #[test]
    fn a_bare_read_is_no_root() {
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = m; gl_Position = p; }");
        assert_eq!(of(&g, |o| matches!(o, Op::LoadLocal { .. })), [H]);
    }

    /// An array's index is a tree of its own; the element is the array's
    /// precision.
    #[test]
    fn an_index_is_a_tree_of_its_own() {
        let g = vs("varying highp float v;\n\
                    void main(){ mediump float t[2]; t[0] = a; t[1] = b; int i = int(b); \
                    v = t[i] * 2.0; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert!(alu(&g, AluOp::Ieq).iter().all(|&p| p == H), "the index compare is an int compare");
    }

    /// `++`, `*=` and `+=` are `x = x op y`: one tree, Medium when its
    /// operands are.
    #[test]
    fn a_step_and_a_compound_assignment_are_trees_of_their_own() {
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; m++; v = m; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fadd), [M]);
        let g = vs("varying highp float v;\nvoid main(){ highp float m = a; m++; v = m; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fadd), [H]);
        let g =
            vs("varying highp float v;\nvoid main(){ mediump float m = a; m *= 2.0; m += b; v = m; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(alu(&g, AluOp::Fadd), [H]);
    }

    /// A store of an output records the output's declared precision: the
    /// vertex position highp, a mediump varying Medium, ES 1.00's colour
    /// mediump.
    #[test]
    fn an_output_store_records_the_output_s_precision() {
        let g = vs("varying mediump float w;\nvoid main(){ w = a; gl_Position = p; }");
        assert_eq!(of(&g, |o| matches!(o, Op::StoreOutput { .. })), [M, H]);
        let g = compile(
            "precision mediump float;\nvarying vec2 t;\nvoid main(){ gl_FragColor = vec4(t * 2.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        assert_eq!(of(&g, |o| matches!(o, Op::StoreOutput { .. })), [M]);
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
    }

    /// `gl_FragCoord` is mediump in ES 1.00 (case 84) and highp in 3.00.
    #[test]
    fn frag_coord_is_mediump_in_es_1_00_only() {
        let g = compile(
            "precision mediump float;\nvoid main(){ gl_FragColor = vec4(gl_FragCoord.x * 0.5, 0.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        /* a report-es2 build refuses an ES 3.00 source */
        if crate::REPORT_ES2 {
            return;
        }
        let g = compile(
            "#version 300 es\nprecision mediump float;\nout vec4 o;\n\
             void main(){ o = vec4(gl_FragCoord.x * 0.5, 0.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
    }

    /// Open question E.5: the front end folds `x / c` into a multiply by
    /// the reciprocal it computes at 32 bits (lower.rs `reciprocal`); Mesa
    /// computes `frcp` of the 16-bit `c` at 16 bits (review probes div1,
    /// q4div: `/ (constant float16_t 1.299805)`). Where `c` has no exact
    /// f16 value the two differ (`/ 1.3`: 0.76904 against 0.76953). The
    /// fold stays (keeping `Frcp` moved the PowerVR's words, second review
    /// B-B), so in a Medium tree its constant records Unknown; where `c`
    /// has an f16 value the two agree and it is Medium.
    #[test]
    fn a_reciprocal_folded_in_a_lowered_tree_is_unknown_where_it_differs() {
        let consts = |g: &Got| of(g, |o| matches!(o, Op::ConstF32(_)));
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = m / 1.3; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(alu(&g, AluOp::Frcp), [], "folded, as on main");
        assert_eq!(consts(&g), [M, Prec::Unknown], "1.3 itself, then its 32-bit reciprocal");
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = m / 4.0; gl_Position = p; }");
        assert_eq!(consts(&g), [M, M]);
        let g = vs("varying highp float v;\nvoid main(){ v = b / 1.3; gl_Position = p; }");
        assert_eq!(consts(&g), [H, H]);
    }

    /// A float constant the lowering shares between a 16-bit tree and a
    /// 32-bit one (`const_value` reuses one within a block) records High:
    /// a literal read at each user's width. Medium on a constant means it
    /// is in 16-bit trees only, so a reader may take its fp16 value.
    #[test]
    fn a_constant_shared_by_both_widths_is_high() {
        let g = vs("varying mediump float w;\nvarying highp float v;\n\
                    void main(){ mediump float m = a; w = m * 1.3; v = b * 1.3; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M, H]);
        assert_eq!(of(&g, |o| matches!(o, Op::ConstF32(_))), [H], "one constant, shared");
        let g = vs("varying mediump float w;\nvoid main(){ mediump float m = a; w = m * 1.3; gl_Position = p; }");
        assert_eq!(of(&g, |o| matches!(o, Op::ConstF32(_))), [M]);
    }

    /// Every slot records its own storage: the shadow slot of an output
    /// read back and a function's result slot are pushed through
    /// `push_local` too, so no local takes another's entry. Mesa (review
    /// probes r_shadow, r_ret, drm-shim GLSL IR): `highp float h`,
    /// `float16_t m`, `temporary mediump float16_t f_retval`.
    #[test]
    fn every_slot_records_its_own_storage() {
        let g = compile(
            "precision mediump float; varying mediump vec4 v; uniform highp float k;\n\
             void main(){ gl_FragColor = v; highp float h = k * k; mediump float m = v.x * v.y; \
             gl_FragColor = gl_FragColor * 0.5 + vec4(h, m, 0.0, 0.0); }",
            Stage::Fragment,
        );
        assert_eq!(local(&g, "h"), H);
        assert_eq!(local(&g, "m"), M);
        assert_eq!(local(&g, "gl_FragColor"), H, "an output's shadow");
        let g = vs("attribute mediump vec4 q; varying highp vec4 v;\n\
                    mediump float f(mediump float x){ if (x > 0.5) return x * 2.0; return x * 3.0; }\n\
                    void main(){ gl_Position = p; highp float h = p.x * p.y; mediump float m = q.y * q.z; \
                    v = vec4(f(q.x), m, h, 1.0); }");
        assert_eq!(local(&g, "h"), H);
        assert_eq!(local(&g, "m"), M);
        assert!(g.locals.iter().any(|(n, p)| n.is_empty() && *p == M), "the result slot: {:?}", g.locals);
    }

    /// The corpus shader the review found 92 table entries for 93 locals
    /// in: the tables are as long as what they describe.
    #[test]
    fn the_tables_are_in_step_on_a_dead_cube_shader() {
        let src = include_str!("../../../tests/fixtures/deqp_random_dead_cube/all_features.fragment.13.frag");
        let mut buf = std::vec![0u8; 1 << 22];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, a.alloc_str(src).expect("arena")).expect("compiles");
        assert_eq!(l.module.prec.len(), l.module.instrs.len());
        assert_eq!(l.module.local_prec.len(), l.module.locals.len());
    }

    /// Every `.vert` and `.frag` under `dir`, recursively.
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "vert" || x == "frag") {
                out.push(p);
            }
        }
    }

    /// The kind of every value of `m`, by [`Module::value_kind`]: the
    /// function the lowering records each [`Prec`] by, so the check has
    /// no copy of its own.
    fn kinds(m: &Module) -> Vec<ValueKind> {
        let mut k = Vec::with_capacity(m.instrs.len());
        for i in 0..m.instrs.len() {
            let x = m.value_kind(ValueId(i as u32), &k);
            k.push(x);
        }
        k
    }

    /// The values whose class contradicts their kind: a float or a bool
    /// that records None, or an int that records a class.
    fn unclassed(m: &Module) -> Vec<usize> {
        let k = kinds(m);
        (0..m.instrs.len())
            .filter(|&i| {
                let p = m.prec_of(ValueId(i as u32));
                match k[i] {
                    ValueKind::Float | ValueKind::Bool => p == Prec::None,
                    ValueKind::Int => p != Prec::None,
                    ValueKind::Void => false,
                }
            })
            .collect()
    }

    /// Over the tree's shaders, invariants that hold for every correct
    /// shader, whatever another change adds to the corpus:
    ///
    /// - the tables are in step with the instructions and locals;
    /// - every value's class agrees with its kind ([`Module::value_kind`],
    ///   the function the lowering decides by): a float or a bool never
    ///   records None, an int always does;
    /// - no `Iand`, `Ior` or `Ixor` mixes a bool with a non-bool, which
    ///   [`AluOp::result_kind`]'s arm for them rests on;
    /// - no float or bool value was lowered under a context with no class
    ///   ([`super::super::lower::NO_CONTEXT`] stays 0);
    /// - the lowering lowers no expression the scan did not reach
    ///   ([`super::super::lower::UNSEEN`] stays 0).
    ///
    /// What this test notices is a value WITHOUT a class. It does not
    /// notice a rule that refuses more than it should: a change that made
    /// every pow argument Unknown, or every `?:` on a constant condition,
    /// keeps every
    /// value classed and passes here (ninth review, mutations A5 and A6).
    /// Each rule is guarded by its own unit tests on its own sources:
    ///
    /// - pow / asin / acos arguments not known to vary (A5 is killed by
    ///   these six): `an_argument_not_known_to_vary_into_pow_is_unknown`,
    ///   `only_a_bare_read_is_known_to_vary`,
    ///   `a_computed_pow_argument_is_refused_and_a_bare_read_is_not`,
    ///   `a_local_named_like_an_input_is_the_local`,
    ///   `a_constant_through_a_parameter_into_pow_is_unknown`,
    ///   `an_exact_fp16_constant_argument_is_not_pinned`;
    /// - a `?:` whose condition is not evaluated (A6 is killed by these
    ///   three): `a_select_on_a_constant_condition_is_the_chosen_side`,
    ///   `mesa_s_rules_decide_more_constants`,
    ///   `a_condition_outside_the_measured_forms_is_unknown`;
    /// - a run-time index into a folded `const`:
    ///   `a_run_time_index_into_a_folded_const_is_unknown`;
    /// - an inexact reciprocal fold:
    ///   `a_reciprocal_folded_in_a_lowered_tree_is_unknown_where_it_differs`.
    ///
    /// Which corpus shaders a rule refuses is not listed here: a new
    /// shader that calls pow on a computed value is refused by design
    /// (glmark2's `pow(max(dot(N, H), 0.0), 100.0)`). One positive pin
    /// keeps the Unknown path live over the corpus.
    #[test]
    fn every_value_of_the_corpus_has_its_kind_s_class() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for d in ["tools/shaders", "tools/mali/testdata", "tests/fixtures", "tools/cts/testdata"] {
            sources(&root.join(d), &mut files);
        }
        let mut buf = std::vec![0u8; 1 << 23];
        let (mut compiled, mut bad, mut unknown) = (0usize, Vec::new(), Vec::<String>::new());
        super::super::lower::UNSEEN.with(|n| n.set(0));
        super::super::lower::NO_CONTEXT.with(|n| n.set(0));
        for f in &files {
            let Ok(src) = std::fs::read_to_string(f) else { continue };
            let stage = if f.extension().is_some_and(|x| x == "vert") { Stage::Vertex } else { Stage::Fragment };
            let a = Arena::new(&mut buf);
            let Ok(text) = a.alloc_str(&src) else { continue };
            let Ok(l) = compile_to_ir(&a, stage, text) else { continue };
            let m = &l.module;
            compiled += 1;
            if !m.has_prec() {
                bad.push(std::format!("{}: tables out of step", f.display()));
                continue;
            }
            if m.prec.contains(&Prec::Unknown) {
                let rel = f.strip_prefix(root).unwrap_or(f).display().to_string();
                unknown.push(rel);
            }
            for i in unclassed(m) {
                bad.push(std::format!("{}: %{i} {:?} {:?}", f.display(), m.prec[i], m.instrs[i]));
            }
            /* what `AluOp::result_kind`'s `Iand | Ior | Ixor` arm rests on:
             * no side is a bool unless both are */
            let k = kinds(m);
            for (i, op) in m.instrs.iter().enumerate() {
                if let Op::Alu(AluOp::Iand | AluOp::Ior | AluOp::Ixor, [x, y, _]) = op {
                    if (k[x.val.0 as usize] == ValueKind::Bool) != (k[y.val.0 as usize] == ValueKind::Bool) {
                        bad.push(std::format!("{}: %{i} mixes a bool with a non-bool: {op:?}", f.display()));
                    }
                }
            }
        }
        std::println!("{compiled} of {} shaders compiled", files.len());
        assert!(compiled > 900, "{compiled} shaders compiled");
        let unseen = super::super::lower::UNSEEN.with(|n| n.get());
        assert_eq!(unseen, 0, "expressions lowered that the precision scan never reached");
        let no_context = super::super::lower::NO_CONTEXT.with(|n| n.get());
        assert_eq!(no_context, 0, "float or bool values lowered under no class, left to the Unknown rule");
        /* the Unknown path is live: the dead-cube shader's `asin(float(k))`
         * of a global `k` NIR may prove constant (fourth review) */
        assert!(
            unknown.iter().any(|u| u == "tests/fixtures/deqp_random_dead_cube/all_features.fragment.96.frag"),
            "{unknown:?}"
        );
        std::println!("{} shaders carry an Unknown: {unknown:?}", unknown.len());
        assert!(
            bad.is_empty(),
            "{} values whose class is not their kind's:\n{}",
            bad.len(),
            bad[..bad.len().min(20)].join("\n")
        );
    }

    /// A `?:` whose condition `ast_to_hir` evaluates is the chosen side
    /// itself (ast_to_hir.cpp:1973-1978): joined with its parent. Mesa
    /// (review probe sc3, drm-shim GLSL IR): `(true ? a.y : h.z) * a.x` is
    /// `f162f(float16_t * (swiz y f2fmp(a)) (swiz x f2fmp(a)))`, a 16-bit
    /// multiply. A condition this module cannot evaluate is Unknown.
    #[test]
    fn a_select_on_a_constant_condition_is_the_chosen_side() {
        let head = "attribute mediump vec4 q; attribute highp vec4 h; varying highp vec4 v;\n";
        let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4((true ? q.y : h.z) * q.x); }}"));
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(alu(&g, AluOp::Bcsel), [M], "the select stands for the chosen side");
        let g = vs(&std::format!(
            "{head}const bool B = false;\nvoid main(){{ gl_Position = p; v = vec4(B ? q.z : h.x * 2.0); }}"
        ));
        assert_eq!(alu(&g, AluOp::Fmul), [H], "the highp side is chosen");
        let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4((2.0 > 1.0 ? h.y : q.z) * q.x); }}"));
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
        let g =
            vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4((sin(0.5) > 0.3 ? q.y : h.z) * q.x); }}"));
        assert_eq!(alu(&g, AluOp::Fmul), [Prec::Unknown], "a condition not evaluated here");
    }

    /// asin and acos are one NIR op each, which nir_builtin_builder.c
    /// builds at 32 bits for a 16-bit operand (:232-233, :243-244:
    /// `nir_f2f16(nir_asin(nir_f2f32(x)))`): the expansion records High,
    /// as pow's does. atan stays at its operand's width (Medium). Mesa
    /// (review probe v_asin; drm-shim `asin(e)`, e mediump: `FMA.f32`,
    /// `FRSQ.f32`).
    #[test]
    fn asin_and_acos_are_full_width_and_atan_is_not() {
        let head = "attribute mediump vec4 q; varying highp vec4 v;\n";
        for f in ["asin", "acos"] {
            let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4({f}(q.x) * q.y); }}"));
            let fmul = alu(&g, AluOp::Fmul);
            assert_eq!(fmul.last(), Some(&M), "{f}: the reader's product");
            for op in [AluOp::Ffma, AluOp::Fsqrt, AluOp::Fabs] {
                assert!(alu(&g, op).iter().all(|&p| p == H), "{f}: {op:?} of the expansion");
            }
        }
        let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4(atan(q.x)); }}"));
        assert!(alu(&g, AluOp::Fmul).iter().all(|&p| p == M), "atan is Medium");
    }

    /// A constant argument of a Medium built-in is passed to its mediump
    /// parameter: its fp16 value. pow's expansion is High, so the constant
    /// records Medium (every reader takes its fp16 value) and the High
    /// expansion reads it widened. Mesa (review probe v_pow, drm-shim GLSL
    /// IR): `pow (f2fmp a) (f2fmp (constant float (0.100000)))`; the
    /// width branch's precrule_pow_const reads 0x3dccc000.
    #[test]
    fn a_constant_argument_of_a_full_width_builtin_is_its_fp16_value() {
        let g = vs("attribute mediump vec4 q; varying highp vec4 v;\n\
                    void main(){ gl_Position = p; v = vec4(pow(q.x, 0.1)); }");
        let c = of(&g, |o| matches!(o, Op::ConstF32(k) if f32::from_bits(k[0]) == 0.1));
        assert_eq!(c, [M]);
        assert!(alu(&g, AluOp::Flog2).iter().all(|&p| p == H));
        /* a folded argument that is no single constant: the fold is High
         * and its fp16 rounding into the parameter has no form here */
        let g = vs("attribute mediump vec4 q; varying highp vec4 v;\n\
                    void main(){ gl_Position = p; v = vec4(pow(q.x, 0.05 * 3.0)); }");
        assert_eq!(alu(&g, AluOp::Fmul).first(), Some(&Prec::Unknown), "{:?}", g.ops);
    }

    /// The width review's S-a: the result slot of a user function with
    /// an early `return` had no `local_prec` entry, and every femtovg /
    /// offarm shader was refused for its tables. The exact reproducer, and
    /// the largest such corpus shader.
    #[test]
    fn a_function_with_an_early_return_gives_its_result_slot_a_class() {
        let g = compile(
            "precision mediump float; varying vec2 t;\n\
             float f(vec2 x){ if (x.y > 0.0) { return x.x; } return x.y; }\n\
             void main(){ gl_FragColor = vec4(f(t)); }",
            Stage::Fragment,
        );
        assert_eq!(g.locals.len(), g.locals.iter().filter(|l| l.1 != Prec::Unknown).count());
        assert!(g.locals.iter().any(|(n, p)| n.is_empty() && *p == M), "{:?}", g.locals);
        let src = include_str!("../../../tools/mali/testdata/fvg_fill_color_glyph_nodiscard.frag");
        let mut buf = std::vec![0u8; 1 << 22];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, a.alloc_str(src).expect("arena")).expect("compiles");
        assert!(l.module.has_prec());
    }

    /// A `const` read where `ast_to_hir` folds (a call's or a
    /// constructor's argument) is its 32-bit value, folded at 32 bits.
    /// Mesa (review probe v_constarg, drm-shim GLSL IR): `vec4(c * c)` of
    /// `const mediump float c = 1.3` is `constant f16vec4 (1.690430)`, the
    /// fp16 rounding of 1.3 * 1.3; fp16(1.3)^2 is 1.6895, one ulp off. So
    /// the slot holds the 32-bit value (High): a 16-bit reader narrows it,
    /// which is the fp16 value Mesa's 16-bit storage gives, and the folded
    /// product reads 1.3.
    #[test]
    fn a_const_whose_initialiser_folds_is_stored_at_full_width() {
        let g = vs("attribute mediump vec4 q; varying highp vec4 v;\nconst mediump float c = 1.3;\n\
                    void main(){ gl_Position = p; v = q * (c * c) + vec4(c * c); }");
        assert_eq!(local(&g, "c"), H);
        assert_eq!(alu(&g, AluOp::Fmul), [M, M, H], "q * (c * c) is 16-bit; vec4(c * c) is the 32-bit fold");
        let g = vs("varying highp vec4 v;\nvoid main(){ gl_Position = p; mediump float m = p.x; v = vec4(m * m); }");
        assert_eq!(local(&g, "m"), M, "a mediump local that is no const is stored at 16 bits");
    }

    /// A projective look-up divides its coordinate by the last lane. Mesa
    /// divides in the texture instruction's lowering, on the coordinate,
    /// which is a 32-bit root (A.3: a look-up's arguments are trees of
    /// their own, each root widened): drm-shim for the width review's
    /// t_proj (`texture2DProjLod(s, vec3(p.xy, c * d), 0.0)`, lowp `s`) is
    /// `FRCP.f32` and `FMA.f32` before `TEX_SINGLE`. So the divide records
    /// High; the look-up keeps its sampler's class. (`c * d` is a Medium
    /// root, 16-bit in Mesa's GLSL IR; its `FMA.f32` is NIR's late rule.)
    #[test]
    fn a_projective_divide_is_full_width() {
        let g = vs("attribute mediump float c; attribute mediump float d; uniform sampler2D s;\n\
                    varying highp vec4 v;\n\
                    void main(){ gl_Position = p; v = texture2DProjLod(s, vec3(p.xy, c * d), 0.0); }");
        assert_eq!(alu(&g, AluOp::Frcp), [H]);
        assert_eq!(alu(&g, AluOp::Fmul), [M, H], "c * d, then the divide");
        assert_eq!(of(&g, |o| matches!(o, Op::Tex { .. })), [M]);
    }

    /// Each input and output records its declared precision (A.2), which
    /// the width pass needs for a varying's width and which a bare read
    /// does not show (it records High).
    #[test]
    fn every_input_and_output_records_its_declared_precision() {
        let p_of = |src: &str, stage: Stage| {
            let mut buf = std::vec![0u8; 1 << 21];
            let a = Arena::new(&mut buf);
            let l = compile_to_ir(&a, stage, a.alloc_str(src).expect("arena")).expect("compiles");
            let io = |ds: &[super::super::ir::IoDecl]| -> Vec<(String, Prec)> {
                ds.iter().map(|d| (String::from(d.name), d.prec)).collect()
            };
            (io(l.module.inputs), io(l.module.outputs))
        };
        let (i, o) = p_of(
            "attribute highp vec4 p; attribute mediump vec4 q; varying mediump vec2 w; varying highp float h;\n\
             void main(){ gl_Position = p; w = q.xy; h = q.z; }",
            Stage::Vertex,
        );
        assert_eq!(i, [(String::from("p"), H), (String::from("q"), M)]);
        assert!(o.contains(&(String::from("w"), M)) && o.contains(&(String::from("h"), H)), "{o:?}");
        assert!(o.contains(&(String::from("gl_Position"), H)), "{o:?}");
        let (i, o) = p_of(
            "precision mediump float; varying vec2 w; varying highp float h;\n\
             void main(){ gl_FragColor = vec4(w, h, 1.0); }",
            Stage::Fragment,
        );
        assert!(i.contains(&(String::from("w"), M)) && i.contains(&(String::from("h"), H)), "{i:?}");
        assert_eq!(o, [(String::from("gl_FragColor"), M)]);
    }

    /// The class `classify` gives the variable `name` of `src`. The
    /// lowering refuses a `precision` statement inside a body, so this asks
    /// the scan alone.
    fn decl_class(src: &str, stage: Stage, name: &str) -> Prec {
        use super::super::ast::{Decl, DeclId};
        use super::super::{parser, pp, sema};
        let mut buf = std::vec![0u8; 1 << 21];
        let a = Arena::new(&mut buf);
        let text = a.alloc_str(src).expect("arena");
        let pre = pp::preprocess(&a, text).expect("preprocesses");
        let ast = parser::resolve_io(&a, parser::parse(&a, pre.tokens).expect("parses"), stage).expect("io");
        let types = sema::check(&a, &ast, stage).expect("checks");
        let p = super::classify(&a, &ast, &types, stage).expect("classifies");
        let id = ast.decls.iter().position(|d| matches!(d, Decl::Var { name: n, .. } if *n == name)).expect("declared");
        p.decl(DeclId(id as u32))
    }

    /// Mesa's `add_default_precision_qualifier` (glsl_symbol_table.cpp:
    /// 194-207) REPLACES a default that exists in any scope: a `precision`
    /// statement in a block or in another function changes every later
    /// declaration, in Mesa. Only a type with no default yet gets one
    /// scoped to the block. Review probes (drm-shim GLSL IR): v_inner2 and
    /// v_inner4, a vertex `{ precision mediump float; }` or one in another
    /// function, then `float u`: `(declare (mediump ) float16_t u)`;
    /// f_inner2, a mediump fragment shader's `{ precision highp float; }`:
    /// `(declare (highp ) float u)`.
    #[test]
    fn a_precision_statement_replaces_an_existing_default_beyond_its_block() {
        let vs_head = "attribute highp vec4 p; attribute highp vec4 h; varying highp vec4 v;\n";
        let u = |body: &str| decl_class(&std::format!("{vs_head}{body}"), Stage::Vertex, "u");
        assert_eq!(u("void main(){ gl_Position = p; { precision mediump float; } float u = h.z; v = vec4(u); }"), M);
        assert_eq!(
            u("void f(){ precision mediump float; }\nvoid main(){ gl_Position = p; f(); float u = h.z; v = vec4(u); }"),
            M
        );
        let f = decl_class(
            "precision mediump float; varying highp vec4 v;\n\
             void main(){ { precision highp float; } float u = v.z; gl_FragColor = vec4(u); }",
            Stage::Fragment,
            "u",
        );
        assert_eq!(f, H);
        /* no default before it: the block's own, gone after it */
        let f = decl_class(
            "varying highp vec4 v;\n\
             void main(){ { precision highp float; } mediump float u = v.z; gl_FragColor = vec4(u); }",
            Stage::Fragment,
            "u",
        );
        assert_eq!(f, M);
    }

    /// A run-time index into a folded `const` mediump vector or array
    /// reads its 16-bit storage in Mesa (second review B-A, probe p21:
    /// `16 bcsel ... 0x2e66, 0x3266, 0x34cd`, then `f2f32`, then the 32-bit
    /// multiply), while a constant index or a whole read gets the exact
    /// 32-bit value. The slot holds the 32-bit value (B4 (b)), so the
    /// run-time-indexed read records Unknown: no single class is right.
    #[test]
    fn a_run_time_index_into_a_folded_const_is_unknown() {
        let g = vs("uniform int i; varying highp vec4 v; const mediump vec3 cv = vec3(0.1, 0.2, 0.3);\n\
                    void main(){ gl_Position = p; v = vec4(a * cv[i], b * cv.y, a * cv[1], 1.0); }");
        assert!(!alu(&g, AluOp::Bcsel).is_empty());
        assert!(alu(&g, AluOp::Bcsel).iter().all(|&p| p == Prec::Unknown), "{:?}", alu(&g, AluOp::Bcsel));
        assert_eq!(alu(&g, AluOp::Fmul), [H, H, H], "the readers stay High; the Unknown value refuses");
        /* a highp const, and a mediump local that is no folded const */
        let g = vs("uniform int i; varying highp vec4 v; const highp vec3 cv = vec3(0.1, 0.2, 0.3);\n\
                    void main(){ gl_Position = p; v = vec4(a * cv[i]); }");
        assert!(alu(&g, AluOp::Bcsel).iter().all(|&p| p == H));
    }

    /// Second review S-A: forms Mesa's rules decide that were Unknown.
    /// A `const` variable argument of pow is folded by `ast_to_hir` and
    /// passed to the mediump parameter: its fp16 value, so its read records
    /// Medium (the slot holds the 32-bit value and the read narrows it). A
    /// `?:` condition of literal arithmetic or of a component of a literal
    /// vector is evaluated (Mesa folds both and takes the chosen side).
    #[test]
    fn mesa_s_rules_decide_more_constants() {
        let g = vs(
            "attribute mediump vec4 q; varying highp vec4 v; const mediump float E = 0.1; const highp float F = 0.2;\n\
                    void main(){ gl_Position = p; v = vec4(pow(q.x, E), pow(q.y, F), 0.0, 1.0); }",
        );
        let loads = of(&g, |o| matches!(o, Op::LoadLocal { .. }));
        assert_eq!(loads[..2], [M, M], "the two exponents, each read at its fp16 value");
        let head = "attribute mediump vec4 q; attribute highp vec4 h; varying highp vec4 v;\n";
        for cond in ["2.0 * 0.5 > 0.7", "vec2(1.0, 0.0).x > 0.5", "1.0 - 0.25 >= 0.75"] {
            let g =
                vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4((({cond}) ? q.y : h.z) * q.x); }}"));
            assert_eq!(alu(&g, AluOp::Fmul).last(), Some(&M), "{cond}");
        }
    }

    /// Third review B-C, B-D: the `?:` evaluator is a whitelist of forms
    /// measured against Mesa. uint arithmetic wraps on the u32 lane in Mesa
    /// (`0u - 1u > 5u` is true, probe p3/u1: a 16-bit product), and a
    /// converting constructor converts (`ivec2(1.5, 0.0).x == 1` is true,
    /// p3/u2). Neither is evaluated here: the `?:` is Unknown, never the
    /// wrong side. Int arithmetic, shifts, division, conversions and bool or
    /// int constructors are not evaluated either.
    #[test]
    fn a_condition_outside_the_measured_forms_is_unknown() {
        let head = "attribute mediump vec4 q; attribute highp vec4 h; varying highp vec4 v;\n";
        for cond in [
            "ivec2(1.5, 0.0).x == 1",
            "1 + 2 == 3",
            "(-1 < 0) == true",
            "bvec2(true, false).x",
            "int(1.5) == 1",
            "5 / 2 == 2",
        ] {
            let g =
                vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4((({cond}) ? q.x : h.x) * q.y); }}"));
            assert_eq!(alu(&g, AluOp::Fmul).last(), Some(&Prec::Unknown), "{cond}");
        }
        if crate::REPORT_ES2 {
            return;
        }
        let g = compile(
            "#version 300 es\nin highp vec4 p; in mediump vec4 q; in highp vec4 h; out highp vec4 v;\n\
             void main(){ gl_Position = p; v = vec4((0u - 1u > 5u ? q.x : h.x) * q.y); }",
            Stage::Vertex,
        );
        assert_eq!(alu(&g, AluOp::Fmul).last(), Some(&Prec::Unknown), "0u - 1u > 5u");
        /* the measured forms still choose */
        let g = vs(&std::format!(
            "{head}void main(){{ gl_Position = p; v = vec4(((0.1 + 0.2 > 0.3) ? h.x : q.x) * q.y); }}"
        ));
        assert_eq!(alu(&g, AluOp::Fmul).last(), Some(&M), "0.1 + 0.2 > 0.3 is false in f32");
    }

    /// Third review B-E: a constant passed through a user function's
    /// parameter into pow, asin or acos. Mesa inlines and folds the
    /// parameter's `f2fmp` to the fp16 constant (probe p3/u5: `load_const
    /// (0x3dccc000)`, `fpow`); with a value that is no constant (p3/x1,
    /// `f(p.y)`) it reads the 32-bit value, as we do. The lowering does not
    /// carry the constant into the body, so that read records Unknown.
    #[test]
    fn a_constant_through_a_parameter_into_pow_is_unknown() {
        let src = |arg: &str| {
            std::format!(
                "attribute mediump vec4 q; varying highp vec4 v; const mediump float E = 0.1;\n\
                 mediump float f(const in mediump float e){{ return pow(q.z, e); }}\n\
                 mediump float g(mediump float e){{ return f(e); }}\n\
                 void main(){{ gl_Position = p; v = vec4({arg}); }}"
            )
        };
        for arg in ["f(E)", "g(E)", "f(0.1)"] {
            let g = vs(&src(arg));
            assert!(g.prec.contains(&Prec::Unknown), "{arg}: the exponent read");
        }
        for arg in ["f(p.y)", "g(p.y)"] {
            let g = vs(&src(arg));
            assert!(!g.prec.contains(&Prec::Unknown), "{arg}");
        }
    }

    /// Fourth review: NIR proves a value constant after inlining and
    /// propagation, which the front end cannot see. Probes p4/f06 (`highp
    /// float t = 0.1; f(t)`: `load_const (0x3dccc000)`, `fpow`) and p4/f09
    /// (`t` set in an `if`: a `bcsel` of 0x3e99a000 and 0x3dccc000, then
    /// `fpow`): Mesa uses the fp16 values, we read 32 bits. So an argument
    /// of pow, asin or acos is Unknown unless it is known to vary: it reads
    /// an input or a uniform (or a parameter given one). A whitelist of the
    /// non-constant, not a list of constant forms. (A highp local read by
    /// pow directly makes the call High, unlowered, which Mesa computes at
    /// 32 bits too.)
    #[test]
    fn an_argument_not_known_to_vary_into_pow_is_unknown() {
        let head = "attribute mediump vec4 q; varying highp vec4 v; uniform highp float uh;\n\
                    mediump float f(mediump float e){ return pow(q.z, e); }\n";
        for body in
            ["highp float t = 0.1; v = vec4(f(t));", "highp float t = 0.1; if (uh > 0.0) t = 0.3; v = vec4(f(t));"]
        {
            let g = vs(&std::format!("{head}void main(){{ gl_Position = p; {body} }}"));
            assert!(g.prec.contains(&Prec::Unknown), "{body}");
        }
        for body in ["v = vec4(f(uh));", "v = vec4(f(p.y));", "v = vec4(pow(q.z, uh + p.x));"] {
            let g = vs(&std::format!("{head}void main(){{ gl_Position = p; {body} }}"));
            assert!(!g.prec.contains(&Prec::Unknown), "{body}");
        }
    }

    /// Fifth review B-F, B-G: "known to vary" was still a guess about what
    /// NIR folds. Probes p5/g01 (`r(h.x)` with `e = 0.1;` in `r`), g02
    /// (`f(uh * 0.0 + 0.1)`), g04 (`f(h.x - h.x + 0.1)`), g05
    /// (`f(h.x * 0.0 + 0.1)`): Mesa's final NIR is `fpow` of `0x3dccc000`,
    /// the fp16 value; we recorded High. Only a bare read of an input, a
    /// uniform or a built-in input (a swizzle, a member or an index of one)
    /// is known to vary, or a parameter every call gives one and the callee
    /// never writes. g08 (`f(us.m)`, `f(ua[1])`) and g06 (`g(h.x)`, `g`
    /// calling `f`) stay High, as in Mesa; g10's `x * 1.0`, g11's
    /// `max(h.x, 0.1)` and g09's `float(gl_VertexID)`, which Mesa keeps at
    /// 32 bits, are refused: a refusal, never a wrong record.
    #[test]
    fn only_a_bare_read_is_known_to_vary() {
        let head = "attribute mediump vec4 q; attribute highp vec4 h; varying highp vec4 v; uniform highp float uh;\n\
                    struct S { mediump float m; }; uniform S us; uniform mediump float ua[2];\n\
                    mediump float f(mediump float e){ return pow(q.z, e); }\n\
                    mediump float r(mediump float e){ e = 0.1; return pow(q.z, e); }\n\
                    mediump float g(mediump float x){ return f(x); }\n\
                    mediump float k(mediump float x){ return f(x * 1.0); }\n";
        for arg in [
            "r(h.x)",
            "f(uh * 0.0 + 0.1)",
            "f(h.x - h.x + 0.1)",
            "f(h.x * 0.0 + 0.1)",
            "k(h.x)",
            "f(max(h.x, 0.1))",
            "f(h.y > 0.5 ? h.x : 0.1)",
        ] {
            let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4({arg}); }}"));
            assert!(g.prec.contains(&Prec::Unknown), "{arg}");
        }
        for arg in ["f(us.m)", "f(ua[1])", "f(h.zw.x)", "g(h.x)", "f(uh)"] {
            let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4({arg}); }}"));
            assert!(!g.prec.contains(&Prec::Unknown), "{arg}");
        }
    }

    /// Sixth review B-H: a local of `main` named like an attribute or a
    /// uniform is that local, not the input. The arguments' "known to vary"
    /// was decided in the callee's scope, where `main`'s locals are not
    /// visible, so the name fell through to the input. Probes p6/h13
    /// (`highp float h = 0.1; f(h)`, attribute `h`), h14 (a local `uh`,
    /// uniform `uh`), h15 (`highp vec4 h = vec4(0.1); f(h.x)`): Mesa's
    /// final NIR is `fpow` of 0x3dccc000, the fp16 constant. And p6/h01:
    /// a local in the callee named like its parameter is that local.
    #[test]
    fn a_local_named_like_an_input_is_the_local() {
        let head = "attribute mediump vec4 q; attribute highp vec4 h; varying highp vec4 v; uniform highp float uh;\n\
                    mediump float f(mediump float e){ return pow(q.z, e); }\n\
                    mediump float k(mediump float e){ { mediump float e = 0.1; return pow(q.z, e); } }\n";
        let wrong = [
            "highp float h = 0.1; v = vec4(f(h));",
            "highp float uh = 0.1; v = vec4(f(uh));",
            "highp vec4 h = vec4(0.1); v = vec4(f(h.x));",
            "v = vec4(k(h.x));",
        ]
        .into_iter()
        .filter(|body| {
            !vs(&std::format!("{head}void main(){{ gl_Position = p; {body} }}")).prec.contains(&Prec::Unknown)
        })
        .collect::<Vec<_>>();
        assert!(wrong.is_empty(), "recorded with no Unknown: {wrong:?}");
        let g = vs(&std::format!("{head}void main(){{ gl_Position = p; v = vec4(f(h.x), f(uh), 0.0, 0.0); }}"));
        assert!(!g.prec.contains(&Prec::Unknown), "the inputs themselves");
    }

    /// Sixth review (c): p3/x1 shares the constant 0.5 between pow's
    /// argument (pinned to its fp16 value) and asin's full-width expansion,
    /// which made it Unknown. 0.5 is an fp16 value: either width reads the
    /// same, so it is not pinned.
    #[test]
    fn an_exact_fp16_constant_argument_is_not_pinned() {
        let g = compile(
            "attribute highp vec4 p; attribute mediump float a; uniform mediump float mu; varying highp vec4 v;\n\
             mediump float f(mediump float e){ return pow(a, e); }\n\
             void main(){ gl_Position = p; v = vec4(pow(a, 0.5), pow(mu, 0.5), asin(a), f(p.y)); }",
            Stage::Vertex,
        );
        assert!(!g.prec.contains(&Prec::Unknown), "{:?}", g.prec);
    }

    /// The forms a pow argument is refused in, pinned on their own
    /// sources rather than by which corpus shaders hold them (round 8): a
    /// computed argument (glmark2's lit shaders, `pow(max(dot(N, H), 0.0),
    /// 100.0)`; a dot; a product; a local holding one) is Unknown; a bare
    /// read of a varying or a uniform is not.
    #[test]
    fn a_computed_pow_argument_is_refused_and_a_bare_read_is_not() {
        let head = "precision mediump float; varying vec3 n; uniform vec3 hv; uniform float uf;\n";
        let unknowns = |body: &str| {
            let g = compile(&std::format!("{head}void main(){{ {body} }}"), Stage::Fragment);
            g.prec.iter().filter(|&&p| p == Prec::Unknown).count()
        };
        for body in [
            "gl_FragColor = vec4(pow(max(dot(n, hv), 0.0), 100.0));",
            "gl_FragColor = vec4(pow(dot(n, hv), 100.0));",
            "gl_FragColor = vec4(pow(max(n.x, 0.0), 100.0));",
            "gl_FragColor = vec4(pow(max(n.x, 0.0), uf));",
            "gl_FragColor = vec4(pow(n.x * hv.x, 100.0));",
            "float m = max(dot(n, hv), 0.0); gl_FragColor = vec4(pow(m, 100.0));",
        ] {
            assert_eq!(unknowns(body), 1, "{body}");
        }
        for body in ["gl_FragColor = vec4(pow(n.x, 100.0));", "gl_FragColor = vec4(pow(n.x, uf));"] {
            assert_eq!(unknowns(body), 0, "{body}");
        }
    }

    /// The copy back of an `out` or `inout` parameter into the caller's
    /// variable reads the parameter's slot, a 32-bit copy: High, never
    /// None. It ran under the statement `f(m);`, a void call, whose
    /// instructions record None (the width author's out-parameter
    /// reference, round 8). Mesa stores the parameter at 32 bits (A.5) and
    /// narrows it into a mediump `m` at the copy.
    #[test]
    fn an_out_parameter_s_copy_back_has_a_class() {
        let srcs = [
            "void f(out mediump float r) { r = a * 2.0; }\n\
             void main() { mediump float m; f(m); v = m * 1.5; gl_Position = p; }",
            "void f(inout mediump float r) { r = r * 2.0; }\n\
             void main() { mediump float m = a; f(m); v = m * 1.5; gl_Position = p; }",
            "void f(out mediump vec2 r) { r = vec2(a, b); }\n\
             void main() { mediump vec2 m; f(m); v = m.x * m.y; gl_Position = p; }",
            "struct S { mediump float x; highp float y; };\n\
             void f(out S r) { r = S(a, b); }\n\
             void main() { S m; f(m); v = m.x * m.y; gl_Position = p; }",
        ];
        for src in srcs {
            let g = vs(&std::format!("varying highp float v;\n{src}"));
            let loads: Vec<(usize, Prec)> = g
                .ops
                .iter()
                .zip(&g.prec)
                .enumerate()
                .filter(|(_, (o, _))| matches!(o, Op::LoadLocal { .. }))
                .map(|(i, (_, p))| (i, *p))
                .collect();
            assert!(loads.iter().all(|&(_, p)| p != Prec::None), "{src}\n{loads:?}");
        }
        /* the copy back reads the parameter's 32-bit slot: High (Mesa,
         * review probe p8/n7: `f2fmp (var_ref lowerp)` into the mediump
         * variable), not Medium */
        let g = vs("varying highp float v;\n\
                    void f(out mediump float r) { r = a * 2.0; }\n\
                    void main() { mediump float m; f(m); v = m * 1.5; gl_Position = p; }");
        let r = g.locals.iter().position(|(n, _)| n == "r").expect("the parameter's slot");
        let copy =
            g.ops.iter().zip(&g.prec).find(|(o, _)| matches!(o, Op::LoadLocal { local, .. } if local.0 as usize == r));
        assert_eq!(copy.map(|c| *c.1), Some(H), "the copy back");
    }

    /// The copy IN of an `inout` argument that is an array element (at a
    /// constant or a run-time index), a swizzle or a struct is a full-width
    /// read into the 32-bit parameter (eighth review B-I; Mesa, probes
    /// p8/n2, n3: `inout_tmp`, then `lowerp = f162f(inout_tmp)`): High.
    /// It ran under the void call's context and recorded None.
    #[test]
    fn an_inout_argument_s_copy_in_is_full_width() {
        let head = "varying mediump vec4 v; uniform int ui; attribute mediump vec4 q;\n";
        for (src, var) in [
            (
                "void sp(inout mediump vec4 io){ io = io * 0.5; }\n\
                 void main(){ mediump vec4 arr[2]; arr[0] = q; arr[1] = q; sp(arr[1]); v = arr[0] + arr[1]; gl_Position = p; }",
                "arr",
            ),
            (
                "void sp(inout mediump vec4 io){ io = io * 0.5; }\n\
                 void main(){ mediump vec4 arr[2]; arr[0] = q; arr[1] = q; sp(arr[ui]); v = arr[0] + arr[1]; gl_Position = p; }",
                "arr",
            ),
            (
                "void sp(inout mediump vec2 io){ io = io * 0.5; }\n\
                 void main(){ mediump vec4 w = q; sp(w.zx); v = w; gl_Position = p; }",
                "w",
            ),
            (
                "struct S { mediump vec2 m; highp float k; }; void ss(inout S t){ t.m *= 2.0; t.k += 1.0; }\n\
                 void main(){ S s; s.m = q.xy; s.k = p.x; ss(s); v = vec4(s.m, s.k, 1.0); gl_Position = p; }",
                "s",
            ),
        ] {
            let g = vs(&std::format!("{head}{src}"));
            let first_op = g.ops.iter().position(|o| matches!(o, Op::Alu(AluOp::Fmul | AluOp::Fadd, _)));
            let ids: Vec<usize> = g.locals.iter().enumerate().filter(|(_, l)| l.0 == var).map(|(i, _)| i).collect();
            let copy: Vec<Prec> = g.ops[..first_op.unwrap_or(g.ops.len())]
                .iter()
                .zip(&g.prec)
                .filter(|(o, _)| matches!(o, Op::LoadLocal { local, .. } if ids.contains(&(local.0 as usize))))
                .map(|(_, p)| *p)
                .collect();
            assert!(!copy.is_empty() && copy.iter().all(|&p| p == H), "{src}\n{copy:?}");
            assert!(!alu(&g, AluOp::Bcsel).contains(&Prec::None), "{src}: the index ladder");
        }
    }

    /// The rule that makes "a float or bool value with class None"
    /// impossible (lower.rs `Lower::instr_prec`): such a value lowered
    /// under a context with no class records Unknown, never None, unless
    /// its site sets the class Mesa was measured at. Shown by taking the
    /// copy in's measured class away (`UNMEASURED`), as a new site that
    /// sets none would be: its loads, and the select ladder of a run-time
    /// index (a float `Bcsel`, whose kind comes from its sides; ninth
    /// review should-fix 1), record Unknown and are counted.
    #[test]
    fn a_float_value_under_no_context_is_unknown_never_none() {
        for (index, ladder) in [("1", false), ("ui", true)] {
            let src = std::format!(
                "varying mediump vec4 v; attribute mediump vec4 q; uniform int ui;\n\
                 void sp(inout mediump vec4 io){{ io = io * 0.5; }}\n\
                 void main(){{ mediump vec4 arr[2]; arr[0] = q; arr[1] = q; sp(arr[{index}]); v = arr[0] + arr[1]; gl_Position = p; }}"
            );
            super::super::lower::UNMEASURED.with(|u| u.set(true));
            super::super::lower::NO_CONTEXT.with(|n| n.set(0));
            let mut buf = std::vec![0u8; 1 << 22];
            let a = Arena::new(&mut buf);
            let l = compile_to_ir(
                &a,
                Stage::Vertex,
                a.alloc_str(&std::format!("attribute highp vec4 p;\n{src}")).expect("arena"),
            );
            super::super::lower::UNMEASURED.with(|u| u.set(false));
            let l = l.expect("compiles");
            let m = &l.module;
            assert!(super::super::lower::NO_CONTEXT.with(|n| n.get()) > 0, "{src}: the copy in fell to the rule");
            assert!(unclassed(m).is_empty(), "{src}: {:?}", unclassed(m));
            assert!(m.prec.contains(&Prec::Unknown), "{src}");
            if ladder {
                let k = kinds(m);
                let selects: Vec<Prec> = (0..m.instrs.len())
                    .filter(|&i| matches!(m.instrs[i], Op::Alu(AluOp::Bcsel, _)) && k[i] == ValueKind::Float)
                    .map(|i| m.prec[i])
                    .collect();
                assert!(selects.contains(&Prec::Unknown), "{src}: the ladder's selects {selects:?}");
            }
        }
    }

    /// `&&`, `||`, `^^` on bools lower to `Iand`, `Ior`, `Ixor`, which
    /// also serve int `&`, `|`, `^`: the value's kind, not the opcode,
    /// decides its class (ninth review B-K; they recorded None). A bool
    /// records the class of the tree it is in, as every float does:
    /// Medium in a lowered (mediump) tree, High in a full-width tree or in
    /// none. Mesa's GLSL IR has the same: `(var_ref b) || (var_ref c)` of
    /// two bool variables, which have no precision, is in no lowered tree
    /// (probe q1). The bit width of a bool is not this class: it is the
    /// width of the compares that made it (`pan_nir_lower_bool_to_bitsize`,
    /// docs/notes/mali-precision.md A.6), which the width pass works out
    /// from the compares; what it needs here is that a bool is never None,
    /// which it reads as an int (32 bits, no conversion). Mesa 26.2.1
    /// final NIR for the forms:
    ///
    /// - b1: `16 iand flt, flt`, `16 ixor`, `16 ior`, `16 inot`: the
    ///   compares are 16-bit, so are the bools made from them;
    /// - b2: `32 iand %7, %8` of a bool uniform and a widened 16-bit
    ///   compare: the uniform is 32-bit;
    /// - q2: `32 iand (ilt_pan ...), (i2i32 flt_pan)`: an int compare is
    ///   32-bit, and makes the `&&` 32-bit;
    /// - q1: `16 ieq_pan %8, %6` of two 16-bit bools: an `==` of bools is
    ///   a bool operation like `&&` (an `==` of ints is a 32-bit compare).
    #[test]
    fn a_bool_operation_records_its_tree_s_class_never_none() {
        let fs = |src: &str| compile(src, Stage::Fragment);
        /* b1 */
        let g = fs("precision mediump float; varying vec4 v;\n\
                    void main(){ bool b = v.x > 0.5 && v.y < 0.3; bool c = b ^^ (v.z > 0.1); bool d = !(b || c);\n\
                    gl_FragColor = (d || c) ? v : vec4(0.0); }");
        assert_eq!(alu(&g, AluOp::Iand), [M], "b && c of two mediump compares");
        assert_eq!(alu(&g, AluOp::Ixor), [M], "a bool local ^^ a mediump compare");
        assert_eq!(alu(&g, AluOp::Ior), [H, H], "|| of two bool locals: no lowered tree");
        assert_eq!(alu(&g, AluOp::Bnot), [H]);
        /* b2 */
        let g = vs("varying mediump vec4 v; uniform bool ub; uniform bvec2 ubv;\n\
                    void main(){ bvec2 q = bvec2(a > 0.0, ub); bool e = all(q) && any(ubv); v = e ? vec4(a) : vec4(float(ubv.y)); gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Iand), [H], "all(q) && any(ubv)");
        /* q1 */
        let g = fs("precision mediump float; varying vec4 v;\n\
                    void main(){ bool b = v.x > 0.5; bool c = v.y < 0.3; bool e = b == c; bool f = b != (v.z > 0.1);\n\
                    gl_FragColor = (e || f) ? v : vec4(0.0); }");
        assert_eq!(alu(&g, AluOp::Ieq), [H], "== of two bool locals");
        assert_eq!(alu(&g, AluOp::Ine), [M], "a bool local != a mediump compare");
        if !crate::REPORT_ES2 {
            /* q2: an int compare is a 32-bit bool, whatever tree it is in */
            let g = fs("#version 300 es\nprecision mediump float; in vec4 v; flat in mediump int kk; out vec4 o;\n\
                        void main(){ bool b = (kk < 3) && (v.x > 0.5); bool c = (kk == 2) == (v.y < 0.3);\n\
                        o = (b || c) ? v : vec4(0.0); }");
            assert_eq!(alu(&g, AluOp::Ilt), [H]);
            assert_eq!(alu(&g, AluOp::Iand), [H], "an int compare && a mediump compare");
            assert_eq!(alu(&g, AluOp::Ieq), [H, H], "== of ints, then == of that and a compare");
        }
    }

    /// An int value records None whatever opcode makes it, and a select
    /// is the kind of its two sides, not of its condition: an int `?:`
    /// and an int run-time index ladder are Int. The pack built-ins work
    /// a float's bits with integer operations: `Iand` of a float and an
    /// int mask, and unpackHalf2x16's select of a float or of bits made
    /// by integer operations, are bits, Int.
    #[test]
    fn an_int_select_and_float_bits_record_none() {
        let g = vs("varying mediump vec4 v; uniform bool ub; uniform int ui; uniform int ia[4];\n\
                    void main(){ int t = ub ? ui : 7; int s = ia[ui]; v = vec4(a * float(t + s)); gl_Position = p; }");
        let selects = alu(&g, AluOp::Bcsel);
        assert!(!selects.is_empty() && selects.iter().all(|&c| c == Prec::None), "{selects:?}");
        /* a bool uniform's load is the 0 or 1 the GL core stores, an int;
         * `I2b` makes the bool (lower.rs `bool_from_block`) */
        let loads = of(&g, |o| matches!(o, Op::LoadUniform { .. } | Op::LoadUniformIdx { .. }));
        assert!(!loads.is_empty() && loads.iter().all(|&c| c == Prec::None), "{loads:?}");
        assert_eq!(alu(&g, AluOp::I2b), [H], "the bool `ub`");
        if crate::REPORT_ES2 {
            return;
        }
        let src = "#version 300 es\nin highp vec4 p; in mediump vec4 q; uniform highp uint uu; out highp vec4 v;\n\
                   void main(){ v = vec4(float(packHalf2x16(q.xy)), unpackHalf2x16(uu), 0.0); gl_Position = p; }";
        let mut buf = std::vec![0u8; 1 << 22];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, a.alloc_str(src).expect("arena")).expect("compiles");
        let m = &l.module;
        let k = kinds(m);
        let src_k = |s: super::super::ir::Src| k[s.val.0 as usize];
        let class_of = |f: &dyn Fn(&Op) -> bool| -> Vec<Prec> {
            (0..m.instrs.len()).filter(|&i| f(&m.instrs[i])).map(|i| m.prec[i]).collect()
        };
        let bits_and = class_of(&|o| matches!(o, Op::Alu(AluOp::Iand, [x, ..]) if src_k(*x) == ValueKind::Float));
        assert_eq!(bits_and, [Prec::None], "packHalf2x16's `Iand` of the float's bits");
        let mixed = class_of(&|o| matches!(o, Op::Alu(AluOp::Bcsel, [_, x, y]) if src_k(*x) != src_k(*y)));
        assert_eq!(mixed, [Prec::None], "unpackHalf2x16's select of a float or bits");
        /* isnan and isinf of a mediump value: the int compare of the
         * float's bits is a 32-bit bool in the mediump tree it serves */
        let g = compile(
            "#version 300 es\nin highp vec4 p; in mediump vec4 q; out mediump vec4 v;\n\
             void main(){ v = (isnan(q.x * 0.5) || isinf(q.y * 0.5)) ? q : vec4(0.0); gl_Position = p; }",
            Stage::Vertex,
        );
        assert_eq!(alu(&g, AluOp::Fmul), [M, M], "the trees are mediump");
        assert_eq!(alu(&g, AluOp::Ult), [H], "isnan's compare of the bits");
        assert_eq!(alu(&g, AluOp::Ieq), [H], "isinf's compare of the bits");
        assert_eq!(alu(&g, AluOp::Iand), [Prec::None, Prec::None], "the bits");
    }

    /// The pack built-ins (ES 3.00) take a highp argument: Mesa widens it
    /// and computes the clamp, scale and rounding at 32 bits (probes p8/m1,
    /// m3, drm-shim GLSL IR: `vec2 f162f`, `vec2 sat`, `vec2 *`, `vec2
    /// round_even`). Their float steps record High; they ran under the
    /// uint call's context and recorded None (eighth review B-J).
    #[test]
    fn a_pack_builtin_s_float_steps_are_full_width() {
        if crate::REPORT_ES2 {
            return;
        }
        for call in ["packHalf2x16(q.xy)", "packUnorm2x16(q.xy)", "packSnorm2x16(q.zw)"] {
            let g = compile(
                &std::format!(
                    "#version 300 es\nin highp vec4 p; in mediump vec4 q; out mediump vec4 v;\n\
                     void main(){{ v = vec4(float({call})); gl_Position = p; }}"
                ),
                Stage::Vertex,
            );
            let float_steps: Vec<Prec> = g
                .ops
                .iter()
                .zip(&g.prec)
                .filter(|(o, _)| {
                    matches!(
                        o,
                        Op::Alu(
                            AluOp::Fabs
                                | AluOp::Ffma
                                | AluOp::Ffloor
                                | AluOp::Fmax
                                | AluOp::Fmin
                                | AluOp::Fmul
                                | AluOp::Ftrunc
                                | AluOp::Fsign
                                | AluOp::Fadd,
                            _
                        )
                    )
                })
                .map(|(_, p)| *p)
                .collect();
            assert!(!float_steps.is_empty() && float_steps.iter().all(|&p| p == H), "{call}: {float_steps:?}");
        }
    }

    /// The eighth review's six corpus-style shaders (ES 3.00 control
    /// flow, look-ups, `inverse`/`transpose`/`outerProduct`, the pack
    /// built-ins, `out`/`inout` through array elements, swizzles and
    /// structs, a discard loop, `gl_PointCoord`, globals, `++`/`--`):
    /// every value has its kind's class, no expression escapes the scan, and
    /// no value is left to the no-context rule.
    #[test]
    fn the_eighth_review_s_shaders_give_every_value_its_kind_s_class() {
        let shaders = [
            (
                Stage::Fragment,
                r#"#version 300 es
precision mediump float; in mediump vec4 vc; in highp vec2 uvh; flat in mediump int fi; uniform mediump sampler2D s; uniform highp usampler2D us; uniform mediump int ui; uniform mediump vec4 uv; layout(location = 0) out mediump vec4 o0; layout(location = 1) out highp vec4 o1;
void main(){ vec4 r = vec4(0.0); switch (ui) { case 0: r = vc; break; case 1: r = vc * 2.0; break; default: r = uv; } int k = 0; do { r += vec4(0.1); k++; } while (k < 3); o0 = r; o1 = vec4(dFdx(vc.x), fwidth(vc.y), float(fi & 3), float(ui << 1)); }"#,
            ),
            (
                Stage::Fragment,
                r#"#version 300 es
precision mediump float; in mediump vec4 vc; in highp vec2 uvh; flat in mediump int fi; uniform mediump sampler2D s; uniform highp usampler2D us; uniform mediump int ui; uniform mediump vec4 uv; out mediump vec4 o;
void main(){ mediump vec4 t = textureGrad(s, uvh, vec2(0.1), vec2(0.2)) + texelFetch(s, ivec2(uvh), 0); uvec4 q = texelFetch(us, ivec2(1, 1), 0); o = t + vec4(q) * 0.5; gl_FragDepth = vc.x; }"#,
            ),
            (
                Stage::Vertex,
                r#"#version 300 es
in highp vec4 p; in mediump vec4 a; out mediump vec4 v; uniform mediump mat3 m3; uniform highp mat4 m4;
void main(){ mat3 im = inverse(m3); mat4 t = transpose(m4); mat2 op = outerProduct(a.xy, a.zw); uint ph = packHalf2x16(a.xy); v = vec4(im * a.xyz, 1.0) + t * p + vec4(op[0], unpackHalf2x16(ph)) + vec4(float(floatBitsToInt(a.x) >> 16)); gl_Position = p; }"#,
            ),
            (
                Stage::Vertex,
                r#"attribute highp vec4 p; attribute mediump vec4 a; varying mediump vec4 v; uniform mediump vec4 ua[4]; uniform int ui;
struct L { mediump vec3 c; highp float k; }; uniform L ls[2];
void sp(out mediump vec2 o2, inout mediump vec4 io, out L lo){ o2 = a.xy * 2.0; io = io * 0.5 + a; lo.c = a.xyz; lo.k = p.x; }
void main(){ mediump vec4 w = a; mediump vec2 q; L l; mediump vec4 arr[2]; arr[0] = a; arr[1] = a.wzyx; sp(q, w, l); sp(arr[1].xy, arr[ui], l); mediump vec3 sw = vec3(0.0); sp(sw.yz, w, l); v = w + vec4(q, l.c.x, l.k) + arr[0] + arr[1] + vec4(sw, 1.0) + ua[ui] + vec4(ls[1].c, ls[0].k); gl_Position = p; }"#,
            ),
            (
                Stage::Fragment,
                r#"precision mediump float; varying mediump vec4 v; uniform sampler2D s; uniform mediump float uf;
mediump float g(mediump float x){ return x > 0.5 ? x : -x; }
void cp(out mediump float r){ r = v.x * 2.0; } void pass(out mediump float r2){ cp(r2); }
void main(){ mediump float m; pass(m); vec4 c = texture2D(s, gl_PointCoord) * vec4(m); for (int i = 0; i < 4; i++) { if (c.x > uf) { discard; } c += vec4(g(c.y)); } gl_FragColor = c + (gl_FrontFacing ? vec4(1.0) : vec4(0.0)) + vec4(gl_FragCoord.xy, mix(c.zw, v.xy, bvec2(true, false))); }"#,
            ),
            (
                Stage::Vertex,
                r#"attribute highp vec4 p; attribute mediump vec4 a; varying highp vec4 v; uniform highp float uh;
highp float glob = 0.25; const mediump vec3 cv = vec3(0.1, 0.2, 0.3);
void main(){ glob = glob * uh; mediump float t = a.x; t++; --t; v = vec4(glob, cv) + vec4(t) + vec4(pow(a.y, 2.0), asin(clamp(a.z, -1.0, 1.0)), length(a.xy), atan(a.x, a.y)); gl_Position = p; }"#,
            ),
        ];
        for (stage, src) in shaders {
            if crate::REPORT_ES2 && src.starts_with("#version 300 es") {
                continue;
            }
            super::super::lower::UNSEEN.with(|n| n.set(0));
            super::super::lower::NO_CONTEXT.with(|n| n.set(0));
            let mut buf = std::vec![0u8; 1 << 22];
            let a = Arena::new(&mut buf);
            let l = compile_to_ir(&a, stage, a.alloc_str(src).expect("arena")).expect("compiles");
            let m = &l.module;
            assert!(m.has_prec(), "{src}");
            let bad = unclassed(m);
            assert!(bad.is_empty(), "{src}\nvalues whose class is not their kind's: {bad:?}");
            assert_eq!(super::super::lower::UNSEEN.with(|n| n.get()), 0, "{src}");
            assert_eq!(super::super::lower::NO_CONTEXT.with(|n| n.get()), 0, "{src}");
        }
    }

    /// Every arm of [`Op::value_kind`] and [`AluOp::result_kind`] on a
    /// real lowered shader: each row is a GLSL form, the instructions it
    /// lowers to that the row picks, and the kind the form has in GLSL.
    /// The width pass reads these kinds, so a changed arm fails a row here
    /// (tenth review, should-fix 1: `gl_FrontFacing` as a float and an
    /// `Iand` of one bool and one int as a bool survived every test). The
    /// `Iand` of a bool and a non-bool is never made; the corpus test
    /// asserts that, as the arm's comment says.
    #[test]
    fn every_value_kind_arm_is_the_kind_of_its_glsl_form() {
        use super::super::ir::IoSlot;
        type Pick = fn(&Module, &Op) -> bool;
        fn uni(m: &Module, name: &str) -> u16 {
            m.uniforms.iter().find(|u| u.name == name).unwrap_or_else(|| panic!("no uniform {name}")).dword_off
        }
        fn slot(m: &Module, l: super::super::ir::LocalId, name: &str) -> bool {
            m.locals.get(l.0 as usize).is_some_and(|d| d.name == name)
        }
        fn is(o: &Op, a: AluOp) -> bool {
            matches!(o, Op::Alu(x, _) if *x == a)
        }
        let fs1 = "precision mediump float; varying vec4 v; uniform vec4 uf; uniform int ui; uniform bool ub;\n\
                   uniform vec4 ua[3]; uniform int ia[3]; uniform sampler2D s;\n\
                   void main(){ float x = v.x + uf.y; int i = ui * 2 + ia[ui]; bool b = ub && (v.y < 0.5);\n\
                   if (gl_FrontFacing) discard;\n\
                   for (int k = 0; k < 2; k++) { x = x * 0.5; b = !b; }\n\
                   bvec2 bb = bvec2(b, v.z > 0.0);\n\
                   gl_FragColor = texture2D(s, v.xy) * x + ua[i] + gl_FragCoord\n\
                       + vec4(gl_PointCoord, float(i), float(any(bb))) + vec4(float(b || bb.y), float(-i), float(int(x)), float(bool(x))); }";
        let int_sel = "precision mediump float; varying vec4 v; uniform int ui;\n\
                       void main(){ int t = v.x > 0.5 ? ui : 3; gl_FragColor = vec4(float(t)); }";
        let float_sel = "precision mediump float; varying vec4 v; uniform float uf;\n\
                         void main(){ float t = v.x > 0.5 ? uf : v.y; gl_FragColor = vec4(t); }";
        let vs1 = "attribute highp vec4 p; attribute mediump vec4 a; varying mediump vec4 v; uniform bvec2 ubv; uniform int ui;\n\
                   void main(){ v = vec4(float(all(ubv)), float(ui == 2), float(true), 0.0) + vec4(float(ubv.x ^^ ubv.y)); gl_Position = p; }";
        let fs3 = "#version 300 es\nprecision mediump float; in vec4 v; flat in int fi; uniform highp usampler2D us; uniform highp uint uu;\n\
                   out vec4 o;\n\
                   void main(){ uvec4 t = texture(us, v.xy); ivec2 z = textureSize(us, 0); uint w = (uu / 3u) >> 1u;\n\
                   int j = (fi & 3) << 1; o = vec4(t) + vec4(float(w), float(~j), float(z.x), float(uu < 2u)) + vec4(float(uint(v.x))); }";
        let vs3 = "#version 300 es\nin highp vec4 p; in mediump vec4 a; out highp vec4 v;\n\
                   void main(){ v = vec4(float(gl_VertexID), float(gl_InstanceID), 0.0, 0.0); gl_Position = p; }";
        let pack = "#version 300 es\nin highp vec4 p; in mediump vec4 a; out highp vec4 v;\n\
                    void main(){ v = vec4(float(packHalf2x16(a.xy))); gl_Position = p; }";
        let unpack = "#version 300 es\nin highp vec4 p; uniform highp uint uu; out highp vec4 v;\n\
                      void main(){ v = vec4(unpackHalf2x16(uu), 0.0, 0.0); gl_Position = p; }";
        use ValueKind::{Bool as B, Float as F, Int as I, Void as V};
        let rows: &[(&str, Stage, &str, Pick, ValueKind)] = &[
            ("a float literal", Stage::Fragment, fs1, |_, o| matches!(o, Op::ConstF32(_)), F),
            ("an int literal", Stage::Fragment, fs1, |_, o| matches!(o, Op::ConstI32(_)), I),
            ("a bool literal", Stage::Vertex, vs1, |_, o| matches!(o, Op::ConstBool(_)), B),
            (
                "a float uniform",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadUniform { dword_off, .. } if *dword_off == uni(m, "uf")),
                F,
            ),
            (
                "an int uniform",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadUniform { dword_off, .. } if *dword_off == uni(m, "ui")),
                I,
            ),
            (
                "a bool uniform's 0 or 1",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadUniform { dword_off, .. } if *dword_off == uni(m, "ub")),
                I,
            ),
            (
                "a float array uniform element",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadUniformIdx { base_dw, .. } if *base_dw == uni(m, "ua")),
                F,
            ),
            (
                "an int array uniform element",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadUniformIdx { base_dw, .. } if *base_dw == uni(m, "ia")),
                I,
            ),
            ("a varying", Stage::Fragment, fs1, |_, o| matches!(o, Op::LoadInput { slot: IoSlot::Varying(_), .. }), F),
            ("an attribute", Stage::Vertex, vs1, |_, o| matches!(o, Op::LoadInput { slot: IoSlot::Attrib(_), .. }), F),
            (
                "gl_FrontFacing",
                Stage::Fragment,
                fs1,
                |_, o| matches!(o, Op::LoadInput { slot: IoSlot::FrontFacing, .. }),
                B,
            ),
            (
                "gl_FragCoord",
                Stage::Fragment,
                fs1,
                |_, o| matches!(o, Op::LoadInput { slot: IoSlot::FragCoord, .. }),
                F,
            ),
            (
                "gl_PointCoord",
                Stage::Fragment,
                fs1,
                |_, o| matches!(o, Op::LoadInput { slot: IoSlot::PointCoord, .. }),
                F,
            ),
            (
                "a float local",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadLocal { local, .. } if slot(m, *local, "x")),
                F,
            ),
            (
                "an int local",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadLocal { local, .. } if slot(m, *local, "k")),
                I,
            ),
            (
                "a bool local",
                Stage::Fragment,
                fs1,
                |m, o| matches!(o, Op::LoadLocal { local, .. } if slot(m, *local, "b")),
                B,
            ),
            ("a look-up", Stage::Fragment, fs1, |_, o| matches!(o, Op::Tex { .. }), F),
            ("a store", Stage::Fragment, fs1, |_, o| matches!(o, Op::StoreLocal { .. } | Op::StoreOutput { .. }), V),
            ("discard", Stage::Fragment, fs1, |_, o| matches!(o, Op::Discard), V),
            ("+ of floats", Stage::Fragment, fs1, |_, o| is(o, AluOp::Fadd), F),
            ("* of floats", Stage::Fragment, fs1, |_, o| is(o, AluOp::Fmul), F),
            ("float(int)", Stage::Fragment, fs1, |_, o| is(o, AluOp::I2f), F),
            ("float(bool)", Stage::Fragment, fs1, |_, o| is(o, AluOp::B2f), F),
            ("< of floats", Stage::Fragment, fs1, |_, o| is(o, AluOp::Flt), B),
            ("< of ints", Stage::Fragment, fs1, |_, o| is(o, AluOp::Ilt), B),
            ("bool(float)", Stage::Fragment, fs1, |_, o| is(o, AluOp::F2b), B),
            ("a bool uniform made a bool", Stage::Fragment, fs1, |_, o| is(o, AluOp::I2b), B),
            ("!", Stage::Fragment, fs1, |_, o| is(o, AluOp::Bnot), B),
            ("any", Stage::Fragment, fs1, |_, o| is(o, AluOp::Bany), B),
            ("all", Stage::Vertex, vs1, |_, o| is(o, AluOp::Ball), B),
            ("== of ints", Stage::Vertex, vs1, |_, o| is(o, AluOp::Ieq), B),
            ("* and + of ints", Stage::Fragment, fs1, |_, o| is(o, AluOp::Imul) || is(o, AluOp::Iadd), I),
            ("- of an int", Stage::Fragment, fs1, |_, o| is(o, AluOp::Ineg), I),
            ("int(float)", Stage::Fragment, fs1, |_, o| is(o, AluOp::F2i), I),
            ("&& of bools", Stage::Fragment, fs1, |_, o| is(o, AluOp::Iand), B),
            ("|| of bools", Stage::Fragment, fs1, |_, o| is(o, AluOp::Ior), B),
            ("^^ of bools", Stage::Vertex, vs1, |_, o| is(o, AluOp::Ixor), B),
            ("?: of ints", Stage::Fragment, int_sel, |_, o| is(o, AluOp::Bcsel), I),
            ("?: of floats", Stage::Fragment, float_sel, |_, o| is(o, AluOp::Bcsel), F),
            /* ES 3.00 */
            ("gl_VertexID", Stage::Vertex, vs3, |_, o| matches!(o, Op::LoadInput { slot: IoSlot::VertexId, .. }), I),
            (
                "gl_InstanceID",
                Stage::Vertex,
                vs3,
                |_, o| matches!(o, Op::LoadInput { slot: IoSlot::InstanceId, .. }),
                I,
            ),
            (
                "a flat int input",
                Stage::Fragment,
                fs3,
                |m, o| matches!(o, Op::LoadInput { slot, .. } if m.inputs.iter().any(|d| d.slot() == *slot && d.name == "fi")),
                I,
            ),
            ("a uint look-up", Stage::Fragment, fs3, |_, o| matches!(o, Op::Tex { .. }), I),
            ("textureSize", Stage::Fragment, fs3, |_, o| matches!(o, Op::TexSize { .. }), I),
            ("/ and >> of uints", Stage::Fragment, fs3, |_, o| is(o, AluOp::Udiv) || is(o, AluOp::Ushr), I),
            ("& and << of ints", Stage::Fragment, fs3, |_, o| is(o, AluOp::Iand) || is(o, AluOp::Ishl), I),
            ("~ of an int", Stage::Fragment, fs3, |_, o| is(o, AluOp::Inot), I),
            ("float(uint)", Stage::Fragment, fs3, |_, o| is(o, AluOp::U2f), F),
            ("uint(float)", Stage::Fragment, fs3, |_, o| is(o, AluOp::F2u), I),
            ("< of uints", Stage::Fragment, fs3, |_, o| is(o, AluOp::Ult), B),
            ("packHalf2x16's integer work on the float's bits", Stage::Vertex, pack, |_, o| is(o, AluOp::Iand), I),
            ("unpackHalf2x16's selects of a float or bits", Stage::Vertex, unpack, |_, o| is(o, AluOp::Bcsel), I),
        ];
        let mut buf = std::vec![0u8; 1 << 22];
        for &(what, stage, src, pick, want) in rows {
            if crate::REPORT_ES2 && src.starts_with("#version 300 es") {
                continue;
            }
            let a = Arena::new(&mut buf);
            let l = compile_to_ir(&a, stage, a.alloc_str(src).expect("arena"))
                .unwrap_or_else(|e| panic!("{what}: line {}: {}", e.line, e.msg));
            let m = &l.module;
            let k = kinds(m);
            let got: Vec<ValueKind> = (0..m.instrs.len()).filter(|&i| pick(m, &m.instrs[i])).map(|i| k[i]).collect();
            assert!(!got.is_empty(), "{what}: the shader has no such instruction");
            assert!(got.iter().all(|&x| x == want), "{what}: {got:?}, want {want:?}");
        }
    }

    #[test]
    fn f16_exact_knows_the_f16_values() {
        for x in [0.0f32, -0.0, 1.0, 1.5, 65504.0, -2.0, 6.103_515_6e-5, 5.960_464_5e-8, 0.000_122_070_31] {
            assert!(super::f16_exact(x), "{x}");
        }
        for x in [1.3f32, 65520.0, 1.0e-9, f32::NAN, f32::INFINITY, 1.0 + 1.0 / 4096.0] {
            assert!(!super::f16_exact(x), "{x}");
        }
    }
}
