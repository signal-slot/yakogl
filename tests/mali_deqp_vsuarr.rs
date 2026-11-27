// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsuarr_*` reference (Mesa
//! 26.2.1 on the G610 drm-shim, plan #387 item `vsunifarr-design`,
//! `docs/notes/mali-vsuarr-lowering.md`) against the Mali online compiler.
//! The refusal is `MaliError::UniformArray`, "a uniform array is not lowered
//! yet", in the vertex shader: 193 lines in the Verdin iMX95 (Mali-G310)
//! dEQP-GLES2 run of 2026-11-25 (`tools/cts/log/2026-11-24-imx95-gles2-S1b-e63ab28e`),
//! the same 193 `uniform_api` cases as the NanoPC-T6 run at 8ff01055.
//!
//! These are the residue the earlier references leave: the
//! `value.initial.*` cases (36 of the 193), whose vertex shaders compare
//! every array element with zero. Mesa folds those compares into one
//! reduction (`ior` of bool words, `umax` of int words, `fmax` of the `abs`
//! of fp16 lanes) and one compare at the end, a different word sequence from
//! the assigned cases' `deqp_vsuniarr_*` / `deqp_unif2_arr_*` references.
//!
//! | Reference | dEQP-GLES2.functional.uniform_api.value.initial. | Cases | Array |
//! |---|---|--:|---|
//! | `init_bool` | `{render,get_uniform}.basic_array.bool*` | 6 | `bool u_var[3]` |
//! | `init_int` | `{render,get_uniform}.basic_array.int_*` | 4 | `mediump int u_var[3]` |
//! | `init_bvec4` | `{render,get_uniform}.basic_array.bvec4*` | 6 | `bvec4 u_var[3]` |
//! | `init_ivec4` | `{render,get_uniform}.basic_array.ivec4_*` | 4 | `mediump ivec4 u_var[3]` |
//! | `init_mat4` | `{render,get_uniform}.basic_array.mat4_*` | 4 | `mediump mat4 u_var[3]` (fp16) |
//! | `init_multi` | `{render,get_uniform}.multiple_basic_array.*` | 4 | float, ivec3 and bvec2 arrays |
//! | `init_struct_mat` | `{render,get_uniform}.array_in_struct.mat4_mat2_*` | 4 | a struct's `mat4 m2[3]`, `mat2 m3[3]` (fp32) |
//! | `init_nested_mat` | `{render,get_uniform}.nested_structs_arrays.mat4_mat2_*` | 4 | a nested struct's mat4 array and struct array |
//!
//! Every byte-compare is `#[ignore]`d while the compiler refuses it;
//! `references_load` and `refusals_pinned` run now. A lowering that takes a
//! reference removes its `#[ignore]` and its pin and adds it to [`MATCHED`].
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsuarr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix.
const UNIFORM_ARRAY: &str = "a uniform array is not lowered yet";

/// The references the compiler makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("init_bool", UNIFORM_ARRAY),
    ("init_int", UNIFORM_ARRAY),
    ("init_bvec4", UNIFORM_ARRAY),
    ("init_ivec4", UNIFORM_ARRAY),
    ("init_mat4", UNIFORM_ARRAY),
    ("init_multi", UNIFORM_ARRAY),
    ("init_struct_mat", UNIFORM_ARRAY),
    ("init_nested_mat", UNIFORM_ARRAY),
];

fn on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsuarr_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    v.sort();
    v
}

/// Every reference on disk is listed, and loads: both shaders, the
/// `.expect.json` with the three regions of a vertex-and-fragment draw, and
/// a `.license` beside each shader.
#[test]
fn references_load() {
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk(), listed, "every deqp_vsuarr_* reference has a test and a pin");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    for name in &listed {
        let r = mali_ref::Reference::load(&format!("deqp_vsuarr_{name}"));
        assert!(r.vs.contains("uniform ") && r.vs.contains("v_vtxOut"), "{name}: not a uniform_api vertex shader");
        assert!(r.fs.contains("gl_FragColor"), "{name}: not a fragment shader");
        for region in ["\"vs.position\"", "\"vs.varying\"", "\"fs.fragment\""] {
            assert!(r.json.contains(region), "{name}: no {region} region");
        }
        for ext in ["vert.license", "frag.license"] {
            assert!(dir.join(format!("deqp_vsuarr_{name}.{ext}")).is_file(), "{name}: no .{ext}");
        }
    }
}

/// Each refused reference is still refused with its pinned text (the vertex
/// shader's: it is compiled first, and every fragment shader here is the
/// varying alone).
#[test]
fn refusals_pinned() {
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsuarr_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// `bool u_var[3]`, every element `== false`: Mesa ORs the three words
/// (`MOV.i32`, two `LSHIFT_OR.i32`) and selects 1.0 where the OR is 0
/// (`CSEL.u32.eq r0, r0^, 0x0, 0x3F800000, 0x0`). FAU 11 to 13.
#[test]
#[ignore = "refused: a uniform array (uniform bool u_var[3])"]
fn init_bool() {
    mali_ref::assert_matches("deqp_vsuarr_init_bool");
}

/// `mediump int u_var[3]`, every element `== 0`: an unsigned max of the
/// three words (`CSEL.u32.gt` twice), one `ICMP_OR.u32.eq.m1 r0, r0^, 0x0,
/// 0x0`, then the 1.0 / 0 select.
#[test]
#[ignore = "refused: a uniform array (uniform mediump int u_var[3])"]
fn init_int() {
    mali_ref::assert_matches("deqp_vsuarr_init_int");
}

/// `bvec4 u_var[3]`, every lane `== false`: one `LSHIFT_OR.i32` tree over
/// the twelve words (FAU 11 to 22), its leaves regrouped across elements,
/// one `CSEL.u32.eq` at the end.
#[test]
#[ignore = "refused: a uniform array (uniform bvec4 u_var[3])"]
fn init_bvec4() {
    mali_ref::assert_matches("deqp_vsuarr_init_bvec4");
}

/// `mediump ivec4 u_var[3]`, every lane `== 0`: one `CSEL.u32.gt` (umax)
/// tree over the twelve words, one `ICMP_OR.u32.eq.m1` with 0.
#[test]
#[ignore = "refused: a uniform array (uniform mediump ivec4 u_var[3])"]
fn init_ivec4() {
    mali_ref::assert_matches("deqp_vsuarr_init_ivec4");
}

/// `mediump mat4 u_var[3]` against zero matrices: fp16, 24 words (FAU 11
/// to 34); no `FADD_IMM`: an `FMAX.v2f16` tree of the 48 lanes' `abs`, one
/// `FCMP_OR.v2f16.lt.m1` with 0.05 (`2a662a66`, FAU 35), `S16_TO_S32`.
#[test]
#[ignore = "refused: a uniform array (uniform mediump mat4 u_var[3])"]
fn init_mat4() {
    mali_ref::assert_matches("deqp_vsuarr_init_mat4");
}

/// The float, ivec3 and bvec2 arrays against zero: the fp16 `abs`/`FMAX`
/// of the three floats, the `umax` trees of the ivec3 lanes, the `ior` of
/// the bvec2 words with a `.not`, joined by `LSHIFT_AND.i32`. FAU 11 to 28
/// in the order of first use.
#[test]
#[ignore = "refused: a uniform array (the ivec3 and bvec2 arrays beside a float array)"]
fn init_multi() {
    mali_ref::assert_matches("deqp_vsuarr_init_multi");
}

/// A struct's mat4/mat2 members and member arrays against zero: 80 fp32
/// words (FAU 11 to 90), each pair `MOV.i32` + `V2F32_TO_V2F16`, then the
/// zero-compare `FMAX.v2f16` tree as `init_mat4`.
#[test]
#[ignore = "refused: a uniform array (a struct's mat4 and mat2 array members)"]
fn init_struct_mat() {
    mali_ref::assert_matches("deqp_vsuarr_init_struct_mat");
}

/// The nested struct's mat4 array and struct array against zero: 96 fp32
/// words (FAU 11 to 106), narrowed and reduced as `init_struct_mat`.
#[test]
#[ignore = "refused: a uniform array (a nested struct's mat4 array and struct array)"]
fn init_nested_mat() {
    mali_ref::assert_matches("deqp_vsuarr_init_nested_mat");
}
