// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsuniarr_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsuniarr`,
//! `docs/notes/mali-deqp-vsuniarr.md`) against the Mali online compiler. The
//! refusal is `MaliError::UniformArray`, "a uniform array is not lowered
//! yet", in the vertex shader: 193 board Fail at 8ff01055. The ones a
//! lowering takes run ([`MATCHED`]); the rest are `#[ignore]`d with their
//! refusal. `refusals_pinned` runs now and holds each refusal's exact text
//! (the vertex shader's: it is compiled first, and every fragment shader
//! here is the varying alone), so a lowering must move its pin.
//!
//! HOST-ONLY shapes at ab919063: `deqp-host-measure.sh` over the 193 board
//! cases of the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055` (all in S1,
//! all `uniform_api`, all board Fail). All 193 refuse with this text first,
//! in the vertex shader, at `nir.rs:1103` (`uniform_of`: an array
//! `array_element` does not take, any element but float, vec2, vec3, vec4).
//! Every index is a constant; no case reads an array at a run-time index.
//!
//! | Shape | Cases | The vertex shader's arrays | Reference |
//! |---|--:|---|---|
//! | `bvec4` | 32 | `uniform bvec4 u_var[3]`, every element compared | `bvec4` (here) |
//! | `ivec4` | 22 | `uniform mediump ivec4 u_var[3]` | `ivec4` (here) |
//! | `mat4` | 16 | `uniform mediump mat4 u_var[3]` | `mat4` (here) |
//! | `multi` | 18 | `mediump float u_var_0[3]`, `mediump ivec3 u_var_1[3]`, `bvec2 u_var_2[3]` in one block | `multi` (here) |
//! | `struct_mat` | 14 | a struct's members `mat4 m2[3]`, `mat2 m3[3]` behind `mat4 m0`, `mat2 m1` | `struct_mat` (here) |
//! | `nested_mat` | 10 | `mat4 ms1[2]` and the struct array `ms2[2]` (`mat4`, `mat2`) of a nested struct | `nested_mat` (here) |
//! | `unused_bvec4` | 4 | a struct's `bool m2[3]`, `bvec4 m3[3]`, read: `m0` and `m3[1]` alone | `unused_bvec4` (here) |
//! | `unused_mat2` | 4 | a struct's `mat4 m2[3]`, `mat2 m3[3]`, read: `m0` and `m3[1]` alone | `unused_mat2` (here) |
//! | `bool` | 32 | `uniform bool u_var[3]` | covered: `deqp_unif2_arr_bool_array` |
//! | `int` | 22 | `uniform mediump int u_var[3]` | covered: `deqp_unif2_arr_int_array` |
//! | `random` | 19 | `uniform_api.random.*`: one program a case, int, bool and matrix arrays, plain and struct members, most read in part | none (past the 8); `random.10` is `deqp_unif2_arr_mat_array` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsuniarr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix.
const UNIFORM_ARRAY: &str = "a uniform array is not lowered yet";
/// The references the compiler makes byte-equal: the int and bool vector
/// arrays, whose elements load since plan #377 spike-uniform-rest3 (nir.rs
/// `array_element`) and whose compares plan #387 mali-train-3b lowers (a
/// vector's `==` in nir.rs `vs_reduce`, the product of the compare results
/// in `vs_mask_tree`).
const MATCHED: &[&str] = &["bvec4", "ivec4", "multi", "unused_bvec4"];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("mat4", UNIFORM_ARRAY),
    ("struct_mat", UNIFORM_ARRAY),
    ("nested_mat", UNIFORM_ARRAY),
    ("unused_mat2", UNIFORM_ARRAY),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsuniarr_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsuniarr_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsuniarr_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// `uniform bvec4 u_var[3]`: twelve 32-bit words, FAU 11 to 22. Mesa
/// folds each `==` with a constant bvec4 into the words themselves:
/// `LSHIFT_AND.i32` of the lanes expected true, `LSHIFT_OR.i32` and one
/// `.not` of the lanes expected false.
#[test]
fn bvec4() {
    mali_ref::assert_matches("deqp_vsuniarr_bvec4");
}

/// `uniform mediump ivec4 u_var[3]`: twelve 32-bit words, FAU 11 to 22;
/// each lane one `ICMP_OR.u32.eq.m1` or `ICMP_AND` with its constant (a byte
/// lane of an embedded table, an `IADD_IMM` or a pushed FAU constant).
#[test]
fn ivec4() {
    mali_ref::assert_matches("deqp_vsuniarr_ivec4");
}

/// `uniform mediump mat4 u_var[3]`: fp16, 24 words of lane pairs, FAU 11
/// to 34 (a column two words, an element eight); `FADD_IMM.v2f16` of each
/// word and an `FMAX.v2f16` tree of the 48 `abs`, one `FCMP_OR.v2f16.lt`
/// with the 0.05 at FAU 35.
#[test]
#[ignore = "refused: a uniform array (uniform mediump mat4 u_var[3])"]
fn mat4() {
    mali_ref::assert_matches("deqp_vsuniarr_mat4");
}

/// `mediump float u_var_0[3]`, `mediump ivec3 u_var_1[3]` and `bvec2
/// u_var_2[3]` in one block. Mesa pushes the bvec2 words first (FAU 11 to
/// 16), then the fp16 floats, one a word (17 to 19), then the ivec3 words
/// (20 to 28): not the declaration's order.
#[test]
fn multi() {
    mali_ref::assert_matches("deqp_vsuniarr_multi");
}

/// A struct's `mat4 m2[3]` and `mat2 m3[3]` behind `mat4 m0` and `mat2 m1`:
/// fp32 members, 80 words in declaration order (FAU 11 to 90), each pair
/// narrowed with `V2F32_TO_V2F16` before the fp16 compare tree.
#[test]
#[ignore = "refused: a uniform array (a struct's mat4 and mat2 array members)"]
fn struct_mat() {
    mali_ref::assert_matches("deqp_vsuniarr_struct_mat");
}

/// The nested struct's `mat4 ms1[2]` and its struct array `ms2[2]` (a mat4
/// and a mat2 each): 96 fp32 words in declaration order, FAU 11 to 106.
#[test]
#[ignore = "refused: a uniform array (a nested struct's mat4 array and struct array)"]
fn nested_mat() {
    mali_ref::assert_matches("deqp_vsuniarr_nested_mat");
}

/// `unused_uniforms.bool_bvec4`: of `bool m0; bvec4 m1; bool m2[3]; bvec4
/// m3[3]` the shader reads `m0` and `m3[1]` alone, and Mesa pushes those
/// five words alone (FAU 11 to 15: ffffffff, 0, ffffffff, ffffffff, 0 for
/// `m0` true and `m3[1]` true, false, true, false, so not `m0` first).
#[test]
fn unused_bvec4() {
    /* drawn with its struct members set (the FAU words above): compared
     * with their values, as `mali_ref::STRUCT_VALUES` does for the
     * references it lists */
    let r = mali_ref::Reference::load("deqp_vsuniarr_unused_bvec4");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("the pair is refused: {e}"));
    let d = mali_ref::differences_members(&r, &pair, true);
    assert!(d.is_empty(), "deqp_vsuniarr_unused_bvec4: differs from its reference:\n{}", d.join("\n"));
}

/// `unused_uniforms.mat4_mat2`: `m0` and `m3[1]` alone are read, 16 and 4
/// fp32 words (FAU 11 to 30).
#[test]
#[ignore = "refused: a uniform array (one mat2 element of a struct's array member read)"]
fn unused_mat2() {
    mali_ref::assert_matches("deqp_vsuniarr_unused_mat2");
}
