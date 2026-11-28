// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsconst2_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsconst2`,
//! `docs/notes/mali-deqp-vsconst2.md`) against the Mali online compiler. The
//! refusal is `MaliError::ConstantFold`, "a constant operation on or to a
//! value that is not a finite normal float". All of them are refused at the
//! tip: each is `#[ignore]`d, and `refusals_pinned` runs now and holds the
//! refusal's exact text, so a lowering must move its pin.
//!
//! HOST-ONLY shapes at 14e7a7421: `deqp-host-measure.sh` over the 259 board
//! cases of the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055` whose text is
//! this refusal; 19 still have it first (no vertex-shader case: the
//! unrolled loops of `shaders.indexing` now stop at the constant-product
//! refusal, `deqp_vsconst_*`'s pins).
//!
//! | Shape | Cases | Board | Stage, site, op | References |
//! |---|--:|---|---|---|
//! | `atan_zero`: `atan` of a zero lane at mediump (`atan(0.0)`, `atan(vec4(.., 0.0, ..))`); a 16-bit `frcp` of 0.0 reaches the fold (the cause inside Mesa's `atan` lowering is not traced) | 3 (`shaders.random.trigonometric.fragment.{3,10}`, `all_features.fragment.40`) | Fail | FS, `nir_fs.rs:1400` (`fold_rcp16`, a zero operand), `rcp(0)` | `rcp0_atan_zero`, `rcp0_atan_vec4_lanes` (minimal), `rcp0_atan_uniform`, `rcp0_asin_atan_const`, `rcp0_all_features` (the cases' programs) |
//! | `fs_log2`, `fs_atan`: `log2`, `log`, `pow`, `atan` of constants | 16 | Fail | FS, `nir.rs:2430` (`flog2`, `iand`) | covered: `deqp_cfold2_const_call_transcendental_{log2,pow,atan}` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsconst2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal of every reference here, without the `Valhall: ` prefix and
/// the ` is not lowered yet` suffix.
const NONFINITE: &str = "a constant operation on or to a value that is not a finite normal float";

/// The references the compiler makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("rcp0_atan_zero", NONFINITE),
    ("rcp0_atan_vec4_lanes", NONFINITE),
    ("rcp0_atan_uniform", NONFINITE),
    ("rcp0_asin_atan_const", NONFINITE),
    ("rcp0_all_features", NONFINITE),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsconst2_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsconst2_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsconst2_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// `atan(0.0)` alone in a mediump colour lane: a 16-bit `frcp` of 0.0 reaches
/// `fold_rcp16`, which refuses it (`atan(0.5)`, `atan(2.0)` and `asin` of
/// the same lanes do not reach it).
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the fp16 rcp(0) of an atan of 0.0, nir_fs.rs:1400)"]
fn rcp0_atan_zero() {
    mali_ref::assert_matches("deqp_vsconst2_rcp0_atan_zero");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the fp16 rcp(0) of an atan of 0.0, nir_fs.rs:1400)"]
fn rcp0_atan_vec4_lanes() {
    mali_ref::assert_matches("deqp_vsconst2_rcp0_atan_vec4_lanes");
}

/// `shaders.random.trigonometric.fragment.10`: the zero is a constant
/// `atan(float(false))` beside `atan(a.ragb)` of a uniform.
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the fp16 rcp(0) of an atan of 0.0, nir_fs.rs:1400)"]
fn rcp0_atan_uniform() {
    mali_ref::assert_matches("deqp_vsconst2_rcp0_atan_uniform");
}

/// `shaders.random.trigonometric.fragment.3`: `asin(atan(vec4(b, 0, a, 0)))`.
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the fp16 rcp(0) of an atan of 0.0, nir_fs.rs:1400)"]
fn rcp0_asin_atan_const() {
    mali_ref::assert_matches("deqp_vsconst2_rcp0_asin_atan_const");
}

/// `shaders.random.all_features.fragment.40`.
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the fp16 rcp(0) of an atan of 0.0, nir_fs.rs:1400)"]
fn rcp0_all_features() {
    mali_ref::assert_matches("deqp_vsconst2_rcp0_all_features");
}
