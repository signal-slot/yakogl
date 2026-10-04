// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! M1's gate for the GC7000: the whole offline blob table through the decoder
//! and back.
//!
//! Host tests over the Vivante table, so they are compiled only when that is
//! the table in the build (`host-test,gpu-vivante,shaderc-online`).

#![cfg(all(test, feature = "gpu-vivante"))]

use super::decode::{decode_instr, DecodeError};
use super::encode::encode_instr;
use super::isa::{coverage, MAX_FIELDS};
use super::isa_tables::OPS;
use crate::shaderc::table::SHC_BLOB_TABLE;
use crate::vivante::clear_shader::{GC_CLEAR_PS, GC_CLEAR_VS};
use std::collections::BTreeMap;

/// The corpus: every row of the offline table, plus the two internal shaders
/// the backend clears with. The clear pair is not in the table but is code
/// this driver runs on silicon, and leaving it out would mean the gate
/// covered less than the driver ships.
fn corpus() -> Vec<(String, Vec<u32>)> {
    let mut out: Vec<(String, Vec<u32>)> =
        (0..SHC_BLOB_TABLE.len()).map(|i| (format!("{} [{i}]", SHC_BLOB_TABLE[i].source_name()), code_of(i))).collect();
    for (name, sh) in [("clear.vert", &GC_CLEAR_VS.0), ("clear.frag", &GC_CLEAR_PS.0)] {
        // SAFETY: `usc` points at a `[u32; usc_dw]` static of clear_shader.rs.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        out.push((name.to_string(), dw.to_vec()));
    }
    out
}

/// The instruction words of one table row.
fn code_of(i: usize) -> Vec<u32> {
    let b = &SHC_BLOB_TABLE[i];
    // SAFETY: `usc` points at a `[u32; usc_dw]` static of the generated table
    // (vivante_blobs.rs), which outlives the test.
    let dw = unsafe { core::slice::from_raw_parts(b.shader.usc, b.shader.usc_dw as usize) };
    dw.to_vec()
}

/// M1's acceptance for this GPU: decode every blob into instructions and
/// write them back exactly.
///
/// The encoder starts from zero and writes only the operation's pinned bits
/// and its fields, so a bit the tables do not account for comes back zero and
/// the comparison fails. That is what makes this a test of the tables rather
/// than of the blobs.
#[test]
fn round_trip_every_blob_in_the_offline_table() {
    let mut blobs_ok = 0usize;
    let mut instrs = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let corpus = corpus();
    let total = corpus.len();

    for (name, code) in &corpus {
        if !code.len().is_multiple_of(4) {
            failures.push(format!("{name}: {} dwords is not whole instructions", code.len()));
            continue;
        }
        let mut ok = true;
        for (n, dw) in code.as_chunks::<4>().0.iter().enumerate() {
            let quad = [dw[0], dw[1], dw[2], dw[3]];
            let instr = match decode_instr(&quad) {
                Ok(x) => x,
                Err(e) => {
                    failures.push(format!("{name} instr {n}: {e:?} {quad:08x?}"));
                    ok = false;
                    break;
                }
            };
            let back = encode_instr(&instr);
            if back != quad {
                failures.push(format!("{name} instr {n} ({}): {back:08x?} != {quad:08x?}", instr.op().name));
                ok = false;
                break;
            }
            instrs += 1;
        }
        if ok {
            blobs_ok += 1;
        }
    }

    if !failures.is_empty() {
        let shown: Vec<&str> = failures.iter().take(10).map(|s| s.as_str()).collect();
        panic!(
            "{blobs_ok} of {total} shaders round-tripped ({instrs} instructions); {} failures, first {}:\n{}",
            failures.len(),
            shown.len(),
            shown.join("\n")
        );
    }
    assert_eq!(blobs_ok, total);
    // A second, independent statement about the same run: a decoder that gave
    // up after each blob's first instruction would still report every blob as
    // round-tripped.
    assert!(instrs > 10_000, "only {instrs} instructions decoded; the walk stopped early");
    eprintln!("gc7000 round trip: {blobs_ok}/{total} shaders, {instrs} instructions");
}

/// A decoder that guesses is worse than one that fails, so it refuses an
/// instruction that two operations claim. Show the corpus has none.
#[test]
fn no_instruction_in_the_corpus_decodes_two_ways() {
    let mut n = 0;
    for (name, code) in &corpus() {
        for dw in code.as_chunks::<4>().0.iter() {
            let quad = [dw[0], dw[1], dw[2], dw[3]];
            match decode_instr(&quad) {
                Ok(_) => n += 1,
                Err(DecodeError::Ambiguous) => panic!("{name}: ambiguous instruction {quad:08x?}"),
                Err(e) => panic!("{name}: {e:?} {quad:08x?}"),
            }
        }
    }
    assert!(n > 10_000);
}

/// No instruction in the corpus has a don't-care bit set.
///
/// Seven of the ninety operations declare bits the description marks `x`
/// (bit 34 of the five branch forms and of `call`, and nineteen bits of
/// `store` and `img_store`). They belong to no field, so nothing names their
/// value; the decoder carries them verbatim so the round trip stays exact
/// whatever they hold. What this test records is what they actually hold in
/// the table today, which is zero everywhere - so the day a shader arrives
/// with one of them set, it is a failure with a name rather than a silent
/// change of behaviour in the code generator.
#[test]
fn no_dont_care_bit_is_set_in_the_corpus() {
    let mut seen = 0usize;
    for (name, code) in &corpus() {
        for dw in code.as_chunks::<4>().0.iter() {
            let instr = decode_instr(&[dw[0], dw[1], dw[2], dw[3]]).expect("decodes");
            if instr.dont_care != 0 {
                panic!("{name}: {} has don't-care bits set: {:#x}", instr.op().name, instr.dont_care);
            }
            if instr.op().dont_care != 0 {
                seen += 1;
            }
        }
    }
    // The claim is only worth something if the corpus reaches an operation
    // that has such bits at all.
    assert!(seen > 0, "no instruction in the corpus belongs to an operation with don't-care bits");
    eprintln!("gc7000 don't-care bits: {seen} instructions could have had one set; none did");
}

/// Every operation's pinned bits, don't-care bits and fields together must
/// cover all 128 bits.
/// This is the property the round trip depends on, checked on the tables
/// directly so that an operation the corpus never uses cannot hide a hole
/// until the code generator emits it.
#[test]
fn every_operation_covers_all_128_bits() {
    let mut holes = Vec::new();
    for op in OPS {
        let c = coverage(op);
        if c != u128::MAX {
            holes.push(format!("{}: {} bits unaccounted for", op.name, (!c).count_ones()));
        }
        assert!(op.fields.len() <= MAX_FIELDS, "{} has {} fields, more than MAX_FIELDS", op.name, op.fields.len());
    }
    assert!(holes.is_empty(), "{holes:?}");
    eprintln!("gc7000 tables: {} operations, all 128 bits accounted for in each", OPS.len());
}

/// What the corpus uses, counted: the code generator's target list.
#[test]
fn isa_coverage() {
    let mut used: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut n = 0;
    let corpus = corpus();
    for (_, code) in &corpus {
        for dw in code.as_chunks::<4>().0.iter() {
            let instr = decode_instr(&[dw[0], dw[1], dw[2], dw[3]]).expect("decodes");
            *used.entry(instr.op().name).or_default() += 1;
            n += 1;
        }
    }
    let mut rows: Vec<(&&str, &usize)> = used.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if std::env::var("YAKOGL_WRITE_ISA_COVERAGE").as_deref() == Ok("1") {
        let mut doc = String::new();
        doc.push_str("# What the GC7000 offline table's shaders actually use\n\n");
        doc.push_str(
            "Counted by `cargo test --no-default-features \
             --features host-test,gpu-vivante,shaderc-online isa_coverage`, which \
             regenerates this file with `YAKOGL_WRITE_ISA_COVERAGE=1`.\nIt is the code \
             generator's target list: an operation that is not here is one nothing in \
             the table needs, and one here with a large count is one the generator has \
             to get right first.\n\n",
        );
        doc.push_str(&format!(
            "Corpus: the GC7000 `SHC_BLOB_TABLE` plus the backend's two internal clear \
             shaders, {} shaders and {n} instructions; {} of the description's {} \
             operations appear.\n\n| operation | count |\n|---|---:|\n",
            corpus.len(),
            used.len(),
            OPS.len()
        ));
        for (k, v) in &rows {
            doc.push_str(&format!("| `{k}` | {v} |\n"));
        }
        std::fs::write("docs/online-shaderc/gc7000-isa-coverage.md", doc).expect("write coverage");
    }
    eprintln!(
        "gc7000 coverage: {n} instructions, {} of {} operations used: {}",
        used.len(),
        OPS.len(),
        rows.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
    );
    assert!(!used.is_empty());
}

/// The builder's gate: an instruction made from an operation and operands is
/// the same instruction the machine already runs.
///
/// Three claims, weakest to strongest.
mod build_tests {
    use super::corpus;
    use crate::shaderc::online::vivante::build::{
        Builder, Dst, Src, RGROUP_TEMP, RGROUP_UNIFORM_0, SWIZZLE_IDENTITY, WRITE_MASK_ALL,
    };
    use crate::shaderc::online::vivante::decode::decode_instr;
    use crate::shaderc::online::vivante::encode::encode_instr;

    /// The bring-up triangle's shaders (driver/src/vivante/gc_shaders.h) are a
    /// single NOP each, because the hardware loads attributes straight into
    /// the shader's temporaries and reads the outputs back out of them, so a
    /// pass-through needs no arithmetic. Build that NOP.
    ///
    /// It is a weak test on its own - four zero dwords - and it is here
    /// because it is the one whose *meaning* is independently known: those
    /// exact words are what the driver has drawn a triangle with on silicon.
    #[test]
    fn the_bring_up_triangles_shader_builds_to_the_sample_words() {
        let gc_tri_vs_code: [u32; 4] = [0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000];
        let nop = Builder::new("nop").build().expect("nop exists");
        assert_eq!(encode_instr(&nop), gc_tri_vs_code);
    }

    /// An operation an instruction does not have is an error, not a silently
    /// dropped operand. Without this, a code generator that names a field
    /// wrongly emits a subtly different shader and nothing says so.
    #[test]
    fn a_field_the_operation_does_not_have_is_an_error() {
        // `nop` has no sources at all.
        assert!(Builder::new("nop").src(0, Src::reg(RGROUP_TEMP, 1)).build().is_err());
        assert!(Builder::new("not an operation").build().is_err());
        // A value that does not fit its field is refused rather than truncated.
        assert!(Builder::new("mov").set("SRC2_REG", 1 << 20).build().is_err());
    }

    /// The real one: take every instruction the driver ships, read its
    /// operands back out, build a fresh instruction from those operands
    /// through the builder alone, and require the bits to be identical.
    ///
    /// This is what says the operand model covers what real shaders contain.
    /// A round trip through `decode`/`encode` only shows the fields are
    /// preserved; this shows they can be *stated* - that a caller who knows
    /// only "this operation, this destination, these sources, this type" can
    /// reach the same instruction, which is exactly what a code generator
    /// knows and nothing more.
    #[test]
    fn every_instruction_in_the_corpus_can_be_rebuilt_from_its_operands() {
        let mut rebuilt = 0usize;
        let mut skipped = 0usize;
        let mut failures: Vec<String> = Vec::new();

        for (name, code) in &corpus() {
            for (n, dw) in code.as_chunks::<4>().0.iter().enumerate() {
                let quad = [dw[0], dw[1], dw[2], dw[3]];
                let want = decode_instr(&quad).expect("decodes");
                let op = want.op();

                // Operations whose fields are not all operands (a branch
                // target, a texture id, the sub-opcode of an extended form)
                // are outside the operand model; the generic `set` covers
                // them and they are counted rather than quietly passed.
                let operand_fields = ["DST", "DST_USE", "TYPE_BIT01", "TYPE_BIT2"];
                let src_fields = [
                    "SRC0",
                    "SRC0_REG",
                    "SRC0_RGROUP",
                    "SRC0_AMODE",
                    "SRC1",
                    "SRC1_REG",
                    "SRC1_RGROUP",
                    "SRC1_AMODE",
                    "SRC2",
                    "SRC2_REG",
                    "SRC2_RGROUP",
                    "SRC2_AMODE",
                ];
                let mut b = Builder::new(op.name);
                let mut covered = true;
                for (i, f) in op.fields.iter().enumerate() {
                    if operand_fields.contains(&f.name) || src_fields.contains(&f.name) {
                        continue;
                    }
                    // Everything else is set verbatim: the builder's escape
                    // hatch is part of the contract, not a workaround.
                    if want.vals[i] != 0 {
                        b = b.set(f.name, want.vals[i]);
                    }
                }
                if want.get("DST").is_some() {
                    b = b.dst(want.dst().unwrap()).set("DST_USE", want.get("DST_USE").unwrap());
                }
                for s in 0..3 {
                    if let Some(src) = want.src(s) {
                        b = b.src(s, src);
                    }
                }
                if want.get("TYPE_BIT01").is_some() {
                    let t = (want.get("TYPE_BIT2").unwrap_or(0) << 2) | want.get("TYPE_BIT01").unwrap();
                    b = b.ty(t as u8);
                }
                let mut got = match b.build() {
                    Ok(g) => g,
                    Err(e) => {
                        failures.push(format!("{name} instr {n} ({}): {e:?}", op.name));
                        skipped += 1;
                        continue;
                    }
                };
                // The don't-care bits are carried, not stated: nothing names
                // them, so a builder cannot be asked for them.
                got.dont_care = want.dont_care;
                if encode_instr(&got) != quad {
                    failures.push(format!(
                        "{name} instr {n} ({}): {:08x?} != {quad:08x?}",
                        op.name,
                        encode_instr(&got)
                    ));
                    covered = false;
                }
                if covered {
                    rebuilt += 1;
                } else {
                    skipped += 1;
                }
            }
        }

        if !failures.is_empty() {
            let shown: Vec<&str> = failures.iter().take(10).map(|s| s.as_str()).collect();
            panic!("{rebuilt} rebuilt, {skipped} failed; first {}:\n{}", shown.len(), shown.join("\n"));
        }
        assert!(rebuilt > 10_000, "only {rebuilt} instructions rebuilt; the walk stopped early");
        eprintln!("gc7000 builder: {rebuilt} instructions rebuilt from their operands, bit for bit");
    }

    /// A worked example, so the API is readable rather than only correct:
    /// `mul r1.xyzw, u2.xyzw, r3.xyzw` built from nothing.
    #[test]
    fn a_multiply_reads_as_a_multiply() {
        let m = Builder::new("mul")
            .dst(Dst::reg(1, WRITE_MASK_ALL))
            .src(0, Src::reg(RGROUP_UNIFORM_0, 2))
            .src(1, Src::reg(RGROUP_TEMP, 3))
            .ty(0) // f32
            .build()
            .expect("mul builds");
        let back = decode_instr(&encode_instr(&m)).expect("decodes");
        assert_eq!(back.op().name, "mul");
        assert_eq!(back.dst().unwrap(), Dst { reg: 1, write_mask: WRITE_MASK_ALL, amode: 0 });
        let s0 = back.src(0).unwrap();
        assert_eq!((s0.group, s0.reg, s0.swizzle), (RGROUP_UNIFORM_0, 2, SWIZZLE_IDENTITY));
        let s1 = back.src(1).unwrap();
        assert_eq!((s1.group, s1.reg), (RGROUP_TEMP, 3));
    }
}

/// The bitset name is the key and the mnemonic is the spelling, and they are
/// not the same thing.
///
/// isaspec's `displayname` makes `branch_unary` and `branch_binary` both
/// print as `branch`: one instruction with different operand counts. A
/// listing that prints the bitset name disagrees with Mesa's disassembler on
/// exactly the `branch_unary` occurrences of this corpus, which is how
/// pvrgpu-platform's cross-check found it. The opposite mistake is just as
/// easy: `bit_findlsb` and `bit_findmsb` share an opcode and carry no
/// `displayname`, so they are genuinely two mnemonics and must not be
/// collapsed by a rule about opcodes.
#[test]
fn the_mnemonic_is_what_is_printed_and_the_name_is_what_is_keyed() {
    // Keys are unique; spellings need not be.
    for (i, op) in OPS.iter().enumerate() {
        assert!(OPS.iter().position(|o| o.name == op.name) == Some(i), "two operations share the key '{}'", op.name);
        assert!(!op.mnemonic.is_empty(), "{} has no mnemonic", op.name);
    }
    let spelling = |name: &str| OPS.iter().find(|o| o.name == name).map(|o| o.mnemonic);
    assert_eq!(spelling("branch_unary"), Some("branch"));
    assert_eq!(spelling("branch_binary"), Some("branch"));
    assert_eq!(spelling("texkill_unary"), Some("texkill"));
    // ... and the pair that must NOT be collapsed, though they share an opcode.
    assert_eq!(spelling("bit_findlsb"), Some("bit_findlsb"));
    assert_eq!(spelling("bit_findmsb"), Some("bit_findmsb"));

    // The claim is only worth something if the corpus reaches one of them.
    let mut branch_unary = 0usize;
    for (_, code) in &corpus() {
        for dw in code.as_chunks::<4>().0.iter() {
            let instr = decode_instr(&[dw[0], dw[1], dw[2], dw[3]]).expect("decodes");
            if instr.op().name == "branch_unary" {
                branch_unary += 1;
            }
        }
    }
    assert!(branch_unary > 0, "no branch_unary in the corpus; this test proves nothing");
    eprintln!("gc7000 mnemonics: {branch_unary} branch_unary occurrences print as `branch`");
}

/// The builder's defaults, audited against the corpus.
///
/// The round trip above proves the encoder faithful for values that EXIST: it
/// decodes an instruction and re-encodes it, so it carries whatever the corpus
/// had through both directions. It can never construct a default and ask what
/// that produces, which is how `PMODE` sat at 0 for as long as it did while
/// every shipped instruction carries 1. This module proves the other half: the
/// values the builder INVENTS.
#[cfg(test)]
mod builder_default_audit {
    use super::*;
    use crate::shaderc::online::vivante::build::{op_id, Builder};
    use crate::shaderc::online::vivante::decode;
    use std::collections::BTreeMap;

    /// What a field is allowed to look like across the corpus.
    enum Expect {
        /// Every instruction carrying the field has this value.
        Invariant(u32),
        /// This value dominates; the others are named and counted, because an
        /// exception nobody can name is an unexplained result, not a default.
        MajorityWith(u32, &'static str, usize),
    }

    /// Fields no operand sets, so the builder's zero-or-otherwise decides
    /// them. `SAT`, `TYPE_*`, `DST*`, `SRC*`, `COND`, `TEX_*` and `TARGET` are
    /// absent on purpose: a caller sets each of those.
    const DEFAULTS: &[(&str, u32, Expect)] = &[
        /* `nop` is the one exception and it is a real case, not noise: its
         * whole encoding is zero, so its PMODE is too. Excluding the nops
         * would turn this into a clean invariant and hide the fact that the
         * builder has to special-case them -- which is how the rebuild test
         * caught this the moment the default moved. */
        ("PMODE", 1, Expect::MajorityWith(1, "`nop`, whose whole encoding is zero", 16)),
        ("SKPHP", 0, Expect::Invariant(0)),
        ("UNK", 0, Expect::Invariant(0)),
        ("DST_FULL", 0, Expect::Invariant(0)),
        ("LOW_HALF", 0, Expect::Invariant(0)),
        ("HIGH_HALF", 0, Expect::Invariant(0)),
        ("RMODE", 0, Expect::MajorityWith(0, "the transcendentals, which Mesa rounds toward zero", 83)),
    ];

    /// Value counts per field over EVERY corpus instruction, nops included.
    ///
    /// Nothing is excluded. An earlier version dropped the all-zero nops, and
    /// that exclusion was doing real work: it turned PMODE into a clean
    /// invariant and hid the fact that `nop` genuinely carries 0. The nops are
    /// counted and returned so a reader can see there are sixteen of them.
    fn distributions() -> (BTreeMap<&'static str, BTreeMap<u32, usize>>, usize) {
        let mut out: BTreeMap<&'static str, BTreeMap<u32, usize>> = BTreeMap::new();
        let mut nops = 0usize;
        for (_, code) in corpus() {
            for (quad, _) in code.as_chunks::<4>().0.iter().zip(0..) {
                let q: [u32; 4] = *quad;
                if q == [0, 0, 0, 0] {
                    nops += 1;
                }
                let Ok(i) = decode::decode_instr(&q) else { continue };
                for f in i.op().fields.iter() {
                    if let Some(v) = i.get(f.name) {
                        *out.entry(f.name).or_default().entry(v).or_insert(0) += 1;
                    }
                }
            }
        }
        (out, nops)
    }

    #[test]
    fn every_builder_default_matches_the_corpus() {
        let (dist, nops) = distributions();
        assert_eq!(nops, 16, "the corpus's nop count is what PMODE's exception count must match");
        let mut wrong: Vec<String> = Vec::new();

        for (name, default, expect) in DEFAULTS {
            let Some(d) = dist.get(name) else {
                wrong.push(format!("{name}: no operation in the corpus carries it (unmeasured)"));
                continue;
            };
            let total: usize = d.values().sum();
            match expect {
                Expect::Invariant(v) => {
                    if d.len() != 1 || !d.contains_key(v) {
                        wrong.push(format!("{name}: expected {v} in all {total}, corpus has {d:?}"));
                    } else if default != v {
                        wrong.push(format!("{name}: corpus is always {v}, the builder defaults to {default}"));
                    }
                }
                Expect::MajorityWith(v, why, n) => {
                    let others: usize = d.iter().filter(|(k, _)| *k != v).map(|(_, c)| c).sum();
                    if others != *n {
                        wrong.push(format!("{name}: {others} exceptions, {n} expected ({why}); corpus {d:?}"));
                    } else if default != v {
                        wrong.push(format!("{name}: majority {v}, the builder defaults to {default}"));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} default(s) disagree with the corpus:\n  {}", wrong.len(), wrong.join("\n  "));
    }

    /// The audit above says what the corpus holds; this says the builder
    /// actually produces it. Without this the table could be right and the
    /// constructor still wrong -- which was the whole defect.
    #[test]
    fn a_freshly_built_instruction_carries_those_defaults() {
        /* `nop` is checked separately below, since its PMODE is the one
         * documented exception. */
        for op in ["mov", "mul", "add", "mad", "dp3"] {
            let id = op_id(op).unwrap_or_else(|| panic!("the ISA tables carry {op}"));
            let b = Builder::of(id).build().unwrap_or_else(|e| panic!("{op} builds: {e:?}"));
            for (name, default, _) in DEFAULTS {
                if !id.has_field(name) {
                    continue;
                }
                assert_eq!(b.get(name), Some(*default), "{op}: {name} default");
            }
        }
        let nop = op_id("nop").expect("the ISA tables carry nop");
        let b = Builder::of(nop).build().expect("nop builds");
        assert_eq!(b.get("PMODE"), Some(0), "a nop's whole encoding is zero, PMODE included");
    }
}

/// No row of the GC7000 table reads a uniform array at a run-time index
/// through the address register (`movar` then a relative constant-file read):
/// on the i.MX8MP that read returns element 0 whatever the index (the online
/// compiler's LoadUniformIdx note, select.rs; gl_test 45 es3dyn alone read
/// 3 x cols[0], record 594b010e 002d0102 bf990000 bf9900e6, 2026-10-04).
/// tools/vivante/etna_shc.c now lowers uniforms to UBO 0 as Mesa's GL state
/// tracker does, so such an index is a `load` from the stage's uniform block
/// (`SHC_VIV_CONST_UBO_ADDR`). Two Qt particle rows are still the old shape:
/// their `ubuf.sizetable` is packed in the GL core's block (stride 1) where
/// Mesa's offsets assume a vec4 per element, so gen_shader_table refuses to
/// recompile them; they are named here, an open item, not a pass.
#[test]
fn no_table_row_indexes_a_uniform_array_through_the_address_register() {
    const STILL_MOVAR: [&str; 2] = ["qt/imageparticle_sprite.vert", "qt/imageparticle_tabled.vert"];
    let mut with: Vec<String> = (0..SHC_BLOB_TABLE.len())
        .filter(|&i| {
            code_of(i).as_chunks::<4>().0.iter().any(|q| decode_instr(q).is_ok_and(|instr| instr.op().name == "movar"))
        })
        .map(|i| SHC_BLOB_TABLE[i].source_name().to_string())
        .collect();
    with.sort();
    assert_eq!(with, STILL_MOVAR, "rows that read a uniform array through movar");

    /* and es300/dynidx.frag, gl_test 45's array, reads it with `load` from
     * the block whose address the backend writes */
    let i = (0..SHC_BLOB_TABLE.len())
        .find(|&i| SHC_BLOB_TABLE[i].source_name() == "es300/dynidx.frag")
        .expect("es300/dynidx.frag is a row");
    let loads = code_of(i)
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|q| decode_instr(q).is_ok_and(|instr| instr.op().name == "load"))
        .count();
    assert!(loads > 0, "dynidx.frag reads cols[k] with a load");
    assert!(
        SHC_BLOB_TABLE[i].shader.viv.const_contents().contains(&crate::shaderc::SHC_VIV_CONST_UBO_ADDR),
        "and the backend is asked for the block's address"
    );
}
