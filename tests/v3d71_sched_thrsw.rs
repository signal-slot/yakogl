// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! sched71's thread switches, the last-thread-switch pair and the program
//! end at V3D 7.1, against Mesa 26.2.1 src/broadcom/compiler/qpu_schedule.c
//! (MIT) at ver >= 71 (plan #365, item sched71-thrsw).
//!
//! Features: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.
//!
//! Each case builds a list of [`Op71`]s and a hand-made [`Allocation71`],
//! packs it with [`pack71`] and asserts word by word where the THRSWs and the
//! operations landed. Every packed program also round-trips through codec71
//! (`encode71`, `decode71`), passes `verify71` and runs in
//! `sim::run_gen(Gen::V71, ..)` without a sim error. For each rule there is a
//! case where it lets an operation into a slot and one where it keeps it out:
//!
//! * no SFU operation in slot 1 or 2, no `ldvary` in slot 2
//!   (`qpu_inst_before_thrsw_valid_in_delay_slot`, qpu_schedule.c:2079-2091);
//! * rf0's flops conflict (`update_scoreboard_for_rf0_flops`, :1692-1745;
//!   :2110-2119);
//! * what from after a THRSW may fill its slots
//!   (`qpu_inst_after_thrsw_valid_in_delay_slot`, :2132-2215);
//! * a THRSW moved back up to three words (`emit_thrsw`, :2250-2330);
//! * the last-thrsw pair (:2365-2377);
//! * the thread-end slots (`qpu_inst_valid_in_thrend_slot`, :1979-2061 at
//!   ver >= 71), with the 4.2-only rules (GFXH-1684's VPMWT, GFXH-1625's
//!   TMUWT, no register-file write in any slot) not applied.
//!
//! A case where sched71 breaks a rule is a defect, written as an `#[ignore]`d
//! case with the expected placement; this file does not change sched71.rs.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_sched_thrsw`.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test"))]

use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::alloc71::{Allocation71, Rf};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa::{small_imm_index, Cond, Pf, Uf};
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Alu71, Instr71, MulOp71};
use yakogl::shaderc::online::v3d::listing71::write_instr71;
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sched71::{pack71, Sched71Error};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, Texture, Varying, LANES};
use yakogl::shaderc::online::v3d::verify71::{is_sfu_op, verify71};
use yakogl::shaderc::online::v3d::vop71::{Dst71, FlagOp71, Op71, Sfu71, Src71, Uniform, V};

/// W in every lane (the FS payload `Src71::Payload(3)` at 7.1).
const W: f32 = 2.0;

// ---------------------------------------------------------------------------
// Building operations.
// ---------------------------------------------------------------------------

fn v(n: u32) -> Src71 {
    Src71::V(V(n))
}
fn imm(x: u32) -> Src71 {
    Src71::SmallImm(small_imm_index(x).expect("a small immediate"))
}
fn ldunif(d: u32, x: f32) -> Op71 {
    Op71::Ldunif { dst: V(d), u: Uniform::constant(x.to_bits()) }
}
fn fadd(d: u32, a: Src71, b: Src71) -> Op71 {
    Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(d)), a, b }
}
fn fmul(d: u32, a: Src71, b: Src71) -> Op71 {
    Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(d)), a, b }
}
/// An integer add of a small immediate: its word can take no THRSW (no
/// `v3d71_sig_map` entry has THRSW with a small-immediate signal,
/// qpu_pack.c:164-197).
fn add_imm(d: u32, a: Src71, x: u32) -> Op71 {
    Op71::Add { op: AddOp71::Add, dst: Dst71::V(V(d)), a, b: imm(x) }
}
/// A TLB colour write through the mul unit.
fn tlb(n: u32) -> Op71 {
    Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(n), b: Src71::None }
}
/// A TLB colour write through the add unit.
fn tlb_add(n: u32) -> Op71 {
    Op71::Add { op: AddOp71::Mov, dst: Dst71::Tlb, a: v(n), b: Src71::None }
}
/// A TMU register write (`Dst71::Magic`): 34 TMUT, 33 TMUS.
fn tmu(waddr: u8, n: u32) -> Op71 {
    Op71::Mul { op: MulOp71::Mov, dst: Dst71::Magic(waddr), a: v(n), b: Src71::None }
}
const TMUT: u8 = 34;
const TMUS: u8 = 33;

// ---------------------------------------------------------------------------
// Packing and running.
// ---------------------------------------------------------------------------

/// A packed program: its words as codec71 decodes them, and the uniform
/// stream.
struct Prog {
    instrs: Vec<Instr71>,
    unif: Vec<u32>,
}

/// Pack `ops` with virtual register `n` in rf`regs[n]`.
fn pack(ops: &[Op71], regs: &[u8]) -> Result<Prog, Sched71Error> {
    let rfs: Vec<Rf> = regs.iter().map(|&r| Rf(r)).collect();
    /* an arena of this test's own: the library's scratch region is one
     * static, and the tests run in parallel */
    let mut buf = vec![0u8; 1 << 16];
    let a = Arena::new(&mut buf);
    let al = Allocation71 { reg: &rfs, rf_end: regs.iter().copied().max().unwrap_or(0) + 1 };
    pack71(ops, &al, &a).map(|p| Prog { instrs: p.instrs.to_vec(), unif: p.uniforms.iter().map(|u| u.data).collect() })
}

fn listing(instrs: &[Instr71]) -> String {
    let mut s = String::new();
    for (k, i) in instrs.iter().enumerate() {
        s.push_str(&format!("{k:3}: "));
        write_instr71(&mut s, i).unwrap();
        s.push('\n');
    }
    s
}

fn alu(i: &Instr71) -> &Alu71 {
    match i {
        Instr71::Alu(a) => a,
        Instr71::Branch(_) => panic!("a branch in straight-line code"),
    }
}

/// The words that carry a THRSW.
fn thrsws(p: &Prog) -> Vec<usize> {
    (0..p.instrs.len()).filter(|&k| alu(&p.instrs[k]).sig.thrsw).collect()
}

/// The first word for which `f` holds.
fn find(p: &Prog, f: impl Fn(&Alu71) -> bool) -> usize {
    p.instrs.iter().position(|i| f(alu(i))).unwrap_or_else(|| panic!("not found\n{}", listing(&p.instrs)))
}

/// The words for which `f` holds.
fn all(p: &Prog, f: impl Fn(&Alu71) -> bool) -> Vec<usize> {
    (0..p.instrs.len()).filter(|&k| f(alu(&p.instrs[k]))).collect()
}

fn fs_inputs(varyings: Vec<Varying>) -> Inputs {
    let f = |x: f32| [x.to_bits(); LANES];
    Inputs::Fragment(Box::new(FsInputs {
        w: f(W),
        w_centroid: f(W),
        z: f(0.5),
        x: f(1.0),
        y: f(1.0),
        back_facing: [false; LANES],
        varyings,
    }))
}

/// Round-trip every word through codec71, verify at 7.1 and run in sim71.
fn check_with(p: &Prog, inputs: &Inputs, env: &Env) -> Outcome {
    let text = listing(&p.instrs);
    let words: Vec<u64> =
        p.instrs.iter().map(|i| encode71(i).unwrap_or_else(|e| panic!("{e:?} encoding {i:?}\n{text}"))).collect();
    for (k, (i, w)) in p.instrs.iter().zip(&words).enumerate() {
        assert_eq!(decode71(*w).expect("decodes"), *i, "word {k}\n{text}");
    }
    verify71(&p.instrs, false).unwrap_or_else(|e| panic!("verify71: {e:?}\n{text}"));
    let o = sim::run_gen(Gen::V71, &words, &p.unif, inputs, env).unwrap_or_else(|e| panic!("sim71: {e}\n{text}"));
    assert_eq!(o.uniforms_read, p.unif.len(), "{text}");
    o
}

/// A fragment program, four threads, the given varyings.
fn check(p: &Prog, varyings: Vec<Varying>) -> Outcome {
    check_with(p, &fs_inputs(varyings), &Env { threads: 4, ..Env::default() })
}

/// Pack and check a fragment program.
fn run(ops: &[Op71], regs: &[u8]) -> (Prog, Outcome) {
    run_vary(ops, regs, Vec::new())
}

fn run_vary(ops: &[Op71], regs: &[u8], varyings: Vec<Varying>) -> (Prog, Outcome) {
    let p = pack(ops, regs).unwrap_or_else(|e| panic!("{e:?}"));
    let o = check(&p, varyings);
    (p, o)
}

/// The one value every lane of TLB write `n` holds.
fn tlb_f(o: &Outcome, n: usize) -> f32 {
    let l = o.tlb[n].value;
    assert!(l.iter().all(|x| *x == l[0]), "{l:?}");
    f32::from_bits(l[0].expect("defined"))
}

fn vary(x: f32, c: f32) -> Varying {
    Varying { vary: [x.to_bits(); LANES], c: [c.to_bits(); LANES] }
}

// ---------------------------------------------------------------------------
// No SFU operation in slot 1 or 2 (qpu_schedule.c:2079-2084, ver >= 71:
// `v3d_qpu_instr_is_sfu`).
// ---------------------------------------------------------------------------

/// Lets in: an SFU operation may be the THRSW's own word (slot 0); keeps out:
/// the THRSW does not move back further, where the SFU would be slot 1 or 2
/// (qpu_schedule.c:2079-2084, through `valid_thrsw_sequence` at :2221-2240).
#[test]
fn an_sfu_op_takes_slot_0_of_a_thrsw_moved_back_onto_it() {
    let ops = [
        ldunif(0, 4.0),
        fadd(1, v(0), v(0)),
        Op71::Sfu { op: Sfu71::Recip, dst: V(2), a: v(1) },
        Op71::Thrsw,
        fadd(3, v(0), v(0)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(2),
        tlb(3),
    ];
    let (p, o) = run(&ops, &[4, 5, 6, 7]);
    let text = listing(&p.instrs);
    let sfu = find(&p, |a| a.add.op == AddOp71::Recip);
    assert_eq!(sfu, 2, "{text}");
    assert_eq!(thrsws(&p)[0], sfu, "the THRSW on the SFU's word, not before it\n{text}");
    assert_eq!(tlb_f(&o, 0), 1.0 / 8.0);
    assert_eq!(tlb_f(&o, 1), 8.0);
}

/// Keeps out: an SFU operation from after a THRSW does not go into its
/// delay slots (qpu_schedule.c:2079-2084 via :2152-2156), while an add from
/// after it does (lets in).
#[test]
fn an_sfu_op_after_a_thrsw_waits_for_its_delay_slots() {
    let ops = [
        ldunif(0, 4.0),
        Op71::Thrsw,
        fadd(1, v(0), v(0)),
        Op71::Sfu { op: Sfu71::Recip, dst: V(2), a: v(0) },
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(1),
        tlb(2),
    ];
    let (p, o) = run(&ops, &[4, 5, 6]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    let add = find(&p, |a| a.add.op == AddOp71::Fadd);
    assert_eq!(add, t + 1, "the add in slot 1\n{text}");
    let sfu = find(&p, |a| a.add.op == AddOp71::Recip);
    assert_eq!(sfu, t + 3, "the SFU after slot 2\n{text}");
    for k in all(&p, |a| is_sfu_op(a.add.op)) {
        for t in thrsws(&p) {
            assert!(!(t + 1..=t + 2).contains(&k), "an SFU op in a THRSW's slot\n{text}");
        }
    }
    assert_eq!(tlb_f(&o, 0), 8.0);
    assert_eq!(tlb_f(&o, 1), 0.25);
}

// ---------------------------------------------------------------------------
// No ldvary in slot 2 (qpu_schedule.c:2086-2091: slot 2 only at ver >= 71,
// any slot but 0 at 4.2).
// ---------------------------------------------------------------------------

/// Lets in: an `ldvary` from after a THRSW goes into its slot 1, which 4.2
/// forbids and 7.1 allows (qpu_schedule.c:2086-2091 via :2152-2156).
#[test]
fn an_ldvary_goes_into_slot_1_at_7_1() {
    let ops = [
        ldunif(0, 0.5),
        Op71::Thrsw,
        Op71::Ldvary { dst: V(1) },
        fmul(2, v(1), Src71::Payload(3)),
        fadd(3, v(2), Src71::LdvaryC),
        fadd(4, v(3), v(0)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(4),
    ];
    let (p, o) = run_vary(&ops, &[4, 5, 6, 7, 8], vec![vary(3.0, 0.25)]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    assert_eq!(find(&p, |a| a.sig.ldvary), t + 1, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 0.5);
}

/// Keeps out: with slot 1's signal taken (an `ldunifrf`), the `ldvary` from
/// after the THRSW skips slot 2 (qpu_schedule.c:2086-2091) and goes to the
/// word after it.
#[test]
fn an_ldvary_stays_out_of_slot_2() {
    let ops = [
        ldunif(0, 0.5),
        Op71::Thrsw,
        ldunif(1, 0.25),
        Op71::Ldvary { dst: V(2) },
        fmul(3, v(2), Src71::Payload(3)),
        fadd(4, v(3), Src71::LdvaryC),
        fadd(5, v(4), v(0)),
        fadd(6, v(5), v(1)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(6),
    ];
    let (p, o) = run_vary(&ops, &[4, 5, 6, 7, 8, 9, 10], vec![vary(3.0, 0.25)]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    assert!(alu(&p.instrs[t + 1]).sig.ldunifrf, "slot 1's signal is taken\n{text}");
    assert_eq!(find(&p, |a| a.sig.ldvary), t + 3, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 0.5 + 0.25);
}

/// The same rule moving a THRSW back: an `ldvary` two words before the
/// THRSW's place would be slot 2, so the THRSW goes one word later, where the
/// `ldvary` is slot 1 (keeps out of 2, lets into 1: qpu_schedule.c:2086-2091
/// through `valid_thrsw_sequence`).
#[test]
fn a_thrsw_moved_back_leaves_an_ldvary_in_slot_1_not_2() {
    let ops = [
        ldunif(0, 0.5),
        ldunif(1, 0.25),
        Op71::Ldvary { dst: V(2) },
        Op71::Thrsw,
        fmul(3, v(2), Src71::Payload(3)),
        fadd(4, v(3), Src71::LdvaryC),
        fadd(5, v(4), v(0)),
        fadd(6, v(5), v(1)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(6),
    ];
    let (p, o) = run_vary(&ops, &[4, 5, 6, 7, 8, 9, 10], vec![vary(3.0, 0.25)]);
    let text = listing(&p.instrs);
    // Each ldunifrf names its register: one per word, the ldvary after them.
    assert_eq!(all(&p, |a| a.sig.ldunifrf)[..2], [0, 1], "{text}");
    let ld = find(&p, |a| a.sig.ldvary);
    assert_eq!(ld, 2, "{text}");
    assert_eq!(thrsws(&p)[0], ld - 1, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 0.5 + 0.25);
}

// ---------------------------------------------------------------------------
// rf0's flops conflict (qpu_schedule.c:1692-1745; :2110-2119).
// ---------------------------------------------------------------------------

/// Keeps out: an `ldvary`'s C lands in rf0 in the word of an `ldunifrf` (a
/// signal writing the register file): the conflict is set (:1692-1700,
/// :1739-1744), and no signal writing the register file goes into the
/// THRSW's slot 2 (:2110-2119), or rf0 (C) is undefined after the switch.
#[test]
fn the_rf0_flops_conflict_keeps_a_register_signal_out_of_slot_2() {
    let ops = [
        Op71::Ldvary { dst: V(0) },
        ldunif(1, 1.0),
        Op71::Thrsw,
        ldunif(2, 0.5),
        fmul(3, v(0), Src71::Payload(3)),
        fadd(4, v(3), Src71::LdvaryC),
        fadd(5, v(4), v(1)),
        fadd(6, v(5), v(2)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(6),
    ];
    let (p, o) = run_vary(&ops, &[4, 5, 6, 7, 8, 9, 10], vec![vary(3.0, 0.25)]);
    let text = listing(&p.instrs);
    let ld = find(&p, |a| a.sig.ldvary);
    assert!(alu(&p.instrs[ld + 1]).sig.ldunifrf, "the conflict is set up\n{text}");
    let t = thrsws(&p)[0];
    assert!(t <= ld + 1, "{text}");
    assert!(!alu(&p.instrs[t + 2]).sig.writes_address(), "{text}");
    let second = all(&p, |a| a.sig.ldunifrf)[1];
    assert_eq!(second, t + 3, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 1.0 + 0.5);
}

/// Lets in: a plain `ldunif`'s implicit rf0 write lands in a word with no
/// register-file signal, so there is no conflict, and an `ldunifrf` from
/// after the THRSW takes its slot 2 (:2110-2119 does not apply). rf0 keeps
/// its value across the switch.
#[test]
fn with_no_rf0_flops_conflict_a_register_signal_takes_slot_2() {
    let ops = [
        ldunif(0, 1.0),
        Op71::Thrsw,
        ldunif(1, 0.5),
        ldunif(2, 0.25),
        fadd(3, v(0), v(1)),
        fadd(4, v(3), v(2)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(4),
    ];
    let (p, o) = run(&ops, &[0, 4, 5, 6, 7]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    assert!(alu(&p.instrs[t]).sig.ldunif, "{text}");
    assert_eq!(all(&p, |a| a.sig.ldunifrf), [t + 1, t + 2], "{text}");
    assert_eq!(tlb_f(&o, 0), 1.75);
}

/// Keeps out: an `ldvary` in slot 1 has its C land in slot 2
/// (`last_implicit_rf0_write_tick == tick`, :1739-1742, :2116-2117), so an
/// `ldunifrf` stays out of slot 2.
#[test]
fn an_ldvary_in_slot_1_keeps_a_register_signal_out_of_slot_2() {
    let ops = [
        ldunif(0, 0.5),
        Op71::Thrsw,
        Op71::Ldvary { dst: V(1) },
        ldunif(2, 0.25),
        fmul(3, v(1), Src71::Payload(3)),
        fadd(4, v(3), Src71::LdvaryC),
        fadd(5, v(4), v(0)),
        fadd(6, v(5), v(2)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(6),
    ];
    let (p, o) = run_vary(&ops, &[4, 5, 6, 7, 8, 9, 10], vec![vary(3.0, 0.25)]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    assert_eq!(find(&p, |a| a.sig.ldvary), t + 1, "{text}");
    assert_eq!(all(&p, |a| a.sig.ldunifrf)[1], t + 3, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 0.5 + 0.25);
}

// ---------------------------------------------------------------------------
// What from after a THRSW may fill its slots
// (`qpu_inst_after_thrsw_valid_in_delay_slot`, qpu_schedule.c:2132-2215).
// ---------------------------------------------------------------------------

/// Lets in: ALU operations writing registers, an `ldunif` and a copy fill a
/// THRSW's two slots; at 7.1 there is no accumulator to keep out
/// (`v3d_qpu_writes_accum`, :2190-2194, is false for the register file).
#[test]
fn register_writes_from_after_a_thrsw_fill_its_slots() {
    let ops = [
        ldunif(0, 1.5),
        Op71::Thrsw,
        fadd(1, v(0), v(0)),
        ldunif(2, 0.25),
        Op71::Copy { dst: V(3), src: v(0) },
        fadd(4, v(1), v(2)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(4),
        tlb(3),
    ];
    let (p, o) = run(&ops, &[4, 5, 6, 7, 8]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    assert_eq!(find(&p, |a| a.add.op == AddOp71::Fadd), t + 1, "{text}");
    assert_eq!(all(&p, |a| a.sig.ldunifrf)[1], t + 1, "{text}");
    assert_eq!(all(&p, |a| a.add.op == AddOp71::Fadd)[1], t + 2, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.25);
    assert_eq!(tlb_f(&o, 1), 1.5);
}

/// Keeps out: a TLB write waits for the last-thrsw pair's slots (TLB
/// access, :2156-2160); the add before it fills slot 2 (lets in).
#[test]
fn a_tlb_write_waits_for_the_pairs_slots() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, fadd(1, v(0), v(0)), tlb(1), tlb(0)];
    let (p, o) = run(&ops, &[4, 5]);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    assert_eq!(t[1], t[0] + 1, "{text}");
    assert_eq!(find(&p, |a| a.add.op == AddOp71::Fadd), t[0] + 2, "{text}");
    let tlbs = all(&p, |a| a.mul.magic && a.mul.waddr == 7);
    assert_eq!(tlbs[0], t[0] + 3, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0);
}

/// Keeps out: a flag push from after a THRSW waits for its slots (flags are
/// invalidated across a switch, :2203-2207), while the add before it fills
/// slot 1 (lets in).
#[test]
fn a_flag_push_waits_for_the_thrsw_slots() {
    let ops = [
        ldunif(0, 0.25),
        ldunif(1, 1.0),
        Op71::Thrsw,
        fadd(2, v(0), v(0)),
        Op71::Flags { unit: FlagOp71::Add(AddOp71::Fcmp), a: Src71::Payload(2), b: v(0), pf: Pf::PushN, uf: Uf::None },
        Op71::CondMov { dst: V(1), src: v(2), cond: Cond::IfA },
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(1),
    ];
    let (p, o) = run(&ops, &[4, 5, 6]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[0];
    assert_eq!(find(&p, |a| a.add.op == AddOp71::Fadd), t + 1, "{text}");
    let push = find(&p, |a| a.flags.apf != Pf::None);
    assert_eq!(push, t + 3, "{text}");
    // Z is 0.5 in every lane: not below 0.25, the move does not happen.
    assert_eq!(tlb_f(&o, 0), 1.0);
}

/// Keeps out: a look-up's WRTMUC and TMU writes from after a THRSW wait for
/// its slots (:2166-2181), and so does the LDTMU that waits on the TMU
/// (`v3d_qpu_waits_on_tmu`, :2182-2188). The look-up before the THRSW has
/// its writes in the slots of the THRSW moved back over them (lets in: the
/// before side has no TMU rule).
#[test]
fn tmu_accesses_wait_for_the_thrsw_slots() {
    let ops = [
        ldunif(0, 0.25),
        ldunif(1, 0.75),
        Op71::Wrtmuc { u: Uniform::constant(0x1) },
        Op71::Wrtmuc { u: Uniform::constant(0x1) },
        tmu(TMUT, 0),
        tmu(TMUS, 1),
        Op71::Thrsw,
        Op71::Ldtmu { dst: V(2) },
        Op71::Wrtmuc { u: Uniform::constant(0x1) },
        Op71::Wrtmuc { u: Uniform::constant(0x1) },
        tmu(TMUT, 1),
        tmu(TMUS, 0),
        Op71::Thrsw,
        Op71::Thrsw,
        Op71::Ldtmu { dst: V(3) },
        tlb(2),
        tlb(3),
    ];
    let texels = vec![[10.0, 0.0, 0.0, 1.0], [20.0, 0.0, 0.0, 1.0], [30.0, 0.0, 0.0, 1.0], [40.0, 0.0, 0.0, 1.0]];
    let env = Env { textures: vec![Texture::flat(2, 2, texels)], threads: 4, ..Env::default() };
    let p = pack(&ops, &[4, 5, 6, 7]).unwrap_or_else(|e| panic!("{e:?}"));
    let o = check_with(&p, &fs_inputs(Vec::new()), &env);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    let s = all(&p, |a| a.mul.magic && a.mul.waddr == TMUS);
    assert!(s[0] > t[0] && s[0] <= t[0] + 2, "the first look-up's S write in a slot\n{text}");
    let ld = all(&p, |a| a.sig.ldtmu);
    assert!(ld[0] >= t[0] + 3, "{text}");
    let c = all(&p, |a| a.sig.wrtmuc);
    assert!(c[2] >= t[0] + 3, "{text}");
    assert!(ld[1] >= t[1] + 3, "{text}");
    assert_eq!(o.tmu_lookups, 2);
    assert_eq!(o.ldtmus_without_switch, 0);
    assert_eq!(tlb_f(&o, 0), 20.0); // s 0.75, t 0.25: texel (1, 0)
    assert_eq!(tlb_f(&o, 1), 30.0); // s 0.25, t 0.75: texel (0, 1)
}

// ---------------------------------------------------------------------------
// emit_thrsw: a THRSW moved back up to three words (qpu_schedule.c:2250-2330).
// ---------------------------------------------------------------------------

/// Lets in: a THRSW goes back onto the earliest of the last three words when
/// all three are valid in its slots (:2289-2345); keeps out: never further
/// (`if (++slots_filled == 3) break`, :2335-2336).
#[test]
fn a_thrsw_moves_back_three_words_and_no_further() {
    let ops = [
        ldunif(0, 1.0),
        fadd(1, v(0), v(0)),
        fadd(2, v(1), v(1)),
        fadd(3, v(2), v(2)),
        fadd(4, v(3), v(3)),
        Op71::Thrsw,
        fadd(5, v(4), v(0)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(5),
    ];
    let (p, o) = run(&ops, &[4, 5, 6, 7, 8, 9]);
    let text = listing(&p.instrs);
    let last = find(&p, |a| a.add.op == AddOp71::Fadd && a.add.waddr == 8);
    assert_eq!(last, 4, "{text}");
    assert_eq!(thrsws(&p)[0], last - 2, "{text}");
    assert_eq!(tlb_f(&o, 0), 17.0);
}

/// Keeps out: a word whose signals cannot take a THRSW (a small immediate,
/// no `v3d71_sig_map` entry) is skipped (`v3d_qpu_sig_pack` fails,
/// :2307-2316); lets in: the next word.
#[test]
fn a_thrsw_skips_a_word_whose_signal_cannot_take_it() {
    let ops = [
        ldunif(0, 1.0),
        add_imm(1, v(0), 1),
        fadd(2, v(1), v(1)),
        fadd(3, v(2), v(2)),
        Op71::Thrsw,
        fadd(4, v(3), v(0)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(4),
    ];
    let (p, o) = run(&ops, &[4, 5, 6, 7, 8]);
    let text = listing(&p.instrs);
    let imm_word = find(&p, |a| a.sig.small_imm_b);
    assert_eq!(imm_word, 1, "{text}");
    assert_eq!(thrsws(&p)[0], imm_word + 1, "{text}");
    // 1.0's bits plus 1, doubled twice as a float, plus 1.0.
    let x = f32::from_bits(1.0f32.to_bits() + 1);
    assert_eq!(tlb_f(&o, 0), x * 4.0 + 1.0);
}

/// Keeps out: no THRSW in the delay slots of the previous one, neither moved
/// back into them (:2284-2289) nor placed there (:2265-2271); lets in: a
/// THRSW three words after the previous one.
#[test]
fn a_thrsw_stays_out_of_the_previous_thrsw_slots() {
    let ops = [
        ldunif(0, 1.0),
        Op71::Thrsw,
        fadd(1, v(0), v(0)),
        Op71::Thrsw,
        fadd(2, v(1), v(0)),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb(2),
    ];
    let (p, o) = run(&ops, &[4, 5, 6]);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    assert_eq!(t[1], t[0] + 3, "{text}");
    assert_eq!(t[2], t[1] + 3, "{text}");
    assert_eq!(t[3], t[2] + 1, "{text}");
    assert_eq!(tlb_f(&o, 0), 3.0);
}

// ---------------------------------------------------------------------------
// The last-thrsw pair (qpu_schedule.c:2365-2377).
// ---------------------------------------------------------------------------

/// Lets in: the pair's second THRSW goes into the word after the first, an
/// operation's word (`merge_inst->link.next`, :2373-2376).
#[test]
fn the_pairs_second_thrsw_shares_the_next_word() {
    let ops = [ldunif(0, 1.0), fadd(1, v(0), v(0)), fadd(2, v(1), v(1)), Op71::Thrsw, Op71::Thrsw, tlb(2)];
    let (p, o) = run(&ops, &[4, 5, 6]);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    assert_eq!(t[0], 0, "{text}");
    assert_eq!(t[1], 1, "{text}");
    assert_eq!(alu(&p.instrs[1]).add.op, AddOp71::Fadd, "{text}");
    assert_eq!(tlb_f(&o, 0), 4.0);
}

/// Keeps out: the pair needs two consecutive words that take a THRSW
/// (:2318-2327): the first does not go onto the word before a
/// small-immediate one, though a lone THRSW would (lets in, the second
/// program).
#[test]
fn the_pair_skips_a_word_before_one_that_cannot_take_a_thrsw() {
    let prefix = [ldunif(0, 1.0), add_imm(1, v(0), 1), fadd(2, v(1), v(1))];
    let mut ops = prefix.to_vec();
    ops.extend([Op71::Thrsw, Op71::Thrsw, tlb(2)]);
    let (p, o) = run(&ops, &[4, 5, 6]);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    assert_eq!(find(&p, |a| a.sig.small_imm_b), 1, "{text}");
    assert_eq!(t[0], 2, "{text}");
    assert_eq!(t[1], 3, "{text}");
    let x = f32::from_bits(1.0f32.to_bits() + 1);
    assert_eq!(tlb_f(&o, 0), x * 2.0);

    let mut ops = prefix.to_vec();
    ops.extend([Op71::Thrsw, fadd(3, v(2), v(0)), Op71::Thrsw, Op71::Thrsw, tlb(3)]);
    let (p, o) = run(&ops, &[4, 5, 6, 7]);
    let text = listing(&p.instrs);
    assert_eq!(thrsws(&p)[0], 0, "a lone THRSW goes onto word 0\n{text}");
    assert_eq!(tlb_f(&o, 0), x * 2.0 + 1.0);
}

/// With nothing to move back onto, the pair is two words of their own
/// (`emit_nop` when `slots_filled <= 1`, :2368-2371), and the program end
/// waits for the first one's slots (:2265-2271).
#[test]
fn a_pair_with_nothing_before_it() {
    let ops = [Op71::Thrsw, Op71::Thrsw];
    let p = pack(&ops, &[]).unwrap_or_else(|e| panic!("{e:?}"));
    check(&p, Vec::new());
    let text = listing(&p.instrs);
    assert_eq!(thrsws(&p), [0, 1, 3], "{text}");
    assert_eq!(p.instrs.len(), 6, "{text}");
}

// ---------------------------------------------------------------------------
// The program end (`qpu_inst_valid_in_thrend_slot`, qpu_schedule.c:1979-2061,
// ver >= 71).
// ---------------------------------------------------------------------------

/// Lets in: TLB writes in thread-end slots 0 and 1; keeps out: a TLB write
/// in slot 2 (`is_tlb_z_write`, :1984-1985; sched71 keeps every TLB write
/// out, the words not saying which is Z: stricter than Mesa, not a breach).
#[test]
fn the_end_takes_tlb_writes_in_slots_0_and_1_not_2() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb(0), tlb(0), tlb(0)];
    let (p, o) = run(&ops, &[4]);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    let tlbs = all(&p, |a| a.mul.magic && a.mul.waddr == 7);
    assert_eq!(tlbs, [3, 4, 5], "{text}");
    assert_eq!(t[2], 4, "{text}");
    assert_eq!(p.instrs.len(), 7, "{text}");
    assert_eq!(o.tlb.len(), 3);
}

/// Keeps out: the end does not go onto a word whose signal cannot take a
/// THRSW (a small immediate, :2307-2316 with is_thrend), though the word is
/// valid in every thread-end slot; lets in: the word after it.
#[test]
fn the_end_skips_a_word_whose_signal_cannot_take_a_thrsw() {
    let one = Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: imm(1), b: Src71::None };
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, one, tlb(0)];
    let (p, o) = run(&ops, &[4]);
    let text = listing(&p.instrs);
    assert!(alu(&p.instrs[3]).sig.small_imm_c, "{text}");
    assert_eq!(thrsws(&p)[2], 4, "{text}");
    assert_eq!(p.instrs.len(), 7, "{text}");
    assert_eq!(o.tlb[0].value, [Some(1); LANES]);
    assert_eq!(tlb_f(&o, 1), 1.5);
}

/// Keeps out: the end's own word writes no register through an ALU
/// (:2018-2027); the THRSW goes onto a word of its own.
#[test]
fn the_end_word_writes_no_register_through_an_alu() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb(0), fadd(1, v(0), v(0))];
    let (p, o) = run(&ops, &[4, 5]);
    let text = listing(&p.instrs);
    let w = find(&p, |a| a.add.op == AddOp71::Fadd);
    assert_eq!(w, 3, "the add shares the TLB write's word\n{text}");
    assert_eq!(thrsws(&p)[2], 4, "{text}");
    assert_eq!(p.instrs.len(), 7, "{text}");
    assert_eq!(o.tlb.len(), 1);
}

/// Lets in: the end's word may carry a uniform and a signal writing the
/// register file (4.2 refused that, :2012-2015, not at 7.1), and slot 1 an
/// ALU register write (4.2's "no writing physical registers at the end",
/// :2003-2016, is not applied).
#[test]
fn the_end_takes_a_register_signal_in_slot_0_and_an_alu_write_in_slot_1() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb(0), ldunif(1, 0.5), fadd(2, v(1), v(1))];
    let (p, o) = run(&ops, &[4, 5, 6]);
    let text = listing(&p.instrs);
    let t = thrsws(&p);
    assert_eq!(t[2], 3, "{text}");
    let a = alu(&p.instrs[3]);
    assert!(a.sig.ldunifrf && a.mul.magic && a.mul.waddr == 7, "{text}");
    assert_eq!(find(&p, |a| a.add.op == AddOp71::Fadd), 4, "{text}");
    assert_eq!(p.instrs.len(), 6, "{text}");
    assert_eq!(o.tlb.len(), 1);
}

/// Keeps out: no uniform after the end's word (:1987-1988): the end does not
/// go back onto the word before an `ldunifrf`'s.
#[test]
fn no_uniform_after_the_end_word() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb(0), tlb(0), ldunif(1, 0.5)];
    let (p, o) = run(&ops, &[4, 5]);
    let text = listing(&p.instrs);
    let u = all(&p, |a| a.sig.ldunifrf)[1];
    assert_eq!(u, 4, "{text}");
    assert_eq!(thrsws(&p)[2], u, "{text}");
    assert_eq!(o.tlb.len(), 2);
}

/// Keeps out: no `ldvary` in any thread-end slot (:1993-1994).
#[test]
fn no_ldvary_in_the_end_slots() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb(0), Op71::Ldvary { dst: V(1) }, tlb(0)];
    let (p, o) = run_vary(&ops, &[4, 5], vec![vary(1.0, 0.0)]);
    let text = listing(&p.instrs);
    let ld = find(&p, |a| a.sig.ldvary);
    assert_eq!(ld, 3, "{text}");
    assert_eq!(thrsws(&p)[2], 4, "{text}");
    assert_eq!(o.tlb.len(), 2);
}

/// Keeps out: no thread-end slot reads or writes rf2 or rf3 (:2043-2056);
/// lets in: the same program with the value in rf4 ends on the TLB write.
#[test]
fn no_rf2_or_rf3_in_the_end_slots() {
    let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb(0)];
    for (r, end) in [(2u8, 4usize), (3, 4), (4, 3)] {
        let (p, o) = run(&ops, &[r]);
        let text = listing(&p.instrs);
        assert_eq!(find(&p, |a| a.mul.magic && a.mul.waddr == 7), 3, "rf{r}\n{text}");
        assert_eq!(thrsws(&p)[2], end, "rf{r}\n{text}");
        assert_eq!(tlb_f(&o, 0), 1.5);
    }
    // A write of rf3 keeps the end off word 3 (it would be slot 1) and off
    // word 4 (its own word): the end gets a word of its own. A write of rf5
    // is slot 1 of an end on word 3.
    for (r, end) in [(3u8, 5usize), (5, 3)] {
        let ops = [ldunif(0, 1.5), Op71::Thrsw, Op71::Thrsw, tlb_add(0), tlb_add(0), fmul(1, v(0), v(0))];
        let (p, _) = run(&ops, &[4, r]);
        let text = listing(&p.instrs);
        assert_eq!(find(&p, |a| a.mul.op == MulOp71::Fmul), 4, "rf{r}\n{text}");
        assert_eq!(thrsws(&p)[2], end, "rf{r}\n{text}");
    }
}

/// Lets in (GFXH-1625 is 4.2's, :1997-2001): a TMUWT in the last thread-end
/// slot.
#[test]
fn a_tmuwt_in_the_last_end_slot_at_7_1() {
    let ops = [
        ldunif(0, 1.5),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb_add(0),
        tlb_add(0),
        Op71::Add { op: AddOp71::Tmuwt, dst: Dst71::None, a: Src71::None, b: Src71::None },
    ];
    let (p, o) = run(&ops, &[4]);
    let text = listing(&p.instrs);
    let t = thrsws(&p)[2];
    assert_eq!(find(&p, |a| a.add.op == AddOp71::Tmuwt), t + 2, "{text}");
    assert_eq!(p.instrs.len(), t + 3, "{text}");
    assert_eq!(o.tlb.len(), 2);
}

/// Lets in (GFXH-1684 is 4.2's, `v3d_qpu_waits_vpm` at :1990-1991): a VPMWT
/// in a thread-end slot.
#[test]
fn a_vpmwt_in_an_end_slot_at_7_1() {
    let ops = [
        ldunif(0, 1.5),
        Op71::Thrsw,
        Op71::Thrsw,
        tlb_add(0),
        Op71::Add { op: AddOp71::Vpmwt, dst: Dst71::None, a: Src71::None, b: Src71::None },
    ];
    let (p, o) = run(&ops, &[4]);
    let text = listing(&p.instrs);
    // The add unit is the TLB write's in word 3: the VPMWT is word 4, slot 1
    // of the end on word 3.
    assert_eq!(find(&p, |a| a.add.op == AddOp71::Vpmwt), 4, "{text}");
    assert_eq!(thrsws(&p)[2], 3, "{text}");
    assert_eq!(o.tlb.len(), 1);
}

/// Keeps out: a TMUWT (`v3d_qpu_waits_on_tmu`, :2182-2188) and a MULTOP
/// (:2196-2201) from after a THRSW wait for its slots; the add before them
/// fills slot 1 (lets in).
#[test]
fn a_tmuwt_or_a_multop_waits_for_the_thrsw_slots() {
    let tmuwt = Op71::Add { op: AddOp71::Tmuwt, dst: Dst71::None, a: Src71::None, b: Src71::None };
    let multop = Op71::Mul { op: MulOp71::Multop, dst: Dst71::Nop, a: v(0), b: v(0) };
    for (name, op) in [("tmuwt", tmuwt), ("multop", multop)] {
        let ops = [ldunif(0, 1.5), Op71::Thrsw, fadd(1, v(0), v(0)), op, Op71::Thrsw, Op71::Thrsw, tlb(1)];
        let (p, o) = run(&ops, &[4, 5]);
        let text = listing(&p.instrs);
        let t = thrsws(&p)[0];
        assert_eq!(find(&p, |a| a.add.op == AddOp71::Fadd), t + 1, "{name}\n{text}");
        let w = find(&p, |a| a.add.op == AddOp71::Tmuwt || a.mul.op == MulOp71::Multop);
        assert_eq!(w, t + 3, "{name}\n{text}");
        assert_eq!(tlb_f(&o, 0), 3.0);
    }
}
