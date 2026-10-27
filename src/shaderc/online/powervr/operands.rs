// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The register a source or destination section names.
//!
//! [`super::decode`] recovers which variant each section is and the value of
//! every settable field. This reads the operands back out of that: for each
//! source slot, which register file and which index.
//!
//! # Why this is not simply `Region::get("sbA")`
//!
//! The field names are variant-specific, because the widths are. A source
//! section names its first operand's bank as `sbA_1bit_b0`, `sbA_2bit_b2`,
//! `sbA_3bit_b1`, `sbA_3bit_b3` or `sbA_3bit_b4` depending on the form -- one
//! bit where only `special` and `temp` can be named, two where `vtxin` and
//! `coeff` join them, three for the whole file set. That is the same fact the
//! seam records as "some slots in some instruction forms cannot name
//! `shared`", and it is why legalisation exists at all.
//!
//! So an operand is found by its role prefix (`sbA_`, `sA_`) rather than by a
//! fixed name, and its width comes from the field the variant actually uses.

use super::isa::{bit_set, BitSetId, Region};

/// One operand: which file, which register in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Operand {
    /// The raw `regbank` value; [`super::sel::Bank`] names the ones the
    /// kernel produces.
    pub bank: u32,
    pub index: u32,
}

/* An operand deliberately does NOT carry the width of the field it was read
 * from. That width belongs to the VARIANT, not to the register: the same
 * register read out of a three-bit-bank form and written into a two-bit-bank
 * one is the same operand, and a struct that carried the width would make
 * those two compare unequal. Which is exactly what happened -- the
 * choose/fill/read-back identity test failed on 37 sections of
 * `uniform_color.frag` where nothing but the field width had changed. The
 * width is available where it belongs, from `src_capability` and
 * `dst_capability`. */

/// The value and width of one settable field, by its exact name.
///
/// Exact, not a prefix: the settable names are `s0`, `sb0`, `s1`, `sb1`, ...,
/// and `s0` is a prefix of nothing but `sb0` shares its first letter. A
/// prefix match here would be the kind of near-miss this file exists to
/// avoid.
fn field(r: &Region, name: &str) -> Option<(u32, u8)> {
    let set = bit_set(r.set);
    let v = r.variant();
    v.uses.iter().enumerate().find_map(|(i, u)| {
        (u.fixed.is_none() && u.name == name).then(|| {
            let f = &set.fields[u.field];
            (f.xform.decode(r.vals[i]), f.bits)
        })
    })
}

/// The operands a source section names, indexed by TRUE SLOT 0..5.
///
/// A group has six source slots, and the two source sections carry three
/// each: the lower section names `s0`, `s1`, `s2` and the upper `s3`, `s4`,
/// `s5`. Indexing the result by the true slot rather than by position within
/// the section is what makes the per-slot bank widths -- the legalisation
/// constraint -- line up across the two halves, and it is why the ISA keeps
/// `Variant::upper_src` at all: the two sections have the same bit layout and
/// only their position in the group tells them apart.
pub fn srcs(r: &Region) -> [Option<Operand>; 6] {
    debug_assert_eq!(r.set, BitSetId::Src);
    let mut out = [None; 6];
    let base = if r.variant().upper_src == Some(true) { 3 } else { 0 };
    for i in 0..3 {
        let slot = base + i;
        let (bn, sn) = (SLOT_BANK[slot], SLOT_IDX[slot]);
        if let (Some((bank, _)), Some((index, _))) = (field(r, bn), field(r, sn)) {
            out[slot] = Some(Operand { bank, index });
        }
    }
    out
}

pub(super) const SLOT_BANK: [&str; 6] = ["sb0", "sb1", "sb2", "sb3", "sb4", "sb5"];
pub(super) const SLOT_IDX: [&str; 6] = ["s0", "s1", "s2", "s3", "s4", "s5"];

/// The operands a destination section names.
///
/// A group writes one register or two. The one-destination forms name it
/// `dN`/`dbN` and the two-destination forms `d0`/`d1`, so both spellings are
/// read and the single form lands in slot 0.
pub fn dsts(r: &Region) -> [Option<Operand>; 2] {
    debug_assert_eq!(r.set, BitSetId::Dst);
    let mut out = [None; 2];
    for (slot, (bn, dn)) in [("db0", "d0"), ("db1", "d1")].into_iter().enumerate() {
        if let (Some((bank, _)), Some((index, _))) = (field(r, bn), field(r, dn)) {
            out[slot] = Some(Operand { bank, index });
        }
    }
    if out[0].is_none() {
        if let (Some((bank, _)), Some((index, _))) = (field(r, "dbN"), field(r, "dN")) {
            out[0] = Some(Operand { bank, index });
        }
    }
    out
}

/// What one source-section variant can hold: per slot, how wide its bank and
/// index fields are. `None` for a slot the variant does not have.
///
/// This is the legalisation constraint in the form legalisation needs it. A
/// slot with a 1-bit bank can name `special` and `temp` and nothing else, so
/// an operand in the shared file cannot go there whatever its index; a 6-bit
/// index cannot hold register 64 whatever its bank.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SlotCap {
    pub bank_bits: u8,
    pub index_bits: u8,
}

/// The capability of every slot of a source-section variant, by TRUE SLOT.
pub fn src_capability(variant: u16) -> [Option<SlotCap>; 6] {
    let set = bit_set(BitSetId::Src);
    let v = &set.variants[variant as usize];
    let mut out = [None; 6];
    let base = if v.upper_src == Some(true) { 3 } else { 0 };
    for i in 0..3 {
        let slot = base + i;
        let bank = v.uses.iter().find(|u| u.fixed.is_none() && u.name == SLOT_BANK[slot]);
        let idx = v.uses.iter().find(|u| u.fixed.is_none() && u.name == SLOT_IDX[slot]);
        if let (Some(b), Some(x)) = (bank, idx) {
            out[slot] = Some(SlotCap { bank_bits: set.fields[b.field].bits, index_bits: set.fields[x.field].bits });
        }
    }
    out
}

impl SlotCap {
    /// Can this slot name that register?
    ///
    /// A read or write through an index register (bank idx0 or idx1) takes
    /// a slot with an 11-bit index field whatever its value: pco sizes every
    /// indexed operand at 11 bits (`pco_map_reg_index_bits`, Mesa
    /// src/imagination/pco/pco_map.h.py, MIT), so no narrower form of one
    /// exists in the corpus. A `3b8i` slot holding `shared[idx0 + 8]` (67)
    /// is one pco never emits.
    pub fn holds(&self, op: &Operand) -> bool {
        const IDX0: u32 = 6;
        let bank_ok = self.bank_bits >= 32 || op.bank < (1u32 << self.bank_bits);
        let idx_ok = self.index_bits >= 32 || op.index < (1u32 << self.index_bits);
        let indexed_ok = op.bank < IDX0 || self.index_bits >= 11;
        bank_ok && idx_ok && indexed_ok
    }
}

/// The capability of every slot of a DESTINATION-section variant.
///
/// Two slots, because a group writes one register or two. The one-destination
/// forms name it `dN`/`dbN` and the two-destination forms `d0`/`d1`; a single
/// destination is reported in slot 0 whichever spelling the variant uses, so a
/// caller asking "can this variant hold my one destination" does not have to
/// know which family it is looking at.
pub fn dst_capability(variant: u16) -> [Option<SlotCap>; 2] {
    let set = bit_set(BitSetId::Dst);
    let v = &set.variants[variant as usize];
    let mut out = [None; 2];
    let pairs = [[("db0", "d0"), ("db1", "d1")], [("dbN", "dN"), ("", "")]];
    for family in pairs {
        for (slot, (bn, dn)) in family.into_iter().enumerate() {
            if bn.is_empty() || out[slot].is_some() {
                continue;
            }
            let bank = v.uses.iter().find(|u| u.fixed.is_none() && u.name == bn);
            let idx = v.uses.iter().find(|u| u.fixed.is_none() && u.name == dn);
            if let (Some(b), Some(x)) = (bank, idx) {
                out[slot] = Some(SlotCap { bank_bits: set.fields[b.field].bits, index_bits: set.fields[x.field].bits });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::isa::BitSetId;
    use super::*;

    /// The variant NAME says how many operands it has and how wide each
    /// bank field is: `3lo_3b11i_2b8i_3b11i_3m` is three lower sources with
    /// bank/index widths 3/11, 2/8, 3/11. Parsing that name is an oracle
    /// INDEPENDENT of the field-name prefixes this module matches on, so the
    /// two agreeing is evidence the extraction is right rather than merely
    /// self-consistent.
    fn widths_from_name(name: &str) -> Vec<(u8, u8)> {
        let mut out = Vec::new();
        for part in name.split('_').skip(1) {
            if let Some((b, i)) = part.strip_suffix('i').and_then(|p| p.split_once('b')) {
                if let (Ok(b), Ok(i)) = (b.parse::<u8>(), i.parse::<u8>()) {
                    out.push((b, i));
                }
            }
        }
        out
    }

    #[test]
    fn every_source_variant_yields_the_operands_its_name_declares() {
        let set = bit_set(BitSetId::Src);
        let mut checked = 0;
        for (vi, v) in set.variants.iter().enumerate() {
            let want = widths_from_name(v.name);
            let r = Region::new(BitSetId::Src, vi as u16);
            let got = srcs(&r);
            let n = got.iter().flatten().count();
            assert_eq!(n, want.len(), "variant {} declares {} operands, extraction found {n}", v.name, want.len());
            /* Lower variants fill slots 0..2, upper 3..5, and the name lists
             * them in that order within its own half. */
            let base = if v.upper_src == Some(true) { 3 } else { 0 };
            for (i, (bank_bits, _idx_bits)) in want.iter().enumerate() {
                assert_eq!(
                    src_capability(vi as u16)[base + i].expect("slot present").bank_bits,
                    *bank_bits,
                    "variant {} slot {}: name says a {bank_bits}-bit bank",
                    v.name,
                    base + i
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 16, "the source section had 16 variants when this was written");
    }

    #[test]
    fn every_destination_variant_yields_the_operands_its_name_declares() {
        let set = bit_set(BitSetId::Dst);
        for (vi, v) in set.variants.iter().enumerate() {
            let want = widths_from_name(v.name);
            let r = Region::new(BitSetId::Dst, vi as u16);
            let got = dsts(&r);
            let n = got.iter().flatten().count();
            assert_eq!(n, want.len(), "variant {} declares {} destinations, extraction found {n}", v.name, want.len());
            for (slot, (bank_bits, _)) in want.iter().enumerate() {
                assert_eq!(
                    dst_capability(vi as u16)[slot].expect("slot present").bank_bits,
                    *bank_bits,
                    "variant {} slot {slot}",
                    v.name
                );
            }
        }
    }

    /// The bank width per slot is the legalisation constraint, so pin the
    /// shape of it rather than only that it parses: a 1-bit bank can name
    /// two files and a 3-bit bank eight, and `shared` is not reachable from
    /// the narrow ones.
    #[test]
    fn some_source_slots_cannot_name_the_shared_file() {
        let set = bit_set(BitSetId::Src);
        let mut narrow = 0;
        let mut wide = 0;
        for vi in 0..set.variants.len() {
            for cap in src_capability(vi as u16).iter().flatten() {
                if cap.bank_bits < 3 {
                    narrow += 1;
                } else {
                    wide += 1;
                }
            }
        }
        assert!(narrow > 0, "no slot is narrow; the legalisation constraint would not exist");
        assert!(wide > 0, "no slot is wide; nothing could read the shared file directly");
    }
}
