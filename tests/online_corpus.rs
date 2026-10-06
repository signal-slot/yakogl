// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every shader in `tools/shaders/` through the online compiler of this
//! build's GPU, and the exact message of every refusal.
//!
//! `tools/shaders/` is the corpus the offline tables were built from: gl_test,
//! the Slint and FemtoVG samples and the Qt sweeps. The online compiler is the
//! product path, so a source here that it refuses is a shader an application
//! hands us and gets back as a compile error. Run it per GPU with the output
//! visible:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test online_corpus -- --nocapture
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test online_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", any(feature = "gpu-powervr", feature = "gpu-vivante")))]
use core::ffi::c_char;
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

fn compile(stage: u32, src: &str) -> Result<(), String> {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect())
    }
}

/// The refusals this build's online compiler makes, one `<path>: <message>`
/// line each in path order: `tests/fixtures/online_refused_<GPU>.txt`. A
/// change to the set or to a message fails here and re-derives the file
/// (`ONLINE_CORPUS_WRITE=1` writes what this run saw, to be read and
/// committed).
/// The fixture's name: the GPU, and `_es2` where the build reports ES 2.0
/// and so refuses every `#version 300 es` source.
const GPU: &str = match (cfg!(feature = "gpu-powervr"), cfg!(feature = "report-es2")) {
    (true, false) => "powervr",
    (true, true) => "powervr_es2",
    (false, false) => "gc7000",
    (false, true) => "gc7000_es2",
};

#[test]
fn every_corpus_source_compiles_online_or_is_a_listed_refusal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    let mut refused = String::new();
    let mut n_refused = 0usize;
    for p in &files {
        let rel = p.strip_prefix(&root).expect("under the root").to_string_lossy().into_owned();
        let stage = if rel.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let text = std::fs::read_to_string(p).expect("a UTF-8 source");
        if let Err(msg) = compile(stage, &text) {
            /* one line per source: the log's lines joined */
            let msg = msg.trim_end().replace('\n', " / ");
            refused.push_str(&format!("{rel}: {msg}\n"));
            n_refused += 1;
        }
    }
    println!("{GPU}: {} sources, {} compiled, {n_refused} refused", files.len(), files.len() - n_refused);
    print!("{refused}");
    /* The population first: an empty walk would make "nothing refused" true. */
    assert_eq!(files.len(), 757, "tools/shaders holds 757 .vert/.frag sources; re-derive the pin if it changed");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/online_refused_{GPU}.txt"));
    if std::env::var_os("ONLINE_CORPUS_WRITE").is_some() {
        std::fs::write(&fixture, &refused).expect("the fixture is writable");
    }
    let want = std::fs::read_to_string(&fixture).expect("the fixture exists");
    assert!(want.lines().count() > 0, "the fixture lists the refusals; an empty one is not a pin");
    assert_eq!(
        refused,
        want,
        "the online refusals changed against {}; the run's list is printed above",
        fixture.display()
    );
}
