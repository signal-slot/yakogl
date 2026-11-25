// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsuniarr2_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsuniarr2b`) against the Mali online
//! compiler: the `random` shape `tests/mali_deqp_vsuniarr.rs` leaves uncovered.
//! The refusal is `MaliError::UniformArray`, "a uniform array is not lowered
//! yet", in the vertex shader. The ones a lowering takes run ([`MATCHED`]); the
//! rest are `#[ignore]`d with their refusal. `refusals_pinned` runs now and
//! holds each refusal's exact text, so a lowering must move its pin.
//!
//! HOST-ONLY measure at 670913e1 (`deqp-host-measure.sh --cases`, 19 cases of
//! `dEQP-GLES2.functional.uniform_api.random.*` that the board slices
//! `2026-11-24-t6-gles2-S{1..4}-8ff01055` refused with the vertex-shader
//! uniform-array refusal): all 19 refuse on the host, in five shapes of the
//! measure (bool_array 8, mat_array 5, int_array 3, array_in_struct 2,
//! struct_array 1). Each case is one program with a random mix of uniforms of
//! one dEQP type set, compared in the vertex shader (and, in some, in the
//! fragment shader too) to constants; unread elements are `// UNUSED`.
//! Cases that declare a sampler (`sampler2D`, `samplerCube`) are left out:
//! `gl_ref.py --uniform-values` does not set one, so Mesa gives no reference.
//!
//! | Reference | Case | The vertex shader's arrays and members | Read |
//! |---|---|---|---|
//! | `mat3_part` | random.10 | `mat3 u_var0[5]`, `ivec4 u_var1` | `u_var0[2]` alone |
//! | `struct_mat4_arr` | random.15 | struct `{float m0; mat4 m1[3]}`, `float`, `ivec3`, `float` | `m1[1]` alone |
//! | `mat2_struct` | random.17 | `mat2 u_var0[4]`, `mat3`, struct `{ivec2, bvec2}` | `u_var0[0]`, `[2]` |
//! | `mat4_mixed` | random.53 | `mat4 u_var0[3]`, `vec3`, two `int`, `bool` | every element |
//! | `struct_vec3_mat2` | random.57 | `mat3`, struct `{vec3; vec3 m0[1]; mat3; mat2 m3[3]}` | `m3[0]`, `m3[2]` |
//! | `ivec3_part` | random.66 | `ivec3 u_var0[2]` | `u_var0[1]` alone |
//! | `bvec3_part` | random.67 | `vec3`, `bvec3 u_var1[5]` | `u_var1[1]`, `[3]` |
//! | `struct_nested` | random.74 | `float`, `bool[4]`, `float[4]`, struct array `{bool, vec3, bvec4[2], mat3[4], ivec3}` | `m0[1]`: bool, `m3[2]`, `m4` |
//!
//! Past the eight: random.6, 24, 37, 61 (each declares a sampler) and
//! random.71, 75, 76, 77, 83, 94, 99 (the same kinds of arrays and members as
//! above, other sizes and values).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsuniarr2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix.
const UNIFORM_ARRAY: &str = "a uniform array is not lowered yet";

/// The references the compiler makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("mat3_part", UNIFORM_ARRAY),
    ("struct_mat4_arr", UNIFORM_ARRAY),
    ("mat2_struct", UNIFORM_ARRAY),
    ("mat4_mixed", UNIFORM_ARRAY),
    ("struct_vec3_mat2", UNIFORM_ARRAY),
    ("ivec3_part", UNIFORM_ARRAY),
    ("bvec3_part", UNIFORM_ARRAY),
    ("struct_nested", UNIFORM_ARRAY),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsuniarr2_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsuniarr2_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsuniarr2_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// `uniform mediump mat3 u_var0[5]`: only `u_var0[2]` is read.
#[test]
#[ignore = "refused: a uniform array (uniform mat3 u_var0[5])"]
fn mat3_part() {
    mali_ref::assert_matches("deqp_vsuniarr2_mat3_part");
}

/// A struct with `mat4 m1[3]`: only `m1[1]` is read.
#[test]
#[ignore = "refused: a uniform array (struct member mat4 m1[3])"]
fn struct_mat4_arr() {
    mali_ref::assert_matches("deqp_vsuniarr2_struct_mat4_arr");
}

/// `uniform mediump mat2 u_var0[4]`: elements 0 and 2 are read.
#[test]
#[ignore = "refused: a uniform array (uniform mat2 u_var0[4])"]
fn mat2_struct() {
    mali_ref::assert_matches("deqp_vsuniarr2_mat2_struct");
}

/// `uniform mediump mat4 u_var0[3]`, every element compared, beside scalars.
#[test]
#[ignore = "refused: a uniform array (uniform mat4 u_var0[3])"]
fn mat4_mixed() {
    mali_ref::assert_matches("deqp_vsuniarr2_mat4_mixed");
}

/// A struct with `vec3 m0[1]` and `mat2 m3[3]`: `m3[0]` and `m3[2]` are read.
#[test]
#[ignore = "refused: a uniform array (struct members vec3 m0[1], mat2 m3[3])"]
fn struct_vec3_mat2() {
    mali_ref::assert_matches("deqp_vsuniarr2_struct_vec3_mat2");
}

/// `uniform mediump ivec3 u_var0[2]`: `u_var0[1]` is read.
#[test]
#[ignore = "refused: a uniform array (uniform ivec3 u_var0[2])"]
fn ivec3_part() {
    mali_ref::assert_matches("deqp_vsuniarr2_ivec3_part");
}

/// `uniform bvec3 u_var1[5]`: elements 1 and 3 are read.
#[test]
#[ignore = "refused: a uniform array (uniform bvec3 u_var1[5])"]
fn bvec3_part() {
    mali_ref::assert_matches("deqp_vsuniarr2_bvec3_part");
}

/// A struct array holding `bvec4 m2[2]`, `mat3 m3[4]`: three members are read.
#[test]
#[ignore = "refused: a uniform array (uniform bool u_var1[4], float u_var2[4], struct arrays)"]
fn struct_nested() {
    mali_ref::assert_matches("deqp_vsuniarr2_struct_nested");
}
