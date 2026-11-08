// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every shader glmark2 passes to `glShaderSource` through this build's online
//! compiler, and the vertex/fragment pairs glmark2 links through its link step.
//!
//! The corpus is GPL-3.0 text and is never committed: `tools/hostharness/glmark2/run.sh
//! "$GLMARK2_SRC"` generates it into `build/glmark2-host/shaders/` (or
//! `GLMARK2_CORPUS` names another directory holding `index.txt`). With no
//! corpus at the default place the test prints a SKIP line and returns: the
//! plain test rows of the gate run before the corpus exists. A directory
//! `GLMARK2_CORPUS` names must hold the corpus, or the test fails: that is
//! how the gate's own row (`tools/check.sh`, which generates the corpus first
//! and makes a missing `GLMARK2_SRC` a gap) refuses an empty run. Every
//! shader must compile and link. Per back end:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test glmark2_corpus -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test glmark2_corpus -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test glmark2_corpus -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test glmark2_corpus -- --nocapture
//! ```
//!
//! Links: every vertex/fragment pair of one index entry is tried, and a shader
//! that links with no partner in its entry fails (named by its own file in the
//! list). A shader that does not compile is not linked.
//!
//! The Mali (`gpu-mali`) only reports: its back end still refuses shapes
//! glmark2 uses (issues #32 and #33, docs/notes/mali-glmark2-forecast.md),
//! so it prints one `mali: <vs>+<fs> <ok|refused: reason|error: reason>` line
//! per program and a `mali-entry:` line per entry, and fails only on a
//! corpus that is not one. Its programs are the pairs glmark2 linked, read
//! from the run's PowerVR logs (`<corpus>/../logs/powervr/<n>.log`, a
//! vertex shader's `gm2: shader` line followed by its fragment shader's),
//! or every pair of an entry when the logs are not there. Each is built the
//! way glLinkProgram builds it on the Mali (src/gl/program.rs `link_program`):
//! the two compiles, the vertex shader for the fragment shader
//! (`compile_vs_linked` with no map), then the link.
#![cfg(all(
    feature = "shaderc-online",
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d", feature = "gpu-mali")
))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const BACKEND: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "gc7000"
} else if cfg!(feature = "gpu-mali") {
    "mali"
} else {
    "v3d"
};

fn text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim().replace('\n', " / ")
}

/// A refusal of the compiler or link (`-ENOTSUP`: a shape the back end does
/// not build yet), told apart from an error (any other code).
#[derive(Debug)]
enum Fail {
    Refused(String),
    Error(String),
}

impl Fail {
    fn of(rc: i32, log: &[c_char]) -> Fail {
        let m = format!("rc {rc}: {}", text(log));
        /* ENOTSUP, the code `report` and the Mali's link give a refusal */
        if rc == -95 {
            Fail::Refused(m)
        } else {
            Fail::Error(m)
        }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::Refused(m) | Fail::Error(m) => f.write_str(m),
        }
    }
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, Fail> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text_z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text_z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(Fail::of(rc, &log))
    }
}

fn link(vs: &ShcShader, fs: &mut ShcShader) -> Result<(), Fail> {
    let mut log = [0 as c_char; 1024];
    let f = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { f(vs, fs, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(Fail::of(rc, &log))
    }
}

/// The corpus: its directory, its files in index order, and per entry its
/// vertex and fragment files. None (after the SKIP line) when there is none
/// at the default place.
type Entries = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn corpus() -> Option<(PathBuf, Vec<String>, Entries)> {
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
            "SKIP glmark2_corpus: no corpus at {} (it is never committed; set GLMARK2_SRC and run tools/hostharness/glmark2/run.sh)",
            dir.display()
        );
        return None;
    };
    /* the files, and per entry its vertex and fragment files */
    let mut files = Vec::new();
    let mut by_entry: Entries = BTreeMap::new();
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
    assert_eq!(files.len(), 69, "index.txt lists 69 shaders; re-derive the pin if it changed");
    Some((dir, files, by_entry))
}

#[cfg(not(feature = "gpu-mali"))]
fn stage_of(n: &str) -> u32 {
    if n.contains(".vert.") {
        SHC_STAGE_VERTEX
    } else {
        SHC_STAGE_FRAGMENT
    }
}

/// The one test of this binary (it resets the compiler: src/test_rules.rs,
/// one test per such binary): the Mali reports, the others must build all.
#[test]
fn every_glmark2_shader_compiles_and_links() {
    #[cfg(not(feature = "gpu-mali"))]
    all_compile_and_link();
    #[cfg(feature = "gpu-mali")]
    every_program_reported_on_the_mali();
}

#[cfg(not(feature = "gpu-mali"))]
fn all_compile_and_link() {
    let Some((dir, files, by_entry)) = corpus() else {
        return;
    };
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).expect("a shader file");
    let mut failed: BTreeMap<String, String> = BTreeMap::new();
    for f in &files {
        yakogl::shaderc::online::backend::forget_compiled();
        if let Err(m) = compile(stage_of(f), &read(f)) {
            failed.insert(f.clone(), m.to_string());
        }
    }
    let n_failed = failed.len();

    /* Links. An entry may hold several programs (ideas, shadow), and the
     * index does not say which vertex shader went with which fragment shader,
     * so every pair of an entry is tried and a shader must link with at least
     * one partner of each entry it appears in. */
    let mut tried: BTreeMap<(String, String), Result<(), String>> = BTreeMap::new();
    for (entry, (vss, fss)) in &by_entry {
        let mut vs_ok: BTreeMap<&String, bool> = vss.iter().map(|v| (v, false)).collect();
        let mut fs_ok: BTreeMap<&String, bool> = fss.iter().map(|f| (f, false)).collect();
        let mut why = Vec::new();
        for v in vss {
            for f in fss {
                if failed.contains_key(v) || failed.contains_key(f) {
                    continue;
                }
                let key = (v.clone(), f.clone());
                if !tried.contains_key(&key) {
                    yakogl::shaderc::online::backend::forget_compiled();
                    let vs = compile(SHC_STAGE_VERTEX, &read(v)).expect("compiled above");
                    let mut fs = compile(SHC_STAGE_FRAGMENT, &read(f)).expect("compiled above");
                    tried.insert(key.clone(), link(&vs, &mut fs).map_err(|e| e.to_string()));
                }
                match &tried[&key] {
                    Ok(()) => {
                        vs_ok.insert(v, true);
                        fs_ok.insert(f, true);
                    }
                    Err(m) => why.push(format!("{v}+{f}: {m}")),
                }
            }
        }
        for (n, ok) in vs_ok.iter().chain(fs_ok.iter()) {
            if !ok && !failed.contains_key(*n) {
                failed
                    .entry(n.to_string())
                    .or_insert_with(|| format!("links with nothing in `{entry}`: {}", why.join(" | ")));
            }
        }
    }
    let n_linked = tried.values().filter(|r| r.is_ok()).count();
    let pairs = by_entry.len();
    println!(
        "{BACKEND}: {} shaders, {} compiled, {n_failed} failed; {pairs} entries, {} pairs tried, {n_linked} linked",
        files.len(),
        files.len() - n_failed,
        tried.len()
    );
    for (f, m) in &failed {
        println!("  FAIL {f}: {m}");
    }
    assert!(failed.is_empty(), "{BACKEND}: shaders that do not compile or link: {failed:#?}");
}

/// The programs glmark2 linked, per entry, from the run's PowerVR logs: in
/// each `<n>.log` (entry `n` of `entries.txt`), a vertex shader's
/// `gm2: shader <id> vert <hash>` line and the fragment shader's line after
/// it. None when the logs are not beside the corpus or name a shader the
/// index lacks.
#[cfg(feature = "gpu-mali")]
fn logged_programs(dir: &Path, files: &[String]) -> Option<BTreeMap<(String, String), Vec<String>>> {
    let out = dir.parent()?;
    let entries = std::fs::read_to_string(out.join("entries.txt")).ok()?;
    let file_of = |stage: &str, hash: &str| {
        let tail = format!("-{hash}.{stage}.glsl");
        files.iter().find(|f| f.ends_with(&tail)).cloned()
    };
    let mut programs: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (i, entry) in entries.lines().enumerate() {
        let log = std::fs::read_to_string(out.join(format!("logs/powervr/{i}.log"))).ok()?;
        let mut vs: Option<String> = None;
        for l in log.lines().filter_map(|l| l.strip_prefix("gm2: shader ")) {
            let mut w = l.split_whitespace().skip(1);
            let (stage, hash) = (w.next()?, w.next()?);
            let f = file_of(stage, hash)?;
            if stage == "vert" {
                vs = Some(f);
            } else if let Some(v) = vs.take() {
                let e = programs.entry((v, f)).or_default();
                if !e.iter().any(|x| x == entry) {
                    e.push(entry.to_string());
                }
            }
        }
    }
    (!programs.is_empty()).then_some(programs)
}

/// One program built as glLinkProgram builds it on the Mali.
#[cfg(feature = "gpu-mali")]
fn mali_program(vs_src: &str, fs_src: &str) -> Result<(), Fail> {
    yakogl::shaderc::online::backend::forget_compiled();
    /* both stages' compiles, so that a refused vertex shader does not hide
     * the fragment shader's refusal */
    let (mut vs, mut fs) = match (compile(SHC_STAGE_VERTEX, vs_src), compile(SHC_STAGE_FRAGMENT, fs_src)) {
        (Ok(v), Ok(f)) => (v, f),
        (Err(e), Ok(_)) | (Ok(_), Err(e)) => return Err(e),
        (Err(a), Err(b)) => {
            let m = format!("{a} | {b}");
            return Err(if matches!((&a, &b), (Fail::Refused(_), Fail::Refused(_))) {
                Fail::Refused(m)
            } else {
                Fail::Error(m)
            });
        }
    };
    let relink = yakogl::shaderc::online::SHC_ONLINE.compile_vs_linked.expect("the online back end relinks");
    let src_z = format!("{vs_src}\0");
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    // SAFETY: a NUL-terminated source, no map, the program's fragment shader,
    // a real output and a log of the size announced.
    let rc = unsafe {
        relink(src_z.as_ptr() as *const c_char, core::ptr::null(), &fs, &mut out, log.as_mut_ptr(), log.len())
    };
    match rc {
        1 => {}
        0 => vs = out,
        _ => return Err(Fail::of(rc, &log)),
    }
    link(&vs, &mut fs)
}

#[cfg(feature = "gpu-mali")]
fn every_program_reported_on_the_mali() {
    let Some((dir, files, by_entry)) = corpus() else {
        return;
    };
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).expect("a shader file");
    let (programs, source) = match logged_programs(&dir, &files) {
        Some(p) => (p, "the run's logs"),
        None => {
            let mut p: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
            for (entry, (vss, fss)) in &by_entry {
                for v in vss {
                    for f in fss {
                        p.entry((v.clone(), f.clone())).or_default().push(entry.clone());
                    }
                }
            }
            (p, "every pair of an entry (no logs beside the corpus)")
        }
    };
    let mut verdict: BTreeMap<(String, String), Result<(), Fail>> = BTreeMap::new();
    for (v, f) in programs.keys() {
        let r = mali_program(&read(v), &read(f));
        match &r {
            Ok(()) => println!("mali: {v}+{f} ok"),
            Err(Fail::Refused(m)) => println!("mali: {v}+{f} refused: {m}"),
            Err(Fail::Error(m)) => println!("mali: {v}+{f} error: {m}"),
        }
        verdict.insert((v.clone(), f.clone()), r);
    }
    /* per entry, by name */
    let mut n_entries = 0;
    for entry in by_entry.keys() {
        let (mut ok, mut refused, mut error) = (0, 0, 0);
        for (key, es) in &programs {
            if es.contains(entry) {
                match &verdict[key] {
                    Ok(()) => ok += 1,
                    Err(Fail::Refused(_)) => refused += 1,
                    Err(Fail::Error(_)) => error += 1,
                }
            }
        }
        n_entries += 1;
        println!("mali-entry: {entry}: {} programs, {ok} ok, {refused} refused, {error} error", ok + refused + error);
    }
    let n_ok = verdict.values().filter(|r| r.is_ok()).count();
    let n_refused = verdict.values().filter(|r| matches!(r, Err(Fail::Refused(_)))).count();
    println!(
        "{BACKEND}: {} shaders; {n_entries} entries, {} programs ({source}): {n_ok} ok, {n_refused} refused, {} error",
        files.len(),
        verdict.len(),
        verdict.len() - n_ok - n_refused
    );
    assert_eq!(n_entries, 33, "index.txt names glmark2's 33 default entries");
}
