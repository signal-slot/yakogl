// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 7.1 gate: every distinct QPU word Mesa's v3d compiler emits for
//! the tools/shaders corpus when it targets V3D 7.1 (the Raspberry Pi 5),
//! decoded and written back bit for bit. The 7.1 twin of `roundtrip`.
//!
//! The corpus is `tools/v3d/qpu_corpus_71.txt`, written by
//! `tools/v3d/qpu_corpus.py --device 71` (the words of the same program pairs
//! as the 4.2 corpus, compiled by `v3d_shc --device 71`). Its counts are
//! pinned here. The words are packed and unpacked by Mesa's own `qpu_pack.c`
//! with the 7.1 device, so what the round trip measures is the wrapper
//! around it ([`super::codec71`]): the mirror, the canonical form and the
//! strict rules, which the tests below feed known-bad words and
//! instructions.

use super::codec71::{decode, encode, from_mesa, to_mesa};
use super::decode::DecodeError;
use super::encode::EncodeError;
use super::isa71::*;
use super::qpu_ffi::{self, Gen};
use std::collections::BTreeMap;

static CORPUS: &str = include_str!("../../../../tools/v3d/qpu_corpus_71.txt");

// The pins: the header of qpu_corpus_71.txt as it was generated (7bd8a8e94,
// Mesa 26.2.1 v3d_shc --device 71). Re-derive them, deliberately, when the
// corpus changes.
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

#[derive(Default, Debug)]
struct Outcome {
    processed: usize,
    ok: usize,
    decode_refused: BTreeMap<DecodeError, (usize, u64)>,
    encode_refused: BTreeMap<String, (usize, u64)>,
    differing: Vec<(u64, u64)>,
}

impl Outcome {
    fn skipped(&self) -> usize {
        self.decode_refused.values().map(|v| v.0).sum::<usize>()
            + self.encode_refused.values().map(|v| v.0).sum::<usize>()
    }
    fn summary(&self) -> String {
        format!(
            "{} processed, {} round-tripped, {} skipped (decode {:x?}, encode {:x?}), {} differing{}",
            self.processed,
            self.ok,
            self.skipped(),
            self.decode_refused,
            self.encode_refused,
            self.differing.len(),
            self.differing.first().map(|(w, b)| format!(", first {w:#018x} -> {b:#018x}")).unwrap_or_default()
        )
    }
}

fn round_trip(words: &[u64]) -> Outcome {
    let mut o = Outcome::default();
    for &w in words {
        o.processed += 1;
        let i = match decode(w) {
            Ok(i) => i,
            Err(e) => {
                o.decode_refused.entry(e).or_insert((0, w)).0 += 1;
                continue;
            }
        };
        match encode(&i) {
            Ok(back) if back == w => o.ok += 1,
            Ok(back) => o.differing.push((w, back)),
            Err(e) => o.encode_refused.entry(format!("{e:?}")).or_insert((0, w)).0 += 1,
        }
    }
    o
}

/// The fixture is what its header says, and the header is what is pinned.
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
    let o = round_trip(&words);
    eprintln!("v3d 7.1 round trip: {}", o.summary());
    assert_eq!(o.processed, UNIQUE_WORDS, "processed == in the corpus: {}", o.summary());
    assert_eq!(o.skipped(), 0, "nothing skipped: {}", o.summary());
    assert!(o.differing.is_empty(), "nothing differs: {}", o.summary());
    assert_eq!(o.ok, UNIQUE_WORDS);
}

/// What the 7.1 corpus exercises, and what is 7.1 about it.
#[test]
fn corpus_coverage_is_7x() {
    let mut add: BTreeMap<String, usize> = BTreeMap::new();
    let mut mul: BTreeMap<String, usize> = BTreeMap::new();
    let (mut alu, mut branch, mut rf0, mut sfu, mut simm) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for w in corpus() {
        match decode(w).expect("the round trip decodes every word") {
            Instr71::Alu(a) => {
                alu += 1;
                *add.entry(format!("{:?}", a.add.op)).or_default() += 1;
                *mul.entry(format!("{:?}", a.mul.op)).or_default() += 1;
                rf0 += a.writes_rf0_implicitly() as usize;
                sfu += a.is_sfu() as usize;
                simm += (0..4).any(|k| a.small_imm(k).is_some()) as usize;
                // No accumulators: nothing writes the magic addresses 0..4.
                for (waddr, magic) in [(a.add.waddr, a.add.magic), (a.mul.waddr, a.mul.magic)] {
                    assert!(!(magic && waddr <= 4), "{w:#018x}: a magic write to r{waddr} on 7.1");
                }
            }
            Instr71::Branch(_) => branch += 1,
        }
    }
    eprintln!("v3d 7.1 corpus: {alu} ALU words, {branch} branch words, {rf0} write rf0 implicitly, {sfu} SFU ops, {simm} small immediates");
    eprintln!("  add ops ({} kinds): {add:?}", add.len());
    eprintln!("  mul ops ({} kinds): {mul:?}", mul.len());
    assert_eq!(alu + branch, UNIQUE_WORDS);
    assert!(rf0 > 0 && sfu > 0 && simm > 0, "the corpus reaches the 7.1 differences");
    // The special functions are ALU operations on 7.1, never magic writes.
    for w in corpus() {
        if let Instr71::Alu(a) = decode(w).unwrap() {
            for (waddr, magic) in [(a.add.waddr, a.add.magic), (a.mul.waddr, a.mul.magic)] {
                assert!(!(magic && (19..=24).contains(&waddr)), "{w:#018x}: an SFU magic write on 7.1");
            }
        }
    }
}

/// The model converts to the mirror and back unchanged, for every variant of
/// every enum it holds.
#[test]
fn the_model_survives_the_mirror() {
    let mut a = Alu71::nop();
    for op in AddOp71::ALL {
        a.add.op = op;
        let i = Instr71::Alu(a);
        assert_eq!(from_mesa(&to_mesa(&i)), Some(i), "{op:?}");
    }
    a = Alu71::nop();
    for op in MulOp71::ALL {
        a.mul.op = op;
        let i = Instr71::Alu(a);
        assert_eq!(from_mesa(&to_mesa(&i)), Some(i), "{op:?}");
    }
    a = Alu71::nop();
    for u in Unpack71::ALL {
        for raddr in [0u8, 1, 17, 63] {
            a.add.a = Input71 { raddr, unpack: u };
            a.add.b = Input71 { raddr: 63 - raddr, unpack: u };
            a.mul.a = Input71 { raddr: raddr ^ 5, unpack: u };
            a.mul.b = Input71 { raddr: raddr ^ 9, unpack: u };
            let i = Instr71::Alu(a);
            assert_eq!(from_mesa(&to_mesa(&i)), Some(i), "{u:?} r{raddr}");
        }
    }
    a = Alu71::nop();
    for k in 0..16 {
        let mut s = Sig71::default();
        let f = [
            &mut s.thrsw,
            &mut s.ldunif,
            &mut s.ldunifa,
            &mut s.ldunifrf,
            &mut s.ldunifarf,
            &mut s.ldtmu,
            &mut s.ldvary,
            &mut s.ldtlb,
            &mut s.ldtlbu,
            &mut s.ucb,
            &mut s.rotate,
            &mut s.wrtmuc,
            &mut s.small_imm_a,
            &mut s.small_imm_b,
            &mut s.small_imm_c,
            &mut s.small_imm_d,
        ];
        *f.into_iter().nth(k).unwrap() = true;
        a.sig = s;
        let i = Instr71::Alu(a);
        assert_eq!(from_mesa(&to_mesa(&i)), Some(i), "sig field {k}");
    }
    // A value the model has no variant for is a refusal (ldvpm is V3D 3.x).
    let mut m = to_mesa(&Instr71::Alu(Alu71::nop()));
    m.sig.set(7, true);
    assert_eq!(from_mesa(&m), None);
    let mut m = to_mesa(&Instr71::Alu(Alu71::nop()));
    m.body.alu.add.op = 107;
    assert_eq!(from_mesa(&m), None);
}

/// The 7.1 sub-gates of what the notes say is different: the signal map, the
/// implicit rf0, the SFU as ALU operations, `quad`/`rep`, no accumulators.
#[test]
fn what_7x_changes_in_the_word() {
    // Signal 2 is `ldunif` on both generations; on 7.1 it writes rf0.
    let mut a = Alu71::nop();
    a.sig.ldunif = true;
    let word = encode(&Instr71::Alu(a)).expect("ldunif encodes");
    assert!(decode(word).is_ok());
    assert!(a.writes_rf0_implicitly(), "ldunif writes rf0 on 7.1");
    assert!(
        !qpu_ffi::writes_rf0_implicitly(Gen::V42, &to_mesa(&Instr71::Alu(a))),
        "and does not on 4.2 (it writes r5)"
    );
    // The same signal combination that is signal 14 on 7.1 (small_imm_a) is
    // not a 4.2 signal: the 4.2 decoder refuses the word the 7.1 one makes.
    let mut s = Alu71::nop();
    s.sig.small_imm_a = true;
    let w = encode(&Instr71::Alu(s)).expect("small_imm_a encodes on 7.1");
    assert_ne!(w, encode(&Instr71::Alu(Alu71::nop())).unwrap());
    // SFU: RECIP is an add ALU operation reading one operand on 7.1.
    let mut r = Alu71::nop();
    r.add = AddAlu71 {
        op: AddOp71::Recip,
        a: Input71 { raddr: 3, unpack: Unpack71::None },
        b: Input71::default(),
        waddr: 0,
        magic: false,
        pack: OutPack::None,
    };
    assert!(r.is_sfu());
    let w = encode(&Instr71::Alu(r)).expect("recip encodes");
    // Operand b is not read: the word carries Mesa's address for it, not ours.
    assert!(super::codec71::same_instr(&Instr71::Alu(r), &decode(w).unwrap()));
    // The names: 5 is quad and 55 rep on 7.1; r5 and r5rep on 4.2.
    assert_eq!(magic_waddr_name(5), Some("quad"));
    assert_eq!(magic_waddr_name(55), Some("rep"));
    assert_eq!(crate::shaderc::online::v3d::isa::magic_waddr_name(5), Some("r5"));
    assert_eq!(crate::shaderc::online::v3d::isa::magic_waddr_name(55), Some("r5rep"));
}

/// Mesa's 7.1 decoder refuses words the 4.2 one reads, and the other way
/// round: the generation is a parameter that matters.
#[test]
fn the_generation_changes_the_decode() {
    let mut differ = 0;
    for w in corpus().into_iter().step_by(11) {
        let a = qpu_ffi::unpack_for(Gen::V71, w).is_some();
        let b = qpu_ffi::unpack_for(Gen::V42, w).is_some();
        differ += (a != b) as usize;
        assert!(a, "{w:#018x}: a 7.1 corpus word Mesa's 7.1 unpack refuses");
    }
    assert!(differ > 0, "some 7.1 words are not 4.2 words");
}

/// The strict rules refuse what Mesa's packer would write.
#[test]
fn strict_refusals() {
    let mut a = Alu71::nop();
    a.add = AddAlu71 {
        op: AddOp71::Add,
        a: Input71 { raddr: 64, unpack: Unpack71::None },
        b: Input71::default(),
        waddr: 1,
        magic: false,
        pack: OutPack::None,
    };
    assert_eq!(encode(&Instr71::Alu(a)), Err(EncodeError::RaddrTooWide));
    a.add.a.raddr = 1;
    a.add.waddr = 64;
    assert_eq!(encode(&Instr71::Alu(a)), Err(EncodeError::WaddrTooWide));
    a.add.waddr = 1;
    a.sig_addr = 64;
    assert_eq!(encode(&Instr71::Alu(a)), Err(EncodeError::SigAddrTooWide));
    a.sig_addr = 0;
    // A magic write to an address no reference names (25 is in the gap).
    a.add.waddr = 25;
    a.add.magic = true;
    assert_eq!(encode(&Instr71::Alu(a)), Err(EncodeError::MagicWaddrUnnamed));
    a.add.magic = false;
    a.add.waddr = 1;
    // A small-immediate index past the table (48..63) on a read operand.
    a.sig.small_imm_a = true;
    a.add.a.raddr = 50;
    assert_eq!(encode(&Instr71::Alu(a)), Err(EncodeError::SmallImmOutOfRange));
    a.add.a.raddr = 5;
    assert!(encode(&Instr71::Alu(a)).is_ok());
    // A mul operand b the operation does not read is not in the word: Mesa
    // writes the opcode's address (63 for a NOP) whatever the builder says.
    let mut n = Alu71::nop();
    n.mul.b = Input71 { raddr: 9, unpack: Unpack71::None };
    assert_eq!(encode(&Instr71::Alu(n)), Ok(encode(&Instr71::Alu(Alu71::nop())).unwrap()));
    // A word Mesa unpacks but cannot give back is refused by the decoder.
    let junk: Vec<u64> = (0..2000u64).map(|i| i.wrapping_mul(0x9e37_79b9_7f4a_7c15)).collect();
    let mut refused = 0;
    for w in junk {
        match decode(w) {
            Ok(i) => assert_eq!(encode(&i), Ok(w), "{w:#018x}"),
            Err(DecodeError::MesaRefuses | DecodeError::NotReproducedByMesa) => refused += 1,
            Err(_) => refused += 1,
        }
    }
    assert!(refused > 0);
}

/// Random words: whatever the decoder accepts, the encoder writes back.
#[test]
fn every_accepted_random_word_encodes_back() {
    let mut x = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let (mut ok, mut refused) = (0usize, 0usize);
    let c = corpus();
    for k in 0..200_000 {
        // Half the words are corpus words with a few bits flipped, so that
        // the population is near what the compiler writes.
        let w = if k % 2 == 0 {
            next()
        } else {
            c[(next() % c.len() as u64) as usize] ^ (1u64 << (next() % 64)) ^ (1u64 << (next() % 64))
        };
        match decode(w) {
            Ok(i) => {
                assert_eq!(encode(&i), Ok(w), "{w:#018x} -> {i:?}");
                ok += 1;
            }
            Err(_) => refused += 1,
        }
    }
    eprintln!("v3d 7.1 random words: {ok} accepted and round-tripped, {refused} refused");
    assert!(ok > 1000);
}
