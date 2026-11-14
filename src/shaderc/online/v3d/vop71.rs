// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2017 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/compiler/ (vir_register_allocate.c,
// qpu_schedule.c, nir_to_vir.c, vir_to_qpu.c) and qpu/qpu_instr.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 7.1 (Raspberry Pi 5) back end's virtual operations, and the
//! interface contract of its stages.
//!
//! The 7.1 back end is new files beside the 4.2 one, which it leaves
//! unchanged: it starts from the 4.2 selection ([`super::select::Selected`],
//! whose [`super::select::VOp`]s and shader-record facts are the same for both
//! generations up to the register model) and ends at [`Instr71`] words that
//! [`super::codec71::encode71`] writes and [`super::verify71::verify71`]
//! checks. Until `docs/notes/v3d71-backend.md` says otherwise, this header is
//! the contract.
//!
//! # The pipeline
//!
//! ```text
//! Selected (4.2 VOp) --select71--> Selected71 (Op71)
//!                    --alloc71---> Allocation71 (a register per V)
//!                    --sched71_cf--> Scheduled71 (Instr71, uniforms, edges)
//!                       (calls sched71::pack71 on each straight-line run)
//!                    --chain71---> encode71, verify71, registers_fit_threads71
//! ```
//!
//! Each stage's public function (every one returns its own error type, whose
//! `NotYet(&'static str)` variant is a refusal by name, never a panic):
//!
//! * [`super::select71::select71`]`(sel: &Selected<'_>, a: &Arena<'a>) ->
//!   Result<Selected71<'a>, Select71Error>`: each 4.2 `VOp` to [`Op71`]s. The
//!   4.2 `Operand::R5` (ldvary's C term) becomes [`Src71::LdvaryC`], an SFU
//!   `Add` becomes [`Op71::Sfu`], and 4.2's magic SFU waddrs never appear.
//! * [`super::alloc71::allocate71`]`(sel: &Selected71<'_>, threads: u32, a:
//!   &Arena<'a>) -> Result<Allocation71<'a>, Alloc71Error>`: one
//!   register-file register per [`V`], no accumulators.
//! * [`super::sched71::pack71`]`(ops: &[Op71], al: &Allocation71<'_>, a:
//!   &Arena<'a>) -> Result<Packed71<'a>, Sched71Error>`: one straight-line
//!   run (no [`Op71::LoopHead`], [`Op71::Label`] or branch) into words,
//!   thread switches and their delay slots included.
//! * [`super::sched71_cf::schedule71`]`(sel: &Selected71<'_>, al:
//!   &Allocation71<'_>, a: &Arena<'a>) -> Result<Scheduled71<'a>,
//!   Sched71Error>`: the whole program: splits it at control flow, packs each
//!   run with `pack71`, places the branches and their three delay slots, and
//!   moves the uniform stream with them.
//! * [`super::chain71::compile_stage71`]`(sel: &Selected<'_>, a: &Arena<'a>)
//!   -> Result<Stage71<'a>, V3d71Error>`: the stages above for one selected
//!   shader at 4 threads, then 2, encoded and verified;
//!   [`super::chain71::compile_program71`] is the GL program, once chain.rs
//!   takes the generation as a parameter.
//!
//! # The 7.1 rules each stage follows (Mesa 26.2.1, MIT)
//!
//! Registers (alloc71):
//!
//! * No accumulators: `has_accumulators = ver < 71`
//!   (common/v3d_device_info.c:76). Every value lives in rf0..rf63 at 1 or 2
//!   threads, rf0..rf31 at 4 (`PHYS_COUNT >> threads`,
//!   vir_register_allocate.c:1066-1072), as on 4.2.
//! * rf0 is written implicitly by `ldvary`, `ldunif` and `ldunifa`
//!   (`v3d_qpu_writes_rf0_implicitly`, qpu/qpu_instr.c:1030-1040; `ldtmu` is
//!   not one of them). A value live across such a write cannot be in rf0
//!   (vir_register_allocate.c:1167-1177), nor can an `ldvary`'s own
//!   destination (:1314-1322), nor a value live from the last `ldvary` to an
//!   explicit rf0 read (:1245-1261).
//! * An `ldunif` whose destination is rf0 stays `ldunif`; any other
//!   destination makes it `ldunifrf` with the address in the COND field
//!   (vir_to_qpu.c:328-350), which then cannot merge with a conditional
//!   operation. So a uniform's value tries rf0 first, unless it is within 4
//!   instructions of an `ldvary` in a fragment shader (`try_rf0`,
//!   vir_register_allocate.c:1323-1336, 965-975), and rf0 is otherwise the
//!   last choice (:993-1013).
//! * The fragment payload at 7.1 is rf1 (centroid W), rf2 (Z) and rf3 (W),
//!   not rf0..rf2 (nir_to_vir.c:4851-4858; `reg_is_payload`,
//!   vir_register_allocate.c:1104-1124): [`Src71::Payload`] names those.
//! * The last three instructions avoid rf0..rf3 (`safe_rf_start` 4 at 7.1,
//!   3 at 4.2, vir_register_allocate.c:977-987).
//!
//! Read ports and pairing (sched71):
//!
//! * Each of add a, add b, mul a, mul b is its own raddr with its own
//!   small-immediate bit; two operations merge unless both use a small
//!   immediate (`qpu_merge_raddrs`, qpu_schedule.c:1001-1032), so there is no
//!   two-port limit as on 4.2.
//! * MOV and FMOV may move from the mul unit to the add unit at 7.1
//!   (`can_do_mul_as_add`, qpu_schedule.c:1174-1183).
//! * An SFU result ([`Sfu71`]) is read two instructions later (latency 2,
//!   qpu_schedule.c:1870-1871; a read before that stalls, :1661-1668), and
//!   an SFU stays out of a thread switch's delay slots 1 and 2
//!   (qpu_schedule.c:2079-2083); `v3d_qpu_instr_is_sfu`
//!   (qpu/qpu_instr.c:788-803) is [`super::verify71::is_sfu_op`].
//!
//! I/O signals:
//!
//! * `ldvary` writes its destination and, one instruction later, its C term
//!   to rf0 (qpu_schedule.c:2708-2722); the 4.2 r5 C is rf0 at 7.1
//!   (`emit_fragment_varying`, nir_to_vir.c:1048-1058). It may sit in a thread
//!   switch's delay slots 0 and 1, not 2 (qpu_schedule.c:2085-2090,
//!   1545-1559).
//! * After an implicit rf0 write paired with another register-file signal
//!   write, the thread switch's last delay slot holds no signal that writes
//!   the register file (`has_rf0_flops_conflict`, qpu_schedule.c:1692-1745,
//!   2111-2118).
//! * The thread end: slot 0 writes no register through either ALU, and the
//!   delay slots neither read nor write rf2/rf3 (qpu_schedule.c:2017-2055).
//!
//! 4.2 errata that do not apply at 7.1: GFXH-1684, the closing `vpmwt` of
//! a vertex shader (`emit_vert_end`, nir_to_vir.c:2163-2170), and GFXH-1625,
//! no TMUWT in the final instruction (qpu_schedule.c:1997-2001), are both
//! `ver == 42` only.

use super::isa::{BranchCond, Cond, Msfign, Pf, Uf};
pub use super::isa71::{AddOp71, Instr71, MulOp71, Unpack71};
pub use super::select::{Uniform, V};

/// The refusal every 7.1 stage gives while it is a stub.
pub const NOT_YET_71: &str = "not yet implemented at 7.1";

/// Where an operand comes from at 7.1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Src71 {
    /// A virtual register, read as it is.
    V(V),
    /// A virtual register through an input unpack ([`Unpack71::Abs`], `L`,
    /// `H`, ...): 4.2's `Operand::Abs`, `Lo` and `Hi`.
    Unpacked(V, Unpack71),
    /// A small-immediate table index (one per instruction at 7.1).
    SmallImm(u8),
    /// A fragment payload register: 1 (centroid W), 2 (Z) or 3 (W).
    Payload(u8),
    /// rf0 holding the C term of the last [`Op71::Ldvary`]: 4.2's r5.
    LdvaryC,
    /// No operand.
    None,
}

impl Src71 {
    /// The virtual register the operand reads, if any.
    pub fn vreg(&self) -> Option<V> {
        match *self {
            Src71::V(v) | Src71::Unpacked(v, _) => Some(v),
            _ => None,
        }
    }
}

/// Where a result goes at 7.1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dst71 {
    V(V),
    /// The TLB colour (or Z) write.
    Tlb,
    /// The TLB write that consumes the next uniform as its config.
    Tlbu(Uniform),
    /// The magic "no register".
    Nop,
    /// No destination field (STVPM*).
    None,
    /// A magic write address (the TMU registers; never 0..4 or 19..24,
    /// reserved at 7.1, see [`super::isa71::magic_waddr_name`]).
    Magic(u8),
}

/// The SFU operations, add-unit operations at 7.1 (4.2 wrote magic waddrs
/// 19..24).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfu71 {
    Recip,
    Rsqrt,
    Exp,
    Log,
    Sin,
    Rsqrt2,
}

impl Sfu71 {
    /// The add operation that issues it.
    pub fn add_op(self) -> AddOp71 {
        match self {
            Sfu71::Recip => AddOp71::Recip,
            Sfu71::Rsqrt => AddOp71::Rsqrt,
            Sfu71::Exp => AddOp71::Exp,
            Sfu71::Log => AddOp71::Log,
            Sfu71::Sin => AddOp71::Sin,
            Sfu71::Rsqrt2 => AddOp71::Rsqrt2,
        }
    }
}

/// The operation a [`Op71::Flags`] pushes its flag with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlagOp71 {
    Add(AddOp71),
    Mul(MulOp71),
}

/// A branch target: a [`Op71::Label`]'s number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Label(pub u32);

/// One unit of work for the 7.1 scheduler: 4.2's [`super::select::VOp`]
/// with the 7.1 register model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op71 {
    Add {
        op: AddOp71,
        dst: Dst71,
        a: Src71,
        b: Src71,
    },
    Mul {
        op: MulOp71,
        dst: Dst71,
        a: Src71,
        b: Src71,
    },
    /// An SFU operation on the add unit, its result in a register.
    Sfu {
        op: Sfu71,
        dst: V,
        a: Src71,
    },
    /// The next uniform into `dst`: `ldunif` when `dst` is given rf0 (its
    /// implicit destination), `ldunifrf` otherwise. Writes rf0 implicitly
    /// either way.
    Ldunif {
        dst: V,
        u: Uniform,
    },
    /// The next varying into `dst`; its C term reaches rf0 one instruction
    /// later ([`Src71::LdvaryC`]).
    Ldvary {
        dst: V,
    },
    /// The next word of the oldest look-up's result into `dst` (an explicit
    /// address at 7.1, no implicit rf0 write).
    Ldtmu {
        dst: V,
    },
    /// The `wrtmuc` signal with its TMU configuration uniform.
    Wrtmuc {
        u: Uniform,
    },
    /// One THRSW signal; two in a row are the last-thread-switch pair.
    Thrsw,
    /// A flag push (or update) with no register written.
    Flags {
        unit: FlagOp71,
        a: Src71,
        b: Src71,
        pf: Pf,
        uf: Uf,
    },
    /// `dst = src` where `cond` holds; a use of `dst`, not a definition.
    CondMov {
        dst: V,
        src: Src71,
        cond: Cond,
    },
    /// `setmsf -, src` where `cond` holds.
    Setmsf {
        src: Src71,
        cond: Cond,
    },
    /// `dst = src` on a register already defined (loop-carried values).
    Copy {
        dst: V,
        src: Src71,
    },
    /// The top of a counted loop's body.
    LoopHead,
    /// The counted loop's back edge to the matching [`Op71::LoopHead`].
    BackEdge {
        cond: BranchCond,
        msfign: Msfign,
    },
    /// A forward branch to `label`.
    FwdBranch {
        cond: BranchCond,
        msfign: Msfign,
        label: Label,
    },
    /// A forward branch's target.
    Label(Label),
}

impl Op71 {
    /// The virtual register this operation defines.
    pub fn def(&self) -> Option<V> {
        match *self {
            Op71::Add { dst: Dst71::V(v), .. } | Op71::Mul { dst: Dst71::V(v), .. } => Some(v),
            Op71::Sfu { dst, .. } | Op71::Ldunif { dst, .. } | Op71::Ldvary { dst } | Op71::Ldtmu { dst } => Some(dst),
            _ => None,
        }
    }

    /// The operands it reads.
    pub fn uses(&self) -> [Src71; 2] {
        match *self {
            Op71::Add { a, b, .. } | Op71::Mul { a, b, .. } | Op71::Flags { a, b, .. } => [a, b],
            Op71::Sfu { a, .. } | Op71::Setmsf { src: a, .. } => [a, Src71::None],
            Op71::CondMov { dst, src, .. } | Op71::Copy { dst, src } => [src, Src71::V(dst)],
            _ => [Src71::None; 2],
        }
    }

    /// The operation writes rf0 implicitly (`v3d_qpu_writes_rf0_implicitly`,
    /// qpu/qpu_instr.c:1030-1040): alloc71 keeps every value live across it
    /// out of rf0.
    pub fn writes_rf0_implicitly(&self) -> bool {
        matches!(self, Op71::Ldunif { .. } | Op71::Ldvary { .. })
    }

    /// Control flow: [`super::sched71::pack71`] does not take it, and
    /// [`super::sched71_cf::schedule71`] splits the program at it.
    pub fn is_control_flow(&self) -> bool {
        matches!(self, Op71::LoopHead | Op71::BackEdge { .. } | Op71::FwdBranch { .. } | Op71::Label(_))
    }
}

/// The 7.1 selection of one shader. The shader-record facts stay in the 4.2
/// [`super::select::Selected`] it was made from.
#[derive(Clone, Copy, Debug)]
pub struct Selected71<'a> {
    pub ops: &'a [Op71],
    /// A fragment shader (else a vertex or coordinate shader).
    pub fragment: bool,
    /// Virtual registers handed out.
    pub vregs: u32,
}

/// An empty 4.2 selection for the 7.1 stubs' tests: the one struct literal of
/// [`super::select::Selected`] the 7.1 files hold.
#[cfg(test)]
pub(crate) fn empty_selected(fragment: bool) -> super::select::Selected<'static> {
    super::select::Selected {
        ops: &[],
        fragment,
        vregs: 0,
        vattr_sizes: [0; 16],
        vpm_output_rows: 0,
        uses_vid: false,
        uses_iid: false,
        uses_center_w: false,
        implicit_point_varyings: false,
        writes_z: false,
        disable_ez: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfu_ops_are_verify71s() {
        for s in [Sfu71::Recip, Sfu71::Rsqrt, Sfu71::Exp, Sfu71::Log, Sfu71::Sin, Sfu71::Rsqrt2] {
            assert!(super::super::verify71::is_sfu_op(s.add_op()), "{s:?}");
        }
    }

    #[test]
    fn implicit_rf0_and_defs() {
        let u = Uniform::constant(0);
        assert!(Op71::Ldunif { dst: V(0), u }.writes_rf0_implicitly());
        assert!(Op71::Ldvary { dst: V(1) }.writes_rf0_implicitly());
        assert!(!Op71::Ldtmu { dst: V(2) }.writes_rf0_implicitly());
        assert_eq!(Op71::Sfu { op: Sfu71::Recip, dst: V(3), a: Src71::V(V(0)) }.def(), Some(V(3)));
        assert_eq!(Op71::CondMov { dst: V(4), src: Src71::LdvaryC, cond: Cond::None }.def(), None);
        assert!(Op71::Label(Label(0)).is_control_flow());
    }
}
