// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! codec71's signal decode of all 32 SIG field values against Mesa's
//! `v3d71_sig_map`, and the re-encode of each valid signal set.
//!
//! Features: --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table
//!
//! The table is transcribed from Mesa 26.2.1 src/broadcom/qpu/qpu_pack.c:164-199
//! (`v3d71_sig_map`; the unpack/pack that use it are lines 201-244). A value
//! Mesa leaves reserved (23, 28, 29) unpacks to the all-zero set and
//! `v3d_qpu_sig_unpack` refuses it (qpu_pack.c:211-213).

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa71::{Alu71, Instr71, Sig71};

const SIG_SHIFT: u32 = 53; // qpu_pack.c:50
const SIG_MASK: u64 = 0x1f << SIG_SHIFT; // QPU_MASK(57, 53)

const THRSW: u32 = 1 << 0;
const LDUNIF: u32 = 1 << 1;
const LDUNIFRF: u32 = 1 << 2;
const LDUNIFA: u32 = 1 << 3;
const LDUNIFARF: u32 = 1 << 4;
const LDTMU: u32 = 1 << 5;
const LDVARY: u32 = 1 << 6;
const LDTLB: u32 = 1 << 7;
const LDTLBU: u32 = 1 << 8;
const UCB: u32 = 1 << 9;
const WRTMUC: u32 = 1 << 10;
const SMIMM_A: u32 = 1 << 11;
const SMIMM_B: u32 = 1 << 12;
const SMIMM_C: u32 = 1 << 13;
const SMIMM_D: u32 = 1 << 14;

/// qpu_pack.c:164-199. `None` is a reserved value (a comment in Mesa).
#[rustfmt::skip]
const V3D71_SIG_MAP: [Option<u32>; 32] = [
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

fn bits(s: &Sig71) -> u32 {
    let f = [
        s.thrsw,
        s.ldunif,
        s.ldunifrf,
        s.ldunifa,
        s.ldunifarf,
        s.ldtmu,
        s.ldvary,
        s.ldtlb,
        s.ldtlbu,
        s.ucb,
        s.wrtmuc,
        s.small_imm_a,
        s.small_imm_b,
        s.small_imm_c,
        s.small_imm_d,
    ];
    assert!(!s.rotate, "rotate is not in the 7.1 map");
    f.iter().enumerate().fold(0, |acc, (i, &b)| acc | ((b as u32) << i))
}

fn from_bits(b: u32) -> Sig71 {
    let g = |m: u32| b & m != 0;
    Sig71 {
        thrsw: g(THRSW),
        ldunif: g(LDUNIF),
        ldunifrf: g(LDUNIFRF),
        ldunifa: g(LDUNIFA),
        ldunifarf: g(LDUNIFARF),
        ldtmu: g(LDTMU),
        ldvary: g(LDVARY),
        ldtlb: g(LDTLB),
        ldtlbu: g(LDTLBU),
        ucb: g(UCB),
        rotate: false,
        wrtmuc: g(WRTMUC),
        small_imm_a: g(SMIMM_A),
        small_imm_b: g(SMIMM_B),
        small_imm_c: g(SMIMM_C),
        small_imm_d: g(SMIMM_D),
    }
}

fn nop_word() -> u64 {
    encode71(&Instr71::Alu(Alu71::nop())).expect("the nop encodes")
}

#[test]
fn decode_matches_mesa_for_all_32_values() {
    let base = nop_word() & !SIG_MASK;
    for (v, want) in V3D71_SIG_MAP.iter().enumerate() {
        let w = base | ((v as u64) << SIG_SHIFT);
        match (want, decode71(w)) {
            (Some(b), Ok(Instr71::Alu(a))) => {
                assert_eq!(bits(&a.sig), *b, "sig value {v}: decoded signal set");
            }
            (None, Err(_)) => {}
            (Some(_), r) => panic!("sig value {v}: Mesa accepts it, codec71 gave {r:?}"),
            (None, r) => panic!("sig value {v}: Mesa marks it reserved, codec71 gave {r:?}"),
        }
    }
}

#[test]
fn every_valid_set_reencodes_to_its_value() {
    let base = nop_word() & !SIG_MASK;
    for (v, want) in V3D71_SIG_MAP.iter().enumerate() {
        let Some(b) = want else { continue };
        let mut a = Alu71::nop();
        a.sig = from_bits(*b);
        let w = encode71(&Instr71::Alu(a)).unwrap_or_else(|e| panic!("sig value {v}: {e:?}"));
        assert_eq!((w & SIG_MASK) >> SIG_SHIFT, v as u64, "sig value {v}: SIG field");
        // Nothing else changes for a signal that does not take its address
        // from the COND field.
        if !a.sig.writes_address() {
            assert_eq!(w & !SIG_MASK, base, "sig value {v}: rest of the word");
        }
    }
}

#[test]
fn sets_outside_the_map_are_refused() {
    // Every other combination of the 15 signals has no value in Mesa's map
    // (v3d_qpu_sig_pack finds no entry, qpu_pack.c:235-241).
    let valid: Vec<u32> = V3D71_SIG_MAP.iter().flatten().copied().collect();
    for b in 0..(1u32 << 15) {
        if valid.contains(&b) {
            continue;
        }
        let mut a = Alu71::nop();
        a.sig = from_bits(b);
        if encode71(&Instr71::Alu(a)).is_ok() {
            panic!("codec71 encodes the signal set {b:#x}, which Mesa's 7.1 map lacks");
        }
    }
}
