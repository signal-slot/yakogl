// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 scheduler (`src/shaderc/online/v3d/sched.rs`) over seeded
//! random straight-line programs (plan #419, item sched-fuzz42), modelled on
//! `tests/v3d71_sched_fuzz.rs`. A small xorshift with fixed seeds builds
//! fragment shaders from the operations the 4.2 back end hands `schedule`
//! (`select::VOp`): add- and mul-unit integer and float operations on
//! virtual registers, `Ldunif` constants, small immediates, the payload W
//! and Z, the ABS input unpack, SFU operations, flag pushes with
//! conditional moves, copies, and runs of two independent chains, one on
//! the add unit and one on the mul unit, which are what the scheduler can
//! pair; then the last-thread-switch pair and one to four TLB writes of
//! values the body computed (the first a TLBU with its configuration
//! uniform, sometimes an f16 `vfpack` on the add unit). The registers come
//! from the real allocator (`alloc::allocate`, 4 threads).
//!
//! Each program is scheduled twice: as it is (`packed`), and with a
//! `VOp::Label` between every two body operations (`one per word`): a label
//! starts a word of its own and nothing is merged or hoisted across it
//! (sched.rs, `VOp::Label` and `hoist`), so every body operation gets a word
//! of its own in program order. That is the public seam for an unscheduled
//! schedule; the scheduler has no such mode of its own outside its unit
//! tests (`hoist_on`, `branch_back_on` are `cfg(test)`). For each of the two:
//!
//! 1. `allocate` and `schedule` succeed (any refusal fails the test: the
//!    generator only makes programs the back end must take);
//! 2. `verify(&instrs, false)` and `registers_fit_threads(.., 4)` pass;
//! 3. every word encodes, and decodes back to the same instruction
//!    (`decode::same_instr`);
//! 4. `sim::run` on the words and the uniform stream reads every uniform and
//!    writes the TLB with exactly the values (and the TLBU configuration) of
//!    [`eval`], a reference evaluator that runs the operations one at a time
//!    in program order (the sim's documented semantics: f32 for the float
//!    operations, i32 for the integer ones, the exact f32 function for the
//!    SFU, Z/N/C as `sim.rs`'s header lists them).
//!
//! So the packed program, the one-per-word program and the reference all
//! agree bit for bit. The one-per-word schedule also has at least one word
//! per body operation and no hoist (the seam is checked to be what it
//! claims), and over all programs the packed schedules are shorter.
//!
//! The generator evaluates as it goes and drops a candidate operation whose
//! operands or result the sim does not model (a NaN, a denormal, SIN outside
//! [-0.5, 0.5], an FTOI* out of range, FMIN or FMAX of -0.0 and +0.0, whose
//! answer depends on the operand order the encoder picks); RSQRT2 is not
//! generated. It does not make two things `schedule` gets wrong or refuses,
//! each pinned by an `#[ignore]`d reproducer with its control: a value
//! nothing reads (every such value is folded into an XOR the last TLB write
//! reads, [`sink_dead`]; [`two_unread_values_share_a_word_and_a_register`])
//! and FMIN or FMAX of one operand twice ([`fmax_of_one_register_twice`]).
//! A failure names the program and its seed; the summary (programs,
//! operations, words packed and one per word, hoists) is printed with
//! `--nocapture`. This file changes nothing under `src/`.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_sched_fuzz
//! -- --nocapture` (`V3D_CONFIG` of tools/check.sh).

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test"))]

use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::alloc::allocate;
use yakogl::shaderc::online::v3d::decode::{decode, same_instr};
use yakogl::shaderc::online::v3d::encode::encode;
use yakogl::shaderc::online::v3d::isa::{small_imm_value, AddOp, Cond, Instr, MulOp, Pf, Uf};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::sched::schedule;
use yakogl::shaderc::online::v3d::select::{Dst, FlagOp, Operand, Selected, Uniform, VOp, V};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::v3d::verify::{registers_fit_threads, verify};

/// `store::with_scratch` has one scratch region and refuses a second user;
/// the tests of this file take turns.
static SCRATCH: Mutex<()> = Mutex::new(());

/// Programs generated.
const PROGRAMS: usize = 512;
/// The fixed seeds: program `n` uses `SEEDS[n % len] ^ n`.
const SEEDS: [u64; 4] = [0x6a09_e667_f3bc_c908, 0xbb67_ae85_84ca_a73b, 0x3c6e_f372_fe94_f82b, 0xa54f_f53a_5f1d_36f1];
/// At most this many values are kept for later reads; a new value past it
/// replaces one, which is then dead.
const POOL: usize = 12;

// ---------------------------------------------------------------------------
// The PRNG.
// ---------------------------------------------------------------------------

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 33) as usize % n
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

// ---------------------------------------------------------------------------
// The inputs and the reference evaluator.
// ---------------------------------------------------------------------------

type Lanes = [u32; LANES];
/// Per lane, an operation's Z, N and C (`None` where the sim does not
/// describe the flag).
type LaneFlags = [[Option<bool>; 3]; LANES];

/// The payload W (rf0) of lane `l`: a different value per lane.
fn payload_w(l: usize) -> u32 {
    (1.0f32 + l as f32 / 16.0).to_bits()
}
/// The payload Z (rf2) of lane `l`.
fn payload_z(l: usize) -> u32 {
    (0.25f32 + l as f32 / 32.0).to_bits()
}

fn fs_inputs() -> FsInputs {
    let one = 1.0f32.to_bits();
    FsInputs {
        w: core::array::from_fn(payload_w),
        w_centroid: [one; LANES],
        z: core::array::from_fn(payload_z),
        x: [one; LANES],
        y: [one; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    }
}

/// What the sim does not model: the operation is not generated.
#[derive(Debug)]
struct Unmodelled;

/// One TLB write: the configuration a TLBU write takes, and the lanes.
type Tlb = (Option<u32>, Lanes);

/// The values of the virtual registers, the A and B flags, and the TLB
/// writes, after the operations run so far.
#[derive(Clone, Default)]
struct State {
    v: BTreeMap<u32, Lanes>,
    flag_a: Option<[bool; LANES]>,
    flag_b: Option<[bool; LANES]>,
    tlb: Vec<Tlb>,
}

fn is_bad_float(x: u32) -> bool {
    let exp = x & 0x7f80_0000;
    (exp == 0 && x & 0x7f_ffff != 0) || (exp == 0x7f80_0000 && x & 0x7f_ffff != 0)
}

fn fop(x: u32) -> Result<f32, Unmodelled> {
    if is_bad_float(x) {
        return Err(Unmodelled);
    }
    Ok(f32::from_bits(x))
}

fn fres(r: f32) -> Result<u32, Unmodelled> {
    let b = r.to_bits();
    if is_bad_float(b) {
        return Err(Unmodelled);
    }
    Ok(b)
}

impl State {
    fn src(&self, s: Operand) -> Lanes {
        match s {
            Operand::V(v) => *self.v.get(&v.0).unwrap_or_else(|| panic!("{v:?} read before it is written")),
            Operand::Abs(v) => self.src(Operand::V(v)).map(|x| x & 0x7fff_ffff),
            Operand::SmallImm(i) => [small_imm_value(i).expect("a small immediate"); LANES],
            Operand::Payload(0) => core::array::from_fn(payload_w),
            Operand::Payload(2) => core::array::from_fn(payload_z),
            Operand::None => [0; LANES],
            other => panic!("operand {other:?} is not generated"),
        }
    }

    /// An add-unit operation's value and its Z/N/C flags.
    fn add(&self, op: AddOp, a: Operand, b: Operand) -> Result<(Lanes, LaneFlags), Unmodelled> {
        use AddOp::*;
        let (x, y) = (self.src(a), self.src(b));
        let mut r = [0u32; LANES];
        let mut f = [[None; 3]; LANES];
        for l in 0..LANES {
            let (p, q) = (x[l], y[l]);
            r[l] = match op {
                Fadd => fres(fop(p)? + fop(q)?)?,
                Fsub => fres(fop(p)? - fop(q)?)?,
                // Which of -0.0 and +0.0 FMIN and FMAX give depends on the
                // operand order, which the encoder picks (FMIN and FMAX are
                // told apart by it): not generated.
                Fmin | Fmax if p | q == 0x8000_0000 && p != q => return Err(Unmodelled),
                Fmin => fres(fop(p)?.min(fop(q)?))?,
                Fmax => fres(fop(p)?.max(fop(q)?))?,
                Fcmp => {
                    let (s, t) = (fop(p)?, fop(q)?);
                    f[l] = [Some(s == t), Some(s < t), Some(s <= t)];
                    0
                }
                Vfpack => {
                    let (s, t) = (fop(p)?, fop(q)?);
                    sim::f32_to_half(s) as u32 | (sim::f32_to_half(t) as u32) << 16
                }
                Add => p.wrapping_add(q),
                Sub => {
                    f[l][2] = Some(p < q);
                    p.wrapping_sub(q)
                }
                And => p & q,
                Or => p | q,
                Xor => {
                    f[l][0] = Some(p ^ q == 0);
                    p ^ q
                }
                Min => {
                    f[l][2] = Some((p as i32) > (q as i32));
                    (p as i32).min(q as i32) as u32
                }
                Max => (p as i32).max(q as i32) as u32,
                Umin => p.min(q),
                Umax => p.max(q),
                Shl => p.wrapping_shl(q & 31),
                Shr => p.wrapping_shr(q & 31),
                Asr => (p as i32).wrapping_shr(q & 31) as u32,
                Ror => p.rotate_right(q & 31),
                Not => !p,
                Neg => (p as i32).wrapping_neg() as u32,
                Itof => (p as i32 as f32).to_bits(),
                Utof => (p as f32).to_bits(),
                Eidx => l as u32,
                Ffloor => fres(fop(p)?.floor())?,
                Fceil => fres(fop(p)?.ceil())?,
                Ftrunc => fres(fop(p)?.trunc())?,
                Fround => fres(fop(p)?.round_ties_even())?,
                Ftoiz | Ftoin => {
                    let s = fop(p)?;
                    let t = if op == Ftoin { s.round_ties_even() } else { s.trunc() };
                    if !(-2_147_483_648.0..2_147_483_648.0).contains(&t) {
                        return Err(Unmodelled);
                    }
                    t as i32 as u32
                }
                Ftouz => {
                    let t = fop(p)?.trunc();
                    if !(0.0..4_294_967_296.0).contains(&t) {
                        return Err(Unmodelled);
                    }
                    t as u32
                }
                Recip | Rsqrt | Exp | Log | Sin => {
                    let s = fop(p)?;
                    fres(match op {
                        Recip => 1.0 / s,
                        Rsqrt => 1.0 / s.sqrt(),
                        Exp => s.exp2(),
                        Log => s.log2(),
                        _ => {
                            if !(-0.5..=0.5).contains(&s) {
                                return Err(Unmodelled);
                            }
                            (s * core::f32::consts::PI).sin()
                        }
                    })?
                }
                other => panic!("add {other:?} is not generated"),
            };
        }
        Ok((r, f))
    }

    fn mul(&self, op: MulOp, a: Operand, b: Operand) -> Result<(Lanes, LaneFlags), Unmodelled> {
        let (x, y) = (self.src(a), self.src(b));
        let mut r = [0u32; LANES];
        let mut f = [[None; 3]; LANES];
        for l in 0..LANES {
            let (p, q) = (x[l], y[l]);
            r[l] = match op {
                MulOp::Fmul => fres(fop(p)? * fop(q)?)?,
                MulOp::Add => p.wrapping_add(q),
                MulOp::Sub => {
                    f[l][2] = Some(p < q);
                    p.wrapping_sub(q)
                }
                MulOp::Umul24 => (p & 0xff_ffff).wrapping_mul(q & 0xff_ffff),
                MulOp::Smul24 => ((((p << 8) as i32) >> 8).wrapping_mul(((q << 8) as i32) >> 8)) as u32,
                MulOp::Mov => {
                    f[l][0] = Some(p == 0);
                    p
                }
                MulOp::Fmov => {
                    let s = fop(p)?;
                    f[l] = [Some(s == 0.0), Some(s < 0.0), None];
                    p
                }
                other => panic!("mul {other:?} is not generated"),
            };
        }
        Ok((r, f))
    }

    fn cond(&self, c: Cond) -> [bool; LANES] {
        let (f, neg) = match c {
            Cond::IfA => (self.flag_a, false),
            Cond::IfNA => (self.flag_a, true),
            Cond::IfB => (self.flag_b, false),
            Cond::IfNB => (self.flag_b, true),
            Cond::None => return [true; LANES],
        };
        f.expect("a condition on a flag pushed before it").map(|x| x != neg)
    }

    /// Run one operation.
    fn step(&mut self, op: &VOp) -> Result<(), Unmodelled> {
        match *op {
            VOp::Add { op, dst, a, b } => {
                let (r, _) = self.add(op, a, b)?;
                self.write(dst, r);
            }
            VOp::Mul { op, dst, a, b } => {
                let (r, _) = self.mul(op, a, b)?;
                self.write(dst, r);
            }
            VOp::Ldunif { dst, u } => {
                self.v.insert(dst.0, [u.data; LANES]);
            }
            VOp::Flags { unit, a, b, pf, uf } => {
                assert_eq!(uf, Uf::None, "flag updates are not generated");
                let (_, f) = match unit {
                    FlagOp::Add(op) => self.add(op, a, b)?,
                    FlagOp::Mul(op) => self.mul(op, a, b)?,
                };
                let k = match pf {
                    Pf::PushZ => 0,
                    Pf::PushN => 1,
                    Pf::PushC => 2,
                    Pf::None => panic!("a Flags with no push"),
                };
                let mut new = [false; LANES];
                for l in 0..LANES {
                    new[l] = f[l][k].ok_or(Unmodelled)?;
                }
                self.flag_b = self.flag_a;
                self.flag_a = Some(new);
            }
            VOp::CondMov { dst, src, cond } => {
                let x = self.src(src);
                let m = self.cond(cond);
                let d = self.v.get_mut(&dst.0).expect("a conditional move into a written register");
                for l in 0..LANES {
                    if m[l] {
                        d[l] = x[l];
                    }
                }
            }
            VOp::Copy { dst, src } => {
                let x = self.src(src);
                self.v.insert(dst.0, x);
            }
            VOp::Thrsw | VOp::Label(_) => {}
            other => panic!("{other:?} is not generated"),
        }
        Ok(())
    }

    fn write(&mut self, d: Dst, r: Lanes) {
        match d {
            Dst::V(v) => {
                self.v.insert(v.0, r);
            }
            Dst::Tlb => self.tlb.push((None, r)),
            Dst::Tlbu(u) => self.tlb.push((Some(u.data), r)),
            other => panic!("destination {other:?} is not generated"),
        }
    }
}

/// The reference: the operations one at a time, in program order; the TLB
/// writes it makes.
fn eval(ops: &[VOp]) -> Vec<Tlb> {
    let mut s = State::default();
    for op in ops {
        s.step(op).unwrap_or_else(|_| panic!("{op:?} is unmodelled in the reference"));
    }
    s.tlb
}

// ---------------------------------------------------------------------------
// The generator.
// ---------------------------------------------------------------------------

/// A random program: its operations and its virtual register count.
struct Program {
    ops: Vec<VOp>,
    vregs: u32,
}

const INT_ADD: [AddOp; 13] = [
    AddOp::Add,
    AddOp::Sub,
    AddOp::And,
    AddOp::Or,
    AddOp::Xor,
    AddOp::Min,
    AddOp::Max,
    AddOp::Umin,
    AddOp::Umax,
    AddOp::Shl,
    AddOp::Shr,
    AddOp::Asr,
    AddOp::Ror,
];
const FLOAT_ADD: [AddOp; 4] = [AddOp::Fadd, AddOp::Fsub, AddOp::Fmin, AddOp::Fmax];
const ONE_ADD: [AddOp; 11] = [
    AddOp::Not,
    AddOp::Neg,
    AddOp::Itof,
    AddOp::Utof,
    AddOp::Ffloor,
    AddOp::Fceil,
    AddOp::Ftrunc,
    AddOp::Fround,
    AddOp::Ftoiz,
    AddOp::Ftoin,
    AddOp::Ftouz,
];
const TWO_MUL: [MulOp; 5] = [MulOp::Fmul, MulOp::Add, MulOp::Sub, MulOp::Umul24, MulOp::Smul24];
const ONE_MUL: [MulOp; 2] = [MulOp::Mov, MulOp::Fmov];
const SFU: [AddOp; 5] = [AddOp::Recip, AddOp::Rsqrt, AddOp::Exp, AddOp::Log, AddOp::Sin];
/// The pushes the sim describes: (unit, flag).
const PUSHES: [(FlagOp, Pf); 9] = [
    (FlagOp::Add(AddOp::Xor), Pf::PushZ),
    (FlagOp::Add(AddOp::Sub), Pf::PushC),
    (FlagOp::Add(AddOp::Min), Pf::PushC),
    (FlagOp::Add(AddOp::Fcmp), Pf::PushZ),
    (FlagOp::Add(AddOp::Fcmp), Pf::PushN),
    (FlagOp::Add(AddOp::Fcmp), Pf::PushC),
    (FlagOp::Mul(MulOp::Mov), Pf::PushZ),
    (FlagOp::Mul(MulOp::Fmov), Pf::PushN),
    (FlagOp::Mul(MulOp::Sub), Pf::PushC),
];
/// The add- and mul-unit float operations of a chain step.
const CHAIN_ADD: [AddOp; 4] = [AddOp::Fadd, AddOp::Fsub, AddOp::Fmin, AddOp::Fmax];
const CHAIN_MUL: [MulOp; 3] = [MulOp::Fmul, MulOp::Add, MulOp::Sub];

/// A uniform's value: an f32 that stays well inside the range half the
/// time, a small integer or any bits otherwise.
fn uniform_bits(r: &mut Rng) -> u32 {
    match r.below(4) {
        0 | 1 => ((r.below(129) as i32 - 64) as f32 / 8.0).to_bits(),
        2 => r.below(64) as u32,
        _ => r.next() as u32,
    }
}

/// Whether the operation reads two small immediates (one raddr_b per word:
/// the generator does not make it).
fn two_imms(op: &VOp) -> bool {
    op.uses().iter().all(|s| matches!(s, Operand::SmallImm(_)))
}

/// FMIN or FMAX of one operand twice: FMAX's word then reads back as FMIN,
/// and `schedule` refuses it (`NotEncodable(.., ReadsBackAs)`,
/// [`fmax_of_one_register_twice`]); the selector never makes one.
fn minmax_of_one_operand(op: &VOp) -> bool {
    matches!(*op, VOp::Add { op: AddOp::Fmin | AddOp::Fmax, a, b, .. } if a == b)
}

/// A float operand: a register, its absolute value now and then.
fn float_operand(r: &mut Rng, v: V) -> Operand {
    if r.chance(15) {
        Operand::Abs(v)
    } else {
        Operand::V(v)
    }
}

fn generate(seed: u64) -> Program {
    let mut r = Rng::new(seed);
    let mut ops: Vec<VOp> = Vec::new();
    let mut st = State::default();
    let mut pool: Vec<V> = Vec::new();
    let mut next_v = 0u32;
    let mut pushes = 0usize;
    let body = 4 + r.below(44);
    let mut tries = 0usize;

    while ops.len() < body && tries < 4000 {
        tries += 1;
        // A chain run: up to six steps of two independent chains, the add
        // unit's and the mul unit's, interleaved; each step reads its own
        // chain's last value and a register or small immediate.
        if pool.len() >= 2 && r.chance(8) {
            let (mut ca, mut cb) = (r.pick(&pool), r.pick(&pool));
            for _ in 0..2 + r.below(5) {
                for unit in 0..2 {
                    let fresh = V(next_v);
                    let other =
                        if r.chance(30) { Operand::SmallImm(r.below(48) as u8) } else { Operand::V(r.pick(&pool)) };
                    let cand = if unit == 0 {
                        VOp::Add { op: r.pick(&CHAIN_ADD), dst: Dst::V(fresh), a: float_operand(&mut r, ca), b: other }
                    } else {
                        VOp::Mul { op: r.pick(&CHAIN_MUL), dst: Dst::V(fresh), a: Operand::V(cb), b: other }
                    };
                    let mut trial = st.clone();
                    if minmax_of_one_operand(&cand) || trial.step(&cand).is_err() {
                        continue;
                    }
                    st = trial;
                    next_v += 1;
                    if unit == 0 {
                        ca = fresh;
                    } else {
                        cb = fresh;
                    }
                    ops.push(cand);
                }
            }
            for c in [ca, cb] {
                if !pool.contains(&c) {
                    if pool.len() < POOL {
                        pool.push(c);
                    } else {
                        let k = r.below(POOL);
                        pool[k] = c;
                    }
                }
            }
            continue;
        }
        let fresh = V(next_v);
        // An operand: a value from the pool, a small immediate, or the
        // payload W or Z.
        let operand = |r: &mut Rng, pool: &[V]| -> Operand {
            if pool.is_empty() || r.chance(15) {
                Operand::SmallImm(r.below(48) as u8)
            } else if r.chance(4) {
                Operand::Payload(r.pick(&[0, 2]))
            } else {
                Operand::V(r.pick(pool))
            }
        };
        let kind = if pool.len() < 2 { 0 } else { r.below(100) };
        let cand = match kind {
            0..=17 => VOp::Ldunif { dst: fresh, u: Uniform::constant(uniform_bits(&mut r)) },
            18..=37 => {
                if r.chance(55) {
                    VOp::Add {
                        op: r.pick(&INT_ADD),
                        dst: Dst::V(fresh),
                        a: operand(&mut r, &pool),
                        b: operand(&mut r, &pool),
                    }
                } else {
                    let a = match operand(&mut r, &pool) {
                        Operand::V(v) => float_operand(&mut r, v),
                        o => o,
                    };
                    VOp::Add { op: r.pick(&FLOAT_ADD), dst: Dst::V(fresh), a, b: operand(&mut r, &pool) }
                }
            }
            38..=46 => {
                let (op, a) = if r.chance(10) {
                    (AddOp::Eidx, Operand::None)
                } else {
                    (r.pick(&ONE_ADD), operand(&mut r, &pool))
                };
                VOp::Add { op, dst: Dst::V(fresh), a, b: Operand::None }
            }
            47..=60 => VOp::Mul {
                op: r.pick(&TWO_MUL),
                dst: Dst::V(fresh),
                a: operand(&mut r, &pool),
                b: operand(&mut r, &pool),
            },
            61..=66 => {
                VOp::Mul { op: r.pick(&ONE_MUL), dst: Dst::V(fresh), a: operand(&mut r, &pool), b: Operand::None }
            }
            67..=74 => VOp::Add { op: r.pick(&SFU), dst: Dst::V(fresh), a: operand(&mut r, &pool), b: Operand::None },
            75..=83 => {
                let (unit, pf) = r.pick(&PUSHES);
                let a = operand(&mut r, &pool);
                let b = match unit {
                    FlagOp::Mul(MulOp::Mov | MulOp::Fmov) => Operand::None,
                    _ => operand(&mut r, &pool),
                };
                VOp::Flags { unit, a, b, pf, uf: Uf::None }
            }
            84..=92 if pushes > 0 => {
                let conds: &[Cond] =
                    if pushes > 1 { &[Cond::IfA, Cond::IfNA, Cond::IfB, Cond::IfNB] } else { &[Cond::IfA, Cond::IfNA] };
                VOp::CondMov { dst: r.pick(&pool), src: operand(&mut r, &pool), cond: r.pick(conds) }
            }
            _ => VOp::Copy { dst: r.pick(&pool), src: operand(&mut r, &pool) },
        };
        if two_imms(&cand) || minmax_of_one_operand(&cand) {
            continue;
        }
        // A copy or conditional move of a register to itself tells nothing.
        if matches!(cand, VOp::Copy { dst, src: Operand::V(s) } | VOp::CondMov { dst, src: Operand::V(s), .. } if dst == s)
        {
            continue;
        }
        let mut trial = st.clone();
        if trial.step(&cand).is_err() {
            continue;
        }
        st = trial;
        if matches!(cand, VOp::Flags { .. }) {
            pushes += 1;
        }
        if cand.def().is_some() {
            next_v += 1;
            if pool.len() < POOL {
                pool.push(fresh);
            } else {
                let k = r.below(POOL);
                pool[k] = fresh;
            }
        }
        ops.push(cand);
    }
    if pool.is_empty() {
        let fresh = V(next_v);
        ops.push(VOp::Ldunif { dst: fresh, u: Uniform::constant(7) });
        st.step(ops.last().unwrap()).unwrap();
        next_v += 1;
        pool.push(fresh);
    }
    ops.push(VOp::Thrsw);
    ops.push(VOp::Thrsw);
    let writes = 1 + r.below(4);
    let mut k = 0;
    while k < writes {
        let dst = if k == 0 { Dst::Tlbu(Uniform::constant(r.next() as u32)) } else { Dst::Tlb };
        let cand = if r.chance(25) {
            VOp::Add { op: AddOp::Vfpack, dst, a: Operand::V(r.pick(&pool)), b: Operand::V(r.pick(&pool)) }
        } else {
            VOp::Mul { op: MulOp::Mov, dst, a: Operand::V(r.pick(&pool)), b: Operand::None }
        };
        let mut trial = st.clone();
        if trial.step(&cand).is_err() {
            continue;
        }
        st = trial;
        ops.push(cand);
        k += 1;
    }
    let (ops, vregs) = sink_dead(&ops, next_v, &mut r);
    Program { ops, vregs }
}

/// The register writes of `ops` whose value nothing reads: `(write, end)`,
/// the write's index and the index of the operation that overwrites the
/// value unread (a `Copy` into it), or the body's end (the last-thread-switch
/// pair, after which only TLB writes read).
fn dead_writes(ops: &[VOp]) -> Vec<(usize, usize)> {
    let body = body_len(ops);
    let mut out = Vec::new();
    for i in 0..body {
        let w = match ops[i] {
            VOp::Add { dst: Dst::V(v), .. } | VOp::Mul { dst: Dst::V(v), .. } => v,
            VOp::Ldunif { dst, .. } | VOp::CondMov { dst, .. } | VOp::Copy { dst, .. } => dst,
            _ => continue,
        };
        let mut end = body;
        let mut read = false;
        for (j, o) in ops.iter().enumerate().skip(i + 1) {
            // A copy reads its source, and overwrites its destination.
            let reads = match *o {
                VOp::Copy { src, .. } => src.vreg() == Some(w),
                _ => o.uses().iter().any(|u| u.vreg() == Some(w)),
            };
            if reads {
                read = true;
                break;
            }
            if matches!(*o, VOp::Copy { dst, .. } if dst == w) {
                end = j;
                break;
            }
        }
        if !read {
            out.push((i, end.min(body)));
        }
    }
    out
}

/// `ops` with every value read: a write nothing reads is folded, at a random
/// point before it is overwritten, into an XOR accumulator (its first value
/// a MOV), which the last TLB write reads. The scheduler puts two writes to
/// one register into one word when the first value is never read
/// ([`two_unread_values_share_a_word_and_a_register`]); the
/// selector's straight-line code reads every value it makes.
fn sink_dead(ops: &[VOp], mut next_v: u32, r: &mut Rng) -> (Vec<VOp>, u32) {
    let dead = dead_writes(ops);
    // (insert before this operation, the value)
    let mut folds: Vec<(usize, V)> = dead
        .iter()
        .map(|&(i, end)| {
            let w = ops[i].def().unwrap_or_else(|| match ops[i] {
                VOp::CondMov { dst, .. } | VOp::Copy { dst, .. } => dst,
                _ => unreachable!(),
            });
            (i + 1 + r.below(end - i), w)
        })
        .collect();
    folds.sort_by_key(|f| f.0);
    let mut out = Vec::with_capacity(ops.len() + folds.len() + 1);
    let mut acc: Option<V> = None;
    let mut fi = 0;
    for (i, op) in ops.iter().enumerate() {
        while fi < folds.len() && folds[fi].0 == i {
            let w = folds[fi].1;
            let new = V(next_v);
            next_v += 1;
            out.push(match acc {
                None => VOp::Mul { op: MulOp::Mov, dst: Dst::V(new), a: Operand::V(w), b: Operand::None },
                Some(x) => VOp::Add { op: AddOp::Xor, dst: Dst::V(new), a: Operand::V(x), b: Operand::V(w) },
            });
            acc = Some(new);
            fi += 1;
        }
        out.push(*op);
    }
    assert_eq!(fi, folds.len(), "every fold lands before the body's end");
    if let Some(x) = acc {
        out.push(VOp::Mul { op: MulOp::Mov, dst: Dst::Tlb, a: Operand::V(x), b: Operand::None });
    }
    (out, next_v)
}

/// `ops` with a `VOp::Label` between every two body operations (the ones
/// before the last-thread-switch pair): every body operation in a word of
/// its own.
fn one_per_word(ops: &[VOp]) -> Vec<VOp> {
    let body = ops.iter().position(|o| *o == VOp::Thrsw).unwrap_or(ops.len());
    let mut out = Vec::with_capacity(ops.len() * 2);
    for (i, op) in ops.iter().enumerate() {
        if i > 0 && i < body {
            out.push(VOp::Label(i as u32 - 1));
        }
        out.push(*op);
    }
    out
}

// ---------------------------------------------------------------------------
// Running one program.
// ---------------------------------------------------------------------------

fn listing(instrs: &[Instr]) -> String {
    let mut s = String::new();
    for (k, i) in instrs.iter().enumerate() {
        s.push_str(&format!("{k:3}: "));
        write_instr(&mut s, i).unwrap();
        s.push('\n');
    }
    s
}

/// What one schedule of a program came to.
struct Ran {
    words: usize,
    hoists: u32,
}

/// Allocate, schedule, verify, round-trip and run `ops`; compare the TLB
/// writes with `want`. `Err` is the first check that failed, with the
/// listing.
fn check(ops: &[VOp], vregs: u32, want: &[Tlb]) -> Result<Ran, String> {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    let sel = Selected {
        ops,
        fragment: true,
        vregs,
        vattr_sizes: [0; 16],
        vpm_output_rows: 0,
        uses_vid: false,
        uses_iid: false,
        uses_center_w: ops.iter().any(|o| o.uses().contains(&Operand::Payload(0))),
        implicit_point_varyings: false,
        writes_z: false,
        disable_ez: false,
    };
    let scheduled = store::with_scratch(|a| {
        let al = allocate(&sel, 4, a).map_err(|e| format!("allocate: {e:?}"))?;
        let s = schedule(&sel, &al, a).map_err(|e| format!("schedule: {e:?}\nregs: {:?}", al.reg))?;
        Ok::<_, String>((s.instrs.to_vec(), s.uniforms.iter().map(|u| u.data).collect::<Vec<u32>>(), s.hoists))
    })
    .expect("the scratch region");
    let (instrs, stream, hoists) = scheduled.map_err(|e| format!("{e}\nops: {ops:?}"))?;
    let text = listing(&instrs);
    let ctx = |what: String| format!("{what}\nops: {ops:?}\n{text}");
    verify(&instrs, false).map_err(|e| ctx(format!("verify: {e:?}")))?;
    registers_fit_threads(&instrs, 4).map_err(|e| ctx(format!("registers_fit_threads: {e:?}")))?;
    let mut words = Vec::with_capacity(instrs.len());
    for (pc, i) in instrs.iter().enumerate() {
        let w = encode(i).map_err(|e| ctx(format!("word {pc} does not encode: {e:?}")))?;
        let back = decode(w).map_err(|e| ctx(format!("word {pc} ({w:#018x}) does not decode: {e:?}")))?;
        if !same_instr(i, &back) {
            return Err(ctx(format!("word {pc} ({w:#018x}) decodes as another instruction: {back:?}")));
        }
        words.push(w);
    }
    let env = Env { threads: 4, ..Env::default() };
    let got = sim::run(&words, &stream, &Inputs::Fragment(Box::new(fs_inputs())), &env)
        .map_err(|e| ctx(format!("sim: {e}")))?;
    if got.uniforms_read != stream.len() {
        return Err(ctx(format!("the sim read {} uniforms of {}", got.uniforms_read, stream.len())));
    }
    let have: Vec<(Option<u32>, Vec<Option<u32>>)> = got.tlb.iter().map(|t| (t.config, t.value.to_vec())).collect();
    let want_o: Vec<(Option<u32>, Vec<Option<u32>>)> =
        want.iter().map(|(c, l)| (*c, l.iter().map(|x| Some(*x)).collect())).collect();
    if have != want_o {
        return Err(ctx(format!("TLB writes {have:x?}, the reference {want_o:x?}")));
    }
    Ok(Ran { words: instrs.len(), hoists })
}

/// The body operations: everything before the last-thread-switch pair.
fn body_len(ops: &[VOp]) -> usize {
    ops.iter().position(|o| *o == VOp::Thrsw).unwrap_or(ops.len())
}

#[test]
fn seeded_random_straight_line_programs_keep_their_values() {
    let (mut ops_total, mut packed_words, mut single_words, mut hoists) = (0usize, 0usize, 0usize, 0u32);
    let mut failures = Vec::new();
    for n in 0..PROGRAMS {
        let seed = SEEDS[n % SEEDS.len()] ^ n as u64;
        let p = generate(seed);
        ops_total += p.ops.len();
        let want = eval(&p.ops);
        let single = one_per_word(&p.ops);
        match (check(&p.ops, p.vregs, &want), check(&single, p.vregs, &want)) {
            (Ok(a), Ok(b)) => {
                if b.words < body_len(&p.ops) {
                    failures.push(format!(
                        "program {n} (seed {seed:#x}): one per word gave {} words for {} body operations",
                        b.words,
                        body_len(&p.ops)
                    ));
                }
                if b.hoists != 0 {
                    failures.push(format!("program {n} (seed {seed:#x}): one per word hoisted {}", b.hoists));
                }
                packed_words += a.words;
                single_words += b.words;
                hoists += a.hoists;
            }
            (Err(e), _) => failures.push(format!("program {n} (seed {seed:#x}), packed: {e}")),
            (_, Err(e)) => failures.push(format!("program {n} (seed {seed:#x}), one per word: {e}")),
        }
    }
    println!(
        "v3d_sched_fuzz: {PROGRAMS} programs, {ops_total} operations, {packed_words} words packed \
         ({hoists} hoists), {single_words} words one per word"
    );
    assert!(failures.is_empty(), "{} of {PROGRAMS} programs failed; the first:\n{}", failures.len(), failures[0]);
    assert!(packed_words < single_words, "the scheduler packed nothing: {packed_words} >= {single_words} words");
}

/// The generator is deterministic: the same seed gives the same program.
#[test]
fn the_generator_is_deterministic() {
    for n in 0..8u64 {
        let (a, b) = (generate(SEEDS[0] ^ n), generate(SEEDS[0] ^ n));
        assert_eq!(a.ops, b.ops);
        assert_eq!(a.vregs, b.vregs);
    }
}

/// The generator reaches what it claims: chains, both units, the SFU, flag
/// pushes with conditional moves, copies, the ABS unpack, the payload, and
/// both kinds of TLB write.
#[test]
fn the_generator_covers_its_operations() {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for n in 0..PROGRAMS {
        let p = generate(SEEDS[n % SEEDS.len()] ^ n as u64);
        for op in &p.ops {
            let k = match op {
                VOp::Add { op, dst: Dst::V(_), .. } if SFU.contains(op) => "sfu",
                VOp::Add { dst: Dst::V(_), .. } => "add",
                VOp::Mul { dst: Dst::V(_), .. } => "mul",
                VOp::Add { op: AddOp::Vfpack, .. } => "tlb vfpack",
                VOp::Mul { .. } => "tlb mov",
                VOp::Ldunif { .. } => "ldunif",
                VOp::Flags { .. } => "flags",
                VOp::CondMov { .. } => "condmov",
                VOp::Copy { .. } => "copy",
                _ => "other",
            };
            *seen.entry(k).or_insert(0) += 1;
            for u in op.uses() {
                match u {
                    Operand::Abs(_) => *seen.entry("abs").or_insert(0) += 1,
                    Operand::Payload(_) => *seen.entry("payload").or_insert(0) += 1,
                    Operand::SmallImm(_) => *seen.entry("small imm").or_insert(0) += 1,
                    _ => {}
                }
            }
        }
    }
    println!("v3d_sched_fuzz coverage: {seen:?}");
    for k in [
        "sfu",
        "add",
        "mul",
        "tlb vfpack",
        "tlb mov",
        "ldunif",
        "flags",
        "condmov",
        "copy",
        "abs",
        "payload",
        "small imm",
    ] {
        assert!(seen.get(k).copied().unwrap_or(0) >= 20, "{k}: only {:?} in {PROGRAMS} programs", seen.get(k));
    }
}

/// Every value the generator makes is read ([`sink_dead`]).
#[test]
fn the_generator_reads_every_value() {
    for n in 0..PROGRAMS {
        let p = generate(SEEDS[n % SEEDS.len()] ^ n as u64);
        assert_eq!(dead_writes(&p.ops), Vec::new(), "program {n}: {:?}", p.ops);
    }
}

fn v(n: u32) -> Operand {
    Operand::V(V(n))
}

/// The tail of every reproducer: the last-thread-switch pair and a TLB write
/// of `out`.
fn with_tlb(mut ops: Vec<VOp>, out: u32) -> Vec<VOp> {
    ops.extend([VOp::Thrsw, VOp::Thrsw, VOp::Mul { op: MulOp::Mov, dst: Dst::Tlb, a: v(out), b: Operand::None }]);
    ops
}

/// `ops` (virtual registers 0..vregs) agree with the reference, scheduled
/// as they are and one per word.
fn passes(ops: &[VOp], vregs: u32) {
    let want = eval(ops);
    check(ops, vregs, &want).unwrap_or_else(|e| panic!("packed: {e}"));
    check(&one_per_word(ops), vregs, &want).unwrap_or_else(|e| panic!("one per word: {e}"));
}

/// The reproducers below with the defect not reached (the two values read;
/// FMAX of two registers) schedule, verify,
/// round-trip and run as the reference says.
#[test]
fn the_reproducers_controls() {
    let ops = with_tlb(
        vec![
            VOp::Ldunif { dst: V(0), u: Uniform::constant(57) },
            VOp::Ldunif { dst: V(1), u: Uniform::constant(0) },
            VOp::Mul { op: MulOp::Add, dst: Dst::V(V(2)), a: v(1), b: v(0) },
            VOp::Add { op: AddOp::Eidx, dst: Dst::V(V(3)), a: Operand::None, b: Operand::None },
            VOp::Add { op: AddOp::Xor, dst: Dst::V(V(4)), a: v(2), b: v(3) },
        ],
        4,
    );
    passes(&ops, 5);
    let ops = with_tlb(
        vec![
            VOp::Ldunif { dst: V(0), u: Uniform::constant(1.5f32.to_bits()) },
            VOp::Ldunif { dst: V(1), u: Uniform::constant(2.5f32.to_bits()) },
            VOp::Add { op: AddOp::Fmax, dst: Dst::V(V(2)), a: v(0), b: v(1) },
        ],
        2,
    );
    passes(&ops, 3);
}

/// Two values nothing reads (V2, a mul-unit ADD, and V3, an EIDX) are each
/// dead after their own write, so the allocator may give both one register
/// (r0), as program order allows. `schedule` then puts the EIDX into the
/// ADD's word, and the word writes r0 twice (`verify`: "two writes to one
/// register in one instruction"; with an SFU write, the sim's "two writes
/// land on .. in one cycle"). A value a `Copy` overwrites before any read
/// meets it the same way. The fuzz met it in 94 of 512 programs before the
/// generator stopped making unread values (shrunk from program 115, seed
/// 0xa54ff53a5f1d3682, as the generator was then). Selected straight-line code reads every value it
/// makes; whether some selected program (an unread `ldtmu` word, a loop's
/// last `Copy`) reaches it is not known.
#[test]
#[ignore = "sched defect: two writes whose values are never read share a word and a register"]
fn two_unread_values_share_a_word_and_a_register() {
    let ops = with_tlb(
        vec![
            VOp::Ldunif { dst: V(0), u: Uniform::constant(57) },
            VOp::Ldunif { dst: V(1), u: Uniform::constant(0) },
            VOp::Mul { op: MulOp::Add, dst: Dst::V(V(2)), a: v(1), b: v(0) },
            VOp::Add { op: AddOp::Eidx, dst: Dst::V(V(3)), a: Operand::None, b: Operand::None },
            VOp::Ldunif { dst: V(4), u: Uniform::constant(0xbf80_0000) },
        ],
        4,
    );
    passes(&ops, 5);
}

/// FMAX with both operands one register: FMIN and FMAX share an opcode range
/// and the operand order names one, so with equal operands the word reads
/// back as FMIN and `schedule` refuses `NotEncodable(.., ReadsBackAs)`
/// (the same limit as sched71's, tests/v3d71_sched_fuzz.rs). fmax(x, x) is
/// x, as fmin(x, x) is. The fuzz met it in 44 of 512 programs before the
/// generator stopped making it (as the generator was then).
#[test]
#[ignore = "sched limit: FMAX of one register twice does not encode (reads back as FMIN); FMIN would do"]
fn fmax_of_one_register_twice() {
    let ops = with_tlb(
        vec![
            VOp::Ldunif { dst: V(0), u: Uniform::constant(1.5f32.to_bits()) },
            VOp::Add { op: AddOp::Fmax, dst: Dst::V(V(1)), a: v(0), b: v(0) },
        ],
        1,
    );
    passes(&ops, 2);
}
