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
        3 => Prec::Fold32,
        4 => Prec::Unknown,
        _ => Prec::None,
    }
}

/// Three bits of a table entry. No class is [`Prec::Fold32`] (only the
/// lowering's folds are); it has its code for completeness.
const fn bits_of(p: Prec) -> u8 {
    match p {
        Prec::None => 0,
        Prec::High => 1,
        Prec::Medium => 2,
        Prec::Fold32 => 3,
        Prec::Unknown => 4,
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
fn join(x: Prec, y: Prec) -> Prec {
    match (x, y) {
        (Prec::High, _) | (_, Prec::High) => Prec::High,
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
                self.vars.push(self.a, Var { name, class, folds, medium_sampler })?;
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
                            self.vars.push(self.a, Var { name, class, folds: false, medium_sampler })?;
                        }
                    }
                    self.stmt(b)?;
                    self.cut(mark);
                    self.defaults = saved;
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
                self.defaults = saved;
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
                self.defaults = saved;
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
        self.defaults = saved;
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
                if self.folds(c) {
                    /* `ast_to_hir` takes the chosen side itself (:1973-1978);
                     * which one is not worked out here, so the two are
                     * joined */
                    let flags = (self.info(t) | self.info(f)) & EMITS;
                    self.set(e, Kind::Op, typed(join(self.class(t), self.class(f))), flags);
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
                self.set(e, Kind::Deref, typed(self.deref_class(b)), folds | emits);
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
                if ids.iter().all(|&x| self.folds(x) || self.class(x) == Prec::Medium) {
                    Prec::Medium
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
                let j = kind == Kind::Op && lowered;
                self.mark(c, false, folded);
                self.mark(t, j, folded);
                self.mark(f, j, folded);
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
    use super::super::ir::{AluOp, Module, Op, Prec, ValueId};
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
        assert_eq!(local(&g, "c"), M, "a const mediump float is stored at 16 bits");
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
        let g = compile(
            "#version 300 es\nprecision mediump float;\nout vec4 o;\n\
             void main(){ o = vec4(gl_FragCoord.x * 0.5, 0.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        assert_eq!(alu(&g, AluOp::Fmul), [H]);
    }

    /// Open question E.5: the front end folds `x / c` into a multiply by
    /// the reciprocal it computes at 32 bits (lower.rs `reciprocal`); Mesa
    /// computes `frcp` of the 16-bit `c` at 16 bits. Where `c` has no
    /// exact f16 value the two differ (`/ 1.3`: 0.76904 against 0.76953),
    /// so the constant is marked [`Prec::Fold32`]; where it has one they
    /// agree and it is Medium.
    #[test]
    fn a_reciprocal_folded_in_a_lowered_tree_is_marked_where_it_differs() {
        let consts = |g: &Got| of(g, |o| matches!(o, Op::ConstF32(_)));
        let g = vs("varying highp float v;\nvoid main(){ mediump float m = a; v = m / 1.3; gl_Position = p; }");
        assert_eq!(alu(&g, AluOp::Fmul), [M]);
        assert_eq!(consts(&g), [M, Prec::Fold32], "1.3 itself, then its reciprocal");
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

    /// Not an int GL type (int, uint and their vectors).
    fn float_gl_type(t: u16) -> bool {
        !matches!(t, 0x1404 | 0x1405 | 0x8B53..=0x8B55 | 0x8DC6..=0x8DC8)
    }

    /// The value `v` is a float or a bool by what loads or selects it.
    fn float_value(m: &Module, v: ValueId, depth: u32) -> bool {
        use super::super::ir::{IoSlot, ScalarType};
        let row = |dw: u16| m.uniforms.iter().find(|u| dw >= u.dword_off && dw < u.dword_off + u.dwords.max(1));
        match m.instrs.get(v.0 as usize) {
            Some(Op::LoadLocal { local, .. }) => m
                .locals
                .get(local.0 as usize)
                .is_some_and(|d| d.ty.scalar != ScalarType::I32 && d.ty.scalar != ScalarType::U32),
            Some(Op::LoadUniform { dword_off, .. }) => row(*dword_off).is_some_and(|u| float_gl_type(u.gl_type)),
            Some(Op::LoadUniformIdx { base_dw, .. }) => row(*base_dw).is_some_and(|u| float_gl_type(u.gl_type)),
            Some(Op::LoadInput { slot, .. }) => match slot {
                IoSlot::Attrib(_) | IoSlot::Varying(_) => {
                    m.inputs.iter().find(|d| d.slot() == *slot).is_some_and(|d| float_gl_type(d.gl_type))
                }
                IoSlot::VertexId | IoSlot::InstanceId => false,
                _ => true,
            },
            Some(Op::Alu(AluOp::Bcsel, [_, x, _])) if depth < 64 => float_value(m, x.val, depth + 1),
            _ => false,
        }
    }

    /// Over the tree's shaders: the tables are in step, no instruction is
    /// Unknown (an expression the scan did not reach), and no value that is
    /// a float by its operation records None (the review's light_amount
    /// struct copies recorded None for 144 selects and loads).
    #[test]
    fn every_float_value_of_the_corpus_has_a_class() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for d in ["tools/shaders", "tools/mali/testdata", "tests/fixtures", "tools/cts/testdata"] {
            sources(&root.join(d), &mut files);
        }
        let mut buf = std::vec![0u8; 1 << 23];
        let (mut compiled, mut bad) = (0usize, Vec::new());
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
            for (i, op) in m.instrs.iter().enumerate() {
                let p = m.prec_of(ValueId(i as u32));
                let float = match *op {
                    Op::ConstF32(_) | Op::ConstBool(_) => true,
                    Op::Alu(a, _) => matches!(
                        a,
                        AluOp::Fadd
                            | AluOp::Fmul
                            | AluOp::Ffma
                            | AluOp::Fmin
                            | AluOp::Fmax
                            | AluOp::Fneg
                            | AluOp::Fabs
                            | AluOp::Fsign
                            | AluOp::Frcp
                            | AluOp::Frsq
                            | AluOp::Fsqrt
                            | AluOp::Fexp2
                            | AluOp::Flog2
                            | AluOp::Fsin
                            | AluOp::Fcos
                            | AluOp::Ffloor
                            | AluOp::Fceil
                            | AluOp::Ftrunc
                            | AluOp::Ffract
                            | AluOp::Fdot2
                            | AluOp::Fdot3
                            | AluOp::Fdot4
                            | AluOp::Flt
                            | AluOp::Fle
                            | AluOp::Feq
                            | AluOp::Fne
                            | AluOp::B2f
                            | AluOp::I2f
                            | AluOp::U2f
                    ),
                    Op::Tex { ret: super::super::ir::ScalarType::F32, .. } => true,
                    _ => float_value(m, ValueId(i as u32), 0),
                };
                if p == Prec::Unknown || (float && p == Prec::None) {
                    bad.push(std::format!("{}: %{i} {p:?} {op:?}", f.display()));
                }
            }
        }
        std::println!("{compiled} of {} shaders compiled", files.len());
        assert!(compiled > 900, "{compiled} shaders compiled");
        assert!(bad.is_empty(), "{} values with no class:\n{}", bad.len(), bad[..bad.len().min(20)].join("\n"));
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
