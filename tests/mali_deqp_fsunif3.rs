// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_fsunif3_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-fsunif3b`) against the Mali online
//! compiler. These are the `uniform_api.random.*` programs of the mediump
//! fragment-shader uniform refusal (`MaliError::FsUniform`) that
//! `tests/mali_deqp_fsunif2.rs` left undrawn: its `api_random` row drew only
//! `random.13` of the 14 (`docs/notes/mali-deqp-fsunif2.md` section 2), and
//! `random.23` cannot be drawn on Mesa (more than 64 uniforms: "export
//! failed", GL_INVALID_OPERATION in `glDrawArrays`). None is taken by a
//! lowering yet: every reference is `#[ignore]`d and `refusals_pinned` runs
//! now and holds each one's exact refusal text (the fragment shader's: no
//! vertex shader here has a uniform), so a lowering must move its pin.
//!
//! HOST-ONLY at 22c5fc5f: `deqp-host-measure.sh --cases` over the 12
//! programs `uniform_api.random.{4,9,31,34,39,42,47,62,80,81,89,93}` (board
//! Fail in the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055`, VK-GL-CTS
//! 93bca01 with the yakogl target); all 12 began on the host and refused with
//! this text first, in the fragment shader (none not reproduced). Grouped by
//! the uniform constructs the program reads (the first read uniform `fs16`
//! takes in no arm refuses, `docs/notes/mali-deqp-fsunif2.md` section 5);
//! eight drawn, four whose constructs the drawn ones or earlier references
//! already hold left out:
//!
//! | Shape | Cases | Constructs the fragment shader reads | Reference |
//! |---|--:|---|---|
//! | `arr_mat3_mat2`: top-level `mat3` and `mat2` arrays | 1 | `mat3[5]`, `mat3[3]`, `mat2[4]`, `mat3`, `vec2` | `random_80` |
//! | `arr_int_ivec2`: top-level `int` and `ivec2` arrays with matrices | 1 | `int[4]`, `ivec2[2]`, `mat2`, `mat3`, `vec3` | `random_34` |
//! | `arr_bvec_mat3`: top-level `bvec4` and `mat3` arrays, a `bvec4` struct | 1 | `bvec4[4]`, `mat3[2]`, `mat4`, `bvec2`, struct `bvec4` | `random_4` |
//! | `struct_sampler_bvec`: a struct with a `samplerCube` member, a `bvec4` array | 1 | `bvec4[2]`, `ivec3`, struct `bvec2`/`bvec4` (`--zero-uniforms`) | `random_93` |
//! | `struct_mat3_arr_sampler`: nested structs with a `mat3` array and a struct array, a `sampler2D` | 1 | struct `mat3[2]`, `mat2`, `int`, `bool`, `vec2`, struct array; `float`, `mat3` (`--zero-uniforms`) | `random_9` |
//! | `struct_ivec_arr`: nested structs with `ivec4`, `int` and `vec2` arrays | 1 | struct `ivec4[2]`, `int[1]`, `vec2[2]` | `random_31` |
//! | `struct_bvec_arr_sampler`: a struct with a `bvec2` array and matrices, a `samplerCube` | 1 | struct `mat3`, `bvec2[2]`, `mat2`; `float` (`--zero-uniforms`) | `random_62` |
//! | `struct_mat4_arr_ivec_arr`: nested structs with a `mat4` array, an `ivec3` array | 1 | struct `mat4[2]`, `vec2`, `ivec3[4]`, `vec4`; `mat4[1]`, `mat4`, `ivec2` | `random_81` |
//! | not drawn: `ivec4[3]` (and an unread `mat2`) | 1 | `random.39` | `deqp_fsunif_arr_ivec4` |
//! | not drawn: `mat4[3]`, `ivec2`, `ivec3` | 1 | `random.42` | `deqp_fsunif_arr_mat4` |
//! | not drawn: struct `bvec4[3]`, `int[1]`, `bvec4[5]`, a `sampler2D` | 1 | `random.47` | `random_4`, `random_9`, `deqp_fsunif_struct_int_ivec4` |
//! | not drawn: `bvec3[2]`, structs of `vec4[2]`, `mat3`, `ivec4`, `bvec2` | 1 | `random.89` | `random_4`, `random_31`, `deqp_fsunif_arr_bvec4` |
//!
//! The three programs with a sampler were drawn with `--zero-uniforms` (as
//! `deqp_fsunif2_random_13`): every uniform word is 0. The other five with
//! gl_ref.py's values (`--uniform-values`); gl_ref.py leaves a top-level
//! array unset (its location lookup is -1), and the three with struct
//! members (`random_4`, `random_31`, `random_81`) compare their FAU words
//! with the members' values (`mali_ref::differences_members`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsunif3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix and the ` is not lowered yet`
/// suffix.
const FS_UNIFORM: &str = "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp";

/// The references the compiler makes byte-equal: none yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("random_4", FS_UNIFORM),
    ("random_9", FS_UNIFORM),
    ("random_31", FS_UNIFORM),
    ("random_34", FS_UNIFORM),
    ("random_62", FS_UNIFORM),
    ("random_80", FS_UNIFORM),
    ("random_81", FS_UNIFORM),
    ("random_93", FS_UNIFORM),
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
            f.strip_prefix("deqp_fsunif3_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_fsunif3_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_fsunif3_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// `uniform_api.random.4`: a `bvec4[4]` and a `mat3[2]` read one element
/// each, a `mat4`, a `bvec2` and a struct's `bvec4`.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (bvec4 and mat3 arrays)"]
fn random_4() {
    assert_matches_members("deqp_fsunif3_random_4");
}

/// `uniform_api.random.9`: nested structs with a `mat3[2]` and an array of
/// structs (`int`, `mat2`, `vec2`), a `sampler2D`; every uniform word is 0
/// (`--zero-uniforms`).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a mat3 array and a struct array in nested structs)"]
fn random_9() {
    mali_ref::assert_matches("deqp_fsunif3_random_9");
}

/// `uniform_api.random.31`: nested structs with `ivec4[2]`, `int[1]` and
/// `vec2[2]` members.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (ivec4 and int arrays in nested structs)"]
fn random_31() {
    assert_matches_members("deqp_fsunif3_random_31");
}

/// `uniform_api.random.34`: `int[4]` and `ivec2[2]` (one element read each),
/// `mat2`, `mat3` and `vec3`.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (int and ivec2 arrays, mat2, mat3)"]
fn random_34() {
    mali_ref::assert_matches("deqp_fsunif3_random_34");
}

/// `uniform_api.random.62`: a struct of `mat3`, `bvec2[2]`, `float` and
/// `mat2`, an unread `samplerCube`; every uniform word is 0
/// (`--zero-uniforms`).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a bvec2 array and matrices in a struct)"]
fn random_62() {
    mali_ref::assert_matches("deqp_fsunif3_random_62");
}

/// `uniform_api.random.80`: `mat3[5]`, `mat3[3]` and `mat2[4]` (one element
/// read each), a `mat3` and a `vec2`.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (mat3 and mat2 arrays)"]
fn random_80() {
    mali_ref::assert_matches("deqp_fsunif3_random_80");
}

/// `uniform_api.random.81`: nested structs with a `mat4[2]`, a struct with
/// an `ivec3[4]`, a `mat4[1]`, a `mat4` and an `ivec2`.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (mat4 and ivec3 arrays in structs)"]
fn random_81() {
    assert_matches_members("deqp_fsunif3_random_81");
}

/// `uniform_api.random.93`: a `bvec4[2]`, an `ivec3`, a struct of `bvec2`,
/// `samplerCube` and `bvec4`; every uniform word is 0 (`--zero-uniforms`).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (a bvec4 array, a struct with a sampler)"]
fn random_93() {
    mali_ref::assert_matches("deqp_fsunif3_random_93");
}
