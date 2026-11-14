// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL source and stage of every program of `tools/v3d/qpu_corpus_71.txt`
//! (`tools/v3d/qpu_corpus_71_sources.tsv`, written by
//! `tools/v3d/qpu_corpus_71_sources.py`).
//!
//! Host only. Run with
//! `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_corpus_sources`.
//!
//! The programs are the vertex/fragment pairs named by `qpu_counts_71.tsv`
//! (the corpus file holds only words); the corpus header's `# pairs:` line
//! gives their number. The table has three rows per program (fs, vs, cs; the
//! cs is Mesa's coordinate shader of the vertex shader, so it names the vs
//! source). The 4.2 compile status is not recorded: it can move.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Programs in qpu_corpus_71.txt ("# pairs: 1090 ok of 1116 formed").
const PROGRAMS: usize = 1090;
/// Three stages per program.
const ROWS: usize = 3 * PROGRAMS;

fn read(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn table() -> Vec<(String, String, String)> {
    let text = read("tools/v3d/qpu_corpus_71_sources.tsv");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("program\tstage\tsource"), "header line");
    lines
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            assert_eq!(c.len(), 3, "row {l:?}");
            (c[0].to_string(), c[1].to_string(), c[2].to_string())
        })
        .collect()
}

#[test]
fn corpus_header_pins_the_program_count() {
    let corpus = read("tools/v3d/qpu_corpus_71.txt");
    let line = corpus.lines().find(|l| l.starts_with("# pairs: ")).expect("# pairs: header");
    let n: usize = line["# pairs: ".len()..].split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(n, PROGRAMS, "{line}");
}

#[test]
fn covers_every_program_exactly_once() {
    let rows = table();
    assert_eq!(rows.len(), ROWS);
    let mut sorted = rows.clone();
    sorted.sort();
    assert_eq!(rows, sorted, "rows are sorted");

    // The programs of the corpus: one per row of qpu_counts_71.tsv.
    let mut want: BTreeMap<String, (String, String)> = BTreeMap::new();
    for l in read("tools/v3d/qpu_counts_71.tsv").lines() {
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        let c: Vec<&str> = l.split('\t').collect();
        let key = format!("{}+{}", c[0], c[1]);
        assert!(want.insert(key.clone(), (c[0].into(), c[1].into())).is_none(), "{key} twice in qpu_counts_71.tsv");
    }
    assert_eq!(want.len(), PROGRAMS);

    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut progs: BTreeSet<&str> = BTreeSet::new();
    for (prog, stage, src) in &rows {
        assert!(seen.insert((prog.clone(), stage.clone())), "{prog} {stage} twice");
        let (vs, fs) = want.get(prog).unwrap_or_else(|| panic!("{prog}: not a program"));
        let expect = match stage.as_str() {
            "fs" => fs,
            "vs" | "cs" => vs,
            s => panic!("{prog}: stage {s:?}"),
        };
        assert_eq!(src, expect, "{prog} {stage}");
        progs.insert(prog);
    }
    assert_eq!(progs.len(), PROGRAMS);
    for p in want.keys() {
        for st in ["fs", "vs", "cs"] {
            assert!(seen.contains(&(p.clone(), st.to_string())), "{p} has no {st} row");
        }
    }
}

#[test]
fn every_source_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let srcs: BTreeSet<String> = table().into_iter().map(|r| r.2).collect();
    assert!(!srcs.is_empty());
    for s in &srcs {
        assert!(root.join(s).is_file(), "tools/shaders/{s} does not exist");
    }
}
