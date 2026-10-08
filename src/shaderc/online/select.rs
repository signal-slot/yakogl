// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Instruction selection: `ir::Module` to GC7000 instructions in **virtual**
//! registers.
//!
//! This is the code generator's front half. It walks the IR that
//! `docs/online-shaderc/ir.md` defines and decides, per operation, which
//! single machine instruction implements it and which needs the lowering
//! library. What it emits is not yet an instruction: the registers are
//! virtual, because selection cannot know how many temporaries exist. Turning
//! those into `vivante::build::Src`/`Dst` is the allocator's job, and
//! [`SelInstr`] is the boundary.
//!
//! ```text
//!   ir::Module --(this file)--> [SelInstr] --(allocator)--> [Instr] --(abi)--> ShcShader
//!   front end                    selection    allocation            ABI
//! ```
//!
//! Two register kinds are **pre-coloured**, and an allocator must not move
//! them. That is why [`SelDst`] and [`SelSrc`] are not simply "a virtual
//! register and a swizzle":
//!
//! - **fragment inputs.** `gc7000-abi.md`: a fragment shader reads its
//!   varyings from registers `1..N`, `num_in` **is** the varying count, and
//!   the driver builds the `VS_OUTPUT` map from `fs.vs_reg_of_in[i]` in that
//!   order. Renumbering them yields a program that passes every check here and
//!   draws the wrong colours. `abi.rs` refuses a shader whose varyings are not
//!   `1..=N` in order, so this file assigns exactly that.
//! - **outputs.** The fragment colour register is reported as
//!   `ps_color_out_reg` and the vertex position as `vs_pos_out_reg`; the
//!   compiler chooses them and must then keep them.

use super::abi::{ConstSlot, IoReg};
use super::arena::{AVec, Arena, ArenaError};
use super::ir;
use super::vivante::alloc::{Cond, OpType, MAX_BRANCH_TARGET};
use super::vivante::build::{self, OpId};
use super::vivante::builtins;
use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/* The interface-A types are `vivante::alloc`'s, not this file's.
 *
 * Both of us defined them from the same specification and both were right;
 * two definitions of one seam is the defect. The allocator is the consumer,
 * so its definitions are the ones that exist and this file emits them. What
 * is re-exported here is a convenience for readers of the pipeline, not a
 * second copy. */
pub use super::vivante::alloc::{SelDst, SelInstr, SelSrc, VReg};

/// What selection produced for one stage.
pub struct Selected<'a> {
    pub instrs: &'a [SelInstr],
    /// The constant register file, one entry per dword, in register order.
    /// `abi::assemble` takes this unchanged.
    pub consts: &'a [ConstSlot],
    /// How many virtual registers were handed out; the allocator sizes its map
    /// from this.
    pub vregs: u32,
    /// The register the vertex position was written to.
    pub vs_pos_out_reg: Option<u8>,
    /// The register the fragment colour was written to.
    pub ps_color_out_reg: Option<u8>,
    /// The register `gl_PointSize` was written to, when the shader writes it.
    pub vs_pointsize_out_reg: Option<u8>,
    /// The input register `gl_VertexID` (`.x`) and `gl_InstanceID` (`.y`)
    /// arrive in, when the vertex shader reads either; see
    /// `Sel::vs_id_reg`. Not an io row: the driver programs it through
    /// `FE_HALTI5_ID_CONFIG`, as Mesa's etnaviv does.
    pub vs_id_in_reg: Option<u8>,
    /// The shader's inputs with the registers this file assigned them, ready
    /// for `abi::assemble`. Produced here rather than recomputed there, so
    /// that the numbering the instructions read from and the numbering the
    /// driver is told about cannot come from two different rules.
    pub io_in: &'a [IoReg<'a>],
    /// The shader's outputs, likewise.
    pub io_out: &'a [IoReg<'a>],
    /// The instructions that are a loop's BACK EDGE, by index.
    ///
    /// Every other branch this file emits goes forward, and `abi::assemble`
    /// refuses a backward one -- a branch that jumps back without a loop
    /// behind it is a program that never ends, and on this board a program
    /// that never ends wedges the GPU. A loop has one such branch at its end
    /// and one more per `continue`, and selection names each here rather than
    /// the validator having to guess which backward jump was meant.
    pub back_edges: &'a [usize],
    /// Every loop's guard, in the order the loops were selected: the most
    /// times its head is entered before the hidden counter leaves it
    /// (`loop_convert`). The recognized count plus one for a constant
    /// `for` loop, [`LOOP_CAP`] for any other.
    pub loop_limits: &'a [u32],
}

/// The most times a loop's head is entered when its trip count cannot be
/// read from the program: the PowerVR's and the V3D's `UNIFORM_LOOP_CAP`
/// (powervr/unroll.rs), the same number on every GPU. A DELIBERATE
/// DEVIATION from GLSL, which has no bound: a loop past it stops (its result
/// is then undefined) where GLSL would run on, because a program that does not
/// end stops the GPU until the driver resets it -- a lost frame, which on a
/// cluster is a safety event. gl_test 104's band 7 (a 1e9 bound read per
/// pixel) hung the i.MX8MP until a reset before this.
pub const LOOP_CAP: u32 = crate::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP;

pub use super::loop_count::EXACT_PASSES_MAX;

/// Why selection could not produce a program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectError {
    /// An IR construct this selection does not handle yet. The M3 subset is a
    /// passthrough vertex shader and a literal-colour fragment shader; this
    /// names what was met instead, so that a gap reads as a gap rather than as
    /// a miscompile.
    Unsupported(&'static str),
    /// A source naming a value no instruction produced.
    UndefinedValue(ir::ValueId),
    /// More io registers than the shader ABI holds.
    TooManyIo { got: usize, max: usize },
    /// The scratch arena ran out.
    OutOfScratch,
    /// The lowering library could not produce a sequence for an operation.
    /// Carried rather than flattened to a string so the message names the
    /// operation: "a builtin is missing" without saying which one sends the
    /// reader back to the shader to guess.
    Lower(builtins::LowerError),
}

impl From<builtins::LowerError> for SelectError {
    fn from(e: builtins::LowerError) -> SelectError {
        SelectError::Lower(e)
    }
}

/// The operand type an IR operation is performed in.
///
/// The GC7000 does not put the type in the opcode -- `add` is float or `s32`
/// depending on three type bits -- so this is a real decision and not a
/// formality. The IR carries no per-value type, but [`ir::AluOp`] names its
/// family in the opcode itself (`Fadd` against `Iadd`), which is the same
/// information; where it is not, this returns `None` and the compile stops
/// rather than picking a type that would change what the instruction computes.
fn op_type(op: ir::AluOp) -> Option<OpType> {
    use ir::AluOp::*;
    Some(match op {
        /* Float arithmetic, and the comparisons that read floats: their
         * operands are floats even though their result is a boolean. */
        Fadd | Fmul | Ffma | Fmin | Fmax | Fneg | Fabs | Fsign | Frcp | Frsq | Fsqrt | Fexp2 | Flog2 | Fsin | Fcos
        | Ffloor | Fceil | Ftrunc | Ffract | Fddx | Fddy | Fdot2 | Fdot3 | Fdot4 | Flt | Fle | Feq | Fne | F2b => {
            OpType::F32
        }
        /* A conversion is typed by its INTEGER side, whichever side that is:
         * the reference encodes every float-to-int as `f2i.s32` (40 of 44,
         * the other 4 `.u32`) exactly as it encodes `i2f.s32`. F2i was typed
         * by its float operand, and on silicon `int(u_in.x) == 2` came out
         * false with the loop and the constructors right (gl_test case 72 on
         * 511b9eb, imx8mp-board). */
        F2i => OpType::S32,
        /* Integer arithmetic, and the comparisons and conversions that read
         * integers. */
        Iadd | Imul | Idiv | Imod | Ineg | Ishl | Ishr | Iand | Ior | Ixor | Ilt | Ile | Ieq | Ine | I2f | I2b => {
            OpType::S32
        }
        /* Boolean operations. This used to refuse, because a bool is not a
         * machine type here and the corpus has them as both float 0/1 and
         * integer masks -- choosing wrongly makes `not` clear the wrong bits.
         *
         * MEASURED, 2026-09-22, by compiling the four shapes through the
         * reference compiler in one process and decoding what came back
         * (`tests/bool_probe.rs`). A bool is an **all-ones integer mask**:
         *
         *   float(u > 0.5)   cmp.gt r1.x       -> and.u32  r1, r1, #0x3f800
         *   !(u > 0.5)       cmp.gt r1.x       -> not.s32  r1.x, r1.x
         *   any(gt(v, ...))  cmp.gt r1.xy      -> or.s32   r1.x, r1.y, r1.x
         *   b ? a : c        cmp.gt r1.x       -> select.u32.nz r1, r1, r2, r2
         *
         * `and` with the top-20 bits of 1.0f (0x3f800) turning the mask into
         * 1.0 is only correct if the mask is all-ones, which is what fixes the
         * representation; `builtins.rs` had already chosen all-ones, as Mesa
         * does, so this measurement confirms that choice rather than making it.
         *
         * The types below are the ones the reference emitted, field by field:
         * U32 (TYPE_BIT2=1, TYPE_BIT01=2) for the mask-to-number `and` and --
         * note -- for `select` too, even where both its arms were float vec4s,
         * because selecting on a mask is a bitwise choice and not arithmetic;
         * S32 (TYPE_BIT01=1) for `not` and for the `or` of a reduction.
         *
         * That settles `Bcsel`, which the IR cannot type on its own: its two
         * arms may be float or int and the opcode does not say which. U32 is
         * right for both, so the ambiguity does not have to be resolved. */
        B2f | B2i | Bcsel => OpType::U32,
        Bnot | Bany | Ball => OpType::S32,
        /* GLSL ES 3.00 (ir.md, IR v1). Typed as Mesa's etnaviv table types
         * them (etnaviv_compiler_nir_emit.c, MIT): the signed ones S32
         * (IOPC(imin/imax), IOP(inot)), the unsigned ones U32 (UOP(ushr),
         * UOPC(ult32/uge32), UOPC(umin/umax), UOP(u2f32), UOP(f2u32)). On
         * this machine TYPE is the ONLY difference between `rshift.s32` and
         * `rshift.u32`, `cmp.lt.s32` and `cmp.lt.u32`, `i2f.s32` and
         * `i2f.u32`; so the type chosen here is the whole of the
         * signed/unsigned distinction, and getting it wrong is a picture
         * wrong only for values >= 2^31. None of the U32 forms is in the
         * reference corpus (ES 1.00 has no uint), so they are transcribed,
         * not measured.
         *
         * Udiv and Umod are exact 32-bit sequences, `builtins::udiv`
         * (nir_lower_idiv's emit_udiv), which types each of its own
         * instructions; U32 here only lets them past this table. */
        Imin | Imax | Inot => OpType::S32,
        Ushr | Ult | Ule | Umin | Umax | U2f | F2u | Udiv | Umod => OpType::U32,
    })
}

impl From<ArenaError> for SelectError {
    fn from(_: ArenaError) -> SelectError {
        SelectError::OutOfScratch
    }
}

/// The operand ordinal of a `mov`'s single source.
///
/// An **ordinal**, counted as a reader of the IR counts them, not a hardware
/// slot: `SelInstr::operand` places it, because which slot an operation reads
/// is an ISA fact and belongs where the ISA is. On this machine a `mov` reads
/// SRC2 and nothing else, and `add` reads SRC0 and SRC2 with a real gap
/// between them, so a generator placing its own operands would produce
/// instructions that encode perfectly and read registers nobody wrote.
const MOV_OPERAND: usize = 0;

/// Where the copies this file emits come from, for the program most recently
/// selected.
///
/// The allocator partitions the emitted `mov`s by what they MOVE
/// (coalescable, live-across, seed, io, `vivante::alloc::last_mov_breakdown`).
/// That says whose work each copy is but not which construct wrote it, and
/// the construct is visible only here. The two instruments answer the two
/// halves of the same question; neither can answer the other's.
///
/// Counted rather than argued because my last two guesses at "where do these
/// copies come from" were both wrong by measurement, on the same day.
#[derive(Clone, Copy, Default)]
pub struct MovSites {
    /// A store to a local slot that no preceding instruction could be
    /// retargeted at.
    pub store_local: usize,
    /// A load from a slot that `src` would not fold, because the slot is
    /// written again before the read.
    pub load_local: usize,
    /// `if_convert`'s seed: the then arm's register starting as a copy of the
    /// slot, when the arm can observe the slot's old value.
    pub if_seed: usize,
    /// A store to an output register.
    pub store_output: usize,
    /// A dynamically indexed uniform read.
    pub uniform_idx: usize,
    /// Slots renamed rather than copied through, and the two reasons a slot
    /// could not be: an arm touches it, or one of its stores has no register
    /// to name. See `slot_is_renamable`.
    pub renamed: usize,
    pub blocked_by_cf: usize,
    pub blocked_by_shape: usize,
    /// Constant seeds `fold_constant_reads` deleted, and why the others
    /// stayed: the instruction would have held more operands of the group
    /// than the reference ever does, the OPERATION never takes that group,
    /// a reader negates or takes the absolute of an immediate (whose
    /// modifier fields are payload), or the copy is not one this may move.
    ///
    /// A decline is not a failure -- it is the size of the next item, and
    /// without the split the leftovers are one number with no shape.
    pub folded: usize,
    /// Instructions `eliminate_dead` deleted. Here so the two passes' totals
    /// can be added up against the instruction count: a fold and a deletion
    /// of the same instruction are one instruction removed, not two, and
    /// without this the difference is unexplained.
    pub dead_deleted: usize,
    pub fold_over_cap: usize,
    pub fold_wrong_op: usize,
    pub fold_modifier: usize,
    pub fold_unsound: usize,
    /// Why `retarget_last_to` could not make the producing instruction write
    /// the slot itself, which is what turns a store into no instruction at
    /// all. Every one of these is a `mov` that stayed.
    pub retarget_swizzled: usize,
    pub retarget_not_a_value: usize,
    pub retarget_read_twice: usize,
    /// Declined because the store reads its value through an alias (a local
    /// slot's read resolved to the register that filled it), whose other
    /// readers `uses_of` cannot see.
    pub retarget_aliased: usize,
    pub retarget_is_a_slot: usize,
    pub retarget_not_last: usize,
    /// Operands `legalise_operands` had to materialise into a register
    /// because the operation never takes that register group in the
    /// reference. Each is one instruction added.
    pub legalised: usize,
    /// Of the swizzled refusals, how many a lane remap would rescue. See
    /// `lane_remap_would_do_it`.
    pub retarget_lane_remap: usize,
    /// Of the copies a LOAD needed, the two reasons the slot could not be
    /// renamed instead: an arm touches it, or one of its stores is masked.
    pub load_local_cf: usize,
    pub load_local_masked: usize,
    /// Of the control-flow ones, how many a select at the join could carry.
    pub load_local_joinable: usize,
    /// Loads that named the slot's current register instead of copying it.
    pub load_snapshot: usize,
    /// Copies whose slot has masked stores that ALL come from one value: the
    /// ones per-component tracking could remove, as opposed to the ones it
    /// merely touches. See `masked_stores_share_one_value`.
    pub load_one_value: usize,
    pub store_one_value: usize,
    /// Of the stores whose SOURCE is another slot's register: whether the
    /// destination slot is touched by an arm (so the join writes it and an
    /// alias is unsound) or not.
    pub slot_to_slot_under_cf: usize,
    pub slot_to_slot_plain: usize,
}

impl MovSites {
    const ZERO: Self = Self {
        store_local: 0,
        load_local: 0,
        if_seed: 0,
        store_output: 0,
        uniform_idx: 0,
        renamed: 0,
        blocked_by_cf: 0,
        blocked_by_shape: 0,
        folded: 0,
        dead_deleted: 0,
        fold_over_cap: 0,
        fold_wrong_op: 0,
        fold_modifier: 0,
        fold_unsound: 0,
        retarget_swizzled: 0,
        retarget_not_a_value: 0,
        retarget_read_twice: 0,
        retarget_aliased: 0,
        retarget_is_a_slot: 0,
        retarget_not_last: 0,
        legalised: 0,
        retarget_lane_remap: 0,
        load_local_cf: 0,
        load_local_masked: 0,
        load_local_joinable: 0,
        load_snapshot: 0,
        load_one_value: 0,
        store_one_value: 0,
        slot_to_slot_under_cf: 0,
        slot_to_slot_plain: 0,
    };
}

#[cfg(feature = "host-test")]
mod site_store {
    use super::MovSites;
    use core::cell::Cell;

    std::thread_local! {
        static SITES: Cell<MovSites> = const { Cell::new(MovSites::ZERO) };
    }

    pub(super) fn get() -> MovSites {
        SITES.with(|c| c.get())
    }

    pub(super) fn update(f: impl FnOnce(&mut MovSites)) {
        SITES.with(|c| {
            let mut v = c.get();
            f(&mut v);
            c.set(v);
        });
    }
}

#[cfg(not(feature = "host-test"))]
mod site_store {
    use super::MovSites;
    use crate::util::Global;

    static SITES: Global<MovSites> = Global::new(MovSites::ZERO);

    pub(super) fn get() -> MovSites {
        // SAFETY: single-threaded by contract.
        *unsafe { SITES.get() }
    }

    pub(super) fn update(f: impl FnOnce(&mut MovSites)) {
        // SAFETY: single-threaded by contract. Global::get() already hands
        // back a &mut T -- there is no separate get_mut on it.
        let v = unsafe { SITES.get() };
        f(v);
    }
}

/// The copy sites of the program most recently selected.
pub fn last_mov_sites() -> MovSites {
    site_store::get()
}

fn note_site(f: impl FnOnce(&mut MovSites)) {
    site_store::update(f);
}

/// Where the rasteriser leaves `gl_FragCoord` in the fragment stage.
///
/// Measured against the reference compiler; see `input_reg`. It is the reason
/// [`FS_COLOR_MIN_REG`] is 1 rather than 0.
const FS_FRAGCOORD_REG: u8 = 0;

/// The most io registers the shader ABI holds.
const MAX_IO: usize = crate::shaderc::SHC_VIV_MAX_IO;

/// The register a vertex shader writes `gl_Position` to.
///
/// Position first, then the varyings, mirroring the fragment side so both
/// stages number registers the same way. The driver reads it back as
/// `vs_pos_out_reg`, so any register would serve; choosing 0 consistently
/// means a dump of either stage reads alike.
const VS_POS_REG: u8 = 0;

/// The range `gl_PointSize` is clamped to: GL_ALIASED_POINT_SIZE_RANGE as
/// `gl/state.rs` reports it for the GC7000 (POINT_SIZE_MAX there); the two
/// must agree.
pub const POINT_SIZE_MIN: f32 = 1.0;
pub const POINT_SIZE_MAX: f32 = 64.0;

/// The lowest register a fragment shader may write its colour to.
///
/// **Never 0.** Measured over the offline table: of the 138 real fragment
/// blobs, **zero** write the colour to register 0 (the distribution is 1 in
/// 93, 2 in 39, 3 in 4, and one each at 7 and 8). The three that appeared to
/// use 0 in an earlier count are the all-zero placeholder rows, which have no
/// program in them at all.
///
/// That is the same fact as fragment inputs starting at register 1 in all 116
/// blobs that read a varying: t0 is reserved in this stage and is not the
/// compiler's to hand out. We wrote the colour there, told the driver to read
/// it from there through `ps_color_out_reg`, and the draw produced no visible
/// fragment on the board -- with no GL error, because every part of that is
/// internally consistent and only the hardware disagrees.
const FS_COLOR_MIN_REG: u8 = 1;

/// A source read through its first swizzle lane in all four positions: the
/// form a single-lane test is fed to the machine in (see `branch_convert`).
/// An immediate is left alone -- its "swizzle" bits are its payload.
fn splat_first_lane(s: SelSrc) -> SelSrc {
    let splat = |sw: [u8; 4]| [sw[0]; 4];
    match s {
        SelSrc::Virtual { v, swizzle, neg, abs } => SelSrc::Virtual { v, swizzle: splat(swizzle), neg, abs },
        SelSrc::Input { reg, swizzle, neg, abs } => SelSrc::Input { reg, swizzle: splat(swizzle), neg, abs },
        SelSrc::Physical(p) if !p.is_immediate() => SelSrc::Physical(build::Src { swizzle: splat(p.swizzle), ..p }),
        other => other,
    }
}

/// Push `i`, reading at most ONE constant-file register.
///
/// The reference reads two
/// different constant-file registers in one instruction 0 times in
/// 13,908 (two operands of ONE register, `u0.x` and `u0.y`, it does
/// all the time -- which is what tests/operand_banks.rs counted, and
/// why that count said nothing about this). We folded a uniform and a
/// compiler literal into one `mul` -- `mul t, u0.xxxx, u1.xxxx`, the
/// argument times 1/pi in front of every sin and cos -- and on silicon
/// sin(pi/6) read 0.314 (gl_test 77, imx8mp-board at 7902405) while
/// log2 and exp2, which read one, were right. Every constant register
/// after the first is copied whole into a temporary first, keeping its
/// addressing mode, and read from there through the same swizzle and
/// modifiers. The copies go in front of the instruction, so an index
/// a branch recorded stays the index of what it meant.
fn push_one_constant_register<'a>(
    instrs: &mut AVec<'a, SelInstr>,
    arena: &'a Arena<'a>,
    next: &core::cell::Cell<u32>,
    mov: OpId,
    mut i: SelInstr,
) -> Result<(), SelectError> {
    let mut first: Option<(u8, u16)> = None;
    for src in i.srcs.iter_mut().take(3) {
        let Some(SelSrc::Physical(p)) = *src else { continue };
        if p.group != build::RGROUP_UNIFORM_0 && p.group != build::RGROUP_UNIFORM_1 {
            continue;
        }
        let key = (p.group, p.reg);
        match first {
            None => first = Some(key),
            Some(k) if k == key => {}
            Some(_) => {
                let v = VReg(next.get());
                next.set(next.get() + 1);
                let whole = SelSrc::Physical(build::Src { swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false, ..p });
                instrs.push(
                    arena,
                    SelInstr::new(mov, OpType::F32)
                        .dst(SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL })
                        .operand(MOV_OPERAND, whole),
                )?;
                *src = Some(SelSrc::Virtual { v, swizzle: p.swizzle, neg: p.neg, abs: p.abs });
            }
        }
    }
    instrs.push(arena, i)?;
    Ok(())
}

struct Sel<'a> {
    m: &'a ir::Module<'a>,
    arena: &'a Arena<'a>,
    /// The `SHC_VIV_KEY_FRONT_FLIPPED` variant: `gl_FrontFacing` with the
    /// opposite sense (`front_facing`).
    front_flipped: bool,
    instrs: AVec<'a, SelInstr>,
    consts: AVec<'a, ConstSlot>,
    next_vreg: u32,
    /// Where the fragment colour ended up, once the store is seen.
    ///
    /// Decided while walking rather than beforehand, because the best answer
    /// depends on what is being stored: `gl_FragColor = v_col` wants the
    /// varying's own register, so that the copy disappears.
    ps_color_reg: Option<u8>,
    /// The register each colour output location was stored to (MRT): index
    /// is the location, 0 as `ps_color_reg`.
    colour_regs: [Option<u8>; MAX_COLOUR_OUTPUTS as usize],
    /// `mov`, resolved once rather than by name per instruction. The name is
    /// the key and the mnemonic is not unique (`branch_unary` and
    /// `branch_binary` both print `branch`), so an `OpId` cannot be ambiguous
    /// where a string can.
    mov: OpId,
    /// The register each local slot currently lives in.
    ///
    /// A slot is normally one fixed register, but `if_convert` gives an arm
    /// its OWN register for every slot it writes, so that the arm's stores do
    /// not have to be undone afterwards. That redirection is here rather than
    /// in a parameter because every read and every write of a slot goes
    /// through `local_reg`, and one of them missing the redirection is a
    /// shader that computes the wrong thing without failing.
    locals_in: &'a mut [VReg],
    /// The OPERAND a renamed slot currently holds, in walk order.
    ///
    /// `None` until the slot's first store. See `slot_is_renamable`: the
    /// value may have no register of its own (a uniform read, a literal), so
    /// this is a `SelSrc` and not a `VReg`.
    locals_val: &'a mut [Option<SelSrc>],
    /// What a `LoadLocal` READ, snapshotted when that load was walked.
    ///
    /// The slot's operand at the time of the LOAD, not at the time of the
    /// consumer: `c = a; d = c; c = b; use(d)` must see `a`, and `src` runs
    /// when the consumer is selected, by which point `locals_val` has moved
    /// on. A test pins exactly this (`a_local_read_before_it_is_written_again
    /// _is_copied_first`), and it caught the version of this change that
    /// looked the slot up at use time.
    value_alias: &'a mut [Option<SelSrc>],
    /// Where each loop's back edge ended up; see [`Selected::back_edges`].
    back_edges: AVec<'a, usize>,
    /// Branches emitted for a `break` and not yet given a destination.
    ///
    /// A `break` jumps to the end of the loop, which has not been emitted when
    /// the `break` is. Entries from the innermost loop's mark upwards belong
    /// to it; anything below belongs to a loop further out, which patches its
    /// own when it closes.
    breaks: AVec<'a, usize>,
    /// The first instruction of each enclosing loop, innermost last: where
    /// a `continue` jumps. Known when the `continue` is emitted, so it needs
    /// no patching.
    loop_tops: AVec<'a, usize>,
    /// See [`Selected::loop_limits`].
    loop_limits: AVec<'a, u32>,
}

impl<'a> Sel<'a> {
    /// The input register a slot arrives in.
    ///
    /// Fragment varyings are registers `1..=N` **in the order the shader
    /// declares them**, which is what the driver's `VS_OUTPUT` map is built
    /// against; vertex attributes take their location.
    fn input_reg(&self, slot: ir::IoSlot) -> Result<u8, SelectError> {
        match slot {
            ir::IoSlot::Attrib(loc) => Ok(loc),
            ir::IoSlot::Varying(loc) => {
                let idx = self
                    .used_varying_index(self.m.inputs, loc, true)
                    .ok_or(SelectError::Unsupported("a varying read that the module does not declare"))?;
                if idx + 1 > MAX_IO {
                    return Err(SelectError::TooManyIo { got: idx + 1, max: MAX_IO });
                }
                /* From 1, never 0. Measured over the offline table: of the 116
                 * fragment blobs that read any varying, all 116 have their
                 * lowest input register at 1. Register 0 is not ours to hand
                 * out in this stage. */
                Ok((idx + 1) as u8)
            }
            /* `gl_FragCoord` is temp register 0, all four components.
             *
             * MEASURED, 2026-09-22, against the reference compiler
             * (`tests/builtin_probe.rs`, since deleted -- the sequences are
             * here and in backend.md):
             *
             *   gl_FragCoord.xy * k   mul  t?, t0.xyxx, uniform
             *   gl_FragCoord.zw       mov  t?, t0.zwzz
             *   gl_FragCoord          mov  t?, t0.xyzw
             *
             * with `num_in 0` every time: it is NOT an io register and must
             * not join the io list, which is why this returns a register
             * rather than going through `used_varying_index`. `io_is_used`
             * cannot name it -- `ir::IoKind` has no variant for it -- so the
             * list is safe by construction rather than by care.
             *
             * This also explains a fact that was measured before it was
             * understood: `FS_COLOR_MIN_REG` is 1 because 0 of the 138 real
             * fragment blobs write their colour to register 0. They do not
             * because the rasteriser has already put gl_FragCoord there.
             *
             * `SelSrc::Input` is a reservation inside the temp file, so the
             * allocator counts t0 in `temps` and nothing aliases it. */
            ir::IoSlot::FragCoord => Ok(FS_FRAGCOORD_REG),

            /* Not selected, and each for its own reason rather than one
             * shared shrug -- the three arrive in completely different
             * places, so "a built-in input" is not a bucket anyone can work
             * on.
             *
             * `gl_FrontFacing` is an INTERNAL register (`RGROUP_INTERNAL`),
             * not a temp: the reference compares internal register 0 against
             * integer 0 with COND 6 to make the mask. That needs an operand
             * kind this file does not have yet, which is the work, and no
             * source in the corpus asks for it.
             *
             * `gl_PointCoord` is an ordinary fragment input row with no
             * vertex output behind it, as Mesa's etnaviv makes it
             * (etnaviv_compiler_nir.c, etna_link_shader: "point/tex coord is
             * an input to the PS without matching VS output, so it gets a
             * varying slot without being assigned a VS register"): the row
             * after every declared varying, two components, named
             * `gl_PointCoord` so the link leaves it unpaired and the driver
             * programs its components as POINTCOORD_X/Y. The reference
             * compiler (etna_shc) aborts on it, so the ABI is Mesa's, not the
             * table's. */
            ir::IoSlot::FrontFacing => Err(SelectError::Unsupported("gl_FrontFacing is not selected yet")),
            ir::IoSlot::PointCoord => self.pntc_reg(),
            ir::IoSlot::VertexId | ir::IoSlot::InstanceId if self.m.stage == SHC_STAGE_VERTEX => self.vs_id_reg(),
            _ => Err(SelectError::Unsupported("this built-in input is not selected yet")),
        }
    }

    /// Make the instruction that produced this value write `dst` itself.
    ///
    /// True when it did, in which case the caller emits no copy. Three things
    /// have to hold, and each of them is a way to get a wrong picture:
    ///
    /// * the value must be **the last instruction's whole result** -- a
    ///   partial write would leave the other lanes of the output unwritten;
    /// * the store may read it **through a swizzle**, and then the
    ///   permutation is pushed into the instruction: it computes in lane `i`
    ///   what it computed in lane `swizzle[i]`, by reading its own sources
    ///   through the same swizzle. Only for an operation whose lanes are
    ///   independent (`computes_lanewise`) -- a dot product sums across them,
    ///   so permuting its sources changes the sum rather than moving it -- and
    ///   never for a scalar operation, whose sources the builder splats from
    ///   lane 0, so its lanes already hold one value and composing would
    ///   change WHICH component that value came from;
    /// * it must have **no other use**, because after this its value lives in
    ///   the output register and anything else reading it would read the
    ///   register the output happens to occupy.
    fn retarget_last_to(&mut self, dst: SelDst, src: ir::Src, sel: &SelSrc) -> bool {
        /* `sel` is what `src` resolved to -- including `src.swizzle`, which is
         * why this does not look at it separately -- and only a virtual
         * register can be
         * retargeted, and it must be the one the last instruction wrote. */
        let SelSrc::Virtual { v, swizzle, neg, abs } = *sel else {
            note_site(|x| x.retarget_not_a_value += 1);
            return false;
        };
        if neg || abs {
            note_site(|x| x.retarget_swizzled += 1);
            return false;
        }
        if self.uses_of(src.val) != 1 {
            note_site(|x| x.retarget_read_twice += 1);
            return false;
        }
        /* `uses_of` counts the uses of THIS IR value. A read of a local slot
         * is its own value, resolved (value_alias) to the register of the
         * value that filled the slot -- and every other read of the slot
         * resolves to that register too, invisible here. Qt Quick 3D's
         * motionvector shader stores one slot to two outputs
         * (`vPosition = _41; gl_Position = _41;`): the first store retargeted
         * the mad that computed _41 into vPosition's register, and the second
         * then read a register nothing wrote -- gl_Position was the partial
         * sum (GC7000 online, found by frontend2's ES 3.00 step 1). Only a
         * store that reads its own value's register may take the defining
         * instruction. */
        if v.0 != src.val.0 {
            note_site(|x| x.retarget_aliased += 1);
            return false;
        }
        /* Only a VALUE register may be retargeted, never a local slot's.
         * `uses_of` counts the uses of the IR value; a slot's register has
         * readers of its own that it cannot see, so retargeting the store that
         * fills a slot would leave every later read of that slot reading a
         * register the output now occupies. Values live below
         * `instrs.len()`, slots above it. */
        if v.0 >= self.m.instrs.len() as u32 {
            note_site(|x| x.retarget_is_a_slot += 1);
            /* Could a RENAME take it instead? The value is one slot's
             * register being stored into another, which retargeting can never
             * remove -- it would redirect the instruction that filled the
             * first slot. An alias could, but only where nothing else writes
             * the destination, and a slot an arm touches is written by the
             * join. Counted apart so the item is not read as "another 149 the
             * retarget could have taken". */
            if let SelDst::Virtual { v: dst_reg, .. } = dst {
                let slot = dst_reg.0.checked_sub(self.m.instrs.len() as u32);
                if let Some(l) = slot {
                    if self.slot_under_control_flow(ir::LocalId(l)) {
                        note_site(|x| x.slot_to_slot_under_cf += 1);
                    } else {
                        note_site(|x| x.slot_to_slot_plain += 1);
                    }
                }
            }
            return false;
        }
        let Some(last) = self.instrs.len().checked_sub(1) else { return false };
        match self.instrs[last].dst {
            Some(SelDst::Virtual { v: wrote, write_mask }) if wrote == v && write_mask == build::WRITE_MASK_ALL => {}
            _ => {
                note_site(|x| x.retarget_not_last += 1);
                return false;
            }
        }
        /* The store reads the value through a swizzle, so the instruction has
         * to compute in lane `i` what it computed in lane `swizzle[i]`. See
         * `lane_remap_would_do_it` for why that is only sound for an
         * operation whose lanes are independent. */
        if swizzle != ir::SWIZZLE_IDENTITY {
            let p = self.instrs[last];
            if p.tex.is_some() || p.target.is_some() || !p.op.computes_lanewise() {
                note_site(|x| x.retarget_swizzled += 1);
                return false;
            }
            /* A SCALAR operation's sources are splatted from lane 0 by the
             * builder, so all four of its destination lanes already hold the
             * same number and any permutation of them is the identity.
             * Composing its swizzles would NOT move the value, it would
             * change which component it is computed from -- `rcp(x.x)` would
             * become `rcp(x.z)`. So the remap for these is to do nothing. */
            if !p.op.reads_one_component() {
                for slot in self.instrs[last].srcs.iter_mut() {
                    if let Some(sr) = *slot {
                        *slot = Some(compose(sr, swizzle));
                    }
                }
            }
            note_site(|x| x.retarget_lane_remap += 1);
        }
        self.instrs[last].dst = Some(dst);
        true
    }

    /// How many operands in the whole body read this value: every source of
    /// every instruction (`Op::for_each_src`) and every `if`'s condition.
    ///
    /// The condition was not counted, and dEQP's
    /// operator.sequence.side_effects.affect_and showed what that costs:
    /// `u_false == sideEffectA` is stored to the `&&`'s temporary AND is the
    /// `if`'s condition, the store was counted as its only use, the compare
    /// was retargeted into the slot's register, and the if-converted arm's
    /// `select` read the value's own register, which nothing wrote. On the
    /// host it reads 0 and the picture is right; on the i.MX8MP it held the
    /// previous thread's value and funcB's store took effect (2026-09-24).
    /// A StoreLocal's array index was not counted either.
    fn uses_of(&self, val: ir::ValueId) -> usize {
        let mut n = 0usize;
        for op in self.m.instrs.iter() {
            op.for_each_src(|s| n += usize::from(s.val == val));
        }
        for node in self.m.nodes.iter() {
            if let ir::Node::If { cond, .. } = node {
                n += usize::from(cond.val == val);
            }
        }
        n
    }

    /// Is this source already sitting in the destination register, whole?
    ///
    /// Only an io register can be: a virtual register is the allocator's to
    /// place and is not known to be anywhere yet, and a constant-file read is
    /// in a different file entirely. The swizzle must be the identity -- a
    /// `.yzwx` of a register onto itself is a real permutation and needs the
    /// instruction -- and neither modifier may be set, since negate and
    /// absolute change the value even when the register does not move.
    fn is_already_in_place(&self, dst: u8, src: &SelSrc) -> bool {
        match *src {
            SelSrc::Input { reg, swizzle, neg, abs } => reg == dst && swizzle == ir::SWIZZLE_IDENTITY && !neg && !abs,
            _ => false,
        }
    }

    /// Is this the module's only store to `slot`?
    ///
    /// What makes leaving a value in place (no instruction at all) sound: the
    /// output register then holds the input to the end only because nothing
    /// else writes it. A second store to the same output -- before it, after
    /// it, or in an `if` arm -- is an instruction writing that register, which
    /// makes the output one the allocator places like a value, and nothing
    /// then keeps the input's register intact to the end. `d = f; d = e;` with
    /// `e` and `d` both in t1 compiled to `mov t1, t2` alone, and
    /// `gl_FragColor = v; if (c) gl_FragColor = k;` put the compare into the
    /// varying's register (dEQP fragment_ops.interaction.basic_shader.16, 61,
    /// 63, 84, i.MX8MP, 2362422d).
    fn single_store(&self, slot: ir::IoSlot) -> bool {
        self.m.instrs.iter().filter(|op| matches!(op, ir::Op::StoreOutput { slot: s, .. } if *s == slot)).count() == 1
    }

    /// Does any instruction actually touch this declared io?
    ///
    /// The front end declares everything the source does and numbers it; the
    /// code generator decides the SET, which is es100-frontend's correction
    /// and the right split -- an attribute or varying nobody reads gets no
    /// register, and 27 of them are declared and eliminated across the GC7000
    /// table.
    fn io_is_used(&self, kind: ir::IoKind, location: u8, input: bool) -> bool {
        let want = match kind {
            ir::IoKind::Attribute => ir::IoSlot::Attrib(location),
            ir::IoKind::Varying => ir::IoSlot::Varying(location),
            ir::IoKind::Position => ir::IoSlot::Position,
            ir::IoKind::PointSize => ir::IoSlot::PointSize,
            ir::IoKind::Colour => ir::IoSlot::FragColor(0),
            ir::IoKind::Depth => ir::IoSlot::FragDepth,
        };
        self.m.instrs.iter().any(|op| match op {
            ir::Op::LoadInput { slot, .. } if input => *slot == want,
            ir::Op::StoreOutput { slot, .. } if !input => *slot == want,
            _ => false,
        })
    }

    /// Where a varying sits among the USED varyings of its own list.
    ///
    /// **The one place that decides this.** Registers run `1..=N` with no
    /// gaps -- `abi.rs` refuses a fragment shader whose varyings do not --
    /// so dropping an unused one renumbers every varying after it. If the
    /// list handed to the driver and the numbering the instructions read from
    /// came from two different walks, they would disagree silently and the
    /// picture would be built from the wrong registers. Both callers ask
    /// here.
    ///
    /// Safe because `link::by_register` pairs the two stages **by name**: the
    /// vertex and fragment shaders are compiled separately and may drop
    /// different varyings, and the link still matches what is left.
    ///
    /// A MATRIX varying is one declaration over one location per column
    /// (`varying mat2 m` is stored and loaded at `Varying(L)` and
    /// `Varying(L + 1)`), so it takes one register per column and every
    /// varying after it moves up by its extra columns. Counting declarations
    /// instead refused every matrix varying ("a varying written that the
    /// module does not declare" -- dEQP's matrix, conversions, functions and
    /// linkage groups, about 250 case-sides).
    /// The fragment input register `gl_PointCoord` arrives in: the one after
    /// every varying row the shader reads (see `input_reg`).
    fn pntc_reg(&self) -> Result<u8, SelectError> {
        let rows: usize = self
            .m
            .inputs
            .iter()
            .filter(|d| d.kind == ir::IoKind::Varying && self.decl_is_used(d, true))
            .map(|d| usize::from(varying_columns(self.m.inputs, d)))
            .sum();
        if rows + 1 > MAX_IO {
            return Err(SelectError::TooManyIo { got: rows + 1, max: MAX_IO });
        }
        Ok((rows + 1) as u8)
    }

    /// The vertex input register `gl_VertexID` and `gl_InstanceID` arrive
    /// in: the one after every attribute register, the vertex id in `.x` and
    /// the instance id in `.y`, both as integers.
    ///
    /// This is Mesa's etnaviv (MIT; etnaviv_nir.c sets `vs_id_in_reg =
    /// infile.num_reg`, and etnaviv_compiler_nir_ra.c pins load_vertex_id to
    /// `num_reg`.x and load_instance_id to `num_reg`.y), and it is what the
    /// reference compiler emitted for gl_test's `es300/vid.vert`:
    /// `vs_id_in_reg` 1 after the attribute in register 0, read as
    /// `iabs.s32 t1._y__, void, void, t1.xxxx`. The driver feeds it as one
    /// more vertex input and sets `FE_HALTI5_ID_CONFIG` to `reg * 4` and
    /// `reg * 4 + 1` (`vivante/shader.rs`, from etnaviv_shader.c).
    ///
    /// Attribute registers are their locations here, not a compacted count,
    /// so "after every attribute" is one past the highest location a used
    /// attribute (every column of a matrix one) occupies. A register below
    /// that may be one a vertex element writes.
    fn vs_id_reg(&self) -> Result<u8, SelectError> {
        let mut end = 0usize;
        let mut rows = 0usize;
        for d in self.m.inputs.iter().filter(|d| d.kind == ir::IoKind::Attribute && self.decl_is_used(d, true)) {
            let cols = usize::from(varying_columns(self.m.inputs, d));
            end = end.max(usize::from(d.location) + cols);
            rows += cols;
        }
        /* The ids are one more vertex input (vivante/shader.rs appends it
         * after the elements), so they and the attributes share the 16. */
        let max = crate::vivante::state::GC_MAX_VERTEX_ELEMENTS;
        if rows + 1 > max {
            return Err(SelectError::TooManyIo { got: rows + 1, max });
        }
        Ok(end as u8)
    }

    fn used_varying_index(&self, decls: &[ir::IoDecl<'a>], location: u8, input: bool) -> Option<usize> {
        let mut offset = 0usize;
        for d in decls.iter().filter(|d| d.kind == ir::IoKind::Varying && self.decl_is_used(d, input)) {
            let cols = varying_columns(decls, d);
            if (d.location..d.location.saturating_add(cols)).contains(&location) {
                return Some(offset + usize::from(location - d.location));
            }
            offset += usize::from(cols);
        }
        None
    }

    /// Is any location of this declaration read (an input) or written (an
    /// output)? A matrix varying or attribute is used if any of its columns
    /// is: the front end reads column j of a matrix attribute as
    /// `LoadInput Attrib(location + j)`, and an attribute checked only at its
    /// first location dropped out of the program whenever the shader read
    /// just a later column -- dEQP default_vertex_attrib.mat2/3/4.* read
    /// `a_value[1]` alone, glGetAttribLocation said -1 and dEQP fed index 0
    /// (16 cases on the i.MX8MP, found by frontend3).
    fn decl_is_used(&self, d: &ir::IoDecl<'a>, input: bool) -> bool {
        if d.kind != ir::IoKind::Varying && d.kind != ir::IoKind::Attribute {
            return self.io_is_used(d.kind, d.location, input);
        }
        let decls = if input { self.m.inputs } else { self.m.outputs };
        (0..varying_columns(decls, d)).any(|k| self.io_is_used(d.kind, d.location.saturating_add(k), input))
    }

    /// The one store to this slot, when it has exactly one and that store
    /// copies a value that is already in a register.
    ///
    /// `float c = a; ... use(c)` puts `a` in a slot and reads it back. The
    /// slot is a register, so that is a `mov` and a register of its own --
    /// for a value that was already sitting in one. Propagating the source to
    /// every read removes both.
    ///
    /// Three conditions, and each is a way to be wrong without it:
    ///
    /// * **exactly one store**, or a read might belong to a different one;
    /// * **the whole register**, since a masked store leaves the other lanes
    ///   holding whatever the slot had, which is not the source's;
    /// * the source is an attribute, a uniform read or a literal -- something
    ///   `src` folds into an operand anyway. A computed value is not
    ///   propagated here: the instruction that computed it can write the slot
    ///   itself, which `retarget_last_to` already does, and chasing a chain of
    ///   slots would recurse as deep as the shader is long.
    fn propagable_store(&self, local: ir::LocalId) -> Option<(u32, ir::Src)> {
        /* Index order is one path through the program only -- see
         * `local_is_stable_after`. That is fatal for a slot an arm touches,
         * and irrelevant for one no arm touches. */
        if self.slot_under_control_flow(local) {
            return None;
        }
        let mut found = None;
        for (i, op) in self.m.instrs.iter().enumerate() {
            if let ir::Op::StoreLocal { local: l, index: None, src, write_mask } = op {
                if *l != local {
                    continue;
                }
                if found.is_some() || *write_mask != build::WRITE_MASK_ALL {
                    return None;
                }
                let simple = matches!(
                    self.m.def(src.val),
                    Some(
                        ir::Op::LoadInput { .. }
                            | ir::Op::ConstF32(_)
                            | ir::Op::ConstI32(_)
                            | ir::Op::ConstBool(_)
                            | ir::Op::LoadUniform { .. }
                    )
                );
                if !simple {
                    return None;
                }
                found = Some((i as u32, *src));
            } else if matches!(op, ir::Op::StoreLocal { local: l, .. } if *l == local) {
                /* An indexed store to the same slot: not modelled. */
                return None;
            }
        }
        found
    }

    /// The virtual register a local slot lives in.
    ///
    /// Reserved before selection walks the body, so every slot has a number
    /// before the first instruction asks for one and a lowering's scratch
    /// (which counts up from after them) cannot collide.
    fn local_reg(&self, local: ir::LocalId) -> VReg {
        self.locals_in[local.0 as usize]
    }

    /// The register a slot lives in when nothing has redirected it.
    fn local_home(m: &ir::Module<'_>, local: u32) -> VReg {
        VReg(m.instrs.len() as u32 + local)
    }

    /// Does any arm of any `if` or loop read or write this slot?
    ///
    /// The precise form of the question the two guards above used to ask as
    /// "does this shader contain an `if`". Both of them reason by comparing
    /// positions in the flat instruction array, and that is sound for a slot
    /// exactly when index order is execution order FOR THE INSTRUCTIONS THAT
    /// TOUCH IT.
    ///
    /// **`instrs` is a pool in allocation order, not execution order**
    /// (es100-frontend, 2026-09-22: an arm is committed before the statement
    /// that contains it, so its instructions hold lower indices than code
    /// that runs before them). That is exactly why the question has to be
    /// "does an ARM touch this slot" and not "is there an arm": if no arm
    /// touches it, every instruction that does is a top-level one, top-level
    /// statements are committed in order, and index order is execution order
    /// among them. If an arm touches it, neither guard can reason about it at
    /// all.
    fn slot_under_control_flow(&self, local: ir::LocalId) -> bool {
        fn touches(m: &ir::Module<'_>, r: ir::Range, local: ir::LocalId, nested: bool) -> bool {
            m.nodes_of(r).iter().any(|n| match *n {
                ir::Node::Instr(id) => {
                    nested
                        && matches!(
                            m.instr(id),
                            Some(ir::Op::LoadLocal { local: l, .. } | ir::Op::StoreLocal { local: l, .. }) if *l == local
                        )
                }
                ir::Node::If { then_, else_, .. } => {
                    touches(m, then_, local, true) || touches(m, else_, local, true)
                }
                ir::Node::Loop { body } => touches(m, body, local, true),
                _ => false,
            })
        }
        touches(self.m, self.m.body, local, false)
    }

    /// Does a LOOP touch this slot?
    ///
    /// Narrower than `slot_under_control_flow` and asked separately, because
    /// a loop breaks a different rule. An `if`'s arms run once and in the
    /// order the nodes are written; a loop's body runs again, so a store
    /// EARLIER in the body than a read still overwrites what that read saw,
    /// on the next iteration. Program order does not answer that question and
    /// nothing short of real liveness does, so a slot a loop touches is left
    /// alone.
    fn slot_under_loop(&self, local: ir::LocalId) -> bool {
        fn go(m: &ir::Module<'_>, r: ir::Range, local: ir::LocalId, in_loop: bool) -> bool {
            m.nodes_of(r).iter().any(|n| match *n {
                ir::Node::Instr(id) => {
                    in_loop
                        && matches!(
                            m.instr(id),
                            Some(ir::Op::LoadLocal { local: l, .. } | ir::Op::StoreLocal { local: l, .. }) if *l == local
                        )
                }
                ir::Node::If { then_, else_, .. } => {
                    go(m, then_, local, in_loop) || go(m, else_, local, in_loop)
                }
                ir::Node::Loop { body } => go(m, body, local, true),
                _ => false,
            })
        }
        go(self.m, self.m.body, local, false)
    }

    /// Is this slot stored anywhere AFTER this load, in PROGRAM order?
    ///
    /// Over the NODES, not over the instruction array. `instrs` is a pool in
    /// allocation order (es100-frontend's warning, and it is load-bearing
    /// here): an arm's store may sit at a lower index than a load that runs
    /// before it, and an index comparison would then call a slot stable while
    /// a later store is waiting inside an `if`. The node walk asks the
    /// question the name asks.
    ///
    /// A store in EITHER arm of an `if` that follows the load counts, without
    /// asking which arm runs: the slot's register may be written, and that is
    /// enough to make the answer "no".
    fn stored_after(&self, local: ir::LocalId, load: ir::InstrId) -> bool {
        fn go(m: &ir::Module<'_>, r: ir::Range, local: ir::LocalId, load: ir::InstrId, seen: &mut bool) -> bool {
            for n in m.nodes_of(r) {
                match *n {
                    ir::Node::Instr(id) => {
                        if id == load {
                            *seen = true;
                            continue;
                        }
                        if *seen && matches!(m.instr(id), Some(ir::Op::StoreLocal { local: l, .. }) if *l == local) {
                            return true;
                        }
                    }
                    ir::Node::If { then_, else_, .. } => {
                        if go(m, then_, local, load, seen) || go(m, else_, local, load, seen) {
                            return true;
                        }
                    }
                    ir::Node::Loop { body } if go(m, body, local, load, seen) => return true,
                    _ => {}
                }
            }
            false
        }
        let mut seen = false;
        go(self.m, self.m.body, local, load, &mut seen)
    }

    /// May this slot be renamed instead of copied through?
    ///
    /// **The largest single source of copies in this file**, measured rather
    /// than guessed (`tests/mov_survey.rs`, 2026-09-22): of 2,544 copies over
    /// the corpus, 1,037 are a store to a slot and 1,089 a load from one, and
    /// only 326 are if-conversion's. The front end gives every expression
    /// temporary a slot, so `x = a; use(x); x = b; use(x)` copies the value
    /// in and back out again at every version -- for a slot that is one
    /// register holding successive, unrelated values.
    ///
    /// Renaming removes both copies: the slot stops being a register of its
    /// own and becomes a NAME for whichever value register currently holds
    /// it. A store points the name at the value; a load reads it. Nothing
    /// moves.
    ///
    /// Three conditions, and each is a way to be wrong without it:
    ///
    /// * **no arm of any `if` or loop touches it**, because the name is
    ///   maintained in walk order and an arm's version must not escape it
    ///   (`slot_under_control_flow` -- and note that reasoning by instruction
    ///   INDEX is only valid for such a slot, since `instrs` is a pool in
    ///   allocation order);
    /// * **every store writes the whole register**, since a masked store
    ///   keeps the other lanes and so needs a register that persists.
    ///
    /// The slot's current value is kept as a full OPERAND
    /// (`Sel::locals_val`), not as a register number, and that is the whole
    /// reason this reaches anything: measured over the corpus, 626 of the
    /// 1,185 slots are written at least once from a uniform, an attribute or
    /// a literal -- values that are folded into their consumers and have no
    /// register at all. A register-valued rename reaches 30 slots; an
    /// operand-valued one reaches those 626 as well.
    /// Could a SELECT AT THE JOIN carry this slot, rather than the arm
    /// copying the slot in and out?
    ///
    /// Counting only. The condition is the one the survey's `JoinReach` uses:
    /// every touching construct is an `if` whose arms may both be evaluated
    /// (a texture fetch or a discard in an arm forces a real branch, and then
    /// only one arm runs), no loop touches the slot, and every store to it
    /// writes the whole register.
    ///
    /// Why it is counted here and not only in the survey: the survey counts
    /// SLOTS and this counts the COPIES they cost, and those are different
    /// numbers by a factor nobody guessed right. I declined this item once on
    /// 65 slots without asking what the 65 were worth.
    fn slot_join_reachable(&self, local: ir::LocalId) -> bool {
        fn walk(s: &Sel<'_>, r: ir::Range, local: ir::LocalId) -> bool {
            s.m.nodes_of(r).iter().all(|n| match *n {
                ir::Node::If { then_, else_, .. } => {
                    let touched = s.stores_local(then_, local)
                        || s.stores_local(else_, local)
                        || s.reads_or_partly_writes(then_, local)
                        || s.reads_or_partly_writes(else_, local);
                    if touched && (s.must_not_speculate(then_) || s.must_not_speculate(else_)) {
                        return false;
                    }
                    walk(s, then_, local) && walk(s, else_, local)
                }
                /* A loop's slot cannot be a select at a join: the value has
                 * to survive the back edge. */
                ir::Node::Loop { body } => {
                    !(s.stores_local(body, local) || s.reads_or_partly_writes(body, local)) && walk(s, body, local)
                }
                _ => true,
            })
        }
        let whole = self.m.instrs.iter().all(|op| match op {
            ir::Op::StoreLocal { local: l, index, write_mask, .. } if *l == local => {
                index.is_none() && *write_mask == build::WRITE_MASK_ALL
            }
            _ => true,
        });
        whole && walk(self, self.m.body, local)
    }

    /// Do ALL the stores to this slot take their value from ONE IR value?
    ///
    /// The sizing question for per-component slot tracking, and the reason
    /// the item is smaller than the copies blamed on it. Tracking a slot's
    /// components apart removes a copy only if a later READ can still be ONE
    /// operand -- this machine reads four lanes of one register through one
    /// swizzle, so components living in four registers cannot be read as one
    /// operand at all and the masked stores that gather them are the work,
    /// not waste.
    ///
    /// `vec4 c; c.xy = a.xy; c.zw = a.zw;` qualifies and is free under the
    /// tracking; `vec4 c; c.x = f; c.y = g;` does not, whatever is tracked.
    fn masked_stores_share_one_value(&self, local: ir::LocalId) -> bool {
        let mut from: Option<u32> = None;
        let mut any = false;
        for op in self.m.instrs.iter() {
            let ir::Op::StoreLocal { local: l, index, src, .. } = op else { continue };
            if *l != local {
                continue;
            }
            if index.is_some() {
                return false;
            }
            any = true;
            match from {
                None => from = Some(src.val.0),
                Some(v) if v == src.val.0 => {}
                Some(_) => return false,
            }
        }
        any
    }

    fn slot_is_renamable(&self, local: ir::LocalId) -> bool {
        if self.slot_under_control_flow(local) {
            return false;
        }
        let mut stored = false;
        for op in self.m.instrs.iter() {
            let ir::Op::StoreLocal { local: l, index, src, write_mask } = op else { continue };
            if *l != local {
                continue;
            }
            if index.is_some() || *write_mask != build::WRITE_MASK_ALL {
                return false;
            }
            let _ = src;
            stored = true;
        }
        stored
    }

    /// Is every read of this slot after `at`?
    ///
    /// A read BEFORE the store reads whatever the slot held then, which the
    /// propagated source is not. Straight-line only, as everywhere here.
    fn every_read_after(&self, local: ir::LocalId, at: u32) -> bool {
        !self
            .m
            .instrs
            .iter()
            .take(at as usize)
            .any(|op| matches!(op, ir::Op::LoadLocal { local: l, .. } if *l == local))
    }

    /// Is this slot left alone from `from` to the end of the body?
    ///
    /// Straight-line only, which is what makes the question answerable by
    /// comparing indices: `nodes` refuses control flow before this is reached.
    fn local_is_stable_after(&self, local: ir::LocalId, from: u32) -> bool {
        /* UNSOUND across control flow, and silently so. The question asked is
         * "does any later instruction store this slot", answered by comparing
         * positions in the flat array -- which assumes every instruction after
         * `from` runs. A slot stored in one arm of an `if` and read after it
         * has no later store at all, so this would say "stable" and the read
         * would fold to the arm's value as though the arm always ran. The
         * picture would be wrong, not the compile.
         *
         * Asked PER SLOT rather than per program. This used to be off
         * entirely whenever the body held an `If`, which is sound and threw
         * away every slot in the shader to protect the ones an arm touches --
         * and after if-conversion became leaf-only, that is most shaders. */
        if self.slot_under_control_flow(local) {
            return false;
        }
        self.m
            .instrs
            .iter()
            .skip(from as usize + 1)
            .all(|op| !matches!(op, ir::Op::StoreLocal { local: l, .. } if *l == local))
    }

    /// Where the fragment colour goes: above every input, and never t0.
    ///
    /// The shipped compiler often puts it *on* an input register -- 37 of the
    /// blobs with one varying write the colour to that varying's register --
    /// which is safe there because it knows the input is dead by then. This
    /// compiler does no liveness analysis, so it takes the first register
    /// above all of them instead. That costs a register on a shader that reads
    /// several varyings and cannot be wrong, which is the right trade until
    /// liveness exists; reusing a register whose value is still needed would
    /// draw a picture rather than fail.
    fn fs_color_reg(&self) -> u8 {
        let highest = self
            .m
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.kind == ir::IoKind::Varying)
            .map(|(i, _)| (i + 1) as u8)
            .max()
            .unwrap_or(0);
        highest.saturating_add(1).max(FS_COLOR_MIN_REG)
    }

    /// The register an output is written to.
    fn output_reg(&self, slot: ir::IoSlot) -> Result<u8, SelectError> {
        match slot {
            ir::IoSlot::Position => Ok(VS_POS_REG),
            ir::IoSlot::FragColor(0) => Ok(self.fs_color_reg()),
            /* A further draw buffer's colour (ES 3.00 `layout(location = n)
             * out`, an output array's element n): a register of its own above
             * the first colour's, which the allocator then places like any
             * output an instruction writes. Four outputs, as the GC7000 drives
             * four render targets (Mesa's max_render_targets for HALTI2+). */
            ir::IoSlot::FragColor(n) if n < MAX_COLOUR_OUTPUTS => Ok(self.fs_color_reg().saturating_add(n)),
            ir::IoSlot::FragColor(_) => Err(SelectError::Unsupported("a colour output past the fourth draw buffer")),
            ir::IoSlot::Varying(loc) => {
                /* A vertex output feeding the fragment stage. Its number is
                 * the compiler's, and `link()` pairs the stages by name, so
                 * any consistent numbering serves; 1.. after the position
                 * keeps the two stages' dumps comparable. */
                let idx = self
                    .used_varying_index(self.m.outputs, loc, false)
                    .ok_or(SelectError::Unsupported("a varying written that the module does not declare"))?;
                if idx + 1 > MAX_IO {
                    return Err(SelectError::TooManyIo { got: idx + 1, max: MAX_IO });
                }
                Ok((idx + 1) as u8)
            }
            ir::IoSlot::PointSize => Ok(self.vs_pointsize_reg()),
            _ => Err(SelectError::Unsupported("this output is not selected yet")),
        }
    }

    /// Where `gl_PointSize` goes: above every other register this stage uses.
    ///
    /// There is no rule to copy. Only 4 real vertex blobs in the offline table
    /// write it, at registers 1, 2, 2 and 3, and in one of them it sits below
    /// the highest input -- so it is an allocation result, like
    /// `ps_color_out_reg`, and the shipped compiler places it wherever
    /// liveness allows.
    ///
    /// Four samples are not a rule, so this does not invent one from them. It
    /// takes a register above everything else the stage uses, which cannot
    /// collide with a value still wanted whatever the shader does. That costs
    /// one register on the handful of shaders that set a point size.
    fn vs_pointsize_reg(&self) -> u8 {
        let ins = self.m.inputs.iter().filter(|d| d.kind == ir::IoKind::Attribute).map(|d| d.location).max();
        /* A matrix varying takes a register per column (`used_varying_index`). */
        let outs: u8 = self
            .m
            .outputs
            .iter()
            .filter(|d| d.kind == ir::IoKind::Varying)
            .map(|d| varying_columns(self.m.outputs, d))
            .sum();
        /* Varyings occupy 1..=N and the position is VS_POS_REG.
         *
         * The position used to be folded in with `.max(VS_POS_REG)`, which is
         * dead code while that register is 0 -- clippy's
         * `unnecessary_min_or_max` says so, correctly. The DEPENDENCY is real
         * though: if the position ever moves off register 0 this count has to
         * include it. So it is asserted at compile time instead of written as
         * an operation the optimiser removes and a reader mistakes for a
         * guarantee. */
        const _: () =
            assert!(VS_POS_REG == 0, "the register count must include the position once it is not register 0");
        let highest = ins.unwrap_or(0).max(outs);
        highest.saturating_add(1)
    }

    /// Fill the constant file with the stage's uniform block, one entry per
    /// dword, in order, before any literal.
    ///
    /// The file then mirrors the block: block dword `d` is constant register
    /// `d / 4`, component `d % 4`, which is what makes [`Sel::uniform_src`] a
    /// division rather than a lookup.
    ///
    /// The shipped compiler also gives a `mat4` four consecutive registers,
    /// but **in the opposite order**: `hello.vert`'s `CD_4` is `12,13,14,15 /
    /// 8,9,10,11 / 4,5,6,7 / 0,1,2,3`, so its `u0` is column 3. Both layouts
    /// are self-consistent, because the IR names a uniform by its block dword
    /// and this file decides the register. What must never differ is which
    /// column meets which vector component, and that is the IR's to say --
    /// see the `mat4 * vec4` selection test, which had it backwards until
    /// es100-frontend checked it against `CD_4`.
    ///
    /// Rounded up to a whole register because a register IS four dwords: a
    /// block of six would otherwise leave the first literal sharing a register
    /// with two uniform dwords, and a literal's register number is what
    /// [`Sel::const_reg`] hands out.
    fn uniform_block(&mut self) -> Result<(), SelectError> {
        let used = self.m.uniform_dwords as usize;
        let dwords = used.div_ceil(4) * 4;
        /* The last register's lanes past the block are padding, which no
         * read reaches (`uniform_src` refuses one): described as uniform
         * dwords they named dwords the block does not have, and the backend
         * logged each one as missing on every draw (the i.MX8MP cluster's
         * 2.7 s frame, tests/gc_const_map.rs). */
        for d in 0..dwords {
            let slot = if d < used { ConstSlot::uniform(d as u32) } else { ConstSlot::UNUSED };
            self.consts.push(self.arena, slot)?;
        }
        /* A dynamically indexed array needs nothing more here: it is read
         * with `load` from the block's copy in memory (LoadUniformIdx), not
         * through the address register. Until 2026-09-25 each such array got
         * a second copy here, one element per register, for `mov v,
         * u[base][a.x]` -- the read that returned element 0 on the i.MX8MP. */
        Ok(())
    }

    /// A read of `comps` dwords of the uniform block starting at `dword_off`.
    fn uniform_src(&self, dword_off: u16, comps: u8, sw: [u8; 4]) -> Result<SelSrc, SelectError> {
        if dword_off >= self.m.uniform_dwords {
            return Err(SelectError::Unsupported("a uniform read past the block the module declares"));
        }
        let reg = dword_off / 4;
        let first = (dword_off % 4) as u8;
        /* The load's own components, then the caller's swizzle on top of them.
         * Components the load does not cover repeat its last rather than
         * running into the next uniform: reading four lanes of a vec2 uniform
         * must not quietly return the two dwords after it. */
        let mut base = [0u8; 4];
        for (i, b) in base.iter_mut().enumerate() {
            *b = first + (i as u8).min(comps.saturating_sub(1));
        }
        let mut p = build::Src::reg(build::RGROUP_UNIFORM_0, reg);
        p.swizzle =
            [base[sw[0] as usize & 3], base[sw[1] as usize & 3], base[sw[2] as usize & 3], base[sw[3] as usize & 3]];
        Ok(SelSrc::Physical(p))
    }

    /// Place a vec4 literal in the constant file, packed.
    ///
    /// A source operand addresses four components of **one** register, so a
    /// literal's distinct values must all live in the same register -- but
    /// they need not occupy four dwords. `vec4(1.0, 1.0, 0.66, 1.0)` has two
    /// distinct values, so it takes two dwords and a swizzle of `[0, 0, 1, 0]`.
    ///
    /// That is what the shipped compiler does, and it is why this replaced a
    /// register per literal: measured on `hello.frag`, whose source is exactly
    /// that constructor, the table's constant file is `1.0, 0.66` where ours
    /// was `1.0, 1.0, 0.66, 1.0`. The file is a scarce resource shared with
    /// the uniforms, so spending four dwords on two values is not free, and
    /// the difference showed up as three disagreeing fields in the oracle.
    ///
    /// Literals are also shared between uses: a register already holding the
    /// values a new literal needs is reused, and one with room takes the
    /// missing values rather than starting a register of its own.
    ///
    /// Returns the register and the swizzle that reads the literal back from
    /// it, which the caller composes with the use's own swizzle.
    fn const_reg(&mut self, bits: [u32; 4]) -> Result<(u16, [u8; 4]), SelectError> {
        /* Literals begin after the uniform block, which `uniform_block` laid
         * down first and whose dwords are not ours to pack into. */
        let first_literal = (self.m.uniform_dwords as usize).div_ceil(4);

        /* A register can serve this literal if, for each distinct value, it
         * already holds it or has a free dword to take it. Registers are
         * scanned in order, so the result does not depend on how the arena
         * happens to be laid out. */
        let n_regs = self.consts.len().div_ceil(4);
        for reg in first_literal..=n_regs {
            if let Some(sw) = self.try_place(reg, bits)? {
                return Ok((reg as u16, sw));
            }
        }
        /* Unreachable in practice: the loop above includes one register past
         * the end, which is empty and therefore always fits four values. */
        Err(SelectError::Unsupported("a literal that fits in no constant register"))
    }

    /// Try to read or write `bits` within one constant register, growing the
    /// file if that register is the next one. `None` if it does not fit.
    fn try_place(&mut self, reg: usize, bits: [u32; 4]) -> Result<Option<[u8; 4]>, SelectError> {
        let base = reg * 4;
        /* A scratch copy, so a register that turns out not to fit is left
         * exactly as it was: a half-written register would hold values no
         * instruction reads and shift every literal after it. */
        let mut slots = [None; 4];
        for (i, slot) in slots.iter_mut().enumerate() {
            if base + i < self.consts.len() {
                *slot = Some(self.consts[base + i]);
            }
        }
        let mut sw = [0u8; 4];
        for (lane, want) in bits.iter().enumerate() {
            let want_slot = ConstSlot::literal(*want);
            let found = slots.iter().position(|s| *s == Some(want_slot));
            let at = match found {
                Some(at) => at,
                None => match slots.iter().position(|s| s.is_none()) {
                    Some(free) => {
                        slots[free] = Some(want_slot);
                        free
                    }
                    /* Full, and this value is not in it. */
                    None => return Ok(None),
                },
            };
            sw[lane] = at as u8;
        }
        /* It fits: commit, extending the file to cover the register. A
         * register is four dwords, so an unused lane becomes UNUSED rather
         * than a value the file does not actually hold. */
        while self.consts.len() < base + 4 {
            let i = self.consts.len() - base;
            self.consts.push(self.arena, slots[i].unwrap_or(ConstSlot::UNUSED))?;
        }
        for (i, slot) in slots.iter().enumerate() {
            if let Some(c) = slot {
                self.consts[base + i] = *c;
            }
        }
        Ok(Some(sw))
    }

    /// Turn an IR source into a selected one, folding the definitions that need
    /// no instruction of their own.
    ///
    /// A `LoadInput` becomes a read of its input register and a `ConstF32` a
    /// read of the constant file, rather than each being materialised into a
    /// temporary with a `mov` in front of it. That folding is the substance of
    /// instruction selection on this machine: the hardware reads those places
    /// directly.
    /// A literal read from the constant file: the literal's own lane
    /// mapping, then the use's swizzle on top of it, so the packing is not
    /// visible to a caller that wrote `.yzw`.
    fn const_file_src(&mut self, bits: [u32; 4], swizzle: [u8; 4]) -> Result<SelSrc, SelectError> {
        let (reg, base) = self.const_reg(bits)?;
        let mut p = build::Src::reg(build::RGROUP_UNIFORM_0, reg);
        p.swizzle = [
            base[swizzle[0] as usize & 3],
            base[swizzle[1] as usize & 3],
            base[swizzle[2] as usize & 3],
            base[swizzle[3] as usize & 3],
        ];
        Ok(SelSrc::Physical(p))
    }

    /// An integer-typed literal (`ConstI32`, or `ConstBool` as its mask):
    /// an s20 immediate when every lane holds one value that fits, the
    /// constant file otherwise.
    fn int_literal_src(&mut self, bits: [u32; 4], swizzle: [u8; 4]) -> Result<SelSrc, SelectError> {
        if bits.iter().all(|b| *b == bits[0]) {
            if let Ok(imm) = build::Src::imm_s20(bits[0] as i32) {
                return Ok(SelSrc::Physical(imm));
            }
        }
        self.const_file_src(bits, swizzle)
    }

    fn src(&mut self, s: ir::Src) -> Result<SelSrc, SelectError> {
        let def = *self.m.def(s.val).ok_or(SelectError::UndefinedValue(s.val))?;
        match def {
            /* gl_FrontFacing is not an input register: `instr` computed it
             * into the value's own virtual register (front_facing). */
            ir::Op::LoadInput { slot: ir::IoSlot::FrontFacing, .. } => {
                Ok(SelSrc::Virtual { v: VReg(s.val.0), swizzle: s.swizzle, neg: false, abs: false })
            }
            /* gl_FragCoord is t0 with its y transformed (`frag_coord`), and
             * gl_PointCoord its row with its t transformed (`point_coord`). */
            ir::Op::LoadInput { slot: ir::IoSlot::FragCoord | ir::IoSlot::PointCoord, .. } => {
                Ok(SelSrc::Virtual { v: VReg(s.val.0), swizzle: s.swizzle, neg: false, abs: false })
            }
            /* gl_InstanceID is the `.y` of the id register; its one lane is
             * component 0 of the IR value, so every lane reads `.y`. */
            ir::Op::LoadInput { slot: ir::IoSlot::InstanceId, .. } => {
                let reg = self.input_reg(ir::IoSlot::InstanceId)?;
                Ok(SelSrc::Input { reg, swizzle: [1; 4], neg: false, abs: false })
            }
            ir::Op::LoadInput { slot, .. } => {
                let reg = self.input_reg(slot)?;
                Ok(SelSrc::Input { reg, swizzle: s.swizzle, neg: false, abs: false })
            }
            ir::Op::ConstF32(bits) => {
                /* A literal whose four lanes are one value, exactly
                 * representable, is an instruction immediate and costs no
                 * constant register at all. That is what the reference does:
                 * gl_test's ES 1.00 `tex.vert` writes `Unused = 0.5` as
                 * `mov t3, void, void, 0.5:f20`, where we spent four dwords of
                 * the constant file on it (const_count 20 against its 16).
                 *
                 * One value only, because an immediate is a scalar broadcast
                 * across the lanes -- there is no per-lane immediate -- and
                 * only when `imm_f32` accepts it, which it does for a value
                 * whose low 12 mantissa bits are zero and refuses otherwise
                 * rather than rounding. A rounded colour is a picture that is
                 * merely wrong. */
                if bits[0] == bits[1] && bits[1] == bits[2] && bits[2] == bits[3] {
                    if let Ok(imm) = build::Src::imm_f32(f32::from_bits(bits[0])) {
                        return Ok(SelSrc::Physical(imm));
                    }
                }
                self.const_file_src(bits, s.swizzle)
            }
            /* An integer literal, and a bool as the all-ones mask it is
             * here (see `op_type`): the same folding as a float one, with
             * the s20 immediate in place of the f20. An integer operation
             * reads its operand's BITS, so an int has to arrive as an int --
             * until 2026-09-23 the front end handed these over as float
             * constants and `i < 3` compared against the bits of 3.0.
             *
             * The immediate is sign-extended, which is what makes `-1:s20`
             * the true mask the comparisons already use. A value outside
             * 20 bits goes to the constant file as its 32-bit pattern. */
            ir::Op::ConstI32(v) => self.int_literal_src(v.map(|x| x as u32), s.swizzle),
            ir::Op::ConstBool(v) => self.int_literal_src(v.map(|b| if b { u32::MAX } else { 0 }), s.swizzle),
            /* Folded like a literal: the hardware reads the constant file
             * directly, so a uniform needs no instruction of its own. */
            ir::Op::LoadUniform { dword_off, comps } => self.uniform_src(dword_off, comps, s.swizzle),

            /* A local slot IS a virtual register, so reading one is reading
             * that register -- no instruction, exactly as an attribute read
             * is no instruction.
             *
             * That folding is only sound while the slot still holds what the
             * load saw. `float c = 1.0; float d = c; c = 2.0; use(d);` would
             * otherwise make the use of `d` read 2.0. Control flow is refused
             * here, so the body is straight-line and "written again later" is
             * a comparison of instruction indices; when it is, the load is
             * materialised into its own register and the copy is what makes
             * it a snapshot. */
            /* Propagated: the slot holds a value that is already in a
             * register, so the read becomes that register and the store that
             * filled the slot disappears. */
            ir::Op::LoadLocal { local, index: None, .. }
                if self.propagable_store(local).is_some_and(|(at, _)| at < s.val.0) =>
            {
                let (_, inner) = self.propagable_store(local).expect("just checked");
                let base = self.src(ir::Src { val: inner.val, swizzle: inner.swizzle })?;
                Ok(compose(base, s.swizzle))
            }

            /* A renamed slot is whatever operand its last store named, AS OF
             * THIS LOAD -- snapshotted when the load was walked, because by
             * the time the consumer is selected the slot may hold something
             * else. See `Sel::value_alias`. */
            ir::Op::LoadLocal { index: None, .. }
                if self.value_alias.get(s.val.0 as usize).copied().flatten().is_some() =>
            {
                let base = self.value_alias[s.val.0 as usize].expect("just checked");
                Ok(compose(base, s.swizzle))
            }

            ir::Op::LoadLocal { local, index: None, .. } if self.local_is_stable_after(local, s.val.0) => {
                Ok(SelSrc::Virtual { v: self.local_reg(local), swizzle: s.swizzle, neg: false, abs: false })
            }
            _ => Ok(SelSrc::Virtual { v: VReg(s.val.0), swizzle: s.swizzle, neg: false, abs: false }),
        }
    }

    /// One lane of a scalar operation.
    ///
    /// `sin`, `cos` and `log` are not one instruction on this core. It has
    /// the new transcendental unit, which Mesa's etnaviv lowers
    /// (etnaviv_nir.c `etna_lower_alu`, MIT) and the reference compiler emits
    /// the same way -- `mul t, x, u[1/pi]`; `sin.rtz t.zw, t`; `mul r, t.z,
    /// t.w` in every one of its shaders that calls them:
    ///
    /// * `sin`/`cos` take their argument in units of pi: it is scaled by 1/pi
    ///   first (the old cores used 2/pi);
    /// * `sin`, `cos` and `log` write TWO lanes, and the result is their
    ///   product;
    /// * they round toward zero.
    ///
    /// We emitted a plain `sin`/`cos`/`log`, so every one of them computed
    /// something else on silicon. `exp`, `rcp`, `rsq` and `sqrt` are single
    /// instructions (the reference's are too).
    fn scalar(
        &mut self,
        op: ir::AluOp,
        opid: OpId,
        ty: OpType,
        dst: SelDst,
        srcs: [Option<SelSrc>; 3],
    ) -> Result<(), SelectError> {
        let paired = matches!(op, ir::AluOp::Fsin | ir::AluOp::Fcos | ir::AluOp::Flog2);
        if !paired {
            let mut i = SelInstr::new(opid, ty).dst(dst);
            for (ord, s) in srcs.into_iter().enumerate() {
                if let Some(s) = s {
                    i = i.operand(ord, s);
                }
            }
            return self.emit(i);
        }
        let mul = build::op_id("mul").ok_or(SelectError::Unsupported("the machine has no `mul`"))?;
        let mut fresh = || {
            let v = VReg(self.next_vreg);
            self.next_vreg += 1;
            v
        };
        let (scaled, pair) = (fresh(), fresh());
        let x = srcs[0].ok_or(SelectError::Unsupported("a transcendental with no operand"))?;
        let splat = |v: VReg, c: u8| SelSrc::Virtual { v, swizzle: [c; 4], neg: false, abs: false };
        let arg = if matches!(op, ir::AluOp::Fsin | ir::AluOp::Fcos) {
            let inv_pi = (1.0f64 / core::f64::consts::PI) as f32;
            let k = self.const_file_src([inv_pi.to_bits(); 4], ir::SWIZZLE_IDENTITY)?;
            self.emit(
                SelInstr::new(mul, OpType::F32)
                    .dst(SelDst::Virtual { v: scaled, write_mask: 0x1 })
                    .operand(0, x)
                    .operand(1, k),
            )?;
            splat(scaled, 0)
        } else {
            x
        };
        self.emit(
            SelInstr::new(opid, ty)
                .dst(SelDst::Virtual { v: pair, write_mask: 0x3 })
                .operand(0, arg)
                .round_toward_zero(),
        )?;
        self.emit(SelInstr::new(mul, OpType::F32).dst(dst).operand(0, splat(pair, 0)).operand(1, splat(pair, 1)))
    }

    fn emit(&mut self, i: SelInstr) -> Result<(), SelectError> {
        let next = core::cell::Cell::new(self.next_vreg);
        let r = push_one_constant_register(&mut self.instrs, self.arena, &next, self.mov, i);
        self.next_vreg = next.get();
        r
    }

    /// One IR arithmetic operation.
    ///
    /// `builtins::native` is asked first and `builtins::lower` only when the
    /// answer is `None`, which is the contract that file documents: the list
    /// of what the machine has and the expansions for what it does not live
    /// together, so an operation cannot be in both.
    fn alu(&mut self, id: ir::InstrId, op: ir::AluOp, srcs: &[ir::Src; 3]) -> Result<(), SelectError> {
        let ty = op_type(op).ok_or(SelectError::Unsupported("a boolean operation: its operand type is not settled"))?;
        let n = op.arity();
        let mut sel = [None, None, None];
        for (i, out) in sel.iter_mut().enumerate().take(n) {
            *out = Some(self.src(srcs[i])?);
        }
        /* `dsx` and `dsy` read their one source from BOTH of their slots,
         * SRC0 and SRC2: Mesa's etnaviv (MIT, etnaviv_compiler_nir.c,
         * nir_intrinsic_ddx / ddy) sets `.src[0] = src, .src[1] = src`, and
         * its second operand is the instruction's SRC2. With SRC2 empty the
         * allocator refused every derivative (WrongSourceSlot(2)). */
        /* `dFdy` is along GL's y, which the rasteriser counts the other way
         * on a target this backend draws mirrored: the source is scaled by
         * the same per-draw y scale `gl_FragCoord` takes (-1 there, 1
         * elsewhere; `frag_coord`), as Mesa's nir_lower_wpos_ytransform
         * turns `ddy(p)` into `ddy(p * transform.x)` (lower_ddy). */
        if op == ir::AluOp::Fddy {
            let mul = build::op_id("mul").ok_or(SelectError::Unsupported("the instruction set has no `mul`"))?;
            let (reg, [ls, _]) = self.fragcoord_ytf_reg()?;
            let mut scale = build::Src::reg(build::RGROUP_UNIFORM_0, reg);
            scale.swizzle = [ls; 4];
            let p = sel[0].ok_or(SelectError::Unsupported("a derivative with no operand"))?;
            let v = VReg(self.next_vreg);
            self.next_vreg += 1;
            self.emit(
                SelInstr::new(mul, OpType::F32)
                    .dst(SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL })
                    .operand(0, p)
                    .operand(1, SelSrc::Physical(scale)),
            )?;
            sel[0] = Some(SelSrc::Virtual { v, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false });
        }
        if matches!(op, ir::AluOp::Fddx | ir::AluOp::Fddy) {
            sel[1] = sel[0];
        }

        /* The instruction's result is the value the instruction defines:
         * `instrs[i]` produces `ValueId(i)`, and `src` turns a use of it into
         * `VReg(i)`. One numbering, so no map. */
        let dst = SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL };

        if let Some(name) = builtins::native(op, ty) {
            let opid = build::op_id(name).ok_or(SelectError::Unsupported(
                "the instruction set lacks a native opcode the lowering library names",
            ))?;
            /* A scalar operation computes ONE value, from its source's first
             * swizzle lane, and writes it to every lane it is told to: `sin`
             * of a vec4 emitted once was sin(u.x) in all four lanes. So it
             * is emitted once per lane, each reading its own lane; the lanes
             * nobody reads are deleted afterwards (alloc::eliminate_dead). */
            if opid.reads_one_component() {
                for lane in 0..4u8 {
                    let at = |s: SelSrc| lane_of(s, lane);
                    let srcs_c = [sel[0].map(at), sel[1].map(at), sel[2].map(at)];
                    let dst_c = SelDst::Virtual { v: VReg(id.0), write_mask: 1 << lane };
                    self.scalar(op, opid, ty, dst_c, srcs_c)?;
                }
                return Ok(());
            }
            let mut i = SelInstr::new(opid, ty).dst(dst);
            for (ord, s) in sel.into_iter().enumerate() {
                if let Some(s) = s {
                    i = i.operand(ord, s);
                }
            }
            return self.emit(i);
        }

        /* Not native, so the sequence belongs to `builtins::lower`. It emits
         * through a callback and takes fresh temporaries from a counter that
         * starts above every ValueId, so a lowering's scratch cannot collide
         * with a value the IR defined.
         *
         * This call was impossible for a while: `builtins.rs` declared its own
         * `SelInstr`/`SelDst`/`SelSrc`/`VReg` beside `alloc.rs`'s and the two
         * had diverged. No adapter was written in the meantime, deliberately
         * -- the conversion would have been lossy in the direction needed (a
         * `Physical` operand that is not a uniform register had no
         * representation on the other side), and a converter that drops what
         * it is handed emits a plausible instruction reading the wrong
         * register, which draws a picture rather than failing. pvrgpu-platform
         * made it one set of types instead, which is what this call rests on.
         *
         * `lower` takes a plain slice, so the arity-sized prefix is flattened
         * out of the Option array. The filler is never read: `lower` checks
         * the length against `AluOp::arity` and refuses a mismatch. */
        let mut flat = [SelSrc::Virtual { v: VReg(0), swizzle: [0; 4], neg: false, abs: false }; 3];
        for (i, src) in sel.iter().enumerate() {
            if let Some(src) = src {
                flat[i] = *src;
            }
        }

        /* The unsigned divide's reciprocal scale is not an f20 immediate, so
         * it comes from the constant file (builtins::UDIV_RCP_SCALE_BITS). */
        let udiv_k = match op {
            ir::AluOp::Udiv | ir::AluOp::Umod | ir::AluOp::Imod => {
                Some(self.const_file_src([builtins::UDIV_RCP_SCALE_BITS; 4], ir::SWIZZLE_IDENTITY)?)
            }
            _ => None,
        };
        let arena = self.arena;
        let mov = self.mov;
        let instrs = &mut self.instrs;
        let next = core::cell::Cell::new(self.next_vreg);
        let mut push_err = None;
        let r = {
            let mut out = |i: SelInstr| {
                if push_err.is_none() {
                    push_err = push_one_constant_register(instrs, arena, &next, mov, i).err();
                }
            };
            let mut fresh = || {
                let v = VReg(next.get());
                next.set(next.get() + 1);
                v
            };
            match udiv_k {
                Some(k) if op == ir::AluOp::Imod => builtins::imod(dst, &flat[..n], k, &mut out, &mut fresh),
                Some(k) => builtins::udiv(op == ir::AluOp::Umod, dst, &flat[..n], k, &mut out, &mut fresh),
                None => builtins::lower(op, ty, dst, &flat[..n], &mut out, &mut fresh),
            }
        };
        self.next_vreg = next.get();
        r?;
        match push_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// One texture look-up.
    ///
    /// The same shape as [`Select::alu`] and for the same reason: the lowering
    /// emits through a callback and takes scratch from the counter that starts
    /// above every `ValueId`, so the projective form -- which is a reciprocal,
    /// a multiply and a look-up on this machine -- cannot collide with a value
    /// the IR defined.
    ///
    /// The sampler is a BINDING and never a register: it rides in `TEX_ID`,
    /// which `builtins::texture` puts in the instruction's `tex` field.
    fn tex(
        &mut self,
        id: ir::InstrId,
        kind: ir::Es100Tex,
        sampler: u8,
        coord: ir::Src,
        extra: Option<ir::Src>,
    ) -> Result<(), SelectError> {
        let dst = SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL };
        let c = self.src(coord)?;
        let x = match extra {
            Some(e) => Some(self.src(e)?),
            None => None,
        };

        let arena = self.arena;
        let mov = self.mov;
        let instrs = &mut self.instrs;
        let next = core::cell::Cell::new(self.next_vreg);
        let mut push_err = None;
        let r = {
            let mut out = |i: SelInstr| {
                if push_err.is_none() {
                    push_err = push_one_constant_register(instrs, arena, &next, mov, i).err();
                }
            };
            let mut fresh = || {
                let v = VReg(next.get());
                next.set(next.get() + 1);
                v
            };
            builtins::texture(kind, sampler, dst, c, x, &mut out, &mut fresh)
        };
        self.next_vreg = next.get();
        r?;
        match push_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// `texelFetch(s, p, lod)`: `builtins::texel_fetch`, with the level
    /// turned into a float first as Mesa's etnaviv does
    /// (`legalize_txf_lod`). A literal level is folded into the f20
    /// immediate the reference carries (`0.0:f20` for `texelFetch(.., 0)`);
    /// any other is one `i2f`.
    fn tex_fetch(&mut self, id: ir::InstrId, sampler: u8, coord: ir::Src, lod: ir::Src) -> Result<(), SelectError> {
        let dst = SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL };
        let c = self.src(coord)?;
        let literal = match self.m.def(lod.val) {
            Some(ir::Op::ConstI32(bits)) => {
                build::Src::imm_f32(bits[usize::from(lod.swizzle[0])] as f32).ok().map(SelSrc::Physical)
            }
            _ => None,
        };
        let level = match literal {
            Some(imm) => imm,
            None => {
                let i2f = build::op_id("i2f").ok_or(SelectError::Unsupported("the instruction set has no `i2f`"))?;
                let l = self.src(lod)?;
                let v = VReg(self.next_vreg);
                self.next_vreg += 1;
                self.emit(
                    SelInstr::new(i2f, OpType::S32)
                        .dst(SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL })
                        .operand(0, l),
                )?;
                SelSrc::Virtual { v, swizzle: ir::SWIZZLE_SPLAT0, neg: false, abs: false }
            }
        };
        let arena = self.arena;
        let mov = self.mov;
        let instrs = &mut self.instrs;
        let next = core::cell::Cell::new(self.next_vreg);
        let mut push_err = None;
        let r = {
            let mut out = |i: SelInstr| {
                if push_err.is_none() {
                    push_err = push_one_constant_register(instrs, arena, &next, mov, i).err();
                }
            };
            builtins::texel_fetch(sampler, dst, c, level, &mut out)
        };
        self.next_vreg = next.get();
        r?;
        match push_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// `textureSize(s, 0)`: the bound texture's width, height and depth (the
    /// layers of an array, the slices of a volume), which the driver puts in
    /// the constant file per draw (`SHC_VIV_CONST_TEXTURE_WIDTH` / `_HEIGHT` /
    /// `_DEPTH`, data = the sampler's unit; `vivante/shader.rs const_value`).
    /// Mesa's etnaviv lowers the query the same way (etnaviv_nir_lower_texture.c
    /// `lower_txs`: `load_texture_size_etna`), and the reference read
    /// `es300/texsize.frag`'s sizes straight from such a register (contents
    /// 5, 6, 7 with the unit as data).
    ///
    /// Only level 0: another level is `max(size >> lod, 1)` per lane with the
    /// layer count unshifted (Mesa leaves it to nir_lower_tex's
    /// `lower_txs_lod`), which nothing in the corpus asks for and nothing has
    /// measured, so it is refused by name.
    fn tex_size(&mut self, id: ir::InstrId, sampler: u8, lod: ir::Src) -> Result<(), SelectError> {
        let zero = match self.m.def(lod.val) {
            Some(ir::Op::ConstI32(bits)) => bits[usize::from(lod.swizzle[0])] == 0,
            _ => false,
        };
        if !zero {
            return Err(SelectError::Unsupported("textureSize at a level other than a literal 0 is not selected yet"));
        }
        let kinds = [
            crate::shaderc::SHC_VIV_CONST_TEXTURE_WIDTH,
            crate::shaderc::SHC_VIV_CONST_TEXTURE_HEIGHT,
            crate::shaderc::SHC_VIV_CONST_TEXTURE_DEPTH,
        ];
        let (reg, lanes) = self.driver_reg(kinds.map(|k| ConstSlot { contents: k, data: u32::from(sampler) }))?;
        let mut p = build::Src::reg(build::RGROUP_UNIFORM_0, reg);
        p.swizzle = [lanes[0], lanes[1], lanes[2], lanes[2]];
        self.emit(
            SelInstr::new(self.mov, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL })
                .operand(MOV_OPERAND, SelSrc::Physical(p)),
        )
    }

    /// The constant register holding these three driver-filled dwords, and
    /// the lane of each: the one already placed, or a new register after
    /// everything else.
    fn driver_reg(&mut self, want: [ConstSlot; 3]) -> Result<(u16, [u8; 3]), SelectError> {
        let n = self.consts.len();
        for base in (0..n).step_by(4) {
            let lane_of = |w: ConstSlot| (base..(base + 4).min(n)).position(|i| self.consts[i] == w);
            if let (Some(a), Some(b), Some(c)) = (lane_of(want[0]), lane_of(want[1]), lane_of(want[2])) {
                return Ok(((base / 4) as u16, [a as u8, b as u8, c as u8]));
            }
        }
        while !self.consts.len().is_multiple_of(4) {
            self.consts.push(self.arena, ConstSlot::UNUSED)?;
        }
        let reg = (self.consts.len() / 4) as u16;
        for c in [want[0], want[1], want[2], ConstSlot::UNUSED] {
            self.consts.push(self.arena, c)?;
        }
        Ok((reg, [0, 1, 2]))
    }

    /// `gl_FrontFacing` as the reference computes it: internal register 0
    /// compared against the integer 0 with COND 6 (`ne`), true written as
    /// all ones -- the boolean mask every other comparison here makes, so a
    /// reader of the value needs nothing special. Mesa's etnaviv reads the
    /// same register (etnaviv_compiler_nir.c: load_front_face is
    /// ISA_REG_GROUP_INTERNAL). The internal group is an operand only `cmp`
    /// takes (`OpId::takes_group`), which is why it is one here and not a
    /// folded operand of the reader.
    fn front_facing(&mut self, id: ir::InstrId) -> Result<(), SelectError> {
        let cmp = build::op_id("cmp").ok_or(SelectError::Unsupported("the instruction set has no `cmp`"))?;
        let imm = |v: i32| {
            build::Src::imm_s20(v)
                .map(SelSrc::Physical)
                .map_err(|_| SelectError::Unsupported("an immediate out of range"))
        };
        let internal = SelSrc::Physical(build::Src::reg(build::RGROUP_INTERNAL, 0));
        let mut i = SelInstr::new(cmp, OpType::S32)
            .dst(SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL })
            .operand(0, internal)
            .operand(1, imm(0)?)
            .operand(2, imm(-1)?);
        /* The hardware value has no sense of its own: which winding reads
         * as front is compiled in, one binary per sense, as Mesa's etnaviv
         * keys it (etnaviv_nir.c: ine for one winding, ieq for the other)
         * and as the offline table's `@front_cw` twins are built. */
        i.cond = Some(if self.front_flipped { Cond::Eq } else { Cond::Ne });
        self.emit(i)
    }

    /// `gl_FragCoord` with GL's y on every target: t0 as the rasteriser left
    /// it, and y' = y * scale + offset from a constant register the driver
    /// fills per draw (`SHC_VIV_CONST_FRAGCOORD_Y_SCALE` / `_OFFSET`: -1 and
    /// the target height on a target the backend draws mirrored, the window
    /// surface; 1 and 0 otherwise). The GC7000 draws a top-down target
    /// mirrored, so its rasteriser's y counts from the top there, and gl_test
    /// 82/84 and dEQP builtin_variable.fragcoord_xyz read it flipped. The same
    /// pair the PowerVR programs carry (`fragcoord_ytf_off1`), Mesa's
    /// nir_lower_wpos_ytransform in shape. The pixel centre survives:
    /// H - (row + 0.5) = (H - 1 - row) + 0.5.
    fn frag_coord(&mut self, id: ir::InstrId) -> Result<(), SelectError> {
        let mad = build::op_id("mad").ok_or(SelectError::Unsupported("the instruction set has no `mad`"))?;
        let (reg, [ls, lo]) = self.fragcoord_ytf_reg()?;
        let lane = |l: u8| {
            let mut p = build::Src::reg(build::RGROUP_UNIFORM_0, reg);
            p.swizzle = [l; 4];
            SelSrc::Physical(p)
        };
        let t0 = |sw: [u8; 4]| SelSrc::Input { reg: FS_FRAGCOORD_REG, swizzle: sw, neg: false, abs: false };
        let v = VReg(id.0);
        self.emit(
            SelInstr::new(self.mov, OpType::F32)
                .dst(SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL })
                .operand(MOV_OPERAND, t0(ir::SWIZZLE_IDENTITY)),
        )?;
        self.emit(
            SelInstr::new(mad, OpType::F32)
                .dst(SelDst::Virtual { v, write_mask: 0b0010 })
                .operand(0, t0([1; 4]))
                .operand(1, lane(ls))
                .operand(2, lane(lo)),
        )
    }

    /// The constant register and lane holding the stage's uniform-block
    /// address (`SHC_VIV_CONST_UBO_ADDR`, block 0), for `load`: the one
    /// already placed, or a new register after everything else.
    fn ubo_addr_reg(&mut self) -> Result<(u16, u8), SelectError> {
        let want = ConstSlot { contents: crate::shaderc::SHC_VIV_CONST_UBO_ADDR, data: 0 };
        if let Some(i) = (0..self.consts.len()).find(|&i| self.consts[i] == want) {
            return Ok(((i / 4) as u16, (i % 4) as u8));
        }
        while !self.consts.len().is_multiple_of(4) {
            self.consts.push(self.arena, ConstSlot::UNUSED)?;
        }
        let reg = (self.consts.len() / 4) as u16;
        for c in [want, ConstSlot::UNUSED, ConstSlot::UNUSED, ConstSlot::UNUSED] {
            self.consts.push(self.arena, c)?;
        }
        Ok((reg, 0))
    }

    /// `gl_PointCoord` with GL's t on every target: its row as the
    /// rasteriser fills it, and t' = t * scale + offset from a constant pair
    /// the driver fills per draw (`SHC_VIV_CONST_PNTC_Y_SCALE` / `_OFFSET`:
    /// 1 and 0 on a target the backend draws mirrored, -1 and 1 otherwise).
    /// The sprite's t runs from the top of the target as the rasteriser sees
    /// it; that is GL's top only on a mirrored target. dEQP
    /// builtin_variable.pointcoord broke on the i.MX8MP when pbuffers became
    /// y-up (orientation step 1, 2a57943d: it passed at 89278cf1, failed at
    /// 1eda74e5), as gl_FragCoord.y had on the window (1f745b8f).
    fn point_coord(&mut self, id: ir::InstrId) -> Result<(), SelectError> {
        let mad = build::op_id("mad").ok_or(SelectError::Unsupported("the instruction set has no `mad`"))?;
        let row = self.pntc_reg()?;
        let (reg, [ls, lo]) = self
            .driver_pair_reg(crate::shaderc::SHC_VIV_CONST_PNTC_Y_SCALE, crate::shaderc::SHC_VIV_CONST_PNTC_Y_OFFSET)?;
        let lane = |l: u8| {
            let mut p = build::Src::reg(build::RGROUP_UNIFORM_0, reg);
            p.swizzle = [l; 4];
            SelSrc::Physical(p)
        };
        let input = |sw: [u8; 4]| SelSrc::Input { reg: row, swizzle: sw, neg: false, abs: false };
        let v = VReg(id.0);
        self.emit(
            SelInstr::new(self.mov, OpType::F32)
                .dst(SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL })
                .operand(MOV_OPERAND, input(ir::SWIZZLE_IDENTITY)),
        )?;
        self.emit(
            SelInstr::new(mad, OpType::F32)
                .dst(SelDst::Virtual { v, write_mask: 0b0010 })
                .operand(0, input([1; 4]))
                .operand(1, lane(ls))
                .operand(2, lane(lo)),
        )
    }

    /// The constant register holding the y-transform pair, and the two lanes
    /// (scale, offset): the first one already holding both, or a new register
    /// after everything else.
    fn fragcoord_ytf_reg(&mut self) -> Result<(u16, [u8; 2]), SelectError> {
        self.driver_pair_reg(
            crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_SCALE,
            crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_OFFSET,
        )
    }

    /// The constant register and lanes of a (scale, offset) pair the driver
    /// fills per draw: the first register already holding both, or a new one
    /// after everything else.
    fn driver_pair_reg(&mut self, scale_kind: u8, offset_kind: u8) -> Result<(u16, [u8; 2]), SelectError> {
        let scale = ConstSlot { contents: scale_kind, data: 0 };
        let offset = ConstSlot { contents: offset_kind, data: 0 };
        let n = self.consts.len();
        for base in (0..n).step_by(4) {
            let lane_of = |want: ConstSlot| (base..(base + 4).min(n)).position(|i| self.consts[i] == want);
            if let (Some(a), Some(b)) = (lane_of(scale), lane_of(offset)) {
                return Ok(((base / 4) as u16, [a as u8, b as u8]));
            }
        }
        while !self.consts.len().is_multiple_of(4) {
            self.consts.push(self.arena, ConstSlot::UNUSED)?;
        }
        let reg = (self.consts.len() / 4) as u16;
        for c in [scale, offset, ConstSlot::UNUSED, ConstSlot::UNUSED] {
            self.consts.push(self.arena, c)?;
        }
        Ok((reg, [0, 1]))
    }

    fn instr(&mut self, id: ir::InstrId, op: &ir::Op) -> Result<(), SelectError> {
        match *op {
            ir::Op::LoadInput { slot: ir::IoSlot::FrontFacing, .. } => self.front_facing(id),
            ir::Op::LoadInput { slot: ir::IoSlot::FragCoord, .. } => self.frag_coord(id),
            ir::Op::LoadInput { slot: ir::IoSlot::PointCoord, .. } => self.point_coord(id),
            /* Folded into their uses by `src`; they emit nothing themselves. */
            ir::Op::LoadInput { .. }
            | ir::Op::ConstF32(_)
            | ir::Op::ConstI32(_)
            | ir::Op::ConstBool(_)
            | ir::Op::LoadUniform { .. } => Ok(()),

            ir::Op::StoreOutput { slot: ir::IoSlot::PointSize, src } => {
                /* gl_PointSize is clamped to GL_ALIASED_POINT_SIZE_RANGE here,
                 * because the GC7000 does not clamp it to what the GL reports:
                 * dEQP rasterization.limits.points drew its size-80 point at
                 * 80 against a reported 64 (i.MX8MP, Zephyr, 2362422d). Mesa's
                 * etnaviv reports 8192 and passes by the hardware's own clamp,
                 * but that needs an 8200-wide framebuffer and ours stop at
                 * 4096 (GL_MAX_RENDERBUFFER_SIZE). max against the floor, then
                 * min against the ceiling, as the PowerVR compiler does. */
                let s = self.src(src)?;
                let imm = |v: f32| {
                    build::Src::imm_f32(v)
                        .map(SelSrc::Physical)
                        .map_err(|_| SelectError::Unsupported("a point size bound that is not an immediate"))
                };
                let (lo_bound, hi_bound) = (imm(POINT_SIZE_MIN)?, imm(POINT_SIZE_MAX)?);
                let lo = VReg(self.next_vreg);
                self.next_vreg += 1;
                let whole = SelSrc::Virtual { v: lo, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false };
                self.lower_one(
                    ir::AluOp::Fmax,
                    SelDst::Virtual { v: lo, write_mask: build::WRITE_MASK_ALL },
                    &[s, lo_bound],
                )?;
                let reg = self.output_reg(ir::IoSlot::PointSize)?;
                self.lower_one(
                    ir::AluOp::Fmin,
                    SelDst::Fixed { reg, write_mask: build::WRITE_MASK_ALL },
                    &[whole, hi_bound],
                )
            }
            ir::Op::StoreOutput { slot, src } => {
                let s = self.src(src)?;
                /* The fragment colour goes wherever it already is, when it is
                 * already somewhere: `gl_FragColor = v_col` gives the colour
                 * that varying's own register (ps_color_out_reg 1) and emits
                 * no instruction, as the reference program does.
                 *
                 * Safe without liveness because the varying is the value
                 * being stored: whatever it held is exactly what the colour
                 * must be, so nothing that is still wanted is overwritten.
                 *
                 * **Only above [`FS_COLOR_MIN_REG`].** `gl_FragCoord` is an
                 * input too, and it is register 0, which is not a register the
                 * colour may live in -- 0 of the 138 real fragment blobs put
                 * their colour there, which is what that constant records.
                 * Without this guard `gl_FragColor = gl_FragCoord` compiled to
                 * an EMPTY program claiming the colour was already in place in
                 * register 0; the reference emits `mov t1, t0.xyzw`. Found by
                 * diffing the two back ends on that one line of GLSL, which is
                 * the only way it could have been found: every source in the
                 * corpus that reads gl_FragCoord also uses control flow, so
                 * the oracle never reached it. */
                let reg = match (slot, &s) {
                    (ir::IoSlot::FragColor(0), SelSrc::Input { reg, swizzle, neg, abs })
                        if *swizzle == ir::SWIZZLE_IDENTITY
                            && !neg
                            && !abs
                            && *reg >= FS_COLOR_MIN_REG
                            && self.single_store(slot) =>
                    {
                        *reg
                    }
                    _ => self.output_reg(slot)?,
                };
                if slot == ir::IoSlot::FragColor(0) {
                    self.ps_color_reg = Some(reg);
                }
                if let ir::IoSlot::FragColor(n) = slot {
                    if let Some(r) = self.colour_regs.get_mut(n as usize) {
                        *r = Some(reg);
                    }
                }
                /* No copy of a register onto itself: an input arrives in the
                 * same temporary file the outputs are read from, so an output
                 * given the register its source occupies is already in place
                 * and the program needs no instruction, as the reference
                 * program does (`gl_Position = a_pos` is four zero dwords:
                 * codegen chain_tests
                 * `the_passthrough_vertex_shader_becomes_a_real_shcshader`). */
                if self.is_already_in_place(reg, &s) && self.single_store(slot) {
                    return Ok(());
                }
                /* Or the instruction that computed it can write the output
                 * register itself, and the copy disappears.
                 *
                 * The reference compiler does this: `hello.vert`'s blob ends
                 * `mad t0, u3, t0.xxxx, t1` -- the last accumulate writes the
                 * position register, rather than a temporary that a `mov`
                 * then copies. Without it we emit five instructions where the
                 * shipped blob has four. */
                if self.retarget_last_to(SelDst::Fixed { reg, write_mask: build::WRITE_MASK_ALL }, src, &s) {
                    return Ok(());
                }
                note_site(|v| v.store_output += 1);
                self.emit(
                    SelInstr::new(self.mov, OpType::F32)
                        .dst(SelDst::Fixed { reg, write_mask: build::WRITE_MASK_ALL })
                        .operand(MOV_OPERAND, s),
                )
            }

            /* Everything below is a gap, and each says which. The lowering
             * library (pvrgpu-platform's `native`/`lower`) is the seam the
             * arithmetic goes through and is not wired yet; the rest is
             * outside the M3 subset by design. An IR construct quietly skipped
             * would produce a shorter shader that still draws, which is the
             * failure this whole path exists to avoid. */
            ir::Op::Alu(op, srcs) => self.alu(id, op, &srcs),
            /* A dynamically indexed uniform array.
             *
             * MEASURED against the reference: `f2i` the index (the front end
             * already gives it as one), `movar` it into the address register,
             * then read the array's base register with `amode = 1`, which adds
             * the address register to the register number.
             *
             *   f2i   t.x, u[fi]
             *   movar a.x, t.x
             *   mov   dst, u[base].xyzw  (amode 1)
             *
             * The array lives one element per REGISTER -- see `uniform_block`
             * -- because the address register steps registers, not dwords.
             *
             * The clamp is ours and not the reference's: it emits none.
             * `ir.md` requires it, because an out-of-range read walks into
             * another uniform or a sampler's properties, and "undefined" in
             * ES 2.0 does not have to mean "whatever is next in the file". */
            ir::Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
                if len == 0 {
                    return Err(SelectError::Unsupported("an indexed array of no elements"));
                }
                let idx = self.src(index)?;
                let imm = |v: i32| {
                    build::Src::imm_s20(v)
                        .map(SelSrc::Physical)
                        .map_err(|_| SelectError::Unsupported("an index bound too large for an immediate"))
                };
                let hi = imm(i32::from(len) - 1)?;
                let zero = imm(0)?;

                let mut fresh = || {
                    let v = VReg(self.next_vreg);
                    self.next_vreg += 1;
                    v
                };
                let (c1, lo, c2, cl, off, got) = (fresh(), fresh(), fresh(), fresh(), fresh(), fresh());
                let whole = |v: VReg| SelSrc::Virtual { v, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false };
                let dst = |v: VReg| SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL };

                /* min(index, len - 1) then max(that, 0). Two compares and two
                 * selects, because this machine has no integer min or max.
                 * UNIIDX_NO_CLAMP drops them and uses the index itself, as
                 * Mesa's etnaviv does (a measurement, never a default). */
                let addr = if uniidx_knob(UNIIDX_NO_CLAMP) {
                    idx
                } else {
                    self.lower_one(ir::AluOp::Ilt, dst(c1), &[idx, hi])?;
                    self.lower_one(ir::AluOp::Bcsel, dst(lo), &[whole(c1), idx, hi])?;
                    self.lower_one(ir::AluOp::Ilt, dst(c2), &[whole(lo), zero])?;
                    self.lower_one(ir::AluOp::Bcsel, dst(cl), &[whole(c2), zero, whole(lo)])?;
                    whole(cl)
                };

                /* The read itself, as Mesa's etnaviv ships it for this GPU:
                 * the stage's uniform block is in memory as well as in the
                 * constant file (the driver copies it out per draw and fills
                 * SHC_VIV_CONST_UBO_ADDR with its address), and the element
                 * is a `load` from there:
                 *
                 *   imullo0.s32 t.x, index, stride_bytes
                 *   add.s32     t.x, t.x, base_bytes      (when base > 0)
                 *   load.u32    v, t.x, u[ubo_addr]
                 *
                 * Mesa's GL path lowers every uniform to UBO 0 and turns only
                 * constant-offset loads back into constant-file reads
                 * (etnaviv_nir_lower_ubo_to_uniform), so a run-time index is
                 * always this load; its golden stream for gl_test 108 D is
                 * `imullo0.s32 t1.x, u0.x, u0.y; load.u32 t1, t1.x, u0.z`
                 * (cs_golden mode 25, 2026-09-25). The constant file's own
                 * relative addressing -- `movar a.x` then `mov v, u[base][a.x]`
                 * -- read element 0 on the i.MX8MP whatever the index, in every
                 * shape tried (108, 116, 117, 120-122, 128, and with the clamp
                 * dropped and the array at its own base). */
                let load = build::op_id("load").ok_or(SelectError::Unsupported("the machine has no `load`"))?;
                let stride = imm(i32::from(stride_dw) * 4)?;
                let (ubo_reg, ubo_lane) = self.ubo_addr_reg()?;
                let mut base = build::Src::reg(build::RGROUP_UNIFORM_0, ubo_reg);
                base.swizzle = [ubo_lane; 4];
                let x = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: false, abs: false };
                let imul = build::op_id("imullo0").ok_or(SelectError::Unsupported("the machine has no `imullo0`"))?;
                self.emit(
                    SelInstr::new(imul, OpType::S32)
                        .dst(SelDst::Virtual { v: off, write_mask: 0x1 })
                        .operand(0, addr)
                        .operand(1, stride),
                )?;
                if base_dw != 0 {
                    let first = imm(i32::from(base_dw) * 4)?;
                    /* the constant first, the running offset second (SRC0 and
                     * SRC2, as the corpus's `add.s32 t2, u4.xxxx, void, t2`) */
                    let add = build::op_id("add").ok_or(SelectError::Unsupported("the machine has no `add`"))?;
                    self.emit(
                        SelInstr::new(add, OpType::S32)
                            .dst(SelDst::Virtual { v: off, write_mask: 0x1 })
                            .operand(0, first)
                            .operand(1, x(off)),
                    )?;
                }
                self.emit(
                    SelInstr::new(load, OpType::U32)
                        .dst(dst(got))
                        .operand(0, x(off))
                        .operand(1, SelSrc::Physical(base)),
                )?;

                /* Element k is `comps` dwords from its start; lanes past them
                 * repeat the last, as a read of a narrower uniform does. */
                let lane = comps.saturating_sub(1).min(3);
                self.emit(
                    SelInstr::new(self.mov, OpType::F32)
                        .dst(SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL })
                        .operand(
                            MOV_OPERAND,
                            SelSrc::Virtual {
                                v: got,
                                swizzle: [0, 1.min(lane), 2.min(lane), 3.min(lane)],
                                neg: false,
                                abs: false,
                            },
                        ),
                )
            }
            /* A local slot is a virtual register and a store to it is a write
             * to that register under the IR's own write mask -- which is what
             * `SelDst::Virtual` already expresses, so locals need no second
             * numbering and no slot file. They take virtual registers from the
             * same counter as everything else, reserved before selection
             * starts so a lowering's scratch cannot land on one.
             *
             * The write mask needs no translation: destination lane i takes
             * `src.swizzle[i]`, which is what this machine's encoding does.
             * Measured on `cube.vert`'s shipped blob, whose second instruction
             * is `mov t1.__zw, void, void, u0.xxxy` -- lane z takes swizzle[2]
             * and lane w swizzle[3], giving z = u0.x and w = u0.y. Consuming
             * the source's lanes in order across the set bits would give
             * w = u0.x, and a position with w = 0. */
            /* Its readers take the source directly, so the copy is not
             * emitted. Only when every read comes after it, which in
             * straight-line code is an index comparison. */
            ir::Op::StoreLocal { local, index: None, .. }
                if self.propagable_store(local).is_some_and(|(at, _)| at == id.0)
                    && self.every_read_after(local, id.0) =>
            {
                Ok(())
            }

            /* Renaming: the slot becomes a NAME for the operand the value
             * is, and nothing is emitted. See `slot_is_renamable`. */
            ir::Op::StoreLocal { local, index: None, src, .. } if self.slot_is_renamable(local) => {
                let s = self.src(src)?;
                self.locals_val[local.0 as usize] = Some(s);
                Ok(())
            }

            ir::Op::StoreLocal { local, index: None, src, write_mask } => {
                let v = self.local_reg(local);
                let s = self.src(src)?;
                /* The instruction that computed the value can write the slot
                 * itself, exactly as it can write an output. This is where
                 * most of the copies were: the front end gives every
                 * expression temporary a slot, so without it each one costs a
                 * `mov` AND a register that the mov's destination occupies.
                 * Measured over the corpus before this: 462 `mov`s against the
                 * reference's 95. */
                if self.retarget_last_to(SelDst::Virtual { v, write_mask }, src, &s) {
                    return Ok(());
                }
                note_site(|x| x.store_local += 1);
                /* The same question from the store side: this copy goes too,
                 * if the slot's components are tracked and they all come from
                 * one value. */
                if !self.slot_under_control_flow(local)
                    && !self.slot_is_renamable(local)
                    && self.masked_stores_share_one_value(local)
                {
                    note_site(|x| x.store_one_value += 1);
                }
                self.emit(
                    SelInstr::new(self.mov, OpType::F32).dst(SelDst::Virtual { v, write_mask }).operand(MOV_OPERAND, s),
                )
            }

            /* Folded into its uses by `src`, exactly as an attribute read
             * is, whenever the slot still holds what this load saw. The
             * condition has to be the same one `src` asks, or the load would
             * be both folded there and materialised here. */
            ir::Op::LoadLocal { local, index: None, .. }
                if self.locals_val.get(local.0 as usize).copied().flatten().is_some() =>
            {
                self.value_alias[id.0 as usize] = self.locals_val[local.0 as usize];
                Ok(())
            }

            ir::Op::LoadLocal { local, index: None, .. } if self.local_is_stable_after(local, id.0) => Ok(()),

            /* SNAPSHOTTED, not copied: nothing stores this slot after the
             * read, so the register it lives in right now still holds this
             * value when the consumer is selected, and the consumer can name
             * that register.
             *
             * `local_is_stable_after` cannot answer this. It refuses every
             * slot an arm touches -- 1,041 of our 2,120 copies come from
             * loads of those slots -- because it compares positions in the
             * instruction POOL, which is allocation order. This asks the same
             * question over the nodes, in program order, and takes the
             * register AS OF THE LOAD (`value_alias`) rather than at use
             * time, so an `if` that redirects the slot after this point
             * cannot change what the consumer reads.
             *
             * A slot a LOOP touches is excluded: its body runs again, so a
             * store before the read in the body is still a store after it. */
            ir::Op::LoadLocal { local, index: None, .. }
                if !self.slot_under_loop(local) && !self.stored_after(local, id) =>
            {
                let v = self.local_reg(local);
                self.value_alias[id.0 as usize] =
                    Some(SelSrc::Virtual { v, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false });
                note_site(|x| x.load_snapshot += 1);
                Ok(())
            }

            /* Materialised, because `src` will decline to fold it: the slot is
             * written again after this read, so the value has to be copied
             * before that happens. */
            ir::Op::LoadLocal { local, index: None, .. } => {
                let from = self.local_reg(local);
                note_site(|v| v.load_local += 1);
                /* WHY this load could not be folded, which is the same
                 * question as which fix would remove the copy. A slot under
                 * control flow needs a join; one with a masked store needs the
                 * slot's value tracked per component rather than as one
                 * operand. Counting them apart because they are different
                 * pieces of work and the totals have been read as one item. */
                if self.slot_under_control_flow(local) {
                    note_site(|v| v.load_local_cf += 1);
                    if self.slot_join_reachable(local) {
                        note_site(|v| v.load_local_joinable += 1);
                    }
                } else if !self.slot_is_renamable(local) {
                    note_site(|v| v.load_local_masked += 1);
                    if self.masked_stores_share_one_value(local) {
                        note_site(|v| v.load_one_value += 1);
                    }
                }
                self.emit(
                    SelInstr::new(self.mov, OpType::F32)
                        .dst(SelDst::Virtual { v: VReg(id.0), write_mask: build::WRITE_MASK_ALL })
                        .operand(
                            MOV_OPERAND,
                            SelSrc::Virtual { v: from, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false },
                        ),
                )
            }

            ir::Op::LoadLocal { .. } | ir::Op::StoreLocal { .. } => {
                Err(SelectError::Unsupported("an indexed local slot: arrays need a dynamic register index"))
            }
            /* IR v1's look-ups: the five ES 1.00 shapes select as before, one
             * to one (`Op::es100_tex`); every other shape is refused by name
             * until this back end selects it. */
            op @ ir::Op::Tex { sampler, coord, .. } => match (op.es100_tex(), three_lane_lookup(&op), fetch_level(&op))
            {
                (Some((kind, extra)), _, _) | (None, Some((kind, extra)), _) => {
                    self.tex(id, kind, sampler, coord, extra)
                }
                (None, None, Some(lod)) => self.tex_fetch(id, sampler, coord, lod),
                (None, None, None) => Err(SelectError::Unsupported(
                    "an ES 3.00 texture look-up (a shadow sampler, an integer result, a gradient, an offset or a \
                     cube fetch) is not selected yet",
                )),
            },
            ir::Op::TexSize { sampler, lod, .. } => self.tex_size(id, sampler, lod),
            /* `discard` is one `texkill`, with no operands and no
             * destination. Measured against the reference:
             *
             *   cmp                       the condition into a mask
             *   branch_unary.not -> 3     skip the kill when it is false
             *   texkill
             *   mov                       the colour
             *
             * The branch is `branch_convert`'s doing: an arm holding a
             * `Discard` is speculation-unsafe by construction -- running it on
             * the wrong side of the condition kills a live fragment -- so
             * `must_not_speculate` already routes it away from if-conversion. */
            ir::Op::Discard => {
                let kill = build::op_id("texkill").ok_or(SelectError::Unsupported("the machine has no `texkill`"))?;
                self.emit(SelInstr::new(kill, OpType::F32))
            }
        }
    }

    /// Does this arm hold control flow of its own?
    ///
    /// Only a LEAF `if` is if-converted. A nested one gets a real branch, and
    /// the reason is a measurement rather than a preference.
    ///
    /// If-conversion saves and restores every slot EITHER ARM WRITES, at every
    /// level. Nest it and the cost compounds: the outer `if` saves every slot
    /// its whole subtree writes, then the inner one saves them again. dEQP's
    /// `hsl_to_rgb` is six `if`s deep over nineteen slots, and we compiled it
    /// to **222 instructions in 60 temps** where the reference needs 50 in 10.
    /// Broken down, ours was 156 `mov` + 41 `select` + 25 real operations
    /// against the reference's 22 + 7 + 20: **the work was the same size and
    /// everything else was bookkeeping.**
    ///
    /// Measured over the wedge pair and the whole corpus, three arms:
    ///
    /// | | hsl VS temps | hsl VS instrs | corpus score |
    /// |---|---|---|---|
    /// | if-convert everything | 60 | 222 | 130 of 142 |
    /// | leaf only (this) | 13 | 84 | **137 of 142** |
    /// | never if-convert | 10 | 74 | 137 of 142 |
    ///
    /// Seven sources move from refused to compiled, all of them refused for a
    /// temps count no load-balancing band was measured over. Never
    /// if-converting scores the same and is two instructions cheaper again,
    /// and it is still the wrong choice: the reference if-converts -- its
    /// `hsl_to_rgb` has 7 `select`s and one branch -- so a leaf `if` is worth
    /// converting, and it is the nesting that is not.
    ///
    /// **This matters beyond quality.** 60 temps is above the 21 the band was
    /// measured over and, at 67, above the 6 bits the driver puts the count on
    /// the wire in. `hsl_to_rgb_vertex` is the shader that hangs the GC7000
    /// (task #52g); at 13 temps it is inside every one of those limits. That
    /// is not a claim that it now runs -- only the board says that -- but it
    /// is the producer side of it.
    fn nests_control_flow(&self, r: ir::Range) -> bool {
        /* The direct nodes only: an arm's statements ARE its nodes (the front
         * end flattens blocks), so an `if` two levels down is a direct node of
         * the arm between. */
        self.m.nodes_of(r).iter().any(|n| matches!(n, ir::Node::If { .. } | ir::Node::Loop { .. }))
    }

    /// Does this range hold anything that must not run speculatively?
    ///
    /// If-conversion runs BOTH arms and picks the answer afterwards, so an arm
    /// may only hold work that is safe to do on the wrong side of the
    /// condition. Measured against the reference compiler: arms of seven ALU
    /// operations each are if-converted, an arm holding a `texture2D` is not
    /// -- that one gets a real `branch_unary`. `Discard` is in the same class
    /// by construction: running it speculatively kills a live fragment.
    fn must_not_speculate(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            /* An output store too: if-conversion joins local SLOTS with a
             * select, and an output is not a slot. Both arms' stores ran one
             * after the other, so `if (c) gl_FragColor = a; else
             * gl_FragColor = b;` wrote b whatever c was, and
             * visualization.vert's projected gl_Position was never used. The
             * reference branches there; so does this. */
            ir::Node::Instr(id) => {
                matches!(self.m.instr(*id), Some(ir::Op::Tex { .. } | ir::Op::Discard | ir::Op::StoreOutput { .. }))
            }
            ir::Node::If { then_, else_, .. } => self.must_not_speculate(*then_) || self.must_not_speculate(*else_),
            ir::Node::Loop { body } => self.must_not_speculate(*body),
            /* A `break` is a jump. If-conversion runs both arms, so an arm
             * holding one would take the jump whatever the condition said --
             * the loop would stop on its first iteration, silently and with a
             * picture to show for it. Every loop the front end builds puts its
             * exit test in exactly this shape (`if (!cond) break;`), so this
             * arm is what makes loops work at all, not a corner case. */
            ir::Node::Break | ir::Node::Continue => true,
            _ => false,
        })
    }

    /// Does this range store that slot, at any depth?
    fn stores_local(&self, r: ir::Range, local: ir::LocalId) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            ir::Node::Instr(id) => {
                matches!(self.m.instr(*id), Some(ir::Op::StoreLocal { local: l, .. }) if *l == local)
            }
            ir::Node::If { then_, else_, .. } => self.stores_local(*then_, local) || self.stores_local(*else_, local),
            ir::Node::Loop { body } => self.stores_local(*body, local),
            _ => false,
        })
    }

    /// An `if` with a real branch, for an arm that may not be speculated.
    ///
    /// If-conversion runs both arms, so an arm holding a `texture2D` or a
    /// `discard` cannot use it -- the fetch would happen on the wrong side of
    /// the condition. This is the form the reference compiler emits for
    /// exactly those, measured:
    ///
    /// ```text
    ///   branch_unary.not cond -> L_else      skip the then arm when false
    ///   <then arm>
    ///   branch -> L_end
    /// L_else:
    ///   <else arm>
    /// L_end:
    /// ```
    ///
    /// `COND = 0xa` in the reference's `branch_unary` is [`Cond::Not`], which
    /// the enum already had, and `TARGET` is an absolute instruction index.
    ///
    /// **This is simpler than if-conversion, not harder**, and the reason is
    /// worth stating: only one arm runs, so a slot each arm writes simply
    /// holds whichever value ran. None of if-conversion's save, restore and
    /// select per slot is needed. The cost is the branch itself and the
    /// divergence, which is why it is used only where speculation is unsafe
    /// rather than everywhere.
    ///
    /// An `if` with no `else` needs no unconditional jump: the conditional
    /// branch goes straight to the end.
    fn branch_convert(&mut self, cond: ir::Src, then_: ir::Range, else_: ir::Range) -> Result<(), SelectError> {
        /* The condition is ONE lane, and the branch is fed that lane in all
         * four swizzle positions: a branch reading other lanes decides on
         * them too. Every `branch_unary` of the reference programs reads a
         * splat (tests/branch_splat.rs pins ours). */
        let cond_src = splat_first_lane(self.src(cond)?);
        let br_un =
            build::op_id("branch_unary").ok_or(SelectError::Unsupported("the machine has no `branch_unary`"))?;
        let br = build::op_id("branch").ok_or(SelectError::Unsupported("the machine has no `branch`"))?;

        /* Skip the then arm when the condition is false. The target is a
         * placeholder: a forward branch cannot know its destination until the
         * arm it jumps over has been emitted. */
        let mut test = SelInstr::new(br_un, OpType::U32);
        test.cond = Some(Cond::Not);
        self.emit(test.operand(0, cond_src).target(0))?;
        let at_test = self.instrs.len() - 1;

        self.nodes(then_)?;

        let at_jump = if else_.len == 0 {
            None
        } else {
            /* The reference's unconditional `branch` carries no TYPE bits
             * (type 0 = F32) while its `branch_unary` carries U32; measured,
             * not assumed. */
            self.emit(SelInstr::new(br, OpType::F32).target(0))?;
            Some(self.instrs.len() - 1)
        };

        let else_starts = self.instrs.len();
        self.patch_target(at_test, else_starts)?;

        self.nodes(else_)?;

        if let Some(j) = at_jump {
            let ends = self.instrs.len();
            self.patch_target(j, ends)?;
        }
        Ok(())
    }

    /// A loop: the body, then one branch back to its first instruction.
    ///
    /// ```text
    /// L_top:
    ///   <body>            the exit test is inside it, as `if (!cond) break;`
    ///   branch -> L_top   the back edge
    /// L_end:
    /// ```
    ///
    /// There is no loop instruction and no counter on this machine, so the
    /// shape is the same `branch` an `if` uses, pointed backwards. That is the
    /// one branch in a program allowed to point backwards, and it says so by
    /// index in [`Selected::back_edges`]; `abi::assemble` refuses any other.
    /// The permission is per branch and not per program on purpose -- a second
    /// backward jump in a shader that has a loop is still a program that may
    /// never end, and on this board that wedges the GPU with no way back but a
    /// reset.
    ///
    /// **Nothing here bounds the trip count**, and nothing can: the test is
    /// the source's own. What this does guarantee is that the only way back is
    /// the one the source asked for.
    ///
    /// A `break` is an unconditional branch whose destination is `L_end`,
    /// which does not exist yet when it is emitted, so it is recorded and
    /// patched when the loop closes -- the same back-patch `branch_convert`
    /// uses, over a list because a loop may hold several.
    fn loop_convert(&mut self, before: &[ir::Node], body: ir::Range) -> Result<(), SelectError> {
        /* A loop that cannot leave is a program that never ends, and a program
         * that never ends on this GPU is a wedge the driver recovers from only
         * by resetting it -- a lost frame, which on a cluster is a safety
         * event. Every loop the ES 1.00 front end builds carries its exit test
         * as a `break` (`while`, `do`/`while` and `for` all go through
         * `break_unless`), so a body with none is either `for(;;)` with no way
         * out, or a shape this file has not met. Refusing is the difference
         * between a gap someone can read and a board nobody can reach. */
        if !self.can_break(body) {
            return Err(SelectError::Unsupported(
                "a loop with no `break`: it would never end, and a shader that never ends wedges this GPU",
            ));
        }
        /* A `break` is not enough: a bound read at run time can still keep
         * the loop going for longer than the GPU is allowed to run (gl_test
         * 104's band 7, a 1e9 bound per pixel, until a reset). So every loop
         * also carries a hidden counter of its head entries and leaves once it
         * passes `limit`. For a constant `for` loop the limit is its own entry
         * count, so the guard never fires and the loop stays exact; for any
         * other it is LOOP_CAP, the PowerVR's and the V3D's number.
         *
         * ```text
         *   mov   cnt.x, 0
         * L_top:                       (continue and the back edge land here)
         *   add   cnt.x, cnt.x, 1
         *   cmp.ge stop.x, cnt.x, limit + 1
         *   branch.not stop.x -> L_body
         *   branch -> L_end
         * L_body:
         *   <body>
         *   branch -> L_top
         * L_end:
         * ```
         *
         * At the head rather than at the end, so a `continue` (a jump back to
         * L_top) is counted too: a loop whose every pass continues would
         * otherwise never reach the counter. Only operations selection already
         * emits: an integer `add`, `cmp` and `branch_unary` with `Not`, as
         * every `break`. */
        let m = self.m;
        let limit = super::loop_count::constant_loop_entries(m, body, &|i, float| {
            super::loop_count::init_from_preceding(m, before, i, float)
        })
        .unwrap_or(LOOP_CAP);
        self.loop_limits.push(self.arena, limit)?;
        let br = build::op_id("branch").ok_or(SelectError::Unsupported("the machine has no `branch`"))?;
        let br_un =
            build::op_id("branch_unary").ok_or(SelectError::Unsupported("the machine has no `branch_unary`"))?;
        let add = build::op_id("add").ok_or(SelectError::Unsupported("the machine has no `add`"))?;
        let cmp = build::op_id("cmp").ok_or(SelectError::Unsupported("the machine has no `cmp`"))?;
        let (cnt, stop) = (VReg(self.next_vreg), VReg(self.next_vreg + 1));
        self.next_vreg += 2;
        let x = |v: VReg| SelSrc::Virtual { v, swizzle: [0; 4], neg: false, abs: false };
        let zero = self.int_literal_src([0; 4], ir::SWIZZLE_IDENTITY)?;
        let one = self.int_literal_src([1; 4], ir::SWIZZLE_IDENTITY)?;
        let past = self.int_literal_src([limit + 1; 4], ir::SWIZZLE_IDENTITY)?;
        let all_ones = self.int_literal_src([u32::MAX; 4], ir::SWIZZLE_IDENTITY)?;
        /* The copy in the reference's form (`legal_copy`: `mov.f32` of the
         * bits; it never emits `mov.s32`). */
        let (mov_ty, zero) = legal_copy(OpType::S32, zero);
        self.emit(
            SelInstr::new(self.mov, mov_ty).dst(SelDst::Virtual { v: cnt, write_mask: 0x1 }).operand(MOV_OPERAND, zero),
        )?;

        let top = self.instrs.len();
        self.emit(
            SelInstr::new(add, OpType::S32)
                .dst(SelDst::Virtual { v: cnt, write_mask: 0x1 })
                .operand(0, x(cnt))
                .operand(1, one),
        )?;
        let mut ge = SelInstr::new(cmp, OpType::S32);
        ge.cond = Some(Cond::Ge);
        self.emit(
            ge.dst(SelDst::Virtual { v: stop, write_mask: 0x1 })
                .operand(0, x(cnt))
                .operand(1, past)
                .operand(2, all_ones),
        )?;
        let mut go_on = SelInstr::new(br_un, OpType::U32);
        go_on.cond = Some(Cond::Not);
        self.emit(go_on.operand(0, x(stop)).target(0))?;
        let at_go_on = self.instrs.len() - 1;
        self.emit(SelInstr::new(br, OpType::F32).target(0))?;
        let at_leave = self.instrs.len() - 1;
        let body_starts = self.instrs.len();
        self.patch_target(at_go_on, body_starts)?;

        /* Breaks belonging to an enclosing loop are below this mark and are
         * left alone. The guard's own exit is one of this loop's. */
        let mark = self.breaks.len();
        self.breaks.push(self.arena, at_leave)?;

        self.loop_tops.push(self.arena, top)?;
        let r = self.nodes(body);
        self.loop_tops.pop();
        r?;

        self.emit(SelInstr::new(br, OpType::F32).target(0))?;
        let at_back = self.instrs.len() - 1;
        self.patch_target(at_back, top)?;
        self.back_edges.push(self.arena, at_back)?;

        let end = self.instrs.len();
        while self.breaks.len() > mark {
            let at = self.breaks.as_slice()[self.breaks.len() - 1];
            self.breaks.pop();
            self.patch_target(at, end)?;
        }
        Ok(())
    }

    /// Can this loop body reach a `break` of its own?
    ///
    /// A nested loop's `break` leaves the INNER loop, so this does not descend
    /// into one -- counting it would accept an outer loop that never ends.
    fn can_break(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            ir::Node::Break => true,
            ir::Node::If { then_, else_, .. } => self.can_break(*then_) || self.can_break(*else_),
            _ => false,
        })
    }

    /// `continue`: a jump back to the top of the innermost loop -- a second
    /// BACK EDGE of that loop, and listed as one, both for `abi::assemble`
    /// (which refuses any backward branch it is not told about) and for the
    /// allocator, whose liveness must carry every value live around the loop
    /// across it as well. Whatever runs before the next iteration -- a `for`
    /// step, a `do`/`while` test -- the front end has already emitted in
    /// front of the `Continue` (lower.rs, `Stmt::Continue`); a `do`/`while`
    /// test skipped here would be a loop that never ends.
    fn continue_loop(&mut self) -> Result<(), SelectError> {
        let br = build::op_id("branch").ok_or(SelectError::Unsupported("the machine has no `branch`"))?;
        let Some(&top) = self.loop_tops.as_slice().last() else {
            return Err(SelectError::Unsupported("a `continue` outside a loop"));
        };
        self.emit(SelInstr::new(br, OpType::F32).target(0))?;
        let at = self.instrs.len() - 1;
        self.patch_target(at, top)?;
        self.back_edges.push(self.arena, at)?;
        Ok(())
    }

    /// `break`: a jump past the loop's back edge, patched by `loop_convert`.
    fn break_out(&mut self) -> Result<(), SelectError> {
        let br = build::op_id("branch").ok_or(SelectError::Unsupported("the machine has no `branch`"))?;
        self.emit(SelInstr::new(br, OpType::F32).target(0))?;
        let at = self.instrs.len() - 1;
        self.breaks.push(self.arena, at)?;
        Ok(())
    }

    /// Fill in a forward branch's destination once it is known.
    fn patch_target(&mut self, at: usize, to: usize) -> Result<(), SelectError> {
        if to > MAX_BRANCH_TARGET as usize {
            return Err(SelectError::Unsupported("a branch target beyond the fifteen bits the encoding carries"));
        }
        self.instrs.as_mut_slice()[at].target = Some(to as u16);
        Ok(())
    }

    /// An `if` without a branch: run both arms, then choose.
    ///
    /// This is what the reference compiler does, measured rather than assumed
    /// -- see `docs/online-shaderc/backend.md`. It emits no branch for an
    /// `if`/`else`, for arms of seven operations each, or for a nested `if`;
    /// it branches only when an arm holds a texture fetch.
    ///
    /// Values do not cross an arm boundary: the IR contract puts anything that
    /// does into a `Local` slot. So the only state to reconcile is the slots,
    /// and the shape is:
    ///
    /// ```text
    ///   mov saved[l],  slot[l]        for every slot either arm writes
    ///   <then arm>                    stores land in slot[l] as usual
    ///   mov then[l],   slot[l]
    ///   mov slot[l],   saved[l]       put it back for the else arm
    ///   <else arm>
    ///   select.nz slot[l], cond, then[l], slot[l]
    /// ```
    ///
    /// The else arm may be empty, and then `slot[l]` still holds `saved[l]` --
    /// the pre-`if` value -- which is exactly the right answer for an `if`
    /// with no `else`, with no special case for it.
    ///
    /// The copies are the price of correctness before liveness: the reference
    /// gets the same result in one `select` because it knows which registers
    /// are dead. This does not, and a wrong register here draws a picture
    /// rather than failing, so it copies.
    fn if_convert(&mut self, cond: ir::Src, then_: ir::Range, else_: ir::Range) -> Result<(), SelectError> {
        if self.must_not_speculate(then_) || self.must_not_speculate(else_) {
            return self.branch_convert(cond, then_, else_);
        }
        /* An arm holding another `if` branches instead. See
         * `nests_control_flow`: if-converting one costs every enclosing level
         * a save and a restore of every slot the whole subtree writes, and it
         * is where the 60 temps came from. */
        if self.nests_control_flow(then_) || self.nests_control_flow(else_) {
            return self.branch_convert(cond, then_, else_);
        }
        /* The condition is computed before the `if`, in straight-line code, so
         * it is resolved once here and read by every `select` below. */
        let cond = self.src(cond)?;

        /* Two scratch registers per slot, numbered rather than remembered:
         * this crate has no allocator, and reserving a block keeps the three
         * passes below able to name the same registers without carrying a map
         * between them. Slots are few, and by the rule above the arms hold no
         * `if` of their own, so a block is never reserved twice over. */
        let n_locals = self.m.locals.len() as u32;
        let base = self.next_vreg;
        let whole = |v: VReg| SelSrc::Virtual { v, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false };

        let written = |s: &Self, l: u32| {
            let id = ir::LocalId(l);
            s.stores_local(then_, id) || s.stores_local(else_, id)
        };
        /* Numbered by POSITION AMONG THE WRITTEN SLOTS, not by slot index.
         * Reserving two registers for every declared slot instead cost 14 of
         * the sources this was meant to unblock: the allocator's 256 registers
         * ran out before the code did. The rank is recomputed rather than
         * stored because there is no allocator here and the counts are tiny. */
        let rank = |s: &Self, l: u32| (0..l).filter(|k| written(s, *k)).count() as u32;
        let n_written = rank(self, n_locals);
        /* ONE register per written slot, either way. Only the THEN arm is
         * redirected: by the time the else arm runs, the slot still holds the
         * value from before the `if`, because the then arm wrote somewhere
         * else. So the else arm can write the slot itself, needs no copy in
         * and needs none out, and the select's false operand is the slot.
         *
         * Redirecting both arms works too and costs a register per slot and a
         * longer live range for nothing: measured over the corpus, 1,778
         * temps against 1,736 for this. */
        let no_else = else_.len == 0;
        self.next_vreg += n_written;
        let then_val = |s: &Self, l: u32| VReg(base + rank(s, l));

        /* The then arm writes its OWN register for every slot it touches, the
         * else arm writes the slot, and the join picks:
         *
         * ```text
         *   mov then[l], slot[l]        only when the THEN arm can READ it
         *   <then arm>                  its stores land in then[l]
         *   <else arm>                  its stores land in slot[l], which
         *                               still holds the pre-`if` value
         *   select.nz slot[l], cond, then[l], slot[l]
         * ```
         *
         * This used to save the slot, run the then arm, copy the result out
         * and put the saved value back before the else arm -- three `mov`s per
         * written slot, unconditional, whatever the arms did. Redirecting the
         * then arm makes all but the first unnecessary, and the first is only
         * needed when that arm can observe the slot's old value: it reads the
         * slot, or it writes only some of its lanes and the rest must survive.
         * Measured over the corpus, most arms need neither.
         *
         * An `if` with no `else` falls out of the same shape with no special
         * case: nothing wrote the slot, so the false operand is already right.
         */
        /* The then arm's register has to start as a copy of the slot unless
         * that arm defines the whole of it itself. Two ways it does not:
         *
         * * it READS the slot, or writes only some lanes -- the old
         *   condition, about the arm observing the value;
         * * **it does not write the slot at all**, which happens whenever the
         *   ELSE arm writes a slot the then arm leaves alone. `written` is
         *   true for the pair of arms, so a register is reserved and the join
         *   selects from it; without a seed that register is never written,
         *   and the join picks an UNDEFINED register on the true path.
         *
         * The second was missing, and it is not an edge case: any
         * `if (c) { x = ...; } else { y = ...; }` hits it for `y`. Repro and
         * measurement in `an_arm_that_writes_nothing_still_needs_the_slots_value`.
         */
        let needs_seed = |s: &Self, r: ir::Range, l: u32| {
            s.reads_or_partly_writes(r, ir::LocalId(l)) || !s.stores_local(r, ir::LocalId(l))
        };

        for l in 0..n_locals {
            if written(self, l) && needs_seed(self, then_, l) {
                let from = whole(self.local_reg(ir::LocalId(l)));
                note_site(|v| v.if_seed += 1);
                self.emit(
                    SelInstr::new(self.mov, OpType::F32)
                        .dst(SelDst::Virtual { v: then_val(self, l), write_mask: build::WRITE_MASK_ALL })
                        .operand(MOV_OPERAND, from),
                )?;
            }
        }
        for l in 0..n_locals {
            if written(self, l) {
                self.locals_in[l as usize] = then_val(self, l);
            }
        }
        self.nodes(then_)?;
        for l in 0..n_locals {
            self.locals_in[l as usize] = Self::local_home(self.m, l);
        }

        if !no_else {
            self.nodes(else_)?;
        }

        /* The condition is ONE lane, and every lane of every select must be
         * decided by it -- the same splat `branch_convert` gives a branch.
         * Fed through as it was, lane c of a slot was chosen by lane c of the
         * condition's register, which only in lane x held the comparison:
         * dEQP loops.*.conditional_body (`do if (i < 2) res = res.yzwx;
         * while (++i < 3)`) rotated lanes y..w on every trip, because lanes
         * y..w of the compare saw 0 < 2 (imx8mp-board, e31bd3e, 10 cases). */
        let cond = splat_first_lane(cond);
        for l in 0..n_locals {
            if written(self, l) {
                let slot = self.local_reg(ir::LocalId(l));
                let dst = SelDst::Virtual { v: slot, write_mask: build::WRITE_MASK_ALL };
                /* The false answer is the slot: whatever the else arm wrote,
                 * or -- with no else arm -- the value from before the `if`,
                 * which is still there because the then arm wrote elsewhere. */
                self.lower_one(ir::AluOp::Bcsel, dst, &[cond, whole(then_val(self, l)), whole(slot)])?;
            }
        }
        Ok(())
    }

    /// Can this arm observe the slot's value from before the `if`?
    ///
    /// Two ways: it reads the slot, or it writes only some of the lanes and
    /// the others have to survive. Either means the arm's own register has to
    /// start as a copy of the slot; otherwise it starts undefined and nothing
    /// notices, because every lane the arm writes is correct and the ones it
    /// does not are the bug.
    fn reads_or_partly_writes(&self, r: ir::Range, local: ir::LocalId) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            ir::Node::Instr(id) => match self.m.instr(*id) {
                Some(ir::Op::LoadLocal { local: l, .. }) => *l == local,
                Some(ir::Op::StoreLocal { local: l, write_mask, .. }) => {
                    *l == local && *write_mask != build::WRITE_MASK_ALL
                }
                _ => false,
            },
            /* By `nests_control_flow` an if-converted arm holds no control
             * flow, so there is nothing to recurse into. Answering `true` for
             * anything else keeps that an assumption this cannot be wrong
             * about. */
            _ => true,
        })
    }

    /// Emit one lowered operation, the same way [`Select::alu`] does.
    fn lower_one(&mut self, op: ir::AluOp, dst: SelDst, srcs: &[SelSrc]) -> Result<(), SelectError> {
        let ty = op_type(op).ok_or(SelectError::Unsupported("an operation with no settled type"))?;
        let arena = self.arena;
        let mov = self.mov;
        let instrs = &mut self.instrs;
        let next = core::cell::Cell::new(self.next_vreg);
        let mut push_err = None;
        let r = {
            let mut out = |i: SelInstr| {
                if push_err.is_none() {
                    push_err = push_one_constant_register(instrs, arena, &next, mov, i).err();
                }
            };
            let mut fresh = || {
                let v = VReg(next.get());
                next.set(next.get() + 1);
                v
            };
            builtins::lower(op, ty, dst, srcs, &mut out, &mut fresh)
        };
        self.next_vreg = next.get();
        r?;
        match push_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    /// Delete instructions nothing reads.
    ///
    /// **25 of the corpus's 305 immediate seeds are never read at all**
    /// (`last_seed_breakdown`): a literal materialised into a
    /// register that no later instruction names. They are not a folding
    /// problem, they are dead code, and this is the cheapest item on either of
    /// our lists because it needs no reasoning about the instruction set at
    /// all.
    ///
    /// What may be deleted is narrow on purpose:
    ///
    /// * only a write to a VIRTUAL register -- a write to a fixed one is an
    ///   output or an address, and the fact that this file cannot see a read
    ///   of it is exactly why it must not be removed;
    /// * never a `texkill`, which kills the fragment and writes nothing, and
    ///   never a branch, whose effect is the jump;
    /// * only when no surviving instruction reads that register.
    ///
    /// Iterated to a fixed point, because deleting one write can make the
    /// instruction that produced ITS operand dead in turn. The corpus
    /// converges in two passes; the loop is bounded by the instruction count
    /// so a bug cannot hang the compiler.
    fn eliminate_dead(&mut self) -> Result<(), SelectError> {
        let n = self.instrs.len();
        if n == 0 {
            return Ok(());
        }
        let dead = self.arena.alloc_filled(n, false)?;
        let mut again = true;
        let mut rounds = 0;
        while again && rounds <= n {
            again = false;
            rounds += 1;
            for i in 0..n {
                if dead[i] {
                    continue;
                }
                let ins = self.instrs.as_slice()[i];
                /* A side effect is not a value: `texkill` and every branch
                 * stay whatever else is true of them. */
                if ins.target.is_some() || ins.dst.is_none() {
                    continue;
                }
                let Some(SelDst::Virtual { v, .. }) = ins.dst else { continue };
                let read = (0..n).filter(|j| !dead[*j] && *j != i).any(|j| {
                    self.instrs.as_slice()[j]
                        .srcs
                        .iter()
                        .flatten()
                        .any(|r| matches!(r, SelSrc::Virtual { v: rv, .. } if *rv == v))
                });
                if !read {
                    dead[i] = true;
                    again = true;
                }
            }
        }
        let gone = dead.iter().filter(|d| **d).count();
        if gone == 0 {
            return Ok(());
        }
        note_site(|k| k.dead_deleted += gone);
        self.compact(dead)
    }

    /// Delete `mov d, s` by making every later read of `d` read `s` instead.
    ///
    /// Measured before writing it: over the 119 sources that compile, we emit
    /// **2,625 `mov`s in 5,146 instructions -- 51%** -- against the
    /// reference's 258 in 2,060, which is 12%. Most of ours are the price of
    /// selecting each IR value into its own register and then copying it where
    /// it is wanted, and a copy whose destination is read and never rewritten
    /// is exactly the one nobody needs.
    ///
    /// Four conditions, and each of them is load-bearing:
    ///
    /// 1. The copy is a **whole register between two virtuals**, with no
    ///    condition, saturate, texture or modifier. A partial write leaves the
    ///    other lanes alone, so the destination is not simply the source.
    /// 2. `d` is **written exactly once**, here. Otherwise a later write
    ///    changes what the propagated reads should have seen.
    /// 3. `s` is **never written after** this point. A local slot's register
    ///    is written repeatedly, so this is what stops a slot's later value
    ///    being read where its earlier one was meant -- the same unsoundness
    ///    `local_is_stable_after` guards, one level down.
    /// 4. `d` is **never read before** this point. A slot can be, and reading
    ///    it there means its earlier value, which `s` is not.
    ///
    /// The reader's own swizzle and modifiers are preserved: the composed
    /// swizzle is the copy's read through the reader's, and negate and
    /// absolute belong to the reader and are carried across unchanged.
    fn propagate_copies(&mut self) -> Result<(), SelectError> {
        let n = self.instrs.len();
        if n < 2 {
            return Ok(());
        }
        let dead = self.arena.alloc_filled(n, false)?;

        for i in 0..n {
            let (d, s, sw) = {
                let ins = self.instrs.as_slice()[i];
                if ins.op != self.mov || ins.cond.is_some() || ins.saturate || ins.tex.is_some() {
                    continue;
                }
                let Some(SelDst::Virtual { v: d, write_mask }) = ins.dst else { continue };
                if write_mask != build::WRITE_MASK_ALL {
                    continue;
                }
                let mut only = None;
                let mut count = 0;
                for sl in ins.srcs.iter().flatten() {
                    count += 1;
                    only = Some(*sl);
                }
                if count != 1 {
                    continue;
                }
                let Some(SelSrc::Virtual { v: s, swizzle, neg: false, abs: false }) = only else { continue };
                if d == s {
                    continue;
                }
                (d, s, swizzle)
            };

            let mut ok = true;
            for (j, ins) in self.instrs.as_slice().iter().enumerate() {
                if dead[j] {
                    continue;
                }
                if let Some(SelDst::Virtual { v, .. }) = ins.dst {
                    /* (2) another write of `d`, and (3) a write of `s` after. */
                    if (v == d && j != i) || (v == s && j > i) {
                        ok = false;
                        break;
                    }
                }
                /* (4) a read of `d` before the copy. */
                if j < i && ins.srcs.iter().flatten().any(|r| matches!(r, SelSrc::Virtual { v, .. } if *v == d)) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }

            let slice = self.instrs.as_mut_slice();
            for (j, item) in slice.iter_mut().enumerate().take(n).skip(i + 1) {
                if dead[j] {
                    continue;
                }
                for slot in item.srcs.iter_mut() {
                    let Some(SelSrc::Virtual { v, swizzle: outer, neg, abs }) = *slot else { continue };
                    if v != d {
                        continue;
                    }
                    let base = SelSrc::Virtual { v: s, swizzle: sw, neg: false, abs: false };
                    if let SelSrc::Virtual { v: nv, swizzle: ns, .. } = compose(base, outer) {
                        *slot = Some(SelSrc::Virtual { v: nv, swizzle: ns, neg, abs });
                    }
                }
            }
            dead[i] = true;
        }

        self.compact(dead)
    }

    /// Delete `mov d, <uniform or literal>` by making every reader name the
    /// constant directly.
    ///
    /// `src()` already folds a uniform read and a literal into the operand
    /// that uses them, so these seeds are what is LEFT: a value that had to be
    /// materialised because something between the definition and the use --
    /// a local slot, an if-conversion, a lowering's scratch -- wanted a
    /// register. Measured on my own tree before writing this: 426 of our 2,424
    /// `mov`s are seeds (156 uniform, 270 immediate), and 210 of them are read
    /// at most once.
    ///
    /// # What decides whether a fold is allowed
    ///
    /// Two independent limits, both measured from the reference compiler's
    /// 13,908 instructions rather than assumed (`tests/operand_banks.rs`):
    ///
    /// 1. **How many operands of one register group an instruction holds** --
    ///    temp 3, uniform 3, immediate 2, internal 1, and `uniform_1` and
    ///    `thread` never appear at all. A fold that would exceed one is
    ///    declined, because an instruction the encoding has no room for is a
    ///    shader that assembles and draws something else.
    /// 2. **Which OPERATIONS take an operand of that group**
    ///    (`OpId::takes_group`). The reference gives `texld` 568 source
    ///    operands and every one is a temporary; a branch's condition is
    ///    always a temporary too. Those are refused.
    ///
    /// # And what makes it sound
    ///
    /// The soundness conditions are `propagate_copies`'s, with one relaxed
    /// and one added:
    ///
    /// * `d` is written exactly once, here, and never read before it. Both as
    ///   there, and for the same reasons.
    /// * "the source is not written after this point" is **vacuous** here: the
    ///   constant file is written by the driver before the draw and an
    ///   immediate lives inside the instruction, so neither can change under
    ///   a reader. That is the whole reason this is a separate pass rather
    ///   than a case of that one.
    /// * an **indexed** uniform read (`amode != 0`) is refused: it depends on
    ///   the address register, which the shader DOES write (`movar`), so
    ///   moving the read past one would read a different uniform.
    ///
    /// An immediate needs one more rule of its own. Its `swizzle`, `neg` and
    /// `abs` fields are not modifiers, they are the value's payload, so a
    /// reader's negate or absolute cannot be carried into it and such a
    /// reader is refused; a reader's SWIZZLE is dropped rather than composed,
    /// which is sound because an immediate is one scalar broadcast to all
    /// four lanes. `compose` says the same thing from the other side, and a
    /// test pins the number.
    fn fold_constant_reads(&mut self) -> Result<(), SelectError> {
        /* Why a seed was left alone. Each is a size for the next item, and
         * the reasons are not interchangeable: an over-cap decline is the
         * machine's limit, a wrong-operation decline is the reference's
         * silence, and an unsound one is this pass refusing to move a value. */
        enum Why {
            OverCap,
            WrongOp,
            Modifier,
            Unsound,
        }

        let n = self.instrs.len();
        if n < 2 {
            return Ok(());
        }
        let dead = self.arena.alloc_filled(n, false)?;

        for i in 0..n {
            let (d, p) = {
                let ins = self.instrs.as_slice()[i];
                if ins.op != self.mov || ins.cond.is_some() || ins.saturate || ins.tex.is_some() {
                    continue;
                }
                /* ANY write mask, unlike `propagate_copies`. `d` is written
                 * exactly once (checked below), so the lanes this seed does
                 * not write were never written at all: a reader of them reads
                 * an undefined register, and reading the constant's lane
                 * there instead is a refinement, not a change. The lanes it
                 * does write are the constant's, lane for lane. Refusing the
                 * masked seed kept every `vec2`/`vec3` literal local in a
                 * register of its own: dEQP tmp_array
                 * vec2/vec3_const_write_static_loop_read_vertex held 36
                 * `mov t.xy, k` seeds live across the loop, 45 temporaries
                 * against the vertex ceiling of 21, where the vec4 variant
                 * folds them into immediates and compiles. */
                let Some(SelDst::Virtual { v: d, .. }) = ins.dst else { continue };
                let mut only = None;
                let mut count = 0;
                for sl in ins.srcs.iter().flatten() {
                    count += 1;
                    only = Some(*sl);
                }
                if count != 1 {
                    continue;
                }
                let Some(SelSrc::Physical(p)) = only else { continue };
                /* Only a place the shader itself never writes. */
                let constant = p.group == build::RGROUP_UNIFORM_0 || p.group == build::RGROUP_IMMED;
                if !constant {
                    continue;
                }
                if !p.is_immediate() {
                    /* On a uniform these ARE modifiers; a seed carries none,
                     * and one that did would have to compose with every
                     * reader's. Indexed reads are refused above. */
                    if p.neg || p.abs || p.amode != 0 {
                        continue;
                    }
                }
                (d, p)
            };

            let mut why = None;
            let mut readers = 0usize;
            for (j, ins) in self.instrs.as_slice().iter().enumerate() {
                if dead[j] {
                    continue;
                }
                /* Another write of `d`: a later one would change what the
                 * propagated reads should have seen. */
                if let Some(SelDst::Virtual { v, .. }) = ins.dst {
                    if v == d && j != i {
                        why = Some(Why::Unsound);
                        break;
                    }
                }
                let reads = ins.srcs.iter().flatten().any(|r| matches!(r, SelSrc::Virtual { v, .. } if *v == d));
                if !reads {
                    continue;
                }
                /* A read BEFORE the copy is a read of whatever was there. */
                if j < i {
                    why = Some(Why::Unsound);
                    break;
                }
                readers += 1;
                if !ins.op.takes_group(p.group) {
                    why = Some(Why::WrongOp);
                    break;
                }
                if p.is_immediate()
                    && ins
                        .srcs
                        .iter()
                        .flatten()
                        .any(|r| matches!(r, SelSrc::Virtual { v, neg, abs, .. } if *v == d && (*neg || *abs)))
                {
                    why = Some(Why::Modifier);
                    break;
                }
                let mut after = ins.srcs;
                for slot in after.iter_mut() {
                    if matches!(*slot, Some(SelSrc::Virtual { v, .. }) if v == d) {
                        *slot = Some(SelSrc::Physical(p));
                    }
                }
                if !within_caps(&after) {
                    why = Some(Why::OverCap);
                    break;
                }
            }
            if let Some(w) = why {
                note_site(|k| match w {
                    Why::OverCap => k.fold_over_cap += 1,
                    Why::WrongOp => k.fold_wrong_op += 1,
                    Why::Modifier => k.fold_modifier += 1,
                    Why::Unsound => k.fold_unsound += 1,
                });
                continue;
            }
            /* A seed nothing reads is dead code, which `eliminate_dead` owns:
             * deleting it here would make this pass's numbers include work it
             * did not do. */
            if readers == 0 {
                continue;
            }

            let slice = self.instrs.as_mut_slice();
            for (j, item) in slice.iter_mut().enumerate().take(n).skip(i + 1) {
                if dead[j] {
                    continue;
                }
                for slot in item.srcs.iter_mut() {
                    let Some(SelSrc::Virtual { v, swizzle: outer, neg, abs }) = *slot else { continue };
                    if v != d {
                        continue;
                    }
                    *slot = Some(if p.is_immediate() {
                        /* One value in all four lanes, so the reader's lane
                         * choice is the identity on it; its negate and
                         * absolute were refused above. */
                        SelSrc::Physical(p)
                    } else {
                        let mut folded = p;
                        folded.swizzle = [
                            p.swizzle[outer[0] as usize & 3],
                            p.swizzle[outer[1] as usize & 3],
                            p.swizzle[outer[2] as usize & 3],
                            p.swizzle[outer[3] as usize & 3],
                        ];
                        /* The reader's own modifiers, not the seed's: the
                         * seed has none, checked above. */
                        folded.neg = neg;
                        folded.abs = abs;
                        SelSrc::Physical(folded)
                    });
                }
            }
            dead[i] = true;
            note_site(|k| k.folded += 1);
        }

        self.compact(dead)
    }

    /// Put an operand the operation may not take into a register first.
    ///
    /// `src()` reads a uniform or a literal straight into the operand that
    /// uses it, which is the substance of selection on this machine -- but it
    /// does it for EVERY operation, and the reference compiler does not.
    /// Across its 13,908 instructions a texture instruction's coordinate is
    /// always a temporary (568 of 568) and so is a branch's condition (208 of
    /// 208), while `mad` takes 2,912 uniform operands and 221 immediates.
    /// `OpId::takes_group` holds that measurement.
    ///
    /// So `if (u_flag > 0.0) { texture(...); }` -- an `if` whose arm may not
    /// be speculated, which selects a real branch -- would give that branch a
    /// constant-file condition, and no shipped shader does. This materialises
    /// it into a temporary instead, which is exactly what the reference
    /// emits.
    ///
    /// Two things this is NOT:
    ///
    /// * not a refusal. Refusing would lose a shader over a rule the corpus
    ///   supports but does not prove; materialising costs one instruction and
    ///   cannot be wrong.
    /// * not a fix for anything measured to be broken. It does not happen in
    ///   our corpus (0 of 137 sources put a constant in a branch; the 18
    ///   `rcp`s that read an immediate are the only operands we emit that the
    ///   reference never does, and those are the front end's un-folded
    ///   `1.0 / literal`). It is a guard for shaders we have not compiled.
    ///
    /// Runs AFTER the fold, which would otherwise put the constant straight
    /// back; the two rules are the same rule, so `fold_constant_reads`
    /// declines exactly what this would have to undo.
    ///
    /// **Inserting instructions moves branch targets the same way deleting
    /// them does**, so the map is built here too, and a target that named an
    /// instruction now names the first of the moves in front of it -- the
    /// jump has to land where the operand is materialised, not after it.
    fn legalise_operands(&mut self) -> Result<(), SelectError> {
        let n = self.instrs.len();
        let takes = |op: build::OpId, s: &SelSrc| op.takes_group(group_of(s));
        let needs = |ins: &SelInstr| ins.srcs.iter().flatten().any(|s| !takes(ins.op, s));
        if !self.instrs.as_slice().iter().any(needs) {
            return Ok(());
        }
        let moved_to = self.arena.alloc_filled(n + 1, 0usize)?;
        /* Sized once: one `mov` per operand an operation cannot take, and the
         * instructions themselves. Grown by doubling in a bump arena, every
         * abandoned block stayed allocated -- on uniform_api.random.23 this
         * pass took 514 KB of scratch for a 175 KB list. */
        let moves: usize = self
            .instrs
            .as_slice()
            .iter()
            .map(|ins| ins.srcs.iter().flatten().filter(|s| !takes(ins.op, s)).count())
            .sum();
        let mut out: AVec<'a, SelInstr> = AVec::with_capacity(self.arena, n + moves)?;
        for (i, landed) in moved_to.iter_mut().enumerate().take(n) {
            let mut ins = self.instrs.as_slice()[i];
            /* Before the moves, so a branch to this instruction lands on the
             * code that fills its operand. */
            *landed = out.len();
            for slot in ins.srcs.iter_mut() {
                let Some(s) = *slot else { continue };
                if takes(ins.op, &s) {
                    continue;
                }
                let v = VReg(self.next_vreg);
                self.next_vreg += 1;
                let (ty, s) = legal_copy(ins.ty, s);
                out.push(
                    self.arena,
                    SelInstr::new(self.mov, ty)
                        .dst(SelDst::Virtual { v, write_mask: build::WRITE_MASK_ALL })
                        .operand(MOV_OPERAND, s),
                )?;
                /* Read back whole and unmodified: the move already applied
                 * whatever the operand carried, including an immediate's
                 * broadcast. */
                *slot = Some(SelSrc::Virtual { v, swizzle: ir::SWIZZLE_IDENTITY, neg: false, abs: false });
                note_site(|k| k.legalised += 1);
            }
            out.push(self.arena, ins)?;
        }
        moved_to[n] = out.len();
        for item in out.as_mut_slice() {
            if let Some(t) = item.target {
                item.target = Some(moved_to[(t as usize).min(n)] as u16);
            }
        }
        for e in self.back_edges.as_mut_slice() {
            *e = moved_to[(*e).min(n)];
        }
        self.instrs = out;
        Ok(())
    }

    /// Drop the instructions marked dead, and move every index that named one.
    ///
    /// Shared by the two passes that delete instructions -- copy propagation
    /// and dead-code elimination -- because the index rewriting is the part
    /// that is easy to get wrong and expensive to get wrong: a branch target
    /// is an instruction index, so deleting an instruction moves every target
    /// past it, and a branch left pointing at the old index is a shader that
    /// runs and draws something else.
    fn compact(&mut self, dead: &[bool]) -> Result<(), SelectError> {
        let n = self.instrs.len();
        /* Compact in place, then drop the tail.
         *
         * **A branch target is an instruction index, so deleting an
         * instruction moves every target past it.** Left alone that is a
         * branch into the wrong place -- a shader that runs and draws
         * something else, which is the failure this whole file keeps meeting.
         * So the compaction records where each surviving instruction went and
         * rewrites the targets through that map.
         *
         * A target may name the instruction one past the end (the `L_end` an
         * `if` jumps to when its arm falls through), so the map is one longer
         * than the list and its last entry is the new length. */
        let moved_to = self.arena.alloc_filled(n + 1, 0usize)?;
        let slice = self.instrs.as_mut_slice();
        let mut w = 0;
        for r in 0..n {
            moved_to[r] = w;
            if !dead[r] {
                slice[w] = slice[r];
                w += 1;
            }
        }
        moved_to[n] = w;
        for item in slice.iter_mut().take(w) {
            if let Some(t) = item.target {
                item.target = Some(moved_to[(t as usize).min(n)] as u16);
            }
        }
        /* A back edge is named by the INDEX of its branch, so it moves with
         * the rest. Forgetting it here would leave the validator permitting a
         * backward jump at some other instruction's index and refusing the
         * real one -- and copy propagation deletes `mov`s from inside loop
         * bodies, so the two indices do differ. A branch is never dead, so
         * `moved_to` always has its new home. */
        for e in self.back_edges.as_mut_slice() {
            *e = moved_to[(*e).min(n)];
        }
        while self.instrs.len() > w {
            self.instrs.pop();
        }
        Ok(())
    }

    fn nodes(&mut self, r: ir::Range) -> Result<(), SelectError> {
        /* Is this the top-level body, rather than an arm? Only there may a
         * `Return` appear, and only as the final node -- see below. */
        let is_body = r.first == self.m.body.first && r.len == self.m.body.len;
        for (i, n) in self.m.nodes_of(r).iter().enumerate() {
            match *n {
                ir::Node::Instr(id) => {
                    let op = *self.m.instr(id).ok_or(SelectError::Unsupported("a node naming no instruction"))?;
                    self.instr(id, &op)?;
                }
                /* `return` at the end of main is the end of the shader: the
                 * GC7000 marks the last instruction rather than branching, and
                 * that marking belongs to the builder.
                 *
                 * Anywhere else it is NOT the end of the shader, and treating
                 * it as one would silently drop everything after it. The front
                 * end guarantees it never emits one: an early `return` inside
                 * an `if` becomes a store to a result slot, with whatever
                 * followed the `if` lowered into its `else` arm
                 * (es100-frontend, 2026-09-22). This asserts that guarantee
                 * rather than relying on it, because the failure it prevents
                 * is a shader that compiles and computes half of itself. */
                ir::Node::Return => {
                    if !is_body || i + 1 != r.len as usize {
                        return Err(SelectError::Unsupported(
                            "a `return` that is not the last node of the body: the front end promises it never emits one",
                        ));
                    }
                }
                ir::Node::If { cond, then_, else_ } => self.if_convert(cond, then_, else_)?,
                ir::Node::Loop { body } => {
                    let before = &self.m.nodes_of(r)[..i];
                    self.loop_convert(before, body)?
                }
                ir::Node::Break => self.break_out()?,
                ir::Node::Continue => self.continue_loop()?,
            }
        }
        Ok(())
    }
}

/// Select instructions for `m`, in virtual registers.
///
/// Everything returned borrows `arena`, which is the compile's scratch.
/// `rows` with one row per `slot` (a vertex attribute location): the first
/// row of each location, widened to the widest of the rows sharing it.
fn one_row_per_location<'a>(arena: &'a Arena<'a>, rows: &'a [IoReg<'a>]) -> Result<&'a [IoReg<'a>], SelectError> {
    let mut v: AVec<'a, IoReg<'a>> = AVec::new();
    for r in rows {
        if let Some(i) = v.as_slice().iter().position(|x| x.slot == r.slot) {
            let kept = &mut v.as_mut_slice()[i];
            if r.components > kept.components {
                *kept = *r;
            }
            continue;
        }
        v.push(arena, *r)?;
    }
    Ok(v.into_slice())
}

pub fn select<'a>(m: &'a ir::Module<'a>, arena: &'a Arena<'a>) -> Result<Selected<'a>, SelectError> {
    select_keyed(m, arena, false)
}

/// [`select`], with `front_flipped` set for the fragment shader's
/// `SHC_VIV_KEY_FRONT_FLIPPED` variant: the draw's winding and the target's y
/// flip disagree, so `gl_FrontFacing` is true where the hardware says back.
pub fn select_keyed<'a>(
    m: &'a ir::Module<'a>,
    arena: &'a Arena<'a>,
    front_flipped: bool,
) -> Result<Selected<'a>, SelectError> {
    if m.stage != SHC_STAGE_VERTEX && m.stage != SHC_STAGE_FRAGMENT {
        return Err(SelectError::Unsupported("a stage that is neither vertex nor fragment"));
    }
    let mov = build::op_id("mov").ok_or(SelectError::Unsupported("the instruction set has no `mov`"))?;
    /* The slot -> register table, seeded with each slot's home register.
     * `if_convert` redirects entries while it selects an arm. */
    let locals_in = arena.alloc_filled(m.locals.len(), VReg(0))?;
    let locals_val = arena.alloc_filled(m.locals.len(), None::<SelSrc>)?;
    let value_alias = arena.alloc_filled(m.instrs.len(), None::<SelSrc>)?;
    for (l, e) in locals_in.iter_mut().enumerate() {
        *e = VReg(m.instrs.len() as u32 + l as u32);
    }
    site_store::update(|v| *v = MovSites::ZERO);
    let mut s = Sel {
        m,
        arena,
        front_flipped,
        locals_in,
        locals_val,
        value_alias,
        /* NOT reserved at the IR's size, though that took random.23's
         * selection from 1.15 MB to 659 KB on a fresh arena: once the front
         * end's scratch is given back (lower.rs relocate), the one large
         * block lands above its garbage, and the random.23 stand-in went
         * from compiling in 2,044,776 bytes to not compiling at all
         * (the_whole_compile_of_a_random23_shape_fits_the_budget). Grown by
         * doubling, as before. */
        instrs: AVec::new(),
        consts: AVec::new(),
        mov,
        /* Virtual numbers follow value numbers, so VReg(i) is the value
         * instrs[i] produced. The folded definitions never get one, which
         * leaves gaps: the allocator maps what it is given and must not assume
         * the numbering is dense. */
        /* After the values AND after the local slots, both of which already
         * have numbers by the time a lowering asks for scratch. */
        next_vreg: m.instrs.len() as u32 + m.locals.len() as u32,
        ps_color_reg: None,
        colour_regs: [None; MAX_COLOUR_OUTPUTS as usize],
        back_edges: AVec::new(),
        breaks: AVec::new(),
        loop_tops: AVec::new(),
        loop_limits: AVec::new(),
    };
    s.uniform_block()?;
    s.nodes(m.body)?;
    s.propagate_copies()?;
    s.fold_constant_reads()?;
    s.legalise_operands()?;
    s.eliminate_dead()?;

    /* The io lists, with the registers assigned above. Every DECLARED input
     * and output is listed, not only the ones an instruction touched.
     *
     * es100-frontend's correction says the front end numbers and the code
     * generator decides the set, because an unused attribute or varying gets
     * no register -- 32 samplers and 27 attributes or varyings are declared
     * and eliminated across the GC7000 table. Filtering is therefore right,
     * and it is NOT done here yet, because dropping a varying also renumbers
     * the ones after it (they are 1..=N with no gaps) and that renumbering
     * changes the driver's VS_OUTPUT stream. Doing half of it -- dropping the
     * declaration but keeping the number, or the reverse -- is worse than
     * doing none. The M3 shaders declare exactly what they use, so it cannot
     * bite tonight; it is an open item, not an oversight. */
    let io_of = |decls: &[ir::IoDecl<'a>], out: bool| -> Result<&'a [IoReg<'a>], SelectError> {
        let mut v = AVec::new();
        for d in decls {
            /* Declared and never touched: no register, and no row in the list
             * the driver reads. The set is decided here and the numbering by
             * `used_varying_index`, which every caller asks -- see its note on
             * why one place has to own both. */
            if !s.decl_is_used(d, !out) {
                continue;
            }
            /* A matrix varying: one row per column, each its own register,
             * slot and name (see `used_varying_index` and `column_name`). */
            if d.kind == ir::IoKind::Varying && varying_columns(decls, d) > 1 {
                for k in 0..varying_columns(decls, d) {
                    let slot = ir::IoSlot::Varying(d.location + k);
                    let reg = if out { s.output_reg(slot)? } else { s.input_reg(slot)? };
                    v.push(
                        arena,
                        IoReg {
                            reg,
                            slot: d.location + k,
                            components: d.components,
                            interp: d.interp,
                            name: column_name(arena, d.name, k)?,
                            /* the column's own type: GL_FLOAT_VEC2/3/4 */
                            gl_type: 0x8b50 + u16::from(d.components.clamp(2, 4) - 2),
                        },
                    )?;
                }
                continue;
            }
            let slot = match d.kind {
                ir::IoKind::Attribute => ir::IoSlot::Attrib(d.location),
                ir::IoKind::Varying => ir::IoSlot::Varying(d.location),
                /* A builtin is NOT an io-list entry. It is named by its own
                 * field -- `vs_pos_out_reg`, `vs_pointsize_out_reg`,
                 * `ps_color_out_reg`, `ps_depth_out_reg` -- and listing it in
                 * `in_`/`out` as well would say it twice, in two places that
                 * can then disagree.
                 *
                 * Measured over the whole offline table: **zero** of the io
                 * entries in all 307 blobs carry a `gl_` name, while every one
                 * of the 169 vertex blobs has a position register and 11 of
                 * them have `num_out = 0` -- position-only shaders, whose
                 * position is plainly not in the list. All 138 fragment blobs
                 * have `num_out = 0`, so `gl_FragColor` is never listed
                 * either.
                 *
                 * We listed them, which made a plain `gl_FragColor = ...`
                 * shader report one output where the table reports none. That
                 * count reaches the driver. */
                ir::IoKind::Position | ir::IoKind::PointSize | ir::IoKind::Colour | ir::IoKind::Depth => continue,
            };
            let reg = if out { s.output_reg(slot)? } else { s.input_reg(slot)? };
            v.push(
                arena,
                IoReg {
                    reg,
                    slot: d.location,
                    components: d.components,
                    interp: d.interp,
                    name: d.name,
                    gl_type: d.gl_type,
                },
            )?;
        }
        /* `gl_PointCoord`, when the fragment shader reads it: the last input
         * row, at the register `input_reg` gave its reads. */
        if !out && m.stage == SHC_STAGE_FRAGMENT && reads_point_coord(m) {
            v.push(
                arena,
                IoReg {
                    reg: s.pntc_reg()?,
                    slot: PNTC_SLOT,
                    components: 2,
                    interp: 0,
                    name: POINT_COORD_ROW,
                    gl_type: 0x8b50, /* GL_FLOAT_VEC2 */
                },
            )?;
        }
        Ok(v.into_slice())
    };
    let io_in = io_of(m.inputs, false)?;
    /* Attributes the link bound to one location (glBindAttribLocation
     * aliasing, `compile_vs_linked` -> `with_attrib_map`) read that
     * location's one input register, which one vertex element feeds: the
     * location is listed once, as wide as the widest of them, and counts once
     * against the 16 (the GL core answers the attribute queries from its own
     * copy of the declarations). Without aliasing every location is one row
     * already, so nothing changes. */
    let io_in = if m.stage == SHC_STAGE_VERTEX { one_row_per_location(arena, io_in)? } else { io_in };
    let mut io_out = io_of(m.outputs, true)?;
    /* A fragment shader writing more than the first colour lists every colour
     * it writes as an output row, by location: the driver programs one
     * render target per row (PS_OUTPUT_REG byte n), and the rows are what
     * the allocator's placement rewrites. A shader writing only the first
     * lists none -- ps_color_out_reg says it, as for every shader before
     * draw buffers, so their records are what they were. */
    if m.stage == SHC_STAGE_FRAGMENT && s.colour_regs[1..].iter().any(Option::is_some) {
        let mut v = AVec::new();
        for (loc, reg) in s.colour_regs.iter().enumerate() {
            let Some(reg) = *reg else { continue };
            let loc = loc as u8;
            /* the declaration it belongs to: its own location, or the array
             * that starts below it */
            let decl = m
                .outputs
                .iter()
                .filter(|d| d.kind == ir::IoKind::Colour && d.location <= loc)
                .max_by_key(|d| d.location);
            let (name, gl_type) = match decl {
                Some(d) => (column_name(arena, d.name, loc - d.location)?, d.gl_type),
                None => ("", 0x8b52), /* GL_FLOAT_VEC4 */
            };
            v.push(arena, IoReg { reg, slot: loc, components: 4, interp: 0, name, gl_type })?;
        }
        io_out = v.into_slice();
    }

    let vs_pos_out_reg = (m.stage == SHC_STAGE_VERTEX).then_some(VS_POS_REG);
    let reads_ids = m
        .instrs
        .iter()
        .any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::VertexId | ir::IoSlot::InstanceId, .. }));
    let vs_id_in_reg = if m.stage == SHC_STAGE_VERTEX && reads_ids { Some(s.vs_id_reg()?) } else { None };
    /* Named only when the shader writes it: an unwritten gl_PointSize must
     * stay NO_REG, or the driver reads a register the code never set. */
    let vs_pointsize_out_reg = m.outputs.iter().any(|d| d.kind == ir::IoKind::PointSize).then(|| s.vs_pointsize_reg());
    /* Whatever the store chose, or the default for a shader that writes the
     * colour from something other than an input read whole. */
    let ps_color_out_reg = (m.stage == SHC_STAGE_FRAGMENT).then(|| s.ps_color_reg.unwrap_or_else(|| s.fs_color_reg()));
    Ok(Selected {
        instrs: s.instrs.into_slice(),
        consts: s.consts.into_slice(),
        vregs: s.next_vreg,
        vs_pos_out_reg,
        ps_color_out_reg,
        vs_pointsize_out_reg,
        vs_id_in_reg,
        io_in,
        io_out,
        back_edges: s.back_edges.into_slice(),
        loop_limits: s.loop_limits.into_slice(),
    })
}

/// Colour outputs a fragment shader may write: the render targets the GC7000
/// drives (Mesa etnaviv's max_render_targets, num_rts / 2 for HALTI2+).
pub const MAX_COLOUR_OUTPUTS: u8 = 4;

/// The name of the fragment input row `gl_PointCoord` arrives in
/// ([`crate::shaderc::SHC_VIV_POINT_COORD`], which the link and the driver
/// look for).
pub const POINT_COORD_ROW: &str = crate::shaderc::SHC_VIV_POINT_COORD;

/// Its `slot`: Mesa's VARYING_SLOT_PNTC, clear of every varying location.
pub const PNTC_SLOT: u8 = 25;

/// Does the module read `gl_PointCoord`?
fn reads_point_coord(m: &ir::Module<'_>) -> bool {
    m.instrs.iter().any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::PointCoord, .. }))
}

/// A measurement switch for run-time-indexed uniform reads (gl_test 108):
/// [`UNIIDX_NO_CLAMP`] drops the clamp in front of the offset multiply, the
/// one thing that still differs from Mesa's etnaviv, which does not clamp.
/// 0 (the default) is the compiler as it is; the bit is never a default --
/// the clamp is an `ir.md` rule. Read at compile time, so set it before the
/// first glCompileShader. (Bit 1, reading a vec4 array at its own base
/// instead of a per-array copy, went with the copy on 2026-09-25: the read
/// is a `load` from memory now.)
#[no_mangle]
pub static mut shc_debug_uniidx: i32 = 0;

/// [`shc_debug_uniidx`]: no clamp in front of the offset multiply.
pub const UNIIDX_NO_CLAMP: i32 = 1;

/// Set [`shc_debug_uniidx`].
#[no_mangle]
pub extern "C" fn shc_debug_set_uniidx(bits: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(shc_debug_uniidx).write(bits) }
}

/// [`shc_debug_uniidx`]'s bits as they stand (the precompiled tier stands
/// aside while any is set: its rows were compiled with none).
pub(crate) fn uniidx_knobs() -> i32 {
    // SAFETY: as above; a plain read.
    unsafe { core::ptr::addr_of!(shc_debug_uniidx).read() }
}

fn uniidx_knob(bit: i32) -> bool {
    // SAFETY: as above; a plain read.
    (unsafe { core::ptr::addr_of!(shc_debug_uniidx).read() }) & bit != 0
}

/// The copy [`Sel::legalise_operands`] puts an operand through, in the
/// reference compiler's own form.
///
/// A plain copy is a `mov` at TYPE F32 -- all 1,840 register copies in the
/// reference are -- and an integer literal is `mov` with a `:u20`
/// immediate (its 5 integer materialisations; it has no `mov.s32` at all).
/// Typing the copy like its consumer gave `mov.s32 t, 16:s20` in front of an
/// ES 3.00 shift, a form the reference never emits
/// (tests/encoding_vocabulary.rs). A copy that carries a modifier, or a
/// negative literal, keeps the consumer's type: there the type is part of
/// what the copy computes, and the vocabulary check still sees the form.
fn legal_copy(consumer: OpType, s: SelSrc) -> (OpType, SelSrc) {
    match s {
        SelSrc::Physical(p) if p.is_immediate() => match (p.amode >> 1) & 3 {
            /* s20: re-encode a non-negative value as u20, the same bits */
            1 => {
                let v = ((p.imm_payload_bits().unwrap_or(0) << 12) as i32) >> 12;
                match u32::try_from(v).ok().and_then(|u| build::Src::imm_u20(u).ok()) {
                    Some(u) => (OpType::F32, SelSrc::Physical(u)),
                    None => (consumer, s),
                }
            }
            _ => (OpType::F32, s),
        },
        SelSrc::Physical(p) if !p.neg && !p.abs => (OpType::F32, s),
        SelSrc::Virtual { neg: false, abs: false, .. } | SelSrc::Input { neg: false, abs: false, .. } => {
            (OpType::F32, s)
        }
        _ => (consumer, s),
    }
}

/// A look-up whose coordinate has three lanes and that the 2D instructions
/// already serve: a cube with an explicit level or a bias, and a 3D or 2D
/// array sampler with an implicit level, a bias or an explicit level.
///
/// The instruction does not know the sampler's shape: cube-ness, depth and
/// layers are the sampler's descriptor (src/vivante/texture.rs), and the
/// reference compiler's 78 texldl all carry the level in SRC1 beside a
/// coordinate that may be three lanes (its first is `mad t1.xyz_` then
/// `texldl t1, tex0, t1.xyzw, 0.0:f20`). So a `Lod` is texldl, a `Bias`
/// texldb and an implicit level texld, the coordinate in SRC0 with its three
/// lanes, exactly as `builtins::texture` emits for the 2D kinds this maps to.
///
/// - Cube {Lod, Bias}: ES 1.00's textureCubeLod and textureCube(s, c, bias),
///   ES 3.00's textureLod and texture(samplerCube, c, bias). 98 of
///   dEQP-GLES2's 180 compile gaps on this GPU (host run at ac11728) were
///   these two.
/// - D3 and D2Array: ES 3.00's texture() and textureLod(). The reference did
///   the same for gl_test's `es300/vol.frag` (`texldl t2._y__, tex0.xxxx,
///   t1.xyzw, 0.0:f20` and `texld t2.x___, tex0.xxxx, t1.xyzw`) and
///   `es300/array.frag` (`texld t2.xyz_, tex0.xyzx, t2.xyzw`, the layer in z
///   as the shader computed it), as Mesa's etnaviv does: it passes an array
///   coordinate to the instruction unchanged.
fn three_lane_lookup(op: &ir::Op) -> Option<(ir::Es100Tex, Option<ir::Src>)> {
    let ir::Op::Tex { dim, shadow: false, proj: false, ret: ir::ScalarType::F32, lod, offset: None, .. } = *op else {
        return None;
    };
    match (dim, lod) {
        (ir::TexDim::Cube | ir::TexDim::D3 | ir::TexDim::D2Array, ir::TexLod::Lod(l)) => {
            Some((ir::Es100Tex::Tex2DLod, Some(l)))
        }
        (ir::TexDim::Cube | ir::TexDim::D3 | ir::TexDim::D2Array, ir::TexLod::Bias(b)) => {
            Some((ir::Es100Tex::Tex2DBias, Some(b)))
        }
        /* texld, as TexCube already is: the coordinate's lanes are the
         * instruction's whatever the shape */
        (ir::TexDim::D3 | ir::TexDim::D2Array, ir::TexLod::Implicit) => Some((ir::Es100Tex::TexCube, None)),
        _ => None,
    }
}

/// A `texelFetch` the GC7000 selects: a 2D, 3D or 2D array sampler with a
/// float result and no offset. Its level.
fn fetch_level(op: &ir::Op) -> Option<ir::Src> {
    match *op {
        ir::Op::Tex {
            dim: ir::TexDim::D2 | ir::TexDim::D3 | ir::TexDim::D2Array,
            shadow: false,
            proj: false,
            ret: ir::ScalarType::F32,
            lod: ir::TexLod::Fetch(l),
            offset: None,
            ..
        } => Some(l),
        _ => None,
    }
}

/// The register group an operand will be encoded in.
///
/// A virtual register and an io input are both temporaries by the time the
/// allocator is done; a physical source already carries its group.
fn group_of(s: &SelSrc) -> u8 {
    match s {
        SelSrc::Virtual { .. } | SelSrc::Input { .. } => build::RGROUP_TEMP,
        SelSrc::Physical(p) => p.group,
    }
}

/// The most operands of one group an instruction may hold, measured from the
/// reference compiler's 13,908 instructions (`tests/operand_banks.rs`).
///
/// A group the reference never uses has a cap of zero, which refuses it.
fn group_cap(group: u8) -> usize {
    match group {
        build::RGROUP_TEMP | build::RGROUP_UNIFORM_0 => 3,
        build::RGROUP_IMMED => 2,
        build::RGROUP_INTERNAL => 1,
        _ => 0,
    }
}

/// Would this instruction's operands be within the caps?
///
/// Two limits: the operands per register group, and **one constant-file
/// register** however many of its lanes are read -- the reference never
/// reads two different ones in one instruction (0 in 13,908;
/// `push_one_constant_register`, whose copies this is what keeps
/// `fold_constant_reads` from folding back).
fn within_caps(srcs: &[Option<SelSrc>; 3]) -> bool {
    let mut per = [0usize; 8];
    let mut constant: Option<(u8, u16)> = None;
    for src in srcs.iter().flatten() {
        let Some(n) = per.get_mut(group_of(src) as usize) else { return false };
        *n += 1;
        if let SelSrc::Physical(p) = src {
            if p.group == build::RGROUP_UNIFORM_0 || p.group == build::RGROUP_UNIFORM_1 {
                match constant {
                    None => constant = Some((p.group, p.reg)),
                    Some(k) if k == (p.group, p.reg) => {}
                    Some(_) => return false,
                }
            }
        }
    }
    per.iter().enumerate().all(|(g, n)| *n <= group_cap(g as u8))
}

/// How many locations -- and registers -- a varying declaration takes: one
/// per column of a matrix (`GL_FLOAT_MAT2`/`3`/`4`) declared once, one for
/// anything else.
///
/// **A matrix the front end already split is one location per
/// declaration.** Since 3beded7 the front end declares every varying once per
/// location -- `m`, `m[1]` -- each still carrying the matrix's GL type;
/// expanding those again gave each column two registers and moved every
/// varying after it (es100/matvary.frag's v_m[1].y read v_after's lane in
/// the differential). Split is recognised by name: another declaration of the
/// same base name, which is what the link pairs by.
fn varying_columns(decls: &[ir::IoDecl<'_>], d: &ir::IoDecl<'_>) -> u8 {
    let base = |n: &str| n.split('[').next().unwrap_or(n).len();
    let split = decls
        .iter()
        .filter(|o| o.kind == ir::IoKind::Varying && o.name[..base(o.name)] == d.name[..base(d.name)])
        .count()
        > 1;
    if split {
        return 1;
    }
    match d.gl_type {
        0x8b5a => 2,
        0x8b5b => 3,
        0x8b5c => 4,
        _ => 1,
    }
}

/// The name of column `k` of a matrix varying in the io rows: the name
/// itself for column 0, `name[k]` after it. The GC7000 link pairs the two
/// stages' rows by name (`link::by_register`), both stages are named by this
/// one function, and `[` cannot occur in a GLSL identifier, so a column name
/// can never collide with a real varying. Varying names are not reported by
/// any GL query, so nothing outside the link sees them.
fn column_name<'a>(arena: &'a Arena<'a>, name: &'a str, k: u8) -> Result<&'a str, SelectError> {
    if k == 0 {
        return Ok(name);
    }
    let mut buf = [0u8; 64];
    let tail = [b'[', b'0' + k, b']'];
    let n = name.len().min(buf.len() - tail.len());
    buf[..n].copy_from_slice(&name.as_bytes()[..n]);
    buf[n..n + 3].copy_from_slice(&tail);
    let s = core::str::from_utf8(&buf[..n + 3])
        .map_err(|_| SelectError::Unsupported("a varying name that is not UTF-8"))?;
    Ok(arena.alloc_str(s)?)
}

/// A source read through ONE of its lanes, in every position: what a scalar
/// operation computing lane `lane` of a vector reads. An immediate is left
/// alone -- it is one value in every lane, and its swizzle bits are payload.
fn lane_of(s: SelSrc, lane: u8) -> SelSrc {
    let pick = |sw: [u8; 4]| [sw[lane as usize & 3]; 4];
    match s {
        SelSrc::Virtual { v, swizzle, neg, abs } => SelSrc::Virtual { v, swizzle: pick(swizzle), neg, abs },
        SelSrc::Input { reg, swizzle, neg, abs } => SelSrc::Input { reg, swizzle: pick(swizzle), neg, abs },
        SelSrc::Physical(p) if p.is_immediate() => SelSrc::Physical(p),
        SelSrc::Physical(p) => SelSrc::Physical(build::Src { swizzle: pick(p.swizzle), ..p }),
    }
}

/// One swizzle applied on top of another.
///
/// The inner source already carries its own lane mapping; a read of it with
/// `outer` selects from those lanes, not from the register's. Getting this
/// backwards would silently permute a propagated value.
///
/// **Never give this an immediate.** On an immediate source the swizzle,
/// negate and absolute fields are not modifiers, they are the value's payload
/// (`build::Src::imm_payload`), so permuting them changes the number. The one
/// caller that can meet one -- `fold_constant_reads` -- handles it separately
/// and says why.
fn compose(src: SelSrc, outer: [u8; 4]) -> SelSrc {
    let pick = |base: [u8; 4]| -> [u8; 4] {
        [
            base[outer[0] as usize & 3],
            base[outer[1] as usize & 3],
            base[outer[2] as usize & 3],
            base[outer[3] as usize & 3],
        ]
    };
    match src {
        SelSrc::Input { reg, swizzle, neg, abs } => SelSrc::Input { reg, swizzle: pick(swizzle), neg, abs },
        /* An immediate is a SCALAR broadcast to all four lanes, so reading
         * its lanes in any order gives the same value and the outer swizzle
         * is the identity on it. Applying `pick` here instead would permute
         * the payload bits and silently change the number: measured, a
         * `vec4 c = vec4(0.5); use(c.yzxw)` compiled to `0.00036621094` --
         * `tests/online_oracle.rs`. */
        SelSrc::Physical(p) if p.is_immediate() => SelSrc::Physical(p),
        SelSrc::Physical(mut p) => {
            p.swizzle = pick(p.swizzle);
            SelSrc::Physical(p)
        }
        SelSrc::Virtual { v, swizzle, neg, abs } => SelSrc::Virtual { v, swizzle: pick(swizzle), neg, abs },
    }
}

/// Where `operand(0)` of a `mov` actually lands, for the tests that check the
/// placement happened. Selection never uses it: naming a slot is the
/// allocator's business, and a test that asserted the ordinal instead would
/// pass whatever the placement did.
#[cfg(test)]
const MOV_SRC_SLOT_FOR_TESTS: usize = 2;

#[cfg(test)]
mod tests {
    use super::super::store;
    use super::*;

    /// The loop guard `loop_convert` puts in front of every loop body: `mov
    /// cnt, 0` (before the loop), then `add`, `cmp.ge`, `branch.not` and the
    /// exit `branch` at the loop's top. A loop at the start of a program
    /// therefore has its first instruction at LOOP_TOP and its body at GUARD.
    const GUARD: usize = 5;
    const LOOP_TOP: u16 = 1;

    fn io(name: &'static str, kind: ir::IoKind, location: u8) -> ir::IoDecl<'static> {
        ir::IoDecl { name, kind, location, components: 4, interp: ir::INTERP_SMOOTH, gl_type: 0 }
    }

    /// A matrix attribute read only through a later column is present as a
    /// whole: dEQP default_vertex_attrib.mat2/3/4.* read `a_value[1]` alone,
    /// and a use check at the first location only dropped `a_value` from the
    /// program (glGetAttribLocation -1; 16 cases on the i.MX8MP). One row, at
    /// the first column's register, with the matrix type -- the form the
    /// driver and the GL core read a matrix attribute in.
    #[test]
    fn a_matrix_attribute_read_through_column_one_alone_is_in_the_program() {
        let _s = store::test_lock();
        for (gl_type, cols) in [(0x8b5au16, 2u8), (0x8b5b, 3), (0x8b5c, 4)] {
            let inputs = [
                io("a_position", ir::IoKind::Attribute, 0),
                ir::IoDecl { gl_type, components: cols, ..io("a_value", ir::IoKind::Attribute, 1) },
            ];
            let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
            /* only column 1 of the matrix: Attrib(1 + 1) */
            let instrs = [
                ir::Op::LoadInput { slot: ir::IoSlot::Attrib(2), comps: cols },
                ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(0)) },
            ];
            let nodes: [ir::Node; 2] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                inputs: &inputs,
                outputs: &outputs,
                body: ir::Range { first: 0, len: 2 },
                ..ir::Module::empty(SHC_STAGE_VERTEX)
            };
            store::with_scratch(|a| {
                let s = select(&m, a).expect("selects");
                let rows: std::vec::Vec<(&str, u8, u16)> = s.io_in.iter().map(|r| (r.name, r.reg, r.gl_type)).collect();
                assert_eq!(rows, [("a_value", 1, gl_type)], "mat{cols}: the matrix, whole, at its first column");
            })
            .unwrap();
        }
    }

    /// The M3 vertex shader: `gl_Position = a_pos;`
    #[test]
    fn a_passthrough_vertex_shader_needs_no_instruction_at_all() {
        /* `gl_Position = a_pos` with the attribute at register 0 and the
         * position read from register 0 is already done before it starts: the
         * value arrives in the register the output is taken from, and the
         * reference program for this source is four zero dwords. */
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let (n, pos) = store::with_scratch(|a| select(&m, a).map(|s| (s.instrs.len(), s.vs_pos_out_reg)))
            .unwrap()
            .expect("selects");
        assert_eq!(n, 0, "nothing to do, so nothing is emitted");
        assert_eq!(pos, Some(VS_POS_REG));
    }

    /// Case 89's fragment half, at the IR (the GLSL ES 3.00 front end is
    /// frontend2's and not on main yet): a `flat in uint` read as
    /// `float(v_idx)` is `i2f` at U32 -- the ONLY difference from the signed
    /// conversion on this machine -- and the input's row says flat
    /// (GC_INTERP_FLAT = 2 = INTERP_FLAT), which the driver programs per
    /// component.
    #[test]
    fn a_flat_uint_varying_converts_unsigned_and_keeps_its_flat_row() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 1 },
            ir::Op::Alu(ir::AluOp::U2f, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Instr(ir::InstrId(2)), ir::Node::Return];
        let inputs = [ir::IoDecl { components: 1, interp: ir::INTERP_FLAT, ..io("v_idx", ir::IoKind::Varying, 0) }];
        let outputs = [io("fragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 3 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let i2f = build::op_id("i2f").expect("the ISA has i2f");
        let (tys, interp) = store::with_scratch(|a| {
            select(&m, a).map(|s| {
                let tys: Vec<OpType> = s.instrs.iter().filter(|i| i.op == i2f).map(|i| i.ty).collect();
                (tys, s.io_in.iter().map(|r| r.interp).collect::<Vec<u8>>())
            })
        })
        .unwrap()
        .expect("selects");
        assert_eq!(tys, [OpType::U32], "one i2f, unsigned");
        assert_eq!(interp, [ir::INTERP_FLAT], "the input row is flat");
    }

    /// {Cube, Lod} and {Cube, Bias} -- textureCubeLod and the biased
    /// textureCube, 98 of dEQP-GLES2's 180 compile gaps on this GPU -- are one
    /// texldl / texldb with the level or bias in SRC1 and the coordinate in
    /// SRC0, the form the reference's 78 texldl take; the implicit cube stays
    /// one texld with no SRC1.
    #[test]
    fn a_cube_look_up_with_a_level_or_a_bias_is_one_texldl_or_texldb() {
        let _s = store::test_lock();
        let run = |lod: fn(ir::Src) -> ir::TexLod| {
            let instrs = [
                ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 3 },
                ir::Op::ConstF32([2.0f32.to_bits(); 4]),
                ir::Op::Tex {
                    dim: ir::TexDim::Cube,
                    shadow: false,
                    proj: false,
                    ret: ir::ScalarType::F32,
                    sampler: 3,
                    coord: ir::Src::id(ir::ValueId(0)),
                    lod: lod(ir::Src::splat(ir::ValueId(1))),
                    offset: None,
                },
                ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(2)) },
            ];
            let nodes = [
                ir::Node::Instr(ir::InstrId(0)),
                ir::Node::Instr(ir::InstrId(1)),
                ir::Node::Instr(ir::InstrId(2)),
                ir::Node::Instr(ir::InstrId(3)),
                ir::Node::Return,
            ];
            let inputs = [ir::IoDecl { components: 3, ..io("v_dir", ir::IoKind::Varying, 0) }];
            let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                body: ir::Range { first: 0, len: 5 },
                inputs: &inputs,
                outputs: &outputs,
                ..ir::Module::empty(SHC_STAGE_FRAGMENT)
            };
            store::with_scratch(|a| {
                select(&m, a).map(|s| {
                    s.instrs
                        .iter()
                        .filter(|i| i.tex.is_some())
                        .map(|i| (i.op.op().name, i.tex.map(|t| t.id), i.srcs[0].is_some(), i.srcs[1].is_some()))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap()
            .expect("selects")
        };
        assert_eq!(run(ir::TexLod::Lod), [("texldl", Some(3), true, true)], "textureCubeLod");
        assert_eq!(run(ir::TexLod::Bias), [("texldb", Some(3), true, true)], "textureCube with a bias");
        assert_eq!(run(|_| ir::TexLod::Implicit), [("texld", Some(3), true, false)], "textureCube, as before");
    }

    #[test]
    fn a_move_that_permutes_is_not_elided() {
        /* The elision is only for a copy of a register onto itself, whole and
         * unmodified. `gl_Position = a_pos.yzwx` reads the same register and
         * is a real permutation, so it needs its instruction. */
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreOutput {
                slot: ir::IoSlot::Position,
                src: ir::Src { val: ir::ValueId(0), swizzle: [1, 2, 3, 0] },
            },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let n = store::with_scratch(|a| select(&m, a).map(|s| s.instrs.len())).unwrap().expect("selects");
        assert_eq!(n, 1, "a permutation is work");
    }

    /// The M3 fragment shader: `gl_FragColor = vec4(1, 0, 0, 1);`
    #[test]
    fn a_literal_colour_fragment_shader_reads_the_constant_file() {
        let _s = store::test_lock();
        let red = [0x3f80_0000u32, 0, 0, 0x3f80_0000];
        let instrs = [
            ir::Op::ConstF32(red),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let (n, i, consts0, consts_len, col) = store::with_scratch(|a| {
            select(&m, a).map(|s| (s.instrs.len(), s.instrs[0], s.consts[0], s.consts.len(), s.ps_color_out_reg))
        })
        .unwrap()
        .expect("selects");
        assert_eq!(n, 1);
        match i.dst {
            Some(SelDst::Fixed { reg, .. }) => assert_eq!(reg, FS_COLOR_MIN_REG),
            other => panic!("{other:?}"),
        }
        match i.srcs[MOV_SRC_SLOT_FOR_TESTS] {
            Some(SelSrc::Physical(p)) => {
                assert_eq!(p.group, build::RGROUP_UNIFORM_0, "a literal is read from the constant file");
                assert_eq!(p.reg, 0, "the first literal takes the first constant register");
                /* Opaque red is two distinct values, 1.0 and 0.0, so the
                 * register holds two dwords and the swizzle reads them back
                 * as x, y, y, x -- the packing the shipped compiler does. */
                assert_eq!(p.swizzle, [0, 1, 1, 0]);
            }
            other => panic!("expected a constant-file read, got {other:?}"),
        }
        /* Four dwords of file because a register IS four dwords; two of them
         * are UNUSED rather than values nothing reads. */
        assert_eq!(consts_len, 4);
        assert_eq!(consts0, ConstSlot::literal(0x3f80_0000), "and they are LITERALs, touching no uniform path");
        assert_eq!(col, Some(FS_COLOR_MIN_REG));
    }

    #[test]
    fn fragment_varyings_are_pre_coloured_at_registers_one_upward() {
        /* The constraint abi.rs enforces and an allocator must not undo: the
         * driver builds VS_OUTPUT from fs.vs_reg_of_in[i] in order, so a
         * renumbered varying draws the wrong colours with nothing to report
         * it. Selection assigns 1..=N in DECLARATION order, not by location. */
        let _s = store::test_lock();
        /* Both read, so the subject is declaration order rather than the
         * filtering; `a_varying_nobody_reads_takes_no_register` covers that. */
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(3), comps: 4 },
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(7), comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes: [ir::Node; 3] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let inputs = [io("v_uv", ir::IoKind::Varying, 3), io("v_col", ir::IoKind::Varying, 7)];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 3 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        /* The colour is this varying read whole, so it takes the varying's own
         * register and the copy disappears -- what the reference compiler does
         * for exactly this shape. The register numbering is therefore read off
         * `io_in` and `ps_color_out_reg` rather than off an instruction, which
         * is the same fact stated where it now lives. */
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.io_in.len(), 2);
            assert_eq!(s.io_in[0].reg, 1, "the first declared varying");
            assert_eq!(
                s.io_in[1].reg, 2,
                "the second declared varying is register 2, by declaration order and not by location"
            );
            assert_eq!(s.ps_color_out_reg, Some(2), "and the colour is that varying, in place");
            assert!(s.instrs.is_empty(), "so there is nothing to copy");
        })
        .unwrap();
    }

    #[test]
    fn a_swizzled_source_carries_its_swizzle_through() {
        /* Decision 1 of ir.md, end to end: the swizzle survives selection
         * rather than being flattened into extra moves. */
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreOutput {
                slot: ir::IoSlot::Position,
                src: ir::Src { val: ir::ValueId(0), swizzle: [2, 1, 0, 3] },
            },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(1)), ir::Node::Return];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let i = store::with_scratch(|a| select(&m, a).map(|s| s.instrs[0])).unwrap().expect("selects");
        match i.srcs[MOV_SRC_SLOT_FOR_TESTS] {
            Some(SelSrc::Input { swizzle, .. }) => assert_eq!(swizzle, [2, 1, 0, 3]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_source_naming_no_definition_is_refused() {
        let _s = store::test_lock();
        let instrs = [ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(9)) }];
        let nodes = [ir::Node::Instr(ir::InstrId(0))];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 1 },
            outputs: &outputs,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let e = store::with_scratch(|a| select(&m, a).map(|_| ())).unwrap().expect_err("refused");
        assert_eq!(e, SelectError::UndefinedValue(ir::ValueId(9)));
    }

    /// `gl_PointCoord` is the fragment input row after every varying the
    /// shader reads, two components, named so the link leaves it unpaired
    /// (Mesa's etnaviv, etna_link_shader), and its reads come from that
    /// register. It was the last input refused as "not selected yet".
    #[test]
    fn gl_point_coord_is_the_input_row_after_the_varyings() {
        let _s = store::test_lock();
        let pntc = ir::Op::LoadInput { slot: ir::IoSlot::PointCoord, comps: 2 };
        let colour = |v: u32| ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(v)) };
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let rows = |m: &ir::Module<'_>| {
            store::with_scratch(|a| {
                select(m, a).map(|s| {
                    let reads: std::vec::Vec<u8> = s
                        .instrs
                        .iter()
                        .flat_map(|i| i.srcs.iter().flatten())
                        .filter_map(|x| match x {
                            SelSrc::Input { reg, .. } => Some(*reg),
                            _ => None,
                        })
                        .collect();
                    let rows_in: std::vec::Vec<(u8, u8, std::string::String)> =
                        s.io_in.iter().map(|r| (r.reg, r.components, r.name.into())).collect();
                    (rows_in, reads)
                })
            })
            .unwrap()
            .expect("selects")
        };

        /* alone: register 1, the first a fragment input takes */
        let instrs = [pntc, colour(0)];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1))];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 2 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let (got, _) = rows(&m);
        assert_eq!(got, [(1, 2, POINT_COORD_ROW.into())]);

        /* after a varying the shader reads: register 2, the last row */
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 4 },
            pntc,
            ir::Op::Alu(
                ir::AluOp::Fadd,
                [ir::Src::id(ir::ValueId(0)), ir::Src::id(ir::ValueId(1)), ir::Src::id(ir::ValueId(0))],
            ),
            colour(2),
        ];
        let nodes = [0, 1, 2, 3].map(|i| ir::Node::Instr(ir::InstrId(i)));
        let inputs = [io("v", ir::IoKind::Varying, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let (got, reads) = rows(&m);
        assert_eq!(got, [(1, 4, "v".into()), (2, 2, POINT_COORD_ROW.into())]);
        assert!(reads.contains(&2), "gl_PointCoord is read from its row's register: {reads:?}");
    }

    /// A bool is an all-ones integer mask, and each boolean operation is
    /// performed in the type the reference compiler used.
    ///
    /// These four types are not a preference. They were read off the
    /// instructions the reference compiler produced for the same four sources,
    /// decoded field by field (the sequences are written out above
    /// `op_type`). Getting one wrong is silent -- a `not` in the wrong type
    /// clears the wrong bits and the shader computes a plausible wrong answer
    /// -- so they are pinned here rather than left to a comment.
    #[test]
    fn a_boolean_operation_is_performed_in_the_type_the_reference_used() {
        use ir::AluOp::*;
        /* U32 for turning a mask into a number and for `select`, because both
         * are bitwise choices on the mask; S32 for `not` and for the `or` and
         * `and` of a reduction. */
        for (op, want) in [(B2f, OpType::U32), (B2i, OpType::U32), (Bcsel, OpType::U32)] {
            assert_eq!(op_type(op), Some(want), "{op:?}");
        }
        for op in [Bnot, Bany, Ball] {
            assert_eq!(op_type(op), Some(OpType::S32), "{op:?}");
        }

        /* And it reaches the machine: `!b` selects, rather than refusing, and
         * arrives as a `not` in that type. */
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::Alu(Bnot, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1)), ir::Node::Instr(ir::InstrId(2))];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 3 },
            uniform_dwords: 4,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("a boolean operation is selected, not refused");
            let n = s.instrs.iter().find(|i| i.op == build::op_id("not").expect("the machine has `not`"));
            assert_eq!(n.expect("a `not` was emitted").ty, OpType::S32);
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }

    /// `gl_FragColor = gl_FragCoord` copies, and the colour never lands in
    /// register 0.
    ///
    /// The colour "is already in place" rule is measured and right for a
    /// varying, and wrong for `gl_FragCoord`, which is also an input but sits
    /// in register 0 -- a register the colour may not live in. Unguarded, this
    /// shader compiled to an empty program that claimed the colour was already
    /// there. The reference emits `mov t1, t0.xyzw`, and with the guard we now
    /// emit that byte for byte.
    #[test]
    fn the_fragment_colour_is_copied_out_of_gl_fragcoord_and_not_left_in_register_zero() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::FragCoord, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1))];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("gl_FragCoord is selected");
            assert!(!s.instrs.is_empty(), "the copy is emitted, not elided into an empty program");
            let c = s.ps_color_out_reg.expect("a fragment colour register");
            assert!(c >= FS_COLOR_MIN_REG, "the colour must not be register 0, which holds gl_FragCoord; got {c}");
            let src = &s.instrs[0].srcs[MOV_SRC_SLOT_FOR_TESTS];
            let Some(SelSrc::Input { reg, swizzle, .. }) = src else { panic!("reads gl_FragCoord, got {src:?}") };
            assert_eq!((*reg, *swizzle), (FS_FRAGCOORD_REG, ir::SWIZZLE_IDENTITY));
            /* then y' = y * scale + offset from the driver-filled pair
             * (frag_coord): a mad writing lane y from t0.y and two lanes of
             * one constant register holding FRAGCOORD_Y_SCALE / _OFFSET */
            let mad = build::op_id("mad").expect("the machine has `mad`");
            let m = s.instrs.iter().find(|i| i.op == mad).expect("the y transform's mad");
            assert!(matches!(m.dst, Some(SelDst::Virtual { write_mask: 0b0010, .. })), "lane y only: {:?}", m.dst);
            assert!(matches!(m.srcs[0], Some(SelSrc::Input { reg: FS_FRAGCOORD_REG, swizzle: [1, 1, 1, 1], .. })));
            let (Some(SelSrc::Physical(sc)), Some(SelSrc::Physical(of))) = (m.srcs[1], m.srcs[2]) else {
                panic!("scale and offset from the constant file: {:?}", m.srcs)
            };
            assert_eq!(sc.reg, of.reg, "one constant register holds the pair");
            let lane = |p: build::Src| s.consts[p.reg as usize * 4 + p.swizzle[0] as usize].contents;
            assert_eq!(
                (lane(sc), lane(of)),
                (crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_SCALE, crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_OFFSET)
            );
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }

    /// An `if` becomes a `select`, and the select reads the two arms' answers.
    ///
    /// The shape is what the reference compiler produces for the same source
    /// -- `cmp` then `select`, no branch -- and the operands are what make it
    /// right rather than merely present: a select reading the wrong register
    /// compiles perfectly and draws the wrong arm.
    #[test]
    fn an_if_is_converted_into_a_select_over_both_arms() {
        let _s = store::test_lock();
        /* c = u; if (cond) { c = k } ; gl_FragColor = c */
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 4 }, // 0: the condition
            ir::Op::ConstF32([1.0f32.to_bits(); 4]),        // 1: the then value
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            }, // 2
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 }, // 3
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(3)) }, // 4
        ];
        let nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 5, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
            /* The arm's body, out of line: a Range indexes the one flat node
             * array, so an arm sits after the block that jumps into it. */
            ir::Node::Instr(ir::InstrId(2)),
        ];
        let locals = [ir::VarDecl { name: "c", ty: ir::Type::F32X4, array_len: 1 }];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            locals: &locals,
            body: ir::Range { first: 0, len: 5 },
            uniform_dwords: 4,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("an if is converted, not refused");
            let sel = build::op_id("select").expect("the machine has `select`");
            let n = s.instrs.iter().filter(|i| i.op == sel).count();
            assert_eq!(n, 1, "one select for the one slot the arm writes, got {n}");
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }

    #[test]
    fn a_uniform_is_read_from_the_constant_file_and_costs_no_instruction() {
        /* The constant file mirrors the uniform block, so block dword 6 is
         * register 1 component 2 -- no lookup table, and no `mov` to
         * materialise the value, because the hardware reads the file as an
         * operand. */
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadUniform { dword_off: 6, comps: 2 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1))];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let sel = store::with_scratch(|a| {
            let s = select(&m, a)?;
            /* Eight block dwords = two whole constant registers, each entry
             * naming its own dword. */
            assert_eq!(s.consts.len(), 8);
            for (i, c) in s.consts.iter().enumerate() {
                assert_eq!(*c, ConstSlot::uniform(i as u32), "const slot {i}");
            }
            /* One instruction: the store. The load folded into it. */
            assert_eq!(s.instrs.len(), 1);
            let src = &s.instrs[0].srcs[MOV_SRC_SLOT_FOR_TESTS];
            let Some(SelSrc::Physical(p)) = src else { panic!("a uniform read is a register operand, got {src:?}") };
            assert_eq!(p.group, build::RGROUP_UNIFORM_0);
            assert_eq!(p.reg, 1, "block dword 6 is register 1");
            /* Components 2 and 3, then 3 again: a vec2 read must not run on
             * into the dword after it. */
            assert_eq!(p.swizzle, [2, 3, 3, 3]);
            Ok::<_, SelectError>(())
        })
        .unwrap();
        sel.expect("selected");
    }

    #[test]
    fn a_uniform_read_past_the_declared_block_is_refused() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadUniform { dword_off: 9, comps: 1 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1))];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let e = store::with_scratch(|a| select(&m, a).map(|_| ())).unwrap().expect_err("refused");
        assert_eq!(e, SelectError::Unsupported("a uniform read past the block the module declares"));
    }

    #[test]
    fn a_matrix_times_a_vector_is_a_mul_and_three_mads() {
        /* The sequence is not a choice: `hello.vert` in SHC_SOURCE_TABLE is
         * exactly `gl_Position = transformationMatrix * myVertex`, and its
         * shipped GC7000 blob is four instructions, temps 2 -- a `mul` and
         * three `mad`, NOT four `dp4`s, which is the other plausible encoding
         * and the one a reader expects.
         *
         * **Which column goes with which component**, because the blob's
         * disassembly reads the opposite way round and I had it wrong here
         * until es100-frontend caught it. The listing is
         *
         *     mul  t1, u0, t0.wwww
         *     mad  t1, u1, t0.zzzz, t1
         *     ...
         *
         * which looks like "column 0 against v.w". It is not. The constant
         * file map that ships with it, `CD_4` in `vivante_blobs.rs`, is
         * `12,13,14,15 / 8,9,10,11 / 4,5,6,7 / 0,1,2,3`, so constant register
         * 0 holds block dwords 12..15 and IS column 3. Every term pairs
         * **column i with v[i]**; what runs backwards is the offline
         * compiler's constant-REGISTER numbering, not the algebra.
         *
         * So this builds the IR in block dwords, where the pairing is plain,
         * and the wrong version of it computed `M * reverse(v)`. That draws a
         * correct picture for every identity or symmetric matrix, which is
         * most bring-up shaders -- it would have reached silicon looking fine.
         *
         * Built here as IR rather than from GLSL so this pins selection and
         * not the front end's expansion. The end-to-end check is the oracle's:
         * hello.vert through the whole chain against the shipped blob. */
        let _s = store::test_lock();
        let v = |c: u8| ir::Src { val: ir::ValueId(1), swizzle: [c; 4] };
        let col = |dword_off: u16| ir::Op::LoadUniform { dword_off, comps: 4 };
        let instrs = [
            /* 0..4: the four columns, by block dword. Highest column first,
             * which is the shipped blob's instruction order. */
            col(12),
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            col(8),
            col(4),
            col(0),
            /* 5: col3 * v.w */
            ir::Op::Alu(ir::AluOp::Fmul, [ir::Src::id(ir::ValueId(0)), v(3), ir::Src::id(ir::ValueId(0))]),
            /* 6..8: += col2 * v.z, col1 * v.y, col0 * v.x */
            ir::Op::Alu(ir::AluOp::Ffma, [ir::Src::id(ir::ValueId(2)), v(2), ir::Src::id(ir::ValueId(5))]),
            ir::Op::Alu(ir::AluOp::Ffma, [ir::Src::id(ir::ValueId(3)), v(1), ir::Src::id(ir::ValueId(6))]),
            ir::Op::Alu(ir::AluOp::Ffma, [ir::Src::id(ir::ValueId(4)), v(0), ir::Src::id(ir::ValueId(7))]),
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(8)) },
        ];
        let nodes: [ir::Node; 10] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let inputs = [io("myVertex", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 10 },
            uniform_dwords: 16,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selected");
            let id = |n: &str| build::op_id(n).expect("the instruction set has this opcode");
            assert_eq!(s.instrs[0].op, id("mul"));
            for (i, instr) in s.instrs.iter().enumerate().take(4).skip(1) {
                assert_eq!(instr.op, id("mad"), "instruction {i}");
            }
            /* Column i against v[i]: instruction 0 reads the column at block
             * dword 12 (column 3) against the w lane, and so on down. Stated
             * against the constant register this file assigns, which is
             * dword_off / 4 -- the forward numbering, not the shipped
             * compiler's reversed one. Both are self-consistent; what must
             * never differ is the pairing. */
            for (i, (want_reg, want_lane)) in [(3u16, 3u8), (2, 2), (1, 1), (0, 0)].into_iter().enumerate() {
                let p = s.instrs[i]
                    .srcs
                    .iter()
                    .flatten()
                    .find_map(|x| match x {
                        SelSrc::Physical(p) => Some(p),
                        _ => None,
                    })
                    .expect("a matrix column is a constant-file read");
                assert_eq!(p.reg, want_reg, "instruction {i} reads column {want_reg}");
                let lane = s.instrs[i]
                    .srcs
                    .iter()
                    .flatten()
                    .find_map(|x| match x {
                        SelSrc::Input { swizzle, .. } => Some(swizzle[0]),
                        _ => None,
                    })
                    .expect("and the vector");
                assert_eq!(lane, want_lane, "against v[{want_lane}]");
            }
            assert_eq!(s.consts.len(), 16, "mat4 = sixteen block dwords");
        })
        .unwrap();
    }

    #[test]
    fn fragment_varyings_start_at_register_one() {
        /* Not a style choice. All 116 fragment blobs in the offline table that
         * read a varying have their lowest input register at 1, so register 0
         * is reserved in this stage and a varying placed there would be read
         * from the wrong place. */
        let _s = store::test_lock();
        let inputs = [
            ir::IoDecl {
                kind: ir::IoKind::Varying,
                location: 0,
                components: 4,
                interp: 0,
                gl_type: 0,
                name: "v_color",
            },
            ir::IoDecl { kind: ir::IoKind::Varying, location: 1, components: 2, interp: 0, gl_type: 0, name: "v_uv" },
        ];
        /* Both are read, so both keep a register and the subject is the
         * ORDER. A varying nobody reads is dropped instead -- see
         * `a_varying_nobody_reads_takes_no_register`. */
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 4 },
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(1), comps: 2 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes: [ir::Node; 3] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            body: ir::Range { first: 0, len: 3 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.io_in.len(), 2);
            assert_eq!(s.io_in[0].reg, 1, "the first varying");
            assert_eq!(s.io_in[1].reg, 2, "and the second after it");
            assert!(s.io_in.iter().all(|io| io.reg != 0), "register 0 is reserved in the fragment stage");
        })
        .unwrap();
    }

    #[test]
    fn an_operation_the_machine_lacks_comes_back_through_the_lowering_library() {
        /* The seam itself, not the sequence: `Fmin` has no opcode, so `native`
         * returns None and `lower` writes it as `select` with a condition.
         * That the GC7000 spells min this way is builtins.rs's measurement
         * (303 of 303 `.gt`/`.lt` selects in the shipped table repeat src0
         * into src2); what this checks is that the instruction it emits
         * reaches our instruction list at all, with the destination we asked
         * for. The seam was uncallable for a day, so it is worth a test that
         * fails if it comes apart again. */
        let _s = store::test_lock();
        let inputs = [
            ir::IoDecl { kind: ir::IoKind::Varying, location: 0, components: 4, interp: 0, gl_type: 0, name: "a" },
            ir::IoDecl { kind: ir::IoKind::Varying, location: 1, components: 4, interp: 0, gl_type: 0, name: "b" },
        ];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 4 },
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(1), comps: 4 },
            ir::Op::Alu(
                ir::AluOp::Fmin,
                [ir::Src::id(ir::ValueId(0)), ir::Src::id(ir::ValueId(1)), ir::Src::id(ir::ValueId(0))],
            ),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(2)) },
        ];
        let nodes: [ir::Node; 4] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            /* One instruction: the lowered `select` writes the colour
             * register itself, because its result is used only by the store
             * and the copy would be pure loss. */
            assert_eq!(s.instrs.len(), 1, "the lowered instruction, writing the output directly");
            assert_eq!(s.instrs[0].op, build::op_id("select").expect("the ISA has select"));
            assert!(s.instrs[0].cond.is_some(), "the form that tests a condition");
            match s.instrs[0].dst {
                Some(SelDst::Fixed { reg, .. }) => {
                    assert_eq!(reg, s.ps_color_out_reg.expect("a fragment shader has one"))
                }
                other => panic!("expected the colour register, got {other:?}"),
            }
        })
        .unwrap();
    }

    /// A store that permutes lanes is done by the instruction that computed
    /// them, not by a copy after it -- except where the lanes are not
    /// independent.
    ///
    /// Three cases in one test because they are one decision, and the wrong
    /// answer to any of them is a shader that computes something else:
    ///
    /// 1. an ordinary componentwise operation takes the permutation into its
    ///    own sources and the copy disappears;
    /// 2. a DOT PRODUCT does not: it sums across the lanes, so permuting its
    ///    sources changes the sum rather than moving it, and the copy stays;
    /// 3. a SCALAR operation is retargeted but its sources are left alone.
    ///    The builder splats them from lane 0, so all four of its destination
    ///    lanes already hold one number and any permutation of them is the
    ///    identity -- while composing would change WHICH component that
    ///    number was computed from, turning `rcp(x.x)` into `rcp(x.y)`.
    ///
    /// Worth 229 of the corpus's store copies (6,203 -> 6,040 instructions).
    /// An arm that writes nothing still needs the slot's value, or the join
    /// selects a register nobody wrote.
    ///
    /// THE DEFECT THIS PINS (tests/read_before_write.rs counts the class):
    ///
    /// ```glsl
    /// v = a;
    /// if (c) { } else { v = b; }
    /// gl_Position = v;
    /// ```
    ///
    /// compiled to
    ///
    /// ```text
    ///   mov    t7, u1          ; v = a
    ///   mov    t7, attr0       ; the else arm, over the top of a
    ///   select t7, c, t8, t7   ; t8 IS NEVER WRITTEN
    /// ```
    ///
    /// With `c` true the shader used whatever `t8` held, where the language
    /// says `a` -- and `a` had already been destroyed by the else arm. A
    /// shader that compiles, draws, and is wrong on one side of a condition.
    ///
    /// The cause: `written` is true for the PAIR of arms, so the join selects
    /// from the then arm's register whenever EITHER arm writes the slot,
    /// while the seed that fills that register only ran when the then arm
    /// could observe the slot. An arm that leaves a slot alone got no seed
    /// and no store.
    ///
    /// **It is not an edge case.** Any `if (c) { x = ...; } else { y = ...; }`
    /// hits it for `y`, and 30 of the corpus's 137 sources contained one --
    /// every FemtoVG fill, gradient, image and stencil shader, which is what
    /// the UI draws with. The fix costs 84 seeds, 104 instructions and 76
    /// temps over the corpus.
    #[test]
    fn an_arm_that_writes_nothing_still_needs_the_slots_value() {
        let locals = [ir::VarDecl { name: "v", ty: ir::Type::F32X4, array_len: 1 }];
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(3)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(5)) },
        ];
        let nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range::EMPTY,
                else_: ir::Range { first: 6, len: 2 },
            },
            ir::Node::Instr(ir::InstrId(5)),
            ir::Node::Instr(ir::InstrId(6)),
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
        ];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 6 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            /* EVERY virtual register an instruction reads must be written by
             * an earlier one. Stated over the whole program rather than by
             * naming the select's operand, because the defect is "a register
             * nobody wrote" and that is the property worth asserting. */
            let mut written: Vec<VReg> = Vec::new();
            for (i, ins) in s.instrs.iter().enumerate() {
                for src in ins.srcs.iter().flatten() {
                    if let SelSrc::Virtual { v, .. } = src {
                        assert!(
                            written.contains(v),
                            "instruction {i} ({}) reads {v:?}, which nothing has written",
                            ins.op.op().name
                        );
                    }
                }
                if let Some(SelDst::Virtual { v, .. }) = ins.dst {
                    written.push(v);
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn a_load_names_the_slots_register_unless_a_loop_touches_the_slot() {
        let locals = [ir::VarDecl { name: "c", ty: ir::Type::F32X4, array_len: 1 }];

        /* `if (u) { c = <a uniform>; }  gl_Position = c;`
         *
         * The load is after the join and nothing stores the slot again, so it
         * names the register the join wrote. */
        let straight = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(3)) },
        ];
        let straight_nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 4, len: 2 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
        ];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &straight,
            nodes: &straight_nodes,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 4 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            select(&m, a).expect("selects");
            assert_eq!(last_mov_sites().load_snapshot, 1, "the load named the register the `if` left the slot in");
            assert_eq!(last_mov_sites().load_local, 0, "and cost no copy");
        })
        .unwrap();

        /* The same slot inside a LOOP, stored before the read in the body.
         * Program order says nothing stores it after the read; the next
         * iteration does. */
        let looped = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(3)) },
        ];
        let loop_nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range::EMPTY,
                else_: ir::Range { first: 6, len: 1 },
            },
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
            ir::Node::Break,
            ir::Node::Loop { body: ir::Range { first: 0, len: 6 } },
        ];
        let m = ir::Module { locals: &locals, ..while_loop_module(&looped, &loop_nodes, SHC_STAGE_FRAGMENT) };
        store::with_scratch(|a| {
            select(&m, a).expect("a loop with a slot in it selects");
            assert_eq!(last_mov_sites().load_snapshot, 0, "a slot a loop touches is never named, only copied");
        })
        .unwrap();
    }

    #[test]
    fn a_store_that_permutes_lanes_is_done_by_the_instruction_that_made_them() {
        fn one(op: ir::AluOp, store_swizzle: [u8; 4]) -> (usize, [Option<SelSrc>; 3]) {
            producer(ir::Op::Alu(op, [ir::Src::id(ir::ValueId(0)); 3]), store_swizzle)
        }
        /* The store reads the value itself, through the swizzle: the shape a
         * retarget may take. Through a slot it may not (the slot's other
         * reads resolve to the same register, unseen): the next test. */
        fn direct(op: ir::AluOp, store_swizzle: [u8; 4]) -> (usize, [Option<SelSrc>; 3]) {
            let instrs = [
                ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                ir::Op::Alu(op, [ir::Src::id(ir::ValueId(0)); 3]),
                ir::Op::StoreOutput {
                    slot: ir::IoSlot::Position,
                    src: ir::Src { val: ir::ValueId(1), swizzle: store_swizzle },
                },
            ];
            let nodes: [ir::Node; 3] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
            let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
            let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                inputs: &inputs,
                outputs: &outputs,
                body: ir::Range { first: 0, len: 3 },
                ..ir::Module::empty(SHC_STAGE_VERTEX)
            };
            let _s = store::test_lock();
            store::with_scratch(|a| {
                let s = select(&m, a).expect("selects");
                (s.instrs.len(), s.instrs[0].srcs)
            })
            .unwrap()
        }
        fn producer_at(made_by: ir::Op, k: usize) -> (usize, [Option<SelSrc>; 3]) {
            let instrs = [
                ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                made_by,
                ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(1)) },
            ];
            let nodes: [ir::Node; 3] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
            let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
            let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                inputs: &inputs,
                outputs: &outputs,
                body: ir::Range { first: 0, len: 3 },
                ..ir::Module::empty(SHC_STAGE_VERTEX)
            };
            let _s = store::test_lock();
            store::with_scratch(|a| {
                let s = select(&m, a).expect("selects");
                (s.instrs.len(), s.instrs[k].srcs)
            })
            .unwrap()
        }
        fn producer(made_by: ir::Op, store_swizzle: [u8; 4]) -> (usize, [Option<SelSrc>; 3]) {
            let instrs = [
                ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                made_by,
                ir::Op::StoreLocal {
                    local: ir::LocalId(0),
                    index: None,
                    src: ir::Src { val: ir::ValueId(1), swizzle: store_swizzle },
                    write_mask: build::WRITE_MASK_ALL,
                },
                ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
                ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(3)) },
            ];
            let nodes: [ir::Node; 5] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
            let locals =
                [ir::VarDecl { name: "c", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
            let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
            let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                inputs: &inputs,
                outputs: &outputs,
                locals: &locals,
                body: ir::Range { first: 0, len: 5 },
                ..ir::Module::empty(SHC_STAGE_VERTEX)
            };
            let _s = store::test_lock();
            store::with_scratch(|a| {
                let s = select(&m, a).expect("selects");
                (s.instrs.len(), s.instrs[0].srcs)
            })
            .unwrap()
        }

        /* 1. `add`, permuted. The instruction reads its own source through
         *    the swizzle, so lane 0 gets what lane 1 of the sum would have
         *    held. (Stored directly: through a slot the store reads an alias,
         *    and aliases are not retargeted -- see the next test.) */
        let (n, srcs) = direct(ir::AluOp::Fadd, [1, 2, 3, 0]);
        assert_eq!(n, 1, "the add writes the position register itself, permutation and all");
        for s in srcs.iter().flatten() {
            assert!(
                matches!(s, SelSrc::Input { swizzle: [1, 2, 3, 0], .. }),
                "the permutation moved into the operand, got {s:?}"
            );
        }

        /* 2. `dp3`: the copy stays and the sources are untouched. */
        let (n, srcs) = one(ir::AluOp::Fdot3, [0, 0, 0, 0]);
        assert_eq!(n, 2, "the dot product keeps its copy");
        for s in srcs.iter().flatten() {
            assert!(
                matches!(s, SelSrc::Input { swizzle: [0, 1, 2, 3], .. }),
                "a dot product's operands must not be permuted, got {s:?}"
            );
        }

        /* 2b. A TEXTURE FETCH reads its lanes as coordinates, so the copy
         *     stays there too. The lead asked for this case beside the dot
         *     product because they fail the same condition for two different
         *     reasons, and a rule that happened to catch only one of them
         *     would look right. */
        let (n, srcs) =
            producer(ir::Op::es100(ir::Es100Tex::Tex2D, 0, ir::Src::id(ir::ValueId(0)), None), [0, 0, 0, 0]);
        assert_eq!(n, 2, "the texture fetch keeps its copy");
        for s in srcs.iter().flatten() {
            assert!(
                matches!(s, SelSrc::Input { swizzle: [0, 1, 2, 3], .. }),
                "a texture coordinate must not be permuted, got {s:?}"
            );
        }

        /* 3. `rcp`, a SCALAR operation: one instruction per lane, lane c
         *    reading lane c of its operand, and the store's copy. This case
         *    used to assert ONE rcp writing the output with the operand's
         *    identity swizzle -- which computed rcp(v.x) into every lane of
         *    a vec4, the defect vtransc.frag and gl_test case 76 draw. The
         *    lanes nobody reads are deleted after selection
         *    (alloc::eliminate_dead), not here. */
        let (n, _) = one(ir::AluOp::Frcp, [0, 0, 0, 0]);
        assert_eq!(n, 5, "four per-lane reciprocals and the store's copy");
        for lane in 0..4u8 {
            let (_, srcs) = producer_at(ir::Op::Alu(ir::AluOp::Frcp, [ir::Src::id(ir::ValueId(0)); 3]), lane as usize);
            for s in srcs.iter().flatten() {
                assert!(
                    matches!(s, SelSrc::Input { swizzle, .. } if *swizzle == [lane; 4]),
                    "lane {lane}'s reciprocal must read lane {lane} of its operand, got {s:?}"
                );
            }
        }
    }

    /// One slot stored to TWO outputs (Qt Quick 3D's motionvector shader:
    /// `_41 = mvp * v; vPosition = _41; gl_Position = _41;`). Both reads of
    /// the slot resolve to the register of the value that filled it, so the
    /// first store may not take the instruction that computed it: it did,
    /// and gl_Position then copied a register nothing wrote (the partial sum,
    /// on the GC7000 online path). Both outputs must be written from the
    /// finished value.
    #[test]
    fn a_slot_stored_to_two_outputs_is_not_retargeted_into_either() {
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Varying(0), src: ir::Src::id(ir::ValueId(3)) },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(5)) },
        ];
        let nodes: [ir::Node; 7] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let locals =
            [ir::VarDecl { name: "_41", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("vPosition", ir::IoKind::Varying, 0), io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 7 },
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            /* whatever each output register is written from, it must be a
             * register some instruction writes */
            let written: std::vec::Vec<VReg> = s
                .instrs
                .iter()
                .filter_map(|i| match i.dst {
                    Some(SelDst::Virtual { v, .. }) => Some(v),
                    _ => None,
                })
                .collect();
            let outs: std::vec::Vec<_> =
                s.instrs.iter().filter(|i| matches!(i.dst, Some(SelDst::Fixed { .. }))).collect();
            assert_eq!(outs.len(), 2, "two output writes: {:?}", s.instrs);
            for o in outs {
                for src in o.srcs.iter().flatten() {
                    if let SelSrc::Virtual { v, .. } = src {
                        assert!(written.contains(v), "an output copies {v:?}, which nothing writes: {:?}", s.instrs);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn a_swizzle_over_a_broadcast_literal_reads_the_same_number() {
        /* FOUND WHILE WRITING `fold_constant_reads`, and it was live: an
         * immediate's swizzle, negate and absolute fields are not modifiers,
         * they are the top 20 bits of the value (`build::Src::imm_payload`).
         * `compose` permuted them like lanes, so
         *
         *     vec4 c = vec4(0.5);  gl_FragColor = c.yzxw * v;
         *
         * compiled to `mul t2, 0.00036621094:f20, t1` -- a shader that runs,
         * draws, and computes with a constant nobody wrote. `alloc.rs` guards
         * the same hazard in two places and says so in both; this path did
         * not.
         *
         * The fix is not to carry the swizzle differently but to drop it: an
         * immediate is a scalar broadcast to all four lanes, so every
         * permutation of them reads the same number.
         *
         * The store/load pair is what puts the literal behind a slot, which
         * is the only way a swizzle reaches `compose` on top of one -- a
         * direct `vec4(0.5).yzxw` never gets that far. */
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "c", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
        let half = 0.5f32.to_bits();
        let instrs = [
            ir::Op::ConstF32([half; 4]),
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(0)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput {
                slot: ir::IoSlot::Position,
                src: ir::Src { val: ir::ValueId(2), swizzle: [1, 2, 0, 3] },
            },
        ];
        let nodes: [ir::Node; 4] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let imm = s
                .instrs
                .iter()
                .flat_map(|i| i.srcs.iter().flatten())
                .find_map(|src| match src {
                    SelSrc::Physical(p) if p.is_immediate() => p.imm_payload_bits(),
                    _ => None,
                })
                .expect("the literal is carried as an immediate");
            assert_eq!(
                f32::from_bits(imm << 12),
                0.5,
                "a swizzle over a broadcast immediate must not move its payload bits"
            );
        })
        .unwrap();
    }

    #[test]
    fn a_local_written_once_costs_one_instruction_and_its_reads_are_free() {
        /* A slot is a virtual register: the store is a write to it under the
         * IR's mask, and a read that the slot still holds folds into its use.
         * This is the shape the front end emits for `float c = ...; use(c);`
         * and for the multi-argument constructors, which are a temporary
         * written under masks and read back once. */
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "c", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(0)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(2)) },
        ];
        let nodes: [ir::Node; 4] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            /* Nothing at all. The slot holds an attribute, which is already in
             * a register, so the read takes that register directly and the
             * store is not emitted -- and the position store is then a copy of
             * register 0 onto itself, which is elided too.
             *
             * This used to be two instructions: a `mov` into the slot and a
             * `mov` out of it. Both were pure loss, and the front end gives
             * every expression temporary a slot, so they were the bulk of the
             * 462 `mov`s this compiler emitted where the reference emits 95. */
            assert!(s.instrs.is_empty(), "a slot holding an attribute costs nothing");
        })
        .unwrap();
    }

    /// Dead-code elimination moves the back edge too, not only copy
    /// propagation.
    ///
    /// The lead's requirement, and the reason is that the two passes share one
    /// compaction: a stale branch target after DCE is the same silent hazard
    /// as a stale one after copy propagation -- a shader that runs and draws
    /// something else. Testing the shared code through only one of its two
    /// callers is testing the caller, not the code.
    ///
    /// The loop body here holds an instruction nothing reads, so DCE deletes
    /// it and every index after it moves, the back edge among them.
    #[test]
    fn dead_code_elimination_moves_the_back_edge_with_the_instruction_it_names() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
            /* 2: computed in the loop and read by nothing. */
            ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(1)); 3]),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range::EMPTY,
                else_: ir::Range { first: 5, len: 1 },
            },
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Break,
            ir::Node::Loop { body: ir::Range { first: 0, len: 5 } },
        ];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            outputs: &outputs,
            body: ir::Range { first: 6, len: 1 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let add = build::op_id("add").expect("the machine has `add`");
            let br = build::op_id("branch").expect("the machine has `branch`");
            /* Vacuity guard: the dead add must actually have been deleted, or
             * the compaction never ran and this proves nothing. */
            /* The one `add` left is the loop guard's counter (loop_convert). */
            assert_eq!(s.instrs.iter().filter(|i| i.op == add).count(), 1, "the unread add is gone");
            assert_eq!(s.instrs[LOOP_TOP as usize].op, add, "and the one left is the guard's, at the loop's top");
            assert_eq!(s.back_edges.len(), 1, "one loop, one back edge");
            let e = s.back_edges[0];
            assert_eq!(s.instrs[e].op, br, "after DCE the index still names the back edge");
            assert_eq!(s.instrs[e].target, Some(LOOP_TOP), "and it still points at the loop's top");
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }

    /// An instruction nothing reads is deleted; one with an effect is not.
    ///
    /// Both directions, because the dangerous half of dead-code elimination is
    /// the deletion it should not make. A write to a FIXED register is an
    /// output or an address -- this file cannot see the read, and that is
    /// exactly why it must not conclude there is none -- and a `texkill`
    /// writes no register at all while killing the fragment.
    #[test]
    fn a_result_nothing_reads_goes_and_an_effect_stays() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            /* 1: computed and never read by anything. */
            ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(0)); 3]),
            /* 2: the output, which must survive. */
            ir::Op::Alu(ir::AluOp::Fmul, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(2)) },
        ];
        let nodes: [ir::Node; 4] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let add = build::op_id("add").expect("the machine has `add`");
            let mul = build::op_id("mul").expect("the machine has `mul`");
            assert!(!s.instrs.iter().any(|i| i.op == add), "nothing reads the add: {:?}", s.instrs);
            assert!(s.instrs.iter().any(|i| i.op == mul), "the multiply feeds the position and stays");
        })
        .unwrap();
    }

    /// `discard` survives dead-code elimination, because its effect is not a
    /// value.
    #[test]
    fn a_discard_is_not_dead_although_it_writes_nothing() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::Discard,
            ir::Op::LoadUniform { dword_off: 0, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes: [ir::Node; 3] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 3 },
            uniform_dwords: 4,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let kill = build::op_id("texkill").expect("the machine has `texkill`");
            assert!(s.instrs.iter().any(|i| i.op == kill), "the discard must survive: {:?}", s.instrs);
        })
        .unwrap();
    }

    /// A slot an `if` arm writes must still be a CHOICE after the join.
    ///
    /// `c = a1; if (t) c = a0; gl_Position = c;` -- the position is
    /// `select(t, a0, a1)`, and it must stay that under every later change to
    /// how slots are represented.
    ///
    /// **Written before the change it guards**, at the lead's instruction and
    /// for a reason today has demonstrated twice: renaming a slot to an
    /// operand makes a store RECORD a name rather than move bits, and the
    /// moment an arm's store is allowed to do that, the name it records is
    /// valid only on that path. Forget the join and the shader takes the then
    /// arm's value unconditionally -- a wrong picture, not a failed compile,
    /// and exactly the shape of the load-time snapshot bug I made this
    /// afternoon.
    ///
    /// The assertion is that BOTH attributes reach the program. If the join
    /// were skipped, attribute 1 would simply vanish; counting instructions
    /// would not notice, and neither would a test that only checked the then
    /// value.
    #[test]
    fn a_slot_an_arm_writes_is_still_a_choice_after_the_join() {
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "c", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(1), comps: 4 },
            /* 3: c = a1, before the `if`. */
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(2)),
                write_mask: build::WRITE_MASK_ALL,
            },
            /* 4: c = a0, inside the then arm. */
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(5)) },
        ];
        /* node 0 is the arm; 1..5 are the body. */
        let nodes = [
            ir::Node::Instr(ir::InstrId(4)),
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 0, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(5)),
            ir::Node::Instr(ir::InstrId(6)),
        ];
        let inputs = [io("a_zero", ir::IoKind::Attribute, 0), io("a_one", ir::IoKind::Attribute, 1)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 1, len: 7 },
            uniform_dwords: 4,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let reads = |reg: u8| {
                s.instrs
                    .iter()
                    .any(|i| i.srcs.iter().flatten().any(|x| matches!(x, SelSrc::Input { reg: r, .. } if *r == reg)))
            };
            assert!(reads(0), "the then arm's value must reach the program: {:?}", s.instrs);
            assert!(
                reads(1),
                "and so must the value from before the `if` -- without it the shader takes the arm unconditionally"
            );
        })
        .unwrap();
    }

    #[test]
    fn a_local_read_before_it_is_written_again_reads_what_was_there_then() {
        /* `c = a1; d = c; c = a0; gl_Position = d;` -- `d` must be `a1`.
         *
         * This test used to store the SAME value twice and assert that a copy
         * was emitted, which made it a test of the mechanism rather than of
         * the hazard: with the same value on both sides, every possible
         * answer is right. Renaming slots to operands (2026-09-22) made the
         * copy unnecessary and the assertion failed -- correctly -- so the
         * fixture now stores two DIFFERENT attributes and asserts the answer
         * instead of the instruction.
         *
         * It is the test that catches looking the slot up when the CONSUMER
         * is selected rather than when the load is: by then the slot holds
         * `a1`, and the position would silently come from the wrong
         * attribute. I wrote that version first. */
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "c", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(1), comps: 4 },
            /* 2: the slot holds attribute ONE... */
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: build::WRITE_MASK_ALL,
            },
            /* 3: ...read it there... */
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            /* 4: ...then put attribute ZERO in it.
             *
             * This way round on purpose: the position register IS register 0
             * and so is attribute 0, so a position written from attribute 0
             * is a copy onto itself and vanishes. Reading the slot at use
             * time would therefore produce an EMPTY program, and "no
             * instructions" is the one wrong answer a weaker fixture could
             * not tell from the right one. */
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(0)),
                write_mask: build::WRITE_MASK_ALL,
            },
            /* 5: the use of what was read at 3, which must be attribute 1. */
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(3)) },
        ];
        let nodes: [ir::Node; 6] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let inputs = [io("a_zero", ir::IoKind::Attribute, 0), io("a_one", ir::IoKind::Attribute, 1)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 6 },
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            /* Whatever it took, the position must read attribute 0's
             * register. Attribute n lands in register n. */
            let from_zero = s.instrs.iter().any(|i| {
                matches!(i.dst, Some(SelDst::Fixed { reg: 0, .. }))
                    && i.srcs.iter().flatten().any(|x| matches!(x, SelSrc::Input { reg: 0, .. }))
            });
            let from_one = s.instrs.iter().any(|i| {
                matches!(i.dst, Some(SelDst::Fixed { reg: 0, .. }))
                    && i.srcs.iter().flatten().any(|x| matches!(x, SelSrc::Input { reg: 1, .. }))
            });
            assert!(from_one, "the position must come from attribute 1: {:?}", s.instrs);
            assert!(!from_zero, "and never from attribute 0, which the slot held later");
        })
        .unwrap();
    }

    #[test]
    fn a_broadcast_literal_is_an_immediate_and_costs_no_constant_register() {
        /* What the reference does: gl_test's ES 1.00 `tex.vert` writes
         * `Unused = 0.5` as `mov t3, void, void, 0.5:f20`, no constant
         * register at all, where we spent four dwords of the file on it. */
        let _s = store::test_lock();
        let half = 0.5f32.to_bits();
        let instrs = [
            ir::Op::ConstF32([half; 4]),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1))];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 2 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert!(s.consts.is_empty(), "an immediate needs no constant file");
            assert_eq!(s.instrs.len(), 1);
        })
        .unwrap();
    }

    #[test]
    fn a_literal_that_would_round_takes_the_constant_file_instead() {
        /* 0.1 has non-zero low mantissa bits, so `imm_f32` refuses it -- and
         * the fall-back must be the constant file, not a rounded immediate.
         * A rounded colour is a picture that is merely wrong. */
        let _s = store::test_lock();
        let tenth = 0.1f32.to_bits();
        let instrs = [
            ir::Op::ConstF32([tenth; 4]),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes = [ir::Node::Instr(ir::InstrId(0)), ir::Node::Instr(ir::InstrId(1))];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 2 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.consts.len(), 4, "a register, because the immediate would have to round");
            assert_eq!(s.consts[0], ConstSlot::literal(tenth), "and it is the value the shader wrote");
        })
        .unwrap();
    }

    #[test]
    fn a_varying_nobody_reads_takes_no_register() {
        /* The front end declares what the source declares; the code generator
         * decides the SET. 27 attributes and varyings are declared and
         * eliminated across the GC7000 table, so this is the common case and
         * not an optimisation for later.
         *
         * Dropping one renumbers every varying after it, which is why the set
         * and the numbering come from one place -- see `used_varying_index`.
         * It is safe across the stage boundary because `link::by_register`
         * pairs the two shaders by NAME, so they may drop different varyings
         * and still match. */
        let _s = store::test_lock();
        let inputs = [io("v_unread", ir::IoKind::Varying, 0), io("v_used", ir::IoKind::Varying, 1)];
        let outputs = [io("gl_FragColor", ir::IoKind::Colour, 0)];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(1), comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(0)) },
        ];
        let nodes: [ir::Node; 2] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            body: ir::Range { first: 0, len: 2 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.io_in.len(), 1, "the unread varying is not in the list the driver reads");
            assert_eq!(s.io_in[0].name, "v_used");
            /* Register 1, not 2: it moved up when the one before it went. */
            assert_eq!(s.io_in[0].reg, 1);
            assert_eq!(s.ps_color_out_reg, Some(1), "and the colour is that varying, in place");
        })
        .unwrap();
    }

    #[test]
    fn a_slot_written_under_a_mask_is_not_propagated() {
        /* A masked store leaves the slot's other lanes holding what they had,
         * which is not the source's -- so the reader cannot simply take the
         * source. `vec4(a.xyz, 1.0)` is built exactly this way, and getting it
         * wrong would put the attribute's w where the 1.0 belongs. */
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "t", ty: ir::Type { scalar: ir::ScalarType::F32, comps: 4 }, array_len: 1 }];
        let inputs = [io("a_pos", ir::IoKind::Attribute, 0)];
        let outputs = [io("gl_Position", ir::IoKind::Position, 0)];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(0)),
                /* xyz only */
                write_mask: 0b0111,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(2)) },
        ];
        let nodes: [ir::Node; 4] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            outputs: &outputs,
            locals: &locals,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert!(!s.instrs.is_empty(), "the masked store is real work and must be emitted");
            match s.instrs[0].dst {
                Some(SelDst::Virtual { write_mask, .. }) => assert_eq!(write_mask, 0b0111),
                other => panic!("{other:?}"),
            }
        })
        .unwrap();
    }

    /// An arm that can see the slot's old value gets a seeding copy; one that
    /// cannot does not.
    ///
    /// The correctness half of per-arm registers. An arm writes its own
    /// register, so the register starts as whatever was in it -- which is
    /// right only if every lane the arm's readers see is one the arm wrote.
    /// Read the slot, or write only some of its lanes, and the rest have to be
    /// copied in first. Getting this wrong draws a picture rather than
    /// failing, so it is asserted in both directions.
    #[test]
    fn an_arm_is_seeded_only_when_it_can_see_the_slot_it_writes() {
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "c", ty: ir::Type::F32X4, array_len: 1 }];

        /* `write_mask` and whether the arm reads the slot are the two knobs;
         * everything else is the same program. */
        let count_movs = |mask: u8, read_first: bool| -> usize {
            let mut instrs: Vec<ir::Op> =
                vec![ir::Op::LoadUniform { dword_off: 0, comps: 1 }, ir::Op::LoadUniform { dword_off: 4, comps: 4 }];
            let mut arm: Vec<ir::Node> = Vec::new();
            if read_first {
                instrs.push(ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 });
                arm.push(ir::Node::Instr(ir::InstrId(2)));
            }
            let store = instrs.len() as u32;
            instrs.push(ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: mask,
            });
            arm.push(ir::Node::Instr(ir::InstrId(store)));
            let out = instrs.len() as u32;
            instrs.push(ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 });
            instrs.push(ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(out)) });

            let arm_len = arm.len() as u32;
            let mut nodes = arm;
            nodes.push(ir::Node::Instr(ir::InstrId(0)));
            nodes.push(ir::Node::Instr(ir::InstrId(1)));
            nodes.push(ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 0, len: arm_len },
                else_: ir::Range::EMPTY,
            });
            nodes.push(ir::Node::Instr(ir::InstrId(out)));
            nodes.push(ir::Node::Instr(ir::InstrId(out + 1)));
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                locals: &locals,
                body: ir::Range { first: arm_len, len: 5 },
                uniform_dwords: 8,
                ..ir::Module::empty(SHC_STAGE_FRAGMENT)
            };
            store::with_scratch(|a| {
                let s = select(&m, a).expect("selects");
                s.instrs.len()
            })
            .unwrap()
        };

        let whole_blind = count_movs(build::WRITE_MASK_ALL, false);
        let whole_reading = count_movs(build::WRITE_MASK_ALL, true);
        let partial_blind = count_movs(0b0011, false);
        assert!(
            whole_reading > whole_blind,
            "an arm that reads the slot needs a seeding copy: {whole_reading} against {whole_blind}"
        );
        assert!(
            partial_blind > whole_blind,
            "so does one that writes only some lanes: {partial_blind} against {whole_blind}"
        );
    }

    /// A leaf `if` is if-converted; one with an `if` in an arm branches.
    ///
    /// The rule that took `hsl_to_rgb`'s vertex shader from 60 temps to 13 and
    /// the corpus from 130 to 137. It is a shape test, not a count test: what
    /// matters is which lowering each `if` got, and counting instructions
    /// would break every time either lowering improves.
    #[test]
    fn a_nested_if_branches_and_a_leaf_if_does_not() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(1)) },
        ];
        let br_un = build::op_id("branch_unary").expect("the machine has `branch_unary`");
        let leaf = ir::Node::If {
            cond: ir::Src::id(ir::ValueId(0)),
            then_: ir::Range { first: 0, len: 1 },
            else_: ir::Range::EMPTY,
        };

        /* One `if` whose arm is a plain instruction: if-converted, no branch. */
        let flat =
            [ir::Node::Instr(ir::InstrId(1)), ir::Node::Instr(ir::InstrId(0)), leaf, ir::Node::Instr(ir::InstrId(2))];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &flat,
            body: ir::Range { first: 1, len: 3 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert!(!s.instrs.iter().any(|i| i.op == br_un), "a leaf `if` is if-converted");
        })
        .unwrap();

        /* The same `if`, now inside another one's arm: the OUTER one branches.
         * Node 2 is the leaf; the outer arm is the single node at 2. */
        let nested = [
            /* 0: the inner arm; 1: the leaf `if`, which is the OUTER one's
             * arm; 2..4: the body. */
            ir::Node::Instr(ir::InstrId(1)),
            leaf,
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 1, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(2)),
        ];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nested,
            body: ir::Range { first: 2, len: 3 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let n = s.instrs.iter().filter(|i| i.op == br_un).count();
            assert_eq!(n, 1, "the outer `if` branches, the inner one does not");
        })
        .unwrap();
    }

    /// A loop with no way out is refused rather than emitted.
    ///
    /// This test used to assert that every loop was refused, and before that
    /// that an `if` was too. What is left is the part that must never stop
    /// being true: a shader that cannot reach its own end wedges this GPU, and
    /// the driver's only answer is a reset -- a lost frame, which in a cluster
    /// is a safety event, so it has to be caught here where it is still a
    /// compile error.
    #[test]
    fn a_loop_with_no_way_out_is_refused_rather_than_emitted() {
        let _s = store::test_lock();
        let nodes = [ir::Node::Loop { body: ir::Range::EMPTY }];
        let m =
            ir::Module { nodes: &nodes, body: ir::Range { first: 0, len: 1 }, ..ir::Module::empty(SHC_STAGE_FRAGMENT) };
        let e = store::with_scratch(|a| select(&m, a).map(|_| ())).unwrap().expect_err("refused");
        assert_eq!(
            e,
            SelectError::Unsupported(
                "a loop with no `break`: it would never end, and a shader that never ends wedges this GPU"
            )
        );
    }

    /// The module the front end builds for `while (c) { ... }`, as IR.
    ///
    /// Shaped exactly as `lower.rs` shapes it -- the exit test inside the
    /// body, as `if (c) {} else { break; }` -- because a loop that passes a
    /// hand-made shape and fails the real one is worth nothing. `work` is the
    /// body's own instruction.
    fn while_loop_module<'a>(instrs: &'a [ir::Op], nodes: &'a [ir::Node], stage: u32) -> ir::Module<'a> {
        ir::Module {
            instrs,
            nodes,
            /* The `Loop` node is the last one; the body is just it. */
            body: ir::Range { first: nodes.len() as u32 - 1, len: 1 },
            uniform_dwords: 8,
            ..ir::Module::empty(stage)
        }
    }

    /// A loop is one branch back to its first instruction, and it says which
    /// branch that is.
    #[test]
    fn a_loop_closes_with_one_branch_back_to_its_top_and_names_it() {
        let _s = store::test_lock();
        let instrs = [
            /* the exit test's value */
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            /* the body's work */
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
        ];
        let nodes = [
            /* 0..2: the loop body */
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range::EMPTY,
                else_: ir::Range { first: 3, len: 1 },
            },
            ir::Node::Instr(ir::InstrId(1)),
            /* 3: the break, the `else` arm of the test */
            ir::Node::Break,
            /* 4: the loop itself */
            ir::Node::Loop { body: ir::Range { first: 0, len: 3 } },
        ];
        let m = while_loop_module(&instrs, &nodes, SHC_STAGE_FRAGMENT);
        store::with_scratch(|a| {
            let s = select(&m, a).expect("a loop selects");
            let br = build::op_id("branch").expect("the machine has `branch`");
            assert_eq!(s.back_edges.len(), 1, "one loop, one back edge");
            let e = s.back_edges[0];
            assert_eq!(s.instrs[e].op, br, "the index names the branch that closes the loop");
            /* 0 is the guard's `mov cnt, 0`, run once before the loop; the
             * loop's first instruction is the guard's `add` (loop_convert). */
            assert_eq!(s.instrs[e].target, Some(LOOP_TOP), "back to the loop's first instruction");
            assert_eq!(s.loop_limits, [LOOP_CAP], "a head test on a uniform: the loop gets the cap");
            /* Every OTHER branch goes forward: the break and the exit test's
             * own jump leave the loop, they do not re-enter it. */
            for (i, ins) in s.instrs.iter().enumerate() {
                if i == e {
                    continue;
                }
                if let Some(t) = ins.target {
                    assert!(t as usize > i, "branch at {i} goes back to {t} without being a back edge");
                }
            }
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }

    /// Copy propagation deletes instructions; the back edge is one of the
    /// things that moves when it does.
    ///
    /// A back edge is named by the INDEX of its branch, and the branch's own
    /// target is an index too. Remapping the targets and forgetting the list
    /// would leave `abi::assemble` permitting a backward jump at some other
    /// instruction's index and refusing the real one -- a shader that compiled
    /// yesterday failing today for a reason nothing in it explains.
    #[test]
    fn copy_propagation_moves_the_back_edge_with_the_instruction_it_names() {
        let _s = store::test_lock();
        let locals = [ir::VarDecl { name: "acc", ty: ir::Type::F32X4, array_len: 1 }];
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadUniform { dword_off: 4, comps: 4 },
            /* A whole-register store: the `mov` this emits is exactly what
             * copy propagation exists to delete. */
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: 0b1111,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(3)) },
        ];
        let nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range::EMPTY,
                else_: ir::Range { first: 6, len: 1 },
            },
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
            ir::Node::Break,
            ir::Node::Loop { body: ir::Range { first: 0, len: 6 } },
        ];
        let m = ir::Module { locals: &locals, ..while_loop_module(&instrs, &nodes, SHC_STAGE_FRAGMENT) };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("a loop with a copy in it selects");
            let br = build::op_id("branch").expect("the machine has `branch`");
            let self_mov = build::op_id("mov").expect("the machine has `mov`");
            /* Vacuity guard, and the whole reason this module is shaped the
             * way it is. Selection emits SEVEN instructions here -- two
             * branches for the exit test, the break's branch, the uniform
             * read, the store to the slot, the colour write and the back edge
             * -- and three passes change it. Copy propagation folds the
             * uniform read into the store that copied it;
             * `fold_constant_reads` makes the colour write read the constant
             * file directly; `legalise_operands` then puts the loop's
             * condition -- a uniform, which no branch in the reference reads
             * -- into a register in front of the branch. Seven out, two
             * deleted, one added, six left, and the back edge ends at 5.
             *
             * All three rewrite the indices, so the remap is exercised in
             * both directions here: deletion moves targets down, insertion
             * moves them up, and the second is the one no other test covers.
             * The counts say which pass did what, so one of them quietly
             * doing another's work would fail rather than net out. */
            /* Plus the loop guard's GUARD instructions in front of the body
             * (loop_convert), which none of the three passes touches. */
            assert_eq!(s.instrs.len(), 6 + GUARD, "seven emitted, two deleted, one added, and the guard");
            assert_eq!(last_mov_sites().folded, 1, "copy propagation deleted one and the fold the other");
            assert_eq!(last_mov_sites().legalised, 1, "and legalisation put the branch's condition in a register");
            /* The condition is materialised in front of the branch that reads
             * it, and the branch reads the register rather than the constant
             * file -- which is the shape the reference's 208 branch operands
             * all have. */
            assert_eq!(s.instrs[GUARD].op, self_mov, "the materialising move comes first in the body");
            assert!(
                matches!(s.instrs[GUARD + 1].srcs[0], Some(SelSrc::Virtual { .. })),
                "the branch reads a temporary, not the constant file"
            );
            /* EVERY target moved with the insertion, which is the half of
             * this test that inserting instructions exercises and deleting
             * them does not: the exit test's two branches now name 3 and 4
             * where they named 2 and 3, and the arm's jump names one past
             * the end. */
            let at = |i: usize| Some((i + GUARD) as u16);
            assert_eq!(s.instrs[GUARD + 1].target, at(3));
            assert_eq!(s.instrs[GUARD + 2].target, at(4));
            assert_eq!(s.instrs[GUARD + 3].target, at(6), "one past the end, which is where an arm falls through to");
            assert_eq!(s.instrs[GUARD - 1].target, at(6), "the guard's own exit also lands one past the end");
            assert_eq!(s.back_edges.len(), 1);
            let e = s.back_edges[0];
            assert_eq!(e, s.instrs.len() - 1, "the back edge is the loop's last instruction");
            assert_eq!(s.instrs[e].op, br, "after compaction the index still names the back edge");
            assert_eq!(s.instrs[e].target, Some(LOOP_TOP), "and it still points at the loop's top");
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }

    /// An arm that must not be run speculatively gets a real branch, never
    /// if-conversion.
    ///
    /// If-conversion runs BOTH arms, and a texture fetch on the wrong side of
    /// the condition is a real fetch. Silently converting it is the worst
    /// outcome available: a picture wrong only where the condition is false.
    /// This asserted a refusal until the branch target existed; now it asserts
    /// the branch, which is the same requirement with the gap closed.
    #[test]
    fn an_arm_holding_a_texture_fetch_gets_a_real_branch_rather_than_speculation() {
        let _s = store::test_lock();
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 4 },
            ir::Op::es100(ir::Es100Tex::Tex2D, 0, ir::Src::id(ir::ValueId(0)), None),
        ];
        let nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                /* The arm's body sits after the two body nodes. */
                then_: ir::Range { first: 2, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(1)),
        ];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: 2 },
            uniform_dwords: 4,
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("a speculation-unsafe arm branches, it does not refuse");
            let br = build::op_id("branch_unary").expect("the machine has `branch_unary`");
            let test = s.instrs.iter().find(|i| i.op == br).expect("a conditional branch was emitted");
            /* `Cond::Not` is the 0xa the reference's `branch_unary` carries:
             * skip the then arm when the condition is false. */
            assert_eq!(test.cond, Some(Cond::Not));
            /* The target must be resolved. A branch whose target stayed 0 is
             * not a broken branch -- it is a jump to the top of the shader,
             * and the hardware would run it. */
            let t = test.target.expect("a branch carries a target");
            assert!(
                t as usize <= s.instrs.len(),
                "target {t} is past the end of a {}-instruction program",
                s.instrs.len()
            );
            assert!(t > 0, "a target of 0 jumps to the top of the shader");
            Ok::<_, SelectError>(())
        })
        .unwrap()
        .unwrap();
    }
}

/// Integer and bool literals, compiled from GLSL through the whole online
/// path and read back as a listing.
#[cfg(test)]
mod int_literal_tests {
    use super::super::vivante::{decode, listing};
    use super::super::{backend, store};
    use crate::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    use core::ffi::c_char;

    /// The listing of a vertex shader and its constant-file data.
    fn listing_of(src: &str) -> (String, Vec<u32>) {
        listing_of_stage(SHC_STAGE_VERTEX, src)
    }

    fn listing_of_stage(stage: u32, src: &str) -> (String, Vec<u32>) {
        let _s = store::test_lock();
        let text = format!("{src}\0");
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let f = backend::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, a log of the size given.
        let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        // SAFETY: the back end NUL-terminates the log.
        let why = unsafe { crate::util::cstr_bytes(log.as_ptr()) };
        assert_eq!(rc, 0, "compiles; rc {rc}: {}", core::str::from_utf8(why).unwrap_or("<not utf8>"));
        let mut t = String::new();
        for (i, q) in out.usc_words().as_chunks::<4>().0.iter().enumerate() {
            let instr = decode::decode_instr(q).expect("our own output decodes");
            listing::instr_text(&mut t, i, &instr).expect("listing");
        }
        let n = out.viv.const_count as usize;
        let consts = if n == 0 {
            Vec::new()
        } else {
            // SAFETY: the shader's own length describes its own region.
            unsafe { core::slice::from_raw_parts(out.viv.const_data, n) }.to_vec()
        };
        (t, consts)
    }

    /// `i < 3` is an integer compare against the INTEGER 3. Until
    /// 2026-09-23 the front end handed the literal over as a float and this
    /// listing read `cmp.lt.s32 ..., 3.0:f20` and `add.s32 ..., 1.0:f20`:
    /// the loop ran twice and `float(k)` read 1.08e9. When the front end
    /// was fixed, `ConstI32` reached this selector for the first time and
    /// was refused ("a non-float literal") on 19 FemtoVG/Slint sources.
    ///
    /// Revert check: restore that refusal in `instr` and remove the two
    /// arms in `src`, and this panics on rc -95.
    #[test]
    fn an_int_literal_is_an_s20_immediate() {
        let (t, _) = listing_of(
            "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nvoid main(){ float a=in0; int k=3; for(int i=0;i<3;i++){ a=a+1.0; } out0=a+float(k); gl_Position=p; }",
        );
        assert!(t.contains("cmp.lt.s32") && t.contains(", 3:s20,"), "the bound is the integer 3:\n{t}");
        assert!(t.contains("add.s32") && t.contains(", 1:s20"), "the step is the integer 1:\n{t}");
        assert!(t.contains("3:s20") && t.contains("i2f.s32"), "float(k) converts the integer 3:\n{t}");
        for line in t.lines().filter(|l| l.contains(".s32")) {
            assert!(!line.contains(":f20"), "no float literal feeds an integer op: {line}\n{t}");
        }
    }

    /// gl_test case 72's shader compiles for this GPU, and no float literal
    /// feeds an integer operation in it. Its colours are checked on the host
    /// by lower.rs's evaluator and on the board by the case itself.
    #[test]
    fn gl_test_intlit_compiles() {
        let src = include_str!("../../../tools/shaders/es100/intlit.frag");
        let (t, _) = listing_of_stage(SHC_STAGE_FRAGMENT, src);
        assert!(t.contains(".s32"), "the shader does integer work:\n{t}");
        for line in t.lines().filter(|l| l.contains(".s32")) {
            assert!(!line.contains(":f20"), "no float literal feeds an integer op: {line}\n{t}");
        }
    }

    /// gl_test case 97's pair compiles for this GPU and links at the reported
    /// limit: fifteen vec4 varyings, fifteen fragment io rows, one short of
    /// the refusal link.rs makes past GL_MAX_VARYING_VECTORS.
    #[test]
    fn gl_test_varyings15_compiles_and_links_at_the_limit() {
        let _s = store::test_lock();
        let compile = |stage: u32, src: &str| {
            let text = format!("{src}\0");
            let mut out = ShcShader::ZERO;
            let mut log = [0 as c_char; 512];
            let f = backend::SHC_ONLINE.compile.expect("the online back end compiles");
            // SAFETY: a NUL-terminated source, a real output, a log of the size given.
            let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
            // SAFETY: the back end NUL-terminates the log.
            let why = unsafe { crate::util::cstr_bytes(log.as_ptr()) };
            assert_eq!(rc, 0, "compiles: {}", core::str::from_utf8(why).unwrap_or("?"));
            out
        };
        let vs = compile(SHC_STAGE_VERTEX, include_str!("../../../tools/shaders/es100/varyings15.vert"));
        let mut fs = compile(SHC_STAGE_FRAGMENT, include_str!("../../../tools/shaders/es100/varyings15.frag"));
        assert_eq!(fs.viv.num_in, 15, "fifteen fragment io rows");
        assert_eq!(fs.viv.num_in as usize, crate::gl::context::MAX_VARYING_VECTORS, "exactly the reported limit");
        let mut log = [0 as c_char; 256];
        // SAFETY: two distinct valid shaders and a log of the stated size.
        let rc = unsafe { crate::shaderc::link::link_shaders_viv(&vs, &mut fs, log.as_mut_ptr(), log.len(), "test") };
        // SAFETY: NUL-terminated by the writer.
        let why = unsafe { crate::util::cstr_bytes(log.as_ptr()) };
        assert_eq!(rc, 0, "links: {}", core::str::from_utf8(why).unwrap_or("?"));
    }

    /// `gl_FrontFacing` is one `cmp.ne.s32` of internal register 0 against
    /// the integer 0, true all ones -- the reference's form -- and a shader
    /// reading it as a bool through `?:` selects on that mask.
    #[test]
    fn gl_front_facing_is_internal_register_0_compared_with_zero() {
        let src = "precision mediump float;\nvoid main(){ gl_FragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0); }";
        let (t, _) = listing_of_stage(SHC_STAGE_FRAGMENT, src);
        let cmp = t.lines().find(|l| l.contains("cmp.ne.s32")).unwrap_or_else(|| panic!("a cmp.ne.s32:\n{t}"));
        assert!(cmp.contains(" 0:s20,") && cmp.contains("-1:s20"), "against 0, true all ones: {cmp}\n{t}");
        assert!(t.contains("select"), "the ?: selects on the mask:\n{t}");
    }

    /// A literal an s20 immediate cannot hold goes to the constant file as
    /// its 32-bit pattern, and a bool is the all-ones mask the comparisons
    /// already produce (`-1:s20`).
    #[test]
    fn a_wide_int_and_a_bool_literal() {
        let (t, consts) = listing_of(
            "attribute vec4 p;\nattribute float in0;\nvarying float out0;\nvoid main(){ int k=int(in0)+1000000; bvec2 b=bvec2(true,false); out0=float(k)+(b.x?1.0:0.0)+(b.y?2.0:0.0); gl_Position=p; }",
        );
        assert!(consts.contains(&1_000_000), "1000000 is in the constant file as an integer: {consts:x?}\n{t}");
        assert!(t.contains("-1:s20") && t.contains(" 0:s20"), "true is all ones, false zero:\n{t}");
    }
}

/// The loop guard (`loop_convert`, gl_test 104): every loop carries a hidden
/// counter of its head entries, its own count for a constant `for` loop and
/// LOOP_CAP for any other.
#[cfg(test)]
mod loop_guard_tests {
    use super::super::sema::Stage;
    use super::super::{eval, lower, store};
    use super::*;

    /// Select `src` (a fragment shader) and hand `f` the IR and the selection.
    fn with_selected<T>(src: &str, f: impl FnOnce(&ir::Module<'_>, &Selected<'_>) -> T) -> T {
        let _l = store::test_lock();
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let m = a.alloc_copy(core::slice::from_ref(&l.module)).unwrap();
            let m: &ir::Module<'_> = &m[0];
            let sel = select(m, a).expect("selects");
            f(m, &sel)
        })
        .unwrap()
    }

    /// What the IR evaluator stores to gl_FragColor with uniform dword 0 = `u`.
    fn colour(m: &ir::Module<'_>, u: f32) -> f32 {
        let uniforms = [u.to_bits(), 0, 0, 0];
        let out = eval::run(m, &eval::Inputs { io: &[], uniforms: &uniforms }).expect("the evaluator runs it");
        out.output_f32(ir::IoSlot::FragColor(0)).expect("a colour")[0]
    }

    /// The guard's compare value for loop `k`: the `limit + 1` its `cmp.ge`
    /// reads, found at the instruction after the loop's top `add`.
    fn guard_compare(s: &Selected<'_>) -> Vec<u32> {
        let cmp = build::op_id("cmp").unwrap();
        let add = build::op_id("add").unwrap();
        let mut v = Vec::new();
        for (i, ins) in s.instrs.iter().enumerate() {
            if ins.op == add && s.instrs.get(i + 1).is_some_and(|c| c.op == cmp && c.cond == Some(Cond::Ge)) {
                if let Some(SelSrc::Physical(p)) = s.instrs[i + 1].srcs.iter().flatten().nth(1).copied() {
                    let bits = p.imm_payload_bits().unwrap_or(u32::MAX);
                    v.push(((bits << 12) as i32 >> 12) as u32);
                }
            }
        }
        v
    }

    /// gl_test 104's band 7 bound (1e9, read per pixel) is no constant: the
    /// loop gets LOOP_CAP, and the guard's exit lands past the back edge.
    /// Not under report-es2: divloop.frag is `#version 300 es`, which an
    /// ES 2.0-reporting front end refuses. The cap on a data-dependent loop
    /// stays covered there by the `while (true)` case of
    /// `the_recognizer_reads_constant_for_loops_and_nothing_else`.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn gl_test_104s_loop_gets_the_cap_and_leaves_past_its_back_edge() {
        let src = include_str!("../../../tools/shaders/es300/divloop.frag");
        with_selected(src, |_, s| {
            assert_eq!(s.loop_limits, [LOOP_CAP], "divloop's loop: limit {:?}, want the cap {LOOP_CAP}", s.loop_limits);
            assert_eq!(guard_compare(s), [LOOP_CAP + 1], "the guard compares against the cap + 1");
            let e = *s.back_edges.last().expect("a back edge");
            let top = s.instrs[e].target.expect("the back edge has a target") as usize;
            let exit = s.instrs[top + 3].target.expect("the guard's exit branch has a target") as usize;
            assert_eq!(exit, e + 1, "the guard leaves to just past the back edge (limit {})", s.loop_limits[0]);
        });
    }

    /// A constant `for` longer than LOOP_CAP keeps its own count: 5000
    /// iterations, 5001 head entries, and the evaluator's sum is exact. The
    /// PowerVR and the V3D cut this loop at 4096 (their unroller simulates
    /// 256 passes, then caps).
    #[test]
    fn a_constant_loop_of_5000_keeps_its_count_and_is_exact() {
        let src = "precision highp float; uniform vec4 u; void main(){ float s = 0.0; \
                   for (int i = 0; i < 5000; i++) { s += u.x; } gl_FragColor = vec4(s); }";
        with_selected(src, |m, s| {
            assert_eq!(
                s.loop_limits,
                [5001],
                "recognized: 5000 iterations + the failing entry, got {:?}",
                s.loop_limits
            );
            assert_eq!(guard_compare(s), [5002], "guard compare = limit + 1 for limit {}", s.loop_limits[0]);
            assert_eq!(colour(m, 1.0), 5000.0, "5000 passes in the evaluator");
        });
    }

    /// A 10-iteration loop: recognized (11 entries), exact, and its guard
    /// compares against 12, which the counter never reaches.
    #[test]
    fn a_ten_iteration_loop_is_exact_and_its_guard_never_fires() {
        let src = "precision highp float; uniform vec4 u; void main(){ float s = 0.0; \
                   for (int i = 0; i < 10; i++) { s += u.x; } gl_FragColor = vec4(s); }";
        with_selected(src, |m, s| {
            assert_eq!(s.loop_limits, [11], "10 iterations + the failing entry, got {:?}", s.loop_limits);
            assert_eq!(guard_compare(s), [12], "never reached: at most 11 entries (limit {})", s.loop_limits[0]);
            assert_eq!(colour(m, 0.5), 5.0);
        });
    }

    /// The boundary shared with the PowerVR and the V3D (loop_count.rs
    /// EXACT_PASSES_MAX): 65,536 passes keep their count, 65,537 get the cap.
    #[test]
    fn the_exact_boundary_is_65536_passes_on_the_gc7000_too() {
        let src = |n: u32| {
            std::format!("precision highp float; uniform vec4 u; void main(){{ float s = 0.0; for (int i = 0; i < {n}; i++) {{ s += u.x; }} gl_FragColor = vec4(s); }}")
        };
        with_selected(&src(65_536), |_, s| {
            assert_eq!(s.loop_limits, [65_537], "65,536 passes: exact, {:?}", s.loop_limits)
        });
        with_selected(&src(65_537), |_, s| {
            assert_eq!(s.loop_limits, [LOOP_CAP], "65,537 passes: capped, {:?}", s.loop_limits)
        });
    }

    /// The forms the recognizer reads, and ones it must not: a float index,
    /// `<=` with a step of 2, the index on the right, a `continue` (its step
    /// repeated before it). A `while (true)` with a break on data, and a loop
    /// whose index the body writes, get the cap.
    #[test]
    fn the_recognizer_reads_constant_for_loops_and_nothing_else() {
        let cases: [(&str, u32); 6] = [
            ("for (float i = 1.0; i <= 24.0; i += 1.0) { s += u.x; }", 25),
            ("for (int i = 3; i <= 20; i += 2) { s += u.x; }", 10),
            ("for (int i = 0; 7 > i; i++) { s += u.x; }", 8),
            ("for (int i = 0; i < 6; i++) { if (u.y > 0.5) continue; s += u.x; }", 7),
            ("while (true) { if (s > u.x) break; s += 1.0; }", LOOP_CAP),
            ("for (int i = 0; i < 6; i++) { s += u.x; if (s > 2.0) i = 7; }", LOOP_CAP),
        ];
        for (body, want) in cases {
            let src = std::format!(
                "precision highp float; uniform vec4 u; void main(){{ float s = 0.0; {body} gl_FragColor = vec4(s); }}"
            );
            with_selected(&src, |_, s| {
                assert_eq!(s.loop_limits, [want], "{body}: limit {:?}, want {want}", s.loop_limits);
            });
        }
    }

    /// dEQP tmp_array vec2/vec3_const_write_static_loop_read_vertex, reduced:
    /// literal array elements read through a loop index. A `vec2` or `vec3`
    /// element is stored under a partial write mask (`mov t.xy, 4.0`), and
    /// `fold_constant_reads` used to take whole-register seeds only, so each
    /// one kept a register live across the loop (45 temporaries on the
    /// vertex stage against its ceiling of 21: "Failed to compile") where the
    /// `vec4` variant folds every one into an immediate. Now all widths fold
    /// alike: no seed of a literal element survives, and the value is exact.
    #[test]
    fn a_vec2_or_vec3_literal_element_read_in_a_loop_folds_like_a_vec4_one() {
        /* Immediate seeds before the loop's top: the accumulator's zero, the
         * write index's 1, the loop index's and the guard's starts -- and,
         * unfolded, one per literal element. */
        let imm_seeds = |s: &Selected<'_>| {
            let mov = build::op_id("mov").unwrap();
            let e = *s.back_edges.last().expect("a back edge");
            let top = s.instrs[e].target.expect("the back edge has a target") as usize;
            s.instrs[..top]
                .iter()
                .filter(|i| {
                    i.op == mov
                        && matches!(i.dst, Some(SelDst::Virtual { .. }))
                        && matches!(i.srcs.iter().flatten().next(), Some(SelSrc::Physical(p)) if p.is_immediate())
                })
                .count()
        };
        let mut seeds = Vec::new();
        for (ty, ctor, out) in
            [("vec2", "vec2", "vec4(r, 0.0, 1.0)"), ("vec3", "vec3", "vec4(r, 1.0)"), ("vec4", "vec4", "r")]
        {
            let mut src = std::format!(
                "precision mediump float; uniform vec4 u; void main() {{ {ty} arr[8]; arr[0] = {ty}(u); int i = 1;"
            );
            for k in 1..8 {
                src += &std::format!(" arr[i++] = {ctor}({k}.0);");
            }
            src +=
                &std::format!(" {ty} r = {ty}(0.0); for (int i = 0; i < 8; i++) r += arr[i]; gl_FragColor = {out}; }}");
            with_selected(&src, |m, s| {
                seeds.push((ty, imm_seeds(s)));
                /* u.x = 0.5: 0.5 + 1 + 2 + ... + 7 = 28.5 */
                assert_eq!(colour(m, 0.5), 28.5, "{ty}: the sum through the loop");
            });
        }
        let vec4 = seeds[2].1;
        for (ty, n) in &seeds {
            assert_eq!(
                *n, vec4,
                "{ty}: {n} literal seeds left in registers, the vec4 variant leaves {vec4}: {seeds:?}"
            );
        }
        assert_eq!(vec4, 4, "the accumulator, the write index and the two counters: {seeds:?}");
    }
}
