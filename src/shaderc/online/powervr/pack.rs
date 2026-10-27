// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Which slot each instruction of a new group takes, and what the header says
//! about the set.
//!
//! The inverse of the reading that [`super::decode`] does privately: given a
//! header class, that finds the slots; given the phases, this finds the class.
//! It also refuses, which the reading direction never has to.
//!
//! Every rule here was measured over the 576,296 groups of the offline table
//! before this file existed (`roundtrip::what_decides_which_slot_an_instruction_occupies`,
//! `::which_phase_sets_a_group_can_have`, `::what_signals_the_phase_two_family`),
//! because a packer built on an assumption and then checked against the same
//! assumption would be circular.
//!
//! # The three facts it is built from
//!
//! **The slot is a function of the phase alone.** A lookup, not a search. It
//! is not injective -- phases 2 and 3 both take slot 2, phases 0 and 7 both
//! take slot 0 -- and the colliding pairs never co-occur in the corpus, so
//! those are exclusions to enforce rather than cases to resolve.
//!
//! **Only twelve phase sets occur**, out of the 255 a free choice of eight
//! phases allows. Membership is therefore a whitelist: a set outside it is
//! refused rather than encoded, because a group encoding a combination the
//! hardware does not expect decodes as a DIFFERENT group rather than failing.
//!
//! **Nothing signals the phase-2 family.** `oporg` names only the coarse class;
//! which of the family is present is implicit in the byte layout, which the
//! decoder recovers by trying each subset. So this sets the class and the
//! caller emits the instructions -- there is no subset field to fill.

/// Why a set of phases cannot be packed into one group.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotPackable {
    /// Two phases that take the same slot. The pairs are (2, 3) and (0, 7).
    SlotCollision { a: u8, b: u8, slot: u8 },
    /// A phase the ISA has no slot for.
    UnknownPhase { phase: u8 },
    /// A combination that does not occur in the corpus. Refused rather than
    /// guessed at: see the whitelist argument above.
    NotAKnownSet,
}

/// The slot a phase occupies, measured over every group in the table.
pub fn slot_of(phase: u8) -> Option<u8> {
    Some(match phase {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        4 => 3,
        5 => 4,
        6 => 5,
        7 => 0,
        _ => return None,
    })
}

/// The twelve phase sets that occur, each sorted ascending, most common first.
///
/// The order is by frequency so a reader sees which cases matter: a lone
/// phase 0 is 76.3 % of the corpus and the last four together are under 1.5 %.
pub const LEGAL_SETS: &[&[u8]] = &[
    &[0],
    &[7],
    &[0, 1, 4, 5],
    &[0, 2],
    &[0, 1, 3, 4, 5],
    &[0, 1],
    &[0, 4],
    &[6],
    &[0, 3, 4, 5],
    &[3],
    &[0, 1, 2],
    &[5],
];

/// May these phases share a group? `phases` must be sorted and deduplicated.
///
/// Both checks are made, and in this order, so the error says which problem it
/// is: a collision names the two phases and the slot they fight over, which is
/// actionable, where `NotAKnownSet` only says the combination is unknown.
pub fn check(phases: &[u8]) -> Result<(), NotPackable> {
    let mut seen: [Option<u8>; 6] = [None; 6];
    for &p in phases {
        let slot = slot_of(p).ok_or(NotPackable::UnknownPhase { phase: p })?;
        if let Some(other) = seen[slot as usize] {
            return Err(NotPackable::SlotCollision { a: other, b: p, slot });
        }
        seen[slot as usize] = Some(p);
    }
    if LEGAL_SETS.contains(&phases) {
        Ok(())
    } else {
        Err(NotPackable::NotAKnownSet)
    }
}

/// The `oporg` value for a main-ALU group's phase set.
///
/// It says which of the base phases, the phase-2 family and the backend phase
/// are present, and nothing about WHICH of the family -- that is implicit in
/// the bytes.
pub fn oporg_for(phases: &[u8]) -> Option<u32> {
    let has = |p: u8| phases.contains(&p);
    let p2 = has(2) || has(3) || has(4) || has(5);
    Some(match (has(0), has(1), p2, has(6)) {
        (true, false, false, false) => 0,
        (false, false, true, false) => 1,
        (false, false, false, true) => 2,
        (true, true, false, false) => 3,
        (true, false, true, false) => 4,
        (true, true, true, false) => 5,
        (true, false, true, true) => 6,
        (true, true, true, true) => 7,
        _ => return None,
    })
}

/// The `opcnt` value for a bitwise or control group: a presence bitmask over
/// phases 0, 1 and 2.
pub fn opcnt_for(phases: &[u8]) -> Option<u32> {
    let mut m = 0u32;
    for &p in phases {
        m |= match p {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => return None,
        };
    }
    Some(m)
}

/// The header variant a group of this ALU type takes.
///
/// Measured over all 576,296 groups of the offline table
/// (`roundtrip::what_decides_the_header_variant_and_the_optional_sections`):
/// the variant is a function of `alutype` alone, main -> `main`, bitwise ->
/// `bitwise`, control -> `control`, with no exceptions.
///
/// **`main_brief` never occurs.** The builder's first version chose the first
/// variant carrying `oporg`, `length` and `da`, which is `main_brief` -- a
/// form the reference compiler does not emit once in half a million groups.
/// Choosing it by the name `main` instead would have been the same guess with
/// a different constant, which is why this is a measured rule with a test
/// asserting it rather than a name.
pub fn header_variant_of(alutype: u32) -> Option<u16> {
    let want = match alutype {
        0 => "main",
        2 => "bitwise",
        3 => "control",
        _ => return None,
    };
    let set = super::isa::bit_set(super::isa::BitSetId::IgrpHdr);
    set.variants.iter().position(|v| v.name == want).map(|i| i as u16)
}

/// Is the upper source section present?
///
/// Measured: it is absent in exactly 30,260 groups, which are exactly the
/// groups with no lower section either -- control groups carrying no sections
/// at all. So the upper section is present if and only if the lower one is,
/// and a group with operands always has both even when the upper holds only
/// zeros.
pub fn upper_present(has_lower: bool) -> bool {
    has_lower
}

/// Is the ISS section present?
///
/// Measured: absent in 99,539 groups, and none of them is a main-ALU group.
/// Every one of the 476,757 main groups has one; no bitwise or control group
/// does. So it follows the ALU type, not the operands -- except that a group
/// with no sections at all has none of anything.
pub fn iss_present(alutype: u32, has_lower: bool) -> bool {
    alutype == 0 && has_lower
}
