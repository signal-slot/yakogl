// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every shader glmark2 passes to `glShaderSource` (tools/shaders/glmark2/)
//! through this build's online compiler, and the vertex/fragment pairs glmark2
//! links through its link step.
//!
//! The expected refusals are `tools/shaders/glmark2/known-uncompilable.txt`
//! (`<shader file> <backend> <reason>`); a failure not listed fails the test,
//! and so does a listed entry that now compiles. Per back end:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test glmark2_corpus -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test glmark2_corpus -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test glmark2_corpus -- --nocapture
//! ```
//!
//! Links: every vertex/fragment pair of one index entry is tried, and a shader
//! that links with no partner in its entry fails (named by its own file in the
//! list). A shader that does not compile is not linked.
#![cfg(all(feature = "shaderc-online", any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::path::Path;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const BACKEND: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "gc7000"
} else {
    "v3d"
};

fn text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim().replace('\n', " / ")
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text_z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text_z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(format!("rc {rc}: {}", text(&log)))
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

#[test]
fn every_glmark2_shader_compiles_or_is_listed() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders/glmark2");
    let index = std::fs::read_to_string(dir.join("index.txt")).expect("index.txt");
    /* the files, and per entry its vertex and fragment files */
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
    assert_eq!(files.len(), 69, "index.txt lists 69 shaders; re-derive the pin if it changed");

    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    let list = std::fs::read_to_string(dir.join("known-uncompilable.txt")).expect("known-uncompilable.txt");
    for l in list.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut p = l.splitn(3, ' ');
        let (f, b, why) = (p.next().unwrap(), p.next().expect("a backend"), p.next().expect("a reason"));
        assert!(matches!(b, "powervr" | "gc7000" | "v3d"), "unknown backend in `{l}`");
        if b == BACKEND {
            assert!(listed.insert(f.to_string(), why.to_string()).is_none(), "`{f}` listed twice");
        }
    }

    let read = |n: &str| std::fs::read_to_string(dir.join(n)).expect("a shader file");
    let stage_of = |n: &str| if n.contains(".vert.") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
    let mut failed: BTreeMap<String, String> = BTreeMap::new();
    for f in &files {
        yakogl::shaderc::online::backend::forget_compiled();
        if let Err(m) = compile(stage_of(f), &read(f)) {
            failed.insert(f.clone(), m);
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
                    tried.insert(key.clone(), link(&vs, &mut fs));
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
    let unlisted: Vec<_> = failed.iter().filter(|(f, _)| !listed.contains_key(*f)).collect();
    let stale: Vec<_> = listed.keys().filter(|f| !failed.contains_key(*f)).collect();
    assert!(unlisted.is_empty(), "{BACKEND}: unlisted failures (fix, or add to known-uncompilable.txt): {unlisted:#?}");
    assert!(stale.is_empty(), "{BACKEND}: listed but now fine (remove from known-uncompilable.txt): {stale:#?}");
}
