// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The comparator between OUR V3D 7.1 program and MESA's for the same GLSL
//! stage (plan #365, acceptance criterion c2: "match Mesa's 7.1 output
//! through V3D_SHC").
//!
//! Our code will not be word-identical to Mesa's (another scheduler, another
//! allocator), so "match" is defined here, once:
//!
//! * **Matches** = the two programs, decoded with `codec71::decode71` and run
//!   in `sim71` (`run71`) on the same inputs, produce the same outputs: the
//!   same TLB writes (count, configuration word, every lane), the same VPM
//!   rows, the same multisample flags (the discarded lanes). Bit for bit,
//!   except where either run executed an SFU operation (RECIP, RSQRT, EXP,
//!   LOG, SIN: `Outcome::approximate`), whose precision only the hardware
//!   knows: floats are then compared to a relative 1e-5 ([`SFU_REL`]). A
//!   word that does not decode, or a run the interpreter refuses (it stops
//!   with an `Err` naming what it does not model), is a mismatch, never a
//!   pass: the verdict says which side.
//! * **Counts** are reported, not part of "matches": instructions, thrsw
//!   count, ldvary count, the uniform stream's length (the stream as given
//!   with the program), TMU accesses (the instructions writing a TMU magic
//!   register, plus LDTMUs), and the placement of the last thread switch (its
//!   index and the number of instructions after it).
//! * **Policy** ([`Policy::max_worse_percent`]): `Some(n)` makes
//!   [`Verdict::passes`] fail when one of ours (instructions, thrsw, ldvary,
//!   uniform length, TMU accesses) exceeds Mesa's by more than n percent
//!   (any excess where Mesa's is 0). `None` only reports.
//!
//! The verdict carries one line per mismatch ([`Verdict::diffs`]) and one per
//! count that differs ([`Verdict::count_notes`]).
//!
//! The uniform stream belongs to a program (the order of its uniforms is the
//! compiler's choice): a [`Side`] is the words with their stream, and the
//! caller builds both from the same logical values. A [`Case`] is the shared
//! input: the fragment or vertex inputs and the environment (textures,
//! memory). [`random_fs_inputs`] draws them in the style of
//! `tests/v3d_sim_mesa.rs`.

#![allow(dead_code)]

use yakogl::shaderc::online::v3d::codec71::decode71;
use yakogl::shaderc::online::v3d::isa71::{Alu71, Instr71};
use yakogl::shaderc::online::v3d::qpu_ffi::{magic_waddr_name_for, Gen};
use yakogl::shaderc::online::v3d::sim::{Env, FsInputs, Inputs, Outcome, Varying, LANES};
use yakogl::shaderc::online::v3d::sim71::run71;

/// The relative tolerance where an SFU operation ran.
pub const SFU_REL: f32 = 1e-5;

/// One program: its words and the uniform stream it reads.
#[derive(Clone, Debug, Default)]
pub struct Side {
    pub words: Vec<u64>,
    pub uniforms: Vec<u32>,
}

/// One set of inputs both programs run on.
#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub inputs: Inputs,
    pub env: Env,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Policy {
    /// Fail when a count of ours is worse than Mesa's by more than this
    /// percentage; `None` reports only.
    pub max_worse_percent: Option<u32>,
}

/// The structural counts of one program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub instructions: usize,
    pub thrsw: usize,
    pub ldvary: usize,
    pub uniform_len: usize,
    pub tmu_accesses: usize,
    /// Index of the last instruction with a thread switch.
    pub last_thrsw: Option<usize>,
    /// Instructions after it.
    pub after_last_thrsw: Option<usize>,
}

impl Counts {
    /// The counts the policy bounds, by name.
    fn bounded(&self) -> [(&'static str, usize); 5] {
        [
            ("instructions", self.instructions),
            ("thrsw", self.thrsw),
            ("ldvary", self.ldvary),
            ("uniform stream length", self.uniform_len),
            ("TMU accesses", self.tmu_accesses),
        ]
    }
}

#[derive(Clone, Debug, Default)]
pub struct Verdict {
    pub ours: Counts,
    pub mesa: Counts,
    /// One line per mismatch (decode, run, output). Empty = the programs agree.
    pub diffs: Vec<String>,
    /// One line per count that differs.
    pub count_notes: Vec<String>,
    /// Lines for counts of ours worse than Mesa's beyond the policy.
    pub policy_failures: Vec<String>,
    /// Cases whose two runs both completed.
    pub cases_run: usize,
}

impl Verdict {
    /// The outputs agree on every case, and at least one case ran.
    pub fn matches(&self) -> bool {
        self.diffs.is_empty() && self.cases_run > 0
    }
    /// [`Verdict::matches`] and no count beyond the policy.
    pub fn passes(&self) -> bool {
        self.matches() && self.policy_failures.is_empty()
    }
}

/// A decoded instruction; a branch has no ALU half.
enum Decoded {
    Alu(Alu71),
    Branch,
}

fn decode_all(what: &str, words: &[u64], diffs: &mut Vec<String>) -> Option<Vec<Decoded>> {
    let mut out = Vec::new();
    let mut ok = true;
    for (i, w) in words.iter().enumerate() {
        match decode71(*w) {
            Ok(Instr71::Alu(a)) => out.push(Decoded::Alu(a)),
            Ok(Instr71::Branch(_)) => out.push(Decoded::Branch),
            Err(e) => {
                diffs.push(format!("{what}: word {i} {w:#018x} does not decode: {e:?}"));
                ok = false;
            }
        }
    }
    ok.then_some(out)
}

fn is_tmu_write(waddr: u8, magic: bool) -> bool {
    magic && magic_waddr_name_for(Gen::V71, waddr).is_some_and(|n| n.starts_with("tmu"))
}

fn count(code: &[Decoded], side: &Side) -> Counts {
    let mut c = Counts { instructions: code.len(), uniform_len: side.uniforms.len(), ..Counts::default() };
    for (i, ins) in code.iter().enumerate() {
        let Decoded::Alu(a) = ins else { continue };
        if a.sig.thrsw {
            c.thrsw += 1;
            c.last_thrsw = Some(i);
        }
        c.ldvary += a.sig.ldvary as usize;
        c.tmu_accesses += a.sig.ldtmu as usize
            + is_tmu_write(a.add.waddr, a.add.magic) as usize
            + is_tmu_write(a.mul.waddr, a.mul.magic) as usize;
    }
    c.after_last_thrsw = c.last_thrsw.map(|i| code.len() - 1 - i);
    c
}

fn same_lane(a: Option<u32>, b: Option<u32>, rel: bool) -> bool {
    if a == b {
        return true;
    }
    let (Some(a), Some(b)) = (a, b) else { return false };
    if !rel {
        return false;
    }
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    if x.is_nan() || y.is_nan() {
        return x.is_nan() && y.is_nan();
    }
    x.is_finite() && y.is_finite() && (x - y).abs() <= SFU_REL * x.abs().max(y.abs())
}

fn show(v: Option<u32>) -> String {
    v.map_or("undef".to_string(), |v| format!("{v:#010x} ({})", f32::from_bits(v)))
}

fn compare_lanes(what: &str, a: &[Option<u32>], b: &[Option<u32>], rel: bool, diffs: &mut Vec<String>) {
    for l in 0..LANES {
        if !same_lane(a[l], b[l], rel) {
            diffs.push(format!("{what} lane {l}: ours {} mesa {}", show(a[l]), show(b[l])));
        }
    }
}

fn compare_outcomes(case: &str, ours: &Outcome, mesa: &Outcome, diffs: &mut Vec<String>) {
    let rel = !ours.approximate.is_empty() || !mesa.approximate.is_empty();
    if ours.tlb.len() != mesa.tlb.len() {
        diffs.push(format!("{case}: TLB writes: ours {} mesa {}", ours.tlb.len(), mesa.tlb.len()));
    }
    for (i, (o, m)) in ours.tlb.iter().zip(&mesa.tlb).enumerate() {
        if o.config != m.config {
            diffs.push(format!("{case}: TLB write {i} config: ours {:?} mesa {:?}", o.config, m.config));
        }
        compare_lanes(&format!("{case}: TLB write {i}"), &o.value, &m.value, rel, diffs);
    }
    if ours.vpm_out.len() != mesa.vpm_out.len() {
        diffs.push(format!("{case}: VPM rows: ours {} mesa {}", ours.vpm_out.len(), mesa.vpm_out.len()));
    }
    for (i, (o, m)) in ours.vpm_out.iter().zip(&mesa.vpm_out).enumerate() {
        match (o, m) {
            (None, None) => {}
            (Some(o), Some(m)) => {
                let o: Vec<_> = o.iter().map(|v| Some(*v)).collect();
                let m: Vec<_> = m.iter().map(|v| Some(*v)).collect();
                compare_lanes(&format!("{case}: VPM row {i}"), &o, &m, rel, diffs);
            }
            _ => diffs.push(format!("{case}: VPM row {i}: written by only one ({} / {})", o.is_some(), m.is_some())),
        }
    }
    if ours.msf != mesa.msf {
        diffs.push(format!("{case}: multisample flags: ours {:?} mesa {:?}", ours.msf, mesa.msf));
    }
}

/// Compare `ours` with `mesa` on `cases`.
pub fn compare(ours: &Side, mesa: &Side, cases: &[Case], policy: Policy) -> Verdict {
    let mut v = Verdict::default();
    let o = decode_all("ours", &ours.words, &mut v.diffs);
    let m = decode_all("mesa", &mesa.words, &mut v.diffs);
    let (Some(o), Some(m)) = (o, m) else { return v };
    v.ours = count(&o, ours);
    v.mesa = count(&m, mesa);

    let (a, b) = (v.ours, v.mesa);
    for ((name, x), (_, y)) in a.bounded().into_iter().zip(b.bounded()) {
        if x != y {
            v.count_notes.push(format!("{name}: ours {x} mesa {y}"));
        }
        if let Some(n) = policy.max_worse_percent {
            if x > y && (x - y) * 100 > y * n as usize {
                v.policy_failures.push(format!("{name}: ours {x} is worse than mesa {y} by more than {n}%"));
            }
        }
    }
    if a.after_last_thrsw != b.after_last_thrsw {
        v.count_notes.push(format!(
            "last thrsw: ours at {:?} ({:?} after) mesa at {:?} ({:?} after)",
            a.last_thrsw, a.after_last_thrsw, b.last_thrsw, b.after_last_thrsw
        ));
    }

    for c in cases {
        let ro = run71(&ours.words, &ours.uniforms, &c.inputs, &c.env);
        let rm = run71(&mesa.words, &mesa.uniforms, &c.inputs, &c.env);
        match (ro, rm) {
            (Ok(ro), Ok(rm)) => {
                v.cases_run += 1;
                compare_outcomes(&c.name, &ro, &rm, &mut v.diffs);
            }
            (ro, rm) => {
                for (who, r) in [("ours", ro), ("mesa", rm)] {
                    if let Err(e) = r {
                        v.diffs.push(format!("{}: {who} did not run: {e}", c.name));
                    }
                }
            }
        }
    }
    v
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    /// A float in [-2, 2] with |x| >= 0.25 (as tests/v3d_sim_mesa.rs draws).
    fn float(&mut self) -> f32 {
        let x = (self.next() % 7001) as f32 / 4000.0 + 0.25;
        if self.next() & 1 == 0 {
            x
        } else {
            -x
        }
    }
    fn lanes(&mut self, positive: bool) -> [u32; LANES] {
        std::array::from_fn(|_| {
            let f = self.float();
            (if positive { f.abs() } else { f }).to_bits()
        })
    }
}

/// Fragment inputs from random values: W and Z positive, `nvary` varyings
/// (random interpolated value and coefficient per lane).
pub fn random_fs_inputs(seed: u64, nvary: usize) -> Inputs {
    let mut r = Rng(seed.max(1) ^ 0x9e37_79b9_7f4a_7c15);
    let w = r.lanes(true);
    let z = r.lanes(true);
    Inputs::Fragment(Box::new(FsInputs {
        w,
        w_centroid: w,
        z,
        x: std::array::from_fn(|l| (l as f32 + 0.5).to_bits()),
        y: std::array::from_fn(|l| ((l / 4) as f32 + 0.5).to_bits()),
        back_facing: [false; LANES],
        varyings: (0..nvary).map(|_| Varying { vary: r.lanes(false), c: r.lanes(false) }).collect(),
    }))
}

/// `n` cases of [`random_fs_inputs`] with a one-thread environment.
pub fn random_fs_cases(seed: u64, nvary: usize, n: usize) -> Vec<Case> {
    (0..n)
        .map(|i| Case {
            name: format!("random#{i}"),
            inputs: random_fs_inputs(seed.wrapping_add(i as u64 * 7919), nvary),
            env: Env { threads: 1, ..Env::default() },
        })
        .collect()
}
