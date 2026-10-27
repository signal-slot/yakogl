// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/qpu/qpu_pack.c (the V3D_QPU_*_SHIFT/_MASK
// defines) (MIT). The upstream part stays under its licence
// (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! Test only: where the fields sit in a V3D 4.2 QPU word (Mesa's
//! `qpu_pack.c:47-111`, the `V3D_QPU_*_SHIFT`/`_MASK` defines), for the tests
//! that build words by hand -- a known-bad word for the decoder's strict
//! rules, the field enumeration of `mesa_ref`. The product code has no copy:
//! the encoder and the decoder are Mesa's own (`qpu_ffi`). Each range is
//! checked against Mesa's packer by `the_fields_are_where_mesa_packs_them`.

#![allow(dead_code)]

/// A bit range of the 64-bit word.
#[derive(Clone, Copy, Debug)]
pub struct Bits {
    pub shift: u32,
    pub width: u32,
}

impl Bits {
    pub const fn mask(self) -> u64 {
        ((1u64 << self.width) - 1) << self.shift
    }
    pub const fn get(self, w: u64) -> u32 {
        ((w >> self.shift) & ((1u64 << self.width) - 1)) as u32
    }
    /// Write `v` into the range; a value too wide for it is a defect of the
    /// test, and panics rather than being masked into a different value.
    pub fn set(self, w: &mut u64, v: u32) {
        assert!((v as u64) < (1u64 << self.width), "{v} does not fit {} bits", self.width);
        *w = (*w & !self.mask()) | ((v as u64) << self.shift);
    }
}

const fn bits(high: u32, low: u32) -> Bits {
    Bits { shift: low, width: high - low + 1 }
}

// ALU word.
pub const OP_MUL: Bits = bits(63, 58);
pub const SIG: Bits = bits(57, 53);
/// The condition/flags field, or, when the signal writes an address, the
/// signal's destination (bit 6 is "magic").
pub const COND: Bits = bits(52, 46);
pub const COND_SIG_MAGIC_ADDR: u32 = 1 << 6;
/// Mul ALU magic write.
pub const MM: Bits = bits(45, 45);
/// Add ALU magic write (also the IN/OUT bit of the LDVPM* operations).
pub const MA: Bits = bits(44, 44);
pub const WADDR_M: Bits = bits(43, 38);
pub const WADDR_A: Bits = bits(37, 32);
pub const OP_ADD: Bits = bits(31, 24);
pub const MUL_B: Bits = bits(23, 21);
pub const MUL_A: Bits = bits(20, 18);
pub const ADD_B: Bits = bits(17, 15);
pub const ADD_A: Bits = bits(14, 12);
pub const RADDR_A: Bits = bits(11, 6);
/// On 4.2 raddr_b also holds the small immediate's index. (On 7.x the four
/// operands each have a 6-bit raddr of their own, `RADDR_C`/`RADDR_D` over
/// 18..23 and 12..17, and the mux fields are gone.)
pub const RADDR_B: Bits = bits(5, 0);

// Branch word (OP_MUL is 0 and SIG & 0b11000 == 0b10000).
/// Offset bits 3..23. Its top three bits (53..55) are the low three bits of
/// the SIG field, which is why a branch is recognised by SIG's top two bits
/// only.
pub const BRANCH_ADDR_LOW: Bits = bits(55, 35);
pub const BRANCH_COND: Bits = bits(34, 32);
/// Offset bits 24..31.
pub const BRANCH_ADDR_HIGH: Bits = bits(31, 24);
pub const BRANCH_MSFIGN: Bits = bits(22, 21);
pub const BRANCH_BDU: Bits = bits(17, 15);
pub const BRANCH_UB: Bits = bits(14, 14);
pub const BRANCH_BDI: Bits = bits(13, 12);
/// The bits of a branch word that no field of the branch covers: 0..5 (the
/// ALU's raddr_b), 18..20 and 23. Mesa's decoder ignores them and its encoder
/// writes zero; this decoder refuses a word that sets one.
pub const BRANCH_UNUSED: u64 = bits(5, 0).mask() | bits(20, 18).mask() | bits(23, 23).mask();
/// Bits 56..57 of a branch: SIG's top two bits, `0b10`.
pub const BRANCH_SIG_HIGH: Bits = bits(57, 56);

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::v3d::encode::encode;
    use crate::shaderc::online::v3d::isa::*;

    /// Each field this file names, moved by one value through Mesa's packer:
    /// the word changes in exactly that range.
    #[test]
    fn the_fields_are_where_mesa_packs_them() {
        let nop = Alu::nop();
        let base = encode(&Instr::Alu(nop)).unwrap();
        let moved = |f: &dyn Fn(&mut Alu)| {
            let mut a = nop;
            f(&mut a);
            encode(&Instr::Alu(a)).unwrap() ^ base
        };
        assert_eq!(moved(&|a| a.raddr_a = 1), 1 << RADDR_A.shift);
        assert_eq!(moved(&|a| a.raddr_b = 1), 1 << RADDR_B.shift);
        assert_eq!(moved(&|a| a.add.waddr = 7), 1 << WADDR_A.shift);
        assert_eq!(moved(&|a| a.mul.waddr = 7), 1 << WADDR_M.shift);
        assert_eq!(moved(&|a| a.add.magic = false), MA.mask());
        assert_eq!(moved(&|a| a.mul.magic = false), MM.mask());
        // "add rf1, r1, r2" / "fmul rf2, r1, r2": the muxes and opcodes.
        let mut a = nop;
        a.add = AddAlu {
            op: AddOp::Add,
            a: Input { mux: Mux::R1, ..Input::default() },
            b: Input { mux: Mux::R2, ..Input::default() },
            waddr: 1,
            magic: false,
            pack: OutPack::None,
        };
        a.mul = MulAlu {
            op: MulOp::Fmul,
            a: Input { mux: Mux::R1, unpack: Unpack::None },
            b: Input { mux: Mux::R2, unpack: Unpack::None },
            waddr: 2,
            magic: false,
            pack: OutPack::None,
        };
        let w = encode(&Instr::Alu(a)).unwrap();
        assert_eq!((ADD_A.get(w), ADD_B.get(w), WADDR_A.get(w), MA.get(w)), (1, 2, 1, 0));
        assert_eq!((MUL_A.get(w), MUL_B.get(w), WADDR_M.get(w), MM.get(w)), (1, 2, 2, 0));
        assert_eq!(OP_ADD.get(w), 56, "ADD is opcode 56");
        assert_eq!(OP_MUL.get(w) >> 4, 1, "FMUL with no pack is opcode 16 + the unpacks");
        // The signal and the flags.
        let mut s = nop;
        s.sig.ldunif = true;
        assert_eq!(SIG.get(encode(&Instr::Alu(s)).unwrap()), 2);
        let mut f = nop;
        f.flags.apf = Pf::PushZ;
        assert_eq!(COND.get(encode(&Instr::Alu(f)).unwrap()), 1);
        // A branch: condition, msfign, bdi, ub/bdu, raddr_a, the offset.
        let b = Branch {
            cond: BranchCond::AnyA,
            msfign: Msfign::Q,
            bdi: BranchDest::Rel,
            ub: true,
            bdu: BranchDest::Regfile,
            raddr_a: 5,
            offset: 0x1234_5678,
        };
        let w = encode(&Instr::Branch(b)).unwrap();
        assert_eq!(BRANCH_SIG_HIGH.get(w), 0b10);
        assert_eq!(BRANCH_COND.get(w), 6, "ANYA");
        assert_eq!(BRANCH_MSFIGN.get(w), 2);
        assert_eq!(BRANCH_BDI.get(w), 1);
        assert_eq!((BRANCH_UB.get(w), BRANCH_BDU.get(w)), (1, 3));
        assert_eq!(RADDR_A.get(w), 5);
        assert_eq!((BRANCH_ADDR_LOW.get(w) << 3) | (BRANCH_ADDR_HIGH.get(w) << 24), 0x1234_5678);
        assert_eq!(w & BRANCH_UNUSED, 0);
    }
}
