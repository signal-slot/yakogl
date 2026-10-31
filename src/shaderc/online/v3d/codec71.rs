// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Decoding and encoding V3D 7.1 QPU words ([`super::isa71`]): the 7.1 twin
//! of [`super::decode`] and [`super::encode`], through Mesa's
//! `v3d_qpu_instr_unpack` / `v3d_qpu_instr_pack` with the 7.1 device
//! ([`super::qpu_ffi`], [`Gen::V71`]).
//!
//! The rules are the 4.2 ones, with the same error types. The decoder
//! accepts a word only if Mesa's pack of what Mesa unpacked gives the word
//! back (so every word it accepts encodes back to itself), and refuses a
//! magic write to an address no reference names and a small immediate past
//! the table. The encoder refuses what Mesa's packer asserts on or silently
//! drops, and checks that its word decodes to the instruction it was given.
//!
//! What is different from 4.2 is the mirror: an operand is a register
//! address (`struct v3d_qpu_input`'s `raddr`, which shares its storage with
//! the 4.x `mux`), the word-level `raddr_a`/`raddr_b` are not written
//! (Mesa's 7.1 packer takes the addresses from the operands), and the
//! signal map is the 7.1 one.

use super::decode::DecodeError;
use super::encode::EncodeError;
use super::isa71::*;
use super::qpu_ffi::{
    self, branch_from_mesa, branch_to_mesa, find, Gen, MAlu, MAluHalf, MBody, MFlags, MInput, MInstr, MSig,
    INSTR_TYPE_ALU, INSTR_TYPE_BRANCH,
};

const G: Gen = Gen::V71;

// The bit of each field of `struct v3d_qpu_sig` (pinned against C by
// qpu_ffi's tests): declaration order, `ldvpm` (7) is not on 7.1.
const SIG_FIELD_ORDER: usize = 17;
const SIG_LDVPM: usize = 7;

pub(super) fn sig_to_mesa(s: &Sig71) -> MSig {
    let mut m = MSig::default();
    let v: [bool; SIG_FIELD_ORDER] = [
        s.thrsw,
        s.ldunif,
        s.ldunifa,
        s.ldunifrf,
        s.ldunifarf,
        s.ldtmu,
        s.ldvary,
        false,
        s.ldtlb,
        s.ldtlbu,
        s.ucb,
        s.rotate,
        s.wrtmuc,
        s.small_imm_a,
        s.small_imm_b,
        s.small_imm_c,
        s.small_imm_d,
    ];
    for (i, x) in v.into_iter().enumerate() {
        m.set(i, x);
    }
    m
}

fn sig_from_mesa(m: &MSig) -> Option<Sig71> {
    if m.get(SIG_LDVPM) {
        return None;
    }
    let g = |i| m.get(i);
    Some(Sig71 {
        thrsw: g(0),
        ldunif: g(1),
        ldunifa: g(2),
        ldunifrf: g(3),
        ldunifarf: g(4),
        ldtmu: g(5),
        ldvary: g(6),
        ldtlb: g(8),
        ldtlbu: g(9),
        ucb: g(10),
        rotate: g(11),
        wrtmuc: g(12),
        small_imm_a: g(13),
        small_imm_b: g(14),
        small_imm_c: g(15),
        small_imm_d: g(16),
    })
}

fn input_to_mesa(i: &Input71) -> MInput {
    // `raddr` is the first byte of the union it shares with `mux`; the rest
    // of the four bytes stays zero.
    MInput { mux: i.raddr as u32, unpack: i.unpack as u32 }
}

fn input_from_mesa(m: &MInput) -> Option<Input71> {
    Some(Input71 { raddr: (m.mux & 0xff) as u8, unpack: find(&Unpack71::ALL, m.unpack, |x| x as u32)? })
}

/// The model as Mesa's struct. Total: every model value is a Mesa value.
pub fn to_mesa(instr: &Instr71) -> MInstr {
    let mut m = MInstr::zeroed();
    match instr {
        Instr71::Alu(a) => {
            m.type_ = INSTR_TYPE_ALU;
            m.sig = sig_to_mesa(&a.sig);
            m.sig_addr = a.sig_addr;
            m.sig_magic = a.sig_magic;
            let f = &a.flags;
            m.flags = MFlags {
                ac: f.ac as u32,
                mc: f.mc as u32,
                apf: f.apf as u32,
                mpf: f.mpf as u32,
                auf: f.auf as u32,
                muf: f.muf as u32,
            };
            m.body = MBody {
                alu: MAlu {
                    add: MAluHalf {
                        op: a.add.op as u32,
                        a: input_to_mesa(&a.add.a),
                        b: input_to_mesa(&a.add.b),
                        waddr: a.add.waddr,
                        magic_write: a.add.magic,
                        output_pack: a.add.pack as u32,
                    },
                    mul: MAluHalf {
                        op: a.mul.op as u32,
                        a: input_to_mesa(&a.mul.a),
                        b: input_to_mesa(&a.mul.b),
                        waddr: a.mul.waddr,
                        magic_write: a.mul.magic,
                        output_pack: a.mul.pack as u32,
                    },
                },
            };
        }
        Instr71::Branch(b) => branch_to_mesa(b, &mut m),
    }
    m
}

/// Mesa's struct as the model, as it stands (no canonical form applied),
/// `None` for a value the model has no variant for.
pub fn from_mesa(m: &MInstr) -> Option<Instr71> {
    match m.type_ {
        INSTR_TYPE_ALU => {
            // SAFETY: an ALU instruction's body is `alu`.
            let alu = unsafe { m.body.alu };
            let f = &m.flags;
            Some(Instr71::Alu(Alu71 {
                sig: sig_from_mesa(&m.sig)?,
                sig_addr: m.sig_addr,
                sig_magic: m.sig_magic,
                flags: Flags {
                    ac: find(&Cond::ALL, f.ac, |x| x as u32)?,
                    mc: find(&Cond::ALL, f.mc, |x| x as u32)?,
                    apf: find(&Pf::ALL, f.apf, |x| x as u32)?,
                    mpf: find(&Pf::ALL, f.mpf, |x| x as u32)?,
                    auf: find(&Uf::ALL, f.auf, |x| x as u32)?,
                    muf: find(&Uf::ALL, f.muf, |x| x as u32)?,
                },
                add: AddAlu71 {
                    op: find(&AddOp71::ALL, alu.add.op, |x| x as u32)?,
                    a: input_from_mesa(&alu.add.a)?,
                    b: input_from_mesa(&alu.add.b)?,
                    waddr: alu.add.waddr,
                    magic: alu.add.magic_write,
                    pack: find(&OutPack::ALL, alu.add.output_pack, |x| x as u32)?,
                },
                mul: MulAlu71 {
                    op: find(&MulOp71::ALL, alu.mul.op, |x| x as u32)?,
                    a: input_from_mesa(&alu.mul.a)?,
                    b: input_from_mesa(&alu.mul.b)?,
                    waddr: alu.mul.waddr,
                    magic: alu.mul.magic_write,
                    pack: find(&OutPack::ALL, alu.mul.output_pack, |x| x as u32)?,
                },
            }))
        }
        INSTR_TYPE_BRANCH => {
            // SAFETY: a branch's body is `branch`.
            let b = unsafe { m.body.branch };
            Some(Instr71::Branch(branch_from_mesa(&b)?))
        }
        _ => None,
    }
}

/// What `v3d_qpu_instr_pack` asserts rather than refuses: a value that does
/// not fit its field, an LDVPM* with a magic write.
fn pack_preconditions(m: &MInstr) -> bool {
    match m.type_ {
        INSTR_TYPE_ALU => {
            // SAFETY: an ALU instruction's body is `alu`.
            let alu = unsafe { m.body.alu };
            let ldvpm = [60, 61, 62, 63, 64, 70, 71].contains(&alu.add.op);
            let wide = |i: &MInput| i.mux >= 64;
            m.sig_addr < 64
                && alu.add.waddr < 64
                && alu.mul.waddr < 64
                && ![alu.add.a, alu.add.b, alu.mul.a, alu.mul.b].iter().any(wide)
                && !(ldvpm && alu.add.magic_write)
        }
        // SAFETY: a branch's body is `branch`.
        INSTR_TYPE_BRANCH => unsafe { m.body.branch }.raddr_a < 64,
        _ => false,
    }
}

/// The instruction with the operands its operations do not read cleared. On
/// 7.1 a decoded instruction keeps them, because the word has them (Mesa's
/// packer writes an unread operand a as given, and fixes an unread operand
/// b's address by the opcode: a mul NOP has 63 there), so the decoder
/// returns the instruction whole and the round trip is exact. A builder's
/// unread operand is a don't-care, which [`same_instr`] compares without.
pub fn masked(instr: &Instr71) -> Instr71 {
    let Instr71::Alu(mut alu) = *instr else { return *instr };
    let add_n = super::isa::num_src(add_op_args(alu.add.op));
    if add_n < 2 {
        alu.add.b = Input71::default();
    }
    if add_n < 1 {
        alu.add.a = Input71::default();
    }
    let mul_n = super::isa::num_src(mul_op_args(alu.mul.op));
    if mul_n < 2 {
        alu.mul.b = Input71::default();
    }
    if mul_n < 1 {
        alu.mul.a = Input71::default();
    }
    Instr71::Alu(alu)
}

/// `instr` and `back` are the same instruction: equal without their unread operands, or
/// FADD/FADDNF/FMIN/FMAX with their operands the other way round (the order
/// of the two addresses is what tells FADD from FADDNF and FMIN from FMAX).
pub fn same_instr(instr: &Instr71, back: &Instr71) -> bool {
    let want = masked(instr);
    let back = masked(back);
    if want == back {
        return true;
    }
    match want {
        Instr71::Alu(mut a) if matches!(a.add.op, AddOp71::Fadd | AddOp71::Faddnf | AddOp71::Fmin | AddOp71::Fmax) => {
            core::mem::swap(&mut a.add.a, &mut a.add.b);
            Instr71::Alu(a) == back
        }
        _ => false,
    }
}

/// The small-immediate operands of an ALU instruction that are read, with
/// their address and whether the signal makes it one.
fn small_imm_out_of_range(a: &Alu71) -> bool {
    let add_n = super::isa::num_src(add_op_args(a.add.op));
    let mul_n = super::isa::num_src(mul_op_args(a.mul.op));
    [
        (a.sig.small_imm_a, add_n >= 1, a.add.a.raddr),
        (a.sig.small_imm_b, add_n >= 2, a.add.b.raddr),
        (a.sig.small_imm_c, mul_n >= 1, a.mul.a.raddr),
        (a.sig.small_imm_d, mul_n >= 2, a.mul.b.raddr),
    ]
    .iter()
    .any(|&(on, read, raddr)| on && read && small_imm_value(raddr).is_none())
}

/// The magic write addresses of an instruction are all named.
fn magic_named(alu: &Alu71) -> bool {
    let unnamed = |waddr: u8, magic: bool| magic && magic_waddr_name(waddr).is_none();
    !(unnamed(alu.add.waddr, alu.add.magic)
        || unnamed(alu.mul.waddr, alu.mul.magic)
        || (alu.sig.writes_address() && unnamed(alu.sig_addr, alu.sig_magic)))
}

/// Mesa's 7.1 unpack of a mul FMOV reads the input unpack as
/// `(raddr_d >> 2) & 3` (qpu_pack.c, `v3d71_qpu_mul_unpack`) although its
/// pack writes `packed << 2` with `packed` up to 6: SAT, NSAT and MAX0 (4, 5,
/// 6) set bit 4 of raddr_d (word bit 16), which the unpack drops, giving ABS,
/// NONE and L back. Mesa's own compiler emits such words (147 of the 9078 in
/// the 7.1 corpus). The model reads the field whole: with word bit 16 set the
/// unpack is 4 + what Mesa read, and 7 (H with the bit) is refused. The word
/// is then held to the same rule as every other, `pack(unpack(w)) == w`.
fn fix_mul_fmov_unpack(w: u64, m: &mut MInstr) -> Result<(), DecodeError> {
    const RADDR_D_BIT4: u64 = 1 << 16;
    if m.type_ != INSTR_TYPE_ALU || w & RADDR_D_BIT4 == 0 {
        return Ok(());
    }
    // SAFETY: an ALU instruction's body is `alu`.
    let alu = unsafe { &mut m.body.alu };
    if alu.mul.op != MulOp71::Fmov as u32 {
        return Ok(());
    }
    alu.mul.a.unpack = match find(&Unpack71::ALL, alu.mul.a.unpack, |x| x as u32) {
        Some(Unpack71::Abs) => Unpack71::Sat as u32,
        Some(Unpack71::None) => Unpack71::Nsat as u32,
        Some(Unpack71::L) => Unpack71::Max0 as u32,
        _ => return Err(DecodeError::NotReproducedByMesa),
    };
    Ok(())
}

/// Decode one 7.1 word (`v3d_qpu_instr_unpack` with the 7.1 device, and the
/// rules in the module comment).
pub fn decode(w: u64) -> Result<Instr71, DecodeError> {
    let mut m = qpu_ffi::unpack_for(G, w).ok_or(DecodeError::MesaRefuses)?;
    fix_mul_fmov_unpack(w, &mut m)?;
    if !pack_preconditions(&m) {
        return Err(DecodeError::NotReproducedByMesa);
    }
    match qpu_ffi::pack_for(G, &m) {
        Some(back) if back == w => {}
        _ => return Err(DecodeError::NotReproducedByMesa),
    }
    let instr = from_mesa(&m).ok_or(DecodeError::OutsideModel)?;
    if let Instr71::Alu(a) = &instr {
        if !magic_named(a) {
            return Err(DecodeError::MagicWaddrUnnamed);
        }
        if small_imm_out_of_range(a) {
            return Err(DecodeError::SmallImmOutOfRange);
        }
    }
    Ok(instr)
}

/// Encode one 7.1 instruction, and check that the word decodes back to it.
pub fn encode(instr: &Instr71) -> Result<u64, EncodeError> {
    if let Instr71::Alu(a) = instr {
        if [a.add.a, a.add.b, a.mul.a, a.mul.b].iter().any(|i| i.raddr >= 64) {
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
        if small_imm_out_of_range(a) {
            return Err(EncodeError::SmallImmOutOfRange);
        }
        if !magic_named(a) {
            return Err(EncodeError::MagicWaddrUnnamed);
        }
    }
    if let Instr71::Branch(b) = instr {
        if b.raddr_a >= 64 {
            return Err(EncodeError::RaddrTooWide);
        }
    }
    let m = to_mesa(instr);
    debug_assert!(pack_preconditions(&m), "the checks above are Mesa's asserts");
    let w = qpu_ffi::pack_for(G, &m).ok_or(EncodeError::MesaRefuses)?;
    let back = decode(w).map_err(EncodeError::DoesNotDecode)?;
    if !same_instr(instr, &back) {
        return Err(EncodeError::ReadsBackAs);
    }
    Ok(w)
}
