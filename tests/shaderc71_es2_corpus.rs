// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 (Raspberry Pi 5) online back end over the dEQP-GLES2 shader
//! library: the expect-pass, `version 100` programs of the
//! `data/gles2/shaders/*.test` corpus, the population
//! `tools/cts/es2lib-ref71.sh` measured Mesa 7.1 on (its
//! `tools/shaders/deqp_es2_lib_counts_71.tsv` is printed next to ours, not
//! enforced).
//!
//! The corpus is the CTS text and is never committed:
//! `DEQP_GLES2_TEST_DIR` names the directory of the `.test` files (a
//! VK-GL-CTS 93bca01 checkout's `data/gles2/shaders`,
//! `tools/cts/deps-env.sh`). Without it the test prints that it skipped and
//! returns; a `DEQP_GLES2_TEST_DIR` that holds no `.test` file fails, which
//! is how the gate's own row refuses an empty run.
//!
//! The reader is `tests/deqp_shader_library` (shared with
//! `tests/es3_shaderc_corpus.rs`): `programs()` yields, per case, the
//! programs as dEQP forms them (a `both` case gives the `_vertex` and
//! `_fragment` programs, a case with two sources gives one program). Each is
//! compiled through `chain71::compile_program71`; every word of every stage
//! of a program that compiles decodes with codec71 to its instruction and the
//! instructions pass verify71, `registers_fit_threads71` and
//! `branches_are_structured71`. Refusals are counted, summarised by message
//! and named.
//!
//! Pinned: `PROGRAMS`, `COMPILED` and `REFUSED` exactly, and the refusals
//! against `tools/shaders/known-uncompilable-v3d71-es2.txt`
//! (`<key>: <exact message> # <reason>`): a refusal not listed, a listed
//! refusal whose message changed, and a listed key that now compiles all
//! fail. Mesa 7.1's counts (and the stages it refuses) are printed, not
//! enforced.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_es2_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

#[path = "deqp_shader_library/mod.rs"]
mod deqp_shader_library;

use deqp_shader_library::{programs, Case, Expect};
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

static LIB_COUNTS_71: &str = include_str!("../tools/shaders/deqp_es2_lib_counts_71.tsv");
static LIST: &str = include_str!("../tools/shaders/known-uncompilable-v3d71-es2.txt");

/// The expect-pass, `version 100` programs of VK-GL-CTS 93bca01
/// data/gles2/shaders (es2lib-ref71.sh's `programs: 1778`).
const PROGRAMS: usize = 1778;
/// Those that compile at 7.1 and pass codec71 and verify71 (at 9f5f6e76).
const COMPILED: usize = 1778;
/// Those the 7.1 back end refuses: the entries of the list.
const REFUSED: usize = 0;

/// The start of `l`'s trailing reason: the rightmost `#` with a space before
/// it, a space after it and text after that (tests/shaderc71_lists.rs's
/// rule; a `#` glued to a word, `#version`, stays in the message).
fn trailing_reason(l: &str) -> Option<usize> {
    let mut from = l.len();
    while let Some(j) = l[..from].rfind("# ") {
        if j > 0 && l.as_bytes()[j - 1] == b' ' && !l[j + 2..].trim().is_empty() {
            return Some(j);
        }
        from = j;
    }
    None
}

/// The list: key to the exact refusal message, the trailing reason dropped.
fn listed() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for l in LIST.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let j = trailing_reason(l).unwrap_or_else(|| panic!("known-uncompilable-v3d71-es2.txt: no `# reason`: {l}"));
        let (k, msg) = l[..j]
            .trim_end()
            .split_once(": ")
            .unwrap_or_else(|| panic!("known-uncompilable-v3d71-es2.txt: not `<key>: <message>`: {l}"));
        assert!(m.insert(k.to_string(), msg.to_string()).is_none(), "{k} is listed twice");
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
fn compile(key: &str, vs: &str, fs: &str) -> Result<[usize; 3], String> {
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
        check_stage(&format!("{key} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage(&format!("{key} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage(&format!("{key} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
        Ok([p.fs.code.words.len(), p.vs.code.words.len(), p.cs.code.words.len()])
    })
    .expect("scratch")
}

/// Mesa 7.1's per-stage counts: `<case>\t<stage>` to instructions, when it
/// compiled.
fn mesa() -> BTreeMap<(String, String), usize> {
    let mut m = BTreeMap::new();
    for l in LIB_COUNTS_71.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 7, "deqp_es2_lib_counts_71.tsv row: {l}");
        if c[2] == "yes" {
            m.insert((c[0].to_string(), c[1].to_string()), c[4].parse().expect("instructions"));
        }
    }
    m
}

/// The programs of the expect-pass, `version 100` cases, named as
/// es2lib-ref71.sh names them (`<file stem>.<case>[<suffix>]`).
fn library(dir: &Path) -> Vec<(String, String, String)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("DEQP_GLES2_TEST_DIR={}: {e}", dir.display()))
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "test"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for p in &files {
        let text = std::fs::read_to_string(p).expect("a .test file");
        let stem = p.file_stem().expect("a file stem").to_string_lossy().into_owned();
        for case in deqp_shader_library::parse_file(&text) {
            if case.expect != Expect::Pass || case.glsl300 {
                continue;
            }
            for (suffix, srcs) in programs(&case) {
                let name = format!("{stem}.{}{suffix}", case.name);
                match srcs {
                    Ok((vs, fs)) => out.push((name, vs, fs)),
                    Err(why) => println!("  skipped {name}: {why}"),
                }
            }
        }
    }
    out
}

#[test]
fn every_es2_library_program_compiles_at_7_1_or_is_named() {
    let Some(dir) = std::env::var_os("DEQP_GLES2_TEST_DIR") else {
        println!("shaderc71_es2_corpus: DEQP_GLES2_TEST_DIR is not set; the dEQP-GLES2 library part is skipped");
        return;
    };
    let dir = PathBuf::from(dir);
    let programs = library(&dir);
    assert!(!programs.is_empty(), "DEQP_GLES2_TEST_DIR={} holds no expect-pass version 100 program", dir.display());
    let mesa = mesa();

    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    let mut compiled = 0usize;
    println!("shaderc71 es2 library: program, ours (fs, vs words), Mesa 7.1 (fs, vs instructions)");
    for (name, vs, fs) in &programs {
        match compile(name, vs, fs) {
            Ok(n) => {
                compiled += 1;
                let m = ["fs", "vs"].map(|s| mesa.get(&(name.clone(), s.to_string())).copied());
                println!("  {name}: ours fs {} vs {}, Mesa {:?} {:?}", n[0], n[1], m[0], m[1]);
            }
            Err(e) => {
                refused.insert(name.clone(), e);
            }
        }
    }
    println!(
        "shaderc71 es2 library: {} programs, {compiled} compiled at 7.1, {} refused",
        programs.len(),
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
    /* Mesa 7.1's refusals, printed next to ours, not enforced. */
    for l in LIB_COUNTS_71.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        if c[2] != "yes" {
            let ours = if refused.contains_key(c[0]) { "refused" } else { "compiled" };
            println!("  Mesa 7.1 refuses {} {}: {} (ours: {ours})", c[0], c[1], c[3]);
        }
    }

    /* The list, both ways, then the pins. */
    let list = listed();
    let mut bad = Vec::new();
    for (k, m) in &refused {
        match list.get(k) {
            None => bad.push(format!("refused, not listed: {k}: {m}")),
            Some(want) if want != m => bad.push(format!("listed message changed: {k}: listed `{want}`, now `{m}`")),
            Some(_) => {}
        }
    }
    for k in list.keys().filter(|k| !refused.contains_key(*k)) {
        bad.push(format!("listed but compiles now (or no such program): {k}"));
    }
    assert!(bad.is_empty(), "tools/shaders/known-uncompilable-v3d71-es2.txt is out of date:\n{}", bad.join("\n"));
    assert_eq!(
        (programs.len(), compiled, refused.len()),
        (PROGRAMS, COMPILED, REFUSED),
        "(programs, compiled, refused) at 7.1"
    );
}

/// The reader's own cases (the ES 3.00 side is es3_shaderc_corpus's); this
/// pins that a `version 100` `both` case becomes two programs, as
/// es2lib-ref71.sh counts them.
#[test]
fn a_version_100_both_case_gives_two_programs() {
    let src = r#"
group g "g"
    case both_case
        version 100
        values { input float in0 = 1.0; output float out0 = 1.0; }
        both ""
            precision mediump float;
            ${DECLARATIONS}
            void main()
            {
                ${SETUP}
                out0 = in0;
                ${OUTPUT}
            }
        ""
    end
    case full_case
        version 100
        vertex ""
            ${VERTEX_DECLARATIONS}
            void main()
            {
                ${VERTEX_OUTPUT}
            }
        ""
        fragment ""
            precision mediump float;
            ${FRAGMENT_DECLARATIONS}
            void main()
            {
                ${FRAGMENT_OUTPUT}
            }
        ""
    end
end
"#;
    let cases: Vec<Case> = deqp_shader_library::parse_file(src);
    assert_eq!(cases.len(), 2, "two cases: {src}");
    let names: Vec<String> =
        cases.iter().flat_map(|c| programs(c).into_iter().map(move |(s, _)| format!("{}{s}", c.name))).collect();
    assert_eq!(
        names,
        ["g.both_case_vertex".to_string(), "g.both_case_fragment".to_string(), "g.full_case".to_string()]
    );
}
