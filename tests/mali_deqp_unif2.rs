// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_unif2_*` (plan #387, item unif-ref,
//! `docs/notes/mali-deqp-unif2.md`), in the form of `tests/mali_deqp_intval.rs`.
//! They are the uniform shapes of the dEQP-GLES2 shader pairs that no
//! `deqp_unitype_*` or `deqp_uniarray_*` reference covers (the note's section
//! 2 names the existing reference of each other shape). Drawn with
//! `tools/mali/drm_shim_ref.sh run` and `pan_ref_diff.py extract --draw
//! --attributes` (note, section 4). The four struct shapes
//! (`arr_struct_array`, `fs_struct_uniform`, `vs_mediump_struct`,
//! `vs_struct_uniform`) are drawn without `--uniform-values`, which
//! `gl_ref.py` does not support for a struct uniform: their FAU words are
//! Mesa's with every uniform 0, and they are refused, so no test compares them.
//!
//! `vs_mat_uniform` compiles byte-equal ([`LOWERED`]); the rest stay
//! `#[ignore]`d with their whole refusal ([`REFUSED`], asserted exactly by
//! `refusals_pinned`): the lowering that lifts one moves it in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_unif2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

const ARRAY: &str = "Valhall: a uniform array is not lowered yet";
const OTHER_VS: &str = "Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet";
const FS_OTHER: &str = "Valhall: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, is not lowered yet";
const LANE: &str = "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
const ARITH: &str = "Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet";

/// The references that compile byte-equal, without the `deqp_unif2_` prefix.
const LOWERED: &[&str] = &["vs_mat_uniform"];

/// Every ignored reference with its whole `MaliError` message.
const REFUSED: &[(&str, &str)] = &[
    ("arr_bool_array", ARRAY),
    ("arr_int_array", ARRAY),
    ("arr_mat_array", ARRAY),
    ("arr_struct_array", OTHER_VS),
    ("fs_bool_uniform", FS_OTHER),
    ("fs_int_uniform", FS_OTHER),
    ("fs_mat_uniform", LANE),
    ("fs_other", FS_OTHER),
    ("fs_struct_uniform", FS_OTHER),
    ("vs_bool_uniform", ARITH),
    ("vs_int_uniform", OTHER_VS),
    ("vs_mediump_array", LANE),
    ("vs_mediump_struct", LANE),
    ("vs_struct_uniform", OTHER_VS),
];

#[test]
fn refusals_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_unif2_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = LOWERED.iter().copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_unif2_* reference has no test here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_unif2_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(_) => changed.push(format!("{name}: now compiles; move it to LOWERED")),
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
fn vs_mat_uniform() {
    mali_ref::assert_matches("deqp_unif2_vs_mat_uniform");
}

/// An ignored test per refused reference: the byte-compare the lowering enables.
macro_rules! refused {
    ($($name:ident),* $(,)?) => {$(
        #[test]
        #[ignore = "refused: see REFUSED"]
        fn $name() {
            mali_ref::assert_matches(concat!("deqp_unif2_", stringify!($name)));
        }
    )*};
}

refused!(
    arr_bool_array,
    arr_int_array,
    arr_mat_array,
    arr_struct_array,
    fs_bool_uniform,
    fs_int_uniform,
    fs_mat_uniform,
    fs_other,
    fs_struct_uniform,
    vs_bool_uniform,
    vs_int_uniform,
    vs_mediump_array,
    vs_mediump_struct,
    vs_struct_uniform,
);
