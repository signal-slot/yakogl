// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_loops_*` reference (Mesa
//! 26.2.1 words of the dEQP-GLES2 loop shapes, `docs/notes/mali-deqp-loops.md`
//! section 5) against the Mali online compiler, through `tests/mali_ref`.
//!
//! A reference still refused is ignored with its refusal and the step
//! (lower-loops-1 or lower-loops-2) that lowers it; the lowering removes
//! the `ignore` and the shape's entry in `REFUSALS`. The counted vertex
//! loops (`vs_for_const`, `vs_while_const`, `vs_dowhile_const`) are
//! byte-equal since plan #367 lower-int-loops' vertex unroll.
//! `refusals_pinned` runs now: it asserts each reference's exact current
//! refusal (the vertex shader's when it refuses, else the fragment shader's).
//! The mediump kept loops of `fs_for_uni`, `fs_while_uni`, `fs_dowhile_uni`,
//! `fs_cond_break`, `fs_cond_continue`, `fs_double_continue`, `fs_mixed` and
//! `fs_inf_break` compile (note section 10), as do `fs_select` and
//! `fs_float_uni` (section 13): each is a compile check here. So are the
//! vertex kept loops `vs_for_uni`, `vs_vector`, `vs_select`,
//! `vs_dowhile_uni` and `vs_float_uni` (spike-vsloop-rest: `res` carried
//! as 16-bit pairs in the varying program).
//! A kept-loop reference carries the pass counter in its words, which the
//! shared compare does not edit: its lowering compares it with `CapEdit` in
//! `src/shaderc/online/mali/tests.rs` and turns its test here into a compile
//! check (note section 5).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_loops
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// Compiles (101 passes, past Mesa's unroll limit: kept as a do-while,
/// nir_fs.rs `rotated_loop`). A kept loop: its words are Mesa's with the
/// loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_101_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_101() {
    compiles("deqp_loops_fs_101");
}

/// Compiles (step 2b: the `break` merged into the head's exit test). A kept loop: its words are Mesa's with
/// the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_cond_break_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_cond_break() {
    compiles("deqp_loops_fs_cond_break");
}

/// Compiles (step 2b: the `continue` flattened into `MUX.v2i16` selects). A kept loop: its words are Mesa's with
/// the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_cond_continue_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_cond_continue() {
    compiles("deqp_loops_fs_cond_continue");
}

/// Compiles (step 2b: a bare `continue` ending the body). A kept loop: its words are Mesa's with
/// the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_double_continue_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_double_continue() {
    compiles("deqp_loops_fs_double_continue");
}

/// Compiles (the do-while's tail test its terminator, the `if` four
/// `MUX.v2i16`). A kept loop: its words are Mesa's with the loop cap's,
/// compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_dowhile_if_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_dowhile_if() {
    compiles("deqp_loops_fs_dowhile_if");
}

/// Compiles (step 1b: the do-while). A kept loop: its words are Mesa's with
/// the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_dowhile_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_dowhile_uni() {
    compiles("deqp_loops_fs_dowhile_uni");
}

/// Compiles (the fp16 float counter, one 16-bit phi of one lane, its head
/// test `FCMP_OR.v2f16`). A kept loop: its words are Mesa's with the loop
/// cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_float_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_float_uni() {
    compiles("deqp_loops_fs_float_uni");
}

/// Compiles (step 1b: the mediump kept loop). A kept loop: its words are Mesa's with
/// the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_for_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_for_uni() {
    compiles("deqp_loops_fs_for_uni");
}

#[test]
#[ignore = "differs: no loop since the array rule (`guessed_trips`), the words of its constant-bound twin (`fs_idx_dyn_is_its_constant_bound_twin`), whose sum Mesa fuses into three FMA.v2f16 and ours does not (#367 lower-fma)"]
fn fs_idx_dyn() {
    mali_ref::assert_matches("deqp_loops_fs_idx_dyn");
}

/// The array rule (nir_fs.rs `guessed_trips`, docs/notes/mali-deqp-loops.md
/// section 10): `for (int i = 0; i < ui_four; i++) arr[i] = ..;` over
/// `float arr[4]` is unrolled by the array's length with its run-time
/// bound dropped, as Mesa's `partial_unroll` leaves it. Both stages are the
/// program of the same pair with the bound `4`, word for word, or refuse
/// alike (the vertex twin's constop refusal).
#[test]
fn fs_idx_dyn_is_its_constant_bound_twin() {
    for name in ["deqp_loops_fs_idx_dyn", "deqp_loops_vs_idx_dyn"] {
        let r = mali_ref::Reference::load(name);
        let twin = |s: &str| s.replace("i < ui_four", "i < 4");
        match (mali_ref::compile_pair(&r.vs, &r.fs), mali_ref::compile_pair(&twin(&r.vs), &twin(&r.fs))) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.fs.code, b.fs.code, "{name}: fragment words");
                assert_eq!(a.vs.code, b.vs.code, "{name}: vertex words");
            }
            (Err(a), Err(b)) => assert_eq!(a, b, "{name}: refusals"),
            (a, b) => panic!("{name}: {:?} against its twin's {:?}", a.err(), b.err()),
        }
    }
}

/// Compiles (step 2b: the lone `if (c) break;` terminator); a kept loop, its words are Mesa's with the
/// loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_inf_break_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_inf_break() {
    compiles("deqp_loops_fs_inf_break");
}

/// Compiles (step 2b: `continue` and `else if (..) break;`); a kept loop, its words are Mesa's with the
/// loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_mixed_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_mixed() {
    compiles("deqp_loops_fs_mixed");
}

/// Compiles (fs_for_uni's loop, its bound `ub_true ? ui_three : 0` a
/// `CSEL.u32.eq` on the bool uniform's word). A kept loop: its words are
/// Mesa's with the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_select_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_select() {
    compiles("deqp_loops_fs_select");
}

/// Byte-equal: the loop goes as Mesa's `wrapper_unroll` drops it, its one
/// pass the else side of the head test's inverse compare
/// (`opt_if_simplification`), the varying's lanes converted and selected
/// by `MUX.v2i16` (nir_fs.rs `walk_if16`, `Fs::sel16`).
#[test]
fn fs_uncond_break() {
    mali_ref::assert_matches("deqp_loops_fs_uncond_break");
}

#[test]
#[ignore = "refused: a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside, (an indexed store in a kept loop; then a second, sequenced kept loop), lowered by lower-loops-2"]
fn fs_vec_dyn() {
    mali_ref::assert_matches("deqp_loops_fs_vec_dyn");
}

/// Compiles (step 1b: the `while` form). A kept loop: its words are Mesa's with
/// the loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_fs_while_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn fs_while_uni() {
    compiles("deqp_loops_fs_while_uni");
}

/// Unrolled as Mesa unrolls it (plan #367 lower-int-loops: the vertex
/// walk unrolls a counted loop, nir.rs `walk`).
#[test]
fn vs_dowhile_const() {
    mali_ref::assert_matches("deqp_loops_vs_dowhile_const");
}

/// Compiles (the vertex kept loop's fp16 pairs, Mesa's do-while: the pairs'
/// `vec2`s after the head test). A kept loop: its words are Mesa's with the
/// loop cap's, compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_vs_dowhile_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn vs_dowhile_uni() {
    compiles("deqp_loops_vs_dowhile_uni");
}

/// Compiles (the vertex kept loop's fp16 pairs and the fp32 counter's
/// `FCMP_OR.f32` head). A kept loop: its words are Mesa's with the loop cap's,
/// compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_vs_float_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn vs_float_uni() {
    compiles("deqp_loops_vs_float_uni");
}

/// Unrolled as Mesa unrolls it (plan #367 lower-int-loops: the vertex
/// walk unrolls a counted loop, nir.rs `walk`).
#[test]
fn vs_for_const() {
    mali_ref::assert_matches("deqp_loops_vs_for_const");
}

/// Compiles (the vertex kept loop: `res` carried as two 16-bit `vec2` phis in
/// the varying program). A kept loop: its words are Mesa's with the loop cap's,
/// compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_vs_for_uni_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn vs_for_uni() {
    compiles("deqp_loops_vs_for_uni");
}

#[test]
#[ignore = "refused: a constant product in a shape Mesa's algebraic rules move or fold, the vertex twin's of the array rule (`fs_idx_dyn_is_its_constant_bound_twin`; its counter's integer constants fold, ConstantFold step A)"]
fn vs_idx_dyn() {
    mali_ref::assert_matches("deqp_loops_vs_idx_dyn");
}

#[test]
#[ignore = "refused: a constant product in a shape Mesa's algebraic rules move or fold (its unrolled counter's integer constants fold, ConstantFold step A)"]
fn vs_idx_static() {
    mali_ref::assert_matches("deqp_loops_vs_idx_static");
}

/// Unrolled as Mesa unrolls it: counted loops nested in a counted loop, the
/// vertex walk's nested `LoopBegin` steps (nir.rs `vs_loop_shape`,
/// `put_loop`).
#[test]
fn vs_nested_const() {
    mali_ref::assert_matches("deqp_loops_vs_nested_const");
}

/// Compiles (the vertex kept loop, its bound the `CSEL.u32` on the bool
/// uniform's word). A kept loop: its words are Mesa's with the loop cap's,
/// compared in `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_vs_select_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn vs_select() {
    compiles("deqp_loops_vs_select");
}

/// Compiles (the vertex kept loop, the `ivec4` counter one scalar phi). A kept
/// loop: its words are Mesa's with the loop cap's, compared in
/// `src/shaderc/online/mali/tests.rs`
/// `deqp_loops_vs_vector_is_mesa_26_2_1_with_the_loop_cap`.
#[test]
fn vs_vector() {
    compiles("deqp_loops_vs_vector");
}

/// Unrolled as Mesa unrolls it (plan #367 lower-int-loops: the vertex
/// walk unrolls a counted loop, nir.rs `walk`).
#[test]
fn vs_while_const() {
    mali_ref::assert_matches("deqp_loops_vs_while_const");
}

/// The pair of reference `name` compiles and links (a kept loop's words are
/// compared with the cap in `src/shaderc/online/mali/tests.rs`).
fn compiles(name: &str) {
    let r = mali_ref::Reference::load(name);
    if let Err(e) = mali_ref::compile_pair(&r.vs, &r.fs) {
        panic!("{name}: refused: {e}");
    }
}

/// The first refusal of each reference at the tip, without the "Valhall: "
/// prefix and the " is not lowered yet" suffix.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_loops_fs_vec_dyn", "a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside,"),
    ("deqp_loops_vs_idx_dyn", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
    ("deqp_loops_vs_idx_static", "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)"),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!("{name} now compiles: drop its ignore and its entry in REFUSALS"),
            Err(e) => assert_eq!(
                e.strip_prefix("Valhall: ").and_then(|e| e.strip_suffix(" is not lowered yet")),
                Some(*text),
                "{name}: the refusal moved: {e}"
            ),
        }
    }
}
