// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 (Raspberry Pi 5) online back end over the glmark2-es2 corpus:
//! the 7.1 twin of `tests/glmark2_corpus.rs`, which runs it through
//! `chain71::compile_program71`.
//!
//! The corpus is GPL-3.0 text and is never committed: `tools/hostharness/glmark2/run.sh
//! "$GLMARK2_SRC"` generates it into `build/glmark2-host/shaders/` (or
//! `GLMARK2_CORPUS` names another directory holding `index.txt`). With no
//! corpus at the default place the test prints a SKIP line and returns: the
//! plain test rows of the gate run before the corpus exists. A directory
//! `GLMARK2_CORPUS` names must hold the corpus, or the test fails: that is
//! how the gate's own row refuses an empty run.
//!
//! Every vertex/fragment pair of one index entry is compiled as one program
//! at 7.1 (the link step of a chain71 build is part of the compile, so a pair
//! that compiles has linked). A shader must compile with at least one partner
//! of every entry it appears in, or it is named by its own file. Every pair
//! that compiles is checked: each word of its fragment, vertex and coordinate
//! shader decodes with codec71 to its instruction (`same_instr71`), and the
//! instructions pass verify71, `registers_fit_threads71` and
//! `branches_are_structured71` (tests/v3d71_online_corpus.rs's `check_stage`).
//!
//! Mesa's 7.1 counts per pair (`tools/shaders/glmark2_counts_71.tsv`, made by
//! tools/hostharness/glmark2/ref71.sh) are printed next to ours for every
//! pair both compile. Printed, not enforced.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_glmark2_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::sched71_cf::branches_are_structured71;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

/// index.txt's shaders (tests/glmark2_corpus.rs's pin of the same corpus).
const SHADERS: usize = 69;

static COUNTS_71: &str = include_str!("../tools/shaders/glmark2_counts_71.tsv");

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

/// Compile `vs` + `fs` as one program at 7.1 and check its words. `names`
/// are the two sources' names for the front end's message. The word counts
/// (fs, vs, cs), or the refusal's message (the front end's, or
/// `V3d71Error`'s Debug form).
fn compile(key: &str, names: [&str; 2], vs: &str, fs: &str) -> Result<[usize; 3], String> {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, name, src) in [(Stage::Vertex, names[0], vs), (Stage::Fragment, names[1], fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => return Err(format!("front end {name}: {}", e.msg)),
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let p = compile_program71(v, f, Prim::Triangles, TARGET71, a).map_err(|e| format!("{e:?}"))?;
        check_stage(&format!("{key} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage(&format!("{key} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage(&format!("{key} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
        Ok([p.fs.code.words.len(), p.vs.code.words.len(), p.cs.code.words.len()])
    })
    .expect("scratch")
}

/// Mesa 7.1's row per pair key: `(vs, fs)` instructions, or its reason.
fn mesa() -> BTreeMap<String, Result<[usize; 2], String>> {
    let mut m = BTreeMap::new();
    for l in COUNTS_71.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 6, "glmark2_counts_71.tsv row: {l}");
        let r = if c[1] == "-" {
            Err(c[5].to_string())
        } else {
            Ok([c[1].parse().expect("vs instructions"), c[2].parse().expect("fs instructions")])
        };
        assert!(m.insert(c[0].to_string(), r).is_none(), "{} is listed twice", c[0]);
    }
    m
}

#[test]
fn every_glmark2_shader_compiles_and_links_at_7_1() {
    let named = std::env::var_os("GLMARK2_CORPUS");
    let dir = match &named {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("build/glmark2-host/shaders"),
    };
    let Ok(index) = std::fs::read_to_string(dir.join("index.txt")) else {
        assert!(
            named.is_none(),
            "GLMARK2_CORPUS={} has no index.txt: run tools/hostharness/glmark2/run.sh \"$GLMARK2_SRC\" first",
            dir.display()
        );
        eprintln!(
            "SKIP shaderc71_glmark2_corpus: no corpus at {} (it is never committed; set GLMARK2_SRC and run tools/hostharness/glmark2/run.sh)",
            dir.display()
        );
        return;
    };

    /* the files, and per entry its vertex and fragment files
     * (tests/glmark2_corpus.rs's reading of the index) */
    let mut files = Vec::new();
    let mut by_entry: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for l in index.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (f, entries) = l.split_once(": ").expect("`<file>: <entries>`");
        files.push(f.to_string());
        for e in entries.split("; ") {
            let slot = by_entry.entry(e.to_string()).or_default();
            if f.contains(".vert.") {
                slot.0.push(f.to_string());
            } else {
                slot.1.push(f.to_string());
            }
        }
    }
    assert_eq!(files.len(), SHADERS, "index.txt lists {SHADERS} shaders; re-derive the pin if it changed");
    let mesa = mesa();

    let read = |n: &str| std::fs::read_to_string(dir.join(n)).expect("a shader file");
    /* One program per pair: it compiles, or it is a named refusal. A shader
     * fails when no pair of it compiles. */
    let mut tried: BTreeMap<(String, String), Result<[usize; 3], String>> = BTreeMap::new();
    let mut failed: BTreeMap<String, String> = BTreeMap::new();
    let mut linked = 0usize;
    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    println!("shaderc71 glmark2: pair, ours (fs, vs, cs words), Mesa 7.1 (vs, fs instructions)");
    for (entry, (vss, fss)) in &by_entry {
        let mut vs_ok: BTreeMap<&String, bool> = vss.iter().map(|v| (v, false)).collect();
        let mut fs_ok: BTreeMap<&String, bool> = fss.iter().map(|f| (f, false)).collect();
        for v in vss {
            for f in fss {
                let key = (v.clone(), f.clone());
                let r = tried
                    .entry(key.clone())
                    .or_insert_with(|| compile(&format!("{v}+{f}"), [v, f], &read(v), &read(f)));
                let pair_key = format!("{}+{}", key.0, key.1);
                match r {
                    Ok(n) => {
                        vs_ok.insert(v, true);
                        fs_ok.insert(f, true);
                        linked += 1;
                        let m = match mesa.get(pair_key.as_str()) {
                            Some(Ok([mvs, mfs])) => format!("Mesa {mvs} {mfs}"),
                            Some(Err(_)) => "Mesa refused".to_string(),
                            None => "no Mesa row".to_string(),
                        };
                        println!("  {pair_key}: ours fs {} vs {} cs {}, {}", n[0], n[1], n[2], m);
                    }
                    Err(e) => {
                        refused.insert(pair_key, e.clone());
                    }
                }
            }
        }
        for (n, ok) in vs_ok.iter().chain(fs_ok.iter()) {
            if !ok {
                failed.entry(n.to_string()).or_insert_with(|| format!("compiles and links with nothing in `{entry}`"));
            }
        }
    }
    println!(
        "shaderc71 glmark2: {} shaders, {} entries, {} pairs tried, {} compiled and linked at 7.1, {} refused",
        files.len(),
        by_entry.len(),
        tried.len(),
        linked,
        refused.len()
    );
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
    for (k, m) in &refused {
        println!("  refused {k}: {m}");
    }
    assert!(failed.is_empty(), "v3d71: glmark2 shaders that do not compile and link at 7.1: {failed:#?}");
}
