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
//! All four compile: a refusal of any of them fails its test.
//!
//! Then each shape's fragment shader is compared with Mesa's for the same
//! GLSL (`v3d_shc --device 71`, `tests/v3d71_ref`) by `tests/v3d71_compare`:
//! both programs, each with its own uniform stream built from the shape's
//! values, run in sim71 on the shape's own inputs and on random ones, and
//! must give the same TLB writes, VPM rows and multisample flags. The
//! instruction, thread-switch, ldvary, uniform and TMU counts are printed,
//! not enforced. Without `V3D_SHC` / `MESA_V3D_TREE` (tools/v3d/README.md),
//! or without the tools the reference needs, each shape prints
//! `gap: V3D_SHC ...` naming what is missing and compares nothing: a gap,
//! never a pass.
//!
//! Feature set:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_shapes -- --nocapture
//! ```

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

mod v3d71_common;
mod v3d71_compare;
mod v3d71_ref;

use std::sync::Mutex;
use v3d71_common::*;
use v3d71_compare::{compare, random_fs_cases, Case, Policy, Side};
use v3d71_ref::{compile71, find_v3d_shc};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, V3d71Error, V3dProgram71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71, same_instr71};
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_CONSTANT, QUNIFORM_TMU_CONFIG_P0, QUNIFORM_TMU_CONFIG_P1, QUNIFORM_UNIFORM,
};
use yakogl::shaderc::online::v3d::sim::{Env, FsInputs, Inputs, Sampler, Varying, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

/// What compiling a shape at 7.1 gave: the checked fragment shader's run, or
/// the refusal.
enum Got {
    /// `fs`: the fragment shader's words and its resolved uniform stream.
    Ran {
        fs: Side,
        threads: u32,
        out: Box<Outputs>,
    },
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
        let fs = Side { words: p.fs.code.words.to_vec(), uniforms: stream(&p, &s) };
        let out = run71_stream(&fs.words, &fs.uniforms, &s).unwrap_or_else(|e| panic!("{e}"));
        Got::Ran { fs, threads: p.fs.threads, out: Box::new(out) }
    })
    .expect("scratch")
}

/// The shape's own fragment inputs (W 1, the shape's varyings: what
/// `v3d71_common::run71_stream` runs) and four random cases with the
/// shape's varying count, all with the shape's texture.
fn cases(shape: &Shape) -> Vec<Case> {
    let one = 1.0f32.to_bits();
    let env = Env {
        textures: shape.texture.iter().cloned().collect(),
        samplers: shape.texture.iter().map(Sampler::of).collect(),
        threads: 1,
        ..Env::default()
    };
    let own = FsInputs {
        w: [one; LANES],
        w_centroid: [one; LANES],
        z: [0.5f32.to_bits(); LANES],
        x: core::array::from_fn(|l| ((l % 4) as f32 + 0.5).to_bits()),
        y: core::array::from_fn(|l| ((l / 4) as f32 + 0.5).to_bits()),
        back_facing: [false; LANES],
        varyings: shape.varyings.iter().map(|v| Varying { vary: v.map(f32::to_bits), c: [0; LANES] }).collect(),
    };
    let mut out = vec![Case { name: "own".into(), inputs: Inputs::Fragment(Box::new(own)), env: env.clone() }];
    for mut c in random_fs_cases(0x71 + shape.name.len() as u64, shape.varyings.len(), 4) {
        c.env = env.clone();
        out.push(c);
    }
    out
}

/// Our fragment shader against Mesa's 7.1 one for the same GLSL, or the
/// named gap (printed, nothing compared).
fn compare_with_mesa(shape: &Shape, ours: &Side) {
    let name = shape.name;
    let shc = match find_v3d_shc() {
        Ok(p) => p,
        Err(g) => return println!("gap: V3D_SHC v3d71_shapes {name}: {}", g.0),
    };
    let m = match compile71(&shc, shape) {
        Err(g) => return println!("gap: V3D_SHC v3d71_shapes {name}: {}", g.0),
        Ok(Err(e)) => panic!("{name}: Mesa's 7.1 reference did not compile: {e}"),
        Ok(Ok(m)) => m,
    };
    assert_eq!(m.ver, 71, "{name}: v3d_shc --device 71 reports gpu.ver {}", m.ver);
    let mesa = Side { words: m.fs.code.clone(), uniforms: m.fs_stream(shape).unwrap_or_else(|e| panic!("{e}")) };
    let v = compare(ours, &mesa, &cases(shape), Policy::default());
    let (o, t) = (v.ours, v.mesa);
    println!(
        "compare v3d_shc 7.1 {name}: {} on {} cases; fs ours/mesa: instructions {}/{}, thrsw {}/{}, \
         ldvary {}/{}, uniforms {}/{}, TMU {}/{}",
        if v.matches() { "same outputs" } else { "DIFFERENT outputs" },
        v.cases_run,
        o.instructions,
        t.instructions,
        o.thrsw,
        t.thrsw,
        o.ldvary,
        t.ldvary,
        o.uniform_len,
        t.uniform_len,
        o.tmu_accesses,
        t.tmu_accesses
    );
    assert!(v.matches(), "{name}: our 7.1 fs and Mesa's differ:\n{}", v.diffs.join("\n"));
}

/// The shape compiles at 7.1, passes every check, gives its colour and,
/// with the reference, Mesa's outputs.
fn compiles_and_runs(shape: Shape) {
    match compile(&shape) {
        Got::Ran { fs, threads, out } => {
            println!(
                "{}: fs {} words at {threads} threads, {} instructions run, {} TMU look-ups, lane 0 {:?}",
                shape.name,
                fs.words.len(),
                out.instructions,
                out.tmu_lookups,
                out.colour[0]
            );
            assert!(!out.approximate, "{}: sim71 approximated an operation", shape.name);
            compare_with_mesa(&shape, &fs);
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
