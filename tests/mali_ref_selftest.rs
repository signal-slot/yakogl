// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Self-test of `tests/mali_ref/mod.rs`, the shared byte-compare of a Mali
//! program against an expect.json reference. Two references the compiler
//! already matches (`tools/mali/testdata/cases.txt`: `tri` and
//! `tri_varying`, every region `equal`), a changed shader that must differ,
//! one refused shape, and the uniform words of each GL type.
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

/// A vertex shader that writes gl_PointSize is compared by its position
/// program without the store (the reference's draw is of triangles), with
/// the secondary offset counted from that program and Mesa's FAU table.
#[test]
fn a_point_size_shader_is_compared_by_its_no_psiz_program() {
    let r = mali_ref::Reference::load("deqp_varylink_va_multi3");
    assert!(r.vs.contains("gl_PointSize"));
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
    assert!(pair.vs.no_psiz.is_some_and(|off| off > 0), "a no_psiz copy after the position program");
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "{}", d.join("\n"));
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

/// The word `gl_ref.py --uniform-values` leaves in a reference follows the
/// uniform's GL type, as Mesa uploads it (measured: the ivec3 of
/// `deqp_intval_reslib_mixed_fs` is `00000010 00000011 00000012`, the bvec3
/// of `deqp_intval_reslib_bvec_fs` is `ffffffff 00000000 ffffffff`).
#[test]
fn a_uniform_word_follows_its_gl_type() {
    use yakogl::gl::consts as gl;
    let ty = |t: u32| u16::try_from(t).expect("a GL type fits 16 bits");
    let names = ["u_f".to_string(), "u_i".to_string(), "u_b".to_string()];
    let rows = [
        ("u_f".to_string(), 0, 2, ty(gl::GL_FLOAT_VEC2)),
        ("u_i".to_string(), 2, 3, ty(gl::GL_INT_VEC3)),
        ("u_b".to_string(), 5, 3, ty(gl::GL_BOOL_VEC3)),
    ];
    let words: Vec<u32> = (0..8).map(|d| mali_ref::uniform_value(&names, &rows, d)).collect();
    assert_eq!(words, [16f32.to_bits(), 17f32.to_bits(), 32, 33, 34, u32::MAX, 0, u32::MAX]);
}
