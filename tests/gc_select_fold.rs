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
    assert_ne!(sh.viv.ps_color_out_reg, SHC_VIV_NO_REG, "a fragment shader writes the colour");
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

/// t5v: t5 with the lanes of each `vec2` swapped, tilt_v's shape. The
/// constant lane is stored first, so the product is no longer the last
/// instruction when its lane is stored; `retarget_last_to` now looks past
/// the constant lane's `mov` and the product writes the slot's lane itself.
/// 10 before rank T (`mov t.x, 0.0; mul tmp; mov t.y, tmp` a tap; the note
/// counted 8 after), 7 now: a tap is `mul t.y; mov t.x, 0.0; add`, and the
/// sr patch's copy fold takes one of the vec4 assembly's two copies. No lane
/// copy is left, and the swapped lanes cost nothing over t5.
///
/// The taps here are `2.0 * b.x` and `-2.0 * b.x`, which stay products: the
/// note's `1.0 * b.x` and `-1.0 * b.x` are `b.x` and `-b.x` since rank 4b,
/// and t5v as the note writes it (pinned in gc_terrain_shader_count.rs) is
/// 7 with no `mul` at all.
fn t5v_a_product_stored_after_a_constant_lane_writes_the_lane_itself() {
    let p = compile(
        SHC_STAGE_FRAGMENT,
        &fs("", "gl_FragColor = vec4(a.xy + vec2(0.0, -2.0 * b.x), a.xy + vec2(0.0, 2.0 * b.x));"),
    );
    show("t5v (k = 2)", &p);
    assert_eq!(p.len(), 7, "t5v is 7 instructions (10 before rank T)");
    assert_eq!(count(&p, "mul"), 2, "the two products");
    assert_eq!(count(&p, "add"), 2, "the two sums");
    assert_eq!(count(&p, "mov"), 3, "the two zero lanes and the output copy, no lane copy");
    let t5 = compile(
        SHC_STAGE_FRAGMENT,
        &fs("", "gl_FragColor = vec4(a.xy + vec2(-2.0 * b.x, 0.0), a.xy + vec2(2.0 * b.x, 0.0));"),
    );
    show("t5 (k = 2)", &t5);
    assert!(t5.len() >= p.len(), "the swapped lanes cost nothing over t5 ({} against {})", p.len(), t5.len());
}

/// t4, rank 4a: tilt-shift's constant divisor. `abs(1.0 - S)` and its
/// reciprocal were computed from `const float S = 0.5` by the shader, five
/// instructions a pass (`mov 0.5; mov -; add; mov |.|; rcp`); folded, the
/// shader is `add t, 0.5, -a.y; mul t, |t|, 2.0`. 9 before the sr patch, 5
/// before rank 4; the note's target was 2 with both. The value is checked
/// against the arithmetic on the host interpreter: folding is bit-exact.
fn t4_a_constant_divisor_is_folded_to_its_reciprocal() {
    let src = fs("const float S = 0.5;\n", "gl_FragColor = vec4(abs(S - a.y) / abs(1.0 - S));");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("t4", &p);
    assert_eq!(p.len(), 2, "t4 is 2 instructions (5 before rank 4)");
    assert_eq!(count(&p, "rcp"), 0, "the reciprocal of a constant is a constant");
    let c = compile_raw(SHC_STAGE_FRAGMENT, &src);
    for y in [0.0f32, 0.25, 0.5, 0.875, -1.5] {
        let want = (0.5 - y).abs() / 0.5;
        close(colour_of(&c, [0.0, y, 0.0, 0.0], [0.0; 4]), [want; 4], "t4");
    }
}

/// t4b, rank 4b: `vec4(a.xyz, 1.0) * 1.0 + 0.0`. The `mul` by one and the
/// `add` of zero are the operand under another name; what is left is the
/// constructor's two lane stores and the output copy, or fewer where the
/// copy fold reads through them.
fn t4b_a_product_by_one_and_a_sum_with_zero_are_not_emitted() {
    let src = fs("", "gl_FragColor = vec4(a.xyz, 1.0) * 1.0 + 0.0;");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("t4b", &p);
    assert_eq!(count(&p, "mul") + count(&p, "add"), 0, "no mul by 1.0, no add of 0.0");
    assert!(p.len() <= 3, "the constructor and the output copy at most, got {}", p.len());
    let c = compile_raw(SHC_STAGE_FRAGMENT, &src);
    close(colour_of(&c, [0.25, -0.5, 2.0, 9.0], [0.0; 4]), [0.25, -0.5, 2.0, 1.0], "t4b");
}

/// `x * -1.0` reads `-x` on the operand's negate modifier: one instruction,
/// the output copy of `-a`, and the value is the negation.
fn a_product_by_minus_one_reads_the_negated_operand() {
    let src = fs("", "gl_FragColor = a * -1.0;");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("neg", &p);
    assert_eq!(p.len(), 1, "one copy");
    assert!(p[0].contains("-t1"), "the copy reads -a: {}", p[0]);
    let c = compile_raw(SHC_STAGE_FRAGMENT, &src);
    close(colour_of(&c, [0.25, -0.5, 2.0, 9.0], [0.0; 4]), [-0.25, 0.5, -2.0, -9.0], "neg");
}

/// A fold `fold_float` refuses is still computed by the shader: `exp2(0.5)`
/// is not exact on the host, so the `exp` stays (and `1.0 / 3.0`, which is
/// exact as a rounding, goes). The values are the host's either way.
fn a_refused_fold_is_still_computed() {
    let src = fs("", "gl_FragColor = vec4(exp2(0.5), 1.0 / 3.0, exp2(3.0), 0.0) + a;");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("refused", &p);
    assert_eq!(count(&p, "exp"), 1, "exp2(0.5) is computed, exp2(3.0) is 8.0");
    assert_eq!(count(&p, "rcp"), 0, "1.0 / 3.0 is a constant");
    let c = compile_raw(SHC_STAGE_FRAGMENT, &src);
    close(colour_of(&c, [1.0; 4], [0.0; 4]), [1.0 + 2f32.sqrt(), 1.0 + 1.0 / 3.0, 9.0, 1.0], "refused");
}

/// `(x * c1) * c2` with `c2` a power of two is one `mul` by `c1 * c2`: tilt's
/// `TextureStep * abs(..) * 2.0`. Here `c1` is `1.0 / 600.0` (not an f20
/// immediate, so it sits in the constant file) and the merged constant takes
/// its place. The result equals the two roundings' because scaling by two
/// commutes with rounding.
fn a_product_scaled_by_a_power_of_two_is_one_mul() {
    let src = fs("const float S = 1.0 / 600.0;\n", "gl_FragColor = vec4(S * abs(a.y) * 2.0);");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("merge", &p);
    assert_eq!(count(&p, "mul"), 1, "one mul for the two");
    let c = compile_raw(SHC_STAGE_FRAGMENT, &src);
    for y in [0.0f32, 0.3, -0.7, 123.456] {
        let want = (1.0f32 / 600.0) * y.abs() * 2.0;
        close(colour_of(&c, [0.0, y, 0.0, 0.0], [0.0; 4]), [want; 4], "merge");
    }
    /* `* 3.0` is not a power of two: two muls, as before. */
    let p3 =
        compile(SHC_STAGE_FRAGMENT, &fs("const float S = 1.0 / 600.0;\n", "gl_FragColor = vec4(S * abs(a.y) * 3.0);"));
    assert_eq!(count(&p3, "mul"), 2, "a scale that is not a power of two is not merged");
}

/// An identity whose operand is a slot that is stored again later reads
/// what the slot held at the time, not the later value: `c = a; d = c *
/// 1.0; c = b; colour = d` is `a`. `src` copies such a read before the
/// store, and the alias names the copy.
fn an_identity_of_a_slot_stored_again_later_keeps_the_earlier_value() {
    let src = fs("", "vec4 c = a; vec4 d = c * 1.0; c = b; gl_FragColor = d + c * 0.0 + (c + 0.0) * 0.0;");
    let p = compile(SHC_STAGE_FRAGMENT, &src);
    show("slot", &p);
    let c = compile_raw(SHC_STAGE_FRAGMENT, &src);
    close(colour_of(&c, [1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0]), [1.0, 2.0, 3.0, 4.0], "slot");
}

/// One test: the online compiler's scratch arena is not reentrant, so the
/// cases run one after another on one thread.
#[test]
fn gc_select_fold_micro_shaders() {
    t5v_a_product_stored_after_a_constant_lane_writes_the_lane_itself();
    t4_a_constant_divisor_is_folded_to_its_reciprocal();
    t4b_a_product_by_one_and_a_sum_with_zero_are_not_emitted();
    a_product_by_minus_one_reads_the_negated_operand();
    a_refused_fold_is_still_computed();
    a_product_scaled_by_a_power_of_two_is_one_mul();
    an_identity_of_a_slot_stored_again_later_keeps_the_earlier_value();
}
