// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The four P8 shapes of docs/notes/pre-study-rpi5.md (pass-through,
//! uniform, `if`, texture; `tests/v3d71_common`) compiled at V3D 7.1 by our
//! own chain (`chain71::compile_program71`), with no Mesa in the loop.
//!
//! A shape that compiles is checked stage by stage: every word of the
//! fragment, vertex and coordinate shader decodes with codec71 and encodes
//! back to the same word, the instructions pass verify71 (with the stage's
//! `single_seg`) and `registers_fit_threads71` at the stage's thread count,
//! and the fragment shader runs in sim71 (`run71_stream`, the program's own
//! uniform stream resolved below) to the shape's colour in every lane.
//!
//! A shape that does not compile yet is an expected refusal pinned by its
//! exact error, never a skip: when the stage that refuses it lands, the pin
//! fails and is replaced by the checks above.
//!
//! The comparison with v3d_shc's 7.1 output for the same GLSL is not here
//! yet (plan #365 item corpus71 adds it).
//!
//! Feature set:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_shapes -- --nocapture
//! ```

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

mod v3d71_common;

use std::sync::Mutex;
use v3d71_common::*;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, V3d71Error, V3dProgram71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71, same_instr71};
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_CONSTANT, QUNIFORM_TMU_CONFIG_P0, QUNIFORM_TMU_CONFIG_P1, QUNIFORM_UNIFORM,
};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

/// What compiling a shape at 7.1 gave: the checked fragment shader's run, or
/// the refusal.
enum Got {
    Ran { fs_words: usize, threads: u32, out: Box<Outputs> },
    Refused(V3d71Error),
}

/// One stage's words: decode71 gives its instruction, encode71 gives the
/// word back, and verify71 and registers_fit_threads71 accept it.
fn check_stage(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
        assert_eq!(encode71(&back), Ok(w), "{what}: word {n} does not re-encode to itself");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
    assert!(c.back_edges.is_empty() && c.fwd_branches.is_empty(), "{what}: a branch in a straight-line program");
}

/// The fragment shader's uniform stream as the driver would write it for
/// the shape: a constant (the TLB configuration among them) and a TMU
/// configuration word (sim71 reads the texture index in P0's bits 24..31)
/// are their data, a `QUNIFORM_UNIFORM` is dword `data` of the shape's
/// uniform values. Anything else is not modelled here, by name.
fn stream(p: &V3dProgram71<'_>, shape: &Shape) -> Vec<u32> {
    p.fs.code
        .uniforms
        .iter()
        .map(|u| match u.contents {
            QUNIFORM_CONSTANT | QUNIFORM_TMU_CONFIG_P0 | QUNIFORM_TMU_CONFIG_P1 => u.data,
            QUNIFORM_UNIFORM => shape
                .uniforms
                .get(u.data as usize)
                .unwrap_or_else(|| panic!("{}: uniform dword {} past the shape's values", shape.name, u.data))
                .to_bits(),
            c => panic!("{}: uniform contents {c} not modelled by this test", shape.name),
        })
        .collect()
}

/// Compile `shape` at 7.1 and, when it compiles, check all three stages and
/// run the fragment shader.
fn compile(shape: &Shape) -> Got {
    /* the scratch region is one per process: the tests take turns */
    static SCRATCH: Mutex<()> = Mutex::new(());
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(shape.vs).unwrap()).map_err(|e| e.msg).unwrap();
        let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(shape.fs).unwrap()).map_err(|e| e.msg).unwrap();
        let v = &a.alloc_filled(1, v.module).unwrap()[0];
        let f = &a.alloc_filled(1, f.module).unwrap()[0];
        let p = match compile_program71(v, f, Prim::Triangles, TARGET71, a) {
            Ok(p) => p,
            Err(e) => return Got::Refused(e),
        };
        let name = shape.name;
        check_stage(&format!("{name} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage(&format!("{name} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage(&format!("{name} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
        assert!(!p.fs.single_seg, "{name}: a fragment shader starts in the first thread section");
        let mut s = shape.clone();
        s.threads = p.fs.threads;
        let out = run71_stream(p.fs.code.words, &stream(&p, &s), &s).unwrap_or_else(|e| panic!("{e}"));
        Got::Ran { fs_words: p.fs.code.words.len(), threads: p.fs.threads, out: Box::new(out) }
    })
    .expect("scratch")
}

/// The shape compiles at 7.1, passes every check and gives its colour.
fn compiles_and_runs(shape: Shape) {
    match compile(&shape) {
        Got::Ran { fs_words, threads, out } => {
            println!(
                "{}: fs {fs_words} words at {threads} threads, {} instructions run, {} TMU look-ups, lane 0 {:?}",
                shape.name, out.instructions, out.tmu_lookups, out.colour[0]
            );
            assert!(!out.approximate, "{}: sim71 approximated an operation", shape.name);
        }
        Got::Refused(e) => panic!("{}: refused at 7.1: {e:?}", shape.name),
    }
}

#[test]
fn passthrough_compiles_and_runs_at_7_1() {
    compiles_and_runs(passthrough());
}

#[test]
fn uniform_compiles_and_runs_at_7_1() {
    compiles_and_runs(uniform());
}

/// select.rs if-converts the shape's `if` (both arms are constant colour
/// writes): the selection has no branch, so it is straight-line code and
/// pack71 takes it (check_stage asserts no branch).
#[test]
fn if_compiles_and_runs_at_7_1() {
    compiles_and_runs(if_branch());
}

#[test]
fn texture_compiles_and_runs_at_7_1() {
    compiles_and_runs(texture());
}
