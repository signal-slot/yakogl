// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Decoding and encoding V3D 7.1 QPU words, and the generation as a
//! parameter ([`Gen`], [`AnyInstr`], [`decode_gen`], [`encode_any`]) over the
//! 4.2 codec of [`super::decode`] / [`super::encode`], which is unchanged.
//!
//! The 7.1 codec has the 4.2 codec's contract: Mesa 26.2.1's
//! `v3d_qpu_instr_unpack` / `_pack` do the bit work (through
//! [`super::qpu_ffi`], with the 7.1 device), a word is accepted only if
//! Mesa's pack of Mesa's unpack gives it back ([`DecodeError::NotReproducedByMesa`]),
//! and the encoder decodes what it wrote and refuses an instruction whose
//! word reads back as another. The errors are the 4.2 codec's
//! ([`DecodeError`], [`EncodeError`]); [`EncodeError::UnpackUnmeasuredOnSilicon`]
//! is not raised on 7.1, whose FTOI* unpack is a field of its own.
//!
//! Two rules of our own, as on 4.2: a magic write to an address no 7.1
//! reference names ([`super::isa71::magic_waddr_name`]; the accumulator and
//! SFU addresses are reserved on 7.1), and a small-immediate index past the
//! table in an operand the operation reads.
//!
//! The canonical form of a 7.1 instruction: operand b of an operation with
//! fewer than two operands is [`Operand::default`] (its raddr is opcode), an
//! unread operand a keeps its address (Mesa writes it) but has no unpack, and
//! an STVPM's waddr/MA is its opcode.

use super::decode::DecodeError;
use super::encode::EncodeError;
use super::isa::{num_src, Instr};
use super::isa71::*;
use super::qpu_ffi::{self, Gen};

/// An instruction of either generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnyInstr {
    V42(Instr),
    V71(Instr71),
}

impl AnyInstr {
    pub fn gen(&self) -> Gen {
        match self {
            AnyInstr::V42(_) => Gen::V42,
            AnyInstr::V71(_) => Gen::V71,
        }
    }
}

/// Decode one word of a generation.
pub fn decode_gen(gen: Gen, w: u64) -> Result<AnyInstr, DecodeError> {
    match gen {
        Gen::V42 => super::decode::decode(w).map(AnyInstr::V42),
        Gen::V71 => decode71(w).map(AnyInstr::V71),
    }
}

/// Encode an instruction for the generation its variant names.
pub fn encode_any(instr: &AnyInstr) -> Result<u64, EncodeError> {
    match instr {
        AnyInstr::V42(i) => super::encode::encode(i),
        AnyInstr::V71(i) => encode71(i),
    }
}

/// The model's canonical form of a 7.1 instruction (see the module comment).
pub fn canonical71(instr: &Instr71) -> Instr71 {
    let Instr71::Alu(mut alu) = *instr else { return *instr };
    let add_n = num_src(add_op_args(alu.add.op));
    if add_n < 2 {
        alu.add.b = Operand::default();
    }
    if add_n < 1 {
        alu.add.a.unpack = Unpack71::None;
    }
    if matches!(alu.add.op, AddOp71::Stvpmv | AddOp71::Stvpmd | AddOp71::Stvpmp) {
        alu.add.waddr = 0;
        alu.add.magic = false;
    }
    let mul_n = num_src(mul_op_args(alu.mul.op));
    if mul_n < 2 {
        alu.mul.b = Operand::default();
    }
    if mul_n < 1 {
        alu.mul.a.unpack = Unpack71::None;
    }
    Instr71::Alu(alu)
}

/// `instr` and `back` are the same instruction: equal in canonical form, or
/// FADD/FADDNF/FMIN/FMAX with their operands the other way round (the
/// operand order is the opcode; the small-immediate flags of a and b move
/// with the operands).
pub fn same_instr71(instr: &Instr71, back: &Instr71) -> bool {
    let want = canonical71(instr);
    if want == *back {
        return true;
    }
    match want {
        Instr71::Alu(mut a) if matches!(a.add.op, AddOp71::Fadd | AddOp71::Faddnf | AddOp71::Fmin | AddOp71::Fmax) => {
            core::mem::swap(&mut a.add.a, &mut a.add.b);
            core::mem::swap(&mut a.sig.small_imm_a, &mut a.sig.small_imm_b);
            Instr71::Alu(a) == *back
        }
        _ => false,
    }
}

/// The magic write addresses of a decoded instruction are all named.
fn magic_named71(alu: &Alu71) -> bool {
    let unnamed = |waddr: u8, magic: bool| magic && magic_waddr_name(waddr).is_none();
    !(unnamed(alu.add.waddr, alu.add.magic)
        || unnamed(alu.mul.waddr, alu.mul.magic)
        || (alu.sig.writes_address() && unnamed(alu.sig_addr, alu.sig_magic)))
}

/// A small-immediate operand the operations read is inside the table.
fn small_imms_in_table(alu: &Alu71) -> bool {
    alu.read_operands().all(|(o, imm)| !imm || Alu71::small_imm(o).is_some())
}

/// Decode one V3D 7.1 word (`v3d_qpu_instr_unpack`, and the rules above).
pub fn decode71(w: u64) -> Result<Instr71, DecodeError> {
    let m = qpu_ffi::unpack71(w).ok_or(DecodeError::MesaRefuses)?;
    if !qpu_ffi::pack_preconditions_71(&m) {
        return Err(DecodeError::NotReproducedByMesa);
    }
    match qpu_ffi::pack_for(Gen::V71, &m) {
        Some(back) if back == w => {}
        _ => return Err(DecodeError::NotReproducedByMesa),
    }
    let instr = qpu_ffi::from_mesa71(&m).ok_or(DecodeError::OutsideModel)?;
    if let Instr71::Alu(a) = &instr {
        if !magic_named71(a) {
            return Err(DecodeError::MagicWaddrUnnamed);
        }
        if !small_imms_in_table(a) {
            return Err(DecodeError::SmallImmOutOfRange);
        }
    }
    Ok(canonical71(&instr))
}

/// Encode one V3D 7.1 instruction, and check that the word decodes back to it.
pub fn encode71(instr: &Instr71) -> Result<u64, EncodeError> {
    if let Instr71::Alu(a) = instr {
        if a.operands().iter().any(|(o, _)| o.raddr >= 64) {
            return Err(EncodeError::RaddrTooWide);
        }
        if a.add.waddr >= 64 || a.mul.waddr >= 64 {
            return Err(EncodeError::WaddrTooWide);
        }
        if a.sig_addr >= 64 {
            return Err(EncodeError::SigAddrTooWide);
        }
        let ldvpm = matches!(
            a.add.op,
            AddOp71::LdvpmvIn
                | AddOp71::LdvpmvOut
                | AddOp71::LdvpmdIn
                | AddOp71::LdvpmdOut
                | AddOp71::Ldvpmp
                | AddOp71::LdvpmgIn
                | AddOp71::LdvpmgOut
        );
        if ldvpm && a.add.magic {
            return Err(EncodeError::MagicNotAllowed);
        }
        if !small_imms_in_table(a) {
            return Err(EncodeError::SmallImmOutOfRange);
        }
        if !magic_named71(a) {
            return Err(EncodeError::MagicWaddrUnnamed);
        }
    }
    if let Instr71::Branch(b) = instr {
        if b.raddr_a >= 64 {
            return Err(EncodeError::RaddrTooWide);
        }
    }
    let m = qpu_ffi::to_mesa71(instr);
    if !qpu_ffi::pack_preconditions_71(&m) {
        // Only both small-immediate flags of the add ALU are left (the
        // other asserts are refused above): Mesa has no signal for it.
        return Err(EncodeError::MesaRefuses);
    }
    let w = qpu_ffi::pack_for(Gen::V71, &m).ok_or(EncodeError::MesaRefuses)?;
    let back = decode71(w).map_err(EncodeError::DoesNotDecode)?;
    if !same_instr71(instr, &back) {
        return Err(EncodeError::ReadsBackAs);
    }
    Ok(w)
}

/// The 7.1 bit layout, pinned against Mesa 26.2.1's tables. The bit work is
/// Mesa's own C (`qpu_pack.c`), so these tests do not check the packer
/// against itself: each expected value below is written out from the table it
/// names (file:line of `third_party/mesa-26.2.1/src/broadcom/qpu/qpu_pack.c`),
/// and a model variant mapped to the wrong Mesa entry fails by name.
#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// An inclusive range of a table's opcode byte (bits 31..24) or raddr.
    type R = (u8, u8);
    const ANY: &[R] = &[];

    /// `v3d71_add_ops` (qpu_pack.c:585-722): the add operation, the opcode
    /// byte range, and the raddr_b values (bits 5..0) that select it among
    /// the operations sharing the byte; empty is `ANYOPMASK`, where raddr_b
    /// is operand b.
    const ADD_TABLE: &[(AddOp71, R, &[R])] = &[
        (AddOp71::Fadd, (0, 47), ANY),
        (AddOp71::Faddnf, (0, 47), ANY),
        (AddOp71::Vfadd, (48, 52), ANY),
        (AddOp71::Add, (56, 56), ANY),
        (AddOp71::Sub, (60, 60), ANY),
        (AddOp71::Fsub, (64, 111), ANY),
        (AddOp71::Vfsub, (112, 119), ANY),
        (AddOp71::Min, (120, 120), ANY),
        (AddOp71::Max, (121, 121), ANY),
        (AddOp71::Umin, (122, 122), ANY),
        (AddOp71::Umax, (123, 123), ANY),
        (AddOp71::Shl, (124, 124), ANY),
        (AddOp71::Shr, (125, 125), ANY),
        (AddOp71::Asr, (126, 126), ANY),
        (AddOp71::Ror, (127, 127), ANY),
        (AddOp71::Fmin, (128, 175), ANY),
        (AddOp71::Fmax, (128, 175), ANY),
        (AddOp71::Vfmin, (176, 180), ANY),
        (AddOp71::And, (181, 181), ANY),
        (AddOp71::Or, (182, 182), ANY),
        (AddOp71::Xor, (183, 183), ANY),
        (AddOp71::Vadd, (184, 184), ANY),
        (AddOp71::Vsub, (185, 185), ANY),
        (AddOp71::Not, (186, 186), &[(0, 0)]),
        (AddOp71::Neg, (186, 186), &[(1, 1)]),
        (AddOp71::Flapush, (186, 186), &[(2, 2)]),
        (AddOp71::Flbpush, (186, 186), &[(3, 3)]),
        (AddOp71::Flpop, (186, 186), &[(4, 4)]),
        (AddOp71::Clz, (186, 186), &[(5, 5)]),
        (AddOp71::Setmsf, (186, 186), &[(6, 6)]),
        (AddOp71::Setrevf, (186, 186), &[(7, 7)]),
        (AddOp71::Nop, (187, 187), &[(0, 0)]),
        (AddOp71::Tidx, (187, 187), &[(1, 1)]),
        (AddOp71::Eidx, (187, 187), &[(2, 2)]),
        (AddOp71::Lr, (187, 187), &[(3, 3)]),
        (AddOp71::Vfla, (187, 187), &[(4, 4)]),
        (AddOp71::Vflna, (187, 187), &[(5, 5)]),
        (AddOp71::Vflb, (187, 187), &[(6, 6)]),
        (AddOp71::Vflnb, (187, 187), &[(7, 7)]),
        (AddOp71::Xcd, (187, 187), &[(8, 8)]),
        (AddOp71::Fxcd, (187, 187), &[(32, 34)]),
        (AddOp71::Fycd, (187, 187), &[(36, 38)]),
        (AddOp71::Ycd, (187, 187), &[(9, 9)]),
        (AddOp71::Msf, (187, 187), &[(10, 10)]),
        (AddOp71::Revf, (187, 187), &[(11, 11)]),
        (AddOp71::Iid, (187, 187), &[(12, 12)]),
        (AddOp71::Sampid, (187, 187), &[(13, 13)]),
        (AddOp71::Barrierid, (187, 187), &[(14, 14)]),
        (AddOp71::Tmuwt, (187, 187), &[(15, 15)]),
        (AddOp71::Vpmwt, (187, 187), &[(16, 16)]),
        (AddOp71::Flafirst, (187, 187), &[(17, 17)]),
        (AddOp71::Flnafirst, (187, 187), &[(18, 18)]),
        (AddOp71::SetnnmodeUu, (187, 187), &[(48, 48)]),
        (AddOp71::SetnnmodeSu, (187, 187), &[(49, 49)]),
        (AddOp71::SetnnmodeUs, (187, 187), &[(50, 50)]),
        (AddOp71::SetnnmodeSs, (187, 187), &[(51, 51)]),
        (AddOp71::LdvpmvIn, (188, 188), &[(0, 0)]),
        (AddOp71::LdvpmdIn, (188, 188), &[(1, 1)]),
        (AddOp71::Ldvpmp, (188, 188), &[(2, 2)]),
        (AddOp71::Recip, (188, 188), &[(32, 32)]),
        (AddOp71::Rsqrt, (188, 188), &[(33, 33)]),
        (AddOp71::Exp, (188, 188), &[(34, 34)]),
        (AddOp71::Log, (188, 188), &[(35, 35)]),
        (AddOp71::Sin, (188, 188), &[(36, 36)]),
        (AddOp71::Rsqrt2, (188, 188), &[(37, 37)]),
        (AddOp71::Ballot, (188, 188), &[(38, 38)]),
        (AddOp71::Bcastf, (188, 188), &[(39, 39)]),
        (AddOp71::Alleq, (188, 188), &[(40, 40)]),
        (AddOp71::Allfeq, (188, 188), &[(41, 41)]),
        (AddOp71::LdvpmgIn, (189, 189), ANY),
        (AddOp71::Stvpmv, (190, 190), ANY),
        (AddOp71::Stvpmd, (190, 190), ANY),
        (AddOp71::Stvpmp, (190, 190), ANY),
        (AddOp71::Vfmov, (191, 191), &[(0, 0)]),
        (AddOp71::Vfabs, (191, 191), &[(5, 5)]),
        (AddOp71::Vfneg, (191, 191), &[(10, 10)]),
        (AddOp71::Vfnab, (191, 191), &[(15, 15)]),
        (AddOp71::Fcmp, (192, 207), ANY),
        (AddOp71::Vfcmp, (208, 215), ANY),
        (AddOp71::Vfmax, (240, 244), ANY),
        (AddOp71::Fround, (245, 245), &[(0, 2), (4, 6), (8, 10), (12, 14)]),
        (AddOp71::Ftoin, (245, 245), &[(3, 3), (7, 7), (11, 11), (15, 15)]),
        (AddOp71::Ftrunc, (245, 245), &[(16, 18), (20, 22), (24, 26), (28, 30)]),
        (AddOp71::Ftoiz, (245, 245), &[(19, 19), (23, 23), (27, 27), (31, 31)]),
        (AddOp71::Ffloor, (245, 245), &[(32, 34), (36, 38), (40, 42), (44, 46)]),
        (AddOp71::Ftouz, (245, 245), &[(35, 35), (39, 39), (43, 43), (47, 47)]),
        (AddOp71::Fceil, (245, 245), &[(48, 50), (52, 54), (56, 58), (60, 62)]),
        (AddOp71::Ftoc, (245, 245), &[(51, 51), (55, 55), (59, 59), (63, 63)]),
        (AddOp71::Fdx, (246, 246), &[(0, 2), (4, 6), (8, 10), (12, 14)]),
        (AddOp71::Fdy, (246, 246), &[(16, 18), (20, 22), (24, 26), (28, 30)]),
        (AddOp71::Itof, (246, 246), &[(32, 34)]),
        (AddOp71::Utof, (246, 246), &[(36, 38)]),
        (AddOp71::Vpack, (247, 247), ANY),
        (AddOp71::V8pack, (248, 248), ANY),
        (AddOp71::Fmov, (249, 249), &[(0, 2), (4, 6), (8, 10), (12, 14), (16, 18), (20, 22), (24, 26)]),
        (AddOp71::Mov, (249, 249), &[(3, 3), (7, 7), (11, 11), (15, 15), (19, 19)]),
        (AddOp71::V10pack, (250, 250), ANY),
        (AddOp71::V11fpack, (251, 251), ANY),
        (AddOp71::Rotq, (252, 252), ANY),
        (AddOp71::Rot, (253, 253), ANY),
        (AddOp71::Shuffle, (254, 254), ANY),
    ];

    /// `v3d71_mul_ops` (qpu_pack.c:731-785): the mul operation, the opcode
    /// (bits 63..58) range, and the raddr_d values (bits 17..12) that select
    /// it; empty is `ANYOPMASK`.
    const MUL_TABLE: &[(MulOp71, R, &[R])] = &[
        (MulOp71::Add, (1, 1), ANY),
        (MulOp71::Sub, (2, 2), ANY),
        (MulOp71::Umul24, (3, 3), ANY),
        (MulOp71::Vfmul, (4, 8), ANY),
        (MulOp71::Smul24, (9, 9), ANY),
        (MulOp71::Multop, (10, 10), ANY),
        (MulOp71::V8dot, (11, 11), ANY),
        (MulOp71::Fmov, (14, 14), &[(0, 2), (4, 6), (8, 10), (12, 14), (16, 18), (20, 22), (24, 26), (28, 30)]),
        (MulOp71::Mov, (14, 14), &[(3, 3), (7, 7), (11, 11), (15, 15), (19, 19)]),
        (MulOp71::Ftounorm16, (14, 14), &[(32, 32)]),
        (MulOp71::Ftosnorm16, (14, 14), &[(33, 33)]),
        (MulOp71::Vftounorm8, (14, 14), &[(34, 34)]),
        (MulOp71::Vftosnorm8, (14, 14), &[(35, 35)]),
        (MulOp71::Funpackunormlo, (14, 14), &[(36, 36)]),
        (MulOp71::Funpackunormhi, (14, 14), &[(37, 37)]),
        (MulOp71::Funpacksnormlo, (14, 14), &[(38, 38)]),
        (MulOp71::Funpacksnormhi, (14, 14), &[(39, 39)]),
        (MulOp71::Vftounorm10lo, (14, 14), &[(48, 48)]),
        (MulOp71::Vftounorm10hi, (14, 14), &[(49, 49)]),
        (MulOp71::Nop, (14, 14), &[(63, 63)]),
        (MulOp71::Fmul, (16, 63), ANY),
    ];

    fn in_ranges(v: u64, r: &[R]) -> bool {
        r.is_empty() || r.iter().any(|&(lo, hi)| (lo as u64..=hi as u64).contains(&v))
    }

    /// A representative operand pair for an operation of `n` sources.
    fn operands(n: usize) -> (Operand, Operand) {
        let a = if n >= 1 { Operand { raddr: 3, unpack: Unpack71::None } } else { Operand::default() };
        let b = if n >= 2 { Operand { raddr: 4, unpack: Unpack71::None } } else { Operand::default() };
        (a, b)
    }

    fn add_instr(op: AddOp71) -> Instr71 {
        let mut a = Alu71::nop();
        let (oa, ob) = operands(num_src(add_op_args(op)));
        a.add = AddAlu71 { op, a: oa, b: ob, waddr: 2, magic: false, pack: OutPack::None };
        Instr71::Alu(a)
    }

    fn mul_instr(op: MulOp71) -> Instr71 {
        let mut a = Alu71::nop();
        let (oa, ob) = operands(num_src(mul_op_args(op)));
        a.mul = MulAlu71 { op, a: oa, b: ob, waddr: 3, magic: false, pack: OutPack::None };
        Instr71::Alu(a)
    }

    /// (a): every add operation lands in Mesa's opcode byte and raddr_b
    /// selector. The operations the table lacks are refused.
    #[test]
    fn add_opcodes_match_mesas_v71_table() {
        let mut listed = Vec::new();
        for &(op, (lo, hi), raddr) in ADD_TABLE {
            listed.push(op);
            let w = encode71(&add_instr(op)).unwrap_or_else(|e| panic!("{op:?}: {e:?}"));
            let byte = (w >> 24) & 0xff;
            assert!((lo as u64..=hi as u64).contains(&byte), "{op:?}: opcode byte {byte}");
            if !raddr.is_empty() {
                assert!(in_ranges(w & 0x3f, raddr), "{op:?}: raddr_b {}", w & 0x3f);
            }
        }
        // Mesa's v3d71_add_ops has no entry of its own for VFPACK (tested
        // below), VPMSETUP (4.x only) and the OUT forms (packed as the IN form
        // with the magic bit, which encode71 refuses).
        let unlisted: Vec<AddOp71> = AddOp71::ALL.iter().copied().filter(|o| !listed.contains(o)).collect();
        assert_eq!(
            unlisted,
            [AddOp71::Vfpack, AddOp71::Vpmsetup, AddOp71::LdvpmvOut, AddOp71::LdvpmdOut, AddOp71::LdvpmgOut]
        );
        for op in [AddOp71::Vpmsetup, AddOp71::LdvpmvOut, AddOp71::LdvpmdOut, AddOp71::LdvpmgOut] {
            assert!(encode71(&add_instr(op)).is_err(), "{op:?} has no 7.1 table entry");
        }
    }

    /// VFPACK is three opcode ranges around ADD and SUB (qpu_pack.c:589-595).
    #[test]
    fn vfpack_is_in_its_three_ranges() {
        let w = encode71(&add_instr(AddOp71::Vfpack)).expect("encodes");
        let byte = (w >> 24) & 0xff;
        assert!((53..=63).contains(&byte) && byte != 56 && byte != 60, "{byte}");
    }

    /// (a): the same for the mul operations.
    #[test]
    fn mul_opcodes_match_mesas_v71_table() {
        for &(op, (lo, hi), raddr) in MUL_TABLE {
            let w = encode71(&mul_instr(op)).unwrap_or_else(|e| panic!("{op:?}: {e:?}"));
            let code = w >> 58;
            assert!((lo as u64..=hi as u64).contains(&code), "{op:?}: opcode {code}");
            if !raddr.is_empty() {
                assert!(in_ranges((w >> 12) & 0x3f, raddr), "{op:?}: raddr_d {}", (w >> 12) & 0x3f);
            }
        }
        // MulOp71 has every table entry but UMUL24_RTOP0, which shares
        // UMUL24's opcode (qpu_pack.c:737-738) and is not a member.
        assert_eq!(MUL_TABLE.len(), MulOp71::ALL.len());
    }

    /// The 7.1 signal map (qpu_pack.c:164-197): the index of each signal.
    #[test]
    fn signal_indices_match_mesas_v71_map() {
        let none = Sig71::default();
        let s = |f: &dyn Fn(&mut Sig71)| {
            let mut x = none;
            f(&mut x);
            x
        };
        let map: [(u64, Sig71); 29] = [
            (0, none),
            (1, s(&|x| x.thrsw = true)),
            (2, s(&|x| x.ldunif = true)),
            (3, s(&|x| (x.thrsw, x.ldunif) = (true, true))),
            (4, s(&|x| x.ldtmu = true)),
            (5, s(&|x| (x.thrsw, x.ldtmu) = (true, true))),
            (6, s(&|x| (x.ldtmu, x.ldunif) = (true, true))),
            (7, s(&|x| (x.thrsw, x.ldtmu, x.ldunif) = (true, true, true))),
            (8, s(&|x| x.ldvary = true)),
            (9, s(&|x| (x.thrsw, x.ldvary) = (true, true))),
            (10, s(&|x| (x.ldvary, x.ldunif) = (true, true))),
            (11, s(&|x| (x.thrsw, x.ldvary, x.ldunif) = (true, true, true))),
            (12, s(&|x| x.ldunifrf = true)),
            (13, s(&|x| (x.thrsw, x.ldunifrf) = (true, true))),
            (14, s(&|x| x.small_imm_a = true)),
            (15, s(&|x| x.small_imm_b = true)),
            (16, s(&|x| x.ldtlb = true)),
            (17, s(&|x| x.ldtlbu = true)),
            (18, s(&|x| x.wrtmuc = true)),
            (19, s(&|x| (x.thrsw, x.wrtmuc) = (true, true))),
            (20, s(&|x| (x.ldvary, x.wrtmuc) = (true, true))),
            (21, s(&|x| (x.thrsw, x.ldvary, x.wrtmuc) = (true, true, true))),
            (22, s(&|x| x.ucb = true)),
            (24, s(&|x| x.ldunifa = true)),
            (25, s(&|x| x.ldunifarf = true)),
            (26, s(&|x| (x.ldtmu, x.wrtmuc) = (true, true))),
            (27, s(&|x| (x.thrsw, x.ldtmu, x.wrtmuc) = (true, true, true))),
            (30, s(&|x| x.small_imm_c = true)),
            (31, s(&|x| x.small_imm_d = true)),
        ];
        for (idx, sig) in &map {
            assert_eq!(qpu_ffi::sig71_unpack(*idx as u32), Some(*sig), "sig {idx}");
            // Through the whole word: a NOP carrying the signal and, where
            // the signal needs one, a small-immediate operand.
            let mut a = Alu71::nop();
            a.sig = *sig;
            let imm = Operand { raddr: 5, ..Operand::default() };
            if sig.small_imm_a {
                a.add = AddAlu71 { op: AddOp71::Mov, a: imm, waddr: 2, magic: false, ..a.add };
            }
            if sig.small_imm_b {
                a.add = AddAlu71 { op: AddOp71::Add, b: imm, waddr: 2, magic: false, ..a.add };
            }
            if sig.small_imm_c {
                a.mul = MulAlu71 { op: MulOp71::Mov, a: imm, waddr: 3, magic: false, ..a.mul };
            }
            if sig.small_imm_d {
                a.mul = MulAlu71 { op: MulOp71::Fmul, b: imm, waddr: 3, magic: false, ..a.mul };
            }
            if a.sig.writes_address() {
                a.sig_addr = 7;
            }
            let w = encode71(&Instr71::Alu(a)).unwrap_or_else(|e| panic!("sig {idx}: {e:?}"));
            assert_eq!((w >> 53) & 31, *idx, "sig {idx}: bits 57..53");
        }
        // The reserved indices (23: ROT on 4.2; 28, 29), and the ROT signal
        // the 7.1 map does not have.
        for idx in [23u32, 28, 29] {
            assert_eq!(qpu_ffi::sig71_unpack(idx), None, "sig {idx} is reserved");
        }
        let mut a = Alu71::nop();
        a.sig.rotate = true;
        assert!(encode71(&Instr71::Alu(a)).is_err(), "no ROT signal at 7.1");
    }

    /// The small-immediate table (qpu_pack.c:239-265), index by index, and
    /// where each flag's operand address sits in the word (raddr_a 11..6,
    /// raddr_b 5..0, raddr_c 23..18, raddr_d 17..12).
    #[test]
    fn small_immediates_are_mesas_table() {
        use super::super::isa::{small_imm_index, small_imm_value};
        let mut table: Vec<u32> = (0..16).collect();
        table.extend((-16i32..0).map(|v| v as u32));
        // 2^-8 .. 2^7 as f32 bits: biased exponent 119..=134.
        table.extend((119u32..135).map(|e| e << 23));
        assert_eq!(table.len(), 48);
        for (i, v) in table.iter().enumerate() {
            assert_eq!(small_imm_value(i as u8), Some(*v), "index {i}");
            assert_eq!(small_imm_index(*v), Some(i as u8), "value {v:#x}");
        }
        for i in 48u8..64 {
            assert_eq!(small_imm_value(i), None, "index {i}");
        }
        // Written out: 2^-8, 1.0, 2.0, 2^7.
        assert_eq!(small_imm_value(32), Some(0x3b80_0000));
        assert_eq!(small_imm_value(40), Some(0x3f80_0000));
        assert_eq!(small_imm_value(41), Some(0x4000_0000));
        assert_eq!(small_imm_value(47), Some(0x4300_0000));

        let imm = Operand { raddr: 41, ..Operand::default() };
        let two = Operand { raddr: 2, ..Operand::default() };
        let mut a = Alu71::nop();
        a.sig.small_imm_a = true;
        a.add = AddAlu71 { op: AddOp71::Mov, a: imm, waddr: 2, magic: false, ..a.add };
        let w = encode71(&Instr71::Alu(a)).expect("a");
        assert_eq!((w >> 6) & 63, 41, "small_imm_a: raddr_a");
        let mut a = Alu71::nop();
        a.sig.small_imm_b = true;
        a.add = AddAlu71 { op: AddOp71::Add, a: two, b: imm, waddr: 2, magic: false, ..a.add };
        let w = encode71(&Instr71::Alu(a)).expect("b");
        assert_eq!(w & 63, 41, "small_imm_b: raddr_b");
        let mut a = Alu71::nop();
        a.sig.small_imm_c = true;
        a.mul = MulAlu71 { op: MulOp71::Mov, a: imm, waddr: 3, magic: false, ..a.mul };
        let w = encode71(&Instr71::Alu(a)).expect("c");
        assert_eq!((w >> 18) & 63, 41, "small_imm_c: raddr_c");
        let mut a = Alu71::nop();
        a.sig.small_imm_d = true;
        a.mul = MulAlu71 { op: MulOp71::Fmul, a: two, b: imm, waddr: 3, magic: false, ..a.mul };
        let w = encode71(&Instr71::Alu(a)).expect("d");
        assert_eq!((w >> 12) & 63, 41, "small_imm_d: raddr_d");
    }

    /// The COND field (bits 52..46) for an ALU without an address-writing
    /// signal, from `v3d_qpu_flags_pack`'s table (qpu_pack.c:343-420).
    #[test]
    fn flags_are_mesas_cond_field() {
        // (flags, the field), written out from the table: the set of flag
        // groups present picks the high bits, the group values fill the low.
        let f = |ac: Cond, mc: Cond, apf: Pf, mpf: Pf, auf: Uf, muf: Uf| Flags { ac, mc, apf, mpf, auf, muf };
        let n = (Cond::None, Cond::None, Pf::None, Pf::None, Uf::None, Uf::None);
        let mut cases: Vec<(Flags, u64)> = vec![(f(n.0, n.1, n.2, n.3, n.4, n.5), 0)];
        for (k, pf) in [Pf::PushZ, Pf::PushN, Pf::PushC].into_iter().enumerate() {
            let k = k as u64 + 1;
            cases.push((f(n.0, n.1, pf, n.3, n.4, n.5), k)); // APF
            cases.push((f(n.0, n.1, n.2, pf, n.4, n.5), 0x10 | k)); // MPF
            cases.push((f(Cond::IfB, n.1, n.2, pf, n.4, n.5), 0x20 | (1 << 2) | k)); // AC | MPF
            cases.push((f(n.0, Cond::IfNA, pf, n.3, n.4, n.5), 0x30 | (2 << 2) | k));
            // MC | APF
        }
        for (k, uf) in Uf::ALL.iter().copied().filter(|u| *u != Uf::None).enumerate() {
            let code = k as u64 + 4; // UF - ANDZ + 4
            cases.push((f(n.0, n.1, n.2, n.3, uf, n.5), code)); // AUF
            cases.push((f(n.0, n.1, n.2, n.3, n.4, uf), 0x10 | code)); // MUF
            cases.push((f(n.0, Cond::IfNB, n.2, n.3, uf, n.5), 0x40 | (3 << 4) | code));
            // MC | AUF
        }
        for (k, c) in [Cond::IfA, Cond::IfB, Cond::IfNA, Cond::IfNB].into_iter().enumerate() {
            let k = k as u64;
            cases.push((f(c, n.1, n.2, n.3, n.4, n.5), 0x20 | (k << 2))); // AC
            cases.push((f(n.0, c, n.2, n.3, n.4, n.5), 0x30 | (k << 2))); // MC
            cases.push((f(c, Cond::IfNA, n.2, n.3, n.4, n.5), 0x40 | (2 << 4) | k));
            // MC | AC
        }
        for (fl, want) in &cases {
            let mut a = Alu71::nop();
            a.add = AddAlu71 {
                op: AddOp71::Fadd,
                a: Operand { raddr: 3, ..Operand::default() },
                b: Operand { raddr: 4, ..Operand::default() },
                waddr: 2,
                magic: false,
                pack: OutPack::None,
            };
            a.mul = MulAlu71 {
                op: MulOp71::Fmul,
                a: Operand { raddr: 5, ..Operand::default() },
                b: Operand { raddr: 6, ..Operand::default() },
                waddr: 3,
                magic: false,
                pack: OutPack::None,
            };
            a.flags = *fl;
            let w = encode71(&Instr71::Alu(a)).unwrap_or_else(|e| panic!("{fl:?}: {e:?}"));
            assert_eq!((w >> 46) & 0x7f, *want, "{fl:?}");
            assert_eq!(decode71(w), Ok(Instr71::Alu(a)), "{fl:?}");
        }
        // Flag combinations the table lacks are refused (ac with auf, apf
        // with mpf, ...).
        for fl in [
            f(Cond::IfA, n.1, n.2, n.3, Uf::AndZ, n.5),
            f(n.0, n.1, Pf::PushZ, Pf::PushN, n.4, n.5),
            f(n.0, n.1, n.2, n.3, Uf::AndZ, Uf::AndN),
            f(Cond::IfA, Cond::IfB, Pf::PushZ, n.3, n.4, n.5),
        ] {
            let mut a = Alu71::nop();
            a.flags = fl;
            assert!(encode71(&Instr71::Alu(a)).is_err(), "{fl:?}");
        }
    }

    /// The branch fields (qpu_pack.c:63-102, 2571-2619): the word of a branch
    /// built bit by bit from them, and what Mesa's unpack refuses.
    #[test]
    fn branch_fields_are_mesas() {
        let conds = [
            (BranchCond::Always, 0u64),
            (BranchCond::A0, 2),
            (BranchCond::NA0, 3),
            (BranchCond::AllA, 4),
            (BranchCond::AnyNA, 5),
            (BranchCond::AnyA, 6),
            (BranchCond::AllNA, 7),
        ];
        for (cond, code) in conds {
            for (msf, mcode) in [(Msfign::None, 0u64), (Msfign::P, 1), (Msfign::Q, 2)] {
                for bdi in BranchDest::ALL {
                    for ub in [false, true] {
                        for bdu in BranchDest::ALL {
                            if !ub && bdu != BranchDest::Abs {
                                continue;
                            }
                            let reads_raddr = bdi == BranchDest::Regfile || (ub && bdu == BranchDest::Regfile);
                            let has_off = matches!(bdi, BranchDest::Abs | BranchDest::Rel);
                            let b = Branch {
                                cond,
                                msfign: msf,
                                bdi,
                                ub,
                                bdu,
                                raddr_a: if reads_raddr { 37 } else { 0 },
                                offset: if has_off { 0x0123_4560 } else { 0 },
                            };
                            let mut want = 16u64 << 53; // SIG = 16 (bits 57..53); OP_MUL = 0
                            want |= code << 32;
                            want |= mcode << 21;
                            want |= (bdi as u64) << 12;
                            if ub {
                                want |= 1 << 14;
                                want |= (bdu as u64) << 15;
                            }
                            if reads_raddr {
                                want |= 37 << 6;
                            }
                            if has_off {
                                let off = b.offset as u64;
                                want |= ((off & 0x00ff_ffff) >> 3) << 35; // ADDR_LOW, bits 55..35
                                want |= (off >> 24) << 24; // ADDR_HIGH, bits 31..24
                            }
                            assert_eq!(encode71(&Instr71::Branch(b)), Ok(want), "{b:?}");
                            assert_eq!(decode71(want), Ok(Instr71::Branch(b)), "{b:?}");
                        }
                    }
                }
            }
        }
        // Refused: branch condition 1 (between ALWAYS and A0), MSFIGN 3, and
        // OP_MUL 0 with a signal field that is not 16..23.
        let base = 16u64 << 53;
        assert!(decode71(base | (1 << 32)).is_err(), "cond 1");
        assert!(decode71(base | (3 << 21)).is_err(), "msfign 3");
        for sig in [0u64, 1, 8, 15, 24, 31] {
            assert!(decode71(sig << 53).is_err(), "OP_MUL 0 with sig {sig} is not a branch");
        }
    }

    static CORPUS: &str = include_str!("../../../../tools/v3d/qpu_corpus_71.txt");

    fn corpus() -> Vec<u64> {
        CORPUS.lines().filter_map(|l| l.strip_prefix("0x")).map(|h| u64::from_str_radix(h, 16).unwrap()).collect()
    }

    /// The add operations Mesa's table allows for a word's opcode byte and
    /// raddr_b, and the mul operations for its opcode and raddr_d.
    fn table_candidates(w: u64) -> (Vec<AddOp71>, Vec<MulOp71>) {
        let (byte, rb, code, rd) = ((w >> 24) & 0xff, w & 0x3f, w >> 58, (w >> 12) & 0x3f);
        let mut add: Vec<AddOp71> = ADD_TABLE
            .iter()
            .filter(|(_, (lo, hi), r)| (*lo as u64..=*hi as u64).contains(&byte) && in_ranges(rb, r))
            .map(|(op, _, _)| *op)
            .collect();
        if matches!(byte, 53..=55 | 57..=59 | 61..=63) {
            add.push(AddOp71::Vfpack);
        }
        let mul = MUL_TABLE
            .iter()
            .filter(|(_, (lo, hi), r)| (*lo as u64..=*hi as u64).contains(&code) && in_ranges(rd, r))
            .map(|(op, _, _)| *op)
            .collect();
        (add, mul)
    }

    /// (a): literal words of `tools/v3d/qpu_corpus_71.txt` (Mesa's own output
    /// for 7.1), one per opcode class and per signal, decode to an operation
    /// Mesa's table allows for the word's bits and encode back to the word.
    #[test]
    fn literal_corpus_words_decode_as_the_tables_say() {
        let words: [u64; 41] = [
            // opcode classes, then one word per signal index of the corpus.
            0x0400_0710_0569_b20a,
            0x0400_0042_7d79_f757,
            0x3800_0349_8530_329e,
            0x0400_005e_b76d_d24a,
            0x0400_0114_ba1c_3441,
            0x0400_2303_bb24_200f,
            0x0440_0781_bc71_d000,
            0x3800_2180_be03_f001,
            0x5400_5246_c018_375e,
            0x5440_9246_c110_60c0,
            0x3840_d186_c478_3002,
            0x3800_7186_c503_f344,
            0x7400_9506_c929_0300,
            0x0400_00c8_f504_2157,
            0x0400_038c_f675_41e4,
            0x0c00_021a_f964_0303,
            0x3800_1247_3500_3147,
            0x0400_0081_3874_0700,
            0x07e0_05d4_4518_1352,
            0x3884_6193_4903_f406,
            0x0200_0008_0000_d000,
            0x0420_0454_f630_f420,
            0x0440_0103_387c_0780,
            0x0460_0389_381c_c1cb,
            0x0480_a304_0525_1058,
            0x04a3_e31d_0524_408d,
            0x04c0_4784_7c6d_0015,
            0x3901_2190_bb03_f00b,
            0x3921_3186_bb03_f000,
            0x0582_6308_0525_209e,
            0x05a5_630d_0525_f15a,
            0x05c0_d206_7810_301e,
            0x05e0_005f_b515_95c1,
            0x3a40_0142_410d_83dd,
            0x3a60_218b_f503_f257,
            0x3a81_3186_bb03_f000,
            0x3b00_2186_0503_f18f,
            0x3b21_e190_bc03_f000,
            0x3b41_b886_bb14_3000,
            0x0bc0_035b_7d7c_748a,
            0x07e0_02c5_f528_1197,
        ];
        let mut sigs = Vec::new();
        let corpus = corpus();
        for w in words {
            assert!(corpus.contains(&w), "{w:#018x} is not a corpus word");
            let decoded = decode71(w).unwrap_or_else(|e| panic!("{w:#018x}: {e:?}"));
            assert_eq!(encode71(&decoded), Ok(w), "{w:#018x}");
            let Instr71::Alu(a) = decoded else { continue };
            let (adds, muls) = table_candidates(w);
            assert!(adds.contains(&a.add.op), "{w:#018x}: add {:?} not in {adds:?}", a.add.op);
            assert!(muls.contains(&a.mul.op), "{w:#018x}: mul {:?} not in {muls:?}", a.mul.op);
            sigs.push((w >> 53) & 31);
        }
        assert_eq!(sigs.len(), 40, "every word but the branch is an ALU word");
    }

    /// (a): the same cross-check over every word of the 7.1 corpus: the
    /// decoded add and mul operation is one Mesa's table allows for its bits,
    /// and the signal index is the one the 7.1 map gives the decoded signals.
    #[test]
    fn every_corpus_word_agrees_with_the_tables() {
        let mut n = 0;
        for w in corpus() {
            let Ok(Instr71::Alu(a)) = decode71(w) else { continue };
            let (adds, muls) = table_candidates(w);
            assert!(adds.contains(&a.add.op), "{w:#018x}: add {:?} not in {adds:?}", a.add.op);
            assert!(muls.contains(&a.mul.op), "{w:#018x}: mul {:?} not in {muls:?}", a.mul.op);
            assert_eq!(qpu_ffi::sig71_unpack(((w >> 53) & 31) as u32), Some(a.sig), "{w:#018x}: signal");
            n += 1;
        }
        assert!(n > 9000, "{n}");
    }

    /// (b): unpack(pack(x)) == x for every operation, with representative
    /// operands: each input unpack, each output pack and a few destinations,
    /// accepted ones only (an encoder refusal is a refusal, not a mismatch),
    /// and every operation has at least one accepted form.
    #[test]
    fn every_operation_round_trips_with_representative_operands() {
        let ops = AddOp71::ALL.iter().map(|&o| (Some(o), None)).chain(MulOp71::ALL.iter().map(|&o| (None, Some(o))));
        let mut accepted = 0usize;
        for (add, mul) in ops {
            let mut ok_any = false;
            let base = match (add, mul) {
                (Some(o), _) => add_instr(o),
                (_, Some(o)) => mul_instr(o),
                _ => unreachable!(),
            };
            let Instr71::Alu(base) = base else { unreachable!() };
            for u in Unpack71::ALL {
                for pack in OutPack::ALL {
                    for (waddr, magic) in [(2u8, false), (60, false), (WADDR_NOP, true), (9, true)] {
                        let mut a = base;
                        if add.is_some() {
                            a.add.a.unpack = u;
                            a.add.b.unpack = u;
                            a.add.pack = pack;
                            a.add.waddr = waddr;
                            a.add.magic = magic;
                        } else {
                            a.mul.a.unpack = u;
                            a.mul.b.unpack = u;
                            a.mul.pack = pack;
                            a.mul.waddr = waddr;
                            a.mul.magic = magic;
                        }
                        let i = Instr71::Alu(a);
                        let Ok(w) = encode71(&i) else { continue };
                        let back = decode71(w).unwrap_or_else(|e| panic!("{i:?}: {w:#018x}: {e:?}"));
                        assert!(same_instr71(&i, &back), "{i:?} -> {w:#018x} -> {back:?}");
                        assert_eq!(encode71(&back), Ok(w), "{i:?}");
                        accepted += 1;
                        ok_any = true;
                    }
                }
            }
            let listed = add.is_some_and(|o| ADD_TABLE.iter().any(|e| e.0 == o) || o == AddOp71::Vfpack)
                || mul.is_some_and(|o| MUL_TABLE.iter().any(|e| e.0 == o));
            assert_eq!(ok_any, listed, "{add:?} {mul:?}: encodable exactly when Mesa's table lists it");
        }
        assert!(accepted > 1000, "{accepted}");
    }

    /// (c): words whose opcode, selector or field Mesa's 7.1 tables have no
    /// entry for are refused, from a NOP word with the field replaced.
    #[test]
    fn unpack_refuses_what_mesas_tables_refuse() {
        let nop = encode71(&Instr71::Alu(Alu71::nop())).expect("nop");
        let with_add = |byte: u64, rb: u64| (nop & !(0xff << 24) & !0x3f) | (byte << 24) | rb;
        let with_mul = |code: u64, rd: u64| (nop & !(0x3f << 58) & !(0x3f << 12)) | (code << 58) | (rd << 12);
        let refused = |w: u64, what: &str| assert!(decode71(w).is_err(), "{what}: {w:#018x} decodes");
        assert_eq!(decode71(with_add(187, 0)), Ok(Instr71::Alu(Alu71::nop())));
        // add: opcode bytes no entry covers (216..=239, 255), and selectors
        // outside the entries of the shared bytes.
        for byte in (216..=239).chain([255]) {
            refused(with_add(byte, 0), "add opcode gap");
        }
        for rb in 8..64 {
            refused(with_add(186, rb), "186 selector");
        }
        for rb in (19..32).chain(39..48).chain(52..64) {
            refused(with_add(187, rb), "187 selector");
        }
        for rb in (3..32).chain(42..64) {
            refused(with_add(188, rb), "188 selector");
        }
        for rb in (1..16).filter(|r| ![5, 10, 15].contains(r)).chain(16..64) {
            refused(with_add(191, rb), "191 selector");
        }
        // mul: 0 is a branch or nothing, 12, 13 and 15 have no entry, and
        // opcode 14's selectors outside its entries.
        for code in [12u64, 13, 15] {
            refused(with_mul(code, 0), "mul opcode gap");
        }
        for rd in (40..48).chain(50..63) {
            refused(with_mul(14, rd), "mul 14 selector");
        }
        refused(nop & !(0x3f << 58), "OP_MUL 0 with an ALU signal");
        // The reserved signals, and a cond field Mesa's unpack refuses
        // (0x10: qpu_pack.c:323).
        for sig in [23u64, 28, 29] {
            refused((nop & !(31 << 53)) | (sig << 53), "reserved signal");
        }
        refused((nop & !(0x7f << 46)) | (0x10 << 46), "cond 0x10");
        // A write address Mesa names nothing at (25..31) is refused by us.
        let mut a = Alu71::nop();
        a.add = AddAlu71 { waddr: 28, magic: true, ..a.add };
        assert!(encode71(&Instr71::Alu(a)).is_err());
    }
}
