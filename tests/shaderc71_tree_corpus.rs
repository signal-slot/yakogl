// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every `.vert`/`.frag` of `tools/shaders` outside `es100` through the V3D
//! 7.1 (Raspberry Pi 5) online back end, pinned. `tests/v3d71_online_corpus.rs`
//! covers `tools/shaders/es100` and `qpu_corpus_71.txt`; this is the rest of
//! the tree `tests/v3d_online_corpus.rs` walks at 4.2.
//!
//! * Each source is compiled at 4.2 alone (as `v3d_online_corpus.rs` does) and
//!   at 7.1 through `chain71::compile_program71` with a stub of the other stage
//!   (a stub VS writes every varying the FS declares from a uniform vec4, a
//!   stub FS adds every varying the VS declares into its output; the idea of
//!   `v3d71_online_corpus.rs`, in an ES 1.00 and an ES 3.00 flavour). Key: the
//!   source's path.
//! * Each `X.vert` + `X.frag` of one directory whose two sources both compile
//!   at 7.1 is one program, linked through `compile_program71`. Key:
//!   `<vert>+<frag>`.
//!
//! A program that compiles is checked: every word of its FS, VS and CS
//! decodes with codec71 to its instruction, and the instructions pass verify71
//! and `registers_fit_threads71`.
//!
//! Every 7.1 refusal is in `tools/shaders/known-uncompilable-v3d71-tree.txt`,
//! `<key>: <exact message>`, with a `#` reason block above it that says
//! whether 4.2 also refuses it (`4.2 also refuses`) or the refusal is
//! `7.1-only`. The test fails on an unlisted refusal, on a changed message, on
//! a listed entry that now compiles, and on a 7.1-only mark that no longer
//! matches 4.2. The number of programs that compile is pinned.
//!
//! tools/check.sh runs every `tests/` target with this feature set (its
//! "every tests/ target" row), so this test needs no check.sh row of its own.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_tree_corpus -- --nocapture
//! ```
#![cfg(all(feature = "gpu-v3d", feature = "shaderc-online"))]

use core::ffi::c_char;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::sched71_cf::branches_are_structured71;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// `.vert`/`.frag` sources of tools/shaders outside es100.
const SOURCES: usize = 198;
/// Programs (single sources with a stub, and linked pairs) that compile at 7.1.
const COMPILED: usize = 190;

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

// ---------------------------------------------------------------------------
// 4.2
// ---------------------------------------------------------------------------

/// Does the 4.2 compiler take this source (alone)?
fn compiles_at_4_2(stage: u32, src: &str) -> bool {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    yakogl::shaderc::online::backend::forget_compiled();
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) == 0 }
}

/// Do the two sources compile and link at 4.2?
fn links_at_4_2(vsrc: &str, fsrc: &str) -> bool {
    let mut vs = ShcShader::ZERO;
    let mut fs = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let c = yakogl::shaderc::online::SHC_ONLINE.compile.expect("compiles");
    let l = yakogl::shaderc::online::SHC_ONLINE.link.expect("links");
    yakogl::shaderc::online::backend::forget_compiled();
    let (zv, zf) = (format!("{vsrc}\0"), format!("{fsrc}\0"));
    // SAFETY: NUL-terminated sources, real outputs, logs of the size announced.
    unsafe {
        c(SHC_STAGE_VERTEX, zv.as_ptr() as *const c_char, &mut vs, log.as_mut_ptr(), log.len()) == 0
            && c(SHC_STAGE_FRAGMENT, zf.as_ptr() as *const c_char, &mut fs, log.as_mut_ptr(), log.len()) == 0
            && l(&vs, &mut fs, log.as_mut_ptr(), log.len()) == 0
    }
}

// ---------------------------------------------------------------------------
// 7.1
// ---------------------------------------------------------------------------

/// The instruction counts (fs, vs, cs) of a program that compiled, or the
/// refusal's message.
type Outcome = Result<[usize; 3], String>;

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

fn compile71(key: &str, names: [&str; 2], vs: &str, fs: &str) -> Outcome {
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

// ---------------------------------------------------------------------------
// Stubs of the other stage.
// ---------------------------------------------------------------------------

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

const TYPES: [&str; 16] = [
    "float", "vec2", "vec3", "vec4", "mat2", "mat3", "mat4", "int", "ivec2", "ivec3", "ivec4", "uint", "uvec2",
    "uvec3", "uvec4", "bool",
];
const QUALS: [&str; 6] = ["smooth", "centroid", "highp", "mediump", "lowp", "flat"];

/// One interface variable.
struct Var {
    flat: bool,
    ty: String,
    name: String,
    arr: usize,
}

fn is_es300(src: &str) -> bool {
    src.contains("#version 300") || src.contains("#version 310")
}

/// The source's interface declarations: the statements that start (after
/// qualifiers) with the storage keyword `keyword`.
fn interface(src: &str, keyword: &str) -> Vec<Var> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let t = strip_comments(src);
    /* preprocessor lines are not statements */
    let t: String = t.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n");
    let mut out = Vec::new();
    for stmt in t.split([';', '{', '}']) {
        let mut toks: Vec<&str> = stmt.split_whitespace().collect();
        /* `layout(...)` may hold spaces: drop up to the closing paren */
        if toks.first().is_some_and(|k| k.starts_with("layout")) {
            let end = toks.iter().position(|k| k.contains(')')).unwrap_or(0);
            toks.drain(..=end);
        }
        let mut flat = false;
        let mut seen = false;
        let mut ty = None;
        let mut i = 0;
        while i < toks.len() {
            let k = toks[i];
            i += 1;
            if k == keyword {
                seen = true;
            } else if k == "flat" {
                flat = true;
            } else if QUALS.contains(&k) {
            } else if TYPES.contains(&k) {
                ty = Some(k);
                break;
            } else {
                break;
            }
        }
        let (true, Some(ty)) = (seen, ty) else { continue };
        for d in toks[i..].join(" ").split(',') {
            let d = d.trim();
            let (name, n) = match d.split_once('[') {
                Some((n, rest)) => (n.trim(), rest.trim_end_matches(']').trim().parse().unwrap_or(1)),
                None => (d, 0),
            };
            if name.is_empty() || !name.bytes().all(word) {
                continue;
            }
            let int = ty.starts_with('i') || ty.starts_with('u') || ty == "bool";
            out.push(Var { flat: flat || int, ty: ty.into(), name: name.into(), arr: n });
        }
    }
    out
}

/// An expression of type `ty` from the uniform vec4 `src`.
fn value(ty: &str, src: &str) -> String {
    match ty {
        "float" => format!("{src}.x"),
        "int" => format!("int({src}.x)"),
        "uint" => format!("uint({src}.x)"),
        "bool" => format!("({src}.x > 0.5)"),
        "vec4" => format!("vec4({src})"),
        t if t.starts_with("vec") => format!("{src}.{}", &"xyzw"[..t[3..].parse::<usize>().unwrap()]),
        t if t.starts_with("ivec") || t.starts_with("uvec") => {
            let n: usize = t[4..].parse().unwrap();
            format!("{t}(ivec4({src}).{})", &"xyzw"[..n])
        }
        t => {
            let n: usize = t[3..].parse().unwrap();
            format!("{t}({})", vec![format!("{src}.x"); n * n].join(", "))
        }
    }
}

fn decl(es300: bool, kw: &str, v: &Var) -> String {
    let arr = if v.arr > 0 { format!("[{}]", v.arr) } else { String::new() };
    if es300 {
        format!("{}{kw} highp {} {}{arr};", if v.flat { "flat " } else { "" }, v.ty, v.name)
    } else {
        format!("varying highp {} {}{arr};", v.ty, v.name)
    }
}

fn elems(v: &Var) -> Vec<String> {
    (0..v.arr.max(1)).map(|i| if v.arr > 0 { format!("{}[{i}]", v.name) } else { v.name.clone() }).collect()
}

fn header(es300: bool, vertex: bool) -> Vec<String> {
    let mut s: Vec<String> = Vec::new();
    if es300 {
        s.extend(["#version 300 es".into(), "precision highp float;".into(), "precision highp int;".into()]);
        s.push(if vertex { "layout(location = 0) in vec4 a_pos;".into() } else { "out vec4 o_color;".into() });
    } else {
        s.extend(["#version 100".into(), "precision highp float;".into()]);
        if vertex {
            s.push("attribute vec4 a_pos;".into());
        }
    }
    s
}

fn stub_vs(es300: bool, list: &[Var]) -> String {
    let mut s = header(es300, true);
    s.push("uniform vec4 u_stub;".into());
    let mut body = vec!["    gl_Position = a_pos;".to_string()];
    for v in list {
        s.push(decl(es300, "out", v));
        for e in elems(v) {
            body.push(format!("    {e} = {};", value(&v.ty, "u_stub")));
        }
    }
    s.extend(["void main()".into(), "{".into()]);
    s.extend(body);
    s.extend(["}".into(), String::new()]);
    s.join("\n")
}

fn stub_fs(es300: bool, list: &[Var]) -> String {
    let mut s = header(es300, false);
    let mut body = vec!["    vec4 c = vec4(0.0);".to_string()];
    for v in list {
        s.push(decl(es300, "in", v));
        for e in elems(v) {
            if v.ty.starts_with("mat") {
                body.push(format!("    c.x += {e}[0].x;"));
            } else if matches!(v.ty.as_str(), "float" | "int" | "uint" | "bool") {
                body.push(format!("    c += vec4({e});"));
            } else {
                let n: usize = v.ty[v.ty.len() - 1..].parse().unwrap();
                let pad = ", 0.0".repeat(4 - n);
                let f = if v.ty.starts_with("vec") { e.clone() } else { format!("vec{n}({e})") };
                body.push(if n == 4 { format!("    c += vec4({f});") } else { format!("    c += vec4({f}{pad});") });
            }
        }
    }
    s.extend(["void main()".into(), "{".into()]);
    s.extend(body);
    s.push(if es300 { "    o_color = c;".into() } else { "    gl_FragColor = c;".into() });
    s.extend(["}".into(), String::new()]);
    s.join("\n")
}

/// The stub of the other stage for `src`.
fn stub_for(vert: bool, src: &str) -> String {
    let es300 = is_es300(src);
    let (out_kw, in_kw) = if es300 { ("out", "in") } else { ("varying", "varying") };
    if vert {
        stub_fs(es300, &interface(src, out_kw))
    } else {
        stub_vs(es300, &interface(src, in_kw))
    }
}

// ---------------------------------------------------------------------------
// The list.
// ---------------------------------------------------------------------------

/// `<key>: <message>` lines, with whether the `#` block above says `7.1-only`.
fn listed() -> BTreeMap<String, (String, bool)> {
    let path = root().join("known-uncompilable-v3d71-tree.txt");
    let list = std::fs::read_to_string(&path).expect("the tree list exists");
    let mut m = BTreeMap::new();
    let mut only = false;
    for l in list.lines() {
        if l.trim().is_empty() {
            only = false;
        } else if l.starts_with('#') {
            if l.contains("7.1-only") {
                only = true;
            }
        } else {
            let (k, msg) = l.split_once(": ").expect("`<key>: <message>`");
            assert!(m.insert(k.to_string(), (msg.to_string(), only)).is_none(), "{k} is listed twice");
        }
    }
    m
}

fn tag(only: bool) -> &'static str {
    if only {
        "7.1-only"
    } else {
        "4.2 also refuses"
    }
}

#[test]
fn tools_shaders_beyond_es100_compile_at_7_1_or_are_listed_refusals() {
    let start = std::time::Instant::now();
    let r = root();
    let mut files = Vec::new();
    sources(&r, &mut files);
    let mut rels: Vec<String> = files
        .iter()
        .map(|p| p.strip_prefix(&r).unwrap().to_string_lossy().into_owned())
        .filter(|p| !p.starts_with("es100/"))
        .collect();
    rels.sort();
    assert_eq!(rels.len(), SOURCES, "tools/shaders outside es100 holds {SOURCES} sources; re-derive the pins");

    let mut compiled: BTreeMap<String, String> = BTreeMap::new();
    let mut refused: BTreeMap<String, (String, bool)> = BTreeMap::new(); /* message, 7.1-only */
    let mut programs = 0usize;
    for rel in &rels {
        let src = std::fs::read_to_string(r.join(rel)).expect("a UTF-8 source");
        let vert = rel.ends_with(".vert");
        let ok42 = compiles_at_4_2(if vert { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT }, &src);
        let stub = stub_for(vert, &src);
        let got = if vert {
            compile71(rel, [rel, "<stub fs>"], &src, &stub)
        } else {
            compile71(rel, ["<stub vs>", rel], &stub, &src)
        };
        programs += 1;
        match got {
            Ok(_) => {
                compiled.insert(rel.clone(), src);
            }
            Err(m) => {
                refused.insert(rel.clone(), (m, ok42));
            }
        }
    }
    let n_single = compiled.len();
    let mut n_pairs = 0usize;
    for (v, vsrc) in compiled.iter().filter(|(k, _)| k.ends_with(".vert")) {
        let f = format!("{}.frag", v.trim_end_matches(".vert"));
        let Some(fsrc) = compiled.get(&f) else { continue };
        programs += 1;
        let key = format!("{v}+{f}");
        match compile71(&key, [v, &f], vsrc, fsrc) {
            Ok(_) => n_pairs += 1,
            Err(m) => {
                refused.insert(key, (m, links_at_4_2(vsrc, fsrc)));
            }
        }
    }
    let n_ok = n_single + n_pairs;
    let n_only = refused.values().filter(|(_, o)| *o).count();
    println!(
        "shaderc71 tree: {} sources, {programs} programs, {n_ok} compiled ({n_single} single, {n_pairs} pairs), \
         {} refused, {n_only} 7.1-only; {:?}",
        rels.len(),
        refused.len(),
        start.elapsed()
    );
    for (k, (m, o)) in &refused {
        println!("  {}{k}: {m}", if *o { "[7.1-only] " } else { "" });
    }

    let want = listed();
    let mut bad = Vec::new();
    for (k, (m, o)) in &refused {
        match want.get(k) {
            None => bad.push(format!("NEW refusal, not listed: {k}: {m}")),
            Some((w, _)) if w != m => bad.push(format!("message changed: {k}\n    listed: {w}\n    now:    {m}")),
            Some((_, wo)) if wo != o => bad.push(format!("{k}: listed as {}, now {}", tag(*wo), tag(*o))),
            Some(_) => {}
        }
    }
    for k in want.keys().filter(|k| !refused.contains_key(*k)) {
        bad.push(format!("listed but now compiles (remove it): {k}"));
    }
    assert!(bad.is_empty(), "known-uncompilable-v3d71-tree.txt is out of date:\n{}", bad.join("\n"));
    assert_eq!(n_ok, COMPILED, "programs compiled at 7.1: a moved count is a decision");
}
