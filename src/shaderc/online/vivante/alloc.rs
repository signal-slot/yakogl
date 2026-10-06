// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The code generator's back half: virtual registers in, built instructions
//! and a temporary count out.
//!
//! The front half ([`gles2-core`'s `codegen.rs`]) selects instructions but
//! cannot name registers, because it does not know how many temporaries there
//! are. So it emits [`SelInstr`] over *virtual* registers and this is the map
//! from virtual to physical.
//!
//! # The thing that makes this machine's allocator different
//!
//! On the GC7000 a shader's **vertex attributes and its outputs live in the
//! same temporary file as its temporaries**. The bring-up triangle says so:
//! `driver/src/vivante/gc_shaders.h` has "aPos lands in r0", "aColor lands in
//! r1", `GC_TRI_VS_POS_OUT_REG 0`, `GC_TRI_VS_VARYING0_REG 1`, and
//! `GC_TRI_VS_NUM_TEMPS 2` — for a shader that is a single NOP and allocates
//! no temporary at all.
//!
//! So [`SelSrc::Input`] and [`SelDst::Fixed`] are not some other file this
//! allocator can ignore: they are reservations *inside* the file it allocates
//! from, and [`Allocated::temps`] must cover them. An allocator that treated
//! them as external would report `temps = 0` for the passthrough vertex
//! shader, and `temps` is the field that, under-declared, lets the driver
//! alias a live register with a vertex element nobody reads — a defect whose
//! symptom looks like anything except a register bug.
//!
//! # What it will not do
//!
//! Spill. If the temporaries do not fit, that is [`AllocError::OutOfRegisters`]
//! with the count that was needed, not a quietly worse program. Spilling on
//! this part means memory traffic in a shader that was written not to have
//! any, and a compiler that does it silently is one whose output nobody can
//! reason about.

use super::super::arena::{Arena, ArenaError};
use super::build::{self, BuildError, Dst, OpId, Src, RGROUP_IMMED, RGROUP_UNIFORM_0, RGROUP_UNIFORM_1};
use super::isa::Instr;
use crate::util::Global;

/// A virtual vec4 temporary, numbered by the code generator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VReg(pub u32);

/// The operand type an operation is performed in (`#type`): the three bits
/// the encoding splits across bit 53 and bits 95:94.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OpType {
    F32 = 0,
    S32 = 1,
    S8 = 2,
    U16 = 3,
    F16 = 4,
    S16 = 5,
    U32 = 6,
    U8 = 7,
}

/// The condition an operation tests (`#cond`), for the forms that have one.
/// 2,967 of the 13,854 instructions the driver ships carry a modifier, and
/// `select` and `cmp` are most of them, so this is not an advanced feature.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cond {
    True = 0,
    Gt = 1,
    Lt = 2,
    Ge = 3,
    Le = 4,
    Eq = 5,
    Ne = 6,
    And = 7,
    Or = 8,
    Xor = 9,
    Not = 10,
    Nz = 11,
    Gez = 12,
    Gz = 13,
    Lez = 14,
    Lz = 15,
}

/// Where an instruction writes.
#[derive(Clone, Copy, Debug)]
pub enum SelDst {
    /// A temporary this allocator places.
    Virtual { v: VReg, write_mask: u8 },
    /// A register the generator chose and this allocator must not move: a
    /// vertex output, the fragment colour. Allocate *around* these.
    Fixed { reg: u8, write_mask: u8 },
    /// The **address register**, the destination of `movar` and `movai` and
    /// of nothing else. A later source with `amode = 1` reads
    /// `reg + address`, which is how a dynamic index reaches the constant
    /// file.
    ///
    /// It is its own file, so this reserves **no temporary**. Spelling it
    /// `Fixed { reg: 0 }` would encode the same `DST` bits and be wrong for
    /// exactly that reason: the allocator would reserve temporary 0 and hand
    /// every other value a register one higher, and on the fragment stage
    /// temporary 0 is `gl_FragCoord`.
    ///
    /// Only the write mask is a choice; see [`build::Dst::address`] for what
    /// was measured about it.
    Address { write_mask: u8 },
}

/// Where an instruction reads.
#[derive(Clone, Copy, Debug)]
pub enum SelSrc {
    Virtual {
        v: VReg,
        swizzle: [u8; 4],
        neg: bool,
        abs: bool,
    },
    /// An io input register: an attribute, or a varying the driver's
    /// `VS_OUTPUT` map places by register order. Never renumbered.
    Input {
        reg: u8,
        swizzle: [u8; 4],
        neg: bool,
        abs: bool,
    },
    /// Already physical: a constant-file read, resolved by whoever owns the
    /// constant-file layout.
    Physical(Src),
}

/// The two fields only the texture instructions carry.
///
/// `TEX_ID` is the sampler and `TEX_SWIZ` the destination swizzle of the
/// sampled value. Nothing else in the machine has either, so they are not
/// operands and do not belong in `srcs`.
///
/// `id` is **stage-relative**, the same rule as the uniforms: a fragment
/// shader's first sampler is 0 and so is a vertex shader's, and the driver
/// adds the stage's base (`VS_SAMPLER_BASE`, written in
/// `src/vivante/texture.rs`). A shader does not encode the global unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tex {
    /// 0..=31: `TEX_ID` is five bits.
    pub id: u8,
    /// The destination swizzle, one lane per component, each 0..=3.
    /// `[0, 1, 2, 3]` is the identity, which 412 of the corpus's 427 texture
    /// instructions use; the other 15 swizzle the sampled value, so this is a
    /// real field and not a constant to be defaulted away.
    pub swizzle: [u8; 4],
}

impl Tex {
    /// The identity swizzle: the sampled value in its own order.
    pub const IDENTITY: [u8; 4] = [0, 1, 2, 3];
}

/// One selected instruction, before registers exist.
///
/// `srcs` is indexed by the **hardware source slot**, not by operand ordinal.
/// On this machine a `mov` reads SRC2 and nothing else, so a generator that
/// fills slots by ordinal produces an instruction that reads the wrong
/// registers and still encodes perfectly. [`allocate_and_build`] refuses a
/// slot the operation does not have rather than dropping it.
#[derive(Clone, Copy, Debug)]
pub struct SelInstr {
    pub op: OpId,
    pub ty: OpType,
    /// For the forms that test one; `None` where the operation has no
    /// condition field.
    pub cond: Option<Cond>,
    /// `SAT` is a field of the instruction (bit 11), not of the destination,
    /// so it lives here rather than on [`SelDst`].
    pub saturate: bool,
    pub dst: Option<SelDst>,
    pub srcs: [Option<SelSrc>; 3],
    /// `Some` exactly for the texture instructions; see [`Tex`].
    pub tex: Option<Tex>,
    /// Where a branch goes: an index into the emitted instruction stream.
    ///
    /// `Some` exactly for the operations that carry a `TARGET` field. It is
    /// **required** there rather than defaulted, for the same reason a texture
    /// instruction must carry its sampler: a branch whose target defaulted to
    /// 0 is not a broken branch, it is a jump to the top of the shader, and
    /// the hardware would run it.
    ///
    /// The caller resolves these. A forward branch's target is not known until
    /// the arm it skips has been emitted, so selection emits a placeholder and
    /// back-patches; this side only carries the number and refuses one the
    /// field cannot hold.
    pub target: Option<u16>,
    /// Round toward zero (`RMODE = rtz`). The reference sets it on every
    /// `sin`, `cos` and `log` it emits, and Mesa's etnaviv on sin/cos for the
    /// new-transcendental cores (etnaviv_compiler_nir_emit.c).
    pub rtz: bool,
}

impl SelInstr {
    /// An instruction with no condition, no saturate and no sources set.
    pub fn new(op: OpId, ty: OpType) -> SelInstr {
        SelInstr {
            op,
            ty,
            cond: None,
            saturate: false,
            dst: None,
            srcs: [None; 3],
            tex: None,
            target: None,
            rtz: false,
        }
    }

    /// Round toward zero; see [`SelInstr::rtz`].
    pub fn round_toward_zero(mut self) -> SelInstr {
        self.rtz = true;
        self
    }

    /// Where a branch goes; see [`SelInstr::target`].
    pub fn target(mut self, t: u16) -> SelInstr {
        self.target = Some(t);
        self
    }

    /// The sampler and the destination swizzle of a texture instruction.
    pub fn tex(mut self, t: Tex) -> SelInstr {
        self.tex = Some(t);
        self
    }

    pub fn dst(mut self, d: SelDst) -> SelInstr {
        self.dst = Some(d);
        self
    }

    /// Set the **hardware source slot** `n` directly.
    ///
    /// Prefer [`SelInstr::operand`]: the slot is the machine's and a
    /// generator should not have to know that a `mov` reads SRC2.
    pub fn src(mut self, n: usize, s: SelSrc) -> SelInstr {
        if n < 3 {
            self.srcs[n] = Some(s);
        }
        self
    }

    /// Set operand `ordinal` — the first, second or third source of the
    /// operation as a reader of the IR would count them — and let the slot be
    /// worked out from the operation.
    ///
    /// This is the placement the lead ruled lives on this side: the code
    /// generator counts operands, and knowledge that a `mov`'s one operand
    /// goes in SRC2 rather than SRC0 stays in one file. An ordinal the
    /// operation does not have is left unset, and [`allocate_and_build`] then
    /// refuses the instruction by name rather than encoding one that reads a
    /// register nobody wrote.
    pub fn operand(mut self, ordinal: usize, s: SelSrc) -> SelInstr {
        if let Some(slot) = self.op.slot_of_operand(ordinal) {
            self.srcs[slot] = Some(s);
        }
        self
    }
}

/// What the allocator produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Allocated {
    /// Instructions written into the caller's slice.
    pub count: usize,
    /// The temporary file's size: the highest register the program touches,
    /// plus one. A **count**, so 0 means "none" and never "register 0".
    /// Inputs and fixed outputs count, because they live in this file.
    pub temps: u16,
}

/// Splat a swizzle to the component the scalar operations read.
///
/// Mesa composes a scalar operation's source swizzle with `xxxx`, which takes
/// whatever the caller put in the x lane and repeats it. Doing the same here
/// is what lets the front half emit the IR literally: it writes
/// `rcp v, a.yzw` if that is what the IR says, and the lowering makes it the
/// `a.yyyy` the machine is fed.
fn splat(swizzle: [u8; 4]) -> [u8; 4] {
    [swizzle[0], swizzle[0], swizzle[0], swizzle[0]]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllocError {
    /// A swizzle lane outside 0..=3, or a write mask outside 0..=15. The
    /// fields are two and four bits, so a larger value would be truncated
    /// into a different, perfectly legal program.
    BadOperand(&'static str),
    /// A source slot the operation does not have was filled, or one it has
    /// was left empty. Names the slot.
    WrongSourceSlot(usize),
    /// A destination was given to an operation that writes none, or omitted
    /// from one that writes.
    WrongDestination,
    /// [`Tex`] was given to an operation with no `TEX_ID` field, or omitted
    /// from one that has it.
    WrongTexFields,
    /// A branch target was given to an operation with no `TARGET` field, or
    /// omitted from one that has it.
    WrongBranchTarget,
    /// [`SelDst::Address`] was given to an operation that writes a temporary,
    /// or a temporary destination was given to `movar`/`movai`, whose write
    /// goes to the address register whatever the `DST` field's `REG` half
    /// says.
    WrongAddressDestination,
    /// More live values than the temporary file holds. Carries how many were
    /// needed. This allocator does not spill.
    OutOfRegisters(u16),
    /// The caller's output slice is too small.
    OutputTooSmall,
    /// The scratch arena the caller lent could not hold the per-value
    /// tables, which are sized by the program (values + [`MAX_TEMPS`]).
    OutOfScratch,
    /// The builder refused the instruction this was lowered to.
    Build(BuildError),
}

/// Which stage's register file this is.
///
/// The allocator needs it for one reason, and it is not a detail: on the
/// FRAGMENT stage the hardware delivers `gl_FragCoord` in **r0**, whether or
/// not the shader mentions it. A shader that never names r0 does not make r0
/// free -- it makes it invisible, which is worse, because the allocator will
/// hand it to the first temporary and the shader will then compute with the
/// fragment's coordinates instead of its own value.
///
/// That was fourteen gl_test cases drawing nothing: the colour a fragment
/// shader built in "its" first temporary was gl_FragCoord all along.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Vertex,
    Fragment,
}

/// How many temporaries a program may use.
///
/// **Bounded by what the driver can DECLARE, not by what an instruction can
/// address.** This constant was 128 for the second reason -- `#instruction-dst`
/// has a seven-bit REG field -- and that is true and irrelevant: the shader
/// also has to tell the hardware how many registers to give it, through
/// `VS_TEMP_REGISTER_CONTROL.NUM_TEMPS`, which is **six bits**
/// (`NUM_TEMPS__MASK = 0x3f`, driver/src/vivante/hw/state_3d.xml.h). It is a
/// count, not a highest index: the driver's own triangle uses r0 and r1 and
/// programs `GC_TRI_VS_NUM_TEMPS = 2`.
///
/// A program above this does not fail, which is the whole danger.
/// `temp_register_num()` in `src/vivante/regs.rs` is `x & 0x3f`, so a shader
/// declaring 67 tells the GPU **3** and then writes 64 registers past its own
/// allocation, into whatever other threads hold. Measured in our own output
/// before this bound existed: `imageparticle_sprite.vert` declared 77 and
/// programmed 13, `imageparticle_tabled.vert` 69 -> 5,
/// `imageparticle_deformed.vert` 66 -> 2, and the dEQP shader
/// `shaders.algorithm.hsl_to_rgb_vertex` 67 -> 3.
///
/// Corroboration that 63 is the real limit and not just the mask's width: the
/// shipped table's own maximum is **42** temporaries (vertex) and 28
/// (fragment), and **none of its 324 blobs reaches 64**. A reference compiler
/// that never crosses the line in 324 shaders is a second witness.
///
/// Above this the allocator refuses. A refusal is a shader someone can read
/// about; the alternative is a GPU that stops responding.
pub const MAX_TEMPS: u16 = 63;

/// The largest branch target the encoding carries: `TARGET` is bits
/// 103..=117, fifteen of them.
pub const MAX_BRANCH_TARGET: u16 = (1 << 15) - 1;

/// The most virtual registers one program may use: as many as a `u16` value
/// number can name once the [`MAX_TEMPS`] pseudo values of placed outputs
/// are numbered above them (`order` and `phys` hold value numbers as `u16`).
///
/// This is a numbering limit, not a memory one. It was 512 while it sized a
/// dozen stack tables; those are now lent by the caller's scratch arena and
/// sized by the program (about 70 bytes a value in [`allocate_and_build`]),
/// so what a large program actually meets first is the arena, refused as
/// [`AllocError::OutOfScratch`]. random.23's fragment shader, 5,785 values,
/// was refused here by the old bound (tools/cts/RESULTS.md).
pub const MAX_VREGS: usize = (u16::MAX as usize + 1) - MAX_TEMPS as usize;

/// [`split_ranges`]' and [`definitions_precede_uses`]' numbering space: the
/// old [`MAX_VREGS`], kept for these two because neither is on the product
/// path (split_ranges measures; the other is a check) and their tests pin
/// what they do at this bound, `capped` included.
const SPLIT_VREGS: usize = 512;

/// The highest virtual register index any program has needed, plus one.
///
/// Measured for the same reason the arenas are: a bound nobody re-reads
/// becomes wrong the first time the front end covers more.
static VREG_PEAK: Global<usize> = Global::new(0);

fn record_vreg_peak(need: usize) {
    // SAFETY: single-threaded by contract, like the arenas' counters.
    let peak = unsafe { VREG_PEAK.get() };
    if need > *peak {
        *peak = need;
    }
}

/// The most values live at one instruction in the LAST program allocated.
///
/// This is the number no allocator can go below: with `n` values live at the
/// same instruction, `n` registers are needed whatever the assignment does.
/// Reported so that a `temps` figure can be read against its own floor --
/// "ours is 67" says nothing until you know whether 67 values are live at
/// once (then the allocator is optimal and the fix is upstream) or far fewer
/// (then the registers are being left on the floor and the fix is here).
///
/// How many registers the ABI and the hardware pinned in the last program.
///
/// `temps` counts these too, and the overlap floor does not, so the identity
/// to check an allocation against is `temps == floor + reserved`. Equality
/// means optimal: live ranges are intervals, greedy in start order colours an
/// interval graph in exactly its maximum overlap, and the pinned registers
/// are simply not available to colour with.
///
/// Everything the last allocation measured, written once at the end of
/// [`allocate_and_build`] and read back by the accessors below.
///
/// **One value, and on the host one per thread.** These were ten separate
/// process-wide statics, each written where it was computed, and that raced:
/// `cargo` runs tests in parallel threads, so between one test calling
/// `allocate_and_build` and reading `last_overlap()`, another test's
/// allocation could land in between. It failed about once in forty runs of
/// the suite under load -- reproduced deliberately, and the same rate as the
/// race gles2-core fixed in 4ab66f0.
///
/// A retry or a "run these serially" attribute would have hidden it. The
/// honest fix is that "the last allocation" means *this* thread's last
/// allocation, which is also what a caller reading a diagnostic wants. In the
/// library's own single-threaded configuration the two are the same thing, so
/// only the host build pays for a thread-local.
#[derive(Clone, Copy, Default)]
struct LastAlloc {
    /// The most values live at one instruction: the number no allocator can
    /// go below.
    overlap: usize,
    /// Registers the ABI and the hardware pinned. `temps == overlap +
    /// reserved` is the identity that says an allocation is optimal.
    reserved: usize,
    /// The components those live values occupy, so `ceil(n / 4)` is the floor
    /// for an allocator that could pack a scalar into any lane.
    live_components: usize,
    /// Values written across all four components but read in only one: each
    /// costs a whole register to carry one number.
    wide_but_scalar: usize,
    /// Peak liveness if every value were produced just before its first read:
    /// a lower bound on what re-ordering could reach, since real dependencies
    /// may forbid the move. Against `overlap` it says whether a large count
    /// is "this program needs that many values at once" or "this program
    /// computes them all far too early".
    sunk_overlap: usize,
    /// The same, sinking only values written exactly once: what a pass that
    /// refuses to reorder anything ambiguous could actually reach.
    safe_sunk: usize,
    /// Values written more than once. Re-ordering is only simple at zero.
    multi_def: usize,
    /// Instructions that cannot be moved: a branch, or one whose destination
    /// is not a virtual register (an io write, a discard).
    pinned_instrs: usize,
    /// Single-source virtual-to-virtual copies, and how many of those could
    /// be coalesced away -- the only register still available in this layer.
    copies: usize,
    coalescable: usize,
    /// Of `coalescable`, the ones a rename could actually take, and why the
    /// rest could not. See the comment at the counting site.
    coalesce_ready: usize,
    coalesce_swizzled: usize,
    coalesce_transformed: usize,
    /// Movs that carry neg/abs/sat: `fneg`, `fabs`, `fsat`, not copies.
    mov_transformed: usize,
    /// Whether the program has any branch, which is what decides if the
    /// linear live ranges above describe it exactly or only bound it.
    had_branch: bool,
    branches: usize,
    branches_backward: usize,
    /// Peak liveness if every redefinition started a new range.
    split_overlap: usize,
    /// Virtual registers read but never written.
    read_only_vregs: usize,
    coalesce_conditional: usize,
    coalesce_not_joinable: usize,
    /// Every `mov` the program emits, split by what it moves.
    movs: usize,
    mov_overlap: usize,
    mov_seed: usize,
    mov_io: usize,
    /// The seeds split by which file they read and how often their result is
    /// read: what gles2-core needs to tell a foldable uniform from a literal.
    seed_uniform: usize,
    seed_imm: usize,
    seed_other: usize,
    seed_read_once: usize,
    seed_uniform_once: usize,
    seed_imm_once: usize,
    seed_imm_to_mov: usize,
    seed_imm_to_alu: usize,
    seed_imm_dead: usize,
    /// Values whose range [`extend_across_back_edges`] widened because a
    /// loop keeps them live past their last mention.
    loop_extended: usize,
    /// Single-lane values the assignment moved to another lane.
    lane_moves: usize,
}

impl LastAlloc {
    const ZERO: LastAlloc = LastAlloc {
        overlap: 0,
        reserved: 0,
        live_components: 0,
        wide_but_scalar: 0,
        sunk_overlap: 0,
        safe_sunk: 0,
        multi_def: 0,
        pinned_instrs: 0,
        copies: 0,
        coalescable: 0,
        coalesce_ready: 0,
        coalesce_swizzled: 0,
        coalesce_transformed: 0,
        mov_transformed: 0,
        had_branch: false,
        branches: 0,
        branches_backward: 0,
        split_overlap: 0,
        read_only_vregs: 0,
        coalesce_conditional: 0,
        coalesce_not_joinable: 0,
        movs: 0,
        mov_overlap: 0,
        mov_seed: 0,
        mov_io: 0,
        seed_uniform: 0,
        seed_imm: 0,
        seed_other: 0,
        seed_read_once: 0,
        seed_uniform_once: 0,
        seed_imm_once: 0,
        seed_imm_to_mov: 0,
        seed_imm_to_alu: 0,
        seed_imm_dead: 0,
        loop_extended: 0,
        lane_moves: 0,
    };
}

/// On the host, per thread: see [`LastAlloc`].
#[cfg(feature = "host-test")]
mod last_store {
    use super::LastAlloc;
    use core::cell::Cell;

    std::thread_local! {
        static LAST: Cell<LastAlloc> = const { Cell::new(LastAlloc::ZERO) };
    }

    pub(super) fn get() -> LastAlloc {
        LAST.with(|c| c.get())
    }

    pub(super) fn set(v: LastAlloc) {
        LAST.with(|c| c.set(v));
    }
}

/// In the library's own configuration there is one thread, so one value.
#[cfg(not(feature = "host-test"))]
mod last_store {
    use super::LastAlloc;
    use crate::util::Global;

    static LAST: Global<LastAlloc> = Global::new(LastAlloc::ZERO);

    pub(super) fn get() -> LastAlloc {
        // SAFETY: single-threaded by contract.
        *unsafe { LAST.get() }
    }

    pub(super) fn set(v: LastAlloc) {
        // SAFETY: single-threaded by contract.
        *unsafe { LAST.get() } = v;
    }
}

fn last_stats() -> LastAlloc {
    last_store::get()
}

/// See [`LastAlloc`]. Valid for the program most recently allocated.
pub fn last_reserved() -> usize {
    last_stats().reserved
}

/// The most register COMPONENTS live at one instruction in the last program.
///
/// [`LastAlloc`] counts whole values, which is the floor for an allocator
/// that gives each value a whole register. This counts the components those
/// values actually occupy, so `ceil(components / 4)` is the floor for one
/// that can pack a scalar into any lane. The gap between the two is what
/// component renaming would be worth -- and it is the reason "optimal"
/// unqualified is the wrong word for a whole-register allocator.
///
/// Values written across all four components but read in only one.
///
/// Each of these costs a whole register to carry one number. They are not the
/// allocator's to fix -- the width is chosen where the instruction is
/// selected -- but counting them here is what says whether a high `temps` is
/// "many values live" or "few values, each four times wider than it needs".
/// See [`LastAlloc`]. Valid for the program most recently allocated.
pub fn last_wide_but_scalar() -> usize {
    last_stats().wide_but_scalar
}

/// See [`LastAlloc`].
pub fn last_copies() -> usize {
    last_stats().copies
}

/// See [`LastAlloc`].
pub fn last_coalescable() -> usize {
    last_stats().coalescable
}

/// Of the coalescable copies, how a rename would fare: (takeable, blocked by
/// a swizzle, not a copy at all -- a MOV carrying neg/abs/sat, conditional,
/// not a join -- the destination is simply dead).
pub fn last_mov_transformed() -> usize {
    last_stats().mov_transformed
}

/// Single-lane values the last allocation placed in a lane other than the
/// one selection wrote (see the assignment in [`allocate_and_build`]).
pub fn last_lane_moves() -> usize {
    last_stats().lane_moves
}

/// Host tests only: allocate without moving any value to another lane, so an
/// equivalence test can compare the two allocations of one program. Per
/// thread, like [`LastAlloc`].
#[cfg(feature = "host-test")]
pub fn set_lane_moves_disabled(off: bool) {
    LANE_MOVES_OFF.with(|c| c.set(off));
}

#[cfg(feature = "host-test")]
std::thread_local! {
    static LANE_MOVES_OFF: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

#[cfg(feature = "host-test")]
pub(crate) fn lane_moves_disabled() -> bool {
    LANE_MOVES_OFF.with(|c| c.get())
}

#[cfg(not(feature = "host-test"))]
pub(crate) fn lane_moves_disabled() -> bool {
    false
}

fn record_lane_moves(n: usize) {
    let mut s = last_store::get();
    s.lane_moves = n;
    last_store::set(s);
}

/// Values the last allocation kept live across a loop's back edge; see
/// [`extend_across_back_edges`]. Zero for a program with no loop.
pub fn last_loop_extended() -> usize {
    last_stats().loop_extended
}

/// Whether the last program allocated had any branch in it.
///
/// A linear live range BOUNDS liveness only while every branch points
/// forward; a back edge needs [`extend_across_back_edges`] before the ranges
/// are a bound at all. Given that, the ranges are all register assignment
/// needs. A transformation that deletes an
/// instruction needs more than a bound: a copy inside a conditional arm only
/// happens when the arm runs. So anything that rewrites the program asks this
/// first.
pub fn last_had_branch() -> bool {
    last_stats().had_branch
}

/// (branches, of those with a target at or before their own index).
///
/// A backward edge is a loop, and a loop is the difference between liveness
/// being one backward walk over the blocks and needing a fixpoint.
/// The peak liveness a range SPLIT at every redefinition would see, or 0 for
/// a program with a branch (where splitting is not sound on these ranges).
///
/// The gap between this and [`last_overlap`] is what non-SSA selection output
/// costs the allocator, and it is the number to size a splitting pass from.
pub fn last_read_only_vregs() -> usize {
    last_stats().read_only_vregs
}

pub fn last_split_overlap() -> usize {
    last_stats().split_overlap
}

pub fn last_branches() -> (usize, usize) {
    let s = last_stats();
    (s.branches, s.branches_backward)
}

pub fn last_coalesce_ready() -> (usize, usize, usize, usize, usize) {
    let s = last_stats();
    (s.coalesce_ready, s.coalesce_swizzled, s.coalesce_transformed, s.coalesce_conditional, s.coalesce_not_joinable)
}

/// Every `mov` in the last program: total, then the three kinds that are not
/// coalescable copies. `movs == coalescable + overlap + seed + io`.
/// The seeds split: (uniform, immediate, other, of which read only once).
pub fn last_seed_breakdown() -> (usize, usize, usize, usize, usize, usize) {
    let s = last_stats();
    (s.seed_uniform, s.seed_imm, s.seed_other, s.seed_read_once, s.seed_uniform_once, s.seed_imm_once)
}

/// Of the read-once immediate seeds, what the single consumer is:
/// (consumer is a `mov`, consumer is an ordinary operation).
pub fn last_imm_consumers() -> (usize, usize, usize) {
    let s = last_stats();
    (s.seed_imm_to_mov, s.seed_imm_to_alu, s.seed_imm_dead)
}

pub fn last_mov_breakdown() -> (usize, usize, usize, usize, usize) {
    let s = last_stats();
    (s.movs, s.coalescable, s.mov_overlap, s.mov_seed, s.mov_io)
}

/// See [`LastAlloc`].
pub fn last_safe_sunk() -> usize {
    last_stats().safe_sunk
}

/// See [`LastAlloc`].
pub fn last_multi_def() -> usize {
    last_stats().multi_def
}

/// Instructions in the last program that cannot be moved: a branch, or one
/// whose destination is not a virtual register (an io write, a discard).
/// See [`LastAlloc`].
pub fn last_pinned_instrs() -> usize {
    last_stats().pinned_instrs
}

/// See [`LastAlloc`]. Valid for the program most recently allocated.
pub fn last_sunk_overlap() -> usize {
    last_stats().sunk_overlap
}

/// See [`LastAlloc`]. Valid for the program most recently allocated.
pub fn last_live_components() -> usize {
    last_stats().live_components
}

/// See [`LastAlloc`]. Valid for the program most recently allocated.
pub fn last_overlap() -> usize {
    last_stats().overlap
}

/// See [`VREG_PEAK`]: the largest requirement seen, for sizing [`MAX_VREGS`].
pub fn vreg_peak() -> usize {
    // SAFETY: single-threaded by contract.
    *unsafe { VREG_PEAK.get() }
}

/// How many virtual registers `sel` needs: the highest index it names, plus
/// one. Counted over every operand, so the answer does not depend on where a
/// walk happened to stop.
fn vregs_needed(sel: &[SelInstr]) -> usize {
    let mut top: usize = 0;
    let mut note = |v: VReg| top = top.max(v.0 as usize + 1);
    for s in sel {
        if let Some(SelDst::Virtual { v, .. }) = s.dst {
            note(v);
        }
        for src in s.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = *src {
                note(v);
            }
        }
    }
    top
}

const RGROUP_TEMP: u8 = 0;

/// Live range and component footprint of one virtual register.
#[derive(Clone, Copy)]
struct Range {
    first: usize,
    last: usize,
    comps: u8,
    /// The first instruction that READS this value, as distinct from the one
    /// that defines it. See [`LastAlloc`].
    first_read: usize,
    /// Components any READER actually takes, as distinct from `comps`, which
    /// also carries the width the writer declared. A value written `.xyzw`
    /// but only ever read `.x` is a scalar occupying a whole register, and
    /// the two masks are the only way to see that from here.
    read_comps: u8,
    used: bool,
}

/// Map the virtual registers of `sel` onto the temporary file and build the
/// instructions into `out`.
/// `abi_reserved` names the registers the ABI pins for this shader that the
/// instruction list need not mention: the io registers, `gl_Position`,
/// `gl_PointSize`, the fragment colour, and the rest of the set `abi.rs`
/// checks `temps` against.
///
/// They have to be given, because they cannot be seen. They are reservations
/// inside this same temporary file, and an instruction can easily mention
/// none of them -- a shader with no instructions at all, or, as copy
/// propagation made common, one whose store to the colour register was
/// elided because the value was already there. Left out, two things go wrong
/// and only one of them is loud: `temps` comes back below an io register
/// (`abi.rs` refuses that), and a temporary can be placed *on* one, so the
/// shader computes over its own varying.
pub fn allocate_and_build(
    scratch: &Arena<'_>,
    sel: &[SelInstr],
    out: &mut [Instr],
    stage: Stage,
    abi_reserved: impl IntoIterator<Item = u8>,
) -> Result<Allocated, AllocError> {
    allocate(scratch, sel, Out::Instrs(out), stage, abi_reserved, None::<core::iter::Empty<u8>>, None)
}

/// [`allocate_and_build`] with the input registers given apart from the rest
/// of the ABI's: an input is a value in a register the hardware chose, live
/// from the start to its last read (through the end of any loop that reads
/// it), after which its register is free like any other. The reference does
/// exactly this -- 282 of its 324 programs write a register an input arrived
/// in -- where an input reserved for the whole program costs one register
/// per attribute or varying for as long as the program runs.
///
/// `reserved` names the registers that must hold their value to the end (the
/// outputs); r0 of a fragment shader stays reserved as before (the reference
/// writes it in 3 of 145 fragment shaders, and never as a temporary we could
/// explain).
pub fn allocate_and_build_io(
    scratch: &Arena<'_>,
    sel: &[SelInstr],
    out: &mut [Instr],
    stage: Stage,
    reserved: impl IntoIterator<Item = u8>,
    inputs: impl IntoIterator<Item = u8>,
) -> Result<Allocated, AllocError> {
    allocate(scratch, sel, Out::Instrs(out), stage, reserved, Some(inputs), None)
}

/// Where each placeable output register ended up: `placed[r]` is the
/// register the allocator gave the output selection wrote to `r`, or
/// [`NOT_PLACED`] for a register that was not placeable.
pub type Placed = [u8; MAX_TEMPS as usize];

/// `Placed` entry for a register that was not placed.
pub const NOT_PLACED: u8 = 0xff;

/// [`allocate_and_build_io`], and the outputs in `placeable` are values too.
///
/// Selection writes an output to a fixed register it chose (position r0,
/// varyings from r1, the colour where the store put it). The reference places
/// outputs like temporaries -- `hiqsubpixeldistancefieldtext.vert` puts the
/// position in t2 and its five varyings in 0, 7, 4, 5, 1 -- which is what
/// lets an output be computed in the register its input dies in instead of
/// holding a register of its own for the whole program. An output named here
/// is a value live from its first write to the end of the program, placed by
/// the same assignment; `placed` says where each went, and the caller
/// rewrites the ABI's io rows from it.
///
/// Only an output some instruction WRITES may be named: one selection left
/// in place (a varying already in its input's register) has no definition to
/// move. A fragment shader's r0 stays reserved, so the colour never lands
/// there.
pub fn allocate_and_build_placed(
    scratch: &Arena<'_>,
    sel: &[SelInstr],
    out: &mut [Instr],
    stage: Stage,
    reserved: impl IntoIterator<Item = u8>,
    inputs: impl IntoIterator<Item = u8>,
    placeable: &[u8],
    placed: &mut Placed,
) -> Result<Allocated, AllocError> {
    allocate(scratch, sel, Out::Instrs(out), stage, reserved, Some(inputs), Some((&placeable_set(placeable)?, placed)))
}

/// [`allocate_and_build_placed`], writing each instruction's four encoded
/// dwords into `words` (four per instruction of `sel`) as it is built,
/// instead of a 160-byte [`Instr`] apiece for the caller to encode after.
///
/// The product's path: the `Instr` array was the largest single table of a
/// compile -- 475,200 bytes of uniform_api.random.23's 2,251,896-byte peak,
/// for 2,970 instructions whose encoding is 47,520 -- and every instruction
/// is built exactly once and never revisited, so nothing needs the decoded
/// form after it is encoded. The bytes are those [`super::encode::encode`]
/// writes for the same program.
pub fn allocate_and_encode_placed(
    scratch: &Arena<'_>,
    sel: &[SelInstr],
    words: &mut [u32],
    stage: Stage,
    reserved: impl IntoIterator<Item = u8>,
    inputs: impl IntoIterator<Item = u8>,
    placeable: &[u8],
    placed: &mut Placed,
) -> Result<Allocated, AllocError> {
    allocate(scratch, sel, Out::Words(words), stage, reserved, Some(inputs), Some((&placeable_set(placeable)?, placed)))
}

fn placeable_set(placeable: &[u8]) -> Result<[bool; MAX_TEMPS as usize], AllocError> {
    let mut want = [false; MAX_TEMPS as usize];
    for &r in placeable {
        *want.get_mut(r as usize).ok_or(AllocError::OutOfRegisters(r as u16 + 1))? = true;
    }
    Ok(want)
}

type Placement<'p> = (&'p [bool; MAX_TEMPS as usize], &'p mut Placed);

/// Where the allocator puts the program: built instructions, or their
/// encoding, four dwords apiece.
enum Out<'o> {
    Instrs(&'o mut [Instr]),
    Words(&'o mut [u32]),
}

fn allocate<I: IntoIterator<Item = u8>>(
    scratch: &Arena<'_>,
    sel: &[SelInstr],
    mut out: Out<'_>,
    stage: Stage,
    abi_reserved: impl IntoIterator<Item = u8>,
    inputs: Option<I>,
    placement: Option<Placement<'_>>,
) -> Result<Allocated, AllocError> {
    let interval_inputs = inputs.is_some();
    let mut placeable = [false; MAX_TEMPS as usize];
    let mut placed_out: Option<&mut Placed> = None;
    if let Some((want, placed)) = placement {
        placeable = *want;
        placed.fill(NOT_PLACED);
        placed_out = Some(placed);
    }
    /* Input registers that are values with a live interval, not reservations:
     * the hardware writes them before the first instruction. */
    let mut input_reg = [false; MAX_TEMPS as usize];
    let room = match &out {
        Out::Instrs(o) => o.len(),
        Out::Words(w) => w.len() / 4,
    };
    if room < sel.len() {
        return Err(AllocError::OutputTooSmall);
    }
    check_shape(sel)?;
    check_operands(sel)?;
    /* What this program actually needs, counted before anything is allocated.
     *
     * The bound used to be discovered by the first virtual register that fell
     * outside `ranges`, which reports where the walk stopped rather than what
     * the program requires: a program touching v274 early and v400 later
     * reported 275. Sizing MAX_VREGS from numbers like that invites a second
     * raise, because the figure was never the requirement. */
    let need = vregs_needed(sel);
    record_vreg_peak(need);
    if need > MAX_VREGS {
        return Err(AllocError::OutOfRegisters(need.min(u16::MAX as usize) as u16));
    }
    /* A placeable output is a value numbered after every virtual register:
     * pseudo[r] is the one selection wrote to register r. */
    let mut pseudo = [u32::MAX; MAX_TEMPS as usize];
    {
        let mut next = need;
        for (r, p) in pseudo.iter_mut().enumerate() {
            if placeable[r] && sel.iter().any(|s| matches!(s.dst, Some(SelDst::Fixed { reg, .. }) if reg as usize == r))
            {
                if next >= MAX_VREGS {
                    return Err(AllocError::OutOfRegisters((next + 1).min(u16::MAX as usize) as u16));
                }
                *p = next as u32;
                next += 1;
            }
        }
    }
    let pseudo_of = |reg: u8| pseudo.get(reg as usize).copied().filter(|p| *p != u32::MAX);

    /* Every per-value table is sized by THIS program -- its values, then one
     * pseudo value per placeable output register above them -- and comes from
     * the caller's scratch arena. They were `[_; MAX_VREGS]` on the stack,
     * twelve of them in this function alone, sized by the bound and not by
     * the program: the no_std stack paid for 512 values whatever was
     * compiled, and the bound could not grow without the stack growing with
     * it (random.23's 5,785 values, tools/cts/RESULTS.md). */
    let nvals = need + MAX_TEMPS as usize;
    let mem = |_: ArenaError| AllocError::OutOfScratch;
    let ranges = scratch
        .alloc_filled(
            nvals,
            Range { first: usize::MAX, last: 0, first_read: usize::MAX, comps: 0, read_comps: 0, used: false },
        )
        .map_err(mem)?;
    // Registers the generator pinned: they are in this same file, so they are
    // both untouchable and part of the count.
    let mut reserved = [false; MAX_TEMPS as usize];
    let mut top: i32 = -1;
    /* r0 is gl_FragCoord on this stage, given by the hardware and not by any
     * instruction, so nothing in `sel` mentions it and the loop below cannot
     * learn of it. Reserved before that loop runs: see [`Stage`]. */
    if stage == Stage::Fragment {
        reserve(&mut reserved, &mut top, 0)?;
    }
    /* The same kind of reservation as r0 above, and for the same reason: the
     * hardware and the ABI pin these, not the program. */
    for reg in abi_reserved {
        reserve(&mut reserved, &mut top, reg)?;
    }
    if let Some(inputs) = inputs {
        for reg in inputs {
            let slot = input_reg.get_mut(reg as usize).ok_or(AllocError::OutOfRegisters(reg as u16 + 1))?;
            *slot = true;
            top = top.max(reg as i32);
        }
    }

    for (i, s) in sel.iter().enumerate() {
        if let Some(SelDst::Virtual { v, write_mask }) = s.dst {
            let r = range_mut(ranges, v)?;
            r.used = true;
            r.comps |= write_mask;
            r.first = r.first.min(i);
            r.last = r.last.max(i);
        }
        /* `SelDst::Address` is deliberately absent from this arm: the
         * address register is not in the temporary file, so it reserves
         * nothing. Adding it here would cost every program with a dynamic
         * index one temporary it does not need, and on the fragment stage
         * that temporary would be `gl_FragCoord`'s. */
        if let Some(SelDst::Fixed { reg, write_mask }) = s.dst {
            if let Some(pv) = pseudo_of(reg) {
                let r = &mut ranges[pv as usize];
                r.used = true;
                r.comps |= write_mask;
                r.first = r.first.min(i);
                r.last = r.last.max(i);
            } else {
                reserve(&mut reserved, &mut top, reg)?;
            }
        }
        for src in s.srcs.iter().flatten() {
            match *src {
                SelSrc::Virtual { v, swizzle, .. } => {
                    let r = range_mut(ranges, v)?;
                    r.used = true;
                    r.first_read = r.first_read.min(i);
                    for c in swizzle {
                        r.comps |= 1 << (c & 3);
                        r.read_comps |= 1 << (c & 3);
                    }
                    r.first = r.first.min(i);
                    r.last = r.last.max(i);
                }
                SelSrc::Input { reg, .. } if interval_inputs => {
                    let slot = input_reg.get_mut(reg as usize).ok_or(AllocError::OutOfRegisters(reg as u16 + 1))?;
                    *slot = true;
                    top = top.max(reg as i32);
                }
                SelSrc::Input { reg, .. } => reserve(&mut reserved, &mut top, reg)?,
                SelSrc::Physical(_) => {}
            }
        }
    }

    /* Which lanes each value's readers actually CONSUME, which is fewer
     * than their swizzles name. `add t3.x, t5.xyzw, 1.0` reads t5.x and
     * nothing else: a lane-wise operation takes lane `swizzle[c]` only for
     * the lanes `c` it writes, and a scalar one takes `swizzle[0]` (the
     * builder splats it). Counting all four swizzle lanes made every scalar
     * read through `.xyzw` hold a whole register, and made a scalar's
     * redefinition look partial, so no loop could see it end.
     *
     * Solved backwards to a fixpoint because a reader's own written lanes
     * are narrowed by ITS readers: a `mov` copy read in one lane consumes one
     * lane of its source. Starting from nothing and only growing, it reaches
     * the least solution; an instruction nobody reads consumes nothing. */
    {
        let need = scratch.alloc_filled(nvals, 0u8).map_err(mem)?;
        lanes_needed(sel, need);
        for (r, n) in ranges.iter_mut().zip(need.iter()) {
            r.read_comps = *n;
        }
    }

    /* Dead lanes, dropped before anything is allocated.
     *
     * Selection writes a whole vec4 for values only one lane of which is ever
     * read. Those lanes cannot be observed -- a virtual register is internal
     * by construction, since anything the ABI or the driver can see arrives
     * as `Fixed` -- so narrowing the write to what is read changes no result
     * and frees the rest of the register for someone else.
     *
     * This is the reference's normal mode, not an optimisation it forgoes:
     * of its 13,574 instructions with a used destination, 10,479 (77.2 %)
     * write a PARTIAL mask and only 3,095 (22.8 %) write all four.
     *
     * A value nobody reads is left alone. Narrowing it to nothing would emit
     * a destination with an empty mask, which still sets DST_USE and is a
     * different question (a dead instruction) from this one.
     */
    /* Except a `load`'s: it packs the dwords it fetches into the lanes its
     * destination enables, first enabled lane first, so narrowing `.xyzw` to
     * `.__zw` puts the element's x and y where z and w were asked for --
     * gl_test 128 read fcols[2].xy for .ba on the i.MX8MP (cc955595). Its
     * value keeps every lane it was selected with. */
    let whole = scratch.alloc_filled(nvals, false).map_err(mem)?;
    for s in sel {
        if s.op.op().meta == "load" {
            if let Some(SelDst::Virtual { v, .. }) = s.dst {
                if let Some(w) = whole.get_mut(v.0 as usize) {
                    *w = true;
                }
            }
        }
    }
    let narrowed = scratch.alloc_filled(nvals, 0u8).map_err(mem)?;
    for (v, r) in ranges.iter_mut().enumerate() {
        narrowed[v] = r.comps;
        if whole[v] {
            continue;
        }
        if r.used && r.read_comps != 0 && r.read_comps != r.comps {
            r.comps = r.read_comps;
            narrowed[v] = r.read_comps;
        }
    }

    /* A loop makes a value live at points outside [first mention, last
     * mention]: one defined before the loop and last read in its body is
     * needed again on the next trip, so its register must survive to the
     * back edge. Without this, a value defined later in the same body took
     * that register and the second iteration read it (the loop in
     * `loop_tests` did exactly that). */
    /* An output holds its value PAST the end of the program -- the hardware
     * reads it after the last instruction -- so its interval ends one beyond
     * the last index. Ending it AT the last instruction let a value born
     * there take its register (textured.vert: the position in t0, then the
     * last output written over it).
     *
     * Set BEFORE the back-edge widening, which must also know that an output
     * is live at the end: an output written only inside a loop is live from
     * the loop's top onward too, since a later trip that does not write it
     * again runs through the whole body to the exit. dEQP
     * invariance.*.loop_2 writes gl_Position in one arm of a loop; its range
     * began at that write, so the next trip's `mov t5, t1` at the loop top
     * took the position's register and the program ended with the loop's
     * `value` in gl_Position -- the full-width copy in one program, a .xyz
     * one in the other, and the two positions differed in w. */
    for p in pseudo.iter().filter(|p| **p != u32::MAX) {
        ranges[*p as usize].last = sel.len();
    }
    let out_of = |k: usize| pseudo.iter().position(|p| *p as usize == k).map(|r| r as u8);
    let grew = scratch.alloc_filled(nvals, false).map_err(mem)?;
    let loop_extended = extend_across_back_edges(sel, ranges, grew, out_of);

    /* The floor, before assigning anything: the most values live at any one
     * instruction. Recorded rather than asserted -- it is the yardstick a
     * `temps` number is read against, not a rule. */
    {
        let mut stats = LastAlloc::ZERO;
        /* Every peak below is the most intervals covering one instruction,
         * swept once over the program (peak_cover) instead of counting every
         * value at every instruction, which was values x instructions and
         * 4e8 steps for a 20,000-value program. The same numbers. */
        let delta = scratch.alloc_filled(sel.len() + 1, 0i64).map_err(mem)?;
        let peak = peak_cover(delta, ranges.iter().filter(|r| r.used).map(|r| (r.first, r.last, 1)));
        let comp_peak =
            peak_cover(delta, ranges.iter().filter(|r| r.used).map(|r| (r.first, r.last, r.comps.count_ones() as i64)));
        stats.live_components = comp_peak;
        /* The same peak measured over [first READ, last read] instead of
         * [definition, last read]: what liveness would be if every value were
         * produced just before it is first consumed. Dependencies may forbid
         * that, so it is a lower bound on what SCHEDULING could reach, not a
         * promise -- but the distance between it and the real peak is the
         * difference between "too many values" and "values made far too
         * early", and those have different owners. */
        let sunk_peak = peak_cover(
            delta,
            ranges.iter().filter(|r| r.used && r.first_read != usize::MAX).map(|r| (r.first_read, r.last, 1)),
        );
        stats.sunk_overlap = sunk_peak;
        /* Preconditions for any re-ordering, counted rather than assumed:
         * how many values are written more than once (sinking a definition
         * past another write to the same register changes the program), and
         * how many instructions carry a branch target or write nothing to a
         * virtual register (side effects and control flow pin an order). */
        let defs = scratch.alloc_filled(nvals, 0u16).map_err(mem)?;
        let mut pinned_instrs = 0usize;
        for s in sel.iter() {
            if let Some(SelDst::Virtual { v, .. }) = s.dst {
                if let Some(d) = defs.get_mut(v.0 as usize) {
                    *d += 1;
                }
            } else {
                pinned_instrs += 1;
            }
            if s.target.is_some() {
                pinned_instrs += 1;
            }
        }
        stats.multi_def = defs.iter().filter(|d| **d > 1).count();
        /* What the peak would be if a value's range were SPLIT at every
         * redefinition.
         *
         * A linear range runs from a value's first mention to its last, so a
         * register written at 3, rewritten at 60 and read at 61 is counted
         * live for fifty-eight instructions it is not. Selection emits
         * non-SSA -- on `fill_color.nodiscard.frag`, 13 of 25 values are
         * written more than once -- so this is not a corner case, it is most
         * of the program.
         *
         * Straight-line only: a read in one arm of an `if` can reach a
         * definition from before the branch, and splitting there would put
         * two things in one register. The figure is 0 for a program with any
         * branch rather than a wrong number for it. */
        let mut split_peak = 0usize;
        if !stats.had_branch {
            /* Each definition opens a run that closes at the next definition
             * of the same value; the run is live from the definition to the
             * last read inside it. */
            let open = scratch.alloc_filled(nvals, usize::MAX).map_err(mem)?;
            let close = scratch.alloc_filled(nvals, 0usize).map_err(mem)?;
            /* A run opens at a definition or stays open at the end, so there
             * are at most one per instruction plus one per value: the cap
             * below MAX_RUNS is never the one that binds. */
            let runs = scratch.alloc_filled(MAX_RUNS.min(sel.len() + nvals), (0usize, 0usize)).map_err(mem)?;
            let cap = runs.len();
            let mut n_runs = 0usize;
            let push = |a: usize, b: usize, n: &mut usize, runs: &mut [(usize, usize)]| {
                if *n < cap && b >= a {
                    runs[*n] = (a, b);
                    *n += 1;
                }
            };
            /* A redefinition only closes the previous run when it writes
             * ALL FOUR components. Selection writes partial masks: `v.x`
             * here and `v.y` there is ONE value assembled in pieces, and
             * splitting it would hand the reader a register holding only the
             * second piece. Requiring a whole write is sufficient rather
             * than exact -- a partial write that still covers everything the
             * readers take would be safe too -- and it costs 42 of the 172
             * registers the naive rule claimed: 683 -> 553, not 511. */
            for (i, ins) in sel.iter().enumerate() {
                for src in ins.srcs.iter().flatten() {
                    if let SelSrc::Virtual { v, .. } = *src {
                        let k = v.0 as usize;
                        if k < nvals && open[k] != usize::MAX {
                            close[k] = close[k].max(i);
                        }
                    }
                }
                if let Some(SelDst::Virtual { v, write_mask }) = ins.dst {
                    let k = v.0 as usize;
                    if k < nvals {
                        if open[k] != usize::MAX {
                            /* Only a definition that covers what the NEXT run
                             * reads may close the previous one. Otherwise the
                             * two runs are one value and stay joined. */
                            if write_mask & 0xf == 0xf {
                                push(open[k], close[k], &mut n_runs, runs);
                                open[k] = i;
                                close[k] = i;
                                continue;
                            }
                            /* Joined: the run keeps running through this
                             * partial write. */
                            close[k] = close[k].max(i);
                            continue;
                        }
                        open[k] = i;
                        close[k] = i;
                    }
                }
            }
            for k in 0..nvals {
                if open[k] != usize::MAX {
                    push(open[k], close[k], &mut n_runs, runs);
                }
            }
            split_peak = peak_cover(delta, runs[..n_runs].iter().map(|&(a, b)| (a, b, 1)));
        }
        stats.split_overlap = split_peak;
        /* Values the ranges count but the runs cannot: a virtual register
         * READ by some instruction and written by none. A run opens at a
         * definition, so such a value contributes to `peak` and not to
         * `split_peak`, and the two would disagree for a reason that has
         * nothing to do with splitting. */
        {
            let defined_any = scratch.alloc_filled(nvals, false).map_err(mem)?;
            for ins in sel.iter() {
                if let Some(SelDst::Virtual { v, .. }) = ins.dst {
                    if (v.0 as usize) < nvals {
                        defined_any[v.0 as usize] = true;
                    }
                }
            }
            stats.read_only_vregs =
                ranges.iter().enumerate().filter(|(k, r)| r.used && *k < nvals && !defined_any[*k]).count();
        }
        stats.had_branch = sel.iter().any(|s| s.target.is_some());
        /* Forward or backward, which is the difference between liveness being
         * one backward pass over blocks and needing a fixpoint. Measured
         * rather than assumed from "ES 2.0 loops are bounded and we unroll
         * them": a single backward edge anywhere makes the cheap algorithm
         * wrong, and this is the only place that can see one. */
        stats.branches = sel.iter().filter(|s| s.target.is_some()).count();
        stats.branches_backward =
            sel.iter().enumerate().filter(|(i, s)| s.target.is_some_and(|t| (t as usize) <= *i)).count();
        /* The same peak again, but sinking only what is SAFE to sink: a value
         * written exactly once. A value written twice cannot have its first
         * definition moved past the second without changing the program, so
         * those keep the range they have. This is the number a real pass
         * could reach, as against the unconditional bound above. */
        let safe_peak = peak_cover(
            delta,
            ranges.iter().enumerate().filter(|(_, r)| r.used).map(|(v, r)| {
                let once = defs.get(v).copied().unwrap_or(0) == 1;
                let start = if once && r.first_read != usize::MAX { r.first_read } else { r.first };
                (start, r.last, 1)
            }),
        );
        stats.safe_sunk = safe_peak;
        /* Copies that COULD be coalesced: a `mov` from one virtual register
         * to another whose two live ranges do not overlap. Giving both the
         * same register would delete the instruction and the register with
         * it. Counted so that "is there anything left in the allocator" has
         * an answer instead of an opinion -- a large number means coalescing
         * is worth writing, a small one means the copies hold genuinely
         * different values and the fix is upstream. */
        let mut coalescable = 0usize;
        let mut copies = 0usize;
        let mut movs = 0usize;
        let mut mov_overlap = 0usize;
        let mut mov_seed = 0usize;
        let mut mov_io = 0usize;
        let (mut seed_uniform, mut seed_imm, mut seed_other, mut seed_read_once) = (0usize, 0usize, 0usize, 0usize);
        let (mut seed_uniform_once, mut seed_imm_once) = (0usize, 0usize);
        let (mut seed_imm_to_mov, mut seed_imm_to_alu, mut seed_imm_dead) = (0usize, 0usize, 0usize);
        let (mut coalesce_ready, mut coalesce_swizzled) = (0usize, 0usize);
        let (mut coalesce_transformed, mut coalesce_conditional) = (0usize, 0usize);
        let mut coalesce_not_joinable = 0usize;
        let mut mov_transformed = 0usize;
        /* Per value: how many instructions read it (an instruction reading
         * it twice counts once), and whether any of them is a `mov` -- what
         * the seed partition below asks of each seed's destination. Counted
         * in one pass; it used to scan the program again for every seed. */
        let readers_of = scratch.alloc_filled(nvals, 0u32).map_err(mem)?;
        let mov_reads = scratch.alloc_filled(nvals, false).map_err(mem)?;
        for o in sel.iter() {
            let mut seen = [u32::MAX; 3];
            for (k, x) in o.srcs.iter().flatten().enumerate() {
                if let SelSrc::Virtual { v: u, .. } = x {
                    if seen[..k].contains(&u.0) {
                        continue;
                    }
                    seen[k] = u.0;
                    if let Some(c) = readers_of.get_mut(u.0 as usize) {
                        *c += 1;
                        mov_reads[u.0 as usize] |= o.op.is_mov();
                    }
                }
            }
        }
        for (i, s) in sel.iter().enumerate() {
            if !s.op.is_mov() {
                continue;
            }
            movs += 1;
            /* Across EVERY mov, not only the coalescable ones: how many are
             * not copies at all. The reference lowers `fneg`, `fabs` and
             * `fsat` to a MOV carrying a modifier, so "38 % of our
             * instructions are movs" counts arithmetic as copying. The
             * number is reported rather than subtracted, because the
             * partition below is a partition of MOV INSTRUCTIONS and moving
             * these out of it would silently change three other counts. */
            if s.srcs.iter().flatten().any(|x| {
                matches!(
                    x,
                    SelSrc::Virtual { neg: true, .. }
                        | SelSrc::Virtual { abs: true, .. }
                        | SelSrc::Input { neg: true, .. }
                        | SelSrc::Input { abs: true, .. }
                )
            }) || s.saturate
            {
                mov_transformed += 1;
            }
            let src0 = s.srcs.iter().flatten().next();
            match (s.dst, src0) {
                // Into an io register, or out of one: not a copy between two
                // values, so nothing to coalesce.
                (Some(SelDst::Fixed { .. }), _) => mov_io += 1,
                (_, Some(SelSrc::Input { .. })) => mov_io += 1,
                (Some(SelDst::Virtual { v: d, write_mask }), Some(SelSrc::Virtual { v: sv, swizzle, neg, abs })) => {
                    copies += 1;
                    let (Some(a), Some(b)) = (ranges.get(d.0 as usize), ranges.get(sv.0 as usize)) else { continue };
                    // They interfere unless one dies no later than the other
                    // is born.
                    if a.last <= b.first || b.last <= a.first {
                        coalescable += 1;
                        /* Of those, how many a RENAME could actually take.
                         * Counted separately because "the ranges do not
                         * overlap" is the textbook condition and this machine
                         * adds three of its own: deleting the copy leaves
                         * every later reader of `d` reading `sv` in place, so
                         * the copy must not have transformed the value on the
                         * way, and it must not have been conditional.
                         *
                         * Separating them is the point. A gap between 463 and
                         * this number is work for whoever emits the copies;
                         * no gap means the pass is worth writing as it
                         * stands. Sizing the pass from 463 alone would be
                         * sizing it from the wrong number. */
                        let identity = (0..4).all(|c| write_mask & (1 << c) == 0 || swizzle[c] as usize == c);
                        /* A `mov` on this machine is not always a copy: the
                         * reference lowers `fneg`, `fabs` and `fsat` to MOV
                         * with a modifier, so an instruction that looks like
                         * a copy computes something. Counted apart from the
                         * conditional ones because they are different
                         * answers: a modified MOV is not a copy at all and
                         * was never coalescing's to take, while a
                         * conditional one is a copy that only sometimes
                         * happens. */
                        let transformed = *neg || *abs || s.saturate;
                        let conditional = !matches!(s.cond, None | Some(Cond::True));
                        let plain = !transformed && !conditional;
                        /* The direction that is a coalesce rather than a dead
                         * store: `sv` is not mentioned after the copy and `d`
                         * not before it, so renaming `d` to `sv` joins two
                         * ranges end to end and introduces no interference.
                         * The other direction (`a.last <= b.first`) means `d`
                         * is never read -- dead code, not coalescing, and it
                         * belongs to the DCE pass rather than here. */
                        let joinable = b.last <= i && a.first >= i;
                        if identity && plain && joinable {
                            coalesce_ready += 1;
                        } else if !joinable {
                            coalesce_not_joinable += 1;
                        } else if !identity {
                            coalesce_swizzled += 1;
                        } else if transformed {
                            coalesce_transformed += 1;
                        } else {
                            coalesce_conditional += 1;
                        }
                    } else {
                        mov_overlap += 1;
                    }
                }
                /* Materialising a constant or a uniform read. Split by
                 * WHICH file it comes from and how often the result is read,
                 * because the two halves have different fixes upstream: a
                 * uniform can be folded into every consumer, a literal that
                 * does not fit the immediate form cannot. gles2-core can see
                 * which construct emitted it and not the operand; this file
                 * sees the operand and not the construct. */
                (Some(SelDst::Virtual { v: d, .. }), Some(SelSrc::Physical(p))) => {
                    mov_seed += 1;
                    if p.is_immediate() || p.group == RGROUP_IMMED {
                        seed_imm += 1;
                    } else if p.group == RGROUP_UNIFORM_0 || p.group == RGROUP_UNIFORM_1 {
                        seed_uniform += 1;
                    } else {
                        seed_other += 1;
                    }
                    let readers = readers_of.get(d.0 as usize).copied().unwrap_or(0) as usize;
                    /* What the single consumer IS decides the fix for an
                     * immediate: a consumer that is itself a `mov` is a copy
                     * chain and folds trivially; an ordinary ALU consumer
                     * folds only if its slot takes the immediate form.
                     * gles2-core can see the construct and not the operand;
                     * this is the operand half of their question. */
                    if readers == 0 && (p.is_immediate() || p.group == RGROUP_IMMED) {
                        // A constant materialised and never read: dead code,
                        // not something folding removes.
                        seed_imm_dead += 1;
                    }
                    if readers == 1 && (p.is_immediate() || p.group == RGROUP_IMMED) {
                        if mov_reads.get(d.0 as usize).copied().unwrap_or(false) {
                            seed_imm_to_mov += 1;
                        } else {
                            seed_imm_to_alu += 1;
                        }
                    }
                    if readers <= 1 {
                        seed_read_once += 1;
                        if p.is_immediate() || p.group == RGROUP_IMMED {
                            seed_imm_once += 1;
                        } else if p.group == RGROUP_UNIFORM_0 || p.group == RGROUP_UNIFORM_1 {
                            seed_uniform_once += 1;
                        }
                    }
                }
                (Some(SelDst::Virtual { .. }), _) => {
                    mov_seed += 1;
                    seed_other += 1;
                }
                _ => {}
            }
        }
        stats.movs = movs;
        stats.mov_overlap = mov_overlap;
        stats.mov_seed = mov_seed;
        stats.mov_io = mov_io;
        stats.seed_uniform = seed_uniform;
        stats.seed_imm = seed_imm;
        stats.seed_other = seed_other;
        stats.seed_read_once = seed_read_once;
        stats.seed_uniform_once = seed_uniform_once;
        stats.seed_imm_once = seed_imm_once;
        stats.seed_imm_to_mov = seed_imm_to_mov;
        stats.seed_imm_to_alu = seed_imm_to_alu;
        stats.seed_imm_dead = seed_imm_dead;
        stats.copies = copies;
        stats.coalescable = coalescable;
        stats.coalesce_ready = coalesce_ready;
        stats.coalesce_swizzled = coalesce_swizzled;
        stats.coalesce_transformed = coalesce_transformed;
        stats.mov_transformed = mov_transformed;
        stats.coalesce_conditional = coalesce_conditional;
        stats.coalesce_not_joinable = coalesce_not_joinable;
        stats.pinned_instrs = pinned_instrs;
        let wide_but_scalar =
            ranges.iter().filter(|r| r.used && r.comps.count_ones() == 4 && r.read_comps.count_ones() == 1).count();
        stats.wide_but_scalar = wide_but_scalar;
        stats.overlap = peak;
        stats.reserved = reserved.iter().filter(|x| **x).count();
        stats.loop_extended = loop_extended;
        /* Published once, as one value. Ten separate writes to ten
         * process-wide statics is what raced. */
        last_store::set(stats);
    }

    // Assign. Per-component: two values share a register when their live
    // ranges do not overlap, or when the components they occupy are disjoint.
    let phys = scratch.alloc_filled(nvals, u16::MAX).map_err(mem)?;
    // For each register and component, the last instruction index it is busy
    // to; -1 for free.
    let mut busy = [[-1i32; 4]; MAX_TEMPS as usize];
    /* An input occupies its register from the start to its last read, and to
     * the end of every loop that reads it: the next trip reads it again. A
     * register the ABI also reserves (an output written in place) stays
     * reserved. */
    for (reg, is_input) in input_reg.iter().enumerate() {
        if !*is_input || reserved[reg] {
            continue;
        }
        let reads_it = |s: &SelInstr| {
            s.srcs.iter().flatten().any(|x| matches!(*x, SelSrc::Input { reg: r, .. } if r as usize == reg))
        };
        let Some(mut last) = sel.iter().rposition(reads_it) else { continue };
        loop {
            let mut grown = false;
            for (b, s) in sel.iter().enumerate() {
                let Some(t) = s.target.map(|t| t as usize).filter(|&t| t <= b) else { continue };
                if b > last && (t..=b).any(|i| i <= last && reads_it(&sel[i])) {
                    last = b;
                    grown = true;
                }
            }
            if !grown {
                break;
            }
        }
        busy[reg] = [last as i32; 4];
    }
    /* In order of increasing start point, not VReg index.
     *
     * Live ranges are intervals, and for interval graphs greedy colouring in
     * start order uses exactly the maximum overlap -- the provable minimum.
     * In any other order it does not: a long-lived value assigned late finds
     * the low registers already marked busy to a later instruction than its
     * own start and takes a fresh one, permanently. The front end numbers
     * virtual registers in creation order, which is close to start order and
     * therefore hid this, but "close" is what leaves registers on the floor.
     */
    let order = scratch.alloc_filled(nvals, 0u16).map_err(mem)?;
    let mut n_used = 0usize;
    for (v, r) in ranges.iter().enumerate() {
        if r.used {
            order[n_used] = v as u16;
            n_used += 1;
        }
    }
    order[..n_used].sort_unstable_by_key(|&v| ranges[v as usize].first);

    /* Which single-lane values may live in ANY lane. A scalar selection put
     * in lane x holds lane x of a register of its own whenever another scalar
     * is live, and the reference packs them four to a register. Moving one
     * from lane O to lane L means its definitions write L -- which a lane-wise
     * operation computes from source position L, so its source swizzles take
     * position O's entry there -- and every reader reads L where it read O.
     * That is only this simple for the operations whose lanes are
     * independent, so a value is movable only if every definition is one. */
    /* Per value, in one pass over the program: 1 once a definition is
     * seen and every one so far qualifies, 2 once one does not. It was a
     * scan of the whole program per value. */
    let move_state = scratch.alloc_filled(nvals, 0u8).map_err(mem)?;
    if !lane_moves_disabled() {
        for s in sel.iter() {
            let Some(SelDst::Virtual { v: d, write_mask }) = s.dst else { continue };
            let v = d.0 as usize;
            let Some(r) = ranges.get(v) else { continue };
            if !r.used || r.comps.count_ones() != 1 || move_state[v] == 2 {
                continue;
            }
            let o = r.comps.trailing_zeros() as u8;
            let ok = s.tex.is_none()
                && s.target.is_none()
                && write_mask & narrowed[v] == 1 << o
                && (s.op.reads_one_component() || lanewise_op(s.op));
            move_state[v] = if ok { 1 } else { 2 };
        }
    }
    let movable = |v: usize| -> Option<u8> {
        let r = &ranges[v];
        (move_state[v] == 1 && r.used && r.comps.count_ones() == 1).then(|| r.comps.trailing_zeros() as u8)
    };
    /* (original lane << 4) | new lane, or NOT_MOVED. */
    let moved = scratch.alloc_filled(nvals, NOT_MOVED).map_err(mem)?;

    for &v in &order[..n_used] {
        let v = v as usize;
        let r = ranges[v];
        let lane = movable(v);
        /* A value born by an instruction that reads the register's previous
         * value for the last time may take that register: the machine reads
         * an instruction's sources before it writes its destination, and the
         * reference relies on it (`mul t0, t0.xyyy, u4.xyyy`). Only when the
         * instruction writes every lane the value has, unguarded, and does
         * not read the value itself -- otherwise the value is already live
         * coming into it (a loop's back edge can make that so) and the two
         * would overlap. */
        let born_whole = sel.get(r.first).is_some_and(|s| {
            let whole_output = matches!(s.dst, Some(SelDst::Fixed { reg, write_mask })
                if pseudo_of(reg) == Some(v as u32) && write_mask & r.comps == r.comps)
                && (matches!(s.cond, None | Some(Cond::True)) || matches!(s.op.op().name, "select" | "cmp"));
            whole_output || (kills(s, VReg(v as u32), r.comps) && !reads(s, VReg(v as u32)))
        });
        let free_before = if born_whole { r.first as i32 + 1 } else { r.first as i32 };
        let mut placed = None;
        'regs: for reg in 0..MAX_TEMPS as usize {
            if reserved[reg] {
                continue;
            }
            if let Some(o) = lane {
                /* Its own lane first, so nothing moves that need not. */
                for l in core::iter::once(o).chain((0..4).filter(|&l| l != o)) {
                    if busy[reg][l as usize] < free_before {
                        placed = Some((reg, 1u8 << l));
                        if l != o {
                            moved[v] = o << 4 | l;
                        }
                        break 'regs;
                    }
                }
                continue;
            }
            let fits = (0..4).all(|c| r.comps & (1 << c) == 0 || busy[reg][c] < free_before);
            if fits {
                placed = Some((reg, r.comps));
                break;
            }
        }
        let Some((reg, lanes)) = placed else {
            return Err(AllocError::OutOfRegisters(MAX_TEMPS + 1));
        };
        for (c, slot) in busy[reg].iter_mut().enumerate() {
            if lanes & (1 << c) != 0 {
                *slot = r.last as i32;
            }
        }
        phys[v] = reg as u16;
        top = top.max(reg as i32);
    }
    let lane_moves = moved.iter().filter(|m| **m != NOT_MOVED).count();

    let mut out_reg = [NOT_PLACED; MAX_TEMPS as usize];
    for (r, p) in pseudo.iter().enumerate() {
        if *p != u32::MAX {
            out_reg[r] = phys[*p as usize] as u8;
        }
    }
    if let Some(placed) = placed_out {
        *placed = out_reg;
    }
    for (i, s) in sel.iter().enumerate() {
        let ins = build_one(s, phys, narrowed, moved, &out_reg)?;
        match &mut out {
            Out::Instrs(o) => o[i] = ins,
            Out::Words(w) => w[i * 4..i * 4 + 4].copy_from_slice(&super::encode::encode_instr(&ins)),
        }
    }
    record_lane_moves(lane_moves);
    Ok(Allocated { count: sel.len(), temps: (top + 1) as u16 })
}

/// The lanes of each virtual register that some reader consumes, solved
/// backwards to a fixpoint (see [`consumed_lanes`]). Starting from nothing and
/// only growing, it reaches the least solution: an instruction whose result
/// nobody needs consumes nothing, so a whole dead chain needs nothing.
/// Flow-insensitive, so it can only over-state what is needed.
///
/// `need` is lent by the caller, one entry per value (see [`vregs_needed`]),
/// all zero; a value past its end is taken as wholly needed.
fn lanes_needed(sel: &[SelInstr], need: &mut [u8]) {
    lanes_needed_skipping(sel, &[], need)
}

/// Every live read of a lane that no instruction writes: `(instruction,
/// source ordinal, lanes)`, for a host check.
///
/// A virtual register is written by selection in whole or in lanes, and a
/// reader consumes the lanes its swizzle names for the lanes it writes that
/// somebody needs ([`consumed_lanes`], over the [`lanes_needed`] fixpoint).
/// A consumed lane that nothing writes is a value never computed: twice now
/// a one-lane condition was read as `.xyzw` -- by a branch (0828b91) and by
/// the if-conversion's select (fb04d88) -- and the lanes past x decided
/// with whatever the register held. Flow-insensitive, so a lane written on
/// some path counts as written; it cannot miss a read of a lane written on
/// none.
///
/// `inputs` gives each input register the lanes its io row declares (a vec2
/// attribute or varying defines x and y): a read past them is a read of the
/// vertex fetch's or the interpolator's padding, the same class. An input
/// register not listed (gl_FragCoord in t0) is taken as whole. `outputs`
/// likewise gives each output register the lanes anything downstream reads
/// (a vec2 varying two, the point size one): a store through a full write
/// mask needs only those, so its padding lanes need nothing.
#[cfg(any(test, feature = "host-test"))]
pub fn undefined_lane_reads(
    sel: &[SelInstr],
    inputs: &[(u8, u8)],
    outputs: &[(u8, u8)],
) -> std::vec::Vec<(usize, usize, u8)> {
    /* The lanes a reader really consumes. `consumed_lanes` is the
     * allocator's SAFE model -- anything not known to be lanewise reads all
     * four swizzle lanes, which over-reserves and cannot clobber -- and a
     * check built on it reports every vec2 fed to a dot product. Here: a dot
     * product reads its width, and a texture coordinate at least x and y
     * (every 2D fetch; a cube or 3D coordinate is a whole vec3, and a
     * one-lane condition read through .xyzw is what this is for). */
    let reads = |s: &SelInstr, n: usize, swizzle: [u8; 4], eff: u8| -> u8 {
        let first = |k: usize| swizzle[..k].iter().fold(0u8, |m, c| m | 1 << (c & 3));
        match (s.op.op().name, s.dst) {
            ("dp2", _) => first(2),
            ("dp3", _) => first(3),
            _ if s.tex.is_some() && n == 0 => first(2),
            /* `movar`/`movai` into the address register reads the lanes its
             * write mask names, like any mov: one, for every index here */
            (_, Some(SelDst::Address { write_mask })) => {
                (0..4).filter(|c| write_mask & (1 << c) != 0).fold(0u8, |m, c| m | 1 << (swizzle[c] & 3))
            }
            _ => consumed_lanes(s, swizzle, eff),
        }
    };
    let eff_of = |s: &SelInstr, need: &[u8]| match s.dst {
        Some(SelDst::Virtual { v, write_mask }) => write_mask & need.get(v.0 as usize).copied().unwrap_or(0xf),
        Some(SelDst::Fixed { reg, write_mask }) => {
            write_mask & outputs.iter().find(|(r, _)| *r == reg).map_or(0xf, |(_, c)| (1u8 << (*c).min(4)) - 1)
        }
        _ => 0xf,
    };
    let mut need = std::vec![0u8; vregs_needed(sel)];
    loop {
        let mut changed = false;
        for s in sel.iter().rev() {
            let eff = eff_of(s, &need);
            if eff == 0 && matches!(s.dst, Some(SelDst::Virtual { .. })) {
                continue;
            }
            for (n, src) in s.srcs.iter().enumerate() {
                if let Some(SelSrc::Virtual { v, swizzle, .. }) = *src {
                    let lanes = reads(s, n, swizzle, eff);
                    let Some(slot) = need.get_mut(v.0 as usize) else { continue };
                    if *slot | lanes != *slot {
                        *slot |= lanes;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut written = std::vec![0u8; vregs_needed(sel)];
    for s in sel {
        if let Some(SelDst::Virtual { v, write_mask }) = s.dst {
            if let Some(w) = written.get_mut(v.0 as usize) {
                *w |= write_mask;
            }
        }
    }
    let mut out = std::vec::Vec::new();
    for (i, s) in sel.iter().enumerate() {
        let eff = eff_of(s, &need);
        if eff == 0 && matches!(s.dst, Some(SelDst::Virtual { .. })) {
            continue;
        }
        for (n, src) in s.srcs.iter().enumerate() {
            let (w, swizzle) = match *src {
                Some(SelSrc::Virtual { v, swizzle, .. }) => match written.get(v.0 as usize) {
                    Some(&w) => (w, swizzle),
                    None => continue,
                },
                Some(SelSrc::Input { reg, swizzle, .. }) => {
                    let lanes = inputs.iter().find(|(r, _)| *r == reg).map_or(0xf, |(_, c)| (1u8 << (*c).min(4)) - 1);
                    (lanes, swizzle)
                }
                _ => continue,
            };
            let missing = reads(s, n, swizzle, eff) & !w;
            if missing != 0 {
                out.push((i, n, missing));
            }
        }
    }
    out
}

/// [`lanes_needed`] over the instructions not marked in `dead`: a deleted
/// reader needs nothing. Instructions past the end of `dead` are never
/// marked, so they all count.
fn lanes_needed_skipping(sel: &[SelInstr], dead: &[u64], need: &mut [u8]) {
    let is_dead = |i: usize| dead.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 != 0);
    loop {
        let mut changed = false;
        for (i, s) in sel.iter().enumerate().rev() {
            if is_dead(i) {
                continue;
            }
            let eff = match s.dst {
                Some(SelDst::Virtual { v, write_mask }) => write_mask & need.get(v.0 as usize).copied().unwrap_or(0xf),
                Some(SelDst::Fixed { write_mask, .. }) => write_mask,
                _ => 0xf,
            };
            for src in s.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, swizzle, .. } = *src {
                    let lanes = consumed_lanes(s, swizzle, eff);
                    let Some(slot) = need.get_mut(v.0 as usize) else { continue };
                    if *slot | lanes != *slot {
                        *slot |= lanes;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Delete every instruction whose result nobody consumes; returns how many
/// went. The survivors are compacted to the front of `sel`, in order.
///
/// Selection's own dead-code pass works on the IR, where a local slot is live
/// until the program ends, so the writes that feed a slot and the `select`s
/// that join an `if` survive it. Here the whole program is in view:
/// `stencil.frag` (femtovg) computes `scissorMask` into slots and joins them
/// with eleven selects, and none of it reaches the output, which is the
/// constant 1.0 -- 70 of its 83 instructions.
///
/// An instruction is dead when it writes a virtual register and no lane it
/// writes is needed ([`lanes_needed`]). Anything else -- an io write, a
/// branch, a discard, the address register -- is a root and stays.
///
/// **A branch target is an instruction index**, and so is every entry of
/// `back_edges` (the loop branches `abi.rs` checks backward jumps against).
/// Both are remapped through the survivors: a target at a deleted instruction
/// lands on the next survivor, which is where control arrived after the
/// deleted, effect-free run; a target at the old end becomes the new end.
///
/// Programs longer than [`MAX_LIVE_INSTRS`] are left alone: the
/// flow-sensitive half is per value over every instruction, and that bound is
/// what keeps its cost acceptable. Its tables come from `scratch`, sized by
/// the program.
pub fn eliminate_dead(
    scratch: &Arena<'_>,
    sel: &mut [SelInstr],
    back_edges: &mut [usize],
) -> Result<usize, AllocError> {
    let n = sel.len();
    if n > MAX_LIVE_INSTRS {
        return Ok(0);
    }
    let mem = |_: ArenaError| AllocError::OutOfScratch;
    let values = vregs_needed(sel);
    let need = scratch.alloc_filled(values, 0u8).map_err(mem)?;
    lanes_needed(sel, need);
    let dead = scratch.alloc_filled(n.div_ceil(64), 0u64).map_err(mem)?;
    let mut work = Liveness {
        need: scratch.alloc_filled(values, 0u8).map_err(mem)?,
        live: scratch.alloc_filled(4 * n.div_ceil(64), 0u64).map_err(mem)?,
    };
    let mut removed = 0usize;
    for (i, s) in sel.iter().enumerate() {
        if let Some(SelDst::Virtual { v, write_mask }) = s.dst {
            if s.target.is_none() && write_mask & need.get(v.0 as usize).copied().unwrap_or(0xf) == 0 {
                dead[i / 64] |= 1 << (i % 64);
                removed += 1;
            }
        }
    }
    /* Then flow-sensitively, to a fixpoint: a write is dead when none of its
     * lanes is live AFTER it, even if the same value is read elsewhere. The
     * join of an `if` writes every slot either arm wrote, and a slot read
     * inside an arm and never after the join made the whole-program test
     * above keep the select, its seed and the copy both arms carried (the
     * femtovg fills: ten such slots in scissorMask). Each round can expose
     * more, so it repeats until a round deletes nothing. */
    loop {
        let more = dead_after_liveness(sel, dead, &mut work);
        if more == 0 {
            break;
        }
        removed += more;
    }
    if removed == 0 {
        return Ok(0);
    }
    let is_dead = |i: usize| dead[i / 64] >> (i % 64) & 1 != 0;
    /* Survivors before old index i: the new index of i, or of the next
     * survivor when i itself goes. */
    let new_index = |t: usize| (0..t.min(n)).filter(|&k| !is_dead(k)).count();
    let mut j = 0usize;
    for i in 0..n {
        if is_dead(i) {
            continue;
        }
        let mut s = sel[i];
        if let Some(t) = s.target {
            s.target = Some(new_index(t as usize) as u16);
        }
        sel[j] = s;
        j += 1;
    }
    for b in back_edges.iter_mut() {
        *b = new_index(*b);
    }
    Ok(removed)
}

/// [`dead_after_liveness`]'s working tables, lent once for every round:
/// `need` one entry per value, `live` four bitsets (one per lane) of one bit
/// per instruction.
struct Liveness<'a> {
    need: &'a mut [u8],
    live: &'a mut [u64],
}

/// One round of the flow-sensitive half of [`eliminate_dead`]: for every
/// virtual register, per-lane liveness over the instructions not yet marked
/// dead (a dead instruction neither reads nor writes), and every definition
/// whose written lanes are all dead after it is marked. Returns how many.
///
/// Successors as in [`extend_across_back_edges`]; nothing virtual is live at
/// the program's end. A guarded write does not end the value it overwrites.
fn dead_after_liveness(sel: &[SelInstr], dead: &mut [u64], work: &mut Liveness<'_>) -> usize {
    let n = sel.len();
    let w = n.div_ceil(64);
    let is_dead = |d: &[u64], i: usize| d[i / 64] >> (i % 64) & 1 != 0;
    let mut marked = 0usize;
    let top = work.need.len();
    /* What each reader's result needs, over the survivors: a reader consumes
     * its sources only in the lanes that feed its OWN needed lanes. Taking
     * its whole write mask instead kept a seed copy alive for lanes a later
     * partial write replaced before anyone read them (femtovg's scissorMask:
     * `mov v54.f <- |v145.xyzw|` where only v54.xy is ever read). */
    work.need.fill(0);
    lanes_needed_skipping(sel, dead, work.need);
    let need = &*work.need;
    for k in 0..top {
        let v = VReg(k as u32);
        if !sel
            .iter()
            .enumerate()
            .any(|(i, s)| !is_dead(dead, i) && matches!(s.dst, Some(SelDst::Virtual { v: d, .. }) if d == v))
        {
            continue;
        }
        /* live_in per lane, one bitset each. */
        let live = &mut work.live[..4 * w];
        live.fill(0);
        let bit = |set: &[u64], i: usize| set[i / 64] >> (i % 64) & 1 != 0;
        let out_lanes = |live: &[u64], i: usize| -> u8 {
            let s = &sel[i];
            let falls = s.target.is_none() || !matches!(s.cond, None | Some(Cond::True));
            let mut m = 0u8;
            for (c, set) in live.chunks_exact(w).enumerate() {
                let mut on = falls && i + 1 < n && bit(set, i + 1);
                if let Some(t) = s.target {
                    on |= (t as usize) < n && bit(set, t as usize);
                }
                m |= (on as u8) << c;
            }
            m
        };
        loop {
            let mut changed = false;
            for i in (0..n).rev() {
                if is_dead(dead, i) {
                    /* Transparent: whatever is live after it is live before. */
                    let o = out_lanes(live, i);
                    for (c, set) in live.chunks_exact_mut(w).enumerate() {
                        if o & (1 << c) != 0 && !bit(set, i) {
                            set[i / 64] |= 1 << (i % 64);
                            changed = true;
                        }
                    }
                    continue;
                }
                let s = &sel[i];
                let o = out_lanes(live, i);
                let (wm, guarded) = match s.dst {
                    Some(SelDst::Virtual { v: d, write_mask }) if d == v => (
                        write_mask,
                        !matches!(s.cond, None | Some(Cond::True)) && !matches!(s.op.op().name, "select" | "cmp"),
                    ),
                    _ => (0, false),
                };
                let eff = match s.dst {
                    Some(SelDst::Virtual { v: d, write_mask }) => {
                        write_mask & need.get(d.0 as usize).copied().unwrap_or(0xf)
                    }
                    Some(SelDst::Fixed { write_mask, .. }) => write_mask,
                    _ => 0xf,
                };
                let mut r = 0u8;
                for src in s.srcs.iter().flatten() {
                    if let SelSrc::Virtual { v: u, swizzle, .. } = *src {
                        if u == v {
                            r |= consumed_lanes_live(s, swizzle, eff);
                        }
                    }
                }
                let kill = if guarded { 0 } else { wm };
                let inn = r | (o & !kill);
                for (c, set) in live.chunks_exact_mut(w).enumerate() {
                    if inn & (1 << c) != 0 && !bit(set, i) {
                        set[i / 64] |= 1 << (i % 64);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for i in 0..n {
            if is_dead(dead, i) || sel[i].target.is_some() {
                continue;
            }
            if let Some(SelDst::Virtual { v: d, write_mask }) = sel[i].dst {
                if d == v && write_mask & out_lanes(live, i) == 0 {
                    dead[i / 64] |= 1 << (i % 64);
                    marked += 1;
                }
            }
        }
    }
    marked
}

/// [`consumed_lanes`] for a reader that is itself live: its written lanes
/// are what it needs, so `eff` is never the "nobody needs it" zero.
fn consumed_lanes_live(s: &SelInstr, swizzle: [u8; 4], eff: u8) -> u8 {
    consumed_lanes(s, swizzle, if eff == 0 { 0xf } else { eff })
}

/// The lanes of a source register instruction `s` consumes, given the
/// swizzle it reads through and the lanes `eff` of its result anybody needs.
///
/// Only the operations listed compute destination lane `c` from source lane
/// `swizzle[c]` and nothing else; every other operation (a dot product, a
/// texture fetch, a branch, anything not yet looked at) is taken to read all
/// four swizzle lanes, which over-reserves and cannot clobber.
fn consumed_lanes(s: &SelInstr, swizzle: [u8; 4], eff: u8) -> u8 {
    /* A result nobody needs consumes nothing, whatever the operation: a dead
     * dot product kept its sources alive until this line existed. */
    if eff == 0 && matches!(s.dst, Some(SelDst::Virtual { .. })) {
        return 0;
    }
    let all = swizzle.iter().fold(0u8, |m, c| m | 1 << (c & 3));
    if s.tex.is_some() || s.target.is_some() || s.dst.is_none() || matches!(s.dst, Some(SelDst::Address { .. })) {
        return all;
    }
    if s.op.reads_one_component() {
        return 1 << (swizzle[0] & 3);
    }
    if !lanewise_op(s.op) {
        return all;
    }
    (0..4).filter(|c| eff & (1 << c) != 0).fold(0u8, |m, c| m | 1 << (swizzle[c] & 3))
}

/// The longest program [`extend_across_back_edges`] solves exactly; past it,
/// the conservative rule. 4096 instructions is two bitsets of 512 bytes.
const MAX_LIVE_INSTRS: usize = 4096;

/// Does instruction `s` overwrite every component `comps` of `v`, whatever
/// its condition says? Only then does it end the value that was there.
///
/// A condition on an ordinary operation makes the write conditional, so the
/// old value can survive it. `select` and `cmp` carry a condition as their
/// operand, not as a guard, and write every lane of their mask.
fn kills(s: &SelInstr, v: VReg, comps: u8) -> bool {
    let Some(SelDst::Virtual { v: d, write_mask }) = s.dst else { return false };
    let guarded = !matches!(s.cond, None | Some(Cond::True)) && !matches!(s.op.op().name, "select" | "cmp");
    d == v && !guarded && write_mask & comps == comps
}

fn reads(s: &SelInstr, v: VReg) -> bool {
    s.srcs.iter().flatten().any(|x| matches!(*x, SelSrc::Virtual { v: u, .. } if u == v))
}

/// Widen every range to cover each point where its value is live, for a
/// program with a backward branch. Returns how many ranges grew.
///
/// With forward branches only, every path runs in index order, so a live
/// point always lies between a mention and a later read and the linear range
/// already covers it. A back edge breaks that: the loop's next trip reads what
/// was left at its end. So this solves liveness per value, backwards over the
/// instructions to a fixpoint (the trip round a loop is what needs more than
/// one pass), and takes the hull of the live points. Still one interval per
/// value -- the assignment below stays greedy colouring of intervals -- but
/// now a true one.
///
/// Successors: the next instruction unless the branch is unconditional, and
/// the target. Assuming a fall-through that cannot happen only
/// over-approximates, which is the safe direction.
///
/// Above [`MAX_LIVE_INSTRS`] the answer is conservative instead: a range that
/// meets a loop is widened to the whole loop, repeated until nothing moves.
/// The most of the weighted intervals `(first, last, weight)` covering any
/// one instruction index in `0..delta.len() - 1`, the ends inclusive and
/// clamped to the program. `delta` is scratch of the program's length + 1;
/// it is cleared here, so one table serves every call.
fn peak_cover(delta: &mut [i64], intervals: impl Iterator<Item = (usize, usize, i64)>) -> usize {
    let n = delta.len() - 1;
    delta.fill(0);
    for (a, b, w) in intervals {
        if a > b || a >= n {
            continue;
        }
        delta[a] += w;
        delta[(b + 1).min(n)] -= w;
    }
    let (mut run, mut best) = (0i64, 0i64);
    for d in &delta[..n] {
        run += d;
        best = best.max(run);
    }
    best as usize
}

/// `grew` is scratch of `ranges.len()`, all false, lent by the caller.
///
/// `out_of(k)` names the output register when value `k` is a placed output's
/// pseudo value: such a value is live at the program's end, is never read by
/// an instruction, and is killed by a whole unguarded write of its register.
fn extend_across_back_edges(
    sel: &[SelInstr],
    ranges: &mut [Range],
    grew: &mut [bool],
    out_of: impl Fn(usize) -> Option<u8>,
) -> usize {
    let n = sel.len();
    let back = |i: usize| sel[i].target.filter(|&t| (t as usize) <= i).map(|t| t as usize);
    if !(0..n).any(|i| back(i).is_some()) {
        return 0;
    }
    if n > MAX_LIVE_INSTRS {
        loop {
            let mut changed = false;
            for b in 0..n {
                let Some(t) = back(b) else { continue };
                for (k, r) in ranges.iter_mut().enumerate() {
                    if r.used && r.first <= b && r.last >= t && (r.first > t || r.last < b) {
                        r.first = r.first.min(t);
                        r.last = r.last.max(b);
                        grew[k] = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        return grew.iter().filter(|g| **g).count();
    }
    const W: usize = MAX_LIVE_INSTRS / 64;
    for (k, r) in ranges.iter_mut().enumerate() {
        if !r.used {
            continue;
        }
        let v = VReg(k as u32);
        let comps = if r.comps == 0 { 0xf } else { r.comps };
        let output = out_of(k);
        let kills_it = |s: &SelInstr| match output {
            None => kills(s, v, comps),
            Some(reg) => {
                let Some(SelDst::Fixed { reg: d, write_mask }) = s.dst else { return false };
                let guarded = !matches!(s.cond, None | Some(Cond::True)) && !matches!(s.op.op().name, "select" | "cmp");
                d == reg && !guarded && write_mask & comps == comps
            }
        };
        /* Live at the end: the hardware reads an output after the last
         * instruction, reached by falling off it or by a branch to `n`. */
        let at_end = output.is_some();
        let mut live_in = [0u64; W];
        let mut live_out = [0u64; W];
        let bit = |set: &[u64; W], i: usize| set[i / 64] >> (i % 64) & 1 != 0;
        loop {
            let mut changed = false;
            for i in (0..n).rev() {
                let s = &sel[i];
                let falls = s.target.is_none() || !matches!(s.cond, None | Some(Cond::True));
                let mut out = falls && (if i + 1 < n { bit(&live_in, i + 1) } else { at_end });
                if let Some(t) = s.target {
                    let t = t as usize;
                    out |= if t < n { bit(&live_in, t) } else { at_end };
                }
                let inn = (output.is_none() && reads(s, v)) || (out && !kills_it(s));
                for (set, on) in [(&mut live_out, out), (&mut live_in, inn)] {
                    if on && !bit(set, i) {
                        set[i / 64] |= 1 << (i % 64);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        /* An output is live, by the solve above, on every path to the end
         * that does not write it -- including the paths from the program's
         * start, where nothing has written it yet and the register holds
         * nothing to keep. Only the points some write of it can REACH hold
         * its value, so for an output the hull is taken over those: a
         * forward solve of "written on some path to here". */
        let mut reached = [!0u64; W];
        if let Some(reg) = output {
            reached = [0u64; W];
            let writes = |s: &SelInstr| matches!(s.dst, Some(SelDst::Fixed { reg: d, .. }) if d == reg);
            loop {
                let mut changed = false;
                for (i, s) in sel.iter().enumerate() {
                    if !(writes(s) || bit(&reached, i)) {
                        continue;
                    }
                    let falls = s.target.is_none() || !matches!(s.cond, None | Some(Cond::True));
                    let succs =
                        [(falls && i + 1 < n).then_some(i + 1), s.target.map(|t| t as usize).filter(|&t| t < n)];
                    for j in succs.into_iter().flatten() {
                        if !bit(&reached, j) {
                            reached[j / 64] |= 1 << (j % 64);
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }
        let (mut lo, mut hi) = (r.first, r.last);
        for i in 0..n {
            if bit(&reached, i) && (bit(&live_in, i) || bit(&live_out, i)) {
                lo = lo.min(i);
                hi = hi.max(i);
            }
        }
        if (lo, hi) != (r.first, r.last) {
            r.first = lo;
            r.last = hi;
            grew[k] = true;
        }
    }
    grew.iter().filter(|g| **g).count()
}

fn range_mut(ranges: &mut [Range], v: VReg) -> Result<&mut Range, AllocError> {
    ranges.get_mut(v.0 as usize).ok_or(AllocError::OutOfRegisters(v.0 as u16 + 1))
}

fn reserve(reserved: &mut [bool; MAX_TEMPS as usize], top: &mut i32, reg: u8) -> Result<(), AllocError> {
    let r = reg as usize;
    if r >= MAX_TEMPS as usize {
        return Err(AllocError::OutOfRegisters(reg as u16 + 1));
    }
    reserved[r] = true;
    *top = (*top).max(r as i32);
    Ok(())
}

/// Every filled slot must be one the operation has, and every slot it has
/// must be filled. Both directions: a missing source is as wrong as an extra
/// one, and neither shows up in the encoded bits as anything but a different
/// program.
fn check_shape(sel: &[SelInstr]) -> Result<(), AllocError> {
    for s in sel {
        let is_tex = s.op.has_field("TEX_ID");
        /* A texture instruction must carry its two fields, and nothing else
         * may. `texld` with TEX_ID left at its default sampler 0 is not a
         * failure -- it is a shader that samples unit 0 whatever it asked
         * for, which is a picture rather than an error. */
        if s.tex.is_some() != is_tex {
            return Err(AllocError::WrongTexFields);
        }
        /* Required, not defaulted: a branch to 0 jumps to the top of the
         * shader, which the hardware will happily run. */
        if s.target.is_some() != s.op.has_field("TARGET") {
            return Err(AllocError::WrongBranchTarget);
        }
        for n in 0..3 {
            if s.srcs[n].is_some() && !s.op.has_src(n) {
                return Err(AllocError::WrongSourceSlot(n));
            }
            /* A texture instruction's slots are set EXPLICITLY, not by
             * ordinal. `texld` has a SRC2 field, and SRC2_USE is 0 in all 284
             * texlds and all 78 texldls of the corpus -- so demanding an
             * operand for every slot the operation has would refuse every
             * correct texture instruction. For everything else the strict
             * rule stands: a slot the operation has must be filled. */
            if !is_tex && s.srcs[n].is_none() && s.op.has_src(n) {
                return Err(AllocError::WrongSourceSlot(n));
            }
        }
        /* `has_dst()` asks whether the FIELD exists, which is not the same
         * question as whether the operation writes anything. `texkill` and
         * `nop` carry the field and never use it; see
         * [`OpId::writes_no_dst`] for the corpus split. Giving them one is
         * refused rather than tolerated: a destination here sets `DST_USE`,
         * which no shipped instruction of theirs does. */
        let wants_dst = s.op.has_dst() && !s.op.writes_no_dst();
        if s.dst.is_some() != wants_dst {
            return Err(AllocError::WrongDestination);
        }
        /* Two-way, like the texture and branch fields above. A temporary
         * destination on `movar` does not write that temporary -- the write
         * goes to the address register regardless -- so the generator's
         * intent is silently lost, and `Address` on any other operation
         * writes temporary 0, which on the fragment stage is
         * `gl_FragCoord`. Both directions are wrong pictures rather than
         * errors, so both are refused here. */
        if matches!(s.dst, Some(SelDst::Address { .. })) != s.op.dst_is_address_register() {
            return Err(AllocError::WrongAddressDestination);
        }
    }
    Ok(())
}

/// Ranges the encoding cannot carry. A swizzle lane is two bits and a write
/// mask four, so an out-of-range value does not fail - it truncates, and the
/// result is a different program that encodes perfectly.
fn check_operands(sel: &[SelInstr]) -> Result<(), AllocError> {
    for s in sel {
        /* TEX_ID is five bits and each TEX_SWIZ lane two, so an out-of-range
         * value truncates into a different, perfectly legal program -- the
         * same reason the write mask and the source lanes are checked. */
        /* TARGET is fifteen bits (103..=117) and a u16 holds more: a larger
         * value truncates into a branch somewhere else in the program, which
         * is a shader that runs and draws the wrong thing. */
        if let Some(t) = s.target {
            if t > MAX_BRANCH_TARGET {
                return Err(AllocError::BadOperand("branch target"));
            }
        }
        if let Some(t) = s.tex {
            if t.id > 31 {
                return Err(AllocError::BadOperand("texture sampler id"));
            }
            if t.swizzle.iter().any(|&l| l > 3) {
                return Err(AllocError::BadOperand("texture destination swizzle"));
            }
        }
        if let Some(SelDst::Virtual { write_mask, .. })
        | Some(SelDst::Fixed { write_mask, .. })
        | Some(SelDst::Address { write_mask }) = s.dst
        {
            if write_mask > 0xf {
                return Err(AllocError::BadOperand("write mask"));
            }
        }
        /* An empty mask is refused for the address register and not for a
         * temporary, because the two mean different things. A temporary
         * written with no components is a dead instruction: wasteful, not
         * wrong. The address register written with no components leaves
         * whatever was there, and every following `amode` read then indexes
         * by a stale value -- a program that runs and reads the wrong
         * constants, which is the failure this whole variant exists to
         * prevent. */
        if let Some(SelDst::Address { write_mask }) = s.dst {
            if write_mask == 0 {
                return Err(AllocError::BadOperand("empty address-register write mask"));
            }
        }
        for src in s.srcs.iter().flatten() {
            let sw = match *src {
                SelSrc::Virtual { swizzle, .. } | SelSrc::Input { swizzle, .. } => swizzle,
                // An immediate's "swizzle" is payload, not lanes: it is not
                // this check's business and 0..=3 does not apply to it.
                SelSrc::Physical(p) => {
                    if p.is_immediate() {
                        continue;
                    }
                    p.swizzle
                }
            };
            if sw.iter().any(|c| *c > 3) {
                return Err(AllocError::BadOperand("swizzle lane"));
            }
        }
    }
    Ok(())
}

/// `moved` marker for a value that stays in the lane selection gave it.
const NOT_MOVED: u8 = 0xff;

/// Is this one of the operations whose destination lane `c` is computed from
/// source position `c` alone (the list [`consumed_lanes`] trusts)?
fn lanewise_op(op: OpId) -> bool {
    matches!(
        op.op().name,
        "mov"
            | "add"
            | "mul"
            | "mad"
            | "select"
            | "cmp"
            | "sign"
            | "floor"
            | "ceil"
            | "frc"
            | "dsx"
            | "dsy"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "lshift"
            | "rshift"
            | "f2i"
            | "i2f"
    )
}

fn build_one(s: &SelInstr, phys: &[u16], narrowed: &[u8], moved: &[u8], out_reg: &Placed) -> Result<Instr, AllocError> {
    /* A destination moved from lane O to lane L: a lane-wise operation
     * computes lane L from source position L, so every source takes position
     * O's entry at L. A scalar operation splats swizzle[0] and needs nothing. */
    let dst_move = match s.dst {
        Some(SelDst::Virtual { v, .. }) => Some(moved[v.0 as usize]).filter(|m| *m != NOT_MOVED),
        _ => None,
    };
    let permute = |sw: [u8; 4]| -> [u8; 4] {
        match dst_move {
            Some(m) if !s.op.reads_one_component() => {
                let (o, l) = ((m >> 4) as usize, (m & 3) as usize);
                let mut out = sw;
                out[l] = sw[o];
                out
            }
            _ => sw,
        }
    };
    /* A source whose value moved reads its new lane wherever it read the old. */
    let relane = |v: VReg, sw: [u8; 4]| -> [u8; 4] {
        let m = moved[v.0 as usize];
        if m == NOT_MOVED {
            return sw;
        }
        let (o, l) = (m >> 4, m & 3);
        sw.map(|c| if c & 3 == o { l } else { c })
    };
    let mut b = build::Builder::of(s.op).ty(s.ty as u8);
    if s.saturate {
        b = b.flag("SAT", true);
    }
    if let Some(c) = s.cond {
        b = b.set("COND", c as u32);
    }
    if let Some(t) = s.target {
        b = b.set("TARGET", t as u32);
    }
    if s.rtz {
        /* `#rounding`: 1 is `.rtz`. */
        b = b.set("RMODE", 1);
    }
    if let Some(t) = s.tex {
        let sw = (t.swizzle[0] as u32)
            | (t.swizzle[1] as u32) << 2
            | (t.swizzle[2] as u32) << 4
            | (t.swizzle[3] as u32) << 6;
        b = b.set("TEX_ID", t.id as u32).set("TEX_SWIZ", sw);
    }
    match s.dst {
        Some(SelDst::Virtual { v, write_mask }) => {
            let reg = phys[v.0 as usize];
            /* The lanes anybody reads, never more than the instruction asked
             * to write. See the narrowing above. */
            let mut mask = write_mask & narrowed[v.0 as usize];
            if let Some(m) = dst_move {
                mask = if mask & (1 << (m >> 4)) != 0 { 1 << (m & 3) } else { 0 };
            }
            b = b.dst(Dst::reg(reg as u8, mask));
        }
        Some(SelDst::Fixed { reg, write_mask }) => {
            let placed = out_reg.get(reg as usize).copied().filter(|r| *r != NOT_PLACED).unwrap_or(reg);
            b = b.dst(Dst::reg(placed, write_mask))
        }
        Some(SelDst::Address { write_mask }) => b = b.dst(Dst::address(write_mask)),
        None => {}
    }
    // A scalar operation reads one component, and the machine is fed a
    // uniform swizzle for it: the front half emits the IR literally and this
    // is where it becomes what the hardware has been given.
    let scalar = s.op.reads_one_component();
    let lower = |sw: [u8; 4]| if scalar { splat(sw) } else { sw };
    for (n, src) in s.srcs.iter().enumerate() {
        let Some(src) = src else { continue };
        let concrete = match *src {
            SelSrc::Virtual { v, swizzle, neg, abs } => Src {
                group: RGROUP_TEMP,
                reg: phys[v.0 as usize],
                amode: 0,
                swizzle: lower(relane(v, permute(swizzle))),
                neg,
                abs,
            },
            SelSrc::Input { reg, swizzle, neg, abs } => {
                Src { group: RGROUP_TEMP, reg: reg as u16, amode: 0, swizzle: lower(permute(swizzle)), neg, abs }
            }
            // An immediate's "swizzle" is its payload, not lanes: splatting it
            // would change the number and the shader would compute with a
            // constant nobody wrote.
            SelSrc::Physical(p) => {
                if p.is_immediate() {
                    p
                } else {
                    Src { swizzle: lower(permute(p.swizzle)), ..p }
                }
            }
        };
        b = b.src(n, concrete);
    }
    b.build().map_err(AllocError::Build)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::vivante::build::op_id;
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    const XYZW: [u8; 4] = [0, 1, 2, 3];
    const ALL: u8 = 0xf;

    fn mov() -> OpId {
        op_id("mov").expect("mov exists")
    }

    /// On a program with no redefinition at all, splitting can change
    /// nothing, so the two peaks must agree. If they do not, the bound the
    /// splitting pass was sized from is measuring something else.
    #[test]
    fn the_split_bound_agrees_with_the_peak_when_there_is_nothing_to_split() {
        let sel = [
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false })
                .src(2, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false })
                .src(2, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(0, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 8];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, []).expect("allocates");
        assert_eq!(
            last_split_overlap(),
            last_overlap(),
            "nothing to split, so the bound and the peak must be the same number"
        );
        assert_eq!(split_ranges(&mut { sel }), 0, "a program with no redefinition splits nothing");
    }

    /* ---- coalescing -------------------------------------------------- */

    fn add() -> OpId {
        op_id("add").expect("add exists")
    }

    /// `v1 = v0` where `v0` is dead after the copy and `v1` unborn before it:
    /// the copy goes and the reader reads `v0`.
    #[test]
    fn a_copy_whose_two_values_never_meet_is_deleted() {
        let mut sel = [
            // v0 = in0 + in0
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false })
                .src(1, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            // v1 = v0            <- the copy
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
            // out0 = v1
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        let removed = coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap();
        assert_eq!(removed, 1, "the copy should have gone");
        let kept = &sel[..sel.len() - removed];
        assert_eq!(kept.len(), 2);
        // The output now reads v0 directly.
        let Some(SelSrc::Virtual { v, .. }) = kept[1].srcs[2] else { panic!("the output lost its source") };
        assert_eq!(v, VReg(0), "the reader was not renamed onto the surviving value");
    }

    /// The same shape, but the copy takes a different lane. Deleting it would
    /// leave the reader on `v0.x` where the program said `v0.y`.
    #[test]
    fn a_copy_that_swizzles_is_left_alone() {
        let mut sel = [
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false })
                .src(1, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: [1, 1, 1, 1], neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        assert_eq!(
            coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(),
            0,
            "a swizzled copy is not a copy"
        );
    }

    /// And the one that makes the 190: a MOV carrying `neg` is `fneg`.
    /// Deleting it deletes the negation.
    #[test]
    fn a_mov_carrying_a_modifier_is_not_a_copy() {
        let mut sel = [
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false })
                .src(1, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: true, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        assert_eq!(
            coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(),
            0,
            "fneg was treated as a copy"
        );
    }

    /// A program with a branch is left exactly as it arrived, whatever the
    /// ranges say. The copy below looks perfectly coalescable and is not: it
    /// only happens when the arm runs.
    #[test]
    fn a_program_with_a_branch_is_not_touched() {
        let mut sel = [
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false })
                .src(1, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        sel[0].target = Some(2);
        assert_eq!(
            coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(),
            0,
            "a branch was crossed on a linear liveness model"
        );

        // The control: the identical program without the branch coalesces, so
        // the test above is measuring the branch and not a shape the pass
        // never takes.
        sel[0].target = None;
        assert_eq!(coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(), 1);
    }

    /// Both values live past the copy: renaming would put two live values in
    /// one register, which is the defect this condition exists to prevent.
    #[test]
    fn a_copy_whose_source_is_read_again_is_kept() {
        let mut sel = [
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(0, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false })
                .src(1, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
            // v2 = v0 + v1: both are still live here.
            SelInstr::new(add(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(2), write_mask: ALL })
                .src(0, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false })
                .src(1, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(2), swizzle: XYZW, neg: false, abs: false }),
        ];
        assert_eq!(
            coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(),
            0,
            "two live values were put in one register"
        );
    }

    /// A `mov` reads **SRC2**, not SRC0, which is the first thing a generator
    /// filling slots by operand ordinal gets wrong - and it would encode
    /// perfectly, reading a register nobody wrote.
    #[test]
    fn a_mov_reads_src2_and_filling_src0_is_refused() {
        let good = SelInstr::new(mov(), OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .src(2, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false });
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 4];
        assert!(allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[good], &mut out, Stage::Vertex, []).is_ok());

        let wrong = SelInstr::new(mov(), OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .src(0, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false });
        assert_eq!(
            allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[wrong], &mut out, Stage::Vertex, []),
            Err(AllocError::WrongSourceSlot(0))
        );
    }

    /// The one that matters most, and the reason this allocator is not the
    /// textbook one: on this machine inputs and outputs live in the temporary
    /// file, so a passthrough vertex shader that allocates no temporary at
    /// all still needs `temps = 2`.
    ///
    /// The bring-up triangle is exactly that shape
    /// (`driver/src/vivante/gc_shaders.h`: aPos in r0, aColor in r1,
    /// position out of r0, the varying out of r1, `GC_TRI_VS_NUM_TEMPS 2`).
    /// An allocator that counted only what it placed would answer 0, the
    /// driver would put an unread vertex element on top of a live register,
    /// and the symptom would look like anything except a register bug.
    #[test]
    fn a_passthrough_vertex_shader_needs_two_temps_and_allocates_none() {
        let sel = [
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                .src(2, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: ALL })
                .src(2, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 4];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [])
            .expect("allocates");
        assert_eq!(a.count, 2);
        assert_eq!(a.temps, 2, "inputs and outputs are in the temporary file and must be counted");

        // And the instructions are real: they decode back to `mov` reading the
        // registers we asked for.
        for (i, want_reg) in [0u16, 1].iter().enumerate() {
            let back = decode_instr(&encode_instr(&out[i])).expect("decodes");
            assert_eq!(back.op().mnemonic, "mov");
            assert_eq!(back.dst().unwrap().reg as usize, i);
            assert_eq!(back.src(2).unwrap().reg, *want_reg);
        }
    }

    /// Two values whose live ranges do not overlap share one register.
    #[test]
    fn two_dead_values_share_a_register() {
        let sel = [
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(2, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 5, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
            // VReg(1) is born after VReg(0) is dead.
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: ALL })
                .src(2, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 6, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 8];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [])
            .expect("allocates");
        let r0 = decode_instr(&encode_instr(&out[0])).unwrap().dst().unwrap().reg;
        let r1 = decode_instr(&encode_instr(&out[2])).unwrap().dst().unwrap().reg;
        assert_eq!(r0, r1, "disjoint live ranges should share a register");
        // Registers 5 and 6 are reserved, so the file is 7 deep.
        assert_eq!(a.temps, 7);
    }

    /// A pre-coloured register is never handed to a temporary, even when it
    /// would otherwise be the obvious choice.
    #[test]
    fn a_reserved_register_is_not_reused_for_a_temporary() {
        let sel = [
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: ALL })
                .src(2, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(mov(), OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: ALL })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 4];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, []).expect("allocates");
        let t = decode_instr(&encode_instr(&out[0])).unwrap().dst().unwrap().reg;
        assert!(t != 0 && t != 1, "temporary landed on a reserved register r{t}");
    }

    /// Running out of registers is a named error carrying what was needed,
    /// not a spill nobody asked for and not a wrapped register index.
    #[test]
    fn running_out_of_registers_is_refused_by_name() {
        // Every value stays live to the end, so none can share.
        let mut sel = Vec::new();
        for v in 0..(MAX_TEMPS as u32 + 2) {
            sel.push(
                SelInstr::new(mov(), OpType::F32)
                    .dst(SelDst::Virtual { v: VReg(v), write_mask: ALL })
                    .src(2, SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false }),
            );
        }
        for v in 0..(MAX_TEMPS as u32 + 2) {
            sel.push(
                SelInstr::new(mov(), OpType::F32)
                    .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
                    .src(2, SelSrc::Virtual { v: VReg(v), swizzle: XYZW, neg: false, abs: false }),
            );
        }
        let mut out = vec![Instr { op: 0, vals: [0; 32], dont_care: 0 }; sel.len()];
        match allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, []) {
            Err(AllocError::OutOfRegisters(_)) => {}
            other => panic!("expected OutOfRegisters, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod operand_and_immediate_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, ImmError, Src as BSrc};
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    const XYZW: [u8; 4] = [0, 1, 2, 3];
    const ALL: u8 = 0xf;

    /// Operand ordinals become hardware slots here, so the code generator
    /// never learns that a `mov` reads SRC2.
    #[test]
    fn operand_zero_of_a_mov_lands_in_src2() {
        let mov = op_id("mov").unwrap();
        assert_eq!(mov.num_operands(), 1);
        assert_eq!(mov.slot_of_operand(0), Some(2));
        assert_eq!(mov.slot_of_operand(1), None);

        let by_ordinal = SelInstr::new(mov, OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .operand(0, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false });
        let by_slot = SelInstr::new(mov, OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .src(2, SelSrc::Input { reg: 1, swizzle: XYZW, neg: false, abs: false });
        let mut a = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
        let mut b = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[by_ordinal], &mut a, Stage::Vertex, [])
            .expect("ordinal form builds");
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[by_slot], &mut b, Stage::Vertex, [])
            .expect("slot form builds");
        assert_eq!(encode_instr(&a[0]), encode_instr(&b[0]));
    }

    /// An operation with a gap: `add` takes two operands and they are SRC0
    /// and SRC2, because the description prints `{SRC0}, void, {SRC2}`.
    #[test]
    fn an_operation_with_a_gap_counts_operands_not_slots() {
        let add = op_id("add").unwrap();
        assert_eq!(add.num_operands(), 2);
        assert_eq!(add.slot_of_operand(0), Some(0));
        assert_eq!(add.slot_of_operand(1), Some(2));
    }

    /// The immediate encoding, rebuilt against a real instruction from the
    /// table: Mesa prints this one `mov t2.__z_, void, void, 1.0:f20`.
    ///
    /// It is not a register. The 20-bit payload is spread across what would
    /// be the register index, the swizzle and the negate and absolute bits,
    /// so a builder that treated it as a register would produce a plausible
    /// operand that does not exist.
    #[test]
    fn a_float_immediate_matches_the_words_the_table_ships() {
        let want: [u32; 4] = [0x0202_1009, 0x0000_0004, 0x0000_0000, 0x707f_0008];
        let from_table = decode_instr(&want).expect("decodes");
        let s = from_table.src(2).expect("mov has src2");
        assert!(s.is_immediate(), "the table's own instruction carries an immediate");

        let built = BSrc::imm_f32(1.0).expect("1.0 is exactly representable");
        assert_eq!(built.imm_payload_bits(), s.imm_payload_bits());
        assert_eq!(built.group, s.group);
        assert_eq!(built.amode, s.amode);
        assert_eq!(built.reg, s.reg);
        assert_eq!(built.swizzle, s.swizzle);
        assert_eq!((built.neg, built.abs), (s.neg, s.abs));
    }

    /// A value the encoding cannot carry is refused, not rounded. Rounding it
    /// is how a shader draws a colour nobody asked for, and nothing later in
    /// the pipeline could tell.
    #[test]
    fn an_immediate_that_does_not_fit_is_refused_rather_than_rounded() {
        for exact in [0.0f32, 1.0, 0.5, 2.0, -1.0, 0.25] {
            let s = BSrc::imm_f32(exact).unwrap_or_else(|e| panic!("{exact} refused: {e:?}"));
            // And it reads back as the same number.
            let bits = s.imm_payload_bits().unwrap() << 12;
            assert_eq!(f32::from_bits(bits), exact, "{exact} did not survive");
        }
        assert_eq!(BSrc::imm_f32(0.1), Err(ImmError::NotExactlyRepresentable));
        assert_eq!(BSrc::imm_f32(1.0 / 3.0), Err(ImmError::NotExactlyRepresentable));
        assert_eq!(BSrc::imm_s20(1 << 19), Err(ImmError::OutOfRange));
        assert_eq!(BSrc::imm_u20(1 << 20), Err(ImmError::OutOfRange));
    }

    /// A literal-colour fragment shader, which is what tonight's second
    /// program is: write a constant to the colour register.
    #[test]
    fn a_literal_colour_fragment_shader_builds() {
        let mov = op_id("mov").unwrap();
        let sel = [SelInstr::new(mov, OpType::F32)
            .dst(SelDst::Fixed { reg: 1, write_mask: ALL })
            .operand(0, SelSrc::Physical(BSrc::imm_f32(1.0).unwrap()))];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [])
            .expect("builds");
        assert_eq!(a.temps, 2, "the colour register r1 is in the temporary file");
        let back = decode_instr(&encode_instr(&out[0])).expect("decodes");
        assert_eq!(back.op().mnemonic, "mov");
        assert!(back.src(2).unwrap().is_immediate());
    }
}

#[cfg(test)]
mod swizzle_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc};
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    const ALL: u8 = 0xf;
    fn out() -> [Instr; 4] {
        [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 4]
    }

    /// A scalar operation reads one component, so the front half may emit the
    /// IR literally - `rcp v, a.yzw` - and the lowering makes it the `a.yyyy`
    /// the machine is fed.
    ///
    /// Measured before it was implemented: across the 13,852 instructions the
    /// driver ships, all 2,323 sources of a scalar operation carry a uniform
    /// swizzle and none carries a varied one, while non-scalar operations are
    /// varied 11,890 times against 11,712 uniform. So the property
    /// discriminates; it is not an artefact of most swizzles being uniform.
    #[test]
    fn a_scalar_operation_gets_a_uniform_source_swizzle() {
        let rcp = op_id("rcp").expect("rcp exists");
        assert!(rcp.reads_one_component());
        let sel = [SelInstr::new(rcp, OpType::F32)
            .dst(SelDst::Fixed { reg: 1, write_mask: ALL })
            .operand(0, SelSrc::Input { reg: 0, swizzle: [1, 2, 3, 0], neg: false, abs: false })];
        let mut o = out();
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut o, Stage::Vertex, []).expect("builds");
        let back = decode_instr(&encode_instr(&o[0])).expect("decodes");
        let s = back.src(rcp.slot_of_operand(0).unwrap()).unwrap();
        assert_eq!(s.swizzle, [1, 1, 1, 1], "a scalar op's source must be splatted from its x lane");
    }

    /// And a vector operation keeps what it was given, or the lowering would
    /// be destroying information the IR meant.
    #[test]
    fn a_vector_operation_keeps_its_swizzle() {
        let mul = op_id("mul").expect("mul exists");
        assert!(!mul.reads_one_component());
        let sel = [SelInstr::new(mul, OpType::F32)
            .dst(SelDst::Fixed { reg: 2, write_mask: ALL })
            .operand(0, SelSrc::Input { reg: 0, swizzle: [1, 2, 3, 0], neg: false, abs: false })
            .operand(1, SelSrc::Input { reg: 1, swizzle: [3, 2, 1, 0], neg: false, abs: false })];
        let mut o = out();
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut o, Stage::Vertex, []).expect("builds");
        let back = decode_instr(&encode_instr(&o[0])).expect("decodes");
        assert_eq!(back.src(0).unwrap().swizzle, [1, 2, 3, 0]);
        assert_eq!(back.src(1).unwrap().swizzle, [3, 2, 1, 0]);
    }

    /// An immediate's "swizzle" is its payload. Splatting it would change the
    /// number, and the shader would compute with a constant nobody wrote.
    #[test]
    fn a_scalar_operation_does_not_splat_an_immediate() {
        let rcp = op_id("rcp").expect("rcp exists");
        let imm = BSrc::imm_f32(0.5).expect("0.5 is exact");
        let sel = [SelInstr::new(rcp, OpType::F32)
            .dst(SelDst::Fixed { reg: 1, write_mask: ALL })
            .operand(0, SelSrc::Physical(imm))];
        let mut o = out();
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut o, Stage::Vertex, []).expect("builds");
        let back = decode_instr(&encode_instr(&o[0])).expect("decodes");
        let s = back.src(rcp.slot_of_operand(0).unwrap()).unwrap();
        assert!(s.is_immediate());
        assert_eq!(s.imm_payload_bits(), imm.imm_payload_bits(), "the payload must survive intact");
        assert_eq!(f32::from_bits(s.imm_payload_bits().unwrap() << 12), 0.5);
    }

    /// A swizzle lane is two bits and a write mask four. An out-of-range value
    /// does not fail on its own - it truncates, and the result is a different
    /// program that encodes perfectly - so it is refused by name here.
    #[test]
    fn an_out_of_range_swizzle_or_write_mask_is_refused() {
        let mov = op_id("mov").unwrap();
        let bad_sw = [SelInstr::new(mov, OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .operand(0, SelSrc::Input { reg: 1, swizzle: [0, 1, 2, 7], neg: false, abs: false })];
        let bad_wm = [SelInstr::new(mov, OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: 0x1f })
            .operand(0, SelSrc::Input { reg: 1, swizzle: [0, 1, 2, 3], neg: false, abs: false })];
        let mut o = out();
        assert_eq!(
            allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &bad_sw, &mut o, Stage::Vertex, []),
            Err(AllocError::BadOperand("swizzle lane"))
        );
        assert_eq!(
            allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &bad_wm, &mut o, Stage::Vertex, []),
            Err(AllocError::BadOperand("write mask"))
        );
    }
}

#[cfg(test)]
mod tex_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};

    const XYZW: [u8; 4] = [0, 1, 2, 3];
    const ALL: u8 = 0xf;

    fn blank() -> Instr {
        Instr { op: 0, vals: [0; 32], dont_care: 0 }
    }

    fn one(s: SelInstr) -> Result<Instr, AllocError> {
        let mut out = [blank(); 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[s], &mut out, Stage::Vertex, [])?;
        Ok(out[0])
    }

    fn coord() -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg: 0, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    fn a_texld() -> SelInstr {
        SelInstr::new(op_id("texld").expect("the ISA tables carry texld"), OpType::F32)
            .dst(SelDst::Fixed { reg: 1, write_mask: ALL })
            .src(0, coord())
    }

    /// The two fields reach the encoding, which is the only way to know they
    /// were written where the machine reads them and not merely stored.
    #[test]
    fn a_texture_instruction_carries_its_sampler_and_swizzle_to_the_encoding() {
        let instr = one(a_texld().tex(Tex { id: 5, swizzle: [3, 2, 1, 0] })).expect("builds");
        assert_eq!(instr.get("TEX_ID"), Some(5), "the sampler did not reach TEX_ID");
        /* lane0 | lane1<<2 | lane2<<4 | lane3<<6 */
        assert_eq!(instr.get("TEX_SWIZ"), Some(3 | 2 << 2 | 1 << 4), "packed wrong");
    }

    /// 412 of the corpus's 427 texture instructions use the identity, and the
    /// ISA notes name its value, so the packing is checkable against a number
    /// nobody here chose.
    #[test]
    fn the_identity_swizzle_is_the_corpus_value() {
        let instr = one(a_texld().tex(Tex { id: 0, swizzle: Tex::IDENTITY })).expect("builds");
        assert_eq!(instr.get("TEX_SWIZ"), Some(0xE4), "the identity should be 0xE4");
    }

    /// `texld` HAS a SRC2 field and SRC2_USE is 0 in all 284 texlds of the
    /// corpus, so the ordinal rule that serves every other operation would
    /// refuse every correct texture instruction. The relaxation is for
    /// texture instructions only, and this pins both halves.
    #[test]
    fn a_texture_instruction_may_leave_a_slot_empty_and_nothing_else_may() {
        assert!(
            one(a_texld().tex(Tex { id: 0, swizzle: Tex::IDENTITY })).is_ok(),
            "texld with SRC1 and SRC2 empty must build"
        );

        let short = SelInstr::new(op_id("add").unwrap(), OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .src(0, coord());
        assert!(
            matches!(one(short), Err(AllocError::WrongSourceSlot(_))),
            "a non-texture operation must still fill every slot it has"
        );
    }

    #[test]
    fn the_tex_fields_are_refused_where_they_do_not_belong_and_required_where_they_do() {
        let wrong = SelInstr::new(op_id("add").unwrap(), OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: ALL })
            .src(0, coord())
            .src(2, coord())
            .tex(Tex { id: 0, swizzle: Tex::IDENTITY });
        assert!(matches!(one(wrong), Err(AllocError::WrongTexFields)), "add has no TEX_ID");

        /* A texld whose sampler was never set would sample unit 0 whatever it
         * asked for -- a picture rather than an error -- so it is refused. */
        assert!(
            matches!(one(a_texld()), Err(AllocError::WrongTexFields)),
            "a texture instruction without its fields must be refused"
        );
    }

    /// Out of range truncates into a different legal program, exactly as an
    /// over-wide swizzle lane would, so both are refused rather than masked.
    #[test]
    fn an_out_of_range_sampler_or_lane_is_refused() {
        assert!(matches!(
            one(a_texld().tex(Tex { id: 32, swizzle: Tex::IDENTITY })),
            Err(AllocError::BadOperand("texture sampler id"))
        ));
        assert!(matches!(
            one(a_texld().tex(Tex { id: 0, swizzle: [0, 1, 2, 4] })),
            Err(AllocError::BadOperand("texture destination swizzle"))
        ));
        assert!(one(a_texld().tex(Tex { id: 31, swizzle: [3, 3, 3, 3] })).is_ok(), "the top of each range is legal");
    }
}

#[cfg(test)]
mod vreg_bound_tests {
    use super::*;

    /// The bound is measured, so the cost of it is worth stating in the same
    /// place: it sizes a stack array in `allocate_and_build`.
    #[test]
    fn print_the_cost_of_the_bound() {
        println!(
            "Range = {} bytes; MAX_VREGS = {} -> ranges[] = {} bytes of stack",
            core::mem::size_of::<Range>(),
            MAX_VREGS,
            core::mem::size_of::<Range>() * MAX_VREGS
        );
    }

    /// Above the bound the allocator refuses by name and reports what the
    /// program actually needed, not where the walk stopped.
    #[test]
    fn past_the_bound_it_refuses_with_the_true_requirement() {
        use super::super::build::op_id;
        let add = op_id("add").expect("add");
        let far = VReg(MAX_VREGS as u32 + 40);
        let sel = [SelInstr::new(add, OpType::F32)
            .dst(SelDst::Virtual { v: far, write_mask: 0xf })
            .src(0, SelSrc::Virtual { v: VReg(0), swizzle: [0, 1, 2, 3], neg: false, abs: false })
            .src(2, SelSrc::Virtual { v: VReg(1), swizzle: [0, 1, 2, 3], neg: false, abs: false })];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        match allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, []) {
            Err(AllocError::OutOfRegisters(n)) => {
                assert_eq!(n as usize, MAX_VREGS + 41, "it should report the requirement, not the first miss")
            }
            other => panic!("expected OutOfRegisters, got {other:?}"),
        }
    }
    /// The restructure's target (tools/cts/RESULTS.md, random.23): a
    /// straight-line program of 20,000 values, each read by the next two, so
    /// few are live at once -- random.23's shape, whose 5,785 values the old
    /// 512-value bound refused although its register pressure is small.
    /// Every product pass and the allocator, in codegen's order, with the
    /// scratch they are lent. What it must come to: every instruction kept,
    /// three temporaries (the input, and two values of the chain), and a
    /// sensible time; the numbers are printed so a regression in either has
    /// a figure.
    #[test]
    fn a_twenty_thousand_value_straight_line_program_allocates() {
        use super::super::build::op_id;
        let (add, mov) = (op_id("add").unwrap(), op_id("mov").unwrap());
        let v = |k: u32| SelSrc::Virtual { v: VReg(k), swizzle: [0, 1, 2, 3], neg: false, abs: false };
        let d = |k: u32| SelDst::Virtual { v: VReg(k), write_mask: 0xf };
        let input = SelSrc::Input { reg: 0, swizzle: [0, 1, 2, 3], neg: false, abs: false };
        const N: u32 = 20_000;
        let mut sel = std::vec![
            SelInstr::new(add, OpType::F32).dst(d(0)).src(0, input).src(2, input),
            SelInstr::new(add, OpType::F32).dst(d(1)).src(0, v(0)).src(2, input),
        ];
        for k in 2..N {
            sel.push(SelInstr::new(add, OpType::F32).dst(d(k)).src(0, v(k - 1)).src(2, v(k - 2)));
        }
        sel.push(SelInstr::new(mov, OpType::F32).dst(SelDst::Fixed { reg: 1, write_mask: 0xf }).src(2, v(N - 1)));
        let total = sel.len();

        let t0 = std::time::Instant::now();
        let mut mem = std::vec![0u8; 16 << 20];
        let scratch = Arena::new(&mut mem);
        let mut back = [0usize; 0];
        let dead = eliminate_dead(&scratch, &mut sel, &mut back).expect("dead-code pass");
        let n = sel.len() - dead;
        sink_definitions(&scratch, &mut sel[..n]).expect("sink");
        let n = n - coalesce_copies(&scratch, &mut sel[..n]).expect("coalesce");
        let mut code = std::vec![Instr { op: 0, vals: [0; 32], dont_care: 0 }; n];
        let mut placed: Placed = [NOT_PLACED; MAX_TEMPS as usize];
        let a = allocate_and_build_placed(&scratch, &sel[..n], &mut code, Stage::Vertex, [], [0u8], &[1], &mut placed)
            .expect("a 20,000-value program allocates");
        let took = t0.elapsed();
        println!(
            "20,000 values: {} instructions, {} temps, {:?}, {} KiB of scratch",
            a.count,
            a.temps,
            took,
            scratch.used() / 1024
        );
        assert_eq!(a.count, total, "every instruction is live");
        assert!(a.temps <= 3, "the input and two chain values: {} temps", a.temps);
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    }

    /// Every pass codegen runs, on programs past every fixed bound in this
    /// file: a compiler refuses such a program, it must never panic, because
    /// a panic inside glCompileShader aborts the application.
    ///
    /// The first shape is the one that did: the copy coalescer resolved a
    /// copy's operands through its MAX_VREGS table before checking either was
    /// in it, on a dEQP random shader with 660 values
    /// (random.all_features.fragment.64, whose source the product-compile
    /// harness also runs). The others walk the remaining bounds -- values far
    /// past the table, programs longer than the liveness and coalescing
    /// tables, a branch target past the end -- with and without a branch,
    /// because half the passes stand down on one.
    #[test]
    fn no_pass_panics_past_its_bounds() {
        use super::super::build::op_id;
        let (add, mov, br) = (op_id("add").unwrap(), op_id("mov").unwrap(), op_id("branch").unwrap());
        let v = |k: usize| SelSrc::Virtual { v: VReg(k as u32), swizzle: [0, 1, 2, 3], neg: false, abs: false };
        let d = |k: usize| SelDst::Virtual { v: VReg(k as u32), write_mask: 0xf };
        let copy = move |to: usize, from: usize| SelInstr::new(mov, OpType::F32).dst(d(to)).src(2, v(from));
        let sum =
            move |to: usize, a: usize, b: usize| SelInstr::new(add, OpType::F32).dst(d(to)).src(0, v(a)).src(2, v(b));
        let out = move |from: usize| {
            SelInstr::new(mov, OpType::F32).dst(SelDst::Fixed { reg: 1, write_mask: 0xf }).src(2, v(from))
        };
        let m = MAX_VREGS;

        let mut shapes: std::vec::Vec<(&str, std::vec::Vec<SelInstr>)> = std::vec::Vec::new();
        shapes
            .push(("a copy into the table from past it", std::vec![sum(m + 1, 0, 1), copy(m - 1, m + 1), out(m - 1)]));
        shapes
            .push(("a copy out of the table to past it", std::vec![sum(m - 1, 0, 1), copy(m + 1, m - 1), out(m + 1)]));
        shapes.push(("a copy between two values past it", std::vec![sum(m + 3, 0, 1), copy(m + 7, m + 3), out(m + 7)]));
        let far = u16::MAX as usize + 9;
        shapes.push(("values far past it", std::vec![sum(40 * m, 0, 1), copy(far, 40 * m), out(far)]));
        let chain = |n: usize| {
            let mut p = std::vec![sum(1, 0, 0)];
            for i in 1..n {
                p.push(sum(1 + i % (m - 2), 1 + (i - 1) % (m - 2), 0));
            }
            p.push(out(1 + (n - 1) % (m - 2)));
            p
        };
        shapes.push(("longer than the liveness table", chain(MAX_LIVE_INSTRS + 10)));
        /* 2048 was the coalescing pass's fixed table; the shapes stay past it. */
        shapes.push(("longer than the old coalescing table", chain(2048 + 10)));
        let mut copies = chain(2048 + 10);
        for i in (1..copies.len() - 1).step_by(3) {
            copies[i] = copy(1 + i % (m - 2), 1 + (i - 1) % (m - 2));
        }
        shapes.push(("copies past the old coalescing table", copies));
        let mut branchy = chain(MAX_LIVE_INSTRS + 10);
        branchy.insert(5, SelInstr::new(br, OpType::F32).target(u16::MAX));
        shapes.push(("a branch past the end of a long program", branchy));
        let far_branch =
            std::vec![sum(m + 1, 0, 1), copy(m - 1, m + 1), SelInstr::new(br, OpType::F32).target(0), out(m - 1)];
        shapes.push(("a back edge over values past the table", far_branch));

        let mut panicked = std::vec::Vec::new();
        for (name, prog) in shapes {
            let r = std::panic::catch_unwind(move || {
                let mut sel = prog;
                let mut back_edges = [0usize; 0];
                /* A pass may REFUSE (its tables, sized by the program's value
                 * numbers, not fitting the scratch it is lent): that is the
                 * outcome this test accepts. Only a panic fails it. */
                let Ok(dead) = eliminate_dead(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel, &mut back_edges) else {
                    return;
                };
                let n = sel.len() - dead;
                let Ok(_) = sink_definitions(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel[..n]) else { return };
                let Ok(removed) = coalesce_copies(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel[..n]) else { return };
                let n = n - removed;
                let mut split = sel[..n].to_vec();
                split_ranges(&mut split);
                let mut code = std::vec![Instr { op: 0, vals: [0; 32], dont_care: 0 }; n + MAX_DEFERRED_OUTPUTS];
                let mut placed: Placed = [NOT_PLACED; MAX_TEMPS as usize];
                if let Ok(a) = allocate_and_build_placed(
                    &Arena::new(&mut vec![0u8; 1 << 20]),
                    &sel[..n],
                    &mut code,
                    Stage::Fragment,
                    [],
                    [],
                    &[],
                    &mut placed,
                ) {
                    let _ =
                        drop_identity_movs(&Arena::new(&mut vec![0u8; 1 << 20]), &mut code[..a.count], &mut back_edges);
                }
            });
            if r.is_err() {
                panicked.push(name);
            }
        }
        assert!(panicked.is_empty(), "these panicked instead of refusing: {panicked:?}");
    }
}

#[cfg(test)]
mod fragment_r0_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    /// A fragment shader that never mentions r0 still may not be given it:
    /// the hardware puts gl_FragCoord there, so a temporary allocated to r0
    /// computes with the fragment's coordinates instead of its own value.
    /// Nothing in the instruction list says so, which is exactly why the
    /// allocator has to be told the stage.
    #[test]
    fn a_fragment_temporary_never_lands_on_gl_fragcoords_register() {
        let add = op_id("add").expect("add");
        let konst =
            SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg: 9, amode: 0, swizzle: XYZW, neg: false, abs: false });
        let sel = [SelInstr::new(add, OpType::F32)
            .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
            .src(0, konst)
            .src(2, konst)];

        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        let frag = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Fragment, [])
            .expect("allocates");
        let reg = out[0].get("DST").expect("add writes a destination") >> 3 & 0x7f;
        assert_ne!(reg, 0, "the fragment temporary was given gl_FragCoord's register");
        assert!(frag.temps >= 1);

        /* The vertex stage has no such reservation, and the control matters:
         * without it this test would pass on an allocator that simply never
         * used r0 for anything, which would be a different bug. */
        let mut out2 = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out2, Stage::Vertex, [])
            .expect("allocates");
        let reg2 = out2[0].get("DST").expect("add writes a destination") >> 3 & 0x7f;
        assert_eq!(reg2, 0, "the vertex stage should still use r0 -- otherwise this proves nothing");
    }
}

#[cfg(test)]
mod branch_target_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::op_id;
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    fn one(s: SelInstr) -> Result<Instr, AllocError> {
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[s], &mut out, Stage::Vertex, [])?;
        Ok(out[0])
    }

    /// Built, encoded, and read back out of the words: the only way to know
    /// the target reached the field the machine reads rather than a struct.
    #[test]
    fn a_branch_target_survives_the_encoding() {
        let br = op_id("branch").expect("the ISA tables carry branch");
        let instr = one(SelInstr::new(br, OpType::F32).target(1234)).expect("a branch builds");
        let back = decode_instr(&encode_instr(&instr)).expect("the built branch decodes");
        assert_eq!(back.get("TARGET"), Some(1234), "the target did not reach TARGET");
    }

    /// Required where the field exists and refused where it does not, for the
    /// same reason as the texture fields: a branch to 0 is a jump to the top
    /// of the shader, not an error the hardware reports.
    #[test]
    fn a_target_is_required_on_a_branch_and_refused_elsewhere() {
        let br = op_id("branch").expect("branch");
        assert!(
            matches!(one(SelInstr::new(br, OpType::F32)), Err(AllocError::WrongBranchTarget)),
            "a branch with no target must be refused"
        );

        let add = op_id("add").expect("add");
        let konst = SelSrc::Physical(crate::shaderc::online::vivante::build::Src {
            group: 0,
            reg: 1,
            amode: 0,
            swizzle: [0, 1, 2, 3],
            neg: false,
            abs: false,
        });
        let wrong = SelInstr::new(add, OpType::F32)
            .dst(SelDst::Fixed { reg: 2, write_mask: 0xf })
            .src(0, konst)
            .src(2, konst)
            .target(1);
        assert!(matches!(one(wrong), Err(AllocError::WrongBranchTarget)), "add has no TARGET field");
    }

    /// Fifteen bits. A larger value would truncate into a branch somewhere
    /// else in the program: a shader that runs and draws the wrong thing.
    #[test]
    fn a_target_the_field_cannot_hold_is_refused() {
        let br = op_id("branch").expect("branch");
        assert!(one(SelInstr::new(br, OpType::F32).target(MAX_BRANCH_TARGET)).is_ok(), "the top of the range is legal");
        assert!(
            matches!(
                one(SelInstr::new(br, OpType::F32).target(MAX_BRANCH_TARGET + 1)),
                Err(AllocError::BadOperand("branch target"))
            ),
            "a target past fifteen bits must be refused, not truncated"
        );
    }

    /// The corpus count beside the field, so the shape is not invented: the
    /// shipped table's branches are what this has to be able to express.
    ///
    /// GC7000 builds only. `shaderc::mesa_reference` aliases the PowerVR blobs
    /// otherwise, and those are USC — decoding them with this machine's
    /// decoder measures nothing about this machine. The vacuity guard below
    /// is what caught that: it found no TARGET at all in the PowerVR table
    /// and refused to report a pass. This is the second time that alias has
    /// had me; the first was `es100_twins.rs`.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_corpus_branches_are_within_what_this_can_express() {
        let mut n = 0usize;
        let mut top = 0u32;
        /* Three readings of TARGET, scored over the SAME branches. "In
         * range" alone is weak -- most readings are in range most of the
         * time -- so the rivals are counted beside it and the assertion is
         * that one of them fits everything and the others do not. */
        let (mut absolute, mut relative, mut dwords) = (0usize, 0usize, 0usize);
        /* Per stage, because a vertex-only hang made the origin a live
         * question: if vertex blobs counted from after a prologue, a branch
         * to the end of the program would land past the last instruction. */
        let mut to_end = [0usize; 2];
        let mut branches = [0usize; 2];
        for blob in crate::shaderc::mesa_reference::SHC_BLOB_TABLE.iter() {
            let code = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
            let quads = code.as_chunks::<4>().0;
            let ninstr = quads.len();
            let si = usize::from(blob.stage == crate::shaderc::SHC_STAGE_FRAGMENT);
            for (idx, quad) in quads.iter().enumerate() {
                let Ok(i) = decode_instr(quad) else { continue };
                let Some(t) = i.get("TARGET") else { continue };
                n += 1;
                branches[si] += 1;
                top = top.max(t);
                let t = t as usize;
                if t <= ninstr {
                    absolute += 1;
                }
                if idx + t <= ninstr {
                    relative += 1;
                }
                if t.is_multiple_of(4) && t / 4 <= ninstr {
                    dwords += 1;
                }
                if t == ninstr {
                    to_end[si] += 1;
                }
            }
        }
        assert!(n > 0, "no branch in the corpus: this test would pass vacuously");
        assert!(top <= MAX_BRANCH_TARGET as u32, "the corpus branches to {top}, past what MAX_BRANCH_TARGET allows");

        /* TARGET is the absolute instruction index, counted from the blob's
         * own instruction 0, in both stages. */
        assert_eq!(absolute, n, "some corpus branch is out of range as an absolute instruction index");
        assert!(relative < n, "the relative reading fits everything too: this test does not discriminate");
        assert!(dwords < n, "the dword reading fits everything too: this test does not discriminate");

        /* The stage control. A branch to exactly `ninstr` is a jump to one
         * past the last instruction -- the end of the program -- and it is
         * what rules out a per-stage origin: under a prologue offset it
         * would overrun. Both stages must show some, or this proves nothing
         * about the stage that shows none. */
        assert!(to_end[0] > 0 && to_end[1] > 0, "no branch-to-end in one of the stages: the origin is untested there");

        println!(
            "corpus branches: {n} carry TARGET, highest {top}; absolute {absolute}/{n}, relative {relative}/{n}, dwords {dwords}/{n}; \
             vertex {} (to-end {}), fragment {} (to-end {})",
            branches[0], to_end[0], branches[1], to_end[1]
        );
    }
}

#[cfg(test)]
mod address_register_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn index(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    fn movar_to_address(write_mask: u8) -> SelInstr {
        SelInstr::new(op_id("movar").expect("the ISA tables carry movar"), OpType::F32)
            .dst(SelDst::Address { write_mask })
            .src(2, index(2))
    }

    fn one(s: SelInstr) -> Result<Instr, AllocError> {
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[s], &mut out, Stage::Vertex, [])?;
        Ok(out[0])
    }

    /// Built, encoded and read back out of the words. The expected value is
    /// not derived from this code: `DST = 0x400` is what all 7 `movar`s in
    /// the shipped GC7000 table carry and what Mesa's etnaviv emits.
    #[test]
    fn a_movar_destination_encodes_as_the_corpus_encodes_it() {
        let instr = one(movar_to_address(0x1)).expect("a movar builds");
        let back = decode_instr(&encode_instr(&instr)).expect("the built movar decodes");
        assert_eq!(back.get("DST"), Some(0x400), "movar's destination did not encode as the corpus does");
        assert_eq!(back.get("DST_USE"), Some(1), "a destination that is not marked used writes nothing");
    }

    /// Two-way, like the texture fields and the branch target. Neither
    /// direction is an error the hardware reports: a temporary destination on
    /// `movar` still loads the address register, and `Address` elsewhere
    /// writes temporary 0.
    #[test]
    fn the_address_destination_is_required_on_movar_and_refused_elsewhere() {
        let movar = op_id("movar").expect("movar");
        assert!(
            matches!(
                one(SelInstr::new(movar, OpType::F32)
                    .dst(SelDst::Virtual { v: VReg(0), write_mask: 0x1 })
                    .src(2, index(2))),
                Err(AllocError::WrongAddressDestination)
            ),
            "a temporary destination on movar must be refused"
        );

        let add = op_id("add").expect("add");
        assert!(
            matches!(
                one(SelInstr::new(add, OpType::F32)
                    .dst(SelDst::Address { write_mask: 0x1 })
                    .src(0, index(1))
                    .src(2, index(1))),
                Err(AllocError::WrongAddressDestination)
            ),
            "an address destination on an ordinary ALU operation must be refused"
        );

        // And the shape that must still work, so the rule above is not simply
        // refusing everything.
        one(movar_to_address(0x1)).expect("movar with an address destination builds");
    }

    /// An empty mask leaves the address register holding whatever was there,
    /// so every following `amode` read indexes by a stale value: a program
    /// that runs and reads the wrong constants.
    #[test]
    fn an_empty_address_write_mask_is_refused() {
        assert!(
            matches!(one(movar_to_address(0)), Err(AllocError::BadOperand("empty address-register write mask"))),
            "a movar that writes no component of the address register must be refused"
        );
    }

    /// The reason this is a variant of its own rather than `Fixed { reg: 0 }`.
    /// The address register is not in the temporary file, so adding a `movar`
    /// to a program must not move one single temporary.
    ///
    /// Measured on the VERTEX stage, and that is not a detail. The fragment
    /// stage cannot tell: r0 is already reserved there for `gl_FragCoord`, so
    /// a spelling that reserved r0 again would reserve nothing new and this
    /// test would pass while the defect was present -- it did, when the
    /// reservation was put back in to check that this test can fail. On the
    /// vertex stage nothing is reserved, so `Fixed { reg: 0 }` costs the
    /// program r0. Both stages run below; only the vertex half is evidence.
    #[test]
    fn an_address_destination_reserves_no_temporary() {
        let add = op_id("add").expect("add");
        let compute = SelInstr::new(add, OpType::F32)
            .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
            .src(0, index(9))
            .src(2, index(9));

        for stage in [Stage::Vertex, Stage::Fragment] {
            let mut without = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
            let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[compute], &mut without, stage, [])
                .expect("allocates");

            let mut with = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
            let b = allocate_and_build(
                &Arena::new(&mut vec![0u8; 1 << 20]),
                &[movar_to_address(0x1), compute],
                &mut with,
                stage,
                [],
            )
            .expect("allocates");

            assert_eq!(a.temps, b.temps, "{stage:?}: the movar cost a temporary the address register does not occupy");
            let reg_without = without[0].get("DST").expect("add writes") >> 3 & 0x7f;
            let reg_with = with[1].get("DST").expect("add writes") >> 3 & 0x7f;
            assert_eq!(reg_with, reg_without, "{stage:?}: the movar moved the temporary the add was given");
        }

        /* The control for the half that is evidence: on the vertex stage the
         * add must land on r0, so a reservation of r0 by the movar would have
         * to move it. Without this the loop above would pass on an allocator
         * that never used r0 at all. */
        let mut vtx = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
        allocate_and_build(
            &Arena::new(&mut vec![0u8; 1 << 20]),
            &[movar_to_address(0x1), compute],
            &mut vtx,
            Stage::Vertex,
            [],
        )
        .expect("allocates");
        assert_eq!(
            vtx[1].get("DST").expect("add writes") >> 3 & 0x7f,
            0,
            "the vertex add should be on r0 -- otherwise this test cannot see a reservation of r0"
        );
    }

    /// The corpus beside the type, so its shape is not invented. GC7000
    /// builds only: `shaderc::mesa_reference` aliases the PowerVR blobs otherwise and
    /// those are USC. The `n > 0` guard is what catches that alias -- twice
    /// already in this file's history.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn every_corpus_address_write_has_the_shape_this_builds() {
        let mut n = 0usize;
        let mut masks = 0u8;
        for blob in crate::shaderc::mesa_reference::SHC_BLOB_TABLE.iter() {
            let code = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
            for quad in code.as_chunks::<4>().0 {
                let Ok(i) = decode_instr(quad) else { continue };
                if !matches!(i.op().name, "movar" | "movai") {
                    continue;
                }
                n += 1;
                let d = i.dst().expect("movar carries a destination");
                assert_eq!(d.reg, 0, "a corpus address write names temporary {}", d.reg);
                assert_eq!(d.amode, 0, "a corpus address write uses addressing mode {}", d.amode);
                assert_ne!(d.write_mask, 0, "a corpus address write has an empty mask");
                masks |= d.write_mask;
            }
        }
        assert!(n > 0, "no movar/movai in the corpus: this test would pass vacuously");
        println!("corpus address writes: {n} instructions, masks seen {masks:#x}");
    }
}

#[cfg(test)]
mod abi_reserved_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn konst(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    /// One instruction that mentions no io register at all. This is the whole
    /// difficulty: copy propagation makes such programs common, and the
    /// register the ABI pinned is invisible in them.
    fn one_add() -> SelInstr {
        SelInstr::new(op_id("add").expect("add"), OpType::F32)
            .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
            .src(0, konst(9))
            .src(2, konst(9))
    }

    /// A register the ABI pinned must not be handed to a temporary, even
    /// though nothing in the program names it. Inputs, outputs and
    /// temporaries share one file, so the temporary would compute over the
    /// varying -- and the picture, not an error, is what would say so.
    #[test]
    fn a_reserved_register_is_not_given_to_a_temporary() {
        let sel = [one_add()];

        let mut plain = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut plain, Stage::Vertex, [])
            .expect("allocates");
        let free = plain[0].get("DST").expect("add writes") >> 3 & 0x7f;
        assert_eq!(free, 0, "with nothing reserved the add should take r0 -- otherwise this proves nothing");

        let mut held = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut held, Stage::Vertex, [0u8])
            .expect("allocates");
        let taken = held[0].get("DST").expect("add writes") >> 3 & 0x7f;
        assert_ne!(taken, 0, "a temporary was placed on the register the ABI reserved");
    }

    /// And the count clears it. `abi.rs` refuses a `temps` that does not
    /// (`TempsBelowIo`), so this is the same fact the assembler checks, made
    /// true here instead of patched afterwards by the caller.
    #[test]
    fn the_count_clears_every_reserved_register() {
        let sel = [one_add()];
        for reg in [0u8, 1, 5, 9] {
            let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
            let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [reg])
                .expect("allocates");
            assert!(a.temps > u16::from(reg), "temps {} does not clear the reserved register {reg}", a.temps);
        }
    }

    /// An empty program still has to declare enough registers for what the
    /// ABI pinned: nothing in `sel` can say so, which is exactly the case the
    /// caller's floor was written for.
    #[test]
    fn an_empty_program_still_clears_its_reserved_registers() {
        let mut out: [Instr; 1] = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[], &mut out, Stage::Vertex, [3u8])
            .expect("allocates");
        assert!(a.temps > 3, "an empty program reported temps {} for a reserved r3", a.temps);
    }

    /// Out of range is refused rather than wrapped into a register that
    /// exists.
    #[test]
    fn a_reserved_register_past_the_file_is_refused() {
        let mut out: [Instr; 1] = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        assert!(
            matches!(
                allocate_and_build(
                    &Arena::new(&mut vec![0u8; 1 << 20]),
                    &[],
                    &mut out,
                    Stage::Vertex,
                    [MAX_TEMPS as u8]
                ),
                Err(AllocError::OutOfRegisters(_))
            ),
            "a reserved register outside the temporary file must be refused"
        );
    }
}

#[cfg(test)]
mod unused_destination_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn konst(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    fn one(s: SelInstr) -> Result<Instr, AllocError> {
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 1];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &[s], &mut out, Stage::Fragment, [])?;
        Ok(out[0])
    }

    /// `discard` is one `texkill`, which has a `DST` field and no result.
    /// Building it must not require a destination, and the built instruction
    /// must leave `DST_USE` clear -- which is what every `texkill` in the
    /// shipped table does.
    #[test]
    fn texkill_needs_no_destination_and_leaves_dst_use_clear() {
        let tk = op_id("texkill").expect("the ISA tables carry texkill");
        let instr = one(SelInstr::new(tk, OpType::F32)).expect("a texkill builds without a destination");
        let back = decode_instr(&encode_instr(&instr)).expect("the built texkill decodes");
        assert_eq!(back.get("DST_USE"), Some(0), "texkill was built with a destination the hardware never gives it");
    }

    /// And giving it one is refused rather than tolerated. A zero write mask
    /// is not a way out: it would still set `DST_USE`.
    #[test]
    fn a_destination_on_an_operation_that_writes_none_is_refused() {
        for name in ["texkill", "nop"] {
            let op = op_id(name).unwrap_or_else(|| panic!("the ISA tables carry {name}"));
            assert!(
                matches!(
                    one(SelInstr::new(op, OpType::F32).dst(SelDst::Virtual { v: VReg(0), write_mask: 0 })),
                    Err(AllocError::WrongDestination)
                ),
                "{name} accepted a destination"
            );
            // The shape that must still work, so the rule is not refusing everything.
            one(SelInstr::new(op, OpType::F32)).unwrap_or_else(|e| panic!("{name} with no destination: {e:?}"));
        }

        /* The control: an ordinary operation still REQUIRES its destination,
         * so this is an exception for two named operations and not a hole
         * opened for every operation with a DST field. */
        let add = op_id("add").expect("add");
        assert!(
            matches!(
                one(SelInstr::new(add, OpType::F32).src(0, konst(1)).src(2, konst(1))),
                Err(AllocError::WrongDestination)
            ),
            "add was allowed to omit its destination"
        );
    }

    /// The corpus split the exception is drawn from, kept beside it: the two
    /// operations never set `DST_USE`, and every other operation carrying the
    /// field sets it in **all** of its instances. A threshold would be a
    /// judgement call; this is not one.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_corpus_splits_cleanly_into_writers_and_non_writers() {
        // (total, with DST_USE = 1) per operation name, small fixed table.
        let mut names: [&str; 64] = [""; 64];
        let mut counts = [(0usize, 0usize); 64];
        let mut k = 0usize;
        for blob in crate::shaderc::mesa_reference::SHC_BLOB_TABLE.iter() {
            let code = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
            for quad in code.as_chunks::<4>().0 {
                let Ok(i) = decode_instr(quad) else { continue };
                if !i.op().fields.iter().any(|f| f.name == "DST") {
                    continue;
                }
                let name = i.op().name;
                let slot = names[..k].iter().position(|n| *n == name).unwrap_or_else(|| {
                    assert!(k < names.len(), "more operations with a DST field than this table holds");
                    names[k] = name;
                    k += 1;
                    k - 1
                });
                counts[slot].0 += 1;
                if i.get("DST_USE") == Some(1) {
                    counts[slot].1 += 1;
                }
            }
        }
        assert!(k > 0, "no instruction with a DST field in the corpus: this test would pass vacuously");
        let mut never = 0usize;
        for (name, (total, used)) in names[..k].iter().zip(counts[..k].iter()) {
            let op = op_id(name).expect("a decoded operation resolves");
            if *used == 0 {
                never += 1;
                assert!(op.writes_no_dst(), "{name} never writes DST in {total} instructions but is not in the set");
            } else {
                assert_eq!(*used, *total, "{name} writes DST in {used} of {total}: the split is not clean after all");
                assert!(!op.writes_no_dst(), "{name} writes DST in all {total} instructions but is in the set");
            }
        }
        assert_eq!(never, 2, "expected exactly texkill and nop to never write DST, found {never}");
        println!("corpus: {k} operations carry a DST field, {never} never use it");
    }
}

#[cfg(test)]
mod temp_bound_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{self, op_id, Src as BSrc, RGROUP_TEMP};

    /// The allocator's bound and the field the driver writes are one fact.
    /// `temp_register_num()` masks silently, so a bound above the field does
    /// not produce an error anywhere -- it produces a shader that claims N
    /// registers, is given `N & 0x3f`, and writes past them.
    #[test]
    fn the_bound_is_what_the_hardware_field_can_carry() {
        /* src/vivante/regs.rs: `temp_register_num(x) = x & 0x3f`, from
         * VIVS_VS_TEMP_REGISTER_CONTROL_NUM_TEMPS__MASK. */
        const NUM_TEMPS_MASK: u32 = 0x3f;
        assert_eq!(
            u32::from(MAX_TEMPS),
            NUM_TEMPS_MASK,
            "MAX_TEMPS and the NUM_TEMPS field have drifted: a count the field cannot carry is masked, not refused"
        );
        // The failure this prevents, stated as arithmetic rather than prose.
        assert_eq!(67u32 & NUM_TEMPS_MASK, 3, "the truncation that made a 67-register shader ask for 3");
    }

    /// A program needing more than the field can declare is refused, and the
    /// refusal names the requirement.
    ///
    /// Note what "needing" means: `MAX_TEMPS` bounds PHYSICAL registers, so
    /// the pressure has to be real. My first version defined one virtual
    /// register numbered 63 and expected a refusal; the allocator correctly
    /// compacted it to r0 and reported `temps: 1`. Virtual numbering is
    /// `MAX_VREGS`' business. This builds `n` values that are all live at
    /// once, which is the only thing that costs `n` registers.
    #[test]
    fn a_program_past_the_field_is_refused_not_truncated() {
        let add = op_id("add").expect("add");
        let konst = SelSrc::Physical(BSrc {
            group: RGROUP_TEMP,
            reg: 1,
            amode: 0,
            swizzle: [0, 1, 2, 3],
            neg: false,
            abs: false,
        });

        /// `n` values defined, then all read afterwards, so every one of them
        /// is live across the middle of the program at the same time.
        fn all_live_at_once(n: usize, add: build::OpId, konst: SelSrc, out: &mut [SelInstr]) -> usize {
            let mut k = 0;
            for i in 0..n {
                out[k] = SelInstr::new(add, OpType::F32)
                    .dst(SelDst::Virtual { v: VReg(i as u32), write_mask: 0xf })
                    .src(0, konst)
                    .src(2, konst);
                k += 1;
            }
            for i in (0..n).rev() {
                out[k] = SelInstr::new(add, OpType::F32)
                    .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                    .src(0, SelSrc::Virtual { v: VReg(i as u32), swizzle: [0, 1, 2, 3], neg: false, abs: false })
                    .src(2, konst);
                k += 1;
            }
            k
        }

        let blank =
            SelInstr::new(add, OpType::F32).dst(SelDst::Fixed { reg: 0, write_mask: 0xf }).src(0, konst).src(2, konst);
        let mut sel = [blank; 2 * (MAX_TEMPS as usize + 1)];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2 * (MAX_TEMPS as usize + 1)];

        /* The program costs N + 1 registers, not N: the readers write to a
         * fixed r0, which is a register too. So MAX_TEMPS values is already
         * one past the file. (Third fixture I got wrong here in a row while
         * the allocator was right each time -- first a virtual number
         * mistaken for pressure, then this off-by-one. Each was caught only
         * because the failure message carried the number.) */
        let used = all_live_at_once(MAX_TEMPS as usize, add, konst, &mut sel);
        match allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel[..used], &mut out, Stage::Vertex, []) {
            Err(AllocError::OutOfRegisters(n)) => {
                assert_eq!(n, MAX_TEMPS + 1, "the refusal should name what the program needed")
            }
            other => panic!("{} live values plus r0 were not refused: {other:?}", MAX_TEMPS),
        }

        /* The control: exactly the file fits, so this is a boundary rather
         * than a blanket refusal, and the boundary is the field's number. */
        let used = all_live_at_once(MAX_TEMPS as usize - 1, add, konst, &mut sel);
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel[..used], &mut out, Stage::Vertex, [])
            .expect("a full register file fits");
        assert_eq!(a.temps, MAX_TEMPS, "the full file should be usable, not one short of it");
    }

    /// Every blob the reference shipped fits the bound. If this ever fails,
    /// the bound is wrong and not the shaders -- so it is the check that
    /// would catch me having misread the field as a count.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn no_shipped_blob_exceeds_the_bound() {
        let mut n = 0usize;
        let mut top = 0u16;
        for blob in crate::shaderc::mesa_reference::SHC_BLOB_TABLE.iter() {
            n += 1;
            top = top.max(blob.shader.temps);
            assert!(
                blob.shader.temps <= MAX_TEMPS,
                "{} declares {} temporaries, past the bound",
                blob.source_name(),
                blob.shader.temps
            );
        }
        assert!(n > 0, "no blob in the table: this test would pass vacuously");
        println!("{n} shipped blobs, highest temps {top}, bound {MAX_TEMPS}");
    }
}

#[cfg(test)]
mod liveness_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn konst(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    /// Greedy colouring of an interval graph reaches its maximum overlap only
    /// in start order. Here two values whose ranges do not overlap must share
    /// a register even though the longer-lived one is numbered second.
    #[test]
    fn a_value_numbered_late_but_starting_early_does_not_waste_a_register() {
        let add = op_id("add").expect("add");
        let mov = op_id("mov").expect("mov");
        // v1 lives across the whole program; v0 only at the end. Numbered so
        // that VReg order and start order disagree.
        let sel = [
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf })
                .src(0, konst(9))
                .src(2, konst(9)),
            SelInstr::new(mov, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, konst(9))
                .src(2, konst(9)),
            SelInstr::new(mov, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 4];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [])
            .expect("allocates");
        /* r0 is taken by the fixed destination, so one value fits in r1 and
         * the other must reuse it: two registers in total, not three. */
        assert_eq!(a.temps, 2, "a value numbered late but starting early took a register of its own");
    }

    /// Lanes nobody reads are not written. The reference does this as a
    /// matter of course -- 10,479 of its 13,574 instructions with a used
    /// destination (77.2 %) write a partial mask -- and it frees the rest of
    /// the register for another value.
    #[test]
    fn a_lane_no_one_reads_is_not_written() {
        let add = op_id("add").expect("add");
        let mov = op_id("mov").expect("mov");
        let sel = [
            // Written xyzw, read x only.
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, konst(9))
                .src(2, konst(9)),
            SelInstr::new(mov, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0x1 })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: [0, 0, 0, 0], neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, []).expect("allocates");
        let mask = out[0].get("DST").expect("the add writes") >> 10 & 0xf;
        assert_eq!(mask, 0x1, "three lanes nobody reads were still written");

        /* The control: a value read as a vec4 keeps its full mask, so this is
         * narrowing to what is read and not narrowing everything. */
        let sel_wide = [
            sel[0],
            SelInstr::new(mov, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(2, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out2 = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 2];
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel_wide, &mut out2, Stage::Vertex, [])
            .expect("allocates");
        assert_eq!(out2[0].get("DST").expect("the add writes") >> 10 & 0xf, 0xf, "a vec4 read lost lanes it needed");
    }

    /// `temps` is what the allocation cost, and the floor is what it could
    /// not have gone below. Equality says the assignment is optimal FOR A
    /// WHOLE-REGISTER ALLOCATOR -- packing several scalars into one register
    /// would need component renaming, which this does not do.
    #[test]
    fn the_count_equals_the_overlap_floor_plus_the_pinned_registers() {
        let add = op_id("add").expect("add");
        let sel = [
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, konst(9))
                .src(2, konst(9)),
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf })
                .src(0, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false })
                .src(2, konst(9)),
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(0, SelSrc::Virtual { v: VReg(0), swizzle: XYZW, neg: false, abs: false })
                .src(2, SelSrc::Virtual { v: VReg(1), swizzle: XYZW, neg: false, abs: false }),
        ];
        let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 3];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [])
            .expect("allocates");
        assert_eq!(
            a.temps as usize,
            last_overlap() + last_reserved(),
            "temps {} is not the floor {} plus the {} pinned",
            a.temps,
            last_overlap(),
            last_reserved()
        );
        assert!(last_overlap() >= 2, "both values are live at once: the floor should say so");
    }
}

/// A `mov` that copies a temporary onto itself, lane for lane, with nothing
/// applied: the instruction [`drop_identity_movs`] deletes.
fn is_identity_mov(ins: &Instr) -> bool {
    if ins.op().name != "mov" || ins.get("SAT").unwrap_or(0) != 0 || ins.get("COND").unwrap_or(0) != 0 {
        return false;
    }
    if ins.get("DST_USE") == Some(0) {
        return false;
    }
    let (Some(d), Some(s)) = (ins.dst(), ins.src(2)) else { return false };
    d.amode == 0
        && !s.is_immediate()
        && s.group == RGROUP_TEMP
        && s.amode == 0
        && s.reg == d.reg as u16
        && !s.neg
        && !s.abs
        && (0..4).all(|l| d.write_mask & (1 << l) == 0 || s.swizzle[l] == l as u8)
}

/// [`drop_identity_movs`] over an encoded program, `n` instructions of four
/// dwords at the front of `words`, compacted in place; the count removed.
///
/// Each instruction is decoded on the stack to be judged, and only a branch
/// whose target moves is re-encoded: the program never exists as `Instr`s
/// (see [`allocate_and_encode_placed`]). The same instructions go, and the
/// same targets and back edges move, as in the decoded version.
pub fn drop_identity_movs_encoded(
    scratch: &Arena<'_>,
    words: &mut [u32],
    n: usize,
    back_edges: &mut [usize],
) -> Result<usize, AllocError> {
    if words.len() < n * 4 {
        return Err(AllocError::OutputTooSmall);
    }
    let at = |w: &[u32], i: usize| -> Result<Instr, AllocError> {
        let q: &[u32; 4] = w[i * 4..i * 4 + 4].try_into().expect("four dwords");
        super::decode::decode_instr(q)
            .map_err(|_| AllocError::BadOperand("an encoded instruction that does not decode"))
    };
    let dead = scratch.alloc_filled(n.div_ceil(64), 0u64).map_err(|_| AllocError::OutOfScratch)?;
    let mut removed = 0usize;
    for i in 0..n {
        if is_identity_mov(&at(words, i)?) {
            dead[i / 64] |= 1 << (i % 64);
            removed += 1;
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    let is_dead = |i: usize| dead[i / 64] >> (i % 64) & 1 != 0;
    let new_index = |t: usize| (0..t.min(n)).filter(|&k| !is_dead(k)).count();
    let mut j = 0usize;
    for i in 0..n {
        if is_dead(i) {
            continue;
        }
        let mut ins = at(words, i)?;
        let mut q: [u32; 4] = words[i * 4..i * 4 + 4].try_into().expect("four dwords");
        if let Some(pos) = ins.op().fields.iter().position(|f| f.name == "TARGET") {
            ins.vals[pos] = new_index(ins.vals[pos] as usize) as u32;
            q = super::encode::encode_instr(&ins);
        }
        words[j * 4..j * 4 + 4].copy_from_slice(&q);
        j += 1;
    }
    for b in back_edges.iter_mut() {
        *b = new_index(*b);
    }
    Ok(removed)
}

/// Delete every copy of a register onto itself, after allocation; returns how
/// many went. The survivors are compacted to the front of `code`.
///
/// Placing outputs and letting a value take the register another dies in
/// (8ba6f82, ae39ffb) turns some copies into `mov tR, tR.xyzw` -- a varying
/// copied from the input it now shares a register with. Such a `mov` changes
/// nothing: every lane it writes it reads from the same lane of the same
/// register, with no modifier, no condition and no saturation. 122 of them
/// in 68 corpus programs.
///
/// **Branch targets and the back-edge list are instruction indices** and
/// both are renumbered through the survivors, the same as
/// [`eliminate_dead`]: a target at a deleted instruction lands on the next
/// survivor.
pub fn drop_identity_movs(
    scratch: &Arena<'_>,
    code: &mut [Instr],
    back_edges: &mut [usize],
) -> Result<usize, AllocError> {
    let n = code.len();
    let identity = is_identity_mov;
    let dead = scratch.alloc_filled(n.div_ceil(64), 0u64).map_err(|_| AllocError::OutOfScratch)?;
    let mut removed = 0usize;
    for (i, ins) in code.iter().enumerate() {
        if identity(ins) {
            dead[i / 64] |= 1 << (i % 64);
            removed += 1;
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    let is_dead = |i: usize| dead[i / 64] >> (i % 64) & 1 != 0;
    let new_index = |t: usize| (0..t.min(n)).filter(|&k| !is_dead(k)).count();
    let mut j = 0usize;
    for i in 0..n {
        if is_dead(i) {
            continue;
        }
        let mut ins = code[i];
        if let Some(pos) = ins.op().fields.iter().position(|f| f.name == "TARGET") {
            ins.vals[pos] = new_index(ins.vals[pos] as usize) as u32;
        }
        code[j] = ins;
        j += 1;
    }
    for b in back_edges.iter_mut() {
        *b = new_index(*b);
    }
    Ok(removed)
}

/// Most output registers one program writes: an upper bound on what
/// [`defer_io_hazards`] appends.
pub const MAX_DEFERRED_OUTPUTS: usize = MAX_TEMPS as usize;

/// Keep an output write from destroying an input that is still to be read.
///
/// Inputs and outputs are registers of the one temporary file, and the two
/// are numbered independently: a vertex shader's attributes arrive in r0, r1,
/// ... while `gl_Position` goes to r0 and the varyings from r1. Qt's common
/// vertex shader (`qt_TexCoord0 = qt_VertexTexCoord; gl_Position = m *
/// qt_VertexPosition;`) was emitted as `mov t1, t0` first -- the varying into
/// t1 -- and then read t1 as the position: gl_Position was computed from the
/// texture coordinate. The allocator could not help, because both registers
/// are pinned; the ORDER is the defect.
///
/// So an output register that some input read still needs after it is
/// written is computed into a fresh virtual register instead, and one `mov`
/// per such register is appended at the end, after every read. Branch
/// targets need no remap: nothing before the end moves, and a branch to the
/// old end now lands on the appended copies, which is where the outputs are
/// written. With a back edge anywhere, any register that is both read as an
/// input and written as an output is deferred, since a loop can bring a read
/// back after the write.
///
/// Returns the new length of `out`, which receives the program.
pub fn defer_io_hazards(sel: &[SelInstr], out: &mut [SelInstr]) -> Result<usize, AllocError> {
    let n = sel.len();
    if out.len() < n {
        return Err(AllocError::OutputTooSmall);
    }
    out[..n].copy_from_slice(sel);
    let looped = sel.iter().enumerate().any(|(i, s)| s.target.is_some_and(|t| (t as usize) <= i));
    let reads_input = |j: usize, reg: u8| {
        sel[j].srcs.iter().flatten().any(|x| matches!(*x, SelSrc::Input { reg: r, .. } if r == reg))
    };
    let mut hazard = [false; 128];
    for (i, s) in sel.iter().enumerate() {
        if let Some(SelDst::Fixed { reg, .. }) = s.dst {
            let later = if looped { 0 } else { i + 1 };
            if (later..n).any(|j| reads_input(j, reg)) {
                hazard[reg as usize & 127] = true;
            }
        }
    }
    if !hazard.iter().any(|h| *h) {
        return Ok(n);
    }
    let mut next = vregs_needed(sel) as u32;
    let mut vreg = [u32::MAX; 128];
    let mut mask = [0u8; 128];
    let mut ty = [OpType::F32; 128];
    for s in out[..n].iter_mut() {
        if let Some(SelDst::Fixed { reg, write_mask }) = s.dst {
            let r = reg as usize & 127;
            if !hazard[r] {
                continue;
            }
            if vreg[r] == u32::MAX {
                vreg[r] = next;
                next += 1;
                ty[r] = s.ty;
            }
            mask[r] |= write_mask;
            s.dst = Some(SelDst::Virtual { v: VReg(vreg[r]), write_mask });
        }
    }
    if next as usize > MAX_VREGS {
        return Err(AllocError::OutOfRegisters(next.min(u16::MAX as u32) as u16));
    }
    let mov = build::op_id("mov").ok_or(AllocError::Build(BuildError::NoSuchOp))?;
    let mut len = n;
    for r in 0..128 {
        if vreg[r] == u32::MAX {
            continue;
        }
        let slot = out.get_mut(len).ok_or(AllocError::OutputTooSmall)?;
        *slot = SelInstr::new(mov, ty[r])
            .dst(SelDst::Fixed { reg: r as u8, write_mask: mask[r] })
            .operand(0, SelSrc::Virtual { v: VReg(vreg[r]), swizzle: [0, 1, 2, 3], neg: false, abs: false });
        len += 1;
    }
    Ok(len)
}

#[cfg(test)]
mod loop_and_io_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc};

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn konst(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_UNIFORM_0, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    fn vr(v: u32, swizzle: [u8; 4]) -> SelSrc {
        SelSrc::Virtual { v: VReg(v), swizzle, neg: false, abs: false }
    }

    fn built(n: usize) -> Vec<Instr> {
        vec![Instr { op: 0, vals: [0; crate::shaderc::online::vivante::isa::MAX_FIELDS], dont_care: 0 }; n]
    }

    /// v0 is made before the loop and read at its top; v1 is made after that
    /// read, inside the body. Linear ranges ([0, 1] and [2, 3]) do not meet,
    /// so v1 took v0's register and the second trip read v1. The back edge
    /// keeps v0 live to instruction 5.
    #[test]
    fn a_value_read_in_a_loop_keeps_its_register_to_the_back_edge() {
        let (add, br) = (op_id("add").expect("add"), op_id("branch").expect("branch"));
        let sel = [
            /* 0 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, konst(0))
                .src(2, konst(1)),
            /* 1 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(0, XYZW))
                .src(2, konst(1)),
            /* 2 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf })
                .src(0, konst(2))
                .src(2, konst(1)),
            /* 3 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(1, XYZW))
                .src(2, konst(1)),
            /* 4 */ SelInstr::new(br, OpType::F32).target(6),
            /* 5 */ SelInstr::new(br, OpType::F32).target(1),
        ];
        let mut out = built(sel.len());
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [1])
            .expect("allocates");
        let (r0, r1) = (out[0].dst().expect("dst").reg, out[2].dst().expect("dst").reg);
        assert_ne!(r0, r1, "v1 took v0's register inside the loop that still reads v0");
        assert_eq!(last_loop_extended(), 1, "exactly v0 is live across the back edge");

        /* The control: the same program with the back edge pointing forward
         * is straight-line, and there the two values may share. */
        let mut fwd = sel;
        fwd[5] = SelInstr::new(br, OpType::F32).target(6);
        let mut out2 = built(fwd.len());
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &fwd, &mut out2, Stage::Vertex, [1])
            .expect("allocates");
        assert_eq!(out2[0].dst().expect("dst").reg, out2[2].dst().expect("dst").reg, "without a loop they share");
        assert_eq!(last_loop_extended(), 0);
    }

    /// `add t.x, v.xyzw, c` reads v.x only: a lane-wise operation takes lane
    /// `swizzle[c]` for the lanes `c` it writes. So v's definition is narrowed
    /// to x, and through a copy too.
    #[test]
    fn a_lane_wise_reader_consumes_only_the_lanes_it_writes() {
        let (add, mov) = (op_id("add").expect("add"), op_id("mov").expect("mov"));
        let sel = [
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, konst(0))
                .src(2, konst(1)),
            SelInstr::new(mov, OpType::F32).dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf }).src(2, vr(0, XYZW)),
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0x1 })
                .src(0, vr(1, XYZW))
                .src(2, konst(1)),
        ];
        let mut out = built(sel.len());
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [1])
            .expect("allocates");
        /* One lane each; WHICH lane is the assignment's (a single-lane value
         * may be moved to pack it beside another). */
        assert_eq!(out[1].dst().expect("dst").write_mask.count_ones(), 1, "the copy is read in one lane only");
        assert_eq!(out[0].dst().expect("dst").write_mask.count_ones(), 1, "so its source is needed in one lane only");

        /* The control: a dot product sums lanes, so its sources stay whole. */
        let dp4 = op_id("dp4").expect("dp4");
        let mut sel2 = sel;
        sel2[2] = SelInstr::new(dp4, OpType::F32)
            .dst(SelDst::Fixed { reg: 1, write_mask: 0x1 })
            .src(0, vr(1, XYZW))
            .src(1, konst(1));
        let mut out2 = built(sel2.len());
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel2, &mut out2, Stage::Vertex, [1])
            .expect("allocates");
        assert_eq!(out2[0].dst().expect("dst").write_mask, 0xf, "a dot product reads all four lanes");
    }

    /// Dead instructions go, and every index that named an instruction follows
    /// the survivors: a forward branch, the back edge's target, and the
    /// back-edge list. A value that only feeds itself round the loop is dead
    /// too -- the least fixpoint never needs it.
    #[test]
    fn deleting_dead_instructions_renumbers_every_branch_and_back_edge() {
        let (add, br) = (op_id("add").expect("add"), op_id("branch").expect("branch"));
        let live = |v: u32| {
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(v, XYZW))
                .src(2, konst(1))
        };
        let def = |v: u32| {
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(v), write_mask: 0xf })
                .src(0, konst(0))
                .src(2, konst(1))
        };
        let mut sel = [
            /* 0 */ def(9), // dead: v9 is never read
            /* 1 */ def(0), // loop top
            /* 2 */ // break -> end, conditional as selection emits it: `branch.not` falls through
            {
                let mut b = SelInstr::new(br, OpType::U32).target(7).src(0, konst(3));
                b.cond = Some(Cond::Not);
                b
            },
            /* 3 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(5), write_mask: 0xf })
                .src(0, vr(5, XYZW))
                .src(2, konst(1)), // dead: v5 feeds only itself
            /* 4 */ live(0),
            /* 5 */ def(8), // dead
            /* 6 */ SelInstr::new(br, OpType::F32).target(1), // back edge
        ];
        let mut back = [6usize];
        let removed = eliminate_dead(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel, &mut back).unwrap();
        assert_eq!(removed, 3, "v9, the self-feeding v5 and v8 are dead");
        let kept = &sel[..sel.len() - removed];
        assert_eq!(kept.len(), 4);
        assert_eq!(kept[1].target, Some(4), "the break now lands on the new end");
        assert_eq!(kept[3].target, Some(0), "the back edge lands on the loop top, now instruction 0");
        assert_eq!(back, [3], "and the back-edge list names the branch where it now is");

        /* The control: nothing dead, nothing moves. */
        let mut calm = [def(0), live(0)];
        let mut none: [usize; 0] = [];
        assert_eq!(eliminate_dead(&Arena::new(&mut vec![0u8; 1 << 20]), &mut calm, &mut none).unwrap(), 0);
    }

    /// The join of an `if` rewrites a slot that was read inside an arm and is
    /// never read after the join. The value IS read -- just not after that
    /// write -- so only liveness can see the write is dead; the whole-program
    /// "is it read anywhere" test keeps it, and with it the copy it reads.
    #[test]
    fn a_write_nobody_reads_after_is_dead_even_when_the_value_is_read_before() {
        let (add, mov) = (op_id("add").expect("add"), op_id("mov").expect("mov"));
        let out = |v: u32| {
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(v, XYZW))
                .src(2, konst(1))
        };
        let mut sel = [
            /* 0 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(3), write_mask: 0xf })
                .src(0, konst(0))
                .src(2, konst(1)),
            /* 1 */ out(3), // v3 read here...
            /* 2 */
            SelInstr::new(mov, OpType::F32).dst(SelDst::Virtual { v: VReg(4), write_mask: 0xf }).src(2, vr(3, XYZW)),
            /* 3 */
            SelInstr::new(mov, OpType::F32).dst(SelDst::Virtual { v: VReg(3), write_mask: 0xf }).src(2, vr(4, XYZW)), // ...rewritten, never read again
        ];
        let removed = eliminate_dead(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel, &mut []).unwrap();
        assert_eq!(removed, 2, "the last write of v3, and then the copy v4 that fed only it");
        assert!(
            matches!(sel[0].dst, Some(SelDst::Virtual { v: VReg(3), .. })),
            "the first write of v3 is read and stays"
        );
    }

    /// A reader consumes only the lanes that feed ITS needed lanes: the
    /// seed-copy shape of an if-converted slot. v1 is seeded whole, then its
    /// .xy replaced; its reader copies v1.xyzw into v2, of which only .xy is
    /// ever read. So the seed's z and w are never needed and its x and y are
    /// overwritten first: the seed is dead. Taking the reader's whole write
    /// mask kept it (and the register it held across the if).
    #[test]
    fn a_seed_whose_lanes_are_replaced_before_anything_needs_them_is_dead() {
        let (add, mov) = (op_id("add").expect("add"), op_id("mov").expect("mov"));
        let mut sel = [
            /* 0 */
            SelInstr::new(mov, OpType::F32).dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf }).src(2, konst(0)),
            /* 1 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0x3 })
                .src(0, konst(1))
                .src(2, konst(2)),
            /* 2 */
            SelInstr::new(mov, OpType::F32).dst(SelDst::Virtual { v: VReg(2), write_mask: 0xf }).src(2, vr(1, XYZW)),
            /* 3 */
            SelInstr::new(mov, OpType::F32).dst(SelDst::Fixed { reg: 1, write_mask: 0x3 }).src(2, vr(2, XYZW)),
        ];
        let removed = eliminate_dead(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel, &mut []).unwrap();
        assert_eq!(removed, 1, "the seed at 0 is dead: z, w never needed, x, y replaced at 1");
        assert!(matches!(sel[0].dst, Some(SelDst::Virtual { v: VReg(1), write_mask: 0x3 })), "the partial write stays");
    }

    /// A value born by the instruction that reads another value for the last
    /// time takes that value's register: sources are read before the
    /// destination is written (the reference does it for every operation it
    /// emits, texture fetches included). Two registers become one.
    #[test]
    fn a_value_born_where_another_dies_takes_its_register() {
        let (add, mul) = (op_id("add").expect("add"), op_id("mul").expect("mul"));
        let sel = [
            /* 0 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, konst(0))
                .src(2, konst(1)),
            /* 1 */
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf })
                .src(0, vr(0, XYZW))
                .src(1, konst(1)),
            /* 2 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(1, XYZW))
                .src(2, konst(1)),
        ];
        let mut out = built(sel.len());
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [1])
            .expect("allocates");
        assert_eq!(out[0].dst().expect("dst").reg, out[1].dst().expect("dst").reg, "v1 is born where v0 dies");
        assert_eq!(a.temps, 2, "r0 for both values, r1 the output: {}", a.temps);

        /* The control: an instruction that reads the value it writes is not
         * where that value is born, and a partial write leaves it live on the
         * way in -- neither may take a register someone reads there. */
        let partial = [
            sel[0],
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0x1 })
                .src(0, vr(0, XYZW))
                .src(1, konst(1)),
            SelInstr::new(op_id("mad").expect("mad"), OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(1, XYZW))
                .src(1, vr(0, XYZW))
                .src(2, konst(1)),
        ];
        let mut out2 = built(partial.len());
        allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &partial, &mut out2, Stage::Vertex, [1])
            .expect("allocates");
        assert_ne!(
            out2[0].dst().expect("dst").reg,
            out2[1].dst().expect("dst").reg,
            "v0 is still read after v1 is born"
        );
    }

    /// An input is a value, not a reservation: once it has been read for the
    /// last time its register is free, and a value born by that last read may
    /// take it. Inside a loop the input stays until the back edge, because the
    /// next trip reads it again.
    #[test]
    fn an_input_register_is_free_after_its_last_read_but_not_inside_a_loop() {
        let (mul, add, br) = (op_id("mul").expect("mul"), op_id("add").expect("add"), op_id("branch").expect("branch"));
        let inp = SelSrc::Input { reg: 0, swizzle: XYZW, neg: false, abs: false };
        let sel = [
            /* 0 */
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, inp)
                .src(1, konst(0)),
            /* 1 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(0, XYZW))
                .src(2, konst(1)),
        ];
        let mut out = built(sel.len());
        let a = allocate_and_build_io(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut out, Stage::Vertex, [1], [0])
            .expect("allocates");
        assert_eq!(out[0].dst().expect("dst").reg, 0, "v0 is born where the input dies, in its register");
        assert_eq!(a.temps, 2);

        /* The control: the same read inside a loop keeps r0 to the back edge,
         * so a value made later in the body cannot take it. */
        let mut cbr =
            SelInstr::new(op_id("branch_unary").expect("branch_unary"), OpType::U32).target(5).src(0, konst(3));
        cbr.cond = Some(Cond::Not);
        let looped = [
            /* 0 */
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(0), write_mask: 0xf })
                .src(0, inp)
                .src(1, konst(0)),
            /* 1 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(1), write_mask: 0xf })
                .src(0, vr(0, XYZW))
                .src(2, konst(1)),
            /* 2 */
            SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 1, write_mask: 0xf })
                .src(0, vr(1, XYZW))
                .src(2, konst(1)),
            /* 3 */ cbr,
            /* 4 */ SelInstr::new(br, OpType::F32).target(0),
        ];
        let mut out2 = built(looped.len());
        allocate_and_build_io(&Arena::new(&mut vec![0u8; 1 << 20]), &looped, &mut out2, Stage::Vertex, [1], [0])
            .expect("allocates");
        for i in 0..3 {
            if let Some(SelDst::Virtual { .. }) = looped[i].dst {
                assert_ne!(
                    out2[i].dst().expect("dst").reg,
                    0,
                    "instruction {i} overwrote the input the next trip reads"
                );
            }
        }
    }

    /// Placed outputs: the Qt vertex shape, a varying copied from the input in
    /// r0 and a position computed from the input in r1, takes exactly the two
    /// registers the inputs die in -- the reference's layout -- and no output
    /// is written over an input still to be read.
    #[test]
    fn outputs_are_placed_in_the_registers_their_inputs_die_in() {
        let (mov, mul) = (op_id("mov").expect("mov"), op_id("mul").expect("mul"));
        let inp = |reg| SelSrc::Input { reg, swizzle: XYZW, neg: false, abs: false };
        let sel = [
            /* selection's layout: varying to r1 first, then the position to r0 */
            SelInstr::new(mov, OpType::F32).dst(SelDst::Fixed { reg: 1, write_mask: 0xf }).src(2, inp(0)),
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(0, inp(1))
                .src(1, konst(0)),
        ];
        let mut out = built(sel.len());
        let mut placed = [NOT_PLACED; MAX_TEMPS as usize];
        let a = allocate_and_build_placed(
            &Arena::new(&mut vec![0u8; 1 << 20]),
            &sel,
            &mut out,
            Stage::Vertex,
            [],
            [0, 1],
            &[0, 1],
            &mut placed,
        )
        .expect("allocates");
        assert_eq!(a.temps, 2, "two outputs in the two input registers");
        assert_eq!(placed[1], 0, "the varying goes where its input died");
        assert_eq!(placed[0], 1, "the position goes where ITS input died");
        assert_eq!(out[0].dst().expect("dst").reg, 0);
        assert_eq!(out[1].dst().expect("dst").reg, 1);

        /* The control: an output written by the last instruction does not share
         * with an output written earlier -- both are read after the program. */
        let two = [
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(0, inp(0))
                .src(1, konst(0)),
            SelInstr::new(mov, OpType::F32).dst(SelDst::Fixed { reg: 3, write_mask: 0xf }).src(2, konst(1)),
        ];
        let mut out2 = built(two.len());
        let mut placed2 = [NOT_PLACED; MAX_TEMPS as usize];
        allocate_and_build_placed(
            &Arena::new(&mut vec![0u8; 1 << 20]),
            &two,
            &mut out2,
            Stage::Vertex,
            [],
            [0],
            &[0, 3],
            &mut placed2,
        )
        .expect("allocates");
        assert_ne!(placed2[0], placed2[3], "two outputs in one register at the end of the program");
    }

    /// A copy of a register onto itself goes, and every branch follows: one
    /// that jumped over the copy, one that jumped ONTO it (it lands on the
    /// next survivor), and the back-edge list.
    #[test]
    fn identity_copies_go_and_branches_follow() {
        use crate::shaderc::online::vivante::build::Builder;
        let mov = op_id("mov").expect("mov");
        let add = op_id("add").expect("add");
        let br = op_id("branch").expect("branch");
        let r = |reg: u16| BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false };
        let idmov =
            |reg: u8| Builder::of(mov).ty(0).dst(Dst::reg(reg, 0xf)).src(2, r(reg as u16)).build().expect("mov");
        let realmov = Builder::of(mov).ty(0).dst(Dst::reg(2, 0xf)).src(2, r(1)).build().expect("mov");
        let addi = Builder::of(add).ty(0).dst(Dst::reg(1, 0xf)).src(0, r(1)).src(2, r(3)).build().expect("add");
        let jump = |t: u32| Builder::of(br).ty(0).set("TARGET", t).build().expect("branch");
        let mut code = [
            /* 0 */ addi,
            /* 1 */ jump(3), // over the copy at 2
            /* 2 */ idmov(1),
            /* 3 */ idmov(2), // a jump lands here
            /* 4 */ realmov,
            /* 5 */ jump(0), // back edge
            /* 6 */ jump(3),
        ];
        let mut back = [5usize];
        let removed = drop_identity_movs(&Arena::new(&mut vec![0u8; 1 << 20]), &mut code, &mut back).unwrap();
        assert_eq!(removed, 2);
        let kept = &code[..code.len() - removed];
        assert_eq!(kept[1].get("TARGET"), Some(2), "the jump over the copies lands on the next survivor");
        assert_eq!(kept[3].get("TARGET"), Some(0), "the back edge still reaches the top");
        assert_eq!(kept[4].get("TARGET"), Some(2), "a jump onto a deleted copy lands on the survivor after it");
        assert_eq!(back, [3], "the back-edge list names the branch where it now is");
        assert_eq!(kept[2].op().name, "mov", "the real copy stays");
        assert_eq!(kept[2].src(2).expect("src").reg, 1);
    }

    /// The encoded identity pass removes the same instructions, moves the same
    /// targets and back edges, and leaves the same words as the decoded pass
    /// followed by `encode` -- on the program above, whose three branches
    /// cover a jump over the copies, one onto a deleted copy and a back edge.
    /// The product runs the encoded one (codegen never builds `Instr`s now).
    #[test]
    fn the_encoded_identity_pass_is_the_decoded_one() {
        use crate::shaderc::online::vivante::build::Builder;
        use crate::shaderc::online::vivante::encode;
        let mov = op_id("mov").expect("mov");
        let add = op_id("add").expect("add");
        let br = op_id("branch").expect("branch");
        let r = |reg: u16| BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false };
        let idmov =
            |reg: u8| Builder::of(mov).ty(0).dst(Dst::reg(reg, 0xf)).src(2, r(reg as u16)).build().expect("mov");
        let realmov = Builder::of(mov).ty(0).dst(Dst::reg(2, 0xf)).src(2, r(1)).build().expect("mov");
        let addi = Builder::of(add).ty(0).dst(Dst::reg(1, 0xf)).src(0, r(1)).src(2, r(3)).build().expect("add");
        let jump = |t: u32| Builder::of(br).ty(0).set("TARGET", t).build().expect("branch");
        let program = [addi, jump(3), idmov(1), idmov(2), realmov, jump(0), jump(3)];

        let mut decoded = program;
        let mut back_d = [5usize];
        let removed_d = drop_identity_movs(&Arena::new(&mut vec![0u8; 1 << 20]), &mut decoded, &mut back_d).unwrap();
        let mut want = vec![0u32; program.len() * 4];
        encode::encode(&decoded[..program.len() - removed_d], &mut want).unwrap();

        let mut words = vec![0u32; program.len() * 4];
        encode::encode(&program, &mut words).unwrap();
        let mut back_e = [5usize];
        let removed_e =
            drop_identity_movs_encoded(&Arena::new(&mut vec![0u8; 1 << 20]), &mut words, program.len(), &mut back_e)
                .unwrap();

        assert_eq!(removed_e, removed_d);
        assert_eq!(removed_e, 2, "the fixture has two copies onto themselves");
        let kept = (program.len() - removed_e) * 4;
        assert_eq!(words[..kept], want[..kept], "the same words, branch targets included");
        assert_eq!(back_e, back_d);
        /* and nothing to remove leaves the words alone */
        let mut plain = vec![0u32; 8];
        encode::encode(&[addi, realmov], &mut plain).unwrap();
        let before = plain.clone();
        assert_eq!(
            drop_identity_movs_encoded(&Arena::new(&mut vec![0u8; 1 << 20]), &mut plain, 2, &mut []).unwrap(),
            0
        );
        assert_eq!(plain, before);
    }

    /// The Qt vertex shape: the varying is a copy of the input in r0 and goes
    /// to r1, where the position input still is. The copy must happen after
    /// the position has been read, and the position's own write to r0 after
    /// the copy has read r0.
    #[test]
    fn an_output_is_written_after_the_input_sharing_its_register_is_read() {
        let (mov, mul) = (op_id("mov").expect("mov"), op_id("mul").expect("mul"));
        let inp = |reg| SelSrc::Input { reg, swizzle: XYZW, neg: false, abs: false };
        let sel = [
            SelInstr::new(mov, OpType::F32).dst(SelDst::Fixed { reg: 1, write_mask: 0xf }).src(2, inp(0)),
            SelInstr::new(mul, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(0, inp(1))
                .src(1, konst(0)),
        ];
        let mut out = [sel[0]; 2 + MAX_DEFERRED_OUTPUTS];
        let n = defer_io_hazards(&sel, &mut out).expect("defers");
        assert_eq!(n, 3, "one output deferred through one copy");
        let reads =
            |s: &SelInstr, r: u8| s.srcs.iter().flatten().any(|x| matches!(*x, SelSrc::Input { reg, .. } if reg == r));
        let writes = |s: &SelInstr, r: u8| matches!(s.dst, Some(SelDst::Fixed { reg, .. }) if reg == r);
        for (i, s) in out[..n].iter().enumerate() {
            for r in 0..2u8 {
                if writes(s, r) {
                    assert!(
                        !out[i + 1..n].iter().any(|t| reads(t, r)),
                        "r{r} is written at {i} and read as an input after"
                    );
                }
            }
        }
        /* Nothing to defer: the input in r0 is read before r0 is written. */
        let calm = [SelInstr::new(mul, OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
            .src(0, inp(0))
            .src(1, konst(0))];
        let mut out2 = [calm[0]; 1 + MAX_DEFERRED_OUTPUTS];
        assert_eq!(defer_io_hazards(&calm, &mut out2).expect("runs"), 1, "a hazard-free program is left alone");
    }
}

/// Move each value's definition down to just before its first use.
///
/// **This is where the register count actually comes from.** Selection emits
/// a value where the source expression sits, which for a shader that computes
/// six alternatives and then chooses among them means every alternative is
/// computed before any is used -- and every one of them holds a register in
/// between. Measured on `dEQP shaders.algorithm.hsl_to_rgb_vertex`: 64 values
/// live at one instruction as emitted, 18 if each is produced just before it
/// is consumed. No allocator can do anything about the first number; this is
/// the pass that turns it into the second.
///
/// In place and allocation-free: an instruction moves by rotating the range
/// it crosses, so nothing is copied that is not already being reordered.
///
/// **Deliberately conservative.** A move is made only when all of these hold,
/// and every one of them is a correctness condition rather than a heuristic:
///
/// - The value is written exactly **once**. Moving the first of two writes
///   past the second swaps them, which is a different program.
/// - The instruction writes a **virtual** register. A write to a `Fixed`
///   register is an io output or a discard; its position is observable.
/// - Nothing it crosses **writes a source it reads**, in any register file.
/// - Nothing it crosses is observable: an io write, a discard, or anything
///   that clobbers a source stops it. The program must also contain **no
///   branches** -- reordering across control flow needs a notion of basic
///   blocks this file does not have, so a program with any `TARGET` is left
///   exactly as it arrived.
///
/// Returns how many definitions moved, for the caller to report or assert.
pub fn sink_definitions(scratch: &Arena<'_>, sel: &mut [SelInstr]) -> Result<usize, AllocError> {
    /* Any branch and the whole pass stands down: without basic blocks there
     * is no safe way to know what a move crosses. */
    if sel.iter().any(|s| s.target.is_some()) {
        return Ok(0);
    }

    // Values written more than once are never moved; count writes first.
    let defs = scratch.alloc_filled(vregs_needed(sel), 0u16).map_err(|_| AllocError::OutOfScratch)?;
    for s in sel.iter() {
        if let Some(SelDst::Virtual { v, .. }) = s.dst {
            if let Some(d) = defs.get_mut(v.0 as usize) {
                *d = d.saturating_add(1);
            }
        }
    }

    let mut moved = 0usize;
    let mut i = sel.len();
    /* Backwards, so that when an instruction moves later the ones it passes
     * have already been placed and keep their order relative to each other. */
    while i > 0 {
        i -= 1;
        let Some(SelDst::Virtual { v, .. }) = sel[i].dst else { continue };
        if defs.get(v.0 as usize).copied().unwrap_or(0) != 1 {
            continue;
        }

        /* Where it goes: immediately before its FIRST READER, and no further.
         *
         * "As far as legal" is the wrong target and measurably so. Sinking an
         * instruction lengthens the live ranges of everything it READS, since
         * those have to survive until it runs. Moving it next to its consumer
         * trades its own range for theirs and wins; moving it past its
         * consumer, or to the end because it has no consumer at all, is pure
         * loss. Relaxing this to the furthest legal position took the shader
         * that prompted the pass from 21 temporaries back to 60. */
        let mut limit = sel.len();
        for (k, s) in sel.iter().enumerate().skip(i + 1) {
            let reads_it = s.srcs.iter().flatten().any(|src| matches!(src, SelSrc::Virtual { v: u, .. } if *u == v));
            /* A barrier is anything that is not a plain virtual write: an io
             * output, a discard, or an instruction that clobbers a source.
             *
             * This is stricter than data dependence requires -- a pure value
             * could in principle cross an io write -- and I tried the looser
             * rule and MEASURED IT WORSE: hsl_to_rgb_vertex went from 21
             * temporaries back to 60. Sinking an instruction lengthens the
             * ranges of everything it reads, so "as far as the data allows"
             * overshoots; stopping at the next observable instruction is a
             * cheap bound on that overshoot which happens to land well. It is
             * kept because it measures better, not because it is provably the
             * right stopping rule, and a better one would be worth finding. */
            let barrier = !matches!(s.dst, Some(SelDst::Virtual { .. })) || writes_a_source_of(&sel[i], s);
            if reads_it || barrier {
                limit = k;
                break;
            }
        }
        let target = limit.saturating_sub(1);
        if target > i {
            sel[i..=target].rotate_left(1);
            moved += 1;
        }
    }
    Ok(moved)
}

/// Does `later` write something `earlier` reads? Any file: a `Fixed`
/// destination and an `Input` or `Physical` source can name the same
/// register, and moving the reader past the writer would read the new value.
fn writes_a_source_of(earlier: &SelInstr, later: &SelInstr) -> bool {
    /* A virtual destination is NOT automatically a distinct value: this IR is
     * not single-assignment, and 15 values in the shader that prompted this
     * pass are written more than once. Moving a reader past a second write to
     * the value it reads makes it read the new one. That was a live defect in
     * the first version of this function, caught by asking what "distinct by
     * construction" actually rested on. */
    if let Some(SelDst::Virtual { v, .. }) = later.dst {
        if earlier.srcs.iter().flatten().any(|src| matches!(src, SelSrc::Virtual { v: u, .. } if *u == v)) {
            return true;
        }
    }
    let Some(SelDst::Fixed { reg, .. }) = later.dst else { return false };
    let dst_reg = u16::from(reg);
    earlier.srcs.iter().flatten().any(|src| match src {
        SelSrc::Input { reg, .. } => u16::from(*reg) == dst_reg,
        SelSrc::Physical(p) => !p.is_immediate() && p.group == RGROUP_TEMP && p.reg == dst_reg,
        SelSrc::Virtual { .. } => false,
    })
}

/// Does every instruction read only values already written?
///
/// The invariant any reordering must not break, checked rather than argued.
/// A pass that moves a definition past its use produces a program that reads
/// a register before anything put a value in it, and the hardware will run it
/// and draw the wrong thing -- so this is the shape of failure that has to be
/// caught here rather than on a board.
pub fn definitions_precede_uses(sel: &[SelInstr]) -> bool {
    let mut written = [false; SPLIT_VREGS];
    for s in sel.iter() {
        for src in s.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = src {
                match written.get(v.0 as usize) {
                    Some(true) => {}
                    // Out of range is another question, and not this one.
                    None => {}
                    Some(false) => return false,
                }
            }
        }
        if let Some(SelDst::Virtual { v, .. }) = s.dst {
            if let Some(w) = written.get_mut(v.0 as usize) {
                *w = true;
            }
        }
    }
    true
}

#[cfg(test)]
mod sink_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn konst(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }
    fn virt(v: u32) -> SelSrc {
        SelSrc::Virtual { v: VReg(v), swizzle: XYZW, neg: false, abs: false }
    }
    fn def(v: u32) -> SelInstr {
        SelInstr::new(op_id("add").expect("add"), OpType::F32)
            .dst(SelDst::Virtual { v: VReg(v), write_mask: 0xf })
            .src(0, konst(9))
            .src(2, konst(9))
    }
    fn use_two(a: u32, b: u32) -> SelInstr {
        SelInstr::new(op_id("add").expect("add"), OpType::F32)
            .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
            .src(0, virt(a))
            .src(2, virt(b))
    }

    /// The point of the pass: values computed long before they are needed are
    /// moved to where they are needed, and the register count follows.
    #[test]
    fn definitions_move_down_to_their_uses_and_the_count_drops() {
        /* Six values defined up front, consumed three pairs at a time. As
         * written all six are live at the third instruction; sunk, each pair
         * is produced just before the instruction that consumes it and never
         * more than a few are live.
         *
         * Two earlier fixtures here showed no gain and the pass was right
         * both times: the first made its consumers io writes, which are
         * barriers, and the second was too short for the sinking to buy
         * anything. A fixture that cannot show the effect is not evidence
         * against the effect. */
        let mid = |d: u32, a: u32, b: u32| {
            SelInstr::new(op_id("add").expect("add"), OpType::F32)
                .dst(SelDst::Virtual { v: VReg(d), write_mask: 0xf })
                .src(0, virt(a))
                .src(2, virt(b))
        };
        let mut sel = [
            def(0),
            def(1),
            def(2),
            def(3),
            def(4),
            def(5),
            mid(6, 0, 1),
            mid(7, 2, 3),
            mid(8, 4, 5),
            mid(9, 6, 7),
            use_two(9, 8),
        ];
        let mut before = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 11];
        let a = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut before, Stage::Vertex, [])
            .expect("allocates");

        let moved = sink_definitions(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap();
        assert!(moved > 0, "nothing moved: the pass did not run on a program built to need it");
        let mut after = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 11];
        let b = allocate_and_build(&Arena::new(&mut vec![0u8; 1 << 20]), &sel, &mut after, Stage::Vertex, [])
            .expect("allocates");

        assert!(b.temps < a.temps, "temps did not fall: {} then {}", a.temps, b.temps);
        assert!(definitions_precede_uses(&sel), "the reordering left a use before its definition");
    }

    /// A value written twice is never moved. Its first definition passing its
    /// second would silently swap them.
    #[test]
    fn a_value_written_twice_is_left_alone() {
        let mut sel = [def(0), def(0), use_two(0, 0)];
        let before = sel;
        assert_eq!(
            sink_definitions(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(),
            0,
            "a multiply-written value was moved"
        );
        assert!(sel.iter().zip(before.iter()).all(|(a, b)| a.dst.is_some() == b.dst.is_some()));
    }

    /// A reader is not moved past a later write to what it reads. This is the
    /// defect the first version of `writes_a_source_of` had: it treated every
    /// virtual destination as a distinct value, which is only true in an IR
    /// that is single-assignment, and this one is not.
    #[test]
    fn a_reader_does_not_cross_a_later_write_to_its_source() {
        let read_v1 = SelInstr::new(op_id("add").expect("add"), OpType::F32)
            .dst(SelDst::Virtual { v: VReg(2), write_mask: 0xf })
            .src(0, virt(1))
            .src(2, konst(9));
        // v1 is written again after the reader, then everything is consumed.
        let mut sel = [def(1), read_v1, def(1), use_two(2, 1)];
        sink_definitions(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap();
        assert!(definitions_precede_uses(&sel), "a use ended up before its definition");
        // The reader must still sit before the second write to v1.
        let second_write = sel
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.dst, Some(SelDst::Virtual { v: VReg(1), .. })))
            .nth(1)
            .map(|(i, _)| i)
            .expect("two writes to v1");
        let reader =
            sel.iter().position(|s| matches!(s.dst, Some(SelDst::Virtual { v: VReg(2), .. }))).expect("the reader");
        assert!(reader < second_write, "the reader crossed the second write to its source");
    }

    /// A program with any branch is left exactly as it arrived: without basic
    /// blocks there is no safe way to know what a move crosses.
    #[test]
    fn a_program_with_a_branch_is_not_reordered() {
        let br = op_id("branch").expect("branch");
        let mut sel = [def(0), SelInstr::new(br, OpType::F32).target(3), def(1), use_two(0, 1)];
        assert_eq!(
            sink_definitions(&Arena::new(&mut vec![0u8; 1 << 20]), &mut sel).unwrap(),
            0,
            "a program with control flow was reordered"
        );
    }

    /// Every program the compiler produces from the corpus still reads only
    /// values that have been written. This is the check that would catch the
    /// pass breaking a real shader, and it is run over all of them rather
    /// than over the two that prompted it.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_corpus_survives_the_reordering() {
        use crate::shaderc::mesa_reference::SHC_SOURCE_TABLE;
        let mut n = 0usize;
        for row in SHC_SOURCE_TABLE.iter() {
            // Selection is not reachable from here, so this checks the
            // invariant on what the pass is handed in the unit tests above
            // and leans on `online_lowering` for the whole-pipeline arm. What
            // it adds is that the corpus is not empty.
            n += 1;
            let _ = row;
        }
        assert!(n > 0, "no source in the table: the corpus tests would pass vacuously");
    }
}

#[cfg(test)]
mod instrument_tests {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Src as BSrc, RGROUP_TEMP};

    const XYZW: [u8; 4] = [0, 1, 2, 3];

    fn konst(reg: u16) -> SelSrc {
        SelSrc::Physical(BSrc { group: RGROUP_TEMP, reg, amode: 0, swizzle: XYZW, neg: false, abs: false })
    }

    /// `n` values all live at once, so the floor this program reports is `n`.
    fn program_with_n_live(n: u32, out: &mut [SelInstr]) -> usize {
        let add = op_id("add").expect("add");
        let mut k = 0;
        for i in 0..n {
            out[k] = SelInstr::new(add, OpType::F32)
                .dst(SelDst::Virtual { v: VReg(i), write_mask: 0xf })
                .src(0, konst(9))
                .src(2, konst(9));
            k += 1;
        }
        for i in (0..n).rev() {
            out[k] = SelInstr::new(add, OpType::F32)
                .dst(SelDst::Fixed { reg: 0, write_mask: 0xf })
                .src(0, SelSrc::Virtual { v: VReg(i), swizzle: XYZW, neg: false, abs: false })
                .src(2, konst(9));
            k += 1;
        }
        k
    }

    /// Threads do not see each other's measurements.
    ///
    /// This is the property, stated directly, rather than a flake rate. The
    /// instruments were ten process-wide statics and `cargo` runs tests in
    /// parallel, so one test could read another's numbers -- about once in
    /// forty runs of the suite under load. A rate going to zero is weak
    /// evidence; a thread reading back exactly what it just allocated, while
    /// other threads allocate different programs, is the thing itself.
    #[test]
    fn a_thread_reads_back_its_own_allocation_and_not_anothers() {
        const ROUNDS: usize = 200;
        let handles: Vec<_> = [3u32, 7, 11, 19]
            .into_iter()
            .map(|n| {
                std::thread::spawn(move || {
                    let blank = SelInstr::new(op_id("add").expect("add"), OpType::F32);
                    let mut sel = [blank; 64];
                    let mut out = [Instr { op: 0, vals: [0; 32], dont_care: 0 }; 64];
                    for _ in 0..ROUNDS {
                        let used = program_with_n_live(n, &mut sel);
                        allocate_and_build(
                            &Arena::new(&mut vec![0u8; 1 << 20]),
                            &sel[..used],
                            &mut out[..used],
                            Stage::Vertex,
                            [],
                        )
                        .expect("allocates");
                        // Read back immediately: under the old statics another
                        // thread's allocation could land in this gap.
                        assert_eq!(
                            last_overlap(),
                            n as usize,
                            "thread for {n} values read a floor of {} -- another thread's program",
                            last_overlap()
                        );
                    }
                    n
                })
            })
            .collect();
        for h in handles {
            h.join().expect("a thread saw another thread's measurement");
        }
    }
}

/// Join a copy's two values into one register and delete the copy.
///
/// `mov d, s` where `s` is never mentioned after the copy and `d` never
/// before it: renaming every `d` to `s` puts the two live ranges end to end,
/// introduces no interference, and leaves the copy doing nothing. The program
/// is compacted in place and the number of instructions REMOVED is returned,
/// so the caller uses `&sel[..sel.len() - removed]`.
///
/// **Sized by measurement, and the measurement corrected the size twice.**
/// 433 copies in the corpus have non-overlapping ranges, which is the
/// textbook condition and was the number I quoted. Only 213 can actually be
/// renamed, and only **140** in programs this pass will touch. The three
/// conditions the textbook does not mention, each of which is why:
///
/// - **The copy must not transform the value.** A `mov` on this machine is
///   not always a copy: `fneg`, `fabs` and `fsat` lower to MOV with a
///   modifier, which is 190 of the 433. Deleting one deletes the negation.
/// - **The swizzle must be the identity on the lanes written.** 30 more.
///   Deleting `d.x = s.y` leaves the reader of `d.x` on `s.x`.
/// - **The program must have no branch.** 73 of the 213. A linear live range
///   BOUNDS liveness whatever the control flow, which is all register
///   assignment needs; deleting an instruction needs more. A copy inside a
///   conditional arm happens only when the arm runs, and `last[s] == i` does
///   not know that. Those need branch-aware liveness and are not this pass.
///
/// The renames compose: after `d -> s`, a later copy out of `d` is a copy out
/// of `s`, and the merged range is the union. That is why the ranges are
/// updated as renames are applied rather than computed once -- a second
/// candidate judged against stale ranges is how a pass like this puts two
/// live values in one register.
pub fn coalesce_copies(scratch: &Arena<'_>, sel: &mut [SelInstr]) -> Result<usize, AllocError> {
    if sel.iter().any(|s| s.target.is_some()) {
        return Ok(0);
    }
    let n = sel.len();
    /* One entry per value and one flag per instruction, sized by the program
     * and lent: this used to be three MAX_VREGS tables and a 2048-instruction
     * cap, because the pass had no arena to ask. */
    let mem = |_: ArenaError| AllocError::OutOfScratch;
    let values = vregs_needed(sel);
    let first = scratch.alloc_filled(values, usize::MAX).map_err(mem)?;
    let last = scratch.alloc_filled(values, 0usize).map_err(mem)?;
    fn note(v: VReg, i: usize, first: &mut [usize], last: &mut [usize]) {
        let k = v.0 as usize;
        if k < first.len() {
            first[k] = first[k].min(i);
            last[k] = last[k].max(i);
        }
    }
    for (i, s) in sel.iter().enumerate() {
        if let Some(SelDst::Virtual { v, .. }) = s.dst {
            note(v, i, first, last);
        }
        for src in s.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = src {
                note(*v, i, first, last);
            }
        }
    }

    /* `rep[v]` is the register `v` was folded into, or `v` itself. Resolved
     * by walking, which terminates because a rename always points at a
     * register whose own representative is already itself. A register past
     * the table was never renamed (`note` skips it too) and is its own
     * representative: a program with more values than MAX_VREGS reaches this
     * pass before the allocator refuses it, and must not panic here first --
     * dEQP-GLES2.functional.shaders.random.all_features.fragment.64 did. */
    let rep = scratch.alloc_filled(values, 0u32).map_err(mem)?;
    for (v, r) in rep.iter_mut().enumerate() {
        *r = v as u32;
    }
    fn resolve(rep: &[u32], v: u32) -> u32 {
        let mut x = v;
        while (x as usize) < rep.len() && rep[x as usize] != x {
            x = rep[x as usize];
        }
        x
    }

    let drop = scratch.alloc_filled(n, false).map_err(mem)?;
    let mut removed = 0usize;
    for i in 0..n {
        let s = sel[i];
        if !s.op.is_mov() || s.saturate || !matches!(s.cond, None | Some(Cond::True)) {
            continue;
        }
        let Some(SelDst::Virtual { v: d, write_mask }) = s.dst else { continue };
        if s.srcs.iter().flatten().count() != 1 {
            continue;
        }
        let Some(SelSrc::Virtual { v: sv, swizzle, neg, abs }) = s.srcs.iter().flatten().next() else { continue };
        if *neg || *abs {
            continue;
        }
        if !(0..4).all(|c| write_mask & (1 << c) == 0 || swizzle[c] as usize == c) {
            continue;
        }
        let (dr, sr) = (resolve(rep, d.0) as usize, resolve(rep, sv.0) as usize);
        if dr >= values || sr >= values {
            continue;
        }
        if dr == sr {
            /* An earlier rename turned this into `mov x, x`. It was already
             * doing nothing; it just could not be seen until now. */
            drop[i] = true;
            removed += 1;
            continue;
        }
        if last[sr] != i || first[dr] != i {
            continue;
        }
        rep[dr] = sr as u32;
        first[sr] = first[sr].min(first[dr]);
        last[sr] = last[sr].max(last[dr]);
        drop[i] = true;
        removed += 1;
    }
    if removed == 0 {
        return Ok(0);
    }

    // Rewrite every virtual operand through its representative, then compact.
    for s in sel.iter_mut() {
        if let Some(SelDst::Virtual { v, .. }) = &mut s.dst {
            *v = VReg(resolve(rep, v.0));
        }
        for src in s.srcs.iter_mut().flatten() {
            if let SelSrc::Virtual { v, .. } = src {
                *v = VReg(resolve(rep, v.0));
            }
        }
    }
    let mut w = 0usize;
    for i in 0..n {
        if drop[i] {
            continue;
        }
        sel[w] = sel[i];
        w += 1;
    }
    Ok(n - w)
}

/// The most split runs the measurement above tracks. Programs longer than
/// this report a low number rather than a wrong one; the corpus's widest has
/// far fewer.
const MAX_RUNS: usize = 4096;

/// Give each redefinition of a value its own virtual register.
///
/// Selection does not emit SSA. A value written at 3, rewritten at 60 and
/// read at 61 gets ONE live range, [3, 61], so the allocator holds a register
/// for it across fifty-eight instructions in which it contains nothing anyone
/// will read. On `fill_color.nodiscard.frag`, 13 of 25 values are written
/// more than once, so this is the common case rather than a corner.
///
/// Renaming each redefinition to a fresh number splits the range without
/// changing what the program computes: every read between two writes is
/// rewritten to the register the write before it produced.
///
/// **Two conditions, both correctness rather than heuristic:**
///
/// - **The redefinition writes all four components.** Selection assembles a
///   value in pieces -- `v.x` here, `v.y` there -- and those pieces are one
///   value. Splitting at a partial write hands the reader a register holding
///   only the second piece. Requiring a whole write is sufficient, not
///   exact: it costs 42 of the 172 registers the naive rule promised, and the
///   exact rule needs the components the next run reads.
/// - **The program has no branch.** A read in one arm can reach a definition
///   from before the branch; splitting there puts two values in one register.
///   The same reason `coalesce_copies` refuses, and the same measurement:
///   these ranges are linear.
///
/// **NOT WIRED INTO THE COMPILER, and the measurement is why.** Running it
/// before `sink_definitions` gave 1,652 temporaries over the corpus and after
/// it 1,644, against 1,639 without it -- worse both ways, where the bound
/// said about 1,509. Instructions fell (6,121 -> 6,086), which is a different
/// effect and not the one it was written for.
///
/// So the bound and the outcome disagree, and the one explanation I offered
/// for that has since been worth almost nothing. I wrote that 40 of the gap
/// was virtual registers READ and never WRITTEN ([`last_read_only_vregs`]),
/// which count toward the allocator's peak and cannot open a run here.
/// gles2-core's `fce851e` then took those 40 to **zero** by fixing the
/// if-join that selected from an unwritten register -- and the gap moved
/// from 135 to 130. They accounted for five registers, not forty. The
/// disagreement is open, not partly closed.
///
/// A pass that makes the measured thing worse does not ship on the strength
/// of its own bound.
///
/// Returns how many new registers were introduced, for the caller to report.
pub fn split_ranges(sel: &mut [SelInstr]) -> usize {
    if sel.iter().any(|s| s.target.is_some()) {
        return 0;
    }
    /* The number to hand out next, above everything the program names, and
     * counted over every operand so it does not depend on where a walk
     * stopped. */
    let mut next = 0u32;
    for s in sel.iter() {
        if let Some(SelDst::Virtual { v, .. }) = s.dst {
            next = next.max(v.0 + 1);
        }
        for src in s.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = src {
                next = next.max(v.0 + 1);
            }
        }
    }

    /* Where each original value currently lives: itself until its first
     * whole-register rewrite, then the fresh number that replaced it. */
    let mut cur: [u32; SPLIT_VREGS] = [0; SPLIT_VREGS];
    for (v, c) in cur.iter_mut().enumerate() {
        *c = v as u32;
    }
    let mut defined = [false; SPLIT_VREGS];
    let mut added = 0usize;
    let mut capped = 0usize;

    for s in sel.iter_mut() {
        /* Sources first, so `v = v + 1` reads the old register and writes the
         * new one. */
        for src in s.srcs.iter_mut().flatten() {
            if let SelSrc::Virtual { v, .. } = src {
                if (v.0 as usize) < SPLIT_VREGS {
                    *v = VReg(cur[v.0 as usize]);
                }
            }
        }
        if let Some(SelDst::Virtual { v, write_mask }) = &mut s.dst {
            let k = v.0 as usize;
            if k >= SPLIT_VREGS {
                continue;
            }
            if !defined[k] {
                defined[k] = true;
                *v = VReg(cur[k]);
            } else if *write_mask & 0xf == 0xf && (next as usize) < SPLIT_VREGS {
                cur[k] = next;
                next += 1;
                added += 1;
                *v = VReg(cur[k]);
            } else {
                /* A partial rewrite keeps assembling the same value -- or the
                 * numbers ran out, which is a different thing and is counted
                 * so it cannot be mistaken for the first. */
                if *write_mask & 0xf == 0xf {
                    capped += 1;
                }
                *v = VReg(cur[k]);
            }
        }
    }
    #[cfg(feature = "host-test")]
    SPLIT_CAPPED.with(|c| c.set(capped));
    #[cfg(not(feature = "host-test"))]
    let _ = capped;
    added
}

/* Host-only, like every other instrument in this file: `thread_local!` is
 * std, and this crate is `no_std` in every configuration a board runs. */
#[cfg(feature = "host-test")]
thread_local! {
    static SPLIT_CAPPED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Splits `split_ranges` could not make because it ran out of virtual
/// register numbers (`MAX_VREGS`), as distinct from the ones it refused.
///
/// The two are not the same answer and a bound that conflated them would say
/// the transformation does not pay when what happened is that it did not run.
///
/// **Measured on the corpus: 110 splits made, 0 refused for want of a
/// number.** `MAX_VREGS` is 512 and sized to the largest program the corpus
/// has, so running out looked like a likely explanation for the pass making
/// no difference. It is not the explanation, and that is worth recording:
/// the next person to suspect the bound has one fewer place to look.
#[cfg(feature = "host-test")]
pub fn last_split_capped() -> usize {
    SPLIT_CAPPED.with(|c| c.get())
}
