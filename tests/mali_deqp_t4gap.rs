// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the `tools/mali/testdata/deqp_t4gap_*` references: the
//! Mesa 26.2.1 references that a step of `docs/notes/mali-lowering-schedule.md`
//! section 5.5a (Train 4) lacked (plan #387 item `train4-refgaps`,
//! `docs/notes/mali-train4-refs.md`). Each is drawn on Mesa 26.2.1's G610
//! drm-shim with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim`.
//!
//! All ten are `load_mat3` shapes (train 4 commit 4, `mali-highpfs-lowering.md`
//! L3): the vertex half of nine `_vertex` cases of
//! `dEQP-GLES2.functional.shaders.conversions.*` that the design pinned only
//! for three, and `shaders.linkage.varying_type_mat3`. The case name is the
//! reference's name without `highpfs_l3_` and `_vs`
//! (`docs/notes/mali-train4-refs.md` has the table).
//!
//! [`STATE`] pins each reference's state at the tip: `Ok(true)` byte-equal,
//! `Ok(false)` compiles with other words than Mesa's, `Err(text)` refused with
//! the whole `MaliError` text. The step that changes a state moves it here
//! and drops the test's `#[ignore]` in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_t4gap
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal of every reference at the tip: the fragment walk reads a
/// `mat3` varying load that was never emitted (`mali-deqp-irindex.md` 5.3).
const IRINDEX: &str = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)";

const STATE: &[(&str, Result<bool, &str>)] = &[
    ("highpfs_l3_bool_bvec2_int_vec4_bool_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_bool_float_int_vec2_bool_bvec2_float_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_bool_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_float_bvec4_ivec2_bool_bool_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_int_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_ivec3_ivec3_ivec3_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_mat2_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_mat4_to_mat3_vs", Err(IRINDEX)),
    ("highpfs_l3_varying_type_mat3", Err(IRINDEX)),
    ("highpfs_l3_vec2_ivec2_float_float_int_bool_bool_to_mat3_vs", Err(IRINDEX)),
];

fn reference(name: &str) -> mali_ref::Reference {
    mali_ref::Reference::load(&format!("deqp_t4gap_{name}"))
}

fn differences(name: &str) -> Result<Vec<String>, String> {
    let r = reference(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs)?;
    Ok(mali_ref::differences_members(&r, &pair, false))
}

/// Every reference on disk is pinned, and each is in its pinned state.
#[test]
fn state_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_t4gap_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let listed: Vec<String> = STATE.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(on_disk, listed, "every deqp_t4gap_* reference has a pin (sorted)");
    let mut changed = Vec::new();
    for (name, want) in STATE {
        let now = differences(name).map(|d| d.is_empty());
        let same = match (&now, want) {
            (Ok(a), Ok(b)) => a == b,
            (Err(a), Err(b)) => a == b,
            _ => false,
        };
        if !same {
            changed.push(format!("{name}: pinned {want:?}, now {now:?}"));
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

fn assert_matches(name: &str) {
    let d = differences(name).unwrap_or_else(|e| panic!("deqp_t4gap_{name}: the pair is refused: {e}"));
    assert!(d.is_empty(), "deqp_t4gap_{name}: differs from its reference:\n{}", d.join("\n"));
}

/// Every pinned refusal is still that refusal, whole text, or says at once
/// which reference moved (a reference that now compiles moves to `Ok(..)`).
#[test]
fn refusals_pinned() {
    let mut moved = Vec::new();
    for (name, want) in STATE {
        if let Err(text) = want {
            if let Some(m) = mali_ref::refusal_change(&format!("deqp_t4gap_{name}"), text) {
                moved.push(m);
            }
        }
    }
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_bool_bvec2_int_vec4_bool_to_mat3_vs() {
    assert_matches("highpfs_l3_bool_bvec2_int_vec4_bool_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_bool_float_int_vec2_bool_bvec2_float_to_mat3_vs() {
    assert_matches("highpfs_l3_bool_float_int_vec2_bool_bvec2_float_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_bool_to_mat3_vs() {
    assert_matches("highpfs_l3_bool_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_float_bvec4_ivec2_bool_bool_to_mat3_vs() {
    assert_matches("highpfs_l3_float_bvec4_ivec2_bool_bool_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_int_to_mat3_vs() {
    assert_matches("highpfs_l3_int_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_ivec3_ivec3_ivec3_to_mat3_vs() {
    assert_matches("highpfs_l3_ivec3_ivec3_ivec3_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_mat2_to_mat3_vs() {
    assert_matches("highpfs_l3_mat2_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_mat4_to_mat3_vs() {
    assert_matches("highpfs_l3_mat4_to_mat3_vs");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_varying_type_mat3() {
    assert_matches("highpfs_l3_varying_type_mat3");
}

/// Refused with [`IRINDEX`] (train 4 commit 4, `load_mat3` L3).
#[test]
#[ignore = "refused: Defect(IrIndex), load_mat3 (train 4 commit 4)"]
fn highpfs_l3_vec2_ivec2_float_float_int_bool_bool_to_mat3_vs() {
    assert_matches("highpfs_l3_vec2_ivec2_float_float_int_bool_bool_to_mat3_vs");
}
