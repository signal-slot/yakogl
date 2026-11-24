// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's vertex loops on the Mali (plan #367 lower-int-loops): the
//! `for` of an `int` counter in a vertex shader, unrolled when its bound is
//! a constant and kept when it is an `int` uniform (glmark2 loop's vertex
//! shader 3cb0, `for (int i = 0; i < VertexLoops; i++) d = fract(3.0 *
//! d);`), against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_intloop_{const,cond,uniform}_vs` and
//! `gm2_cf_loop_uni_vs` (docs/notes/mali-int-loops.md, "glmark2's vertex
//! loops"), built as glCompileShader and glLinkProgram build them
//! (`tests/mali_ref/mod.rs`) and compared in every region.
//!
//! A kept loop carries the pass counter every kept loop of this back end
//! has (docs/notes/mali-loop-board-failures.md, a DELIBERATE deviation from
//! Mesa): its program is compared against the reference with the counter's
//! edits applied ([`CapEdit`], the rule of `src/shaderc/online/mali/tests.rs`):
//! the three words of the counter added, the pre-header word whose
//! `.reconverge` the counter's start takes, the head test's third source,
//! the registers the counter moves and the back edge's offset fixed. Every
//! other word, the facts and the FAU words are Mesa's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_int_loops
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use mali_ref::Reference;

/// One change the kept-loop cap makes to a Mesa program's position words,
/// at a position of the result, the edits applied in order.
#[derive(Clone, Copy, Debug)]
enum CapEdit {
    /// A word of the cap, inserted.
    Add(usize, u64),
    /// The reference's word, rewritten.
    Fix(usize, u64),
}

/// The words of `vs.position` in the reference's JSON.
fn position_words(json: &str) -> (usize, usize, Vec<u64>) {
    let at = json.find("\"vs.position\"").expect("vs.position");
    let start = at + json[at..].find("\"words\": [").expect("words") + "\"words\": [".len();
    let end = start + json[start..].find(']').expect("the words' end");
    let words =
        json[start..end].split(',').filter_map(|x| u64::from_str_radix(x.trim().trim_matches('"'), 16).ok()).collect();
    (start, end, words)
}

/// Reference `r` with the cap's `edits` applied to its position program.
fn capped(mut r: Reference, edits: &[CapEdit]) -> Reference {
    let (start, end, mut words) = position_words(&r.json);
    for &e in edits {
        match e {
            CapEdit::Add(at, w) => words.insert(at, w),
            CapEdit::Fix(at, w) => {
                assert_ne!(words[at], w, "{e:?}: the reference already has the word");
                words[at] = w;
            }
        }
    }
    let list: Vec<String> = words.iter().map(|w| format!("\n    \"{w:016x}\"")).collect();
    r.json = format!("{}{}\n   {}", &r.json[..start], list.join(","), &r.json[end..]);
    r
}

fn assert_capped(r: Reference, edits: &[CapEdit]) {
    let name = r.name.clone();
    let r = capped(r, edits);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "{name} (with the loop cap):\n{}", d.join("\n"));
}

/// The pass counter's three words, as `nir.rs` `vs_kept_begin` and
/// `vs_kept_head` make them: the pre-header's start, the header's count
/// down and its compare with 0.
fn is_cap_word(w: u64) -> bool {
    let mut s = String::new();
    yakogl::shaderc::online::mali::isa::describe_word(w, &mut s).expect("a word");
    (s.starts_with("IADD_IMM.i32.reconverge ") && s.ends_with(", 0x0, #0x1000"))
        || (s.starts_with("IADD_IMM.i32 ") && s.ends_with("^, #0xFFFFFFFF"))
        || (s.starts_with("ICMP_OR.s32.lt.m1 ") && s.ends_with(", 0x0, 0x0"))
}

/// A `for` of five passes (`i < 5`): Mesa unrolls it (`simple_unroll`),
/// five `fract(3.0 * d)` in a row and the one after the loop, no int left.
#[test]
fn gm2_intloop_const_vs_unrolls_as_mesa() {
    mali_ref::assert_matches_unset("gm2_intloop_const_vs");
}

/// The same steps written as three `if`/`else` steps, no loop: the
/// flattened selects (unchanged by this lowering; the loop cases' control).
#[test]
fn gm2_intloop_cond_vs_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_intloop_cond_vs");
}

/// glmark2 loop's vertex shader 3cb0 as the benchmark writes it, the bound
/// an `int` uniform (`u_loops`): Mesa keeps the loop. The header's
/// `ICMP_OR.s32.ge.m1` of the counter and the uniform's FAU word (`u11.w0`,
/// read in place), the `BRANCHZ` to the exit, the body `FMA`, `FROUND`,
/// `FADD`, the counter's `IADD_IMM #0x1` and the back edge; `d` carried in
/// r4 (its value before the loop copied there, `MOV.i32 r4, r3`, as
/// `fract(a_pos.x)` is read again after the loop).
#[test]
fn gm2_intloop_uniform_vs_is_mesa_with_the_loop_cap() {
    assert_capped(
        Reference::load_unset("gm2_intloop_uniform_vs"),
        &[
            CapEdit::Fix(4, 0x0110c500000000c0), // IADD_IMM.i32 r5, 0x0, #0x0 (Mesa: .reconverge)
            CapEdit::Add(5, 0x5110c600001000c0), // IADD_IMM.i32.reconverge r6, 0x0, #0x1000
            CapEdit::Add(6, 0x0110c6ffffffff46), // IADD_IMM.i32 r6, r6^, #0xFFFFFFFF
            CapEdit::Add(7, 0x00f8c70480c0c006), // ICMP_OR.s32.lt.m1 r7, r6, 0x0, 0x0
            CapEdit::Fix(8, 0x00f8c70280479605), // ICMP_OR.s32.ge.m1 r7, r5, u11.w0, r7^ (Mesa: r6, .., 0x0)
            CapEdit::Fix(9, 0x501fc02000000547), // BRANCHZ.reconverge r7^.h0, offset:5 (Mesa: r6^)
            CapEdit::Fix(11, 0x0090c700800d0004), // FROUND.f32.rtn r7, r4 (Mesa: r6)
            CapEdit::Fix(12, 0x00a4c41000004744), // FADD.f32 r4, r4^, r7^.neg (Mesa: r6^)
            CapEdit::Fix(14, 0x501fc017fffff7c0), // BRANCHZ.eq.reconverge 0x0, offset:-9 (Mesa: -7)
        ],
    );
}

/// glmark2 loop's vertex shader 3cb0's shape (`gm2_cf_loop_uni_vs`, drawn
/// with gl_ref.py's `--uniform-values`: `u_n` is 32, `u11.w0`), its loop
/// feeding gl_Position: in the position program, as Mesa's.
#[test]
fn gm2_cf_loop_uni_vs_is_mesa_with_the_loop_cap() {
    assert_capped(
        Reference::load("gm2_cf_loop_uni_vs"),
        &[
            CapEdit::Fix(3, 0x0110c400000000c0), // IADD_IMM.i32 r4, 0x0, #0x0 (Mesa: .reconverge)
            CapEdit::Add(4, 0x5110c500001000c0), // IADD_IMM.i32.reconverge r5, 0x0, #0x1000
            CapEdit::Add(5, 0x0110c5ffffffff45), // IADD_IMM.i32 r5, r5^, #0xFFFFFFFF
            CapEdit::Add(6, 0x00f8c60480c0c005), // ICMP_OR.s32.lt.m1 r6, r5, 0x0, 0x0
            CapEdit::Fix(7, 0x00f8c60280469604), // ICMP_OR.s32.ge.m1 r6, r4, u11.w0, r6^ (Mesa: r5, .., 0x0)
            CapEdit::Fix(8, 0x501fc02000000546), // BRANCHZ.reconverge r6^.h0, offset:5 (Mesa: r5^)
            CapEdit::Fix(10, 0x0090c600800d0003), // FROUND.f32.rtn r6, r3 (Mesa: r5)
            CapEdit::Fix(11, 0x00a4c31000004643), // FADD.f32 r3, r3^, r6^.neg (Mesa: r5^)
            CapEdit::Fix(13, 0x501fc017fffff7c0), // BRANCHZ.eq.reconverge 0x0, offset:-9 (Mesa: -7)
        ],
    );
}

/// The words the tests above add are the counter's, and nothing else.
#[test]
fn the_added_words_are_the_counters() {
    for w in [
        0x5110c600001000c0u64,
        0x0110c6ffffffff46,
        0x00f8c70480c0c006,
        0x5110c500001000c0,
        0x0110c5ffffffff45,
        0x00f8c60480c0c005,
    ] {
        assert!(is_cap_word(w), "{w:016x} is not a word of the loop cap");
    }
}

const FS: &str = "precision mediump float; void main() { gl_FragColor = vec4(1.0); }";

/// A counted loop past `max_unroll_iterations` (32 passes): Mesa keeps it,
/// with a constant bound; no vertex reference draws that form.
#[test]
fn a_counted_loop_of_33_passes_stays_refused() {
    let vs = "attribute vec3 p; void main() { float d = p.x;
        for (int i = 0; i < 33; i++) d = fract(3.0 * d);
        gl_Position = vec4(p.x, d, p.z, 1.0); }";
    mali_ref::assert_refused(vs, FS, "a loop in a vertex shader");
}

/// A loop that writes gl_Position in its body, and one nested in another
/// (`intloop_loop_nested_vs`): no vertex reference.
#[test]
fn a_loop_storing_an_output_or_nested_stays_refused() {
    let vs = "attribute vec3 p; uniform int n; void main() {
        for (int i = 0; i < n; i++) gl_Position = vec4(p, 1.0); }";
    mali_ref::assert_refused(vs, FS, "a loop in a vertex shader");
    mali_ref::assert_refused_ref("intloop_loop_nested_vs", "a loop in a vertex shader");
}

/// An `int` uniform in arithmetic is read as its word since plan #387
/// intval-vs16 (vs16-2: `IADD_IMM.i32 r, u.w0, #0x1` and `S32_TO_F32`, the
/// fragment walk's reslib_int_vs form): the pair compiles.
#[test]
fn an_int_uniform_in_arithmetic_compiles() {
    let vs = "attribute vec3 p; uniform int n; void main() {
        gl_Position = vec4(p * float(n + 1), 1.0); }";
    assert!(mali_ref::compile_pair(vs, FS).is_ok(), "the pair compiles");
}

/// The fragment half of the int-loop family is not this lowering's: glmark2
/// loop's fragment shader 3678 keeps a mediump loop of a uniform bound,
/// refused in `nir_fs.rs` `walk_kept_loop` (gm2_intloop_uniform_fs).
#[test]
fn the_mediump_fragment_kept_loop_stays_refused() {
    mali_ref::assert_refused_ref("gm2_intloop_uniform_fs", "a loop Mesa keeps");
}
