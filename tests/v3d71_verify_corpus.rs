// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `verify71` (Mesa's qpu_validate.c rules at 7.1) against Mesa's own 7.1
//! output.
//!
//! Mesa's validator accepts every program its compiler emits, so `verify71`
//! must accept them too:
//!
//! 1. every whole program of `tests/fixtures/v3d_mesa_programs_71.txt` (the
//!    7.1 programs `tests/v3d_sim_mesa.rs` runs: the fs, vs and cs of each
//!    pair), counted and pinned;
//! 2. with `V3D_SHC` set, every program `tools/v3d/glsl2v3d.py --device 71`
//!    regenerates (`tools/v3d/qpu_counts_71.tsv`'s), their instruction counts
//!    against the table's; without it a named gap (`gap: V3D_SHC`), never a
//!    pass;
//! 3. mutations: an accepted program with one rule broken is rejected, with
//!    that rule's message.
//!
//! `tests/v3d71_ref/**` and `tools/v3d/hazards71.tsv` are not on this tree
//! yet; their cases join when they land.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_verify_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use yakogl::shaderc::online::v3d::codec71::decode71;
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Alu71, Instr71, MulOp71};
use yakogl::shaderc::online::v3d::verify71::verify71;

static FIXTURE_71: &str = include_str!("fixtures/v3d_mesa_programs_71.txt");
static COUNTS_71: &str = include_str!("../tools/v3d/qpu_counts_71.tsv");

/// The programs of the fixture: (pair and stage, words).
fn fixture_programs() -> Vec<(String, Vec<u64>)> {
    let mut out = Vec::new();
    let mut pair = String::new();
    for line in FIXTURE_71.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        if w[0] == "pair" {
            pair = format!("{} + {}", w[1], w[2]);
        } else if matches!(w[0], "fs" | "vs" | "cs") && w.get(1) == Some(&"code") {
            let words = w[2..]
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap());
            out.push((format!("{pair} {}", w[0]), words.collect()));
        }
    }
    out
}

fn decode_all(words: &[u64]) -> Result<Vec<Instr71>, String> {
    words.iter().enumerate().map(|(i, w)| decode71(*w).map_err(|e| format!("word {i} {w:#018x}: {e:?}"))).collect()
}

// ---------------------------------------------------------------------------
// 1. The fixture.
// ---------------------------------------------------------------------------

/// Programs in the fixture: the fs, vs and cs of 242 compiled pairs.
const FIXTURE_PROGRAMS: usize = 726;
/// Of them verify71 accepts.
const FIXTURE_ACCEPTED: usize = 726;
/// `prog_data->single_seg` of a fixture program, which the fixture does not
/// carry: the program starts in its last thread section, so it has one
/// THRSW instruction and no earlier switch (a program with a thread switch
/// before the end has at least two).
fn single_seg_of(p: &[Instr71]) -> bool {
    thrsw_ips(p).len() == 1
}

/// Programs with a word codec71 does not decode (not verifiable here).
const FIXTURE_UNDECODABLE: usize = 0;

#[test]
fn verify71_accepts_every_7_1_program_of_the_mesa_fixture() {
    let progs = fixture_programs();
    let (mut ok, mut undecodable) = (0, 0);
    let mut rejected: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for (name, words) in &progs {
        match decode_all(words) {
            Err(e) => {
                println!("undecodable: {name}: {e}");
                undecodable += 1;
            }
            Ok(p) => match verify71(&p, single_seg_of(&p)) {
                Ok(()) => ok += 1,
                Err(e) => rejected.entry(e.rule).or_default().push(format!("{name} at {}", e.ip)),
            },
        }
    }
    for (rule, who) in &rejected {
        println!("rejected by '{rule}': {} (first: {})", who.len(), who[0]);
    }
    println!("fixture: {} programs, {ok} accepted, {undecodable} undecodable", progs.len());
    assert_eq!(progs.len(), FIXTURE_PROGRAMS, "the fixture's programs moved");
    assert_eq!(undecodable, FIXTURE_UNDECODABLE, "programs codec71 refuses moved");
    assert_eq!(ok, FIXTURE_ACCEPTED, "verify71 disagrees with Mesa's validator on {rejected:?}");
}

// ---------------------------------------------------------------------------
// 2. V3D_SHC: regenerate qpu_counts_71.tsv's programs.
// ---------------------------------------------------------------------------

/// Prints `vs fs stage word...` per program of a glsl2v3d.py `--json`/`--keep`
/// run (the layout tools/v3d/qpu_corpus.py reads).
const DUMP: &str = r#"
import json, os, re, sys
src = sys.argv[1]
js = json.load(open(os.path.join(src, "pairs.json")))
for p in js["pairs"]:
    if p["status"] != "ok":
        continue
    d = os.path.join(src, "keep", re.sub(r"[^\w.]", "_", p["vs"]) + "__" + re.sub(r"[^\w.]", "_", p["fs"]))
    out = json.load(open(os.path.join(d, "out.json")))
    for st in ("fs", "vs", "cs"):
        print(p["vs"], p["fs"], st, "ss" if out[st].get("single_seg") else "ms", *out[st]["code"])
"#;

/// (vs, fs, stage, single_seg, words)
type Generated = Vec<(String, String, String, bool, Vec<u64>)>;

fn regenerate(shc: &str) -> Result<Generated, String> {
    let dir = std::env::temp_dir().join(format!("v3d71_verify_corpus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/v3d/glsl2v3d.py");
    let o = std::process::Command::new("python3")
        .env("V3D_SHC", shc)
        .arg(&script)
        .arg("--json")
        .arg(dir.join("pairs.json"))
        .arg("--keep")
        .arg(dir.join("keep"))
        .args(["--device", "71"])
        .output()
        .map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(format!("glsl2v3d.py: {}", String::from_utf8_lossy(&o.stderr)));
    }
    let o = std::process::Command::new("python3")
        .args([OsString::from("-I"), OsString::from("-c"), OsString::from(DUMP), dir.clone().into_os_string()])
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if !o.status.success() {
        return Err(format!("dump: {}", String::from_utf8_lossy(&o.stderr)));
    }
    Ok(String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            let words = w[4..].iter().map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()).collect();
            (w[0].to_string(), w[1].to_string(), w[2].to_string(), w[3] == "ss", words)
        })
        .collect())
}

/// Pairs in qpu_counts_71.tsv (three programs each).
const COUNTS_PAIRS: usize = 1090;

#[test]
fn verify71_accepts_the_programs_v3d_shc_makes_at_7_1() {
    let table: Vec<Vec<&str>> =
        COUNTS_71.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).map(|l| l.split('\t').collect()).collect();
    assert_eq!(table.len(), COUNTS_PAIRS, "qpu_counts_71.tsv's rows moved");
    let Some(shc) = std::env::var("V3D_SHC").ok().filter(|s| !s.is_empty() && Path::new(s).is_file()) else {
        println!(
            "gap: V3D_SHC (not set, or not a file: the {COUNTS_PAIRS} pairs of qpu_counts_71.tsv are not regenerated)"
        );
        return;
    };
    let progs = regenerate(&shc).expect("v3d_shc at ver 71 through glsl2v3d.py");
    let want: BTreeMap<(&str, &str), [usize; 3]> = table
        .iter()
        .map(|r| ((r[0], r[1]), [r[2].parse().unwrap(), r[3].parse().unwrap(), r[4].parse().unwrap()]))
        .collect();
    let (mut ok, mut bad) = (0, Vec::new());
    for (vs, fs, st, ss, words) in &progs {
        let col = ["fs", "vs", "cs"].iter().position(|s| s == st).unwrap();
        let n = want.get(&(vs.as_str(), fs.as_str())).map(|r| r[col]);
        if n != Some(words.len()) {
            bad.push(format!("{vs} + {fs} {st}: {} instructions, table {n:?}", words.len()));
            continue;
        }
        match decode_all(words).and_then(|p| verify71(&p, *ss).map_err(|e| format!("{} at {}", e.rule, e.ip))) {
            Ok(()) => ok += 1,
            Err(e) => bad.push(format!("{vs} + {fs} {st}: {e}")),
        }
    }
    println!("v3d_shc ver 71: {} programs, {ok} accepted", progs.len());
    assert_eq!(progs.len(), COUNTS_PAIRS * 3);
    assert!(bad.is_empty(), "{} programs differ: {:#?}", bad.len(), &bad[..bad.len().min(20)]);
}

// ---------------------------------------------------------------------------
// 3. Mutations.
// ---------------------------------------------------------------------------

fn accepted() -> Vec<(String, Vec<Instr71>)> {
    fixture_programs()
        .into_iter()
        .filter_map(|(n, w)| decode_all(&w).ok().map(|p| (n, p)))
        .filter(|(_, p)| verify71(p, single_seg_of(p)).is_ok())
        .collect()
}

fn alu(i: &Instr71) -> Option<&Alu71> {
    match i {
        Instr71::Alu(a) => Some(a),
        _ => None,
    }
}

fn alu_mut(i: &mut Instr71) -> &mut Alu71 {
    match i {
        Instr71::Alu(a) => a,
        _ => panic!("not an ALU instruction"),
    }
}

fn has_signal(a: &Alu71) -> bool {
    let s = &a.sig;
    s.thrsw
        || s.ldunif
        || s.ldunifa
        || s.ldunifrf
        || s.ldunifarf
        || s.ldtmu
        || s.ldvary
        || s.ldtlb
        || s.ldtlbu
        || s.wrtmuc
}

/// An ALU instruction with nothing in it: both operations NOP, no signal.
fn is_empty(i: &Instr71) -> bool {
    alu(i).is_some_and(|a| a.add.op == AddOp71::Nop && a.mul.op == MulOp71::Nop && !has_signal(a))
}

fn thrsw_ips(p: &[Instr71]) -> Vec<usize> {
    (0..p.len()).filter(|&i| alu(&p[i]).is_some_and(|a| a.sig.thrsw)).collect()
}

/// A mutation edits a copy of a program; `None` when it does not apply.
type Mutate = fn(&[Instr71]) -> Option<Vec<Instr71>>;

struct Case {
    name: &'static str,
    rule: &'static str,
    /// Programs of the fixture the mutation applies to (all must be rejected).
    applies: usize,
    mutate: Mutate,
}

fn first_branch(p: &[Instr71]) -> Option<usize> {
    p.iter().position(|i| matches!(i, Instr71::Branch(_)))
}

const CASES: &[Case] = &[
    Case {
        name: "the THRSW signals removed",
        rule: "No program-end THRSW found",
        applies: 726,
        mutate: |p| {
            let mut q = p.to_vec();
            let t = thrsw_ips(&q);
            for &i in &t {
                alu_mut(&mut q[i]).sig.thrsw = false;
            }
            (!t.is_empty()).then_some(q)
        },
    },
    Case {
        name: "a THRSW moved one slot up, into the delay slot of the one before",
        rule: "THRSW too close to another THRSW.",
        applies: 250,
        mutate: |p| {
            // the last pair's second THRSW is the one after the first, move
            // the first back one instruction (no gap between them leaves two
            // adjacent ones, so move it two back instead)
            let mut q = p.to_vec();
            let t = thrsw_ips(&q);
            let &last = t.last()?;
            // needs an earlier, separate switch to be too close to
            let first = *t.first()?;
            if first == last || last < 2 {
                return None;
            }
            alu_mut(&mut q[last]).sig.thrsw = false;
            alu_mut(&mut q[first + 2]).sig.thrsw = true;
            Some(q)
        },
    },
    Case {
        name: "a branch in a branch's delay slot",
        rule: "branch in a branch delay slot.",
        applies: 21,
        mutate: |p| {
            let mut q = p.to_vec();
            let b = first_branch(&q)?;
            q[b + 1] = q[b];
            Some(q)
        },
    },
    Case {
        name: "a THRSW in a branch's delay slot",
        rule: "THRSW in a branch delay slot.",
        applies: 21,
        mutate: |p| {
            let mut q = p.to_vec();
            let b = first_branch(&q)?;
            alu(q.get(b + 1)?)?;
            alu_mut(&mut q[b + 1]).sig.thrsw = true;
            Some(q)
        },
    },
    Case {
        name: "rf0 written explicitly right after an LDVARY",
        rule: "rf0 written right after a LDVARY (qpu_schedule.c, ours)",
        applies: 2,
        mutate: |p| {
            let mut q = p.to_vec();
            let v = (0..q.len().saturating_sub(1)).find(|&i| {
                alu(&q[i]).is_some_and(|a| a.sig.ldvary && !a.sig.thrsw)
                    && alu(&q[i + 1]).is_some_and(|a| a.add.op == AddOp71::Nop && !has_signal(a))
            })?;
            let a = alu_mut(&mut q[v + 1]);
            a.add.op = AddOp71::Mov;
            a.add.magic = false;
            a.add.waddr = 0;
            Some(q)
        },
    },
    Case {
        name: "LDUNIF right after an LDVARY",
        rule: "LDUNIF after a LDVARY",
        applies: 63,
        mutate: |p| {
            let mut q = p.to_vec();
            let v = (0..q.len().saturating_sub(1)).find(|&i| {
                alu(&q[i]).is_some_and(|a| a.sig.ldvary) && alu(&q[i + 1]).is_some_and(|a| !has_signal(a))
            })?;
            alu_mut(&mut q[v + 1]).sig.ldunif = true;
            Some(q)
        },
    },
    Case {
        name: "two small immediates in one instruction",
        rule: "only one small immediate can be enabled per instruction",
        applies: 674,
        mutate: |p| {
            let mut q = p.to_vec();
            let i =
                q.iter().position(|i| alu(i).is_some_and(|a| a.add.op != AddOp71::Nop && a.mul.op != MulOp71::Nop))?;
            let a = alu_mut(&mut q[i]);
            a.sig.small_imm_a = true;
            a.sig.small_imm_c = true;
            Some(q)
        },
    },
    Case {
        name: "a write to a reserved magic address",
        rule: "write to a reserved magic waddr",
        applies: 384,
        mutate: |p| {
            let mut q = p.to_vec();
            let i = q.iter().position(is_empty)?;
            let a = alu_mut(&mut q[i]);
            a.add.op = AddOp71::Mov;
            a.add.magic = true;
            a.add.waddr = 1;
            Some(q)
        },
    },
    Case {
        name: "the next branch pulled up into the first one third delay slot",
        rule: "branch in a branch delay slot.",
        applies: 21,
        mutate: |p| {
            let mut q = p.to_vec();
            let b = first_branch(&q)?;
            let n = (b + 1..q.len()).find(|&i| matches!(q[i], Instr71::Branch(_)))?;
            if n <= b + 3 {
                return None;
            }
            // pull the later branch up into the first one's third slot
            q.swap(n, b + 3);
            Some(q)
        },
    },
];

#[test]
fn a_broken_rule_is_rejected_with_its_message() {
    let progs = accepted();
    assert!(!progs.is_empty());
    let mut failures = Vec::new();
    for c in CASES {
        let (mut tried, mut hit) = (0, 0);
        let mut other: BTreeMap<String, usize> = BTreeMap::new();
        for (name, p) in &progs {
            let Some(q) = (c.mutate)(p) else { continue };
            tried += 1;
            match verify71(&q, single_seg_of(p)) {
                Err(e) if e.rule == c.rule => hit += 1,
                Err(e) => *other.entry(format!("{} (e.g. {name})", e.rule)).or_default() += 1,
                Ok(()) => *other.entry(format!("accepted (e.g. {name})")).or_default() += 1,
            }
        }
        println!("mutation '{}': {tried} programs, {hit} rejected as '{}'", c.name, c.rule);
        if tried != c.applies || hit != tried {
            failures.push(format!("{}: {hit} of {tried}, other: {other:?}", c.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
