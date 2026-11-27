// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_fsunif_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-fsunif`,
//! `docs/notes/mali-deqp-fsunif.md`) against the Mali online compiler. The
//! refusal is `MaliError::FsUniform`, "a mediump fragment shader uniform
//! other than a float, vec2, vec3, vec4, vec4 array or int read in its own
//! fp16 lane pairs or as one lane, or one declared highp": 271 board Fail
//! and 1 NotSupported at 8ff01055. The ones a lowering takes run
//! ([`MATCHED`]); the rest are `#[ignore]`d with their refusal.
//! `refusals_pinned` runs now and holds each refusal's exact text (the
//! fragment shader's: every vertex shader here compiles), so a lowering
//! must move its pin.
//!
//! HOST-ONLY shapes at 7462e3d5: `deqp-host-measure.sh` over the 272 board
//! cases of the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055`, all 272
//! refused with this text first (159 programs), each put at one refusal
//! site by a probe.
//!
//! | Shape | Cases | Board Fail / NS | Site | References |
//! |---|--:|---|---|---|
//! | `arr_bvec`: `uniform bvec4 u_var[3]`, `uniform_api.*.basic_array.bvec4*_fragment` | 16 | 16 / 0 | `select.rs:628` (the type) | `arr_bvec4` (here) |
//! | `arr_ivec`: `uniform mediump ivec4 u_var[3]` | 11 | 11 / 0 | `select.rs:628` | `arr_ivec4` (here) |
//! | `arr_mat`: `uniform mediump mat4 u_var[3]` | 8 | 8 / 0 | `select.rs:628` | `arr_mat4` (here) |
//! | `struct_int`: a struct of `int`, `ivec4` and arrays of them (`array_in_struct` 11, `nested_structs_arrays` 7) | 18 | 18 / 0 | `select.rs:628` | `struct_int_ivec4` (here) |
//! | `mat16`: `shaders.matrix.*_{lowp,mediump}_*_fragment`, a mat2/mat3/mat4 whose halves are read across the fp16 pairs | 32 | 32 / 0 | `nir_fs.rs:10089` (two uniform halves that are no aligned pair) | `mat4_add_float`, `mat2_mul_mat2` (here) |
//! | `mat32`: `shaders.matrix.*_highp_*_fragment`, a highp matrix into the mediump colour | 11 | 11 / 0 | `nir_fs.rs:9678` (two fp32 words that are no aligned pair) | `highp_mat4_add_float` (here) |
//! | `struct32`: `shaders.struct.uniform.*_fragment`, mediump float members Mesa keeps fp32 | 8 | 7 / 1 | `nir_fs.rs:9683` 4 (a word alone), `nir_fs.rs:9678` 4 | `struct_basic` (here, matched by lower-fsunif); covered: `deqp_uniarray_struct_member_fs` |
//! | `arr_bool`: `uniform bool u_var[3]` | 16 | 16 / 0 | `select.rs:628` | covered: `deqp_unif2_fs_bool_uniform` |
//! | `arr_int`: `uniform mediump int u_var[3]` | 11 | 11 / 0 | `select.rs:628` | covered: `deqp_unif2_fs_int_uniform` |
//! | `struct_bool`: a struct of `bool`, `bvec4` and arrays of them (`array_in_struct` 11, nested 7) | 18 | 18 / 0 | `select.rs:628` | covered: `deqp_unif2_fs_struct_uniform` |
//! | `arr_float`: `uniform mediump float u_var[3]` (`_fragment` 9, `_both` 9) | 18 | 18 / 0 | `nir_fs.rs:10089` | covered: `deqp_uniarray_api_float` |
//! | `rnd_two`: two float uniforms in one word, `shaders.random` | 5 | 5 / 0 | `nir_fs.rs:10089` | covered: `deqp_unif2_fs_other` |
//! | `rnd_lane`: a uniform's half beside a computed lane, `shaders.random` 44, `interaction.basic_shader` 3 | 47 | 47 / 0 | `nir_fs.rs:10102` | covered: `deqp_intval3_rsg_swizzle12_fs` |
//! | `invariance`: `shaders.invariance.*.common_subexpression_{0,2,3}` | 9 | 9 / 0 | `nir_fs.rs:10102` | covered: `deqp_misc2_invariant_cse` |
//! | `struct_mat`: a struct of `mat4`, `mat2` and arrays of them (`array_in_struct` 9, nested 5) | 14 | 14 / 0 | `select.rs:628` | not drawn (the 8 reference limit): `arr_mat4` here in `deqp_unif2_fs_struct_uniform`'s layout |
//! | `multi_arr`: `float[3]`, `ivec3[3]`, `bvec2[3]` together | 9 | 9 / 0 | `select.rs:628` | not drawn: `arr_ivec4`, `arr_bvec4` here beside `deqp_uniarray_api_float` |
//! | `multi_nested`: three nested structs of float and vec4 arrays | 7 | 7 / 0 | `select.rs:628` | not drawn |
//! | `api_random`: `uniform_api.random.*` | 14 | 14 / 0 | `select.rs:628` 13, `mod.rs:1601` 1 (more than 64 uniforms) | not drawn |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsunif
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix and the ` is not lowered yet`
/// suffix.
/// Where the int and bool array shapes stand once their rows are laid
/// (plan #377 spike-uniform-rest3, select.rs `UboLayout::fs16`): the
/// broadcast colour of tests/mali_deqp_fclane.rs.
const COLOUR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant";
const FS_UNIFORM: &str = "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp";

/// The references the compiler makes byte-equal: `highp_mat4_add_float`,
/// a shader whose floats are all highp, since a matrix local is not
/// grafted (plan #387 item fclane-lower-1, nir_fs.rs `grafted_locals`);
/// `struct_basic`, an fp32 struct word beside an int member's float as one
/// colour word (plan #402 item lower-fsunif, nir_fs.rs `Leaf::WideInt`).
const MATCHED: &[&str] = &["highp_mat4_add_float", "struct_basic"];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("arr_bvec4", COLOUR_LANE),
    ("arr_ivec4", COLOUR_LANE),
    ("arr_mat4", FS_UNIFORM),
    ("struct_int_ivec4", COLOUR_LANE),
    ("mat4_add_float", FS_UNIFORM),
    ("mat2_mul_mat2", FS_UNIFORM),
];

/// [`mali_ref::assert_matches`] of a reference whose struct members
/// gl_ref.py set (`--uniform-values`): its FAU words are compared with the
/// members' values, not with zeros (`mali_ref::differences_members`; the
/// reference is not in `mali_ref::STRUCT_VALUES`).
fn assert_matches_members(name: &str) {
    let r = mali_ref::Reference::load(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: the pair is refused: {e}"));
    let d = mali_ref::differences_members(&r, &pair, true);
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_fsunif_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_fsunif_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_fsunif_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// Twelve 32-bit words, one a lane, in FAU u2..u7 (a bool is `0` or `~0`);
/// each `==` is `LSHIFT_AND.i32` over the words, a false constant lane
/// first inverted (`LSHIFT_OR.i32 .., u.not`), element 0's two false lanes
/// ORed from one FAU slot (u2, Mesa's `bi_opt_reorder_push`). No fp16
/// conversion: the one float is the `MUX.v2i16` of the mask between
/// `0x3c00` and 0.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a bvec4 array, select.rs:628)"]
fn arr_bvec4() {
    mali_ref::assert_matches("deqp_fsunif_arr_bvec4");
}

/// Twelve 32-bit words in FAU u2..u7 (`caps->int16` is off: no integer is
/// narrowed); each lane one `ICMP_OR/AND.u32.eq.m1` against its constant.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (an ivec4 array, select.rs:628)"]
fn arr_ivec4() {
    mali_ref::assert_matches("deqp_fsunif_arr_ivec4");
}

/// 48 fp16 halves in 24 words, FAU u2..u13: an element is 8 words, a column
/// two, each word one `FADD_IMM.v2f16` of its two constants, then
/// `FMAX.v2f16` of the `.abs` halves and one `FCMP_OR.v2f16.lt` with 0.05
/// (`0x2a66`, u14.w0).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a mat4 array, select.rs:628)"]
fn arr_mat4() {
    mali_ref::assert_matches("deqp_fsunif_arr_mat4");
}

/// The struct flattened in member order, one 32-bit word a lane: `m0` u2.w0,
/// `m1` u2.w1..u4.w0, `m2[3]` u4.w1..u5.w1, `m3[3]` u6.w0..u11.w1 (20
/// words).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (int and ivec4 arrays in a struct, select.rs:628)"]
fn struct_int_ivec4() {
    assert_matches_members("deqp_fsunif_struct_int_ivec4");
}

/// The mat4 as 16 halves in FAU u2..u5, two words a column. A column's
/// `.xyz`, `.yzw`, `.zwx`, `.wxy` reads a word whole (`u2.w0`), one half
/// twice (`u3.w0.h11`, `u2.w1.h00`) or the halves apart in two `FADD.v2f16`.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a mat4's halves across its fp16 pairs, nir_fs.rs:10089)"]
fn mat4_add_float() {
    mali_ref::assert_matches("deqp_fsunif_mat4_add_float");
}

/// Two mat2, a word a column, in Mesa's push order and not the
/// declarations': `u_in0`'s columns are u2.w0 and u3.w0, `u_in1`'s u2.w1
/// and u3.w1. The product's column is `FMA.v2f16` of a column word of
/// `u_in0` with one half of `u_in1`'s (`u2.w1.h00`, `r0.h11` of a `MOV.i32`
/// of the word).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a mat2's halves across its fp16 pairs, nir_fs.rs:10089)"]
fn mat2_mul_mat2() {
    mali_ref::assert_matches("deqp_fsunif_mat2_mul_mat2");
}

/// The twelve fp32 words the shader reads, packed in FAU u2..u7 (the four
/// unread components are not pushed); `FADD.f32` at 32 bits, then each
/// colour lane narrowed alone (`FADD.f32 r.h0, r, -0`) and paired by
/// `MKVEC.v2i16`.
#[test]
fn highp_mat4_add_float() {
    mali_ref::assert_matches("deqp_fsunif_highp_mat4_add_float");
}

/// Four fp32 words in FAU u2..u3: `s.a` and `s.b.x` narrowed together
/// (`V2F32_TO_V2F16 r0, u2.w0, u2.w1`), `s.b.y` alone (`FADD.f32 r1.h0,
/// u3.w0, -0`), the int `s.c` through `S32_TO_F32` and narrowed alone,
/// the two halves paired by `MKVEC.v2i16` before ATEST.
#[test]
fn struct_basic() {
    assert_matches_members("deqp_fsunif_struct_basic");
}
