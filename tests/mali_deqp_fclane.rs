// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_fclane_*` (plan #387, item
//! fclane-ref, `docs/notes/mali-deqp-fclane.md`): one reference per colour
//! shape of the dEQP-GLES2 fragment shaders whose first refusal is
//! `a gl_FragColor lane other than the varying's own lane or a constant`
//! (821 cases at `9ac31f30`, HOST-ONLY count). `tests/mali_ref` does the
//! compare; the references were drawn with `--uniform-values`
//! (`tools/mali/deqp_ref.sh --instrument shim`, the command is in the note).
//!
//! Ten references are byte-equal ([`LOWERED`]): mechanisms A, B and D's
//! `tex_coord_ramp` of `docs/notes/mali-fclane-lowering.md` (plan #387,
//! item fclane-lower-1). Every other is refused, so its test is `#[ignore]`d
//! with its refusal pinned exactly by `refusals_pinned` ([`REFUSED`]): the
//! lowering step that lifts a shape moves it to [`LOWERED`] (or [`DIFFERS`]
//! when it compiles with other words than Mesa's) in the same commit, drops
//! the `#[ignore]` and runs the compare. Shapes whose reference `gl_ref.py`
//! cannot draw (a `structType` uniform of the `uniform_api` cases) have none:
//! see the note. `mat_cols_sum` and `mat_elem_pairs` were drawn again with
//! `--mat-columns` (their vertex shader's `a_coords` after a matrix
//! attribute took the matrix's second column's location without it).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fclane
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal, without the
/// `deqp_fclane_` prefix: each has a byte-compare test below.
const LOWERED: &[&str] = &[
    "mat_cols_sum",
    "mat_elem_pairs",
    "mat_vec_lanes",
    "op_pair_xyz",
    "op_pair_xyzw",
    "op_pair_yz",
    "struct_fields",
    "tex_coord_ramp",
    "tex_zw_lane",
    "varying_merged",
];

/// The references that compile with other words than Mesa's.
const DIFFERS: &[&str] = &[];

const COLOUR_FORM: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
/// `mat4_swz_sum`'s refusal past its colour lanes: the sums of a constant
/// matrix's lanes, which Mesa's rules fold and reassociate once they are
/// scalar (`FMA.f32 r1, r1^, 0x44000000.h1, 0x0.neg` of four equal lanes).
const ARITH_CONSTANT: &str = "Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is not lowered yet";
/// `op_pair_x`'s: its lane is `exp`, the arith family's.
const ARITHMETIC: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";

/// Every ignored reference with its whole `MaliError` message (all twenty
/// were the colour-lane text at `9ac31f30`; one whose lane source is
/// lowered and whose lane operation is another family's has that family's
/// text, as the note's section 5 says).
const REFUSED: &[(&str, &str)] = &[
    ("bcast_compare_array", COLOUR_FORM),
    ("bcast_compare_bool", COLOUR_FORM),
    ("bcast_compare_float", COLOUR_FORM),
    ("bcast_compare_int", COLOUR_FORM),
    ("bcast_const", COLOUR_FORM),
    ("mat4_swz_sum", ARITH_CONSTANT),
    ("op_pair_other", COLOUR_FORM),
    ("op_pair_x", ARITHMETIC),
    ("rnd_vec4", COLOUR_FORM),
    ("vec3_local", COLOUR_FORM),
];

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn bcast_compare_array() {
    mali_ref::assert_matches("deqp_fclane_bcast_compare_array");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn bcast_compare_bool() {
    mali_ref::assert_matches("deqp_fclane_bcast_compare_bool");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn bcast_compare_float() {
    mali_ref::assert_matches("deqp_fclane_bcast_compare_float");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn bcast_compare_int() {
    mali_ref::assert_matches("deqp_fclane_bcast_compare_int");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn bcast_const() {
    mali_ref::assert_matches("deqp_fclane_bcast_const");
}

#[test]
#[ignore = "refused: a constant product in a shape Mesa's algebraic rules move or fold (the sums of a constant matrix's lanes); pinned by refusals_pinned"]
fn mat4_swz_sum() {
    mali_ref::assert_matches("deqp_fclane_mat4_swz_sum");
}

#[test]
fn mat_cols_sum() {
    mali_ref::assert_matches("deqp_fclane_mat_cols_sum");
}

#[test]
fn mat_elem_pairs() {
    mali_ref::assert_matches("deqp_fclane_mat_elem_pairs");
}

#[test]
fn mat_vec_lanes() {
    mali_ref::assert_matches("deqp_fclane_mat_vec_lanes");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn op_pair_other() {
    mali_ref::assert_matches("deqp_fclane_op_pair_other");
}

#[test]
#[ignore = "refused: arithmetic other than +, -, * (exp, the arith family's); pinned by refusals_pinned"]
fn op_pair_x() {
    mali_ref::assert_matches("deqp_fclane_op_pair_x");
}

#[test]
fn op_pair_xyz() {
    mali_ref::assert_matches("deqp_fclane_op_pair_xyz");
}

#[test]
fn op_pair_xyzw() {
    mali_ref::assert_matches("deqp_fclane_op_pair_xyzw");
}

#[test]
fn op_pair_yz() {
    mali_ref::assert_matches("deqp_fclane_op_pair_yz");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn rnd_vec4() {
    mali_ref::assert_matches("deqp_fclane_rnd_vec4");
}

#[test]
fn struct_fields() {
    mali_ref::assert_matches("deqp_fclane_struct_fields");
}

#[test]
fn tex_coord_ramp() {
    mali_ref::assert_matches("deqp_fclane_tex_coord_ramp");
}

#[test]
fn tex_zw_lane() {
    mali_ref::assert_matches("deqp_fclane_tex_zw_lane");
}

#[test]
fn varying_merged() {
    mali_ref::assert_matches("deqp_fclane_varying_merged");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant; pinned by refusals_pinned"]
fn vec3_local() {
    mali_ref::assert_matches("deqp_fclane_vec3_local");
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, and the three lists cover every `deqp_fclane_*`
/// reference on disk.
#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_fclane_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_fclane_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_fclane_{name}"));
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
        let r = mali_ref::Reference::load(&format!("deqp_fclane_{name}"));
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
