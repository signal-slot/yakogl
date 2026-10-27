// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Derived from Mesa 26.2.1 src/imagination/pco/ (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The shape of the PowerVR Rogue USC encoding tables, and the bit-level
//! reads and writes over them.
//!
//! The tables themselves are in [`super::isa_tables`], generated from Mesa's
//! pco declarations (see `tools/pco-isa/README.md`); nothing here knows any
//! particular instruction. A *bit set* is one encodable region of a group
//! (the group header, a source section, the destination section, the main ALU
//! instruction word, ...). It owns named *fields*, each assembled from one or
//! more *pieces* — a run of bits at a byte offset — most significant piece
//! first. A *variant* is one concrete form of that region: an ordered list of
//! field uses, each either pinned to a fixed value (which is what tells one
//! variant from another when decoding) or left settable, and a byte count.
//! The uses of a variant tile its bytes exactly, with no gap and no overlap,
//! so re-encoding a decoded variant reproduces its bytes.

/// A run of bits of one field, at `byte`, from bit `hi` down to bit `lo`.
pub struct Piece {
    pub byte: u8,
    pub hi: u8,
    pub lo: u8,
}

/// How a field's logical value is transformed on its way into the bits.
/// Decoding applies the inverse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Xform {
    /// The raw bits are the value.
    Identity,
    /// `[1..4]` stored as `[0..3]` (a repeat count).
    SubOne,
    /// `[1..16]` stored as `[1..15, 0]` (a group length in words).
    WrapMod16,
    /// A byte offset with bit 0 known clear.
    Shr1,
    /// A byte offset with bits 1:0 known clear.
    Shr2,
}

impl Xform {
    /// bits -> logical value.
    pub fn decode(self, raw: u32) -> u32 {
        match self {
            Xform::Identity => raw,
            Xform::SubOne => raw + 1,
            Xform::WrapMod16 => {
                if raw == 0 {
                    16
                } else {
                    raw
                }
            }
            Xform::Shr1 => raw << 1,
            Xform::Shr2 => raw << 2,
        }
    }

    /// logical value -> bits, the inverse of [`Xform::decode`].
    ///
    /// `None` when the value is not representable under this transform, which
    /// is a refusal and not a clamp: a repeat count of 0 or a group length of
    /// 17 has no encoding, and writing the nearest thing that fits would
    /// produce a well-formed group computing something else. That failure --
    /// a helper that masks instead of failing -- is the one this project has
    /// already paid for twice.
    pub fn encode(self, val: u32) -> Option<u32> {
        match self {
            Xform::Identity => Some(val),
            Xform::SubOne => val.checked_sub(1),
            Xform::WrapMod16 => match val {
                16 => Some(0),
                1..=15 => Some(val),
                _ => None,
            },
            Xform::Shr1 => (val & 1 == 0).then_some(val >> 1),
            Xform::Shr2 => (val & 3 == 0).then_some(val >> 2),
        }
    }
}

/// Why a field could not be given a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetError {
    /// This variant has no settable field of that name. Almost always the
    /// wrong variant was chosen rather than the wrong name.
    NoSuchField { name: &'static str },
    /// The value has no encoding under the field's transform.
    NotRepresentable { name: &'static str, val: u32 },
    /// The value does not fit the field's width. Refused rather than
    /// truncated: the bits that would be dropped are the difference between
    /// register 5 and register 69.
    TooWide { name: &'static str, val: u32, bits: u8 },
}

/// One named enumeration, for naming a field value in a listing.
pub struct EnumDef {
    pub name: &'static str,
    pub elems: &'static [(u32, &'static str)],
}

impl EnumDef {
    pub fn name_of(&self, v: u32) -> Option<&'static str> {
        self.elems.iter().find(|(x, _)| *x == v).map(|(_, n)| *n)
    }
}

/// A field of a bit set: its bits, gathered most significant piece first.
pub struct Field {
    pub name: &'static str,
    pub pieces: &'static [Piece],
    pub bits: u8,
    pub xform: Xform,
    /// The enumeration that names this field's values, if any.
    pub enum_id: Option<usize>,
    /// A field the encoding requires to hold one value, whatever the variant.
    pub reserved: Option<u32>,
}

impl Field {
    /// The number of bytes this field reaches into.
    pub fn span(&self) -> usize {
        self.pieces.iter().map(|p| p.byte as usize + 1).max().unwrap_or(0)
    }
}

/// One field of a variant: pinned to `fixed`, or settable under `name`.
pub struct FieldUse {
    pub field: usize,
    pub fixed: Option<u32>,
    pub name: &'static str,
}

/// One concrete form of a bit set's region.
pub struct Variant {
    pub name: &'static str,
    pub bytes: u8,
    /// For the source sections only: whether this form encodes the upper
    /// source group (s3..s5) rather than the lower one (s0..s2). The two have
    /// the same bit layout, so only its position in the group tells them
    /// apart; this is what says which forms belong to which section.
    pub upper_src: Option<bool>,
    pub uses: &'static [FieldUse],
}

/// A region of the encoding, with its fields and every form it can take.
pub struct BitSet {
    pub name: &'static str,
    pub fields: &'static [Field],
    pub variants: &'static [Variant],
}

/// Which instruction form, in which phase, is which operation.
pub struct OpEnc {
    pub name: &'static str,
    pub phase: u8,
    pub set: u8,
    pub variant: u16,
    /// The settable fields this operation pins to a constant. Two operations
    /// can share one instruction form and differ only here (the single-source
    /// form is FRCP at `sngl_op=rcp` and MBYP at `sngl_op=byp`).
    pub consts: &'static [(&'static str, u32)],
}

/// Index into [`super::isa_tables::BIT_SETS`]; the generator writes the same
/// order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum BitSetId {
    IgrpHdr = 0,
    Src = 1,
    Iss = 2,
    Dst = 3,
    Main = 4,
    Backend = 5,
    Bitwise = 6,
    Ctrl = 7,
}

/// The pipeline phases, in the order the generator writes them.
pub const PH_0: u8 = 0;
pub const PH_1: u8 = 1;
pub const PH_2: u8 = 2;
pub const PH_2_PCK: u8 = 3;
pub const PH_2_TST: u8 = 4;
pub const PH_2_MOV: u8 = 5;
pub const PH_BACKEND: u8 = 6;
pub const PH_CTRL: u8 = 7;

/// The largest number of field uses any variant has (28 today); a decoded
/// region carries one value per use.
pub const MAX_USES: usize = 32;
/// The largest number of bytes any variant takes (8 today).
pub const MAX_VARIANT_BYTES: usize = 8;

pub fn bit_set(id: BitSetId) -> &'static BitSet {
    &super::isa_tables::BIT_SETS[id as usize]
}

/// Read a field's raw (encoded) value out of `buf[off..]`.
pub fn read_field(buf: &[u8], off: usize, set: &BitSet, field: usize) -> u32 {
    let f = &set.fields[field];
    let mut v: u32 = 0;
    for p in f.pieces {
        let n = p.hi - p.lo + 1;
        let b = buf[off + p.byte as usize];
        v = (v << n) | ((b >> p.lo) as u32 & ((1u32 << n) - 1));
    }
    v
}

/// Write a field's raw (encoded) value into `buf[off..]`, leaving the bits
/// outside its pieces alone.
pub fn write_field(buf: &mut [u8], off: usize, set: &BitSet, field: usize, val: u32) {
    let f = &set.fields[field];
    let mut consumed = 0u32;
    for p in f.pieces.iter().rev() {
        let n = (p.hi - p.lo + 1) as u32;
        let mask = (1u32 << n) - 1;
        let bits = ((val >> consumed) & mask) as u8;
        let b = &mut buf[off + p.byte as usize];
        *b &= !((mask as u8) << p.lo);
        *b |= bits << p.lo;
        consumed += n;
    }
}

/// Does `variant` of `set` match the bytes at `buf[off..]`? Every field the
/// variant pins to a value, and every field the encoding reserves, must read
/// back as that value.
pub fn variant_matches(buf: &[u8], off: usize, avail: usize, set: &BitSet, variant: &Variant) -> bool {
    let n = variant.bytes as usize;
    if n > avail {
        return false;
    }
    for u in variant.uses {
        let f = &set.fields[u.field];
        if f.span() > n {
            return false;
        }
        let want = match (u.fixed, f.reserved) {
            (Some(v), _) => v,
            (None, Some(v)) => v,
            (None, None) => continue,
        };
        if read_field(buf, off, set, u.field) != want {
            return false;
        }
    }
    true
}

/// One decoded region: which variant it is, and the value of each of its
/// settable fields, in the variant's own use order.
#[derive(Clone, Copy)]
pub struct Region {
    pub set: BitSetId,
    pub variant: u16,
    pub vals: [u32; MAX_USES],
}

impl Region {
    pub fn variant(&self) -> &'static Variant {
        &bit_set(self.set).variants[self.variant as usize]
    }

    pub fn bytes(&self) -> usize {
        self.variant().bytes as usize
    }

    /// The logical value of a settable field by name, if the variant has one.
    pub fn get(&self, name: &str) -> Option<u32> {
        let set = bit_set(self.set);
        let v = self.variant();
        v.uses
            .iter()
            .enumerate()
            .find(|(_, u)| u.name == name)
            .map(|(i, u)| set.fields[u.field].xform.decode(self.vals[i]))
    }

    /// The raw value of a settable field by name (what is in the bits).
    pub fn raw(&self, name: &str) -> Option<u32> {
        let v = self.variant();
        v.uses.iter().enumerate().find(|(_, u)| u.name == name).map(|(i, _)| self.vals[i])
    }
}

impl Region {
    /// An empty region of one variant, every settable field zero.
    ///
    /// The counterpart of [`decode_as`] for a group being BUILT rather than
    /// read back. Fixed and reserved fields need no value here: they are
    /// written from the tables by [`encode_region`], which is what keeps a
    /// constructed group and a decoded one the same kind of thing.
    pub fn new(set: BitSetId, variant: u16) -> Region {
        Region { set, variant, vals: [0; MAX_USES] }
    }

    /// Give a settable field its logical value, or say why it cannot have it.
    pub fn set(&mut self, name: &'static str, val: u32) -> Result<(), SetError> {
        let set = bit_set(self.set);
        let v = self.variant();
        let (i, u) = v
            .uses
            .iter()
            .enumerate()
            .find(|(_, u)| u.name == name && u.fixed.is_none())
            .ok_or(SetError::NoSuchField { name })?;
        let f = &set.fields[u.field];
        let raw = f.xform.encode(val).ok_or(SetError::NotRepresentable { name, val })?;
        if f.bits < 32 && raw >= (1u32 << f.bits) {
            return Err(SetError::TooWide { name, val, bits: f.bits });
        }
        self.vals[i] = raw;
        Ok(())
    }
}

/// Decode the region at `buf[off..]` as the given variant, reading every
/// settable field. The caller has already established the variant matches.
pub fn decode_as(buf: &[u8], off: usize, id: BitSetId, variant: u16) -> Region {
    let set = bit_set(id);
    let v = &set.variants[variant as usize];
    let mut r = Region { set: id, variant, vals: [0; MAX_USES] };
    for (i, u) in v.uses.iter().enumerate() {
        if u.fixed.is_none() && set.fields[u.field].reserved.is_none() {
            r.vals[i] = read_field(buf, off, set, u.field);
        }
    }
    r
}

/// Write a decoded region back out. Returns the bytes written.
pub fn encode_region(r: &Region, out: &mut [u8], off: usize) -> usize {
    let set = bit_set(r.set);
    let v = r.variant();
    let n = v.bytes as usize;
    for b in &mut out[off..off + n] {
        *b = 0;
    }
    for (i, u) in v.uses.iter().enumerate() {
        let val = match (u.fixed, set.fields[u.field].reserved) {
            (Some(x), _) => x,
            (None, Some(x)) => x,
            (None, None) => r.vals[i],
        };
        write_field(out, off, set, u.field, val);
    }
    n
}

/// Find the one variant of `set` that matches here, among `allowed` (all of
/// them when `allowed` is `None`), optionally restricted to one source half.
pub fn match_one(
    buf: &[u8],
    off: usize,
    avail: usize,
    id: BitSetId,
    allowed: Option<&[u16]>,
    upper_src: Option<bool>,
) -> Option<u16> {
    let set = bit_set(id);
    let mut found = None;
    for (i, v) in set.variants.iter().enumerate() {
        if let Some(a) = allowed {
            if !a.contains(&(i as u16)) {
                continue;
            }
        }
        if upper_src.is_some() && v.upper_src != upper_src {
            continue;
        }
        if variant_matches(buf, off, avail, set, v) {
            if found.is_some() {
                return None; // ambiguous: the caller must not guess
            }
            found = Some(i as u16);
        }
    }
    found
}

/// The variants of `set` that `phase` may hold, or `None` when that phase
/// puts nothing in that bit set.
pub fn phase_variants(phase: u8, id: BitSetId) -> Option<&'static [u16]> {
    super::isa_tables::PHASE_VARIANTS[phase as usize].iter().find(|(s, _)| *s == id as u8).map(|(_, v)| *v)
}

/// The operations a decoded instruction region could be: every op whose form
/// and phase match and whose constant fields hold.
pub fn ops_for(r: &Region, phase: u8) -> impl Iterator<Item = &'static OpEnc> + '_ {
    super::isa_tables::OPS.iter().filter(move |o| {
        o.phase == phase
            && o.set == r.set as u8
            && o.variant == r.variant
            && o.consts.iter().all(|(f, v)| r.raw(f) == Some(*v))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;

    /// `encode` must be the exact inverse of `decode` for every value each
    /// FIELD can actually hold.
    ///
    /// Over the real fields rather than over an invented range of raw values,
    /// and the difference is not pedantry: `WrapMod16` maps raw 0 and raw 16
    /// both to 16, so it is not injective over 0..0xffff -- but it is only
    /// ever used on a 4-bit field, where raw 16 cannot occur. Testing a range
    /// the encoding cannot produce would have demanded an inverse that does
    /// not exist and cannot be needed. The domain is the field's width.
    #[test]
    fn encode_inverts_decode_over_every_field_the_tables_declare() {
        let mut checked = 0usize;
        for set in super::super::isa_tables::BIT_SETS {
            for f in set.fields {
                /* Exhaustive to 16 bits. Wider fields are swept at their
                 * boundaries instead: `branch_offset` is 31 bits and `Shr1`,
                 * so "wide implies Identity" is not a rule to lean on. */
                let wide: [u32; 6] = {
                    let max = if f.bits >= 32 { u32::MAX } else { (1u32 << f.bits) - 1 };
                    [0, 1, 2, max / 2, max - 1, max]
                };
                let narrow: alloc::vec::Vec<u32> =
                    if f.bits <= 16 { (0..=((1u32 << f.bits) - 1)).collect() } else { wide.to_vec() };
                for raw in narrow {
                    let logical = f.xform.decode(raw);
                    assert_eq!(
                        f.xform.encode(logical),
                        Some(raw),
                        "field {} ({} bits): raw {raw} decoded to {logical} and did not come back",
                        f.name,
                        f.bits
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 1000, "only {checked} values checked; the tables look empty");
    }

    /// A value with no encoding is refused, not clamped to the nearest one.
    #[test]
    fn a_value_the_transform_cannot_hold_is_refused() {
        assert_eq!(Xform::SubOne.encode(0), None, "a repeat count of 0 has no encoding");
        assert_eq!(Xform::WrapMod16.encode(0), None, "a group of 0 words has no encoding");
        assert_eq!(Xform::WrapMod16.encode(17), None, "17 words does not fit a 4-bit length");
        assert_eq!(Xform::Shr1.encode(3), None, "an odd byte offset has bit 0 set");
        assert_eq!(Xform::Shr2.encode(6), None, "6 is not a multiple of 4");
        /* ... and the values either side of each refusal do encode, so the
         * refusal is a boundary and not a blanket. */
        assert_eq!(Xform::SubOne.encode(1), Some(0));
        assert_eq!(Xform::WrapMod16.encode(16), Some(0));
        assert_eq!(Xform::Shr1.encode(4), Some(2));
        assert_eq!(Xform::Shr2.encode(8), Some(2));
    }

    /// `Region::set` refuses a name the variant does not have and a value
    /// wider than the field, rather than writing something plausible.
    #[test]
    fn setting_a_field_refuses_rather_than_truncating() {
        /* The group header: every group has one, so this needs no particular
         * instruction to exist. */
        let set = bit_set(BitSetId::IgrpHdr);
        let (vi, v) = set
            .variants
            .iter()
            .enumerate()
            .find(|(_, v)| v.uses.iter().any(|u| u.fixed.is_none()))
            .expect("the header has a variant with a settable field");
        let mut r = Region::new(BitSetId::IgrpHdr, vi as u16);

        assert_eq!(
            r.set("a_field_no_variant_has", 1),
            Err(SetError::NoSuchField { name: "a_field_no_variant_has" }),
            "a misspelled name must not silently do nothing"
        );

        /* A settable field of this variant, given a value one bit too wide. */
        let u = v.uses.iter().find(|u| u.fixed.is_none()).unwrap();
        let f = &set.fields[u.field];
        if f.bits < 32 && f.xform == Xform::Identity {
            let too_big = 1u32 << f.bits;
            assert_eq!(
                r.set(u.name, too_big),
                Err(SetError::TooWide { name: u.name, val: too_big, bits: f.bits }),
                "{} is {} bits and took a {}-bit value",
                u.name,
                f.bits,
                f.bits + 1
            );
            assert_eq!(r.set(u.name, too_big - 1), Ok(()), "the widest value that fits must be accepted");
        }
    }

    /// A region built from scratch and written out is the same bytes as one
    /// decoded from those bytes: construction and read-back are one format.
    #[test]
    fn a_constructed_region_encodes_and_decodes_as_itself() {
        let set = bit_set(BitSetId::IgrpHdr);
        let (vi, v) = set
            .variants
            .iter()
            .enumerate()
            .find(|(_, v)| v.uses.iter().any(|u| u.fixed.is_none()))
            .expect("a header variant with a settable field");
        let mut r = Region::new(BitSetId::IgrpHdr, vi as u16);
        /* Give every settable field a value that fits. */
        for u in v.uses.iter().filter(|u| u.fixed.is_none()) {
            let f = &set.fields[u.field];
            if f.reserved.is_some() {
                continue;
            }
            let raw = if f.bits >= 32 { 1 } else { (1u32 << f.bits) - 1 };
            if let Some(logical) = Some(f.xform.decode(raw)) {
                let _ = r.set(u.name, logical);
            }
        }
        let mut buf = [0u8; 32];
        let n = encode_region(&r, &mut buf, 0);
        assert_eq!(n, v.bytes as usize);
        let back = decode_as(&buf, 0, BitSetId::IgrpHdr, vi as u16);
        assert_eq!(back.vals, r.vals, "a constructed region did not read back as itself");
    }
}
