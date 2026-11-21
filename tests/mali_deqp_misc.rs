// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_misc_*` (plan #377, family misc: the
//! small refusals — comparison forms, the mediump/lowp vertex `mat4 * mat4`
//! over 32 work registers, mediump `gl_FragCoord.z`, the factored constant
//! matrix products, an invariant `gl_Position`; see
//! `docs/notes/mali-deqp-misc.md`). One test per reference. Nine compile
//! byte-equal and run: `cmp_fne_hp`, `cmp_ine0_hp`, `cmp_notequal_vec2`,
//! the four `constmat_*`, `fragcoord_xyz` and `invariant_position` (a copy,
//! which has no arithmetic to keep exact). The two `spill_*` are refused,
//! each test ignored with its refusal quoted; the lowering removes the
//! `ignore` and the reference's entry in `REFUSALS` together. `refusals_pinned` runs now: it
//! asserts each ignored reference's exact current refusal (the vertex
//! shader's when it refuses, else the fragment shader's).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_misc
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
fn misc_cmp_fne_hp() {
    mali_ref::assert_matches("deqp_misc_cmp_fne_hp");
}

#[test]
fn misc_cmp_ine0_hp() {
    mali_ref::assert_matches("deqp_misc_cmp_ine0_hp");
}

/// Drawn with `--uniform-values`: the FAU word of the bool `ref_out0` is
/// Mesa's true, `ffffffff`.
#[test]
fn misc_cmp_notequal_vec2() {
    mali_ref::assert_matches("deqp_misc_cmp_notequal_vec2");
}

#[test]
fn misc_constmat_mat2_highp() {
    mali_ref::assert_matches("deqp_misc_constmat_mat2_highp");
}

#[test]
fn misc_constmat_mat2_mediump() {
    mali_ref::assert_matches("deqp_misc_constmat_mat2_mediump");
}

#[test]
fn misc_constmat_mat4_highp() {
    mali_ref::assert_matches("deqp_misc_constmat_mat4_highp");
}

#[test]
fn misc_constmat_mat4_mediump() {
    mali_ref::assert_matches("deqp_misc_constmat_mat4_mediump");
}

#[test]
fn misc_fragcoord_xyz() {
    mali_ref::assert_matches("deqp_misc_fragcoord_xyz");
}

#[test]
fn misc_invariant_position() {
    mali_ref::assert_matches("deqp_misc_invariant_position");
}

#[test]
#[ignore = "refused: a program needing more than the split file's 32 work registers (the full file, spilling), lowered by lower-misc"]
fn misc_spill_lowp_mat4_mat4() {
    mali_ref::assert_matches("deqp_misc_spill_lowp_mat4_mat4");
}

#[test]
#[ignore = "refused: a program needing more than the split file's 32 work registers (the full file, spilling), lowered by lower-misc"]
fn misc_spill_mediump_mat4_mat4() {
    mali_ref::assert_matches("deqp_misc_spill_mediump_mat4_mat4");
}

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    (
        "deqp_misc_spill_lowp_mat4_mat4",
        "a program needing more than the split file's 32 work registers (the full file, spilling)",
    ),
    (
        "deqp_misc_spill_mediump_mat4_mat4",
        "a program needing more than the split file's 32 work registers (the full file, spilling)",
    ),
];

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        mali_ref::assert_refused_ref(name, text);
    }
}
