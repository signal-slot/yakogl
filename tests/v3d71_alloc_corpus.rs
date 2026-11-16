// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `allocate71`'s rules over every vertex/fragment pair of `tools/shaders/`
//! (the pairs `tests/v3d_online_corpus.rs` links): each stage's [`Selected`]
//! comes from the public front half as chain.rs `compile_program_keyed` makes
//! it, goes through `select71`, then `allocate71` at 4 threads, and at 2 where
//! 4 fails or is not allowed (`select::tmu_input_writes` > 4), as chain71's
//! `build71` does, one pressure level after the other.
//!
//! The checks do not use alloc71's own liveness: a backward dataflow over
//! the operations' control flow (loop back edges, forward branches) says
//! what is live after every operation, and from it
//!
//! * no two virtual registers live at once share a register (a definition
//!   interferes with everything live after it; a value lent a lifetime hole
//!   is not live there),
//! * rf0 ([`RF_IMPLICIT`]) is assigned only to [`Op71::Ldunif`] destinations
//!   and no such value is live across a write of rf0 (`ldunif`, `ldvary`), a
//!   read of ldvary's C term, a thread switch or control flow, and a plain
//!   `ldunif` never lands between an `ldvary` and a read of its C term,
//! * rf1..rf3 (the fragment payload, read through `Src71::Payload`) are
//!   never handed out, whatever the stage,
//! * every register is below `rf_end71(threads)` and `rf_end` is one past
//!   the highest register from `RF_FIRST71` up that a defined value uses.
//!
//! The counts that allocate at 4 threads, at 2 and not at all are pinned, and
//! the refusals are listed by name.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_alloc_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::{self, IoKind};
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::powervr::unroll::{unroll, Trip};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::alloc71::{allocate71, rf_end71, Alloc71Error, Rf, RF_FIRST71, RF_IMPLICIT};
use yakogl::shaderc::online::v3d::isa::BranchCond;
use yakogl::shaderc::online::v3d::prepare::{fragment_inputs, prepare_fragment_with, prepare_vertex_with, SOp};
use yakogl::shaderc::online::v3d::select::{
    select_fragment, select_vertex, tmu_input_writes, Pressure, SelectError, Selected,
};
use yakogl::shaderc::online::v3d::select71::{select71, Select71Error};
use yakogl::shaderc::online::v3d::vop71::{Op71, Selected71, Src71};

/// chain.rs `PRESSURES` (a fragment shader skips `LazyAttributes`).
const PRESSURES: [Pressure; 4] = [Pressure::None, Pressure::LazyAttributes, Pressure::Remat, Pressure::Serial];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("tools/shaders is readable") {
        let p = e.expect("a directory entry").path();
        if p.is_dir() {
            sources(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("vert" | "frag")) {
            out.push(p);
        }
    }
}

/// One bit per virtual register.
type Bits = Vec<u64>;
fn set(b: &mut Bits, v: usize) {
    b[v / 64] |= 1 << (v % 64);
}
fn clear(b: &mut Bits, v: usize) {
    b[v / 64] &= !(1 << (v % 64));
}
fn has(b: &Bits, v: usize) -> bool {
    b[v / 64] >> (v % 64) & 1 == 1
}

/// What an operation reads and writes for liveness: a [`Op71::Copy`] defines
/// its destination (it does not read it), a [`Op71::CondMov`] reads and keeps
/// it.
fn reads(op: &Op71) -> Vec<usize> {
    let srcs: Vec<Src71> = match *op {
        Op71::Copy { src, .. } => vec![src],
        _ => op.uses().to_vec(),
    };
    srcs.iter().filter_map(|s| s.vreg()).map(|v| v.0 as usize).collect()
}
fn writes(op: &Op71) -> Option<usize> {
    match *op {
        Op71::Copy { dst, .. } => Some(dst.0 as usize),
        _ => op.def().map(|v| v.0 as usize),
    }
}

/// The successors of operation `i`.
fn succs(ops: &[Op71], i: usize, label_at: &BTreeMap<u32, usize>, head_of: &BTreeMap<usize, usize>) -> Vec<usize> {
    let mut s = Vec::new();
    let next = |s: &mut Vec<usize>| {
        if i + 1 < ops.len() {
            s.push(i + 1);
        }
    };
    match ops[i] {
        Op71::BackEdge { cond, .. } => {
            s.push(head_of[&i]);
            if cond != BranchCond::Always {
                next(&mut s);
            }
        }
        Op71::FwdBranch { cond, label, .. } => {
            s.push(label_at[&label.0]);
            if cond != BranchCond::Always {
                next(&mut s);
            }
        }
        _ => next(&mut s),
    }
    s
}

/// Live-out per operation, by backward dataflow to a fixed point.
fn liveness(sel: &Selected71<'_>) -> Vec<Bits> {
    let ops = sel.ops;
    let words = (sel.vregs as usize).div_ceil(64).max(1);
    let mut label_at = BTreeMap::new();
    let mut head_of = BTreeMap::new();
    let mut heads = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match *op {
            Op71::Label(l) => {
                label_at.insert(l.0, i);
            }
            Op71::LoopHead => heads.push(i),
            Op71::BackEdge { .. } => {
                head_of.insert(i, heads.pop().expect("a back edge closes a head"));
            }
            _ => {}
        }
    }
    let succ: Vec<Vec<usize>> = (0..ops.len()).map(|i| succs(ops, i, &label_at, &head_of)).collect();
    let mut live_in = vec![vec![0u64; words]; ops.len()];
    let mut live_out = vec![vec![0u64; words]; ops.len()];
    loop {
        let mut changed = false;
        for i in (0..ops.len()).rev() {
            let mut out = vec![0u64; words];
            for &s in &succ[i] {
                for (o, x) in out.iter_mut().zip(&live_in[s]) {
                    *o |= x;
                }
            }
            let mut inn = out.clone();
            if let Some(w) = writes(&ops[i]) {
                clear(&mut inn, w);
            }
            for r in reads(&ops[i]) {
                set(&mut inn, r);
            }
            if inn != live_in[i] || out != live_out[i] {
                changed = true;
                live_in[i] = inn;
                live_out[i] = out;
            }
        }
        if !changed {
            return live_out;
        }
    }
}

/// Every rule above, against `al` for `sel` at `threads`. Returns the
/// violations (empty: the allocation is sound).
fn check(sel: &Selected71<'_>, reg: &[Rf], rf_end: u8, threads: u32) -> Vec<String> {
    let ops = sel.ops;
    let mut bad = Vec::new();
    let defined: Vec<bool> = {
        let mut d = vec![false; sel.vregs as usize];
        for op in ops {
            if let Some(w) = writes(op) {
                d[w] = true;
            }
        }
        d
    };
    let end = rf_end71(threads).expect("a thread count the V3D runs");
    let live_out = liveness(sel);
    /* the registers: bounds, the payload, rf_end */
    let mut top = RF_FIRST71;
    for (v, &r) in reg.iter().enumerate().filter(|(v, _)| defined[*v]) {
        if r.0 >= end {
            bad.push(format!("v{v} in rf{} at or past rf_end71({threads}) = {end}", r.0));
        }
        if (1..RF_FIRST71).contains(&r.0) {
            bad.push(format!("v{v} in rf{}, a payload register", r.0));
        }
        if r.0 >= RF_FIRST71 {
            top = top.max(r.0 + 1);
        }
    }
    if rf_end != top {
        bad.push(format!("rf_end {rf_end}, the highest register used makes it {top}"));
    }
    /* interference: a definition against everything live after it */
    for (i, op) in ops.iter().enumerate() {
        let Some(w) = writes(op) else { continue };
        for v in 0..sel.vregs as usize {
            if v != w && has(&live_out[i], v) && reg[v] == reg[w] {
                bad.push(format!("op {i}: v{w} is written into rf{}, which v{v} holds and still needs", reg[w].0));
            }
        }
    }
    /* rf0 */
    let mut last_ldvary: Option<usize> = None;
    for (i, op) in ops.iter().enumerate() {
        let Some(w) = op.def() else {
            if matches!(*op, Op71::CondMov { dst, .. } | Op71::Copy { dst, .. } if reg[dst.0 as usize] == RF_IMPLICIT) {
                bad.push(format!("op {i}: an ALU write of rf0"));
            }
            continue;
        };
        if reg[w.0 as usize] == RF_IMPLICIT && !matches!(op, Op71::Ldunif { .. }) {
            bad.push(format!("op {i}: rf0 given to the destination of {op:?}"));
        }
    }
    for (i, op) in ops.iter().enumerate() {
        /* a value held in rf0 across operation i (live after it, not defined by it) */
        for v in 0..sel.vregs as usize {
            if reg[v] != RF_IMPLICIT || !defined[v] || writes(op) == Some(v) || !has(&live_out[i], v) {
                continue;
            }
            let clobbers = op.writes_rf0_implicitly()
                || op.uses().contains(&Src71::LdvaryC)
                || *op == Op71::Thrsw
                || op.is_control_flow();
            if clobbers {
                bad.push(format!("op {i} ({op:?}): v{v} is in rf0 across it"));
            }
        }
        /* a plain ldunif between an ldvary and the read of its C term */
        if let Op71::Ldunif { dst, .. } = *op {
            if reg[dst.0 as usize] == RF_IMPLICIT && last_ldvary.is_some() {
                let c_after = ops[i + 1..]
                    .iter()
                    .find(|o| matches!(o, Op71::Ldvary { .. }) || o.uses().contains(&Src71::LdvaryC))
                    .is_some_and(|o| o.uses().contains(&Src71::LdvaryC));
                if c_after {
                    bad.push(format!("op {i}: a plain ldunif overwrites ldvary's C term (rf0) before it is read"));
                }
            }
        }
        if matches!(op, Op71::Ldvary { .. }) {
            last_ldvary = Some(i);
        }
    }
    bad
}

static RF0_VALUES: AtomicUsize = AtomicUsize::new(0);

fn defined(sel: &Selected71<'_>, v: usize) -> bool {
    sel.ops.iter().any(|o| writes(o) == Some(v))
}

/// One stage through select71 and allocate71 at the pressure levels, as
/// build71 does: Ok(threads) for the first level that allocates, else the
/// refusal by name.
fn allocate_stage<'a>(
    select: impl Fn(Pressure) -> Result<Selected<'a>, SelectError>,
    levels: &[Pressure],
    a: &Arena<'a>,
    label: &str,
    bad: &mut Vec<String>,
) -> Result<u32, String> {
    let mut last = String::from("no pressure level");
    for &pr in levels {
        let sel = match select(pr) {
            Ok(s) => s,
            Err(e) => return Err(format!("select: {e:?}")),
        };
        let s71 = match select71(&sel, a) {
            Ok(s) => s,
            Err(Select71Error::NotOn71(m)) => return Err(format!("select71 NotOn71: {m}")),
            Err(Select71Error::NotYet(m)) => return Err(format!("select71 NotYet: {m}")),
            Err(e) => return Err(format!("select71: {e:?}")),
        };
        let four = tmu_input_writes(sel.ops) <= 16 / 4;
        let try_at = |t: u32| allocate71(&s71, t, a);
        let (al, threads) = match if four { try_at(4) } else { Err(Alloc71Error::DoesNotFit) } {
            Ok(al) => (al, 4),
            Err(Alloc71Error::DoesNotFit) => match try_at(2) {
                Ok(al) => (al, 2),
                Err(Alloc71Error::DoesNotFit) => {
                    last = "alloc71 DoesNotFit at 2 threads".into();
                    continue;
                }
                Err(e) => return Err(format!("alloc71 {e:?}")),
            },
            Err(e) => return Err(format!("alloc71 {e:?}")),
        };
        let in_rf0 = (0..s71.vregs as usize).filter(|&v| al.reg[v] == RF_IMPLICIT && defined(&s71, v)).count();
        RF0_VALUES.fetch_add(in_rf0, Ordering::Relaxed);
        for m in check(&s71, al.reg, al.rf_end, threads) {
            bad.push(format!("{label} ({threads} threads, {pr:?}): {m}"));
        }
        return Ok(threads);
    }
    Err(last)
}

#[derive(Default)]
struct Counts {
    /// stages by the thread count they allocate at
    four: usize,
    two: usize,
    /// stages not allocated, by name
    refused: BTreeMap<String, usize>,
    /// pairs whose front half stops before any stage
    front: BTreeMap<String, usize>,
}

/// One pair, all three stages (fragment, vertex, coordinate).
fn run_pair(name: &str, vs_src: &str, fs_src: &str, c: &mut Counts, bad: &mut Vec<String>) {
    let r = store::with_scratch(|a| {
        let front = |e: String| -> Result<(), String> { Err(e) };
        let v = match compile_to_ir(a, Stage::Vertex, a.alloc_str(vs_src).unwrap()) {
            Ok(l) => l.module,
            Err(e) => return front(format!("front end: {}", e.msg)),
        };
        let f = match compile_to_ir(a, Stage::Fragment, a.alloc_str(fs_src).unwrap()) {
            Ok(l) => l.module,
            Err(e) => return front(format!("front end: {}", e.msg)),
        };
        let v: &ir::Module<'_> = &a.alloc_filled(1, v).unwrap()[0];
        let f: &ir::Module<'_> = &a.alloc_filled(1, f).unwrap()[0];
        stages(name, v, f, a, c, bad)
    });
    match r {
        Some(Ok(())) => {}
        Some(Err(m)) => *c.front.entry(m).or_default() += 1,
        None => *c.front.entry("scratch busy".into()).or_default() += 1,
    }
}

fn unrolled<'a>(m: &'a ir::Module<'a>, a: &'a Arena<'a>) -> Result<(&'a ir::Module<'a>, &'a [Trip]), String> {
    match unroll(m, a) {
        Ok(Some(u)) => Ok((u.module, u.trips)),
        Ok(None) => Ok((m, &[])),
        Err(e) => Err(format!("unroll: {}", e.message())),
    }
}

/// chain.rs `compile_program_keyed`'s front half, then each stage.
fn stages<'a>(
    name: &str,
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    a: &'a Arena<'a>,
    c: &mut Counts,
    bad: &mut Vec<String>,
) -> Result<(), String> {
    let (vs, vs_trips) = unrolled(vs, a)?;
    let (fs, fs_trips) = unrolled(fs, a)?;
    let fsp = prepare_fragment_with(fs, fs_trips, 0, a).map_err(|e| format!("prepare fragment: {e:?}"))?;
    let inputs = fragment_inputs(&fsp, a).map_err(|e| format!("inputs: {e:?}"))?;
    if inputs.len() > 64 {
        return Err("more than 64 fragment shader inputs".into());
    }
    let (mut flat, mut centroid) = (0u64, 0u64);
    let wanted = a.alloc_filled(inputs.len(), (0u8, 0u8)).map_err(|_| "scratch")?;
    for (i, (&(loc, comp), w)) in inputs.iter().zip(wanted.iter_mut()).enumerate() {
        let d = fs
            .inputs
            .iter()
            .find(|d| d.kind == IoKind::Varying && d.location == loc)
            .ok_or("a varying without a declaration")?;
        if d.interp == ir::INTERP_FLAT {
            flat |= 1 << i;
        }
        if d.interp == ir::INTERP_CENTROID {
            centroid |= 1 << i;
        }
        let o = vs
            .outputs
            .iter()
            .find(|o| o.kind == IoKind::Varying && o.name == d.name)
            .ok_or("a varying without a vertex output")?;
        *w = (o.location, comp);
    }
    let vsp = prepare_vertex_with(vs, wanted, vs_trips, 0, a).map_err(|e| format!("prepare vertex: {e:?}"))?;
    let csp = prepare_vertex_with(vs, &[], vs_trips, 0, a).map_err(|e| format!("prepare coord: {e:?}"))?;
    let mut records: Vec<u8> = Vec::new();
    for op in vsp.ops {
        if let SOp::Attr { loc, .. } = *op {
            if !records.contains(&loc) {
                records.push(loc);
            }
        }
    }
    records.sort_unstable();
    if records.len() > 16 {
        return Err("more than 16 vertex attributes".into());
    }
    let mut note = |what: &str, r: Result<u32, String>| match r {
        Ok(4) => c.four += 1,
        Ok(_) => c.two += 1,
        Err(m) => *c.refused.entry(format!("{name} {what}: {m}")).or_default() += 1,
    };
    let fl = format!("{name} fs");
    let r = allocate_stage(
        |pr| select_fragment(&fsp, inputs, flat, centroid, pr, a),
        &[Pressure::None, Pressure::Remat, Pressure::Serial],
        a,
        &fl,
        bad,
    );
    note("fragment", r);
    let vl = format!("{name} vs");
    let r = allocate_stage(|pr| select_vertex(&vsp, &records, false, 0, pr, a), &PRESSURES, a, &vl, bad);
    note("vertex", r);
    let cl = format!("{name} cs");
    let r = allocate_stage(|pr| select_vertex(&csp, &records, true, 0, pr, a), &PRESSURES, a, &cl, bad);
    note("coordinate", r);
    Ok(())
}

#[test]
fn allocate71_is_sound_over_every_corpus_pair() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    assert_eq!(files.len(), 324, "tools/shaders holds 324 .vert/.frag sources; re-derive the pins if it changed");
    let mut c = Counts::default();
    let mut bad = Vec::new();
    let mut pairs = 0usize;
    for v in files.iter().filter(|p| p.extension().is_some_and(|x| x == "vert")) {
        let f = v.with_extension("frag");
        if !f.exists() {
            continue;
        }
        pairs += 1;
        let rel = v.strip_prefix(&root).unwrap().with_extension("").to_string_lossy().into_owned();
        let (vs, fs) = (std::fs::read_to_string(v).unwrap(), std::fs::read_to_string(&f).unwrap());
        run_pair(&rel, &vs, &fs, &mut c, &mut bad);
    }
    println!("alloc71: {pairs} pairs, stages allocated at 4 threads {}, at 2 threads {}", c.four, c.two);
    println!("  front half stops before the stages ({} pairs):", c.front.values().sum::<usize>());
    for (m, n) in &c.front {
        println!("    {n:3} x {m}");
    }
    println!("  stages not allocated ({}):", c.refused.values().sum::<usize>());
    for (m, n) in &c.refused {
        println!("    {n:3} x {m}");
    }
    assert!(
        bad.is_empty(),
        "{} violation(s), first 20:\n{}",
        bad.len(),
        bad.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
    let rf0 = RF0_VALUES.load(Ordering::Relaxed);
    println!("  values the allocator put in rf0: {rf0}");
    assert_eq!(rf0, RF0, "values (counted per allocation) in rf0");
    assert_eq!(pairs, PAIRS, "pairs with both a .vert and a .frag");
    assert_eq!(c.four, FOUR, "stages allocated at 4 threads");
    assert_eq!(c.two, TWO, "stages allocated at 2 threads");
    assert_eq!(c.refused.keys().map(String::as_str).collect::<Vec<_>>(), REFUSED, "stages not allocated, by name");
    assert_eq!(c.refused.values().sum::<usize>(), NOT_ALLOCATED, "stages not allocated");
    assert_eq!(c.front.values().sum::<usize>(), FRONT_STOPS, "pairs the front half stops");
}

/// Pairs with both a `.vert` and a `.frag` (the 4.2 test's link step takes
/// the ones whose two sources compile).
const PAIRS: usize = 36;
/// Of the 31 pairs that reach their stages (3 each: fragment, vertex,
/// coordinate), 92 allocate at 4 threads, none needs 2 and one does not
/// allocate at all.
const FOUR: usize = 92;
const TWO: usize = 0;
const NOT_ALLOCATED: usize = 1;
/// 3 desktop GLSL sources and 2 pairs whose varying has no vertex output.
const FRONT_STOPS: usize = 5;
/// Values in rf0, summed over the allocations made (a stage that fails at
/// a pressure level and is retried counts at each).
const RF0: usize = 256;

/// The stage no pressure level fits in 2 threads (varyings15 reads 15 vec4).
const REFUSED: [&str; 1] = ["es100/varyings15 fragment: alloc71 DoesNotFit at 2 threads"];

#[cfg(test)]
mod checker {
    use super::*;
    use yakogl::shaderc::online::v3d::vop71::{AddOp71, Dst71, Uniform, V};

    fn ld(d: u32) -> Op71 {
        Op71::Ldunif { dst: V(d), u: Uniform::constant(d) }
    }
    fn add(d: u32, a: u32, b: u32) -> Op71 {
        Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(d)), a: Src71::V(V(a)), b: Src71::V(V(b)) }
    }
    fn run(ops: &[Op71], vregs: u32, reg: &[u8], rf_end: u8) -> Vec<String> {
        let sel = Selected71 { ops, fragment: true, vregs };
        let reg: Vec<Rf> = reg.iter().map(|&r| Rf(r)).collect();
        check(&sel, &reg, rf_end, 4)
    }

    /// The checker is not vacuous: each rule rejects a hand-made violation,
    /// and accepts the sound allocation next to it.
    #[test]
    fn each_rule_rejects_its_violation() {
        let ops = [ld(0), ld(1), add(2, 0, 1)];
        assert_eq!(run(&ops, 3, &[4, 5, 4], 6), Vec::<String>::new());
        assert!(!run(&ops, 3, &[4, 4, 5], 6).is_empty(), "two live values in one register");
        assert!(!run(&ops, 3, &[4, 5, 4], 7).is_empty(), "rf_end past the highest register");
        assert!(!run(&ops, 3, &[4, 5, 32], 33).is_empty(), "a register at rf_end71(4)");
        assert!(!run(&ops, 3, &[2, 5, 4], 6).is_empty(), "a payload register handed out");
        /* rf0 for a value an ldvary ends, and for a non-ldunif */
        let ops = [ld(0), Op71::Ldvary { dst: V(1) }, add(2, 0, 1)];
        assert!(!run(&ops, 3, &[0, 4, 5], 6).is_empty(), "rf0 live across an ldvary");
        assert!(!run(&ops, 3, &[4, 0, 5], 6).is_empty(), "rf0 for an ldvary's destination");
        /* a plain ldunif between an ldvary and the read of its C term */
        let ops = [
            Op71::Ldvary { dst: V(0) },
            ld(1),
            Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(2)), a: Src71::V(V(0)), b: Src71::LdvaryC },
            add(3, 1, 2),
        ];
        assert!(!run(&ops, 4, &[4, 0, 5, 6], 7).is_empty(), "ldunif into rf0 inside ldvary's C term");
        assert_eq!(run(&ops, 4, &[4, 5, 6, 7], 8), Vec::<String>::new());
    }
}
