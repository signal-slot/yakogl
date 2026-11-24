// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_mixprec3_*` reference
//! (Mesa 26.2.1 drm-shim, plan #402 item `ref-mixprec3`,
//! `docs/notes/mali-deqp-mixprec3.md`) against the Mali online compiler.
//! The references are the shapes of two refusals at 8ff01055
//! (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`) that
//! no `deqp_mixprec`, `mixcol`, `misc*`, `varylink*`, `posvary` or `gaps`
//! reference draws:
//!
//! * MIX, `a fragment shader that mixes highp and mediump or lowp float
//!   values (Mesa's fp32 and fp16)`: 168 Fail on the board;
//! * LOAD, `a fragment varying load or conversion of a form no reference
//!   draws (...)`: 50 Fail on the board.
//!
//! ```text
//! text  shape (reference)    board cases  dEQP-GLES2.functional.shaders.*        the shape
//! MIX   hp_vec3_fn           5            exponential.{sqrt,inversesqrt}.highp_   a highp vec3/vec4 built-in result
//!                                         vec{3,4}, geometric.normalize.highp_vec3  written into the mediump colour
//! MIX   hp_dot               3            geometric.dot.highp_vec{2,3,4}          two highp vector varyings reduced
//!                                                                                 to one highp float, into color.x
//! MIX   hp_bvec4             5            float_compare.*.highp_vec4              four highp compares, every colour
//!                                                                                 lane a bool (no 1.0 lane)
//! LOAD  hp2_sum              14           binary_operator.{add,sub,mul}{,_assign_  a highp float op of two (three)
//!                                         effect,_assign_result}.highp_float,      highp scalar varyings into color.x
//!                                         common_functions.mix, geometric.{distance,
//!                                         dot,reflect}.highp_float (distance vec2)
//! LOAD  hp_vec3_sum          3            binary_operator.{add,sub,mul}.highp_    a highp float with a highp vec3,
//!                                         float_vec3                              into color.xyz
//! LOAD  mp_vec3_scale        3            unary_operator.{plus,post_increment_    a mediump vec3 varying copy into
//!                                         result,post_decrement_result}.mediump_vec3  color.xyz, then `* 0.5 + 0.5`
//! LOAD  lp_vec3_scale        3            the same, lowp_vec3                     the same at lowp
//! LOAD  vary_array_static    4            indexing.varying_array.{float,vec2}_    a mediump varying array read at
//!                                         static_write_static{,_loop}_read        constant indices
//! ```
//!
//! Covered elsewhere (no reference here): MIX hp_scale 21, hp_bool 12,
//! hp_seq/f2i_seq 5, hp_param 4, int_uniform_struct 4, int_uniform 1
//! (`deqp_mixcol_*`); LOAD ck_scale_highp 3, lz_rnd_other 2 (`deqp_mixcol_*`),
//! return.conditional_return_* 2 (`deqp_gaps_mixprec_fs_varying_other`).
//! Not re-measured (no dEQP build on this host): MIX matrix.* 11 and the
//! random shaders (MIX 97, LOAD 16).
//!
//! Every reference is refused at the tip, so its test is `#[ignore]`d and
//! [`refusals_pinned`] (running) holds its refusal and checks that every
//! `deqp_mixprec3_*` reference on disk is listed. The lowering that lifts
//! one (plan #402 item `lower-mixprec3`) removes its `ignore`, moves it from
//! [`REFUSED`] to [`MATCHED`] in the same commit. The sources are written
//! from dEQP's generators in VK-GL-CTS 93bca01 (each `.license` names the
//! lines), drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (GPU g610, `--uniform-values`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const MIX: &str = "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet";
const LOAD: &str = "Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet";

/// The references byte-equal now, their tests running.
const MATCHED: &[&str] = &[];

/// Every refused reference with its whole refusal text.
const REFUSED: &[(&str, &str)] = &[
    ("hp_vec3_fn", MIX),
    ("hp_dot", MIX),
    ("hp_bvec4", MIX),
    ("hp2_sum", LOAD),
    ("hp_vec3_sum", LOAD),
    ("mp_vec3_scale", LOAD),
    ("lp_vec3_scale", LOAD),
    ("vary_array_static", LOAD),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_mixprec3_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_mixprec3_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_mixprec3_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16); pinned by refusals_pinned"]
fn hp_vec3_fn() {
    mali_ref::assert_matches("deqp_mixprec3_hp_vec3_fn");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16); pinned by refusals_pinned"]
fn hp_dot() {
    mali_ref::assert_matches("deqp_mixprec3_hp_dot");
}

#[test]
#[ignore = "refused: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16); pinned by refusals_pinned"]
fn hp_bvec4() {
    mali_ref::assert_matches("deqp_mixprec3_hp_bvec4");
}

/// Mesa's link moves `in0 + in1` into the vertex shader: the fragment
/// shader loads one fp32 lane.
#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn hp2_sum() {
    mali_ref::assert_matches("deqp_mixprec3_hp2_sum");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn hp_vec3_sum() {
    mali_ref::assert_matches("deqp_mixprec3_hp_vec3_sum");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn mp_vec3_scale() {
    mali_ref::assert_matches("deqp_mixprec3_mp_vec3_scale");
}

/// Mesa's fragment words are those of `mp_vec3_scale`.
#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn lp_vec3_scale() {
    mali_ref::assert_matches("deqp_mixprec3_lp_vec3_scale");
}

#[test]
#[ignore = "refused: a fragment varying load or conversion of a form no reference draws; pinned by refusals_pinned"]
fn vary_array_static() {
    mali_ref::assert_matches("deqp_mixprec3_vary_array_static");
}
