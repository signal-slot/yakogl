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

/// A gl_FragColor lane written from something else: where `fsmulti`'s
/// colour stands once its uniforms are taken (its float, ivec3, mat3 and
/// bvec2 compares in one product, the varying a constant the link
/// propagates).
const FRAGCOLOR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant";
/// An operand form the fragment walk has no reference for: `fsmat3_mediump`'s
/// column sum, a lane alone first, since plan #377's constop lowering takes
/// its colour lane beside 1.0. Mesa's program is `vec16.rs`'s chunks of the
/// columns plus `v_coords`, a varying that route does not read.
const FS_OPERAND: &str = "a fragment shader operand of a form or in a place no reference draws";

/// The references the lowering makes byte-equal (`vsstruct_partial` and
/// `vsstruct_rip` since plan #377's lower-intval-1 took the `isOk` result in
/// the fragment colour, `bool_colour`; `fsmat4_mediump` since its constop
/// lowering took a `vec4(.., 1.0)` colour lane beside a computed one;
/// `fsmat2_mediump`, `vsmat2_mediump` and `vsmat3_mediump` since
/// lower-unitype: the mat2 columns' `vec16.rs` chunks and the vertex
/// walk's fp16 arithmetic of fp16 uniform words and narrowed mediump
/// attributes, nir.rs `vs_alu16u`). Since plan #377's spike-unitype-rest:
/// `vsmat3_lowp` (the 1.0 kept in the flat `.v4` beside fp16 arithmetic
/// of uniforms, nir.rs `uniform_sum16`), `vsapi_mat2` (the fp16 compare
/// of the lanes' `fmax`, `vs_cmp_and`), `vsstruct_add` (`(x + d) - d` is
/// `x`, the unread varying dropped; the two narrowed members one
/// `V2F32_TO_V2F16`), `depthrange_fs` (`diff * 0.5 + 0.5` at fp32,
/// `Leaf::Wide32`), `vsbvec3` (the OR of z and y first, Mesa's FAU order
/// through `bi_opt_reorder_push`), `fsivec4` (`Leaf::IntWide` beside the
/// `.ba` texels; the vertex `mat4 * vec4` in fp16 pairs of every column
/// word, `vs_pair16`) and `vsmat3_light` (a struct array element's member
/// precision, mod.rs `uniform_precision`).
const MATCHED: &[&str] = &[
    "vsmat2_highp",
    "vsmat3_highp",
    "fshighp_color",
    "depthrange_vs",
    "fsmat4_mediump",
    "vsstruct_partial",
    "vsstruct_rip",
    "fsmat2_mediump",
    "vsmat2_mediump",
    "vsmat3_mediump",
    "vsmat3_lowp",
    "vsapi_mat2",
    "vsstruct_add",
    "depthrange_fs",
    "vsbvec3",
    "fsivec4",
    "vsmat3_light",
];

/// The references that compile with other words than Mesa's.
/// `vsapi_struct`: Mesa narrows the fp32 struct members (`m0`, `m1.xyzw`)
/// in pairs `(m0, x)`, `(y, z)` and `w` alone, rebuilds `m1`'s words with
/// `MKVEC`, and folds the five compares into `fmax(|m0|, fmax(fmax(|w|,
/// |z|), fmax(|x|, |y|)))` at fp16; the vertex walk compares each member at
/// fp32 (the fold of `vs_cmp_and` reads 16-bit lanes only).
const DIFFERS: &[&str] = &["vsapi_struct"];

/// Every refused reference with its refusal, without the `Valhall: `
/// prefix and the ` is not lowered yet` suffix.
const REFUSED: &[(&str, &str)] = &[("fsmat3_mediump", FS_OPERAND), ("fsmulti", FRAGCOLOR_LANE)];

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
fn vsmat3_light() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_light");
}

#[test]
fn vsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat2_mediump");
}

#[test]
fn vsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_mediump");
}

#[test]
fn vsmat3_lowp() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_lowp");
}

#[test]
fn vsapi_mat2() {
    mali_ref::assert_matches("deqp_unitype_vsapi_mat2");
}

#[test]
fn vsstruct_partial() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_partial");
}

#[test]
fn vsstruct_add() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_add");
}

#[test]
fn vsstruct_rip() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_rip");
}

#[test]
#[ignore = "differs: Mesa narrows the struct members in pairs and folds the five compares into one fp16 fmax tree, the vertex walk compares each at fp32"]
fn vsapi_struct() {
    mali_ref::assert_matches("deqp_unitype_vsapi_struct");
}

/// Drawn with `--uniform-values`: the bvec3 is true, false, true.
#[test]
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
fn depthrange_fs() {
    mali_ref::assert_matches("deqp_unitype_depthrange_fs");
}

#[test]
fn fsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat2_mediump");
}

#[test]
#[ignore = "refused: a fragment shader operand of a form or in a place no reference draws (the column sum's lane alone first); Mesa's program is vec16.rs's column chunks with a varying, which that route does not read"]
fn fsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat3_mediump");
}

#[test]
fn fsmat4_mediump() {
    mali_ref::assert_matches("deqp_unitype_fsmat4_mediump");
}

#[test]
fn fsivec4() {
    mali_ref::assert_matches("deqp_unitype_fsivec4");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant, lowered by lower-intval"]
fn fsmulti() {
    mali_ref::assert_matches("deqp_unitype_fsmulti");
}
