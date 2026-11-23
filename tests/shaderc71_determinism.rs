// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The 7.1 online back end is deterministic inside one process: every
//! tools/shaders/es100 program (with the other stage's stub, as
//! tests/v3d71_online_corpus.rs builds it) is compiled three times, in
//! forward order, in reverse order and interleaved with other programs (a
//! program from the far end of the list is compiled before each one). The
//! three runs must give identical words, thread counts and uniform streams
//! for the fragment, vertex and coordinate stages. (det71 showed the words
//! equal across processes and profiles.)
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_determinism
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, TARGET71};
use yakogl::shaderc::online::{lower, store};

/// tools/shaders/es100's sources.
const ES100_PROGRAMS: usize = 126;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders")
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

/// Everything a program's three stages emit, as text: per stage the thread
/// count, the words and the uniform stream; or the refusal.
fn fingerprint(vs: &str, fs: &str) -> String {
    /* the scratch region is one per process: the tests take turns */
    static SCRATCH: Mutex<()> = Mutex::new(());
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs), (Stage::Fragment, fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => return format!("front end: {}", e.msg),
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        match compile_program71(v, f, Prim::Triangles, TARGET71, a) {
            Err(e) => format!("refused: {e:?}"),
            Ok(p) => {
                let mut s = String::new();
                for (n, c, t) in [
                    ("fs", &p.fs.code, p.fs.threads),
                    ("vs", &p.vs.code, p.vs.threads),
                    ("cs", &p.cs.code, p.cs.threads),
                ] {
                    s += &format!("{n} threads {t} words {:016x?} uniforms {:?}\n", c.words, c.uniforms);
                }
                s
            }
        }
    })
    .expect("scratch")
}

// ---------------------------------------------------------------------------
// tests/v3d71_online_corpus.rs's stubs (es100_ref71.sh's), unchanged.
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

#[test]
fn es100_output_is_byte_identical_in_any_order() {
    let mut files = Vec::new();
    sources(&root().join("es100"), &mut files);
    let mut rels: Vec<String> =
        files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().into_owned()).collect();
    rels.sort();
    assert_eq!(rels.len(), ES100_PROGRAMS, "tools/shaders/es100 holds {ES100_PROGRAMS} sources");

    /* (vertex source, fragment source) per program */
    let pairs: Vec<(String, String)> = rels
        .iter()
        .map(|rel| {
            let src = std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let vary = varyings(&src);
            if rel.ends_with(".vert") {
                (src, stub_fs(&vary))
            } else {
                (stub_vs(&vary), src)
            }
        })
        .collect();
    let n = pairs.len();
    let one = |i: usize| fingerprint(&pairs[i].0, &pairs[i].1);

    let forward: Vec<String> = (0..n).map(one).collect();
    let mut reverse = vec![String::new(); n];
    for i in (0..n).rev() {
        reverse[i] = one(i);
    }
    /* interleaved: before each program, another from the far end is compiled */
    let mut mixed = vec![String::new(); n];
    for (i, slot) in mixed.iter_mut().enumerate() {
        let _other = one((n - 1 - i + 7) % n);
        *slot = one(i);
    }

    let compiled = forward.iter().filter(|f| f.starts_with("fs threads")).count();
    assert!(compiled > n / 2, "only {compiled} of {n} compiled: the comparison would be vacuous");
    for (i, rel) in rels.iter().enumerate() {
        assert_eq!(forward[i], reverse[i], "{rel}: forward and reverse runs differ");
        assert_eq!(forward[i], mixed[i], "{rel}: forward and interleaved runs differ");
    }
}
