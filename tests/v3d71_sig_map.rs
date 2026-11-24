// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
//
// The SIG table below is transcribed from Mesa 26.2.1
// src/broadcom/qpu/qpu_pack.c and src/broadcom/qpu/qpu_instr.h (MIT,
// Copyright © 2016 Broadcom).
//! codec71's signal decode of all 32 SIG field values against Mesa's
//! `v3d71_sig_map`, and the re-encode of each signal set the map has.
//!
//! Features: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table` (as `v3d_sim_mesa`).
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_sig_map`.
//!
//! The table is transcribed from Mesa 26.2.1 src/broadcom/qpu/qpu_pack.c:164-197
//! (`v3d71_sig_map`; `v3d_qpu_sig_unpack` is lines 199-215, `v3d_qpu_sig_pack`
//! lines 217-238, the SIG field 57..53 line 50-51). The three values Mesa
//! leaves reserved (23, 28, 29) unpack to the all-zero set and Mesa's
//! `v3d_qpu_instr_unpack` refuses the word that carries one
//! (qpu_pack.c:2414-2417); a signal set with no row in the map is refused by
//! Mesa's ALU pack (`v3d_qpu_sig_pack`, reached at qpu_pack.c:2523-2524),
//! which is what `encode71` reports as `MesaRefuses`. Nothing here changes
//! `codec71.rs`: a defect is a failure to fix, not a pin.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::decode::DecodeError;
use yakogl::shaderc::online::v3d::encode::EncodeError;
use yakogl::shaderc::online::v3d::isa71::{Alu71, Instr71, Sig71};

/// `V3D_QPU_SIG_SHIFT` / `V3D_QPU_SIG_MASK = QPU_MASK(57, 53)` (qpu_pack.c:50-51).
const SIG_SHIFT: u32 = 53;
const SIG_MASK: u64 = 0x1f << SIG_SHIFT;

// The sixteen signals of Mesa's `struct v3d_qpu_sig` (qpu_instr.h:42-59) that
// 7.1 has, in declaration order (`ldvpm` is 4.2 only). One bit each; `bits`
// and `from_bits` below are inverses, and the map lists sets in these bits.
const THRSW: u16 = 1 << 0;
const LDUNIF: u16 = 1 << 1;
const LDUNIFA: u16 = 1 << 2;
const LDUNIFRF: u16 = 1 << 3;
const LDUNIFARF: u16 = 1 << 4;
const LDTMU: u16 = 1 << 5;
const LDVARY: u16 = 1 << 6;
const LDTLB: u16 = 1 << 7;
const LDTLBU: u16 = 1 << 8;
const UCB: u16 = 1 << 9;
const ROTATE: u16 = 1 << 10;
const WRTMUC: u16 = 1 << 11;
const SMIMM_A: u16 = 1 << 12;
const SMIMM_B: u16 = 1 << 13;
const SMIMM_C: u16 = 1 << 14;
const SMIMM_D: u16 = 1 << 15;

/// qpu_pack.c:164-197, one row per SIG field value. `None` is a value Mesa
/// leaves reserved (its map row is all zero, so `v3d_qpu_sig_unpack` returns
/// false for it, qpu_pack.c:212-214). No row of the 7.1 map sets `rotate`,
/// and none sets two of the small-immediate signals.
#[rustfmt::skip]
const V3D71_SIG_MAP: [Option<u16>; 32] = [
    Some(0),                       // [0]
    Some(THRSW),                   // [1]
    Some(LDUNIF),                  // [2]
    Some(THRSW | LDUNIF),          // [3]
    Some(LDTMU),                   // [4]
    Some(THRSW | LDTMU),           // [5]
    Some(LDTMU | LDUNIF),          // [6]
    Some(THRSW | LDTMU | LDUNIF),  // [7]
    Some(LDVARY),                  // [8]
    Some(THRSW | LDVARY),          // [9]
    Some(LDVARY | LDUNIF),         // [10]
    Some(THRSW | LDVARY | LDUNIF), // [11]
    Some(LDUNIFRF),                // [12]
    Some(THRSW | LDUNIFRF),        // [13]
    Some(SMIMM_A),                 // [14]
    Some(SMIMM_B),                 // [15]
    Some(LDTLB),                   // [16]
    Some(LDTLBU),                  // [17]
    Some(WRTMUC),                  // [18]
    Some(THRSW | WRTMUC),          // [19]
    Some(LDVARY | WRTMUC),         // [20]
    Some(THRSW | LDVARY | WRTMUC), // [21]
    Some(UCB),                     // [22]
    None,                          // [23] reserved
    Some(LDUNIFA),                 // [24]
    Some(LDUNIFARF),               // [25]
    Some(LDTMU | WRTMUC),          // [26]
    Some(THRSW | LDTMU | WRTMUC),  // [27]
    None,                          // [28] reserved
    None,                          // [29] reserved
    Some(SMIMM_C),                 // [30]
    Some(SMIMM_D),                 // [31]
];

/// The signal set's bits, as this file's table names them.
fn bits(s: &Sig71) -> u16 {
    let f = [
        s.thrsw,
        s.ldunif,
        s.ldunifa,
        s.ldunifrf,
        s.ldunifarf,
        s.ldtmu,
        s.ldvary,
        s.ldtlb,
        s.ldtlbu,
        s.ucb,
        s.rotate,
        s.wrtmuc,
        s.small_imm_a,
        s.small_imm_b,
        s.small_imm_c,
        s.small_imm_d,
    ];
    f.iter().enumerate().fold(0, |acc, (i, &b)| acc | ((b as u16) << i))
}

fn from_bits(b: u16) -> Sig71 {
    let g = |m: u16| b & m != 0;
    Sig71 {
        thrsw: g(THRSW),
        ldunif: g(LDUNIF),
        ldunifa: g(LDUNIFA),
        ldunifrf: g(LDUNIFRF),
        ldunifarf: g(LDUNIFARF),
        ldtmu: g(LDTMU),
        ldvary: g(LDVARY),
        ldtlb: g(LDTLB),
        ldtlbu: g(LDTLBU),
        ucb: g(UCB),
        rotate: g(ROTATE),
        wrtmuc: g(WRTMUC),
        small_imm_a: g(SMIMM_A),
        small_imm_b: g(SMIMM_B),
        small_imm_c: g(SMIMM_C),
        small_imm_d: g(SMIMM_D),
    }
}

/// `encode71` of the 7.1 NOP: a word whose only nonzero signal-relevant
/// fields are the SIG field (zero) and its NOP magic writes.
fn nop_word() -> u64 {
    encode71(&Instr71::Alu(Alu71::nop())).expect("the NOP encodes")
}

/// Every SIG field value decodes to Mesa's row: the signals of the map, and
/// a refusal for the reserved values.
#[test]
fn decode_matches_mesa_for_all_32_values() {
    let base = nop_word() & !SIG_MASK;
    for (v, want) in V3D71_SIG_MAP.iter().enumerate() {
        let w = base | ((v as u64) << SIG_SHIFT);
        match decode71(w) {
            Ok(Instr71::Alu(a)) => {
                let Some(b) = want else {
                    panic!("SIG value {v}: Mesa's map leaves it reserved, codec71 decoded {:?}", a.sig);
                };
                assert_eq!(bits(&a.sig), *b, "SIG value {v}: decoded signal set");
            }
            Ok(other) => panic!("SIG value {v}: decoded to a non-ALU instruction {other:?}"),
            Err(e) => {
                assert!(want.is_none(), "SIG value {v}: Mesa's map has it, codec71 refused: {e:?}");
                assert_eq!(e, DecodeError::MesaRefuses, "SIG value {v}: refusal");
            }
        }
    }
}

/// Each signal set the map has re-encodes to its value: on the NOP the SIG
/// field alone changes, and no signal is dropped or gained.
#[test]
fn every_mapped_set_reencodes_to_its_value() {
    let base = nop_word() & !SIG_MASK;
    for (v, want) in V3D71_SIG_MAP.iter().enumerate() {
        let Some(b) = want else { continue };
        let mut a = Alu71::nop();
        a.sig = from_bits(*b);
        let w = encode71(&Instr71::Alu(a))
            .unwrap_or_else(|e| panic!("SIG value {v}: Mesa's map has it, encode71 refused: {e:?}"));
        assert_eq!((w & SIG_MASK) >> SIG_SHIFT, v as u64, "SIG value {v}: SIG field");
        assert_eq!(w, base | ((v as u64) << SIG_SHIFT), "SIG value {v}: rest of the word");
        // The word decodes back to the same signal set (encode71's own
        // round trip names a mismatch, so this pins the set itself).
        let Ok(Instr71::Alu(back)) = decode71(w) else {
            panic!("SIG value {v}: its own word no longer decodes");
        };
        assert_eq!(bits(&back.sig), *b, "SIG value {v}: decoded signal set");
    }
}

/// A signal set with no row in Mesa's 7.1 map is refused, and by
/// `MesaRefuses`: it is Mesa's `v3d_qpu_sig_pack` (`memcmp` over the map)
/// that refuses, not one of `encode71`'s own rules. The sweep covers every
/// combination of the sixteen signals that the map's 29 rows leave out, so
/// it also catches a signal `to_mesa71` would drop: a set with a dropped
/// signal encodes as another set, and the drop shows as a set the sweep
/// refuses but Mesa's map has.
#[test]
fn sets_outside_the_map_are_refused() {
    let valid: Vec<u16> = V3D71_SIG_MAP.iter().flatten().copied().collect();
    assert_eq!(valid.len(), 29, "Mesa's 7.1 map has 29 rows");
    let distinct: std::collections::BTreeSet<u16> = valid.iter().copied().collect();
    assert_eq!(distinct.len(), 29, "the transcribed rows are distinct");
    for b in 0..(1u32 << 16) {
        let b = b as u16;
        if valid.contains(&b) {
            continue;
        }
        let mut a = Alu71::nop();
        a.sig = from_bits(b);
        match encode71(&Instr71::Alu(a)) {
            Err(EncodeError::MesaRefuses) => {}
            Err(e) => panic!("the signal set {b:#06x} is refused as {e:?}, not MesaRefuses"),
            Ok(_) => panic!("codec71 encodes the signal set {b:#06x}, which Mesa's 7.1 map lacks"),
        }
    }
}
