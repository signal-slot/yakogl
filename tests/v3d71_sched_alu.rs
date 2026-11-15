// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! sched71's ALU rules on hand-built programs (plan #365, item
//! sched71-alu). `pack71` merges ALU operations into words by the rules of
//! Mesa 26.2.1's qpu_schedule.c and qpu_validate.c (MIT):
//!
//! * no read-port limit: four raddrs, and two operations merge unless both
//!   use a small immediate (`qpu_merge_raddrs`, qpu_schedule.c:1013-1032);
//! * operands a/b go with an add operation only, c/d with a mul one only
//!   (qpu_validate.c:204-218);
//! * a mul MOV/FMOV moves to a free add unit (`can_do_mul_as_add`,
//!   :1174-1183; `qpu_convert_mul_to_add`, :1200-1233);
//! * a result is readable one instruction later, an SFU result two
//!   (:1870-1871).
//!
//! The programs are [`Op71`] lists with [`Allocation71`]s built by hand (no
//! select71, no alloc71). Each rule has a case where it lets operations
//! merge or move and one where it keeps them apart; the test asserts the
//! word and unit each operation lands in. Every program is verified
//! (`verify71`, `registers_fit_threads71`), round-tripped through codec71
//! and run in sim71 (`sim::run_gen(Gen::V71, ..)`) for its values, and no
//! word of any program holds two small immediates.
//!
//! This file does not change `src/`: a defect is an `#[ignore = "reason"]`
//! case.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_sched_alu`.

#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", feature = "no-shader-table"))]

use std::sync::Mutex;
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::alloc71::{Allocation71, Rf};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa::small_imm_index;
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Alu71, Instr71, MulOp71};
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sched71::pack71;
use yakogl::shaderc::online::v3d::select::Uniform;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::v3d::vop71::{Dst71, Op71, Sfu71, Src71, V};

fn v(n: u32) -> Src71 {
    Src71::V(V(n))
}

fn imm(x: f32) -> Src71 {
    Src71::SmallImm(small_imm_index(x.to_bits()).expect("a small immediate"))
}

fn uni(x: f32) -> Uniform {
    Uniform::constant(x.to_bits())
}

/// `ldunifrf` of `x` into virtual register `n` (allocated to rf4 or above).
fn load(n: u32, x: f32) -> Op71 {
    Op71::Ldunif { dst: V(n), u: uni(x) }
}

fn fadd(d: u32, a: Src71, b: Src71) -> Op71 {
    Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(d)), a, b }
}

fn fmul(d: u32, a: Src71, b: Src71) -> Op71 {
    Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(d)), a, b }
}

fn mul_mov(d: u32, a: Src71) -> Op71 {
    Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(V(d)), a, b: Src71::None }
}

/// The last-thread-switch pair, then each of `outs` written to the TLB, in
/// order: the way a fragment shader reads its results back.
fn finish(mut ops: Vec<Op71>, outs: &[u32]) -> Vec<Op71> {
    ops.extend([Op71::Thrsw, Op71::Thrsw]);
    for &o in outs {
        ops.push(Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(o), b: Src71::None });
    }
    ops
}

fn alu(i: &Instr71) -> &Alu71 {
    match i {
        Instr71::Alu(a) => a,
        Instr71::Branch(_) => panic!("a branch in a straight-line run"),
    }
}

fn smalls(a: &Alu71) -> usize {
    let s = &a.sig;
    [s.small_imm_a, s.small_imm_b, s.small_imm_c, s.small_imm_d].iter().filter(|&&b| b).count()
}

/// A packed and run program.
struct Prog {
    instrs: Vec<Instr71>,
    out: Outcome,
}

impl Prog {
    fn alu(&self, k: usize) -> &Alu71 {
        alu(&self.instrs[k])
    }

    /// The first word whose add unit holds `op`.
    fn add_word(&self, op: AddOp71) -> usize {
        self.instrs.iter().position(|i| alu(i).add.op == op).unwrap_or_else(|| panic!("no {op:?} on an add unit"))
    }

    /// The first word whose mul unit holds `op`.
    fn mul_word(&self, op: MulOp71) -> usize {
        self.instrs.iter().position(|i| alu(i).mul.op == op).unwrap_or_else(|| panic!("no {op:?} on a mul unit"))
    }

    /// Every word whose add unit holds `op`.
    fn add_words(&self, op: AddOp71) -> Vec<usize> {
        (0..self.instrs.len()).filter(|&k| self.alu(k).add.op == op).collect()
    }

    /// The `n`th TLB value, which must be the same in every lane.
    fn tlb(&self, n: usize) -> f32 {
        let l = self.out.tlb[n].value;
        assert!(l.iter().all(|x| *x == l[0]), "{l:?}");
        f32::from_bits(l[0].expect("defined"))
    }

    fn listing(&self) -> String {
        self.instrs.iter().enumerate().map(|(k, i)| format!("{k:3}: {i:?}\n")).collect()
    }
}

/// The scratch region is one per process: the tests take turns.
static SCRATCH: Mutex<()> = Mutex::new(());

fn pack(ops: &[Op71], regs: &[u8]) -> (Vec<Instr71>, Vec<u32>) {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    let rfs: Vec<Rf> = regs.iter().map(|&r| Rf(r)).collect();
    store::with_scratch(|a| {
        let al = Allocation71 { reg: &rfs, rf_end: regs.iter().copied().max().unwrap_or(0) + 1 };
        let p = pack71(ops, &al, a).unwrap_or_else(|e| panic!("pack71: {e:?}"));
        (p.instrs.to_vec(), p.uniforms.iter().map(|u| u.data).collect())
    })
    .expect("scratch")
}

/// Pack, verify, round-trip, run in sim71; no word has two small immediates.
fn run(ops: &[Op71], regs: &[u8]) -> Prog {
    let (instrs, unif) = pack(ops, regs);
    let text: String = instrs.iter().enumerate().map(|(k, i)| format!("{k:3}: {i:?}\n")).collect();
    let words: Vec<u64> = instrs.iter().map(|i| encode71(i).unwrap_or_else(|e| panic!("{e:?}\n{text}"))).collect();
    for (i, w) in instrs.iter().zip(&words) {
        assert_eq!(decode71(*w).expect("decodes"), *i, "{text}");
        assert_eq!(encode71(&decode71(*w).unwrap()).unwrap(), *w, "{text}");
    }
    verify71(&instrs, false).unwrap_or_else(|e| panic!("verify71 {e:?}\n{text}"));
    registers_fit_threads71(&instrs, 4).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
    for (k, i) in instrs.iter().enumerate() {
        assert!(smalls(alu(i)) <= 1, "word {k} holds two small immediates\n{text}");
    }
    let f = |x: f32| [x.to_bits(); LANES];
    let inputs = Inputs::Fragment(Box::new(FsInputs {
        w: f(2.0),
        w_centroid: f(2.0),
        z: f(0.5),
        x: f(1.0),
        y: f(1.0),
        back_facing: [false; LANES],
        varyings: Vec::new(),
    }));
    let env = Env { threads: 4, ..Env::default() };
    let out = sim::run_gen(Gen::V71, &words, &unif, &inputs, &env).unwrap_or_else(|e| panic!("sim71: {e}\n{text}"));
    assert_eq!(out.uniforms_read, unif.len(), "{text}");
    Prog { instrs, out }
}

/// Registers rf4, rf5, ...: one per virtual register.
fn regs(n: u8) -> Vec<u8> {
    (4..4 + n).collect()
}

// ---------------------------------------------------------------------------
// No read-port limit (qpu_merge_raddrs, qpu_schedule.c:1013-1032).
// ---------------------------------------------------------------------------

/// An add and a mul operation with four different register operands share a
/// word: no two-port limit as at 4.2.
#[test]
fn four_register_operands_merge() {
    let ops = finish(
        vec![load(0, 1.0), load(1, 2.0), load(2, 3.0), load(3, 4.0), fadd(4, v(2), v(3)), fmul(5, v(0), v(1))],
        &[4, 5],
    );
    let p = run(&ops, &regs(6));
    let (a, m) = (p.add_word(AddOp71::Fadd), p.mul_word(MulOp71::Fmul));
    assert_eq!(a, m, "{}", p.listing());
    let w = p.alu(a);
    assert_eq!((w.add.a.raddr, w.add.b.raddr), (6, 7));
    assert_eq!((w.mul.a.raddr, w.mul.b.raddr), (4, 5));
    assert_eq!(smalls(w), 0);
    assert_eq!((p.tlb(0), p.tlb(1)), (7.0, 2.0));
}

/// A small immediate with register operands of the other unit merges, in
/// both directions (add b with a mul of registers, mul d with an add of
/// registers).
#[test]
fn one_small_immediate_merges_with_registers() {
    let ops = finish(vec![load(0, 1.5), load(1, 2.0), fadd(2, v(1), imm(4.0)), fmul(3, v(0), v(1))], &[2, 3]);
    let p = run(&ops, &regs(4));
    let (a, m) = (p.add_word(AddOp71::Fadd), p.mul_word(MulOp71::Fmul));
    assert_eq!(a, m, "{}", p.listing());
    let s = p.alu(a).sig;
    assert!(s.small_imm_b && !s.small_imm_a && !s.small_imm_c && !s.small_imm_d);
    assert_eq!((p.tlb(0), p.tlb(1)), (6.0, 3.0));

    let ops = finish(vec![load(0, 1.5), load(1, 2.0), fadd(2, v(1), v(0)), fmul(3, v(1), imm(8.0))], &[2, 3]);
    let p = run(&ops, &regs(4));
    let (a, m) = (p.add_word(AddOp71::Fadd), p.mul_word(MulOp71::Fmul));
    assert_eq!(a, m, "{}", p.listing());
    let s = p.alu(a).sig;
    assert!(s.small_imm_d && !s.small_imm_a && !s.small_imm_b && !s.small_imm_c);
    assert_eq!((p.tlb(0), p.tlb(1)), (3.5, 16.0));
}

/// An add and a mul operation that both use a small immediate (the same
/// value or two, in any slots) never share a word.
#[test]
fn two_small_immediates_never_share_a_word() {
    for (ia, ib) in [(4.0f32, 8.0f32), (4.0, 4.0)] {
        let ops = finish(vec![load(0, 1.0), load(1, 2.0), fadd(2, v(1), imm(ia)), fmul(3, v(0), imm(ib))], &[2, 3]);
        let p = run(&ops, &regs(4));
        let (a, m) = (p.add_word(AddOp71::Fadd), p.mul_word(MulOp71::Fmul));
        assert_ne!(a, m, "{ia} and {ib}\n{}", p.listing());
        assert_eq!(m, a + 1, "{}", p.listing());
        assert_eq!((p.tlb(0), p.tlb(1)), (2.0 + ia, ib));
    }
    // add a and mul c
    let ops = finish(
        vec![
            load(0, 1.0),
            load(1, 2.0),
            Op71::Add { op: AddOp71::Fsub, dst: Dst71::V(V(2)), a: imm(4.0), b: v(1) },
            fmul(3, imm(8.0), v(0)),
        ],
        &[2, 3],
    );
    let p = run(&ops, &regs(4));
    let (a, m) = (p.add_word(AddOp71::Fsub), p.mul_word(MulOp71::Fmul));
    assert_ne!(a, m, "{}", p.listing());
    assert!(p.alu(a).sig.small_imm_a && p.alu(m).sig.small_imm_c);
    assert_eq!((p.tlb(0), p.tlb(1)), (2.0, 8.0));
}

/// Four operations each with a small immediate land in four words, one
/// immediate in each.
#[test]
fn small_immediates_one_per_word() {
    let ops = finish(
        vec![
            load(0, 1.0),
            fadd(1, v(0), imm(2.0)),
            fmul(2, v(0), imm(4.0)),
            fadd(3, v(0), imm(0.5)),
            fmul(4, v(0), imm(16.0)),
        ],
        &[1, 2, 3, 4],
    );
    let p = run(&ops, &regs(5));
    let with_imm = (0..p.instrs.len()).filter(|&k| smalls(p.alu(k)) == 1).count();
    assert_eq!(with_imm, 4, "{}", p.listing());
    assert_eq!((p.tlb(0), p.tlb(1), p.tlb(2), p.tlb(3)), (3.0, 4.0, 1.5, 16.0));
}

// ---------------------------------------------------------------------------
// Operands a/b with the add unit, c/d with the mul unit (qpu_validate.c:204-218).
// ---------------------------------------------------------------------------

/// Each unit reads its own pair: a mul operation's operands are the c/d
/// slots and an add operation's the a/b ones, in one word.
#[test]
fn operands_go_with_their_unit() {
    let ops = finish(vec![load(0, 3.0), load(1, 5.0), load(2, 7.0), fadd(3, v(1), v(2)), fmul(4, v(0), v(1))], &[3, 4]);
    let p = run(&ops, &regs(5));
    let (a, m) = (p.add_word(AddOp71::Fadd), p.mul_word(MulOp71::Fmul));
    assert_eq!(a, m, "{}", p.listing());
    let w = p.alu(a);
    // v0 rf4, v1 rf5, v2 rf6
    assert_eq!((w.add.a.raddr, w.add.b.raddr), (5, 6));
    assert_eq!((w.mul.a.raddr, w.mul.b.raddr), (4, 5));
    assert_eq!((p.tlb(0), p.tlb(1)), (12.0, 15.0));
}

/// Two operations of one unit do not share a word, whichever unit: an add
/// operation never takes the mul unit and an FMUL never takes the add unit.
#[test]
fn one_unit_one_operation_per_word() {
    let ops = finish(
        vec![
            load(0, 1.0),
            load(1, 2.0),
            fadd(2, v(0), v(1)),
            Op71::Add { op: AddOp71::Fsub, dst: Dst71::V(V(3)), a: v(0), b: v(1) },
        ],
        &[2, 3],
    );
    let p = run(&ops, &regs(4));
    let (a, s) = (p.add_word(AddOp71::Fadd), p.add_word(AddOp71::Fsub));
    assert!(s > a, "{}", p.listing());
    assert!((0..p.instrs.len()).all(|k| p.alu(k).mul.op != MulOp71::Fmul), "{}", p.listing());
    assert_eq!((p.tlb(0), p.tlb(1)), (3.0, -1.0));

    let ops = finish(vec![load(0, 2.0), load(1, 3.0), fmul(2, v(0), v(1)), fmul(3, v(1), v(1))], &[2, 3]);
    let p = run(&ops, &regs(4));
    let words = (0..p.instrs.len()).filter(|&k| p.alu(k).mul.op == MulOp71::Fmul).count();
    assert_eq!(words, 2, "{}", p.listing());
    assert!((0..p.instrs.len()).all(|k| p.alu(k).add.op == AddOp71::Nop), "{}", p.listing());
    assert_eq!((p.tlb(0), p.tlb(1)), (6.0, 9.0));
}

// ---------------------------------------------------------------------------
// A mul MOV/FMOV moves to a free add unit (qpu_schedule.c:1174-1233).
// ---------------------------------------------------------------------------

/// With the mul unit taken, a MOV (and an FMOV) takes the add unit of the
/// same word.
#[test]
fn mul_mov_moves_to_the_add_unit() {
    for (mov, add_op) in [(MulOp71::Mov, AddOp71::Mov), (MulOp71::Fmov, AddOp71::Fmov)] {
        let ops = finish(
            vec![
                load(2, 9.0),
                load(0, 2.0),
                load(1, 3.0),
                fmul(3, v(0), v(1)),
                Op71::Mul { op: mov, dst: Dst71::V(V(4)), a: v(2), b: Src71::None },
            ],
            &[3, 4],
        );
        let p = run(&ops, &regs(5));
        let m = p.mul_word(MulOp71::Fmul);
        let w = p.alu(m);
        assert_eq!(w.add.op, add_op, "{mov:?}\n{}", p.listing());
        // v2 is rf6, v4 rf8
        assert_eq!((w.add.a.raddr, w.add.waddr, w.add.magic), (6, 8, false));
        assert_eq!((p.tlb(0), p.tlb(1)), (6.0, 9.0));
    }
}

/// With the mul unit free the MOV stays on it; with the add unit taken as
/// well it cannot move and goes to a later word's mul unit.
#[test]
fn mul_mov_stays_or_waits() {
    let ops = finish(vec![load(0, 2.0), load(1, 3.0), fadd(2, v(0), v(1)), mul_mov(3, v(0))], &[2, 3]);
    let p = run(&ops, &regs(4));
    let a = p.add_word(AddOp71::Fadd);
    assert_eq!(p.mul_word(MulOp71::Mov), a, "{}", p.listing());
    assert_eq!((p.tlb(0), p.tlb(1)), (5.0, 2.0));

    let ops = finish(
        vec![load(0, 2.0), load(1, 3.0), load(2, 9.0), fadd(3, v(0), v(1)), fmul(4, v(0), v(1)), mul_mov(5, v(2))],
        &[3, 4, 5],
    );
    let p = run(&ops, &regs(6));
    let m = p.mul_word(MulOp71::Fmul);
    assert_eq!(p.add_word(AddOp71::Fadd), m, "{}", p.listing());
    // v5 is rf9
    let mv = (0..p.instrs.len()).find(|&k| p.alu(k).mul.op == MulOp71::Mov && p.alu(k).mul.waddr == 9).unwrap();
    assert!(mv > m, "{}", p.listing());
    assert_eq!((p.tlb(0), p.tlb(1), p.tlb(2)), (5.0, 6.0, 9.0));
}

/// Only a MOV/FMOV into a register moves: a MOV that writes no register
/// waits for a word with a free mul unit.
#[test]
fn only_register_movs_move() {
    let ops = finish(
        vec![
            load(0, 2.0),
            load(1, 3.0),
            fmul(2, v(0), v(1)),
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::Nop, a: v(0), b: Src71::None },
        ],
        &[2],
    );
    let p = run(&ops, &regs(3));
    let m = p.mul_word(MulOp71::Fmul);
    assert_eq!(p.alu(m).add.op, AddOp71::Nop, "{}", p.listing());
    assert!(p.instrs.iter().skip(m + 1).any(|i| alu(i).mul.op == MulOp71::Mov), "{}", p.listing());
    assert_eq!(p.tlb(0), 6.0);
}

// ---------------------------------------------------------------------------
// Latency: one instruction, an SFU result two (qpu_schedule.c:1870-1871).
// ---------------------------------------------------------------------------

/// An add result is read in the next word, never in its own.
#[test]
fn alu_result_is_read_one_word_later() {
    let ops = finish(vec![load(0, 1.0), load(1, 2.0), fadd(2, v(0), v(1)), fadd(3, v(2), v(1))], &[3]);
    let p = run(&ops, &regs(4));
    let words = p.add_words(AddOp71::Fadd);
    assert_eq!(words.len(), 2);
    assert_eq!(words[1], words[0] + 1, "{}", p.listing());
    assert_eq!(p.tlb(0), 5.0);
}

/// A mul result is read one word later too, by the other unit.
#[test]
fn mul_result_is_read_one_word_later() {
    let ops = finish(vec![load(0, 2.0), load(1, 3.0), fmul(2, v(0), v(1)), fadd(3, v(2), v(0))], &[3]);
    let p = run(&ops, &regs(4));
    assert_eq!(p.add_word(AddOp71::Fadd), p.mul_word(MulOp71::Fmul) + 1, "{}", p.listing());
    assert_eq!(p.tlb(0), 8.0);
}

/// Each SFU operation's result is read two words after the operation, the
/// one in between left empty when nothing fills it.
#[test]
fn sfu_result_is_read_two_words_later() {
    for (op, x, want) in [
        (Sfu71::Recip, 4.0f32, 0.25f32),
        (Sfu71::Rsqrt, 4.0, 0.5),
        (Sfu71::Exp, 3.0, 8.0),
        (Sfu71::Log, 8.0, 3.0),
        (Sfu71::Sin, 0.25, std::f32::consts::FRAC_1_SQRT_2),
    ] {
        let ops =
            finish(vec![load(0, x), load(1, 2.0), Op71::Sfu { op, dst: V(2), a: v(0) }, fadd(3, v(2), v(1))], &[3]);
        let p = run(&ops, &regs(4));
        let s = p.add_word(op.add_op());
        let c = p.add_word(AddOp71::Fadd);
        assert_eq!(c, s + 2, "{op:?}\n{}", p.listing());
        let got = p.tlb(0);
        assert!((got - (want + 2.0)).abs() < 1e-3, "{op:?}: {got} vs {}", want + 2.0);
    }
}

/// Rsqrt2 is scheduled like the other SFU operations, but sim71 has no model
/// for it (sim.rs's SFU arm names only the five 4.2 operations), so the
/// schedule, verify and codec are checked and the run is left out.
#[test]
fn rsqrt2_result_is_read_two_words_later() {
    let ops = finish(
        vec![load(0, 4.0), load(1, 2.0), Op71::Sfu { op: Sfu71::Rsqrt2, dst: V(2), a: v(0) }, fadd(3, v(2), v(1))],
        &[3],
    );
    let (instrs, _) = pack(&ops, &regs(4));
    let s = instrs.iter().position(|i| alu(i).add.op == AddOp71::Rsqrt2).unwrap();
    let c = instrs.iter().position(|i| alu(i).add.op == AddOp71::Fadd).unwrap();
    assert_eq!(c, s + 2);
    verify71(&instrs, false).expect("verifies");
    for i in &instrs {
        let w = encode71(i).expect("encodes");
        assert_eq!(decode71(w).unwrap(), *i);
    }
}

/// sim71's value of Rsqrt2, once it has one.
#[test]
#[ignore = "sim71 (sim.rs's SFU arm) has no Rsqrt2: the run is refused, so the values cannot be checked"]
fn rsqrt2_value_in_sim71() {
    let ops = finish(
        vec![load(0, 4.0), load(1, 2.0), Op71::Sfu { op: Sfu71::Rsqrt2, dst: V(2), a: v(0) }, fadd(3, v(2), v(1))],
        &[3],
    );
    let p = run(&ops, &regs(4));
    assert!(p.tlb(0).is_finite());
}

/// An SFU shares its word with a mul operation (an add/mul pair), an
/// independent add operation fills the word after it and the reader of the
/// SFU result comes the word after that.
#[test]
fn sfu_pairs_with_mul_and_fills_the_gap() {
    let ops = finish(
        vec![
            load(1, 2.0),
            load(2, 3.0),
            load(0, 4.0),
            Op71::Sfu { op: Sfu71::Recip, dst: V(3), a: v(0) },
            fmul(4, v(1), v(2)),
            fadd(5, v(1), v(2)),
            fadd(6, v(3), v(5)),
        ],
        &[4, 6],
    );
    let p = run(&ops, &regs(7));
    let s = p.add_word(AddOp71::Recip);
    assert_eq!(p.mul_word(MulOp71::Fmul), s, "{}", p.listing());
    assert_eq!(p.add_words(AddOp71::Fadd), vec![s + 1, s + 2], "{}", p.listing());
    assert_eq!((p.tlb(0), p.tlb(1)), (6.0, 5.25));
}

/// A chain of SFU results: each is read two words after the one before it.
#[test]
fn sfu_chain_steps_by_two() {
    let ops = finish(
        vec![
            load(0, 4.0),
            Op71::Sfu { op: Sfu71::Recip, dst: V(1), a: v(0) },
            Op71::Sfu { op: Sfu71::Exp, dst: V(2), a: v(1) },
            Op71::Sfu { op: Sfu71::Rsqrt, dst: V(3), a: v(2) },
        ],
        &[3],
    );
    let p = run(&ops, &regs(4));
    let (r, e, q) = (p.add_word(AddOp71::Recip), p.add_word(AddOp71::Exp), p.add_word(AddOp71::Rsqrt));
    assert_eq!((e, q), (r + 2, r + 4), "{}", p.listing());
    let want = 1.0 / 2.0f32.powf(0.25).sqrt();
    assert!((p.tlb(0) - want).abs() < 1e-3, "{} vs {want}", p.tlb(0));
}

/// An add/mul pair in one word; a reader of both results is in the word
/// after it.
#[test]
fn add_mul_pair_results() {
    let ops = finish(
        vec![load(0, 1.0), load(1, 2.0), load(2, 3.0), fadd(3, v(0), v(2)), fmul(4, v(1), v(2)), fadd(5, v(3), v(4))],
        &[5],
    );
    let p = run(&ops, &regs(6));
    let pair = p.add_word(AddOp71::Fadd);
    assert_eq!(p.mul_word(MulOp71::Fmul), pair, "{}", p.listing());
    let last = *p.add_words(AddOp71::Fadd).last().unwrap();
    assert_eq!(last, pair + 1, "{}", p.listing());
    assert_eq!(p.tlb(0), 10.0);
}
