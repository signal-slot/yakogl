// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The INT_VALUE refusal of glmark2 ideas' fs c565 and 1aca on the Mali (plan
//! #367 lower-train-2, from proto-int-value-ideas; docs/notes/mali-ideas.md
//! section 4): the `&&` of two fp16 compares in `unitvec`'s `v1.w == 0.0 &&
//! v2.w == 0.0`, which Mesa keeps as the 16-bit `iand` of the two masks
//! (gm2_ideas_2's fs.fragment: `LSHIFT_AND.v2i16 r4, r7^.h00, 0x0.b00, r4^`
//! of two `FCMP_OR.v2f16.eq.m1`). `nir_fs.rs` `Fs::and16` now takes two
//! `feq` (or two `fneu`) sharing an operand, and `Fs::and_masks` folds
//! `iand(flt(a, b), flt(a, c))` into `flt(a, fmin(b, c))`:
//! deqp_intval3_opfs_logical_and_bool is byte-equal
//! (`tests/mali_deqp_intval3.rs`).
//!
//! Both shaders, and their smaller forms gm2_cf_earlyret_fs and
//! gm2_cf_constloop_struct_fs, then stop at the selects of the flattened
//! function: FS_OPERAND (c565) and FRAGCOLOR_LANE (1aca). Pinned below, so
//! that the item that lowers them moves them to [`mali_ref::assert_matches`].
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_int_value_ideas
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use yakogl::shaderc::online::mali::isa;

const FS_OPERAND: &str = "a fragment shader operand of a form or in a place no reference draws";
const FRAGCOLOR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const INT_VALUE: &str = "an integer or boolean value is not lowered yet";

/// ideas c565 (gm2_ideas_2) and the early returns alone (gm2_cf_earlyret_fs)
/// are past INT_VALUE and refused at the operand form of the flattened
/// function's selects.
#[test]
fn ideas_c565_is_past_int_value_at_its_operand_form() {
    let moved: Vec<String> =
        ["gm2_ideas_2", "gm2_cf_earlyret_fs"].iter().filter_map(|n| mali_ref::refusal_change(n, FS_OPERAND)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// ideas 1aca (gm2_ideas_3) and its loop over a struct array alone
/// (gm2_cf_constloop_struct_fs) are past INT_VALUE and refused at the
/// colour's lanes.
#[test]
fn ideas_1aca_is_past_int_value_at_its_colour_lanes() {
    let moved: Vec<String> = ["gm2_ideas_3", "gm2_cf_constloop_struct_fs"]
        .iter()
        .filter_map(|n| mali_ref::refusal_change(n, FRAGCOLOR_LANE))
        .collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

fn fragment_text(fs: &str) -> String {
    let vs = "attribute vec4 p; attribute vec4 a; varying vec4 v; void main() { gl_Position = p; v = a; }";
    let pair = mali_ref::compile_pair(vs, fs).unwrap_or_else(|e| panic!("refused: {e}"));
    let mut s = String::new();
    for w in &pair.fs.code {
        if *w != 0 {
            isa::describe_word(*w, &mut s).unwrap();
            s.push('\n');
        }
    }
    s
}

/// `a > 0.0 && b > 0.0` of two varyings' lanes is one compare of the
/// minimum (nir_opt_algebraic.py:1157), deqp_intval3_opfs_logical_and_bool's
/// shape: no `iand` is left.
#[test]
fn and_of_two_lt_with_a_common_first_operand_is_one_compare_of_fmin() {
    mali_ref::assert_matches("deqp_intval3_opfs_logical_and_bool");
    let t = std::fs::read_to_string("tools/mali/testdata/deqp_intval3_opfs_logical_and_bool.expect.json").unwrap();
    assert!(t.contains("FMIN.v2f16"), "the reference's form");
}

/// `unitvec`'s condition alone, as a select of two constants: the `&&` of a
/// varying's and a uniform's `.w == 0.0` is the 16-bit `iand` of two
/// `FCMP_OR.v2f16.eq.m1` masks, as in gm2_ideas_2 (no fold:
/// nir_opt_algebraic has no `iand` rule of two `feq`s).
#[test]
fn and_of_two_fp16_equalities_is_one_16_bit_iand() {
    let fs = "precision mediump float; varying vec4 v; uniform vec4 u; \
              void main() { float s = 0.5; if (v.w == 0.0 && u.w == 0.0) s = 1.0; \
              gl_FragColor = vec4(s, s, s, 1.0); }";
    let t = fragment_text(fs);
    assert_eq!(t.matches("FCMP_OR.v2f16.eq").count(), 2, "{t}");
    assert!(t.contains("LSHIFT_AND.v2i16"), "{t}");
}

/// The `||` of two fp16 masks (deqp_intval4_logical_or_fs, Mesa's `fmax`
/// rewrite) is not this step's: still INT_VALUE.
#[test]
fn or_of_two_fp16_compares_stays_int_value() {
    mali_ref::assert_refused_ref("deqp_intval4_logical_or_fs", INT_VALUE);
}
