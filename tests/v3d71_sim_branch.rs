// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! sim71 (`sim::run_gen(Gen::V71, ..)`) on branches: delay slots, counted
//! back edges, the uniform stream (`ub`) and the multisample-flag condition
//! (P8: the `if` shape and sched71-cf rely on these).
//!
//! Hand-built programs, encoded with codec71 for 7.1 and with the 4.2
//! encoder for the equivalent 4.2 program (the branch word is the same
//! `Branch` on both: `qpu_pack.c:2460-2480` unpack, `2577-2615` pack, one
//! code path for every ver). Each is run on both interpreters; the TLB
//! writes are what is read, against values computed here and against each
//! other. Programs use only what both generations share (register-file
//! operands, `ldunifrf`, ADD/SUB, SETMSF, the mul MOV to the TLB).
//!
//! The rules relied on (Mesa 26.2.1, `src/broadcom`):
//!
//! * REL target = the branch + 4 instructions + offset / 8; three delay
//!   slots that always execute: `compiler/qpu_schedule.c:2968-2971`
//!   (`offset = (target_ip - (branch_ip + 4)) * sizeof(uint64_t)`), the
//!   slots filled by `emit_branch`; no branch inside the slots,
//!   `compiler/qpu_validate.c:158-162`.
//! * The conditions (`qpu/qpu_instr.h:386-394`): ALLA, ANYA, ANYNA, ALLNA
//!   over the A flags of the lanes; msfign P (`qpu_instr.h:396-410`) ignores
//!   the lanes whose multisample flags are all 0; setmsf is the flag source
//!   (`nir_to_vir.c`, `vir_SETMSF_dest`).
//! * `ub`: the branch reads one word of the uniform stream, taken or not,
//!   and when taken moves the stream to (its own uniform + 1 + word / 4):
//!   `compiler/qpu_schedule.c:2973-2984` (`uniform_data = (successor
//!   start_uniform - (branch_uniform + 1)) * 4`), `qpu/qpu_instr.h:420-432`
//!   (`ub`, `bdu`).

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test"))]

use yakogl::shaderc::online::v3d::codec71::encode71;
use yakogl::shaderc::online::v3d::encode::encode;
use yakogl::shaderc::online::v3d::isa::{
    AddAlu, AddOp, Alu, Branch, BranchCond, BranchDest, Cond, Input, Instr, Msfign, MulOp, Mux, Pf, Unpack,
};
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Alu71, Instr71, MulOp71, Operand};
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, LANES};

// Registers (the payload and the uniforms are loaded into these).
const ZERO: u8 = 11;
const ONE: u8 = 12;
const A0: u8 = 13;
const A1: u8 = 14;
const A2: u8 = 15;
const CNT: u8 = 10;
const FLAG: u8 = 16;
const LIMIT: u8 = 17;
const INC: u8 = 20;
/// The magic TLB address (colour write).
const TLB: u8 = 7;

#[derive(Clone, Copy)]
enum Op {
    Nop,
    /// ldunifrf into a register.
    Unif(u8),
    Add(u8, u8, u8, Pf, Cond),
    Sub(u8, u8, u8, Pf, Cond),
    /// The one integer op whose Z flag sim describes (sim.rs: XOR).
    Xor(u8, u8, u8, Pf, Cond),
    Setmsf(u8),
    Tlb(u8),
    /// Branch to a label (an index into the labels), REL.
    Br(BranchCond, Msfign, bool, usize),
}

/// A program: ops and the position of each label (an op index).
struct Prog {
    ops: Vec<Op>,
    labels: Vec<usize>,
}

fn branch_word(cond: BranchCond, msfign: Msfign, ub: bool, offset: i32) -> Branch {
    Branch {
        cond,
        msfign,
        bdi: BranchDest::Rel,
        ub,
        bdu: if ub { BranchDest::Rel } else { BranchDest::Abs },
        raddr_a: 0,
        offset: offset as u32,
    }
}

fn rel(p: &Prog, at: usize, label: usize) -> i32 {
    (p.labels[label] as i32 - (at as i32 + 4)) * 8
}

fn flags42(a: &mut Alu, pf: Pf, c: Cond) {
    a.flags.apf = pf;
    a.flags.ac = c;
}

fn emit42(p: &Prog, at: usize, op: Op) -> Instr {
    let mut a = Alu::nop();
    match op {
        Op::Nop => {}
        Op::Unif(r) => {
            a.sig.ldunifrf = true;
            a.sig_addr = r;
        }
        Op::Add(d, x, y, pf, c) | Op::Sub(d, x, y, pf, c) | Op::Xor(d, x, y, pf, c) => {
            a.add = AddAlu {
                op: match op {
                    Op::Sub(..) => AddOp::Sub,
                    Op::Xor(..) => AddOp::Xor,
                    _ => AddOp::Add,
                },
                a: Input { mux: Mux::A, unpack: Unpack::None },
                b: Input { mux: Mux::B, unpack: Unpack::None },
                waddr: d,
                magic: false,
                pack: a.add.pack,
            };
            a.raddr_a = x;
            a.raddr_b = y;
            flags42(&mut a, pf, c);
        }
        Op::Setmsf(x) => {
            a.add.op = AddOp::Setmsf;
            a.add.a = Input { mux: Mux::A, unpack: Unpack::None };
            a.raddr_a = x;
        }
        Op::Tlb(x) => {
            a.mul.op = MulOp::Mov;
            a.mul.a = Input { mux: Mux::A, unpack: Unpack::None };
            a.mul.waddr = TLB;
            a.mul.magic = true;
            a.raddr_a = x;
        }
        Op::Br(c, m, ub, l) => return Instr::Branch(branch_word(c, m, ub, rel(p, at, l))),
    }
    Instr::Alu(a)
}

fn emit71(p: &Prog, at: usize, op: Op) -> Instr71 {
    let mut a = Alu71::nop();
    let o = |r: u8| Operand { raddr: r, ..Operand::default() };
    match op {
        Op::Nop => {}
        Op::Unif(r) => {
            a.sig.ldunifrf = true;
            a.sig_addr = r;
        }
        Op::Add(d, x, y, pf, c) | Op::Sub(d, x, y, pf, c) | Op::Xor(d, x, y, pf, c) => {
            a.add.op = match op {
                Op::Sub(..) => AddOp71::Sub,
                Op::Xor(..) => AddOp71::Xor,
                _ => AddOp71::Add,
            };
            a.add.a = o(x);
            a.add.b = o(y);
            a.add.waddr = d;
            a.add.magic = false;
            a.flags.apf = pf;
            a.flags.ac = c;
        }
        Op::Setmsf(x) => {
            a.add.op = AddOp71::Setmsf;
            a.add.a = o(x);
        }
        Op::Tlb(x) => {
            a.mul.op = MulOp71::Mov;
            a.mul.a = o(x);
            a.mul.waddr = TLB;
            a.mul.magic = true;
        }
        Op::Br(c, m, ub, l) => return Instr71::Branch(branch_word(c, m, ub, rel(p, at, l))),
    }
    Instr71::Alu(a)
}

/// The words of `p` in `gen`, wrapped like sim.rs's own tests do: the
/// last-section THRSW pair, the body, then the program end (a THRSW and its
/// two delay slots).
fn words(p: &Prog, gen: Gen) -> Vec<u64> {
    let mut out = Vec::new();
    let mut thrsw = Alu::nop();
    thrsw.sig.thrsw = true;
    let mut thrsw71 = Alu71::nop();
    thrsw71.sig.thrsw = true;
    let w = |gen: Gen, i42: Instr, i71: Instr71| match gen {
        Gen::V42 => encode(&i42).expect("4.2 encodes"),
        _ => encode71(&i71).expect("7.1 encodes"),
    };
    for _ in 0..2 {
        out.push(w(gen, Instr::Alu(thrsw), Instr71::Alu(thrsw71)));
    }
    // Labels and branches are relative to the body: the offset is the same
    // after a fixed-length prefix.
    for (at, op) in p.ops.iter().enumerate() {
        out.push(w(gen, emit42(p, at, *op), emit71(p, at, *op)));
    }
    out.push(w(gen, Instr::Alu(thrsw), Instr71::Alu(thrsw71)));
    out.push(w(gen, Instr::Alu(Alu::nop()), Instr71::Alu(Alu71::nop())));
    out.push(w(gen, Instr::Alu(Alu::nop()), Instr71::Alu(Alu71::nop())));
    out
}

type Tlb = Vec<[Option<u32>; LANES]>;

fn run(p: &Prog, gen: Gen, w: [u32; LANES], z: [u32; LANES], uniforms: &[u32]) -> Result<Outcome, String> {
    // W is rf0 on 4.2 and rf3 on 7.1, Z rf2: copy W into CNT first (the
    // program's own first op does it, see `prologue`).
    let one = [1.0f32.to_bits(); LANES];
    let inputs = FsInputs { w, w_centroid: one, z, x: one, y: one, back_facing: [false; LANES], varyings: Vec::new() };
    sim::run_gen(gen, &words(p, gen), uniforms, &Inputs::Fragment(Box::new(inputs)), &Env::default())
}

/// The payload register of `gen` (`sim71::PAYLOAD_W`; rf0 on 4.2).
fn payload(gen: Gen) -> u8 {
    match gen {
        Gen::V42 => 0,
        _ => 3,
    }
}

/// Run on both generations; the TLB writes must be equal. `build` gets the
/// payload register of the generation.
fn both(build: impl Fn(u8) -> Prog, w: [u32; LANES], z: [u32; LANES], uniforms: &[u32]) -> Tlb {
    let mut got: Vec<Tlb> = Vec::new();
    for gen in [Gen::V42, Gen::V71] {
        let p = build(payload(gen));
        let o = run(&p, gen, w, z, uniforms).unwrap_or_else(|e| panic!("{gen:?}: {e}"));
        got.push(o.tlb.iter().map(|t| t.value).collect());
    }
    assert_eq!(got[0], got[1], "4.2 and 7.1 disagree");
    got.pop().unwrap()
}

fn lanes(f: impl Fn(usize) -> u32) -> [u32; LANES] {
    let mut a = [0; LANES];
    for (i, v) in a.iter_mut().enumerate() {
        *v = f(i);
    }
    a
}

fn all(v: u32) -> [Option<u32>; LANES] {
    [Some(v); LANES]
}

fn per(f: impl Fn(usize) -> u32) -> [Option<u32>; LANES] {
    let mut a = [None; LANES];
    for (i, v) in a.iter_mut().enumerate() {
        *v = Some(f(i));
    }
    a
}

const N: Cond = Cond::None;
const NP: Pf = Pf::None;

/// ZERO = 0, ONE = 1 and LIMIT from the first three uniform words, CNT = the
/// payload, the three accumulators A0..A2 cleared. Ends with a nop so the
/// loads have landed.
fn prologue(pw: u8) -> Vec<Op> {
    vec![
        Op::Unif(ZERO),
        Op::Nop,
        Op::Unif(ONE),
        Op::Nop,
        Op::Unif(LIMIT),
        Op::Nop,
        Op::Add(CNT, pw, ZERO, NP, N),
        Op::Add(A0, ZERO, ZERO, NP, N),
        Op::Add(A1, ZERO, ZERO, NP, N),
        Op::Add(A2, ZERO, ZERO, NP, N),
    ]
}

const U: [u32; 3] = [0, 1, LIM];
/// The third uniform word: the bound of the C-flag loops.
const LIM: u32 = 6;

// ---------------------------------------------------------------------------
// (a) forward branches and their delay slots.
// ---------------------------------------------------------------------------

/// `br cond -> L; 3 delay slots; skipped (poison) x2; L: tlb`.
fn forward(cond: BranchCond, flag_value: bool) -> impl Fn(u8) -> Prog {
    move |pw| {
        let mut ops = prologue(pw);
        // A = (x ^ y == 0): set everywhere for ZERO ^ ZERO, clear for ONE ^ ZERO.
        let (x, y) = if flag_value { (ZERO, ZERO) } else { (ONE, ZERO) };
        ops.push(Op::Xor(FLAG, x, y, Pf::PushZ, N));
        let br_at = ops.len();
        ops.push(Op::Br(cond, Msfign::None, false, 0));
        // delay slots: A0 = W + 1, A1 = A0 + 1, A2 = A1 + 1
        ops.push(Op::Add(A0, CNT, ONE, NP, N));
        ops.push(Op::Add(A1, A0, ONE, NP, N));
        ops.push(Op::Add(A2, A1, ONE, NP, N));
        // skipped when taken: A2 = 100 + ... (poison)
        ops.push(Op::Add(A2, ONE, ONE, NP, N));
        ops.push(Op::Add(A0, ONE, ONE, NP, N));
        let target = ops.len();
        ops.push(Op::Tlb(A2));
        ops.push(Op::Tlb(A0));
        assert!(br_at + 4 < target);
        Prog { ops, labels: vec![target] }
    }
}

/// A taken branch runs the three delay slots and skips what lies between:
/// A2 = W + 3 (the slots), not 2 (the poison).
#[test]
fn a_taken_forward_branch_runs_its_three_delay_slots_and_skips_the_rest() {
    let w = lanes(|i| 10 + i as u32);
    // ALWAYS, and ANYA / ALLA over flags that are set.
    for cond in [BranchCond::Always, BranchCond::AnyA, BranchCond::AllA, BranchCond::AnyNA] {
        let taken = !matches!(cond, BranchCond::AnyNA);
        let t = both(forward(cond, true), w, [1; LANES], &U);
        if taken {
            assert_eq!(t, vec![per(|i| 10 + i as u32 + 3), per(|i| 10 + i as u32 + 1)], "{:?}", cond as u32);
        } else {
            // Not taken: the poison runs after the slots.
            assert_eq!(t, vec![all(2), all(2)], "{:?}", cond as u32);
        }
    }
}

/// With every A flag clear the ANYA / ALLA branches fall through: the three
/// delay slots and then the skipped code run (the poison wins).
#[test]
fn a_branch_not_taken_still_runs_delay_slots_and_falls_through() {
    let w = lanes(|i| 10 + i as u32);
    for cond in [BranchCond::AnyA, BranchCond::AllA] {
        let t = both(forward(cond, false), w, [1; LANES], &U);
        assert_eq!(t, vec![all(2), all(2)], "{:?}", cond as u32);
    }
    // ALLNA / ANYNA over clear flags are taken.
    for cond in [BranchCond::AllNA, BranchCond::AnyNA] {
        let t = both(forward(cond, false), w, [1; LANES], &U);
        assert_eq!(t, vec![per(|i| 10 + i as u32 + 3), per(|i| 10 + i as u32 + 1)], "{:?}", cond as u32);
    }
}

// ---------------------------------------------------------------------------
// (b) counted back edges.
// ---------------------------------------------------------------------------

/// `L: A0 += 1; step CNT with flags; br cond -> L; delay slot: A1 += 1; 2
/// nops; tlb A0, A1, CNT`. A0 counts the iterations, A1 the delay-slot
/// executions (one per iteration, the exit's too).
fn counted(cond: BranchCond, up: bool, pf: Pf) -> impl Fn(u8) -> Prog {
    move |pw| {
        let mut ops = prologue(pw);
        let l = ops.len();
        ops.push(Op::Add(A0, A0, ONE, NP, N));
        if up {
            // A = CNT < LIMIT (unsigned borrow, the C flag of SUB)
            ops.push(Op::Add(CNT, CNT, ONE, NP, N));
            ops.push(Op::Sub(FLAG, CNT, LIMIT, pf, N));
        } else {
            // A = CNT == 0
            ops.push(Op::Sub(CNT, CNT, ONE, NP, N));
            ops.push(Op::Xor(FLAG, CNT, ZERO, pf, N));
        }
        let br_at = ops.len();
        ops.push(Op::Br(cond, Msfign::None, false, 0));
        ops.push(Op::Add(A1, A1, ONE, NP, N));
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Tlb(A0));
        ops.push(Op::Tlb(A1));
        ops.push(Op::Tlb(CNT));
        assert!(br_at > l);
        Prog { ops, labels: vec![l] }
    }
}

fn counts(f: impl Fn(usize) -> u32) -> [u32; LANES] {
    lanes(f)
}

/// ALLNA loops while every lane is non-zero: min(count) turns (lane 5 is
/// the short one). Lanes below zero would count on for ever under ANYNA, so
/// ANYNA is run on equal counts (it ends when every lane is 0 together).
#[test]
fn a_counted_back_edge_on_the_z_flag_any_versus_all() {
    let c = counts(|i| match i {
        3 => 5,
        5 => 2,
        _ => 3,
    });
    let t = both(counted(BranchCond::AllNA, false, Pf::PushZ), c, [1; LANES], &U);
    assert_eq!(t[0], all(2));
    assert_eq!(t[1], all(2));
    assert_eq!(t[2], per(|i| (c[i] as i32 - 2) as u32));
    let t = both(counted(BranchCond::AnyNA, false, Pf::PushZ), [4; LANES], [1; LANES], &U);
    assert_eq!(t, vec![all(4), all(4), all(0)], "turns, delay slots, CNT");
}

/// ANYA / ALLA on the C flag of SUB (A = CNT < LIMIT), counting up: lane 0
/// needs 4 turns, lane 7 only 2.
#[test]
fn a_counted_back_edge_on_the_c_flag_any_versus_all() {
    let c = counts(|i| match i {
        0 => 2,
        7 => 4,
        _ => 3,
    });
    let t = both(counted(BranchCond::AnyA, true, Pf::PushC), c, [1; LANES], &U);
    assert_eq!(t[0], all(4), "while any lane is below the limit");
    assert_eq!(t[1], all(4));
    assert_eq!(t[2], per(|i| c[i] + 4));
    let t = both(counted(BranchCond::AllA, true, Pf::PushC), c, [1; LANES], &U);
    assert_eq!(t[0], all(2), "while every lane is below the limit");
    assert_eq!(t[2], per(|i| c[i] + 2));
}

/// One turn: the branch is not taken the first time; the delay slot ran.
#[test]
fn a_loop_that_runs_once_falls_through_with_its_delay_slot_done() {
    let t = both(counted(BranchCond::AnyNA, false, Pf::PushZ), [1; LANES], [1; LANES], &U);
    assert_eq!(t, vec![all(1), all(1), all(0)]);
}

// ---------------------------------------------------------------------------
// (c) ub: the uniform stream moves with the branch.
// ---------------------------------------------------------------------------

/// `br ub +off words; 3 slots; skipped: ldunifrf; L: ldunifrf INC; tlb INC`.
fn ub_forward(cond: BranchCond) -> impl Fn(u8) -> Prog {
    move |pw| {
        let mut ops = prologue(pw);
        // flags: A = (1 ^ 0 == 0): clear, so ANYA falls through, ANYNA jumps.
        ops.push(Op::Xor(FLAG, ONE, ZERO, Pf::PushZ, N));
        ops.push(Op::Br(cond, Msfign::None, true, 0));
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Unif(INC)); // skipped when taken
        ops.push(Op::Nop);
        let target = ops.len();
        ops.push(Op::Unif(INC));
        ops.push(Op::Nop);
        ops.push(Op::Tlb(INC));
        Prog { ops, labels: vec![target] }
    }
}

/// The stream is [0, 1, limit, branch word, X, Y, Z]; the branch is the fourth read,
/// so a taken branch with a word of 4 bytes lands on Y (stream index 4) and
/// one of 8 on Z; not taken, the stream goes on at X.
#[test]
fn ub_moves_the_uniform_stream_by_the_branch_word_when_taken() {
    let (x, y, z) = (0x11, 0x22, 0x33);
    for (word, want) in [(4u32, y), (8, z), (0, x)] {
        let t = both(ub_forward(BranchCond::AnyNA), [1; LANES], [1; LANES], &[0, 1, LIM, word, x, y, z]);
        assert_eq!(t, vec![all(want)], "word {word}");
    }
}

#[test]
fn ub_reads_its_word_even_when_not_taken() {
    let (x, y, z) = (0x11, 0x22, 0x33);
    // ANYA over clear flags: not taken, the stream was still advanced past
    // the branch word, whatever it says: the skipped-over ldunifrf runs and
    // reads X, the one at the target reads Y (the tlb write shows the last).
    for word in [4u32, 8] {
        let t = both(ub_forward(BranchCond::AnyA), [1; LANES], [1; LANES], &[0, 1, LIM, word, x, y, z]);
        assert_eq!(t, vec![all(y)], "word {word}");
    }
}

/// A back edge that moves the stream back: each turn re-reads the same
/// increment (stream [0, 1, limit, inc, -8]: the branch is stream index 4, its
/// target index 4 + 1 - 2 = 3).
#[test]
fn ub_back_edge_rereads_the_same_uniform_each_turn() {
    let build = |pw: u8| {
        let mut ops = prologue(pw);
        let l = ops.len();
        ops.push(Op::Unif(INC));
        ops.push(Op::Nop);
        ops.push(Op::Add(A0, A0, INC, NP, N));
        ops.push(Op::Sub(CNT, CNT, ONE, NP, N));
        ops.push(Op::Xor(FLAG, CNT, ZERO, Pf::PushZ, N));
        ops.push(Op::Br(BranchCond::AnyNA, Msfign::None, true, 0));
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Tlb(A0));
        Prog { ops, labels: vec![l] }
    };
    let t = both(build, [4; LANES], [1; LANES], &[0, 1, LIM, 7, (-8i32) as u32]);
    assert_eq!(t, vec![all(7 * 4)]);
}

// ---------------------------------------------------------------------------
// (d) lanes disabled with setmsf, then a branch on the msf condition.
// ---------------------------------------------------------------------------

/// SETMSF from Z, flags = (W == 0), `br ALLNA msfign -> L; slots; poison;
/// L: tlb`. The disabled lanes carry W = 0 (A set); the others W != 0.
fn msf_branch(cond: BranchCond, msfign: Msfign) -> impl Fn(u8) -> Prog {
    move |pw| {
        let mut ops = prologue(pw);
        ops.push(Op::Setmsf(2));
        ops.push(Op::Xor(FLAG, pw, ZERO, Pf::PushZ, N));
        ops.push(Op::Br(cond, msfign, false, 0));
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Add(A2, ONE, ONE, NP, N)); // skipped when taken
        ops.push(Op::Add(A2, A2, A2, NP, N));
        let target = ops.len();
        ops.push(Op::Tlb(A2));
        Prog { ops, labels: vec![target] }
    }
}

/// Z = lane mask (setmsf: 0 disables the lane); W = 0 on the disabled
/// lanes, so A (W == 0) is set exactly there.
fn mask_inputs() -> ([u32; LANES], [u32; LANES]) {
    let z = lanes(|i| u32::from(i % 2 == 1));
    let w = lanes(|i| if i % 2 == 1 { 7 } else { 0 });
    (w, z)
}

/// A2 is 0 unless the poison ran: taken -> 0 (never written: A2 cleared by
/// the prologue), not taken -> (1+1)+(2+2) = 4.
#[test]
fn msfign_p_ignores_the_disabled_lanes_in_the_branch_condition() {
    let (w, z) = mask_inputs();
    // ALLNA: every lane's A clear. Disabled lanes have A set, so without
    // msfign the branch falls through; with P they are ignored and it jumps.
    let t = both(msf_branch(BranchCond::AllNA, Msfign::None), w, z, &U);
    assert_eq!(t, vec![all(4)], "msfign none: not taken");
    let t = both(msf_branch(BranchCond::AllNA, Msfign::P), w, z, &U);
    assert_eq!(t, vec![all(0)], "msfign P: taken");
    // ANYA is the dual: the A set lanes are the disabled ones only.
    let t = both(msf_branch(BranchCond::AnyA, Msfign::None), w, z, &U);
    assert_eq!(t, vec![all(0)], "msfign none: taken");
    let t = both(msf_branch(BranchCond::AnyA, Msfign::P), w, z, &U);
    assert_eq!(t, vec![all(4)], "msfign P: not taken");
}

/// The multisample flags are what setmsf wrote (an enabled lane keeps any
/// non-zero value).
#[test]
fn setmsf_leaves_its_values_in_the_outcome() {
    let (w, z) = mask_inputs();
    for gen in [Gen::V42, Gen::V71] {
        let p = msf_branch(BranchCond::AllNA, Msfign::P)(payload(gen));
        let o = run(&p, gen, w, z, &U).unwrap();
        assert_eq!(o.msf, z, "{gen:?}");
    }
}

/// A back edge on ANYNA with msfign P: a disabled lane that would loop for
/// 100 turns does not hold the loop.
#[test]
fn msfign_p_loop_is_not_held_by_a_disabled_lane() {
    let build = |pw: u8| {
        let mut ops = prologue(pw);
        ops.push(Op::Setmsf(2));
        let l = ops.len();
        ops.push(Op::Add(A0, A0, ONE, NP, N));
        ops.push(Op::Sub(CNT, CNT, ONE, NP, N));
        ops.push(Op::Xor(FLAG, CNT, ZERO, Pf::PushZ, N));
        ops.push(Op::Br(BranchCond::AnyNA, Msfign::P, false, 0));
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Nop);
        ops.push(Op::Tlb(A0));
        Prog { ops, labels: vec![l] }
    };
    let c = counts(|i| if i == 0 { 100 } else { 3 });
    let z = lanes(|i| u32::from(i != 0));
    assert_eq!(both(build, c, z, &U), vec![all(3)]);
}
