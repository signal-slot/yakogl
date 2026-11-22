// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The dEQP-GLES2 shaders kept in `tests/deqp_repro_shaders.rs` (random.*,
//! loops, uniform_api.random.23; dEQP is Apache-2.0) through the V3D 7.1
//! (Raspberry Pi 5) online back end. That file is cfg'd to gpu-vivante, so
//! it never meets V3D; its string constants are read here at run time (not
//! copied) and each vertex/fragment pair goes through
//! `chain71::compile_program71` (and `chain::compile_program` at 4.2 for the
//! comparison). Every 7.1 word of a program that compiles decodes with
//! codec71 (`same_instr71`) and passes verify71, `registers_fit_threads71`
//! and `branches_are_structured71`.
//!
//! Only RANDOM_AF_FRAG64 has a vertex shader in the file; the other two
//! fragment shaders are paired with a one-line stub vertex shader that writes
//! their varying from a uniform. The inline snippets in the file's test body
//! (ES 3.00 four outputs, gl_PointCoord) are not constants and not read.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_deqp_repro -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::path::Path;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::{self, Prim};
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::sched71_cf::branches_are_structured71;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

/// Programs: RANDOM_AF_FRAG64 (its own VS), LOOPS_DO_WHILE_ONLY_CONTINUE_FS
/// and UNIFORM_API_RANDOM_23_FS (stub VS).
const PROGRAMS: usize = 3;
/// Compile at 4.2.
const COMPILE_42: usize = 3;
/// Compile at 7.1.
const COMPILE_71: usize = 3;

/// Refusals, by program and exact message (a comment with the reason beside
/// each). None today: random.all_features.fragment.64's FS (660 values, past
/// the old 512-value bound) and uniform_api.random.23 (10.9 KB) both compile
/// at 4.2 and 7.1 here. A refusal that returns is added with its message.
const REFUSED_42: [(&str, &str); 0] = [];
const REFUSED_71: [(&str, &str); 0] = [];

const STUB_LOOPS_VS: &str =
    "attribute vec4 p; uniform vec4 u; varying mediump vec4 v_coords; void main() { gl_Position = p; v_coords = u; }\n";
const STUB_UNIFORM_VS: &str =
    "attribute vec4 p; uniform float u; varying mediump float v_vtxOut; void main() { gl_Position = p; v_vtxOut = u; }\n";

/// The constant `name`'s text: the repro file's `const NAME: &str = r#"..."#;`
/// or `"..."` literal (the latter without escapes: none are used).
fn constant(file: &str, name: &str) -> String {
    let head = format!("const {name}: &str = ");
    let at = file.find(&head).unwrap_or_else(|| panic!("{name} not in deqp_repro_shaders.rs")) + head.len();
    let rest = &file[at..];
    if let Some(raw) = rest.strip_prefix("r#\"") {
        raw[..raw.find("\"#;").expect("raw string end")].to_string()
    } else {
        let body = rest.strip_prefix('"').expect("a string literal");
        let end = body.find("\";").expect("string end");
        assert!(!body[..end].contains('\\'), "{name}: escapes are not handled");
        body[..end].to_string()
    }
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
    assert_eq!(branches_are_structured71(c.instrs, c.back_edges, c.fwd_branches), Ok(()), "{what}: branches");
}

static SCRATCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// (4.2 result, 7.1 result): None for a compile, Some(message) for a refusal.
fn run(name: &str, vs: &str, fs: &str) -> (Option<String>, Option<String>) {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs), (Stage::Fragment, fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => {
                    let msg = format!("front end: {}", e.msg);
                    return (Some(msg.clone()), Some(msg));
                }
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let r42 = chain::compile_program(v, f, Prim::Triangles, TARGET71, a).err().map(|e| format!("{e:?}"));
        let r71 = match compile_program71(v, f, Prim::Triangles, TARGET71, a) {
            Ok(p) => {
                check_stage(&format!("{name} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
                check_stage(&format!("{name} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
                check_stage(&format!("{name} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
                None
            }
            Err(e) => Some(format!("{e:?}")),
        };
        (r42, r71)
    })
    .expect("scratch")
}

#[test]
fn the_deqp_repro_shaders_at_42_and_71() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/deqp_repro_shaders.rs");
    let file = std::fs::read_to_string(path).expect("tests/deqp_repro_shaders.rs");
    let progs = [
        ("RANDOM_AF_FRAG64", constant(&file, "RANDOM_AF_FRAG64_VS"), constant(&file, "RANDOM_AF_FRAG64_FS")),
        ("LOOPS_DO_WHILE_ONLY_CONTINUE", STUB_LOOPS_VS.to_string(), constant(&file, "LOOPS_DO_WHILE_ONLY_CONTINUE_FS")),
        ("UNIFORM_API_RANDOM_23", STUB_UNIFORM_VS.to_string(), constant(&file, "UNIFORM_API_RANDOM_23_FS")),
    ];
    let (mut r42, mut r71) = (Vec::new(), Vec::new());
    let (mut ok42, mut ok71) = (0, 0);
    for (name, vs, fs) in &progs {
        let (a, b) = run(name, vs, fs);
        println!("{name}: 4.2 {} / 7.1 {}", a.as_deref().unwrap_or("ok"), b.as_deref().unwrap_or("ok"));
        match a {
            None => ok42 += 1,
            Some(m) => r42.push((*name, m)),
        }
        match b {
            None => ok71 += 1,
            Some(m) => r71.push((*name, m)),
        }
    }
    println!("programs {}, compile at 4.2 {ok42}, at 7.1 {ok71}", progs.len());
    assert_eq!(progs.len(), PROGRAMS);
    let got42: Vec<(&str, &str)> = r42.iter().map(|(n, m)| (*n, m.as_str())).collect();
    let got71: Vec<(&str, &str)> = r71.iter().map(|(n, m)| (*n, m.as_str())).collect();
    assert_eq!(ok42, COMPILE_42, "4.2: {r42:?}");
    assert_eq!(ok71, COMPILE_71, "7.1: {r71:?}");
    assert_eq!(got42, REFUSED_42);
    assert_eq!(got71, REFUSED_71);
}
