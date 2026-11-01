// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 7.1 gate: every distinct QPU word Mesa's v3d compiler emits for
//! V3D 7.1 for the tools/shaders corpus (`tools/v3d/qpu_corpus_71.txt`, the
//! twin of the 4.2 corpus `roundtrip.rs` gates), decoded and written back bit
//! for bit; the strict rules fed known-bad words and instructions; and a
//! random sweep. The 4.2 gate is `roundtrip.rs`, unchanged.

use super::codec71::{decode71, decode_gen, encode71, encode_any, AnyInstr};
use super::decode::DecodeError;
use super::encode::EncodeError;
use super::isa71::*;
use super::qpu_ffi::{self, Gen};
use std::collections::BTreeMap;

static CORPUS: &str = include_str!("../../../../tools/v3d/qpu_corpus_71.txt");

// The pins: the header of qpu_corpus_71.txt as it was generated (v3d_shc
// --device 71, the programs of the 4.2 corpus). Re-derive them, deliberately,
// when the corpus changes.
const PAIRS_OK: usize = 1090;
const PAIRS_FORMED: usize = 1116;
const WORDS_WITH_REPEATS: usize = 91_412;
const UNIQUE_WORDS: usize = 9_078;

fn header(key: &str) -> &'static str {
    let prefix = format!("# {key}: ");
    CORPUS
        .lines()
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("qpu_corpus_71.txt has no '{key}' header"))
}

fn header_num(v: &str, n: usize) -> usize {
    let nums: Vec<usize> =
        v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().expect("digits")).collect();
    nums[n]
}

/// Every word of the fixture, in file order. A line that is not a word or a
/// comment fails the parse rather than being skipped.
fn corpus() -> Vec<u64> {
    CORPUS
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let hex = l.strip_prefix("0x").unwrap_or_else(|| panic!("not a word: {l:?}"));
            u64::from_str_radix(hex, 16).unwrap_or_else(|e| panic!("not a word: {l:?}: {e}"))
        })
        .collect()
}

#[test]
fn corpus_matches_its_header_and_the_pins() {
    let words = corpus();
    let pairs = header("pairs");
    assert_eq!((header_num(pairs, 0), header_num(pairs, 1)), (PAIRS_OK, PAIRS_FORMED), "pairs: {pairs}");
    let total = header("words");
    let (all, fs, vs, cs) = (header_num(total, 0), header_num(total, 1), header_num(total, 2), header_num(total, 3));
    assert_eq!(all, WORDS_WITH_REPEATS, "words: {total}");
    assert_eq!(fs + vs + cs, all, "the per-stage counts add up: {total}");
    assert_eq!(header_num(header("unique"), 0), UNIQUE_WORDS);
    assert_eq!(words.len(), UNIQUE_WORDS, "the file holds as many words as its header says");
    assert!(words.windows(2).all(|p| p[0] < p[1]), "sorted and without repeats");
}

/// The gate: every 7.1 corpus word decodes and encodes back to itself.
#[test]
fn round_trip_every_corpus_word() {
    let words = corpus();
    let (mut ok, mut bad) = (0usize, Vec::new());
    for &w in &words {
        match decode71(w) {
            Ok(i) => match encode71(&i) {
                Ok(b) if b == w => ok += 1,
                other => bad.push(format!("{w:#018x} -> {other:x?}")),
            },
            Err(e) => bad.push(format!("{w:#018x}: {e:?}")),
        }
    }
    eprintln!("v3d 7.1 round trip: {} processed, {ok} round-tripped, {} failed", words.len(), bad.len());
    assert!(bad.is_empty(), "{} failed, first: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    assert_eq!(ok, UNIQUE_WORDS);
}

/// The same through the generation parameter.
#[test]
fn round_trip_through_the_generation_parameter() {
    for &w in corpus().iter().step_by(11) {
        let i = decode_gen(Gen::V71, w).expect("decodes");
        assert_eq!(i.gen(), Gen::V71);
        assert_eq!(encode_any(&i), Ok(w));
    }
    // The 4.2 side of the parameter is the 4.2 codec.
    let w = 0x3c20318105829000;
    let i = decode_gen(Gen::V42, w).expect("decodes");
    assert!(matches!(i, AnyInstr::V42(_)));
    assert_eq!(encode_any(&i), Ok(w));
}

/// What the corpus exercises: printed so a reader sees what the round trip
/// vouches for, and the 7.1 differences it must show (the SFU as add
/// operations, the four small-immediate positions, the implicit rf0 signals).
#[test]
fn corpus_coverage() {
    let mut add: BTreeMap<String, usize> = BTreeMap::new();
    let mut mul: BTreeMap<String, usize> = BTreeMap::new();
    let (mut alu, mut branch) = (0usize, 0usize);
    let mut imm = [0usize; 4];
    let mut sfu = 0usize;
    for w in corpus() {
        match decode71(w).expect("the round trip decodes every word") {
            Instr71::Alu(a) => {
                alu += 1;
                *add.entry(format!("{:?}", a.add.op)).or_default() += 1;
                *mul.entry(format!("{:?}", a.mul.op)).or_default() += 1;
                for (k, f) in [a.sig.small_imm_a, a.sig.small_imm_b, a.sig.small_imm_c, a.sig.small_imm_d].iter().enumerate()
                {
                    imm[k] += *f as usize;
                }
                sfu += matches!(
                    a.add.op,
                    AddOp71::Recip | AddOp71::Rsqrt | AddOp71::Exp | AddOp71::Log | AddOp71::Sin | AddOp71::Rsqrt2
                ) as usize;
            }
            Instr71::Branch(_) => branch += 1,
        }
    }
    eprintln!("v3d 7.1 corpus: {alu} ALU words, {branch} branch words; small imm a/b/c/d {imm:?}; SFU add ops {sfu}");
    eprintln!("  add ops ({} kinds): {add:?}", add.len());
    eprintln!("  mul ops ({} kinds): {mul:?}", mul.len());
    assert_eq!(alu + branch, UNIQUE_WORDS);
    assert!(sfu > 0, "no SFU add operation in the corpus");
    assert!(imm.iter().all(|&n| n > 0), "a small-immediate position is unexercised: {imm:?}");
}

/// Every word of the 7.1 corpus that is also a word of the 4.2 corpus is a
/// word the two generations read differently or alike; the 7.1 codec never
/// leans on the 4.2 tables: a 4.2 accumulator write is refused on 7.1.
#[test]
fn reserved_magic_addresses_are_refused() {
    for waddr in (0..=4).chain(19..=24) {
        assert_eq!(magic_waddr_name(waddr), None, "waddr {waddr}");
        let mut a = Alu71::nop();
        a.add = AddAlu71 { op: AddOp71::Add, waddr, magic: true, ..a.add };
        assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::MagicWaddrUnnamed), "waddr {waddr}");
    }
    assert_eq!(magic_waddr_name(WADDR_QUAD), Some("quad"));
    assert_eq!(magic_waddr_name(WADDR_REP), Some("rep"));
    assert_eq!(magic_waddr_name(WADDR_NOP), Some("-"));
    // 4.2 still names them.
    assert_eq!(qpu_ffi::magic_waddr_name(3), Some("r3"));
    assert_eq!(qpu_ffi::magic_waddr_name(5), Some("r5"));
    assert_eq!(qpu_ffi::magic_waddr_name(19), Some("recip"));
}

/// Mesa alone on a word: its unpack, then its pack of what it unpacked.
fn mesa_alone(w: u64) -> Option<u64> {
    let m = qpu_ffi::unpack_for(Gen::V71, w)?;
    Some(if qpu_ffi::pack_preconditions_71(&m) { qpu_ffi::pack_for(Gen::V71, &m).unwrap_or(!w) } else { !w })
}

/// Every accepted random word encodes back; every refusal fires.
#[test]
fn every_accepted_random_word_encodes_back() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    };
    let (mut accepted, mut bad) = (0usize, Vec::new());
    let mut refused: BTreeMap<DecodeError, usize> = BTreeMap::new();
    for _ in 0..(1 << 20) {
        let w = next();
        match decode71(w) {
            Ok(i) => {
                accepted += 1;
                match encode71(&i) {
                    Ok(b) if b == w => {}
                    other => bad.push(format!("{w:#018x} -> {other:x?}")),
                }
            }
            Err(e) => *refused.entry(e).or_default() += 1,
        }
    }
    eprintln!("7.1 random sweep: accepted {accepted}; refused {refused:?}");
    assert!(bad.is_empty(), "{} accepted words do not encode back, first: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    assert!(accepted > 10_000, "the sweep accepted too little to mean anything");
    // Mesa alone loses every NotReproducedByMesa word.
    // (Except the saturating mul FMOV words, see the test of that name.)
    let fmov_sat = |w: u64| matches!(decode71(w), Ok(Instr71::Alu(a)) if a.mul.op == MulOp71::Fmov && (w >> 14) & 7 >= 4);
    for &w in corpus().iter().step_by(13) {
        if !fmov_sat(w) {
            assert_eq!(mesa_alone(w), Some(w), "{w:#018x}");
        }
    }
}

/// One-bit mutations of corpus words: an accepted word encodes back, a word
/// refused as lossy is one Mesa's own pack(unpack(w)) changes.
#[test]
fn mutated_corpus_words_are_accepted_exactly_when_they_survive() {
    let (mut accepted, mut lossy, mut rule) = (0usize, 0usize, 0usize);
    for w in corpus().into_iter().step_by(7) {
        for bit in 0..64 {
            let x = w ^ (1u64 << bit);
            match decode71(x) {
                Ok(i) => {
                    accepted += 1;
                    assert_eq!(encode71(&i), Ok(x), "{x:#018x}");
                }
                Err(DecodeError::NotReproducedByMesa) => {
                    assert_ne!(mesa_alone(x), Some(x), "{x:#018x}: refused as lossy, but Mesa gives it back");
                    lossy += 1;
                }
                Err(_) => rule += 1,
            }
        }
    }
    eprintln!("7.1 one-bit mutations: {accepted} accepted, {lossy} refused as lossy, {rule} otherwise");
    assert!(accepted > 0 && lossy > 0 && rule > 0);
}

/// The strict rules, each fed a word Mesa accepts: built from the NOP word.
#[test]
fn strict_refusals_of_words_mesa_accepts() {
    let nop = encode71(&Instr71::Alu(Alu71::nop())).expect("the builder's nop encodes");
    assert!(corpus().binary_search(&nop).is_ok(), "Alu71::nop() ({nop:#018x}) is not a word Mesa emits");
    // A magic write to the reserved r3: Mesa round-trips it.
    let mut a = Alu71::nop();
    a.add = AddAlu71 { op: AddOp71::Add, waddr: 3, magic: true, ..a.add };
    let w = qpu_ffi::pack_for(Gen::V71, &qpu_ffi::to_mesa71(&Instr71::Alu(a))).expect("Mesa packs it");
    assert_eq!(mesa_alone(w), Some(w));
    assert_eq!(decode71(w), Err(DecodeError::MagicWaddrUnnamed));
    // A small-immediate index past the table in a read operand.
    let mut a = Alu71::nop();
    a.add = AddAlu71 { op: AddOp71::Add, a: Operand { raddr: 48, ..a.add.a }, ..a.add };
    a.sig.small_imm_a = true;
    let w = qpu_ffi::pack_for(Gen::V71, &qpu_ffi::to_mesa71(&Instr71::Alu(a))).expect("Mesa packs it");
    assert_eq!(mesa_alone(w), Some(w));
    assert_eq!(decode71(w), Err(DecodeError::SmallImmOutOfRange));
    // ... and in an operand the operation does not read it is not an error.
    let mut a = Alu71::nop();
    a.add.a.raddr = 48;
    a.sig.small_imm_a = true;
    assert!(encode71(&Instr71::Alu(a)).is_ok());
    // A branch with bits Mesa's model drops (bdu without ub).
    let b = 0x0200000500402000u64; // 4.2 "b.anynaq lri"; a branch word on 7.1 too
    assert!(decode71(b).is_ok());
    assert_eq!(decode71(b | 1), Err(DecodeError::NotReproducedByMesa));
    assert_eq!(mesa_alone(b | 1).map(|x| x != b | 1), Some(true));
    // OP_MUL 0 and not a branch signal: Mesa refuses.
    assert_eq!(decode71(0), Err(DecodeError::MesaRefuses));
}

/// The encoder refuses what it cannot write exactly.
#[test]
fn encoder_refusals() {
    // FADDNF of two identical operands is FADD's word.
    let mut a = Alu71::nop();
    a.add = AddAlu71 {
        op: AddOp71::Faddnf,
        a: Operand { raddr: 3, unpack: Unpack71::None },
        b: Operand { raddr: 3, unpack: Unpack71::None },
        waddr: 1,
        magic: false,
        pack: OutPack::None,
    };
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::ReadsBackAs));
    // ... while FADD with its operands the other way round is the same word.
    let mut f = Alu71::nop();
    f.add = AddAlu71 {
        op: AddOp71::Fadd,
        a: Operand { raddr: 9, unpack: Unpack71::None },
        b: Operand { raddr: 3, unpack: Unpack71::Abs },
        waddr: 1,
        magic: false,
        pack: OutPack::None,
    };
    let w = encode71(&Instr71::Alu(f)).expect("encodes");
    core::mem::swap(&mut f.add.a, &mut f.add.b);
    assert_eq!(encode71(&Instr71::Alu(f)), Ok(w));
    // A pack on a NOP, which Mesa drops.
    let mut a = Alu71::nop();
    a.mul.pack = OutPack::H;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::ReadsBackAs));
    // sig_addr without an address-writing signal.
    let mut a = Alu71::nop();
    a.sig_addr = 3;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::ReadsBackAs));
    // Both small-immediate flags of the add ALU: no signal has them.
    let mut a = Alu71::nop();
    a.sig.small_imm_a = true;
    a.sig.small_imm_b = true;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::MesaRefuses));
    // Too-wide fields are refused, not masked.
    let mut a = Alu71::nop();
    a.mul.b.raddr = 64;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::RaddrTooWide));
    let mut a = Alu71::nop();
    a.mul.waddr = 64;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::WaddrTooWide));
    let mut a = Alu71::nop();
    a.sig.ldunifrf = true;
    a.sig_addr = 64;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::SigAddrTooWide));
    // An LDVPM* with `magic`.
    let mut a = Alu71::nop();
    a.add = AddAlu71 { op: AddOp71::LdvpmvIn, waddr: 3, magic: true, ..a.add };
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::MagicNotAllowed));
    // A small immediate past the table in a read operand.
    let mut a = Alu71::nop();
    a.add = AddAlu71 { op: AddOp71::Add, b: Operand { raddr: 50, ..a.add.b }, ..a.add };
    a.sig.small_imm_b = true;
    assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::SmallImmOutOfRange));
}

/// Every add and mul operation the 7.1 model has answers Mesa's operand
/// questions without Mesa asserting, and every model variant converts to the
/// mirror and back unchanged.
#[test]
fn the_model_survives_the_mirror() {
    let mut a = Alu71::nop();
    for op in AddOp71::ALL {
        a.add.op = op;
        let _ = add_op_args(op);
        let i = Instr71::Alu(a);
        assert_eq!(qpu_ffi::from_mesa71(&qpu_ffi::to_mesa71(&i)), Some(i));
    }
    for op in MulOp71::ALL {
        a.mul.op = op;
        let _ = mul_op_args(op);
        let i = Instr71::Alu(a);
        assert_eq!(qpu_ffi::from_mesa71(&qpu_ffi::to_mesa71(&i)), Some(i));
    }
    for u in Unpack71::ALL {
        a.add.a.unpack = u;
        a.mul.b.unpack = u;
        let i = Instr71::Alu(a);
        assert_eq!(qpu_ffi::from_mesa71(&qpu_ffi::to_mesa71(&i)), Some(i));
    }
    // A value the model has no variant for is a refusal: VDWWT (51).
    let mut m = qpu_ffi::to_mesa71(&Instr71::Alu(Alu71::nop()));
    m.body.alu.add.op = 51;
    assert_eq!(qpu_ffi::from_mesa71(&m), None);
}

/// Mesa's 7.1 unpack of a mul FMOV reads two bits of the input unpack where
/// its pack writes three, so SAT, NSAT and MAX0 come back as ABS, NONE and L
/// and Mesa's own pack(unpack(w)) is not w ([`qpu_ffi::unpack71`]). The
/// compiler emits these words (the corpus holds them); the decoder reads
/// them right, and the raw Mesa reading loses them.
#[test]
fn mul_fmov_saturating_unpacks_are_read_with_three_bits() {
    let mut a = Alu71::nop();
    let mut seen = 0;
    for (u, raddr_d_unpack) in [(Unpack71::Sat, 4u64), (Unpack71::Nsat, 5), (Unpack71::Max0, 6)] {
        a.mul = MulAlu71 {
            op: MulOp71::Fmov,
            a: Operand { raddr: 3, unpack: u },
            b: Operand::default(),
            waddr: 2,
            magic: false,
            pack: OutPack::None,
        };
        let w = encode71(&Instr71::Alu(a)).expect("encodes");
        assert_eq!((w >> 14) & 7, raddr_d_unpack, "{u:?}: raddr_d bits 4..2");
        assert_eq!(decode71(w), Ok(Instr71::Alu(a)), "{u:?}");
        assert_ne!(mesa_alone(w), Some(w), "{u:?}: Mesa's own unpack loses the high bit");
        seen += 1;
    }
    assert_eq!(seen, 3);
    let in_corpus = corpus()
        .into_iter()
        .filter(|&w| matches!(decode71(w), Ok(Instr71::Alu(a)) if a.mul.op == MulOp71::Fmov && (a.mul.a.unpack as u32) >= Unpack71::Sat as u32))
        .count();
    eprintln!("mul FMOV with SAT/NSAT/MAX0 in the 7.1 corpus: {in_corpus}");
    assert!(in_corpus > 0, "the corpus has no such word: the fix is unmeasured");
    // 7 (raddr_d 28..30) is read by no unpack: refused.
    let w = encode71(&Instr71::Alu(a)).expect("encodes");
    let mut x = w & !(7 << 14);
    x |= 7 << 14;
    assert_eq!(decode71(x), Err(DecodeError::NotReproducedByMesa));
}

/// The 7.1 enums are Mesa's, by Mesa's own names.
#[test]
fn model_names_are_mesas() {
    let snake = |s: String| {
        let mut o = String::new();
        for (k, ch) in s.chars().enumerate() {
            if ch.is_ascii_uppercase() && k > 0 {
                o.push('_');
            }
            o.push(ch.to_ascii_lowercase());
        }
        o
    };
    for op in AddOp71::ALL {
        assert_eq!(qpu_ffi::add_op_name(op as u32), snake(format!("{op:?}")), "AddOp71::{op:?} = {}", op as u32);
    }
    for op in MulOp71::ALL {
        assert_eq!(qpu_ffi::mul_op_name(op as u32), snake(format!("{op:?}")), "MulOp71::{op:?} = {}", op as u32);
    }
    let want = ["", ".abs", ".l", ".h", ".sat", ".nsat", ".max0", ".ff", ".ll", ".hh", ".swp"];
    for (u, w) in Unpack71::ALL.iter().zip(want) {
        assert_eq!(qpu_ffi::unpack_name(*u as u32), w, "{u:?}");
    }
}
