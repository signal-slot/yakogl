// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// SPDX-FileCopyrightText: 2010 Intel Corporation
// Derived from Mesa 26.2.1 src/broadcom/compiler/ (nir_to_vir.c, vir_register_allocate.c) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Selection: a prepared scalar program ([`super::prepare`]) to QPU
//! operations on virtual registers, one [`VOp`] per add-unit operation,
//! mul-unit operation, signal or thread switch. Pairing them into 64-bit
//! words is [`super::sched`]'s job, registers are [`super::alloc`]'s.
//!
//! # Mesa's conventions, where the driver depends on them
//!
//! The driver (`driver/src/v3d/v3d_frame.c`) builds the shader state record
//! and the uniform stream for Mesa's code and for ours alike, so the I/O
//! follows Mesa 26.2.1 (`src/broadcom/compiler/`, MIT, Copyright Broadcom and
//! Intel), cited at each item:
//!
//! * **Uniforms** are `(contents, data)` with Mesa's `enum quniform_contents`
//!   values ([`QUNIFORM_CONSTANT`], [`QUNIFORM_UNIFORM`] and the four viewport ones,
//!   v3d_compiler.h:180-206), consumed in program order by `ldunifrf` and by
//!   a `tlbu` write.
//! * **Vertex inputs**: each attribute record `r` takes `vattr_sizes[r]` VPM
//!   rows, the records in order; component `c` of record `r` is row
//!   `sum(vattr_sizes[..r]) + c`, read with `ldvpmv_in` from an index the
//!   uniform stream supplies (nir_to_vir.c:2895-2923, ntq_setup_vs_inputs
//!   :2372-2426). `vattr_sizes[r]` is the highest component read, plus one
//!   (:2388-2393).
//! * **Vertex outputs** on 4.2 (v3d_nir_lower_io.c:400-434, 493-585): the
//!   coordinate shader writes clip x y z w at rows 0..3 and the viewport-
//!   scaled x y at 4, 5; the vertex shader writes scaled x y at 0, 1, Zs at
//!   2, 1/Wc at 3 and the varyings from row 4 in the fragment shader's input
//!   order. Scaled x is `ftoiz(ffloor(x * X_SCALE * (1/w)))` (the 4.2 quirk
//!   at :529-543), Zs is `z * Z_SCALE * (1/w) + Z_OFFSET` (:550-556). A
//!   varying the FS reads and the VS never writes is stored as 0
//!   (:562-584). Every VPM index is a CONSTANT uniform (vir_VPM_WRITE,
//!   nir_to_vir.c:2156-2161), and the program ends with `vpmwt` (GFXH-1684,
//!   emit_vert_end :2164-2170).
//! * **Varyings** in the fragment shader: each `ldvary` reads the next
//!   varying in VPM order, so they are issued in the input order, and a
//!   smooth one is `ldvary * W + C`, W from the payload register rf0
//!   (nir_to_vir.c:4853) and C arriving in r5 (qpu_validate.c:193-200).
//! * **Fragment output**: after the last thread switch (a pair of THRSW,
//!   vir_emit_last_thrsw :4997-5034), `vfpack` of (r, g) then (b, a) to the
//!   TLB. The first write carries the TLB config uniform when the config is
//!   not the default 0xffffffff (vir_emit_tlb_color_write :1953-2036). For an
//!   f16 render target 0 with a vec4 colour, per-pixel, the config IS the
//!   default, so no uniform is consumed, which is what Mesa's code for
//!   `gl_FragColor` shows.
//!
//! The fragment shader is compiled for the render target the M5 driver
//! draws: render target 0, RGBA8 (Mesa's f16 TLB path, no R/B swap, no MSAA),
//! for triangles. Anything else is refused by the chain before selection.
//!
//! Constants: a small immediate where the table has the value and the stage
//! is a fragment shader, a CONSTANT uniform otherwise. Mesa turns uniforms
//! into small immediates only in fragment and compute shaders
//! (vir_opt_small_immediates.c:46-52), so no Mesa vertex shader in the corpus
//! reads one, and this compiler does not either.

use super::isa::{small_imm_index, small_imm_value, AddOp, BranchCond, Cond, Msfign, MulOp, Pf, Uf};
use super::prepare::{Prepared, SOp, S};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::ir::AluOp;

/// `enum quniform_contents` (v3d_compiler.h:180-206): the values this
/// compiler emits.
pub const QUNIFORM_CONSTANT: u32 = 0;
/// A dword of the stage's uniform block. Mesa's `data` is a dword offset
/// into Gallium's constant buffer (try_emit_uniform, nir_to_vir.c:2789-2819);
/// ours is the dword offset into the stage's uniform block as the IR states
/// it (`LoadUniform::dword_off` plus the lane), the layout the GL core keeps
/// and `eval.rs` reads as `Inputs::uniforms`.
pub const QUNIFORM_UNIFORM: u32 = 1;
pub const QUNIFORM_VIEWPORT_X_SCALE: u32 = 2;
pub const QUNIFORM_VIEWPORT_Y_SCALE: u32 = 3;
pub const QUNIFORM_VIEWPORT_Z_OFFSET: u32 = 4;
pub const QUNIFORM_VIEWPORT_Z_SCALE: u32 = 5;
/// A texture look-up's TMU configuration parameter 0 and 1 words
/// (v3d_compiler.h's enum; docs/online-shaderc/v3d-backend.md, "Textures"):
/// `data` = the sampler's reflection binding << 24 | the packed parameter.
/// The driver ORs in the texture's (P0) or the sampler's (P1) state address.
pub const QUNIFORM_TMU_CONFIG_P0: u32 = 40;
pub const QUNIFORM_TMU_CONFIG_P1: u32 = 41;
/// A texture's base-level width, and (+1) height, for textureSize: `data`
/// is the sampler's reflection binding (the number P0/P1 carry in bits
/// 24..31), the value the driver's get_texture_size gives
/// (v3d_uniforms.c:50-90: u_minify(width0, first_level)). Mesa's values:
/// printed by a program compiled against v3d_compiler.h (Mesa 26.2.1),
/// with QUNIFORM_TEXTURE_DEPTH 46, _ARRAY_SIZE 47 and _LEVELS 48 after.
pub const QUNIFORM_TEXTURE_WIDTH: u32 = 44;
pub const QUNIFORM_TEXTURE_HEIGHT: u32 = 45;
/// gl_FragCoord.y's window transform, two V3D-private codes above Mesa's
/// enum (Mesa's gallium path gets the pair from st/mesa's
/// FB_WPOS_Y_TRANSFORM state uniform instead, which the V3D compiler never
/// sees): y = FYCD * scale + offset. The driver writes scale -1.0 and offset
/// the target's height for a window (GL's lower-left origin on the V3D's
/// upper-left one), +1.0 and 0.0 for a framebuffer object. `data` is 0 in
/// both; any other value is refused by the driver (the lead's choice (A),
/// 2026-09-24, with v3d-glue's V3D_QUNIFORM_FB_WPOS_Y_* in v3d_frame.h,
/// v3d_job.h and ffi.rs; docs/online-shaderc/v3d-backend.md, "gl_FragCoord").
pub const QUNIFORM_FB_WPOS_Y_SCALE: u32 = 0x1_0000;
/// gl_PointCoord's y transform, y = y_raw * SCALE + OFFSET (Mesa's
/// nir_lower_pntc_ytransform, :76-81, from STATE_FB_PNTC_Y_TRANSFORM): V3D-
/// private codes next to the WPOS pair, data 0, filled per target by the
/// job builder (v3d-glue: window -1.0 / 1.0, FBO 1.0 / 0.0).
pub const QUNIFORM_FB_PNTC_Y_SCALE: u32 = 0x1_0002;
pub const QUNIFORM_FB_PNTC_Y_OFFSET: u32 = 0x1_0003;
pub const QUNIFORM_FB_WPOS_Y_OFFSET: u32 = 0x1_0001;
/// The TMU write addresses a 2D look-up uses (V3D_QPU_WADDR_TMUS/T/B, and
/// TMUSLOD for one at an explicit level; qpu_instr.h).
pub const WADDR_TMUS: u8 = 33;
pub const WADDR_TMUSLOD: u8 = 42;
/// A cube look-up's r (the direction's z) and its retiring write
/// (V3D_QPU_WADDR_TMUR / TMUSCM, qpu_instr.h).
pub const WADDR_TMUR: u8 = 35;
pub const WADDR_TMUSCM: u8 = 40;
/// TMU Config Parameter 2 (v3d_packet.xml, 4.2) for a cube look-up at an
/// explicit level: op Regular (15, bits 20..23) and Disable AutoLOD (bit 1),
/// Mesa's p2 for a txl on a cube (v3d_tex.c:146-149, 303-306, 367-368).
pub const TMU_P2_CUBE_LOD: u32 = (15 << 20) | (1 << 1);
pub const WADDR_TMUT: u8 = 34;
/// A 2D-array look-up's layer (V3D_QPU_WADDR_TMUI).
pub const WADDR_TMUI: u8 = 36;
pub const WADDR_TMUB: u8 = 37;

/// One entry of a uniform stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Uniform {
    pub contents: u32,
    pub data: u32,
}

impl Uniform {
    pub const fn constant(data: u32) -> Uniform {
        Uniform { contents: QUNIFORM_CONSTANT, data }
    }
}

/// The SFU operations, which on 4.2 the add unit issues with a register
/// destination (`v3d_qpu_instr_is_sfu`, qpu_instr.c:788-803): their result
/// is physical (vir_register_allocate.c:1198-1210), readable two instructions
/// later (qpu_schedule.c:1870-1871), and they stay out of a thread switch's
/// delay slots.
pub fn is_sfu(op: AddOp) -> bool {
    matches!(op, AddOp::Recip | AddOp::Rsqrt | AddOp::Exp | AddOp::Log | AddOp::Sin | AddOp::Rsqrt2)
}

/// The comparisons [`Sel::compare`] turns into a flag push.
fn is_comparison(op: AluOp) -> bool {
    matches!(
        op,
        AluOp::Flt
            | AluOp::Fle
            | AluOp::Feq
            | AluOp::Fne
            | AluOp::Ieq
            | AluOp::Ine
            | AluOp::Ilt
            | AluOp::Ile
            | AluOp::Ult
    )
}

/// A virtual register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct V(pub u32);

/// Where an operand comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand {
    V(V),
    /// The absolute value of a register, through the f32 ABS input unpack
    /// (Mesa's fabs: FMOV with V3D_QPU_UNPACK_ABS, nir_to_vir.c:1779-1789).
    Abs(V),
    /// A small-immediate table index.
    SmallImm(u8),
    /// A payload register the fragment shader starts with: rf0 is W, rf2
    /// is Z (nir_to_vir.c:4853-4858; rf1, the centroid W, is not read by
    /// anything this cut selects).
    Payload(u8),
    /// r5: the C term of the `ldvary` issued before this operation.
    R5,
    /// The low (`Lo`) or high (`Hi`) f16 half of a register, as an f32,
    /// through the L/H input unpack: a look-up's result word holds two
    /// channels (Mesa reads them as `r4.l` / `r4.h`).
    Lo(V),
    Hi(V),
    /// No operand (an operation that reads fewer than two).
    None,
}

impl Operand {
    /// The virtual register the operand reads, if any.
    pub fn vreg(&self) -> Option<V> {
        match *self {
            Operand::V(v) | Operand::Abs(v) | Operand::Lo(v) | Operand::Hi(v) => Some(v),
            _ => None,
        }
    }
}

/// Where a result goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dst {
    V(V),
    /// The TLB colour write.
    Tlb,
    /// The TLB colour write that consumes the next uniform as its config.
    Tlbu(Uniform),
    /// The magic "no register" (`vpmwt`'s destination).
    Nop,
    /// No destination field (STVPM*).
    None,
    /// A magic write address (the TMU registers).
    Magic(u8),
}

/// One unit of work for the scheduler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VOp {
    Add {
        op: AddOp,
        dst: Dst,
        a: Operand,
        b: Operand,
    },
    Mul {
        op: MulOp,
        dst: Dst,
        a: Operand,
        b: Operand,
    },
    /// `ldunifrf`: the next uniform into a register.
    Ldunif {
        dst: V,
        u: Uniform,
    },
    /// `ldvary`: the next varying's value into a register the next
    /// instruction, its C term into r5 two instructions later
    /// (qpu_validate.c:193-200).
    Ldvary {
        dst: V,
    },
    /// One THRSW signal. Two in a row are the last-thread-switch pair.
    Thrsw,
    /// An operation that writes no register and pushes a flag: FCMP on the
    /// add unit, or MOV on the mul unit to test a value against zero
    /// (ntq_emit_comparison, nir_to_vir.c:1173-1234; ntq_emit_bool_to_cond,
    /// :1262-1282).
    Flags {
        unit: FlagOp,
        a: Operand,
        b: Operand,
        pf: Pf,
        /// Instead of a push (`pf` None): fold the result into flag A
        /// (AND / NOR with its Z, N or C), the second of two loop exits
        /// merged into one branch ([`merge_exits`]).
        uf: Uf,
    },
    /// `dst = src` in the lanes where `cond` holds on the flags the last
    /// [`VOp::Flags`] pushed; the other lanes keep `dst` (vir_SEL's second
    /// MOV, v3d_compiler.h:1557-1564). A read-modify-write of `dst`, so a
    /// use of it and not a second definition.
    CondMov {
        dst: V,
        src: Operand,
        cond: Cond,
    },
    /// `setmsf -, src`: the lanes' multisample flags become `src`, where
    /// `cond` holds on the last pushed flags (`Cond::None`: every lane).
    /// `src` 0 is a discard (nir_to_vir.c:3753-3792: terminate and
    /// terminate_if, `vir_SETMSF_dest(nop, 0)` under the bool's condition).
    Setmsf {
        src: Operand,
        cond: Cond,
    },
    /// The `wrtmuc` signal: the next uniform, `u`, is a TMU configuration
    /// word (v3d_tex.c vir_WRTMUC).
    Wrtmuc {
        u: Uniform,
    },
    /// `ldtmu`: the next word of the oldest look-up's result into `dst`.
    Ldtmu {
        dst: V,
    },
    /// The top of a counted loop's body (prepare.rs SOp::LoopHead): the
    /// back edge's target, a word nothing before it is merged into.
    LoopHead,
    /// `dst = src` on a register already defined: a loop-carried value at
    /// the end of a pass, or the loop counter. A use of `dst`, not a
    /// definition (like CondMov), so its register lives across the loop.
    Copy {
        dst: V,
        src: Operand,
    },
    /// The counted loop's back edge: a REL branch to the matching
    /// [`VOp::LoopHead`] where `cond` holds on the flags the last
    /// [`VOp::Flags`] pushed, with the uniform stream moved back too
    /// (qpu_schedule.c:2916-2983). `msfign`: which lanes do not vote.
    BackEdge {
        cond: BranchCond,
        msfign: Msfign,
    },
    /// A forward REL branch to [`VOp::Label`] `label`, taken where `cond`
    /// holds on the flags the last [`VOp::Flags`] pushed (`Always`: every
    /// time), with the uniform stream moved past the uniforms it skips: a
    /// uniform branch's jump to its else arm, or over it (prepare.rs
    /// SOp::IfBranch; ntq_emit_uniform_if, nir_to_vir.c:4293-4390).
    FwdBranch {
        cond: BranchCond,
        msfign: Msfign,
        label: u32,
    },
    /// A forward branch's target: the next word starts here, and nothing
    /// before it is merged into a word after it, or the other way round.
    Label(u32),
}

/// The operation a [`VOp::Flags`] pushes its flag with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlagOp {
    Add(AddOp),
    Mul(MulOp),
}

impl VOp {
    /// The virtual register this operation defines.
    pub fn def(&self) -> Option<V> {
        match *self {
            VOp::Add { dst: Dst::V(v), .. } | VOp::Mul { dst: Dst::V(v), .. } => Some(v),
            VOp::Ldunif { dst, .. } | VOp::Ldvary { dst } | VOp::Ldtmu { dst } => Some(dst),
            _ => None,
        }
    }
    /// The operands it reads.
    pub fn uses(&self) -> [Operand; 2] {
        match *self {
            VOp::Add { a, b, .. } | VOp::Mul { a, b, .. } | VOp::Flags { a, b, .. } => [a, b],
            VOp::CondMov { dst, src, .. } => [src, Operand::V(dst)],
            VOp::Setmsf { src, .. } => [src, Operand::None],
            VOp::Copy { dst, src } => [src, Operand::V(dst)],
            _ => [Operand::None; 2],
        }
    }
}

/// Why selection stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectError {
    /// A varying the program reads is not in the input list it was given.
    VaryingNotAnInput,
    /// An attribute the program reads is not in the record list it was given.
    AttributeWithoutRecord,
    /// A VPM row past what this cut writes (16 attribute records, 64 output
    /// rows).
    VpmRowOutOfRange,
    /// A VPM output was written before every attribute was read: the input
    /// and output segments are shared, so the write could overwrite an input
    /// row still to be read.
    VpmWriteBeforeInputRead,
    /// A small-immediate index with no value in the table (a defect: the
    /// index came from [`small_imm_index`]).
    SmallImmNotInTable,
    /// A prepared operation with no selection arm (prepare's SELECTED and
    /// this file disagree: a defect).
    NotSelected,
    /// An operation that reads an operand selection did not supply (a
    /// defect: the value was never made).
    MissingOperand,
    /// An undefined scalar reached selection (prepare refuses one first).
    UndefinedValue,
    OutOfScratch,
}

impl From<ArenaError> for SelectError {
    fn from(_: ArenaError) -> Self {
        SelectError::OutOfScratch
    }
}

/// The selected program and the facts about it the shader record needs.
#[derive(Clone, Copy, Debug)]
pub struct Selected<'a> {
    pub ops: &'a [VOp],
    /// A fragment shader (else a vertex or coordinate shader).
    pub fragment: bool,
    /// Virtual registers handed out.
    pub vregs: u32,
    /// Vertex and coordinate shaders: VPM rows read per attribute record.
    pub vattr_sizes: [u8; 16],
    /// Vertex and coordinate shaders: VPM rows written, before rounding to
    /// sectors (`c->vpm_output_size`, v3d_nir_lower_io.c:433).
    pub vpm_output_rows: u32,
    /// Vertex and coordinate shaders: the stage reads gl_VertexID, so the
    /// shader record asks for it and it is VPM input row 0, ahead of the
    /// attributes (prog_data `uses_vid`, vir.c:880-883).
    pub uses_vid: bool,
    /// Vertex and coordinate shaders: the stage reads gl_InstanceID, so the
    /// shader record asks for it and it is VPM input row 0, ahead of the
    /// vertex ID (prog_data `uses_iid`).
    pub uses_iid: bool,
    /// Fragment shaders: some operation reads the payload W (rf0).
    pub uses_center_w: bool,
    /// Fragment shaders: the first two varyings are the implicit point
    /// coordinates (prog_data `uses_implicit_point_line_varyings`): the
    /// shader reads gl_PointCoord, and the program is for points draws.
    pub implicit_point_varyings: bool,
    /// Fragment shaders: the program writes Z to the TLB itself (the pass-
    /// through Z from FEP), prog_data `writes_z`.
    pub writes_z: bool,
    /// Fragment shaders: early Z/stencil tests are off (prog_data
    /// `disable_ez`, `!early_fragment_tests`).
    pub disable_ez: bool,
}

/// The TLB specifier of Mesa's pass-through Z write: TLB_TYPE_DEPTH (2 << 6)
/// | TLB_V42_DEPTH_TYPE_INVARIANT (0 << 3, "unmodified sideband input
/// used") | TLB_SAMPLE_MODE_PER_PIXEL (1 << 2), in 0xffffff00
/// (nir_to_vir.c:1934-1946, 2110-2129, Mesa 26.2.1).
pub const TLB_Z_FROM_FEP: u32 = 0xffff_ff00 | (2 << 6) | (1 << 2);

struct Sel<'a> {
    ops: AVec<'a, VOp>,
    next: u32,
    /// Per prepared scalar, the operand that holds it.
    of: &'a mut [Operand],
    /// Per look-up (its prepared index), the registers its result words
    /// were read into, and whether they are 32-bit words (one channel each)
    /// rather than f16 pairs.
    tex_words: AVec<'a, (u32, [Option<V>; 4], bool)>,
    /// The look-ups requested and not read yet, oldest first: (prepared
    /// index, the mask of result words asked for, as P0 carries it, and the
    /// 32-bit output). [`Sel::flush_tmu`] gives them one thread switch.
    pending: AVec<'a, (u32, u32, bool)>,
    /// The bindings whose look-ups take the 32-bit TMU output
    /// (`LodKey::ret32` of the compile, fragment stage only: the two stages
    /// number their bindings apart, and the key is the fragment's).
    ret32: u32,
    /// Look-ups share a switch ([`Pressure::Serial`] turns it off).
    pipelined: bool,
    /// The counted loops open at this point, innermost last.
    loop_counters: AVec<'a, LoopFrame>,
    /// At the end of a counted loop's pass, from its first
    /// [`SOp::PhiNext`] to its [`SOp::LoopEnd`]: per carried register that
    /// a copy reads after an earlier copy of the same pass end has written
    /// it, the register that kept its old value ([`Sel::phi_copies`]).
    phi_saved: AVec<'a, (V, V)>,
    /// Between a loop end's first [`SOp::PhiNext`] and its
    /// [`SOp::LoopEnd`]: [`Sel::phi_saved`] is this pass end's.
    in_phi_copies: bool,
    /// A fragment shader: its back edges ignore lanes whose multisample
    /// flags are clear (discarded), msfign P.
    fragment: bool,
    /// Vertex and coordinate shaders: per prepared scalar, the VPM row an
    /// attribute component is read from (unused for other operations);
    /// empty in a fragment shader.
    attr_rows: &'a [u32],
    /// Per prepared scalar, the uniform word a value that is only a load
    /// (a uniform, or a constant that is not a small immediate) reads; it is
    /// loaded where it is read, not where prepare defined it ([`Sel::val`]).
    deferred: &'a [Option<Uniform>],
    /// [`Pressure::Remat`]: a deferred load is made again at every read.
    remat: bool,
    /// The last flag push still in force: an identical push after it is
    /// not emitted (select.rs's selects push the same test once per lane;
    /// FemtoVG's `texType == 1` selects pushed `xor.pushz` three times,
    /// 2026-09-26). Forgotten at a thread switch (flags do not survive one),
    /// at a loop head or back edge, and when an op writes a pushed operand.
    last_push: Option<VOp>,
    /// Labels handed out ([`VOp::Label`]).
    labels: u32,
    /// The uniform branches open at this point, innermost last.
    ifs: AVec<'a, IfFrame>,
    /// Every uniform branch's merged values: (the prepared index of the
    /// [`SOp::IfMerge`], its register).
    merges: AVec<'a, (u32, V)>,
    /// The prepared scalars [`Sel::val`] loaded (and so cached) inside the
    /// open uniform branches and counted loops with a break: forgotten at
    /// the end of each arm, and after such a loop, where the load has not
    /// run on the other path.
    cached: AVec<'a, u32>,
}

/// One open counted loop (select.rs [`Sel::body`], SOp::LoopHead).
#[derive(Clone, Copy)]
struct LoopFrame {
    /// Its pass counter.
    counter: V,
    /// The label after its back edge that its breaks jump to
    /// (SOp::LoopBreak), when it has any.
    exit: Option<u32>,
    /// `cached[mark..]` were loaded inside it: forgotten after it when it
    /// has a break, as a pass that breaks may not have made them.
    mark: usize,
}

/// One open uniform branch (select.rs [`Sel::body`], SOp::IfBranch).
#[derive(Clone, Copy)]
struct IfFrame {
    else_label: u32,
    end_label: u32,
    /// Its merges: `merges[first..last]`.
    first: usize,
    last: usize,
    /// `cached[mark..]` were loaded inside it.
    mark: usize,
}

impl<'a> Sel<'a> {
    fn label(&mut self) -> u32 {
        self.labels += 1;
        self.labels - 1
    }
    /// Forget the loads cached since `mark` (the end of a branch arm).
    fn forget_cached(&mut self, mark: usize) {
        while self.cached.len() > mark {
            if let Some(i) = self.cached.pop() {
                self.of[i as usize] = Operand::None;
            }
        }
    }
    /// A loop's [`SOp::PhiNext`]s, from the one at `first` to its
    /// [`SOp::LoopEnd`], are one PARALLEL copy: every carried value takes
    /// what its source held at the end of the pass. Selected as one
    /// [`VOp::Copy`] each, in order, a copy whose source is a carried
    /// register an earlier copy of the same pass end has written reads the
    /// NEW value. `res = res.yzwx` in a loop whose body is not guarded
    /// (prepare.rs clear_at_pass_start and SOp::LoopBreak drop the guard's
    /// selects) carries x <- y, y <- z, z <- w, w <- x, a cycle, and w took
    /// the new x (dEQP-GLES2 shaders.loops.*_uniform_iterations, 72 cases
    /// on the Raspberry Pi 4, sweep 29). Each such source is first moved
    /// into a register of its own ([`Sel::phi_saved`]), before any copy of
    /// this pass end, as an SSA back end's parallel-copy sequentialisation
    /// does (Mesa's nir_convert_from_ssa emits its phis' copies as
    /// nir_parallel_copy for this reason).
    fn phi_copies(&mut self, p: &Prepared<'_>, first: usize, a: &Arena<'a>) -> Result<(), SelectError> {
        self.in_phi_copies = true;
        while self.phi_saved.pop().is_some() {}
        let mut written: AVec<'a, V> = AVec::new();
        for op in &p.ops[first..] {
            match *op {
                SOp::PhiNext { ops } => {
                    if let Operand::V(src) = self.of[ops[1].0 as usize] {
                        let saved = self.phi_saved.as_slice().iter().any(|&(old, _)| old == src);
                        if !saved && written.as_slice().contains(&src) {
                            let kept = self.v();
                            let mov =
                                VOp::Mul { op: MulOp::Mov, dst: Dst::V(kept), a: Operand::V(src), b: Operand::None };
                            self.push(a, mov)?;
                            self.phi_saved.push(a, (src, kept))?;
                        }
                    }
                    if let Operand::V(dst) = self.of[ops[0].0 as usize] {
                        written.push(a, dst)?;
                    }
                }
                SOp::LoopEnd { .. } => break,
                _ => {}
            }
        }
        written.release(a);
        Ok(())
    }
    fn v(&mut self) -> V {
        self.next += 1;
        V(self.next - 1)
    }
    fn push(&mut self, a: &Arena<'a>, op: VOp) -> Result<(), SelectError> {
        match op {
            VOp::Flags { .. } if self.last_push == Some(op) => return Ok(()),
            VOp::Flags { .. } => self.last_push = Some(op),
            VOp::Thrsw | VOp::LoopHead | VOp::BackEdge { .. } | VOp::FwdBranch { .. } | VOp::Label(_) => {
                self.last_push = None
            }
            _ => {
                /* a write to a pushed operand: the flags no longer say what
                 * a new push of the same operands would */
                let wrote = match op {
                    VOp::CondMov { dst, .. } | VOp::Copy { dst, .. } => Some(dst),
                    _ => op.def(),
                };
                if let (Some(d), Some(VOp::Flags { a: x, b: y, .. })) = (wrote, self.last_push) {
                    if x.vreg() == Some(d) || y.vreg() == Some(d) {
                        self.last_push = None;
                    }
                }
            }
        }
        Ok(self.ops.push(a, op)?)
    }
    /// The operand that holds prepared scalar `x` at this point.
    ///
    /// A deferred load (a uniform, or a constant that is not a small
    /// immediate) is made at its first read and that register is read after
    /// it: Mesa moves every load_uniform and constant to just before its
    /// first use (nir_opt_move with nir_move_load_uniform |
    /// nir_move_const_undef, vir.c:2125-2127, Mesa 26.2.1, MIT), and prepare
    /// defines a uniform where the IR first loads it, which can be long
    /// before (dEQP light_amount: every light's uniforms were loaded before
    /// the first light's use). The first read in program order is the
    /// earliest point any read reaches: the program is straight-line, and a
    /// load first made inside a counted loop's body is made on every pass
    /// (the uniform stream rewinds with the branch).
    ///
    /// Under [`Pressure::Remat`] the load is made again at every read, so it
    /// is live for one operation (Mesa rematerialises a uniform it would
    /// spill the same way: a temp an ldunif defines is SPILL_TYPE_UNIFORM,
    /// and v3d_spill_reg loads it again before each use;
    /// vir_register_allocate.c:255-260, 717-724).
    fn val(&mut self, a: &Arena<'a>, x: S) -> Result<Operand, SelectError> {
        let i = x.0 as usize;
        let Some(&Some(u)) = self.deferred.get(i) else { return Ok(self.of[i]) };
        if !self.remat && self.of[i] != Operand::None {
            return Ok(self.of[i]);
        }
        let v = Operand::V(self.uniform(a, u)?);
        if !self.remat {
            self.of[i] = v;
            if !self.ifs.is_empty() || self.loop_counters.as_slice().iter().any(|f| f.exit.is_some()) {
                self.cached.push(a, i as u32)?;
            }
        }
        Ok(v)
    }
    fn uniform(&mut self, a: &Arena<'a>, u: Uniform) -> Result<V, SelectError> {
        let dst = self.v();
        self.push(a, VOp::Ldunif { dst, u })?;
        Ok(dst)
    }
    /// Two operands one instruction can read: at most one small immediate
    /// (it sits in raddr_b), so a second, different one is loaded through a
    /// CONSTANT uniform instead.
    fn pair(&mut self, a: &Arena<'a>, x: Operand, y: Operand) -> Result<(Operand, Operand), SelectError> {
        match (x, y) {
            (Operand::SmallImm(i), Operand::SmallImm(j)) if i != j => {
                let bits = small_imm_value(j).ok_or(SelectError::SmallImmNotInTable)?;
                Ok((x, Operand::V(self.uniform(a, Uniform::constant(bits))?)))
            }
            _ => Ok((x, y)),
        }
    }
    /// Push the flag that makes `op(x, y)` the condition returned
    /// (ntq_emit_comparison, nir_to_vir.c:1184-1231): IR `Fle(x, y)` is
    /// NIR `fge(y, x)`, FCMP(x, y) pushing C.
    fn compare(&mut self, a: &Arena<'a>, op: AluOp, x: Operand, y: Operand) -> Result<Cond, SelectError> {
        let (x, y) = self.pair(a, x, y)?;
        /* ntq_emit_comparison (nir_to_vir.c:1189-1201 for ieq/ine): the
         * float ones on FCMP, the integer equalities on XOR.pushz, the
         * signed orderings on MIN.pushc, whose C is "a > b": NIR ilt(x, y)
         * is MIN(y, x) under IFA, and IR Ile(x, y) is NIR ige(y, x), MIN(x,
         * y) under IFNA. */
        if let AluOp::Ilt | AluOp::Ile = op {
            let (p, q, cond) = if op == AluOp::Ilt { (y, x, Cond::IfA) } else { (x, y, Cond::IfNA) };
            self.push(a, VOp::Flags { unit: FlagOp::Add(AddOp::Min), a: p, b: q, pf: Pf::PushC, uf: Uf::None })?;
            return Ok(cond);
        }
        let (unit, pf, cond) = match op {
            AluOp::Flt => (AddOp::Fcmp, Pf::PushN, Cond::IfA),
            AluOp::Fle => (AddOp::Fcmp, Pf::PushC, Cond::IfA),
            AluOp::Feq => (AddOp::Fcmp, Pf::PushZ, Cond::IfA),
            AluOp::Fne => (AddOp::Fcmp, Pf::PushZ, Cond::IfNA),
            AluOp::Ieq => (AddOp::Xor, Pf::PushZ, Cond::IfA),
            /* NIR ult32: SUB.pushc, whose C is the unsigned borrow (x < y). */
            AluOp::Ult => (AddOp::Sub, Pf::PushC, Cond::IfA),
            AluOp::Ine => (AddOp::Xor, Pf::PushZ, Cond::IfNA),
            _ => return Err(SelectError::NotSelected),
        };
        self.push(a, VOp::Flags { unit: FlagOp::Add(unit), a: x, b: y, pf, uf: Uf::None })?;
        Ok(cond)
    }
    /// The condition "`x` is not zero" (any bit set), from MOV.pushz: the
    /// IR's truth test (eval.rs: a bool is true when any bit is set).
    fn nonzero(&mut self, a: &Arena<'a>, x: Operand) -> Result<Cond, SelectError> {
        self.push(
            a,
            VOp::Flags { unit: FlagOp::Mul(MulOp::Mov), a: x, b: Operand::None, pf: Pf::PushZ, uf: Uf::None },
        )?;
        Ok(Cond::IfNA)
    }
    /// A 32-bit constant: a small immediate where the stage uses them and
    /// the table has it, else a CONSTANT uniform.
    fn constant(&mut self, a: &Arena<'a>, bits: u32, small_imms: bool) -> Result<Operand, SelectError> {
        match small_imm_index(bits) {
            Some(i) if small_imms => Ok(Operand::SmallImm(i)),
            _ => Ok(Operand::V(self.uniform(a, Uniform::constant(bits))?)),
        }
    }
    /// ntq_fsincos (nir_to_vir.c:991-1001): x / pi (plus 0.5 for cos),
    /// rounded to whole periods, SIN of what is left, and the sign flipped
    /// for an odd period (the period's low bit shifted to bit 31 by SHL by
    /// -1, which the shifter takes modulo 32).
    fn sincos(&mut self, a: &Arena<'a>, x: Operand, is_cos: bool, small_imms: bool) -> Result<Operand, SelectError> {
        let inv_pi = self.constant(a, (1.0f32 / core::f32::consts::PI).to_bits(), small_imms)?;
        let mut input = self.mul(a, MulOp::Fmul, x, inv_pi)?;
        if is_cos {
            let half = self.constant(a, 0.5f32.to_bits(), small_imms)?;
            input = self.add(a, AddOp::Fadd, input, half)?;
        }
        let periods = self.add(a, AddOp::Fround, input, Operand::None)?;
        let frac = self.add(a, AddOp::Fsub, input, periods)?;
        let sin = self.add(a, AddOp::Sin, frac, Operand::None)?;
        let ip = self.add(a, AddOp::Ftoin, periods, Operand::None)?;
        let minus_one = self.constant(a, u32::MAX, small_imms)?;
        let sign = self.add(a, AddOp::Shl, ip, minus_one)?;
        self.add(a, AddOp::Xor, sin, sign)
    }
    /// ~0 and 0, a GLSL bool's two values (eval.rs `bits_of_bool`; Mesa's
    /// vir_uniform_ui(~0) and (0), small immediates in a fragment shader).
    fn bool_constants(&mut self, a: &Arena<'a>, small_imms: bool) -> Result<(Operand, Operand), SelectError> {
        Ok((self.constant(a, !0, small_imms)?, self.constant(a, 0, small_imms)?))
    }
    /// vir_SEL: `t = f`, then `t = t_val` where `cond` holds.
    fn select(&mut self, a: &Arena<'a>, cond: Cond, t_val: Operand, f: Operand) -> Result<Operand, SelectError> {
        let Operand::V(t) = self.mul(a, MulOp::Mov, f, Operand::None)? else { return Err(SelectError::NotSelected) };
        self.push(a, VOp::CondMov { dst: t, src: t_val, cond })?;
        Ok(Operand::V(t))
    }
    /// `o` in a virtual register (an input unpack applies to a register
    /// read only): a small immediate or payload read is moved first.
    fn in_register(&mut self, a: &Arena<'a>, o: Operand) -> Result<V, SelectError> {
        match o {
            Operand::V(v) => Ok(v),
            _ => match self.mul(a, MulOp::Fmov, o, Operand::None)? {
                Operand::V(v) => Ok(v),
                _ => Err(SelectError::NotSelected),
            },
        }
    }
    /// The last thread switch: two THRSW in a row (the scheduler keeps the
    /// second in the next word), before the TLB or VPM writes. Mesa's
    /// vir_emit_last_thrsw emits one for every stage that has switched
    /// (nir_to_vir.c:5003-5028), and a fragment shader always has one
    /// (:5019-5021); the scheduler makes it two in a row
    /// (qpu_schedule.c:2359-2370).
    ///
    /// When the program has no spills, Mesa removes that extra THRSW again
    /// (:5203-5207, vir_restore_last_thrsw :5036-5047): the last look-up's
    /// own switch becomes the last one, unless it is inside a loop
    /// (last_thrsw_at_top_level, :5007; `!c->in_control_flow`, which an `if`
    /// sets too). We never spill, so we do the same: the pair goes where the
    /// last switch stands when it is at the top level, and a new pair is
    /// added when that switch is inside a counted loop or a uniform branch's
    /// arm (between a forward branch and its label: an arm may be skipped),
    /// or, in a fragment shader (`always`), when there is none. For femtovg's
    /// fill_image that is 3 words fewer (the pair and a NOP before the
    /// thrend; Mesa's listing has the look-up's pair, 2026-09-26).
    fn last_thrsw(&mut self, a: &Arena<'a>, always: bool) -> Result<(), SelectError> {
        let ops = self.ops.as_slice();
        match ops.iter().rposition(|o| *o == VOp::Thrsw) {
            Some(last) => {
                let depth = ops[..last].iter().fold(0i32, |d, o| match o {
                    VOp::LoopHead => d + 1,
                    VOp::BackEdge { .. } => d - 1,
                    _ => d,
                });
                let in_arm = ops[..last].iter().any(|o| match *o {
                    VOp::FwdBranch { label, .. } => !ops[..last].contains(&VOp::Label(label)),
                    _ => false,
                });
                if depth == 0 && !in_arm {
                    self.push(a, VOp::Thrsw)?;
                    self.ops.as_mut_slice()[last + 1..].rotate_right(1);
                } else {
                    self.push(a, VOp::Thrsw)?;
                    self.push(a, VOp::Thrsw)?;
                }
            }
            None if always => {
                self.push(a, VOp::Thrsw)?;
                self.push(a, VOp::Thrsw)?;
            }
            None => {}
        }
        Ok(())
    }
    /// The flag a uniform branch or a loop's break branches on (prepare.rs
    /// SOp::IfBranch, SOp::LoopBreak): a comparison pushes its own test, any
    /// other bool its test against zero.
    fn branch_flag(&mut self, a: &Arena<'a>, p: &Prepared<'_>, cond: S) -> Result<Cond, SelectError> {
        match p.ops[cond.0 as usize] {
            SOp::Alu(c, cs) if is_comparison(c) => {
                let (cx, cy) = (self.val(a, cs[0])?, self.val(a, cs[1])?);
                self.compare(a, c, cx, cy)
            }
            _ => {
                let x = self.val(a, cond)?;
                self.nonzero(a, x)
            }
        }
    }
    fn mul(&mut self, a: &Arena<'a>, op: MulOp, x: Operand, y: Operand) -> Result<Operand, SelectError> {
        let (x, y) = self.pair(a, x, y)?;
        let d = self.v();
        self.push(a, VOp::Mul { op, dst: Dst::V(d), a: x, b: y })?;
        Ok(Operand::V(d))
    }
    fn add(&mut self, a: &Arena<'a>, op: AddOp, x: Operand, y: Operand) -> Result<Operand, SelectError> {
        let (x, y) = self.pair(a, x, y)?;
        let d = self.v();
        self.push(a, VOp::Add { op, dst: Dst::V(d), a: x, b: y })?;
        Ok(Operand::V(d))
    }
    /// The arithmetic and constants of `p`, in its order. The loads are
    /// selected already (their operands are in `of`).
    fn body(&mut self, p: &Prepared<'_>, small_imms: bool, a: &Arena<'a>) -> Result<(), SelectError> {
        /* A comparison whose every reader is a Bcsel condition is never
         * made a value: each Bcsel pushes its flag again (Mesa's
         * ntq_emit_bool_to_cond reuses the comparison the same way). */
        let as_value = a.alloc_filled(p.ops.len(), false)?;
        for r in p.color.iter().flatten().chain(&p.position).chain(p.varyings.iter().flatten()).chain(&p.psiz) {
            as_value[r.0 as usize] = true;
        }
        for op in p.ops {
            if matches!(op, SOp::Phi { .. } | SOp::PhiNext { .. } | SOp::LoopEnd { .. } | SOp::IfMerge { .. }) {
                for x in op.operands() {
                    as_value[x.0 as usize] = true;
                }
            }
        }
        for op in p.ops {
            if let SOp::Alu(alu, s) = *op {
                for (k, x) in s.iter().enumerate().take(alu.arity()) {
                    if !(alu == AluOp::Bcsel && k == 0) {
                        as_value[x.0 as usize] = true;
                    }
                }
            }
            if let SOp::Tex { srcs, cube, layer, bias, lod, .. } = *op {
                for x in srcs.iter().take(super::prepare::tex_operands(cube || layer, bias || lod)) {
                    as_value[x.0 as usize] = true;
                }
            }
        }
        for &k in issue_order(p, a)? {
            let i = k as usize;
            let op = &p.ops[i];
            /* Pending look-ups are read in the stretch that made them: a
             * branch or a loop's edge flushes them (ntq_emit_block and the
             * jump case, nir_to_vir.c:4595-4626: never a switch and ldtmu
             * for a look-up of another block, possibly under control flow). */
            if ends_stretch(op) {
                self.flush_tmu(a)?;
            }
            /* A comparison only a select reads is fused into that select. */
            if matches!(op, SOp::Alu(c, _) if is_comparison(*c)) && !as_value[i] {
                continue;
            }
            let o = match *op {
                /* v3d_tex.c v3d_vir_emit_tex, for a regular 2D look-up with
                 * an f16 result: P0 (the result words to return), P1, the
                 * T (and bias or level) writes, then S, which retires the
                 * look-up (TMUSLOD for an explicit level: `txl`, v3d_tex.c:
                 * 374-375, which a vertex shader's every look-up is); then
                 * one ldtmu per word asked for. */
                SOp::Tex { sampler, srcs, cube, layer, bias, lod } => {
                    /* A binding the draw keyed for a 32-bit output
                     * (`LodKey::ret32`: an R32F texture, Mesa's return size
                     * 32): one word per channel, lane l is word l, instead
                     * of two f16 channels a word (v3d_tex.c's
                     * return_channels / output type for a 32-bit return). */
                    let ret32 = sampler < 16 && self.ret32 & (1 << sampler) != 0;
                    let mut words = 0u32;
                    for q in p.ops {
                        if let SOp::TexLane { tex, lane } = *q {
                            if tex[0].0 as usize == i {
                                words |= if ret32 { 1 << lane } else { 1 << (lane / 2) };
                            }
                        }
                    }
                    if words == 0 {
                        continue;
                    }
                    /* Its result words must fit the output FIFO beside the
                     * look-ups still queued, or those are flushed first
                     * (ntq_tmu_fifo_overflow, nir_to_vir.c:251-258, before
                     * the look-up: v3d_tex.c:277-278). */
                    let queued: u32 = self.pending.as_slice().iter().map(|e| e.1.count_ones()).sum();
                    if queued + words.count_ones() > TMU_OUTPUT_WORDS || self.pending.len() >= MAX_TMU_QUEUE {
                        self.flush_tmu(a)?;
                    }
                    let unit = u32::from(sampler) << 24;
                    self.push(a, VOp::Wrtmuc { u: Uniform { contents: QUNIFORM_TMU_CONFIG_P0, data: unit | words } })?;
                    /* P1: per-pixel mask enable (bit 2), as Mesa's fragment
                     * look-ups carry it (v3d_tex.c:320-325; its output for
                     * es100/textured has 41:0x00000004), and every look-up
                     * outside the fragment stage (`stage != FRAGMENT`,
                     * v3d_tex.c:317). */
                    /* bit 0, "Output Type 32-bit" (v3d_packet.xml TMU Config
                     * Parameter 1), for a ret32 binding */
                    let p1 = unit | 4 | u32::from(ret32);
                    self.push(a, VOp::Wrtmuc { u: Uniform { contents: QUNIFORM_TMU_CONFIG_P1, data: p1 } })?;
                    /* A cube at an explicit level: P2 with disable_autolod,
                     * after P1 (TMUSLOD, which implies it, is not a cube
                     * write). */
                    if cube && lod {
                        self.push(a, VOp::Wrtmuc { u: Uniform::constant(TMU_P2_CUBE_LOD) })?;
                    }
                    let (s, t) = (self.val(a, srcs[0])?, self.val(a, srcs[1])?);
                    self.push(a, VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(WADDR_TMUT), a: t, b: Operand::None })?;
                    if cube {
                        let r = self.val(a, srcs[2])?;
                        self.push(a, VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(WADDR_TMUR), a: r, b: Operand::None })?;
                    }
                    /* a 2D-array look-up's layer (Mesa's `is_array` write,
                     * v3d_tex.c): the face of a cube record */
                    if layer {
                        let i = self.val(a, srcs[2])?;
                        self.push(a, VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(WADDR_TMUI), a: i, b: Operand::None })?;
                    }
                    if bias || lod {
                        let b = self.val(a, srcs[2 + usize::from(cube || layer)])?;
                        self.push(a, VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(WADDR_TMUB), a: b, b: Operand::None })?;
                    }
                    /* the retiring S write (v3d_tex.c:371-378): TMUSCM for a
                     * cube, TMUSLOD for a 2D explicit level, else TMUS */
                    let retire = if cube {
                        WADDR_TMUSCM
                    } else if lod {
                        WADDR_TMUSLOD
                    } else {
                        WADDR_TMUS
                    };
                    self.push(a, VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(retire), a: s, b: Operand::None })?;
                    /* Its thread switch and ldtmus wait in the queue until a
                     * result is read (ntq_add_pending_tmu_flush, nir_to_vir.c:
                     * 301-328), or come right away when pipelining is off. */
                    self.pending.push(a, (i as u32, words, ret32))?;
                    if !self.pipelined {
                        self.flush_tmu(a)?;
                    }
                    Operand::None
                }
                /* Lane l of a look-up: word l / 2, its low (even l) or high
                 * f16 half, as an f32. A look-up still queued is flushed
                 * first (ntq_get_src, nir_to_vir.c:856-868: a def not in
                 * the table yet flushes the TMU). The FMOV with the L or H
                 * unpack is Mesa's unpack_half_2x16_split_x/y;
                 * [`fold_unpacks`] later lets each reader that can read the
                 * word as `v.l` / `v.h` do so, and drops the FMOV when no
                 * reader is left. */
                SOp::TexLane { tex, lane } => {
                    if self.pending.as_slice().iter().any(|e| e.0 == tex[0].0) {
                        self.flush_tmu(a)?;
                    }
                    let (regs, ret32) = self
                        .tex_words
                        .as_slice()
                        .iter()
                        .find(|e| e.0 == tex[0].0)
                        .map(|e| (e.1, e.2))
                        .ok_or(SelectError::NotSelected)?;
                    if ret32 {
                        /* a 32-bit word is the channel itself */
                        Operand::V(regs.get(usize::from(lane)).copied().flatten().ok_or(SelectError::NotSelected)?)
                    } else {
                        let v = regs[usize::from(lane / 2)].ok_or(SelectError::NotSelected)?;
                        let half = if lane % 2 == 0 { Operand::Lo(v) } else { Operand::Hi(v) };
                        self.mul(a, MulOp::Fmov, half, Operand::None)?
                    }
                }
                /* Loaded where it is read (Sel::val). */
                SOp::Uniform(_) | SOp::TexSizeWord { .. } => continue,
                /* A uniform branch (prepare.rs SOp::IfBranch; Mesa's
                 * ntq_emit_uniform_if, nir_to_vir.c:4293-4390): the
                 * condition's flag, then a jump to the else arm where it
                 * does not hold in some lane (ANYNA for IFA, ANYA for IFNA),
                 * with msfign P: lanes not dispatched or discarded do not
                 * vote. The condition is uniform, so every lane agrees. */
                SOp::IfBranch { cond } => {
                    let cnd = self.branch_flag(a, p, cond[0])?;
                    let to_else = match cnd {
                        Cond::IfA => BranchCond::AnyNA,
                        Cond::IfNA => BranchCond::AnyA,
                        _ => return Err(SelectError::NotSelected),
                    };
                    /* its merges: the IfMerges right after its IfEnd, each
                     * one register the two arms write */
                    let mut depth = 0usize;
                    let mut end = None;
                    for (j, o) in p.ops.iter().enumerate().skip(i + 1) {
                        match o {
                            SOp::IfBranch { .. } => depth += 1,
                            SOp::IfEnd if depth == 0 => {
                                end = Some(j);
                                break;
                            }
                            SOp::IfEnd => depth -= 1,
                            _ => {}
                        }
                    }
                    let end = end.ok_or(SelectError::NotSelected)?;
                    let first = self.merges.len();
                    for (j, o) in p.ops.iter().enumerate().skip(end + 1) {
                        if !matches!(o, SOp::IfMerge { .. }) {
                            break;
                        }
                        let v = self.v();
                        self.merges.push(a, (j as u32, v))?;
                    }
                    let (else_label, end_label) = (self.label(), self.label());
                    let frame =
                        IfFrame { else_label, end_label, first, last: self.merges.len(), mark: self.cached.len() };
                    self.ifs.push(a, frame)?;
                    self.push(a, VOp::FwdBranch { cond: to_else, msfign: Msfign::P, label: else_label })?;
                    continue;
                }
                /* The end of the then arm: each merge's register takes the
                 * then value (its definition), then the jump over the else
                 * arm, whose first word is the else label. */
                SOp::IfElse => {
                    let f = *self.ifs.as_slice().last().ok_or(SelectError::NotSelected)?;
                    for k in f.first..f.last {
                        let (j, mv) = self.merges.as_slice()[k];
                        let SOp::IfMerge { ops } = p.ops[j as usize] else { return Err(SelectError::NotSelected) };
                        let t = self.val(a, ops[0])?;
                        self.push(a, VOp::Mul { op: MulOp::Mov, dst: Dst::V(mv), a: t, b: Operand::None })?;
                    }
                    self.forget_cached(f.mark);
                    self.push(
                        a,
                        VOp::FwdBranch { cond: BranchCond::Always, msfign: Msfign::None, label: f.end_label },
                    )?;
                    self.push(a, VOp::Label(f.else_label))?;
                    continue;
                }
                /* The end of the else arm: each merge's register takes the
                 * else value (a rewrite, as a loop's carried value is). */
                SOp::IfEnd => {
                    let f = *self.ifs.as_slice().last().ok_or(SelectError::NotSelected)?;
                    for k in f.first..f.last {
                        let (j, mv) = self.merges.as_slice()[k];
                        let SOp::IfMerge { ops } = p.ops[j as usize] else { return Err(SelectError::NotSelected) };
                        let e = self.val(a, ops[1])?;
                        self.push(a, VOp::Copy { dst: mv, src: e })?;
                    }
                    self.forget_cached(f.mark);
                    self.ifs.pop();
                    self.push(a, VOp::Label(f.end_label))?;
                    continue;
                }
                SOp::IfMerge { .. } => {
                    let found = self.merges.as_slice().iter().find(|m| m.0 == i as u32);
                    Operand::V(found.ok_or(SelectError::NotSelected)?.1)
                }
                /* A counted loop (prepare.rs counted_loop; docs/online-
                 * shaderc/v3d-backend.md, "Control flow, step 2"). */
                SOp::Phi { init } => {
                    let x = self.val(a, init[0])?;
                    self.mul(a, MulOp::Mov, x, Operand::None)?
                }
                SOp::LoopHead => {
                    /* this loop's own end: loops nest, so the first LoopEnd
                     * at depth 0 after the head */
                    let mut depth = 0u32;
                    let mut count = None;
                    let mut breaks = false;
                    let mut end = i;
                    for (j, o) in p.ops.iter().enumerate().skip(i + 1) {
                        match *o {
                            SOp::LoopHead => depth += 1,
                            SOp::LoopEnd { count: c, .. } if depth == 0 => {
                                count = Some(c);
                                end = j;
                                break;
                            }
                            SOp::LoopEnd { .. } => depth -= 1,
                            SOp::LoopBreak { .. } if depth == 0 => breaks = true,
                            _ => {}
                        }
                    }
                    let count = count.ok_or(SelectError::NotSelected)?;
                    if count == 0 {
                        return Err(SelectError::NotSelected);
                    }
                    /* The deferred loads (a uniform, a texture-size word,
                     * a constant that is not a small immediate) the loop
                     * reads and that are defined before its head are made
                     * before the head, not at their first read inside it,
                     * where they would be loaded again on every pass (the
                     * stream rewinds with the back edge). Mesa's are: its
                     * nir_opt_move moves a load "just before its first use
                     * inside the same basic block" (nir_opt_move.c:30-31,
                     * Mesa 26.2.1), so one defined before the loop stays
                     * there, and one defined in the body stays in the body.
                     * Hoisting those too (every load the loop reads) held a
                     * dynamically indexed array's elements across the loop:
                     * es300 dynidx 109 -> 120 words (step (m)). Not under
                     * Pressure::Remat, which loads again at every read to
                     * keep live ranges short. filter_image's pass lost its
                     * signal-only reload word (step (m)). */
                    if !self.remat {
                        for o in &p.ops[i + 1..end] {
                            for &x in o.operands() {
                                if (x.0 as usize) < i && self.deferred.get(x.0 as usize).is_some_and(|d| d.is_some()) {
                                    self.val(a, x)?;
                                }
                            }
                        }
                    }
                    /* Loaded before the head, so every entry into the loop
                     * (each pass of an outer loop) starts a full count. */
                    let c = self.uniform(a, Uniform::constant(count))?;
                    let exit = if breaks { Some(self.label()) } else { None };
                    self.loop_counters.push(a, LoopFrame { counter: c, exit, mark: self.cached.len() })?;
                    self.push(a, VOp::LoopHead)?;
                    continue;
                }
                /* A uniform break (prepare.rs SOp::LoopBreak; Mesa's
                 * structured break, ntq_emit_jump): the condition's flag,
                 * then a jump to the loop's exit label where it is `when`,
                 * with msfign P in a fragment shader as a uniform branch's.
                 * The condition is uniform, so every lane agrees. */
                SOp::LoopBreak { cond, when } => {
                    let exit = self.loop_counters.as_slice().last().and_then(|f| f.exit);
                    let exit = exit.ok_or(SelectError::NotSelected)?;
                    let cnd = self.branch_flag(a, p, cond[0])?;
                    let taken = match (cnd, when) {
                        (Cond::IfA, true) | (Cond::IfNA, false) => BranchCond::AnyA,
                        (Cond::IfNA, true) | (Cond::IfA, false) => BranchCond::AnyNA,
                        _ => return Err(SelectError::NotSelected),
                    };
                    let msfign = if self.fragment { Msfign::P } else { Msfign::None };
                    self.push(a, VOp::FwdBranch { cond: taken, msfign, label: exit })?;
                    continue;
                }
                SOp::PhiNext { ops } => {
                    if !self.in_phi_copies {
                        self.phi_copies(p, i, a)?;
                    }
                    let Operand::V(dst) = self.of[ops[0].0 as usize] else { return Err(SelectError::NotSelected) };
                    let src = match self.val(a, ops[1])? {
                        Operand::V(v) => match self.phi_saved.as_slice().iter().find(|&&(old, _)| old == v) {
                            Some(&(_, kept)) => Operand::V(kept),
                            None => Operand::V(v),
                        },
                        src => src,
                    };
                    self.push(a, VOp::Copy { dst, src })?;
                    continue;
                }
                SOp::LoopEnd { exit, has_exit, .. } => {
                    self.in_phi_copies = false;
                    let f = self.loop_counters.pop().ok_or(SelectError::NotSelected)?;
                    let c = f.counter;
                    let one = self.constant(a, 1, small_imms)?;
                    let t = self.add(a, AddOp::Sub, Operand::V(c), one)?;
                    self.push(a, VOp::Copy { dst: c, src: t })?;
                    /* MOV.pushz on the new count: A where it reached 0. It
                     * reads the counter after the Copy, not t: the Copy is
                     * then t's last read, so the allocator writes t straight
                     * into the counter's register (alloc.rs, "A Copy m <- e
                     * that is e's last read"): one `sub c, c, 1`, the Copy
                     * dropped as `mov c, c`. The sub and the push then read
                     * and write the counter alone, so they need not wait for
                     * the end of the pass: order.rs issues them where a unit
                     * is free, and the words after the push can be the back
                     * edge's delay slots (sched.rs branch_back). Reading t
                     * kept t live past the Copy: `sub r0, c, 1; mov c, r0;
                     * mov.pushz r0; branch; 3 NOPs` at the end of every pass
                     * (filter_image, step (g)). */
                    self.push(
                        a,
                        VOp::Flags {
                            unit: FlagOp::Mul(MulOp::Mov),
                            a: Operand::V(c),
                            b: Operand::None,
                            pf: Pf::PushZ,
                            uf: Uf::None,
                        },
                    )?;
                    let cond = if has_exit {
                        /* stop = the break flag | (count == 0); go on while
                         * some lane has stop == 0. */
                        let (tr, fl) = self.bool_constants(a, small_imms)?;
                        let zero = self.select(a, Cond::IfA, tr, fl)?;
                        let e = self.val(a, exit[0])?;
                        let stop = self.add(a, AddOp::Or, e, zero)?;
                        self.push(
                            a,
                            VOp::Flags {
                                unit: FlagOp::Mul(MulOp::Mov),
                                a: stop,
                                b: Operand::None,
                                pf: Pf::PushZ,
                                uf: Uf::None,
                            },
                        )?;
                        BranchCond::AnyA
                    } else {
                        BranchCond::AnyNA
                    };
                    let msfign = if self.fragment { Msfign::P } else { Msfign::None };
                    self.push(a, VOp::BackEdge { cond, msfign })?;
                    if let Some(l) = f.exit {
                        self.forget_cached(f.mark);
                        self.push(a, VOp::Label(l))?;
                    }
                    continue;
                }
                /* gl_FragCoord (emit_fragcoord_input, nir_to_vir.c:1017-1024):
                 * FXCD and FYCD are the pixel centre (their fraction is the
                 * sample offset, :3161), z is the payload's Z, w the
                 * reciprocal of the payload's W. y then goes through the
                 * window transform, as nir_lower_wpos_ytransform's
                 * y * T[0] + T[1] does on Mesa's path. */
                SOp::FragCoord { lane: 0 } => self.add(a, AddOp::Fxcd, Operand::None, Operand::None)?,
                SOp::FragCoord { lane: 1 } => {
                    let y = self.add(a, AddOp::Fycd, Operand::None, Operand::None)?;
                    let scale = self.uniform(a, Uniform { contents: QUNIFORM_FB_WPOS_Y_SCALE, data: 0 })?;
                    let offset = self.uniform(a, Uniform { contents: QUNIFORM_FB_WPOS_Y_OFFSET, data: 0 })?;
                    let scaled = self.mul(a, MulOp::Fmul, y, Operand::V(scale))?;
                    self.add(a, AddOp::Fadd, scaled, Operand::V(offset))?
                }
                SOp::FragCoord { lane: 2 } => Operand::Payload(2),
                SOp::FragCoord { lane: 3 } => self.add(a, AddOp::Recip, Operand::Payload(0), Operand::None)?,
                SOp::FragCoord { .. } => return Err(SelectError::NotSelected),
                /* ntq_emit_intrinsic load_front_face (nir_to_vir.c:3692-
                 * 3699): REVF reads 0 for a front-facing primitive and 1 for
                 * a back-facing one; -1 + REVF is the bool, ~0 for front. */
                /* read before the body (select_fragment) */
                SOp::PointCoord { .. } => continue,
                SOp::FrontFacing => {
                    let revf = self.add(a, AddOp::Revf, Operand::None, Operand::None)?;
                    let minus_one = self.constant(a, u32::MAX, small_imms)?;
                    self.add(a, AddOp::Add, minus_one, revf)?
                }
                SOp::Const(bits) => match small_imm_index(bits) {
                    Some(idx) if small_imms => Operand::SmallImm(idx),
                    /* Loaded at each read instead (Sel::val). */
                    /* Loaded where it is read (Sel::val). */
                    _ => continue,
                },
                SOp::Fsub(s) => {
                    let (x, y) = (self.val(a, s[0])?, self.val(a, s[1])?);
                    self.add(a, AddOp::Fsub, x, y)?
                }
                SOp::Alu(alu, s) => {
                    /* The operands the operation has, each read once: a
                     * read under Remat is a load. A Bcsel's condition that
                     * is a comparison is fused below and never read. */
                    let mut v = [Operand::None; 3];
                    for (k, o) in v.iter_mut().enumerate().take(alu.arity()) {
                        let fused = k == 0
                            && alu == AluOp::Bcsel
                            && matches!(p.ops[s[0].0 as usize], SOp::Alu(c, _) if is_comparison(c));
                        if !fused {
                            *o = self.val(a, s[k])?;
                        }
                    }
                    let [x, y, z] = v;
                    match alu {
                        AluOp::Fadd => self.add(a, AddOp::Fadd, x, y)?,
                        AluOp::Fmul => self.mul(a, MulOp::Fmul, x, y)?,
                        /* nir_to_vir.c:1435-1450 */
                        AluOp::Fmin => self.add(a, AddOp::Fmin, x, y)?,
                        AluOp::Fmax => self.add(a, AddOp::Fmax, x, y)?,
                        /* fneg: XOR with the sign bit, a CONSTANT uniform
                         * (nir_to_vir.c:1398-1406) */
                        AluOp::Fneg => {
                            let sign = self.uniform(a, Uniform::constant(1 << 31))?;
                            self.add(a, AddOp::Xor, x, Operand::V(sign))?
                        }
                        c if is_comparison(c) => {
                            /* A comparison as a value: ~0 where it holds, 0
                             * elsewhere (ntq_emit_cond_to_bool, :1285-1293). */
                            let cond = self.compare(a, alu, x, y)?;
                            let (t, f) = self.bool_constants(a, small_imms)?;
                            self.select(a, cond, t, f)?
                        }
                        AluOp::Bcsel => {
                            /* b32csel: the condition straight from the
                             * comparison that made it when there is one, else
                             * the value tested against zero (:1728-1733,
                             * ntq_emit_bool_to_cond :1262-1282). */
                            let cond = match p.ops[s[0].0 as usize] {
                                SOp::Alu(c, cs) if is_comparison(c) => {
                                    let (cx, cy) = (self.val(a, cs[0])?, self.val(a, cs[1])?);
                                    self.compare(a, c, cx, cy)?
                                }
                                _ => {
                                    self.push(
                                        a,
                                        VOp::Flags {
                                            unit: FlagOp::Mul(MulOp::Mov),
                                            a: x,
                                            b: Operand::None,
                                            pf: Pf::PushZ,
                                            uf: Uf::None,
                                        },
                                    )?;
                                    Cond::IfNA
                                }
                            };
                            self.select(a, cond, y, z)?
                        }
                        /* nir_to_vir.c:1642-1644 (iadd), :1452-1458 (f2i32 as
                         * FTOIZ, truncating), ITOF for i2f32 (:1469). */
                        AluOp::Iadd => self.add(a, AddOp::Add, x, y)?,
                        /* nir_to_vir.c:1669-1677; on bools too (bitwise, as
                         * eval.rs's Iand/Ior/Ixor, on the all-ones pattern every
                         * producer writes: `&&` and `||` lower to these). */
                        AluOp::Iand => self.add(a, AddOp::And, x, y)?,
                        AluOp::Ior => self.add(a, AddOp::Or, x, y)?,
                        AluOp::Ixor => self.add(a, AddOp::Xor, x, y)?,
                        /* The integer division lowering's helpers (prepare.rs
                         * int_div, after nir_lower_idiv); nir_to_vir.c: u2f32
                         * UTOF, f2u32 FTOUZ, ushr SHR, imax MAX. */
                        AluOp::U2f => self.add(a, AddOp::Utof, x, Operand::None)?,
                        AluOp::F2u => self.add(a, AddOp::Ftouz, x, Operand::None)?,
                        AluOp::Ushr => self.add(a, AddOp::Shr, x, y)?,
                        AluOp::Imax => self.add(a, AddOp::Max, x, y)?,
                        /* nir inot is vir_NOT (nir_to_vir.c:1678-1680); a bool
                         * is ~0 or 0, so NOT is its negation exactly. */
                        AluOp::Bnot => self.add(a, AddOp::Not, x, Operand::None)?,
                        /* :1407-1409 */
                        AluOp::Ineg => self.add(a, AddOp::Neg, x, Operand::None)?,
                        /* imul as vir_UMUL (v3d_compiler.h:1584-1588, used at
                         * nir_to_vir.c:1686-1688): MULTOP then UMUL24 on the
                         * same operands give the low 32 bits of the product.
                         * The in-order scheduler keeps the pair in order, and
                         * no thread switch comes between (the program's only
                         * switches are at its end). */
                        AluOp::Imul => {
                            let (x, y) = self.pair(a, x, y)?;
                            self.push(a, VOp::Mul { op: MulOp::Multop, dst: Dst::Nop, a: x, b: y })?;
                            self.mul(a, MulOp::Umul24, x, y)?
                        }
                        AluOp::F2i => self.add(a, AddOp::Ftoiz, x, Operand::None)?,
                        AluOp::I2f => self.add(a, AddOp::Itof, x, Operand::None)?,
                        /* b2f/b2i/i2b: the IR's truth test is "any bit set"
                         * (eval.rs), so each is a select on x != 0 rather
                         * than Mesa's AND with 1.0 / 1 (:1507-1514), which
                         * assumes a bool is exactly ~0 or 0; a bool the GL
                         * core stored as 1 would give 1.4e-45 there. */
                        AluOp::B2f => {
                            let cond = self.nonzero(a, x)?;
                            let one = self.constant(a, 1f32.to_bits(), small_imms)?;
                            let zero = self.constant(a, 0, small_imms)?;
                            self.select(a, cond, one, zero)?
                        }
                        AluOp::B2i => {
                            let cond = self.nonzero(a, x)?;
                            let one = self.constant(a, 1, small_imms)?;
                            let zero = self.constant(a, 0, small_imms)?;
                            self.select(a, cond, one, zero)?
                        }
                        AluOp::I2b => {
                            let cond = self.nonzero(a, x)?;
                            let (t, f) = self.bool_constants(a, small_imms)?;
                            self.select(a, cond, t, f)?
                        }
                        /* bool(float): x != 0.0 (NIR f2b is fneu x, 0.0).
                         * FMOV.pushz tests the float value, so -0.0 is zero
                         * and gives false, as eval.rs's `f(x) != 0.0`; the
                         * same test ntq_fsign uses. */
                        AluOp::F2b => {
                            self.push(
                                a,
                                VOp::Flags {
                                    unit: FlagOp::Mul(MulOp::Fmov),
                                    a: x,
                                    b: Operand::None,
                                    pf: Pf::PushZ,
                                    uf: Uf::None,
                                },
                            )?;
                            let (t, f) = self.bool_constants(a, small_imms)?;
                            self.select(a, Cond::IfNA, t, f)?
                        }
                        /* ntq_fsign (nir_to_vir.c): t = 0.0; FMOV.pushz x,
                         * t = 1.0 where x != 0 (ifna); FMOV.pushn x, t = -1.0
                         * where x < 0 (ifa). -0.0 gives 0.0, as eval.rs. */
                        AluOp::Fsign => {
                            let zero = self.constant(a, 0f32.to_bits(), small_imms)?;
                            let one = self.constant(a, 1f32.to_bits(), small_imms)?;
                            let minus_one = self.constant(a, (-1f32).to_bits(), small_imms)?;
                            let Operand::V(t) = self.mul(a, MulOp::Mov, zero, Operand::None)? else {
                                return Err(SelectError::NotSelected);
                            };
                            self.push(
                                a,
                                VOp::Flags {
                                    unit: FlagOp::Mul(MulOp::Fmov),
                                    a: x,
                                    b: Operand::None,
                                    pf: Pf::PushZ,
                                    uf: Uf::None,
                                },
                            )?;
                            self.push(a, VOp::CondMov { dst: t, src: one, cond: Cond::IfNA })?;
                            self.push(
                                a,
                                VOp::Flags {
                                    unit: FlagOp::Mul(MulOp::Fmov),
                                    a: x,
                                    b: Operand::None,
                                    pf: Pf::PushN,
                                    uf: Uf::None,
                                },
                            )?;
                            self.push(a, VOp::CondMov { dst: t, src: minus_one, cond: Cond::IfA })?;
                            Operand::V(t)
                        }
                        /* dFdx, dFdy (nir_to_vir.c:4052-4065: FDX, FDY of
                         * the value). st/mesa's nir_lower_wpos_ytransform
                         * first scales ddy's operand by the window
                         * transform's y scale (lower_ddy, :209-225): GL's y
                         * runs up the window, the V3D's raster y down, so a
                         * window target (scale -1) negates and an FBO (+1)
                         * does not. The same uniform gl_FragCoord.y reads. */
                        AluOp::Fddx => self.add(a, AddOp::Fdx, x, Operand::None)?,
                        AluOp::Fddy => {
                            let scale = self.uniform(a, Uniform { contents: QUNIFORM_FB_WPOS_Y_SCALE, data: 0 })?;
                            let t = self.mul(a, MulOp::Fmul, x, Operand::V(scale))?;
                            self.add(a, AddOp::Fdy, t, Operand::None)?
                        }
                        /* nir_to_vir.c:1755-1760: FFLOOR, FCEIL */
                        AluOp::Ffloor => self.add(a, AddOp::Ffloor, x, Operand::None)?,
                        AluOp::Fceil => self.add(a, AddOp::Fceil, x, Operand::None)?,
                        /* lower_ffract (v3d_screen.c:568): a - ffloor(a)
                         * (nir_opt_algebraic.py:546), with the V3D's FSUB */
                        AluOp::Ffract => {
                            let f = self.add(a, AddOp::Ffloor, x, Operand::None)?;
                            self.add(a, AddOp::Fsub, x, f)?
                        }
                        /* nir_to_vir.c:1742-1753 */
                        AluOp::Frcp => self.add(a, AddOp::Recip, x, Operand::None)?,
                        AluOp::Frsq => self.add(a, AddOp::Rsqrt, x, Operand::None)?,
                        AluOp::Fexp2 => self.add(a, AddOp::Exp, x, Operand::None)?,
                        AluOp::Flog2 => self.add(a, AddOp::Log, x, Operand::None)?,
                        /* lower_fsqrt (v3d_screen.c:585): fsqrt(a) is
                         * frcp(frsq(a)) (nir_opt_algebraic.py:1908) */
                        AluOp::Fsqrt => {
                            let r = self.add(a, AddOp::Rsqrt, x, Operand::None)?;
                            self.add(a, AddOp::Recip, r, Operand::None)?
                        }
                        AluOp::Fsin => self.sincos(a, x, false, small_imms)?,
                        AluOp::Fcos => self.sincos(a, x, true, small_imms)?,
                        /* fabs: FMOV with the ABS unpack (:1779-1789) */
                        AluOp::Fabs => {
                            let v = self.in_register(a, x)?;
                            self.mul(a, MulOp::Fmov, Operand::Abs(v), Operand::None)?
                        }
                        _ => return Err(SelectError::NotSelected),
                    }
                }
                /* An attribute component is read where prepare's order first
                 * needs it, not all at the top: sixteen vec4 attributes live
                 * at once are 64 values, more than a thread's registers
                 * (dEQP attribute_location.bind_max_attributes). ldvpmv_in
                 * addresses any row at any point; every read precedes the
                 * first output write, which select_vertex emits after the
                 * body (the input and output segments are shared). */
                /* Read at the top already (the eager layout). */
                SOp::Attr { .. } | SOp::VertexId | SOp::InstanceId if self.attr_rows.is_empty() => continue,
                SOp::Attr { .. } | SOp::VertexId | SOp::InstanceId => {
                    let row = *self.attr_rows.get(i).ok_or(SelectError::AttributeWithoutRecord)?;
                    let idx = self.uniform(a, Uniform::constant(row))?;
                    self.add(a, AddOp::LdvpmvIn, Operand::V(idx), Operand::None)?
                }
                SOp::Varying { .. } => continue,
                /* prepare refuses a live Undef; one here is a defect, and
                 * an operand left unset would read a real register. */
                SOp::Undef => return Err(SelectError::UndefinedValue),
            };
            self.of[i] = o;
        }
        /* the last stretch's queued look-ups, before the discard, the
         * outputs and the last switch (select_fragment, select_vertex) */
        self.flush_tmu(a)
    }
    /// The thread switch and the ldtmus of every queued look-up, oldest
    /// first: the order they were requested in, which is the order the TMU
    /// returns their words in (ntq_flush_tmu, nir_to_vir.c:262-293). A
    /// switch between the requests and their results lets the QPU wait for
    /// the TMU in another thread's time instead of stalling on the ldtmu
    /// (the board hung in the fragment stage of the first textured job
    /// without one).
    fn flush_tmu(&mut self, a: &Arena<'a>) -> Result<(), SelectError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.push(a, VOp::Thrsw)?;
        for k in 0..self.pending.len() {
            let (tex, words, ret32) = self.pending.as_slice()[k];
            let mut regs = [None; 4];
            for (w, r) in regs.iter_mut().enumerate() {
                if words & (1 << w) != 0 {
                    let v = self.v();
                    self.push(a, VOp::Ldtmu { dst: v })?;
                    *r = Some(v);
                }
            }
            self.tex_words.push(a, (tex, regs, ret32))?;
        }
        while self.pending.pop().is_some() {}
        Ok(())
    }
}

/// Whether input `k` (0 = a, 1 = b) of `op` can take the f32 input unpack
/// that `unpack` carries ([`Operand::Abs`], [`Operand::Lo`] or
/// [`Operand::Hi`]), read from our encoder's field table (encode.rs
/// add_pack / mul_pack; decode.rs `add_unpack` / `mul_unpack` read the same
/// fields):
///
/// - FADD, FADDNF, FSUB, FMIN, FMAX, FCMP and FMUL: ABS, L and H on both
///   inputs; FMOV on its one input.
/// - VFPACK: L and H on both inputs, no ABS (encode.rs refuses it; Mesa's
///   "these ops can't represent abs", vir_opt_copy_propagate.c:196-212).
/// - FFLOOR, FROUND, FTRUNC, FCEIL, FDX, FDY: L and H on input a, no ABS
///   (code 0 of their field is refused).
/// - FTOIN, FTOIZ, FTOUZ, FTOC: none. Mesa takes L and H there
///   (v3d_qpu_unpacks_f32, qpu_instr.c:1153-1192, Mesa 26.2.1), but our
///   encoder refuses them as unmeasured on silicon
///   (`EncodeError::UnpackUnmeasuredOnSilicon`).
/// - Every other operation (integer, MOV, the SFU writes, the TMU and VPM
///   writes) has no f32 unpack field.
///
/// Any other operand, and a select's CondMov or a Copy (not ADD or MUL
/// operations), is never given one.
fn takes_unpack(op: &VOp, k: usize, unpack: Operand) -> bool {
    let abs = matches!(unpack, Operand::Abs(_));
    let half = matches!(unpack, Operand::Lo(_) | Operand::Hi(_));
    if !abs && !half {
        return false;
    }
    let two =
        |o: AddOp| matches!(o, AddOp::Fadd | AddOp::Faddnf | AddOp::Fsub | AddOp::Fmin | AddOp::Fmax | AddOp::Fcmp);
    let one =
        |o: AddOp| matches!(o, AddOp::Ffloor | AddOp::Fround | AddOp::Ftrunc | AddOp::Fceil | AddOp::Fdx | AddOp::Fdy);
    match *op {
        VOp::Add { op, .. } | VOp::Flags { unit: FlagOp::Add(op), .. } => {
            two(op) || (half && op == AddOp::Vfpack) || (half && k == 0 && one(op))
        }
        VOp::Mul { op: MulOp::Fmul, .. } | VOp::Flags { unit: FlagOp::Mul(MulOp::Fmul), .. } => true,
        VOp::Mul { op: MulOp::Fmov, .. } | VOp::Flags { unit: FlagOp::Mul(MulOp::Fmov), .. } => k == 0,
        _ => false,
    }
}

/// Whether `op` is FADD, FADDNF, FMIN or FMAX, which Mesa's packer
/// (qpu_pack.c v3d42_qpu_add_pack, the encoder) tells apart by the order of
/// their inputs.
fn ordered_inputs(op: &VOp) -> bool {
    matches!(
        op,
        VOp::Add { op: AddOp::Fadd | AddOp::Faddnf | AddOp::Fmin | AddOp::Fmax, .. }
            | VOp::Flags { unit: FlagOp::Add(AddOp::Fadd | AddOp::Faddnf | AddOp::Fmin | AddOp::Fmax), .. }
    )
}

/// `unpack` ([`Operand::Abs`], [`Operand::Lo`] or [`Operand::Hi`]) read
/// from register `s` instead.
fn unpack_from(unpack: Operand, s: V) -> Operand {
    match unpack {
        Operand::Abs(_) => Operand::Abs(s),
        Operand::Lo(_) => Operand::Lo(s),
        Operand::Hi(_) => Operand::Hi(s),
        other => other,
    }
}

/// Mesa's copy propagation of an unpacking move (vir_opt_copy_propagate.c
/// `try_copy_prop`, :167-216, Mesa 26.2.1, MIT): each read of `d` from
/// `fmov d, s.u` (u = ABS, L or H) becomes a read of `s.u` where the
/// reading input takes that unpack ([`takes_unpack`]), and the move goes
/// when nothing reads `d` any more.
///
/// - fabs is selected as `fmov d, s.abs` (nir_to_vir.c:1779-1789); this
///   makes `1.0 - abs(x)` one FSUB, as Mesa's `fsub r2, 0x3f800000,
///   rf3.abs` in femtovg's fill_image.
/// - A look-up lane is `fmov d, w.l` / `w.h` (Sel::body, SOp::TexLane);
///   this makes `lane * c` one FMUL, `gl_FragColor = texture2D(..)` two
///   VFPACKs on the words, and a comparison of a lane one FCMP.
///
/// Only a read of `d` itself (an [`Operand::V`]) is rewritten: an input
/// with an unpack of its own keeps the move (Mesa: "No composing the
/// unpacks"), so `abs(lane)` is still two moves. An L or H move goes only
/// when every read of it takes the unpack (below); Mesa folds into each
/// reader that can, but here that measured longer programs.
///
/// `d` must be written by the move alone, and every write of `s` must come
/// before the move in program order: then `s` holds at each reader the
/// value the move read (a select's result qualifies, both its writes being
/// before; a loop-carried value written at the end of a pass after the move
/// does not, as a reader in a later pass would see the new value).
fn fold_unpacks<'a>(a: &Arena<'a>, ops: &mut AVec<'a, VOp>, vregs: u32) -> Result<(), SelectError> {
    let n = vregs as usize;
    const NONE: u32 = u32::MAX;
    /* per register: how many operations write it (a definition or a
     * rewrite), and the index of the last one */
    let writes = a.alloc_filled(n, 0u32)?;
    let last_write = a.alloc_filled(n, NONE)?;
    for (i, op) in ops.as_slice().iter().enumerate() {
        let w = match *op {
            VOp::CondMov { dst, .. } | VOp::Copy { dst, .. } => Some(dst),
            _ => op.def(),
        };
        if let Some(v) = w {
            writes[v.0 as usize] += 1;
            last_write[v.0 as usize] = i as u32;
        }
    }
    /* unpack_of[d] = the operand of a foldable `fmov d, s.u` (s.abs, s.l or
     * s.h); Operand::None for any other d */
    let unpack_of = a.alloc_filled(n, Operand::None)?;
    for (i, op) in ops.as_slice().iter().enumerate() {
        if let VOp::Mul { op: MulOp::Fmov, dst: Dst::V(d), a: u, b: Operand::None } = *op {
            if let Operand::Abs(s) | Operand::Lo(s) | Operand::Hi(s) = u {
                if writes[d.0 as usize] == 1 && last_write[s.0 as usize] < i as u32 {
                    unpack_of[d.0 as usize] = u;
                }
            }
        }
    }
    /* An L or H move goes only when every read of it takes the unpack;
     * otherwise every reader keeps reading the move's register. Folding
     * into some readers while the move stays saves no instruction and keeps
     * the look-up's word live longer: measured 2026-09-29 (step (h)), it
     * made femtovg's fill_image and texture_copy_unclipped 1-4 words
     * longer, their texType selects then keeping real MOVs. An ABS move
     * still folds into each reader that takes it (fabs's own, below). */
    for op in ops.as_slice() {
        let uses = op.uses();
        let alu = matches!(op, VOp::Add { .. } | VOp::Mul { .. } | VOp::Flags { .. });
        for (k, u) in uses.iter().enumerate() {
            let Some(d) = u.vreg() else { continue };
            let m = unpack_of[d.0 as usize];
            if !matches!(m, Operand::Lo(_) | Operand::Hi(_)) {
                continue;
            }
            let twice = ordered_inputs(op) && uses[0] == uses[1];
            if !(alu && matches!(u, Operand::V(_)) && takes_unpack(op, k, m) && !twice) {
                unpack_of[d.0 as usize] = Operand::None;
            }
        }
    }
    /* Rewrite the reads that can take the unpack; count the reads left,
     * through an unpack or not. */
    let left = a.alloc_filled(n, 0u32)?;
    for op in ops.as_mut_slice() {
        let before = *op;
        let mut after = before;
        if let VOp::Add { a: x, b: y, .. } | VOp::Mul { a: x, b: y, .. } | VOp::Flags { a: x, b: y, .. } = &mut after {
            for (k, slot) in [x, y].into_iter().enumerate() {
                if let Operand::V(d) = *slot {
                    let u = unpack_of[d.0 as usize];
                    if let (Some(s), true) = (u.vreg(), takes_unpack(&before, k, u)) {
                        *slot = unpack_from(u, s);
                    }
                }
            }
        }
        /* FADD/FADDNF and FMIN/FMAX are told apart by the order of their
         * inputs (encode.rs add_pack): two identical inputs encode only FADD
         * and FMIN, so such a read keeps the move. */
        if ordered_inputs(&after) && after.uses()[0] == after.uses()[1] && after != before {
            after = before;
        }
        *op = after;
        for u in after.uses() {
            if let Some(v) = u.vreg() {
                left[v.0 as usize] += 1;
            }
        }
    }
    /* Drop, in place, the moves nothing reads now. */
    let all = ops.as_mut_slice();
    let len = all.len();
    let mut keep = 0;
    for i in 0..len {
        let op = all[i];
        if let VOp::Mul { op: MulOp::Fmov, dst: Dst::V(d), .. } = op {
            if unpack_of[d.0 as usize] != Operand::None && left[d.0 as usize] == 0 {
                continue;
            }
        }
        all[keep] = op;
        keep += 1;
    }
    for _ in keep..len {
        ops.pop();
    }
    Ok(())
}

/// Every operand an operation reads is there: an [`Operand::None`] where the
/// operation reads one would encode as a read of r0, a plausible value.
fn no_missing_operands(ops: &[VOp]) -> Result<(), SelectError> {
    use super::isa::{add_op_args, mul_op_args, num_src};
    for op in ops {
        let (n, a, b) = match *op {
            VOp::Add { op, a, b, .. } | VOp::Flags { unit: FlagOp::Add(op), a, b, .. } => {
                (num_src(add_op_args(op)), a, b)
            }
            VOp::Mul { op, a, b, .. } | VOp::Flags { unit: FlagOp::Mul(op), a, b, .. } => {
                (num_src(mul_op_args(op)), a, b)
            }
            VOp::CondMov { src, .. } | VOp::Setmsf { src, .. } => (1, src, Operand::None),
            _ => continue,
        };
        if (n >= 1 && a == Operand::None) || (n >= 2 && b == Operand::None) {
            return Err(SelectError::MissingOperand);
        }
    }
    Ok(())
}

/// The TLB config for render target `rt` (vir_emit_tlb_color_write,
/// nir_to_vir.c:1953-1984): f16 colour, per-pixel sample mode, the RT
/// number reversed ("7 = RT 0", :1932), the half swap Mesa sets for every
/// f16 target, and the vector size from the number of components. For RT 0
/// with 3 or 4 components it is !0, the TLB's default, and Mesa writes no
/// config (TLB, not TLBU).
pub fn tlb_config_f16(rt: u32, components: u32) -> u32 {
    const TLB_TYPE_F16_COLOR: u32 = 3 << 6;
    const TLB_RENDER_TARGET_SHIFT: u32 = 3;
    const TLB_SAMPLE_MODE_PER_PIXEL: u32 = 1 << 2;
    const TLB_F16_SWAP_HI_LO: u32 = 1 << 1;
    const TLB_VEC_SIZE_4_F16: u32 = 1;
    let mut conf = 0xffff_ff00 | TLB_SAMPLE_MODE_PER_PIXEL | ((7 - rt) << TLB_RENDER_TARGET_SHIFT);
    conf |= TLB_TYPE_F16_COLOR | TLB_F16_SWAP_HI_LO;
    if components >= 3 {
        conf |= TLB_VEC_SIZE_4_F16;
    }
    conf
}

/// How hard selection works to keep values out of registers, in the order
/// the chain tries them (`chain.rs` `vertex` and `fragment`): each level is
/// asked for only when the one before does not fit 2 threads' registers,
/// in place of Mesa's spilling, so every program that fits keeps Mesa's
/// layout.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Pressure {
    /// Mesa's layout: a vertex shader reads every attribute at the top, and
    /// a uniform or a constant is loaded once, at its first read.
    None,
    /// A vertex shader reads each attribute component where it is first
    /// needed, as Mesa's ntq_setup_vs_inputs does (no effect on a fragment
    /// shader).
    LazyAttributes,
    /// As [`Pressure::LazyAttributes`], and a uniform, or a constant that is
    /// not a small immediate, is loaded again at each read ([`Sel::val`]).
    Remat,
    /// As [`Pressure::Remat`], and every look-up has its own thread switch,
    /// its result read right after it: Mesa's last strategies before the
    /// fallback scheduler turn TMU pipelining off (vir.c:2197, "disable TMU
    /// pipelining (2t)"), as queued results are live longer.
    Serial,
}

/// The TMU output FIFO's words per thread at 4 threads: `16 / threads`
/// (ntq_tmu_fifo_overflow, nir_to_vir.c:251-258). It cannot stall, so the
/// words of the queued look-ups never exceed it. The thread count is chosen
/// after selection (chain.rs `build`: 4, else 2), so the 4-thread bound is
/// taken for both: at 2 threads it leaves 4 of the 8 words unused.
pub const TMU_OUTPUT_WORDS: u32 = 16 / 4;
/// At most this many look-ups queued (MAX_TMU_QUEUE_SIZE, v3d_compiler.h:62;
/// ntq_tmu_fifo_overflow). With f16 results of 1 or 2 words each (a 32-bit
/// output: up to 4), [`TMU_OUTPUT_WORDS`] is reached first.
pub const MAX_TMU_QUEUE: usize = 8;

/// Whether `op` ends a straight-line stretch of `p`: a loop's or a uniform
/// branch's structure, a loop's carried value (written at a loop edge) and
/// a merge (written in each arm). Queued look-ups are flushed before it,
/// and [`issue_order`] moves nothing across it.
fn ends_stretch(op: &SOp) -> bool {
    matches!(
        op,
        SOp::LoopHead
            | SOp::LoopEnd { .. }
            | SOp::LoopBreak { .. }
            | SOp::Phi { .. }
            | SOp::PhiNext { .. }
            | SOp::IfBranch { .. }
            | SOp::IfElse
            | SOp::IfEnd
            | SOp::IfMerge { .. }
    )
}

/// The order [`Sel::body`] selects `p`'s operations in: `p`'s own, except
/// that a look-up moves up, with the operations its coordinates need, to
/// right after an earlier look-up of the same straight-line stretch, so the
/// two share one thread switch ([`Sel::flush_tmu`]). Mesa gets the same from
/// nir_schedule, which starts texture fetches early (a large delay for a
/// tex instruction, nir_schedule.c:1115-1120) before its back end queues
/// them (vir.c:2096-2118).
///
/// A group starts at a look-up and takes each later look-up of the stretch
/// that fits: its words and the group's within [`TMU_OUTPUT_WORDS`], at most
/// [`MAX_TMU_QUEUE`] look-ups, and its coordinates independent of the
/// group. A look-up whose coordinates read a lane of a look-up at or after
/// the group's first (a dependent read) stays where it is and gets a switch
/// of its own. Only pure operations move (the stretch has no structure in
/// it: [`ends_stretch`]), each after its operands, so every operand is
/// still selected before its reader. The results' readers keep their
/// order, so the first of them flushes the group.
fn issue_order<'a>(p: &Prepared<'_>, a: &Arena<'a>) -> Result<&'a [u32], SelectError> {
    let n = p.ops.len();
    let out = a.alloc_filled(n, 0u32)?;
    for (k, o) in out.iter_mut().enumerate() {
        *o = k as u32;
    }
    /* per look-up, the words its lanes read (Sel::body's P0 mask) */
    let words = a.alloc_filled(n, 0u32)?;
    for op in p.ops {
        if let SOp::TexLane { tex, lane } = *op {
            words[tex[0].0 as usize] |= 1 << (lane / 2);
        }
    }
    let in_range = a.alloc_filled(n, false)?;
    let needed = a.alloc_filled(n, false)?;
    let tmp = a.alloc_filled(n, 0u32)?;
    let issued = |x: u32| matches!(p.ops[x as usize], SOp::Tex { .. }) && words[x as usize] != 0;
    let mut start = 0usize;
    while start < n {
        /* the stretch [start, end): nothing moves across a boundary, so
         * `out` is still the identity at them */
        let mut end = start;
        while end < n && !ends_stretch(&p.ops[end]) {
            end += 1;
        }
        let mut head = start;
        loop {
            while head < end && !issued(out[head]) {
                head += 1;
            }
            if head >= end {
                break;
            }
            /* out[head..=last] is the group, its look-ups side by side */
            let mut last = head;
            let mut used = words[out[head] as usize].count_ones();
            let mut count = 1usize;
            for j in head + 1..end {
                let tb = out[j];
                if !issued(tb) {
                    continue;
                }
                let w = words[tb as usize].count_ones();
                if used + w > TMU_OUTPUT_WORDS || count >= MAX_TMU_QUEUE {
                    break;
                }
                /* the operations between the group and `tb` that `tb`'s
                 * operands need, walking back from it */
                for &x in &out[head..j] {
                    in_range[x as usize] = true;
                }
                for x in p.ops[tb as usize].operands() {
                    if in_range[x.0 as usize] {
                        needed[x.0 as usize] = true;
                    }
                }
                let mut dependent = false;
                for k in (head..j).rev() {
                    let x = out[k] as usize;
                    if !needed[x] {
                        continue;
                    }
                    /* a look-up of the range reached through one of its
                     * lanes: `tb` reads a result not returned yet */
                    if matches!(p.ops[x], SOp::Tex { .. }) {
                        dependent = true;
                    }
                    for y in p.ops[x].operands() {
                        if in_range[y.0 as usize] {
                            needed[y.0 as usize] = true;
                        }
                    }
                }
                if !dependent {
                    /* out[head..=j] becomes: the needed operations, the
                     * group, `tb`, the rest in their order */
                    let mut m = 0;
                    for &x in out[last + 1..j].iter().filter(|&&x| needed[x as usize]) {
                        tmp[m] = x;
                        m += 1;
                    }
                    let moved = m;
                    for &x in &out[head..=last] {
                        tmp[m] = x;
                        m += 1;
                    }
                    tmp[m] = tb;
                    m += 1;
                    for &x in out[last + 1..j].iter().filter(|&&x| !needed[x as usize]) {
                        tmp[m] = x;
                        m += 1;
                    }
                    for &x in &tmp[..m] {
                        in_range[x as usize] = false;
                        needed[x as usize] = false;
                    }
                    out[head..=j].copy_from_slice(&tmp[..m]);
                    head += moved;
                    last = head + count;
                    count += 1;
                    used += w;
                } else {
                    for &x in &out[head..=j] {
                        in_range[x as usize] = false;
                        needed[x as usize] = false;
                    }
                }
            }
            head = last + 1;
        }
        start = end + 1;
    }
    Ok(out)
}

/// Per prepared scalar of `p`, the uniform word of a value that is only a
/// load: a uniform, or a constant that is not a small immediate where the
/// stage uses them ([`Sel::val`] makes the load).
fn deferred_loads<'a>(p: &Prepared<'_>, small_imms: bool, a: &Arena<'a>) -> Result<&'a [Option<Uniform>], SelectError> {
    let t = a.alloc_filled(p.ops.len(), None::<Uniform>)?;
    for (e, op) in t.iter_mut().zip(p.ops) {
        *e = match *op {
            SOp::Uniform(dw) => Some(Uniform { contents: QUNIFORM_UNIFORM, data: dw as u32 }),
            SOp::TexSizeWord { sampler, lane } => {
                Some(Uniform { contents: QUNIFORM_TEXTURE_WIDTH + u32::from(lane), data: u32::from(sampler) })
            }
            SOp::Const(bits) if !(small_imms && small_imm_index(bits).is_some()) => Some(Uniform::constant(bits)),
            _ => None,
        };
    }
    Ok(t)
}

/// Two uniform exits to one label, one after the other (a counted loop's
/// own test, then a `break`; prepare.rs SOp::LoopBreak), take one branch:
/// Mesa merges two such terminators into one if on the OR of their
/// conditions (nir_opt_loop.c opt_loop_merge_terminators :649-712,
/// merge_terminators :554-604, Mesa 26.2.1), and builds that OR as
/// booleans (8 words before its branch in filter_image). Here the second
/// comparison folds its result into flag A instead of pushing (the flag
/// update ANDxx / NORxx, which Mesa uses for divergent control flow,
/// nir_to_vir.c:3783-3785), so that A is "no exit": `fcmp.pushc; fcmp.andnc;
/// bu.anynap` in place of two compares, two branches and six delay slots.
///
/// The shape, in the selected operations: `Flags` (a push), `FwdBranch` to
/// label L, only register arithmetic and uniform loads, `Flags` (a push),
/// `FwdBranch` to L, both branches "any A" or "any not A" with one msfign.
/// As in Mesa (:684-696), the operations between run on the path that
/// exits too, so only ones without an effect outside the registers qualify:
/// no flag use, no TMU / TLB / VPM access, no thread switch or other branch.
/// The counted back edge, which bounds the loop (ES 1.00 Appendix A), is
/// not a forward branch and is never merged. Labels are resolved by the
/// scheduler after this, so removing a branch moves no other target.
fn merge_exits<'a>(a: &Arena<'a>, ops: &mut AVec<'a, VOp>) -> Result<(), SelectError> {
    let pure = |o: &VOp| match *o {
        VOp::Add { dst: Dst::V(_), op, .. } => op != AddOp::LdvpmvIn,
        VOp::Mul { dst: Dst::V(_), .. } | VOp::Ldunif { .. } | VOp::Copy { .. } => true,
        _ => false,
    };
    let exit = |o: &VOp| match *o {
        VOp::FwdBranch { cond: c @ (BranchCond::AnyA | BranchCond::AnyNA), msfign, label } => Some((c, msfign, label)),
        _ => None,
    };
    let pushed = |o: &VOp| matches!(o, VOp::Flags { pf, uf: Uf::None, .. } if *pf != Pf::None);
    let s = ops.as_mut_slice();
    let mut drop = AVec::<'a, usize>::new();
    let mut i = 0;
    while i + 1 < s.len() {
        let Some((c1, m1, l1)) = exit(&s[i]) else {
            i += 1;
            continue;
        };
        if i == 0 || !pushed(&s[i - 1]) {
            i += 1;
            continue;
        }
        /* the next exit: pure operations, its push, the branch */
        let Some(j) = (i + 1..s.len()).find(|&k| !pure(&s[k])) else { break };
        let merged = match (s.get(j), s.get(j + 1).and_then(exit)) {
            (Some(VOp::Flags { unit, a: x, b: y, pf, uf: Uf::None }), Some((c2, m2, l2)))
                if *pf != Pf::None && m2 == m1 && l2 == l1 =>
            {
                /* A after the first push; exit1 = A under AnyA, !A under
                 * AnyNA. The second's bit x, exit2 = x or !x likewise. The
                 * new A is "no exit" = !exit1 & !exit2. */
                use Uf::*;
                let uf = match (c1, c2, pf) {
                    (BranchCond::AnyNA, BranchCond::AnyA, Pf::PushZ) => AndNZ,
                    (BranchCond::AnyNA, BranchCond::AnyA, Pf::PushN) => AndNN,
                    (BranchCond::AnyNA, BranchCond::AnyA, _) => AndNC,
                    (BranchCond::AnyNA, _, Pf::PushZ) => AndZ,
                    (BranchCond::AnyNA, _, Pf::PushN) => AndN,
                    (BranchCond::AnyNA, _, _) => AndC,
                    (_, BranchCond::AnyA, Pf::PushZ) => NorZ,
                    (_, BranchCond::AnyA, Pf::PushN) => NorN,
                    (_, BranchCond::AnyA, _) => NorC,
                    (_, _, Pf::PushZ) => NorNZ,
                    (_, _, Pf::PushN) => NorNN,
                    (_, _, _) => NorNC,
                };
                Some(VOp::Flags { unit: *unit, a: *x, b: *y, pf: Pf::None, uf })
            }
            _ => None,
        };
        match merged {
            Some(f) => {
                s[j] = f;
                s[j + 1] = VOp::FwdBranch { cond: BranchCond::AnyNA, msfign: m1, label: l1 };
                drop.push(a, i)?;
                i = j + 1;
            }
            None => i += 1,
        }
    }
    if drop.is_empty() {
        return Ok(());
    }
    let mut out: AVec<'a, VOp> = AVec::with_capacity(a, s.len() - drop.len())?;
    let d = drop.as_slice();
    for (k, o) in s.iter().enumerate() {
        if !d.contains(&k) {
            out.push(a, *o)?;
        }
    }
    *ops = out;
    Ok(())
}

/// Select a fragment shader. `inputs` is [`super::prepare::fragment_inputs`];
/// bit i of `flat` says `inputs[i]` is flat (the program's
/// `flat_shade_flags`).
pub fn select_fragment<'a>(
    p: &Prepared<'_>,
    inputs: &[(u8, u8)],
    flat: u32,
    pressure: Pressure,
    a: &Arena<'a>,
) -> Result<Selected<'a>, SelectError> {
    for op in p.ops {
        if let SOp::Varying { loc, comp } = *op {
            if !inputs.contains(&(loc, comp)) {
                return Err(SelectError::VaryingNotAnInput);
            }
        }
    }
    let mut s = Sel {
        ops: AVec::new(),
        next: 0,
        of: a.alloc_filled(p.ops.len(), Operand::None)?,
        tex_words: AVec::new(),
        pending: AVec::new(),
        ret32: crate::shaderc::compile_lod_key().ret32,
        pipelined: pressure < Pressure::Serial,
        attr_rows: &mut [],
        loop_counters: AVec::new(),
        phi_saved: AVec::new(),
        in_phi_copies: false,
        fragment: true,
        deferred: deferred_loads(p, true, a)?,
        remat: pressure >= Pressure::Remat,
        last_push: None,
        labels: 0,
        ifs: AVec::new(),
        merges: AVec::new(),
        cached: AVec::new(),
    };
    /* gl_PointCoord: the two implicit point varyings come before every
     * declared one (Mesa nir_to_vir.c:4869-4873, before ntq_setup_fs_inputs
     * :4948), smooth like any other (emit_fragment_varying with no variable,
     * :1080-1083); y goes through the target's transform (Mesa's
     * nir_lower_pntc_ytransform: ffma(y, transform[0], transform[1])). The
     * program is then for points draws only: the job builder refuses
     * another primitive by name (the lead's decision, 2026-09-24). */
    let implicit_point_varyings = p.ops.iter().any(|o| matches!(o, SOp::PointCoord { .. }));
    if implicit_point_varyings {
        let mut lanes = [Operand::None; 2];
        for l in lanes.iter_mut() {
            let raw = s.v();
            s.push(a, VOp::Ldvary { dst: raw })?;
            let scaled = s.mul(a, MulOp::Fmul, Operand::V(raw), Operand::Payload(0))?;
            *l = s.add(a, AddOp::Fadd, scaled, Operand::R5)?;
        }
        let scale = s.uniform(a, Uniform { contents: QUNIFORM_FB_PNTC_Y_SCALE, data: 0 })?;
        let offset = s.uniform(a, Uniform { contents: QUNIFORM_FB_PNTC_Y_OFFSET, data: 0 })?;
        let y = s.mul(a, MulOp::Fmul, lanes[1], Operand::V(scale))?;
        let y = s.add(a, AddOp::Fadd, y, Operand::V(offset))?;
        for (i, op) in p.ops.iter().enumerate() {
            match *op {
                SOp::PointCoord { lane: 0 } => s.of[i] = lanes[0],
                SOp::PointCoord { .. } => s.of[i] = y,
                _ => {}
            }
        }
    }
    /* The varyings first, in input order: each ldvary takes the next one.
     * A smooth one is vary * W + C (emit_smooth_varying); a flat one is C
     * alone, which its flat-shade flag makes the provoking vertex's value,
     * and the ldvary's own result is not read (emit_flat_varying,
     * nir_to_vir.c: MOV of the C register). */
    for (i, &(loc, comp)) in inputs.iter().enumerate() {
        let raw = s.v();
        s.push(a, VOp::Ldvary { dst: raw })?;
        let value = if i < 32 && flat & (1 << i) != 0 {
            s.mul(a, MulOp::Mov, Operand::R5, Operand::None)?
        } else {
            let scaled = s.mul(a, MulOp::Fmul, Operand::V(raw), Operand::Payload(0))?;
            s.add(a, AddOp::Fadd, scaled, Operand::R5)?
        };
        for (i, op) in p.ops.iter().enumerate() {
            if *op == (SOp::Varying { loc, comp }) {
                s.of[i] = value;
            }
        }
    }
    s.body(p, true, a)?;

    /* A discard: the lanes it holds in clear their multisample flags, so the
     * TLB drops their writes (nir_to_vir.c:3753-3792). Its flags are pushed
     * right before it, with no thread switch between (flags do not survive
     * one); it may come after the last switch, as in Mesa, where a discard
     * is emitted in the body and the last switch is often a look-up's. */
    if let Some(d) = p.discard {
        let zero = s.constant(a, 0, true)?;
        let cond = match p.ops[d.0 as usize] {
            SOp::Const(0) => None,
            SOp::Const(_) => Some(Cond::None),
            SOp::Alu(c, cs) if is_comparison(c) => {
                let (cx, cy) = (s.val(a, cs[0])?, s.val(a, cs[1])?);
                Some(s.compare(a, c, cx, cy)?)
            }
            _ => {
                let x = s.val(a, d)?;
                Some(s.nonzero(a, x)?)
            }
        };
        if let Some(cond) = cond {
            s.push(a, VOp::Setmsf { src: zero, cond })?;
        }
    }

    /* The last thread switch, then the colour (nir_to_vir.c:5085-5099). */
    s.last_thrsw(a, true)?;
    /* A shader that discards runs with late Z/stencil tests and writes Z
     * itself (emit_frag_end, nir_to_vir.c:2071-2129). The default Z and
     * stencil update comes from the FEP, before the shader, so with early
     * tests a discarded fragment still updates the stencil: femtovg's
     * stroke pass incremented it under every fringe pixel it discarded, and
     * its edge pass (stencil EQUAL 0) drew none of them (the Pi 4, 1188 of
     * 1188 fringe pixels, femtovg edge image 998e5188). Mesa keeps early
     * tests with a discard only when the draw writes neither depth nor
     * stencil (can_earlyz_with_discard, v3d_program.c:627-631); the program
     * does not know the draw, so it always takes the late path, which costs
     * speed only. A shader that reads Z (gl_FragCoord.z, the payload rf2)
     * writes it too, as the 4.2 spec asks and Mesa does (`reads_z`). The
     * write is the FEP's Z passed through, before the colour writes; its
     * value is ignored (INVARIANT), so the source is rf2, the FEP's Z,
     * where Mesa reads an undefined rf5. */
    let reads_z = p.ops.contains(&SOp::FragCoord { lane: 2 });
    /* No colour write: Mesa writes Z and flags the shader as possibly
     * discarding, so it takes the late path too (nir_to_vir.c:2120-2126, and
     * no early-tests promotion without a colour write, :2067-2078). */
    let disable_ez = p.discard.is_some() || p.rt_mask == 0;
    let writes_z = disable_ez || reads_z;
    if writes_z {
        s.push(
            a,
            VOp::Mul {
                op: MulOp::Mov,
                dst: Dst::Tlbu(Uniform::constant(TLB_Z_FROM_FEP)),
                a: Operand::Payload(2),
                b: Operand::None,
            },
        )?;
    }
    /* One f16 vec4 write per render target the shader writes, in RT order
     * (emit_frag_end calls vir_emit_tlb_color_write for rt 0..7): the
     * first word of each through TLBU with the RT's config, unless the
     * config is the default (RT 0). */
    for (rt, colour) in p.color.iter().enumerate() {
        if p.rt_mask & (1 << rt) == 0 {
            continue;
        }
        let mut c = [Operand::None; 4];
        for (o, &x) in c.iter_mut().zip(colour) {
            *o = s.val(a, x)?;
        }
        let conf = tlb_config_f16(rt as u32, 4);
        let first = if conf != !0 { Dst::Tlbu(Uniform::constant(conf)) } else { Dst::Tlb };
        let (r, g) = s.pair(a, c[0], c[1])?;
        let (b, al) = s.pair(a, c[2], c[3])?;
        s.push(a, VOp::Add { op: AddOp::Vfpack, dst: first, a: r, b: g })?;
        s.push(a, VOp::Add { op: AddOp::Vfpack, dst: Dst::Tlb, a: b, b: al })?;
    }
    fold_unpacks(a, &mut s.ops, s.next)?;
    merge_exits(a, &mut s.ops)?;
    no_missing_operands(s.ops.as_slice())?;
    /* Any read of rf0, as Mesa decides it (vir_check_payload_w,
     * nir_to_vir.c:5054-5069): the varyings' W and gl_FragCoord.w. */
    let uses_center_w = s.ops.as_slice().iter().any(|o| o.uses().contains(&Operand::Payload(0)));
    Ok(Selected {
        ops: s.ops.into_slice(),
        fragment: true,
        vregs: s.next,
        vattr_sizes: [0; 16],
        vpm_output_rows: 0,
        /* prepare makes SOp::VertexId and SOp::InstanceId in a vertex
         * shader only */
        uses_vid: false,
        uses_iid: false,
        uses_center_w,
        implicit_point_varyings,
        writes_z,
        disable_ez,
    })
}

/// The VPM rows each attribute record of `p` takes: the highest component
/// read, plus one, per record (`records[r]` is the location of record `r`).
pub fn vattr_sizes(p: &Prepared<'_>, records: &[u8]) -> Result<[u8; 16], SelectError> {
    let mut sizes = [0u8; 16];
    for op in p.ops {
        if let SOp::Attr { loc, comp } = *op {
            let r = records.iter().position(|&l| l == loc).ok_or(SelectError::AttributeWithoutRecord)?;
            let slot = sizes.get_mut(r).ok_or(SelectError::VpmRowOutOfRange)?;
            *slot = (*slot).max(comp + 1);
        }
    }
    Ok(sizes)
}

/// Select a vertex shader (`coord` false) or the coordinate shader (`coord`
/// true) of the same source. `records` is the attribute records' locations,
/// in record order (the vertex shader's; the coordinate shader reads a
/// subset through the same records).
/// `pressure`: see [`Pressure`].
pub fn select_vertex<'a>(
    p: &Prepared<'_>,
    records: &[u8],
    coord: bool,
    pressure: Pressure,
    a: &Arena<'a>,
) -> Result<Selected<'a>, SelectError> {
    let sizes = vattr_sizes(p, records)?;
    /* The VPM input rows (Mesa ntq_setup_vs_inputs, nir_to_vir.c:2402-2421,
     * and the attribute index, :2890-2922): the builtins the stage reads
     * first, one row each -- gl_InstanceID, then gl_VertexID (ES 3.00 has
     * no base instance) -- then each attribute record's
     * rows. GFXH-1602 (:2907-2915): when a builtin is read, attribute 0
     * must be active, so a coordinate shader that reads none of attribute
     * 0 has the one row the driver then loads for it (v3d_job.c, the
     * attribute records) and the later records start one row further. */
    let uses_iid = p.ops.contains(&SOp::InstanceId);
    let uses_vid = p.ops.contains(&SOp::VertexId);
    let builtins = u32::from(uses_iid) + u32::from(uses_vid);
    let gfxh1602_row = coord && builtins > 0 && sizes[0] == 0;
    let record_rows =
        |r: usize| -> u32 { (0..r).map(|k| if k == 0 && gfxh1602_row { 1 } else { sizes[k] as u32 }).sum() };
    let attr_rows = a.alloc_filled(p.ops.len(), 0u32)?;
    for (i, op) in p.ops.iter().enumerate() {
        match *op {
            SOp::Attr { loc, comp } => {
                let r = records.iter().position(|&l| l == loc).ok_or(SelectError::AttributeWithoutRecord)?;
                attr_rows[i] = builtins + record_rows(r) + comp as u32;
            }
            /* the only builtin read, so row 0 */
            /* the instance ID first, then the vertex ID (:2415-2421) */
            SOp::InstanceId => attr_rows[i] = 0,
            SOp::VertexId => attr_rows[i] = u32::from(uses_iid),
            _ => {}
        }
    }
    let mut s = Sel {
        ops: AVec::new(),
        next: 0,
        of: a.alloc_filled(p.ops.len(), Operand::None)?,
        tex_words: AVec::new(),
        pending: AVec::new(),
        ret32: 0,
        pipelined: pressure < Pressure::Serial,
        attr_rows: &mut [],
        loop_counters: AVec::new(),
        phi_saved: AVec::new(),
        in_phi_copies: false,
        fragment: false,
        deferred: deferred_loads(p, false, a)?,
        remat: pressure >= Pressure::Remat,
        last_push: None,
        labels: 0,
        ifs: AVec::new(),
        merges: AVec::new(),
        cached: AVec::new(),
    };
    if pressure >= Pressure::LazyAttributes {
        s.attr_rows = attr_rows;
    } else {
        for (i, op) in p.ops.iter().enumerate() {
            if let SOp::Attr { .. } | SOp::VertexId | SOp::InstanceId = *op {
                let idx = s.uniform(a, Uniform::constant(attr_rows[i]))?;
                s.of[i] = s.add(a, AddOp::LdvpmvIn, Operand::V(idx), Operand::None)?;
            }
        }
    }
    s.body(p, false, a)?;
    /* The shared-segment rule the lazy reads rely on: no output is written
     * yet, so no input row has been overwritten. */
    if s.ops.as_slice().iter().any(|o| matches!(o, VOp::Add { op: AddOp::Stvpmv, .. })) {
        return Err(SelectError::VpmWriteBeforeInputRead);
    }
    /* A vertex shader that switched threads (a texture look-up) ends its
     * last thread section before the VPM writes, as the fragment shader
     * does before the TLB ones (Sel::last_thrsw). Without one the stage
     * starts in its last section (single_seg, chain.rs). */
    s.last_thrsw(a, false)?;

    let mut pos = [Operand::None; 4];
    for (o, &x) in pos.iter_mut().zip(&p.position) {
        *o = s.val(a, x)?;
    }
    let store = |s: &mut Sel<'a>, row: u32, val: Operand| -> Result<(), SelectError> {
        let idx = s.uniform(a, Uniform::constant(row))?;
        s.push(a, VOp::Add { op: AddOp::Stvpmv, dst: Dst::None, a: Operand::V(idx), b: val })
    };
    /* v3d_nir_setup_vpm_layout_vs, 4.2 */
    /* A shader that writes gl_PointSize keeps it for every primitive
     * (prepare.rs, the IoSlot::PointSize arm), in Mesa's layout with
     * per_vertex_point_size, the point size after the fixed-
     * function rows (v3d_nir_lower_io.c:410-431): coordinate shader row 6,
     * vertex shader row 4, its varyings one row later. */
    let psiz_row = p.psiz.map(|_| if coord { 6 } else { 4 });
    let skip = u32::from(psiz_row.is_some());
    let (vp_row, rows) = if coord {
        for (i, &x) in pos.iter().enumerate() {
            store(&mut s, i as u32, x)?;
        }
        (4, 6 + skip)
    } else {
        (0, 4 + skip + p.varyings.len() as u32)
    };
    if rows > 64 {
        return Err(SelectError::VpmRowOutOfRange);
    }
    /* v3d_nir_emit_ff_vpm_outputs */
    let rcp_wc = s.add(a, AddOp::Recip, pos[3], Operand::None)?;
    for (i, contents) in [QUNIFORM_VIEWPORT_X_SCALE, QUNIFORM_VIEWPORT_Y_SCALE].into_iter().enumerate() {
        let scale = s.uniform(a, Uniform { contents, data: 0 })?;
        let t = s.mul(a, MulOp::Fmul, pos[i], Operand::V(scale))?;
        let t = s.mul(a, MulOp::Fmul, t, rcp_wc)?;
        let t = s.add(a, AddOp::Ffloor, t, Operand::None)?;
        let t = s.add(a, AddOp::Ftoiz, t, Operand::None)?;
        store(&mut s, vp_row + i as u32, t)?;
    }
    if !coord {
        let zscale = s.uniform(a, Uniform { contents: QUNIFORM_VIEWPORT_Z_SCALE, data: 0 })?;
        let z = s.mul(a, MulOp::Fmul, pos[2], Operand::V(zscale))?;
        let z = s.mul(a, MulOp::Fmul, z, rcp_wc)?;
        let zoff = s.uniform(a, Uniform { contents: QUNIFORM_VIEWPORT_Z_OFFSET, data: 0 })?;
        let z = s.add(a, AddOp::Fadd, z, Operand::V(zoff))?;
        store(&mut s, 2, z)?;
        store(&mut s, 3, rcp_wc)?;
        for (i, v) in p.varyings.iter().enumerate() {
            let val = match v {
                Some(x) => s.val(a, *x)?,
                /* Mesa stores 0 to a varying the FS reads and nothing wrote. */
                None => Operand::V(s.uniform(a, Uniform::constant(0))?),
            };
            store(&mut s, 4 + skip + i as u32, val)?;
        }
    }
    if let (Some(row), Some(x)) = (psiz_row, p.psiz) {
        let v = s.val(a, x)?;
        store(&mut s, row, v)?;
    }
    /* GFXH-1684: VPM writes complete before the end (4.2). */
    s.push(a, VOp::Add { op: AddOp::Vpmwt, dst: Dst::Nop, a: Operand::None, b: Operand::None })?;
    fold_unpacks(a, &mut s.ops, s.next)?;
    merge_exits(a, &mut s.ops)?;
    no_missing_operands(s.ops.as_slice())?;
    Ok(Selected {
        ops: s.ops.into_slice(),
        fragment: false,
        vregs: s.next,
        vattr_sizes: sizes,
        vpm_output_rows: rows,
        uses_vid,
        uses_iid,
        uses_center_w: false,
        implicit_point_varyings: false,
        writes_z: false,
        disable_ez: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;

    /// merge_exits, exhaustively: for both branch senses of each exit, each
    /// pushed bit (Z, N, C) and every value of flag A and of the second
    /// comparison's bit, the merged branch leaves exactly when one of the two
    /// did, under the flag update's rule (sim.rs apply_flags: AND is A & x,
    /// NOR is !(A | x), the N forms negate x). And the shape: one branch
    /// left, to the same label, after a compare that updates rather than
    /// pushes; the operations between are kept. Revert check: swapping the
    /// AND and NOR halves of the table fails the first assertion.
    #[test]
    fn merged_exits_leave_when_either_would() {
        let v = |n: u32| Operand::V(V(n));
        let fcmp = |pf: Pf| VOp::Flags { unit: FlagOp::Add(AddOp::Fcmp), a: v(0), b: v(1), pf, uf: Uf::None };
        let exit = |cond: BranchCond| VOp::FwdBranch { cond, msfign: Msfign::P, label: 7 };
        for c1 in [BranchCond::AnyA, BranchCond::AnyNA] {
            for c2 in [BranchCond::AnyA, BranchCond::AnyNA] {
                for pf in [Pf::PushZ, Pf::PushN, Pf::PushC] {
                    let between = VOp::Add { op: AddOp::Fadd, dst: Dst::V(V(2)), a: v(0), b: v(1) };
                    let ops = [fcmp(Pf::PushC), exit(c1), between, fcmp(pf), exit(c2), VOp::Label(7)];
                    let out: std::vec::Vec<VOp> = store::with_scratch(|a| {
                        let mut o = AVec::new();
                        o.extend_from_slice(a, &ops).unwrap();
                        merge_exits(a, &mut o).unwrap();
                        o.as_slice().to_vec()
                    })
                    .unwrap();
                    assert_eq!(out.len(), 5, "one branch fewer: {out:?}");
                    assert_eq!((out[1], out[3]), (between, exit(BranchCond::AnyNA)), "{out:?}");
                    let VOp::Flags { pf: Pf::None, uf, .. } = out[2] else { panic!("an update: {out:?}") };
                    let (and, neg) = match uf {
                        Uf::AndZ | Uf::AndN | Uf::AndC => (true, false),
                        Uf::AndNZ | Uf::AndNN | Uf::AndNC => (true, true),
                        Uf::NorZ | Uf::NorN | Uf::NorC => (false, false),
                        _ => (false, true),
                    };
                    let bit_ok = match pf {
                        Pf::PushZ => matches!(uf, Uf::AndZ | Uf::AndNZ | Uf::NorZ | Uf::NorNZ),
                        Pf::PushN => matches!(uf, Uf::AndN | Uf::AndNN | Uf::NorN | Uf::NorNN),
                        _ => matches!(uf, Uf::AndC | Uf::AndNC | Uf::NorC | Uf::NorNC),
                    };
                    assert!(bit_ok, "the update reads the bit the push would have: {pf:?} {uf:?}");
                    for a in [false, true] {
                        for x in [false, true] {
                            let e1 = if c1 == BranchCond::AnyA { a } else { !a };
                            let e2 = if c2 == BranchCond::AnyA { x } else { !x };
                            let xv = x != neg;
                            let a2 = if and { a && xv } else { !(a || xv) };
                            assert_eq!(!a2, e1 || e2, "{c1:?} {c2:?} {pf:?} {uf:?} A={a} x={x}");
                        }
                    }
                }
            }
        }
        /* not merged: a flag use between, or another label */
        let cm = VOp::CondMov { dst: V(2), src: v(0), cond: Cond::IfA };
        for between in [cm, VOp::Label(3)] {
            let ops = [fcmp(Pf::PushC), exit(BranchCond::AnyNA), between, fcmp(Pf::PushC), exit(BranchCond::AnyA)];
            let n = store::with_scratch(|a| {
                let mut o = AVec::new();
                o.extend_from_slice(a, &ops).unwrap();
                merge_exits(a, &mut o).unwrap();
                o.len()
            })
            .unwrap();
            assert_eq!(n, 5, "{between:?} between: nothing merged");
        }
    }

    /// fold_unpacks on hand-made operations, ABS: every input that takes the unpack
    /// reads `s.abs` and the move goes; a reader that cannot (FFLOOR) keeps
    /// it; and an FMAX whose two inputs would both become `s.abs` keeps its
    /// move, since FMAX of identical inputs has no encoding. That case is
    /// reachable from GLSL where CSE does not merge two abs() of one value
    /// (one inside a loop, one outside).
    #[test]
    fn fold_unpacks_moves_abs_into_readers_that_take_it() {
        let v = |n: u32| Operand::V(V(n));
        let mov =
            |d: u32, s: u32| VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(d)), a: Operand::Abs(V(s)), b: Operand::None };
        let add = |op: AddOp, d: u32, x: Operand, y: Operand| VOp::Add { op, dst: Dst::V(V(d)), a: x, b: y };
        store::with_scratch(|a| {
            let run = |prog: &[VOp]| -> Vec<VOp> {
                let mut ops = AVec::new();
                for &o in prog {
                    ops.push(a, o).unwrap();
                }
                fold_unpacks(a, &mut ops, 16).unwrap();
                ops.as_slice().to_vec()
            };
            let ld = VOp::Ldvary { dst: V(0) };
            /* FSUB and FMUL read .abs, the move goes */
            let got = run(&[
                ld,
                mov(1, 0),
                add(AddOp::Fsub, 2, Operand::SmallImm(0), v(1)),
                VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(3)), a: v(1), b: v(2) },
            ]);
            assert_eq!(
                got,
                [
                    ld,
                    add(AddOp::Fsub, 2, Operand::SmallImm(0), Operand::Abs(V(0))),
                    VOp::Mul { op: MulOp::Fmul, dst: Dst::V(V(3)), a: Operand::Abs(V(0)), b: v(2) },
                ]
            );
            /* FFLOOR has no ABS code: the move stays for it */
            let prog = [ld, mov(1, 0), add(AddOp::Ffloor, 2, v(1), Operand::None)];
            assert_eq!(run(&prog), prog);
            /* two moves of one register into one FMAX: both inputs would be
             * s.abs, so the FMAX keeps its reads and both moves stay */
            let prog = [ld, mov(1, 0), mov(2, 0), add(AddOp::Fmax, 3, v(1), v(2))];
            assert_eq!(run(&prog), prog);
            /* a source a CondMov writes after the move: the reader would see
             * the new value, so the move stays */
            let prog = [
                ld,
                mov(1, 0),
                VOp::CondMov { dst: V(0), src: Operand::SmallImm(0), cond: Cond::IfA },
                add(AddOp::Fsub, 2, Operand::SmallImm(0), v(1)),
            ];
            assert_eq!(run(&prog), prog);
            /* a select's result (a MOV and a CondMov, both before the move):
             * it holds the same value at the reader, so the read folds */
            let sel = [
                ld,
                VOp::Mul { op: MulOp::Mov, dst: Dst::V(V(4)), a: v(0), b: Operand::None },
                VOp::CondMov { dst: V(4), src: Operand::SmallImm(0), cond: Cond::IfA },
                mov(1, 4),
                add(AddOp::Fsub, 2, Operand::SmallImm(0), v(1)),
            ];
            assert_eq!(
                run(&sel),
                [sel[0], sel[1], sel[2], add(AddOp::Fsub, 2, Operand::SmallImm(0), Operand::Abs(V(4)))],
                "a select's result folds"
            );
        })
        .expect("scratch");
    }

    /// fold_unpacks on hand-made operations, L and H (a look-up lane's
    /// `fmov d, w.l`): the readers of encode.rs's table read `w.l` / `w.h`
    /// (FADD, FCMP as a flag push, VFPACK to the TLB, FFLOOR's input a,
    /// FMOV's one input) and the move goes; FTOIZ (refused on silicon), a
    /// MOV (no unpack field) and a CondMov keep it, for every reader.
    /// `abs(lane)` is the lane move read through ABS by the fabs move: the
    /// ABS folds into the FSUB, and the lane move stays, since its reader
    /// has an unpack of its own (no composing), and a read through an
    /// unpack is a read. Revert check: with the every-reader rule off, the
    /// FTOIZ + FSUB program's FSUB reads `w.l`.
    #[test]
    fn fold_unpacks_moves_l_and_h_into_readers_that_take_them() {
        let v = |n: u32| Operand::V(V(n));
        let lo =
            |d: u32, s: u32| VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(d)), a: Operand::Lo(V(s)), b: Operand::None };
        let hi =
            |d: u32, s: u32| VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(d)), a: Operand::Hi(V(s)), b: Operand::None };
        let add = |op: AddOp, d: u32, x: Operand, y: Operand| VOp::Add { op, dst: Dst::V(V(d)), a: x, b: y };
        store::with_scratch(|a| {
            let run = |prog: &[VOp]| -> Vec<VOp> {
                let mut ops = AVec::new();
                for &o in prog {
                    ops.push(a, o).unwrap();
                }
                fold_unpacks(a, &mut ops, 16).unwrap();
                ops.as_slice().to_vec()
            };
            let ld = VOp::Ldtmu { dst: V(0) };
            let (l0, h0) = (Operand::Lo(V(0)), Operand::Hi(V(0)));
            /* FADD, FCMP, VFPACK, FFLOOR and FMOV read w.l / w.h */
            let fcmp = |x: Operand, y: Operand| VOp::Flags {
                unit: FlagOp::Add(AddOp::Fcmp),
                a: x,
                b: y,
                pf: Pf::PushN,
                uf: Uf::None,
            };
            let pack = |x: Operand, y: Operand| VOp::Add { op: AddOp::Vfpack, dst: Dst::Tlb, a: x, b: y };
            let fmov = |d: u32, x: Operand| VOp::Mul { op: MulOp::Fmov, dst: Dst::V(V(d)), a: x, b: Operand::None };
            let got = run(&[
                ld,
                lo(1, 0),
                hi(2, 0),
                add(AddOp::Fadd, 3, v(1), Operand::SmallImm(0)),
                fcmp(v(2), Operand::SmallImm(0)),
                pack(v(1), v(2)),
                add(AddOp::Ffloor, 4, v(2), Operand::None),
                VOp::Flags { unit: FlagOp::Mul(MulOp::Fmov), a: v(1), b: Operand::None, pf: Pf::PushZ, uf: Uf::None },
            ]);
            assert_eq!(
                got,
                [
                    ld,
                    add(AddOp::Fadd, 3, l0, Operand::SmallImm(0)),
                    fcmp(h0, Operand::SmallImm(0)),
                    pack(l0, h0),
                    add(AddOp::Ffloor, 4, h0, Operand::None),
                    VOp::Flags { unit: FlagOp::Mul(MulOp::Fmov), a: l0, b: Operand::None, pf: Pf::PushZ, uf: Uf::None },
                ]
            );
            /* FTOIZ (L/H unmeasured on silicon), a MOV (to a TMU register:
             * no unpack field) and a CondMov keep the move */
            for reader in [
                add(AddOp::Ftoiz, 3, v(1), Operand::None),
                VOp::Mul { op: MulOp::Mov, dst: Dst::Magic(WADDR_TMUT), a: v(1), b: Operand::None },
                VOp::CondMov { dst: V(5), src: v(1), cond: Cond::IfA },
            ] {
                let prog = [ld, lo(1, 0), reader];
                assert_eq!(run(&prog), prog, "{reader:?} keeps the move");
            }
            /* one reader that cannot take it keeps the move for every
             * reader: the FSUB reads the move's register too */
            let prog = [ld, lo(1, 0), add(AddOp::Ftoiz, 3, v(1), Operand::None), add(AddOp::Fsub, 4, v(1), v(1))];
            assert_eq!(run(&prog), prog);
            /* an FADD whose two inputs would both become w.l keeps its
             * reads: FADD and FADDNF are told apart by input order
             * (encode.rs add_pack), and two identical inputs have one */
            let prog = [ld, lo(1, 0), add(AddOp::Fadd, 4, v(1), v(1))];
            assert_eq!(run(&prog), prog);
            /* abs(abs(x)) by hand: the outer move reads the inner through
             * ABS, and its ABS folds into the FSUB; the inner move is still
             * read (through an unpack), so it stays. Revert check: counting
             * only plain reads drops it. */
            let abs = |d: u32, s: u32| fmov(d, Operand::Abs(V(s)));
            assert_eq!(
                run(&[ld, abs(1, 0), abs(2, 1), add(AddOp::Fsub, 3, Operand::SmallImm(0), v(2))]),
                [ld, abs(1, 0), add(AddOp::Fsub, 3, Operand::SmallImm(0), Operand::Abs(V(1)))]
            );
            /* abs(lane): the ABS folds into the FSUB, the lane move stays */
            assert_eq!(
                run(&[ld, lo(1, 0), fmov(2, Operand::Abs(V(1))), add(AddOp::Fsub, 3, Operand::SmallImm(0), v(2))]),
                [ld, lo(1, 0), add(AddOp::Fsub, 3, Operand::SmallImm(0), Operand::Abs(V(1)))]
            );
        })
        .expect("scratch");
    }
}
