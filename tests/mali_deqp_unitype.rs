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
/// colour stands once its uniforms are taken, and `fsivec4`'s
/// `vec4(fTest).xy` (`i2f32` of the ivec uniform's words into a colour pair
/// beside a texel, no shape of plan #377's lower-intval-1) since its vertex
/// shader compiles (lower-intval-2).
const FRAGCOLOR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant";
/// An operand form the fragment walk has no reference for: `fsmat3_mediump`'s
/// column sum, a lane alone first, since plan #377's constop lowering takes
/// its colour lane beside 1.0. Mesa's program is `vec16.rs`'s chunks of the
/// columns plus `v_coords`, a varying that route does not read.
const FS_OPERAND: &str = "a fragment shader operand of a form or in a place no reference draws";
/// The `Light` array's loop stands before the array (lower-loops, then
/// lower-uniarray).
const VS_LOOP: &str = "a loop in a vertex shader";

/// The references the lowering makes byte-equal (`vsstruct_partial` and
/// `vsstruct_rip` since plan #377's lower-intval-1 took the `isOk` result in
/// the fragment colour, `bool_colour`; `fsmat4_mediump` since its constop
/// lowering took a `vec4(.., 1.0)` colour lane beside a computed one;
/// `fsmat2_mediump`, `vsbvec3`, `vsmat2_mediump` and `vsmat3_mediump` since
/// lower-unitype: the mat2 columns' `vec16.rs` chunks, the bvec3 `all(==)`
/// as `LSHIFT_OR.i32` of the words into one `CSEL.u32.eq`, and the vertex
/// walk's fp16 arithmetic of fp16 uniform words and narrowed mediump
/// attributes, nir.rs `vs_alu16u`).
const MATCHED: &[&str] = &[
    "vsmat2_highp",
    "vsmat3_highp",
    "fshighp_color",
    "depthrange_vs",
    "fsmat4_mediump",
    "vsstruct_partial",
    "vsstruct_rip",
    "fsmat2_mediump",
    "vsbvec3",
    "vsmat2_mediump",
    "vsmat3_mediump",
];

/// The references that compile with other words than Mesa's.
/// `vsmat3_lowp`'s fp16 sums are Mesa's words since lower-unitype; Mesa
/// also stores its constant `v_color.w` in the flat `.v4` (the sums'
/// `f2f32` hides it from the link), where the vertex walk propagates it
/// (the kept lane of a vector from no attribute: varylink's link rule).
/// `vsapi_mat2` and `vsapi_struct`: Mesa folds the products of the 0.05
/// compares' `b2f` into one fp16 compare of the lanes' `fmax` (`FMAX.v2f16`
/// of the `.abs` lanes, `FCMP_OR.v2f16.lt.m1`, `S16_TO_S32` into the
/// `CSEL.u32.eq`), the vertex walk compares each lane at fp32 (the mediump
/// compare in the vertex stage, lower-intval's step 3 and the mixprec
/// family's vertex side).
/// `vsstruct_add`'s `dummy` varying is computed from uniforms alone and
/// cancels in the FS (`out0 + dummy - dummy`): Mesa's link drops it and the
/// VS has no varying program, which `mali_ref` cannot reach (the unitype
/// note's M5 risk 2); ours keeps the varying and its program.
/// `depthrange_fs` since plan #377's constop lowering takes its colour:
/// Mesa's `diff * 0.5 + 0.5` is an `FMA.f32` narrowed after, ours an
/// `FMA.v2f16` of the narrowed `diff`.
const DIFFERS: &[&str] = &["vsmat3_lowp", "vsapi_mat2", "vsapi_struct", "vsstruct_add", "depthrange_fs"];

/// Every refused reference with its refusal, without the `Valhall: `
/// prefix and the ` is not lowered yet` suffix.
const REFUSED: &[(&str, &str)] = &[
    ("vsmat3_light", VS_LOOP),
    ("fsmat3_mediump", FS_OPERAND),
    ("fsivec4", FRAGCOLOR_LANE),
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
fn vsmat2_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat2_mediump");
}

#[test]
fn vsmat3_mediump() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_mediump");
}

#[test]
#[ignore = "differs: Mesa stores the constant v_color.w in the flat .v4 (the fp16 sums' f2f32 hides it), the vertex walk propagates it (varylink's kept lane from no attribute)"]
fn vsmat3_lowp() {
    mali_ref::assert_matches("deqp_unitype_vsmat3_lowp");
}

#[test]
#[ignore = "differs: Mesa folds the 0.05 compares into one VS fp16 compare of the lanes' fmax (FMAX.v2f16, FCMP_OR.v2f16), the vertex walk compares each lane at fp32 (lower-intval step 3)"]
fn vsapi_mat2() {
    mali_ref::assert_matches("deqp_unitype_vsapi_mat2");
}

#[test]
fn vsstruct_partial() {
    mali_ref::assert_matches("deqp_unitype_vsstruct_partial");
}

#[test]
#[ignore = "differs: Mesa's link drops the uniform-only `dummy` varying (out0 + dummy - dummy cancels) and the VS has no varying program; ours keeps it (mali_ref cannot reach the link)"]
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

/// Drawn with its bool uniform unset (gl_ref.py's `--uniform-values` sets
/// no bool): every uniform word 0.
#[test]
fn vsbvec3() {
    mali_ref::assert_matches_unset("deqp_unitype_vsbvec3");
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
#[ignore = "differs: Mesa computes diff's FMA at fp32 and narrows it into the fp16 colour, ours narrows diff first (lower-mixprec)"]
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
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (vec4(ivec4).xy beside a texel), lowered by lower-intval"]
fn fsivec4() {
    mali_ref::assert_matches("deqp_unitype_fsivec4");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant, lowered by lower-intval"]
fn fsmulti() {
    mali_ref::assert_matches("deqp_unitype_fsmulti");
}
