// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali compiler against the `deqp_uniarray_*`
//! references (Mesa 26.2.1, `tools/mali/testdata/`; `docs/notes/mali-deqp-uniarray.md`):
//! uniform arrays and uniforms read at a run-time index. A reference still
//! refused is ignored with the refusal that stops it and the family that
//! lowers it. The references were drawn with `--uniform-values`, except
//! the three with a struct uniform (`docs/notes/mali-deqp-uniarray.md`
//! section 7): gl_ref.py sets no array element, so an array's words are 0,
//! and `tests/mali_ref` gives every other uniform word the value the draw
//! set (`assert_matches`). `refusals_pinned` runs now and
//! pins each refusal's text (the compiler's, without "Valhall: " and
//! " is not lowered yet"; the vertex shader's when it refuses, otherwise the
//! fragment shader's). When a lowering moves a refusal it moves the pin; when
//! a reference compiles its ignore goes and the test must pass.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_uniarray
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp (compare_float's abs and ?: on the elements, Mesa's FMAX of the three abs and one FCMP), owned by constop"]
fn api_float() {
    mali_ref::assert_matches("deqp_uniarray_api_float");
}

#[test]
fn dyn_vec2_fs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec2_fs");
}

/// The vertex walk's fp16 element at a mediump int's index (nir.rs
/// `uniform_idx16`): `LSHIFT_OR.i32` by the byte table's 2 (a vec2's word)
/// or 3 (a vec3's or vec4's two), `IADD_IMM.i32 #0x10`, `LD_PKA.i32`,
/// `.i48` or `.i64` sunk to its first reader, the `FADD.v2f16` sums of the
/// registers, and the varying flat with its kept constants.
#[test]
fn dyn_vec2_vs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec2_vs");
}

#[test]
fn dyn_vec3_fs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec3_fs");
}

#[test]
fn dyn_vec3_vs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec3_vs");
}

#[test]
fn dyn_vec4_fs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec4_fs");
}

#[test]
fn dyn_vec4_vs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec4_vs");
}

#[test]
fn float_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_float_const_fs");
}

#[test]
fn float_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_float_const_vs");
}

#[test]
fn loop_float_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_float_fs");
}

/// The vertex loop unrolled (plan #367 lower-int-loops), each `u_arr[i]`
/// at its folded counter the constant-index read: byte-equal to
/// `float_const_vs`, as Mesa's.
#[test]
fn loop_float_vs() {
    mali_ref::assert_matches("deqp_uniarray_loop_float_vs");
}

#[test]
fn loop_vec2_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_vec2_fs");
}

#[test]
fn loop_vec3_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_vec3_fs");
}

#[test]
fn loop_vec4_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_vec4_fs");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (compare_float's arithmetic), owned by constop"]
fn struct_array() {
    mali_ref::assert_matches("deqp_uniarray_struct");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp (the int member s.c's S32_TO_F32 beside an fp32 member lane), owned by intval"]
fn struct_member_fs() {
    mali_ref::assert_matches("deqp_uniarray_struct_member_fs");
}

/// The int member `s.c` read as `float(s.c)` (nir.rs `int_float_only`):
/// `S32_TO_F32 r2, u7.w0` of its pushed word.
#[test]
fn struct_member_vs() {
    mali_ref::assert_matches("deqp_uniarray_struct_member_vs");
}

#[test]
fn vec2_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_vec2_const_fs");
}

#[test]
fn vec2_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_vec2_const_vs");
}

#[test]
fn vec3_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_vec3_const_fs");
}

#[test]
fn vec3_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_vec3_const_vs");
}

#[test]
fn vec4_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_vec4_const_fs");
}

#[test]
fn vec4_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_vec4_const_vs");
}

/// Each ignored reference's refusal at the tip.
const REFUSALS: &[(&str, &str)] = &[
    (
        "api_float",
        "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp",
    ),
    (
        "struct",
        "a gl_FragColor lane other than the varying's own lane or a constant",
    ),
    (
        "struct_member_fs",
        "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp",
    ),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        mali_ref::assert_refused_ref(&format!("deqp_uniarray_{name}"), text);
    }
}
