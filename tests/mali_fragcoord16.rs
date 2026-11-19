// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! FRAGCOORD_FP16 of plan #367 (lower-fragcoord16) on the Mali: the head
//! `float d = fract(FragCoord.x * FragCoord.y * 0.0001)` of glmark2's
//! conditionals (fs 6c61, a5b5), function (c8ce) and loop (e016, c7ea)
//! fragment shaders, against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_fcoord16_1..6` and `gm2_cf_call_fs`,
//! `gm2_cf_loop_const_fs`, `gm2_cf_ifelse_fs`
//! (docs/notes/mali-fragcoord-fp16.md, mali-fragcoord-fp16-ops.md).
//!
//! glmark2 copies the ES 1.00 (mediump) gl_FragCoord into a local declared
//! `HIGHP_OR_DEFAULT`, highp on an ES device. GLSL's lower_precision narrows
//! no bare dereference, so Mesa's local holds the fp32 read, the head is
//! fp32 (`FMA.f32` x2, `FROUND.f32.rtn`, `FADD.f32`) and the mediump `d`
//! narrows it once (`FADD.f32 r0.h0, r0^, 0x0.neg`); each step after it is a
//! one-lane fp16 operation (`FMA.v2f16 r0, u3.w1, r0^.h00, 0x0.neg`). The
//! all-mediump shapes (gm2_fcoord16_*) narrow the read itself and compute
//! the head at fp16.
//!
//! R1 to R5 are the five glmark2 shapes as glmark2 builds them (its
//! precision header, the `HIGHP_OR_DEFAULT` local, the unused `dummy`
//! varying; the scene steps written out). Before this item every one was
//! refused (FRAGCOLOR_LANE at the tip, FRAGCOORD_FP16 once the colour
//! leaf was given); each compiles now, and those with a reference of the
//! same fragment program are compared word for word through it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_fragcoord16
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// glmark2's vertex shader of these scenes, reduced to what the link sees:
/// the position and the `dummy` varying it writes.
const VS: &str = "attribute vec3 position; varying vec4 dummy; \
                  void main() { dummy = vec4(1.0); gl_Position = vec4(position, 1.0); }";

/// glmark2's precision header (libmatrix `ShaderSource::str`, written out):
/// `HIGHP_OR_DEFAULT` is `highp` on an ES device with highp fragment floats.
const HEAD: &str = "#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n\
                    #define HIGHP_OR_DEFAULT highp\n#else\n#define HIGHP_OR_DEFAULT\n#endif\n\
                    #ifdef GL_ES\nprecision mediump float;\n#endif\nvarying vec4 dummy;\n";

/// The fragment shader of the scenes: `pre` before `main`, `steps` after the
/// head.
fn fs(pre: &str, steps: &str) -> String {
    format!(
        "{HEAD}{pre}void main(void) {{\n    HIGHP_OR_DEFAULT vec2 FragCoord = gl_FragCoord.xy;\n    \
         float d = fract(FragCoord.x * FragCoord.y * 0.0001);\n{steps}    gl_FragColor = vec4(d, d, d, 1.0);\n}}\n"
    )
}

/// R1, conditionals fs 6c61 (fragment-steps=0): the head alone.
fn r1() -> String {
    fs("", "")
}

/// R2, conditionals fs a5b5 (fragment-steps=5): five two-armed `if`s.
fn r2() -> String {
    fs("", &"    if (d >= 0.5)\n        d = fract(2.0 * d);\n    else\n        d = fract(3.0 * d);\n".repeat(5))
}

/// R3, function fs c8ce (fragment-steps=5, complexity low): five calls.
fn r3() -> String {
    fs("float process(float d) {\n    d = fract(3.0 * d);\n    return d;\n}\n", &"    d = process(d);\n".repeat(5))
}

/// R4, loop fs e016 (fragment-loop=false): five steps written out.
fn r4() -> String {
    fs("", &"    d = fract(3.0 * d);\n".repeat(5))
}

/// R5, loop fs c7ea (fragment-uniform=false): a counted loop of five.
fn r5() -> String {
    fs("", "    for (int i = 0; i < 5; i++)\n        d = fract(3.0 * d);\n")
}

/// The fragment program of `fs` (with glmark2's vertex shader) has the
/// words of reference `name`'s, which [`mali_ref::assert_matches_unset`]
/// compares with Mesa's.
fn same_fragment_words(fs: &str, name: &str) {
    let ours = mali_ref::compile_pair(VS, fs).unwrap_or_else(|e| panic!("refused: {e}"));
    let r = mali_ref::Reference::load_unset(name);
    let theirs = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name} refused: {e}"));
    assert_eq!(ours.fs.code, theirs.fs.code, "the fragment words differ from {name}'s");
    assert_eq!(ours.fs.fau, theirs.fs.fau, "the fragment FAU words differ from {name}'s");
    assert_eq!(ours.fs.work_reg_count, theirs.fs.work_reg_count);
}

#[test]
fn r1_6c61_the_head_alone_compiles() {
    mali_ref::compile_pair(VS, &r1()).unwrap_or_else(|e| panic!("refused: {e}"));
}

/// a5b5 compiles; its words are not Mesa's yet ([`gm2_cf_ifelse_fs_compiles_and_still_differs`]).
#[test]
fn r2_a5b5_five_ifs_compile() {
    mali_ref::compile_pair(VS, &r2()).unwrap_or_else(|e| panic!("refused: {e}"));
}

#[test]
fn r3_c8ce_five_calls_have_mesa_s_words() {
    same_fragment_words(&r3(), "gm2_cf_call_fs");
}

#[test]
fn r4_e016_five_steps_have_mesa_s_words() {
    same_fragment_words(&r4(), "gm2_cf_call_fs");
}

#[test]
fn r5_c7ea_the_counted_loop_has_mesa_s_words() {
    same_fragment_words(&r5(), "gm2_cf_loop_const_fs");
}

/// `gl_FragCoord.x * gl_FragCoord.y` of the mediump read: each lane
/// narrowed alone, one `FMA.v2f16 r0, r0^.h00, r1^.h00, 0x0.neg`.
#[test]
fn gm2_fcoord16_1_the_product_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fcoord16_1");
}

/// `* 0.0001` after it: `fmul(fmul(a, b), #c)` is `fmul(fmul(a, c), b)`
/// (nir_opt_algebraic.py:2394), `x * 0.0001` first, by `u3.w0` =
/// 0x068e068e.
#[test]
fn gm2_fcoord16_2_the_scaled_product_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fcoord16_2");
}

/// `fract` of it through a mediump `vec2` copy: `FROUND.v2f16.rtn` and
/// `FADD.v2f16 .., r1^.neg.h00` of one lane.
#[test]
fn gm2_fcoord16_3_fract_through_a_copy_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fcoord16_3");
}

#[test]
fn gm2_fcoord16_4_fract_written_directly_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fcoord16_4");
}

/// An `if` on the head: the two arms' sums one `FADD.v2f16` of the pair
/// (-0.5, 0.25), the select `MUX.v2i16 r0, r1^.h11, r1^.h00, r0^.h00`, the
/// colour `MKVEC.v2i16 r2, r0^.h0, u4.w0.h0`.
#[test]
fn gm2_fcoord16_5_the_branch_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fcoord16_5");
}

/// A call and a counted loop of four `fract(d * 1.7)` on the head.
#[test]
fn gm2_fcoord16_6_the_call_and_loop_match_their_reference() {
    mali_ref::assert_matches_unset("gm2_fcoord16_6");
}

/// glmark2's function c8ce shape: the highp copy's fp32 head, one
/// narrowing, five one-lane fp16 steps.
#[test]
fn gm2_cf_call_fs_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_cf_call_fs");
}

/// glmark2's loop c7ea shape: the same words, the loop unrolled.
#[test]
fn gm2_cf_loop_const_fs_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_cf_loop_const_fs");
}

/// glmark2's conditionals a5b5 shape compiles: the fp32 head and its
/// narrowing are Mesa's, but Mesa's `nir_opt_vectorize` pairs each `if`'s
/// two arms `2.0 * t` and `3.0 * t` into one `FMA.v2f16 r1, u3.w0, r0.h00`
/// (0x42004000) on the fp32-island route, and moves 0.0001 and the alpha's
/// 1.0 pair into registers (`IADD_IMM`), where ours computes each arm alone
/// (58 words, Mesa's 45). When that lands this test fails: make it
/// [`mali_ref::assert_matches_unset`].
#[test]
fn gm2_cf_ifelse_fs_compiles_and_still_differs() {
    let r = mali_ref::Reference::load_unset("gm2_cf_ifelse_fs");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("refused: {e}"));
    let d = mali_ref::differences(&r, &pair);
    assert!(!d.is_empty(), "gm2_cf_ifelse_fs is byte-equal now: pin it with assert_matches_unset");
    assert!(d.iter().all(|x| x.starts_with("fs.fragment")), "the difference moved:\n{}", d.join("\n"));
}

/// What has no reference stays refused: the highp copy beside a read of
/// gl_FragCoord the walk narrows (Mesa reads that one at fp32 too, in the
/// copy's highp tree).
#[test]
fn a_copy_beside_a_mediump_read_stays_refused() {
    let fs = format!(
        "{HEAD}void main(void) {{ HIGHP_OR_DEFAULT vec2 p = gl_FragCoord.xy; \
         float d = fract(p.x * p.y * 0.0001) + gl_FragCoord.x; gl_FragColor = vec4(d, d, d, 1.0); }}"
    );
    mali_ref::assert_refused(VS, &fs, "mixes highp and mediump");
}

/// A highp local that copies anything but gl_FragCoord is still a mixed
/// shader.
#[test]
fn another_highp_local_stays_refused() {
    let fs = "precision mediump float; varying vec4 dummy; \
              void main() { highp float h = dummy.x * 3.0; gl_FragColor = vec4(fract(h)); }";
    mali_ref::assert_refused(VS, fs, "mixes highp and mediump");
}
