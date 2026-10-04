// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The contract between the GLSL ES 1.00 front end and the code generator.
//!
//! **`docs/online-shaderc/ir.md` is the contract; this file implements it.**
//! Change the document and this file together, never one alone. Three people
//! write against it: the front end (M2) builds a [`Module`], the code
//! generator (M3) reads it, and the instruction encoders (M1) are what the
//! code generator emits through.
//!
//! The five decisions, in the document's words and its order:
//!
//! 1. **Vector values, 1 to 4 components, with a swizzle on every operand.**
//!    The first target is the GC7000, a vec4 machine with write masks and
//!    swizzles; the PowerVR USC is scalar. Scalarising a vector IR is a
//!    mechanical per-component expansion, while re-vectorising a scalar one is
//!    a pattern-matching pass over sequences that no longer say they belong
//!    together. One IR serves both only if the vector form is the stored one.
//! 2. **Structured control flow, and no phi nodes.** GLSL has no `goto`, so
//!    the parser's tree is kept ([`Node`]). Values are SSA within
//!    straight-line code; anything that crosses control flow goes through a
//!    [`LocalId`] slot with [`Op::LoadLocal`] / [`Op::StoreLocal`], and the
//!    code generator gives each slot a register.
//! 3. **The front end inlines every call.** ES 1.00 forbids recursion, so
//!    there is one body and no call stack.
//! 4. **Matrices, structs and arrays are not IR types.** A matrix is its
//!    columns; a struct uniform is flattened (that is what
//!    `ShcUniform::inner` encodes); arrays survive only as
//!    [`Op::LoadUniformIdx`] and as [`LocalId`] slots with an index.
//! 5. **No heap.** Every node is POD, every reference is an index, every list
//!    is a flat slice in the caller's arena (`super::arena`, lent by
//!    `super::store::with_scratch`). Nothing here may be stored in a
//!    `ShcShader`: what a shader keeps is copied into `store::keep()` by
//!    `super::abi`.

/// An SSA value: the result of the instruction with the same index.
///
/// There is one instruction array ([`Module::instrs`]); `ValueId(i)` is the
/// value that `instrs[i]` produced. An instruction that produces no value (a
/// store, a discard) is simply never referred to by one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ValueId(pub u32);

/// A mutable slot: where a value that crosses control flow lives, instead of a
/// phi node. Indexes [`Module::locals`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalId(pub u32);

/// An instruction, by index into [`Module::instrs`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InstrId(pub u32);

/// The scalar type a value's components hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScalarType {
    F32,
    I32,
    /// GLSL ES 3.00 `uint`: the same 32 bits as `I32`; the opcode (`Udiv`,
    /// `Ult`, `U2f`, ...) says where the two differ.
    U32,
    /// GLSL `bool`: 0 or 1 in an integer lane.
    Bool,
}

/// A value's type: a scalar kind and 1 to 4 components.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Type {
    pub scalar: ScalarType,
    /// 1..4.
    pub comps: u8,
}

impl Type {
    pub const fn new(scalar: ScalarType, comps: u8) -> Type {
        Type { scalar, comps }
    }
    pub const F32X4: Type = Type::new(ScalarType::F32, 4);
}

/// The identity swizzle: component *i* reads component *i*.
pub const SWIZZLE_IDENTITY: [u8; 4] = [0, 1, 2, 3];
/// Every component reads component 0 — how a scalar feeds a vector operation.
pub const SWIZZLE_SPLAT0: [u8; 4] = [0, 0, 0, 0];

/// One use of a value, with the swizzle that selects its components.
///
/// No negate or absolute-value modifiers: the back ends fold those into the
/// instruction they emit, where the hardware has a bit for it, rather than the
/// IR carrying a form of every operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Src {
    pub val: ValueId,
    /// One source component per destination component.
    pub swizzle: [u8; 4],
}

impl Src {
    /// `val` read straight through.
    pub const fn id(val: ValueId) -> Src {
        Src { val, swizzle: SWIZZLE_IDENTITY }
    }
    /// Component 0 of `val`, broadcast.
    pub const fn splat(val: ValueId) -> Src {
        Src { val, swizzle: SWIZZLE_SPLAT0 }
    }
}

/// A shader input or output, named the way the hardware and the table key
/// them: attributes and varyings by **location**, the rest by what they are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoSlot {
    /// Vertex input at this location.
    Attrib(u8),
    /// Inter-stage varying at this location: a vertex output or a fragment
    /// input, according to the stage.
    Varying(u8),
    /// `gl_Position`.
    Position,
    /// `gl_PointSize`.
    PointSize,
    /// `gl_FragColor` / `gl_FragData[n]`.
    FragColor(u8),
    /// `gl_FragDepth` (ES 3.00; here so the type need not change).
    FragDepth,
    /// `gl_FragCoord`.
    FragCoord,
    /// `gl_FrontFacing`.
    FrontFacing,
    /// `gl_PointCoord`.
    PointCoord,
    /// `gl_VertexID` (ES 3.00): one int lane, vertex stage.
    VertexId,
    /// `gl_InstanceID` (ES 3.00): one int lane, vertex stage.
    InstanceId,
}

/// A sampler's dimensionality (ES 3.00 section 4.1.7).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TexDim {
    D2,
    D3,
    Cube,
    /// A 2D array texture; the layer is the coordinate's z.
    D2Array,
}

/// How a look-up chooses its level of detail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TexLod {
    /// From the derivatives; fragment only. The front end never hands one
    /// to a vertex module: an implicit look-up there is `Lod(0.0)`, the base
    /// level (ES 3.00 section 8.8).
    Implicit,
    /// The implicit level plus a bias, one float lane; fragment only.
    Bias(Src),
    /// An explicit level, one float lane.
    Lod(Src),
    /// Explicit gradients, each read with the identity swizzle at the
    /// coordinate's base width: 2 lanes for D2 and D2Array, 3 for D3 and
    /// Cube. Two ids rather than two `Src`s keeps `Op` at 32 bytes; with
    /// two `Src`s it is 40, on every instruction of every module.
    Grad { ddx: ValueId, ddy: ValueId },
    /// `texelFetch`: integer coordinates and this integer level, no
    /// filtering and no wrap. There is no cube fetch in ES 3.00.
    Fetch(Src),
}

/// The five look-up shapes of GLSL ES 1.00, which were the whole of IR v0's
/// `TexKind` (IR v1 replaced it with `Op::Tex`'s orthogonal fields).
///
/// A back end that has only these asks [`Op::es100_tex`], which maps the new
/// fields back one to one and answers `None` for every other shape -- so it
/// refuses those by name while the ES 1.00 forms compile exactly as before.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Es100Tex {
    /// `texture2D(s, vec2)`: `{D2, Implicit}`.
    Tex2D,
    /// `texture2D(s, vec2, float bias)`: `{D2, Bias}`; fragment only.
    Tex2DBias,
    /// `texture2DLod(s, vec2, float lod)`: `{D2, Lod}`.
    Tex2DLod,
    /// `texture2DProj`: `{D2, proj, Implicit}`, the divisor in lane w.
    Tex2DProj,
    /// `textureCube(s, vec3)`: `{Cube, Implicit}`.
    TexCube,
}

/// The opcode kernel: what the front end lowers the whole ES 1.00 builtin
/// library to, and what a back end implements.
///
/// `ir.md`'s table, in its order. Everything else — `mat*vec`, `pow`, `mod`,
/// `clamp`, `mix`, `step`, `smoothstep`, `atan`, `asin`, `acos`,
/// `matrixCompMult`, the relational vector builtins — is expanded by the front
/// end and is deliberately absent.
///
/// The dot products are the exception that proves the rule: they stay because
/// the GC7000 has `DP2`, `DP3` and `DP4` natively (measured, from the
/// extracted instruction set), and expanding one into a multiply-add chain
/// only to recognise it again in the code generator is the pattern-matching
/// pass that decision 1 exists to avoid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AluOp {
    /* float arithmetic */
    Fadd,
    Fmul,
    /// `a * b + c`.
    Ffma,
    Fmin,
    Fmax,
    Fneg,
    Fabs,
    Fsign,
    /* float transcendental */
    Frcp,
    Frsq,
    Fsqrt,
    Fexp2,
    Flog2,
    Fsin,
    Fcos,
    /* float rounding */
    Ffloor,
    Fceil,
    Ftrunc,
    Ffract,
    /* derivatives; fragment only */
    Fddx,
    Fddy,
    /* dot products: native on the GC7000 */
    Fdot2,
    Fdot3,
    Fdot4,
    /* float compare -> Bool */
    Flt,
    Fle,
    Feq,
    Fne,
    /* integer */
    Iadd,
    Imul,
    Idiv,
    Imod,
    Ineg,
    Ishl,
    Ishr,
    Iand,
    Ior,
    Ixor,
    /* integer compare -> Bool */
    Ilt,
    Ile,
    Ieq,
    Ine,
    /* conversions */
    B2f,
    B2i,
    F2i,
    I2f,
    F2b,
    I2b,
    /* boolean */
    Bnot,
    /// True when any component is true.
    Bany,
    /// True when every component is true.
    Ball,
    /// `cond ? a : b`, component-wise; both arms evaluated.
    Bcsel,
    /* ES 3.00. `Iadd`, `Imul`, `Ineg`, `Ishl`, `Iand`, `Ior`, `Ixor`, `Ieq`
     * and `Ine` serve `uint` too (the bits are the same); these are where
     * signed and unsigned differ, and the two the language added. */
    /// Unsigned division. Like `Idiv` it is an IR op on both GPUs even where
    /// neither has one: each back end expands it its own way.
    Udiv,
    Umod,
    /// Logical shift right (`Ishr` is arithmetic).
    Ushr,
    Ult,
    Ule,
    Imin,
    Imax,
    Umin,
    Umax,
    /// Bitwise `~`.
    Inot,
    U2f,
    F2u,
}

impl AluOp {
    /// How many components the RESULT has, given the widest operand.
    ///
    /// Almost every operation here is component-wise, so the result is as
    /// wide as its operands. The exceptions are the two kinds that REDUCE:
    /// a dot product and the boolean folds, which read a vector and produce
    /// one lane.
    ///
    /// It lives on the opcode because that is where it is true. A back end
    /// that estimates the width instead has to walk forward through the
    /// program guessing, and over-estimates `a.xy + b.xy` as four whenever
    /// the producers were `vec4` -- the swizzle carries four lanes whatever
    /// the type said, which is the same ambiguity that has cost this
    /// compiler elsewhere: a swizzle answers for a width AND for a repeat.
    pub const fn result_comps(self, operand_comps: u8) -> u8 {
        use AluOp::*;
        match self {
            Fdot2 | Fdot3 | Fdot4 | Ball | Bany => 1,
            _ => operand_comps,
        }
    }

    /// How many sources this operation reads.
    ///
    /// The code generator uses it to check an instruction before selecting for
    /// it: an operation given the wrong number of sources would otherwise read
    /// a stale slot, which is a plausible value and therefore a wrong picture.
    pub fn arity(self) -> usize {
        use AluOp::*;
        match self {
            Fneg | Fabs | Fsign | Frcp | Frsq | Fsqrt | Fexp2 | Flog2 | Fsin | Fcos | Ffloor | Fceil | Ftrunc
            | Ffract | Fddx | Fddy | Ineg | B2f | B2i | F2i | I2f | F2b | I2b | Bnot | Bany | Ball | Inot | U2f
            | F2u => 1,
            Ffma | Bcsel => 3,
            _ => 2,
        }
    }
}

/// One instruction. Every arithmetic operation is component-wise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// Four float lanes, as raw bits so that the type stays `Eq`.
    ConstF32([u32; 4]),
    ConstI32([i32; 4]),
    ConstBool([bool; 4]),
    /// `AluOp` over its sources; unused source slots are ignored, per
    /// [`AluOp::arity`].
    Alu(AluOp, [Src; 3]),
    /// `comps` dwords of the stage's uniform block, starting at `dword_off`.
    LoadUniform {
        dword_off: u16,
        comps: u8,
    },
    /// A dynamically indexed uniform: element `index` of an array of `len`
    /// elements `stride_dw` dwords apart, starting at `base_dw`.
    ///
    /// The index is clamped to `[0, len)`. ES 2.0 leaves an out-of-range index
    /// undefined, and reading elsewhere in the constant file — where another
    /// shader's uniforms or a sampler's properties sit — is not an acceptable
    /// "undefined".
    LoadUniformIdx {
        base_dw: u16,
        stride_dw: u16,
        len: u16,
        index: Src,
        comps: u8,
    },
    /// Read a shader input.
    LoadInput {
        slot: IoSlot,
        comps: u8,
    },
    /// Write a shader output.
    StoreOutput {
        slot: IoSlot,
        src: Src,
    },
    /// Read a slot; `index` for an array.
    LoadLocal {
        local: LocalId,
        index: Option<Src>,
        comps: u8,
    },
    /// Write a slot, under a write mask (bit 0 = x ... bit 3 = w).
    StoreLocal {
        local: LocalId,
        index: Option<Src>,
        src: Src,
        write_mask: u8,
    },
    /// Sample a texture (ir.md, "Textures").
    ///
    /// `coord` lanes: D2 xy; D3 and Cube xyz; D2Array xy and the layer in z,
    /// a float the back end rounds and clamps (ES 3.00 section 8.8). A
    /// shadow reference follows in z for D2 and in w for Cube and D2Array.
    /// `proj` (D2 and D3 only) puts the divisor in w, always. A `Fetch`
    /// coordinate is integer lanes of the base width.
    ///
    /// The value is four lanes of `ret` (F32, I32 or U32), or one F32 lane
    /// for a shadow look-up. `offset` is a constant, never on a Cube.
    Tex {
        dim: TexDim,
        shadow: bool,
        proj: bool,
        ret: ScalarType,
        sampler: u8,
        coord: Src,
        lod: TexLod,
        offset: Option<[i8; 3]>,
    },
    /// `textureSize(s, lod)`: an int vector, two lanes for D2 and Cube and
    /// three for D3 and D2Array. `lod` is one int lane.
    TexSize {
        sampler: u8,
        dim: TexDim,
        lod: Src,
    },
    /// `discard`; fragment only.
    Discard,
}

/// A run of [`Node`]s: `nodes[first .. first + len]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub first: u32,
    pub len: u32,
}

impl Range {
    pub const EMPTY: Range = Range { first: 0, len: 0 };
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The structure the parser kept: a tree, not a control-flow graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Node {
    Instr(InstrId),
    If {
        cond: Src,
        then_: Range,
        else_: Range,
    },
    Loop {
        body: Range,
    },
    Break,
    Continue,
    /// `return` from `main`.
    Return,
}

impl Op {
    /// The component count this instruction's own fields STATE, or `None`
    /// when they state none.
    ///
    /// The `Load*` ops and `Tex` carry the count they read, and the two
    /// kinds of ALU that REDUCE produce one lane whatever they read. An
    /// ordinary `Alu` states nothing: its width is the checker's type, which
    /// only the front end has, and which it records in [`Module::comps`].
    ///
    /// This is the same rule the lowering uses when it fills that table, and
    /// it lives here so there is one copy of it.
    pub const fn stated_comps(&self) -> Option<u8> {
        match *self {
            Op::LoadUniform { comps, .. }
            | Op::LoadUniformIdx { comps, .. }
            | Op::LoadInput { comps, .. }
            | Op::LoadLocal { comps, .. } => Some(comps),
            Op::Tex { shadow: true, .. } => Some(1),
            Op::Tex { .. } | Op::ConstF32(_) | Op::ConstI32(_) | Op::ConstBool(_) => Some(4),
            Op::TexSize { dim: TexDim::D2 | TexDim::Cube, .. } => Some(2),
            Op::TexSize { .. } => Some(3),
            Op::Alu(op, _) if op.result_comps(4) == 1 => Some(1),
            _ => None,
        }
    }

    /// One of GLSL ES 1.00's five look-ups as an `Op::Tex`: the inverse of
    /// [`Op::es100_tex`]. `extra` is the bias or the level for the two forms
    /// that take one, and ignored otherwise.
    pub const fn es100(kind: Es100Tex, sampler: u8, coord: Src, extra: Option<Src>) -> Op {
        let (dim, proj, lod) = match (kind, extra) {
            (Es100Tex::Tex2DBias, Some(b)) => (TexDim::D2, false, TexLod::Bias(b)),
            (Es100Tex::Tex2DLod, Some(l)) => (TexDim::D2, false, TexLod::Lod(l)),
            (Es100Tex::Tex2DProj, _) => (TexDim::D2, true, TexLod::Implicit),
            (Es100Tex::TexCube, _) => (TexDim::Cube, false, TexLod::Implicit),
            _ => (TexDim::D2, false, TexLod::Implicit),
        };
        Op::Tex { dim, shadow: false, proj, ret: ScalarType::F32, sampler, coord, lod, offset: None }
    }

    /// An `Op::Tex` that is one of GLSL ES 1.00's five look-ups, as that
    /// shape and its bias or level (`None` for the forms that take neither).
    /// `None` for every other operation and every other look-up.
    pub const fn es100_tex(&self) -> Option<(Es100Tex, Option<Src>)> {
        let Op::Tex { dim, shadow: false, proj, ret: ScalarType::F32, lod, offset: None, .. } = *self else {
            return None;
        };
        Some(match (dim, proj, lod) {
            (TexDim::D2, false, TexLod::Implicit) => (Es100Tex::Tex2D, None),
            (TexDim::D2, false, TexLod::Bias(b)) => (Es100Tex::Tex2DBias, Some(b)),
            (TexDim::D2, false, TexLod::Lod(l)) => (Es100Tex::Tex2DLod, Some(l)),
            (TexDim::D2, true, TexLod::Implicit) => (Es100Tex::Tex2DProj, None),
            (TexDim::Cube, false, TexLod::Implicit) => (Es100Tex::TexCube, None),
            _ => return None,
        })
    }

    /// Call `f` with every source this operation reads, of every kind.
    ///
    /// Exhaustive on purpose -- no wildcard arm -- so an operation added
    /// later cannot be missed by a pass that counts uses. A count that
    /// missed one read gave a value to a store as if it had no other reader
    /// (select.rs `uses_of`, 2026-09-24). An `If` node's condition is a read
    /// too, and lives in the node tree, not here.
    pub fn for_each_src(&self, mut f: impl FnMut(Src)) {
        match *self {
            Op::ConstF32(_) | Op::ConstI32(_) | Op::ConstBool(_) => {}
            Op::Alu(o, srcs) => srcs[..o.arity()].iter().for_each(|s| f(*s)),
            Op::LoadUniform { .. } | Op::LoadInput { .. } | Op::Discard => {}
            Op::LoadUniformIdx { index, .. } => f(index),
            Op::StoreOutput { src, .. } => f(src),
            Op::LoadLocal { index, .. } => index.into_iter().for_each(&mut f),
            Op::StoreLocal { index, src, .. } => {
                index.into_iter().for_each(&mut f);
                f(src);
            }
            Op::Tex { .. } | Op::TexSize { .. } => {
                let (srcs, n) = self.tex_srcs();
                srcs[..n].iter().flatten().for_each(|s| f(*s));
            }
        }
    }

    /// Every source a texture operation reads -- the coordinate and the
    /// level's operands -- for the passes that walk uses. Empty for any
    /// other operation.
    pub fn tex_srcs(&self) -> ([Option<Src>; 4], usize) {
        let id = |v: ValueId| Some(Src::id(v));
        match *self {
            Op::Tex { coord, lod, .. } => match lod {
                TexLod::Implicit => ([Some(coord), None, None, None], 1),
                TexLod::Bias(x) | TexLod::Lod(x) | TexLod::Fetch(x) => ([Some(coord), Some(x), None, None], 2),
                TexLod::Grad { ddx, ddy } => ([Some(coord), id(ddx), id(ddy), None], 3),
            },
            Op::TexSize { lod, .. } => ([Some(lod), None, None, None], 1),
            _ => ([None; 4], 0),
        }
    }
}

/// Interpolation of a fragment input (`ShcVarying::interp`).
pub const INTERP_SMOOTH: u8 = 0;
/// See [`INTERP_SMOOTH`].
pub const INTERP_NOPERSPECTIVE: u8 = 1;
/// See [`INTERP_SMOOTH`].
pub const INTERP_FLAT: u8 = 2;
/// `centroid` (ES 3.00): smooth, sampled at the centroid. A back end without
/// the distinction treats it as [`INTERP_SMOOTH`] and says so.
pub const INTERP_CENTROID: u8 = 3;

/// What a declared input or output is, for reflection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoKind {
    Attribute,
    Varying,
    Position,
    PointSize,
    Colour,
    Depth,
}

/// One declared shader input or output: the reflection beside the IR.
#[derive(Clone, Copy, Debug)]
pub struct IoDecl<'a> {
    pub name: &'a str,
    pub kind: IoKind,
    /// The location of an attribute or varying; ignored for the built-ins.
    pub location: u8,
    /// 1..4.
    pub components: u8,
    /// [`INTERP_SMOOTH`] and friends. Fragment inputs only.
    pub interp: u8,
    /// The GL enum (`GL_FLOAT_VEC4`, ...), as `ShcAttrib::gl_type` holds it.
    pub gl_type: u16,
}

/// One uniform addressable by name: the source-level half of
/// [`ShcUniform`](crate::shaderc::ShcUniform).
#[derive(Clone, Copy, Debug)]
pub struct UniformDecl<'a> {
    pub name: &'a str,
    /// Dword offset in the stage's uniform block, or
    /// [`SHC_UNIFORM_NO_BLOCK`](crate::shaderc::SHC_UNIFORM_NO_BLOCK) for a
    /// sampler, which lives in a descriptor rather than in the block.
    pub dword_off: u16,
    pub dwords: u16,
    pub gl_type: u16,
    pub array_len: u8,
    /// Samplers: the texture unit this binding names.
    pub binding: u8,
    /// Elements per outer index of a flattened array-of-structs member
    /// (0 = 1).
    pub inner: u8,
    /// Samplers: whether the program statically references this one.
    ///
    /// The offline path drops declared-but-unused samplers using the
    /// compiler's post-dead-code list; the front end can only answer
    /// "statically referenced". So it reports every declared sampler with this
    /// flag and the back end applies the filter — neither side pretending the
    /// two questions are the same (`ir.md`, "One known asymmetry").
    pub statically_used: bool,
}

/// A slot values cross control flow through, instead of a phi node.
#[derive(Clone, Copy, Debug)]
pub struct VarDecl<'a> {
    /// The GLSL name, for diagnostics only; `""` for a compiler temporary.
    pub name: &'a str,
    pub ty: Type,
    /// 1 for a scalar slot; the element count for an array.
    pub array_len: u16,
}

/// One stage's IR: what the front end hands the code generator.
///
/// Every field borrows the scratch arena of the compile that built it.
#[derive(Clone, Copy, Debug)]
pub struct Module<'a> {
    /// [`SHC_STAGE_VERTEX`](crate::shaderc::SHC_STAGE_VERTEX) or
    /// [`SHC_STAGE_FRAGMENT`](crate::shaderc::SHC_STAGE_FRAGMENT).
    pub stage: u32,
    /// Every instruction; `ValueId(i)` is what `instrs[i]` produced.
    pub instrs: &'a [Op],
    /// How many components each instruction's value has, indexed by the
    /// same `ValueId` as [`Module::instrs`].
    ///
    /// Read it through [`Module::comps_of`], never by indexing: this is a
    /// property map keyed by value id, not a second ordering of the
    /// program, and the one array whose ORDER means something is `nodes`.
    ///
    /// The front end knows the type at every node and the back ends do not,
    /// so recording it here is the difference between a width and an
    /// estimate. An empty table means an older or hand-built module; the
    /// accessor answers 4 there, which is what a back end assumed anyway.
    pub comps: &'a [u8],
    /// Every node of every block, in one array; a [`Range`] indexes it.
    pub nodes: &'a [Node],
    /// `main`'s body.
    pub body: Range,
    pub locals: &'a [VarDecl<'a>],
    /// Vertex attributes, or fragment inputs, in ascending location.
    pub inputs: &'a [IoDecl<'a>],
    /// Vertex outputs, or the fragment colour.
    pub outputs: &'a [IoDecl<'a>],
    /// Block members in declaration order, then the samplers. The order is
    /// load-bearing: `glGetActiveUniform` indices follow it.
    pub uniforms: &'a [UniformDecl<'a>],
    /// The stage's uniform block size, in dwords.
    pub uniform_dwords: u16,
    pub uses_discard: bool,
    pub uses_front_facing: bool,
}

impl<'a> Module<'a> {
    /// An empty module for `stage`: what a builder starts from.
    pub const fn empty(stage: u32) -> Module<'a> {
        Module {
            stage,
            instrs: &[],
            comps: &[],
            nodes: &[],
            body: Range::EMPTY,
            locals: &[],
            inputs: &[],
            outputs: &[],
            uniforms: &[],
            uniform_dwords: 0,
            uses_discard: false,
            uses_front_facing: false,
        }
    }

    /// The nodes of `r`, or `&[]` when it runs past the array.
    ///
    /// Out of range reads as empty rather than panicking: the IR arrives from
    /// another module, and a code generator that indexed it blindly would turn
    /// a front-end defect into a crash inside a GL call.
    /// How many components the value `id` has: 1 to 4.
    ///
    /// 4 when the module carries no width table, which is what a back end
    /// assumed before there was one -- an over-estimate, never a wrong
    /// narrow answer.
    pub fn comps_of(&self, id: ValueId) -> u8 {
        if let Some(&c) = self.comps.get(id.0 as usize) {
            if (1..=4).contains(&c) {
                return c;
            }
        }
        /* No recorded width: ask the instruction. A `LoadLocal` states the
         * count it reads and a reduction states 1, both exactly, so IR built
         * by hand -- a back end's unit test, with no table at all -- gets a
         * real answer for those rather than the blanket 4. */
        match self.instrs.get(id.0 as usize).and_then(Op::stated_comps) {
            Some(c) if (1..=4).contains(&c) => c,
            _ => 4,
        }
    }

    pub fn nodes_of(&self, r: Range) -> &'a [Node] {
        let first = r.first as usize;
        let end = first.saturating_add(r.len as usize);
        self.nodes.get(first..end).unwrap_or(&[])
    }

    /// The instruction `id` names, or `None` when it is out of range.
    pub fn instr(&self, id: InstrId) -> Option<&'a Op> {
        self.instrs.get(id.0 as usize)
    }

    /// The instruction that produced `v`, or `None` when it is out of range.
    pub fn def(&self, v: ValueId) -> Option<&'a Op> {
        self.instrs.get(v.0 as usize)
    }

    /// The w component of `gl_Position` when a vertex shader writes it as one
    /// compile-time constant on every path, as its bits; `None` otherwise.
    ///
    /// Strict on purpose: the last top-level `StoreOutput` of the position
    /// must come before any top-level `return`, no `if` or loop may store the
    /// position or return, and its w must resolve to a `ConstF32` lane --
    /// directly or through a local whose lane was last stored, at top level,
    /// with such a constant (the form `vec4(p, 0.0, 1.0)` lowers to). A lane
    /// an `if` or a loop may have stored, an indexed store, a uniform, an
    /// attribute, arithmetic: all `None`. A constant W makes every
    /// perspective-interpolated varying affine in screen space, which is
    /// what lets the PowerVR fragment program keep the TPU's own level
    /// (`shaderc::pvr_proj_lod`).
    pub fn constant_position_w(&self) -> Option<u32> {
        let top = self.nodes_of(self.body);
        let mut last = None;
        for (i, n) in top.iter().enumerate() {
            match *n {
                Node::Instr(id) => {
                    if let Some(Op::StoreOutput { slot: IoSlot::Position, src }) = self.instr(id) {
                        last = Some((i, *src));
                    }
                }
                Node::Return => break,
                Node::If { then_, else_, .. } => {
                    if self.subtree_writes_position_or_returns(then_) || self.subtree_writes_position_or_returns(else_)
                    {
                        return None;
                    }
                }
                Node::Loop { body } => {
                    if self.subtree_writes_position_or_returns(body) {
                        return None;
                    }
                }
                Node::Break | Node::Continue => return None,
            }
        }
        let (_, src) = last?;
        self.constant_lane(src, 3, 0)
    }

    /// Does anything under `r` store the position, or return?
    fn subtree_writes_position_or_returns(&self, r: Range) -> bool {
        self.nodes_of(r).iter().any(|n| match *n {
            Node::Instr(id) => matches!(self.instr(id), Some(Op::StoreOutput { slot: IoSlot::Position, .. })),
            Node::Return => true,
            Node::If { then_, else_, .. } => {
                self.subtree_writes_position_or_returns(then_) || self.subtree_writes_position_or_returns(else_)
            }
            Node::Loop { body } => self.subtree_writes_position_or_returns(body),
            Node::Break | Node::Continue => false,
        })
    }

    /// Does anything under `r` store to `local` (any lane in `mask`, or
    /// through an index)?
    fn subtree_stores_local(&self, r: Range, local: LocalId, mask: u8) -> bool {
        self.nodes_of(r).iter().any(|n| match *n {
            Node::Instr(id) => matches!(
                self.instr(id),
                Some(Op::StoreLocal { local: l, index, write_mask, .. })
                    if *l == local && (index.is_some() || write_mask & mask != 0)
            ),
            Node::If { then_, else_, .. } => {
                self.subtree_stores_local(then_, local, mask) || self.subtree_stores_local(else_, local, mask)
            }
            Node::Loop { body } => self.subtree_stores_local(body, local, mask),
            Node::Return | Node::Break | Node::Continue => false,
        })
    }

    /// Lane `lane` of `s` as a constant's bits, read at top level (see
    /// [`Module::constant_position_w`]). `depth` bounds the walk through
    /// locals.
    fn constant_lane(&self, s: Src, lane: usize, depth: u32) -> Option<u32> {
        if depth > 16 {
            return None;
        }
        let pick = usize::from(*s.swizzle.get(lane)?) % 4;
        match *self.def(s.val)? {
            Op::ConstF32(v) => Some(v[pick]),
            Op::LoadLocal { local, index: None, .. } => {
                let top = self.nodes_of(self.body);
                /* the load itself must be at top level */
                let at = top.iter().position(|n| matches!(n, Node::Instr(id) if id.0 == s.val.0))?;
                let bit = 1u8 << pick;
                let mut value = None;
                for n in &top[..at] {
                    match *n {
                        Node::Instr(id) => {
                            if let Some(Op::StoreLocal { local: l, index, src, write_mask }) = self.instr(id) {
                                if *l != local {
                                    continue;
                                }
                                if index.is_some() {
                                    value = None;
                                } else if write_mask & bit != 0 {
                                    value = self.constant_lane(*src, pick, depth + 1);
                                }
                            }
                        }
                        Node::If { then_, else_, .. } => {
                            if self.subtree_stores_local(then_, local, bit)
                                || self.subtree_stores_local(else_, local, bit)
                            {
                                value = None;
                            }
                        }
                        Node::Loop { body } => {
                            if self.subtree_stores_local(body, local, bit) {
                                value = None;
                            }
                        }
                        Node::Return | Node::Break | Node::Continue => return None,
                    }
                }
                value
            }
            _ => None,
        }
    }

    /// The declared output of `kind`, if the shader has one.
    pub fn output_of_kind(&self, kind: IoKind) -> Option<&IoDecl<'a>> {
        self.outputs.iter().find(|o| o.kind == kind)
    }

    /// How many `f32` varying components a fragment shader reads in one
    /// interpolation class.
    ///
    /// PowerVR only: it sizes the USC coefficient allocation
    /// (`driver/docs/gpu-recipes/shader_toolchain.md`). The GC7000 has no
    /// coefficient registers — it wires the stages through `ShcViv`'s register
    /// lists — so the number means nothing there and is not compiled.
    #[cfg(not(feature = "gpu-vivante"))]
    pub fn varying_components(&self, interp: u8) -> u32 {
        self.inputs
            .iter()
            .filter(|i| i.kind == IoKind::Varying && i.interp == interp)
            .map(|i| i.components as u32)
            .sum()
    }

    /// True when the fragment shader needs the `1/w` iterator at `cf0..3`: any
    /// perspective-correct varying pulls it in, a shader with only flat inputs
    /// does not. This is `ShcShader::uses_w`, an allocation fact, which is why
    /// the front end does not report it. PowerVR only, as above.
    #[cfg(not(feature = "gpu-vivante"))]
    pub fn needs_w_iterator(&self) -> bool {
        self.stage == crate::shaderc::SHC_STAGE_FRAGMENT && self.varying_components(INTERP_SMOOTH) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "gpu-vivante"))]
    use crate::shaderc::SHC_STAGE_FRAGMENT;
    use crate::shaderc::SHC_STAGE_VERTEX;

    /// IR v1's texture shape names its gradients by `ValueId` so that an
    /// `Op` stays 32 bytes: two `Src`s would make it 40, on every
    /// instruction of every module (ir.md, "Textures"). IR v0's was 28.
    #[test]
    fn an_op_is_32_bytes() {
        assert_eq!(core::mem::size_of::<Op>(), 32);
    }

    /// The ES 1.00 look-ups round-trip through IR v1's fields, and nothing
    /// else claims to be one of them.
    #[test]
    fn the_es100_look_ups_map_one_to_one() {
        let c = Src::id(ValueId(1));
        let x = Some(Src::id(ValueId(2)));
        for (k, extra) in [
            (Es100Tex::Tex2D, None),
            (Es100Tex::Tex2DBias, x),
            (Es100Tex::Tex2DLod, x),
            (Es100Tex::Tex2DProj, None),
            (Es100Tex::TexCube, None),
        ] {
            assert_eq!(Op::es100(k, 3, c, extra).es100_tex(), Some((k, extra)), "{k:?}");
        }
        let arr = Op::Tex {
            dim: TexDim::D2Array,
            shadow: false,
            proj: false,
            ret: ScalarType::F32,
            sampler: 0,
            coord: c,
            lod: TexLod::Implicit,
            offset: None,
        };
        assert_eq!(arr.es100_tex(), None, "a 2D array look-up is not an ES 1.00 shape");
        assert_eq!(Op::Discard.es100_tex(), None);
    }

    #[test]
    fn a_range_past_the_node_array_reads_as_empty() {
        /* A front-end defect must not become a panic inside glCompileShader:
         * the code generator walks ranges the other module numbered. */
        let m = Module::empty(SHC_STAGE_VERTEX);
        assert!(m.nodes_of(Range { first: 0, len: 9 }).is_empty());
        assert!(m.instr(InstrId(3)).is_none());
        assert!(m.def(ValueId(3)).is_none());
    }

    #[test]
    fn nodes_of_returns_exactly_its_range() {
        let nodes = [Node::Break, Node::Continue, Node::Return];
        let m = Module { nodes: &nodes, ..Module::empty(SHC_STAGE_VERTEX) };
        let got = m.nodes_of(Range { first: 1, len: 2 });
        assert_eq!(got, [Node::Continue, Node::Return]);
    }

    #[test]
    fn an_instruction_that_states_its_width_answers_without_a_table() {
        /* IR BUILT BY HAND -- a back end's unit test -- has no `comps`
         * table, and before this every value in it read as 4. The ops that
         * carry the count they read state it exactly, so the fallback is a
         * fact, not a guess: `LoadLocal { comps: 2 }` IS two lanes, and a
         * reduction IS one, whoever built the module.
         *
         * An ordinary `Alu` still answers 4, because its width is the
         * checker's type and only the front end has that. */
        let instrs = [
            Op::LoadLocal { local: LocalId(0), index: None, comps: 2 },
            Op::Alu(AluOp::Fdot4, [Src::id(ValueId(0)); 3]),
            Op::Alu(AluOp::Fadd, [Src::id(ValueId(0)); 3]),
        ];
        let m = Module { instrs: &instrs, comps: &[], ..Module::empty(SHC_STAGE_VERTEX) };
        assert_eq!(m.comps_of(ValueId(0)), 2, "a load states the count it reads");
        assert_eq!(m.comps_of(ValueId(1)), 1, "a reduction states one lane");
        assert_eq!(m.comps_of(ValueId(2)), 4, "an ordinary ALU states nothing: over-estimate");
    }

    #[test]
    fn a_recorded_width_beats_the_instruction() {
        /* The table is the front end's answer and it is the better one: it
         * knows `a.xy + b.xy` is two lanes, which the instruction cannot
         * say. The fallback must never override it. */
        let instrs = [Op::Alu(AluOp::Fadd, [Src::id(ValueId(0)); 3])];
        let m = Module { instrs: &instrs, comps: &[2], ..Module::empty(SHC_STAGE_VERTEX) };
        assert_eq!(m.comps_of(ValueId(0)), 2);
    }

    #[test]
    fn a_value_names_the_instruction_that_produced_it() {
        /* The one-array rule: ValueId(i) is what instrs[i] produced. If that
         * ever stops holding, every source in the IR points at the wrong
         * definition and nothing says so. */
        let instrs = [Op::ConstF32([0; 4]), Op::Alu(AluOp::Fadd, [Src::id(ValueId(0)); 3])];
        let m = Module { instrs: &instrs, ..Module::empty(SHC_STAGE_VERTEX) };
        assert_eq!(m.def(ValueId(0)), Some(&Op::ConstF32([0; 4])));
        assert!(matches!(m.def(ValueId(1)), Some(Op::Alu(AluOp::Fadd, _))));
    }

    #[test]
    fn arity_matches_the_operations_that_read_three_and_one() {
        /* Checked against the two that are not binary: an operation given the
         * wrong source count reads a stale slot, which is a plausible value
         * and therefore a wrong picture rather than a crash. */
        assert_eq!(AluOp::Ffma.arity(), 3);
        assert_eq!(AluOp::Bcsel.arity(), 3);
        assert_eq!(AluOp::Frcp.arity(), 1);
        assert_eq!(AluOp::Bnot.arity(), 1);
        assert_eq!(AluOp::Fadd.arity(), 2);
        assert_eq!(AluOp::Fdot3.arity(), 2);
    }

    #[test]
    fn the_dot_products_are_in_the_kernel() {
        /* ir.md's one correction to the original proposal, and the reason is
         * measured: the GC7000 has DP2/DP3/DP4 natively. If these ever leave
         * the kernel, the front end starts expanding them into multiply-add
         * chains that the code generator would have to recognise again. */
        for (op, n) in [(AluOp::Fdot2, 2), (AluOp::Fdot3, 3), (AluOp::Fdot4, 4)] {
            assert_eq!(op.arity(), 2, "a dot takes two vectors: {op:?} ({n} components)");
        }
    }

    #[test]
    fn a_swizzle_is_carried_on_every_source() {
        /* Decision 1: the vector form is what is stored. A source without a
         * swizzle would be a scalar IR wearing a vector's name. */
        let s = Src::id(ValueId(4));
        assert_eq!(s.swizzle, SWIZZLE_IDENTITY);
        assert_eq!(Src::splat(ValueId(4)).swizzle, SWIZZLE_SPLAT0);
    }

    #[test]
    fn io_slots_name_attributes_and_varyings_by_location() {
        /* The hardware and the offline table both key them by location, and
         * link() pairs the stages by name separately. A slot that carried a
         * declaration index instead would renumber whenever the front end
         * reordered its declarations. */
        assert_eq!(IoSlot::Attrib(2), IoSlot::Attrib(2));
        assert_ne!(IoSlot::Varying(0), IoSlot::Attrib(0));
        assert_ne!(IoSlot::FragColor(0), IoSlot::Position);
    }

    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn the_w_iterator_follows_the_interpolation_class_not_the_varying_count() {
        let flat = [IoDecl {
            name: "v_id",
            kind: IoKind::Varying,
            location: 0,
            components: 1,
            interp: INTERP_FLAT,
            gl_type: 0,
        }];
        let m = Module { inputs: &flat, ..Module::empty(SHC_STAGE_FRAGMENT) };
        assert!(!m.needs_w_iterator(), "flat inputs alone need no w iterator");

        let smooth = [IoDecl {
            name: "v_uv",
            kind: IoKind::Varying,
            location: 1,
            components: 2,
            interp: INTERP_SMOOTH,
            gl_type: 0,
        }];
        let m = Module { inputs: &smooth, ..Module::empty(SHC_STAGE_FRAGMENT) };
        assert!(m.needs_w_iterator(), "one perspective input is enough");
        assert_eq!(m.varying_components(INTERP_SMOOTH), 2);
    }
}
