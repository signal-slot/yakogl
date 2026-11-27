// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_fsunif2_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-fsunif2`,
//! `docs/notes/mali-deqp-fsunif2.md`) against the Mali online compiler. These
//! are the shapes of the mediump fragment-shader uniform refusal
//! (`MaliError::FsUniform`, 271 board Fail and 1 NotSupported at 8ff01055)
//! that `tests/mali_deqp_fsunif.rs` did not draw ("not drawn" rows of
//! `docs/notes/mali-deqp-fsunif.md` section 3). None is taken by a lowering
//! yet: every reference is `#[ignore]`d and `refusals_pinned` runs now and
//! holds each one's exact refusal text (the fragment shader's: every vertex
//! shader here compiles), so a lowering must move its pin.
//!
//! HOST-ONLY shapes at ef5ce720: `deqp-host-measure.sh --cases` over the 272
//! board cases of the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055`; every
//! case below began on the host and refused with this text first, in the
//! fragment shader. The other shapes of the 272 are covered by the eight
//! `deqp_fsunif_*` references and earlier ones (see the note).
//!
//! | Shape | Cases | Representative case | Reference |
//! |---|--:|---|---|
//! | `struct_mat_array`: a struct of `mat4`, `mat2` and arrays of them (`array_in_struct.mat4_mat2_fragment`, and `unused_uniforms`) | 9 | `uniform_api.value.assigned.by_pointer.render.array_in_struct.mat4_mat2_fragment` | `struct_mat_array` |
//! | `struct_mat_nested`: the same in nested structs (`nested_structs_arrays.mat4_mat2_fragment`) | 5 | `uniform_api.value.assigned.by_pointer.render.nested_structs_arrays.mat4_mat2_fragment` | `struct_mat_nested` |
//! | `multi_basic_array`: `float[3]`, `ivec3[3]` and `bvec2[3]` together | 9 | `uniform_api.value.assigned.by_pointer.render.multiple_basic_array.fragment` | `multi_basic_array` |
//! | `multi_nested`: three nested structs of float and vec4 arrays | 7 | `uniform_api.value.assigned.by_pointer.render.multiple_nested_structs_arrays.fragment` | `multi_nested` |
//! | `api_random`: `uniform_api.random.*`, structs with samplers, arrays, matrices | 14 | `uniform_api.random.13` | `random_13` (`--zero-uniforms`: a sampler member); `random.23` (more than 64 uniforms) cannot be drawn on Mesa: no reference |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsunif2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the compiler makes byte-equal: none yet.
const MATCHED: &[&str] = &[];

/// The broadcast colour (tests/mali_deqp_fclane.rs), where the int and bool
/// arrays stand since plan #377 spike-uniform-rest3 laid their rows.
const COLOUR_LANE: &str = "a gl_FragColor lane other than the varying's own lane or a constant";

/// Where the matrix-array shapes stand once their rows are laid (plan #387
/// item mali-train-3a, unifmp U1, select.rs `UboLayout::fs16`): the compare
/// chain of every element is more than the pair graph holds.
const TOO_LARGE: &str = "a fragment colour of more than 255 vectorised operations and operands";

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("struct_mat_array", TOO_LARGE),
    ("struct_mat_nested", TOO_LARGE),
    ("multi_basic_array", COLOUR_LANE),
    ("multi_nested", COLOUR_LANE),
    ("random_13", TOO_LARGE),
];

/// [`mali_ref::assert_matches`] of a reference whose struct members
/// gl_ref.py set (`--uniform-values`): its FAU words are compared with the
/// members' values (`mali_ref::differences_members`).
fn assert_matches_members(name: &str) {
    let r = mali_ref::Reference::load(name);
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: the pair is refused: {e}"));
    let d = mali_ref::differences_members(&r, &pair, true);
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_fsunif2_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_fsunif2_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_fsunif2_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// A struct of `mat4`, `mat2` and arrays of both; Mesa's FAU holds each
/// leaf's words in member order (gl_ref.py set the members).
#[test]
#[ignore = "refused: a fragment colour of more than 255 vectorised operations and operands (matrices and arrays in a struct; its rows are laid, unifmp U1)"]
fn struct_mat_array() {
    assert_matches_members("deqp_fsunif2_struct_mat_array");
}

/// The same members in nested structs.
#[test]
#[ignore = "refused: a fragment colour of more than 255 vectorised operations and operands (matrices and arrays in nested structs; its rows are laid, unifmp U1)"]
fn struct_mat_nested() {
    assert_matches_members("deqp_fsunif2_struct_mat_nested");
}

/// `float[3]`, `ivec3[3]` and `bvec2[3]` in one program.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (ivec3 and bvec2 arrays)"]
fn multi_basic_array() {
    mali_ref::assert_matches("deqp_fsunif2_multi_basic_array");
}

/// Three nested structs of float and vec4 arrays.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (nested structs of arrays)"]
fn multi_nested() {
    assert_matches_members("deqp_fsunif2_multi_nested");
}

/// `uniform_api.random.13`: structs of bool, mat, sampler and arrays; every
/// uniform word is 0 (the reference was drawn with `--zero-uniforms`).
#[test]
#[ignore = "refused: a fragment colour of more than 255 vectorised operations and operands (random structs with arrays, matrices and a sampler; its rows are laid, unifmp U1)"]
fn random_13() {
    mali_ref::assert_matches("deqp_fsunif2_random_13");
}
