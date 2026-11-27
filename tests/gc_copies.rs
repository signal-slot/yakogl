// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The rest of rank 2 of `docs/notes/gc-terrain-shaders-rest.md` (plan #421
//! item cc-copies, as designed by item design-copies) on the GC7000 online
//! compiler, one micro shader a class. Each copy named below is in the
//! listing before the change (counts in the comments, measured on the tree
//! without it) and gone after, and the host interpreter's colour is the
//! value the GLSL computes:
//!
//! - **R** (`peephole.rs` `retarget_producers`): a dot product or a
//!   reciprocal computed into a temporary and copied into one lane writes the
//!   lane itself;
//! - **P** (`peephole.rs` `fold_copies`): `vec2 c = uv;`, a partial copy that
//!   is the only writer of `c`, is read through by the whole-vec4 adds of the
//!   texture coordinates;
//! - **B** (`alloc.rs` `copy_source_dying_here`): the colour is placed in the
//!   register of the vec4 it copies, which dies there, so the copy is a
//!   self-copy and goes.
//!
//! The refusals of each class are the unit tests beside the code.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_copies -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_VIV_CONST_LITERAL, SHC_VIV_NO_REG};

mod common;
use common::{run, Run};

/// One compile: the decoded instructions, the constant file (literals as
/// they are, every other entry 0) and the colour's register.
struct Compiled {
    code: Vec<Instr>,
    consts: Vec<[u32; 4]>,
    colour: u8,
}

fn compile_raw(stage: u32, src: &str) -> Compiled {
    yakogl::shaderc::online::backend::forget_compiled();
    let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "compile failed: {msg}");
    // SAFETY: the compile filled `usc` with `usc_dw` dwords.
    let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    let code =
        words.chunks(4).map(|c| decode::decode_instr(&c.try_into().expect("four dwords")).expect("decodes")).collect();
    let n = sh.viv.const_count as usize;
    let mut consts = vec![[0u32; 4]; n.div_ceil(4).max(1)];
    if n > 0 {
        // SAFETY: the shader's own count describes both regions.
        let kinds = unsafe { core::slice::from_raw_parts(sh.viv.const_contents, n) };
        let data = unsafe { core::slice::from_raw_parts(sh.viv.const_data, n) };
        for k in 0..n {
            if kinds[k] == SHC_VIV_CONST_LITERAL {
                consts[k / 4][k % 4] = data[k];
            }
        }
    }
    if stage == SHC_STAGE_FRAGMENT {
        assert_ne!(sh.viv.ps_color_out_reg, SHC_VIV_NO_REG, "a fragment shader writes the colour");
    }
    Compiled { code, consts, colour: sh.viv.ps_color_out_reg }
}

/// The program of one compile: its instructions in listing form.
fn compile(stage: u32, src: &str) -> Vec<String> {
    compile_raw(stage, src)
        .code
        .iter()
        .enumerate()
        .map(|(i, ins)| {
            let mut t = String::new();
            listing::instr_text(&mut t, i, ins).expect("lists");
            t
        })
        .collect()
}

/// The colour a fragment program of [`HEAD`] writes for varyings `a` (t1)
/// and `b` (t2), on the host interpreter (`tests/common`).
fn colour_of(c: &Compiled, a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let mut init = vec![[0u32; 4]; 64];
    init[1] = a.map(f32::to_bits);
    init[2] = b.map(f32::to_bits);
    match run(&c.code, &init, &c.consts) {
        Run::Done { regs, .. } => regs[c.colour as usize].map(f32::from_bits),
        Run::Unsupported(why) => panic!("the interpreter cannot run it: {why}"),
    }
}

fn close(got: [f32; 4], want: [f32; 4], what: &str) {
    for l in 0..4 {
        assert!(
            (got[l] - want[l]).abs() <= 1e-6 * want[l].abs().max(1.0),
            "{what}: lane {l} is {} and should be {} (got {got:?}, want {want:?})",
            got[l],
            want[l]
        );
    }
}

/// The opcode column of a listing line (`008 [w w w w] add  t1, ...`).
fn opcode(line: &str) -> &str {
    line.split(']').nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("")
}

fn count(prog: &[String], op: &str) -> usize {
    prog.iter().filter(|l| opcode(l).split('.').next() == Some(op)).count()
}

const HEAD: &str = "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\nuniform sampler2D T;\n";

fn fs(decl: &str, body: &str) -> String {
    format!("{HEAD}{decl}void main() {{ {body} }}\n")
}

fn show(name: &str, prog: &[String]) {
    println!("gc7000 {name}: {} instructions", prog.len());
    for l in prog {
        println!("  {l}");
    }
}

/// A `mov` reading a temporary (not a constant, not a negate): a copy.
fn copies(prog: &[String]) -> usize {
    prog.iter()
        .filter(|l| opcode(l) == "mov" && l.rsplit(", ").next().is_some_and(|s| s.starts_with('t') && !s.contains(':')))
        .count()
}

const A: [f32; 4] = [0.5, -1.25, 2.0, 0.75];
const B: [f32; 4] = [1.5, 4.0, -0.5, 3.0];

fn dot(x: [f32; 4], y: [f32; 4]) -> f32 {
    x.iter().zip(y).map(|(p, q)| p * q).sum()
}

/// R: `vec4(dot(a, a), dot(b, b), dot(a, b), 1.0)` is three `dp4`, each
/// writing its lane of the colour, and the constant lane; 7 before (two lane
/// copies and the colour copy).
fn r_dot_products_write_their_lanes() {
    let src = fs("", "gl_FragColor = vec4(dot(a, a), dot(b, b), dot(a, b), 1.0);");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("R dots", &p);
    assert_eq!(p.len(), 4, "three dp4 and the 1.0 lane (7 before)");
    assert_eq!((count(&p, "dp4"), copies(&p)), (3, 0), "no copy left: {p:#?}");
    close(colour_of(&compile_raw(SHC_STAGE_FRAGMENT, &src), A, B), [dot(A, A), dot(B, B), dot(A, B), 1.0], "R dots");
}

/// R: normalmap's `vec2(0.0, 1.0 / u.y)`, the reciprocal writing lane y
/// itself; 4 before (`rcp t.x; mov o.y, t.x`).
fn r_a_reciprocal_writes_its_lane() {
    let src = fs("", "vec2 o = vec2(0.0, 1.0 / b.y); gl_FragColor = a + o.xyxy;");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("R rcp", &p);
    assert_eq!(p.len(), 3, "rcp, the 0.0 lane, the add (4 before)");
    assert_eq!(copies(&p), 0, "no copy left: {p:#?}");
    let r = 1.0 / B[1];
    close(colour_of(&compile_raw(SHC_STAGE_FRAGMENT, &src), A, B), [A[0], A[1] + r, A[2], A[3] + r], "R rcp");
}

/// P: the blurs' `TextureCoord = vUv`: the two coordinate adds read `uv`'s
/// input register directly; 10 before (`mov t.xy__, uv`).
fn p_a_partial_copy_of_a_varying_is_read_through() {
    let p = compile(
        SHC_STAGE_FRAGMENT,
        &fs(
            "varying vec2 uv;\n",
            "vec2 c = uv; gl_FragColor = texture2D(T, c + vec2(b.x, 0.0)) + texture2D(T, c - vec2(b.x, 0.0));",
        ),
    );
    show("P", &p);
    assert_eq!(p.len(), 9, "four lane stores, two adds, two texld and the sum (10 before)");
    assert!(!p.iter().any(|l| opcode(l) == "mov" && l.contains(".xy__,")), "no partial copy of uv: {p:#?}");
}

/// B: `vec4(a.xy + b.zw, a.zw * b.xy)` is assembled in a temporary and the
/// colour placed in it; 3 before (the colour copy).
fn b_the_colour_takes_the_register_of_the_value_it_copies() {
    let src = fs("", "gl_FragColor = vec4(a.xy + b.zw, a.zw * b.xy);");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("B", &p);
    assert_eq!(p.len(), 2, "an add and a mul (3 before)");
    assert_eq!(copies(&p), 0, "no colour copy: {p:#?}");
    close(
        colour_of(&compile_raw(SHC_STAGE_FRAGMENT, &src), A, B),
        [A[0] + B[2], A[1] + B[3], A[2] * B[0], A[3] * B[1]],
        "B",
    );
}

/// One test: the compiler's scratch arena is not reentrant, so the cases run
/// one after another.
#[test]
fn rank2_rest_copies_go_and_values_stay() {
    r_dot_products_write_their_lanes();
    r_a_reciprocal_writes_its_lane();
    p_a_partial_copy_of_a_varying_is_read_through();
    b_the_colour_takes_the_register_of_the_value_it_copies();
}
