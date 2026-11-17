// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_intval_*` (plan #377, family intval,
//! `docs/notes/mali-deqp-intval.md` sections 5, 7 and 8), one test per
//! reference. `tests/mali_ref` does the compare; the references were drawn
//! with `--uniform-values`, so each uniform word of the FAU is the value
//! gl_ref.py set (an int as the integer, a true bool as `ffffffff`).
//!
//! The twelve fragment-walk references of step 1 (lower-intval-1) and nine
//! vertex-walk references of step 2 (lower-intval-2) compile and are
//! byte-equal ([`LOWERED`]): the shader library's `isOk` chains, the
//! operator suite's int arithmetic, compares and bool logic in either
//! stage, the rsg programs' folded literals and the `uniform_api` bool
//! compares. `opvs_logical_and` compiles with other words than Mesa's
//! ([`DIFFERS`]: Mesa compares the mediump attributes at fp16, the vertex
//! walk at fp32). The rest stay `#[ignore]`d with their refusal
//! ([`REFUSED`], asserted exactly by `refusals_pinned`): the lowering that
//! lifts one must move it in the same commit. `rsg_cond_fs` gets its result
//! from Mesa's link (the dead varyings removed), which `mali_ref` cannot
//! reach: it stays ignored and is covered by the unit tests of
//! `src/shaderc/online/mali/tests.rs`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_intval
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the two steps lowered, without the `deqp_intval_` prefix:
/// each has a byte-compare test below.
const LOWERED: &[&str] = &[
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
    "opvs_mul_ivec3_mediump",
    "opvs_post_increment_int",
    "reslib_bvec_fs",
    "reslib_float_fs",
    "reslib_int_fs",
    "reslib_mixed_fs",
    "rsg_constbool_vs",
    "rsg_conv_vs",
    "uniapi_bool_initial_vs",
    "uniapi_bool_vs",
];

/// The references that compile with other words than Mesa's: a mediump
/// attribute's compare (`a_in0.z > 0.0` of a `mediump vec4` attribute) is
/// fp16 in Mesa (the lane narrowed `FADD.f32 r0.h0`, `FMIN.v2f16` of the
/// `&&`'s two, `FCMP_OR.v2f16.lt.m1`, `S16_TO_S32` into the colour's
/// `CSEL`), fp32 in the vertex walk (the mixprec family's vertex side).
const DIFFERS: &[&str] = &["opvs_logical_and"];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    /* `bvec3 != bvec3` of two `greaterThan` of mediump attributes: `any`
     * of the lanes' `ine`, which the walk's reduction does not take (the
     * mixprec family's vertex side, as `opvs_logical_and`) */
    (
        "opvs_not_equal_bvec3",
        "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet",
    ),
    /* the fragment shader of every `_vertex` shader-library case with an
     * int output: `isOk(float a, int b)` compares the mediump `atemp`
     * widened with `float(b)` and `float(b + 1)` at fp32 and `&&`s the two
     * 32-bit masks (`FCMP_AND.f32.ge.m1`, a form the ISA table lacks): the
     * mixprec family's fragment side */
    ("reslib_int_vs", "Valhall: an integer or boolean value is not lowered yet"),
    /* its vertex shader compiles since step 2; its fragment shader reads
     * the varyings Mesa's link removes as dead, which `mali_ref` cannot
     * reach */
    ("rsg_cond_fs", "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet"),
];

#[test]
fn opfs_add_int_mediump() {
    mali_ref::assert_matches("deqp_intval_opfs_add_int_mediump");
}

#[test]
fn opfs_cmp_ivec3() {
    mali_ref::assert_matches("deqp_intval_opfs_cmp_ivec3");
}

#[test]
fn opfs_div_int_mediump() {
    mali_ref::assert_matches("deqp_intval_opfs_div_int_mediump");
}

#[test]
fn opfs_equal_bvec2() {
    mali_ref::assert_matches("deqp_intval_opfs_equal_bvec2");
}

#[test]
fn opfs_logical_xor() {
    mali_ref::assert_matches("deqp_intval_opfs_logical_xor");
}

#[test]
fn opfs_mul_ivec4_highp() {
    mali_ref::assert_matches("deqp_intval_opfs_mul_ivec4_highp");
}

#[test]
fn opfs_not() {
    mali_ref::assert_matches("deqp_intval_opfs_not");
}

#[test]
fn opfs_sub_assign_ivec2_lowp() {
    mali_ref::assert_matches("deqp_intval_opfs_sub_assign_ivec2_lowp");
}

#[test]
fn opvs_add_int_highp() {
    mali_ref::assert_matches("deqp_intval_opvs_add_int_highp");
}

#[test]
fn opvs_div_ivec2_highp() {
    mali_ref::assert_matches("deqp_intval_opvs_div_ivec2_highp");
}

#[test]
fn opvs_less_ivec4() {
    mali_ref::assert_matches("deqp_intval_opvs_less_ivec4");
}

#[test]
#[ignore = "differs: Mesa compares the mediump attributes at fp16, the vertex walk at fp32 (mixprec's vertex side); pinned by refusals_pinned"]
fn opvs_logical_and() {
    mali_ref::assert_matches("deqp_intval_opvs_logical_and");
}

#[test]
fn opvs_mul_ivec3_mediump() {
    mali_ref::assert_matches("deqp_intval_opvs_mul_ivec3_mediump");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * ... (the `any` of two bvec3's `ine`, mixprec's vertex side)"]
fn opvs_not_equal_bvec3() {
    mali_ref::assert_matches("deqp_intval_opvs_not_equal_bvec3");
}

#[test]
fn opvs_post_increment_int() {
    mali_ref::assert_matches("deqp_intval_opvs_post_increment_int");
}

#[test]
fn reslib_bvec_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_bvec_fs");
}

#[test]
fn reslib_float_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_float_fs");
}

#[test]
fn reslib_int_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_int_fs");
}

#[test]
#[ignore = "refused: an integer or boolean value is not lowered yet (its fragment shader's fp32 compares of the widened mediump atemp and their `&&`, mixprec's fragment side)"]
fn reslib_int_vs() {
    mali_ref::assert_matches("deqp_intval_reslib_int_vs");
}

#[test]
fn reslib_mixed_fs() {
    mali_ref::assert_matches("deqp_intval_reslib_mixed_fs");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant (its fragment shader reads the varyings Mesa's link removes as dead); not comparable by mali_ref"]
fn rsg_cond_fs() {
    mali_ref::assert_matches("deqp_intval_rsg_cond_fs");
}

#[test]
fn rsg_constbool_vs() {
    mali_ref::assert_matches("deqp_intval_rsg_constbool_vs");
}

#[test]
fn rsg_conv_vs() {
    mali_ref::assert_matches("deqp_intval_rsg_conv_vs");
}

#[test]
fn uniapi_bool_initial_vs() {
    mali_ref::assert_matches("deqp_intval_uniapi_bool_initial_vs");
}

#[test]
fn uniapi_bool_vs() {
    mali_ref::assert_matches("deqp_intval_uniapi_bool_vs");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, and the three lists cover every `deqp_intval_*`
/// reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_intval_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_intval_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_intval_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to LOWERED or DIFFERS")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_intval_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
