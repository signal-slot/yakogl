// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mesa 7.1 reference-count tables (`*_counts_71.tsv` under
//! `tools/shaders` and `tools/v3d`, described in `tools/v3d/README.md`) are
//! well-formed: the `# columns:` line is the documented one, every row has
//! that many fields, numeric fields parse, keys are unique, and a refused row
//! follows the table's marker. The checks are per table, because the
//! generators write different layouts:
//!
//! - `es100_counts_71.tsv`, `deqp_es2_lib_counts_71.tsv`,
//!   `deqp_es2_gen_counts_71.tsv`: key, stage (`vs|fs`), compiled
//!   (`yes|no`), reason, instructions, threads, uniforms. `yes` rows carry
//!   reason `ok` and three numbers; `no` rows a reason other than `ok` and
//!   `-` in the three numbers. Rows are sorted by (key, stage), as the
//!   generators sort them, and the header's `compiled:`/`refused:` counts
//!   equal the rows.
//! - `glmark2_counts_71.tsv`: key (`<vs>+<fs>`), vs/fs instructions, vs/fs
//!   threads, reason. Column layout only (its generator is being changed):
//!   six fields, the four numbers numeric or all `-`, `-` exactly when the
//!   reason is not `ok`, keys unique.
//! - `qpu_counts_71.tsv` and any table not listed: field count equal to its
//!   `# columns:` list, no duplicate rows.
//!
//! The test reads files only; the crate still needs a feature set to build. Run:
//! `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_ref_tables`
//!
//! Malformed rows are reported, never repaired here: the tables and their
//! generators are not this test's to edit.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tables() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for d in ["tools/shaders", "tools/v3d"] {
        let Ok(rd) = fs::read_dir(root().join(d)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("_counts_71.tsv")) {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

struct Table {
    name: String,
    comments: Vec<String>,
    rows: Vec<(usize, Vec<String>)>, // (1-based line, fields)
}

fn load(p: &Path) -> Table {
    let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut t =
        Table { name: p.file_name().unwrap().to_string_lossy().into_owned(), comments: Vec::new(), rows: Vec::new() };
    for (i, l) in text.lines().enumerate() {
        if let Some(c) = l.strip_prefix('#') {
            t.comments.push(c.trim().to_string());
        } else if !l.is_empty() {
            t.rows.push((i + 1, l.split('\t').map(str::to_string).collect()));
        }
    }
    t
}

fn num(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The value of a `# <name>: N` header count inside `line`, e.g.
/// `compiled: 59, refused: 34` -> `count("compiled", ..)`.
fn count(comments: &[String], key: &str) -> Option<usize> {
    let pat = format!("{key}: ");
    // Prefer the "program stages" line when present: it counts the rows.
    let lines = comments
        .iter()
        .filter(|c| c.starts_with("program stages:"))
        .chain(comments.iter().filter(|c| c.starts_with("programs:")));
    for c in lines {
        if let Some(i) = c.find(&pat) {
            let r = &c[i + pat.len()..];
            let n: String = r.chars().take_while(char::is_ascii_digit).collect();
            return n.parse().ok();
        }
    }
    None
}

fn columns(t: &Table) -> Option<&str> {
    t.comments.iter().find_map(|c| c.strip_prefix("columns:").map(str::trim))
}

fn check_stage_table(t: &Table, errs: &mut Vec<String>) {
    let n = &t.name;
    if columns(t) != Some("case, stage, compiled, reason, instructions, threads, uniforms")
        && columns(t) != Some("path, stage, compiled, reason, instructions, threads, uniforms")
        && columns(t) != Some("id, stage, compiled, reason, instructions, threads, uniforms")
    {
        errs.push(format!("{n}: columns line is {:?}", columns(t)));
    }
    let want = match n.as_str() {
        "es100_counts_71.tsv" => "path",
        "deqp_es2_lib_counts_71.tsv" => "case",
        _ => "id",
    };
    if !columns(t).is_some_and(|c| c.starts_with(want)) {
        errs.push(format!("{n}: first column must be `{want}`"));
    }
    let (mut yes, mut no) = (0, 0);
    let mut seen = HashSet::new();
    let mut prev: Option<(String, String)> = None;
    for (ln, f) in &t.rows {
        if f.len() != 7 {
            errs.push(format!("{n}:{ln}: {} fields, want 7", f.len()));
            continue;
        }
        if f[0].is_empty() || f[0].contains(char::is_whitespace) {
            errs.push(format!("{n}:{ln}: bad key {:?}", f[0]));
        }
        if f[1] != "vs" && f[1] != "fs" {
            errs.push(format!("{n}:{ln}: stage {:?}", f[1]));
        }
        match f[2].as_str() {
            "yes" => {
                yes += 1;
                if f[3] != "ok" {
                    errs.push(format!("{n}:{ln}: compiled=yes but reason {:?}", f[3]));
                }
                if !f[4..7].iter().all(|s| num(s)) {
                    errs.push(format!("{n}:{ln}: non-numeric counts {:?}", &f[4..7]));
                }
            }
            "no" => {
                no += 1;
                if f[3].is_empty() || f[3] == "ok" || f[3] == "-" {
                    errs.push(format!("{n}:{ln}: refused row without a reason"));
                }
                if !f[4..7].iter().all(|s| s == "-") {
                    errs.push(format!("{n}:{ln}: refused row with counts {:?}", &f[4..7]));
                }
            }
            other => errs.push(format!("{n}:{ln}: compiled {other:?}")),
        }
        let key = (f[0].clone(), f[1].clone());
        if !seen.insert(key.clone()) {
            errs.push(format!("{n}:{ln}: duplicate key {key:?}"));
        }
        if prev.as_ref().is_some_and(|p| *p > key) {
            errs.push(format!("{n}:{ln}: out of order after {:?}", prev.as_ref().unwrap()));
        }
        prev = Some(key);
    }
    for (k, got) in [("compiled", yes), ("refused", no)] {
        match count(&t.comments, k) {
            Some(h) if h == got => {}
            h => errs.push(format!("{n}: header {k} {h:?}, rows {got}")),
        }
    }
}

fn check_glmark2(t: &Table, errs: &mut Vec<String>) {
    let n = &t.name;
    if columns(t) != Some("key (vs file+fs file), vs instructions, fs instructions, vs threads, fs threads, reason") {
        errs.push(format!("{n}: columns line is {:?}", columns(t)));
    }
    let mut seen = HashSet::new();
    for (ln, f) in &t.rows {
        if f.len() != 6 {
            errs.push(format!("{n}:{ln}: {} fields, want 6", f.len()));
            continue;
        }
        if f[0].matches('+').count() != 1 || f[0].contains(char::is_whitespace) {
            errs.push(format!("{n}:{ln}: key {:?} is not <vs>+<fs>", f[0]));
        }
        let numeric = f[1..5].iter().all(|s| num(s));
        let dashes = f[1..5].iter().all(|s| s == "-");
        if !(numeric || dashes) {
            errs.push(format!("{n}:{ln}: numbers {:?}", &f[1..5]));
        }
        if f[5].is_empty() || (f[5] == "ok") != numeric {
            errs.push(format!("{n}:{ln}: reason {:?} does not match the numbers", f[5]));
        }
        if !seen.insert(f[0].clone()) {
            errs.push(format!("{n}:{ln}: duplicate key {:?}", f[0]));
        }
    }
}

fn check_generic(t: &Table, errs: &mut Vec<String>) {
    let n = &t.name;
    let Some(cols) = columns(t) else {
        errs.push(format!("{n}: no `# columns:` line"));
        return;
    };
    let want = cols.split(", ").count();
    let mut seen = HashSet::new();
    for (ln, f) in &t.rows {
        if f.len() != want {
            errs.push(format!("{n}:{ln}: {} fields, columns line has {want}", f.len()));
        }
        if !seen.insert(f.join("\t")) {
            errs.push(format!("{n}:{ln}: duplicate row"));
        }
    }
}

#[test]
fn ref_tables_are_well_formed() {
    let all = tables();
    let names: Vec<_> = all.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    let mut errs = Vec::new();
    // The four tables of tools/v3d/README.md must exist.
    for must in
        ["es100_counts_71.tsv", "glmark2_counts_71.tsv", "deqp_es2_lib_counts_71.tsv", "deqp_es2_gen_counts_71.tsv"]
    {
        if !names.iter().any(|n| n == must) {
            errs.push(format!("{must}: missing"));
        }
    }
    for p in &all {
        let t = load(p);
        if t.rows.is_empty() {
            errs.push(format!("{}: no rows", t.name));
            continue;
        }
        match t.name.as_str() {
            "es100_counts_71.tsv" | "deqp_es2_lib_counts_71.tsv" | "deqp_es2_gen_counts_71.tsv" => {
                check_stage_table(&t, &mut errs)
            }
            "glmark2_counts_71.tsv" => check_glmark2(&t, &mut errs),
            _ => check_generic(&t, &mut errs),
        }
    }
    assert!(errs.is_empty(), "malformed rows:\n{}", errs.join("\n"));
}
