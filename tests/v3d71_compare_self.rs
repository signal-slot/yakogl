// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The 7.1 comparator (`tests/v3d71_compare/mod.rs`) tested on hand-built
//! programs: two equal ones agree; two scheduled differently but equivalent
//! agree and report the count difference; one with a wrong constant
//! disagrees.
//!
//! Run with: --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test"))]

mod v3d71_compare;

use v3d71_compare::{compare, random_fs_cases, Policy, Side};
use yakogl::shaderc::online::v3d::codec71::encode71;
use yakogl::shaderc::online::v3d::isa71::*;

fn thrsw(mut a: Alu71) -> Alu71 {
    a.sig.thrsw = true;
    a
}

fn rf(raddr: u8) -> Operand {
    Operand { raddr, unpack: Unpack71::None }
}

fn add(op: AddOp71, waddr: u8, a: Operand, b: Operand) -> Alu71 {
    let mut i = Alu71::nop();
    i.add = AddAlu71 { op, a, b, waddr, magic: false, pack: OutPack::None };
    i
}

fn mul(op: MulOp71, waddr: u8, a: Operand, b: Operand) -> Alu71 {
    let mut i = Alu71::nop();
    i.mul = MulAlu71 { op, a, b, waddr, magic: false, pack: OutPack::None };
    i
}

/// `mov tlb, rfN` on the mul ALU.
fn to_tlb(r: u8) -> Alu71 {
    let mut i = mul(MulOp71::Mov, 7, rf(r), Operand::default());
    i.mul.magic = true;
    i
}

fn ldunif() -> Alu71 {
    let mut i = Alu71::nop();
    i.sig.ldunif = true;
    i
}

/// The body between the first thread switch pair and the last one, as
/// sim71's own tests wrap it.
fn program(body: &[Alu71], uniforms: &[u32]) -> Side {
    let mut code = vec![thrsw(Alu71::nop()), thrsw(Alu71::nop())];
    code.extend_from_slice(body);
    code.extend([thrsw(Alu71::nop()), Alu71::nop(), Alu71::nop()]);
    Side {
        words: code.iter().map(|a| encode71(&Instr71::Alu(*a)).expect("encodes")).collect(),
        uniforms: uniforms.to_vec(),
    }
}

/// out0 = W * Z + k, out1 = 1 / W (an SFU op); k from the uniform stream.
fn tight(k: f32) -> Side {
    program(
        &[
            ldunif(),
            mul(MulOp71::Fmul, 10, rf(3), rf(2)),
            add(AddOp71::Recip, 11, rf(3), Operand::default()),
            add(AddOp71::Fadd, 12, rf(10), rf(0)),
            to_tlb(12),
            to_tlb(11),
        ],
        &[k.to_bits()],
    )
}

/// The same computation spread out (nops between, other registers).
fn loose(k: f32) -> Side {
    program(
        &[
            ldunif(),
            Alu71::nop(),
            mul(MulOp71::Fmul, 20, rf(3), rf(2)),
            Alu71::nop(),
            add(AddOp71::Fadd, 21, rf(20), rf(0)),
            add(AddOp71::Recip, 22, rf(3), Operand::default()),
            Alu71::nop(),
            to_tlb(21),
            to_tlb(22),
        ],
        &[k.to_bits()],
    )
}

#[test]
fn equal_programs_agree() {
    let cases = random_fs_cases(1, 0, 4);
    let v = compare(&tight(0.75), &tight(0.75), &cases, Policy::default());
    assert!(v.matches(), "{:?}", v.diffs);
    assert_eq!(v.cases_run, 4);
    assert!(v.count_notes.is_empty(), "{:?}", v.count_notes);
    assert_eq!(v.ours, v.mesa);
    assert!(v.passes());
}

#[test]
fn a_different_schedule_agrees_and_the_counts_are_reported() {
    let cases = random_fs_cases(2, 0, 4);
    let v = compare(&loose(0.75), &tight(0.75), &cases, Policy::default());
    assert!(v.matches(), "{:?}", v.diffs);
    assert_eq!(v.ours.instructions, v.mesa.instructions + 3);
    assert_eq!(v.ours.thrsw, 3);
    assert_eq!(v.ours.after_last_thrsw, Some(2));
    assert!(v.count_notes.iter().any(|l| l.starts_with("instructions: ours 14 mesa 11")), "{:?}", v.count_notes);
    assert!(v.passes(), "no policy: counts only report");

    // The policy fails a count worse than Mesa's by more than N percent.
    let strict = Policy { max_worse_percent: Some(10) };
    let v = compare(&loose(0.75), &tight(0.75), &cases, strict);
    assert!(v.matches() && !v.passes());
    assert_eq!(v.policy_failures.len(), 1, "{:?}", v.policy_failures);
    let lax = Policy { max_worse_percent: Some(50) };
    assert!(compare(&loose(0.75), &tight(0.75), &cases, lax).passes());
    // Better than Mesa is never a failure.
    assert!(compare(&tight(0.75), &loose(0.75), &cases, strict).passes());
}

#[test]
fn a_wrong_constant_disagrees() {
    let cases = random_fs_cases(3, 0, 2);
    let v = compare(&tight(0.5), &tight(0.75), &cases, Policy::default());
    assert!(!v.matches());
    assert!(v.diffs.iter().any(|l| l.contains("TLB write 0 lane 0")), "{:?}", v.diffs);
    // The SFU result does not depend on the constant.
    assert!(!v.diffs.iter().any(|l| l.contains("TLB write 1")), "{:?}", v.diffs);
}

#[test]
fn a_run_the_interpreter_refuses_is_not_a_pass() {
    let bad = program(&[add(AddOp71::Vpack, 10, rf(3), rf(2))], &[]);
    let v = compare(&bad, &bad, &random_fs_cases(5, 0, 1), Policy::default());
    assert!(!v.matches());
    assert!(v.diffs.iter().any(|l| l.contains("ours did not run")), "{:?}", v.diffs);
    // A word that does not decode.
    let junk = Side { words: vec![u64::MAX], uniforms: vec![] };
    assert!(!compare(&junk, &bad, &[], Policy::default()).matches());
}
