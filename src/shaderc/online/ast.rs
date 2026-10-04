// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL ES 1.00 syntax tree.
//!
//! Every node is `Copy` and lives in the compile's arena; a node refers to
//! another by a `u32` index into the arena vector that holds it, never by a
//! pointer. That is what lets the tree be built bottom-up in a bump allocator
//! whose blocks move when they grow: an index survives the move, a reference
//! would not.
//!
//! The tree is what the source *said*, not what it means. No type is resolved
//! here, no constructor is told apart from a function call, and a reserved
//! keyword is parsed rather than refused -- all of that is semantic analysis,
//! which can then produce a diagnostic that names the construct instead of
//! failing at a token.

/// Index of an expression in [`Ast::exprs`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExprId(pub u32);
/// Index of a statement in [`Ast::stmts`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StmtId(pub u32);
/// Index of a declaration in [`Ast::decls`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeclId(pub u32);

/// A contiguous run in one of the arena vectors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: u32,
    pub len: u32,
}

impl Range {
    pub const EMPTY: Range = Range { start: 0, len: 0 };

    pub fn indices(&self) -> core::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The precision a declaration carries, or the default for its type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Precision {
    /// None written: the stage's default for the type applies.
    Default,
    Low,
    Medium,
    High,
}

/// A type without its array dimension.
///
/// GLSL ES 1.00 has no non-square matrices, no unsigned integers and two
/// sampler types; `matN` is N columns of `vecN`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BaseType<'a> {
    Void,
    Float,
    Int,
    Bool,
    /// `vec2`..`vec4`.
    Vec(u8),
    /// `ivec2`..`ivec4`.
    IVec(u8),
    /// `bvec2`..`bvec4`.
    BVec(u8),
    /// ES 3.00 `uint`.
    Uint,
    /// ES 3.00 `uvec2`..`uvec4`.
    UVec(u8),
    /// `matCxR`: C columns of R rows (`mat3` is `Mat(3, 3)`; ES 3.00 adds
    /// the non-square ones, `mat2x3` = `Mat(2, 3)`).
    Mat(u8, u8),
    /// A sampler: `sampler2D` and `samplerCube` in ES 1.00, and ES 3.00's
    /// 3D, array, shadow, and integer ones (section 4.1.7).
    Sampler(SamplerType),
    /// A struct by name, declared elsewhere in the unit.
    Struct(&'a str),
}

/// A sampler's dimensionality.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SamplerDim {
    D2,
    D3,
    Cube,
    D2Array,
}

/// What a sampler's look-ups return: `sampler*` float, `isampler*` int,
/// `usampler*` uint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SamplerRet {
    Float,
    Int,
    Uint,
}

/// One of the sampler types of section 4.1.7.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SamplerType {
    pub dim: SamplerDim,
    pub ret: SamplerRet,
    /// `sampler2DShadow`, `samplerCubeShadow`, `sampler2DArrayShadow`.
    pub shadow: bool,
}

impl SamplerType {
    pub const fn new(dim: SamplerDim, ret: SamplerRet, shadow: bool) -> SamplerType {
        SamplerType { dim, ret, shadow }
    }

    /// The type's name as written.
    pub const fn name(self) -> &'static str {
        use SamplerDim::*;
        use SamplerRet::*;
        match (self.ret, self.dim, self.shadow) {
            (Float, D2, false) => "sampler2D",
            (Float, D3, false) => "sampler3D",
            (Float, Cube, false) => "samplerCube",
            (Float, D2Array, false) => "sampler2DArray",
            (Float, D2, true) => "sampler2DShadow",
            (Float, Cube, true) => "samplerCubeShadow",
            (Float, D2Array, true) => "sampler2DArrayShadow",
            (Int, D2, _) => "isampler2D",
            (Int, D3, _) => "isampler3D",
            (Int, Cube, _) => "isamplerCube",
            (Int, D2Array, _) => "isampler2DArray",
            (Uint, D2, _) => "usampler2D",
            (Uint, D3, _) => "usampler3D",
            (Uint, Cube, _) => "usamplerCube",
            (Uint, D2Array, _) => "usampler2DArray",
            (Float, D3, true) => "sampler3DShadow",
        }
    }
}

impl<'a> BaseType<'a> {
    /// ES 1.00's two sampler types.
    pub const SAMPLER_2D: BaseType<'a> = BaseType::Sampler(SamplerType::new(SamplerDim::D2, SamplerRet::Float, false));
    pub const SAMPLER_CUBE: BaseType<'a> =
        BaseType::Sampler(SamplerType::new(SamplerDim::Cube, SamplerRet::Float, false));
}

impl BaseType<'_> {
    /// Components a value of this type has, counting a matrix by column.
    /// Zero for the types that have no components of their own.
    pub fn components(&self) -> u8 {
        match self {
            BaseType::Float | BaseType::Int | BaseType::Uint | BaseType::Bool => 1,
            BaseType::Vec(n) | BaseType::IVec(n) | BaseType::UVec(n) | BaseType::BVec(n) | BaseType::Mat(n, _) => *n,
            BaseType::Void | BaseType::Sampler(_) | BaseType::Struct(_) => 0,
        }
    }

    pub fn is_sampler(&self) -> bool {
        matches!(self, BaseType::Sampler(_))
    }

    /// The lanes one slot of this type holds: a matrix's ROWS (one column
    /// per slot), anything else its components.
    pub fn slot_width(&self) -> u8 {
        match self {
            BaseType::Mat(_, r) => *r,
            other => other.components(),
        }
    }
}

/// A type as written: a base type, a precision, and an optional array size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TypeSpec<'a> {
    pub base: BaseType<'a>,
    pub precision: Precision,
    /// `float a[3]`: the expression inside the brackets, folded to a constant
    /// later. `None` for a non-array.
    pub array_len: Option<ExprId>,
    /// ES 3.00 `float a[] = ...` / `float[] a = ...`: an array whose size is
    /// its initialiser's (section 4.1.9). `array_len` is `None` then; the
    /// checker takes the length from the initialiser's type, and the lowering
    /// asks the checker.
    pub implicit_size: bool,
}

impl TypeSpec<'_> {
    /// Written as an array, sized or not.
    pub fn is_array(&self) -> bool {
        self.array_len.is_some() || self.implicit_size
    }
}

/// What a declaration is for (section 4.3). ES 1.00 has no `in`/`out` on a
/// global; those are parameter qualifiers.
///
/// GLSL ES 3.00 spells the interface `in` and `out` whatever the stage. The
/// parser records them as [`StorageQualifier::In`] / [`StorageQualifier::Out`]
/// because it does not know the stage, and [`Ast::resolve_io`] turns them into
/// what they are -- a vertex `in` is an attribute, a vertex `out` and a
/// fragment `in` are varyings, a fragment `out` is a [`StorageQualifier::FragOut`]
/// -- before any later pass sees them, so the rest of the front end has one
/// vocabulary for both languages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageQualifier {
    /// A local, or a global with no qualifier.
    None,
    Const,
    Attribute,
    Uniform,
    Varying,
    /// ES 3.00 `in`, before [`Ast::resolve_io`].
    In,
    /// ES 3.00 `out`, before [`Ast::resolve_io`].
    Out,
    /// ES 3.00 fragment `out`: a colour output at its layout location
    /// (section 4.3.6), which replaces `gl_FragColor` / `gl_FragData`.
    FragOut,
}

/// An ES 3.00 interpolation qualifier (section 4.3.9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interp {
    /// None written: smooth.
    Default,
    Smooth,
    Flat,
}

/// The interface qualifiers GLSL ES 3.00 adds to a global `in` / `out`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IoQual {
    pub interp: Interp,
    pub centroid: bool,
    /// `layout(location = N)`.
    pub location: Option<u32>,
}

impl IoQual {
    /// No interface qualifier: every ES 1.00 declaration, and every ES 3.00
    /// one that is not an `in` or `out` with one.
    pub const NONE: IoQual = IoQual { interp: Interp::Default, centroid: false, location: None };
}

/// A function parameter's direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamDirection {
    In,
    Out,
    InOut,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Plus,
    Minus,
    Not,
    /// `~`: reserved in ES 1.00, parsed so the error can name it.
    BitNot,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    /// `%`: reserved in ES 1.00.
    Mod,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    AndAnd,
    OrOr,
    XorXor,
    /// `&`, `|`, `^`, `<<`, `>>`: reserved in ES 1.00.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// One expression node.
#[derive(Clone, Copy, Debug)]
pub enum Expr<'a> {
    IntLit(u32),
    /// ES 3.00 `1u`: an unsigned integer constant (section 4.1.3).
    UintLit(u32),
    FloatLit(f32),
    BoolLit(bool),
    /// A variable, or the name of a function being called.
    Ident(&'a str),
    Unary(UnOp, ExprId),
    Binary(BinOp, ExprId, ExprId),
    /// `a = b`, or `a += b` with the operator.
    Assign(Option<BinOp>, ExprId, ExprId),
    /// `c ? t : f`.
    Select(ExprId, ExprId, ExprId),
    /// `a[i]`.
    Index(ExprId, ExprId),
    /// `a.xyz` or `a.member`: the swizzle or member name as written. Which it
    /// is depends on the type of `a`, which the parser does not know.
    Field(ExprId, &'a str),
    /// A function call or a constructor. They share a syntax, and telling
    /// them apart needs to know whether the name is a type.
    Call {
        name: &'a str,
        args: Range,
    },
    /// `a, b`.
    Comma(ExprId, ExprId),
    /// ES 3.00 `T[N](a, b, ...)` or `T[](a, b, ...)`: an array constructor
    /// (section 5.4.4). `len` is the size as written, `None` for `[]`.
    ArrayCtor {
        base: BaseType<'a>,
        len: Option<ExprId>,
        args: Range,
    },
    /// ES 3.00 `a.length()`: the array's size, an integer constant (section
    /// 5.9; the only method the language has).
    Length(ExprId),
}

/// The controlling clause of a `while` or a `for`.
///
/// Section 9's `condition` is `expression | fully_specified_type IDENTIFIER
/// EQUAL initializer`, so `while (bool done = step())` is legal and declares
/// `done` in the loop's scope. `if` and `do ... while` take a plain
/// expression instead, which is why only two statements carry this.
#[derive(Clone, Copy, Debug)]
pub enum Cond {
    Expr(ExprId),
    /// The declaration is the condition; its initialiser is the value tested.
    Decl(DeclId),
}

/// One statement node.
#[derive(Clone, Copy, Debug)]
pub enum Stmt {
    /// `;` alone, or an expression statement.
    Expr(Option<ExprId>),
    /// A declaration inside a function body; the run of [`Ast::decl_ids`] it
    /// produced, since `float a, b;` is two declarations.
    Decl(Range),
    /// `{ ... }`: a run of [`Ast::stmt_ids`].
    Block(Range),
    If {
        cond: ExprId,
        then_s: StmtId,
        else_s: Option<StmtId>,
    },
    While {
        cond: Cond,
        body: StmtId,
    },
    DoWhile {
        body: StmtId,
        cond: ExprId,
    },
    For {
        init: Option<StmtId>,
        cond: Option<Cond>,
        step: Option<ExprId>,
        body: StmtId,
    },
    Break,
    Continue,
    Discard,
    Return(Option<ExprId>),
    /// ES 3.00 section 6.2: `switch (sel) { ... }`. `body` is the
    /// `Stmt::Block` of the braces, whose statements include the
    /// [`Stmt::Case`] labels in the order written.
    Switch {
        sel: ExprId,
        body: StmtId,
    },
    /// `case e:` (`Some`) or `default:` (`None`). A label is a statement of
    /// its switch's body and nothing else; the checker refuses one
    /// anywhere else.
    Case(Option<ExprId>),
}

impl UnOp {
    /// The operator as written, for a diagnostic.
    pub fn text(self) -> &'static str {
        match self {
            UnOp::Plus => "+",
            UnOp::Minus => "-",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
            UnOp::PreInc | UnOp::PostInc => "++",
            UnOp::PreDec | UnOp::PostDec => "--",
        }
    }
}

impl BinOp {
    /// The operator as written, for a diagnostic.
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::AndAnd => "&&",
            BinOp::OrOr => "||",
            BinOp::XorXor => "^^",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }
}

impl Expr<'_> {
    /// What kind of expression this is, in the words a shader author would
    /// use. It names the construct in a diagnostic -- "this expression is not
    /// lowered yet" says nothing a reader can act on, and the front end is
    /// incomplete by construction, so the name is also what the histogram in
    /// `tests/online_lowering.rs` groups by when it decides what to build
    /// next.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Expr::IntLit(_) => "an integer literal",
            Expr::UintLit(_) => "an unsigned integer literal",
            Expr::FloatLit(_) => "a float literal",
            Expr::BoolLit(_) => "a bool literal",
            Expr::Ident(_) => "a name",
            Expr::Unary(op, _) => op.text(),
            Expr::Binary(op, _, _) => op.text(),
            Expr::Assign(None, _, _) => "=",
            Expr::Assign(Some(op), _, _) => match op {
                BinOp::Add => "+=",
                BinOp::Sub => "-=",
                BinOp::Mul => "*=",
                BinOp::Div => "/=",
                BinOp::Mod => "%=",
                BinOp::BitAnd => "&=",
                BinOp::BitOr => "|=",
                BinOp::BitXor => "^=",
                BinOp::Shl => "<<=",
                BinOp::Shr => ">>=",
                _ => "an assigning operator",
            },
            Expr::Select(..) => "?:",
            Expr::Index(..) => "[]",
            Expr::Field(..) => ".",
            Expr::Call { .. } => "a call",
            Expr::Comma(..) => ",",
            Expr::ArrayCtor { .. } => "an array constructor",
            Expr::Length(_) => "length()",
        }
    }
}

impl Stmt {
    /// What kind of statement this is, for the same reason as
    /// [`Expr::kind_name`].
    pub fn kind_name(&self) -> &'static str {
        match self {
            Stmt::Expr(_) => "an expression statement",
            Stmt::Decl(_) => "a local variable",
            Stmt::Block(_) => "a block",
            Stmt::If { .. } => "if",
            Stmt::While { .. } => "while",
            Stmt::DoWhile { .. } => "do",
            Stmt::For { .. } => "for",
            Stmt::Break => "break",
            Stmt::Continue => "continue",
            Stmt::Discard => "discard",
            Stmt::Return(_) => "return",
            Stmt::Switch { .. } => "switch",
            Stmt::Case(Some(_)) => "case",
            Stmt::Case(None) => "default",
        }
    }
}

/// One member of a `struct`.
#[derive(Clone, Copy, Debug)]
pub struct Member<'a> {
    pub ty: TypeSpec<'a>,
    pub name: &'a str,
    pub line: u32,
    /// A uniform block member's own `layout(row_major)` (`Some(true)`) or
    /// `layout(column_major)` (`Some(false)`); `None` takes the block's.
    /// Always `None` for a struct member.
    pub row_major: Option<bool>,
}

/// One function parameter.
#[derive(Clone, Copy, Debug)]
pub struct Param<'a> {
    pub ty: TypeSpec<'a>,
    /// A prototype may leave a parameter unnamed.
    pub name: Option<&'a str>,
    pub direction: ParamDirection,
    pub is_const: bool,
}

/// One top-level or block-level declaration.
#[derive(Clone, Copy, Debug)]
pub enum Decl<'a> {
    /// A variable: one per declarator, so `float a, b;` is two.
    Var {
        storage: StorageQualifier,
        invariant: bool,
        /// ES 3.00 interpolation, `centroid` and `layout(location)`.
        io: IoQual,
        ty: TypeSpec<'a>,
        name: &'a str,
        init: Option<ExprId>,
        line: u32,
    },
    /// `struct S { ... };`
    Struct { name: &'a str, members: Range, line: u32 },
    /// `precision highp float;`
    Precision { precision: Precision, base: BaseType<'a>, line: u32 },
    /// A function prototype, or a definition when `body` is set.
    Function { name: &'a str, ret: TypeSpec<'a>, params: Range, body: Option<StmtId>, line: u32 },
    /// `invariant gl_Position;`
    Invariant { name: &'a str, line: u32 },
}

impl Decl<'_> {
    /// The line the declaration started on.
    pub fn line(&self) -> u32 {
        match self {
            Decl::Var { line, .. }
            | Decl::Struct { line, .. }
            | Decl::Precision { line, .. }
            | Decl::Function { line, .. }
            | Decl::Invariant { line, .. } => *line,
        }
    }
}

/// A parsed translation unit.
///
/// The vectors are the arenas the ids index. `top` is the unit's own
/// top-level declarations, in source order -- and source order is
/// load-bearing, because the reflection's uniform rows come out in it (see
/// `docs/online-shaderc/frontend.md`).
///
/// `top` is its own list rather than a range of [`Ast::decl_ids`] for a
/// reason worth stating, because getting it wrong was silent: declarations
/// inside a function body land in `decl_ids` too, and they land there
/// *before* the enclosing function's own declaration, since the body is
/// parsed first. A "start..end" range over the shared vector therefore
/// swallows every nested declaration, and the top level appears to contain
/// the locals of every function. The same trap applies to a block's
/// statements and a call's arguments; the parser builds each of those runs
/// aside and appends it whole.
pub struct Ast<'a> {
    pub exprs: &'a [Expr<'a>],
    pub stmts: &'a [Stmt],
    pub decls: &'a [Decl<'a>],
    pub members: &'a [Member<'a>],
    pub params: &'a [Param<'a>],
    pub stmt_ids: &'a [StmtId],
    pub decl_ids: &'a [DeclId],
    pub expr_ids: &'a [ExprId],
    /// The source line each expression and statement was parsed at, indexed
    /// by [`ExprId`] / [`StmtId`].
    ///
    /// The later passes work on ids and have no tokens left, so without this
    /// every diagnostic they raise reads `0:0:` and cannot be located in the
    /// source at all -- which is most of what an info log is for.
    pub expr_lines: &'a [u32],
    pub stmt_lines: &'a [u32],
    pub top: &'a [DeclId],
    /// Parsed as GLSL ES 3.00 (`#version 300 es`).
    pub es3: bool,
    /// ES 3.00's uniform blocks, in source order. Each is ALSO in `decls`,
    /// desugared (see [`Block`]); this is what the block table is built
    /// from.
    pub blocks: &'a [Block<'a>],
}

/// A uniform block's packing (ES 3.00 section 4.3.8.3). `shared` and
/// `packed` are laid out as `std140`, which the specification allows: both
/// leave the layout to the implementation, and `std140` is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockPacking {
    Shared,
    Packed,
    Std140,
}

/// `layout(...) uniform Name { members } [instance [N]];` (ES 3.00 section
/// 4.3.7).
///
/// The parser desugars it into what the later passes already know, so the
/// checker, the reflection and the lowering need no block of their own:
/// `struct Name { members };` -- which also puts the block name into the
/// global name space, where section 4.3.7 and dEQP's conflicting_* cases put
/// it -- and then either `uniform Name instance[N];` or, with no instance
/// name, one `uniform T member;` per member. The block's rows are therefore
/// ordinary uniform rows in the default block (the "flattened" design); the
/// GL core copies a bound buffer into them.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    pub name: &'a str,
    pub instance: Option<&'a str>,
    pub array_len: Option<ExprId>,
    /// Into [`Ast::members`], shared with the desugared struct.
    pub members: Range,
    pub packing: BlockPacking,
    /// The block's matrix layout: `row_major`, or the global default in
    /// force (`layout(row_major) uniform;`).
    pub row_major: bool,
    pub line: u32,
}

impl<'a> Ast<'a> {
    pub fn expr(&self, id: ExprId) -> &Expr<'a> {
        &self.exprs[id.0 as usize]
    }

    pub fn stmt(&self, id: StmtId) -> &Stmt {
        &self.stmts[id.0 as usize]
    }

    /// The line `id` was parsed at, or 0 if it is out of range -- which
    /// cannot happen, but a compiler that runs on the target does not panic
    /// to say so.
    pub fn expr_line(&self, id: ExprId) -> u32 {
        self.expr_lines.get(id.0 as usize).copied().unwrap_or(0)
    }

    pub fn stmt_line(&self, id: StmtId) -> u32 {
        self.stmt_lines.get(id.0 as usize).copied().unwrap_or(0)
    }

    pub fn decl(&self, id: DeclId) -> &Decl<'a> {
        &self.decls[id.0 as usize]
    }

    /// The unit's top-level declarations, in source order.
    pub fn top_level(&self) -> impl Iterator<Item = &Decl<'a>> {
        self.top.iter().map(|id| self.decl(*id))
    }

    /// The declarations of a `Stmt::Decl` or the statements of a
    /// `Stmt::Block`, whichever `r` came from.
    pub fn decls_of(&self, r: Range) -> impl Iterator<Item = &Decl<'a>> {
        self.decl_ids[r.indices()].iter().map(|id| self.decl(*id))
    }

    pub fn stmts_of(&self, r: Range) -> impl Iterator<Item = &Stmt> {
        self.stmt_ids[r.indices()].iter().map(|id| self.stmt(*id))
    }

    pub fn args_of(&self, r: Range) -> impl Iterator<Item = &Expr<'a>> {
        self.expr_ids[r.indices()].iter().map(|id| self.expr(*id))
    }
}
