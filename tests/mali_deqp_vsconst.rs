// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_vsconst_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-vsconst`,
//! `docs/notes/mali-deqp-vsconst.md`) against the Mali online compiler. The
//! refusal is `MaliError::ConstantFold`, "a constant operation on or to a
//! value that is not a finite normal float". It grew from 114 board Fail at
//! 37305608 to 144 Fail and 73 NotSupported at 8ff01055. The ones a
//! lowering takes run ([`MATCHED`]); the rest are `#[ignore]`d with their
//! refusal. `refusals_pinned` runs now and holds each refusal's exact text
//! (the VS one when the VS refuses, since it is compiled first; otherwise the
//! FS one), so a lowering must move its pin.
//!
//! HOST-ONLY shapes at e65567ea: `deqp-host-measure.sh` over the 259 board
//! cases of the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055`, 218 refused
//! with this text first, each put at one refusal site by a probe.
//!
//! | Shape | Cases | Board Fail / NS | Stage, site, op | References |
//! |---|--:|---|---|---|
//! | `dynloop`: `shaders.indexing` loop to a uniform bound (`i < ui_four`), write and read | 33 | 0 / 33 | VS, `nir.rs:2412`, `inot(0)` | `dynloop_tmp_array`, `dynloop_varying_array`, `dynloop_matrix_subscript` (here) |
//! | `mixloop`: one loop to a uniform bound, the other static | 25 | 0 / 25 | VS, `nir.rs:2412`, `inot(0)` | `mixloop_tmp_array`, `mixloop_varying_array`, `mixloop_vector_subscript` (here) |
//! | `fsdiv_highp`: `matrix.div.const_highp_matN_matN_fragment` | 3 | 3 / 0 | FS, `nir_fs.rs:6999`, `rcp(0.7)`, `rcp(1.2)` | `fsdiv_highp_mat2`, `fsdiv_highp_mat3` (here) |
//! | `staticloop`: `shaders.indexing` static loops, unrolled | 68 | 54 / 14 | VS, `nir.rs:2412` `inot(0)` 52, `nir.rs:2325` `ieq` 16 | covered: `deqp_cfold2_zero_operand_icmp`, `_zero_operand_inot`, `_const_other_inot` |
//! | `random_int`: int and bool constants in `shaders.random`, `interaction.basic_shader` | 35 | 35 / 0 | VS, `nir.rs:2325` `ieq` 26, `inot(~0)` 2; `nir.rs:2412` `inot(0)` 7 | covered: `deqp_cfold2_random_mixed_icmp`, `_random_mixed_inot`, `deqp_constrnd_int_bits_vs` |
//! | `light_amount`: the unrolled light loop | 16 | 16 / 0 | VS, `nir.rs:2325` `ieq` 14, `nir.rs:2412` `inot(0)` 2 | covered: `deqp_intval5_light_amount_2_2`, `_0_16` |
//! | `constexpr_int`: `constant_expressions` int and bool | 9 | 9 / 0 | VS, `nir.rs:2325` `ieq`, `inot(~0)`, `iabs(9)`; `nir.rs:2412` `inot(0)` | covered: `deqp_cfold2_const_call_other_bvec`, `_ivec`, `_const_other_iabs` |
//! | `struct_loop`: `struct.uniform.loop_struct_array_*` | 2 | 1 / 1 | VS, `nir.rs:2325`, `ieq` | covered: `deqp_cfold2_zero_operand_icmp` (the same unrolled counter) |
//! | `fs_log2`: `log2`, `log`, `pow` of constants | 12 | 12 / 0 | FS, `nir.rs:2412`, `flog2` | covered: `deqp_cfold2_const_call_transcendental_log2`, `_pow` |
//! | `fs_atan`: `atan` of constants | 4 | 4 / 0 | FS, `nir.rs:2412`, `iand` | covered: `deqp_cfold2_const_call_transcendental_atan` |
//! | `fs_rcp16`: mediump and lowp `matrix.div.const_*`, `shaders.random` fragment | 11 | 11 / 0 | FS, `nir_fs.rs:6999`, `rcp` not exact in fp16 | covered: `deqp_cfold2_zero_operand_rcp16`, `_const_other_rcp16`, `_random_mixed_rcp16` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsconst
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The refusal, without the `Valhall: ` prefix and the ` is not lowered yet`
/// suffix.
const CONSTANT_FOLD: &str = "a constant operation on or to a value that is not a finite normal float";

/// The references the compiler makes byte-equal. None yet.
const MATCHED: &[&str] = &[];

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("dynloop_tmp_array", CONSTANT_FOLD),
    ("dynloop_varying_array", CONSTANT_FOLD),
    ("dynloop_matrix_subscript", CONSTANT_FOLD),
    ("mixloop_tmp_array", CONSTANT_FOLD),
    ("mixloop_varying_array", CONSTANT_FOLD),
    ("mixloop_vector_subscript", CONSTANT_FOLD),
    ("fsdiv_highp_mat2", CONSTANT_FOLD),
    ("fsdiv_highp_mat3", CONSTANT_FOLD),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_vsconst_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_vsconst_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_vsconst_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// Mesa partially unrolls the loop by the trip count it guesses from
/// `arr[4]` (`nir_opt_loop_unroll.c:1079-1087`), one `ICMP_OR.s32.lt`
/// against `ui_four` (FAU u5.w1) and one `MUX.v2i16` per element; the
/// unwritten element is `nir_opt_undef`'s NaN, fp16 `0x7e00` in u6.w1.
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the inot of a constant false in the loop, nir.rs:2412)"]
fn dynloop_tmp_array() {
    mali_ref::assert_matches("deqp_vsconst_dynloop_tmp_array");
}

/// Both loops guarded and unrolled: the VS stores four varyings, the FS
/// sums them under four `ICMP_OR.s32.lt` of `ui_four` (FAU u2.w0) with fp16
/// NaN `0x7e00` for the unwritten one.
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the inot of a constant false in the loop, nir.rs:2412)"]
fn dynloop_varying_array() {
    mali_ref::assert_matches("deqp_vsconst_dynloop_varying_array");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the inot of a constant false in the loop, nir.rs:2412)"]
fn dynloop_matrix_subscript() {
    mali_ref::assert_matches("deqp_vsconst_dynloop_matrix_subscript");
}

/// A guarded write read by a static loop: no compare is left, the undef of
/// an unwritten element folds away (the varying is `x + x/2 + x/4 + x/8` in
/// fp16, `0x3000` = 0.125 in FAU).
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the inot of a constant false in the loop, nir.rs:2412)"]
fn mixloop_tmp_array() {
    mali_ref::assert_matches("deqp_vsconst_mixloop_tmp_array");
}

#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the inot of a constant false in the loop, nir.rs:2412)"]
fn mixloop_varying_array() {
    mali_ref::assert_matches("deqp_vsconst_mixloop_varying_array");
}

/// The one reference Mesa keeps a loop in: `tmp[i]` of a `vec2` is no array
/// access to guess a trip count from, so the read loop to `ui_two` stays
/// (`BRANCHZ.reconverge ... offset:-7`).
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (the inot of a constant false in the loop, nir.rs:2412)"]
fn mixloop_vector_subscript() {
    mali_ref::assert_matches("deqp_vsconst_mixloop_vector_subscript");
}

/// A constant colour: FAU u2 = `4100bccd 3c003800`. The highp quotient
/// folds at fp32 and narrows to the mediump colour: lane 2 is `0x3800`
/// (0.5), where the mediump case's fp16 fold gives `0x37fe`
/// (`deqp_cfold2_const_other_rcp16`).
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (rcp(0.7) not exact in fp16, nir_fs.rs:6999)"]
fn fsdiv_highp_mat2() {
    mali_ref::assert_matches("deqp_vsconst_fsdiv_highp_mat2");
}

/// FAU u2 = `b639bcab 3c003db7` (the mediump case's are `b638bcab
/// 3c003db6`).
#[test]
#[ignore = "refused: a constant operation on or to a value that is not a finite normal float (rcp(1.2) not exact in fp16, nir_fs.rs:6999)"]
fn fsdiv_highp_mat3() {
    mali_ref::assert_matches("deqp_vsconst_fsdiv_highp_mat3");
}
