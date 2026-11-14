// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The condition flags of the 7.1 interpreter (`sim::run_gen(Gen::V71, ..)`,
//! `src/shaderc/online/v3d/sim71.rs`): pushes, flag updates, conditional
//! writes and the branch conditions that read them, on small hand-written
//! `Instr71` programs whose lanes differ.
//!
//! Run with `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.
//!
//! The semantics, per lane, A and B being the two flag bits:
//!
//! * `pushz/pushn/pushc` (`enum v3d_qpu_pf`, `qpu_instr.h:70-75`, names
//!   `qpu_instr.c:306-316`) set A to the pushed flag and move the old A to B.
//! * `and<X>/nor<X>` (`enum v3d_qpu_uf`, `qpu_instr.h:77-91`, names
//!   `qpu_instr.c:322-`): A = A & X for `and*`, A = !A & !X for `nor*`, X
//!   being the op's Z, N or C flag, inverted for the `*n*` forms (`andnz`,
//!   `norz`, `nornz`, ...). Mesa's own use: `mov.nornz -, execute` with `ifna`
//!   (`nir_to_vir.c:3486-3492`), `mov.andz` on the exec flag
//!   (`nir_to_vir.c:3783`). What an update does to B is not described: the
//!   interpreter makes B undefined, and so does this test.
//! * The conditions `ifa`, `ifb`, `ifna`, `ifnb` (`enum v3d_qpu_cond`,
//!   `qpu_instr.h:62-68`, inverse `v3d_qpu_cond_invert`, `qpu_instr.c:600-614`)
//!   write only the lanes the flag selects, on the add and the mul ALU alike;
//!   they read the flags as they were before the instruction.
//! * Which op sets which flag is how Mesa uses them (`ntq_emit_comparison`,
//!   `nir_to_vir.c:1183-1226`): FCMP(x, y) Z = x == y, N = x < y, C = x <= y
//!   (`fge(a, b)` is FCMP(b, a).pushc + ifa); XOR Z = result == 0 (`ieq`);
//!   MIN(x, y) C = x > y signed (`ilt(a, b)` is MIN(b, a).pushc + ifa); SUB(x,
//!   y) C = x < y unsigned (`ult`); FMOV Z = x == 0.0, N = x < 0.0
//!   (`nir_to_vir.c:1010-1012`); MOV Z = bits == 0 (`nir_to_vir.c:909`).
//! * The branch conditions (`enum v3d_qpu_branch_cond`, `qpu_instr.h:386-394`)
//!   combine A over the lanes: ALLA, ANYA, ALLNA, ANYNA
//!   (`nir_to_vir.c:4333-4378`); with `msfign P` a lane whose multisample flag
//!   is 0 is left out (`qpu_instr.h:396-404`).
//!
//! A lane that never had a defined flag is undefined, not 0 or 1: the
//! interpreter says so by an undefined result lane (`None` here), and
//! refuses a branch on it. If the interpreter ever differs from the above,
//! the case is `#[ignore]`d with the reason; none is at present.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::encode71;
use yakogl::shaderc::online::v3d::isa::small_imm_index;
use yakogl::shaderc::online::v3d::isa71::*;
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, LANES};

type Lanes = [Option<u32>; LANES];

/// Lane `l` of rf3 (the fragment W): `l - 8`, so it is negative in lanes 0..7,
/// zero in lane 8 and positive after.
fn sa(l: usize) -> i32 {
    l as i32 - 8
}

/// Lane `l` of rf2 (the fragment Z): `(5 l) mod 7 - 3`, which is 0 in lanes 0,
/// 7 and 14 and differs from `sa` in most lanes.
fn sb(l: usize) -> i32 {
    (5 * l as i32) % 7 - 3
}

/// Lane `l` of rf1 (the fragment centroid W), a float: `(l - 8) / 2`.
fn sf(l: usize) -> f32 {
    (l as f32 - 8.0) * 0.5
}

fn inputs() -> Inputs {
    let w = std::array::from_fn(|l| sa(l) as u32);
    let z = std::array::from_fn(|l| sb(l) as u32);
    let wc = std::array::from_fn(|l| sf(l).to_bits());
    Inputs::Fragment(Box::new(FsInputs {
        w,
        w_centroid: wc,
        z,
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    }))
}

const A: u8 = 3; // rf3 = sa
const B: u8 = 2; // rf2 = sb
const F: u8 = 1; // rf1 = sf

fn rf(raddr: u8) -> Operand {
    Operand { raddr, unpack: Unpack71::None }
}

fn imm(v: u32) -> Operand {
    Operand {
        raddr: small_imm_index(v).unwrap_or_else(|| panic!("{v:#x} is not a small immediate")),
        unpack: Unpack71::None,
    }
}

fn nop() -> Alu71 {
    Alu71::nop()
}

fn add(op: AddOp71, waddr: u8, magic: bool, a: Operand, b: Operand) -> Alu71 {
    let mut i = nop();
    i.add = AddAlu71 { op, a, b, waddr, magic, pack: OutPack::None };
    i
}

fn mul(op: MulOp71, waddr: u8, magic: bool, a: Operand, b: Operand) -> Alu71 {
    let mut i = nop();
    i.mul = MulAlu71 { op, a, b, waddr, magic, pack: OutPack::None };
    i
}

/// Mark the operands of `i` that are small immediates (`vals` order: add a, add
/// b, mul a, mul b, as in `Alu71::operands`).
fn with_imm(mut i: Alu71, which: [bool; 4]) -> Alu71 {
    i.sig.small_imm_a = which[0];
    i.sig.small_imm_b = which[1];
    i.sig.small_imm_c = which[2];
    i.sig.small_imm_d = which[3];
    i
}

/// `rfN = value` (a small immediate) on the add ALU.
fn set(r: u8, v: u32) -> Alu71 {
    with_imm(add(AddOp71::Mov, r, false, imm(v), Operand::default()), [true, false, false, false])
}

/// The add ALU's op writing nothing (magic NOP), pushing or updating flags.
fn flag_add(op: AddOp71, a: Operand, b: Operand, apf: Pf, auf: Uf) -> Alu71 {
    let mut i = add(op, WADDR_NOP, true, a, b);
    i.flags.apf = apf;
    i.flags.auf = auf;
    i
}

fn flag_mul(op: MulOp71, a: Operand, b: Operand, mpf: Pf, muf: Uf) -> Alu71 {
    let mut i = mul(op, WADDR_NOP, true, a, b);
    i.flags.mpf = mpf;
    i.flags.muf = muf;
    i
}

/// `mov.<cond> rfD, src` on the add ALU; `src_imm` if `src` is a small immediate.
fn mov_if(cond: Cond, d: u8, src: Operand, src_imm: bool) -> Alu71 {
    let mut i = with_imm(add(AddOp71::Mov, d, false, src, Operand::default()), [src_imm, false, false, false]);
    i.flags.ac = cond;
    i
}

/// The same on the mul ALU.
fn mmov_if(cond: Cond, d: u8, src: Operand, src_imm: bool) -> Alu71 {
    let mut i = with_imm(mul(MulOp71::Mov, d, false, src, Operand::default()), [false, false, src_imm, false]);
    i.flags.mc = cond;
    i
}

fn to_tlb(r: u8) -> Alu71 {
    mul(MulOp71::Mov, 7, true, rf(r), Operand::default())
}

fn thrsw(mut a: Alu71) -> Alu71 {
    a.sig.thrsw = true;
    a
}

fn instr_words(code: &[Instr71]) -> Vec<u64> {
    code.iter().map(|i| encode71(i).unwrap_or_else(|e| panic!("{i:?}: {e:?}"))).collect()
}

/// `body` between the last-section THRSW pair and the program end.
fn program(body: &[Instr71]) -> Vec<u64> {
    let mut code = vec![Instr71::Alu(thrsw(nop())), Instr71::Alu(thrsw(nop()))];
    code.extend_from_slice(body);
    code.extend([Instr71::Alu(thrsw(nop())), Instr71::Alu(nop()), Instr71::Alu(nop())]);
    instr_words(&code)
}

fn alus(body: &[Alu71]) -> Vec<Instr71> {
    body.iter().map(|a| Instr71::Alu(*a)).collect()
}

fn run_instrs(body: &[Instr71]) -> Result<Outcome, String> {
    sim::run_gen(Gen::V71, &program(body), &[], &inputs(), &Env::default())
}

/// Run `body`, then write each of `outs` to the TLB; one lane array per register.
fn run(body: &[Alu71], outs: &[u8]) -> Result<Vec<Lanes>, String> {
    let mut code = alus(body);
    code.extend(outs.iter().map(|r| Instr71::Alu(to_tlb(*r))));
    let o = run_instrs(&code)?;
    assert_eq!(o.tlb.len(), outs.len());
    Ok(o.tlb.iter().map(|t| t.value).collect())
}

/// Like [`run`], for a program that must run.
fn ok(body: &[Alu71], outs: &[u8]) -> Vec<Lanes> {
    run(body, outs).unwrap_or_else(|e| panic!("does not run: {e}"))
}

/// All lanes defined: the values.
fn vals(l: &Lanes) -> [u32; LANES] {
    std::array::from_fn(|i| l[i].unwrap_or_else(|| panic!("lane {i} is undefined: {l:?}")))
}

/// A register holding `old` that a conditional write of `new` was applied to
/// in the lanes `sel` selects.
fn merged(sel: impl Fn(usize) -> bool, old: u32, new: u32) -> [u32; LANES] {
    std::array::from_fn(|l| if sel(l) { new } else { old })
}

/// The flags A and B of one lane, as the test's own model of the above says
/// they are.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fl {
    a: Option<bool>,
    b: Option<bool>,
}

/// `rf10 = 100`, then `mov.<cond> rf10, 1` ([`set`] with a condition) and
/// `mov.<opposite> rf11, 2` after `rf11 = 200`: both conditions of a pair on
/// the flags `pre` leaves.
fn observe(pre: &[Alu71]) -> (Vec<Lanes>, Vec<Lanes>, Vec<Lanes>, Vec<Lanes>) {
    let go = |cond: Cond| {
        let mut body = vec![set(10, 12)];
        body.extend_from_slice(pre);
        body.push(mov_if(cond, 10, imm(3), true));
        run(&body, &[10])
    };
    (
        go(Cond::IfA).unwrap_or_default(),
        go(Cond::IfNA).unwrap_or_default(),
        go(Cond::IfB).unwrap_or_default(),
        go(Cond::IfNB).unwrap_or_default(),
    )
}

/// What `observe`'s four runs must give per lane given the flags the model says.
fn check_conds(pre: &[Alu71], model: impl Fn(usize) -> Fl) {
    let (ifa, ifna, ifb, ifnb) = observe(pre);
    for (name, got, pick) in [
        ("ifa", &ifa, (|f: Fl| f.a) as fn(Fl) -> Option<bool>),
        ("ifna", &ifna, |f: Fl| f.a.map(|x| !x)),
        ("ifb", &ifb, |f: Fl| f.b),
        ("ifnb", &ifnb, |f: Fl| f.b.map(|x| !x)),
    ] {
        assert_eq!(got.len(), 1, "{name}: the program did not run");
        for l in 0..LANES {
            // Selected lane: the new value 3; unselected: the old 12; an
            // undefined flag: the lane may or may not have been written, so
            // it is undefined.
            let want = match pick(model(l)) {
                Some(true) => Some(3),
                Some(false) => Some(12),
                None => None,
            };
            assert_eq!(got[0][l], want, "{name}, lane {l}, flags {:?}", model(l));
        }
    }
}

// ---------------------------------------------------------------------------
// Pushes.
// ---------------------------------------------------------------------------

/// `mov.pushz -, rf3`: A = (rf3 == 0), only lane 8; B stays undefined.
#[test]
fn pushz_sets_a_and_the_four_conditions_read_it() {
    let pre = [flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None)];
    check_conds(&pre, |l| Fl { a: Some(sa(l) == 0), b: None });
}

/// A second push moves A to B (`qpu_instr.c:306-316` names it a push; the
/// interpreter: B = A, A = new): ifb then selects the first push's lanes.
#[test]
fn a_second_push_moves_a_to_b() {
    let pre = [
        flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None),
        flag_add(AddOp71::Mov, rf(B), Operand::default(), Pf::PushZ, Uf::None),
    ];
    check_conds(&pre, |l| Fl { a: Some(sb(l) == 0), b: Some(sa(l) == 0) });
}

/// Three pushes: only the last two survive.
#[test]
fn a_third_push_drops_the_first() {
    let pre = [
        flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None),
        flag_add(AddOp71::Mov, rf(B), Operand::default(), Pf::PushZ, Uf::None),
        flag_add(AddOp71::Xor, rf(A), rf(B), Pf::PushZ, Uf::None),
    ];
    check_conds(&pre, |l| Fl { a: Some(sa(l) == sb(l)), b: Some(sb(l) == 0) });
}

/// The flag each producing op pushes, per Mesa's use of it (module notes).
#[test]
fn every_op_pushes_the_flag_mesa_uses_it_for() {
    let one_f = imm(1.0f32.to_bits());
    let cases: Vec<(&str, Alu71, Box<dyn Fn(usize) -> bool>)> = vec![
        ("xor.pushz (ieq)", flag_add(AddOp71::Xor, rf(A), rf(B), Pf::PushZ, Uf::None), Box::new(|l| sa(l) == sb(l))),
        (
            "fcmp.pushz (feq)",
            with_imm(flag_add(AddOp71::Fcmp, rf(F), one_f, Pf::PushZ, Uf::None), [false, true, false, false]),
            Box::new(|l| sf(l) == 1.0),
        ),
        (
            "fcmp.pushn (flt)",
            with_imm(flag_add(AddOp71::Fcmp, rf(F), one_f, Pf::PushN, Uf::None), [false, true, false, false]),
            Box::new(|l| sf(l) < 1.0),
        ),
        (
            "fcmp.pushc (fle, i.e. fge with swapped operands)",
            with_imm(flag_add(AddOp71::Fcmp, rf(F), one_f, Pf::PushC, Uf::None), [false, true, false, false]),
            Box::new(|l| sf(l) <= 1.0),
        ),
        (
            "min.pushc (ilt(a, b) = MIN(b, a))",
            flag_add(AddOp71::Min, rf(B), rf(A), Pf::PushC, Uf::None),
            // C = x > y with x = rf2, y = rf3, signed.
            Box::new(|l| sb(l) > sa(l)),
        ),
        (
            "sub.pushc (ult)",
            flag_add(AddOp71::Sub, rf(A), rf(B), Pf::PushC, Uf::None),
            // C = x < y unsigned.
            Box::new(|l| (sa(l) as u32) < (sb(l) as u32)),
        ),
        (
            "fmov.pushz",
            flag_add(AddOp71::Fmov, rf(F), Operand::default(), Pf::PushZ, Uf::None),
            Box::new(|l| sf(l) == 0.0),
        ),
        (
            "fmov.pushn (add ALU, new at 7.1)",
            flag_add(AddOp71::Fmov, rf(F), Operand::default(), Pf::PushN, Uf::None),
            Box::new(|l| sf(l) < 0.0),
        ),
    ];
    for (name, push, want) in cases {
        let (ifa, ..) = observe(&[push]);
        let got = &ifa[0];
        for l in 0..LANES {
            assert_eq!(got[l], Some(if want(l) { 3 } else { 12 }), "{name}, lane {l}");
        }
    }
}

/// The mul ALU pushes too (`mpush`): its own Z and N, with the conditions of
/// either ALU reading the result.
#[test]
fn the_mul_alu_pushes_and_its_condition_writes() {
    let pushz = flag_mul(MulOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None);
    let pushn = flag_mul(MulOp71::Fmov, rf(F), Operand::default(), Pf::PushN, Uf::None);
    check_conds(&[pushz], |l| Fl { a: Some(sa(l) == 0), b: None });
    check_conds(&[pushn], |l| Fl { a: Some(sf(l) < 0.0), b: None });
    // apush then mpush: the mul push moves the add push to B.
    let both = [
        flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None),
        flag_mul(MulOp71::Fmov, rf(F), Operand::default(), Pf::PushN, Uf::None),
    ];
    check_conds(&both, |l| Fl { a: Some(sf(l) < 0.0), b: Some(sa(l) == 0) });
    // The mul ALU's write under a condition: `fmul.ifa` on the mul half.
    let mut body = vec![set(10, 12), pushn];
    body.push(mmov_if(Cond::IfNA, 10, imm(3), true));
    let got = ok(&body, &[10]);
    assert_eq!(vals(&got[0]), merged(|l| sf(l) >= 0.0, 12, 3));
}

/// Both halves of one instruction conditional, on the flags of the previous
/// one: ifa on the add write, ifna on the mul write.
#[test]
fn add_and_mul_writes_in_one_instruction_read_the_old_flags() {
    let push = flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None);
    let mut both = nop();
    both = with_imm(both, [true, false, false, false]); // one small-immediate signal per word
    both.add =
        AddAlu71 { op: AddOp71::Mov, a: imm(3), b: Operand::default(), waddr: 10, magic: false, pack: OutPack::None };
    both.mul =
        MulAlu71 { op: MulOp71::Mov, a: rf(B), b: Operand::default(), waddr: 11, magic: false, pack: OutPack::None };
    both.flags.ac = Cond::IfA;
    both.flags.mc = Cond::IfNA;
    let got = ok(&[set(10, 12), set(11, 12), push, both], &[10, 11]);
    assert_eq!(vals(&got[0]), merged(|l| sa(l) == 0, 12, 3));
    assert_eq!(vals(&got[1]), std::array::from_fn(|l| if sa(l) != 0 { sb(l) as u32 } else { 12 }));
}

/// A flag pushed in an instruction is not seen by that instruction's own
/// conditions (they read the flags from before it): `mov.pushz.ifa`.
#[test]
fn an_instruction_reads_the_flags_from_before_it() {
    // Flags first: A = (rf3 == 0). Then in one instruction: a write of rf10
    // under ifa, and a push from rf2 that would change A.
    let first = flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None);
    let mut both = nop();
    both = with_imm(both, [true, false, false, false]);
    both.add =
        AddAlu71 { op: AddOp71::Mov, a: imm(3), b: Operand::default(), waddr: 10, magic: false, pack: OutPack::None };
    both.flags.ac = Cond::IfA;
    both.flags.mpf = Pf::PushZ;
    both.mul = MulAlu71 {
        op: MulOp71::Mov,
        a: rf(B),
        b: Operand::default(),
        waddr: WADDR_NOP,
        magic: true,
        pack: OutPack::None,
    };
    let got = ok(&[set(10, 12), first, both], &[10]);
    assert_eq!(vals(&got[0]), merged(|l| sa(l) == 0, 12, 3));
}

// ---------------------------------------------------------------------------
// Updates.
// ---------------------------------------------------------------------------

/// The 12 update forms: (form, source flag, X inverted, `and` (else `nor`)),
/// A' = A & X' for `and`, !A & !X' for `nor`, X' = X or !X (`qpu_instr.h:77-91`
/// order: ANDZ, ANDNZ, NORNZ, NORZ, ANDN, ANDNN, NORNN, NORN, ANDC, ANDNC,
/// NORNC, NORC).
const FORMS: [(Uf, char, bool, bool); 12] = [
    (Uf::AndZ, 'z', false, true),
    (Uf::AndNZ, 'z', true, true),
    (Uf::NorNZ, 'z', true, false),
    (Uf::NorZ, 'z', false, false),
    (Uf::AndN, 'n', false, true),
    (Uf::AndNN, 'n', true, true),
    (Uf::NorNN, 'n', true, false),
    (Uf::NorN, 'n', false, false),
    (Uf::AndC, 'c', false, true),
    (Uf::AndNC, 'c', true, true),
    (Uf::NorNC, 'c', true, false),
    (Uf::NorC, 'c', false, false),
];

/// A source op giving flag `which`, with X per lane: Z from `mov rf2`
/// (rf2 == 0), N from `fmov rf1` (rf1 < 0), C from `sub rf3, rf2` (rf3 <u rf2).
fn source(which: char, uf: Uf) -> (Alu71, Box<dyn Fn(usize) -> bool>) {
    match which {
        'z' => (flag_add(AddOp71::Mov, rf(B), Operand::default(), Pf::None, uf), Box::new(|l| sb(l) == 0)),
        'n' => (flag_add(AddOp71::Fmov, rf(F), Operand::default(), Pf::None, uf), Box::new(|l| sf(l) < 0.0)),
        _ => (flag_add(AddOp71::Sub, rf(A), rf(B), Pf::None, uf), Box::new(|l| (sa(l) as u32) < (sb(l) as u32))),
    }
}

/// Every update form on a defined A (`mov.pushz -, rf3`: lane 8 only is 1),
/// as the table says; ifa and ifna read the result.
#[test]
fn every_update_form_combines_a_with_the_op_flag() {
    for (uf, which, inv, and) in FORMS {
        let (src, x) = source(which, uf);
        // A starts as a push of the lanes `sa < 3`, so both values occur in
        // lanes with both values of X.
        let init = with_imm(
            flag_add(AddOp71::Fcmp, rf(F), imm(2.0f32.to_bits()), Pf::PushN, Uf::None),
            [false, true, false, false],
        );
        let model = |l: usize| {
            let a0 = sf(l) < 2.0;
            let x = x(l) != inv;
            if and {
                a0 && x
            } else {
                !a0 && !x
            }
        };
        let (ifa, ifna, ..) = observe(&[init, src]);
        for l in 0..LANES {
            assert_eq!(ifa[0][l], Some(if model(l) { 3 } else { 12 }), "{uf:?} ifa, lane {l}");
            assert_eq!(ifna[0][l], Some(if model(l) { 12 } else { 3 }), "{uf:?} ifna, lane {l}");
        }
    }
}

/// Mesa's divergent-code idiom `mov.nornz -, exec` read by `ifna`
/// (`nir_to_vir.c:3486-3492`): exec is 0 in the active lanes, so A' = !A & (exec
/// == 0): the lanes where the condition is false and exec is active.
#[test]
fn the_execute_mask_idiom_nornz_then_ifna() {
    let exec = |l: usize| u32::from(sa(l) > 1); // lanes 10..15 are inactive
    let body = [
        // rf20 = exec: 0, then 1 where rf3 > 1 (min.pushc: C = x > y).
        set(20, 0),
        with_imm(flag_add(AddOp71::Min, rf(A), imm(1), Pf::PushC, Uf::None), [false, true, false, false]),
        mov_if(Cond::IfA, 20, imm(1), true),
        // A = the condition, sf < 2 (lanes 0..11).
        with_imm(
            flag_add(AddOp71::Fcmp, rf(F), imm(2.0f32.to_bits()), Pf::PushN, Uf::None),
            [false, true, false, false],
        ),
        flag_add(AddOp71::Mov, rf(20), Operand::default(), Pf::None, Uf::NorNZ),
        set(21, 0),
        mov_if(Cond::IfNA, 21, imm(1), true),
    ];
    let got = ok(&body, &[21, 20]);
    assert_eq!(vals(&got[1]), std::array::from_fn(|l| exec(l)));
    // NORNZ: A' = !A & Z, Z = exec == 0; ifna writes where A' is false.
    let want = |l: usize| {
        let a1 = !(sf(l) < 2.0) && exec(l) == 0;
        u32::from(!a1)
    };
    assert_eq!(vals(&got[0]), std::array::from_fn(want));
}

/// An update leaves B undefined (nothing says otherwise), so `ifb` after one
/// is refused rather than guessed.
#[test]
fn b_after_an_update_is_undefined_and_ifb_refuses() {
    let push = flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None);
    let push2 = flag_add(AddOp71::Mov, rf(B), Operand::default(), Pf::PushZ, Uf::None);
    let upd = flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::None, Uf::AndZ);
    // Before the update B is defined; after it, a TLB write under ifb is a
    // sink and is refused.
    let mut to_tlb_ifb = to_tlb(A);
    to_tlb_ifb.flags.mc = Cond::IfB;
    let e = run_instrs(&alus(&[push, push2, upd, to_tlb_ifb])).unwrap_err();
    assert!(e.contains("IfB on an undefined flag") || e.contains("conditional"), "{e}");
}

/// The three-valued rule: with A undefined, `and` clears the lanes whose X is
/// false, and `nor` the lanes whose X is true, whatever A was; in the others
/// A stays undefined.
#[test]
fn an_update_on_an_undefined_a_is_three_valued() {
    // `mov.andz -, rf3`: X = rf3 == 0, true in lane 8 only. A & X is false
    // in every other lane (defined), undefined in lane 8.
    let upd = flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::None, Uf::AndZ);
    let got = ok(&[set(10, 12), upd, mov_if(Cond::IfNA, 10, imm(3), true)], &[10]);
    for l in 0..LANES {
        let want = if sa(l) == 0 { None } else { Some(3) };
        assert_eq!(got[0][l], want, "andz on undefined A, lane {l}");
    }
    // `mov.norz -, rf3`: !A & !X; !X true (rf3 != 0) with A undefined: undefined; X true: false.
    let upd = flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::None, Uf::NorZ);
    let got = ok(&[set(10, 12), upd, mov_if(Cond::IfA, 10, imm(3), true)], &[10]);
    for l in 0..LANES {
        let want = if sa(l) == 0 { Some(12) } else { None };
        assert_eq!(got[0][l], want, "norz on undefined A, lane {l}");
    }
}

/// A conditional write on a flag nothing ever defined leaves the lane undefined.
#[test]
fn a_condition_on_a_never_written_flag_leaves_the_lane_undefined() {
    let got = ok(&[set(10, 12), mov_if(Cond::IfA, 10, imm(3), true)], &[10]);
    assert!(got[0].iter().all(Option::is_none), "{:?}", got[0]);
}

// ---------------------------------------------------------------------------
// Branches.
// ---------------------------------------------------------------------------

fn branch(cond: BranchCond, msfign: Msfign) -> Instr71 {
    Instr71::Branch(Branch {
        cond,
        msfign,
        bdi: BranchDest::Rel,
        ub: false,
        bdu: BranchDest::Abs,
        raddr_a: 0,
        // Relative to the branch + 4 (the delay slots): skip one instruction.
        offset: 8,
    })
}

/// Whether the branch was taken, given a flag-making prefix: rf10 is 0, the
/// prefix runs, a branch with three delay slots follows and the instruction it
/// skips would set rf10 to 1.
fn taken(pre: &[Alu71], cond: BranchCond, msfign: Msfign) -> Result<bool, String> {
    let mut code = alus(&[set(10, 0)]);
    code.extend(alus(pre));
    code.push(Instr71::Alu(nop()));
    code.push(branch(cond, msfign));
    code.extend(alus(&[nop(), nop(), nop(), set(10, 1), to_tlb(10)]));
    let o = run_instrs(&code)?;
    Ok(vals(&o.tlb[0].value)[0] == 0)
}

/// A = (rf3 == 0) is true in lane 8 alone (mixed); A over a register of 0 is
/// true everywhere; over a non-zero small immediate nowhere.
fn a_mixed() -> Alu71 {
    flag_add(AddOp71::Mov, rf(A), Operand::default(), Pf::PushZ, Uf::None)
}

fn a_all() -> Vec<Alu71> {
    vec![set(20, 0), flag_add(AddOp71::Mov, rf(20), Operand::default(), Pf::PushZ, Uf::None)]
}

fn a_none() -> Vec<Alu71> {
    vec![set(20, 5), flag_add(AddOp71::Mov, rf(20), Operand::default(), Pf::PushZ, Uf::None)]
}

#[test]
fn branch_conditions_over_the_lanes_of_a() {
    use BranchCond::*;
    // (A's lanes, cond, taken): ALLA all true; ANYA some; ALLNA none true;
    // ANYNA some false (`qpu_instr.h:386-394`).
    let table: [(&str, Vec<Alu71>, [(BranchCond, bool); 5]); 3] = [
        ("all lanes true", a_all(), [(Always, true), (AllA, true), (AnyA, true), (AllNA, false), (AnyNA, false)]),
        ("no lane true", a_none(), [(Always, true), (AllA, false), (AnyA, false), (AllNA, true), (AnyNA, true)]),
        ("lane 8 true", vec![a_mixed()], [(Always, true), (AllA, false), (AnyA, true), (AllNA, false), (AnyNA, true)]),
    ];
    for (what, pre, rows) in table {
        for (cond, want) in rows {
            assert_eq!(taken(&pre, cond, Msfign::None), Ok(want), "{what}, {cond:?}");
        }
    }
}

/// The branch reads A, not B: after a second push with the opposite pattern
/// the branch follows the newest push.
#[test]
fn a_branch_reads_a_after_the_latest_push() {
    let pre = [a_mixed(), flag_add(AddOp71::Mov, rf(B), Operand::default(), Pf::PushZ, Uf::None)];
    // A = rf2 == 0 (lanes 0, 7, 14): some, not all.
    assert_eq!(taken(&pre, BranchCond::AnyA, Msfign::None), Ok(true));
    assert_eq!(taken(&pre, BranchCond::AllNA, Msfign::None), Ok(false));
    // A updated by andnz with an op flag that is true everywhere (rf20 == 0,
    // Z true): A & !X = false everywhere.
    let mut pre = a_all();
    pre.push(flag_add(AddOp71::Mov, rf(20), Operand::default(), Pf::None, Uf::AndNZ));
    assert_eq!(taken(&pre, BranchCond::AllNA, Msfign::None), Ok(true));
}

/// `msfign P`: a lane whose multisample flag is 0 does not count
/// (`qpu_instr.h:396-404`). SETMSF makes lane 8 (rf3 == 0) such a lane; A is
/// true in lane 8 alone.
#[test]
fn msfign_p_leaves_out_lanes_with_no_multisample_flag() {
    let setmsf = {
        let mut i = with_imm(nop(), [false; 4]);
        i.add = AddAlu71 {
            op: AddOp71::Setmsf,
            a: rf(A),
            b: Operand::default(),
            waddr: WADDR_NOP,
            magic: true,
            pack: OutPack::None,
        };
        i
    };
    let pre = [setmsf, a_mixed()];
    // Without the ignore: lane 8 is in, A is true there.
    assert_eq!(taken(&pre, BranchCond::AnyA, Msfign::None), Ok(true));
    assert_eq!(taken(&pre, BranchCond::AllNA, Msfign::None), Ok(false));
    // With it, lane 8 is out and the remaining lanes are all not-A.
    assert_eq!(taken(&pre, BranchCond::AnyA, Msfign::P), Ok(false));
    assert_eq!(taken(&pre, BranchCond::AllNA, Msfign::P), Ok(true));
}

/// A branch on a lane whose A is undefined is refused, not guessed; ALWAYS has
/// no condition to read.
#[test]
fn a_branch_on_an_undefined_a_is_refused() {
    let e = taken(&[], BranchCond::AnyA, Msfign::None).unwrap_err();
    assert!(e.contains("a branch on an undefined A in lane 0"), "{e}");
    assert_eq!(taken(&[], BranchCond::Always, Msfign::None), Ok(true));
}

/// The A0/NA0 conditions (lane 0's A) are not modelled; the interpreter says so.
#[test]
fn a0_and_na0_are_not_modelled() {
    for cond in [BranchCond::A0, BranchCond::NA0] {
        let e = taken(&a_all(), cond, Msfign::None).unwrap_err();
        assert!(e.contains("branch condition"), "{cond:?}: {e}");
    }
}
