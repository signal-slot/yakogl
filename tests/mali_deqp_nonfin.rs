// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex-shader non-finite constant refusal of the Verdin iMX95
//! (Mali-G310) dEQP-GLES2 run of 2026-11-25 (`MaliError::ConstantFold`, "a
//! constant operation on or to a value that is not a finite normal float";
//! 199 vertex lines in the consoles of `tools/cts/log/2026-11-2*-imx95-*`).
//! Plan #387, item `nonfinite-design`; the design is
//! `docs/notes/mali-nonfin-lowering.md`. Every
//! `tools/mali/testdata/deqp_nonfin_*` reference is Mesa 26.2.1 on drm-shim
//! (G610; `tools/mali/g310_check.sh` gives the same words on the G310v2),
//! drawn from a case whose Mesa program keeps a real non-finite constant:
//! `nir_opt_undef`'s fp16 NaN `0x7e00` for an array element the dynamic
//! read loop reaches and no write set, FAU word `7e007e00` read by a
//! `MUX.v2i16`.
//!
//! HOST-ONLY at 8133663c, the 199 cases refuse at an integer or boolean
//! constant op (`inot`, `ieq`, `iabs`), not at a real infinity or NaN. At
//! the train's result commit 224cc3f7 (ConstantFold step A) none of them
//! refuses with this text first; these seven then stop at the
//! constant-product text. So [`refusals_pinned`] accepts either text and
//! stays green in either merge order. It fails when a pair compiles (remove
//! its `#[ignore]`) or moves to a third text. [`references_load`] checks each
//! reference's regions and its NaN word.
//!
//! | Reference | dEQP case (`shaders.indexing.`) | Mesa: where `7e007e00` is read |
//! |---|---|---|
//! | `tmp_float_static_dynread` | `tmp_array.float_static_loop_write_dynamic_loop_read_vertex` | VS u6.w1, 1 `MUX.v2i16` |
//! | `tmp_vec3_static_dynread` | `tmp_array.vec3_static_loop_write_dynamic_loop_read_vertex` | VS u6.w1 |
//! | `tmp_vec4_dyn` | `tmp_array.vec4_dynamic_loop_write_dynamic_loop_read_vertex` | VS u6.w1, 2 `MUX.v2i16` |
//! | `matrix_mat3_dyn` | `matrix_subscript.mat3_dynamic_loop_write_dynamic_loop_read_vertex` | VS u6.w0, 2 `MUX.v2i16` |
//! | `matrix_mat4_static_dynread` | `matrix_subscript.mat4_static_loop_write_dynamic_loop_read_vertex` | VS u6.w1 |
//! | `varying_float_static_dynread` | `varying_array.float_static_loop_write_dynamic_loop_read` | FS u2.w1, 1 `MUX.v2i16` |
//! | `varying_vec4_dyn` | `varying_array.vec4_dynamic_loop_write_dynamic_loop_read` | FS u2.w1, 2 `MUX.v2i16` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_nonfin
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal at 8133663c, without the `Valhall: ` prefix and the ` is not
/// lowered yet` suffix.
const CONSTANT_FOLD: &str = "a constant operation on or to a value that is not a finite normal float";

/// The refusal at the train's 224cc3f7, once ConstantFold step A folds the
/// loop counter's `inot` and compares (`MaliError::ArithConstant`).
const ARITH_CONSTANT: &str = "a constant product in a shape Mesa's algebraic rules move or fold";

/// Mesa's fp16 NaN in both halves of one FAU word (`nir_opt_undef`).
const NAN16_PAIR: &str = "7e007e00";

/// The references the compiler makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference, the stage whose FAU holds the NaN word.
const REFUSED: &[(&str, &str)] = &[
    ("tmp_float_static_dynread", "vs"),
    ("tmp_vec3_static_dynread", "vs"),
    ("tmp_vec4_dyn", "vs"),
    ("matrix_mat3_dyn", "vs"),
    ("matrix_mat4_static_dynread", "vs"),
    ("varying_float_static_dynread", "fs"),
    ("varying_vec4_dyn", "fs"),
];

fn names_on_disk() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_nonfin_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    on_disk
}

/// The FAU words of one region of an expect.json, found by text: the
/// region's `"fau": [...]` list after its name.
fn region_fau(json: &str, region: &str) -> Option<Vec<String>> {
    let at = json.find(&format!("\"{region}\""))?;
    let rest = &json[at..];
    let f = rest.find("\"fau\"")?;
    let open = f + rest[f..].find('[')?;
    let close = open + rest[open..].find(']')?;
    Some(rest[open + 1..close].split(',').map(|w| w.trim().trim_matches('"').to_string()).collect())
}

#[test]
fn references_load() {
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(names_on_disk(), listed, "every deqp_nonfin_* reference has a test and a pin");
    for (name, stage) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_nonfin_{name}"));
        assert!(r.vs.contains("void main()") && r.fs.contains("void main()"), "{name}: shader sources");
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            assert!(r.json.contains(&format!("\"{region}\"")), "{name}: no {region} region");
        }
        let region = if *stage == "vs" { "vs.varying" } else { "fs.fragment" };
        let fau = region_fau(&r.json, region).unwrap_or_else(|| panic!("{name}: no FAU list in {region}"));
        assert!(fau.iter().any(|w| w == NAN16_PAIR), "{name}: {region} FAU has no {NAN16_PAIR}: {fau:?}");
        assert!(r.json.contains("MUX.v2i16"), "{name}: no MUX.v2i16 selects the NaN");
    }
}

#[test]
fn refusals_pinned() {
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, _)| {
            let name = format!("deqp_nonfin_{name}");
            let tip = mali_ref::refusal_change(&name, CONSTANT_FOLD)?;
            let train = mali_ref::refusal_change(&name, ARITH_CONSTANT)?;
            Some(if tip.contains("now compiles") { tip } else { format!("{tip} / {train}") })
        })
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// Mesa unrolls the static write loop and the dynamic read loop by the trip
/// count it guesses from `arr[4]`: one `ICMP_OR.s32.lt` against `ui_four`
/// and one `MUX.v2i16` per element. The element past the written ones is
/// the undef that `nir_opt_undef` makes fp16 NaN, FAU u6.w1 = `7e007e00`,
/// beside `30003000` (0.125 twice) of `coords * 0.5` folded.
#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn tmp_float_static_dynread() {
    mali_ref::assert_matches("deqp_nonfin_tmp_float_static_dynread");
}

#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn tmp_vec3_static_dynread() {
    mali_ref::assert_matches("deqp_nonfin_tmp_vec3_static_dynread");
}

/// vec4 elements: the NaN pair is read by two `MUX.v2i16` (lanes xy, zw).
#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn tmp_vec4_dyn() {
    mali_ref::assert_matches("deqp_nonfin_tmp_vec4_dyn");
}

/// A mat3 column read at a dynamic index: FAU u6.w0 = `7e007e00`, u6.w1 0.
#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn matrix_mat3_dyn() {
    mali_ref::assert_matches("deqp_nonfin_matrix_mat3_dyn");
}

#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn matrix_mat4_static_dynread() {
    mali_ref::assert_matches("deqp_nonfin_matrix_mat4_static_dynread");
}

/// The NaN is the fragment shader's: the VS writes the varyings, the FS
/// sums them under `ICMP_OR.s32.lt` of `ui_four` (FAU u2.w0 = 16) with fp16
/// NaN u2.w1 = `7e007e00` for the unwritten one.
#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn varying_float_static_dynread() {
    mali_ref::assert_matches("deqp_nonfin_varying_float_static_dynread");
}

#[test]
#[ignore = "refused: the loop counter's inot (8133663c) or a constant product (224cc3f7); then the fp16 NaN FAU word"]
fn varying_vec4_dyn() {
    mali_ref::assert_matches("deqp_nonfin_varying_vec4_dyn");
}
