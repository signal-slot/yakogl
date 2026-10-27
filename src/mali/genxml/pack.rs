// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The bit packer the generated descriptors ([`super::v10`]) call.
//!
//! Mesa's packers OR every field into its word and check the ranges only in
//! debug builds (`gen_pack.py:289-336`, `util/bitpack_helpers.h:53-66`);
//! `ALIGN_POT` rounds an `align(n)` field up and the LOD fields are clamped.
//! A descriptor built from a wrong value is then a descriptor of some other
//! state, and nothing says so. Here every such value is a [`PackError`]
//! naming the structure, the field and the cause; one [`Refusal`] per cause,
//! each with its own message.
//!
//! Two fields that share bits (the members of a union in v10.xml: the
//! render-target variants, the blend shader and fixed-function halves, the
//! preload mask and its single-register flags, ...) may each be packed, but
//! not both with a non-zero value: the packer remembers which bits a
//! non-zero field took and refuses the second one.

use core::fmt;

/// Why a value was refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    /// The value needs more bits than the field has.
    TooWide { value: u64, bits: u32 },
    /// A signed value outside the field's two's-complement range.
    OutOfRange { value: i64, bits: u32 },
    /// `shr(shift)`: the value has some of its low `shift` bits set.
    LowBitsSet { value: u64, shift: u32 },
    /// `log2`: zero, or a value that is not a power of two.
    NotPowerOfTwo { value: u64 },
    /// `minus(minus)`: the value is below `minus`.
    BelowMinus { value: u64, minus: u32 },
    /// `align(align)`: the value is not a multiple of `align`.
    NotAligned { value: u64, align: u32 },
    /// A non-zero value the enum does not name.
    NotInEnum { value: u32, enumeration: &'static str },
    /// An enum field at zero while zero is not one of the enum's values, and
    /// no non-zero field that shares its bits (the active arm of a union)
    /// stands in for it. Mesa's header leaves such a field at zero and packs
    /// it; the zero names no state.
    EnumUnset,
    /// A ulod/slod value that is NaN or, rounded to 1/256, outside the field.
    NotFixedPoint { value: f32, bits: u32, signed: bool },
    /// Another field that shares bits with this one is already non-zero.
    SharedBits { other: &'static str },
    /// Unpacking: bits of word `word` that no field owns are set.
    ReservedBits { word: u32, bits: u32 },
    /// Unpacking: the value does not fit the Rust type of the field.
    TooLargeForType { value: u64 },
    /// `unpack_as`: the arm names something that is not a member of a union
    /// of the structure.
    NotAMember,
    /// The generated code asked for a bit past the end of the structure, or
    /// for more fields than it declared (a generator defect).
    Layout,
}

/// A refused field: which structure was being packed or unpacked, the field's
/// path in it (v10.xml's names, `.` between a struct and its member) and why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackError {
    pub structure: &'static str,
    pub field: &'static str,
    pub refusal: Refusal,
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (s, n) = (self.structure, self.field);
        match self.refusal {
            Refusal::TooWide { value, bits } => write!(f, "mali {s}.{n}: {value:#x} does not fit {bits} bits"),
            Refusal::OutOfRange { value, bits } => {
                write!(f, "mali {s}.{n}: {value} is outside the signed {bits}-bit range")
            }
            Refusal::LowBitsSet { value, shift } => {
                write!(f, "mali {s}.{n}: {value:#x} has low bits set that shr({shift}) would drop")
            }
            Refusal::NotPowerOfTwo { value } => write!(f, "mali {s}.{n}: {value:#x} is not a power of two (log2)"),
            Refusal::BelowMinus { value, minus } => {
                write!(f, "mali {s}.{n}: {value} is below the {minus} that minus({minus}) subtracts")
            }
            Refusal::NotAligned { value, align } => {
                write!(f, "mali {s}.{n}: {value:#x} is not a multiple of {align} (align({align}))")
            }
            Refusal::NotInEnum { value, enumeration } => {
                write!(f, "mali {s}.{n}: {value} is not a value of the enum {enumeration}")
            }
            Refusal::EnumUnset => write!(f, "mali {s}.{n}: enum value 0 is not a named value"),
            Refusal::NotFixedPoint { value, bits, signed } => write!(
                f,
                "mali {s}.{n}: {value} is outside the {} {bits}-bit LOD range (8 fractional bits)",
                if signed { "signed" } else { "unsigned" }
            ),
            Refusal::SharedBits { other } => {
                write!(f, "mali {s}.{n}: shares bits with {other}, and both are non-zero")
            }
            Refusal::ReservedBits { word, bits } => {
                write!(f, "mali {s}: word {word} has bits {bits:#010x} set that no field owns")
            }
            Refusal::TooLargeForType { value } => {
                write!(f, "mali {s}.{n}: {value:#x} does not fit the field's Rust type")
            }
            Refusal::NotAMember => write!(f, "mali {s}: the arm names a field that is not a member of a union"),
            Refusal::Layout => write!(f, "mali {s}.{n}: the generated layout is inconsistent"),
        }
    }
}

/// What the packer needs to know of an enum: its name in v10.xml and whether
/// zero is one of its values.
#[derive(Clone, Copy, Debug)]
pub struct EnumDef {
    pub name: &'static str,
    pub zero_named: bool,
}

/// One structure being packed: `W` 32-bit words and room to remember the
/// bits of up to `L` non-zero fields (the generator passes its field count).
pub struct Packer<const W: usize, const L: usize> {
    structure: &'static str,
    words: [u32; W],
    /* (first bit, last bit, field) of each non-zero field so far */
    claims: [(u32, u32, &'static str); L],
    n: usize,
    /* the same for each enum field left at an unnamed zero, judged in finish() */
    unset: [(u32, u32, &'static str); L],
    n_unset: usize,
}

impl<const W: usize, const L: usize> Packer<W, L> {
    pub fn new(structure: &'static str) -> Self {
        Packer { structure, words: [0; W], claims: [(0, 0, ""); L], n: 0, unset: [(0, 0, ""); L], n_unset: 0 }
    }

    fn err(&self, field: &'static str, refusal: Refusal) -> PackError {
        PackError { structure: self.structure, field, refusal }
    }

    /// The raw bits of one field, `size` bits from bit `start`.
    pub fn put(&mut self, field: &'static str, start: u32, size: u32, raw: u64) -> Result<(), PackError> {
        if size < 64 && raw >> size != 0 {
            return Err(self.err(field, Refusal::TooWide { value: raw, bits: size }));
        }
        let end = start + size - 1;
        if end as usize >= W * 32 {
            return Err(self.err(field, Refusal::Layout));
        }
        if raw == 0 {
            return Ok(());
        }
        if let Some(&(_, _, other)) = self.claims[..self.n].iter().find(|&&(s, e, _)| s <= end && start <= e) {
            return Err(self.err(field, Refusal::SharedBits { other }));
        }
        match self.claims.get_mut(self.n) {
            Some(c) => *c = (start, end, field),
            None => return Err(self.err(field, Refusal::Layout)),
        }
        self.n += 1;
        /* up to 64 bits from any bit of a word: at most three words */
        let wide = u128::from(raw) << (start % 32);
        let first = (start / 32) as usize;
        let count = (end / 32) as usize - first + 1;
        for (i, w) in self.words.iter_mut().skip(first).take(count).enumerate() {
            /* the i-th 32-bit slice of `wide` */
            *w |= (wide >> (32 * i)) as u32;
        }
        Ok(())
    }

    /// A plain unsigned field.
    pub fn uint(&mut self, field: &'static str, start: u32, size: u32, v: u64) -> Result<(), PackError> {
        self.put(field, start, size, v)
    }

    /// `modifier="shr(n)"`: the field holds `v >> n`, and `v`'s low bits must be clear.
    pub fn uint_shr(&mut self, field: &'static str, start: u32, size: u32, v: u64, n: u32) -> Result<(), PackError> {
        if v & ((1u64 << n) - 1) != 0 {
            return Err(self.err(field, Refusal::LowBitsSet { value: v, shift: n }));
        }
        self.put(field, start, size, v >> n)
    }

    /// `modifier="minus(n)"`: the field holds `v - n`.
    pub fn uint_minus(&mut self, field: &'static str, start: u32, size: u32, v: u64, n: u32) -> Result<(), PackError> {
        match v.checked_sub(u64::from(n)) {
            Some(raw) => self.put(field, start, size, raw),
            None => Err(self.err(field, Refusal::BelowMinus { value: v, minus: n })),
        }
    }

    /// `modifier="align(n)"`: the field holds `v`, which must already be a
    /// multiple of `n` (Mesa's `ALIGN_POT` would round it up).
    pub fn uint_align(&mut self, field: &'static str, start: u32, size: u32, v: u64, n: u32) -> Result<(), PackError> {
        if !v.is_multiple_of(u64::from(n)) {
            return Err(self.err(field, Refusal::NotAligned { value: v, align: n }));
        }
        self.put(field, start, size, v)
    }

    /// `modifier="log2"`: the field holds log2(`v`); `v` is a power of two.
    pub fn uint_log2(&mut self, field: &'static str, start: u32, size: u32, v: u64) -> Result<(), PackError> {
        if !v.is_power_of_two() {
            return Err(self.err(field, Refusal::NotPowerOfTwo { value: v }));
        }
        self.put(field, start, size, u64::from(v.trailing_zeros()))
    }

    /// A two's-complement field.
    pub fn sint(&mut self, field: &'static str, start: u32, size: u32, v: i64) -> Result<(), PackError> {
        if size < 64 {
            let half = 1i64 << (size - 1);
            if v < -half || v >= half {
                return Err(self.err(field, Refusal::OutOfRange { value: v, bits: size }));
            }
        }
        let mask = if size == 64 { u64::MAX } else { (1u64 << size) - 1 };
        /* the two's-complement bits of a value checked to fit */
        self.put(field, start, size, (v as u64) & mask)
    }

    /// An enum field: a value the enum names. Zero that the enum does not
    /// name is Mesa's header default for a field without one; it is accepted
    /// here only if [`Packer::finish`] finds the field's bits taken by a
    /// non-zero field of another arm of a union.
    pub fn enum_value(
        &mut self,
        field: &'static str,
        start: u32,
        size: u32,
        raw: u32,
        known: bool,
        def: &EnumDef,
    ) -> Result<(), PackError> {
        if raw != 0 && !known {
            return Err(self.err(field, Refusal::NotInEnum { value: raw, enumeration: def.name }));
        }
        self.put(field, start, size, u64::from(raw))?;
        if raw == 0 && !def.zero_named {
            match self.unset.get_mut(self.n_unset) {
                Some(u) => *u = (start, start + size - 1, field),
                None => return Err(self.err(field, Refusal::Layout)),
            }
            self.n_unset += 1;
        }
        Ok(())
    }

    /// The fixed-point value of a LOD, `v` * 256 rounded half away from zero
    /// as Mesa's `llroundf`, or `None` when it is NaN or outside `lo..=hi`.
    fn fixed(v: f32, lo: i64, hi: i64) -> Option<i64> {
        let x = v * 256.0;
        /* NaN fails both comparisons, and so does anything past 2^24, far
         * outside every LOD field (16 bits at most); below it `as i64` is
         * exact truncation toward zero and `x - t` is exact. No `f32::round`:
         * core has none on a no_std target. */
        if !(x > -16_777_216.0 && x < 16_777_216.0) {
            return None;
        }
        let t = x as i64;
        let frac = x - t as f32;
        let r = if frac >= 0.5 {
            t + 1
        } else if frac <= -0.5 {
            t - 1
        } else {
            t
        };
        (lo..=hi).contains(&r).then_some(r)
    }

    /// `type="ulod"`: unsigned, 8 fractional bits.
    pub fn ulod(&mut self, field: &'static str, start: u32, size: u32, v: f32) -> Result<(), PackError> {
        match Self::fixed(v, 0, (1i64 << size) - 1) {
            Some(raw) => self.put(field, start, size, raw as u64),
            None => Err(self.err(field, Refusal::NotFixedPoint { value: v, bits: size, signed: false })),
        }
    }

    /// `type="slod"`: signed, 8 fractional bits.
    pub fn slod(&mut self, field: &'static str, start: u32, size: u32, v: f32) -> Result<(), PackError> {
        let half = 1i64 << (size - 1);
        match Self::fixed(v, -half, half - 1) {
            Some(raw) => self.put(field, start, size, (raw as u64) & ((1u64 << size) - 1)),
            None => Err(self.err(field, Refusal::NotFixedPoint { value: v, bits: size, signed: true })),
        }
    }

    /// The packed words as little-endian bytes (`N` is `4 * W`), unless an
    /// enum field was left at a zero the enum does not name and no non-zero
    /// field shares its bits.
    pub fn finish<const N: usize>(&self) -> Result<[u8; N], PackError> {
        for &(start, end, field) in &self.unset[..self.n_unset] {
            if !self.claims[..self.n].iter().any(|&(s, e, _)| s <= end && start <= e) {
                return Err(self.err(field, Refusal::EnumUnset));
            }
        }
        Ok(self.bytes())
    }

    fn bytes<const N: usize>(&self) -> [u8; N] {
        let mut out = [0u8; N];
        for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.words.iter()) {
            *chunk = w.to_le_bytes();
        }
        out
    }
}

/// The members a caller chose for `unpack_as`, out of the fields of a
/// structure that share bits with another one (its unions' members).
pub struct Arm<'a> {
    structure: &'static str,
    chosen: &'a [&'a str],
}

impl<'a> Arm<'a> {
    /// Every name in `chosen` (v10.xml's path) must be one of `members`.
    pub fn new(structure: &'static str, chosen: &'a [&'a str], members: &[&str]) -> Result<Self, PackError> {
        if chosen.iter().all(|c| members.contains(c)) {
            Ok(Arm { structure, chosen })
        } else {
            Err(PackError { structure, field: "", refusal: Refusal::NotAMember })
        }
    }

    /// Is `name` chosen? If so its bits (`mask`) join `owned`, and must not be
    /// bits of another chosen member (`taken`).
    pub fn claim<const W: usize>(
        &self,
        owned: &mut [u32; W],
        taken: &mut [u32; W],
        name: &'static str,
        mask: &[u32; W],
    ) -> Result<bool, PackError> {
        if !self.chosen.contains(&name) {
            return Ok(false);
        }
        if taken.iter().zip(mask).any(|(t, m)| t & m != 0) {
            return Err(PackError {
                structure: self.structure,
                field: name,
                refusal: Refusal::SharedBits { other: "another chosen member" },
            });
        }
        for ((o, t), m) in owned.iter_mut().zip(taken.iter_mut()).zip(mask) {
            *o |= m;
            *t |= m;
        }
        Ok(true)
    }
}

/// One packed structure being read back.
pub struct Unpacker<const W: usize> {
    structure: &'static str,
    words: [u32; W],
}

impl<const W: usize> Unpacker<W> {
    /// `bytes` as `W` little-endian words; refuses a set bit that `owned`
    /// (a mask per word of the bits some field holds) does not cover.
    pub fn new(structure: &'static str, bytes: &[u8], owned: &[u32; W]) -> Result<Self, PackError> {
        let mut words = [0u32; W];
        for (w, chunk) in words.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *w = u32::from_le_bytes(*chunk);
        }
        for (i, (w, m)) in words.iter().zip(owned.iter()).enumerate() {
            if w & !m != 0 {
                return Err(PackError {
                    structure,
                    field: "",
                    refusal: Refusal::ReservedBits { word: i as u32, bits: w & !m },
                });
            }
        }
        Ok(Unpacker { structure, words })
    }

    fn err(&self, field: &'static str, refusal: Refusal) -> PackError {
        PackError { structure: self.structure, field, refusal }
    }

    /// The raw `size` bits from bit `start`.
    pub fn uint(&self, start: u32, size: u32) -> u64 {
        let first = (start / 32) as usize;
        let last = ((start + size - 1) / 32) as usize;
        let mut wide = 0u128;
        /* the generator checks that every field ends inside the structure */
        for (i, w) in self.words.iter().skip(first).take(last - first + 1).enumerate() {
            wide |= u128::from(*w) << (32 * i);
        }
        let mask = if size == 64 { u64::MAX } else { (1u64 << size) - 1 };
        /* the field's bits, `size` <= 64 of them */
        ((wide >> (start % 32)) as u64) & mask
    }

    /// A one-bit field.
    pub fn bit(&self, start: u32) -> bool {
        self.uint(start, 1) != 0
    }

    /// A two's-complement field, sign-extended.
    pub fn sint(&self, start: u32, size: u32) -> i64 {
        let raw = self.uint(start, size);
        if size == 64 {
            return raw as i64;
        }
        let sign = 1u64 << (size - 1);
        /* sign extension: (raw ^ sign) - sign in two's complement */
        (raw ^ sign).wrapping_sub(sign) as i64
    }

    /// `shr(n)`: the value `raw << n`.
    pub fn uint_shr(&self, field: &'static str, start: u32, size: u32, n: u32) -> Result<u64, PackError> {
        let raw = self.uint(start, size);
        if size + n > 64 && raw >> (64 - n) != 0 {
            return Err(self.err(field, Refusal::TooLargeForType { value: raw }));
        }
        Ok(raw << n)
    }

    /// `minus(n)`: the value `raw + n`.
    pub fn uint_minus(&self, field: &'static str, start: u32, size: u32, n: u32) -> Result<u64, PackError> {
        let raw = self.uint(start, size);
        raw.checked_add(u64::from(n)).ok_or_else(|| self.err(field, Refusal::TooLargeForType { value: raw }))
    }

    /// `align(n)`: the value as stored, which must be a multiple of `n`.
    pub fn uint_align(&self, field: &'static str, start: u32, size: u32, n: u32) -> Result<u64, PackError> {
        let v = self.uint(start, size);
        if !v.is_multiple_of(u64::from(n)) {
            return Err(self.err(field, Refusal::NotAligned { value: v, align: n }));
        }
        Ok(v)
    }

    /// `log2`: the value `1 << raw`.
    pub fn uint_log2(&self, field: &'static str, start: u32, size: u32) -> Result<u64, PackError> {
        let raw = self.uint(start, size);
        if raw >= 64 {
            return Err(self.err(field, Refusal::TooLargeForType { value: raw }));
        }
        Ok(1u64 << raw)
    }

    /// An enum field: zero or a value `known` accepts.
    pub fn enum_value(
        &self,
        field: &'static str,
        start: u32,
        size: u32,
        known: fn(u32) -> bool,
        enumeration: &'static str,
    ) -> Result<u32, PackError> {
        let v = self.narrow(field, self.uint(start, size))?;
        if v != 0 && !known(v) {
            return Err(self.err(field, Refusal::NotInEnum { value: v, enumeration }));
        }
        Ok(v)
    }

    /// `type="ulod"`: unsigned, 8 fractional bits.
    pub fn ulod(&self, start: u32, size: u32) -> f32 {
        self.uint(start, size) as f32 / 256.0
    }

    /// `type="slod"`: signed, 8 fractional bits.
    pub fn slod(&self, start: u32, size: u32) -> f32 {
        self.sint(start, size) as f32 / 256.0
    }

    /// A value for a `u32` field.
    pub fn narrow(&self, field: &'static str, v: u64) -> Result<u32, PackError> {
        u32::try_from(v).map_err(|_| self.err(field, Refusal::TooLargeForType { value: v }))
    }

    /// A value for an `i32` field.
    pub fn narrow_signed(&self, field: &'static str, v: i64) -> Result<i32, PackError> {
        i32::try_from(v).map_err(|_| self.err(field, Refusal::TooLargeForType { value: v as u64 }))
    }
}
