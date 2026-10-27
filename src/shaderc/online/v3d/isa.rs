// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/qpu/qpu_instr.h (the enum members
// and their values) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 4.2 QPU instruction: the model a decoded word is held in and the
//! back end builds.
//!
//! A QPU instruction is one 64-bit word. It is either an **ALU** instruction
//! (the add ALU and the mul ALU issue together, with one signal and one set of
//! condition/flag bits) or a **branch**. The model is Mesa's
//! `struct v3d_qpu_instr` (`src/broadcom/qpu/qpu_instr.h`, Mesa 26.2.1,
//! Copyright (c) 2016 Broadcom, MIT licence), restricted to what V3D 4.2
//! encodes. Where V3D 7.x differs it is said at the item; nothing here decodes
//! or encodes a 7.x word.
//!
//! The model is semantic, not a bag of bit fields: an operation, its operands
//! (a mux and an input unpack each), its destination, its output pack. Where
//! those sit in the word is Mesa's business: [`super::encode`] and
//! [`super::decode`] go through `v3d_qpu_instr_pack` / `v3d_qpu_instr_unpack`
//! ([`super::qpu_ffi`]), and every enum's discriminant is the value of the
//! Mesa enum member it models (pinned by `qpu_ffi`'s tests against Mesa's
//! own name tables), so the conversion is a cast one way and a checked
//! look-up the other. The questions the model asks about an operation or a
//! register (its operands, its name, the small immediates) are Mesa's
//! answers too, through the same FFI.

use super::qpu_ffi;

// ---------------------------------------------------------------------------
// The model.
// ---------------------------------------------------------------------------

/// The signals one ALU instruction can carry (Mesa's `struct v3d_qpu_sig`,
/// the members V3D 4.2 has). Which combinations exist is Mesa's
/// `v3d42_sig_map`; a combination not in it is not encodable.
///
/// Not on 4.2: `ldvpm` (V3D 3.x) and `small_imm_a`/`_c`/`_d` (7.x, one per
/// operand; 4.2 has the one small immediate, in raddr_b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sig {
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
    /// raddr_b holds a small-immediate index, and mux B reads its value.
    pub small_imm_b: bool,
}

impl Sig {
    /// The signal writes a register of its own, named in the COND field
    /// (Mesa's `v3d_qpu_sig_writes_address`). An instruction whose signal
    /// does this has no condition or flag bits.
    pub fn writes_address(&self) -> bool {
        qpu_ffi::sig_writes_address(self)
    }
}

/// An ALU's condition (`enum v3d_qpu_cond`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cond {
    #[default]
    None = 0,
    IfA = 1,
    IfB = 2,
    IfNA = 3,
    IfNB = 4,
}

impl Cond {
    pub const ALL: [Cond; 5] = [Cond::None, Cond::IfA, Cond::IfB, Cond::IfNA, Cond::IfNB];
}

/// Push a flag (`enum v3d_qpu_pf`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pf {
    #[default]
    None = 0,
    PushZ = 1,
    PushN = 2,
    PushC = 3,
}

impl Pf {
    pub const ALL: [Pf; 4] = [Pf::None, Pf::PushZ, Pf::PushN, Pf::PushC];
}

/// Update the flags (`enum v3d_qpu_uf`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Uf {
    #[default]
    None = 0,
    AndZ = 1,
    AndNZ = 2,
    NorNZ = 3,
    NorZ = 4,
    AndN = 5,
    AndNN = 6,
    NorNN = 7,
    NorN = 8,
    AndC = 9,
    AndNC = 10,
    NorNC = 11,
    NorC = 12,
}

impl Uf {
    pub const ALL: [Uf; 13] = {
        use Uf::*;
        [None, AndZ, AndNZ, NorNZ, NorZ, AndN, AndNN, NorNN, NorN, AndC, AndNC, NorNC, NorC]
    };
}

/// Both ALUs' conditions and flag updates (`struct v3d_qpu_flags`). Not every
/// combination is encodable (Mesa's `v3d_qpu_flags_pack` says which).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub ac: Cond,
    pub mc: Cond,
    pub apf: Pf,
    pub mpf: Pf,
    pub auf: Uf,
    pub muf: Uf,
}

/// Where an ALU operand comes from on 4.2: an accumulator, or one of the two
/// register-file read ports (`enum v3d_qpu_mux`). (7.x has no accumulators
/// and no mux; each operand is a raddr.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mux {
    #[default]
    R0 = 0,
    R1 = 1,
    R2 = 2,
    R3 = 3,
    R4 = 4,
    R5 = 5,
    /// The register file at raddr_a.
    A = 6,
    /// The register file at raddr_b, or the small immediate.
    B = 7,
}

impl Mux {
    pub const ALL: [Mux; 8] = [Mux::R0, Mux::R1, Mux::R2, Mux::R3, Mux::R4, Mux::R5, Mux::A, Mux::B];
    pub fn from_bits(v: u32) -> Mux {
        Mux::ALL[(v & 7) as usize]
    }
}

/// An input unpack (`enum v3d_qpu_input_unpack`, the members 4.2 encodes).
/// Not on 4.2: SAT, NSAT, MAX0 (7.1, values 4..6) and the integer
/// UL/UH/IL/IH (7.x MOV).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unpack {
    #[default]
    None = 0,
    /// Absolute value.
    Abs = 1,
    /// The low half as a 16-bit float.
    L = 2,
    /// The high half as a 16-bit float.
    H = 3,
    /// Convert to f16 and replicate into both halves (`.ff`).
    ReplicateF32To16 = 7,
    /// Replicate the low half (`.ll`).
    ReplicateL16 = 8,
    /// Replicate the high half (`.hh`).
    ReplicateH16 = 9,
    /// Swap the halves (`.swp`).
    Swap16 = 10,
}

impl Unpack {
    pub const ALL: [Unpack; 8] = [
        Unpack::None,
        Unpack::Abs,
        Unpack::L,
        Unpack::H,
        Unpack::ReplicateF32To16,
        Unpack::ReplicateL16,
        Unpack::ReplicateH16,
        Unpack::Swap16,
    ];
}

/// An output pack (`enum v3d_qpu_output_pack`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutPack {
    #[default]
    None = 0,
    /// To f16, into the low half, the high half unchanged.
    L = 1,
    /// To f16, into the high half.
    H = 2,
}

impl OutPack {
    pub const ALL: [OutPack; 3] = [OutPack::None, OutPack::L, OutPack::H];
}

/// One ALU operand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub mux: Mux,
    pub unpack: Unpack,
}

/// The add ALU's operations on 4.2 (`enum v3d_qpu_add_op`, in Mesa's order,
/// without the 7.x-only members: VFADD, VFSUB, VFCMP, VFMOV, VFABS, VFNEG,
/// VFNAB, FMOV, MOV, VPACK, V8PACK, V10PACK, V11FPACK, BALLOT, BCASTF, ALLEQ,
/// ALLFEQ, ROTQ, ROT, SHUFFLE, SETNNMODE_*).
///
/// Not a member: VDWWT. Mesa's v3d42 table lists VDWWT (a V3D 3.3 operation,
/// `first_ver` 33 and no `last_ver`) and IID (`first_ver` 40) with the same
/// opcode and muxes, VDWWT first, so on 4.2 Mesa's decoder names the word
/// "vdwwt" while its compiler emits it as IID (nir_to_vir.c:3936,
/// gl_InvocationID in a geometry shader). The word is one operation on this
/// GPU, and it is modelled as [`AddOp::Iid`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddOp {
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
}

impl AddOp {
    pub const ALL: [AddOp; 82] = {
        use AddOp::*;
        [
            Fadd, Faddnf, Vfpack, Add, Sub, Fsub, Min, Max, Umin, Umax, Shl, Shr, Asr, Ror, Fmin, Fmax, Vfmin, And, Or,
            Xor, Vadd, Vsub, Not, Neg, Flapush, Flbpush, Flpop, Recip, Setmsf, Setrevf, Nop, Tidx, Eidx, Lr, Vfla,
            Vflna, Vflb, Vflnb, Fxcd, Xcd, Fycd, Ycd, Msf, Revf, Iid, Sampid, Barrierid, Tmuwt, Vpmsetup, Vpmwt,
            Flafirst, Flnafirst, LdvpmvIn, LdvpmvOut, LdvpmdIn, LdvpmdOut, Ldvpmp, Rsqrt, Exp, Log, Sin, Rsqrt2,
            LdvpmgIn, LdvpmgOut, Fcmp, Vfmax, Fround, Ftoin, Ftrunc, Ftoiz, Ffloor, Ftouz, Fceil, Ftoc, Fdx, Fdy,
            Stvpmv, Stvpmd, Stvpmp, Itof, Clz, Utof,
        ]
    };
}

/// The mul ALU's operations on 4.2 (`enum v3d_qpu_mul_op`, without the 7.x
/// members FTOUNORM16 .. V8DOT).
///
/// Not a member: UMUL24_RTOP0. It is UMUL24's opcode (3) and exists in Mesa
/// only to tell its scheduler that rtop is known to be zero; the word cannot
/// say it, and Mesa's decoder gives UMUL24 back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulOp {
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
}

impl MulOp {
    pub const ALL: [MulOp; 10] = {
        use MulOp::*;
        [Add, Sub, Umul24, Vfmul, Smul24, Multop, Fmov, Mov, Nop, Fmul]
    };
}

/// Flags for [`add_op_args`]/[`mul_op_args`]: the operation writes a
/// destination, reads operand a, reads operand b.
pub const ARG_D: u8 = 1;
pub const ARG_A: u8 = 2;
pub const ARG_B: u8 = 4;

/// Which of destination, a and b an add operation uses (Mesa's
/// `v3d_qpu_add_op_has_dst` and `v3d_qpu_add_op_num_src`).
pub fn add_op_args(op: AddOp) -> u8 {
    qpu_ffi::add_op_args(op)
}

/// Which of destination, a and b a mul operation uses (Mesa's
/// `v3d_qpu_mul_op_has_dst` and `v3d_qpu_mul_op_num_src`).
pub fn mul_op_args(op: MulOp) -> u8 {
    qpu_ffi::mul_op_args(op)
}

/// The number of operands an argument set reads (`v3d_qpu_*_op_num_src`).
pub fn num_src(args: u8) -> usize {
    if args & ARG_B != 0 {
        2
    } else if args & ARG_A != 0 {
        1
    } else {
        0
    }
}

/// The add ALU's half of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddAlu {
    pub op: AddOp,
    /// An operand the operation does not read is [`Input::default`] in a
    /// decoded instruction; its mux bits are part of the opcode instead.
    pub a: Input,
    pub b: Input,
    pub waddr: u8,
    /// `waddr` names a magic register (`enum v3d_qpu_waddr`), not the
    /// register file.
    pub magic: bool,
    pub pack: OutPack,
}

/// The mul ALU's half of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MulAlu {
    pub op: MulOp,
    pub a: Input,
    pub b: Input,
    pub waddr: u8,
    pub magic: bool,
    pub pack: OutPack,
}

/// An ALU instruction: both ALUs, one signal, one flags field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alu {
    pub sig: Sig,
    /// The signal's destination when [`Sig::writes_address`]; zero otherwise.
    pub sig_addr: u8,
    /// `sig_addr` names a magic register.
    pub sig_magic: bool,
    /// Always [`Flags::default`] when the signal writes an address: the field
    /// holds the address instead.
    pub flags: Flags,
    /// The register-file address read port A reads. Present in the word
    /// whether or not an operand selects [`Mux::A`].
    pub raddr_a: u8,
    /// Port B's address, or the small-immediate index under
    /// [`Sig::small_imm_b`].
    pub raddr_b: u8,
    pub add: AddAlu,
    pub mul: MulAlu,
}

/// The magic write address "no register" (`V3D_QPU_WADDR_NOP`).
pub const WADDR_NOP: u8 = 6;

impl Alu {
    /// Mesa's `v3d_qpu_nop()` (vir_to_qpu.c:49): both ALUs NOP, each writing
    /// the magic NOP register, raddr_a/raddr_b 0, no signal, no flags. These
    /// are builder defaults, asserted against the corpus by
    /// `roundtrip::builder_nop_defaults_hold_in_the_corpus`.
    pub fn nop() -> Alu {
        Alu {
            sig: Sig::default(),
            sig_addr: 0,
            sig_magic: false,
            flags: Flags::default(),
            raddr_a: 0,
            raddr_b: 0,
            add: AddAlu {
                op: AddOp::Nop,
                a: Input::default(),
                b: Input::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            },
            mul: MulAlu {
                op: MulOp::Nop,
                a: Input::default(),
                b: Input::default(),
                waddr: WADDR_NOP,
                magic: true,
                pack: OutPack::None,
            },
        }
    }

    /// The small immediate mux B reads, when the signal says raddr_b is one
    /// and the index is in the table.
    pub fn small_imm(&self) -> Option<u32> {
        if self.sig.small_imm_b {
            small_imm_value(self.raddr_b)
        } else {
            None
        }
    }
}

/// The name of a magic register a write address or a signal address can
/// name: Mesa's `v3d_qpu_magic_waddr_name` for 4.x. `None` where Mesa's table
/// has no entry (25..31, 47..54) and past its end (56..63, where Mesa reads
/// out of bounds). No reference names those, so the decoder and the encoder
/// refuse a magic write to them
/// ([`super::decode::DecodeError::MagicWaddrUnnamed`]).
pub fn magic_waddr_name(waddr: u8) -> Option<&'static str> {
    qpu_ffi::magic_waddr_name(waddr)
}

/// Which lanes' condition a branch tests (`enum v3d_qpu_branch_cond`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchCond {
    Always = 0,
    A0 = 1,
    NA0 = 2,
    AllA = 3,
    AnyNA = 4,
    AnyA = 5,
    AllNA = 6,
}

impl BranchCond {
    pub const ALL: [BranchCond; 7] = {
        use BranchCond::*;
        [Always, A0, NA0, AllA, AnyNA, AnyA, AllNA]
    };
}

/// Which lanes a branch ignores (`enum v3d_qpu_msfign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msfign {
    None = 0,
    P = 1,
    Q = 2,
}

impl Msfign {
    pub const ALL: [Msfign; 3] = [Msfign::None, Msfign::P, Msfign::Q];
}

/// How a branch computes the new instruction or uniform address
/// (`enum v3d_qpu_branch_dest`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchDest {
    Abs = 0,
    Rel = 1,
    LinkReg = 2,
    Regfile = 3,
}

impl BranchDest {
    pub const ALL: [BranchDest; 4] = [BranchDest::Abs, BranchDest::Rel, BranchDest::LinkReg, BranchDest::Regfile];
}

/// A branch (`struct v3d_qpu_branch_instr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Branch {
    pub cond: BranchCond,
    pub msfign: Msfign,
    pub bdi: BranchDest,
    /// The uniform stream branches too.
    pub ub: bool,
    /// Meaningful only with `ub`; [`BranchDest::Abs`] (encoding 0) without.
    pub bdu: BranchDest,
    /// The register read when `bdi` or `bdu` is [`BranchDest::Regfile`];
    /// zero otherwise.
    pub raddr_a: u8,
    /// The byte offset for [`BranchDest::Abs`] and [`BranchDest::Rel`]: a
    /// multiple of 8, 32 bits. Zero for the other two.
    pub offset: u32,
}

impl Branch {
    /// raddr_a is read by this branch.
    pub fn reads_raddr_a(&self) -> bool {
        self.bdi == BranchDest::Regfile || (self.ub && self.bdu == BranchDest::Regfile)
    }
    /// The offset field is read by this branch.
    pub fn has_offset(&self) -> bool {
        matches!(self.bdi, BranchDest::Abs | BranchDest::Rel)
    }
}

/// One QPU instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr {
    Alu(Alu),
    Branch(Branch),
}

/// The value of small-immediate index `i` (Mesa's
/// `v3d_qpu_small_imm_unpack`): the integers 0..15 and -16..-1, then the
/// powers of two 2^-8..2^7 as f32 bits. 48..63 are not in the table.
pub fn small_imm_value(i: u8) -> Option<u32> {
    qpu_ffi::small_imm_unpack(i as u32)
}

/// The index of a value in the small-immediate table, if it is there (Mesa's
/// `v3d_qpu_small_imm_pack`).
pub fn small_imm_index(v: u32) -> Option<u8> {
    qpu_ffi::small_imm_pack(v).map(|i| i as u8)
}
