// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Ported from Mesa 26.2.1 src/broadcom/qpu/tests/qpu_disasm.c (test vectors) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! Mesa's own QPU test vectors, the ver 71 ones: every `{ 71, word, text }`
//! row of `src/broadcom/qpu/tests/qpu_disasm.c` (Mesa 26.2.1, Copyright (c)
//! 2016 Broadcom, MIT licence), in order. The 4.2 rows are `golden.rs`.
//!
//! The same three statements per row: the decode prints as Mesa's text, the
//! encode gives the word back, and for FADD/FADDNF/FMIN/FMAX the encode of the
//! swapped operands (with their small-immediate flags) gives the same word.
//! The 5 rows are 7.1's small immediates, one per operand position
//! (`add rf10, 1, rf2` is operand a, `sub rf11, rf3, -2` operand b, the two
//! `fmul`/`smul24` rows operands c and d) and `setnnmode_uu; v8dot`.

use super::codec71::{decode71, encode71};
use super::isa71::*;
use super::listing71::write_instr71;

pub(super) static MESA_V71: &[(u64, &str)] = &[
    (0x39c0000a3803f042, "add rf10, 1, rf2              ; nop"),
    (0x39e0000b3c03f0de, "sub rf11, rf3, -2             ; nop"),
    (0x57c00340bb105000, "nop                           ; fmul rf13, 4, rf5"),
    (0x27e00380bb188000, "nop                           ; smul24 rf14, rf6, 8"),
    (0x2c000300bb042030, "setnnmode_uu                  ; v8dot rf12, rf1, rf2"),
];

/// The number of ver 71 rows in Mesa 26.2.1's qpu_disasm.c (`grep -c
/// '{ 71, '`): a row added or lost in the transcription fails here.
const MESA_V71_ROWS: usize = 5;

fn text_of(i: &Instr71) -> String {
    let mut s = String::new();
    write_instr71(&mut s, i).expect("a String does not fail");
    s
}

#[test]
fn every_mesa_v71_vector_decodes_to_mesas_text() {
    assert_eq!(MESA_V71.len(), MESA_V71_ROWS);
    let mut bad = Vec::new();
    for &(w, want) in MESA_V71 {
        match decode71(w) {
            Ok(i) => {
                let got = text_of(&i);
                if got != want {
                    bad.push(format!("{w:#018x}: got {got:?}, want {want:?}"));
                }
            }
            Err(e) => bad.push(format!("{w:#018x}: {e:?}, want {want:?}")),
        }
    }
    assert!(bad.is_empty(), "{} of {} vectors differ:\n{}", bad.len(), MESA_V71.len(), bad.join("\n"));
}

#[test]
fn every_mesa_v71_vector_encodes_back() {
    for &(w, text) in MESA_V71 {
        let i = decode71(w).unwrap_or_else(|e| panic!("{w:#018x} ({text}): {e:?}"));
        assert_eq!(encode71(&i), Ok(w), "{w:#018x} ({text})");
        if let Instr71::Alu(mut alu) = i {
            if matches!(alu.add.op, AddOp71::Fadd | AddOp71::Faddnf | AddOp71::Fmin | AddOp71::Fmax) {
                core::mem::swap(&mut alu.add.a, &mut alu.add.b);
                core::mem::swap(&mut alu.sig.small_imm_a, &mut alu.sig.small_imm_b);
                assert_eq!(encode71(&Instr71::Alu(alu)), Ok(w), "{w:#018x} ({text}) with a and b swapped");
            }
        }
    }
}

/// Fields read directly, not through the printer.
#[test]
fn mesa_v71_vectors_fields() {
    let Instr71::Alu(a) = decode71(MESA_V71[0].0).unwrap() else { panic!("ALU") };
    assert_eq!((a.add.op, a.add.waddr, a.add.magic), (AddOp71::Add, 10, false));
    assert!(a.sig.small_imm_a && !a.sig.small_imm_b);
    assert_eq!(Alu71::small_imm(a.add.a), Some(1));
    assert_eq!(a.add.b.raddr, 2);

    let Instr71::Alu(a) = decode71(MESA_V71[1].0).unwrap() else { panic!("ALU") };
    assert!(a.sig.small_imm_b && !a.sig.small_imm_a);
    assert_eq!(Alu71::small_imm(a.add.b), Some(-2i32 as u32));

    let Instr71::Alu(a) = decode71(MESA_V71[2].0).unwrap() else { panic!("ALU") };
    assert!(a.sig.small_imm_c);
    assert_eq!((a.mul.op, Alu71::small_imm(a.mul.a)), (MulOp71::Fmul, Some(4)));

    let Instr71::Alu(a) = decode71(MESA_V71[3].0).unwrap() else { panic!("ALU") };
    assert!(a.sig.small_imm_d);
    assert_eq!((a.mul.op, Alu71::small_imm(a.mul.b)), (MulOp71::Smul24, Some(8)));

    let Instr71::Alu(a) = decode71(MESA_V71[4].0).unwrap() else { panic!("ALU") };
    assert_eq!((a.add.op, a.mul.op), (AddOp71::SetnnmodeUu, MulOp71::V8dot));
    assert_eq!((a.mul.waddr, a.mul.a.raddr, a.mul.b.raddr), (12, 1, 2));
}
