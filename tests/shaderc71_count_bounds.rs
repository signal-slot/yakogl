// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Ceilings on the V3D 7.1 online back end's instruction counts and floors
//! on its thread counts, over the two program sets
//! `tests/v3d71_online_corpus.rs` measures (that test pins *what compiles*;
//! this one pins *what the compiles cost*):
//!
//! * **es100**: every `.vert`/`.frag` of `tools/shaders/es100`, each paired
//!   with the stub of the other stage `tools/v3d/es100_ref71.sh` gives Mesa
//!   (the stubs are copied here, line for line), so every program is the one
//!   `tools/v3d/es100_counts_71.tsv` measured.
//! * **qpu_corpus_71**: every `vs + fs` pair of `tools/v3d/qpu_corpus_71.txt`
//!   with the sources `tools/v3d/qpu_corpus_71_sources.tsv` names, against
//!   `tools/v3d/qpu_counts_71.tsv`.
//!
//! For each set, through `chain71::compile_program71`: the total words of
//! the fragment, vertex and coordinate shaders of every program that
//! compiles, the sum of their thread counts, and — on the programs Mesa 7.1
//! compiled too — the number of programs whose comparable total (the source
//! stage for es100, whose Mesa row holds only that stage; fs + vs + cs for
//! qpu_corpus_71) exceeds Mesa's by more than 2x. A `cargo test --nocapture`
//! line per set prints ours, Mesa's and the ratio; the three constants bound
//! the sums of both sets:
//!
//! * `COUNT_CEILING` — the total words. A count regression (more words for
//!   the same programs) breaks it.
//! * `THREAD_FLOOR` — the sum of the thread counts. A change that packs
//!   programs into fewer threads than today breaks it.
//! * `OVER_2X_CEILING` — the programs more than 2x over Mesa. A count
//!   regression on the programs Mesa covers breaks it.
//!
//! A fix that improves the numbers must lower `COUNT_CEILING` and
//! `OVER_2X_CEILING` (or raise `THREAD_FLOOR`) in its own commit, exactly as
//! the corpus test re-pins a moved compile count.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_count_bounds -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, TARGET71};
use yakogl::shaderc::online::{lower, store};

/// tools/shaders/es100's sources (es100_counts_71.tsv's rows).
const ES100_PROGRAMS: usize = 126;
/// qpu_corpus_71.txt's programs (`# pairs: 1090 ok`).
const CORPUS_PROGRAMS: usize = 1090;

// ---------------------------------------------------------------------------
// The pins, measured at b953cc96 (2026-11-22) with this file's cargo
// command: the sums over es100 (12973 words, 1502 threads, 0 programs over
// 2x) and qpu_corpus_71 (107796 words, 10408 threads, 164 over 2x).
// Lowered with the fsat / fclamp_pos FMOV unpacks and the signal-aware
// interleave order (2026-11-23): es100 12918 words, qpu_corpus_71 107117
// (fs 40246 -> 39567); threads and the over-2x count unchanged.
// ---------------------------------------------------------------------------

/// The total words of the fs, vs and cs of every program both sets compile
/// at 7.1.
const COUNT_CEILING: usize = 120_035;
/// The sum of those programs' thread counts (fs + vs + cs each).
const THREAD_FLOOR: usize = 11_910;
/// The programs whose comparable total is more than 2x Mesa 7.1's.
const OVER_2X_CEILING: usize = 164;

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

/// The per-stage words and threads of a program that compiled, or the
/// refusal's message.
type Outcome = Result<([usize; 3], [usize; 3]), String>;

/// Compile `vs` + `fs` at 7.1 and return the fs, vs and cs word counts and
/// thread counts. `names` are the two sources' names for the front end's
/// message. (The words themselves are decoded, verified and branch-checked
/// by `tests/v3d71_online_corpus.rs`; this measures the cost.)
fn compile(names: [&str; 2], vs: &str, fs: &str) -> Outcome {
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
        Ok((
            [p.fs.code.words.len(), p.vs.code.words.len(), p.cs.code.words.len()],
            [p.fs.threads as usize, p.vs.threads as usize, p.cs.threads as usize],
        ))
    })
    .expect("scratch")
}

// ---------------------------------------------------------------------------
// es100_ref71.sh's stubs, line for line (as in tests/v3d71_online_corpus.rs).
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
// One set's tally.
// ---------------------------------------------------------------------------

/// What a set measured: `words` and `threads` are fs, vs and cs sums over
/// the programs that compiled; `ours` and `mesa` are comparable totals on
/// the `both` programs Mesa 7.1 compiled too; `over2` counts the ones more
/// than 2x over.
struct Tally {
    name: &'static str,
    programs: usize,
    compiled: usize,
    words: [usize; 3],
    threads: usize,
    both: usize,
    ours: usize,
    mesa: usize,
    over2: usize,
}

impl Tally {
    fn words_total(&self) -> usize {
        self.words.iter().sum()
    }

    /// The one line per set: ours, Mesa's, and the ratio.
    fn print(&self) {
        println!(
            "shaderc71 {} programs: {} total, {} compiled at 7.1; words {} (fs {} vs {} cs {}), threads {}; \
             on the {} both compile: ours {ours}, Mesa {mesa}, ratio {ratio:.2}",
            self.name,
            self.programs,
            self.compiled,
            self.words_total(),
            self.words[0],
            self.words[1],
            self.words[2],
            self.threads,
            self.both,
            ours = self.ours,
            mesa = self.mesa,
            ratio = self.ours as f64 / self.mesa.max(1) as f64,
        );
    }
}

#[test]
fn the_7_1_counts_stay_within_the_pins() {
    /* --- es100 ---------------------------------------------------------- */
    let mut files = Vec::new();
    sources(&root().join("es100"), &mut files);
    let mut rels: Vec<String> =
        files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().into_owned()).collect();
    rels.sort();
    /* The population first: an empty walk would make the bounds vacuous. */
    assert_eq!(rels.len(), ES100_PROGRAMS, "tools/shaders/es100 holds {ES100_PROGRAMS} sources; re-derive the pins");

    /* Mesa's row per source: its stage's (instructions, threads) when it compiled. */
    let mesa: BTreeMap<&str, Option<usize>> = ES100_71
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c[0], (c[2] == "yes").then(|| c[4].parse().unwrap()))
        })
        .collect();
    assert_eq!(mesa.len(), ES100_PROGRAMS, "es100_counts_71.tsv rows");

    let mut es = Tally { name: "es100", programs: rels.len(), ..ES_EMPTY };
    for rel in &rels {
        let src = read(rel);
        let vary = varyings(&src);
        let vert = rel.ends_with(".vert");
        let (vs, fs) = if vert { (src.clone(), stub_fs(&vary)) } else { (stub_vs(&vary), src.clone()) };
        match compile(if vert { [rel.as_str(), "<stub fs>"] } else { ["<stub vs>", rel.as_str()] }, &vs, &fs) {
            Ok((n, t)) => {
                es.compiled += 1;
                for (w, x) in es.words.iter_mut().zip(n) {
                    *w += x;
                }
                es.threads += t.iter().sum::<usize>();
                /* Mesa's row is the source's stage only; that stage is the
                comparable total. */
                if let Some(mi) = mesa[rel.as_str()] {
                    let o = n[if vert { 1 } else { 0 }];
                    es.both += 1;
                    es.ours += o;
                    es.mesa += mi;
                    if o > 2 * mi {
                        es.over2 += 1;
                        println!("  shaderc71 es100 over 2x: {rel}: ours {o}, Mesa {mi}");
                    }
                }
            }
            Err(e) => println!("  shaderc71 es100 refused {rel}: {e}"),
        }
    }

    /* --- qpu_corpus_71 --------------------------------------------------- */
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
    let qmesa: BTreeMap<String, usize> = COUNTS_71
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (
                format!("{} + {}", c[0], c[1]),
                c[2].parse::<usize>().unwrap() + c[3].parse::<usize>().unwrap() + c[4].parse::<usize>().unwrap(),
            )
        })
        .collect();

    let mut qpu = Tally { name: "qpu_corpus_71", programs: programs.len(), ..ES_EMPTY };
    let mut text: BTreeMap<String, String> = BTreeMap::new();
    for (key, [v, f]) in &programs {
        let vs = text.entry(v.clone()).or_insert_with(|| read(v)).clone();
        let fs = text.entry(f.clone()).or_insert_with(|| read(f)).clone();
        /* every refusal is in known-uncompilable-v3d71.txt, which
        tests/v3d71_online_corpus.rs pins exactly; it contributes nothing here */
        if let Ok((n, t)) = compile([v, f], &vs, &fs) {
            qpu.compiled += 1;
            for (w, x) in qpu.words.iter_mut().zip(n) {
                *w += x;
            }
            qpu.threads += t.iter().sum::<usize>();
            let m = qmesa.get(key).unwrap_or_else(|| panic!("{key}: no qpu_counts_71.tsv row"));
            let o = n[0] + n[1] + n[2];
            qpu.both += 1;
            qpu.ours += o;
            qpu.mesa += m;
            if o > 2 * m {
                qpu.over2 += 1;
                println!("  shaderc71 qpu_corpus_71 over 2x: {key}: ours {o}, Mesa {m}");
            }
        }
    }

    /* --- the bounds, over both sets --------------------------------------- */
    es.print();
    qpu.print();
    let (count, threads, over2) =
        (es.words_total() + qpu.words_total(), es.threads + qpu.threads, es.over2 + qpu.over2);
    println!(
        "shaderc71 bounds: COUNT {count} (ceiling {COUNT_CEILING}), THREADS {threads} (floor \
         {THREAD_FLOOR}), OVER_2X {over2} (ceiling {OVER_2X_CEILING})"
    );
    let fix = "a fix that improves these numbers lowers the ceilings (and raises the floor) in its own commit";
    assert!(count <= COUNT_CEILING, "total words {count} exceed COUNT_CEILING {COUNT_CEILING}: {fix}");
    assert!(threads >= THREAD_FLOOR, "thread sum {threads} is below THREAD_FLOOR {THREAD_FLOOR}: {fix}");
    assert!(over2 <= OVER_2X_CEILING, "{over2} programs over 2x Mesa exceed OVER_2X_CEILING {OVER_2X_CEILING}: {fix}");
}

/// A tally with nothing measured yet (the per-set loops add everything).
const ES_EMPTY: Tally =
    Tally { name: "", programs: 0, compiled: 0, words: [0; 3], threads: 0, both: 0, ours: 0, mesa: 0, over2: 0 };
