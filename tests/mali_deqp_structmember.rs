// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A float member of a uniform struct in a mediump fragment shader, against
//! the Mesa 26.2.1 references `tools/mali/testdata/deqp_structmember_*`
//! (issue #11306, the `shaders.linkage.uniform_struct*` cases).
//!
//! Mesa keeps a struct's float member fp32 in a mediump shader
//! (lower_precision.cpp:1029-1031 asks for a float base type): the block
//! holds one fp32 word a component, and the shader narrows what it reads
//! (`V2F32_TO_V2F16.discard r0, u2.w1, u2.w0`). The walk read the member so,
//! but the routes of a colour of booleans and of integers' floats laid the
//! block out as fp16 halves, so the frame pushed `0x00004c00` where the
//! code reads the fp32 `0x41800000`: Mesa's words with other data, a wrong
//! value on the board. Each pair is verbatim from a host run's `.qpa`; each
//! reference was drawn with `tools/mali/deqp_ref.sh <vert> <frag>
//! tools/mali/testdata/deqp_structmember_<case> --instrument shim`, which sets
//! every member (`val.a` the first value index, `val.b` the next); they are
//! in `tests/mali_ref`'s `STRUCT_VALUES`, so the FAU words are compared too.
//!
//! - `uniform_struct`: `struct { mediump float a; mediump float b; }` read
//!   by both stages: byte-equal, FAU words too.
//! - `uniform_struct_partial`: the same struct, `val.a` read by the vertex
//!   shader and `val.b` by the fragment shader: byte-equal.
//! - `uniform_struct_partial_vec4`: `vec4` members, the fragment shader
//!   reads `val.b.y` alone. Mesa loads each word of a member the shader
//!   reads on its own: its FAU holds `val.b.y` at `u2.w0` and no `val.b.x`
//!   (the fp32 layout alone pushed the member's load from its first word,
//!   `val.b.x` at `u2.w0` and `val.b.y` at `u2.w1`). Byte-equal. The case
//!   passed on the board before only by chance: with the fp16 layout the
//!   word of `val.b`'s halves (1.0, 2.0), read as an fp32, is 2.0037.
//! - `uniform_struct_vec4` (`deqp_audit_o_linkage_uniform_struct_vec4`):
//!   two members read at one lane each (`val.b.y + val.a.x`, words 5 and 0).
//!   Mesa narrows each lane alone (`FADD.f32 r0.h0, u2.w0` and `FADD.f32
//!   r1.h0, u2.w1`); it pairs two narrowings only when their words are
//!   neighbours, whose loads it joins (`uniform_struct`). These routes pair
//!   them anyway: refused until they draw Mesa's words.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_structmember
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::FsUniform`'s text.
const FS_UNIFORM: &str = "a mediump fragment shader uniform other than";

#[test]
fn structmember_uniform_struct() {
    mali_ref::assert_matches("deqp_structmember_uniform_struct");
}

#[test]
fn structmember_uniform_struct_partial() {
    mali_ref::assert_matches("deqp_structmember_uniform_struct_partial");
}

#[test]
fn structmember_uniform_struct_partial_vec4() {
    mali_ref::assert_matches("deqp_structmember_uniform_struct_partial_vec4");
}

#[test]
fn structmember_uniform_struct_vec4_is_refused() {
    mali_ref::assert_refused_ref("deqp_audit_o_linkage_uniform_struct_vec4", FS_UNIFORM);
}
