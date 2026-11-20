// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 (Raspberry Pi 5) online back end over the corpus the 4.2 one
//! is gated on, `tests/v3d_online_corpus.rs`'s 7.1 twin: every program goes
//! through `chain71::compile_program71`, and the count that compiles is
//! pinned.
//!
//! Two program sets:
//!
//! * **es100**: every `.vert`/`.frag` of `tools/shaders/es100` (the es100
//!   part of the file set `tests/v3d_online_corpus.rs` walks), each paired
//!   with the stub of the other stage `tools/v3d/es100_ref71.sh` gives Mesa
//!   (a stub VS writes every varying the FS declares from a uniform vec4, a
//!   stub FS adds every varying the VS declares into gl_FragColor), so that
//!   each row is the program `tools/v3d/es100_counts_71.tsv` measured. Key:
//!   the source's path.
//! * **qpu_corpus_71**: every program of `tools/v3d/qpu_corpus_71.txt`, the
//!   `vs + fs` pairs whose GLSL sources `tools/v3d/qpu_corpus_71_sources.tsv`
//!   names. Key: `<vertex> + <fragment>`, as that table writes it.
//!
//! A program that compiles is checked: every word of its fragment, vertex
//! and coordinate shader decodes with codec71 to its instruction, and the
//! instructions pass verify71 and `registers_fit_threads71` (build71 already
//! ran both and `branches_are_structured71`; this guards the returned code).
//!
//! Every refusal is in `tools/shaders/known-uncompilable-v3d71.txt`, one
//! `<key>: <exact message>` line each (the format of the 4.2 list,
//! `known-uncompilable-v3d.txt`), with the reason in `#` lines above it. The
//! message is the front end's (`front end <source>: <message>`) or
//! `V3d71Error`'s Debug form. The test fails on a refusal that is not listed,
//! on a listed one whose message changed, and on a listed one that now
//! compiles, so the list stays exact.
//!
//! Mesa's instruction counts are in the tree (es100_counts_71.tsv per
//! source, tools/v3d/qpu_counts_71.tsv per pair): for every program both
//! compile, ours and Mesa's are printed with their totals. Printed, not
//! enforced.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_online_corpus -- --nocapture
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

/// tools/shaders/es100's sources (es100_counts_71.tsv's rows).
const ES100_PROGRAMS: usize = 126;
/// Of them, the programs our 7.1 back end compiles.
const ES100_COMPILED: usize = 126;
/// qpu_corpus_71.txt's programs (`# pairs: 1090 ok`).
const CORPUS_PROGRAMS: usize = 1090;
/// Of them, the programs our 7.1 back end compiles.
const CORPUS_COMPILED: usize = 874;

static SOURCES_71: &str = include_str!("../tools/v3d/qpu_corpus_71_sources.tsv");
static COUNTS_71: &str = include_str!("../tools/v3d/qpu_counts_71.tsv");
static ES100_71: &str = include_str!("../tools/v3d/es100_counts_71.tsv");

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("tools/shaders/{rel}: {e}"))
}

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

/// The instruction counts (fs, vs, cs) of a program that compiled and
/// whether a stage of it has a branch, or the refusal's message.
type Outcome = Result<([usize; 3], bool), String>;

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

/// Compile `vs` + `fs` at 7.1. `names` are the two sources' names for the
/// front end's message.
fn compile(key: &str, names: [&str; 2], vs: &str, fs: &str) -> Outcome {
    /* the scratch region is one per process: the tests take turns */
    static SCRATCH: Mutex<()> = Mutex::new(());
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
        let branch =
            [&p.fs.code, &p.vs.code, &p.cs.code].iter().any(|c| !c.back_edges.is_empty() || !c.fwd_branches.is_empty());
        Ok(([p.fs.code.words.len(), p.vs.code.words.len(), p.cs.code.words.len()], branch))
    })
    .expect("scratch")
}

// ---------------------------------------------------------------------------
// es100_ref71.sh's stubs, line for line.
// ---------------------------------------------------------------------------

/// The text without `//` and `/* */` comments, each a space
/// (tools/vivante/es100.py `strip_comments`, which es100_ref71.sh uses).
fn strip_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            out.push(' ');
        } else if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(' ');
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

const TYPES: [&str; 7] = ["float", "vec2", "vec3", "vec4", "mat2", "mat3", "mat4"];

/// `(type, name, array size or 0)` of the source's varying declarations
/// (es100_ref71.sh `varyings`).
fn varyings(text: &str) -> Vec<(String, String, usize)> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let t = strip_comments(text);
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = t[from..].find("varying").map(|k| k + from) {
        from = k + 7;
        if (k > 0 && word(b[k - 1])) || b.get(k + 7).is_some_and(|&c| word(c)) {
            continue;
        }
        let Some(end) = t[k + 7..].find(';').map(|e| e + k + 7) else { break };
        let mut toks = t[k + 7..end].split_whitespace().peekable();
        if toks.peek().is_some_and(|p| ["highp", "mediump", "lowp"].contains(p)) {
            toks.next();
        }
        let Some(ty) = toks.next().filter(|ty| TYPES.contains(ty)) else { continue };
        let decls: String = toks.collect::<Vec<_>>().join(" ");
        for d in decls.split(',') {
            let d = d.trim();
            let (name, n) = match d.split_once('[') {
                Some((n, rest)) => (n.trim(), rest.trim_end_matches(']').trim().parse().expect("a varying array size")),
                None => (d, 0),
            };
            assert!(!name.is_empty() && name.bytes().all(word), "cannot read varying `{d}`");
            out.push((ty.to_string(), name.to_string(), n));
        }
        from = end;
    }
    out
}

/// An expression of type `ty` from the uniform vec4 `src`.
fn value(ty: &str, src: &str) -> String {
    match ty {
        "float" => format!("{src}.x"),
        "vec4" => format!("vec4({src})"),
        t if t.starts_with("vec") => format!("{src}.{}", &"xyzw"[..t[3..].parse::<usize>().unwrap()]),
        t => {
            let n: usize = t[3..].parse().unwrap();
            format!("{t}({})", vec![format!("{src}.x"); n * n].join(", "))
        }
    }
}

fn stub_vs(list: &[(String, String, usize)]) -> String {
    let mut s: Vec<String> =
        ["#version 100", "precision highp float;", "attribute vec4 a_pos;", "uniform vec4 u_stub;"]
            .map(String::from)
            .to_vec();
    let mut body = vec!["    gl_Position = a_pos;".to_string()];
    for (ty, name, arr) in list {
        s.push(format!("varying {ty} {name}{};", if *arr > 0 { format!("[{arr}]") } else { String::new() }));
        for i in 0..(*arr).max(1) {
            let e = if *arr > 0 { format!("{name}[{i}]") } else { name.clone() };
            body.push(format!("    {e} = {};", value(ty, "u_stub")));
        }
    }
    s.extend(["void main()".to_string(), "{".to_string()]);
    s.extend(body);
    s.extend(["}".to_string(), String::new()]);
    s.join("\n")
}

fn stub_fs(list: &[(String, String, usize)]) -> String {
    let mut s: Vec<String> = vec!["#version 100".into(), "precision highp float;".into()];
    let mut body = vec!["    vec4 c = vec4(0.0);".to_string()];
    for (ty, name, arr) in list {
        s.push(format!("varying {ty} {name}{};", if *arr > 0 { format!("[{arr}]") } else { String::new() }));
        for i in 0..(*arr).max(1) {
            let e = if *arr > 0 { format!("{name}[{i}]") } else { name.clone() };
            if let Some(n) = ty.strip_prefix("mat") {
                let n: usize = n.parse().unwrap();
                let sum: Vec<String> = (0..n).map(|k| format!("{e}[{k}].x")).collect();
                body.push(format!("    c.x += {};", sum.join(" + ")));
            } else if ty == "float" {
                body.push(format!("    c += vec4({e});"));
            } else {
                let pad =
                    if ty == "vec4" { String::new() } else { ", 0.0".repeat(4 - ty[3..].parse::<usize>().unwrap()) };
                body.push(format!("    c += vec4({e}{pad});"));
            }
        }
    }
    s.extend(["void main()".to_string(), "{".to_string()]);
    s.extend(body);
    s.extend(["    gl_FragColor = c;".to_string(), "}".to_string(), String::new()]);
    s.join("\n")
}

// ---------------------------------------------------------------------------
// The list, and the judgement.
// ---------------------------------------------------------------------------

/// `<key>: <message>` lines of the list; `#` lines and blank lines are prose.
fn listed() -> BTreeMap<String, String> {
    let path = root().join("known-uncompilable-v3d71.txt");
    let list = std::fs::read_to_string(&path).expect("the V3D 7.1 list exists");
    let mut m = BTreeMap::new();
    for l in list.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (k, msg) = l.split_once(": ").expect("`<key>: <message>`");
        assert!(m.insert(k.to_string(), msg.to_string()).is_none(), "{k} is listed twice");
    }
    m
}

/// The refusals of one set against the list's entries for it (`mine` picks
/// the set's keys): the differences, one line each.
fn judge(refused: &BTreeMap<String, String>, mine: impl Fn(&str) -> bool) -> Vec<String> {
    let want: BTreeMap<String, String> = listed().into_iter().filter(|(k, _)| mine(k)).collect();
    let mut bad = Vec::new();
    for (k, m) in refused {
        match want.get(k) {
            None => bad.push(format!("NEW refusal, not listed: {k}: {m}")),
            Some(w) if w != m => bad.push(format!("message changed: {k}\n    listed: {w}\n    now:    {m}")),
            Some(_) => {}
        }
    }
    for k in want.keys().filter(|k| !refused.contains_key(*k)) {
        bad.push(format!("listed but now compiles (remove it): {k}"));
    }
    bad
}

/// The refusals by message, most frequent first.
fn summary(refused: &BTreeMap<String, String>) {
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for m in refused.values() {
        *by.entry(m.as_str()).or_default() += 1;
    }
    let mut by: Vec<(&str, usize)> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (m, n) in by {
        println!("  {n:>4} x {m}");
    }
}

#[test]
fn every_es100_program_compiles_at_7_1_or_is_a_listed_refusal() {
    let mut files = Vec::new();
    sources(&root().join("es100"), &mut files);
    let mut rels: Vec<String> =
        files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().into_owned()).collect();
    rels.sort();
    /* The population first: an empty walk would make "nothing refused" true. */
    assert_eq!(rels.len(), ES100_PROGRAMS, "tools/shaders/es100 holds {ES100_PROGRAMS} sources; re-derive the pins");

    /* Mesa's row per source: (instructions, threads) when it compiled */
    let mesa: BTreeMap<&str, Option<(usize, usize)>> = ES100_71
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c[0], (c[2] == "yes").then(|| (c[4].parse().unwrap(), c[5].parse().unwrap())))
        })
        .collect();
    assert_eq!(mesa.len(), ES100_PROGRAMS, "es100_counts_71.tsv rows");

    let mut refused = BTreeMap::new();
    let (mut ours_total, mut mesa_total, mut both, mut branchy) = (0usize, 0usize, 0usize, 0usize);
    println!(
        "v3d71 es100: source, stage, ours (instructions), Mesa 7.1 (instructions, threads), `branch` when ours has one"
    );
    for rel in &rels {
        let src = read(rel);
        let vary = varyings(&src);
        let vert = rel.ends_with(".vert");
        let (vs, fs, names) = if vert {
            (src.clone(), stub_fs(&vary), [rel.as_str(), "<stub fs>"])
        } else {
            (stub_vs(&vary), src.clone(), ["<stub vs>", rel.as_str()])
        };
        let got = compile(rel, names, &vs, &fs);
        let m = *mesa.get(rel.as_str()).unwrap_or_else(|| panic!("{rel}: no es100_counts_71.tsv row"));
        branchy += matches!(got, Ok((_, true))) as usize;
        match (&got, m) {
            (Ok((n, br)), Some((mi, mt))) => {
                let o = if vert { n[1] } else { n[0] };
                let br = if *br { "\tbranch" } else { "" };
                println!("  {rel}\t{}\t{o}\t{mi} ({mt} threads){br}", if vert { "vs" } else { "fs" });
                ours_total += o;
                mesa_total += mi;
                both += 1;
            }
            (Ok((n, _)), None) => println!("  {rel}: ours {n:?}, Mesa refused"),
            (Err(e), _) => {
                refused.insert(rel.clone(), e.clone());
            }
        }
    }
    let compiled = rels.len() - refused.len();
    println!(
        "v3d71 es100: {} programs, {compiled} compiled at 7.1 ({branchy} with a branch), {} refused; \
         on the {both} both compile: ours {ours_total} instructions, Mesa {mesa_total}",
        rels.len(),
        refused.len()
    );
    summary(&refused);
    for (k, m) in &refused {
        println!("  refused {k}: {m}");
    }
    let bad = judge(&refused, |k| !k.contains(" + "));
    assert!(bad.is_empty(), "tools/shaders/known-uncompilable-v3d71.txt is out of date (es100):\n{}", bad.join("\n"));
    assert_eq!(compiled, ES100_COMPILED, "es100 programs compiled at 7.1: a moved count is a decision");
}

#[test]
fn every_qpu_corpus_71_program_compiles_at_7_1_or_is_a_listed_refusal() {
    /* the programs, each `vs + fs` key once, with its two sources */
    let mut programs: BTreeMap<String, [String; 2]> = BTreeMap::new();
    for l in SOURCES_71.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 3, "program, stage, source: {l}");
        let e = programs.entry(c[0].to_string()).or_default();
        match c[1] {
            "fs" => e[1] = c[2].to_string(),
            "vs" | "cs" => e[0] = c[2].to_string(),
            s => panic!("stage {s}"),
        }
    }
    assert_eq!(programs.len(), CORPUS_PROGRAMS, "qpu_corpus_71_sources.tsv's programs");
    let mesa: BTreeMap<String, [usize; 3]> = COUNTS_71
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (format!("{} + {}", c[0], c[1]), [c[2].parse().unwrap(), c[3].parse().unwrap(), c[4].parse().unwrap()])
        })
        .collect();

    let mut refused = BTreeMap::new();
    let (mut ours_total, mut mesa_total, mut branchy) = ([0usize; 3], [0usize; 3], 0usize);
    let mut text: BTreeMap<String, String> = BTreeMap::new();
    for (key, [v, f]) in &programs {
        assert!(key == &format!("{v} + {f}"), "{key}: sources {v}, {f}");
        let vs = text.entry(v.clone()).or_insert_with(|| read(v)).clone();
        let fs = text.entry(f.clone()).or_insert_with(|| read(f)).clone();
        match compile(key, [v, f], &vs, &fs) {
            Ok((n, branch)) => {
                branchy += branch as usize;
                let m = mesa.get(key).unwrap_or_else(|| panic!("{key}: no qpu_counts_71.tsv row"));
                let br = if branch { ", branch" } else { "" };
                println!(
                    "  {key}: ours fs {} vs {} cs {}, Mesa fs {} vs {} cs {}{br}",
                    n[0], n[1], n[2], m[0], m[1], m[2]
                );
                for s in 0..3 {
                    ours_total[s] += n[s];
                    mesa_total[s] += m[s];
                }
            }
            Err(e) => {
                refused.insert(key.clone(), e);
            }
        }
    }
    let compiled = programs.len() - refused.len();
    println!(
        "v3d71 qpu_corpus_71: {} programs, {compiled} compiled at 7.1 ({branchy} with a branch), {} refused; \
         on the {compiled} compiled \
         (fs, vs, cs): ours {ours_total:?} instructions, Mesa {mesa_total:?}",
        programs.len(),
        refused.len()
    );
    summary(&refused);
    for (k, m) in &refused {
        println!("  refused {k}: {m}");
    }
    let bad = judge(&refused, |k| k.contains(" + "));
    assert!(
        bad.is_empty(),
        "tools/shaders/known-uncompilable-v3d71.txt is out of date (qpu_corpus_71):\n{}",
        bad.join("\n")
    );
    assert_eq!(compiled, CORPUS_COMPILED, "qpu_corpus_71 programs compiled at 7.1: a moved count is a decision");
}

/// The stubs are es100_ref71.sh's: a source with a float, a vec2 array and
/// a mat2 varying, behind comments.
#[test]
fn the_stubs_are_es100_ref71s() {
    let src = "// varying vec4 not_me;\nvarying highp float a; /* varying vec3 nor_me; */\n\
               varying vec2 b[2], d;\nvarying mat2 m;\nvoid main() {}\n";
    let v = varyings(src);
    let want = [("float", "a", 0), ("vec2", "b", 2), ("vec2", "d", 0), ("mat2", "m", 0)];
    assert_eq!(v.len(), want.len(), "{v:?}");
    for ((ty, n, k), (wt, wn, wk)) in v.iter().zip(want) {
        assert_eq!((ty.as_str(), n.as_str(), *k), (wt, wn, wk));
    }
    assert_eq!(
        stub_vs(&v),
        "#version 100\nprecision highp float;\nattribute vec4 a_pos;\nuniform vec4 u_stub;\nvarying float a;\n\
         varying vec2 b[2];\nvarying vec2 d;\nvarying mat2 m;\nvoid main()\n{\n    gl_Position = a_pos;\n\
         \x20   a = u_stub.x;\n    b[0] = u_stub.xy;\n    b[1] = u_stub.xy;\n    d = u_stub.xy;\n\
         \x20   m = mat2(u_stub.x, u_stub.x, u_stub.x, u_stub.x);\n}\n"
    );
    assert_eq!(
        stub_fs(&v),
        "#version 100\nprecision highp float;\nvarying float a;\nvarying vec2 b[2];\nvarying vec2 d;\n\
         varying mat2 m;\nvoid main()\n{\n    vec4 c = vec4(0.0);\n    c += vec4(a);\n\
         \x20   c += vec4(b[0], 0.0, 0.0);\n    c += vec4(b[1], 0.0, 0.0);\n    c += vec4(d, 0.0, 0.0);\n\
         \x20   c.x += m[0].x + m[1].x;\n    gl_FragColor = c;\n}\n"
    );
}
