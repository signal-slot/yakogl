// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mesa 26.2.1 references of the dEQP-GLES2 `shaders.indexing.*` cases
//! whose first refusal is a loop text and which index a local array, a
//! vector, a matrix, a uniform or a varying array at the loop's counter
//! (steps S2, S7 and S11 of `docs/notes/mali-loops-lowering.md`; plan #387
//! item idxloc-ref), and the four `struct` loops of S7. One reference per
//! shape of `docs/notes/mali-deqp-idxloc.md`, for a type no
//! `deqp_loops*_idx_*` reference covers. Each was drawn on Mesa's drm-shim
//! with `tools/mali/deqp_ref.sh --instrument shim`; the sources are rebuilt
//! from the dEQP generators (`es2fShaderIndexingTests.cpp`,
//! `es2fShaderStructTests.cpp`), as each `.license` says.
//!
//! No reference compiles at the tip: [`refusals_pinned`] asserts each pair's
//! exact first refusal, the loop text. The note gives the refusal measured
//! after the loop (the loop folded by hand) per shape. The lowering that
//! lifts one moves its entry from [`REFUSALS`] to [`MATCHED`] in the same
//! commit. Mesa keeps a loop in eight of them (`fs_tmp_float_cd`,
//! `fs_vec2_dl_static`, `fs_vec3_component_dl`, `vs_vec3_dl_component` and
//! the four `fs_struct_*`): those carry the pass counter the walk adds, so
//! compare them with `CapEdit` as `tests/mali_deqp_loops.rs` says; the other
//! ten have no loop in Mesa's words and are plain
//! [`mali_ref::assert_matches`]. `fs_struct_uni_nested_array` has
//! struct-member uniforms: its compare needs its name in `mali_ref`'s
//! `STRUCT_VALUES`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_idxloc
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `MaliError::LoopMediump`.
const LOOP_MEDIUMP: &str =
    "a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge)";
/// `MaliError::LoopVertex`.
const LOOP_VERTEX: &str = "a loop in a vertex shader";

/// Every reference (without the `deqp_idxloc_` prefix) still refused, with the
/// first refusal of its pair at the tip, without the "Valhall: " prefix and
/// the " is not lowered yet" suffix.
const REFUSALS: &[(&str, &str)] = &[
    ("fs_mat2_dd", LOOP_MEDIUMP),
    ("fs_mat3_dl_sl", LOOP_MEDIUMP),
    ("fs_struct_loop_assign", LOOP_MEDIUMP),
    ("fs_struct_nested_array", LOOP_MEDIUMP),
    ("fs_struct_nested_loop_assign", LOOP_MEDIUMP),
    ("fs_struct_uni_nested_array", LOOP_MEDIUMP),
    ("fs_tmp_float_cd", LOOP_MEDIUMP),
    ("fs_tmp_vec2_dl_sl", LOOP_MEDIUMP),
    ("fs_tmp_vec3_sl_dl", LOOP_MEDIUMP),
    ("fs_uni_vec3", LOOP_MEDIUMP),
    ("fs_var_float_sd", LOOP_MEDIUMP),
    ("fs_var_vec3_sd", LOOP_MEDIUMP),
    ("fs_vec2_dl_static", LOOP_MEDIUMP),
    ("fs_vec3_component_dl", LOOP_MEDIUMP),
    ("vs_mat2_dl_d", LOOP_VERTEX),
    ("vs_tmp_vec2_dl_sl", LOOP_VERTEX),
    ("vs_var_float_dl_s", LOOP_VERTEX),
    ("vs_vec3_dl_component", LOOP_VERTEX),
];

/// References whose pair compiles and matches Mesa's words byte for byte
/// (`mali_ref::assert_matches`). Empty until a lowering lifts a refusal.
const MATCHED: &[&str] = &[];

fn refusal(name: &str) -> Result<(), String> {
    let r = mali_ref::Reference::load(&format!("deqp_idxloc_{name}"));
    mali_ref::compile_pair(&r.vs, &r.fs).map(|_| ())
}

#[test]
fn refusals_pinned() {
    for (name, text) in REFUSALS {
        match refusal(name) {
            Ok(()) => panic!("{name} now compiles: add its byte compare and move it to MATCHED"),
            Err(e) => assert_eq!(
                e.strip_prefix("Valhall: ").and_then(|e| e.strip_suffix(" is not lowered yet")),
                Some(*text),
                "{name}: the refusal moved: {e}"
            ),
        }
    }
}

/// The compares that become the gate once a refusal is lifted (`--ignored`).
/// Each fails at the tip with its refusal.
#[test]
#[ignore = "refused at the tip: steps S2, S7 and S11 of mali-loops-lowering.md, then the next refusals of mali-deqp-idxloc.md"]
fn compares() {
    for (name, _) in REFUSALS {
        mali_ref::assert_matches(&format!("deqp_idxloc_{name}"));
    }
}

#[test]
fn matched_compile() {
    for name in MATCHED {
        refusal(name).unwrap_or_else(|e| panic!("{name} is MATCHED but is refused: {e}"));
    }
}

#[test]
fn references_listed() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("deqp_idxloc_").and_then(|n| n.strip_suffix(".expect.json")).map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSALS.iter().map(|(n, _)| n.to_string()).chain(MATCHED.iter().map(|n| n.to_string())).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_idxloc_* reference is not listed, or the other way round");
}
