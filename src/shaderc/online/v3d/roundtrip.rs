// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 4.2 gate: every distinct QPU word Mesa's v3d compiler emits for
//! the tools/shaders corpus, decoded and written back bit for bit.
//!
//! The corpus is `tools/v3d/qpu_corpus.txt`, written by
//! `tools/v3d/qpu_corpus.py` from a `tools/v3d/glsl2v3d.py` run (the fs, vs
//! and cs code of every program pair that compiles). Its counts are pinned
//! here, so a corpus that grows or shrinks fails the pin rather than quietly
//! changing what the gate covers.
//!
//! The words are packed and unpacked by Mesa's own `qpu_pack.c` (through
//! `qpu_ffi`), so what the round trip measures now is the wrapper around it:
//! the mirror of `struct v3d_qpu_instr` (a field lost or misplaced in the
//! conversion breaks decode-then-encode), the canonical form, and the strict
//! rules, which the tests below feed known-bad words and instructions (the
//! mutation tests: each rule refuses something Mesa alone would accept).
//! What the operations are called is the listing's business; Mesa's golden
//! text (`golden.rs`) and `mesa_ref` check that.

use super::decode::{decode, DecodeError};
use super::encode::{encode, EncodeError};
use super::fields::*;
use super::golden::text_of;
use super::isa::*;
use super::qpu_ffi;
use std::collections::BTreeMap;

static CORPUS: &str = include_str!("../../../../tools/v3d/qpu_corpus.txt");

// The pins: the header of qpu_corpus.txt as it was generated (e0c441974, Mesa
// 26.2.1 v3d_shc, after the Qt shaders left tools/shaders). Re-derive them,
// deliberately, when the corpus changes.
const PAIRS_OK: usize = 1090;
const PAIRS_FORMED: usize = 1116;
const WORDS_WITH_REPEATS: usize = 111_083;
const UNIQUE_WORDS: usize = 8_893;

/// A `# key: value` header line's value.
fn header(key: &str) -> &'static str {
    let prefix = format!("# {key}: ");
    CORPUS
        .lines()
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("qpu_corpus.txt has no '{key}' header"))
}

/// The n-th number in a header value (`"1090 ok of 1116 formed"`, 1 -> 1116).
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

/// The gate: every corpus word decodes and encodes back to itself.
#[test]
fn round_trip_every_corpus_word() {
    let words = corpus();
    let o = round_trip(&words);
    eprintln!("v3d round trip: {}", o.summary());
    assert_eq!(o.processed, UNIQUE_WORDS, "processed == in the corpus: {}", o.summary());
    assert_eq!(o.skipped(), 0, "nothing skipped: {}", o.summary());
    assert!(o.differing.is_empty(), "nothing differs: {}", o.summary());
    assert_eq!(o.ok, UNIQUE_WORDS);
}

/// What the corpus exercises, by operation and signal: printed so that a
/// reader can see what the round trip vouches for.
#[test]
fn corpus_coverage() {
    let mut add: BTreeMap<String, usize> = BTreeMap::new();
    let mut mul: BTreeMap<String, usize> = BTreeMap::new();
    let mut sig: BTreeMap<usize, usize> = BTreeMap::new();
    let (mut alu, mut branch) = (0usize, 0usize);
    for w in corpus() {
        match decode(w).expect("the round trip decodes every word") {
            Instr::Alu(a) => {
                alu += 1;
                *add.entry(format!("{:?}", a.add.op)).or_default() += 1;
                *mul.entry(format!("{:?}", a.mul.op)).or_default() += 1;
                *sig.entry(SIG.get(w) as usize).or_default() += 1;
            }
            Instr::Branch(_) => branch += 1,
        }
    }
    eprintln!("v3d corpus: {alu} ALU words, {branch} branch words");
    eprintln!("  add ops ({} kinds): {add:?}", add.len());
    eprintln!("  mul ops ({} kinds): {mul:?}", mul.len());
    eprintln!("  SIG values ({} of 27): {sig:?}", sig.len());
    assert_eq!(alu + branch, UNIQUE_WORDS);
}

// ---------------------------------------------------------------------------
// (c) Values Mesa's packer writes that the model does not carry, each against
// the corpus. Implied by the round trip; these tests make each population
// explicit and pin the ones the corpus cannot measure. (The mux bits of an
// operand the operation does not read -- `ffs(mask) - 1` of its table entry
// -- are Mesa's table's own and are covered by the round trip alone.)
// ---------------------------------------------------------------------------

/// Default 2: STVPMV/D/P write waddr 0/1/2 and MA 0.
#[test]
fn default_stvpm_waddr_and_ma_hold_in_the_corpus() {
    let mut n = 0;
    for w in corpus() {
        let Instr::Alu(a) = decode(w).expect("decodes") else { continue };
        let want = match a.add.op {
            AddOp::Stvpmv => 0,
            AddOp::Stvpmd => 1,
            AddOp::Stvpmp => 2,
            _ => continue,
        };
        n += 1;
        assert_eq!((WADDR_A.get(w), MA.get(w)), (want, 0), "{w:#018x}");
    }
    eprintln!("STVPM default: population {n}");
    assert_eq!(n, STVPM_POPULATION, "the corpus's STVPM population changed; re-derive the pin");
}

/// How many corpus words carry an STVPM (all STVPMV, the vertex shaders'
/// VPM stores). A zero here would name this default unmeasured.
const STVPM_POPULATION: usize = 492;

/// Default 3: a branch's unused fields are zero.
#[test]
fn default_branch_unused_fields_hold_in_the_corpus() {
    let (mut n, mut no_ub, mut no_raddr, mut no_offset) = (0, 0, 0, 0);
    for w in corpus() {
        let Instr::Branch(b) = decode(w).expect("decodes") else { continue };
        n += 1;
        assert_eq!(w & BRANCH_UNUSED, 0, "{w:#018x}: unused bits");
        if !b.ub {
            no_ub += 1;
            assert_eq!(BRANCH_BDU.get(w), 0, "{w:#018x}: bdu without ub");
        }
        if !b.reads_raddr_a() {
            no_raddr += 1;
            assert_eq!(RADDR_A.get(w), 0, "{w:#018x}: raddr_a unread");
        }
        if !b.has_offset() {
            no_offset += 1;
            assert_eq!(BRANCH_ADDR_LOW.get(w) | BRANCH_ADDR_HIGH.get(w), 0, "{w:#018x}: offset unread");
        }
    }
    eprintln!(
        "branch defaults: {n} branches; unused bits zero in {n}; bdu zero without ub in {no_ub}; \
         raddr_a zero unread in {no_raddr}; offset zero unread in {no_offset}"
    );
    assert_eq!(
        (n, no_ub, no_raddr, no_offset),
        BRANCH_POPULATIONS,
        "the branch populations changed; re-derive the pin"
    );
}

/// (branches, without ub, not reading raddr_a, without an offset). A zero
/// names that default as unmeasured: every branch Mesa emits here branches
/// the uniform stream too and has an offset, so "bdu zero without ub" and
/// "offset zero for lri/regfile" are UNMEASURED by the corpus (the random
/// sweep and strict_refusals_of_words_mesa_accepts are all that cover them).
const BRANCH_POPULATIONS: (usize, usize, usize, usize) = (71, 0, 71, 0);

/// The builder's defaults in [`Alu::nop`]: an add or mul NOP writes the
/// magic NOP register, and a read port no operand selects is 0.
#[test]
fn builder_nop_defaults_hold_in_the_corpus() {
    let words = corpus();
    let nop_word = encode(&Instr::Alu(Alu::nop())).expect("the builder's nop encodes");
    assert!(words.binary_search(&nop_word).is_ok(), "Alu::nop() ({nop_word:#018x}) is not a word Mesa emits");
    assert_eq!(text_of(&Instr::Alu(Alu::nop())), "nop                           ; nop");

    let (mut add_nops, mut mul_nops, mut a_unread, mut b_unread) = (0, 0, 0, 0);
    let mut wrong: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for &w in &words {
        let Instr::Alu(a) = decode(w).expect("decodes") else { continue };
        if a.add.op == AddOp::Nop {
            add_nops += 1;
            if (a.add.waddr, a.add.magic) != (WADDR_NOP, true) {
                wrong.entry("add nop waddr").or_insert((0, w)).0 += 1;
            }
        }
        if a.mul.op == MulOp::Nop {
            mul_nops += 1;
            if (a.mul.waddr, a.mul.magic) != (WADDR_NOP, true) {
                wrong.entry("mul nop waddr").or_insert((0, w)).0 += 1;
            }
        }
        let mut reads = Vec::new();
        for (args, x, y) in [(add_op_args(a.add.op), a.add.a, a.add.b), (mul_op_args(a.mul.op), a.mul.a, a.mul.b)] {
            let n = num_src(args);
            if n >= 1 {
                reads.push(x.mux);
            }
            if n >= 2 {
                reads.push(y.mux);
            }
        }
        if !reads.contains(&Mux::A) {
            a_unread += 1;
            if a.raddr_a != 0 {
                wrong.entry("raddr_a unread").or_insert((0, w)).0 += 1;
            }
        }
        if !reads.contains(&Mux::B) {
            b_unread += 1;
            if a.raddr_b != 0 {
                wrong.entry("raddr_b unread").or_insert((0, w)).0 += 1;
            }
        }
    }
    eprintln!(
        "nop builder defaults: {add_nops} add NOPs, {mul_nops} mul NOPs, raddr_a unread in {a_unread}, \
         raddr_b unread in {b_unread}; disagreements {wrong:x?}"
    );
    assert!(add_nops > 0 && mul_nops > 0 && a_unread > 0 && b_unread > 0, "a population is empty");
    assert!(wrong.is_empty(), "the corpus disagrees with Alu::nop()'s defaults: {wrong:x?}");
}

// ---------------------------------------------------------------------------
// Beyond the corpus: the decoder's refusals, the encoder's refusals, and a
// random sweep of the whole word space.
// ---------------------------------------------------------------------------

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Every word the decoder accepts encodes back to itself, over random words
/// of the whole space and random branch-shaped words; and every refusal the
/// decoder has fires at least once, so none of them is dead code.
#[test]
fn every_accepted_random_word_encodes_back() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut accepted = (0usize, 0usize);
    let mut refused: BTreeMap<DecodeError, usize> = BTreeMap::new();
    let mut bad = Vec::new();
    let mut unmeasured = 0usize;
    const N: usize = 1 << 20;
    for k in 0..N + N / 4 {
        let mut w = rng.next();
        if k >= N {
            // A branch: OP_MUL 0, SIG's top bits 0b10; the rest random, and
            // each optional field cleared half the time, so that words the
            // strict checks accept are common.
            w &= !(OP_MUL.mask() | BRANCH_SIG_HIGH.mask());
            w |= 1 << 57;
            let r = rng.next();
            if r & 1 != 0 {
                w &= !BRANCH_UNUSED;
            }
            if r & 2 != 0 {
                w &= !RADDR_A.mask();
            }
            if r & 4 != 0 {
                w &= !(BRANCH_ADDR_LOW.mask() | BRANCH_ADDR_HIGH.mask());
            }
            if r & 8 != 0 {
                w &= !BRANCH_BDU.mask();
            }
        }
        match decode(w) {
            Ok(i) => {
                match i {
                    Instr::Alu(_) => accepted.0 += 1,
                    Instr::Branch(_) => accepted.1 += 1,
                }
                match encode(&i) {
                    Ok(b) if b == w => {}
                    Err(EncodeError::UnpackUnmeasuredOnSilicon) if is_ftoi_unpack(&i) => unmeasured += 1,
                    other => bad.push(format!("{w:#018x} -> {other:x?}")),
                }
            }
            Err(e) => *refused.entry(e).or_default() += 1,
        }
    }
    eprintln!(
        "random sweep: accepted {} ALU, {} branch ({unmeasured} of them FTOI* L/H, decoded and refused by the \
         encoder as UnpackUnmeasuredOnSilicon); refused {refused:?}",
        accepted.0, accepted.1
    );
    assert!(unmeasured > 0, "the sweep never reached an FTOI* L/H word: the exception is unexercised");
    assert!(bad.is_empty(), "{} accepted words do not encode back, first: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    assert!(accepted.0 > 100_000 && accepted.1 > 10_000, "the sweep accepted too little to mean anything");
    let all = all_decode_errors();
    for e in all {
        assert!(refused.get(&e).copied().unwrap_or(0) > 0, "{e:?} never fired");
    }
    assert_eq!(refused.len(), all.len(), "a refusal fired that the list does not hold: {refused:?}");
}

/// Every [`DecodeError`] variant. The list is checked against an exhaustive
/// `match` (no wildcard), so a variant added to the enum and not here fails
/// to compile or fails the ordinal check, rather than leaving the sweep's
/// "every refusal fires" short by one.
fn all_decode_errors() -> [DecodeError; 5] {
    use DecodeError::*;
    let all = [MesaRefuses, NotReproducedByMesa, OutsideModel, MagicWaddrUnnamed, SmallImmOutOfRange];
    let ordinal = |e: DecodeError| match e {
        MesaRefuses => 0,
        NotReproducedByMesa => 1,
        OutsideModel => 2,
        MagicWaddrUnnamed => 3,
        SmallImmOutOfRange => 4,
    };
    let ords: Vec<usize> = all.iter().map(|&e| ordinal(e)).collect();
    assert_eq!(ords, (0..all.len()).collect::<Vec<_>>(), "the list and the match disagree");
    all
}

/// Mesa alone on a word: its unpack, then its pack of what it unpacked.
/// `None` when the unpack refuses; `Some(!w)` when the pack refuses (or
/// would assert), so the result differs from `w` exactly when Mesa loses it.
fn mesa_alone(w: u64) -> Option<u64> {
    let m = qpu_ffi::unpack(w)?;
    Some(if qpu_ffi::pack_preconditions(&m) { qpu_ffi::pack(&m).unwrap_or(!w) } else { !w })
}

/// The words Mesa accepts and this decoder refuses, one per cause, each
/// built from a Mesa golden vector by setting the bits Mesa would drop. A
/// mutation test of the decoder: each word is first shown to be one Mesa's
/// unpack takes (and whether Mesa's own pack gives it back), so the refusal
/// is the wrapper's.
#[test]
fn strict_refusals_of_words_mesa_accepts() {
    let refused = |w: u64, e: DecodeError, lossy: bool| {
        let back = mesa_alone(w).unwrap_or_else(|| panic!("{w:#018x}: Mesa's unpack refuses it"));
        assert_eq!(back != w, lossy, "{w:#018x}: Mesa's own pack(unpack(w)) is {back:#018x}");
        assert_eq!(decode(w), Err(e), "{w:#018x}");
    };
    // "b.anynaq  lri": bdi LINK_REG, no ub.
    let lri = 0x0200000500402000u64;
    assert!(decode(lri).is_ok());
    let mut w = lri;
    BRANCH_COND.set(&mut w, 1); // Mesa: A0 + (1 - 2) wraps to ALWAYS
    refused(w, DecodeError::NotReproducedByMesa, true);
    let mut w = lri;
    RADDR_A.set(&mut w, 5); // read by no destination
    refused(w, DecodeError::NotReproducedByMesa, true);
    let mut w = lri;
    BRANCH_ADDR_HIGH.set(&mut w, 1); // an offset on a branch to lri
    refused(w, DecodeError::NotReproducedByMesa, true);
    let mut w = lri;
    BRANCH_BDU.set(&mut w, 1); // bdu without ub
    refused(w, DecodeError::NotReproducedByMesa, true);
    refused(lri | 1, DecodeError::NotReproducedByMesa, true); // unused bits
    refused(lri | 1 << 23, DecodeError::NotReproducedByMesa, true);
    // "bu.anynaq  lri, r:unif" with bdu 4: Mesa packs the raw value back.
    let mut w = 0x020000050040e000u64;
    BRANCH_BDU.set(&mut w, 4);
    refused(w, DecodeError::OutsideModel, false);
    // "stvpmv 1, rf8" with MA set.
    let stvpm = 0x3de02040f8ff7201u64;
    assert!(decode(stvpm).is_ok());
    refused(stvpm | MA.mask(), DecodeError::NotReproducedByMesa, true);
    // "ldvpmg_in rf7, r2, r2" made LDVPMP (op 188, mux_b 2), then MA set:
    // Mesa's unpack calls it a magic write, which its pack asserts against.
    let mut w = 0x3de020c7bdfd200du64;
    OP_ADD.set(&mut w, 188);
    ADD_B.set(&mut w, 2);
    let Instr::Alu(a) = decode(w).unwrap() else { panic!("ALU") };
    assert_eq!((a.add.op, a.add.magic), (AddOp::Ldvpmp, false));
    refused(w | MA.mask(), DecodeError::NotReproducedByMesa, true);
    // "vflb.andnn rf24" (op 187, mux_b 0) made FXCD (mux_b 1), mux_a 0 then 1.
    let mut w = 0x5de24398bbdc6218u64;
    ADD_B.set(&mut w, 1);
    ADD_A.set(&mut w, 0);
    let Instr::Alu(a) = decode(w).unwrap() else { panic!("ALU") };
    assert_eq!(a.add.op, AddOp::Fxcd);
    ADD_A.set(&mut w, 1);
    refused(w, DecodeError::NotReproducedByMesa, true);
    // A small-immediate index past the table: Mesa round-trips it.
    let mut w = 0x5de24398bbdc6218u64;
    RADDR_B.set(&mut w, 48);
    refused(w, DecodeError::SmallImmOutOfRange, false);
    // A magic write to an unnamed address (add waddr 25, then 60, with MA):
    // Mesa round-trips both.
    let mut w = 0x3c00318735808000u64; // "vfpack tlb, r0, r1 ; nop"
    WADDR_A.set(&mut w, 25);
    refused(w, DecodeError::MagicWaddrUnnamed, false);
    WADDR_A.set(&mut w, 60);
    refused(w, DecodeError::MagicWaddrUnnamed, false);
}

/// The encoder refuses what it cannot write exactly, where Mesa would write
/// something else. A mutation test of the encoder: for each instruction it
/// is first said whether Mesa's packer alone refuses it, so a refusal Mesa
/// does not make is the wrapper's.
#[test]
fn encoder_refusals() {
    let mesa_refuses = |i: &Instr| {
        let m = qpu_ffi::to_mesa(i);
        assert!(qpu_ffi::pack_preconditions(&m));
        qpu_ffi::pack(&m).is_none()
    };
    let refused = |i: Instr, e: EncodeError, by_mesa_too: bool| {
        assert_eq!(mesa_refuses(&i), by_mesa_too, "{i:?}: does Mesa refuse it too");
        assert_eq!(encode(&i), Err(e), "{i:?}");
    };
    // "vpmsetup -, r5": VPMSETUP reading r0 is FLNAFIRST's word.
    let Instr::Alu(mut a) = decode(0x3c403186bb81d000).unwrap() else { panic!("ALU") };
    a.add.a.mux = Mux::R0;
    refused(Instr::Alu(a), EncodeError::ReadsBackAs, false);
    // "fadd r1, r1, r5": FADDNF of two identical operands is FADD's word.
    let Instr::Alu(mut a) = decode(0x3c20318105829000).unwrap() else { panic!("ALU") };
    a.add.op = AddOp::Faddnf;
    a.add.b = a.add.a;
    refused(Instr::Alu(a), EncodeError::ReadsBackAs, false);
    // ... while FADD with its operands the other way round is the same word.
    let w = 0x3c20318105829000;
    let Instr::Alu(mut a) = decode(w).unwrap() else { panic!("ALU") };
    core::mem::swap(&mut a.add.a, &mut a.add.b);
    assert_eq!(encode(&Instr::Alu(a)), Ok(w));
    // "vfpack tlb, r0, r1": a pack on VFPACK, which Mesa drops.
    let Instr::Alu(mut a) = decode(0x3c00318735808000).unwrap() else { panic!("ALU") };
    a.add.pack = OutPack::L;
    refused(Instr::Alu(a), EncodeError::ReadsBackAs, false);
    // A pack on a NOP, which Mesa drops.
    let mut a = Alu::nop();
    a.mul.pack = OutPack::H;
    refused(Instr::Alu(a), EncodeError::ReadsBackAs, false);
    // sig_addr without an address-writing signal, which Mesa ignores.
    let mut a = Alu::nop();
    a.sig_addr = 3;
    refused(Instr::Alu(a), EncodeError::ReadsBackAs, false);
    // Flags with an address-writing signal (Mesa refuses too).
    let mut a = Alu::nop();
    a.sig.ldunifrf = true;
    a.flags.apf = Pf::PushZ;
    refused(Instr::Alu(a), EncodeError::MesaRefuses, true);
    // "b.anynaq  zero_addr+...": a misaligned offset, raddr_a unread, bdu
    // without ub: Mesa drops each.
    let Instr::Branch(b) = decode(0x02b76a2dd0400000).unwrap() else { panic!("branch") };
    let mut x = b;
    x.offset |= 4;
    refused(Instr::Branch(x), EncodeError::ReadsBackAs, false);
    let mut x = b;
    x.raddr_a = 1;
    refused(Instr::Branch(x), EncodeError::ReadsBackAs, false);
    let mut x = b;
    x.bdu = BranchDest::Rel;
    refused(Instr::Branch(x), EncodeError::ReadsBackAs, false);
    // A magic write to an address no reference names (Mesa packs it).
    let mut a = Alu::nop();
    a.add.waddr = 25;
    refused(Instr::Alu(a), EncodeError::MagicWaddrUnnamed, false);
    let mut a = Alu::nop();
    a.sig.ldunifrf = true;
    (a.sig_addr, a.sig_magic) = (60, true);
    refused(Instr::Alu(a), EncodeError::MagicWaddrUnnamed, false);
    // A small immediate past the table (Mesa packs it).
    let mut a = Alu::nop();
    a.sig.small_imm_b = true;
    a.raddr_b = 48;
    refused(Instr::Alu(a), EncodeError::SmallImmOutOfRange, false);
    // A too-wide field is refused, not masked. Not fed to Mesa: its packer
    // asserts on these, which is what the refusal protects.
    let mut a = Alu::nop();
    a.raddr_a = 64;
    assert_eq!(encode(&Instr::Alu(a)), Err(EncodeError::RaddrTooWide));
    let mut a = Alu::nop();
    a.mul.waddr = 64;
    assert_eq!(encode(&Instr::Alu(a)), Err(EncodeError::WaddrTooWide));
    let mut a = Alu::nop();
    a.sig.ldunifrf = true;
    a.sig_addr = 64;
    assert_eq!(encode(&Instr::Alu(a)), Err(EncodeError::SigAddrTooWide));
    // An LDVPM* with `magic`: its MA bit is IN/OUT (Mesa asserts).
    let mut a = Alu::nop();
    a.add = AddAlu { op: AddOp::LdvpmvIn, waddr: 3, magic: true, ..a.add };
    assert_eq!(encode(&Instr::Alu(a)), Err(EncodeError::MagicNotAllowed));
}

/// An FTOIN/FTOIZ/FTOUZ/FTOC whose input unpack is L or H.
pub(super) fn is_ftoi_unpack(i: &Instr) -> bool {
    matches!(i, Instr::Alu(a)
        if matches!(a.add.op, AddOp::Ftoin | AddOp::Ftoiz | AddOp::Ftouz | AddOp::Ftoc)
            && a.add.a.unpack != Unpack::None)
}

/// FTOIN/FTOIZ/FTOUZ/FTOC with an L or H unpack: the decoder reads the words
/// that exist (249/250 as L, 253/254 as H, as Mesa's decoder does), and the
/// encoder refuses to write either (EncodeError::UnpackUnmeasuredOnSilicon),
/// because which bits the hardware reads as which half is not measured.
/// This pins the refusal. It waits for a board run that feeds an f16 value
/// with different halves through 249/250 and 253/254 and reads the result;
/// then the proven words are allowed. Mesa's packer alone writes 253/254
/// for both, so the refusal is the wrapper's (a mutation test).
#[test]
fn ftoi_unpacks_decode_but_are_refused_until_measured() {
    // "ffloor.ifb rf30.l, r3 ; fmul...", its add half made each operation.
    let Instr::Alu(mut a) = decode(0x9c094adef634b000).unwrap() else { panic!("ALU") };
    for (op, none_opcode) in [(AddOp::Ftoin, 245), (AddOp::Ftoiz, 245), (AddOp::Ftouz, 246), (AddOp::Ftoc, 246)] {
        a.add.op = op;
        a.add.pack = OutPack::None;
        a.add.a.unpack = Unpack::None;
        let w = encode(&Instr::Alu(a)).unwrap_or_else(|e| panic!("{op:?} NONE: {e:?}"));
        assert_eq!(OP_ADD.get(w), none_opcode, "{op:?}: NONE is the only unpack written");
        for u in [Unpack::L, Unpack::H] {
            a.add.a.unpack = u;
            let mesa = qpu_ffi::pack(&qpu_ffi::to_mesa(&Instr::Alu(a))).expect("Mesa packs it");
            assert_eq!(OP_ADD.get(mesa), none_opcode + 8, "{op:?} {u:?}: Mesa writes the H opcode for both");
            assert_eq!(encode(&Instr::Alu(a)), Err(EncodeError::UnpackUnmeasuredOnSilicon), "{op:?} {u:?}");
        }
        a.add.a.unpack = Unpack::Abs;
        assert_eq!(encode(&Instr::Alu(a)), Err(EncodeError::MesaRefuses), "{op:?} ABS");
        for (opcode, u) in [(none_opcode + 4, Unpack::L), (none_opcode + 8, Unpack::H)] {
            let mut x = w;
            OP_ADD.set(&mut x, opcode);
            let i = decode(x).unwrap_or_else(|e| panic!("{op:?} opcode {opcode}: {e:?}"));
            let Instr::Alu(d) = i else { panic!("ALU") };
            assert_eq!((d.add.op, d.add.a.unpack), (op, u), "opcode {opcode} reads as Mesa's decoder reads it");
            assert!(is_ftoi_unpack(&i));
            assert_eq!(encode(&i), Err(EncodeError::UnpackUnmeasuredOnSilicon), "{op:?} opcode {opcode}");
        }
    }
}

/// The wrapper's round trip is load-bearing: feed every corpus word with
/// one bit flipped, and every flipped word the decoder accepts must encode
/// back to itself, while the ones Mesa's own unpack/pack would change are
/// all refused. (With the old hand tables this was a corrupted-table
/// control; the tables are Mesa's now, so the mutation is of the input.)
#[test]
fn mutated_corpus_words_are_accepted_exactly_when_they_survive() {
    let (mut accepted, mut refused_lossy, mut refused_rule) = (0usize, 0usize, 0usize);
    for w in corpus().into_iter().step_by(7) {
        for bit in 0..64 {
            let x = w ^ (1u64 << bit);
            match decode(x) {
                Ok(i) => {
                    accepted += 1;
                    match encode(&i) {
                        Ok(b) => assert_eq!(b, x, "{x:#018x}"),
                        Err(EncodeError::UnpackUnmeasuredOnSilicon) if is_ftoi_unpack(&i) => {}
                        Err(e) => panic!("{x:#018x} decodes but does not encode: {e:?}"),
                    }
                }
                Err(DecodeError::NotReproducedByMesa) => {
                    assert_ne!(mesa_alone(x), Some(x), "{x:#018x}: refused as lossy, but Mesa gives it back");
                    refused_lossy += 1;
                }
                Err(_) => refused_rule += 1,
            }
        }
    }
    eprintln!("one-bit mutations: {accepted} accepted, {refused_lossy} refused as lossy, {refused_rule} otherwise");
    assert!(accepted > 0 && refused_lossy > 0 && refused_rule > 0);
}

/// Mesa's disassembly of every corpus word, compared with ours. It needs the
/// `--keep` directory of a glsl2v3d.py run (each pair's disasm.txt, which
/// v3d_shc writes from Mesa's `v3d_qpu_disasm`), so it does not run in the
/// gate; run it with `V3D_KEEP_DIR=<dir> cargo test ... listing_matches_mesa
/// -- --ignored`.
#[test]
#[ignore = "needs V3D_KEEP_DIR, the --keep output of tools/v3d/glsl2v3d.py"]
fn listing_matches_mesa_on_the_corpus() {
    let dir = std::env::var("V3D_KEEP_DIR").expect("V3D_KEEP_DIR is not set; this test cannot measure anything");
    let mut seen: BTreeMap<u64, String> = BTreeMap::new();
    let (mut lines, mut files) = (0usize, 0usize);
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&dir).expect("V3D_KEEP_DIR is a directory") {
        let path = e.expect("entry").path().join("disasm.txt");
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        files += 1;
        for l in text.lines() {
            // "%4u 0x%016llx %s" (v3d_shc.c disasm()).
            let t = l.trim_start();
            let Some((idx, rest)) = t.split_once(' ') else { continue };
            if idx.is_empty() || !idx.bytes().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let Some(rest) = rest.strip_prefix("0x") else { continue };
            if rest.len() < 16 {
                continue;
            }
            let (hex, mesa) = rest.split_at(16);
            let Ok(w) = u64::from_str_radix(hex, 16) else { continue };
            let mesa = mesa.strip_prefix(' ').unwrap_or(mesa);
            lines += 1;
            if let Some(prev) = seen.insert(w, mesa.to_string()) {
                assert_eq!(prev, mesa, "Mesa printed {w:#018x} two ways");
                continue;
            }
            let ours = decode(w).map(|i| text_of(&i));
            if ours.as_deref() != Ok(mesa) {
                bad.push(format!("{w:#018x}: ours {ours:?}, Mesa {mesa:?}"));
            }
        }
    }
    eprintln!(
        "listing vs Mesa: {files} disasm files, {lines} lines, {} distinct words, {} differ",
        seen.len(),
        bad.len()
    );
    assert!(files > 0 && lines > 0, "no disassembly was read from {dir}");
    assert_eq!(seen.len(), UNIQUE_WORDS, "the keep directory is not the corpus's run");
    assert!(bad.is_empty(), "first: {:?}", &bad[..bad.len().min(5)]);
}

/// The program-level helpers: a whole corpus-like program through
/// encode_program and for_each_instr, and their two failure reports.
#[test]
fn program_helpers() {
    use super::decode::{for_each_instr, ProgramError};
    use super::encode::{encode_program, ProgramEncodeError};
    let words: Vec<u64> = corpus().into_iter().take(1000).collect();
    let mut instrs = Vec::new();
    let n = for_each_instr::<()>(&words, |_, i| {
        instrs.push(*i);
        Ok(())
    })
    .expect("decodes");
    assert_eq!(n, words.len());
    let mut out = vec![0u64; instrs.len()];
    assert_eq!(encode_program(&instrs, &mut out), Ok(instrs.len()));
    assert_eq!(out, words);
    assert_eq!(encode_program(&instrs, &mut out[..10]), Err(ProgramEncodeError::OutTooSmall));
    let mut bad = instrs.clone();
    let k = bad.iter().position(|i| matches!(i, Instr::Alu(_))).expect("an ALU word among the first 1000");
    let Instr::Alu(mut a) = bad[k] else { unreachable!() };
    a.raddr_a = 64;
    bad[k] = Instr::Alu(a);
    assert_eq!(encode_program(&bad, &mut out), Err(ProgramEncodeError::Instr(k, EncodeError::RaddrTooWide)));
    assert_eq!(
        for_each_instr::<()>(&[words[0], 0], |_, _| Ok(())),
        Err(ProgramError::Decode(1, DecodeError::MesaRefuses))
    );
}
