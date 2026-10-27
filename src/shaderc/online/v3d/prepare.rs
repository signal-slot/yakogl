// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The first step of the V3D chain: one stage's IR to a scalar program that
//! holds only what the outputs need.
//!
//! The QPU is scalar per lane (16 lanes of SIMD, each a scalar machine), so
//! the IR's vectors are taken apart here, as `powervr/select.rs` does for the
//! USC: every IR value becomes up to four [`S`] scalars, a source names one
//! component through its swizzle, and a `Local` slot is a per-lane record of
//! the scalar last stored to it (the IR has no control flow in this cut, so a
//! slot is just a name for the most recent store).
//!
//! Then everything no output reaches is dropped. That is not an optimisation:
//! the V3D's I/O is DEFINED by what is live. A vertex attribute the program
//! does not read has no attribute record, a varying the fragment shader does
//! not read has no VPM row, and the coordinate shader is the vertex shader
//! with every varying dead (`v3d-backend.md`, "What is different").
//!
//! # What this cut prepares, and what it refuses
//!
//! Straight-line code only: `ConstF32`, `LoadUniform`, `Fadd`, `Fmul`,
//! `LoadInput` of a
//! vertex attribute or a smooth varying, `StoreOutput` of `gl_Position`, a
//! varying or `gl_FragColor`, and `LoadLocal`/`StoreLocal` without an index.
//! Everything else in the IR kernel comes back as [`PrepError::NotYet`] with
//! the operation's name, never as code: an operation this file does not
//! prepare cannot reach selection by accident.

use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::fold::fold_float;
use crate::shaderc::online::ir::{self, AluOp, IoKind, IoSlot, Module, Node, Op};
use crate::shaderc::online::powervr::unroll::Trip;
use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// A scalar of the prepared program: `ops[S.0]` computes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct S(pub u32);

/// Render targets the V3D 4.2 TLB takes colour for (RT 0..3:
/// `TILE_RENDERING_MODE_CFG_COLOR` holds four), and so the colour outputs a
/// fragment shader may write.
pub const MAX_RTS: usize = 4;

/// What a look-up's lanes are ([`SOp::TexLane`]): floats, or the raw
/// integers of an `isampler` (signed) or `usampler` (unsigned). The TMU's
/// 16-bit output holds two of either a word (f16, or 16-bit integers of the
/// texture's signedness); its 32-bit output one, as it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TexRet {
    Float,
    Uint,
    Sint,
}

/// One scalar operation. The operands are earlier scalars.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SOp {
    /// A 32-bit constant (the bits of an `f32`).
    Const(u32),
    /// Component `comp` of the vertex attribute at `loc`.
    Attr { loc: u8, comp: u8 },
    /// `gl_VertexID` (vertex stage, one int lane): the vertex fetch writes
    /// it into the VPM input segment ahead of the attributes when the shader
    /// record says the stage reads it, and the stage reads it with
    /// LDVPMV_IN (Mesa ntq_setup_vs_inputs, nir_to_vir.c:2402-2421; the
    /// load, :3709-3711).
    VertexId,
    /// `gl_InstanceID` (vertex stage, one int lane): the VPM input row ahead
    /// of the vertex ID when the shader record says the stage reads it
    /// (ntq_setup_vs_inputs, nir_to_vir.c:2402-2421). The driver draws one
    /// instance, so the row holds 0.
    InstanceId,
    /// Component `comp` of the fragment shader's smooth varying at `loc`
    /// (the FRAGMENT shader's location; the chain maps it to the vertex
    /// shader's by name).
    Varying { loc: u8, comp: u8 },
    /// Dword `.0` of the stage's uniform block (the IR's `dword_off` plus
    /// the lane).
    Uniform(u16),
    /// A kernel operation of [`SELECTED`] on `op.arity()` operands (the
    /// rest are unused and hold `S(0)`).
    Alu(AluOp, [S; 3]),
    /// `x - y`: an `Fadd` of a negation, rewritten (Walk::push). The V3D
    /// has FSUB, and x + (-y) is x - y exactly (IEEE 754 defines
    /// subtraction as the addition of the negation), so the XOR with a
    /// sign-bit uniform that the negation costs is gone (Mesa gets the same
    /// from nir_opt_algebraic before its back end).
    Fsub([S; 2]),
    /// A lane of a local no store reached: GLSL leaves its value undefined
    /// (ES 1.00 section 4.1: a variable read before it is written). Kept
    /// only as long as something live reads it; one that survives dead-code
    /// removal is refused ([`UNDEFINED_READ`]), never given a value.
    Undef,
    /// A 2D look-up of the texture at sampler binding `sampler` (the
    /// reflection binding, what P0/P1's bits 24..31 carry): `srcs` are s and
    /// t, and the bias when `bias`, or the explicit level when `lod` (never
    /// both). `cube`: r follows t. `layer`: a 2D-array look-up whose layer
    /// (the face of a cube record, `cube_noseam`) follows t, written to TMUI;
    /// never with `cube`. Its result is read through [`SOp::TexLane`].
    Tex { sampler: u8, srcs: [S; 4], cube: bool, layer: bool, bias: bool, lod: bool },
    /// Lane `lane` (r, g, b, a) of the look-up `tex[0]`, of the sampler's
    /// result type `ret` (a float, or an integer sampler's raw integer).
    TexLane { tex: [S; 1], lane: u8, ret: TexRet },
    /// The width (`lane` 0) or height (1) of the base level of the texture at
    /// sampler binding `sampler`, an integer: a word of the uniform stream
    /// the driver fills (Mesa's QUNIFORM_TEXTURE_WIDTH / _HEIGHT,
    /// get_texture_size, v3d_uniforms.c:50-90). textureSize minifies it.
    TexSizeWord { sampler: u8, lane: u8 },
    /// Lane `lane` (x, y, z, w) of `gl_FragCoord`, a fragment input: x and y
    /// the pixel centre in window coordinates (y lower left, through the
    /// driver's WPOS transform), z the depth, w 1 / the clip w
    /// (select.rs, "gl_FragCoord").
    FragCoord { lane: u8 },
    /// Lane `lane` (s, t) of `gl_PointCoord`, a fragment input read from the
    /// implicit point varyings (select.rs select_fragment).
    PointCoord { lane: u8 },
    /// `gl_FrontFacing`, a fragment input: ~0 for a front-facing primitive,
    /// 0 for a back-facing one (select.rs, from REVF).
    FrontFacing,
    /// A counted loop's carried value (docs/online-shaderc/v3d-backend.md,
    /// "Control flow, step 2"): one lane of a local the loop body writes. It
    /// takes `init[0]` before the loop and the matching [`SOp::PhiNext`] at
    /// the end of each pass; after the loop it holds the last pass's value.
    Phi { init: [S; 1] },
    /// The top of a counted loop's body: every operation between it and its
    /// [`SOp::LoopEnd`] runs once per pass.
    LoopHead,
    /// At the end of a pass: `ops[0]` (a [`SOp::Phi`]) takes `ops[1]`.
    PhiNext { ops: [S; 2] },
    /// The end of a counted loop: the body has run `count` times, or every
    /// lane's break flag is set (`exit[0]`, when `has_exit`), whichever is
    /// first. The unroller guards every store after a break, so a lane that
    /// broke changes nothing in the passes it still runs.
    LoopEnd { count: u32, exit: [S; 1], has_exit: bool },
    /// A uniform branch around a large `if` (docs/online-shaderc/
    /// v3d-backend.md, "Uniform branches"): the operations up to the
    /// matching [`SOp::IfElse`] run when `cond[0]` (a bool computed from
    /// uniforms and constants only, so the same in every fragment) holds;
    /// otherwise the program jumps to the else arm.
    IfBranch { cond: [S; 1] },
    /// The end of the then arm (a jump over the else arm) and the start of
    /// the else arm.
    IfElse,
    /// The end of the else arm. The [`SOp::IfMerge`]s right after it are the
    /// values the arms leave different.
    IfEnd,
    /// After an [`SOp::IfEnd`]: `ops[0]` where the then arm ran, `ops[1]`
    /// where the else arm ran (select.rs gives both one register, written at
    /// the end of each arm).
    IfMerge { ops: [S; 2] },
    /// A `break` of the innermost counted loop on a uniform condition, as a
    /// branch (docs/online-shaderc/v3d-backend.md, "Control flow, step 3"):
    /// where `cond[0]` is `when` (a bool computed from uniforms, constants
    /// and the phis of uniform locals, so the same in every fragment of a
    /// draw), the program jumps to the word after the loop's back edge, as
    /// Mesa's structured break does. Prepared only where every lane the
    /// loop carries still holds its phi ([`Walk::uniform_break`]), so after
    /// the loop the phis are the values on this path too.
    LoopBreak { cond: [S; 1], when: bool },
}

impl SOp {
    pub(super) fn operands(&self) -> &[S] {
        match self {
            SOp::Alu(op, s) => &s[..op.arity()],
            SOp::Fsub(s) => s,
            SOp::Tex { srcs, cube, layer, bias, lod, .. } => &srcs[..tex_operands(*cube || *layer, *bias || *lod)],
            SOp::TexLane { tex, .. } => tex,
            SOp::Phi { init } => init,
            SOp::PhiNext { ops } => ops,
            SOp::LoopEnd { exit, has_exit: true, .. } => exit,
            SOp::IfBranch { cond } => cond,
            SOp::IfMerge { ops } => ops,
            SOp::LoopBreak { cond, .. } => cond,
            _ => &[],
        }
    }
    /// An operation kept whatever reads it: a loop's structure, its breaks
    /// included. (A uniform branch's markers are kept only while something
    /// in the branch is, [`compact`].)
    fn is_effect(&self) -> bool {
        matches!(self, SOp::LoopHead | SOp::PhiNext { .. } | SOp::LoopEnd { .. } | SOp::LoopBreak { .. })
    }
}

/// A prepared stage: only live operations, in an order where every operand
/// comes before its user.
#[derive(Clone, Copy, Debug)]
pub struct Prepared<'a> {
    pub stage: u32,
    pub ops: &'a [SOp],
    /// Vertex shaders: `gl_Position`, x y z w.
    pub position: [S; 4],
    /// Vertex shaders: the varyings asked for (see [`prepare_vertex`]), in the
    /// order asked; `None` where the shader does not write that component.
    pub varyings: &'a [Option<S>],
    /// Fragment shaders: the colour of render target n (`gl_FragColor` /
    /// `gl_FragData[n]`, or the ES 3.00 output at location n), r g b a.
    pub color: [[S; 4]; MAX_RTS],
    /// Fragment shaders: bit n, the shader writes render target n (some
    /// lane of it). An RT not written is four zeros that select.rs does not
    /// write; with no bit the program writes only the pass-through Z (a
    /// depth pre-pass).
    pub rt_mask: u8,
    /// Fragment shaders: bit n, render target n's output is declared `int`
    /// or `uint` (any width: `ivec4`, `uvec2`, `int`, ...; an ES 3.00
    /// output's declaration, `IoDecl::gl_type`). Its TLB write carries the
    /// 32-bit values as they are (select.rs `tlb_config_32b`, Mesa's
    /// TLB_TYPE_F32_COLOR, the one 32-bit type of V3D 4.2) where a float
    /// output's is two f16 pairs.
    pub rt_int: u8,
    /// Fragment shaders: the declared components of render target n's
    /// output (1..4; 4 for `gl_FragColor` and for an RT with no
    /// declaration), the words an integer write takes.
    pub rt_comps: [u8; MAX_RTS],
    /// Fragment shaders: the bool (~0 / 0) that holds in the lanes a
    /// `discard` ran in; `None` when the shader never discards.
    pub discard: Option<S>,
    /// Vertex shaders: the gl_PointSize value, when the shader writes it.
    /// It is kept in the VPM output for every primitive (select.rs
    /// select_vertex): measured on the Pi 4, gl_test case 110.
    pub psiz: Option<S>,
}

/// Why a stage was not prepared. One variant per cause.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrepError {
    /// An IR construct this cut does not prepare, by name.
    NotYet(&'static str),
    /// A source names a value no instruction produced, or a component the
    /// value does not have.
    Undefined(ir::ValueId),
    /// A `LoadUniform` past the end of the stage's uniform block.
    UniformOutOfBlock,
    /// `prepare_vertex` given a fragment shader, or the other way round.
    WrongStage,
    OutOfScratch,
}

impl From<ArenaError> for PrepError {
    fn from(_: ArenaError) -> Self {
        PrepError::OutOfScratch
    }
}

/// The kernel operations this cut selects ([`super::select`] has an arm for
/// each); every other one is refused here by name.
pub const SELECTED: &[AluOp] = &[
    AluOp::Fddx,
    AluOp::Fddy,
    AluOp::Fadd,
    AluOp::Fmul,
    AluOp::Fneg,
    AluOp::Fabs,
    AluOp::Fmin,
    AluOp::Fmax,
    AluOp::Flt,
    AluOp::Fle,
    AluOp::Feq,
    AluOp::Fne,
    AluOp::Ilt,
    AluOp::Ile,
    AluOp::Bnot,
    AluOp::Bcsel,
    AluOp::Frcp,
    AluOp::Frsq,
    AluOp::Fsqrt,
    AluOp::Fexp2,
    AluOp::Flog2,
    AluOp::Fsin,
    AluOp::Fcos,
    AluOp::Ffloor,
    AluOp::Fceil,
    AluOp::Ffract,
    AluOp::Fsign,
    AluOp::Ieq,
    AluOp::Ine,
    AluOp::Iadd,
    AluOp::Iand,
    AluOp::Ior,
    AluOp::Ixor,
    AluOp::Ineg,
    AluOp::Imul,
    AluOp::F2i,
    AluOp::I2f,
    AluOp::B2f,
    AluOp::B2i,
    AluOp::I2b,
    AluOp::F2b,
    AluOp::U2f,
    AluOp::F2u,
    AluOp::Ftrunc,
    AluOp::Ishl,
    AluOp::Ishr,
    AluOp::Ushr,
    AluOp::Imin,
    AluOp::Imax,
    AluOp::Umin,
    AluOp::Umax,
    AluOp::Inot,
    AluOp::Ult,
    AluOp::Ule,
];

/// The refusal for an [`SOp::Undef`] that an output needs.
pub const UNDEFINED_READ: &str = "a read of a local no store reached (undefined in GLSL)";

/// The name an [`AluOp`] is refused under.
pub fn alu_name(op: AluOp) -> &'static str {
    use AluOp::*;
    match op {
        Fadd => "Fadd",
        Fmul => "Fmul",
        Ffma => "Ffma",
        Fmin => "Fmin",
        Fmax => "Fmax",
        Fneg => "Fneg",
        Fabs => "Fabs",
        Fsign => "Fsign",
        Frcp => "Frcp",
        Frsq => "Frsq",
        Fsqrt => "Fsqrt",
        Fexp2 => "Fexp2",
        Flog2 => "Flog2",
        Fsin => "Fsin",
        Fcos => "Fcos",
        Ffloor => "Ffloor",
        Fceil => "Fceil",
        Ftrunc => "Ftrunc",
        Ffract => "Ffract",
        Fddx => "Fddx",
        Fddy => "Fddy",
        Fdot2 => "Fdot2",
        Fdot3 => "Fdot3",
        Fdot4 => "Fdot4",
        Flt => "Flt",
        Fle => "Fle",
        Feq => "Feq",
        Fne => "Fne",
        Iadd => "Iadd",
        Imul => "Imul",
        Idiv => "Idiv",
        Imod => "Imod",
        Ineg => "Ineg",
        Ishl => "Ishl",
        Ishr => "Ishr",
        Iand => "Iand",
        Ior => "Ior",
        Ixor => "Ixor",
        Ilt => "Ilt",
        Ile => "Ile",
        Ieq => "Ieq",
        Ine => "Ine",
        B2f => "B2f",
        B2i => "B2i",
        F2i => "F2i",
        I2f => "I2f",
        F2b => "F2b",
        I2b => "I2b",
        Bnot => "Bnot",
        Bany => "Bany",
        Ball => "Ball",
        Bcsel => "Bcsel",
        Udiv => "Udiv",
        Umod => "Umod",
        Ushr => "Ushr",
        Ult => "Ult",
        Ule => "Ule",
        Imin => "Imin",
        Imax => "Imax",
        Umin => "Umin",
        Umax => "Umax",
        Inot => "Inot",
        U2f => "U2f",
        F2u => "F2u",
    }
}

fn slot_name(slot: IoSlot) -> &'static str {
    match slot {
        IoSlot::Attrib(_) => "attribute",
        IoSlot::Varying(_) => "varying",
        IoSlot::Position => "gl_Position",
        IoSlot::PointSize => "gl_PointSize",
        IoSlot::FragColor(0) => "gl_FragColor",
        IoSlot::FragColor(_) => "gl_FragData[n > 3]",
        IoSlot::FragDepth => "gl_FragDepth",
        IoSlot::FragCoord => "gl_FragCoord",
        IoSlot::FrontFacing => "gl_FrontFacing",
        IoSlot::PointCoord => "gl_PointCoord",
        IoSlot::VertexId => "gl_VertexID",
        IoSlot::InstanceId => "gl_InstanceID",
    }
}

/// The walk: the IR's values as per-lane scalars, and the stores.
/// [`Walk::cube_face`]'s result: the face's (s, t) in [0, 1], the face
/// (the layer, a float) and the major axis (`is_x`; else `is_y`; else z),
/// booleans as ~0 / 0.
struct CubeFace {
    s: S,
    t: S,
    face: S,
    is_x: S,
    is_y: S,
}

struct Walk<'m, 'a> {
    m: &'m Module<'m>,
    ops: AVec<'a, SOp>,
    /// Per IR value, its four lanes (`None`: that lane was never computed).
    lanes: &'a mut [[Option<S>; 4]],
    locals: &'a mut [[Option<S>; 4]],
    position: [Option<S>; 4],
    color: [[Option<S>; 4]; MAX_RTS],
    /// Vertex shaders: every varying component stored, as (location,
    /// component, value); a later store to the same component replaces it.
    stored: AVec<'a, (u8, u8, S)>,
    /// The bool that holds in the lanes a `discard` ran in; `None`: none
    /// has (which a merge reads as false, never as undefined).
    discard: Option<S>,
    /// Vertex shaders: gl_PointSize as written so far (merged after an
    /// `if` as gl_Position is).
    psiz: Option<S>,
    /// The loops the unroller kept, each with its pass count (`Trip`).
    trips: &'m [Trip],
    /// [`crate::shaderc::LodKey::cube_noseam`]: the cube bindings whose
    /// look-ups are emulated per face (0 = none: the hardware's cube look-up).
    cube_noseam: u32,
    /// The break flags of the counted loops the walk is inside that have
    /// one, outermost first.
    enclosing_exits: AVec<'a, ir::LocalId>,
    /// Beside `ops`: the innermost region open when each op was pushed (the
    /// index of a counted loop's LoopHead, or of a uniform branch arm's
    /// IfBranch / IfElse), or NO_LOOP at the top level. What
    /// [`Walk::push`]'s common-subexpression reuse may take.
    loop_of: AVec<'a, u32>,
    /// The regions open now, outermost first.
    open_loops: AVec<'a, u32>,
    /// Per local: every value it holds is the same in every fragment
    /// ([`uniform_locals`]).
    uniform_local: &'a [bool],
    /// Each counted loop's phis and the local each carries: a phi of a
    /// uniform local is uniform ([`Walk::is_uniform`]).
    phi_local: AVec<'a, (S, u32)>,
    /// The break flags of the open counted loops, each as its pass starts
    /// (the loop's phi of it): clear in every pass that runs, because the
    /// back edge leaves when the flag is set (LoopEnd's exit). Only for a
    /// flag the unroller made uniform (`Trip::divergent` false): a flag set
    /// in some lanes only leaves the loop running for the others.
    clear_at_pass_start: AVec<'a, S>,
    /// The counted loop whose uniform `break`s may be branches
    /// ([`Walk::uniform_break`]); `None` outside one, inside an arm of an
    /// `if` (a branch arm may be skipped, a select arm runs on every path),
    /// and inside a kept loop nested in another.
    break_to: Option<BreakTo<'a>>,
}

/// What a `break` of a counted loop checks before it becomes a branch
/// ([`SOp::LoopBreak`]).
#[derive(Clone, Copy)]
struct BreakTo<'a> {
    /// The loop's break flag (`Trip::exit`), and its phi: clear in every
    /// pass that runs (`clear_at_pass_start`).
    flag: ir::LocalId,
    phi: S,
    /// Every (phi, local, lane) the loop carries.
    carried: &'a [(S, u32, usize)],
    /// The discard flag as each pass starts.
    discard: Option<S>,
}

/// Per local of `m`: every value it holds is the same in every fragment of
/// a draw, because every store to it writes a value made of uniforms,
/// constants and such locals only, outside any `if` on a condition that is
/// not. The largest such set: start from "every local" and drop a local at
/// each store that breaks the rule until nothing changes. A kept loop's
/// counter and its break flag are uniform this way when its bound is
/// (femtovg filter_image: `i` from 1.0 by 1.0, the break on `i >=
/// sampleCount` of a uniform), and its phis with them.
fn uniform_locals<'a>(m: &ir::Module<'_>, a: &Arena<'a>) -> Result<&'a [bool], PrepError> {
    let uni = a.alloc_filled(m.locals.len(), true)?;
    /* per value, this pass: 0 not asked, 1 uniform, 2 not */
    let memo = a.alloc_filled(m.instrs.len(), 0u8)?;
    fn value(m: &ir::Module<'_>, id: ir::ValueId, uni: &[bool], memo: &mut [u8], depth: u32) -> bool {
        let i = id.0 as usize;
        match memo.get(i).copied() {
            Some(1) => return true,
            Some(2) | None => return false,
            _ => {}
        }
        /* deep chains are answered "not uniform", which is always safe */
        let ok = depth <= 256
            && match m.def(id) {
                Some(Op::ConstF32(_) | Op::ConstI32(_) | Op::ConstBool(_) | Op::LoadUniform { .. }) => true,
                Some(Op::Alu(op, srcs)) => srcs[..op.arity()].iter().all(|s| value(m, s.val, uni, memo, depth + 1)),
                Some(Op::LoadLocal { local, index: None, .. }) => uni.get(local.0 as usize).copied().unwrap_or(false),
                _ => false,
            };
        memo[i] = if ok { 1 } else { 2 };
        ok
    }
    fn walk(m: &ir::Module<'_>, r: ir::Range, ctl: bool, uni: &mut [bool], memo: &mut [u8], changed: &mut bool) {
        for n in m.nodes_of(r) {
            match *n {
                Node::Instr(id) => {
                    if let Some(Op::StoreLocal { local, index, src, .. }) = m.instr(id) {
                        let ok = ctl && index.is_none() && value(m, src.val, uni, memo, 0);
                        if let Some(u) = uni.get_mut(local.0 as usize) {
                            if *u && !ok {
                                *u = false;
                                *changed = true;
                            }
                        }
                    }
                }
                Node::If { cond, then_, else_ } => {
                    let c = ctl && value(m, cond.val, uni, memo, 0);
                    walk(m, then_, c, uni, memo, changed);
                    walk(m, else_, c, uni, memo, changed);
                }
                Node::Loop { body } => walk(m, body, ctl, uni, memo, changed),
                Node::Break | Node::Continue | Node::Return => {}
            }
        }
    }
    loop {
        let mut changed = false;
        memo.fill(0);
        walk(m, m.body, true, uni, memo, &mut changed);
        if !changed {
            break;
        }
    }
    Ok(uni)
}

/// [`Walk::loop_of`] for an op pushed outside every counted loop.
const NO_LOOP: u32 = u32::MAX;

/// The ALU operations above which a uniform `if` is a branch rather than
/// selects: Mesa's (nir_opt_peephole_select `.limit = 24` for v3d,
/// nir_to_vir.c:2261-2266), counted over prepared scalar operations.
const BRANCH_LIMIT: usize = 24;

impl<'m, 'a> Walk<'m, 'a> {
    fn push(&mut self, a: &Arena<'a>, op: SOp) -> Result<S, PrepError> {
        /* Integer arithmetic on constants is a constant, and a select on a
         * constant is its arm (see fold). */
        if let SOp::Alu(AluOp::Bcsel, [c, t, e]) = op {
            if let Some(v) = self.constant(c) {
                return Ok(if v != 0 { t } else { e });
            }
        }
        if let Some(v) = self.fold(op) {
            return self.push(a, SOp::Const(v));
        }
        if let SOp::Alu(alu, s) = op {
            if let Some(r) = self.algebraic(a, alu, s)? {
                return Ok(r);
            }
        }
        /* One constant of each value: vec4(v, 1.0, 1.0) should not load 1.0
         * twice; one read of each gl_FragCoord lane, of gl_VertexID, and of
         * each varying and attribute component (fixed inputs, like a
         * uniform; select reads each (location, component) once anyway), so
         * that the common-subexpression reuse below sees two reads of `v.y`
         * as one value. */
        if position_free(&op) {
            if let Some(i) = self.ops.as_slice().iter().position(|o| *o == op) {
                return Ok(S(i as u32));
            }
        }
        /* These select.rs computes where they stand (FXCD, FYCD, RECIP of
         * W, REVF), not at the top or at each use: one is reused under the
         * common-subexpression rule below, from an open region only. (An
         * attribute and gl_VertexID are read where they first stand too, in a
         * vertex shader, which has no uniform branches.) */
        let positional = matches!(op, SOp::FragCoord { .. } | SOp::FrontFacing);
        /* Common subexpressions (Mesa's nir_opt_cse): an arithmetic op
         * already computed on the same operands is that value. Every op here
         * is pure, so the one rule is where the earlier op may be read from:
         * one pushed at the top level, or inside a counted loop that is still
         * open (a value from a loop that has ended belongs to its last pass
         * only, and one from another loop's body is not computed on this
         * path). FemtoVG's scissorMask computes scissorMat * vec3(p, 1.0) in
         * both arms of its `if`; if-converted, both copies ran (2026-09-26). */
        if positional || matches!(op, SOp::Alu(..) | SOp::Fsub(_)) {
            let open = self.open_loops.as_slice();
            let reusable = |t: u32| t == NO_LOOP || open.contains(&t);
            let tags = self.loop_of.as_slice();
            if let Some(i) = self.ops.as_slice().iter().zip(tags).position(|(o, &t)| *o == op && reusable(t)) {
                return Ok(S(i as u32));
            }
        }
        let tag = self.open_loops.as_slice().last().copied().unwrap_or(NO_LOOP);
        self.ops.push(a, op)?;
        self.loop_of.push(a, tag)?;
        let s = S(self.ops.len() as u32 - 1);
        /* The regions a later op may reuse this one from: a loop body, and
         * each arm of a uniform branch (a then-arm value is not computed on
         * the else path, nor after the `if` when the else arm ran). */
        match op {
            SOp::LoopHead | SOp::IfBranch { .. } => self.open_loops.push(a, s.0)?,
            SOp::IfElse => {
                self.open_loops.pop();
                self.open_loops.push(a, s.0)?;
            }
            SOp::LoopEnd { .. } | SOp::IfEnd => {
                self.open_loops.pop();
            }
            _ => {}
        }
        Ok(s)
    }

    /// Exact algebraic rewrites of a float operation, in the spirit of Mesa's
    /// nir_opt_algebraic before its back end (the femtovg programs ran 1.6x
    /// Mesa's length, 2026-09-26; `our_instruction_counts_are_pinned`):
    /// - `x * 1.0` is `x`, and `x * -1.0` is `-x`;
    /// - `-(-x)` is `x`, and `(-a) * (-b)` is `a * b`;
    /// - `a * (-b)` (or `(-a) * b`) is `-(a * b)`, so that an add can take
    ///   the negation;
    /// - `x + (-y)` (or `(-y) + x`) is `x - y` ([`SOp::Fsub`]);
    /// - `x + 0.0` and `0.0 + x` (and with -0.0) are `x`: the sign of a zero
    ///   result may differ, which GLSL ES 3.00 section 4.5.1 allows;
    /// - `min(x, x)` and `max(x, x)` are `x` (nir_opt_algebraic.py:984-985,
    ///   there `fcanonicalize(x)`). Not only shorter: FMIN and FMAX are told
    ///   apart by the order of their inputs (encode.rs add_pack), so FMAX of
    ///   two identical inputs has no encoding, and `max(a, a)` was refused
    ///   at scheduling (NotEncodable, ReadsBackAs) before this.
    ///
    /// Each is exact in IEEE 754 (a negation is exact; x * 1.0 is x); the
    /// one difference a board can see is that x * 1.0 flushes a subnormal x
    /// and x does not, which GLSL ES 3.00 section 4.5.1 allows either way.
    /// None when nothing applies.
    fn algebraic(&mut self, a: &Arena<'a>, alu: AluOp, s: [S; 3]) -> Result<Option<S>, PrepError> {
        const ONE: u32 = 0x3f80_0000;
        const MINUS_ONE: u32 = 0xbf80_0000;
        const ZERO: u32 = 0;
        const MINUS_ZERO: u32 = 0x8000_0000;
        let neg_of = |w: &Self, x: S| match w.ops.as_slice().get(x.0 as usize) {
            Some(&SOp::Alu(AluOp::Fneg, [y, _, _])) => Some(y),
            _ => None,
        };
        Ok(Some(match alu {
            AluOp::Fmul => {
                let (x, y) = (s[0], s[1]);
                match (self.constant(x), self.constant(y)) {
                    (_, Some(ONE)) => x,
                    (Some(ONE), _) => y,
                    (_, Some(MINUS_ONE)) => self.push(a, SOp::Alu(AluOp::Fneg, [x, S(0), S(0)]))?,
                    (Some(MINUS_ONE), _) => self.push(a, SOp::Alu(AluOp::Fneg, [y, S(0), S(0)]))?,
                    _ => match (neg_of(self, x), neg_of(self, y)) {
                        (Some(p), Some(q)) => self.push(a, SOp::Alu(AluOp::Fmul, [p, q, S(0)]))?,
                        (Some(p), None) => {
                            let m = self.push(a, SOp::Alu(AluOp::Fmul, [p, y, S(0)]))?;
                            self.push(a, SOp::Alu(AluOp::Fneg, [m, S(0), S(0)]))?
                        }
                        (None, Some(q)) => {
                            let m = self.push(a, SOp::Alu(AluOp::Fmul, [x, q, S(0)]))?;
                            self.push(a, SOp::Alu(AluOp::Fneg, [m, S(0), S(0)]))?
                        }
                        (None, None) => return Ok(None),
                    },
                }
            }
            AluOp::Fmin | AluOp::Fmax if s[0] == s[1] => s[0],
            AluOp::Fneg => match neg_of(self, s[0]) {
                Some(x) => x,
                None => return Ok(None),
            },
            /* x + 0.0 and 0.0 + x are x (nir_opt_algebraic.py:236-237,
             * `fadd(nsz)`): only the sign of a zero result can differ, and
             * GLSL ES 3.00 section 4.5.1 allows "positive and negative zeros
             * to be interchanged" (read from the spec text). -0.0 likewise;
             * x - 0.0 arrives here as x + (-0.0) (the negation folds). */
            AluOp::Fadd if matches!(self.constant(s[1]), Some(ZERO | MINUS_ZERO)) => s[0],
            AluOp::Fadd if matches!(self.constant(s[0]), Some(ZERO | MINUS_ZERO)) => s[1],
            AluOp::Fadd => match (neg_of(self, s[0]), neg_of(self, s[1])) {
                (_, Some(y)) => self.push(a, SOp::Fsub([s[0], y]))?,
                (Some(x), None) => self.push(a, SOp::Fsub([s[1], x]))?,
                (None, None) => return Ok(None),
            },
            _ => return Ok(None),
        }))
    }

    /// The value of `x` when it is a constant.
    fn constant(&self, x: S) -> Option<u32> {
        match self.ops.as_slice().get(x.0 as usize) {
            Some(&SOp::Const(v)) => Some(v),
            _ => None,
        }
    }

    /// An integer operation whose operands are all constants, evaluated as
    /// the IR defines it (eval.rs: wrapping two's-complement arithmetic,
    /// signed comparisons, a bool as ~0 or 0). The unroller leaves such
    /// operations behind: after it expands `for (i = 0; i < 16; i++)
    /// f(2 + i)`, the index `2 + i` is an add of two constants. Folding it,
    /// and the comparisons and selects that read it, collapses the select
    /// over every element that an index into an array becomes (dEQP
    /// light_amount's `u_lights[ndx]`: 3832 prepared operations at 16
    /// lights, 907 folded) to the one element read (Mesa folds these in
    /// NIR, nir_opt_constant_folding, before its back end sees them).
    ///
    /// Float operations on constants are folded too, where the result is
    /// exactly defined ([`fold_float`]). Mesa folds them the same way, and
    /// a GLSL `const` initializer is evaluated when compiling (dEQP's
    /// reference is exact): `int(sqrt(12.25) - 1.5)` computed on the QPU's
    /// SFU (rsqrt, then recip) gave 3.4999998 - 1.5, truncated to 1, not 2,
    /// and dEQP random all_features.fragment.53 took the other branch
    /// (the Pi 4 drew one constant colour where the reference has exp(a)).
    fn fold(&self, op: SOp) -> Option<u32> {
        let SOp::Alu(alu, s) = op else { return None };
        let x = self.constant(s[0])?;
        let y = if alu.arity() >= 2 { self.constant(s[1])? } else { 0 };
        let z = if alu.arity() >= 3 { self.constant(s[2])? } else { 0 };
        if let Some(v) = fold_float(alu, x, y, z) {
            return Some(v);
        }
        let b = |c: bool| if c { u32::MAX } else { 0 };
        let (xi, yi) = (x as i32, y as i32);
        Some(match alu {
            AluOp::Iadd => x.wrapping_add(y),
            AluOp::Imul => x.wrapping_mul(y),
            AluOp::Ineg => x.wrapping_neg(),
            AluOp::Iand => x & y,
            AluOp::Ior => x | y,
            AluOp::Ixor => x ^ y,
            AluOp::Ilt => b(xi < yi),
            AluOp::Ile => b(xi <= yi),
            AluOp::Ieq => b(x == y),
            AluOp::Ine => b(x != y),
            _ => return None,
        })
    }

    /// The face of a cube look-up and its coordinates, GL ES 2.0 3.7.5
    /// (Table 3.21), for the 2D-array emulation of a non-seamless cube
    /// ([`crate::shaderc::LodKey::cube_noseam`]). `abs` are |x|, |y|, |z| of
    /// the direction as given and `d` the direction divided by its major
    /// axis; the face is chosen on `abs` by dEQP's `selectCubeFace` rule
    /// (texref::select_cube_face), ties included. Returns s and t in [0, 1]
    /// and the face (+X -X +Y -Y +Z -Z = 0..5) as a float, the layer, and
    /// the major axis as two flags (x is it; else y is it; else z).
    fn cube_face(&mut self, a: &Arena<'a>, abs: [S; 3], d: [S; 3]) -> Result<CubeFace, PrepError> {
        let [ax, ay, az] = abs;
        let [x, y, z] = d;
        let k = |w: &mut Self, v: f32| w.push(a, SOp::Const(v.to_bits()));
        let (t_, f_) = (self.push(a, SOp::Const(!0))?, self.push(a, SOp::Const(0))?);
        let lt = |w: &mut Self, p: S, q: S| w.op2(a, AluOp::Flt, p, q);
        let le = |w: &mut Self, p: S, q: S| w.op2(a, AluOp::Fle, p, q);
        let eq = |w: &mut Self, p: S, q: S| w.op2(a, AluOp::Feq, p, q);
        /* strictly largest */
        let (c, d1) = (lt(self, ay, ax)?, lt(self, az, ax)?);
        let xs = self.op2(a, AluOp::Iand, c, d1)?;
        let (c, d1) = (lt(self, ax, ay)?, lt(self, az, ay)?);
        let ys = self.op2(a, AluOp::Iand, c, d1)?;
        let (c, d1) = (lt(self, ax, az)?, lt(self, ay, az)?);
        let zs = self.op2(a, AluOp::Iand, c, d1)?;
        /* equal components: x, y or z by dEQP's order of tests */
        let (exy, exz, eyz) = (eq(self, ax, ay)?, eq(self, ax, az)?, eq(self, ay, az)?);
        let (a_x, b_y) = (le(self, az, ax)?, lt(self, ay, ax)?);
        let inner = self.sel(a, eyz, b_y, t_)?;
        let inner = self.sel(a, exz, f_, inner)?;
        let tie_x = self.sel(a, exy, a_x, inner)?;
        let (c_y, d_y) = (lt(self, az, ay)?, le(self, ax, ay)?);
        let inner = self.sel(a, eyz, d_y, f_)?;
        let inner = self.sel(a, exz, c_y, inner)?;
        let tie_y = self.sel(a, exy, f_, inner)?;
        let inner = self.sel(a, zs, f_, tie_x)?;
        let inner = self.sel(a, ys, f_, inner)?;
        let is_x = self.sel(a, xs, t_, inner)?;
        let inner = self.sel(a, zs, f_, tie_y)?;
        let inner = self.sel(a, ys, t_, inner)?;
        let is_y = self.sel(a, xs, f_, inner)?;
        /* Table 3.21 */
        let zero = k(self, 0.0)?;
        let (nx, ny, nz) = (lt(self, x, zero)?, lt(self, y, zero)?, lt(self, z, zero)?);
        let (mx, my, mz) = (self.op1(a, AluOp::Fneg, x)?, self.op1(a, AluOp::Fneg, y)?, self.op1(a, AluOp::Fneg, z)?);
        let (f0, f1, f2) = (k(self, 0.0)?, k(self, 1.0)?, k(self, 2.0)?);
        let (f3, f4, f5) = (k(self, 3.0)?, k(self, 4.0)?, k(self, 5.0)?);
        let fx = self.sel(a, nx, f1, f0)?;
        let fy = self.sel(a, ny, f3, f2)?;
        let fz = self.sel(a, nz, f5, f4)?;
        let sx = self.sel(a, nx, z, mz)?;
        let sz = self.sel(a, nz, mx, x)?;
        let ty = self.sel(a, ny, mz, z)?;
        let pick = |w: &mut Self, vx: S, vy: S, vz: S| -> Result<S, PrepError> {
            let inner = w.sel(a, is_y, vy, vz)?;
            w.sel(a, is_x, vx, inner)
        };
        let face = pick(self, fx, fy, fz)?;
        let sc = pick(self, sx, x, sz)?;
        let tc = pick(self, my, ty, my)?;
        let half = k(self, 0.5)?;
        let s = self.op2(a, AluOp::Fmul, sc, half)?;
        let s = self.op2(a, AluOp::Fadd, s, half)?;
        let t = self.op2(a, AluOp::Fmul, tc, half)?;
        let t = self.op2(a, AluOp::Fadd, t, half)?;
        Ok(CubeFace { s, t, face, is_x, is_y })
    }

    /// The level of detail of a fragment-stage cube look-up on the face
    /// `f` selected, computed in the shader from the derivatives of the
    /// direction `d` (as given, before the normalisation; `inv` = 1 / its
    /// major axis' magnitude): dEQP's `computeCubeLodFromDerivates`
    /// (glsTextureTestUtil.cpp; the exact LOD of GL ES 2.0 3.7.7 for the
    /// face's projected coordinates), with the face size `width` read from
    /// the texture's own size word:
    ///
    /// du/dx = W/2 (dsc/dx ma - sc |dma/dx|) / ma^2 (likewise dv, dy),
    /// lambda = log2(max(|(du/dx, dv/dx)|, |(du/dy, dv/dy)|)) = 0.5 log2(max(px^2, py^2)).
    ///
    /// The TMU's own derivative of the per-face (s, t) cannot be used: in a
    /// quad whose four pixels do not all select one face (a helper pixel
    /// past the edge of a face-filling primitive, or a real seam) the
    /// difference across the face jump is meaningless and the level wrong
    /// (docs/v3d-noseamless-cube.md, "The level of detail"). The derivative
    /// of the direction is continuous across faces, so each pixel's level
    /// is its own face's, as in the reference. Clamped to [-16, 16]: a
    /// constant direction gives log2(0).
    fn cube_lod(&mut self, a: &Arena<'a>, sampler: u8, d: [S; 3], inv: S, f: &CubeFace) -> Result<S, PrepError> {
        let (is_x, is_y) = (f.is_x, f.is_y);
        /* (ma, sc, tc) of a vector: (x, z, y), (y, x, z), (z, x, y) by the
         * major axis (dEQP's maNdx / sNdx / tNdx; the signs do not matter
         * for the level, the formula takes |ma| and |dma|) */
        let axes = |w: &mut Self, [vx, vy, vz]: [S; 3]| -> Result<[S; 3], PrepError> {
            let mut out = [S(0); 3];
            for (o, (cx, cy, cz)) in out.iter_mut().zip([(vx, vy, vz), (vz, vx, vx), (vy, vz, vy)]) {
                let inner = w.sel(a, is_y, cy, cz)?;
                *o = w.sel(a, is_x, cx, inner)?;
            }
            Ok(out)
        };
        let [ma, sc, tc] = axes(self, d)?;
        let ma = self.op1(a, AluOp::Fabs, ma)?;
        let k = self.op2(a, AluOp::Fmul, inv, inv)?;
        /* |(du, dv)|^2 / (W/2)^2 along one screen direction, from that
         * direction's derivative of the direction vector */
        let len2 = |w: &mut Self, dd: [S; 3]| -> Result<S, PrepError> {
            let [mad, scd, tcd] = axes(w, dd)?;
            let mad = w.op1(a, AluOp::Fabs, mad)?;
            let mut sq = [S(0); 2];
            for (o, (c, cd)) in sq.iter_mut().zip([(sc, scd), (tc, tcd)]) {
                let p = w.op2(a, AluOp::Fmul, cd, ma)?;
                let q = w.op2(a, AluOp::Fmul, c, mad)?;
                let q = w.op1(a, AluOp::Fneg, q)?;
                let r = w.op2(a, AluOp::Fadd, p, q)?;
                let r = w.op2(a, AluOp::Fmul, r, k)?;
                *o = w.op2(a, AluOp::Fmul, r, r)?;
            }
            w.op2(a, AluOp::Fadd, sq[0], sq[1])
        };
        let mut dd = [[S(0); 3]; 2];
        for (row, op) in dd.iter_mut().zip([AluOp::Fddx, AluOp::Fddy]) {
            for (o, v) in row.iter_mut().zip(d) {
                *o = self.op1(a, op, v)?;
            }
        }
        let px = len2(self, dd[0])?;
        let py = len2(self, dd[1])?;
        let p2 = self.op2(a, AluOp::Fmax, px, py)?;
        /* (W / 2)^2: the face size from the texture's size word (a cube's
         * faces are square) */
        let w = self.push(a, SOp::TexSizeWord { sampler, lane: 0 })?;
        let w = self.op1(a, AluOp::U2f, w)?;
        let quarter = self.push(a, SOp::Const(0.25f32.to_bits()))?;
        let w2 = self.op2(a, AluOp::Fmul, w, w)?;
        let w2 = self.op2(a, AluOp::Fmul, w2, quarter)?;
        let rho2 = self.op2(a, AluOp::Fmul, p2, w2)?;
        let l2 = self.op1(a, AluOp::Flog2, rho2)?;
        let half = self.push(a, SOp::Const(0.5f32.to_bits()))?;
        self.op2(a, AluOp::Fmul, l2, half)
    }

    /// `l` clamped to [-16, 16].
    fn clamp_lod(&mut self, a: &Arena<'a>, l: S) -> Result<S, PrepError> {
        let lo = self.push(a, SOp::Const((-16.0f32).to_bits()))?;
        let hi = self.push(a, SOp::Const(16.0f32.to_bits()))?;
        let l = self.op2(a, AluOp::Fmax, l, lo)?;
        self.op2(a, AluOp::Fmin, l, hi)
    }
    /// `op(x, y)`, a kernel operation of two operands.
    fn op2(&mut self, a: &Arena<'a>, op: AluOp, x: S, y: S) -> Result<S, PrepError> {
        self.push(a, SOp::Alu(op, [x, y, S(0)]))
    }
    fn op1(&mut self, a: &Arena<'a>, op: AluOp, x: S) -> Result<S, PrepError> {
        self.push(a, SOp::Alu(op, [x, S(0), S(0)]))
    }
    /// `c ? t : e`.
    fn sel(&mut self, a: &Arena<'a>, c: S, t: S, e: S) -> Result<S, PrepError> {
        self.push(a, SOp::Alu(AluOp::Bcsel, [c, t, e]))
    }

    /// The high 32 bits of the unsigned 64-bit product (nir_lower_alu.c,
    /// umul_high under lower_mul_high: Hacker's Delight, Figure 8-2).
    fn umul_high(&mut self, a: &Arena<'a>, x: S, y: S) -> Result<S, PrepError> {
        let mask = self.push(a, SOp::Const(0xffff))?;
        let shift = self.push(a, SOp::Const(16))?;
        let xl = self.op2(a, AluOp::Iand, x, mask)?;
        let yl = self.op2(a, AluOp::Iand, y, mask)?;
        let xh = self.op2(a, AluOp::Ushr, x, shift)?;
        let yh = self.op2(a, AluOp::Ushr, y, shift)?;
        let lo = self.op2(a, AluOp::Imul, xl, yl)?;
        let m1 = self.op2(a, AluOp::Imul, xl, yh)?;
        let m2 = self.op2(a, AluOp::Imul, xh, yl)?;
        let hi = self.op2(a, AluOp::Imul, xh, yh)?;
        let lo_h = self.op2(a, AluOp::Ushr, lo, shift)?;
        let t = self.op2(a, AluOp::Iadd, m2, lo_h)?;
        let w1 = self.op2(a, AluOp::Iand, t, mask)?;
        let w2 = self.op2(a, AluOp::Ushr, t, shift)?;
        let w1 = self.op2(a, AluOp::Iadd, m1, w1)?;
        let w1h = self.op2(a, AluOp::Ushr, w1, shift)?;
        let sum = self.op2(a, AluOp::Iadd, w1h, w2)?;
        self.op2(a, AluOp::Iadd, hi, sum)
    }

    /// `n / d` or `n % d`, unsigned (nir_lower_idiv.c emit_udiv): a reciprocal
    /// estimate refined to 32 bits, then two correction steps.
    fn udiv(&mut self, a: &Arena<'a>, n: S, d: S, modulo: bool) -> Result<S, PrepError> {
        let df = self.op1(a, AluOp::U2f, d)?;
        let rcp = self.op1(a, AluOp::Frcp, df)?;
        let scale = self.push(a, SOp::Const(4294966784.0f32.to_bits()))?;
        let rcp = self.op2(a, AluOp::Fmul, rcp, scale)?;
        let rcp = self.op1(a, AluOp::F2u, rcp)?;
        let neg_d = self.op1(a, AluOp::Ineg, d)?;
        let nrd = self.op2(a, AluOp::Imul, rcp, neg_d)?;
        let adj = self.umul_high(a, rcp, nrd)?;
        let rcp = self.op2(a, AluOp::Iadd, rcp, adj)?;
        let one = self.push(a, SOp::Const(1))?;
        let mut q = self.umul_high(a, n, rcp)?;
        let qd = self.op2(a, AluOp::Imul, q, d)?;
        let neg_qd = self.op1(a, AluOp::Ineg, qd)?;
        let mut r = self.op2(a, AluOp::Iadd, n, neg_qd)?;
        /* First refinement: where r >= d (unsigned), q + 1 and r - d. */
        let below = self.op2(a, AluOp::Ult, r, d)?;
        if !modulo {
            let q1 = self.op2(a, AluOp::Iadd, q, one)?;
            q = self.sel(a, below, q, q1)?;
        }
        let r1 = self.op2(a, AluOp::Iadd, r, neg_d)?;
        r = self.sel(a, below, r, r1)?;
        /* Second refinement. */
        let below = self.op2(a, AluOp::Ult, r, d)?;
        if modulo {
            let r1 = self.op2(a, AluOp::Iadd, r, neg_d)?;
            self.sel(a, below, r, r1)
        } else {
            let q1 = self.op2(a, AluOp::Iadd, q, one)?;
            self.sel(a, below, q, q1)
        }
    }

    /// Integer division and remainder (nir_lower_idiv.c emit_idiv; IR
    /// `Imod` is eval.rs's wrapping_rem, NIR's irem: the sign of `n`).
    fn int_div(&mut self, a: &Arena<'a>, op: AluOp, n: S, d: S) -> Result<S, PrepError> {
        match op {
            AluOp::Udiv => return self.udiv(a, n, d, false),
            AluOp::Umod => return self.udiv(a, n, d, true),
            _ => {}
        }
        let neg_n = self.op1(a, AluOp::Ineg, n)?;
        let neg_d = self.op1(a, AluOp::Ineg, d)?;
        let lhs = self.op2(a, AluOp::Imax, n, neg_n)?;
        let rhs = self.op2(a, AluOp::Imax, d, neg_d)?;
        let zero = self.push(a, SOp::Const(0))?;
        let (res, neg) = if op == AluOp::Idiv {
            let x = self.op2(a, AluOp::Ixor, n, d)?;
            (self.udiv(a, lhs, rhs, false)?, self.op2(a, AluOp::Ilt, x, zero)?)
        } else {
            (self.udiv(a, lhs, rhs, true)?, self.op2(a, AluOp::Ilt, n, zero)?)
        };
        let neg_res = self.op1(a, AluOp::Ineg, res)?;
        self.sel(a, neg, neg_res, res)
    }

    fn src(&self, s: ir::Src, lane: usize) -> Result<S, PrepError> {
        let comp = *s.swizzle.get(lane).ok_or(PrepError::Undefined(s.val))? as usize;
        self.lanes.get(s.val.0 as usize).and_then(|l| l.get(comp).copied().flatten()).ok_or(PrepError::Undefined(s.val))
    }

    fn instr(&mut self, a: &Arena<'a>, id: ir::InstrId) -> Result<(), PrepError> {
        let op = *self.m.instr(id).ok_or(PrepError::Undefined(ir::ValueId(id.0)))?;
        let v = ir::ValueId(id.0);
        let n = (self.m.comps_of(v) as usize).clamp(1, 4);
        let mut out = [None; 4];
        match op {
            Op::ConstF32(bits) => {
                for (o, b) in out.iter_mut().zip(bits) {
                    *o = Some(self.push(a, SOp::Const(b))?);
                }
            }
            /* An int constant is its two's-complement bits, as eval.rs holds
             * it (`x as u32`); what reads it decides what it means. */
            Op::ConstI32(v) => {
                for (o, x) in out.iter_mut().zip(v) {
                    *o = Some(self.push(a, SOp::Const(x as u32))?);
                }
            }
            /* A bool constant is all ones or zero, the pattern every bool
             * producer writes (eval.rs bits_of_bool; select.rs
             * bool_constants). */
            Op::ConstBool(v) => {
                for (o, x) in out.iter_mut().zip(v) {
                    *o = Some(self.push(a, SOp::Const(if x { u32::MAX } else { 0 }))?);
                }
            }
            /* ffma as fmul then fadd, the two roundings eval.rs computes
             * (`x * y + z`): the QPU has no fused multiply-add (no FMA in
             * qpu_pack.c's v3d42 tables), and Mesa's nir_to_vir.c has no
             * nir_op_ffma arm, so its ffma is split before the back end. */
            Op::Alu(AluOp::Ffma, srcs) => {
                for (lane, o) in out.iter_mut().enumerate().take(n) {
                    let (Ok(x), Ok(y), Ok(z)) =
                        (self.src(srcs[0], lane), self.src(srcs[1], lane), self.src(srcs[2], lane))
                    else {
                        continue; /* see the over-estimate note below */
                    };
                    let t = self.push(a, SOp::Alu(AluOp::Fmul, [x, y, S(0)]))?;
                    *o = Some(self.push(a, SOp::Alu(AluOp::Fadd, [t, z, S(0)]))?);
                }
            }
            /* A dot product as the scalar chain eval.rs sums: the products
             * in x y z w order, each added to the running sum,
             * ((x0*y0 + x1*y1) + x2*y2) + x3*y3, one value in every lane.
             * (eval.rs starts the sum at +0.0, so a sum of products that are
             * all -0.0 is +0.0 there and -0.0 here; no other input differs.)
             * Mesa's lowering is the same chain, xyzw order when exact
             * (nir_lower_alu_width.c lower_fdot). */
            /* Integer division and remainder, as Mesa lowers them for the
             * V3D (nir_lower_idiv.c emit_idiv / emit_udiv, the 32-bit path,
             * exact over the whole range; its umul_high as nir_lower_alu.c
             * splits it under the V3D's lower_mul_high). A division by zero
             * is undefined in GLSL and in eval.rs. */
            Op::Alu(div @ (AluOp::Idiv | AluOp::Imod | AluOp::Udiv | AluOp::Umod), srcs) => {
                for (lane, o) in out.iter_mut().enumerate().take(n) {
                    let (x, y) = (self.src(srcs[0], lane)?, self.src(srcs[1], lane)?);
                    *o = Some(self.int_div(a, div, x, y)?);
                }
            }
            Op::Alu(dot @ (AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4), srcs) => {
                let k = match dot {
                    AluOp::Fdot2 => 2,
                    AluOp::Fdot3 => 3,
                    _ => 4,
                };
                let mut sum = None;
                for c in 0..k {
                    let (x, y) = (self.src(srcs[0], c)?, self.src(srcs[1], c)?);
                    let p = self.push(a, SOp::Alu(AluOp::Fmul, [x, y, S(0)]))?;
                    sum = Some(match sum {
                        None => p,
                        Some(q) => self.push(a, SOp::Alu(AluOp::Fadd, [q, p, S(0)]))?,
                    });
                }
                out = [sum; 4];
            }
            /* all() / any() and vector ==, !=: the four lanes of the
             * swizzled operand, by the IR's contract (eval.rs Ball/Bany; the
             * front end repeats a real component into the spare lanes). A
             * bool is ~0 or 0, so AND and OR of the lanes are exact. */
            Op::Alu(red @ (AluOp::Ball | AluOp::Bany), srcs) => {
                let op = if red == AluOp::Ball { AluOp::Iand } else { AluOp::Ior };
                let mut acc = self.src(srcs[0], 0)?;
                for c in 1..4 {
                    let x = self.src(srcs[0], c)?;
                    acc = self.push(a, SOp::Alu(op, [acc, x, S(0)]))?;
                }
                out = [Some(acc); 4];
            }
            Op::Alu(alu, srcs) => {
                if !SELECTED.contains(&alu) {
                    return Err(PrepError::NotYet(alu_name(alu)));
                }
                for (lane, o) in out.iter_mut().enumerate().take(n) {
                    /* The width table may over-estimate (ir.rs, comps_of:
                     * "an over-estimate, never a wrong narrow answer"; the
                     * front end leaves abs() of a vec2 unrecorded, so 4). A lane whose
                     * source lanes do not exist is left uncomputed; a use of
                     * it later is still refused as Undefined. */
                    let mut s = [S(0); 3];
                    let mut ok = true;
                    for (k, x) in s.iter_mut().enumerate().take(alu.arity()) {
                        match self.src(srcs[k], lane) {
                            Ok(v) => *x = v,
                            Err(_) => ok = false,
                        }
                    }
                    if ok {
                        /* A select with an undefined arm: GLSL leaves that
                         * value undefined, so the defined arm is a correct
                         * answer, as merge() takes it after an `if`. The
                         * front end writes a vector lane at a run-time index
                         * as Bcsel(i == lane, x, the old lane), and a local
                         * not yet written has no old lane (dEQP
                         * shaders.indexing, tmp[ui_zero] = ...). */
                        let undef = |x: S| self.ops.as_slice().get(x.0 as usize) == Some(&SOp::Undef);
                        *o = Some(match alu {
                            AluOp::Bcsel if undef(s[2]) => s[1],
                            AluOp::Bcsel if undef(s[1]) => s[2],
                            _ => self.push(a, SOp::Alu(alu, s))?,
                        });
                    }
                }
            }
            Op::LoadInput { slot, comps } => {
                let comps = (comps as usize).min(4);
                match (self.m.stage, slot) {
                    (SHC_STAGE_VERTEX, IoSlot::Attrib(loc)) => {
                        for (c, o) in out.iter_mut().enumerate().take(comps) {
                            *o = Some(self.push(a, SOp::Attr { loc, comp: c as u8 })?);
                        }
                    }
                    (SHC_STAGE_FRAGMENT, IoSlot::Varying(loc)) => {
                        let decl = self
                            .m
                            .inputs
                            .iter()
                            .find(|d| d.kind == IoKind::Varying && d.location == loc)
                            .ok_or(PrepError::NotYet("a varying with no declaration"))?;
                        match decl.interp {
                            /* flat and centroid: the same read; the chain
                             * sets the input's flat-shade or centroid flag,
                             * and select takes C, or W at the centroid */
                            ir::INTERP_SMOOTH | ir::INTERP_FLAT | ir::INTERP_CENTROID => {}
                            ir::INTERP_NOPERSPECTIVE => return Err(PrepError::NotYet("noperspective varying")),
                            _ => {
                                return Err(PrepError::NotYet("an interpolation qualifier this back end does not know"))
                            }
                        }
                        for (c, o) in out.iter_mut().enumerate().take(comps) {
                            *o = Some(self.push(a, SOp::Varying { loc, comp: c as u8 })?);
                        }
                    }
                    (SHC_STAGE_FRAGMENT, IoSlot::FragCoord) => {
                        for (c, o) in out.iter_mut().enumerate().take(comps) {
                            *o = Some(self.push(a, SOp::FragCoord { lane: c as u8 })?);
                        }
                    }
                    (SHC_STAGE_FRAGMENT, IoSlot::PointCoord) => {
                        for (c, o) in out.iter_mut().enumerate().take(comps.min(2)) {
                            *o = Some(self.push(a, SOp::PointCoord { lane: c as u8 })?);
                        }
                    }
                    (SHC_STAGE_FRAGMENT, IoSlot::FrontFacing) => {
                        out[0] = Some(self.push(a, SOp::FrontFacing)?);
                    }
                    (SHC_STAGE_VERTEX, IoSlot::VertexId) => {
                        out[0] = Some(self.push(a, SOp::VertexId)?);
                    }
                    (SHC_STAGE_VERTEX, IoSlot::InstanceId) => {
                        out[0] = Some(self.push(a, SOp::InstanceId)?);
                    }
                    (_, s) => return Err(PrepError::NotYet(slot_name(s))),
                }
            }
            Op::StoreOutput { slot, src } => {
                match (self.m.stage, slot) {
                    (SHC_STAGE_VERTEX, IoSlot::Position) => {
                        for lane in 0..4 {
                            self.position[lane] = Some(self.src(src, lane)?);
                        }
                    }
                    (SHC_STAGE_FRAGMENT, IoSlot::FragColor(rt)) if (rt as usize) < MAX_RTS => {
                        let rt = rt as usize;
                        /* an ES 3.00 output declared narrower than vec4
                         * (`out uvec2`, `out int`) stores its declared lanes;
                         * the rest stay unwritten (0, and select.rs writes
                         * an integer output's declared components only). The
                         * declaration's width, not the value's: `vec4(f)` is
                         * a scalar read through a splat swizzle. */
                        let comps = self
                            .m
                            .outputs
                            .iter()
                            .find(|d| d.kind == IoKind::Colour && usize::from(d.location) == rt)
                            .map_or(4, |d| usize::from(d.components.clamp(1, 4)));
                        for lane in 0..comps {
                            let s = self.src(src, lane)?;
                            /* A lane never written, carried through a store
                             * of the whole vector (`gl_FragColor.xy = ...`),
                             * stays unwritten: 0 (prepare_fragment_with). */
                            if self.ops.as_slice().get(s.0 as usize) != Some(&SOp::Undef) {
                                self.color[rt][lane] = Some(s);
                            }
                        }
                    }
                    /* gl_PointSize: kept for every primitive, unlike Mesa,
                     * which keeps it only for a points draw (vs_key
                     * per_vertex_point_size, v3d_program.c:783-785;
                     * v3d_nir_lower_io.c:155-158). The shader record's
                     * "point size in shaded vertex data" bit only describes
                     * the VPM layout: with the word kept and the bit set, a
                     * triangle and lines drew 0 of 4096 pixels differently
                     * on the Pi 4 (gl_test case 110, 37264ce2), so one
                     * program serves every primitive. */
                    (SHC_STAGE_VERTEX, IoSlot::PointSize) => {
                        self.psiz = Some(self.src(src, 0)?);
                    }
                    (SHC_STAGE_VERTEX, IoSlot::Varying(loc)) => {
                        let comps = self
                            .m
                            .outputs
                            .iter()
                            .find(|d| d.kind == IoKind::Varying && d.location == loc)
                            .map(|d| d.components)
                            .ok_or(PrepError::NotYet("a varying with no declaration"))?;
                        for lane in 0..(comps as usize).min(4) {
                            let s = self.src(src, lane)?;
                            let c = lane as u8;
                            /* A lane the shader never wrote, carried through a
                             * store of the whole vector: `tt.x = t` reads tt,
                             * writes x, and stores all of it (Qt's
                             * imageparticle_tabled). The lane stays unwritten,
                             * so the layout stores 0 there, as Mesa does for a
                             * varying nothing wrote; an earlier store stays. */
                            if self.ops.as_slice().get(s.0 as usize) == Some(&SOp::Undef) {
                                continue;
                            }
                            match self.stored.as_mut_slice().iter_mut().find(|e| e.0 == loc && e.1 == c) {
                                Some(e) => e.2 = s,
                                None => self.stored.push(a, (loc, c, s))?,
                            }
                        }
                    }
                    (_, s) => return Err(PrepError::NotYet(slot_name(s))),
                }
                return Ok(());
            }
            Op::LoadLocal { local, index, comps } => {
                if index.is_some() {
                    return Err(PrepError::NotYet("indexed local"));
                }
                let slot = *self.locals.get(local.0 as usize).ok_or(PrepError::NotYet("undeclared local"))?;
                for (c, o) in out.iter_mut().enumerate().take((comps as usize).min(4)) {
                    /* The walk has no control flow (it refuses every If and
                     * Loop), so "no store before" is "no store on any path". */
                    *o = Some(match slot[c] {
                        Some(s) => s,
                        None => self.push(a, SOp::Undef)?,
                    });
                }
            }
            Op::StoreLocal { local, index, src, write_mask } => {
                if index.is_some() {
                    return Err(PrepError::NotYet("indexed local"));
                }
                /* ir.md: destination lane i takes src.swizzle[i]. */
                let mut lanes = [None; 4];
                for (lane, l) in lanes.iter_mut().enumerate() {
                    if write_mask & (1 << lane) != 0 {
                        *l = Some(self.src(src, lane)?);
                    }
                }
                let slot = self.locals.get_mut(local.0 as usize).ok_or(PrepError::NotYet("undeclared local"))?;
                for (dst, l) in slot.iter_mut().zip(lanes) {
                    if l.is_some() {
                        *dst = l;
                    }
                }
                return Ok(());
            }
            Op::LoadUniform { dword_off, comps } => {
                for (c, o) in out.iter_mut().enumerate().take((comps as usize).min(4)) {
                    let dw = dword_off as usize + c;
                    /* The block the GL core fills is uniform_dwords long; a
                     * load past it would read another stage's values. */
                    if dw >= self.m.uniform_dwords as usize {
                        return Err(PrepError::UniformOutOfBlock);
                    }
                    *o = Some(self.push(a, SOp::Uniform(dw as u16))?);
                }
            }
            /* An array of uniforms at an index read at run time. Every element
             * is loaded as a plain uniform and the index picks one per lane,
             * with ir.md's clamp exactly: element 0 for i <= 0, element k for
             * i == k, the last for i >= len - 1 (eval.rs clamps the same way).
             * Right for an index that differs between lanes too, and needs no
             * UNIFA / QUNIFORM_UBO_ADDR (Mesa's path for a uniform index,
             * which reads its address from lane 0). Bounded by name. */
            Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
                const MAX_SELECTED: usize = 256;
                let (len, comps) = (len as usize, (comps as usize).min(4));
                if len == 0 || len * comps > MAX_SELECTED {
                    return Err(PrepError::NotYet("a uniform array too long to select from (no ldunifa yet)"));
                }
                let idx = self.src(index, 0)?;
                let last = (len - 1) as u32;
                for (c, o) in out.iter_mut().enumerate().take(comps) {
                    let mut v = None;
                    for k in 0..len {
                        let dw = base_dw as usize + k * stride_dw as usize + c;
                        if dw >= self.m.uniform_dwords as usize {
                            return Err(PrepError::UniformOutOfBlock);
                        }
                        let e = self.push(a, SOp::Uniform(dw as u16))?;
                        v = Some(match v {
                            None => e,
                            Some(prev) => {
                                let kk = k as u32;
                                /* the last element: i > len - 2; the others: i == k */
                                let cond = if kk == last {
                                    let below = self.push(a, SOp::Const(kk - 1))?;
                                    self.op2(a, AluOp::Ilt, below, idx)?
                                } else {
                                    let kc = self.push(a, SOp::Const(kk))?;
                                    self.op2(a, AluOp::Ieq, idx, kc)?
                                };
                                self.sel(a, cond, e, prev)?
                            }
                        });
                    }
                    *o = v;
                }
            }
            /* A 2D look-up (femtovg's texture2D, texture2DProj and their bias
             * forms): the lookup and its four lanes. Projection divides s and
             * t by the w lane (where the front end puts q), as a multiply by
             * its reciprocal, the way Mesa lowers it (nir_lower_tex's txp). */
            Op::Tex { dim, shadow, proj, ret, sampler, coord, lod, offset } => {
                let cube = dim == ir::TexDim::Cube;
                if !(dim == ir::TexDim::D2 || cube) {
                    return Err(PrepError::NotYet("a texture look-up other than 2D or cube"));
                }
                if cube && proj {
                    return Err(PrepError::NotYet("a projected cube look-up"));
                }
                if shadow {
                    return Err(PrepError::NotYet("a shadow look-up"));
                }
                /* An isampler / usampler reads its texture's integers raw
                 * (ES 3.0 3.8.13: an integer texture is filtered NEAREST
                 * only); select.rs takes them out of the TMU's words. */
                let lane_ret = match ret {
                    ir::ScalarType::F32 => TexRet::Float,
                    ir::ScalarType::U32 => TexRet::Uint,
                    ir::ScalarType::I32 => TexRet::Sint,
                    _ => return Err(PrepError::NotYet("a texture look-up of another result type")),
                };
                if offset.is_some() {
                    return Err(PrepError::NotYet("a texture look-up with an offset"));
                }
                /* The front end gives a vertex shader's implicit look-up as
                 * Lod(0.0) (ir.rs TexLod), Mesa's lower_invalid_implicit_lod
                 * (vir.c:808): the base level. */
                let (third, is_lod) = match lod {
                    /* No derivatives outside the fragment stage: Mesa's
                     * lower_invalid_implicit_lod (vir.c:808) makes such a
                     * look-up level 0. The front end already does it for
                     * texture2D; a vertex textureCube reaches here implicit. */
                    ir::TexLod::Implicit if self.m.stage != SHC_STAGE_FRAGMENT => {
                        (Some(self.push(a, SOp::Const(0.0f32.to_bits()))?), true)
                    }
                    ir::TexLod::Implicit => (None, false),
                    ir::TexLod::Bias(b) => (Some(self.src(b, 0)?), false),
                    ir::TexLod::Lod(l) => (Some(self.src(l, 0)?), true),
                    _ => return Err(PrepError::NotYet("a texture look-up with gradients")),
                };
                let (mut s, mut t) = (self.src(coord, 0)?, self.src(coord, 1)?);
                if proj {
                    let q = self.src(coord, 3)?;
                    let r = self.push(a, SOp::Alu(AluOp::Frcp, [q, S(0), S(0)]))?;
                    s = self.push(a, SOp::Alu(AluOp::Fmul, [s, r, S(0)]))?;
                    t = self.push(a, SOp::Alu(AluOp::Fmul, [t, r, S(0)]))?;
                }
                /* s, t, then r for a cube (the direction's z), then the bias
                 * or level: packed, so the operands are srcs[..n]. */
                let mut r = if cube { Some(self.src(coord, 2)?) } else { None };
                /* A cube direction is divided by its major axis first, so
                 * that its largest component is +-1: Mesa does it for every
                 * cube look-up (nir_normalize_cubemap_coords, run by the
                 * gallium driver unconditionally, v3d_program.c:374):
                 * coord * rcp(fmax(fmax(|x|, |y|), |z|)). dEQP's random
                 * shaders that sample a cube (48 of random.texture, 8 of
                 * all_features.fragment) all failed by picture on the Pi 4
                 * and every one computes its direction with a largest
                 * component far from 1 (`vec3(-7, -10, q).xzy + ...`);
                 * texture_functions' directions (1.01) passed. The host's
                 * cube model (texref::project_to_face) divides by the major
                 * axis itself, so it cannot see the difference. */
                let mut layer = None;
                let mut third = third;
                let mut is_lod = is_lod;
                if let Some(z) = r {
                    let abs = |w: &mut Self, x: S| w.push(a, SOp::Alu(AluOp::Fabs, [x, S(0), S(0)]));
                    let (s0, t0, z0) = (s, t, z);
                    let (ax, ay, az) = (abs(self, s)?, abs(self, t)?, abs(self, z)?);
                    let m = self.push(a, SOp::Alu(AluOp::Fmax, [ax, ay, S(0)]))?;
                    let m = self.push(a, SOp::Alu(AluOp::Fmax, [m, az, S(0)]))?;
                    let inv = self.push(a, SOp::Alu(AluOp::Frcp, [m, S(0), S(0)]))?;
                    s = self.push(a, SOp::Alu(AluOp::Fmul, [s, inv, S(0)]))?;
                    t = self.push(a, SOp::Alu(AluOp::Fmul, [t, inv, S(0)]))?;
                    let z = self.push(a, SOp::Alu(AluOp::Fmul, [z, inv, S(0)]))?;
                    if sampler < 15 && self.cube_noseam & (1 << sampler) != 0 {
                        /* GL ES 2.0 3.7.5: the face and its (s, t), looked
                         * up as a layer of the cube's 2D-array record
                         * (docs/v3d-noseamless-cube.md) */
                        let dir = [s0, t0, z0];
                        let f = self.cube_face(a, [ax, ay, az], [s, t, z])?;
                        /* A fragment look-up's level is computed here from
                         * the direction's derivatives (cube_lod), the bias
                         * added, and given to the TMU as an explicit level:
                         * the TMU's own, from the per-face (s, t), is wrong
                         * in every quad that straddles a face edge. */
                        if self.m.stage == SHC_STAGE_FRAGMENT && !is_lod {
                            let mut l = self.cube_lod(a, sampler, dir, inv, &f)?;
                            if let Some(b) = third {
                                l = self.op2(a, AluOp::Fadd, l, b)?;
                            }
                            third = Some(self.clamp_lod(a, l)?);
                            is_lod = true;
                        }
                        (s, t, layer) = (f.s, f.t, Some(f.face));
                        r = None;
                    } else {
                        r = Some(z);
                    }
                }
                let mut srcs = [s, t, S(0), S(0)];
                for (slot, x) in srcs[2..].iter_mut().zip([r.or(layer), third].into_iter().flatten()) {
                    *slot = x;
                }
                let tex = self.push(
                    a,
                    SOp::Tex {
                        sampler,
                        srcs,
                        cube: cube && layer.is_none(),
                        layer: layer.is_some(),
                        bias: third.is_some() && !is_lod,
                        lod: is_lod,
                    },
                )?;
                for (lane, o) in out.iter_mut().enumerate() {
                    *o = Some(self.push(a, SOp::TexLane { tex: [tex], lane: lane as u8, ret: lane_ret })?);
                }
            }
            /* textureSize of a 2D or cube sampler (GLSL ES 3.00 8.8: the
             * size of level `lod`; a cube's is one face's): Mesa's
             * ntq_emit_txs (nir_to_vir.c:915-956), each lane the base
             * level's size minified, ntq_minify: max(size >> lod, 1), a
             * logical shift (the V3D's SHR takes the low five bits of the
             * shift, as GLSL leaves a level past the chain undefined). */
            Op::TexSize { sampler, dim: ir::TexDim::D2 | ir::TexDim::Cube, lod } => {
                let lod = self.src(lod, 0)?;
                let one = self.push(a, SOp::Const(1))?;
                for (lane, o) in out.iter_mut().enumerate().take(2) {
                    let size = self.push(a, SOp::TexSizeWord { sampler, lane: lane as u8 })?;
                    let shifted = self.push(a, SOp::Alu(AluOp::Ushr, [size, lod, S(0)]))?;
                    *o = Some(self.push(a, SOp::Alu(AluOp::Imax, [shifted, one, S(0)]))?);
                }
            }
            Op::TexSize { .. } => return Err(PrepError::NotYet("textureSize of a 3D or array sampler")),
            /* The rest of the lane's work is dead, and the TLB drops it
             * (select.rs: SETMSF); what is kept is where it happened. */
            Op::Discard => {
                self.discard = Some(self.push(a, SOp::Const(!0))?);
                return Ok(());
            }
        }
        if let Some(l) = self.lanes.get_mut(v.0 as usize) {
            *l = out;
        }
        Ok(())
    }
}

impl<'m, 'a> Walk<'m, 'a> {
    /// Walk the nodes of `range`. `top`: the body of `main`, whose last
    /// node may be the closing `Return`.
    fn nodes(&mut self, a: &Arena<'a>, range: ir::Range, top: bool) -> Result<(), PrepError> {
        let list = self.m.nodes_of(range);
        for (n, node) in list.iter().enumerate() {
            match *node {
                Node::Instr(id) => self.instr(a, id)?,
                /* The end of `main`: the front end closes the body with one. */
                Node::Return if top && n + 1 == list.len() => {}
                Node::If { cond, then_, else_ } => self.if_convert(a, cond, then_, else_)?,
                Node::Loop { body } => self.counted_loop(a, body)?,
                Node::Break => return Err(PrepError::NotYet("break")),
                Node::Continue => return Err(PrepError::NotYet("continue")),
                Node::Return => return Err(PrepError::NotYet("return")),
            }
        }
        Ok(())
    }

    /// A loop the unroller kept, as a counted loop (docs/online-shaderc/
    /// v3d-backend.md, "Control flow, step 2"): every lane of a local the body
    /// writes becomes a [`SOp::Phi`] before the loop and a [`SOp::PhiNext`] at
    /// its end; after the loop the local reads the phi. A body that can
    /// discard carries the discard flag the same way.
    fn counted_loop(&mut self, a: &Arena<'a>, body: ir::Range) -> Result<(), PrepError> {
        let trip =
            *self.trips.iter().find(|t| t.first == body.first).ok_or(PrepError::NotYet("a loop with no trip count"))?;
        /* the (local, lane)s the body writes, anywhere in it */
        let mut written: AVec<'a, (u32, usize)> = AVec::new();
        self.written_lanes(a, body, &mut written)?;
        let (position0, color0, discard0, psiz0) = (self.position, self.color, self.discard, self.psiz);
        let stored0 = self.stored.len();
        /* The discard flag, carried round a loop whose body can discard: a
         * lane discarded in one pass stays discarded (dEQP discard.*loop*).
         * The unroller guards a discard after a break like a store (its
         * `if (flags) {} else { rest }`), so a lane that left the loop is
         * not discarded by a later pass. */
        let discard_phi = if self.discards(body) {
            let init = match discard0 {
                Some(d) => d,
                None => self.push(a, SOp::Const(0))?,
            };
            let p = self.push(a, SOp::Phi { init: [init] })?;
            self.discard = Some(p);
            Some(p)
        } else {
            None
        };
        let phis = a.alloc_filled(written.len(), S(0))?;
        for (p, &(l, k)) in phis.iter_mut().zip(written.as_slice()) {
            /* A lane first written inside the loop is undefined before it,
             * so any start value is correct. */
            let init = match self.locals[l as usize][k] {
                Some(x) => x,
                None => self.push(a, SOp::Const(0))?,
            };
            *p = self.push(a, SOp::Phi { init: [init] })?;
            self.locals[l as usize][k] = Some(*p);
            self.phi_local.push(a, (*p, l))?;
        }
        /* inside another counted loop: its breaks stay flags */
        let nested = self
            .open_loops
            .as_slice()
            .iter()
            .any(|&t| matches!(self.ops.as_slice().get(t as usize), Some(SOp::LoopHead)));
        self.push(a, SOp::LoopHead)?;
        let enclosing = self.enclosing_exits.len();
        if let Some(l) = trip.exit {
            self.enclosing_exits.push(a, l)?;
        }
        let clear0 = self.clear_at_pass_start.len();
        /* the phi of the break flag, when it is clear in every pass that
         * runs and starts clear (the unroller's store of false before the
         * loop): then it stays clear while no store sets it */
        let mut clear_phi = None;
        if let (Some(l), false) = (trip.exit, trip.divergent) {
            if let Some(&(p, _)) = self.phi_local.as_slice().iter().rev().find(|&&(_, pl)| pl == l.0) {
                self.clear_at_pass_start.push(a, p)?;
                let ops = self.ops.as_slice();
                if matches!(ops[p.0 as usize], SOp::Phi { init } if ops[init[0].0 as usize] == SOp::Const(0)) {
                    clear_phi = Some((l, p));
                }
            }
        }
        let break0 = self.break_to.take();
        if let (Some((flag, phi)), false, true) = (clear_phi, nested, self.m.stage == SHC_STAGE_FRAGMENT) {
            let carried = a.alloc_filled(written.len(), (S(0), 0u32, 0usize))?;
            for (c, (&p, &(l, k))) in carried.iter_mut().zip(phis.iter().zip(written.as_slice())) {
                *c = (p, l, k);
            }
            self.break_to = Some(BreakTo { flag, phi, carried, discard: self.discard });
        }
        self.nodes(a, body, false)?;
        self.break_to = break0;
        while self.clear_at_pass_start.len() > clear0 {
            self.clear_at_pass_start.pop();
        }
        while self.enclosing_exits.len() > enclosing {
            self.enclosing_exits.pop();
        }
        if self.position != position0 || self.color != color0 || self.psiz != psiz0 || self.stored.len() != stored0 {
            return Err(PrepError::NotYet("an output written inside a loop that can leave on data"));
        }
        match discard_phi {
            Some(p) => {
                let next = self.discard.ok_or(PrepError::NotYet("a loop's discard flag with no value"))?;
                if next != p {
                    self.push(a, SOp::PhiNext { ops: [p, next] })?;
                }
                self.discard = Some(p);
            }
            /* discards() found none, so nothing can have changed it */
            None if self.discard != discard0 => {
                return Err(PrepError::NotYet("a discard in a loop the discard scan missed"));
            }
            None => {}
        }
        /* The loop leaves when every lane is done with it: its own break
         * flag, or the break flag of a loop around it. A lane whose outer
         * loop has broken still runs the inner loop (the unroller guards
         * every store after a break), but its inner flag can never be set
         * there, as that store is guarded too; without the outer flags such
         * a pass runs the inner loop's whole count (4096 passes for a
         * uniform bound: dEQP loops.*_uniform_iterations nested, measured on
         * the host at 230,132 instructions for a 2 x 3 nest). */
        let flag = |w: &Self, l: ir::LocalId| {
            w.locals.get(l.0 as usize).and_then(|v| v[0]).ok_or(PrepError::NotYet("a loop exit flag with no value"))
        };
        /* A flag no store in the body set (each uniform `break` a branch,
         * SOp::LoopBreak) is clear at the back edge: the counter alone
         * decides there. */
        let mut exit = match trip.exit {
            Some(l) if clear_phi.is_some_and(|(_, p)| self.locals[l.0 as usize][0] == Some(p)) => None,
            Some(l) => Some(flag(self, l)?),
            None => None,
        };
        for k in 0..enclosing {
            let outer = flag(self, self.enclosing_exits.as_slice()[k])?;
            exit = Some(match exit {
                Some(e) => self.op2(a, AluOp::Ior, e, outer)?,
                None => outer,
            });
        }
        for (&p, &(l, k)) in phis.iter().zip(written.as_slice()) {
            let next = self.locals[l as usize][k].ok_or(PrepError::NotYet("a loop-carried lane with no value"))?;
            if next != p {
                self.push(a, SOp::PhiNext { ops: [p, next] })?;
            }
            self.locals[l as usize][k] = Some(p);
        }
        self.push(a, SOp::LoopEnd { count: trip.count, exit: [exit.unwrap_or(S(0))], has_exit: exit.is_some() })?;
        Ok(())
    }

    /// `range` holds a `discard`, nested arms and loops included.
    fn discards(&self, range: ir::Range) -> bool {
        self.m.nodes_of(range).iter().any(|node| match *node {
            Node::Instr(id) => matches!(self.m.instr(id), Some(Op::Discard)),
            Node::If { then_, else_, .. } => self.discards(then_) || self.discards(else_),
            Node::Loop { body } => self.discards(body),
            _ => false,
        })
    }

    /// Every (local, lane) a store in `range` writes, nested arms included.
    fn written_lanes(
        &self,
        a: &Arena<'a>,
        range: ir::Range,
        out: &mut AVec<'a, (u32, usize)>,
    ) -> Result<(), PrepError> {
        for node in self.m.nodes_of(range) {
            match *node {
                Node::Instr(id) => {
                    if let Some(Op::StoreLocal { local, write_mask, .. }) = self.m.instr(id) {
                        for k in 0..4 {
                            let lk = (local.0, k);
                            if write_mask & (1 << k) != 0 && !out.as_slice().contains(&lk) {
                                out.push(a, lk)?;
                            }
                        }
                    }
                }
                Node::If { then_, else_, .. } => {
                    self.written_lanes(a, then_, out)?;
                    self.written_lanes(a, else_, out)?;
                }
                Node::Loop { body } => self.written_lanes(a, body, out)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// An `if` as straight-line code (docs/online-shaderc/v3d-backend.md,
    /// "Control flow, step 1"): both arms are walked from the same state,
    /// and every lane of a local, of `gl_Position`, of `gl_FragColor` and of
    /// a stored varying that the arms leave different becomes
    /// `Bcsel(cond, then, else)`. An arm that did not write a lane leaves
    /// the value it had before the `if`, which both walks start from; that
    /// is the one-arm-writes case. Every operation prepared here is free of
    /// side effects, so running the untaken arm changes nothing but time.
    ///
    /// A large `if` on a uniform condition is a branch instead (Mesa keeps
    /// the `if` when its arms hold more than 24 ALU operations,
    /// nir_opt_peephole_select with v3d's `.limit = 24`, nir_to_vir.c:
    /// 2261-2266, and branches round it, ntq_emit_uniform_if :4293-4390):
    /// in a fragment shader, when the condition is computed from uniforms
    /// and constants only ([`Walk::is_uniform`]), neither arm holds a loop, a
    /// discard or a jump ([`Walk::branchable`]), and the arms cost more than
    /// [`BRANCH_LIMIT`] or hold a look-up ([`Walk::arms_cost`]). Mesa never
    /// flattens an `if` with a look-up in an arm (block_check_for_allowed_instrs,
    /// nir_opt_peephole_select.c: a tex instruction is refused unless the
    /// limit is ~0), so the pass that does not need it skips its thread
    /// switch and TMU access.
    fn if_convert(
        &mut self,
        a: &Arena<'a>,
        cond: ir::Src,
        then_: ir::Range,
        else_: ir::Range,
    ) -> Result<(), PrepError> {
        let c = self.src(cond, 0)?;
        /* The unroller's guard at the start of a kept loop's body, on the
         * loop's own break flag as the pass found it: clear in every pass
         * that runs (clear_at_pass_start), so the `else` arm is the code,
         * with no select and no branch round it. */
        if self.clear_at_pass_start.as_slice().contains(&c) {
            return self.nodes(a, else_, false);
        }
        if let Some(when) = self.uniform_break(a, c, then_, else_)? {
            self.push(a, SOp::LoopBreak { cond: [c], when })?;
            return Ok(());
        }
        if self.m.stage == SHC_STAGE_FRAGMENT
            && self.branchable(then_)
            && self.branchable(else_)
            && self.is_uniform(a, c)?
        {
            let (cost, look_up, common) = self.arms_cost(a, then_, else_)?;
            if cost > BRANCH_LIMIT || look_up {
                self.hoist_common(a, common)?;
                return self.arms(a, c, then_, else_, true);
            }
        }
        self.arms(a, c, then_, else_, false)
    }

    /// The unroller's `break` of the loop [`Walk::break_to`] names, as a
    /// branch out of it (docs/online-shaderc/v3d-backend.md, "Control flow,
    /// step 3"; Mesa's structured break): `Some(when)` for an `if` one arm
    /// of which only sets the loop's break flag ([`Walk::break_arm`]) and
    /// the other is empty, when the condition is uniform and every lane the
    /// loop carries, the flag and the discard flag still hold their values
    /// from the start of the pass. The pass then leaves where `c == when`
    /// and the phis hold the values the loop leaves with, as its back edge
    /// leaves them. The flag stays clear on the path that goes on, so the
    /// unroller's guard round the rest of the pass is that code
    /// (`clear_at_pass_start`).
    fn uniform_break(
        &self,
        a: &Arena<'a>,
        c: S,
        then_: ir::Range,
        else_: ir::Range,
    ) -> Result<Option<bool>, PrepError> {
        let Some(b) = self.break_to else { return Ok(None) };
        let empty = |r: ir::Range| self.m.nodes_of(r).is_empty();
        let when = if empty(else_) && self.break_arm(then_, b.flag) {
            true
        } else if empty(then_) && self.break_arm(else_, b.flag) {
            false
        } else {
            return Ok(None);
        };
        let unchanged = self.locals[b.flag.0 as usize][0] == Some(b.phi)
            && self.discard == b.discard
            && b.carried.iter().all(|&(p, l, k)| self.locals[l as usize][k] == Some(p));
        if !unchanged || !self.is_uniform(a, c)? {
            return Ok(None);
        }
        Ok(Some(when))
    }

    /// Whether `range` is the unroller's `break` of the loop whose flag is
    /// `flag`: constants, and one store of `true` to lane 0 of the flag
    /// (unroll.rs, a `break` in a kept loop's body).
    fn break_arm(&self, range: ir::Range, flag: ir::LocalId) -> bool {
        let mut stores = 0;
        for node in self.m.nodes_of(range) {
            let Node::Instr(id) = *node else { return false };
            match self.m.instr(id) {
                Some(Op::ConstBool(_) | Op::ConstF32(_) | Op::ConstI32(_)) => {}
                Some(&Op::StoreLocal { local, index: None, src, write_mask: 1 }) if local == flag => {
                    let lane = src.swizzle[0] as usize;
                    if !matches!(self.m.instr(ir::InstrId(src.val.0)), Some(Op::ConstBool(v)) if v[lane]) {
                        return false;
                    }
                    stores += 1;
                }
                _ => return false,
            }
        }
        stores == 1
    }

    /// Whether the nodes of `range` can sit inside a uniform branch's arm:
    /// straight-line code, look-ups and `if`s, with no discard, no loop and
    /// no jump. A look-up's thread switch in an arm is never the program's
    /// last one (select.rs Sel::last_thrsw, Mesa's last_thrsw_at_top_level).
    fn branchable(&self, range: ir::Range) -> bool {
        self.m.nodes_of(range).iter().all(|node| match *node {
            Node::Instr(id) => !matches!(self.m.instr(id), Some(Op::Discard)),
            Node::If { then_, else_, .. } => self.branchable(then_) && self.branchable(else_),
            Node::Loop { .. } | Node::Break | Node::Continue | Node::Return => false,
        })
    }

    /// Whether `c` is computed from uniforms and constants only, and so is
    /// the same in every fragment of a draw.
    fn is_uniform(&self, a: &Arena<'a>, c: S) -> Result<bool, PrepError> {
        let ops = self.ops.as_slice();
        let seen = a.alloc_filled(ops.len(), false)?;
        let mut stack: AVec<'a, S> = AVec::new();
        stack.push(a, c)?;
        while let Some(x) = stack.pop() {
            let i = x.0 as usize;
            if core::mem::replace(&mut seen[i], true) {
                continue;
            }
            match ops[i] {
                SOp::Const(_) | SOp::Uniform(_) => {}
                /* a loop-carried lane of a local every value of which is
                 * uniform (uniform_locals) */
                SOp::Phi { .. }
                    if self
                        .phi_local
                        .as_slice()
                        .iter()
                        .any(|&(p, l)| p == x && self.uniform_local.get(l as usize).copied().unwrap_or(false)) => {}
                SOp::Alu(..) | SOp::Fsub(_) => {
                    for &y in ops[i].operands() {
                        stack.push(a, y)?;
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// The ALU operations the two arms would add as a branch: both are
    /// walked, each in a region of its own (no reuse across them, as in the
    /// branch), counted, and taken back out. Moves are not counted
    /// (negation and absolute value become input modifiers), as Mesa does
    /// not count its move-like operations. The flag says whether an arm
    /// holds a look-up.
    ///
    /// Also returns the work both arms do alike, to go before the branch
    /// ([`Walk::hoist_common`]): each else-arm operation equal to a then-arm
    /// one whose operands are values from before the `if`, such operations,
    /// or inputs, uniforms and constants. Its operands are renumbered: an
    /// index at or past the `if`'s first operation `n0` names entry
    /// `index - n0` of the list itself. Mesa computes femtovg scissorMask's
    /// `scissorMat * vec3(p, 1.0)`, in both arms, once before its branch.
    fn arms_cost(
        &mut self,
        a: &Arena<'a>,
        then_: ir::Range,
        else_: ir::Range,
    ) -> Result<(usize, bool, &'a [SOp]), PrepError> {
        const DRY_THEN: u32 = u32::MAX - 1;
        const DRY_ELSE: u32 = u32::MAX - 2;
        let n0 = self.ops.len();
        let mut n1 = n0;
        let break0 = self.break_to.take();
        let locals0 = a.alloc_copy(self.locals)?;
        let (position0, color0, psiz0, discard0) = (self.position, self.color, self.psiz, self.discard);
        let stored0 = a.alloc_copy(self.stored.as_slice())?;
        for (region, arm) in [(DRY_THEN, then_), (DRY_ELSE, else_)] {
            self.open_loops.push(a, region)?;
            self.nodes(a, arm, false)?;
            self.open_loops.pop();
            if region == DRY_THEN {
                n1 = self.ops.len();
            }
            self.locals.copy_from_slice(locals0);
            (self.position, self.color, self.psiz, self.discard) = (position0, color0, psiz0, discard0);
            let mut stored = AVec::with_capacity(a, stored0.len())?;
            stored.extend_from_slice(a, stored0)?;
            self.stored = stored;
        }
        self.break_to = break0;
        let cost = self.ops.as_slice()[n0..]
            .iter()
            .filter(|o| {
                matches!(o, SOp::Fsub(_)) || matches!(o, SOp::Alu(op, _) if !matches!(op, AluOp::Fneg | AluOp::Fabs))
            })
            .count();
        let look_up = self.ops.as_slice()[n0..].iter().any(|o| matches!(o, SOp::Tex { .. }));
        let common = self.common_work(a, n0, n1)?;
        while self.ops.len() > n0 {
            self.ops.pop();
            self.loop_of.pop();
        }
        Ok((cost, look_up, common))
    }

    /// [`Walk::arms_cost`]'s common work: the then arm is `ops[n0..n1]`, the
    /// else arm `ops[n1..]`.
    fn common_work(&self, a: &Arena<'a>, n0: usize, n1: usize) -> Result<&'a [SOp], PrepError> {
        let ops = self.ops.as_slice();
        let n2 = ops.len();
        let pure = |o: &SOp| matches!(o, SOp::Alu(..) | SOp::Fsub(_) | SOp::FragCoord { .. } | SOp::FrontFacing);
        /* per else op, the then op it equals (NONE: none); per then op,
         * whether it goes before the branch */
        const NONE: u32 = u32::MAX;
        let e2t = a.alloc_filled(n2 - n0, NONE)?;
        let keep = a.alloc_filled(n2 - n0, false)?;
        for j in n1..n2 {
            let e = ops[j];
            if !pure(&e) {
                continue;
            }
            let map = |x: S| -> Option<S> {
                let i = x.0 as usize;
                if i < n0 || (i < n1 && position_free(&ops[i])) {
                    Some(x)
                } else if i >= n1 && e2t[i - n0] != NONE {
                    Some(S(e2t[i - n0]))
                } else {
                    None
                }
            };
            let m = match e {
                SOp::Alu(op, s) => {
                    let mut t = s;
                    let mut ok = true;
                    for x in t.iter_mut().take(op.arity()) {
                        match map(*x) {
                            Some(y) => *x = y,
                            None => ok = false,
                        }
                    }
                    if !ok {
                        continue;
                    }
                    SOp::Alu(op, t)
                }
                SOp::Fsub([x, y]) => match (map(x), map(y)) {
                    (Some(x), Some(y)) => SOp::Fsub([x, y]),
                    _ => continue,
                },
                o => o,
            };
            if let Some(t) = (n0..n1).find(|&t| ops[t] == m) {
                e2t[j - n0] = t as u32;
                keep[t - n0] = true;
                for &x in m.operands() {
                    if (n0..n1).contains(&(x.0 as usize)) {
                        keep[x.0 as usize - n0] = true;
                    }
                }
            }
        }
        /* the kept then ops, in order, renumbered: an operand at or past n0
         * names its entry in the list */
        let pos = a.alloc_filled(n1 - n0, NONE)?;
        let mut out: AVec<'a, SOp> = AVec::new();
        for t in n0..n1 {
            if !keep[t - n0] {
                continue;
            }
            let re = |x: S| -> S {
                let i = x.0 as usize;
                if i < n0 {
                    x
                } else {
                    S(n0 as u32 + pos[i - n0])
                }
            };
            let op = match ops[t] {
                SOp::Alu(op, mut s) => {
                    for x in s.iter_mut().take(op.arity()) {
                        *x = re(*x);
                    }
                    SOp::Alu(op, s)
                }
                SOp::Fsub([x, y]) => SOp::Fsub([re(x), re(y)]),
                o => o,
            };
            pos[t - n0] = out.len() as u32;
            out.push(a, op)?;
        }
        Ok(out.into_slice())
    }

    /// Push [`Walk::common_work`]'s list before a uniform branch: each arm's
    /// walk then finds these operations already made, in the region around
    /// the branch, and reuses them (Walk::push).
    fn hoist_common(&mut self, a: &Arena<'a>, common: &[SOp]) -> Result<(), PrepError> {
        let n0 = self.ops.len() as u32;
        let new = a.alloc_filled(common.len(), S(0))?;
        for (k, &op) in common.iter().enumerate() {
            let re = |x: S| if x.0 >= n0 { new[(x.0 - n0) as usize] } else { x };
            let op = match op {
                SOp::Alu(o, mut s) => {
                    for x in s.iter_mut().take(o.arity()) {
                        *x = re(*x);
                    }
                    SOp::Alu(o, s)
                }
                SOp::Fsub([x, y]) => SOp::Fsub([re(x), re(y)]),
                o => o,
            };
            new[k] = self.push(a, op)?;
        }
        Ok(())
    }

    /// The two arms of an `if`, if-converted (`branch` false) or as a
    /// uniform branch: then the arms are bracketed by [`SOp::IfBranch`],
    /// [`SOp::IfElse`] and [`SOp::IfEnd`], and a lane they leave different
    /// is an [`SOp::IfMerge`] instead of a select.
    fn arms(&mut self, a: &Arena<'a>, c: S, then_: ir::Range, else_: ir::Range, branch: bool) -> Result<(), PrepError> {
        if branch {
            self.push(a, SOp::IfBranch { cond: [c] })?;
        }
        /* a `break` inside either arm stays the flag */
        let break0 = self.break_to.take();
        /* the state before the `if` */
        let locals0 = a.alloc_copy(self.locals)?;
        let (position0, color0, psiz0) = (self.position, self.color, self.psiz);
        let stored0 = a.alloc_copy(self.stored.as_slice())?;
        let discard0 = self.discard;

        self.nodes(a, then_, false)?;
        let discard_t = self.discard;
        let locals_t = a.alloc_copy(self.locals)?;
        let (position_t, color_t, psiz_t) = (self.position, self.color, self.psiz);
        let stored_t = a.alloc_copy(self.stored.as_slice())?;

        /* back to the state before, for the else arm */
        self.locals.copy_from_slice(locals0);
        self.position = position0;
        self.color = color0;
        self.psiz = psiz0;
        let mut stored = AVec::with_capacity(a, stored0.len())?;
        stored.extend_from_slice(a, stored0)?;
        self.stored = stored;
        self.discard = discard0;
        if branch {
            self.push(a, SOp::IfElse)?;
        }
        self.nodes(a, else_, false)?;
        if branch {
            self.push(a, SOp::IfEnd)?;
        }
        self.break_to = break0;

        /* A discard is a side effect, so an arm that did not discard means
         * "not discarded", and the merge selects against false (unlike an
         * unwritten local, whose other value is undefined): `if (c) discard;`
         * leaves `c` itself. */
        self.discard = match (discard_t, self.discard) {
            (t, e) if t == e => t,
            (t, e) => {
                let yes = self.push(a, SOp::Const(!0))?;
                let no = self.push(a, SOp::Const(0))?;
                let (t, e) = (t.unwrap_or(no), e.unwrap_or(no));
                if t == yes && e == no {
                    Some(c)
                } else {
                    Some(self.push(a, SOp::Alu(AluOp::Bcsel, [c, t, e]))?)
                }
            }
        };

        /* self now holds the else arm's state; merge the then arm in */
        for (l, lanes_t) in locals_t.iter().enumerate() {
            for (k, &t) in lanes_t.iter().enumerate() {
                self.locals[l][k] = self.merge(a, c, t, self.locals[l][k], branch)?;
            }
        }
        for k in 0..4 {
            self.position[k] = self.merge(a, c, position_t[k], self.position[k], branch)?;
            for (rt, lanes_t) in color_t.iter().enumerate() {
                self.color[rt][k] = self.merge(a, c, lanes_t[k], self.color[rt][k], branch)?;
            }
        }
        self.psiz = self.merge(a, c, psiz_t, self.psiz, branch)?;
        for &(loc, comp, t) in stored_t.iter() {
            let e = self.stored.as_slice().iter().find(|x| x.0 == loc && x.1 == comp).map(|x| x.2);
            let Some(m) = self.merge(a, c, Some(t), e, branch)? else { continue };
            match self.stored.as_mut_slice().iter_mut().find(|x| x.0 == loc && x.1 == comp) {
                Some(x) => x.2 = m,
                None => self.stored.push(a, (loc, comp, m))?,
            }
        }
        /* A component only the else arm stored needs no merge: the then
         * arm left it unstored, so its value there is undefined (see
         * merge()) and the else arm's value stands. */
        Ok(())
    }

    /// One lane after an `if`: `t` from the then arm, `e` from the else arm,
    /// each already the value from before the `if` where its arm did not
    /// write. When only one has a value, the lane was written for the first
    /// time inside one arm: GLSL leaves the other path's value undefined,
    /// so any value is correct, and the written one is taken (eval.rs reads
    /// that path as UNSET, a NaN, so the three-way check cannot vouch for
    /// the lane either way).
    /// In a uniform branch (`branch`), the two values are an
    /// [`SOp::IfMerge`] instead.
    fn merge(&mut self, a: &Arena<'a>, c: S, t: Option<S>, e: Option<S>, branch: bool) -> Result<Option<S>, PrepError> {
        Ok(match (t, e) {
            (Some(t), Some(e)) if t == e => Some(t),
            (Some(t), Some(e)) if branch => Some(self.push(a, SOp::IfMerge { ops: [t, e] })?),
            (Some(t), Some(e)) => Some(self.push(a, SOp::Alu(AluOp::Bcsel, [c, t, e]))?),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        })
    }
}

/// Walk `m`'s body: straight-line code, with every `if` converted.
fn walk<'m, 'a>(
    m: &'m Module<'m>,
    trips: &'m [Trip],
    cube_noseam: u32,
    a: &Arena<'a>,
) -> Result<Walk<'m, 'a>, PrepError> {
    if m.locals.iter().any(|l| l.array_len > 1) {
        return Err(PrepError::NotYet("local array"));
    }
    let mut w = Walk {
        m,
        ops: AVec::new(),
        lanes: a.alloc_filled(m.instrs.len(), [None; 4])?,
        locals: a.alloc_filled(m.locals.len(), [None; 4])?,
        position: [None; 4],
        color: [[None; 4]; MAX_RTS],
        stored: AVec::new(),
        discard: None,
        psiz: None,
        trips,
        cube_noseam,
        enclosing_exits: AVec::new(),
        loop_of: AVec::new(),
        open_loops: AVec::new(),
        uniform_local: uniform_locals(m, a)?,
        phi_local: AVec::new(),
        clear_at_pass_start: AVec::new(),
        break_to: None,
    };
    w.nodes(a, m.body, true)?;
    Ok(w)
}

/// Keep what `roots` reach, renumbered, operands first; `roots` is rewritten
/// to the new numbering.
fn compact<'a>(ops: &[SOp], roots: &mut [S], a: &Arena<'a>) -> Result<&'a [SOp], PrepError> {
    let live = a.alloc_filled(ops.len(), false)?;
    /* A loop's structure is kept whatever reads it. */
    for (l, op) in live.iter_mut().zip(ops) {
        *l = op.is_effect();
    }
    for r in roots.iter() {
        if let Some(l) = live.get_mut(r.0 as usize) {
            *l = true;
        }
    }
    /* Operands come before their users, so one backward pass marks
     * everything reachable. A uniform branch is kept when an operation
     * inside it, or one of its merges, is live: then its markers and its
     * condition are too, and the pass runs again for what the condition
     * reads. One whose every value is dead (femtovg's stencil and
     * filter_image call scissorMask and never read the result) goes whole,
     * as the selects it replaces would. */
    loop {
        for i in (0..ops.len()).rev() {
            if live[i] {
                for s in ops[i].operands() {
                    live[s.0 as usize] = true;
                }
            }
        }
        let mut grew = false;
        for i in 0..ops.len() {
            if live[i] || !matches!(ops[i], SOp::IfBranch { .. }) {
                continue;
            }
            let (mut depth, mut els, mut end) = (0usize, None, None);
            for (j, o) in ops.iter().enumerate().skip(i + 1) {
                match o {
                    SOp::IfBranch { .. } => depth += 1,
                    SOp::IfElse if depth == 0 => els = Some(j),
                    SOp::IfEnd if depth == 0 => {
                        end = Some(j);
                        break;
                    }
                    SOp::IfEnd => depth -= 1,
                    _ => {}
                }
            }
            let (Some(els), Some(end)) = (els, end) else { return Err(PrepError::NotYet("an unbalanced branch")) };
            let merges = ops[end + 1..].iter().take_while(|o| matches!(o, SOp::IfMerge { .. })).count();
            /* An input, a uniform or a constant that happens to stand inside
             * the branch is read wherever it is needed (Walk::push's first
             * dedupe; select.rs loads it at the top or at each use), so it
             * does not keep the branch. */
            let needs = |j: usize| live[j] && !position_free(&ops[j]);
            if (i + 1..end).any(needs) || live[end + 1..end + 1 + merges].iter().any(|&l| l) {
                live[i] = true;
                live[els] = true;
                live[end] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let map = a.alloc_filled(ops.len(), u32::MAX)?;
    let mut out: AVec<'a, SOp> = AVec::new();
    for i in 0..ops.len() {
        if !live[i] {
            continue;
        }
        let op = match ops[i] {
            SOp::Alu(alu, mut s) => {
                for x in s.iter_mut().take(alu.arity()) {
                    *x = S(map[x.0 as usize]);
                }
                SOp::Alu(alu, s)
            }
            SOp::Tex { sampler, mut srcs, cube, layer, bias, lod } => {
                for x in srcs.iter_mut().take(tex_operands(cube || layer, bias || lod)) {
                    *x = S(map[x.0 as usize]);
                }
                SOp::Tex { sampler, srcs, cube, layer, bias, lod }
            }
            SOp::Fsub(s) => SOp::Fsub(s.map(|x| S(map[x.0 as usize]))),
            SOp::TexLane { tex, lane, ret } => SOp::TexLane { tex: [S(map[tex[0].0 as usize])], lane, ret },
            SOp::Phi { init } => SOp::Phi { init: [S(map[init[0].0 as usize])] },
            SOp::PhiNext { ops } => SOp::PhiNext { ops: [S(map[ops[0].0 as usize]), S(map[ops[1].0 as usize])] },
            SOp::LoopEnd { count, exit, has_exit } => {
                SOp::LoopEnd { count, exit: if has_exit { [S(map[exit[0].0 as usize])] } else { exit }, has_exit }
            }
            SOp::IfBranch { cond } => SOp::IfBranch { cond: [S(map[cond[0].0 as usize])] },
            SOp::LoopBreak { cond, when } => SOp::LoopBreak { cond: [S(map[cond[0].0 as usize])], when },
            SOp::IfMerge { ops } => SOp::IfMerge { ops: ops.map(|x| S(map[x.0 as usize])) },
            o => o,
        };
        map[i] = out.len() as u32;
        out.push(a, op)?;
    }
    for r in roots.iter_mut() {
        *r = S(map[r.0 as usize]);
    }
    Ok(out.into_slice())
}

/// The operations [`Walk::push`] reuses from anywhere, whatever region they
/// stand in: select.rs reads each at the top of the program or where it is
/// used, not where it stands.
fn position_free(op: &SOp) -> bool {
    matches!(
        op,
        SOp::Const(_)
            | SOp::Undef
            | SOp::Uniform(_)
            | SOp::TexSizeWord { .. }
            | SOp::Varying { .. }
            | SOp::Attr { .. }
            | SOp::PointCoord { .. }
            | SOp::VertexId
            | SOp::InstanceId
    )
}

fn refuse_undef(ops: &[SOp]) -> Result<(), PrepError> {
    if ops.contains(&SOp::Undef) {
        return Err(PrepError::NotYet(UNDEFINED_READ));
    }
    Ok(())
}

/// How many of [`SOp::Tex`]'s `srcs` it reads: s and t, r for a cube or the
/// layer of an array look-up (`third_coord`), and the bias or level.
pub(super) fn tex_operands(third_coord: bool, third: bool) -> usize {
    2 + usize::from(third_coord) + usize::from(third)
}

/// Prepare a fragment shader: `gl_FragColor` and what it needs.
pub fn prepare_fragment<'a>(m: &Module<'_>, a: &Arena<'a>) -> Result<Prepared<'a>, PrepError> {
    prepare_fragment_with(m, &[], 0, a)
}

/// [`prepare_fragment`] of a module the unroller made, with the loops it kept.
pub fn prepare_fragment_with<'a>(
    m: &Module<'_>,
    trips: &[Trip],
    cube_noseam: u32,
    a: &Arena<'a>,
) -> Result<Prepared<'a>, PrepError> {
    if m.stage != SHC_STAGE_FRAGMENT {
        return Err(PrepError::WrongStage);
    }
    let mut w = walk(m, trips, cube_noseam, a)?;
    /* A shader that writes no colour at all (legal GLSL: a depth pre-pass)
     * writes only the pass-through Z (select.rs, emit_frag_end, nir_to_vir.c:
     * 2113-2129). A lane of gl_FragColor never written is undefined: 0. */
    let mut rt_mask = 0u8;
    for (rt, lanes) in w.color.iter().enumerate() {
        if lanes.iter().any(Option::is_some) {
            rt_mask |= 1 << rt;
        }
    }
    /* The colours of every RT, then the discard condition: roots kept
     * through compact (an RT not written holds zeros, never read). */
    let mut roots = [S(0); 4 * MAX_RTS + 1];
    for rt in 0..MAX_RTS {
        for lane in 0..4 {
            roots[4 * rt + lane] = match w.color[rt][lane] {
                Some(x) => x,
                None => w.push(a, SOp::Const(0))?,
            };
        }
    }
    roots[4 * MAX_RTS] = w.discard.unwrap_or(S(0));
    let n = if w.discard.is_some() { 4 * MAX_RTS + 1 } else { 4 * MAX_RTS };
    let ops = compact(w.ops.as_slice(), &mut roots[..n], a)?;
    refuse_undef(ops)?;
    let color: [[S; 4]; MAX_RTS] = core::array::from_fn(|rt| core::array::from_fn(|lane| roots[4 * rt + lane]));
    let discard = w.discard.map(|_| roots[4 * MAX_RTS]);
    /* The declared type and width of each ES 3.00 output: an integer one
     * is written to the TLB as 32-bit words (select.rs). gl_FragColor and
     * gl_FragData[n] are vec4 (their declaration says GL_FLOAT_VEC4). */
    let mut rt_int = 0u8;
    let mut rt_comps = [4u8; MAX_RTS];
    for d in m.outputs.iter().filter(|d| d.kind == IoKind::Colour && (d.location as usize) < MAX_RTS) {
        let rt = d.location as usize;
        rt_comps[rt] = d.components.clamp(1, 4);
        if integer_output(d.gl_type) {
            rt_int |= 1 << rt;
        }
    }
    Ok(Prepared {
        stage: SHC_STAGE_FRAGMENT,
        ops,
        position: [S(0); 4],
        varyings: &[],
        color,
        rt_mask,
        rt_int,
        rt_comps,
        discard,
        psiz: None,
    })
}

/// Is a fragment output's GL type an integer one: GL_INT (0x1404),
/// GL_UNSIGNED_INT (0x1405), GL_INT_VEC2..4 (0x8B53..0x8B55) or
/// GL_UNSIGNED_INT_VEC2..4 (0x8DC6..0x8DC8)?
pub fn integer_output(gl_type: u16) -> bool {
    matches!(gl_type, 0x1404 | 0x1405 | 0x8B53..=0x8B55 | 0x8DC6..=0x8DC8)
}

/// The fragment shader's inputs, in the order its `ldvary`s read them:
/// ascending (location, component). The vertex shader writes its varyings
/// in this order ([`prepare_vertex`]'s `wanted`), and this is the FS's
/// `input_slots`.
pub fn fragment_inputs<'a>(p: &Prepared<'_>, a: &Arena<'a>) -> Result<&'a [(u8, u8)], PrepError> {
    let mut v: AVec<'a, (u8, u8)> = AVec::new();
    for op in p.ops {
        if let SOp::Varying { loc, comp } = *op {
            if !v.as_slice().contains(&(loc, comp)) {
                v.push(a, (loc, comp))?;
            }
        }
    }
    v.as_mut_slice().sort_unstable();
    Ok(v.into_slice())
}

/// Prepare a vertex shader writing `gl_Position` and the varying components
/// `wanted` (VERTEX-shader locations), in that order. The coordinate shader
/// is this with `wanted` empty.
pub fn prepare_vertex<'a>(m: &Module<'_>, wanted: &[(u8, u8)], a: &Arena<'a>) -> Result<Prepared<'a>, PrepError> {
    prepare_vertex_with(m, wanted, &[], 0, a)
}

/// [`prepare_vertex`] of a module the unroller made, with the loops it kept.
pub fn prepare_vertex_with<'a>(
    m: &Module<'_>,
    wanted: &[(u8, u8)],
    trips: &[Trip],
    cube_noseam: u32,
    a: &Arena<'a>,
) -> Result<Prepared<'a>, PrepError> {
    if m.stage != SHC_STAGE_VERTEX {
        return Err(PrepError::WrongStage);
    }
    let w = walk(m, trips, cube_noseam, a)?;
    let psiz = w.psiz;
    let vary = a.alloc_filled(wanted.len(), None::<S>)?;
    for (v, &(loc, comp)) in vary.iter_mut().zip(wanted) {
        *v = w.stored.as_slice().iter().find(|e| e.0 == loc && e.1 == comp).map(|e| e.2);
    }
    /* The roots: the position, then each varying that is written, then the
     * point size when it is kept. */
    let nv = vary.iter().flatten().count();
    let roots = a.alloc_filled(4 + nv + usize::from(psiz.is_some()), S(0))?;
    for (r, p) in roots.iter_mut().zip(w.position) {
        /* Legal GLSL, and an undefined position: refused, not invented. */
        *r = p.ok_or(PrepError::NotYet("a vertex shader that writes no gl_Position"))?;
    }
    for (r, v) in roots[4..].iter_mut().zip(vary.iter().flatten()) {
        *r = *v;
    }
    if let Some(x) = psiz {
        roots[4 + nv] = x;
    }
    let ops = compact(w.ops.as_slice(), roots, a)?;
    refuse_undef(ops)?;
    let mut position = [S(0); 4];
    position.copy_from_slice(&roots[..4]);
    for (v, r) in vary.iter_mut().flatten().zip(&roots[4..]) {
        *v = *r;
    }
    let psiz = psiz.map(|_| roots[4 + nv]);
    Ok(Prepared {
        stage: SHC_STAGE_VERTEX,
        ops,
        position,
        varyings: vary,
        color: [[S(0); 4]; MAX_RTS],
        rt_mask: 0,
        rt_int: 0,
        rt_comps: [4; MAX_RTS],
        discard: None,
        psiz,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::lower;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::store;
    use std::format;
    use std::string::String;

    /// A prepared value as text: constants as floats, the rest by shape.
    fn show(ops: &[SOp], s: S) -> String {
        match ops[s.0 as usize] {
            SOp::Const(b) => format!("{}", f32::from_bits(b)),
            SOp::Varying { loc, comp } => format!("v{loc}.{comp}"),
            SOp::Uniform(d) => format!("u{d}"),
            SOp::Alu(AluOp::Bcsel, [c, t, e]) => {
                format!("sel({}, {}, {})", show(ops, c), show(ops, t), show(ops, e))
            }
            SOp::Fsub([x, y]) => format!("Fsub({}, {})", show(ops, x), show(ops, y)),
            SOp::Alu(op, [x, y, _]) if op.arity() == 2 => format!("{op:?}({}, {})", show(ops, x), show(ops, y)),
            SOp::Alu(op, [x, _, _]) => format!("{op:?}({})", show(ops, x)),
            other => format!("{other:?}"),
        }
    }

    /// `gl_FragColor`'s four lanes of `body` (the FS reads `v`, a vec4
    /// varying, and a uniform vec4 `u`), as text.
    fn colour_of(body: &str) -> Result<[String; 4], PrepError> {
        let src = format!("precision mediump float; varying vec4 v; uniform vec4 u; void main() {{ {body} }}");
        store::with_scratch(|a| {
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&src).unwrap()).map_err(|e| e.msg).unwrap();
            let p = prepare_fragment(&f.module, a)?;
            Ok(p.color[0].map(|c| show(p.ops, c)))
        })
        .unwrap()
    }

    /// How many prepared ops of a GLSL ES 1.00 fragment shader (`v` a vec4
    /// varying, `u` a uniform vec4) are `Fmul(v.y, v.z)`.
    fn products_of(body: &str) -> usize {
        let src = format!("precision mediump float; varying vec4 v; uniform vec4 u; void main() {{ {body} }}");
        store::with_scratch(|a| {
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&src).unwrap()).map_err(|e| e.msg).unwrap();
            let (m, trips) = match crate::shaderc::online::powervr::unroll::unroll(&f.module, a).expect("unrolls") {
                Some(u) => (u.module, u.trips),
                None => (&f.module, &[][..]),
            };
            let p = prepare_fragment_with(m, trips, 0, a).expect("prepares");
            p.ops
                .iter()
                .filter(|o| show(p.ops, S(p.ops.iter().position(|x| x == *o).unwrap() as u32)) == "Fmul(v0.1, v0.2)")
                .count()
        })
        .unwrap()
    }

    /// Common subexpressions (Walk::push, Mesa's nir_opt_cse): the same
    /// product in both arms of an if-converted `if` is computed once
    /// (FemtoVG's scissorMask). A product computed inside a counted loop
    /// that has ended is NOT reused after it: that value is its last pass's
    /// only. Revert check: without the reuse the first count is 2; reusing
    /// the loop's product the second is 1.
    #[test]
    fn a_common_subexpression_is_computed_once_but_not_out_of_a_loop() {
        let arms = "float b; if (u.x > 0.0) b = v.y * v.z + 1.0; else b = v.y * v.z - 1.0; \
                    gl_FragColor = vec4(b, v.y * v.z, 0.0, 1.0);";
        assert_eq!(products_of(arms), 1, "one product for both arms and the use after");
        let looped = "float t = 0.0; for (int i = 0; i < 4; i++) { if (v.x > float(i)) break; t += v.y * v.z; } \
                      gl_FragColor = vec4(t, v.y * v.z, 0.0, 1.0);";
        assert_eq!(products_of(looped), 2, "the product after the loop is its own");
    }

    /// prepare's exact algebraic rewrites (Walk::algebraic), each by the
    /// prepared value it leaves: x * 1.0 is x; -(-x) is x; x - y (the front
    /// end's x + (-y)) is FSUB; u - v * w keeps the product and subtracts it;
    /// a * (-b) + c becomes c - a * b. Revert check: without algebraic the
    /// lanes read Fmul(v0.0, 1), Fneg(Fneg(v0.1)), Fadd(v0.2, Fneg(v0.3)).
    /// Work both arms of a uniform branch do alike goes before the branch
    /// (Walk::arms_cost's common work, Walk::hoist_common), as Mesa computes
    /// femtovg scissorMask's matrix product once before its branch: here
    /// `v.y * v.z` in both large arms is one Fmul, before the IfBranch, and
    /// each arm reuses it. Work of one arm stays in it. Revert check: without
    /// the hoist the product is made twice, once in each arm.
    #[test]
    fn work_both_arms_share_goes_before_the_branch() {
        let arm = |k: f32| {
            format!(
                "c = vec4(v.y * v.z) + v * {k:.1} + v.wzyx * v.xxyy * {k:.1} + v.yzwx * v.zwxy; \
                 c = c * c.yzwx + v * c.wzyx;"
            )
        };
        let src = format!(
            "precision highp float; varying vec4 v; uniform vec4 u; void main() {{ vec4 c; \
             if (u.x > 0.5) {{ {} }} else {{ {} }} gl_FragColor = c; }}",
            arm(2.0),
            arm(3.0)
        );
        store::with_scratch(|a| {
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&src).unwrap()).map_err(|e| e.msg).unwrap();
            let p = prepare_fragment(&f.module, a).expect("prepares");
            let at =
                |want: &str| -> Vec<usize> { (0..p.ops.len()).filter(|&i| show(p.ops, S(i as u32)) == want).collect() };
            let branch = p.ops.iter().position(|o| matches!(o, SOp::IfBranch { .. })).expect("a uniform branch");
            let product = at("Fmul(v0.1, v0.2)");
            assert_eq!(product.len(), 1, "one product for both arms");
            assert!(product[0] < branch, "made before the branch: {} vs {branch}", product[0]);
        })
        .unwrap();
    }

    /// A uniform `if` with a look-up in an arm is a branch however small
    /// the arm (Mesa's nir_opt_peephole_select never flattens a tex
    /// instruction), so the draw that does not take it skips the look-up.
    /// The same `if` on a varying condition, or with no look-up and few
    /// operations, stays selects. Revert checks: with look-ups refused in
    /// an arm (branchable) or not counted (arms_cost), the first is 0.
    #[test]
    fn a_uniform_if_with_a_look_up_is_a_branch() {
        let branches = |cond: &str, arm: &str| -> usize {
            let src = format!(
                "precision mediump float; varying vec2 v; uniform float u; uniform sampler2D t; \
                 void main() {{ vec4 c = vec4(0.25); if ({cond}) {{ {arm} }} gl_FragColor = c; }}"
            );
            store::with_scratch(|a| {
                let f =
                    lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&src).unwrap()).map_err(|e| e.msg).unwrap();
                let p = prepare_fragment(&f.module, a).expect("prepares");
                p.ops.iter().filter(|o| matches!(o, SOp::IfBranch { .. })).count()
            })
            .unwrap()
        };
        assert_eq!(branches("u > 0.5", "c = texture2D(t, v);"), 1, "a uniform if with a look-up");
        assert_eq!(branches("v.x > 0.5", "c = texture2D(t, v);"), 0, "a varying condition: selects");
        assert_eq!(branches("u > 0.5", "c = vec4(v, 0.0, 1.0);"), 0, "no look-up, small: selects");
    }

    /// A kept loop's break on a uniform bound (filter_image's shape: `for
    /// (i < 16) { if (i >= n) break; ... }` with `n` a uniform) is uniform
    /// all the way: the counter and the flag are uniform locals
    /// (uniform_locals), so the loop test and the `break` are uniform
    /// breaks, each a branch to the loop's exit (SOp::LoopBreak), and the
    /// flag is never set on the path that goes on: the unroller's guards
    /// round the rest of the body, `if (flags) {} else { rest }`, are that
    /// code (clear_at_pass_start), no branch and no select, and the back
    /// edge tests the counter alone. The same loop broken by a varying
    /// stays selects and a flag. Revert checks: without phis in
    /// is_uniform, no break branch; without uniform_break, two IfBranch
    /// and no LoopBreak (the shape before step (c)).
    #[test]
    fn a_kept_loops_uniform_break_guard_is_a_branch() {
        let body = "c += v * float(i) + v.yzwx * v.zwxy * u + v.wzyx * v.xyxy + v.zzww * u.yyxx; \
                    c = c * c.yzwx + v * c.wzyx;";
        let branches = |bound: &str| -> (usize, usize, bool) {
            let src = format!(
                "precision highp float; varying vec4 v; uniform vec4 u; uniform int n; \
                 void main() {{ vec4 c = vec4(0.0); \
                 for (int i = 0; i < 16; ++i) {{ if (i >= {bound}) break; {body} }} \
                 gl_FragColor = c; }}"
            );
            store::with_scratch(|a| {
                let f =
                    lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&src).unwrap()).map_err(|e| e.msg).unwrap();
                let u =
                    crate::shaderc::online::powervr::unroll::unroll(&f.module, a).expect("unrolls").expect("a loop");
                assert_eq!(u.trips.len(), 1, "the loop is kept");
                let p = prepare_fragment_with(u.module, u.trips, 0, a).expect("prepares");
                let count = |f: fn(&SOp) -> bool| p.ops.iter().filter(|o| f(o)).count();
                let has_exit = p.ops.iter().any(|o| matches!(o, SOp::LoopEnd { has_exit: true, .. }));
                (count(|o| matches!(o, SOp::IfBranch { .. })), count(|o| matches!(o, SOp::LoopBreak { .. })), has_exit)
            })
            .unwrap()
        };
        assert_eq!(branches("n"), (0, 2, false), "a uniform bound: the loop test and the break are breaks");
        assert_eq!(branches("int(v.x * 4.0)"), (0, 0, false), "a varying bound: selects, the counter alone");
    }

    #[test]
    fn exact_algebraic_rewrites_leave_fewer_operations() {
        let c = colour_of("gl_FragColor = vec4(v.x * 1.0, -(-v.y), v.z - v.w, u.x - v.x * u.y);").expect("prepares");
        assert_eq!(c[0], "v0.0", "x * 1.0 is x");
        assert_eq!(c[1], "v0.1", "-(-x) is x");
        assert_eq!(c[2], "Fsub(v0.2, v0.3)", "x + (-y) is FSUB");
        assert_eq!(c[3], "Fsub(u0, Fmul(v0.0, u1))", "u - v * w: FSUB of the product");
        let c =
            colour_of("gl_FragColor = vec4(v.x * -v.y + v.z, v.x * -1.0, (-v.x) * (-v.y), 1.0);").expect("prepares");
        assert_eq!(c[0], "Fsub(v0.2, Fmul(v0.0, v0.1))", "a * (-b) + c is c - a * b");
        assert_eq!(c[1], "Fneg(v0.0)", "x * -1.0 is -x");
        assert_eq!(c[2], "Fmul(v0.0, v0.1)", "(-a) * (-b) is a * b");
        let c = colour_of("float a = abs(v.x); gl_FragColor = vec4(max(a, a), min(a, a), max(a, v.y), 1.0);")
            .expect("prepares");
        assert_eq!(c[0], "Fabs(v0.0)", "max(x, x) is x");
        assert_eq!(c[1], "Fabs(v0.0)", "min(x, x) is x");
        assert_eq!(c[2], "Fmax(Fabs(v0.0), v0.1)", "two different operands stay");
        let c = colour_of("gl_FragColor = vec4(v.x + 0.0, 0.0 + v.y, v.z - 0.0, v.w + -0.0);").expect("prepares");
        assert_eq!(c, ["v0.0", "v0.1", "v0.2", "v0.3"].map(String::from), "x + 0.0, 0.0 + x, x - 0.0, x + -0.0 are x");
    }

    #[test]
    fn an_if_whose_arms_both_write_selects_per_lane() {
        let c =
            colour_of("vec4 c; if (v.x < u.x) { c = vec4(0.25); } else { c = vec4(0.75); } gl_FragColor = c;").unwrap();
        assert_eq!(c, ["sel(Flt(v0.0, u0), 0.25, 0.75)"; 4].map(String::from));
    }

    #[test]
    fn a_lane_one_arm_writes_keeps_its_value_from_before_the_if() {
        /* The one-arm-writes case, with every value distinct, both ways
         * round: the other lanes are untouched, and the lane written in one
         * arm selects against the value from before the if. */
        let then_ =
            colour_of("vec4 c = vec4(0.125, 0.25, 0.375, 0.5); if (v.x < u.x) { c.y = 0.75; } gl_FragColor = c;")
                .unwrap();
        assert_eq!(then_, ["0.125", "sel(Flt(v0.0, u0), 0.75, 0.25)", "0.375", "0.5"]);
        let else_ = colour_of(
            "vec4 c = vec4(0.125, 0.25, 0.375, 0.5); if (v.x < u.x) { } else { c.z = 0.75; } gl_FragColor = c;",
        )
        .unwrap();
        assert_eq!(else_, ["0.125", "0.25", "sel(Flt(v0.0, u0), 0.375, 0.75)", "0.5"]);
    }

    #[test]
    fn nested_ifs_merge_inside_out() {
        let c = colour_of(
            "float f = 0.125; if (v.x < u.x) { if (v.y < u.y) { f = 0.25; } else { f = 0.375; } } \
             gl_FragColor = vec4(f);",
        )
        .unwrap();
        assert_eq!(c[0], "sel(Flt(v0.0, u0), sel(Flt(v0.1, u1), 0.25, 0.375), 0.125)");
        assert!(c.iter().all(|x| *x == c[0]));
    }

    #[test]
    fn gl_fragcolor_written_in_the_arms() {
        let c = colour_of("if (v.x < u.x) { gl_FragColor = vec4(0.25); } else { gl_FragColor = v; }").unwrap();
        assert_eq!(c[0], "sel(Flt(v0.0, u0), 0.25, v0.0)");
        assert_eq!(c[3], "sel(Flt(v0.0, u0), 0.25, v0.3)");
    }

    #[test]
    fn a_lane_first_written_inside_one_arm_takes_the_written_value() {
        /* GLSL leaves f undefined when the arm does not run, so any value is
         * correct; the written one is taken, with no select. */
        let c = colour_of("float f; if (v.x < u.x) { f = 0.75; } gl_FragColor = vec4(f);").unwrap();
        assert_eq!(c, ["0.75"; 4].map(String::from));
    }

    /// The discard condition of `body`, as text.
    fn discard_of(body: &str) -> Option<String> {
        let src = format!("precision mediump float; varying vec4 v; uniform vec4 u; void main() {{ {body} }}");
        store::with_scratch(|a| {
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&src).unwrap()).map_err(|e| e.msg).unwrap();
            let p = prepare_fragment(&f.module, a).unwrap();
            p.discard.map(|d| show(p.ops, d))
        })
        .unwrap()
    }

    #[test]
    fn a_discard_is_a_condition_and_an_arm_without_one_means_false() {
        /* `if (c) discard;` is c itself; the other arm did not discard, which
         * is false, never "undefined" (as an unwritten local would be). */
        assert_eq!(discard_of("if (v.x < u.x) discard; gl_FragColor = v;").as_deref(), Some("Flt(v0.0, u0)"));
        assert_eq!(
            discard_of("if (v.x < u.x) { } else { discard; } gl_FragColor = v;").as_deref(),
            Some("sel(Flt(v0.0, u0), 0, NaN)")
        );
        /* nested: the inner condition holds only where the outer one does */
        assert_eq!(
            discard_of("if (v.x < u.x) { if (v.y < u.y) discard; } gl_FragColor = v;").as_deref(),
            Some("sel(Flt(v0.0, u0), Flt(v0.1, u1), 0)")
        );
        /* unconditional, and none */
        assert_eq!(discard_of("gl_FragColor = v; discard;").as_deref(), Some("NaN"));
        assert_eq!(discard_of("gl_FragColor = v;"), None);
    }

    #[test]
    fn a_varying_stored_in_one_arm_of_the_vertex_shader() {
        let src = "attribute vec4 a; uniform vec4 u; varying vec4 w; void main() { gl_Position = a; \
                   w = vec4(0.125); if (a.x < u.x) { w.y = 0.75; } }";
        store::with_scratch(|ar| {
            let m = lower::compile_to_ir(ar, Stage::Vertex, ar.alloc_str(src).unwrap()).map_err(|e| e.msg).unwrap();
            let p = prepare_vertex(&m.module, &[(0, 0), (0, 1)], ar).unwrap();
            let w: std::vec::Vec<String> = p.varyings.iter().map(|v| show(p.ops, v.unwrap())).collect();
            assert_eq!(w[0], "0.125");
            assert!(w[1].starts_with("sel(Flt(") && w[1].ends_with(", 0.75, 0.125)"), "{w:?}");
        })
        .unwrap();
    }

    #[test]
    fn control_flow_is_refused_by_name() {
        /* Built by hand: an `if` whose then arm holds a `break`. The arm is
         * walked (an `if` is converted), and the node inside it is refused
         * by name. */
        let instrs = [Op::ConstF32([0; 4]), Op::Alu(ir::AluOp::Flt, [ir::Src::id(ir::ValueId(0)); 3])];
        let nodes = [
            Node::Instr(ir::InstrId(0)),
            Node::Instr(ir::InstrId(1)),
            Node::If {
                cond: ir::Src::splat(ir::ValueId(1)),
                then_: ir::Range { first: 3, len: 1 },
                else_: ir::Range::EMPTY,
            },
            Node::Break,
        ];
        let body = ir::Range { first: 0, len: 3 };
        let m = Module { instrs: &instrs, comps: &[4, 4], nodes: &nodes, body, ..Module::empty(SHC_STAGE_FRAGMENT) };
        let r = store::with_scratch(|a| prepare_fragment(&m, a).err()).unwrap();
        assert_eq!(r, Some(PrepError::NotYet("break")));
        /* A return that is not the end of main. */
        let nodes = [Node::Return, Node::Break];
        let m = Module { nodes: &nodes, body: ir::Range { first: 0, len: 2 }, ..Module::empty(SHC_STAGE_FRAGMENT) };
        let r = store::with_scratch(|a| prepare_fragment(&m, a).err()).unwrap();
        assert_eq!(r, Some(PrepError::NotYet("return")));
    }
}
