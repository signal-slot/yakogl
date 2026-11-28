// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsuniarr3_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsuniarr3`,
//! `docs/notes/mali-deqp-vsuniarr3.md`) against the Mali online compiler: the
//! partly read PLAIN matrix arrays that `deqp_vsuniarr_*`, `deqp_vsuniarr2_*`
//! and `deqp_unif2_*` leave out (they read every element of a mat4 array, or a
//! mat3 element, or are struct members). The refusal is
//! `MaliError::UniformArray`, in the vertex shader. The ones a lowering takes
//! run ([`MATCHED`]); the rest are `#[ignore]`d. `refusals_pinned` runs now and
//! holds each refusal's exact text, so a lowering must move its pin.
//!
//! The shaders are hand-made in dEQP's `uniform_api` style (not taken from a
//! dEQP run; the dEQP sources were not built for this item). Each one is
//! refused on the host at the tip (HOST-ONLY, `compile_pair`).
//!
//! | Reference | Declaration | Read |
//! |---|---|---|
//! | `mat4_part` | `uniform mediump mat4 u_var[4]` | `u_var[1]` and `[3]` (mediump) |
//! | `mat4_highp` | `uniform highp mat4 u_var[3]` | `u_var[1]` alone (highp) |
//! | `mat4_col` | `uniform mediump mat4 u_var[3]` | one column, `u_var[2][1]` |
//! | `mat2_part` | `uniform mediump mat2 u_var[4]` | `u_var[3]` alone |
//! | `mat4_first` | `uniform mediump mat4 u_var[3]` and a `float` | `u_var[0]` and the float |
//! | `mat4_mul` | `uniform mediump mat4 u_var[2]` | `gl_Position = u_var[1] * a_position` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsuniarr3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix.
const UNIFORM_ARRAY: &str = "a uniform array is not lowered yet";

/// The references the compiler makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("mat4_part", UNIFORM_ARRAY),
    ("mat4_highp", UNIFORM_ARRAY),
    ("mat4_col", UNIFORM_ARRAY),
    ("mat2_part", UNIFORM_ARRAY),
    ("mat4_first", UNIFORM_ARRAY),
    ("mat4_mul", UNIFORM_ARRAY),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsuniarr3_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsuniarr3_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsuniarr3_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// `uniform mediump mat4 u_var[4]`: `u_var[1]` and `[3]` (mediump).
#[test]
#[ignore = "refused: a uniform array (plain matrix array)"]
fn mat4_part() {
    mali_ref::assert_matches("deqp_vsuniarr3_mat4_part");
}

/// `uniform highp mat4 u_var[3]`: `u_var[1]` alone (highp).
#[test]
#[ignore = "refused: a uniform array (plain matrix array)"]
fn mat4_highp() {
    mali_ref::assert_matches("deqp_vsuniarr3_mat4_highp");
}

/// `uniform mediump mat4 u_var[3]`: one column, `u_var[2][1]`.
#[test]
#[ignore = "refused: a uniform array (plain matrix array)"]
fn mat4_col() {
    mali_ref::assert_matches("deqp_vsuniarr3_mat4_col");
}

/// `uniform mediump mat2 u_var[4]`: `u_var[3]` alone.
#[test]
#[ignore = "refused: a uniform array (plain matrix array)"]
fn mat2_part() {
    mali_ref::assert_matches("deqp_vsuniarr3_mat2_part");
}

/// `uniform mediump mat4 u_var[3]` and a `float`: `u_var[0]` and the float.
#[test]
#[ignore = "refused: a uniform array (plain matrix array)"]
fn mat4_first() {
    mali_ref::assert_matches("deqp_vsuniarr3_mat4_first");
}

/// `uniform mediump mat4 u_var[2]`: `gl_Position = u_var[1] * a_position`.
#[test]
#[ignore = "refused: a uniform array (plain matrix array)"]
fn mat4_mul() {
    mali_ref::assert_matches("deqp_vsuniarr3_mat4_mul");
}
