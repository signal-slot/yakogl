// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_gaps_<family>_<shape>` (plan #377,
//! item ref-gaps-rest): the small dEQP shapes of
//! `tools/mali/testdata/deqp_shapes/index.tsv` that no other reference drew,
//! each drawn from its `deqp_shapes/<family>_<shape>` example pair with
//! `tools/mali/deqp_ref.sh` (the drm-shim instrument). One test per
//! reference. Four compile byte-equal and run ([`MATCHED`]); one kept loop
//! compiles to Mesa's words with the loop cap's ([`CAPPED`], compared in
//! `src/shaderc/online/mali/tests.rs`). One compiles and differs
//! ([`DIFFERS`]) and four are refused ([`REFUSED`], the whole
//! `MaliError` message): those tests are `#[ignore]`d, and
//! `refusals_pinned` asserts each one's state now. The lowering that moves
//! one moves its entry in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_gaps
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal, without the `deqp_gaps_` prefix.
const MATCHED: &[&str] = &["intval_bool_literal", "intval_float_compare", "intval_harness_isok", "intval_other"];

/// The references that compile and differ from their expect.json.
const DIFFERS: &[&str] = &["mixprec_fs_varying_plain"];

/// The kept loops that compile: their words are Mesa's with the loop cap's
/// (the pass counter, which the shared compare does not edit), compared in
/// `src/shaderc/online/mali/tests.rs` (`deqp_gaps_<name>_is_mesa_26_2_1_with_the_loop_cap`).
const CAPPED: &[&str] = &["loops_fs_kept_while"];

/// Every refused reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    (
        "intval_int_uniform",
        "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)",
    ),
    (
        "loops_fs_kept_for_index",
        "Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet",
    ),
    (
        "mixprec_fs_mixed_other",
        "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet",
    ),
    (
        "mixprec_fs_varying_other",
        "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet",
    ),
];

/// intval/bool_literal: constant nested structs with bool and int fields in
/// the vertex shader (`constant_expressions.complex_types.nested_struct_vertex`).
#[test]
fn intval_bool_literal() {
    mali_ref::assert_matches("deqp_gaps_intval_bool_literal");
}

/// intval/float_compare: an if/else on a float varying compare
/// (`conditionals.if.if_else_fragment`).
#[test]
fn intval_float_compare() {
    mali_ref::assert_matches("deqp_gaps_intval_float_compare");
}

/// intval/harness_isok: the shader-library fragment harness alone, a
/// constant `out0` (`preprocessor.basic.empty_directive_fragment`).
#[test]
fn intval_harness_isok() {
    mali_ref::assert_matches("deqp_gaps_intval_harness_isok");
}

/// intval/int_uniform: an `ivec2` of a varying broadcast to `ivec4` and
/// compared with an `ivec4` uniform
/// (`swizzles.vector_swizzles.mediump_ivec2_gggg_fragment`).
#[test]
#[ignore = "refused: Valhall back end defect (IrIndex); pinned by refusals_pinned"]
fn intval_int_uniform() {
    mali_ref::assert_matches("deqp_gaps_intval_int_uniform");
}

/// intval/other: `__LINE__` in a constant int
/// (`preprocessor.predefined_macros.line_1_fragment`).
#[test]
fn intval_other() {
    mali_ref::assert_matches("deqp_gaps_intval_other");
}

/// loops/fs_kept_for_index: a mat2 written and read in run-time-bound loops
/// of a mediump fragment shader
/// (`indexing.matrix_subscript.mat2_dynamic_loop_write_dynamic_loop_read_fragment`).
#[test]
#[ignore = "refused: a loop Mesa keeps in a mediump fragment shader; pinned by refusals_pinned"]
fn loops_fs_kept_for_index() {
    mali_ref::assert_matches("deqp_gaps_loops_fs_kept_for_index");
}

/// loops/fs_kept_while: a while loop with a selected iteration count
/// (`loops.while_constant_iterations.select_iteration_count_fragment`).
/// A kept loop: it compiles, and its words are Mesa's with the loop cap's
/// ([`CAPPED`]).
#[test]
fn loops_fs_kept_while() {
    let r = mali_ref::Reference::load("deqp_gaps_loops_fs_kept_while");
    if let Err(e) = mali_ref::compile_pair(&r.vs, &r.fs) {
        panic!("deqp_gaps_loops_fs_kept_while: refused: {e}");
    }
}

/// mixprec/fs_mixed_other: a random-shader fragment shader of int, bool
/// and mediump float globals (`random.conditionals.fragment.27`).
#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp; pinned by refusals_pinned"]
fn mixprec_fs_mixed_other() {
    mali_ref::assert_matches("deqp_gaps_mixprec_fs_mixed_other");
}

/// mixprec/fs_varying_other: a function returning one of two swizzles of a
/// mediump varying (`return.conditional_return_always_fragment`).
#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn mixprec_fs_varying_other() {
    mali_ref::assert_matches("deqp_gaps_mixprec_fs_varying_other");
}

/// mixprec/fs_varying_plain: a random-shader vertex shader of constant
/// vec4 chains into a plain mediump `v_color`
/// (`random.basic_expression.vertex.8`).
#[test]
#[ignore = "differs: vs.varying is 25 words ours, 33 Mesa's, and the position FAU words differ; pinned by refusals_pinned"]
fn mixprec_fs_varying_plain() {
    mali_ref::assert_matches("deqp_gaps_mixprec_fs_varying_plain");
}

/// Each of [`REFUSED`] is refused with exactly its message (not a
/// substring), each of [`DIFFERS`] compiles and differs, and the four
/// lists ([`CAPPED`] too) cover every `deqp_gaps_*` reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_gaps_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> =
        MATCHED.iter().chain(DIFFERS).chain(CAPPED).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_gaps_* reference has no entry here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_gaps_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to MATCHED or DIFFERS")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_gaps_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to MATCHED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "the pinned states moved:\n{}", changed.join("\n"));
}
