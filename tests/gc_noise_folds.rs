// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000 online compiler's two folds on glmark2 terrain's noise shader,
//! pinned as properties, not counts (plan #421, item noise-fold-guards):
//!
//! * a negate rides on its reader as a source modifier: no `mov` whose only
//!   job is to copy one negated register source (the sr patch, `3ac0d82d`);
//! * a division by a constant multiplies by its folded reciprocal: no
//!   run-time `rcp` of a constant or a uniform-constant (float constant
//!   folding in the GC selection, `6e498505`).
//!
//! Counts are `tests/gc_terrain_shader_count.rs`'s; later passes (mad fusion,
//! copy propagation) move them and must not have to touch this file.
//!
//! The noise text (`terrain-noise.frag`) is GPL-3.0 and never committed: it
//! is built from a glmark2 checkout (`GLMARK2_SRC`, its `data/shaders`) as in
//! `docs/notes/gc-terrain-shaders-rest.md`'s `gen_terrain.py`. With no
//! `GLMARK2_SRC` that half prints a SKIP line. The two micro-shaders are this
//! repository's own and always run.
//!
//! ```text
//! GLMARK2_SRC=... cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_noise_folds -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use std::path::Path;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// The program of one compile, one listing line per instruction.
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

/// The opcode column of a listing line (`008 [w w w w] add  t1, ...`), without
/// its modifiers.
fn opcode(line: &str) -> &str {
    line.split(']').nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("").split('.').next().unwrap_or("")
}

/// The operands of a listing line, after the opcode.
fn operands(line: &str) -> Vec<&str> {
    let rest = line.split(']').nth(1).unwrap_or("");
    rest.trim_start()
        .split_once(char::is_whitespace)
        .map_or("", |x| x.1)
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "void")
        .collect()
}

/// A temporary source with a negate modifier (`-t3.xxxx`). An immediate such as
/// `-1.0:f20` and a negated uniform (`-u0.xxxx`, a literal constant read out of
/// the constant file) are values, not a negate of a computed result.
fn is_negated_reg(o: &str) -> bool {
    o.strip_prefix('-').is_some_and(|r| r.starts_with('t'))
}

/// The instructions that are a `mov` of one negated register source.
fn negate_only_movs(prog: &[String]) -> Vec<&String> {
    prog.iter()
        .filter(|l| opcode(l) == "mov" && operands(l).iter().skip(1).filter(|o| is_negated_reg(o)).count() == 1)
        .collect()
}

/// The `rcp`s whose source is a constant: an immediate or a uniform register
/// (`u..`), not a temporary.
fn rcps_of_constants(prog: &[String]) -> Vec<&String> {
    prog.iter()
        .filter(|l| {
            opcode(l) == "rcp"
                && operands(l).iter().skip(1).any(|o| {
                    let o = o.trim_start_matches(['-', '|']);
                    o.starts_with('u') || o.contains(":f20") || o.contains(":s20") || o.contains(":u20")
                })
        })
        .collect()
}

fn show(name: &str, prog: &[String]) {
    println!("gc7000 {name}: {} instructions", prog.len());
    for l in prog {
        println!("  {l}");
    }
}

const HEAD: &str = "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\n";

fn fs(body: &str) -> String {
    format!("{HEAD}void main() {{ {body} }}\n")
}

/// A negate used by a reader is the reader's source modifier: `-a.x * b.y`
/// and `-a.x + b.y` leave no `mov` that only negates.
fn a_negate_rides_on_its_reader() {
    for (name, body) in [
        ("mul", "gl_FragColor = vec4(-a.x * b.y);"),
        ("add", "gl_FragColor = vec4(-a.x + b.y);"),
        ("vec", "gl_FragColor = vec4(-a.x * b.y, -a.y * b.z, -a.z * b.w, -a.w * b.x);"),
    ] {
        let p = compile(&fs(body));
        show(&format!("negate {name}"), &p);
        let bad = negate_only_movs(&p);
        assert!(bad.is_empty(), "negate {name}: a mov only negates: {bad:?}");
    }
}

/// `x / 7.0` multiplies by the folded reciprocal: a `mul`, no `rcp`.
fn a_constant_divisor_is_a_mul_by_the_folded_reciprocal() {
    for (name, body) in [
        ("literal", "gl_FragColor = vec4(a.x / 7.0);"),
        ("vec", "gl_FragColor = a / 7.0;"),
        ("const", "const float N = 7.0; gl_FragColor = vec4(a.x / N);"),
    ] {
        let p = compile(&fs(body));
        show(&format!("divide {name}"), &p);
        assert_eq!(p.iter().filter(|l| opcode(l) == "rcp").count(), 0, "divide {name}: an rcp is emitted");
        assert!(p.iter().any(|l| opcode(l) == "mul"), "divide {name}: no mul by the reciprocal");
    }
}

/// `libmatrix/shader-source.cc`'s two precision macros and the default
/// fragment precision `ShaderSource` adds.
const PRELUDE: &str =
    "#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n#define HIGHP_OR_DEFAULT highp\n#else\n\
#define HIGHP_OR_DEFAULT\n#endif\n#if defined(GL_ES)\n#define MEDIUMP_OR_DEFAULT mediump\n#else\n\
#define MEDIUMP_OR_DEFAULT\n#endif\n#ifdef GL_ES\nprecision mediump float;\n#endif\n";

/// The noise shader keeps both folds, whatever its length.
fn the_noise_shader_keeps_both_folds() {
    let Some(root) = std::env::var_os("GLMARK2_SRC") else {
        eprintln!("SKIP gc_noise_folds noise half: GLMARK2_SRC (a glmark2 checkout) is not set");
        return;
    };
    let path = Path::new(&root).join("data/shaders/terrain-noise.frag");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = compile(&format!("{PRELUDE}{text}"));
    println!("gc7000 noise.frag: {} instructions", p.len());
    let neg = negate_only_movs(&p);
    assert!(neg.is_empty(), "{} movs only negate, first: {:?}", neg.len(), neg.first());
    let rcp = rcps_of_constants(&p);
    assert!(rcp.is_empty(), "{} rcps of a constant, first: {:?}", rcp.len(), rcp.first());
}

/// One test: the online compiler's scratch arena is not reentrant, and the
/// test harness runs `#[test]`s on parallel threads.
#[test]
fn gc_noise_folds() {
    a_negate_rides_on_its_reader();
    a_constant_divisor_is_a_mul_by_the_folded_reciprocal();
    the_noise_shader_keeps_both_folds();
}
