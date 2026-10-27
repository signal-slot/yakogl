// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Recursive descent from the preprocessed token stream to an [`Ast`].
//!
//! The grammar is the one in the OpenGL ES Shading Language 1.00
//! specification, section 9, with the operator precedence of section 5.1.
//! Three things about it are decisions rather than accidents:
//!
//! * **The parser resolves no types.** Telling a constructor from a function
//!   call needs to know whether `vec4` names a type, and it does not decide
//!   that. The one place the grammar genuinely cannot be parsed without such
//!   knowledge is a statement beginning with an identifier: `S s;` versus
//!   `s = 1;`. So the parser tracks the *names of structs declared so far*
//!   and nothing else, which is the minimum the grammar needs; the rest is
//!   semantic analysis, where a diagnostic can name the construct instead of
//!   failing at a token.
//!
//! * **A word reserved for future use is parsed, then refused.** Section 3.6
//!   reserves `switch`, `half`, `sampler3D` and thirty others. Refusing them
//!   in the lexer would produce "unexpected token"; refusing them here
//!   produces "'switch' : reserved for future use", which is what an
//!   application reading `glGetShaderInfoLog` needs. dEQP's `keywords.test`
//!   is 188 cases of exactly this.
//!
//! * **Operators ES 1.00 reserves are parsed at their C precedence.** `%`,
//!   `&`, `|`, `^`, `<<`, `>>` and their assignment forms go into the tree so
//!   that `a % b` can be refused by name later, rather than the parser
//!   stopping at the operator with "expected an expression".
//!   `reserved_operators.test` is 26 cases that read that message.

use super::arena::{AVec, Arena};
use super::ast::*;
use super::lexer::{Kw, Tok, Token};

/// Type names that exist in GLSL ES 3.00 and not in 1.00.
///
/// Only used to say WHY a name is not a type; the refusal is the same
/// either way, so a name missing from this list costs a worse message and
/// never a wrong verdict.
///
/// `sampler3D` and `sampler2DShadow` are deliberately ABSENT: section 3.6
/// reserves them in ES 1.00, so the lexer already classifies them and they
/// answer "reserved for future use", which is the better sentence and comes
/// from the ES 1.00 specification rather than from a later one. Which two
/// those were is measured, not assumed -- every other name below was checked
/// to reach this path.
const ES300_TYPES: &[&str] = &[
    "uint",
    "uvec2",
    "uvec3",
    "uvec4",
    "mat2x2",
    "mat2x3",
    "mat2x4",
    "mat3x2",
    "mat3x3",
    "mat3x4",
    "mat4x2",
    "mat4x3",
    "mat4x4",
    "sampler2DArray",
    "samplerCubeShadow",
    "sampler2DArrayShadow",
    "isampler2D",
    "isampler3D",
    "isamplerCube",
    "isampler2DArray",
    "usampler2D",
    "usampler3D",
    "usamplerCube",
    "usampler2DArray",
];
use super::{Error, R};

/// Nesting depth the parser accepts before giving up.
///
/// A shader can nest expressions and blocks arbitrarily and the parser is
/// recursive, so without a limit a generated (or hostile) shader would
/// overflow the stack -- 16 KiB on the kernel harness. Exceeding it is a
/// diagnosable refusal, not a crash.
const MAX_DEPTH: u32 = 128;

/// Parse a preprocessed token stream.
pub fn parse<'a>(a: &Arena<'a>, toks: &'a [Token<'a>]) -> R<'a, Ast<'a>> {
    let mut p = Parser {
        a,
        toks,
        i: 0,
        depth: 0,
        exprs: AVec::new(),
        stmts: AVec::new(),
        expr_lines: AVec::new(),
        stmt_lines: AVec::new(),
        decls: AVec::new(),
        members: AVec::new(),
        params: AVec::new(),
        stmt_ids: AVec::new(),
        decl_ids: AVec::new(),
        expr_ids: AVec::new(),
        structs: AVec::new(),
        pending_struct: None,
        blocks: AVec::new(),
        default_packing: BlockPacking::Shared,
        default_row_major: false,
        layout: Layout::NONE,
    };
    let top = p.translation_unit()?;
    Ok(Ast {
        exprs: p.exprs.into_slice(),
        stmts: p.stmts.into_slice(),
        decls: p.decls.into_slice(),
        members: p.members.into_slice(),
        params: p.params.into_slice(),
        stmt_ids: p.stmt_ids.into_slice(),
        decl_ids: p.decl_ids.into_slice(),
        expr_ids: p.expr_ids.into_slice(),
        expr_lines: p.expr_lines.into_slice(),
        stmt_lines: p.stmt_lines.into_slice(),
        top,
        es3: toks.iter().any(|t| t.es3),
        blocks: p.blocks.into_slice(),
    })
}

/// What one `layout(...)` said. `location` is for `in` and `out`; the rest
/// are uniform block qualifiers (ES 3.00 section 4.3.8.3), where a repeated
/// or contradicting qualifier is allowed and the last one wins.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Layout {
    location: Option<u32>,
    packing: Option<BlockPacking>,
    row_major: Option<bool>,
}

impl Layout {
    const NONE: Layout = Layout { location: None, packing: None, row_major: None };
}

/// Give an ES 3.00 unit's `in` and `out` the meaning its stage gives them
/// (see [`StorageQualifier`]): a vertex `in` is an attribute, a vertex `out`
/// and a fragment `in` are varyings, a fragment `out` is a colour output.
/// The declarations are copied with the resolved qualifier; nothing else
/// changes, and an ES 1.00 unit is returned as it came.
///
/// Done once, here, rather than asked of the stage in every later pass: the
/// type checker, the reflection and the lowering then have one vocabulary for
/// both languages, and the ES 1.00 rules they already implement for
/// attributes and varyings apply to ES 3.00's inputs and outputs unchanged.
pub fn resolve_io<'a>(a: &Arena<'a>, ast: Ast<'a>, stage: super::sema::Stage) -> R<'a, Ast<'a>> {
    if !ast.es3 {
        return Ok(ast);
    }
    let vertex = stage == super::sema::Stage::Vertex;
    let mut decls = AVec::with_capacity(a, ast.decls.len())?;
    for d in ast.decls.iter() {
        let d = match *d {
            Decl::Var {
                storage: s @ (StorageQualifier::In | StorageQualifier::Out),
                invariant,
                io,
                ty,
                name,
                init,
                line,
            } => {
                let storage = match (s, vertex) {
                    (StorageQualifier::In, true) => StorageQualifier::Attribute,
                    (StorageQualifier::Out, false) => StorageQualifier::FragOut,
                    _ => StorageQualifier::Varying,
                };
                Decl::Var { storage, invariant, io, ty, name, init, line }
            }
            other => other,
        };
        decls.push(a, d)?;
    }
    Ok(Ast { decls: decls.into_slice(), ..ast })
}

struct Parser<'x, 'a> {
    a: &'x Arena<'a>,
    toks: &'a [Token<'a>],
    i: usize,
    depth: u32,
    exprs: AVec<'a, Expr<'a>>,
    stmts: AVec<'a, Stmt>,
    /// One line per entry of `exprs` / `stmts`, same index. See
    /// [`Parser::push_expr`].
    expr_lines: AVec<'a, u32>,
    stmt_lines: AVec<'a, u32>,
    decls: AVec<'a, Decl<'a>>,
    members: AVec<'a, Member<'a>>,
    params: AVec<'a, Param<'a>>,
    stmt_ids: AVec<'a, StmtId>,
    decl_ids: AVec<'a, DeclId>,
    expr_ids: AVec<'a, ExprId>,
    /// Names of the structs declared so far. The grammar needs this and
    /// nothing else: `S s;` is a declaration only if `S` names a type.
    structs: AVec<'a, &'a str>,
    /// A `struct S { ... }` met while reading a declaration's type. It is a
    /// declaration in its own right and must come out *before* the variable
    /// it typed, so it waits here rather than being appended from inside the
    /// type parser -- which would put it after the run the caller is
    /// building.
    pending_struct: Option<DeclId>,
    blocks: AVec<'a, Block<'a>>,
    /// `layout(std140) uniform;` and `layout(row_major) uniform;`: the
    /// defaults later blocks take (section 4.3.8.3).
    default_packing: BlockPacking,
    default_row_major: bool,
    /// The block qualifiers of the `layout(...)` just read by
    /// [`Parser::qualifiers`], for the declaration that follows it.
    layout: Layout,
}

impl<'a> Parser<'_, 'a> {
    /* ---- the token cursor ---------------------------------------------- */

    fn peek(&self) -> Token<'a> {
        self.toks[self.i.min(self.toks.len() - 1)]
    }

    fn peek_at(&self, n: usize) -> Token<'a> {
        self.toks[(self.i + n).min(self.toks.len() - 1)]
    }

    fn bump(&mut self) -> Token<'a> {
        let t = self.peek();
        if t.kind != Tok::Eof {
            self.i += 1;
        }
        t
    }

    fn at(&self, k: Tok) -> bool {
        self.peek().kind == k
    }

    fn eat(&mut self, k: Tok) -> bool {
        if self.at(k) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, k: Tok, what: &'static str) -> R<'a, Token<'a>> {
        if self.at(k) {
            return Ok(self.bump());
        }
        Err(self.err(what))
    }

    fn err(&self, msg: &'static str) -> Error<'a> {
        let t = self.peek();
        /* `Eof` has no spelling of its own; quoting "" reads better in an
         * info log than quoting nothing at all. */
        let tok = if t.kind == Tok::Eof { "" } else { t.text };
        Error::new(t.line, tok, msg)
    }

    /// An identifier that is not a keyword. A word reserved for future use
    /// gets its own message, because "unexpected token" would send the
    /// application's author hunting for a typo.
    fn ident(&mut self, what: &'static str) -> R<'a, &'a str> {
        let t = self.peek();
        if t.kind != Tok::Ident {
            return Err(self.err(what));
        }
        match t.kw() {
            None => {
                self.i += 1;
                Ok(t.text)
            }
            Some(Kw::Reserved) => Err(Error::new(t.line, t.text, "reserved for future use")),
            Some(_) => Err(Error::new(t.line, t.text, what)),
        }
    }

    fn enter(&mut self) -> R<'a, ()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    /* ---- arena pushes -------------------------------------------------- */

    /* Every node records the line it was built at, so that a later pass can
     * say WHERE. The lowering and the checker work on ids and have no tokens
     * left, and `0:0:` on every message from them makes a rejection
     * impossible to locate in the source -- which is most of what an info log
     * is for. The line is the one under the parser's cursor when the node is
     * made, which for an expression is its last token and for a statement its
     * terminator; close enough to point at, and it costs one `u32` a node. */
    fn push_expr(&mut self, e: Expr<'a>) -> R<'a, ExprId> {
        let id = ExprId(self.exprs.len() as u32);
        self.exprs.push(self.a, e)?;
        let line = self.peek().line;
        self.expr_lines.push(self.a, line)?;
        Ok(id)
    }

    fn push_stmt(&mut self, s: Stmt) -> R<'a, StmtId> {
        let id = StmtId(self.stmts.len() as u32);
        self.stmts.push(self.a, s)?;
        let line = self.peek().line;
        self.stmt_lines.push(self.a, line)?;
        Ok(id)
    }

    fn push_decl(&mut self, d: Decl<'a>) -> R<'a, DeclId> {
        let id = DeclId(self.decls.len() as u32);
        self.decls.push(self.a, d)?;
        Ok(id)
    }

    /* ---- the unit ------------------------------------------------------ */

    fn translation_unit(&mut self) -> R<'a, &'a [DeclId]> {
        let mut top: AVec<DeclId> = AVec::new();
        while !self.at(Tok::Eof) {
            let r = self.external_declaration()?;
            let ids = self.a.alloc_copy(&self.decl_ids.as_slice()[r.indices()])?;
            top.extend_from_slice(self.a, ids)?;
        }
        Ok(top.into_slice())
    }

    /// One top-level declaration or function definition, as the contiguous
    /// run of `decl_ids` it produced.
    fn external_declaration(&mut self) -> R<'a, Range> {
        if self.peek().kw() == Some(Kw::Precision) {
            let id = self.precision_declaration()?;
            let start = self.decl_ids.len() as u32;
            self.decl_ids.push(self.a, id)?;
            return Ok(Range { start, len: 1 });
        }
        /* `invariant gl_Position;` on its own (section 4.6.1). `invariant`
         * before a `varying` is a qualifier and goes through `qualifiers`. */
        if self.peek().kw() == Some(Kw::Invariant)
            && self.peek_at(1).kind == Tok::Ident
            && self.peek_at(1).kw().is_none()
        {
            let line = self.bump().line;
            let name = self.ident("expected a name after 'invariant'")?;
            self.expect(Tok::Semi, "expected ';'")?;
            let id = self.push_decl(Decl::Invariant { name, line })?;
            let start = self.decl_ids.len() as u32;
            self.decl_ids.push(self.a, id)?;
            return Ok(Range { start, len: 1 });
        }
        self.declaration_or_function()
    }

    fn precision_declaration(&mut self) -> R<'a, DeclId> {
        let line = self.bump().line; // `precision`
        let precision = match self.peek().kw() {
            Some(Kw::LowP) => Precision::Low,
            Some(Kw::MediumP) => Precision::Medium,
            Some(Kw::HighP) => Precision::High,
            _ => return Err(self.err("expected a precision qualifier")),
        };
        self.i += 1;
        let base = self.base_type()?;
        self.expect(Tok::Semi, "expected ';'")?;
        self.push_decl(Decl::Precision { precision, base, line })
    }

    /// A declaration, or a function prototype or definition, as the
    /// contiguous run of `decl_ids` it produced.
    ///
    /// The ids are gathered aside and appended in one go, because parsing a
    /// function body appends to `decl_ids` in the middle of this: a
    /// "start..end" range taken across that would contain every local the
    /// body declared.
    fn declaration_or_function(&mut self) -> R<'a, Range> {
        let line = self.peek().line;
        let mut ids: AVec<DeclId> = AVec::new();
        let (storage, invariant, io) = self.qualifiers()?;
        if storage == StorageQualifier::Uniform && self.peek().es3 {
            /* `layout(std140) uniform;`: the default for the blocks after it
             * (section 4.3.8.3). */
            if self.at(Tok::Semi) && self.layout != Layout::NONE && !invariant {
                self.i += 1;
                if let Some(p) = self.layout.packing {
                    self.default_packing = p;
                }
                if let Some(r) = self.layout.row_major {
                    self.default_row_major = r;
                }
                return self.append_decl_ids(&ids);
            }
            /* `uniform Name {`: a uniform block (section 4.3.7). */
            if self.at(Tok::Ident) && self.peek().kw().is_none() && self.peek_at(1).kind == Tok::LBrace {
                return self.uniform_block(line, invariant, &mut ids);
            }
        }
        let block_quals = self.layout.packing.is_some() || self.layout.row_major.is_some();
        if block_quals {
            return Err(Error::new(line, "layout", "a block layout qualifier applies only to a uniform block"));
        }
        let (base, precision) = self.type_with_precision()?;
        if let Some(id) = self.pending_struct.take() {
            ids.push(self.a, id)?;
        }

        /* `struct S { ... };` with no declarator: the struct declaration is
         * the whole of it. */
        if self.at(Tok::Semi) {
            self.i += 1;
            if storage != StorageQualifier::None {
                return Err(Error::new(line, "", "a qualifier needs a declaration"));
            }
            return self.append_decl_ids(&ids);
        }

        /* ES 3.00's `float[3] a;`: the size on the TYPE (section 4.1.9),
         * which every declarator then shares. */
        let array_ty = self.type_array_suffix()?;
        let name_line = self.peek().line;
        let mut name = self.ident("expected a name")?;

        if self.at(Tok::LParen) {
            if storage != StorageQualifier::None || invariant {
                return Err(Error::new(name_line, name, "a function cannot have that qualifier"));
            }
            let (array_len, implicit_size) = match array_ty {
                /* ES 3.00 section 6.1: an array return type "must be
                 * explicitly sized". */
                Some(None) => return Err(Error::new(name_line, name, "an array return type must be sized")),
                Some(Some(e)) => (Some(e), false),
                None => (None, false),
            };
            return self.function_rest(&mut ids, name, TypeSpec { base, precision, array_len, implicit_size }, line);
        }

        /* An init-declarator list: `float a, b[2], c = 1.0;` becomes one
         * declaration per name, in source order, because that order is what
         * the reflection's uniform rows carry out to the application. */
        loop {
            let (array_len, implicit_size) = self.declarator_array(array_ty)?;
            let init = if self.eat(Tok::Assign) { Some(self.assignment_expr()?) } else { None };
            let id = self.push_decl(Decl::Var {
                storage,
                invariant,
                io,
                ty: TypeSpec { base, precision, array_len, implicit_size },
                name,
                init,
                line,
            })?;
            ids.push(self.a, id)?;
            if !self.eat(Tok::Comma) {
                break;
            }
            name = self.ident("expected a name after ','")?;
        }
        self.expect(Tok::Semi, "expected ';'")?;
        self.append_decl_ids(&ids)
    }

    /// `uniform Name { members } [instance [N]];`, the cursor on `Name`, and
    /// its desugaring (see [`Block`]): a struct named after the block, then
    /// the instance, or one uniform per member.
    fn uniform_block(&mut self, line: u32, invariant: bool, ids: &mut AVec<'a, DeclId>) -> R<'a, Range> {
        if invariant {
            return Err(Error::new(line, "invariant", "a uniform block cannot be invariant"));
        }
        let block_layout = self.layout;
        let name_line = self.peek().line;
        let name = self.ident("expected a block name")?;
        if name.starts_with("gl_") {
            return Err(Error::new(name_line, name, "a block name may not begin with 'gl_'"));
        }
        if self.is_struct_name(name) {
            return Err(Error::new(name_line, name, "this name is already a type or a block"));
        }
        self.expect(Tok::LBrace, "expected '{'")?;
        let mstart = self.members.len() as u32;
        while !self.at(Tok::RBrace) {
            if self.at(Tok::Eof) {
                return Err(self.err("expected '}' after the block's members"));
            }
            let mline = self.peek().line;
            let mut row_major = None;
            if self.peek().kw() == Some(Kw::Layout) {
                let l = self.layout_qualifier()?;
                if l.location.is_some() || l.packing.is_some() {
                    return Err(Error::new(mline, "layout", "a block member takes only 'row_major' or 'column_major'"));
                }
                row_major = l.row_major;
            }
            /* Section 4.3.7: a member may repeat `uniform`, and may have no
             * other storage or interface qualifier. */
            match self.peek().kw() {
                Some(Kw::Uniform) => self.i += 1,
                Some(
                    Kw::In
                    | Kw::Out
                    | Kw::InOut
                    | Kw::Attribute
                    | Kw::Varying
                    | Kw::Const
                    | Kw::Centroid
                    | Kw::Flat
                    | Kw::Smooth
                    | Kw::Invariant,
                ) => {
                    let t = self.peek();
                    return Err(Error::new(t.line, t.text, "a uniform block member cannot have this qualifier"));
                }
                _ => {}
            }
            let (base, precision) = self.type_with_precision()?;
            if self.pending_struct.take().is_some() {
                return Err(Error::new(mline, "struct", "a structure cannot be defined in a uniform block"));
            }
            if base.is_sampler() {
                return Err(Error::new(mline, "", "a sampler cannot be a member of a uniform block"));
            }
            let array_ty = self.type_array_suffix()?;
            loop {
                let ml = self.peek().line;
                let mname = self.ident("expected a member name")?;
                let (array_len, implicit_size) = self.declarator_array(array_ty)?;
                if implicit_size {
                    return Err(Error::new(ml, mname, "a block member array must be sized"));
                }
                self.members.push(
                    self.a,
                    Member {
                        ty: TypeSpec { base, precision, array_len, implicit_size },
                        name: mname,
                        line: ml,
                        row_major,
                    },
                )?;
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::Semi, "expected ';' after a block member")?;
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        let members = Range { start: mstart, len: self.members.len() as u32 - mstart };
        if members.is_empty() {
            return Err(Error::new(line, name, "a uniform block must have at least one member"));
        }
        let (instance, array_len) = if self.at(Tok::Ident) {
            let il = self.peek().line;
            let inst = self.ident("expected an instance name")?;
            if inst == name {
                return Err(Error::new(il, inst, "an instance may not have its block's name"));
            }
            let (len, implicit) = self.declarator_array(None)?;
            if implicit {
                return Err(Error::new(il, inst, "an array of blocks must be sized"));
            }
            (Some(inst), len)
        } else {
            (None, None)
        };
        self.expect(Tok::Semi, "expected ';' after a uniform block")?;

        let packing = block_layout.packing.unwrap_or(self.default_packing);
        let row_major = block_layout.row_major.unwrap_or(self.default_row_major);
        self.blocks.push(self.a, Block { name, instance, array_len, members, packing, row_major, line })?;
        /* The desugaring. */
        self.structs.push(self.a, name)?;
        let sid = self.push_decl(Decl::Struct { name, members, line })?;
        ids.push(self.a, sid)?;
        let io = IoQual::NONE;
        match instance {
            Some(inst) => {
                let ty = TypeSpec {
                    base: BaseType::Struct(name),
                    precision: Precision::Default,
                    array_len,
                    implicit_size: false,
                };
                let id = self.push_decl(Decl::Var {
                    storage: StorageQualifier::Uniform,
                    invariant: false,
                    io,
                    ty,
                    name: inst,
                    init: None,
                    line,
                })?;
                ids.push(self.a, id)?;
            }
            None => {
                for k in members.indices() {
                    let m = self.members.as_slice()[k];
                    let id = self.push_decl(Decl::Var {
                        storage: StorageQualifier::Uniform,
                        invariant: false,
                        io,
                        ty: m.ty,
                        name: m.name,
                        init: None,
                        line: m.line,
                    })?;
                    ids.push(self.a, id)?;
                }
            }
        }
        self.append_decl_ids(ids)
    }

    /// Append a gathered run of declaration ids to `decl_ids` in one go, and
    /// return the range it occupies.
    fn append_decl_ids(&mut self, ids: &AVec<'a, DeclId>) -> R<'a, Range> {
        let start = self.decl_ids.len() as u32;
        /* Copy first: `extend_from_slice` borrows `self.decl_ids` mutably
         * while `ids` borrows the same arena, so the slice is taken out of
         * the way before the append. */
        let src = self.a.alloc_copy(ids.as_slice())?;
        self.decl_ids.extend_from_slice(self.a, src)?;
        Ok(Range { start, len: src.len() as u32 })
    }

    /// The part of a function after its name, starting at `(`.
    fn function_rest(
        &mut self,
        ids: &mut AVec<'a, DeclId>,
        name: &'a str,
        ret: TypeSpec<'a>,
        line: u32,
    ) -> R<'a, Range> {
        self.expect(Tok::LParen, "expected '('")?;
        let pstart = self.params.len() as u32;
        /* `void` alone is an empty parameter list, not a parameter. */
        if self.peek().kw() == Some(Kw::Void) && self.peek_at(1).kind == Tok::RParen {
            self.i += 1;
        } else if !self.at(Tok::RParen) {
            loop {
                let p = self.parameter()?;
                self.params.push(self.a, p)?;
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "expected ')'")?;
        let params = Range { start: pstart, len: self.params.len() as u32 - pstart };

        let body = if self.at(Tok::LBrace) { Some(self.compound_statement()?) } else { None };
        if body.is_none() {
            self.expect(Tok::Semi, "expected ';' or a function body")?;
        }
        let id = self.push_decl(Decl::Function { name, ret, params, body, line })?;
        ids.push(self.a, id)?;
        self.append_decl_ids(ids)
    }

    fn parameter(&mut self) -> R<'a, Param<'a>> {
        let mut is_const = false;
        if self.peek().kw() == Some(Kw::Const) {
            is_const = true;
            self.i += 1;
        }
        let direction = match self.peek().kw() {
            Some(Kw::In) => {
                self.i += 1;
                ParamDirection::In
            }
            Some(Kw::Out) => {
                self.i += 1;
                ParamDirection::Out
            }
            Some(Kw::InOut) => {
                self.i += 1;
                ParamDirection::InOut
            }
            _ => ParamDirection::In,
        };
        let (base, precision) = self.type_with_precision()?;
        let array_ty = self.type_array_suffix()?;
        /* A prototype may leave a parameter unnamed: `void f(float);` */
        let name = if self.at(Tok::Ident) && self.peek().kw().is_none() {
            Some(self.ident("expected a parameter name")?)
        } else {
            None
        };
        let (array_len, implicit_size) = self.declarator_array(array_ty)?;
        if implicit_size {
            /* ES 3.00 section 6.1: "Arrays are allowed as arguments and as
             * the return type. In both cases, the array must be explicitly
             * sized." */
            return Err(self.err("an array parameter must be sized"));
        }
        Ok(Param { ty: TypeSpec { base, precision, array_len, implicit_size }, name, direction, is_const })
    }

    /// `[expr]` after a name, if present.
    fn array_suffix(&mut self) -> R<'a, Option<ExprId>> {
        if !self.eat(Tok::LBracket) {
            return Ok(None);
        }
        let e = self.conditional_expr()?;
        self.expect(Tok::RBracket, "expected ']'")?;
        Ok(Some(e))
    }

    /// `[expr]` or `[]` after a TYPE (ES 3.00 section 4.1.9), if present:
    /// `Some(Some(size))`, `Some(None)` for `[]`, `None` for no brackets.
    /// ES 1.00 has no array type, only an array declarator, so there it is
    /// always `None` and the `[` is left for whatever follows to refuse.
    fn type_array_suffix(&mut self) -> R<'a, Option<Option<ExprId>>> {
        if !self.peek().es3 || !self.at(Tok::LBracket) {
            return Ok(None);
        }
        self.i += 1;
        if self.eat(Tok::RBracket) {
            return Ok(Some(None));
        }
        let e = self.conditional_expr()?;
        self.expect(Tok::RBracket, "expected ']'")?;
        Ok(Some(Some(e)))
    }

    /// A declarator's own `[...]`, combined with the type's: at most one of
    /// the two, since ES 3.00 section 4.1.9 has only one-dimensional arrays
    /// ("float[5] a[3]; // illegal"). `[]` is ES 3.00's unsized array, sized
    /// by its initialiser; ES 1.00 has no such form.
    fn declarator_array(&mut self, ty: Option<Option<ExprId>>) -> R<'a, (Option<ExprId>, bool)> {
        let own = if self.at(Tok::LBracket) && self.peek().es3 && self.peek_at(1).kind == Tok::RBracket {
            self.i += 2;
            Some(None)
        } else {
            self.array_suffix()?.map(Some)
        };
        if (ty.is_some() && own.is_some()) || self.at(Tok::LBracket) {
            return Err(self.err("an array of arrays is not allowed"));
        }
        Ok(match ty.or(own) {
            Some(Some(e)) => (Some(e), false),
            Some(None) => (None, true),
            None => (None, false),
        })
    }

    /* ---- qualifiers and types ------------------------------------------ */

    /// The qualifiers in front of a type, in the order the grammar allows
    /// them. ES 1.00: `invariant`? then one storage qualifier. ES 3.00
    /// (section 4.7 and the grammar's `type_qualifier`): `layout(...)`
    /// before a storage qualifier, or `invariant`? then an interpolation
    /// qualifier? then `centroid`? then the storage qualifier -- in that
    /// order, which is what dEQP's `qualification_order` checks.
    fn qualifiers(&mut self) -> R<'a, (StorageQualifier, bool, IoQual)> {
        let mut invariant = false;
        let mut io = IoQual::NONE;
        let line = self.peek().line;
        self.layout = Layout::NONE;
        if self.peek().kw() == Some(Kw::Layout) {
            self.layout = self.layout_qualifier()?;
            io.location = self.layout.location;
        } else {
            if self.peek().kw() == Some(Kw::Invariant) {
                invariant = true;
                self.i += 1;
            }
            match self.peek().kw() {
                Some(Kw::Smooth) => {
                    io.interp = Interp::Smooth;
                    self.i += 1;
                }
                Some(Kw::Flat) => {
                    io.interp = Interp::Flat;
                    self.i += 1;
                }
                _ => {}
            }
        }
        if self.peek().kw() == Some(Kw::Centroid) {
            io.centroid = true;
            self.i += 1;
            if !matches!(self.peek().kw(), Some(Kw::In | Kw::Out)) {
                return Err(Error::new(line, "centroid", "'centroid' must be followed by 'in' or 'out'"));
            }
        }
        let storage = match self.peek().kw() {
            Some(Kw::Const) => StorageQualifier::Const,
            Some(Kw::Attribute) => StorageQualifier::Attribute,
            Some(Kw::Uniform) => StorageQualifier::Uniform,
            Some(Kw::Varying) => StorageQualifier::Varying,
            /* Only an ES 3.00 token is a storage `in` / `out`: in 1.00 they
             * are parameter directions and nothing else. */
            Some(Kw::In) if self.peek().es3 => StorageQualifier::In,
            Some(Kw::Out) if self.peek().es3 => StorageQualifier::Out,
            _ => {
                if io != IoQual::NONE || self.layout != Layout::NONE {
                    let what =
                        if io.location.is_some() || self.layout != Layout::NONE { "layout" } else { "interpolation" };
                    return Err(Error::new(line, what, "an interface qualifier must be followed by 'in' or 'out'"));
                }
                return Ok((StorageQualifier::None, invariant, io));
            }
        };
        self.i += 1;
        if io != IoQual::NONE && !matches!(storage, StorageQualifier::In | StorageQualifier::Out) {
            let what = if io.location.is_some() { "layout" } else { "interpolation" };
            return Err(Error::new(line, what, "an interface qualifier applies only to 'in' and 'out'"));
        }
        /* The block qualifiers belong to `uniform` alone (section 4.3.8.3);
         * `location` to `in` and `out` (4.3.8.1). */
        let block_quals = self.layout.packing.is_some() || self.layout.row_major.is_some();
        if block_quals && storage != StorageQualifier::Uniform {
            return Err(Error::new(line, "layout", "a block layout qualifier applies only to 'uniform'"));
        }
        Ok((storage, invariant, io))
    }

    /// `layout(...)`: `location = N` (an `in` or `out`), or the uniform
    /// block qualifiers `std140`, `shared`, `packed`, `row_major`,
    /// `column_major` (section 4.3.8.3), where the last of a kind wins.
    /// `layout()` is refused: the grammar's list is not empty.
    fn layout_qualifier(&mut self) -> R<'a, Layout> {
        self.i += 1;
        self.expect(Tok::LParen, "expected '(' after 'layout'")?;
        let mut location = None;
        let (mut packing, mut row_major) = (None, None);
        loop {
            let t = self.peek();
            let id = self.ident("expected a layout qualifier")?;
            match id {
                "location" => {
                    self.expect(Tok::Assign, "'location' needs '= <integer>'")?;
                    let n = self.peek();
                    if n.kind != Tok::IntLit {
                        return Err(Error::new(n.line, n.text, "a layout location must be an integer constant"));
                    }
                    self.i += 1;
                    if location.replace(n.int).is_some() {
                        return Err(Error::new(t.line, id, "'location' is given twice"));
                    }
                }
                "std140" => packing = Some(BlockPacking::Std140),
                "shared" => packing = Some(BlockPacking::Shared),
                "packed" => packing = Some(BlockPacking::Packed),
                "row_major" => row_major = Some(true),
                "column_major" => row_major = Some(false),
                _ => return Err(Error::new(t.line, id, "is not a layout qualifier")),
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "expected ')'")?;
        if location.is_some() && (packing.is_some() || row_major.is_some()) {
            return Err(self.err("'location' and a block layout qualifier in one layout"));
        }
        Ok(Layout { location, packing, row_major })
    }

    fn type_with_precision(&mut self) -> R<'a, (BaseType<'a>, Precision)> {
        let precision = match self.peek().kw() {
            Some(Kw::LowP) => {
                self.i += 1;
                Precision::Low
            }
            Some(Kw::MediumP) => {
                self.i += 1;
                Precision::Medium
            }
            Some(Kw::HighP) => {
                self.i += 1;
                Precision::High
            }
            _ => Precision::Default,
        };
        Ok((self.base_type()?, precision))
    }

    fn base_type(&mut self) -> R<'a, BaseType<'a>> {
        let t = self.peek();
        if t.kw() == Some(Kw::Struct) {
            return self.struct_specifier();
        }
        let b = match t.kw() {
            Some(Kw::Void) => BaseType::Void,
            Some(Kw::Float) => BaseType::Float,
            Some(Kw::Int) => BaseType::Int,
            Some(Kw::Bool) => BaseType::Bool,
            Some(Kw::Vec2) => BaseType::Vec(2),
            Some(Kw::Vec3) => BaseType::Vec(3),
            Some(Kw::Vec4) => BaseType::Vec(4),
            Some(Kw::IVec2) => BaseType::IVec(2),
            Some(Kw::IVec3) => BaseType::IVec(3),
            Some(Kw::IVec4) => BaseType::IVec(4),
            Some(Kw::BVec2) => BaseType::BVec(2),
            Some(Kw::BVec3) => BaseType::BVec(3),
            Some(Kw::BVec4) => BaseType::BVec(4),
            Some(Kw::Mat2) => BaseType::Mat(2, 2),
            Some(Kw::Mat3) => BaseType::Mat(3, 3),
            Some(Kw::Mat4) => BaseType::Mat(4, 4),
            /* ES 3.00's non-square matrices; keywords only there. */
            Some(Kw::MatCxR(c, r)) => BaseType::Mat(c, r),
            /* ES 3.00's unsigned types; the lexer only makes these keywords
             * in a `#version 300 es` source. */
            Some(Kw::Uint) => BaseType::Uint,
            Some(Kw::UVec2) => BaseType::UVec(2),
            Some(Kw::UVec3) => BaseType::UVec(3),
            Some(Kw::UVec4) => BaseType::UVec(4),
            Some(Kw::Sampler2D) => BaseType::SAMPLER_2D,
            Some(Kw::SamplerCube) => BaseType::SAMPLER_CUBE,
            /* ES 3.00's samplers (section 4.1.7); keywords only there. */
            Some(
                k @ (Kw::Sampler3D
                | Kw::Sampler2DShadow
                | Kw::SamplerCubeShadow
                | Kw::Sampler2DArray
                | Kw::Sampler2DArrayShadow
                | Kw::ISampler2D
                | Kw::ISampler3D
                | Kw::ISamplerCube
                | Kw::ISampler2DArray
                | Kw::USampler2D
                | Kw::USampler3D
                | Kw::USamplerCube
                | Kw::USampler2DArray),
            ) => {
                use SamplerDim::*;
                use SamplerRet::*;
                let (dim, ret, shadow) = match k {
                    Kw::Sampler3D => (D3, Float, false),
                    Kw::Sampler2DShadow => (D2, Float, true),
                    Kw::SamplerCubeShadow => (Cube, Float, true),
                    Kw::Sampler2DArray => (D2Array, Float, false),
                    Kw::Sampler2DArrayShadow => (D2Array, Float, true),
                    Kw::ISampler2D => (D2, Int, false),
                    Kw::ISampler3D => (D3, Int, false),
                    Kw::ISamplerCube => (Cube, Int, false),
                    Kw::ISampler2DArray => (D2Array, Int, false),
                    Kw::USampler2D => (D2, Uint, false),
                    Kw::USampler3D => (D3, Uint, false),
                    Kw::USamplerCube => (Cube, Uint, false),
                    _ => (D2Array, Uint, false),
                };
                BaseType::Sampler(SamplerType::new(dim, ret, shadow))
            }
            Some(Kw::Reserved) => return Err(Error::new(t.line, t.text, "reserved for future use")),
            Some(_) => return Err(self.err("expected a type")),
            None => {
                if !self.is_struct_name(t.text) {
                    /* A type name from a LATER version of the language reads
                     * as an ordinary identifier to an ES 1.00 lexer, so the
                     * parser's own message would be "expected a type" and
                     * leave the reader to work out why their `sampler2DArray`
                     * is not one.
                     *
                     * It matters because such a source does not have to
                     * announce itself: an application's generated shader can
                     * declare `#version 100` and then use `sampler2DArray`
                     * (five Qt Quick 3D materials did), so the clear "GLSL
                     * ES 3.00 source" refusal never fires for it. Section 4.1 gives
                     * ES 1.00 exactly two sampler types, `sampler2D` and
                     * `samplerCube`; everything below is from ES 3.00. */
                    if ES300_TYPES.contains(&t.text) {
                        return Err(self.err("a GLSL ES 3.00 type, and this source is ES 1.00"));
                    }
                    return Err(self.err("expected a type"));
                }
                BaseType::Struct(t.text)
            }
        };
        self.i += 1;
        Ok(b)
    }

    fn is_struct_name(&self, name: &str) -> bool {
        self.structs.as_slice().contains(&name)
    }

    /// `struct [name] { member... }`, which both names a type and declares it.
    fn struct_specifier(&mut self) -> R<'a, BaseType<'a>> {
        let line = self.bump().line; // `struct`
        let name = if self.at(Tok::Ident) && self.peek().kw().is_none() {
            Some(self.ident("expected a struct name")?)
        } else {
            None
        };
        self.expect(Tok::LBrace, "expected '{' in a struct")?;
        let mstart = self.members.len() as u32;
        while !self.at(Tok::RBrace) {
            if self.at(Tok::Eof) {
                return Err(self.err("expected '}' in a struct"));
            }
            let (base, precision) = self.type_with_precision()?;
            let array_ty = self.type_array_suffix()?;
            loop {
                let mline = self.peek().line;
                let mname = self.ident("expected a member name")?;
                let (array_len, implicit_size) = self.declarator_array(array_ty)?;
                if implicit_size {
                    /* ES 3.00 section 4.1.8: a member array "must be
                     * declared with a size". */
                    return Err(Error::new(mline, mname, "a struct member array must be sized"));
                }
                self.members.push(
                    self.a,
                    Member {
                        ty: TypeSpec { base, precision, array_len, implicit_size },
                        name: mname,
                        line: mline,
                        row_major: None,
                    },
                )?;
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::Semi, "expected ';' after a struct member")?;
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        let members = Range { start: mstart, len: self.members.len() as u32 - mstart };
        if members.is_empty() {
            return Err(Error::new(line, "struct", "a struct must have at least one member"));
        }
        let Some(name) = name else {
            /* An anonymous struct is a type with no name to refer to it by.
             * Nothing in the shader set uses one and the reflection has no
             * name to report for its members, so this is where it stops --
             * as a refusal that says so, not as a wrong answer. */
            return Err(Error::new(line, "struct", "an anonymous struct is not supported"));
        };
        if self.is_struct_name(name) {
            return Err(Error::new(line, name, "struct redefined"));
        }
        self.structs.push(self.a, name)?;
        let id = self.push_decl(Decl::Struct { name, members, line })?;
        self.pending_struct = Some(id);
        Ok(BaseType::Struct(name))
    }

    /* ---- statements ---------------------------------------------------- */

    fn compound_statement(&mut self) -> R<'a, StmtId> {
        self.enter()?;
        let r = self.compound_statement_inner();
        self.leave();
        r
    }

    fn compound_statement_inner(&mut self) -> R<'a, StmtId> {
        self.expect(Tok::LBrace, "expected '{'")?;
        /* Gathered aside: a nested block appends to `stmt_ids` too, so a
         * range taken across this loop would contain the inner block's
         * statements as well as this one's. */
        let mut items: AVec<StmtId> = AVec::new();
        while !self.at(Tok::RBrace) {
            if self.at(Tok::Eof) {
                return Err(self.err("expected '}'"));
            }
            let s = self.statement()?;
            items.push(self.a, s)?;
        }
        self.i += 1; // `}`
        let start = self.stmt_ids.len() as u32;
        let src = self.a.alloc_copy(items.as_slice())?;
        self.stmt_ids.extend_from_slice(self.a, src)?;
        self.push_stmt(Stmt::Block(Range { start, len: src.len() as u32 }))
    }

    /// Does a statement here start a declaration?
    ///
    /// The one question the grammar cannot answer from the token alone:
    /// `S s;` is a declaration and `s = 1;` is not, and the difference is
    /// whether `S` names a struct.
    fn starts_declaration(&self) -> bool {
        let t = self.peek();
        match t.kw() {
            /* A basic type followed by `(` is a CONSTRUCTOR -- section 9's
             * `function_call_header: function_identifier LEFT_PAREN`, whose
             * identifier may be a type specifier -- so `ivec3(a, b, c).x
             * * k;` is an expression statement. Committing to a declaration
             * refused it with "expected a name" (13 dEQP
             * shaders.random.all_features cases). */
            Some(
                Kw::Void
                | Kw::Float
                | Kw::Int
                | Kw::Bool
                | Kw::Vec2
                | Kw::Vec3
                | Kw::Vec4
                | Kw::IVec2
                | Kw::IVec3
                | Kw::IVec4
                | Kw::BVec2
                | Kw::BVec3
                | Kw::BVec4
                | Kw::Mat2
                | Kw::Mat3
                | Kw::Mat4
                | Kw::MatCxR(..)
                | Kw::Uint
                | Kw::UVec2
                | Kw::UVec3
                | Kw::UVec4,
            ) if self.peek_at(1).kind == Tok::LParen || self.array_ctor_ahead() => false,
            Some(
                Kw::Const
                | Kw::Attribute
                | Kw::Uniform
                | Kw::Varying
                | Kw::Layout
                | Kw::Flat
                | Kw::Smooth
                | Kw::Centroid
                | Kw::Invariant
                | Kw::Precision
                | Kw::Struct
                | Kw::Void
                | Kw::Float
                | Kw::Int
                | Kw::Bool
                | Kw::Vec2
                | Kw::Vec3
                | Kw::Vec4
                | Kw::IVec2
                | Kw::IVec3
                | Kw::IVec4
                | Kw::BVec2
                | Kw::BVec3
                | Kw::BVec4
                | Kw::Mat2
                | Kw::Mat3
                | Kw::Mat4
                | Kw::MatCxR(..)
                | Kw::Uint
                | Kw::UVec2
                | Kw::UVec3
                | Kw::UVec4
                | Kw::Sampler2D
                | Kw::SamplerCube
                | Kw::Sampler3D
                | Kw::Sampler2DShadow
                | Kw::SamplerCubeShadow
                | Kw::Sampler2DArray
                | Kw::Sampler2DArrayShadow
                | Kw::ISampler2D
                | Kw::ISampler3D
                | Kw::ISamplerCube
                | Kw::ISampler2DArray
                | Kw::USampler2D
                | Kw::USampler3D
                | Kw::USamplerCube
                | Kw::USampler2DArray
                | Kw::LowP
                | Kw::MediumP
                | Kw::HighP,
            ) => true,
            Some(_) => false,
            None => {
                if t.kind != Tok::Ident || !self.is_struct_name(t.text) {
                    return false;
                }
                /* ES 3.00 `S[2] s;` is a declaration and `S[2](a, b)` an
                 * array constructor: what follows the `]` decides. */
                let after = if self.peek_at(1).kind == Tok::LBracket {
                    match self.matching_bracket(1) {
                        Some(k) => k + 1,
                        None => return false,
                    }
                } else {
                    1
                };
                self.peek_at(after).kind == Tok::Ident && self.peek_at(after).kw().is_none()
            }
        }
    }

    /// The offset of the `]` that closes the `[` at offset `open`, if the
    /// stream has one.
    fn matching_bracket(&self, open: usize) -> Option<usize> {
        let mut depth = 0u32;
        let mut k = open;
        loop {
            match self.peek_at(k).kind {
                Tok::LBracket => depth += 1,
                Tok::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                Tok::Eof | Tok::Semi | Tok::LBrace | Tok::RBrace => return None,
                _ => {}
            }
            k += 1;
        }
    }

    /// Does the cursor stand on `T [ ... ] (`, ES 3.00's array constructor
    /// (section 5.4.4)? A variable indexed is never followed by `(`, so the
    /// shape decides it exactly, whatever `T` names.
    fn array_ctor_ahead(&self) -> bool {
        self.peek().es3
            && self.peek_at(1).kind == Tok::LBracket
            && self.matching_bracket(1).is_some_and(|k| self.peek_at(k + 1).kind == Tok::LParen)
    }

    fn statement(&mut self) -> R<'a, StmtId> {
        self.enter()?;
        let r = self.statement_inner();
        self.leave();
        r
    }

    fn statement_inner(&mut self) -> R<'a, StmtId> {
        if self.at(Tok::LBrace) {
            return self.compound_statement();
        }
        if self.starts_declaration() {
            let r = if self.peek().kw() == Some(Kw::Precision) {
                let id = self.precision_declaration()?;
                let start = self.decl_ids.len() as u32;
                self.decl_ids.push(self.a, id)?;
                Range { start, len: 1 }
            } else {
                self.declaration_or_function()?
            };
            return self.push_stmt(Stmt::Decl(r));
        }
        match self.peek().kw() {
            Some(Kw::If) => return self.if_statement(),
            Some(Kw::While) => return self.while_statement(),
            Some(Kw::Do) => return self.do_statement(),
            Some(Kw::For) => return self.for_statement(),
            Some(Kw::Switch) => return self.switch_statement(),
            /* A label is parsed wherever a statement may stand; the checker
             * is what refuses one outside a switch's body (dEQP's
             * `switch.case_in_if` and its siblings), with a message that
             * says so rather than a syntax error. */
            Some(Kw::Case) => {
                self.i += 1;
                let e = self.expression()?;
                self.expect(Tok::Colon, "expected ':' after a case label")?;
                return self.push_stmt(Stmt::Case(Some(e)));
            }
            Some(Kw::Default) => {
                self.i += 1;
                self.expect(Tok::Colon, "expected ':' after 'default'")?;
                return self.push_stmt(Stmt::Case(None));
            }
            Some(Kw::Break) => {
                self.i += 1;
                self.expect(Tok::Semi, "expected ';'")?;
                return self.push_stmt(Stmt::Break);
            }
            Some(Kw::Continue) => {
                self.i += 1;
                self.expect(Tok::Semi, "expected ';'")?;
                return self.push_stmt(Stmt::Continue);
            }
            Some(Kw::Discard) => {
                self.i += 1;
                self.expect(Tok::Semi, "expected ';'")?;
                return self.push_stmt(Stmt::Discard);
            }
            Some(Kw::Return) => {
                self.i += 1;
                let e = if self.at(Tok::Semi) { None } else { Some(self.expression()?) };
                self.expect(Tok::Semi, "expected ';'")?;
                return self.push_stmt(Stmt::Return(e));
            }
            Some(Kw::Reserved) => {
                let t = self.peek();
                return Err(Error::new(t.line, t.text, "reserved for future use"));
            }
            _ => {}
        }
        if self.eat(Tok::Semi) {
            return self.push_stmt(Stmt::Expr(None));
        }
        let e = self.expression()?;
        self.expect(Tok::Semi, "expected ';'")?;
        self.push_stmt(Stmt::Expr(Some(e)))
    }

    fn if_statement(&mut self) -> R<'a, StmtId> {
        self.i += 1;
        self.expect(Tok::LParen, "expected '(' after 'if'")?;
        let cond = self.expression()?;
        self.expect(Tok::RParen, "expected ')'")?;
        let then_s = self.statement()?;
        let else_s = if self.peek().kw() == Some(Kw::Else) {
            self.i += 1;
            Some(self.statement()?)
        } else {
            None
        };
        self.push_stmt(Stmt::If { cond, then_s, else_s })
    }

    fn while_statement(&mut self) -> R<'a, StmtId> {
        self.i += 1;
        self.expect(Tok::LParen, "expected '(' after 'while'")?;
        let cond = self.condition()?;
        self.expect(Tok::RParen, "expected ')'")?;
        let body = self.statement()?;
        self.push_stmt(Stmt::While { cond, body })
    }

    /// A `while` or `for` controlling clause (section 9's `condition`):
    /// either an expression, or a declaration whose initialiser is the value
    /// tested. `while (bool done = step())` declares `done` in the loop's
    /// scope, and dEQP's scoping group has four cases that need it.
    fn condition(&mut self) -> R<'a, Cond> {
        if !self.starts_declaration() {
            return Ok(Cond::Expr(self.expression()?));
        }
        let line = self.peek().line;
        let (storage, invariant, io) = self.qualifiers()?;
        let (base, precision) = self.type_with_precision()?;
        if self.pending_struct.take().is_some() {
            return Err(Error::new(line, "struct", "a struct cannot be declared in a loop condition"));
        }
        let name = self.ident("expected a name")?;
        self.expect(Tok::Assign, "a declaration used as a condition must have an initializer")?;
        let init = self.assignment_expr()?;
        let id = self.push_decl(Decl::Var {
            storage,
            invariant,
            io,
            ty: TypeSpec { base, precision, array_len: None, implicit_size: false },
            name,
            init: Some(init),
            line,
        })?;
        Ok(Cond::Decl(id))
    }

    /// ES 3.00 section 6.2: `switch ( expression ) { statements }`. The body
    /// must be a compound statement -- the grammar says so -- and its labels
    /// are among its statements.
    fn switch_statement(&mut self) -> R<'a, StmtId> {
        self.i += 1;
        self.expect(Tok::LParen, "expected '(' after 'switch'")?;
        let sel = self.expression()?;
        self.expect(Tok::RParen, "expected ')'")?;
        if !self.at(Tok::LBrace) {
            return Err(self.err("expected '{' after 'switch (...)'"));
        }
        let body = self.compound_statement()?;
        self.push_stmt(Stmt::Switch { sel, body })
    }

    fn do_statement(&mut self) -> R<'a, StmtId> {
        self.i += 1;
        let body = self.statement()?;
        if self.peek().kw() != Some(Kw::While) {
            return Err(self.err("expected 'while' after the body of a 'do'"));
        }
        self.i += 1;
        self.expect(Tok::LParen, "expected '(' after 'while'")?;
        let cond = self.expression()?;
        self.expect(Tok::RParen, "expected ')'")?;
        self.expect(Tok::Semi, "expected ';'")?;
        self.push_stmt(Stmt::DoWhile { body, cond })
    }

    fn for_statement(&mut self) -> R<'a, StmtId> {
        self.i += 1;
        self.expect(Tok::LParen, "expected '(' after 'for'")?;
        let init = if self.eat(Tok::Semi) {
            None
        } else {
            /* The initialiser is a whole statement -- a declaration or an
             * expression -- and it consumes its own ';'. */
            Some(self.statement()?)
        };
        let cond = if self.at(Tok::Semi) { None } else { Some(self.condition()?) };
        self.expect(Tok::Semi, "expected ';' in a 'for'")?;
        let step = if self.at(Tok::RParen) { None } else { Some(self.expression()?) };
        self.expect(Tok::RParen, "expected ')'")?;
        let body = self.statement()?;
        self.push_stmt(Stmt::For { init, cond, step, body })
    }

    /* ---- expressions ---------------------------------------------------
     *
     * Section 5.1's precedence, lowest first: comma, assignment, conditional,
     * then the binary operators by `binary_op` below, then unary, then
     * postfix.
     */

    fn expression(&mut self) -> R<'a, ExprId> {
        let mut e = self.assignment_expr()?;
        while self.eat(Tok::Comma) {
            let rhs = self.assignment_expr()?;
            e = self.push_expr(Expr::Comma(e, rhs))?;
        }
        Ok(e)
    }

    fn assignment_expr(&mut self) -> R<'a, ExprId> {
        self.enter()?;
        let r = self.assignment_expr_inner();
        self.leave();
        r
    }

    fn assignment_expr_inner(&mut self) -> R<'a, ExprId> {
        let lhs = self.conditional_expr()?;
        let op = match self.peek().kind {
            Tok::Assign => None,
            Tok::AddAssign => Some(BinOp::Add),
            Tok::SubAssign => Some(BinOp::Sub),
            Tok::MulAssign => Some(BinOp::Mul),
            Tok::DivAssign => Some(BinOp::Div),
            Tok::ModAssign => Some(BinOp::Mod),
            Tok::AndAssign => Some(BinOp::BitAnd),
            Tok::OrAssign => Some(BinOp::BitOr),
            Tok::XorAssign => Some(BinOp::BitXor),
            Tok::ShlAssign => Some(BinOp::Shl),
            Tok::ShrAssign => Some(BinOp::Shr),
            _ => return Ok(lhs),
        };
        self.i += 1;
        /* Right-associative: `a = b = c` is `a = (b = c)`. */
        let rhs = self.assignment_expr()?;
        self.push_expr(Expr::Assign(op, lhs, rhs))
    }

    fn conditional_expr(&mut self) -> R<'a, ExprId> {
        let cond = self.binary_expr(1)?;
        if !self.eat(Tok::Question) {
            return Ok(cond);
        }
        let t = self.expression()?;
        self.expect(Tok::Colon, "expected ':' in a conditional expression")?;
        let f = self.assignment_expr()?;
        self.push_expr(Expr::Select(cond, t, f))
    }

    fn binary_expr(&mut self, min_bp: u8) -> R<'a, ExprId> {
        self.enter()?;
        let r = self.binary_expr_inner(min_bp);
        self.leave();
        r
    }

    fn binary_expr_inner(&mut self, min_bp: u8) -> R<'a, ExprId> {
        let mut lhs = self.unary_expr()?;
        loop {
            let Some((op, bp)) = binary_op(self.peek().kind) else {
                return Ok(lhs);
            };
            if bp < min_bp {
                return Ok(lhs);
            }
            self.i += 1;
            let rhs = self.binary_expr(bp + 1)?;
            lhs = self.push_expr(Expr::Binary(op, lhs, rhs))?;
        }
    }

    fn unary_expr(&mut self) -> R<'a, ExprId> {
        let op = match self.peek().kind {
            Tok::Plus => UnOp::Plus,
            Tok::Minus => UnOp::Minus,
            Tok::Not => UnOp::Not,
            Tok::Tilde => UnOp::BitNot,
            Tok::Inc => UnOp::PreInc,
            Tok::Dec => UnOp::PreDec,
            _ => return self.postfix_expr(),
        };
        self.i += 1;
        let e = self.unary_expr()?;
        self.push_expr(Expr::Unary(op, e))
    }

    fn postfix_expr(&mut self) -> R<'a, ExprId> {
        let mut e = self.primary_expr()?;
        loop {
            match self.peek().kind {
                Tok::LBracket => {
                    self.i += 1;
                    let idx = self.expression()?;
                    self.expect(Tok::RBracket, "expected ']'")?;
                    e = self.push_expr(Expr::Index(e, idx))?;
                }
                Tok::Dot => {
                    self.i += 1;
                    let t = self.peek();
                    if t.kind != Tok::Ident {
                        return Err(self.err("expected a field or swizzle after '.'"));
                    }
                    self.i += 1;
                    /* ES 3.00's one method, `a.length()` (section 5.9 of
                     * that specification). */
                    if t.es3 && self.at(Tok::LParen) {
                        if t.text != "length" {
                            return Err(Error::new(t.line, t.text, "'length' is the only method"));
                        }
                        self.i += 1;
                        self.expect(Tok::RParen, "'length()' takes no arguments")?;
                        e = self.push_expr(Expr::Length(e))?;
                        continue;
                    }
                    e = self.push_expr(Expr::Field(e, t.text))?;
                }
                Tok::Inc => {
                    self.i += 1;
                    e = self.push_expr(Expr::Unary(UnOp::PostInc, e))?;
                }
                Tok::Dec => {
                    self.i += 1;
                    e = self.push_expr(Expr::Unary(UnOp::PostDec, e))?;
                }
                _ => return Ok(e),
            }
        }
    }

    fn primary_expr(&mut self) -> R<'a, ExprId> {
        let t = self.peek();
        match t.kind {
            Tok::IntLit => {
                /* `1u` is a uint (ES 3.00 section 4.1.3), and only an ES 3.00
                 * lexer lets the suffix through. It is its own node, never an
                 * `IntLit` with a flag, so that `int v = 0u;` -- dEQP's
                 * constants.int_from_uint_2, which must not compile -- is a
                 * type mismatch the checker sees. */
                self.i += 1;
                if t.text.ends_with(['u', 'U']) {
                    return self.push_expr(Expr::UintLit(t.int));
                }
                self.push_expr(Expr::IntLit(t.int))
            }
            Tok::FloatLit => {
                self.i += 1;
                self.push_expr(Expr::FloatLit(t.float))
            }
            Tok::LParen => {
                self.i += 1;
                let e = self.expression()?;
                self.expect(Tok::RParen, "expected ')'")?;
                Ok(e)
            }
            Tok::Ident => match t.kw() {
                Some(Kw::True) => {
                    self.i += 1;
                    self.push_expr(Expr::BoolLit(true))
                }
                Some(Kw::False) => {
                    self.i += 1;
                    self.push_expr(Expr::BoolLit(false))
                }
                Some(Kw::Reserved) => Err(Error::new(t.line, t.text, "reserved for future use")),
                Some(_) if self.array_ctor_ahead() => self.array_ctor(),
                /* A type name here is a constructor: `vec4(1.0)`, `S(1, 2)`.
                 * It reaches the tree as an ordinary call, because whether a
                 * name is a type is not a question for the parser. */
                Some(_) if self.peek_at(1).kind == Tok::LParen => self.call(t.text),
                Some(_) => Err(Error::new(t.line, t.text, "unexpected keyword in an expression")),
                None if self.peek_at(1).kind == Tok::LParen => self.call(t.text),
                None if self.is_struct_name(t.text) && self.array_ctor_ahead() => self.array_ctor(),
                None => {
                    self.i += 1;
                    self.push_expr(Expr::Ident(t.text))
                }
            },
            _ => Err(self.err("expected an expression")),
        }
    }

    /// `T[N](a, b, ...)` or `T[](a, b, ...)`, with the cursor on `T`.
    fn array_ctor(&mut self) -> R<'a, ExprId> {
        let base = self.base_type()?;
        self.expect(Tok::LBracket, "expected '['")?;
        let len = if self.eat(Tok::RBracket) {
            None
        } else {
            let e = self.conditional_expr()?;
            self.expect(Tok::RBracket, "expected ']'")?;
            Some(e)
        };
        self.expect(Tok::LParen, "expected '(' after an array type")?;
        let mut items: AVec<ExprId> = AVec::new();
        if !self.at(Tok::RParen) {
            loop {
                let e = self.assignment_expr()?;
                items.push(self.a, e)?;
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "expected ')' in an array constructor")?;
        let start = self.expr_ids.len() as u32;
        let src = self.a.alloc_copy(items.as_slice())?;
        self.expr_ids.extend_from_slice(self.a, src)?;
        self.push_expr(Expr::ArrayCtor { base, len, args: Range { start, len: src.len() as u32 } })
    }

    /// A call or constructor whose name is at the cursor and whose `(`
    /// follows it.
    fn call(&mut self, name: &'a str) -> R<'a, ExprId> {
        self.i += 2; // the name and the '('
                     /* Gathered aside: an argument that is itself a call appends to
                      * `expr_ids`, so a range taken across this loop would contain the
                      * inner call's arguments too. */
        let mut items: AVec<ExprId> = AVec::new();
        /* `f(void)` is an explicit empty argument list. */
        if self.peek().kw() == Some(Kw::Void) && self.peek_at(1).kind == Tok::RParen {
            self.i += 1;
        } else if !self.at(Tok::RParen) {
            loop {
                let e = self.assignment_expr()?;
                items.push(self.a, e)?;
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "expected ')' in a function call")?;
        let start = self.expr_ids.len() as u32;
        let src = self.a.alloc_copy(items.as_slice())?;
        self.expr_ids.extend_from_slice(self.a, src)?;
        self.push_expr(Expr::Call { name, args: Range { start, len: src.len() as u32 } })
    }
}

/// The binary operator a token is, and its binding power (section 5.1).
///
/// The operators ES 1.00 *reserves* -- `%`, `&`, `|`, `^`, `<<`, `>>` -- are
/// here at their C precedence on purpose: parsing them puts them in the tree,
/// so semantic analysis can say "'%' : reserved" instead of the parser
/// failing with "expected an expression" at the operator.
fn binary_op(t: Tok) -> Option<(BinOp, u8)> {
    Some(match t {
        Tok::OrOr => (BinOp::OrOr, 1),
        Tok::XorXor => (BinOp::XorXor, 2),
        Tok::AndAnd => (BinOp::AndAnd, 3),
        Tok::Pipe => (BinOp::BitOr, 4),
        Tok::Caret => (BinOp::BitXor, 5),
        Tok::Amp => (BinOp::BitAnd, 6),
        Tok::EqEq => (BinOp::Eq, 7),
        Tok::NotEq => (BinOp::Ne, 7),
        Tok::Lt => (BinOp::Lt, 8),
        Tok::Gt => (BinOp::Gt, 8),
        Tok::Le => (BinOp::Le, 8),
        Tok::Ge => (BinOp::Ge, 8),
        Tok::Shl => (BinOp::Shl, 9),
        Tok::Shr => (BinOp::Shr, 9),
        Tok::Plus => (BinOp::Add, 10),
        Tok::Minus => (BinOp::Sub, 10),
        Tok::Star => (BinOp::Mul, 11),
        Tok::Slash => (BinOp::Div, 11),
        Tok::Percent => (BinOp::Mod, 11),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::pp;

    /// Preprocess and parse; the returned tree borrows `buf`.
    fn parse_src<'a>(buf: &'a mut [u8], src: &'a str) -> R<'a, Ast<'a>> {
        let a = Arena::new(buf);
        let pre = pp::preprocess(&a, src)?;
        parse(&a, pre.tokens)
    }

    /// The top-level declarations as a compact line, so a test reads as GLSL
    /// rather than as a tree dump.
    fn ok(src: &str) -> std::string::String {
        let mut buf = [0u8; 1 << 18];
        match parse_src(&mut buf, src) {
            Ok(ast) => {
                let mut s = std::string::String::new();
                for d in ast.top_level() {
                    s.push_str(&summary(&ast, d));
                    s.push(';');
                }
                s
            }
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    fn fails(src: &str) -> &'static str {
        let mut buf = [0u8; 1 << 18];
        match parse_src(&mut buf, src) {
            Ok(_) => panic!("expected a parse failure: {src:?}"),
            Err(e) => e.msg,
        }
    }

    /// A type from a LATER version of the language says so, instead of
    /// leaving the reader with "expected a type".
    ///
    /// The verdict is unchanged -- section 4.1 gives ES 1.00 exactly two
    /// sampler types, `sampler2D` and `samplerCube` -- but such a source
    /// does not have to announce itself: an application's generated shader
    /// can declare `#version 100` and then use `sampler2DArray` (five Qt
    /// Quick 3D materials did), so the clear "GLSL ES 3.00 source" refusal
    /// never fires for it, and the twin-draw survey recorded such sources as
    /// "parse gaps" of the front end rather than as what they are.
    /// A statement that starts with a basic type and `(` is a constructor
    /// call, not a declaration (dEQP shaders.random.all_features, which
    /// write `ivec3(a, b, c).x * k;` as statements). The declaration forms
    /// are unaffected.
    #[test]
    fn a_constructor_can_start_a_statement() {
        ok("void main() { int k = 1; ivec3(1, 2, 3).x * k; vec4(1.0); float(k); }");
        ok("void main() { vec3 v; mat2 m = mat2(1.0); float f; }");
    }

    #[test]
    fn an_es_300_type_name_says_which_version_it_is_from() {
        const LATER: &str = "a GLSL ES 3.00 type, and this source is ES 1.00";
        assert_eq!(fails("uniform sampler2DArray s;\nvoid main() {}"), LATER);
        assert_eq!(fails("uniform sampler2DArrayShadow s;\nvoid main() {}"), LATER);
        /* The two ES 1.00 RESERVES keep their own, better message, which
         * comes from this version's own section 3.6. */
        assert_eq!(fails("uniform sampler3D s;\nvoid main() {}"), "reserved for future use");
        assert_eq!(fails("uniform sampler2DShadow s;\nvoid main() {}"), "reserved for future use");
        assert_eq!(fails("uniform mat2x3 m;\nvoid main() {}"), LATER);
        assert_eq!(fails("uniform uint n;\nvoid main() {}"), LATER);
        assert_eq!(fails("uniform usampler2D s;\nvoid main() {}"), LATER);

        /* An ordinary unknown name is still just not a type -- the list
         * only explains, it never decides. */
        assert_eq!(fails("uniform Wibble w;\nvoid main() {}"), "expected a type");
        /* And the two ES 1.00 sampler types are types. */
        assert!(!ok("uniform sampler2D s;\nvoid main() {}").is_empty());
        assert!(!ok("uniform samplerCube s;\nvoid main() {}").is_empty());
    }

    fn summary(ast: &Ast<'_>, d: &Decl<'_>) -> std::string::String {
        match d {
            Decl::Var { storage, ty, name, init, .. } => std::format!(
                "var {}{} {}{}",
                match storage {
                    StorageQualifier::None => "",
                    StorageQualifier::Const => "const ",
                    StorageQualifier::Attribute => "attribute ",
                    StorageQualifier::Uniform => "uniform ",
                    StorageQualifier::Varying => "varying ",
                    StorageQualifier::In => "in ",
                    StorageQualifier::Out => "out ",
                    StorageQualifier::FragOut => "out(fragment) ",
                },
                type_name(&ty.base),
                name,
                match (ty.array_len, init) {
                    (Some(_), _) => "[]",
                    (None, Some(_)) => "=",
                    _ => "",
                }
            ),
            Decl::Struct { name, members, .. } => std::format!("struct {}({})", name, members.len),
            Decl::Precision { base, .. } => std::format!("precision {}", type_name(base)),
            Decl::Function { name, params, body, .. } => std::format!(
                "fn {}({}){}",
                name,
                params.len,
                match body {
                    Some(b) => match ast.stmt(*b) {
                        Stmt::Block(r) => std::format!("{{{}}}", r.len),
                        _ => "{?}".into(),
                    },
                    None => std::string::String::new(),
                }
            ),
            Decl::Invariant { name, .. } => std::format!("invariant {name}"),
        }
    }

    fn type_name(b: &BaseType<'_>) -> std::string::String {
        match b {
            BaseType::Void => "void".into(),
            BaseType::Float => "float".into(),
            BaseType::Int => "int".into(),
            BaseType::Bool => "bool".into(),
            BaseType::Vec(n) => std::format!("vec{n}"),
            BaseType::IVec(n) => std::format!("ivec{n}"),
            BaseType::Uint => "uint".into(),
            BaseType::UVec(n) => std::format!("uvec{n}"),
            BaseType::BVec(n) => std::format!("bvec{n}"),
            BaseType::Mat(c, r) if c == r => std::format!("mat{c}"),
            BaseType::Mat(c, r) => std::format!("mat{c}x{r}"),
            BaseType::Sampler(s) => s.name().into(),
            BaseType::Struct(s) => std::format!("struct {s}"),
        }
    }

    #[test]
    fn a_minimal_fragment_shader_parses() {
        assert_eq!(ok("void main() { gl_FragColor = vec4(1.0); }"), "fn main(0){1};");
    }

    #[test]
    fn the_declaration_forms_parse() {
        assert_eq!(
            ok("attribute vec4 a_pos;\nvarying vec2 v_uv;\nuniform mat4 u_mvp;\nconst float K = 2.0;"),
            "var attribute vec4 a_pos;var varying vec2 v_uv;var uniform mat4 u_mvp;var const float K=;"
        );
    }

    /// `float a, b[2], c;` is three declarations, and the reflection depends
    /// on them coming out in that order.
    #[test]
    fn one_declaration_list_becomes_one_declaration_per_name() {
        assert_eq!(ok("uniform float a, b[2], c;"), "var uniform float a;var uniform float b[];var uniform float c;");
    }

    #[test]
    fn precision_statements_and_qualifiers_parse() {
        assert_eq!(ok("precision mediump float;\nuniform lowp vec4 c;"), "precision float;var uniform vec4 c;");
    }

    #[test]
    fn structs_parse_and_then_name_a_type() {
        assert_eq!(
            ok("struct Light { vec3 pos; float k; };\nuniform Light u_light;"),
            "struct Light(2);var uniform struct Light u_light;"
        );
        /* The form that declares the struct and a variable at once. */
        assert_eq!(ok("uniform struct S { float a; } s;"), "struct S(1);var uniform struct S s;");
    }

    /// The one place the grammar needs to know a name is a type: `S s;` is a
    /// declaration, `s = 1.0;` is an expression, and only the struct table
    /// tells them apart.
    #[test]
    fn a_struct_name_in_a_body_starts_a_declaration_and_another_name_does_not() {
        assert_eq!(ok("struct S { float a; };\nvoid main() { S s; s.a = 1.0; }"), "struct S(1);fn main(0){2};");
        assert_eq!(ok("void main() { float s; s = 1.0; }"), "fn main(0){2};");
    }

    #[test]
    fn functions_with_parameters_and_prototypes_parse() {
        assert_eq!(
            ok("float f(float x, out vec2 y);\nfloat f(float x, out vec2 y) { return x; }\nvoid g(void) {}"),
            "fn f(2);fn f(2){1};fn g(0){0};"
        );
    }

    #[test]
    fn every_statement_form_parses() {
        let src = "void main() {
            int i = 0;
            if (i > 0) { i = 1; } else i = 2;
            while (i < 3) i++;
            do { i--; } while (i > 0);
            for (int j = 0; j < 4; ++j) { continue; }
            if (i == 0) discard;
            return;
        }";
        assert_eq!(ok(src), "fn main(0){7};");
    }

    /// Section 9's `condition` is `expression | fully_specified_type
    /// IDENTIFIER EQUAL initializer`: a `while` or `for` may declare its
    /// controlling variable. dEQP's scoping group has four cases that do,
    /// and a parser that only accepts an expression rejects shaders the
    /// specification says must build.
    #[test]
    fn a_loop_condition_may_be_a_declaration() {
        let mut buf = [0u8; 1 << 18];
        let ast = parse_src(&mut buf, "void main() { while (bool b = f()) { g(); } }").expect("parse");
        let cond = ast
            .stmts
            .iter()
            .find_map(|s| match s {
                Stmt::While { cond, .. } => Some(*cond),
                _ => None,
            })
            .expect("a while");
        let Cond::Decl(id) = cond else { panic!("the condition declares a variable, it is not an expression") };
        assert!(matches!(ast.decl(id), Decl::Var { name: "b", init: Some(_), .. }));

        /* The same in a `for`, and the ordinary expression form still works. */
        let mut buf2 = [0u8; 1 << 18];
        let ast =
            parse_src(&mut buf2, "void main() { for (int i = 0; bool b = p(i); ++i) {} while (x) {} }").expect("parse");
        assert!(ast.stmts.iter().any(|s| matches!(s, Stmt::For { cond: Some(Cond::Decl(_)), .. })));
        assert!(ast.stmts.iter().any(|s| matches!(s, Stmt::While { cond: Cond::Expr(_), .. })));
    }

    #[test]
    fn a_condition_declaration_must_be_initialised() {
        assert_eq!(
            fails("void main() { while (bool b) {} }"),
            "a declaration used as a condition must have an initializer"
        );
    }

    #[test]
    fn expression_precedence_follows_section_5_1() {
        let mut buf = [0u8; 1 << 18];
        let ast = parse_src(&mut buf, "void main() { float x = 1.0 + 2.0 * 3.0; }").expect("parse");
        /* The tree must be Add(1.0, Mul(2.0, 3.0)), not Mul(Add(..), 3.0). */
        let (l, r) = ast
            .exprs
            .iter()
            .find_map(|e| match e {
                Expr::Binary(BinOp::Add, l, r) => Some((*l, *r)),
                _ => None,
            })
            .expect("an add");
        assert!(matches!(ast.expr(l), Expr::FloatLit(v) if *v == 1.0), "1.0 is the left operand of the add");
        assert!(matches!(ast.expr(r), Expr::Binary(BinOp::Mul, _, _)), "the multiply is its right operand");
    }

    #[test]
    fn postfix_chains_parse_in_order() {
        let mut buf = [0u8; 1 << 18];
        let ast = parse_src(&mut buf, "void main() { float f = m[1].xy.x; }").expect("parse");
        let outer = ast
            .exprs
            .iter()
            .rev()
            .find_map(|e| match e {
                Expr::Field(inner, "x") => Some(*inner),
                _ => None,
            })
            .expect("the .x");
        assert!(matches!(ast.expr(outer), Expr::Field(_, "xy")), ".x applies to (m[1]).xy");
    }

    #[test]
    fn assignment_is_right_associative_and_compound_forms_parse() {
        let mut buf = [0u8; 1 << 18];
        let ast = parse_src(&mut buf, "void main() { a = b = c; d += 1.0; }").expect("parse");
        /* The OUTER assignment is the one whose right operand is itself an
         * assignment; it is pushed last, because its operands are built
         * first. */
        let outer = ast
            .exprs
            .iter()
            .find_map(|e| match e {
                Expr::Assign(None, _, r) if matches!(ast.expr(*r), Expr::Assign(None, _, _)) => Some(*r),
                _ => None,
            })
            .expect("a = (b = c)");
        assert!(matches!(ast.expr(outer), Expr::Assign(None, _, _)), "its right operand is the inner assignment");
        assert!(ast.exprs.iter().any(|e| matches!(e, Expr::Assign(Some(BinOp::Add), _, _))), "d += 1.0");
    }

    #[test]
    fn a_conditional_expression_parses() {
        let mut buf = [0u8; 1 << 18];
        let ast = parse_src(&mut buf, "void main() { float f = a > 0.0 ? 1.0 : 2.0; }").expect("parse");
        assert!(ast.exprs.iter().any(|e| matches!(e, Expr::Select(..))));
    }

    #[test]
    fn calls_and_constructors_share_a_shape() {
        let mut buf = [0u8; 1 << 18];
        let ast = parse_src(&mut buf, "void main() { vec4 c = texture2D(s, vec2(0.0, 1.0)); }").expect("parse");
        let names: std::vec::Vec<&str> = ast
            .exprs
            .iter()
            .filter_map(|e| match e {
                Expr::Call { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["vec2", "texture2D"], "the constructor is an ordinary call in the tree");
        let args = ast
            .exprs
            .iter()
            .find_map(|e| match e {
                Expr::Call { name: "texture2D", args } => Some(*args),
                _ => None,
            })
            .expect("the call");
        assert_eq!(args.len, 2);
    }

    /// A reserved word must produce a message that names it; "unexpected
    /// token" would send the author hunting for a typo.
    #[test]
    fn a_reserved_word_says_so() {
        assert_eq!(fails("void main() { switch (1) {} }"), "reserved for future use");
        assert_eq!(fails("half x;"), "reserved for future use");
        assert_eq!(fails("uniform sampler3D s;"), "reserved for future use");
        assert_eq!(fails("void main() { int goto = 1; }"), "reserved for future use");
    }

    #[test]
    fn syntax_errors_name_what_was_expected() {
        assert_eq!(fails("void main() { float x = 1.0 }"), "expected ';'");
        assert_eq!(fails("void main() { "), "expected '}'");
        assert_eq!(fails("void main() { float x = ; }"), "expected an expression");
        assert_eq!(fails("void main() { if x > 0.0) {} }"), "expected '(' after 'if'");
        assert_eq!(fails("struct S { };"), "a struct must have at least one member");
        assert_eq!(fails("void main() { float x = f(1.0; }"), "expected ')' in a function call");
    }

    /// The reserved operators are parsed, so the tree holds them and semantic
    /// analysis can refuse them by name.
    #[test]
    fn reserved_operators_parse_into_the_tree() {
        let mut buf = [0u8; 1 << 18];
        let ast =
            parse_src(&mut buf, "void main() { int a = b % c; int d = e << f; int g = h & i; }").expect("they parse");
        let ops: std::vec::Vec<BinOp> = ast
            .exprs
            .iter()
            .filter_map(|e| match e {
                Expr::Binary(op, _, _) => Some(*op),
                _ => None,
            })
            .collect();
        assert_eq!(ops, [BinOp::Mod, BinOp::Shl, BinOp::BitAnd]);
    }

    #[test]
    fn a_shader_too_deeply_nested_is_refused_rather_than_overflowing_the_stack() {
        let mut src = std::string::String::from("void main() { float x = ");
        let n = (MAX_DEPTH + 40) as usize;
        for _ in 0..n {
            src.push('(');
        }
        src.push_str("1.0");
        for _ in 0..n {
            src.push(')');
        }
        src.push_str("; }");
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let pre = pp::preprocess(&a, &src).expect("preprocess");
        let Err(e) = parse(&a, pre.tokens) else {
            panic!("a shader nested {n} deep must be refused, not parsed");
        };
        assert_eq!(e.msg, "nested too deeply");
    }

    /// The whole point of the preprocessor feeding the parser: a shader whose
    /// declarations are built by macros parses as the text they expand to.
    /// The FemtoVG family is written this way.
    #[test]
    fn a_macro_built_declaration_parses() {
        assert_eq!(ok("#define N 3\n#define DECL(t, n) uniform t n[N];\nDECL(vec4, frag)"), "var uniform vec4 frag[];");
    }
}
