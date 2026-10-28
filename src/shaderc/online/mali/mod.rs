// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610's (Valhall, arch 10) shader back end, first slice
//! (driver/docs/nanopc-t6-mali.md, milestone L3): the shapes of the
//! triangle programs, compiled to the bytes Mesa 26.2.1 makes for them.
//!
//! The shared front end runs in full, so a shader that is not valid GLSL ES
//! is refused with the front end's own verdict, as on every GPU. A valid
//! shader then goes through Mesa's Valhall pipeline, transcribed pass by
//! pass for the ops this slice selects:
//!
//! 1. [`select`] (a vertex shader, from [`nir`], its ALU operations through
//!    [`emit`], `bi_emit_alu`) and [`nir_fs`] (a fragment shader, on the same
//!    graph, selected through [`emit`]): the IR to BIR as Mesa selects it
//!    (the viewport transform, the IDVS split, the per-word output stores;
//!    the fp16 colour's vectorised arithmetic, ATEST and BLEND; the general
//!    lowering, S1; an `if` flattened by `nir_opt_peephole_select` or kept
//!    as blocks of [`bir`], `emit_if`: `BRANCHZ` to the else side, the
//!    jump, the phis at the join, S5);
//! 2. [`opt`]: the BIR passes after selection (the fneg/fabs modifier
//!    propagation, DCE, `FADD_IMM`), and [`fau`]: the push constants (blend
//!    constants, sysvals, then promoted immediates) and every constant's
//!    form;
//! 3. [`sched`]: the pre-RA pressure scheduler;
//! 4. [`ra`]: LCRA in the split register file (32 work registers);
//! 5. [`flow`]: message slots, flow waits, `.end`, last-use marks;
//! 6. [`pack`]: the blend-shader call, the words, the 128-byte padding.
//!
//! A vertex shader yields two programs from one source, as Mesa's IDVS
//! split does: the position variant at offset 0 and the varying variant
//! after it (empty when the shader writes no varying). The bytes are the
//! reference's: the tests compare every word with the shim driver's
//! programs (Mesa 26.2.1 on drm-shim) and pin the named differences from
//! the board's Mesa 25.0.7. What this slice does not take is refused with
//! a message of the "... is not lowered yet" class, one per gap
//! ([`MaliError`]); it is `-ENOTSUP`, never `-EINVAL`: the shader is not at
//! fault.
//!
//! The record a compile produces carries the program's facts beside the
//! code ([`MaliShader`], the `ShcShader::priv_` of this back end): preload
//! masks, work register counts, the secondary offset, the FAU words and the
//! attribute and varying layout, what the frame builder (src/mali/) puts
//! in the shader program descriptor and the push constants.

/// The order the front end sums `m * v` in for this back end
/// ([`super::ir::MatOrder`]): Mesa's GLSL path, the shim reference's.
pub const MAT_TIMES_VEC_ORDER: super::ir::MatOrder = super::ir::MatOrder::MesaGlsl;

pub mod bir;
pub mod blend;
pub mod emit;
pub mod fau;
pub mod flow;
pub mod isa;
pub mod link;
pub mod motion;
pub mod nir;
pub mod nir_fs;
pub mod opt;
pub mod pack;
pub mod preload;
pub mod ra;
pub mod sched;
pub mod select;

use super::arena::Arena;
use super::codegen::{lower_source, Blocks, CompileError, Which};
use crate::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use bir::{Ins, Prog};
pub use select::UboRow;

/// An invariant of the back end that did not hold: a compiler defect, not a
/// gap and not a verdict on the shader.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Defect {
    IrIndex,
    FauOverflow,
    BranchBeforeRa,
    Schedule,
    Vector,
    DualSource(u16),
    Unallocated,
    Flow,
    Operand,
    OperandName,
    OperandUnset,
    Encode,
    BlendNotLast,
    /// `pack` was handed a code buffer shorter than a padded program of
    /// [`bir::MAX_INSTRS`] ([`MAX_CODE_WORDS`] is sized for two).
    CodeBuffer,
    /// A program's last word packed as zero (`pack`: the `.end` word never
    /// is).
    ZeroLastWord,
    /// The scheduler was handed more instructions than [`bir::MAX_INSTRS`],
    /// which a [`bir::Prog`] cannot hold.
    ScheduleTable,
    /// A pseudo-op (`FABSNEG`) reached the packer: `opt::lower_opt_instructions`
    /// lowers every one first.
    PseudoOp,
    /// The Z/S preload program was asked for with neither plane
    /// (`preload::zs`).
    PreloadPlanes,
    /// A moved expression holds an operation `motion::encode` has no number
    /// for (`motion::OPS`): `motion::flag` marks only those movable.
    MovedOp,
}

/// Why the Valhall back end did not produce code: one variant per gap, each
/// with its own message ([`MaliError::message`], also its `Debug` form,
/// which is what the compile log prints). Every gap's message ends "is not
/// lowered yet"; [`MaliError::Defect`] is the one that does not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MaliError {
    /// A vertex shader with the link's attribute locations.
    LinkedVertexNotLowered,
    /// A blend variant: the Mali blends through its blend descriptor, and
    /// a blend shader (for the modes the descriptor cannot express) is not
    /// lowered.
    BlendVariantNotLowered,
    ControlFlow,
    Arithmetic,
    IntValue,
    /// A uniform of a type the back end does not read (not a float, vec2,
    /// vec3, vec4 or mat4).
    UniformType,
    UniformArray,
    /// A uniform read at an index chosen at run time, other than the one
    /// form referenced (nir_fs.rs: a mediump fragment shader's vec4 array
    /// at the block's start, indexed by an int uniform, read once).
    UniformIndexed,
    /// A vertex shader uniform that is not highp (Mesa would keep it as
    /// fp16).
    UniformPrecision,
    /// A mediump fragment shader's uniform other than the referenced shape
    /// (one vec4 read in its own lane pairs, Mesa's fp16 block). A highp
    /// shader reads its block as fp32 FAU words (nir_fs.rs `uniform_load`).
    FsUniform,
    /// A fragment shader whose float values are not all of one width
    /// ([`FsPrecision`]): Mesa computes the highp ones in fp32 and the
    /// mediump and lowp ones in fp16, converting where they meet.
    MixedPrecision,
    /// fp16 arithmetic on gl_FragCoord in a form no reference draws:
    /// gl_FragCoord is mediump in GLSL ES 1.00 (builtin_variables.cpp:
    /// 1333-1335), so Mesa lowers an operation whose other operands are
    /// constants or such results to fp16 (lower_precision.cpp:295-312;
    /// docs/notes/mali-fragcoord-mediump.md). Lowered: x and y in order in
    /// a mediump shader (prec_84), and in a highp one a compare of one lane
    /// (fbload_predif_fs_coord) and `floor`, `fract` of x and y (prec_82);
    /// any other operation, lane or pairing is this gap. An operation with
    /// a highp operand, or a copy to a highp variable first (femtovg's
    /// `ditherNoise(gl_FragCoord.xy)`), is fp32.
    FragCoordMediump,
    /// More uniform words than the FAU's 128 (Mesa loads the rest from the
    /// UBO).
    UboFallback,
    /// A constant product in a shape Mesa's algebraic rules move or fold:
    /// a constant times a product with a constant (GLSL IR's
    /// `reassociate_constant`), `fneg(fneg(a))`, a constant in a front-end
    /// `ffma`.
    ArithConstant,
    /// A chain GLSL's `do_rebalance_tree` may reassociate.
    Rebalance,
    /// A constant operation whose value is not a plain float.
    ConstantFold,
    /// A front-end `ffma` of a vertex shader that is not a matrix
    /// product's (the fragment stage lowers every `ffma`, nir_fs.rs).
    FusedMultiplyAdd,
    /// An invariant output.
    Invariant,
    /// A vertex shader look-up other than one 2D look-up at a level (a
    /// second one, a size query).
    TexVertex,
    /// A vertex shader look-up at a coordinate with a constant lane (a
    /// cube's intrinsics fold no constant; no reference draws the shape,
    /// for a 2D look-up either).
    TexVertexConstant,
    /// A fragment shader look-up other than `texture2D(sampler2D, vec2)`
    /// (a bias, a level, a projection, a cube map, a texture size).
    TexKind,
    /// A fragment shader look-up beside another in a form only a lone one
    /// takes (a mediump computed coordinate, a cube map), or more look-ups
    /// than [`nir_fs::MAX_LOOK_UPS`].
    TexCount,
    /// A coordinate other than a vec2 varying read whole or two constants.
    TexCoord,
    /// A varying read both as a coordinate and in the colour's arithmetic.
    TexCoordShared,
    /// The look-up's lanes other than in its own pairs, in order.
    TexLanes,
    /// A highp sampler (Mesa keeps its look-up 32-bit).
    TexHighp,
    /// A discard other than a highp shader's `if (a < b) discard;` with no
    /// else (nir_fs.rs `Fs::discard`): an unconditional one (`demote`'s
    /// `DISCARD.f32 0, 0 .eq`), one in a block with other statements, one
    /// behind an integer compare, a side of nothing but discards under
    /// another `if` (the GL front end's `iand` of the conditions), a
    /// mediump shader's.
    DiscardForm,
    /// A loop other than one a constant bound ends within Mesa's unroll
    /// limits (nir_fs.rs `walk_loop`): a run-time bound, more than 32
    /// passes, a body too large to unroll, a second `break` behind a
    /// run-time condition, or one `complex_unroll` takes whose guarded
    /// passes stay branches (`walk_guard`; the one it takes and flattens,
    /// gl_test's databreak, is lowered).
    LoopForm,
    IndexedLocal,
    SpecialInput,
    NoPosition,
    /// A gl_Position lane left unwritten.
    PositionLane,
    /// A vertex shader varying at location 16 or above (the selector keeps
    /// 16 varying slots).
    VaryingLocation,
    VaryingForm,
    /// A varying lane that reads no attribute (a constant or a uniform
    /// alone): Mesa's link moves or rewrites it.
    VaryingNotFromAttribute,
    /// Two varying lanes of equal value: Mesa's link merges them.
    VaryingDuplicate,
    /// A fragment colour of more vectorised operations and operands than
    /// nir_fs.rs keeps.
    FragmentTooLarge,
    /// A fragment colour operand in a place no reference draws
    /// (nir_fs.rs `ADMITTED`).
    FsOperandForm,
    FragOutput,
    /// A fragment shader that writes no colour.
    NoColour,
    Interpolation,
    ColourForm,
    ColourPair,
    /// A pair of a varying lane and the constant 0.0, which Mesa selects as
    /// `U16_TO_U32` (bi_lower_mkvec_swz.c:256-257), an op not transcribed.
    ColourZeroHalf,
    /// A constant fp16 does not hold: one a mediump tree reads that rounds
    /// to no normal fp16 (Mesa's `_mesa_float_to_half` makes a subnormal,
    /// a zero or an infinity, which no reference draws; a normal one is
    /// rounded to nearest even, mixprec_hello.expect.json), or an inexact
    /// colour or point size constant of a highp shader or a vertex stage.
    Fp16Inexact,
    FauPage,
    FauFull,
    Spill,
    /// More local variables than the selector's table (select.rs).
    TooManyLocals,
    /// More IR values than the selector's table (select.rs).
    TooManyValues,
    /// More distinct immediates outside the table than the constant
    /// histogram holds (fau.rs).
    TooManyConstants,
    /// A program longer than [`bir::MAX_INSTRS`].
    InstrTableFull,
    /// A program defining more than [`bir::MAX_NODES`] SSA values.
    ValueTableFull,
    /// A shader of more blocks than [`bir::MAX_BLOCKS`] or more `if`s than
    /// [`nir::MAX_IFS`].
    BlockTableFull,
    /// An `if` whose condition is not a comparison's value (a constant, a
    /// boolean operator's result), or an `if` after a `return`.
    IfForm,
    /// A comparison no reference draws (`!=`, an integer `==` or `!=`).
    CompareForm,
    /// A shape Mesa's algebraic rules rewrite into a form no reference
    /// draws: `max(x, 0.0)` (`fclamp_pos`), `clamp(x, -1.0, 1.0)`
    /// (`fsat_signed`), a negated `float(bool)` in a product.
    AlgebraicForm,
    /// More expressions than the record holds, or one larger, moved by
    /// Mesa's link into the vertex shader (motion.rs).
    MovedTooLarge,
    /// The scratch arena ran out (reported as capacity, not as a gap).
    OutOfScratch,
    Defect(Defect),
}

impl MaliError {
    /// Every gap, for the tests that check the messages are distinct.
    pub const GAPS: [MaliError; 58] = [
        MaliError::LinkedVertexNotLowered,
        MaliError::BlendVariantNotLowered,
        MaliError::ControlFlow,
        MaliError::Arithmetic,
        MaliError::IntValue,
        MaliError::UniformType,
        MaliError::UniformArray,
        MaliError::UniformIndexed,
        MaliError::UniformPrecision,
        MaliError::FsUniform,
        MaliError::MixedPrecision,
        MaliError::FragCoordMediump,
        MaliError::UboFallback,
        MaliError::ArithConstant,
        MaliError::Rebalance,
        MaliError::ConstantFold,
        MaliError::FusedMultiplyAdd,
        MaliError::Invariant,
        MaliError::TexVertex,
        MaliError::TexVertexConstant,
        MaliError::TexKind,
        MaliError::TexCount,
        MaliError::TexCoord,
        MaliError::TexCoordShared,
        MaliError::TexLanes,
        MaliError::TexHighp,
        MaliError::DiscardForm,
        MaliError::LoopForm,
        MaliError::IndexedLocal,
        MaliError::SpecialInput,
        MaliError::NoPosition,
        MaliError::PositionLane,
        MaliError::VaryingLocation,
        MaliError::VaryingForm,
        MaliError::VaryingNotFromAttribute,
        MaliError::VaryingDuplicate,
        MaliError::FragmentTooLarge,
        MaliError::FsOperandForm,
        MaliError::FragOutput,
        MaliError::NoColour,
        MaliError::Interpolation,
        MaliError::ColourForm,
        MaliError::ColourPair,
        MaliError::ColourZeroHalf,
        MaliError::Fp16Inexact,
        MaliError::FauPage,
        MaliError::FauFull,
        MaliError::Spill,
        MaliError::TooManyLocals,
        MaliError::TooManyValues,
        MaliError::TooManyConstants,
        MaliError::InstrTableFull,
        MaliError::ValueTableFull,
        MaliError::BlockTableFull,
        MaliError::IfForm,
        MaliError::CompareForm,
        MaliError::AlgebraicForm,
        MaliError::MovedTooLarge,
    ];

    /// The log text of each gap.
    pub const fn message(self) -> &'static str {
        match self {
            MaliError::LinkedVertexNotLowered => {
                "the Valhall back end of a vertex shader at the link's attribute locations is not lowered yet"
            }
            MaliError::BlendVariantNotLowered => "a Mali blend shader variant is not lowered yet",
            MaliError::ControlFlow => "Valhall: control flow (if, a loop, a return before the end) is not lowered yet",
            MaliError::Arithmetic => {
                "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet"
            }
            MaliError::IntValue => "Valhall: an integer or boolean value is not lowered yet",
            MaliError::UniformType => {
                "Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet"
            }
            MaliError::UniformArray => "Valhall: a uniform array is not lowered yet",
            MaliError::UniformIndexed => {
                "Valhall: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, is not lowered yet"
            }
            MaliError::UniformPrecision => {
                "Valhall: a vertex shader uniform that is not highp (Mesa's fp16 uniform) is not lowered yet"
            }
            MaliError::FsUniform => {
                "Valhall: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, is not lowered yet"
            }
            MaliError::MixedPrecision => {
                "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet"
            }
            MaliError::FragCoordMediump => {
                "Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet"
            }
            MaliError::UboFallback => {
                "Valhall: a uniform past the FAU's 128 words (Mesa loads it from the UBO) is not lowered yet"
            }
            MaliError::ArithConstant => {
                "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet"
            }
            MaliError::Rebalance => {
                "Valhall: three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered yet"
            }
            MaliError::ConstantFold => {
                "Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet"
            }
            MaliError::FusedMultiplyAdd => {
                "Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet"
            }
            MaliError::Invariant => {
                "Valhall: an invariant output (Mesa keeps its arithmetic exact, unfused) is not lowered yet"
            }
            MaliError::TexVertex => {
                "Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet"
            }
            MaliError::TexVertexConstant => {
                "Valhall: a vertex shader look-up at a coordinate with a constant lane (a shape no reference draws) is not lowered yet"
            }
            MaliError::TexKind => {
                "Valhall: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query) is not lowered yet"
            }
            MaliError::TexCount => {
                "Valhall: a fragment shader texture look-up beside another in a form only a lone look-up takes (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet"
            }
            MaliError::TexCoord => {
                "Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet"
            }
            MaliError::TexCoordShared => {
                "Valhall: a varying read both as a texture coordinate and in the colour's arithmetic is not lowered yet"
            }
            MaliError::TexLanes => {
                "Valhall: a texture result's lanes other than its own pairs in order (a swizzle, a lane alone) is not lowered yet"
            }
            MaliError::TexHighp => {
                "Valhall: a highp sampler (Mesa keeps its look-up 32-bit) is not lowered yet"
            }
            MaliError::DiscardForm => {
                "Valhall: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) is not lowered yet"
            }
            MaliError::LoopForm => {
                "Valhall: a loop other than one a constant bound ends within 32 passes (a break behind a run-time condition, which Mesa unrolls with guards) is not lowered yet"
            }
            MaliError::IndexedLocal => "Valhall: an indexed local variable is not lowered yet",
            MaliError::SpecialInput => {
                "Valhall: a built-in input (gl_FragCoord, gl_FrontFacing, gl_PointCoord, gl_VertexID, gl_InstanceID) is not lowered yet"
            }
            MaliError::NoPosition => "Valhall: a vertex shader that leaves gl_Position unwritten is not lowered yet",
            MaliError::PositionLane => "Valhall: a gl_Position lane left unwritten is not lowered yet",
            MaliError::VaryingLocation => "Valhall: a vertex shader varying at location 16 or above is not lowered yet",
            MaliError::VaryingForm => "Valhall: a varying lane left unwritten is not lowered yet",
            MaliError::VaryingNotFromAttribute => {
                "Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet"
            }
            MaliError::VaryingDuplicate => {
                "Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet"
            }
            MaliError::FsOperandForm => {
                "Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet"
            }
            MaliError::FragmentTooLarge => {
                "Valhall: a fragment colour of more than 64 vectorised operations and operands is not lowered yet"
            }
            MaliError::FragOutput => "Valhall: a fragment output other than gl_FragColor is not lowered yet",
            MaliError::NoColour => "Valhall: a fragment shader that leaves gl_FragColor unwritten is not lowered yet",
            MaliError::Interpolation => "Valhall: a flat, centroid or noperspective varying is not lowered yet",
            MaliError::ColourForm => {
                "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet"
            }
            MaliError::ColourPair => "Valhall: a colour pair of a constant then a varying lane is not lowered yet",
            MaliError::ColourZeroHalf => {
                "Valhall: a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32) is not lowered yet"
            }
            MaliError::Fp16Inexact => {
                "Valhall: a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly) is not lowered yet"
            }
            MaliError::FauPage => "Valhall: a push word past FAU page 0 is not lowered yet",
            MaliError::FauFull => "Valhall: a program whose sysvals overflow the FAU is not lowered yet",
            MaliError::Spill => {
                "Valhall: a program needing more than the split file's 32 work registers (the full file, spilling) is not lowered yet"
            }
            MaliError::TooManyLocals => "Valhall: a shader with more than 32 local variables is not lowered yet",
            MaliError::TooManyValues => {
                "Valhall: a shader of more than 2048 IR values (or 768 instructions before selection) is not lowered yet"
            }
            MaliError::TooManyConstants => {
                "Valhall: a program with more than 64 distinct immediates outside the table is not lowered yet"
            }
            MaliError::InstrTableFull => {
                "Valhall: a program longer than the back end's 384-instruction table is not lowered yet"
            }
            MaliError::ValueTableFull => {
                "Valhall: a program defining more than the back end's 384 SSA values is not lowered yet"
            }
            MaliError::BlockTableFull => {
                "Valhall: a shader of more than 32 basic blocks or 8 ifs is not lowered yet"
            }
            MaliError::IfForm => {
                "Valhall: an if whose condition is not a comparison (a constant, a boolean operator) is not lowered yet"
            }
            MaliError::CompareForm => {
                "Valhall: a comparison other than <, <=, >, >= and == of floats and < of ints (no reference draws it) is not lowered yet"
            }
            MaliError::AlgebraicForm => {
                "Valhall: a shape Mesa's algebraic rules rewrite without a reference (max(x, 0.0), clamp(x, -1.0, 1.0), a negated float(bool) in a product) is not lowered yet"
            }
            MaliError::MovedTooLarge => {
                "Valhall: more than 4 fragment expressions, or one of more than 8 operations, that Mesa's link moves into the vertex shader is not lowered yet"
            }
            MaliError::OutOfScratch => "Valhall: the scratch arena ran out",
            MaliError::Defect(_) => "Valhall back end defect: an invariant of the pipeline did not hold",
        }
    }
}

impl core::fmt::Debug for MaliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.message())?;
        if let MaliError::Defect(d) = self {
            write!(f, " ({d:?})")?;
        }
        Ok(())
    }
}

impl From<bir::Full> for MaliError {
    fn from(f: bir::Full) -> MaliError {
        match f {
            bir::Full::Instrs => MaliError::InstrTableFull,
            bir::Full::Values => MaliError::ValueTableFull,
            bir::Full::Blocks => MaliError::BlockTableFull,
        }
    }
}

fn refused(e: MaliError) -> CompileError {
    match e {
        MaliError::OutOfScratch => CompileError::OutOfMemory { which: Which::Scratch, left: 0 },
        e => CompileError::BackEnd(super::codegen::BackEndError::Mali(e)),
    }
}

/// The second program of a vertex shader (Mesa's `secondary_*` fields of
/// `pan_shader_info::vs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Secondary {
    /// Byte offset of the varying program from the start of the code.
    pub offset: u32,
    pub preload: u64,
    pub work_reg_count: u8,
}

/// Where a varying slot lives (`enum pan_varying_section`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Position,
    Generic,
}

/// One slot of a vertex shader's varying layout (`pan_varying_slot`): the
/// output (None = gl_Position, else the varying location), its 32-bit
/// components, its section and byte offset in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VaryingSlot {
    pub varying: Option<u8>,
    pub comps: u8,
    pub section: Section,
    pub offset: u16,
    /// The components in the layout, from `offset` on: every one of a
    /// vertex output, the ones a fragment shader reads (Mesa's link keeps
    /// only those, `compact_varyings`, nir_fs.rs `layout`).
    pub mask: u8,
}

/// [`MaliShader::magic`]'s value ("MALI").
pub const MALI_RECORD_MAGIC: u32 = 0x494c_414d;

/// What a compiled stage needs from the frame beside its code. `'k` is the
/// life of the memory the slices live in: the shader record's keep extent
/// for a record [`MaliShader::of`] finds, which is as long as the
/// `ShcShader` it is read from is held (the keep memory of a freed record
/// is the next compile's).
#[derive(Clone, Copy, Debug)]
pub struct MaliShader<'k> {
    /// [`MALI_RECORD_MAGIC`]: what [`MaliShader::of`] checks before it reads
    /// the rest.
    pub magic: u32,
    pub stage: u32,
    /// The registers the hardware preloads, bit n = rn (`info.preload`).
    pub preload: u64,
    /// `info.work_reg_count` (32: the split file).
    pub work_reg_count: u8,
    /// The varying program, when the vertex shader has one.
    pub secondary: Option<Secondary>,
    /// Vertex, with a gl_PointSize write: the byte offset of the position
    /// program without the point size store (Mesa's `no_psiz_offset`, the
    /// one a draw that is not of points runs, pan_cmdstream.c), after the
    /// position program that has it.
    pub no_psiz_offset: Option<u32>,
    /// The push constants, one 32-bit word each, in FAU order (slot = word
    /// / 2). A trace pads the block to whole slots with a zero word.
    pub fau: &'k [fau::FauWord],
    /// Vertex: (GL location, attribute record) of each attribute read.
    pub attributes: &'k [(u8, u8)],
    /// Vertex: the varying layout; fragment: the varyings read, as the
    /// vertex layout's generic slots.
    pub varyings: &'k [VaryingSlot],
    /// Fragment: colour output 0 is written as fp16 (`blend[0].type` =
    /// `nir_type_float16`).
    pub colour_f16: bool,
    /// Fragment: a varying is read in part ([`VaryingSlot::mask`]), so the
    /// program's vertex shader is built for these reads
    /// ([`compile_vs_for_reads`], at glLinkProgram).
    pub compacted: bool,
    /// Fragment: the expressions Mesa's link moves into the program's
    /// vertex shader (motion.rs), which is built for them too; vertex: the
    /// ones it was built for (none for a shader compiled alone).
    pub moved: &'k [motion::Moved],
    /// The sampler binding whose texture and sampler the stage's look-up
    /// reads at descriptor index [`nir_fs::TEX_INDEX`] of the stage's
    /// texture and sampler tables, if the shader makes one (a vertex
    /// shader's: both IDVS programs read the vertex tables). The first of
    /// [`MaliShader::samplers`].
    pub sampler: Option<u8>,
    /// Fragment: the sampler bindings at descriptor indices 0, 1, .. of the
    /// stage's texture and sampler tables, one per sampler the program
    /// reads, in binding order (texmore_2: `u_tex0`, `u_tex1`; each
    /// look-up's handle names its index, [`nir_fs::tex_handle`]); empty
    /// for a vertex shader, whose one is [`MaliShader::sampler`].
    pub samplers: &'k [u8],
    /// Fragment: the uniform block the program loads from memory (an
    /// `LD_PKA` of a uniform array element at a run-time index), in Mesa's
    /// packed layout ([`select::UboLayout`]'s rows), which the frame uploads
    /// as UBO 0 of the stage's resource table; empty when every uniform
    /// read is a pushed FAU word (Mesa's `ubo_mask` bit 0 clear).
    pub ubo: &'k [UboRow],
    /// Fragment: the shader's quads need their helper invocations (an
    /// implicit-LOD look-up: NIR's `needs_coarse_quad_helper_invocations`,
    /// which Mesa folds into `contains_barrier`, pan_compiler.c:276).
    pub helper_threads: bool,
    /// Fragment: the program discards (`info.fs.can_discard`, NIR's
    /// `uses_discard`, pan_compiler.c:255): the draw runs it even with no
    /// colour written, keeps forward pixel kill off, marks the coverage
    /// modified and updates depth/stencil late (src/mali/frame.rs,
    /// docs/notes/mali-discard.md).
    pub can_discard: bool,
}

impl MaliShader<'_> {
    /// The preloaded r48-r63 as the frame builder's `ShaderMeta::preload`
    /// takes them (`info.preload >> 48`, src/mali/frame.rs).
    pub fn preload_r48_r63(&self) -> u32 {
        (self.preload >> 48) as u32
    }

    /// The FAU register's 64-bit slot count (`fau()` in src/mali/frame.rs:
    /// the words rounded up to whole slots).
    pub fn fau_slots(&self) -> usize {
        self.fau.len().div_ceil(2)
    }

    /// The record of a stage this back end compiled, or None for a record
    /// of another origin or a freed one. The online back end's cache takes
    /// `priv_` for its own record when it publishes a shader and keeps this
    /// back end's pointer beside it (`online::backend::backend_priv`), which
    /// is where the GL core's copies lead. The record and its slices are
    /// borrowed for as long as `sh` is: they live in the shader's keep
    /// extent, which is freed only after its last holder lets go.
    pub fn of(sh: &ShcShader) -> Option<&MaliShader<'_>> {
        let p = super::backend::backend_priv(sh) as *const MaliShader<'static>;
        if p.is_null() || p.align_offset(core::mem::align_of::<MaliShader>()) != 0 {
            return None;
        }
        // SAFETY: an aligned pointer into the keep region, from a live cache
        // record or a code generator's tagged `priv_` (backend_priv returns
        // nothing else); this back end is the only producer of such a
        // record, written by `record` in the extent of the shader that
        // points at it. The magic is a check of that, not the proof.
        let r = unsafe { &*p };
        (r.magic == MALI_RECORD_MAGIC).then_some(r)
    }
}

/// The back end's fixed tables, each with its capacity: what one compile
/// may hold. A shader that needs more is refused by its own gap, never
/// truncated; how much of each the compiles so far used is the high-water
/// mark ([`high_water`]), logged each time it rises, so the tables are sized
/// from measurement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Table {
    /// The front end's IR values (nir.rs and select.rs `MAX_VALUES`).
    IrValues,
    /// The front end's local variables (`MAX_LOCALS`).
    Locals,
    /// The vertex program's NIR instructions (nir.rs `MAX_NODES`).
    NirNodes,
    /// One program's BIR instructions (bir.rs `MAX_INSTRS`).
    Instrs,
    /// One program's SSA values (bir.rs `MAX_NODES`).
    Values,
    /// A shader's push words (fau.rs `FAU_MAX_WORDS`).
    FauWords,
    /// One program's distinct immediates outside the table (fau.rs
    /// `MAX_CONSTANTS`).
    Constants,
    /// A stage's uniforms in the block layout (select.rs `MAX_UBO_ROWS`).
    UboRows,
    /// A fragment colour's vectorised nodes (nir_fs.rs `MAX_PAIR_NODES`).
    FragPairNodes,
}

impl Table {
    pub const ALL: [Table; 9] = [
        Table::IrValues,
        Table::Locals,
        Table::NirNodes,
        Table::Instrs,
        Table::Values,
        Table::FauWords,
        Table::Constants,
        Table::UboRows,
        Table::FragPairNodes,
    ];

    /// The name the log line gives it.
    pub const fn label(self) -> &'static str {
        match self {
            Table::IrValues => "IR values",
            Table::Locals => "locals",
            Table::NirNodes => "NIR instructions",
            Table::Instrs => "BIR instructions",
            Table::Values => "BIR values",
            Table::FauWords => "FAU words",
            Table::Constants => "immediates",
            Table::UboRows => "uniform rows",
            Table::FragPairNodes => "fragment pair nodes",
        }
    }

    pub const fn capacity(self) -> usize {
        match self {
            Table::IrValues => nir::MAX_VALUES,
            Table::Locals => nir::MAX_LOCALS,
            Table::NirNodes => nir::MAX_NODES,
            Table::Instrs => bir::MAX_INSTRS,
            Table::Values => bir::MAX_NODES,
            Table::FauWords => fau::FAU_MAX_WORDS,
            Table::Constants => fau::MAX_CONSTANTS,
            Table::UboRows => select::MAX_UBO_ROWS,
            Table::FragPairNodes => nir_fs::MAX_PAIR_NODES,
        }
    }
}

/// The most of each [`Table`] any compile has used, in [`Table::ALL`]'s
/// order.
static HIGH_WATER: [core::sync::atomic::AtomicUsize; Table::ALL.len()] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; Table::ALL.len()];

/// The most of `t` any compile has used since boot.
pub fn high_water(t: Table) -> usize {
    HIGH_WATER[t as usize].load(core::sync::atomic::Ordering::Relaxed)
}

/// What one compile used of each table: noted as it goes, published once
/// when the compile ends, refused or not ([`Marks::publish`]).
#[derive(Clone, Copy, Default)]
struct Marks([usize; Table::ALL.len()]);

impl Marks {
    fn note(&mut self, t: Table, used: usize) {
        let m = &mut self.0[t as usize];
        *m = (*m).max(used);
    }

    /// One program's instructions and values.
    fn prog(&mut self, p: &Prog<'_>) {
        self.note(Table::Instrs, p.peak);
        self.note(Table::Values, p.ssa_alloc as usize);
    }

    /// Raise each table's high-water mark to this compile's use, with a log
    /// line for each one that rose.
    fn publish(&self) {
        for t in Table::ALL {
            let used = self.0[t as usize];
            let was = HIGH_WATER[t as usize].fetch_max(used, core::sync::atomic::Ordering::Relaxed);
            if used > was {
                crate::glog!("mali shaderc table '{}': high-water {} of {}", t.label(), used, t.capacity());
            }
        }
    }
}

/// One program after every pass: its words (padding included), preload and
/// work registers.
struct Compiled {
    words: usize,
    preload: u64,
    work_reg_count: u8,
}

/// Run the passes after selection on `p`, writing the words into `out`.
fn finish(
    p: &mut Prog<'_>,
    fau_layout: &mut fau::Layout,
    fragment: bool,
    ubo: &select::UboLayout,
    scratch: &Arena<'_>,
    out: &mut [u64],
    marks: &mut Marks,
) -> Result<Compiled, MaliError> {
    let r = finish_passes(p, fau_layout, fragment, ubo, scratch, out, marks);
    marks.prog(p);
    marks.note(Table::FauWords, fau_layout.count);
    r
}

fn finish_passes(
    p: &mut Prog<'_>,
    fau_layout: &mut fau::Layout,
    fragment: bool,
    ubo: &select::UboLayout,
    scratch: &Arena<'_>,
    out: &mut [u64],
    marks: &mut Marks,
) -> Result<Compiled, MaliError> {
    fau::push_ubo(p, fau_layout, fragment, ubo)?;
    /* bifrost_compile.c `bi_compile_variant_nir`, in its order */
    opt::mod_prop_forward(p);
    opt::mod_prop_backward(p);
    opt::dce(p);
    fau::reorder_push(p, fau_layout)?;
    opt::lower_opt_instructions(p);
    opt::lower_mkvec_swz(p)?;
    opt::lower_isel(p)?;
    opt::fuse_add_imm(p);
    opt::fuse_cmp(p);
    marks.note(Table::Constants, fau::lower_constants(p, fau_layout, true)?);
    if fragment {
        flow::analyze_helper_requirements(p);
    }
    sched::pressure_schedule(p, scratch)?;
    if p.n == 0 {
        return Ok(Compiled { words: 0, preload: 0, work_reg_count: 0 });
    }
    let work_reg_count = ra::allocate(p, scratch)?;
    opt::control_flow(p);
    flow::assign_slots(p);
    flow::insert_flow_control_nops(p, fragment)?;
    flow::merge_flow(p, fragment);
    flow::mark_last(p);
    let preload = flow::preload_mask(p, fragment);
    if fragment {
        pack::lower_blend(p)?;
    }
    let words = pack::pack(p, out)?;
    Ok(Compiled { words, preload, work_reg_count })
}

/// The position program without its point size store, packed into `out`
/// (bifrost_compile.c:4724-4750): the store is removed, a NOP keeping its
/// flow control in its place when it had any; nothing is scheduled again.
/// The words written, 0 for a program with no point size store.
fn no_psiz_copy(p: &mut Prog<'_>, out: &mut [u64]) -> Result<usize, MaliError> {
    let Some(at) = p.as_slice().iter().position(|i| matches!(i.op, bir::Op::StoreI16 { .. })) else {
        return Ok(0);
    };
    let flow = p.as_slice()[at].flow;
    p.remove(at);
    if flow != bir::flow::NONE {
        p.insert(at, Ins::nop(flow))?;
    }
    pack::pack(p, out)
}

/// The longest code one stage makes: three programs of [`bir::MAX_INSTRS`]
/// (the position program, its no_psiz copy, the varying program), each
/// padded.
const MAX_CODE_WORDS: usize = 3 * (bir::MAX_INSTRS + 18);

/// The attribute records a vertex shader may read (the frame builder's
/// `MAX_ATTRIBUTES`).
const MAX_ATTRIBUTES: usize = 16;

/// A stage's code and record, before they are copied to keep.
pub struct StageCode<'s> {
    pub code: &'s [u64],
    pub info: MaliShader<'static>,
    fau: fau::Layout,
    /// The uniform block the program loads ([`MaliShader::ubo`]; empty
    /// when it loads none).
    ubo: select::UboLayout,
    attributes: [(u8, u8); MAX_ATTRIBUTES],
    nattr: usize,
    /// What the vertex shader's program drew on ([`nir::Prod`], a bit
    /// each): the population tests.rs references.
    pub prods: u32,
    /// The rows of emit.rs the fragment graph selected ([`emit::bit`]).
    pub rows: u128,
    /// What this compile used of each [`Table`], in [`Table::ALL`]'s order.
    pub marks: [usize; Table::ALL.len()],
    /// The position slot, then the generic varyings.
    varyings: [VaryingSlot; 1 + select::MAX_VARYINGS],
    nvar: usize,
    /// The expressions moved into the vertex shader ([`MaliShader::moved`]).
    moved: [motion::Moved; motion::MAX_MOVED],
    nmoved: usize,
    /// The fragment stage's sampler bindings ([`MaliShader::samplers`]).
    samplers: [u8; nir_fs::MAX_LOOK_UPS],
    nsamplers: usize,
}

fn prog<'s>(scratch: &Arena<'s>) -> Result<Prog<'s>, MaliError> {
    let ins = scratch.alloc_filled(bir::MAX_INSTRS, Ins::NOP).map_err(|_| MaliError::OutOfScratch)?;
    Ok(Prog::new(ins))
}

/// The width of a fragment shader's float values. Mesa takes it from GLSL
/// precision in its GLSL front end: `lower_precision` lowers a mediump or
/// lowp operation to fp16 and leaves a highp one at fp32 (compiler/glsl/
/// lower_precision.cpp:295-312, `handle_precision`; run for an ES stage
/// with the fp16 cap, glsl_parser_extras.cpp:2487-2491), and it retypes a
/// mediump temporary but no shader input (:1023-1060), so a varying keeps
/// its declared precision. The rule here is per shader: the default float
/// precision in force at `main` picks the path (highp: fp32 lanes,
/// nir_fs.rs `select_highp`; otherwise the fp16 pairs), and a value of the
/// other width is refused ([`MaliError::MixedPrecision`]) where the path
/// would compute it at the wrong one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FsPrecision {
    /// The default float precision at `main` is highp.
    pub highp: bool,
    /// A local, a global, a parameter or a function's result is declared
    /// at the other width, or a precision statement in a block changes it.
    pub mixed: bool,
    /// The varying locations declared at the other width (bit n: location
    /// n).
    pub other: u32,
    /// The sampler bindings declared highp (bit n: binding n), set by the
    /// compile from the declarations: a cube look-up of one is 32-bit in a
    /// highp shader (nir_fs.rs `walk_nodes`).
    pub highp_samplers: u32,
    /// The colour is an ES 3.00 output declared highp: Mesa stores it at
    /// fp32 (`blend[0].type` = `nir_type_float32`, `BLEND.v4.f32`), where
    /// gl_FragColor (mediump in ES 1.00) and a mediump output are fp16
    /// (case114_f.expect.json, alu_divloop.expect.json).
    pub colour_f32: bool,
    /// The colour is an ES 3.00 output declared `highp uvec4`: Mesa stores
    /// its four words as they are (`nir_type_uint32`, `BLEND.v4.u32`) and
    /// tests the constant alpha 1.0 (fbload_fbload_int.expect.json's
    /// `ATEST ... 0x3F800000`).
    pub colour_u32: bool,
    /// gl_FragCoord is highp: the shader is ES 3.00 or later, where Mesa
    /// declares the built-in highp, and mediump in ES 1.00
    /// (builtin_variables.cpp:1333-1335, `frag_coord_precision`;
    /// docs/notes/mali-fragcoord-mediump.md 1).
    pub frag_coord_highp: bool,
    /// The vertex shader's constant varying lanes Mesa's link propagates
    /// into this shader ([`link::VaryingConsts`]; none for a shader
    /// compiled alone).
    pub consts: link::VaryingConsts,
}

impl FsPrecision {
    /// The precision of fragment shader `src`, whose IR is `m`.
    pub fn of<'s>(m: &ir_mod::Module<'_>, src: &'s str, scratch: &Arena<'s>) -> Result<FsPrecision, MaliError> {
        use super::ast::{BaseType, Decl, DeclId, Precision, StorageQualifier};
        let defect = |_| MaliError::Defect(Defect::IrIndex);
        let pre = super::pp::preprocess(scratch, src).map_err(defect)?;
        let ast = super::parser::parse(scratch, pre.tokens).map_err(defect)?;
        let ast = super::parser::resolve_io(scratch, ast, super::sema::Stage::Fragment).map_err(defect)?;
        let float = |b: &BaseType<'_>| matches!(b, BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..));
        /* the default in force at each function's definition; main's picks */
        let mut dflt = Precision::Default;
        let mut at_main = None;
        let mut at_fns = [false; 4];
        for d in ast.top_level() {
            match *d {
                Decl::Precision { precision, base: BaseType::Float, .. } => dflt = precision,
                Decl::Function { name: "main", body: Some(_), .. } => at_main = Some(dflt),
                Decl::Function { body: Some(_), .. } => at_fns[dflt as usize] = true,
                _ => {}
            }
        }
        let highp = at_main.unwrap_or(dflt) == Precision::High;
        let other = |p: Precision| p != Precision::Default && (p == Precision::High) != highp;
        /* a function defined under the other default computes at its width */
        let mixed =
            [Precision::Low, Precision::Medium, Precision::High].into_iter().any(|p| at_fns[p as usize] && other(p));
        let mut f = FsPrecision { highp, mixed, frag_coord_highp: pre.version >= 300, ..FsPrecision::default() };
        let mut dflt = Precision::Default;
        for d in ast.top_level() {
            match *d {
                Decl::Precision { precision, base: BaseType::Float, .. } => dflt = precision,
                Decl::Var { storage: StorageQualifier::FragOut, ty, .. } if float(&ty.base) => {
                    let p = if ty.precision == Precision::Default { dflt } else { ty.precision };
                    f.colour_f32 |= p == Precision::High;
                }
                /* an unsigned integer output (fbload_int.frag's `highp uvec4
                 * o_color`): Mesa stores its words as computed,
                 * `BLEND.v4.u32` (fbload_fbload_int.expect.json) */
                Decl::Var { storage: StorageQualifier::FragOut, ty, .. }
                    if matches!(ty.base, BaseType::UVec(4)) && ty.precision == Precision::High =>
                {
                    f.colour_u32 = true;
                }
                Decl::Var { storage: StorageQualifier::Varying, ty, name, .. } if float(&ty.base) => {
                    let p = if ty.precision == Precision::Default { dflt } else { ty.precision };
                    let decl = m.inputs.iter().find(|i| i.kind == ir_mod::IoKind::Varying && i.name == name);
                    if let Some(i) = decl.filter(|_| other(p)) {
                        f.other |= 1u32.checked_shl(i.location as u32).ok_or(MaliError::VaryingLocation)?;
                    }
                }
                _ => {}
            }
        }
        /* every other declaration, at any scope: the varyings are above, a
         * uniform is FsUniform's, an ES 3.00 output the colour's */
        for (i, d) in ast.decls.iter().enumerate() {
            f.mixed |= match *d {
                Decl::Var { storage: StorageQualifier::None | StorageQualifier::Const, ty, .. } => {
                    float(&ty.base) && other(ty.precision)
                }
                Decl::Function { ret, .. } => float(&ret.base) && other(ret.precision),
                Decl::Precision { precision, base: BaseType::Float, .. } => {
                    !ast.top.contains(&DeclId(i as u32)) && other(precision)
                }
                _ => false,
            };
        }
        f.mixed |= ast.params.iter().any(|p| float(&p.ty.base) && other(p.ty.precision));
        Ok(f)
    }
}

/// Compile one stage's IR (the code and the facts; the record is made by
/// [`compile_linked_blocks`]).
///
/// `precision` is each uniform's declared precision (the front end's
/// `reflect::uniform_decls`): Mesa keeps a mediump or lowp uniform as fp16
/// (`LowerPrecisionFloat16Uniforms`), so a vertex shader's must be highp
/// and a fragment shader's is the fp16 block. `fs` is a fragment shader's
/// float precision ([`FsPrecision::of`]).
pub fn compile_module<'s>(
    m: &ir_mod::Module<'_>,
    precision: &[super::reflect::UniformDecl<'_>],
    fs: FsPrecision,
    scratch: &Arena<'s>,
) -> Result<StageCode<'s>, MaliError> {
    compile_module_on(m, precision, fs, None, &[], None, scratch)
}

/// [`compile_module`] of a vertex shader for a program: `reads` are the
/// program's fragment shader's (its record's generic varyings: location
/// and component mask), and only those components are stored, as Mesa's
/// link compacts them; `moved` the fragment shader's expressions the link
/// moves here (motion.rs).
pub fn compile_module_for<'s>(
    m: &ir_mod::Module<'_>,
    precision: &[super::reflect::UniformDecl<'_>],
    reads: Option<&[(u8, u8)]>,
    moved: &[motion::Moved],
    scratch: &Arena<'s>,
) -> Result<StageCode<'s>, MaliError> {
    compile_module_on(m, precision, FsPrecision::default(), reads, moved, None, scratch)
}

/// [`compile_module`] and [`compile_module_for`] in one; `order` a
/// fragment shader's varyings in its program's vertex order (link.rs
/// [`link::VaryingOrder`]).
fn compile_module_on<'s>(
    m: &ir_mod::Module<'_>,
    precision: &[super::reflect::UniformDecl<'_>],
    fs: FsPrecision,
    reads: Option<&[(u8, u8)]>,
    moved: &[motion::Moved],
    order: Option<link::VaryingOrder>,
    scratch: &Arena<'s>,
) -> Result<StageCode<'s>, MaliError> {
    let mut marks = Marks::default();
    if m.instrs.len() <= Table::IrValues.capacity() {
        marks.note(Table::IrValues, m.instrs.len());
    }
    if m.locals.len() <= Table::Locals.capacity() {
        marks.note(Table::Locals, m.locals.len());
    }
    let r = compile_stage(m, precision, fs, reads, moved, order, scratch, &mut marks);
    marks.publish();
    r.map(|sc| StageCode { marks: marks.0, ..sc })
}

fn compile_stage<'s>(
    m: &ir_mod::Module<'_>,
    precision: &[super::reflect::UniformDecl<'_>],
    fs: FsPrecision,
    reads: Option<&[(u8, u8)]>,
    moved: &[motion::Moved],
    order: Option<link::VaryingOrder>,
    scratch: &Arena<'s>,
    marks: &mut Marks,
) -> Result<StageCode<'s>, MaliError> {
    use super::ast::Precision;
    let prec = |name: &str| precision.iter().find(|u| u.name == name).map(|u| u.precision);
    let out = scratch.alloc_filled(MAX_CODE_WORDS, 0u64).map_err(|_| MaliError::OutOfScratch)?;
    let mut layout = fau::Layout::EMPTY;
    let empty = MaliShader {
        magic: MALI_RECORD_MAGIC,
        stage: m.stage,
        preload: 0,
        work_reg_count: 0,
        secondary: None,
        no_psiz_offset: None,
        fau: &[],
        attributes: &[],
        varyings: &[],
        colour_f16: false,
        compacted: false,
        moved: &[],
        sampler: None,
        samplers: &[],
        helper_threads: false,
        can_discard: false,
        ubo: &[],
    };
    let none = VaryingSlot { varying: None, comps: 0, section: Section::Position, offset: 0, mask: 0 };
    let mut sc = StageCode {
        code: &[],
        info: empty,
        fau: fau::Layout::EMPTY,
        ubo: select::UboLayout::NONE,
        attributes: [(0, 0); MAX_ATTRIBUTES],
        nattr: 0,
        varyings: [none; 1 + select::MAX_VARYINGS],
        nvar: 0,
        prods: 0,
        rows: 0,
        marks: [0; Table::ALL.len()],
        moved: [motion::Moved::NONE; motion::MAX_MOVED],
        nmoved: 0,
        samplers: [0; nir_fs::MAX_LOOK_UPS],
        nsamplers: 0,
    };
    let total = if m.stage == SHC_STAGE_VERTEX {
        if moved.len() > motion::MAX_MOVED {
            return Err(MaliError::MovedTooLarge);
        }
        sc.moved[..moved.len()].copy_from_slice(moved);
        sc.nmoved = moved.len();
        let mut vs = nir::build(m, scratch, reads, moved)?;
        marks.note(Table::NirNodes, vs.n);
        let ubo = select::UboLayout::vs(m, &vs)?;
        marks.note(Table::UboRows, ubo.rows());
        /* an `LD_PKA` reads the block from memory: the frame uploads it in
         * this layout (uniarray_vs) */
        if select::UboLayout::loaded(&vs) {
            sc.ubo = ubo;
        }
        for u in m.uniforms.iter().filter(|u| ubo.word(u.dword_off).is_some()) {
            if !matches!(prec(u.name), Some(Precision::High)) {
                return Err(MaliError::UniformPrecision);
            }
        }
        sc.prods = vs.prods;
        let mut p = prog(scratch)?;
        let sel = select::vs_variant(&mut p, &mut vs, 0, &ubo, scratch);
        marks.prog(&p);
        sel?;
        let pos = finish(&mut p, &mut layout, false, &ubo, scratch, out, marks)?;
        let no_psiz = no_psiz_copy(&mut p, &mut out[pos.words..])?;
        if no_psiz > 0 {
            sc.info.no_psiz_offset = Some((pos.words * 8) as u32);
        }
        let pos_words = pos.words + no_psiz;
        let mut p = prog(scratch)?;
        if vs.nvar > 0 {
            let sel = select::vs_variant(&mut p, &mut vs, 1, &ubo, scratch);
            marks.prog(&p);
            sel?;
        }
        let var = finish(&mut p, &mut layout, false, &ubo, scratch, &mut out[pos_words..], marks)?;
        sc.info.preload = pos.preload;
        sc.info.work_reg_count = pos.work_reg_count;
        sc.info.sampler = vs.sampler;
        if var.words > 0 {
            sc.info.secondary = Some(Secondary {
                offset: (pos_words * 8) as u32,
                preload: var.preload,
                work_reg_count: var.work_reg_count,
            });
        }
        /* the attribute records: one per location read, record = location
         * (pan_shc's `attributes`) */
        for d in m.inputs.iter() {
            let ir_mod::IoSlot::Attrib(loc) = d.slot() else { continue };
            let read = vs.order[..vs.len]
                .iter()
                .any(|&id| matches!(vs.nodes[id as usize].kind, nir::Kind::Input { loc: l, .. } if l == loc));
            if read {
                sc.attributes[sc.nattr] = (loc, loc);
                sc.nattr += 1;
            }
        }
        sc.varyings[0] = VaryingSlot { varying: None, comps: 4, section: Section::Position, offset: 0, mask: 0xf };
        sc.nvar = 1;
        for v in &vs.varyings[..vs.nvar] {
            let mask = vs.masks[sc.nvar - 1];
            sc.varyings[sc.nvar] = VaryingSlot {
                varying: Some(v.location),
                comps: mask.count_ones() as u8,
                section: Section::Generic,
                offset: v.offset,
                mask,
            };
            sc.nvar += 1;
        }
        pos_words + var.words
    } else {
        let mut fs = fs;
        for u in m.uniforms.iter().filter(|u| u.dword_off == crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
            if matches!(prec(u.name), Some(Precision::High)) {
                fs.highp_samplers |= 1u32.checked_shl(u32::from(u.binding)).unwrap_or(0);
            }
        }
        /* a highp output of a mediump shader: an fp32 store of fp16 values
         * (no reference) */
        if (fs.colour_f32 || fs.colour_u32) && !fs.highp {
            return Err(MaliError::MixedPrecision);
        }
        let mut f = nir_fs::walk(m, scratch, fs)?;
        /* the link's code motion (motion.rs): a highp shader's graph is
         * the linker's; a mediump shader converts its inputs first
         * (lower_precision), which stops every chain */
        if fs.highp {
            motion::move_into_producer(&mut f, m)?;
        }
        sc.moved = f.moved;
        sc.nmoved = f.nmoved;
        for t in f.look_ups() {
            let decl = m
                .uniforms
                .iter()
                .find(|u| u.dword_off == crate::shaderc::SHC_UNIFORM_NO_BLOCK && u.binding == t.sampler);
            /* a highp shader's look-up takes it at 32 bits */
            let taken = fs.highp;
            if !taken && decl.is_some_and(|u| matches!(prec(u.name), Some(Precision::High))) {
                return Err(MaliError::TexHighp);
            }
        }
        nir_fs::layout(&mut f, m, order)?;
        let mut p = prog(scratch)?;
        let sel = if fs.highp {
            /* the fp32 uniform block; a uniform declared at the other
             * precision is Mesa's fp16 one (LowerPrecisionFloat16Uniforms) */
            let ubo = select::UboLayout::fs32(m, &f.nir)?;
            for u in m.uniforms.iter().filter(|u| ubo.word(u.dword_off).is_some()) {
                f.mixed |= !matches!(prec(u.name), Some(Precision::High));
            }
            nir_fs::select_highp(&mut p, &mut f, m, &ubo, scratch).map(|()| ubo)
        } else if nir_fs::mask_colour(&f) {
            /* a mediump shader's colour of masks: the fp16 uniform block
             * (case 72's `u_in`, its fp16 half read by `F16_TO_S32`; case
             * 112's bool, a word) */
            let highp = |name: &str| matches!(prec(name), Some(Precision::High));
            nir_fs::vectorise_int_sources(&mut f)
                .and_then(|()| select::UboLayout::fs16(m, &f.nir, &highp))
                .and_then(|ubo| nir_fs::select_highp(&mut p, &mut f, m, &ubo, scratch).map(|()| ubo))
        } else {
            /* the fp16 uniform block (LowerPrecisionFloat16Uniforms); a
             * uniform declared highp is Mesa's fp32 one, refused */
            let highp = |name: &str| matches!(prec(name), Some(Precision::High));
            select::UboLayout::fs16(m, &f.nir, &highp)
                .and_then(|ubo| nir_fs::select(&mut p, &mut f, &ubo).map(|()| ubo))
        };
        marks.note(Table::NirNodes, f.nir.n);
        marks.note(Table::FragPairNodes, f.npairs);
        marks.prog(&p);
        let ubo = sel?;
        /* a value of the other width, when no other gap stood first */
        if f.mixed {
            return Err(MaliError::MixedPrecision);
        }
        marks.note(Table::UboRows, ubo.rows());
        let c = finish(&mut p, &mut layout, true, &ubo, scratch, out, marks)?;
        sc.info.preload = c.preload;
        sc.info.work_reg_count = c.work_reg_count;
        sc.info.colour_f16 = !fs.colour_f32 && !fs.colour_u32;
        sc.info.compacted = f.compacted;
        /* the bindings by descriptor index ([`nir_fs::layout`] numbers them) */
        for (_, t) in f.live_look_ups() {
            let i = usize::from(t.index);
            let slot = sc.samplers.get_mut(i).ok_or(MaliError::Defect(Defect::IrIndex))?;
            *slot = t.sampler;
            sc.nsamplers = sc.nsamplers.max(i + 1);
        }
        sc.info.sampler = sc.samplers[..sc.nsamplers].first().copied();
        /* a program with an `LD_PKA` reads the block from memory: the frame
         * uploads it in this layout (Mesa's `ubo_mask` bit 0) */
        if f.nubo > 0 {
            sc.ubo = ubo;
        }
        /* a look-up takes its LOD from the derivatives, unless it is a
         * cube's at a given level (`txl` has no implicit
         * derivative: 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs4/mesa/
         * mesa-es3cube.pandecode, `Requires helper threads: false`) or a
         * `texelFetch` (`TEX_FETCH` is none of `bi_instr_uses_helpers`';
         * fbload_fbload.trace, `Requires helper threads: false`) */
        sc.info.helper_threads = f.live_look_ups().any(|(_, t)| t.cube.is_none() && !t.fetch);
        sc.info.can_discard = f.discards > 0;
        sc.rows = f.rows;
        for r in &f.reads[..f.nreads] {
            sc.varyings[sc.nvar] = VaryingSlot {
                varying: Some(r.location),
                comps: r.mask.count_ones() as u8,
                section: Section::Generic,
                offset: r.offset,
                mask: r.mask,
            };
            sc.nvar += 1;
        }
        c.words
    };
    sc.code = &out[..total];
    sc.fau = layout;
    Ok(sc)
}

use super::ir as ir_mod;

/// The record of a compiled stage: the front end's reflection, the code in
/// `usc` (as 32-bit words, little endian, the position program first) and
/// the [`MaliShader`] in `priv_`, all in keep.
fn record(
    stage: u32,
    l: &super::lower::Lowered<'_>,
    sc: &StageCode<'_>,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    let oom_keep = |_| CompileError::OutOfMemory { which: Which::Keep, left: keep.left() };
    let m = &l.module;
    let mut sh = ShcShader::ZERO;
    let dwords = keep.alloc_filled(sc.code.len() * 2, 0u32).map_err(oom_keep)?;
    for (k, w) in sc.code.iter().enumerate() {
        dwords[2 * k] = *w as u32;
        dwords[2 * k + 1] = (*w >> 32) as u32;
    }
    sh.usc = dwords.as_ptr();
    sh.usc_dw = dwords.len() as u32;
    sh.meta_off = crate::shaderc::SHC_NO_META;
    sh.vs_psize_off = crate::shaderc::SHC_NO_PSIZE;
    sh.blend_consts_off = crate::shaderc::SHC_NO_BLEND_CONSTS;
    /* the varyings, by name, as the link pairs them */
    let ios = if stage == SHC_STAGE_VERTEX { m.outputs } else { m.inputs };
    let mut n = 0usize;
    for io in ios.iter().filter(|io| io.kind == ir_mod::IoKind::Varying) {
        if stage == SHC_STAGE_FRAGMENT
            && !sc.info.varyings.iter().chain(&sc.varyings[..sc.nvar]).any(|v| v.varying == Some(io.location))
        {
            continue;
        }
        /* at most 16: a location of 16 or above is refused (VaryingLocation) */
        let dst = sh.varyings.get_mut(n).ok_or(refused(MaliError::Defect(Defect::IrIndex)))?;
        crate::util::set_name(&mut dst.name, io.name.as_bytes());
        dst.location = io.location;
        dst.components = io.components;
        dst.interp = io.interp;
        n += 1;
    }
    sh.num_varyings = n as u8;
    if stage == SHC_STAGE_VERTEX {
        let mut n = 0usize;
        for &(loc, _) in &sc.attributes[..sc.nattr] {
            let Some(a) = l.reflection.attribs.iter().find(|a| a.location == loc) else { continue };
            let dst = sh.attribs.get_mut(n).ok_or(refused(MaliError::Defect(Defect::IrIndex)))?;
            crate::util::set_name(&mut dst.name, a.name.as_bytes());
            dst.location = a.location;
            dst.components = a.components;
            dst.vi_reg = a.location;
            dst.format = 0;
            dst.gl_type = a.gl_type;
            n += 1;
        }
        sh.num_attribs = n as u8;
    } else {
        sh.num_outputs = 1;
    }
    let rows = super::reflect::uniform_rows(&l.reflection, scratch)
        .map_err(|_| CompileError::OutOfMemory { which: Which::Scratch, left: 0 })?;
    if !rows.is_empty() {
        let kept = keep.alloc_copy(rows).map_err(oom_keep)?;
        sh.uniforms = kept.as_ptr();
        sh.num_uniforms = kept.len() as u8;
    }
    sh.uniform_dwords = l.reflection.uniform_dwords;
    let fau = keep.alloc_copy(sc.fau.as_slice()).map_err(oom_keep)?;
    let attributes = keep.alloc_copy(&sc.attributes[..sc.nattr]).map_err(oom_keep)?;
    let varyings = keep.alloc_copy(&sc.varyings[..sc.nvar]).map_err(oom_keep)?;
    let moved = keep.alloc_copy(&sc.moved[..sc.nmoved]).map_err(oom_keep)?;
    let ubo = keep.alloc_copy(sc.ubo.as_slice()).map_err(oom_keep)?;
    let samplers = keep.alloc_copy(&sc.samplers[..sc.nsamplers]).map_err(oom_keep)?;
    let info = MaliShader { fau, attributes, varyings, moved, ubo, samplers, ..sc.info };
    let rec = keep.alloc_copy(core::slice::from_ref(&info)).map_err(oom_keep)?;
    /* tagged: a record that has not been through the cache (the cache keeps
     * this pointer beside its own record when it publishes the shader) */
    sh.priv_ = super::backend::codegen_priv(rec.as_ptr() as *const core::ffi::c_void);
    Ok(sh)
}

/// One stage: the front end's verdict, then the Valhall pipeline.
pub fn compile_linked_blocks<'s>(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &[&str],
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let l = lower_source(stage, glsl, zero, scratch)?;
    /* the only variant key the Mali takes: a fragment shader for its
     * vertex shader's varying order (link.rs `VaryingOrder`) */
    let order = match (blend_key, link::VaryingOrder::of_key(blend_key)) {
        (0, _) => None,
        (_, Some(o)) if stage == SHC_STAGE_FRAGMENT => Some(o),
        _ => return Err(refused(MaliError::BlendVariantNotLowered)),
    };
    compile_lowered(stage, glsl, l, order, link::VaryingConsts::NONE, scratch, keep)
}

/// [`compile_linked_blocks`] past the front end: `consts` are the vertex
/// shader's constant lanes a fragment shader takes ([`compile_fs_for_vs`]).
fn compile_lowered<'s>(
    stage: u32,
    glsl: &[u8],
    l: super::lower::Lowered<'s>,
    order: Option<link::VaryingOrder>,
    consts: link::VaryingConsts,
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let st = if stage == SHC_STAGE_VERTEX { super::sema::Stage::Vertex } else { super::sema::Stage::Fragment };
    let src = core::str::from_utf8(glsl).map_err(|_| refused(MaliError::Defect(Defect::IrIndex)))?;
    let src = scratch.alloc_str(src).map_err(|_| refused(MaliError::OutOfScratch))?;
    let decls = super::reflect::interface(scratch, st, src).map_err(|_| refused(MaliError::Defect(Defect::IrIndex)))?;
    let fs = if stage == SHC_STAGE_FRAGMENT {
        FsPrecision { consts, ..FsPrecision::of(&l.module, src, scratch).map_err(refused)? }
    } else {
        FsPrecision::default()
    };
    let sc = compile_module_on(&l.module, decls.uniforms, fs, None, &[], order, scratch).map_err(refused)?;
    let sh = record(stage, &l, &sc, scratch, keep)?;
    let blocks = (l.reflection.blocks, l.reflection.block_members, [l.reflection.uniforms, &[][..]]);
    Ok((sh, blocks))
}

/// The varying lanes vertex shader `glsl` stores as constants, which Mesa's
/// link propagates into the program's fragment shader (nir_opt_varyings,
/// [`link::VaryingConsts`]): the first step of the link that builds both
/// stages for the program. Then [`compile_fs_for_vs`] with them, then
/// [`compile_vs_for_reads`] with that fragment shader's reads, which no
/// longer hold the constant lanes (case114_f.expect.json). The lanes of
/// uniforms alone ride along, marked flat (`VaryingConsts::flat`): the
/// fragment shader loads them `.src_flat32` (varyconst_117_uniidxf).
pub fn vs_constants(glsl: &[u8], scratch: &Arena<'_>) -> Result<link::VaryingConsts, CompileError> {
    let l = lower_source(SHC_STAGE_VERTEX, glsl, &[], scratch)?;
    /* built as for a fragment shader that reads nothing: no lane is stored,
     * so none is refused for what it holds */
    let vs = nir::build(&l.module, scratch, Some(&[]), &[]).map_err(refused)?;
    if vs.consts.full() {
        return Err(refused(MaliError::VaryingNotFromAttribute));
    }
    Ok(vs.consts)
}

/// A fragment shader for a program whose vertex shader stores `consts`
/// ([`vs_constants`]): each read of one of those lanes is the constant, as
/// after Mesa's link, and the varyings it reads are laid out without them
/// (the record's reads, compacted, are what [`compile_vs_for_reads`]
/// builds the program's vertex shader for).
pub fn compile_fs_for_vs<'s>(
    glsl: &[u8],
    consts: link::VaryingConsts,
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let l = lower_source(SHC_STAGE_FRAGMENT, glsl, &[], scratch)?;
    compile_lowered(SHC_STAGE_FRAGMENT, glsl, l, None, consts, scratch, keep)
}

/// A vertex shader for a program whose fragment shader reads `reads` (its
/// record's generic varyings: location and component mask): only those
/// components are stored, compacted as Mesa's link does
/// ([`compile_module_for`]); `moved` are the fragment shader's moved
/// expressions as [`ProgramReads::moved`] carries them (motion.rs
/// `encode`), computed here and stored in their lanes.
pub fn compile_vs_for_reads<'s>(
    glsl: &[u8],
    reads: &[(u8, u8)],
    moved: &[u8],
    scratch: &Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let l = lower_source(SHC_STAGE_VERTEX, glsl, &[], scratch)?;
    let src = core::str::from_utf8(glsl).map_err(|_| refused(MaliError::Defect(Defect::IrIndex)))?;
    let src = scratch.alloc_str(src).map_err(|_| refused(MaliError::OutOfScratch))?;
    let decls = super::reflect::interface(scratch, super::sema::Stage::Vertex, src)
        .map_err(|_| refused(MaliError::Defect(Defect::IrIndex)))?;
    let mut mv = [motion::Moved::NONE; motion::MAX_MOVED];
    let n = motion::decode(moved, &mut mv).map_err(refused)?;
    let sc = compile_module_for(&l.module, decls.uniforms, Some(reads), &mv[..n], scratch).map_err(refused)?;
    let sh = record(SHC_STAGE_VERTEX, &l, &sc, scratch, keep)?;
    let blocks = (l.reflection.blocks, l.reflection.block_members, [l.reflection.uniforms, &[][..]]);
    Ok((sh, blocks))
}

/// What a program's vertex shader is built for ([`compile_vs_for_reads`]):
/// the fragment shader's reads (location, component mask) and its moved
/// expressions as bytes (motion.rs `encode`: the record key's form).
#[derive(Clone, Copy, Debug)]
pub struct ProgramReads {
    pub reads: [(u8, u8); select::MAX_VARYINGS],
    pub n: usize,
    pub moved: [u8; motion::MAX_BYTES],
    pub nmoved: usize,
}

impl ProgramReads {
    pub fn reads(&self) -> &[(u8, u8)] {
        &self.reads[..self.n]
    }

    pub fn moved(&self) -> &[u8] {
        &self.moved[..self.nmoved]
    }
}

/// The reads a program's vertex shader is built for, when the fragment
/// shader `fs` (a record of this back end) reads a varying in part or has
/// code the link moves into the vertex shader; None otherwise (the vertex
/// shader's own layout is the program's). `Some(Err)`: the record's moved
/// expressions do not encode (a defect, never a plain vertex shader).
pub fn partial_reads(fs: &ShcShader) -> Option<Result<ProgramReads, MaliError>> {
    let r = MaliShader::of(fs)?;
    (r.stage == SHC_STAGE_FRAGMENT && (r.compacted || !r.moved.is_empty())).then(|| fragment_reads(r))
}

/// The generic varyings a fragment shader's record reads and its moved
/// expressions, as [`compile_vs_for_reads`] takes them.
pub fn fragment_reads(r: &MaliShader<'_>) -> Result<ProgramReads, MaliError> {
    let mut out =
        ProgramReads { reads: [(0, 0); select::MAX_VARYINGS], n: 0, moved: [0; motion::MAX_BYTES], nmoved: 0 };
    for v in r.varyings.iter().filter(|v| v.section == Section::Generic) {
        if let (Some(loc), Some(o)) = (v.varying, out.reads.get_mut(out.n)) {
            *o = (loc, v.mask);
            out.n += 1;
        }
    }
    out.nmoved = motion::encode(r.moved, &mut out.moved)?;
    Ok(out)
}

/// A vertex shader at the attribute locations the link chose: the front
/// end's verdict, then the gap.
pub fn compile_vs_mapped_blocks<'s>(
    glsl: &[u8],
    _map: &crate::shaderc::ShcAttribMap,
    scratch: &Arena<'s>,
    _keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    lower_source(SHC_STAGE_VERTEX, glsl, &[], scratch)?;
    Err(refused(MaliError::LinkedVertexNotLowered))
}

/* the test modules last: clippy's items_after_test_module (a newer clippy
 * than the gate host's flagged them at the top of the file) */
#[cfg(test)]
mod tests;
/// The expect.json reader the tests share (the triangle sample's test too).
#[cfg(test)]
pub(crate) mod tests_ref {
    pub fn region_words(json: &str, name: &str) -> Vec<u64> {
        super::tests::region(json, name).expect("the region").words
    }
}
