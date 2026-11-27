// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_missing_<family>_<shape>` (plan #377,
//! item ref-missing): the four dEQP shapes that `families-refresh-2` left out
//! of `tools/mali/testdata/deqp_shapes/index.tsv` (loops `vs_for_constant`,
//! loops `vs_for_uniform`, loops `fs_kept_for`, mixprec `fs_highp_declared`).
//! Each reference is the Mesa 26.2.1 drm-shim reference of an existing pair of
//! the same shape (`deqp_loops_vs_for_const`, `deqp_loops_vs_for_uni`,
//! `deqp_loops_fs_for_uni`, `deqp_mixprec3_hp_vec3_sum`), the same shader
//! bytes and expect.json with only the `trace` name changed. One test per
//! reference; a refused one is `#[ignore]`d and `refusals_pinned` asserts its
//! whole `MaliError` message. The lowering that moves one moves its entry in
//! the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_missing
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references that compile byte-equal, without the `deqp_missing_` prefix.
const MATCHED: &[&str] = &["loops_vs_for_constant"];

/// The references that compile and differ from their expect.json: the kept
/// mediump loop's words are Mesa's with the loop cap's (20 words against 17;
/// compared for `deqp_loops_fs_for_uni`, the same pair, in
/// `src/shaderc/online/mali/tests.rs`).
const DIFFERS: &[&str] = &["loops_fs_kept_for"];

/// Every refused reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("loops_vs_for_uniform", "Valhall: a loop in a vertex shader is not lowered yet"),
    (
        "mixprec_fs_highp_declared",
        "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet",
    ),
];

/// loops/vs_for_constant: a mediump int `for` with a literal bound in the
/// vertex shader (`loops.for_constant_iterations.basic_mediump_int_vertex`).
#[test]
fn loops_vs_for_constant() {
    mali_ref::assert_matches("deqp_missing_loops_vs_for_constant");
}

/// loops/vs_for_uniform: the same loop bounded by an int uniform
/// (`loops.for_uniform_iterations.basic_mediump_int_vertex`).
#[test]
#[ignore = "refused: a loop in a vertex shader; pinned by refusals_pinned"]
fn loops_vs_for_uniform() {
    mali_ref::assert_matches("deqp_missing_loops_vs_for_uniform");
}

/// loops/fs_kept_for: the same loop in a mediump fragment shader
/// (`loops.for_uniform_iterations.basic_mediump_int_fragment`).
#[test]
#[ignore = "differs: the kept loop's words are Mesa's with the loop cap's; pinned by refusals_pinned"]
fn loops_fs_kept_for() {
    mali_ref::assert_matches("deqp_missing_loops_fs_kept_for");
}

/// mixprec/fs_highp_declared: a highp float plus vec3 in a fragment shader
/// whose colour is mediump (`operator.binary_operator.add.highp_float_vec3_fragment`).
#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn mixprec_fs_highp_declared() {
    mali_ref::assert_matches("deqp_missing_mixprec_fs_highp_declared");
}

/// Each of [`REFUSED`] is refused with exactly its message, each of
/// [`DIFFERS`] compiles and differs, and the three lists cover every
/// `deqp_missing_*` reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_missing_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = MATCHED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_missing_* reference has no entry here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_missing_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_missing_{name}"));
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
