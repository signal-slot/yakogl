// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_unitype_*` reference (Mesa
//! 26.2.1, plan #377 item `ref-unitype`, `docs/notes/mali-deqp-unitype.md`)
//! against the Mali online compiler. The ones the unitype lowering takes run
//! ([`MATCHED`]); the rest are `#[ignore]`d with their refusal and the
//! family that lowers it. `refusals_pinned` runs now and holds each
//! refusal's exact text (the VS one when the VS refuses, it is compiled
//! first; otherwise the FS one), so a lowering must move its pin;
//! `differences_pinned` holds the ones that compile with other words than
//! Mesa's (the VS fp16 arithmetic of mediump values, which Mesa computes at
//! 16 bits and the vertex walk at 32).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_unitype
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// A mediump fragment shader uniform of a type outside the supported list.
const FS_MEDIUMP: &str = "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp";
/// A gl_FragColor lane written from something else: where `fsmat4_mediump`
/// is refused since plan #367's lower-fs-arith lowered the arithmetic in front
/// of it, and where a shader-library result (`vec4(RES, RES, RES, 1.0)` of an
/// fp16 compare, intval's `reslib_float_fs` too) or a matrix sum's lanes beside
/// a constant stand once the uniform is taken.
const FRAGCOLOR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant";
/// An int or bool value: the bvec `==` and the ivec `i2f` behind their loads
/// (lower-intval).
const INT_VALUE: &str = "an integer or boolean value";
/// The `Light` array's loop stands before the array (lower-loops, then
/// lower-uniarray).
const VS_LOOP: &str = "a loop in a vertex shader";

/// The references the lowering makes byte-equal.
/// `vsstruct_partial` and `vsstruct_rip` since lower-mixprec took the `b2f16`
/// of an fp16 compare as a colour lane.
const MATCHED: &[&str] =
    &["vsmat2_highp", "vsmat3_highp", "fshighp_color", "depthrange_vs", "vsstruct_partial", "vsstruct_rip"];

/// The references that compile with other words than Mesa's: Mesa computes
/// a mediump VS expression of fp16 uniforms at 16 bits (`FADD.v2f16`,
/// `FMA.v2f16`, `FCMP_OR.v2f16`), the vertex walk widens each lane first
/// (`F16_TO_F32`) and computes at 32; `vsmat2_mediump` and `vsmat3_*` also
/// keep a constant varying lane Mesa stores (`v_color.w`, varylink's).
/// `vsstruct_add`'s fragment program is not Mesa's either.
const DIFFERS: &[&str] =
    &["vsmat2_mediump", "vsmat3_mediump", "vsmat3_lowp", "vsapi_mat2", "vsapi_struct", "vsstruct_add"];

/// Every refused reference with its refusal, without the `Valhall: `
/// prefix and the ` is not lowered yet` suffix.
const REFUSED: &[(&str, &str)] = &[
    ("vsmat3_light", VS_LOOP),
    ("vsbvec3", INT_VALUE),
    ("depthrange_fs", FRAGCOLOR_LANE),
    ("fsmat2_mediump", FS_MEDIUMP),
    ("fsmat3_mediump", FRAGCOLOR_LANE),
    ("fsmat4_mediump", FRAGCOLOR_LANE),
    ("fsivec4", INT_VALUE),
    ("fsmulti", FRAGCOLOR_LANE),
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
    let mut listed: Vec<String> = REFUSED
        .iter()
        .map(|(n, _)| *n)
        .chain(MATCHED.iter().copied())
        .chain(DIFFERS.iter().copied())
        .map(str::to_string)
        .collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_unitype_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_unitype_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_unitype_{name}"));
        let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal; move it to MATCHED");
    }
}

#[test]
fn vsmat2_highp() {
    mali_ref::assert_matches("deqp_unitype_vsmat2_highp");
}

#[test]
fn vsmat3_highp() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_highp");
}

#[test]
#[ignore = "refused: a loop in a vertex shader (the Light array's), lowered by lower-loops then lower-uniarray"]
fn vsmat3_light() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_light");
}

#[test]
#[ignore = "differs: Mesa computes the mediump sum in VS fp16 (FADD.v2f16), the vertex walk at fp32"]
fn vsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat2_mediump");
}

#[test]
#[ignore = "differs: Mesa computes the mediump product in VS fp16 (FMA.v2f16), the vertex walk at fp32"]
fn vsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_mediump");
}

#[test]
#[ignore = "differs: Mesa computes the lowp difference in VS fp16 (FADD.v2f16), the vertex walk at fp32"]
fn vsmat3_lowp() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_lowp");
}

#[test]
#[ignore = "differs: Mesa computes the 0.05 compares in VS fp16 (FMAX.v2f16, FCMP_OR.v2f16), the vertex walk at fp32"]
fn vsapi_mat2() {
    mali_ref::assert_matches("deqp_unitype_vsapi_mat2");
}

#[test]
fn vsstruct_partial() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_partial");
}

#[test]
#[ignore = "differs: the fragment program (18 words, Mesa's 13, from word 0) and the vertex FAU words; the isOk colour compiles since lower-mixprec"]
fn vsstruct_add() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_add");
}

#[test]
fn vsstruct_rip() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_rip");
}

#[test]
#[ignore = "differs: Mesa narrows the struct members and compares in VS fp16, the vertex walk at fp32"]
fn vsapi_struct() {
    mali_ref::assert_matches("deqp_unitype_vsapi_struct");
}

#[test]
#[ignore = "refused: an integer or boolean value (the bvec ==), lowered by lower-intval"]
fn vsbvec3() {
    mali_ref::assert_matches("deqp_unitype_vsbvec3");
}

#[test]
fn fshighp_color() {
    mali_ref::assert_matches("deqp_unitype_fshighp_color");
}

#[test]
fn depthrange_vs() {
    mali_ref::assert_matches("deqp_unitype_depthrange_vs");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (diff's fp32 FMA into the fp16 colour), lowered by lower-mixprec"]
fn depthrange_fs() {
    mali_ref::assert_matches("deqp_unitype_depthrange_fs");
}

#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a column's lanes paired across columns), lowered by lower-unitype"]
fn fsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat2_mediump");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (the column sum's lane beside 1.0), lowered by lower-unitype"]
fn fsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat3_mediump");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (past its mat4 uniform since plan #367's lower-fs-arith), lowered by lower-unitype"]
fn fsmat4_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat4_mediump");
}

#[test]
#[ignore = "refused: an integer or boolean value (vec4(ivec4)), lowered by lower-intval"]
fn fsivec4() {
    mali_ref::assert_matches("deqp_unitype_fsivec4");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant, lowered by lower-intval"]
fn fsmulti() {
    mali_ref::assert_matches("deqp_unitype_fsmulti");
}
