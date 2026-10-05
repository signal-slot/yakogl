// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Ported from Mesa 26.2.1 src/imagination/pco/pco_group_instrs.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! M1's gate: the whole offline blob table through the decoder and back.
//!
//! These are host tests (`host-test,shaderc-online`) over the PowerVR table,
//! so they are compiled only when that table is the one in the build.

#![cfg(all(test, not(feature = "gpu-vivante")))]

use super::decode::{decode_group, DecodeError, ALU_BITWISE, ALU_MAIN};
use super::encode::encode_group;
use super::isa::{PH_0, PH_1, PH_2, PH_2_MOV, PH_2_PCK, PH_2_TST, PH_BACKEND};
use crate::shaderc::mesa_reference::SHC_BLOB_TABLE;
use std::collections::BTreeMap;

/// The USC of one table row, as bytes. The table stores dwords; the
/// instruction stream is a byte stream, little-endian within each dword.
fn usc_bytes(i: usize) -> Vec<u8> {
    let b = &SHC_BLOB_TABLE[i];
    // SAFETY: `usc` points at a `[u32; usc_dw]` static of the generated
    // table (blobs.rs), which outlives the test.
    let dw = unsafe { core::slice::from_raw_parts(b.shader.usc, b.shader.usc_dw as usize) };
    dw.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// M1's acceptance: decode every blob into instruction groups and write
/// them back byte for byte.
///
/// This is the test of the ISA tables, not of the table of blobs: the
/// only way a group re-encodes exactly is that every bit of it was
/// accounted for by some field of the variant the decoder chose, so a
/// wrong variant, a missed field or a misplaced bit piece all show up
/// here as a byte that differs.
#[test]
fn round_trip_every_blob_in_the_offline_table() {
    let mut blobs_ok = 0usize;
    let mut groups = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut out = vec![0u8; 64 * 1024];

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        assert!(bytes.len() <= out.len(), "blob {i} ({}) is larger than the scratch buffer", blob.source_name());
        let mut off = 0;
        let mut ok = true;
        while off < bytes.len() {
            let g = match decode_group(&bytes, off) {
                Ok(g) => g,
                Err(e) => {
                    failures.push(format!(
                        "{} ({}): decode at +{:#x}: {:?}",
                        SHC_BLOB_TABLE[i].source_name(),
                        i,
                        off,
                        e
                    ));
                    ok = false;
                    break;
                }
            };
            match encode_group(&g, &mut out, off) {
                Ok(n) => {
                    if out[off..off + n] != bytes[off..off + n] {
                        failures.push(format!(
                            "{} ({}): re-encode at +{:#x}: {:02x?} != {:02x?}",
                            SHC_BLOB_TABLE[i].source_name(),
                            i,
                            off,
                            &out[off..off + n],
                            &bytes[off..off + n]
                        ));
                        ok = false;
                        break;
                    }
                }
                Err(e) => {
                    failures.push(format!("{} ({}): encode: {:?}", SHC_BLOB_TABLE[i].source_name(), i, e));
                    ok = false;
                    break;
                }
            }
            groups += 1;
            off += g.total;
        }
        if ok {
            blobs_ok += 1;
        }
    }

    if !failures.is_empty() {
        let shown: Vec<&String> = failures.iter().take(10).collect();
        panic!(
            "{} of {} blobs round-tripped ({} groups); {} failures, first {}:\n{}",
            blobs_ok,
            SHC_BLOB_TABLE.len(),
            groups,
            failures.len(),
            shown.len(),
            shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
        );
    }
    assert_eq!(blobs_ok, SHC_BLOB_TABLE.len());
    // The group count is a second, independent statement about the same
    // run: a decoder that quietly stopped after the first group of every
    // blob would still report every blob as round-tripped.
    assert!(groups > 100_000, "only {groups} groups decoded; the walk stopped early");
    eprintln!("round trip: {blobs_ok}/{} blobs, {groups} instruction groups", SHC_BLOB_TABLE.len());
}

/// A decoder that guesses is worse than one that fails, so the decoder
/// refuses an ambiguous group. Show that none of the corpus is ambiguous,
/// which is what makes the round trip above mean what it says.
/// The numbers a BUILDER has to compute, checked against 576,296 groups that
/// somebody else built.
///
/// Decoding a group READS these fields; constructing one has to PRODUCE them,
/// and that is the part of the USC back end with no reference to copy
/// (`docs/online-shaderc/usc-seam.md`, "the construction side"). The rules
/// come from Mesa's `pco_group_instrs.c` -- `calc_lengths()`, `calc_da()` and
/// `calc_align_padding()` -- read and transcribed. This test turns them from
/// *read* into *measured*: it recomputes each one from the group's decoded
/// parts and compares it with what the blob actually holds.
///
///   body       = header + instructions + sources + iss + destinations
///   word pad   = one byte iff the body is odd
///   hdr.length = (body + word pad + align pad) / 2, in 2-byte words
///   hdr.da     = a prefix length telling the hardware where the source and
///                destination area begins, computed per ALU type
///
/// The length rule cost a wrong first version of this test, so it is spelled
/// out. `calc_lengths()` sets `hdr.length` from a total with no alignment
/// padding in it, which makes the field look as though it excludes that
/// padding -- but `calc_align_padding()` runs afterwards, adds the padding to
/// the total and *recomputes* `hdr.length = total / 2`. The header counts the
/// alignment block. 101,057 groups said so before pco was re-read, and the
/// data was right.
///
/// The entry worth the trouble is `da` for the CONTROL type. The decoder
/// recomputes `da` for MAIN and BITWISE and rejects a parse that disagrees,
/// so the existing round trip already proved those two. It never recomputes
/// CONTROL's -- it only consults it through `sections_present()` -- so the
/// rule for the corpus's 36,899 control groups rested on pco's source alone
/// until now. It holds for all 36,899, with 30,260 of them taking the
/// `no_srcs_dests` branch that makes `da` zero.
///
/// # What this does NOT assert
///
/// That a group carrying alignment padding lands on an 8-byte boundary. Only
/// 455 of the 1,518 padded groups do, and that is not a contradiction of pco:
/// `calc_align_padding()` pads the LAST group so its own total reaches a
/// boundary, and the PENULTIMATE one so that the last group's OFFSET does, so
/// the penultimate's own total need not be aligned at all. Stating the rule
/// properly means tracking a group's offset within its shader, which this
/// test does not do. The count is printed so the number is on record; the
/// builder's alignment step will have to write the rule out properly.
///
/// # A trap that produced a confident wrong reading
///
/// This test first reported 99,539 disagreements and "every group is MAIN".
/// Both were one line: `ALU_MAIN` and `ALU_BITWISE` were not imported, so in
/// `match g.alutype { ALU_MAIN => ... }` they were not constants at all --
/// they were fresh bindings, and the first arm matched everything. Every
/// group was then scored with the MAIN rule, and the 99,539 that "failed"
/// were exactly the 62,640 bitwise plus the 36,899 control groups.
///
/// Rust warns about neither, because both are legal. The defence is the three
/// `assert_eq!`s on the per-type counts: they compare against
/// `isa-coverage.md`, which is derived independently, so a test that has
/// stopped telling the ALU types apart fails instead of passing vacuously.
#[test]
fn the_group_length_and_da_rules_hold_for_every_group() {
    /// `ROGUE_ICACHE_ALIGN` (rogue_hw_defs.h): a shader is padded up to this.
    const ICACHE_ALIGN: usize = 8;

    let mut groups = 0usize;
    let (mut main_groups, mut bitwise_groups, mut control_groups) = (0usize, 0usize, 0usize);
    let (mut control_da_zero, mut aligned_groups, mut aligned_on_boundary) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let name = blob.source_name();
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };

            let hdr = g.hdr.bytes();
            let instr_from = |from: u8| -> usize {
                g.instrs
                    .iter()
                    .enumerate()
                    .filter(|(slot, _)| *slot >= from as usize)
                    .filter_map(|(_, o)| o.as_ref())
                    .map(|(_, r)| r.bytes())
                    .sum()
            };
            let sections: usize = [&g.lower, &g.upper, &g.iss, &g.dst].into_iter().flatten().map(|r| r.bytes()).sum();
            let body = hdr + instr_from(0) + sections;

            // 1. the word pad is there exactly when the body is odd
            let want_pad = u8::from(!body.is_multiple_of(2));
            if g.word_pad != want_pad {
                failures.push(format!("{name} +{off:#x}: word pad {} but the body is {body}", g.word_pad));
            }
            // 2. the total is the sum of the parts; the alignment block closes
            //    a shader, so it sits on top of a group already complete
            let want_total = body + want_pad as usize + g.align_pad as usize;
            if g.total != want_total {
                failures.push(format!("{name} +{off:#x}: total {} but the parts sum to {want_total}", g.total));
            }
            // 3. the header states that in 2-byte words, alignment included
            let length = g.hdr.get("length").unwrap_or(0) as usize;
            if length * 2 != want_total {
                failures.push(format!("{name} +{off:#x}: hdr.length {length} words != {want_total} bytes"));
            }
            // 4. decode assist, per ALU type
            let da = g.hdr.get("da").unwrap_or(0) as usize;
            let want_da = match g.alutype {
                ALU_MAIN => {
                    main_groups += 1;
                    hdr + instr_from(2)
                }
                ALU_BITWISE => {
                    bitwise_groups += 1;
                    hdr + instr_from(0)
                }
                _ => {
                    control_groups += 1;
                    // pco's `no_srcs_dests`; the ISS section is deliberately
                    // not one of the three it looks at.
                    if g.lower.is_none() && g.upper.is_none() && g.dst.is_none() {
                        control_da_zero += 1;
                        0
                    } else {
                        hdr + instr_from(0)
                    }
                }
            };
            if da != want_da {
                failures.push(format!("{name} +{off:#x}: alutype {} da {da}, rule says {want_da}", g.alutype));
            }
            if g.align_pad != 0 {
                aligned_groups += 1;
                aligned_on_boundary += usize::from(want_total.is_multiple_of(ICACHE_ALIGN));
            }

            groups += 1;
            off += g.total;
        }
    }

    if !failures.is_empty() {
        // Bucket by rule first: printing "ten examples" once hid which rule
        // was failing and cost a wrong reading of the corpus.
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for f in &failures {
            let kind = ["word pad", "total", "hdr.length", "da"].into_iter().find(|k| f.contains(k));
            *kinds.entry(kind.unwrap_or("other")).or_default() += 1;
        }
        panic!(
            "{} of {groups} groups disagree; by rule {kinds:?}\nfirst:\n{}",
            failures.len(),
            failures[..failures.len().min(5)].join("\n")
        );
    }
    // The per-type counts must match `isa-coverage.md`'s "Group headers"
    // table, which is derived independently. Without this a run where every
    // group looked like one type would pass the da check trivially -- which
    // is exactly what happened once.
    // The 20 matattr rows (es100/matattr*.vert, matattr.frag and its 16 blend
    // variants) added 297 main and 102 control groups to 539_747 / 42_625.
    // The 48 offarm rows (gl_test 193: offarm/fill_color.nodiscard.offarm{8,32,128}
    // .frag, plain and 15 blend variants each) add 6_681 main, 45 bitwise
    // and 540 control groups to 540_044 / 68_997 / 42_727.
    // The 160 rows of gl_test 194's finer series (offarm2, 4, 12, 16, 24, 48,
    // 64, 96, 192, 256, 16 rows each) add 24_580 main, 150 bitwise and 1_800
    // control groups to 546_725 / 69_042 / 43_267.
    assert_eq!(main_groups, 571_305, "main groups");
    assert_eq!(bitwise_groups, 69_192, "bitwise groups");
    assert_eq!(control_groups, 45_067, "control groups");
    assert!(control_da_zero > 0, "no control group took the da == 0 branch");
    assert!(aligned_groups > 0, "no group carries alignment padding; that branch is untested");
    std::println!(
        "calc_lengths and calc_da verified on {groups} groups: {main_groups} main, {bitwise_groups} bitwise, \
         {control_groups} control ({control_da_zero} of them with no sources or destinations); align pad on \
         {aligned_groups} groups, {aligned_on_boundary} of them landing on an {ICACHE_ALIGN}-byte boundary"
    );
}

/// The phase rule: a group's header declares which phases it fills, and the
/// instructions in it must occupy exactly those.
///
/// This is the next thing the builder needs after the lengths. Constructing a
/// group means choosing an `oporg` and filling phases, and those two are one
/// decision made in two places -- get them out of step and the hardware reads
/// a phase the group did not encode, or skips one it did.
///
/// The legal sets are not "any subset". `oporg` is three bits with eight
/// values, and the enum in the generated tables is:
///
///     p0   p2   be   p0_p1   p0_p2   p0_p1_p2   p0_p2_be   p0_p1_p2_be
///
/// Two absences are the whole constraint, and a builder that did not know
/// them would emit groups the field cannot describe: **there is no `p1`
/// alone**, and **there is no `p1_p2`**. Phase 1 never appears without phase
/// 0. That is consistent with what the shapes do -- `min`'s phase 1 is an
/// `mbyp` feeding `ft1` alongside phase 0's `ft0`, never on its own -- but it
/// is a hardware rule, not a habit of the shapes, and this test is what makes
/// it a measured one.
///
/// The four phase-2 encodings (`p2`, `p2_pck`, `p2_tst`, `p2_mov`) all count
/// as `p2` here: they are the slots within phase 2, not separate phases, and
/// `oporg` does not distinguish them.
///
/// # Which of the eight the corpus actually uses
///
/// Five, over 476,757 main groups:
///
/// | `oporg` | phases | count |
/// |---|---|---:|
/// | 0 | `p0` | 406,519 |
/// | 5 | `p0_p1_p2` | 47,756 |
/// | 4 | `p0_p2` | 10,711 |
/// | 2 | `be` | 7,020 |
/// | 1 | `p2` | 4,751 |
///
/// **`p0_p1` (3), `p0_p2_be` (6) and `p0_p1_p2_be` (7) never occur.** They are
/// encodable and this corpus does not encode them, which is a fact about what
/// the offline compiler emitted rather than about the hardware -- so a builder
/// may emit the five above on the strength of 476,757 witnesses, and the other
/// three only on the strength of the field's own enum. Worth knowing before
/// writing a scheduler that reaches for one of them.
///
/// The 47,756 at `p0_p1_p2` are the shape [`super::builtins`] needs most:
/// `min`, `max`, `ceil` and `sign` are all two `mbyp`s in phases 0 and 1 plus
/// a `tst` and a `movc` in phase 2. Eighty-five per cent of groups are a
/// single phase-0 operation, which is the other thing a builder should know:
/// the common case is one operation in a group, not a filled pipeline.
#[test]
fn a_groups_phases_are_exactly_the_ones_its_header_declares() {
    const P0: u8 = 1;
    const P1: u8 = 2;
    const P2: u8 = 4;
    const BE: u8 = 8;

    /// `oporg` value -> the phase set it declares.
    fn declared(oporg: u32) -> Option<u8> {
        Some(match oporg {
            0 => P0,
            1 => P2,
            2 => BE,
            3 => P0 | P1,
            4 => P0 | P2,
            5 => P0 | P1 | P2,
            6 => P0 | P2 | BE,
            7 => P0 | P1 | P2 | BE,
            _ => return None,
        })
    }

    let mut checked = 0usize;
    let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let name = blob.source_name();
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            off += g.total;

            // Only the header variants that carry the field; the bitwise and
            // control headers organise their phases differently and are not
            // what this rule is about.
            let Some(oporg) = g.hdr.get("oporg") else { continue };
            let Some(want) = declared(oporg) else {
                failures.push(format!("{name}: oporg {oporg} is outside the enum"));
                continue;
            };
            *seen.entry(oporg).or_default() += 1;

            let mut have = 0u8;
            for (phase, _) in g.instrs.iter().flatten() {
                have |= match *phase {
                    PH_0 => P0,
                    PH_1 => P1,
                    PH_2 | PH_2_PCK | PH_2_TST | PH_2_MOV => P2,
                    PH_BACKEND => BE,
                    _ => 0,
                };
            }
            if have != want {
                failures.push(format!("{name}: oporg {oporg} declares {want:#06b} but the group fills {have:#06b}"));
            }
            checked += 1;
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {checked} groups disagree:\n{}",
        failures.len(),
        failures[..failures.len().min(5)].join("\n")
    );
    assert!(checked > 400_000, "only {checked} groups carry an oporg; the rule barely ran");
    // Phase 1 never alone and never with phase 2 only: the two values that
    // would encode those do not exist, so nothing in the corpus can show
    // them -- what this pins is that the eight that DO exist are the eight
    // the builder may choose from.
    std::println!("phase sets over {checked} groups, by oporg: {seen:?}");
}

#[test]
fn no_group_in_the_corpus_decodes_two_ways() {
    let mut n = 0;
    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            match decode_group(&bytes, off) {
                Ok(g) => {
                    off += g.total;
                    n += 1;
                }
                Err(DecodeError::Ambiguous) => {
                    panic!("{} +{off:#x}: ambiguous group", blob.source_name())
                }
                Err(e) => panic!("{} +{off:#x}: {e:?}", blob.source_name()),
            }
        }
    }
    assert!(n > 100_000);
}

/// What the corpus actually uses, counted: the input to the code
/// generator's target list. Writes `docs/online-shaderc/isa-coverage.md`
/// when `YAKOGL_WRITE_ISA_COVERAGE=1`, so the committed document and this
/// decoder cannot drift apart.
#[test]
fn isa_coverage() {
    let mut ops: BTreeMap<String, usize> = BTreeMap::new();
    let mut variants: BTreeMap<String, usize> = BTreeMap::new();
    let mut hdrs: BTreeMap<String, usize> = BTreeMap::new();
    let mut secs: BTreeMap<String, usize> = BTreeMap::new();
    let mut groups = 0usize;
    let mut unnamed = 0usize;

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let g = decode_group(&bytes, off).unwrap_or_else(|e| panic!("{} +{off:#x}: {e:?}", blob.source_name()));
            groups += 1;
            *hdrs.entry(g.hdr.variant().name.to_string()).or_default() += 1;
            for (_, phase, r) in g.instrs() {
                let set = crate::shaderc::online::powervr::isa::bit_set(r.set).name;
                *variants.entry(format!("{}.{}", set, r.variant().name)).or_default() += 1;
                let mut named = false;
                for op in crate::shaderc::online::powervr::isa::ops_for(r, phase) {
                    *ops.entry(op.name.to_string()).or_default() += 1;
                    named = true;
                }
                if !named {
                    unnamed += 1;
                }
            }
            for (label, r) in [("lower_src", &g.lower), ("upper_src", &g.upper), ("iss", &g.iss), ("dst", &g.dst)] {
                if let Some(r) = r {
                    *secs.entry(format!("{}.{}", label, r.variant().name)).or_default() += 1;
                }
            }
            if g.word_pad != 0 {
                *secs.entry("pad.word".to_string()).or_default() += 1;
            }
            if g.align_pad != 0 {
                *secs.entry("pad.align".to_string()).or_default() += 1;
            }
            off += g.total;
        }
    }

    assert_eq!(unnamed, 0, "{unnamed} decoded instructions matched no operation");
    if std::env::var("YAKOGL_WRITE_ISA_COVERAGE").as_deref() == Ok("1") {
        let mut s = String::new();
        s.push_str(&coverage_doc(groups, &hdrs, &variants, &ops, &secs));
        std::fs::write("docs/online-shaderc/isa-coverage.md", s).expect("write coverage");
    }
    eprintln!("coverage: {groups} groups, {} operations, {} instruction forms", ops.len(), variants.len());
}

fn coverage_doc(
    groups: usize,
    hdrs: &BTreeMap<String, usize>,
    variants: &BTreeMap<String, usize>,
    ops: &BTreeMap<String, usize>,
    secs: &BTreeMap<String, usize>,
) -> String {
    let mut s = String::new();
    s.push_str("# What the offline table's USC actually uses\n\n");
    s.push_str(
        "Counted by `cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
         isa_coverage`, which regenerates this file with `YAKOGL_WRITE_ISA_COVERAGE=1`.\n\
         It is the code generator's target list: an instruction form or an operation that \
         is not here is one nothing in the table needs, and one that is here with a large \
         count is one the generator has to get right first.\n\n",
    );
    s.push_str(&format!(
        "Corpus: {} blobs of `SHC_BLOB_TABLE`, {} instruction groups.\n\n",
        SHC_BLOB_TABLE.len(),
        groups
    ));
    let table = |title: &str, m: &BTreeMap<String, usize>| {
        let mut rows: Vec<(&String, &usize)> = m.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut t = format!("## {title}\n\n| | count |\n|---|---:|\n");
        for (k, v) in rows {
            t.push_str(&format!("| `{k}` | {v} |\n"));
        }
        t.push('\n');
        t
    };
    s.push_str(&table("Operations", ops));
    s.push_str(&table("Instruction forms", variants));
    s.push_str(&table("Group headers", hdrs));
    s.push_str(&table("Sections and padding", secs));
    s
}

/// Which source slots can name the shared (uniform) register file, and which
/// can never.
///
/// This is the rule the builder's legalisation pass turns on, and it cost me
/// a wrong explanation to two teammates before I measured it rather than
/// reading the generated table by eye.
///
/// A source operand's register bank is a per-slot field -- `sb0`..`sb5` --
/// but **its width varies by which source-section variant the group uses**,
/// and the width decides which banks it can name:
///
/// | width | enum | banks it can express |
/// |---|---|---|
/// | 1 bit | `regbank1` | `special`, `temp` |
/// | 2 bits | `regbank2` | `special`, `temp`, `vtxin`, `coeff` |
/// | 3 bits | `regbank` | all eight, **including `shared`** |
///
/// So a uniform read is free only where the slot's bank field reaches three
/// bits, and the builder chooses the variant. The consequence is not the one
/// I first described: for a slot that CAN be widened, a shared operand costs
/// **bytes, not an instruction** -- one lower source goes from `1lo_1b6i` at
/// one byte to `1lo_3b11i_2m` at three. The `mbyp` is needed only for the
/// slots that never reach three bits in any variant.
///
/// The asymmetry this test pins is that those slots exist and are always the
/// same ones.
#[test]
fn only_some_source_slots_can_ever_name_the_shared_file() {
    use super::isa::{bit_set, BitSetId};

    let set = bit_set(BitSetId::Src);
    // The widest bank field each slot is given by any variant.
    let mut widest = [0u8; 6];
    for v in set.variants {
        for u in v.uses {
            let f = &set.fields[u.field];
            // The bank fields are named sbN and carry one of the regbank
            // enums; nothing else in this bit set does both.
            // The SLOT is the use's role name, not the field's own name: one
            // physical field is reused in several roles across variants, so
            // `Field::name` is the bit layout and `FieldUse::name` is what it
            // means here. Reading the field's name instead gives every slot a
            // width of zero, silently.
            let Some(n) = u.name.strip_prefix("sb").and_then(|r| r.parse::<usize>().ok()) else {
                continue;
            };
            let Some(e) = f.enum_id else { continue };
            let en = super::isa_tables::ENUMS[e].name;
            assert!(en.starts_with("regbank"), "use {} is sbN but its enum is {en}, not a register bank", u.name);
            assert!(n < 6, "source slot {n} is outside s0..s5");
            widest[n] = widest[n].max(f.bits);
        }
    }

    // Measured, not assumed: s0, s2 and s3 reach three bits; s1, s4 and s5
    // never do. It is NOT a tidy symmetry -- the lower trio (s0, s1, s2) can
    // widen its first and last, and the upper trio (s3, s4, s5) only its
    // first -- so there is no rule of thumb to remember here, only the
    // numbers. What it does mean concretely: a two-operand group with a
    // uniform as its FIRST operand costs bytes, and the same group with the
    // uniform SECOND costs an `mbyp`.
    assert_eq!(widest, [3, 2, 3, 3, 2, 2], "the per-slot widest bank field changed");

    let can_share: Vec<usize> = (0..6).filter(|&i| widest[i] >= 3).collect();
    assert_eq!(can_share, vec![0, 2, 3], "which slots can name `shared`");
    std::println!(
        "source slot bank widths: {widest:?} bits; slots {can_share:?} can name the shared file, \
         the others need an mbyp"
    );
}

/// Every destination in the corpus is a FULL definition, and the allocator
/// depends on it.
///
/// `powervr/alloc.rs` computes liveness over `SelGroup`s, and its correctness
/// rests on one assumption it states in its own file: a group's destination is
/// completely defined by that group. If a destination could be *partially*
/// written, a virtual register's live range would not start there, and the
/// allocator would be free to place something else on a register still holding
/// live data -- a wrong picture, not an error.
///
/// The hardware can express a partial write. `main.movc_ext` carries
/// `maskw0`, a four-bit per-component enable whose enum is `e0 e1 e2 e3 eall`
/// (1, 2, 4, 8, 15). So the assumption is not a hardware guarantee and cannot
/// be one.
///
/// What it is instead: **measured**. Across all 52,723 `movc`-form
/// instructions in the offline corpus -- which includes the 1,312 the
/// operation table calls `movwm`, "move with write mask", since `movwm` is
/// `movc_ext` with `aw` pinned -- `maskw0` is `eall` every single time. Not
/// one partial write in the whole table.
///
/// So the allocator's assumption holds for everything we have seen, and the
/// contract between the two files is: **this builder never emits a partial
/// mask without saying so.** If it ever needs to, this test fails first, and
/// that failure is the signal to teach the allocator about partial
/// definitions rather than to relax the test.
///
/// Not to be confused with `movw0`/`movw1`, which vary (0, 1, 2, 3) and are
/// NOT masks: their enum is `ft0 ft1 ft2 fte`, so they select which internal
/// register feeds a write port. I checked that before reporting, because a
/// varied histogram next to a mask field invites exactly the wrong reading.
#[test]
fn every_destination_in_the_corpus_is_fully_written() {
    const EALL: u32 = 15;
    let mut movc = 0usize;
    let mut partial: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            off += g.total;
            for (_, r) in g.instrs.iter().flatten() {
                if !r.variant().name.starts_with("movc") {
                    continue;
                }
                movc += 1;
                if let Some(m) = r.get("maskw0") {
                    if m != EALL {
                        partial.push(std::format!("{} +{off:#x}: maskw0 {m:#06b}", blob.source_name()));
                    }
                }
            }
        }
    }

    assert!(
        partial.is_empty(),
        "{} of {movc} movc instructions write only part of their destination; \
         powervr/alloc.rs assumes full definitions and must be told:\n{}",
        partial.len(),
        partial[..partial.len().min(5)].join("\n")
    );
    // 49_718 before the 48 offarm rows (gl_test 193), which add 405, and the
    // 160 of gl_test 194, which add 1_350.
    assert_eq!(movc, 51_473, "the movc population changed; re-read the assumption above");
}

/// Three questions the corpus can answer, now that the operands can be read.
///
/// Each was asked by a teammate and each was blocked on exactly this: reading
/// a register number out of a source section needs the variant-dependent bank
/// and index fields. They share one walk because they share one decode.
#[test]
fn what_the_corpus_says_about_the_register_files() {
    use super::operands::{dsts, srcs};

    /* The `regbank` values, from isa_tables' own enum. */
    const TEMP: u32 = 1;
    let mut max_index = [0u32; 8]; // per bank
    let mut seen_bank = [0usize; 8];
    let mut self_reading_groups = 0usize;
    let mut blobs_reading_an_unwritten_temp: Vec<(String, u32)> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        /* Temporaries this blob has written by the time each group runs. */
        let mut written = [false; 256];
        let mut first_unwritten_read: Option<u32> = None;

        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            let src_ops: Vec<_> = [&g.lower, &g.upper]
                .into_iter()
                .flatten()
                .flat_map(|r| srcs(r).into_iter().flatten().collect::<Vec<_>>())
                .collect();
            let dst_ops: Vec<_> = g.dst.iter().flat_map(|r| dsts(r).into_iter().flatten()).collect();

            for op in &src_ops {
                seen_bank[(op.bank & 7) as usize] += 1;
                let b = (op.bank & 7) as usize;
                max_index[b] = max_index[b].max(op.index);
                if op.bank == TEMP && (op.index as usize) < written.len() && !written[op.index as usize] {
                    first_unwritten_read.get_or_insert(op.index);
                }
            }
            /* A group that reads the register it writes, in the same group. */
            if dst_ops.iter().any(|d| src_ops.iter().any(|s| s.bank == d.bank && s.index == d.index)) {
                self_reading_groups += 1;
            }
            for op in &dst_ops {
                let b = (op.bank & 7) as usize;
                seen_bank[b] += 1;
                max_index[b] = max_index[b].max(op.index);
                if op.bank == TEMP && (op.index as usize) < written.len() {
                    written[op.index as usize] = true;
                }
            }
            off += g.total;
        }
        if let Some(r) = first_unwritten_read {
            blobs_reading_an_unwritten_temp.push((blob.source_name().to_string(), r));
        }
    }

    std::println!("\n=== what the corpus says about the register files ===");
    std::println!("bank   operands seen   highest index");
    for b in 0..8 {
        if seen_bank[b] > 0 {
            std::println!("  {b}      {:>10}      {:>8}", seen_bank[b], max_index[b]);
        }
    }
    std::println!("\ngroups reading a register they also write: {self_reading_groups}");
    /* NOT an answer to "which temporary does the hardware pre-load".
     * This walk only sees writes that are ALU DESTINATION OPERANDS. A backend
     * operation -- an iterator, a texture fetch, a uvsw write -- puts a value
     * in a temporary without one, so a temporary written that way looks
     * unwritten here. The spread of "first unwritten read" across two dozen
     * indices below is the signature of that gap, not of two dozen hardware
     * pins, and the r0 count must NOT be read as evidence for r0.
     *
     * Two candidate explanations have been tried and refuted, so the next
     * person starts further along:
     *
     * - THE REPEAT COUNT. A group repeats `rpt` times (2 bits, SubOne, so
     *   1..4) over consecutive registers, so one destination operand can mean
     *   four registers written. Accounting for it moved 1,736 blobs to 1,758
     *   -- slightly WORSE, because the reads widen too. Not the gap.
     * - PHANTOM SOURCE SLOTS, the worry that a section encodes more slots
     *   than the instructions read, which would also have inflated the index
     *   maxima above. It does not: the main instruction names its operands as
     *   `s0`/`s1`/`s2` directly (its modifier fields are `s0neg`, `s1abs`,
     *   `s2flr`), so a slot the section carries is a slot some instruction in
     *   the group reads. The bank and index figures stand.
     *
     * What has NOT been tried: this walk assumes program order is execution
     * order, and a shader has PHASES (`phas`, `oporg`) whose order the
     * hardware chooses. That is where to look next. */
    std::println!(
        "\nNOT AN ANSWER (writes through backend ops are not tracked):\n  \
         blobs whose first temp read is of a temp this walk never saw written: {} of {}",
        blobs_reading_an_unwritten_temp.len(),
        SHC_BLOB_TABLE.len()
    );
    let mut by_reg: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for (_, r) in &blobs_reading_an_unwritten_temp {
        *by_reg.entry(*r).or_default() += 1;
    }
    for (reg, n) in &by_reg {
        std::println!("   first such temporary r{reg}: {n} blobs");
    }

    /* The walk must have seen the corpus, not an empty table. */
    assert!(seen_bank.iter().sum::<usize>() > 100_000, "the operand walk found almost nothing");

    /* ---- what this measurement DOES establish ---- */

    /* A group reading a register it also writes is ordinary, not exotic: an
     * `fmad` accumulating into its destination is the common case. Pinned
     * because gles2-core assumed the opposite and wrote a test against it. */
    assert!(
        self_reading_groups > 100_000,
        "only {self_reading_groups} self-reading groups; the accumulate idiom should be everywhere"
    );

    /* Demonstrated LOWER BOUNDS on the files, not limits. The highest index a
     * shipped shader names is a floor under the file's size and says nothing
     * about its ceiling. */
    assert!(max_index[1] >= 123, "temp floor moved: {}", max_index[1]);
    assert!(max_index[4] >= 254, "shared floor moved: {}", max_index[4]);
    assert!(max_index[2] >= 33, "vtxin floor moved: {}", max_index[2]);

    /* Indexed addressing is REAL in shipped shaders: `idx0` carries tens of
     * thousands of operands. `sel::Bank` cannot name it, which is a seam gap
     * rather than an oversight -- nothing our kernel produces needs it yet --
     * but "nothing uses it" would have been wrong. */
    assert!(seen_bank[6] > 1000, "idx0 was expected to be used by the corpus");

    /* The coefficient file is almost never an ALU source (hundreds of
     * operands against a million temps), which matches the ABI: a varying is
     * ITERATED into a register first and read from there. */
    assert!(seen_bank[3] < seen_bank[1] / 1000, "coeff is being read directly far more than expected");
}

/// The source-section chooser, against every section the corpus contains.
///
/// For each source section pco emitted, take the operands out of it and ask
/// [`legalise::choose_src_variant`] which section it would have picked. The
/// real one demonstrably holds those operands, so it is always a candidate,
/// and the chooser takes the narrowest candidate -- therefore it must never
/// pick a WIDER one. If it does, the capability model is wrong, and that is
/// the failure this test exists to catch before any group is built.
///
/// Picking a narrower one is counted, not asserted, and NOT claimed as a win.
/// pco chose a wider section than the operands need in 24,587 of 1,092,072,
/// and there are two explanations this test cannot tell apart: bytes left on
/// the table, or a constraint on section width that comes from the rest of the
/// group rather than from the operands -- the header's length, or where the
/// ISS and destination sections are expected to begin. Settling it means
/// building the whole group with the narrower section and checking it decodes
/// to the same instructions, which is the builder's job and not this test's.
#[test]
fn the_source_variant_chooser_never_picks_a_wider_section_than_pco_did() {
    use super::legalise::choose_src_variant;
    use super::operands::srcs;

    let set = super::isa::bit_set(super::isa::BitSetId::Src);
    let (mut same, mut narrower, mut sections) = (0usize, 0usize, 0usize);
    let mut refused: Vec<String> = Vec::new();
    let mut wider: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            for r in [&g.lower, &g.upper].into_iter().flatten() {
                sections += 1;
                let ops = srcs(r);
                let actual = set.variants[r.variant as usize].bytes;
                match choose_src_variant(&ops, 0) {
                    Ok(v) => {
                        let chosen = set.variants[v as usize].bytes;
                        if chosen > actual {
                            wider.push(format!(
                                "{}: pco used {} ({actual}B), chooser picked {} ({chosen}B)",
                                blob.source_name(),
                                set.variants[r.variant as usize].name,
                                set.variants[v as usize].name
                            ));
                        } else if chosen < actual {
                            narrower += 1;
                        } else {
                            same += 1;
                        }
                    }
                    Err(e) => refused.push(format!("{}: {e:?}", blob.source_name())),
                }
            }
            off += g.total;
        }
    }

    std::println!("\n=== the source-section chooser against pco ===");
    std::println!("sections   {sections}");
    std::println!("  same width as pco    {same}");
    std::println!("  narrower than pco    {narrower}");
    std::println!("  WIDER than pco       {}", wider.len());
    std::println!("  refused              {}", refused.len());
    for m in wider.iter().take(5) {
        std::println!("   wider: {m}");
    }
    for m in refused.iter().take(5) {
        std::println!("   refused: {m}");
    }

    assert!(sections > 10_000, "only {sections} sections; the walk found almost nothing");
    assert!(wider.is_empty(), "{} sections chose a wider encoding than pco did", wider.len());
    assert!(refused.is_empty(), "{} sections were refused outright", refused.len());
}

/// The instruction-form chooser, against every instruction the corpus holds.
///
/// For each encoded instruction, recover which operation it is and which
/// source modifiers it carries, then ask [`instr::instr_region`] which form it
/// would pick for that operation with those modifiers. Same argument as the
/// source-section test: pco's own form demonstrably carries them, so it is
/// always a candidate, and the chooser takes the narrowest -- so it can never
/// pick a wider one unless the model of which form carries which modifier is
/// wrong.
///
/// The narrower count is 118,089 of 782,683, and it is NOT a saving either.
/// It is a larger share than the source sections' 24,587 of 1,092,072, and the
/// likely reason is the same one only stronger: an instruction's form is
/// constrained by the GROUP it sits in, not only by its own modifiers -- the
/// phases packed beside it, and the source section they collectively need.
/// Choosing per instruction in isolation, as this does, will pick narrow forms
/// that do not survive being packed. The builder is what can tell the two
/// apart, by assembling a whole group and decoding it back.
#[test]
fn the_instruction_form_chooser_never_picks_a_wider_form_than_pco_did() {
    use super::instr::instr_region;
    use super::isa::{bit_set, ops_for, BitSetId};
    use super::sel::Mods;

    let (mut same, mut narrower, mut instrs, mut unnamed) = (0usize, 0usize, 0usize, 0usize);
    let mut wider: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            for (_slot, phase, r) in g.instrs() {
                instrs += 1;
                let Some(o) = ops_for(r, phase).next() else {
                    unnamed += 1;
                    continue;
                };
                /* The modifiers this instruction actually carries. */
                let m = |n: &str| r.get(n).unwrap_or(0) != 0;
                let mods = [
                    Mods { neg: m("s0neg"), abs: m("s0abs"), flr: m("s0flr") },
                    Mods { neg: m("s1neg"), abs: m("s1abs"), flr: m("s1flr") },
                    Mods { neg: m("s2neg"), abs: m("s2abs"), flr: m("s2flr") },
                ];
                let actual = bit_set(r.set).variants[r.variant as usize].bytes;
                match instr_region(o.name, phase, &mods, &[]) {
                    Ok(built) => {
                        let chosen = bit_set(built.set).variants[built.variant as usize].bytes;
                        if chosen > actual {
                            wider.push(format!("{}: {} phase {phase}", blob.source_name(), o.name));
                        } else if chosen < actual {
                            narrower += 1;
                        } else {
                            same += 1;
                        }
                    }
                    Err(e) => refused.push(format!("{}: {} phase {phase}: {e:?}", blob.source_name(), o.name)),
                }
            }
            off += g.total;
        }
    }

    std::println!("\n=== the instruction-form chooser against pco ===");
    std::println!("instructions     {instrs}");
    std::println!("  same form width  {same}");
    std::println!("  narrower         {narrower}");
    std::println!("  WIDER            {}", wider.len());
    std::println!("  refused          {}", refused.len());
    std::println!("  no op name       {unnamed}  (forms the tables do not name; not this test's subject)");
    for m in wider.iter().take(5) {
        std::println!("   wider: {m}");
    }
    for m in refused.iter().take(8) {
        std::println!("   refused: {m}");
    }

    assert!(instrs > 100_000, "only {instrs} instructions walked");
    assert!(wider.is_empty(), "{} instructions chose a wider form than pco did", wider.len());
    assert!(refused.is_empty(), "{} instructions were refused outright", refused.len());
    let _ = BitSetId::Main;
}

/// The destination-section chooser, against every one the corpus holds.
///
/// Same argument as the other two, and the result is sharper: 529,225
/// sections, and the chooser picks EXACTLY pco's width every time -- narrower
/// 0 as well as wider 0.
///
/// That zero is worth more than the two it completes, because it corroborates
/// the reading of the other two's narrower counts. The destination section has
/// nothing pulling it wider than its operands need: it is chosen last and no
/// other part of the group depends on its width. So here "narrowest that fits"
/// IS pco's choice, exactly, a half-million times. Where the source sections
/// (2.3 %) and the instruction forms (15 %) differ, the difference is
/// therefore much more likely to be the group constraining the choice than pco
/// leaving bytes on the table -- which is the prediction recorded against
/// those two, now with evidence rather than only an argument.
#[test]
fn the_destination_chooser_never_picks_a_wider_section_than_pco_did() {
    use super::legalise::choose_dst_variant;
    use super::operands::dsts;

    let set = super::isa::bit_set(super::isa::BitSetId::Dst);
    let (mut same, mut narrower, mut sections) = (0usize, 0usize, 0usize);
    let (mut wider, mut refused) = (Vec::<String>::new(), Vec::<String>::new());

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            if let Some(r) = &g.dst {
                sections += 1;
                let actual = set.variants[r.variant as usize].bytes;
                match choose_dst_variant(&dsts(r)) {
                    Ok(v) => {
                        let chosen = set.variants[v as usize].bytes;
                        match chosen.cmp(&actual) {
                            core::cmp::Ordering::Greater => wider.push(format!(
                                "{}: pco {} ({actual}B), chooser {} ({chosen}B)",
                                blob.source_name(),
                                set.variants[r.variant as usize].name,
                                set.variants[v as usize].name
                            )),
                            core::cmp::Ordering::Less => narrower += 1,
                            core::cmp::Ordering::Equal => same += 1,
                        }
                    }
                    Err(e) => refused.push(format!("{}: {e:?}", blob.source_name())),
                }
            }
            off += g.total;
        }
    }

    std::println!("\n=== the destination-section chooser against pco ===");
    std::println!(
        "sections   {sections}   same {same}   narrower {narrower}   WIDER {}   refused {}",
        wider.len(),
        refused.len()
    );
    for m in wider.iter().take(5) {
        std::println!("   wider: {m}");
    }
    for m in refused.iter().take(5) {
        std::println!("   refused: {m}");
    }
    assert!(sections > 10_000, "only {sections} destination sections walked");
    assert!(wider.is_empty(), "{} chose a wider destination section than pco", wider.len());
    assert!(refused.is_empty(), "{} destination sections refused", refused.len());
}

/// Closing a group through [`assemble::close`] reproduces it exactly.
///
/// The round trip so far has been decode -> encode: bytes in, the same bytes
/// out, which tests the TABLES. This is decode -> close -> encode, which tests
/// the BUILDER: the header's `length` and `da`, the word pad and the total are
/// thrown away and recomputed from the parts, and the group still comes back
/// byte for byte.
///
/// It is the first test of anything that constructs rather than reads, and it
/// is what says the four derived-field rules are sufficient -- not merely true
/// of the corpus, which the earlier test established, but enough to rebuild
/// every group in it from its parts alone.
#[test]
fn every_group_closes_back_to_its_own_bytes() {
    use super::assemble::{close, Parts};

    let mut out = vec![0u8; 64 * 1024];
    let (mut groups, mut blobs) = (0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let name = blob.source_name();
        let mut off = 0;
        let mut ok = true;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let parts = Parts {
                alutype: g.alutype,
                instrs: g.instrs,
                lower: g.lower,
                upper: g.upper,
                iss: g.iss,
                dst: g.dst,
                align_pad: g.align_pad,
            };
            match close(&parts, g.hdr) {
                Ok(rebuilt) => {
                    if rebuilt.total != g.total || rebuilt.word_pad != g.word_pad {
                        failures.push(format!(
                            "{name} +{off:#x}: total {} vs {}, pad {} vs {}",
                            rebuilt.total, g.total, rebuilt.word_pad, g.word_pad
                        ));
                        ok = false;
                    } else if encode_group(&rebuilt, &mut out, 0).is_ok() && out[..g.total] != bytes[off..off + g.total]
                    {
                        failures.push(format!("{name} +{off:#x}: re-closed group differs in bytes"));
                        ok = false;
                    }
                }
                Err(e) => {
                    failures.push(format!("{name} +{off:#x}: {e:?}"));
                    ok = false;
                }
            }
            if failures.len() > 20 {
                break;
            }
            off += g.total;
        }
        if ok {
            blobs += 1;
        }
    }

    std::println!("\n=== closing every group from its parts ===");
    std::println!("groups {groups}   blobs whole {blobs} of {}", SHC_BLOB_TABLE.len());
    for f in failures.iter().take(10) {
        std::println!("   {f}");
    }
    assert!(groups > 100_000, "only {groups} groups closed");
    assert!(failures.is_empty(), "{} groups did not close back to themselves", failures.len());
}

/// What decides an instruction's slot, and what decides which phases a group
/// has at all.
///
/// Measured before any packer is written, because a packer built on an
/// assumption and then checked against it would be circular. Three results,
/// two settled and one open:
///
/// **The slot is a function of the phase alone**, over all 576,296 groups:
/// phases 0,1,2 take slots 0,1,2; phase 3 also takes slot 2; phases 4,5,6 take
/// slots 3,4,5; phase 7 also takes slot 0. So packing is a lookup and not a
/// search. It is a function but not injective -- phases 2 and 3 share slot 2,
/// as do 0 and 7 -- which means those pairs are mutually exclusive in a group.
///
/// **`opcnt` is a per-slot presence bitmask**, on the groups that carry it:
/// 1 -> {0}, 3 -> {0,1}, 5 -> {0,2}, 7 -> {0,1,2}, which is bits 0,1,2 of the
/// value naming phases 0,1,2. Exact on every group with the field.
///
/// **`oporg` alone does NOT name the phase set**, and that is the open one. Its
/// value 4 covers both {0,3,4,5} and {0,4}, and 5 covers {0,1,3,4,5} and
/// {0,1,4,5}; the groups carrying `oporg` do not carry `opcnt`. So for a
/// main-ALU group the presence of the later phases is signalled somewhere
/// else, and `MAIN_FIELDS`' `p2end_movc` and `p2end_tst` -- "phase 2 end" --
/// are where to look: the fields would be inside the INSTRUCTIONS rather than
/// the header, which is a different shape of answer from the other two.
///
/// The first version of this unioned phases across all groups sharing an
/// `oporg`, which showed 4 -> {0,3,4,5} and would have supported the false
/// conclusion that `oporg` names the set. Per-group exact sets are what show
/// it does not.
#[test]
fn what_decides_which_slot_an_instruction_occupies() {
    let mut phase_to_slots = std::collections::BTreeMap::<u8, std::collections::BTreeSet<u8>>::new();
    #[allow(clippy::type_complexity)]
    let mut oporg_to_phases: std::collections::BTreeMap<
        (u32, i64),
        std::collections::BTreeSet<std::collections::BTreeSet<u8>>,
    > = std::collections::BTreeMap::new();
    let mut groups = 0usize;

    for (i, _) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let oporg = g.hdr.get("oporg").unwrap_or(u32::MAX);
            /* The EXACT phase set of THIS group, not a union across groups:
             * unioning would show only which phases ever appear under an
             * oporg, which is a much weaker statement than "oporg names the
             * set". */
            let mut this_set = std::collections::BTreeSet::new();
            for (slot, phase, _) in g.instrs() {
                phase_to_slots.entry(phase).or_default().insert(slot);
                this_set.insert(phase);
            }
            let opcnt = g.hdr.get("opcnt").map(|v| v as i64).unwrap_or(-1);
            oporg_to_phases.entry((oporg, opcnt)).or_default().insert(this_set);
            off += g.total;
        }
    }

    std::println!("\n=== which slot does a phase occupy? ({groups} groups) ===");
    let mut one_to_one = true;
    for (phase, slots) in &phase_to_slots {
        let v: Vec<_> = slots.iter().collect();
        std::println!("  phase {phase} -> slots {v:?}");
        if slots.len() != 1 {
            one_to_one = false;
        }
    }
    std::println!("\nphase determines slot uniquely: {one_to_one}");
    std::println!("\n=== (oporg, opcnt) -> the DISTINCT phase sets ===");
    let mut oporg_is_a_function = true;
    for ((oporg, opcnt), sets) in &oporg_to_phases {
        for set in sets {
            let v: Vec<_> = set.iter().collect();
            std::println!("  oporg {oporg} opcnt {opcnt} -> {v:?}");
        }
        if sets.len() != 1 {
            oporg_is_a_function = false;
            std::println!("     ^ {} DISTINCT sets: oporg does not name the set", sets.len());
        }
    }
    std::println!("\n(oporg, opcnt) names the phase set exactly: {oporg_is_a_function}");
    assert!(groups > 100_000);

    /* Settled, and pinned so a table regeneration that moved them would be
     * caught here rather than in a shader that runs the wrong instruction. */
    assert!(one_to_one, "a phase occupied more than one slot; packing is not a lookup");
    for (phase, slot) in [(0u8, 0u8), (1, 1), (2, 2), (3, 2), (4, 3), (5, 4), (6, 5), (7, 0)] {
        if let Some(slots) = phase_to_slots.get(&phase) {
            assert_eq!(slots.iter().copied().collect::<Vec<_>>(), vec![slot], "phase {phase}");
        }
    }
    /* Open, and asserted as open: if this ever becomes true, the comment above
     * is wrong and the packer can be simpler than it is about to be. */
    assert!(!oporg_is_a_function, "oporg now names the phase set exactly; re-read the doc comment");
}

/// Which phase sets actually occur, and how often.
///
/// The packer's rules stated as measurements rather than predictions: a set
/// that never occurs in 576,296 groups is one the packer should refuse to
/// build, and a set that dominates is the one to get right first.
#[test]
fn which_phase_sets_a_group_can_have() {
    extern crate alloc;
    let mut sets = std::collections::BTreeMap::<Vec<u8>, usize>::new();
    let mut groups = 0usize;
    for (i, _) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let mut v: Vec<u8> = g.instrs().map(|(_, p, _)| p).collect();
            v.sort_unstable();
            v.dedup();
            *sets.entry(v).or_default() += 1;
            off += g.total;
        }
    }
    let mut rows: Vec<_> = sets.iter().collect();
    rows.sort_by_key(|(_, n)| core::cmp::Reverse(**n));
    std::println!("\n=== phase sets over {groups} groups ===");
    for (set, n) in &rows {
        std::println!("  {:>7}  {:>5.1}%  {set:?}", n, 100.0 * **n as f64 / groups as f64);
    }
    std::println!("\ndistinct phase sets: {}", rows.len());
    assert!(groups > 100_000);

    /* Twelve sets and no more, out of the 255 a free choice of eight phases
     * would allow. So the packer's rule is a WHITELIST -- accept these and
     * refuse the rest -- which is a stronger and simpler statement than any
     * structural rule derived from them, and it cannot drift without this
     * count changing. */
    assert_eq!(rows.len(), 12, "the set of legal phase combinations changed");

    let has = |v: &[u8]| sets.contains_key(v);
    /* Never alone, though it occurs with 0: a phase-1 operation needs a
     * phase-0 one ahead of it. */
    assert!(!has(&[1]), "phase 1 alone now occurs");
    /* The two slot collisions the phase->slot map implies, confirmed as real
     * exclusions rather than merely unobserved: 2 and 3 share slot 2, 0 and 7
     * share slot 0. */
    for set in sets.keys() {
        assert!(!(set.contains(&2) && set.contains(&3)), "{set:?} has both phases that take slot 2");
        assert!(!(set.contains(&0) && set.contains(&7)), "{set:?} has both phases that take slot 0");
    }
    /* A lone phase-0 group is the overwhelming case, so it is the one to get
     * right first -- but it is 76 %, not the 85 % a guess would put on it. */
    let lone_p0 = *sets.get(&alloc::vec![0u8]).unwrap_or(&0);
    let share = 100.0 * lone_p0 as f64 / groups as f64;
    assert!((70.0..80.0).contains(&share), "a lone phase 0 is {share:.1} % of groups");
}

/// What signals the phase-2 family, and the answer is: nothing does.
///
/// `oporg` was shown not to name the phase set for main-ALU groups, and
/// `MAIN_FIELDS`' `p2end_movc` and `p2end_tst` were the obvious candidates.
/// This measures them and they are **absent from every instruction of every
/// group that `oporg` fails to pin down** -- they are settable in forms those
/// groups do not use. Refuted.
///
/// The answer was in our own decoder the whole time, which is the lesson worth
/// keeping: `decode.rs`'s phase-candidate loop TRIES EVERY NON-EMPTY SUBSET of
/// the phase-2 family and takes the one that parses. So the subset is not
/// signalled at all -- it is implicit in the byte layout, recoverable because
/// the variants' lengths tile exactly.
///
/// That inverts the question for the builder, and makes it easy rather than
/// hard. There is no field to compute: set `oporg` to the coarse class (which
/// of p0, p1, the phase-2 family and the backend phase are present), emit the
/// instructions wanted, and the group reads back as itself. The twelve
/// measured phase sets are exactly the combinations `oporg_slots` can realise.
///
/// Recorded as a test rather than deleted because "we looked and there is no
/// such field" is worth as much to the next reader as finding one, and because
/// if a regeneration ever gives these groups a `p2end_*`, this says so.
#[test]
fn what_signals_the_phase_two_family() {
    /* key: (oporg, p2end_tst seen, p2end_movc seen) -> the exact phase sets */
    #[allow(clippy::type_complexity)]
    let mut obs: std::collections::BTreeMap<(u32, i8, i8), std::collections::BTreeSet<alloc::vec::Vec<u8>>> =
        std::collections::BTreeMap::new();
    extern crate alloc;

    for (i, _) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            let oporg = g.hdr.get("oporg").unwrap_or(u32::MAX);
            /* Only the groups oporg failed to pin down. */
            if oporg == 4 || oporg == 5 || oporg == 1 {
                let mut tst = -1i8;
                let mut movc = -1i8;
                for (_, _, r) in g.instrs() {
                    if let Some(v) = r.get("p2end_tst") {
                        tst = v as i8;
                    }
                    if let Some(v) = r.get("p2end_movc") {
                        movc = v as i8;
                    }
                }
                let mut set: alloc::vec::Vec<u8> = g.instrs().map(|(_, p, _)| p).collect();
                set.sort_unstable();
                set.dedup();
                obs.entry((oporg, tst, movc)).or_default().insert(set);
            }
            off += g.total;
        }
    }

    std::println!("\n=== (oporg, p2end_tst, p2end_movc) -> phase sets ===");
    std::println!("    (-1 = the field is absent from every instruction of the group)");
    let mut determined = true;
    for ((oporg, tst, movc), sets) in &obs {
        for s in sets {
            std::println!("  oporg {oporg}  p2end_tst {tst:>2}  p2end_movc {movc:>2}  -> {s:?}");
        }
        if sets.len() > 1 {
            determined = false;
            std::println!("     ^ {} distinct sets: still not pinned", sets.len());
        }
    }
    std::println!("\np2end_* closes the question: {determined}");
    assert!(
        !determined,
        "p2end_* now pins the phase-2 family; the doc comment above is out of date and \
         the builder can stop relying on the layout being self-describing"
    );
    /* The fields really are absent, rather than present-and-uninformative:
     * -1 is what the probe writes when no instruction of the group has them. */
    for (oporg, tst, movc) in obs.keys() {
        assert_eq!((*tst, *movc), (-1, -1), "oporg {oporg} unexpectedly carries a p2end field");
    }
}

/// The packer against pco: every group's phase set accepted, and the header
/// class it computes equal to the one pco wrote.
///
/// Two claims in one walk. `check` must accept every set that occurs, or the
/// whitelist is wrong and the packer would refuse programs the hardware runs.
/// And `oporg_for`/`opcnt_for` must reproduce the header field pco chose, which
/// is what says the inverse of the decoder's private mapping is right.
#[test]
fn the_packer_accepts_every_corpus_group_and_computes_pcos_header() {
    use super::pack::{check, opcnt_for, oporg_for};
    extern crate alloc;

    let (mut groups, mut with_oporg, mut with_opcnt) = (0usize, 0usize, 0usize);
    let mut refused: Vec<String> = Vec::new();
    let mut wrong_class: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let mut set: alloc::vec::Vec<u8> = g.instrs().map(|(_, p, _)| p).collect();
            set.sort_unstable();
            set.dedup();

            if let Err(e) = check(&set) {
                refused.push(format!("{}: {set:?}: {e:?}", blob.source_name()));
            }
            if let Some(actual) = g.hdr.raw("oporg") {
                with_oporg += 1;
                if oporg_for(&set) != Some(actual) {
                    wrong_class.push(format!(
                        "{}: {set:?}: oporg {:?} computed, pco wrote {actual}",
                        blob.source_name(),
                        oporg_for(&set)
                    ));
                }
            } else if let Some(actual) = g.hdr.raw("opcnt") {
                with_opcnt += 1;
                if opcnt_for(&set) != Some(actual) {
                    wrong_class.push(format!(
                        "{}: {set:?}: opcnt {:?} computed, pco wrote {actual}",
                        blob.source_name(),
                        opcnt_for(&set)
                    ));
                }
            }
            if refused.len() + wrong_class.len() > 20 {
                break;
            }
            off += g.total;
        }
    }

    std::println!("\n=== the packer against pco ===");
    std::println!("groups {groups}   with oporg {with_oporg}   with opcnt {with_opcnt}");
    std::println!("  refused by check()      {}", refused.len());
    std::println!("  wrong header class      {}", wrong_class.len());
    for m in refused.iter().take(5) {
        std::println!("   refused: {m}");
    }
    for m in wrong_class.iter().take(5) {
        std::println!("   class:   {m}");
    }

    assert!(groups > 100_000, "only {groups} groups walked");
    assert!(refused.is_empty(), "{} groups the hardware runs were refused", refused.len());
    assert!(wrong_class.is_empty(), "{} groups got a header class pco disagrees with", wrong_class.len());
}

/// Choose a section, fill it, read it back: the operands must be identical.
///
/// The property the builder stands on, over every operand set in the table.
/// The three pieces are tested as a unit because that is how they are used:
/// `choose_src_variant` picks a form, `fill_srcs` writes the registers into
/// it, and `srcs` is what the hardware's decoder equivalent will see. A
/// mismatch anywhere in the three shows up here as a register that changed
/// between being written and being read.
#[test]
fn choosing_a_section_filling_it_and_reading_it_back_is_the_identity() {
    use super::isa::{BitSetId, Region};
    use super::legalise::{choose_dst_variant, choose_src_variant, fill_dsts, fill_srcs};
    use super::operands::{dsts, srcs};

    let (mut src_ok, mut dst_ok) = (0usize, 0usize);
    let mut bad: Vec<String> = Vec::new();

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            for r in [&g.lower, &g.upper].into_iter().flatten() {
                let want = srcs(r);
                match choose_src_variant(&want, 0) {
                    Ok(v) => {
                        let mut fresh = Region::new(BitSetId::Src, v);
                        if let Err(e) = fill_srcs(&mut fresh, &want, 0) {
                            bad.push(format!("{}: fill_srcs {e:?}", blob.source_name()));
                        } else if srcs(&fresh) != want {
                            bad.push(format!(
                                "{}: variant {} -> {}\n      want {:?}\n      got  {:?}",
                                blob.source_name(),
                                super::isa::bit_set(BitSetId::Src).variants[r.variant as usize].name,
                                super::isa::bit_set(BitSetId::Src).variants[v as usize].name,
                                want,
                                srcs(&fresh)
                            ));
                        } else {
                            src_ok += 1;
                        }
                    }
                    Err(e) => bad.push(format!("{}: choose_src {e:?}", blob.source_name())),
                }
            }
            if let Some(r) = &g.dst {
                let want = dsts(r);
                match choose_dst_variant(&want) {
                    Ok(v) => {
                        let mut fresh = Region::new(BitSetId::Dst, v);
                        if let Err(e) = fill_dsts(&mut fresh, &want) {
                            bad.push(format!("{}: fill_dsts {e:?}", blob.source_name()));
                        } else if dsts(&fresh) != want {
                            bad.push(format!("{}: dsts changed through choose+fill", blob.source_name()));
                        } else {
                            dst_ok += 1;
                        }
                    }
                    Err(e) => bad.push(format!("{}: choose_dst {e:?}", blob.source_name())),
                }
            }
            if bad.len() > 20 {
                break;
            }
            off += g.total;
        }
    }

    std::println!("\n=== choose -> fill -> read back ===");
    std::println!("source sections {src_ok}   destination sections {dst_ok}   mismatches {}", bad.len());
    for m in bad.iter().take(8) {
        std::println!("   {m}");
    }
    assert!(src_ok > 100_000 && dst_ok > 100_000, "the walk found too little: {src_ok}/{dst_ok}");
    assert!(bad.is_empty(), "{} sections did not survive choose+fill+read", bad.len());
}

/// What do `w0p` and `w1p` mean?
///
/// Measured because the builder emitted 37 groups that decoded to nothing and
/// these were the only header fields it never set. A guess would have been
/// cheap here and wrong is expensive, so the corpus answers it.
#[test]
fn what_w0p_and_w1p_track() {
    use super::operands::dsts;
    let mut tbl: std::collections::BTreeMap<(u32, u32), std::collections::BTreeSet<(bool, bool)>> =
        std::collections::BTreeMap::new();
    let mut groups = 0usize;

    for (i, _) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let w0 = g.hdr.get("w0p").unwrap_or(u32::MAX);
            let w1 = g.hdr.get("w1p").unwrap_or(u32::MAX);
            let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
            tbl.entry((w0, w1)).or_default().insert((d[0].is_some(), d[1].is_some()));
            off += g.total;
        }
    }

    std::println!("\n=== (w0p, w1p) -> which destination slots are occupied ({groups} groups) ===");
    let mut exact = true;
    for ((w0, w1), sets) in &tbl {
        for s in sets {
            std::println!("  w0p {w0} w1p {w1} -> dst0 {} dst1 {}", s.0, s.1);
        }
        if sets.len() > 1 {
            exact = false;
        }
    }
    std::println!("\nthe pair determines the destination slots exactly: {exact}");
    assert!(groups > 100_000);
}

/// pco's own groups for the shader the builder is trying to reproduce.
///
/// The oracle for USC-2, printed rather than asserted: every header field of
/// the first groups of `texturefill.batchable.vert`, so a built group can be
/// diffed against the real one field by field instead of by its bytes.
///
/// # What it already showed
///
/// The builder emitted 37 groups that did not parse, and this says why in
/// three differences, none of which a reading of the tables would have given:
///
/// 1. **The header variant is `main` (3 bytes), not `main_brief` (2).** The
///    builder picked the first variant carrying `oporg`, `length` and `da`,
///    which is the brief one. pco uses the full one even for the simplest
///    group.
/// 2. **The UPPER source section is present** -- `1up_1b6i` -- on a group with
///    a single phase-0 instruction reading one source. The builder emits none.
///    This is also why an earlier probe found "no group of the simplest shape
///    in the whole corpus": it required `upper.is_none()`, and that shape does
///    not exist. The filter was right and the assumption behind it was wrong.
/// 3. **The ISS section is present.** The builder emits none.
///
/// So the remaining failure is not a missing bit but three missing or wrong
/// structural choices, and `rpt` is a non-issue: it is `SubOne`, so the zero a
/// fresh region holds already decodes as 1, which is what pco writes.
#[test]
fn pcos_groups_for_the_usc2_shader() {
    extern crate alloc;
    let name_matches = |n: &str| n.starts_with("texturefill.batchable.vert");
    let Some(i) = SHC_BLOB_TABLE.iter().position(|b| name_matches(b.source_name())) else {
        std::println!("texturefill.batchable.vert: no blob in the table");
        return;
    };
    let hdrs = super::isa::bit_set(super::isa::BitSetId::IgrpHdr);
    let bytes = usc_bytes(i);
    std::println!("\n=== {} : {} bytes ===", SHC_BLOB_TABLE[i].source_name(), bytes.len());

    let mut off = 0;
    let mut n = 0;
    while off < bytes.len() && n < 4 {
        let Ok(g) = decode_group(&bytes, off) else { break };
        if g.total == 0 {
            break;
        }
        let phases: alloc::vec::Vec<u8> = g.instrs().map(|(_, p, _)| p).collect();
        let v = &hdrs.variants[g.hdr.variant as usize];
        std::println!("\ngroup {n} at +{off:#x}, {} bytes: {:02x?}", g.total, &bytes[off..off + g.total]);
        std::println!("  phases {phases:?}   alutype {}", g.alutype);
        std::println!("  header variant {} ({} B)", v.name, v.bytes);
        for u in v.uses.iter().filter(|u| u.fixed.is_none() && !u.name.is_empty()) {
            std::println!("    {:10} = {:?}", u.name, g.hdr.get(u.name));
        }
        for (label, r, id) in [
            ("lower", &g.lower, super::isa::BitSetId::Src),
            ("upper", &g.upper, super::isa::BitSetId::Src),
            ("iss", &g.iss, super::isa::BitSetId::Iss),
            ("dst", &g.dst, super::isa::BitSetId::Dst),
        ] {
            match r {
                None => std::println!("  {label:6} ABSENT"),
                Some(r) => {
                    let set = super::isa::bit_set(id);
                    let v = &set.variants[r.variant as usize];
                    std::println!("  {label:6} {}", v.name);
                    for u in v.uses.iter().filter(|u| u.fixed.is_none() && !u.name.is_empty()) {
                        std::println!("      {:8} = {:?}", u.name, r.get(u.name));
                    }
                }
            }
        }
        for (slot, phase, ir) in g.instrs() {
            std::println!(
                "  instr  slot {slot} phase {phase}: set {:?} variant {}",
                ir.set,
                super::isa::bit_set(ir.set).variants[ir.variant as usize].name
            );
        }
        std::println!("  word_pad {}  align_pad {}", g.word_pad, g.align_pad);
        off += g.total;
        n += 1;
    }
    assert!(n > 0, "no group decoded from the USC-2 shader's blob");
}

/// Which predicate empties the "simplest shape" filter, one at a time.
///
/// A probe found no phase-0-only group with one source section, one
/// destination and no upper section, and the oracle in
/// `pcos_groups_for_the_usc2_shader` suggested why: the two real groups it
/// printed both had an upper section. But two examples are not a corpus, and
/// "these two have it" is not "every group has it" -- the same gap between
/// always and sometimes that the oporg measurement fell into.
///
/// So this counts, over all 576,296 groups, how many survive each predicate as
/// it is added. The one that takes the count to zero is the answer, measured.
#[test]
fn which_predicate_empties_the_simplest_shape() {
    extern crate alloc;
    let mut counts = [0usize; 6];
    let mut groups = 0usize;

    for (i, _) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let phases: alloc::vec::Vec<u8> = g.instrs().map(|(_, p, _)| p).collect();
            /* Cumulative: each row adds one predicate to the row above. */
            let p0 = phases.as_slice() == [0];
            let p1 = p0 && g.lower.is_some();
            let p2 = p1 && g.dst.is_some();
            let p3 = p2 && g.align_pad == 0;
            let p4 = p3 && g.upper.is_none();
            let p5 = p4 && g.iss.is_none();
            for (k, ok) in [p0, p1, p2, p3, p4, p5].into_iter().enumerate() {
                if ok {
                    counts[k] += 1;
                }
            }
            off += g.total;
        }
    }

    let labels = [
        "phases == [0]",
        "  + a lower source section",
        "  + a destination",
        "  + no alignment padding",
        "  + NO UPPER section",
        "  + NO ISS section",
    ];
    std::println!("\n=== which predicate empties the filter ({groups} groups) ===");
    for (k, l) in labels.iter().enumerate() {
        std::println!("  {:>7}  {l}", counts[k]);
    }

    assert!(counts[0] > 400_000, "a lone phase 0 should be most of the corpus");
    /* The finding, pinned: an upper source section is present in EVERY group
     * of this shape, so a builder that omits it emits something the hardware
     * has no way to read. Not an inference from two examples any more. */
    assert_eq!(counts[4], 0, "a group of this shape without an upper section now exists");
    assert!(counts[3] > 0, "the shape itself must occur, or the zero above means nothing");
}

/// What decides the header variant, and whether the upper and ISS sections
/// are present, over every group in the table.
///
/// The builder first chose the header variant by capability (first one
/// carrying `oporg`, `length`, `da`) and then by the name `main`. Both are
/// guesses; the second is the first with a different constant. This measures
/// the rule instead, the way the section and form choosers were measured.
#[test]
fn what_decides_the_header_variant_and_the_optional_sections() {
    extern crate alloc;
    use alloc::collections::BTreeMap;

    let hdrs = super::isa::bit_set(super::isa::BitSetId::IgrpHdr);
    let mut by_variant: BTreeMap<&str, usize> = BTreeMap::new();
    /* For each variant, which alutypes use it. */
    let mut alutypes: BTreeMap<&str, alloc::collections::BTreeSet<u32>> = BTreeMap::new();
    let mut upper_absent = 0usize;
    let mut iss_absent = 0usize;
    let mut lower_absent = 0usize;
    /* The shapes of the groups that lack an upper or an ISS section. */
    let mut upper_absent_shapes: BTreeMap<alloc::string::String, usize> = BTreeMap::new();
    let mut iss_absent_shapes: BTreeMap<alloc::string::String, usize> = BTreeMap::new();
    let mut groups = 0usize;

    for (i, _) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let vname = hdrs.variants[g.hdr.variant as usize].name;
            *by_variant.entry(vname).or_default() += 1;
            alutypes.entry(vname).or_default().insert(g.alutype);

            let shape = alloc::format!(
                "alutype {} lower {} upper {} iss {} dst {}",
                g.alutype,
                g.lower.is_some(),
                g.upper.is_some(),
                g.iss.is_some(),
                g.dst.is_some()
            );
            if g.upper.is_none() {
                upper_absent += 1;
                *upper_absent_shapes.entry(shape.clone()).or_default() += 1;
            }
            if g.iss.is_none() {
                iss_absent += 1;
                *iss_absent_shapes.entry(shape).or_default() += 1;
            }
            if g.lower.is_none() {
                lower_absent += 1;
            }
            off += g.total;
        }
    }

    std::println!("\n=== header variant, over {groups} groups ===");
    for (v, n) in &by_variant {
        std::println!("  {n:>7}  {v:12}  alutypes {:?}", alutypes[v]);
    }
    std::println!("\n=== optional sections ===");
    std::println!("  lower absent {lower_absent}");
    std::println!("  upper absent {upper_absent}");
    for (s, n) in upper_absent_shapes.iter().take(6) {
        std::println!("      {n:>7}  {s}");
    }
    std::println!("  iss   absent {iss_absent}");
    for (s, n) in iss_absent_shapes.iter().take(6) {
        std::println!("      {n:>7}  {s}");
    }
    assert!(groups > 100_000);
}

/// The measured rule, asserted against every group: the header variant, the
/// upper section's presence and the ISS section's presence are all predictable
/// from the group's own content, and the prediction is right 576,296 times.
///
/// The same validation the section and form choosers got, and the reason it
/// exists: the builder chose the header variant twice by guessing -- first the
/// first variant with the right fields, then the name `main` -- and only a
/// prediction checked against the corpus can say a rule is the rule.
#[test]
fn the_header_and_section_rules_predict_every_group() {
    use super::pack::{header_variant_of, iss_present, upper_present};

    let hdrs = super::isa::bit_set(super::isa::BitSetId::IgrpHdr);
    let (mut checked, mut wrong) = (0usize, 0usize);
    let mut first: Option<std::string::String> = None;

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            checked += 1;
            let want_v = hdrs.variants[g.hdr.variant as usize].name;
            let got_v = header_variant_of(g.alutype).map(|i| hdrs.variants[i as usize].name);
            let ok_v = got_v == Some(want_v);
            let ok_u = upper_present(g.lower.is_some()) == g.upper.is_some();
            let ok_i = iss_present(g.alutype, g.lower.is_some()) == g.iss.is_some();
            if !(ok_v && ok_u && ok_i) {
                wrong += 1;
                if first.is_none() {
                    first = Some(std::format!(
                        "{}: alutype {} lower {} -> variant {:?} (want {want_v}), upper {} (want {}), iss {} (want {})",
                        blob.source_name(),
                        g.alutype,
                        g.lower.is_some(),
                        got_v,
                        upper_present(g.lower.is_some()),
                        g.upper.is_some(),
                        iss_present(g.alutype, g.lower.is_some()),
                        g.iss.is_some()
                    ));
                }
            }
            off += g.total;
        }
    }
    std::println!("\n=== the header and section rules against pco ===");
    std::println!("  groups {checked}   mispredicted {wrong}");
    if let Some(f) = &first {
        std::println!("  first: {f}");
    }
    assert!(checked > 100_000);
    assert_eq!(wrong, 0, "the rule mispredicted {wrong} groups; first: {first:?}");
}

/// USC-2's host gate: pco's own group content, through the builder, byte for
/// byte.
///
/// The obvious diff -- our compiler's output against pco's blob for the same
/// source -- cannot be a gate, because two compilers with different allocators
/// and different selection will never agree byte for byte, so "the differences
/// are explainable" is a reading rather than a pass. This removes both from the
/// question: take pco's OWN decoded groups, feed their content back through
/// the builder's choosers, fillers, packer and assembler, and require the same
/// bytes out.
///
/// It exercises everything the composition does except the shape table -- the
/// operations come from the decoded group rather than from `shape(op)`,
/// because several kernel operations share one encoded form and the mapping
/// back is not a function. `shape` is validated separately against the ISA.
#[test]
fn pcos_own_groups_rebuilt_through_the_builder_are_byte_identical() {
    use super::assemble::{close, Parts};
    use super::instr::{instr_region, ABS, FLR, NEG};
    use super::isa::{bit_set, ops_for, BitSetId, Region};
    use super::legalise::{choose_dst_variant, choose_src_variant, fill_dsts, fill_srcs};
    use super::operands::{dsts, srcs};
    use super::pack::{header_variant_of, iss_present, upper_present};
    use super::sel::Mods;

    let (mut same, mut rebuilt) = (0usize, 0usize);
    let mut differing: Vec<String> = Vec::new();
    let mut skipped: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut field_diffs: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut uvsw_variants: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut uvsw_ops: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut out = vec![0u8; 64];

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            /* Only what the builder can express today. Everything else is
             * counted by reason rather than silently passed. */

            /* Every instruction of the group, each into the slot its phase
             * takes. A group the builder cannot express is counted by reason;
             * none is silently passed. */
            let mut instrs = [None; super::decode::MAX_INSTRS];
            let mut pinned: Option<u16> = None;
            let mut first_op: Option<&'static str> = None;
            let mut refused: Option<&str> = None;
            for (slot, phase, ir) in g.instrs() {
                let Some(op) = ops_for(ir, phase).next() else {
                    refused = Some("the tables name no operation for this form");
                    break;
                };
                if first_op.is_none() {
                    first_op = Some(op.name);
                }
                let m = |n: &str| ir.get(n).unwrap_or(0) != 0;
                let mods = [
                    Mods { neg: m("s0neg"), abs: m("s0abs"), flr: m("s0flr") },
                    Mods { neg: m("s1neg"), abs: m("s1abs"), flr: m("s1flr") },
                    Mods { neg: m("s2neg"), abs: m("s2abs"), flr: m("s2flr") },
                ];
                /* Every modifier of pco's instruction that is not a source
                 * modifier, handed back to the builder as content. The names
                 * come from pco's own variant, so this stays a measurement: if
                 * the builder picks a form that cannot carry one it refuses,
                 * and the group is counted. */
                let iset = bit_set(ir.set);
                let extra: std::vec::Vec<(&'static str, u32)> = iset.variants[ir.variant as usize]
                    .uses
                    .iter()
                    .filter(|u| u.fixed.is_none() && !u.name.is_empty())
                    .filter(|u| !NEG.contains(&u.name) && !ABS.contains(&u.name) && !FLR.contains(&u.name))
                    .filter_map(|u| ir.get(u.name).map(|v| (u.name, v)))
                    .collect();
                let Ok(instr) = instr_region(op.name, phase, &mods, &extra) else {
                    refused = Some("no form carries these modifiers");
                    break;
                };
                if let Some(v) = super::instr::pinned_src_variant(op.name) {
                    if pinned.is_some_and(|p| p != v) {
                        refused = Some("two operations pinning different source sections");
                        break;
                    }
                    pinned = Some(v);
                }
                instrs[slot as usize] = Some((phase, instr));
            }
            if let Some(why) = refused {
                *skipped.entry(why).or_default() += 1;
                off += g.total;
                continue;
            }
            let op_name = first_op.unwrap_or("<no instruction>");

            let src_ops = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
            /* pco's own source mux, handed back as content like the
             * instruction modifiers: the builder still chooses the variant. */
            let mux = g.lower.as_ref().and_then(|l| l.get("is0")).unwrap_or(0);
            let dst_ops = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
            let lower = match src_ops.iter().any(Option::is_some) {
                false => None,
                true => match pinned.map(Ok).unwrap_or_else(|| choose_src_variant(&src_ops, mux)) {
                    Ok(v) => {
                        let mut r = Region::new(BitSetId::Src, v);
                        if fill_srcs(&mut r, &src_ops, mux).is_err() {
                            *skipped.entry("fill_srcs refused").or_default() += 1;
                            off += g.total;
                            continue;
                        }
                        Some(r)
                    }
                    Err(_) => {
                        *skipped.entry("choose_src refused").or_default() += 1;
                        off += g.total;
                        continue;
                    }
                },
            };
            let dst = match dst_ops[0].is_some() {
                false => None,
                true => match choose_dst_variant(&dst_ops) {
                    Ok(v) => {
                        let mut r = Region::new(BitSetId::Dst, v);
                        let _ = fill_dsts(&mut r, &dst_ops);
                        Some(r)
                    }
                    Err(_) => {
                        *skipped.entry("choose_dst refused").or_default() += 1;
                        off += g.total;
                        continue;
                    }
                },
            };

            /* The sections and header by the measured rules, not by copying
             * pco's choice -- copying would make this test vacuous. */
            let has_lower = lower.is_some();
            let upper = upper_present(has_lower).then(|| *g.upper.as_ref().unwrap());
            let iss = iss_present(g.alutype, has_lower).then(|| *g.iss.as_ref().unwrap());
            let Some(hv) = header_variant_of(g.alutype) else {
                *skipped.entry("no header variant for this ALU type").or_default() += 1;
                off += g.total;
                continue;
            };
            let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
            /* Every header field that is CONTENT, taken from pco; `length` and
             * `da` are the two this builder derives and are deliberately not
             * copied, so the gate still tests them. Naming the content fields
             * one by one is how `ctrlop` came to be missed for all 19,249
             * control groups -- for those the operation lives in the header,
             * not in an instruction -- so the list is a rule, not a list. */
            for u in bit_set(BitSetId::IgrpHdr).variants[hv as usize].uses.iter() {
                if u.fixed.is_some() || u.name.is_empty() || u.name == "length" || u.name == "da" {
                    continue;
                }
                if let Some(v) = g.hdr.get(u.name) {
                    let _ = hdr.set(u.name, v);
                }
            }

            /* Every group of this operation, matched or not: a rule read off
             * only the failures is a rule fitted to them. */
            if let Some(l) = g.lower.as_ref() {
                let vn = bit_set(l.set).variants[l.variant as usize].name;
                for (_, ph, ir) in g.instrs() {
                    if ph != PH_BACKEND {
                        continue;
                    }
                    if let Some(o) = ops_for(ir, ph).next() {
                        *uvsw_variants.entry(vn).or_default() += 1;
                        *uvsw_ops.entry(o.name).or_default() += 1;
                    }
                }
            }
            let parts = Parts { alutype: g.alutype, instrs, lower, upper, iss, dst, align_pad: g.align_pad };
            match close(&parts, hdr) {
                Ok(built) => {
                    rebuilt += 1;
                    if encode_group(&built, &mut out, 0).is_ok() && out[..built.total] == bytes[off..off + g.total] {
                        same += 1;
                    } else {
                        /* WHICH FIELD, not which byte. Decode the built group
                         * and compare its instruction's settable fields with
                         * pco's, so the answer is a field name rather than a
                         * bit position to be interpreted. */
                        let mut named = false;
                        let redecode = decode_group(&out[..built.total], 0);
                        if let Err(e) = &redecode {
                            *field_diffs
                                .entry(match e {
                                    DecodeError::Truncated => "  reject: Truncated",
                                    DecodeError::NoParse => "  reject: NoParse",
                                    DecodeError::Ambiguous => "  reject: Ambiguous",
                                })
                                .or_default() += 1;
                        }
                        if let Ok(back) = redecode {
                            if let (Some((_, _, a)), Some((_, _, b))) = (back.instrs().next(), g.instrs().next()) {
                                let set = bit_set(a.set);
                                if a.variant == b.variant {
                                    for u in set.variants[a.variant as usize]
                                        .uses
                                        .iter()
                                        .filter(|u| u.fixed.is_none() && !u.name.is_empty())
                                    {
                                        if a.get(u.name) != b.get(u.name) {
                                            *field_diffs.entry(u.name).or_default() += 1;
                                        }
                                    }
                                    named = set.variants[a.variant as usize]
                                        .uses
                                        .iter()
                                        .filter(|u| u.fixed.is_none() && !u.name.is_empty())
                                        .any(|u| a.get(u.name) != b.get(u.name));
                                } else {
                                    named = true;
                                    *field_diffs.entry("<a different instruction variant>").or_default() += 1;
                                }
                            }
                        } else {
                            named = true;
                            *field_diffs.entry("<the built bytes do not decode>").or_default() += 1;
                        }
                        *field_diffs
                            .entry(match (g.alutype, g.instrs().count(), g.align_pad) {
                                (_, _, p) if p != 0 => "  class: alignment block",
                                (0, 1, _) => "  class: main, one instruction",
                                (0, _, _) => "  class: main, several instructions",
                                (2, _, _) => "  class: bitwise",
                                _ => "  class: control",
                            })
                            .or_default() += 1;
                        if differing.len() < 3 {
                            let hset = bit_set(BitSetId::IgrpHdr);
                            differing.push(std::format!(
                                "{} +{off:#x}: alutype {} instrs {} pad {}\n      hdr mine/pco  {}",
                                blob.source_name(),
                                g.alutype,
                                g.instrs().count(),
                                g.align_pad,
                                hset.variants[g.hdr.variant as usize]
                                    .uses
                                    .iter()
                                    .filter(|u| u.fixed.is_none() && !u.name.is_empty())
                                    .map(|u| std::format!(
                                        "{}={:?}/{:?}",
                                        u.name,
                                        built.hdr.get(u.name),
                                        g.hdr.get(u.name)
                                    ))
                                    .collect::<std::vec::Vec<_>>()
                                    .join(" ")
                            ));
                        }
                        *field_diffs
                            .entry(match built.total.cmp(&g.total) {
                                core::cmp::Ordering::Less => "  length: built SHORTER than pco",
                                core::cmp::Ordering::Equal => "  length: built equals pco",
                                core::cmp::Ordering::Greater => "  length: built LONGER than pco",
                            })
                            .or_default() += 1;
                        if !named {
                            *field_diffs.entry("<every instruction field agrees: elsewhere>").or_default() += 1;
                        }
                        if let Ok(back) = &redecode {
                            for (which, a, b) in [
                                ("lower", &back.lower, &g.lower),
                                ("upper", &back.upper, &g.upper),
                                ("iss", &back.iss, &g.iss),
                                ("dst", &back.dst, &g.dst),
                            ] {
                                match (a, b) {
                                    (Some(a), Some(b)) if a.variant != b.variant && which == "lower" => {
                                        if differing.len() < 5 {
                                            let vs = bit_set(a.set);
                                            differing.push(std::format!(
                                                "{} +{off:#x}: lower mine={} pco={}  mux={}\n      pco fields  {}",
                                                blob.source_name(),
                                                vs.variants[a.variant as usize].name,
                                                vs.variants[b.variant as usize].name,
                                                mux,
                                                vs.variants[b.variant as usize]
                                                    .uses
                                                    .iter()
                                                    .filter(|u| u.fixed.is_none() && !u.name.is_empty())
                                                    .map(|u| std::format!("{}={:?}", u.name, b.get(u.name)))
                                                    .collect::<std::vec::Vec<_>>()
                                                    .join(" ")
                                            ));
                                        }
                                        *field_diffs.entry("  section: lower, a different variant").or_default() += 1;
                                        *field_diffs
                                            .entry(match (op_name, g.hdr.get("rpt").unwrap_or(0)) {
                                                (_, 0) => "    narrow: rpt=0",
                                                (_, 1) => "    narrow: rpt=1",
                                                _ => "    narrow: rpt>1",
                                            })
                                            .or_default() += 1;
                                        *field_diffs.entry(op_name).or_default() += 1;
                                        *field_diffs
                                            .entry(if g.upper.is_some() {
                                                "    narrow: has upper"
                                            } else {
                                                "    narrow: no upper"
                                            })
                                            .or_default() += 1;
                                    }
                                    (Some(a), Some(b)) if a.variant != b.variant => {
                                        *field_diffs
                                            .entry(match which {
                                                "lower" => "  section: lower, a different variant",
                                                "upper" => "  section: upper, a different variant",
                                                "iss" => "  section: iss, a different variant",
                                                _ => "  section: dst, a different variant",
                                            })
                                            .or_default() += 1;
                                    }
                                    (Some(a), Some(b)) => {
                                        for u in bit_set(a.set).variants[a.variant as usize]
                                            .uses
                                            .iter()
                                            .filter(|u| u.fixed.is_none() && !u.name.is_empty())
                                        {
                                            if a.get(u.name) != b.get(u.name) {
                                                *field_diffs.entry(u.name).or_default() += 1;
                                            }
                                        }
                                    }
                                    (None, Some(_)) | (Some(_), None) => {
                                        *field_diffs.entry("  section: present on one side only").or_default() += 1;
                                    }
                                    (None, None) => {}
                                }
                            }
                        }
                        if false {
                            let n = built.total.min(g.total);
                            let first = (0..n).find(|k| out[*k] != bytes[off + k]);
                            differing.push(format!(
                            "{} +{off:#x}: built {} B vs {} B, first differing byte {:?}\n      built {:02x?}\n      pco   {:02x?}",
                            blob.source_name(),
                            built.total,
                            g.total,
                            first,
                            &out[..built.total],
                            &bytes[off..off + g.total]
                        ));
                        }
                    }
                }
                Err(_) => *skipped.entry("close refused").or_default() += 1,
            }
            off += g.total;
        }
    }

    std::println!("\n=== pco's own groups rebuilt through the builder ===");
    std::println!("  rebuilt {rebuilt}   byte-identical {same}   differing {}", rebuilt - same);
    for (why, n) in &skipped {
        std::println!("  skipped {n:>7}  {why}");
    }
    std::println!("  backend-phase ops: {uvsw_ops:?}");
    std::println!("  pco's lower-section variant for every backend-phase group:");
    for (v, n) in &uvsw_variants {
        std::println!("      {n:>7}  {v}");
    }
    /* The evidence for `instr::pinned_src_variant`, asserted rather than
     * printed: the backend phase is NOT the axis -- it uses the 1-byte form
     * 480 times -- so the pin belongs to the operation. Were this only a
     * printout, a later table change could quietly remove the ground under
     * the pin and the gate would still read 0. */
    assert!(uvsw_variants.contains_key("1lo_1b6i"), "the backend phase would then be the axis, not the operation");
    // 792 before the 20 matattr rows; matattr2/3/4.vert write 6 more.
    assert_eq!(uvsw_ops.get("uvsw.write"), Some(&798), "the population the pin was measured over changed");
    assert_eq!(
        uvsw_variants.get("1lo_3b11i_2m").copied(),
        Some(uvsw_ops["uvsw.write"] + uvsw_ops["uvsw.write.emit.endtask"]),
        "the pinned form is used by exactly the pinned operations and no other"
    );
    std::println!("  which instruction field differs:");
    for (f, n) in &field_diffs {
        std::println!("      {n:>7}  {f}");
    }
    for d in differing.iter() {
        std::println!("   {d}");
    }
    /* Gate 1 of USC-2. pco's own content, re-encoded through our builder with
     * OUR section variants, header variant, `length` and `da` -- so a pass
     * says our encoding rules agree with the reference compiler's, over its
     * whole corpus, independently of our allocator, which never runs here.
     *
     * The two assertions belong together. A zero difference count says
     * nothing on its own: the first version of this harness rebuilt only
     * single-instruction main-ALU groups and read 0 while quietly skipping
     * 159,305 of the 576,296. So the count of groups actually rebuilt is
     * pinned to the corpus size, and any skip at all fails the gate by
     * name. */
    assert!(skipped.is_empty(), "groups the harness did not rebuild: {skipped:?}");
    // 651_369 + 399 (the 20 matattr rows) + 7_266 (the 48 offarm rows, gl_test 193)
    // + 26_530 (the 160 offarm rows, gl_test 194)
    assert_eq!(rebuilt, 685_564, "the gate no longer covers the whole corpus");
    assert_eq!(rebuilt - same, 0, "groups whose bytes our builder does not reproduce from pco's own content");
    assert!(rebuilt > 1000, "only {rebuilt} groups went through the builder");
}

/// The operand widths every register file actually has, per source slot.
///
/// Owed to the allocator: it decides where a value lives, and a bank it can
/// name in slot 0 may be unnameable in slot 2, so "how many temporaries" is
/// not the only limit that matters. Measured from the source-section
/// variants rather than read off a document -- a slot can hold a bank only
/// if some variant's bank field is wide enough for it, and the index it can
/// reach is that variant's index width.
#[test]
fn the_widest_operand_each_bank_can_reach_in_each_source_slot() {
    use super::isa::{bit_set, BitSetId};
    use super::operands::src_capability;
    const BANKS: [(u32, &str); 8] = [
        (0, "special"),
        (1, "temp"),
        (2, "vtxin"),
        (3, "coeff"),
        (4, "shared"),
        (5, "coeff_alt"),
        (6, "idx0"),
        (7, "idx1"),
    ];
    let set = bit_set(BitSetId::Src);
    std::println!("=== the widest index each bank reaches, by source slot ===");
    let mut temp_slot0 = 0u32;
    for (bank, name) in BANKS {
        let mut per_slot = [0i64; 6];
        for (slot, cell) in per_slot.iter_mut().enumerate() {
            let mut best: i64 = -1;
            for vi in 0..set.variants.len() {
                let Some(c) = src_capability(vi as u16)[slot] else { continue };
                if !c.holds(&super::operands::Operand { bank, index: 0 }) {
                    continue;
                }
                /* The reachable index is whatever the field accepts, asked of
                 * the field rather than computed from a width this test
                 * would then have to keep in step. */
                let top = (0..=31)
                    .rev()
                    .map(|b| (1u32 << b) - 1)
                    .find(|i| c.holds(&super::operands::Operand { bank, index: *i }))
                    .unwrap_or(0);
                best = best.max(top as i64);
            }
            *cell = best;
        }
        if bank == 1 {
            temp_slot0 = per_slot[0].max(0) as u32;
        }
        std::println!("  {name:>9}  {per_slot:?}");
    }
    /* The two facts the allocator needs, pinned rather than printed.
     *
     * The six slots are not alike: 0, 2 and 3 reach index 2047 in every bank,
     * while 1, 4 and 5 stop at 255 AND cannot name `shared` at all. So a
     * uniform can only be read from three of the six slots, and a group
     * wanting one in slot 1 needs a move -- that is a legalisation
     * consequence of the encoding, not a choice, and it is why
     * `choose_src_variant` can refuse a perfectly well formed group. */
    assert_eq!(temp_slot0, 2047, "a temporary's reach in slot 0 changed");
    let shared_in = |slot: usize| {
        (0..set.variants.len())
            .filter_map(|vi| src_capability(vi as u16)[slot])
            .any(|c| c.holds(&super::operands::Operand { bank: 4, index: 0 }))
    };
    assert!(shared_in(0) && shared_in(2) && shared_in(3), "a uniform must be readable in slots 0, 2 and 3");
    assert!(
        !shared_in(1) && !shared_in(4) && !shared_in(5),
        "a uniform became readable in a slot that could not name it, so the move legalisation inserts is no longer needed"
    );
}

/// The vertex output layout the shipped PowerVR blobs use.
///
/// The epilogue's write ORDER and the record's `vtxout_off` describe the same
/// thing, so they must come from one place; this measures what that place has
/// to produce. `gl_Position` first, then the varyings -- but whether the
/// varyings follow position immediately, and in what order, is the table's to
/// say, not mine.
#[test]
fn where_the_shipped_blobs_put_position_and_the_varyings_in_the_vertex_output() {
    use crate::shaderc::mesa_reference::SHC_BLOB_TABLE;
    use crate::shaderc::SHC_STAGE_VERTEX;
    let mut shapes: std::collections::BTreeMap<std::string::String, usize> = std::collections::BTreeMap::new();
    let (mut vs, mut with_varyings) = (0usize, 0usize);
    for b in SHC_BLOB_TABLE.iter().filter(|b| b.stage == SHC_STAGE_VERTEX && b.blend_key == 0) {
        vs += 1;
        let n = (b.shader.num_varyings as usize).min(b.shader.varyings.len());
        if n == 0 {
            continue;
        }
        with_varyings += 1;
        let mut offs: std::vec::Vec<(u8, u8)> =
            b.shader.varyings[..n].iter().map(|v| (v.vtxout_off, v.components)).collect();
        offs.sort();
        /* Is the layout exactly position followed by the varyings packed in
         * offset order with no holes? Recorded as a shape rather than
         * asserted, because a hole would be a real fact about the layout. */
        let mut want = 4u8; // gl_Position is four dwords
        let packed = offs.iter().all(|(off, comp)| {
            let ok = *off == want;
            want += comp;
            ok
        });
        let shape = if packed {
            std::format!("position(4) then {n} varyings packed, size {want}")
        } else {
            std::format!("NOT packed after position: {offs:?}")
        };
        *shapes.entry(shape.chars().take(46).collect()).or_default() += 1;
    }
    std::println!("=== vertex output layout in the shipped blobs ===");
    std::println!("  {vs} vertex blobs, {with_varyings} with varyings");
    for (s, n) in &shapes {
        std::println!("  {n:5}  {s}");
    }
}

/// How the reference compiler encodes an immediate.
///
/// The limits survey says an immediate operand is the biggest single reason
/// the builder refuses a source it was given (44 of the 106 the allocator
/// accepts). The USC does not put one in a source section: it carries it
/// inside the instruction, in an `imm16` or `imm32` field that only some
/// variants have. This measures WHICH variants, for WHICH operations, so the
/// support is written against what the machine is shown to do rather than
/// against the name of a field.
#[test]
fn which_instruction_variants_carry_an_immediate_and_for_which_operations() {
    use super::isa::{bit_set, ops_for};
    let mut by_variant: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut widths: BTreeMap<&str, usize> = BTreeMap::new();
    let mut values: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    let mut groups_with = 0usize;
    let mut groups_total = 0usize;

    for (i, _blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups_total += 1;
            let mut had = false;
            for (_, phase, ir) in g.instrs() {
                let set = bit_set(ir.set);
                let v = &set.variants[ir.variant as usize];
                for f in ["imm16", "imm32", "imm_addr"] {
                    let Some(val) = ir.get(f) else { continue };
                    had = true;
                    let op = ops_for(ir, phase).next().map(|o| o.name).unwrap_or("<unknown>");
                    *by_variant.entry((op, v.name)).or_default() += 1;
                    *widths.entry(f).or_default() += 1;
                    let e = values.entry(f).or_insert((u32::MAX, 0));
                    e.0 = e.0.min(val);
                    e.1 = e.1.max(val);
                }
            }
            if had {
                groups_with += 1;
            }
            off += g.total;
        }
    }

    std::println!("=== how the reference compiler carries an immediate ===");
    std::println!("  {groups_with} of {groups_total} groups carry one");
    std::println!("  by field: {widths:?}");
    for (f, (lo, hi)) in &values {
        std::println!("    {f}: values {lo:#x} .. {hi:#x}");
    }
    std::println!("  operation and the variant it uses:");
    for ((op, v), n) in &by_variant {
        std::println!("    {n:6}  {op:<24} {v}");
    }
    assert!(groups_with > 0, "no group in the table carries an immediate, so this measured nothing");
}

/// `uvsw.write`'s `imm_addr` is the destination dword offset.
///
/// The epilogue was built on the opposite reading -- that the write carries
/// no destination and an implicit pointer advances by the repeat count -- and
/// gate 1 could not contradict it, because that gate hands pco's own field
/// values back to the builder, so a field the HEAD-BUILT path never sets is
/// still correct there. Gate 2 could not either: it compares which operations
/// appear, not what their fields say.
///
/// The test is not "does the offset match a running count" -- it does not,
/// because the reference writes the varyings first and `gl_Position` last.
/// The property that makes it an ADDRESS is that the `(imm_addr, rpt)` spans
/// of one shader never OVERLAP.
///
/// "Tile it exactly" was the first version of this assertion and it was too
/// strong: 198 of 204 shaders do tile, and the six that do not are the
/// `qtparticleshaders` family, which leaves dword 10 or 11 unwritten --
/// `[(0,4), (4,2), (6,4), (10,1), (12,1)]`. A gap is a component the shader
/// simply does not write, which an address permits and a running pointer
/// could not produce. So the gap is evidence FOR the reading, and the
/// assertion is the one the evidence supports.
#[test]
fn uvsw_writes_immediate_address_never_overlaps_within_a_shader() {
    use super::isa::ops_for;
    let (mut tiled, mut not_tiled) = (0usize, std::vec::Vec::new());

    for (i, blob) in SHC_BLOB_TABLE.iter().enumerate() {
        let bytes = usc_bytes(i);
        let mut spans: std::vec::Vec<(u32, u32)> = std::vec::Vec::new();
        let mut off = 0usize;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            for (_, phase, ir) in g.instrs() {
                let Some(op) = ops_for(ir, phase).next() else { continue };
                if op.name.starts_with("uvsw.write") {
                    spans.push((ir.get("imm_addr").unwrap_or(u32::MAX), g.hdr.get("rpt").unwrap_or(1)));
                }
            }
            off += g.total;
        }
        if spans.is_empty() {
            continue;
        }
        spans.sort();
        let mut end = 0u32;
        let ok = spans.iter().all(|(at, n)| {
            let clear = *at >= end;
            end = at + n;
            clear
        });
        if ok {
            tiled += 1;
        } else if not_tiled.len() < 6 {
            not_tiled.push(std::format!("{}: {spans:?}", blob.source_name()));
        }
    }

    std::println!("=== do a shader's output writes ever overlap? ===");
    std::println!("  shaders whose writes are disjoint and ascending: {tiled}");
    std::println!("  shaders with an overlap: {}", not_tiled.len());
    for d in &not_tiled {
        std::println!("    {d}");
    }
    assert!(tiled > 0, "no shader was disjoint, so imm_addr is not an offset and the epilogue needs another reading");
    assert!(not_tiled.is_empty(), "a shader writes the same output dword twice, which an address would not do");
}

/// Every register file `prepare::attested_banks` lets an operation read in a
/// slot is one some single-operation group of that operation reads there in
/// the corpus -- and, the other direction, the table names every file the
/// corpus shows for the operations it covers except `idx0` (indexed
/// addressing, which selection does not produce) and `coeff` (no vertex
/// shader reads it). One entry has no corpus group: `mbyp` from `coeff`,
/// which pco_shc build-new emits for a flat varying (`fragment::flat_group`)
/// and the corpus, compiled before any flat varying, never shows.
///
/// The table was written from a scratch count (usc-backend, 2026-09-23); this
/// is that count, kept.
#[test]
fn every_attested_bank_occurs_in_the_corpus() {
    use super::isa::ops_for;
    use super::operands::srcs;
    use super::prepare::attested_banks;
    use super::sel::Bank;
    let ops = ["fadd", "fmul", "fmad", "frcp", "frsq", "fexp", "flog", "mbyp", "fdsx", "fdsy"];
    let mut seen: BTreeMap<(&str, usize, &str), usize> = BTreeMap::new();
    let mut groups = 0usize;
    let regbank = super::isa_tables::ENUMS.iter().find(|e| e.name == "regbank").expect("no regbank enum");
    for i in 0..SHC_BLOB_TABLE.len() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let ins: Vec<_> = g.instrs().collect();
            if ins.len() == 1 {
                let (_, ph, r) = ins[0];
                if let Some(op) = ops_for(r, ph).next().map(|o| o.name).filter(|n| ops.contains(n)) {
                    for (slot, o) in g.lower.as_ref().map(srcs).unwrap_or([None; 6]).iter().enumerate() {
                        if let Some(o) = o {
                            let bank = regbank.name_of(o.bank).unwrap_or("?");
                            *seen.entry((op, slot, bank)).or_default() += 1;
                        }
                    }
                }
            }
            off += g.total;
        }
    }
    // 651_369 + 399 (the 20 matattr rows) + 7_266 (the 48 offarm rows, gl_test 193)
    // + 26_530 (the 160 offarm rows, gl_test 194)
    assert_eq!(groups, 685_564, "the corpus changed size");
    let all = [Bank::Temp, Bank::Special, Bank::VtxIn, Bank::Coeff, Bank::Shared];
    for op in ops {
        for slot in 0..3 {
            for b in all {
                let n = seen.get(&(op, slot, b.isa_name())).copied().unwrap_or(0);
                let claimed = attested_banks(op, slot).contains(&b);
                if (op, slot, b) == ("mbyp", 0, Bank::Coeff) {
                    assert!(claimed && n == 0, "mbyp from coeff: now in the corpus ({n}); drop the exception");
                    continue;
                }
                assert_eq!(claimed, n > 0, "{op} slot {slot} {}: table says {claimed}, corpus has {n}", b.isa_name());
            }
        }
    }
    /* A known positive for the count itself: the uniform moves. */
    assert!(seen.get(&("mbyp", 0, "shared")).copied().unwrap_or(0) > 30_000);
}

/// `prepare::attested_mods` is exactly the modifier sets each operation
/// carries in the corpus's single-operation groups (a saturated group
/// excluded: the builder never saturates), in both directions.
#[test]
fn every_attested_modifier_set_occurs_in_the_corpus() {
    use super::instr::{ABS, FLR, NEG};
    use super::isa::ops_for;
    use super::prepare::{attested_mods, mods_mask};
    use super::sel::Mods;
    use std::collections::BTreeSet;
    let ops = ["fadd", "fmul", "fmad", "frcp", "frsq", "fexp", "flog", "mbyp", "fdsx", "fdsy"];
    let mut seen: BTreeSet<(&str, u16)> = BTreeSet::new();
    let mut groups = 0usize;
    for i in 0..SHC_BLOB_TABLE.len() {
        let bytes = usc_bytes(i);
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            groups += 1;
            let ins: Vec<_> = g.instrs().collect();
            if ins.len() == 1 {
                let (_, ph, r) = ins[0];
                if let Some(op) = ops_for(r, ph).next().map(|o| o.name).filter(|n| ops.contains(n)) {
                    if r.get("sat").unwrap_or(0) == 0 {
                        let f = |n: &str| r.get(n).unwrap_or(0) != 0;
                        let m = [0, 1, 2].map(|s| Mods { neg: f(NEG[s]), abs: f(ABS[s]), flr: f(FLR[s]) });
                        seen.insert((op, mods_mask(&m)));
                    }
                }
            }
            off += g.total;
        }
    }
    // 651_369 + 399 (the 20 matattr rows) + 7_266 (the 48 offarm rows, gl_test 193)
    // + 26_530 (the 160 offarm rows, gl_test 194)
    assert_eq!(groups, 685_564, "the corpus changed size");
    for op in ops {
        let corpus: BTreeSet<u16> = seen.iter().filter(|(o, _)| *o == op).map(|(_, m)| *m).collect();
        let table: BTreeSet<u16> = attested_mods(op).iter().copied().collect();
        assert_eq!(table, corpus, "{op}: the table's modifier sets and the corpus's differ");
    }
    assert!(seen.contains(&("fadd", 1)), "the known positive: fadd with s0neg, 26,942 groups");
}
