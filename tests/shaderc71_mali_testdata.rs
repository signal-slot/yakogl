// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The dEQP-GLES2 vertex/fragment pairs of `tools/mali/testdata` (the shapes
//! `tests/mali_deqp_*.rs` measure the Mali back end on) through the V3D online
//! back ends: the 4.2 one (Raspberry Pi 4, compile and link as
//! `tests/v3d_online_corpus.rs` does) and the 7.1 one (Raspberry Pi 5,
//! `chain71::compile_program71` as `tests/v3d71_online_corpus.rs` does).
//!
//! A pair is an `X.vert` and an `X.frag` in one directory of the tree (the top
//! level and every subdirectory). For each 7.1 program that compiles, every
//! word of its FS, VS and CS decodes with codec71 to its instruction and the
//! instructions pass verify71 and `registers_fit_threads71`.
//!
//! Four counts are pinned: pairs, compiled at 4.2, compiled at 7.1, and the
//! 7.1-only refusals (pairs 4.2 compiles and 7.1 refuses). Those are listed,
//! one `<key>: <exact message>` line each, in
//! `tools/shaders/known-uncompilable-v3d71-mali.txt` (the format of
//! known-uncompilable-v3d71.txt, the reason in `#` lines). The test fails on an
//! unlisted 7.1-only refusal, on a changed message, and on a listed entry that
//! now compiles.
//!
//! `tools/check.sh` runs it with "every tests/ target".
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_mali_testdata -- --nocapture
//! ```
#![cfg(all(feature = "gpu-v3d", feature = "shaderc-online"))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// `X.vert` + `X.frag` pairs of tools/mali/testdata.
const PAIRS: usize = 1556;
/// Of them, the pairs the 4.2 back end compiles and links.
const COMPILED_42: usize = 1552;
/// Of them, the pairs the 7.1 back end compiles.
const COMPILED_71: usize = 1553;
/// Pairs 4.2 compiles and 7.1 refuses (the lines of the list).
const ONLY_71_REFUSED: usize = 0;

fn pairs(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf, PathBuf)>) {
    for e in std::fs::read_dir(dir).expect("tools/mali/testdata is readable") {
        let p = e.expect("a directory entry").path();
        if p.is_dir() {
            pairs(&p, root, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("vert") {
            let f = p.with_extension("frag");
            if f.is_file() {
                let key = p.with_extension("");
                let key = key.strip_prefix(root).expect("under the root").to_string_lossy().into_owned();
                out.push((key, p, f));
            }
        }
    }
}

fn text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim_end().replace('\n', " / ")
}

fn compile42(stage: u32, src: &str) -> Result<ShcShader, String> {
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

fn ok42(vs: &str, fs: &str) -> bool {
    /* the linker only takes shaders this compiler holds: forget the rest first */
    yakogl::shaderc::online::backend::forget_compiled();
    let Ok(v) = compile42(SHC_STAGE_VERTEX, vs) else { return false };
    let Ok(mut f) = compile42(SHC_STAGE_FRAGMENT, fs) else { return false };
    let mut log = [0 as c_char; 1024];
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    unsafe { link(&v, &mut f, log.as_mut_ptr(), log.len()) == 0 }
}

fn check_stage(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
}

fn compile71(key: &str, vs: &str, fs: &str) -> Result<(), String> {
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, name, src) in [(Stage::Vertex, "vert", vs), (Stage::Fragment, "frag", fs)] {
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
        Ok(())
    })
    .expect("scratch")
}

fn listed() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders/known-uncompilable-v3d71-mali.txt");
    let list = std::fs::read_to_string(&path).expect("the list exists");
    let mut m = BTreeMap::new();
    for l in list.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (k, msg) = l.split_once(": ").expect("`<key>: <message>`");
        assert!(m.insert(k.to_string(), msg.to_string()).is_none(), "{k} is listed twice");
    }
    m
}

#[test]
fn every_mali_testdata_pair_compiles_at_7_1_or_is_a_listed_refusal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut all = Vec::new();
    pairs(&root, &root, &mut all);
    all.sort();
    let t0 = std::time::Instant::now();

    let (mut c42, mut c71) = (0usize, 0usize);
    let mut only71: BTreeMap<String, String> = BTreeMap::new();
    for (key, v, f) in &all {
        let vs = std::fs::read_to_string(v).expect("a UTF-8 source");
        let fs = std::fs::read_to_string(f).expect("a UTF-8 source");
        let a = ok42(&vs, &fs);
        let b = compile71(key, &vs, &fs);
        c42 += usize::from(a);
        c71 += usize::from(b.is_ok());
        if let (true, Err(m)) = (a, b) {
            only71.insert(key.clone(), m);
        }
    }
    println!(
        "mali testdata: {} pairs, {c42} compiled at 4.2, {c71} at 7.1, {} 7.1-only refusals, {:?}",
        all.len(),
        only71.len(),
        t0.elapsed()
    );
    for (k, m) in &only71 {
        println!("  {k}: {m}");
    }

    let known = listed();
    let mut bad = Vec::new();
    for (k, m) in &only71 {
        match known.get(k) {
            None => bad.push(format!("unlisted 7.1-only refusal: {k}: {m}")),
            Some(l) if l != m => bad.push(format!("message changed: {k}: listed `{l}`, now `{m}`")),
            Some(_) => {}
        }
    }
    for k in known.keys().filter(|k| !only71.contains_key(*k)) {
        bad.push(format!("listed but now compiles (remove it): {k}"));
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!(all.len(), PAIRS, "pairs");
    assert_eq!(c42, COMPILED_42, "compiled at 4.2");
    assert_eq!(c71, COMPILED_71, "compiled at 7.1");
    assert_eq!(only71.len(), ONLY_71_REFUSED, "7.1-only refusals");
}
