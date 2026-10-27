// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Decoding a V3D 4.2 QPU word into an [`Instr`]: Mesa's
//! `v3d_qpu_instr_unpack` ([`super::qpu_ffi`]), and the rules by which this
//! decoder is stricter than Mesa.
//!
//! Mesa's model has no place for a few bit patterns its unpack accepts (an
//! STVPM with the MA bit, a branch's unused bits, FXCD's pack variants), so
//! for them Mesa's own pack(unpack(w)) is not w. This decoder refuses those
//! words instead of dropping the bits: it packs what Mesa unpacked, with
//! Mesa's packer, and accepts the word only if that gives the word back
//! ([`DecodeError::NotReproducedByMesa`]). So every word it accepts encodes
//! back to itself (`roundtrip::every_accepted_random_word_encodes_back`), with
//! one named exception, the FTOI policy below.
//!
//! Beyond pack(unpack(w)) == w, three rules of our own (the reason each is
//! stricter than Mesa is at its variant):
//!
//! * [`DecodeError::MagicWaddrUnnamed`]: a magic write to an address no
//!   reference names.
//! * [`DecodeError::SmallImmOutOfRange`]: a small-immediate index past the
//!   table.
//! * The FTOI policy: FTOIN/FTOIZ/FTOUZ/FTOC with an L or H input unpack
//!   (opcodes 249/250 and 253/254) decode as Mesa's unpack reads them, even
//!   the L words Mesa's pack cannot give back (it writes 253/254 for L and H
//!   alike), and the encoder refuses all of them
//!   ([`super::encode::EncodeError::UnpackUnmeasuredOnSilicon`]).
//!
//! And the model's canonical form: an operand the operation does not read is
//! [`Input::default`] (its mux bits are part of the opcode, which Mesa's
//! pack writes back from the operation), and an STVPM's waddr/MA is its
//! opcode (waddr 0, not magic in the model).

use super::isa::*;
use super::qpu_ffi;

/// Why a word is not decoded. One variant per cause, so a histogram over a
/// corpus keys on the cause, never on wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DecodeError {
    /// Mesa's `v3d_qpu_instr_unpack` refuses the word (not an ALU word or a
    /// branch, a reserved signal or flags value, no table entry for the
    /// opcode and muxes, an STVPM waddr past 2, branch msfign 3).
    MesaRefuses,
    /// Mesa unpacks the word, but its pack of what it unpacked is another
    /// word (or refuses it, or would assert on it): the word holds bits
    /// Mesa's model drops (an STVPM or LDVPMP with MA, FXCD/FYCD's pack
    /// variants, branch condition 1, a branch's unused bits, bdu without
    /// ub, raddr_a or an offset a branch does not read, an output pack 3).
    NotReproducedByMesa,
    /// Mesa gives back the word, from a value the 4.2 model has no variant
    /// for: a uniform branch's bdu 4..7 (Mesa stores the raw field).
    OutsideModel,
    /// A magic write (add or mul waddr, or a signal's address) to an address
    /// no reference names: 25..31, 47..54, 56..63. Mesa packs it back; its
    /// disassembler prints "UNKNOWN" for the first two ranges and reads past
    /// the end of its name table for the third.
    MagicWaddrUnnamed,
    /// The signal says raddr_b is a small immediate and raddr_b is 48..63,
    /// past the table. Mesa's unpack accepts it; its disassembler asserts.
    SmallImmOutOfRange,
}

/// The FTOI* ops, whose L/H unpack the decoder reads and the encoder refuses.
pub(super) fn is_ftoi(op: AddOp) -> bool {
    matches!(op, AddOp::Ftoin | AddOp::Ftoiz | AddOp::Ftouz | AddOp::Ftoc)
}

/// The add opcode's bits 2..3 (word bits 26..27): where Mesa's unpack reads
/// an FTOI* input unpack (qpu_pack.c:1058-1065: 249/250 and 253/254 are
/// 245/246 with an unpack). The FTOI policy compares the rest of the word.
const FTOI_UNPACK_BITS: u64 = 0b11 << 26;

/// The model's canonical form of an instruction: unread operands cleared,
/// an STVPM's waddr and MA (its opcode) cleared. [`decode`] returns this
/// form, and [`super::encode::encode`] compares its word's decode with it.
pub fn canonical(instr: &Instr) -> Instr {
    let Instr::Alu(mut alu) = *instr else { return *instr };
    let add_n = num_src(add_op_args(alu.add.op));
    if add_n < 2 {
        alu.add.b = Input::default();
    }
    if add_n < 1 {
        alu.add.a = Input::default();
    }
    if matches!(alu.add.op, AddOp::Stvpmv | AddOp::Stvpmd | AddOp::Stvpmp) {
        alu.add.waddr = 0;
        alu.add.magic = false;
    }
    let mul_n = num_src(mul_op_args(alu.mul.op));
    if mul_n < 2 {
        alu.mul.b = Input::default();
    }
    if mul_n < 1 {
        alu.mul.a = Input::default();
    }
    Instr::Alu(alu)
}

/// `instr` and `back` are the same instruction: equal in canonical form, or
/// FADD/FADDNF/FMIN/FMAX with their two operands the other way round (Mesa's
/// pack orders them, since the order is what tells FADD from FADDNF and FMIN
/// from FMAX; the operations are commutative).
pub fn same_instr(instr: &Instr, back: &Instr) -> bool {
    let want = canonical(instr);
    if want == *back {
        return true;
    }
    match want {
        Instr::Alu(mut a) if matches!(a.add.op, AddOp::Fadd | AddOp::Faddnf | AddOp::Fmin | AddOp::Fmax) => {
            core::mem::swap(&mut a.add.a, &mut a.add.b);
            Instr::Alu(a) == *back
        }
        _ => false,
    }
}

/// The magic write addresses of a decoded instruction are all named.
pub(super) fn magic_named(alu: &Alu) -> bool {
    let unnamed = |waddr: u8, magic: bool| magic && magic_waddr_name(waddr).is_none();
    !(unnamed(alu.add.waddr, alu.add.magic)
        || unnamed(alu.mul.waddr, alu.mul.magic)
        || (alu.sig.writes_address() && unnamed(alu.sig_addr, alu.sig_magic)))
}

/// Decode one word (`v3d_qpu_instr_unpack`, and the rules above).
pub fn decode(w: u64) -> Result<Instr, DecodeError> {
    let m = qpu_ffi::unpack(w).ok_or(DecodeError::MesaRefuses)?;
    let instr = qpu_ffi::from_mesa(&m);
    // The FTOI policy: an FTOI* L/H word is compared with Mesa's pack of its
    // NONE form outside the two bits that carry the unpack.
    let ftoi_unpack = matches!(instr, Some(Instr::Alu(a))
        if is_ftoi(a.add.op) && matches!(a.add.a.unpack, Unpack::L | Unpack::H));
    let (reference, mask) = if ftoi_unpack {
        let Some(Instr::Alu(mut a)) = instr else { unreachable!() };
        a.add.a.unpack = Unpack::None;
        (qpu_ffi::to_mesa(&Instr::Alu(a)), !FTOI_UNPACK_BITS)
    } else {
        (m, !0)
    };
    if !qpu_ffi::pack_preconditions(&reference) {
        return Err(DecodeError::NotReproducedByMesa);
    }
    match qpu_ffi::pack(&reference) {
        Some(back) if (back ^ w) & mask == 0 => {}
        _ => return Err(DecodeError::NotReproducedByMesa),
    }
    let instr = instr.ok_or(DecodeError::OutsideModel)?;
    if let Instr::Alu(a) = &instr {
        if !magic_named(a) {
            return Err(DecodeError::MagicWaddrUnnamed);
        }
        if a.sig.small_imm_b && small_imm_value(a.raddr_b).is_none() {
            return Err(DecodeError::SmallImmOutOfRange);
        }
    }
    Ok(canonical(&instr))
}

/// Walk a whole program, calling `f` with each instruction and its index.
/// Stops at the first word that does not decode and says which.
pub fn for_each_instr<E>(
    code: &[u64],
    mut f: impl FnMut(usize, &Instr) -> Result<(), E>,
) -> Result<usize, ProgramError<E>> {
    for (i, &w) in code.iter().enumerate() {
        let instr = decode(w).map_err(|e| ProgramError::Decode(i, e))?;
        f(i, &instr).map_err(ProgramError::Visit)?;
    }
    Ok(code.len())
}

/// Why a program walk stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramError<E> {
    /// Instruction `.0` does not decode.
    Decode(usize, DecodeError),
    /// The visitor refused.
    Visit(E),
}
