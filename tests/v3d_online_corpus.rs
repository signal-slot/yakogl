// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every shader in `tools/shaders/` through the V3D (Raspberry Pi 4) online
//! compiler, and the vertex/fragment pairs of the tree through its link step.
//!
//! `tests/online_corpus.rs` does this for the PowerVR and the Vivante; it has
//! no V3D case. This file is the V3D's. A source the V3D compiler refuses
//! would otherwise show up only on the board. The refusals this compiler is
//! known to make are in `tools/shaders/known-uncompilable-v3d.txt`, one
//! `<path>: <exact message>` line each (a link refusal is `<vert>+<frag>:
//! <message>`), with the reason in `#` lines above it. The test fails on a
//! refusal that is not listed, on a listed one whose message changed, and on a
//! listed one that now compiles, so the list stays exact.
//!
//! Links: a `X.vert` and an `X.frag` in one directory are one program. Only
//! pairs whose two sources both compile are linked.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_online_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

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

fn text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim_end().replace('\n', " / ")
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text(&log))
    }
}

fn link(vs: &ShcShader, fs: &mut ShcShader) -> Result<(), String> {
    let mut log = [0 as c_char; 1024];
    let f = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { f(vs, fs, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("rc {rc}: {}", text(&log)))
    }
}

/// `<key>: <message>` lines of the list; `#` lines and blank lines are prose.
fn listed() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders/known-uncompilable-v3d.txt");
    let list = std::fs::read_to_string(&path).expect("the V3D list exists");
    let mut m = BTreeMap::new();
    for l in list.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (k, msg) = l.split_once(": ").expect("`<path>: <message>`");
        assert!(m.insert(k.to_string(), msg.to_string()).is_none(), "{k} is listed twice");
    }
    m
}

#[test]
fn every_corpus_source_compiles_on_v3d_or_is_a_listed_refusal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    /* The population first: an empty walk would make "nothing refused" true. */
    assert_eq!(files.len(), 798, "tools/shaders holds 798 .vert/.frag sources; re-derive the pin if it changed");
    let rel_of = |p: &PathBuf| p.strip_prefix(&root).expect("under the root").to_string_lossy().into_owned();

    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    let mut ok: BTreeMap<String, String> = BTreeMap::new();
    for p in &files {
        let rel = rel_of(p);
        let stage = if rel.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let src = std::fs::read_to_string(p).expect("a UTF-8 source");
        yakogl::shaderc::online::backend::forget_compiled();
        match compile(stage, &src) {
            Ok(_) => {
                ok.insert(rel, src);
            }
            Err(m) => {
                refused.insert(rel, m);
            }
        }
    }
    let n_compiled = ok.len();

    /* `X.vert` + `X.frag` of one directory, both compiled */
    let mut n_pairs = 0usize;
    for (v, vsrc) in ok.iter().filter(|(k, _)| k.ends_with(".vert")) {
        let f = format!("{}.frag", v.trim_end_matches(".vert"));
        let Some(fsrc) = ok.get(&f) else { continue };
        n_pairs += 1;
        /* the linker only takes shaders this compiler holds: forget the rest first */
        yakogl::shaderc::online::backend::forget_compiled();
        let vs = compile(SHC_STAGE_VERTEX, vsrc).expect("compiled above");
        let mut fs = compile(SHC_STAGE_FRAGMENT, fsrc).expect("compiled above");
        if let Err(m) = link(&vs, &mut fs) {
            refused.insert(format!("{v}+{f}"), m);
        }
    }
    println!(
        "v3d: {} sources, {n_compiled} compiled, {} refused or unlinked, {n_pairs} pairs linked",
        files.len(),
        refused.len()
    );
    for (k, m) in &refused {
        println!("  {k}: {m}");
    }

    let want = listed();
    let mut bad = Vec::new();
    for (k, m) in &refused {
        match want.get(k) {
            None => bad.push(format!("NEW refusal, not listed: {k}: {m}")),
            Some(w) if w != m => bad.push(format!("message changed: {k}\n    listed: {w}\n    now:    {m}")),
            Some(_) => {}
        }
    }
    for k in want.keys().filter(|k| !refused.contains_key(*k)) {
        bad.push(format!("listed but now compiles/links (remove it): {k}"));
    }
    assert!(bad.is_empty(), "tools/shaders/known-uncompilable-v3d.txt is out of date:\n{}", bad.join("\n"));
}
