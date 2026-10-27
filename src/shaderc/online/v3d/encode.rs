// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Writing an [`Instr`] as a V3D 4.2 QPU word: Mesa's `v3d_qpu_instr_pack`
//! ([`super::qpu_ffi`]), checked.
//!
//! Where Mesa's packer silently drops a value its model holds (a pack on an
//! operation that has none, raddr_a on a branch that reads no register, a
//! branch offset that is not a multiple of 8, `sig_addr` without a signal
//! that writes one), this encoder refuses: every word is decoded again
//! ([`super::decode::decode`], itself Mesa's unpack plus the strict rules)
//! and an instruction whose word reads back as anything else is refused as
//! [`EncodeError::ReadsBackAs`] (VPMSETUP from r0 is FLNAFIRST's word;
//! FADDNF of two identical operands is FADD's). Before Mesa is called, the
//! values its packer asserts on rather than refuses are refused here, and
//! the three rules of our own that are stricter than Mesa
//! ([`EncodeError::MagicWaddrUnnamed`], [`EncodeError::SmallImmOutOfRange`],
//! [`EncodeError::UnpackUnmeasuredOnSilicon`]) are applied.

use super::decode::{decode, is_ftoi, magic_named, same_instr, DecodeError};
use super::isa::*;
use super::qpu_ffi;

/// Why an instruction is not encoded. One variant per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// raddr_a or raddr_b does not fit 6 bits (Mesa asserts).
    RaddrTooWide,
    /// A waddr does not fit 6 bits (Mesa asserts).
    WaddrTooWide,
    /// `sig_addr` does not fit 6 bits (Mesa asserts).
    SigAddrTooWide,
    /// `magic` on an LDVPM* (the MA bit is its IN/OUT; Mesa asserts).
    MagicNotAllowed,
    /// A magic write, or a signal's magic address, to an address no
    /// reference names (see [`magic_waddr_name`]). Mesa packs it.
    MagicWaddrUnnamed,
    /// raddr_b is past the small-immediate table under `small_imm_b`. Mesa
    /// packs it; its disassembler asserts.
    SmallImmOutOfRange,
    /// An input unpack other than NONE on FTOIN/FTOIZ/FTOUZ/FTOC: which bits
    /// the hardware reads as L and as H is not measured on silicon.
    /// Mesa's decoder reads opcode 249/250 as L and 253/254 as H
    /// (qpu_pack.c:1062-1065, 1150-1160). Mesa's encoder ORs the unpack into
    /// opcode 245/246, whose bit 2 is already set (`opcode |= packed << 2`,
    /// qpu_pack.c:1795), so it writes 253/254 for L and H alike, and its
    /// compiler asks for L on every 16-bit source (nir_to_vir.c:1459-1467):
    /// on the board, 253/254 runs where Mesa means L. The corpus has none
    /// of these unpacks (178 of 178 words are NONE), and the ES 2.0 compiler
    /// never needs an f16 unpack on these operations. Refused until a board
    /// run feeds an f16 value with different halves through 249/250 and
    /// 253/254; the waiting test is
    /// `roundtrip::ftoi_unpacks_decode_but_are_refused_until_measured`.
    UnpackUnmeasuredOnSilicon,
    /// Mesa's `v3d_qpu_instr_pack` refuses the instruction (a signal
    /// combination, a flags combination, a pack or an unpack the operation
    /// has no encoding for; flags with an address-writing signal).
    MesaRefuses,
    /// The word decodes as a different instruction (Mesa dropped a value).
    ReadsBackAs,
    /// The word Mesa wrote does not decode (only a defect could do this).
    DoesNotDecode(DecodeError),
}

/// Encode one instruction, and check that the word decodes back to it.
pub fn encode(instr: &Instr) -> Result<u64, EncodeError> {
    if let Instr::Alu(a) = instr {
        if a.raddr_a >= 64 || a.raddr_b >= 64 {
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
            AddOp::LdvpmvIn
                | AddOp::LdvpmvOut
                | AddOp::LdvpmdIn
                | AddOp::LdvpmdOut
                | AddOp::Ldvpmp
                | AddOp::LdvpmgIn
                | AddOp::LdvpmgOut
        );
        if ldvpm && a.add.magic {
            return Err(EncodeError::MagicNotAllowed);
        }
        if a.sig.small_imm_b && small_imm_value(a.raddr_b).is_none() {
            return Err(EncodeError::SmallImmOutOfRange);
        }
        if !magic_named(a) {
            return Err(EncodeError::MagicWaddrUnnamed);
        }
        if is_ftoi(a.add.op) && matches!(a.add.a.unpack, Unpack::L | Unpack::H) {
            return Err(EncodeError::UnpackUnmeasuredOnSilicon);
        }
    }
    if let Instr::Branch(b) = instr {
        if b.raddr_a >= 64 {
            return Err(EncodeError::RaddrTooWide);
        }
    }
    let m = qpu_ffi::to_mesa(instr);
    debug_assert!(qpu_ffi::pack_preconditions(&m), "the checks above are Mesa's asserts");
    let w = qpu_ffi::pack(&m).ok_or(EncodeError::MesaRefuses)?;
    let back = decode(w).map_err(EncodeError::DoesNotDecode)?;
    if !same_instr(instr, &back) {
        return Err(EncodeError::ReadsBackAs);
    }
    Ok(w)
}

/// Why a program was not encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramEncodeError {
    /// `out` has room for fewer words than there are instructions.
    OutTooSmall,
    /// Instruction `.0` does not encode.
    Instr(usize, EncodeError),
}

/// Encode a program into `out`, which must have room for every instruction;
/// the number of words written. Nothing is partial: on an error the caller
/// must not use `out`.
pub fn encode_program(instrs: &[Instr], out: &mut [u64]) -> Result<usize, ProgramEncodeError> {
    if out.len() < instrs.len() {
        return Err(ProgramEncodeError::OutTooSmall);
    }
    for (i, instr) in instrs.iter().enumerate() {
        out[i] = encode(instr).map_err(|e| ProgramEncodeError::Instr(i, e))?;
    }
    Ok(instrs.len())
}
