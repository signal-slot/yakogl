// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The decoder against Mesa beyond the corpus: every combination of every
//! field group, with Mesa's own verdict as the reference.
//!
//! The corpus only proves the words Mesa's compiler happens to emit. The
//! encoder can produce any word the decoder accepts (it decodes every word it
//! writes and refuses one that reads back differently), so "nothing encodes
//! silently wrong" comes down to "the decoder means what Mesa means, on every
//! word it accepts". [`enumeration`] builds, per field group, every value of
//! that group's fields with the rest of the word held fixed:
//!
//! * the add half: opcode x mux_a x mux_b x MA x six waddrs;
//! * the mul half: opcode x mux_a x mux_b x MM x four waddrs;
//! * signal x condition field x eight raddr_b values, with an FADD and an
//!   FMUL that read port B (so small immediates and signal addresses print);
//! * both waddrs 0..63 with and without magic;
//! * branches: SIG's top bits x cond x msfign x bdi x ub x bdu x raddr_a x
//!   five offsets x the unused bits.
//!
//! Each half of the ALU decode reads only its own fields plus the signal (for
//! the small immediate) and the flags field, which is why varying the groups
//! one at a time covers the decoder's inputs. That independence is read from
//! Mesa's qpu_pack.c (v3d42_qpu_add_unpack, v3d42_qpu_mul_unpack,
//! v3d_qpu_instr_unpack_alu), and this test assumes it.
//!
//! Since the decoder became Mesa's unpack (qpu_ffi), what this still tests
//! is what is ours: which words the wrapper refuses on top of Mesa (the
//! strict rules, the model's reach), and the listing (listing.rs, our own
//! printer) of every word it accepts, against Mesa's `v3d_qpu_disasm`. The
//! pinned digest is the one of the hand-transcribed decoder (a87ed25c):
//! the same digest from the wrapper is the evidence that the switch to
//! Mesa's packer accepts, refuses and prints exactly what the transcription
//! did.
//!
//! `tools/v3d/qpu_ref.c` gives Mesa's answer per word (unpack, re-pack,
//! text). [`expected_line`] turns it into the line our decoder must produce,
//! under a short named rule for the words we refuse on purpose. The ignored
//! test compares word by word and prints the digest of Mesa's lines; the gate
//! test pins that digest and recomputes it from our decoder alone, so the
//! gate carries the whole comparison without a fixture.

use super::decode::decode;
use super::encode::{encode, EncodeError};
use super::fields::*;
use super::golden::text_of;
use super::isa::*;
use super::qpu_ffi;
use super::roundtrip::is_ftoi_unpack;

/// FNV-1a over the lines, so the gate can pin Mesa's answer as one number.
fn fnv(lines: &[String]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for l in lines {
        for b in l.bytes().chain(core::iter::once(b'\n')) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// The words, group by group (see the module comment).
fn enumeration() -> Vec<u64> {
    let mut out = Vec::new();
    let mut nop = encode(&Instr::Alu(Alu::nop())).expect("nop");
    RADDR_A.set(&mut nop, 5);
    RADDR_B.set(&mut nop, 9);

    // The add half.
    for op in 0..256 {
        for ma in 0..2 {
            for a in 0..8 {
                for b in 0..8 {
                    for waddr in [0, 1, 2, 3, 6, 35] {
                        let mut w = nop;
                        OP_ADD.set(&mut w, op);
                        MA.set(&mut w, ma);
                        ADD_A.set(&mut w, a);
                        ADD_B.set(&mut w, b);
                        WADDR_A.set(&mut w, waddr);
                        out.push(w);
                    }
                }
            }
        }
    }
    // The mul half (opcode 0 is not an ALU word, so the ALU/branch split is
    // tested too).
    for op in 0..64 {
        for mm in 0..2 {
            for a in 0..8 {
                for b in 0..8 {
                    for waddr in [0, 6, 14, 35] {
                        let mut w = nop;
                        OP_MUL.set(&mut w, op);
                        MM.set(&mut w, mm);
                        MUL_A.set(&mut w, a);
                        MUL_B.set(&mut w, b);
                        WADDR_M.set(&mut w, waddr);
                        out.push(w);
                    }
                }
            }
        }
    }
    // Signal x flags field x raddr_b, with operands on port B: add "fadd rf1,
    // rf5, <B>" (opcode 5: no pack, NONE unpacks), mul "fmul rf2, <B>, rf5"
    // (opcode 21).
    let mut rb = nop;
    OP_ADD.set(&mut rb, 5);
    ADD_A.set(&mut rb, Mux::A as u32);
    ADD_B.set(&mut rb, Mux::B as u32);
    WADDR_A.set(&mut rb, 1);
    MA.set(&mut rb, 0);
    OP_MUL.set(&mut rb, 21);
    MUL_A.set(&mut rb, Mux::B as u32);
    MUL_B.set(&mut rb, Mux::A as u32);
    WADDR_M.set(&mut rb, 2);
    MM.set(&mut rb, 0);
    for sig in 0..32 {
        for cond in 0..128 {
            for raddr_b in [0, 9, 15, 31, 32, 47, 48, 63] {
                let mut w = rb;
                SIG.set(&mut w, sig);
                COND.set(&mut w, cond);
                RADDR_B.set(&mut w, raddr_b);
                out.push(w);
            }
        }
    }
    // Every write address, register file and magic, on both halves.
    for waddr in 0..64 {
        for (ma, mm) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let mut w = rb;
            WADDR_A.set(&mut w, waddr);
            WADDR_M.set(&mut w, waddr);
            MA.set(&mut w, ma);
            MM.set(&mut w, mm);
            out.push(w);
        }
    }
    // Branches.
    for sig_high in 0..4 {
        for cond in 0..8 {
            for msfign in 0..4 {
                for bdi in 0..4 {
                    for ub in 0..2 {
                        for bdu in 0..8 {
                            for raddr_a in [0, 35] {
                                for offset in [0u32, 0x7316_fe10, 0xffff_fff8, 8, 0x0100_0000] {
                                    for unused in [0u64, 1, 1 << 18, 1 << 23] {
                                        let mut w = unused;
                                        BRANCH_SIG_HIGH.set(&mut w, sig_high);
                                        BRANCH_COND.set(&mut w, cond);
                                        BRANCH_MSFIGN.set(&mut w, msfign);
                                        BRANCH_BDI.set(&mut w, bdi);
                                        BRANCH_UB.set(&mut w, ub);
                                        BRANCH_BDU.set(&mut w, bdu);
                                        RADDR_A.set(&mut w, raddr_a);
                                        BRANCH_ADDR_LOW.set(&mut w, (offset & 0x00ff_ffff) >> 3);
                                        BRANCH_ADDR_HIGH.set(&mut w, offset >> 24);
                                        out.push(w);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Our line for a word: the listing, or `R` when the decoder refuses it.
fn our_line(w: u64) -> String {
    match decode(w) {
        Ok(i) => format!("{w:#018x} {}", text_of(&i)),
        Err(_) => format!("{w:#018x} R"),
    }
}

/// Which rule gave a word's expected line. One variant per rule; the ignored
/// test counts each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Expect {
    /// Text, as Mesa prints it (with Mesa's `vdwwt` read as `iid`).
    Text,
    /// Text, for FTOIN/FTOIZ/FTOUZ/FTOC with an L or H unpack (opcodes
    /// 249/250, 253/254): decoded as Mesa decodes them, even the L words
    /// Mesa's own pack cannot give back (it writes 253/254 for L,
    /// qpu_pack.c:1795). Our encoder refuses all of them
    /// (EncodeError::UnpackUnmeasuredOnSilicon); counted apart.
    TextFtoiUnpackEncoderRefuses,
    /// Text, for Mesa's `vdwwt` (IID on this GPU), counted apart.
    TextVdwwtAsIid,
    /// Mesa's unpack refuses the word.
    MesaRefuses,
    /// Mesa unpacks it but its pack refuses the result.
    MesaCannotPack,
    /// Mesa's pack gives a different word: its model lost bits.
    MesaLossy,
    /// A small immediate past the table: Mesa's disassembler asserts
    /// (qpu_disasm.c:77); we refuse the word even where nothing reads it.
    SmallImmPastTable,
    /// LDVPMP with MA: Mesa's pack asserts !magic_write (qpu_pack.c:1660).
    LdvpmpMagic,
    /// A magic write or magic signal address that Mesa's name table does
    /// not name (qpu_instr.c:43-99); qpu_ref prints `OOB` for the ones past
    /// the table's end, where Mesa's text is undefined.
    MagicWaddrUnnamed,
    /// A uniform branch with bdu 4..7: no such destination; Mesa prints
    /// nothing for it and packs the raw value back.
    BranchBduReserved,
}

/// What our decoder must print for `w`, given Mesa's answer `mesa` (the text
/// after the word in qpu_ref's line).
fn expected_line(w: u64, mesa: &str) -> (String, Expect) {
    let refused = |e| (format!("{w:#018x} R"), e);
    if mesa == "U=0" {
        return refused(Expect::MesaRefuses);
    }
    let rest = mesa.strip_prefix("U=1 K=").expect("qpu_ref line");
    let (k, rest) = rest.split_once(" P=0x").expect("qpu_ref line");
    let (p, text) = rest.split_at(16);
    let text = text.strip_prefix(' ').unwrap_or(text);
    let p = u64::from_str_radix(p, 16).expect("hex");
    let alu = OP_MUL.get(w) != 0;
    if alu && matches!(SIG.get(w), 14 | 15 | 31) && RADDR_B.get(w) >= 48 {
        return refused(Expect::SmallImmPastTable);
    }
    if alu && OP_ADD.get(w) == 188 && ADD_B.get(w) == 2 && MA.get(w) == 1 {
        return refused(Expect::LdvpmpMagic);
    }
    if alu {
        let sig = qpu_ffi::sig_unpack(SIG.get(w)).expect("Mesa unpacked it");
        let unnamed = |waddr: u32, magic: bool| magic && magic_waddr_name(waddr as u8).is_none();
        let cond = COND.get(w);
        if unnamed(WADDR_A.get(w), MA.get(w) == 1 && !ma_is_opcode(w))
            || unnamed(WADDR_M.get(w), MM.get(w) == 1)
            || (sig.writes_address() && unnamed(cond & 63, cond & 64 != 0))
        {
            return refused(Expect::MagicWaddrUnnamed);
        }
    }
    if !alu && BRANCH_UB.get(w) == 1 && BRANCH_BDU.get(w) >= 4 {
        return refused(Expect::BranchBduReserved);
    }
    if k != "1" {
        return refused(Expect::MesaCannotPack);
    }
    // FTOIN/FTOIZ (245, mux_b 3/7) and FTOUZ/FTOC (246, mux_b 3/7) in their
    // L/H words: Mesa's text, whatever its pack gives back.
    let ftoi_unpack = alu && matches!(OP_ADD.get(w), 249 | 250 | 253 | 254) && ADD_B.get(w) & 3 == 3;
    if p != w && !ftoi_unpack {
        return refused(Expect::MesaLossy);
    }
    // Mesa's "vdwwt" is this GPU's IID (see AddOp). The add half is padded
    // to column 30 before "; ", so the shorter name is re-padded.
    let (text, class) = match text.strip_prefix("vdwwt").and_then(|r| r.split_once("; ")) {
        Some((add, mul)) => (format!("{:<30}; {mul}", format!("iid{add}").trim_end()), Expect::TextVdwwtAsIid),
        None if ftoi_unpack => (text.to_string(), Expect::TextFtoiUnpackEncoderRefuses),
        None => (text.to_string(), Expect::Text),
    };
    (format!("{w:#018x} {text}"), class)
}

/// The MA bit of an LDVPM*/STVPM word is part of its opcode, not a magic
/// write (qpu_pack.c:1191-1205): add opcodes 188 (mux_b 0, 1; LDVPMP is
/// mux_b 2 and IS read as a magic write by Mesa, see LdvpmpMagic), 189, 248.
fn ma_is_opcode(w: u64) -> bool {
    match OP_ADD.get(w) {
        188 => ADD_B.get(w) < 2,
        189 | 248 => true,
        _ => false,
    }
}

// Mesa's answer over the enumeration, pinned from the ignored test below
// (qpu_ref built from Mesa 26.2.1 at a87ed25c).
const ENUMERATION_WORDS: usize = 590_080;
const MESA_TEXT_LINES: usize = 239_445;
const MESA_DIGEST: u64 = 0x6459_a414_7eb6_8fdf;
/// Decoded FTOI* L/H words the encoder refuses: 4 operations x 2 unpacks x
/// 96 (MA 2 x mux_a 8 x waddr 6) in the add-half group.
const FTOI_UNPACK_REFUSED_BY_ENCODER: usize = 768;

/// The gate: our decoder over every field combination gives Mesa's pinned
/// answer, and every word it accepts encodes back to itself.
#[test]
fn decoder_matches_mesa_on_every_field_combination() {
    let words = enumeration();
    let lines: Vec<String> = words.iter().map(|&w| our_line(w)).collect();
    let text = lines.iter().filter(|l| !l.ends_with(" R")).count();
    let (mut bad, mut unmeasured) = (Vec::new(), 0usize);
    for &w in &words {
        let Ok(i) = decode(w) else { continue };
        match encode(&i) {
            Ok(b) if b == w => {}
            Err(EncodeError::UnpackUnmeasuredOnSilicon) if is_ftoi_unpack(&i) => unmeasured += 1,
            _ => bad.push(w),
        }
    }
    let digest = fnv(&lines);
    eprintln!(
        "mesa enumeration: {} words, {text} decoded ({unmeasured} FTOI* L/H of them refused by the encoder as \
         UnpackUnmeasuredOnSilicon), digest {digest:#018x}",
        words.len()
    );
    assert!(bad.is_empty(), "{} accepted words do not encode back, first {:#018x}", bad.len(), bad[0]);
    assert_eq!(unmeasured, FTOI_UNPACK_REFUSED_BY_ENCODER, "the FTOI* L/H population changed");
    assert_eq!(words.len(), ENUMERATION_WORDS, "the enumeration changed; re-run the ignored Mesa test");
    assert_eq!(text, MESA_TEXT_LINES, "the number of decoded words differs from Mesa's");
    assert_eq!(digest, MESA_DIGEST, "our lines differ from Mesa's; run the ignored test to see where");
}

/// Mesa's answer for every enumerated word, compared word by word. Needs the
/// harness: build tools/v3d/qpu_ref.c (its header) and run
/// `V3D_QPU_REF=<binary> cargo test ... mesa_ref -- --ignored --nocapture`.
#[test]
#[ignore = "needs V3D_QPU_REF, the tools/v3d/qpu_ref.c binary"]
fn decoder_agrees_with_mesa_on_every_field_combination() {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let bin = std::env::var("V3D_QPU_REF").expect("V3D_QPU_REF is not set; this test cannot measure anything");
    let words = enumeration();
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{bin}: {e}"));
    let mut stdin = child.stdin.take().expect("stdin");
    let feed = words.clone();
    let writer = std::thread::spawn(move || {
        for w in feed {
            writeln!(stdin, "{w:016x}").expect("write to qpu_ref");
        }
    });
    let out = BufReader::new(child.stdout.take().expect("stdout"));
    // Bytes, not str: a line Mesa prints that is not UTF-8 is a finding to
    // show, not a read error.
    let mesa: Vec<String> = out
        .split(b'\n')
        .map(|l| {
            String::from_utf8(l.expect("read qpu_ref")).unwrap_or_else(|e| format!("NOT-UTF8 {:x?}", e.as_bytes()))
        })
        .collect();
    writer.join().expect("writer");
    assert!(child.wait().expect("wait").success(), "qpu_ref failed");
    assert_eq!(mesa.len(), words.len(), "qpu_ref answered a different number of words");

    let mut expected = Vec::with_capacity(words.len());
    let mut classes: BTreeMap<Expect, usize> = BTreeMap::new();
    let mut bad = Vec::new();
    for (&w, m) in words.iter().zip(&mesa) {
        let (word, rest) = m.split_once(' ').expect("qpu_ref line");
        assert_eq!(u64::from_str_radix(word.trim_start_matches("0x"), 16).expect("hex"), w, "out of step");
        let (line, class) = expected_line(w, rest);
        *classes.entry(class).or_default() += 1;
        let ours = our_line(w);
        if ours != line {
            bad.push(format!("{ours:?}\n    expected {line:?}\n    Mesa {m:?}"));
        }
        expected.push(line);
    }
    let text = expected.iter().filter(|l| !l.ends_with(" R")).count();
    eprintln!(
        "Mesa over the enumeration: {} words, {text} text; classes {classes:?}; digest {:#018x}; {} differ",
        words.len(),
        fnv(&expected),
        bad.len()
    );
    assert!(bad.is_empty(), "{} words differ, first:\n{}", bad.len(), bad[..bad.len().min(10)].join("\n"));
    assert_eq!((words.len(), text, fnv(&expected)), (ENUMERATION_WORDS, MESA_TEXT_LINES, MESA_DIGEST), "re-pin");
}
