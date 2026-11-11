// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The micro-shaders of `docs/notes/gc-compiler-ranks-3-5.md` through the
//! GC7000 online compiler (plan #328, item gt-tiled-probe): rank T (a product
//! retargeted past a constant-lane store, t5v), rank 4a/4b (float constant
//! folding and the one-operand identities, t4 and t4b) and rank 4c (a
//! constant lane of a constructed vector read as the constant, t4c). Each
//! case pins the instruction count the note targets and says, from the
//! listing, which instruction went.
//!
//! Through the public compile path (`SHC_ONLINE.compile`, what
//! `glCompileShader` calls), so the counts are the ones
//! `tests/gc_terrain_shader_count.rs` pins.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_select_fold -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// The program of one compile: its instructions in listing form.
fn compile(stage: u32, src: &str) -> Vec<String> {
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

/// t5v: t5 with the lanes of each `vec2` swapped, tilt_v's shape. The
/// constant lane is stored first, so the product is no longer the last
/// instruction when its lane is stored; `retarget_last_to` now looks past
/// the constant lane's `mov` and the product writes the slot's lane itself.
/// 10 before rank T (`mov t.x, 0.0; mul tmp; mov t.y, tmp` a tap; the note
/// counted 8 after), 7 now: a tap is `mul t.y; mov t.x, 0.0; add`, and the
/// sr patch's copy fold takes one of the vec4 assembly's two copies. No lane
/// copy is left, and the swapped lanes cost nothing over t5.
#[test]
fn t5v_a_product_stored_after_a_constant_lane_writes_the_lane_itself() {
    let p = compile(
        SHC_STAGE_FRAGMENT,
        &fs("", "gl_FragColor = vec4(a.xy + vec2(0.0, -1.0 * b.x), a.xy + vec2(0.0, 1.0 * b.x));"),
    );
    show("t5v", &p);
    assert_eq!(p.len(), 7, "t5v is 7 instructions (10 before rank T)");
    assert_eq!(count(&p, "mul"), 2, "the two products");
    assert_eq!(count(&p, "add"), 2, "the two sums");
    assert_eq!(count(&p, "mov"), 3, "the two zero lanes and the output copy, no lane copy");
    let t5 = compile(
        SHC_STAGE_FRAGMENT,
        &fs("", "gl_FragColor = vec4(a.xy + vec2(-1.0 * b.x, 0.0), a.xy + vec2(1.0 * b.x, 0.0));"),
    );
    show("t5", &t5);
    assert!(t5.len() >= p.len(), "the swapped lanes cost nothing over t5 ({} against {})", p.len(), t5.len());
}
