// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GFXH-1684 and GFXH-1625 are V3D 4.2 errata that Mesa does not apply at
//! 7.1; `verify71` must not either (docs/notes/v3d71-backend.md, section f).
//!
//! * GFXH-1625, "No TMUWT in the last instruction": Mesa refuses it only at
//!   `ver == 42` (qpu_validate.c:422-426, Mesa 26.2.1): the instruction two
//!   after the last THRSW. `verify` (4.2) carries the rule (verify.rs, "TMUWT
//!   in last instruction"); `verify71` has none. The programs below are the
//!   same thread end at both generations, built by hand, with a TMUWT in the
//!   second delay slot of the last THRSW, encoded and decoded back with the
//!   generation's codec before they are verified.
//! * GFXH-1684, "VPM writes need to be complete by the end of the shader":
//!   Mesa ends a vertex or geometry shader with a VPMWT only at `ver == 42`
//!   (nir_to_vir.c:2166-2169 `emit_vert_end`, :2175-2178 `emit_geom_end`).
//!   That is an emission rule, not a validator rule: qpu_validate.c has no
//!   check for it, and neither does verify.rs (its only VPMWT mention is the
//!   peripheral count). So nothing is asserted about 4.2 refusing a vertex
//!   shader without the final VPMWT; the test only pins that verify71 accepts
//!   one (and that verify, having no such rule, does too).
//!
//! Each erratum's programs are also run against a rule both generations
//! share (the program-end THRSW, "No program-end THRSW found"; a THRSW in a
//! branch delay slot), which both verifiers must still refuse, so a pass
//! cannot come from `verify71` accepting everything.
//!
//! Feature set: host-test,gpu-v3d,shaderc-online,no-shader-table.
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", feature = "no-shader-table"))]

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::decode::decode;
use yakogl::shaderc::online::v3d::encode::encode;
use yakogl::shaderc::online::v3d::isa::{self as i42, Instr};
use yakogl::shaderc::online::v3d::isa71::{self as i71, Instr71};
use yakogl::shaderc::online::v3d::verify::verify;
use yakogl::shaderc::online::v3d::verify71::verify71;

/// A 4.2 instruction, through the 4.2 codec and back.
fn rt42(a: i42::Alu) -> Instr {
    decode(encode(&Instr::Alu(a)).expect("encode 4.2")).expect("decode 4.2")
}
/// A 7.1 instruction, through codec71 and back.
fn rt71(a: i71::Alu71) -> Instr71 {
    decode71(encode71(&Instr71::Alu(a)).expect("encode 7.1")).expect("decode 7.1")
}

fn nop42() -> i42::Alu {
    i42::Alu::nop()
}
fn nop71() -> i71::Alu71 {
    i71::Alu71::nop()
}
fn thrsw42() -> i42::Alu {
    let mut a = nop42();
    a.sig.thrsw = true;
    a
}
fn thrsw71() -> i71::Alu71 {
    let mut a = nop71();
    a.sig.thrsw = true;
    a
}
fn tmuwt42() -> i42::Alu {
    let mut a = nop42();
    a.add.op = i42::AddOp::Tmuwt;
    a
}
fn tmuwt71() -> i71::Alu71 {
    let mut a = nop71();
    a.add.op = i71::AddOp71::Tmuwt;
    a
}
fn vpmwt42() -> i42::Alu {
    let mut a = nop42();
    a.add.op = i42::AddOp::Vpmwt;
    a
}
fn vpmwt71() -> i71::Alu71 {
    let mut a = nop71();
    a.add.op = i71::AddOp71::Vpmwt;
    a
}
/// A VPM write of a register-file value (no accumulator, so neither
/// generation's accumulator rules come in).
fn stvpmv42() -> i42::Alu {
    let mut a = nop42();
    a.add.op = i42::AddOp::Stvpmv;
    a.add.a.mux = i42::Mux::A;
    a.add.b.mux = i42::Mux::B;
    a.raddr_a = 1;
    a.raddr_b = 0;
    a
}
fn stvpmv71() -> i71::Alu71 {
    let mut a = nop71();
    a.add.op = i71::AddOp71::Stvpmv;
    a.add.a.raddr = 1;
    a.add.b.raddr = 0;
    a
}

fn r42(v: &[i42::Alu]) -> Result<(), &'static str> {
    let p: Vec<Instr> = v.iter().map(|a| rt42(*a)).collect();
    verify(&p, true).map_err(|e| e.rule)
}
fn r71(v: &[i71::Alu71]) -> Result<(), &'static str> {
    let p: Vec<Instr71> = v.iter().map(|a| rt71(*a)).collect();
    verify71(&p, true).map_err(|e| e.rule)
}

#[test]
fn gfxh_1625_tmuwt_in_the_last_instruction_is_4_2_only() {
    /* the thread end: THRSW, then its two delay slots, TMUWT in the second */
    let n = nop42();
    assert_eq!(r42(&[n, n, thrsw42(), n, tmuwt42()]), Err("TMUWT in last instruction"));
    let n = nop71();
    assert_eq!(r71(&[n, n, thrsw71(), n, tmuwt71()]), Ok(()), "GFXH-1625 does not apply at 7.1");

    /* the TMUWT in the first slot is fine at both: the rule is the last instruction only */
    let n = nop42();
    assert_eq!(r42(&[n, n, thrsw42(), tmuwt42(), n]), Ok(()));
    let n = nop71();
    assert_eq!(r71(&[n, n, thrsw71(), tmuwt71(), n]), Ok(()));
}

#[test]
fn gfxh_1625_programs_still_meet_the_shared_rules() {
    /* no program-end THRSW at all: refused at both generations, TMUWT or not */
    let (n4, n7) = (nop42(), nop71());
    assert_eq!(r42(&[n4, tmuwt42(), n4]), Err("No program-end THRSW found"));
    assert_eq!(r71(&[n7, tmuwt71(), n7]), Err("No program-end THRSW found"));

    /* a THRSW in a branch's delay slot, then a thread end with the TMUWT */
    let br42 = Instr::Branch(i42::Branch {
        cond: i42::BranchCond::Always,
        msfign: i42::Msfign::None,
        bdi: i42::BranchDest::Rel,
        ub: false,
        bdu: i42::BranchDest::Rel,
        raddr_a: 0,
        offset: 0,
    });
    let br71 = Instr71::Branch(i71::Branch {
        cond: i71::BranchCond::Always,
        msfign: i71::Msfign::None,
        bdi: i71::BranchDest::Rel,
        ub: false,
        bdu: i71::BranchDest::Rel,
        raddr_a: 0,
        offset: 0,
    });
    let mut p42 = vec![br42, Instr::Alu(thrsw42()), Instr::Alu(n4), Instr::Alu(n4)];
    p42.extend([Instr::Alu(thrsw42()), Instr::Alu(n4), Instr::Alu(tmuwt42())]);
    let mut p71 = vec![br71, Instr71::Alu(thrsw71()), Instr71::Alu(n7), Instr71::Alu(n7)];
    p71.extend([Instr71::Alu(thrsw71()), Instr71::Alu(n7), Instr71::Alu(tmuwt71())]);
    assert_eq!(verify(&p42, true).map_err(|e| e.rule), Err("THRSW in a branch delay slot."));
    assert_eq!(verify71(&p71, true).map_err(|e| e.rule), Err("THRSW in a branch delay slot."));
}

#[test]
fn gfxh_1684_a_vertex_shader_without_the_final_vpmwt() {
    /* VPM write, then the thread end, with no VPMWT anywhere. verify.rs has
     * no GFXH-1684 rule (Mesa's validator has none: the erratum is
     * emit_vert_end's, nir_to_vir.c:2166-2169), so 4.2 is not asserted to
     * refuse; verify71 must accept. */
    let (n4, n7) = (nop42(), nop71());
    assert_eq!(r71(&[stvpmv71(), n7, n7, thrsw71(), n7, n7]), Ok(()));
    assert_eq!(r42(&[stvpmv42(), n4, n4, thrsw42(), n4, n4]), Ok(()), "verify.rs does not enforce GFXH-1684");

    /* the 4.2 shape with its VPMWT is not refused at 7.1 either */
    assert_eq!(r42(&[stvpmv42(), n4, vpmwt42(), thrsw42(), n4, n4]), Ok(()));
    assert_eq!(r71(&[stvpmv71(), n7, vpmwt71(), thrsw71(), n7, n7]), Ok(()));

    /* shared rule on the same programs: without the program-end THRSW both refuse */
    assert_eq!(r42(&[stvpmv42(), n4, n4]), Err("No program-end THRSW found"));
    assert_eq!(r71(&[stvpmv71(), n7, n7]), Err("No program-end THRSW found"));
}
