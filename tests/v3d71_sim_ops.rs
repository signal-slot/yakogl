// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! sim71 (`src/shaderc/online/v3d/sim71.rs`, run through [`run71`]) checked
//! op by op on what a V3D 7.1 back end for GLSL ES 1.00 will emit: the
//! float and integer ALU operations of both units, the SFU as ALU
//! operations, input unpacks and output packs, the flags and conditions,
//! LDVARY / LDUNIF / LDUNIFRF / LDUNIFA, the TMU (general memory and a
//! texture lookup), THRSW and its delay slots, branches, and the TLB writes.
//!
//! Every program is built by hand from [`isa71`] types and encoded with
//! [`codec71::encode71`]. Every expected value below is written from the ISA
//! semantics (Mesa 26.2.1 `qpu_instr.h` / `qpu_pack.c`, and `nir_to_vir.c`
//! for what each NIR op lowers to: the semantics are named at each test),
//! never taken from a run of sim71. The SFU results are compared with a
//! relative tolerance, as `tests/v3d_sim_mesa.rs` does.
//!
//! A case sim71 gets wrong is an `#[ignore]` test whose reason names it; the
//! ops sim71 refuses ("unmodelled") are listed by `unmodelled_inventory`
//! (run with `--nocapture`).
//!
//! Features: --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::texref::Texture;
use yakogl::shaderc::online::v3d::codec71::{encode71, same_instr71};
use yakogl::shaderc::online::v3d::isa::{num_src, small_imm_index};
use yakogl::shaderc::online::v3d::isa71::*;
use yakogl::shaderc::online::v3d::sim::{Env, FsInputs, Inputs, Outcome, Region, Varying, LANES};
use yakogl::shaderc::online::v3d::sim71::run71;

// ---------------------------------------------------------------- builders

/// A destination: an address and whether it is a magic register.
type D = (u8, bool);

fn rfd(n: u8) -> D {
    (n, false)
}

const TLB: D = (7, true);
const TLBU: D = (8, true);
const NOPD: D = (WADDR_NOP, true);
const TMUD: D = (11, true);
const TMUA: D = (12, true);
const TMUS: D = (33, true);
const TMUT: D = (34, true);

/// An operand: a register-file address, a small immediate (by bit pattern),
/// the same with an unpack, or none.
#[derive(Clone, Copy)]
enum S {
    R(u8),
    I(u32),
    U(u8, Unpack71),
    N,
}

fn opd(s: S) -> (Operand, bool) {
    match s {
        S::R(r) => (Operand { raddr: r, unpack: Unpack71::None }, false),
        S::U(r, unpack) => (Operand { raddr: r, unpack }, false),
        S::I(v) => (
            Operand {
                raddr: small_imm_index(v).unwrap_or_else(|| panic!("{v:#x} is no small immediate")),
                unpack: Unpack71::None,
            },
            true,
        ),
        S::N => (Operand::default(), false),
    }
}

fn f(x: f32) -> u32 {
    x.to_bits()
}

/// An ALU instruction with only the add half: operands an operation does not
/// read are written as the encoding wants them (default).
fn add(op: AddOp71, d: D, a: S, b: S) -> Alu71 {
    let n = num_src(add_op_args(op));
    let mut i = Alu71::nop();
    let (oa, sa) = if n >= 1 { opd(a) } else { opd(S::N) };
    let (ob, sb) = if n >= 2 { opd(b) } else { opd(S::N) };
    i.sig.small_imm_a = sa;
    i.sig.small_imm_b = sb;
    i.add = AddAlu71 { op, a: oa, b: ob, waddr: d.0, magic: d.1, pack: OutPack::None };
    i
}

fn mul(op: MulOp71, d: D, a: S, b: S) -> Alu71 {
    let n = num_src(mul_op_args(op));
    let mut i = Alu71::nop();
    let (oa, sa) = if n >= 1 { opd(a) } else { opd(S::N) };
    let (ob, sb) = if n >= 2 { opd(b) } else { opd(S::N) };
    i.sig.small_imm_c = sa;
    i.sig.small_imm_d = sb;
    i.mul = MulAlu71 { op, a: oa, b: ob, waddr: d.0, magic: d.1, pack: OutPack::None };
    i
}

/// The add half of `a` and the mul half of `m` in one instruction.
fn both(a: Alu71, m: Alu71) -> Alu71 {
    let mut i = a;
    i.mul = m.mul;
    i.sig.small_imm_c = m.sig.small_imm_c;
    i.sig.small_imm_d = m.sig.small_imm_d;
    i
}

fn sig(mut a: Alu71, set: impl FnOnce(&mut Sig71)) -> Alu71 {
    set(&mut a.sig);
    a
}

fn thrsw() -> Alu71 {
    sig(Alu71::nop(), |s| s.thrsw = true)
}

fn nop() -> Alu71 {
    Alu71::nop()
}

fn with_flags(mut a: Alu71, set: impl FnOnce(&mut Flags)) -> Alu71 {
    set(&mut a.flags);
    a
}

/// `mov tlb, rfN` on the mul ALU (how Mesa writes a colour).
fn to_tlb(r: u8) -> Alu71 {
    mul(MulOp71::Mov, TLB, S::R(r), S::N)
}

fn ldunifrf(r: u8) -> Alu71 {
    let mut a = sig(nop(), |s| s.ldunifrf = true);
    a.sig_addr = r;
    a
}

fn ldunif() -> Alu71 {
    sig(nop(), |s| s.ldunif = true)
}

fn ldvary(r: u8) -> Alu71 {
    let mut a = sig(nop(), |s| s.ldvary = true);
    a.sig_addr = r;
    a
}

fn ldtmu(r: u8) -> Alu71 {
    let mut a = sig(nop(), |s| s.ldtmu = true);
    a.sig_addr = r;
    a
}

fn word(i: &Instr71) -> u64 {
    let w = encode71(i).unwrap_or_else(|e| panic!("{i:?}: {e:?}"));
    // The encoding is the one codec71's round trip pins; an instruction the
    // codec turns into another one is a test bug, not a sim71 one.
    let back = yakogl::shaderc::online::v3d::codec71::decode71(w).expect("decodes");
    assert!(same_instr71(i, &back), "{i:?} round-trips to {back:?}");
    w
}

/// A program body and the uniform stream it reads.
#[derive(Default)]
struct P {
    code: Vec<Instr71>,
    uni: Vec<u32>,
}

impl P {
    fn push(&mut self, a: Alu71) -> &mut P {
        self.code.push(Instr71::Alu(a));
        self
    }

    /// LDUNIFRF into `r` of the next uniform, which is `v`.
    fn ld(&mut self, r: u8, v: u32) -> &mut P {
        self.uni.push(v);
        self.push(ldunifrf(r))
    }

    fn tlb(&mut self, r: u8) -> &mut P {
        self.push(to_tlb(r))
    }

    fn nops(&mut self, n: usize) -> &mut P {
        for _ in 0..n {
            self.push(nop());
        }
        self
    }

    /// The whole program, as it stands.
    fn words(&self) -> Vec<u64> {
        self.code.iter().map(word).collect()
    }

    /// `pre`, then the last-section THRSW pair and one padding instruction
    /// (the pair's first THRSW is a switch three instructions later, and no
    /// LDVARY may sit in a THRSW's second delay slot), then the body, then
    /// the thread end: a THRSW and its two delay slots.
    fn framed(&self) -> P {
        let mut p = P { code: Vec::new(), uni: self.uni.clone() };
        p.push(thrsw()).push(thrsw()).push(nop());
        p.code.extend(self.code.iter().copied());
        p.push(thrsw()).push(nop()).push(nop());
        p
    }

    fn run(&self) -> Result<Outcome, String> {
        self.run_with(&fs_inputs(Vec::new()), &Env { threads: 1, ..Env::default() })
    }

    fn run_with(&self, inputs: &Inputs, env: &Env) -> Result<Outcome, String> {
        run71(&self.framed().words(), &self.uni, inputs, env)
    }

    fn run_raw(&self, inputs: &Inputs, env: &Env) -> Result<Outcome, String> {
        run71(&self.words(), &self.uni, inputs, env)
    }
}

const W: f32 = 2.0;
const Z: f32 = 0.5;

/// A fragment's inputs: W (rf3) 2.0, Z (rf2) 0.5, FXCD the lane number as a
/// float, FYCD 7.0.
fn fs_inputs(varyings: Vec<Varying>) -> Inputs {
    let all = |x: f32| [f(x); LANES];
    Inputs::Fragment(Box::new(FsInputs {
        w: all(W),
        w_centroid: all(W),
        z: all(Z),
        x: std::array::from_fn(|l| f(l as f32)),
        y: all(7.0),
        back_facing: [false; LANES],
        varyings,
    }))
}

/// The value of TLB write `n` in every lane (each must be defined).
fn lanes(o: &Outcome, n: usize) -> [u32; LANES] {
    let w = o.tlb.get(n).unwrap_or_else(|| panic!("no TLB write {n}: {:?}", o.tlb));
    std::array::from_fn(|l| w.value[l].unwrap_or_else(|| panic!("TLB write {n} lane {l} undefined")))
}

/// Same in every lane.
fn tlb(o: &Outcome, n: usize) -> u32 {
    let v = lanes(o, n);
    assert!(v.iter().all(|x| *x == v[0]), "TLB write {n}: {v:08x?}");
    v[0]
}

fn tlb_f(o: &Outcome, n: usize) -> f32 {
    f32::from_bits(tlb(o, n))
}

/// `d = op(a, b)` on the add ALU with `a` and `b` in rf10 and rf11 (loaded
/// from the uniform stream), the result of every lane.
fn calc_add(op: AddOp71, a: u32, b: u32) -> u32 {
    let mut p = P::default();
    p.ld(10, a).ld(11, b).push(add(op, rfd(12), S::R(10), S::R(11))).tlb(12);
    tlb(&p.run().unwrap_or_else(|e| panic!("{op:?}: {e}")), 0)
}

fn calc_mul(op: MulOp71, a: u32, b: u32) -> u32 {
    let mut p = P::default();
    p.ld(10, a).ld(11, b).push(mul(op, rfd(12), S::R(10), S::R(11))).tlb(12);
    tlb(&p.run().unwrap_or_else(|e| panic!("{op:?}: {e}")), 0)
}

// ------------------------------------------------------------------ the ALU

/// The add ALU's float operations, IEEE binary32 (`nir_to_vir.c`
/// `ntq_emit_alu`: fadd, fsub, fmin, fmax and the roundings).
#[test]
fn add_alu_float_ops() {
    use AddOp71::*;
    let cases: &[(AddOp71, f32, f32, f32)] = &[
        (Fadd, 1.5, 2.25, 3.75),
        (Faddnf, 1.5, 2.25, 3.75),
        (Fsub, 1.5, 2.25, -0.75),
        (Fmin, 1.5, -2.0, -2.0),
        (Fmax, 1.5, -2.0, 1.5),
        (Ffloor, -1.5, 0.0, -2.0),
        (Fceil, -1.5, 0.0, -1.0),
        (Ftrunc, -1.5, 0.0, -1.0),
        (Fround, 2.5, 0.0, 2.0), // fround_even: ties to even
        (Fround, 3.5, 0.0, 4.0),
        (Fround, -2.5, 0.0, -2.0),
    ];
    for &(op, a, b, want) in cases {
        let got = f32::from_bits(calc_add(op, f(a), f(b)));
        assert_eq!(got, want, "{op:?}({a}, {b})");
    }
}

/// Float to integer and back (`f2i32` is FTOIZ, `f2u32` FTOUZ, and FTOIN is
/// `f2i32` of `fround_even`, `nir_to_vir.c:1452-1458`).
#[test]
fn add_alu_conversions() {
    use AddOp71::*;
    assert_eq!(calc_add(Ftoiz, f(-2.7), 0) as i32, -2);
    assert_eq!(calc_add(Ftoiz, f(2.7), 0) as i32, 2);
    assert_eq!(calc_add(Ftouz, f(2.7), 0), 2);
    assert_eq!(calc_add(Ftoin, f(2.5), 0) as i32, 2);
    assert_eq!(calc_add(Ftoin, f(-3.5), 0) as i32, -4);
    assert_eq!(calc_add(Ftoin, f(3.5), 0) as i32, 4);
    assert_eq!(f32::from_bits(calc_add(Itof, (-3i32) as u32, 0)), -3.0);
    assert_eq!(f32::from_bits(calc_add(Utof, 0x8000_0000, 0)), 2147483648.0);
    assert_eq!(f32::from_bits(calc_add(Utof, 7, 0)), 7.0);
}

/// `pack_half_2x16_split(a, b)`: a to the low half, b to the high
/// (`nir_to_vir.c:1807`); 1.0 is 0x3c00 and 0.5 is 0x3800 as an f16.
#[test]
fn add_alu_vfpack() {
    assert_eq!(calc_add(AddOp71::Vfpack, f(1.0), f(0.5)), 0x3800_3c00);
    assert_eq!(calc_add(AddOp71::Vfpack, f(-2.0), f(0.0)), 0x0000_c000);
}

/// The add ALU's integer operations, i32/u32 two's complement, shift counts
/// mod 32.
#[test]
fn add_alu_integer_ops() {
    use AddOp71::*;
    let cases: &[(AddOp71, u32, u32, u32)] = &[
        (Add, 7, 5, 12),
        (Add, 0xffff_ffff, 2, 1),
        (Sub, 5, 7, 0xffff_fffe),
        (And, 0xff00_ff00, 0x0ff0_0ff0, 0x0f00_0f00),
        (Or, 0xff00_ff00, 0x0ff0_0ff0, 0xfff0_fff0),
        (Xor, 0xff00_ff00, 0x0ff0_0ff0, 0xf0f0_f0f0),
        (Min, 0xffff_fffd, 2, 0xffff_fffd), // signed: -3 < 2
        (Max, 0xffff_fffd, 2, 2),
        (Umin, 0xffff_fffd, 2, 2), // unsigned: 0xfffffffd > 2
        (Umax, 0xffff_fffd, 2, 0xffff_fffd),
        (Shl, 1, 4, 16),
        (Shl, 1, 33, 2),
        (Shr, 0x8000_0000, 31, 1),
        (Asr, 0x8000_0000, 31, 0xffff_ffff),
        (Asr, 0x4000_0000, 30, 1),
        (Ror, 1, 1, 0x8000_0000),
        (Ror, 0x8000_0001, 4, 0x1800_0000),
        (Not, 0, 0, 0xffff_ffff),
        (Neg, 5, 0, 0xffff_fffb),
        (Clz, 0x00ff_0000, 0, 8),
        (Clz, 0, 0, 32),
    ];
    for &(op, a, b, want) in cases {
        assert_eq!(calc_add(op, a, b), want, "{op:?}({a:#x}, {b:#x})");
    }
}

/// The mul ALU: integer add and sub (7.1 has them on both), UMUL24 and
/// SMUL24 (the low 24 bits of each operand, signed for SMUL24), FMUL, FMOV
/// and MOV.
#[test]
fn mul_alu_ops() {
    use MulOp71::*;
    assert_eq!(calc_mul(Add, 7, 5), 12);
    assert_eq!(calc_mul(Sub, 5, 7), 0xffff_fffe);
    assert_eq!(calc_mul(Umul24, 0xff00_0005, 3), 15);
    assert_eq!(calc_mul(Umul24, 0x00ff_ffff, 2), 0x01ff_fffe);
    assert_eq!(calc_mul(Smul24, 0x00ff_ffff, 5) as i32, -5); // the low 24 bits are -1
    assert_eq!(calc_mul(Smul24, 0x0000_0007, 0x00ff_fffe) as i32, -14);
    assert_eq!(f32::from_bits(calc_mul(Fmul, f(1.5), f(-2.0))), -3.0);
    assert_eq!(f32::from_bits(calc_mul(Fmul, f(0.0), f(-2.0))), -0.0);
    assert_eq!(calc_mul(Fmov, f(-1.25), 0), f(-1.25));
    assert_eq!(calc_mul(Mov, 0xdead_beef, 0), 0xdead_beef);
}

/// Both halves in one instruction, and a small immediate as an operand
/// (2.0 and 0.5 are in the table, as are the integers 0..15). At 7.1 an
/// instruction has one small-immediate signal (`v3d71_sig_map`: SMIMM_A ..
/// SMIMM_D are signals of their own, none combines with another), so the
/// other half reads registers.
#[test]
fn both_halves_and_small_immediates() {
    let mut p = P::default();
    p.ld(10, f(3.0));
    p.push(both(add(AddOp71::Fadd, rfd(11), S::R(10), S::I(f(2.0))), mul(MulOp71::Fmul, rfd(12), S::R(10), S::R(10))));
    p.push(both(add(AddOp71::Add, rfd(13), S::R(10), S::R(10)), mul(MulOp71::Sub, rfd(14), S::R(10), S::I(2))));
    p.tlb(11).tlb(12).tlb(13).tlb(14);
    let o = p.run().expect("runs");
    assert_eq!(tlb_f(&o, 0), 5.0);
    assert_eq!(tlb_f(&o, 1), 9.0);
    assert_eq!(tlb(&o, 2), f(3.0).wrapping_mul(2));
    assert_eq!(tlb(&o, 3), f(3.0).wrapping_sub(2));
}

/// A register write lands at the next instruction: a read in the same
/// instruction sees the old value (both halves swap two registers), the next
/// instruction sees the new one.
#[test]
fn register_writes_land_at_the_next_instruction() {
    let mut p = P::default();
    p.ld(10, 5).ld(11, 9);
    p.push(both(add(AddOp71::Mov, rfd(10), S::R(11), S::N), mul(MulOp71::Mov, rfd(11), S::R(10), S::N)));
    p.push(add(AddOp71::Add, rfd(12), S::R(10), S::R(11)));
    p.tlb(10).tlb(11).tlb(12);
    let o = p.run().expect("runs");
    assert_eq!((tlb(&o, 0), tlb(&o, 1), tlb(&o, 2)), (9, 5, 14));
}

/// f32 to f16 rounds to nearest even (NIR's `pack_half_2x16_split`):
/// 0.1f32 is 0x2e66 (0.0999756), 1/3 is 0x3555, 1.0009765625 (1 + 2^-10) is
/// exact (0x3c01) and 1 + 2^-11 is a tie, to the even 0x3c00.
#[test]
fn vfpack_rounds_to_nearest_even() {
    assert_eq!(calc_add(AddOp71::Vfpack, f(0.1), f(1.0 / 3.0)), 0x3555_2e66);
    assert_eq!(calc_add(AddOp71::Vfpack, f(1.0 + 1.0 / 1024.0), f(1.0 + 1.0 / 2048.0)), 0x3c00_3c01);
}

/// Lanes are independent: EIDX is the lane number, FXCD the lane as a float.
#[test]
fn lanes_are_independent() {
    let mut p = P::default();
    p.push(add(AddOp71::Eidx, rfd(10), S::N, S::N));
    p.push(add(AddOp71::Fxcd, rfd(11), S::N, S::N));
    p.ld(12, 100);
    p.push(add(AddOp71::Add, rfd(13), S::R(10), S::R(12)));
    p.push(mul(MulOp71::Fmul, rfd(14), S::R(11), S::I(f(2.0))));
    p.tlb(13).tlb(14);
    let o = p.run().expect("runs");
    for l in 0..LANES {
        assert_eq!(lanes(&o, 0)[l], 100 + l as u32);
        assert_eq!(f32::from_bits(lanes(&o, 1)[l]), 2.0 * l as f32);
    }
}

// ------------------------------------------------------- unpacks and packs

/// An f16 input unpack: L and H take the low or high half as an f16
/// (`qpu_instr.h` `V3D_QPU_UNPACK_L/H`); ABS clears the sign.
#[test]
fn float_unpacks() {
    let two_one = 0x3c00_4000; // H = 1.0, L = 2.0
    let mut p = P::default();
    p.ld(10, two_one).ld(11, f(-1.5)).ld(12, f(10.0));
    p.push(add(AddOp71::Fmov, rfd(13), S::U(10, Unpack71::L), S::N));
    p.push(add(AddOp71::Fmov, rfd(14), S::U(10, Unpack71::H), S::N));
    p.push(add(AddOp71::Fadd, rfd(15), S::U(11, Unpack71::Abs), S::R(12)));
    p.push(mul(MulOp71::Fmul, rfd(16), S::U(10, Unpack71::H), S::R(12)));
    p.push(mul(MulOp71::Fmov, rfd(17), S::U(11, Unpack71::Abs), S::N));
    p.tlb(13).tlb(14).tlb(15).tlb(16).tlb(17);
    let o = p.run().expect("runs");
    assert_eq!(tlb_f(&o, 0), 2.0);
    assert_eq!(tlb_f(&o, 1), 1.0);
    assert_eq!(tlb_f(&o, 2), 11.5);
    assert_eq!(tlb_f(&o, 3), 10.0);
    assert_eq!(tlb_f(&o, 4), 1.5);
}

/// 7.1's FMOV clamps (SAT [0, 1], NSAT [-1, 1], MAX0 [0, +inf]) and MOV's
/// integer unpacks (UL, UH, IL, IH: a half zero- or sign-extended), on the
/// add ALU, and MOV's on the mul ALU (`qpu_instr.h:320-346`).
#[test]
fn seven_one_unpacks() {
    let mut p = P::default();
    p.ld(10, f(1.5)).ld(11, f(-1.5)).ld(12, f(0.25)).ld(13, 0x8001_fffe);
    let fm = |d: u8, r: u8, u| add(AddOp71::Fmov, rfd(d), S::U(r, u), S::N);
    p.push(fm(20, 10, Unpack71::Sat));
    p.push(fm(21, 11, Unpack71::Sat));
    p.push(fm(22, 12, Unpack71::Sat));
    p.push(fm(23, 11, Unpack71::Nsat));
    p.push(fm(24, 10, Unpack71::Nsat));
    p.push(fm(25, 11, Unpack71::Max0));
    p.push(fm(26, 10, Unpack71::Max0));
    for (d, u) in [(27, Unpack71::Ul), (28, Unpack71::Uh), (29, Unpack71::Il), (30, Unpack71::Ih)] {
        p.push(add(AddOp71::Mov, rfd(d), S::U(13, u), S::N));
    }
    for (d, u) in [(31, Unpack71::Ul), (32, Unpack71::Uh), (33, Unpack71::Il), (34, Unpack71::Ih)] {
        p.push(mul(MulOp71::Mov, rfd(d), S::U(13, u), S::N));
    }
    for r in (20..=34).step_by(1) {
        p.tlb(r);
    }
    let o = p.run().expect("runs");
    let want_f = [1.0, 0.0, 0.25, -1.0, 1.0, 0.0, 1.5];
    for (i, w) in want_f.iter().enumerate() {
        assert_eq!(tlb_f(&o, i), *w, "write {i}");
    }
    let ints = [0xfffe, 0x8001, 0xffff_fffe, 0xffff_8001];
    for (i, w) in ints.iter().enumerate() {
        assert_eq!(tlb(&o, 7 + i), *w, "add MOV unpack {i}");
        assert_eq!(tlb(&o, 11 + i), *w, "mul MOV unpack {i}");
    }
}

/// An output pack converts the result to f16 into one half and leaves the
/// other half of the destination as it was (`V3D_QPU_PACK_L/H`).
#[test]
fn output_packs() {
    let mut p = P::default();
    p.ld(10, f(1.0)).ld(11, f(0.5)).ld(12, 0xdead_beef).ld(13, 0xdead_beef);
    let mut a = add(AddOp71::Fadd, rfd(12), S::R(10), S::I(f(0.0)));
    a.add.pack = OutPack::L;
    p.push(a);
    let mut m = mul(MulOp71::Fmul, rfd(13), S::R(11), S::I(f(1.0)));
    m.mul.pack = OutPack::H;
    p.push(m);
    p.tlb(12).tlb(13);
    let o = p.run().expect("runs");
    assert_eq!(tlb(&o, 0), 0xdead_3c00);
    assert_eq!(tlb(&o, 1), 0x3800_beef);
}

// --------------------------------------------------------------------- SFU

fn sfu(op: AddOp71, x: f32) -> (f32, Vec<&'static str>) {
    let mut p = P::default();
    p.ld(10, f(x)).push(add(op, rfd(12), S::R(10), S::N)).tlb(12);
    let o = p.run().unwrap_or_else(|e| panic!("{op:?}({x}): {e}"));
    (tlb_f(&o, 0), o.approximate)
}

/// The SFU as ALU operations (7.1 has no magic SFU registers): the result is
/// in the register file for the next instruction, and the run lists the
/// operation as approximate (the hardware's precision is not modelled).
/// recip = 1/x, rsqrt = 1/sqrt(x), exp = 2^x, log = log2(x), sin takes its
/// argument in units of pi, reduced to [-0.5, 0.5] (`ntq_fsincos`).
#[test]
fn sfu_as_alu_ops() {
    use AddOp71::*;
    let near = |got: f32, want: f64, what: &str| {
        let rel = ((got as f64 - want) / want).abs();
        assert!(rel < 1e-5, "{what}: {got} vs {want} (relative {rel})");
    };
    for (op, x, want, name) in [
        (Recip, 4.0f32, 0.25f64, "recip"),
        (Recip, -8.0, -0.125, "recip"),
        (Rsqrt, 4.0, 0.5, "rsqrt"),
        (Rsqrt, 2.0, 1.0 / 2.0f64.sqrt(), "rsqrt"),
        (Exp, 3.0, 8.0, "exp"),
        (Exp, -1.0, 0.5, "exp"),
        (Log, 8.0, 3.0, "log"),
        (Log, 0.5, -1.0, "log"),
        (Sin, 0.25, (std::f64::consts::PI * 0.25).sin(), "sin"),
        (Sin, -0.5, -1.0, "sin"),
    ] {
        let (got, approx) = sfu(op, x);
        near(got, want, name);
        assert_eq!(approx, [name], "{op:?}");
    }
}

/// An SFU result is read by the very next instruction (Mesa's scheduler
/// makes it stall until it is there, `qpu_schedule.c:779-786`): a chain of
/// dependent SFU ops with nothing in between.
#[test]
fn sfu_result_is_there_for_the_next_instruction() {
    let mut p = P::default();
    p.ld(10, f(16.0));
    p.push(add(AddOp71::Rsqrt, rfd(11), S::R(10), S::N)); // 0.25
    p.push(add(AddOp71::Recip, rfd(12), S::R(11), S::N)); // 4.0
    p.push(add(AddOp71::Log, rfd(13), S::R(12), S::N)); // 2.0
    p.push(add(AddOp71::Exp, rfd(14), S::R(13), S::N)); // 4.0
    p.tlb(14);
    let o = p.run().expect("runs");
    assert!((tlb_f(&o, 0) - 4.0).abs() < 1e-4);
    assert_eq!(o.approximate, ["rsqrt", "recip", "log", "exp"]);
}

/// Mesa's own argument-range rule for SIN is the caller's: outside
/// [-0.5, 0.5] sim71 refuses rather than reduce.
#[test]
fn sfu_sin_outside_the_reduced_range_is_refused() {
    let mut p = P::default();
    p.ld(10, f(0.75)).push(add(AddOp71::Sin, rfd(12), S::R(10), S::N)).tlb(12);
    let e = p.run().unwrap_err();
    assert!(e.contains("outside the reduced range"), "{e}");
}

// -------------------------------------------------------------------- flags

/// A prelude: rf10 = lane number (EIDX), rf11 = 3, rf14 = 4.
fn lane_prelude(p: &mut P) {
    p.push(add(AddOp71::Eidx, rfd(10), S::N, S::N));
    p.ld(11, 3).ld(14, 4);
}

/// Per lane, whether the write `under` (an instruction writing 1 to rf12,
/// after rf12 was zeroed) happened.
fn mask_after(setup: impl FnOnce(&mut P), under: Alu71) -> [bool; LANES] {
    let mut p = P::default();
    lane_prelude(&mut p);
    setup(&mut p);
    p.push(add(AddOp71::Mov, rfd(12), S::I(0), S::N));
    p.push(under);
    p.tlb(12);
    let o = p.run().expect("runs");
    lanes(&o, 0).map(|v| v == 1)
}

fn lane_is(n: u32) -> impl Fn(usize) -> bool {
    move |l| l as u32 == n
}

fn eq_mask(want: impl Fn(usize) -> bool) -> [bool; LANES] {
    std::array::from_fn(want)
}

/// `xor nop, a, b .pushz`: Z is "the xor is 0" (`ieq`, `nir_to_vir.c`
/// `ntq_emit_comparison`), A takes it and B takes the old A; IFA, IFNA, IFB
/// and IFNB select the lanes of A and B, and only those lanes are written.
#[test]
fn flags_push_and_conditions_on_the_add_half() {
    let pushz = |a: u8, b: u8| with_flags(add(AddOp71::Xor, NOPD, S::R(a), S::R(b)), |f| f.apf = Pf::PushZ);
    let under = |c: Cond| with_flags(add(AddOp71::Mov, rfd(12), S::I(1), S::N), |f| f.ac = c);
    let setup = |p: &mut P| {
        p.push(pushz(10, 11)); // A = lane 3
        p.push(pushz(10, 14)); // A = lane 4, B = lane 3
    };
    assert_eq!(mask_after(setup, under(Cond::None)), [true; LANES]);
    assert_eq!(mask_after(setup, under(Cond::IfA)), eq_mask(lane_is(4)));
    assert_eq!(mask_after(setup, under(Cond::IfNA)), eq_mask(|l| l != 4));
    assert_eq!(mask_after(setup, under(Cond::IfB)), eq_mask(lane_is(3)));
    assert_eq!(mask_after(setup, under(Cond::IfNB)), eq_mask(|l| l != 3));
}

/// The mul half has its own condition (`mc`) and its own push (`mpf`).
#[test]
fn flags_on_the_mul_half() {
    let setup = |p: &mut P| {
        p.push(with_flags(mul(MulOp71::Mov, NOPD, S::R(10), S::N), |f| f.mpf = Pf::PushZ));
        // A = lane 0
    };
    let under = |c: Cond| with_flags(mul(MulOp71::Mov, rfd(12), S::I(1), S::N), |f| f.mc = c);
    assert_eq!(mask_after(setup, under(Cond::IfA)), eq_mask(lane_is(0)));
    assert_eq!(mask_after(setup, under(Cond::IfNA)), eq_mask(|l| l != 0));
    // The add half of the same instruction may use the flags too.
    let both_ifa = both(
        with_flags(add(AddOp71::Mov, rfd(13), S::R(15), S::N), |f| f.ac = Cond::IfA),
        mul(MulOp71::Mov, rfd(12), S::R(15), S::N),
    );
    let mut p = P::default();
    lane_prelude(&mut p);
    p.ld(15, 2);
    setup(&mut p);
    p.push(add(AddOp71::Mov, rfd(13), S::I(0), S::N));
    p.push(both_ifa);
    p.tlb(13);
    let o = p.run().expect("runs");
    assert_eq!(lanes(&o, 0).map(|v| v == 2), eq_mask(lane_is(0)));
}

/// The update forms: `andz` A = A & Z, `andnz` A & !Z, `norz` !A & !Z,
/// `nornz` !A & Z (`v3d_qpu_uf`), here with A = lane 3 and Z = lane 4.
#[test]
fn flags_update_forms() {
    let setup = |uf: Uf| {
        move |p: &mut P| {
            p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(10), S::R(11)), |f| f.apf = Pf::PushZ)); // A = lane 3
            p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(10), S::R(14)), |f| f.auf = uf));
            // Z = lane 4
        }
    };
    let ifa = with_flags(add(AddOp71::Mov, rfd(12), S::I(1), S::N), |f| f.ac = Cond::IfA);
    assert_eq!(mask_after(setup(Uf::AndZ), ifa), eq_mask(|_| false));
    assert_eq!(mask_after(setup(Uf::AndNZ), ifa), eq_mask(lane_is(3)));
    assert_eq!(mask_after(setup(Uf::NorZ), ifa), eq_mask(|l| l != 3 && l != 4));
    assert_eq!(mask_after(setup(Uf::NorNZ), ifa), eq_mask(lane_is(4)));
}

/// FMOV pushes Z (the value is 0.0) and N (it is below 0.0): the sign test
/// of `ntq_fsign`; on both ALUs.
#[test]
fn flags_from_fmov() {
    let mk = |pf: Pf, on_mul: bool| {
        move |p: &mut P| {
            p.push(add(AddOp71::Fxcd, rfd(15), S::N, S::N));
            p.ld(16, f(3.0));
            p.push(add(AddOp71::Fsub, rfd(17), S::R(15), S::R(16))); // lane - 3.0
            if on_mul {
                p.push(with_flags(mul(MulOp71::Fmov, NOPD, S::R(17), S::N), |f| f.mpf = pf));
            } else {
                p.push(with_flags(add(AddOp71::Fmov, NOPD, S::R(17), S::N), |f| f.apf = pf));
            }
        }
    };
    let ifa = with_flags(add(AddOp71::Mov, rfd(12), S::I(1), S::N), |f| f.ac = Cond::IfA);
    for on_mul in [false, true] {
        assert_eq!(mask_after(mk(Pf::PushZ, on_mul), ifa), eq_mask(lane_is(3)), "z, mul {on_mul}");
        assert_eq!(mask_after(mk(Pf::PushN, on_mul), ifa), eq_mask(|l| l < 3), "n, mul {on_mul}");
    }
}

/// The comparisons Mesa builds (`ntq_emit_comparison`): `feq` is FCMP.pushz,
/// `flt` FCMP.pushn, `fge(a, b)` FCMP(b, a).pushc, `ilt(a, b)` MIN(b, a).pushc
/// and `ult(a, b)` SUB(a, b).pushc, each with an IFA (here against 3).
#[test]
fn flags_from_comparisons() {
    let ifa = with_flags(add(AddOp71::Mov, rfd(12), S::I(1), S::N), |f| f.ac = Cond::IfA);
    let float_setup = |op: AddOp71, pf: Pf, swap: bool| {
        move |p: &mut P| {
            p.push(add(AddOp71::Fxcd, rfd(15), S::N, S::N));
            p.ld(16, f(3.0));
            let (a, b) = if swap { (16, 15) } else { (15, 16) };
            p.push(with_flags(add(op, NOPD, S::R(a), S::R(b)), |f| f.apf = pf));
        }
    };
    // lane == 3.0
    assert_eq!(mask_after(float_setup(AddOp71::Fcmp, Pf::PushZ, false), ifa), eq_mask(lane_is(3)));
    // lane < 3.0
    assert_eq!(mask_after(float_setup(AddOp71::Fcmp, Pf::PushN, false), ifa), eq_mask(|l| l < 3));
    // fge(lane, 3.0) = FCMP(3.0, lane).pushc = (3.0 <= lane)
    assert_eq!(mask_after(float_setup(AddOp71::Fcmp, Pf::PushC, true), ifa), eq_mask(|l| l >= 3));
    // ilt(lane, 3) = MIN(3, lane).pushc: C = (3 > lane)
    let int = |op: AddOp71, a: u8, b: u8| {
        move |p: &mut P| {
            p.push(with_flags(add(op, NOPD, S::R(a), S::R(b)), |f| f.apf = Pf::PushC));
        }
    };
    assert_eq!(mask_after(int(AddOp71::Min, 11, 10), ifa), eq_mask(|l| l < 3));
    // ult(lane, 3) = SUB(lane, 3).pushc: C = (lane < 3 unsigned)
    assert_eq!(mask_after(int(AddOp71::Sub, 10, 11), ifa), eq_mask(|l| l < 3));
    // ieq's MOV.pushz of a boolean: Z = (the value is 0)
    let mov = |p: &mut P| {
        p.push(with_flags(add(AddOp71::Mov, NOPD, S::R(10), S::N), |f| f.apf = Pf::PushZ));
    };
    assert_eq!(mask_after(mov, ifa), eq_mask(lane_is(0)));
}

/// `discard`: SETMSF under a condition clears the multisample flags of the
/// lanes (`nir_to_vir.c` `ntq_emit_intrinsic` for discard); the outcome keeps
/// the flags.
#[test]
fn discard_clears_msf_lanes() {
    let mut p = P::default();
    lane_prelude(&mut p);
    p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(10), S::R(11)), |f| f.apf = Pf::PushZ)); // A = lane 3
    p.push(with_flags(add(AddOp71::Setmsf, NOPD, S::I(0), S::N), |f| f.ac = Cond::IfA));
    let o = p.run().expect("runs");
    for l in 0..LANES {
        assert_eq!(o.msf[l] == 0, l == 3, "lane {l}");
    }
    // MSF reads them back; Z is "the flags are 0".
    let mut p = P::default();
    lane_prelude(&mut p);
    p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(10), S::R(11)), |f| f.apf = Pf::PushZ));
    p.push(with_flags(add(AddOp71::Setmsf, NOPD, S::I(0), S::N), |f| f.ac = Cond::IfA));
    p.push(with_flags(add(AddOp71::Msf, rfd(12), S::N, S::N), |f| f.apf = Pf::PushZ));
    p.push(add(AddOp71::Mov, rfd(13), S::I(0), S::N));
    p.push(with_flags(add(AddOp71::Mov, rfd(13), S::I(1), S::N), |f| f.ac = Cond::IfA));
    p.tlb(13);
    let o = p.run().expect("runs");
    assert_eq!(lanes(&o, 0).map(|v| v == 1), eq_mask(lane_is(3)));
}

// --------------------------------------------------------- ldvary, ldunif*

fn varying(vary: impl Fn(usize) -> f32, c: f32) -> Varying {
    Varying { vary: std::array::from_fn(|l| f(vary(l))), c: [f(c); LANES] }
}

/// LDVARY names its destination (rf10 here) and writes the C coefficient to
/// rf0 implicitly, one instruction after the destination (`qpu_schedule.c:
/// 715-726`, `v3d_qpu_writes_rf0_implicitly`): the old rf0 is still there
/// the instruction after the LDVARY, C the one after. Varyings come in order.
#[test]
fn ldvary_writes_rf0_implicitly_later() {
    let mut p = P::default();
    p.ld(0, f(7.0));
    p.push(ldvary(10)); // i1
    p.push(add(AddOp71::Fmov, rfd(12), S::R(0), S::N)); // i2: still 7.0
    p.push(add(AddOp71::Fmov, rfd(13), S::R(0), S::N)); // i3: C0
    p.push(add(AddOp71::Fmov, rfd(14), S::R(10), S::N)); // the destination
    p.push(ldvary(11));
    p.nops(1);
    p.push(add(AddOp71::Fadd, rfd(15), S::R(11), S::R(0))); // vary1 + C1
    p.tlb(12).tlb(13).tlb(14).tlb(15);
    let vs = vec![varying(|l| l as f32, 0.25), varying(|l| 10.0 + l as f32, 0.5)];
    let o = p.run_with(&fs_inputs(vs), &Env { threads: 1, ..Env::default() }).expect("runs");
    assert_eq!(tlb_f(&o, 0), 7.0);
    assert_eq!(tlb_f(&o, 1), 0.25);
    for l in 0..LANES {
        assert_eq!(f32::from_bits(lanes(&o, 2)[l]), l as f32);
        assert_eq!(f32::from_bits(lanes(&o, 3)[l]), 10.5 + l as f32);
    }
}

/// The interpolation Mesa emits at 7.1 for a varying (`emit_fragment_varying`
/// on 7.x): `ldvary` then `fmul r, vary, W` and `fadd r, r, C` with W in rf3.
#[test]
fn varying_interpolation_shape() {
    let mut p = P::default();
    p.push(ldvary(10));
    p.nops(1);
    p.push(mul(MulOp71::Fmul, rfd(11), S::R(10), S::R(3)));
    p.push(add(AddOp71::Fadd, rfd(12), S::R(11), S::R(0)));
    p.tlb(12);
    let o =
        p.run_with(&fs_inputs(vec![varying(|l| l as f32, 0.25)]), &Env { threads: 1, ..Env::default() }).expect("runs");
    for l in 0..LANES {
        assert_eq!(f32::from_bits(lanes(&o, 0)[l]), l as f32 * W + 0.25);
    }
}

/// LDUNIF without an address writes rf0 for the next instruction; LDUNIFRF
/// names the register; a TLBU write and a branch with the uniform bit take
/// the next stream word as well, in program order.
#[test]
fn ldunif_and_ldunifrf_take_the_stream_in_order() {
    let mut p = P::default();
    p.uni = vec![f(1.5), f(2.5), 0xffff_ff33];
    p.push(ldunif());
    p.push(add(AddOp71::Fmov, rfd(10), S::R(0), S::N));
    p.push(ldunifrf(11));
    p.nops(1);
    p.push(add(AddOp71::Fadd, rfd(12), S::R(10), S::R(11)));
    p.push(mul(MulOp71::Mov, TLBU, S::R(12), S::N)); // takes the third word
    let o = p.run().expect("runs");
    assert_eq!(o.uniforms_read, 3);
    assert_eq!(o.tlb.len(), 1);
    assert_eq!(o.tlb[0].config, Some(0xffff_ff33));
    assert_eq!(tlb_f(&o, 0), 4.0);
}

/// Two LDUNIFs in a row: each writes rf0 for the next instruction, so a read
/// in between sees the earlier one.
#[test]
fn back_to_back_ldunif() {
    let mut p = P::default();
    p.uni = vec![11, 22];
    p.push(ldunif());
    p.push(sig(add(AddOp71::Mov, rfd(10), S::R(0), S::N), |s| s.ldunif = true));
    p.push(add(AddOp71::Mov, rfd(11), S::R(0), S::N));
    p.tlb(10).tlb(11);
    let o = p.run().expect("runs");
    assert_eq!(tlb(&o, 0), 11);
    assert_eq!(tlb(&o, 1), 22);
}

/// UNIFA by LDUNIFRF into the magic address, LDUNIFA to rf0 and LDUNIFARF to
/// a register, each taking the word at the address and adding 4
/// (`ntq_emit_load_unifa`), no sooner than 4 instructions after the write
/// (`qpu_schedule.c:1437-1442`).
#[test]
fn ldunifa_and_ldunifarf_by_address() {
    let mut p = P::default();
    p.uni = vec![0x200];
    let mut set = ldunifrf(9);
    set.sig_magic = true;
    p.push(set);
    p.nops(3);
    p.push(sig(nop(), |s| s.ldunifa = true));
    p.push(add(AddOp71::Mov, rfd(10), S::R(0), S::N));
    let mut by_rf = sig(nop(), |s| s.ldunifarf = true);
    by_rf.sig_addr = 11;
    p.push(by_rf);
    p.nops(1);
    p.push(sig(nop(), |s| s.ldunifa = true));
    p.nops(1);
    p.tlb(10).tlb(11).tlb(0);
    let env = Env {
        threads: 1,
        memory: vec![Region { base: 0x200, words: vec![Some(11), Some(22), Some(33)] }],
        ..Env::default()
    };
    let o = p.run_with(&fs_inputs(Vec::new()), &env).expect("runs");
    assert_eq!((tlb(&o, 0), tlb(&o, 1), tlb(&o, 2)), (11, 22, 33));
}

/// ...and a read one instruction after the UNIFA write is refused.
#[test]
fn ldunifa_too_soon_is_refused() {
    let mut p = P::default();
    p.uni = vec![0x200];
    let mut set = ldunifrf(9);
    set.sig_magic = true;
    p.push(set);
    p.push(sig(nop(), |s| s.ldunifa = true));
    let env = Env { threads: 1, memory: vec![Region { base: 0x200, words: vec![Some(1)] }], ..Env::default() };
    let e = p.run_with(&fs_inputs(Vec::new()), &env).unwrap_err();
    assert!(e.contains("LDUNIFA"), "{e}");
}

// --------------------------------------------------------------------- TMU

/// A general memory write then read through the TMU, as Mesa's spills and
/// scratch use it (`vir_register_allocate.c`): TMUD gives the data, a write
/// to TMUA (the address) retires it; TMUWT waits; a second TMUA write with
/// no TMUD is a read whose word LDTMU takes, here per lane (address
/// 0x1000 + 4 * lane, data 100 + lane).
fn tmu_memory_program(p: &mut P) {
    p.push(add(AddOp71::Eidx, rfd(10), S::N, S::N));
    p.ld(13, 0x1000).ld(14, 100);
    p.push(add(AddOp71::Shl, rfd(11), S::R(10), S::I(2)));
    p.push(add(AddOp71::Add, rfd(12), S::R(10), S::R(14)));
    p.push(add(AddOp71::Add, rfd(11), S::R(11), S::R(13)));
    p.push(add(AddOp71::Mov, TMUD, S::R(12), S::N));
    p.push(add(AddOp71::Mov, TMUA, S::R(11), S::N));
    p.push(add(AddOp71::Tmuwt, NOPD, S::N, S::N));
    p.push(add(AddOp71::Mov, TMUA, S::R(11), S::N)); // the read
}

#[test]
fn ldtmu_returns_a_general_read_one_thread() {
    let mut p = P::default();
    tmu_memory_program(&mut p);
    p.push(ldtmu(15));
    p.tlb(15);
    // One thread: Mesa emits no switch, and the run counts the LDTMU as one
    // without a switch since its request.
    let o = p.run().expect("runs");
    for l in 0..LANES {
        assert_eq!(lanes(&o, 0)[l], 100 + l as u32);
    }
    assert_eq!((o.ldtmus, o.ldtmus_without_switch), (1, 1));
}

/// With two threads Mesa switches between the request and the LDTMU; so does
/// this program (a THRSW before the pair), and the word is the same.
#[test]
fn ldtmu_after_a_thread_switch() {
    let mut p = P::default();
    tmu_memory_program(&mut p);
    p.push(thrsw());
    p.nops(2);
    p.push(ldtmu(15));
    // The last-section pair, the same program frame as `framed`.
    p.push(thrsw()).push(thrsw()).nops(1);
    p.tlb(15);
    p.push(thrsw()).nops(2);
    let o = p.run_raw(&fs_inputs(Vec::new()), &Env { threads: 2, ..Env::default() }).expect("runs");
    for l in 0..LANES {
        assert_eq!(lanes(&o, 0)[l], 100 + l as u32);
    }
    assert_eq!((o.ldtmus, o.ldtmus_without_switch), (1, 0));
}

/// An LDTMU with nothing returned is refused.
#[test]
fn ldtmu_with_nothing_waiting_is_refused() {
    let mut p = P::default();
    p.push(ldtmu(15));
    let e = p.run().unwrap_err();
    assert!(e.contains("LDTMU with no TMU result waiting"), "{e}");
}

/// A regular 2D look-up (`v3d_vir_emit_tex`): two WRTMUC signals take P0 (the
/// four return words, texture 0) and P1 (32-bit output); T is written to
/// TMUT and S to TMUS retires the lookup, then four LDTMUs return r, g, b, a.
#[test]
fn texture_lookup_nearest() {
    // 2x2: texel (x, y) at y * 2 + x.
    let texels: Vec<[f32; 4]> = (0..4).map(|i| [i as f32 / 4.0, 0.5, 0.25, 1.0 - i as f32 / 4.0]).collect();
    let env = Env { threads: 1, textures: vec![Texture::flat(2, 2, texels.clone())], ..Env::default() };
    let mut p = P::default();
    // The stream: the two coordinates (LDUNIFRF), then P0 and P1 (WRTMUC).
    p.ld(10, f(0.75)).ld(11, f(0.25));
    p.uni.extend([0xf, 1]);
    p.push(sig(nop(), |s| s.wrtmuc = true));
    p.push(sig(add(AddOp71::Mov, TMUT, S::R(11), S::N), |s| s.wrtmuc = true));
    p.push(add(AddOp71::Mov, TMUS, S::R(10), S::N));
    for r in 20..24 {
        p.push(ldtmu(r));
    }
    p.nops(1);
    for r in 20..24 {
        p.tlb(r);
    }
    let o = p.run_with(&fs_inputs(Vec::new()), &env).expect("runs");
    assert_eq!(o.tmu_lookups, 1);
    // S = 0.75, T = 0.25 is the texel at x = 1, y = 0.
    for c in 0..4 {
        assert_eq!(tlb_f(&o, c), texels[1][c], "channel {c}");
    }
}

// ---------------------------------------------------------------- THRSW

/// A THRSW has two delay slots which run, and the thread end is the third
/// instruction after its THRSW (the last-section pair, then one more).
#[test]
fn thrsw_delay_slots_run_and_the_end_is_three_instructions_later() {
    let mut p = P::default();
    p.ld(10, 111).ld(11, 222);
    p.push(thrsw()).push(thrsw());
    p.nops(1);
    p.push(thrsw());
    p.tlb(10); // first delay slot of the thread end
    p.tlb(11); // second
               // An instruction after the end never runs.
    p.tlb(10);
    let o = p.run_raw(&fs_inputs(Vec::new()), &Env { threads: 1, ..Env::default() }).expect("runs");
    assert_eq!(o.tlb.len(), 2);
    assert_eq!((tlb(&o, 0), tlb(&o, 1)), (111, 222));
    assert_eq!(o.instructions, p.code.len() - 1);
}

/// A switch in the middle: the delay slots run in the old thread, and the
/// register file is as they left it afterwards.
#[test]
fn thrsw_switch_keeps_the_register_file() {
    let mut p = P::default();
    p.ld(10, f(5.0));
    p.push(thrsw());
    p.push(add(AddOp71::Fadd, rfd(11), S::R(10), S::R(10))); // delay slot 1
    p.push(mul(MulOp71::Fmul, rfd(12), S::R(10), S::I(f(4.0)))); // delay slot 2
    p.nops(1);
    p.tlb(11).tlb(12);
    p.push(thrsw()).push(thrsw()).nops(1);
    p.tlb(11);
    p.push(thrsw()).nops(2);
    let o = p.run_raw(&fs_inputs(Vec::new()), &Env { threads: 1, ..Env::default() }).expect("runs");
    assert_eq!((tlb_f(&o, 0), tlb_f(&o, 1), tlb_f(&o, 2)), (10.0, 20.0, 10.0));
}

/// 7.1 has no GFXH-1625: a TMUWT in the thread end's last delay slot is
/// legal (`qpu_validate.c`'s rule is `devinfo->ver < 71` there; the 4.2
/// interpreter refuses it).
#[test]
fn tmuwt_in_the_last_delay_slot_is_fine_at_7_1() {
    let mut p = P::default();
    p.ld(10, 0x40).ld(11, 9);
    p.push(add(AddOp71::Mov, TMUD, S::R(11), S::N));
    p.push(add(AddOp71::Mov, TMUA, S::R(10), S::N));
    p.push(thrsw()).push(thrsw()).nops(1);
    p.push(thrsw()).nops(1);
    p.push(add(AddOp71::Tmuwt, NOPD, S::N, S::N));
    let o = p.run_raw(&fs_inputs(Vec::new()), &Env { threads: 1, ..Env::default() }).expect("runs");
    assert_eq!(o.instructions, p.code.len());
}

/// An unwaited general TMU write at the thread end is refused.
#[test]
fn an_unwaited_tmu_write_at_the_end_is_refused() {
    let mut p = P::default();
    p.ld(10, 0x40).ld(11, 9);
    p.push(add(AddOp71::Mov, TMUD, S::R(11), S::N));
    p.push(add(AddOp71::Mov, TMUA, S::R(10), S::N));
    let e = p.run().unwrap_err();
    assert!(e.contains("not waited for by a TMUWT"), "{e}");
}

// ------------------------------------------------------------------ branches

fn branch(cond: BranchCond, skip: i32) -> Instr71 {
    Instr71::Branch(Branch {
        cond,
        msfign: Msfign::None,
        bdi: BranchDest::Rel,
        ub: false,
        bdu: BranchDest::Abs,
        raddr_a: 0,
        offset: (skip * 8) as u32,
    })
}

/// How the lanes' A flag is set for a branch: all true, none, or lane 3.
#[derive(Clone, Copy, Debug)]
enum Flag {
    All,
    None,
    Lane3,
}

/// Run `branch cond` over the flag and report whether it was taken: the
/// instruction after the three delay slots is skipped when taken; the
/// delay slot always runs.
fn taken(cond: BranchCond, flag: Flag) -> bool {
    let mut p = P::default();
    lane_prelude(&mut p);
    p.ld(20, 111).ld(21, 222).ld(22, 333).ld(23, 99);
    let (a, b) = match flag {
        Flag::All => (10, 10),  // x ^ x == 0 in every lane
        Flag::None => (10, 23), // lane != 99
        Flag::Lane3 => (10, 11),
    };
    p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(a), S::R(b)), |f| f.apf = Pf::PushZ));
    p.code.push(branch(cond, 1));
    p.tlb(20); // delay slot 1: always runs
    p.nops(2);
    p.tlb(21); // skipped when taken
    p.tlb(22); // the target
    let o = p.run().unwrap_or_else(|e| panic!("{cond:?} {flag:?}: {e}"));
    assert_eq!(tlb(&o, 0), 111, "the delay slot runs");
    let last = o.tlb.len() - 1;
    assert_eq!(tlb(&o, last), 333);
    assert!(o.tlb.len() == 2 || o.tlb.len() == 3, "{:?}", o.tlb.len());
    o.tlb.len() == 2
}

/// ALWAYS, ALLA, ANYA, ALLNA and ANYNA over the lanes' A flags
/// (`nir_to_vir.c:4332-4380`).
#[test]
fn branch_conditions_over_lanes() {
    use BranchCond::*;
    use Flag::*;
    for (cond, flag, want) in [
        (Always, All, true),
        (Always, None, true),
        (AllA, All, true),
        (AllA, Lane3, false),
        (AllA, None, false),
        (AnyA, Lane3, true),
        (AnyA, All, true),
        (AnyA, None, false),
        (AllNA, None, true),
        (AllNA, Lane3, false),
        (AllNA, All, false),
        (AnyNA, Lane3, true),
        (AnyNA, None, true),
        (AnyNA, All, false),
    ] {
        assert_eq!(taken(cond, flag), want, "{cond:?} over {flag:?}");
    }
}

/// `msfign P` ignores the lanes whose multisample flag is 0 (a discarded
/// lane, `nir_to_vir.c:4332-4380`): with lane 3 discarded and A set only in
/// lane 3, ALLNA is taken with P and not without it.
#[test]
fn branch_msfign_p_ignores_discarded_lanes() {
    let run = |msfign: Msfign| {
        let mut p = P::default();
        lane_prelude(&mut p);
        p.ld(20, 111).ld(21, 222).ld(22, 333);
        let a3 = with_flags(add(AddOp71::Xor, NOPD, S::R(10), S::R(11)), |f| f.apf = Pf::PushZ);
        p.push(a3);
        p.push(with_flags(add(AddOp71::Setmsf, NOPD, S::I(0), S::N), |f| f.ac = Cond::IfA));
        p.push(a3);
        p.code.push(Instr71::Branch(Branch {
            cond: BranchCond::AllNA,
            msfign,
            bdi: BranchDest::Rel,
            ub: false,
            bdu: BranchDest::Abs,
            raddr_a: 0,
            offset: 8,
        }));
        p.nops(3);
        p.tlb(21); // skipped when taken
        p.tlb(22);
        p.run().expect("runs").tlb.len() == 1
    };
    assert!(run(Msfign::P));
    assert!(!run(Msfign::None));
}

/// What a back end must not emit, by name: the branch conditions on lane 0
/// (A0, NA0), `msfign Q`, and absolute, link-register and register-file
/// branch destinations.
#[test]
fn unmodelled_branch_forms_are_refused() {
    let one = |cond: BranchCond, msfign: Msfign, bdi: BranchDest, raddr_a: u8| {
        let mut p = P::default();
        lane_prelude(&mut p);
        p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(10), S::R(10)), |f| f.apf = Pf::PushZ));
        p.code.push(Instr71::Branch(Branch {
            cond,
            msfign,
            bdi,
            ub: false,
            bdu: BranchDest::Abs,
            raddr_a,
            offset: if matches!(bdi, BranchDest::Rel | BranchDest::Abs) { 8 } else { 0 },
        }));
        p.nops(5);
        p.run()
    };
    let rel = BranchDest::Rel;
    assert!(one(BranchCond::A0, Msfign::None, rel, 0).unwrap_err().contains("branch condition A0"));
    assert!(one(BranchCond::NA0, Msfign::None, rel, 0).unwrap_err().contains("branch condition NA0"));
    assert!(one(BranchCond::Always, Msfign::Q, rel, 0).unwrap_err().contains("msfign Q"));
    for bdi in [BranchDest::Abs, BranchDest::LinkReg, BranchDest::Regfile] {
        let raddr = if bdi == BranchDest::Regfile { 10 } else { 0 };
        let e = one(BranchCond::Always, Msfign::None, bdi, raddr).unwrap_err();
        assert!(e.contains("branch destination"), "{bdi:?}: {e}");
    }
}

/// A backward branch: a counted loop (rf6 counts to 3, rf9 counts the delay
/// slot's runs), the loop shape Mesa emits.
#[test]
fn counted_loop() {
    let mut p = P::default();
    p.ld(8, 3); // 0
    p.push(add(AddOp71::Mov, rfd(6), S::I(0), S::N)); // 1
    p.push(add(AddOp71::Mov, rfd(9), S::I(0), S::N)); // 2
    p.push(add(AddOp71::Add, rfd(6), S::R(6), S::I(1))); // 3: L
    p.push(with_flags(add(AddOp71::Xor, NOPD, S::R(6), S::R(8)), |f| f.apf = Pf::PushZ)); // 4
    p.code.push(branch(BranchCond::AllNA, 3 - 5 - 4)); // 5: back to 3
    p.push(add(AddOp71::Add, rfd(9), S::R(9), S::I(1))); // 6: delay slot
    p.nops(2);
    p.tlb(6).tlb(9);
    let o = p.run().expect("runs");
    assert_eq!((tlb(&o, 0), tlb(&o, 1)), (3, 3));
}

/// A branch that also branches the uniform stream (`ub`, `bdu`): the stream
/// moves to the branch's own uniform + 1 + value / 4, so the loads on the
/// skipped path do not consume theirs.
#[test]
fn branch_with_the_uniform_stream() {
    let mut p = P::default();
    p.uni = vec![4, 0xbad, 0x600d];
    p.code.push(Instr71::Branch(Branch {
        cond: BranchCond::Always,
        msfign: Msfign::None,
        bdi: BranchDest::Rel,
        ub: true,
        bdu: BranchDest::Rel,
        raddr_a: 0,
        offset: 8,
    }));
    p.nops(3);
    p.push(ldunifrf(10)); // skipped, with its uniform
    p.push(ldunifrf(10)); // the target reads the third word
    p.nops(1);
    p.tlb(10);
    let o = p.run().expect("runs");
    assert_eq!(tlb(&o, 0), 0x600d);
}

// ---------------------------------------------------------------------- TLB

/// The colour write of a 16F render target: `vfpack(r, g)` and `vfpack(b, a)`
/// then `mov tlb` twice (`vir_emit_tlb_color_write`); the run records each
/// write in order.
#[test]
fn tlb_colour_writes_in_order() {
    let mut p = P::default();
    p.ld(10, f(1.0)).ld(11, f(0.5)).ld(12, f(0.0)).ld(13, f(2.0));
    p.push(add(AddOp71::Vfpack, rfd(14), S::R(10), S::R(11)));
    p.push(add(AddOp71::Vfpack, rfd(15), S::R(12), S::R(13)));
    p.tlb(14).tlb(15);
    let o = p.run().expect("runs");
    assert_eq!(o.tlb.len(), 2);
    assert!(o.tlb.iter().all(|w| w.config.is_none()));
    assert_eq!(tlb(&o, 0), 0x3800_3c00);
    assert_eq!(tlb(&o, 1), 0x4000_0000);
}

/// A TLB write on the add half (`mov tlb` is an add op at 7.1 too), and TLBU
/// taking its configuration from the stream.
#[test]
fn tlb_writes_on_either_half_and_tlbu_configuration() {
    let mut p = P::default();
    p.uni = vec![0xffff_ff11, 0xffff_ff22];
    p.push(add(AddOp71::Mov, TLB, S::R(3), S::N));
    p.push(add(AddOp71::Mov, TLBU, S::R(2), S::N));
    p.push(mul(MulOp71::Mov, TLBU, S::R(3), S::N));
    let o = p.run().expect("runs");
    assert_eq!(o.tlb.len(), 3);
    assert_eq!(o.tlb[0].config, None);
    assert_eq!(tlb_f(&o, 0), W);
    assert_eq!(o.tlb[1].config, Some(0xffff_ff11));
    assert_eq!(tlb_f(&o, 1), Z);
    assert_eq!(o.tlb[2].config, Some(0xffff_ff22));
    assert_eq!(tlb_f(&o, 2), W);
}

/// The payload: W in rf3, centroid W in rf1 and Z in rf2 at entry; rf0 holds
/// nothing (it is where LDUNIF, LDUNIFA and LDVARY's C land).
#[test]
fn fragment_payload_registers() {
    let mut p = P::default();
    p.tlb(3).tlb(1).tlb(2);
    let o = p.run().expect("runs");
    assert_eq!((tlb_f(&o, 0), tlb_f(&o, 1), tlb_f(&o, 2)), (W, W, Z));
}

// ------------------------------------------------------- refusals, inventory

/// What the interpreter refuses by name, for the back end's authors:
/// accumulator and QUAD/REP writes, an ALU register-file write in the thread
/// end's THRSW, an LDVARY in a THRSW's second delay slot, and two
/// restricted peripherals in one instruction.
#[test]
fn refusals_by_name() {
    let mut p = P::default();
    p.push(add(AddOp71::Fadd, (WADDR_QUAD, true), S::R(3), S::R(2)));
    let e = p.run().unwrap_err();
    assert!(e.contains("magic register quad"), "{e}");

    let end = |last: Alu71| {
        let mut p = P::default();
        p.push(thrsw()).push(thrsw()).nops(1);
        p.push(last).nops(2);
        p.run_raw(&fs_inputs(Vec::new()), &Env { threads: 1, ..Env::default() })
    };
    let e = end(sig(add(AddOp71::Fadd, rfd(10), S::R(3), S::R(2)), |s| s.thrsw = true)).unwrap_err();
    assert!(e.contains("an ALU register-file write at the thread end"), "{e}");

    let mut p = P::default();
    p.push(thrsw()).push(thrsw()).push(thrsw()).nops(1);
    p.push(ldvary(10));
    let e = p.run_raw(&fs_inputs(vec![varying(|_| 0.0, 0.0)]), &Env::default()).unwrap_err();
    assert!(e.contains("an LDVARY in a THRSW's second delay slot"), "{e}");

    let mut p = P::default();
    p.uni = vec![0xffff_ff11];
    p.push(both(add(AddOp71::Mov, TMUT, S::R(3), S::N), mul(MulOp71::Mov, TMUS, S::R(2), S::N)));
    let e = p.run().unwrap_err();
    assert!(e.contains("peripherals") || e.contains("TMU"), "{e}");
}

/// Run a probe program: an instruction the codec cannot encode is reported as
/// such (not a refusal of the interpreter).
fn probe(p: &P) -> Result<(), String> {
    let mut words = Vec::new();
    for i in &p.framed().code {
        match encode71(i) {
            Ok(w) => words.push(w),
            Err(e) => return Err(format!("not encodable in this probe ({e:?})")),
        }
    }
    run71(&words, &p.uni, &fs_inputs(Vec::new()), &Env { threads: 1, ..Env::default() }).map(|_| ())
}

/// Every operation of both ALUs, run once with defined operands in a
/// fragment shader: which ones sim71 computes and which it refuses. Printed
/// (`--nocapture`); pinned for the operations a GLSL ES 1.00 back end emits.
#[test]
fn unmodelled_inventory() {
    let try_add = |op: AddOp71| -> Result<(), String> {
        let mut p = P::default();
        p.ld(10, f(0.25)).ld(11, f(0.5));
        let d = if add_op_args(op) & ARG_D == 0 || op == AddOp71::Fcmp { NOPD } else { rfd(12) };
        p.push(add(op, d, S::R(10), S::R(11)));
        p.tlb(3);
        probe(&p)
    };
    let try_mul = |op: MulOp71| -> Result<(), String> {
        let mut p = P::default();
        p.ld(10, f(0.25)).ld(11, f(0.5));
        let d = if mul_op_args(op) & ARG_D == 0 { NOPD } else { rfd(12) };
        p.push(mul(op, d, S::R(10), S::R(11)));
        p.tlb(3);
        probe(&p)
    };
    let mut refused = Vec::new();
    for op in AddOp71::ALL {
        if let Err(e) = try_add(op) {
            refused.push(format!("add {op:?}: {e}"));
        }
    }
    for op in MulOp71::ALL {
        if let Err(e) = try_mul(op) {
            refused.push(format!("mul {op:?}: {e}"));
        }
    }
    println!("sim71 refuses {} of {} operations:", refused.len(), AddOp71::ALL.len() + MulOp71::ALL.len());
    for r in &refused {
        println!("  {r}");
    }
    // The back end's emission set must not be among them.
    let needed_add = [
        "Fadd", "Fsub", "Fmin", "Fmax", "Ffloor", "Fceil", "Ftrunc", "Fround", "Ftoin", "Ftoiz", "Ftouz", "Itof",
        "Utof", "Add", "Sub", "And", "Or", "Xor", "Min", "Max", "Umin", "Umax", "Shl", "Shr", "Asr", "Ror", "Not",
        "Neg", "Clz", "Fmov", "Mov", "Vfpack", "Eidx", "Fxcd", "Fycd", "Msf", "Setmsf", "Tmuwt", "Tidx", "Revf",
        "Recip", "Rsqrt", "Exp", "Log",
    ];
    let needed_mul = ["Add", "Sub", "Umul24", "Smul24", "Fmul", "Fmov", "Mov", "Nop"];
    for r in &refused {
        for n in needed_add {
            assert!(!r.starts_with(&format!("add {n}:")), "needed and refused: {r}");
        }
        for n in needed_mul {
            assert!(!r.starts_with(&format!("mul {n}:")), "needed and refused: {r}");
        }
    }
}
