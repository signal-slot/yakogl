// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Semantic analysis: the GLSL ES 1.00 type system over the syntax tree.
//!
//! The parser accepted the shape of the program; this decides whether it
//! means anything. It is where ES 1.00 differs most from the desktop GLSL the
//! offline pipeline compiled by proxy, and where dEQP's largest negative
//! groups live -- `invalid_implicit_conversions.test` alone is 256 cases that
//! must fail to compile.
//!
//! # No implicit conversions, in either direction
//!
//! Section 4.1.10: GLSL ES 1.00 has **no** implicit conversions. `int i =
//! 1.5;` is an error, and so is `float f = 1;` -- the direction desktop GLSL
//! allows. `vec2 v = vec2(1, 2.0);` is fine, because a constructor converts
//! explicitly. Getting this wrong in the permissive direction is how the
//! shipping pipeline compiles shaders the specification rejects: it hands the
//! rewritten text to glslang as GLSL 450, which has the conversions.
//!
//! # A check that is not certain does not fire
//!
//! The checker reports an error only where it has determined the types
//! involved. Where it cannot -- a call it does not model, a construct whose
//! type it does not yet compute -- it stays silent rather than guessing, and
//! the expression's type becomes [`Type::UNKNOWN`], which every rule treats
//! as "no opinion".
//!
//! That is deliberate. A front end that rejects a valid shader breaks an
//! application that works today, in a way the application cannot route
//! around; one that accepts an invalid shader loses a conformance case. The
//! corpus measures the two directions separately
//! (`tools/online_oracle.py`: false rejects must be 0, must-fail coverage
//! rises as rules land), so the silence shows up as a number rather than
//! hiding as a pass.

use super::arena::{AVec, Arena};
use super::ast::*;
use super::{Error, R};

/// Which stage is being compiled; several rules differ between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Vertex,
    Fragment,
}

/// A resolved type: a base type and an array length (0 = not an array).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Type<'a> {
    pub base: BaseType<'a>,
    pub array_len: u32,
}

impl<'a> Type<'a> {
    /// "I could not determine this." Every rule treats it as no opinion, so
    /// an unmodelled construct silences the checks that depend on it rather
    /// than producing a wrong diagnosis.
    pub const UNKNOWN: Type<'a> = Type { base: BaseType::Void, array_len: u32::MAX };

    pub fn scalar(base: BaseType<'a>) -> Type<'a> {
        Type { base, array_len: 0 }
    }

    pub fn is_unknown(&self) -> bool {
        self.array_len == u32::MAX
    }

    pub fn is_array(&self) -> bool {
        self.array_len > 0 && !self.is_unknown()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScalarKind {
    Float,
    Int,
    /// ES 3.00 `uint`: an integer kind of its own, never converted to or
    /// from `int` implicitly (ES 3.00 section 4.1.10).
    Uint,
    Bool,
}

/// The scalar a type is made of: `vec3` and `float` are both float.
fn scalar_of(b: &BaseType<'_>) -> Option<ScalarKind> {
    Some(match b {
        BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..) => ScalarKind::Float,
        BaseType::Int | BaseType::IVec(_) => ScalarKind::Int,
        BaseType::Uint | BaseType::UVec(_) => ScalarKind::Uint,
        BaseType::Bool | BaseType::BVec(_) => ScalarKind::Bool,
        _ => return None,
    })
}

/// The vector of `n` components over `kind`.
fn vector_of(kind: ScalarKind, n: u8) -> BaseType<'static> {
    match (kind, n) {
        (ScalarKind::Float, 1) => BaseType::Float,
        (ScalarKind::Int, 1) => BaseType::Int,
        (ScalarKind::Uint, 1) => BaseType::Uint,
        (ScalarKind::Bool, 1) => BaseType::Bool,
        (ScalarKind::Float, n) => BaseType::Vec(n),
        (ScalarKind::Int, n) => BaseType::IVec(n),
        (ScalarKind::Uint, n) => BaseType::UVec(n),
        (ScalarKind::Bool, n) => BaseType::BVec(n),
    }
}

/// Section 7.5's built-in struct type, the type of `gl_DepthRange`.
pub const DEPTH_RANGE_PARAMETERS: &str = "gl_DepthRangeParameters";

/// The operators ES 1.00 section 5.1's table marks reserved: `%` and the
/// bit-wise ones (`~`, unary, is refused on its own). ES 3.00 section 5.1
/// makes every one of them an operator on integers.
fn is_integer_op(op: BinOp) -> bool {
    matches!(op, BinOp::Mod | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr)
}

/// An integer kind: `int` or `uint`.
fn is_integer(k: ScalarKind) -> bool {
    matches!(k, ScalarKind::Int | ScalarKind::Uint)
}

/// A vector or scalar: not a matrix, sampler or struct.
fn is_numeric_vector(b: &BaseType<'_>) -> bool {
    matches!(
        b,
        BaseType::Float
            | BaseType::Int
            | BaseType::Uint
            | BaseType::Bool
            | BaseType::Vec(_)
            | BaseType::IVec(_)
            | BaseType::UVec(_)
            | BaseType::BVec(_)
    )
}

/// What a name in the symbol table DENOTES.
///
/// Section 4.2.6: "Within each scope, there is a single name space.
/// Declaring a variable adds a variable name to the name space. Declaring a
/// function adds a function name to the name space. Declaring a structure
/// adds a structure name and a constructor name to the name space." One
/// space, three kinds of occupant -- and section 4.2.7 then says a name may
/// be declared "either as a variable declaration or as function declarations
/// or as a structure", never as two of them.
///
/// The kind is recorded rather than inferred from the type, because a
/// function and a structure name both sit in the table with no type of their
/// own and would otherwise be indistinguishable from each other.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameKind {
    Variable,
    Function,
    /// A structure name, which is also its constructor's name.
    Type,
}

#[derive(Clone, Copy)]
struct Symbol<'a> {
    name: &'a str,
    kind: NameKind,
    ty: Type<'a>,
    storage: StorageQualifier,
    /// A function parameter rather than a variable.
    ///
    /// `const` means two different things on the two, and section 5.10
    /// depends on the difference: a `const` VARIABLE initialised with a
    /// constant expression IS one, and a `const` PARAMETER is only
    /// read-only -- its value arrives at run time. dEQP calls the case
    /// `const_float_function_gotcha` and it is the reason this flag exists
    /// rather than a test on `storage` alone.
    is_param: bool,
}

/// An expression's type and what may be done with it.
#[derive(Clone, Copy)]
struct Val<'a> {
    ty: Type<'a>,
    /// May appear on the left of an assignment.
    lvalue: bool,
    /// Declared `const`, a uniform, an attribute, or a literal.
    constant: bool,
}

impl<'a> Val<'a> {
    fn rvalue(ty: Type<'a>) -> Val<'a> {
        Val { ty, lvalue: false, constant: false }
    }

    fn unknown() -> Val<'a> {
        Val::rvalue(Type::UNKNOWN)
    }
}

/// The type the checker gave each expression in the unit, indexed by
/// [`ExprId`].
///
/// The checker works this out for every expression anyway, in order to apply
/// section 4.1.10's "no implicit conversions in either direction"; throwing it
/// away only meant the lowering had to guess it back from context, which it
/// cannot do for a matrix. `mat4 * vec4` and `float * vec4` are the same
/// syntax and different code.
///
/// An expression the checker had no opinion about is [`Type::UNKNOWN`] rather
/// than absent, so a caller reads one type for every expression and an
/// unmodelled construct silences the decisions that depend on it instead of
/// producing a wrong one.
#[derive(Clone, Copy)]
pub struct ExprTypes<'a>(&'a [Type<'a>]);

impl<'a> ExprTypes<'a> {
    pub fn of(&self, id: ExprId) -> Type<'a> {
        /* Out of range cannot happen -- the slice is as long as the AST's
         * expression table -- but an index that panics has no place in a
         * compiler that runs on the target. */
        self.0.get(id.0 as usize).copied().unwrap_or(Type::UNKNOWN)
    }
}

/// Check a parsed shader.
pub fn check<'a>(a: &Arena<'a>, ast: &Ast<'a>, stage: Stage) -> R<'a, ExprTypes<'a>> {
    let types = a.alloc_filled(ast.exprs.len(), Type::UNKNOWN)?;
    let mut s = Sema {
        a,
        ast,
        stage,
        scopes: AVec::new(),
        marks: AVec::new(),
        has_main: false,
        loop_depth: 0,
        switch_depth: 0,
        default_float_precision: false,
        sampler_defaults: AVec::new(),
        returns: None,
        types,
    };
    s.unit()?;
    Ok(ExprTypes(s.types))
}

struct Sema<'x, 'a> {
    a: &'x Arena<'a>,
    ast: &'x Ast<'a>,
    stage: Stage,
    /// The symbol table: a flat stack, with [`Sema::marks`] recording where
    /// each scope began. That is how a scope is popped without an allocation
    /// per scope.
    scopes: AVec<'a, Symbol<'a>>,
    marks: AVec<'a, u32>,
    has_main: bool,
    loop_depth: u32,
    /// How many `switch` bodies enclose the statement being checked: a
    /// `break` is legal inside one as well as inside a loop (ES 3.00 6.2).
    switch_depth: u32,
    /// Whether `precision <p> float;` has been seen yet.
    ///
    /// Section 4.5.3: the VERTEX language gives float a default precision,
    /// and the FRAGMENT language does not. So in a fragment shader every
    /// declaration of a float, float vector or float matrix must either
    /// carry a precision qualifier of its own or follow a default float
    /// precision declaration. `int` and the samplers have fragment defaults
    /// and are not affected.
    ///
    /// It is a running flag rather than a whole-unit scan because the rule
    /// is positional -- the default must have been declared BEFORE the
    /// declaration that relies on it.
    default_float_precision: bool,
    /// The sampler types a `precision` statement has given a default so
    /// far. ES 3.00 section 4.5.4 predeclares one for `sampler2D` and
    /// `samplerCube` only; every other sampler must be declared with a
    /// precision, or after a default for its type.
    sampler_defaults: AVec<'a, SamplerType>,
    /// The return type of the function whose body is being walked, and
    /// `None` outside any body.
    ///
    /// Section 6.1: "If the return type is not void, any return statements
    /// within the function definition must include a return-expression and
    /// the type of the expression must match the return type. If the return
    /// type of the function is specified as void, any such return statements
    /// must not include a return-expression." A `return` cannot be judged
    /// without knowing which function it is in, and the statement walker had
    /// no way to know.
    returns: Option<Type<'a>>,
    /// Filled by [`Sema::expr`], one entry per expression in the AST. See
    /// [`ExprTypes`] for why the checker keeps this rather than discarding it.
    types: &'a mut [Type<'a>],
}

impl<'a> Sema<'_, 'a> {
    /* ---- scopes -------------------------------------------------------- */

    fn push_scope(&mut self) -> R<'a, ()> {
        let n = self.scopes.len() as u32;
        self.marks.push(self.a, n)?;
        Ok(())
    }

    fn pop_scope(&mut self) {
        if let Some(n) = self.marks.pop() {
            while self.scopes.len() > n as usize {
                self.scopes.pop();
            }
        }
    }

    /// Section 6.1, the rules that make a set of function declarations
    /// agree with each other.
    ///
    /// Two declarations of one name are OVERLOADS only when their parameter
    /// types differ. Same name and same parameter types means the same
    /// SIGNATURE, and then the specification allows exactly one prototype
    /// and one definition, which must agree in every other respect:
    ///
    /// > A function prototype ... must be followed by a function definition
    /// > with the same name, return type and parameter list.
    /// > Functions ... cannot be overloaded on the return type or on the
    /// > parameter qualifiers alone.
    ///
    /// That single grouping is twelve of dEQP's rules and twenty-four of its
    /// must-fail cases -- `duplicate_function`, the four
    /// `prototype_mismatch_*`, and all seven `*_overload` -- because they
    /// are all the same question asked of the same group. None of them was
    /// refused before 2026-09-22: the checker looked a name up, and never
    /// asked whether the declarations of it were consistent.
    ///
    /// A signature is the parameter BASE types and their array-ness, which
    /// is what [`resolve_overload`] matches a call against. Everything else
    /// -- the return type, precisions, `in`/`out`/`inout`, `const` -- must
    /// agree rather than distinguish.
    fn check_function_declarations(&self) -> R<'a, ()> {
        let decls: &[Decl<'a>] = self.ast.decls;
        for (i, d) in decls.iter().enumerate() {
            let Decl::Function { name, ret, params, body, line } = d else { continue };
            if !self.ast.top.iter().any(|t| t.0 as usize == i) {
                continue;
            }
            let ps = &self.ast.params[params.indices()];
            /* Section 4.2.6: "Functions (including built-in functions) may
             * not be redefined. User defined functions may overload built-in
             * functions." Overloading is legal and redefining is not, so the
             * test is on the SIGNATURE, not on the name: section 10.10's own
             * example, `int sin(int x)`, is a legal overload and must keep
             * building. A prototype is refused for the same reason as a
             * definition -- it declares the same signature, and section
             * 4.2.7 counts a declaration by its whole signature.
             *
             * dEQP `scoping.invalid.redeclare_builtin` (a prototype) and
             * `redefine_builtin` (a body). */
            /* Section 6.1: "Arrays are allowed as arguments, but not as the
             * return type", and "The return type can also be a structure if
             * the structure does not contain an array."
             *
             * The second is the one dEQP asks -- a structure is a legal
             * return type right up until a member of it is an array, and
             * nothing about the declaration says so from the outside.
             *
             * dEQP `functions.invalid.return_array_in_struct`. */
            /* ES 3.00 section 6.1 lifts both: "Arrays are allowed as
             * arguments and as the return type. In both cases, the array
             * must be explicitly sized" (the parser refuses `float[] f()`). */
            if ret.is_array() && !self.ast.es3 {
                return Err(Error::new(*line, name, "a function cannot return an array"));
            }
            if let (BaseType::Struct(sname), false) = (ret.base, self.ast.es3) {
                if self.struct_contains_array(sname, 0) {
                    return Err(Error::new(*line, name, "a returned structure cannot contain an array"));
                }
            }
            if builtin_has_signature(name, ps) {
                return Err(Error::new(*line, name, "this signature belongs to a built-in, which cannot be redefined"));
            }
            /* Only the FIRST declaration of each signature reports, so one
             * mismatch is one error rather than one per later declaration. */
            for (j, e) in decls.iter().enumerate().take(i) {
                let Decl::Function { name: n2, ret: r2, params: p2, body: b2, .. } = e else { continue };
                if n2 != name || !self.ast.top.iter().any(|t| t.0 as usize == j) {
                    continue;
                }
                let qs = &self.ast.params[p2.indices()];
                if !same_signature(self.ast, ps, qs) {
                    /* Different parameter types: a legitimate overload. */
                    continue;
                }
                /* The DISAGREEMENTS are reported before the duplicate,
                 * because two definitions that also differ in return type are
                 * both -- and "cannot be overloaded on the return type" says
                 * which part of the shader to change, while "already defined"
                 * sends the reader to delete one of them. dEQP's
                 * `return_type_overload` and `const_overload` are two
                 * DEFINITIONS, so this order is what decides their message. */
                if ret.base != r2.base {
                    return Err(Error::new(*line, name, "cannot be overloaded on the return type"));
                }
                if precisions_disagree(ret.precision, r2.precision) {
                    return Err(Error::new(*line, name, "cannot be overloaded on the return precision"));
                }
                if let Some(bad) = ps.iter().zip(qs).find(|(p, q)| p.direction != q.direction) {
                    let who = bad.0.name.unwrap_or(*name);
                    return Err(Error::new(*line, who, "cannot be overloaded on a parameter's in/out qualifier"));
                }
                if let Some(bad) = ps.iter().zip(qs).find(|(p, q)| p.is_const != q.is_const) {
                    let who = bad.0.name.unwrap_or(*name);
                    return Err(Error::new(*line, who, "cannot be overloaded on a parameter's const qualifier"));
                }
                if let Some(bad) = ps.iter().zip(qs).find(|(p, q)| precisions_disagree(p.ty.precision, q.ty.precision))
                {
                    let who = bad.0.name.unwrap_or(*name);
                    return Err(Error::new(*line, who, "cannot be overloaded on a parameter's precision"));
                }
                if body.is_some() && b2.is_some() {
                    return Err(Error::new(*line, name, "this function is already defined"));
                }
            }
        }
        Ok(())
    }

    /// Section 7.2: "a shader may assign values to either gl_FragColor or
    /// gl_FragData, but not both."
    ///
    /// The spec then defines what counts, and the definition is the whole
    /// rule: "A shader contains a static assignment to a variable x if,
    /// after pre-processing, the shader contains a statement that would
    /// write to x, whether or not run-time flow of control will cause that
    /// statement to be executed."
    ///
    /// So this is a question about the TEXT, not about what runs. dEQP asks
    /// it three ways -- both writes in `main`, one of them under `if
    /// (false)`, and one of them in a function nobody calls -- and all three
    /// are the same question, because none of "reachable", "called" or
    /// "taken" appears in the rule.
    ///
    /// Which is why this walks the expression POOL rather than the
    /// statements. Everywhere else in this compiler a pool index means
    /// allocation order and reading it as execution order is a bug; here
    /// there is no order to get wrong, because the rule asks only whether
    /// the shader CONTAINS such a statement.
    fn check_one_fragment_output(&self) -> R<'a, ()> {
        let (mut color, mut data) = (false, false);
        for e in self.ast.exprs.iter() {
            let target = match *e {
                Expr::Assign(_, target, _) => target,
                Expr::Unary(UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec, target) => target,
                _ => continue,
            };
            match self.lvalue_root(target) {
                Some("gl_FragColor") => color = true,
                Some("gl_FragData") => data = true,
                _ => {}
            }
        }
        if color && data {
            return Err(Error::new(
                0,
                "gl_FragData",
                "a fragment shader writes either 'gl_FragColor' or 'gl_FragData', never both",
            ));
        }
        Ok(())
    }

    /// The variable an assignment target ultimately names: `gl_FragData` for
    /// `gl_FragData[1].rgb`, and `None` when the target is not rooted in a
    /// plain name.
    ///
    /// Only the target's own root, so `arr[gl_FragColor.x] = 1.0` answers
    /// `arr`: the subscript is read, not written.
    fn lvalue_root(&self, mut e: ExprId) -> Option<&'a str> {
        for _ in 0..64 {
            match *self.ast.expr(e) {
                Expr::Ident(n) => return Some(n),
                Expr::Field(inner, _) | Expr::Index(inner, _) => e = inner,
                _ => return None,
            }
        }
        None
    }

    fn declare(&mut self, sym: Symbol<'a>, line: u32) -> R<'a, ()> {
        /* Redeclaring in the SAME scope is an error; shadowing an outer one
         * is not (section 4.2.3). */
        /* Section 3.7: "Identifiers starting with 'gl_' are reserved for use
         * by OpenGL ES. No user-defined identifiers may begin with 'gl_'."
         *
         * Asked here rather than in the lexer because it is about
         * DECLARING a name, not about writing one: a shader must still be
         * able to read and assign `gl_Position`. The built-in variables
         * reach the table through `declare_builtin_variables`, which pushes
         * them directly, so everything that arrives here is user-defined by
         * construction. */
        if sym.name.starts_with("gl_") {
            return Err(Error::new(line, sym.name, "'gl_' is reserved: a user-defined name cannot begin with it"));
        }
        let from = self.marks.as_slice().last().copied().unwrap_or(0) as usize;
        if let Some(prev) = self.scopes.as_slice()[from..].iter().find(|s| s.name == sym.name) {
            /* Section 4.2.7 gives the two collisions different words: the
             * same KIND twice is a repeated declaration, and two different
             * kinds is a name used for two things at once, which says to
             * rename rather than to delete. */
            let msg = match (prev.kind, sym.kind) {
                (a, b) if a == b => "redeclared in the same scope",
                (NameKind::Function, NameKind::Type) | (NameKind::Type, NameKind::Function) => {
                    "already declared as a function, and a name cannot also be a structure"
                }
                (NameKind::Variable, _) | (_, NameKind::Variable) => {
                    "already declared in this scope as something other than a variable"
                }
                _ => "redeclared in the same scope",
            };
            return Err(Error::new(line, sym.name, msg));
        }
        self.scopes.push(self.a, sym)?;
        Ok(())
    }

    /// What `name` already denotes in the CURRENT scope, ignoring outer
    /// ones -- which is the scope section 4.2.7's "at most once" is about.
    fn in_current_scope(&self, name: &str) -> Option<NameKind> {
        let from = self.marks.as_slice().last().copied().unwrap_or(0) as usize;
        self.scopes.as_slice()[from..].iter().find(|s| s.name == name).map(|s| s.kind)
    }

    fn lookup(&self, name: &str) -> Option<Symbol<'a>> {
        self.scopes.as_slice().iter().rev().find(|s| s.name == name).copied()
    }

    /* ---- the unit ------------------------------------------------------ */

    fn unit(&mut self) -> R<'a, ()> {
        self.push_scope()?;
        self.declare_builtin_variables()?;
        /* No pre-pass over the functions: section 4.2.2 gives a name
         * declared outside all function definitions a scope that "starts
         * from where it is declared", so each one enters the table when
         * `declaration` reaches it, and a call written earlier finds
         * nothing. See `Sema::call`. */
        self.check_function_declarations()?;
        if self.stage == Stage::Fragment {
            self.check_one_fragment_output()?;
        }
        for d in self.ast.top_level() {
            self.declaration(d, true)?;
        }
        if !self.has_main {
            return Err(Error::new(0, "", "a shader must define 'void main()'"));
        }
        self.pop_scope();
        Ok(())
    }

    /// The built-in variables of this stage (sections 7.1 and 7.2).
    fn declare_builtin_variables(&mut self) -> R<'a, ()> {
        let vec4 = Type::scalar(BaseType::Vec(4));
        let mut add = |name: &'a str, ty: Type<'a>, storage: StorageQualifier| -> R<'a, ()> {
            self.scopes.push(self.a, Symbol { name, kind: NameKind::Variable, ty, storage, is_param: false })?;
            Ok(())
        };
        match self.stage {
            Stage::Vertex => {
                add("gl_Position", vec4, StorageQualifier::None)?;
                add("gl_PointSize", Type::scalar(BaseType::Float), StorageQualifier::None)?;
            }
            Stage::Fragment => {
                /* gl_FragCoord, gl_FrontFacing and gl_PointCoord are
                 * read-only inputs; `Attribute` is the storage that says
                 * "this stage may not write it". */
                add("gl_FragCoord", vec4, StorageQualifier::Attribute)?;
                add("gl_FrontFacing", Type::scalar(BaseType::Bool), StorageQualifier::Attribute)?;
                add("gl_PointCoord", Type::scalar(BaseType::Vec(2)), StorageQualifier::Attribute)?;
                /* ES 3.00 removed both (section 7.2 of that specification):
                 * a fragment shader declares its own `out` variables. */
                /* ES 3.00 section 7.2: `highp float gl_FragDepth`, written
                 * (and read back) by the fragment shader. ES 1.00 has it only
                 * through EXT_frag_depth, which is not offered. */
                if self.ast.es3 {
                    add("gl_FragDepth", Type::scalar(BaseType::Float), StorageQualifier::None)?;
                }
                if !self.ast.es3 {
                    add("gl_FragColor", vec4, StorageQualifier::None)?;
                    /* gl_FragData[gl_MaxDrawBuffers]; one buffer here. */
                    add("gl_FragData", Type { base: BaseType::Vec(4), array_len: 1 }, StorageQualifier::None)?;
                }
            }
        }
        /* ES 3.00 section 7.1: the vertex language's integer inputs. Read
         * only, which `Attribute` says. */
        if self.ast.es3 && self.stage == Stage::Vertex {
            add("gl_VertexID", Type::scalar(BaseType::Int), StorageQualifier::Attribute)?;
            add("gl_InstanceID", Type::scalar(BaseType::Int), StorageQualifier::Attribute)?;
        }
        /* Section 7.5's built-in uniform state, in both stages:
         * `uniform gl_DepthRangeParameters gl_DepthRange;`, a struct of
         * three highp floats (`near`, `far`, `diff`) that `field` types. */
        add("gl_DepthRange", Type::scalar(BaseType::Struct(DEPTH_RANGE_PARAMETERS)), StorageQualifier::Uniform)?;
        /* Section 7.4's constants (7.3 in ES 3.00, a different list), in
         * both stages: `const mediump int`, so they are constant expressions
         * (an array may be sized by one) and read-only. Their values are
         * `reflect::builtin_constant`'s. */
        let names = if self.ast.es3 {
            super::reflect::BUILTIN_CONSTANTS_ES300
        } else {
            super::reflect::BUILTIN_CONSTANTS_ES100
        };
        for &name in names {
            add(name, Type::scalar(BaseType::Int), StorageQualifier::Const)?;
        }
        Ok(())
    }

    /* ---- declarations -------------------------------------------------- */

    fn declaration(&mut self, d: &Decl<'a>, global: bool) -> R<'a, ()> {
        match d {
            Decl::Var { storage, invariant, io, ty, name, init, line } => {
                let (storage, line) = (*storage, *line);
                self.check_float_precision(ty, name, line)?;
                /* ES 3.00 section 4.5.4: only `sampler2D` and `samplerCube`
                 * have a predeclared default precision; a declaration of any
                 * other sampler needs one of its own or a default before it. */
                if let BaseType::Sampler(st) = ty.base {
                    if self.ast.es3
                        && ty.precision == Precision::Default
                        && ty.base != BaseType::SAMPLER_2D
                        && ty.base != BaseType::SAMPLER_CUBE
                        && !self.sampler_defaults.as_slice().contains(&st)
                    {
                        return Err(Error::new(
                            line,
                            name,
                            "this sampler type has no default precision (section 4.5.4)",
                        ));
                    }
                }
                self.check_storage(storage, ty, name, line, global)?;
                self.check_interface(storage, io, ty, name, line)?;
                /* Section 4.6.1 lists what may be declared invariant:
                 * built-in special variables and VARYING variables, on
                 * either side of either stage. A declaration combining
                 * `invariant` with a storage qualifier can therefore only be
                 * combining it with `varying` -- the spec's own example of
                 * the form is `invariant varying mediump vec3 Color;` -- and
                 * an `attribute` or a `uniform` is not on the list.
                 *
                 * The built-ins are reached by the other form, `invariant
                 * gl_Position;`, which is `Decl::Invariant` and is not this. */
                if *invariant && storage != StorageQualifier::Varying {
                    return Err(Error::new(line, name, "only a varying or a built-in variable may be invariant"));
                }
                /* ES 3.00 `float a[] = ...`: the size is the initialiser's
                 * (section 4.1.9, "the array is implicitly sized by the
                 * initializer"), so the initialiser is typed first. */
                let init_v = match (ty.implicit_size, init) {
                    (true, Some(e)) => Some(self.expr(*e)?),
                    (true, None) => return Err(Error::new(line, name, "an array without a size needs an initializer")),
                    _ => None,
                };
                let len = match (ty.array_len, init_v) {
                    (None, None) => 0,
                    (None, Some(v)) if v.ty.is_array() => v.ty.array_len,
                    (None, Some(v)) if v.ty.is_unknown() => {
                        return Err(Error::new(line, name, "the size of this array's initializer is not known"))
                    }
                    (None, Some(_)) => {
                        return Err(Error::new(line, name, "an array without a size needs an array to initialize it"))
                    }
                    (Some(e), _) => match self.array_size(e) {
                        Some(n) if n > 0 => n as u32,
                        Some(_) => return Err(Error::new(line, name, "an array size must be greater than zero")),
                        None => return Err(Error::new(line, name, "an array size must be a constant expression")),
                    },
                };
                let vty = Type { base: ty.base, array_len: len };
                if let Some(e) = init {
                    match storage {
                        StorageQualifier::Attribute | StorageQualifier::Varying | StorageQualifier::FragOut => {
                            return Err(Error::new(line, name, "this qualifier does not allow an initializer"))
                        }
                        StorageQualifier::Uniform => {
                            return Err(Error::new(line, name, "a uniform cannot have an initializer"))
                        }
                        _ => {}
                    }
                    let v = match init_v {
                        Some(v) => v,
                        None => self.expr(*e)?,
                    };
                    self.check_assignable(&vty, &v.ty, line, name)?;
                    if storage == StorageQualifier::Const && !self.const_expr(*e) {
                        /* Section 4.3.2: "Initializers for const declarations
                         * must be a constant expression". Section 5.10 says
                         * what one is, and what it is NOT: a uniform, a
                         * non-const variable, a call to a user-defined
                         * function, and -- the case dEQP names
                         * `const_float_function_gotcha` -- a `const`
                         * PARAMETER, whose value arrives at run time.
                         *
                         * Ten dEQP must-fail cases compiled here until
                         * 2026-09-22, all in `constants.test`. */
                        return Err(Error::new(line, name, "a const needs a constant expression to initialize it"));
                    }
                }
                self.declare(Symbol { name, kind: NameKind::Variable, ty: vty, storage, is_param: false }, line)
            }
            Decl::Function { name, ret, params, body, line } => {
                /* Section 4.1.7: samplers "cannot be used as out or inout
                 * function parameters. These restrictions also apply to any
                 * structures that contain sampler types." */
                for p in self.ast.params[params.indices()].iter() {
                    if p.direction != ParamDirection::In && self.carries_sampler(p.ty.base, 0) {
                        return Err(Error::new(
                            *line,
                            p.name.unwrap_or(name),
                            "a sampler, or a struct holding one, cannot be an out or inout parameter",
                        ));
                    }
                }
                if *name == "main" {
                    if !params.is_empty() {
                        return Err(Error::new(*line, name, "'main' takes no parameters"));
                    }
                    if ret.base != BaseType::Void {
                        return Err(Error::new(*line, name, "'main' must return void"));
                    }
                    if body.is_some() {
                        self.has_main = true;
                    }
                }
                /* The name enters the current (global) scope HERE, at the
                 * point it is written -- section 4.2.2 -- and before the
                 * body is walked, so that the body may call it. A prototype
                 * declares it and the matching definition then finds it
                 * already there, which section 4.2.7 allows ("a single
                 * function prototype plus the corresponding function
                 * definition"). */
                if self.in_current_scope(name) != Some(NameKind::Function) {
                    /* Not already a function here: `declare` either adds it
                     * or reports the clash with whatever else holds the
                     * name. A name that IS already a function here is the
                     * one repeat section 4.2.7 allows -- "a single function
                     * prototype plus the corresponding function definition"
                     * -- and `check_function_declarations` polices the rest
                     * of that pair. */
                    self.declare(
                        Symbol {
                            name,
                            kind: NameKind::Function,
                            ty: Type::UNKNOWN,
                            storage: StorageQualifier::None,
                            is_param: false,
                        },
                        *line,
                    )?;
                }
                let Some(b) = body else { return Ok(()) };
                self.push_scope()?;
                let r = (|| {
                    for p in self.ast.params[params.indices()].iter() {
                        let Some(pname) = p.name else { continue };
                        let len = match p.ty.array_len {
                            None => 0,
                            Some(e) => self.array_size(e).unwrap_or(0).max(0) as u32,
                        };
                        self.declare(
                            Symbol {
                                name: pname,
                                kind: NameKind::Variable,
                                ty: Type { base: p.ty.base, array_len: len },
                                storage: if p.is_const { StorageQualifier::Const } else { StorageQualifier::None },
                                is_param: true,
                            },
                            *line,
                        )?;
                    }
                    /* Set for the body only, and restored after, so that a
                     * `return` is judged against the function it is in. */
                    let rlen = match ret.array_len {
                        Some(e) => self.array_size(e).unwrap_or(0).max(0) as u32,
                        None => 0,
                    };
                    let outer = self.returns.replace(Type { base: ret.base, array_len: rlen });
                    let r = self.stmt(*b);
                    self.returns = outer;
                    r
                })();
                self.pop_scope();
                r
            }
            /* A structure declaration puts its name in the ONE name space of
             * its scope (section 4.2.6), so that a name cannot be both a
             * structure and a function or a variable (section 4.2.7). The
             * members themselves are checked where the structure is used.
             *
             * The symbol carries no type: a structure NAME is not a value,
             * and `call` reaches a constructor through the AST rather than
             * through this table, so being here changes nothing but the
             * collision check. */
            Decl::Struct { name, line, .. } => self.declare(
                Symbol {
                    name,
                    kind: NameKind::Type,
                    ty: Type::UNKNOWN,
                    storage: StorageQualifier::None,
                    is_param: false,
                },
                *line,
            ),
            /* A precision statement and a lone `invariant` carry no
             * expression. */
            Decl::Precision { base, .. } => {
                if let BaseType::Sampler(st) = base {
                    self.sampler_defaults.push(self.a, *st)?;
                }
                if *base == BaseType::Float {
                    self.default_float_precision = true;
                }
                Ok(())
            }
            Decl::Invariant { .. } => Ok(()),
        }
    }

    /// Where a storage qualifier may appear (sections 4.3 and 7).
    /// Section 4.5.3, the rule that has no default in the fragment stage.
    ///
    /// > The fragment language has no default precision qualifier for
    /// > floating point types. Hence for float, floating point vector and
    /// > matrix variable declarations, either the declaration must include a
    /// > precision qualifier or the default float precision must have been
    /// > previously declared.
    ///
    /// Only DECLARATIONS, and only the float family: `int`, `bool` and the
    /// samplers all have fragment defaults (mediump int, lowp sampler2D),
    /// and a use of a value whose declaration was qualified is fine. Getting
    /// this wrong in the other direction would refuse working shaders, so
    /// the test beside it fixes every edge: an explicit qualifier on the
    /// declaration, an int-only shader, a sampler-only shader, and the
    /// vertex stage, all of which must still compile.
    fn check_float_precision(&self, ty: &TypeSpec<'a>, name: &'a str, line: u32) -> R<'a, ()> {
        if self.stage != Stage::Fragment || self.default_float_precision {
            return Ok(());
        }
        if ty.precision != Precision::Default {
            return Ok(());
        }
        if !matches!(ty.base, BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..)) {
            return Ok(());
        }
        Err(Error::new(line, name, "'float' : type requires declaration of default precision qualifier"))
    }

    /// The GLSL ES 3.00 rules for an `in` or `out` (sections 4.3.4 to
    /// 4.3.6 and 4.3.9 of that specification), after `resolve_io` gave it
    /// its meaning. ES 1.00's attribute and varying rules apply as well, in
    /// `check_storage`, except where these widen them.
    fn check_interface(
        &self,
        storage: StorageQualifier,
        io: &IoQual,
        ty: &TypeSpec<'a>,
        name: &'a str,
        line: u32,
    ) -> R<'a, ()> {
        if !self.ast.es3 {
            return Ok(());
        }
        let int_base =
            |b: &BaseType<'_>| matches!(b, BaseType::Int | BaseType::IVec(_) | BaseType::Uint | BaseType::UVec(_));
        let mut integer = int_base(&ty.base);
        /* ES 3.00 section 4.3.4 lets a vertex output or fragment input be a
         * structure. Its members are what any varying may be (a float or
         * integer scalar or vector, or a float matrix); a member that is an
         * array, a structure, a bool or a sampler is not lowered here and is
         * refused by name. A structure with an integer member is an integer
         * input, and must be flat like one. */
        if let (StorageQualifier::Varying, BaseType::Struct(sname)) = (storage, ty.base) {
            if ty.array_len.is_some() {
                return Err(Error::new(
                    line,
                    name,
                    "an array of structures as a vertex output or fragment input is not lowered yet",
                ));
            }
            let members = self
                .ast
                .decls
                .iter()
                .find_map(|d| match d {
                    Decl::Struct { name: n, members, .. } if *n == sname => Some(*members),
                    _ => None,
                })
                .map(|r| &self.ast.members[r.indices()])
                .unwrap_or(&[]);
            for m in members {
                let ok = m.ty.array_len.is_none()
                    && matches!(
                        m.ty.base,
                        BaseType::Float
                            | BaseType::Vec(_)
                            | BaseType::Mat(..)
                            | BaseType::Int
                            | BaseType::IVec(_)
                            | BaseType::Uint
                            | BaseType::UVec(_)
                    );
                if !ok {
                    return Err(Error::new(
                        m.line,
                        m.name,
                        "a member of a structure between the stages must be a float or integer scalar, vector or matrix",
                    ));
                }
                integer |= int_base(&m.ty.base);
            }
        }
        match storage {
            StorageQualifier::Varying => {
                /* Section 4.3.4/4.3.6: an integer vertex output or fragment
                 * input "must be qualified with the interpolation qualifier
                 * flat". */
                if integer && io.interp != Interp::Flat {
                    return Err(Error::new(line, name, "an integer 'in' or 'out' between the stages must be 'flat'"));
                }
                if io.location.is_some() {
                    return Err(Error::new(line, name, "a vertex output or fragment input takes no layout location"));
                }
            }
            /* Section 4.3.6: a fragment output is a float, integer or
             * unsigned scalar or vector, or an array of those -- never a
             * bool, a matrix or a struct. */
            StorageQualifier::FragOut
                if !matches!(
                    ty.base,
                    BaseType::Float
                        | BaseType::Vec(_)
                        | BaseType::Int
                        | BaseType::IVec(_)
                        | BaseType::Uint
                        | BaseType::UVec(_)
                ) =>
            {
                return Err(Error::new(line, name, "a fragment output must be a float or integer scalar or vector"));
            }
            _ => {}
        }
        Ok(())
    }

    fn check_storage(
        &self,
        storage: StorageQualifier,
        ty: &TypeSpec<'a>,
        name: &'a str,
        line: u32,
        global: bool,
    ) -> R<'a, ()> {
        /* Section 4.3, stated once for every qualifier: "Local variables can
         * only use the storage qualifier const."
         *
         * This used to name `attribute` and `varying`, which is the same
         * rule written as a list -- and `uniform` was the item missing from
         * it, so `void f() { uniform float u; }` compiled. Section 4.3.4
         * also says the qualifier "is used to declare global variables", but
         * the general sentence is the one to enforce, because it cannot be
         * short by one. */
        if !global && !matches!(storage, StorageQualifier::None | StorageQualifier::Const) {
            return Err(Error::new(line, name, "a local variable may only be qualified 'const'"));
        }
        if storage == StorageQualifier::Attribute {
            if self.stage == Stage::Fragment {
                return Err(Error::new(line, name, "'attribute' is not allowed in a fragment shader"));
            }
            /* Section 4.3.3: an attribute is a float, vector or matrix, and
             * never an array. ES 3.00 (4.3.4) adds the integer types. */
            let es3_int = self.ast.es3
                && matches!(ty.base, BaseType::Int | BaseType::IVec(_) | BaseType::Uint | BaseType::UVec(_));
            if !es3_int && !matches!(ty.base, BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..)) {
                return Err(Error::new(line, name, "an attribute must be a float, vector or matrix"));
            }
            if ty.array_len.is_some() {
                return Err(Error::new(line, name, "an attribute cannot be an array"));
            }
        }
        /* Section 4.3.5: a varying is a float, vector or matrix, or an array
         * of those. */
        if storage == StorageQualifier::Varying
            && !matches!(ty.base, BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..))
            && !(self.ast.es3
                && matches!(
                    ty.base,
                    BaseType::Int | BaseType::IVec(_) | BaseType::Uint | BaseType::UVec(_) | BaseType::Struct(_)
                ))
        {
            return Err(Error::new(line, name, "a varying must be a float, vector or matrix"));
        }
        /* Section 4.1.7: a sampler is a uniform or a function parameter,
         * never a local or a global of its own. */
        if ty.base.is_sampler() && storage != StorageQualifier::Uniform {
            return Err(Error::new(line, name, "a sampler must be a uniform"));
        }
        Ok(())
    }

    /* ---- statements ---------------------------------------------------- */

    fn stmt(&mut self, id: StmtId) -> R<'a, ()> {
        match *self.ast.stmt(id) {
            Stmt::Expr(None) => Ok(()),
            Stmt::Expr(Some(e)) => self.expr(e).map(|_| ()),
            Stmt::Decl(r) => {
                for i in r.indices() {
                    let d = *self.ast.decl(self.ast.decl_ids[i]);
                    self.declaration(&d, false)?;
                }
                Ok(())
            }
            Stmt::Block(r) => {
                self.push_scope()?;
                let out = self.block(r);
                self.pop_scope();
                out
            }
            Stmt::If { cond, then_s, else_s } => {
                let c = self.expr(cond)?;
                self.require_bool(&c.ty)?;
                self.stmt(then_s)?;
                match else_s {
                    Some(e) => self.stmt(e),
                    None => Ok(()),
                }
            }
            Stmt::While { cond, body } => {
                self.push_scope()?;
                let r = self.loop_body(Some(cond), body);
                self.pop_scope();
                r
            }
            Stmt::DoWhile { body, cond } => {
                self.loop_depth += 1;
                let r = self.stmt(body);
                self.loop_depth -= 1;
                r?;
                let c = self.expr(cond)?;
                self.require_bool(&c.ty)
            }
            Stmt::For { init, cond, step, body } => {
                self.push_scope()?;
                let r = (|| {
                    if let Some(i) = init {
                        self.stmt(i)?;
                    }
                    let r = self.loop_body(cond, body);
                    if let Some(s) = step {
                        self.expr(s)?;
                    }
                    r
                })();
                self.pop_scope();
                r
            }
            Stmt::Break => {
                if self.loop_depth == 0 && self.switch_depth == 0 {
                    return Err(Error::new(0, "break", "'break' outside a loop or a switch"));
                }
                Ok(())
            }
            /* A `continue` in a switch continues the enclosing LOOP, so a
             * switch alone does not make one legal. */
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    return Err(Error::new(0, "continue", "'continue' outside a loop"));
                }
                Ok(())
            }
            Stmt::Switch { sel, body } => self.switch(sel, body),
            /* Reached only through a statement that is not the switch body
             * itself -- `if (c) case 1:`, a label in a loop inside the
             * switch, or no switch at all: section 6.2's labels belong to
             * the switch's own statement list. */
            Stmt::Case(e) => Err(Error::new(
                0,
                if e.is_some() { "case" } else { "default" },
                "a case label may only stand directly in the body of a switch",
            )),
            Stmt::Discard => {
                if self.stage != Stage::Fragment {
                    return Err(Error::new(0, "discard", "'discard' is only allowed in a fragment shader"));
                }
                Ok(())
            }
            /* Section 6.1, quoted on [`Sema::returns`]: a non-void function
             * must return a value of its declared type, and a void one must
             * return no value at all. `main` is void like any other, so
             * `return;` in it is fine and `return 1;` is not.
             *
             * dEQP `functions.invalid.return_without_value` and
             * `return_value_from_void_function`. */
            Stmt::Return(e) => {
                let want = self.returns.unwrap_or(Type::UNKNOWN);
                match e {
                    Some(e) => {
                        let v = self.expr(e)?;
                        if want.base == BaseType::Void {
                            return Err(Error::new(0, "return", "a void function cannot return a value"));
                        }
                        self.check_assignable(&want, &v.ty, 0, "return")
                    }
                    None => {
                        if want.base != BaseType::Void && !want.is_unknown() {
                            return Err(Error::new(0, "return", "this function must return a value"));
                        }
                        Ok(())
                    }
                }
            }
        }
    }

    /// ES 3.00 section 6.2, `switch`:
    ///
    /// - the expression is a scalar `int` or `uint`, and every `case`
    ///   label is a constant expression of that same type;
    /// - no two labels have the same value, and there is at most one
    ///   `default`;
    /// - no statement comes before the first label, and a label is not the
    ///   last thing in the body (dEQP `switch.no_statement_after_label`);
    /// - `break` inside it leaves it.
    ///
    /// The body is a scope, like any compound statement.
    fn switch(&mut self, sel: ExprId, body: StmtId) -> R<'a, ()> {
        let t = self.expr(sel)?;
        let int_like = matches!(t.ty.base, BaseType::Int | BaseType::Uint) && !t.ty.is_array();
        if !t.ty.is_unknown() && !int_like {
            return Err(Error::new(0, "switch", "the expression of a switch must be a scalar int or uint"));
        }
        let Stmt::Block(r) = *self.ast.stmt(body) else {
            return Err(Error::new(0, "switch", "the body of a switch must be a compound statement"));
        };
        self.push_scope()?;
        self.switch_depth += 1;
        let res = self.switch_body(t.ty.base, r);
        self.switch_depth -= 1;
        self.pop_scope();
        res
    }

    fn switch_body(&mut self, sel: BaseType<'a>, r: Range) -> R<'a, ()> {
        let mut seen: AVec<'a, i64> = AVec::new();
        let (mut labelled, mut default, mut last_label) = (false, false, false);
        for i in r.indices() {
            let id = self.ast.stmt_ids[i];
            match *self.ast.stmt(id) {
                Stmt::Case(Some(e)) => {
                    let v = self.expr(e)?;
                    if !self.const_expr(e) {
                        return Err(Error::new(0, "case", "a case label must be a constant expression"));
                    }
                    if !v.ty.is_unknown() && (v.ty.base != sel || v.ty.is_array()) {
                        return Err(Error::new(0, "case", "a case label must have the type of the switch expression"));
                    }
                    /* A value this can fold is checked against the others;
                     * one it cannot is let through rather than refused on a
                     * guess. */
                    if let Some(k) = self.const_int(e) {
                        if seen.as_slice().contains(&k) {
                            return Err(Error::new(0, "case", "two case labels have the same value"));
                        }
                        seen.push(self.a, k)?;
                    }
                    labelled = true;
                    last_label = true;
                }
                Stmt::Case(None) => {
                    if default {
                        return Err(Error::new(0, "default", "a switch may have only one 'default' label"));
                    }
                    default = true;
                    labelled = true;
                    last_label = true;
                }
                _ => {
                    if !labelled {
                        return Err(Error::new(0, "switch", "a statement in a switch comes before its first label"));
                    }
                    self.stmt(id)?;
                    last_label = false;
                }
            }
        }
        if last_label {
            return Err(Error::new(0, "switch", "a case label must be followed by a statement"));
        }
        Ok(())
    }

    /// A loop's controlling clause and body, with the loop depth raised so
    /// that `break` and `continue` are legal inside it.
    /// The statements of a compound statement, in whatever scope is current.
    ///
    /// Separate from the `Stmt::Block` arm because who pushes the scope is
    /// exactly what `statement_no_new_scope` decides -- see
    /// [`Sema::stmt_no_new_scope`].
    fn block(&mut self, r: Range) -> R<'a, ()> {
        for i in r.indices() {
            self.stmt(self.ast.stmt_ids[i])?;
        }
        Ok(())
    }

    /// The body of a `for` or a `while`, which SHARES the scope holding the
    /// loop's own declaration.
    ///
    /// Section 4.2.2: a variable "declared in a while test or a for
    /// statement ... is scoped to the end of the following
    /// statement-no-new-scope", and the grammar gives both loops a
    /// `statement_no_new_scope` body. So the braces after `for (int i = 0;
    /// ...)` do NOT open a scope of their own, and `int i` inside them is a
    /// second declaration of `i` in ONE scope, not a shadow of an outer one
    /// (section 4.2.7: at most once within a scope).
    ///
    /// A `do ... while` is different -- the grammar gives it
    /// `statement_with_scope` -- and so is a function body, which section
    /// 4.2.2 says in prose "has a scope nested inside the function's
    /// definition" (dEQP's `local_variable_hides_function_parameter` is a
    /// must-BUILD case, which settles it). Both keep the ordinary
    /// `Stmt::Block` path.
    fn stmt_no_new_scope(&mut self, id: StmtId) -> R<'a, ()> {
        match *self.ast.stmt(id) {
            Stmt::Block(r) => self.block(r),
            _ => self.stmt(id),
        }
    }

    fn loop_body(&mut self, cond: Option<Cond>, body: StmtId) -> R<'a, ()> {
        if let Some(c) = cond {
            match c {
                Cond::Expr(e) => {
                    let v = self.expr(e)?;
                    self.require_bool(&v.ty)?;
                }
                Cond::Decl(d) => {
                    let d = *self.ast.decl(d);
                    self.declaration(&d, false)?;
                }
            }
        }
        self.loop_depth += 1;
        let r = self.stmt_no_new_scope(body);
        self.loop_depth -= 1;
        r
    }

    fn require_bool(&self, ty: &Type<'a>) -> R<'a, ()> {
        if ty.is_unknown() || (ty.base == BaseType::Bool && !ty.is_array()) {
            return Ok(());
        }
        Err(Error::new(0, "", "a condition must be a bool"))
    }

    /* ---- expressions --------------------------------------------------- */

    /// The type of `id`, recorded as well as returned.
    ///
    /// The recording is here, in the one place every expression passes
    /// through, rather than at each `Ok` inside: a path added later cannot
    /// forget to do it.
    fn expr(&mut self, id: ExprId) -> R<'a, Val<'a>> {
        let v = self.expr_kind(id)?;
        if let Some(slot) = self.types.get_mut(id.0 as usize) {
            *slot = v.ty;
        }
        Ok(v)
    }

    fn expr_kind(&mut self, id: ExprId) -> R<'a, Val<'a>> {
        match *self.ast.expr(id) {
            Expr::IntLit(_) => Ok(Val { ty: Type::scalar(BaseType::Int), lvalue: false, constant: true }),
            Expr::UintLit(_) => Ok(Val { ty: Type::scalar(BaseType::Uint), lvalue: false, constant: true }),
            Expr::FloatLit(_) => Ok(Val { ty: Type::scalar(BaseType::Float), lvalue: false, constant: true }),
            Expr::BoolLit(_) => Ok(Val { ty: Type::scalar(BaseType::Bool), lvalue: false, constant: true }),
            Expr::Ident(name) => match self.lookup(name) {
                /* A structure's -- or a uniform block's -- name is a type,
                 * not a value (a constructor call is `Expr::Call`). dEQP's
                 * uniform_block.invalid.reference_using_block_name reads a
                 * member through the BLOCK name, which names no variable. */
                Some(s) if s.kind == NameKind::Type => Err(Error::new(0, name, "a type name is not a value")),
                Some(s) => Ok(Val {
                    ty: s.ty,
                    /* A uniform, an attribute, and a varying in the fragment
                     * stage, are read-only (sections 4.3.3 to 4.3.5). */
                    lvalue: !matches!(s.storage, StorageQualifier::Uniform | StorageQualifier::Attribute)
                        && !(s.storage == StorageQualifier::Varying && self.stage == Stage::Fragment)
                        && s.storage != StorageQualifier::Const,
                    constant: matches!(
                        s.storage,
                        StorageQualifier::Const | StorageQualifier::Uniform | StorageQualifier::Attribute
                    ),
                }),
                None => Err(Error::new(0, name, "undeclared identifier")),
            },
            Expr::Unary(op, x) => self.unary(op, x),
            Expr::Binary(op, l, r) => self.binary(op, l, r),
            Expr::Assign(op, l, r) => self.assign(op, l, r),
            Expr::Select(c, t, f) => {
                let c = self.expr(c)?;
                self.require_bool(&c.ty)?;
                let t = self.expr(t)?;
                let f = self.expr(f)?;
                /* Section 5.9: the two arms must have the same type. */
                if !t.ty.is_unknown() && !f.ty.is_unknown() && t.ty != f.ty {
                    return Err(Error::new(0, "?:", "the two arms of a conditional must have the same type"));
                }
                Ok(Val::rvalue(if t.ty.is_unknown() { f.ty } else { t.ty }))
            }
            Expr::Index(base, idx) => {
                let b = self.expr(base)?;
                let i = self.expr(idx)?;
                /* ES 1.00 section 5.7 indexes with an `int`; ES 3.00 section
                 * 5.7 with "an integer expression" of either signedness. */
                let integral = i.ty.base == BaseType::Int || (self.ast.es3 && i.ty.base == BaseType::Uint);
                if !i.ty.is_unknown() && !(integral && !i.ty.is_array()) {
                    return Err(Error::new(0, "[]", "an array index must be an integer"));
                }
                Ok(Val { ty: self.index_type(&b.ty), lvalue: b.lvalue, constant: b.constant })
            }
            Expr::Field(base, field) => {
                let b = self.expr(base)?;
                self.field(&b, field)
            }
            Expr::Call { name, args } => self.call(name, args),
            Expr::Comma(l, r) => {
                self.expr(l)?;
                self.expr(r)
            }
            Expr::ArrayCtor { base, len, args } => self.array_constructor(base, len, args),
            Expr::Length(x) => {
                /* ES 3.00 section 5.9: `.length()` applies to an array and
                 * "returns an integer constant". */
                let v = self.expr(x)?;
                if !v.ty.is_unknown() && !v.ty.is_array() {
                    return Err(Error::new(0, "length", "length() applies to an array"));
                }
                Ok(Val { ty: Type::scalar(BaseType::Int), lvalue: false, constant: true })
            }
        }
    }

    /// ES 3.00 `T[N](a, b, ...)` and `T[](a, b, ...)` (section 5.4.4):
    /// "There must be exactly the same number of arguments as the size of
    /// the array being constructed. If no size is present in the
    /// constructor, the array is explicitly sized to the number of arguments
    /// provided", and each argument has the element type -- there are no
    /// implicit conversions (4.1.10) and no arrays of arrays (4.1.9).
    fn array_constructor(&mut self, base: BaseType<'a>, len: Option<ExprId>, args: Range) -> R<'a, Val<'a>> {
        let mut n = 0u32;
        let mut unknown = false;
        for i in args.indices() {
            let v = self.expr(self.ast.expr_ids[i])?;
            n += 1;
            if v.ty.is_unknown() {
                unknown = true;
                continue;
            }
            if v.ty.is_array() {
                return Err(Error::new(0, "[]", "an array of arrays is not allowed"));
            }
            if v.ty.base != base {
                return Err(Error::new(0, "[]", "an array constructor's arguments must have the element type"));
            }
        }
        if base.is_sampler() || base == BaseType::Void {
            return Err(Error::new(0, "[]", "an array of this type cannot be constructed"));
        }
        if n == 0 {
            return Err(Error::new(0, "[]", "an array constructor needs at least one argument"));
        }
        if let Some(e) = len {
            match self.array_size(e) {
                Some(k) if k > 0 && k as u32 == n => {}
                Some(k) if k > 0 => {
                    return Err(Error::new(0, "[]", "an array constructor takes one argument per element"))
                }
                Some(_) => return Err(Error::new(0, "[]", "an array size must be greater than zero")),
                None => return Err(Error::new(0, "[]", "an array size must be a constant expression")),
            }
        }
        if unknown {
            return Ok(Val::rvalue(Type { base, array_len: n }));
        }
        Ok(Val::rvalue(Type { base, array_len: n }))
    }

    /// An array size: a constant integer expression, or ES 3.00's
    /// `a.length()`, whose value is the TYPE of `a` -- which only the
    /// checker has, since a name's declaration depends on scope.
    fn array_size(&mut self, e: ExprId) -> Option<i64> {
        if let Expr::Length(x) = *self.ast.expr(e) {
            let v = self.expr(e).ok()?;
            let _ = v;
            let t = self.types.get(x.0 as usize).copied()?;
            return t.is_array().then_some(i64::from(t.array_len));
        }
        self.const_int(e)
    }

    /// The type of `x[i]`.
    fn index_type(&self, ty: &Type<'a>) -> Type<'a> {
        if ty.is_unknown() {
            return Type::UNKNOWN;
        }
        if ty.is_array() {
            return Type { base: ty.base, array_len: 0 };
        }
        match ty.base {
            /* Indexing a matrix gives a column. */
            BaseType::Mat(_, r) => Type::scalar(BaseType::Vec(r)),
            BaseType::Vec(_) => Type::scalar(BaseType::Float),
            BaseType::IVec(_) => Type::scalar(BaseType::Int),
            BaseType::UVec(_) => Type::scalar(BaseType::Uint),
            BaseType::BVec(_) => Type::scalar(BaseType::Bool),
            _ => Type::UNKNOWN,
        }
    }

    /// `x.field`: a member of a struct, or a swizzle on a vector.
    fn field(&mut self, base: &Val<'a>, field: &'a str) -> R<'a, Val<'a>> {
        if base.ty.is_unknown() {
            return Ok(Val::unknown());
        }
        if base.ty.base == BaseType::Struct(DEPTH_RANGE_PARAMETERS) && !base.ty.is_array() {
            /* Section 7.5 declares it; no user struct can share the name,
             * since a user identifier cannot begin with `gl_` (3.7). */
            if !matches!(field, "near" | "far" | "diff") {
                return Err(Error::new(0, field, "no such member"));
            }
            return Ok(Val { ty: Type::scalar(BaseType::Float), lvalue: false, constant: base.constant });
        }
        if let BaseType::Struct(sname) = base.ty.base {
            let Some(members) = self.struct_members(sname) else {
                return Ok(Val::unknown());
            };
            let Some(m) = self.ast.members[members.indices()].iter().find(|m| m.name == field) else {
                return Err(Error::new(0, field, "no such member"));
            };
            let len = match m.ty.array_len {
                None => 0,
                Some(e) => self.const_int(e).unwrap_or(0).max(0) as u32,
            };
            return Ok(Val {
                ty: Type { base: m.ty.base, array_len: len },
                lvalue: base.lvalue,
                constant: base.constant,
            });
        }
        /* A swizzle. Section 5.5: the sets are xyzw, rgba and stpq, they may
         * not be mixed, and at most four components may be named. */
        let Some(kind) = scalar_of(&base.ty.base) else {
            return Err(Error::new(0, field, "this type has no fields"));
        };
        if !is_numeric_vector(&base.ty.base) || base.ty.is_array() {
            return Err(Error::new(0, field, "this type has no fields"));
        }
        let width = base.ty.base.components();
        let bytes = field.as_bytes();
        if bytes.is_empty() || bytes.len() > 4 {
            return Err(Error::new(0, field, "a swizzle names one to four components"));
        }
        let set_of = |c: u8| -> Option<(u8, u8)> {
            const SETS: [&[u8; 4]; 3] = [b"xyzw", b"rgba", b"stpq"];
            SETS.iter().enumerate().find_map(|(si, s)| s.iter().position(|&x| x == c).map(|i| (si as u8, i as u8)))
        };
        let (mut set, mut repeated, mut seen) = (None, false, [false; 4]);
        for &c in bytes {
            let Some((si, i)) = set_of(c) else {
                return Err(Error::new(0, field, "not a component name"));
            };
            if *set.get_or_insert(si) != si {
                return Err(Error::new(0, field, "a swizzle cannot mix component sets"));
            }
            if i >= width {
                return Err(Error::new(0, field, "component out of range for this type"));
            }
            repeated |= seen[i as usize];
            seen[i as usize] = true;
        }
        Ok(Val {
            ty: Type::scalar(vector_of(kind, bytes.len() as u8)),
            /* A swizzle that names a component twice is not an lvalue
             * (section 5.8). */
            lvalue: base.lvalue && !repeated,
            constant: base.constant,
        })
    }

    /// Whether the structure has an array anywhere inside it, including
    /// inside a structure member of its own -- section 6.1 says "does not
    /// contain an array", not "has no array member".
    ///
    /// `depth` bounds the walk. A structure cannot contain itself (its own
    /// name is not visible until the end of its declaration, section 4.2.2),
    /// so the nesting is finite, but a checker must not be the thing that
    /// proves it: an unbounded walk over a malformed unit is a hang inside
    /// `glCompileShader`.
    fn struct_contains_array(&self, name: &str, depth: u32) -> bool {
        const MAX_NESTING: u32 = 16;
        if depth >= MAX_NESTING {
            return false;
        }
        let Some(members) = self.struct_members(name) else { return false };
        self.ast.members[members.indices()].iter().any(|m| {
            m.ty.array_len.is_some()
                || matches!(m.ty.base, BaseType::Struct(inner) if self.struct_contains_array(inner, depth + 1))
        })
    }

    /// Is `base` a sampler, or a struct with one somewhere inside it?
    /// Section 4.1.7 applies the sampler restrictions to both.
    fn carries_sampler(&self, base: BaseType<'a>, depth: u32) -> bool {
        const MAX_NESTING: u32 = 16;
        match base {
            b if b.is_sampler() => true,
            BaseType::Struct(name) if depth < MAX_NESTING => {
                let Some(members) = self.struct_members(name) else { return false };
                self.ast.members[members.indices()].iter().any(|m| self.carries_sampler(m.ty.base, depth + 1))
            }
            _ => false,
        }
    }

    fn struct_members(&self, name: &str) -> Option<Range> {
        self.ast.decls.iter().find_map(|d| match d {
            Decl::Struct { name: n, members, .. } if *n == name => Some(*members),
            _ => None,
        })
    }

    fn unary(&mut self, op: UnOp, x: ExprId) -> R<'a, Val<'a>> {
        if op == UnOp::BitNot && !self.ast.es3 {
            /* Reserved by section 5.1, and parsed so that it can be named. */
            return Err(Error::new(0, "~", "reserved in GLSL ES 1.00"));
        }
        let v = self.expr(x)?;
        if v.ty.is_unknown() {
            return Ok(Val::unknown());
        }
        match op {
            UnOp::Not => {
                if v.ty.base != BaseType::Bool || v.ty.is_array() {
                    return Err(Error::new(0, "!", "'!' takes a bool"));
                }
                Ok(Val::rvalue(Type::scalar(BaseType::Bool)))
            }
            UnOp::Plus | UnOp::Minus => {
                if scalar_of(&v.ty.base) == Some(ScalarKind::Bool) || v.ty.is_array() || scalar_of(&v.ty.base).is_none()
                {
                    return Err(Error::new(0, "-", "this type has no arithmetic"));
                }
                Ok(Val::rvalue(v.ty))
            }
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                if !v.lvalue {
                    return Err(Error::new(0, "++", "cannot modify this expression"));
                }
                Ok(Val::rvalue(v.ty))
            }
            /* ES 3.00 section 5.9: "The operand must be of type signed or
             * unsigned integer or integer vector". */
            UnOp::BitNot => {
                if v.ty.is_array() || !scalar_of(&v.ty.base).is_some_and(is_integer) {
                    return Err(Error::new(0, "~", "'~' takes an integer"));
                }
                Ok(Val::rvalue(v.ty))
            }
        }
    }

    fn binary(&mut self, op: BinOp, l: ExprId, r: ExprId) -> R<'a, Val<'a>> {
        if is_integer_op(op) && !self.ast.es3 {
            /* Section 5.1 reserves these; using one is an error, which is
             * what dEQP's reserved_operators group reads. */
            return Err(Error::new(0, op.text(), "reserved in GLSL ES 1.00"));
        }
        let a = self.expr(l)?;
        let b = self.expr(r)?;
        if a.ty.is_unknown() || b.ty.is_unknown() {
            return Ok(Val::unknown());
        }
        match op {
            BinOp::AndAnd | BinOp::OrOr | BinOp::XorXor => {
                if a.ty.base != BaseType::Bool || b.ty.base != BaseType::Bool {
                    return Err(Error::new(0, "&&", "a logical operator takes bools"));
                }
                Ok(Val::rvalue(Type::scalar(BaseType::Bool)))
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                /* Section 5.1: the relational operators take two scalars. */
                let ok =
                    |t: &Type<'a>| matches!(t.base, BaseType::Float | BaseType::Int | BaseType::Uint) && !t.is_array();
                if !ok(&a.ty) || !ok(&b.ty) {
                    return Err(Error::new(0, "<", "a relational operator takes two scalars"));
                }
                if a.ty.base != b.ty.base {
                    return Err(Error::new(0, "<", "no implicit conversion in GLSL ES 1.00"));
                }
                Ok(Val::rvalue(Type::scalar(BaseType::Bool)))
            }
            BinOp::Eq | BinOp::Ne => {
                if a.ty != b.ty {
                    return Err(Error::new(0, "==", "the two sides must have the same type"));
                }
                /* ES 1.00 section 5.9: the equality operators "operate on all
                 * types except arrays, structures containing arrays, sampler
                 * types and structures containing sampler types". ES 3.00's
                 * section 5.9 drops the arrays from that list. */
                if a.ty.is_array() && !self.ast.es3 {
                    return Err(Error::new(0, "==", "arrays cannot be compared in GLSL ES 1.00"));
                }
                Ok(Val::rvalue(Type::scalar(BaseType::Bool)))
            }
            _ if is_integer_op(op) => self.integer_operator(op, &a.ty, &b.ty),
            _ => self.arithmetic(op, &a.ty, &b.ty),
        }
    }

    /// The type of ES 3.00's integer operators, `%`, `&`, `|`, `^`, `<<` and
    /// `>>` (ES 3.00 section 5.9).
    ///
    /// `%` and the bit-wise three follow the arithmetic operators' shape
    /// rule -- the same type, or a scalar with a vector, whose type the
    /// result takes -- over integers only, and "the fundamental types of the
    /// operands (signed or unsigned) must match". The shifts are the
    /// exception on both counts: "One operand can be signed while the other
    /// is unsigned. In all cases, the resulting type will be the same type
    /// as the left operand. If the first operand is a scalar, the second
    /// operand has to be a scalar as well. If the first operand is a vector,
    /// the second operand must be a scalar or a vector with the same size".
    fn integer_operator(&self, op: BinOp, a: &Type<'a>, b: &Type<'a>) -> R<'a, Val<'a>> {
        let t = op.text();
        if a.is_array() || b.is_array() {
            return Err(Error::new(0, t, "an array has no arithmetic"));
        }
        let (Some(ka), Some(kb)) = (scalar_of(&a.base), scalar_of(&b.base)) else {
            return Err(Error::new(0, t, "this operator takes integers"));
        };
        if !is_integer(ka) || !is_integer(kb) || !is_numeric_vector(&a.base) || !is_numeric_vector(&b.base) {
            return Err(Error::new(0, t, "this operator takes integers"));
        }
        let (an, bn) = (a.base.components(), b.base.components());
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            if bn != 1 && bn != an {
                return Err(Error::new(0, t, "a shift's right operand must be a scalar or as wide as the left"));
            }
            return Ok(Val::rvalue(*a));
        }
        if ka != kb {
            return Err(Error::new(0, t, "the operands must both be signed or both unsigned"));
        }
        if an == 1 {
            return Ok(Val::rvalue(*b));
        }
        if bn != 1 && an != bn {
            return Err(Error::new(0, t, "the two sides must have the same type"));
        }
        Ok(Val::rvalue(*a))
    }

    /// The type of `a op b` for `+ - * /` (section 5.1).
    fn arithmetic(&self, op: BinOp, a: &Type<'a>, b: &Type<'a>) -> R<'a, Val<'a>> {
        if a.is_array() || b.is_array() {
            return Err(Error::new(0, "+", "an array has no arithmetic"));
        }
        let (Some(ka), Some(kb)) = (scalar_of(&a.base), scalar_of(&b.base)) else {
            return Err(Error::new(0, "+", "this type has no arithmetic"));
        };
        if ka == ScalarKind::Bool || kb == ScalarKind::Bool {
            return Err(Error::new(0, "+", "a bool has no arithmetic"));
        }
        if ka != kb {
            /* The rule this module exists for: section 4.1.10 has no implicit
             * conversions, in either direction. */
            return Err(Error::new(0, "+", "no implicit conversion in GLSL ES 1.00"));
        }
        let (am, bm) = (matches!(a.base, BaseType::Mat(..)), matches!(b.base, BaseType::Mat(..)));
        let (an, bn) = (a.base.components(), b.base.components());
        /* A scalar combines with anything and gives the other type. */
        if an == 1 && !am {
            return Ok(Val::rvalue(*b));
        }
        if bn == 1 && !bm {
            return Ok(Val::rvalue(*a));
        }
        if op == BinOp::Mul && (am || bm) {
            /* Matrix products are the only place the operand types
             * legitimately differ (section 5.11): a matCxR times a vecC is
             * a vecR, a vecR times a matCxR is a vecC, and a matCxR times a
             * matPxC is a matPxR -- the inner sizes must agree. */
            let bad = || Err(Error::new(0, "*", "these operand types cannot be multiplied"));
            return match (a.base, b.base) {
                (BaseType::Mat(c, r), BaseType::Vec(k)) if k == c => Ok(Val::rvalue(Type::scalar(BaseType::Vec(r)))),
                (BaseType::Vec(k), BaseType::Mat(c, r)) if k == r => Ok(Val::rvalue(Type::scalar(BaseType::Vec(c)))),
                (BaseType::Mat(c1, r1), BaseType::Mat(c2, r2)) if c1 == r2 => {
                    Ok(Val::rvalue(Type::scalar(BaseType::Mat(c2, r1))))
                }
                _ => bad(),
            };
        }
        if a.base != b.base {
            return Err(Error::new(0, "+", "the two sides must have the same type"));
        }
        Ok(Val::rvalue(*a))
    }

    fn assign(&mut self, op: Option<BinOp>, l: ExprId, r: ExprId) -> R<'a, Val<'a>> {
        /* Section 5.8: "The assignments remainder into (%=), left shift by
         * (<<=), right shift by (>>=), inclusive or into ( |=), and exclusive
         * or into ( ^=) are reserved for future use", and section 5.1's
         * table reserves `&=` with them. dEQP reserved_operators
         * operator_assign_* (12 cases) were refused only by the lowering,
         * as a gap. */
        if let Some(op) = op.filter(|&op| is_integer_op(op) && !self.ast.es3) {
            let text = match op {
                BinOp::Mod => "%=",
                BinOp::BitAnd => "&=",
                BinOp::BitOr => "|=",
                BinOp::BitXor => "^=",
                BinOp::Shl => "<<=",
                _ => ">>=",
            };
            return Err(Error::new(0, text, "reserved in GLSL ES 1.00"));
        }
        let lhs = self.expr(l)?;
        let rhs = self.expr(r)?;
        if !lhs.lvalue {
            return Err(Error::new(0, "=", "cannot assign to this expression"));
        }
        /* Section 4.1.7: "Samplers cannot be treated as l-values ...
         * These restrictions also apply to any structures that contain
         * sampler types." The TARGET's type decides: `s.f = 1.0` on a struct
         * holding a sampler assigns a float, not the struct. */
        if self.carries_sampler(lhs.ty.base, 0) {
            return Err(Error::new(0, "=", "a sampler, or a struct holding one, cannot be assigned to"));
        }
        let Some(op) = op else {
            self.check_assignable(&lhs.ty, &rhs.ty, 0, "=")?;
            return Ok(Val::rvalue(lhs.ty));
        };
        /* `a op= b` means `a = a op b` and must type-check as one. */
        if lhs.ty.is_unknown() || rhs.ty.is_unknown() {
            return Ok(Val::rvalue(lhs.ty));
        }
        let v = if is_integer_op(op) {
            self.integer_operator(op, &lhs.ty, &rhs.ty)?
        } else {
            self.arithmetic(op, &lhs.ty, &rhs.ty)?
        };
        if !v.ty.is_unknown() && v.ty != lhs.ty {
            return Err(Error::new(0, "=", "the result does not fit the left-hand side"));
        }
        Ok(Val::rvalue(lhs.ty))
    }

    /// May a value of type `from` initialise, or be assigned to, `to`?
    fn check_assignable(&self, to: &Type<'a>, from: &Type<'a>, line: u32, what: &'a str) -> R<'a, ()> {
        if to.is_unknown() || from.is_unknown() || to == from {
            return Ok(());
        }
        if scalar_of(&to.base).is_some() && scalar_of(&to.base) != scalar_of(&from.base) {
            return Err(Error::new(line, what, "no implicit conversion in GLSL ES 1.00"));
        }
        Err(Error::new(line, what, "the types do not match"))
    }

    /// A call: a constructor, a struct constructor, a built-in, or a user
    /// function.
    fn call(&mut self, name: &'a str, args: Range) -> R<'a, Val<'a>> {
        /* The arguments are walked whatever the callee is, so their own
         * errors are reported even when the call's type cannot be. */
        let mut arg_tys: AVec<Type<'a>> = AVec::new();
        for i in args.indices() {
            let v = self.expr(self.ast.expr_ids[i])?;
            arg_tys.push(self.a, v.ty)?;
        }
        if let Some(base) = constructor_type(name, self.ast.es3) {
            return self.constructor(base, arg_tys.as_slice());
        }
        /* A structure's constructor is its NAME, and section 4.2.2 scopes
         * that name like any other: "A structure name declaration is
         * visible at the end of the struct_specifier in which it was
         * declared", and its own example hides one behind a variable:
         *
         *     S S = S(0,0); // 'S' is only visible as a struct and constructor
         *     S;            // 'S' is now visible only as a variable
         *
         * So the call is a constructor only when the innermost visible
         * declaration of the name IS the structure. `struct_members` scans
         * the whole AST and answered for a structure declared anywhere;
         * dEQP scoping.invalid.use_struct_type_before_declaration calls
         * `S(x).val` in a function written above `struct S`, and it compiled
         * as soon as the member read of a call was lowered (2026-09-23) --
         * until then a lowering gap had been refusing it for the wrong
         * reason. */
        if self.struct_members(name).is_some() {
            match self.lookup(name) {
                Some(sym) if sym.kind == NameKind::Type => {
                    return Ok(Val::rvalue(Type::scalar(BaseType::Struct(name))));
                }
                None => return Err(Error::new(0, name, "this structure is used before it is declared")),
                Some(_) => {
                    return Err(Error::new(
                        0,
                        name,
                        "this name is hidden by a variable, so it is not a constructor here",
                    ))
                }
            }
        }
        /* A user-defined function declares its return type, so there is
         * nothing to infer -- but the ARGUMENTS still have to match, and
         * that is checked here rather than in the lookup.
         *
         * `resolve_overload` answers "which declaration is this call for",
         * and when there is only ONE candidate it answers with that one
         * without comparing types, which is right for a lookup and useless
         * as a check. So `void func(float)` called as `func(2)` resolved,
         * typed, and compiled. Section 6.1 gives the call its parameter
         * types and section 4.1.10 has no implicit conversions in either
         * direction, so a match is exact.
         *
         * Six dEQP must-fail cases -- base type, scalar against vector,
         * vector size -- and measuring is what found it: making an
         * UNMATCHED call an error moved neither column, because with one
         * declaration nothing was ever unmatched. */
        /* Section 4.2.2: the scope of a name declared outside all function
         * definitions "starts from where it is declared". A call to a
         * function whose first declaration comes LATER in the unit is out
         * of scope, however well the arguments would have matched -- which
         * is why this is asked before the overload is resolved.
         *
         * `resolve_overload` scans the whole AST, so it answers for a
         * function declared anywhere; the symbol table is what carries the
         * position. dEQP `scoping.invalid.use_function_before_declaration`. */
        if self.lookup(name).is_none()
            && self.ast.decls.iter().any(|d| matches!(d, Decl::Function { name: n, .. } if *n == name))
        {
            return Err(Error::new(0, name, "this function is called before it is declared"));
        }
        if let Some(d) = resolve_overload(self.ast, name, arg_tys.as_slice(), false) {
            if let Decl::Function { ret, params, .. } = self.ast.decl(d) {
                self.check_call_arguments(name, &self.ast.params[params.indices()], arg_tys.as_slice())?;
                let rlen = match ret.array_len {
                    Some(e) => super::reflect::const_int_expr(self.ast, e).unwrap_or(0).max(0) as u32,
                    None => 0,
                };
                return Ok(Val::rvalue(Type { base: ret.base, array_len: rlen }));
            }
        }
        if self.ast.es3 {
            if let Some(r) = self.texture_es3(name, args, arg_tys.as_slice()) {
                return r;
            }
            if let Some(r) = matrix_builtin_es3(name, arg_tys.as_slice()) {
                return r;
            }
            if let Some(r) = common_builtin_es3(name, arg_tys.as_slice()) {
                return r;
            }
            /* modf(genType x, out genType i) (section 8.3): the fraction,
             * with the whole part written to i -- which must be something a
             * value can be written to. */
            if name == "modf" {
                let tys = arg_tys.as_slice();
                if tys.iter().any(|t| t.is_unknown()) {
                    return Ok(Val::unknown());
                }
                let float_gen = |t: &Type<'_>| !t.is_array() && matches!(t.base, BaseType::Float | BaseType::Vec(_));
                let [x, i] = tys else {
                    return Err(Error::new(0, name, "modf takes two arguments (section 8.3)"));
                };
                if !float_gen(x) || x != i {
                    return Err(Error::new(0, name, "no form of this function takes these arguments (section 8.3)"));
                }
                let out_arg = self.ast.expr_ids[args.start as usize + 1];
                if !self.expr(out_arg)?.lvalue {
                    return Err(Error::new(0, name, "an out argument must be something that can be written"));
                }
                return Ok(Val::rvalue(*x));
            }
            /* ES 3.00 has none of ES 1.00's texture look-ups: section 8.8 of
             * that specification spells them `texture`, `textureProj`,
             * `textureLod` ..., and `texture2D` is not among them. */
            if ES100_TEXTURES.contains(&name) {
                return Err(Error::new(0, name, "not a GLSL ES 3.00 function (section 8.8 names it texture...)"));
            }
        }
        self.check_texture_stage(name, arg_tys.len())?;
        if let Some(ty) = builtin_return(name, arg_tys.as_slice()) {
            return Ok(Val::rvalue(ty));
        }
        /* The name IS a user-defined function and no declaration of it
         * accepts these arguments.
         *
         * Section 6.1 gives the call the declared parameter types, and
         * section 4.1.10 has no implicit conversions in either direction, so
         * a match is exact -- `void func(float)` called as `func(2)` matches
         * nothing. dEQP checks three shapes of that (base type, scalar
         * against vector, vector size) and all six cases COMPILED here,
         * because an unmatched call fell through to "unknown" and unknown
         * silences every rule above it.
         *
         * Unknown is still the answer when the name is not a function of
         * ours -- an unmodelled built-in must stay silent -- and when any
         * ARGUMENT was itself untypeable, since then the mismatch might be
         * ours rather than the shader's. */
        let is_user_function = self.ast.decls.iter().any(|d| matches!(d, Decl::Function { name: n, .. } if *n == name));
        if is_user_function && !arg_tys.as_slice().iter().any(|t| t.is_unknown()) {
            return Err(Error::new(0, name, "no declaration of this function takes these arguments"));
        }
        /* A name that is no built-in, no constructor, no structure and no
         * function of this unit. Section 6.1: "All functions must be either
         * declared with a prototype or defined with a body before they are
         * called", and section 4.2.6 puts variables and functions in one
         * name space, so a variable's name is not callable either.
         *
         * `BUILTINS` names every section 8 function (and the derivatives
         * of OES_standard_derivatives), so "an unmodelled built-in" -- the
         * reason this answered unknown until 2026-09-23 -- no longer exists.
         * Until then `vec(1.0)` compiled here and only a lowering gap
         * refused it, in words that said the compiler owed it (the host
         * dEQP run, shaders.constants.const_float_assign_varying's vertex
         * shader). */
        if !is_user_function && !BUILTINS.iter().any(|(n, _)| *n == name) {
            return Err(match self.lookup(name) {
                Some(s) if s.kind == NameKind::Variable => {
                    Error::new(0, name, "this name is a variable, not a function")
                }
                _ => Error::new(0, name, "no function of this name is declared"),
            });
        }
        Ok(Val::unknown())
    }

    /// Section 8.7, the two sentences that make a texture lookup depend on
    /// the stage it is written in:
    ///
    /// > Functions containing the bias parameter are available only in the
    /// > fragment shader.
    ///
    /// > The built-ins suffixed with "Lod" are allowed only in a vertex
    /// > shader.
    ///
    /// The reason is in the paragraph above them: level of detail is not
    /// computed by fixed functionality for vertex shaders, so the vertex
    /// stage must be told the lod and the fragment stage may only adjust it.
    ///
    /// The bias forms are told apart by ARITY, because bias is always the
    /// third argument of the three lookups that take one, and `texture2DProj`
    /// takes either a `vec3` or a `vec4` coordinate without changing that.
    ///
    /// Only reached when no user declaration matched the call, so a user's
    /// own overload of one of these names is judged as the user function it
    /// is (and redefining the built-in signature itself is already refused).
    ///
    /// dEQP `invalid_texture_functions.texture2d_bias_in_vertex` and
    /// `texture2dlod_in_fragment`, plus the two `textureCube` twins, which
    /// were already refused but not for this reason.
    fn check_texture_stage(&self, name: &'a str, argc: usize) -> R<'a, ()> {
        const LOD: &[&str] = &["texture2DLod", "texture2DProjLod", "textureCubeLod"];
        const BIAS_AT_3: &[&str] = &["texture2D", "texture2DProj", "textureCube"];
        if LOD.contains(&name) && self.stage != Stage::Vertex {
            return Err(Error::new(0, name, "a 'Lod' texture lookup is only allowed in a vertex shader"));
        }
        if argc == 3 && BIAS_AT_3.contains(&name) && self.stage != Stage::Fragment {
            return Err(Error::new(0, name, "a texture lookup with a bias is only allowed in a fragment shader"));
        }
        Ok(())
    }

    /// ES 3.00 section 8.8's texture look-ups, typed against the exact
    /// signatures the section lists. `None`: `name` is not one of them.
    ///
    /// Every function is a set of (sampler, coordinate, extras) rows; which
    /// rows exist depends on the sampler's dimension and on shadow, and the
    /// ones the section does not list are refused -- `textureOffset` on a
    /// cube, `texelFetch` on a shadow sampler, `textureLod` on
    /// `samplerCubeShadow`. The bias forms are fragment-only ("the bias
    /// parameter is not accepted in a vertex shader"), and an offset must be
    /// a constant expression.
    fn texture_es3(&self, name: &'a str, args: Range, tys: &[Type<'a>]) -> Option<R<'a, Val<'a>>> {
        if !ES300_TEXTURES.contains(&name) {
            return None;
        }
        Some(self.texture_es3_inner(name, args, tys))
    }

    fn texture_es3_inner(&self, name: &'a str, args: Range, tys: &[Type<'a>]) -> R<'a, Val<'a>> {
        let bad = || Error::new(0, name, "no form of this texture look-up takes these arguments (section 8.8)");
        if tys.iter().any(|t| t.is_unknown()) {
            return Ok(Val::unknown());
        }
        let Some(BaseType::Sampler(s)) = tys.first().filter(|t| !t.is_array()).map(|t| t.base) else {
            return Err(bad());
        };
        if tys.iter().skip(1).any(|t| t.is_array()) {
            return Err(bad());
        }
        let (d2, cube, arr) = (s.dim == SamplerDim::D2, s.dim == SamplerDim::Cube, s.dim == SamplerDim::D2Array);
        /* The coordinate's width without shadow or projection, and the
         * gradient and offset widths. */
        let base_w: u8 = if d2 { 2 } else { 3 };
        let grad_w: u8 = if d2 || arr { 2 } else { 3 };
        let vec = |n: u8| if n == 1 { BaseType::Float } else { BaseType::Vec(n) };
        let ivec = |n: u8| if n == 1 { BaseType::Int } else { BaseType::IVec(n) };
        let result = || match (s.shadow, s.ret) {
            (true, _) => BaseType::Float,
            (_, SamplerRet::Float) => BaseType::Vec(4),
            (_, SamplerRet::Int) => BaseType::IVec(4),
            (_, SamplerRet::Uint) => BaseType::UVec(4),
        };
        let ok = |b: BaseType<'a>| Ok(Val::rvalue(Type::scalar(b)));
        let is = |k: usize, b: BaseType<'a>| tys.get(k).is_some_and(|t| t.base == b);

        if name == "textureSize" {
            if tys.len() != 2 || !is(1, BaseType::Int) {
                return Err(bad());
            }
            return ok(ivec(if d2 || cube { 2 } else { 3 }));
        }
        let proj = name.starts_with("textureProj");
        let fetch = name.starts_with("texelFetch");
        let offset = name.ends_with("Offset");
        let grad = name.contains("Grad");
        let lod = name.contains("Lod");
        /* Which samplers have the function at all (section 8.8's lists). */
        let exists = if fetch {
            !s.shadow && !cube
        } else if proj {
            (d2 || s.dim == SamplerDim::D3) && !(s.shadow && s.dim == SamplerDim::D3)
        } else if grad && offset {
            !cube
        } else if grad {
            true
        } else if lod {
            !(s.shadow && (cube || arr)) && !(offset && cube)
        } else if offset {
            !cube && !(s.shadow && arr)
        } else {
            true
        };
        if !exists {
            return Err(bad());
        }
        let mut k = 1;
        /* The coordinate. */
        let coord_ok = if fetch {
            is(k, ivec(base_w))
        } else if proj {
            let w = if s.shadow || s.dim == SamplerDim::D3 { 4 } else { 3 };
            is(k, vec(w)) || (d2 && !s.shadow && is(k, BaseType::Vec(4)))
        } else {
            is(k, vec(base_w + u8::from(s.shadow)))
        };
        if !coord_ok {
            return Err(bad());
        }
        k += 1;
        if fetch || lod {
            if !is(k, if fetch { BaseType::Int } else { BaseType::Float }) {
                return Err(bad());
            }
            k += 1;
        }
        if grad {
            if !is(k, vec(grad_w)) || !is(k + 1, vec(grad_w)) {
                return Err(bad());
            }
            k += 2;
        }
        if offset {
            if !is(k, ivec(if s.dim == SamplerDim::D3 { 3 } else { 2 })) {
                return Err(bad());
            }
            /* Section 8.8: "The offset value must be a constant expression." */
            if !self.const_expr(self.ast.expr_ids[args.start as usize + k]) {
                return Err(Error::new(0, name, "a texel offset must be a constant expression (section 8.8)"));
            }
            k += 1;
        }
        /* An optional trailing bias: only the forms with an implicit level,
         * never `texture` on a sampler2DArrayShadow, and only in a fragment
         * shader. */
        if tys.len() == k + 1 {
            let biasable = !fetch && !lod && !grad && !(name == "texture" && s.shadow && arr);
            if !biasable || !is(k, BaseType::Float) {
                return Err(bad());
            }
            if self.stage != Stage::Fragment {
                return Err(Error::new(0, name, "a texture look-up with a bias is only allowed in a fragment shader"));
            }
            k += 1;
        }
        if tys.len() != k {
            return Err(bad());
        }
        ok(result())
    }

    /// Section 6.1 and 4.1.10: every argument must already have its
    /// parameter's type, exactly.
    ///
    /// An argument the checker could not type is skipped rather than
    /// reported: the mismatch might be the compiler's and not the shader's,
    /// which is the same conservative direction an unknown call takes.
    fn check_call_arguments(&self, name: &'a str, ps: &[Param<'a>], args: &[Type<'a>]) -> R<'a, ()> {
        if ps.len() != args.len() {
            return Err(Error::new(0, name, "wrong number of arguments for this function"));
        }
        for (p, a) in ps.iter().zip(args) {
            if a.is_unknown() {
                continue;
            }
            if p.ty.base != a.base || p.ty.array_len.is_some() != a.is_array() {
                return Err(Error::new(0, p.name.unwrap_or(name), "this argument is not the parameter's type"));
            }
        }
        Ok(())
    }

    /// `vec4(...)`, `mat3(...)`, `int(...)`: section 5.4.
    fn constructor(&self, base: BaseType<'static>, args: &[Type<'a>]) -> R<'a, Val<'a>> {
        if args.is_empty() {
            return Err(Error::new(0, "()", "a constructor needs at least one argument"));
        }
        if args.iter().any(|t| t.is_unknown()) {
            return Ok(Val::rvalue(Type::scalar(base)));
        }
        if args.iter().any(|t| t.is_array() || matches!(t.base, BaseType::Struct(_)) || t.base.is_sampler()) {
            return Err(Error::new(0, "()", "this type cannot be used in a constructor"));
        }
        /* Section 5.4.2, as it states the rules:
         *
         *   - one SCALAR argument fills a vector, or a matrix's diagonal;
         *   - "If a matrix is constructed from a matrix" the overlap is
         *     copied, whatever the two sizes, but "If a matrix argument is
         *     given to a matrix constructor, it is an error to have any
         *     other arguments";
         *   - otherwise "there must be enough components provided in the
         *     arguments to provide an initializer for every component",
         *     and "It is an error to provide extra arguments beyond this
         *     last used argument".
         *
         * A scalar constructor is the same rule with one component:
         * `float(vec3)` takes the first and `float(a, b)` has an extra.
         *
         * The count is a question about each argument's TYPE: a `matN` is
         * N*N components, not the N columns `components()` answers. Until
         * 2026-09-23 a single argument was accepted whatever its width, so
         * `bvec3(bvec2)` compiled (dEQP conversions.vector_illegal, 54
         * cases, refused only by a lowering gap for the wrong reason). */
        fn comps(b: BaseType<'_>) -> u32 {
            match b {
                BaseType::Mat(c, r) => u32::from(c) * u32::from(r),
                other => u32::from(other.components().max(1)),
            }
        }
        let need = comps(base);
        let target_is_matrix = matches!(base, BaseType::Mat(..));
        if args.len() == 1 {
            let a = args[0].base;
            if comps(a) == 1 || (target_is_matrix && matches!(a, BaseType::Mat(..))) || comps(a) >= need {
                return Ok(Val::rvalue(Type::scalar(base)));
            }
            return Err(Error::new(0, "()", "not enough arguments for this constructor"));
        }
        if target_is_matrix && args.iter().any(|t| matches!(t.base, BaseType::Mat(..))) {
            return Err(Error::new(0, "()", "a matrix argument to a matrix constructor must be its only argument"));
        }
        let have: u32 = args.iter().map(|t| comps(t.base)).sum();
        if have < need {
            return Err(Error::new(0, "()", "not enough arguments for this constructor"));
        }
        let before_last: u32 = args[..args.len() - 1].iter().map(|t| comps(t.base)).sum();
        if before_last >= need {
            return Err(Error::new(0, "()", "too many arguments for this constructor"));
        }
        Ok(Val::rvalue(Type::scalar(base)))
    }

    /// Fold a constant integer expression; the same rule `reflect.rs` uses,
    /// so an array size means the same thing to both.
    /// Section 5.10: is this expression a CONSTANT EXPRESSION?
    ///
    /// Literals, `const` variables whose own initialisers were constant
    /// expressions, constructors of those, and built-in calls whose
    /// arguments are all constant expressions -- except the texture
    /// look-ups, which the section excludes by name.
    ///
    /// Everything else is not, and the interesting cases are the ones that
    /// LOOK constant: a uniform is fixed for a draw but not at compile time,
    /// and a `const` parameter is read-only but arrives at run time.
    fn const_expr(&self, e: ExprId) -> bool {
        match *self.ast.expr(e) {
            Expr::IntLit(_) | Expr::UintLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) => true,
            /* ES 3.00 section 5.10: an array constructor whose arguments are
             * all constant expressions is one, and `.length()` of an
             * explicitly sized array is an integer constant. */
            Expr::ArrayCtor { args, .. } => args.indices().all(|i| self.const_expr(self.ast.expr_ids[i])),
            Expr::Length(_) => true,
            Expr::Ident(name) => match self.lookup(name) {
                Some(s) => s.storage == StorageQualifier::Const && !s.is_param,
                None => false,
            },
            Expr::Unary(_, x) => self.const_expr(x),
            Expr::Binary(_, l, r) | Expr::Comma(l, r) => self.const_expr(l) && self.const_expr(r),
            Expr::Select(c, t, f) => self.const_expr(c) && self.const_expr(t) && self.const_expr(f),
            Expr::Index(b, i) => self.const_expr(b) && self.const_expr(i),
            Expr::Field(b, _) => self.const_expr(b),
            /* An assignment is not an expression that can be constant, and
             * ES 1.00 does not allow one in an initialiser anyway. */
            Expr::Assign(..) => false,
            Expr::Call { name, args } => {
                let every_arg = || args.indices().all(|i| self.const_expr(self.ast.expr_ids[i]));
                if constructor_type(name, self.ast.es3).is_some() || self.struct_members(name).is_some() {
                    return every_arg();
                }
                if resolve_overload(self.ast, name, &[], false).is_some()
                    || self.ast.decls.iter().any(|d| matches!(d, Decl::Function { name: n, .. } if *n == name))
                {
                    /* A user-defined function, whatever its arguments: its
                     * body runs, so its result is not a constant expression
                     * (5.10). */
                    return false;
                }
                /* A built-in. The texture look-ups are excluded by name in
                 * 5.10; everything else is constant when its arguments are. */
                if name.starts_with("texture") {
                    return false;
                }
                every_arg()
            }
        }
    }

    fn const_int(&self, e: ExprId) -> Option<i64> {
        super::reflect::const_int_expr(self.ast, e)
    }
}

/// Do two written precisions DISAGREE?
///
/// Only when both are explicit. `Precision::Default` is not a precision --
/// it is "whichever one is in force here" -- so a declaration that writes
/// none cannot contradict one that writes the default's own value.
/// dEQP's `declarations.default_vs_explicit_precision` is exactly that
/// shape and must BUILD:
///
/// ```text
/// precision mediump float;
/// float func (float f);              <- Default, meaning mediump
/// float func (mediump float f) {}    <- the same thing, written out
/// ```
///
/// while `invalid.argument_precision_overload` writes `lowp` against
/// `mediump` and must not. Comparing the written tokens refused the first
/// as well; that cost two must-build cases when this check first landed,
/// and the diff against the previous build is how it was found.
///
/// WHAT THIS DOES NOT CATCH: a prototype writing nothing and a definition
/// writing a precision that is NOT the default (`precision mediump float;`
/// with `float func(float);` and `float func(lowp float)`). Resolving that
/// needs the default-precision stack per type, which the checker does not
/// keep; dEQP does not test it, and inventing a verdict without the value
/// in hand is what a wrong refusal is made of.
fn precisions_disagree(a: Precision, b: Precision) -> bool {
    a != Precision::Default && b != Precision::Default && a != b
}

/// Do two parameter lists have the same SIGNATURE?
///
/// The base types and their array-ness, and nothing else: section 6.1 makes
/// the qualifiers and the precisions things that must AGREE between two
/// declarations of one signature, not things that tell two signatures apart.
/// One parameter slot of a built-in's prototype, as chapter 8 writes it.
///
/// The families (`Gen`, `Vec`, `IVec`, `BVec`, `Mat`) stand for a set of
/// widths, and every family slot in ONE prototype must take the SAME width:
/// `genType mix (genType x, genType y, float a)` is three floats or three
/// vec3s, never a mix. No ES 1.00 prototype needs two independent widths,
/// which is why one width is threaded through a match.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arg {
    /// `genType`: `float`, `vec2`, `vec3` or `vec4`.
    Gen,
    Float,
    /// `vec`: `vec2`..`vec4`, with no scalar -- what section 8.6 means.
    Vec,
    IVec,
    BVec,
    Vec2,
    Vec3,
    Vec4,
    /// `mat`: `mat2`..`mat4`.
    Mat,
    Sampler2D,
    SamplerCube,
}

/// The parameter lists each built-in ALREADY HAS, transcribed from chapter 8
/// of the specification.
///
/// Section 4.2.6: "Functions (including built-in functions) may not be
/// redefined. User defined functions may overload built-in functions", and
/// section 10.10 resolves it in those words, with `int sin(int x)` given as
/// a legal overload. So the question a user declaration has to be asked is
/// exactly "does a built-in of this name already have this SIGNATURE", and
/// the answer needs the real parameter lists -- `builtin_return` cannot give
/// it, because it computes a result from whatever arguments it is handed and
/// accepts every one of them.
///
/// A name absent from this table claims NOTHING, and so does a shape the
/// table cannot express. Being too narrow leaves a shader compiling that a
/// conformant implementation would refuse; being too WIDE refuses a legal
/// overload, which is the direction that breaks working shaders. The
/// derivative built-ins (`dFdx`, `dFdy`, `fwidth`) are deliberately absent:
/// they come from an extension rather than from the core language, so
/// whether they exist at all depends on a directive.
const BUILTIN_SIGS: &[(&str, &[&[Arg]])] = &[
    /* 8.1 angle and trigonometry */
    ("radians", &[&[Arg::Gen]]),
    ("degrees", &[&[Arg::Gen]]),
    ("sin", &[&[Arg::Gen]]),
    ("cos", &[&[Arg::Gen]]),
    ("tan", &[&[Arg::Gen]]),
    ("asin", &[&[Arg::Gen]]),
    ("acos", &[&[Arg::Gen]]),
    ("atan", &[&[Arg::Gen], &[Arg::Gen, Arg::Gen]]),
    /* 8.2 exponential */
    ("pow", &[&[Arg::Gen, Arg::Gen]]),
    ("exp", &[&[Arg::Gen]]),
    ("log", &[&[Arg::Gen]]),
    ("exp2", &[&[Arg::Gen]]),
    ("log2", &[&[Arg::Gen]]),
    ("sqrt", &[&[Arg::Gen]]),
    ("inversesqrt", &[&[Arg::Gen]]),
    /* 8.3 common */
    ("abs", &[&[Arg::Gen]]),
    ("sign", &[&[Arg::Gen]]),
    ("floor", &[&[Arg::Gen]]),
    ("ceil", &[&[Arg::Gen]]),
    ("fract", &[&[Arg::Gen]]),
    ("mod", &[&[Arg::Gen, Arg::Float], &[Arg::Gen, Arg::Gen]]),
    ("min", &[&[Arg::Gen, Arg::Gen], &[Arg::Gen, Arg::Float]]),
    ("max", &[&[Arg::Gen, Arg::Gen], &[Arg::Gen, Arg::Float]]),
    ("clamp", &[&[Arg::Gen, Arg::Gen, Arg::Gen], &[Arg::Gen, Arg::Float, Arg::Float]]),
    ("mix", &[&[Arg::Gen, Arg::Gen, Arg::Gen], &[Arg::Gen, Arg::Gen, Arg::Float]]),
    ("step", &[&[Arg::Gen, Arg::Gen], &[Arg::Float, Arg::Gen]]),
    ("smoothstep", &[&[Arg::Gen, Arg::Gen, Arg::Gen], &[Arg::Float, Arg::Float, Arg::Gen]]),
    /* 8.4 geometric */
    ("length", &[&[Arg::Gen]]),
    ("distance", &[&[Arg::Gen, Arg::Gen]]),
    ("dot", &[&[Arg::Gen, Arg::Gen]]),
    ("cross", &[&[Arg::Vec3, Arg::Vec3]]),
    ("normalize", &[&[Arg::Gen]]),
    ("faceforward", &[&[Arg::Gen, Arg::Gen, Arg::Gen]]),
    ("reflect", &[&[Arg::Gen, Arg::Gen]]),
    ("refract", &[&[Arg::Gen, Arg::Gen, Arg::Float]]),
    /* 8.5 matrix */
    ("matrixCompMult", &[&[Arg::Mat, Arg::Mat]]),
    /* 8.6 vector relational */
    ("lessThan", &[&[Arg::Vec, Arg::Vec], &[Arg::IVec, Arg::IVec]]),
    ("lessThanEqual", &[&[Arg::Vec, Arg::Vec], &[Arg::IVec, Arg::IVec]]),
    ("greaterThan", &[&[Arg::Vec, Arg::Vec], &[Arg::IVec, Arg::IVec]]),
    ("greaterThanEqual", &[&[Arg::Vec, Arg::Vec], &[Arg::IVec, Arg::IVec]]),
    ("equal", &[&[Arg::Vec, Arg::Vec], &[Arg::IVec, Arg::IVec], &[Arg::BVec, Arg::BVec]]),
    ("notEqual", &[&[Arg::Vec, Arg::Vec], &[Arg::IVec, Arg::IVec], &[Arg::BVec, Arg::BVec]]),
    ("any", &[&[Arg::BVec]]),
    ("all", &[&[Arg::BVec]]),
    ("not", &[&[Arg::BVec]]),
    /* 8.7 texture lookup */
    ("texture2D", &[&[Arg::Sampler2D, Arg::Vec2], &[Arg::Sampler2D, Arg::Vec2, Arg::Float]]),
    (
        "texture2DProj",
        &[
            &[Arg::Sampler2D, Arg::Vec3],
            &[Arg::Sampler2D, Arg::Vec3, Arg::Float],
            &[Arg::Sampler2D, Arg::Vec4],
            &[Arg::Sampler2D, Arg::Vec4, Arg::Float],
        ],
    ),
    ("texture2DLod", &[&[Arg::Sampler2D, Arg::Vec2, Arg::Float]]),
    ("texture2DProjLod", &[&[Arg::Sampler2D, Arg::Vec3, Arg::Float], &[Arg::Sampler2D, Arg::Vec4, Arg::Float]]),
    ("textureCube", &[&[Arg::SamplerCube, Arg::Vec3], &[Arg::SamplerCube, Arg::Vec3, Arg::Float]]),
    ("textureCubeLod", &[&[Arg::SamplerCube, Arg::Vec3, Arg::Float]]),
];

/// Whether a built-in of this name already has this exact parameter list.
///
/// An ARRAY parameter can never match: no built-in takes one, so a
/// declaration with one is an overload however its base types read.
fn builtin_has_signature(name: &str, params: &[Param<'_>]) -> bool {
    if params.iter().any(|p| p.ty.array_len.is_some()) {
        return false;
    }
    let Some((_, sigs)) = BUILTIN_SIGS.iter().find(|(n, _)| *n == name) else { return false };
    sigs.iter().any(|sig| signature_matches(sig, params))
}

/// One prototype against one parameter list, with the family width threaded
/// through so that every family slot agrees.
fn signature_matches(sig: &[Arg], params: &[Param<'_>]) -> bool {
    if sig.len() != params.len() {
        return false;
    }
    let mut width: Option<u8> = None;
    for (a, p) in sig.iter().zip(params) {
        /* `want` is the width this slot fixes for the whole list, or None
         * when the slot's type carries no family width. */
        let want = match (a, p.ty.base) {
            (Arg::Gen, BaseType::Float) => Some(1),
            (Arg::Gen, BaseType::Vec(n)) => Some(n),
            (Arg::Vec, BaseType::Vec(n)) => Some(n),
            (Arg::IVec, BaseType::IVec(n)) => Some(n),
            (Arg::BVec, BaseType::BVec(n)) => Some(n),
            (Arg::Mat, BaseType::Mat(n, r)) if n == r => Some(n),
            (Arg::Float, BaseType::Float) => None,
            (Arg::Vec2, BaseType::Vec(2)) => None,
            (Arg::Vec3, BaseType::Vec(3)) => None,
            (Arg::Vec4, BaseType::Vec(4)) => None,
            (Arg::Sampler2D, BaseType::SAMPLER_2D) => None,
            (Arg::SamplerCube, BaseType::SAMPLER_CUBE) => None,
            _ => return false,
        };
        if let Some(w) = want {
            if *width.get_or_insert(w) != w {
                return false;
            }
        }
    }
    true
}

/// The SIZE a parameter's array declaration states, or `None` when the
/// parameter is not an array.
///
/// A size the checker cannot fold answers `Some(0)` rather than `None`: it
/// is still an array, and two unfoldable sizes then compare equal, which
/// leaves the old behaviour for a declaration that is already wrong for
/// another reason (section 6.1 requires the size to be there at all).
fn param_array_len(ast: &Ast<'_>, p: &Param<'_>) -> Option<i64> {
    p.ty.array_len.map(|e| super::reflect::const_int_expr(ast, e).unwrap_or(0))
}

fn same_signature(ast: &Ast<'_>, a: &[Param<'_>], b: &[Param<'_>]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| {
            /* Section 6.1, on what makes two declarations the same
             * function: "This includes exact match of array size as well."
             * So `float func(float f[3])` and `float func(float f[4])` are
             * two OVERLOADS, not one signature declared twice -- dEQP's
             * `overloading.array_size` is exactly that and must build.
             * Comparing array-NESS instead of the size called them the same
             * signature and reported the second as already defined. */
            p.ty.base == q.ty.base && param_array_len(ast, p) == param_array_len(ast, q)
        })
}

/// The user-defined function a call names, resolved against the argument
/// types.
///
/// ES 1.00 allows a user function to be overloaded on its parameter types
/// (section 6.1), and there is no implicit conversion, so a match is exact.
/// Taking the first declaration of the name instead is wrong in a way that
/// does not look wrong: dEQP's `overloading.user_func_arg_*` shaders declare
/// `float f(float)` beside `vec3 f(vec3)`, and reading the first one's return
/// type made `f(v).xyz` a swizzle of a float and an `if (f(b))` a condition
/// that is not a bool. Ten must-build cases were rejected for reasons that
/// were consequences of the wrong signature rather than anything in the
/// shader.
///
/// With one candidate the match is not required: a valid shader's only
/// function of that name is the one being called, and demanding an exact
/// match would refuse a call whose argument the checker could not type. With
/// several, an unambiguous exact match or nothing -- guessing between
/// overloads is how a shader silently gets the wrong body.
///
/// `defined_only` asks for a definition rather than a prototype, which is
/// what the lowering needs and the checker does not.
pub fn resolve_overload<'a>(ast: &Ast<'a>, name: &str, args: &[Type<'a>], defined_only: bool) -> Option<DeclId> {
    let is_candidate = |d: &Decl<'a>| match d {
        Decl::Function { name: n, body, .. } => *n == name && (!defined_only || body.is_some()),
        _ => false,
    };
    let mut only = None;
    let mut n = 0usize;
    for (i, d) in ast.decls.iter().enumerate() {
        /* The unit's own top-level declarations; a nested one cannot be a
         * function in ES 1.00. */
        if ast.top.iter().any(|t| t.0 as usize == i) && is_candidate(d) {
            n += 1;
            only = Some(DeclId(i as u32));
        }
    }
    if n == 0 {
        return None;
    }
    if n == 1 {
        return only;
    }
    if args.iter().any(|t| t.is_unknown()) {
        return None;
    }
    let (mut def, mut proto) = (None, None);
    let (mut defs, mut protos) = (0usize, 0usize);
    for (i, d) in ast.decls.iter().enumerate() {
        if !ast.top.iter().any(|t| t.0 as usize == i) || !is_candidate(d) {
            continue;
        }
        let Decl::Function { params, body, .. } = d else { continue };
        let ps = &ast.params[params.indices()];
        if ps.len() != args.len() {
            continue;
        }
        /* The same section 6.1 sentence on the CALL side: an argument
         * picks the declaration whose array size it matches exactly. */
        let exact = ps.iter().zip(args).all(|(p, a)| {
            let arg_len = a.is_array().then_some(a.array_len as i64);
            p.ty.base == a.base && param_array_len(ast, p) == arg_len
        });
        if !exact {
            continue;
        }
        /* A second exact match is NOT a second overload.
         *
         * Two candidates that match the same argument types have the same
         * parameter types, so they are the same SIGNATURE. Section 6.1 lets
         * one signature have a prototype AND a definition -- that is how a
         * function is called before it is defined -- but only one of each:
         *
         *     float func();                     prototype  |  legal
         *     void main(){ ... func() ... }                |  together
         *     float func(void){ return -1.0; }  definition |
         *
         * Counting them as two overloads and giving up left the call
         * UNTYPED, and the lowering then refused every operator applied to
         * it ("the type of this expression is not known"). Measured:
         * defining it before use lowered, and so did calling it with no
         * operator, which is what named the cause.
         *
         * Two prototypes, or two definitions, is a redeclaration and the
         * shader is wrong; `None` keeps it refused. That case is worth its
         * own arm rather than falling out of the ordering: making the
         * prototype work cost dEQP's `invalid.double_declare` and
         * `invalid.redeclare_function` -- four must-fail cases we began
         * accepting -- until the counts below replaced "prefer the body". */
        if body.is_some() {
            defs += 1;
            def = Some(DeclId(i as u32));
        } else {
            protos += 1;
            proto = Some(DeclId(i as u32));
        }
    }
    /* ES 3.00 lifts the prototype half: its section 6.1 lets a function be
     * declared any number of times, and dEQP-GLES3's functions.double_declare
     * and scoping.redeclare_function must BUILD where the ES 1.00 cases of
     * the same names must fail. Two definitions stay an error in both. */
    if defs > 1 || (protos > 1 && !ast.es3) {
        return None;
    }
    /* The definition when there is one, so a caller that needs a body gets
     * it; the prototype is enough to answer a type. */
    def.or(proto)
}

/// What a built-in function of section 8 returns.
///
/// Only the *shape* of the result, never whether the call is well formed:
/// this decides a type, and a wrong argument is the checker's other rules'
/// business. A built-in whose result type is guessed wrongly would make
/// `mat4 * v` and `float * v` indistinguishable in the lowering, which is why
/// this exists at all -- before it, every expression containing a `min` or a
/// `texture2D` was untyped, and 32 of the GC7000 corpus's sources stopped on
/// the multiply that followed one.
#[derive(Clone, Copy)]
enum Returns {
    /// The widest of its arguments: the `genType` family, where the result is
    /// as wide as the vector operand and a `float` argument rides along.
    /// `min(v3, f)` is a `vec3`; so is `step(f, v3)`, where the wide one is
    /// the second argument, and `smoothstep(f, f, v3)`, where it is the
    /// third. Taking the widest rather than a fixed position covers all
    /// three without three rules.
    Widest,
    Float,
    Vec3,
    Vec4,
    /// A `bvec` as wide as the arguments: the relational built-ins.
    BoolVec,
    Bool,
}

/// Section 8's library, by what it returns.
const BUILTINS: &[(&str, Returns)] = &[
    /* 8.1 angle and trigonometry, 8.2 exponential, 8.3 common: all genType */
    ("radians", Returns::Widest),
    ("degrees", Returns::Widest),
    ("sin", Returns::Widest),
    ("cos", Returns::Widest),
    ("tan", Returns::Widest),
    ("asin", Returns::Widest),
    ("acos", Returns::Widest),
    ("atan", Returns::Widest),
    ("pow", Returns::Widest),
    ("exp", Returns::Widest),
    ("log", Returns::Widest),
    ("exp2", Returns::Widest),
    ("log2", Returns::Widest),
    ("sqrt", Returns::Widest),
    ("inversesqrt", Returns::Widest),
    ("abs", Returns::Widest),
    ("sign", Returns::Widest),
    ("floor", Returns::Widest),
    ("ceil", Returns::Widest),
    ("fract", Returns::Widest),
    ("mod", Returns::Widest),
    ("min", Returns::Widest),
    ("max", Returns::Widest),
    ("clamp", Returns::Widest),
    ("mix", Returns::Widest),
    ("step", Returns::Widest),
    ("smoothstep", Returns::Widest),
    /* 8.4 geometric */
    ("length", Returns::Float),
    ("distance", Returns::Float),
    ("dot", Returns::Float),
    ("cross", Returns::Vec3),
    ("normalize", Returns::Widest),
    ("faceforward", Returns::Widest),
    ("reflect", Returns::Widest),
    ("refract", Returns::Widest),
    /* 8.5 matrix */
    ("matrixCompMult", Returns::Widest),
    /* 8.6 vector relational */
    ("lessThan", Returns::BoolVec),
    ("lessThanEqual", Returns::BoolVec),
    ("greaterThan", Returns::BoolVec),
    ("greaterThanEqual", Returns::BoolVec),
    ("equal", Returns::BoolVec),
    ("notEqual", Returns::BoolVec),
    ("any", Returns::Bool),
    ("all", Returns::Bool),
    ("not", Returns::BoolVec),
    /* GL_OES_standard_derivatives, which is an extension rather than
     * section 8 -- but the shaders that use it guard it with `#extension`,
     * and refusing to TYPE it would refuse the shader for the wrong reason.
     * Whether the extension is advertised is `gl::state`'s answer and is
     * checked where the directive is read, not here. */
    ("dFdx", Returns::Widest),
    ("dFdy", Returns::Widest),
    ("fwidth", Returns::Widest),
    /* 8.7 texture: every one of them samples four channels */
    ("texture2D", Returns::Vec4),
    ("texture2DProj", Returns::Vec4),
    ("texture2DLod", Returns::Vec4),
    ("texture2DProjLod", Returns::Vec4),
    ("textureCube", Returns::Vec4),
    ("textureCubeLod", Returns::Vec4),
];

/// ES 3.00 section 8.6's matrix functions that ES 1.00 does not have:
/// `transpose`, `inverse`, `determinant` and `outerProduct`. `None`: not one
/// of them. (The non-square forms come with the non-square types.)
fn matrix_builtin_es3<'a>(name: &'a str, tys: &[Type<'a>]) -> Option<R<'a, Val<'a>>> {
    let bad = || Err(Error::new(0, name, "no form of this matrix function takes these arguments (section 8.6)"));
    let one_matrix = || match tys {
        [t] if !t.is_array() => match t.base {
            BaseType::Mat(c, r) => Some((c, r)),
            _ => None,
        },
        _ => None,
    };
    Some(match name {
        /* transpose(matCxR) is a matRxC. */
        "transpose" => match one_matrix() {
            Some((c, r)) => Ok(Val::rvalue(Type::scalar(BaseType::Mat(r, c)))),
            None if tys.iter().any(|t| t.is_unknown()) => Ok(Val::unknown()),
            None => bad(),
        },
        /* inverse and determinant are square-only (section 8.6). */
        "inverse" => match one_matrix() {
            Some((c, r)) if c == r => Ok(Val::rvalue(Type::scalar(BaseType::Mat(c, r)))),
            None if tys.iter().any(|t| t.is_unknown()) => Ok(Val::unknown()),
            _ => bad(),
        },
        "determinant" => match one_matrix() {
            Some((c, r)) if c == r => Ok(Val::rvalue(Type::scalar(BaseType::Float))),
            None if tys.iter().any(|t| t.is_unknown()) => Ok(Val::unknown()),
            _ => bad(),
        },
        /* outerProduct(c, r): c as a column, r as a row -- as many columns
         * as r has components, as many rows as c has. */
        "outerProduct" => match tys {
            [a, b] if a.is_unknown() || b.is_unknown() => Ok(Val::unknown()),
            [a, b] if !a.is_array() && !b.is_array() => match (a.base, b.base) {
                (BaseType::Vec(rows), BaseType::Vec(cols)) => Ok(Val::rvalue(Type::scalar(BaseType::Mat(cols, rows)))),
                _ => bad(),
            },
            _ => bad(),
        },
        _ => return None,
    })
}

/// The section 8.3 common functions ES 3.00 adds (ES 1.00 has none of these
/// names, so an ES 1.00 source may use them as its own): `trunc`, `round`,
/// `roundEven` of a float genType; `isnan` and `isinf` giving a bool of
/// its width; and the bit casts between a float genType and an int or uint
/// one. `None`: not one of them.
pub const COMMON_ES3: &[&str] = &[
    /* section 8.1's hyperbolic functions, the same shape */
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "trunc",
    "round",
    "roundEven",
    "isnan",
    "isinf",
    "floatBitsToInt",
    "floatBitsToUint",
    "intBitsToFloat",
    "uintBitsToFloat",
    /* section 8.4's floating-point pack and unpack functions */
    "packSnorm2x16",
    "unpackSnorm2x16",
    "packUnorm2x16",
    "unpackUnorm2x16",
    "packHalf2x16",
    "unpackHalf2x16",
];

fn common_builtin_es3<'a>(name: &'a str, tys: &[Type<'a>]) -> Option<R<'a, Val<'a>>> {
    if !COMMON_ES3.contains(&name) {
        return None;
    }
    let bad = || Err(Error::new(0, name, "no form of this function takes these arguments (section 8.3)"));
    let [t] = tys else { return Some(bad()) };
    if t.is_unknown() {
        return Some(Ok(Val::unknown()));
    }
    if t.is_array() {
        return Some(bad());
    }
    /* pack: a vec2 to a uint; unpack: a uint to a vec2 */
    if name.starts_with("pack") {
        return Some(if t.base == BaseType::Vec(2) { Ok(Val::rvalue(Type::scalar(BaseType::Uint))) } else { bad() });
    }
    if name.starts_with("unpack") {
        return Some(if t.base == BaseType::Uint { Ok(Val::rvalue(Type::scalar(BaseType::Vec(2)))) } else { bad() });
    }
    let width = |b: BaseType<'_>, kind: ScalarKind| match (b, kind) {
        (BaseType::Float, ScalarKind::Float)
        | (BaseType::Int, ScalarKind::Int)
        | (BaseType::Uint, ScalarKind::Uint) => Some(1),
        (BaseType::Vec(n), ScalarKind::Float)
        | (BaseType::IVec(n), ScalarKind::Int)
        | (BaseType::UVec(n), ScalarKind::Uint) => Some(n),
        _ => None,
    };
    let (from, to) = match name {
        "trunc" | "round" | "roundEven" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" => {
            (ScalarKind::Float, Some(ScalarKind::Float))
        }
        "isnan" | "isinf" => (ScalarKind::Float, None),
        "floatBitsToInt" => (ScalarKind::Float, Some(ScalarKind::Int)),
        "floatBitsToUint" => (ScalarKind::Float, Some(ScalarKind::Uint)),
        "intBitsToFloat" => (ScalarKind::Int, Some(ScalarKind::Float)),
        _ => (ScalarKind::Uint, Some(ScalarKind::Float)),
    };
    let Some(n) = width(t.base, from) else { return Some(bad()) };
    Some(Ok(Val::rvalue(Type::scalar(match to {
        Some(k) => vector_of(k, n),
        None if n == 1 => BaseType::Bool,
        None => BaseType::BVec(n),
    }))))
}

/// ES 1.00's texture look-ups (section 8.7), which ES 3.00 does not have.
const ES100_TEXTURES: &[&str] =
    &["texture2D", "texture2DProj", "texture2DLod", "texture2DProjLod", "textureCube", "textureCubeLod"];

/// ES 3.00's texture look-ups (section 8.8 of that specification).
pub const ES300_TEXTURES: &[&str] = &[
    "texture",
    "textureProj",
    "textureLod",
    "textureOffset",
    "texelFetch",
    "texelFetchOffset",
    "textureProjOffset",
    "textureLodOffset",
    "textureProjLod",
    "textureProjLodOffset",
    "textureGrad",
    "textureGradOffset",
    "textureProjGrad",
    "textureProjGradOffset",
    "textureSize",
];

/// The type a call to a built-in produces, or `None` if the name is not one.
fn builtin_return<'a>(name: &str, args: &[Type<'a>]) -> Option<Type<'a>> {
    let (_, shape) = BUILTINS.iter().find(|(n, _)| *n == name)?;
    /* An argument the checker could not type makes the result untypeable
     * too, rather than a guess that happens to be a float. */
    if args.iter().any(|t| t.is_unknown()) {
        return None;
    }
    let widest = args.iter().map(|t| t.base.components().max(1)).max().unwrap_or(1);
    Some(Type::scalar(match shape {
        Returns::Widest => {
            let base = args.iter().max_by_key(|t| t.base.components())?.base;
            /* A matrix argument keeps its own shape (`matrixCompMult`); a
             * vector widens to its own width; a scalar stays scalar. */
            /* The scalar kind is the first argument's: ES 3.00 gives `abs`,
             * `sign`, `min`, `max` and `clamp` integer forms, and `mix`'s
             * third argument may be a bvec that must not decide the
             * result. */
            let kind = match args.first().and_then(|t| scalar_of(&t.base)) {
                Some(ScalarKind::Int) => ScalarKind::Int,
                Some(ScalarKind::Uint) => ScalarKind::Uint,
                _ => ScalarKind::Float,
            };
            match base {
                BaseType::Mat(c, r) => BaseType::Mat(c, r),
                _ => vector_of(kind, widest),
            }
        }
        Returns::Float => BaseType::Float,
        Returns::Vec3 => BaseType::Vec(3),
        Returns::Vec4 => BaseType::Vec(4),
        Returns::BoolVec if widest > 1 => BaseType::BVec(widest),
        Returns::BoolVec | Returns::Bool => BaseType::Bool,
    }))
}

/// The type a constructor name builds, or `None` if the name is not one.
///
/// `es3`: the ES 3.00 type names are constructors only in an ES 3.00 source;
/// in ES 1.00 `uint` is an ordinary identifier.
pub fn constructor_type(name: &str, es3: bool) -> Option<BaseType<'static>> {
    if es3 {
        match name {
            "uint" => return Some(BaseType::Uint),
            "uvec2" => return Some(BaseType::UVec(2)),
            "uvec3" => return Some(BaseType::UVec(3)),
            "uvec4" => return Some(BaseType::UVec(4)),
            "mat2x2" => return Some(BaseType::Mat(2, 2)),
            "mat3x3" => return Some(BaseType::Mat(3, 3)),
            "mat4x4" => return Some(BaseType::Mat(4, 4)),
            "mat2x3" => return Some(BaseType::Mat(2, 3)),
            "mat2x4" => return Some(BaseType::Mat(2, 4)),
            "mat3x2" => return Some(BaseType::Mat(3, 2)),
            "mat3x4" => return Some(BaseType::Mat(3, 4)),
            "mat4x2" => return Some(BaseType::Mat(4, 2)),
            "mat4x3" => return Some(BaseType::Mat(4, 3)),
            _ => {}
        }
    }
    Some(match name {
        "float" => BaseType::Float,
        "int" => BaseType::Int,
        "bool" => BaseType::Bool,
        "vec2" => BaseType::Vec(2),
        "vec3" => BaseType::Vec(3),
        "vec4" => BaseType::Vec(4),
        "ivec2" => BaseType::IVec(2),
        "ivec3" => BaseType::IVec(3),
        "ivec4" => BaseType::IVec(4),
        "bvec2" => BaseType::BVec(2),
        "bvec3" => BaseType::BVec(3),
        "bvec4" => BaseType::BVec(4),
        "mat2" => BaseType::Mat(2, 2),
        "mat3" => BaseType::Mat(3, 3),
        "mat4" => BaseType::Mat(4, 4),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::{parser, pp};

    fn check_src<'a>(buf: &'a mut [u8], src: &'a str, stage: Stage) -> R<'a, ExprTypes<'a>> {
        let a = Arena::new(buf);
        let pre = pp::preprocess(&a, src)?;
        let ast = parser::parse(&a, pre.tokens)?;
        check(&a, &ast, stage)
    }

    /// A fragment snippet with the default float precision in front of it.
    ///
    /// Section 4.5.3 makes a fragment shader that declares a float without
    /// one invalid, so every snippet below would otherwise be testing its own
    /// rule AND tripping that one -- and `fails` asserts a specific message,
    /// which the precision error would pre-empt. The sources here are minimal
    /// on purpose; carrying the declaration in each would say nothing about
    /// the rule each test is for. `float_precision_tests` uses its own
    /// helper and sees the raw source.
    ///
    /// [`with_default_precision`] does it.
    fn ok_stage(src: &str, stage: Stage) {
        let mut buf = [0u8; 1 << 18];
        let mut owned = [0u8; 4096];
        let src = with_default_precision(&mut owned, src, stage);
        if let Err(e) = check_src(&mut buf, src, stage) {
            panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg);
        }
    }

    /// Prepend `precision mediump float;` to a FRAGMENT source that does not
    /// already set a default. See [`es100`] for why.
    fn with_default_precision<'b>(buf: &'b mut [u8], src: &'b str, stage: Stage) -> &'b str {
        if stage != Stage::Fragment || src.contains("precision") {
            return src;
        }
        const HEAD: &str = "precision mediump float;\n";
        let n = HEAD.len() + src.len();
        assert!(n <= buf.len(), "test source too long for the precision prefix");
        buf[..HEAD.len()].copy_from_slice(HEAD.as_bytes());
        buf[HEAD.len()..n].copy_from_slice(src.as_bytes());
        core::str::from_utf8(&buf[..n]).expect("the pieces were both utf-8")
    }

    fn ok(src: &str) {
        ok_stage(src, Stage::Fragment);
    }

    fn ok_vs(src: &str) {
        ok_stage(src, Stage::Vertex);
    }

    /// [`fails`] for a source that has to be judged in a NAMED stage --
    /// the section 8.7 texture rules are the reason it exists, since each of
    /// them is about the stage and not about the shader.
    fn fails_stage(src: &str, stage: Stage) -> &'static str {
        let mut buf = [0u8; 1 << 18];
        let mut owned = [0u8; 4096];
        let src = with_default_precision(&mut owned, src, stage);
        match check_src(&mut buf, src, stage) {
            Ok(_) => panic!("expected a compile failure in {stage:?}: {src:?}"),
            Err(e) => e.msg,
        }
    }

    /// Section 4.1.7. Revert check: with `carries_sampler` answering
    /// `false`, all five of the refused shaders compile.
    #[test]
    fn a_sampler_is_never_written_and_never_an_out_parameter() {
        let param = "a sampler, or a struct holding one, cannot be an out or inout parameter";
        let assign = "a sampler, or a struct holding one, cannot be assigned to";
        assert_eq!(fails("uniform sampler2D t; void f(out sampler2D s) {} void main() { f(t); }"), param);
        assert_eq!(fails("uniform sampler2D t; void f(inout sampler2D s) {} void main() { f(t); }"), param);
        assert_eq!(
            fails("struct S { float a; sampler2D x; }; uniform S t; void f(out S s); void main() { f(t); }"),
            param
        );
        assert_eq!(
            fails("uniform sampler2D t; uniform sampler2D u; void f(sampler2D s) { s = u; } void main() { f(t); }"),
            assign
        );
        assert_eq!(
            fails(
                "struct I { sampler2D x; }; struct S { float y; I i; }; uniform S t; uniform S u;\n\
                 void f(S s) { s = u; } void main() { f(t); }"
            ),
            assign
        );
        /* The member is a float: assigning it is not assigning the struct. */
        ok("struct S { sampler2D x; float y; }; uniform S t;\n\
            void f(S s) { s.y = 1.0; } void main() { f(t); }");
        ok("uniform sampler2D t; float f(in sampler2D s) { return 1.0; } void main() { f(t); }");
    }

    fn fail_err(src: &str) -> Error<'static> {
        let mut buf = [0u8; 1 << 18];
        let mut owned = [0u8; 4096];
        let src = with_default_precision(&mut owned, src, Stage::Fragment);
        match check_src(&mut buf, src, Stage::Fragment) {
            Ok(_) => panic!("expected a compile failure: {src:?}"),
            Err(e) => Error::new(e.line, if e.tok.is_empty() { "" } else { leak(e.tok) }, e.msg),
        }
    }

    fn leak(s: &str) -> &'static str {
        std::boxed::Box::leak(std::string::String::from(s).into_boxed_str())
    }

    fn fails(src: &str) -> &'static str {
        let mut buf = [0u8; 1 << 18];
        let mut owned = [0u8; 4096];
        let src = with_default_precision(&mut owned, src, Stage::Fragment);
        match check_src(&mut buf, src, Stage::Fragment) {
            Ok(_) => panic!("expected a compile failure: {src:?}"),
            Err(e) => e.msg,
        }
    }

    /// Section 4.1.10, and the rule this module exists for: the shipping
    /// pipeline cannot enforce it, because it judges the shader as GLSL 450,
    /// which has the conversions.
    #[test]
    fn there_are_no_implicit_conversions_in_either_direction() {
        assert_eq!(fails("void main() { int i = 1.5; }"), "no implicit conversion in GLSL ES 1.00");
        assert_eq!(fails("void main() { float f = 1; }"), "no implicit conversion in GLSL ES 1.00");
        assert_eq!(fails("void main() { float f = 1.0 + 1; }"), "no implicit conversion in GLSL ES 1.00");
        assert_eq!(fails("void main() { bool b = 1.0 < 1; }"), "no implicit conversion in GLSL ES 1.00");
        /* A constructor converts explicitly, so these are fine. */
        ok("void main() { int i = int(1.5); float f = float(1); vec2 v = vec2(1, 2.0); }");
    }

    #[test]
    fn a_read_only_variable_cannot_be_assigned() {
        assert_eq!(fails("uniform vec4 u;\nvoid main() { u = vec4(1.0); }"), "cannot assign to this expression");
        assert_eq!(fails("const float K = 1.0;\nvoid main() { K = 2.0; }"), "cannot assign to this expression");
        assert_eq!(
            fails("varying vec2 v;\nvoid main() { v = vec2(1.0); }"),
            "cannot assign to this expression",
            "a varying is read-only in the fragment shader"
        );
        assert_eq!(fails("void main() { gl_FragCoord = vec4(1.0); }"), "cannot assign to this expression");
        /* The vertex shader writes its varyings. */
        ok_vs("varying vec2 v;\nvoid main() { v = vec2(1.0); gl_Position = vec4(1.0); }");
    }

    #[test]
    fn swizzles_follow_section_5_5() {
        ok("varying vec4 v;\nvoid main() { float f = v.x; vec2 a = v.rg; vec3 b = v.stp; gl_FragColor = v.xyzw; }");
        assert_eq!(fails("varying vec2 v;\nvoid main() { float f = v.z; }"), "component out of range for this type");
        assert_eq!(fails("varying vec4 v;\nvoid main() { vec2 a = v.xr; }"), "a swizzle cannot mix component sets");
        assert_eq!(fails("varying vec4 v;\nvoid main() { float f = v.q1; }"), "not a component name");
        assert_eq!(
            fails("varying vec4 v;\nvoid main() { float f = v.xyzwx; }"),
            "a swizzle names one to four components"
        );
        /* A repeated component reads but does not assign (section 5.8). */
        ok("varying vec4 v;\nvoid main() { vec2 a = v.xx; }");
        assert_eq!(fails("void main() { vec4 c; c.xx = vec2(1.0); }"), "cannot assign to this expression");
    }

    #[test]
    fn the_reserved_operators_are_refused_by_name() {
        assert_eq!(fails("void main() { int a = 1; int b = a % 2; }"), "reserved in GLSL ES 1.00");
        assert_eq!(fails("void main() { int a = 1; int b = a << 2; }"), "reserved in GLSL ES 1.00");
        assert_eq!(fails("void main() { int a = 1; int b = ~a; }"), "reserved in GLSL ES 1.00");
    }

    /// Section 5.8 reserves the compound forms too, and the refusal names
    /// the operator that was written. Revert check: without the arm in
    /// `assign`, every `op=` line compiles (the lowering refused them
    /// later, as a gap).
    /// Section 7.5: `gl_DepthRange` is a built-in uniform in both stages,
    /// with three float members, and it is read-only. It used to be an
    /// "undeclared identifier": a verdict on dEQP's two legal
    /// builtin_variable.depth_range shaders.
    ///
    /// Revert check: without the `add` for it, the first line fails with
    /// "undeclared identifier".
    #[test]
    fn gl_depthrange_is_a_builtin_uniform() {
        ok("void main() { gl_FragColor = vec4(gl_DepthRange.near, gl_DepthRange.far, gl_DepthRange.diff, 1.0); }");
        ok_vs("void main() { gl_Position = vec4(gl_DepthRange.diff); }");
        assert_eq!(fails("void main() { gl_FragColor = vec4(gl_DepthRange.depth); }"), "no such member");
        assert_eq!(
            fails("void main() { gl_DepthRange.near = 1.0; gl_FragColor = vec4(0.0); }"),
            "cannot assign to this expression"
        );
        assert_eq!(
            fails("void main() { vec2 v = gl_DepthRange.near; gl_FragColor = vec4(0.0); }"),
            "the types do not match"
        );
    }

    #[test]
    fn the_reserved_assignments_are_refused_by_name() {
        for op in ["%=", "&=", "|=", "^=", "<<=", ">>="] {
            let e = fail_err(&format!("void main() {{ int a = 1; a {op} 2; }}"));
            assert_eq!((e.tok, e.msg), (op, "reserved in GLSL ES 1.00"));
        }
        let e = fail_err("void main() { int a = 1; int b = a >> 2; }");
        assert_eq!(e.tok, ">>", "the binary form names its own operator");
        ok("void main() { int a = 1; a += 2; a -= 1; a *= 3; a /= 2; }");
    }

    #[test]
    fn qualifiers_are_checked_against_the_stage_and_the_type() {
        assert_eq!(fails("attribute vec4 a;\nvoid main() {}"), "'attribute' is not allowed in a fragment shader");
        ok_vs("attribute vec4 a;\nvoid main() { gl_Position = a; }");
        assert_eq!(fails("uniform vec4 u = vec4(1.0);\nvoid main() {}"), "a uniform cannot have an initializer");
        assert_eq!(fails("void main() { sampler2D s; }"), "a sampler must be a uniform");
        assert_eq!(fails("varying int v;\nvoid main() {}"), "a varying must be a float, vector or matrix");
    }

    #[test]
    fn a_shader_must_define_main() {
        assert_eq!(fails("uniform vec4 u;\n"), "a shader must define 'void main()'");
        assert_eq!(fails("void main();\n"), "a shader must define 'void main()'", "a prototype is not a definition");
        assert_eq!(fails("int main() { return 0; }"), "'main' must return void");
    }

    #[test]
    fn discard_is_fragment_only() {
        ok("void main() { discard; }");
        let mut buf = [0u8; 1 << 18];
        let Err(e) = check_src(&mut buf, "void main() { discard; }", Stage::Vertex) else {
            panic!("discard is not allowed in a vertex shader");
        };
        assert_eq!(e.msg, "'discard' is only allowed in a fragment shader");
    }

    /// Section 4.2.2: the body of a `for` or a `while` is
    /// `statement_no_new_scope`, so it shares the scope holding the loop's
    /// own declaration.
    ///
    /// dEQP `scoping.invalid.redeclare_for_init_statement_variable`. The
    /// three neighbouring cases that ask the same question of a loop
    /// CONDITION were already refused, but for the wrong reason -- each
    /// writes `int a = (i < 10)`, which is section 4.1.10 -- so this is the
    /// only one that actually turns the scope rule on.
    #[test]
    fn a_loop_body_shares_the_scope_of_the_loops_own_declaration() {
        assert_eq!(
            fails("void main() { for (int i = 0; i < 10; i++) { int i = 11; } }"),
            "redeclared in the same scope"
        );
        assert_eq!(
            fails("void main() { int a = 0; while (bool b = (a < 5)) { bool b = true; a++; } }"),
            "redeclared in the same scope",
            "a while body shares the scope of a declaration in the while TEST"
        );
        /* But a loop body still SHADOWS the scope outside the loop: the
         * scope it shares is the loop's own, which the enclosing `int a` is
         * not in. Asserting a failure here is what this test did first, and
         * it was wrong. */
        ok("void main() { int a = 0; while (a < 5) { int a = 1; a = 2; } }");
        /* Still a NESTED scope with respect to everything further out, and a
         * block written inside the body opens one of its own. */
        ok("void main() { float x = 1.0; for (int i = 0; i < 10; i++) { float x = 2.0; x = 3.0; } x = 4.0; }");
        ok("void main() { for (int i = 0; i < 10; i++) { { int i = 11; } } }");
        /* A do-while body is `statement_with_scope` (the grammar), so it
         * does open one -- this must keep building. */
        ok("void main() { int i = 0; do { int i = 11; i = 12; } while (i < 10); }");
    }

    /// Section 4.2.6: one name space per scope, holding variables,
    /// functions AND structure names; section 4.2.7: a name may be declared
    /// "either as a variable declaration or as function declarations or as a
    /// structure", never as two of those.
    ///
    /// dEQP `scoping.invalid.conflict_function_struct`. Its sibling
    /// `conflict_function_variable` was already refused, because a variable
    /// went into the table and met the function there; a structure name went
    /// nowhere at all, so the two never met.
    #[test]
    fn a_name_is_a_variable_or_a_function_or_a_structure_but_not_two() {
        assert_eq!(
            fails("void f(int x); struct f { int x; }; void main() {}"),
            "already declared as a function, and a name cannot also be a structure"
        );
        assert_eq!(
            fails("struct f { int x; }; void f(int x); void main() {}"),
            "already declared as a function, and a name cannot also be a structure",
            "the collision is the same whichever came first"
        );
        assert_eq!(
            fails("struct S { int x; }; float S; void main() {}"),
            "already declared in this scope as something other than a variable"
        );
        assert_eq!(
            fails("struct S { int x; }; struct S { int y; }; void main() {}"),
            "struct redefined",
            "the parser already refuses this one, before the name space is reached"
        );
        /* A structure name is not a value, but everything a structure is
         * actually FOR must keep building: the type, the constructor, a
         * member, and an unrelated variable beside it. */
        ok("struct S { int x; }; void main() { S s = S(1); int y = s.x; }");
        ok("struct S { int x; }; void main() { S S2 = S(1); S2.x = 2; }");
        /* And a local structure is scoped like any other declaration. */
        ok("void main() { struct S { int x; } ; S s = S(1); s.x = 2; }");
    }

    /// Section 4.2.2: a name declared outside all function definitions has
    /// a scope that "starts from where it is declared and persists to the
    /// end of the compilation unit". So a call written above the callee's
    /// first declaration is out of scope, and a prototype is how a shader
    /// says otherwise.
    ///
    /// dEQP `scoping.invalid.use_function_before_declaration` and
    /// `functions.invalid.call_before_definition` -- one rule, asked in two
    /// files.
    /// Section 6.1: a function must be declared before it is called, and a
    /// name that is not one -- a typo, a variable -- is not callable
    /// (`vec(1.0)` compiled until 2026-09-23).
    #[test]
    fn only_a_declared_function_can_be_called() {
        assert_eq!(fails("void main() { vec4 v = vec(1.0); }"), "no function of this name is declared");
        assert_eq!(
            fails("void main() { float f = 1.0; float g = f(2.0); }"),
            "this name is a variable, not a function"
        );
        ok("float h(float x) { return x; }\nvoid main() { float g = h(2.0) + sin(1.0); }");
    }

    /// Section 5.4.2's component rules, each as the section states it.
    /// `bvec3(bvec2)` compiled until 2026-09-23 because one argument was
    /// accepted whatever its width (dEQP conversions.vector_illegal, 54
    /// cases, refused until then only by a lowering gap -- the right answer
    /// for the wrong reason).
    ///
    /// Revert check: accept any single argument again and the first
    /// assertion fails.
    #[test]
    fn a_constructor_takes_exactly_the_components_it_needs() {
        let body = |e: &str| {
            std::format!("void main() {{ vec4 v = vec4(0.0); ivec2 i = ivec2(0); mat2 m = mat2(1.0); {e}; }}")
        };
        assert_eq!(fails(&body("bvec3 b = bvec3(bvec2(true))")), "not enough arguments for this constructor");
        assert_eq!(fails(&body("vec2 a = vec2(1.0, 2.0, 3.0)")), "too many arguments for this constructor");
        assert_eq!(fails(&body("vec3 a = vec3(v, 1.0)")), "too many arguments for this constructor");
        assert_eq!(
            fails(&body("mat3 a = mat3(m, 1.0)")),
            "a matrix argument to a matrix constructor must be its only argument"
        );
        /* And what the section allows, including the ones a naive count
         * would refuse: a matrix is N*N components, not N. */
        for e in [
            "vec2 a = vec2(v)",
            "vec4 a = vec4(m)",
            "vec3 a = vec3(1.0)",
            "mat3 a = mat3(m)",
            "mat2 a = mat2(v)",
            "vec4 a = vec4(i, 1.0, 2.0)",
            "float f = float(v)",
            "vec3 a = vec3(i, v)",
        ] {
            ok(&body(e));
        }
    }

    /// Section 4.2.2 scopes a structure's name, and so its constructor,
    /// like any other: visible from the end of its struct_specifier, and
    /// hidden by a variable of the same name declared after it.
    ///
    /// dEQP scoping.invalid.use_struct_type_before_declaration (must fail)
    /// and scoping.valid.local_int_variable_hides_struct_type (must build:
    /// the variable is visible only after its initialiser, so the `S` in
    /// `int S = S(in0).val;` is still the constructor).
    ///
    /// Revert check: answer every `struct_members` hit as a constructor again
    /// and the first two assertions fail.
    #[test]
    fn a_structure_constructor_is_scoped_like_its_name() {
        assert_eq!(
            fails("float func(float x) { return S(x).val; }\nstruct S { float val; };\nvoid main() {}"),
            "this structure is used before it is declared"
        );
        assert_eq!(
            fails("struct S { float val; };\nvoid main() { float S = 1.0; float t = S(2.0).val; }"),
            "this name is hidden by a variable, so it is not a constructor here"
        );
        ok("precision mediump float;\nstruct S { float val; };\nvoid main() { float S = S(1.0).val; gl_FragColor = vec4(S); }");
    }

    #[test]
    fn a_function_must_be_declared_before_it_is_called() {
        assert_eq!(
            fails("float func(float x) { return bar(x); } float bar(float x) { return x; } void main() {}"),
            "this function is called before it is declared"
        );
        /* A prototype puts the name in scope, which is what prototypes are
         * for -- the same shader with one added must build. */
        ok("float bar(float x); float func(float x) { return bar(x); }             float bar(float x) { return x; } void main() { gl_FragColor = vec4(func(1.0)); }");
        /* Calling downwards, and calling itself, both stay legal. */
        ok("float bar(float x) { return x; } void main() { gl_FragColor = vec4(bar(1.0)); }");
        /* A name that is not a function of ours is left alone: an
         * unmodelled built-in must not become "called before declared". */
        ok("void main() { gl_FragColor = vec4(sin(1.0)); }");
    }

    /// Section 4.2.6: "Functions (including built-in functions) may not be
    /// redefined. User defined functions may overload built-in functions."
    /// Section 10.10 resolves it in those words and gives `int sin(int x)`
    /// as the legal overload -- so the rule is on the SIGNATURE, and the
    /// must-BUILD half of this test is the more important half.
    ///
    /// dEQP `scoping.invalid.redeclare_builtin` / `redefine_builtin`.
    #[test]
    fn a_builtins_signature_cannot_be_redeclared_but_can_be_overloaded() {
        const REDEFINED: &str = "this signature belongs to a built-in, which cannot be redefined";
        assert_eq!(fails("float sin(float x); void main() {}"), REDEFINED, "a prototype");
        assert_eq!(fails("float sin(float x) { return x + 1.0; } void main() {}"), REDEFINED, "a body");
        assert_eq!(fails("vec3 normalize(vec3 v) { return v; } void main() {}"), REDEFINED, "a vec3 genType");
        assert_eq!(fails("float min(float a, float b) { return a; } void main() {}"), REDEFINED, "a second form");
        assert_eq!(
            fails("vec4 texture2D(sampler2D s, vec2 c) { return vec4(0.0); } void main() {}"),
            REDEFINED,
            "a sampler prototype"
        );

        /* THE OVERLOADS, which must all keep building. Section 10.10's own
         * example first. */
        ok("int sin(int x) { return x; } void main() { gl_FragColor = vec4(float(sin(1))); }");
        ok("float sin(vec2 a, vec2 b) { return a.x + b.x; } void main() {}");
        ok("float length(float a, float b) { return a; } void main() {}");
        /* The family width must AGREE across one prototype: `genType mix`
         * is three of one width, so a mixed one is an overload. */
        ok("vec3 mix(vec3 a, vec2 b, vec3 c) { return a; } void main() {}");
        /* No built-in takes an array, so a declaration with one is always
         * an overload however its base types read. */
        ok("float sin(float x[2]) { return x[0]; } void main() {}");
        /* A name of ours that is not a built-in at all is untouched. */
        ok("float mysin(float x) { return x; } void main() {}");
    }

    /// Section 6.1: "If the return type is not void, any return statements
    /// within the function definition must include a return-expression and
    /// the type of the expression must match the return type. If the return
    /// type of the function is specified as void, any such return statements
    /// must not include a return-expression."
    ///
    /// dEQP `functions.invalid.return_without_value` and
    /// `return_value_from_void_function`.
    #[test]
    fn a_return_agrees_with_the_return_type_of_the_function_it_is_in() {
        assert_eq!(
            fails("float f(float x) { return; } void main() { gl_FragColor = vec4(f(1.0)); }"),
            "this function must return a value"
        );
        assert_eq!(
            fails("void f(float x) { return x; } void main() { f(1.0); }"),
            "a void function cannot return a value"
        );
        /* `main` is void like any other function, which section 6.4 says in
         * so many words ("The function main can use return"). */
        ok("void main() { return; }");
        assert_eq!(fails("void main() { return 1; }"), "a void function cannot return a value");
        /* The TYPE must match too, with no conversion (section 4.1.10). */
        assert_eq!(
            fails("float f(float x) { return 1; } void main() { gl_FragColor = vec4(f(1.0)); }"),
            "no implicit conversion in GLSL ES 1.00"
        );
        /* And the agreeing cases keep building, including a return nested
         * inside a conditional and a plain `return;` in a void function. */
        ok("float f(float x) { if (x > 0.0) { return x; } return 0.0; } \
            void main() { gl_FragColor = vec4(f(1.0)); }");
        ok("void f(float x) { if (x > 0.0) { return; } } void main() { f(1.0); }");
    }

    /// Section 4.3: "Local variables can only use the storage qualifier
    /// const."
    ///
    /// dEQP `functions.invalid.uniform_local`. Its siblings
    /// `attribute_local` and `varying_local` were already refused, by the
    /// same check written as a LIST of two qualifiers -- and `uniform` was
    /// the item missing from the list. The spec states it once, for all of
    /// them, and a rule written that way cannot be short by one.
    #[test]
    fn a_local_variable_may_only_be_qualified_const() {
        const LOCAL: &str = "a local variable may only be qualified 'const'";
        assert_eq!(fails("void f() { uniform float u; } void main() { f(); }"), LOCAL);
        assert_eq!(fails("void f() { attribute float a; } void main() { f(); }"), LOCAL);
        assert_eq!(fails("void f() { varying float v; } void main() { f(); }"), LOCAL);
        /* `const` is the one that is allowed, and no qualifier at all is
         * the ordinary case. */
        ok("void main() { const float c = 1.0; float x = c; gl_FragColor = vec4(x); }");
        ok("void main() { float x = 1.0; gl_FragColor = vec4(x); }");
        /* At GLOBAL scope all three stay legal. */
        ok("uniform float u; void main() { gl_FragColor = vec4(u); }");
        ok("varying float v; void main() { gl_FragColor = vec4(v); }");
    }

    /// Section 6.1: "Arrays are allowed as arguments, but not as the return
    /// type", and "The return type can also be a structure if the structure
    /// does not contain an array."
    ///
    /// dEQP `functions.invalid.return_array_in_struct`.
    #[test]
    fn a_return_type_may_not_be_or_contain_an_array() {
        assert_eq!(
            fails("struct Foo { float f; float arr[2]; }; Foo func() { Foo f; return f; } void main() {}"),
            "a returned structure cannot contain an array"
        );
        /* "Contain", not "have as a member": an array one structure deeper
         * is still inside the returned one. */
        assert_eq!(
            fails(
                "struct Inner { float arr[2]; }; struct Outer { Inner i; }; \
                 Outer func() { Outer o; return o; } void main() {}"
            ),
            "a returned structure cannot contain an array"
        );
        /* A structure return type is legal -- that is the whole point of
         * the sentence -- and so is an array PARAMETER. */
        ok("struct Foo { float f; }; Foo func() { Foo f; f.f = 1.0; return f; } \
            void main() { gl_FragColor = vec4(func().f); }");
        ok("float func(float a[2]) { return a[0]; } \
            void main() { float v[2]; v[0] = 1.0; v[1] = 2.0; gl_FragColor = vec4(func(v)); }");
    }

    /// Section 8.7: "Functions containing the bias parameter are available
    /// only in the fragment shader" and "The built-ins suffixed with 'Lod'
    /// are allowed only in a vertex shader" -- because level of detail is
    /// not computed by fixed functionality for vertex shaders.
    ///
    /// dEQP `invalid_texture_functions`, all four cases. Two were already
    /// refused before this rule existed, but not for this reason; both now
    /// say which sentence they broke.
    #[test]
    fn a_texture_lookup_is_restricted_to_one_stage_by_its_extra_argument() {
        const BIAS: &str = "a texture lookup with a bias is only allowed in a fragment shader";
        const LOD: &str = "a 'Lod' texture lookup is only allowed in a vertex shader";
        assert_eq!(
            fails_stage(
                "uniform sampler2D s; void main() { gl_Position = texture2D(s, vec2(0.0), 1.0); }",
                Stage::Vertex
            ),
            BIAS
        );
        assert_eq!(
            fails_stage(
                "uniform samplerCube s; void main() { gl_Position = textureCube(s, vec3(0.0), 1.0); }",
                Stage::Vertex
            ),
            BIAS
        );
        assert_eq!(
            fails_stage(
                "uniform sampler2D s; void main() { gl_FragColor = texture2DLod(s, vec2(0.0), 1.0); }",
                Stage::Fragment
            ),
            LOD
        );
        assert_eq!(
            fails_stage(
                "uniform samplerCube s; void main() { gl_FragColor = textureCubeLod(s, vec3(0.0), 1.0); }",
                Stage::Fragment
            ),
            LOD
        );

        /* Each in the stage it belongs to, which must keep building --
         * this is the half that would break working shaders. */
        ok_stage("uniform sampler2D s; void main() { gl_FragColor = texture2D(s, vec2(0.0), 1.0); }", Stage::Fragment);
        ok_stage("uniform sampler2D s; void main() { gl_Position = texture2DLod(s, vec2(0.0), 1.0); }", Stage::Vertex);
        /* And the plain lookups, which carry neither argument, are allowed
         * in BOTH stages -- the paragraph above those two sentences says so
         * in as many words. */
        ok_stage("uniform sampler2D s; void main() { gl_FragColor = texture2D(s, vec2(0.0)); }", Stage::Fragment);
        ok_stage("uniform sampler2D s; void main() { gl_Position = texture2D(s, vec2(0.0)); }", Stage::Vertex);
    }

    /// Section 7.2: "a shader may assign values to either gl_FragColor or
    /// gl_FragData, but not both", where a static assignment is defined as
    /// "a statement that would write to x, whether or not run-time flow of
    /// control will cause that statement to be executed".
    ///
    /// dEQP `fragdata.write_fragcolor_and_fragdata_{simple,static_if,
    /// unused_func}` -- three shapes of one rule, because none of
    /// "reachable", "called" or "taken" appears in it.
    #[test]
    fn a_fragment_shader_writes_one_output_or_the_other_never_both() {
        const BOTH: &str = "a fragment shader writes either 'gl_FragColor' or 'gl_FragData', never both";
        assert_eq!(fails("void main() { gl_FragColor = vec4(1.0); gl_FragData[0] = vec4(1.0); }"), BOTH);
        assert_eq!(
            fails("void main() { gl_FragColor = vec4(1.0); if (false) { gl_FragData[0] = vec4(1.0); } }"),
            BOTH,
            "a branch never taken is still a statement that would write"
        );
        assert_eq!(
            fails("void unused() { gl_FragData[0] = vec4(1.0); } void main() { gl_FragColor = vec4(1.0); }"),
            BOTH,
            "a function nobody calls is still part of the shader"
        );
        /* A write through a field or a subscript is still a write to the
         * variable it is rooted in. */
        assert_eq!(fails("void main() { gl_FragColor.r = 1.0; gl_FragData[0].g = 1.0; }"), BOTH);

        /* Either alone is the ordinary case, and both must keep building. */
        ok("void main() { gl_FragColor = vec4(1.0); }");
        ok("void main() { gl_FragData[0] = vec4(1.0); }");
        ok("void main() { gl_FragData[0] = vec4(1.0); gl_FragData[1] = vec4(0.0); }");
        /* READING one while writing the other is not an assignment to it --
         * the rule is about statements that write. */
        ok("void main() { gl_FragData[0] = vec4(gl_FragCoord.x); }");
        /* The rule is fragment-only: a vertex shader has neither. */
        ok_stage("void main() { gl_Position = vec4(1.0); }", Stage::Vertex);
    }

    /// Section 3.7: "Identifiers starting with 'gl_' are reserved for use by
    /// OpenGL ES. No user-defined identifiers may begin with 'gl_'."
    ///
    /// dEQP `keywords.invalid_identifiers.gl_begin`.
    #[test]
    fn a_user_defined_name_cannot_begin_with_gl_() {
        const RESERVED: &str = "'gl_' is reserved: a user-defined name cannot begin with it";
        assert_eq!(fails("void main() { float gl_Invalid = 1.0; }"), RESERVED);
        assert_eq!(fails("uniform float gl_Thing; void main() {}"), RESERVED, "a global");
        assert_eq!(fails("void gl_f() {} void main() { gl_f(); }"), RESERVED, "a function");
        assert_eq!(fails("struct gl_S { int x; }; void main() {}"), RESERVED, "a structure");
        assert_eq!(fails("void f(float gl_p) {} void main() { f(1.0); }"), RESERVED, "a parameter");

        /* It is about DECLARING the name. Reading and assigning the built-in
         * variables is what a shader is for, and must keep building --
         * `declare_builtin_variables` pushes them without going through
         * `declare`, which is what makes that work. */
        ok("void main() { gl_FragColor = vec4(gl_FragCoord.x); }");
        ok_stage("void main() { gl_Position = vec4(1.0); gl_PointSize = 1.0; }", Stage::Vertex);
        /* And a name that merely CONTAINS "gl_" is fine. */
        ok("void main() { float my_gl_x = 1.0; gl_FragColor = vec4(my_gl_x); }");
    }

    /// Section 4.6.1 lists what may be declared invariant: built-in special
    /// variables and VARYING variables, on either side of either stage. An
    /// `attribute` and a `uniform` are not on that list, and the spec's own
    /// example of the combined form is `invariant varying mediump vec3
    /// Color;`.
    ///
    /// dEQP `declarations.invalid_declarations.invariant_{attribute,uniform}`.
    #[test]
    fn only_a_varying_or_a_builtin_may_be_invariant() {
        const ONLY: &str = "only a varying or a built-in variable may be invariant";
        assert_eq!(
            fails_stage(
                "invariant attribute mediump float val; void main() { gl_Position = vec4(1.0); }",
                Stage::Vertex
            ),
            ONLY
        );
        assert_eq!(fails("invariant uniform mediump float val; void main() {}"), ONLY);
        assert_eq!(fails("invariant mediump float val; void main() {}"), ONLY, "no storage qualifier either");

        /* The two forms the section does allow. */
        ok("invariant varying mediump vec3 Color; void main() { gl_FragColor = vec4(Color, 1.0); }");
        ok_stage("invariant gl_Position; void main() { gl_Position = vec4(1.0); }", Stage::Vertex);
        ok("varying mediump vec3 Color; invariant Color; void main() { gl_FragColor = vec4(Color, 1.0); }");
    }

    /// Section 6.1, on what makes two declarations the same function:
    /// "This includes exact match of array size as well."
    ///
    /// So `float func(float f[3])` and `float func(float f[4])` are two
    /// OVERLOADS. The checker compared array-NESS, called them one
    /// signature, and reported the second as already defined -- a VERDICT on
    /// a shader dEQP says must build (`overloading.array_size`), which is
    /// worse than a gap: it tells the author to rewrite working GLSL.
    #[test]
    fn two_array_parameters_of_different_sizes_are_two_overloads() {
        ok("float func(float f[3]) { return f[0]; } \
            float func(float f[4]) { return f[1]; } \
            void main() { float a[3]; a[0] = 1.0; gl_FragColor = vec4(func(a)); }");
        /* The same size twice is still one signature declared twice. */
        assert_eq!(
            fails("float func(float f[3]) { return f[0]; } float func(float f[3]) { return f[1]; } void main() {}"),
            "this function is already defined"
        );
        /* And the CALL picks by size, so an argument that matches neither
         * declaration is still refused. */
        assert_eq!(
            fails(
                "float func(float f[3]) { return f[0]; } float func(float f[4]) { return f[1]; } \
                 void main() { float a[2]; gl_FragColor = vec4(func(a)); }"
            ),
            "no declaration of this function takes these arguments"
        );
    }

    #[test]
    fn scopes_nest_and_shadow() {
        ok("void main() { float x = 1.0; { float x = 2.0; x = 3.0; } x = 4.0; }");
        assert_eq!(fails("void main() { float x = 1.0; float x = 2.0; }"), "redeclared in the same scope");
        assert_eq!(fails("void main() { y = 1.0; }"), "undeclared identifier");
        /* A local goes out of scope at the end of its block. */
        assert_eq!(fails("void main() { { float x = 1.0; } x = 2.0; }"), "undeclared identifier");
    }

    #[test]
    fn matrix_and_vector_arithmetic_follows_section_5_1() {
        ok_vs("uniform mat4 m;\nattribute vec4 p;\nvoid main() { gl_Position = m * p; }");
        ok("uniform mat3 a;\nvoid main() { mat3 b = a * a; }");
        ok("varying vec4 v;\nvoid main() { gl_FragColor = v * 2.0; }");
        assert_eq!(
            fails("uniform mat4 m;\nuniform vec3 v;\nvoid main() { vec3 r = m * v; }"),
            "these operand types cannot be multiplied"
        );
        assert_eq!(
            fails("varying vec4 a;\nvarying vec2 b;\nvoid main() { vec4 c = a + b; }"),
            "the two sides must have the same type"
        );
    }

    /// A built-in call stays legal whatever the checker's own type table
    /// says about it. What changed on 2026-09-23 is the third case: a name
    /// that is no built-in and no function of the unit was left "unknown"
    /// in case it was an unmodelled built-in, but the table now names every
    /// section 8 function, so it is refused as section 6.1 says.
    #[test]
    fn a_built_in_call_is_legal_and_an_undeclared_one_is_not() {
        ok("uniform sampler2D s;\nvarying vec2 uv;\nvoid main() { gl_FragColor = texture2D(s, uv); }");
        ok("varying vec4 v;\nvoid main() { gl_FragColor = vec4(dot(v.xy, v.zw)); }");
        assert_eq!(fails("void main() { float f = someHelper(1, 2.0); }"), "no function of this name is declared");
    }

    #[test]
    fn a_function_may_be_called_before_it_is_defined() {
        ok("float helper(float x);\nvoid main() { float f = helper(1.0); }\nfloat helper(float x) { return x; }");
    }

    #[test]
    fn conditions_must_be_bool() {
        ok("void main() { if (true) { discard; } }");
        assert_eq!(fails("void main() { float f = 1.0; if (f) { discard; } }"), "a condition must be a bool");
        assert_eq!(fails("void main() { while (1) { discard; } }"), "a condition must be a bool");
    }

    /// ES 3.00 section 6.1: a prototype may be repeated; ES 1.00 refuses
    /// it (the same dEQP case names expect opposite verdicts).
    ///
    /// Revert check: drop the `!ast.es3` and the ES 3.00 shape is refused
    /// with "no declaration of this function takes these arguments".
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn es300_repeats_a_prototype_and_es100_does_not() {
        let src = "float func(float x);\nfloat func(float);\nfloat func(float y) { return y; }\n\
                   void main() { float f = func(1.0); }";
        assert_eq!(fails(src), "no declaration of this function takes these arguments");
        let mut buf = [0u8; 1 << 18];
        let es3 = std::format!("#version 300 es\nprecision mediump float;\n{src}");
        check_src(&mut buf, &es3, Stage::Fragment).unwrap_or_else(|e| panic!("refused: {}", e.msg));
    }

    #[test]
    fn break_and_continue_need_a_loop() {
        ok("void main() { for (int i = 0; i < 2; ++i) { break; } }");
        /* Two messages since ES 3.00's `switch`, where `break` is legal and
         * `continue` is not. */
        assert_eq!(fails("void main() { break; }"), "'break' outside a loop or a switch");
        assert_eq!(fails("void main() { continue; }"), "'continue' outside a loop");
    }
    /// The checker's types are kept, not thrown away, and they are the ones
    /// the lowering cannot work out for itself.
    ///
    /// `u_mvp * a_pos` and `u_scale * a_pos` are the same syntax; one is a
    /// matrix product that becomes four instructions and the other is a
    /// component-wise multiply that becomes one. Nothing but the type of the
    /// left operand tells them apart, which is why `ExprTypes` exists.
    #[test]
    fn the_type_of_every_expression_is_kept() {
        let src = "uniform mat4 u_mvp;\n\
                   uniform float u_scale;\n\
                   attribute vec4 a_pos;\n\
                   void main() { gl_Position = u_mvp * a_pos + u_scale * a_pos; }\n";
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let pre = pp::preprocess(&a, src).expect("preprocess");
        let ast = parser::parse(&a, pre.tokens).expect("parse");
        let types = check(&a, &ast, Stage::Vertex).expect("check");

        /* Walk the expressions rather than guessing an id: the ids are an
         * internal numbering and a test that hard-codes one breaks for a
         * reason that is not a defect. */
        let mut mat = 0;
        let mut scalar = 0;
        let mut product = 0;
        for i in 0..ast.exprs.len() {
            let id = ExprId(i as u32);
            let ty = types.of(id);
            match *ast.expr(id) {
                Expr::Ident("u_mvp") => {
                    assert_eq!(ty.base, BaseType::Mat(4, 4), "u_mvp is a mat4");
                    mat += 1;
                }
                Expr::Ident("u_scale") => {
                    assert_eq!(ty.base, BaseType::Float, "u_scale is a float");
                    scalar += 1;
                }
                /* Both products are vec4: `mat4 * vec4` by section 5.10 and
                 * `float * vec4` by the scalar rule. Identical results from
                 * different operations -- exactly why the operand types, not
                 * the result, decide the lowering. */
                Expr::Binary(BinOp::Mul, ..) => {
                    assert_eq!(ty.base, BaseType::Vec(4), "both products are vec4");
                    product += 1;
                }
                _ => {}
            }
        }
        assert_eq!((mat, scalar, product), (1, 1, 2), "each operand appears once and there are two products");
    }
    /// A user function may be overloaded on its parameter types (section
    /// 6.1), so the call's arguments decide which declaration it names --
    /// not the order the declarations appear in.
    ///
    /// `f(a).xyz` is the shape that catches it: with the `float` overload
    /// chosen, the swizzle is three components of a scalar and the checker
    /// refuses a shader that is correct. dEQP's `overloading.user_func_arg_*`
    /// are exactly this, and ten of them were rejected before the resolution
    /// existed.
    #[test]
    fn an_overloaded_user_function_is_resolved_by_its_arguments() {
        ok_vs(
            "float f(float x) { return x; }\n\
             vec3 f(vec3 v) { return v; }\n\
             attribute vec3 a;\n\
             void main() { gl_Position = vec4(f(a).xyz, 1.0); }\n",
        );
        /* And the other way round, so the test is not passing because the
         * second declaration happens to be the one taken. */
        ok_vs(
            "vec3 f(vec3 v) { return v; }\n\
             float f(float x) { return x; }\n\
             attribute float a;\n\
             void main() { gl_Position = vec4(f(a), 0.0, 0.0, 1.0); }\n",
        );
    }
}

#[cfg(test)]
mod float_precision_tests {
    use super::*;

    /// The same four lines `tests::check_src` uses; duplicated rather than
    /// imported because that one is private to its own module.
    fn check_src<'a>(buf: &'a mut [u8], src: &'a str, stage: Stage) -> R<'a, ExprTypes<'a>> {
        let a = Arena::new(buf);
        let pre = super::super::pp::preprocess(&a, src)?;
        let ast = super::super::parser::parse(&a, pre.tokens)?;
        check(&a, &ast, stage)
    }

    fn ok(src: &str, stage: Stage) {
        let mut buf = [0u8; 1 << 18];
        if let Err(e) = check_src(&mut buf, src, stage) {
            panic!("expected success for {src:?}, got {:?}: {}", e.tok, e.msg);
        }
    }

    fn refused(src: &str, stage: Stage) -> &'static str {
        let mut buf = [0u8; 1 << 18];
        match check_src(&mut buf, src, stage) {
            Ok(_) => panic!("expected a refusal for {src:?}"),
            Err(e) => e.msg,
        }
    }

    /// Section 4.5.3: the fragment language has NO default float precision,
    /// so a float, float-vector or float-matrix declaration must carry a
    /// qualifier or follow a `precision <p> float;`.
    ///
    /// Found on silicon by cts-runner, not by the corpus: our online pair
    /// compiled and RAN a shader glslang refuses, and dEQP's shader tests
    /// contain no case for it, because dEQP's own templates always emit the
    /// declaration. A false ACCEPT, which is the direction the false-reject
    /// column cannot show.
    #[test]
    fn a_fragment_float_declaration_needs_a_precision() {
        let msg = refused(
            "varying vec4 v_color;\nvoid main(){ gl_FragColor = vec4(clamp(v_color.xyz, 0.0, 1.0), 1.0); }",
            Stage::Fragment,
        );
        assert!(msg.contains("default precision qualifier"), "{msg}");
    }

    /// The rule is POSITIONAL: the default must already have been declared.
    #[test]
    fn the_default_must_come_before_the_declaration_that_needs_it() {
        ok("precision mediump float;\nvarying vec4 v;\nvoid main(){ gl_FragColor = v; }", Stage::Fragment);
        refused("varying vec4 v;\nprecision mediump float;\nvoid main(){ gl_FragColor = v; }", Stage::Fragment);
    }

    /// The four ways a fragment shader is legal without a default, each one
    /// a shader that must still compile. A rule of this shape fails by
    /// refusing working code, so the edges are the test.
    #[test]
    fn what_the_rule_must_not_refuse() {
        /* An explicit qualifier on the declaration itself. */
        ok("varying mediump vec4 v;\nvoid main(){ gl_FragColor = v; }", Stage::Fragment);
        /* `int` has a fragment default (mediump), so it needs nothing. */
        ok("void main(){ int i = 1; gl_FragColor = vec4(0.0); }", Stage::Fragment);
        /* The samplers have one too (lowp). */
        ok(
            "uniform sampler2D s;\nvarying mediump vec2 t;\nvoid main(){ gl_FragColor = texture2D(s,t); }",
            Stage::Fragment,
        );
        /* And the VERTEX language has a default float precision, which is
         * the whole reason this rule is fragment-only. */
        ok("attribute vec4 p;\nvoid main(){ gl_Position = p; }", Stage::Vertex);
    }

    /// A local declaration is a declaration: setting a default for `int`
    /// does not set one for `float`.
    #[test]
    fn a_default_for_another_type_does_not_serve_float() {
        let msg =
            refused("precision mediump int;\nvoid main(){ float x = 1.0; gl_FragColor = vec4(x); }", Stage::Fragment);
        assert!(msg.contains("default precision qualifier"), "{msg}");
    }
}

#[cfg(test)]
mod prototype_tests {
    use super::*;

    const HEAD: &str = "attribute vec4 p;\nattribute float in0;\nvarying float out0;\n";

    fn lowers(body: &str) -> Result<(), &'static str> {
        let mut buf = [0u8; 1 << 19];
        let a = Arena::new(&mut buf);
        let mut src = [0u8; 1024];
        let n = HEAD.len() + body.len();
        src[..HEAD.len()].copy_from_slice(HEAD.as_bytes());
        src[HEAD.len()..n].copy_from_slice(body.as_bytes());
        let text = core::str::from_utf8(&src[..n]).unwrap();
        let pre = super::super::pp::preprocess(&a, text).map_err(|e| e.msg)?;
        let ast = super::super::parser::parse(&a, pre.tokens).map_err(|e| e.msg)?;
        let types = check(&a, &ast, Stage::Vertex).map_err(|e| e.msg)?;
        let refl = super::super::reflect::reflect(&a, &ast).map_err(|e| e.msg)?;
        super::super::lower::lower(&a, &ast, Stage::Vertex, &refl, types).map_err(|e| e.msg)?;
        Ok(())
    }

    /// Section 6.1: a function may be DECLARED before it is defined, which
    /// is how it is called before its body appears. A prototype and its
    /// definition are one function with one signature, not two overloads.
    ///
    /// Counting them as two left the call untyped, and the lowering then
    /// refused every operator applied to it -- 12 dEQP cases, all reading
    /// "the type of this expression is not known", which named neither the
    /// prototype nor the operator.
    #[test]
    fn a_prototype_and_its_definition_are_one_function() {
        assert_eq!(
            lowers(
                "float func();\nvoid main(){ out0 = func() * in0; gl_Position=p; }\nfloat func(void){ return -1.0; }"
            ),
            Ok(())
        );
    }

    /// The two controls that named the cause: without the operator, and
    /// with the definition first, it lowered all along.
    #[test]
    fn the_controls_that_named_it() {
        assert_eq!(
            lowers("float func();\nvoid main(){ out0 = func(); gl_Position=p; }\nfloat func(void){ return -1.0; }"),
            Ok(())
        );
        assert_eq!(
            lowers("float func(void){ return -1.0; }\nvoid main(){ out0 = func() * in0; gl_Position=p; }"),
            Ok(())
        );
    }

    /// Real overloads still resolve by argument type.
    #[test]
    fn different_signatures_are_still_overloads() {
        assert_eq!(
            lowers("float f(float a){return a;}\nfloat f(vec2 a){return a.x;}\nvoid main(){ out0=f(in0)*f(vec2(1.0)); gl_Position=p; }"),
            Ok(())
        );
    }

    /// AND THE PART THAT COST FOUR CASES. One signature may have ONE
    /// prototype and ONE definition. Two of either is a redeclaration and
    /// the shader is wrong.
    ///
    /// The first version of this fix said "prefer the one with a body",
    /// which quietly accepted two prototypes -- dEQP's
    /// `invalid.double_declare` and `invalid.redeclare_function`, four
    /// must-fail cases that began compiling. Measured by diffing the
    /// accepted set against the build before the change, which is the only
    /// reason it was noticed: the false-reject column went down and said
    /// nothing.
    #[test]
    fn declaring_one_signature_twice_stays_refused_where_it_was() {
        /* dEQP's own shape uses the result, and these are the four cases
         * that went from refused to accepted while the false-reject column
         * went DOWN and said nothing. */
        assert!(lowers("float f();\nfloat f();\nvoid main(){ out0=f()*in0; gl_Position=p; }\nfloat f(){return 1.0;}")
            .is_err());
        assert!(lowers("float f(){return 1.0;}\nfloat f(){return 2.0;}\nvoid main(){ out0=f()*in0; gl_Position=p; }")
            .is_err());
        assert!(
            lowers("float f(){return 1.0;}\nfloat f(){return 2.0;}\nvoid main(){ out0=f(); gl_Position=p; }").is_err()
        );

        /* AND THE GAP THAT IS NOW CLOSED, kept because it was pinned here
         * as standing and the record should show what closed it.
         *
         * Redeclaration is still not DETECTED as such -- `resolve_overload`
         * answers `None` for two prototypes rather than reporting them --
         * but a call it cannot resolve is an ERROR now instead of an
         * untyped value, so the shape reaches a refusal by the other road:
         *
         *     float f(); float f();
         *     void main(){ out0 = f(); ... }     <- refused
         *
         * This asserted `.is_ok()` when it was written, with a note that a
         * real "one declaration per signature" rule would close it. What
         * closed it was the ARGUMENT check, one commit later: an unresolved
         * call stops rather than going untyped. The rule I named is still
         * the better diagnosis and still unwritten; the verdict is right
         * either way. */
        assert!(
            lowers("float f();\nfloat f();\nvoid main(){ out0=f(); gl_Position=p; }\nfloat f(){return 1.0;}").is_err()
        );
    }
}

#[cfg(test)]
mod const_expr_tests {
    use super::*;

    const HEAD: &str = "precision mediump float;\nattribute vec4 p;\nvarying float out0;\n";

    fn check<'a>(buf: &'a mut [u8], body: &'a str, src: &'a mut [u8]) -> Result<(), &'static str> {
        let n = HEAD.len() + body.len();
        src[..HEAD.len()].copy_from_slice(HEAD.as_bytes());
        src[HEAD.len()..n].copy_from_slice(body.as_bytes());
        let text = core::str::from_utf8(&src[..n]).unwrap();
        let a = Arena::new(buf);
        let pre = super::super::pp::preprocess(&a, text).map_err(|e| e.msg)?;
        let ast = super::super::parser::parse(&a, pre.tokens).map_err(|e| e.msg)?;
        super::check(&a, &ast, Stage::Vertex).map_err(|e| e.msg)?;
        Ok(())
    }

    fn refused(body: &str) -> &'static str {
        let mut buf = [0u8; 1 << 18];
        let mut src = [0u8; 2048];
        match check(&mut buf, body, &mut src) {
            Ok(()) => panic!("expected a refusal for {body:?}"),
            Err(m) => m,
        }
    }

    fn ok(body: &str) {
        let mut buf = [0u8; 1 << 18];
        let mut src = [0u8; 2048];
        if let Err(m) = check(&mut buf, body, &mut src) {
            panic!("expected success for {body:?}, got {m}");
        }
    }

    /// Section 4.3.2: "Initializers for const declarations must be a
    /// constant expression", and 5.10 says what one is.
    ///
    /// Ten dEQP must-fail cases compiled here. They are FALSE ACCEPTS, the
    /// column that read "coverage, not a pass mark" all day and hid them.
    #[test]
    fn a_const_needs_a_constant_expression() {
        for body in [
            "uniform float u;\nvoid main(){ const float c = u; out0=c; gl_Position=p; }",
            "void main(){ float v=20.0; const float c = v; out0=c; gl_Position=p; }",
            "float f(){return 50.0;}\nvoid main(){ const float c = f(); out0=c; gl_Position=p; }",
        ] {
            assert_eq!(refused(body), "a const needs a constant expression to initialize it");
        }
    }

    /// dEQP calls this one `const_float_function_gotcha`, and it is why the
    /// symbol table records whether a name is a PARAMETER.
    ///
    /// `const` means two different things on the two: a `const` variable
    /// initialised with a constant expression IS one, and a `const`
    /// parameter is merely read-only -- its value arrives at run time. A
    /// test on `storage` alone answers the same for both.
    ///
    /// Revert check: drop `&& !s.is_param` from `const_expr` and this case
    /// alone starts compiling again.
    #[test]
    fn a_const_parameter_is_read_only_not_constant() {
        assert_eq!(
            refused("float f(const float g){ const float c = g; return c*2.0; }\nvoid main(){ const float k=10.0; out0=f(k); gl_Position=p; }"),
            "a const needs a constant expression to initialize it"
        );
    }

    /// The edges, which is where a rule of this shape fails: each of these
    /// IS a constant expression by 5.10 and must still compile.
    #[test]
    fn what_the_rule_must_not_refuse() {
        ok("void main(){ const float c = 1.5; out0=c; gl_Position=p; }");
        ok("const float a = 2.0;\nvoid main(){ const float c = a * 3.0; out0=c; gl_Position=p; }");
        ok("void main(){ const vec2 c = vec2(1.0, 2.0); out0=c.x; gl_Position=p; }");
        ok("void main(){ const float c = max(1.0, 2.0); out0=c; gl_Position=p; }");
        /* And the rule is about `const` only: an ordinary variable may be
         * initialised from anything. */
        ok("uniform float u;\nvoid main(){ float v = u; out0=v; gl_Position=p; }");
        ok("void main(){ float v=1.0; float w = v; out0=w; gl_Position=p; }");
    }
}

#[cfg(test)]
mod declaration_agreement_tests {
    use super::*;

    fn run(src: &str) -> Result<(), &'static str> {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let pre = super::super::pp::preprocess(&a, src).map_err(|e| e.msg)?;
        let ast = super::super::parser::parse(&a, pre.tokens).map_err(|e| e.msg)?;
        check(&a, &ast, Stage::Vertex).map_err(|e| e.msg)?;
        Ok(())
    }

    const HEAD: &str = "precision mediump float;\n";
    const MAIN: &str = "void main(){ gl_Position = vec4(1.0); }\n";

    fn refused(body: &str) -> &'static str {
        let mut s = [0u8; 2048];
        let n = HEAD.len() + body.len() + MAIN.len();
        s[..HEAD.len()].copy_from_slice(HEAD.as_bytes());
        s[HEAD.len()..HEAD.len() + body.len()].copy_from_slice(body.as_bytes());
        s[HEAD.len() + body.len()..n].copy_from_slice(MAIN.as_bytes());
        match run(core::str::from_utf8(&s[..n]).unwrap()) {
            Ok(()) => panic!("expected a refusal for {body:?}"),
            Err(m) => m,
        }
    }

    fn ok(body: &str) {
        let mut s = [0u8; 2048];
        let n = HEAD.len() + body.len() + MAIN.len();
        s[..HEAD.len()].copy_from_slice(HEAD.as_bytes());
        s[HEAD.len()..HEAD.len() + body.len()].copy_from_slice(body.as_bytes());
        s[HEAD.len() + body.len()..n].copy_from_slice(MAIN.as_bytes());
        if let Err(m) = run(core::str::from_utf8(&s[..n]).unwrap()) {
            panic!("expected success for {body:?}, got {m}");
        }
    }

    /// Section 6.1. Two declarations of one name are OVERLOADS only when
    /// their parameter types differ; same name and same parameter types is
    /// one SIGNATURE, which may have one prototype and one definition, and
    /// they must agree in everything else.
    ///
    /// Twelve dEQP rules and twenty-four must-fail cases were the same
    /// question asked of the same group, and none of them was asked: the
    /// checker looked a name up and never checked that its declarations
    /// were consistent.
    #[test]
    fn one_signature_is_one_prototype_and_one_definition() {
        assert_eq!(
            refused("float f(float a){return a;}\nfloat f(float a){return a;}\n"),
            "this function is already defined"
        );
        assert_eq!(
            refused("void f(vec3 a);\nfloat f(vec3 a){return a.x;}\n"),
            "cannot be overloaded on the return type"
        );
        assert_eq!(
            refused("float f(float a){return a;}\nint f(float a){return int(a);}\n"),
            "cannot be overloaded on the return type"
        );
    }

    /// And not on a parameter's qualifiers either -- `in`/`out`/`inout`,
    /// `const`, or an explicit precision.
    #[test]
    fn nor_on_a_parameters_qualifiers() {
        assert_eq!(
            refused("void f(float a);\nvoid f(out float a){a=1.0;}\n"),
            "cannot be overloaded on a parameter's in/out qualifier"
        );
        assert_eq!(
            refused("void f(float a){}\nvoid f(const float a){}\n"),
            "cannot be overloaded on a parameter's const qualifier"
        );
        assert_eq!(
            refused("float f(lowp float a){return a;}\nfloat f(mediump float a){return a;}\n"),
            "cannot be overloaded on a parameter's precision"
        );
        assert_eq!(
            refused("lowp float f(float a){return a;}\nmediump float f(float a){return a;}\n"),
            "cannot be overloaded on the return precision"
        );
    }

    /// THE COST THIS CHECK NEARLY HAD, and the reason `precisions_disagree`
    /// exists rather than `!=`.
    ///
    /// `Precision::Default` is not a precision -- it is whichever one is in
    /// force -- so a declaration writing none cannot contradict one writing
    /// the default's own value. dEQP's
    /// `declarations.default_vs_explicit_precision` is that shape and must
    /// BUILD. Comparing the written tokens refused it, which cost two
    /// must-build cases when this check first landed; the diff against the
    /// previous build is how it was found, because the false-reject total
    /// moved by 2 while the false-accept total moved by 24.
    ///
    /// Revert check: compare `p.ty.precision != q.ty.precision` directly
    /// and this test fails while every refusal above still passes.
    #[test]
    fn writing_no_precision_does_not_contradict_writing_the_default() {
        ok("float f(float a);\nfloat f(mediump float a){return -a;}\n");
    }

    /// Real overloads, and a prototype that agrees with its definition,
    /// must both still work.
    #[test]
    fn what_the_check_must_not_refuse() {
        ok("float f(float a){return a;}\nfloat f(vec2 a){return a.x;}\n");
        ok("float f(vec3 a);\nfloat f(vec3 a){return a.x;}\n");
        ok("void f(out float a);\nvoid f(out float a){a=1.0;}\n");
    }
}

#[cfg(test)]
mod call_argument_tests {
    use super::*;

    fn run(src: &str) -> Result<(), &'static str> {
        let mut buf = [0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let pre = super::super::pp::preprocess(&a, src).map_err(|e| e.msg)?;
        let ast = super::super::parser::parse(&a, pre.tokens).map_err(|e| e.msg)?;
        check(&a, &ast, Stage::Vertex).map_err(|e| e.msg)?;
        Ok(())
    }

    const TAIL: &str = "void main(){ gl_Position = vec4(1.0); }\n";

    fn whole<'b>(body: &str, buf: &'b mut [u8]) -> &'b str {
        let n = body.len() + TAIL.len();
        buf[..body.len()].copy_from_slice(body.as_bytes());
        buf[body.len()..n].copy_from_slice(TAIL.as_bytes());
        core::str::from_utf8(&buf[..n]).unwrap()
    }

    fn refused(body: &str) -> &'static str {
        let mut b = [0u8; 2048];
        match run(whole(body, &mut b)) {
            Ok(()) => panic!("expected a refusal for {body:?}"),
            Err(m) => m,
        }
    }

    /// Sections 6.1 and 4.1.10: a call takes its parameters' types exactly,
    /// because there are no implicit conversions in either direction.
    ///
    /// Six dEQP must-fail cases compiled here, and the reason is worth more
    /// than the rule: `resolve_overload` answers "which declaration is this
    /// call for", and with only ONE candidate it answers without comparing
    /// types. Right for a lookup, useless as a check -- so the check lives
    /// in the caller now.
    ///
    /// MEASURED, not reasoned: my first attempt made an UNMATCHED call an
    /// error instead of leaving it untyped, and moved NEITHER column,
    /// because with one declaration nothing was ever unmatched. Diffing
    /// both columns against a build of the parent is what showed a change
    /// that did nothing.
    #[test]
    fn a_call_takes_its_parameters_types_exactly() {
        for body in [
            "void func (float f){}\nvoid g(){ func(2); }\n",
            "void func (vec2 f){}\nvoid g(){ func(2.0); }\n",
            "void func (vec2 f){}\nvoid g(){ func(vec3(1.0)); }\n",
        ] {
            assert_eq!(refused(body), "this argument is not the parameter's type", "for {body:?}");
        }
    }

    #[test]
    fn and_the_right_number_of_them() {
        assert_eq!(
            refused("void func (float f){}\nvoid g(){ func(1.0, 2.0); }\n"),
            "wrong number of arguments for this function"
        );
    }

    /// The edges: a correct call, a real overload set resolving by argument
    /// type, and a built-in, must all still compile. A rule of this shape
    /// fails by refusing working code.
    ///
    /// Revert check: remove the `check_call_arguments` call and the three
    /// refusals above compile again while these keep passing.
    #[test]
    fn what_the_check_must_not_refuse() {
        let mut b = [0u8; 2048];
        run(whole("void func (float f){}\nvoid g(){ func(2.0); }\n", &mut b)).unwrap();
        let mut b2 = [0u8; 2048];
        run(whole(
            "float f(float a){return a;}\nfloat f(vec2 a){return a.x;}\nvoid g(){ float x = f(1.0) + f(vec2(1.0)); }\n",
            &mut b2,
        ))
        .unwrap();
        let mut b3 = [0u8; 2048];
        run(whole("void g(){ float x = max(1.0, 2.0); }\n", &mut b3)).unwrap();
    }
}

/// ES 3.00 section 5.9's integer operators and 4.1.10's `uint`, as typing
/// rules. The vertex stage, so no precision statement is needed.
#[cfg(test)]
mod es3_integer_tests {
    use super::*;
    use crate::shaderc::online::{parser, pp};

    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    fn run(body: &str) -> Result<(), std::string::String> {
        let src = std::format!("#version 300 es\nvoid f() {{ {body} }}\nvoid main() {{ gl_Position = vec4(0.0); }}\n");
        let mut buf = std::vec![0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let pre = pp::preprocess(&a, &src).map_err(|e| std::string::String::from(e.msg))?;
        let ast = parser::parse(&a, pre.tokens).map_err(|e| std::string::String::from(e.msg))?;
        check(&a, &ast, Stage::Vertex).map(|_| ()).map_err(|e| e.msg.into())
    }

    #[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
    fn refused(body: &str) -> std::string::String {
        match run(body) {
            Ok(()) => panic!("compiled: {body}"),
            Err(m) => m,
        }
    }

    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn what_the_rules_must_not_refuse() {
        for body in [
            "uint a = 1u; uint b = a * 3u + a / 2u - 1u;",
            "uvec3 v = uvec3(1u); uvec3 w = v % 2u; w = w & v | 1u; w ^= v;",
            /* a shift may mix signedness, and takes the left operand's type */
            "int i = 1; uint u = 2u; int j = i << u; uint k = u >> i; ivec2 s = ivec2(i) << 1u;",
            "ivec3 a = ivec3(1) << ivec3(2);",
            "int i = ~5; uint u = ~0u; i %= 3; u <<= 2u;",
            "bool b = 1u < 2u; float f = float(3u); uint c = uint(2.5); int d = int(4294967295u);",
            "float a[4]; float x = a[1u];",
            "float a[2u];",
        ] {
            if let Err(m) = run(body) {
                panic!("refused {body:?}: {m}");
            }
        }
    }

    /// Revert check: let `integer_operator` accept any integer pair and the
    /// first two compile.
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn what_the_rules_refuse() {
        assert!(refused("int i = 1; uint u = 1u; uint k = i + u;").contains("implicit conversion"));
        assert!(refused("int i = 1; uint u = 1u; int k = i & int(u) | u;").contains("signed"));
        assert!(refused("float f = 1.0; float g = f % 2.0;").contains("integers"));
        assert!(refused("ivec2 k = 1 << ivec2(1);").contains("shift"));
        assert!(refused("ivec3 a = ivec3(1) & ivec2(1);").contains("same type"));
        assert!(refused("float f = ~1.0;").contains("integer"));
        assert!(refused("uint u = 1;").contains("implicit conversion"));
    }

    /// ES 1.00 reserves every one of these operators, and keeps doing so.
    #[test]
    fn es1_still_reserves_them() {
        let mut buf = std::vec![0u8; 1 << 18];
        let a = Arena::new(&mut buf);
        let src = "void main() { int i = 5 % 2; gl_Position = vec4(0.0); }";
        let pre = pp::preprocess(&a, src).unwrap();
        let ast = parser::parse(&a, pre.tokens).unwrap();
        assert_eq!(check(&a, &ast, Stage::Vertex).err().expect("refused").msg, "reserved in GLSL ES 1.00");
    }
}
