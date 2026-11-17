// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali compiler against the `deqp_uniarray_*`
//! references (Mesa 26.2.1, `tools/mali/testdata/`; `docs/notes/mali-deqp-uniarray.md`):
//! uniform arrays and uniforms read at a run-time index. Every reference is
//! refused at the tip, so each byte-compare is ignored with the refusal that
//! stops it and the family that lowers it. `refusals_pinned` runs now and
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
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn api_float() {
    mali_ref::assert_matches("deqp_uniarray_api_float");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn dyn_vec2_fs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec2_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn dyn_vec2_vs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec2_vs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn dyn_vec3_fs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec3_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn dyn_vec3_vs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec3_vs");
}

#[test]
#[ignore = "refused: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, lowered by lower-uniarray"]
fn dyn_vec4_fs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec4_fs");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member, owned by unitype"]
fn dyn_vec4_vs() {
    mali_ref::assert_matches("deqp_uniarray_dyn_vec4_vs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn float_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_float_const_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn float_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_float_const_vs");
}

#[test]
#[ignore = "refused: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, lowered by lower-uniarray"]
fn loop_float_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_float_fs");
}

#[test]
#[ignore = "refused: a loop in a vertex shader, owned by loops"]
fn loop_float_vs() {
    mali_ref::assert_matches("deqp_uniarray_loop_float_vs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn loop_vec2_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_vec2_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn loop_vec3_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_vec3_fs");
}

#[test]
#[ignore = "refused: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, lowered by lower-uniarray"]
fn loop_vec4_fs() {
    mali_ref::assert_matches("deqp_uniarray_loop_vec4_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn struct_array() {
    mali_ref::assert_matches("deqp_uniarray_struct");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn struct_member_fs() {
    mali_ref::assert_matches("deqp_uniarray_struct_member_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn struct_member_vs() {
    mali_ref::assert_matches("deqp_uniarray_struct_member_vs");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, owned by unitype"]
fn vec2_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_vec2_const_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn vec2_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_vec2_const_vs");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, owned by unitype"]
fn vec3_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_vec3_const_fs");
}

#[test]
#[ignore = "refused: a uniform array, lowered by lower-uniarray"]
fn vec3_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_vec3_const_vs");
}

#[test]
#[ignore = "refused: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first), owned by arith, plan 367"]
fn vec4_const_fs() {
    mali_ref::assert_matches("deqp_uniarray_vec4_const_fs");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member, owned by unitype"]
fn vec4_const_vs() {
    mali_ref::assert_matches("deqp_uniarray_vec4_const_vs");
}

/// Each ignored reference's refusal at the tip.
const REFUSALS: &[(&str, &str)] = &[
    (
        "api_float",
        "a uniform array",
    ),
    (
        "dyn_vec2_fs",
        "a uniform array",
    ),
    (
        "dyn_vec2_vs",
        "a uniform array",
    ),
    (
        "dyn_vec3_fs",
        "a uniform array",
    ),
    (
        "dyn_vec3_vs",
        "a uniform array",
    ),
    (
        "dyn_vec4_fs",
        "a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform",
    ),
    (
        "dyn_vec4_vs",
        "a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member",
    ),
    (
        "float_const_fs",
        "a uniform array",
    ),
    (
        "float_const_vs",
        "a uniform array",
    ),
    (
        "loop_float_fs",
        "a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform",
    ),
    (
        "loop_float_vs",
        "a loop in a vertex shader",
    ),
    (
        "loop_vec2_fs",
        "a uniform array",
    ),
    (
        "loop_vec3_fs",
        "a uniform array",
    ),
    (
        "loop_vec4_fs",
        "a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform",
    ),
    (
        "struct",
        "a uniform array",
    ),
    (
        "struct_member_fs",
        "a uniform array",
    ),
    (
        "struct_member_vs",
        "a uniform array",
    ),
    (
        "vec2_const_fs",
        "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp",
    ),
    (
        "vec2_const_vs",
        "a uniform array",
    ),
    (
        "vec3_const_fs",
        "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp",
    ),
    (
        "vec3_const_vs",
        "a uniform array",
    ),
    (
        "vec4_const_fs",
        "a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first)",
    ),
    (
        "vec4_const_vs",
        "a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member",
    ),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        mali_ref::assert_refused_ref(&format!("deqp_uniarray_{name}"), text);
    }
}
