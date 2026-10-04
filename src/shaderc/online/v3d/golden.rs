// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Ported from Mesa 26.2.1 src/broadcom/qpu/tests/qpu_disasm.c (test vectors) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Mesa's own QPU test vectors, the ver 42 ones: every `{ 42, word, text }`
//! row of `src/broadcom/qpu/tests/qpu_disasm.c` (Mesa 26.2.1, Copyright (c)
//! 2016 Broadcom, MIT licence), transcribed in order, the duplicate row
//! included. Mesa's test binary, built from the same tree, passes all of them
//! and the 5 ver 71 rows (50 PASS, exit 0).
//!
//! Three statements per row, as Mesa's test makes them: the decode prints as
//! Mesa's text (every field the listing shows), the encode gives the word
//! back, and for FADD/FADDNF/FMIN/FMAX the encode of the swapped operands
//! gives the same word (the operand order is the opcode).

use super::decode::decode;
use super::encode::encode;
use super::isa::*;
use super::listing::write_instr;

/// The ver 42 rows of qpu_disasm.c's `tests[]`, in order. The 5 ver 71 rows
/// (7.1 small immediates and `setnnmode_uu; v8dot`) are not 4.2 and are left
/// out.
pub(super) static MESA_V42: &[(u64, &str)] = &[
    (0x3d003186bb800000, "nop                           ; nop                         ; ldvary.rf0"),
    (0x3c20318105829000, "fadd r1, r1, r5               ; nop                         ; thrsw"),
    (0x3c403186bb81d000, "vpmsetup -, r5                ; nop                         ; ldunif"),
    (0x3c002380b6edb000, "or rf0, r3, r3                ; mov vpm, r3"),
    (0x9c094adef634b000, "ffloor.ifb rf30.l, r3         ; fmul.pushz rf43.l, r5, r1.h"),
    (0xb0044c56ba326840, "flpop rf22, rf33              ; fmul.pushz rf49.l, r4.h, r1.abs"),
    // vfmul input packing
    (0x101e8b6e8aad4000, "fmax.nornn rf46, r4.l, r2.l   ; vfmul.ifnb rf45, r3, r5"),
    (0x1857d3c219825000, "faddnf.norc r2.l, r5.l, r4    ; vfmul.ifb rf15, r0.ll, r4   ; ldunif"),
    (0x1c0a0dfde2294000, "fcmp.ifna rf61.h, r4.abs, r2.l; vfmul rf55, r2.hh, r1"),
    (0x2011c89b402cc000, "fsub.norz rf27, r4.abs, r1.abs; vfmul.ifa rf34, r3.swp, r1"),
    (0xe01b42ab3bb063c0, "vfpack.andnc rf43, rf15.l, r0.h; fmul.ifna rf10.h, r4.l, r5.abs"),
    (0x600b8b87fb4d1000, "fdx.ifnb rf7.h, r1.l          ; fmul.pushn rf46, r3.l, r2.abs"),
    // small immediates
    (0x5de24398bbdc6218, "vflb.andnn rf24               ; fmul rf14, -8, rf8.h"),
    (0x25ef83d8b166f00f, "vfmin.pushn rf24, 15.ff, r5   ; smul24.ifnb rf15, r1, r3"),
    (0xadedcdf70839f990, "faddnf.pushc rf55, -16.l, r3.abs; fmul.ifb rf55.l, rf38.l, r1.h"),
    (0x7dff89fa6a01f020, "fsub.nornc rf58.h, 0x3b800000.l, r3.l; fmul.ifnb rf39, r0.h, r0.h"),
    // branch conditions
    (0x02000006002034c0, "b.anyap  rf19"),
    (0x02679356b4201000, "b.anyap  -1268280496"),
    (0x02b76a2dd0400000, "b.anynaq  zero_addr+0xd0b76a28"),
    (0x0200000500402000, "b.anynaq  lri"),
    (0x0216fe167301c8c0, "bu.anya  zero_addr+0x7316fe10, rf35"),
    (0x020000050040e000, "bu.anynaq  lri, r:unif"),
    (0x0200000300006000, "bu.na0  lri, a:unif"),
    // special waddr names
    (0x3c00318735808000, "vfpack tlb, r0, r1            ; nop"),
    (0xe0571c938e8d5000, "fmax.andc recip, r5.h, r2.l   ; fmul.ifb rf50.h, r3.l, r4.abs; ldunif"),
    (0xc04098d4382c9000, "add.pushn rsqrt, r1, r1       ; fmul rf35.h, r3.abs, r1.abs ; ldunif"),
    (0x481edcd6b3184500, "vfmin.norn log, r4.hh, r0     ; fmul.ifnb rf51, rf20.abs, r0.l"),
    (0x041618d57c453000, "shl.andn exp, r3, r2          ; add.ifb rf35, r1, r2"),
    (0x7048e5da49272800, "fsub.ifa rf26, r2.l, rf32     ; fmul.pushc sin, r1.h, r1.abs; ldunif"),
    // signals
    (0x1f010520cf60a000, "fcmp.andz rf32, r2.h, r1.h    ; vfmul rf20, r0.hh, r3       ; ldunifa"),
    (0x932045e6c16ea000, "fcmp rf38, r2.abs, r5         ; fmul rf23.l, r3, r3.abs     ; ldunifarf.rf1"),
    (0xd72f0434e43ae5c0, "fcmp rf52.h, rf23, r5.abs     ; fmul rf16.h, rf23, r1       ; ldunifarf.rf60"),
    (0xdb3048eb9d533780, "fmax rf43.l, r3.h, rf30       ; fmul rf35.h, r4, r2.l       ; ldunifarf.r1"),
    (0x733620471e6ce700, "faddnf rf7.l, rf28.h, r1.l    ; fmul r1, r3.h, r3.abs       ; ldunifarf.rsqrt2"),
    (0x9c094adef634b000, "ffloor.ifb rf30.l, r3         ; fmul.pushz rf43.l, r5, r1.h"),
    // opcodes
    (0x3de020c7bdfd200d, "ldvpmg_in rf7, r2, r2         ; mov r3, 13"),
    (0x3de02040f8ff7201, "stvpmv 1, rf8                 ; mov r1, 1"),
    (0xd8000e50bb2d3000, "sampid rf16                   ; fmul rf57.h, r3, r1.l"),
    // SFU instructions
    (0xe98d60c1ba2aef80, "recip rf1, rf62               ; fmul r3.h, r2.l, r1.l       ; ldunifrf.rf53"),
    (0x7d87c2debc51c000, "rsqrt rf30, r4                ; fmul rf11, r4.h, r2.h       ; ldunifrf.rf31"),
    (0xb182475abc2bb000, "rsqrt2 rf26, r3               ; fmul rf29.l, r2.h, r1.abs   ; ldunifrf.rf9"),
    (0x79880808bc0b6900, "sin rf8, rf36                 ; fmul rf32, r2.h, r0.l       ; ldunifrf.rf32"),
    (0x04092094bc5a28c0, "exp.ifb rf20, r2              ; add r2, rf35, r2"),
    (0xe00648bfbc32a000, "log rf63, r2                  ; fmul.andnn rf34.h, r4.l, r1.abs"),
    // barrierid
    (0x3c203192bb814000, "barrierid syncb               ; nop                         ; thrsw"),
];

/// The number of ver 42 rows in Mesa 26.2.1's qpu_disasm.c (`grep -c
/// '{ 42, '`): a row added or lost in the transcription fails here.
const MESA_V42_ROWS: usize = 45;

pub(super) fn text_of(i: &Instr) -> String {
    let mut s = String::new();
    write_instr(&mut s, i).expect("a String does not fail");
    s
}

/// The rows whose decode does not print as Mesa's text, one line each.
pub(super) fn listing_mismatches(decode: impl Fn(u64) -> Option<Instr>) -> Vec<String> {
    let mut bad = Vec::new();
    for &(w, want) in MESA_V42 {
        match decode(w) {
            Some(i) => {
                let got = text_of(&i);
                if got != want {
                    bad.push(format!("{w:#018x}: got {got:?}, want {want:?}"));
                }
            }
            None => bad.push(format!("{w:#018x}: does not decode, want {want:?}")),
        }
    }
    bad
}

/// Every row decodes, and prints as Mesa's text.
#[test]
fn every_mesa_v42_vector_decodes_to_mesas_text() {
    assert_eq!(MESA_V42.len(), MESA_V42_ROWS);
    let bad = listing_mismatches(|w| decode(w).ok());
    assert!(bad.is_empty(), "{} of {} vectors differ:\n{}", bad.len(), MESA_V42.len(), bad.join("\n"));
}

/// Every row encodes back to its word, and the four operand-order operations
/// encode their swapped form to the same word.
#[test]
fn every_mesa_v42_vector_encodes_back() {
    let mut swapped = 0;
    for &(w, text) in MESA_V42 {
        let i = decode(w).unwrap_or_else(|e| panic!("{w:#018x} ({text}): {e:?}"));
        assert_eq!(encode(&i), Ok(w), "{w:#018x} ({text})");
        if let Instr::Alu(mut alu) = i {
            if matches!(alu.add.op, AddOp::Fadd | AddOp::Faddnf | AddOp::Fmin | AddOp::Fmax) {
                core::mem::swap(&mut alu.add.a, &mut alu.add.b);
                assert_eq!(encode(&Instr::Alu(alu)), Ok(w), "{w:#018x} ({text}) with a and b swapped");
                swapped += 1;
            }
        }
    }
    // fadd x1, faddnf x3, fmax x3: the swap check ran on something.
    assert_eq!(swapped, 7);
}

/// A few fields read directly, not through the printer, so that a printer
/// defect cannot vouch for a decoder defect.
#[test]
fn mesa_v42_vectors_fields() {
    let Instr::Alu(a) = decode(0x5de24398bbdc6218).unwrap() else { panic!("ALU") };
    assert!(a.sig.small_imm_b);
    assert_eq!(a.mul.a.mux, Mux::B);
    assert_eq!(a.small_imm(), Some(-8i32 as u32));
    assert_eq!(a.mul.b, Input { mux: Mux::A, unpack: Unpack::H });
    assert_eq!(a.raddr_a, 8);
    assert_eq!(a.flags.auf, Uf::AndNN);

    let Instr::Alu(a) = decode(0x7dff89fa6a01f020).unwrap() else { panic!("ALU") };
    assert_eq!(a.small_imm(), Some(0x3b80_0000));
    assert_eq!((a.add.op, a.add.pack, a.add.waddr, a.add.magic), (AddOp::Fsub, OutPack::H, 58, false));

    let Instr::Alu(a) = decode(0x733620471e6ce700).unwrap() else { panic!("ALU") };
    assert!(a.sig.ldunifarf && a.sig.writes_address());
    assert_eq!((a.sig_addr, a.sig_magic), (24, true)); // rsqrt2
    assert_eq!(a.flags, Flags::default());

    let Instr::Branch(b) = decode(0x0216fe167301c8c0).unwrap() else { panic!("branch") };
    assert_eq!(
        b,
        Branch {
            cond: BranchCond::AnyA,
            msfign: Msfign::None,
            bdi: BranchDest::Abs,
            ub: true,
            bdu: BranchDest::Regfile,
            raddr_a: 35,
            offset: 0x7316_fe10,
        }
    );
    let Instr::Branch(b) = decode(0x02679356b4201000).unwrap() else { panic!("branch") };
    assert_eq!((b.bdi, b.offset as i32, b.msfign), (BranchDest::Rel, -1268280496, Msfign::P));
}
