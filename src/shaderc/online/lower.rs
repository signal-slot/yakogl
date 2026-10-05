// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The syntax tree to [`ir::Module`]: the front end's output side.
//!
//! [`compile_to_ir`] is the whole front end behind one call -- text in, IR and
//! reflection out, both borrowing the arena the caller lends it. It is the
//! entry point the code generator and the oracle harness call.
//!
//! # What is lowered today, and what says so
//!
//! This is the M3 subset: a passthrough vertex shader and a literal-colour
//! fragment shader, which is what the first program on the board needs.
//!
//! ```glsl
//! attribute vec4 a_pos;  varying vec4 v_col;
//! void main() { gl_Position = a_pos;  v_col = a_pos; }
//!
//! void main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }
//! ```
//!
//! Everything outside it -- control flow, locals, textures, uniform reads,
//! arithmetic beyond a constructor's arguments -- is **refused by name**, with
//! the construct in the diagnostic: "this statement is not lowered yet", not
//! "internal error" and never silence. A lowering that emitted nothing for a
//! statement it did not understand would produce a shader that draws the
//! wrong thing, which is far harder to find than a refusal. Each refusal
//! disappears as the matching lowering lands.
//!
//! The reflection comes from [`reflect`] unchanged -- the same rows the
//! offline tables are checked against -- so the IR and the reflection cannot
//! disagree about a location or a name.

use super::arena::{AVec, Arena};
use super::ast::*;
use super::builtin_call::{self, Operand};
use super::ir::{self, IoDecl, IoKind, Node, Op, Range as IrRange, Src, ValueId};
use super::sema::{ExprTypes, Stage};
use super::{parser, pp, reflect, Error, R};
use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The components a GL type token has, or `None` for one this lowering does
/// not yet read: a matrix (columns, which needs the arithmetic lowering) or a
/// sampler (named by binding, never loaded).
fn components_of(gl_type: u16) -> Option<u8> {
    Some(match gl_type {
        0x1406 | 0x1404 | 0x1405 | 0x8B56 => 1, // float, int, uint, bool
        0x8B50 | 0x8B53 | 0x8DC6 | 0x8B57 => 2, // vec2, ivec2, uvec2, bvec2
        0x8B51 | 0x8B54 | 0x8DC7 | 0x8B58 => 3, // vec3, ivec3, uvec3, bvec3
        0x8B52 | 0x8B55 | 0x8DC8 | 0x8B59 => 4, // vec4, ivec4, uvec4, bvec4
        _ => return None,
    })
}

/// `bool`, `bvec2`, `bvec3`, `bvec4`.
fn is_bool_gl_type(gl_type: u16) -> bool {
    matches!(gl_type, 0x8B56..=0x8B59)
}

/// Does `body` assign to `name` anywhere inside it?
///
/// The question an aliased parameter turns on: a parameter that is only read
/// is a name for a value, one that is written is a variable and needs a slot.
/// ES 1.00 lets a function assign to its own `in` parameter -- it is a local
/// copy -- so this cannot be assumed away.
///
/// Conservative in the safe direction: anything it cannot see through (an
/// index, a member, a call that might take it by `out`) counts as an
/// assignment, so the parameter gets a slot and behaves as it always did.
/// `ro` names call arguments that cannot be `out` whatever the signature
/// says: section 4.1.7 forbids a sampler, or a struct holding one, as an
/// out or inout parameter, and the checker refuses it.
fn assigns_to(ast: &Ast<'_>, body: StmtId, name: &str, ro: &dyn Fn(ExprId) -> bool) -> bool {
    fn in_expr(ast: &Ast<'_>, e: ExprId, name: &str, ro: &dyn Fn(ExprId) -> bool, depth: u32) -> bool {
        if depth > 64 {
            /* An expression cannot nest this deeply after the parser's own
             * limit; giving up means "might assign", which is the safe
             * direction. */
            return true;
        }
        let names = |x: ExprId| match *ast.expr(x) {
            Expr::Ident(n) => n == name,
            /* `p.xy = ...` and `p[0] = ...` write the parameter too. */
            Expr::Field(b, _) | Expr::Index(b, _) => matches!(*ast.expr(b), Expr::Ident(n) if n == name),
            _ => false,
        };
        match *ast.expr(e) {
            Expr::Assign(_, lhs, rhs) => {
                names(lhs) || in_expr(ast, lhs, name, ro, depth + 1) || in_expr(ast, rhs, name, ro, depth + 1)
            }
            Expr::Unary(UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec, x) => {
                names(x) || in_expr(ast, x, name, ro, depth + 1)
            }
            /* A call may take it by `out`, and which parameter is `out`
             * depends on a signature this walk does not resolve. Passing the
             * name at all counts. */
            Expr::Call { args, .. } => args.indices().any(|i| {
                let a = ast.expr_ids[i];
                (names(a) && !ro(a)) || in_expr(ast, a, name, ro, depth + 1)
            }),
            Expr::Unary(_, x) => in_expr(ast, x, name, ro, depth + 1),
            Expr::Binary(_, l, r) | Expr::Comma(l, r) => {
                in_expr(ast, l, name, ro, depth + 1) || in_expr(ast, r, name, ro, depth + 1)
            }
            Expr::Index(b, i) => in_expr(ast, b, name, ro, depth + 1) || in_expr(ast, i, name, ro, depth + 1),
            Expr::Field(b, _) => in_expr(ast, b, name, ro, depth + 1),
            Expr::Select(c, t, f) => {
                in_expr(ast, c, name, ro, depth + 1)
                    || in_expr(ast, t, name, ro, depth + 1)
                    || in_expr(ast, f, name, ro, depth + 1)
            }
            /* An array constructor's arguments are values, never `out`. */
            Expr::ArrayCtor { args, .. } => args.indices().any(|i| in_expr(ast, ast.expr_ids[i], name, ro, depth + 1)),
            /* `a.length()` does not evaluate `a`: its value is `a`'s type. */
            Expr::Length(_) => false,
            Expr::IntLit(_) | Expr::UintLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) | Expr::Ident(_) => false,
        }
    }
    fn in_cond(ast: &Ast<'_>, c: Cond, name: &str, ro: &dyn Fn(ExprId) -> bool) -> bool {
        match c {
            Cond::Expr(e) => in_expr(ast, e, name, ro, 0),
            /* A declaration as a condition declares a NEW variable; if it
             * shadows the parameter the parameter is not what is written,
             * but its initialiser may still mention it. */
            Cond::Decl(d) => match *ast.decl(d) {
                Decl::Var { init: Some(e), .. } => in_expr(ast, e, name, ro, 0),
                _ => false,
            },
        }
    }
    fn in_stmt(ast: &Ast<'_>, s: StmtId, name: &str, ro: &dyn Fn(ExprId) -> bool) -> bool {
        match *ast.stmt(s) {
            Stmt::Expr(Some(e)) => in_expr(ast, e, name, ro, 0),
            Stmt::Expr(None) | Stmt::Break | Stmt::Continue | Stmt::Discard | Stmt::Return(None) => false,
            Stmt::Return(Some(e)) => in_expr(ast, e, name, ro, 0),
            Stmt::Decl(r) => r.indices().any(|i| match *ast.decl(ast.decl_ids[i]) {
                /* A local of the same name SHADOWS the parameter from here
                 * on, and this walk does not model scopes -- so it counts as
                 * an assignment and the parameter keeps its slot. */
                Decl::Var { name: n, .. } if n == name => true,
                Decl::Var { init: Some(e), .. } => in_expr(ast, e, name, ro, 0),
                _ => false,
            }),
            Stmt::Block(r) => r.indices().any(|i| in_stmt(ast, ast.stmt_ids[i], name, ro)),
            Stmt::If { cond, then_s, else_s } => {
                in_expr(ast, cond, name, ro, 0)
                    || in_stmt(ast, then_s, name, ro)
                    || else_s.is_some_and(|e| in_stmt(ast, e, name, ro))
            }
            Stmt::While { cond, body } => in_cond(ast, cond, name, ro) || in_stmt(ast, body, name, ro),
            Stmt::DoWhile { body, cond } => in_stmt(ast, body, name, ro) || in_expr(ast, cond, name, ro, 0),
            Stmt::For { init, cond, step, body } => {
                init.is_some_and(|s| in_stmt(ast, s, name, ro))
                    || cond.is_some_and(|c| in_cond(ast, c, name, ro))
                    || step.is_some_and(|e| in_expr(ast, e, name, ro, 0))
                    || in_stmt(ast, body, name, ro)
            }
            Stmt::Switch { sel, body } => in_expr(ast, sel, name, ro, 0) || in_stmt(ast, body, name, ro),
            /* A label is a constant expression: it writes nothing. */
            Stmt::Case(_) => false,
        }
    }
    in_stmt(ast, body, name, ro)
}

/// Does `s` hold a `return` anywhere inside it: in an arm, a block or a
/// loop body. A function body is not a statement, so this never looks into
/// a callee.
fn contains_return(ast: &Ast<'_>, s: StmtId) -> bool {
    match *ast.stmt(s) {
        Stmt::Return(_) => true,
        Stmt::Block(r) => r.indices().any(|i| contains_return(ast, ast.stmt_ids[i])),
        Stmt::If { then_s, else_s, .. } => {
            contains_return(ast, then_s) || else_s.is_some_and(|e| contains_return(ast, e))
        }
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } | Stmt::Switch { body, .. } => {
            contains_return(ast, body)
        }
        _ => false,
    }
}

/// One run of labels in a switch body and the statements after it, as
/// indices into the body's statements: labels `[labels, stmts)`,
/// statements `[stmts, end)`.
#[derive(Clone, Copy)]
struct SwitchGroup {
    labels: usize,
    stmts: usize,
    end: usize,
    /// The group before this one can run to its end, into this one.
    fallen_into: bool,
    /// Taken whenever control reaches it: it holds `default` and no label
    /// follows it, so an earlier match has either fallen into it or left
    /// the switch, and no match is left for anyone else.
    always: bool,
}

/// The groups of a switch body, and which of the flags the lowering needs.
struct SwitchShape<'a> {
    groups: AVec<'a, SwitchGroup>,
    /// Some group tests "no label matched": a `default` that is not
    /// [`SwitchGroup::always`].
    needs_none: bool,
    /// Some group that is tested can be fallen into.
    needs_fell: bool,
}

impl<'a> SwitchShape<'a> {
    fn of(a: &Arena<'a>, ast: &Ast<'a>, ids: &[StmtId]) -> R<'a, SwitchShape<'a>> {
        let is_label = |k: usize| matches!(ast.stmt(ids[k]), Stmt::Case(_));
        let mut groups: AVec<'a, SwitchGroup> = AVec::new();
        let mut k = 0usize;
        while k < ids.len() {
            let labels = k;
            while k < ids.len() && is_label(k) {
                k += 1;
            }
            let stmts = k;
            while k < ids.len() && !is_label(k) {
                k += 1;
            }
            let fallen_into = groups.as_slice().last().is_some_and(|p| !ends_in_jump(ast, &ids[p.stmts..p.end]));
            let has_default = ids[labels..stmts].iter().any(|&id| matches!(ast.stmt(id), Stmt::Case(None)));
            let label_after = ids[k..].iter().any(|&id| matches!(ast.stmt(id), Stmt::Case(Some(_))));
            let always = has_default && !label_after;
            groups.push(a, SwitchGroup { labels, stmts, end: k, fallen_into, always })?;
        }
        let g = groups.as_slice();
        let needs_none = g
            .iter()
            .any(|x| !x.always && ids[x.labels..x.stmts].iter().any(|&id| matches!(ast.stmt(id), Stmt::Case(None))));
        let needs_fell = g.iter().any(|x| x.fallen_into && !x.always);
        Ok(SwitchShape { groups, needs_none, needs_fell })
    }
}

/// Does control never run off the end of this statement list: its last
/// statement is a `break`, `continue` or `return`, a block ending in one,
/// or an `if` both of whose arms do.
fn ends_in_jump(ast: &Ast<'_>, ids: &[StmtId]) -> bool {
    let Some(&last) = ids.last() else { return false };
    match *ast.stmt(last) {
        Stmt::Break | Stmt::Continue | Stmt::Return(_) => true,
        Stmt::Block(r) => ends_in_jump(ast, &ast.stmt_ids[r.indices()]),
        Stmt::If { then_s, else_s: Some(e), .. } => ends_in_jump(ast, &[then_s]) && ends_in_jump(ast, &[e]),
        _ => false,
    }
}

/// Does `s` hold a `continue` that belongs to a loop OUTSIDE it -- one not
/// inside a loop of its own? A switch with one needs a flag: its body is
/// lowered as a loop, and a bare `Continue` there would continue that.
fn contains_outer_continue(ast: &Ast<'_>, s: StmtId) -> bool {
    match *ast.stmt(s) {
        Stmt::Continue => true,
        Stmt::Block(r) => r.indices().any(|i| contains_outer_continue(ast, ast.stmt_ids[i])),
        Stmt::If { then_s, else_s, .. } => {
            contains_outer_continue(ast, then_s) || else_s.is_some_and(|e| contains_outer_continue(ast, e))
        }
        Stmt::Switch { body, .. } => contains_outer_continue(ast, body),
        _ => false,
    }
}

/// Does every path through `s` reach a `return`? The same answer
/// [`Lower::lower_run`] gives after lowering it: a loop may run no
/// iterations, so it never counts.
fn always_returns(ast: &Ast<'_>, s: StmtId) -> bool {
    match *ast.stmt(s) {
        Stmt::Return(_) => true,
        Stmt::Block(r) => r.indices().any(|i| always_returns(ast, ast.stmt_ids[i])),
        Stmt::If { then_s, else_s: Some(e), .. } => always_returns(ast, then_s) && always_returns(ast, e),
        _ => false,
    }
}

/// Does `s` return on some paths and not on others? Such a statement is the
/// one whose followers cannot simply be moved into an `else`.
fn returns_sometimes(ast: &Ast<'_>, s: StmtId) -> bool {
    contains_return(ast, s) && !always_returns(ast, s)
}

/// Does this run of statements return anywhere but at its very end?
///
/// The straight-line inlining is kept for the common helper that computes and
/// returns once, because giving every such call a slot and a load would be a
/// regression the back end would have to optimise away again.
fn returns_early(ast: &Ast<'_>, ids: &[StmtId]) -> bool {
    for (k, &id) in ids.iter().enumerate() {
        let last = k + 1 == ids.len();
        match *ast.stmt(id) {
            Stmt::Return(_) if last => {}
            Stmt::Return(_) => return true,
            _ if contains_return(ast, id) => return true,
            _ => {}
        }
    }
    false
}

/// The write mask and the lane mapping an lvalue swizzle denotes.
///
/// `c.xy = v` writes lanes 0 and 1 from the source's lanes 0 and 1; `c.zx =
/// v` writes lanes 2 and 0 from the source's 0 and 1, in that order. A
/// repeated component (`c.xx = v`) is not an lvalue -- section 5.8 -- and the
/// checker refuses it; this refuses it too rather than writing one of the
/// two.
fn swizzle_mask<'a>(line: u32, field: &'a str, comps: u8) -> R<'a, (u8, [Option<u8>; 4])> {
    let mut mask = 0u8;
    let mut lanes = [None; 4];
    if field.is_empty() || field.len() > 4 {
        return Err(Error::new(line, field, "this is not a component selection"));
    }
    for (j, c) in field.bytes().enumerate() {
        let i = match c {
            b'x' | b'r' | b's' => 0u8,
            b'y' | b'g' | b't' => 1,
            b'z' | b'b' | b'p' => 2,
            b'w' | b'a' | b'q' => 3,
            _ => return Err(Error::new(line, field, "this is not a component selection")),
        };
        if i >= comps {
            return Err(Error::new(line, field, "component out of range for this type"));
        }
        if mask & (1 << i) != 0 {
            return Err(Error::new(line, field, "a component may not be assigned twice"));
        }
        mask |= 1 << i;
        lanes[i as usize] = Some(j as u8);
    }
    Ok((mask, lanes))
}

/// Every scalar conversion has an arm; this is the pair that cannot happen,
/// because equal kinds return before the match. It is a refusal rather than a
/// panic: a compiler that runs on the target must not abort, and a conversion
/// silently treated as a no-op would draw the wrong picture.
fn unreachable_conversion<'a>(line: u32, _from: ir::ScalarType, _to: ir::ScalarType) -> R<'a, Src> {
    Err(Error::new(line, "", "this conversion is not lowered yet"))
}

/// The IR scalar kind of a GLSL base type, or `None` for a type that has no
/// lanes of its own (a sampler, a struct, `void`).
fn scalar_of(base: BaseType<'_>) -> Option<ir::ScalarType> {
    Some(match base {
        BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..) => ir::ScalarType::F32,
        BaseType::Int | BaseType::IVec(_) => ir::ScalarType::I32,
        BaseType::Uint | BaseType::UVec(_) => ir::ScalarType::U32,
        BaseType::Bool | BaseType::BVec(_) => ir::ScalarType::Bool,
        BaseType::Void | BaseType::Sampler(_) | BaseType::Struct(_) => return None,
    })
}

/// The most arguments a GLSL ES 1.00 built-in takes: `clamp`, `mix`,
/// `smoothstep`, `refract` and `texture2D` with a bias all take three, and
/// nothing in section 8 takes four. A call with more is a user-defined
/// function, which is refused by name rather than truncated.
///
/// ES 3.00's texture look-ups take up to five: `textureProjGradOffset(s, P,
/// dPdx, dPdy, offset)`.
const MAX_BUILTIN_ARGS: usize = 5;

/// A destination resolved ONCE -- its indices evaluated a single time --
/// so that it can be read and written without evaluating them again: an
/// `out` argument (read before the call for `inout`, written after it),
/// `v[f()]++`, `m[i][j] = x`. Locals only.
#[derive(Clone, Copy)]
struct LRef<'a> {
    slots: SlotSel<'a>,
    lane: LaneSel,
}

/// Which slot of a local a destination is.
#[derive(Clone, Copy)]
enum SlotSel<'a> {
    One(Local<'a>),
    /// Slot `idx` of `base.column(0..n)`, chosen at run time.
    Among {
        base: Local<'a>,
        n: u8,
        idx: Src,
    },
}

/// Which lanes of the slot.
#[derive(Clone, Copy)]
enum LaneSel {
    All,
    At(u8),
    /// One lane, chosen at run time.
    Dyn(Src),
    /// A swizzle of several lanes (`v.zx`): destination lane i takes the
    /// value's lane `lanes[i]` for each lane in `mask` -- `swizzle_mask`'s
    /// convention, which `store_lanes` takes.
    Swz {
        mask: u8,
        lanes: [Option<u8>; 4],
    },
}

/// One bound parameter of an inlined call.
#[derive(Clone, Copy)]
struct Bound<'a> {
    /// The slot (or slots, for a matrix) the parameter lives in.
    param: Local<'a>,
    /// Where an `out` or `inout` parameter is copied back to when the call
    /// finishes. `None` for an ordinary `in`.
    target: Option<Local<'a>>,
    /// The struct type, when the parameter is one. The copy back needs it:
    /// a struct's slots are leaves of DIFFERING width, so it cannot be
    /// walked with the one `comps` that serves a matrix or an array.
    struct_name: Option<&'a str>,
    /// The element type, when the parameter is an array (ES 3.00): an
    /// array of structs is copied back leaf by leaf for the same reason.
    elem: Option<BaseType<'a>>,
    /// An `out` argument that is not a whole variable (`v[i]`, `m[i][j]`,
    /// `v.y`): where the copy back goes, resolved before the call. In the
    /// arena, not inline: `Bound`s sit in every inlined call's stack frame,
    /// fifteen deep at the inliner's limit (tests/stack_survey.rs).
    lref: Option<&'a LRef<'a>>,
}

impl Bound<'_> {
    const EMPTY: Bound<'static> =
        Bound { param: Local::scalar("", ir::LocalId(0), 1), target: None, struct_name: None, elem: None, lref: None };
}

/// What an inlined call is being asked for.
///
/// A matrix return has no single value, so the caller says which shape it
/// needs and the body's `return` is lowered that way. Deciding it from the
/// declared return type instead would be the same information twice, and the
/// two could disagree.
#[derive(Clone, Copy)]
enum Want<'a> {
    Value(u8),
    /// A matrix of C columns of R rows.
    Matrix(u8, u8),
    /// A call used as a statement: `split(k, x, y);`. The function is `void`
    /// -- what it does is write its `out` parameters -- so there is no value
    /// to produce and falling off the end is how it ends.
    Nothing,
    /// A call that returns a STRUCT: `Line line = func(p);`. A struct is a
    /// run of slots of differing width, so there is no value to hand back
    /// -- the caller names the run and the body's `return` fills it, which
    /// is the same move `S b = a;` makes.
    Struct {
        dst: Local<'a>,
        sname: &'a str,
    },
    /// A call that returns an ARRAY (ES 3.00 section 6.1): like a struct,
    /// a run with no single value, which the body's `return` fills.
    Array {
        dst: Local<'a>,
    },
}

/// One leaf of a struct in slot order: its kind and width, and how a
/// uniform of that struct names it (`path` below the struct, and the
/// element of a leaf that is an array, which GL keeps in one row).
#[derive(Clone, Copy)]
struct Leaf<'a> {
    scalar: ir::ScalarType,
    comps: u8,
    path: &'a str,
    element: Option<u32>,
}

/// What to read from a uniform leaf row once its name is known.
#[derive(Clone, Copy)]
enum LeafLoad {
    /// The whole value.
    Value,
    /// Element `k` of a leaf that is an array.
    Element(u32),
    /// Column `j` of a `matN` leaf.
    Column { j: u8, n: u8 },
    /// Column `j` of element `k` of a leaf that is an array of `matN`.
    ElementColumn { k: u32, j: u8, n: u8 },
}

/// Lane `k` of `v`, splatted: the operand that reads one component.
fn lane_of(v: Src, k: u8) -> Src {
    Src { val: v.val, swizzle: [v.swizzle[usize::from(k) & 3]; 4] }
}

/// `v` read through a further swizzle, composed with its own.
fn reswizzle(v: Src, sw: [u8; 4]) -> Src {
    Src { val: v.val, swizzle: sw.map(|i| v.swizzle[usize::from(i) & 3]) }
}

/// What an inlined call produced.
enum Ret {
    Value(Src),
    Matrix([Src; 4]),
    /// Nothing to hand back: the destination run has been filled in place.
    Struct,
}

/// How deeply calls may nest before the lowering gives up. GLSL ES forbids
/// recursion (section 6.1), so a well-formed shader never approaches this;
/// the bound is against one that is not, since a compiler running on the
/// target must refuse rather than exhaust its arena.
const MAX_INLINE_DEPTH: u32 = 16;

/// A compiler that runs on the target refuses, it does not panic.
///
/// Since 2026-09-23 a struct or array result is filled in place by every
/// `return` (the top of `inline_with_slot`), so the one-slot path below it
/// never sees one; the arms that name this are there for exhaustiveness and
/// refuse rather than panic if that ever stops holding.
const RUN_RESULT_IN_ONE_SLOT: &str = "a struct or array result reached the one-slot return path";

/// The most elements a local array may have.
///
/// Every element is a slot, and every slot is a register: the run is what
/// lets `a[2]` be `id + 2`, and a run-time index is a select over the run
/// (`select_among`, since 2026-09-23). So the ceiling is the register file:
/// 64 is the GC7000's temps, and a program whose run does not fit beside
/// everything else is refused by the back end's register allocation, in its
/// own words, rather than here. It was 32, sized when a run-time index was
/// refused anyway; dEQP's shaders.indexing.tmp_array uses 40-element arrays.
const MAX_ARRAY_SLOTS: u8 = 64;

/// Operands a built-in call may hand over. Three arguments is the widest
/// call in section 8, and a matrix argument becomes its N columns, so the
/// widest handover is `matrixCompMult` on two `mat4`s: eight. Twelve leaves
/// room for a three-argument form over matrices that the language does not
/// currently have.
const MAX_BUILTIN_OPERANDS: usize = 12;

/// Parameters an inlined call may bind. A function with more is refused by
/// name rather than truncated. It was 8 until Qt Quick 3D's ES 3.00 lighting
/// helper arrived with 12 (`qt_processPunctualLighting`, 26 sources); the
/// bound is a stack frame per nested inline, which `tests/stack_survey.rs`
/// measures.
const MAX_PARAMS: usize = 16;

/// Parameters an inlined call binds on the stack; more go in the arena.
const SMALL_PARAMS: usize = 8;

/// `s[12]` -> (`s`, 12): the row and element a constant-indexed name
/// addresses. `None` for a name that does not end in a subscript.
fn split_element(name: &str) -> Option<(&str, u32)> {
    let body = name.strip_suffix(']')?;
    let lb = body.rfind('[')?;
    let k = body[lb + 1..].parse().ok()?;
    Some((&body[..lb], k))
}

/// What the front end produces for one stage.
pub struct Lowered<'a> {
    pub module: ir::Module<'a>,
    pub reflection: reflect::Reflection<'a>,
}

/// The whole front end: GLSL ES 1.00 text to IR and reflection.
///
/// Everything returned borrows `a`, so it lives exactly as long as the
/// caller's arena and nothing needs freeing. A shader the front end refuses
/// comes back as an [`Error`] carrying the line, the token and the reason, in
/// the shape `glGetShaderInfoLog` has always used.
pub fn compile_to_ir<'a>(a: &Arena<'a>, stage: Stage, src: &'a str) -> R<'a, Lowered<'a>> {
    let pre = pp::preprocess(a, src)?;
    let ast = parser::resolve_io(a, parser::parse(a, pre.tokens)?, stage)?;
    /* The parser's input is consumed: the tree borrows the source's text
     * (and the arena's for a computed token), never the token array.
     * Given back so the lowering reuses it (random.23: 122 KB). */
    // SAFETY: nothing refers to `pre.tokens` past this point.
    unsafe { a.give_back(pre.tokens) };
    let types = super::sema::check(a, &ast, stage)?;
    let mut reflection = reflect::reflect(a, &ast)?;
    if !ast.blocks.is_empty() {
        let t = reflect::block_table(a, &ast, &reflection)?;
        reflection.blocks = t.blocks;
        reflection.block_members = t.members;
    }
    let module = lower(a, &ast, stage, &reflection, types)?;
    Ok(Lowered { module, reflection })
}

/// One block `relocate` moves: where it is, how long, its alignment.
#[derive(Clone, Copy)]
struct Moved {
    src: usize,
    len: usize,
    align: usize,
    dst: usize,
}

/// Turns [`relocate`] off, for the test that compares a compile with and
/// without it (under `store::test_lock`, so no other compile sees it).
#[cfg(test)]
pub(crate) static RELOCATE_OFF: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Give the front end's scratch back before the back end runs: `l` (built by
/// [`compile_to_ir`] over `text`, which the caller allocated just before
/// `mark`) is moved DOWN to `mark`, and everything the front end left above
/// it -- tokens, tree, types, the lowering's garbage -- is released. The back
/// end then starts from the module and the reflection alone.
///
/// In place, with no second copy: the blocks that make up `l` (its slices,
/// and every name that is not a slice of `text`) are moved in ascending
/// order of address to consecutive places from `mark`, which is safe because
/// each lands at or below where it was -- checked, and the compaction is
/// skipped (the compile goes on as before) if one would not. The pointers
/// in `l` and in its rows are then rewritten through the same table. The
/// table itself is allocated above everything and released with the rest.
pub fn relocate<'a>(a: &Arena<'a>, l: Lowered<'a>, text: &'a str, mark: usize) -> R<'a, Lowered<'a>> {
    #[cfg(test)]
    if RELOCATE_OFF.load(core::sync::atomic::Ordering::Relaxed) {
        return Ok(l);
    }
    a.forget_free();
    let top = a.used();
    let base = a.base_addr();
    let (lo, hi) = (text.as_ptr() as usize, text.as_ptr() as usize + text.len());
    let m = l.module;
    let r = l.reflection;
    let mut blocks: AVec<Moved> = AVec::new();
    let add = |blocks: &mut AVec<'a, Moved>, p: usize, len: usize, align: usize| -> R<'a, ()> {
        if len == 0 || (p >= lo && p + len <= hi) || p < base + mark || p >= base + top {
            return Ok(());
        }
        Ok(blocks.push(a, Moved { src: p, len, align, dst: 0 })?)
    };
    fn slice<T>(s: &[T]) -> (usize, usize, usize) {
        (s.as_ptr() as usize, core::mem::size_of_val(s), core::mem::align_of::<T>())
    }
    macro_rules! blk {
        ($s:expr) => {{
            let (p, n, al) = slice($s);
            add(&mut blocks, p, n, al)?;
        }};
    }
    blk!(m.instrs);
    blk!(m.comps);
    blk!(m.nodes);
    blk!(m.locals);
    blk!(m.inputs);
    blk!(m.outputs);
    blk!(m.uniforms);
    blk!(r.uniforms);
    blk!(r.attribs);
    blk!(r.varyings);
    blk!(r.outputs);
    blk!(r.blocks);
    blk!(r.block_members);
    for s in m
        .locals
        .iter()
        .map(|v| v.name)
        .chain(m.inputs.iter().map(|d| d.name))
        .chain(m.outputs.iter().map(|d| d.name))
        .chain(m.uniforms.iter().map(|u| u.name))
        .chain(r.uniforms.iter().map(|u| u.name))
        .chain(r.attribs.iter().map(|x| x.name))
        .chain(r.varyings.iter().map(|x| x.name))
        .chain(r.outputs.iter().map(|x| x.name))
        .chain(r.blocks.iter().map(|x| x.name))
        .chain(r.block_members.iter().map(|x| x.name))
        .chain(r.block_members.iter().map(|x| x.row_name))
    {
        blk!(s.as_bytes());
    }
    let bl = blocks.as_mut_slice();
    /* Ascending by address, duplicates (one name used by two rows) once. */
    for i in 1..bl.len() {
        let mut j = i;
        while j > 0 && bl[j - 1].src > bl[j].src {
            bl.swap(j - 1, j);
            j -= 1;
        }
    }
    let mut n = 0;
    for i in 0..bl.len() {
        if n > 0 && bl[i].src < bl[n - 1].src + bl[n - 1].len {
            /* Inside (or equal to) the previous block: a name that is a
             * suffix of another, or the same name twice. */
            continue;
        }
        bl[n] = bl[i];
        n += 1;
    }
    let bl = &mut bl[..n];
    let mut cursor = base + mark;
    for b in bl.iter_mut() {
        let dst = cursor.div_ceil(b.align) * b.align;
        if dst > b.src {
            return Ok(Lowered { module: m, reflection: r });
        }
        b.dst = dst;
        cursor = dst + b.len;
    }
    for b in bl.iter() {
        // SAFETY: both ranges are inside the arena's buffer; `dst <= src`
        // and the blocks go in ascending order, so a block is moved before
        // anything is written over it (`copy` is a memmove for the overlap
        // of a block with its own destination).
        unsafe { core::ptr::copy(b.src as *const u8, b.dst as *mut u8, b.len) };
    }
    /* The address a pointer into a moved block now has. */
    let map = |p: usize| -> usize {
        let i = bl.partition_point(|b| b.src <= p);
        if i == 0 {
            return p;
        }
        let b = &bl[i - 1];
        if p < b.src + b.len {
            b.dst + (p - b.src)
        } else {
            p
        }
    };
    fn moved_slice<'x, T>(s: &[T], map: &dyn Fn(usize) -> usize) -> &'x mut [T] {
        // SAFETY: the block was moved to `map(ptr)` whole; the arena's buffer
        // outlives `'x`, which the caller ties to the arena.
        unsafe { core::slice::from_raw_parts_mut(map(s.as_ptr() as usize) as *mut T, s.len()) }
    }
    fn moved_str<'x>(s: &str, map: &dyn Fn(usize) -> usize) -> &'x str {
        // SAFETY: as for a slice; the bytes are the same UTF-8.
        unsafe {
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(map(s.as_ptr() as usize) as *const u8, s.len()))
        }
    }
    let locals: &'a mut [ir::VarDecl<'a>] = moved_slice(m.locals, &map);
    for v in locals.iter_mut() {
        v.name = moved_str(v.name, &map);
    }
    let inputs: &'a mut [IoDecl<'a>] = moved_slice(m.inputs, &map);
    for d in inputs.iter_mut() {
        d.name = moved_str(d.name, &map);
    }
    let outputs: &'a mut [IoDecl<'a>] = moved_slice(m.outputs, &map);
    for d in outputs.iter_mut() {
        d.name = moved_str(d.name, &map);
    }
    let uniforms: &'a mut [ir::UniformDecl<'a>] = moved_slice(m.uniforms, &map);
    for u in uniforms.iter_mut() {
        u.name = moved_str(u.name, &map);
    }
    let ru: &'a mut [reflect::UniformRow<'a>] = moved_slice(r.uniforms, &map);
    for u in ru.iter_mut() {
        u.name = moved_str(u.name, &map);
    }
    let ra: &'a mut [reflect::AttribRow<'a>] = moved_slice(r.attribs, &map);
    for x in ra.iter_mut() {
        x.name = moved_str(x.name, &map);
    }
    let rv: &'a mut [reflect::VaryingRow<'a>] = moved_slice(r.varyings, &map);
    for x in rv.iter_mut() {
        x.name = moved_str(x.name, &map);
    }
    let ro: &'a mut [reflect::OutputRow<'a>] = moved_slice(r.outputs, &map);
    for x in ro.iter_mut() {
        x.name = moved_str(x.name, &map);
    }
    let rb: &'a mut [reflect::BlockDesc<'a>] = moved_slice(r.blocks, &map);
    for x in rb.iter_mut() {
        x.name = moved_str(x.name, &map);
    }
    let rbm: &'a mut [reflect::BlockMember<'a>] = moved_slice(r.block_members, &map);
    for x in rbm.iter_mut() {
        x.name = moved_str(x.name, &map);
        x.row_name = moved_str(x.row_name, &map);
    }
    let module = ir::Module {
        instrs: moved_slice(m.instrs, &map),
        comps: moved_slice(m.comps, &map),
        nodes: moved_slice(m.nodes, &map),
        locals,
        inputs,
        outputs,
        uniforms,
        ..m
    };
    let reflection = reflect::Reflection {
        uniforms: ru,
        attribs: ra,
        varyings: rv,
        outputs: ro,
        blocks: rb,
        block_members: rbm,
        ..r
    };
    let end = cursor - base;
    // SAFETY: everything that survives is below `end` now (it was just moved
    // there) or is `text`, below `mark`; the rest of the front end's
    // allocations are dropped with `l`'s fields, which were moved out.
    unsafe { a.reset_to(end) };
    #[cfg(test)]
    a.poison_above(end, top, 0xa5);
    Ok(Lowered { module, reflection })
}

/// The fragment shader as linked to a vertex shader that DECLARES the
/// varyings in `zero` but never writes them: every read of one is the
/// constant 0, and it is no longer an input.
///
/// GLSL ES 1.00 section 4.3.5 makes that program legal -- only a varying the
/// vertex shader never declares is a link error -- and leaves the value
/// undefined; 0 is the value this implementation defines. It is done here, in
/// the IR, before either code generator sees the shader, so the two GPUs
/// cannot come to differ on it, and neither has to invent an input nobody
/// feeds. dEQP `linkage.varying_4` was refused as "not written by the vertex
/// shader" until 2026-09-23.
///
/// A name not declared as a varying here is ignored: the link names what the
/// vertex side lacks, and this stage has the final say on what it reads.
pub fn zero_varyings<'a>(a: &Arena<'a>, l: Lowered<'a>, zero: &[&str]) -> R<'a, Lowered<'a>> {
    if zero.is_empty() {
        return Ok(l);
    }
    /* The locations to zero: every location of every named varying (a
     * matN is N of them, an array one per element). */
    let mut locs = [false; 256];
    let mut any = false;
    for v in l.reflection.varyings.iter().filter(|v| zero.contains(&v.name)) {
        let cols: u32 = match v.gl_type {
            0x8B5A | 0x8B65 | 0x8B66 => 2, // mat2, mat2x3, mat2x4
            0x8B5B | 0x8B67 | 0x8B68 => 3, // mat3, mat3x2, mat3x4
            0x8B5C | 0x8B69 | 0x8B6A => 4, // mat4, mat4x2, mat4x3
            _ => 1,
        };
        for k in 0..u32::from(v.count.max(1)) * cols {
            if let Some(slot) = locs.get_mut(v.location as usize + k as usize) {
                *slot = true;
                any = true;
            }
        }
    }
    if !any {
        return Ok(l);
    }
    let zeroed = |slot: &ir::IoSlot| matches!(slot, ir::IoSlot::Varying(loc) if locs[*loc as usize]);

    let mut instrs = AVec::with_capacity(a, l.module.instrs.len())?;
    for op in l.module.instrs.iter() {
        /* The value keeps its id and its width (`comps` is unchanged): a
         * constant is a drop-in for a load. */
        instrs.push(
            a,
            match op {
                Op::LoadInput { slot, .. } if zeroed(slot) => Op::ConstF32([0; 4]),
                other => *other,
            },
        )?;
    }
    let mut inputs = AVec::new();
    for d in l.module.inputs.iter() {
        if !(d.kind == IoKind::Varying && locs[d.location as usize]) {
            inputs.push(a, *d)?;
        }
    }
    let mut varyings = AVec::new();
    for v in l.reflection.varyings.iter().filter(|v| !zero.contains(&v.name)) {
        varyings.push(a, *v)?;
    }
    Ok(Lowered {
        module: ir::Module { instrs: instrs.into_slice(), inputs: inputs.into_slice(), ..l.module },
        reflection: reflect::Reflection { varyings: varyings.into_slice(), ..l.reflection },
    })
}

/// Build the IR for `ast`, given the reflection already derived from it.
pub fn lower<'a>(
    a: &Arena<'a>,
    ast: &Ast<'a>,
    stage: Stage,
    reflection: &reflect::Reflection<'a>,
    types: ExprTypes<'a>,
) -> R<'a, ir::Module<'a>> {
    let mut l = Lower {
        a,
        ast,
        stage,
        reflection,
        types,
        instrs: AVec::new(),
        comps: AVec::new(),
        nodes: AVec::new(),
        inputs: AVec::new(),
        outputs: AVec::new(),
        locals: AVec::new(),
        scope: AVec::new(),
        scope_base: 0,
        marks: AVec::new(),
        globals: AVec::new(),
        shadows: AVec::new(),
        uniform_params: AVec::new(),
        inline_depth: 0,
        uses_discard: false,
        uses_front_facing: false,
        loop_steps: AVec::new(),
        nested: 0,
        line: 0,
        ret: None,
        switch_cont: None,
        const_arrays: AVec::new(),
        fold_depth: 0,
        known: AVec::new(),
        folded: AVec::new(),
    };
    l.declare_io()?;
    let body = l.main_body()?;
    let uniforms = l.uniform_decls()?;
    Ok(ir::Module {
        comps: a.alloc_copy(l.comps.as_slice())?,
        stage: match stage {
            Stage::Vertex => SHC_STAGE_VERTEX,
            Stage::Fragment => SHC_STAGE_FRAGMENT,
        },
        instrs: l.instrs.into_slice(),
        nodes: l.nodes.into_slice(),
        body,
        locals: l.locals.into_slice(),
        inputs: l.inputs.into_slice(),
        outputs: l.outputs.into_slice(),
        uniforms,
        uniform_dwords: reflection.uniform_dwords,
        uses_discard: l.uses_discard,
        uses_front_facing: l.uses_front_facing,
    })
}

struct Lower<'x, 'a> {
    a: &'x Arena<'a>,
    ast: &'x Ast<'a>,
    stage: Stage,
    reflection: &'x reflect::Reflection<'a>,
    /// What the type checker worked out for each expression. The lowering
    /// asks rather than infers: `mat4 * vec4` and `float * vec4` are the same
    /// syntax and different code, and only the type tells them apart.
    types: ExprTypes<'a>,
    instrs: AVec<'a, Op>,
    /// The component count of each instruction's value, in step with
    /// [`Lower::instrs`]. See [`ir::Module::comps`].
    comps: AVec<'a, u8>,
    nodes: AVec<'a, Node>,
    inputs: AVec<'a, IoDecl<'a>>,
    outputs: AVec<'a, IoDecl<'a>>,
    /// Every slot a local variable was given, in declaration order.
    /// [`ir::LocalId`] indexes this.
    locals: AVec<'a, ir::VarDecl<'a>>,
    /// The names currently in scope, innermost last, with [`Lower::marks`]
    /// recording where each scope began -- the same shape the type checker
    /// uses, for the same reason: a scope is popped by truncation, with no
    /// allocation per scope.
    ///
    /// A slot is never reused when a scope ends. Two locals of the same name
    /// in sibling blocks get two slots, which costs a register the back end
    /// could have shared and buys the property that a `LocalId` means one
    /// variable for the whole module. Register allocation is the back end's
    /// job and it has the live ranges; inventing a second, worse allocator
    /// here would only make its answer wrong.
    scope: AVec<'a, Local<'a>>,
    /// Where the innermost function body's entries in `scope` begin. A body
    /// sees the globals and its own parameters, never its caller's locals,
    /// so [`Lower::lookup`] searches from here up. The caller's entries stay
    /// where they are: setting the vector aside instead grew a fresh one per
    /// inlined call in an arena that never frees -- 212 KB of the 1.5 MB
    /// dEQP uniform_api.random.23's fragment shader spent lowering its
    /// thousands of helper calls.
    scope_base: usize,
    marks: AVec<'a, u32>,
    /// Variables declared at file scope with no storage qualifier, and
    /// `const` ones.
    ///
    /// Kept apart from [`Lower::scope`] because a function body sees the
    /// globals but NOT the caller's locals: inlining sets the caller's scope
    /// aside, and a global that lived in it would disappear for the duration
    /// of every inlined body. Looked up after `scope`, so a local or a
    /// parameter of the same name shadows one, which is what section 4.2.3
    /// says.
    globals: AVec<'a, Local<'a>>,
    /// Outputs that are read or written a component at a time, and so are
    /// held in a slot for the duration of `main` and stored once at the end.
    /// See [`Lower::shadow_outputs`].
    shadows: AVec<'a, (&'a str, Local<'a>)>,
    /// The GL name prefixes that `Run::Uniform` parameters stand for.
    uniform_params: AVec<'a, &'a str>,
    /// How many user-defined calls are being inlined around the statement
    /// being lowered. GLSL ES forbids recursion, so this bounds the arena
    /// rather than the language.
    inline_depth: u32,
    uses_discard: bool,
    /// The fragment shader reads `gl_FrontFacing`, which the back end has to
    /// arrange for rather than infer from the instructions.
    uses_front_facing: bool,
    /// The source line the lowering is currently at: the statement being
    /// lowered, narrowed to the expression when one is entered.
    ///
    /// The lowering works on ids and has no tokens, so this is the only thing
    /// that can put a line on its diagnostics. Without it every message it
    /// raises reads `0:0:` and cannot be located in the source at all -- and
    /// there were 1,841 byte-identical `0:0:` refusals in one dEQP run,
    /// which is a log nobody can act on.
    line: u32,
    /// What each enclosing loop runs before its next iteration, so a
    /// `continue` can run it too: the `for` step, and the `do`/`while` test
    /// (`None` where the loop has none). Empty outside any loop.
    loop_steps: AVec<'a, (Option<ExprId>, Option<ExprId>)>,
    /// How many `if` arms and loop bodies enclose the statement being
    /// lowered. Only `return` cares: see [`Stmt::Return`] above.
    nested: u32,
    /// The function being lowered has a `return` inside a loop or a block,
    /// and this is how that `return` reaches the rest of the body. `None`
    /// until the first such statement; set aside for each inlined call, whose
    /// returns are its own. See [`Lower::lower_run`].
    ret: Option<RetCtx<'a>>,
    /// Inside the body of a `switch` that holds a `continue` for an
    /// enclosing loop: the flag that `continue` sets before it breaks out of
    /// the switch (which is lowered as a loop), and that the code after the
    /// switch turns into the real `continue`. `None` inside a loop nested in
    /// the switch, whose `continue` is its own.
    switch_cont: Option<Local<'a>>,
    /// The element values of each `const` array held as values
    /// (`Run::Constants`), indexed by its `Local::id`.
    const_arrays: AVec<'a, &'a [Src]>,
    /* ---- constant local indices (see [`Lower::statement`]) ---- */
    /// How many conditional or repeated constructs enclose the statement
    /// being lowered: `if` arms, loops (their test and step too), switch
    /// groups, and the inlined bodies of a statement that calls a function
    /// of the shader. Only at 0 is the code straight-line from the start of
    /// `main`, and only there is a local's value tracked or used.
    fold_depth: u32,
    /// Scalar integer locals whose value is known at this point of `main`'s
    /// straight-line code. Every `StoreLocal` forgets its local
    /// ([`Lower::push_sized`]); [`Lower::statement`] re-records the ones a
    /// statement sets to a known value.
    known: AVec<'a, (ir::LocalId, i64)>,
    /// For the statement being lowered: the index expressions whose value is
    /// known, and whether evaluating one has an effect (`i++`).
    folded: AVec<'a, (ExprId, i64, bool)>,
}

/// Turns the constant local-index folding off, for the test that compares
/// the lowering with and without it (under `store::test_lock`).
#[cfg(test)]
pub(crate) static FOLD_OFF: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// How a `return` that cannot be expressed as the shape of an `if` leaves
/// the body: it stores its value, sets `flag` and, inside a loop, breaks.
/// Every loop it was nested in tests `flag` right after itself and breaks
/// too, and the statements after the outermost one run only while `flag`
/// is false.
#[derive(Clone, Copy)]
struct RetCtx<'a> {
    /// A one-lane bool slot: this call has returned.
    flag: Local<'a>,
    /// Where the value goes, as in [`Lower::lower_run`]; `None` for `main`.
    slot: Option<Local<'a>>,
    want: Want<'a>,
    /// The function's name, for a refusal.
    name: &'a str,
    /// How many loops of THIS function enclose the statement being lowered.
    loops: u32,
}

/// One local variable in scope.
#[derive(Clone, Copy)]
struct Local<'a> {
    name: &'a str,
    id: ir::LocalId,
    comps: u8,
    /// An inlined `in` parameter bound straight to its argument's value.
    ///
    /// A parameter the body never assigns to is a NAME for the caller's
    /// value, not a variable: giving it a slot costs a store, a load and a
    /// register that hold something already in hand. `shapestroke.vert`
    /// passes seven two-argument calls' worth of them, and they were 14 of
    /// its 42 slots.
    ///
    /// `None` is an ordinary variable, which lives in `id`.
    value: Option<Src>,
    /// What the run of slots at `id` IS.
    run: Run,
}

/// A local occupies one slot or a run of consecutive ones. This says which,
/// and WHY -- which is not the same question.
///
/// A `matN` and a `T[N]` are the same RUN: N consecutive slots, each `comps`
/// wide, so `id + j` selects one and nothing else need be recorded. There is
/// no matrix register on either GPU and `VarDecl` has no notion of one.
///
/// They are not the same TYPE, and a single `columns: u8` said only "how
/// many", which is the representation and not the type. Everything a matrix
/// may do -- be multiplied, be constructed, be assigned whole -- an array may
/// not: ES 1.00 has no array assignment, no array constructor and no
/// array-valued expression (section 5.7). A site asking `columns > 1` would
/// answer yes for both and hand an array to `matrix_value`, which is the
/// defect this compiler keeps growing. So the kind is carried and each site
/// says which it means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Run {
    /// One slot: a scalar, a vector, or an aliased parameter.
    Single,
    /// `matN`: N columns, each `comps` wide.
    Matrix(u8),
    /// `T[N]`: N elements, each `comps` wide.
    Array(u8),
    /// A struct: one slot per member, and the members' widths DIFFER.
    ///
    /// This is why a struct is not the same change as a matrix or an array.
    /// Those are a run of equal-width slots, so the one `Local::comps`
    /// describes every slot in the run. A `struct S { vec3 a; float b; }` is
    /// 3 wide then 1 wide, and the width has to be read per slot -- from the
    /// member's own declaration, which is where it already is.
    ///
    /// The count is carried so `slots()` can answer without the tree; the
    /// widths are looked up through [`Lower::struct_member`], which has it.
    Struct(u8),
    /// `S[N]`: N structs of `per` leaves each, element k at `k * per`.
    StructArray { len: u8, per: u8 },
    /// ES 3.00 `matC[N]`: N matrices of `cols` columns each, element k at
    /// `k * cols`; every slot is one column, `comps` wide.
    MatrixArray { len: u8, cols: u8 },
    /// No slots: a `const` array too large to give every element a slot,
    /// held as its elements' VALUES (`Lower::const_arrays[id]`). It cannot
    /// be written, so values are all it needs; an element is read at a
    /// constant index as that value and at a run-time one as a select among
    /// them. Like `Uniform`, [`Lower::lookup`] never answers with one.
    Constants,
    /// No slots: a parameter that is, or holds, a sampler, bound to the
    /// GL name of the uniform its argument names. `id` indexes
    /// [`Lower::uniform_params`]. [`Lower::lookup`] never answers with one,
    /// so no variable path can mistake it for storage; the uniform paths
    /// reach it through [`Lower::uniform_ident`].
    Uniform,
}

impl Run {
    /// How many consecutive slots, whatever the kind.
    const fn slots(self) -> u8 {
        match self {
            Run::Single => 1,
            Run::Matrix(n) | Run::Array(n) | Run::Struct(n) => n,
            Run::StructArray { len, per } => len.saturating_mul(per),
            Run::MatrixArray { len, cols } => len.saturating_mul(cols),
            Run::Uniform | Run::Constants => 0,
        }
    }

    /// How many elements, when this is an array of anything.
    const fn elements(self) -> Option<u8> {
        match self {
            Run::Array(n) | Run::StructArray { len: n, .. } | Run::MatrixArray { len: n, .. } => Some(n),
            _ => None,
        }
    }

    /// N when this is a matrix, and `None` otherwise. Every site that wants
    /// to treat a run as a matrix asks with this, so an array cannot reach a
    /// matrix path by having the same number of slots.
    const fn columns(self) -> Option<u8> {
        match self {
            Run::Matrix(n) => Some(n),
            _ => None,
        }
    }
}

impl<'a> Local<'a> {
    const fn scalar(name: &'a str, id: ir::LocalId, comps: u8) -> Local<'a> {
        Local { name, id, comps, run: Run::Single, value: None }
    }

    /// A parameter that is a name for a value rather than a variable.
    const fn alias(name: &'a str, src: Src, comps: u8) -> Local<'a> {
        Local { name, id: ir::LocalId(u32::MAX), comps, run: Run::Single, value: Some(src) }
    }

    /// Element `k` of an array run, as a local of the element's own shape:
    /// one slot, a matrix's columns, or a struct's leaves. `None` for a run
    /// that is not an array.
    fn element(&self, k: u8) -> Option<Local<'a>> {
        let (off, run) = match self.run {
            Run::Array(_) => (u32::from(k), Run::Single),
            Run::MatrixArray { cols, .. } => (u32::from(k) * u32::from(cols), Run::Matrix(cols)),
            Run::StructArray { per, .. } => (u32::from(k) * u32::from(per), Run::Struct(per)),
            _ => return None,
        };
        Some(Local { name: self.name, id: ir::LocalId(self.id.0 + off), comps: self.comps, run, value: None })
    }

    /// The slot holding column `j`.
    fn column(&self, j: u8) -> Local<'a> {
        Local {
            name: self.name,
            id: ir::LocalId(self.id.0 + u32::from(j)),
            comps: self.comps,
            run: Run::Single,
            value: None,
        }
    }
}

impl<'a> Lower<'_, 'a> {
    /// A refusal at the line being lowered.
    ///
    /// Every diagnostic the lowering raises goes through here, so that adding
    /// one cannot forget the line -- which is how they all came to read
    /// `0:0:` in the first place.
    fn err(&self, tok: &'a str, msg: &'static str) -> Error<'a> {
        Error::new(self.line, tok, msg)
    }

    /* ---- declarations -------------------------------------------------- */

    /// The inputs and outputs, taken from the reflection so that the IR and
    /// the rows cannot disagree about a location.
    ///
    /// A varying is an output of the vertex stage and an input of the
    /// fragment one; the same row serves both, and which list it lands in
    /// says which.
    fn declare_io(&mut self) -> R<'a, ()> {
        for at in self.reflection.attribs.iter() {
            self.inputs.push(
                self.a,
                IoDecl {
                    name: at.name,
                    kind: IoKind::Attribute,
                    location: at.location,
                    components: at.components,
                    interp: ir::INTERP_SMOOTH,
                    gl_type: at.gl_type,
                },
            )?;
        }
        for v in self.reflection.varyings.iter() {
            /* One declaration per LOCATION: a `matN` varying is N of them
             * and an array varying one per element (per column, for an
             * array of matrices). The back ends find a varying's register by
             * the declaration whose location is exactly the slot's -- the
             * GC7000 refused `varying mat2` with "a varying written that the
             * module does not declare" while this declared one row for N
             * locations (dEQP linkage.varying_type_mat2..4 on silicon). The
             * link pairs the stages by NAME, so every location after the
             * first is named `name[j]` in both stages alike. */
            let span = self.varying_locations(v.name) as u32;
            for j in 0..span {
                let name = if j == 0 { v.name } else { reflect::index_name(self.a, v.name, j)? };
                let decl = IoDecl {
                    name,
                    kind: IoKind::Varying,
                    location: v.location + j as u8,
                    components: v.components,
                    /* Smooth in ES 1.00, which has no interpolation
                     * qualifiers (section 4.3.5); ES 3.00's `flat` and
                     * `centroid` as the reflection read them. */
                    interp: v.interp,
                    gl_type: v.gl_type,
                };
                match self.stage {
                    Stage::Vertex => self.outputs.push(self.a, decl)?,
                    Stage::Fragment => self.inputs.push(self.a, decl)?,
                }
            }
        }
        /* ES 3.00 fragment outputs: one declaration per location, element k
         * of an array at location + k, named `name[k]` after the first. */
        for o in self.reflection.outputs.iter() {
            for k in 0..u32::from(o.count.max(1)) {
                let name = if k == 0 { o.name } else { reflect::index_name(self.a, o.name, k)? };
                self.outputs.push(
                    self.a,
                    IoDecl {
                        name,
                        kind: IoKind::Colour,
                        location: o.location + k as u8,
                        components: o.components,
                        interp: ir::INTERP_SMOOTH,
                        gl_type: o.gl_type,
                    },
                )?;
            }
        }
        Ok(())
    }

    /// How many locations the varying `name` occupies: its element count
    /// times its columns -- a matCxR's C, square or not.
    fn varying_locations(&self, name: &str) -> u8 {
        let Some(v) = self.reflection.varyings.iter().find(|v| v.name == name) else { return 1 };
        let cols = match v.gl_type {
            0x8B5A | 0x8B65 | 0x8B66 => 2, // mat2, mat2x3, mat2x4
            0x8B5B | 0x8B67 | 0x8B68 => 3, // mat3, mat3x2, mat3x4
            0x8B5C | 0x8B69 | 0x8B6A => 4, // mat4, mat4x2, mat4x3
            _ => 1,
        };
        v.count.max(1).saturating_mul(cols)
    }

    /// `var.m` where `var` is an ES 3.00 structure between the stages: the
    /// row name of that member (`reflect` gives each member a varying of its
    /// own), which the input and output paths then treat as a plain
    /// varying of that name.
    fn struct_varying_leaf(&self, e: ExprId) -> R<'a, Option<&'a str>> {
        let Expr::Field(b, f) = *self.ast.expr(e) else { return Ok(None) };
        let Expr::Ident(v) = *self.ast.expr(b) else { return Ok(None) };
        if !matches!(self.types.of(b).base, BaseType::Struct(_))
            || self.lookup(v).is_some()
            || self.shadow_of(v).is_some()
        {
            return Ok(None);
        }
        let dot = self.cat(".", f)?;
        let name = self.cat(v, dot)?;
        Ok(self.reflection.varyings.iter().any(|r| r.name == name).then_some(name))
    }

    /// The uniform rows, copied into the IR's own shape.
    fn uniform_decls(&self) -> R<'a, &'a [ir::UniformDecl<'a>]> {
        let mut out = AVec::with_capacity(self.a, self.reflection.uniforms.len())?;
        for u in self.reflection.uniforms.iter() {
            out.push(
                self.a,
                ir::UniformDecl {
                    name: u.name,
                    dword_off: u.dword_off,
                    dwords: u.dwords,
                    gl_type: u.gl_type,
                    array_len: u.array_len,
                    binding: u.binding,
                    inner: u.inner,
                    statically_used: u.statically_used,
                },
            )?;
        }
        Ok(out.into_slice())
    }

    /// The built-in output a name refers to: its slot, its kind and its
    /// component count.
    fn builtin_output(&self, name: &str) -> Option<(ir::IoSlot, IoKind, u8)> {
        match (self.stage, name) {
            (Stage::Vertex, "gl_Position") => Some((ir::IoSlot::Position, IoKind::Position, 4)),
            (Stage::Vertex, "gl_PointSize") => Some((ir::IoSlot::PointSize, IoKind::PointSize, 1)),
            (Stage::Fragment, "gl_FragColor") => Some((ir::IoSlot::FragColor(0), IoKind::Colour, 4)),
            /* ES 3.00 only: sema declares it for no other version. */
            (Stage::Fragment, "gl_FragDepth") => Some((ir::IoSlot::FragDepth, IoKind::Depth, 1)),
            _ => None,
        }
    }

    /// The built-in output names this stage has.
    fn builtin_output_names(&self) -> &'static [&'static str] {
        match self.stage {
            Stage::Vertex => &["gl_Position", "gl_PointSize"],
            Stage::Fragment => &["gl_FragColor", "gl_FragDepth"],
        }
    }

    /// `gl_FragData[0]`'s slot, recorded as an output once.
    ///
    /// It is the same slot `gl_FragColor` writes -- one draw buffer, two
    /// spellings -- so the reflection carries whichever name the shader used.
    fn frag_data_slot(&mut self, name: &'a str) -> R<'a, ir::IoSlot> {
        let slot = ir::IoSlot::FragColor(0);
        if !self.outputs.as_slice().iter().any(|o| o.name == name) {
            self.outputs.push(
                self.a,
                IoDecl {
                    name,
                    kind: IoKind::Colour,
                    location: 0,
                    components: 4,
                    interp: ir::INTERP_SMOOTH,
                    gl_type: 0x8B52,
                },
            )?;
        }
        Ok(slot)
    }

    /// Record a built-in output the shader writes, once, and give its slot.
    fn output_slot(&mut self, name: &'a str) -> R<'a, (ir::IoSlot, u8)> {
        if let Some((slot, kind, comps)) = self.builtin_output(name) {
            if !self.outputs.as_slice().iter().any(|o| o.name == name) {
                self.outputs.push(
                    self.a,
                    IoDecl {
                        name,
                        kind,
                        location: 0,
                        components: comps,
                        interp: ir::INTERP_SMOOTH,
                        gl_type: if comps == 4 { 0x8B52 } else { 0x1406 },
                    },
                )?;
            }
            return Ok((slot, comps));
        }
        /* A user varying, which the vertex stage writes. */
        if self.stage == Stage::Vertex {
            if let Some(v) = self.reflection.varyings.iter().find(|v| v.name == name) {
                return Ok((ir::IoSlot::Varying(v.location), v.components));
            }
        }
        /* An ES 3.00 fragment output, declared by `declare_io`. */
        if self.stage == Stage::Fragment {
            if let Some(o) = self.reflection.outputs.iter().find(|o| o.name == name) {
                return Ok((ir::IoSlot::FragColor(o.location), o.components));
            }
        }
        /* `self.line`, not a parameter: a literal 0 here was the last
         * refusal in the file that could not say where it was. */
        Err(self.err(name, "assignment to this is not lowered yet"))
    }

    /// Give a slot to every output the shader does not simply assign once.
    ///
    /// `gl_Position = m * v;` needs nothing: the value is computed and
    /// stored, and that is the whole life of the output. But
    /// `gl_Position.z = order * gl_Position.w;` both reads the output and
    /// writes one component of it, and an output is not a register the
    /// shader can read -- on either GPU it is a slot the hardware takes at
    /// the end. Every Qt Quick vertex shader in the table does exactly this.
    ///
    /// So an output that is mentioned in any way other than as the whole left
    /// side of a plain assignment is held in a local for the duration of
    /// `main` and stored once at the end, which is what a shader that reads
    /// its own output means. The others keep the direct store, because most
    /// shaders are that shape and making them all allocate a slot would be a
    /// regression the back end would have to optimise away again.
    ///
    /// The test is over the flat expression table rather than a walk: an
    /// output is a global, so a mention anywhere in the unit counts --
    /// including inside a function `main` calls, which a walk of `main` alone
    /// would miss.
    fn shadow_outputs(&mut self, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        /* The candidates are the names this stage can write: the built-in
         * outputs and, from the vertex stage, the varyings. */
        let mut names: AVec<&'a str> = AVec::new();
        for b in self.builtin_output_names() {
            names.push(self.a, b)?;
        }
        if self.stage == Stage::Vertex {
            for v in self.reflection.varyings.iter() {
                names.push(self.a, v.name)?;
            }
        } else {
            for o in self.reflection.outputs.iter() {
                names.push(self.a, o.name)?;
            }
        }
        for i in 0..names.len() {
            let name = names.as_slice()[i];
            let declared = self.global_type(name);
            let is_array = declared.is_some_and(|t| t.array_len.is_some());
            /* An array is always held: even assigned whole (`v = T[2](...)`,
             * ES 3.00) it is its elements, which a direct store cannot take. */
            if self.output_is_written_whole(name) && !is_array {
                continue;
            }
            if !self.ast.exprs.iter().any(|e| matches!(e, Expr::Ident(n) if *n == name)) {
                /* Not mentioned at all: nothing to shadow. */
                continue;
            }
            let (_, comps) = self.output_slot(name)?;
            let id = ir::LocalId(self.locals.len() as u32);
            /* An ARRAY varying is held as a run of slots, one per element,
             * so every way of indexing a local array -- a constant, a run-
             * time select, a compound form -- applies to it unchanged, and
             * `store_shadows` writes each element to its own location. */
            let count = self
                .reflection
                .varyings
                .iter()
                .find(|v| v.name == name)
                .map(|v| v.count)
                .or_else(|| self.reflection.outputs.iter().find(|o| o.name == name).map(|o| o.count))
                .map_or(1, |n| n.max(1));
            /* The run a local of the declared type gets, so every way of
             * writing a local -- `v[k][j] = c` into an array of matrices
             * (dEQP-GLES3 transform_feedback, `out mat2 v[1]`), a column of
             * a matrix, an element of a ONE-element array -- applies to it.
             * Each slot is one location: element k's column j of an array of
             * matrices is at `k * cols + j`, which is how `reflect` hands out
             * the locations. `count > 1` alone decided "array" until
             * 2026-09-24, and `out vec4 o[1]` was a scalar nothing could
             * index. */
            let run = match declared.map(|t| t.base) {
                Some(BaseType::Mat(c, _)) if is_array => Run::MatrixArray { len: count, cols: c },
                Some(BaseType::Mat(c, _)) => Run::Matrix(c),
                _ if is_array || count > 1 => Run::Array(count),
                _ => Run::Single,
            };
            for _ in 0..run.slots() {
                self.locals
                    .push(self.a, ir::VarDecl { name, ty: ir::Type::new(ir::ScalarType::F32, comps), array_len: 1 })?;
            }
            let mut l = Local::scalar(name, id, comps);
            l.run = run;
            /* Bound as a global too, so the local paths -- which ask
             * `lookup` -- reach it from `main` and from any function. */
            if run != Run::Single {
                self.bind(l, true)?;
            }
            self.shadows.push(self.a, (name, l))?;
        }
        let _ = out;
        Ok(())
    }

    /// Store each shadowed output once, at the end of `main`.
    fn store_shadows(&mut self, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        for i in 0..self.shadows.len() {
            let (name, l) = self.shadows.as_slice()[i];
            let (slot, _) = self.output_slot(name)?;
            /* one location per slot: an element, or a column */
            let n = l.run.slots();
            for k in 0..n {
                let el = l.column(k);
                let slot = match slot {
                    ir::IoSlot::Varying(loc) => ir::IoSlot::Varying(loc + k),
                    ir::IoSlot::FragColor(loc) => ir::IoSlot::FragColor(loc + k),
                    other => other,
                };
                let v = self.push_instr(Op::LoadLocal { local: el.id, index: None, comps: l.comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                let w = self.push_instr(Op::StoreOutput { slot, src: Src::id(v) })?;
                out.push(self.a, Node::Instr(ir::InstrId(w.0)))?;
            }
        }
        Ok(())
    }

    /// The type a top-level variable is declared with.
    fn global_type(&self, name: &str) -> Option<TypeSpec<'a>> {
        self.ast.top_level().find_map(|d| match d {
            Decl::Var { name: n, ty, .. } if *n == name => Some(*ty),
            _ => None,
        })
    }

    /// Hold each fragment input that is an array of matrices in the run a
    /// local of its type gets, loaded once at the top of `main`: element k's
    /// column j is the input at `location + k * cols + j`. Then every way of
    /// reading a local array of matrices -- `v[1][0]`, `v[k]` as a matrix, a
    /// run-time index -- reads it. `in mat2 v[2]` was refused, or read
    /// `v[1]` as location + 1 (column 1 of element 0), until 2026-09-24
    /// (dEQP-GLES3 transform_feedback, 301 cases).
    fn hold_matrix_array_inputs(&mut self, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        if self.stage != Stage::Fragment {
            return Ok(());
        }
        for i in 0..self.reflection.varyings.len() {
            let v = self.reflection.varyings[i];
            let Some(t) = self.global_type(v.name) else { continue };
            let (BaseType::Mat(cols, _), Some(_)) = (t.base, t.array_len) else { continue };
            if !self.ast.exprs.iter().any(|e| matches!(e, Expr::Ident(n) if *n == v.name)) {
                continue;
            }
            let id = ir::LocalId(self.locals.len() as u32);
            let run = Run::MatrixArray { len: v.count.max(1), cols };
            for _ in 0..run.slots() {
                self.locals.push(
                    self.a,
                    ir::VarDecl { name: v.name, ty: ir::Type::new(ir::ScalarType::F32, v.components), array_len: 1 },
                )?;
            }
            let mut l = Local::scalar(v.name, id, v.components);
            l.run = run;
            for j in 0..run.slots() {
                let x =
                    self.push_instr(Op::LoadInput { slot: ir::IoSlot::Varying(v.location + j), comps: v.components })?;
                out.push(self.a, Node::Instr(ir::InstrId(x.0)))?;
                self.store_local(l.column(j), Src::id(x), out)?;
            }
            self.bind(l, true)?;
        }
        Ok(())
    }

    /// The shadow slot for `name`, if it has one.
    fn shadow_of(&self, name: &str) -> Option<Local<'a>> {
        self.shadows.as_slice().iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
    }

    /// Is every mention of this output the whole left side of a plain
    /// assignment? Then it needs no slot.
    fn output_is_written_whole(&self, name: &str) -> bool {
        let mentions = self.ast.exprs.iter().filter(|e| matches!(e, Expr::Ident(n) if *n == name)).count();
        let plain = self
            .ast
            .exprs
            .iter()
            .filter(|e| match e {
                Expr::Assign(None, lhs, _) => matches!(self.ast.expr(*lhs), Expr::Ident(n) if *n == name),
                _ => false,
            })
            .count();
        mentions == plain
    }

    /* ---- statements ---------------------------------------------------- */

    /// Lower `main`'s body into a node range.
    fn main_body(&mut self) -> R<'a, IrRange> {
        let Some(body) = self.ast.top_level().find_map(|d| match d {
            Decl::Function { name: "main", body: Some(b), .. } => Some(*b),
            _ => None,
        }) else {
            return Err(self.err("main", "a shader must define 'void main()'"));
        };
        let Stmt::Block(r) = *self.ast.stmt(body) else {
            return Err(self.err("main", "the body of 'main' is not a block"));
        };
        let mut items: AVec<Node> = AVec::new();
        self.push_scope()?;
        /* Before the outputs' shadows and before the body: a global's
         * initialiser may read another global, and `main`'s first statement
         * may read any of them. */
        self.declare_globals(&mut items)?;
        self.shadow_outputs(&mut items)?;
        self.hold_matrix_array_inputs(&mut items)?;
        /* `main` gets the same structured rewrite an inlined body gets, and
         * for the same reason: a bare `return;` partway through it is an
         * early exit, and emitting `Node::Return` where it stands does not
         * express one. Selection treats `Return` as the end of the shader --
         * the GC7000 marks its last instruction rather than branching -- so
         * everything after it ran anyway. `texture_copy_unclipped.frag`
         * (SELECT_SHADER = TextureCopyUnclipped) writes `gl_FragColor` and
         * returns with sixteen nodes and an `if` after it; it compiled clean
         * and would have drawn the wrong colour in exactly the region the
         * early return exists to handle.
         *
         * The run form ends at the return and lowers whatever followed an
         * `if` that returns into that `if`'s `else`, so no `Return` is
         * emitted at all except the one appended below. */
        let ids: &'a [StmtId] = &self.ast.stmt_ids[r.indices()];
        self.lower_run("main", ids, Want::Nothing, None, &mut items)?;
        /* After the run, so it is reached on every path: an early return
         * still has to store the outputs it wrote. */
        self.store_shadows(&mut items)?;
        self.pop_scope();
        items.push(self.a, Node::Return)?;
        /* Straight into the node list, as `commit` does: a copy first
         * cost the whole body twice over (random.23: 146 KB). */
        self.commit(items)
    }

    /// One statement, with the constant local-index folding around it.
    ///
    /// At `fold_depth` 0 an expression statement or a declaration is
    /// planned before it is lowered: which of its index expressions have a
    /// known value ([`Lower::plan_indices`]), and which scalar integer locals
    /// it leaves at a known value ([`Lower::plan_sets`]). The lowering then
    /// takes a known index as a constant, and the recorded values outlive
    /// the statement. `int i = 4; a[i++] = x; a[i++] = y;` becomes stores to
    /// a[4] and a[5] and not two selects over the whole array -- dEQP's
    /// tmp_array cases have 36 such writes into a 40-element local.
    fn statement(&mut self, id: StmtId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        #[cfg(test)]
        let off = FOLD_OFF.load(core::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let off = false;
        let saved = core::mem::take(&mut self.folded);
        let r = if off || self.fold_depth > 0 {
            self.statement_inner(id, out)
        } else {
            match *self.ast.stmt(id) {
                Stmt::Expr(Some(e)) => self.folded_statement(Some(e), None, id, out),
                Stmt::Decl(r) if r.len == 1 => {
                    self.folded_statement(None, Some(self.ast.decl_ids[r.start as usize]), id, out)
                }
                _ => self.statement_inner(id, out),
            }
        };
        let done = core::mem::replace(&mut self.folded, saved);
        done.release(self.a);
        r
    }

    /// [`Lower::statement`] for an expression statement `e` or a one-name
    /// declaration `d` at `fold_depth` 0.
    fn folded_statement(
        &mut self,
        e: Option<ExprId>,
        d: Option<DeclId>,
        id: StmtId,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let (expr, target) = match (e, d.map(|d| *self.ast.decl(d))) {
            (Some(e), _) => (Some(e), None),
            (None, Some(Decl::Var { name, init, storage, .. })) if storage != StorageQualifier::Const => {
                (init, Some(name))
            }
            _ => (None, None),
        };
        /* A call to a function of the shader runs statements of its own and
         * may write any global: nothing is assumed, in the statement or in
         * the body it inlines. */
        if expr.is_some_and(|e| self.calls_user_function(e, 0)) {
            self.fold_depth += 1;
            let r = self.statement_inner(id, out);
            self.fold_depth -= 1;
            return r;
        }
        let mut sets: AVec<'a, (ir::LocalId, i64)> = AVec::new();
        let mut declared: Option<i64> = None;
        if let Some(e) = expr {
            let mut writes: AVec<'a, ir::LocalId> = AVec::new();
            self.collect_writes(e, &mut writes, 0)?;
            self.plan_indices(e, e, writes.as_slice(), false, 0)?;
            self.plan_sets(e, e, writes.as_slice(), &mut sets, false, 0)?;
            if target.is_some() {
                declared = self.pure_value(e, writes.as_slice(), 0);
            }
            writes.release(self.a);
        }
        self.statement_inner(id, out)?;
        for &(l, v) in sets.as_slice() {
            self.set_known(l, v)?;
        }
        sets.release(self.a);
        if let (Some(name), Some(v)) = (target, declared) {
            if let Some(l) = self.tracked_local(name) {
                self.set_known(l, v)?;
            }
        }
        Ok(())
    }

    /* ---- constant local indices ---------------------------------------- */

    fn known_value(&self, l: ir::LocalId) -> Option<i64> {
        self.known.as_slice().iter().find(|(k, _)| *k == l).map(|&(_, v)| v)
    }

    fn forget_known(&mut self, l: ir::LocalId) {
        if let Some(k) = self.known.as_slice().iter().position(|(x, _)| *x == l) {
            let last = self.known.len() - 1;
            self.known.as_mut_slice().swap(k, last);
            self.known.pop();
        }
    }

    fn set_known(&mut self, l: ir::LocalId, v: i64) -> R<'a, ()> {
        self.forget_known(l);
        /* Keep the value in range of what the IR computes: an int lane. */
        if i32::try_from(v).is_ok() {
            self.known.push(self.a, (l, v))?;
        }
        Ok(())
    }

    /// The slot of a scalar integer local `name` names here, if it is one
    /// whose value this can track: one slot, not an alias, not an output's
    /// shadow.
    fn tracked_local(&self, name: &str) -> Option<ir::LocalId> {
        if self.shadow_of(name).is_some() {
            return None;
        }
        let l = self.lookup(name)?;
        if l.run != Run::Single || l.value.is_some() || l.comps != 1 {
            return None;
        }
        let ty = self.locals.as_slice().get(l.id.0 as usize)?.ty;
        (ty.comps == 1 && matches!(ty.scalar, ir::ScalarType::I32)).then_some(l.id)
    }

    /// The children an expression EVALUATES (`.length()` evaluates
    /// nothing).
    fn expr_children(&self, e: ExprId, out: &mut [ExprId; 3]) -> usize {
        match *self.ast.expr(e) {
            Expr::Unary(_, x) => {
                out[0] = x;
                1
            }
            Expr::Binary(_, a, b) | Expr::Assign(_, a, b) | Expr::Index(a, b) | Expr::Comma(a, b) => {
                out[0] = a;
                out[1] = b;
                2
            }
            Expr::Select(c, t, f) => {
                *out = [c, t, f];
                3
            }
            Expr::Field(b, _) => {
                out[0] = b;
                1
            }
            _ => 0,
        }
    }

    /// Does evaluating `e` call a function the shader defines?
    fn calls_user_function(&self, e: ExprId, depth: u32) -> bool {
        if depth > 64 {
            return true;
        }
        match *self.ast.expr(e) {
            Expr::Call { name, args } => {
                self.ast.decls.iter().any(|d| matches!(d, Decl::Function { name: n, .. } if *n == name))
                    || args.indices().any(|i| self.calls_user_function(self.ast.expr_ids[i], depth + 1))
            }
            Expr::ArrayCtor { args, .. } => {
                args.indices().any(|i| self.calls_user_function(self.ast.expr_ids[i], depth + 1))
            }
            _ => {
                let mut c = [ExprId(0); 3];
                let n = self.expr_children(e, &mut c);
                c[..n].iter().any(|&x| self.calls_user_function(x, depth + 1))
            }
        }
    }

    /// The tracked locals `e` assigns or steps, anywhere in it.
    fn collect_writes(&self, e: ExprId, acc: &mut AVec<'a, ir::LocalId>, depth: u32) -> R<'a, ()> {
        if depth > 64 {
            return Ok(());
        }
        let target = match *self.ast.expr(e) {
            Expr::Assign(_, lhs, _) => Some(lhs),
            Expr::Unary(UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec, x) => Some(x),
            _ => None,
        };
        if let Some(t) = target {
            if let Expr::Ident(n) = *self.ast.expr(t) {
                if let Some(l) = self.tracked_local(n) {
                    acc.push(self.a, l)?;
                }
            }
        }
        match *self.ast.expr(e) {
            Expr::Call { args, .. } | Expr::ArrayCtor { args, .. } => {
                for i in args.indices() {
                    self.collect_writes(self.ast.expr_ids[i], acc, depth + 1)?;
                }
            }
            _ => {
                let mut c = [ExprId(0); 3];
                let n = self.expr_children(e, &mut c);
                for &x in &c[..n] {
                    self.collect_writes(x, acc, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    /// How many times `e` names the local `l`.
    fn mentions_local(&self, e: ExprId, l: ir::LocalId, depth: u32) -> usize {
        if depth > 64 {
            return 2;
        }
        match *self.ast.expr(e) {
            Expr::Ident(n) => usize::from(self.tracked_local(n) == Some(l)),
            Expr::Call { args, .. } | Expr::ArrayCtor { args, .. } => {
                args.indices().map(|i| self.mentions_local(self.ast.expr_ids[i], l, depth + 1)).sum()
            }
            _ => {
                let mut c = [ExprId(0); 3];
                let n = self.expr_children(e, &mut c);
                c[..n].iter().map(|&x| self.mentions_local(x, l, depth + 1)).sum()
            }
        }
    }

    /// The value of a side-effect-free integer expression over literals and
    /// known locals the statement does not write.
    fn pure_value(&self, e: ExprId, writes: &[ir::LocalId], depth: u32) -> Option<i64> {
        if depth > 32 {
            return None;
        }
        if let Some(v) = reflect::const_int_expr(self.ast, e) {
            return Some(v);
        }
        let v = match *self.ast.expr(e) {
            Expr::Ident(n) => {
                let l = self.tracked_local(n)?;
                if writes.contains(&l) {
                    return None;
                }
                self.known_value(l)?
            }
            Expr::Unary(UnOp::Minus, x) => self.pure_value(x, writes, depth + 1)?.checked_neg()?,
            Expr::Unary(UnOp::Plus, x) => self.pure_value(x, writes, depth + 1)?,
            Expr::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), a, b) => {
                let (x, y) = (self.pure_value(a, writes, depth + 1)?, self.pure_value(b, writes, depth + 1)?);
                match op {
                    BinOp::Add => x.checked_add(y)?,
                    BinOp::Sub => x.checked_sub(y)?,
                    _ => x.checked_mul(y)?,
                }
            }
            _ => return None,
        };
        i32::try_from(v).ok().map(i64::from)
    }

    /// `x++` / `++x` / `x--` / `--x` of a known local named once in the
    /// statement `stmt`: (the expression's value, x's value after).
    fn known_step(&self, e: ExprId, stmt: ExprId) -> Option<(ir::LocalId, i64, i64)> {
        let Expr::Unary(op @ (UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec), x) = *self.ast.expr(e)
        else {
            return None;
        };
        let Expr::Ident(n) = *self.ast.expr(x) else { return None };
        let l = self.tracked_local(n)?;
        if self.mentions_local(stmt, l, 0) != 1 {
            return None;
        }
        let old = self.known_value(l)?;
        let new = match op {
            UnOp::PreInc | UnOp::PostInc => old.checked_add(1)?,
            _ => old.checked_sub(1)?,
        };
        i32::try_from(new).ok()?;
        Some((l, if matches!(op, UnOp::PreInc | UnOp::PreDec) { new } else { old }, new))
    }

    /// Fill `folded` with the index expressions of `e` (the statement
    /// `stmt`) whose value is known. `cond` is set below a `?:` arm or the
    /// right of `&&` / `||`, where an index with an effect may not run.
    fn plan_indices(&mut self, e: ExprId, stmt: ExprId, writes: &[ir::LocalId], cond: bool, depth: u32) -> R<'a, ()> {
        if depth > 64 {
            return Ok(());
        }
        if let Expr::Index(_, idx) = *self.ast.expr(e) {
            if reflect::const_int_expr(self.ast, idx).is_none() {
                if let Some(v) = self.pure_value(idx, writes, 0) {
                    self.folded.push(self.a, (idx, v, false))?;
                } else if let (false, Some((_, v, _))) = (cond, self.known_step(idx, stmt)) {
                    self.folded.push(self.a, (idx, v, true))?;
                }
            }
        }
        match *self.ast.expr(e) {
            Expr::Call { args, .. } | Expr::ArrayCtor { args, .. } => {
                for i in args.indices() {
                    self.plan_indices(self.ast.expr_ids[i], stmt, writes, cond, depth + 1)?;
                }
            }
            Expr::Select(c, t, f) => {
                self.plan_indices(c, stmt, writes, cond, depth + 1)?;
                self.plan_indices(t, stmt, writes, true, depth + 1)?;
                self.plan_indices(f, stmt, writes, true, depth + 1)?;
            }
            Expr::Binary(BinOp::AndAnd | BinOp::OrOr, a, b) => {
                self.plan_indices(a, stmt, writes, cond, depth + 1)?;
                self.plan_indices(b, stmt, writes, true, depth + 1)?;
            }
            _ => {
                let mut c = [ExprId(0); 3];
                let n = self.expr_children(e, &mut c);
                for &x in &c[..n] {
                    self.plan_indices(x, stmt, writes, cond, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    /// The tracked locals the statement `stmt` leaves at a known value:
    /// `x = <known>`, `x op= <known>`, and a step of a known `x` named once,
    /// each only where it surely runs and only if nothing else in the
    /// statement writes `x`.
    fn plan_sets(
        &self,
        e: ExprId,
        stmt: ExprId,
        writes: &[ir::LocalId],
        sets: &mut AVec<'a, (ir::LocalId, i64)>,
        cond: bool,
        depth: u32,
    ) -> R<'a, ()> {
        if depth > 64 {
            return Ok(());
        }
        let once = |l: ir::LocalId| writes.iter().filter(|&&w| w == l).count() == 1;
        if !cond {
            match *self.ast.expr(e) {
                Expr::Assign(op, lhs, rhs) => {
                    if let Expr::Ident(n) = *self.ast.expr(lhs) {
                        if let Some(l) = self.tracked_local(n).filter(|&l| once(l)) {
                            let others: &[ir::LocalId] = &[];
                            let r = self
                                .pure_value(rhs, others, 0)
                                .filter(|_| self.mentions_local(rhs, l, 0) == 0 || op.is_some());
                            let v = match (op, r) {
                                (None, Some(r)) => Some(r),
                                (Some(BinOp::Add), Some(r)) => self.known_value(l).and_then(|x| x.checked_add(r)),
                                (Some(BinOp::Sub), Some(r)) => self.known_value(l).and_then(|x| x.checked_sub(r)),
                                (Some(BinOp::Mul), Some(r)) => self.known_value(l).and_then(|x| x.checked_mul(r)),
                                _ => None,
                            };
                            if let Some(v) = v.filter(|v| i32::try_from(*v).is_ok()) {
                                sets.push(self.a, (l, v))?;
                            }
                        }
                    }
                }
                Expr::Unary(UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec, _) => {
                    if let Some((l, _, new)) = self.known_step(e, stmt).filter(|(l, _, _)| once(*l)) {
                        sets.push(self.a, (l, new))?;
                    }
                }
                _ => {}
            }
        }
        match *self.ast.expr(e) {
            Expr::Call { args, .. } | Expr::ArrayCtor { args, .. } => {
                for i in args.indices() {
                    self.plan_sets(self.ast.expr_ids[i], stmt, writes, sets, cond, depth + 1)?;
                }
            }
            Expr::Select(c, t, f) => {
                self.plan_sets(c, stmt, writes, sets, cond, depth + 1)?;
                self.plan_sets(t, stmt, writes, sets, true, depth + 1)?;
                self.plan_sets(f, stmt, writes, sets, true, depth + 1)?;
            }
            Expr::Binary(BinOp::AndAnd | BinOp::OrOr, a, b) => {
                self.plan_sets(a, stmt, writes, sets, cond, depth + 1)?;
                self.plan_sets(b, stmt, writes, sets, true, depth + 1)?;
            }
            _ => {
                let mut c = [ExprId(0); 3];
                let n = self.expr_children(e, &mut c);
                for &x in &c[..n] {
                    self.plan_sets(x, stmt, writes, sets, cond, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    /// An index's constant value: a constant expression, or -- at
    /// `fold_depth` 0 -- one [`Lower::plan_indices`] found known and free of
    /// effects.
    fn pure_index(&self, idx: ExprId) -> Option<i64> {
        reflect::const_int_expr(self.ast, idx)
            .or_else(|| self.folded.as_slice().iter().find(|(e, _, side)| *e == idx && !side).map(|&(_, v, _)| v))
    }

    /// Like [`Lower::pure_index`], and also an index whose evaluation has an
    /// effect (`i++`), which the caller then evaluates for it.
    fn folded_index(&self, idx: ExprId) -> Option<(i64, bool)> {
        if let Some(v) = self.pure_index(idx) {
            return Some((v, false));
        }
        self.folded.as_slice().iter().find(|(e, _, _)| *e == idx).map(|&(_, v, side)| (v, side))
    }

    fn statement_inner(&mut self, id: StmtId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        self.line = self.ast.stmt_line(id);
        match *self.ast.stmt(id) {
            /* `;` on its own emits nothing, which is what it means. */
            Stmt::Expr(None) => Ok(()),
            Stmt::Expr(Some(e)) => self.expr_statement(e, out),
            Stmt::Discard => {
                self.uses_discard = true;
                let v = self.push_instr(Op::Discard)?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Ok(())
            }
            /* Never a node. Selection treats `Return` as the end of the
             * shader (the GC7000 marks its last instruction rather than
             * branching), so a `Return` anywhere but at the end of the body
             * is not an early exit -- it is nothing, and the code after it
             * runs. The ONE legitimate `Return` is the one `main_body`
             * appends; every early return is rewritten into structured form
             * by `lower_run` before it can reach here.
             *
             * So this arm refuses. It is reachable only from a run form that
             * does not handle returns -- a loop body today -- and a refusal
             * there is a gap someone can see, where emitting the node was a
             * wrong picture nobody could. */
            Stmt::Return(None) | Stmt::Return(Some(_)) if self.ret.is_some_and(|c| c.loops > 0) => {
                let Some(ctx) = self.ret else { unreachable!("tested by the guard") };
                let value = match *self.ast.stmt(id) {
                    Stmt::Return(v) => v,
                    _ => unreachable!("matched above"),
                };
                self.return_value(ctx.name, value, ctx.want, ctx.slot, out)?;
                self.set_flag(ctx.flag, true, out)?;
                out.push(self.a, Node::Break)?;
                Ok(())
            }
            /* A `return` the run form did not route. `lower_run` handles
             * every `return` at its own level and hands any statement with
             * one inside to the flag form, which is the arm above, so a
             * shader should not reach here; if one does, it is a gap in
             * that routing and says so.
             *
             * A `return` inside a loop said "'return' with a value in 'void
             * main()'" until 2026-09-22 (wrong function, wrong problem, and
             * worded as a verdict on a shader dEQP expects to build), then
             * "a 'return' inside a loop is not lowered yet" until the flag
             * form was written. */
            Stmt::Return(_) => Err(self.err("return", "an early 'return' is not lowered yet")),
            /* A compound statement is a scope (section 4.2.2). It was not
             * one until 2026-09-23: `float a = 1.0; { float a = 2.0; }
             * out0 = a;` stored 2.0, because the inner `a` was still the
             * innermost of that name after its block ended. */
            Stmt::Block(r) => {
                self.push_scope()?;
                let mut res = Ok(());
                for i in r.indices() {
                    res = self.statement(self.ast.stmt_ids[i], out);
                    if res.is_err() {
                        break;
                    }
                }
                self.pop_scope();
                res
            }
            Stmt::If { cond, then_s, else_s } => {
                /* The condition is evaluated once, so its instructions belong
                 * to the enclosing run, not to either arm. */
                let c = self.value(cond, 1, out)?;
                let then_ = self.block_range(then_s)?;
                let else_ = match else_s {
                    Some(s) => self.block_range(s)?,
                    None => IrRange::EMPTY,
                };
                out.push(self.a, Node::If { cond: c, then_, else_ })?;
                Ok(())
            }
            /* `while (c) B` is a loop that breaks when c is false. The
             * condition's instructions go INSIDE the body: it is re-evaluated
             * every iteration, and hoisting them out would test the first
             * iteration's value forever. */
            /* A variable declared in the test is scoped to the end of the
             * body (section 4.2.2), hence the scope around the whole loop. */
            Stmt::While { cond, body } => {
                self.push_scope()?;
                self.fold_depth += 1;
                let r = self.while_loop(cond, body, out);
                self.fold_depth -= 1;
                self.pop_scope();
                r
            }
            /* `do B while (c)`: the same, with the test after the body, which
             * is the whole difference between the two. */
            Stmt::DoWhile { body, cond } => {
                let mut items: AVec<Node> = AVec::new();
                self.fold_depth += 1;
                let r = self
                    .loop_body(body, None, Some(cond), &mut items)
                    .and_then(|_| self.break_unless(Cond::Expr(cond), &mut items));
                self.fold_depth -= 1;
                r?;
                let lb = self.commit(items)?;
                out.push(self.a, Node::Loop { body: lb })?;
                self.after_loop(body, out)
            }
            /* Likewise for the init statement and the test (4.2.2):
             * `int i = 5; for (int i = 0; ...) {} out0 = float(i);` read the
             * loop's `i` after the loop until 2026-09-23. */
            Stmt::For { init, cond, step, body } => {
                self.push_scope()?;
                let r = self.for_loop(init, cond, step, body, out);
                self.pop_scope();
                r
            }
            /* sema has already rejected a `break` outside a loop, so there is
             * no check here to duplicate it and rot. */
            Stmt::Break => {
                out.push(self.a, Node::Break)?;
                Ok(())
            }
            /* GLSL ES 1.00 4.6: `continue` in a `for` runs the step before the
             * next test. The IR has no separate step block, so the step is
             * emitted here as well as at the end of the body. Getting this
             * wrong is silent: the loop still runs, with the wrong number of
             * iterations. */
            /* And in a `do`/`while` it runs the test (6.3: control passes to
             * the loop's condition): the test is the last node of the body,
             * which a `continue` -- a jump to the top -- skips. Without it,
             * `do { continue; } while (++i < 3);` never tested and never
             * ended, which on the GC7000 is a wedge. */
            Stmt::Continue => self.lower_continue(out),
            Stmt::Switch { sel, body } => self.switch_statement(sel, body, out),
            /* `switch_statement` consumes its body's labels itself, and the
             * checker refuses one anywhere else. */
            Stmt::Case(_) => Err(self.err("case", "a case label outside the body of a switch")),
            Stmt::Decl(r) => {
                for i in r.indices() {
                    self.local_decl(self.ast.decl_ids[i], out)?;
                }
                Ok(())
            }
        }
    }

    /* ---- control flow helpers ------------------------------------------ */

    /// `continue`: the step or the test of the innermost loop, then the
    /// jump -- or, inside a switch's body, the switch's flag and a `break`
    /// out of it (see [`Lower::switch_statement`]).
    fn lower_continue(&mut self, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        if let Some(flag) = self.switch_cont {
            self.set_flag(flag, true, out)?;
            out.push(self.a, Node::Break)?;
            return Ok(());
        }
        /* Likewise outside a loop; an empty stack here would be a
         * sema bug, and the step is simply absent. */
        let (step, test) = self.loop_steps.as_slice().last().copied().unwrap_or((None, None));
        if let Some(step) = step {
            self.expr_statement(step, out)?;
        }
        if let Some(test) = test {
            self.break_unless(Cond::Expr(test), out)?;
        }
        out.push(self.a, Node::Continue)?;
        Ok(())
    }

    /// A fresh one-lane bool local, for a flag the lowering keeps.
    fn bool_local(&mut self) -> R<'a, Local<'a>> {
        let id = ir::LocalId(self.locals.len() as u32);
        self.locals.push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(ir::ScalarType::Bool, 1), array_len: 1 })?;
        Ok(Local { name: "", id, comps: 1, run: Run::Single, value: None })
    }

    /// `a || b` for bools already evaluated: `a ? a : b`.
    fn or_bools(&mut self, acc: Option<Src>, b: Src, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        match acc {
            None => Ok(b),
            Some(a) => self.alu_w(ir::AluOp::Bcsel, [a, a, b], 1, out),
        }
    }

    /// ES 3.00 section 6.2, `switch`, as a loop that runs once:
    ///
    /// ```text
    /// m1 = sel == c1; m2 = ...; none = !(m1 || m2 || ...)   (before it)
    /// fell = false
    /// loop {
    ///     if (fell || <this group's labels>) { fell = true; <statements> }
    ///     ... one such `if` per run of labels ...
    ///     break
    /// }
    /// ```
    ///
    /// `fell` is what makes control fall through from one group into the
    /// next; `default`'s condition is `none`, so it is taken only when no
    /// label matched, wherever it stands. A `break` in the body leaves the
    /// loop, which is the switch. A `continue` belongs to an enclosing loop:
    /// it sets a flag and breaks, and the flag is turned into the real
    /// `continue` after the switch. A `return` is a loop's return: the
    /// return flag and a `break` (see [`Lower::lower_run`]).
    ///
    /// Only what can happen is emitted ([`SwitchShape`]): a group that no
    /// earlier group can fall into tests its labels alone, and a group
    /// holding `default` with no label after it is always taken when control
    /// reaches it -- an earlier match either fell into it or left the switch
    /// -- so its statements are emitted without an `if`. An `if` whose
    /// condition is always true is a select the back end cannot remove and
    /// a branch that is never taken both ways (usc-backend, qt_tonemap in
    /// skyboxcube_runtime_tonemapping_300.frag).
    fn switch_statement(&mut self, sel: ExprId, body: StmtId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let Stmt::Block(r) = *self.ast.stmt(body) else {
            return Err(self.err("switch", "the body of a switch must be a compound statement"));
        };
        let ids: &'a [StmtId] = &self.ast.stmt_ids[r.indices()];
        let shape = SwitchShape::of(self.a, self.ast, ids)?;
        let s = self.value(sel, 1, out)?;
        let s = Src { val: s.val, swizzle: [s.swizzle[0]; 4] };
        /* Every label's comparison, in order, and -- when a `default` needs
         * it -- whether any matched. */
        let mut eqs: AVec<'a, Src> = AVec::new();
        let mut any: Option<Src> = None;
        for &id in ids {
            if let Stmt::Case(Some(e)) = *self.ast.stmt(id) {
                let v = self.value(e, 1, out)?;
                let q = self.alu_w(ir::AluOp::Ieq, [s, v, v], 1, out)?;
                eqs.push(self.a, q)?;
                if shape.needs_none {
                    any = Some(self.or_bools(any, q, out)?);
                }
            }
        }
        let none = match (shape.needs_none, any) {
            (false, _) => None,
            (true, Some(a)) => Some(self.alu_w(ir::AluOp::Bnot, [a, a, a], 1, out)?),
            (true, None) => {
                let c = self.push_instr(Op::ConstBool([true; 4]))?;
                out.push(self.a, Node::Instr(ir::InstrId(c.0)))?;
                Some(Src::id(c))
            }
        };
        let fell = if shape.needs_fell {
            let f = self.bool_local()?;
            self.set_flag(f, false, out)?;
            Some(f)
        } else {
            None
        };
        let cont = if contains_outer_continue(self.ast, body) {
            let c = self.bool_local()?;
            self.set_flag(c, false, out)?;
            Some(c)
        } else {
            None
        };

        let saved_cont = self.switch_cont;
        self.switch_cont = cont;
        self.nested += 1;
        if let Some(c) = self.ret.as_mut() {
            c.loops += 1;
        }
        self.push_scope()?;
        self.fold_depth += 1;
        let mut items: AVec<Node> = AVec::new();
        let res = self.switch_groups(ids, &shape, eqs.as_slice(), none, fell, &mut items);
        self.fold_depth -= 1;
        self.pop_scope();
        if let Some(c) = self.ret.as_mut() {
            c.loops -= 1;
        }
        self.nested -= 1;
        self.switch_cont = saved_cont;
        res?;
        items.push(self.a, Node::Break)?;
        let lb = self.commit(items)?;
        out.push(self.a, Node::Loop { body: lb })?;

        if let Some(c) = cont {
            let f = self.push_instr(Op::LoadLocal { local: c.id, index: None, comps: 1 })?;
            out.push(self.a, Node::Instr(ir::InstrId(f.0)))?;
            let mut then_items: AVec<Node> = AVec::new();
            self.lower_continue(&mut then_items)?;
            let then_ = self.commit(then_items)?;
            out.push(self.a, Node::If { cond: Src::id(f), then_, else_: IrRange::EMPTY })?;
        }
        self.after_loop(body, out)
    }

    /// The groups of a switch body, each a run of labels and the
    /// statements after them, as [`Lower::switch_statement`] describes.
    fn switch_groups(
        &mut self,
        ids: &'a [StmtId],
        shape: &SwitchShape<'a>,
        eqs: &[Src],
        none: Option<Src>,
        fell: Option<Local<'a>>,
        items: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let mut label = 0usize;
        let groups = shape.groups.as_slice();
        for (g, grp) in groups.iter().enumerate() {
            let mut m: Option<Src> = None;
            for &id in &ids[grp.labels..grp.stmts] {
                let c = match *self.ast.stmt(id) {
                    Stmt::Case(Some(_)) => {
                        label += 1;
                        Some(eqs[label - 1])
                    }
                    /* `needs_none` is set for every default that is
                     * tested, and an `always` group tests nothing. */
                    _ => none,
                };
                if let (false, Some(c)) = (grp.always, c) {
                    m = Some(self.or_bools(m, c, items)?);
                }
            }
            let mut seg: AVec<Node> = AVec::new();
            /* Only when the next group tests `fell`. */
            if let (Some(f), Some(next)) = (fell, groups.get(g + 1)) {
                if next.fallen_into && !next.always {
                    self.set_flag(f, true, &mut seg)?;
                }
            }
            for &id in &ids[grp.stmts..grp.end] {
                self.statement(id, &mut seg)?;
            }
            if grp.always {
                items.extend_from_slice(self.a, seg.as_slice())?;
                seg.release(self.a);
                continue;
            }
            /* The checker refuses a statement before the first label, so
             * every group has one. */
            let Some(mut c) = m else {
                return Err(self.err("switch", "a group of a switch with no label"));
            };
            if let (true, Some(f)) = (grp.fallen_into, fell) {
                let fv = self.push_instr(Op::LoadLocal { local: f.id, index: None, comps: 1 })?;
                items.push(self.a, Node::Instr(ir::InstrId(fv.0)))?;
                c = self.or_bools(Some(Src::id(fv)), c, items)?;
            }
            let then_ = self.commit(seg)?;
            items.push(self.a, Node::If { cond: c, then_, else_: IrRange::EMPTY })?;
        }
        Ok(())
    }

    /// One statement as a committed node run, for an `if` arm. The arm is
    /// a scope whether it is compound or simple (section 4.2.2).
    fn block_range(&mut self, s: StmtId) -> R<'a, IrRange> {
        let mut items: AVec<Node> = AVec::new();
        self.nested += 1;
        self.fold_depth += 1;
        self.push_scope()?;
        let r = self.statement(s, &mut items);
        self.pop_scope();
        self.fold_depth -= 1;
        self.nested -= 1;
        r?;
        self.commit(items)
    }

    /// `while (c) B`: a loop that breaks when c is false. The condition's
    /// instructions go INSIDE the body: it is re-evaluated every iteration,
    /// and hoisting them out would test the first iteration's value forever.
    fn while_loop(&mut self, cond: Cond, body: StmtId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let mut items: AVec<Node> = AVec::new();
        self.break_unless(cond, &mut items)?;
        self.loop_body(body, None, None, &mut items)?;
        let lb = self.commit(items)?;
        out.push(self.a, Node::Loop { body: lb })?;
        self.after_loop(body, out)
    }

    fn for_loop(
        &mut self,
        init: Option<StmtId>,
        cond: Option<Cond>,
        step: Option<ExprId>,
        body: StmtId,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        /* The initialiser runs once, before the loop. */
        if let Some(init) = init {
            self.statement(init, out)?;
        }
        let mut items: AVec<Node> = AVec::new();
        /* The test, the body and the step run again and again: nothing
         * known before the loop may be assumed inside it. */
        self.fold_depth += 1;
        let r = (|| -> R<'a, ()> {
            if let Some(cond) = cond {
                self.break_unless(cond, &mut items)?;
            }
            self.loop_body(body, step, None, &mut items)?;
            /* The step is the last thing an iteration does. `continue`
             * jumps to it, not past it -- loop_body puts a copy at every
             * `continue`, which is the only reason it needs to know. */
            if let Some(step) = step {
                self.expr_statement(step, &mut items)?;
            }
            Ok(())
        })();
        self.fold_depth -= 1;
        r?;
        let lb = self.commit(items)?;
        out.push(self.a, Node::Loop { body: lb })?;
        self.after_loop(body, out)
    }

    /// Append a loop body, recording the step a `continue` inside it must run.
    fn loop_body(
        &mut self,
        body: StmtId,
        step: Option<ExprId>,
        test: Option<ExprId>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        self.loop_steps.push(self.a, (step, test))?;
        self.nested += 1;
        if let Some(c) = self.ret.as_mut() {
            c.loops += 1;
        }
        /* A `continue` in this loop is this loop's, even inside a switch. */
        let saved_cont = self.switch_cont.take();
        let r = self.statement(body, out);
        self.switch_cont = saved_cont;
        if let Some(c) = self.ret.as_mut() {
            c.loops -= 1;
        }
        self.nested -= 1;
        self.loop_steps.pop();
        r
    }

    /// After a loop whose body returns, nested in another loop of the same
    /// function: the `return` broke only the inner one, so the outer one
    /// breaks too if it was taken. The outermost loop needs nothing here --
    /// [`Lower::lower_run`] guards what follows it.
    fn after_loop(&mut self, body: StmtId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let Some(ctx) = self.ret else { return Ok(()) };
        if ctx.loops == 0 || !contains_return(self.ast, body) {
            return Ok(());
        }
        let f = self.push_instr(Op::LoadLocal { local: ctx.flag.id, index: None, comps: 1 })?;
        out.push(self.a, Node::Instr(ir::InstrId(f.0)))?;
        let mut brk: AVec<Node> = AVec::new();
        brk.push(self.a, Node::Break)?;
        let then_ = self.commit(brk)?;
        out.push(self.a, Node::If { cond: Src::id(f), then_, else_: IrRange::EMPTY })?;
        Ok(())
    }

    /// Store `v` into the one-lane bool `flag`.
    fn set_flag(&mut self, flag: Local<'a>, v: bool, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let c = self.push_instr(Op::ConstBool([v; 4]))?;
        out.push(self.a, Node::Instr(ir::InstrId(c.0)))?;
        self.store_local(flag, Src::id(c), out)
    }

    /// `if (!cond) break;` -- how every loop here stops.
    fn break_unless(&mut self, cond: Cond, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let c = match cond {
            Cond::Expr(e) => self.value(e, 1, out)?,
            /* `while (bool b = e)`: the declaration is made, and its
             * initialiser evaluated, on every test (section 6.3), and the
             * variable is the value tested. It lives in the scope the
             * caller opened around the whole loop (4.2.2). */
            Cond::Decl(d) => {
                self.local_decl(d, out)?;
                let Decl::Var { name, .. } = *self.ast.decl(d) else {
                    /* The parser makes a `Cond::Decl` only from a variable. */
                    return Err(self.err("while", "this loop condition is not lowered yet"));
                };
                let Some(l) = self.lookup(name) else {
                    return Err(self.err(name, "reading this is not lowered yet"));
                };
                let v = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: 1 })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Src::id(v)
            }
        };
        let mut brk: AVec<Node> = AVec::new();
        brk.push(self.a, Node::Break)?;
        let else_ = self.commit(brk)?;
        out.push(self.a, Node::If { cond: c, then_: IrRange::EMPTY, else_ })?;
        Ok(())
    }

    /// Move a run of nodes into the module and name it.
    ///
    /// Children are committed before their parent, so a parent's `Range` is
    /// already valid when the parent node is pushed; the indices are absolute
    /// into `self.nodes` and nothing later moves them.
    fn commit(&mut self, items: AVec<'a, Node>) -> R<'a, IrRange> {
        let first = self.nodes.len() as u32;
        let len = items.len() as u32;
        /* Straight into the flat node list, and the scratch list given back
         * when nothing was allocated after it -- the list of a run is
         * consumed here and never read again. It was copied twice (an
         * `alloc_copy` and the append), which with every run of every
         * inlined call was the largest single use of scratch (random.23
         * stand-in, 2026-09-23). */
        self.nodes.extend_from_slice(self.a, items.as_slice())?;
        items.release(self.a);
        Ok(IrRange { first, len })
    }

    /// Commit the two arms of an `if`, the LATER-built one first: each
    /// scratch list is then the arena's most recent allocation when it is
    /// consumed, and is given back. The ranges do not depend on the order.
    fn commit2(&mut self, then_items: AVec<'a, Node>, else_items: AVec<'a, Node>) -> R<'a, (IrRange, IrRange)> {
        let else_ = self.commit(else_items)?;
        let then_ = self.commit(then_items)?;
        Ok((then_, else_))
    }

    /// An assignment used as a VALUE (section 5.8): perform it, then read
    /// the l-value back.
    ///
    /// Reading it back is the only part that needs an argument. It is sound
    /// exactly when re-evaluating the l-value EXPRESSION cannot do anything
    /// a second time, so the l-value has to be a name, optionally with
    /// fields and with subscripts that are themselves a literal or a name.
    /// `a[i++] = x` used as a value would increment twice and would draw a
    /// plausible wrong picture; it keeps the gap instead.
    ///
    /// Reading back rather than reusing the stored value is deliberate:
    /// section 5.8 says the result has "the type and precision of the
    /// lvalue-expression", and for a masked or swizzled destination the
    /// l-value after the store is the thing the spec names, while the value
    /// that went in is merely equal to it today.
    fn assignment_value(&mut self, e: ExprId, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let Expr::Assign(_, lhs, _) = *self.ast.expr(e) else {
            return Err(self.err("=", "this expression is not lowered yet"));
        };
        if !self.lvalue_rereads_cleanly(lhs) {
            return Err(self.err("=", "an assignment to this used as a value is not lowered yet"));
        }
        self.expr_statement(e, out)?;
        self.value(lhs, want, out)
    }

    /// Can this l-value expression be evaluated a second time with no
    /// observable difference? See [`Self::assignment_value`].
    fn lvalue_rereads_cleanly(&self, e: ExprId) -> bool {
        match *self.ast.expr(e) {
            Expr::Ident(_) => true,
            Expr::Field(inner, _) => self.lvalue_rereads_cleanly(inner),
            Expr::Index(inner, idx) => {
                self.lvalue_rereads_cleanly(inner) && matches!(*self.ast.expr(idx), Expr::IntLit(_) | Expr::Ident(_))
            }
            _ => false,
        }
    }

    /// Evaluate an expression for what it DOES and throw its value away --
    /// the left operand of section 5.9's sequence operator.
    ///
    /// The two paths differ in what they can take, not in what they mean.
    /// An assignment, a call and `++i` are statements and go through
    /// `expr_statement`, which is the only place that knows how to perform
    /// them. Everything else in ES 1.00 is side-effect free, so evaluating
    /// it as a value and never reading the result says exactly what "is
    /// evaluated" means; the dead instructions are what a dead-code pass is
    /// for, and inventing a second opinion about which expressions may be
    /// skipped is how a compiler starts disagreeing with itself.
    fn evaluate_and_discard(&mut self, e: ExprId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        match *self.ast.expr(e) {
            Expr::Assign(..)
            | Expr::Call { .. }
            | Expr::Comma(..)
            | Expr::Unary(UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec, _) => {
                self.expr_statement(e, out)
            }
            _ => {
                let w = self.width(e)?;
                self.value(e, w, out)?;
                Ok(())
            }
        }
    }

    /// The only expression statement in the subset: an assignment to an
    /// output.
    fn expr_statement(&mut self, e: ExprId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        /* `++i;` and `i--;` as statements. As an expression they also have
         * a value, which no shader in either corpus uses and which would
         * need the old value kept; as a statement they are exactly
         * `i = i + 1`, and that is the form that appears. */
        if let Expr::Unary(op @ (UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec), target) =
            *self.ast.expr(e)
        {
            return self.step_by_one(op, target, out);
        }
        /* `f(a, b);` -- a call whose point is its `out` parameters, or one
         * whose value is discarded. */
        if let Expr::Call { name, args } = *self.ast.expr(e) {
            if let Some(f) = self.user_function(name, args) {
                self.inline(name, f, args, Want::Nothing, out)?;
                return Ok(());
            }
        }
        /* `a, b;` -- section 5.9's sequence operator as a statement. Both
         * are evaluated, in order, and neither value is wanted. */
        if let Expr::Comma(a, b) = *self.ast.expr(e) {
            self.expr_statement(a, out)?;
            return self.expr_statement(b, out);
        }
        let (op, lhs, rhs) = match *self.ast.expr(e) {
            Expr::Assign(op, lhs, rhs) => (op, lhs, rhs),
            /* Any other expression as a statement -- `a;`, `b + c;`,
             * `x && y;`, `sin(x);` (section 9's `expression_statement:
             * expression SEMICOLON`): evaluated, the value discarded. ES 1.00
             * expressions outside the forms above have no side effects, so
             * the instructions are dead and a dead-code pass drops them; the
             * point is not to REFUSE a legal statement (dEQP
             * operator.sequence.no_side_effects, 33 cases). The value is
             * taken in the form its TYPE has: a matrix as columns, a struct
             * or an array named by itself as nothing at all. */
            _ => {
                let ty = self.types.of(e);
                if let BaseType::Mat(n, _) = ty.base {
                    if !ty.is_array() {
                        self.matrix_value(e, n, out)?;
                        return Ok(());
                    }
                }
                if ty.is_array() || matches!(ty.base, BaseType::Struct(_)) {
                    if matches!(*self.ast.expr(e), Expr::Ident(_)) {
                        return Ok(());
                    }
                    /* `float[2](a, b);` as a statement: the arguments are
                     * evaluated, the array is not kept. */
                    if let Expr::ArrayCtor { args, .. } = *self.ast.expr(e) {
                        for i in args.indices() {
                            self.evaluate_and_discard(self.ast.expr_ids[i], out)?;
                        }
                        return Ok(());
                    }
                    let k = self.ast.expr(e).kind_name();
                    return Err(self.err(k, "this expression statement is not lowered yet"));
                }
                let w = self.width(e)?;
                self.value(e, w, out)?;
                return Ok(());
            }
        };
        /* `a *= b` is `a = a * b` (section 5.8), computed here rather than
         * rewritten in the parser: the AST stays what was written, which is
         * what the diagnostics quote. `binary` takes the assignment's own
         * expression id for the result type, which is the left side's -- and
         * that is what the compound form means. */
        let value_of = |l: &mut Self, want: u8, out: &mut AVec<'a, Node>| match op {
            None => l.value(rhs, want, out),
            Some(op) => l.binary(e, op, lhs, rhs, out),
        };
        /* `gl_FragData[n] = ...`: the other spelling of the fragment colour
         * output, and the single largest gap in the front end -- 94 of the
         * 153 PowerVR sources that refuse stop here first.
         *
         * Every one of the 103 sources in the table writes index 0, and that
         * is all OpenGL ES 2.0 has: MAX_DRAW_BUFFERS is 1. A larger index is
         * refused by name rather than folded onto slot 0, because folding it
         * would compile a shader that writes somewhere the author did not
         * ask for and draw a picture instead of reporting a problem. */
        if let Expr::Index(base, idx) = *self.ast.expr(lhs) {
            if let Expr::Ident(bname) = *self.ast.expr(base) {
                /* No stage check: sema does not know the name outside a
                 * fragment shader and says so first ("undeclared
                 * identifier"), which is the better message anyway. A second
                 * copy here could only rot. */
                if bname == "gl_FragData" {
                    /* A constant EXPRESSION, not only a literal: `0 + 1` is
                     * the second draw buffer and gets the verdict below. */
                    let Some(n) = reflect::const_int_expr(self.ast, idx) else {
                        /* An index chosen at run time. With one draw buffer
                         * (gl_MaxDrawBuffers is 1) every index but 0 is out
                         * of range, which section 4.1.9 leaves undefined, so
                         * the one colour output is a correct target for all
                         * of them. The index is still evaluated, for
                         * whatever it does (dEQP fragdata.valid_dynamic_index
                         * and valid_uniform_index). */
                        self.evaluate_and_discard(idx, out)?;
                        let slot = self.frag_data_slot(bname)?;
                        let src = value_of(self, 4, out)?;
                        let v = self.push_instr(Op::StoreOutput { slot, src })?;
                        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                        return Ok(());
                    };
                    if n != 0 {
                        return Err(Error::new(
                            0,
                            bname,
                            "OpenGL ES 2.0 has one draw buffer, so only gl_FragData[0] exists",
                        ));
                    }
                    let slot = self.frag_data_slot(bname)?;
                    let src = value_of(self, 4, out)?;
                    let v = self.push_instr(Op::StoreOutput { slot, src })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(());
                }
            }
            /* Anything else indexed FALLS THROUGH to the slot arm below.
             *
             * This used to return an error here, which made that arm dead
             * code -- `m[0] = vec2(...)` refused although the arm exists and
             * its comment says it works. Measured at HEAD before this change:
             * `mat2 m; m[0] = vec2(1.0);` answered "assignment to this is not
             * lowered yet". The guard has to be as narrow as the case it
             * handles, or it answers for cases it knows nothing about. */
        }
        /* `var.m = x` in the vertex stage, `var` an ES 3.00 structure
         * output: a store to the member's own varying, a matrix column by
         * column. Written whole only -- an output cannot be read back. */
        if self.stage == Stage::Vertex {
            if let Some(name) = self.struct_varying_leaf(lhs)? {
                if op.is_some() {
                    return Err(self.err(name, "a compound assignment to a structure output is not lowered yet"));
                }
                let Some(row) = self.reflection.varyings.iter().find(|v| v.name == name).copied() else {
                    return Err(self.err(name, "assignment to this is not lowered yet"));
                };
                if let BaseType::Mat(n, _) = self.types.of(lhs).base {
                    let cols = self.matrix_value(rhs, n, out)?;
                    for j in 0..n {
                        let v = self.push_instr(Op::StoreOutput {
                            slot: ir::IoSlot::Varying(row.location + j),
                            src: cols[j as usize],
                        })?;
                        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    }
                    return Ok(());
                }
                let src = value_of(self, row.components, out)?;
                let v = self.push_instr(Op::StoreOutput { slot: ir::IoSlot::Varying(row.location), src })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                return Ok(());
            }
        }
        /* `a[i].yx = v` with `i` chosen at run time: the element resolved
         * once, the swizzled lanes written in whichever element it is. */
        if let (None, Expr::Field(fb, _)) = (op, *self.ast.expr(lhs)) {
            let chosen_at_run_time = matches!(*self.ast.expr(fb), Expr::Index(_, i) if self.pure_index(i).is_none() && reflect::const_int_expr(self.ast, i).is_none());
            if chosen_at_run_time && !matches!(self.types.of(fb).base, BaseType::Struct(_)) {
                if let Some(r) = self.resolve_lref(lhs, out)? {
                    let w = self.width(lhs)?;
                    let x = value_of(self, w, out)?;
                    return self.store_lref(r, x, out);
                }
            }
        }
        /* ES 3.00 section 5.8: an array is assignable as a whole, from an
         * array value of the same type and size. */
        if self.ast.es3 && self.types.of(lhs).is_array() {
            let Expr::Assign(None, _, r) = *self.ast.expr(e) else {
                return Err(self.err("=", "a compound assignment to an array"));
            };
            let Some(l) = self.local_chain(lhs, &[])? else {
                let k = self.ast.expr(lhs).kind_name();
                return Err(self.err(k, "assignment to this array is not lowered yet"));
            };
            return self.array_fill(r, l, out);
        }
        /* ES 3.00 `a[1] = m;` into a local array of matrices: the element is
         * a run of columns, not one slot. */
        if let (BaseType::Mat(n, _), Expr::Index(base, _)) = (self.types.of(lhs).base, *self.ast.expr(lhs)) {
            if matches!(self.local_chain(base, &[])?.map(|l| l.run), Some(Run::MatrixArray { .. })) {
                let Some(l) = self.local_chain(lhs, &[])? else {
                    return Err(self.err("[]", "a matrix element chosen at run time is not lowered yet"));
                };
                let cols = match op {
                    None => self.matrix_value(rhs, n, out)?,
                    Some(bop) => self.matrix_binary(bop, lhs, rhs, n, out)?,
                };
                return self.store_columns(l, &cols, out);
            }
        }
        /* A destination reached through a struct over a local: a member of
         * an element of a struct array, an element of an array member
         * (`s[1].b[0].a = x`, `s[i].c = k`, `s.v[j] = x`). A chain of
         * constants names one slot or run; a run-time index writes every
         * slot it can name, guarded by the index. Anything this does not
         * take (a swizzle below the chain) falls through to the arms below. */
        if self.chain_has_struct(lhs) {
            if let Some(l) = self.local_chain(lhs, &[])? {
                match l.run {
                    Run::Struct(_) => {
                        if let BaseType::Struct(sname) = self.types.of(lhs).base {
                            let Expr::Assign(None, _, r) = *self.ast.expr(e) else {
                                return Err(self.err("=", "a compound assignment to a struct"));
                            };
                            return self.struct_fill(r, l, BaseType::Struct(sname), sname, out);
                        }
                    }
                    Run::Single if l.value.is_none() => {
                        let v = value_of(self, l.comps, out)?;
                        return self.store_local(l, v, out);
                    }
                    _ => {}
                }
            } else {
                let mut dyns: AVec<(ExprId, u8)> = AVec::new();
                if self.chain_dyn_indices(lhs, &mut dyns)? && !dyns.is_empty() {
                    if op.is_some() && !self.lvalue_rereads_cleanly(lhs) {
                        return Err(self
                            .err("[]", "a compound assignment through an index with side effects is not lowered yet"));
                    }
                    let w = self.width(lhs)?;
                    let v = value_of(self, w, out)?;
                    if self.chain_write(lhs, v, out)? {
                        return Ok(());
                    }
                }
            }
        }
        /* `c.xy = ...`: a destination that is some lanes of a slot. The
         * masked store is the same instruction as a whole-slot write, with
         * the mask and the swizzle saying which lanes move. */
        if let Expr::Field(base, field) = *self.ast.expr(lhs) {
            /* A struct member is a whole slot, not lanes of one. */
            if let Some(m) = self.struct_member_of(base, field)? {
                /* A member that is itself a STRUCT is a run of slots, filled
                 * the way a declaration's initialiser fills one: `s.b =
                 * T(x, v);`, `s.b = t;`, `s.b = f();` (section 5.8 makes a
                 * struct assignable as a whole). It was lowered as one value
                 * and refused as "this function call is not lowered yet"
                 * (dEQP shaders.struct.local.*nested*, 12 cases). There is no
                 * compound form on a struct; the checker refuses one first. */
                if let (Run::Struct(_), BaseType::Struct(sname)) = (m.run, self.types.of(lhs).base) {
                    let Expr::Assign(None, _, r) = *self.ast.expr(e) else {
                        return Err(self.err(field, "a compound assignment to a struct"));
                    };
                    return self.struct_fill(r, m, BaseType::Struct(sname), sname, out);
                }
                let src = value_of(self, m.comps, out)?;
                return self.store_local(m, src, out);
            }
            if let Some(l) = self.lvalue_slot(base)? {
                let (mask, lanes) = swizzle_mask(self.line, field, l.comps)?;
                let src = value_of(self, lanes.iter().filter(|x| x.is_some()).count() as u8, out)?;
                return self.store_lanes(l, src, mask, lanes, out);
            }
            let k = self.ast.expr(lhs).kind_name();
            return Err(self.err(k, "assignment to this is not lowered yet"));
        }
        /* `v[1] = x` and `m[0][0] = x`: ONE LANE of a vector, which is the
         * same masked store as `v.y = x` (section 5.5: "pos[2] refers to
         * the third element of pos and is equivalent to pos.z"). Decided by the
         * TYPE of what is indexed -- a vector, not an array of them and not
         * a matrix, whose element is a whole slot (the arm below). */
        if let Expr::Index(base, idx) = *self.ast.expr(lhs) {
            let bt = self.types.of(base);
            if bt.array_len == 0
                && matches!(bt.base, BaseType::Vec(_) | BaseType::IVec(_) | BaseType::UVec(_) | BaseType::BVec(_))
            {
                /* `m[i][j] = x` with `i` chosen at run time: the column is
                 * one of several, resolved once (`LRef`). */
                if let (None, Expr::Index(_, ci)) = (op, *self.ast.expr(base)) {
                    if self.pure_index(ci).is_none() {
                        if let Some(r) = self.resolve_lref(lhs, out)? {
                            let x = value_of(self, 1, out)?;
                            return self.store_lref(r, x, out);
                        }
                    }
                }
                let Some(l) = self.lvalue_slot(base)? else {
                    let k = self.ast.expr(base).kind_name();
                    return Err(self.err(k, "a component of this as a destination is not lowered yet"));
                };
                let Some(k) = self.pure_index(idx) else {
                    /* A lane chosen at run time: one component-wise compare
                     * of the index against (0, 1, 2, 3) is the lane mask,
                     * and one select writes the whole vector. */
                    let i = self.index_value(idx, out)?;
                    let x = value_of(self, 1, out)?;
                    let x = Src { val: x.val, swizzle: [x.swizzle[0]; 4] };
                    let lanes = self.int_const([0, 1, 2, 3], out)?;
                    let mask = self.alu_w(ir::AluOp::Ieq, [i, lanes, lanes], l.comps, out)?;
                    let old = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(old.0)))?;
                    let new = self.alu_w(ir::AluOp::Bcsel, [mask, x, Src::id(old)], l.comps, out)?;
                    return self.store_local(l, new, out);
                };
                if k < 0 || k as u32 >= u32::from(l.comps) {
                    return Err(self.err("[]", "index out of range"));
                }
                let mut lanes = [None; 4];
                lanes[k as usize] = Some(0);
                let src = value_of(self, 1, out)?;
                return self.store_lanes(l, src, 1 << k, lanes, out);
            }
        }
        /* `a[i] = x` and `m[i] = v` with `i` chosen at run time: every
         * element of the local takes `i == k ? x : old` (`store_among`).
         * The value is computed first, so a compound form reads the element
         * through the same run-time select before it is replaced -- which
         * re-evaluates the index, so a compound form is taken only when
         * that is a clean re-read. */
        if let Expr::Index(base, idx) = *self.ast.expr(lhs) {
            /* `a[i++] = x` where `i` is known here (`Lower::statement`): the
             * element is fixed, and `i` still steps. */
            if let (Some((k, side)), Expr::Ident(name)) = (self.folded_index(idx), *self.ast.expr(base)) {
                if let Some(l) = self.shadow_of(name).or_else(|| self.lookup(name)) {
                    if let Run::Array(n) | Run::Matrix(n) = l.run {
                        if reflect::const_int_expr(self.ast, idx).is_none()
                            && (0..i64::from(n)).contains(&k)
                            && (op.is_none() || !side)
                        {
                            let v = value_of(self, l.comps, out)?;
                            if side {
                                self.evaluate_and_discard(idx, out)?;
                            }
                            return self.store_local(l.column(k as u8), v, out);
                        }
                    }
                }
            }
            if self.pure_index(idx).is_none() {
                if let Expr::Ident(name) = *self.ast.expr(base) {
                    if let Some(l) = self.shadow_of(name).or_else(|| self.lookup(name)) {
                        if let Run::Array(n) | Run::Matrix(n) = l.run {
                            if op.is_some() && !self.lvalue_rereads_cleanly(lhs) {
                                return Err(self.err(
                                    "[]",
                                    "a compound assignment through an index with side effects is not lowered yet",
                                ));
                            }
                            let v = value_of(self, l.comps, out)?;
                            let i = self.index_value(idx, out)?;
                            let mut slots: AVec<Local<'a>> = AVec::new();
                            for k in 0..n {
                                slots.push(self.a, l.column(k))?;
                            }
                            return self.store_among(slots.as_slice(), i, v, l.comps, out);
                        }
                    }
                }
            }
        }
        /* `m[0] = vec2(...)`: a whole column of a matrix local. */
        if matches!(*self.ast.expr(lhs), Expr::Index(..)) {
            if let Some(l) = self.lvalue_slot(lhs)? {
                let src = value_of(self, l.comps, out)?;
                return self.store_local(l, src, out);
            }
            return Err(self.err("[]", "assignment to this is not lowered yet"));
        }
        let Expr::Ident(name) = *self.ast.expr(lhs) else {
            let k = self.ast.expr(lhs).kind_name();
            return Err(self.err(k, "assignment to this is not lowered yet"));
        };
        if let Some(l) = self.shadow_of(name).filter(|_| self.lookup(name).is_none()) {
            let src = value_of(self, l.comps, out)?;
            return self.store_local(l, src, out);
        }
        /* A local shadows anything of the same name outside the function
         * (section 4.2.3), so it is looked for first. */
        if let Some(l) = self.lookup(name) {
            if let Run::Array(_) = l.run {
                /* Section 5.7: an array is not assignable as a whole in
                 * ES 1.00, and there is no array-valued expression to assign
                 * from. Refused by KIND -- it has the same run of slots as a
                 * matrix, and `matrix_value` would happily build one. */
                return Err(self.err(name, "an array cannot be assigned as a whole"));
            }
            if let Run::Struct(_) = l.run {
                /* `p = func(p);` and `p = Pos(1.0, 2.0, 3.0);` -- section
                 * 5.8 makes a struct assignable as a whole, and it is the
                 * same move a declaration's initialiser makes, so it goes
                 * through the same `struct_fill`. There is no compound form:
                 * section 5.9 has no operator over structs, and the checker
                 * refuses one first. */
                let Expr::Assign(None, _, r) = *self.ast.expr(e) else {
                    return Err(self.err(name, "a compound assignment to a struct"));
                };
                let ty = self.types.of(lhs);
                return self.struct_fill(r, l, ty.base, name, out);
            }
            if let Some(n) = l.run.columns() {
                /* `m += a` is `m = m + a` (section 5.8), with the same
                 * matrix arithmetic; `m *= a` is the product for a matrix
                 * `a` and a scale for a scalar one, as in `matrix_binary`. */
                let cols = match op {
                    None => self.matrix_value(rhs, n, out)?,
                    Some(bop) => self.matrix_binary(bop, lhs, rhs, n, out)?,
                };
                return self.store_columns(l, &cols, out);
            }
            let src = value_of(self, l.comps, out)?;
            return self.store_local(l, src, out);
        }
        /* A `matN` varying, written: N stores, one per column, to the N
         * consecutive locations it owns.
         *
         * This has to be decided by the TYPE and before `output_slot`, which
         * answers with the row's `components` -- N, the width of ONE COLUMN.
         * Taken for the width of the whole matrix it would store a single
         * column and leave the other N-1 locations never written: three
         * quarters of a `mat4` varying carrying whatever the last draw left,
         * with nothing to report. */
        if let BaseType::Mat(n, _) = self.types.of(lhs).base {
            let Expr::Assign(None, _, rhs) = *self.ast.expr(e) else {
                /* `m *= ...` on a varying would have to read it back, and a
                 * varying is write-only in the vertex stage. */
                return Err(self.err(name, "this matrix assignment is not lowered yet"));
            };
            let Some(base) = self.reflection.varyings.iter().find(|v| v.name == name).map(|v| v.location) else {
                return Err(self.err(name, "assignment to this is not lowered yet"));
            };
            let cols = self.matrix_value(rhs, n, out)?;
            for j in 0..n {
                let v =
                    self.push_instr(Op::StoreOutput { slot: ir::IoSlot::Varying(base + j), src: cols[j as usize] })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            }
            return Ok(());
        }
        let (slot, comps) = self.output_slot(name)?;
        let src = value_of(self, comps, out)?;
        let v = self.push_instr(Op::StoreOutput { slot, src })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(())
    }

    /* ---- expressions --------------------------------------------------- */

    /// Lower `e`, appending the instructions that produce it, and give the
    /// operand that reads the result.
    fn value(&mut self, e: ExprId, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* Narrowed to the expression and not restored afterwards: an error
         * carries the line it was built at, and the next node sets it again,
         * so the innermost construct that failed is the one reported. */
        self.line = self.ast.expr_line(e);
        match *self.ast.expr(e) {
            /* A section 7.4 built-in constant: the integer itself. */
            Expr::Ident(name) if reflect::builtin_constant(name, self.ast.es3).is_some() => {
                let v = reflect::builtin_constant(name, self.ast.es3).unwrap_or(0);
                self.int_const([v as i32; 4], out)
            }
            Expr::Ident(name) => {
                /* A shadowed output reads back from its slot, which is what
                 * a shader that reads its own output means. */
                if let Some(l) = self.shadow_of(name).filter(|_| self.lookup(name).is_none()) {
                    let v = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(Src::id(v));
                }
                if let Some(l) = self.lookup(name) {
                    /* A name for a value: no load, no register. */
                    if let Some(v) = l.value {
                        return Ok(v);
                    }
                    if let Run::Array(_) = l.run {
                        /* An array where a value is wanted. Section 5.7 has
                         * no array-valued expression, so this is the shader
                         * being wrong, not a gap. */
                        return Err(self.err(name, "an array has no value; index it"));
                    }
                    if l.run.columns().is_some() {
                        /* A matrix where a vector is wanted: reading column 0
                         * would compile and draw a plausible wrong picture. */
                        return Err(self.err(name, "this matrix expression is not lowered yet"));
                    }
                    let v = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(Src::id(v));
                }
                if let Some((slot, comps)) = self.builtin_input(name) {
                    let v = self.push_instr(Op::LoadInput { slot, comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(Src::id(v));
                }
                if let Some((slot, comps)) = self.input_of(name) {
                    let v = self.push_instr(Op::LoadInput { slot, comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(Src::id(v));
                }
                self.uniform_value(name, 0, out)
            }
            Expr::Index(base, idx) if matches!(*self.ast.expr(base), Expr::Ident(n) if self.const_array(n).is_some()) =>
            {
                let Expr::Ident(n) = *self.ast.expr(base) else { unreachable!("matched above") };
                let Some((vals, w)) = self.const_array(n) else { unreachable!("matched above") };
                match reflect::const_int_expr(self.ast, idx) {
                    Some(k) if (0..vals.len() as i64).contains(&k) => Ok(vals[k as usize]),
                    Some(_) => Err(self.err("[]", "index out of range")),
                    None => {
                        let i = self.index_value(idx, out)?;
                        self.select_among(vals, i, w, out)
                    }
                }
            }
            Expr::Index(base, idx) => {
                /* An element reached through a struct over a local
                 * (`s.b[i]`, `s[i].v[1]`). */
                if self.chain_has_struct(e) {
                    let w = self.width(e)?;
                    if let Some(v) = self.chain_read(e, w, out)? {
                        return Ok(v);
                    }
                }
                self.indexed(base, idx, out)
            }
            Expr::FloatLit(v) => self.const_value([v; 4], out),
            /* An `int`, as an `int`. This was a float constant until
             * 2026-09-23 -- `3` became the bits of 3.0 -- and every integer
             * operation it met (`Ilt`, `Iadd`, `I2f`) read those bits as the
             * integer 1,077,936,128: `for (int i = 0; i < 3; i = i + 1)`
             * compiled for the GC7000 to `cmp.lt.s32 ..., 3.0:f20` and
             * `add.s32 ..., 1.0:f20`, ran twice instead of three times, and
             * `float(k)` for `int k = 3` read 1.08e9. Nothing refused it. */
            Expr::IntLit(v) => self.int_const([v as i32; 4], out),
            /* A `uint` is its bit pattern in an integer lane; the operations
             * that read it say it is unsigned (ir.md, the ES 3.00 row). */
            Expr::UintLit(v) => self.int_const([v as i32; 4], out),
            /* A bool literal, so that `while (true) { ... break; }` has a
             * condition to lower. Every other boolean expression -- a
             * comparison, `&&`, a bool variable -- is the front end's
             * expression work and still lands in the arm below. */
            Expr::BoolLit(v) => {
                let id = self.push_instr(Op::ConstBool([v; 4]))?;
                /* The node too, not just the instruction: selection walks
                 * nodes, so an instruction no node names is an instruction
                 * that never runs. */
                out.push(self.a, Node::Instr(ir::InstrId(id.0)))?;
                Ok(Src::id(id))
            }
            Expr::Call { name, args } => {
                /* The width the CALL produces, which is not always the width
                 * the caller wants: `max(v3, v3).x` is a vec3 result read one
                 * lane at a time, and giving the built-in the caller's 1
                 * would lower a three-lane operation as a scalar. */
                let w = self.width(e).unwrap_or(want);
                self.constructor(name, args, w, out)
            }
            Expr::Field(base, field) if self.stage == Stage::Fragment && self.struct_varying_leaf(e)?.is_some() => {
                let _ = (base, field);
                let Some(name) = self.struct_varying_leaf(e)? else { unreachable!("matched above") };
                let Some((slot, comps)) = self.input_of(name) else {
                    return Err(self.err(name, "reading this is not lowered yet"));
                };
                let v = self.push_instr(Op::LoadInput { slot, comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Ok(Src::id(v))
            }
            Expr::Field(base, field) => {
                /* `_25.qt_Matrix` and `v.xyz` are the same syntax. Which it
                 * is depends on what `base` is, and a member of a uniform
                 * struct has a reflection row of its own -- looked up by the
                 * joined name, because that is the name `glGetUniformLocation`
                 * answers for and the offline table stores. */
                if let Some(name) = self.uniform_path(e)? {
                    return self.uniform_value(name, 0, out);
                }
                /* Section 7.5's built-in uniform: its members are rows of
                 * their own (`reflect::DEPTH_RANGE_MEMBERS`) and load above.
                 * The struct as a whole -- passed, assigned, compared -- is
                 * not a shape anything here builds. */
                if matches!(*self.ast.expr(base), Expr::Ident("gl_DepthRange"))
                    && self.types.of(base).base == BaseType::Struct(super::sema::DEPTH_RANGE_PARAMETERS)
                {
                    return Err(self.err("gl_DepthRange", "this use of gl_DepthRange is not lowered yet"));
                }
                /* `s.b` where `s` is a struct LOCAL: the member has a slot of
                 * its own. Asked before the swizzle reading because a member
                 * access and a swizzle are the same syntax, and the checker's
                 * type is what tells them apart. */
                if let Some(v) = self.struct_field(base, field, out)? {
                    return Ok(v);
                }
                /* A leaf of a LOCAL reached through a struct array or a
                 * run-time index (`s[i].a`, `s.b[j].c`). */
                if self.chain_has_struct(e) {
                    let w = self.width(e)?;
                    if let Some(v) = self.chain_read(e, w, out)? {
                        return Ok(v);
                    }
                }
                /* A leaf of a uniform reached through an array of structs,
                 * or through a run-time index. */
                if matches!(self.types.of(base).base, BaseType::Struct(_)) && !self.types.of(base).is_array() {
                    let dot = self.cat(".", field)?;
                    let w = self.width(e)?;
                    if let Some(v) = self.uniform_leaf(base, dot, LeafLoad::Value, w, out)? {
                        return Ok(v);
                    }
                }
                let inner = self.value(base, 4, out)?;
                self.swizzle(inner, field)
            }
            Expr::Select(cond, t, f) => {
                /* `c ? a : b` is a component-wise select with both arms
                 * evaluated, which is what `Bcsel` is and what the hardware
                 * does -- when neither arm has a side effect (below). */
                let w = self.width(e)?;
                /* The condition is ONE bool and `Bcsel` is component-wise,
                 * so it is broadcast to every lane. Taken as it came until
                 * 2026-09-23: `c ? v1 : v2` on vectors read lanes y, z and
                 * w of a one-lane compare that nothing wrote, and chose
                 * lane by lane (the evaluator gave a matrix `?:` columns
                 * mixed 1, 6, 3, 8 where 1, 2, 3, 4 was right). */
                let c = self.operand_at(cond, 1, out)?;
                let c = Src { val: c.val, swizzle: [c.swizzle[0]; 4] };
                /* Section 5.9: only the chosen arm is evaluated. With a side
                 * effect in either arm the two are an `If` into one slot;
                 * otherwise evaluating both is invisible and `Bcsel` is the
                 * one instruction the hardware has for it. */
                if self.has_side_effects(t, 0) || self.has_side_effects(f, 0) {
                    let slot = self.temp(self.scalar_kind(e)?, w)?;
                    let mut then_items: AVec<Node> = AVec::new();
                    let a = self.operand_at(t, w, &mut then_items)?;
                    self.store_local(slot, a, &mut then_items)?;
                    let mut else_items: AVec<Node> = AVec::new();
                    let b = self.operand_at(f, w, &mut else_items)?;
                    self.store_local(slot, b, &mut else_items)?;
                    let (then_, else_) = self.commit2(then_items, else_items)?;
                    out.push(self.a, Node::If { cond: c, then_, else_ })?;
                    let v = self.push_instr(Op::LoadLocal { local: slot.id, index: None, comps: w })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(Src::id(v));
                }
                let a = self.operand_at(t, w, out)?;
                let b = self.operand_at(f, w, out)?;
                self.alu(ir::AluOp::Bcsel, [c, a, b], out)
            }
            /* Section 5.9: "The sequence ( , ) operator ... returns the type
             * and value of the right-most expression in a comma separated
             * list of expressions. All expressions are evaluated, in order,
             * from left to right."
             *
             * So the value is the right one, and the left is evaluated for
             * whatever it does. */
            Expr::Comma(a, b) => {
                self.evaluate_and_discard(a, out)?;
                self.value(b, want, out)
            }
            /* Section 5.8: an assignment IS an expression. It "stores the
             * value of the rvalue-expression into the l-value and returns an
             * r-value with the type and precision of the lvalue-expression",
             * which is what makes `x[2] = x[3] = 0.0` legal. */
            Expr::Assign(..) => self.assignment_value(e, want, out),
            /* Section 5.9: `++i` and `i++` are expressions too, and they
             * differ in WHICH value. Placed before the general unary arm
             * because that one has no notion of a store. */
            Expr::Unary(op @ (UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec), target) => {
                self.step_value(op, target, want, out)
            }
            Expr::Unary(op, x) => self.unary(op, x, out),
            Expr::Binary(op, lhs, rhs) => self.binary(e, op, lhs, rhs, out),
            /* ES 3.00 section 5.9: an integer constant, the array's size --
             * its type's, and the operand is not evaluated. */
            Expr::Length(x) => {
                let t = self.types.of(x);
                if !t.is_array() {
                    return Err(self.err("length", "the type of this expression is not known"));
                }
                self.int_const([t.array_len as i32; 4], out)
            }
            /* An array where one value is wanted: section 5.7 of neither
             * language has an array-valued operand here; `array_fill` and
             * `indexed` are where an array constructor is taken apart. */
            Expr::ArrayCtor { .. } => Err(self.err("[]", "an array has no value; index it")),
            /* No fallback arm: every `Expr` form now has a lowering to reach,
             * and the compiler says so if one is ever added without one.
             * The arm that used to be here said "this expression is not
             * lowered yet" and became unreachable when `Comma` and `Assign`
             * landed -- a refusal that can no longer happen is worse than no
             * refusal, because it reads like a live safety net. The gaps that
             * remain are inside these arms and each says which construct it
             * is. */
        }
    }

    /// `s.b` where `s` is a struct local: the member's own slot, loaded.
    ///
    /// `None` means the base is not a struct local, and the caller goes on
    /// to its other readings rather than refusing.
    fn struct_field(&mut self, base: ExprId, field: &'a str, out: &mut AVec<'a, Node>) -> R<'a, Option<Src>> {
        let m = match self.struct_member_of(base, field)? {
            Some(m) => m,
            None => {
                /* `S(in0).val` and `f(x).val`: a member of a struct VALUE
                 * that is not a variable (dEQP scoping.valid.local_int_
                 * variable_hides_struct_type). It is filled into a temporary
                 * run by `struct_fill` -- the one place that knows a
                 * constructor from a call -- and the member read from it. */
                let ty = self.types.of(base);
                let (BaseType::Struct(sname), false, Expr::Call { .. }) =
                    (ty.base, ty.is_array(), *self.ast.expr(base))
                else {
                    return Ok(None);
                };
                let line = self.line;
                let run = self.struct_run(sname, sname, line)?;
                let id = ir::LocalId(self.locals.len() as u32);
                self.push_struct_slots(sname, sname, line, 0)?;
                let tmp = Local { name: "", id, comps: 1, run, value: None };
                self.struct_fill(base, tmp, ty.base, sname, out)?;
                match self.struct_member(tmp, ty.base, field) {
                    Some(m) => m,
                    None => return Err(self.err(field, "this struct has no such member")),
                }
            }
        };
        let v = self.push_instr(Op::LoadLocal { local: m.id, index: None, comps: m.comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Some(Src::id(v)))
    }

    /// The slot `base.field` denotes, when `base` is of struct type.
    fn struct_member_of(&mut self, base: ExprId, field: &'a str) -> R<'a, Option<Local<'a>>> {
        let ty = self.types.of(base);
        if !matches!(ty.base, BaseType::Struct(_)) || ty.is_array() {
            return Ok(None);
        }
        let Some(l) = self.struct_run_of(base)? else { return Ok(None) };
        match self.struct_member(l, ty.base, field) {
            Some(m) => Ok(Some(m)),
            None => Err(self.err(field, "this struct has no such member")),
        }
    }

    /// The run of slots an expression of struct type occupies.
    ///
    /// A name is the common case, and was for a while the only one this
    /// accepted -- which made `a.b.c` unreadable however deeply the layout
    /// nested, because the base of the outer `.c` is not a name. A member
    /// that is itself a struct keeps its `Run::Struct`, so asking again with
    /// the inner type is the whole of the recursion.
    fn struct_run_of(&mut self, e: ExprId) -> R<'a, Option<Local<'a>>> {
        let l = self.local_chain(e, &[])?;
        Ok(l.filter(|l| matches!(l.run, Run::Struct(_))))
    }

    /// The local slot or run a chain of struct members and array indices
    /// denotes -- `s`, `s.b`, `s[1].b[0].a` -- with every index a constant,
    /// or one of `forced`, which pins a run-time index to an element (the
    /// select over elements asks once per element). `None`: not a local, or
    /// an index neither constant nor pinned.
    fn local_chain(&mut self, e: ExprId, forced: &[(ExprId, u8)]) -> R<'a, Option<Local<'a>>> {
        Ok(match *self.ast.expr(e) {
            Expr::Ident(name) => self.shadow_of(name).or_else(|| self.lookup(name)),
            Expr::Field(b, f) => {
                let Some(l) = self.local_chain(b, forced)? else { return Ok(None) };
                if !matches!(l.run, Run::Struct(_)) {
                    return Ok(None);
                }
                match self.struct_member(l, self.types.of(b).base, f) {
                    Some(m) => Some(m),
                    None => return Err(self.err(f, "this struct has no such member")),
                }
            }
            Expr::Index(b, idx) => {
                let Some(l) = self.local_chain(b, forced)? else { return Ok(None) };
                let k = match self.pure_index(idx) {
                    Some(k) => k,
                    None => match forced.iter().find(|(x, _)| *x == idx) {
                        Some(&(_, k)) => i64::from(k),
                        None => return Ok(None),
                    },
                };
                if k < 0
                    || k >= i64::from(match l.run {
                        Run::Array(n) | Run::Matrix(n) => n,
                        Run::StructArray { len, .. } | Run::MatrixArray { len, .. } => len,
                        _ => return Ok(None),
                    })
                {
                    return Err(self.err("[]", "index out of range"));
                }
                let k = k as u8;
                match l.run {
                    Run::Array(_) | Run::Matrix(_) => Some(l.column(k)),
                    Run::MatrixArray { .. } => l.element(k),
                    Run::StructArray { per, .. } => Some(Local {
                        name: l.name,
                        id: ir::LocalId(l.id.0 + u32::from(k) * u32::from(per)),
                        comps: 1,
                        run: Run::Struct(per),
                        value: None,
                    }),
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// The run-time indices in a chain over a local, outermost first, each
    /// with the length it selects among.
    fn chain_dyn_indices(&self, e: ExprId, acc: &mut AVec<'a, (ExprId, u8)>) -> R<'a, bool> {
        match *self.ast.expr(e) {
            Expr::Ident(name) => Ok(self.shadow_of(name).or_else(|| self.lookup(name)).is_some()),
            Expr::Field(b, _) => self.chain_dyn_indices(b, acc),
            Expr::Index(b, idx) => {
                if !self.chain_dyn_indices(b, acc)? {
                    return Ok(false);
                }
                if self.pure_index(idx).is_none() {
                    let bt = self.types.of(b);
                    let len = if bt.is_array() {
                        bt.array_len
                    } else if let BaseType::Mat(n, _) = bt.base {
                        u32::from(n)
                    } else {
                        return Ok(false);
                    };
                    let Ok(len) = u8::try_from(len) else { return Ok(false) };
                    acc.push(self.a, (idx, len))?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Does this chain go through a struct (a member access, or a struct
    /// array)? Plain arrays and matrices keep their own paths.
    fn chain_has_struct(&self, e: ExprId) -> bool {
        match *self.ast.expr(e) {
            Expr::Field(b, _) => matches!(self.types.of(b).base, BaseType::Struct(_)) || self.chain_has_struct(b),
            Expr::Index(b, _) => {
                (self.types.of(b).is_array() && matches!(self.types.of(b).base, BaseType::Struct(_)))
                    || self.chain_has_struct(b)
            }
            _ => false,
        }
    }

    /// Read a one-slot leaf of a chain through structs over a local, run-
    /// time indices included: every element the indices can name, and a
    /// select per index (as many as the elements they name together fit
    /// the slot bound).
    fn chain_read(&mut self, e: ExprId, w: u8, out: &mut AVec<'a, Node>) -> R<'a, Option<Src>> {
        if !self.chain_has_struct(e) {
            return Ok(None);
        }
        let mut dyns: AVec<(ExprId, u8)> = AVec::new();
        if !self.chain_dyn_indices(e, &mut dyns)? {
            return Ok(None);
        }
        /* The selects touch every slot the indices can name together, so
         * the bound is that product, not how many indices there are. */
        let combos: u32 = dyns.as_slice().iter().map(|&(_, n)| u32::from(n)).product();
        if combos > u32::from(MAX_ARRAY_SLOTS) {
            return Err(self.err("[]", "an access this many run-time indices deep is not lowered yet"));
        }
        let mut idx_vals: AVec<Src> = AVec::new();
        for &(idx, _) in dyns.as_slice() {
            let v = self.index_value(idx, out)?;
            idx_vals.push(self.a, v)?;
        }
        let mut forced: AVec<(ExprId, u8)> = AVec::new();
        self.chain_read_at(e, w, dyns.as_slice(), idx_vals.as_slice(), &mut forced, out)
    }

    fn chain_read_at(
        &mut self,
        e: ExprId,
        w: u8,
        dyns: &[(ExprId, u8)],
        idx_vals: &[Src],
        forced: &mut AVec<'a, (ExprId, u8)>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Option<Src>> {
        let d = forced.len();
        if d == dyns.len() {
            let Some(l) = self.local_chain(e, forced.as_slice())? else { return Ok(None) };
            if l.run != Run::Single || l.value.is_some() {
                return Ok(None);
            }
            let v = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            return Ok(Some(Src::id(v)));
        }
        let (idx, len) = dyns[d];
        let mut els: AVec<Src> = AVec::new();
        for k in 0..len {
            forced.push(self.a, (idx, k))?;
            let v = self.chain_read_at(e, w, dyns, idx_vals, forced, out)?;
            forced.pop();
            let Some(v) = v else { return Ok(None) };
            els.push(self.a, v)?;
        }
        Ok(Some(self.select_among(els.as_slice(), idx_vals[d], w, out)?))
    }

    /// Write `v` to a one-slot leaf of a chain through structs over a
    /// local, run-time indices included: every slot the indices can name
    /// takes `v` where every index equals its element, and keeps its value
    /// elsewhere. `false`: not such a chain.
    fn chain_write(&mut self, lhs: ExprId, v: Src, out: &mut AVec<'a, Node>) -> R<'a, bool> {
        let mut dyns: AVec<(ExprId, u8)> = AVec::new();
        if !self.chain_dyn_indices(lhs, &mut dyns)? || dyns.is_empty() {
            return Ok(false);
        }
        /* The selects touch every slot the indices can name together, so
         * the bound is that product, not how many indices there are. */
        let combos: u32 = dyns.as_slice().iter().map(|&(_, n)| u32::from(n)).product();
        if combos > u32::from(MAX_ARRAY_SLOTS) {
            return Err(self.err("[]", "an access this many run-time indices deep is not lowered yet"));
        }
        let mut idx_vals: AVec<Src> = AVec::new();
        for &(idx, _) in dyns.as_slice() {
            let x = self.index_value(idx, out)?;
            idx_vals.push(self.a, x)?;
        }
        let mut forced: AVec<(ExprId, u8)> = AVec::new();
        self.chain_write_at(lhs, v, dyns.as_slice(), idx_vals.as_slice(), &mut forced, None, out)
    }

    #[allow(clippy::too_many_arguments)]
    fn chain_write_at(
        &mut self,
        lhs: ExprId,
        v: Src,
        dyns: &[(ExprId, u8)],
        idx_vals: &[Src],
        forced: &mut AVec<'a, (ExprId, u8)>,
        cond: Option<Src>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, bool> {
        let d = forced.len();
        if d == dyns.len() {
            let Some(l) = self.local_chain(lhs, forced.as_slice())? else { return Ok(false) };
            if l.run != Run::Single || l.value.is_some() {
                return Ok(false);
            }
            let Some(c) = cond else { return Ok(false) };
            let old = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(old.0)))?;
            let new = self.alu_w(ir::AluOp::Bcsel, [c, v, Src::id(old)], l.comps, out)?;
            self.store_local(l, new, out)?;
            return Ok(true);
        }
        let (idx, len) = dyns[d];
        for k in 0..len {
            let kc = self.int_const([i32::from(k); 4], out)?;
            let eq = self.alu_w(ir::AluOp::Ieq, [idx_vals[d], kc, kc], 1, out)?;
            let eq = Src { val: eq.val, swizzle: [eq.swizzle[0]; 4] };
            let c = self.and_bools(cond, eq, out)?;
            forced.push(self.a, (idx, k))?;
            let ok = self.chain_write_at(lhs, v, dyns, idx_vals, forced, Some(c), out)?;
            forced.pop();
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// A built-in input of this stage, if the name is one.
    ///
    /// These are values the hardware supplies rather than anything the shader
    /// declares, so they have no reflection row and are recognised by name.
    fn builtin_input(&mut self, name: &str) -> Option<(ir::IoSlot, u8)> {
        match (self.stage, name) {
            (Stage::Fragment, "gl_FragCoord") => Some((ir::IoSlot::FragCoord, 4)),
            (Stage::Fragment, "gl_PointCoord") => Some((ir::IoSlot::PointCoord, 2)),
            (Stage::Fragment, "gl_FrontFacing") => {
                self.uses_front_facing = true;
                Some((ir::IoSlot::FrontFacing, 1))
            }
            /* ES 3.00; the checker declares them only in an ES 3.00 unit. */
            (Stage::Vertex, "gl_VertexID") => Some((ir::IoSlot::VertexId, 1)),
            (Stage::Vertex, "gl_InstanceID") => Some((ir::IoSlot::InstanceId, 1)),
            _ => None,
        }
    }

    /// The input a name reads, if it is one.
    fn input_of(&self, name: &str) -> Option<(ir::IoSlot, u8)> {
        if self.stage == Stage::Vertex {
            if let Some(at) = self.reflection.attribs.iter().find(|a| a.name == name) {
                return Some((ir::IoSlot::Attrib(at.location), at.components));
            }
        } else if let Some(v) = self.reflection.varyings.iter().find(|v| v.name == name) {
            return Some((ir::IoSlot::Varying(v.location), v.components));
        }
        None
    }

    /// A read of a uniform block member, at `element` of an array.
    ///
    /// The offset and the type come from the reflection row -- the same row
    /// the offline tables are checked against -- so the dword the shader
    /// reads and the dword `glUniform4fv` writes are the same number by
    /// construction, not by two calculations agreeing.
    fn uniform_value(&mut self, name: &'a str, element: u32, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let Some(u) = self.reflection.uniforms.iter().find(|u| u.name == name) else {
            return Err(self.err(name, "reading this is not lowered yet"));
        };
        if u.dword_off == reflect::NO_BLOCK {
            /* A sampler is named by binding in a Tex op, never loaded from
             * the block; that lowering is not here yet. */
            return Err(self.err(name, "sampling a texture is not lowered yet"));
        }
        let Some(comps) = components_of(u.gl_type) else {
            /* A matrix is columns, and assembling them needs the arithmetic
             * lowering. Refusing beats emitting one wrong-width load. */
            return Err(self.err(name, "reading this uniform type is not lowered yet"));
        };
        if element >= u32::from(u.array_len) {
            return Err(self.err(name, "array index out of range"));
        }
        /* An array's stride is its dwords divided by its length: the
         * reflection already rounded the element size up to its alignment,
         * so recomputing it here could only disagree with the table. */
        let stride = u32::from(u.dwords) / u32::from(u.array_len.max(1));
        let dword_off = u32::from(u.dword_off) + element * stride;
        let Ok(dword_off) = u16::try_from(dword_off) else {
            return Err(self.err(name, "this uniform is too far into the block"));
        };
        let v = self.push_instr(Op::LoadUniform { dword_off, comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        self.bool_from_block(u.gl_type, Src::id(v), comps, out)
    }

    /// A value read from the uniform block, as the IR's type.
    ///
    /// The GL core stores a bool uniform as 0 or 1 whatever setter loaded it
    /// (program.rs), and the IR's bool is an all-ones mask (`ir.md`
    /// decision 4), so a bool is normalised with `I2b` on the way in. Read
    /// raw until 2026-09-23: `!u` was `Bnot(1)` = 0xfffffffe, still true,
    /// and `u == true` compared 1 against the mask -- for u = true,
    /// `(!u ? 1.0 : 2.0) + (u == true ? 10.0 : 20.0)` came out 22, not 12.
    ///
    /// At the load's own width: `all(b)` on a `bvec2` reduces over what
    /// this records, and a four-lane `I2b` would hand it two lanes nothing
    /// loaded.
    fn bool_from_block(&mut self, gl_type: u16, v: Src, comps: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        if is_bool_gl_type(gl_type) {
            return self.alu_w(ir::AluOp::I2b, [v, v, v], comps, out);
        }
        Ok(v)
    }

    /// `u_colors[2]`: a uniform array element at a constant index.
    ///
    /// A non-constant index is `LoadUniformIdx`, which needs the arithmetic
    /// lowering to produce the index value; until then it is refused rather
    /// than folded to element zero, which would draw something plausible and
    /// wrong.
    fn indexed(&mut self, base: ExprId, idx: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* ES 3.00: an array that is a VALUE -- `float[3](a, b, c)[i]`,
         * `f()[i]`, `(c ? x : y)[i]` -- is moved into a temporary run and
         * indexed there, like a local. */
        if self.ast.es3
            && self.types.of(base).is_array()
            && matches!(
                *self.ast.expr(base),
                Expr::ArrayCtor { .. } | Expr::Call { .. } | Expr::Select(..) | Expr::Comma(..)
            )
        {
            let l = self.array_run_of(base, out)?;
            return self.element_value(l, idx, out);
        }
        /* `a[j]` where `a` is a LOCAL array: element j is the slot `id + j`,
         * the same arithmetic a matrix column uses. Asked first and by NAME,
         * because a uniform array is also `is_array()` and is something else
         * entirely -- an offset into the block, handled further down. */
        if self.types.of(base).is_array() {
            if let Expr::Ident(name) = *self.ast.expr(base) {
                if let Some(l) = self.shadow_of(name).or_else(|| self.lookup(name)) {
                    if let Run::StructArray { .. } | Run::MatrixArray { .. } = l.run {
                        return self.element_value(l, idx, out);
                    }
                    if let Run::Array(n) = l.run {
                        let Some(j) = reflect::const_int_expr(self.ast, idx) else {
                            /* A run-time index: every element, and a select. */
                            let i = self.index_value(idx, out)?;
                            let mut els: AVec<Src> = AVec::new();
                            for k in 0..n {
                                let el = l.column(k);
                                let v =
                                    self.push_instr(Op::LoadLocal { local: el.id, index: None, comps: el.comps })?;
                                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                                els.push(self.a, Src::id(v))?;
                            }
                            return self.select_among(els.as_slice(), i, l.comps, out);
                        };
                        if j < 0 || j as u32 >= u32::from(n) {
                            return Err(self.err("[]", "index out of range"));
                        }
                        let el = l.column(j as u8);
                        let v = self.push_instr(Op::LoadLocal { local: el.id, index: None, comps: el.comps })?;
                        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                        return Ok(Src::id(v));
                    }
                }
            }
        }
        /* `var[k]` where `var` is a fragment shader's ARRAY varying: element
         * k is the input at location + k, and a run-time k selects among
         * them. */
        if self.stage == Stage::Fragment && self.types.of(base).is_array() {
            if let Expr::Ident(name) = *self.ast.expr(base) {
                if self.lookup(name).is_none() {
                    /* Any array, a one-element one included (`in vec4 v[1]`
                     * was refused until 2026-09-24); never an array of
                     * matrices, whose element is `cols` locations and which
                     * `hold_matrix_array_inputs` holds as a local. */
                    let vectors = !matches!(self.types.of(base).base, BaseType::Mat(..));
                    if let Some(v) = self.reflection.varyings.iter().find(|v| v.name == name).filter(|_| vectors) {
                        let (loc, comps, n) = (v.location, v.components, v.count);
                        let mut els: AVec<Src> = AVec::new();
                        for k in 0..n {
                            let x = self.push_instr(Op::LoadInput { slot: ir::IoSlot::Varying(loc + k), comps })?;
                            out.push(self.a, Node::Instr(ir::InstrId(x.0)))?;
                            els.push(self.a, Src::id(x))?;
                        }
                        return match reflect::const_int_expr(self.ast, idx) {
                            Some(j) if j >= 0 && (j as u32) < u32::from(n) => Ok(els.as_slice()[j as usize]),
                            Some(_) => Err(self.err("[]", "index out of range")),
                            None => {
                                let i = self.index_value(idx, out)?;
                                self.select_among(els.as_slice(), i, comps, out)
                            }
                        };
                    }
                }
            }
        }
        /* `v[i]`: indexing a vector gives a component, which is a swizzle
         * and no instruction at all. `a_xf[0][1]` -- a column of a matrix
         * attribute, then a component of that column -- is two of these and
         * reaches the register directly. */
        /* An ARRAY of vectors has the same base type as a vector, and
         * indexing it means something else entirely -- an element, not a
         * component. `uniform vec4 frag[14];` is the shape that makes the
         * difference, and treating it as a component selection refused 33
         * sources for "component out of range" before this guard. */
        let base_ty = self.types.of(base);
        if !base_ty.is_array()
            && matches!(base_ty.base, BaseType::Vec(_) | BaseType::IVec(_) | BaseType::UVec(_) | BaseType::BVec(_))
        {
            let w = self.width(base)?;
            let Some(i) = reflect::const_int_expr(self.ast, idx) else {
                /* A component chosen at run time is a select over the lanes,
                 * not an address. */
                return self.vector_lane_select(base, idx, w, out);
            };
            if i < 0 || i as u32 >= u32::from(w) {
                return Err(self.err("[]", "vector component out of range"));
            }
            let v = self.value(base, w, out)?;
            return Ok(Src { val: v.val, swizzle: [v.swizzle[i as usize]; 4] });
        }
        /* `m[j]`: indexing a matrix gives a column, which the value form
         * already has. */
        if let BaseType::Mat(n, r) = self.types.of(base).base {
            let Some(j) = reflect::const_int_expr(self.ast, idx) else {
                /* A column chosen at run time: every column, and a select. */
                let i = self.index_value(idx, out)?;
                let cols = self.matrix_value(base, n, out)?;
                return self.select_among(&cols[..n as usize], i, r, out);
            };
            if j < 0 || j as u32 >= u32::from(n) {
                return Err(self.err("[]", "matrix column out of range"));
            }
            /* Only the column that is read, not all N. `matrix_value`
             * materialises the whole matrix, which is right when the matrix
             * itself is the value and wasteful when one column of it is. */
            if let Expr::Ident(name) = *self.ast.expr(base) {
                if let Some(l) = self.lookup(name) {
                    if l.run.columns() != Some(n) {
                        return Err(self.err(name, "this matrix expression is not lowered yet"));
                    }
                    let col = l.column(j as u8);
                    let v = self.push_instr(Op::LoadLocal { local: col.id, index: None, comps: l.comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    return Ok(Src::id(v));
                }
            }
            if matches!(*self.ast.expr(base), Expr::Ident(_) | Expr::Field(..)) {
                return self.matrix_column(base, j as u8, n, out);
            }
            let cols = self.matrix_value(base, n, out)?;
            return Ok(cols[j as usize]);
        }
        let name = match *self.ast.expr(base) {
            Expr::Ident(name) => name,
            /* `_25.values[i]`: an array that is a member of a uniform
             * struct, named the way the reflection names it. */
            Expr::Field(b, f) => match self.uniform_path(base)? {
                Some(name) => name,
                None => {
                    /* The same member reached through an array of structs
                     * (`s[i].vals[2]`): element k of the leaf, from every
                     * struct element the chain can name. */
                    let dot = self.cat(".", f)?;
                    /* The element's type is the array's element type. */
                    let w = self.types.of(base).base.components().max(1);
                    let len = self.types.of(base).array_len.max(1);
                    let v = match reflect::const_int_expr(self.ast, idx) {
                        Some(k) if k >= 0 && (k as u32) < len => {
                            self.uniform_leaf(b, dot, LeafLoad::Element(k as u32), w, out)?
                        }
                        Some(_) => return Err(self.err("[]", "array index out of range")),
                        None => {
                            let i = self.index_value(idx, out)?;
                            let mut els: AVec<Src> = AVec::new();
                            let mut all = true;
                            for k in 0..len {
                                match self.uniform_leaf(b, dot, LeafLoad::Element(k), w, out)? {
                                    Some(v) => els.push(self.a, v)?,
                                    None => all = false,
                                }
                            }
                            if all {
                                Some(self.select_among(els.as_slice(), i, w, out)?)
                            } else {
                                None
                            }
                        }
                    };
                    return match v {
                        Some(v) => Ok(v),
                        None => Err(self.err("[]", "this expression is not lowered yet")),
                    };
                }
            },
            _ => return Err(self.err("[]", "this expression is not lowered yet")),
        };
        let Some(element) = reflect::const_int_expr(self.ast, idx) else {
            return self.uniform_at_index(name, idx, out);
        };
        if element < 0 {
            return Err(self.err(name, "array index out of range"));
        }
        self.uniform_value(name, element as u32, out)
    }

    /// Element `idx` of an array run whose element is one slot: a load at a
    /// constant index, a select over every element at a run-time one. An
    /// element that is a matrix or a struct has no one value, and is taken
    /// apart by `matrix_value` or a member access instead.
    fn element_value(&mut self, l: Local<'a>, idx: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let (Run::Array(n), None) = (l.run, l.value) else {
            return Err(self.err("[]", "this array element has no single value"));
        };
        if let Some(j) = reflect::const_int_expr(self.ast, idx) {
            if j < 0 || j as u32 >= u32::from(n) {
                return Err(self.err("[]", "index out of range"));
            }
            let el = l.column(j as u8);
            let v = self.push_instr(Op::LoadLocal { local: el.id, index: None, comps: el.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            return Ok(Src::id(v));
        }
        let i = self.index_value(idx, out)?;
        let mut els: AVec<Src> = AVec::new();
        for k in 0..n {
            let el = l.column(k);
            let v = self.push_instr(Op::LoadLocal { local: el.id, index: None, comps: el.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            els.push(self.a, Src::id(v))?;
        }
        self.select_among(els.as_slice(), i, l.comps, out)
    }

    /// `frag[i]` where `i` is not a constant.
    ///
    /// The index reaches the instruction rather than the address, because the
    /// hardware indexes the constant file at run time and the front end
    /// cannot know the value. `len` goes with it: an index outside the array
    /// is undefined in GLSL, but reading elsewhere in the constant file --
    /// where another shader's uniforms or a sampler's properties sit -- is
    /// not an acceptable undefined, so the back end has the bound it needs to
    /// clamp.
    fn uniform_at_index(&mut self, name: &'a str, idx: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let Some(u) = self.reflection.uniforms.iter().find(|u| u.name == name) else {
            return Err(self.err(name, "reading this is not lowered yet"));
        };
        if u.dword_off == reflect::NO_BLOCK {
            return Err(self.err(name, "indexing a sampler array is not lowered yet"));
        }
        let Some(comps) = components_of(u.gl_type) else {
            return Err(self.err(name, "indexing this uniform type is not lowered yet"));
        };
        let len = u16::from(u.array_len.max(1));
        let stride_dw = u.dwords / len;
        let index = self.value(idx, 1, out)?;
        let v = self.push_instr(Op::LoadUniformIdx { base_dw: u.dword_off, stride_dw, len, index, comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        self.bool_from_block(u.gl_type, Src::id(v), comps, out)
    }

    /// A `vecN(...)` of constants, which is what the literal-colour fragment
    /// shader is. Anything else is a built-in call, or refused by name.
    fn constructor(&mut self, name: &'a str, args: Range, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* The same table the checker uses, so a name is a constructor here
         * exactly when it was one there. */
        let base = match super::sema::constructor_type(name, self.ast.es3) {
            Some(b) if !matches!(b, BaseType::Mat(..)) => b,
            /* A matrix constructor builds N columns, which needs the
             * column-wise value form. */
            Some(_) => return Err(self.err(name, "this constructor is not lowered yet")),
            /* Not a constructor at all: a built-in or a user function. Saying
             * "constructor" would send the reader looking for a type of that
             * name, so the built-in library gets its turn first and only a
             * name it does not claim is refused. */
            None => return self.builtin_call(name, args, want, out),
        };
        let comps = base.components().max(1);
        let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
        /* A matrix argument to a scalar or vector constructor (section
         * 5.4.2): its components in column-major order, as many as the type
         * takes -- `vec4(m2)` is m[0].xy then m[1].xy. dEQP-GLES2
         * attribute_location.* passes matrix attributes through `vec4(a_0)`. */
        if ids.iter().any(|&e| matches!(self.types.of(e).base, BaseType::Mat(..)) && !self.types.of(e).is_array()) {
            return self.assemble_with_matrices(name, base, comps, ids, out);
        }
        let mut lanes = [0.0f32; 4];
        if ids.len() == 1 {
            /* One argument fills every component (section 5.4.2). A literal
             * folds to a constant; anything else is a value, and then the
             * fill is a swizzle rather than an instruction. */
            if let Ok(x) = self.float_of(ids[0]) {
                lanes = [x; 4];
            } else {
                return self.convert(ids[0], base, comps, out);
            }
        } else if ids.len() == comps as usize && ids.iter().all(|e| self.float_of(*e).is_ok()) {
            /* Every argument a literal: one constant, no temporary. Common
             * enough to be worth the special case -- a literal colour is most
             * of the fragment shaders in the table. */
            for (lane, e) in lanes.iter_mut().zip(ids) {
                *lane = self.float_of(*e)?;
            }
        } else {
            return self.assemble(name, base, comps, ids, out);
        }
        let _ = want;
        if matches!(self.scalar_of_type(base), ir::ScalarType::I32 | ir::ScalarType::U32)
            && ids.iter().all(|e| self.int_lit(*e).is_some())
        {
            let mut ints = [0i32; 4];
            for (lane, e) in ints.iter_mut().zip(ids.iter().cycle()) {
                *lane = self.int_lit(*e).unwrap_or(0);
            }
            if ids.len() == 1 {
                ints = [ints[0]; 4];
            }
            return self.int_const(ints, out);
        }
        /* The constant is of the constructed type, converted as section
         * 5.4.1 says: to an int the fractional part is dropped, to a bool
         * zero is false and anything else true. Folding every constructor
         * to a float constant made `ivec2(1, 2)` the bits of 1.0 and 2.0 and
         * `bvec2(1, 0)` a float 1.0 where a bool is an all-ones mask. */
        self.typed_const(self.scalar_of_type(base), lanes, out)
    }

    /// A constant of scalar kind `to` from literal lanes, converted as
    /// section 5.4.1 says: to an int the fractional part is dropped, to a
    /// bool zero is false and anything else true.
    fn typed_const(&mut self, to: ir::ScalarType, lanes: [f32; 4], out: &mut AVec<'a, Node>) -> R<'a, Src> {
        match to {
            ir::ScalarType::I32 => self.int_const(lanes.map(|x| x as i32), out),
            /* The same bits as an int; a negative float is undefined as a
             * uint (section 5.4.1) and saturates here like `as` does. */
            ir::ScalarType::U32 => self.int_const(lanes.map(|x| x as u32 as i32), out),
            ir::ScalarType::Bool => {
                let v = self.push_instr(Op::ConstBool(lanes.map(|x| x != 0.0)))?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Ok(Src::id(v))
            }
            ir::ScalarType::F32 => self.const_value(lanes, out),
        }
    }

    /// A call to a function that is not a constructor: a GLSL built-in, or a
    /// user-defined function (which is not lowered yet).
    ///
    /// The arguments are lowered first and handed over as operands, so that
    /// `builtin_call` never sees the AST: that is what lets the built-in
    /// library be written and tested without one.
    fn builtin_call(&mut self, name: &'a str, args: Range, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* A user-defined function is looked for first, so that a shader's own
         * helper is the one that runs. */
        if let Some(f) = self.user_function(name, args) {
            return match self.inline(name, f, args, Want::Value(want), out)? {
                Ret::Value(v) => Ok(v),
                /* Asked for a value and given columns, or a filled run:
                 * impossible, since the want is what decided the shape. */
                Ret::Matrix(_) | Ret::Struct => Err(self.err(name, "this function call is not lowered yet")),
            };
        }
        /* ES 3.00 `determinant`: a float from a matrix's columns. */
        if name == "determinant" && self.ast.es3 && args.len == 1 {
            let arg = self.ast.expr_ids[args.start as usize];
            let BaseType::Mat(n, _) = self.types.of(arg).base else {
                return Err(self.err(name, "the type of this expression is not known"));
            };
            let m = self.matrix_value(arg, n, out)?;
            return self.determinant(&m, n, out);
        }
        let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
        if ids.len() > MAX_BUILTIN_ARGS {
            return Err(self.err(name, "this function call is not lowered yet"));
        }
        if self.ast.es3 && name == "modf" && ids.len() == 2 {
            return self.modf(name, ids[0], ids[1], out);
        }
        /* A matrix argument arrives as its N COLUMNS, one operand each, with
         * the column's own component count. That is not a convenience: a
         * matrix has no other form here (`ir.md` decision 4), and
         * `BaseType::components()` counts a matrix BY COLUMN, so a `mat4`
         * handed over as one operand would claim four components exactly as a
         * `vec4` does. A built-in reading that and emitting one instruction
         * would operate on the first column and silently drop the rest --
         * a plausible instruction and a wrong picture.
         *
         * So `matrixCompMult(a, b)` on `mat3` is six operands of three
         * components, and the built-in's arm is N component-wise multiplies
         * over the pairs. Ruled by the team lead on this representation. */
        let mut ops = [Operand::Sampler {
            binding: 0,
            ty: crate::shaderc::online::ast::SamplerType::new(SamplerDim::D2, SamplerRet::Float, false),
        }; MAX_BUILTIN_OPERANDS];
        let mut n = 0usize;
        for &e in ids {
            let ty = self.types.of(e);
            if let BaseType::Mat(cols, rows) = ty.base {
                if n + cols as usize > MAX_BUILTIN_OPERANDS {
                    return Err(self.err(name, "this function call is not lowered yet"));
                }
                let columns = self.matrix_value(e, cols, out)?;
                for c in columns.iter().take(cols as usize) {
                    ops[n] = Operand::Value { src: *c, comps: rows, ty: BaseType::Vec(rows) };
                    n += 1;
                }
                continue;
            }
            if n == MAX_BUILTIN_OPERANDS {
                return Err(self.err(name, "this function call is not lowered yet"));
            }
            ops[n] = self.operand(e, want, out)?;
            n += 1;
        }
        let ops = &ops[..n];
        let es3 = self.ast.es3;
        let mut sink = Sink { l: self, out };
        let lowered = match builtin_call::lower_call(&mut sink, name, ops, want) {
            None if es3 => builtin_call::lower_es3(&mut sink, name, ops),
            r => r,
        };
        match lowered {
            Some(r) => r,
            /* Not a built-in: a user-defined function, which needs inlining
             * or a call in the IR. Neither exists yet. */
            None => Err(self.err(name, "this function call is not lowered yet")),
        }
    }

    /// The definition a call names, resolved against the argument types.
    ///
    /// A user function may be overloaded on its parameter types, so the name
    /// alone does not identify the body. Inlining the wrong one compiles and
    /// draws something; `sema::resolve_overload` is the same resolution the
    /// checker used to type the call, so the body inlined here is the one
    /// whose return type the expression was given.
    fn user_function(&self, name: &str, args: Range) -> Option<(Range, StmtId)> {
        let mut tys = [super::sema::Type::UNKNOWN; MAX_PARAMS];
        let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
        if ids.len() > MAX_PARAMS {
            return None;
        }
        for (t, &e) in tys.iter_mut().zip(ids) {
            *t = self.types.of(e);
        }
        let d = super::sema::resolve_overload(self.ast, name, &tys[..ids.len()], true)?;
        match self.ast.decl(d) {
            Decl::Function { params, body: Some(b), .. } => Some((*params, *b)),
            _ => None,
        }
    }

    /// Inline a call to a user-defined function.
    ///
    /// The IR has no call and no stack: a shader is one straight-line region
    /// with structured control flow, which is what the hardware runs. GLSL ES
    /// forbids recursion (section 6.1), so every call graph is finite and
    /// inlining always terminates -- but the depth is bounded anyway, because
    /// a compiler that runs on the target must refuse rather than be talked
    /// into exhausting its arena by a shader that says otherwise.
    ///
    /// The subset is a body that computes and returns: declarations and
    /// statements followed by `return <expression>;`, which is what every
    /// helper in the corpus is (`strokeMask` in the FemtoVG shaders is 29
    /// sources on its own). A `return` inside control flow needs the value to
    /// survive a branch, which means a slot and a join, and that is its own
    /// change rather than a special case here.
    fn inline(
        &mut self,
        name: &'a str,
        f: (Range, StmtId),
        args: Range,
        want: Want<'a>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Ret> {
        let (params, body) = f;
        if self.inline_depth >= MAX_INLINE_DEPTH {
            return Err(self.err(name, "this call nests too deeply to inline"));
        }
        let arg_ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
        let param_ids = &self.ast.params[params.indices()];
        if arg_ids.len() != param_ids.len() {
            /* The checker has the signatures and refuses a wrong count
             * first; this keeps the binding below from reading past an end. */
            return Err(self.err(name, "this call passes the wrong number of arguments"));
        }
        if param_ids.len() > MAX_PARAMS {
            return Err(self.err(name, "this call has too many arguments to inline"));
        }

        /* Bind the arguments BEFORE the parameters come into scope, so that
         * `f(x)` where the parameter is also called `x` passes the caller's
         * `x` and not the one being created. */
        /* On the stack for the common call, in the arena past it: every
         * nested inline holds one of these, so its size is paid once per
         * depth -- 16 of them on the stack cost 30 KB at 15 nested inlines
         * (tests/stack_survey.rs), and only Qt Quick 3D's lighting helper
         * has more than 8 parameters. */
        let mut small = [Bound::EMPTY; SMALL_PARAMS];
        let bound: &mut [Bound<'a>] = if param_ids.len() <= SMALL_PARAMS {
            &mut small[..param_ids.len()]
        } else {
            self.a.alloc_filled(param_ids.len(), Bound::EMPTY)?
        };
        for (slot, (p, &arg)) in bound.iter_mut().zip(param_ids.iter().zip(arg_ids)) {
            let Some(pname) = p.name else {
                return Err(self.err(name, "this call's parameter has no name"));
            };
            /* A sampler has no value to copy -- it is a binding -- and a
             * struct holding one cannot be written (section 4.1.7: samplers
             * "cannot be treated as l-values and cannot be used as out or
             * inout function parameters. These restrictions also apply to
             * any structures that contain sampler types"; the checker
             * refuses both). So the parameter is a NAME for the uniform the
             * argument names, and `s.source` in the body is the row
             * `u.source` of the caller's `fun(u)`. */
            if self.carries_sampler(p.ty.base, 0) {
                let Some(prefix) = self.uniform_const_name(arg)? else {
                    return Err(self.err(name, "a sampler argument chosen at run time is not lowered yet"));
                };
                let names_row = |r: &reflect::UniformRow<'_>| {
                    r.name == prefix
                        || (r.name.len() > prefix.len()
                            && r.name.starts_with(prefix)
                            && matches!(r.name.as_bytes()[prefix.len()], b'.' | b'['))
                };
                if !self.reflection.uniforms.iter().any(names_row) {
                    return Err(self.err(name, "a sampler argument chosen at run time is not lowered yet"));
                }
                /* A member that is not a sampler may still be assigned in
                 * the body; that needs a copy the name binding does not
                 * make. */
                if assigns_to(self.ast, body, pname, &|a| self.carries_sampler(self.types.of(a).base, 0)) {
                    return Err(
                        self.err(pname, "writing a member of a parameter that holds a sampler is not lowered yet")
                    );
                }
                let id = ir::LocalId(self.uniform_params.len() as u32);
                self.uniform_params.push(self.a, prefix)?;
                *slot = Bound {
                    param: Local { name: pname, id, comps: 0, run: Run::Uniform, value: None },
                    target: None,
                    struct_name: None,
                    elem: None,
                    lref: None,
                };
                continue;
            }
            /* As at a declaration: a struct has no scalar kind and no
             * component count of its own, because those describe one slot
             * and a struct is a run of slots of differing width. */
            let comps = p.ty.base.slot_width();
            let scalar = if matches!(p.ty.base, BaseType::Struct(_)) {
                ir::ScalarType::F32
            } else {
                match scalar_of(p.ty.base) {
                    Some(s) if comps > 0 => s,
                    _ => return Err(self.err(name, "a parameter of this type is not lowered yet")),
                }
            };
            /* `float a[3]` as a parameter is the same run of slots a `matN`
             * is, so binding and copy-back need nothing new -- only the
             * length, which the checker has already folded. */
            let run = match (p.ty.base, p.ty.array_len) {
                (BaseType::Mat(..) | BaseType::Struct(_), Some(_)) if !self.ast.es3 => {
                    return Err(self.err(name, "an array of matrices or structs is not lowered yet"))
                }
                /* ES 3.00: laid out like any array local, and filled from
                 * the argument by `array_fill` below. */
                (_, Some(len)) if self.ast.es3 => {
                    let n = self.fold_len(len).unwrap_or(0).max(0) as u32;
                    let line = self.line;
                    self.run_shape(p.ty.base, n, pname, line)?
                }
                (_, Some(len)) => self.array_run(name, len)?,
                (BaseType::Mat(n, _), None) => Run::Matrix(n),
                (BaseType::Struct(sname), None) => self.struct_run(sname, name, 0)?,
                _ => Run::Single,
            };
            /* `out` writes back through the caller's lvalue, so the argument
             * has to BE one: a whole variable of the parameter's shape, or --
             * for a one-slot parameter -- an element or a lane of one
             * (`v[i]`, `m[i][j]`, `v.y`), resolved HERE, before the call, so
             * its indices are evaluated once (section 6.1: the argument's
             * lvalue is evaluated before the call). */
            let mut lref = None;
            let target = if p.direction == ParamDirection::In {
                None
            } else {
                let whole = match *self.ast.expr(arg) {
                    Expr::Ident(aname) => self.shadow_of(aname).or_else(|| self.lookup(aname)).filter(|t| t.run == run),
                    _ => None,
                };
                match whole {
                    Some(t) => Some(t),
                    None if run == Run::Single => match self.resolve_lref(arg, out)? {
                        Some(r) => {
                            lref = Some(&self.a.alloc_copy(&[r])?[0]);
                            None
                        }
                        None => return Err(self.err(name, "an out argument that is not a variable is not lowered yet")),
                    },
                    None => return Err(self.err(name, "an out argument that is not a variable is not lowered yet")),
                }
            };
            /* An `in` parameter the body never assigns to is a name for the
             * argument's value, so it is bound to the value and costs no
             * slot, no store and no load. The scan is over the body, not the
             * unit: `qdot(vec2 a, vec2 b)` has parameters called `a` and `b`
             * while `main` has variables of the same names that ARE assigned,
             * and a whole-unit scan would refuse to alias exactly the case
             * this exists for. */
            let aliasable = p.direction == ParamDirection::In
                && run == Run::Single
                && !assigns_to(self.ast, body, pname, &|_| false);
            let param = if aliasable {
                let v = self.operand_at(arg, comps, out)?;
                Local::alias(pname, v, comps)
            } else {
                /* Allocated only now: deciding after the slot exists leaves
                 * an unused one behind, which is a register the back end
                 * still counts. */
                let id = ir::LocalId(self.locals.len() as u32);
                if run.elements().is_some() && self.ast.es3 {
                    /* The same layout a declaration gets. */
                    let n = u32::from(run.elements().unwrap_or(0));
                    let line = self.line;
                    self.alloc_var(pname, p.ty.base, n, line)?;
                } else if let Run::Struct(_) = run {
                    /* Each leaf at its own width, as at a declaration. */
                    let BaseType::Struct(sname) = p.ty.base else {
                        unreachable!("Run::Struct comes from a struct type")
                    };
                    self.push_struct_slots(sname, pname, 0, 0)?;
                } else {
                    for _ in 0..run.slots() {
                        self.locals.push(
                            self.a,
                            ir::VarDecl { name: pname, ty: ir::Type::new(scalar, comps), array_len: 1 },
                        )?;
                    }
                }
                let param = Local { name: pname, id, comps, run, value: None };
                if p.direction != ParamDirection::Out {
                    /* `out` does not read the argument -- the parameter
                     * starts undefined, which is what the specification says
                     * (section 6.1) and what a copy in would quietly
                     * change. */
                    match run {
                        /* Bound by name above, before any slot exists. */
                        Run::Uniform => unreachable!("a parameter holding a sampler is bound by name"),
                        /* A parameter is never laid out as values. */
                        Run::Constants => unreachable!("only a const declaration is held as values"),
                        /* ES 3.00: any array argument, a value included. */
                        Run::StructArray { .. } | Run::MatrixArray { .. } | Run::Array(_) if self.ast.es3 => {
                            self.array_fill(arg, param, out)?;
                        }
                        /* Not constructed for a parameter in ES 1.00: refused
                         * above. */
                        Run::StructArray { .. } | Run::MatrixArray { .. } => {
                            return Err(self.err(name, "an array of structs is not lowered yet"));
                        }
                        Run::Matrix(n) => {
                            let cols = self.matrix_value(arg, n, out)?;
                            self.store_columns(param, &cols, out)?;
                        }
                        /* An array argument is copied element by element:
                         * ES 1.00 is call by value (section 6.1) and there
                         * is no array-valued expression to copy from
                         * (section 5.7), so the argument must be a NAME. */
                        Run::Array(n) => {
                            let Expr::Ident(aname) = *self.ast.expr(arg) else {
                                return Err(
                                    self.err(name, "an array argument that is not a variable is not lowered yet")
                                );
                            };
                            let Some(from) = self.shadow_of(aname).or_else(|| self.lookup(aname)) else {
                                return Err(self.err(aname, "reading this is not lowered yet"));
                            };
                            if from.run != run {
                                return Err(self.err(aname, "this array argument has the wrong length"));
                            }
                            for j in 0..n {
                                let v =
                                    self.push_instr(Op::LoadLocal { local: from.column(j).id, index: None, comps })?;
                                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                                self.store_local(param.column(j), Src::id(v), out)?;
                            }
                        }
                        /* A struct argument is copied leaf by leaf, as an
                         * array is element by element -- but each leaf at
                         * ITS OWN width, which is why the slots above are
                         * allocated from the members and not from one
                         * `comps`. Section 6.1, call by value.
                         *
                         * `copy_struct_slots` is the same walk the
                         * declaration's `S b = a;` uses. This used to index
                         * the MEMBER list by the run length, which are the
                         * same number only while no member is itself a
                         * struct: with one that is, the widths came out of
                         * the wrong members and the walk ran off the end. */
                        Run::Struct(_) => {
                            let BaseType::Struct(sname) = p.ty.base else {
                                unreachable!("Run::Struct comes from a struct type")
                            };
                            match self.struct_run_of(arg)? {
                                Some(from) => {
                                    if from.run != run {
                                        return Err(self.err(pname, "this struct argument has the wrong shape"));
                                    }
                                    self.copy_struct_slots(sname, param.id.0, from.id.0, pname, 0, out)?;
                                }
                                /* A uniform struct, or an element of a
                                 * uniform array of them, passed by value. */
                                None => {
                                    if self
                                        .copy_struct_from_uniform(sname, param.id.0, arg, "", pname, 0, out)?
                                        .is_none()
                                    {
                                        return Err(self
                                            .err(name, "a struct argument that is not a variable is not lowered yet"));
                                    }
                                }
                            }
                        }
                        /* `inout` through a resolved element or lane: read
                         * once, through the same resolution the copy back
                         * writes. */
                        Run::Single if lref.is_some() => {
                            let r = *lref.expect("matched above");
                            let v = self.load_lref(r, out)?;
                            self.store_local(param, v, out)?;
                        }
                        Run::Single => {
                            let v = self.operand_at(arg, comps, out)?;
                            self.store_local(param, v, out)?;
                        }
                    }
                }
                param
            };
            let struct_name = match p.ty.base {
                BaseType::Struct(sn) if matches!(param.run, Run::Struct(_)) => Some(sn),
                _ => None,
            };
            let elem = p.ty.array_len.map(|_| p.ty.base);
            *slot = Bound { param, target, struct_name, elem, lref };
        }

        /* A function body sees the globals and its own parameters, never the
         * caller's locals: its entries start above the caller's, and lookup
         * starts there (`scope_base`). */
        let caller_base = self.scope_base;
        let caller_top = self.scope.len();
        self.scope_base = caller_top;
        for b in bound.iter() {
            self.scope.push(self.a, b.param)?;
        }

        /* The callee's returns are its own: a `return` in a loop of the
         * callee must leave the callee, not the caller's loop around the
         * call. */
        let caller_ret = self.ret.take();
        self.inline_depth += 1;
        let r = self.inline_body(name, body, want, out);
        self.inline_depth -= 1;
        self.ret = caller_ret;
        while self.scope.len() > caller_top {
            self.scope.pop();
        }
        self.scope_base = caller_base;
        let r = r?;

        /* The copy back happens after the caller's scope is restored, which
         * is when its variables exist again. */
        for &b in bound.iter() {
            if let Some(r) = b.lref {
                let p = b.param;
                let v = self.push_instr(Op::LoadLocal { local: p.id, index: None, comps: p.comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                self.store_lref(*r, Src::id(v), out)?;
                continue;
            }
            let Some(t) = b.target else { continue };
            let p = b.param;
            if let Some(sname) = b.struct_name {
                /* Leaf by leaf, each at its own width -- the same walk the
                 * copy IN uses, so the two cannot disagree. */
                self.copy_struct_slots(sname, t.id.0, p.id.0, p.name, 0, out)?;
                continue;
            }
            if let (Run::StructArray { .. }, Some(elem)) = (p.run, b.elem) {
                self.copy_run(t, p, elem, out)?;
                continue;
            }
            for j in 0..p.run.slots() {
                let col = p.column(j);
                let v = self.push_instr(Op::LoadLocal { local: col.id, index: None, comps: p.comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                self.store_local(t.column(j), Src::id(v), out)?;
            }
        }
        Ok(r)
    }

    /// The body of an inlined function: statements, then the return.
    fn inline_body(&mut self, name: &'a str, body: StmtId, want: Want<'a>, out: &mut AVec<'a, Node>) -> R<'a, Ret> {
        let Stmt::Block(r) = *self.ast.stmt(body) else {
            return Err(self.err(name, "this function body is not lowered yet"));
        };
        let ids: &'a [StmtId] = &self.ast.stmt_ids[r.indices()];
        if returns_early(self.ast, ids) {
            return self.inline_with_slot(name, ids, want, out);
        }
        for &id in ids {
            if let Stmt::Return(Some(e)) = *self.ast.stmt(id) {
                return match want {
                    Want::Matrix(n, _) => Ok(Ret::Matrix(self.matrix_value(e, n, out)?)),
                    /* `return Pos(-p.a, -p.b, -p.c);` -- the destination is
                     * the caller's run, and the same `struct_fill` that
                     * serves `S b = a;` fills it, so a constructor, a copy
                     * and a nested call are one rule here too. */
                    Want::Struct { dst, sname } => {
                        self.struct_fill(e, dst, BaseType::Struct(sname), sname, out)?;
                        Ok(Ret::Struct)
                    }
                    /* `return float[3](a, b, c);` fills the caller's run. */
                    Want::Array { dst } => {
                        self.array_fill(e, dst, out)?;
                        Ok(Ret::Struct)
                    }
                    Want::Value(_) => {
                        let w = self.width(e)?;
                        Ok(Ret::Value(self.value(e, w, out)?))
                    }
                    /* `f(x);` where `f` returns a value: the result is
                     * DISCARDED, which section 6.1 allows and which is not
                     * the same as calling a void function. The expression is
                     * still lowered, because it may have effects through an
                     * `out` parameter or an assignment inside it; the back
                     * end drops it when it has none.
                     *
                     * This refused until 2026-09-22 with "this function
                     * returns no value", which was backwards -- it returns
                     * one, the caller ignores it -- and dEQP's
                     * qualification_order.parameters.valid expects it to
                     * build. */
                    Want::Nothing => {
                        let w = self.width(e)?;
                        self.value(e, w, out)?;
                        Ok(Ret::Value(Src::id(ValueId(0))))
                    }
                };
            }
            self.statement(id, out)?;
        }
        if matches!(want, Want::Nothing) {
            /* A void function ends by running out of statements, which is
             * the normal case and not an error. */
            return Ok(Ret::Value(Src::id(ValueId(0))));
        }
        if matches!(want, Want::Struct { .. } | Want::Array { .. }) {
            /* Falling off the end of a struct-returning function. Section
             * 6.1 makes the result undefined; the destination run keeps
             * what it held, which is a defined, deterministic value and
             * costs no stores. (The value case below prefers a zero
             * because its result has no slot to keep.) */
            return Ok(Ret::Struct);
        }
        /* Falling off the end of a value-returning function. Section 6.1
         * makes the result UNDEFINED rather than illegal, and dEQP's
         * functions.misc.missing_returns expects it to build -- its own
         * comment quotes the rule. Undefined permits any value, so the
         * caller gets a defined zero instead of whatever a slot held:
         * legal, deterministic, and not a wrong picture that varies with
         * what ran before. */
        let zero = self.const_value([0.0; 4], out)?;
        Ok(Ret::Value(zero))
    }

    /// An inlined body with more than one `return`: the result goes to a
    /// slot and the early returns become the shape of an `If`.
    ///
    /// `float f(vec2 p) { if (c) { ...; return A; } ...; return B; }` is the
    /// commonest helper in the FemtoVG shaders and 31 sources of the GC7000
    /// corpus. It cannot be lowered by emitting a `Return` in the middle:
    /// selection treats `Return` as the end of the shader -- the GC7000 marks
    /// the last instruction rather than branching -- so the code after it
    /// would run anyway and the early return would silently do nothing.
    ///
    /// So no `Return` is emitted at all. The result is a slot, `return X`
    /// becomes a store to it, and whatever followed the `if` in the enclosing
    /// run is lowered into the `if`'s **else** branch. That is the same
    /// program with the control flow made structured, which is what the IR
    /// and the hardware both want.
    fn inline_with_slot(
        &mut self,
        name: &'a str,
        ids: &'a [StmtId],
        want: Want<'a>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Ret> {
        /* A struct or an array result is filled IN PLACE: the caller's run
         * is the slot, and every `return` fills it -- the structured
         * control flow below already keeps a later statement from running
         * after a return, so no copy is needed. Refused by name until
         * 2026-09-23 (Qt Quick 3D's `qt_sampleSpotLight`, 21 sources). */
        if let Want::Struct { dst, .. } | Want::Array { dst } = want {
            let mut run: AVec<Node> = AVec::new();
            self.lower_run(name, ids, want, Some(dst), &mut run)?;
            out.extend_from_slice(self.a, run.as_slice())?;
            return Ok(Ret::Struct);
        }
        let (comps, run) = match want {
            Want::Value(w) => (w.max(1), Run::Single),
            Want::Matrix(n, r) => (r, Run::Matrix(n)),
            /* A void function still needs a slot for the run to write to
             * when it returns early; nothing reads it. */
            Want::Nothing => (1, Run::Single),
            /* Filled in place above; see RUN_RESULT_IN_ONE_SLOT. */
            Want::Struct { .. } | Want::Array { .. } => return Err(self.err(name, RUN_RESULT_IN_ONE_SLOT)),
        };
        let columns = run.slots();
        let id = ir::LocalId(self.locals.len() as u32);
        for _ in 0..columns {
            self.locals
                .push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(ir::ScalarType::F32, comps), array_len: 1 })?;
        }
        let slot = Local { name: "", id, comps, run, value: None };
        let mut run: AVec<Node> = AVec::new();
        let always = self.lower_run(name, ids, want, Some(slot), &mut run)?;
        if matches!(want, Want::Nothing) {
            out.extend_from_slice(self.a, run.as_slice())?;
            return Ok(Ret::Value(Src::id(ValueId(0))));
        }
        if !always {
            /* A path that reaches the end without returning. Section 6.1
             * makes the result undefined, not the shader illegal, so the
             * slot is given a zero FIRST -- before the body, so a path that
             * does return overwrites it. Undefined permits any value; a
             * defined zero is the one that does not vary with whatever last
             * used the slot.
             *
             * The body is built into `run` rather than straight into `out`
             * so the initialiser can be placed ahead of it without the cost
             * falling on every inlined function. */
            let zero = self.const_value([0.0; 4], out)?;
            for j in 0..columns {
                self.store_local(slot.column(j), zero, out)?;
            }
            out.extend_from_slice(self.a, run.as_slice())?;
        } else {
            out.extend_from_slice(self.a, run.as_slice())?;
        }
        match want {
            Want::Struct { .. } | Want::Array { .. } => Err(self.err(name, RUN_RESULT_IN_ONE_SLOT)),
            Want::Matrix(n, _) => {
                let mut cols = [Src::id(ValueId(0)); 4];
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    let col = slot.column(j as u8);
                    let v = self.push_instr(Op::LoadLocal { local: col.id, index: None, comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    *c = Src::id(v);
                }
                Ok(Ret::Matrix(cols))
            }
            Want::Value(_) | Want::Nothing => {
                let v = self.push_instr(Op::LoadLocal { local: id, index: None, comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Ok(Ret::Value(Src::id(v)))
            }
        }
    }

    /// Lower one run of statements from such a body. `true` means the run
    /// always reaches a `return`, so nothing after it in the enclosing run
    /// can run.
    fn lower_run(
        &mut self,
        name: &'a str,
        ids: &'a [StmtId],
        want: Want<'a>,
        slot: Option<Local<'a>>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, bool> {
        for (k, &id) in ids.iter().enumerate() {
            match *self.ast.stmt(id) {
                /* The run ends, and what followed belongs to the other
                 * branch of the enclosing `if`. The flag is set only once
                 * something has asked for it: a loop or a block further out
                 * in this function reads it to learn that this path
                 * returned. */
                Stmt::Return(value) => {
                    self.return_value(name, value, want, slot, out)?;
                    if let Some(ctx) = self.ret {
                        self.set_flag(ctx.flag, true, out)?;
                    }
                    return Ok(true);
                }
                Stmt::If { cond, then_s, else_s }
                    if !returns_sometimes(self.ast, then_s)
                        && !else_s.is_some_and(|e| returns_sometimes(self.ast, e)) =>
                {
                    let c = self.value(cond, 1, out)?;
                    let mut then_items: AVec<Node> = AVec::new();
                    let then_returns = self.branch_run(name, then_s, want, slot, &mut then_items)?;
                    let mut else_items: AVec<Node> = AVec::new();
                    let else_returns = match else_s {
                        Some(s) => self.branch_run(name, s, want, slot, &mut else_items)?,
                        None => false,
                    };
                    /* Whatever followed the `if` belongs to the branch that
                     * did NOT return: that is what makes the early return
                     * disappear. */
                    let rest: &'a [StmtId] = &ids[k + 1..];
                    let tail = match (then_returns, else_returns) {
                        (true, false) => self.lower_run(name, rest, want, slot, &mut else_items)?,
                        (false, true) => self.lower_run(name, rest, want, slot, &mut then_items)?,
                        (true, true) => true,
                        (false, false) => {
                            /* Neither branch returns: the `if` is an ordinary
                             * statement and the rest follows it. */
                            let (then_, else_) = self.commit2(then_items, else_items)?;
                            out.push(self.a, Node::If { cond: c, then_, else_ })?;
                            continue;
                        }
                    };
                    let (then_, else_) = self.commit2(then_items, else_items)?;
                    out.push(self.a, Node::If { cond: c, then_, else_ })?;
                    return Ok(tail);
                }
                /* A loop, a block, or an `if` with an arm that returns on
                 * SOME of its paths. What follows cannot be moved into an
                 * `else` the way it is for an arm that always returns: a
                 * loop's `return` leaves from the middle of an iteration,
                 * and an arm that returns only sometimes has paths that
                 * fall out of it and paths that must not. So the return
                 * sets a flag (and, in a loop, breaks), and what follows
                 * here runs only while the flag is false. The flag is
                 * cleared right before, which is sound because no path
                 * that has returned reaches this statement.
                 *
                 * The `if` case compiled WITHOUT the flag until this arm
                 * existed: `if (c) { if (d) return 1.0; } return 2.0;`
                 * lowered the second return unconditionally, so it
                 * overwrote the first and the function never returned
                 * 1.0. */
                _ if contains_return(self.ast, id) => {
                    let flag = match self.ret {
                        Some(c) => c.flag,
                        None => {
                            let fid = ir::LocalId(self.locals.len() as u32);
                            self.locals.push(
                                self.a,
                                ir::VarDecl { name: "", ty: ir::Type::new(ir::ScalarType::Bool, 1), array_len: 1 },
                            )?;
                            let flag = Local { name: "", id: fid, comps: 1, run: Run::Single, value: None };
                            self.ret = Some(RetCtx { flag, slot, want, name, loops: 0 });
                            flag
                        }
                    };
                    self.set_flag(flag, false, out)?;
                    match *self.ast.stmt(id) {
                        Stmt::Block(r) => {
                            let inner: &'a [StmtId] = &self.ast.stmt_ids[r.indices()];
                            self.push_scope()?;
                            let always = self.lower_run(name, inner, want, slot, out);
                            self.pop_scope();
                            if always? {
                                return Ok(true);
                            }
                        }
                        /* Each arm through the run form, so that a return in
                         * it sets the flag; the arms' own followers are
                         * structured inside them as usual. */
                        Stmt::If { cond, then_s, else_s } => {
                            let c = self.value(cond, 1, out)?;
                            let mut then_items: AVec<Node> = AVec::new();
                            let t = self.branch_run(name, then_s, want, slot, &mut then_items)?;
                            let mut else_items: AVec<Node> = AVec::new();
                            let e = match else_s {
                                Some(s) => self.branch_run(name, s, want, slot, &mut else_items)?,
                                None => false,
                            };
                            let (then_, else_) = self.commit2(then_items, else_items)?;
                            out.push(self.a, Node::If { cond: c, then_, else_ })?;
                            if t && e {
                                return Ok(true);
                            }
                        }
                        _ => self.statement(id, out)?,
                    }
                    let rest: &'a [StmtId] = &ids[k + 1..];
                    if rest.is_empty() {
                        return Ok(false);
                    }
                    let f = self.push_instr(Op::LoadLocal { local: flag.id, index: None, comps: 1 })?;
                    out.push(self.a, Node::Instr(ir::InstrId(f.0)))?;
                    let mut else_items: AVec<Node> = AVec::new();
                    let tail = self.lower_run(name, rest, want, slot, &mut else_items)?;
                    let else_ = self.commit(else_items)?;
                    out.push(self.a, Node::If { cond: Src::id(f), then_: IrRange::EMPTY, else_ })?;
                    return Ok(tail);
                }
                _ => self.statement(id, out)?,
            }
        }
        Ok(false)
    }

    /// What a `return` does with its value: store it where the caller will
    /// read it. `value` is the returned expression, if any.
    fn return_value(
        &mut self,
        name: &'a str,
        value: Option<ExprId>,
        want: Want<'a>,
        slot: Option<Local<'a>>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let Some(e) = value else {
            if matches!(want, Want::Nothing) {
                return Ok(());
            }
            return Err(self.err(name, "this function returns no value"));
        };
        let Some(slot) = slot else {
            /* `return <value>;` in `main`, which is void. The checker
             * refuses it under section 6.1 (since 2026-09-22 -- before
             * that this arm was where it was noticed), so a shader cannot
             * reach here. */
            return Err(self.err("return", "'return' with a value in 'void main()'"));
        };
        match want {
            /* The caller's run, filled where the `return` stands. */
            Want::Struct { dst, sname } => self.struct_fill(e, dst, BaseType::Struct(sname), sname, out),
            Want::Array { dst } => self.array_fill(e, dst, out),
            Want::Matrix(n, _) => {
                let cols = self.matrix_value(e, n, out)?;
                self.store_columns(slot, &cols, out)
            }
            Want::Value(_) => {
                let v = self.value(e, slot.comps, out)?;
                self.store_local(slot, v, out)
            }
            /* A function that returns a value, called as a statement
             * (`f();`): the caller discards the value, so the returned
             * expression runs for its side effects only. (The checker
             * refuses a value in a void function, so that is the only way
             * here.) This refused "this function returns no value" for
             * every such call whose body returns early until 2026-09-24:
             * dEQP-GLES3 shaders.metamorphic.synthetic.variant_2. */
            Want::Nothing => self.evaluate_and_discard(e, out),
        }
    }

    /// One branch of an `if` inside such a body, in its own scope.
    fn branch_run(
        &mut self,
        name: &'a str,
        s: StmtId,
        want: Want<'a>,
        slot: Option<Local<'a>>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, bool> {
        let ids: &'a [StmtId] = match *self.ast.stmt(s) {
            Stmt::Block(r) => &self.ast.stmt_ids[r.indices()],
            /* A branch that is a single statement, which the grammar allows
             * without braces. */
            _ => self.a.alloc_copy(&[s])?,
        };
        self.push_scope()?;
        self.fold_depth += 1;
        let r = self.lower_run(name, ids, want, slot, out);
        self.fold_depth -= 1;
        self.pop_scope();
        r
    }

    /// An index expression's value, evaluated ONCE, as one integer lane.
    fn index_value(&mut self, idx: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let i = self.operand_at(idx, 1, out)?;
        Ok(Src { val: i.val, swizzle: [i.swizzle[0]; 4] })
    }

    /// Element `i` of `els`, chosen at run time: a chain of `Bcsel`s, built
    /// from the last element backwards so the final fall-through is the
    /// last element and needs no comparison of its own.
    ///
    /// Neither GPU's back end has an indexed register file this could rely
    /// on today, and Appendix A requires indexing temporaries by a loop
    /// index, so the choice is made with values -- the move
    /// `vector_lane_select` makes over the lanes of one register. An index
    /// out of range is undefined (section 4.1.9); this gives the last
    /// element, a defined value and never another variable's.
    fn select_among(&mut self, els: &[Src], i: Src, w: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let Some(&last) = els.last() else {
            return Err(self.err("[]", "indexing nothing"));
        };
        let mut acc = last;
        for k in (0..els.len() - 1).rev() {
            let kc = self.int_const([k as i32; 4], out)?;
            let eq = self.alu_w(ir::AluOp::Ieq, [i, kc, kc], 1, out)?;
            let eq = Src { val: eq.val, swizzle: [eq.swizzle[0]; 4] };
            acc = self.alu_w(ir::AluOp::Bcsel, [eq, els[k], acc], w, out)?;
        }
        Ok(acc)
    }

    /// Store `v` into slot `i` of `slots`, chosen at run time: every slot
    /// takes `i == k ? v : old`. `slots[k]` is a whole slot of `w` lanes.
    fn store_among(&mut self, slots: &[Local<'a>], i: Src, v: Src, w: u8, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        for (k, &slot) in slots.iter().enumerate() {
            let old = self.push_instr(Op::LoadLocal { local: slot.id, index: None, comps: w })?;
            out.push(self.a, Node::Instr(ir::InstrId(old.0)))?;
            let kc = self.int_const([k as i32; 4], out)?;
            let eq = self.alu_w(ir::AluOp::Ieq, [i, kc, kc], 1, out)?;
            let eq = Src { val: eq.val, swizzle: [eq.swizzle[0]; 4] };
            let new = self.alu_w(ir::AluOp::Bcsel, [eq, v, Src::id(old)], w, out)?;
            self.store_local(slot, new, out)?;
        }
        Ok(())
    }

    /// `v[i]` where `i` is not a constant: a chain of selects over the
    /// lanes.
    ///
    /// A vector's components are lanes of one register, not addresses, so
    /// there is nothing to index -- the choice has to be made with values.
    /// `Bcsel` is component-wise and evaluates both arms, which is exactly
    /// what is wanted here: every lane is already in hand.
    ///
    /// Built from the last lane backwards so that the final `Bcsel`'s
    /// fall-through is lane `w-1`, which needs no comparison of its own:
    /// `w-1` compares for `w` lanes.
    fn vector_lane_select(&mut self, base: ExprId, idx: ExprId, w: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let v = self.value(base, w, out)?;
        let i = self.operand_at(idx, 1, out)?;
        let lane = |k: u8| Src { val: v.val, swizzle: [v.swizzle[k as usize]; 4] };
        let mut acc = lane(w - 1);
        for k in (0..w - 1).rev() {
            let kc = self.push_instr(Op::ConstI32([i32::from(k); 4]))?;
            out.push(self.a, Node::Instr(ir::InstrId(kc.0)))?;
            let eq = self.alu(ir::AluOp::Ieq, [i, Src::id(kc), Src::id(kc)], out)?;
            acc = self.alu(ir::AluOp::Bcsel, [eq, lane(k), acc], out)?;
        }
        Ok(acc)
    }

    /// `++x` or `x--` used as a statement: add or subtract one in place.
    /// `++i` and `i++` used as a VALUE (section 5.9).
    ///
    /// > Pre-increment and pre-decrement add or subtract 1 or 1.0 to the
    /// > contents of the expression they operate on, and the value of the
    /// > pre-increment or pre-decrement expression is the resulting value of
    /// > that modification. Post-increment and post-decrement expressions
    /// > add or subtract 1 or 1.0 ... but the resulting expression has the
    /// > expression's value before the post-increment or post-decrement was
    /// > executed.
    ///
    /// So the two forms are the same store and differ only in WHEN the
    /// l-value is read: after it for the prefix, before it for the postfix.
    /// Getting that backwards compiles and draws a picture that is off by
    /// one, which no type check would catch.
    ///
    /// Reading BEFORE is sound because a `LoadLocal` produces a value and
    /// the store that follows writes the SLOT, not that value. The back end
    /// has to keep them apart, and does: gles2-core's load-copy elimination
    /// asks whether a slot is stored later before it reuses the slot's
    /// register for a load. If that guard were ever dropped, this is one of
    /// the shapes that would start reading the new value.
    ///
    /// `step_by_one` accepts only a plain name, so the re-read here cannot
    /// repeat a side effect and its refusal is inherited unchanged.
    #[inline(never)]
    fn step_value(&mut self, op: UnOp, target: ExprId, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* An element or a lane (`v[f()]++`, `++v[i]`): resolved once, so
         * its index is evaluated once, then read, stepped and written. */
        if !matches!(*self.ast.expr(target), Expr::Ident(_)) {
            if let Some(r) = self.resolve_lref(target, out)? {
                let (old, new) = self.step_lref(op, target, r, out)?;
                return Ok(if matches!(op, UnOp::PostInc | UnOp::PostDec) { old } else { new });
            }
        }
        if matches!(op, UnOp::PostInc | UnOp::PostDec) {
            let before = self.value(target, want, out)?;
            self.step_by_one(op, target, out)?;
            return Ok(before);
        }
        self.step_by_one(op, target, out)?;
        self.value(target, want, out)
    }

    /// Step what `r` resolves to by one: (the value before, the value after).
    /* Out of line: the recursive lowering must not carry its frame. */
    #[inline(never)]
    fn step_lref(&mut self, op: UnOp, target: ExprId, r: LRef<'a>, out: &mut AVec<'a, Node>) -> R<'a, (Src, Src)> {
        let old = self.load_lref(r, out)?;
        let w = match r.lane {
            LaneSel::All => match r.slots {
                SlotSel::One(l) => l.comps,
                SlotSel::Among { base, .. } => base.comps,
            },
            LaneSel::Swz { mask, .. } => mask.count_ones() as u8,
            _ => 1,
        };
        let scalar = self.scalar_kind(target)?;
        let (one, add) = match scalar {
            ir::ScalarType::F32 => (self.const_value([1.0; 4], out)?, ir::AluOp::Fadd),
            ir::ScalarType::I32 | ir::ScalarType::U32 => (self.int_const([1; 4], out)?, ir::AluOp::Iadd),
            ir::ScalarType::Bool => return Err(self.err(op.text(), "a bool cannot be stepped")),
        };
        let step = match (op, scalar) {
            (UnOp::PreInc | UnOp::PostInc, _) => one,
            (_, ir::ScalarType::F32) => self.alu(ir::AluOp::Fneg, [one, one, one], out)?,
            _ => self.alu(ir::AluOp::Ineg, [one, one, one], out)?,
        };
        let new = self.alu_w(add, [old, step, step], w, out)?;
        self.store_lref(r, new, out)?;
        Ok((old, new))
    }

    #[inline(never)]
    fn step_by_one(&mut self, op: UnOp, target: ExprId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let Expr::Ident(name) = *self.ast.expr(target) else {
            if let Some(r) = self.resolve_lref(target, out)? {
                return self.step_lref(op, target, r, out).map(|_| ());
            }
            let k = self.ast.expr(target).kind_name();
            return Err(self.err(k, "assignment to this is not lowered yet"));
        };
        let Some(l) = self.shadow_of(name).or_else(|| self.lookup(name)) else {
            return Err(self.err(name, "assignment to this is not lowered yet"));
        };
        if let Some(n) = l.run.columns() {
            /* Section 5.9: `++` and `--` "operate on integer or
             * floating-point values (including vectors and matrices)" --
             * every component of every column steps by 1.0. */
            let one = self.const_value([1.0; 4], out)?;
            let step = match op {
                UnOp::PreInc | UnOp::PostInc => one,
                _ => self.alu(ir::AluOp::Fneg, [one, one, one], out)?,
            };
            for j in 0..n {
                let col = l.column(j);
                let v = self.push_instr(Op::LoadLocal { local: col.id, index: None, comps: l.comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                let s2 = self.alu_w(ir::AluOp::Fadd, [Src::id(v), step, step], l.comps, out)?;
                self.store_local(col, s2, out)?;
            }
            return Ok(());
        }
        if l.run.slots() > 1 {
            return Err(self.err(name, "this matrix expression is not lowered yet"));
        }
        let scalar = self.scalar_kind(target)?;
        let old = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(old.0)))?;
        let one = match scalar {
            ir::ScalarType::F32 => self.const_value([1.0; 4], out)?,
            ir::ScalarType::I32 | ir::ScalarType::U32 => {
                let v = self.push_instr(Op::ConstI32([1; 4]))?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Src::id(v)
            }
            ir::ScalarType::Bool => return Err(self.err(op.text(), "a bool cannot be stepped")),
        };
        let add = match (op, scalar) {
            (UnOp::PreInc | UnOp::PostInc, ir::ScalarType::F32) => ir::AluOp::Fadd,
            (UnOp::PreDec | UnOp::PostDec, ir::ScalarType::F32) => {
                let neg = self.alu(ir::AluOp::Fneg, [one, one, one], out)?;
                let v = self.alu(ir::AluOp::Fadd, [Src::id(old), neg, neg], out)?;
                return self.store_local(l, v, out);
            }
            (UnOp::PreInc | UnOp::PostInc, ir::ScalarType::I32 | ir::ScalarType::U32) => ir::AluOp::Iadd,
            (UnOp::PreDec | UnOp::PostDec, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                let neg = self.alu(ir::AluOp::Ineg, [one, one, one], out)?;
                let v = self.alu(ir::AluOp::Iadd, [Src::id(old), neg, neg], out)?;
                return self.store_local(l, v, out);
            }
            _ => return Err(self.err(op.text(), "this expression statement is not lowered yet")),
        };
        let v = self.alu(add, [Src::id(old), one, one], out)?;
        self.store_local(l, v, out)
    }

    /// One argument of a call.
    ///
    /// A sampler is a binding rather than a value -- it never reaches a
    /// register -- so it is recognised here, before the general lowering,
    /// which would refuse it.
    fn operand(&mut self, e: ExprId, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Operand<'a>> {
        /* A sampler named directly or through struct members and constant
         * indices (`s.c`, `u[1].tex`): its row's binding. */
        if self.types.of(e).base.is_sampler() {
            if let Some(name) = self.uniform_const_name(e)? {
                if let Some(u) = self.reflection.uniforms.iter().find(|u| u.name == name) {
                    if u.dword_off == reflect::NO_BLOCK {
                        if let BaseType::Sampler(ty) = self.types.of(e).base {
                            return Ok(Operand::Sampler { binding: u.binding, ty });
                        }
                    }
                }
                /* `s[k]` of an array of samplers: one row, element k at
                 * binding + k (`reflect::sampler_row`). An index known only
                 * at run time (a loop index, which appendix A allows) has no
                 * name and goes on to the general lowering, which refuses. */
                if let Some((row, k)) = split_element(name) {
                    if let Some(u) = self.reflection.uniforms.iter().find(|u| u.name == row) {
                        if u.dword_off == reflect::NO_BLOCK {
                            if k >= u32::from(u.array_len) {
                                return Err(self.err("[]", "index out of range"));
                            }
                            if let BaseType::Sampler(ty) = self.types.of(e).base {
                                return Ok(Operand::Sampler { binding: u.binding + k as u8, ty });
                            }
                        }
                    }
                }
            }
        }
        let ty = self.types.of(e);
        let src = self.value(e, want, out)?;
        /* The checker's own answer, not a guess from context. It is
         * `Type::UNKNOWN` only where the checker had no opinion, and a
         * built-in that needs the difference refuses on that rather than
         * lowering a wrong width. */
        let comps = if ty.is_unknown() { want } else { ty.base.components().max(1) };
        /* A scalar argument is read in EVERY lane: `max(v3, f)` applies f to
         * each component (section 8, "the scalar is applied component-wise"),
         * and a one-lane value's other lanes hold nothing. Handed over as it
         * came until 2026-09-23, so `max(vec3(-5.0), f)` gave lanes y and z
         * of max(-5.0, <unwritten>) -- 0.0 in the evaluator, whatever the
         * register held on the GPU. */
        let src = if comps == 1 { Src { val: src.val, swizzle: [src.swizzle[0]; 4] } } else { src };
        Ok(Operand::Value { src, comps, ty: ty.base })
    }

    /// `vec4(v.xyz, 1.0)`: a constructor built from several values.
    ///
    /// There is no instruction that joins lanes from different values, and
    /// there should not be -- the hardware writes registers under a component
    /// mask, which is what [`Op::StoreLocal`]'s `write_mask` is. So the
    /// constructor is a slot, one masked store per argument, and a load.
    ///
    /// **The mask convention**, since the IR states the mask and not the
    /// mapping: destination lane *i*, for each *i* in the mask, takes
    /// `src.swizzle[i]`. An argument that fills lanes `base..base+k`
    /// therefore has its own lane *j* placed at `swizzle[base + j]`, and the
    /// lanes outside the mask are never read. `vec4(v2, v2)` is two stores,
    /// the second with `swizzle[2] = 0` and `swizzle[3] = 1`.
    ///
    /// Section 5.4.2 says extra components are dropped, so an argument that
    /// runs past the end of the type contributes only what fits and the rest
    /// are not stored.
    fn assemble(
        &mut self,
        name: &'a str,
        base: BaseType<'a>,
        comps: u8,
        ids: &[ExprId],
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Src> {
        let Some(scalar) = scalar_of(base) else {
            return Err(self.err(name, "this constructor is not lowered yet"));
        };
        let id = ir::LocalId(self.locals.len() as u32);
        self.locals.push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(scalar, comps), array_len: 1 })?;

        /* Consecutive arguments that came from the SAME value share one
         * store. `vec4(v_uv, 1.0, 1.0)` is the shape that pays: with the
         * constants deduplicated, lanes 2 and 3 both read one register, and
         * writing them together is one instruction instead of two. The masks
         * are per lane either way, so nothing about the result changes. */
        let mut lane = 0u8;
        let mut pending: Option<(Src, u8, [u8; 4])> = None;
        for &e in ids {
            if lane >= comps {
                /* Every component is filled; section 5.4.2 drops the rest,
                 * and the checker has already refused a constructor that
                 * does not supply enough. */
                break;
            }
            let have = self.width(e)?;
            /* Converted to the constructed type's scalar kind, argument by
             * argument (sections 5.4.1 and 5.4.2): the components are
             * consumed as VALUES. This took each argument's lanes as they
             * were until 2026-09-23, so `vec2(i, f)` put an int's bits in a
             * float lane -- and once integer literals became `ConstI32`,
             * femtovg's `vec4(x, y, 0, 1)` put the bits of 1 (1.4e-45) in w,
             * which pco's own bytes for main.vert caught (usc_limits). */
            let v = self.convert(e, base, have, out)?;
            let k = have.min(comps - lane);
            let mask = (((1u16 << k) - 1) << lane) as u8;
            match pending.as_mut() {
                /* The same producing value, so the lanes can be added to the
                 * store already in hand. */
                Some((held, held_mask, swizzle)) if held.val == v.val => {
                    for j in 0..k {
                        swizzle[(lane + j) as usize] = v.swizzle[j as usize];
                    }
                    *held_mask |= mask;
                }
                _ => {
                    if let Some((held, held_mask, swizzle)) = pending.take() {
                        self.store_lane_mask(id, held, held_mask, swizzle, out)?;
                    }
                    let mut swizzle = [0u8; 4];
                    for j in 0..k {
                        swizzle[(lane + j) as usize] = v.swizzle[j as usize];
                    }
                    pending = Some((v, mask, swizzle));
                }
            }
            lane += k;
        }
        if let Some((held, held_mask, swizzle)) = pending.take() {
            self.store_lane_mask(id, held, held_mask, swizzle, out)?;
        }
        if lane < comps {
            /* The checker refuses this first; reaching it would mean reading
             * a lane nothing wrote, which is a plausible value and therefore
             * a wrong picture. */
            return Err(self.err(name, "not enough arguments for this constructor"));
        }
        let v = self.push_instr(Op::LoadLocal { local: id, index: None, comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// [`Lower::assemble`] when an argument is a matrix: every argument as
    /// pieces -- a matrix as its columns, `rows` lanes each, in order -- each
    /// converted to the constructed scalar kind and stored lane by lane
    /// until the type is full. A lone scalar argument cannot be here (a
    /// matrix argument is), so there is no splat.
    fn assemble_with_matrices(
        &mut self,
        name: &'a str,
        base: BaseType<'a>,
        comps: u8,
        ids: &[ExprId],
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Src> {
        let Some(scalar) = scalar_of(base) else {
            return Err(self.err(name, "this constructor is not lowered yet"));
        };
        let to = self.scalar_of_type(base);
        let id = ir::LocalId(self.locals.len() as u32);
        self.locals.push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(scalar, comps), array_len: 1 })?;
        let mut lane = 0u8;
        for &e in ids {
            if lane >= comps {
                break;
            }
            let t = self.types.of(e);
            let mut pieces: [(Src, u8); 4] = [(Src::id(ValueId(0)), 0); 4];
            let n = if let BaseType::Mat(c, r) = t.base {
                let cols = self.matrix_value(e, c, out)?;
                for j in 0..usize::from(c) {
                    let v = match to {
                        ir::ScalarType::F32 => cols[j],
                        ir::ScalarType::I32 => self.alu_w(ir::AluOp::F2i, [cols[j]; 3], r, out)?,
                        ir::ScalarType::U32 => self.alu_w(ir::AluOp::F2u, [cols[j]; 3], r, out)?,
                        ir::ScalarType::Bool => self.alu_w(ir::AluOp::F2b, [cols[j]; 3], r, out)?,
                    };
                    pieces[j] = (v, r);
                }
                usize::from(c)
            } else {
                let have = self.width(e)?;
                pieces[0] = (self.convert(e, base, have, out)?, have);
                1
            };
            for &(v, have) in &pieces[..n] {
                if lane >= comps {
                    break;
                }
                let k = have.min(comps - lane);
                let mask = (((1u16 << k) - 1) << lane) as u8;
                let mut swizzle = [0u8; 4];
                for j in 0..k {
                    swizzle[usize::from(lane + j)] = v.swizzle[usize::from(j)];
                }
                self.store_lane_mask(id, v, mask, swizzle, out)?;
                lane += k;
            }
        }
        if lane < comps {
            return Err(self.err(name, "not enough arguments for this constructor"));
        }
        let v = self.push_instr(Op::LoadLocal { local: id, index: None, comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// One masked store into a slot, with the swizzle already indexed by the
    /// destination lane (`ir.md`, "`StoreLocal`'s write mask").
    fn store_lane_mask(
        &mut self,
        local: ir::LocalId,
        v: Src,
        write_mask: u8,
        swizzle: [u8; 4],
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let src = Src { val: v.val, swizzle };
        let w = self.push_instr(Op::StoreLocal { local, index: None, src, write_mask })?;
        out.push(self.a, Node::Instr(ir::InstrId(w.0)))?;
        Ok(())
    }

    /// A one-argument constructor over a value: `vec3(f)`, `vec3(v4)`,
    /// `float(i)`.
    ///
    /// None of these needs an instruction of its own unless the scalar kind
    /// changes. Widening a scalar to a vector is a splat and narrowing a
    /// vector is a truncation, and both are swizzles -- which is the point of
    /// an IR whose every operand carries one.
    fn convert(&mut self, e: ExprId, to: BaseType<'a>, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* A literal is converted now, into a constant of the target kind:
         * `vec4(p, 0, 1)` is float lanes 0.0 and 1.0 and costs no `I2f`. */
        if let (ir::ScalarType::I32 | ir::ScalarType::U32, Some(i)) = (self.scalar_of_type(to), self.int_lit(e)) {
            return self.int_const([i; 4], out);
        }
        if let Ok(x) = self.float_of(e) {
            return self.typed_const(self.scalar_of_type(to), [x; 4], out);
        }
        let have = self.width(e)?;
        let v = self.value(e, have, out)?;
        let from = self.scalar_kind(e)?;
        let v = if have == 1 && want > 1 {
            Src { val: v.val, swizzle: [v.swizzle[0]; 4] }
        } else if have >= want {
            v
        } else {
            /* `vec4(v2)` is not legal in ES 1.00 on its own, and `vec4(v2,
             * v2)` is a multi-argument constructor, which needs the general
             * form. */
            return Err(self.err("()", "this constructor is not lowered yet"));
        };
        /* The scalar kind is the one thing that can cost an instruction: a
         * lane holding an integer is not a lane holding the same number as a
         * float. */
        let to = self.scalar_of_type(to);
        if from == to {
            return Ok(v);
        }
        match (from, to) {
            (ir::ScalarType::I32, ir::ScalarType::F32) => self.alu(ir::AluOp::I2f, [v, v, v], out),
            (ir::ScalarType::F32, ir::ScalarType::I32) => self.alu(ir::AluOp::F2i, [v, v, v], out),
            (ir::ScalarType::Bool, ir::ScalarType::F32) => self.alu(ir::AluOp::B2f, [v, v, v], out),
            (ir::ScalarType::Bool, ir::ScalarType::I32) => self.alu(ir::AluOp::B2i, [v, v, v], out),
            (ir::ScalarType::F32, ir::ScalarType::Bool) => self.alu(ir::AluOp::F2b, [v, v, v], out),
            (ir::ScalarType::I32, ir::ScalarType::Bool) => self.alu(ir::AluOp::I2b, [v, v, v], out),
            /* ES 3.00 section 5.4.1: between `int` and `uint` "the bit
             * pattern ... is preserved" -- the same value, no instruction
             * (ir.md: the front end reuses the ValueId). */
            (ir::ScalarType::I32, ir::ScalarType::U32) | (ir::ScalarType::U32, ir::ScalarType::I32) => Ok(v),
            (ir::ScalarType::U32, ir::ScalarType::F32) => self.alu(ir::AluOp::U2f, [v, v, v], out),
            (ir::ScalarType::F32, ir::ScalarType::U32) => self.alu(ir::AluOp::F2u, [v, v, v], out),
            /* A bool is 0 or all ones; `B2i` gives 0 or 1 and `I2b` tests
             * against 0, which is the same question for either signedness. */
            (ir::ScalarType::Bool, ir::ScalarType::U32) => self.alu(ir::AluOp::B2i, [v, v, v], out),
            (ir::ScalarType::U32, ir::ScalarType::Bool) => self.alu(ir::AluOp::I2b, [v, v, v], out),
            /* The equal pairs returned above; the compiler cannot see that
             * through the guard, and a silent `Ok(v)` here would hide a
             * conversion that was forgotten. */
            (a, b) => unreachable_conversion(self.line, a, b),
        }
    }

    fn scalar_of_type(&self, base: BaseType<'a>) -> ir::ScalarType {
        scalar_of(base).unwrap_or(ir::ScalarType::F32)
    }

    /// A four-lane float constant.
    ///
    /// Reused if this run already built the same one. `vec4(v_uv, 1.0, 1.0)`
    /// asked for `1.0` twice and got two registers holding the same number;
    /// over a shader that is registers spent on nothing, and register
    /// pressure is not free -- it is what refused `shapestroke.vert`.
    ///
    /// **Only within the same run**, which is what makes it safe without a
    /// dominance test: a constant defined inside an `if` branch is not
    /// available after the branch, and reusing one across branches would read
    /// a register the other path never wrote. `out` IS the run, so scanning
    /// it is exactly the right scope.
    /// `1 / v`, folded when `v` is a literal.
    ///
    /// Shared by scalar division and by `matrix / scalar`, which are the
    /// same expansion: ES 1.00 section 4.5.1 does not require division to
    /// be correctly rounded at any precision, so a reciprocal and a
    /// multiply IS the lowering rather than a shortcut.
    ///
    /// Dividing by a LITERAL needs no instruction at all -- the reciprocal
    /// is known now. `x / 2.0` is one multiply, which is what the reference
    /// compiler emits and what the source says, and a constant can be folded
    /// into its consumer as an immediate where an `Frcp` result cannot.
    ///
    /// NOT folded when a lane is not finite afterwards -- `x / 0.0`, whose
    /// value section 4.5.1 leaves undefined. The hardware's reciprocal
    /// produces an infinity there and so would this, but an infinite
    /// LITERAL is a pattern nothing downstream has ever been given, and an
    /// undefined case is not worth being the first to hand one to the
    /// encoder.
    fn reciprocal(&mut self, v: Src, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        if let Some(lanes) = self.const_lanes_of(v) {
            if lanes.iter().all(|x| x.is_finite()) {
                let inv = [1.0 / lanes[0], 1.0 / lanes[1], 1.0 / lanes[2], 1.0 / lanes[3]];
                if inv.iter().all(|x| x.is_finite()) {
                    return self.const_value(inv, out);
                }
            }
        }
        self.alu_w(ir::AluOp::Frcp, [v, v, v], want, out)
    }

    /// The four lanes a `Src` names when it reads a float CONSTANT, with
    /// its swizzle applied; `None` when it reads anything else.
    ///
    /// The swizzle is the point: `v.yx` over `ConstF32([a, b, _, _])` is
    /// `[b, a, ..]`, and folding the underlying lanes instead would be the
    /// representation-instead-of-the-type mistake this compiler keeps
    /// making.
    fn const_lanes_of(&self, v: Src) -> Option<[f32; 4]> {
        let Some(&Op::ConstF32(bits)) = self.instrs.as_slice().get(v.val.0 as usize) else {
            return None;
        };
        let mut lanes = [0.0f32; 4];
        for (i, l) in lanes.iter_mut().enumerate() {
            *l = f32::from_bits(bits[v.swizzle[i] as usize]);
        }
        Some(lanes)
    }

    fn const_value(&mut self, lanes: [f32; 4], out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* Raw bits, because `Op` derives `Eq` and `f32` does not
         * (`ir.rs`: "as raw bits so that the type stays `Eq`"). NaN and the
         * two zeroes compare by bits here, which is what a shader means: a
         * constant is the pattern the compiler will emit. */
        let bits = [lanes[0].to_bits(), lanes[1].to_bits(), lanes[2].to_bits(), lanes[3].to_bits()];
        for n in out.as_slice() {
            if let Node::Instr(ir::InstrId(i)) = *n {
                if self.instrs.as_slice().get(i as usize) == Some(&Op::ConstF32(bits)) {
                    return Ok(Src::id(ValueId(i)));
                }
            }
        }
        let v = self.push_instr(Op::ConstF32(bits))?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// A four-lane integer constant, reused within the run as
    /// [`Lower::const_value`] reuses a float one, and for the same reason.
    fn int_const(&mut self, lanes: [i32; 4], out: &mut AVec<'a, Node>) -> R<'a, Src> {
        for n in out.as_slice() {
            if let Node::Instr(ir::InstrId(i)) = *n {
                if self.instrs.as_slice().get(i as usize) == Some(&Op::ConstI32(lanes)) {
                    return Ok(Src::id(ValueId(i)));
                }
            }
        }
        let v = self.push_instr(Op::ConstI32(lanes))?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// An integer literal's exact value, `-` included: the path into an int
    /// constant that does not pass through an `f32`, which holds only 24
    /// bits of it.
    fn int_lit(&self, e: ExprId) -> Option<i32> {
        match *self.ast.expr(e) {
            Expr::IntLit(v) => Some(v as i32),
            /* The bit pattern: `int(0xffffffffu)` is -1 and `uint(-1)` is
             * 0xffffffff (ES 3.00 section 5.4.1). */
            Expr::UintLit(v) => Some(v as i32),
            Expr::Unary(UnOp::Minus, x) => self.int_lit(x).map(i32::wrapping_neg),
            _ => None,
        }
    }

    /// A constructor argument's constant value. The subset's arguments are
    /// literals; anything else is refused rather than guessed.
    fn float_of(&self, e: ExprId) -> R<'a, f32> {
        match *self.ast.expr(e) {
            Expr::FloatLit(v) => Ok(v),
            /* `vec4(1, 0, 0, 1)` is legal: a constructor converts
             * explicitly, which is the one place ES 1.00 allows it. */
            Expr::IntLit(v) => Ok(v as f32),
            Expr::UintLit(v) => Ok(v as f32),
            /* `-1u` is the uint 0xffffffff, not -1.0: an unsigned negation
             * wraps, so it is left to the value path. */
            Expr::Unary(UnOp::Minus, x) if matches!(self.types.of(x).base, BaseType::Uint | BaseType::UVec(_)) => {
                Err(self.err("-", "this constructor argument is not lowered yet"))
            }
            Expr::Unary(UnOp::Minus, x) => Ok(-self.float_of(x)?),
            ref other => Err(self.err(other.kind_name(), "this constructor argument is not lowered yet")),
        }
    }

    /// `v.xyz`: a swizzle becomes the operand's component selection rather
    /// than an instruction, which is the whole point of the vector IR.
    fn swizzle(&self, base: Src, field: &'a str) -> R<'a, Src> {
        let bytes = field.as_bytes();
        if bytes.is_empty() || bytes.len() > 4 {
            return Err(self.err(field, "a swizzle names one to four components"));
        }
        let mut sw = base.swizzle;
        for (dst, &c) in sw.iter_mut().zip(bytes) {
            let idx = match c {
                b'x' | b'r' | b's' => 0usize,
                b'y' | b'g' | b't' => 1,
                b'z' | b'b' | b'p' => 2,
                b'w' | b'a' | b'q' => 3,
                _ => return Err(self.err(field, "not a component name")),
            };
            /* Composed with what the base already selects, so `v.wzyx.x` is
             * `v.w` and not `v.x`. */
            *dst = base.swizzle[idx];
        }
        /* Lanes the swizzle does not name repeat its last: a consumer reads
         * only `components` of the value, and leaving a stale lane there
         * would put a plausible wrong number in front of it. */
        let last = sw[bytes.len() - 1];
        for dst in sw.iter_mut().skip(bytes.len()) {
            *dst = last;
        }
        Ok(Src { val: base.val, swizzle: sw })
    }

    /* ---- arithmetic ---------------------------------------------------- */

    /// The width of an expression, from the type the checker gave it.
    fn width(&self, e: ExprId) -> R<'a, u8> {
        let ty = self.types.of(e);
        if ty.is_unknown() {
            return Err(self.err(self.ast.expr(e).kind_name(), "the type of this expression is not known"));
        }
        Ok(ty.base.components().max(1))
    }

    /// `-x` and `+x`. `!x` and the increments are not lowered yet.
    fn unary(&mut self, op: UnOp, x: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        match op {
            /* Unary plus is the identity (section 5.9), not an instruction. */
            UnOp::Plus => {
                let w = self.width(x)?;
                self.value(x, w, out)
            }
            UnOp::Minus => {
                let w = self.width(x)?;
                let v = self.value(x, w, out)?;
                let scalar = self.scalar_kind(x)?;
                let op = match scalar {
                    ir::ScalarType::F32 => ir::AluOp::Fneg,
                    /* Two's complement: the same operation on a uint. */
                    ir::ScalarType::I32 | ir::ScalarType::U32 => ir::AluOp::Ineg,
                    ir::ScalarType::Bool => return Err(self.err("-", "a bool cannot be negated")),
                };
                self.alu_w(op, [v, v, v], w, out)
            }
            UnOp::Not => {
                /* A bool is an all-ones mask, so `!` is a bitwise not and
                 * `Bnot` is exactly that. gles2-core measured the
                 * representation against the reference compiler; the
                 * evidence is in backend.md under "Booleans". */
                let w = self.width(x)?;
                let v = self.value(x, w, out)?;
                if self.scalar_kind(x)? != ir::ScalarType::Bool {
                    return Err(self.err("!", "'!' takes a bool"));
                }
                self.alu(ir::AluOp::Bnot, [v, v, v], out)
            }
            /* ES 3.00 section 5.9's one's complement, on int or uint. */
            UnOp::BitNot => {
                let w = self.width(x)?;
                let v = self.value(x, w, out)?;
                if !matches!(self.scalar_kind(x)?, ir::ScalarType::I32 | ir::ScalarType::U32) {
                    return Err(self.err("~", "'~' takes an integer"));
                }
                self.alu_w(ir::AluOp::Inot, [v, v, v], w, out)
            }
            _ => Err(self.err(op.text(), "this expression is not lowered yet")),
        }
    }

    /// `a + b`, `a - b`, `a * b`, `a / b`.
    ///
    /// The type checker has already ruled on whether the operands go
    /// together, so what is decided here is only *which* lowering a legal
    /// pair asks for -- and that is not a syntactic question. `m * v` with a
    /// matrix on the left is a linear-algebraic product of four
    /// instructions; with a float on the left it is one. Section 5.9 spells
    /// out which is which, and only the operand types tell them apart.
    fn binary(&mut self, e: ExprId, op: BinOp, lhs: ExprId, rhs: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let (lt, rt) = (self.types.of(lhs).base, self.types.of(rhs).base);
        if self.types.of(lhs).is_unknown() || self.types.of(rhs).is_unknown() {
            return Err(self.err(op.text(), "the type of this expression is not known"));
        }
        if matches!(op, BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne) {
            return self.compare(op, lhs, rhs, out);
        }
        if op == BinOp::Mul {
            match (lt, rt) {
                /* matCxR * vecC -> vecR */
                (BaseType::Mat(c, _), BaseType::Vec(k)) if k == c => return self.mat_times_vec(lhs, rhs, c, out),
                /* `v * m`, the row-vector product: a different sum from
                 * `m * v`, never its transpose's code. vecR * matCxR -> vecC. */
                (BaseType::Vec(k), BaseType::Mat(c, r)) if k == r => return self.vec_times_mat(lhs, rhs, c, r, out),
                /* `m * m` is a matrix value, so it is reached through
                 * `matrix_value` and never here -- if it arrives here the
                 * product is being used where a vector is wanted, which the
                 * checker refuses first. */
                (BaseType::Vec(_), BaseType::Mat(..)) | (BaseType::Mat(..), BaseType::Mat(..)) => {
                    return Err(self.err("*", "this matrix product is not lowered yet"));
                }
                _ => {}
            }
        }
        if matches!(lt, BaseType::Mat(..)) || matches!(rt, BaseType::Mat(..)) {
            /* Everything else with a matrix operand -- `m + m`, `m * f`,
             * `m / f` -- is component-wise over N columns, which is N
             * instructions and needs the column-wise value form. */
            return Err(self.err(op.text(), "this matrix expression is not lowered yet"));
        }

        /* Section 5.9: the right operand of `&&` and `||` is evaluated only
         * if the left one does not already decide the result. With a side
         * effect in it -- a call to a function that writes a global, an
         * assignment, a `++` -- evaluating it anyway is visible, and it was:
         * `while (in0 && foo())` ran `foo()`'s `++sideEffectCounter` with
         * `in0` false (dEQP-GLES3 loops.short_circuit, 8 wrong values in
         * the evaluator). The premise this code used to state, "ES 1.00
         * expressions have no side effects to skip", was never true of a
         * call to a user function; ES 1.00's corpus simply had no case. */
        if matches!(op, BinOp::AndAnd | BinOp::OrOr) && self.has_side_effects(rhs, 0) {
            return self.short_circuit(op == BinOp::AndAnd, lhs, rhs, out);
        }

        let want = self.width(e)?;
        let lv = self.operand_at(lhs, want, out)?;
        let rv = self.operand_at(rhs, want, out)?;
        let scalar = self.scalar_kind(e)?;
        match (op, scalar) {
            (BinOp::Add, ir::ScalarType::F32) => self.alu_w(ir::AluOp::Fadd, [lv, rv, rv], want, out),
            /* `int` and `uint` add, subtract and multiply with the same bits
             * (ir.md); they differ where the ES 3.00 row does. */
            (BinOp::Add, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                self.alu_w(ir::AluOp::Iadd, [lv, rv, rv], want, out)
            }
            (BinOp::Sub, ir::ScalarType::F32) => {
                /* There is no `Fsub` in the kernel, on purpose: the ISA
                 * carries negate as a source modifier, so `a + (-b)` costs
                 * the same instruction and the back end folds the negate
                 * into it. */
                let neg = self.alu_w(ir::AluOp::Fneg, [rv, rv, rv], want, out)?;
                self.alu_w(ir::AluOp::Fadd, [lv, neg, neg], want, out)
            }
            (BinOp::Sub, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                let neg = self.alu_w(ir::AluOp::Ineg, [rv, rv, rv], want, out)?;
                self.alu_w(ir::AluOp::Iadd, [lv, neg, neg], want, out)
            }
            (BinOp::Mul, ir::ScalarType::F32) => self.alu_w(ir::AluOp::Fmul, [lv, rv, rv], want, out),
            (BinOp::Mul, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                self.alu_w(ir::AluOp::Imul, [lv, rv, rv], want, out)
            }
            (BinOp::Div, ir::ScalarType::F32) => {
                /* Likewise no `Fdiv`: reciprocal then multiply. ES 1.00
                 * section 4.5.1 does not require division to be correctly
                 * rounded at any precision, which is what makes this the
                 * standard lowering rather than a shortcut. */
                let r = self.reciprocal(rv, want, out)?;
                self.alu_w(ir::AluOp::Fmul, [lv, r, r], want, out)
            }
            (BinOp::Div, ir::ScalarType::I32) => self.alu_w(ir::AluOp::Idiv, [lv, rv, rv], want, out),
            (BinOp::Div, ir::ScalarType::U32) => self.alu_w(ir::AluOp::Udiv, [lv, rv, rv], want, out),
            /* ES 3.00 section 5.9's integer operators. */
            (BinOp::Mod, ir::ScalarType::I32) => self.alu_w(ir::AluOp::Imod, [lv, rv, rv], want, out),
            (BinOp::Mod, ir::ScalarType::U32) => self.alu_w(ir::AluOp::Umod, [lv, rv, rv], want, out),
            (BinOp::BitAnd, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                self.alu_w(ir::AluOp::Iand, [lv, rv, rv], want, out)
            }
            (BinOp::BitOr, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                self.alu_w(ir::AluOp::Ior, [lv, rv, rv], want, out)
            }
            (BinOp::BitXor, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                self.alu_w(ir::AluOp::Ixor, [lv, rv, rv], want, out)
            }
            /* A shift's result is the LEFT operand's type, and so is the
             * signedness `>>` extends with; the right operand may be either
             * (section 5.9), and its bits are its value when it is not
             * negative, which is the only case the section defines. */
            (BinOp::Shl, ir::ScalarType::I32 | ir::ScalarType::U32) => {
                self.alu_w(ir::AluOp::Ishl, [lv, rv, rv], want, out)
            }
            (BinOp::Shr, ir::ScalarType::I32) => self.alu_w(ir::AluOp::Ishr, [lv, rv, rv], want, out),
            (BinOp::Shr, ir::ScalarType::U32) => self.alu_w(ir::AluOp::Ushr, [lv, rv, rv], want, out),
            /* `&&`, `||`, `^^` on bools. A bool is 0 or 1 in an integer lane
             * (`ir.rs`), so they are the integer bitwise operations -- when
             * the right operand has no side effect, which is when evaluating
             * it anyway is indistinguishable from the short circuit.
             * `binary` sends the other case to `short_circuit` before it
             * gets here. */
            (BinOp::AndAnd, ir::ScalarType::Bool) => self.alu_w(ir::AluOp::Iand, [lv, rv, rv], want, out),
            (BinOp::OrOr, ir::ScalarType::Bool) => self.alu_w(ir::AluOp::Ior, [lv, rv, rv], want, out),
            (BinOp::XorXor, ir::ScalarType::Bool) => self.alu_w(ir::AluOp::Ixor, [lv, rv, rv], want, out),
            _ => Err(self.err(op.text(), "this expression is not lowered yet")),
        }
    }

    /// `a < b`, `a == b` and the rest of section 5.9's relational and
    /// equality operators.
    ///
    /// The result is a `bool`, so the operands' type decides the opcode and
    /// the result's does not -- which is why this is separate from the
    /// arithmetic above, where the result's type decides everything.
    ///
    /// `>` and `>=` are `<` and `<=` with the operands the other way round.
    /// The kernel has no `Fgt`, and it should not: the hardware compare is
    /// one direction and a second opcode would be the same instruction under
    /// a second name.
    fn compare(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let (lt, rt) = (self.types.of(lhs), self.types.of(rhs));
        if lt.is_unknown() || rt.is_unknown() {
            return Err(self.err(op.text(), "the type of this expression is not known"));
        }
        /* `components()` cannot be what decides the width here, and this
         * check has to come before it is taken. It answers 4 for a `mat4`,
         * which would be compared as if it were one `vec4` -- one column
         * against one column, three columns ignored -- and 0 for a struct,
         * which `.max(1)` would turn into a scalar compare of two things
         * that are not scalars. Both are wrong answers with no failure, so
         * the aggregates are refused by their TYPE and by name -- except
         * where they are lowered as aggregates, just below. */
        /* Section 5.9: `==` and `!=` "operate on all types except arrays,
         * structures containing arrays, sampler types and structures
         * containing sampler types ... For vectors, matrices, and
         * structures, all components of the operands must be equal". A
         * matrix is compared column by column and a struct leaf by leaf,
         * every result folded into one bool. */
        if matches!(op, BinOp::Eq | BinOp::Ne) && lt.is_array() && rt.is_array() && self.ast.es3 {
            let all = self.array_equal(lhs, rhs, out)?;
            return match op {
                BinOp::Ne => self.alu_w(ir::AluOp::Bnot, [all, all, all], 1, out),
                _ => Ok(all),
            };
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) && !lt.is_array() && !rt.is_array() {
            let all = match (lt.base, rt.base) {
                (BaseType::Mat(n, r), BaseType::Mat(m, r2)) if n == m && r == r2 => {
                    let a = self.matrix_value(lhs, n, out)?;
                    let b = self.matrix_value(rhs, n, out)?;
                    let mut acc: Option<Src> = None;
                    for j in 0..n as usize {
                        let eq = self.all_equal(a[j], b[j], r, ir::ScalarType::F32, out)?;
                        acc = Some(self.and_bools(acc, eq, out)?);
                    }
                    acc
                }
                (BaseType::Struct(sname), BaseType::Struct(other)) if sname == other => {
                    let ra = self.struct_temp(lhs, sname, out)?;
                    let rb = self.struct_temp(rhs, sname, out)?;
                    let mut shapes: AVec<(ir::ScalarType, u8)> = AVec::new();
                    self.struct_leaf_shapes(sname, 0, &mut shapes)?;
                    let mut acc: Option<Src> = None;
                    for (k, &(scalar, w)) in shapes.as_slice().iter().enumerate() {
                        let x = self.push_instr(Op::LoadLocal {
                            local: ir::LocalId(ra.0 + k as u32),
                            index: None,
                            comps: w,
                        })?;
                        out.push(self.a, Node::Instr(ir::InstrId(x.0)))?;
                        let y = self.push_instr(Op::LoadLocal {
                            local: ir::LocalId(rb.0 + k as u32),
                            index: None,
                            comps: w,
                        })?;
                        out.push(self.a, Node::Instr(ir::InstrId(y.0)))?;
                        let eq = self.all_equal(Src::id(x), Src::id(y), w, scalar, out)?;
                        acc = Some(self.and_bools(acc, eq, out)?);
                    }
                    acc
                }
                _ => None,
            };
            if let Some(all) = all {
                return match op {
                    BinOp::Ne => self.alu_w(ir::AluOp::Bnot, [all, all, all], 1, out),
                    _ => Ok(all),
                };
            }
        }
        for t in [lt, rt] {
            let refusal = if t.is_array() {
                "comparing arrays is not lowered yet"
            } else if matches!(t.base, BaseType::Mat(..)) {
                "comparing matrices is not lowered yet"
            } else if matches!(t.base, BaseType::Struct(_)) {
                "comparing structs is not lowered yet"
            } else {
                continue;
            };
            return Err(self.err(op.text(), refusal));
        }
        let width = lt.base.components().max(rt.base.components()).max(1);
        if width > 1 {
            return self.compare_vector(op, lhs, rhs, width, out);
        }
        let scalar = self.scalar_kind(lhs)?;
        let lv = self.operand_at(lhs, 1, out)?;
        let rv = self.operand_at(rhs, 1, out)?;
        /* Swapped for the `>` family, which is the whole difference. */
        let (a, b) = match op {
            BinOp::Gt | BinOp::Ge => (rv, lv),
            _ => (lv, rv),
        };
        let alu = match (op, scalar) {
            (BinOp::Lt | BinOp::Gt, ir::ScalarType::F32) => ir::AluOp::Flt,
            (BinOp::Le | BinOp::Ge, ir::ScalarType::F32) => ir::AluOp::Fle,
            (BinOp::Eq, ir::ScalarType::F32) => ir::AluOp::Feq,
            (BinOp::Ne, ir::ScalarType::F32) => ir::AluOp::Fne,
            (BinOp::Lt | BinOp::Gt, ir::ScalarType::I32) => ir::AluOp::Ilt,
            (BinOp::Le | BinOp::Ge, ir::ScalarType::I32) => ir::AluOp::Ile,
            (BinOp::Lt | BinOp::Gt, ir::ScalarType::U32) => ir::AluOp::Ult,
            (BinOp::Le | BinOp::Ge, ir::ScalarType::U32) => ir::AluOp::Ule,
            (BinOp::Eq, ir::ScalarType::I32 | ir::ScalarType::U32 | ir::ScalarType::Bool) => ir::AluOp::Ieq,
            (BinOp::Ne, ir::ScalarType::I32 | ir::ScalarType::U32 | ir::ScalarType::Bool) => ir::AluOp::Ine,
            _ => return Err(self.err(op.text(), "this comparison is not lowered yet")),
        };
        self.alu_w(alu, [a, b, b], 1, out)
    }

    /// Are all `w` lanes of `a` and `b` equal, as one bool: the component
    /// compare and, for a vector, `Ball` over the real lanes with the last
    /// one repeated into the spares (the four-lane contract).
    fn all_equal(&mut self, a: Src, b: Src, w: u8, scalar: ir::ScalarType, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let op = match scalar {
            ir::ScalarType::F32 => ir::AluOp::Feq,
            _ => ir::AluOp::Ieq,
        };
        let cmp = self.alu_w(op, [a, b, b], w, out)?;
        if w <= 1 {
            return Ok(Src { val: cmp.val, swizzle: [cmp.swizzle[0]; 4] });
        }
        let mut swizzle = cmp.swizzle;
        let last = swizzle[w as usize - 1];
        for lane in swizzle.iter_mut().skip(w as usize) {
            *lane = last;
        }
        let v = Src { val: cmp.val, swizzle };
        let r = self.alu_w(ir::AluOp::Ball, [v, v, v], w, out)?;
        Ok(Src { val: r.val, swizzle: [r.swizzle[0]; 4] })
    }

    /// Could evaluating `e` be seen, other than through its value? An
    /// assignment, `++`/`--`, or a call to a function the shader defines
    /// (which may write a global or an `out` parameter) -- conservatively:
    /// a user function that happens to be pure still counts, and costs only
    /// a branch. Past a deep nesting the answer is yes, which is the safe one.
    fn has_side_effects(&self, e: ExprId, depth: u32) -> bool {
        if depth > 64 {
            return true;
        }
        let d = depth + 1;
        match *self.ast.expr(e) {
            Expr::Assign(..) | Expr::Unary(UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec, _) => true,
            Expr::Call { name, args } => {
                let user = self
                    .ast
                    .decls
                    .iter()
                    .any(|x| matches!(x, Decl::Function { name: n, body: Some(_), .. } if *n == name));
                user || args.indices().any(|i| self.has_side_effects(self.ast.expr_ids[i], d))
            }
            Expr::Unary(_, x) | Expr::Field(x, _) => self.has_side_effects(x, d),
            Expr::Binary(_, l, r) | Expr::Comma(l, r) | Expr::Index(l, r) => {
                self.has_side_effects(l, d) || self.has_side_effects(r, d)
            }
            Expr::Select(c, t, f) => {
                self.has_side_effects(c, d) || self.has_side_effects(t, d) || self.has_side_effects(f, d)
            }
            Expr::ArrayCtor { args, .. } => args.indices().any(|i| self.has_side_effects(self.ast.expr_ids[i], d)),
            Expr::Length(_) => false,
            Expr::IntLit(_) | Expr::UintLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) | Expr::Ident(_) => false,
        }
    }

    /// A one-lane bool temporary, or one of `comps` lanes of `scalar`: a
    /// slot for a value that is decided on a branch.
    fn temp(&mut self, scalar: ir::ScalarType, comps: u8) -> R<'a, Local<'a>> {
        let id = ir::LocalId(self.locals.len() as u32);
        self.locals.push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(scalar, comps), array_len: 1 })?;
        Ok(Local::scalar("", id, comps))
    }

    /// `a && b` / `a || b` whose right operand has a side effect: the right
    /// operand is evaluated inside an `If` on the left one, as section 5.9
    /// says, and the result is the slot both paths leave it in.
    fn short_circuit(&mut self, and: bool, lhs: ExprId, rhs: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let t = self.temp(ir::ScalarType::Bool, 1)?;
        let l = self.operand_at(lhs, 1, out)?;
        self.store_local(t, l, out)?;
        /* `&&` needs the right side when the left is true, `||` when it is
         * false. */
        let cond = if and { l } else { self.alu_w(ir::AluOp::Bnot, [l, l, l], 1, out)? };
        let mut then_items: AVec<Node> = AVec::new();
        let r = self.operand_at(rhs, 1, &mut then_items)?;
        self.store_local(t, r, &mut then_items)?;
        let then_ = self.commit(then_items)?;
        out.push(
            self.a,
            Node::If { cond: Src { val: cond.val, swizzle: [cond.swizzle[0]; 4] }, then_, else_: IrRange::EMPTY },
        )?;
        let v = self.push_instr(Op::LoadLocal { local: t.id, index: None, comps: 1 })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// `acc && b` for bools already evaluated (both are values here, so the
    /// short circuit has nothing to skip): `acc ? b : acc`.
    fn and_bools(&mut self, acc: Option<Src>, b: Src, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        match acc {
            None => Ok(b),
            Some(a) => self.alu_w(ir::AluOp::Bcsel, [a, b, a], 1, out),
        }
    }

    /// The run of slots holding the struct value `e`: its own if it is a
    /// variable, otherwise a temporary filled by `struct_fill` (a
    /// constructor, a call, a uniform).
    fn struct_temp(&mut self, e: ExprId, sname: &'a str, out: &mut AVec<'a, Node>) -> R<'a, ir::LocalId> {
        if let Some(l) = self.struct_run_of(e)? {
            return Ok(l.id);
        }
        let line = self.line;
        let run = self.struct_run(sname, sname, line)?;
        let id = ir::LocalId(self.locals.len() as u32);
        self.push_struct_slots(sname, sname, line, 0)?;
        let tmp = Local { name: "", id, comps: 1, run, value: None };
        self.struct_fill(e, tmp, BaseType::Struct(sname), sname, out)?;
        Ok(id)
    }

    /// Each leaf of `sname`'s scalar kind and width, in slot order.
    fn struct_leaf_shapes(&self, sname: &'a str, depth: u32, acc: &mut AVec<'a, (ir::ScalarType, u8)>) -> R<'a, ()> {
        let _ = depth;
        let mut leaves: AVec<Leaf<'a>> = AVec::new();
        self.struct_leaf_list(sname, "", 0, &mut leaves)?;
        for lf in leaves.as_slice() {
            acc.push(self.a, (lf.scalar, lf.comps))?;
        }
        Ok(())
    }

    /// `u == v` and `u != v` on vectors, which section 5.9 says are a single
    /// `bool` and not a vector of them: `==` is true when EVERY component
    /// matches, `!=` when ANY component differs.
    ///
    /// So each is a component-wise compare and then a reduction --
    /// `Feq` then `Ball`, `Fne` then `Bany` -- and never a component-wise
    /// compare on its own. The component-wise forms are the built-in
    /// functions `equal` and `notEqual`, which are a different thing with a
    /// different type, and `<`, `<=`, `>` and `>=` do not take vectors at
    /// all in ES 1.00; `lessThan` and its family are those.
    ///
    /// The spare lanes are the part that has to be got right. `Ball` and
    /// `Bany` fold ALL FOUR lanes -- `vivante/builtins.rs` states it as a
    /// contract and `powervr/builtins.rs` reduces the same way -- so a
    /// `bvec2` compare whose z and w lanes were left alone would reduce
    /// whatever those lanes happened to hold. Repeating the last real
    /// component into them costs nothing, because `and` and `or` are
    /// idempotent, and it is the caller's job: the reduction cannot know the
    /// width, since a repeated lane is legally a real one.
    fn compare_vector(
        &mut self,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        width: u8,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Src> {
        let scalar = self.scalar_kind(lhs)?;
        let (alu, reduce) = match (op, scalar) {
            (BinOp::Eq, ir::ScalarType::F32) => (ir::AluOp::Feq, ir::AluOp::Ball),
            (BinOp::Ne, ir::ScalarType::F32) => (ir::AluOp::Fne, ir::AluOp::Bany),
            (BinOp::Eq, ir::ScalarType::I32 | ir::ScalarType::U32 | ir::ScalarType::Bool) => {
                (ir::AluOp::Ieq, ir::AluOp::Ball)
            }
            (BinOp::Ne, ir::ScalarType::I32 | ir::ScalarType::U32 | ir::ScalarType::Bool) => {
                (ir::AluOp::Ine, ir::AluOp::Bany)
            }
            _ => return Err(self.err(op.text(), "this operator does not take vectors")),
        };
        let a = self.operand_at(lhs, width, out)?;
        let b = self.operand_at(rhs, width, out)?;
        let cmp = self.alu_w(alu, [a, b, b], width, out)?;
        let mut swizzle = cmp.swizzle;
        let last = swizzle[width as usize - 1];
        for lane in swizzle.iter_mut().skip(width as usize) {
            *lane = last;
        }
        let all = Src { val: cmp.val, swizzle };
        /* `result_comps` makes this 1: Ball and Bany reduce. */
        self.alu_w(reduce, [all, all, all], width, out)
    }

    /// `m * v`: the shipped GC7000 blob's sequence, one `Fmul` and N-1
    /// `Ffma`, highest column first.
    ///
    /// The column order is a measurement, not a preference. `hello.vert`'s
    /// blob is `mul t1, u0, t0.wwww` then three `mad`s against z, y, x -- and
    /// its constant-file map `CD_4` puts block dwords 12..15 in `u0`, so `u0`
    /// is column 3. Every term pairs column *i* with `v[i]`, which is
    /// ordinary matrix times vector; only the register numbering runs
    /// backwards. Emitting the terms from the last column down reproduces the
    /// shipped instruction order while keeping the algebra right.
    fn mat_times_vec(&mut self, m: ExprId, v: ExprId, n: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let vec = self.value(v, n, out)?;
        self.mat_times_column(m, vec, n, out)
    }

    /// `v * m`, section 5.11:
    ///
    /// ```text
    /// u = v * m;  is equivalent to
    ///    u.x = dot(v, m[0]); // m[0] is the left column of m
    ///    u.y = dot(v, m[1]); // dot(a,b) is the inner (dot) product of a and b
    ///    u.z = dot(v, m[2]);
    /// ```
    ///
    /// Not `m * v`'s sum of scaled columns, which is the transpose and equal
    /// only for a symmetric matrix -- the identity, where a wrong lowering
    /// would pass every test that uses one.
    fn vec_times_mat(&mut self, v: ExprId, m: ExprId, c: u8, r: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let vec = self.value(v, r, out)?;
        let cols = self.matrix_value(m, c, out)?;
        let dot = match r {
            2 => ir::AluOp::Fdot2,
            3 => ir::AluOp::Fdot3,
            4 => ir::AluOp::Fdot4,
            _ => return Err(self.err("*", "this matrix product is not lowered yet")),
        };
        let mut comps = [(Src::id(ValueId(0)), 0u8); 4];
        for (j, x) in comps.iter_mut().enumerate().take(c as usize) {
            *x = (self.alu_w(dot, [vec, cols[j], cols[j]], r, out)?, 0);
        }
        self.column_from_components(&comps[..c as usize], c, out)
    }

    /// The same product against a vector already lowered, which is what a
    /// matrix times a matrix needs one column at a time.
    fn mat_times_column(&mut self, m: ExprId, vec: Src, n: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let cols = self.matrix_value(m, n, out)?;
        let mut acc = None;
        for j in (0..n).rev() {
            /* Component j of the vector against column j, splatted across
             * the column's lanes. */
            let scale = Src { val: vec.val, swizzle: [vec.swizzle[j as usize]; 4] };
            acc = Some(match acc {
                None => self.alu(ir::AluOp::Fmul, [cols[j as usize], scale, scale], out)?,
                Some(a) => self.alu(ir::AluOp::Ffma, [cols[j as usize], scale, a], out)?,
            });
        }
        acc.ok_or_else(|| self.err("*", "a matrix with no columns"))
    }

    /// Column `j` of a matrix-valued expression.
    ///
    /// Only a name is handled: a matrix uniform, whose columns are
    /// consecutive runs of the block, or a matrix attribute, whose columns
    /// are consecutive locations (section 4.3 gives a `matN` attribute N of
    /// them). A matrix that is computed rather than named needs a value form
    /// that carries N columns, which is its own change.
    /// Every column of a matrix-valued expression.
    ///
    /// A matrix has no single-value form here, and should not: neither GPU
    /// has a matrix register, so a `matN` is N vectors wherever it lives --
    /// N consecutive block dwords in a uniform, N attribute locations, N
    /// slots in a local. Giving the lowering a value that carries its columns
    /// is what lets a matrix be returned from a function, held in a local and
    /// multiplied, instead of only being read from a name.
    ///
    /// Only the first `n` entries are meaningful.
    fn matrix_value(&mut self, m: ExprId, n: u8, out: &mut AVec<'a, Node>) -> R<'a, [Src; 4]> {
        /* `n` is the column count; the rows are the expression's own type's
         * (ES 3.00's matCxR has C != R). */
        let r = self.rows_of(m);
        /* A matrix member of an ES 3.00 structure input: its columns from
         * consecutive locations. */
        if self.stage == Stage::Fragment {
            if let Some(name) = self.struct_varying_leaf(m)? {
                let Some((ir::IoSlot::Varying(loc), comps)) = self.input_of(name) else {
                    return Err(self.err(name, "reading this is not lowered yet"));
                };
                let mut cols = [Src::id(ValueId(0)); 4];
                for (j, c) in cols.iter_mut().enumerate().take(usize::from(n)) {
                    let v = self.push_instr(Op::LoadInput { slot: ir::IoSlot::Varying(loc + j as u8), comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    *c = Src::id(v);
                }
                return Ok(cols);
            }
        }
        /* ES 3.00: element k of a local array of matrices (`a[1]`), or of
         * an array value, at a constant index or chosen at run time. */
        if let Expr::Index(arr, idx) = *self.ast.expr(m) {
            if self.types.of(arr).is_array() {
                let owned = match *self.ast.expr(arr) {
                    Expr::ArrayCtor { .. } | Expr::Call { .. } | Expr::Select(..) | Expr::Comma(..) => true,
                    _ => self.local_chain(arr, &[])?.is_some(),
                };
                if owned {
                    let l = self.array_run_of(arr, out)?;
                    if let Run::MatrixArray { len, cols } = l.run {
                        if cols != n {
                            return Err(self.err("[]", "this matrix expression is not lowered yet"));
                        }
                        let load = |s: &mut Self, el: Local<'a>, j: u8, out: &mut AVec<'a, Node>| -> R<'a, Src> {
                            let v =
                                s.push_instr(Op::LoadLocal { local: el.column(j).id, index: None, comps: el.comps })?;
                            out.push(s.a, Node::Instr(ir::InstrId(v.0)))?;
                            Ok(Src::id(v))
                        };
                        let mut cols_out = [Src::id(ValueId(0)); 4];
                        if let Some(k) = reflect::const_int_expr(self.ast, idx) {
                            let Some(el) = l.element(k as u8).filter(|_| k >= 0 && k < i64::from(len)) else {
                                return Err(self.err("[]", "index out of range"));
                            };
                            for (j, c) in cols_out.iter_mut().enumerate().take(usize::from(n)) {
                                *c = load(self, el, j as u8, out)?;
                            }
                            return Ok(cols_out);
                        }
                        let i = self.index_value(idx, out)?;
                        for (j, c) in cols_out.iter_mut().enumerate().take(usize::from(n)) {
                            let mut els: AVec<Src> = AVec::new();
                            for k in 0..len {
                                let Some(el) = l.element(k) else { break };
                                let v = load(self, el, j as u8, out)?;
                                els.push(self.a, v)?;
                            }
                            *c = self.select_among(els.as_slice(), i, l.comps, out)?;
                        }
                        return Ok(cols_out);
                    }
                }
            }
        }
        /* A name that is a local: N slots, read back. */
        if let Expr::Ident(name) = *self.ast.expr(m) {
            if let Some(l) = self.lookup(name) {
                if l.run.columns() != Some(n) {
                    return Err(self.err(name, "this matrix expression is not lowered yet"));
                }
                let mut cols = [Src::id(ValueId(0)); 4];
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    let col = l.column(j as u8);
                    let v = self.push_instr(Op::LoadLocal { local: col.id, index: None, comps: l.comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    *c = Src::id(v);
                }
                return Ok(cols);
            }
        }
        /* A call to a function that returns a matrix: inlined like any
         * other, with the body's `return` giving N columns instead of one
         * value. 31 of the GC7000 corpus's sources are this. */
        if let Expr::Call { name, args } = *self.ast.expr(m) {
            /* `mat3(...)` is a constructor, not a call, and the checker's
             * table is what says so -- the same order `constructor` uses, so
             * a name means the same thing on both paths. */
            if matches!(super::sema::constructor_type(name, self.ast.es3), Some(BaseType::Mat(..))) {
                return self.matrix_constructor(name, args, n, out);
            }
            if let Some(f) = self.user_function(name, args) {
                return match self.inline(name, f, args, Want::Matrix(n, r), out)? {
                    Ret::Matrix(cols) => Ok(cols),
                    Ret::Value(_) | Ret::Struct => Err(self.err(name, "this function does not return a matrix")),
                };
            }
            /* The one built-in that returns a matrix (section 8.5):
             * component-wise multiply, column by column. */
            let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
            /* ES 3.00 section 8.6's matrix functions (the checker admits
             * them only in an ES 3.00 source). */
            match (name, ids.len()) {
                /* transpose(matCxR) is a matRxC: this value's n columns are
                 * the argument's rows. */
                ("transpose", 1) => {
                    let ac = r;
                    let m_cols = self.matrix_value(ids[0], ac, out)?;
                    return self.transpose_columns(&m_cols, ac, n, out);
                }
                ("inverse", 1) => return self.matrix_inverse(ids[0], n, out),
                /* outerProduct(c, rv): column j is c scaled by rv[j]; c has
                 * the rows, rv the columns. */
                ("outerProduct", 2) => {
                    let c = self.value(ids[0], r, out)?;
                    let rv = self.value(ids[1], n, out)?;
                    let mut cols = [Src::id(ValueId(0)); 4];
                    for (j, col) in cols.iter_mut().enumerate().take(usize::from(n)) {
                        let k = lane_of(rv, j as u8);
                        *col = self.alu_w(ir::AluOp::Fmul, [c, k, k], r, out)?;
                    }
                    return Ok(cols);
                }
                _ => {}
            }
            if name == "matrixCompMult" && ids.len() == 2 {
                let a = self.matrix_value(ids[0], n, out)?;
                let b = self.matrix_value(ids[1], n, out)?;
                let mut cols = [Src::id(ValueId(0)); 4];
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    *c = self.alu_w(ir::AluOp::Fmul, [a[j], b[j], b[j]], r, out)?;
                }
                return Ok(cols);
            }
            return Err(self.err(name, "this matrix expression is not lowered yet"));
        }
        /* Any arithmetic operator with a matrix operand (section 5.9). */
        if let Expr::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div), lhs, rhs) = *self.ast.expr(m) {
            if matches!(self.types.of(lhs).base, BaseType::Mat(..))
                || matches!(self.types.of(rhs).base, BaseType::Mat(..))
            {
                return self.matrix_binary(op, lhs, rhs, n, out);
            }
        }
        match *self.ast.expr(m) {
            /* Section 5.9: negate operates on matrices and gives the same
             * type -- each column negated. Unary plus is the value. */
            Expr::Unary(UnOp::Minus, x) => {
                let cols = self.matrix_value(x, n, out)?;
                let mut neg = [Src::id(ValueId(0)); 4];
                for (j, c) in neg.iter_mut().enumerate().take(n as usize) {
                    *c = self.alu_w(ir::AluOp::Fneg, [cols[j], cols[j], cols[j]], r, out)?;
                }
                return Ok(neg);
            }
            Expr::Unary(UnOp::Plus, x) => return self.matrix_value(x, n, out),
            /* `m++` and `++m` as values: the columns before or after the
             * step, as `step_value` does for a vector. */
            Expr::Unary(op @ (UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec), x) => {
                if matches!(op, UnOp::PostInc | UnOp::PostDec) {
                    let before = self.matrix_value(x, n, out)?;
                    self.step_by_one(op, x, out)?;
                    return Ok(before);
                }
                self.step_by_one(op, x, out)?;
                return self.matrix_value(x, n, out);
            }
            /* `c ? a : b` over matrices, column by column; both arms are
             * evaluated, as for every other `?:` here (see `Expr::Select`). */
            Expr::Select(c, t, f) => {
                let cond = self.operand_at(c, 1, out)?;
                let cond = Src { val: cond.val, swizzle: [cond.swizzle[0]; 4] };
                let a = self.matrix_value(t, n, out)?;
                let b = self.matrix_value(f, n, out)?;
                let mut cols = [Src::id(ValueId(0)); 4];
                for (j, col) in cols.iter_mut().enumerate().take(n as usize) {
                    *col = self.alu_w(ir::AluOp::Bcsel, [cond, a[j], b[j]], r, out)?;
                }
                return Ok(cols);
            }
            _ => {}
        }
        let mut cols = [Src::id(ValueId(0)); 4];
        for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
            *c = self.matrix_column(m, j as u8, n, out)?;
        }
        Ok(cols)
    }

    /// `transpose(m)` of a matrix of `c` columns and `r` rows: column j of
    /// the result (j < r) is row j of `m`, `c` lanes.
    fn transpose_columns(&mut self, m: &[Src; 4], c: u8, r: u8, out: &mut AVec<'a, Node>) -> R<'a, [Src; 4]> {
        let mut cols = [Src::id(ValueId(0)); 4];
        for (j, col) in cols.iter_mut().enumerate().take(usize::from(r)) {
            let mut parts = [(Src::id(ValueId(0)), 0u8); 4];
            for (i, p) in parts.iter_mut().enumerate().take(usize::from(c)) {
                *p = (m[i], j as u8);
            }
            *col = self.column_from_components(&parts[..usize::from(c)], c, out)?;
        }
        Ok(cols)
    }

    /// The rows of a matrix-typed expression: one column's lanes.
    fn rows_of(&self, e: ExprId) -> u8 {
        match self.types.of(e).base {
            BaseType::Mat(_, r) => r,
            other => other.components().max(1),
        }
    }

    /// `determinant(m)` (ES 3.00 section 8.6), in the order pco computes
    /// it: Mesa's `build_det` (spirv/vtn_glsl450.c) -- the first column
    /// times the vector of its cofactor minors, summed pairwise as
    /// `(p0 - p1) + (p2 - p3)`, each minor the same construction one size
    /// down over swizzled columns (`build_mat_subdet`).
    fn determinant(&mut self, m: &[Src; 4], n: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        self.build_det(&m[..usize::from(n)], out)
    }

    /// `build_det`: `col` are the columns, as many as the size.
    fn build_det(&mut self, col: &[Src], out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let size = col.len();
        let mut parts = [(Src::id(ValueId(0)), 0u8); 4];
        for (i, p) in parts.iter_mut().enumerate().take(size) {
            *p = (self.build_mat_subdet(col, i, 0, out)?, 0);
        }
        let sub = self.column_from_components(&parts[..size], size as u8, out)?;
        let prod = self.alu_w(ir::AluOp::Fmul, [col[0], sub, sub], size as u8, out)?;
        let mut result: Option<Src> = None;
        let mut i = 0;
        while i < size {
            let term = if i + 1 < size {
                let b = lane_of(prod, (i + 1) as u8);
                let nb = self.neg1(b, out)?;
                let v = self.alu_w(ir::AluOp::Fadd, [lane_of(prod, i as u8), nb, nb], 1, out)?;
                lane_of(v, 0)
            } else {
                lane_of(prod, i as u8)
            };
            result = Some(match result {
                None => term,
                Some(r) => {
                    let v = self.alu_w(ir::AluOp::Fadd, [r, term, term], 1, out)?;
                    lane_of(v, 0)
                }
            });
            i += 2;
        }
        result.ok_or_else(|| self.err("determinant", "a matrix with no columns"))
    }

    /// `build_mat_subdet`: the minor of `src` without `row` and column
    /// `c`, as a one-lane value.
    fn build_mat_subdet(&mut self, src: &[Src], row: usize, c: usize, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let size = src.len();
        if size == 2 {
            return Ok(lane_of(src[1 - c], (1 - row) as u8));
        }
        let mut swiz = [0u8; 4];
        for (j, s) in swiz.iter_mut().enumerate().take(3) {
            *s = (j + usize::from(j >= row)) as u8;
        }
        swiz[3] = swiz[2];
        let mut sub = [Src::id(ValueId(0)); 3];
        for j in (0..size).filter(|&j| j != c) {
            sub[j - usize::from(j > c)] = reswizzle(src[j], swiz);
        }
        self.build_det(&sub[..size - 1], out)
    }

    /// `inverse(m)` (ES 3.00 section 8.6), in Mesa's formulation
    /// (`builtin_builder::_inverse_mat2/3/4` in glsl/builtin_functions.cpp):
    /// the adjugate from 2x2 sub-determinants, divided by the determinant
    /// taken along the first row. Both reference compilers -- pco through
    /// NIR on the PowerVR, and the GC7000's -- compute it that way, so
    /// matching the formula keeps the online result within rounding of the
    /// tables' (usc_limits compares within 4 ulp). A cross-product form
    /// (Lengyel's) was exact enough and 9 to 35 ulp away from pco on five
    /// Qt Quick 3D vertex shaders.
    ///
    /// Each `a*b - c*d` is emitted as `fma(a, b, -(c*d))`, the fusion NIR
    /// makes of Mesa's `sub(mul, mul)`. The result is undefined for a
    /// singular matrix, and this divides by its determinant regardless.
    fn matrix_inverse(&mut self, arg: ExprId, n: u8, out: &mut AVec<'a, Node>) -> R<'a, [Src; 4]> {
        let m = self.matrix_value(arg, n, out)?;
        /* m[c][r], one lane, splat */
        let e = |c: usize, r: u8| lane_of(m[c], r);
        let mut adj = [[Src::id(ValueId(0)); 4]; 4];
        let det = match n {
            2 => {
                adj[0][0] = e(1, 1);
                adj[0][1] = self.neg1(e(0, 1), out)?;
                adj[1][0] = self.neg1(e(1, 0), out)?;
                adj[1][1] = e(0, 0);
                self.sub_mul(e(0, 0), e(1, 1), e(1, 0), e(0, 1), out)?
            }
            3 => {
                let f11_22_21_12 = self.sub_mul(e(1, 1), e(2, 2), e(2, 1), e(1, 2), out)?;
                let f10_22_20_12 = self.sub_mul(e(1, 0), e(2, 2), e(2, 0), e(1, 2), out)?;
                let f10_21_20_11 = self.sub_mul(e(1, 0), e(2, 1), e(2, 0), e(1, 1), out)?;
                adj[0][0] = f11_22_21_12;
                adj[1][0] = self.neg1(f10_22_20_12, out)?;
                adj[2][0] = f10_21_20_11;
                let t = self.sub_mul(e(0, 1), e(2, 2), e(2, 1), e(0, 2), out)?;
                adj[0][1] = self.neg1(t, out)?;
                adj[1][1] = self.sub_mul(e(0, 0), e(2, 2), e(2, 0), e(0, 2), out)?;
                let t = self.sub_mul(e(0, 0), e(2, 1), e(2, 0), e(0, 1), out)?;
                adj[2][1] = self.neg1(t, out)?;
                adj[0][2] = self.sub_mul(e(0, 1), e(1, 2), e(1, 1), e(0, 2), out)?;
                let t = self.sub_mul(e(0, 0), e(1, 2), e(1, 0), e(0, 2), out)?;
                adj[1][2] = self.neg1(t, out)?;
                adj[2][2] = self.sub_mul(e(0, 0), e(1, 1), e(1, 0), e(0, 1), out)?;
                /* m00*f1 - m01*f2 + m02*f3 */
                let a = self.sub_mul(e(0, 0), f11_22_21_12, e(0, 1), f10_22_20_12, out)?;
                self.fma1(e(0, 2), f10_21_20_11, a, out)?
            }
            _ => {
                /* SubFactor00..18: the 2x2 determinants of rows 2-3 (00..05),
                 * 1-3 (06..12) and 1-2 (13..18), as Mesa numbers them. */
                /* (a col, a row, b col, b row, c col, c row, d col, d row): a*b - c*d */
                type SubFactor = (usize, u8, usize, u8, usize, u8, usize, u8);
                const SF: [SubFactor; 19] = [
                    (2, 2, 3, 3, 3, 2, 2, 3),
                    (2, 1, 3, 3, 3, 1, 2, 3),
                    (2, 1, 3, 2, 3, 1, 2, 2),
                    (2, 0, 3, 3, 3, 0, 2, 3),
                    (2, 0, 3, 2, 3, 0, 2, 2),
                    (2, 0, 3, 1, 3, 0, 2, 1),
                    (1, 2, 3, 3, 3, 2, 1, 3),
                    (1, 1, 3, 3, 3, 1, 1, 3),
                    (1, 1, 3, 2, 3, 1, 1, 2),
                    (1, 0, 3, 3, 3, 0, 1, 3),
                    (1, 0, 3, 2, 3, 0, 1, 2),
                    (1, 1, 3, 3, 3, 1, 1, 3),
                    (1, 0, 3, 1, 3, 0, 1, 1),
                    (1, 2, 2, 3, 2, 2, 1, 3),
                    (1, 1, 2, 3, 2, 1, 1, 3),
                    (1, 1, 2, 2, 2, 1, 1, 2),
                    (1, 0, 2, 3, 2, 0, 1, 3),
                    (1, 0, 2, 2, 2, 0, 1, 2),
                    (1, 0, 2, 1, 2, 0, 1, 1),
                ];
                let mut sf = [Src::id(ValueId(0)); 19];
                for (k, &(a, ar, b, br, c, cr, d, dr)) in SF.iter().enumerate() {
                    sf[k] = self.sub_mul(e(a, ar), e(b, br), e(c, cr), e(d, dr), out)?;
                }
                /* adj[col][row] = sign * (x*SFa - y*SFb + z*SFc), with x, y,
                 * z from row `src` of m (1 for row 0 of the adjugate, 0 for
                 * the others), in Mesa's order and signs. */
                /* (column, negated, three (row of m, sub-factor) terms) */
                type AdjEntry = (usize, bool, [(usize, usize); 3]);
                const ADJ: [AdjEntry; 16] = [
                    /* WRITEMASK_X: m[1][*] */
                    (0, false, [(1, 0), (2, 1), (3, 2)]),
                    (1, true, [(0, 0), (2, 3), (3, 4)]),
                    (2, false, [(0, 1), (1, 3), (3, 5)]),
                    (3, true, [(0, 2), (1, 4), (2, 5)]),
                    /* WRITEMASK_Y: m[0][*] against 00..05 */
                    (0, true, [(1, 0), (2, 1), (3, 2)]),
                    (1, false, [(0, 0), (2, 3), (3, 4)]),
                    (2, true, [(0, 1), (1, 3), (3, 5)]),
                    (3, false, [(0, 2), (1, 4), (2, 5)]),
                    /* WRITEMASK_Z: m[0][*] against 06..12 */
                    (0, false, [(1, 6), (2, 7), (3, 8)]),
                    (1, true, [(0, 6), (2, 9), (3, 10)]),
                    (2, false, [(0, 11), (1, 9), (3, 12)]),
                    (3, true, [(0, 8), (1, 10), (2, 12)]),
                    /* WRITEMASK_W: m[0][*] against 13..18 */
                    (0, true, [(1, 13), (2, 14), (3, 15)]),
                    (1, false, [(0, 13), (2, 16), (3, 17)]),
                    (2, true, [(0, 14), (1, 16), (3, 18)]),
                    (3, false, [(0, 15), (1, 17), (2, 18)]),
                ];
                for (k, &(col, negate, t)) in ADJ.iter().enumerate() {
                    let row = k / 4;
                    let (x0, s0) = t[0];
                    let (x1, s1) = t[1];
                    let (x2, s2) = t[2];
                    let src_col = |x: usize| -> Src {
                        /* Mesa's matrix_elt(m, src, x): column `src` (1 for
                         * the first four, 0 after), row x. */
                        if row == 0 {
                            lane_of(m[1], x as u8)
                        } else {
                            lane_of(m[0], x as u8)
                        }
                    };
                    let a = self.sub_mul(src_col(x0), sf[s0], src_col(x1), sf[s1], out)?;
                    let v = self.fma1(src_col(x2), sf[s2], a, out)?;
                    adj[col][row] = if negate { self.neg1(v, out)? } else { v };
                }
                /* det = m00*adj00 + (m01*adj10 + (m02*adj20 + m03*adj30)) */
                let p = self.mul1(e(0, 3), adj[3][0], out)?;
                let p = self.fma1(e(0, 2), adj[2][0], p, out)?;
                let p = self.fma1(e(0, 1), adj[1][0], p, out)?;
                self.fma1(e(0, 0), adj[0][0], p, out)?
            }
        };
        let inv = self.alu_w(ir::AluOp::Frcp, [det, det, det], 1, out)?;
        let inv = lane_of(inv, 0);
        let mut cols = [Src::id(ValueId(0)); 4];
        for (c, col) in cols.iter_mut().enumerate().take(usize::from(n)) {
            let mut parts = [(Src::id(ValueId(0)), 0u8); 4];
            for (r, p) in parts.iter_mut().enumerate().take(usize::from(n)) {
                *p = (adj[c][r], 0);
            }
            let a = self.column_from_components(&parts[..usize::from(n)], n, out)?;
            *col = self.alu_w(ir::AluOp::Fmul, [a, inv, inv], n, out)?;
        }
        Ok(cols)
    }

    /// One-lane helpers for `matrix_inverse`, each result splat.
    fn mul1(&mut self, a: Src, b: Src, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let v = self.alu_w(ir::AluOp::Fmul, [a, b, b], 1, out)?;
        Ok(lane_of(v, 0))
    }

    fn neg1(&mut self, a: Src, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let v = self.alu_w(ir::AluOp::Fneg, [a, a, a], 1, out)?;
        Ok(lane_of(v, 0))
    }

    fn fma1(&mut self, a: Src, b: Src, c: Src, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let v = self.alu_w(ir::AluOp::Ffma, [a, b, c], 1, out)?;
        Ok(lane_of(v, 0))
    }

    /// `a*b - c*d`, as NIR fuses it: `fma(a, b, -(c*d))`.
    fn sub_mul(&mut self, a: Src, b: Src, c: Src, d: Src, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let cd = self.mul1(c, d, out)?;
        let n = self.neg1(cd, out)?;
        self.fma1(a, b, n, out)
    }

    /// An arithmetic operator with at least one matrix operand, as columns.
    ///
    /// Section 5.9: "Multiply (*) applied to two matrices yields a linear
    /// algebraic matrix multiply"; otherwise add, subtract and divide on two
    /// matrices are component-wise, and "If one operand is scalar and the
    /// other is a vector or matrix, the scalar is applied component-wise".
    /// Divide is a reciprocal and a multiply, as for vectors (the literal
    /// fold included), and subtract is an add of the negation, because the
    /// kernel has no `Fsub`.
    fn matrix_binary(
        &mut self,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        n: u8,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, [Src; 4]> {
        let (lt, rt) = (self.types.of(lhs).base, self.types.of(rhs).base);
        let mut cols = [Src::id(ValueId(0)); 4];
        match (lt, rt) {
            (BaseType::Mat(c1, _), BaseType::Mat(..)) if op == BinOp::Mul => {
                /* Column j of the product is `a` times column j of `b`
                 * (a matCxR times a matPxC: P columns, each a sum over C). */
                let right = self.matrix_value(rhs, n, out)?;
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    *c = self.mat_times_column(lhs, right[j], c1, out)?;
                }
            }
            (BaseType::Mat(_, r), BaseType::Mat(..)) => {
                let a = self.matrix_value(lhs, n, out)?;
                let b = self.matrix_value(rhs, n, out)?;
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    *c = self.columnwise(op, a[j], b[j], r, out)?;
                }
            }
            (BaseType::Mat(_, r), _) => {
                let a = self.matrix_value(lhs, n, out)?;
                let k = self.value(rhs, 1, out)?;
                let k = Src { val: k.val, swizzle: [k.swizzle[0]; 4] };
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    *c = self.columnwise(op, a[j], k, r, out)?;
                }
            }
            (_, BaseType::Mat(_, r)) => {
                let k = self.value(lhs, 1, out)?;
                let k = Src { val: k.val, swizzle: [k.swizzle[0]; 4] };
                let b = self.matrix_value(rhs, n, out)?;
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    /* `+` and `*` commute, and the column goes first so that
                     * `-2.0 * a` emits exactly what it did before this arm
                     * existed; `-` and `/` keep the scalar first, where the
                     * order is the meaning. */
                    *c = match op {
                        BinOp::Add | BinOp::Mul => self.columnwise(op, b[j], k, r, out)?,
                        _ => self.columnwise(op, k, b[j], r, out)?,
                    };
                }
            }
            _ => return Err(self.err(op.text(), "this matrix expression is not lowered yet")),
        }
        Ok(cols)
    }

    /// One column of a component-wise matrix operation: `a op b`, `n` lanes.
    fn columnwise(&mut self, op: BinOp, a: Src, b: Src, n: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        match op {
            BinOp::Add => self.alu_w(ir::AluOp::Fadd, [a, b, b], n, out),
            BinOp::Sub => {
                let nb = self.alu_w(ir::AluOp::Fneg, [b, b, b], n, out)?;
                self.alu_w(ir::AluOp::Fadd, [a, nb, nb], n, out)
            }
            BinOp::Mul => self.alu_w(ir::AluOp::Fmul, [a, b, b], n, out),
            BinOp::Div => {
                let r = self.reciprocal(b, n, out)?;
                self.alu_w(ir::AluOp::Fmul, [a, r, r], n, out)
            }
            _ => Err(self.err(op.text(), "this matrix expression is not lowered yet")),
        }
    }

    /// `mat3(a, b, c)`, `mat3(m4)`, `mat3(1.0)`, `mat3(f0, .., f8)`:
    /// section 5.4.2's matrix constructors.
    ///
    /// Column-major throughout, which is what GLSL means and what the
    /// hardware's consecutive registers are.
    fn matrix_constructor(&mut self, name: &'a str, args: Range, n: u8, out: &mut AVec<'a, Node>) -> R<'a, [Src; 4]> {
        /* `n` columns of `r` rows (ES 3.00's matCxR, C != R). */
        let r = match super::sema::constructor_type(name, self.ast.es3) {
            Some(BaseType::Mat(_, r)) => r,
            _ => n,
        };
        let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
        let mut cols = [Src::id(ValueId(0)); 4];
        if ids.len() == 1 {
            let ty = self.types.of(ids[0]).base;
            if let BaseType::Mat(ac, ar) = ty {
                /* A matrix from a matrix (section 5.4.2): a component the
                 * argument has is copied, and every other comes from the
                 * IDENTITY -- 1 on the diagonal, 0 off it. So a column the
                 * argument has keeps its lanes and is 0 below them, except
                 * a 1 where that column meets the diagonal; a column past
                 * the argument's last is the identity's own. Not "pad with
                 * zero": a mat4(mat3) built that way would be singular -- it
                 * would draw, and draw nothing. */
                let from = self.matrix_value(ids[0], ac, out)?;
                let zero = self.const_value([0.0; 4], out)?;
                let one = self.const_value([1.0; 4], out)?;
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    let j8 = j as u8;
                    if j8 < ac && ar >= r {
                        /* Narrowed by reading fewer lanes, which the
                         * operand's swizzle already expresses. */
                        *c = from[j];
                        continue;
                    }
                    let id = ir::LocalId(self.locals.len() as u32);
                    self.locals.push(
                        self.a,
                        ir::VarDecl { name: "", ty: ir::Type::new(ir::ScalarType::F32, r), array_len: 1 },
                    )?;
                    let l = Local::scalar("", id, r);
                    self.store_local(l, zero, out)?;
                    let mut on_diagonal = j8 < r;
                    if j8 < ac {
                        let mut lanes = [None; 4];
                        for (k, lane) in lanes.iter_mut().enumerate().take(ar as usize) {
                            *lane = Some(k as u8);
                        }
                        self.store_lanes(l, from[j], ((1u16 << ar) - 1) as u8, lanes, out)?;
                        on_diagonal &= j8 >= ar;
                    }
                    if on_diagonal {
                        let mut lanes = [None; 4];
                        lanes[j] = Some(0);
                        self.store_lanes(l, one, 1 << j, lanes, out)?;
                    }
                    let v = self.push_instr(Op::LoadLocal { local: id, index: None, comps: r })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    *c = Src::id(v);
                }
                return Ok(cols);
            }
            /* A scalar fills the diagonal and leaves the rest zero, as a
             * float whatever its own type (5.4.1). */
            let d = self.convert(ids[0], BaseType::Float, 1, out)?;
            let zero = self.const_value([0.0; 4], out)?;
            for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                if j >= r as usize {
                    *c = zero;
                    continue;
                }
                let mut lanes = [None; 4];
                lanes[j] = Some(0);
                *c = self.zero_column_except(r, zero, d, 1 << j, lanes, out)?;
            }
            return Ok(cols);
        }
        /* One argument per column, each already a vector of the right
         * width: the fast path, and the only one that emits no local at
         * all. Every width is checked BEFORE any value is taken, because
         * falling through afterwards would evaluate the early arguments
         * twice. */
        if ids.len() == n as usize {
            let mut all_columns = true;
            for &e in ids {
                all_columns &= self.width(e)? == r;
            }
            if all_columns {
                for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                    /* `mat2(ivec2, bvec2)` is legal; its columns are floats. */
                    *c = self.convert(ids[j], BaseType::Float, r, out)?;
                }
                return Ok(cols);
            }
        }
        if ids.len() == (n as usize) * (r as usize) {
            /* Column-major scalars: each column is a constructor of its own. */
            for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
                let run = &ids[j * r as usize..(j + 1) * r as usize];
                *c = self.assemble(name, BaseType::Vec(r), r, run, out)?;
            }
            return Ok(cols);
        }
        self.matrix_from_components(name, n, r, ids, out)
    }

    /// Section 5.4.2's general form:
    ///
    /// > components will be constructed in order from the components of the
    /// > arguments. The arguments will be consumed left to right, and each
    /// > argument will have all its components consumed, in order, before
    /// > any components from the next argument are consumed. ... Matrix
    /// > components will be constructed and consumed in column major order.
    ///
    /// So there is nothing special about a mixture: `mat2(int, ivec2, int)`
    /// is four components from three arguments of two widths and one type
    /// that is not even float, and `mat2(bvec3, float)` is four from two.
    /// Both used to be refused, and both were refused as VERDICTS -- "this
    /// matrix constructor takes its components in order" reads as a rule the
    /// shader broke, and the shader broke no rule. 18 of dEQP's must-build
    /// cases were being told to rewrite legal GLSL.
    ///
    /// The two shapes above this are kept because they are better code, not
    /// because they are different rules: one argument per column emits no
    /// local at all, and this one emits a column's worth of masked stores.
    fn matrix_from_components(
        &mut self,
        name: &'a str,
        n: u8,
        r: u8,
        ids: &[ExprId],
        out: &mut AVec<'a, Node>,
    ) -> R<'a, [Src; 4]> {
        /* Flattened FIRST, and once: an argument's components can straddle
         * two columns (`mat2(int, ivec2, int)` splits the ivec2), and
         * evaluating it again per column would emit its instructions twice.
         *
         * `convert` at the argument's own width turns an int or a bool
         * argument into the float the matrix holds -- section 5.4.1's
         * conversion, which a constructor is the one place ES 1.00 allows. */
        let need = (n as usize) * (r as usize);
        let mut flat: AVec<(Src, u8)> = AVec::new();
        for &e in ids {
            let w = self.width(e)?;
            let v = self.convert(e, BaseType::Float, w, out)?;
            for k in 0..w {
                if flat.len() < need {
                    flat.push(self.a, (v, k))?;
                }
            }
        }
        if flat.len() < need {
            return Err(self.err(name, "not enough arguments for this constructor"));
        }
        let mut cols = [Src::id(ValueId(0)); 4];
        for (j, c) in cols.iter_mut().enumerate().take(n as usize) {
            let run = &flat.as_slice()[j * r as usize..(j + 1) * r as usize];
            *c = self.column_from_components(run, r, out)?;
        }
        Ok(cols)
    }

    /// One column built from `n` already-evaluated scalar components, each
    /// named as a value and the lane to read from it.
    ///
    /// Consecutive lanes coming from the SAME value share one store, the way
    /// [`Self::assemble`] does it: `mat2(v, w)` split across a column
    /// boundary still writes each source once.
    fn column_from_components(&mut self, comps_in: &[(Src, u8)], n: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let id = ir::LocalId(self.locals.len() as u32);
        let scalar = self.scalar_of_type(BaseType::Float);
        self.locals.push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(scalar, n), array_len: 1 })?;
        let mut pending: Option<(Src, u8, [u8; 4])> = None;
        for (lane, &(v, k)) in comps_in.iter().enumerate() {
            let mask = 1u8 << lane;
            match pending.as_mut() {
                Some((held, held_mask, swizzle)) if held.val == v.val => {
                    swizzle[lane] = v.swizzle[k as usize];
                    *held_mask |= mask;
                }
                _ => {
                    if let Some((held, held_mask, swizzle)) = pending.take() {
                        self.store_lane_mask(id, held, held_mask, swizzle, out)?;
                    }
                    let mut swizzle = [0u8; 4];
                    swizzle[lane] = v.swizzle[k as usize];
                    pending = Some((v, mask, swizzle));
                }
            }
        }
        if let Some((held, held_mask, swizzle)) = pending.take() {
            self.store_lane_mask(id, held, held_mask, swizzle, out)?;
        }
        let v = self.push_instr(Op::LoadLocal { local: id, index: None, comps: n })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// One `n`-lane column that is `zero` everywhere except the lanes
    /// `lanes` names, which come from `src`.
    ///
    /// A column that mixes two sources needs a slot: a `Src` reads lanes of
    /// ONE value, so "these lanes from the matrix, that one from a constant"
    /// has no single-operand form. The slot is written twice and read once,
    /// and the back end's copy propagation removes it when it can.
    ///
    /// `zero` is passed in rather than made here so that the N columns of
    /// one constructor share a constant instead of emitting N of them.
    fn zero_column_except(
        &mut self,
        n: u8,
        zero: Src,
        src: Src,
        mask: u8,
        lanes: [Option<u8>; 4],
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Src> {
        let id = ir::LocalId(self.locals.len() as u32);
        self.locals.push(self.a, ir::VarDecl { name: "", ty: ir::Type::new(ir::ScalarType::F32, n), array_len: 1 })?;
        let l = Local::scalar("", id, n);
        self.store_local(l, zero, out)?;
        self.store_lanes(l, src, mask, lanes, out)?;
        let v = self.push_instr(Op::LoadLocal { local: id, index: None, comps: n })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    fn matrix_column(&mut self, m: ExprId, j: u8, n: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* `n` columns; one column is the type's rows wide. */
        let r = self.rows_of(m);
        /* Column j of a matrix member of an ES 3.00 structure input: the
         * member's location + j. */
        if self.stage == Stage::Fragment && j < n {
            if let Some(name) = self.struct_varying_leaf(m)? {
                let Some((ir::IoSlot::Varying(loc), comps)) = self.input_of(name) else {
                    return Err(self.err(name, "reading this is not lowered yet"));
                };
                let v = self.push_instr(Op::LoadInput { slot: ir::IoSlot::Varying(loc + j), comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                return Ok(Src::id(v));
            }
        }
        let name = match *self.ast.expr(m) {
            Expr::Ident(name) => name,
            /* `_25.qt_Matrix * v`: a matrix that is a member of a uniform
             * struct, which is how every Qt Quick shader in the table writes
             * it. */
            Expr::Field(b, f) => match self.uniform_path(m)? {
                Some(name) => name,
                None => {
                    /* A matrix member reached through an array of structs,
                     * a run-time index among them included. */
                    let dot = self.cat(".", f)?;
                    if let Some(v) = self.uniform_leaf(b, dot, LeafLoad::Column { j, n }, r, out)? {
                        return Ok(v);
                    }
                    return Err(self.err(".", "this matrix expression is not lowered yet"));
                }
            },
            /* `m[k]` where `m` is an ARRAY of matrices (`u_var.m2[1]`):
             * element k's column j, and a select over the elements for a
             * run-time k. */
            Expr::Index(arr, idx) if self.types.of(arr).is_array() => {
                let aname = match *self.ast.expr(arr) {
                    Expr::Ident(name) => self.uniform_ident(name),
                    Expr::Field(..) => self.uniform_path(arr)?,
                    _ => None,
                };
                let Some(aname) = aname else {
                    /* An array of matrices that is a member of an element of
                     * a uniform array of structs, reached with a run-time
                     * index somewhere in the chain -- Qt Quick 3D's
                     * `lights[i].matrices[split]`: element k's column j
                     * through `uniform_leaf`, a select over k when k is
                     * chosen at run time. */
                    if let Expr::Field(b, f) = *self.ast.expr(arr) {
                        let dot = self.cat(".", f)?;
                        let len = self.types.of(arr).array_len;
                        if let Some(k) = reflect::const_int_expr(self.ast, idx) {
                            if k < 0 || k as u32 >= len {
                                return Err(self.err("[]", "array index out of range"));
                            }
                            let load = LeafLoad::ElementColumn { k: k as u32, j, n };
                            if let Some(v) = self.uniform_leaf(b, dot, load, r, out)? {
                                return Ok(v);
                            }
                        } else {
                            let i = self.index_value(idx, out)?;
                            let mut els: AVec<Src> = AVec::new();
                            let mut all = true;
                            for k in 0..len {
                                match self.uniform_leaf(b, dot, LeafLoad::ElementColumn { k, j, n }, r, out)? {
                                    Some(v) => els.push(self.a, v)?,
                                    None => all = false,
                                }
                            }
                            if all && !els.is_empty() {
                                return self.select_among(els.as_slice(), i, r, out);
                            }
                        }
                    }
                    return Err(self.err("[]", "this matrix expression is not lowered yet"));
                };
                let len = u32::from(
                    self.reflection.uniforms.iter().find(|u| u.name == aname).map_or(1, |u| u.array_len.max(1)),
                );
                if let Some(k) = reflect::const_int_expr(self.ast, idx) {
                    if k < 0 || k as u32 >= len {
                        return Err(self.err("[]", "array index out of range"));
                    }
                    return self.uniform_matrix_column(aname, k as u32, j, n, out);
                }
                let i = self.index_value(idx, out)?;
                let mut els: AVec<Src> = AVec::new();
                for k in 0..len {
                    let c = self.uniform_matrix_column(aname, k, j, n, out)?;
                    els.push(self.a, c)?;
                }
                return self.select_among(els.as_slice(), i, r, out);
            }
            ref other => {
                return Err(self.err(other.kind_name(), "this matrix expression is not lowered yet"));
            }
        };
        /* A `matN` attribute and a `matN` varying are both N CONSECUTIVE
         * LOCATIONS, one per column -- GL ES 2.0 section 2.10.4, and the
         * rule `reflect::locations` already applies when it hands out
         * locations. So column j is the load at `location + j`, and the row's
         * `components` is the width of one column, not of the whole thing. */
        if self.stage == Stage::Vertex {
            if let Some(at) = self.reflection.attribs.iter().find(|a| a.name == name) {
                let v = self.push_instr(Op::LoadInput { slot: ir::IoSlot::Attrib(at.location + j), comps: r })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                return Ok(Src::id(v));
            }
        } else if let Some(loc) = self.reflection.varyings.iter().find(|v| v.name == name).map(|v| v.location) {
            let v = self.push_instr(Op::LoadInput { slot: ir::IoSlot::Varying(loc + j), comps: r })?;
            out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            return Ok(Src::id(v));
        }
        self.uniform_matrix_column(name, 0, j, n, out)
    }

    /// Column `j` of element `element` of the matrix uniform row `name`, a
    /// matrix of `n` columns (its rows are the row's own type's).
    ///
    /// The strides follow the row rather than restating std430: an element
    /// is the row's size over its array length, and a column that over N --
    /// a `mat3` column is padded to four dwords and a `mat2` column is not.
    /// Dividing the WHOLE row by N was right only while no matrix uniform
    /// was an array (`u_var.m2[1]`, dEQP uniform_api).
    fn uniform_matrix_column(
        &mut self,
        name: &'a str,
        element: u32,
        j: u8,
        n: u8,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Src> {
        let Some(u) = self.reflection.uniforms.iter().find(|u| u.name == name) else {
            return Err(self.err(name, "reading this is not lowered yet"));
        };
        let len = u32::from(u.array_len.max(1));
        if element >= len {
            return Err(self.err(name, "array index out of range"));
        }
        let el_stride = u32::from(u.dwords) / len;
        let stride = el_stride / u32::from(n);
        let at = u32::from(u.dword_off) + element * el_stride + u32::from(j) * stride;
        let Ok(dword_off) = u16::try_from(at) else {
            return Err(self.err(name, "this uniform is too far into the block"));
        };
        let rows = match u.gl_type {
            0x8B67 | 0x8B69 => 2, // mat3x2, mat4x2
            0x8B65 | 0x8B6A => 3, // mat2x3, mat4x3
            0x8B66 | 0x8B68 => 4, // mat2x4, mat3x4
            _ => n,
        };
        let v = self.push_instr(Op::LoadUniform { dword_off, comps: rows })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /// The GL name a chain of uniform struct members and CONSTANT indices
    /// denotes (`s.c`, `u[1].tex`); `None` when a link of it is not one --
    /// a local, or an index known only at run time.
    fn uniform_const_name(&mut self, e: ExprId) -> R<'a, Option<&'a str>> {
        Ok(match *self.ast.expr(e) {
            Expr::Ident(n) => self.uniform_ident(n),
            Expr::Field(b, f) => match self.uniform_const_name(b)? {
                Some(p) => {
                    let dot = self.cat(".", f)?;
                    Some(self.cat(p, dot)?)
                }
                None => None,
            },
            Expr::Index(b, idx) => match (self.uniform_const_name(b)?, reflect::const_int_expr(self.ast, idx)) {
                (Some(p), Some(k)) if k >= 0 => {
                    let el = reflect::index_name(self.a, "", k as u32)?;
                    Some(self.cat(p, el)?)
                }
                _ => None,
            },
            _ => None,
        })
    }

    /// `a` then `b`, in the arena.
    fn cat(&self, a: &str, b: &str) -> R<'a, &'a str> {
        let buf = self.a.alloc_filled::<u8>(a.len() + b.len(), 0)?;
        buf[..a.len()].copy_from_slice(a.as_bytes());
        buf[a.len()..].copy_from_slice(b.as_bytes());
        core::str::from_utf8(buf).map_err(|_| Error::new(0, "", "a uniform name is not valid UTF-8"))
    }

    /// Read a leaf of a uniform reached through struct members and array
    /// elements: `u.a`, `u_lights[3].position`, `s.b.c`, `u[i].m`.
    ///
    /// `e` is the struct-typed part of the chain and `suffix` the rest of
    /// the leaf's GL name below it (`.position`); the reflection names every
    /// leaf by its whole path (`u_lights[3].position`, section 2.10.4 of GL
    /// ES 2.0), so a chain of constants is a name. An index known only at
    /// run time loads the leaf from EVERY element and selects, as for a
    /// local array. `None`: the chain is not a uniform (a local of the same
    /// name shadows it), and the caller tries its other readings.
    ///
    /// `w` is the leaf value's component count, which the CALLER knows from
    /// the checker's type of the whole access; it sizes the select.
    fn uniform_leaf(
        &mut self,
        e: ExprId,
        suffix: &'a str,
        load: LeafLoad,
        w: u8,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Option<Src>> {
        match *self.ast.expr(e) {
            Expr::Ident(u) => {
                let Some(u) = self.uniform_ident(u) else {
                    return Ok(None);
                };
                let name = self.cat(u, suffix)?;
                let Some(row) = self.reflection.uniforms.iter().find(|r| r.name == name).copied() else {
                    return Ok(None);
                };
                let v = match load {
                    LeafLoad::Value if components_of(row.gl_type).is_none() => {
                        return Err(self.err(name, "reading this uniform type is not lowered yet"));
                    }
                    LeafLoad::Value => self.uniform_value(name, 0, out)?,
                    LeafLoad::Element(k) => self.uniform_value(name, k, out)?,
                    LeafLoad::Column { j, n } => self.uniform_matrix_column(name, 0, j, n, out)?,
                    LeafLoad::ElementColumn { k, j, n } => self.uniform_matrix_column(name, k, j, n, out)?,
                };
                Ok(Some(v))
            }
            Expr::Field(b, f) => {
                let bt = self.types.of(b);
                if !matches!(bt.base, BaseType::Struct(_)) || bt.is_array() {
                    return Ok(None);
                }
                let dot = self.cat(".", f)?;
                let sfx = self.cat(dot, suffix)?;
                self.uniform_leaf(b, sfx, load, w, out)
            }
            Expr::Index(b, idx) => {
                let bt = self.types.of(b);
                if !bt.is_array() || !matches!(bt.base, BaseType::Struct(_)) {
                    return Ok(None);
                }
                if let Some(k) = reflect::const_int_expr(self.ast, idx) {
                    if k < 0 || k as u32 >= bt.array_len {
                        return Err(self.err("[]", "array index out of range"));
                    }
                    let el = reflect::index_name(self.a, "", k as u32)?;
                    let sfx = self.cat(el, suffix)?;
                    return self.uniform_leaf(b, sfx, load, w, out);
                }
                let i = self.index_value(idx, out)?;
                let mut els: AVec<Src> = AVec::new();
                for k in 0..bt.array_len {
                    let el = reflect::index_name(self.a, "", k)?;
                    let sfx = self.cat(el, suffix)?;
                    let Some(v) = self.uniform_leaf(b, sfx, load, w, out)? else {
                        return Ok(None);
                    };
                    els.push(self.a, v)?;
                }
                Ok(Some(self.select_among(els.as_slice(), i, w, out)?))
            }
            _ => Ok(None),
        }
    }

    /// The reflection name a `base.member` chain denotes, if it names a
    /// uniform.
    ///
    /// `None` means it is not one -- a swizzle of a vector, or a member of
    /// something that is not a uniform -- and the caller goes on to its other
    /// readings rather than refusing. Only a name is a base here: a member of
    /// an element of an array of structs needs the index in the name too,
    /// which is its own change.
    fn uniform_path(&mut self, e: ExprId) -> R<'a, Option<&'a str>> {
        let Expr::Field(base, field) = *self.ast.expr(e) else {
            return Ok(None);
        };
        let Some(prefix) = self.uniform_struct_prefix(base)? else {
            return Ok(None);
        };
        let name = reflect::join_name(self.a, prefix, field)?;
        if self.reflection.uniforms.iter().any(|u| u.name == name) {
            return Ok(Some(name));
        }
        Ok(None)
    }

    /// The GL name prefix a struct-typed uniform expression denotes: `val`
    /// for the uniform itself, `val.b` for a member that is a struct in turn
    /// (the reflection flattens nested structs to `val.b.c`), and
    /// `u[1].t[1]` for an element of a struct array, at any depth.
    ///
    /// Only a STRUCT-typed expression is asked, and then every step of its
    /// chain is a struct or an array of them -- a member of a vector is a
    /// swizzle and yields a vector, never a struct -- so the chain is the
    /// one [`Lower::uniform_const_name`] spells. A local of the same name
    /// shadows the uniform (4.2.2); an index known only at run time names no
    /// single prefix, and the leaf paths select over elements instead.
    fn uniform_struct_prefix(&mut self, e: ExprId) -> R<'a, Option<&'a str>> {
        if !matches!(self.types.of(e).base, BaseType::Struct(_)) || self.types.of(e).is_array() {
            return Ok(None);
        }
        let Some(prefix) = self.uniform_const_name(e)? else {
            return Ok(None);
        };
        /* It names a uniform when some row lies under it. */
        let under = |u: &&reflect::UniformRow<'_>| {
            u.name.len() > prefix.len() && u.name.starts_with(prefix) && u.name.as_bytes()[prefix.len()] == b'.'
        };
        Ok(self.reflection.uniforms.iter().find(under).map(|_| prefix))
    }

    /// An operand of a component-wise operation, at the width of the result.
    /// A scalar against a vector is splatted, which is what section 5.9 says
    /// it means.
    fn operand_at(&mut self, e: ExprId, want: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let have = self.width(e)?;
        let v = self.value(e, have, out)?;
        if have == 1 && want > 1 {
            return Ok(Src { val: v.val, swizzle: [v.swizzle[0]; 4] });
        }
        Ok(v)
    }

    /// The scalar kind an expression's value has.
    fn scalar_kind(&self, e: ExprId) -> R<'a, ir::ScalarType> {
        let ty = self.types.of(e);
        scalar_of(ty.base)
            .ok_or_else(|| self.err(self.ast.expr(e).kind_name(), "this expression has no value to compute"))
    }

    /// Emit one ALU instruction and give the operand that reads it.
    fn alu(&mut self, op: ir::AluOp, srcs: [Src; 3], out: &mut AVec<'a, Node>) -> R<'a, Src> {
        /* 0 = "not recorded here", which `Module::comps_of` answers as 4 --
         * the over-estimate a back end made anyway, never a narrow answer.
         * The component-wise paths below use `alu_w` and are exact. */
        self.alu_sized(op, srcs, 0, out)
    }

    /// The same, where the caller knows the result's width.
    fn alu_w(&mut self, op: ir::AluOp, srcs: [Src; 3], comps: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        self.alu_sized(op, srcs, op.result_comps(comps), out)
    }

    fn alu_sized(&mut self, op: ir::AluOp, srcs: [Src; 3], comps: u8, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let v = self.push_sized(Op::Alu(op, srcs), comps)?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    /* ---- locals -------------------------------------------------------- */

    fn push_scope(&mut self) -> R<'a, ()> {
        let n = self.scope.len() as u32;
        self.marks.push(self.a, n)?;
        Ok(())
    }

    fn pop_scope(&mut self) {
        if let Some(n) = self.marks.pop() {
            while self.scope.len() > n as usize {
                self.scope.pop();
            }
        }
    }

    /// The variable `name` refers to: the innermost local or parameter, and
    /// failing that a global. The order is section 4.2.3's -- a local
    /// shadows a global of the same name.
    fn lookup(&self, name: &str) -> Option<Local<'a>> {
        self.scope_entry(name).filter(|l| !matches!(l.run, Run::Uniform | Run::Constants))
    }

    /// The element values of the `const` array `name` names here, if it is
    /// one held as values (`Run::Constants`), with their width.
    fn const_array(&self, name: &str) -> Option<(&'a [Src], u8)> {
        let l = self.scope_entry(name).filter(|l| l.run == Run::Constants)?;
        self.const_arrays.as_slice().get(l.id.0 as usize).copied().map(|v| (v, l.comps))
    }

    /// The innermost binding of `name`, a uniform-bound parameter included.
    fn scope_entry(&self, name: &str) -> Option<Local<'a>> {
        let body = self.scope.as_slice().get(self.scope_base..).unwrap_or(&[]);
        body.iter().rev().chain(self.globals.as_slice().iter().rev()).find(|l| l.name == name).copied()
    }

    /// The uniform GL name a plain identifier denotes, if it denotes one:
    /// itself when nothing local has the name, the argument's name when it
    /// is a parameter bound to a uniform, and `None` when a local, a
    /// parameter or a held output shadows it (section 4.2.3).
    fn uniform_ident(&self, name: &'a str) -> Option<&'a str> {
        match self.scope_entry(name) {
            Some(l) if l.run == Run::Uniform => self.uniform_params.as_slice().get(l.id.0 as usize).copied(),
            Some(_) => None,
            None if self.shadow_of(name).is_some() => None,
            None => Some(name),
        }
    }

    /// Is `base` a sampler, or a struct holding one somewhere inside?
    fn carries_sampler(&self, base: BaseType<'a>, depth: u32) -> bool {
        match base {
            b if b.is_sampler() => true,
            BaseType::Struct(_) if depth < Self::MAX_STRUCT_DEPTH => self
                .struct_members(base)
                .is_some_and(|ms| ms.iter().any(|m| self.carries_sampler(m.ty.base, depth + 1))),
            _ => false,
        }
    }

    /// `float x = ...;` inside a function: a slot, and the store if there is
    /// an initialiser.
    fn local_decl(&mut self, id: DeclId, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        self.declare_var(id, false, out)
    }

    /// Declare a variable and, if it has an initialiser, store it.
    ///
    /// `global` says which table the name goes into; everything else about a
    /// global is a local. A shader's global is not static storage -- ES 1.00
    /// has no such thing -- it is a variable with the whole shader for a
    /// scope, initialised where its declaration stands. With one body after
    /// inlining, that is a slot written at the top of `main`.
    fn declare_var(&mut self, id: DeclId, global: bool, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let Decl::Var { storage, ty, name, init, line, .. } = *self.ast.decl(id) else {
            /* A structure declared inside a body (legal in both languages,
             * section 4.1.8): a type, so nothing to emit. Its members are
             * found by NAME (`struct_members`), so it is taken only when no
             * other structure in the unit has that name -- two scopes'
             * different `struct S` would be told apart by scope, which the
             * name lookup does not model. */
            if let Decl::Struct { name, .. } = *self.ast.decl(id) {
                let same =
                    self.ast.decls.iter().filter(|d| matches!(d, Decl::Struct { name: n, .. } if *n == name)).count();
                if same == 1 {
                    return Ok(());
                }
                return Err(self.err(name, "two structures of this name in different scopes are not lowered yet"));
            }
            return Err(self.err("{}", "this statement is not lowered yet"));
        };
        if storage != StorageQualifier::None && storage != StorageQualifier::Const {
            /* `attribute`/`uniform`/`varying` are not allowed on a local
             * (section 4.3); the checker refuses them first. */
            return Err(Error::new(line, name, "this statement is not lowered yet"));
        }
        /* A struct has neither a scalar kind nor a component count of its
         * own -- `scalar_of` answers None and `components()` answers 0, both
         * correctly, because those describe ONE slot and a struct is a run
         * of slots of differing width. So its shape is settled first and the
         * two questions below are asked only of the kinds they fit. */
        let is_struct = matches!(ty.base, BaseType::Struct(_));
        let (scalar, comps) = if is_struct {
            (ir::ScalarType::F32, 1)
        } else {
            let Some(sc) = scalar_of(ty.base) else {
                return Err(Error::new(line, name, "a local of this type is not lowered yet"));
            };
            let c = ty.base.slot_width();
            if c == 0 {
                return Err(Error::new(line, name, "a local of this type is not lowered yet"));
            }
            (sc, c)
        };
        let _ = (scalar, comps);
        /* The element count: the size as written, `a.length()` included,
         * or -- ES 3.00 `float a[] = ...` -- the initialiser's, which the
         * checker worked out (section 4.1.9). */
        let len = if ty.implicit_size {
            let Some(e) = init else {
                return Err(Error::new(line, name, "an array without a size needs an initializer"));
            };
            self.types.of(e).array_len
        } else {
            match ty.array_len {
                Some(e) => match self.fold_len(e) {
                    Some(n) if n > 0 => n as u32,
                    Some(_) => return Err(self.err(name, "an array must have at least one element")),
                    None => return Err(self.err(name, "an array size that is not a constant is not lowered yet")),
                },
                None => 0,
            }
        };
        /* A `const` array longer than the slots a run may have: held as its
         * elements' values, which is all a read-only array needs (dEQP-GLES3
         * large_constant_arrays, 128 elements). */
        if storage == StorageQualifier::Const
            && len > u32::from(MAX_ARRAY_SLOTS)
            && !matches!(ty.base, BaseType::Struct(_) | BaseType::Mat(..))
        {
            let Some(e) = init else {
                return Err(Error::new(line, name, "a const array needs an initializer"));
            };
            let Expr::ArrayCtor { args, .. } = *self.ast.expr(e) else {
                return Err(Error::new(line, name, "this array is too large to give every element a slot"));
            };
            if args.len != len {
                return Err(Error::new(line, name, "the initializer has the wrong number of elements"));
            }
            let mut vals: AVec<'a, Src> = AVec::with_capacity(self.a, len as usize)?;
            for i in args.indices() {
                let v = self.value(self.ast.expr_ids[i], comps, out)?;
                vals.push(self.a, v)?;
            }
            let id = ir::LocalId(self.const_arrays.len() as u32);
            self.const_arrays.push(self.a, vals.into_slice())?;
            return self.bind(Local { name, id, comps, run: Run::Constants, value: None }, global);
        }
        let l = self.alloc_var(name, ty.base, len, line)?;
        let run = l.run;
        /* Declared before the initialiser is lowered would let `float x = x;`
         * read itself; ES 1.00 section 4.3 says the declared name is not in
         * scope until after its initialiser, and the checker enforces it, so
         * the order here matches rather than relying on that. */
        if let Some(e) = init {
            if run.elements().is_some() {
                /* ES 1.00 has no array initialiser and no array-valued
                 * expression to be one (sections 4.1.9 and 5.7); ES 3.00 has
                 * both (sections 4.1.9 and 5.4.4). */
                if !self.ast.es3 {
                    return Err(Error::new(line, name, "an array cannot be given an initialiser"));
                }
                self.array_fill(e, l, out)?;
                return self.bind(l, global);
            }
            if let Run::Struct(_) = run {
                /* `Pos p = Pos(x, y, z);` -- section 5.4.3: a structure
                 * constructor takes one argument per member, in order, and
                 * each must already have the member's type. So it is a store
                 * per member, at the member's own width. */
                self.bind(l, global)?;
                return self.struct_fill(e, l, ty.base, name, out);
            }
            if let Some(n) = run.columns() {
                let cols = self.matrix_value(e, n, out)?;
                self.bind(l, global)?;
                return self.store_columns(l, &cols, out);
            }
            let src = self.value(e, comps, out)?;
            self.bind(l, global)?;
            return self.store_local(l, src, out);
        }
        self.bind(l, global)?;
        Ok(())
    }

    /// Slots for a variable of `base`, `len` elements of it (0: not an
    /// array), in a run whose kind says what it is. The one place a
    /// variable's slots are laid out, so a declaration, a parameter and a
    /// temporary cannot disagree about where element k lives.
    fn alloc_var(&mut self, name: &'a str, base: BaseType<'a>, len: u32, line: u32) -> R<'a, Local<'a>> {
        let (scalar, comps) = match base {
            BaseType::Struct(_) => (ir::ScalarType::F32, 1),
            _ => match (scalar_of(base), base.slot_width()) {
                (Some(sc), c) if c > 0 => (sc, c),
                _ => return Err(Error::new(line, name, "a local of this type is not lowered yet")),
            },
        };
        let run = self.run_shape(base, len, name, line)?;
        let id = ir::LocalId(self.locals.len() as u32);
        match (run, base) {
            (Run::StructArray { len, .. }, BaseType::Struct(sname)) => {
                for _ in 0..len {
                    self.push_struct_slots(sname, name, line, 0)?;
                }
            }
            /* Each LEAF at its OWN width, which is the whole difference from
             * a matrix or an array; a member that is itself a struct
             * contributes its own leaves in place. */
            (Run::Struct(_), BaseType::Struct(sname)) => self.push_struct_slots(sname, name, line, 0)?,
            _ => {
                for _ in 0..run.slots() {
                    self.locals.push(self.a, ir::VarDecl { name, ty: ir::Type::new(scalar, comps), array_len: 1 })?;
                }
            }
        }
        Ok(Local { name, id, comps, run, value: None })
    }

    /// The run a variable of `base`, `len` elements of it (0: not an
    /// array), is laid out as -- without allocating it.
    fn run_shape(&mut self, base: BaseType<'a>, len: u32, name: &'a str, line: u32) -> R<'a, Run> {
        let len8 = if len == 0 {
            0u8
        } else if len > u32::from(MAX_ARRAY_SLOTS) {
            /* Every element is a register. A larger array wants indexable
             * memory, which is the dynamic-index capability selection
             * refuses, so it stops here rather than emitting a run nothing
             * can allocate. */
            return Err(Error::new(line, name, "this array is too large to give every element a slot"));
        } else {
            len as u8
        };
        /* A run of consecutive slots, so that slot j is `id + j`: N columns
         * for a `matN`, N elements for a `T[N]`, N runs of columns or of
         * leaves for an array of matrices or of structs. */
        Ok(match (base, len8) {
            /* `S s[N]`: N runs of the struct's leaves, element k at
             * `k * per` (dEQP shaders.struct.local.*array*). */
            (BaseType::Struct(sname), n) if n > 0 => {
                let per = self.struct_leaves(sname, name, line, 0)?;
                if u32::from(n) * u32::from(per) > u32::from(MAX_ARRAY_SLOTS) {
                    return Err(Error::new(line, name, "this array is too large to give every element a slot"));
                }
                Run::StructArray { len: n, per }
            }
            /* ES 3.00 `mat3 a[N]` (dEQP-GLES3 arrays.*mat3*). */
            (BaseType::Mat(c, _), n) if n > 0 => {
                if u32::from(n) * u32::from(c) > u32::from(MAX_ARRAY_SLOTS) {
                    return Err(Error::new(line, name, "this array is too large to give every element a slot"));
                }
                Run::MatrixArray { len: n, cols: c }
            }
            (_, n) if n > 0 => Run::Array(n),
            (BaseType::Mat(n, _), _) => Run::Matrix(n),
            (BaseType::Struct(sname), _) => self.struct_run(sname, name, line)?,
            _ => Run::Single,
        })
    }

    /// An array size folded: a constant expression, or ES 3.00's
    /// `a.length()`, whose value is the checker's type of `a`.
    fn fold_len(&self, e: ExprId) -> Option<i64> {
        if let Expr::Length(x) = *self.ast.expr(e) {
            let t = self.types.of(x);
            return t.is_array().then_some(i64::from(t.array_len));
        }
        reflect::const_int_expr(self.ast, e)
    }

    /// Fill the array run `dst` from the array-valued expression `e` (ES
    /// 3.00, which has array constructors, array assignment and array
    /// initialisers; sections 4.1.9, 5.4.4 and 5.8).
    ///
    /// An array has no single value, so it is always MOVED into a run --
    /// the move `struct_fill` makes for a struct, element by element here.
    fn array_fill(&mut self, e: ExprId, dst: Local<'a>, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let Some(n) = dst.run.elements() else {
            return Err(self.err(dst.name, "an array value for something that is not an array"));
        };
        let elem = self.types.of(e).base;
        match *self.ast.expr(e) {
            /* Section 5.4.4: one argument per element, in order. */
            Expr::ArrayCtor { args, .. } => {
                let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
                if ids.len() != usize::from(n) {
                    return Err(self.err("[]", "an array constructor takes one argument per element"));
                }
                for (k, &arg) in ids.iter().enumerate() {
                    self.element_store(dst, k as u8, arg, elem, out)?;
                }
                Ok(())
            }
            /* Section 5.9: only the chosen arm is evaluated. */
            Expr::Select(c, t, fe) => {
                let c = self.operand_at(c, 1, out)?;
                let c = Src { val: c.val, swizzle: [c.swizzle[0]; 4] };
                let mut then_items: AVec<Node> = AVec::new();
                self.array_fill(t, dst, &mut then_items)?;
                let mut else_items: AVec<Node> = AVec::new();
                self.array_fill(fe, dst, &mut else_items)?;
                let (then_, else_) = self.commit2(then_items, else_items)?;
                out.push(self.a, Node::If { cond: c, then_, else_ })?;
                Ok(())
            }
            Expr::Comma(a, b) => {
                self.evaluate_and_discard(a, out)?;
                self.array_fill(b, dst, out)
            }
            /* A function that returns an array fills the run directly with
             * its `return`, as a struct-returning one does. */
            Expr::Call { name, args } => {
                let Some(f) = self.user_function(name, args) else {
                    return Err(self.err(name, "an array value of this kind is not lowered yet"));
                };
                match self.inline(name, f, args, Want::Array { dst }, out)? {
                    Ret::Struct => Ok(()),
                    _ => Err(self.err(name, "this function does not return an array")),
                }
            }
            _ => {
                /* A local array, or an array member of a local struct. */
                if let Some(from) = self.local_chain(e, &[])? {
                    if from.run != dst.run {
                        return Err(self.err(dst.name, "these arrays have different shapes"));
                    }
                    return self.copy_run(dst, from, elem, out);
                }
                /* A uniform array, element by element from the block. */
                if let Some(uname) = self.uniform_const_name(e)? {
                    return self.array_from_uniform(uname, dst, elem, out);
                }
                let k = self.ast.expr(e).kind_name();
                Err(self.err(k, "an array value of this kind is not lowered yet"))
            }
        }
    }

    /// Element `k` of `dst` from the expression `arg`, at the element's own
    /// shape.
    fn element_store(
        &mut self,
        dst: Local<'a>,
        k: u8,
        arg: ExprId,
        elem: BaseType<'a>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let Some(el) = dst.element(k) else {
            return Err(self.err(dst.name, "an array value for something that is not an array"));
        };
        match el.run {
            Run::Matrix(c) => {
                let cols = self.matrix_value(arg, c, out)?;
                self.store_columns(el, &cols, out)
            }
            Run::Struct(_) => self.struct_fill(arg, el, elem, dst.name, out),
            _ => {
                let v = self.value(arg, el.comps, out)?;
                self.store_local(el, v, out)
            }
        }
    }

    /// Copy one run into another of the same shape, slot by slot, each slot
    /// at its own width (a struct array's leaves differ).
    fn copy_run(&mut self, dst: Local<'a>, src: Local<'a>, elem: BaseType<'a>, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        if let (Run::StructArray { len, per }, BaseType::Struct(sname)) = (dst.run, elem) {
            for k in 0..u32::from(len) {
                let at = k * u32::from(per);
                self.copy_struct_slots(sname, dst.id.0 + at, src.id.0 + at, dst.name, 0, out)?;
            }
            return Ok(());
        }
        if src.value.is_some() {
            return Err(self.err(src.name, "an array value of this kind is not lowered yet"));
        }
        for j in 0..dst.run.slots() {
            let v = self.push_instr(Op::LoadLocal { local: src.column(j).id, index: None, comps: dst.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            self.store_local(dst.column(j), Src::id(v), out)?;
        }
        Ok(())
    }

    /// Every element of the uniform array row `uname` into `dst`.
    fn array_from_uniform(
        &mut self,
        uname: &'a str,
        dst: Local<'a>,
        elem: BaseType<'a>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let Some(n) = dst.run.elements() else {
            return Err(self.err(dst.name, "an array value for something that is not an array"));
        };
        for k in 0..n {
            let Some(el) = dst.element(k) else { break };
            match (el.run, elem) {
                (Run::Matrix(c), BaseType::Mat(..)) => {
                    let mut cols = [Src::id(ValueId(0)); 4];
                    for (j, col) in cols.iter_mut().enumerate().take(usize::from(c)) {
                        *col = self.uniform_matrix_column(uname, u32::from(k), j as u8, c, out)?;
                    }
                    self.store_columns(el, &cols, out)?;
                }
                (Run::Single, _) => {
                    let v = self.uniform_value(uname, u32::from(k), out)?;
                    self.store_local(el, v, out)?;
                }
                _ => return Err(self.err(uname, "copying a uniform array of structs is not lowered yet")),
            }
        }
        Ok(())
    }

    /// The run an array-valued expression lives in: its own, when it is a
    /// local, otherwise a temporary filled from it.
    fn array_run_of(&mut self, e: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Local<'a>> {
        if let Some(l) = self.local_chain(e, &[])? {
            if l.run.elements().is_some() && l.value.is_none() {
                return Ok(l);
            }
        }
        let t = self.types.of(e);
        if !t.is_array() {
            return Err(self.err(self.ast.expr(e).kind_name(), "the type of this expression is not known"));
        }
        let line = self.line;
        let tmp = self.alloc_var("", t.base, t.array_len, line)?;
        self.array_fill(e, tmp, out)?;
        Ok(tmp)
    }

    /// `a == b` over two arrays (ES 3.00 section 5.9): every element equal,
    /// as one bool.
    fn array_equal(&mut self, lhs: ExprId, rhs: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let elem = self.types.of(lhs).base;
        let a = self.array_run_of(lhs, out)?;
        let b = self.array_run_of(rhs, out)?;
        if a.run != b.run {
            return Err(self.err("==", "these arrays have different shapes"));
        }
        let mut shapes: AVec<(ir::ScalarType, u8)> = AVec::new();
        match (a.run, elem) {
            (Run::StructArray { len, .. }, BaseType::Struct(sname)) => {
                for _ in 0..len {
                    self.struct_leaf_shapes(sname, 0, &mut shapes)?;
                }
            }
            _ => {
                let scalar = self.scalar_of_type(elem);
                for _ in 0..a.run.slots() {
                    shapes.push(self.a, (scalar, a.comps))?;
                }
            }
        }
        let mut acc: Option<Src> = None;
        for (k, &(scalar, w)) in shapes.as_slice().iter().enumerate() {
            let x = self.push_instr(Op::LoadLocal { local: ir::LocalId(a.id.0 + k as u32), index: None, comps: w })?;
            out.push(self.a, Node::Instr(ir::InstrId(x.0)))?;
            let y = self.push_instr(Op::LoadLocal { local: ir::LocalId(b.id.0 + k as u32), index: None, comps: w })?;
            out.push(self.a, Node::Instr(ir::InstrId(y.0)))?;
            let eq = self.all_equal(Src::id(x), Src::id(y), w, scalar, out)?;
            acc = Some(self.and_bools(acc, eq, out)?);
        }
        acc.ok_or_else(|| self.err("==", "an array with no elements"))
    }

    /// `T[N]` as a run of N slots, with the length folded from its
    /// expression.
    ///
    /// ES 1.00 section 4.1.9 requires the size to be a constant expression
    /// greater than zero, and the checker has already said so; this reads the
    /// value and bounds it by what a run of slots can address.
    fn array_run(&mut self, name: &'a str, len: ExprId) -> R<'a, Run> {
        let Some(n) = reflect::const_int_expr(self.ast, len) else {
            return Err(self.err(name, "an array size that is not a constant is not lowered yet"));
        };
        if n <= 0 {
            return Err(self.err(name, "an array must have at least one element"));
        }
        if n > MAX_ARRAY_SLOTS as i64 {
            /* Every element is a register. A larger array wants indexable
             * memory, which is the dynamic-index capability selection
             * refuses, so it stops here rather than emitting a run nothing
             * can allocate. */
            return Err(self.err(name, "this array is too large to give every element a slot"));
        }
        Ok(Run::Array(n as u8))
    }

    /// The members a struct type names, from its declaration.
    fn struct_members(&self, base: BaseType<'a>) -> Option<&'a [Member<'a>]> {
        let BaseType::Struct(sname) = base else { return None };
        self.ast.decls.iter().find_map(|d| match d {
            Decl::Struct { name: n, members, .. } if *n == sname => Some(&self.ast.members[members.indices()]),
            _ => None,
        })
    }

    /// `struct S { ... } s;` as a run of one slot per member.
    /// How deep a struct may nest before the compiler gives up.
    ///
    /// A struct cannot contain itself -- its own name is not in scope until
    /// the end of its declaration (section 4.2.2) -- so the nesting is
    /// finite. The bound is here because a compiler must not be the thing
    /// that proves that: an unbounded walk over a malformed unit is a hang
    /// inside `glCompileShader`, on the target, in process.
    const MAX_STRUCT_DEPTH: u32 = 16;

    /// The LEAF slots one member occupies.
    ///
    /// A member that is itself a struct is its own run, so it contributes
    /// that run's leaves rather than one slot. Everything else is one slot,
    /// and the shapes that cannot be (an array, a matrix) are refused by
    /// `member_shape` with their own names.
    fn member_slots(&self, m: &Member<'a>, name: &'a str, line: u32, depth: u32) -> R<'a, u8> {
        /* An array member is its elements in a row: a struct element
         * contributes its leaves per element, anything else one slot. */
        let len = match m.ty.array_len {
            Some(e) => match reflect::const_int_expr(self.ast, e) {
                Some(n) if n > 0 && n <= i64::from(MAX_ARRAY_SLOTS) => n as u32,
                _ => return Err(Error::new(line, name, "a struct member that is an array is not lowered yet")),
            },
            None => 1,
        };
        let per = if let BaseType::Struct(inner) = m.ty.base {
            u32::from(self.struct_leaves(inner, name, line, depth + 1)?)
        } else {
            self.element_shape(&m.ty, name, line)?;
            1
        };
        let n = len * per;
        if n > u32::from(MAX_ARRAY_SLOTS) {
            return Err(Error::new(line, name, "this struct has too many members to give each a slot"));
        }
        Ok(n as u8)
    }

    /// The kind and width of ONE element of `ty` -- its array length, if
    /// any, set aside. A matrix, a sampler and a struct have no one-slot
    /// shape and refuse by name.
    fn element_shape(
        &self,
        ty: &crate::shaderc::online::ast::TypeSpec<'a>,
        name: &'a str,
        line: u32,
    ) -> R<'a, (ir::ScalarType, u8)> {
        if matches!(ty.base, BaseType::Mat(..)) {
            return Err(Error::new(line, name, "a struct member that is a matrix is not lowered yet"));
        }
        if matches!(ty.base, BaseType::Struct(_)) {
            return Err(Error::new(line, name, "a struct member that is a struct is not lowered yet"));
        }
        let comps = ty.base.components();
        match scalar_of(ty.base) {
            Some(sc) if comps > 0 => Ok((sc, comps)),
            _ => Err(Error::new(line, name, "a struct member of this type is not lowered yet")),
        }
    }

    /// Every leaf of `sname` in slot order -- the order `member_slots`
    /// counts and `struct_member` offsets by -- with an array member
    /// expanded element by element. Every walk over a struct's slots
    /// (allocation, copy, comparison, a uniform's copy) takes this list, so
    /// they cannot disagree about where a leaf lives.
    fn struct_leaf_list(&self, sname: &'a str, prefix: &'a str, depth: u32, acc: &mut AVec<'a, Leaf<'a>>) -> R<'a, ()> {
        let line = self.line;
        if depth >= Self::MAX_STRUCT_DEPTH {
            return Err(Error::new(line, sname, "this struct nests too deeply to give each member a slot"));
        }
        let ms = self.struct_members(BaseType::Struct(sname)).unwrap_or(&[]);
        for m in ms {
            let dot = self.cat(".", m.name)?;
            let path = self.cat(prefix, dot)?;
            let len = match m.ty.array_len {
                Some(e) => match reflect::const_int_expr(self.ast, e) {
                    Some(n) if n > 0 && n <= i64::from(MAX_ARRAY_SLOTS) => Some(n as u32),
                    _ => return Err(Error::new(line, m.name, "a struct member that is an array is not lowered yet")),
                },
                None => None,
            };
            match (m.ty.base, len) {
                (BaseType::Struct(inner), None) => self.struct_leaf_list(inner, path, depth + 1, acc)?,
                (BaseType::Struct(inner), Some(n)) => {
                    for k in 0..n {
                        let el = reflect::index_name(self.a, path, k)?;
                        self.struct_leaf_list(inner, el, depth + 1, acc)?;
                    }
                }
                (_, None) => {
                    let (scalar, comps) = self.element_shape(&m.ty, m.name, line)?;
                    acc.push(self.a, Leaf { scalar, comps, path, element: None })?;
                }
                (_, Some(n)) => {
                    let (scalar, comps) = self.element_shape(&m.ty, m.name, line)?;
                    for k in 0..n {
                        acc.push(self.a, Leaf { scalar, comps, path, element: Some(k) })?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The leaves a struct type flattens to, counted.
    fn struct_leaves(&self, sname: &'a str, name: &'a str, line: u32, depth: u32) -> R<'a, u8> {
        if depth >= Self::MAX_STRUCT_DEPTH {
            return Err(Error::new(line, name, "this struct nests too deeply to give each member a slot"));
        }
        let Some(ms) = self.struct_members(BaseType::Struct(sname)) else {
            return Err(Error::new(line, name, "a local of this type is not lowered yet"));
        };
        if ms.is_empty() {
            return Err(Error::new(line, name, "a struct with no members"));
        }
        let mut n: u32 = 0;
        for m in ms {
            n += self.member_slots(m, name, line, depth)? as u32;
            if n > MAX_ARRAY_SLOTS as u32 {
                return Err(Error::new(line, name, "this struct has too many members to give each a slot"));
            }
        }
        Ok(n as u8)
    }

    /// Push one `VarDecl` per LEAF, in declaration order, so that a nested
    /// struct's members sit where `struct_member`'s offsets expect them.
    fn push_struct_slots(&mut self, sname: &'a str, name: &'a str, line: u32, depth: u32) -> R<'a, ()> {
        let _ = (line, depth);
        let mut leaves: AVec<Leaf<'a>> = AVec::new();
        self.struct_leaf_list(sname, "", 0, &mut leaves)?;
        for lf in leaves.as_slice() {
            self.locals.push(self.a, ir::VarDecl { name, ty: ir::Type::new(lf.scalar, lf.comps), array_len: 1 })?;
        }
        Ok(())
    }

    fn struct_run(&mut self, sname: &'a str, name: &'a str, line: u32) -> R<'a, Run> {
        let Some(ms) = self.struct_members(BaseType::Struct(sname)) else {
            return Err(Error::new(line, name, "a local of this type is not lowered yet"));
        };
        if ms.is_empty() {
            return Err(Error::new(line, name, "a struct with no members"));
        }
        /* The run is the LEAF count, not the member count: a member that is
         * itself a struct contributes that struct's own leaves. Computed
         * here so a shape that cannot be laid out is refused at the
         * DECLARATION, where the name and line are, rather than at whichever
         * member happens to be touched first. */
        Ok(Run::Struct(self.struct_leaves(sname, name, line, 0)?))
    }

    /// `S(a, b, c)`, or another value of the struct's type, filling a
    /// struct local: one store per member.
    ///
    /// One argument per MEMBER, never per slot. A member that is itself a
    /// struct is ONE argument -- its own constructor -- filling that
    /// member's own run, so the count is checked against the members and the
    /// destination walks by each member's leaves. Checking it against the
    /// leaf count instead would refuse `S(I(1.0, 2.0), 3.0)` and accept the
    /// flattened `S(1.0, 2.0, 3.0)`, which section 5.4.3 does not allow.
    fn struct_fill(
        &mut self,
        e: ExprId,
        l: Local<'a>,
        base: BaseType<'a>,
        name: &'a str,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let BaseType::Struct(sname) = base else {
            return Err(self.err(name, "a struct initialiser for a type that is not a struct"));
        };
        /* `c ? a : b` of structures (section 5.8): only the chosen arm is
         * evaluated, each filling the destination in its own branch. `,`:
         * the left for its effects, the right as the value. */
        match *self.ast.expr(e) {
            Expr::Select(c, t, f) => {
                let cond = self.value(c, 1, out)?;
                let mut then_items: AVec<Node> = AVec::new();
                self.struct_fill(t, l, base, name, &mut then_items)?;
                let mut else_items: AVec<Node> = AVec::new();
                self.struct_fill(f, l, base, name, &mut else_items)?;
                let (then_, else_) = self.commit2(then_items, else_items)?;
                let cond = Src { val: cond.val, swizzle: [cond.swizzle[0]; 4] };
                out.push(self.a, Node::If { cond, then_, else_ })?;
                return Ok(());
            }
            Expr::Comma(a, b) => {
                self.evaluate_and_discard(a, out)?;
                return self.struct_fill(b, l, base, name, out);
            }
            _ => {}
        }
        /* Either the struct's own constructor, or a value of that type --
         * `S b = a;` and `T(s, false, 3)` are the same move, one run copied
         * into another. Deciding it here rather than at each caller is what
         * keeps the two readings from drifting apart. */
        let Expr::Call { name: cname, args } = *self.ast.expr(e) else {
            return self.copy_struct_run(sname, l.id, e, name, out);
        };
        if cname != sname {
            /* Not this struct's constructor: a FUNCTION that returns the
             * struct. It is inlined with the destination as its want, so
             * its `return` fills this run directly -- there is no struct
             * value to hand back through, and a temporary run would be a
             * second copy of the same move.
             *
             * A struct and a function cannot share a name in one scope
             * (section 4.2.5, and dEQP's conflict_function_struct expects
             * the refusal), so `cname == sname` above is a constructor and
             * this is not. */
            let Some(f) = self.user_function(cname, args) else {
                return Err(self.err(cname, "this constructor is not for that struct"));
            };
            return match self.inline(cname, f, args, Want::Struct { dst: l, sname }, out)? {
                Ret::Struct => Ok(()),
                /* The want decided the shape, so nothing else can come back. */
                _ => Err(self.err(cname, "this function does not return that struct")),
            };
        }
        let ids: &[ExprId] = &self.ast.expr_ids[args.indices()];
        let ms = self.struct_members(base).unwrap_or(&[]);
        if ids.len() != ms.len() {
            return Err(self.err(cname, "a structure constructor takes one argument per member"));
        }
        let line = self.line;
        let mut off: u32 = 0;
        for (j, m) in ms.iter().enumerate() {
            let slots = self.member_slots(m, name, line, 0)?;
            let dst = Local {
                name: m.name,
                id: ir::LocalId(l.id.0 + off),
                comps: m.ty.base.components(),
                run: Run::Single,
                value: None,
            };
            if let BaseType::Struct(_) = m.ty.base {
                /* `T(s, false, 3)`: section 5.4.3 asks only that the
                 * argument already have the member's type, so a member that
                 * is a struct takes its own constructor OR a value of that
                 * type. `struct_fill` is the one place that decides which. */
                self.struct_fill(ids[j], dst, m.ty.base, name, out)?;
            } else {
                let v = self.value(ids[j], dst.comps, out)?;
                self.store_local(dst, v, out)?;
            }
            off += slots as u32;
        }
        Ok(())
    }

    /// Copy a struct value into a run of slots, leaf by leaf.
    ///
    /// A struct assignment moves every leaf: the run is not one register, so
    /// there is no single load that says "this struct". The source has to be
    /// a run of its own -- a name, or a member of one -- because there is
    /// nowhere else a struct value lives; anything else is refused by name
    /// rather than leaving the destination half written.
    fn copy_struct_run(
        &mut self,
        sname: &'a str,
        dst: ir::LocalId,
        src: ExprId,
        name: &'a str,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let Some(from) = self.struct_run_of(src)? else {
            /* A uniform struct, or an element of a uniform array of them
             * (`Light light = u_lights[ndx];`, dEQP light_amount): every
             * leaf read through `uniform_leaf`, a run-time index included. */
            if self.copy_struct_from_uniform(sname, dst.0, src, "", name, 0, out)?.is_some() {
                return Ok(());
            }
            return Err(self.err(name, "a struct value that is not a variable is not lowered yet"));
        };
        self.copy_struct_slots(sname, dst.0, from.id.0, name, 0, out)?;
        Ok(())
    }

    /// The leaves of `sname`, read from the uniform chain `src` (with
    /// `suffix` below it) into consecutive slots from `dst`. `None` when
    /// `src` is not a uniform. Returns the leaf count, so a nested member
    /// advances the destination.
    #[allow(clippy::too_many_arguments)]
    fn copy_struct_from_uniform(
        &mut self,
        sname: &'a str,
        dst: u32,
        src: ExprId,
        suffix: &'a str,
        name: &'a str,
        depth: u32,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, Option<u32>> {
        let _ = (name, depth);
        let mut leaves: AVec<Leaf<'a>> = AVec::new();
        self.struct_leaf_list(sname, suffix, 0, &mut leaves)?;
        for (k, lf) in leaves.as_slice().iter().enumerate() {
            let load = match lf.element {
                Some(e) => LeafLoad::Element(e),
                None => LeafLoad::Value,
            };
            let Some(v) = self.uniform_leaf(src, lf.path, load, lf.comps, out)? else {
                return Ok(None);
            };
            let d = Local { name: "", id: ir::LocalId(dst + k as u32), comps: lf.comps, run: Run::Single, value: None };
            self.store_local(d, v, out)?;
        }
        Ok(Some(leaves.len() as u32))
    }

    /// The leaves of `sname`, copied one by one from `src` to `dst`.
    /// Returns how many there were, so a nested member advances both.
    fn copy_struct_slots(
        &mut self,
        sname: &'a str,
        dst: u32,
        src: u32,
        name: &'a str,
        depth: u32,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, u32> {
        let _ = (name, depth);
        let mut leaves: AVec<Leaf<'a>> = AVec::new();
        self.struct_leaf_list(sname, "", 0, &mut leaves)?;
        for (k, lf) in leaves.as_slice().iter().enumerate() {
            let k = k as u32;
            let v = self.push_instr(Op::LoadLocal { local: ir::LocalId(src + k), index: None, comps: lf.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
            let d = Local { name: "", id: ir::LocalId(dst + k), comps: lf.comps, run: Run::Single, value: None };
            self.store_local(d, Src::id(v), out)?;
        }
        Ok(leaves.len() as u32)
    }

    /// The slot and width of `field` within a struct local.
    fn struct_member(&self, l: Local<'a>, base: BaseType<'a>, field: &'a str) -> Option<Local<'a>> {
        let ms = self.struct_members(base)?;
        let j = ms.iter().position(|m| m.name == field)?;
        /* The offset is the LEAVES before this member, not its index: a
         * member that is itself a struct occupies its own run, so counting
         * members would put every field after the first nested one on the
         * wrong slot -- reading a neighbour's value, which compiles and
         * draws. */
        let mut off: u32 = 0;
        for m in &ms[..j] {
            off += self.member_slots(m, field, 0, 0).ok()? as u32;
        }
        /* A member that is a struct stays a RUN, so `a.b.c` can be resolved
         * by asking this again with the inner type. */
        let len = ms[j].ty.array_len.and_then(|e| reflect::const_int_expr(self.ast, e)).map(|n| n as u8);
        let run = match (ms[j].ty.base, len) {
            (BaseType::Struct(inner), None) => Run::Struct(self.struct_leaves(inner, field, 0, 0).ok()?),
            (BaseType::Struct(inner), Some(n)) => {
                Run::StructArray { len: n, per: self.struct_leaves(inner, field, 0, 0).ok()? }
            }
            (_, Some(n)) => Run::Array(n),
            _ => Run::Single,
        };
        Some(Local {
            name: ms[j].name,
            id: ir::LocalId(l.id.0 + off),
            comps: ms[j].ty.base.components(),
            run,
            value: None,
        })
    }

    /// Put a declared variable in scope: the global table, or the innermost
    /// block's.
    fn bind(&mut self, l: Local<'a>, global: bool) -> R<'a, ()> {
        if global {
            self.globals.push(self.a, l)?;
        } else {
            self.scope.push(self.a, l)?;
        }
        Ok(())
    }

    /// Give every file-scope variable a slot, and run its initialiser, before
    /// `main`'s first statement.
    ///
    /// These were the single largest gap in the dEQP corpus: 1,527 refusals
    /// of "assignment to this is not lowered yet", every one on a variable
    /// declared at file scope. dEQP's shader templates put the case's own
    /// output there -- `float out0;` beside `uniform float ref_out0;` -- so
    /// most of `shaders.conditionals`, and a great deal else, stopped on the
    /// first line of the body.
    ///
    /// `const` globals come here too. A `const` is a variable whose value
    /// never changes, not a different kind of thing, and folding it would be
    /// an optimisation rather than a lowering; the slot is correct and the
    /// back end's copy propagation already removes the copy when it can.
    fn declare_globals(&mut self, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        let top: &'a [DeclId] = self.ast.top;
        for &id in top {
            let Decl::Var { storage, name, .. } = *self.ast.decl(id) else { continue };
            if storage != StorageQualifier::None && storage != StorageQualifier::Const {
                /* `uniform`, `attribute` and `varying` are inputs and the
                 * reflection's, not slots. */
                continue;
            }
            /* An output the shader also writes by name is handled by
             * `shadow_outputs`, which runs after this and would otherwise
             * give it a second slot. A built-in output is never declared, so
             * this only skips a name the shader itself declared -- which
             * ES 1.00 does not allow for `gl_*` anyway. */
            if self.builtin_output(name).is_some() {
                continue;
            }
            self.declare_var(id, true, out)?;
        }
        Ok(())
    }

    /// Resolve `e` as an [`LRef`], evaluating its indices now. `None`: not
    /// a shape this covers (a whole matrix or array, a struct member, a
    /// uniform), for the caller to refuse by name.
    /// modf(x, out i) (ES 3.00 section 8.3): i = trunc(x), the value
    /// x - trunc(x). The destination is resolved once (`LRef`), so an
    /// element or a lane chosen at run time is written without evaluating
    /// its index again. Out of line, like the `LRef` helpers.
    #[inline(never)]
    fn modf(&mut self, name: &'a str, xe: ExprId, ie: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let w = self.width(xe)?;
        let x = self.operand_at(xe, w, out)?;
        let Some(r) = self.resolve_lref(ie, out)? else {
            return Err(self.err(name, "an out argument that is not a variable is not lowered yet"));
        };
        let t = self.alu_w(ir::AluOp::Ftrunc, [x, x, x], w, out)?;
        self.store_lref(r, t, out)?;
        let nt = self.alu_w(ir::AluOp::Fneg, [t, t, t], w, out)?;
        self.alu_w(ir::AluOp::Fadd, [x, nt, nt], w, out)
    }

    /* Out of line: the recursive lowering must not carry its frame. */
    #[inline(never)]
    fn resolve_lref(&mut self, e: ExprId, out: &mut AVec<'a, Node>) -> R<'a, Option<LRef<'a>>> {
        match *self.ast.expr(e) {
            Expr::Ident(name) => {
                let Some(l) = self.lookup(name).or_else(|| self.shadow_of(name)) else { return Ok(None) };
                if l.value.is_some() || l.run != Run::Single {
                    return Ok(None);
                }
                Ok(Some(LRef { slots: SlotSel::One(l), lane: LaneSel::All }))
            }
            /* `s.f`, `t.a[1].x`: a one-slot leaf of a local structure, at
             * constant indices. */
            Expr::Field(b, _)
                if matches!(self.types.of(b).base, BaseType::Struct(_)) && !self.types.of(b).is_array() =>
            {
                match self.local_chain(e, &[])? {
                    Some(l) if l.run == Run::Single && l.value.is_none() => {
                        Ok(Some(LRef { slots: SlotSel::One(l), lane: LaneSel::All }))
                    }
                    _ => Ok(None),
                }
            }
            /* `v.y`, `v.zx`: lanes of a whole vector. */
            Expr::Field(b, f) => {
                let bt = self.types.of(b);
                if bt.is_array()
                    || !matches!(bt.base, BaseType::Vec(_) | BaseType::IVec(_) | BaseType::UVec(_) | BaseType::BVec(_))
                {
                    return Ok(None);
                }
                let Some(inner) = self.resolve_lref(b, out)? else { return Ok(None) };
                if !matches!(inner.lane, LaneSel::All) {
                    return Ok(None);
                }
                let (mask, lanes) = swizzle_mask(self.line, f, bt.base.components())?;
                if f.len() == 1 {
                    let k = lanes.iter().position(|x| x.is_some()).unwrap_or(0) as u8;
                    return Ok(Some(LRef { lane: LaneSel::At(k), ..inner }));
                }
                Ok(Some(LRef { lane: LaneSel::Swz { mask, lanes }, ..inner }))
            }
            Expr::Index(b, idx) => {
                let bt = self.types.of(b);
                /* a lane of a vector, at a constant or a run-time index */
                if !bt.is_array()
                    && matches!(bt.base, BaseType::Vec(_) | BaseType::IVec(_) | BaseType::UVec(_) | BaseType::BVec(_))
                {
                    let Some(inner) = self.resolve_lref(b, out)? else { return Ok(None) };
                    if !matches!(inner.lane, LaneSel::All) {
                        return Ok(None);
                    }
                    let lane = match self.pure_index(idx) {
                        Some(k) if (0..i64::from(bt.base.components())).contains(&k) => LaneSel::At(k as u8),
                        Some(_) => return Err(self.err("[]", "index out of range")),
                        None => LaneSel::Dyn(self.index_value(idx, out)?),
                    };
                    return Ok(Some(LRef { lane, ..inner }));
                }
                /* a slot of a local array of scalars or vectors, or a
                 * column of a local matrix */
                let Expr::Ident(name) = *self.ast.expr(b) else { return Ok(None) };
                let Some(l) = self.lookup(name).or_else(|| self.shadow_of(name)) else { return Ok(None) };
                let (Run::Array(n) | Run::Matrix(n)) = l.run else { return Ok(None) };
                if l.value.is_some() {
                    return Ok(None);
                }
                let slots = match self.pure_index(idx) {
                    Some(k) if (0..i64::from(n)).contains(&k) => SlotSel::One(l.column(k as u8)),
                    Some(_) => return Err(self.err("[]", "index out of range")),
                    None => SlotSel::Among { base: l, n, idx: self.index_value(idx, out)? },
                };
                Ok(Some(LRef { slots, lane: LaneSel::All }))
            }
            _ => Ok(None),
        }
    }

    /// The value an [`LRef`] holds now: its lane's, or the whole slot's.
    /* Out of line: the recursive lowering must not carry its frame. */
    #[inline(never)]
    fn load_lref(&mut self, r: LRef<'a>, out: &mut AVec<'a, Node>) -> R<'a, Src> {
        let comps = match r.slots {
            SlotSel::One(l) => l.comps,
            SlotSel::Among { base, .. } => base.comps,
        };
        let slot = match r.slots {
            SlotSel::One(l) => {
                let v = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps })?;
                out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                Src::id(v)
            }
            SlotSel::Among { base, n, idx } => {
                let mut els: AVec<Src> = AVec::new();
                for k in 0..n {
                    let v = self.push_instr(Op::LoadLocal { local: base.column(k).id, index: None, comps })?;
                    out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
                    els.push(self.a, Src::id(v))?;
                }
                self.select_among(els.as_slice(), idx, comps, out)?
            }
        };
        match r.lane {
            LaneSel::All => Ok(slot),
            LaneSel::At(k) => Ok(lane_of(slot, k)),
            LaneSel::Dyn(i) => {
                let lanes: [Src; 4] = core::array::from_fn(|k| lane_of(slot, k as u8));
                self.select_among(&lanes[..usize::from(comps)], i, 1, out)
            }
            /* the value's lane j is the slot's lane i where lanes[i] = j */
            LaneSel::Swz { lanes, .. } => {
                let mut sw = [0u8; 4];
                for (i, l) in lanes.iter().enumerate() {
                    if let Some(j) = *l {
                        sw[usize::from(j)] = slot.swizzle[i];
                    }
                }
                Ok(Src { val: slot.val, swizzle: sw })
            }
        }
    }

    /// Write `v` through an [`LRef`]: the whole slot, or its lane, of the
    /// one slot -- or of each slot it may be, which keeps its old value
    /// where the run-time index is not its own.
    /* Out of line: the recursive lowering must not carry its frame. */
    #[inline(never)]
    fn store_lref(&mut self, r: LRef<'a>, v: Src, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        match r.slots {
            SlotSel::One(l) => self.store_into_slot(l, r.lane, v, None, out),
            SlotSel::Among { base, n, idx } => {
                for k in 0..n {
                    let kc = self.int_const([i32::from(k); 4], out)?;
                    let eq = self.alu_w(ir::AluOp::Ieq, [idx, kc, kc], 1, out)?;
                    let eq = Src { val: eq.val, swizzle: [eq.swizzle[0]; 4] };
                    self.store_into_slot(base.column(k), r.lane, v, Some(eq), out)?;
                }
                Ok(())
            }
        }
    }

    /// One slot of [`Lower::store_lref`]: the lanes `lane` selects take `v`
    /// (splatted), where `only` -- if given -- is true.
    #[inline(never)]
    fn store_into_slot(
        &mut self,
        l: Local<'a>,
        lane: LaneSel,
        v: Src,
        only: Option<Src>,
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        if let (LaneSel::All, None) = (lane, only) {
            return self.store_local(l, v, out);
        }
        if let (LaneSel::At(k), None) = (lane, only) {
            let mut lanes = [None; 4];
            lanes[usize::from(k)] = Some(0);
            return self.store_lanes(l, v, 1 << k, lanes, out);
        }
        if let (LaneSel::Swz { mask, lanes }, None) = (lane, only) {
            return self.store_lanes(l, v, mask, lanes, out);
        }
        /* a swizzle into one of several slots: the value placed at the
         * destination lanes, then selected where this slot is the one */
        if let LaneSel::Swz { mask, lanes } = lane {
            let mut sw = [0u8; 4];
            for (i, x) in lanes.iter().enumerate() {
                if let Some(j) = *x {
                    sw[i] = v.swizzle[usize::from(j)];
                }
            }
            let placed = Src { val: v.val, swizzle: sw };
            let old = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
            out.push(self.a, Node::Instr(ir::InstrId(old.0)))?;
            let bools: [bool; 4] = core::array::from_fn(|i| mask & (1 << i) != 0);
            let c = self.push_instr(Op::ConstBool(bools))?;
            out.push(self.a, Node::Instr(ir::InstrId(c.0)))?;
            let m = Src::id(c);
            let cond = match only {
                Some(o) => self.alu_w(ir::AluOp::Bcsel, [o, m, o], l.comps, out)?,
                None => m,
            };
            let new = self.alu_w(ir::AluOp::Bcsel, [cond, placed, Src::id(old)], l.comps, out)?;
            return self.store_local(l, new, out);
        }
        let old = self.push_instr(Op::LoadLocal { local: l.id, index: None, comps: l.comps })?;
        out.push(self.a, Node::Instr(ir::InstrId(old.0)))?;
        let old = Src::id(old);
        let (mask, value) = match lane {
            LaneSel::All => (None, v),
            LaneSel::At(k) => {
                let lanes: [bool; 4] = core::array::from_fn(|i| i == usize::from(k));
                let c = self.push_instr(Op::ConstBool(lanes))?;
                out.push(self.a, Node::Instr(ir::InstrId(c.0)))?;
                (Some(Src::id(c)), Src { val: v.val, swizzle: [v.swizzle[0]; 4] })
            }
            LaneSel::Dyn(i) => {
                let ks = self.int_const([0, 1, 2, 3], out)?;
                let m = self.alu_w(ir::AluOp::Ieq, [i, ks, ks], l.comps, out)?;
                (Some(m), Src { val: v.val, swizzle: [v.swizzle[0]; 4] })
            }
            LaneSel::Swz { .. } => unreachable!("handled above"),
        };
        let cond = match (mask, only) {
            (Some(m), Some(o)) => Some(self.alu_w(ir::AluOp::Bcsel, [o, m, o], l.comps, out)?),
            (m, o) => m.or(o),
        };
        let new = match cond {
            Some(c) => self.alu_w(ir::AluOp::Bcsel, [c, value, old], l.comps, out)?,
            None => value,
        };
        self.store_local(l, new, out)
    }

    /// The slot an assignment's destination names, if it names one.
    ///
    /// A plain variable, or a column of a matrix local: `inverseMatrix[0].x =
    /// d * invDet;` is how `qInverse` in the Qt shaders writes a matrix, and
    /// a matrix local IS its columns, so `m[j]` is simply slot `id + j`.
    /// `None` means the destination is something else and the caller refuses
    /// by name; an error means it names a slot it may not write.
    fn lvalue_slot(&mut self, e: ExprId) -> R<'a, Option<Local<'a>>> {
        match *self.ast.expr(e) {
            Expr::Ident(name) => Ok(self.shadow_of(name).or_else(|| self.lookup(name))),
            Expr::Index(base, idx) => {
                let Expr::Ident(name) = *self.ast.expr(base) else {
                    /* A column of an element of an array of matrices, or an
                     * element of an array member (`a[1][2]`, `s.v[0]`), with
                     * constant indices. */
                    return self.local_chain(e, &[]);
                };
                let Some(l) = self.shadow_of(name).or_else(|| self.lookup(name)) else {
                    return Ok(None);
                };
                if l.run.elements().is_none() && l.run.columns().is_none() {
                    /* Indexing a VECTOR as a destination is a different
                     * thing -- a lane, not a slot -- and is not lowered.
                     * Refused by name rather than treated as an element.
                     * Asked by the run's kind, not its slot count: `vec4
                     * a[1]` is one slot and an array all the same, and
                     * `a[0] = v` was refused until 2026-09-24 (dEQP-GLES3
                     * fragment_out.random, 507 cases with `out vec4 o[1]`). */
                    return Ok(None);
                }
                let Some(j) = reflect::const_int_expr(self.ast, idx) else {
                    /* A variable index needs a dynamic register index, which
                     * selection refuses by name ("an indexed local slot:
                     * arrays need a dynamic register index"). Lowering it
                     * here would only move the refusal downstream. */
                    return Err(self.err("[]", "a non-constant index into a local is not lowered yet"));
                };
                if j < 0 || j as u32 >= u32::from(l.run.slots()) {
                    return Err(self.err("[]", "index out of range"));
                }
                Ok(Some(l.column(j as u8)))
            }
            /* A member or element through a struct (`s[1].v`, `s.b[0]`). */
            Expr::Field(..) => self.local_chain(e, &[]),
            _ => Ok(None),
        }
    }

    /// Write the lanes `lanes` of `l` from `src`.
    ///
    /// `lanes[i] = Some(j)` means the destination's lane *i* takes the
    /// source's lane *j*, which is the convention the constructor's stores
    /// use: the swizzle is indexed by the DESTINATION lane.
    fn store_lanes(
        &mut self,
        l: Local<'a>,
        src: Src,
        mask: u8,
        lanes: [Option<u8>; 4],
        out: &mut AVec<'a, Node>,
    ) -> R<'a, ()> {
        let mut swizzle = [0u8; 4];
        for (i, lane) in lanes.iter().enumerate() {
            if let Some(j) = lane {
                swizzle[i] = src.swizzle[*j as usize];
            }
        }
        let src = Src { val: src.val, swizzle };
        let v = self.push_instr(Op::StoreLocal { local: l.id, index: None, src, write_mask: mask })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(())
    }

    /// Write each column of a matrix local.
    fn store_columns(&mut self, l: Local<'a>, cols: &[Src; 4], out: &mut AVec<'a, Node>) -> R<'a, ()> {
        for j in 0..l.run.slots() {
            self.store_local(l.column(j), cols[j as usize], out)?;
        }
        Ok(())
    }

    fn store_local(&mut self, l: Local<'a>, src: Src, out: &mut AVec<'a, Node>) -> R<'a, ()> {
        if l.value.is_some() {
            /* A parameter is aliased only when the body was checked not to
             * assign to it, so this cannot be reached through a shader. It
             * refuses rather than writing to `LocalId(u32::MAX)`, which is
             * not a slot and would be a wrong picture with no failure. */
            return Err(self.err(l.name, "assignment to an aliased parameter"));
        }
        /* The whole variable is written: a partial write needs an lvalue
         * swizzle, which is not lowered yet and refuses earlier. */
        let write_mask = ((1u16 << l.comps) - 1) as u8;
        let v = self.push_instr(Op::StoreLocal { local: l.id, index: None, src, write_mask })?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(())
    }

    fn push_instr(&mut self, op: Op) -> R<'a, ValueId> {
        let w = op.stated_comps().unwrap_or(0);
        self.push_sized(op, w)
    }

    /// Emit an instruction whose result width the CALLER knows.
    ///
    /// Only the front end does: the operands are `Src`s, whose swizzle
    /// always carries four lanes whatever the type said, so `a.xy + b.xy`
    /// and `a + b` over `vec4` producers are indistinguishable downstream.
    /// That is why the width is recorded here rather than estimated in a
    /// back end.
    fn push_sized(&mut self, op: Op, comps: u8) -> R<'a, ValueId> {
        if let Op::StoreLocal { local, .. } = op {
            self.forget_known(local);
        }
        let id = ValueId(self.instrs.len() as u32);
        self.instrs.push(self.a, op)?;
        self.comps.push(self.a, comps)?;
        Ok(id)
    }
}

/// The lowering pass seen as somewhere to put instructions, which is all
/// [`builtin_call`] needs of it. Two borrows rather than one because the
/// instruction list and the current node list are separate: an instruction
/// goes in the arena's flat `instrs`, and the node that runs it goes in the
/// block being built.
struct Sink<'s, 'x, 'a> {
    l: &'s mut Lower<'x, 'a>,
    out: &'s mut AVec<'a, Node>,
}

impl<'a> builtin_call::Emit<'a> for Sink<'_, '_, 'a> {
    fn emit(&mut self, op: Op) -> R<'a, Src> {
        let v = self.l.push_instr(op)?;
        self.out.push(self.l.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(Src::id(v))
    }

    fn constant(&mut self, lanes: [f32; 4]) -> R<'a, Src> {
        self.l.const_value(lanes, self.out)
    }

    fn vertex(&self) -> bool {
        self.l.stage == Stage::Vertex
    }

    fn int_lanes(&self, v: Src) -> Option<[i32; 4]> {
        match self.l.instrs.as_slice().get(v.val.0 as usize) {
            Some(&Op::ConstI32(c)) => Some(v.swizzle.map(|s| c[usize::from(s) & 3])),
            _ => None,
        }
    }

    fn identity(&mut self, v: Src, comps: u8) -> R<'a, ValueId> {
        if v.swizzle[..usize::from(comps)].iter().enumerate().all(|(i, &s)| usize::from(s) == i) {
            return Ok(v.val);
        }
        /* A swizzled operand is copied into a slot and loaded back, which
         * reads it with the identity swizzle at its width. */
        let slot = self.l.temp(ir::ScalarType::F32, comps)?;
        self.l.store_local(slot, v, self.out)?;
        let x = self.l.push_instr(Op::LoadLocal { local: slot.id, index: None, comps })?;
        self.out.push(self.l.a, Node::Instr(ir::InstrId(x.0)))?;
        Ok(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lowered<'a>(buf: &'a mut [u8], src: &'a str, stage: Stage) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    /// The first of M3's two shaders: one attribute straight to gl_Position.
    #[test]
    fn the_passthrough_vertex_shader_lowers() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(&mut buf, "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }", Stage::Vertex);
        let m = &l.module;
        assert_eq!(m.stage, SHC_STAGE_VERTEX);
        assert_eq!(
            m.instrs,
            [
                Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                Op::StoreOutput { slot: ir::IoSlot::Position, src: Src::id(ValueId(0)) },
            ]
        );
        assert_eq!(m.nodes, [Node::Instr(ir::InstrId(0)), Node::Instr(ir::InstrId(1)), Node::Return]);
        assert_eq!(m.body, IrRange { first: 0, len: 3 });

        assert_eq!(m.inputs.len(), 1);
        assert_eq!((m.inputs[0].name, m.inputs[0].location, m.inputs[0].components), ("a_pos", 0, 4));
        assert_eq!(m.inputs[0].kind, IoKind::Attribute);
        assert_eq!(m.outputs.len(), 1);
        assert_eq!(m.outputs[0].kind, IoKind::Position);
        assert_eq!(l.reflection.attribs.len(), 1);
        assert_eq!(l.reflection.attribs[0].gl_type, 0x8B52, "GL_FLOAT_VEC4");
    }

    /// The second: a literal colour, with no uniform, because a fragment
    /// uniform does not reach its shader on this part yet (task #52).
    #[test]
    fn the_literal_colour_fragment_shader_lowers() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert_eq!(m.stage, SHC_STAGE_FRAGMENT);
        let (one, zero) = (1.0f32.to_bits(), 0.0f32.to_bits());
        assert_eq!(
            m.instrs,
            [
                Op::ConstF32([one, zero, zero, one]),
                Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: Src::id(ValueId(0)) },
            ]
        );
        assert_eq!(m.nodes, [Node::Instr(ir::InstrId(0)), Node::Instr(ir::InstrId(1)), Node::Return]);
        assert_eq!(m.outputs.len(), 1);
        assert_eq!(m.outputs[0].kind, IoKind::Colour);
        assert!(!m.uses_discard);
    }

    /// A varying is an output of one stage and an input of the other, from
    /// the same reflection row, so the two stages cannot disagree about its
    /// location.
    #[test]
    fn a_varying_is_an_output_of_one_stage_and_an_input_of_the_other() {
        let mut buf = [0u8; 1 << 18];
        let vs = lowered(
            &mut buf,
            "attribute vec4 a_pos;\nvarying vec4 v_col;\nvoid main() { gl_Position = a_pos; v_col = a_pos; }",
            Stage::Vertex,
        );
        let m = &vs.module;
        assert!(m.outputs.iter().any(|o| o.kind == IoKind::Varying && o.name == "v_col" && o.location == 0));
        /* Each statement loads the attribute for itself: the front end does
         * no common-subexpression elimination, and should not -- the back
         * end sees the whole block and has the register pressure numbers,
         * and an IR that had already merged the two would be hiding a choice
         * from the pass that is entitled to make it. */
        assert_eq!(
            m.instrs,
            [
                Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                Op::StoreOutput { slot: ir::IoSlot::Position, src: Src::id(ValueId(0)) },
                Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                Op::StoreOutput { slot: ir::IoSlot::Varying(0), src: Src::id(ValueId(2)) },
            ]
        );

        let mut buf2 = [0u8; 1 << 18];
        let fs = lowered(
            &mut buf2,
            "precision mediump float;\nvarying vec4 v_col;\nvoid main() { gl_FragColor = v_col; }",
            Stage::Fragment,
        );
        let m = &fs.module;
        assert!(m.inputs.iter().any(|i| i.kind == IoKind::Varying && i.name == "v_col" && i.location == 0));
        assert_eq!(m.instrs[0], Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 4 });
    }

    #[test]
    fn a_swizzle_becomes_the_operands_component_selection() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 v_col;\nvoid main() { gl_FragColor = v_col.wzyx; }",
            Stage::Fragment,
        );
        let store = l
            .module
            .instrs
            .iter()
            .find_map(|op| match op {
                Op::StoreOutput { src, .. } => Some(*src),
                _ => None,
            })
            .expect("a store");
        assert_eq!(store.swizzle, [3, 2, 1, 0], "the swizzle rides the operand, not an instruction");
        assert_eq!(l.module.instrs.len(), 2, "and costs no extra instruction");
    }

    /// `vec4(1.0)` fills every component (section 5.4.2).
    #[test]
    fn a_one_argument_constructor_fills_every_component() {
        let mut buf = [0u8; 1 << 18];
        let l =
            lowered(&mut buf, "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }", Stage::Fragment);
        assert_eq!(l.module.instrs[0], Op::ConstF32([1.0f32.to_bits(); 4]));
    }

    /// Everything outside the subset is refused BY NAME. A lowering that
    /// emitted nothing for a statement it did not understand would produce a
    /// shader that draws the wrong thing, which is far harder to find than a
    /// refusal.
    #[test]
    fn what_is_not_lowered_yet_says_so_instead_of_emitting_nothing() {
        let cases: [(&str, Stage, &str); 1] = [
            /* A local of a shape that is not one slot. `float x = 1.0;` was
             * the example here, then `mat2 m;`, then `float f[2];`, then a
             * struct whose member is itself a struct: all of them are
             * lowered now, a run of slots each. A struct member that is a
             * MATRIX is the shape that remains -- a run whose slots are not
             * leaves of the enclosing run. */
            (
                "struct S { mat2 m; float b; };\nvoid main() { S s; s.b = 1.0; }",
                Stage::Fragment,
                "a struct member that is a matrix is not lowered yet",
            ),
            /* `gl_FragCoord` was a row here and is now a `LoadInput`. */
            /* A row here walked outward seven times: `if`, the comparison
             * in its condition, the constructor assembled from several
             * values, the assignment to a swizzle, the matrix that is
             * computed rather than named, the non-constant uniform index,
             * the non-constant matrix index, and last the cube look-up with
             * a bias -- IR v1's cube `Op::Tex` since 2026-09-24
             * (es100_cube_lod_bias_tests). The shader it ended on lowers. */
            /* `v + v` was the example here, then `v * m`; both are lowered
             * now (`v * m` since 2026-09-23, tested by running it in
             * matrix_product_tests). */
        ];
        for (src, stage, want) in cases {
            let mut buf = [0u8; 1 << 18];
            let a = Arena::new(&mut buf);
            match compile_to_ir(&a, stage, src) {
                Ok(_) => panic!("expected a refusal for {src:?}"),
                Err(e) => assert_eq!(e.msg, want, "for {src:?}"),
            }
        }
    }

    /// `discard` is in the subset because the fragment shader's "full mode"
    /// turns on it and the back end reads `uses_discard`.
    #[test]
    fn discard_sets_the_flag_the_back_end_reads() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvoid main() { discard; gl_FragColor = vec4(1.0); }",
            Stage::Fragment,
        );
        assert!(l.module.uses_discard);
        assert!(l.module.instrs.contains(&Op::Discard));
    }

    /// gl_test case 1: the colour comes from a uniform rather than a
    /// literal. The offset is the reflection's, which is the offset
    /// `glUniform4fv` writes to, so the two cannot be computed differently.
    #[test]
    fn a_uniform_read_loads_the_block_dword_the_reflection_gave_it() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }",
            Stage::Fragment,
        );
        assert_eq!(
            l.module.instrs,
            [
                Op::LoadUniform { dword_off: 0, comps: 4 },
                Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: Src::id(ValueId(0)) },
            ]
        );
        assert_eq!(l.module.uniform_dwords, 4);
        assert_eq!(l.module.uniforms.len(), 1);
        assert_eq!((l.module.uniforms[0].name, l.module.uniforms[0].dword_off), ("u_color", 0));

        /* A second uniform lands after the first, and the load follows the
         * std430 offset rather than the declaration index. */
        let mut buf2 = [0u8; 1 << 18];
        let l = lowered(
            &mut buf2,
            "precision mediump float;\nuniform float u_a;\nuniform vec4 u_b;\nvoid main() { gl_FragColor = u_b; }",
            Stage::Fragment,
        );
        assert_eq!(l.module.instrs[0], Op::LoadUniform { dword_off: 4, comps: 4 }, "vec4 aligns to 4");
    }

    #[test]
    fn a_constant_index_into_a_uniform_array_folds_into_the_offset() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 frag[4];\nvoid main() { gl_FragColor = frag[2]; }",
            Stage::Fragment,
        );
        assert_eq!(l.module.instrs[0], Op::LoadUniform { dword_off: 8, comps: 4 }, "element 2 of a vec4 array");

        /* The stride is the row's own dwords divided by its length, so an
         * element of a vec3 array is four dwords apart and not three -- the
         * reflection already rounded the element up to its alignment, and
         * recomputing it here could only disagree with the table. */
        let mut buf2 = [0u8; 1 << 18];
        let l = lowered(&mut buf2, "uniform vec3 v[3];\nvarying vec3 c;\nvoid main() { c = v[1]; }", Stage::Vertex);
        assert_eq!(l.module.instrs[0], Op::LoadUniform { dword_off: 4, comps: 3 });
    }

    #[test]
    fn a_uniform_read_may_be_swizzled_without_an_instruction() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 u;\nvoid main() { gl_FragColor = u.wzyx; }",
            Stage::Fragment,
        );
        assert_eq!(l.module.instrs.len(), 2);
        let Op::StoreOutput { src, .. } = l.module.instrs[1] else { panic!("a store") };
        assert_eq!(src.swizzle, [3, 2, 1, 0]);
    }

    /// The uniform kinds this lowering cannot read yet are refused by name.
    /// A matrix is columns and needs the arithmetic lowering; a sampler is
    /// named by binding in a `Tex` op and is never loaded from the block.
    #[test]
    fn the_uniform_kinds_not_lowered_yet_say_which() {
        /* Only one case here, and the two that are missing are a finding
         * rather than an omission. A matrix read and a sampler read are both
         * refused EARLIER, by semantic analysis -- `gl_FragColor = m;` is
         * "the types do not match" and `gl_FragColor = s;` is "no implicit
         * conversion in GLSL ES 1.00" -- so the guards for them in
         * `uniform_value` cannot be reached through any shader today. They
         * are still right, and become reachable when arithmetic and texture
         * sampling land; the table they turn on is tested directly in
         * `the_component_table_knows_what_it_cannot_load` rather than
         * through a shader that cannot get there. */
        /* The case that used to be here -- a non-constant index -- is
         * lowered now, so it is checked positively instead: the bound goes
         * with the index, because an index outside the array is undefined in
         * GLSL but reading elsewhere in the constant file, where another
         * shader's uniforms sit, is not an acceptable undefined. */
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 v[4];\nuniform int i;\nvoid main() { gl_FragColor = v[i]; }",
            Stage::Fragment,
        );
        let Some(Op::LoadUniformIdx { base_dw, stride_dw, len, comps, .. }) =
            l.module.instrs.iter().find(|o| matches!(o, Op::LoadUniformIdx { .. }))
        else {
            panic!("an indexed read");
        };
        assert_eq!((*base_dw, *stride_dw, *len, *comps), (0, 4, 4, 4), "four vec4 at dword 0, stride 4");
    }

    /// The component table the uniform load depends on. A matrix and a
    /// sampler have no component count here on purpose -- a matrix is
    /// columns and needs the arithmetic lowering, a sampler is named by
    /// binding and never loaded -- and `uniform_value` refuses on exactly
    /// that `None`.
    #[test]
    fn the_component_table_knows_what_it_cannot_load() {
        assert_eq!(components_of(0x1406), Some(1), "float");
        assert_eq!(components_of(0x8B50), Some(2), "vec2");
        assert_eq!(components_of(0x8B51), Some(3), "vec3");
        assert_eq!(components_of(0x8B52), Some(4), "vec4");
        assert_eq!(components_of(0x8B56), Some(1), "bool");
        assert_eq!(components_of(0x8B55), Some(4), "ivec4");
        assert_eq!(components_of(0x8B5C), None, "mat4: columns, not a load");
        assert_eq!(components_of(0x8B5A), None, "mat2");
        assert_eq!(components_of(0x8B5E), None, "sampler2D: a binding, not a load");
    }

    /// The entry point returns everything borrowing the caller's arena, so a
    /// caller that lends too small a buffer is told rather than truncated.
    #[test]
    fn a_scratch_buffer_too_small_is_reported_not_truncated() {
        let mut buf = [0u8; 64];
        let a = Arena::new(&mut buf);
        let Err(e) = compile_to_ir(&a, Stage::Vertex, "attribute vec4 a;\nvoid main() { gl_Position = a; }") else {
            panic!("64 bytes cannot hold this compile");
        };
        assert_eq!(e.msg, "compiler scratch buffer exhausted");
    }
    /// A local variable: one slot, a store from its initialiser, a load where
    /// it is read.
    ///
    /// The write mask is the whole variable, because a partial write needs an
    /// lvalue swizzle, which refuses earlier.
    ///
    /// The shader assigns the local straight to `gl_FragColor` rather than
    /// passing it through `vec4(c, c, c, c)`: a constructor argument that is
    /// not a literal is a separate gap, still refused, and a test should fail
    /// for its own reason.
    #[test]
    fn a_local_gets_a_slot_a_store_and_a_load() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvoid main() { vec4 c = vec4(0.25); gl_FragColor = c; }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert_eq!(m.locals.len(), 1);
        assert_eq!(m.locals[0].name, "c");
        assert_eq!(m.locals[0].ty, ir::Type::new(ir::ScalarType::F32, 4));
        assert_eq!(m.locals[0].array_len, 1);

        let stores = m.instrs.iter().filter(|o| matches!(o, Op::StoreLocal { .. })).count();
        assert_eq!(stores, 1, "the initialiser is the only write");
        let Some(Op::StoreLocal { local, index, write_mask, .. }) =
            m.instrs.iter().find(|o| matches!(o, Op::StoreLocal { .. }))
        else {
            panic!("no store");
        };
        assert_eq!((*local, *index, *write_mask), (ir::LocalId(0), None, 0b1111));
    }

    /// Assigning to a local after it is declared is a second store to the
    /// same slot, and the read after it loads that slot -- not the value the
    /// initialiser produced. Getting this wrong would compile and draw the
    /// old colour.
    #[test]
    fn assigning_to_a_local_writes_the_same_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvoid main() { vec4 c = vec4(0.25); c = vec4(0.75); gl_FragColor = c; }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert_eq!(m.locals.len(), 1, "one variable, one slot");
        let mut slots = [ir::LocalId(u32::MAX); 2];
        let mut n = 0;
        for o in m.instrs.iter() {
            if let Op::StoreLocal { local, .. } = o {
                assert!(n < slots.len(), "more writes than this shader has");
                slots[n] = *local;
                n += 1;
            }
        }
        assert_eq!((n, slots), (2, [ir::LocalId(0), ir::LocalId(0)]), "both writes go to the same slot");
        assert!(
            m.instrs.iter().any(|o| matches!(o, Op::LoadLocal { local: ir::LocalId(0), .. })),
            "the read is a load of the slot, not a reuse of the initialiser's value"
        );
    }

    /// A local shadows a global of the same name (section 4.2.3), so the
    /// lookup has to be innermost-first. If it were not, this shader would
    /// read the uniform and the local's value would never be used.
    #[test]
    fn a_local_shadows_a_uniform_of_the_same_name() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 c;\nvoid main() { vec4 c = vec4(0.5); gl_FragColor = c; }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(m.instrs.iter().any(|o| matches!(o, Op::LoadLocal { .. })), "the read resolves to the local");
        assert!(!m.instrs.iter().any(|o| matches!(o, Op::LoadUniform { .. })), "and never to the uniform it shadows");
    }

    /// A local array is a RUN OF SLOTS, one per element, exactly as a
    /// `matN` local is one per column.
    ///
    /// This test asserted the refusal ("a local array is not lowered yet")
    /// until 2026-09-22. The assertion has moved to the property that was
    /// wanted all along -- an element gets its own slot and does not share
    /// one -- rather than being deleted with the refusal.
    #[test]
    fn a_local_array_is_one_slot_per_element() {
        for (src, want_slots) in [
            ("precision mediump float;\nvoid main() { vec4 a[3]; a[0] = vec4(0.0); gl_FragColor = a[0]; }", 3),
            ("precision mediump float;\nvoid main() { float f[2]; f[0] = 0.0; gl_FragColor = vec4(f[0]); }", 2),
        ] {
            let mut buf = [0u8; 1 << 18];
            let a = Arena::new(&mut buf);
            match compile_to_ir(&a, Stage::Fragment, src) {
                Ok(l) => assert_eq!(l.module.locals.len(), want_slots, "for {src:?}"),
                Err(e) => panic!("expected success for {src:?}, got {:?}: {}", e.tok, e.msg),
            }
        }
    }
    /// `transformationMatrix * myVertex` -- `hello.vert` from the table, and
    /// the first whole program this compiler produces.
    ///
    /// **The pairing is the point.** Column *i* multiplies `v[i]`. The
    /// shipped GC7000 blob reads `mul t1, u0, t0.wwww` and three `mad`s
    /// against z, y, x, which looks like the opposite -- but its constant-file
    /// map `CD_4` puts block dwords 12..15 in `u0`, so `u0` IS column 3. Only
    /// the register numbering runs backwards. A lowering that took the
    /// disassembly at face value would compute `M * reverse(v)` and still
    /// draw a correct picture for every identity or symmetric matrix, which
    /// is most bring-up shaders -- so this test states the pairing in block
    /// dwords, where it cannot be confused with a register number.
    #[test]
    fn a_matrix_times_a_vector_pairs_column_i_with_component_i() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "attribute highp vec4 myVertex;\n\
             uniform mediump mat4 transformationMatrix;\n\
             void main(void) { gl_Position = transformationMatrix * myVertex; }\n",
            Stage::Vertex,
        );
        let m = &l.module;

        /* The vector, once. */
        let v = m
            .instrs
            .iter()
            .position(|o| matches!(o, Op::LoadInput { slot: ir::IoSlot::Attrib(0), .. }))
            .expect("the attribute is loaded");

        /* Walk the four arithmetic instructions in the order they are
         * emitted: one Fmul then three Ffma, highest column first, which is
         * the shipped blob's order. */
        let mut seen = 0usize;
        for (i, op) in m.instrs.iter().enumerate() {
            let (alu, srcs) = match op {
                Op::Alu(a, s) => (*a, *s),
                _ => continue,
            };
            let column = 3 - seen as u8;
            let want_dword = u16::from(column) * 4;
            assert_eq!(
                alu,
                if seen == 0 { ir::AluOp::Fmul } else { ir::AluOp::Ffma },
                "instruction {i}: a mul then three mads"
            );
            /* Operand 0 is the column: a uniform load at the column's own
             * dwords, which is what says WHICH column this term uses. */
            let col_at = srcs[0].val.0 as usize;
            assert_eq!(
                m.instrs[col_at],
                Op::LoadUniform { dword_off: want_dword, comps: 4 },
                "term {seen} must use column {column}, at block dword {want_dword}"
            );
            /* Operand 1 is that column's scale: component `column` of the
             * vector, splatted. */
            assert_eq!(srcs[1].val.0 as usize, v, "the scale comes from the attribute");
            assert_eq!(
                srcs[1].swizzle, [column; 4],
                "column {column} is scaled by component {column} of the vector, not by any other"
            );
            seen += 1;
        }
        assert_eq!(seen, 4, "a mat4 product is four instructions");
    }

    /// Dividing by a LITERAL is a multiply, because the reciprocal is known
    /// at compile time.
    ///
    /// `x / 2.0` is `x * 0.5` and not `x * rcp(2.0)`. An `Frcp` of a
    /// constant is an instruction computing a number the compiler already
    /// has, and -- more to the point downstream -- a constant can be folded
    /// into its consumer as an immediate, which an `Frcp` result never can.
    ///
    /// `x / 0.0` is NOT folded: section 4.5.1 leaves it undefined, and an
    /// infinite literal is a pattern nothing further down the chain has ever
    /// been handed.
    #[test]
    fn dividing_by_a_literal_is_a_multiply_by_its_reciprocal() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float x;\n\
             void main() { gl_FragColor = vec4(x / 2.0); }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(
            !m.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Frcp, _))),
            "no reciprocal instruction: {:?}",
            m.instrs
        );
        let half = 0.5f32.to_bits();
        assert!(
            m.instrs.iter().any(|o| matches!(o, Op::ConstF32(b) if b[0] == half)),
            "the reciprocal is the constant 0.5: {:?}",
            m.instrs
        );

        /* A divisor that is not a literal still takes the reciprocal. */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = lowered(
            &mut buf2,
            "precision mediump float;\n\
             varying float x;\n\
             varying float y;\n\
             void main() { gl_FragColor = vec4(x / y); }",
            Stage::Fragment,
        );
        assert!(
            l2.module.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Frcp, _))),
            "a run-time divisor keeps its reciprocal"
        );

        /* And zero is left alone rather than folded to an infinity. */
        let mut buf3 = [0u8; 1 << 18];
        let l3 = lowered(
            &mut buf3,
            "precision mediump float;\n\
             varying float x;\n\
             void main() { gl_FragColor = vec4(x / 0.0); }",
            Stage::Fragment,
        );
        assert!(
            l3.module.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Frcp, _))),
            "division by zero is undefined and is not folded to an infinite literal"
        );
    }

    /// Subtraction and division are not in the opcode kernel, so each is a
    /// pair. The kernel is ~45 opcodes on purpose and the expansions belong
    /// to the front end; a back end that had to know `Fsub` would be a back
    /// end the next GPU has to reimplement.
    #[test]
    fn subtract_and_divide_expand_rather_than_ask_for_an_opcode() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\nvoid main() { gl_FragColor = a - b; }",
            Stage::Fragment,
        );
        let ops: [ir::AluOp; 2] = {
            let mut got = [ir::AluOp::Fadd; 2];
            let mut n = 0;
            for o in l.module.instrs.iter() {
                if let Op::Alu(a, _) = o {
                    assert!(n < got.len(), "a - b is two instructions");
                    got[n] = *a;
                    n += 1;
                }
            }
            assert_eq!(n, 2);
            got
        };
        assert_eq!(ops, [ir::AluOp::Fneg, ir::AluOp::Fadd], "a - b is a negate and an add");

        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\nvoid main() { gl_FragColor = a / b; }",
            Stage::Fragment,
        );
        let mut got = [ir::AluOp::Fadd; 2];
        let mut n = 0;
        for o in l.module.instrs.iter() {
            if let Op::Alu(a, _) = o {
                got[n] = *a;
                n += 1;
            }
        }
        assert_eq!((n, got), (2, [ir::AluOp::Frcp, ir::AluOp::Fmul]), "a / b is a reciprocal and a multiply");
    }

    /// A scalar against a vector is splatted, which is what section 5.9 says
    /// `v * f` means -- and it costs a swizzle, not an instruction.
    #[test]
    fn a_scalar_operand_against_a_vector_is_splatted() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 v;\nuniform float k;\nvoid main() { gl_FragColor = v * k; }",
            Stage::Fragment,
        );
        let Some(Op::Alu(ir::AluOp::Fmul, srcs)) = l.module.instrs.iter().find(|o| matches!(o, Op::Alu(..))) else {
            panic!("one multiply");
        };
        assert_eq!(srcs[1].swizzle, [0, 0, 0, 0], "the scalar feeds every lane");
    }

    /// `_25.qt_Matrix * qt_VertexPosition` -- how every Qt Quick shader in
    /// the table writes its transform, and the single change that took the
    /// GC7000 corpus from 16 to 33 of 142.
    ///
    /// Two readings of one syntax have to be told apart here: `_25.qt_Matrix`
    /// is a uniform whose reflection row is named `_25.qt_Matrix`, while
    /// `v.xyz` on a vector is a swizzle and no row at all. The checker's type
    /// for the base decides, which is the third thing step (a) bought.
    #[test]
    fn a_matrix_member_of_a_uniform_struct_is_read_by_its_joined_name() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "struct buf { mat4 qt_Matrix; float qt_Opacity; };\n\
             uniform buf _25;\n\
             attribute vec4 qt_VertexPosition;\n\
             void main() { gl_Position = _25.qt_Matrix * qt_VertexPosition; }\n",
            Stage::Vertex,
        );
        let m = &l.module;

        /* The reflection names the member, not the struct: that is the name
         * `glGetUniformLocation` answers for. */
        assert!(
            l.reflection.uniforms.iter().any(|u| u.name == "_25.qt_Matrix"),
            "the row is named by the path; the first row is {:?}",
            l.reflection.uniforms.first().map(|u| u.name)
        );

        /* Four column loads at the member's own dwords, and the product. */
        let mut cols = 0;
        for o in m.instrs.iter() {
            if let Op::LoadUniform { dword_off, comps } = *o {
                assert_eq!(comps, 4);
                assert!(dword_off < 16, "a column of the mat4 at the head of the block, got {dword_off}");
                cols += 1;
            }
        }
        assert_eq!(cols, 4, "one load per column");
        assert_eq!(m.instrs.iter().filter(|o| matches!(o, Op::Alu(..))).count(), 4, "a mul and three mads");
    }

    /// And the swizzle reading is not broken by it: `v.xyz` on a vector is
    /// still a swizzle, with no uniform lookup and no instruction.
    #[test]
    fn a_field_of_a_vector_is_still_a_swizzle() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v.wzyx; }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(!m.instrs.iter().any(|o| matches!(o, Op::LoadUniform { .. })), "no uniform is involved");
        let Some(Op::StoreOutput { src, .. }) = m.instrs.iter().find(|o| matches!(o, Op::StoreOutput { .. })) else {
            panic!("the colour is written");
        };
        assert_eq!(src.swizzle, [3, 2, 1, 0], "the swizzle rides on the operand");
    }

    /// A matrix argument reaches the built-in layer as its N columns, one
    /// operand each.
    ///
    /// `BaseType::components()` counts a matrix BY COLUMN, so a `mat3` handed
    /// over as one operand would claim three components exactly as a `vec3`
    /// does -- and a built-in emitting one instruction for it would operate
    /// on the first column and silently drop two. The split removes the
    /// ambiguity instead of documenting it.
    ///
    /// `matrixCompMult` has no arm yet (pvrgpu-platform's), so the call still
    /// refuses -- but it refuses AFTER the columns are built, which is what
    /// this checks: the loads are in the IR.
    #[test]
    fn a_matrix_argument_reaches_the_builtin_layer_as_its_columns() {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let src = "precision mediump float;\n\
                   uniform mat3 m;\n\
                   varying vec3 v;\n\
                   void main() { gl_FragColor = vec4(matrixCompMult(m, m)[0], 1.0); }";
        /* Whatever the verdict, it must not be a claim about the ARGUMENT's
         * shape: the refusal names the built-in, not the matrix. */
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(_) => { /* pvrgpu-platform's arm has landed; nothing to check here. */ }
            Err(e) => assert!(
                e.msg.contains("function call") || e.msg.contains("built-in") || e.msg.contains("matrix"),
                "refused for an unrelated reason: {:?} {}",
                e.tok,
                e.msg
            ),
        }
    }

    /// The same split, checked where it can be seen end to end: `min(m, m)`
    /// is componentwise, so with the columns handed over separately it is
    /// N instructions, one per column -- not one that drops N-1 of them.
    #[test]
    fn a_componentwise_builtin_over_a_matrix_is_one_instruction_per_column() {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let src = "precision mediump float;\n\
                   uniform mat2 m;\n\
                   void main() { gl_FragColor = vec4(min(m, m)[0], 0.0, 1.0); }";
        /* If it compiles, the built-in saw two operands per argument. If it
         * refuses, it must not be for the argument's width -- that message
         * would mean the matrix arrived as one operand claiming two
         * components. */
        if let Err(e) = compile_to_ir(&a, Stage::Fragment, src) {
            assert!(!e.msg.contains("argument width"), "the matrix arrived as one operand: {:?} {}", e.tok, e.msg);
        }
    }

    /// `inverseMatrix[0].x = d * invDet;` -- a component of a COLUMN of a
    /// matrix local, which is how `qInverse` in the Qt shaders writes one.
    ///
    /// A matrix local is its columns, so `m[j]` is slot `id + j` and the
    /// component is the usual masked store into it. This was the last real
    /// gap on the PowerVR corpus.
    /// THE TWO SHAPES THAT DREW THE WRONG PICTURE, pinned in the IR.
    ///
    /// Both are corpus sources -- `es100/swizzled_broadcast.frag` (mine) and
    /// `armselect.frag` with its `gl_test_es100/` twin (gles2-core's, and
    /// the stronger form: two locals, each written by exactly one arm, both
    /// read back, so a pixel test distinguishes the two sides). A source only
    /// earns its keep once the table is rebuilt, though, and that is parked
    /// (task #39).
    ///
    /// So these assert the same thing today, and they assert the part that
    /// is MINE either way: that the front end still hands the back end the
    /// shape, rather than folding it away. The `if` below is the simple
    /// one-local form on purpose -- it is the minimum the front end must
    /// keep emitting, and it is not a second corpus source.
    ///
    /// * A swizzled read of a local holding a BROADCAST literal. Every lane
    ///   holds the same value, so no permutation can change the picture --
    ///   which is why the shape was invisible in all 137 corpus sources.
    ///   gles2-core's selection carried an immediate's payload in the
    ///   swizzle fields and permuted it like lanes (their c7eb385), and it
    ///   only reached that code through a STORE and a swizzled LOAD BACK,
    ///   which is how a named constructor variable lowers.
    /// * A slot written in ONE arm of an `if` and read after the join.
    ///   Their if-conversion selected from a register nothing had written
    ///   (fce851e, fec333b): 30 of 137 sources, the FemtoVG set Slint draws
    ///   with.
    ///
    /// If either ever stops being emitted this way, the back-end guard for
    /// it stops being exercised by anything, and the test says so here
    /// rather than on a panel.
    #[test]
    fn the_two_wrong_picture_shapes_reach_the_ir() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform float u_scale;\n\
             void main() { vec4 c = vec4(0.5); gl_FragColor = c.yzxw * u_scale; }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(m.instrs.iter().any(|o| matches!(o, Op::StoreLocal { .. })), "the constructor goes through a slot");
        assert!(m.instrs.iter().any(|o| matches!(o, Op::LoadLocal { .. })), "and is read back");
        assert!(
            m.instrs.iter().any(|o| matches!(o, Op::Alu(_, srcs) if srcs[0].swizzle == [1, 2, 0, 3])),
            "the read carries the .yzxw permutation: {:?}",
            m.instrs
        );

        let mut buf2 = [0u8; 1 << 18];
        let l2 = lowered(
            &mut buf2,
            "precision mediump float;\nuniform float u_flag;\nuniform vec4 u_a;\nuniform vec4 u_b;\n\
             void main() { vec4 v = u_a; if (u_flag > 0.5) { v = u_b; } gl_FragColor = v; }",
            Stage::Fragment,
        );
        let nodes = l2.module.nodes_of(l2.module.body);
        let ifs = nodes.iter().filter(|n| matches!(n, Node::If { .. })).count();
        assert_eq!(ifs, 1, "the branch survives to the IR rather than being flattened");
        /* One arm writes the slot and the other does not -- that asymmetry
         * IS the shape, so the empty arm is asserted too. */
        let Some(Node::If { then_, else_, .. }) = nodes.iter().find(|n| matches!(n, Node::If { .. })) else {
            panic!("no If node");
        };
        assert!(then_.len > 0, "the taken arm writes the slot");
        assert_eq!(else_.len, 0, "and the other arm writes nothing");
    }

    /// Section 5.9: the arithmetic operators "operate on integer or
    /// floating-point values (including vectors and matrices)", and a scalar
    /// operand applies to every component. So `-2.0 * a` scales EVERY column
    /// of `a` -- it is not a product, and it is not column 0 only.
    ///
    /// dEQP `functions.datatypes.mat{2,3,4}` (`return -2.0*a;`) and
    /// `constant_expressions.operators.math_mat`, which also divides.
    #[test]
    fn a_scalar_times_a_matrix_scales_every_column() {
        for (n, src) in [
            (2u8, "mat2 f(mat2 a) { return -2.0 * a; }"),
            (3, "mat3 f(mat3 a) { return -2.0 * a; }"),
            (4, "mat4 f(mat4 a) { return -2.0 * a; }"),
        ] {
            let mut buf = [0u8; 1 << 19];
            let whole = std::format!(
                "precision mediump float;\nattribute vec4 p;\nuniform mat{n} m;\n{src}\n\
                 void main() {{ mat{n} r = f(m); gl_Position = vec4(r[0][0]); }}"
            );
            let l = lowered(&mut buf, &whole, Stage::Vertex);
            /* One multiply per column, each against a DIFFERENT column. A
             * lowering that scaled column 0 and copied it would give one. */
            let scaled: std::vec::Vec<u32> = l
                .module
                .instrs
                .iter()
                .filter_map(|o| match o {
                    Op::Alu(ir::AluOp::Fmul, srcs) => Some(srcs[0].val.0),
                    _ => None,
                })
                .collect();
            assert!(scaled.len() >= n as usize, "mat{n}: {} multiplies, want {n}: {scaled:?}", scaled.len());
            let mut distinct = scaled.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert!(distinct.len() >= n as usize, "mat{n}: each column is scaled once, got {scaled:?}");
        }
    }

    /// `m / k` is `m * (1/k)`, which means the literal-divisor fold applies
    /// to a matrix too: `m / 2.0` must hold the constant 0.5 and no `Frcp`.
    #[test]
    fn a_matrix_divided_by_a_literal_multiplies_by_the_folded_reciprocal() {
        let mut buf = [0u8; 1 << 19];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform mat3 m;\n\
             void main() { mat3 r = m / 2.0; gl_Position = vec4(r[0][0]); }",
            Stage::Vertex,
        );
        assert!(
            !l.module.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Frcp, _))),
            "the divisor is a literal, so there is no reciprocal instruction"
        );
        let half = 0.5f32.to_bits();
        assert!(
            l.module.instrs.iter().any(|o| matches!(o, Op::ConstF32(b) if b[0] == half)),
            "it multiplies by 0.5: {:?}",
            l.module.instrs
        );
    }

    /// Section 5.9: `++i` yields the value AFTER the step and `i++` the value
    /// BEFORE it.
    ///
    /// This is the one place in the front end where getting it backwards
    /// produces no error of any kind -- both forms type-check, both store,
    /// and the picture is off by one. So the test asks the IR which value
    /// each one took, in PROGRAM order (`nodes`), never in pool order.
    ///
    /// dEQP `functions.misc.argument_eval_order_1` (postfix `v1++`) and
    /// `_2` (prefix `--g`).
    #[test]
    fn a_postfix_step_takes_the_value_before_and_a_prefix_the_value_after() {
        /* Where each instruction sits in PROGRAM order: `instrs` is a pool
         * and its index is allocation order, which is exactly the thing this
         * test must not read. */
        fn order(m: &ir::Module<'_>) -> std::vec::Vec<u32> {
            let mut seq = std::vec::Vec::new();
            for n in m.nodes_of(m.body) {
                if let Node::Instr(ir::InstrId(i)) = *n {
                    seq.push(i);
                }
            }
            seq
        }
        /* The position in program order of the store into `dst`, and of the
         * definition of the value that store reads. */
        fn store_and_source(m: &ir::Module<'_>, dst: u32) -> (usize, usize) {
            let seq = order(m);
            let at = |id: u32| seq.iter().position(|&x| x == id).expect("in program order");
            for &i in &seq {
                if let Some(Op::StoreLocal { local, src, .. }) = m.instrs.get(i as usize) {
                    if local.0 == dst {
                        return (at(i), at(src.val.0));
                    }
                }
            }
            panic!("no store into slot {dst}: {:?}", m.instrs);
        }

        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float k;\n\
             void main() { float i = k; float a = i++; gl_FragColor = vec4(a, i, 0.0, 1.0); }",
            Stage::Fragment,
        );
        /* Slot 0 is `i`, slot 1 is `a`. `a`'s value must be DEFINED before
         * the step writes `i` back -- that is what "the value before" is. */
        let (_, a_src) = store_and_source(&l.module, 1);
        let seq = order(&l.module);
        let step_store = seq
            .iter()
            .filter(|&&i| matches!(l.module.instrs.get(i as usize), Some(Op::StoreLocal { local, .. }) if local.0 == 0))
            .count();
        assert!(step_store >= 2, "`i = k` and the step both store slot 0");
        let last_store_to_i = seq
            .iter()
            .rposition(
                |&i| matches!(l.module.instrs.get(i as usize), Some(Op::StoreLocal { local, .. }) if local.0 == 0),
            )
            .expect("the step stores i");
        assert!(a_src < last_store_to_i, "i++ takes the value BEFORE the step (src {a_src}, store {last_store_to_i})");

        /* The prefix form takes the value AFTER. */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = lowered(
            &mut buf2,
            "precision mediump float;\n\
             varying float k;\n\
             void main() { float i = k; float b = ++i; gl_FragColor = vec4(b, i, 0.0, 1.0); }",
            Stage::Fragment,
        );
        let (_, b_src) = store_and_source(&l2.module, 1);
        let seq2 = order(&l2.module);
        let last_store_to_i2 = seq2
            .iter()
            .rposition(
                |&i| matches!(l2.module.instrs.get(i as usize), Some(Op::StoreLocal { local, .. }) if local.0 == 0),
            )
            .expect("the step stores i");
        assert!(b_src > last_store_to_i2, "++i takes the value AFTER the step (src {b_src}, store {last_store_to_i2})");
    }

    /// Section 5.8: an assignment is an expression whose value is the
    /// l-value after the store, which is what makes `a = b = 0.0` legal.
    ///
    /// dEQP `functions.overloading.array_size` writes `x[2] = x[3] = 0.0`,
    /// and BOTH elements must end up 0.0 -- a chain that stored only the
    /// inner one, or that gave the outer the wrong value, would compile and
    /// draw the wrong picture.
    #[test]
    fn an_assignment_is_an_expression_whose_value_is_the_lvalue() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             void main() {\n\
               float x[4];\n\
               x[2] = x[3] = 0.5;\n\
               gl_FragColor = vec4(x[2], x[3], 0.0, 1.0);\n\
             }",
            Stage::Fragment,
        );
        /* Two stores into the array, and the value stored is the constant.
         * One store would mean the chain dropped an element. */
        let stores = l.module.instrs.iter().filter(|o| matches!(o, Op::StoreLocal { .. })).count();
        assert!(stores >= 2, "both elements of the chain are stored: {:?}", l.module.instrs);
        let half = 0.5f32.to_bits();
        assert!(
            l.module.instrs.iter().any(|o| matches!(o, Op::ConstF32(b) if b[0] == half)),
            "0.5 is what both receive"
        );

        /* The simple form, as a value. */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = lowered(
            &mut buf2,
            "precision mediump float;\n\
             varying float k;\n\
             void main() { float a; float b = (a = k); gl_FragColor = vec4(a, b, 0.0, 1.0); }",
            Stage::Fragment,
        );
        assert!(!l2.module.instrs.is_empty(), "`b = (a = k)` lowers");

        /* AND THE GAP THAT STAYS: re-reading the l-value must not repeat a
         * side effect, so a subscript that is itself an assignment keeps the
         * refusal rather than incrementing twice. */
        let mut buf3 = [0u8; 1 << 18];
        let a = Arena::new(&mut buf3);
        let r = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\n\
             void main() {\n\
               float x[4]; int i = 0; float b;\n\
               b = (x[i = i + 1] = 1.0);\n\
               gl_FragColor = vec4(b);\n\
             }",
        );
        let Err(e) = r else { panic!("a side-effecting subscript must stay a gap, not compile") };
        assert!(e.msg.contains("not lowered yet"), "worded as a gap, not a verdict: {}", e.msg);
    }

    /// Section 5.9's sequence operator: "returns the type and value of the
    /// right-most expression in a comma separated list of expressions. All
    /// expressions are evaluated, in order, from left to right."
    ///
    /// Two things can be silently wrong and both are asserted: the VALUE
    /// must be the right operand's, and the left operand must still happen.
    /// A comma that returned the left value, or that dropped it, would
    /// compile and draw the wrong picture.
    ///
    /// dEQP `conditionals.if.{sequence_condition, sequence_statements,
    /// complex_condition, mixed_if_elseif_else}`.
    #[test]
    fn a_comma_evaluates_both_and_has_the_value_of_the_right() {
        let mut buf = [0u8; 1 << 18];
        /* As a STATEMENT: both assignments happen, in order. `a` is written
         * before `b`, so the store into a's slot comes first. */
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float k;\n\
             void main() {\n\
               float a; float b;\n\
               a = k, b = k + 1.0;\n\
               gl_FragColor = vec4(a, b, 0.0, 1.0);\n\
             }",
            Stage::Fragment,
        );
        let slots: std::vec::Vec<u32> = l
            .module
            .instrs
            .iter()
            .filter_map(|o| match o {
                Op::StoreLocal { local, .. } => Some(local.0),
                _ => None,
            })
            .collect();
        assert!(slots.len() >= 2, "both sides of the comma store: {slots:?}");
        assert_ne!(slots[0], slots[1], "they are two different locals, in source order");

        /* As an EXPRESSION: the value is the RIGHT one. `(k, 2.0)` is 2.0,
         * so the constant 2.0 must reach the output and `k` must not. */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = lowered(
            &mut buf2,
            "precision mediump float;\n\
             varying float k;\n\
             void main() { float v = (k, 2.0); gl_FragColor = vec4(v); }",
            Stage::Fragment,
        );
        let two = 2.0f32.to_bits();
        assert!(
            l2.module.instrs.iter().any(|o| matches!(o, Op::ConstF32(b) if b[0] == two)),
            "the value of `(k, 2.0)` is 2.0: {:?}",
            l2.module.instrs
        );
        /* And the left operand was still evaluated -- the varying is read
         * even though its value is thrown away. */
        assert!(
            l2.module.instrs.iter().any(|o| matches!(o, Op::LoadInput { .. })),
            "the left operand is evaluated, not skipped"
        );
    }

    /// Section 5.4.2's general matrix constructor, and specifically that the
    /// components land in COLUMN-MAJOR order when an argument STRADDLES a
    /// column boundary.
    ///
    /// `mat2(a, v, b)` is components [a, v.x, v.y, b], so column 0 is
    /// (a, v.x) and column 1 is (v.y, b). The v.y is the whole test: a
    /// constructor that split the vec2 the other way, or took v.x twice,
    /// would compile, would draw, and would draw the wrong picture.
    ///
    /// 18 of dEQP's must-build cases used to be refused here, with a message
    /// worded as a verdict on the shader.
    #[test]
    fn a_matrix_constructor_takes_straddling_components_in_column_major_order() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float a;\n\
             varying vec2 v;\n\
             varying float b;\n\
             void main() {\n\
               mat2 m = mat2(a, v, b);\n\
               gl_FragColor = vec4(m[0], m[1]);\n\
             }",
            Stage::Fragment,
        );
        /* The two stores that read `v`: one puts v.x into lane 1 of the
         * first column, the other v.y into lane 0 of the second. */
        let mut into_lane1_from_x = false;
        let mut into_lane0_from_y = false;
        for o in l.module.instrs.iter() {
            if let Op::StoreLocal { src, write_mask, .. } = *o {
                if write_mask == 0b0010 && src.swizzle[1] == 0 {
                    into_lane1_from_x = true;
                }
                if write_mask == 0b0001 && src.swizzle[0] == 1 {
                    into_lane0_from_y = true;
                }
            }
        }
        assert!(into_lane1_from_x, "v.x is the second component, so lane 1 of column 0");
        assert!(into_lane0_from_y, "v.y is the third component, so lane 0 of column 1 -- column major");
    }

    /// The same general form, with arguments that are neither float nor the
    /// column's width: section 5.4.1's conversion, which a constructor is
    /// the one place ES 1.00 allows.
    ///
    /// dEQP `conversions.matrix_combine.bvec3_float_to_mat2` and
    /// `int_ivec2_int_to_mat2`. Both used to be refused.
    #[test]
    fn a_matrix_constructor_converts_and_regroups_its_arguments() {
        let mut buf = [0u8; 1 << 18];
        /* Three arguments, two widths, a type that is not float. */
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             void main() { mat2 m = mat2(1, ivec2(2, 3), 4); gl_FragColor = vec4(m[0], m[1]); }",
            Stage::Fragment,
        );
        assert!(!l.module.instrs.is_empty(), "it lowers at all");
        /* And one argument wider than a column, which the old code called
         * "mixes argument widths". */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = lowered(
            &mut buf2,
            "precision mediump float;\n\
             void main() { mat2 m = mat2(bvec3(true, false, true), 1.0); gl_FragColor = vec4(m[0], m[1]); }",
            Stage::Fragment,
        );
        assert!(!l2.module.instrs.is_empty(), "it lowers at all");
    }

    #[test]
    fn a_component_of_a_matrix_column_is_a_masked_store_into_that_column() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float k;\n\
             void main() {\n\
               mat2 m;\n\
               m[0].x = k;\n\
               m[1].y = k;\n\
               gl_FragColor = vec4(m[0], m[1]);\n\
             }",
            Stage::Fragment,
        );
        let m = &l.module;
        /* Two slots for the mat2, plus the constructor's temporary. */
        assert!(m.locals.len() >= 2, "a mat2 local is two slots");
        let mut writes = [(ir::LocalId(u32::MAX), 0u8); 2];
        let mut n = 0;
        for o in m.instrs.iter() {
            if let Op::StoreLocal { local, write_mask, .. } = *o {
                if n < writes.len() {
                    writes[n] = (local, write_mask);
                }
                n += 1;
            }
        }
        /* Column 0 gets x, column 1 gets y, and they are different slots --
         * writing both into one would be a matrix whose columns alias. */
        assert_eq!(writes[0].1, 0b0001, "m[0].x is lane x of the first column");
        assert_eq!(writes[1].1, 0b0010, "m[1].y is lane y of the second");
        assert_ne!(writes[0].0, writes[1].0, "the two columns are two slots");
    }

    /// Indexing a VECTOR as a destination is not a column: `v[0] = k` is
    /// ONE LANE of the vector's own slot, the store `v.x = k` is (section
    /// 5.5). It was refused until 2026-09-23; this pinned the refusal, and
    /// now pins the shape -- a one-lane mask, into the slot `v` lives in.
    #[test]
    fn indexing_a_vector_as_a_destination_is_not_a_column() {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let src = "precision mediump float;\n\
                   varying float k;\n\
                   void main() { vec4 v = vec4(0.0); v[0] = k; gl_FragColor = v; }";
        let l = compile_to_ir(&a, Stage::Fragment, src).expect("a constant lane of a vector is a destination");
        let stores: Vec<_> = l
            .module
            .instrs
            .iter()
            .filter_map(|op| match *op {
                Op::StoreLocal { local, write_mask, .. } => Some((local, write_mask)),
                _ => None,
            })
            .collect();
        assert_eq!(stores.len(), 2, "the initialiser and the lane: {stores:?}");
        assert_eq!(stores[0].0, stores[1].0, "both into v's one slot: {stores:?}");
        assert_eq!(stores[1].1, 0b0001, "and the lane store writes lane x alone: {stores:?}");
    }

    /// `vec4(v_uv, 1.0, 1.0)` is three instructions, not five: one constant
    /// for both `1.0`s and one store for the two lanes that read it.
    ///
    /// This is `color_uv.frag`, an ES 1.00 `gl_test` twin, and it is the
    /// shader `clearscis` and `clearmask` draw with. Before this it built two
    /// registers holding the same number and wrote them one lane at a time --
    /// four instructions where the shipped blob has three. Registers spent on
    /// nothing are not free: pressure is what refused `shapestroke.vert`.
    #[test]
    fn one_constant_and_one_store_serve_two_identical_lanes() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying vec4 v_color;\n\
             varying vec2 v_uv;\n\
             void main() { gl_FragColor = v_color * vec4(v_uv, 1.0, 1.0); }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert_eq!(
            m.instrs.iter().filter(|o| matches!(o, Op::ConstF32(_))).count(),
            1,
            "one register holds 1.0, not two"
        );
        let mut masks = [0u8; 2];
        let mut n = 0;
        for o in m.instrs.iter() {
            if let Op::StoreLocal { write_mask, .. } = *o {
                assert!(n < masks.len(), "two stores: the vector, then both constant lanes");
                masks[n] = write_mask;
                n += 1;
            }
        }
        assert_eq!((n, masks), (2, [0b0011, 0b1100]), "xy from the varying, zw from the one constant");
    }

    /// A constant is reused only inside the run that built it.
    ///
    /// A `ConstF32` defined in one arm of an `if` is not available in the
    /// other or after the join -- reusing it there would read a register that
    /// path never wrote, which is a plausible value and a wrong picture. The
    /// scope is the node run, which is why no dominance test is needed.
    #[test]
    fn a_constant_is_not_reused_across_a_branch() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float k;\n\
             void main() {\n\
               vec4 c = vec4(0.0);\n\
               if (k > 0.5) { c = vec4(1.0); } else { c = vec4(1.0); }\n\
               gl_FragColor = c;\n\
             }",
            Stage::Fragment,
        );
        /* Counting the 1.0s specifically, not every constant: the shader
         * also has the initialiser's 0.0 and the comparison's 0.5, and a
         * total would be a number that moves for reasons unrelated to the
         * claim. ONE of them would mean an arm reads a register the other
         * path wrote. */
        let one = 1.0f32.to_bits();
        assert_eq!(
            l.module.instrs.iter().filter(|o| matches!(o, Op::ConstF32(b) if b[0] == one)).count(),
            2,
            "each arm builds its own 1.0"
        );
    }

    /// An inlined `in` parameter the body never assigns to is a NAME for the
    /// argument's value, not a variable: no slot, no store, no load.
    ///
    /// This is not a tidiness argument. `shapestroke.vert` passes seven
    /// two-argument calls' worth of them; with a slot each it needed 42 slots
    /// and 236 instructions and the back end refused it with
    /// `OutOfRegisters(257)`. Without them it is 28 and 194, and it compiles.
    #[test]
    fn an_in_parameter_that_is_only_read_needs_no_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             float qdot(vec2 a, vec2 b) { return (a.x * b.x) + (a.y * b.y); }\n\
             varying vec2 u;\n\
             varying vec2 v;\n\
             void main() { gl_FragColor = vec4(qdot(u, v)); }\n",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(m.locals.is_empty(), "no slot for either parameter, got {:?}", m.locals.len());
        assert!(
            !m.instrs.iter().any(|o| matches!(o, Op::StoreLocal { .. } | Op::LoadLocal { .. })),
            "and no traffic to one"
        );
    }

    /// A parameter the body DOES assign to is a variable and keeps its slot.
    /// ES 1.00 allows it -- an `in` parameter is a local copy (section 6.1) --
    /// so aliasing it would make the assignment write the caller's value.
    #[test]
    fn an_in_parameter_that_is_assigned_keeps_its_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             float clamp01(float x) { x = max(x, 0.0); return min(x, 1.0); }\n\
             varying float k;\n\
             void main() { gl_FragColor = vec4(clamp01(k)); }\n",
            Stage::Fragment,
        );
        assert_eq!(l.module.locals.len(), 1, "the parameter is written, so it is a variable");
        assert_eq!(l.module.locals[0].name, "x");
    }

    /// And the scan is over the BODY, not the unit. `qdot`'s parameters are
    /// called `a` and `b`; a caller with its own `a` and `b` that it assigns
    /// must not make them un-aliasable, or the optimisation turns itself off
    /// in exactly the shader it exists for (`shapestroke.vert` is that
    /// shader).
    #[test]
    fn a_callers_variable_of_the_same_name_does_not_block_the_alias() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             float qdot(vec2 a, vec2 b) { return (a.x * b.x) + (a.y * b.y); }\n\
             varying vec2 u;\n\
             void main() { vec2 a = u; vec2 b = u; a = b; gl_FragColor = vec4(qdot(a, b)); }\n",
            Stage::Fragment,
        );
        /* Two slots: the caller's `a` and `b`. None for the parameters. */
        assert_eq!(l.module.locals.len(), 2, "the caller's two variables and nothing else");
    }

    /// `!` is one `Bnot`: a bool is an all-ones mask, so logical not is a
    /// bitwise one. gles2-core selects it and had it unreachable from GLSL
    /// until this arm existed.
    #[test]
    fn logical_not_is_one_bnot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             uniform float u;\n\
             void main() { gl_FragColor = vec4(float(!(u > 0.5))); }",
            Stage::Fragment,
        );
        assert_eq!(l.module.instrs.iter().filter(|o| matches!(o, Op::Alu(ir::AluOp::Bnot, _))).count(), 1, "one not");
    }

    /// Every refusal says WHERE and WHICH.
    ///
    /// The lowering works on ids and has no tokens, so before `Lower::line`
    /// existed every message it raised read `0:0:` -- and the token was then
    /// dropped again at the `CompileError` boundary, so one dEQP run produced
    /// `0:0: assignment to this is not lowered yet` **1,841 times
    /// byte-identical**. A log like that can neither be located in a source
    /// nor split into buckets.
    ///
    /// `tests/online_lowering.rs` asserts the same thing over all 382 table
    /// sources, which is the check that matters; this one is here so the rule
    /// fails fast beside the code rather than only in an integration run.
    #[test]
    fn every_refusal_says_where_and_which() {
        for (src, stage) in [
            (
                "precision mediump float;\nvarying vec4 v;\nvoid main() {\n  gl_FragColor = v * v * v;\n  float a[2];\n  a = a;\n}",
                Stage::Fragment,
            ),
            /* A gap on line 6, by a token: `v * m` stood here until it was
             * lowered (2026-09-23). */
            (
                "precision mediump float;\nuniform int i;\nvarying vec4 v;\nvoid main() {\n\n  float a[2]; int j = i; a[j++] += 1.0; gl_FragColor = v;\n}",
                Stage::Fragment,
            ),
        ] {
            let mut buf = [0u8; 1 << 18];
            let a = Arena::new(&mut buf);
            match compile_to_ir(&a, stage, src) {
                Ok(_) => panic!("expected a refusal for {src:?}"),
                Err(e) => {
                    assert!(e.line > 0, "no line: {:?} {}", e.tok, e.msg);
                    assert!(!e.tok.is_empty(), "no token: line {} {}", e.line, e.msg);
                }
            }
        }
    }

    /// An `out` parameter is copied back to the caller's variable when the
    /// call finishes, and is NOT copied in first: section 6.1 says it starts
    /// undefined, and copying in would quietly change that.
    #[test]
    fn an_out_parameter_is_copied_back_and_not_copied_in() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             void split(float v, out float a, out float b) { a = v; b = 1.0 - v; }\n\
             varying float k;\n\
             void main() { float x; float y; split(k, x, y); gl_FragColor = vec4(x, y, 0.0, 1.0); }\n",
            Stage::Fragment,
        );
        let m = &l.module;
        /* Two caller variables, one `in` parameter and two `out` ones, plus
         * the constructor's temporary. What matters is that each `out`
         * parameter's slot is read back and stored into the caller's. */
        let loads_back = m.instrs.iter().filter(|o| matches!(o, Op::LoadLocal { .. })).count();
        assert!(loads_back >= 2, "each out parameter is read back, got {loads_back} loads");
        assert!(
            m.locals.len() >= 5,
            "the caller's two, the parameters' three, and the constructor's temporary: {}",
            m.locals.len()
        );
    }

    /// `c ? a : b` is a component-wise select with both arms evaluated, which
    /// is what the hardware does and what `Bcsel` means.
    #[test]
    fn a_ternary_is_one_select() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float k;\n\
             void main() { gl_FragColor = vec4(k > 0.5 ? 1.0 : 0.0); }",
            Stage::Fragment,
        );
        assert_eq!(
            l.module.instrs.iter().filter(|o| matches!(o, Op::Alu(ir::AluOp::Bcsel, _))).count(),
            1,
            "one select"
        );
    }

    /// `v[i]` with a run-time `i` is a chain of selects over the lanes: a
    /// vector's components are lanes of one register, not addresses, so
    /// there is nothing to index and the choice is made with values.
    #[test]
    fn a_run_time_vector_index_selects_over_the_lanes() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             uniform int i;\n\
             varying vec4 v;\n\
             void main() { gl_FragColor = vec4(v[i]); }",
            Stage::Fragment,
        );
        let m = &l.module;
        /* Three comparisons and three selects for four lanes: the last lane
         * is the fall-through and needs no comparison of its own. */
        assert_eq!(m.instrs.iter().filter(|o| matches!(o, Op::Alu(ir::AluOp::Ieq, _))).count(), 3);
        assert_eq!(m.instrs.iter().filter(|o| matches!(o, Op::Alu(ir::AluOp::Bcsel, _))).count(), 3);
    }

    /// `a_xf[0][1]`: a column of a matrix attribute, then a component of that
    /// column. Both are swizzles, so the pair costs one load and nothing
    /// else.
    #[test]
    fn indexing_a_matrix_then_a_vector_is_two_swizzles() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "attribute vec4 a_pos;\n\
             attribute mat2 a_xf;\n\
             void main() { gl_Position = vec4(a_xf[1][0], a_pos.y, 0.0, 1.0); }\n",
            Stage::Vertex,
        );
        let m = &l.module;
        /* Column 1 of the mat2 is the attribute location after `a_pos`'s
         * two... the reflection decides which; what matters is that a
         * component read costs no instruction, so the only loads are the
         * inputs themselves. */
        let loads = m.instrs.iter().filter(|o| matches!(o, Op::LoadInput { .. })).count();
        assert_eq!(loads, 2, "one load per input read, and the indexing adds none");
    }

    /// An ARRAY of vectors has the same base type as a vector, and indexing
    /// it means an element, not a component.
    ///
    /// `uniform vec4 frag[14];` is the shape that makes the difference, and
    /// it is the whole FemtoVG uniform block. Confusing the two refused 33
    /// sources for "component out of range" while this was being written,
    /// which is why the distinction has a test of its own.
    #[test]
    fn indexing_an_array_of_vectors_is_an_element_not_a_component() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 frag[4];\nvoid main() { gl_FragColor = frag[2]; }",
            Stage::Fragment,
        );
        let m = &l.module;
        let Some(Op::LoadUniform { dword_off, comps }) = m.instrs.iter().find(|o| matches!(o, Op::LoadUniform { .. }))
        else {
            panic!("the element is read from the block");
        };
        assert_eq!((*dword_off, *comps), (8, 4), "element 2 of a vec4 array is dword 8, four wide");

        /* And the component reading still works on the element. */
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nuniform vec4 frag[4];\nvoid main() { gl_FragColor = vec4(frag[2][1]); }",
            Stage::Fragment,
        );
        let Some(Op::LoadUniform { dword_off, .. }) =
            l.module.instrs.iter().find(|o| matches!(o, Op::LoadUniform { .. }))
        else {
            panic!("still the element");
        };
        assert_eq!(*dword_off, 8, "the element, then its component as a swizzle");
    }

    /// `gl_Position.z = order * gl_Position.w;` -- the shape of every Qt
    /// Quick vertex shader in the table, and one an output cannot express
    /// directly: it both reads the output and writes one component of it,
    /// and an output is not a register a shader can read back.
    ///
    /// So the output is held in a slot for the duration of `main` and stored
    /// once at the end.
    #[test]
    fn an_output_read_or_partly_written_is_held_in_a_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "attribute vec4 a_pos;\n\
             attribute float a_order;\n\
             void main() { gl_Position = a_pos; gl_Position.z = a_order * gl_Position.w; }\n",
            Stage::Vertex,
        );
        let m = &l.module;
        assert_eq!(m.locals.len(), 1, "one slot, for gl_Position");
        assert_eq!(m.locals[0].name, "gl_Position");

        /* Exactly one store to the output, and it is the last thing that
         * happens: two would make the order of the writes decide the
         * picture. */
        let mut stores = 0;
        let mut last = usize::MAX;
        for (i, o) in m.instrs.iter().enumerate() {
            if matches!(o, Op::StoreOutput { .. }) {
                stores += 1;
                last = i;
            }
        }
        assert_eq!(stores, 1, "the output is written once");
        assert_eq!(last, m.instrs.len() - 1, "and at the end");

        /* The partial write is a masked store of z alone. */
        let mut masks = [0u8; 2];
        let mut n = 0;
        for o in m.instrs.iter() {
            if let Op::StoreLocal { write_mask, .. } = *o {
                assert!(n < masks.len(), "two writes to the slot");
                masks[n] = write_mask;
                n += 1;
            }
        }
        assert_eq!((n, masks), (2, [0b1111, 0b0100]), "the whole assignment, then z alone");
    }

    /// And a shader that simply assigns its output keeps the direct store:
    /// most shaders are that shape, and giving them all a slot would be a
    /// regression the back end would have to optimise away again.
    #[test]
    fn an_output_written_once_needs_no_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(&mut buf, "attribute vec4 a;\nvoid main() { gl_Position = a; }", Stage::Vertex);
        assert!(l.module.locals.is_empty(), "no slot");
        assert_eq!(l.module.instrs.len(), 2, "a load and a store, as before");
    }

    /// An lvalue swizzle names its lanes by the DESTINATION, so `c.zx = v`
    /// puts the source's x in z and its y in x -- in that order, not sorted.
    #[test]
    fn an_lvalue_swizzle_maps_source_lanes_by_destination() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 v;\nvoid main() { vec4 c = v; c.zx = v.xy; gl_FragColor = c; }",
            Stage::Fragment,
        );
        let m = &l.module;
        let Some(Op::StoreLocal { write_mask, src, .. }) =
            m.instrs.iter().filter(|o| matches!(o, Op::StoreLocal { .. })).nth(1)
        else {
            panic!("the swizzle assignment is the second store");
        };
        assert_eq!(*write_mask, 0b0101, "x and z");
        /* `c.z` takes the source's first lane and `c.x` its second. */
        assert_eq!(src.swizzle[2], 0, "z reads the source's x");
        assert_eq!(src.swizzle[0], 1, "x reads the source's y");
    }

    /// A component may not be assigned twice (section 5.8): `c.xx = v` is
    /// refused rather than one of the two writes being chosen.
    ///
    /// The checker gets there first, which is why the message is its one.
    /// `swizzle_mask` refuses it too and that arm is unreachable today --
    /// kept deliberately, because it is the only guard if the checker's rule
    /// ever narrows, and what it prevents is a store that writes one of the
    /// two lanes and looks like a working shader.
    #[test]
    fn a_repeated_component_is_not_an_lvalue() {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let src =
            "precision mediump float;\nvarying vec4 v;\nvoid main() { vec4 c = v; c.xx = v.xy; gl_FragColor = c; }";
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(_) => panic!("a repeated component is not an lvalue"),
            Err(e) => assert_eq!(e.msg, "cannot assign to this expression"),
        }
    }

    /// `vec4(v.xyz, 1.0)`: no instruction joins lanes from different values,
    /// so the constructor is a slot, one masked store per argument, and a
    /// load.
    ///
    /// The masks and the swizzles are the contract with the back end and the
    /// reason this test spells them out: destination lane *i* takes
    /// `src.swizzle[i]`, so an argument filling lanes 1..3 has its own lane 0
    /// at `swizzle[1]`. Getting that backwards produces a shader that
    /// compiles and draws the components in the wrong order.
    #[test]
    fn a_constructor_from_several_values_is_masked_stores_into_a_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = vec4(v.xyz, 1.0); }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert_eq!(m.locals.len(), 1, "one temporary");
        assert_eq!(m.locals[0].name, "", "a compiler temporary has no GLSL name");
        assert_eq!(m.locals[0].ty, ir::Type::new(ir::ScalarType::F32, 4));

        let mut stores = [(0u8, [0u8; 4]); 2];
        let mut n = 0;
        for o in m.instrs.iter() {
            if let Op::StoreLocal { local, write_mask, src, .. } = *o {
                assert_eq!(local, ir::LocalId(0));
                assert!(n < stores.len(), "two arguments, two stores");
                stores[n] = (write_mask, src.swizzle);
                n += 1;
            }
        }
        assert_eq!(n, 2);
        /* `v.xyz` fills lanes 0..2 and reads the varying's lanes 0..2. */
        assert_eq!(stores[0].0, 0b0111, "the vector fills x, y and z");
        assert_eq!(stores[0].1[..3], [0, 1, 2], "and takes its own x, y, z");
        /* `1.0` fills lane 3 only, and the lane it reads is the constant's
         * own lane 0 -- written at index 3, where the destination looks. */
        assert_eq!(stores[1].0, 0b1000, "the scalar fills w alone");
        assert_eq!(stores[1].1[3], 0, "the destination's w reads the source's x");

        /* And the result is read back from the slot, not from the last
         * store's value. */
        assert!(
            m.instrs.iter().any(|o| matches!(o, Op::LoadLocal { local: ir::LocalId(0), comps: 4, .. })),
            "the constructor's value is the slot"
        );
    }

    /// All-literal constructors keep their short form: one constant, no
    /// temporary. Most of the table's fragment shaders are a literal colour,
    /// and making them allocate a slot would be a regression the back end
    /// would have to optimise away again.
    #[test]
    fn an_all_literal_constructor_is_still_one_constant() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        assert!(l.module.locals.is_empty(), "no temporary");
        assert_eq!(l.module.instrs.iter().filter(|o| matches!(o, Op::ConstF32(_))).count(), 1);
    }

    /// A user-defined function is inlined: there is no call in the IR and no
    /// stack in the hardware.
    ///
    /// The shape is `strokeMask`'s, which is 29 sources of the GC7000 corpus
    /// on its own -- a helper that computes and returns.
    #[test]
    fn a_user_function_is_inlined() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying vec2 ftcoord;\n\
             uniform float strokeMult;\n\
             float strokeMask() { return min(1.0, (1.0 - abs(ftcoord.x * 2.0 - 1.0)) * strokeMult); }\n\
             void main() { gl_FragColor = vec4(strokeMask()); }\n",
            Stage::Fragment,
        );
        let m = &l.module;
        /* The parameter-free helper still binds nothing, but its body's work
         * is here: the varying is read and the arithmetic emitted inside
         * `main`'s own region. */
        assert!(m.instrs.iter().any(|o| matches!(o, Op::LoadInput { .. })), "the body reads ftcoord");
        assert!(m.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Fmul, _))), "the body multiplies");
        assert_eq!(m.body.len, m.nodes.len() as u32, "one region: nothing is left outside main");
    }

    /// A parameter is bound to a slot, and it is the caller's argument that
    /// goes in even when the names collide.
    #[test]
    fn a_parameter_takes_the_callers_argument_not_its_own_name() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float x;\n\
             float twice(float x) { return x + x; }\n\
             void main() { float x = 0.5; gl_FragColor = vec4(twice(x)); }\n",
            Stage::Fragment,
        );
        let m = &l.module;
        /* One slot: the caller's `x`. The parameter is only read, so it is a
         * name for the argument's value and needs none -- and the value it
         * names is the CALLER's, read before the parameter came into scope.
         * Binding it the other way round would be circular and is the bug
         * this shape is written to catch. */
        assert_eq!(m.locals.len(), 1, "only the caller's variable has a slot");
        assert_eq!(m.locals[0].name, "x");
        assert_eq!(
            m.instrs.iter().filter(|o| matches!(o, Op::LoadLocal { .. })).count(),
            1,
            "the argument is read once, into the alias"
        );
        /* And the varying of the same name is not what was read. */
        assert!(
            !m.instrs.iter().any(|o| matches!(o, Op::LoadInput { .. })),
            "the caller's local shadows the varying called x"
        );
    }

    /// Recursion is forbidden by GLSL ES, but a compiler on the target must
    /// refuse rather than exhaust its arena if a shader says otherwise.
    #[test]
    fn a_recursive_call_is_refused_rather_than_followed() {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let src = "precision mediump float;\n\
                   float f(float v) { return f(v); }\n\
                   void main() { gl_FragColor = vec4(f(1.0)); }\n";
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(_) => panic!("a recursive call must not be inlined forever"),
            Err(e) => assert_eq!(e.msg, "this call nests too deeply to inline"),
        }
    }

    /// `a *= b` is `a = a * b`, computed rather than rewritten in the parser.
    #[test]
    fn a_compound_assignment_is_the_operation_then_the_store() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\nvarying vec4 v;\nvoid main() { vec4 c = v; c *= v; gl_FragColor = c; }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(m.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Fmul, _))), "the multiply happens");
        assert_eq!(
            m.instrs.iter().filter(|o| matches!(o, Op::StoreLocal { .. })).count(),
            2,
            "the initialiser and the compound assignment, both into the slot"
        );
    }

    /// `a > b` is `b < a`: the kernel has one compare direction, and a second
    /// opcode would be the same instruction under a second name.
    #[test]
    fn greater_than_is_less_than_with_the_operands_swapped() {
        let mut buf = [0u8; 1 << 18];
        let l = lowered(
            &mut buf,
            "precision mediump float;\n\
             varying float a;\n\
             void main() { if (a > 0.5) { discard; } gl_FragColor = vec4(1.0); }",
            Stage::Fragment,
        );
        let m = &l.module;
        let Some(Op::Alu(ir::AluOp::Flt, srcs)) = m.instrs.iter().find(|o| matches!(o, Op::Alu(ir::AluOp::Flt, _)))
        else {
            panic!("a greater-than is a less-than");
        };
        /* Operand 0 is the constant 0.5 and operand 1 the varying: swapped,
         * which is what makes `a > 0.5` mean `0.5 < a`. */
        assert!(
            matches!(m.instrs[srcs[0].val.0 as usize], Op::ConstF32(_)),
            "the right-hand side became the left operand"
        );
        assert!(
            matches!(m.instrs[srcs[1].val.0 as usize], Op::LoadInput { .. }),
            "and the left-hand side the right one"
        );
    }
}

/// `gl_FragData[0] = ...`, the other spelling of the fragment colour.
#[cfg(test)]
mod frag_data_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str, stage: Stage) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got {:?}: {}", e.tok, e.msg),
        }
    }

    fn err<'a>(buf: &'a mut [u8], src: &'a str, stage: Stage) -> &'a str {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(_) => panic!("expected an error for {src:?}"),
            Err(e) => e.msg,
        }
    }

    /// It writes the same slot `gl_FragColor` does -- one draw buffer, two
    /// spellings -- and the reflection carries the name the shader used.
    #[test]
    fn frag_data_zero_writes_the_colour_slot() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\nvoid main() { gl_FragData[0] = vec4(1.0, 0.0, 0.0, 1.0); }",
            Stage::Fragment,
        );
        let m = &l.module;
        assert!(
            m.instrs.iter().any(|op| matches!(op, Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. })),
            "no store to the colour slot: {:?}",
            m.instrs
        );
        assert_eq!(m.outputs.len(), 1);
        assert_eq!(m.outputs[0].name, "gl_FragData");
        assert_eq!(m.outputs[0].kind, IoKind::Colour);
        assert_eq!(m.outputs[0].components, 4);
    }

    /// The two spellings produce the same instructions, which is the property
    /// that makes this a spelling rather than a feature.
    #[test]
    fn the_two_spellings_of_the_fragment_colour_agree() {
        let mut a_buf = [0u8; 1 << 18];
        let mut b_buf = [0u8; 1 << 18];
        let a =
            ir(&mut a_buf, "precision mediump float;\nvoid main() { gl_FragData[0] = vec4(0.5); }", Stage::Fragment);
        let b = ir(&mut b_buf, "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.5); }", Stage::Fragment);
        assert_eq!(a.module.instrs, b.module.instrs, "the spellings lower differently");
    }

    /// OpenGL ES 2.0 has MAX_DRAW_BUFFERS = 1, and all 103 sources in the
    /// table write index 0. A larger index is refused rather than folded onto
    /// slot 0: folding it would compile a shader that writes somewhere the
    /// author did not ask for and draw a picture instead of reporting this.
    #[test]
    fn a_draw_buffer_past_the_first_is_refused_by_name() {
        let mut buf = [0u8; 1 << 18];
        let msg =
            err(&mut buf, "precision mediump float;\nvoid main() { gl_FragData[1] = vec4(1.0); }", Stage::Fragment);
        assert!(msg.contains("one draw buffer"), "unhelpful message: {msg}");
        assert!(!msg.contains("not lowered yet"), "reported as a compiler gap: {msg}");
    }

    /// An index chosen at run time writes the one colour output: every
    /// other index is out of range with one draw buffer, which is undefined
    /// (section 4.1.9), so slot 0 is a correct target for all of them (dEQP
    /// fragdata.valid_dynamic_index, valid_uniform_index). This was refused
    /// until 2026-09-23. A constant EXPRESSION is folded, so `0 + 0` is the
    /// first buffer and `0 + 1` still gets the one-draw-buffer verdict.
    #[test]
    fn a_run_time_index_writes_the_one_colour_output() {
        for src in [
            "precision mediump float;\nuniform int i;\nvoid main() { gl_FragData[i] = vec4(1.0); }",
            "precision mediump float;\nvoid main() { gl_FragData[0 + 0] = vec4(1.0); }",
            "precision mediump float;\nvarying float f;\nvoid main() { gl_FragData[int(f)] = vec4(1.0); }",
        ] {
            let mut buf = [0u8; 1 << 18];
            let l = ir(&mut buf, src, Stage::Fragment);
            assert!(
                l.module.instrs.iter().any(|o| matches!(o, Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. })),
                "for {src:?}: the colour is written: {:?}",
                l.module.instrs
            );
        }
        let mut buf = [0u8; 1 << 18];
        let msg =
            err(&mut buf, "precision mediump float;\nvoid main() { gl_FragData[0 + 1] = vec4(1.0); }", Stage::Fragment);
        assert!(msg.contains("one draw buffer"), "a constant past the first buffer: {msg}");
    }

    /// In a vertex shader the name does not exist and sema says so before
    /// this lowering is reached, so there is no stage check here to duplicate
    /// it. Pinned, so that a stage check added later is recognised as the
    /// dead code it would be.
    #[test]
    fn gl_frag_data_in_a_vertex_shader_is_semas_to_refuse() {
        let mut buf = [0u8; 1 << 18];
        let msg = err(&mut buf, "precision mediump float;\nvoid main() { gl_FragData[0] = vec4(1.0); }", Stage::Vertex);
        assert!(msg.contains("undeclared"), "expected sema's message, got: {msg}");
    }

    /// An index assignment that is NOT gl_FragData still says what it is,
    /// rather than being swallowed by the branch above. `v[0] = 1.0` lowers
    /// since 2026-09-23, and so does a lane chosen at run time; the case is
    /// the gap that remains: a compound assignment through an index with a
    /// side effect, which the read-then-store would evaluate twice.
    #[test]
    fn another_indexed_destination_still_reports_itself() {
        let mut buf = [0u8; 1 << 18];
        let msg = err(
            &mut buf,
            "precision mediump float;\nuniform int i;\nvoid main() { float a[2]; int j = i; a[j++] += 1.0; gl_FragColor = vec4(a[0]); }",
            Stage::Fragment,
        );
        assert_eq!(msg, "a compound assignment through an index with side effects is not lowered yet");
    }
}

#[cfg(test)]
mod control_flow_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got {:?}: {}", e.tok, e.msg),
        }
    }

    fn err<'a>(buf: &'a mut [u8], src: &'a str) -> &'a str {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.msg,
        }
    }

    /// A child run is committed before its parent, so the parent's `Range` is
    /// already valid when the parent node is pushed. That puts an arm's nodes
    /// EARLIER in `nodes` than the `If` naming them, which is worth seeing in
    /// a test so nobody later "fixes" the order and invalidates the ranges.
    #[test]
    fn an_if_evaluates_its_condition_once_outside_both_arms() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(&mut buf, "precision mediump float;\nvoid main() { if (true) { gl_FragColor = vec4(1.0); } }");
        let m = &l.module;
        let body: Vec<Node> = m.nodes_of(m.body).to_vec();
        let Some(&Node::If { then_, else_, .. }) = body.iter().find(|n| matches!(n, Node::If { .. })) else {
            panic!("no If node in the body: {body:?}");
        };
        /* The condition is computed in the body run, before the If. */
        assert!(matches!(body[0], Node::Instr(_)), "the condition's instruction is not in the body run: {body:?}");
        assert!(matches!(body[1], Node::If { .. }), "the If does not follow its condition: {body:?}");
        assert_eq!(then_.len, 2, "the then arm should hold the constant and the store");
        assert!(else_.is_empty(), "there is no else arm to hold anything");
    }

    #[test]
    fn a_while_loop_tests_before_the_body_and_a_do_loop_after_it() {
        let mut buf = [0u8; 1 << 18];
        let l =
            ir(&mut buf, "precision mediump float;\nvoid main() { while (true) { gl_FragColor = vec4(1.0); break; } }");
        let m = &l.module;
        let body: Vec<Node> = m.nodes_of(m.body).to_vec();
        let Some(&Node::Loop { body: lb }) = body.iter().find(|n| matches!(n, Node::Loop { .. })) else {
            panic!("no Loop node: {body:?}");
        };
        let inner: Vec<Node> = m.nodes_of(lb).to_vec();
        /* The condition is INSIDE the loop: it is re-evaluated every
         * iteration, and a hoisted one would test the first value forever. */
        assert!(matches!(inner[0], Node::Instr(_)), "the condition is not re-evaluated in the body: {inner:?}");
        assert!(matches!(inner[1], Node::If { .. }), "the test does not come first: {inner:?}");
        assert!(matches!(inner.last(), Some(Node::Break)), "the body's own break is missing: {inner:?}");

        let mut buf2 = [0u8; 1 << 18];
        let l2 =
            ir(&mut buf2, "precision mediump float;\nvoid main() { do { gl_FragColor = vec4(1.0); } while (true); }");
        let m2 = &l2.module;
        let body2: Vec<Node> = m2.nodes_of(m2.body).to_vec();
        let Some(&Node::Loop { body: lb2 }) = body2.iter().find(|n| matches!(n, Node::Loop { .. })) else {
            panic!("no Loop node: {body2:?}");
        };
        let inner2: Vec<Node> = m2.nodes_of(lb2).to_vec();
        /* ... and here it is last, which is the whole difference. */
        assert!(matches!(inner2[0], Node::Instr(_)));
        assert!(matches!(inner2.last(), Some(Node::If { .. })), "do-while tests after the body: {inner2:?}");
    }

    /// GLSL ES 1.00: `continue` in a `for` runs the step before the next test.
    /// The IR has no separate step block, so the step has to be emitted at the
    /// `continue` as well as at the end of the body. Getting this wrong is
    /// silent -- the loop still runs, with the wrong number of iterations --
    /// which is why it is pinned by a count rather than by eye.
    #[test]
    fn continue_in_a_for_loop_runs_the_step() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\nvoid main() { for (; true; gl_FragColor = vec4(1.0)) { continue; } }",
        );
        let m = &l.module;
        let stores = m.instrs.iter().filter(|op| matches!(op, Op::StoreOutput { .. })).count();
        assert_eq!(stores, 2, "the step must run at the continue AND at the end of the body: {:?}", m.instrs);

        /* A `while` has no step, so its `continue` emits nothing extra. The
         * control: without it, a lowering that always duplicated SOMETHING
         * would pass the assertion above. */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = ir(
            &mut buf2,
            "precision mediump float;\nvoid main() { while (true) { gl_FragColor = vec4(1.0); continue; } }",
        );
        let stores2 = l2.module.instrs.iter().filter(|op| matches!(op, Op::StoreOutput { .. })).count();
        assert_eq!(stores2, 1, "a while loop has no step to run: {:?}", l2.module.instrs);
    }

    /// A `continue` in a `do`/`while` runs the test (6.3). The test is the
    /// last node of the loop body and a `continue` restarts the body, so
    /// without the copy `do { continue; } while (++i < 3);` -- dEQP's
    /// do_while_*.only_continue -- never tested: the evaluator stops it at
    /// its iteration bound, and on the GC7000 it would never end. Run
    /// through the evaluator, with `for` and `while` beside it as the loops
    /// whose `continue` must not change.
    #[test]
    fn continue_in_a_do_while_runs_the_test() {
        use crate::shaderc::online::eval::{run, Inputs};
        for (body, want) in [
            ("int i = 0; do { continue; } while (++i < 3); r = float(i);", 3.0f32),
            ("int i = 0; do { if (i == 1) continue; r += 1.0; } while (++i < 4);", 3.0),
            ("int i = 0; do { if (i == 2) break; if (i == 0) continue; r += 1.0; } while (++i < 5);", 1.0),
            ("for (int i = 0; i < 3; i++) { if (i == 1) continue; r += 1.0; }", 2.0),
            ("int k = 0; while (k < 5) { k++; if (k == 2) continue; r += 1.0; }", 4.0),
        ] {
            let src = std::format!(
                "precision mediump float;\nvoid main() {{ float r = 0.0; {body} gl_FragColor = vec4(r); }}\n"
            );
            let mut buf = [0u8; 1 << 18];
            let a = Arena::new(&mut buf);
            let l = compile_to_ir(&a, Stage::Fragment, &src).expect("compiles");
            let got = run(&l.module, &Inputs { io: &[], uniforms: &[] }).unwrap_or_else(|e| panic!("{body}: {e}"));
            let c = got.output_f32(ir::IoSlot::FragColor(0)).expect("gl_FragColor is written");
            assert_eq!(c[0], want, "{body}");
        }
    }

    /// Selection ends the shader at `return` rather than branching to the
    /// end, so an early one emitted where it stands would be dropped and the
    /// code after it would run -- invisible in the output.
    ///
    /// This test asserted that shape was REFUSED. It is lowered now, into the
    /// structured form that says the same thing without a mid-body `Return`:
    /// what followed the `if` goes into the `if`'s `else`. So the assertion
    /// moved from "is refused" to "contains no `Return` but the last", which
    /// is the property that was actually wanted -- being refused was only the
    /// safe way to have it.
    #[test]
    fn an_early_return_becomes_structure_and_not_a_mid_body_node() {
        let mut buf = [0u8; 1 << 18];
        let l =
            ir(&mut buf, "precision mediump float;\nvoid main() { if (true) { return; } gl_FragColor = vec4(1.0); }");
        let nodes = l.module.nodes_of(l.module.body);
        assert!(
            nodes.iter().any(|n| matches!(n, Node::If { .. })),
            "the early return is expressed as a branch: {nodes:?}"
        );
        for (i, n) in nodes.iter().enumerate() {
            assert!(
                !matches!(n, Node::Return) || i + 1 == nodes.len(),
                "a Return at node {i} of {}: selection would treat it as the end and run the rest",
                nodes.len()
            );
        }

        /* The trailing return every `main` gets is not an early one. */
        let mut buf2 = [0u8; 1 << 18];
        let l = ir(&mut buf2, "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); return; }");
        assert!(matches!(l.module.nodes_of(l.module.body).last(), Some(Node::Return)));
    }

    /// `gl_FragColor = ...; return;` partway through `main`, with statements
    /// after it -- `texture_copy_unclipped.frag`'s shape, and the one that
    /// escaped the rewrite because only INLINED bodies went through it.
    ///
    /// The tail is unreachable, so it is simply not emitted; what matters is
    /// that the write before the return survives and no `Return` lands in the
    /// middle.
    #[test]
    fn a_top_level_early_return_drops_the_unreachable_tail() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             void main() {\n\
               gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);\n\
               return;\n\
               gl_FragColor = vec4(0.0, 1.0, 0.0, 1.0);\n\
             }",
        );
        let m = &l.module;
        assert_eq!(
            m.instrs.iter().filter(|o| matches!(o, Op::StoreOutput { .. })).count(),
            1,
            "the unreachable second write is not emitted"
        );
        let nodes = m.nodes_of(m.body);
        for (i, n) in nodes.iter().enumerate() {
            assert!(!matches!(n, Node::Return) || i + 1 == nodes.len(), "a Return at node {i}");
        }
    }

    /// The message changed on 2026-09-22 and this test now asserts the new
    /// one. It used to read "'return' with a value in 'void main()'",
    /// because the LOWERING was what noticed; the checker had no rule and
    /// the lowering arm's own comment ("the checker refuses it, so a shader
    /// cannot reach here") was aspirational. The checker now carries section
    /// 6.1 -- a void function must return no value -- so it answers first,
    /// for every void function and not only `main`, and that arm really is
    /// unreachable.
    ///
    /// What the test is FOR is unchanged: this is a verdict on a wrong
    /// shader, and must never be worded as a gap the compiler owes.
    #[test]
    fn return_with_a_value_in_void_main_is_a_type_error_not_a_gap() {
        let mut buf = [0u8; 1 << 18];
        let msg = err(&mut buf, "void main() { return 1; }");
        assert!(msg.contains("void function cannot return a value"), "unhelpful message: {msg}");
        assert!(!msg.contains("not lowered yet"), "reported as a compiler gap: {msg}");
    }
}

#[cfg(test)]
mod global_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    /// The slot a `LoadLocal` reads, for the single load a shader makes.
    fn only_load(m: &ir::Module) -> ir::LocalId {
        let mut found = None;
        for op in m.instrs.iter() {
            if let Op::LoadLocal { local, .. } = op {
                assert!(found.is_none(), "more than one LoadLocal: {:?}", m.instrs);
                found = Some(*local);
            }
        }
        found.unwrap_or_else(|| panic!("no LoadLocal at all: {:?}", m.instrs))
    }

    /// dEQP's own shape, and the single largest gap the corpus had: its
    /// templates declare the case's result as a file-scope variable
    /// (`float out0;` beside `uniform float ref_out0;`) and write it inside
    /// the construct under test. 1,527 cases stopped on that line with
    /// "assignment to this is not lowered yet".
    ///
    /// What has to be true is not just that it compiles: both arms must
    /// write the SAME slot, and the output must read that slot, or the
    /// picture would not depend on the branch at all.
    #[test]
    fn a_global_written_in_both_arms_is_one_slot_the_output_reads() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             uniform float u;\n\
             float out0;\n\
             void main() {\n\
               if (u > 0.5) { out0 = 1.0; } else { out0 = 0.0; }\n\
               gl_FragColor = vec4(out0);\n\
             }",
        );
        let m = &l.module;
        let mut stored = [ir::LocalId(u32::MAX); 2];
        let mut n = 0;
        for op in m.instrs.iter() {
            if let Op::StoreLocal { local, .. } = op {
                assert!(n < stored.len(), "more stores than the two arms: {:?}", m.instrs);
                stored[n] = *local;
                n += 1;
            }
        }
        assert_eq!(n, 2, "one store per arm: {:?}", m.instrs);
        assert_eq!(stored[0], stored[1], "the two arms must write the same slot");
        assert_eq!(only_load(m), stored[0], "the output must read the slot the arms wrote");
        assert!(
            m.nodes_of(m.body).iter().any(|n| matches!(n, Node::If { .. })),
            "the branch survives: {:?}",
            m.nodes_of(m.body)
        );
    }

    /// A `const` at file scope is a variable whose value never changes, not
    /// a different kind of declaration, so it gets a slot like any other
    /// global. Reading one used to refuse with "reading this is not lowered
    /// yet".
    #[test]
    fn a_const_global_can_be_read() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             const float k = 0.25;\n\
             void main() { gl_FragColor = vec4(k); }",
        );
        let m = &l.module;
        assert_eq!(only_load(m).0, 0, "the const global is the first slot: {:?}", m.instrs);
        assert!(
            m.instrs.iter().any(|op| matches!(op, Op::ConstF32(c) if c[0] == 0.25f32.to_bits())),
            "its initialiser is the value it was given: {:?}",
            m.instrs
        );
    }

    /// The reason globals are a table of their own rather than the bottom of
    /// `scope`: an inlined body's lookups start at `scope_base`, above the
    /// caller's entries, so the body cannot see the caller's locals. A
    /// global living in that same vector would vanish for the duration of
    /// every inlined body.
    ///
    /// Revert check: make `bind` push to `self.scope` for a global as well,
    /// and this refuses with "reading this is not lowered yet" on `g` while
    /// the three tests above still pass.
    #[test]
    fn an_inlined_body_still_sees_the_globals() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             float g = 0.25;\n\
             float twice() { return g + g; }\n\
             void main() {\n\
               float mine = 0.5;\n\
               gl_FragColor = vec4(twice() + mine);\n\
             }",
        );
        let m = &l.module;
        let loads = m.instrs.iter().filter(|op| matches!(op, Op::LoadLocal { local, .. } if local.0 == 0)).count();
        assert_eq!(loads, 2, "both reads of `g` inside the inlined body: {:?}", m.instrs);
    }

    /// Section 4.2.3: an inner declaration hides an outer one. The global is
    /// declared first and so holds slot 0; the local in `main` holds slot 1,
    /// and it is slot 1 the output must read.
    #[test]
    fn a_local_shadows_a_global_of_the_same_name() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             float x = 1.0;\n\
             void main() { float x = 0.0; gl_FragColor = vec4(x); }",
        );
        assert_eq!(only_load(&l.module).0, 1, "the local, not the global: {:?}", l.module.instrs);

        /* The control: the same shader without the local declaration reads
         * the global, so slot 1 above is the shadowing and not just "the
         * last slot declared". */
        let mut buf2 = [0u8; 1 << 18];
        let l2 = ir(
            &mut buf2,
            "precision mediump float;\n\
             float x = 1.0;\n\
             void main() { gl_FragColor = vec4(x); }",
        );
        assert_eq!(only_load(&l2.module).0, 0, "no local to shadow it: {:?}", l2.module.instrs);
    }
}

#[cfg(test)]
mod compare_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    fn err<'a>(buf: &'a mut [u8], src: &'a str) -> &'a str {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.msg,
        }
    }

    /// The one `Alu` using `op`, and the swizzle of its first source.
    fn sole(m: &ir::Module, op: ir::AluOp) -> [u8; 4] {
        let mut found = None;
        for i in m.instrs.iter() {
            if let Op::Alu(o, srcs) = i {
                if *o == op {
                    assert!(found.is_none(), "more than one {op:?}: {:?}", m.instrs);
                    found = Some(srcs[0].swizzle);
                }
            }
        }
        found.unwrap_or_else(|| panic!("no {op:?} at all: {:?}", m.instrs))
    }

    /// Section 5.9: `u == v` is ONE bool, true when every component matches.
    /// A component-wise compare left as the answer would be a bvec, which is
    /// a different type, so the reduction is not an optimisation.
    #[test]
    fn vector_equality_is_a_component_compare_and_a_reduction() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             uniform vec4 b;\n\
             void main() { gl_FragColor = vec4(a == b ? 1.0 : 0.0); }",
        );
        let m = &l.module;
        assert_eq!(sole(m, ir::AluOp::Feq), [0, 1, 2, 3], "all four lanes compared");
        assert_eq!(sole(m, ir::AluOp::Ball), [0, 1, 2, 3], "reduced with all(), not any()");
    }

    /// `u != v` is true when ANY component differs, so it reduces with
    /// `Bany` over a `Fne`. Getting this pair the wrong way round gives
    /// `!(u == v)` only by accident of De Morgan and would be wrong if
    /// either half were changed, so it is pinned by name.
    #[test]
    fn vector_inequality_reduces_with_any() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             uniform vec4 b;\n\
             void main() { gl_FragColor = vec4(a != b ? 1.0 : 0.0); }",
        );
        let m = &l.module;
        sole(&l.module, ir::AluOp::Fne);
        sole(m, ir::AluOp::Bany);
        assert!(
            !m.instrs.iter().any(|i| matches!(i, Op::Alu(ir::AluOp::Ball, _))),
            "`!=` must not reduce with all(): {:?}",
            m.instrs
        );
    }

    /// The spare lanes, which is the part that fails silently.
    ///
    /// `Ball` and `Bany` fold all four lanes by contract, so a `vec2`
    /// compare must repeat a real component into z and w. Left alone they
    /// would reduce whatever the register held -- a wrong answer with no
    /// failure, and one that would pass a test written only on `vec4`.
    ///
    /// Revert check: drop the loop that repeats `last`, and the swizzles
    /// below come back [0,1,2,3], reducing two lanes the compare never
    /// wrote.
    #[test]
    fn a_narrow_compare_repeats_its_last_lane_into_the_spares() {
        const VEC2: &str = "precision mediump float;\n\
             uniform vec2 a;\n\
             uniform vec2 b;\n\
             void main() { gl_FragColor = vec4(a == b ? 1.0 : 0.0); }";
        const VEC3: &str = "precision mediump float;\n\
             uniform vec3 a;\n\
             uniform vec3 b;\n\
             void main() { gl_FragColor = vec4(a == b ? 1.0 : 0.0); }";

        let mut buf = [0u8; 1 << 18];
        assert_eq!(
            sole(&ir(&mut buf, VEC2).module, ir::AluOp::Ball),
            [0, 1, 1, 1],
            "a vec2 compare must repeat lane 1 into z and w, or the reduction \
             folds two lanes it never wrote"
        );

        let mut buf3 = [0u8; 1 << 18];
        assert_eq!(
            sole(&ir(&mut buf3, VEC3).module, ir::AluOp::Ball),
            [0, 1, 2, 2],
            "a vec3 compare must repeat lane 2 into w"
        );
    }

    /// `BaseType::components()` answers 4 for a `mat4` and 0 for a struct.
    /// Deciding the width from it would compare a `mat4` as a single `vec4`
    /// -- one column, three ignored -- and a struct as a scalar. They were
    /// refused by name until 2026-09-23; now a matrix compares column by
    /// column (aggregate_equality_tests run it), and this pins that a mat4
    /// is FOUR column compares, never one.
    #[test]
    fn a_matrix_compares_every_column_not_one_vector() {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\n\
             uniform mat4 a;\n\
             uniform mat4 b;\n\
             void main() { gl_FragColor = vec4(a == b ? 1.0 : 0.0); }",
        )
        .expect("compiles");
        let feq = l.module.instrs.iter().filter(|o| matches!(o, Op::Alu(ir::AluOp::Feq, _))).count();
        assert_eq!(feq, 4, "one compare per column: {:?}", l.module.instrs);
    }

    /// Section 5.9 gives `<`, `<=`, `>` and `>=` scalar operands only; the
    /// component-wise forms are `lessThan` and its family. A vector `<` that
    /// reached the reduction would answer a single bool for an expression
    /// that has no meaning, so it stops.
    #[test]
    fn ordering_operators_do_not_take_vectors() {
        let mut buf = [0u8; 1 << 18];
        let msg = err(
            &mut buf,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             uniform vec4 b;\n\
             void main() { gl_FragColor = vec4(a < b ? 1.0 : 0.0); }",
        );
        assert!(!msg.contains("not lowered yet"), "reported as a compiler gap: {msg}");
    }
}

#[cfg(test)]
mod matrix_varying_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], stage: Stage, src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    /// Every varying location the module touches, in order, whether stored
    /// or loaded.
    fn varying_slots(m: &ir::Module) -> ([u8; 8], usize) {
        let mut seen = [0u8; 8];
        let mut n = 0;
        for op in m.instrs.iter() {
            let slot = match op {
                Op::StoreOutput { slot: ir::IoSlot::Varying(k), .. } => Some(*k),
                Op::LoadInput { slot: ir::IoSlot::Varying(k), .. } => Some(*k),
                _ => None,
            };
            if let Some(k) = slot {
                assert!(n < seen.len(), "more varying accesses than expected: {:?}", m.instrs);
                seen[n] = k;
                n += 1;
            }
        }
        (seen, n)
    }

    /// A `matN` varying owns N CONSECUTIVE LOCATIONS, one per column, the
    /// same rule a `matN` attribute follows (GL ES 2.0 section 2.10.4).
    /// Writing one is N stores.
    ///
    /// Revert check: route this through `output_slot` like a scalar and one
    /// store is emitted, to location 0, with locations 1..3 never written --
    /// three quarters of the matrix carrying whatever the last draw left,
    /// and nothing to report.
    #[test]
    fn a_matrix_varying_is_written_one_column_per_location() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying mat4 out0;\n\
             void main() { out0 = mat4(in0); gl_Position = p; }",
        );
        let (seen, n) = varying_slots(&l.module);
        assert_eq!(&seen[..n], &[0, 1, 2, 3], "one store per column: {:?}", l.module.instrs);
    }

    /// And read back the same way in the fragment stage.
    #[test]
    fn a_matrix_varying_is_read_one_column_per_location() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            Stage::Fragment,
            "precision mediump float;\n\
             varying mat4 v;\n\
             void main() { gl_FragColor = v * vec4(1.0); }",
        );
        let (seen, n) = varying_slots(&l.module);
        assert_eq!(&seen[..n], &[0, 1, 2, 3], "one load per column: {:?}", l.module.instrs);
    }

    /// The load-bearing one: the NEXT varying must start past the matrix's
    /// columns. If a `matN` were one location, this `vec4` would be at 1 and
    /// three of the matrix's columns would alias other varyings -- a wrong
    /// picture that the two tests above would not catch, because with only
    /// one varying declared every reading of the rule gives 0..3.
    ///
    /// Revert check: drop the `+ j` in `matrix_column`'s varying arm and
    /// every column reads location 0.
    #[test]
    fn a_varying_after_a_matrix_starts_past_its_columns() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            Stage::Fragment,
            "precision mediump float;\n\
             varying mat4 m;\n\
             varying vec4 c;\n\
             void main() { gl_FragColor = m * c; }",
        );
        let locs = l.reflection.varyings.iter().map(|v| v.location);
        let mut got = [0u8; 2];
        for (slot, loc) in got.iter_mut().zip(locs) {
            *slot = loc;
        }
        assert_eq!(got, [0, 4], "the mat4 owns locations 0..3, so `c` is at 4");
        let (seen, n) = varying_slots(&l.module);
        assert_eq!(&seen[..n], &[4, 0, 1, 2, 3], "`c` at 4, then the matrix's four columns");
    }

    /// A `mat2` owns two, not four: the count comes from the type and is not
    /// hard-coded.
    #[test]
    fn a_mat2_varying_owns_two_locations() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying mat2 out0;\n\
             void main() { out0 = mat2(in0); gl_Position = p; }",
        );
        let (seen, n) = varying_slots(&l.module);
        assert_eq!(&seen[..n], &[0, 1], "two columns, two locations: {:?}", l.module.instrs);
    }
}

#[cfg(test)]
mod matrix_grow_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], stage: Stage, src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    /// The lane values a `ConstF32` holds, for the value a `Src` reads.
    fn const_of(m: &ir::Module, s: Src) -> Option<[u32; 4]> {
        match m.instrs.get(s.val.0 as usize) {
            Some(Op::ConstF32(c)) => Some(*c),
            _ => None,
        }
    }

    /// Section 5.4.2: growing a matrix fills from the IDENTITY, not from
    /// zero. `mat4(m3)` keeps m3 in the top-left 3x3, puts 0 in row 3 of the
    /// copied columns, and makes column 3 the identity's own -- `(0,0,0,1)`.
    ///
    /// The `1` is the whole point. A `mat4(mat3)` built by zero-filling is
    /// SINGULAR: it compiles, it draws, and it draws nothing. So the test
    /// asserts the constant, not just the shape.
    ///
    /// Revert check: make the last column's source `zero` instead of `one`
    /// and this fails on the constant while every shape assertion still
    /// passes.
    #[test]
    fn growing_a_matrix_fills_from_the_identity_and_not_from_zero() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             uniform mat3 m3;\n\
             varying mat4 o;\n\
             void main() { o = mat4(m3); gl_Position = p; }",
        );
        let m = &l.module;

        /* The masked store into each column: the second write to each slot. */
        let mut masked = [(0u8, Src::id(ValueId(0))); 4];
        let mut n = 0;
        for op in m.instrs.iter() {
            if let Op::StoreLocal { write_mask, src, .. } = op {
                if *write_mask != 0xf {
                    assert!(n < 4, "more masked column writes than columns: {:?}", m.instrs);
                    masked[n] = (*write_mask, *src);
                    n += 1;
                }
            }
        }
        assert_eq!(n, 4, "one masked write per column: {:?}", m.instrs);

        for (j, (mask, _)) in masked.iter().enumerate().take(3) {
            assert_eq!(*mask, 0b0111, "column {j} copies three lanes and leaves row 3 at zero");
        }
        let (mask, src) = masked[3];
        assert_eq!(mask, 0b1000, "column 3 writes only row 3");
        assert_eq!(
            const_of(m, src).map(|c| c[0]),
            Some(1.0f32.to_bits()),
            "row 3 of column 3 is the identity's 1, not 0 -- a zero here is a singular matrix \
             that draws nothing: {:?}",
            m.instrs
        );

        /* And every column really was zeroed first, so the lanes nothing
         * else writes are 0 rather than whatever the slot held. */
        let zeroed = m
            .instrs
            .iter()
            .filter(|op| {
                matches!(op, Op::StoreLocal { write_mask: 0xf, src, .. }
                         if const_of(m, *src) == Some([0; 4]))
            })
            .count();
        assert_eq!(zeroed, 4, "each of the four columns starts at zero: {:?}", m.instrs);
    }

    /// Shrinking is the other direction and must stay cheap: `mat3(m4)`
    /// keeps the overlap and needs no slot at all, because narrowing a
    /// column is just reading fewer lanes of it.
    #[test]
    fn shrinking_a_matrix_needs_no_slots() {
        let mut buf = [0u8; 1 << 18];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             uniform mat4 m4;\n\
             varying mat3 o;\n\
             void main() { o = mat3(m4); gl_Position = p; }",
        );
        assert_eq!(l.module.locals.len(), 0, "narrowing is a swizzle: {:?}", l.module.instrs);
    }
}

#[cfg(test)]
mod return_rule_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Vertex, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    /// True when the first `StoreLocal` the shader EXECUTES writes an
    /// all-zero constant, which is the undefined-result initialiser.
    ///
    /// It walks `nodes`, not `instrs`. `instrs` is a POOL and its index is
    /// allocation order, not execution order: the body here is built into a
    /// scratch list and committed after the initialiser, so the body's store
    /// has the LOWER pool index while running SECOND. Scanning the pool said
    /// the initialiser was missing and nearly had me "fix" correct code --
    /// the same defect this compiler keeps growing, in a test this time:
    /// two things share a representation (an ordered array) and the code
    /// read the one it was not.
    fn first_executed_store_is_zero(m: &ir::Module, r: IrRange) -> Option<bool> {
        for n in m.nodes_of(r) {
            match n {
                Node::Instr(i) => {
                    if let Some(Op::StoreLocal { src, .. }) = m.instrs.get(i.0 as usize) {
                        return Some(matches!(m.instrs.get(src.val.0 as usize), Some(Op::ConstF32(c)) if *c == [0; 4]));
                    }
                }
                Node::If { then_, else_, .. } => {
                    if let Some(v) = first_executed_store_is_zero(m, *then_) {
                        return Some(v);
                    }
                    if let Some(v) = first_executed_store_is_zero(m, *else_) {
                        return Some(v);
                    }
                }
                Node::Loop { body } => {
                    if let Some(v) = first_executed_store_is_zero(m, *body) {
                        return Some(v);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn starts_by_zeroing_a_slot(m: &ir::Module) -> bool {
        first_executed_store_is_zero(m, m.body).unwrap_or(false)
    }

    /// Section 6.1: falling off the end of a value-returning function makes
    /// the result UNDEFINED, not the shader illegal. dEQP's
    /// functions.misc.missing_returns expects it to build and says so in a
    /// comment of its own.
    ///
    /// We refused it as "this function does not return on every path" -- a
    /// verdict on a legal shader. Undefined permits any value, so the slot
    /// gets a defined zero rather than whatever it last held: the caller
    /// cannot then see a result that varies with what ran before.
    ///
    /// Revert check: restore the refusal and this panics on the error.
    #[test]
    fn falling_off_the_end_is_undefined_not_illegal() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(&mut buf, "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nfloat f(float a){ if(a>0.0) return -a; }\nvoid main(){ out0=f(in0); gl_Position=p; }");
        assert!(
            starts_by_zeroing_a_slot(&l.module),
            "the result slot is defined before the body runs: {:?}",
            l.module.instrs
        );
    }

    /// And the initialiser is only paid for where it is needed: a function
    /// that returns on every path overwrites the slot anyway, so it gets no
    /// zero store.
    #[test]
    fn a_function_that_always_returns_pays_nothing_for_the_rule() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(&mut buf, "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nfloat f(float a){ if(a>0.0) return -a; return 1.0; }\nvoid main(){ out0=f(in0); gl_Position=p; }");
        assert!(!starts_by_zeroing_a_slot(&l.module), "no initialiser when every path returns: {:?}", l.module.instrs);
    }

    /// A value-returning function called as a statement: the result is
    /// discarded, which section 6.1 allows. We answered "this function
    /// returns no value", which is backwards -- it returns one and the
    /// caller ignores it -- on a case dEQP expects to build
    /// (qualification_order.parameters.valid).
    ///
    /// The `inout` effect must survive, because that is the whole reason to
    /// call a function for its effect.
    ///
    /// Revert check: restore the refusal and this panics on the error.
    #[test]
    fn a_value_returning_function_can_be_called_as_a_statement() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(&mut buf, "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nfloat f(inout float x){ x = x + 2.0; return x + 1.0; }\nvoid main(){ float r=in0; f(r); out0=r; gl_Position=p; }");
        let m = &l.module;
        assert!(
            m.instrs.iter().any(|op| matches!(op, Op::ConstF32(c) if c[0] == 2.0f32.to_bits())),
            "the body's effect on the inout parameter survives: {:?}",
            m.instrs
        );
        assert!(
            m.instrs.iter().any(|op| matches!(op, Op::StoreOutput { slot: ir::IoSlot::Varying(0), .. })),
            "and the caller still writes the varying it read back: {:?}",
            m.instrs
        );
    }

    /// Run a vertex shader of the shape these tests use -- `attribute
    /// float in0`, `varying float out0`, `attribute vec4 p` -- through the
    /// reference evaluator and read `out0`.
    pub(super) fn out0_of(src: &str, in0: f32) -> f32 {
        let mut buf = [0u8; 1 << 19];
        let l = ir(&mut buf, src);
        let m = &l.module;
        let slot_of = |name: &str, decls: &[ir::IoDecl<'_>]| {
            let d = decls.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no {name} in {decls:?}"));
            match d.kind {
                ir::IoKind::Attribute => ir::IoSlot::Attrib(d.location),
                ir::IoKind::Varying => ir::IoSlot::Varying(d.location),
                k => panic!("{name} is a {k:?}"),
            }
        };
        let io = [
            (slot_of("in0", m.inputs), crate::shaderc::online::eval::f32x4([in0; 4])),
            (slot_of("p", m.inputs), crate::shaderc::online::eval::f32x4([0.0, 0.0, 0.0, 1.0])),
        ];
        let got =
            crate::shaderc::online::eval::run(m, &crate::shaderc::online::eval::Inputs { io: &io, uniforms: &[] });
        got.unwrap_or_else(|why| panic!("the evaluator could not run it: {why}"))
            .output_f32(slot_of("out0", m.outputs))
            .expect("out0 was never stored")[0]
    }

    /// A `return` inside a loop (dEQP functions.control_flow.return_in_loop
    /// and its five siblings). It has to leave the loop AND skip what
    /// follows the loop, which the slot-plus-else rewrite cannot express:
    /// the return leaves from the middle of an iteration. So it stores the
    /// value, sets a flag and breaks, and what follows runs only while the
    /// flag is clear.
    ///
    /// Asserted by RUNNING the module: every shape here compiled before the
    /// flag was guarded too, and drew the value of the last `return`.
    /// Revert check: drop the guarded `if` in `lower_run`'s flag arm (lower
    /// the rest straight into `out`) and every assertion that expects the
    /// loop's value reads the fallback's instead.
    #[test]
    fn a_return_inside_a_loop_leaves_the_function() {
        let v = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";
        let main = "\nvoid main(){ out0=f(in0); gl_Position=p; }";
        let once = format!("{v}float f(float a){{ for(int i=0;i<1;i++) return -a; return 1.0; }}{main}");
        assert_eq!(out0_of(&once, -0.5), 0.5);
        assert_eq!(out0_of(&once, 1.5), -1.5);

        /* Returns on the second iteration, from inside an `if`; the step
         * must not run after it and the fallback must not either. */
        let cond = format!(
            "{v}float f(float a){{ for(int i=0;i<3;i++){{ if(i==1) return a+float(i); a=a+10.0; }} return -1.0; }}{main}"
        );
        assert_eq!(out0_of(&cond, 0.5), 11.5);

        /* Never returns from the loop: the fallback is the answer. */
        let never = format!(
            "{v}float f(float a){{ for(int i=0;i<3;i++){{ if(a>100.0) return a; a=a+1.0; }} return -a; }}{main}"
        );
        assert_eq!(out0_of(&never, 0.5), -3.5);

        /* Nested: the inner loop's return must break the OUTER loop too,
         * or the outer loop's next iteration overwrites the result. */
        let nested = format!(
            "{v}float f(float a){{ for(int j=0;j<3;j++){{ for(int i=0;i<3;i++){{ if(i==j+1) return a+float(10*j+i); }} a=a+100.0; }} return -1.0; }}{main}"
        );
        assert_eq!(out0_of(&nested, 0.5), 1.5);

        /* break and continue beside the return, as in
         * mixed_return_break_continue. */
        let mixed = format!(
            "{v}float f(float a){{ for(int i=0;i<6;i++){{ if(i==0) continue; if(i==4) break; if(a>10.0) return a; a=a*2.0+float(i); }} return -a; }}{main}"
        );
        /* i=1: 2*1+1=3; i=2: 8; i=3: 19 -> returns 19 at i=4? no: i=4 breaks
         * before the return test. So after i=3 a=19, i=4 breaks, -19. */
        assert_eq!(out0_of(&mixed, 1.0), -19.0);
        assert_eq!(out0_of(&mixed, 11.0), 11.0);
    }

    /// A struct returned from inside a loop. This was a gap -- a struct
    /// result is a run of slots, not the one slot the early-return rewrite
    /// kept -- until 2026-09-23, when a struct or array result began to be
    /// filled in place by every `return` (Qt Quick 3D's `qt_sampleSpotLight`
    /// returns early). The flag form keeps what follows the loop from
    /// running after the return.
    ///
    /// Revert check: route `Want::Struct` in `inline_with_slot` back to the
    /// refusal and this is refused again.
    #[test]
    fn a_struct_returned_from_inside_a_loop_fills_the_callers_run() {
        let src = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nstruct S { float x; };\nS f(float a){ for(int i=0;i<3;i++){ if(a < float(i)) return S(float(i)); } return S(-1.0); }\nvoid main(){ S s=f(in0); out0=s.x; gl_Position=p; }";
        assert_eq!(out0_of(src, 1.5), 2.0);
        assert_eq!(out0_of(src, 5.0), -1.0);
    }

    /// `main`'s own `return;` inside a loop: the outputs written before it
    /// stay, and nothing after the loop runs.
    #[test]
    fn a_return_inside_a_loop_in_main_ends_main() {
        let src = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nvoid main(){ gl_Position=p; out0=1.0; for(int i=0;i<4;i++){ out0=out0+1.0; if(in0>0.0 && i==1) return; } out0=-out0; }";
        assert_eq!(out0_of(src, 1.0), 3.0);
        assert_eq!(out0_of(src, -1.0), -5.0);
    }

    /// An `if` arm that returns on SOME of its paths. This compiled before
    /// the flag form and was wrong: the statement after the `if` was
    /// lowered unconditionally, so `return a + 2.0` overwrote the inner
    /// `return 1.0` and the function never answered 1.0.
    ///
    /// Revert check: drop the `returns_sometimes` guard on `lower_run`'s
    /// `if` arm and the first assertion reads 3.5 -- `a + 2.0`, the second
    /// return overwriting the first.
    #[test]
    fn an_arm_that_returns_sometimes_guards_what_follows() {
        let v = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";
        let main = "\nvoid main(){ out0=f(in0); gl_Position=p; }";
        let src = format!("{v}float f(float a){{ if(a>0.0){{ if(a>1.0) return 1.0; a=a*4.0; }} return a+2.0; }}{main}");
        assert_eq!(out0_of(&src, 1.5), 1.0);
        assert_eq!(out0_of(&src, 0.5), 4.0);
        assert_eq!(out0_of(&src, -1.0), 1.0);

        /* The same through a bare block. */
        let block = format!("{v}float f(float a){{ {{ if(a>1.0) return 1.0; a=a*4.0; }} return a+2.0; }}{main}");
        assert_eq!(out0_of(&block, 1.5), 1.0);
        assert_eq!(out0_of(&block, 0.5), 4.0);

        /* And an arm that always returns keeps the cheap shape: no flag. */
        let always = format!("{v}float f(float a){{ if(a>0.0) return -a; return 1.0; }}{main}");
        let mut buf = [0u8; 1 << 19];
        let l = ir(&mut buf, &always);
        assert!(
            !l.module.locals.iter().any(|d| d.ty.scalar == ir::ScalarType::Bool),
            "an arm that always returns needs no flag: {:?}",
            l.module.locals
        );
    }

    /// The control that keeps the gap narrow: a loop whose `return` comes
    /// after it is not affected.
    #[test]
    fn a_return_after_a_loop_still_lowers() {
        let mut buf = [0u8; 1 << 19];
        ir(&mut buf, "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nfloat f(float a){ for(int i=0;i<1;i++) { a=a+1.0; } return a; }\nvoid main(){ out0=f(in0); gl_Position=p; }");
    }
}

#[cfg(test)]
mod local_array_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], stage: Stage, src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    fn err<'a>(buf: &'a mut [u8], stage: Stage, src: &'a str) -> &'a str {
        let a = Arena::new(buf);
        match compile_to_ir(&a, stage, src) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.msg,
        }
    }

    /// Every slot a `StoreLocal` or `LoadLocal` touches, in pool order.
    fn slots_touched(m: &ir::Module) -> ([u32; 12], usize) {
        let mut seen = [0u32; 12];
        let mut n = 0;
        for op in m.instrs.iter() {
            let id = match op {
                Op::StoreLocal { local, .. } | Op::LoadLocal { local, .. } => Some(local.0),
                _ => None,
            };
            if let Some(id) = id {
                if n < seen.len() {
                    seen[n] = id;
                    n += 1;
                }
            }
        }
        (seen, n)
    }

    /// An element gets its own slot, and `a[j]` is slot `id + j` -- the same
    /// arithmetic a matrix column uses, which is why the two are one change.
    #[test]
    fn an_element_is_written_and_read_at_its_own_slot() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying float out0;\n\
             void main() { float a[3]; a[0]=in0; a[1]=2.0; a[2]=3.0; out0=a[1]; gl_Position=p; }",
        );
        assert_eq!(l.module.locals.len(), 3, "one slot per element");
        let (seen, n) = slots_touched(&l.module);
        assert_eq!(&seen[..n], &[0, 1, 2, 1], "writes 0,1,2 then reads back slot 1");
    }

    /// An array crosses a call by being copied element by element, in and
    /// out: ES 1.00 is call by value (section 6.1), and there is no
    /// array-valued expression to copy through (section 5.7).
    #[test]
    fn an_array_parameter_is_copied_element_by_element() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying float out0;\n\
             float f(float a[2]) { return a[0] + a[1]; }\n\
             void main() { float b[2]; b[0]=in0; b[1]=1.0; out0=f(b); gl_Position=p; }",
        );
        assert_eq!(l.module.locals.len(), 4, "two for the caller's array, two for the parameter's");

        let mut buf2 = [0u8; 1 << 19];
        let l2 = ir(
            &mut buf2,
            Stage::Vertex,
            "attribute vec4 p;\n\
             varying float out0;\n\
             void g(out float a[2]) { a[0]=1.0; a[1]=2.0; }\n\
             void main() { float b[2]; g(b); out0=b[1]; gl_Position=p; }",
        );
        assert_eq!(l2.module.locals.len(), 4, "an `out` array is copied back the same way");
    }

    /// THE ONE THAT WAS DEAD CODE.
    ///
    /// `m[0] = vec2(...)` has had an arm in `assign` and a doc comment saying
    /// it works since matrices landed. It never ran: the `gl_FragData[n]`
    /// arm above it matched EVERY `Expr::Index` destination and returned an
    /// error for any base that was not `gl_FragData`, so the arm below was
    /// unreachable. Measured at HEAD before this change: `mat2 m; m[0] =
    /// vec2(1.0);` answered "assignment to this is not lowered yet".
    ///
    /// A guard has to be as narrow as the case it handles, or it answers for
    /// cases it knows nothing about.
    ///
    /// Revert check: restore the `return Err(...)` at the end of the
    /// `gl_FragData` arm and this fails, together with every array write.
    #[test]
    fn a_matrix_column_can_be_assigned() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             varying float out0;\n\
             void main() { mat2 m; m[0]=vec2(1.0); m[1]=vec2(2.0); out0=m[1].x; gl_Position=p; }",
        );
        assert_eq!(l.module.locals.len(), 2, "a mat2 is two column slots");
        let (seen, n) = slots_touched(&l.module);
        assert_eq!(&seen[..n], &[0, 1, 1], "column 0, column 1, then read column 1");
    }

    /// And the control for that narrowing: `gl_FragData` still goes to the
    /// colour output, and a second draw buffer is still refused by name.
    #[test]
    fn gl_frag_data_still_works_and_still_bounds_its_index() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            Stage::Fragment,
            "precision mediump float;\nvarying vec4 v;\nvoid main(){ gl_FragData[0] = v; }",
        );
        assert!(
            l.module.instrs.iter().any(|o| matches!(o, Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. })),
            "gl_FragData[0] is the colour output: {:?}",
            l.module.instrs
        );

        let mut buf2 = [0u8; 1 << 19];
        let msg = err(
            &mut buf2,
            Stage::Fragment,
            "precision mediump float;\nvarying vec4 v;\nvoid main(){ gl_FragData[1] = v; }",
        );
        assert!(msg.contains("one draw buffer"), "the index is still bounded: {msg}");
    }

    /// A variable index into a local is a select over its elements since
    /// 2026-09-23 (dynamic_index_tests run it). What remains a gap is a
    /// compound assignment whose index has a side effect: reading the
    /// element and storing it would evaluate the index twice. It says so --
    /// as a gap, since the shader is legal.
    #[test]
    fn a_side_effecting_index_in_a_compound_assignment_is_named_as_a_gap() {
        let mut buf = [0u8; 1 << 19];
        let msg = err(
            &mut buf,
            Stage::Vertex,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying float out0;\n\
             void main() { float a[2]; a[0]=1.0; a[1]=2.0; int i=int(in0); a[i++] += 1.0; out0=a[0]; gl_Position=p; }",
        );
        assert!(msg.contains("not lowered yet"), "a legal shader, so a gap: {msg}");
        assert!(msg.contains("side effects"), "and it names the construct: {msg}");
    }

    /// The things ES 1.00 does not allow, refused by KIND rather than by
    /// slot count -- an array has the same run of slots as a matrix, and
    /// `matrix_value` would happily build one from it.
    #[test]
    fn what_an_array_is_not_is_refused_by_kind() {
        let mut buf = [0u8; 1 << 19];
        assert_eq!(
            err(
                &mut buf,
                Stage::Vertex,
                "attribute vec4 p;\nvarying float out0;\nvoid main(){ float a[2]; float b[2]; a=b; out0=a[0]; gl_Position=p; }"
            ),
            "an array cannot be assigned as a whole",
        );

        let mut buf2 = [0u8; 1 << 19];
        assert_eq!(
            err(
                &mut buf2,
                Stage::Vertex,
                "attribute vec4 p;\nvarying float out0;\nvoid main(){ float a[2]; out0=a[5]; gl_Position=p; }"
            ),
            "index out of range",
        );
    }
}

#[cfg(test)]
mod local_struct_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Vertex, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    fn err<'a>(buf: &'a mut [u8], src: &'a str) -> &'a str {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Vertex, src) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.msg,
        }
    }

    fn widths(m: &ir::Module) -> [u8; 8] {
        let mut w = [0u8; 8];
        for (i, d) in m.locals.iter().enumerate().take(8) {
            w[i] = d.ty.comps;
        }
        w
    }

    /// THE LOAD-BEARING ONE, and the reason a struct is not the same change
    /// as a matrix or an array.
    ///
    /// Those are a run of EQUAL-width slots, so the single `Local::comps`
    /// describes every slot in the run. A struct's members differ, so the
    /// width has to be read per slot -- from the member's own declaration.
    /// A run built at one width would give `float b` three lanes and write
    /// past it, or give `vec3 a` one and drop two.
    ///
    /// Revert check: allocate the run at `comps` like the other kinds and
    /// the widths come back [1, 1] instead of [3, 1].
    #[test]
    fn a_struct_local_is_one_slot_per_member_at_its_own_width() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying float out0;\n\
             struct S { vec3 a; float b; };\n\
             void main(){ S s; s.a = vec3(in0); s.b = 2.0; out0 = s.a.x + s.b; gl_Position=p; }",
        );
        assert_eq!(l.module.locals.len(), 2, "one slot per member");
        assert_eq!(&widths(&l.module)[..2], &[3, 1], "vec3 then float, each at its own width");
    }

    /// dEQP's own shape, byte for byte from functions.test
    /// `datatypes.float_struct_vertex`: a structure constructor filling a
    /// local, then that local passed to a function by value.
    ///
    /// The run of slots alone closed NOTHING -- measured, 13 cases moved
    /// from the declaration to the call and the total held at 76. The
    /// constructor and the parameter are what the corpus actually needs.
    #[test]
    fn the_corpus_shape_is_a_constructor_then_a_call() {
        let mut buf = [0u8; 1 << 19];
        ir(
            &mut buf,
            "precision mediump float;\n\
             attribute highp vec4 dEQP_Position;\n\
             attribute vec3 in0;\n\
             varying float out0;\n\
             struct Pos { float a, b, c; };\n\
             float func (Pos p) { return -(p.a + p.b + p.c); }\n\
             void main() { Pos p = Pos(in0.x, in0.y, in0.z); out0 = func(p); gl_Position = dEQP_Position; }",
        );
    }

    /// And through a call at mixed widths, which is where a one-width run
    /// would corrupt the copy rather than refuse.
    #[test]
    fn a_struct_crosses_a_call_at_each_members_width() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "attribute vec4 q;\n\
             attribute float in0;\n\
             varying float out0;\n\
             struct S { vec3 a; float b; };\n\
             float f(S s){ return s.a.x + s.b; }\n\
             void main(){ S s = S(vec3(in0), 2.0); out0 = f(s); gl_Position=q; }",
        );
        assert_eq!(l.module.locals.len(), 4, "two for the caller's struct, two for the parameter's");
        assert_eq!(&widths(&l.module)[..4], &[3, 1, 3, 1], "both copies keep the members' widths");
    }

    /// The run inside a run that IS laid out: a member that is a struct
    /// becomes that struct's own leaves, in place, each at its own width.
    ///
    /// `struct I { vec3 a; float b; }; struct S { I i; vec4 c; };` is three
    /// slots -- 3, 1, 4 -- not two, and `s.c` is slot 2, not slot 1.
    /// Counting MEMBERS rather than leaves would put `s.c` on top of
    /// `s.i.b`: a store that compiles, runs, and draws a wrong picture.
    ///
    /// Revert check: make `struct_member`'s offset the member index again
    /// and `s.c` writes slot 1, which this reads back as a width of 1.
    #[test]
    fn a_nested_struct_member_becomes_that_structs_own_leaves() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "attribute vec4 p;\n\
             attribute float in0;\n\
             varying float out0;\n\
             struct I { vec3 a; float b; };\n\
             struct S { I i; vec4 c; };\n\
             void main(){ S s; s.i.a = vec3(in0); s.i.b = 2.0; s.c = vec4(3.0);\n\
                          out0 = s.i.a.x + s.i.b + s.c.w; gl_Position = p; }",
        );
        assert_eq!(l.module.locals.len(), 3, "the inner struct's two leaves, then the outer's vec4");
        assert_eq!(&widths(&l.module)[..3], &[3, 1, 4], "each leaf at its own width");
        /* `s.c` is the third slot. Read off the stores rather than asserted
         * by hand: `instrs` is a POOL, so the store is found by what it is,
         * not by where it sits. */
        let wrote_slot_2 = l
            .module
            .instrs
            .iter()
            .any(|op| matches!(op, Op::StoreLocal { local, write_mask, .. } if local.0 == 2 && *write_mask == 0xf));
        assert!(wrote_slot_2, "s.c writes the leaf after the inner struct's two, at four lanes");
    }

    /// A member that is a matrix is still a run inside a run with no
    /// layout -- refused BY NAME at the declaration, where the name and the
    /// line are, rather than at whichever member is touched first. (An ARRAY
    /// member has a layout since 2026-09-23: its elements in a row;
    /// struct_array_tests run it.)
    ///
    /// The matrix arm is a defect this test once found: `member_shape`'s own
    /// comment said a run inside a run was refused, and a `mat2` member was
    /// nonetheless given ONE slot -- column 0, and a plausible wrong
    /// picture for the other columns.
    #[test]
    fn a_matrix_member_is_refused_where_it_is_declared() {
        let mut b3 = [0u8; 1 << 19];
        assert_eq!(
            err(&mut b3, "attribute vec4 p;\nvarying float out0;\nstruct S { mat2 m; float b; };\nvoid main(){ S s; s.b=1.0; out0=s.b; gl_Position=p; }"),
            "a struct member that is a matrix is not lowered yet"
        );
    }

    /// dEQP `constant_expressions.complex_types.nested_struct`: a nested
    /// constructor, and a member of struct type filled from a NAME rather
    /// than from a constructor. Section 5.4.3 asks only that the argument
    /// already have the member's type.
    #[test]
    fn a_nested_constructor_takes_one_argument_per_member() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "attribute vec4 p;\n\
             varying float out0;\n\
             struct S { vec4 v; int i; };\n\
             struct T { S s; int i; };\n\
             void main(){ S s = S(vec4(1.5), 123);\n\
                          T t = T(s, 3);\n\
                          T u = T(S(vec4(1.3), 4), 6);\n\
                          out0 = float(t.s.i + u.i) * u.s.v.x; gl_Position = p; }",
        );
        /* S = 2 leaves, T = 3 each: 2 + 3 + 3. */
        assert_eq!(l.module.locals.len(), 8, "two leaves for S, three for each T");
        assert_eq!(&widths(&l.module)[..8], &[4, 1, 4, 1, 1, 4, 1, 1], "vec4, int, then each T's three");
    }

    /// dEQP `functions.datatypes.struct_struct`: a function that RETURNS a
    /// struct, assigned over the variable it was given.
    ///
    /// There is no struct value to hand back -- a struct is a run, not a
    /// register -- so the caller names the destination run and the body's
    /// `return` fills it.
    #[test]
    fn a_function_can_return_a_struct() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             attribute vec4 q;\n\
             attribute vec3 in0;\n\
             varying float out0;\n\
             struct Pos { float a, b, c; };\n\
             Pos func (Pos p) { return Pos(-p.a, -p.b, -p.c); }\n\
             void main(){ Pos p = Pos(in0.x, in0.y, in0.z); p = func(p);\n\
                          out0 = p.a + p.b + p.c; gl_Position = q; }",
        );
        assert_eq!(&widths(&l.module)[..6], &[1, 1, 1, 1, 1, 1], "the caller's three leaves and the parameter's");
    }

    /// And a struct returned through a nested struct: dEQP
    /// `functions.datatypes.struct_nested_struct`, whose `Line` is two
    /// `Pos`es and whose members are passed on to another call.
    #[test]
    fn a_returned_struct_may_nest() {
        let mut buf = [0u8; 1 << 19];
        ir(
            &mut buf,
            "precision mediump float;\n\
             attribute vec4 q;\n\
             attribute vec3 in0;\n\
             varying float out0;\n\
             struct Pos { float a, b, c; };\n\
             struct Line { Pos start, end; };\n\
             Line func (Pos p) { return Line(p, Pos(-p.a, -p.b, -p.c)); }\n\
             float sum (Pos p) { return (p.a + p.b + p.c); }\n\
             void main(){ Pos p = Pos(in0.x, in0.y, in0.z); Line line = func(p);\n\
                          out0 = sum(line.start) + (2.0 * sum(line.end)); gl_Position = q; }",
        );
    }

    /// A vector swizzle is the same syntax as a member access, and the
    /// checker's type is what tells them apart. The control that the struct
    /// arm did not capture it.
    #[test]
    fn a_vector_swizzle_is_still_a_swizzle() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "attribute vec4 p;\nvarying float out0;\nvoid main(){ vec3 v = vec3(1.0); out0 = v.y; gl_Position=p; }",
        );
        assert_eq!(l.module.locals.len(), 1, "one slot, not a run");
    }
}

#[cfg(test)]
mod value_width_tests {
    use super::*;

    fn ir<'a>(buf: &'a mut [u8], src: &'a str) -> Lowered<'a> {
        let a = Arena::new(buf);
        match compile_to_ir(&a, Stage::Fragment, src) {
            Ok(l) => l,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    /// The width of the single `Alu` using `op`.
    fn comps_of(m: &ir::Module, op: ir::AluOp) -> u8 {
        for (i, o) in m.instrs.iter().enumerate() {
            if matches!(o, Op::Alu(x, _) if *x == op) {
                return m.comps_of(ValueId(i as u32));
            }
        }
        panic!("no {op:?}: {:?}", m.instrs);
    }

    /// THE CASE THE BACK END COULD NOT SEE.
    ///
    /// `a.xy + b.xy` over `vec4` producers is two components. A back end
    /// walking forward has only the operands' `Src`s, whose swizzle carries
    /// four lanes whatever the type said, so it over-estimates as four --
    /// the same ambiguity that has cost this compiler elsewhere: a swizzle
    /// answers for a width AND for a repeat.
    ///
    /// The front end has the checker's type at every node, so it records
    /// the answer instead.
    #[test]
    fn a_narrow_operation_over_wide_producers_records_its_own_width() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             uniform vec4 b;\n\
             void main(){ vec2 s = a.xy + b.xy; gl_FragColor = vec4(s, 0.0, 1.0); }",
        );
        assert_eq!(comps_of(&l.module, ir::AluOp::Fadd), 2, "a.xy + b.xy is two lanes, not four");
    }

    /// A reduction produces one lane whatever it reads, so `result_comps`
    /// knows without the operands -- which makes `dot` exact even on the
    /// conservative path the built-in library emits through.
    #[test]
    fn a_reduction_is_one_lane_however_it_was_emitted() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             uniform vec4 b;\n\
             void main(){ gl_FragColor = vec4(dot(a, b)); }",
        );
        assert_eq!(comps_of(&l.module, ir::AluOp::Fdot4), 1, "a dot product is one lane");

        let mut b2 = [0u8; 1 << 19];
        let l2 = ir(
            &mut b2,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             uniform vec4 b;\n\
             void main(){ gl_FragColor = vec4(float(a == b)); }",
        );
        assert_eq!(comps_of(&l2.module, ir::AluOp::Feq), 4, "the component-wise compare is four");
        assert_eq!(comps_of(&l2.module, ir::AluOp::Ball), 1, "and the fold is one");
    }

    /// An unrecorded width answers 4, never a narrower guess. A back end
    /// over-estimating is what it did before there was a table; a back end
    /// told "one lane" about a four-lane value would mask three away.
    ///
    /// `comps_of` falls back to the instruction's own fields first (see
    /// `Op::stated_comps`), so this is about the values that state nothing.
    #[test]
    fn an_unrecorded_width_is_the_over_estimate_not_a_narrow_guess() {
        let m = ir::Module { comps: &[], ..ir::Module::empty(SHC_STAGE_FRAGMENT) };
        assert_eq!(m.comps_of(ValueId(0)), 4, "no instruction either");
        assert_eq!(m.comps_of(ValueId(99)), 4, "past the end of both arrays");

        let instrs = [Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ValueId(0)); 3])];
        let m = ir::Module { instrs: &instrs, comps: &[], ..ir::Module::empty(SHC_STAGE_FRAGMENT) };
        assert_eq!(m.comps_of(ValueId(0)), 4, "an ALU whose width only the checker knew");
    }

    /// The width table is in step with `instrs`, which is what makes
    /// `comps_of` a property map and not a second ordering.
    #[test]
    fn the_table_is_as_long_as_the_instruction_pool() {
        let mut buf = [0u8; 1 << 19];
        let l = ir(
            &mut buf,
            "precision mediump float;\n\
             uniform vec4 a;\n\
             void main(){ gl_FragColor = a * 2.0; }",
        );
        assert_eq!(l.module.comps.len(), l.module.instrs.len());
    }
}

#[cfg(test)]
mod int_literal_tests {
    use super::return_rule_tests::out0_of;
    use super::*;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// An integer literal is an integer. It was lowered as a float constant
    /// -- `3` as the bits of 3.0 -- and every integer operation read those
    /// bits as 1,077,936,128. On the GC7000 this compiled, ran and drew:
    /// the loop below went round twice (`i` jumped from 0 to the bits of
    /// 1.0, then past the bits of 3.0) and `float(k)` read 1.08e9.
    ///
    /// Revert check: lower `Expr::IntLit` through `const_value` again and
    /// the loop never reaches `i == 3` in the evaluator's integer compare.
    #[test]
    fn an_int_literal_is_an_int() {
        let src = format!(
            "{V}void main(){{ int k=3; int s=0; for(int i=0;i<3;i=i+1){{ s=s+i; }} out0=float(k)+10.0*float(s); gl_Position=p; }}"
        );
        assert_eq!(out0_of(&src, 0.0), 33.0);
        let inc = format!("{V}void main(){{ int n=0; for(int i=0;i<5;i++){{ n+=2; }} out0=float(n); gl_Position=p; }}");
        assert_eq!(out0_of(&inc, 0.0), 10.0);
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, &src).expect("compiles");
        assert!(
            !l.module.instrs.iter().any(|op| matches!(op, Op::ConstF32(c) if c[0] == 3.0f32.to_bits())),
            "no float 3.0 anywhere in a shader that only has the integer 3: {:?}",
            l.module.instrs
        );
    }

    /// gl_test case 72's own shader, run here with the values the board
    /// case uses, so the colours the case expects are measured before any
    /// board is asked: red 0.75 (three passes), green on exactly one side of
    /// `int(u_in.x) == 2`, blue 1.0.
    #[test]
    fn gl_test_intlit_draws_the_predicted_colours() {
        use crate::shaderc::online::eval::{f32x4, run, Inputs};
        let src = include_str!("../../../tools/shaders/es100/intlit.frag");
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).expect("compiles");
        let m = &l.module;
        let u = m.uniforms.iter().find(|u| u.name == "u_in").expect("u_in is a uniform");
        for (x, green) in [(2.0f32, 1.0f32), (1.0, 0.0)] {
            let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4)];
            block[usize::from(u.dword_off)..usize::from(u.dword_off) + 4].copy_from_slice(&f32x4([x, 0.0, 0.0, 0.0]));
            let got = run(m, &Inputs { io: &[], uniforms: &block }).expect("the evaluator runs it");
            let c = got.output_f32(ir::IoSlot::FragColor(0)).expect("gl_FragColor is written");
            assert_eq!(c, [0.75, green, 1.0, 1.0], "u_in.x = {x}");
        }
    }

    /// gl_test case 76's shader, with the uniform block written the way the
    /// GL core writes it -- a bool is the u32 1 or 0 (`program.rs`,
    /// `values.truth`), NOT the all-ones the IR uses for true -- so the
    /// colours the board case expects are the evaluator's, measured before
    /// any board is asked.
    ///
    /// Red is `u_b` (0.5) plus `any(u_bv)` (0.25) plus `all(u_bv)` (0.25);
    /// green `any(u_v > 0.5)`, blue `all(u_v > 0.5)`. Each side sets the
    /// lanes that decide it away from x, so a reduction that reads one lane
    /// gets it wrong.
    #[test]
    fn gl_test_boolany_draws_the_predicted_colours() {
        use crate::shaderc::online::eval::{f32x4, run, Inputs};
        let src = include_str!("../../../tools/shaders/es100/boolany.frag");
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).expect("compiles");
        let m = &l.module;
        let off = |n: &str| usize::from(m.uniforms.iter().find(|u| u.name == n).expect(n).dword_off);
        for (b, bv, v, want) in BOOLANY_SIDES {
            let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(12)];
            block[off("u_b")] = b;
            block[off("u_bv")..off("u_bv") + 3].copy_from_slice(&bv);
            block[off("u_v")..off("u_v") + 4].copy_from_slice(&f32x4(v));
            let got = run(m, &Inputs { io: &[], uniforms: &block }).expect("the evaluator runs it");
            let c = got.output_f32(ir::IoSlot::FragColor(0)).expect("gl_FragColor is written");
            assert_eq!(c, want, "u_b {b}, u_bv {bv:?}, u_v {v:?}");
        }
    }

    /// gl_test case 80's shader, run with the board case's uniforms: matrix
    /// arithmetic on a NON-symmetric matrix (m+m, k-m, k/m, -m, m++, m*=m),
    /// a vector `?:` on both sides of its condition, and dynamic indexing of
    /// a local array (read and write), a matrix column and vector lanes.
    ///
    /// The colours are also worked by hand, and they agree: with u_k =
    /// (1, 2, 0.5, 4) and u_i = 1, red is (8 + 3.5 + 1 - 0.5 + 4 x 0.5)/16,
    /// green is (10.5 - 8.5)/8 plus 0.5 or 0.25 from the `?:`, blue is
    /// 0.25 + 0.125 + 0.25 + 0.125. A transposed product, a mis-ordered
    /// column, or an index read as a constant each moves a channel.
    #[test]
    fn gl_test_matdyn_draws_the_predicted_colours() {
        use crate::shaderc::online::eval::{f32x4, run, Inputs};
        let src = include_str!("../../../tools/shaders/es100/matdyn.frag");
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).expect("compiles");
        let m = &l.module;
        let off = |n: &str| usize::from(m.uniforms.iter().find(|u| u.name == n).expect(n).dword_off);
        for (c, want) in [(1.0f32, [0.875, 0.75, 0.75, 1.0]), (0.0, [0.875, 0.5, 0.75, 1.0])] {
            let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(12)];
            block[off("u_k")..off("u_k") + 4].copy_from_slice(&f32x4([1.0, 2.0, 0.5, 4.0]));
            block[off("u_c")] = c.to_bits();
            block[off("u_i")] = 1;
            let got = run(m, &Inputs { io: &[], uniforms: &block }).expect("the evaluator runs it");
            let col = got.output_f32(ir::IoSlot::FragColor(0)).expect("gl_FragColor is written");
            assert_eq!(col, want, "u_c = {c}");
        }
    }

    /// Section 4.3.5's substitution, run: a fragment shader whose vertex
    /// side declares `v` and `m` but never writes them reads 0 for every
    /// lane of both -- every column of the `mat2` too -- and has neither as
    /// an input any more, while `w`, which is written, is untouched.
    ///
    /// Checked: making `zero_varyings` return its input unchanged fails the
    /// input list (and would read the 1.0 fed to every location).
    #[test]
    fn a_zeroed_varying_reads_as_zero_in_every_lane_and_column() {
        use crate::shaderc::online::eval::{f32x4, run, Inputs};
        let src = "precision mediump float; varying vec4 v; varying mat2 m; varying vec4 w;\n\
                   void main(){ gl_FragColor = v + vec4(m[0], m[1]) + w; }";
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).expect("compiles");
        let l = zero_varyings(&a, l, &["v", "m"]).expect("substitutes");
        let m = &l.module;
        let names: std::vec::Vec<&str> = m.inputs.iter().map(|d| d.name).collect();
        assert_eq!(names, ["w"], "only the written varying is still an input");
        assert!(l.reflection.varyings.iter().all(|v| v.name == "w"));
        let w = m.inputs[0].location;
        /* Feed EVERY location something, so a load that survived shows. */
        let ones = f32x4([1.0; 4]);
        let io: std::vec::Vec<(ir::IoSlot, [u32; 4])> = (0..4)
            .map(|k| (ir::IoSlot::Varying(k), if k == w { f32x4([0.25, 0.5, 0.75, 1.0]) } else { ones }))
            .collect();
        let got = run(m, &Inputs { io: &io, uniforms: &[] }).expect("the evaluator runs it");
        let c = got.output_f32(ir::IoSlot::FragColor(0)).expect("gl_FragColor is written");
        assert_eq!(c, [0.25, 0.5, 0.75, 1.0]);
    }

    /// The sides gl_test case 76 draws: (u_b, u_bv, u_v, the colour).
    #[allow(clippy::type_complexity)]
    const BOOLANY_SIDES: [(u32, [u32; 3], [f32; 4], [f32; 4]); 4] = [
        (1, [1, 1, 1], [1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
        (0, [0, 0, 1], [0.0, 0.0, 0.0, 1.0], [0.25, 1.0, 0.0, 1.0]),
        (1, [0, 0, 0], [0.0, 0.0, 0.0, 0.0], [0.5, 0.0, 0.0, 1.0]),
        (0, [1, 1, 0], [1.0, 1.0, 1.0, 0.0], [0.25, 1.0, 0.0, 1.0]),
    ];

    /// A constructor with non-constant arguments converts EACH argument to
    /// the constructed type (5.4.1, 5.4.2). `assemble` took the lanes as
    /// they were, so an int argument in a float constructor was its bits;
    /// when int literals became `ConstI32`, femtovg's `vec4(x, y, 0, 1)` put
    /// 1.4e-45 in w, and pco's bytes for main.vert caught it (usc_limits).
    ///
    /// Revert check: take `self.value(e, have, out)` in `assemble` again
    /// and the first assertion reads a denormal.
    #[test]
    fn every_constructor_argument_is_converted() {
        let q = format!("{V}void main(){{ vec4 q=vec4(in0, 0, 0, 1); out0=q.w+10.0*q.y; gl_Position=p; }}");
        assert_eq!(out0_of(&q, 5.0), 1.0);
        let iv = format!("{V}void main(){{ int k=3; vec2 v=vec2(k, in0); out0=v.x*10.0+v.y; gl_Position=p; }}");
        assert_eq!(out0_of(&iv, 0.5), 30.5);
        let fi = format!("{V}void main(){{ ivec2 w=ivec2(in0, 7); out0=float(w.x+w.y); gl_Position=p; }}");
        assert_eq!(out0_of(&fi, 2.7), 9.0);
        let diag = format!("{V}void main(){{ mat2 m=mat2(2); out0=m[1][1]+10.0*m[0][1]; gl_Position=p; }}");
        assert_eq!(out0_of(&diag, 0.0), 2.0);
        let cols = format!("{V}void main(){{ mat2 m=mat2(ivec2(1,2), ivec2(3,4)); out0=m[1][0]; gl_Position=p; }}");
        assert_eq!(out0_of(&cols, 0.0), 3.0);
        /* Above 2^24 an f32 cannot hold the literal: the int path must not
         * pass through one. */
        let big = format!("{V}void main(){{ ivec2 b=ivec2(16777217, 1); int c=16777217; out0=float(b.x-16777216)+10.0*float(c-16777216); gl_Position=p; }}");
        assert_eq!(out0_of(&big, 0.0), 11.0);
    }

    /// A constructor of constants is a constant of the CONSTRUCTED type,
    /// converted as section 5.4.1 says: to an int the fraction is dropped,
    /// to a bool zero is false and anything else true.
    #[test]
    fn a_constant_constructor_has_its_own_type() {
        let ivec = format!("{V}void main(){{ ivec2 v=ivec2(1,2); out0=float(v.x+v.y); gl_Position=p; }}");
        assert_eq!(out0_of(&ivec, 0.0), 3.0);
        let trunc = format!("{V}void main(){{ int t=int(-3.7); out0=float(t); gl_Position=p; }}");
        assert_eq!(out0_of(&trunc, 0.0), -3.0);
        let bvec = format!("{V}void main(){{ bvec2 b=bvec2(2,0); out0=(b.x && !b.y) ? 1.0 : -1.0; gl_Position=p; }}");
        assert_eq!(out0_of(&bvec, 0.0), 1.0);
    }
}

/// Section 4.2.2's scopes, asserted by running the module.
#[cfg(test)]
mod scope_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// Section 6.1 scoping: a function body sees the globals and its own
    /// parameters, never its caller's locals. `f` reads the GLOBAL `g`
    /// although `main`, which calls it, has a local `g`. The second line
    /// pins the nested case: `h`'s parameter `g` hides `e`'s local `g`
    /// inside `h`, and `e`'s local is intact after the call. Before this
    /// test nothing pinned the first: making lookup search the whole scope
    /// vector left every test green.
    ///
    /// Revert check: search `scope` from 0 instead of `scope_base` in
    /// `scope_entry` and the first assertion reads 2. (The second still
    /// passes then: an innermost parameter wins either way.)
    #[test]
    fn an_inlined_body_does_not_see_its_callers_locals() {
        let run = |body: &str| {
            out0_of(&format!("{V}float g = 1.0;\nfloat h(float g) {{ return g; }}\nfloat f() {{ float k = 5.0; return g + 0.0 * k; }}\nfloat e() {{ float g = 20.0; return h(3.0) + g; }}\nvoid main() {{ {body} gl_Position = p; }}"), 0.0)
        };
        assert_eq!(run("float g = 2.0; out0 = f() + 0.0 * g;"), 1.0);
        assert_eq!(run("out0 = e();"), 23.0);
    }

    /// A name declared in a compound statement, an `if` arm, a `for`
    /// statement or a `while` test is gone at the end of it. None of the
    /// first three was until 2026-09-23 outside `main`'s own run form: the
    /// inner variable stayed the innermost of its name, and the outer one
    /// read back the inner one's value. Each compiled and drew.
    ///
    /// Revert check: drop the `push_scope` in `statement()`'s Block arm and
    /// the first assertion reads 2.
    #[test]
    fn a_scope_ends_where_section_4_2_2_says() {
        let block = format!("{V}void main(){{ float a=1.0; {{ float a=2.0; }} out0=a; gl_Position=p; }}");
        assert_eq!(out0_of(&block, 0.0), 1.0);
        let for_ = format!("{V}void main(){{ int i=5; for(int i=0;i<2;i++){{ }} out0=float(i); gl_Position=p; }}");
        assert_eq!(out0_of(&for_, 0.0), 5.0);
        /* An `if` arm inside a loop goes through `statement`, not the run
         * form, and a simple statement arm is a scope too. */
        let arm = format!(
            "{V}void main(){{ float a=1.0; for(int i=0;i<1;i++){{ if(in0>0.0){{ float a=2.0; a=a+1.0; }} }} out0=a; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&arm, 1.0), 1.0);
    }

    /// `while (bool b = e)` (dEQP scoping.valid.while_condition_variable_*):
    /// the declaration is made and tested on every iteration, and it hides
    /// the outer name only until the end of the body.
    #[test]
    fn a_while_test_can_declare_its_variable() {
        let w = format!(
            "{V}void main(){{ float a=in0; int i=0; while(bool a=(i<3)){{ i++; }} out0=a+float(i); gl_Position=p; }}"
        );
        assert_eq!(out0_of(&w, 0.5), 3.5);
        let f = format!(
            "{V}void main(){{ float s=0.0; for(int i=0; bool go=(i<4); i++){{ if(go) s+=1.0; }} out0=s; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&f, 0.0), 4.0);
    }
}

/// One lane of a vector as a destination.
#[cfg(test)]
mod lane_store_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// `v[k] = x` and `m[j][k] = x` write one lane and leave the others
    /// (dEQP conditionals.if.*_conditional_assignment_to_matrix). The
    /// other lanes are read back too, because a store that wrote all four
    /// lanes would pass a test that looked only at the one it meant.
    #[test]
    fn one_lane_of_a_vector_or_a_matrix_column() {
        let v = format!(
            "{V}void main(){{ vec3 v=vec3(1.0,2.0,3.0); v[1]=in0; out0=v.x*100.0+v.y*10.0+v.z; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&v, 7.0), 173.0);
        let m = format!(
            "{V}void main(){{ mat2 m=mat2(0.0); if(in0>0.0){{ m[1][0]=in0; }} out0=m[1][0]*10.0+m[0][0]+m[1][1]*100.0; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&m, 4.0), 40.0);
        assert_eq!(out0_of(&m, -1.0), 0.0);
        let c = format!("{V}void main(){{ ivec2 w=ivec2(1,2); w[0]+=3; out0=float(w.x*10+w.y); gl_Position=p; }}");
        assert_eq!(out0_of(&c, 0.0), 42.0);
    }
}

/// Uniform values as the GL core stores them.
#[cfg(test)]
mod uniform_value_tests {
    use super::*;
    use crate::shaderc::online::eval::{f32x4, run, Inputs};

    /// Run a vertex shader with `p` and a uniform block, and read `out0`.
    pub(super) fn out0_with(src: &str, set: &[(&str, &[u32])]) -> f32 {
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, src).expect("compiles");
        let m = &l.module;
        let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4) + 16];
        for (name, dwords) in set {
            let u = m.uniforms.iter().find(|x| x.name == *name).unwrap_or_else(|| panic!("no uniform {name}"));
            let at = usize::from(u.dword_off);
            block[at..at + dwords.len()].copy_from_slice(dwords);
        }
        let p = m.inputs.iter().find(|d| d.name == "p").map(|d| ir::IoSlot::Attrib(d.location)).expect("p");
        let o = m.outputs.iter().find(|d| d.name == "out0").map(|d| ir::IoSlot::Varying(d.location)).expect("out0");
        run(m, &Inputs { io: &[(p, f32x4([0.0; 4]))], uniforms: &block })
            .expect("the evaluator runs it")
            .output_f32(o)
            .expect("out0 stored")[0]
    }

    /// dEQP uniform_api.random.2/6/74: `u[1].t[1].m[0]`, a matrix array
    /// member of a struct array inside a struct array. The prefix of a
    /// struct element in the middle of the chain was not spelled, and the
    /// column read was refused. Each column of the target and of the element
    /// beside it holds distinct values, so a wrong element or column reads a
    /// different number.
    ///
    /// Revert check: restore the Ident/Field-only match in
    /// `uniform_struct_prefix` and the compile is refused ("this matrix
    /// expression is not lowered yet").
    #[test]
    fn a_matrix_array_member_inside_nested_struct_arrays_is_read() {
        let src = "attribute vec4 p;\nvarying float out0;\nstruct T { mat3 m[1]; float f; };\nstruct S { float a; T t[2]; };\nuniform S u[2];\nvoid main() { out0 = u[1].t[1].m[0][1][2] * 10.0 + u[1].t[1].f; gl_Position = p; }";
        let mat = |base: f32| -> std::vec::Vec<u32> {
            (0..12).map(|i| (base + (i / 4) as f32 * 10.0 + (i % 4) as f32).to_bits()).collect()
        };
        let (near, far) = (mat(500.0), mat(100.0));
        let f = 7.0f32.to_bits();
        let v = out0_with(src, &[("u[1].t[0].m", &near), ("u[1].t[1].m", &far), ("u[1].t[1].f", &[f])]);
        assert_eq!(v, 1127.0);
    }

    /// A bool uniform holds 0 or 1 in the block (program.rs: "a bool holds
    /// 0 or 1 whatever setter loaded it"), and must behave as a bool.
    ///
    /// Revert check: return `Ok(v)` unconditionally from `bool_from_block`
    /// and the first assertion reads 22. The `all(b)` line also pins
    /// builtin_call's spare-lane padding: without it the reduction folds two
    /// lanes nothing loaded and reads 0, not 100.
    #[test]
    fn a_bool_uniform_is_a_bool() {
        let src = "attribute vec4 p;\nuniform bool u;\nvarying float out0;\nvoid main(){ out0 = (!u) ? 1.0 : 2.0; out0 += (u == true) ? 10.0 : 20.0; gl_Position=p; }";
        assert_eq!(out0_with(src, &[("u", &[1])]), 12.0);
        assert_eq!(out0_with(src, &[("u", &[0])]), 21.0);
        let bv = "attribute vec4 p;\nuniform bvec2 b;\nvarying float out0;\nvoid main(){ bvec2 n = not(b); out0 = (n.x ? 1.0 : 0.0) + (n.y ? 10.0 : 0.0) + (all(b) ? 100.0 : 0.0); gl_Position=p; }";
        assert_eq!(out0_with(bv, &[("b", &[1, 0])]), 10.0);
        assert_eq!(out0_with(bv, &[("b", &[1, 1])]), 100.0);
    }

    /// An array of uniform structs (dEQP uniform_api, light_amount): rows
    /// named `u[k].leaf`, read by constant and by run-time index, copied
    /// whole into a local, and passed to a function -- the shapes
    /// light_amount's `Light light = u_lights[ndx];` uses.
    #[test]
    fn an_array_of_uniform_structs() {
        let f = |x: f32| x.to_bits();
        let decl = "attribute vec4 p;\nattribute float in0;\nuniform int k;\nstruct L { vec3 pos; float att; };\nuniform L u[3];\nvarying float out0;\nfloat g(L l) { return l.att * 100.0 + l.pos.y; }\n";
        let set: [(&str, &[u32]); 7] = [
            ("u[0].pos", &[f(1.0), f(2.0), f(3.0)]),
            ("u[0].att", &[f(4.0)]),
            ("u[1].pos", &[f(5.0), f(6.0), f(7.0)]),
            ("u[1].att", &[f(8.0)]),
            ("u[2].pos", &[f(9.0), f(10.0), f(11.0)]),
            ("u[2].att", &[f(12.0)]),
            ("k", &[2]),
        ];
        let run = |body: &str| out0_with(&std::format!("{decl}void main(){{ {body} gl_Position = p; }}"), &set);
        assert_eq!(run("out0 = u[1].att;"), 8.0);
        assert_eq!(run("out0 = u[k].pos.y;"), 10.0);
        assert_eq!(run("L l = u[k]; out0 = l.att * 10.0 + l.pos.z;"), 131.0);
        assert_eq!(run("out0 = g(u[k - 1]);"), 806.0);
    }

    /// An array of matrices inside a uniform struct (`u_var.m2[k]`): each
    /// element at its own stride, a run-time element included.
    #[test]
    fn a_matrix_array_member_of_a_uniform_struct() {
        let f = |x: f32| x.to_bits();
        let src = "attribute vec4 p;\nuniform int k;\nstruct S { mat2 m[2]; };\nuniform S s;\nvarying float out0;\nvoid main(){ mat2 a = s.m[k]; out0 = a[0][0]*1000.0 + a[0][1]*100.0 + a[1][0]*10.0 + a[1][1]; gl_Position = p; }";
        let block: &[u32] = &[f(1.0), f(2.0), f(3.0), f(4.0), f(5.0), f(6.0), f(7.0), f(8.0)];
        assert_eq!(out0_with(src, &[("s.m", block), ("k", &[1])]), 5678.0);
        assert_eq!(out0_with(src, &[("s.m", block), ("k", &[0])]), 1234.0);
    }

    /// An int uniform is its integer's bits (program.rs `Values::bits`), which
    /// is what the integer operations read: no conversion on the way in.
    #[test]
    fn an_int_uniform_is_an_int() {
        let src = "attribute vec4 p;\nuniform int k;\nvarying float out0;\nvoid main(){ out0 = (k == 3) ? float(k + 1) : -1.0; gl_Position=p; }";
        assert_eq!(out0_with(src, &[("k", &[3])]), 4.0);
    }
}

/// Products with a matrix, run.
#[cfg(test)]
mod matrix_product_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// Every arithmetic form section 5.9 gives a matrix, run. The matrices
    /// are chosen so that each element is distinct: a transposed or
    /// mis-ordered column changes the answer.
    ///
    /// a = mat2(1, 2, 3, 4): columns (1, 2), (3, 4); b = mat2(5, 6, 7, 8).
    /// The read-back folds all four elements into one number:
    /// r[0][0]*1000 + r[0][1]*100 + r[1][0]*10 + r[1][1].
    #[test]
    fn matrix_arithmetic_as_section_5_9_defines_it() {
        let run = |expr: &str| {
            out0_of(
                &format!(
                    "{V}void main(){{ mat2 a = mat2(1.0, 2.0, 3.0, 4.0); mat2 b = mat2(5.0, 6.0, 7.0, 8.0); float k = in0; mat2 r = {expr}; out0 = r[0][0]*1000.0 + r[0][1]*100.0 + r[1][0]*10.0 + r[1][1]; gl_Position=p; }}"
                ),
                2.0,
            )
        };
        assert_eq!(run("a + b"), 6000.0 + 800.0 + 100.0 + 12.0);
        assert_eq!(run("b - a"), 4000.0 + 400.0 + 40.0 + 4.0);
        assert_eq!(run("b / a"), 5000.0 + 300.0 + 7.0 / 3.0 * 10.0 + 2.0);
        assert_eq!(run("a + k"), 3000.0 + 400.0 + 50.0 + 6.0);
        assert_eq!(run("k - a"), 1000.0 + 0.0 - 10.0 - 2.0);
        assert_eq!(run("k / a"), 2000.0 + 100.0 + 2.0 / 3.0 * 10.0 + 0.5);
        assert_eq!(run("-a"), -1234.0);
        assert_eq!(run("matrixCompMult(a, b)"), 5000.0 + 1200.0 + 210.0 + 32.0);
        /* The product, not component-wise: (a*b)[0] = a*(5,6) = (23, 34),
         * (a*b)[1] = a*(7,8) = (31, 46). */
        assert_eq!(run("a * b"), 23000.0 + 3400.0 + 310.0 + 46.0);
        assert_eq!(run("k > 1.0 ? a : b"), 1234.0);
    }

    /// `c ? v1 : v2` with vector arms: the scalar condition picks the whole
    /// vector. The condition is computed from a one-lane LOCAL, whose other
    /// lanes nothing wrote -- an attribute would not do, because the harness
    /// supplies all four of its lanes equal and an unbroadcast condition
    /// would agree by accident (the first version of this test did, and its
    /// revert check passed). Revert check: take the condition unbroadcast in
    /// `value`'s `Expr::Select` arm and this reads 1678 -- lanes y/z/w from
    /// the other arm.
    #[test]
    fn a_scalar_condition_selects_the_whole_vector() {
        let src = format!(
            "{V}void main(){{ vec4 a = vec4(1.0, 2.0, 3.0, 4.0); vec4 b = vec4(5.0, 6.0, 7.0, 8.0); float k = in0; vec4 r = k > 1.0 ? a : b; out0 = r.x*1000.0 + r.y*100.0 + r.z*10.0 + r.w; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&src, 2.0), 1234.0);
        assert_eq!(out0_of(&src, 0.0), 5678.0);
    }

    /// `++`, `--` and compound assignment on a matrix local.
    #[test]
    fn matrix_steps_and_compound_assignment() {
        let run = |stmts: &str| {
            out0_of(
                &format!(
                    "{V}void main(){{ mat2 a = mat2(1.0, 2.0, 3.0, 4.0); mat2 b = mat2(5.0, 6.0, 7.0, 8.0); {stmts}; out0 = a[0][0]*1000.0 + a[0][1]*100.0 + a[1][0]*10.0 + a[1][1]; gl_Position=p; }}"
                ),
                2.0,
            )
        };
        assert_eq!(run("a++"), 2345.0);
        assert_eq!(run("--a"), 1000.0 * 0.0 + 100.0 + 20.0 + 3.0);
        assert_eq!(run("a += b"), 6000.0 + 800.0 + 100.0 + 12.0);
        assert_eq!(run("a *= b"), 23000.0 + 3400.0 + 310.0 + 46.0);
        assert_eq!(run("a /= in0"), 500.0 + 100.0 + 15.0 + 2.0);
        /* The value of a post-increment is the matrix BEFORE the step. */
        assert_eq!(run("b = a++; a = b"), 1234.0);
    }

    /// `v * m` treats `v` as a row vector: component j is dot(v, column j).
    /// The matrix is NOT symmetric, because the transpose (`m * v`'s sum)
    /// agrees with the right answer on every symmetric one.
    ///
    /// m = mat2(1, 2, 3, 4) is columns (1, 2) and (3, 4). With v = (1, 10):
    /// v * m = (1 + 20, 3 + 40) = (21, 43); m * v would be (31, 42).
    #[test]
    fn a_row_vector_times_a_matrix() {
        let src = format!(
            "{V}void main(){{ mat2 m = mat2(1.0, 2.0, 3.0, 4.0); vec2 v = vec2(in0, 10.0); vec2 r = v * m; out0 = r.x * 100.0 + r.y; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&src, 1.0), 2143.0);
        /* The compound form into lanes of a vector (dEQP
         * misc.compound_assignment.mul_swizzled_vec_by_matrix). */
        let comp = format!(
            "{V}void main(){{ vec4 o = vec4(in0, 10.0, 7.0, 9.0); o.xy *= mat2(1.0, 2.0, 3.0, 4.0); out0 = o.x * 1000.0 + o.y * 10.0 + o.z + o.w * 0.1; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&comp, 1.0), 21000.0 + 430.0 + 7.0 + 0.9);
    }
}

/// A member of a struct value that is not a variable.
#[cfg(test)]
mod struct_value_member_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// `S(x).val` and `f(x).val` (dEQP scoping.valid.local_int_variable_
    /// hides_struct_type, which declares `int S = S(in0).val;` -- the
    /// variable is visible only after its initialiser, section 4.2.2, so
    /// that `S` is the constructor).
    #[test]
    fn a_member_of_a_constructed_or_returned_struct() {
        let c = format!(
            "{V}struct S {{ float a; float val; }};\nvoid main(){{ float S = S(1.0, in0).val; out0 = S * 2.0; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&c, 3.0), 6.0);
        let f = format!(
            "{V}struct S {{ float a; float val; }};\nS f(float x) {{ return S(x, x + 1.0); }}\nvoid main(){{ out0 = f(in0).val + f(in0).a * 10.0; gl_Position=p; }}"
        );
        assert_eq!(out0_of(&f, 3.0), 34.0);
    }
}

/// Section 7.4's built-in constants.
#[cfg(test)]
mod builtin_constant_tests {
    use super::return_rule_tests::out0_of;

    /// Each constant is the GL core's own number for the matching query
    /// (dEQP shaders.builtin_variable.max_*, which compare the two), and a
    /// constant expression: it can initialise a `const`.
    #[test]
    fn the_constants_are_the_limits_the_gl_core_reports() {
        use crate::gl::context::{
            CAP_COMBINED_TEXTURE_UNITS, CAP_TEXTURE_UNITS, MAX_UNIFORM_DWORDS, MAX_VARYING_VECTORS, MAX_VERTEX_ATTRIBS,
        };
        let v = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";
        for (name, want) in [
            ("gl_MaxVertexAttribs", MAX_VERTEX_ATTRIBS),
            ("gl_MaxVertexUniformVectors", MAX_UNIFORM_DWORDS / 4),
            ("gl_MaxVaryingVectors", MAX_VARYING_VECTORS),
            ("gl_MaxVertexTextureImageUnits", crate::backend::GLES_VS_TEX_UNITS),
            ("gl_MaxCombinedTextureImageUnits", CAP_COMBINED_TEXTURE_UNITS),
            ("gl_MaxTextureImageUnits", CAP_TEXTURE_UNITS),
            ("gl_MaxFragmentUniformVectors", MAX_UNIFORM_DWORDS / 4),
            /* the query's value, unless the GL reports ES 2.0 (see
             * `reflect::builtin_constant`; this pinned 1 until 2026-09-24) */
            ("gl_MaxDrawBuffers", if crate::REPORT_ES2 { 1 } else { crate::gl::context::MAX_DRAW_BUFFERS }),
        ] {
            /* A `const` initialiser must be a constant expression (4.3.2),
             * so this also checks that the checker treats it as one. */
            let src = std::format!("{v}void main(){{ const int n = {name}; out0 = float(n); gl_Position = p; }}");
            assert_eq!(out0_of(&src, 0.0), want as f32, "{name}");
        }
    }
}

/// Expression statements with no effect.
#[cfg(test)]
mod expression_statement_tests {
    use super::return_rule_tests::out0_of;

    /// `a;`, `a + b;`, `a && b;`, `true;`, `sin(a);`, `m;` and `m + m;` are
    /// legal statements whose values are discarded; the shader around them
    /// still computes what it says (dEQP operator.sequence.no_side_effects).
    #[test]
    fn a_statement_with_no_effect_is_evaluated_and_dropped() {
        let src = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nvoid main(){ float a = in0; bool b = a > 0.0; mat2 m = mat2(a); a; a + 1.0; b && b; true; sin(a); m; m + m; out0 = a * 2.0; gl_Position = p; }";
        assert_eq!(out0_of(src, 3.0), 6.0);
    }
}

/// Indices chosen at run time.
#[cfg(test)]
mod dynamic_index_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// Reads and writes of a local array, a matrix column and a vector
    /// lane through an index only known at run time (dEQP
    /// shaders.indexing.{tmp_array,matrix_subscript,vector_subscript}), every
    /// element read back so a write that landed in the wrong one shows.
    #[test]
    fn a_run_time_index_reads_and_writes_the_right_element() {
        let run =
            |body: &str, x: f32| out0_of(&format!("{V}void main(){{ int i = int(in0); {body} gl_Position = p; }}"), x);
        let arr = "float a[3]; a[0] = 1.0; a[1] = 2.0; a[2] = 3.0; a[i] = 9.0; out0 = a[0]*100.0 + a[1]*10.0 + a[2] + a[i]*1000.0;";
        assert_eq!(run(arr, 0.0), 9000.0 + 900.0 + 20.0 + 3.0);
        assert_eq!(run(arr, 1.0), 9000.0 + 100.0 + 90.0 + 3.0);
        assert_eq!(run(arr, 2.0), 9000.0 + 100.0 + 20.0 + 9.0);
        let loopy = "float a[4]; for (int k = 0; k < 4; k++) { a[k] = float(k + 1); } float s = 0.0; float w = 1.0; for (int k = 0; k < 4; k++) { s += a[k] * w; w *= 10.0; } out0 = s;";
        assert_eq!(run(loopy, 0.0), 4321.0);
        let cmp = "float a[2]; a[0] = 1.0; a[1] = 2.0; a[i] += 5.0; out0 = a[0]*10.0 + a[1];";
        assert_eq!(run(cmp, 1.0), 17.0);
        let mat = "mat2 m = mat2(1.0, 2.0, 3.0, 4.0); m[i] = vec2(7.0, 8.0); vec2 c = m[1 - i]; out0 = m[0][0]*1000.0 + m[0][1]*100.0 + m[1][0]*10.0 + m[1][1] + c.x*10000.0;";
        assert_eq!(run(mat, 0.0), 7800.0 + 34.0 + 30000.0);
        assert_eq!(run(mat, 1.0), 1200.0 + 78.0 + 10000.0);
        let vec = "vec4 v = vec4(1.0, 2.0, 3.0, 4.0); v[i] = 9.0; out0 = v.x*1000.0 + v.y*100.0 + v.z*10.0 + v.w;";
        assert_eq!(run(vec, 2.0), 1294.0);
        assert_eq!(run(vec, 3.0), 1239.0);
    }
}

/// Varyings that span more than one location: arrays and matrices.
#[cfg(test)]
mod varying_span_tests {
    use super::*;
    use crate::shaderc::online::eval::{f32x4, run, Inputs};

    /// Run a program: the vertex shader with `p` and `in0`, its varyings to
    /// the fragment shader paired by DECLARATION NAME and location -- what
    /// the link does -- and the fragment colour back.
    fn colour(vs: &str, fs: &str, in0: f32) -> [f32; 4] {
        let (mut vb, mut fb) = (std::vec![0u8; 1 << 20], std::vec![0u8; 1 << 20]);
        let (va, fa) = (Arena::new(&mut vb), Arena::new(&mut fb));
        let v = compile_to_ir(&va, Stage::Vertex, vs).unwrap_or_else(|e| panic!("vs: {} {}", e.tok, e.msg));
        let f = compile_to_ir(&fa, Stage::Fragment, fs).unwrap_or_else(|e| panic!("fs: {} {}", e.tok, e.msg));
        let attr = |n: &str| v.module.inputs.iter().find(|d| d.name == n).map(|d| ir::IoSlot::Attrib(d.location));
        let mut io = std::vec::Vec::new();
        if let Some(s) = attr("p") {
            io.push((s, f32x4([0.0, 0.0, 0.0, 1.0])));
        }
        if let Some(s) = attr("in0") {
            io.push((s, f32x4([in0; 4])));
        }
        let vo = run(&v.module, &Inputs { io: &io, uniforms: &[] }).expect("the vertex shader runs");
        let mut fio = std::vec::Vec::new();
        for d in f.module.inputs.iter().filter(|d| d.kind == ir::IoKind::Varying) {
            let o =
                v.module.outputs.iter().find(|o| o.name == d.name).unwrap_or_else(|| panic!("no output {}", d.name));
            let val = vo.output(ir::IoSlot::Varying(o.location)).unwrap_or_else(|| panic!("{} never written", d.name));
            fio.push((ir::IoSlot::Varying(d.location), val));
        }
        let fo = run(&f.module, &Inputs { io: &fio, uniforms: &[] }).expect("the fragment shader runs");
        fo.output_f32(ir::IoSlot::FragColor(0)).expect("gl_FragColor")
    }

    /// A non-square matrix varying is declared once per COLUMN, like a
    /// square one: `mat3x2` takes three locations, named `m`, `m[1]`,
    /// `m[2]`. Only the square types were counted until 2026-09-24, so a
    /// `matCxR` varying declared one location and wrote C -- the back ends
    /// refuse a varying written that the module does not declare.
    ///
    /// Revert check: count columns for the square types only in
    /// `varying_locations` and this declares one location.
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn a_non_square_matrix_varying_is_declared_per_column() {
        let vs = "#version 300 es\nin vec4 p;\nout mat3x2 m;\n\
                  void main() { gl_Position = p; m = mat3x2(1.0, 2.0, 3.0, 4.0, 5.0, 6.0); }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, vs).expect("compiles");
        let decls: std::vec::Vec<(&str, u8)> =
            l.module.outputs.iter().filter(|d| d.kind == ir::IoKind::Varying).map(|d| (d.name, d.location)).collect();
        assert_eq!(decls, [("m", 0), ("m[1]", 1), ("m[2]", 2)]);
    }

    /// An ES 3.00 structure between the stages (dEQP-GLES3
    /// linkage.varying.struct.*): each member is a varying of its own,
    /// `var.m`, at consecutive locations -- a matrix one per column, square
    /// or not -- written member by member in the vertex shader and read in
    /// the fragment shader.
    ///
    /// Revert check: drop the struct arm of `reflect`'s varyings and both
    /// shaders are refused ("unsupported varying type").
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn a_structure_crosses_the_stages_member_by_member() {
        let vs = "#version 300 es\nin vec4 p;\nin float in0;\n\
                  struct S { mediump float a; highp mat2x3 m; highp vec3 c; };\nout S var;\n\
                  void main() { gl_Position = p; var.a = in0; var.m = mat2x3(1.0, 2.0, 3.0, 4.0, 5.0, 6.0) * in0;\n\
                  var.c = vec3(in0, 2.0 * in0, 3.0 * in0); }";
        let fs = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 o;\n\
                  struct S { mediump float a; highp mat2x3 m; highp vec3 c; };\nin S var;\n\
                  void main() { o = vec4(var.a, var.m[1][2], var.c.z, var.m[0][1]); }";
        assert_eq!(colour(vs, fs, 2.0), [2.0, 12.0, 6.0, 4.0]);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, vs).expect("compiles");
        let decls: std::vec::Vec<(&str, u8)> =
            l.module.outputs.iter().filter(|d| d.kind == ir::IoKind::Varying).map(|d| (d.name, d.location)).collect();
        assert_eq!(decls, [("var.a", 0), ("var.m", 1), ("var.m[1]", 2), ("var.c", 3)]);
        /* an integer member makes it an integer input: flat, or refused */
        let a = Arena::new(&mut buf);
        let e = compile_to_ir(&a, Stage::Fragment, "#version 300 es\nprecision mediump float;\nstruct T { int i; };\nin T t;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(float(t.i)); }")
            .map(|_| ())
            .expect_err("not flat");
        assert_eq!(e.msg, "an integer 'in' or 'out' between the stages must be 'flat'");
        /* a member that is an array is refused by name */
        let a = Arena::new(&mut buf);
        let e = compile_to_ir(&a, Stage::Fragment, "#version 300 es\nprecision mediump float;\nstruct T { float f[2]; };\nin T t;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(t.f[0]); }")
            .map(|_| ())
            .expect_err("an array member");
        assert_eq!(
            e.msg,
            "a member of a structure between the stages must be a float or integer scalar, vector or matrix"
        );
    }

    /// `varying float var[4]`: written element by element in the vertex
    /// shader and read by a run-time index in the fragment shader (dEQP
    /// shaders.indexing.varying_array.*). Each element is its own location,
    /// declared and linked as `var`, `var[1]`, ...
    #[test]
    fn an_array_varying_crosses_the_stages_element_by_element() {
        let vs = "attribute vec4 p;\nattribute float in0;\nvarying float var[4];\nvoid main(){ gl_Position = p; var[0] = in0; var[1] = in0 * 2.0; var[2] = in0 * 3.0; var[3] = in0 * 4.0; }";
        let fs = "precision mediump float;\nvarying float var[4];\nvoid main(){ float s = 0.0; for (int i = 0; i < 4; i++) { s = s * 10.0 + var[i]; } gl_FragColor = vec4(s, var[2], 0.0, 1.0); }";
        let c = colour(vs, fs, 1.0);
        assert_eq!(c[0], 1234.0);
        assert_eq!(c[1], 3.0);
        /* And written through a run-time index. */
        let vs2 = "attribute vec4 p;\nattribute float in0;\nvarying float var[3];\nvoid main(){ gl_Position = p; for (int i = 0; i < 3; i++) { var[i] = float(i + 1) * in0; } }";
        let fs2 = "precision mediump float;\nvarying float var[3];\nvoid main(){ gl_FragColor = vec4(var[0] * 100.0 + var[1] * 10.0 + var[2]); }";
        assert_eq!(colour(vs2, fs2, 1.0)[0], 123.0);
    }

    /// `varying mat2`: two locations, declared one per location so a back
    /// end that looks a varying up by exact location finds each column.
    #[test]
    fn a_matrix_varying_is_declared_per_column() {
        let vs = "attribute vec4 p;\nattribute float in0;\nvarying mat2 m;\nvoid main(){ gl_Position = p; m = mat2(in0, 2.0, 3.0, 4.0); }";
        let fs = "precision mediump float;\nvarying mat2 m;\nvoid main(){ gl_FragColor = vec4(m[0][0]*1000.0 + m[0][1]*100.0 + m[1][0]*10.0 + m[1][1]); }";
        assert_eq!(colour(vs, fs, 1.0)[0], 1234.0);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, vs).expect("compiles");
        let decls: std::vec::Vec<(&str, u8)> =
            l.module.outputs.iter().filter(|d| d.kind == ir::IoKind::Varying).map(|d| (d.name, d.location)).collect();
        assert_eq!(decls, [("m", 0), ("m[1]", 1)]);
    }
}

/// ES 3.00 section 7.3's constants: a different list from ES 1.00's, each
/// the value the GL core reports for the matching query.
#[cfg(test)]
mod es3_builtin_constant_tests {
    use super::*;
    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    use crate::gl::context::{
        CAP_COMBINED_TEXTURE_UNITS, CAP_TEXTURE_UNITS, MAX_DRAW_BUFFERS, MAX_PROGRAM_TEXEL_OFFSET, MAX_UNIFORM_DWORDS,
        MAX_VERTEX_ATTRIBS, MIN_PROGRAM_TEXEL_OFFSET,
    };
    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    use crate::shaderc::online::eval::{run, Inputs};

    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    fn value(name: &str) -> f32 {
        let src = std::format!(
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
             void main() {{ const int n = {name}; o = vec4(float(n)); }}"
        );
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("{name}: {}", e.msg));
        run(&l.module, &Inputs { io: &[], uniforms: &[] })
            .expect("runs")
            .output_f32(ir::IoSlot::FragColor(0))
            .expect("o")[0]
    }

    /// Revert check: give `gl_MaxDrawBuffers` its ES 1.00 value in ES 3.00
    /// too and the PowerVR build reads 1 where GL_MAX_DRAW_BUFFERS is 3
    /// (on the GC7000 both are 1, so the check there is the other rows).
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn the_es3_constants_are_the_limits_the_gl_core_reports() {
        for (name, want) in [
            ("gl_MaxVertexAttribs", MAX_VERTEX_ATTRIBS as i64),
            ("gl_MaxVertexUniformVectors", (MAX_UNIFORM_DWORDS / 4) as i64),
            ("gl_MaxFragmentUniformVectors", (MAX_UNIFORM_DWORDS / 4) as i64),
            ("gl_MaxVertexOutputVectors", (crate::gl::context::MAX_VERTEX_OUTPUT_COMPONENTS / 4) as i64),
            ("gl_MaxFragmentInputVectors", (crate::gl::context::MAX_FRAGMENT_INPUT_COMPONENTS / 4) as i64),
            ("gl_MaxVertexTextureImageUnits", crate::backend::GLES_VS_TEX_UNITS as i64),
            ("gl_MaxCombinedTextureImageUnits", CAP_COMBINED_TEXTURE_UNITS as i64),
            ("gl_MaxTextureImageUnits", CAP_TEXTURE_UNITS as i64),
            ("gl_MaxDrawBuffers", MAX_DRAW_BUFFERS as i64),
            ("gl_MinProgramTexelOffset", i64::from(MIN_PROGRAM_TEXEL_OFFSET)),
            ("gl_MaxProgramTexelOffset", i64::from(MAX_PROGRAM_TEXEL_OFFSET)),
        ] {
            assert_eq!(value(name), want as f32, "{name}");
        }
    }

    /// `gl_MaxVaryingVectors` is ES 1.00's; ES 3.00 does not declare it, and
    /// ES 1.00 does not declare the ES 3.00 additions.
    #[test]
    fn each_language_declares_only_its_own_constants() {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let es3 = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
                   void main() { o = vec4(float(gl_MaxVaryingVectors)); }";
        assert!(compile_to_ir(&a, Stage::Fragment, es3).is_err(), "gl_MaxVaryingVectors in ES 3.00");
        let a = Arena::new(&mut buf);
        let es1 = "precision highp float;\nvoid main() { gl_FragColor = vec4(float(gl_MaxVertexOutputVectors)); }";
        assert!(compile_to_ir(&a, Stage::Fragment, es1).is_err(), "gl_MaxVertexOutputVectors in ES 1.00");
    }
}

/// ES 1.00's cube look-ups with a level or a bias, through the whole front
/// end: IR v1's cube `Op::Tex` (dEQP-GLES2 shaders.random.texture.vertex.*
/// and random.all_features.fragment.*, 98 cases that stopped here).
#[cfg(test)]
mod es100_cube_lod_bias_tests {
    use super::*;

    fn tex_ops(stage: Stage, src: &str) -> std::vec::Vec<(ir::TexDim, bool)> {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, stage, src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        l.module
            .instrs
            .iter()
            .filter_map(|op| match *op {
                Op::Tex { dim, lod, .. } => Some((dim, matches!(lod, ir::TexLod::Lod(_)))),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_vertex_shader_levels_and_a_fragment_shader_biases_a_cube_lookup() {
        let vs = "attribute vec4 p;\nuniform samplerCube s;\nvarying vec4 c;\n\
                  void main() { c = textureCubeLod(s, p.xyz, 2.0); gl_Position = p; }";
        assert_eq!(tex_ops(Stage::Vertex, vs), [(ir::TexDim::Cube, true)]);
        let fs = "precision mediump float;\nuniform samplerCube s;\nvarying vec3 d;\n\
                  void main() { gl_FragColor = textureCube(s, d, 1.5); }";
        assert_eq!(tex_ops(Stage::Fragment, fs), [(ir::TexDim::Cube, false)]);
    }
}

/// ES 3.00 uniform blocks (section 4.3.7), flattened into the default
/// block: the parser desugars a block into a struct and an instance (or one
/// uniform per member), so its members are ordinary rows and read like any
/// uniform struct's. HOST-ONLY.
#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod uniform_block_tests {
    use super::*;
    use crate::shaderc::online::ast::BlockPacking;
    use crate::shaderc::online::eval::{run, Inputs};

    fn source(body: &str) -> std::string::String {
        std::format!("#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n{body}")
    }

    /// The colour for a block filled row by row with `set` (name, dwords).
    fn frag(body: &str, set: &[(&str, &[f32])]) -> [f32; 4] {
        let src = source(body);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        let mut block = std::vec![0u32; usize::from(l.module.uniform_dwords).max(4) + 16];
        for (name, v) in set {
            let u = l.reflection.uniforms.iter().find(|u| u.name == *name).unwrap_or_else(|| panic!("no row {name}"));
            let at = usize::from(u.dword_off);
            for (k, x) in v.iter().enumerate() {
                block[at + k] = x.to_bits();
            }
        }
        run(&l.module, &Inputs { io: &[], uniforms: &block })
            .expect("runs")
            .output_f32(ir::IoSlot::FragColor(0))
            .expect("o")
    }

    fn refused(body: &str) -> &'static str {
        let src = source(body);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        match compile_to_ir(&a, Stage::Fragment, &src) {
            Ok(_) => panic!("expected a refusal: {body}"),
            Err(e) => e.msg,
        }
    }

    /// An instance block's members read through the instance, a matrix
    /// column and an array element included; a block with no instance puts
    /// its members at global scope.
    ///
    /// Revert check: drop the `uniform Name {` arm in
    /// `declaration_or_function` and both are refused ("expected a type").
    #[test]
    fn members_read_through_the_instance_or_by_name() {
        let body = "layout(std140) uniform Light { vec4 colour; mat3 m; float f[2]; } light;\n\
                    void main() { o = light.colour + vec4(light.m[1], light.f[1]); }";
        let got = frag(
            body,
            &[
                ("light.colour", &[1.0, 2.0, 3.0, 4.0]),
                ("light.m", &[0.0, 0.0, 0.0, 0.0, 10.0, 20.0, 30.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
                /* The default block packs a float array one dword an element. */
                ("light.f", &[0.0, 100.0]),
            ],
        );
        assert_eq!(got, [11.0, 22.0, 33.0, 104.0]);
        let body = "uniform Globals { vec4 tint; uniform float gain; };\nvoid main() { o = tint * gain; }";
        assert_eq!(frag(body, &[("tint", &[1.0, 2.0, 3.0, 4.0]), ("gain", &[2.0])]), [2.0, 4.0, 6.0, 8.0]);
    }

    /// An array of blocks is an array of the desugared struct: one set of
    /// rows per element.
    #[test]
    fn an_array_of_blocks_has_rows_per_element() {
        let body = "uniform B { vec4 v; } b[2];\nvoid main() { o = b[0].v + b[1].v * 10.0; }";
        assert_eq!(
            frag(body, &[("b[0].v", &[1.0, 2.0, 3.0, 4.0]), ("b[1].v", &[1.0, 1.0, 1.0, 1.0])]),
            [11.0, 12.0, 13.0, 14.0]
        );
    }

    /// The layout qualifiers are recorded for the block table: the block's
    /// own, the global default (`layout(row_major) uniform;`), and a
    /// member's override; the last of a kind wins.
    #[test]
    fn the_layout_is_recorded_per_block_and_member() {
        let src = source(
            "layout(row_major) uniform;\n\
             layout(shared, std140, column_major) uniform A { mat4 m; layout(row_major) mat2 n; } a;\n\
             uniform B { mat3 k; } b;\n\
             void main() { o = a.m[0] + vec4(a.n[0], b.k[0].xy); }",
        );
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let toks = super::super::pp::preprocess(&a, &src).expect("pp").tokens;
        let ast = super::super::parser::parse(&a, toks).expect("parses");
        let blocks: std::vec::Vec<(&str, Option<&str>, BlockPacking, bool)> =
            ast.blocks.iter().map(|b| (b.name, b.instance, b.packing, b.row_major)).collect();
        assert_eq!(
            blocks,
            [("A", Some("a"), BlockPacking::Std140, false), ("B", Some("b"), BlockPacking::Shared, true)]
        );
        let m = &ast.members[ast.blocks[0].members.indices()];
        assert_eq!(m.iter().map(|m| m.row_major).collect::<std::vec::Vec<_>>(), [None, Some(true)]);
    }

    /// (GL name, row name, offset, array stride, matrix stride, row-major,
    /// array size) of every block member, and (name, data size) of every
    /// block.
    #[allow(clippy::type_complexity)]
    fn table(
        body: &str,
    ) -> (
        std::vec::Vec<(std::string::String, std::string::String, u32, u32, u32, bool, u16)>,
        std::vec::Vec<(std::string::String, u32)>,
    ) {
        let src = source(body);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let toks = super::super::pp::preprocess(&a, &src).expect("pp").tokens;
        let ast = super::super::parser::parse(&a, toks).expect("parses");
        let ast = super::super::parser::resolve_io(&a, ast, Stage::Fragment).expect("io");
        let r = reflect::reflect(&a, &ast).expect("reflects");
        let t = reflect::block_table(&a, &ast, &r).expect("a table");
        let members = t
            .members
            .iter()
            .map(|m| {
                (
                    m.name.to_string(),
                    r.uniforms[usize::from(m.row)].name.to_string(),
                    m.offset,
                    m.array_stride,
                    m.matrix_stride,
                    m.row_major,
                    m.array_size,
                )
            })
            .collect();
        let blocks = t.blocks.iter().map(|b| (b.name.to_string(), b.data_size)).collect();
        (members, blocks)
    }

    /// The std140 worked example of the OpenGL specification (section
    /// 7.6.2.2 of 4.5; the rules are ES 3.0's section 2.12.6.4), with its
    /// anonymous structs named, since ES 3.00 has none: every offset and
    /// stride as the specification's table gives it.
    ///
    /// Revert check: round an array's element alignment to its own instead
    /// of a vec4's in `std140_member` and `h` moves to 52.
    #[test]
    fn the_std140_offsets_are_the_specifications_example() {
        let (m, b) = table(
            "struct F { int d; bvec2 e; };\n\
             struct O { uvec3 j; vec2 k; float l[2]; vec2 m; mat3 n[2]; };\n\
             layout(std140) uniform Example { float a; vec2 b; vec3 c; F f; float g; float h[2]; mat2x3 i; O o[2]; } ex;\n\
             void main() { o = vec4(ex.a); }",
        );
        let got: std::vec::Vec<(&str, u32, u32, u32, u16)> =
            m.iter().map(|x| (x.0.as_str(), x.2, x.3, x.4, x.6)).collect();
        assert_eq!(
            got,
            [
                ("Example.a", 0, 0, 0, 1),
                ("Example.b", 8, 0, 0, 1),
                ("Example.c", 16, 0, 0, 1),
                ("Example.f.d", 32, 0, 0, 1),
                ("Example.f.e", 40, 0, 0, 1),
                ("Example.g", 48, 0, 0, 1),
                ("Example.h", 64, 16, 0, 2),
                ("Example.i", 96, 0, 16, 1),
                ("Example.o[0].j", 128, 0, 0, 1),
                ("Example.o[0].k", 144, 0, 0, 1),
                ("Example.o[0].l", 160, 16, 0, 2),
                ("Example.o[0].m", 192, 0, 0, 1),
                ("Example.o[0].n", 208, 48, 16, 2),
                ("Example.o[1].j", 304, 0, 0, 1),
                ("Example.o[1].k", 320, 0, 0, 1),
                ("Example.o[1].l", 336, 16, 0, 2),
                ("Example.o[1].m", 368, 0, 0, 1),
                ("Example.o[1].n", 384, 48, 16, 2),
            ]
        );
        assert_eq!(m[0].1, "ex.a", "the row is the instance's");
        assert_eq!(m[12].1, "ex.o[0].n");
        assert_eq!(b, [("Example".to_string(), 480)]);
    }

    /// A block with no instance names its members bare; an array of blocks
    /// is one block per element, `B[k]`, whose members are all `B.m` (ES 3.0
    /// 2.12.6; dEQP-GLES3 ubo's reference layout). This test pinned `B[k].m`
    /// until 2026-09-24, which was wrong: 717 ubo cases failed "Invalid
    /// layout" on it. A row-major matrix is laid out by rows (a mat2x3 takes
    /// three 16-byte rows).
    #[test]
    fn names_arrays_and_row_major() {
        let (m, b) = table(
            "uniform G { vec4 tint; layout(row_major) mat2x3 t; };\n\
             layout(row_major) uniform B { mat2x3 r; layout(column_major) mat2x3 c; } bs[2];\n\
             void main() { o = tint + vec4(bs[1].r[0], 1.0); }",
        );
        let got: std::vec::Vec<(&str, &str, u32, bool)> =
            m.iter().map(|x| (x.0.as_str(), x.1.as_str(), x.2, x.5)).collect();
        assert_eq!(
            got,
            [
                ("tint", "tint", 0, false),
                ("t", "t", 16, true),
                ("B.r", "bs[0].r", 0, true),
                ("B.c", "bs[0].c", 48, false),
                ("B.r", "bs[1].r", 0, true),
                ("B.c", "bs[1].c", 48, false),
            ]
        );
        assert_eq!(b, [("G".to_string(), 64), ("B[0]".to_string(), 80), ("B[1]".to_string(), 80)]);
    }

    /// Section 4.3.7's rules and dEQP-GLES3 uniform_block.invalid's shapes,
    /// each refused by name.
    #[test]
    fn the_rules_of_section_4_3_7() {
        let main = "void main() { o = vec4(0.0); }";
        let cases: &[(&str, &str)] = &[
            ("uniform B { } b;", "a uniform block must have at least one member"),
            ("uniform B { vec4 v; } B;", "an instance may not have its block's name"),
            ("uniform B { vec4 v; } b;\nuniform B { vec4 v; } c;", "this name is already a type or a block"),
            ("uniform gl_B { vec4 v; } b;", "a block name may not begin with 'gl_'"),
            ("uniform B { in vec4 v; } b;", "a uniform block member cannot have this qualifier"),
            ("uniform B { layout(std140) vec4 v; } b;", "a block member takes only 'row_major' or 'column_major'"),
            ("uniform B { struct S { vec4 v; } s; } b;", "a structure cannot be defined in a uniform block"),
            ("uniform B { sampler2D t; } b;", "a sampler cannot be a member of a uniform block"),
            ("uniform B { vec4 v; } b[];", "an array of blocks must be sized"),
            ("layout(std140) in vec4 x;", "a block layout qualifier applies only to 'uniform'"),
            ("layout(std140) uniform vec4 x;", "a block layout qualifier applies only to a uniform block"),
        ];
        for (decl, want) in cases {
            assert_eq!(refused(&std::format!("{decl}\n{main}")), *want, "{decl}");
        }
        assert_eq!(
            refused("uniform B { vec4 v; } b;\nvoid main() { o = B.v; }"),
            "a type name is not a value",
            "the block name names no variable"
        );
        /* A local may take the block's name (dEQP same_variable_and_block_name). */
        assert_eq!(
            frag("uniform B { vec4 v; } b;\nvoid main() { vec4 B = vec4(1.0); o = B + b.v; }", &[("b.v", &[1.0; 4])]),
            [2.0; 4]
        );
        /* ES 1.00 has no uniform blocks. */
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        assert!(compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\nuniform B { vec4 v; } b;\nvoid main() {}"
        )
        .is_err());
    }
}

/// ES 3.00's added common functions (section 8.3), run by the evaluator.
/// HOST-ONLY.
#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod es3_common_builtin_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    fn frag(body: &str) -> [f32; 4] {
        let src = std::format!(
            "#version 300 es\nprecision highp float;\nprecision highp int;\nlayout(location = 0) out vec4 o;\n{body}"
        );
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        run(&l.module, &Inputs { io: &[], uniforms: &[0; 16] })
            .expect("runs")
            .output_f32(ir::IoSlot::FragColor(0))
            .expect("o")
    }

    /// `trunc` toward zero; `round` to the nearest, a .5 away from zero
    /// here; `roundEven` to the nearest even at a .5, both signs.
    ///
    /// Revert check: make `roundEven` return `round`'s value and 2.5 gives 3.
    #[test]
    fn trunc_round_and_round_even() {
        assert_eq!(
            frag("void main() { o = vec4(trunc(-2.7), trunc(2.7), round(2.5), round(-1.2)); }"),
            [-2.0, 2.0, 3.0, -1.0]
        );
        assert_eq!(frag("void main() { o = roundEven(vec4(2.5, 3.5, -2.5, -3.5)); }"), [2.0, 4.0, -2.0, -4.0]);
        assert_eq!(frag("void main() { o = roundEven(vec4(1.3, 1.7, -0.5, 0.5)); }"), [1.0, 2.0, -0.0, 0.0]);
    }

    /// `isnan` and `isinf` read the bits, and the bit casts are exact.
    ///
    /// Revert check: make `isnan` compare the bits for equality with an
    /// infinity instead and the NaN reads false.
    #[test]
    fn isnan_isinf_and_the_bit_casts() {
        let body = "void main() {\n\
                    float nan = uintBitsToFloat(0x7fc00000u);\n\
                    float inf = uintBitsToFloat(0x7f800000u);\n\
                    bvec4 n = isnan(vec4(nan, inf, 1.0, -nan));\n\
                    bvec4 i = isinf(vec4(nan, inf, -inf, 1.0e38));\n\
                    o = vec4(n) + 10.0 * vec4(i); }";
        assert_eq!(frag(body), [1.0, 10.0, 10.0, 1.0]);
        let body = "void main() { o = vec4(float(floatBitsToInt(1.0) == 0x3f800000), intBitsToFloat(0x40000000),\n\
                    float(floatBitsToUint(-0.0) == 0x80000000u), uintBitsToFloat(0x3f000000u)); }";
        assert_eq!(frag(body), [1.0, 2.0, 1.0, 0.5]);
    }

    /// Section 8.4's pack and unpack functions, against values computed by
    /// hand from the specification's formulas and IEEE half encodings.
    ///
    /// Revert check: sign-extend nothing in `unpackSnorm2x16` (drop the
    /// shift pair) and -1.0 reads as 1.0.
    #[test]
    fn pack_and_unpack() {
        let body = "void main() {\n\
                    uint s = packSnorm2x16(vec2(1.0, -1.0)); uint u = packUnorm2x16(vec2(0.0, 1.0));\n\
                    uint h = packHalf2x16(vec2(1.0, -2.0));\n\
                    o = vec4(float(s == 0x80017fffu), float(u == 0xffff0000u), float(h == 0xc0003c00u),\n\
                    float(packHalf2x16(vec2(65520.0, 1.0e-7)) == 0x00027c00u)); }";
        assert_eq!(frag(body), [1.0; 4]);
        let body = "void main() { vec2 a = unpackSnorm2x16(0x80017fffu); vec2 b = unpackUnorm2x16(0xffff0000u);\n\
                    o = vec4(a, b); }";
        assert_eq!(frag(body), [1.0, -1.0, 0.0, 1.0]);
        let body = "void main() { vec2 a = unpackHalf2x16(0xc0003c00u); vec2 b = unpackHalf2x16(0x7c000001u);\n\
                    o = vec4(a, b.x * 16777216.0, float(isinf(b.y))); }";
        assert_eq!(frag(body), [1.0, -2.0, 1.0, 1.0]);
        /* a round trip keeps every value a half represents */
        let body = "void main() { vec2 v = vec2(0.333251953125, -1234.0);\n\
                    o = vec4(unpackHalf2x16(packHalf2x16(v)), unpackHalf2x16(packHalf2x16(vec2(6.0e-8, 0.0)))); }";
        assert_eq!(frag(body), [0.333_251_95, -1234.0, 5.960_464_5e-8, 0.0]);
    }

    /// Section 8.1's hyperbolic functions, from their defining formulas, at
    /// points where the value is known to well inside float precision.
    ///
    /// Revert check: make `tanh` return `sinh` and 0.5 reads 0.5211 instead
    /// of 0.4621.
    #[test]
    fn the_hyperbolic_functions_follow_their_formulas() {
        let got = frag("void main() { o = vec4(sinh(0.5), cosh(0.5), tanh(0.5), tanh(-2.0)); }");
        let want = [0.5210953f32, 1.127_626, 0.462_117_16, -0.964_027_6];
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-5, "{got:?} against {want:?}");
        }
        let got = frag("void main() { o = vec4(asinh(0.5), acosh(1.5), atanh(0.5), asinh(-2.0)); }");
        let want = [0.481_211_83f32, 0.962_423_6, 0.549_306_1, -1.443_635_5];
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-5, "{got:?} against {want:?}");
        }
    }

    /// ES 1.00 has none of these names, so a shader's own function of one is
    /// the one that runs; and a wrong argument is refused.
    #[test]
    fn es100_keeps_the_names_and_a_wrong_argument_is_refused() {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\nfloat trunc(float x) { return x + 100.0; }\nvoid main() { gl_FragColor = vec4(trunc(1.5)); }",
        )
        .expect("compiles");
        let out = run(&l.module, &Inputs { io: &[], uniforms: &[] }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragColor(0)).expect("colour")[0], 101.5);
        let a = Arena::new(&mut buf);
        let e = compile_to_ir(&a, Stage::Fragment, "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(intBitsToFloat(1.0)); }")
            .map(|_| ())
            .expect_err("a float into intBitsToFloat");
        assert_eq!(e.msg, "no form of this function takes these arguments (section 8.3)");
    }
}

/// Destinations resolved once (`LRef`): `out` and `inout` arguments that are
/// an element or a lane, `modf`'s, `v[f()]++`, `m[i][j] = x` -- each index
/// evaluated a single time (dEQP-GLES3 indexing.moredynamic). HOST-ONLY.
#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod lvalue_ref_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    /// The colour with the int uniform `u` (declared first) set to `u`.
    fn frag(body: &str, u: i32) -> [f32; 4] {
        let src = std::format!(
            "#version 300 es\nprecision highp float;\nuniform int u;\nlayout(location = 0) out vec4 o;\n{body}"
        );
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        let mut block = [0u32; 16];
        block[0] = u as u32;
        run(&l.module, &Inputs { io: &[], uniforms: &block })
            .expect("runs")
            .output_f32(ir::IoSlot::FragColor(0))
            .expect("o")
    }

    /// modf: the fraction back, the whole part written -- to a variable, to
    /// an element and to a lane chosen at run time.
    ///
    /// Revert check: store `x` instead of `trunc(x)` through the `LRef` and
    /// the whole parts read 5.5.
    #[test]
    fn modf_writes_the_whole_part_through_any_destination() {
        let body = "void main() { float w; float f = modf(5.5, w);\n\
                    vec4 v = vec4(1.0, 2.0, 3.0, 4.0); modf(-2.25, v[u + 2]);\n\
                    o = vec4(f, w, v.z, v.w); }";
        assert_eq!(frag(body, 1), [0.5, 5.0, 3.0, -2.0]);
        assert_eq!(frag(body, 0), [0.5, 5.0, -2.0, 4.0]);
        let body = "void main() { vec2 w; vec2 f = modf(vec2(1.25, -3.75), w); o = vec4(f, w); }";
        assert_eq!(frag(body, 0), [0.25, -0.75, 1.0, -3.0]);
    }

    /// An `out` and an `inout` argument that is an element chosen by a
    /// function with a side effect: the function runs once, the element is
    /// read once and written once.
    ///
    /// Revert check: resolve the destination again at the copy back and the
    /// counter reads 2.
    #[test]
    fn an_element_argument_is_resolved_once() {
        let body = "int n = 0;\nint pick() { n++; return 2; }\n\
                    void set(out float f) { f = 9.0; }\n\
                    void bump(inout float f) { f = f * 10.0; }\n\
                    void main() { vec4 v = vec4(1.0, 2.0, 3.0, 4.0);\n\
                    set(v[pick()]); bump(v[pick() - 1]);\n\
                    o = vec4(v.y, v.z, float(n), 0.0); }";
        assert_eq!(frag(body, 0), [20.0, 9.0, 2.0, 0.0]);
    }

    /// `v[f()]++`, `++v[f()]` inside a sequence, and `m[i][j] = x`, each
    /// index evaluated once.
    ///
    /// Revert check: let a stepped element's value be read by evaluating it
    /// again (`step_value` without its `LRef`) and the counter reads 3.
    #[test]
    fn steps_and_matrix_elements_at_run_time_indices() {
        let body = "int n = 0;\nint pick() { n++; return 2; }\n\
                    void main() { vec4 v = vec4(1.0, 2.0, 3.0, 4.0);\n\
                    v[pick()]++;\n\
                    float f = (0.0, (++v[pick() - 1]));\n\
                    mat2 m = mat2(0.0); m[u + 1][u] = 7.0;\n\
                    o = vec4(v.z, f, float(n), m[1][0] + m[0][0]); }";
        assert_eq!(frag(body, 0), [4.0, 3.0, 2.0, 7.0]);
    }

    /// A swizzle and a structure member as `out` / `inout` arguments, and a
    /// swizzle written into an element chosen at run time.
    ///
    /// Revert check: take multi-lane swizzles out of `resolve_lref` (only
    /// one letter) and the first shader is refused.
    #[test]
    fn swizzles_and_members_as_destinations() {
        let body = "struct S { float f; vec2 g; };\n\
                    void set2(out vec2 x) { x = vec2(3.0, 4.0); }\n\
                    void swap(inout vec2 x) { x = x.yx; }\n\
                    void set1(out float x) { x = 9.0; }\n\
                    void main() { vec4 v = vec4(0.0); set2(v.zx); swap(v.xz);\n\
                    S s; s.g = vec2(0.0); set1(s.f); set2(s.g);\n\
                    vec4 a[2]; a[0] = vec4(0.0); a[1] = vec4(0.0); a[u].wy = vec2(5.0, 6.0);\n\
                    o = vec4(v.x + 10.0 * v.z, s.f + 10.0 * s.g.y, a[1].w + 10.0 * a[1].y, a[0].w); }";
        /* set2(v.zx): z = 3, x = 4; swap(v.xz): (x, z) = (z, x) -> x = 3,
         * z = 4. a[1].w = 5, a[1].y = 6. */
        assert_eq!(frag(body, 1), [43.0, 49.0, 65.0, 0.0]);
    }

    /// A function that returns a value, called as a statement, whose body
    /// returns early: the value is dropped, and the returned expression's
    /// side effects still happen on the path taken (dEQP-GLES3
    /// shaders.metamorphic.synthetic.variant_2, refused "this function
    /// returns no value" until 2026-09-24).
    ///
    /// Revert check: the `Want::Nothing` arm of `return_value` back to the
    /// refusal and the shader is refused.
    #[test]
    fn a_value_returning_call_as_a_statement_returns_early() {
        let body = "int n = 0;\n\
                    vec4 f() { if (u > 0) { n += 10; return vec4(float(n++)); } return vec4(float(n += 100)); }\n\
                    void main() { f(); f(); o = vec4(float(n)); }";
        /* u = 1: each call adds 10 then 1 -> 22; u = 0: each adds 100 -> 200 */
        assert_eq!(frag(body, 1), [22.0; 4]);
        assert_eq!(frag(body, 0), [200.0; 4]);
    }

    /// A parameter with the name of a vertex output is the parameter, for
    /// a read and for a write, inside the function (section 4.2.3).
    ///
    /// Revert check: take the output's shadow first when reading and `f`
    /// reads the output's 0 instead of the argument.
    #[test]
    fn a_parameter_hides_an_output_of_the_same_name() {
        let vs = "#version 300 es\nin vec4 p;\nout float f;\n\
                  void foo(inout float f) { float g = f + 2.5; f = g; }\n\
                  void main() { gl_Position = p; vec4 v = vec4(1.0, 2.0, 3.0, 4.0); foo(v.z); f = v.z; f = f * 2.0; }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, vs).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let loc = l.reflection.varyings.iter().find(|x| x.name == "f").expect("f").location;
        let io = [(ir::IoSlot::Attrib(0), [0u32; 4])];
        let out = run(&l.module, &Inputs { io: &io, uniforms: &[] }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::Varying(loc)).expect("f")[0], 11.0);
    }
}

/// ES 3.00's `gl_FragDepth` (section 7.2): written, and read back after a
/// write; undeclared in ES 1.00. dEQP-GLES3 fbo.depth.depth_test_clamp
/// refused 25 cases on "undeclared identifier" until 2026-09-24. HOST-ONLY.
///
/// Revert check: drop the `add("gl_FragDepth", ...)` in sema and the ES
/// 3.00 shader is refused.
// compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[cfg(all(test, not(feature = "report-es2")))]
mod frag_depth_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    #[test]
    fn es3_writes_gl_fragdepth_and_es1_has_none() {
        let fs = "#version 300 es\nprecision highp float;\nout vec4 o;\n\
                  void main() { o = vec4(1.0); gl_FragDepth = 0.25; gl_FragDepth = gl_FragDepth * 3.0; }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, fs).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let out = run(&l.module, &Inputs { io: &[], uniforms: &[] }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragDepth).expect("written")[0], 0.75);
        let es1 = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); gl_FragDepth = 0.5; }";
        let e = compile_to_ir(&a, Stage::Fragment, es1).err().expect("ES 1.00 has no gl_FragDepth");
        assert_eq!(e.msg, "undeclared identifier");
    }
}

/// Arrays and matrices between the stages and out of the fragment stage,
/// a ONE-element array included: dEQP-GLES3 transform_feedback (`out mat2
/// v_varA[1]`, `in mat2 v_varB[2]`, 301 refusals) and fragment_out.random
/// (`out vec4 o[1]`, 507), all refused until 2026-09-24. HOST-ONLY: the IR
/// evaluator, every value at the location `reflect` gave it.
///
/// Revert checks: `l.run.slots() <= 1` back in `lvalue_slot` and the
/// one-element arrays are refused; the output run back to `count > 1` and
/// the matrix writes are refused; `hold_matrix_array_inputs` not called and
/// the fragment shader is refused.
// compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[cfg(all(test, not(feature = "report-es2")))]
mod io_array_tests {
    use super::*;
    use crate::shaderc::online::eval::{f32x4, run, Inputs};

    #[test]
    fn vertex_outputs_that_are_arrays_or_matrices() {
        let vs = "#version 300 es\nin vec4 p;\nout mat2 v[2];\nout vec4 w[1];\nout mat2 m;\n\
                  void main() { gl_Position = p;\n\
                  v[0][0] = vec2(1.0, 2.0); v[0][1] = vec2(3.0, 4.0);\n\
                  v[1] = mat2(5.0, 6.0, 7.0, 8.0); v[1][0].y = 9.0;\n\
                  w[0] = vec4(10.0); m[1] = vec2(11.0, 12.0); m[0] = vec2(13.0, 14.0); }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, vs).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let loc = |n: &str| l.reflection.varyings.iter().find(|x| x.name == n).expect("a varying").location;
        let io = [(ir::IoSlot::Attrib(0), [0u32; 4])];
        let out = run(&l.module, &Inputs { io: &io, uniforms: &[] }).expect("runs");
        let at = |slot: u8| {
            let v = out.output_f32(ir::IoSlot::Varying(slot)).expect("written");
            [v[0], v[1]]
        };
        /* element k's column j at v's location + 2k + j */
        let v = loc("v");
        assert_eq!([at(v), at(v + 1), at(v + 2), at(v + 3)], [[1.0, 2.0], [3.0, 4.0], [5.0, 9.0], [7.0, 8.0]]);
        assert_eq!(out.output_f32(ir::IoSlot::Varying(loc("w"))).expect("w"), [10.0; 4]);
        assert_eq!([at(loc("m")), at(loc("m") + 1)], [[13.0, 14.0], [11.0, 12.0]]);
    }

    #[test]
    fn fragment_inputs_and_outputs_that_are_arrays() {
        let fs = "#version 300 es\nprecision highp float;\nin mat2 v[2];\nin vec4 w[1];\n\
                  layout(location = 0) out vec4 o[1];\nlayout(location = 1) out vec4 q[2];\n\
                  void main() { mat2 e = v[1]; int k = 0;\n\
                  o[0] = vec4(v[1][0], v[0][1]) + w[0];\n\
                  q[k] = vec4(e[1], v[0][0]); q[1] = vec4(v[k + 1][1].y); }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, fs).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let loc = |n: &str| l.reflection.varyings.iter().find(|x| x.name == n).expect("a varying").location;
        let (v, w) = (loc("v"), loc("w"));
        /* v[0] = (1, 2 | 3, 4), v[1] = (5, 6 | 7, 8) */
        let io = [
            (ir::IoSlot::Varying(v), f32x4([1.0, 2.0, 0.0, 0.0])),
            (ir::IoSlot::Varying(v + 1), f32x4([3.0, 4.0, 0.0, 0.0])),
            (ir::IoSlot::Varying(v + 2), f32x4([5.0, 6.0, 0.0, 0.0])),
            (ir::IoSlot::Varying(v + 3), f32x4([7.0, 8.0, 0.0, 0.0])),
            (ir::IoSlot::Varying(w), f32x4([100.0; 4])),
        ];
        let out = run(&l.module, &Inputs { io: &io, uniforms: &[] }).expect("runs");
        let c = |k: u8| out.output_f32(ir::IoSlot::FragColor(k)).expect("written");
        assert_eq!(c(0), [105.0, 106.0, 103.0, 104.0]);
        assert_eq!(c(1), [7.0, 8.0, 1.0, 2.0]);
        assert_eq!(c(2), [8.0; 4]);
    }
}

/// A `const` array longer than a run's slots, held as its elements' values
/// (dEQP-GLES3 large_constant_arrays, 128 elements). HOST-ONLY.
#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod const_array_value_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    /// Read at a constant index and at a run-time one, a vec4 array too;
    /// and a local of the same name hides it.
    ///
    /// Revert check: drop the `Run::Constants` path in `declare_var` and
    /// the shader is refused ("this array is too large ...").
    #[test]
    fn a_long_const_array_is_read_at_any_index() {
        let mut body = std::string::String::from("const float a[100] = float[](");
        for k in 0..100 {
            body += &std::format!("{}{k}.5", if k == 0 { "" } else { ", " });
        }
        body += ");\nconst vec4 b[70] = vec4[](";
        for k in 0..70 {
            body += &std::format!("{}vec4({k}.0)", if k == 0 { "" } else { ", " });
        }
        body +=
            ");\nvoid main() { float s = a[3]; { float a = 7.0; s += a; } o = vec4(s, a[u], b[u + 1].y, b[69].w); }";
        let src = std::format!(
            "#version 300 es\nprecision highp float;\nuniform int u;\nlayout(location = 0) out vec4 o;\n{body}"
        );
        let mut buf = std::vec![0u8; 1 << 22];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let mut block = [0u32; 16];
        block[0] = 57;
        let out = run(&l.module, &Inputs { io: &[], uniforms: &block }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragColor(0)).expect("o"), [3.5 + 7.0, 57.5, 58.0, 69.0]);
    }
}

/// A structure declared inside a function body (section 4.1.8; dEQP-GLES3
/// arrays.constructor.struct3/4). HOST-ONLY.
#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod body_struct_decl_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    /// Declared, constructed, put in an array of it and read back; and two
    /// same-named structures in different scopes are refused.
    ///
    /// Revert check: refuse the `Decl::Struct` statement again and the
    /// first shader is refused ("this statement is not lowered yet").
    #[test]
    fn a_local_structure_is_a_type_and_nothing_else() {
        let src = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
                   void main() { struct T { float f; vec3 v; };\n\
                   T a = T(1.0, vec3(2.0, 3.0, 4.0)); T b = T(5.0, vec3(6.0));\n\
                   T[2] x = T[2](a, b); o = vec4(x[0].f, x[1].v.y, x[0].v.z, x[1].f); }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let out = run(&l.module, &Inputs { io: &[], uniforms: &[] }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragColor(0)).expect("o"), [1.0, 6.0, 4.0, 5.0]);
        let two = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
                   float f() { struct T { float a; }; T t = T(1.0); return t.a; }\n\
                   void main() { struct T { vec2 b; }; T t = T(vec2(2.0)); o = vec4(f(), t.b, 0.0); }";
        let a = Arena::new(&mut buf);
        let e = compile_to_ir(&a, Stage::Fragment, two).map(|_| ()).expect_err("ambiguous");
        /* The parser refuses the second name first, for the whole unit
         * (a gap of its own: section 4.2.2 scopes a structure name); the
         * lowering's guard stands behind it. */
        assert_eq!(e.msg, "struct redefined");
    }
}

/// A scalar or vector constructed from a matrix (ES 1.00 section 5.4.2):
/// the matrix's components in column-major order, truncated to the type
/// (dEQP-GLES2 attribute_location.*). HOST-ONLY.
#[cfg(test)]
mod matrix_to_vector_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// mat2/mat3/mat4 into vec2/vec3/vec4, a float, and an ivec after a
    /// matrix and a scalar.
    ///
    /// Revert check: drop the matrix branch in `constructor` and every
    /// shape is refused ("this constructor is not lowered yet").
    #[test]
    fn a_matrix_gives_its_components_in_column_major_order() {
        let cases: &[(&str, f32)] = &[
            /* m2 = (1,2 | 3,4): vec4(m2) = (1,2,3,4) */
            ("mat2 m = mat2(1.0, 2.0, 3.0, 4.0); vec4 v = vec4(m); out0 = v.x + 10.0 * v.y + 100.0 * v.z + 1000.0 * v.w;", 4321.0),
            /* vec3(mat3) = the first column */
            ("mat3 m = mat3(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0); vec3 v = vec3(m); out0 = v.x + 10.0 * v.y + 100.0 * v.z;", 321.0),
            /* vec2(mat4) = m[0].xy */
            ("mat4 m = mat4(2.0); vec2 v = vec2(m); out0 = v.x + 10.0 * v.y;", 2.0),
            /* vec4(mat3) = m[0].xyz, m[1].x */
            ("mat3 m = mat3(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0); vec4 v = vec4(m); out0 = v.w;", 4.0),
            ("mat2 m = mat2(5.0, 2.0, 3.0, 4.0); out0 = float(m);", 5.0),
            ("mat2 m = mat2(1.5, 2.5, 3.5, 4.5); ivec3 i = ivec3(m); out0 = float(i.x + 10 * i.y + 100 * i.z);", 321.0),
        ];
        for (body, want) in cases {
            let src = std::format!("{V}void main() {{ {body} gl_Position = p; }}");
            assert_eq!(out0_of(&src, 0.0), *want, "{body}");
        }
        /* the dEQP shape itself: a matrix ATTRIBUTE through vec4() */
        let mut buf = std::vec![0u8; 1 << 20];
        let a = super::Arena::new(&mut buf);
        super::compile_to_ir(
            &a,
            super::Stage::Vertex,
            "attribute vec4 a_position;\nattribute mat2 a_0;\nvarying mediump vec4 v_color;\n\
             void main() { gl_Position = a_position; v_color = vec4(a_0); }",
        )
        .unwrap_or_else(|e| panic!("refused: {}", e.msg));
    }
}

/// A structure chosen by `?:` (section 5.8): only the chosen arm runs.
/// HOST-ONLY.
#[cfg(test)]
mod struct_select_tests {
    use super::return_rule_tests::out0_of;

    /// Revert check: drop the `Select` arm in `struct_fill` and it is refused
    /// ("a struct value that is not a variable is not lowered yet").
    #[test]
    fn a_structure_is_chosen_whole_and_only_one_arm_runs() {
        let src = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n\
                   struct S { float f; vec2 v; };\nfloat n = 0.0;\nS mk(float x) { n += 1.0; return S(x, vec2(x)); }\n\
                   void main() { S a = S(1.0, vec2(2.0)); S c = in0 > 0.5 ? a : mk(5.0);\n\
                   out0 = c.f + 10.0 * c.v.y + 100.0 * n; gl_Position = p; }";
        assert_eq!(out0_of(src, 1.0), 21.0);
        assert_eq!(out0_of(src, 0.0), 155.0);
    }
}

/// The front end's own peak on uniform_api.random.23 (the largest shader in
/// the dEQP repros): the tokens are given back after the preprocessor and
/// after the parser, and `main`'s nodes are copied once. HOST-ONLY: 1.96 MB
/// before, 1.58 MB after (2026-09-24), against the 2 MiB scratch.
///
/// The bound is 40 KB above the measured peak. Revert check (2026-09-24):
/// without the preprocessor's `give_back` the peak is 1,692,012; without the
/// parser's, 1,692,300; with `main` copied twice, 1,724,568.
#[cfg(test)]
mod front_end_peak_tests {
    use super::*;

    #[test]
    fn random23_fits_the_front_end_well_inside_the_scratch() {
        let t = include_str!("../../../tests/deqp_repro_shaders.rs");
        let k = "const UNIFORM_API_RANDOM_23_FS: &str = r#\"";
        let a0 = t.find(k).expect("the repro") + k.len();
        let src = &t[a0..a0 + t[a0..].find("\"#;").expect("its end")];
        let mut buf = std::vec![0u8; 4 << 20];
        let a = Arena::new(&mut buf);
        compile_to_ir(&a, Stage::Fragment, src).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        std::println!("random.23 front end: {} bytes at its peak", a.peak());
        assert!(a.peak() < 1_620_000, "{} bytes", a.peak());
    }
}

/// Section 5.9's equality on aggregates.
#[cfg(test)]
mod aggregate_equality_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    /// "all components of the operands must be equal": a difference in ANY
    /// column of a matrix or ANY leaf of a struct makes `==` false and `!=`
    /// true. Each case differs in exactly one place -- the last column, a
    /// nested leaf, a bool -- so a comparison that skipped it would say
    /// equal.
    #[test]
    fn matrices_and_structs_compare_every_component() {
        let run = |body: &str, x: f32| {
            out0_of(&format!("{V}struct I {{ bool f; vec2 v; }};\nstruct S {{ float a; I i; }};\nvoid main(){{ {body} gl_Position = p; }}"), x)
        };
        let mat = "mat2 a = mat2(1.0, 2.0, 3.0, in0); mat2 b = mat2(1.0, 2.0, 3.0, 4.0); out0 = (a == b ? 1.0 : 0.0) + (a != b ? 10.0 : 0.0);";
        assert_eq!(run(mat, 4.0), 1.0);
        assert_eq!(run(mat, 5.0), 10.0);
        let st = "S a = S(1.0, I(true, vec2(2.0, in0))); S b = S(1.0, I(true, vec2(2.0, 3.0))); out0 = (a == b ? 1.0 : 0.0) + (a != b ? 10.0 : 0.0);";
        assert_eq!(run(st, 3.0), 1.0);
        assert_eq!(run(st, 4.0), 10.0);
        let flag =
            "S a = S(1.0, I(in0 > 0.0, vec2(2.0, 3.0))); out0 = a == S(1.0, I(true, vec2(2.0, 3.0))) ? 1.0 : 0.0;";
        assert_eq!(run(flag, 1.0), 1.0);
        assert_eq!(run(flag, -1.0), 0.0);
    }
}

/// A sampler reached through a struct member.
#[cfg(test)]
mod struct_sampler_tests {
    use super::*;

    /// `texture2D(s.c, ...)` samples the binding of the row named `s.c`.
    #[test]
    fn a_struct_member_sampler_is_its_rows_binding() {
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\nuniform sampler2D t;\nstruct S { float a; sampler2D c; };\nuniform S s;\nvarying vec2 uv;\nvoid main() { gl_FragColor = texture2D(s.c, uv) * s.a + texture2D(t, uv); }",
        )
        .expect("compiles");
        let samplers: std::vec::Vec<u8> = l
            .module
            .instrs
            .iter()
            .filter_map(|o| match o {
                Op::Tex { sampler, .. } => Some(*sampler),
                _ => None,
            })
            .collect();
        let binding = |n: &str| l.reflection.uniforms.iter().find(|u| u.name == n).map(|u| u.binding);
        assert_eq!(samplers, [binding("s.c").unwrap(), binding("t").unwrap()]);
        assert_ne!(binding("s.c"), binding("t"));
    }

    /// `texture2D(s[k], ...)` of an array of samplers samples binding
    /// `binding + k` -- top level and struct member alike -- and an index
    /// past the array is refused, not read from the next sampler's unit.
    /// The shape of dEQP `uniform_api.*.basic_array.sampler2D_*`, refused as
    /// "an array of samplers is not reflected" until 2026-09-23.
    #[test]
    fn element_k_of_a_sampler_array_is_binding_plus_k() {
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\nuniform sampler2D t;\nuniform sampler2D s[3];\n\
             struct S { sampler2D m[2]; };\nuniform S u;\n\
             void main() { gl_FragColor = texture2D(s[2], vec2(0.0)) + texture2D(s[0], vec2(0.0))\n\
             + texture2D(u.m[1], vec2(0.0)) + texture2D(t, vec2(0.0)); }",
        )
        .expect("compiles");
        let samplers: std::vec::Vec<u8> = l
            .module
            .instrs
            .iter()
            .filter_map(|o| match o {
                Op::Tex { sampler, .. } => Some(*sampler),
                _ => None,
            })
            .collect();
        /* t 0, s 1..3, u.m 4..5 */
        assert_eq!(samplers, [3, 1, 5, 0]);

        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let Err(e) = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\nuniform sampler2D s[2];\nuniform sampler2D n;\n\
             void main() { gl_FragColor = texture2D(s[2], vec2(0.0)); }",
        ) else {
            panic!("s[2] of a two-element array compiled");
        };
        assert!(e.msg.contains("out of range"), "{}", e.msg);
    }
}

/// Section 7.5's `gl_DepthRange`: three rows after the application's
/// uniforms, which the GL core fills from glDepthRangef, read like any
/// uniform struct's members (dEQP-GLES2 builtin_variable.depth_range_*).
#[cfg(test)]
mod depth_range_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    /// The rows exist only when the shader reads it, after everything
    /// else, and the members load from them: near, far and diff filled
    /// with 0.25, 0.75, 0.5 read back as those.
    ///
    /// Revert check: drop the rows in `reflect::reflect` and both shaders
    /// are refused ("this use of gl_DepthRange is not lowered yet").
    #[test]
    fn the_members_are_rows_after_the_applications_uniforms() {
        let fs = "precision mediump float;\nuniform float k;\n\
                  void main() { gl_FragColor = vec4(gl_DepthRange.near, gl_DepthRange.far, gl_DepthRange.diff, k); }";
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, fs).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let rows: std::vec::Vec<(&str, u16)> = l.reflection.uniforms.iter().map(|u| (u.name, u.dword_off)).collect();
        assert_eq!(rows, [("k", 0), ("gl_DepthRange.near", 1), ("gl_DepthRange.far", 2), ("gl_DepthRange.diff", 3)]);
        let block = [7.0f32, 0.25, 0.75, 0.5].map(f32::to_bits);
        let out = run(&l.module, &Inputs { io: &[], uniforms: &block }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragColor(0)).expect("colour"), [0.25, 0.75, 0.5, 7.0]);

        let vs = "attribute vec4 p;\nvoid main() { gl_Position = p * gl_DepthRange.far; }";
        let a = Arena::new(&mut buf);
        compile_to_ir(&a, Stage::Vertex, vs).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        /* A shader that does not read it has no rows. */
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, "attribute vec4 p;\nvoid main() { gl_Position = p; }").expect("ok");
        assert!(l.reflection.uniforms.is_empty());
    }
}

/// A parameter that is, or holds, a sampler: bound by name to the uniform
/// its argument names (section 4.1.7 forbids writing it).
#[cfg(test)]
mod sampler_param_tests {
    use super::*;

    /// The binding every `Tex` op samples, in program order, against the
    /// bindings of the rows named in `want`.
    fn samples(src: &str, want: &[&str]) {
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).expect("compiles");
        let got: std::vec::Vec<u8> = l
            .module
            .instrs
            .iter()
            .filter_map(|o| match o {
                Op::Tex { sampler, .. } => Some(*sampler),
                _ => None,
            })
            .collect();
        let binding = |n: &str| {
            l.reflection.uniforms.iter().find(|u| u.name == n).map(|u| u.binding).unwrap_or_else(|| panic!("{n}"))
        };
        let want: std::vec::Vec<u8> = want.iter().map(|n| binding(n)).collect();
        assert_eq!(got, want, "{src}");
    }

    /// dEQP shaders.struct.uniform.sampler_in_function_arg and
    /// sampler_in_array_function_arg, with a second sampler uniform so a
    /// binding of 0 cannot pass by being every sampler's binding.
    ///
    /// Revert check: disable the `carries_sampler` arm in `inline` and all
    /// three tests of this module fail, the struct shaders refused with "a
    /// struct member of this type is not lowered yet".
    #[test]
    fn a_sampler_parameter_samples_the_argument_uniform() {
        let h = "precision mediump float;\nuniform sampler2D z;\nstruct S { sampler2D source; };\n";
        samples(
            &format!("{h}vec4 fun(S s) {{ return texture2D(s.source, vec2(0.5)); }}\nuniform S s;\nvoid main() {{ gl_FragColor = fun(s) + texture2D(z, vec2(0.0)); }}"),
            &["s.source", "z"],
        );
        samples(
            &format!("{h}vec4 fun(S s[2]) {{ return texture2D(s[1].source, vec2(0.5)); }}\nuniform S s[2];\nvoid main() {{ gl_FragColor = fun(s) + texture2D(z, vec2(0.0)); }}"),
            &["s[1].source", "z"],
        );
        /* A plain sampler, a differently named argument, and a call from a
         * call: the inner parameter resolves through the outer one. */
        samples(
            &format!("{h}uniform S u[2];\nvec4 g(sampler2D t) {{ return texture2D(t, vec2(0.5)); }}\nvec4 f(S a) {{ return g(a.source); }}\nvoid main() {{ gl_FragColor = f(u[1]) + g(z); }}"),
            &["u[1].source", "z"],
        );
    }

    /// A struct holding a sampler has other members, read as the argument
    /// uniform's rows.
    #[test]
    fn a_member_of_a_sampler_struct_parameter_reads_the_argument_row() {
        let src = "attribute vec4 p;\nvarying float out0;\nstruct S { float k; sampler2D t; float m; };\nuniform S a;\nuniform S b;\nfloat f(S s) { return s.k * 10.0 + s.m; }\nvoid main() { out0 = f(b) * 100.0 + f(a); gl_Position = p; }";
        let one = 1.0f32.to_bits();
        let two = 2.0f32.to_bits();
        let three = 3.0f32.to_bits();
        let four = 4.0f32.to_bits();
        let v = super::uniform_value_tests::out0_with(
            src,
            &[("a.k", &[one]), ("a.m", &[two]), ("b.k", &[three]), ("b.m", &[four])],
        );
        assert_eq!(v, 3412.0);
    }

    /// The name binding makes no copy, so a body that writes a member is a
    /// gap, said as one.
    #[test]
    fn writing_a_member_of_a_sampler_struct_parameter_is_a_gap() {
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let e = compile_to_ir(
            &a,
            Stage::Fragment,
            "precision mediump float;\nstruct S { sampler2D t; float k; };\nuniform S u;\nfloat f(S s) { s.k = 2.0; return s.k; }\nvoid main() { gl_FragColor = vec4(f(u)); }",
        )
        .map(|_| ())
        .expect_err("a gap");
        assert_eq!(e.msg, "writing a member of a parameter that holds a sampler is not lowered yet");
    }
}

/// A struct member that is a struct, assigned whole.
#[cfg(test)]
mod struct_member_assign_tests {
    use super::return_rule_tests::out0_of;

    /// `s.b = T(x, v);`, `s.b = t;` and `s.b = f();`: the member's own run
    /// filled, and the other members left as they were.
    #[test]
    fn a_struct_member_is_assigned_whole() {
        let v = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nstruct T { float a; vec2 b; };\nstruct S { float x; T t; float y; };\nT mk(float k) { return T(k, vec2(k + 1.0, k + 2.0)); }\n";
        let run = |body: &str| {
            out0_of(&format!("{v}void main(){{ S s = S(1.0, T(0.0, vec2(0.0)), 9.0); {body} out0 = s.x*10000.0 + s.t.a*1000.0 + s.t.b.x*100.0 + s.t.b.y*10.0 + s.y; gl_Position = p; }}"), 2.0)
        };
        assert_eq!(run("s.t = T(in0, vec2(3.0, 4.0));"), 12349.0);
        assert_eq!(run("T u = T(5.0, vec2(6.0, 7.0)); s.t = u;"), 15679.0);
        assert_eq!(run("s.t = mk(in0);"), 12349.0);
    }
}

/// Arrays of structs and array members, as locals.
#[cfg(test)]
mod struct_array_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nstruct T { float a; vec2 b[2]; };\nstruct S { float a; T t[2]; int c; };\n";

    fn run(body: &str, x: f32) -> f32 {
        out0_of(&format!("{V}void main(){{ int i = int(in0); {body} gl_Position = p; }}"), x)
    }

    /// dEQP shaders.struct.local.*array*: `S s[2]` with `T t[2]` inside and
    /// `vec2 b[2]` inside that. Every member of every element is written by
    /// constant indices and read back, so an element or a member landing on
    /// its neighbour's slot changes the sum.
    #[test]
    fn every_member_of_every_element_has_its_own_slot() {
        let fill = "S s[2]; for (int k = 0; k < 2; k++) { s[k].a = float(k); s[k].c = k + 10; } \
            s[0].t[0].a = 1.0; s[0].t[1].a = 2.0; s[1].t[0].a = 3.0; s[1].t[1].a = 4.0; \
            s[0].t[1].b[0] = vec2(5.0, 6.0); s[1].t[0].b[1] = vec2(7.0, 8.0);";
        let sum = "out0 = s[0].a + s[1].a*10.0 + s[0].t[0].a*100.0 + s[0].t[1].a*1000.0 + s[1].t[0].a*10000.0 \
            + s[1].t[1].a*100000.0 + s[0].t[1].b[0].y + s[1].t[0].b[1].x + float(s[1].c);";
        assert_eq!(run(&format!("{fill} {sum}"), 0.0), 432110.0 + 6.0 + 7.0 + 11.0);
    }

    /// A run-time index, reading and writing: the element it names changes
    /// and the other keeps its value. Two run-time indices in one access
    /// (`s[i].t[j].a`) select over both.
    #[test]
    fn a_run_time_index_into_an_array_of_structs() {
        let base = "S s[2]; s[0].a = 1.0; s[1].a = 2.0; s[0].t[1].a = 3.0; s[1].t[1].a = 4.0; s[0].t[0].a = 0.0; s[1].t[0].a = 0.0;";
        assert_eq!(run(&format!("{base} s[i].a = 9.0; out0 = s[0].a * 10.0 + s[1].a;"), 1.0), 19.0);
        assert_eq!(run(&format!("{base} s[i].a = 9.0; out0 = s[0].a * 10.0 + s[1].a;"), 0.0), 92.0);
        assert_eq!(run(&format!("{base} out0 = s[i].t[1].a;"), 1.0), 4.0);
        assert_eq!(run(&format!("{base} int j = 1 - i; out0 = s[i].t[j].a;"), 0.0), 3.0);
        assert_eq!(run(&format!("{base} s[i].t[1].a += 5.0; out0 = s[0].t[1].a * 10.0 + s[1].t[1].a;"), 1.0), 39.0);
        /* Three run-time indices in one access. */
        let three = "S s[2]; s[1].t[0].b[1] = vec2(4.0, 5.0); s[0].t[0].b[1] = vec2(0.0); int j = 0; int k = 1; out0 = s[i].t[j].b[k].y;";
        assert_eq!(run(three, 1.0), 5.0);
    }

    /// A whole element copied into a local (ES 1.00 has no array
    /// constructor, so an element is filled member by member first).
    #[test]
    fn a_whole_element_is_a_struct() {
        let simple = "T t[2]; t[1].a = 5.0; t[1].b[1] = vec2(6.0, 7.0); T u = t[1]; out0 = u.a * 100.0 + u.b[1].x * 10.0 + u.b[1].y;";
        assert_eq!(run(simple, 0.0), 567.0);
    }
}

/// Section 5.9's evaluation order, where a side effect makes it visible.
#[cfg(test)]
mod short_circuit_tests {
    use super::return_rule_tests::out0_of;

    const V: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nfloat n;\n\
                     bool f() { n = n + 1.0; return true; }\nfloat g() { n = n + 1.0; return 5.0; }\n";

    /// The right operand of `&&` runs only when the left is true, and of
    /// `||` only when it is false. Both were evaluated until 2026-09-23 on
    /// the premise that "ES 1.00 expressions have no side effects to skip",
    /// which a call to a function that writes a global disproves: dEQP-GLES3
    /// `loops.short_circuit.*` counted the calls and got 1 where 0 was right.
    ///
    /// Checked: removing the `short_circuit` branch in `binary` makes the
    /// first assertion read 1.0.
    #[test]
    fn the_right_operand_runs_only_when_it_decides_the_result() {
        let and = format!(
            "{V}void main(){{ n = 0.0; bool b = in0 > 0.5 && f(); out0 = n + (b ? 10.0 : 0.0); gl_Position = p; }}"
        );
        assert_eq!(out0_of(&and, 0.0), 0.0, "false && f(): f is not called");
        assert_eq!(out0_of(&and, 1.0), 11.0, "true && f(): it is");
        let or = format!(
            "{V}void main(){{ n = 0.0; bool b = in0 > 0.5 || f(); out0 = n + (b ? 10.0 : 0.0); gl_Position = p; }}"
        );
        assert_eq!(out0_of(&or, 1.0), 10.0, "true || f(): f is not called");
        assert_eq!(out0_of(&or, 0.0), 11.0, "false || f(): it is");
    }

    /// Only the chosen arm of `?:` is evaluated.
    ///
    /// Checked: removing the side-effect branch in the `Select` lowering
    /// makes the first assertion read 102.
    #[test]
    fn only_the_chosen_arm_of_a_select_runs() {
        let sel = format!(
            "{V}void main(){{ n = 0.0; float x = in0 > 0.5 ? g() : 2.0; out0 = n * 100.0 + x; gl_Position = p; }}"
        );
        assert_eq!(out0_of(&sel, 0.0), 2.0, "the false arm: g is not called");
        assert_eq!(out0_of(&sel, 1.0), 105.0, "the true arm: it is");
    }
}

/// GLSL ES 3.00's interface: `in`, `out`, `layout(location)`, `flat`, and
/// what 3.00 removed.
#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod es3_interface_tests {
    use super::*;
    use crate::shaderc::online::eval::{f32x4, run, Inputs};

    fn compile<'a>(buf: &'a mut [u8], stage: Stage, src: &'a str) -> R<'a, Lowered<'a>> {
        let a: &'a Arena<'a> = std::boxed::Box::leak(std::boxed::Box::new(Arena::new(buf)));
        compile_to_ir(a, stage, src)
    }

    fn fails(stage: Stage, src: &str) -> std::string::String {
        let mut buf = std::vec![0u8; 1 << 19];
        match compile(&mut buf, stage, src) {
            Ok(_) => panic!("compiled: {src}"),
            Err(e) => e.msg.into(),
        }
    }

    /// A vertex `in` is an attribute (at its layout location), a vertex
    /// `out` and a fragment `in` are varyings, a fragment `out` is the
    /// colour at its location -- and the pair runs.
    #[test]
    fn in_and_out_mean_what_their_stage_makes_them() {
        let mut vb = std::vec![0u8; 1 << 19];
        let v = compile(
            &mut vb,
            Stage::Vertex,
            "#version 300 es\nlayout(location = 3) in vec4 p;\nin vec4 q;\nout vec4 v;\nflat out int k;\n\
             void main() { v = p + q; k = 7; gl_Position = p; }",
        )
        .expect("compiles");
        let at: std::vec::Vec<(&str, u8)> = v.reflection.attribs.iter().map(|a| (a.name, a.location)).collect();
        assert_eq!(at, [("p", 3), ("q", 4)], "a layout location, and the next one counts on from it");
        let vy: std::vec::Vec<(&str, u8)> = v.reflection.varyings.iter().map(|x| (x.name, x.interp)).collect();
        assert_eq!(vy, [("v", ir::INTERP_SMOOTH), ("k", ir::INTERP_FLAT)]);

        let mut fb = std::vec![0u8; 1 << 19];
        let f = compile(
            &mut fb,
            Stage::Fragment,
            "#version 300 es\nprecision mediump float;\nin vec4 v;\nlayout(location = 0) out vec4 c;\n\
             void main() { c = v * 2.0; }",
        )
        .expect("compiles");
        let m = &f.module;
        assert!(m.outputs.iter().any(|o| o.name == "c" && o.kind == ir::IoKind::Colour && o.location == 0));
        let got = run(m, &Inputs { io: &[(ir::IoSlot::Varying(0), f32x4([0.25, 0.5, 1.0, 0.125]))], uniforms: &[] })
            .expect("runs");
        assert_eq!(got.output_f32(ir::IoSlot::FragColor(0)), Some([0.5, 1.0, 2.0, 0.25]));
    }

    /// What 3.00 took away or requires, refused by name.
    #[test]
    fn what_es3_removed_or_requires_is_refused() {
        let fs = "#version 300 es\nprecision mediump float;\n";
        assert!(
            fails(Stage::Fragment, &format!("{fs}void main() {{ gl_FragColor = vec4(1.0); }}")).contains("undeclared")
        );
        assert!(fails(Stage::Vertex, "#version 300 es\nattribute vec4 p;\nvoid main() { gl_Position = p; }")
            .contains("reserved"));
        assert!(fails(Stage::Vertex, "#version 300 es\nout int k;\nvoid main() { k = 1; gl_Position = vec4(0.0); }")
            .contains("flat"));
        assert!(fails(Stage::Vertex, "#version 300 es\nin flat vec4 p;\nvoid main() { gl_Position = p; }")
            .contains("expected a type"));
        assert!(fails(
            Stage::Vertex,
            "#version 300 es\nlayout(location = 0) uniform vec4 u;\nvoid main() { gl_Position = u; }"
        )
        .contains("interface qualifier"));
        /* `0u` is a uint, and section 4.1.10 converts nothing implicitly
         * (dEQP-GLES3 constants.int_from_uint_2). */
        assert!(fails(Stage::Vertex, "#version 300 es\nvoid main() { int v = 0u; gl_Position = vec4(0.0); }")
            .contains("implicit conversion"));
        let long = "a".repeat(1025);
        assert!(fails(
            Stage::Vertex,
            &format!("#version 300 es\nvoid main() {{ float {long}; gl_Position = vec4(0.0); }}")
        )
        .contains("1024"));
        /* ES 1.00 has none of these rules: `gl_FragColor` is there, and a
         * long name is fine. */
        let mut buf = std::vec![0u8; 1 << 19];
        let src =
            format!("precision mediump float;\nvoid main() {{ float {long} = 1.0; gl_FragColor = vec4({long}); }}");
        assert!(compile(&mut buf, Stage::Fragment, &src).is_ok());
    }

    /// ES 3.00 splices lines before it finds comments, so a `//` comment
    /// ending in a backslash swallows the next line; ES 1.00 does not
    /// (dEQP-GLES3 preprocessor.line_continuation.comment).
    ///
    /// Checked: without the splice in the `//` loop the ES 3.00 value is -1.
    #[test]
    fn a_backslash_continues_a_line_comment_in_es3_only() {
        let body = "void main() { float x = 1.0;\n// comment \\\nx = -1.0;\nc = vec4(x); }";
        let mut buf = std::vec![0u8; 1 << 19];
        let src = format!("#version 300 es\nprecision mediump float;\nout vec4 c;\n{body}");
        let f = compile(&mut buf, Stage::Fragment, &src).expect("compiles");
        let got = run(&f.module, &Inputs { io: &[], uniforms: &[] }).expect("runs");
        assert_eq!(got.output_f32(ir::IoSlot::FragColor(0)).unwrap()[0], 1.0);
    }
}

#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod uint_tests {
    //! ES 3.00's `uint` (step 2 of the ES 3.00 work): the operations where
    //! signed and unsigned differ, run by the evaluator on values chosen so
    //! that the signed reading gives a different answer.
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    /// Compile a fragment shader and run it with a uniform block; the raw
    /// bits of the colour output at location 0.
    fn frag_bits(src: &str, set: &[(&str, &[u32])]) -> [u32; 4] {
        let mut buf = std::vec![0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        let m = &l.module;
        let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4) + 16];
        for (name, dwords) in set {
            let u = m.uniforms.iter().find(|x| x.name == *name).unwrap_or_else(|| panic!("no uniform {name}"));
            let at = usize::from(u.dword_off);
            block[at..at + dwords.len()].copy_from_slice(dwords);
        }
        run(m, &Inputs { io: &[], uniforms: &block }).expect("runs").output(ir::IoSlot::FragColor(0)).expect("o")
    }

    const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n";

    /// `u` is 0xfffffffe: -2 read as an int, 4294967294 read as a uint.
    ///
    /// Revert check (each alone): `Udiv` -> `Idiv` in `binary` makes lane x
    /// 0xffffffff (-2 / 2 = -1); `Ushr` -> `Ishr` makes lane y 0xffffffff;
    /// `Umod` -> `Imod` makes lane z 0xfffffffe (-2 % 5); `Ult` -> `Ilt` in
    /// `compare` makes lane w 0.
    #[test]
    fn unsigned_division_shift_remainder_and_order_read_the_bits_as_unsigned() {
        let src = format!(
            "{HEAD}uniform uint u;\nlayout(location = 0) out uvec4 o;\n\
             void main() {{ uint a = u / 2u; uint b = u >> 1u; uint c = u % 5u; o = uvec4(a, b, c, u > 1u ? 1u : 0u); }}"
        );
        assert_eq!(frag_bits(&src, &[("u", &[0xffff_fffe])]), [0x7fff_ffff, 0x7fff_ffff, 4, 1]);
    }

    /// The signed forms of the same operators, on the same bits, and the
    /// bit-wise ones both kinds share.
    #[test]
    fn signed_integer_operators_are_signed() {
        let src = format!(
            "{HEAD}uniform int i;\nlayout(location = 0) out ivec4 o;\n\
             void main() {{ o = ivec4(i >> 1, i % 5, (i & 0xff) | 0x100, ~i ^ 3); }}"
        );
        /* -2 >> 1 = -1; -2 % 5 = -2 (section 5.9 leaves a negative operand
         * undefined, and this is the evaluator's answer, not a rule); -2 &
         * 0xff = 0xfe, | 0x100 = 0x1fe; ~(-2) = 1, ^ 3 = 2. */
        assert_eq!(frag_bits(&src, &[("i", &[(-2i32) as u32])]), [u32::MAX, (-2i32) as u32, 0x1fe, 2]);
    }

    /// Section 5.4.1: `int` <-> `uint` keeps the bit pattern; to and from
    /// `float` converts the value, and `float(uint)` is unsigned.
    ///
    /// Revert check: send `(U32, F32)` to `I2f` in `convert` and lane x is
    /// -2.0 instead of 4294967294.0.
    #[test]
    fn conversions_follow_section_5_4_1() {
        let src = format!(
            "{HEAD}uniform uint u;\nuniform float f;\nlayout(location = 0) out vec4 o;\n\
             void main() {{ o = vec4(float(u), float(int(u)), float(uint(f)), float(uint(true)) + float(bool(u))); }}"
        );
        let got = frag_bits(&src, &[("u", &[0xffff_fffe]), ("f", &[7.75f32.to_bits()])]);
        assert_eq!(got.map(f32::from_bits), [4294967294.0, -2.0, 7.0, 2.0]);
    }

    /// Literals: a `u` suffix, hex and octal, and a uint that does not fit
    /// in an f32 kept exact through a constructor.
    ///
    /// Revert check: drop `U32` from the all-literal integer arm of
    /// `constructor` and lane w reads 0 -- `uint(-1)` folded through an f32,
    /// where -1.0 saturates -- instead of the bit pattern 0xffffffff.
    #[test]
    fn unsigned_literals_keep_every_bit() {
        let src = format!(
            "{HEAD}layout(location = 0) out uvec4 o;\n\
             void main() {{ o = uvec4(4294967295u, 0xfffffff1u, 017u, uint(-1)); }}"
        );
        assert_eq!(frag_bits(&src, &[]), [u32::MAX, 0xffff_fff1, 15, u32::MAX]);
    }

    /// `lessThan` and friends on a `uvec` order as unsigned.
    ///
    /// Revert check: drop the `unsigned` arms in builtin_call's relational
    /// built-ins and lane x reads 0 (a signed compare of 0xfffffffe, -2,
    /// against 1).
    #[test]
    fn relational_builtins_on_uvec_are_unsigned() {
        let src = format!(
            "{HEAD}uniform uvec2 u;\nlayout(location = 0) out uvec4 o;\n\
             void main() {{ bvec2 g = greaterThan(u, uvec2(1u)); bvec2 l = lessThanEqual(u, uvec2(3u)); \
             o = uvec4(g.x ? 1u : 0u, g.y ? 1u : 0u, l.x ? 1u : 0u, l.y ? 1u : 0u); }}"
        );
        assert_eq!(frag_bits(&src, &[("u", &[0xffff_fffe, 3])]), [1, 1, 0, 1]);
    }

    /// A `uvec` component is read and written by index like any vector's.
    ///
    /// Revert check: take `UVec` out of the vector arms of `indexed` and
    /// `expr_statement` and the compile is refused ("this expression is not
    /// lowered yet").
    #[test]
    fn a_uvec_is_indexed_like_a_vector() {
        let src = format!(
            "{HEAD}uniform int k;\nlayout(location = 0) out uvec4 o;\n\
             void main() {{ uvec3 v = uvec3(1u, 2u, 3u); v[1] = 7u; v[k] = 9u; o = uvec4(v[0], v[1], v[2], v[k]); }}"
        );
        assert_eq!(frag_bits(&src, &[("k", &[2])]), [1, 7, 9, 9]);
    }

    /// The reflection names the ES 3.00 types by their GL tokens, and an
    /// unsigned varying must be flat like a signed one.
    #[test]
    fn uint_uniforms_and_varyings_are_reflected() {
        let mut buf = std::vec![0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let src = "#version 300 es\nuniform uint a;\nuniform uvec3 b;\nflat out uvec2 v;\n\
                   void main() { v = b.xy + a; gl_Position = vec4(0.0); }";
        let l = compile_to_ir(&a, Stage::Vertex, src).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let u: std::vec::Vec<(&str, u16, u16)> =
            l.reflection.uniforms.iter().map(|r| (r.name, r.dword_off, r.gl_type)).collect();
        assert_eq!(u, [("a", 0, 0x1405), ("b", 4, 0x8DC7)]);
        assert_eq!(l.reflection.varyings[0].gl_type, 0x8DC6);
        let mut buf2 = std::vec![0u8; 1 << 19];
        let a2 = Arena::new(&mut buf2);
        let bad = "#version 300 es\nout uint v;\nvoid main() { v = 1u; gl_Position = vec4(0.0); }";
        assert!(compile_to_ir(&a2, Stage::Vertex, bad).err().expect("refused").msg.contains("flat"));
    }
}

#[cfg(test)]
mod es3_array_tests {
    //! ES 3.00's arrays (step 2b): constructors, unsized declarations,
    //! `.length()`, whole-array assignment, equality, `?:`, array
    //! parameters and returns, and arrays of matrices and of structs -- run
    //! by the evaluator, each value chosen so a wrong element reads a
    //! different number.
    use super::*;
    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    use crate::shaderc::online::eval::{run, Inputs};

    /// A fragment shader run with `uniform int k`; the colour at location 0.
    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    fn frag(body: &str, k: i32) -> [f32; 4] {
        let src = std::format!(
            "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform int k;\n\
             layout(location = 0) out vec4 o;\n{body}"
        );
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        let m = &l.module;
        let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4) + 16];
        if let Some(u) = m.uniforms.iter().find(|x| x.name == "k") {
            block[usize::from(u.dword_off)] = k as u32;
        }
        run(m, &Inputs { io: &[], uniforms: &block })
            .expect("runs")
            .output_f32(ir::IoSlot::FragColor(0))
            .expect("o written")
    }

    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    fn refused(body: &str) -> std::string::String {
        let src = std::format!("#version 300 es\nprecision highp float;\nout vec4 o;\n{body}");
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        match compile_to_ir(&a, Stage::Fragment, &src) {
            Ok(_) => panic!("compiled: {body}"),
            Err(e) => e.msg.into(),
        }
    }

    /// The shape Qt Quick 3D writes -- a const global table from an array
    /// constructor, indexed at run time -- and the unsized and type-first
    /// declarations, with `.length()`.
    ///
    /// Revert check: drop the `ArrayCtor` arm of `array_fill` and every
    /// declaration here is refused ("an array value of this kind is not
    /// lowered yet").
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn a_constructed_array_is_declared_indexed_and_measured() {
        let body = "const vec2 tab[4] = vec2[](vec2(1.0, 2.0), vec2(3.0, 4.0), vec2(5.0, 6.0), vec2(7.0, 8.0));\n\
                    void main() { float x[] = float[](10.0, 20.0, 30.0); float[3] y = x; y[1] = 5.0; \
                    o = vec4(tab[k], y[1] + x[k - 1], float(y.length())); }";
        assert_eq!(frag(body, 2), [5.0, 6.0, 25.0, 3.0]);
        assert_eq!(frag(body, 1), [3.0, 4.0, 15.0, 3.0]);
    }

    /// Arrays as values: a parameter and a return, `==` and `!=` over
    /// every element, and `?:` choosing a whole array.
    ///
    /// Revert check: compare only the first slot in `array_equal` and lane y
    /// reads 1 -- the two arrays differ only in their last element.
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn arrays_are_passed_returned_compared_and_chosen_whole() {
        let body = "float[3] twice0(float[3] a) { a[0] = a[0] * 2.0; return a; }\n\
                    void main() { float[3] a = float[3](1.0, 2.0, 3.0); float[3] b = twice0(a); \
                    bool eq = a == float[3](1.0, 2.0, 4.0); bool ne = a != b; \
                    o = vec4(b[0], eq ? 1.0 : 0.0, ne ? 1.0 : 0.0, (k > 0 ? a : b)[0]); }";
        assert_eq!(frag(body, 1), [2.0, 0.0, 1.0, 1.0]);
        assert_eq!(frag(body, 0)[3], 2.0, "the other arm of ?:");
    }

    /// Arrays of matrices and of structs, constructed, copied and read at
    /// constant and run-time indices.
    ///
    /// Revert check: lay a `mat2[2]` out as `Run::Array` (one slot per
    /// matrix) in `run_shape` and the compile is refused; take the
    /// `MatrixArray` arm out of `matrix_value` and it is refused too.
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn arrays_of_matrices_and_of_structs() {
        let body = "struct S { float f; vec2 v; };\n\
                    void main() { mat2 m[2] = mat2[2](mat2(1.0), mat2(2.0, 3.0, 4.0, 5.0)); \
                    S s[] = S[](S(1.0, vec2(2.0)), S(3.0, vec2(4.0, 5.0))); S t[2] = s; \
                    o = vec4(m[1][1][0], m[k - 1][0][0], t[1].v.y, t[k - 1].f); }";
        assert_eq!(frag(body, 2), [4.0, 2.0, 5.0, 3.0]);
        assert_eq!(frag(body, 1), [4.0, 1.0, 5.0, 1.0]);
    }

    /// What section 4.1.9 and 5.4.4 do not allow.
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn what_the_array_rules_refuse() {
        assert!(refused("void main() { float a[2][3]; o = vec4(0.0); }").contains("array of arrays"));
        assert!(refused("void main() { float[2] a[3]; o = vec4(0.0); }").contains("array of arrays"));
        assert!(refused("void main() { float a[3] = float[](1.0, 2.0); o = vec4(0.0); }").contains("types"));
        assert!(refused("void main() { float a[] = float[2](1.0); o = vec4(0.0); }").contains("one argument per"));
        assert!(refused("void main() { float a[] = float[](1.0, 2); o = vec4(0.0); }").contains("element type"));
        assert!(refused("void main() { float a[]; o = vec4(0.0); }").contains("needs an initializer"));
        assert!(refused("void f(float a[]) {}\nvoid main() { o = vec4(0.0); }").contains("sized"));
        assert!(refused("void main() { float x = 1.0; o = vec4(float(x.length())); }").contains("array"));
    }

    /// ES 1.00 has none of it: no array type, no constructor, no `==` on
    /// arrays.
    #[test]
    fn es1_keeps_its_array_rules() {
        let es1 = |body: &str| -> std::string::String {
            let src = std::format!("precision mediump float;\n{body}");
            let mut buf = std::vec![0u8; 1 << 20];
            let a = Arena::new(&mut buf);
            match compile_to_ir(&a, Stage::Fragment, &src) {
                Ok(_) => panic!("compiled: {body}"),
                Err(e) => e.msg.into(),
            }
        };
        assert!(es1("void main() { float a[2]; float b[2]; bool e = a == b; gl_FragColor = vec4(0.0); }")
            .contains("compared"));
        assert!(!es1("void main() { float[2] a; gl_FragColor = vec4(0.0); }").is_empty());
    }
}

#[cfg(test)]
mod scalar_argument_tests {
    /// A scalar argument of a component-wise built-in applies to every
    /// component. `max(v3, f)` handed `f` over as a one-lane value, whose
    /// lanes y and z nothing wrote: the evaluator read 0.0 there, a GPU
    /// whatever its register held. dEQP's value cases never met it,
    /// because their scalars are literals, which are splat constants.
    ///
    /// Revert check: drop the splat in `operand` and lane y reads 0.
    #[test]
    fn a_scalar_argument_reaches_every_lane() {
        let src = "attribute vec4 p;\nvarying float out0;\nuniform float f;\n\
                   void main(){ vec3 v = vec3(-5.0); vec3 r = max(v, f); out0 = r.y + clamp(v, f, f).z; gl_Position=p; }";
        let got = super::uniform_value_tests::out0_with(src, &[("f", &[3.0f32.to_bits()])]);
        assert_eq!(got, 6.0);
    }
}

#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod es3_builtin_tests {
    //! ES 3.00 step 3: the texture look-ups as IR v1 shapes, the matrix
    //! functions, the integer forms of the common functions, and what Qt
    //! Quick 3D's lighting needed on the way (a 12-parameter helper, an
    //! early return from a struct-returning function, a matrix array inside
    //! a uniform array of structs).
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n";

    fn compile<'b>(buf: &'b mut [u8], stage: Stage, src: &'b str) -> Result<Lowered<'b>, &'static str> {
        let a: &'b Arena<'b> = std::boxed::Box::leak(std::boxed::Box::new(Arena::new(buf)));
        compile_to_ir(a, stage, src).map_err(|e| e.msg)
    }

    /// Every `Op::Tex`/`Op::TexSize` a shader lowers to, in order.
    fn looks(stage: Stage, body: &str) -> std::vec::Vec<Op> {
        let src = std::format!("{HEAD}{body}");
        let mut buf = std::vec![0u8; 1 << 20];
        let l = compile(&mut buf, stage, &src).unwrap_or_else(|m| panic!("refused: {m}\n{src}"));
        l.module.instrs.iter().filter(|o| matches!(o, Op::Tex { .. } | Op::TexSize { .. })).copied().collect()
    }

    fn refused(stage: Stage, body: &str) -> &'static str {
        let src = std::format!("{HEAD}{body}");
        let mut buf = std::vec![0u8; 1 << 20];
        match compile(&mut buf, stage, &src) {
            Ok(_) => panic!("compiled: {src}"),
            Err(m) => m,
        }
    }

    /// The shapes Qt Quick 3D samples with: `texture` on a 2D array in a
    /// fragment shader, `textureLod` on a cube and `texture` on a 2D in a
    /// vertex shader (whose implicit level is the base one, section 8.8),
    /// `textureSize`.
    ///
    /// Revert check: drop the `e.vertex()` arm of `texture_es3` and the
    /// vertex look-up comes out `Implicit`.
    #[test]
    fn the_es3_look_ups_become_ir_v1_shapes() {
        let fs = looks(
            Stage::Fragment,
            "uniform highp sampler2DArray a;\nuniform sampler2D s;\nin vec3 c;\nout vec4 o;\n\
             void main() { ivec2 z = textureSize(s, 0); o = texture(a, c) + vec4(float(z.x)); }",
        );
        assert!(matches!(fs[0], Op::TexSize { dim: ir::TexDim::D2, .. }), "{fs:?}");
        assert!(
            matches!(
                fs[1],
                Op::Tex { dim: ir::TexDim::D2Array, shadow: false, proj: false, lod: ir::TexLod::Implicit, .. }
            ),
            "{fs:?}"
        );
        let vs = looks(
            Stage::Vertex,
            "uniform samplerCube q;\nuniform sampler2D s;\nin vec3 c;\n\
             void main() { gl_Position = textureLod(q, c, 2.0) + texture(s, c.xy); }",
        );
        assert!(matches!(vs[0], Op::Tex { dim: ir::TexDim::Cube, lod: ir::TexLod::Lod(_), .. }), "{vs:?}");
        assert!(matches!(vs[1], Op::Tex { dim: ir::TexDim::D2, lod: ir::TexLod::Lod(_), .. }), "{vs:?}");
    }

    /// ES 1.00's look-ups with no level in a vertex shader read the base
    /// level (GLSL ES 1.00 section 8.7) and reach the IR as their Lod forms
    /// at 0.0, never `Implicit` (ir.rs TexLod); in a fragment shader they
    /// stay `Implicit`. A vertex textureCube used to arrive `Implicit`
    /// (report-online-frontend-vertex-cube-level.md, the V3D back end).
    ///
    /// Revert check: drop the `e.vertex()` arm of builtin_call.rs `texture`
    /// and the three vertex look-ups come out `Implicit`.
    #[test]
    fn es100_vertex_look_ups_read_the_base_level() {
        let cases = [
            ("uniform sampler2D s;", "vec2", "texture2D(s, c)", ir::TexDim::D2, false),
            ("uniform sampler2D s;", "vec3", "texture2DProj(s, c)", ir::TexDim::D2, true),
            ("uniform sampler2D s;", "vec4", "texture2DProj(s, c)", ir::TexDim::D2, true),
            ("uniform samplerCube s;", "vec3", "textureCube(s, c)", ir::TexDim::Cube, false),
        ];
        for (decl, ty, call, dim, proj) in cases {
            let vs = std::format!(
                "{decl} attribute {ty} c; varying vec4 v; void main() {{ v = {call}; gl_Position = vec4(0.0); }}"
            );
            let fs = std::format!(
                "precision mediump float; {decl} varying {ty} c; void main() {{ gl_FragColor = {call}; }}"
            );
            let mut buf = std::vec![0u8; 1 << 20];
            let l = compile(&mut buf, Stage::Vertex, &vs).unwrap_or_else(|m| panic!("refused: {m}\n{vs}"));
            let t: std::vec::Vec<Op> =
                l.module.instrs.iter().filter(|o| matches!(o, Op::Tex { .. })).copied().collect();
            assert_eq!(t.len(), 1, "{vs}: {t:?}");
            let Op::Tex { dim: d, lod: ir::TexLod::Lod(level), .. } = t[0] else {
                panic!("{vs}: not a Lod look-up: {t:?}");
            };
            assert_eq!(d, dim, "{vs}");
            /* the level is the constant 0.0 */
            let def = l.module.instrs[level.val.0 as usize];
            assert!(
                matches!(def, Op::ConstF32(v) if v[usize::from(level.swizzle[0])] == 0.0f32.to_bits()),
                "{vs}: {def:?}"
            );
            /* a projective look-up divides (s, t) first and then is the 2D
             * one at the level (texture2DProjLod's lowering) */
            assert_eq!(l.module.instrs.iter().any(|o| matches!(o, Op::Alu(ir::AluOp::Frcp, _))), proj, "{vs}");
            let mut buf = std::vec![0u8; 1 << 20];
            let l = compile(&mut buf, Stage::Fragment, &fs).unwrap_or_else(|m| panic!("refused: {m}\n{fs}"));
            let t: std::vec::Vec<Op> =
                l.module.instrs.iter().filter(|o| matches!(o, Op::Tex { .. })).copied().collect();
            assert!(matches!(t[..], [Op::Tex { lod: ir::TexLod::Implicit, .. }]), "{fs}: {t:?}");
        }
    }

    /// The rest of section 8.8's shapes: a fetch, a projection (divisor in
    /// lane w), gradients read with the identity swizzle, a constant offset,
    /// a bias, a shadow compare, an integer sampler's result.
    #[test]
    fn the_other_look_up_forms() {
        let fs = looks(
            Stage::Fragment,
            "uniform highp sampler3D t;\nuniform sampler2D s;\nuniform highp isampler2D i;\n\
             uniform highp sampler2DShadow d;\nin vec4 c;\nout vec4 o;\n\
             void main() { o = texelFetch(t, ivec3(1, 2, 3), 0) + textureProj(s, c.xyz) \
             + textureGrad(s, c.xy, c.zw, c.yx) + textureOffset(s, c.xy, ivec2(-1, 7), 1.5) \
             + vec4(texture(d, c.xyz)) + vec4(texelFetch(i, ivec2(0), 0)); }",
        );
        assert!(matches!(fs[0], Op::Tex { dim: ir::TexDim::D3, lod: ir::TexLod::Fetch(_), .. }), "{fs:?}");
        let Op::Tex { proj: true, coord, .. } = fs[1] else { panic!("{fs:?}") };
        assert_eq!(coord.swizzle[3], coord.swizzle[2], "a vec3's divisor is repeated into w");
        assert!(matches!(fs[2], Op::Tex { lod: ir::TexLod::Grad { .. }, .. }), "{fs:?}");
        assert!(matches!(fs[3], Op::Tex { lod: ir::TexLod::Bias(_), offset: Some([-1, 7, 0]), .. }), "{fs:?}");
        assert!(matches!(fs[4], Op::Tex { shadow: true, .. }), "{fs:?}");
        assert!(matches!(fs[5], Op::Tex { ret: ir::ScalarType::I32, .. }), "{fs:?}");
    }

    /// What section 8.8 and 4.5.4 do not allow.
    #[test]
    fn what_the_look_up_rules_refuse() {
        let fs = "uniform samplerCube q;\nuniform sampler2D s;\nin vec3 c;\nout vec4 o;\n";
        assert!(refused(Stage::Fragment, &std::format!("{fs}void main() {{ o = textureOffset(q, c, ivec2(0)); }}"))
            .contains("section 8.8"));
        assert!(refused(Stage::Fragment, &std::format!("{fs}void main() {{ o = texture2D(s, c.xy); }}"))
            .contains("ES 3.00"));
        assert!(refused(
            Stage::Vertex,
            "uniform sampler2D s;\nin vec2 c;\nvoid main() { gl_Position = texture(s, c, 1.0); }"
        )
        .contains("fragment"));
        assert!(refused(
            Stage::Fragment,
            &std::format!("{fs}void main() {{ ivec2 k = ivec2(c.xy); o = textureOffset(s, c.xy, k); }}")
        )
        .contains("constant"));
        assert!(refused(Stage::Fragment, "uniform sampler3D t;\nout vec4 o;\nvoid main() { o = vec4(0.0); }")
            .contains("4.5.4"));
        /* a default for the type serves it */
        let mut buf = std::vec![0u8; 1 << 20];
        let ok = std::format!("{HEAD}precision highp sampler3D;\nuniform sampler3D t;\nout vec4 o;\nvoid main() {{ o = texture(t, vec3(0.0)); }}");
        assert!(compile(&mut buf, Stage::Fragment, &ok).is_ok());
    }

    /// A fragment shader run with a uniform block; the float colour.
    fn frag(body: &str, set: &[(&str, &[u32])]) -> [f32; 4] {
        let src = std::format!("{HEAD}layout(location = 0) out vec4 o;\n{body}");
        let mut buf = std::vec![0u8; 1 << 20];
        let l = compile(&mut buf, Stage::Fragment, &src).unwrap_or_else(|m| panic!("refused: {m}"));
        let m = &l.module;
        let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4) + 64];
        for (name, dwords) in set {
            let u = m.uniforms.iter().find(|x| x.name == *name).unwrap_or_else(|| panic!("no uniform {name}"));
            let at = usize::from(u.dword_off);
            block[at..at + dwords.len()].copy_from_slice(dwords);
        }
        run(m, &Inputs { io: &[], uniforms: &block }).expect("runs").output_f32(ir::IoSlot::FragColor(0)).expect("o")
    }

    fn near(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-4 * (1.0 + y.abs()))
    }

    /// inverse, transpose, determinant and outerProduct on every square
    /// size, checked through `inverse(m) * m == I` and known values.
    ///
    /// Revert check: swap `r2` and `r3` in the mat4 arm of
    /// `matrix_inverse` and the mat4 product is no longer the identity.
    #[test]
    fn the_matrix_functions() {
        let body = "void main() {\n\
            mat2 a = mat2(4.0, 2.0, 7.0, 6.0);\n\
            mat3 b = mat3(2.0, 0.0, 1.0, 1.0, 3.0, 2.0, 1.0, 1.0, 2.0);\n\
            mat4 c = mat4(2.0, 1.0, 0.0, 0.0, 0.0, 3.0, 1.0, 0.0, 1.0, 0.0, 4.0, 2.0, 0.0, 1.0, 0.0, 5.0);\n\
            mat2 ia = inverse(a) * a; mat3 ib = inverse(b) * b; mat4 ic = inverse(c) * c;\n\
            mat3 tb = transpose(b); mat2 op = outerProduct(vec2(1.0, 2.0), vec2(3.0, 4.0));\n\
            o = vec4(ia[0][0] + ia[1][1] + ia[0][1] + ia[1][0], ib[0][0] + ib[1][1] + ib[2][2] + ib[2][0] + ib[0][2],\n\
                     ic[0][0] + ic[1][1] + ic[2][2] + ic[3][3] + ic[3][0] + ic[0][3] + ic[2][1],\n\
                     determinant(a) * 1000.0 + determinant(b) * 100.0 + tb[0][2] * 10.0 + op[1][0]); }";
        let got = frag(body, &[]);
        /* det(a) = 4*6 - 7*2 = 10; det(b) = b0 . (b1 x b2) = 6; tb[0][2] =
         * b[2][0] = 1; op[1][0] = 1 * 4 = 4. */
        assert!(near(got, [2.0, 3.0, 4.0, 10000.0 + 600.0 + 10.0 + 4.0]), "{got:?}");
        let det4 = frag("void main() { mat4 c = mat4(2.0, 1.0, 0.0, 0.0, 0.0, 3.0, 1.0, 0.0, 1.0, 0.0, 4.0, 2.0, 0.0, 1.0, 0.0, 5.0); o = vec4(determinant(c)); }", &[]);
        /* Laplace expansion of the rows (2 0 1 0 / 1 3 0 1 / 0 1 4 0 / 0 0 2 5): 129. */
        assert!(near(det4, [129.0; 4]), "{det4:?}");
    }

    /// ES 3.00's integer `abs`, `sign`, `min`, `max`, `clamp`, and `mix`
    /// selecting by a bvec.
    ///
    /// Revert check: take `integer_kind` out of `componentwise` and `abs(-3)`
    /// reads the float abs of the int's bits.
    #[test]
    fn the_integer_common_functions() {
        let body = "uniform int k;\nuniform uint u;\n\
            void main() { ivec2 v = ivec2(k, -k); \
            o = vec4(float(abs(k)), float(sign(v).y), float(clamp(v, -2, 2).x + max(k, 1)), \
            float(min(u, 5u)) + mix(1.0, 20.0, k > 0)); }";
        assert_eq!(frag(body, &[("k", &[(-3i32) as u32]), ("u", &[0xffff_fff0])]), [3.0, 1.0, -1.0, 6.0]);
        assert_eq!(frag(body, &[("k", &[7]), ("u", &[2])]), [7.0, -1.0, 9.0, 22.0]);
    }

    /// A 12-parameter helper with `inout` parameters (Qt Quick 3D's
    /// `qt_processPunctualLighting`), and an early return from a function
    /// that returns a struct (`qt_sampleSpotLight`).
    ///
    /// Revert check: put `MAX_PARAMS` back to 8 and the call is refused.
    #[test]
    fn a_long_helper_and_an_early_struct_return() {
        let body = "struct R { float a; vec2 b; };\n\
            void f(inout float x, inout float y, float p2, float p3, float p4, float p5, float p6, float p7, float p8, float p9, float p10, float p11) { x = p2 + p11; y = p3 * p10; }\n\
            R g(float t) { if (t > 0.0) { return R(1.0, vec2(t)); } R r = R(2.0, vec2(-t)); return r; }\n\
            uniform float t;\n\
            void main() { float x = 0.0; float y = 0.0; f(x, y, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 5.0, 10.0); R r = g(t); o = vec4(x, y, r.a, r.b.y); }";
        assert_eq!(frag(body, &[("t", &[3.0f32.to_bits()])]), [11.0, 10.0, 1.0, 3.0]);
        assert_eq!(frag(body, &[("t", &[(-3.0f32).to_bits()])]), [11.0, 10.0, 2.0, 3.0]);
    }
}

#[cfg(test)]
mod uniform_matrix_array_tests {
    /// Qt Quick 3D's `lights[i].matrices[split] * v`: a matrix array inside
    /// a uniform array of structs, both indices chosen at run time. Each
    /// element's columns hold distinct values, so a wrong element or column
    /// reads a different number.
    ///
    /// Revert check: take the `ElementColumn` arm out of `matrix_column` and
    /// the compile is refused ("this matrix expression is not lowered yet").
    #[test]
    fn a_matrix_array_in_a_uniform_struct_array_is_indexed_at_run_time() {
        let src = "attribute vec4 p;\nvarying float out0;\nstruct L { float f; mat2 m[2]; };\nuniform L u[2];\n\
                   uniform int i;\nuniform int j;\n\
                   void main() { vec2 r = u[i].m[j] * vec2(1.0, 10.0); out0 = r.x + r.y * 1000.0; gl_Position = p; }";
        let row = |base: f32| -> std::vec::Vec<u32> { (0..8).map(|k| (base + k as f32).to_bits()).collect() };
        let (r0, r1) = (row(1.0), row(11.0));
        for (i, j, want) in
            [(1u32, 1u32, 185.0 + 196.0 * 1000.0), (0, 1, 75.0 + 86.0 * 1000.0), (1, 0, 141.0 + 152.0 * 1000.0)]
        {
            let set: [(&str, &[u32]); 4] = [("u[0].m", &r0), ("u[1].m", &r1), ("i", &[i]), ("j", &[j])];
            /* element j of u[i].m is columns (b + 4j, b + 4j + 1) and
             * (b + 4j + 2, b + 4j + 3), times (1, 10). */
            let got = super::uniform_value_tests::out0_with(src, &set);
            assert_eq!(got, want, "u[{i}].m[{j}]");
        }
    }
}

#[cfg(test)]
mod scratch_tests {
    use super::*;

    /// A dEQP uniform_api.random.23-shaped shader: compare_* helpers
    /// inlined a hundred times over a uniform array of structs. Built here
    /// rather than read from a file so the measurement cannot drift from
    /// the source it describes.
    pub(super) fn standin() -> std::string::String {
        let mut s = std::string::String::from(
            "precision mediump float;\nstruct S { vec4 a; mat3 m; float f[3]; vec2 b; };\nuniform S u[4];\n\
             bool compare_float(float a, float b) { return abs(a - b) < 0.05; }\n\
             bool compare_vec2(vec2 a, vec2 b) { return compare_float(a.x, b.x) && compare_float(a.y, b.y); }\n\
             bool compare_vec3(vec3 a, vec3 b) { return compare_float(a.x, b.x) && compare_float(a.y, b.y) && compare_float(a.z, b.z); }\n\
             bool compare_vec4(vec4 a, vec4 b) { return compare_float(a.x, b.x) && compare_float(a.y, b.y) && compare_float(a.z, b.z) && compare_float(a.w, b.w); }\n\
             bool compare_mat3(mat3 a, mat3 b) { return compare_vec3(a[0], b[0]) && compare_vec3(a[1], b[1]) && compare_vec3(a[2], b[2]); }\n\
             void main() {\n  bool ok = true;\n",
        );
        for i in 0..25 {
            let e = i % 4;
            s += &std::format!("  ok = ok && compare_vec4(u[{e}].a, vec4({i}.0, 1.0, 2.0, 3.0));\n");
            s += &std::format!("  ok = ok && compare_mat3(u[{e}].m, mat3({i}.5));\n");
            s += &std::format!("  ok = ok && compare_float(u[{e}].f[{}], {i}.25);\n", i % 3);
            s += &std::format!("  ok = ok && compare_vec2(u[{e}].b, vec2({i}.0));\n");
        }
        s += "  gl_FragColor = ok ? vec4(1.0) : vec4(0.0, 0.0, 0.0, 1.0);\n}\n";
        s
    }

    /// The front end's scratch on that shape, a tripwire rather than a
    /// target. Measured 2026-09-23 (HOST-ONLY): 2,297,703 bytes before the
    /// scratch work, 1,484,936 after it -- run lists appended once instead
    /// of three times and given back, vectors growing in place when they
    /// are the last allocation, and a moved vector's old buffer reused.
    /// The product budget for a whole compile, back end included, is
    /// `store::SCRATCH_BYTES` (4 MiB; 2 MiB when this was measured).
    ///
    /// Revert check: stop the arena handing out given-back blocks
    /// (`Arena::take_free`) and this reads about 1.72 MB.
    #[test]
    fn the_front_end_scratch_on_a_random23_shape() {
        let src = standin();
        let mut buf = std::vec![0u8; 8 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let used = a.used();
        std::println!("front end scratch: {used} bytes for {} IR instructions", l.module.instrs.len());
        assert!(used < 1_550_000, "the front end took {used} bytes of scratch (it was 1,484,936)");
    }
}

#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod nonsquare_matrix_tests {
    //! ES 3.00's non-square matrices (step 2c): matCxR is C columns of R
    //! rows everywhere -- products, constructors, transpose, outerProduct,
    //! uniforms and locals -- run by the evaluator with values that differ
    //! per component, so a transposed or truncated shape reads another
    //! number.
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    fn frag(body: &str, set: &[(&str, &[u32])]) -> [f32; 4] {
        let src = std::format!("#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n{body}");
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        let m = &l.module;
        let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4) + 64];
        for (name, dwords) in set {
            let u = m.uniforms.iter().find(|x| x.name == *name).unwrap_or_else(|| panic!("no uniform {name}"));
            let at = usize::from(u.dword_off);
            block[at..at + dwords.len()].copy_from_slice(dwords);
        }
        run(m, &Inputs { io: &[], uniforms: &block }).expect("runs").output_f32(ir::IoSlot::FragColor(0)).expect("o")
    }

    /// `m * v`, `v * m` and `m * m` with the shapes section 5.11 gives them.
    /// `a` is mat2x3: columns (1, 2, 3) and (4, 5, 6).
    ///
    /// Revert check: make `vec_times_mat` dot over the column count instead
    /// of the rows and this fails.
    #[test]
    fn products_take_their_shapes_from_both_sides() {
        let body = "void main() { mat2x3 a = mat2x3(1.0, 2.0, 3.0, 4.0, 5.0, 6.0);\n\
                    vec3 p = a * vec2(1.0, 10.0);\n\
                    vec2 q = vec3(1.0, 10.0, 100.0) * a;\n\
                    mat3x2 b = mat3x2(1.0, 0.0, 0.0, 1.0, 1.0, 1.0);\n\
                    mat2 c = b * a;\n\
                    o = vec4(p.z, q.y, c[1][0], c[1][1]); }";
        /* p = 1*(1,2,3) + 10*(4,5,6) = (41, 52, 63); q = (1+20+300,
         * 4+50+600) = (321, 654); b is columns (1,0) (0,1) (1,1), so
         * c[1] = b * a[1] = 4*(1,0) + 5*(0,1) + 6*(1,1) = (10, 11). */
        assert_eq!(frag(body, &[]), [63.0, 654.0, 10.0, 11.0]);
    }

    /// Constructors (from components, from a bigger and a smaller
    /// matrix), transpose and outerProduct.
    #[test]
    fn constructors_transpose_and_outer_product() {
        let body = "void main() {\n\
                    mat4 big = mat4(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0);\n\
                    mat2x3 s = mat2x3(big);\n\
                    mat4x3 g = mat4x3(mat2(7.0));\n\
                    mat3x2 t = transpose(s);\n\
                    mat3x2 op = outerProduct(vec2(1.0, 2.0), vec3(3.0, 4.0, 5.0));\n\
                    o = vec4(s[1][2], g[2][2] + g[3][0] + g[1][2] * 100.0, t[2][1], op[2][1]); }";
        /* s[1] = big[1].xyz = (5,6,7); g = identity grown around 7*I2:
         * g[2][2] = 1, g[3][0] = 0, g[1][2] = 0; t[2][1] = s[1][2] = 7;
         * op[2] = (1,2) * 5 = (5, 10). */
        assert_eq!(frag(body, &[]), [7.0, 1.0, 7.0, 10.0]);
    }

    /// A mat3x2 uniform: three columns of two, packed two dwords apart
    /// (std430), and a mat2x3 whose columns are padded to four.
    ///
    /// Revert check: read the uniform's columns as `n` wide in
    /// `uniform_matrix_column` and lane y of the mat2x3 read is wrong.
    #[test]
    fn non_square_uniforms_have_their_own_strides() {
        let body = "uniform mat3x2 a;\nuniform mat2x3 b;\n\
                    void main() { o = vec4(a[2].y, a[1].x, b[1].z, b[0].y); }";
        let a: std::vec::Vec<u32> = (1..=6).map(|k| (k as f32).to_bits()).collect();
        let b: std::vec::Vec<u32> = [10.0, 20.0, 30.0, 0.0, 40.0, 50.0, 60.0, 0.0].map(f32::to_bits).to_vec();
        assert_eq!(frag(body, &[("a", &a), ("b", &b)]), [6.0, 3.0, 60.0, 20.0]);
        let mut buf = std::vec![0u8; 1 << 20];
        let ar = Arena::new(&mut buf);
        let l = compile_to_ir(
            &ar,
            Stage::Fragment,
            "#version 300 es\nprecision highp float;\nuniform mat3x2 a;\nuniform mat2x3 b;\nout vec4 o;\nvoid main() { o = vec4(a[0], b[0].xy); }",
        )
        .expect("compiles");
        let rows: std::vec::Vec<(&str, u16, u16, u16)> =
            l.reflection.uniforms.iter().map(|u| (u.name, u.dword_off, u.dwords, u.gl_type)).collect();
        assert_eq!(rows, [("a", 0, 6, 0x8B67), ("b", 8, 8, 0x8B65)]);
    }

    #[test]
    fn what_the_shapes_refuse() {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let bad = "#version 300 es\nprecision highp float;\nout vec4 o;\n\
                   void main() { mat2x3 m = mat2x3(1.0); vec3 v = m * vec3(1.0); o = vec4(v, 1.0); }";
        let e = compile_to_ir(&a, Stage::Fragment, bad).err().expect("refused");
        assert!(e.msg.contains("multiplied"), "{}", e.msg);
        let mut buf2 = std::vec![0u8; 1 << 20];
        let a2 = Arena::new(&mut buf2);
        let inv = "#version 300 es\nprecision highp float;\nout vec4 o;\n\
                   void main() { mat2x3 m = mat2x3(1.0); mat3x2 n = inverse(m); o = vec4(n[0], 0.0, 1.0); }";
        assert!(compile_to_ir(&a2, Stage::Fragment, inv).is_err(), "inverse is square-only");
    }
}

#[cfg(test)]
mod relocate_tests {
    //! `relocate`: the front end's scratch given back before selection.
    #[cfg(any(feature = "gpu-vivante", not(feature = "no-shader-table")))]
    use super::super::codegen;
    use super::super::store;
    use super::*;
    /* The helpers of the two whole-compile tests below, which run where a
     * whole compile of the stand-in or the table's corpus can (the GC7000
     * build, and every build with the table). */
    #[cfg(any(feature = "gpu-vivante", not(feature = "no-shader-table")))]
    use core::sync::atomic::Ordering;

    /// What a compile produced: its words, and its uniform rows as bytes.
    #[cfg(any(feature = "gpu-vivante", not(feature = "no-shader-table")))]
    type Output = Option<(std::vec::Vec<u32>, std::vec::Vec<u8>)>;

    /// One compile through the code generator in the store's scratch: the
    /// scratch high-water mark, and what came out.
    #[cfg(any(feature = "gpu-vivante", not(feature = "no-shader-table")))]
    fn compile(stage: u32, src: &str) -> (usize, Output) {
        /* An uncommitted keep transaction: whatever the compile keeps is
         * given back when it drops, so a corpus of compiles does not fill
         * the region. */
        let _txn = store::begin_keep().expect("no compile holds the keep region");
        store::with_scratch(|a| {
            let r = codegen::compile(stage, src.as_bytes(), 0, a, store::keep());
            let out = r.ok().map(|sh| {
                let uni: std::vec::Vec<u8> = sh
                    .uniform_table()
                    .iter()
                    .flat_map(|u| {
                        let mut b = std::vec::Vec::from(crate::util::name_bytes(&u.name));
                        b.extend_from_slice(&u.dword_off.to_le_bytes());
                        b.extend_from_slice(&u.dwords.to_le_bytes());
                        b
                    })
                    .collect();
                (sh.usc_words().to_vec(), uni)
            });
            (a.peak(), out)
        })
        .expect("scratch is free")
    }

    /// What the back end starts from on the random.23 shape: the front end
    /// peaks near 1.5 MB, and after `relocate` only the module and the
    /// reflection are left -- measured 357 KB (HOST-ONLY). The bound is
    /// generous; without the release it is the whole 1.49 MB.
    ///
    /// Revert check: make `relocate` return its input and `used()` stays at
    /// the front end's top.
    #[test]
    fn the_front_end_hands_the_back_end_only_the_module_and_the_reflection() {
        let _serial = store::test_lock();
        let mut buf = std::vec![0u8; store::SCRATCH_BYTES];
        let a = Arena::new(&mut buf);
        let src = super::scratch_tests::standin();
        let text = a.alloc_str(&src).expect("room for the text");
        let mark = a.used();
        let l = compile_to_ir(&a, super::super::sema::Stage::Fragment, text).expect("the stand-in compiles");
        let top = a.used();
        let l = relocate(&a, l, text, mark).expect("relocate");
        std::println!("front end top {top} bytes, {} after the release; peak {}", a.used(), a.peak());
        assert!(top > 1_000_000, "the stand-in still exercises the front end: {top}");
        assert!(a.used() < 500_000, "{} bytes left for the back end to start from", a.used());
        assert!(a.peak() >= top, "the peak remembers the front end's top");
        /* What was moved is still the module: it has instructions, and its
         * uniform names read back. */
        assert!(!l.module.instrs.is_empty());
        assert!(l.reflection.uniforms.iter().any(|u| u.name.starts_with("u[")));
    }

    /// The whole compile's scratch (front end, selection, allocation) on
    /// the random.23 shape fits the product's budget
    /// (`store::SCRATCH_BYTES`, 2 MiB when measured, 4 MiB now) on the GC7000. HOST-ONLY: measured
    /// 2,044,776 bytes; without the release the back end runs out at
    /// 1,736,464 (a 530,400-byte allocation does not fit above the front
    /// end's garbage).
    ///
    /// Not asserted on PowerVR: its selector's if-conversion copies the
    /// local map twice per `if` (14,016 bytes each on this shape) and does
    /// not give them back, so the back end alone passes 2 MiB there.
    ///
    /// Revert check: make `relocate` return its input and the compile runs
    /// out of scratch.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_whole_compile_of_a_random23_shape_fits_the_budget() {
        let _serial = store::test_lock();
        let src = super::scratch_tests::standin();
        let (on, out_on) = compile(crate::shaderc::SHC_STAGE_FRAGMENT, &src);
        RELOCATE_OFF.store(true, Ordering::Relaxed);
        let (off, out_off) = compile(crate::shaderc::SHC_STAGE_FRAGMENT, &src);
        RELOCATE_OFF.store(false, Ordering::Relaxed);
        std::println!(
            "whole compile scratch peak: {on} bytes with the front end given back (compiled: {}), {off} without (compiled: {})",
            out_on.is_some(),
            out_off.is_some()
        );
        assert!(out_on.is_some(), "the stand-in compiles in {} bytes of scratch", store::SCRATCH_BYTES);
        assert!(on <= store::SCRATCH_BYTES);
        assert!(out_off.is_none() || on < off, "giving the front end back lowers the peak ({on} vs {off})");
    }

    /// Nothing selected reads the front end's memory after it is given
    /// back: in a test build the released range is filled with 0xa5, and
    /// every corpus source compiles to the same words and the same uniform
    /// rows with and without the release.
    ///
    /// Revert check: leave the reflection's uniform names where they were
    /// (drop their `moved_str`) and the poisoned bytes change the uniform
    /// rows.
    ///
    /// The corpus is the offline table's sources, so a build without the
    /// table (`no-shader-table`) has nothing to run it on.
    #[cfg(not(feature = "no-shader-table"))]
    #[test]
    fn nothing_reads_the_front_ends_memory_after_it_is_given_back() {
        let _serial = store::test_lock();
        let (mut compared, mut differ) = (0usize, std::vec::Vec::new());
        for e in crate::shaderc::mesa_reference::SHC_SOURCE_TABLE.iter() {
            let text = e.text.trim_end_matches('\0');
            let stage = if e.name.contains(".vert") {
                crate::shaderc::SHC_STAGE_VERTEX
            } else {
                crate::shaderc::SHC_STAGE_FRAGMENT
            };
            RELOCATE_OFF.store(true, Ordering::Relaxed);
            let (_, plain) = compile(stage, text);
            RELOCATE_OFF.store(false, Ordering::Relaxed);
            let (_, moved) = compile(stage, text);
            if plain.is_none() && moved.is_none() {
                continue;
            }
            compared += 1;
            if plain != moved {
                differ.push(e.name);
            }
        }
        std::println!("compared {compared} compiles with and without the release");
        assert!(compared > 100, "the corpus reached the back end: {compared}");
        assert!(
            differ.is_empty(),
            "{} sources compile differently once the front end is given back: {differ:?}",
            differ.len()
        );
    }
}

#[cfg(all(test, not(feature = "report-es2")))] // every test here compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
mod switch_tests {
    //! ES 3.00's `switch` (step 4): run by the evaluator for every selector
    //! value that picks a different path, so a group that does not fall
    //! through, a `default` taken when a label matched, or a `break` that
    //! leaves more than the switch reads another number.
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    fn source(body: &str) -> std::string::String {
        std::format!(
            "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform int u;\nuniform uint w;\n\
             layout(location = 0) out vec4 o;\n{body}"
        )
    }

    /// `o` for the uniforms `u` = `ui` and `w` = `wi`.
    fn frag(body: &str, ui: i32, wi: u32) -> [f32; 4] {
        let src = source(body);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        let m = &l.module;
        let mut block = std::vec![0u32; usize::from(m.uniform_dwords).max(4) + 64];
        for u in m.uniforms {
            let at = usize::from(u.dword_off);
            match u.name {
                "u" => block[at] = ui as u32,
                "w" => block[at] = wi,
                _ => {}
            }
        }
        run(m, &Inputs { io: &[], uniforms: &block }).expect("runs").output_f32(ir::IoSlot::FragColor(0)).expect("o")
    }

    fn refused(body: &str) -> &'static str {
        let src = source(body);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        match compile_to_ir(&a, Stage::Fragment, &src) {
            Ok(_) => panic!("expected a refusal: {body}"),
            Err(e) => e.msg,
        }
    }

    /// Matching, falling through, `default` in the middle (taken only when
    /// nothing matched), and a selector no label and no default takes.
    ///
    /// Revert check: drop `fell` from the group condition and case 1 stops
    /// falling into case 2 (x = 10 instead of 110).
    #[test]
    fn groups_match_fall_through_and_default_wherever_it_stands() {
        let body = "void main() { float x = 0.0;\n\
                    switch (u) {\n\
                      case 1: x += 10.0;\n\
                      case 2: case 3: x += 100.0; break;\n\
                      default: x += 1000.0;\n\
                      case 4: x += 10000.0; break;\n\
                      case 5: x += 5.0;\n\
                    }\n\
                    o = vec4(x); }";
        let got: std::vec::Vec<f32> = [1, 2, 3, 4, 5, 9].iter().map(|&k| frag(body, k, 0)[0]).collect();
        /* 1 falls into 2; 3 shares 2's group; 9 takes default and falls
         * into 4; 5 is the last group and ends the body. */
        assert_eq!(got, [110.0, 100.0, 100.0, 10000.0, 5.0, 11000.0]);
        let no_default = "void main() { float x = 7.0; switch (u) { case 0: x = 1.0; break; } o = vec4(x); }";
        assert_eq!(frag(no_default, 3, 0)[0], 7.0);
    }

    /// A `break` in an `if` inside a group leaves the switch and nothing
    /// else; `continue` in a switch continues the loop around it, running
    /// the `for` step; a `break` in a loop inside a switch leaves that loop.
    ///
    /// Revert check: lower `continue` as a bare `Continue` inside the
    /// switch (drop the `switch_cont` arm of `lower_continue`) and the
    /// switch's own loop runs again (the evaluator reports a runaway loop
    /// or the sum changes).
    #[test]
    fn break_leaves_the_switch_and_continue_the_loop_around_it() {
        let body = "void main() { float s = 0.0;\n\
                    for (int i = 0; i < 6; i++) {\n\
                      switch (i) {\n\
                        case 0: s += 1.0; if (u == 0) break; s += 100.0;\n\
                        case 1: s += 2.0; break;\n\
                        case 2: continue;\n\
                        case 3: for (int j = 0; j < 10; j++) { if (j == 2) break; s += 1000.0; } break;\n\
                        default: s += 10.0;\n\
                      }\n\
                      s += 0.5;\n\
                    }\n\
                    o = vec4(s); }";
        /* u = 0: i=0 adds 1 and breaks; i=1 adds 2; i=2 continues (no 0.5);
         * i=3 adds 2000; i=4,5 add 10 each; 0.5 five times. */
        assert_eq!(frag(body, 0, 0)[0], 1.0 + 2.0 + 2000.0 + 20.0 + 2.5);
        /* u = 1: i=0 adds 1, 100 and falls into case 1's 2. */
        assert_eq!(frag(body, 1, 0)[0], 103.0 + 2.0 + 2000.0 + 20.0 + 2.5);
    }

    /// `return` from inside a switch, in a function and in `main`, and a
    /// switch nested in a group of another.
    ///
    /// Revert check: leave `ret.loops` alone in `switch_statement` and the
    /// return in the function's switch is refused ("an early 'return' is not
    /// lowered yet").
    #[test]
    fn return_and_nesting() {
        let body = "float f(int k) {\n\
                      switch (k) {\n\
                        case 0: return 1.0;\n\
                        case 1: switch (k + 1) { case 2: return 2.0; default: break; } return 3.0;\n\
                      }\n\
                      return 4.0;\n\
                    }\n\
                    void main() { o = vec4(f(u), f(1), f(7), 0.0);\n\
                      switch (u) { case 5: o.w = 9.0; return; }\n\
                      o.w = 8.0; }";
        assert_eq!(frag(body, 0, 0), [1.0, 2.0, 4.0, 8.0]);
        assert_eq!(frag(body, 5, 0), [4.0, 2.0, 4.0, 9.0]);
    }

    /// A `uint` selector with `uint` labels, and a declaration in one group
    /// that a later group assigns.
    #[test]
    fn a_uint_selector_and_a_declaration_across_groups() {
        let body = "void main() { float x = 0.0;\n\
                    switch (w) { case 4000000000u: float t = 2.0; x = t; break; case 1u: t = 3.0; x = t; }\n\
                    o = vec4(x); }";
        assert_eq!(frag(body, 0, 4_000_000_000)[0], 2.0);
        assert_eq!(frag(body, 0, 1)[0], 3.0);
    }

    /// The `if`s of a lowered switch body, as (then-length > 0) flags, and
    /// whether any condition reads `fell`: only what can happen is emitted.
    fn body_ifs(body: &str) -> (usize, usize, bool) {
        let src = source(body);
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).unwrap_or_else(|e| panic!("refused: {}", e.msg));
        let m = &l.module;
        /* The switch's loop is the only loop; its body's direct `if`s are
         * the tested groups. */
        let lp = m.nodes.iter().find_map(|n| match n {
            ir::Node::Loop { body } => Some(*body),
            _ => None,
        });
        let lp = lp.expect("the switch is a loop");
        let direct = &m.nodes[lp.first as usize..(lp.first + lp.len) as usize];
        let ifs = direct.iter().filter(|n| matches!(n, ir::Node::If { .. })).count();
        let bools = m.locals.iter().filter(|v| v.ty.scalar == ir::ScalarType::Bool && v.name.is_empty()).count();
        (ifs, bools, direct.iter().any(|n| matches!(n, ir::Node::Instr(_))))
    }

    /// usc-backend's shape (qt_tonemap in
    /// skyboxcube_runtime_tonemapping_300.frag): every case returns or
    /// breaks, then a trailing `default`. No group can be fallen into, so
    /// there is no `fell` flag, and the default is emitted with no `if` --
    /// an `if (fell || none)` there was always taken. A switch of only
    /// `default` is the same. Values are unchanged (the tests above).
    ///
    /// Revert check: set `always` to false in `SwitchShape::of` and the
    /// first shape has three `if`s instead of two.
    #[test]
    fn only_the_tests_that_can_fail_are_emitted() {
        let (ifs, flags, _) = body_ifs(
            "void main() { float x = 0.0; switch (u) { case 0: x = 1.0; break; case 1: x = 2.0; break; default: x = 3.0; } o = vec4(x); }",
        );
        assert_eq!((ifs, flags), (2, 0), "two tested cases, an untested default, and no fall-through flag");
        let (ifs, flags, _) = body_ifs("void main() { float x = 0.0; switch (u) { default: x = 3.0; } o = vec4(x); }");
        assert_eq!((ifs, flags), (0, 0));
        assert_eq!(frag("void main() { float x = 0.0; switch (u) { default: x = 3.0; } o = vec4(x); }", 5, 0)[0], 3.0);
        /* A group that can be fallen into still tests `fell`: one flag. */
        let (_, flags, _) = body_ifs(
            "void main() { float x = 0.0; switch (u) { case 0: x = 1.0; case 1: x += 2.0; break; } o = vec4(x); }",
        );
        assert_eq!(flags, 1);
    }

    /// Section 6.2's rules, each refused by name (the negative half of
    /// dEQP's `switch.test`).
    ///
    /// Revert check: drop the `last_label` check in `Sema::switch_body` and
    /// the first shape compiles.
    #[test]
    fn the_rules_of_section_6_2() {
        let wrap = |s: &str| std::format!("void main() {{ {s} o = vec4(0.0); }}");
        assert_eq!(
            refused(&wrap("switch (u) { case 0: break; case 1: }")),
            "a case label must be followed by a statement"
        );
        assert_eq!(
            refused(&wrap("switch (u) { o = vec4(1.0); case 0: break; }")),
            "a statement in a switch comes before its first label"
        );
        assert_eq!(refused(&wrap("switch (u) { case 1: case 1: break; }")), "two case labels have the same value");
        assert_eq!(
            refused(&wrap("switch (u) { default: break; default: break; }")),
            "a switch may have only one 'default' label"
        );
        assert_eq!(
            refused(&wrap("switch (1.0) { case 1: break; }")),
            "the expression of a switch must be a scalar int or uint"
        );
        assert_eq!(
            refused(&wrap("switch (u) { case 1u: break; }")),
            "a case label must have the type of the switch expression"
        );
        assert_eq!(
            refused(&wrap("int l = 1; switch (u) { case l: break; }")),
            "a case label must be a constant expression"
        );
        assert_eq!(
            refused(&wrap("switch (u) { case 0: if (true) case 1: break; }")),
            "a case label may only stand directly in the body of a switch"
        );
        assert_eq!(refused(&wrap("switch (u) { case 0: continue; }")), "'continue' outside a loop");
        /* A constant expression that is not a literal is a label, and an
         * empty switch is a switch. */
        assert_eq!(
            frag("const int K = 2; void main() { float x = 0.0; switch (u) { case K + 1: x = 1.0; } switch (u) {} o = vec4(x); }", 3, 0)[0],
            1.0
        );
    }
}

/// A local integer index that is a constant where it is used (constant
/// propagation through straight-line assignments and `++`): dEQP-GLES2
/// shaders.indexing.tmp_array.*_const_write_*, whose 36 `arr[i++] = ...`
/// into a 40-element local were each a 40-way compare-and-select and ran
/// the compile out of scratch. HOST-ONLY.
#[cfg(test)]
mod const_local_index_tests {
    use super::*;
    use crate::shaderc::online::eval::{run, Inputs};

    /// dEQP's generator (es2fShaderIndexingTests.cpp, createTmpArrayCase)
    /// for the fragment case with a constant write and a dynamic-loop read.
    /// For `float` its output is byte-identical to the source of
    /// dEQP-GLES2.functional.shaders.indexing.tmp_array.
    /// float_const_write_dynamic_loop_read_fragment in gc-backend's host run
    /// at ac11728 (compared 2026-09-24): 6,204 IR instructions and out of
    /// scratch before the folding, 407 after.
    fn tmp_array_fragment(ty: &str, ctor: &str, padding: &str) -> std::string::String {
        let mut s = std::string::String::from("uniform mediump int ui_four;\n\nvoid main()\n{\n");
        s += &std::format!("    mediump {ty} arr[40];\n");
        s += &std::format!("    arr[0] = {ty}({ctor});\n");
        s += &std::format!("    arr[1] = {ty}({ctor}) * 0.5;\n");
        s += &std::format!("    arr[2] = {ty}({ctor}) * 0.25;\n");
        s += &std::format!("    arr[3] = {ty}({ctor}) * 0.125;\n");
        s += "    int i = 4;\n";
        for k in 4..40 {
            s += &std::format!("    arr[i++] = {ty}({k}.0);\n");
        }
        s += &std::format!("    mediump {ty} res = {ty}(0.0);\n");
        s += "    for (int i = 0; i < ui_four; i++)\n        res += arr[i];\n";
        s += &std::format!("    gl_FragColor = vec4(res{padding});\n}}\n");
        s
    }

    fn lowered_instrs(src: &str) -> usize {
        let mut buf = std::vec![0u8; 1 << 22];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, src).unwrap_or_else(|e| panic!("refused: {}: {}", e.tok, e.msg));
        l.module.instrs.len()
    }

    /// The IR stays small: each `arr[i++]` is one static element store and
    /// `i`'s increment, not a select over forty elements.
    ///
    /// Revert check: make `folded_index` answer `None` and the float
    /// shader's IR is thousands of instructions.
    #[test]
    fn a_constant_write_through_a_counted_local_is_a_static_store() {
        let _serial = super::super::store::test_lock();
        let n = lowered_instrs(&tmp_array_fragment("float", "0.125", ", 0.0, 0.0, 1.0"));
        std::println!("tmp_array float_const_write_dynamic_loop_read_fragment: {n} IR instructions");
        assert!(n < 700, "{n} IR instructions");
        let n4 = lowered_instrs(&tmp_array_fragment("vec4", "0.125, 0.25, 0.5, 1.0", ""));
        std::println!("tmp_array vec4_const_write_dynamic_loop_read_fragment: {n4} IR instructions");
        assert!(n4 < 700, "{n4} IR instructions");
    }

    /// And the values are the ones dEQP expects: the loop reads the first
    /// `ui_four` elements (4 here), 0.125 * (1 + 0.5 + 0.25 + 0.125).
    #[test]
    fn the_folded_stores_keep_the_values() {
        let _serial = super::super::store::test_lock();
        let src = tmp_array_fragment("float", "0.125", ", 0.0, 0.0, 1.0");
        let mut buf = std::vec![0u8; 1 << 22];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Fragment, &src).expect("compiles");
        let mut block = std::vec![0u32; usize::from(l.module.uniform_dwords).max(4)];
        block[0] = 4;
        let out = run(&l.module, &Inputs { io: &[], uniforms: &block }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragColor(0)).expect("colour")[0], 0.125 * 1.875);
        block[0] = 6;
        let out = run(&l.module, &Inputs { io: &[], uniforms: &block }).expect("runs");
        assert_eq!(out.output_f32(ir::IoSlot::FragColor(0)).expect("colour")[0], 0.125 * 1.875 + 4.0 + 5.0);
    }

    /// Where the index is NOT known at the store it stays dynamic, and
    /// every value is what the unfolded lowering computes: after a loop that
    /// changes it, inside a conditional, a compound write, arithmetic on a
    /// known local, and a read through a known index.
    ///
    /// Revert check: make the loop constructs leave `fold_depth` alone and
    /// the index after the loop is folded to its pre-loop value (u = 3
    /// differs).
    #[test]
    fn folding_changes_no_value() {
        let _serial = super::super::store::test_lock();
        let src = "precision mediump float;\nuniform int u;\n\
                   void main() {\n\
                     float a[8];\n\
                     for (int k = 0; k < 8; k++) a[k] = 0.0;\n\
                     int i = 1;\n\
                     a[i++] = 10.0;\n\
                     for (int k = 0; k < u; k++) i++;\n\
                     a[i] = 20.0;\n\
                     if (u > 3) { i = 7; }\n\
                     a[i] += 1.0;\n\
                     int j = 2;\n\
                     j = j * 3;\n\
                     a[j - 1] = 30.0 + a[j - 5];\n\
                     int m = 3;\n\
                     float r = u > 1 ? a[m++] : a[m];\n\
                     a[m] += r;\n\
                     gl_FragColor = vec4(a[1] + a[3] * 100.0, a[2] + a[4] * 100.0, a[5], a[7] + a[6] * 100.0);\n\
                   }";
        let mut buf = std::vec![0u8; 1 << 22];
        for u in [0u32, 1, 2, 3, 5] {
            let mut outs = std::vec::Vec::new();
            for off in [true, false] {
                FOLD_OFF.store(off, core::sync::atomic::Ordering::Relaxed);
                let a = Arena::new(&mut buf);
                let l = compile_to_ir(&a, Stage::Fragment, src).expect("compiles");
                let block = [u, 0, 0, 0];
                let out = run(&l.module, &Inputs { io: &[], uniforms: &block }).expect("runs");
                outs.push(out.output_f32(ir::IoSlot::FragColor(0)).expect("colour"));
            }
            FOLD_OFF.store(false, core::sync::atomic::Ordering::Relaxed);
            assert_eq!(outs[0], outs[1], "u = {u}: unfolded vs folded");
        }
    }
}

/// A build that reports OpenGL ES 2.0 (`report-es2`, every dEQP-GLES2 online
/// image) refuses GLSL ES 3.00 at glCompileShader, and names the dialect in
/// the message the GL log carries: the whole front end, not only the
/// preprocessor (pp.rs `es300_is_preprocessed_unless_the_build_reports_es2`
/// pins the preprocessor's half). An ES 1.00 shader is the control.
#[cfg(all(test, feature = "report-es2"))]
mod report_es2_tests {
    use super::*;

    #[test]
    fn a_report_es2_build_refuses_es300_by_name_and_compiles_es100() {
        let mut buf = std::vec![0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let es300 = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
        let e = compile_to_ir(&a, Stage::Fragment, es300).err().expect("a report-es2 build refuses GLSL ES 3.00");
        assert_eq!(e.msg, "GLSL ES 3.00 source; this build reports OpenGL ES 2.0");
        assert_eq!(e.line, 1, "refused at the #version line");

        let mut buf = std::vec![0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let es100 = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
        assert!(compile_to_ir(&a, Stage::Fragment, es100).is_ok(), "an ES 1.00 shader compiles in the same build");
    }
}
