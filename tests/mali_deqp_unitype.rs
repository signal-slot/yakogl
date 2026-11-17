// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_unitype_*` reference (Mesa
//! 26.2.1, plan #377 item `ref-unitype`, `docs/notes/mali-deqp-unitype.md`)
//! against the Mali online compiler. All of them are refused at the tip, so
//! each byte-compare is `#[ignore]`d with its refusal and the family that
//! lowers it; `lower-unitype` removes the `ignore` as each shape is
//! lowered. `refusals_pinned` runs now and holds each refusal's exact text
//! (the VS one when the VS refuses, it is compiled first; otherwise the FS
//! one), so a lowering must move its pin.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_unitype
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// A uniform of a type the table has no row for (the VS reaches it first).
const OTHER_TYPE: &str = "a uniform other than a float, vec2, vec3, vec4 or mat4";
/// A vertex shader uniform that is not highp or not a plain float type.
const VS_NOT_HIGHP: &str = "a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member";
/// A mediump fragment shader uniform of a type outside the supported list.
const FS_MEDIUMP: &str = "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp";
/// A gl_FragColor lane written from something else: where `fsmat4_mediump`
/// is refused since plan #367's lower-fs-arith lowered the arithmetic in front
/// of it.
const FRAGCOLOR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant";

/// Every reference with its refusal at the tip, without the `Valhall: `
/// prefix and the ` is not lowered yet` suffix.
const REFUSED: &[(&str, &str)] = &[
    ("vsmat2_highp", OTHER_TYPE),
    ("vsmat3_highp", OTHER_TYPE),
    ("vsmat3_light", OTHER_TYPE),
    ("vsmat2_mediump", OTHER_TYPE),
    ("vsmat3_mediump", OTHER_TYPE),
    ("vsmat3_lowp", OTHER_TYPE),
    ("vsapi_mat2", OTHER_TYPE),
    ("vsstruct_partial", VS_NOT_HIGHP),
    ("vsstruct_add", VS_NOT_HIGHP),
    ("vsstruct_rip", VS_NOT_HIGHP),
    ("vsapi_struct", VS_NOT_HIGHP),
    ("vsbvec3", OTHER_TYPE),
    ("fshighp_color", FS_MEDIUMP),
    ("depthrange_vs", VS_NOT_HIGHP),
    ("depthrange_fs", FS_MEDIUMP),
    ("fsmat2_mediump", FS_MEDIUMP),
    ("fsmat3_mediump", FS_MEDIUMP),
    ("fsmat4_mediump", FRAGCOLOR_LANE),
    ("fsivec4", FS_MEDIUMP),
    ("fsmulti", FS_MEDIUMP),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_unitype_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = REFUSED.iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_unitype_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_unitype_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsmat2_highp() {
    mali_ref::assert_matches("deqp_unitype_vsmat2_highp");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsmat3_highp() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_highp");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4 (behind the Light struct array), lowered by lower-uniarray"]
fn vsmat3_light() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_light");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat2_mediump");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_mediump");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsmat3_lowp() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_lowp");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsapi_mat2() {
    mali_ref::assert_matches("deqp_unitype_vsapi_mat2");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp, lowered by lower-unitype"]
fn vsstruct_partial() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_partial");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp, lowered by lower-unitype"]
fn vsstruct_add() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_add");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp, lowered by lower-unitype"]
fn vsstruct_rip() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_rip");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp, lowered by lower-unitype"]
fn vsapi_struct() {
    mali_ref::assert_matches("deqp_unitype_vsapi_struct");
}

#[test]
#[ignore = "refused: a uniform other than a float, vec2, vec3, vec4 or mat4, lowered by lower-unitype"]
fn vsbvec3() {
    mali_ref::assert_matches("deqp_unitype_vsbvec3");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int, lowered by lower-unitype"]
fn fshighp_color() {
    mali_ref::assert_matches("deqp_unitype_fshighp_color");
}

#[test]
#[ignore = "refused: a vertex shader uniform that is not highp, lowered by lower-unitype"]
fn depthrange_vs() {
    mali_ref::assert_matches("deqp_unitype_depthrange_vs");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int, lowered by lower-unitype"]
fn depthrange_fs() {
    mali_ref::assert_matches("deqp_unitype_depthrange_fs");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int, lowered by lower-unitype"]
fn fsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat2_mediump");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int, lowered by lower-unitype"]
fn fsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat3_mediump");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (past its mat4 uniform since plan #367's lower-fs-arith), lowered by lower-unitype"]
fn fsmat4_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat4_mediump");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int, lowered by lower-unitype"]
fn fsivec4() {
    mali_ref::assert_matches("deqp_unitype_fsivec4");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int, lowered by lower-unitype"]
fn fsmulti() {
    mali_ref::assert_matches("deqp_unitype_fsmulti");
}
