// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/qpu/qpu_instr.h (the enum members
// and their values) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 7.1 QPU instruction (the Raspberry Pi 5): the model a decoded
//! word is held in, beside the 4.2 model of [`super::isa`], which it leaves
//! unchanged.
//!
//! Same shape as 4.2 (one 64-bit word, an ALU instruction with an add and a
//! mul half and one signal, or a branch) and the same Mesa structure behind
//! it, but the ISA differs where Mesa 26.2.1 says so (`qpu_pack.c`, above
//! `v3d71_add_ops`; `qpu_instr.h`):
//!
//! * **No accumulators, no mux.** Each of the four ALU operands (add a, add b,
//!   mul a, mul b) is a register-file read address, [`Operand::raddr`], with
//!   its own small-immediate signal ([`Sig71::small_imm_a`] .. `_d`). The
//!   instruction-level raddr_a/raddr_b of 4.2 are gone: the add's raddr_b,
//!   for an operation with one operand, is part of the opcode.
//! * **Implicit destinations.** `ldunif`, `ldtmu` and `ldvary` without an
//!   address write `rf0` (Mesa's 7.1 signal map); the signal's address is the
//!   COND field only for the signals that [`Sig71::writes_address`] says.
//! * **The SFU is an ALU operation.** [`AddOp71::Recip`] .. `Rsqrt2` are add
//!   operations; the magic addresses 19..24 (and 0..4, the accumulators) are
//!   reserved, see [`magic_waddr_name`]. Address 5 is `quad`, 55 `rep`.
//! * **More operations and input unpacks:** the half-float vector family,
//!   MOV/FMOV, VPACK/V8PACK/V10PACK/V11FPACK, the subgroup operations,
//!   SETNNMODE, the mul-side normalised conversions and V8DOT; SAT, NSAT, MAX0
//!   and the integer UL/UH/IL/IH unpacks.
//!
//! As in 4.2, every enum's discriminant is the value of the Mesa enum member
//! it models, pinned by `qpu_ffi`'s tests against Mesa's own name tables. The
//! types shared with 4.2 (conditions, flags, output packs, branches) are
//! [`super::isa`]'s.

use super::isa::num_src;
use super::qpu_ffi::{self, Gen};

pub use super::isa::{Branch, BranchCond, BranchDest, Cond, Flags, Msfign, OutPack, Pf, Uf, ARG_A, ARG_B, ARG_D};

/// The signals one 7.1 ALU instruction can carry (Mesa's `struct
/// v3d_qpu_sig`, the members V3D 7.1 has; which combinations exist is
/// `v3d71_sig_map`). Not on 7.1: `ldvpm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sig71 {
    pub thrsw: bool,
    pub ldunif: bool,
    pub ldunifa: bool,
    pub ldunifrf: bool,
    pub ldunifarf: bool,
    pub ldtmu: bool,
    pub ldvary: bool,
    pub ldtlb: bool,
    pub ldtlbu: bool,
    pub ucb: bool,
    pub rotate: bool,
    pub wrtmuc: bool,
    /// The add ALU's operand a is a small-immediate index.
    pub small_imm_a: bool,
    /// The add ALU's operand b is.
    pub small_imm_b: bool,
    /// The mul ALU's operand a is.
    pub small_imm_c: bool,
    /// The mul ALU's operand b is.
    pub small_imm_d: bool,
}

impl Sig71 {
    /// The signal's destination is named in the COND field (Mesa's
    /// `v3d_qpu_sig_writes_address`); such an instruction has no condition or
    /// flag bits.
    pub fn writes_address(&self) -> bool {
        qpu_ffi::sig71_writes_address(self)
    }
}

/// An input unpack (`enum v3d_qpu_input_unpack`), every member 7.1 encodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unpack71 {
    #[default]
    None = 0,
    Abs = 1,
    L = 2,
    H = 3,
    /// Saturate to [0, 1].
    Sat = 4,
    /// Saturate to [-1, 1].
    Nsat = 5,
    /// Clamp below at 0.
    Max0 = 6,
    ReplicateF32To16 = 7,
    ReplicateL16 = 8,
    ReplicateH16 = 9,
    Swap16 = 10,
    /// The low 16 bits as an unsigned integer (MOV).
    Ul = 11,
    Uh = 12,
    /// The low 16 bits as a signed integer.
    Il = 13,
    Ih = 14,
}

impl Unpack71 {
    pub const ALL: [Unpack71; 15] = {
        use Unpack71::*;
        [None, Abs, L, H, Sat, Nsat, Max0, ReplicateF32To16, ReplicateL16, ReplicateH16, Swap16, Ul, Uh, Il, Ih]
    };
}

/// One ALU operand on 7.1: the register-file read address (or the
/// small-immediate index, when the signal says so for this operand) and its
/// input unpack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operand {
    pub raddr: u8,
    pub unpack: Unpack71,
}

/// The add ALU's operations on 7.1 (`enum v3d_qpu_add_op`, all members, in
/// Mesa's order). Not a member: VDWWT, which has no entry in
/// `v3d71_add_ops`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddOp71 {
    Fadd = 0,
    Faddnf = 1,
    Vfpack = 2,
    Add = 3,
    Sub = 4,
    Fsub = 5,
    Min = 6,
    Max = 7,
    Umin = 8,
    Umax = 9,
    Shl = 10,
    Shr = 11,
    Asr = 12,
    Ror = 13,
    Fmin = 14,
    Fmax = 15,
    Vfmin = 16,
    Vfadd = 17,
    Vfsub = 18,
    Vfcmp = 19,
    Vfmov = 20,
    Vfabs = 21,
    Vfneg = 22,
    Vfnab = 23,
    And = 24,
    Or = 25,
    Xor = 26,
    Vadd = 27,
    Vsub = 28,
    Not = 29,
    Neg = 30,
    Flapush = 31,
    Flbpush = 32,
    Flpop = 33,
    Recip = 34,
    Setmsf = 35,
    Setrevf = 36,
    Nop = 37,
    Tidx = 38,
    Eidx = 39,
    Lr = 40,
    Vfla = 41,
    Vflna = 42,
    Vflb = 43,
    Vflnb = 44,
    Fxcd = 45,
    Xcd = 46,
    Fycd = 47,
    Ycd = 48,
    Msf = 49,
    Revf = 50,
    Iid = 52,
    Sampid = 53,
    Barrierid = 54,
    Tmuwt = 55,
    Vpmsetup = 56,
    Vpmwt = 57,
    Flafirst = 58,
    Flnafirst = 59,
    LdvpmvIn = 60,
    LdvpmvOut = 61,
    LdvpmdIn = 62,
    LdvpmdOut = 63,
    Ldvpmp = 64,
    Rsqrt = 65,
    Exp = 66,
    Log = 67,
    Sin = 68,
    Rsqrt2 = 69,
    LdvpmgIn = 70,
    LdvpmgOut = 71,
    Fcmp = 72,
    Vfmax = 73,
    Fround = 74,
    Ftoin = 75,
    Ftrunc = 76,
    Ftoiz = 77,
    Ffloor = 78,
    Ftouz = 79,
    Fceil = 80,
    Ftoc = 81,
    Fdx = 82,
    Fdy = 83,
    Stvpmv = 84,
    Stvpmd = 85,
    Stvpmp = 86,
    Itof = 87,
    Clz = 88,
    Utof = 89,
    Fmov = 90,
    Mov = 91,
    Vpack = 92,
    V8pack = 93,
    V10pack = 94,
    V11fpack = 95,
    Ballot = 96,
    Bcastf = 97,
    Alleq = 98,
    Allfeq = 99,
    Rotq = 100,
    Rot = 101,
    Shuffle = 102,
    SetnnmodeUu = 103,
    SetnnmodeSu = 104,
    SetnnmodeUs = 105,
    SetnnmodeSs = 106,
}

impl AddOp71 {
    pub const ALL: [AddOp71; 106] = {
        use AddOp71::*;
        [
            Fadd, Faddnf, Vfpack, Add, Sub, Fsub, Min, Max, Umin, Umax, Shl, Shr, Asr, Ror, Fmin, Fmax, Vfmin, Vfadd,
            Vfsub, Vfcmp, Vfmov, Vfabs, Vfneg, Vfnab, And, Or, Xor, Vadd, Vsub, Not, Neg, Flapush, Flbpush, Flpop,
            Recip, Setmsf, Setrevf, Nop, Tidx, Eidx, Lr, Vfla, Vflna, Vflb, Vflnb, Fxcd, Xcd, Fycd, Ycd, Msf, Revf,
            Iid, Sampid, Barrierid, Tmuwt, Vpmsetup, Vpmwt, Flafirst, Flnafirst, LdvpmvIn, LdvpmvOut, LdvpmdIn,
            LdvpmdOut, Ldvpmp, Rsqrt, Exp, Log, Sin, Rsqrt2, LdvpmgIn, LdvpmgOut, Fcmp, Vfmax, Fround, Ftoin,
            Ftrunc, Ftoiz, Ffloor, Ftouz, Fceil, Ftoc, Fdx, Fdy, Stvpmv, Stvpmd, Stvpmp, Itof, Clz, Utof, Fmov, Mov,
            Vpack, V8pack, V10pack, V11fpack, Ballot, Bcastf, Alleq, Allfeq, Rotq, Rot, Shuffle, SetnnmodeUu,
            SetnnmodeSu, SetnnmodeUs, SetnnmodeSs,
        ]
    };
}

/// The mul ALU's operations on 7.1 (`enum v3d_qpu_mul_op`, all members).
/// Not a member: UMUL24_RTOP0, which is UMUL24's opcode (see
/// [`super::isa::MulOp`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulOp71 {
    Add = 0,
    Sub = 1,
    Umul24 = 2,
    Vfmul = 4,
    Smul24 = 5,
    Multop = 6,
    Fmov = 7,
    Mov = 8,
    Nop = 9,
    Fmul = 10,
    Ftounorm16 = 11,
    Ftosnorm16 = 12,
    Vftounorm8 = 13,
    Vftosnorm8 = 14,
    Funpackunormlo = 15,
    Funpackunormhi = 16,
    Funpacksnormlo = 17,
    Funpacksnormhi = 18,
    Vftounorm10lo = 19,
    Vftounorm10hi = 20,
    V8dot = 21,
}

impl MulOp71 {
    pub const ALL: [MulOp71; 21] = {
        use MulOp71::*;
        [
            Add, Sub, Umul24, Vfmul, Smul24, Multop, Fmov, Mov, Nop, Fmul, Ftounorm16, Ftosnorm16, Vftounorm8,
            Vftosnorm8, Funpackunormlo, Funpackunormhi, Funpacksnormlo, Funpacksnormhi, Vftounorm10lo,
            Vftounorm10hi, V8dot,
        ]
    };
}

/// Which of destination, a and b an add operation uses (Mesa's
/// `v3d_qpu_add_op_has_dst` and `_num_src`).
pub fn add_op_args(op: AddOp71) -> u8 {
    qpu_ffi::add_op71_args(op)
}

/// Which of destination, a and b a mul operation uses.
pub fn mul_op_args(op: MulOp71) -> u8 {
    qpu_ffi::mul_op71_args(op)
}

/// The add ALU's half of a 7.1 instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddAlu71 {
    pub op: AddOp71,
    /// Operand a's address is written whether or not the operation reads it;
    /// operand b of an operation with fewer than two operands is
    /// [`Operand::default`] in a decoded instruction (its raddr is part of
    /// the opcode).
    pub a: Operand,
    pub b: Operand,
    pub waddr: u8,
    /// `waddr` names a magic register (`enum v3d_qpu_waddr`).
    pub magic: bool,
    pub pack: OutPack,
}

/// The mul ALU's half of a 7.1 instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MulAlu71 {
    pub op: MulOp71,
    pub a: Operand,
    pub b: Operand,
    pub waddr: u8,
    pub magic: bool,
    pub pack: OutPack,
}

/// A 7.1 ALU instruction: both ALUs, one signal, one flags field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alu71 {
    pub sig: Sig71,
    /// The signal's destination when [`Sig71::writes_address`]; zero otherwise.
    pub sig_addr: u8,
    pub sig_magic: bool,
    /// Always [`Flags::default`] when the signal writes an address.
    pub flags: Flags,
    pub add: AddAlu71,
    pub mul: MulAlu71,
}

/// The magic write address "no register" (`V3D_QPU_WADDR_NOP`).
pub const WADDR_NOP: u8 = 6;
/// `V3D_QPU_WADDR_QUAD`, which is R5's place on 4.x.
pub const WADDR_QUAD: u8 = 5;
/// `V3D_QPU_WADDR_REP`, which is R5REP's place on 4.x.
pub const WADDR_REP: u8 = 55;

impl Alu71 {
    /// Mesa's `v3d_qpu_nop()`: both ALUs NOP, each writing the magic NOP
    /// register, operand a's address 0, no signal, no flags.
    pub fn nop() -> Alu71 {
        Alu71 {
            sig: Sig71::default(),
            sig_addr: 0,
            sig_magic: false,
            flags: Flags::default(),
            add: AddAlu71 {
                op: AddOp71::Nop,
                a: Operand::default(),
                b: Operand::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            },
            mul: MulAlu71 {
                op: MulOp71::Nop,
                a: Operand::default(),
                b: Operand::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            },
        }
    }

    /// The four operands with their small-immediate flag, in the order add a,
    /// add b, mul a, mul b.
    pub fn operands(&self) -> [(Operand, bool); 4] {
        [
            (self.add.a, self.sig.small_imm_a),
            (self.add.b, self.sig.small_imm_b),
            (self.mul.a, self.sig.small_imm_c),
            (self.mul.b, self.sig.small_imm_d),
        ]
    }

    /// The value of a small-immediate operand (`None` for an index past the
    /// table).
    pub fn small_imm(op: Operand) -> Option<u32> {
        super::isa::small_imm_value(op.raddr)
    }

    /// The operands the operations read (not the ones whose raddr is
    /// opcode), in the order of [`Alu71::operands`], with their small-imm
    /// flag.
    pub fn read_operands(&self) -> impl Iterator<Item = (Operand, bool)> {
        let (an, mn) = (num_src(add_op_args(self.add.op)), num_src(mul_op_args(self.mul.op)));
        self.operands().into_iter().enumerate().filter_map(move |(i, o)| {
            let read = match i {
                0 => an >= 1,
                1 => an >= 2,
                2 => mn >= 1,
                _ => mn >= 2,
            };
            read.then_some(o)
        })
    }
}

/// A 7.1 instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr71 {
    Alu(Alu71),
    Branch(Branch),
}

/// The name of a magic register on 7.1: Mesa's `v3d_qpu_magic_waddr_name` for
/// ver 71, `None` where Mesa's table has no entry (25..31, 47..54), past its
/// end (56..63), and for the addresses `qpu_instr.h` marks "Reserved on V3D
/// 7.x" (0..4, the accumulators, and 19..24, the SFU of 4.x). A magic write
/// to one of those is refused ([`super::decode::DecodeError::MagicWaddrUnnamed`]).
pub fn magic_waddr_name(waddr: u8) -> Option<&'static str> {
    qpu_ffi::magic_waddr_name_for(Gen::V71, waddr)
}
