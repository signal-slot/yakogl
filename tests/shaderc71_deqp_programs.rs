// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 (Raspberry Pi 5) online back end over the harvested
//! dEQP-GLES2 programs: every program of `tools/cts/deqp-es2-programs.sh`'s
//! harvest (the C++-generated dEQP-GLES2 shaders — shaders.random, operator,
//! struct, loops, ... — which the `data/gles2/shaders` library does not hold).
//!
//! The harvest is dEQP's output and is never committed:
//! `DEQP_ES2_PROGRAMS` names the harvest's OUT directory (the one holding
//! `index.tsv` and `programs/`, `tools/cts/deqp-es2-programs.sh --out`).
//! Without it the test prints that it skipped and returns; a
//! `DEQP_ES2_PROGRAMS` that holds no `index.tsv` fails, which is how the
//! gate's own row refuses an empty run.
//!
//! Each row of `index.tsv` (id, first case, case count, the 4.2 outcome, the
//! first refusal line) is one program: its `programs/<id>.vert` and
//! `programs/<id>.frag` are compiled through `chain71::compile_program71`;
//! every word of every stage of a program that compiles decodes with codec71
//! to its instruction and the instructions pass verify71,
//! `registers_fit_threads71` and `branches_are_structured71`
//! (tests/v3d71_online_corpus.rs's `check_stage`).
//!
//! Refusals are counted, summarised by message and named. A program the 4.2
//! back end compiled and the 7.1 one refuses is printed on its own line:
//! those are the 7.1-only refusals this corpus exists to find. Mesa's 7.1
//! per-stage counts (`tools/shaders/deqp_es2_gen_counts_71.tsv`, made by
//! tools/cts/deqp-es2-programs-ref71.sh) are printed next to ours for the
//! programs both compile. Printed, not enforced; the count that compiles is
//! the pin's input, re-derived with the corpus.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_deqp_programs -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::sched71_cf::branches_are_structured71;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

static GEN_COUNTS_71: &str = include_str!("../tools/shaders/deqp_es2_gen_counts_71.tsv");
static KNOWN_71: &str = include_str!("../tools/shaders/known-uncompilable-v3d71-deqp-gen.txt");

/// The pins, from the harvest of CTS 93bca01 at tip da11eb6b
/// (`tools/cts/deqp-es2-programs.sh`): the programs, those the 4.2 back end
/// compiled (index.tsv), those the 7.1 back end compiled, and those it
/// refused. A harvest of another CTS commit moves them: re-derive and change
/// them with the corpus.
const PROGRAMS: usize = 10599;
const COMPILED_42: usize = 9723;
const COMPILED_71: usize = 9720;
const REFUSED_71: usize = 879;
/// Programs 4.2 refused and 7.1 compiles. Each 7.1-only refusal (4.2
/// compiled, 7.1 refuses) is in `known-uncompilable-v3d71-deqp-gen.txt`.
const COMPILED_71_ONLY: usize = 3;

/// The list's entries: `<program id>: <message>`; `#` lines are reasons.
fn listed() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for l in KNOWN_71.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (k, v) = l.split_once(": ").unwrap_or_else(|| panic!("a list line without `: `: {l}"));
        assert!(m.insert(k.to_string(), v.to_string()).is_none(), "{k} listed twice");
    }
    m
}

/// The scratch region is one per process: the tests take turns
/// (tests/v3d71_online_corpus.rs's idiom).
static SCRATCH: Mutex<()> = Mutex::new(());

/// One stage's words decode to its instructions, which pass verify71, fit
/// its thread count and have structured branches.
fn check_stage(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
    assert_eq!(branches_are_structured71(c.instrs, c.back_edges, c.fwd_branches), Ok(()), "{what}: branches");
}

/// Compile `vs` + `fs` as one program at 7.1 and check its words: the word
/// counts (fs, vs, cs), or the refusal's message (the front end's, or
/// `V3d71Error`'s Debug form).
fn compile(id: &str, vs: &str, fs: &str) -> Result<[usize; 3], String> {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs), (Stage::Fragment, fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => return Err(format!("front end {stage:?}: {}", e.msg)),
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let p = compile_program71(v, f, Prim::Triangles, TARGET71, a).map_err(|e| format!("{e:?}"))?;
        check_stage(&format!("{id} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage(&format!("{id} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage(&format!("{id} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
        Ok([p.fs.code.words.len(), p.vs.code.words.len(), p.cs.code.words.len()])
    })
    .expect("scratch")
}

/// Mesa 7.1's per-stage counts: `<id>\t<stage>` to instructions, when it
/// compiled.
fn mesa() -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for l in GEN_COUNTS_71.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 7, "deqp_es2_gen_counts_71.tsv row: {l}");
        if c[2] == "yes" {
            m.insert(format!("{}\t{}", c[0], c[1]), c[4].parse().expect("instructions"));
        }
    }
    m
}

#[test]
fn every_harvested_deqp_program_compiles_at_7_1_or_is_named() {
    let Some(dir) = std::env::var_os("DEQP_ES2_PROGRAMS") else {
        println!("shaderc71_deqp_programs: DEQP_ES2_PROGRAMS is not set; the harvested programs part is skipped");
        return;
    };
    let dir = PathBuf::from(dir);
    let index = std::fs::read_to_string(dir.join("index.tsv")).unwrap_or_else(|e| {
        panic!("DEQP_ES2_PROGRAMS={} holds no index.tsv: run tools/cts/deqp-es2-programs.sh first ({e})", dir.display())
    });

    /* Mesa's row per program and stage, and the harvest's 4.2 outcome */
    let mesa = mesa();
    let mut rows = Vec::new();
    let mut n_42 = 0usize;
    for l in index.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 5, "index.tsv row: {l}");
        if c[0] == "id" {
            continue; // the header
        }
        n_42 += (c[3] == "compiled") as usize;
        rows.push((c[0].to_string(), c[1].to_string(), c[3] == "compiled"));
    }
    assert!(!rows.is_empty(), "DEQP_ES2_PROGRAMS={} holds no program rows", dir.display());

    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    let mut only_71: BTreeMap<String, String> = BTreeMap::new();
    let mut ids: BTreeMap<String, String> = BTreeMap::new(); // program id to refusal
    let mut gained = 0usize; // 4.2 refused, 7.1 compiled
    let mut compiled = 0usize;
    let mut both = 0usize;
    let (mut ours, mut mesa_n) = ([0usize; 2], [0usize; 2]);
    let read = |name: &str| std::fs::read_to_string(dir.join("programs").join(name)).expect("a harvested source");
    for (id, case, c42) in &rows {
        let vs = read(&format!("{id}.vert"));
        let fs = read(&format!("{id}.frag"));
        match compile(id, &vs, &fs) {
            Ok(n) => {
                compiled += 1;
                gained += !*c42 as usize;
                /* Mesa's rows for this program's two stages, when it has them */
                let m = [mesa.get(&format!("{id}\tfs")).copied(), mesa.get(&format!("{id}\tvs")).copied()];
                if let (Some(m0), Some(m1)) = (m[0], m[1]) {
                    ours[0] += n[0];
                    ours[1] += n[1];
                    mesa_n[0] += m0;
                    mesa_n[1] += m1;
                    both += 1;
                }
            }
            Err(e) => {
                let key = format!("{id} {case}");
                ids.insert(id.clone(), e.clone());
                if *c42 {
                    only_71.insert(key.clone(), e.clone());
                }
                refused.insert(key, e);
            }
        }
    }
    println!(
        "shaderc71 deqp programs: {} programs ({} compiled at 4.2), {compiled} compiled at 7.1, {} refused; \
         on the {both} Mesa 7.1 compiled too (fs, vs): ours {ours:?} words, Mesa {mesa_n:?} instructions",
        rows.len(),
        n_42,
        refused.len()
    );
    /* 7.1-only refusals: the 4.2 back end built them */
    for (k, m) in &only_71 {
        println!("  7.1-only refusal {k}: {m}");
    }
    /* refusals by message, most frequent first (tests/v3d71_online_corpus.rs's summary) */
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for m in refused.values() {
        *by.entry(m.as_str()).or_default() += 1;
    }
    let mut by: Vec<(&str, usize)> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (m, n) in &by {
        println!("  {n:>4} x {m}");
    }

    /* the pins */
    assert_eq!(rows.len(), PROGRAMS, "programs");
    assert_eq!(n_42, COMPILED_42, "compiled at 4.2");
    assert_eq!(compiled, COMPILED_71, "compiled at 7.1");
    assert_eq!(refused.len(), REFUSED_71, "refused at 7.1");
    assert_eq!(gained, COMPILED_71_ONLY, "compiled at 7.1 only");

    /* every program 4.2 compiles compiles at 7.1 or is listed with its exact
     * message; a listed program that compiles now (or refuses with another
     * message, or 4.2 refuses too) fails */
    let known = listed();
    for (k, m) in &only_71 {
        let id = k.split(' ').next().unwrap();
        assert_eq!(known.get(id), Some(m), "7.1-only refusal {k} is not listed with this message");
    }
    for (id, m) in &known {
        assert_eq!(ids.get(id), Some(m), "{id} is listed but no longer refuses with that message");
        let p = format!("{id} ");
        assert!(only_71.keys().any(|k| k.starts_with(&p)), "{id} is listed but 4.2 refuses it too");
    }
}
