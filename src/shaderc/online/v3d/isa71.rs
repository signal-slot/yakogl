// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/qpu/qpu_instr.h (the enum members
// and their values) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 7.1 QPU instruction (the Raspberry Pi 5): the 7.1 twin of
//! [`super::isa`], whose 4.2 model is unchanged.
//!
//! One `struct v3d_qpu_instr` (Mesa 26.2.1, Copyright (c) 2016 Broadcom, MIT
//! licence) serves both generations; what 7.1 changes in what it holds
//! (`docs/notes/pre-study-rpi5.md`, section 3.3):
//!
//! * **No accumulators, no mux.** An operand is a register-file address
//!   ([`Input71::raddr`], six bits) of its own: the add ALU's two operands
//!   and the mul ALU's two read four addresses (`raddr_a` .. `raddr_d`)
//!   instead of two ports, and each may be a small immediate
//!   ([`Sig71::small_imm_a`] .. `small_imm_d`) instead of a register. The
//!   word-level `raddr_a`/`raddr_b` of 4.2 are not part of the model.
//! * **`rf0` is the implicit destination** of the signals that wrote
//!   r4/r5 (`ldunif`, `ldtmu`, `ldvary`, ...): see
//!   [`Alu71::writes_rf0_implicitly`]. Magic write addresses 0..4 are
//!   reserved, 5 is `quad` and 55 `rep` ([`magic_waddr_name`]).
//! * **The special functions are ALU operations** ([`AddOp71::Recip`] ..
//!   [`AddOp71::Rsqrt2`], [`Alu71::is_sfu`]), no longer magic writes to
//!   addresses 19..24, which are reserved.
//! * More operations ([`AddOp71`], [`MulOp71`]) and input unpacks
//!   ([`Unpack71`]).
//!
//! Where a field sits in the word is Mesa's business (`v3d_qpu_instr_pack`
//! and `_unpack` with the 7.1 device, [`super::codec71`]); every
//! discriminant here is the value of the Mesa enum member it models, pinned
//! by `qpu_ffi`'s tests against Mesa's own name tables. Branches are the
//! same on both generations and are [`Branch`].

use super::qpu_ffi::{self, Gen};

pub use super::isa::{Branch, BranchCond, BranchDest, Cond, Flags, Msfign, OutPack, Pf, Uf, ARG_A, ARG_B, ARG_D};

/// The generation this module models.
pub const GEN: Gen = Gen::V71;

/// The signals one 7.1 ALU instruction can carry (Mesa's `struct
/// v3d_qpu_sig`, the members `v3d71_sig_map` uses; `ldvpm` is V3D 3.x). Which
/// combinations exist is that map; one not in it is not encodable.
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
    /// The add ALU's operand a is a small-immediate index, not a register.
    pub small_imm_a: bool,
    /// The add ALU's operand b is one.
    pub small_imm_b: bool,
    /// The mul ALU's operand a is one.
    pub small_imm_c: bool,
    /// The mul ALU's operand b is one.
    pub small_imm_d: bool,
}

impl Sig71 {
    /// The signal writes a register of its own, named in the COND field.
    pub fn writes_address(&self) -> bool {
        qpu_ffi::sig_writes_address_mesa(GEN, &super::codec71::sig_to_mesa(self))
    }
}

/// An input unpack (`enum v3d_qpu_input_unpack`, every member 7.1 can
/// encode), the values being Mesa's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unpack71 {
    #[default]
    None = 0,
    Abs = 1,
    L = 2,
    H = 3,
    /// Saturate to [0, 1] (`V3D71_QPU_UNPACK_SAT`).
    Sat = 4,
    /// Saturate to [-1, 1].
    Nsat = 5,
    /// Saturate to [0, +inf].
    Max0 = 6,
    ReplicateF32To16 = 7,
    ReplicateL16 = 8,
    ReplicateH16 = 9,
    Swap16 = 10,
    /// The low 16 bits as an unsigned integer.
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

/// One ALU operand on 7.1: a register-file address (or, under the matching
/// `small_imm_*` signal, a small-immediate index) and an unpack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input71 {
    pub raddr: u8,
    pub unpack: Unpack71,
}

/// The add ALU's operations (`enum v3d_qpu_add_op`, every member, in Mesa's
/// order; the 7.x ones are FMOV .. SETNNMODE_SS). Not every member has a 7.1
/// encoding (VDWWT is V3D 3.3's): the decoder says which do.
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
    Vdwwt = 51,
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
    pub const ALL: [AddOp71; 107] = {
        use AddOp71::*;
        [
            Fadd,
            Faddnf,
            Vfpack,
            Add,
            Sub,
            Fsub,
            Min,
            Max,
            Umin,
            Umax,
            Shl,
            Shr,
            Asr,
            Ror,
            Fmin,
            Fmax,
            Vfmin,
            Vfadd,
            Vfsub,
            Vfcmp,
            Vfmov,
            Vfabs,
            Vfneg,
            Vfnab,
            And,
            Or,
            Xor,
            Vadd,
            Vsub,
            Not,
            Neg,
            Flapush,
            Flbpush,
            Flpop,
            Recip,
            Setmsf,
            Setrevf,
            Nop,
            Tidx,
            Eidx,
            Lr,
            Vfla,
            Vflna,
            Vflb,
            Vflnb,
            Fxcd,
            Xcd,
            Fycd,
            Ycd,
            Msf,
            Revf,
            Vdwwt,
            Iid,
            Sampid,
            Barrierid,
            Tmuwt,
            Vpmsetup,
            Vpmwt,
            Flafirst,
            Flnafirst,
            LdvpmvIn,
            LdvpmvOut,
            LdvpmdIn,
            LdvpmdOut,
            Ldvpmp,
            Rsqrt,
            Exp,
            Log,
            Sin,
            Rsqrt2,
            LdvpmgIn,
            LdvpmgOut,
            Fcmp,
            Vfmax,
            Fround,
            Ftoin,
            Ftrunc,
            Ftoiz,
            Ffloor,
            Ftouz,
            Fceil,
            Ftoc,
            Fdx,
            Fdy,
            Stvpmv,
            Stvpmd,
            Stvpmp,
            Itof,
            Clz,
            Utof,
            Fmov,
            Mov,
            Vpack,
            V8pack,
            V10pack,
            V11fpack,
            Ballot,
            Bcastf,
            Alleq,
            Allfeq,
            Rotq,
            Rot,
            Shuffle,
            SetnnmodeUu,
            SetnnmodeSu,
            SetnnmodeUs,
            SetnnmodeSs,
        ]
    };

    /// A special-function operation (on 4.2 these are magic writes).
    pub fn is_sfu(self) -> bool {
        matches!(self, AddOp71::Recip | AddOp71::Rsqrt | AddOp71::Exp | AddOp71::Log | AddOp71::Sin | AddOp71::Rsqrt2)
    }
}

/// The mul ALU's operations (`enum v3d_qpu_mul_op`, every member; the 7.x
/// ones are FTOUNORM16 .. V8DOT). UMUL24_RTOP0 is UMUL24's opcode; Mesa's
/// decoder gives UMUL24 back, so it is not something a word can say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulOp71 {
    Add = 0,
    Sub = 1,
    Umul24 = 2,
    Umul24Rtop0 = 3,
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
    pub const ALL: [MulOp71; 22] = {
        use MulOp71::*;
        [
            Add,
            Sub,
            Umul24,
            Umul24Rtop0,
            Vfmul,
            Smul24,
            Multop,
            Fmov,
            Mov,
            Nop,
            Fmul,
            Ftounorm16,
            Ftosnorm16,
            Vftounorm8,
            Vftosnorm8,
            Funpackunormlo,
            Funpackunormhi,
            Funpacksnormlo,
            Funpacksnormhi,
            Vftounorm10lo,
            Vftounorm10hi,
            V8dot,
        ]
    };
}

/// Which of destination, a and b an add operation uses (Mesa's
/// `v3d_qpu_add_op_has_dst` and `_num_src`), as [`ARG_D`]/[`ARG_A`]/[`ARG_B`].
pub fn add_op_args(op: AddOp71) -> u8 {
    qpu_ffi::add_op_args_raw(op as u32)
}

/// The same for a mul operation.
pub fn mul_op_args(op: MulOp71) -> u8 {
    qpu_ffi::mul_op_args_raw(op as u32)
}

/// The add ALU's half of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddAlu71 {
    pub op: AddOp71,
    /// An operand the operation does not read is [`Input71::default`] in a
    /// decoded instruction.
    pub a: Input71,
    pub b: Input71,
    pub waddr: u8,
    /// `waddr` names a magic register, not the register file.
    pub magic: bool,
    pub pack: OutPack,
}

/// The mul ALU's half of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MulAlu71 {
    pub op: MulOp71,
    pub a: Input71,
    pub b: Input71,
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

impl Alu71 {
    /// Mesa's `v3d_qpu_nop()`: both ALUs NOP writing the magic NOP register,
    /// no signal, no flags.
    pub fn nop() -> Alu71 {
        Alu71 {
            sig: Sig71::default(),
            sig_addr: 0,
            sig_magic: false,
            flags: Flags::default(),
            add: AddAlu71 {
                op: AddOp71::Nop,
                a: Input71::default(),
                b: Input71::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            },
            mul: MulAlu71 {
                op: MulOp71::Nop,
                a: Input71::default(),
                b: Input71::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            },
        }
    }

    /// The signal (not an operation) writes rf0: Mesa's
    /// `v3d_qpu_writes_rf0_implicitly` for 7.1.
    pub fn writes_rf0_implicitly(&self) -> bool {
        qpu_ffi::writes_rf0_implicitly(GEN, &super::codec71::to_mesa(&Instr71::Alu(*self)))
    }

    /// The add ALU does a special function (Mesa's `v3d_qpu_instr_is_sfu`).
    pub fn is_sfu(&self) -> bool {
        self.add.op.is_sfu()
    }

    /// The small immediate operand `which` reads (0..3 for a, b, c, d: the
    /// add ALU's a and b, the mul ALU's a and b), when its signal says the
    /// address is one and the index is in the table.
    pub fn small_imm(&self, which: usize) -> Option<u32> {
        let (on, raddr) = match which {
            0 => (self.sig.small_imm_a, self.add.a.raddr),
            1 => (self.sig.small_imm_b, self.add.b.raddr),
            2 => (self.sig.small_imm_c, self.mul.a.raddr),
            3 => (self.sig.small_imm_d, self.mul.b.raddr),
            _ => return None,
        };
        if on {
            small_imm_value(raddr)
        } else {
            None
        }
    }
}

/// The name of a magic register on 7.1 (Mesa's `v3d_qpu_magic_waddr_name`
/// with the 7.1 device: 5 is `quad`, 55 `rep`). `None` where Mesa's table has
/// no entry (25..31, 47..54) and past its end (56..63).
pub fn magic_waddr_name(waddr: u8) -> Option<&'static str> {
    qpu_ffi::magic_waddr_name_for(GEN, waddr)
}

/// The value of small-immediate index `i` (the table is the same on both
/// generations).
pub fn small_imm_value(i: u8) -> Option<u32> {
    qpu_ffi::small_imm_unpack(i as u32)
}

/// One 7.1 QPU instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr71 {
    Alu(Alu71),
    Branch(Branch),
}
