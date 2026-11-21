// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_fclane2_*` (plan #387, item
//! fclane2-struct): the `bcast_compare` shape of
//! `docs/notes/mali-deqp-fclane.md` (`vec4(result, result, result, 1.0)` of
//! a product of uniform compares) where the uniform is a `structType` of
//! the dEQP-GLES2 `uniform_api` cases, which had no reference while
//! `tools/mali/gl_ref.py` could not set struct members (94 must-pass cases
//! at `9ac31f30`, HOST-ONLY count of the note). The fragment shader is the
//! case's; the vertex shader is the `_fragment` cases' (`v_vtxOut = 1.0`,
//! `deqp_fclane_bcast_compare_int.vert`). One test per reference:
//!
//! - `struct_basic_float`: `uniform_api ... basic_struct.float_vec4`
//!   (`struct { mediump float m0; mediump vec4 m1; }`), the fragment shader
//!   verbatim from `deqp_unif2_vs_mediump_struct.frag`
//!   (`value.assigned.by_pointer.get_uniform.basic_struct.float_vec4_both`);
//! - `struct_basic_int`, `struct_basic_bool`: `basic_struct.int_ivec4` and
//!   `.bool_bvec4`, rebuilt from the generator (`es2fUniformApiTests.cpp`
//!   `basicStruct`, no array members) in the text of the verbatim
//!   `array_in_struct` shaders;
//! - `struct_arrmember_float`: `array_in_struct.float_vec4` (members `m2[3]`,
//!   `m3[3]`), verbatim from `deqp_uniarray_struct.frag`
//!   (`basic_array_assign_full.array_in_struct.float_vec4_both`);
//! - `struct_array_float`: `struct_in_array.float_vec4` (`uniform structType
//!   u_var[3]`, read as `u_var[i].m0`), rebuilt (`structInArray`);
//! - `struct_nested_float`: `nested_structs_arrays.float_vec4` (a struct
//!   member `m1` holding `ms1[2]` and a struct array `ms2[2]`), rebuilt
//!   (`nestedArraysStructs`).
//!
//! A rebuilt shader has the generator's declarations and compare functions;
//! its literals are not a case's (they are constants in the words and FAU:
//! a lowering compares them as they are). `int_ivec4` and `bool_bvec4` with
//! array members are refused earlier by the mediump-uniform text (the
//! `deqp_unif2_*_struct_*` references), so they are not here.
//!
//! Each reference was drawn with `tools/mali/deqp_ref.sh <vert> <frag>
//! tools/mali/testdata/deqp_fclane2_<shape> --instrument shim`, that is
//! `gl_ref.py --uniform-values`, which sets every struct member by its dEQP
//! name (`u_var.m2[1]`, `u_var[2].m1`, `u_var.m1.ms2[0].mss1`), each member
//! the next value index (`tools/mali/README-deqp_ref.md`, "Struct
//! uniforms"). `struct_members_set` checks the FAU holds the first member's
//! value. Note: `tests/mali_ref`'s `differences` still compares a program
//! with a struct-member uniform row against every uniform 0 (the older
//! struct references were drawn without values); these references need it
//! to compute gl_ref.py's member values before a byte-compare can pass.
//!
//! None compiles at the tip: each test is ignored with its refusal quoted;
//! the lowering removes the `ignore` and moves the reference from
//! `REFUSALS` to `MATCHED` together. `refusals_pinned` runs now: it asserts
//! each ignored reference's exact current refusal and names every one that
//! moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fclane2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn fclane2_struct_basic_float() {
    mali_ref::assert_matches("deqp_fclane2_struct_basic_float");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn fclane2_struct_basic_int() {
    mali_ref::assert_matches("deqp_fclane2_struct_basic_int");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn fclane2_struct_basic_bool() {
    mali_ref::assert_matches("deqp_fclane2_struct_basic_bool");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn fclane2_struct_arrmember_float() {
    mali_ref::assert_matches("deqp_fclane2_struct_arrmember_float");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn fclane2_struct_array_float() {
    mali_ref::assert_matches("deqp_fclane2_struct_array_float");
}

#[test]
#[ignore = "refused: a gl_FragColor lane other than the varying's own lane or a constant"]
fn fclane2_struct_nested_float() {
    mali_ref::assert_matches("deqp_fclane2_struct_nested_float");
}

const COLOUR_FORM: &str =
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";

/// Every ignored reference above and the refusal the compiler gives it now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_fclane2_struct_arrmember_float", COLOUR_FORM),
    ("deqp_fclane2_struct_array_float", COLOUR_FORM),
    ("deqp_fclane2_struct_basic_bool", COLOUR_FORM),
    ("deqp_fclane2_struct_basic_float", COLOUR_FORM),
    ("deqp_fclane2_struct_basic_int", COLOUR_FORM),
    ("deqp_fclane2_struct_nested_float", COLOUR_FORM),
];

/// The references that compile byte-equal (their tests run): none yet.
const MATCHED: &[&str] = &[];

fn on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_fclane2_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    names.sort();
    names
}

#[test]
fn refusals_pinned() {
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk(), named, "a deqp_fclane2_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS.iter().filter_map(|(name, text)| mali_ref::refusal_change(name, text)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// The fragment program's FAU words of a reference's expect.json.
fn fragment_fau(name: &str) -> Vec<String> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tools/mali/testdata/{name}.expect.json"));
    let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let fs = &json[json.find("\"fs.fragment\"").expect("an fs.fragment region")..];
    let fau = &fs[fs.find("\"fau\": [").expect("a fau list") + 8..];
    fau[..fau.find(']').expect("the end of the fau list")]
        .split(',')
        .map(|w| w.trim().trim_matches('"').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Each reference was drawn with its struct members set: the first member
/// (`u_var.m0`, `u_var[0].m0`, value index 0 of `gl_ref.py --uniform-values`)
/// is in the fragment FAU as Mesa uploads it: 16.0 as fp32 for a mediump
/// float member, 16 for an int, all ones (true) for a bool. Without the
/// values every uniform word would be 0.
#[test]
fn struct_members_set() {
    for name in on_disk() {
        let want = if name.ends_with("_int") {
            "00000010"
        } else if name.ends_with("_bool") {
            "ffffffff"
        } else {
            "41800000"
        };
        let fau = fragment_fau(&name);
        assert!(fau.iter().any(|w| w == want), "{name}: FAU {fau:?} lacks the first member's value {want}");
    }
}
