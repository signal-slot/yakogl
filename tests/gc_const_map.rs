// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000 constant file the online compiler describes names only
//! dwords the uniform block has.
//!
//! The file is laid out in whole vec4 registers, so a block that ends part
//! way through a register (femtovg's `uniform vec2 viewSize`; its fragment
//! block of 58 dwords) leaves padding lanes in the last one. Those lanes
//! were described as uniform dwords past the block, and the backend resolves
//! every constant on every draw (`shader.rs` `const_value`): each one logged
//! "uniform dword N missing", four lines a draw for every femtovg program.
//! That was 2.7 s of the i.MX8MP cluster's frame 2, all in the command-stream
//! step (records/imx8mp/drawstats-2026-10-05). The rule here is the general
//! one: every `SHC_VIV_CONST_UNIFORM` slot of every program names a dword
//! below the stage's `uniform_dwords`.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_char;
use std::path::{Path, PathBuf};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_VIV_CONST_UNIFORM};

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

/// The compiled program, or `None` for a source the compiler refuses
/// (online_corpus.rs owns the refusals and their messages).
fn compile(stage: u32, src: &str) -> Option<ShcShader> {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    (rc == 0).then_some(out)
}

/// Every UNIFORM slot past the block, as (slot, dword, block size).
fn past_the_block(sh: &ShcShader) -> Vec<(usize, u32, u16)> {
    let (contents, data) = (sh.viv.const_contents(), sh.viv.const_data());
    contents
        .iter()
        .zip(data)
        .enumerate()
        .filter(|(_, (c, d))| **c == SHC_VIV_CONST_UNIFORM && **d >= u32::from(sh.uniform_dwords))
        .map(|(i, (_, d))| (i, *d, sh.uniform_dwords))
        .collect()
}

#[test]
fn every_uniform_constant_names_a_dword_of_the_block() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    /* gl_test case 183's two stages: the board's reproducer of this */
    let suite = include_str!("../driver/tools/pvrlab/gl_test.c");
    let case183 = [
        (
            "gl_test 183 vertex",
            SHC_STAGE_VERTEX,
            "#version 100\nattribute vec2 a_pos;\nuniform vec2 u_scale;\n\
             void main() { gl_Position = vec4(a_pos * u_scale, 0.0, 1.0); }\n",
        ),
        (
            "gl_test 183 fragment",
            SHC_STAGE_FRAGMENT,
            "#version 100\nprecision mediump float;\nuniform vec2 u_rg;\n\
             void main() { gl_FragColor = vec4(u_rg, 0.0, 1.0); }\n",
        ),
    ];
    assert!(suite.contains("uniform vec2 u_scale;") && suite.contains("uniform vec2 u_rg;"), "case 183's text");
    let mut all: Vec<(String, u32, String)> = files
        .iter()
        .map(|p| {
            let stage = if p.extension().and_then(|x| x.to_str()) == Some("vert") {
                SHC_STAGE_VERTEX
            } else {
                SHC_STAGE_FRAGMENT
            };
            let name = p.strip_prefix(&root).unwrap().display().to_string();
            (name, stage, std::fs::read_to_string(p).expect("readable"))
        })
        .collect();
    all.extend(case183.iter().map(|(n, s, t)| (n.to_string(), *s, t.to_string())));
    let (mut compiled, mut partial, mut bad) = (0usize, 0usize, Vec::new());
    for (name, stage, text) in &all {
        let Some(sh) = compile(*stage, text) else { continue };
        compiled += 1;
        if sh.uniform_dwords % 4 != 0 {
            partial += 1;
        }
        let past = past_the_block(&sh);
        if !past.is_empty() {
            bad.push(format!("{name}: {past:?}"));
        }
    }
    println!(
        "{} sources, {compiled} compiled, {partial} with a block that ends part way through a register, {} naming a dword past it",
        all.len(),
        bad.len()
    );
    for b in bad.iter().take(10) {
        println!("  {b}");
    }
    /* the population, pinned: a corpus that grows, or a compiler that
     * starts or stops refusing a source, fails here so the pin is re-derived
     * (the refusals themselves are online_corpus.rs's); the rule is exercised
     * only by blocks that end part way through a register, and case 183 is
     * two of them */
    assert_eq!((all.len(), compiled), (720, 595), "sources and programs compiled: re-derive the pin");
    assert!(partial >= 2, "no block ends part way through a register: the rule is not exercised");
    assert!(bad.is_empty(), "{} programs name uniform dwords past their block (the first: {})", bad.len(), bad[0]);
}
