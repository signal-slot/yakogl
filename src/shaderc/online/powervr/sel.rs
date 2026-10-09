// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Derived from Mesa 26.2.1 src/imagination/pco/ (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! What selection hands the USC builder.
//!
//! Agreed between gles2-core (who produces these) and pvrgpu-platform (who
//! consumes them) before either half was written, because the GC7000's
//! equivalent seam cost a morning when each side widened its own end of it.
//!
//! The GC7000's equivalent is `vivante::alloc::SelInstr`, and the two are
//! deliberately not the same type. `docs/online-shaderc/usc-seam.md` fixes
//! that: a union would have to be wide enough for both machines, and every
//! field only one of them uses is a field the other's back end can read by
//! mistake.
//!
//! # Why this is group-level and carries no phases
//!
//! A USC instruction is a group: a header and up to one operation per
//! pipeline phase. `min` is four operations across three phases -- two `mbyp`
//! moving the sources into internal registers, a `tst`, a `movc`
//! ([`super::builtins`]). The obvious design is therefore to let selection
//! say which operation goes in which phase.
//!
//! That would be wrong, and the reason is worth stating because it is the
//! whole argument for this file's shape. Every internal register in that
//! group -- `ft0`, `ft1`, `ftt`, `is1..is4` -- is wired by the SHAPE, not by
//! selection. Which value feeds `src0` is selection's decision; that `src0`
//! reaches phase 2 through an `mbyp` into `ft0`, because phase 2 can only read
//! internal registers, is a fact about the machine. If [`SelGroup`] carried
//! phases, selection would be re-deriving what [`super::builtins::shape`]
//! already knows, and the first time the two disagreed the builder would emit
//! a well-formed group computing something else -- which is exactly the
//! failure the seam document refused the union type for.
//!
//! So selection says *what to compute*, and everything phase-shaped is
//! derived below this seam. A pleasant consequence: the two back ends' select
//! layers end up structurally alike, and the divergence lives entirely in the
//! machine half.
//!
//! # Three places this differs from the GC7000's, each for a measured reason
//!
//! **There is no `cond` field.** On the GC7000 `Cond` rides the instruction.
//! Here the comparison is `tst_op`, a field of the `tst` operation *inside*
//! the group, and the group's identity is already the [`AluOp`] -- `Flt`
//! means `tst_op = l`. A separate `cond` would be a second place to say one
//! thing. It would also invite selection to reach for one of the ten
//! `tst_op` values (`z gz gez c e g ge ne l le`) that the ~52-op kernel has
//! no operation for.
//!
//! **Sources carry a floor modifier as well as negate and absolute.** On this
//! machine all three are source modifiers (`s0flr`/`s1flr`/`s2flr`, beside
//! `s0neg` and `s0abs`; `docs/online-shaderc/isa.md`), and pco declares
//! `fneg`, `fabs` and `fflr` as pseudo operations that it folds into the
//! consumer. If `flr` were not expressible on the operand, the folded form
//! could not be represented at all and every `floor` would cost an `mbyp`.
//!
//! **A source can name the shared (uniform) register file directly.** The
//! source bank enum in the generated tables is `special | temp | vtxin |
//! coeff | shared | coeff_alt | idx0 | idx1`, so a uniform read costs no
//! instruction, as on the GC7000. Two narrower banks exist -- `regbank1 =
//! special | temp` and `regbank2 = special | temp | vtxin | coeff` -- so some
//! slots in some instruction forms cannot name `shared`, and the builder will
//! have to insert an `mbyp` when a shape puts a shared operand in such a
//! slot. That is the builder's problem by design: selection states the
//! operand and this side takes the consequences.
//!
//! # What is deliberately not here yet
//!
//! Write masks and repeat counts (`movwm` exists in the corpus, 1,312 times,
//! and the destination section has `w0p`/`w1p`), and the texture path
//! (`smp`, 5,488). They are absent because nothing in this file could say
//! what they mean yet without guessing, and a field that selection fills in
//! from a guess is worse than a field that is missing: the missing one
//! refuses to compile. They go in when the builder needs them and the tables
//! say what they encode.

use super::builtins::{shape, Shape};
use crate::shaderc::online::ir::AluOp;

/// A virtual register, before allocation. Numbered by selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VReg(pub u32);

/// The register files a source may name.
///
/// These are the names the generated `regbank` enum uses
/// (`isa_tables::ENUMS`), not a reinterpretation of them, so that a reader
/// can grep one against the other. `coeff_alt`, `idx0` and `idx1` exist in
/// the hardware enum and are absent here because nothing in the kernel
/// produces them yet; when indexed addressing arrives, `idx0`/`idx1` are
/// where it goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bank {
    /// Ordinary temporaries; what an allocated [`VReg`] becomes.
    Temp,
    /// The uniform/shared file. Readable as a source, so a uniform costs no
    /// instruction of its own.
    Shared,
    /// Vertex inputs (attributes).
    VtxIn,
    /// Iterator coefficients: how a fragment shader reads its varyings.
    Coeff,
    /// The special file: the constants and hardware values the ISA names.
    Special,
    /// Indexed access through index register 0. The register index is
    /// `idxbank | offset << 3` (Mesa `pco_map_reg_index`): `idxbank` 3 is
    /// the shared file, so index `3 | k << 3` reads `shared[idx0 + k]`;
    /// `idxbank` 5 names the index register itself, so writing index 5 sets
    /// it. See [`IDX_SELF`] and [`idx_shared`].
    Idx0,
}

/// The `Bank::Idx0` register index that names the index register itself.
pub const IDX_SELF: u16 = 5;

/// The `Bank::Idx0` register index that reads `shared[idx0 + offset]`.
pub const fn idx_shared(offset: u16) -> u16 {
    3 | offset << 3
}

impl Bank {
    /// The name the generated `regbank` enum gives this file.
    pub const fn isa_name(self) -> &'static str {
        match self {
            Bank::Temp => "temp",
            Bank::Shared => "shared",
            Bank::VtxIn => "vtxin",
            Bank::Coeff => "coeff",
            Bank::Special => "special",
            Bank::Idx0 => "idx0",
        }
    }

    /// The value the encoding carries for this file.
    ///
    /// NOT `self as u32`. The declaration order above is ours and the
    /// hardware's is `special temp vtxin coeff shared`, so the cast agrees
    /// for `VtxIn` and `Coeff` only. Until this existed the builder wrote the cast:
    /// every temporary destination was encoded as a special register and
    /// every uniform read as a temporary, and the "decode back" check passed
    /// because it translated the expected value with the same cast
    /// (usc-backend, 2026-09-23). Looked up by name in the generated table
    /// so that there is one source for the numbering, and
    /// `hw_values_are_the_isa_tables_values` holds the two together.
    pub fn hw(self) -> u32 {
        const BY_NAME: [(Bank, u32); 6] = [
            (Bank::Special, 0),
            (Bank::Temp, 1),
            (Bank::VtxIn, 2),
            (Bank::Coeff, 3),
            (Bank::Shared, 4),
            (Bank::Idx0, 6),
        ];
        let mut i = 0;
        while BY_NAME[i].0 != self {
            i += 1;
        }
        BY_NAME[i].1
    }

    /// The file a hardware bank value names, if it is one of ours.
    pub fn from_hw(v: u32) -> Option<Bank> {
        [Bank::Special, Bank::Temp, Bank::VtxIn, Bank::Coeff, Bank::Shared, Bank::Idx0]
            .into_iter()
            .find(|b| b.hw() == v)
    }
}

/// The operand type, as the ISA's `tst_type` field spells it.
///
/// Seven values in three bits, and NOT the GC7000's eight: there is no `F16`
/// here and the encodings differ, which is one concrete reason the two
/// machines' selection types are not one type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OpType {
    F32 = 0,
    U16 = 1,
    S16 = 2,
    U8 = 3,
    S8 = 4,
    U32 = 5,
    S32 = 6,
}

/// The per-source modifiers. All three are encoded on the source, not as
/// instructions, so folding them is free and NOT folding them costs an
/// `mbyp` each.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub neg: bool,
    pub abs: bool,
    /// `floor`. The GC7000 has a floor *instruction*; this machine has a
    /// modifier, which is why `Ffloor` is `Shape::Modifier` in
    /// [`super::builtins`].
    pub flr: bool,
}

/// One operand of a group.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelSrc {
    /// A value this compilation produced; the allocator places it.
    Virtual { v: VReg, mods: Mods },
    /// A register selection did not choose and the allocator must not move:
    /// an attribute, a varying's coefficient, a shared-file uniform.
    Fixed { bank: Bank, reg: u16, mods: Mods },
    /// A literal. The USC encodes these inside the instruction (the corpus's
    /// `bbyp0bm_imm32`, 33,013 of them), so an immediate is an operand rather
    /// than a constant-file entry the caller must first materialise.
    Imm(u32),
}

/// Where a group writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelDst {
    /// A temporary the allocator places.
    Virtual(VReg),
    /// A register the generator chose and the allocator must not move: a
    /// vertex output's slot, the fragment colour. Allocate *around* these.
    Fixed { bank: Bank, reg: u16 },
}

/// What a group computes.
///
/// Almost always a kernel [`AluOp`]. [`SelOp::Copy`] is the exception, and it
/// is here because the machine has an operation the kernel does not: the USC's
/// move is `mbyp`, which is not an `AluOp` and never will be -- the IR has no
/// copy operation because a copy is not a computation.
///
/// Selection needs one anyway. A store into a local slot whose value the
/// preceding groups just computed is retargeted and costs nothing, but a
/// genuine copy is left over, and over the corpus that is 101 of the 533
/// sources -- more than the texture path, the varyings, the dots and control
/// flow together (gles2-core, 2026-09-22).
///
/// The alternative selection offered was `Fadd x, 0.0`, and it is worth
/// recording why it was refused rather than just which way the ruling went:
/// it is correct for every float except `-0.0`, which it turns into `+0.0`.
/// A defect that is right on all but one input is the kind that reaches a
/// board and never fails there.
/// How a [`SelOp::Sample`] chooses its level: `smp`'s `lodm`, with `pplod`
/// when the data block carries a word for it (after the coordinates, as
/// pco_nir_tex.c pco_emit_nir_smp orders the block).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SmpLod {
    /// The hardware's own level, from the coordinates' derivatives
    /// (`lodm = normal`, no data word).
    Implicit,
    /// `textureLod`: the word replaces the level (`lodm = replace`, `pplod`).
    Replace,
    /// `texture(s, p, bias)`: the word is added to the implicit level
    /// (`lodm = bias`, `pplod`).
    Bias,
    /// The level from gradients the data block carries after the
    /// coordinates, d/dx then d/dy of each coordinate in turn (`lodm =
    /// gradients`, no `pplod`; pco_emit_nir_smp's order). The gradients come
    /// from the [`SelOp::SampleGrad`] groups right before the sample.
    Gradients,
}

impl SmpLod {
    /// Whether the data block carries one word for the level (`pplod`).
    pub fn in_block(self) -> bool {
        matches!(self, SmpLod::Replace | SmpLod::Bias)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelOp {
    /// A kernel operation. [`super::builtins::shape`] says how it is built.
    Alu(AluOp),
    /// Move `srcs[0]` to `dst`, bits unchanged -- `-0.0` and every NaN
    /// payload included. The builder emits `mbyp`, which is the operation it
    /// already emits for a [`Shape::Modifier`] no consumer would take, so
    /// nothing new appears on the machine side.
    Copy,
    /// A texture fetch, in two stages of the pipeline.
    ///
    /// As SELECTION emits it: sampler binding `binding`, the coordinate in
    /// `srcs[0..2]`, and the four results in the four consecutive virtual
    /// registers from `dst`. It never reaches the builder in that form: the
    /// chain expands it (`chain::expand_samples`) into moves filling the
    /// sample data block, one resolved `Sample`, a [`SelOp::WaitData`] and
    /// moves out of the result block.
    ///
    /// As the BUILDER takes it: `srcs` are the image descriptor (shared),
    /// the data block's first temporary and the sampler state (shared), and
    /// `dst` the result block's first temporary -- pco's `smp` operands.
    /// `proj`: `texture2DProj`, the hardware's projective form (`smp` with
    /// `proj`), whose selection form carries the divisor as `srcs[2]`.
    /// `lod`: how the level is chosen ([`SmpLod`]); a replaced level or a
    /// bias is carried as `srcs[2]`, never with `proj`.
    /// `fetch`: `texelFetch`, the coordinate in texels (`smp` with
    /// `nncoords`), always with `lod`.
    /// `offset`: `textureOffset` / `textureLodOffset`, the constant texel
    /// offset packed as the lookup word the data block ends with (`smp`
    /// with `soo`; u in bits 0..5, v in 6..11, rogue_hw_defs.h).
    /// `cube`: the TPU's own cube (`smp` `dmn` 3d, three coordinates, the
    /// descriptor a TEXTYPE_CUBE: `gl::native_cube`); the third coordinate
    /// comes from the [`SelOp::SampleZ`] right before it.
    Sample { binding: u8, proj: bool, lod: SmpLod, fetch: bool, offset: Option<u16>, cube: bool },
    /// The third coordinate of the cube [`SelOp::Sample`] that follows
    /// (`srcs[0]`): a group of its own so that liveness sees the read. The
    /// chain (`chain::expand_samples`) consumes it; it never reaches the
    /// builder.
    SampleZ,
    /// One coordinate's gradients for the [`SmpLod::Gradients`] sample that
    /// follows: d/dx in `srcs[0]`, d/dy in `srcs[1]`, one group per
    /// coordinate in coordinate order. Consumed by the chain
    /// (`chain::expand_samples`) like [`SelOp::SampleZ`].
    SampleGrad,
    /// Coefficient `term` (0 = A, the x slope; 1 = B, the y slope;
    /// `ROGUE_USC_COEFFICIENT_SET_*`) of a varying component's plane --
    /// `srcs[0]` is the component's input register -- or of the W plane when
    /// `srcs[0]` is `None`. Selection does not know where the coefficients
    /// sit; the chain (`chain::build_fragment_for`) turns it into a
    /// [`SelOp::Copy`] from the coefficient register.
    PlaneCoeff { term: u8 },
    /// Wait for data-return channel 0: `wdf`, after a sample.
    WaitData,
    /// Word `word` of sampler `binding`'s image state (`textureSize` reads
    /// its fields, as pco's usclib_tex_state_size does). Selection does not
    /// know where the descriptors sit; the chain (`chain::expand_samples`)
    /// turns it into a [`SelOp::Copy`] from that shared register, so it
    /// never reaches the builder.
    TexWord { binding: u8, word: u8 },
    /// A machine operation with no IR counterpart, which the fragment
    /// epilogue needs (`discard`'s ISP feedback, as pco builds it).
    Machine(MachineOp),
    /// The top of a loop kept as a loop: where its back edge lands. Emits
    /// nothing.
    LoopBegin,
    /// The back edge: `br if(p0)` to the matching [`SelOp::LoopBegin`].
    ///
    /// The only BACKWARD branch the builder emits (the other branches go
    /// forward past a branched `if`'s arm, [`SelOp::Pred`], where the
    /// execution mask decides). It must directly follow the
    /// [`MachineOp::LoopTest`] of the loop's own counter, which is the only
    /// thing that sets p0 for it: the counter is the same in every instance,
    /// so every instance takes the branch or none does. A data-dependent
    /// value never drives it -- divergent instances under a plain branch are
    /// what pco's `cnd*` execution-mask machinery exists for, and this path
    /// does not use it (`verify_loops` holds the rule).
    LoopEnd,
    /// An `if` arm's edge, as pco predicates an `if` (Mesa
    /// `src/imagination/pco/pco_cf.c`, `lower_if_pred_exec`, MIT, read and
    /// transcribed): after the `tstz.p0` of the condition
    /// ([`MachineOp::LoopTest`], p0 set where the condition is false), the
    /// groups up to [`PredArm::Else`] run `if(!p0)` (the then arm), those up
    /// to [`PredArm::End`] `if(p0)` (the else arm). Emits nothing. The arms'
    /// values are joined after `End` by the selects an if-conversion makes,
    /// on the same condition, so an arm that does not run leaves only
    /// values nothing reads; a group where the condition holds in no
    /// instance costs about nothing (gl_test 186/187 on the AM62P).
    ///
    /// The same markers delimit the arms of a BRANCHED `if` (pco_cf.c
    /// `lower_if_cond_exec`), told apart by the group before
    /// [`PredArm::Then`]: [`MachineOp::CndIf`] rather than the test. Its
    /// arms run under the execution mask and carry no condition; the
    /// assembler puts a `br` taken when no instance is active
    /// (`cf::branch_none_active`) before each arm that has groups, past it,
    /// and [`MachineOp::CndElse`] (right after `Else`) and
    /// [`MachineOp::CndIfEnd`] (right after `End`) switch the mask. The
    /// joins are the predicated form's: an instance the mask leaves out
    /// writes nothing, exactly as under a condition.
    Pred(PredArm),
}

/// Which edge of a predicated `if` a [`SelOp::Pred`] marks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PredArm {
    /// The then arm starts: its groups run where p0 is clear.
    Then,
    /// The else arm starts: its groups run where p0 is set.
    Else,
    /// Both arms are over: what follows runs everywhere.
    End,
}

/// The machine operations [`SelOp::Machine`] carries. Each is a template in
/// `shapes.rs`, transcribed from pco.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MachineOp {
    /// `ubfe(value, offset, bits)`: the unsigned bit field.
    Ubfe,
    /// `pck.cov(x)`: the coverage mask alpha-to-coverage derives from `x`.
    PckCov,
    /// `savmsk.vm`: the fragment's sample coverage mask. No operand.
    SavMsk,
    /// `sin`/`cos` as pco builds them (`trans_trig`): `fred` part A of `x`
    /// (into `w1`), part B of `x` and part A, `fsinc` of part B (which also
    /// sets p0), and `psel_trig` of the `fsinc` result and part B.
    FredA(Trig),
    FredB(Trig),
    Fsinc(Trig),
    PselTrig,
    /// `p0 = (x == 0)` and nothing else (`tstz.p0`, pco's own form of a
    /// predicate write): a loop's test, on `counter >= trip`, so p0 holds
    /// while the counter is below the trip count.
    LoopTest,
    /// `x >> n`, logical (`shift shr`).
    Ushr,
    /// `x == 0` as a boolean, all ones or zero (`tstz` into a register).
    Tstz,
    /// A move of a special-class register through S1 (`movs1`): the only
    /// way to read `x_p`, `y_p` and their sample twins.
    MovS1,
    /// `|x|` as a signed integer (`iadd32` with `s0abs`).
    Iabs,
    /// `clamp(x, 0.0, 1.0)`: `fadd(x, 0.0)` saturated (`fadd sat=1`), the
    /// one group pco makes of NIR's `fsat`, which nir_opt_algebraic forms
    /// from `fmin(fmax(x, 0.0), 1.0)` (prepare.rs [`fusion_pairs`]).
    ///
    /// [`fusion_pairs`]: super::prepare::fusion_pairs
    Fsat,
    /// A u32 to float, and a float to u32 rounding toward zero.
    U2f,
    F2u,
    /// `a >= b` as unsigned integers, a boolean.
    Uge,
    /// The high 32 bits of the unsigned 64-bit product (`imadd64`, w1).
    UmulHi,
    /// The execution mask (`powervr::cf`, pco's construction) around a loop
    /// whose instances may leave at different passes. Each reads the emc in
    /// `srcs[0]` and writes it back to `dst` -- `CndInit` reads special 0
    /// instead: it sets the emc up, every instance active (`cndst 1`).
    CndInit,
    /// A loop level entered (`cndst 2`).
    CndEnter,
    /// The instances whose p0 is clear -- the loop's test said they are
    /// done -- leave the loop's level (`cndsm p0_false`, by 2).
    CndLeave,
    /// The loop's end: p0 set while the instance is still in (`cndlt 2`),
    /// for the back edge.
    CndLoopEnd,
    /// A branched `if` entered (`cndst 1`): the instances whose p0 is
    /// clear -- the condition held, the then arm's -- stay active
    /// (`pcnd p0_false`).
    CndIf,
    /// The branched `if`'s other arm (`cndef 1`): the active instances and
    /// the inactive ones of this level change places.
    CndElse,
    /// The branched `if` left (`cndend 1`): every instance active again
    /// that was before it.
    CndIfEnd,
}

impl MachineOp {
    /// An execution-mask operation (`powervr::cf`): it reads the emc in
    /// `srcs[0]` (special 0 for [`MachineOp::CndInit`]), writes it back to
    /// `dst`, and runs in every instance whatever the mask.
    pub fn is_cnd(self) -> bool {
        matches!(
            self,
            MachineOp::CndInit
                | MachineOp::CndEnter
                | MachineOp::CndLeave
                | MachineOp::CndLoopEnd
                | MachineOp::CndIf
                | MachineOp::CndElse
                | MachineOp::CndIfEnd
        )
    }
}

/// Which function a `fred` reduces for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trig {
    Sin,
    Cos,
}

impl SelOp {
    /// How many operands this takes.
    pub fn arity(self) -> usize {
        match self {
            SelOp::Copy => 1,
            SelOp::Sample { .. } => 3,
            SelOp::WaitData | SelOp::LoopBegin | SelOp::LoopEnd | SelOp::TexWord { .. } | SelOp::Pred(_) => 0,
            SelOp::SampleZ | SelOp::PlaneCoeff { .. } => 1,
            SelOp::SampleGrad => 2,
            SelOp::Alu(op) => SelGroup::arity(op),
            SelOp::Machine(MachineOp::Ubfe) => 3,
            SelOp::Machine(MachineOp::PckCov | MachineOp::LoopTest | MachineOp::Tstz | MachineOp::MovS1) => 1,
            SelOp::Machine(MachineOp::Ushr | MachineOp::Uge | MachineOp::UmulHi) => 2,
            SelOp::Machine(MachineOp::Iabs | MachineOp::U2f | MachineOp::F2u | MachineOp::Fsat) => 1,
            SelOp::Machine(MachineOp::SavMsk) => 0,
            SelOp::Machine(MachineOp::FredA(_) | MachineOp::Fsinc(_)) => 1,
            SelOp::Machine(MachineOp::FredB(_) | MachineOp::PselTrig) => 2,
            /* the emc (special 0 for the set-up) */
            SelOp::Machine(
                MachineOp::CndInit
                | MachineOp::CndEnter
                | MachineOp::CndLeave
                | MachineOp::CndLoopEnd
                | MachineOp::CndIf
                | MachineOp::CndElse
                | MachineOp::CndIfEnd,
            ) => 1,
        }
    }

    /// May a group carrying this reach the builder?
    pub fn is_buildable(self) -> bool {
        match self {
            SelOp::Copy
            | SelOp::Sample { .. }
            | SelOp::WaitData
            | SelOp::Machine(_)
            | SelOp::LoopBegin
            | SelOp::LoopEnd
            | SelOp::Pred(_) => true,
            SelOp::Alu(op) => SelGroup::is_buildable(op),
            SelOp::TexWord { .. } | SelOp::SampleZ | SelOp::SampleGrad | SelOp::PlaneCoeff { .. } => false,
        }
    }
}

/// One instruction group, as selection describes it.
///
/// `op` is what the group computes; [`super::builtins::shape`] turns it into
/// the operations, phases and internal wiring. There is one of these per
/// kernel operation per component, with two exceptions on either side of the
/// seam: a [`Shape::Lower`] entry expands into several groups inside the
/// builder, and a [`Shape::SelectionExpands`] entry -- the dot products -- is
/// already gone before it gets here, because only selection knows the
/// components.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelGroup {
    pub op: SelOp,
    pub ty: OpType,
    pub dst: Option<SelDst>,
    /// The EXTERNAL operands, in the kernel operation's own argument order --
    /// not in hardware slot order. The GC7000's hardware slot is not the
    /// operand ordinal either, and conflating the two was a real defect
    /// there; here the question does not arise at this level, because the
    /// shape assigns slots.
    pub srcs: [Option<SelSrc>; 3],
}

impl SelGroup {
    /// How many operands the kernel operation takes, from the IR's own
    /// definition rather than from how many `Some`s a caller happened to fill
    /// in.
    pub fn arity(op: AluOp) -> usize {
        use AluOp::*;
        match op {
            Ffma | Bcsel => 3,
            Fneg | Fabs | Fsign | Frcp | Frsq | Fsqrt | Fexp2 | Flog2 | Fsin | Fcos | Ffloor | Fceil | Ftrunc
            | Ffract | Fddx | Fddy | Ineg | Bnot | Bany | Ball | B2f | B2i | F2i | I2f | F2b | I2b => 1,
            _ => 2,
        }
    }

    /// May a group carrying this operation reach the builder?
    ///
    /// Selection calls this before emitting, so a refusal names the operation
    /// rather than surfacing as a malformed group later. It is the same
    /// question [`shape`] answers; this wrapper exists so selection does not
    /// have to match on [`Shape`], which is the builder's vocabulary.
    ///
    /// Two shapes answer no, for opposite reasons. [`Shape::Refused`] is "the
    /// machine cannot". [`Shape::SelectionExpands`] is "selection already
    /// did": a dot product must never arrive as a group, because three source
    /// slots cannot carry a `Fdot3`'s six operands, so the seam refuses it
    /// here rather than letting a half-filled group reach the builder.
    pub fn is_buildable(op: AluOp) -> bool {
        !matches!(shape(op), Shape::Refused(_) | Shape::SelectionExpands(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bank named here must be a name the generated ISA table uses.
    ///
    /// The enum is a transcription of the hardware's `regbank` field, and a
    /// transcription that drifts from its source is the failure this file
    /// would not otherwise notice: a `Bank` the encoder cannot map produces a
    /// refusal at build time, but a `Bank` that maps to the WRONG hardware
    /// value produces a shader that reads the wrong register file.
    #[test]
    fn every_bank_is_a_name_the_isa_table_has() {
        let table = super::super::isa_tables::ENUMS
            .iter()
            .find(|e| e.name == "regbank")
            .expect("the generated tables have no `regbank` enum");
        for (bank, name) in [
            (Bank::Temp, "temp"),
            (Bank::Shared, "shared"),
            (Bank::VtxIn, "vtxin"),
            (Bank::Coeff, "coeff"),
            (Bank::Special, "special"),
        ] {
            assert!(
                table.elems.iter().any(|(_, n)| *n == name),
                "{bank:?} claims to be `{name}`, which `regbank` does not contain"
            );
        }
    }

    /// The value [`Bank::hw`] encodes is the value the generated `regbank`
    /// enum gives the same NAME. The name check above could not catch the
    /// defect this exists for: every name was right and three values were not.
    #[test]
    fn hw_values_are_the_isa_tables_values() {
        let table = super::super::isa_tables::ENUMS
            .iter()
            .find(|e| e.name == "regbank")
            .expect("the generated tables have no `regbank` enum");
        let all = [Bank::Temp, Bank::Shared, Bank::VtxIn, Bank::Coeff, Bank::Special];
        for bank in all {
            let (v, _) = table
                .elems
                .iter()
                .find(|(_, n)| *n == bank.isa_name())
                .unwrap_or_else(|| panic!("{bank:?}: no `{}` in regbank", bank.isa_name()));
            assert_eq!(bank.hw(), *v, "{bank:?} encodes {} but the table says `{}` is {v}", bank.hw(), bank.isa_name());
            assert_eq!(Bank::from_hw(*v), Some(bank));
        }
        /* The cast the builder used to write disagrees for three of the five
         * (Temp, Shared, Special); if it ever agreed for all five this test
         * would be vacuous. */
        assert_eq!(all.iter().filter(|b| **b as u32 != b.hw()).count(), 3);
    }

    /// Selection must be able to express every operation the machine half can
    /// build, and no more. Both directions matter: an operation `shape()`
    /// knows but `is_buildable` rejects is a capability thrown away, and the
    /// reverse is a group the builder cannot finish.
    #[test]
    fn buildable_agrees_with_the_machine_half() {
        for op in ALL {
            let expected = !matches!(shape(*op), Shape::Refused(_) | Shape::SelectionExpands(_));
            assert_eq!(SelGroup::is_buildable(*op), expected, "{op:?}");
        }
        // Six operations may not reach the builder; if this set changes the
        // test should be read again rather than the list updated.
        let refused: Vec<_> = ALL.iter().filter(|o| !SelGroup::is_buildable(**o)).collect();
        assert_eq!(
            refused,
            vec![&AluOp::Ftrunc, &AluOp::Fdot2, &AluOp::Fdot3, &AluOp::Fdot4, &AluOp::Idiv, &AluOp::Imod],
            "the refusal set changed"
        );
        // ... and every one of them for the same reason: selection already
        // expanded it (trunc to nir lower_ftrunc's select, the dots to
        // fmul/fmad, the divides to nir_lower_idiv's sequence). None is "the
        // machine cannot" any more.
        for op in [AluOp::Ftrunc, AluOp::Fdot2, AluOp::Fdot3, AluOp::Fdot4, AluOp::Idiv, AluOp::Imod] {
            assert!(matches!(shape(op), Shape::SelectionExpands(_)), "{op:?}");
        }
    }

    /// The copy is buildable, takes one operand, and is NOT any kernel
    /// operation.
    ///
    /// The last clause is the point. If `SelOp::Copy` could be confused with
    /// an `AluOp`, the builder would look its shape up in `builtins` and find
    /// something -- the failure this variant exists to prevent is exactly a
    /// copy being emitted as `Fadd x, 0.0`, which is right for every float
    /// but `-0.0`.
    #[test]
    fn the_copy_is_its_own_operation_and_takes_one_source() {
        assert!(SelOp::Copy.is_buildable());
        assert_eq!(SelOp::Copy.arity(), 1);
        for op in ALL {
            assert_ne!(SelOp::Copy, SelOp::Alu(*op), "the copy collides with {op:?}");
        }
    }

    /// `SelOp`'s two questions must agree with `SelGroup`'s for every kernel
    /// operation, so wrapping an `AluOp` never changes the answer.
    #[test]
    fn wrapping_an_alu_op_answers_the_same_as_the_bare_one() {
        for op in ALL {
            assert_eq!(SelOp::Alu(*op).is_buildable(), SelGroup::is_buildable(*op), "{op:?}");
            assert_eq!(SelOp::Alu(*op).arity(), SelGroup::arity(*op), "{op:?}");
        }
    }

    /// Arity must match the shapes: a one-operand kernel operation whose
    /// shape reads two sources would leave the second unset.
    #[test]
    fn arity_never_exceeds_the_operand_array() {
        for op in ALL {
            let n = SelGroup::arity(*op);
            assert!((1..=3).contains(&n), "{op:?} has arity {n}");
        }
        assert_eq!(SelGroup::arity(AluOp::Ffma), 3);
        assert_eq!(SelGroup::arity(AluOp::Bcsel), 3);
        assert_eq!(SelGroup::arity(AluOp::Fadd), 2);
        assert_eq!(SelGroup::arity(AluOp::Frcp), 1);
    }

    use AluOp::*;
    const ALL: &[AluOp] = &[
        Fadd, Fmul, Ffma, Fmin, Fmax, Fneg, Fabs, Fsign, Frcp, Frsq, Fsqrt, Fexp2, Flog2, Fsin, Fcos, Ffloor, Fceil,
        Ftrunc, Ffract, Fddx, Fddy, Fdot2, Fdot3, Fdot4, Flt, Fle, Feq, Fne, Iadd, Imul, Idiv, Imod, Ineg, Ishl, Ishr,
        Iand, Ior, Ixor, Ilt, Ile, Ieq, Ine, B2f, B2i, F2i, I2f, F2b, I2b, Bnot, Bany, Ball, Bcsel,
    ];
}
