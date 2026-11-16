// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Self-test of `tests/mali_ref/mod.rs`, the shared byte-compare of a Mali
//! program against an expect.json reference. Two references the compiler
//! already matches (`tools/mali/testdata/cases.txt`: `tri` and
//! `tri_varying`, every region `equal`), a changed shader that must differ,
//! and one refused shape.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_ref_selftest
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
fn tri_matches_its_reference() {
    mali_ref::assert_matches("tri");
}

#[test]
fn tri_varying_matches_its_reference() {
    mali_ref::assert_matches("tri_varying");
}

#[test]
fn a_changed_shader_is_reported_with_a_word_diff() {
    let r = mali_ref::Reference::load("tri");
    let fs = r.fs.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(0.0, 1.0, 0.0, 1.0)");
    assert_ne!(fs, r.fs);
    let pair = mali_ref::compile_pair(&r.vs, &fs).expect("the changed pair compiles");
    let d = mali_ref::differences(&r, &pair);
    assert!(!d.is_empty(), "a different colour must differ from the reference");
    assert!(d.iter().any(|m| m.starts_with("fs.fragment")), "{d:?}");
    assert!(d.iter().all(|m| !m.starts_with("vs.")), "{d:?}");
}

#[test]
fn a_refused_shape_stays_refused() {
    let vs = "attribute vec2 pos; uniform int n; void main(){ gl_Position = vec4(pos, float(n), 1.0); }";
    let fs = "precision mediump float; void main(){ gl_FragColor = vec4(1.0); }";
    mali_ref::assert_refused(vs, fs, "a uniform other than a float, vec2, vec3, vec4 or mat4");
}

#[test]
#[should_panic(expected = "but the pair compiles")]
fn a_pair_that_compiles_is_not_refused() {
    let r = mali_ref::Reference::load("tri");
    mali_ref::assert_refused(&r.vs, &r.fs, "anything");
}
