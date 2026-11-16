// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `tools/v3d/qpu_corpus_71_sources.tsv`: every program of
//! `tools/v3d/qpu_corpus_71.txt` exactly once, each stage with the GLSL
//! source it was compiled from (the mapping `corpus71` recompiles the 7.1
//! corpus from).
//!
//! The corpus holds words, not programs. Its header names the programs
//! (`# pairs: N ok of M formed`), and the `qpu_corpus.py --device 71` run
//! that wrote it also wrote `tools/v3d/qpu_counts_71.tsv`, which lists them
//! by their `tools/shaders` paths. The table, written by
//! `tools/v3d/qpu_corpus_71_sources.py`, unfolds each pair into its fs, vs
//! and cs programs (the cs, the coordinate shader, from the same source as
//! the vs) under the program key `vs + fs`. Checked here, with no Mesa
//! build and no `v3d_shc`:
//!
//! * the table keys exactly `qpu_counts_71.tsv`'s programs -- so it covers
//!   every program of the corpus once, with one row per stage -- and its
//!   program count is the corpus header's;
//! * every row's source is a file under `tools/shaders/`, a `.frag` for the
//!   fs and a `.vert` for the vs and the cs, and it is the side of the key
//!   its stage is compiled from;
//! * the table is sorted (program, then fs, vs, cs), as generated;
//! * the counts are pinned: `PROGRAMS` pairs, `ROWS` rows, `SOURCES`
//!   distinct sources. A moved number is a decision, not a drift.
//!
//! It records no 4.2 compile status: `corpus71` measures what our back end
//! compiles at 7.1, and that moves with it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_corpus_sources -- --nocapture
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", feature = "no-shader-table"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

static TABLE: &str = include_str!("../tools/v3d/qpu_corpus_71_sources.tsv");
static CORPUS_71: &str = include_str!("../tools/v3d/qpu_corpus_71.txt");
static COUNTS_71: &str = include_str!("../tools/v3d/qpu_counts_71.tsv");

/// Programs of qpu_corpus_71.txt: its `# pairs: N ok`, named by
/// qpu_counts_71.tsv. A pair is the unit (glsl2v3d.py's rule): a program's
/// fs is compiled first, its vs keyed by the fs's inputs, its cs from the
/// vs's source for binning.
const PROGRAMS: usize = 1090;
/// Rows of the sources table: fs, vs and cs per program.
const ROWS: usize = 3270;
/// Distinct sources under tools/shaders, 98 vertex and 210 fragment.
const SOURCES: usize = 308;
/// Of them, the fragment shaders (the rest are vertex shaders).
const FRAGMENT_SOURCES: usize = 210;

/// The stages, in the order v3d_shc compiles them.
const STAGES: [&str; 3] = ["fs", "vs", "cs"];

fn stage_of(stage: &str) -> usize {
    STAGES.iter().position(|s| *s == stage).unwrap_or_else(|| panic!("row stage is not fs/vs/cs: {stage:?}"))
}

/// Data lines of a `#`-commented table, split on tabs.
fn data_lines(text: &str) -> Vec<Vec<&str>> {
    text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).map(|l| l.split('\t').collect()).collect()
}

#[test]
fn sources71_covers_every_corpus_program_once() {
    // 1. The corpus header names PROGRAMS programs.
    let pairs: Vec<usize> = CORPUS_71
        .lines()
        .filter_map(|l| l.strip_prefix("# pairs: ").and_then(|r| r.split(' ').next()))
        .filter_map(|n| n.parse().ok())
        .collect();
    assert_eq!(pairs, [PROGRAMS], "qpu_corpus_71.txt's `# pairs:` moved (or vanished): {pairs:?}");

    // 2. qpu_counts_71.tsv -- the run that wrote the corpus -- names exactly them.
    let counts = data_lines(COUNTS_71);
    assert_eq!(counts.len(), PROGRAMS, "qpu_counts_71.tsv's rows moved");
    let mut counts_pairs: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (n, c) in counts.iter().enumerate() {
        assert_eq!(c.len(), 5, "qpu_counts_71.tsv row {n} is not five tab-separated fields: {c:?}");
        assert!(counts_pairs.insert((c[0], c[1])), "qpu_counts_71.tsv lists {} + {} twice", c[0], c[1]);
    }
    assert_eq!(counts_pairs.len(), PROGRAMS, "qpu_counts_71.tsv names fewer than {PROGRAMS} pairs");

    // 3. The table covers each program once, one row per stage, right source, sorted.
    let rows = data_lines(TABLE);
    assert_eq!(rows.len(), ROWS, "qpu_corpus_71_sources.tsv's rows moved");
    let mut by_program: BTreeMap<&str, [Option<&str>; 3]> = BTreeMap::new();
    let mut prev: Option<(&str, usize)> = None;
    for (n, r) in rows.iter().enumerate() {
        assert_eq!(r.len(), 3, "row {n} is not `program\\tstage\\tsource`: {r:?}");
        let (key, stage, src) = (r[0], r[1], r[2]);
        let si = stage_of(stage);
        assert!(
            prev.is_none_or(|p| (key, si) >= p),
            "rows are not sorted (program, then fs, vs, cs): row {n} is {key} {stage}"
        );
        prev = Some((key, si));
        let want = if stage == "fs" { ".frag" } else { ".vert" };
        assert!(src.ends_with(want), "row {n}: the {stage} of {key} comes from {src}, not a {want} source");
        assert!(
            by_program.entry(key).or_insert([None; 3])[si].replace(src).is_none(),
            "row {n}: {key} has two {stage} rows"
        );
    }
    assert_eq!(by_program.len(), PROGRAMS, "the table covers {} programs, want {PROGRAMS}", by_program.len());

    let mut table_pairs: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (key, stages) in &by_program {
        let (vs, fs) = key.split_once(" + ").unwrap_or_else(|| panic!("program key is not `vs + fs`: {key:?}"));
        let vs_src = stages[stage_of("vs")].unwrap_or_else(|| panic!("{key}: no vs row"));
        let fs_src = stages[stage_of("fs")].unwrap_or_else(|| panic!("{key}: no fs row"));
        let cs_src = stages[stage_of("cs")].unwrap_or_else(|| panic!("{key}: no cs row"));
        assert_eq!(vs_src, vs, "{key}: the vs comes from {vs_src}, not the key's {vs}");
        assert_eq!(fs_src, fs, "{key}: the fs comes from {fs_src}, not the key's {fs}");
        assert_eq!(cs_src, vs_src, "{key}: the cs comes from {cs_src}, not from the vertex source {vs_src}");
        table_pairs.insert((vs, fs));
    }
    assert_eq!(table_pairs, counts_pairs, "the table's programs are not qpu_counts_71.tsv's");

    // 4. Every source is a file under tools/shaders, and the counts hold.
    let srcs: BTreeSet<&str> = rows.iter().map(|r| r[2]).collect();
    assert_eq!(srcs.len(), SOURCES, "the table's distinct sources moved");
    let n_frag = srcs.iter().filter(|s| s.ends_with(".frag")).count();
    assert_eq!(n_frag, FRAGMENT_SOURCES, "the table's fragment sources moved");
    let shaders = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let missing: Vec<&str> = srcs.iter().copied().filter(|s| !shaders.join(s).is_file()).collect();
    assert!(missing.is_empty(), "not files under tools/shaders: {missing:?}");

    println!(
        "srctab: {} programs, {} rows, {} sources ({} vertex, {} fragment)",
        by_program.len(),
        rows.len(),
        srcs.len(),
        srcs.len() - n_frag,
        n_frag
    );
}
