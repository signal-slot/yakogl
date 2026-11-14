// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 small immediates: the table and the four signals against Mesa.
//!
//! Features: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table` (as `v3d_sim_mesa`).
//!
//! At 7.1 an operand is a small immediate when the instruction's one
//! small-immediate signal says so for that operand: `small_imm_a` = add a
//! (raddr_a), `small_imm_b` = add b (raddr_b), `small_imm_c` = mul a
//! (raddr_c), `small_imm_d` = mul b (raddr_d); the 6-bit raddr indexes Mesa's
//! table. Checked here against Mesa 26.2.1 src/broadcom/qpu/qpu_pack.c:
//!
//! 1. every index of the 48-entry table decodes to Mesa's value, and indices
//!    48..63 are refused;
//! 2. each signal round-trips word for word, on the right unit and slot;
//! 3. two small-immediate signals in one word are refused (Mesa's
//!    `v3d71_sig_map` has no entry for one), and the 32 packed signals never
//!    decode to more than one.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::decode::DecodeError;
use yakogl::shaderc::online::v3d::encode::EncodeError;
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Alu71, Instr71, MulOp71, Operand, Unpack71};

/// Mesa's `small_immediates` (qpu_pack.c:239-264, 48 entries; the
/// `STATIC_ASSERT` at qpu_pack.c:283).
fn mesa_table() -> Vec<u32> {
    let mut t: Vec<u32> = Vec::new();
    // qpu_pack.c:240-243: 0, 1, .. 15.
    t.extend(0..16u32);
    // qpu_pack.c:244-247: -16, -15, .. -1.
    t.extend((-16..0i32).map(|v| v as u32));
    t.extend_from_slice(&MESA_FLOATS);
    t
}

/// qpu_pack.c:248-263, as spelled there.
const MESA_FLOATS: [u32; 16] = [
    0x3b800000, // qpu_pack.c:248 2.0^-8
    0x3c000000, // :249
    0x3c800000, // :250
    0x3d000000, // :251
    0x3d800000, // :252
    0x3e000000, // :253
    0x3e800000, // :254
    0x3f000000, // :255 2.0^-1
    0x3f800000, // :256 2.0^0
    0x40000000, // :257
    0x40800000, // :258
    0x41000000, // :259
    0x41800000, // :260
    0x42000000, // :261
    0x42800000, // :262
    0x43000000, // :263 2.0^7
];

const REG: Operand = Operand { raddr: 3, unpack: Unpack71::None };

fn imm(i: u8) -> Operand {
    Operand { raddr: i, ..Operand::default() }
}

/// An ALU instruction with operand `which` (0..4 = add a, add b, mul a,
/// mul b) a small immediate of index `i` and the other operand of its unit a
/// register.
fn with_imm(which: usize, i: u8) -> Alu71 {
    let mut a = Alu71::nop();
    if which < 2 {
        a.add.op = AddOp71::Add;
        a.add.waddr = 1;
        a.add.magic = false;
        a.add.a = REG;
        a.add.b = REG;
        if which == 0 {
            a.add.a = imm(i);
            a.sig.small_imm_a = true;
        } else {
            a.add.b = imm(i);
            a.sig.small_imm_b = true;
        }
    } else {
        a.mul.op = MulOp71::Add;
        a.mul.waddr = 2;
        a.mul.magic = false;
        a.mul.a = REG;
        a.mul.b = REG;
        if which == 2 {
            a.mul.a = imm(i);
            a.sig.small_imm_c = true;
        } else {
            a.mul.b = imm(i);
            a.sig.small_imm_d = true;
        }
    }
    a
}

fn flags(a: &Alu71) -> [bool; 4] {
    [a.sig.small_imm_a, a.sig.small_imm_b, a.sig.small_imm_c, a.sig.small_imm_d]
}

fn alu(w: u64) -> Alu71 {
    match decode71(w).expect("decodes") {
        Instr71::Alu(a) => a,
        Instr71::Branch(_) => panic!("branch"),
    }
}

#[test]
fn table_matches_mesa() {
    let t = mesa_table();
    assert_eq!(t.len(), 48);
    assert_eq!(t[16], (-16i32) as u32);
    assert_eq!(t[31], u32::MAX);
    for (i, &want) in t.iter().enumerate() {
        assert_eq!(Alu71::small_imm(imm(i as u8)), Some(want), "index {i}");
    }
    // v3d_qpu_small_imm_unpack refuses an index past the array
    // (qpu_pack.c:267-272).
    for i in 48..64u8 {
        assert_eq!(Alu71::small_imm(imm(i)), None, "index {i}");
    }
}

#[test]
fn every_index_round_trips_on_every_signal() {
    let t = mesa_table();
    // The packed signal of each operand (v3d71_sig_map, qpu_pack.c:180, 181,
    // 195, 196) and the shift of its raddr field (qpu_pack.c:87 raddr_c at
    // 18, :104 raddr_d at 12, :107 raddr_a at 6, :110 raddr_b at 0).
    let sig = [14u64, 15, 30, 31];
    let shift = [6u32, 0, 18, 12];
    for which in 0..4 {
        for i in 0..48u8 {
            let a = with_imm(which, i);
            let w = encode71(&Instr71::Alu(a)).unwrap_or_else(|e| panic!("signal {which} index {i}: {e:?}"));
            assert_eq!((w >> 53) & 0x1f, sig[which], "signal {which} index {i}");
            assert_eq!((w >> shift[which]) & 0x3f, i as u64, "raddr field, signal {which} index {i}");
            // Word for word, and the operand on the right unit and slot.
            let d = alu(w);
            assert_eq!(d, a, "signal {which} index {i}");
            assert_eq!(encode71(&Instr71::Alu(d)), Ok(w));
            for (k, f) in flags(&d).iter().enumerate() {
                assert_eq!(*f, k == which, "flag {k}, signal {which}");
            }
            for (k, (o, flag)) in d.operands().iter().enumerate() {
                assert_eq!(*flag, k == which);
                if *flag {
                    assert_eq!(Alu71::small_imm(*o), Some(t[i as usize]));
                }
            }
        }
    }
}

#[test]
fn index_past_the_table_is_refused() {
    for which in 0..4 {
        for i in 48..64u8 {
            let r = encode71(&Instr71::Alu(with_imm(which, i)));
            assert_eq!(r, Err(EncodeError::SmallImmOutOfRange), "signal {which} index {i}");
        }
    }
    // A word with such an index in add b (raddr_b, bits 0..5) decodes to an
    // error too.
    let w = encode71(&Instr71::Alu(with_imm(1, 0))).unwrap();
    assert_eq!(decode71(w | 63).map(|_| ()), Err(DecodeError::SmallImmOutOfRange));
}

#[test]
fn two_small_imm_signals_are_refused() {
    // v3d71_sig_map (qpu_pack.c:164-197) has one SMIMM_x entry per signal:
    // no entry carries two.
    for (p, q) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
        let mut a = with_imm(p, 4);
        let b = with_imm(q, 5);
        if q < 2 {
            a.add = b.add;
        } else {
            a.mul = b.mul;
        }
        a.sig.small_imm_a |= b.sig.small_imm_a;
        a.sig.small_imm_b |= b.sig.small_imm_b;
        a.sig.small_imm_c |= b.sig.small_imm_c;
        a.sig.small_imm_d |= b.sig.small_imm_d;
        assert_eq!(flags(&a).iter().filter(|f| **f).count(), 2);
        assert_eq!(encode71(&Instr71::Alu(a)), Err(EncodeError::MesaRefuses), "signals {p}+{q}");
    }
    // The 32 packed signals: at most one small-immediate flag survives a
    // decode, and exactly the four of the map carry one.
    let base = encode71(&Instr71::Alu(Alu71::nop())).unwrap();
    let mut with_imm_sig = Vec::new();
    for s in 0..32u64 {
        let w = (base & !(0x1fu64 << 53)) | (s << 53);
        if let Ok(Instr71::Alu(a)) = decode71(w) {
            let n = flags(&a).iter().filter(|f| **f).count();
            assert!(n <= 1, "signal {s}");
            if n == 1 {
                with_imm_sig.push(s);
            }
        }
    }
    assert_eq!(with_imm_sig, [14, 15, 30, 31]);
}
