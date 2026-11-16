// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The self-test of the shared V3D 7.1 helper (`tests/v3d71_common`): its
//! runner accepts hand-assembled [`Instr71`] programs whose colour is known,
//! and rejects a wrong one. It checks the helper, not a compiler.
//!
//! Feature set: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

mod v3d71_common;

use v3d71_common::*;
use yakogl::shaderc::online::v3d::codec71::encode71;
use yakogl::shaderc::online::v3d::isa::small_imm_index;
use yakogl::shaderc::online::v3d::isa71::*;
use yakogl::shaderc::online::v3d::sim::LANES;

fn rf(raddr: u8) -> Operand {
    Operand { raddr, unpack: Unpack71::None }
}

fn nop() -> Alu71 {
    Alu71::nop()
}

/// `ldunifrf rfN`: the next uniform into a register-file address.
fn ldunifrf(dst: u8) -> Alu71 {
    Alu71 { sig: Sig71 { ldunifrf: true, ..Sig71::default() }, sig_addr: dst, sig_magic: false, ..nop() }
}

/// `vfpack waddr(magic), a, b`.
fn vfpack(waddr: u8, a: Operand, b: Operand) -> Alu71 {
    let mut i = nop();
    i.add = AddAlu71 { op: AddOp71::Vfpack, a, b, waddr, magic: true, pack: OutPack::None };
    i
}

const TLB: u8 = 7;
const TLBU: u8 = 8;

fn thrsw() -> Alu71 {
    Alu71 { sig: Sig71 { thrsw: true, ..Sig71::default() }, ..nop() }
}

fn words(code: &[Alu71]) -> Vec<u64> {
    code.iter().map(|a| encode71(&Instr71::Alu(*a)).expect("encodes")).collect()
}

/// The thread end: THRSW and its two delay slots.
fn end() -> [Alu71; 3] {
    [thrsw(), nop(), nop()]
}

fn expect_all(c: [f32; 4]) -> [[f32; 4]; LANES] {
    [c; LANES]
}

/// Four uniforms packed as f16 pairs to the TLB: the colour is the uniform.
fn uniform_program(swap_gb: bool) -> Vec<u64> {
    let mut code = vec![ldunifrf(10), ldunifrf(11), ldunifrf(12), ldunifrf(13), nop()];
    code.push(vfpack(TLBU, rf(10), rf(11)));
    // The wrong program writes b, a in the other order.
    code.push(if swap_gb { vfpack(TLB, rf(13), rf(12)) } else { vfpack(TLB, rf(12), rf(13)) });
    code.extend(end());
    words(&code)
}

#[test]
fn a_uniform_colour_program_runs() {
    let mut shape = Shape::bare("hand: uniform colour");
    shape.uniforms = vec![0.25, 0.5, 0.75, 1.0];
    shape.expect = expect_all([0.25, 0.5, 0.75, 1.0]);
    let out = run71(&uniform_program(false), &shape).expect("the program is right");
    println!("{}: {} instructions, colour {:?}", shape.name, out.instructions, out.colour[0]);
    assert!(out.discarded.iter().all(|d| !d));
    assert_eq!(out.instructions, 10);
}

#[test]
fn an_alu_program_runs() {
    // r = u0 + u1 (add ALU), g = u0 * u1 (mul ALU, the same instruction),
    // b = u0, a = 1.0 (a small immediate).
    let one = small_imm_index(1.0f32.to_bits()).expect("1.0 is in the small-immediate table");
    let mut both = nop();
    both.add = AddAlu71 { op: AddOp71::Fadd, a: rf(10), b: rf(11), waddr: 12, magic: false, pack: OutPack::None };
    both.mul = MulAlu71 { op: MulOp71::Fmul, a: rf(10), b: rf(11), waddr: 13, magic: false, pack: OutPack::None };
    let mut pack_ba = vfpack(TLB, rf(10), Operand { raddr: one, unpack: Unpack71::None });
    pack_ba.sig.small_imm_b = true;
    let mut code = vec![ldunifrf(10), ldunifrf(11), nop(), both, nop()];
    code.push(vfpack(TLBU, rf(12), rf(13)));
    code.push(pack_ba);
    code.extend(end());
    let mut shape = Shape::bare("hand: alu");
    shape.uniforms = vec![0.25, 0.5];
    shape.expect = expect_all([0.75, 0.125, 0.25, 1.0]);
    let out = run71(&words(&code), &shape).expect("the program is right");
    assert_eq!(out.colour[3], [0.75, 0.125, 0.25, 1.0]);
}

#[test]
fn the_shapes_are_well_formed() {
    let shapes = shapes();
    assert_eq!(shapes.iter().map(|s| s.name).collect::<Vec<_>>(), ["passthrough", "uniform", "if", "texture"]);
    for s in &shapes {
        assert!(s.vs.contains("void main") && s.fs.contains("void main"), "{}: sources", s.name);
        assert!(s.varyings.iter().all(|v| v.len() == LANES));
        assert_eq!(s.stream().len(), s.uniforms.len() + 1);
    }
    // The if shape's lanes split at the uniform.
    let i = if_branch();
    assert_eq!(i.expect[8], IF_ELSE);
    assert_eq!(i.expect[9], IF_THEN);
    assert!(texture().texture.is_some());
}

#[test]
fn a_wrong_program_is_rejected() {
    let mut shape = Shape::bare("hand: wrong");
    shape.uniforms = vec![0.25, 0.5, 0.75, 1.0];
    shape.expect = expect_all([0.25, 0.5, 0.75, 1.0]);
    // b and a swapped: the run succeeds, the colour is not the shape's.
    let e = run71(&uniform_program(true), &shape).expect_err("a swapped colour must be rejected");
    println!("rejected: {e}");
    assert!(e.contains("channel 2"), "{e}");
    // A word codec71 does not take.
    let mut bad = uniform_program(false);
    bad[3] = 0xffff_ffff_ffff_ffff;
    let e = run71(&bad, &shape).expect_err("a word that does not decode must be rejected");
    println!("rejected: {e}");
    assert!(e.contains("word 3"), "{e}");
    // A program that never writes the colour.
    let e = run71(&words(&[nop(), thrsw(), nop(), nop()]), &shape).expect_err("no colour writes");
    println!("rejected: {e}");
    assert!(e.contains("colour TLB writes"), "{e}");
}
