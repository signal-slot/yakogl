// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000's constant-count loop unrolling (`select.rs` `unrolled`, the
//! shared `powervr/unroll.rs` under `Rules::Counted`): a constant `for` loop
//! of 1 to 32 passes that leaves by its head test alone is expanded before
//! selection, as Mesa etnaviv's `nir_opt_loop_unroll` does
//! (`max_unroll_iterations = 32`), and a uniform array read at the index it
//! then knows is a constant-file read. Every other loop stays a loop with
//! select.rs `loop_convert`'s guard. Rank 1 of
//! `docs/notes/gc-terrain-shaders-rest.md` (plan #328, item
//! x2-tiled-probe-run).
//!
//! Through the public compile path for the GC (`SHC_ONLINE.compile`, what
//! `glCompileShader` calls); a loop shows in the program as its back edge,
//! a `branch` to an earlier instruction.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_unroll -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// The program of one fragment compile: its instructions in listing form.
fn compile(src: &str) -> Vec<String> {
    yakogl::shaderc::online::backend::forget_compiled();
    let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(SHC_STAGE_FRAGMENT, text.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "compile failed: {msg}");
    // SAFETY: the compile filled `usc` with `usc_dw` dwords.
    let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    words
        .chunks(4)
        .enumerate()
        .map(|(i, c)| {
            let ins = decode::decode_instr(&c.try_into().expect("four dwords")).expect("decodes");
            let mut t = String::new();
            listing::instr_text(&mut t, i, &ins).expect("lists");
            t
        })
        .collect()
}

/// The opcode column of a listing line (`008 [w w w w] add  t1, ...`).
fn opcode(line: &str) -> &str {
    line.split(']').nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("")
}

fn count(prog: &[String], op: &str) -> usize {
    prog.iter().filter(|l| opcode(l).split('.').next() == Some(op)).count()
}

const HEAD: &str = "precision mediump float;\nvarying vec4 a;\n";

fn fs(decl: &str, body: &str) -> String {
    format!("{HEAD}{decl}void main() {{ {body} }}\n")
}

fn show(name: &str, prog: &[String]) {
    println!("gc7000 {name}: {} instructions", prog.len());
    for l in prog {
        println!("  {l}");
    }
}

/// t1 of `gc_terrain_shader_count.rs`: a one-iteration loop over `uniform
/// vec4 P[1]`. Unrolled, `P[i]` is `P[0]`: one `add` of the constant
/// register (and at most the output copy). It was 19 with the loop and the
/// run-time array read (`imullo0`, `load`, the clamp), 23 before the sr
/// patch.
fn a_one_iteration_loop_over_a_uniform_array_is_one_add() {
    let p = compile(&fs(
        "uniform vec4 P[1];\n",
        "vec4 c = a; for (int i = 0; i < 1; i++) { c += P[i]; } gl_FragColor = c;",
    ));
    show("t1", &p);
    assert!(p.len() <= 2, "t1 is 1 or 2 instructions, got {}", p.len());
    assert_eq!(count(&p, "branch"), 0, "no loop left");
    assert_eq!(count(&p, "load") + count(&p, "imullo0"), 0, "the element is a constant-file read");
}

/// 32 passes, etnaviv's limit: expanded, no branch, every element of the
/// array read from the constant file.
fn a_32_iteration_loop_is_unrolled() {
    let p = compile(&fs(
        "uniform vec4 P[32];\n",
        "vec4 c = a; for (int i = 0; i < 32; i++) { c += P[i]; } gl_FragColor = c;",
    ));
    println!("gc7000 32 passes: {} instructions", p.len());
    assert_eq!(count(&p, "branch"), 0, "no loop left");
    assert_eq!(count(&p, "load") + count(&p, "imullo0"), 0, "no run-time array read");
    assert_eq!(count(&p, "add"), 32, "one add a pass");
}

/// 33 passes, one past the limit: still a loop, its back edge and guard in
/// place, and the array read at run time.
fn a_33_iteration_loop_stays_a_loop() {
    let p = compile(&fs(
        "uniform vec4 P[33];\n",
        "vec4 c = a; for (int i = 0; i < 33; i++) { c += P[i]; } gl_FragColor = c;",
    ));
    show("33 passes", &p);
    assert!(count(&p, "branch") >= 2, "the loop's branches are there");
    assert_eq!(count(&p, "load"), 1, "P[i] is read at run time");
}

/// A count read from a uniform is no constant: the loop stays a loop.
fn a_loop_with_a_non_constant_count_stays_a_loop() {
    let p = compile(&fs(
        "uniform vec4 P[4];\nuniform int n;\n",
        "vec4 c = a; for (int i = 0; i < 4; i++) { if (i >= n) break; c += P[i]; } gl_FragColor = c;",
    ));
    show("break on a uniform", &p);
    assert!(count(&p, "branch") >= 2, "the loop's branches are there");
    let q = compile(&fs(
        "uniform vec4 P[4];\nuniform float n;\n",
        "vec4 c = a; for (float x = 0.0; x < n; x += 1.0) { c += P[0]; } gl_FragColor = c;",
    ));
    show("a uniform bound", &q);
    assert!(count(&q, "branch") >= 2, "the loop's branches are there");
}

/// One test: the binary resets the compiler (`forget_compiled`), so it holds
/// one (src/test_rules.rs).
#[test]
fn constant_loops_of_1_to_32_passes_are_unrolled_on_the_gc() {
    a_one_iteration_loop_over_a_uniform_array_is_one_add();
    a_32_iteration_loop_is_unrolled();
    a_33_iteration_loop_stays_a_loop();
    a_loop_with_a_non_constant_count_stays_a_loop();
}
