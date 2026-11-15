// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! sched71 over seeded random straight-line programs (plan #365, item
//! sched71-fuzz). A small xorshift with fixed seeds builds fragment shaders
//! from the operations the 7.1 back end hands `pack71`: add- and mul-unit
//! integer and float operations on virtual registers, `Ldunif` constants
//! (some into a register given rf0, the plain `ldunif`, some not, `ldunifrf`),
//! small immediates, SFU operations, flag pushes with conditional moves, and
//! copies; then the last-thread-switch pair and one to four TLB writes of
//! values the body computed. Each program gets a hand allocation that keeps
//! the alloc71 rule of design note (b): a value in rf0 is never live across
//! an implicit rf0 write (any `Ldunif`), is never a conditional move's or a
//! copy's destination, and is never a TLB output; the other values take
//! rf4..rf31 (4 threads), a register freed at its last read and reused.
//!
//! For each program:
//!
//! 1. `pack71` succeeds, or refuses with a named `Sched71Error`; the only
//!    refusal the generator provokes on purpose is an operation with two
//!    small immediates (`NotPlaceable`), and any other one fails the test;
//! 2. `verify71(&words, false)` and `registers_fit_threads71(.., 4)` pass;
//! 3. every word round-trips through codec71 (`decode71`, then `encode71`
//!    gives the word back);
//! 4. `sim::run_gen(Gen::V71, ..)` on the words and the packed uniform
//!    stream reads every uniform and writes the TLB with exactly the values
//!    of [`eval`], a reference evaluator that runs the operations one at a
//!    time in program order (the sim's documented semantics: f32 for the
//!    float operations, i32 for the integer ones, the exact f32 function for
//!    the SFU, Z/N/C as `sim.rs`'s header lists them).
//!
//! The generator evaluates as it goes and drops a candidate operation whose
//! operands or result the sim does not model (a NaN, a denormal, SIN outside
//! [-0.5, 0.5]); RSQRT2 is not generated (sim71 does not model it). The
//! summary (programs, words, refusals by name) is printed with
//! `--nocapture`. A disagreement that is a sched71 or sim71 defect becomes a
//! separate `#[ignore = "reason"]` test with the minimal program; this file
//! changes nothing under `src/`.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_sched_fuzz
//! -- --nocapture`.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test"))]

use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::alloc71::{Allocation71, Rf};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::encode::EncodeError;
use yakogl::shaderc::online::v3d::isa::{small_imm_value, Cond, Pf, Uf};
use yakogl::shaderc::online::v3d::listing71::write_instr71;
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sched71::{pack71, Sched71Error};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::v3d::vop71::{AddOp71, Dst71, FlagOp71, Instr71, MulOp71, Op71, Sfu71, Src71, Uniform, V};

/// `store::with_scratch` has one scratch region and refuses a second user;
/// the tests of this file take turns.
static SCRATCH: Mutex<()> = Mutex::new(());

/// Programs generated.
const PROGRAMS: usize = 600;
/// The fixed seeds: program `n` uses `SEEDS[n % len] ^ n`.
const SEEDS: [u64; 4] = [0x9e37_79b9_7f4a_7c15, 0xd1b5_4a32_d192_ed03, 0x8cb9_2ba7_2f3d_8dd7, 0x2545_f491_4f6c_dd1d];
/// At most this many values are kept for later reads; a new value past it
/// replaces one, which is then dead.
const POOL: usize = 12;
/// The registers a value that is not in rf0 may take at 4 threads: rf4..rf31
/// (rf1..rf3 are the fragment payload, rf0 is the implicit destination).
const FIRST_RF: u8 = 4;
const END_RF: u8 = 32;

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
// The reference evaluator.
// ---------------------------------------------------------------------------

type Lanes = [u32; LANES];

/// What the sim does not model: the operation is not generated.
#[derive(Debug)]
struct Unmodelled;

/// The values of the virtual registers, the A and B flags, and the TLB
/// writes, after the operations run so far.
#[derive(Clone, Default)]
struct State {
    v: BTreeMap<u32, Lanes>,
    flag_a: Option<[bool; LANES]>,
    flag_b: Option<[bool; LANES]>,
    tlb: Vec<Lanes>,
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
    fn src(&self, s: Src71) -> Lanes {
        match s {
            Src71::V(v) => *self.v.get(&v.0).unwrap_or_else(|| panic!("{v:?} read before it is written")),
            Src71::SmallImm(i) => [small_imm_value(i).expect("a small immediate"); LANES],
            other => panic!("operand {other:?} is not generated"),
        }
    }

    /// An add-unit operation's value and its Z/N/C flags (`None` where the
    /// sim does not describe the flag).
    fn add(&self, op: AddOp71, a: Src71, b: Src71) -> Result<(Lanes, [[Option<bool>; 3]; LANES]), Unmodelled> {
        use AddOp71::*;
        let x = if a == Src71::None { [0; LANES] } else { self.src(a) };
        let y = if b == Src71::None { [0; LANES] } else { self.src(b) };
        let mut r = [0u32; LANES];
        let mut f = [[None; 3]; LANES];
        for l in 0..LANES {
            let (p, q) = (x[l], y[l]);
            r[l] = match op {
                Fadd => fres(fop(p)? + fop(q)?)?,
                Fsub => fres(fop(p)? - fop(q)?)?,
                Fmin => fres(fop(p)?.min(fop(q)?))?,
                Fmax => fres(fop(p)?.max(fop(q)?))?,
                Fcmp => {
                    let (s, t) = (fop(p)?, fop(q)?);
                    f[l] = [Some(s == t), Some(s < t), Some(s <= t)];
                    0
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
                Eidx => l as u32,
                Mov => {
                    f[l][0] = Some(p == 0);
                    p
                }
                Fmov => {
                    let s = fop(p)?;
                    f[l] = [Some(s == 0.0), Some(s < 0.0), None];
                    p
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

    fn mul(&self, op: MulOp71, a: Src71, b: Src71) -> Result<(Lanes, [[Option<bool>; 3]; LANES]), Unmodelled> {
        let x = self.src(a);
        let y = if b == Src71::None { [0; LANES] } else { self.src(b) };
        let mut r = [0u32; LANES];
        let mut f = [[None; 3]; LANES];
        for l in 0..LANES {
            let (p, q) = (x[l], y[l]);
            r[l] = match op {
                MulOp71::Fmul => fres(fop(p)? * fop(q)?)?,
                MulOp71::Add => p.wrapping_add(q),
                MulOp71::Sub => {
                    f[l][2] = Some(p < q);
                    p.wrapping_sub(q)
                }
                MulOp71::Umul24 => (p & 0xff_ffff).wrapping_mul(q & 0xff_ffff),
                MulOp71::Smul24 => ((((p << 8) as i32) >> 8).wrapping_mul(((q << 8) as i32) >> 8)) as u32,
                MulOp71::Mov => {
                    f[l][0] = Some(p == 0);
                    p
                }
                MulOp71::Fmov => {
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
    fn step(&mut self, op: &Op71) -> Result<(), Unmodelled> {
        match *op {
            Op71::Add { op, dst, a, b } => {
                let (r, _) = self.add(op, a, b)?;
                self.write(dst, r);
            }
            Op71::Mul { op, dst, a, b } => {
                let (r, _) = self.mul(op, a, b)?;
                self.write(dst, r);
            }
            Op71::Sfu { op, dst, a } => {
                let (r, _) = self.add(op.add_op(), a, Src71::None)?;
                self.v.insert(dst.0, r);
            }
            Op71::Ldunif { dst, u } => {
                self.v.insert(dst.0, [u.data; LANES]);
            }
            Op71::Flags { unit, a, b, pf, uf } => {
                assert_eq!(uf, Uf::None, "flag updates are not generated");
                let (_, f) = match unit {
                    FlagOp71::Add(op) => self.add(op, a, b)?,
                    FlagOp71::Mul(op) => self.mul(op, a, b)?,
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
            Op71::CondMov { dst, src, cond } => {
                let x = self.src(src);
                let m = self.cond(cond);
                let d = self.v.get_mut(&dst.0).expect("a conditional move into a written register");
                for l in 0..LANES {
                    if m[l] {
                        d[l] = x[l];
                    }
                }
            }
            Op71::Copy { dst, src } => {
                let x = self.src(src);
                self.v.insert(dst.0, x);
            }
            Op71::Thrsw => {}
            other => panic!("{other:?} is not generated"),
        }
        Ok(())
    }

    fn write(&mut self, d: Dst71, r: Lanes) {
        match d {
            Dst71::V(v) => {
                self.v.insert(v.0, r);
            }
            Dst71::Tlb => self.tlb.push(r),
            Dst71::Nop => {}
            other => panic!("destination {other:?} is not generated"),
        }
    }
}

/// The reference: the operations one at a time, in program order; the TLB
/// writes it makes.
fn eval(ops: &[Op71]) -> Vec<Lanes> {
    let mut s = State::default();
    for op in ops {
        s.step(op).unwrap_or_else(|_| panic!("{op:?} is unmodelled in the reference"));
    }
    s.tlb
}

// ---------------------------------------------------------------------------
// The generator.
// ---------------------------------------------------------------------------

/// A random program and its allocation (one register per virtual one).
struct Program {
    ops: Vec<Op71>,
    regs: Vec<u8>,
    /// The first operation with two small immediates, which pack71 refuses
    /// (`NotPlaceable`).
    two_imms: Option<usize>,
}

const INT_ADD: [AddOp71; 13] = [
    AddOp71::Add,
    AddOp71::Sub,
    AddOp71::And,
    AddOp71::Or,
    AddOp71::Xor,
    AddOp71::Min,
    AddOp71::Max,
    AddOp71::Umin,
    AddOp71::Umax,
    AddOp71::Shl,
    AddOp71::Shr,
    AddOp71::Asr,
    AddOp71::Ror,
];
const FLOAT_ADD: [AddOp71; 4] = [AddOp71::Fadd, AddOp71::Fsub, AddOp71::Fmin, AddOp71::Fmax];
const ONE_ADD: [AddOp71; 4] = [AddOp71::Not, AddOp71::Neg, AddOp71::Itof, AddOp71::Fmov];
const TWO_MUL: [MulOp71; 5] = [MulOp71::Fmul, MulOp71::Add, MulOp71::Sub, MulOp71::Umul24, MulOp71::Smul24];
const ONE_MUL: [MulOp71; 2] = [MulOp71::Mov, MulOp71::Fmov];
const SFU: [Sfu71; 5] = [Sfu71::Recip, Sfu71::Rsqrt, Sfu71::Exp, Sfu71::Log, Sfu71::Sin];
/// The pushes the sim describes: (unit, flag).
const PUSHES: [(FlagOp71, Pf); 9] = [
    (FlagOp71::Add(AddOp71::Xor), Pf::PushZ),
    (FlagOp71::Add(AddOp71::Sub), Pf::PushC),
    (FlagOp71::Add(AddOp71::Min), Pf::PushC),
    (FlagOp71::Add(AddOp71::Fcmp), Pf::PushZ),
    (FlagOp71::Add(AddOp71::Fcmp), Pf::PushN),
    (FlagOp71::Add(AddOp71::Fcmp), Pf::PushC),
    (FlagOp71::Mul(MulOp71::Mov), Pf::PushZ),
    (FlagOp71::Mul(MulOp71::Fmov), Pf::PushN),
    (FlagOp71::Mul(MulOp71::Sub), Pf::PushC),
];

/// A uniform's value: an f32 that stays well inside the range half the
/// time, any bits otherwise.
fn uniform_bits(r: &mut Rng) -> u32 {
    match r.below(4) {
        0 | 1 => ((r.below(129) as i32 - 64) as f32 / 8.0).to_bits(),
        2 => r.below(64) as u32,
        _ => r.next() as u32,
    }
}

fn generate(seed: u64) -> Program {
    let mut r = Rng::new(seed);
    let mut ops: Vec<Op71> = Vec::new();
    let mut st = State::default();
    let mut pool: Vec<V> = Vec::new();
    let mut next_v = 0u32;
    let mut pushes = 0usize;
    let body = 4 + r.below(36);

    while ops.len() < body {
        let fresh = V(next_v);
        // An operand: a value from the pool or a small immediate.
        let operand = |r: &mut Rng, pool: &[V]| -> Src71 {
            if pool.is_empty() || r.chance(15) {
                Src71::SmallImm(r.below(48) as u8)
            } else {
                Src71::V(r.pick(pool))
            }
        };
        let kind = if pool.len() < 2 { 0 } else { r.below(100) };
        let cand = match kind {
            0..=19 => Op71::Ldunif { dst: fresh, u: Uniform::constant(uniform_bits(&mut r)) },
            20..=39 => {
                let op = if r.chance(55) { r.pick(&INT_ADD) } else { r.pick(&FLOAT_ADD) };
                Op71::Add { op, dst: Dst71::V(fresh), a: operand(&mut r, &pool), b: operand(&mut r, &pool) }
            }
            40..=47 => {
                let (op, a) = if r.chance(10) {
                    (AddOp71::Eidx, Src71::None)
                } else {
                    (r.pick(&ONE_ADD), operand(&mut r, &pool))
                };
                Op71::Add { op, dst: Dst71::V(fresh), a, b: Src71::None }
            }
            48..=61 => Op71::Mul {
                op: r.pick(&TWO_MUL),
                dst: Dst71::V(fresh),
                a: operand(&mut r, &pool),
                b: operand(&mut r, &pool),
            },
            62..=67 => {
                Op71::Mul { op: r.pick(&ONE_MUL), dst: Dst71::V(fresh), a: operand(&mut r, &pool), b: Src71::None }
            }
            68..=75 => Op71::Sfu { op: r.pick(&SFU), dst: fresh, a: operand(&mut r, &pool) },
            76..=84 => {
                let (unit, pf) = r.pick(&PUSHES);
                let a = operand(&mut r, &pool);
                let b = match unit {
                    FlagOp71::Mul(MulOp71::Mov | MulOp71::Fmov) => Src71::None,
                    _ => operand(&mut r, &pool),
                };
                Op71::Flags { unit, a, b, pf, uf: Uf::None }
            }
            85..=93 if pushes > 0 => {
                let conds: &[Cond] =
                    if pushes > 1 { &[Cond::IfA, Cond::IfNA, Cond::IfB, Cond::IfNB] } else { &[Cond::IfA, Cond::IfNA] };
                Op71::CondMov { dst: r.pick(&pool), src: operand(&mut r, &pool), cond: r.pick(conds) }
            }
            _ => Op71::Copy { dst: r.pick(&pool), src: operand(&mut r, &pool) },
        };
        // Two small immediates in one operation are the one refusal the
        // generator provokes (one per word at 7.1): kept now and then, the
        // second operand made a register otherwise.
        let cand = match cand {
            Op71::Add { op, dst, a: a @ Src71::SmallImm(_), b: Src71::SmallImm(_) }
                if !pool.is_empty() && r.chance(90) =>
            {
                Op71::Add { op, dst, a, b: Src71::V(r.pick(&pool)) }
            }
            Op71::Mul { op, dst, a: a @ Src71::SmallImm(_), b: Src71::SmallImm(_) }
                if !pool.is_empty() && r.chance(90) =>
            {
                Op71::Mul { op, dst, a, b: Src71::V(r.pick(&pool)) }
            }
            Op71::Flags { unit, a: a @ Src71::SmallImm(_), b: Src71::SmallImm(_), pf, uf }
                if !pool.is_empty() && r.chance(90) =>
            {
                Op71::Flags { unit, a, b: Src71::V(r.pick(&pool)), pf, uf }
            }
            c => c,
        };
        // A copy of a register to itself is dropped by sched71 and tells
        // nothing; a conditional move of one is a no-op either way.
        if matches!(cand, Op71::Copy { dst, src: Src71::V(s) } | Op71::CondMov { dst, src: Src71::V(s), .. } if dst == s)
        {
            continue;
        }
        let mut trial = st.clone();
        if trial.step(&cand).is_err() {
            continue;
        }
        st = trial;
        if matches!(cand, Op71::Flags { .. }) {
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
        ops.push(Op71::Ldunif { dst: fresh, u: Uniform::constant(7) });
        next_v += 1;
        pool.push(fresh);
    }
    ops.push(Op71::Thrsw);
    ops.push(Op71::Thrsw);
    for _ in 0..1 + r.below(4) {
        ops.push(Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: Src71::V(r.pick(&pool)), b: Src71::None });
    }
    let regs = allocate(&ops, next_v, &mut r);
    let two_imms = ops.iter().position(|op| op.uses().iter().all(|s| matches!(s, Src71::SmallImm(_))));
    Program { ops, regs, two_imms }
}

/// The hand allocation: rf0 for a value that may hold it (design note (b)),
/// with a 50% chance; rf4..rf31 otherwise, a register free from the
/// operation after its value's last read (the defining operation may take
/// the register of a value it reads last), chosen at random.
fn allocate(ops: &[Op71], vregs: u32, r: &mut Rng) -> Vec<u8> {
    let n = vregs as usize;
    let mut def = vec![usize::MAX; n];
    let mut last = vec![0usize; n];
    // Never in rf0: written by a conditional move or a copy, or a TLB output.
    let mut no_rf0 = vec![false; n];
    for (i, op) in ops.iter().enumerate() {
        if let Some(v) = op.def() {
            def[v.0 as usize] = i;
            last[v.0 as usize] = i;
        }
        for s in op.uses() {
            if let Some(v) = s.vreg() {
                last[v.0 as usize] = i;
            }
        }
        match *op {
            Op71::CondMov { dst, .. } | Op71::Copy { dst, .. } => no_rf0[dst.0 as usize] = true,
            Op71::Mul { dst: Dst71::Tlb, a: Src71::V(v), .. } => no_rf0[v.0 as usize] = true,
            _ => {}
        }
    }
    let ldunif_at: Vec<usize> =
        ops.iter().enumerate().filter(|(_, op)| op.writes_rf0_implicitly()).map(|(i, _)| i).collect();
    let mut regs = vec![0u8; n];
    // The register's current value's last read, or None when free.
    let mut busy: [Option<usize>; 64] = [None; 64];
    let mut rf0_until: Option<usize> = None;
    for (i, op) in ops.iter().enumerate() {
        let Some(v) = op.def() else { continue };
        let k = v.0 as usize;
        let (d, l) = (def[k], last[k]);
        assert_eq!(d, i);
        // rf0: no implicit rf0 write after the definition up to the last
        // read, and rf0 free (the previous rf0 value read last at or before
        // here).
        let crosses = ldunif_at.iter().any(|&j| j > d && j <= l);
        if !no_rf0[k] && !crosses && rf0_until.is_none_or(|u| u <= i) && r.chance(50) {
            regs[k] = 0;
            rf0_until = Some(l);
            continue;
        }
        let free: Vec<u8> = (FIRST_RF..END_RF).filter(|&p| busy[p as usize].is_none_or(|u| u <= i)).collect();
        let p = r.pick(&free);
        regs[k] = p;
        busy[p as usize] = Some(l);
    }
    regs
}

// ---------------------------------------------------------------------------
// Running one program.
// ---------------------------------------------------------------------------

fn listing(instrs: &[Instr71]) -> String {
    let mut s = String::new();
    for (k, i) in instrs.iter().enumerate() {
        s.push_str(&format!("{k:3}: "));
        write_instr71(&mut s, i).unwrap();
        s.push('\n');
    }
    s
}

/// What one program came to.
enum Verdict {
    Ok { words: usize },
    Refused(Sched71Error),
}

/// Pack, verify, round-trip and run `ops` with registers `regs`; `Err` is the
/// first check that failed, with the listing.
fn check(ops: &[Op71], regs: &[u8]) -> Result<Verdict, String> {
    let rfs: Vec<Rf> = regs.iter().map(|&r| Rf(r)).collect();
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    let packed = store::with_scratch(|a| {
        let al = Allocation71 { reg: &rfs, rf_end: regs.iter().copied().max().unwrap_or(0) + 1 };
        pack71(ops, &al, a).map(|p| (p.instrs.to_vec(), p.uniforms.iter().map(|u| u.data).collect::<Vec<u32>>()))
    })
    .expect("the scratch region");
    let (instrs, stream) = match packed {
        Ok(p) => p,
        Err(e) => return Ok(Verdict::Refused(e)),
    };
    let text = listing(&instrs);
    let ctx = |what: String| format!("{what}\nops: {ops:?}\nregs: {regs:?}\n{text}");
    verify71(&instrs, false).map_err(|e| ctx(format!("verify71: {e:?}")))?;
    registers_fit_threads71(&instrs, 4).map_err(|e| ctx(format!("registers_fit_threads71: {e:?}")))?;
    let mut words = Vec::with_capacity(instrs.len());
    for (pc, i) in instrs.iter().enumerate() {
        let w = encode71(i).map_err(|e| ctx(format!("word {pc} does not encode: {e:?}")))?;
        let back = decode71(w).map_err(|e| ctx(format!("word {pc} ({w:#018x}) does not decode: {e:?}")))?;
        let again = encode71(&back).map_err(|e| ctx(format!("word {pc} does not re-encode: {e:?}")))?;
        if again != w {
            return Err(ctx(format!("word {pc}: {w:#018x} re-encodes as {again:#018x}")));
        }
        words.push(w);
    }
    let one = 1.0f32.to_bits();
    let inputs = FsInputs {
        w: [one; LANES],
        w_centroid: [one; LANES],
        z: [0.5f32.to_bits(); LANES],
        x: [one; LANES],
        y: [one; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    let env = Env { threads: 4, ..Env::default() };
    let got = sim::run_gen(Gen::V71, &words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| ctx(format!("sim71: {e}")))?;
    if got.uniforms_read != stream.len() {
        return Err(ctx(format!("sim71 read {} uniforms of {}", got.uniforms_read, stream.len())));
    }
    let want = eval(ops);
    let have: Vec<Vec<Option<u32>>> = got.tlb.iter().map(|t| t.value.to_vec()).collect();
    let want_o: Vec<Vec<Option<u32>>> = want.iter().map(|l| l.iter().map(|x| Some(*x)).collect()).collect();
    if have != want_o {
        return Err(ctx(format!("TLB writes {have:x?}, the reference {want_o:x?}")));
    }
    Ok(Verdict::Ok { words: instrs.len() })
}

/// The variant name of a refusal, without its payload.
fn name(e: &Sched71Error) -> String {
    format!("{e:?}").split(['(', ' ']).next().unwrap().to_string()
}

/// A refusal the generator's programs are known to meet: the two small
/// immediates it provokes, and the two sched71 limits the `#[ignore]`d cases
/// below reproduce. Any other refusal fails the test.
fn known_refusal(p: &Program, e: Sched71Error) -> bool {
    let rf0 = |s: Src71| s.vreg().is_some_and(|v| p.regs[v.0 as usize] == 0);
    match e {
        Sched71Error::NotPlaceable(i) => p.two_imms == Some(i),
        // fmax_with_one_register_twice
        Sched71Error::NotEncodable(i, EncodeError::ReadsBackAs) => {
            matches!(p.ops[i], Op71::Add { op: AddOp71::Fmax, a, b, .. } if a == b)
        }
        // a_coalesced_move_into_rf0: operation i reads a value in rf0 that a
        // move from rf0 to rf0 defined
        Sched71Error::Rf0Clobbered(i) => p.ops[i].uses().iter().any(|&s| {
            rf0(s)
                && p.ops[..i].iter().any(|op| {
                    matches!(*op, Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(d), a, .. }
                        if Src71::V(d) == s && rf0(a))
                })
        }),
        _ => false,
    }
}

#[test]
fn seeded_random_straight_line_programs() {
    let (mut ok, mut words) = (0usize, 0usize);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut ops_total = 0usize;
    let mut rf0_values = 0usize;
    let mut failures = Vec::new();
    for n in 0..PROGRAMS {
        let seed = SEEDS[n % SEEDS.len()] ^ n as u64;
        let p = generate(seed);
        ops_total += p.ops.len();
        rf0_values += p.ops.iter().filter(|op| op.def().is_some_and(|v| p.regs[v.0 as usize] == 0)).count();
        match check(&p.ops, &p.regs) {
            Ok(Verdict::Ok { words: w }) => {
                ok += 1;
                words += w;
            }
            Ok(Verdict::Refused(e)) => {
                if !known_refusal(&p, e) {
                    failures.push(format!("program {n} (seed {seed:#x}): refused {e:?}\nops: {:?}", p.ops));
                }
                *refused.entry(name(&e)).or_insert(0) += 1;
            }
            Err(e) => failures.push(format!("program {n} (seed {seed:#x}): {e}")),
        }
    }
    println!(
        "v3d71_sched_fuzz: {PROGRAMS} programs, {ops_total} operations ({rf0_values} values in rf0), \
         {ok} packed and checked, {words} words; refusals: {refused:?}"
    );
    assert!(failures.is_empty(), "{} of {PROGRAMS} programs failed; the first:\n{}", failures.len(), failures[0]);
    assert!(ok >= PROGRAMS * 9 / 10, "only {ok} of {PROGRAMS} programs packed");
}

/// The generator is deterministic: the same seed gives the same program.
#[test]
fn the_generator_is_deterministic() {
    for n in 0..8u64 {
        let (a, b) = (generate(SEEDS[0] ^ n), generate(SEEDS[0] ^ n));
        assert_eq!(a.ops, b.ops);
        assert_eq!(a.regs, b.regs);
    }
}

fn v(n: u32) -> Src71 {
    Src71::V(V(n))
}

/// The tail of every reproducer: the last-thread-switch pair and a TLB write
/// of `out`.
fn with_tlb(mut ops: Vec<Op71>, out: u32) -> Vec<Op71> {
    ops.extend([Op71::Thrsw, Op71::Thrsw, Op71::Mul { op: MulOp71::Mov, dst: Dst71::Tlb, a: v(out), b: Src71::None }]);
    ops
}

fn passes(ops: &[Op71], regs: &[u8]) {
    match check(ops, regs) {
        Ok(Verdict::Ok { .. }) => {}
        Ok(Verdict::Refused(e)) => panic!("pack71 refuses: {e:?}"),
        Err(e) => panic!("{e}"),
    }
}

/// The reproducers below with the defect not reached (the move's
/// destination in another register; FMIN instead of FMAX) pack, verify,
/// round-trip and run as the reference says.
#[test]
fn the_reproducers_controls() {
    let ops = with_tlb(
        vec![
            Op71::Ldunif { dst: V(0), u: Uniform::constant(5) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(V(1)), a: v(0), b: Src71::None },
            Op71::Add { op: AddOp71::Add, dst: Dst71::V(V(2)), a: v(1), b: v(1) },
        ],
        2,
    );
    passes(&ops, &[0, 5, 4]);
    let ops = with_tlb(
        vec![
            Op71::Ldunif { dst: V(0), u: Uniform::constant(1.5f32.to_bits()) },
            Op71::Add { op: AddOp71::Fmin, dst: Dst71::V(V(1)), a: v(0), b: v(0) },
        ],
        1,
    );
    passes(&ops, &[4, 5]);
}

/// A move whose source and destination alloc71 gave one register (a
/// coalesced copy) is dropped by `Packer71::place`, which carries the
/// source's ready time over to the destination but not rf0's owner
/// (sched71.rs:730-739): with both in rf0, the next read of the destination
/// is refused as `Rf0Clobbered` (:800-802), although rf0 still holds the
/// value. The fuzz meets it in 3 of 600 programs (seeds 0xd1b54a32d192ed12,
/// 0x2545f4914f6cdc4e, 0x2545f4914f6cdcee).
#[test]
#[ignore = "sched71 defect: a dropped rf0-to-rf0 move leaves rf0 owned by the source (Rf0Clobbered)"]
fn a_coalesced_move_into_rf0() {
    let ops = with_tlb(
        vec![
            Op71::Ldunif { dst: V(0), u: Uniform::constant(5) },
            Op71::Mul { op: MulOp71::Mov, dst: Dst71::V(V(1)), a: v(0), b: Src71::None },
            Op71::Add { op: AddOp71::Add, dst: Dst71::V(V(2)), a: v(1), b: v(1) },
        ],
        2,
    );
    passes(&ops, &[0, 0, 4]);
}

/// FMAX with both operands one register (no unpack): FMIN and FMAX share an
/// opcode range and the operand order names one (Mesa 26.2.1
/// qpu/qpu_pack.c:1232-1239 decoding, :1706-1721 encoding), so with equal
/// operands the word reads back as FMIN and `pack71` refuses
/// `NotEncodable(.., ReadsBackAs)`. fmax(x, x) is x, as fmin(x, x) is: pack71
/// could place FMIN (or a MOV). The fuzz meets it in 29 of 600 programs.
#[test]
#[ignore = "sched71 limit: FMAX of one register twice does not encode (reads back as FMIN); FMIN would do"]
fn fmax_with_one_register_twice() {
    let ops = with_tlb(
        vec![
            Op71::Ldunif { dst: V(0), u: Uniform::constant(1.5f32.to_bits()) },
            Op71::Add { op: AddOp71::Fmax, dst: Dst71::V(V(1)), a: v(0), b: v(0) },
        ],
        1,
    );
    passes(&ops, &[4, 5]);
}
