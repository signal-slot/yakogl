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

/// The arithmetic refusal of a fragment shader.
const ARITH: &str = "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)";
/// A gl_FragColor lane that is not the varying's own or a constant.
const FRAGCOLOR: &str = "a gl_FragColor lane other than the varying's own lane or a constant";
/// Control flow.
const CTRL: &str = "control flow (if, a loop, a return before the end)";
/// A loop past Mesa's unroll limits.
const UNROLL: &str = "a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside,";
/// A constant operation on a value that is not a finite normal float.
const NONFINITE: &str = "a constant operation on or to a value that is not a finite normal float";

/// Every reference (without the `deqp_idxloc_` prefix) still refused, with the
/// first refusal of its pair at the tip, without the "Valhall: " prefix and
/// the " is not lowered yet" suffix.
const REFUSALS: &[(&str, &str)] = &[
    ("fs_mat2_dd", ARITH),
    ("fs_mat3_dl_sl", FRAGCOLOR),
    ("fs_struct_loop_assign", LOOP_MEDIUMP),
    ("fs_struct_nested_array", LOOP_MEDIUMP),
    ("fs_struct_nested_loop_assign", CTRL),
    ("fs_struct_uni_nested_array", LOOP_MEDIUMP),
    ("fs_tmp_float_cd", LOOP_MEDIUMP),
    ("fs_tmp_vec3_sl_dl", LOOP_MEDIUMP),
    ("fs_uni_vec3", LOOP_MEDIUMP),
    ("fs_var_float_sd", LOOP_MEDIUMP),
    ("fs_var_vec3_sd", LOOP_MEDIUMP),
    ("fs_vec2_dl_static", UNROLL),
    ("fs_vec3_component_dl", LOOP_MEDIUMP),
    ("vs_mat2_dl_d", NONFINITE),
    ("vs_tmp_vec2_dl_sl", NONFINITE),
    ("vs_var_float_dl_s", NONFINITE),
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

/// References that compile at the tip with other words than Mesa's:
/// `fs_tmp_vec2_dl_sl`'s second fp16 `FMA` takes its operands in the other
/// order. Not refused, so not in [`REFUSALS`].
const DIFFERS: &[&str] = &["fs_tmp_vec2_dl_sl"];

#[test]
fn differences_pinned() {
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_idxloc_{name}"));
        let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal; add it to MATCHED");
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
    let mut listed: Vec<String> = REFUSALS
        .iter()
        .map(|(n, _)| n.to_string())
        .chain(MATCHED.iter().chain(DIFFERS).map(|n| n.to_string()))
        .collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a deqp_idxloc_* reference is not listed, or the other way round");
}
