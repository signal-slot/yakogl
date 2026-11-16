// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_intval_*` (plan #377, family intval,
//! `docs/notes/mali-deqp-intval.md` section 5), one test per reference.
//! `tests/mali_ref` does the compare.
//!
//! All 25 references are refused at the tip with `an integer or boolean value
//! is not lowered yet`, so each test is `#[ignore]`d until the lowering step
//! named in its reason (lower-intval-1 or lower-intval-2) lifts it.
//! `refusals_pinned` runs now and asserts the exact refusal of each ignored
//! reference: a lowering that lifts one must remove its `#[ignore]` and its
//! pin in the same commit. `rsg_constbool_vs` and `rsg_cond_fs` get their
//! result from Mesa's link (a constant varying moved into the FS, dead
//! varyings removed), which `mali_ref` cannot reach: they stay ignored after
//! the lowering and are covered by the unit tests of
//! `src/shaderc/online/mali/tests.rs`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intval
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The whole `MaliError` message every reference below is refused with.
const REFUSED: &str = "Valhall: an integer or boolean value is not lowered yet";

/// Every ignored reference below, without the `deqp_intval_` prefix.
const PINNED: &[&str] = &[
    "opfs_add_int_mediump",
    "opfs_cmp_ivec3",
    "opfs_div_int_mediump",
    "opfs_equal_bvec2",
    "opfs_logical_xor",
    "opfs_mul_ivec4_highp",
    "opfs_not",
    "opfs_sub_assign_ivec2_lowp",
    "opvs_add_int_highp",
    "opvs_div_ivec2_highp",
    "opvs_less_ivec4",
    "opvs_logical_and",
    "opvs_mul_ivec3_mediump",
    "opvs_not_equal_bvec3",
    "opvs_post_increment_int",
    "reslib_bvec_fs",
    "reslib_float_fs",
    "reslib_int_fs",
    "reslib_int_vs",
    "reslib_mixed_fs",
    "rsg_cond_fs",
    "rsg_constbool_vs",
    "rsg_conv_vs",
    "uniapi_bool_initial_vs",
    "uniapi_bool_vs",
];

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_add_int_mediump() {
    mali_ref::assert_matches("deqp_intval_opfs_add_int_mediump");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_cmp_ivec3() {
    mali_ref::assert_matches("deqp_intval_opfs_cmp_ivec3");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_div_int_mediump() {
    mali_ref::assert_matches("deqp_intval_opfs_div_int_mediump");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_equal_bvec2() {
    mali_ref::assert_matches("deqp_intval_opfs_equal_bvec2");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_logical_xor() {
    mali_ref::assert_matches("deqp_intval_opfs_logical_xor");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_mul_ivec4_highp() {
    mali_ref::assert_matches("deqp_intval_opfs_mul_ivec4_highp");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_not() {
    mali_ref::assert_matches("deqp_intval_opfs_not");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn opfs_sub_assign_ivec2_lowp() {
    mali_ref::assert_matches("deqp_intval_opfs_sub_assign_ivec2_lowp");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_add_int_highp() {
    mali_ref::assert_matches("deqp_intval_opvs_add_int_highp");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_div_ivec2_highp() {
    mali_ref::assert_matches("deqp_intval_opvs_div_ivec2_highp");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_less_ivec4() {
    mali_ref::assert_matches("deqp_intval_opvs_less_ivec4");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_logical_and() {
    mali_ref::assert_matches("deqp_intval_opvs_logical_and");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_mul_ivec3_mediump() {
    mali_ref::assert_matches("deqp_intval_opvs_mul_ivec3_mediump");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_not_equal_bvec3() {
    mali_ref::assert_matches("deqp_intval_opvs_not_equal_bvec3");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn opvs_post_increment_int() {
    mali_ref::assert_matches("deqp_intval_opvs_post_increment_int");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn reslib_bvec_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_bvec_fs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn reslib_float_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_float_fs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn reslib_int_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_int_fs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn reslib_int_vs() {
    mali_ref::assert_matches("deqp_intval_reslib_int_vs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1"]
fn reslib_mixed_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_mixed_fs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-1; not comparable by mali_ref: Mesa's link removes the dead varyings, which mali_ref cannot reach"]
fn rsg_cond_fs() {
    mali_ref::assert_matches("deqp_intval_rsg_cond_fs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2; not comparable by mali_ref: Mesa's link moves the constant varying into the FS, which mali_ref cannot reach"]
fn rsg_constbool_vs() {
    mali_ref::assert_matches("deqp_intval_rsg_constbool_vs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn rsg_conv_vs() {
    mali_ref::assert_matches("deqp_intval_rsg_conv_vs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn uniapi_bool_initial_vs() {
    mali_ref::assert_matches("deqp_intval_uniapi_bool_initial_vs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet, lowered by lower-intval-2"]
fn uniapi_bool_vs() {
    mali_ref::assert_matches("deqp_intval_uniapi_bool_vs");
}

/// Each ignored reference is refused with exactly `REFUSED` (the whole
/// `MaliError` message, not a substring), and the list covers every
/// `deqp_intval_*` reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_intval_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    assert_eq!(on_disk, PINNED, "a deqp_intval_* reference has no test here");
    for name in PINNED {
        let r = mali_ref::Reference::load(&format!("deqp_intval_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => panic!("{name}: now compiles; remove its #[ignore] and its pin"),
            Err(e) => assert_eq!(e, REFUSED, "{name}: the refusal changed"),
        }
    }
}
