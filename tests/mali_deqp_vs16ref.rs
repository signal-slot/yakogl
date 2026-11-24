// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_vs16ref_*` (plan #387, item
//! posvary-lower): the references R1-R9 of section 7 of
//! `docs/notes/mali-intval-vs16-lowering.md` that nothing held (R10 is held
//! by existing references; see section 7a of that note). One test per
//! reference, the name without `deqp_vs16ref_`; the dEQP case each stands
//! for (under `dEQP-GLES2.functional.shaders.`):
//!
//! - R1: cmp_{lt,eq,ne}_mediump_vec3: `float_compare.{lessThan,equal,notEqual}.mediump_vec3_vertex`
//! - R2: or_bool: `logical_or.bool_vertex`
//! - R3: xor_bool: `logical_xor.bool_vertex`
//! - R4: not_bool: `not.bool_vertex`
//! - R5: bcmp_{eq,ne}_bvec{3,4}, bany_bvec{3,4}, ball_bvec{3,4}: `bool_compare.{equal,not_equal,any,all}.bvec{3,4}_vertex`
//! - R6: eq_mediump_vec4, ne_mediump_float: `binary_operator.{equal.mediump_vec4,not_equal.mediump_float}_vertex`
//! - R7: lt_lowp_float, lt_mediump_float: `binary_operator.less.{lowp,mediump}_float_vertex`
//! - R8: uapi_int, uapi_ivec3, uapi_struct_int_ivec4: `uniform_api.value.initial.render.{basic.int,basic.ivec3,basic_struct.int_ivec4}_vertex`
//! - R9: uapi_bvec4_mix, uapi_bvec3_mix: `uniform_api` `bvec4`/`bvec3` with a true/false mix
//!
//! Drawn once on Mesa's drm-shim (`tools/mali/deqp_ref.sh --instrument
//! shim`, uniform values from `gl_ref.py`). [`MATCHED`] are byte-equal;
//! [`REFUSALS`] pins the ones the compiler refuses with the text it gives,
//! [`DIFFERS`] the ones it compiles to other words than Mesa. The lowering
//! removes a reference from its list, lists it in [`MATCHED`] and drops the
//! `ignore`. `refusals_pinned` runs now and names every reference that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vs16ref
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_ball_bvec3() {
    mali_ref::assert_matches("deqp_vs16ref_ball_bvec3");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_ball_bvec4() {
    mali_ref::assert_matches("deqp_vs16ref_ball_bvec4");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_bany_bvec3() {
    mali_ref::assert_matches("deqp_vs16ref_bany_bvec3");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_bany_bvec4() {
    mali_ref::assert_matches("deqp_vs16ref_bany_bvec4");
}

#[test]
fn vs16ref_bcmp_eq_bvec3() {
    mali_ref::assert_matches("deqp_vs16ref_bcmp_eq_bvec3");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_bcmp_eq_bvec4() {
    mali_ref::assert_matches("deqp_vs16ref_bcmp_eq_bvec4");
}

#[test]
fn vs16ref_bcmp_ne_bvec3() {
    mali_ref::assert_matches("deqp_vs16ref_bcmp_ne_bvec3");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_bcmp_ne_bvec4() {
    mali_ref::assert_matches("deqp_vs16ref_bcmp_ne_bvec4");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_cmp_eq_mediump_vec3() {
    mali_ref::assert_matches("deqp_vs16ref_cmp_eq_mediump_vec3");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_cmp_lt_mediump_vec3() {
    mali_ref::assert_matches("deqp_vs16ref_cmp_lt_mediump_vec3");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_cmp_ne_mediump_vec3() {
    mali_ref::assert_matches("deqp_vs16ref_cmp_ne_mediump_vec3");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_eq_mediump_vec4() {
    mali_ref::assert_matches("deqp_vs16ref_eq_mediump_vec4");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_lt_lowp_float() {
    mali_ref::assert_matches("deqp_vs16ref_lt_lowp_float");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_lt_mediump_float() {
    mali_ref::assert_matches("deqp_vs16ref_lt_mediump_float");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_ne_mediump_float() {
    mali_ref::assert_matches("deqp_vs16ref_ne_mediump_float");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_not_bool() {
    mali_ref::assert_matches("deqp_vs16ref_not_bool");
}

#[test]
fn vs16ref_or_bool() {
    mali_ref::assert_matches("deqp_vs16ref_or_bool");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_uapi_bvec3_mix() {
    mali_ref::assert_matches("deqp_vs16ref_uapi_bvec3_mix");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * and a dot or matrix product"]
fn vs16ref_uapi_bvec4_mix() {
    mali_ref::assert_matches("deqp_vs16ref_uapi_bvec4_mix");
}

#[test]
fn vs16ref_uapi_int() {
    mali_ref::assert_matches("deqp_vs16ref_uapi_int");
}

#[test]
fn vs16ref_uapi_ivec3() {
    mali_ref::assert_matches("deqp_vs16ref_uapi_ivec3");
}

#[test]
#[ignore = "refused: an integer or boolean value (the five words' umax chain, which Mesa rebalances into a tree)"]
fn vs16ref_uapi_struct_int_ivec4() {
    mali_ref::assert_matches("deqp_vs16ref_uapi_struct_int_ivec4");
}

#[test]
#[ignore = "compiles, but to other words than Mesa's"]
fn vs16ref_xor_bool() {
    mali_ref::assert_matches("deqp_vs16ref_xor_bool");
}

const ARITH: &str = "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";
const INT: &str = "an integer or boolean value is not lowered yet";

/// Every reference the compiler refuses and the text it gives now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_vs16ref_ball_bvec3", ARITH),
    ("deqp_vs16ref_ball_bvec4", ARITH),
    ("deqp_vs16ref_bany_bvec3", ARITH),
    ("deqp_vs16ref_bany_bvec4", ARITH),
    ("deqp_vs16ref_bcmp_eq_bvec4", ARITH),
    ("deqp_vs16ref_bcmp_ne_bvec4", ARITH),
    ("deqp_vs16ref_eq_mediump_vec4", ARITH),
    ("deqp_vs16ref_uapi_bvec3_mix", ARITH),
    ("deqp_vs16ref_uapi_bvec4_mix", ARITH),
    ("deqp_vs16ref_uapi_struct_int_ivec4", INT),
];

/// References that compile at the tip, to other words than Mesa's (the
/// varying region differs).
const DIFFERS: &[&str] = &[
    "deqp_vs16ref_cmp_eq_mediump_vec3",
    "deqp_vs16ref_cmp_lt_mediump_vec3",
    "deqp_vs16ref_cmp_ne_mediump_vec3",
    "deqp_vs16ref_lt_lowp_float",
    "deqp_vs16ref_lt_mediump_float",
    "deqp_vs16ref_ne_mediump_float",
    "deqp_vs16ref_not_bool",
    "deqp_vs16ref_xor_bool",
];

/// The references that compile byte-equal (their tests run): plan #387
/// intval-vs16 step vs16-1, the mediump compares of an attribute against a
/// constant at 16 bits, the `fmax` rule of `||` and the width-three
/// `bvec` `==` / `!=` chain.
const MATCHED: &[&str] = &[
    "deqp_vs16ref_bcmp_eq_bvec3",
    "deqp_vs16ref_bcmp_ne_bvec3",
    "deqp_vs16ref_or_bool",
    /* vs16-2: an int / ivec uniform read as its words, `compare_ivec3`'s
     * product the `CSEL.u32.gt` chain */
    "deqp_vs16ref_uapi_int",
    "deqp_vs16ref_uapi_ivec3",
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_vs16ref_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> =
        REFUSALS.iter().map(|(n, _)| *n).chain(DIFFERS.iter().copied()).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_vs16ref_* reference is in no list here");
    let mut moved: Vec<String> =
        REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    for name in DIFFERS {
        let r = mali_ref::Reference::load(name);
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Err(e) => moved.push(format!("{name}: now refused ({e}); move it to REFUSALS")),
            Ok(p) if mali_ref::differences(&r, &p).is_empty() => {
                moved.push(format!("{name}: now byte-equal; move it to MATCHED and drop its #[ignore]"))
            }
            Ok(_) => {}
        }
    }
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
