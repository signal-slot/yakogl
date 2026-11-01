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
