// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Igalia S.L.
// Derived from Mesa 26.2.1 src/etnaviv/isa/etnaviv.xml (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The Vivante GC7000 instruction encoding: the shape of the tables and the
//! bit-level reads and writes over them.
//!
//! An instruction is a fixed 128 bits, four little-endian dwords. Every
//! operation pins some of those bits (its opcode and the shape bits that say
//! which operands it has) and names the rest as fields. Pinned bits and
//! fields together cover all 128 bits of every operation with no gap and no
//! overlap, which is what lets a decoded instruction be written back exactly.
//!
//! The tables are in [`super::isa_tables`], generated from Mesa's etnaviv ISA
//! description; nothing here knows any particular operation.

/// One named bit range of an instruction.
pub struct Field {
    pub name: &'static str,
    pub low: u8,
    pub high: u8,
    /// The isaspec type: an enumeration name (`#reg_group`), a nested operand
    /// bitset (`#instruction-src`), or a primitive (`bool`, `uint`, `int`).
    pub ty: Option<&'static str>,
    /// How the disassembler prints the field when it is set, where the
    /// description gives a spelling (`.sat`).
    pub display: Option<&'static str>,
}

impl Field {
    pub fn bits(&self) -> u32 {
        (self.high - self.low + 1) as u32
    }
}

/// One operation: the bits that identify it, and the fields that are the rest.
pub struct Op {
    /// The bitset's name, which is unique: this is the key
    /// [`super::build::Builder::new`] resolves an operation by.
    pub name: &'static str,
    /// What the instruction is called in a listing, which is NOT unique:
    /// `branch_unary` and `branch_binary` are both spelled `branch`, because
    /// the description says so (isaspec's `displayname`) - they are one
    /// instruction with different operand counts. `bit_findlsb` and
    /// `bit_findmsb` carry no such spelling and are genuinely two mnemonics
    /// sharing an opcode, so this cannot be collapsed by a rule about
    /// opcodes. Print this; key on `name`.
    pub mnemonic: &'static str,
    /// The instruction class the description groups it in (`alu`, `tex`,
    /// `cf`, ...), or "" where it names none.
    pub meta: &'static str,
    /// The bits this operation pins.
    pub care: u128,
    /// What it pins them to (`word & care == bits` identifies it).
    pub bits: u128,
    /// How a listing prints this instruction: the description's own display
    /// template, `{INSTR_ALU} {DST:align=18}, {SRC0}, {SRC1}, void` and the
    /// like. [`super::isa_tables::TEMPLATES`] holds the named fragments it
    /// refers to.
    pub display: &'static str,
    /// Values the description assembles from other fields, each as the pieces
    /// it is made of: `TYPE` is `TYPE_BIT2 << 2 | TYPE_BIT01`, three bits the
    /// encoding keeps in two places. A listing needs them and they are not
    /// fields, so they travel with the operation rather than being
    /// re-derived in the printer.
    pub derived: &'static [(&'static str, &'static [(&'static str, u32)])],
    /// Bits the description marks `x`: part of no field and pinned to no
    /// value. Nothing names them, so a decoded instruction keeps their value
    /// verbatim ([`Instr::dont_care`]) rather than letting an encoder that
    /// starts from zero quietly decide they were zero.
    pub dont_care: u128,
    pub fields: &'static [Field],
}

/// The most fields any operation has (25 today).
pub const MAX_FIELDS: usize = 32;

/// One decoded instruction: which operation, and the value of each of its
/// fields in the operation's own field order.
#[derive(Clone, Copy)]
pub struct Instr {
    pub op: u16,
    pub vals: [u32; MAX_FIELDS],
    /// The bits of [`Op::dont_care`] as they were read, in place. Zero for
    /// every instruction in the offline table today, which
    /// [`super::roundtrip`] asserts so that the first one that is not is
    /// news rather than silence.
    pub dont_care: u128,
}

impl Instr {
    pub fn op(&self) -> &'static Op {
        &super::isa_tables::OPS[self.op as usize]
    }

    /// A field's value by name, if this operation has one.
    pub fn get(&self, name: &str) -> Option<u32> {
        self.op().fields.iter().position(|f| f.name == name).map(|i| self.vals[i])
    }
}

/// The four dwords of an instruction as one 128-bit word (dword 0 lowest).
pub fn word_of(dw: &[u32; 4]) -> u128 {
    (dw[0] as u128) | ((dw[1] as u128) << 32) | ((dw[2] as u128) << 64) | ((dw[3] as u128) << 96)
}

/// The inverse of [`word_of`].
pub fn dwords_of(w: u128) -> [u32; 4] {
    [w as u32, (w >> 32) as u32, (w >> 64) as u32, (w >> 96) as u32]
}

pub fn read_field(w: u128, f: &Field) -> u32 {
    let n = f.bits();
    let mask: u128 = if n >= 128 { u128::MAX } else { (1u128 << n) - 1 };
    ((w >> f.low) & mask) as u32
}

pub fn write_field(w: &mut u128, f: &Field, v: u32) {
    let n = f.bits();
    let mask: u128 = if n >= 128 { u128::MAX } else { (1u128 << n) - 1 };
    *w &= !(mask << f.low);
    *w |= ((v as u128) & mask) << f.low;
}

/// Every bit of `op` that is either pinned or covered by one of its fields.
/// The tables are expected to cover all 128; [`super::roundtrip`] is what
/// proves they do, on the real corpus rather than on this function.
pub fn coverage(op: &Op) -> u128 {
    let mut c = op.care | op.dont_care;
    for f in op.fields {
        let n = f.bits();
        let mask: u128 = if n >= 128 { u128::MAX } else { (1u128 << n) - 1 };
        c |= mask << f.low;
    }
    c
}
