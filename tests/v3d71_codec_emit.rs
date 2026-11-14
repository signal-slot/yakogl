// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! codec71 over the combinations the V3D 7.1 back end can form (plan #365,
//! item codec71-emit). `roundtrip71.rs` checks the codec on the words Mesa
//! emits; our back end may form instructions Mesa never does (another
//! raddr/signal pairing, a mul op under an unusual condition). Here every
//! add and mul op, every signal (every subset, with an rf and a magic
//! destination), every condition and flag-update form, the branch forms and
//! the SFU-as-ALU ops go through `encode71` and `decode71`, and:
//!
//! * an instruction `encode71` accepts decodes to the same instruction
//!   (`same_instr71`), and the word decodes and encodes to itself;
//! * one it refuses is refused with an error, by name (`named_refusals` and
//!   the per-sweep class lists pin each class); nothing is mis-encoded;
//! * a word that is also in `tools/v3d/qpu_corpus_71.txt` is Mesa's own bits.
//!
//! Defects found are `#[ignore]`d cases with the reason; this file does not
//! change `codec71.rs`.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_codec_emit`.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::{BTreeMap, BTreeSet};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71, same_instr71};
use yakogl::shaderc::online::v3d::encode::EncodeError as E;
use yakogl::shaderc::online::v3d::isa71::*;

static CORPUS: &str = include_str!("../tools/v3d/qpu_corpus_71.txt");

fn corpus() -> BTreeSet<u64> {
    CORPUS
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| u64::from_str_radix(l.strip_prefix("0x").expect("a word"), 16).expect("hex"))
        .collect()
}

/// What happened to the instructions of one sweep.
#[derive(Default)]
struct Tally {
    formed: usize,
    ok: usize,
    in_corpus: usize,
    /// Refusals by the error's name (the variant, without its payload).
    refused: BTreeMap<String, usize>,
    /// The first instruction of each refusal class, for the failure text.
    first: BTreeMap<String, String>,
    words: BTreeSet<u64>,
}

impl Tally {
    fn refusals(&self, name: &str) -> usize {
        self.refused.get(name).copied().unwrap_or(0)
    }
}

/// Encode `i`; an accepted instruction must read back as itself and its word
/// must be a fixed point of decode71/encode71.
fn check(t: &mut Tally, corpus: &BTreeSet<u64>, i: Instr71) {
    t.formed += 1;
    match encode71(&i) {
        Ok(w) => {
            let back = decode71(w).unwrap_or_else(|e| panic!("{i:?} -> {w:#018x} does not decode: {e:?}"));
            assert!(same_instr71(&i, &back), "{i:?} -> {w:#018x} reads back as {back:?}");
            assert_eq!(encode71(&back), Ok(w), "{w:#018x} is not a fixed point");
            if corpus.contains(&w) {
                t.in_corpus += 1;
            }
            t.ok += 1;
            t.words.insert(w);
        }
        Err(e) => {
            let s = format!("{e:?}");
            let name = s.split(['(', ' ']).next().unwrap().to_string();
            t.first.entry(name.clone()).or_insert_with(|| format!("{i:?}"));
            *t.refused.entry(name).or_insert(0) += 1;
        }
    }
}

fn op(raddr: u8) -> Operand {
    Operand { raddr, unpack: Unpack71::None }
}

fn add_base(o: AddOp71) -> Alu71 {
    let mut a = Alu71::nop();
    a.add = AddAlu71 { op: o, a: op(1), b: op(2), waddr: 3, magic: false, pack: OutPack::None };
    a
}

fn mul_base(o: MulOp71) -> Alu71 {
    let mut a = Alu71::nop();
    a.mul = MulAlu71 { op: o, a: op(1), b: op(2), waddr: 3, magic: false, pack: OutPack::None };
    a
}

const RADDRS: [u8; 6] = [0, 1, 31, 32, 62, 63];
/// Small-immediate indices: in the table (0..47) and past it.
const SMALL: [u8; 5] = [0, 15, 31, 47, 48];

/// Every add op: raddr forms, small-immediate forms, unpacks on both
/// operands, destinations (rf, rf0, every magic address), packs, and every
/// condition / push / update of the add half.
fn add_forms(f: &mut dyn FnMut(Instr71)) {
    for &o in AddOp71::ALL.iter() {
        let base = add_base(o);
        let mut ins = |a: Alu71| f(Instr71::Alu(a));
        for &ra in &RADDRS {
            for &rb in &RADDRS {
                let mut a = base;
                a.add.a = op(ra);
                a.add.b = op(rb);
                ins(a);
            }
        }
        for &ra in &SMALL {
            for &rb in &SMALL {
                for (sa, sb) in [(true, false), (false, true), (true, true)] {
                    let mut a = base;
                    a.add.a = op(ra);
                    a.add.b = op(rb);
                    a.sig.small_imm_a = sa;
                    a.sig.small_imm_b = sb;
                    ins(a);
                }
            }
        }
        for &u in &Unpack71::ALL {
            for &ub in &[Unpack71::None, u] {
                let mut a = base;
                a.add.a.unpack = u;
                a.add.b.unpack = ub;
                ins(a);
            }
        }
        for magic in [false, true] {
            for w in 0..64u8 {
                let mut a = base;
                a.add.waddr = w;
                a.add.magic = magic;
                ins(a);
            }
        }
        for &p in &OutPack::ALL {
            let mut a = base;
            a.add.pack = p;
            ins(a);
            a.add.a.unpack = Unpack71::L;
            ins(a);
        }
        for &c in &Cond::ALL {
            for &pf in &Pf::ALL {
                for &uf in &Uf::ALL {
                    let mut a = base;
                    a.flags.ac = c;
                    a.flags.apf = pf;
                    a.flags.auf = uf;
                    ins(a);
                }
            }
        }
    }
}

/// Every mul op: the same dimensions.
fn mul_forms(f: &mut dyn FnMut(Instr71)) {
    for &o in MulOp71::ALL.iter() {
        let base = mul_base(o);
        let mut ins = |a: Alu71| f(Instr71::Alu(a));
        for &ra in &RADDRS {
            for &rb in &RADDRS {
                let mut a = base;
                a.mul.a = op(ra);
                a.mul.b = op(rb);
                ins(a);
            }
        }
        for &ra in &SMALL {
            for &rb in &SMALL {
                for (sc, sd) in [(true, false), (false, true), (true, true)] {
                    let mut a = base;
                    a.mul.a = op(ra);
                    a.mul.b = op(rb);
                    a.sig.small_imm_c = sc;
                    a.sig.small_imm_d = sd;
                    ins(a);
                }
            }
        }
        for &u in &Unpack71::ALL {
            for &ub in &[Unpack71::None, u] {
                let mut a = base;
                a.mul.a.unpack = u;
                a.mul.b.unpack = ub;
                ins(a);
            }
        }
        for magic in [false, true] {
            for w in 0..64u8 {
                let mut a = base;
                a.mul.waddr = w;
                a.mul.magic = magic;
                ins(a);
            }
        }
        for &p in &OutPack::ALL {
            let mut a = base;
            a.mul.pack = p;
            ins(a);
            a.mul.a.unpack = Unpack71::L;
            ins(a);
        }
        for &c in &Cond::ALL {
            for &pf in &Pf::ALL {
                for &uf in &Uf::ALL {
                    let mut a = base;
                    a.flags.mc = c;
                    a.flags.mpf = pf;
                    a.flags.muf = uf;
                    ins(a);
                }
            }
        }
    }
}

fn sig_from_bits(b: u16) -> Sig71 {
    let f = |n: u16| b & (1 << n) != 0;
    Sig71 {
        thrsw: f(0),
        ldunif: f(1),
        ldunifa: f(2),
        ldunifrf: f(3),
        ldunifarf: f(4),
        ldtmu: f(5),
        ldvary: f(6),
        ldtlb: f(7),
        ldtlbu: f(8),
        ucb: f(9),
        rotate: f(10),
        wrtmuc: f(11),
        ..Sig71::default()
    }
}

/// Every subset of the twelve signals, each with the destinations the back end
/// writes (rf0, rf, a named magic, a reserved magic), on a NOP/NOP and on an
/// add+mul instruction; and the small-immediate flags with up to two signals.
fn signal_forms(f: &mut dyn FnMut(Instr71)) {
    let mut busy = Alu71::nop();
    busy.add = AddAlu71 { op: AddOp71::Add, a: op(4), b: op(5), waddr: 6, magic: false, pack: OutPack::None };
    busy.mul = MulAlu71 { op: MulOp71::Fmul, a: op(7), b: op(8), waddr: 9, magic: false, pack: OutPack::None };
    for bits in 0u16..(1 << 12) {
        let sig = sig_from_bits(bits);
        for base in [Alu71::nop(), busy] {
            for (addr, magic) in [(0u8, false), (7, false), (63, false), (6, true), (4, true), (20, true), (56, true)] {
                let mut a = base;
                a.sig = sig;
                a.sig_addr = addr;
                a.sig_magic = magic;
                f(Instr71::Alu(a));
            }
        }
    }
    for bits in 0u16..(1 << 12) {
        if bits.count_ones() > 2 {
            continue;
        }
        for imm in 1u8..16 {
            let mut a = busy;
            a.sig = sig_from_bits(bits);
            a.sig.small_imm_a = imm & 1 != 0;
            a.sig.small_imm_b = imm & 2 != 0;
            a.sig.small_imm_c = imm & 4 != 0;
            a.sig.small_imm_d = imm & 8 != 0;
            a.add.a.raddr = 3;
            a.add.b.raddr = 4;
            a.mul.a.raddr = 5;
            a.mul.b.raddr = 6;
            f(Instr71::Alu(a));
            a.sig_addr = 2;
            f(Instr71::Alu(a));
        }
    }
}

/// Both halves' conditions and flag updates together on an add+mul
/// instruction, and flags beside each single signal (flags with an
/// address-writing signal are a refusal class).
fn flag_forms(f: &mut dyn FnMut(Instr71)) {
    let mut base = add_base(AddOp71::Add);
    base.mul = mul_base(MulOp71::Fmul).mul;
    for &ac in &Cond::ALL {
        for &mc in &Cond::ALL {
            for &apf in &Pf::ALL {
                for &mpf in &Pf::ALL {
                    for &auf in &Uf::ALL {
                        for &muf in &Uf::ALL {
                            let mut a = base;
                            a.flags = Flags { ac, mc, apf, mpf, auf, muf };
                            f(Instr71::Alu(a));
                        }
                    }
                }
            }
        }
    }
    for bits in 0u16..(1 << 12) {
        if bits.count_ones() > 1 {
            continue;
        }
        for flags in [
            Flags { ac: Cond::IfA, ..Flags::default() },
            Flags { mc: Cond::IfNB, ..Flags::default() },
            Flags { apf: Pf::PushZ, ..Flags::default() },
            Flags { muf: Uf::AndZ, ..Flags::default() },
        ] {
            let mut a = base;
            a.sig = sig_from_bits(bits);
            a.flags = flags;
            a.sig_addr = 5;
            f(Instr71::Alu(a));
        }
    }
}

const SFU: [AddOp71; 6] = [AddOp71::Recip, AddOp71::Rsqrt, AddOp71::Exp, AddOp71::Log, AddOp71::Sin, AddOp71::Rsqrt2];

/// The SFU as ALU operations with an rf destination, with the mul half busy,
/// with a signal, and written to the 4.x SFU/accumulator magic addresses
/// (reserved at 7.1).
fn sfu_forms(f: &mut dyn FnMut(Instr71)) {
    for o in SFU {
        for dst in [0u8, 1, 40, 63] {
            let mut a = add_base(o);
            a.add.waddr = dst;
            f(Instr71::Alu(a));
            a.mul = mul_base(MulOp71::Fmul).mul;
            f(Instr71::Alu(a));
            for sig in [
                Sig71 { ldvary: true, ..Sig71::default() },
                Sig71 { ldunifrf: true, ..Sig71::default() },
                Sig71 { ldtmu: true, ..Sig71::default() },
                Sig71 { thrsw: true, ..Sig71::default() },
            ] {
                let mut s = a;
                s.sig = sig;
                s.sig_addr = 9;
                f(Instr71::Alu(s));
            }
        }
        for m in [0u8, 1, 2, 3, 4, 19, 20, 21, 22, 23, 24, WADDR_NOP] {
            let mut a = add_base(o);
            a.add.waddr = m;
            a.add.magic = true;
            f(Instr71::Alu(a));
        }
    }
}

fn branch_forms(f: &mut dyn FnMut(Instr71)) {
    for &cond in &BranchCond::ALL {
        for &msfign in &Msfign::ALL {
            for &bdi in &BranchDest::ALL {
                for ub in [false, true] {
                    for &bdu in &BranchDest::ALL {
                        for raddr_a in [0u8, 5, 63, 64] {
                            for offset in [0u32, 8, 16, 0xffff_fff8, 0x8000_0000, 4, 12] {
                                f(Instr71::Branch(Branch { cond, msfign, bdi, ub, bdu, raddr_a, offset }));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn run(forms: &[&dyn Fn(&mut dyn FnMut(Instr71))]) -> Tally {
    let c = corpus();
    let mut t = Tally::default();
    for form in forms {
        form(&mut |i| check(&mut t, &c, i));
    }
    t
}

fn report(what: &str, t: &Tally) {
    eprintln!(
        "{what}: {} formed, {} encoded ({} distinct words, {} in the corpus), refused {:?}",
        t.formed,
        t.ok,
        t.words.len(),
        t.in_corpus,
        t.refused
    );
    for (k, v) in &t.first {
        eprintln!("  first {k}: {v}");
    }
    assert_eq!(t.ok + t.refused.values().sum::<usize>(), t.formed, "{what}: every instruction is accepted or refused");
}

/// The classes of refusal a sweep may raise, by `EncodeError` name. A new
/// name is a codec change to look at, not to wave through.
fn only_refusals(t: &Tally, allowed: &[&str]) {
    for n in t.refused.keys() {
        assert!(allowed.contains(&n.as_str()), "unexpected refusal class {n}: first {}", t.first[n]);
    }
}

#[test]
fn add_ops_with_every_operand_form() {
    let t = run(&[&add_forms]);
    report("add ops", &t);
    only_refusals(
        &t,
        &["MesaRefuses", "MagicWaddrUnnamed", "SmallImmOutOfRange", "MagicNotAllowed", "ReadsBackAs", "DoesNotDecode"],
    );
    assert!(t.ok > 10_000, "the add sweep encodes a population: {}", t.ok);
    assert!(t.refusals("MagicWaddrUnnamed") > 0);
    assert!(t.refusals("SmallImmOutOfRange") > 0);
    assert!(t.refusals("MesaRefuses") > 0);
    assert!(t.in_corpus > 0, "some emitted words are Mesa's");
}

#[test]
fn mul_ops_with_every_operand_form() {
    let t = run(&[&mul_forms]);
    report("mul ops", &t);
    only_refusals(&t, &["MesaRefuses", "MagicWaddrUnnamed", "SmallImmOutOfRange", "ReadsBackAs"]);
    assert!(t.ok > 3_000, "the mul sweep encodes a population: {}", t.ok);
    assert!(t.refusals("MagicWaddrUnnamed") > 0);
    assert!(t.in_corpus > 0);
}

#[test]
fn every_signal_with_an_rf_destination() {
    let t = run(&[&signal_forms]);
    report("signals", &t);
    only_refusals(&t, &["MesaRefuses", "MagicWaddrUnnamed", "SmallImmOutOfRange", "ReadsBackAs"]);
    let busy = |sig: Sig71, addr: u8| {
        let mut a = Alu71::nop();
        a.sig = sig;
        a.sig_addr = addr;
        a.add = AddAlu71 { op: AddOp71::Add, a: op(4), b: op(5), waddr: 6, magic: false, pack: OutPack::None };
        Instr71::Alu(a)
    };
    // The signals the back end emits with an rf destination all encode.
    for (name, sig, writes) in [
        ("ldvary", Sig71 { ldvary: true, ..Sig71::default() }, true),
        ("ldunif", Sig71 { ldunif: true, ..Sig71::default() }, true),
        ("ldunifrf", Sig71 { ldunifrf: true, ..Sig71::default() }, true),
        ("ldunifarf", Sig71 { ldunifarf: true, ..Sig71::default() }, true),
        ("ldtmu", Sig71 { ldtmu: true, ..Sig71::default() }, true),
        ("ldtlb", Sig71 { ldtlb: true, ..Sig71::default() }, true),
        ("ldtlbu", Sig71 { ldtlbu: true, ..Sig71::default() }, true),
        ("thrsw", Sig71 { thrsw: true, ..Sig71::default() }, false),
        ("ucb", Sig71 { ucb: true, ..Sig71::default() }, false),
        ("rotate", Sig71 { rotate: true, ..Sig71::default() }, false),
        ("wrtmuc", Sig71 { wrtmuc: true, ..Sig71::default() }, false),
        ("ldunifa", Sig71 { ldunifa: true, ..Sig71::default() }, false),
        ("ldvary+thrsw", Sig71 { ldvary: true, thrsw: true, ..Sig71::default() }, true),
        ("ldunifrf+thrsw", Sig71 { ldunifrf: true, thrsw: true, ..Sig71::default() }, true),
        ("ldtmu+thrsw", Sig71 { ldtmu: true, thrsw: true, ..Sig71::default() }, true),
    ] {
        for rf in [0u8, 1, 17, 63] {
            let i = busy(sig, if writes { rf } else { 0 });
            match encode71(&i) {
                Ok(w) => {
                    let back = decode71(w).unwrap();
                    assert!(same_instr71(&i, &back), "{name} rf{rf}: {back:?}");
                }
                Err(e) => eprintln!("refused: {name} -> rf{rf}: {e:?}"),
            }
        }
    }
    assert!(t.in_corpus > 0);
}

#[test]
fn conditions_and_flag_updates() {
    let t = run(&[&flag_forms]);
    report("flags", &t);
    only_refusals(&t, &["MesaRefuses", "ReadsBackAs"]);
    assert!(t.ok > 100);
    assert!(t.refusals("MesaRefuses") + t.refusals("ReadsBackAs") > 0);
}

#[test]
fn sfu_as_alu_operations() {
    let t = run(&[&sfu_forms]);
    report("sfu", &t);
    only_refusals(&t, &["MesaRefuses", "MagicWaddrUnnamed", "ReadsBackAs"]);
    assert!(t.ok > 0);
    // 4.x's SFU and accumulator magic addresses are reserved at 7.1: each of
    // the six ops written to 0..4 and 19..24 is refused by name.
    for o in SFU {
        for m in [0u8, 1, 2, 3, 4, 19, 20, 21, 22, 23, 24] {
            let mut a = add_base(o);
            a.add.waddr = m;
            a.add.magic = true;
            assert_eq!(encode71(&Instr71::Alu(a)), Err(E::MagicWaddrUnnamed), "{o:?} to magic {m}");
        }
        for rf in [0u8, 1, 63] {
            let mut a = add_base(o);
            a.add.waddr = rf;
            let w = encode71(&Instr71::Alu(a)).unwrap_or_else(|e| panic!("{o:?} -> rf{rf}: {e:?}"));
            assert!(same_instr71(&Instr71::Alu(a), &decode71(w).unwrap()));
        }
    }
}

#[test]
fn branches() {
    let t = run(&[&branch_forms]);
    report("branches", &t);
    only_refusals(&t, &["RaddrTooWide", "MesaRefuses", "ReadsBackAs", "DoesNotDecode"]);
    assert!(t.ok > 100);
    assert!(t.refusals("RaddrTooWide") > 0);
}

/// The refusals the codec makes by name, one by one (the list the plan's
/// report carries): each is an `Err`, never a word.
/// Which (op, unpack) pairs Mesa packs into a word its own unpack cannot read
/// (`DoesNotDecode`): refused by encode71, never emitted. Printed for the report.
#[test]
fn does_not_decode_classes() {
    let mut seen = BTreeMap::<String, usize>::new();
    for &o in AddOp71::ALL.iter() {
        for &u in &Unpack71::ALL {
            let mut a = add_base(o);
            a.add.a.unpack = u;
            if matches!(encode71(&Instr71::Alu(a)), Err(E::DoesNotDecode(_))) {
                *seen.entry(format!("{o:?}/a={u:?}")).or_insert(0) += 1;
            }
        }
    }
    eprintln!("DoesNotDecode add classes: {seen:?}");
}

#[test]
fn named_refusals() {
    let e = |a: Alu71| encode71(&Instr71::Alu(a));
    let mut a = add_base(AddOp71::Add);
    a.add.a.raddr = 64;
    assert_eq!(e(a), Err(E::RaddrTooWide));
    let mut a = add_base(AddOp71::Add);
    a.add.waddr = 64;
    assert_eq!(e(a), Err(E::WaddrTooWide));
    let mut a = Alu71::nop();
    a.sig.ldvary = true;
    a.sig_addr = 64;
    assert_eq!(e(a), Err(E::SigAddrTooWide));
    // accumulator r0 as a magic destination: reserved on 7.1.
    let mut a = add_base(AddOp71::Add);
    a.add.waddr = 0;
    a.add.magic = true;
    assert_eq!(e(a), Err(E::MagicWaddrUnnamed));
    let mut a = mul_base(MulOp71::Fmul);
    a.mul.waddr = 20;
    a.mul.magic = true;
    assert_eq!(e(a), Err(E::MagicWaddrUnnamed));
    // a small-immediate index past the table, read by the op.
    let mut a = add_base(AddOp71::Add);
    a.add.b.raddr = 63;
    a.sig.small_imm_b = true;
    assert_eq!(e(a), Err(E::SmallImmOutOfRange));
    // LDVPM with the magic bit.
    let mut a = add_base(AddOp71::LdvpmvIn);
    a.add.magic = true;
    assert_eq!(e(a), Err(E::MagicNotAllowed));
    // Both small-immediate flags of the add ALU: Mesa has no signal.
    let mut a = add_base(AddOp71::Add);
    a.sig.small_imm_a = true;
    a.sig.small_imm_b = true;
    assert!(e(a).is_err());
    // Flags beside an address-writing signal: the COND bits are the address.
    let mut a = add_base(AddOp71::Add);
    a.sig.ldvary = true;
    a.sig_addr = 5;
    a.flags.ac = Cond::IfA;
    assert!(e(a).is_err());
    // A signal pair Mesa's v3d71_sig_map has no row for.
    let mut a = Alu71::nop();
    a.sig.ldunif = true;
    a.sig.ldunifrf = true;
    assert!(e(a).is_err());
}

/// Words the emission sweeps share with Mesa's corpus carry Mesa's bits
/// (`check` counted them); the count is kept so a codec that stops reaching
/// Mesa's words shows.
#[test]
fn emission_set_meets_mesas_corpus() {
    let t = run(&[&add_forms, &mul_forms, &signal_forms, &flag_forms, &sfu_forms, &branch_forms]);
    report("all", &t);
    assert!(t.in_corpus >= 50, "the emission set reaches Mesa's words: {}", t.in_corpus);
    // Every corpus word is some instruction encode71 writes back bit for bit.
    let c = corpus();
    for &w in c.iter().step_by(7) {
        let i = decode71(w).expect("corpus word decodes");
        assert_eq!(encode71(&i), Ok(w));
    }
}
