// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_fsint2_*` reference (Mesa
//! 26.2.1, plan #402 item `ref-fsint2`) against the Mali online compiler: the
//! extra fragment-shader integer/boolean references design-fsint asked for
//! beside `deqp_fsint_*` (`tests/mali_deqp_fsint.rs`). Each pair is the one
//! dEQP-GLES2 (VK-GL-CTS 93bca01) writes, verbatim from the `.qpa` of a
//! HOST-ONLY `tools/mali/deqp-host-measure.sh` run at ab919063 (the slices of
//! `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`), drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610 drm-shim,
//! `--uniform-values`). `tests/mali_ref` does the compare.
//!
//! The ones the compiler makes byte-equal run ([`MATCHED`], none yet); the
//! rest are `#[ignore]`d with their refusal. `refusals_pinned` runs now: it
//! holds each refusal's exact text ([`REFUSED`], without the `Valhall: `
//! prefix and the ` is not lowered yet` suffix), so a lowering must move its
//! pin, and it fails when a `deqp_fsint2_*` reference on disk is in neither
//! list.
//!
//! HOST-ONLY shapes at ab919063 (board verdict: every case below refused "an
//! integer or boolean value" on the board at 8ff01055; `fs` the words of
//! Mesa's `fs.fragment` region):
//!
//! | reference | case | shape | fs | verdict at the tip |
//! |---|---|---|---|---|
//! | `rnd_cond9` | `random.conditionals.fragment.9` | dead `if`/`else` over int, ivec and bool globals and uniforms (Mesa drops every uniform; the colour is `vec4(s, 0.5, 1, r < q)`, `s` 0 and `q` -1.0) | 9 | refused: control flow (the host's first refusal is not the board's) |
//! | `rnd_trig93` | `random.trigonometric.fragment.93` | int and bool constants and globals into `gl_FragColor`, then `asin(vec4) * tan(vec4)` overwrites it | 48 | refused: arithmetic (the int and bool store is overwritten, mixprec2 M2-1) |
//! | `fn_bvec2_ne` | `functions.datatypes.bool_bvec2_fragment` | `bvec2(v_in0)` into a user function `!(a.x == a.y)`, compared with `uniform bool ref_out0` (`isOk`), the bool in three colour lanes | 14 | refused: an integer or boolean value |
//! | `step_lowp_float` | `operator.common_functions.step.lowp_float_fragment` | lowp `step(float, float)`, one lane, `* 0.5 + 0.25` | 12 | refused: an integer or boolean value |
//! | `step_lowp_vec2` | `operator.common_functions.step.lowp_vec2_fragment` | lowp `step(vec2, vec2)`, two lanes, `* 0.5 + 0.25` | 17 | refused: an integer or boolean value |
//!
//! Not drawn: `operator.binary_operator.logical_or.bool_fragment`, whose
//! pair is byte for byte `deqp_intval4_logical_or_fs` (pinned refused, "an
//! integer or boolean value", in `tests/mali_deqp_intval4.rs`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsint2
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The fragment refusal of an integer or boolean value.
const INT_VALUE: &str = "an integer or boolean value";
/// The fragment refusal of control flow, `rnd_cond9`'s first on the host.
/// `rnd_trig93` past its overwritten store (mixprec2 M2-1): `asin * tan`.
const ARITH: &str = "arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)";
const CONTROL_FLOW: &str = "control flow (if, a loop, a return before the end)";

/// The references the compiler makes byte-equal.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal, without the `Valhall: ` prefix
/// and the ` is not lowered yet` suffix.
const REFUSED: &[(&str, &str)] = &[
    ("rnd_cond9", CONTROL_FLOW),
    ("rnd_trig93", ARITH),
    ("fn_bvec2_ne", INT_VALUE),
    ("step_lowp_float", INT_VALUE),
    ("step_lowp_vec2", INT_VALUE),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_fsint2_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_fsint2_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| {
            mali_ref::refusal_change(&format!("deqp_fsint2_{name}"), &format!("Valhall: {text} is not lowered yet"))
        })
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

#[test]
#[ignore = "refused: control flow (if, a loop, a return before the end); on the board at 8ff01055 the first refusal was an integer or boolean value"]
fn rnd_cond9() {
    mali_ref::assert_matches("deqp_fsint2_rnd_cond9");
}

#[test]
#[ignore = "refused: an integer or boolean value, lowered by lower-fsint"]
fn rnd_trig93() {
    mali_ref::assert_matches("deqp_fsint2_rnd_trig93");
}

/// Drawn with `--uniform-values`: `ref_out0` is true.
#[test]
#[ignore = "refused: an integer or boolean value, lowered by lower-fsint"]
fn fn_bvec2_ne() {
    mali_ref::assert_matches("deqp_fsint2_fn_bvec2_ne");
}

#[test]
#[ignore = "refused: an integer or boolean value, lowered by lower-fsint"]
fn step_lowp_float() {
    mali_ref::assert_matches("deqp_fsint2_step_lowp_float");
}

#[test]
#[ignore = "refused: an integer or boolean value, lowered by lower-fsint"]
fn step_lowp_vec2() {
    mali_ref::assert_matches("deqp_fsint2_step_lowp_vec2");
}
