// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Negative controls for `verify71` (src/shaderc/online/v3d/verify71.rs): a
//! sample of Mesa's own V3D 7.1 programs (tests/fixtures/v3d_mesa_programs_71.txt,
//! decoded with codec71) passes, and each mutant, one per rule verify71
//! implements, is refused with that rule's name at the instruction the
//! mutation put it. The rules Mesa 26.2.1's `qpu_validate.c` / `qpu_schedule.c`
//! apply only at 4.2 (GFXH-1684's final VPMWT, GFXH-1625's TMUWT in the last
//! instruction, the accumulator SFU and R4 rules) have mutants that must
//! still PASS. Every mutant is encoded with codec71 and decoded back before
//! it is verified, so what is judged is the word, not the Rust value.
//!
//! A mutant is either a PREFIX (instructions inserted before the sample
//! program, so the error lands at the insertion's index whatever the program
//! does) or a TAIL (the program's last three instructions, its thread end,
//! replaced).
//!
//! Mesa line numbers are those of Mesa 26.2.1, as verify71.rs cites them
//! (`qpu_validate.c` unless named). Rules verify71 marks "(ours)" are Mesa's
//! scheduler's (`qpu_schedule.c`), not its validator's: Mesa's validator
//! accepts those programs, verify71 deliberately does not (counted by the
//! test through the "(ours)" in their names).
//!
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_verify_mutants -- --nocapture

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa::num_src;
use yakogl::shaderc::online::v3d::isa71::*;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};

static FIXTURE: &str = include_str!("fixtures/v3d_mesa_programs_71.txt");

struct Prog {
    name: String,
    threads: u32,
    prog: Vec<Instr71>,
}

/// Mesa's stage programs (pair, stage, threads, words), decoded.
fn programs() -> Vec<Prog> {
    let mut out = Vec::new();
    let mut pair = String::new();
    let mut threads = std::collections::BTreeMap::<String, u32>::new();
    for line in FIXTURE.lines() {
        let w: Vec<&str> = line.split(' ').filter(|s| !s.is_empty()).collect();
        match w.first().copied() {
            Some("pair") => {
                pair = format!("{} + {}", w[1], w[2]);
                threads.clear();
            }
            Some(st @ ("fs" | "vs" | "cs")) if w.get(1) == Some(&"meta") => {
                let t = w[2..].iter().find_map(|kv| kv.strip_prefix("threads=")).expect("threads=");
                threads.insert(st.into(), t.parse().unwrap());
            }
            Some(st @ ("fs" | "vs" | "cs")) if w.get(1) == Some(&"code") && w.len() > 2 => {
                let prog = w[2..]
                    .iter()
                    .map(|s| decode71(u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()).unwrap())
                    .collect();
                out.push(Prog { name: format!("{pair} {st}"), threads: threads[st], prog });
            }
            _ => {}
        }
    }
    out
}

/// Mesa's `single_seg = !c->last_thrsw`: no two THRSW in a row.
fn single_seg(prog: &[Instr71]) -> bool {
    let thrsw = |i: &Instr71| matches!(i, Instr71::Alu(a) if a.sig.thrsw);
    !prog.windows(2).any(|w| thrsw(&w[0]) && thrsw(&w[1]))
}

/// A small sample: every 40th program, and the first of each thread count
/// and of the single-segment programs.
fn sample() -> Vec<Prog> {
    let all = programs();
    assert!(all.len() >= 700, "the fixture's programs");
    let mut picked: Vec<Prog> = Vec::new();
    let mut seen_threads = std::collections::BTreeSet::new();
    let mut seen_single = false;
    for (idx, p) in all.into_iter().enumerate() {
        let new_threads = seen_threads.insert(p.threads);
        let new_single = single_seg(&p.prog) && !std::mem::replace(&mut seen_single, true);
        if idx % 40 == 0 || new_threads || new_single {
            picked.push(p);
        }
    }
    picked
}

// ---------------------------------------------------------------------------
// Builders (the shapes verify71's own tests use).
// ---------------------------------------------------------------------------

fn nop() -> Alu71 {
    Alu71::nop()
}
fn thrsw() -> Alu71 {
    let mut a = nop();
    a.sig.thrsw = true;
    a
}
fn thrsw_with(mut a: Alu71) -> Alu71 {
    a.sig.thrsw = true;
    a
}
fn add(op: AddOp71, waddr: u8, magic: bool) -> Alu71 {
    let mut a = nop();
    a.add.op = op;
    a.add.waddr = waddr;
    a.add.magic = magic;
    a.add.a.raddr = 10;
    if num_src(add_op_args(op)) > 1 {
        a.add.b.raddr = 11;
    }
    a
}
fn mul(op: MulOp71, waddr: u8, magic: bool) -> Alu71 {
    let mut a = nop();
    a.mul.op = op;
    a.mul.waddr = waddr;
    a.mul.magic = magic;
    a.mul.a.raddr = 10;
    a.mul.b.raddr = 11;
    a
}
/// The add unit's operand a reads rf `rf`.
fn reading(mut a: Alu71, rf: u8) -> Alu71 {
    a.add.a.raddr = rf;
    a
}
fn ldvary(rf: u8) -> Alu71 {
    let mut a = nop();
    a.sig.ldvary = true;
    a.sig_addr = rf;
    a
}
fn ldtmu(rf: u8) -> Alu71 {
    let mut a = nop();
    a.sig.ldtmu = true;
    a.sig_addr = rf;
    a
}
fn ldunif() -> Alu71 {
    let mut a = nop();
    a.sig.ldunif = true;
    a
}
fn ldunifa() -> Alu71 {
    let mut a = nop();
    a.sig.ldunifa = true;
    a
}
fn branch() -> Instr71 {
    Instr71::Branch(Branch {
        cond: BranchCond::Always,
        msfign: Msfign::None,
        bdi: BranchDest::Rel,
        ub: false,
        bdu: BranchDest::Abs,
        raddr_a: 0,
        offset: 0,
    })
}
fn i(a: Alu71) -> Instr71 {
    Instr71::Alu(a)
}
fn nops(n: usize) -> Vec<Instr71> {
    (0..n).map(|_| i(nop())).collect()
}

enum Site {
    /// Inserted before the program: the error lands at an index into the
    /// insertion.
    Prefix(Vec<Instr71>),
    /// Replaces the program's last three instructions.
    Tail([Instr71; 3]),
}

struct Mutant {
    name: String,
    /// The Mesa line(s) the rule mirrors.
    mesa: &'static str,
    site: Site,
    /// `Ok(())`, or the rule's name and the index within the site it fires
    /// at (tails: 0..3).
    want: Result<(), (&'static str, usize)>,
    /// No word holds it (Mesa's `pack` refuses): only the Rust value can
    /// carry the mutation, so the codec step is skipped for it.
    unencodable: bool,
    /// Runs only on programs without a last-thread-switch pair (its prefix
    /// brings its own).
    single_base: bool,
}

fn ok(name: &str, mesa: &'static str, site: Site) -> Mutant {
    Mutant { name: name.into(), mesa, site, want: Ok(()), unencodable: false, single_base: false }
}
fn err(name: &str, mesa: &'static str, site: Site, rule: &'static str, at: usize) -> Mutant {
    Mutant { name: name.into(), mesa, site, want: Err((rule, at)), unencodable: false, single_base: false }
}
/// A prefix, followed by nops so that the mutant's own delay slots do not
/// meet the program's first instruction.
fn pre(mut v: Vec<Instr71>) -> Site {
    v.extend(nops(4));
    Site::Prefix(v)
}

#[allow(clippy::vec_init_then_push)] // one push per rule, each under its Mesa citation
fn mutants() -> Vec<Mutant> {
    use AddOp71 as A;
    use MulOp71 as M;
    let mut m = Vec::new();

    // --- qpu_validate.c:157-164, the branch delay slots ---
    m.push(err(
        "branch in a branch delay slot",
        "qpu_validate.c:157-160",
        pre(vec![branch(), i(nop()), branch()]),
        "branch in a branch delay slot.",
        2,
    ));
    m.push(err(
        "branch in a THRSW delay slot",
        "qpu_validate.c:161-164",
        pre(vec![i(thrsw()), i(nop()), branch()]),
        "branch in a THRSW delay slot.",
        2,
    ));
    m.push(err(
        "THRSW in a branch delay slot",
        "qpu_validate.c:349-352",
        pre(vec![branch(), i(thrsw())]),
        "THRSW in a branch delay slot.",
        1,
    ));
    m.push(err(
        "branch near a UNIFA write (ours)",
        "qpu_schedule.c:1483-1489",
        pre(vec![i(add(A::Mov, 9, true)), i(nop()), branch()]),
        "branch within three instructions of a UNIFA write (qpu_schedule.c, ours)",
        2,
    ));

    // --- qpu_validate.c:168-185, the rtop hazard ---
    m.push(err(
        "UMUL24 after a MULTOP cleared by a THRSW",
        "qpu_validate.c:168-185, :429-432",
        pre(vec![i(thrsw()), i(mul(M::Multop, 6, true)), i(nop()), i(mul(M::Umul24, 10, false))]),
        "UMUL24 reads rtop from MULTOP but it got cleared by a previous THRSW",
        3,
    ));
    m.push(ok(
        "UMUL24 after a MULTOP, no THRSW between",
        "qpu_validate.c:168-185",
        pre(vec![i(mul(M::Multop, 6, true)), i(nop()), i(mul(M::Umul24, 10, false))]),
    ));

    // --- qpu_validate.c:204-218, the 7.1 small immediates ---
    let mut two = add(A::Add, 10, false);
    two.mul = mul(M::Add, 11, false).mul;
    two.sig.small_imm_b = true;
    two.sig.small_imm_c = true;
    let mut two_imm = err(
        "two small immediates in one instruction (no word has them: Mesa's pack refuses)",
        "qpu_validate.c:213-217",
        pre(vec![i(two)]),
        "only one small immediate can be enabled per instruction",
        0,
    );
    two_imm.unencodable = true;
    m.push(two_imm);
    let mut a_only = nop();
    a_only.sig.small_imm_a = true;
    m.push(err(
        "small imm a/b with no add operation",
        "qpu_validate.c:204-208",
        pre(vec![i(a_only)]),
        "small imm a/b used but no ADD inst",
        0,
    ));
    let mut d_only = nop();
    d_only.sig.small_imm_d = true;
    m.push(err(
        "small imm c/d with no mul operation",
        "qpu_validate.c:209-212",
        pre(vec![i(d_only)]),
        "small imm c/d used but no MUL inst",
        0,
    ));
    let mut one = add(A::Add, 10, false);
    one.sig.small_imm_b = true;
    m.push(ok("one small immediate with an add operation", "qpu_validate.c:204-217", pre(vec![i(one)])));

    // --- qpu_validate.c:220-227, LDVARY then LDUNIF at 7.1 ---
    m.push(err(
        "LDUNIF in the instruction after LDVARY",
        "qpu_validate.c:220-227",
        pre(vec![i(ldvary(10)), i(ldunif())]),
        "LDUNIF after a LDVARY",
        1,
    ));
    m.push(err(
        "LDUNIFA in the instruction after LDVARY",
        "qpu_validate.c:220-227",
        pre(vec![i(ldvary(10)), i(ldunifa())]),
        "LDUNIF after a LDVARY",
        1,
    ));
    m.push(ok(
        "LDUNIF two instructions after LDVARY",
        "qpu_validate.c:220-227",
        pre(vec![i(ldvary(10)), i(nop()), i(ldunif())]),
    ));
    m.push(ok("LDVARY after LDVARY", "qpu_validate.c (no rule)", pre(vec![i(ldvary(10)), i(ldvary(11))])));

    // --- the rf0 implicit write (qpu_schedule.c:716-726, :2725-2727) ---
    m.push(err(
        "add rf0 write right after LDVARY (ours)",
        "qpu_schedule.c:716-726",
        pre(vec![i(ldvary(10)), i(add(A::Mov, 0, false))]),
        "rf0 written right after a LDVARY (qpu_schedule.c, ours)",
        1,
    ));
    m.push(err(
        "mul rf0 write right after LDVARY (ours)",
        "qpu_schedule.c:716-726",
        pre(vec![i(ldvary(10)), i(mul(M::Mov, 0, false))]),
        "rf0 written right after a LDVARY (qpu_schedule.c, ours)",
        1,
    ));
    m.push(ok(
        "rf0 read right after LDVARY (not a rule)",
        "qpu_schedule.c:622-636, :2589-2730",
        pre(vec![i(ldvary(10)), i(reading(add(A::Fadd, 12, false), 0))]),
    ));
    m.push(ok(
        "explicit rf0 write right after LDUNIF",
        "qpu_validate.c (no rule)",
        pre(vec![i(ldunif()), i(add(A::Mov, 0, false))]),
    ));
    m.push(ok(
        "explicit rf0 write two after LDVARY",
        "qpu_schedule.c:716-726",
        pre(vec![i(ldvary(10)), i(nop()), i(add(A::Mov, 0, false))]),
    ));

    // --- qpu_schedule.c:1440-1442, LDUNIFA after a UNIFA write ---
    m.push(err(
        "LDUNIFA two after a UNIFA write (ours)",
        "qpu_schedule.c:1440-1442",
        pre(vec![i(add(A::Mov, 9, true)), i(nop()), i(ldunifa())]),
        "LDUNIFA within three instructions of a UNIFA write (qpu_schedule.c, ours)",
        2,
    ));
    m.push(ok(
        "LDUNIFA four after a UNIFA write",
        "qpu_schedule.c:1440-1442",
        pre(vec![i(add(A::Mov, 9, true)), i(nop()), i(nop()), i(nop()), i(ldunifa())]),
    ));

    // --- SFU at 7.1: an add operation, its magic waddrs reserved ---
    m.push(ok(
        "SFU result read in the very next instruction",
        "qpu_schedule.c:778-786 (a stall, not a refusal)",
        pre(vec![i(add(A::Recip, 10, false)), i(reading(add(A::Fadd, 12, false), 10))]),
    ));
    m.push(ok(
        "two SFU operations back to back",
        "qpu_validate.c:316-328 (ver 42 only)",
        pre(vec![i(add(A::Recip, 10, false)), i(add(A::Rsqrt, 11, false))]),
    ));
    m.push(ok(
        "SFU result read two instructions later",
        "qpu_validate.c:316-328 (ver 42 only)",
        pre(vec![i(add(A::Recip, 10, false)), i(nop()), i(reading(add(A::Fadd, 12, false), 10))]),
    ));
    m.push(err(
        "legacy magic SFU write (reserved at 7.1)",
        "qpu_validate.c:104-115, :309-314",
        pre(vec![i(add(A::Mov, 19, true))]),
        "write to a reserved magic waddr",
        0,
    ));
    m.push(err(
        "SFU write in a THRSW delay slot",
        "qpu_validate.c:290-296",
        pre(vec![i(thrsw()), i(add(A::Mov, 20, true))]),
        "SFU write started during THRSW delay slots ",
        1,
    ));
    m.push(err(
        "SFU operation in a THRSW delay slot (ours)",
        "qpu_schedule.c:2075-2084",
        pre(vec![i(thrsw()), i(add(A::Recip, 10, false))]),
        "SFU operation in a THRSW delay slot (qpu_schedule.c, ours)",
        1,
    ));

    // --- the THRSW delay slots and pairing ---
    m.push(err(
        "LDVARY in the 2nd THRSW delay slot",
        "qpu_validate.c:302-305",
        pre(vec![i(thrsw()), i(nop()), i(ldvary(10))]),
        "LDVARY in 2nd THRSW delay slot",
        2,
    ));
    let mut first_slot = ok(
        "LDVARY in the 1st THRSW delay slot",
        "qpu_validate.c:302-305",
        Site::Prefix([thrsw(), ldvary(10), nop(), nop(), thrsw(), thrsw(), nop(), nop()].map(i).to_vec()),
    );
    first_slot.single_base = true;
    m.push(first_slot);
    m.push(err(
        "THRSW too close to another THRSW",
        "qpu_validate.c:353-358",
        pre(vec![i(thrsw()), i(nop()), i(thrsw())]),
        "THRSW too close to another THRSW.",
        2,
    ));
    m.push(err(
        "two last-THRSW signals",
        "qpu_validate.c:353-361",
        Site::Prefix([thrsw(), thrsw(), nop(), nop(), thrsw(), thrsw(), nop(), nop()].map(i).to_vec()),
        "Two last-THRSW signals",
        5,
    ));
    m.push(err(
        "UNIFA write in a THRSW delay slot (ours)",
        "qpu_schedule.c:2093-2108",
        pre(vec![i(thrsw()), i(add(A::Mov, 9, true))]),
        "UNIFA write in a THRSW instruction or its delay slots (qpu_schedule.c, ours)",
        1,
    ));
    let mut rf0_undef = err(
        "signal RF write in the last THRSW slot with rf0 undefined (ours)",
        "qpu_schedule.c:1691-1745, :2110-2119",
        Site::Prefix([thrsw(), ldvary(10), ldtmu(11), nop(), nop(), thrsw(), thrsw(), nop(), nop()].map(i).to_vec()),
        "a signal's RF write in the last THRSW delay slot with rf0 undefined (qpu_schedule.c, ours)",
        2,
    );
    rf0_undef.single_base = true;
    m.push(rf0_undef);

    // --- reserved magic waddrs and one-peripheral rules ---
    for w in [0u8, 4, 10, 14, 15, 19, 31, 47, 55, 63] {
        m.push(err(
            &format!("write to reserved magic waddr {w}"),
            "qpu_validate.c:104-115, :309-314",
            pre(vec![i(add(A::Mov, w, true))]),
            "write to a reserved magic waddr",
            0,
        ));
    }
    let mut mulrsv = nop();
    mulrsv.mul = mul(M::Mov, 10, true).mul;
    m.push(err(
        "mul unit write to a reserved magic waddr",
        "qpu_validate.c:309-314",
        pre(vec![i(mulrsv)]),
        "write to a reserved magic waddr",
        0,
    ));
    for w in [5u8, 6, 9, 12, 13, 16, 17, 18, 32] {
        m.push(ok(
            &format!("write to magic waddr {w} (not reserved at 7.1)"),
            "qpu_validate.c:104-115",
            pre(vec![i(add(A::Mov, w, true))]),
        ));
    }
    let mut tmu_tlb = add(A::Mov, 11, true);
    tmu_tlb.mul = mul(M::Mov, 7, true).mul;
    m.push(err(
        "TMU write and TLB write in one instruction",
        "qpu_validate.c:330-344",
        pre(vec![i(tmu_tlb)]),
        "Only one of [TMU, SFU, TSY, TLB read, VPM] allowed",
        0,
    ));
    let mut sfu_tmu = add(A::Recip, 10, false);
    sfu_tmu.mul = mul(M::Mov, 11, true).mul;
    m.push(err(
        "SFU operation and TMU write in one instruction (ours)",
        "qpu_schedule.c:936-975",
        pre(vec![i(sfu_tmu)]),
        "more than one peripheral access (qpu_compatible_peripheral_access, ours)",
        0,
    ));
    let mut wrtmuc = mul(M::Mov, 11, true);
    wrtmuc.sig.wrtmuc = true;
    m.push(ok("WRTMUC with a TMU write other than TMUC", "qpu_schedule.c:936-975", pre(vec![i(wrtmuc)])));

    // --- the program end ---
    m.push(err(
        "no program-end THRSW",
        "qpu_validate.c:478-484",
        Site::Tail([i(nop()), i(nop()), i(nop())]),
        "No program-end THRSW found",
        2,
    ));
    m.push(ok(
        "a thread end of nops (control)",
        "qpu_validate.c:349-371",
        Site::Tail([i(thrsw()), i(nop()), i(nop())]),
    ));

    // --- the 7.1 thread-end register-file rules, qpu_validate.c:373-421 ---
    let mut t = thrsw();
    t.add = add(A::Mov, 10, false).add;
    m.push(err(
        "ADD rf write in the THRSW instruction",
        "qpu_validate.c:380-390",
        Site::Tail([i(t), i(nop()), i(nop())]),
        "ADD RF write at THREND",
        0,
    ));
    let mut t = thrsw();
    t.mul = mul(M::Mov, 10, false).mul;
    m.push(err(
        "MUL rf write in the THRSW instruction",
        "qpu_validate.c:391-401",
        Site::Tail([i(t), i(nop()), i(nop())]),
        "MUL RF write at THREND",
        0,
    ));
    m.push(err(
        "rf3 write in slot 1",
        "qpu_validate.c:402-419",
        Site::Tail([i(thrsw()), i(add(A::Mov, 3, false)), i(nop())]),
        "RF2-3 write after THREND",
        1,
    ));
    m.push(err(
        "rf2 mul write in slot 2",
        "qpu_validate.c:402-419",
        Site::Tail([i(thrsw()), i(nop()), i(mul(M::Mov, 2, false))]),
        "RF2-3 write after THREND",
        2,
    ));
    m.push(err(
        "rf2 signal write in slot 2",
        "qpu_validate.c:402-419",
        Site::Tail([i(thrsw()), i(nop()), i(ldtmu(2))]),
        "RF2-3 write after THREND",
        2,
    ));
    m.push(ok(
        "rf write other than rf2/rf3 in thread-end slots",
        "qpu_validate.c:373-421",
        Site::Tail([i(thrsw()), i(add(A::Mov, 10, false)), i(add(A::Mov, 11, false))]),
    ));
    m.push(ok(
        "rf0 write in a thread-end slot (4.2 forbids)",
        "qpu_validate.c:373-379 (ver 42 only)",
        Site::Tail([i(thrsw()), i(add(A::Mov, 0, false)), i(nop())]),
    ));
    m.push(err(
        "LDVARY in a thread-end slot (ours)",
        "qpu_schedule.c:1978-2060",
        Site::Tail([i(thrsw()), i(ldvary(10)), i(nop())]),
        "LDVARY in a thread-end slot (qpu_schedule.c, ours)",
        1,
    ));
    m.push(err(
        "uniform read in thread-end slot 2 (ours)",
        "qpu_schedule.c:1978-2060",
        Site::Tail([i(thrsw()), i(nop()), i(ldunif())]),
        "uniform read in a thread-end delay slot (qpu_schedule.c, ours)",
        2,
    ));
    m.push(err(
        "rf2 read in a thread-end slot (ours)",
        "qpu_schedule.c:2003-2041",
        Site::Tail([i(thrsw()), i(reading(add(A::Mov, 10, false), 2)), i(nop())]),
        "RF2-3 read after THREND (qpu_schedule.c, ours)",
        1,
    ));

    // --- the 4.2-only rules: these mutants must still pass at 7.1 ---
    m.push(ok(
        "GFXH-1625: TMUWT in the last instruction",
        "qpu_validate.c:422-426 (ver 42)",
        Site::Tail([i(thrsw()), i(nop()), i(add(A::Tmuwt, WADDR_NOP, true))]),
    ));
    m.push(ok(
        "GFXH-1625: TMUWT in the THRSW instruction",
        "qpu_schedule.c:1997-2001 (ver 42)",
        Site::Tail([i(thrsw_with(add(A::Tmuwt, WADDR_NOP, true))), i(nop()), i(nop())]),
    ));
    m.push(ok(
        "GFXH-1684: VPMWT as the final instruction",
        "nir_to_vir.c:2164-2178 (ver 42)",
        Site::Tail([i(thrsw()), i(nop()), i(add(A::Vpmwt, WADDR_NOP, true))]),
    ));
    m.push(ok(
        "GFXH-1684: VPMWT in the THRSW instruction",
        "qpu_schedule.c:1990-1991 (ver 42)",
        Site::Tail([i(thrsw_with(add(A::Vpmwt, WADDR_NOP, true))), i(nop()), i(nop())]),
    ));
    m.push(ok(
        "GFXH-1684: no final VPMWT",
        "nir_to_vir.c:2164-2178 (ver 42)",
        Site::Tail([i(thrsw()), i(nop()), i(nop())]),
    ));
    m.push(ok(
        "rf0 read in a thread-end slot (4.2 forbids)",
        "qpu_schedule.c:2003-2041 (ver 42)",
        Site::Tail([i(thrsw()), i(reading(add(A::Mov, 10, false), 0)), i(nop())]),
    ));
    // A reserved magic waddr has no name, so codec71 refuses to encode it
    // (MagicWaddrUnnamed): these mutants are values only.
    for x in &mut m {
        let n = x.name.as_str();
        x.unencodable |= n.contains("reserved magic waddr")
            || n.starts_with("legacy magic SFU")
            || n == "SFU write in a THRSW delay slot";
    }
    m
}

/// The program through the codec: encode every instruction and decode it
/// back, so a mutant that is not a word fails here.
fn through_codec(name: &str, prog: &[Instr71]) -> Vec<Instr71> {
    prog.iter()
        .enumerate()
        .map(|(n, ins)| {
            let w = encode71(ins).unwrap_or_else(|e| panic!("{name}: instruction {n} does not encode: {e:?}"));
            decode71(w).unwrap_or_else(|e| panic!("{name}: word {w:#x} does not decode: {e:?}"))
        })
        .collect()
}

#[test]
fn unmutated_mesa_programs_pass() {
    let progs = sample();
    println!("{} sampled programs", progs.len());
    assert!(progs.len() >= 15);
    for p in &progs {
        verify71(&p.prog, single_seg(&p.prog)).unwrap_or_else(|e| panic!("{}: {e:?}", p.name));
        registers_fit_threads71(&p.prog, p.threads).unwrap_or_else(|e| panic!("{}: {e:?}", p.name));
        let back = through_codec(&p.name, &p.prog);
        assert_eq!(back, p.prog, "{}", p.name);
    }
}

#[test]
fn every_rule_has_its_mutant() {
    let progs = sample();
    let ms = mutants();
    for m in &ms {
        for p in &progs {
            if m.single_base && !single_seg(&p.prog) {
                continue;
            }
            let (mutated, base) = match &m.site {
                Site::Prefix(v) => {
                    let mut x = v.clone();
                    x.extend(p.prog.iter().copied());
                    (x, 0)
                }
                Site::Tail(t) => {
                    let mut x = p.prog[..p.prog.len() - 3].to_vec();
                    x.extend(t.iter().copied());
                    (x, p.prog.len() - 3)
                }
            };
            let prog = if m.unencodable { mutated } else { through_codec(&m.name, &mutated) };
            let got = verify71(&prog, single_seg(&prog));
            let ctx = format!("mutant \"{}\" ({}) on {}", m.name, m.mesa, p.name);
            match (&m.want, got) {
                (Ok(()), Ok(())) => {}
                (Ok(()), Err(e)) => panic!("{ctx}: must pass, refused at {}: {}", e.ip, e.rule),
                (Err((rule, _)), Ok(())) => panic!("{ctx}: must be refused with {rule:?}, passed"),
                (Err((rule, at)), Err(e)) => {
                    assert_eq!(e.rule, *rule, "{ctx}: rule");
                    assert_eq!(e.ip, base + at, "{ctx}: instruction");
                }
            }
        }
    }
    let refused = ms.iter().filter(|m| m.want.is_err()).count();
    let ours = ms.iter().filter(|m| m.want.is_err() && m.name.contains("(ours)")).count();
    let must_pass = ms.len() - refused;
    println!(
        "{} mutants on {} programs: {refused} refused ({ours} by rules of ours that Mesa's validator lacks, {} by Mesa's), {must_pass} must pass",
        ms.len(),
        progs.len(),
        refused - ours
    );
    assert!(ours >= 8 && refused - ours >= 20 && must_pass >= 20);
}

/// `registers_fit_threads71` (Mesa's allocator, vir_register_allocate.c:1046-1070,
/// :1433-1435: rf0..rf31 at 4 threads, rf0..rf63 at 1 or 2): a write or a read
/// above the limit is refused, and a small immediate is no register.
#[test]
fn registers_above_the_thread_limit_are_refused() {
    const RULE: &str = "a register-file register past what a thread has at this thread count";
    for p in sample() {
        let with = |first: Alu71| {
            let mut v = vec![i(first)];
            v.extend(p.prog.iter().copied());
            through_codec(&p.name, &v)
        };
        let at = |prog: &[Instr71], t| registers_fit_threads71(prog, t).map_err(|e| (e.rule, e.ip));
        let mut imm = reading(add(AddOp71::Mov, 10, false), 40);
        imm.sig.small_imm_a = true;
        let cases = [
            ("rf32 write", with(add(AddOp71::Mov, 32, false))),
            ("rf40 read", with(reading(add(AddOp71::Mov, 10, false), 40))),
            ("signal rf33", with(ldtmu(33))),
            ("mul rf50", with(mul(MulOp71::Mov, 50, false))),
        ];
        for (what, prog) in &cases {
            assert_eq!(at(prog, 4), Err((RULE, 0)), "{} {what} at 4 threads", p.name);
            if p.threads != 4 {
                assert_eq!(at(prog, 2), Ok(()), "{} {what} at 2 threads", p.name);
                assert_eq!(at(prog, 1), Ok(()), "{} {what} at 1 thread", p.name);
            }
        }
        let top = with(add(AddOp71::Mov, 63, false));
        assert_eq!(at(&top, 4), Err((RULE, 0)), "{} rf63 at 4", p.name);
        assert_eq!(at(&top, 2), Ok(()), "{} rf63 at 2", p.name);
        assert_eq!(at(&with(imm), 4), Ok(()), "{} a small immediate is no register", p.name);
        assert!(registers_fit_threads71(&p.prog, 3).is_err(), "3 threads does not exist");
    }
}
