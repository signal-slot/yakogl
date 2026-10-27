// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Decoding a USC binary into instruction groups.
//!
//! A shader is a sequence of *instruction groups*. A group's header says how
//! long it is (in 16-bit words), which ALU it is for, and which pipeline
//! phases carry an instruction; the instructions follow in reverse phase
//! order, then the source, ISS and destination sections, then padding.
//!
//! Two things in a group are not written in its bytes and have to be known:
//!
//! * Which phases are present. The header's `oporg` names a coarse group
//!   (`p0_p1_p2`), not the individual phase-2 slots, so the decoder tries the
//!   possible phase sets and keeps the one that fits the group's length. The
//!   phase matters beyond bookkeeping: the same instruction form means a
//!   different operation in a different phase (main op 6 is FMAD in phase 0
//!   and MOVC in the move phase), so it has to be settled before the bytes
//!   can be read.
//! * Which sections are present. An ISS byte carries no discriminator at all,
//!   so its presence cannot be read off the bytes. Mesa's variant selectors
//!   (`pco_igrp_iss_variant`, `pco_igrp_src_variant`, `pco_igrp_dest_variant`)
//!   settle it structurally, and [`sections_present`] is that rule.
//!
//! Every parse is required to be unique: a group that decodes two ways is a
//! decoder defect, and [`decode_group`] returns [`DecodeError::Ambiguous`]
//! rather than picking one. Over the 2,476 blobs of the offline table
//! (685,564 groups) every group decodes exactly one way.

use super::isa::*;
use super::isa_tables::CTRLOP_VARIANTS;

/// The most instructions one group can hold: phases p0, p1, p2/p2_pck,
/// p2_tst, p2_mov and the backend.
pub const MAX_INSTRS: usize = 6;

pub const ALU_MAIN: u32 = 0;
pub const ALU_BITWISE: u32 = 2;
pub const ALU_CONTROL: u32 = 3;

/// One decoded instruction group.
#[derive(Clone, Copy)]
pub struct Group {
    pub hdr: Region,
    pub alutype: u32,
    /// The instructions, in slot order, each with the phase it was read as.
    pub instrs: [Option<(u8, Region)>; MAX_INSTRS],
    pub lower: Option<Region>,
    pub upper: Option<Region>,
    pub iss: Option<Region>,
    pub dst: Option<Region>,
    /// A single 0xff byte that pads the group to an even length.
    pub word_pad: u8,
    /// The i-cache alignment block that closes a shader: `0xf0 | words`
    /// followed by 0xff filler, `align_pad` bytes in all.
    pub align_pad: u8,
    /// The group's whole length in bytes, header and padding included.
    pub total: usize,
}

impl Group {
    /// Slot, phase, instruction. The slot is the array index and is not
    /// stored a second time: a derived field computed from a slot the caller
    /// passed separately is a field that can disagree with where the
    /// instruction actually sits, which is exactly how `da` was once built
    /// wrong for every backend-phase group in the corpus.
    pub fn instrs(&self) -> impl Iterator<Item = (u8, u8, &Region)> {
        self.instrs.iter().enumerate().filter_map(|(i, o)| o.as_ref().map(|(ph, r)| (i as u8, *ph, r)))
    }

    /// The group's length in bytes as its header states it.
    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The group runs past the end of the buffer.
    Truncated,
    /// No reading of the bytes is consistent with the header.
    NoParse,
    /// More than one reading is consistent; the decoder will not guess.
    Ambiguous,
}

/// Header bytes 0 and 1 are always there; byte 2 only in the extended form.
fn hdr_variant(buf: &[u8], off: usize) -> Option<(u16, u32)> {
    let set = bit_set(BitSetId::IgrpHdr);
    let ext = buf.get(off + 1)? >> 7;
    if ext == 0 {
        // main_brief: no alutype field of its own, always the main ALU.
        let i = set.variants.iter().position(|v| v.name == "main_brief")?;
        return Some((i as u16, ALU_MAIN));
    }
    let alutype = (*buf.get(off + 2)? >> 5) as u32 & 3;
    let name = match alutype {
        ALU_MAIN => "main",
        ALU_BITWISE => "bitwise",
        ALU_CONTROL => "control",
        _ => return None,
    };
    let i = set.variants.iter().position(|v| v.name == name)?;
    Some((i as u16, alutype))
}

/// The phase slots an `oporg` value allows: the always-present ones, whether
/// the phase-2 family is in the group, and whether the backend phase is.
fn oporg_slots(oporg: u32) -> Option<(&'static [u8], bool, bool)> {
    Some(match oporg {
        0b000 => (&[0], false, false),    // p0
        0b001 => (&[], true, false),      // p2
        0b010 => (&[], false, true),      // be
        0b011 => (&[0, 1], false, false), // p0_p1
        0b100 => (&[0], true, false),     // p0_p2
        0b101 => (&[0, 1], true, false),  // p0_p1_p2
        0b110 => (&[0], true, true),      // p0_p2_be
        0b111 => (&[0, 1], true, true),   // p0_p1_p2_be
        _ => return None,
    })
}

/// The phases a slot can be read as, and the bit set holding it.
fn slot_phases(alutype: u32, slot: u8) -> (BitSetId, &'static [u8]) {
    match alutype {
        ALU_MAIN => match slot {
            0 => (BitSetId::Main, &[PH_0]),
            1 => (BitSetId::Main, &[PH_1]),
            2 => (BitSetId::Main, &[PH_2, PH_2_PCK]),
            3 => (BitSetId::Main, &[PH_2_TST]),
            4 => (BitSetId::Main, &[PH_2_MOV]),
            _ => (BitSetId::Backend, &[PH_BACKEND]),
        },
        ALU_BITWISE => (
            BitSetId::Bitwise,
            match slot {
                0 => &[PH_0],
                1 => &[PH_1],
                _ => &[PH_2],
            },
        ),
        _ => (BitSetId::Ctrl, &[PH_CTRL]),
    }
}

/// Which sections a group carries. This is not in the bytes: Mesa's variant
/// selectors decide it from the group, and they say - an ISS byte for every
/// MAIN group and no other; a lower and an upper source section for every
/// MAIN and BITWISE group, each at least one byte even when nothing is in it;
/// and for CONTROL, a section only where that half has a source, which the
/// header's decode-assist field reports as 0 when there is none at all.
/// A destination section exactly when the header says a write port is used.
///
/// Returns the candidate (lower, upper, iss) shapes, in the order to try.
fn sections_present(alutype: u32, da: u32) -> &'static [(bool, bool, bool)] {
    match alutype {
        ALU_MAIN => &[(true, true, true)],
        ALU_BITWISE => &[(true, true, false)],
        _ if da == 0 => &[(false, false, false)],
        _ => &[(true, true, false), (true, false, false), (false, true, false)],
    }
}

/// Decode the instruction group at `buf[off..]`.
pub fn decode_group(buf: &[u8], off: usize) -> Result<Group, DecodeError> {
    if off + 2 > buf.len() {
        return Err(DecodeError::Truncated);
    }
    let words = buf[off] & 0xf;
    let total = if words == 0 { 16 } else { words as usize } * 2;
    if off + total > buf.len() {
        return Err(DecodeError::Truncated);
    }
    let grp = &buf[off..off + total];

    let (hv, alutype) = hdr_variant(grp, 0).ok_or(DecodeError::NoParse)?;
    let hdr = decode_as(grp, 0, BitSetId::IgrpHdr, hv);
    let hdr_len = hdr.bytes();
    let da = hdr.get("da").unwrap_or(0);
    let w0p = hdr.get("w0p").unwrap_or(0) != 0;
    let w1p = hdr.get("w1p").unwrap_or(0) != 0;

    let mut found: Option<Group> = None;
    let mut count = 0usize;

    let mut slot_sets: [[u8; MAX_INSTRS]; 8] = [[0; MAX_INSTRS]; 8];
    let mut slot_lens = [0usize; 8];
    let n_slot_sets = build_slot_sets(alutype, &hdr, &mut slot_sets, &mut slot_lens);

    for s in 0..n_slot_sets {
        let slots = &slot_sets[s][..slot_lens[s]];
        let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
        walk(
            grp,
            total,
            hdr_len,
            alutype,
            da,
            w0p,
            w1p,
            slots,
            slots.len(),
            hdr_len,
            &mut instrs,
            &hdr,
            &mut found,
            &mut count,
        );
    }

    match count {
        0 => Err(DecodeError::NoParse),
        1 => {
            let mut g = found.ok_or(DecodeError::NoParse)?;
            g.total = total;
            Ok(g)
        }
        _ => Err(DecodeError::Ambiguous),
    }
}

/// The phase-slot sets the header allows, most specific first.
fn build_slot_sets(alutype: u32, hdr: &Region, out: &mut [[u8; MAX_INSTRS]; 8], lens: &mut [usize; 8]) -> usize {
    match alutype {
        ALU_MAIN => {
            let oporg = hdr.raw("oporg").unwrap_or(0);
            let Some((base, has_p2, has_be)) = oporg_slots(oporg) else {
                return 0;
            };
            let mut n = 0;
            // The phase-2 family is any non-empty subset of {2, 3, 4} when
            // `oporg` says the group has one, and nothing when it does not.
            for mask in if has_p2 { 1..8u8 } else { 0..1u8 } {
                let mut len = 0;
                for &b in base {
                    out[n][len] = b;
                    len += 1;
                }
                for (i, slot) in [2u8, 3, 4].iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        out[n][len] = *slot;
                        len += 1;
                    }
                }
                if has_be {
                    out[n][len] = 5;
                    len += 1;
                }
                lens[n] = len;
                n += 1;
            }
            n
        }
        ALU_BITWISE => {
            let opcnt = hdr.raw("opcnt").unwrap_or(0);
            let mut len = 0;
            for slot in 0..3u8 {
                if opcnt & (1 << slot) != 0 {
                    out[0][len] = slot;
                    len += 1;
                }
            }
            lens[0] = len;
            1
        }
        _ => {
            out[0][0] = 0;
            lens[0] = 1;
            1
        }
    }
}

/// Decode the instructions of one candidate phase set (they are written in
/// reverse slot order), then its section tail.
#[allow(clippy::too_many_arguments)]
fn walk(
    grp: &[u8],
    total: usize,
    hdr_len: usize,
    alutype: u32,
    da: u32,
    w0p: bool,
    w1p: bool,
    slots: &[u8],
    remaining: usize,
    at: usize,
    instrs: &mut [Option<(u8, Region)>; MAX_INSTRS],
    hdr: &Region,
    found: &mut Option<Group>,
    count: &mut usize,
) {
    if remaining == 0 {
        tail(grp, total, hdr_len, alutype, da, w0p, w1p, at, instrs, hdr, found, count);
        return;
    }
    let slot = slots[remaining - 1];
    let (set_id, phases) = slot_phases(alutype, slot);
    for &phase in phases {
        let allowed: &[u16] = if alutype == ALU_CONTROL {
            let ctrlop = hdr.raw("ctrlop").unwrap_or(0xffff_ffff);
            match CTRLOP_VARIANTS.iter().find(|(c, _)| *c == ctrlop) {
                Some((_, v)) => v,
                None => continue,
            }
        } else {
            match phase_variants(phase, set_id) {
                Some(v) => v,
                None => continue,
            }
        };
        let set = bit_set(set_id);
        for &vi in allowed {
            let v = &set.variants[vi as usize];
            if !variant_matches(grp, at, total - at, set, v) {
                continue;
            }
            let r = decode_as(grp, at, set_id, vi);
            // An instruction form whose operation-selecting fields name no
            // operation in this phase is not something the compiler emits:
            // the form `sngl` is FRCP, FRSQ, FLOG, FEXP, MBYP and the
            // derivatives at particular `sngl_op` values and nothing at the
            // rest. Pruning those is what makes the parse unique - without
            // it 208 of the 2,042 blobs have a group that reads two ways.
            if ops_for(&r, phase).next().is_none() {
                continue;
            }
            instrs[slot as usize] = Some((phase, r));
            walk(
                grp,
                total,
                hdr_len,
                alutype,
                da,
                w0p,
                w1p,
                slots,
                remaining - 1,
                at + v.bytes as usize,
                instrs,
                hdr,
                found,
                count,
            );
            instrs[slot as usize] = None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tail(
    grp: &[u8],
    total: usize,
    hdr_len: usize,
    alutype: u32,
    da: u32,
    w0p: bool,
    w1p: bool,
    instr_end: usize,
    instrs: &[Option<(u8, Region)>; MAX_INSTRS],
    hdr: &Region,
    found: &mut Option<Group>,
    count: &mut usize,
) {
    // The header's decode-assist field is a checksum on the split we chose:
    // for MAIN it counts only the phases above p1, for BITWISE every phase,
    // and for CONTROL it is the whole instruction area, or 0 when the group
    // has no sources and no destinations.
    let instr_bytes = |from: u8| -> usize {
        instrs
            .iter()
            .enumerate()
            .filter(|(slot, _)| *slot >= from as usize)
            .filter_map(|(_, o)| o.as_ref())
            .map(|(_, r)| r.bytes())
            .sum()
    };
    let da_want = match alutype {
        ALU_MAIN => Some(hdr_len + instr_bytes(2)),
        ALU_BITWISE => Some(hdr_len + instr_bytes(0)),
        _ => None,
    };
    if let Some(w) = da_want {
        if da as usize != w {
            return;
        }
    }

    let ndst = usize::from(w0p) + usize::from(w1p);
    for &(lo, up, iss) in sections_present(alutype, da) {
        let mut p = instr_end;
        let mut lower_r = None;
        let mut upper_r = None;
        let mut iss_r = None;
        let mut dst_r = None;
        let mut ok = true;
        for (want, id, upper) in
            [(lo, BitSetId::Src, Some(false)), (up, BitSetId::Src, Some(true)), (iss, BitSetId::Iss, None)]
        {
            if !want {
                continue;
            }
            let Some(vi) = match_one(grp, p, total - p, id, None, upper) else {
                ok = false;
                break;
            };
            let r = decode_as(grp, p, id, vi);
            p += r.bytes();
            match (id, upper) {
                (BitSetId::Src, Some(false)) => lower_r = Some(r),
                (BitSetId::Src, _) => upper_r = Some(r),
                _ => iss_r = Some(r),
            }
        }
        if !ok {
            continue;
        }
        if ndst > 0 {
            let set = bit_set(BitSetId::Dst);
            // One write port uses the single-destination forms, two the
            // double ones; the header's w0p/w1p is what says which.
            let want_dual = ndst == 2;
            let mut vi = None;
            let mut nmatch = 0;
            for (i, v) in set.variants.iter().enumerate() {
                if v.name.starts_with("2_") != want_dual {
                    continue;
                }
                if variant_matches(grp, p, total - p, set, v) {
                    vi = Some(i as u16);
                    nmatch += 1;
                }
            }
            if nmatch != 1 {
                continue;
            }
            let r = decode_as(grp, p, BitSetId::Dst, vi.unwrap());
            p += r.bytes();
            dst_r = Some(r);
        }
        if da_want.is_none() {
            // CONTROL: the decode-assist field is the whole instruction area
            // unless the group has nothing after it.
            let no_sd = !lo && !up && ndst == 0;
            let want = if no_sd { 0 } else { hdr_len + instr_bytes(0) };
            if da as usize != want {
                continue;
            }
        }

        // Padding: an optional 0xff that makes the group an even number of
        // bytes, then an optional i-cache alignment block that closes the
        // shader (0xf0 | words, then 0xff filler).
        let mut q = p;
        let mut word_pad = 0u8;
        let mut align_pad = 0u8;
        let mut rem = total - q;
        if rem > 0 {
            if grp[q] == 0xff && (rem == 1 || (grp[q + 1] & 0xf0) == 0xf0) {
                word_pad = 1;
                q += 1;
                rem -= 1;
            }
            if rem > 0 {
                if (grp[q] & 0xf0) != 0xf0 {
                    continue;
                }
                align_pad = (grp[q] & 0xf) * 2;
                if align_pad as usize != rem || grp[q + 1..total].iter().any(|&b| b != 0xff) {
                    continue;
                }
            }
        }

        *count += 1;
        if *count == 1 {
            *found = Some(Group {
                hdr: *hdr,
                alutype,
                instrs: *instrs,
                lower: lower_r,
                upper: upper_r,
                iss: iss_r,
                dst: dst_r,
                word_pad,
                align_pad,
                total,
            });
        }
    }
}

/// Walk a whole shader, calling `f` with each group in turn.
pub fn for_each_group(
    buf: &[u8],
    mut f: impl FnMut(usize, &Group) -> Result<(), DecodeError>,
) -> Result<usize, DecodeError> {
    let mut off = 0;
    let mut n = 0;
    while off < buf.len() {
        let g = decode_group(buf, off)?;
        f(off, &g)?;
        off += g.total;
        n += 1;
    }
    Ok(n)
}
