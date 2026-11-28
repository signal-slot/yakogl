// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of every `tools/mali/testdata/deqp_fsunif4_*` reference (Mesa
//! 26.2.1 on drm-shim, plan #402 item `ref-fsunif4`,
//! `docs/notes/mali-deqp-fsunif4.md`) against the Mali online compiler.
//! These are the three mediump fragment-shader uniform shapes (`MaliError::
//! FsUniform`) that triage-fsunif-tip found without a reference: S3, S5 and
//! S9. All three refuse at the same arm of `vectorise`
//! (`src/shaderc/online/mali/nir_fs.rs:11167` at c28470db, `(Some(_), ..) |
//! (_, Some(_), ..)`): a pair of 16-bit lanes where one is a mediump uniform's
//! half and no earlier arm takes the pair. None is taken by a lowering yet:
//! every reference is `#[ignore]`d and `refusals_pinned` runs now and holds
//! each one's exact refusal text (the fragment shader's: each vertex shader
//! compiles), so a lowering must move its pin.
//!
//! HOST-ONLY at c28470db: `deqp-host-measure.sh --cases` over the 272 cases
//! of `refusals.tsv` of the slices `2026-11-24-t6-gles2-S{1..4}-8ff01055`
//! with this refusal's board text (VK-GL-CTS 93bca01 with the yakogl
//! target), plus `random.scalar_conversion.combined.94` (board: the VS
//! uniform-type refusal). 88 refuse with this text first on the host; each
//! program went through `mali_ref::compile_pair` with the `nir_fs.rs:11167`
//! arm tagged with the pair's place and kinds (a scratch copy). The shapes
//! here, by (place, first lane, second lane):
//!
//! | Shape | Pair at `nir_fs.rs:11167` | Cases | References |
//! |---|---|--:|---|
//! | S3 | operand: a constant, then a uniform half (`vec4(c, b, ..) * ..`) | 21 | `s3_conv_fs_4`, `s3_expr_fs_20`, `s3_interaction_30` |
//! | S5 | operand: a varying's or computed lane, then a uniform half (`vec4(v, d, ..) * ..`) | 3 | `s5_expr_fs_97`, `s5_expr_fs_74`, `s5_conv_fs_63` |
//! | S9 | colour x,y: a constant, then a uniform half (`vec4(1.0, a, b, a)`) | 2 | `s9_conv_fs_94`, `s9_conv_combined_94` |
//!
//! The other host cases of this refusal at the same arm are S2 (operand,
//! uniform half then constant: 18, `deqp_intval3_rsg_swizzle12_fs`,
//! `deqp_unifmp_rsg_expr10`) and S4 (operand, uniform half then lane: 5,
//! `deqp_unif2_fs_other`); the rest refuse at other lines (`mat16`,
//! `mat32`, `struct32`, ...), covered by `deqp_fsunif_*` and
//! `deqp_unifmp_*`. S9 at colour z,w compiles (the arm at
//! `nir_fs.rs:11145`). All eight were drawn with `tools/mali/deqp_ref.sh`
//! (shim, `--uniform-values`); `s9_conv_combined_94` has the same fragment
//! shader as `s9_conv_fs_94` beside a random vertex shader with an `int`
//! uniform.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsunif4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references the compiler makes byte-equal: none yet.
const MATCHED: &[&str] = &[];

/// The refusal of all eight, without the `Valhall: ` prefix and the ` is
/// not lowered yet` suffix.
const FS_UNIFORM: &str = "a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp";

/// Every refused reference with its refusal.
const REFUSED: &[(&str, &str)] = &[
    ("s3_conv_fs_4", FS_UNIFORM),
    ("s3_expr_fs_20", FS_UNIFORM),
    ("s3_interaction_30", FS_UNIFORM),
    ("s5_expr_fs_97", FS_UNIFORM),
    ("s5_expr_fs_74", FS_UNIFORM),
    ("s5_conv_fs_63", FS_UNIFORM),
    ("s9_conv_fs_94", FS_UNIFORM),
    ("s9_conv_combined_94", FS_UNIFORM),
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let f = e.unwrap().file_name().into_string().unwrap();
            f.strip_prefix("deqp_fsunif4_")?.strip_suffix(".expect.json").map(str::to_string)
        })
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> =
        REFUSED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).map(str::to_string).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "every deqp_fsunif4_* reference has a test and a pin");
    let changed: Vec<String> = REFUSED
        .iter()
        .filter_map(|(name, text)| mali_ref::refusal_change(&format!("deqp_fsunif4_{name}"), text))
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// S3, `shaders.random.scalar_conversion.fragment.4`: `vec4(c, b, 0.375,
/// 5.0) * vec4(..)` added to the colour, `b` a mediump float uniform, `c` a
/// constant (and a mediump `int` uniform `q` in an earlier colour).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S3: a constant, then a uniform half, as an operand)"]
fn s3_conv_fs_4() {
    mali_ref::assert_matches("deqp_fsunif4_s3_conv_fs_4");
}

/// S3, `shaders.random.basic_expression.fragment.20`: `c + vec4(1.5, b, b,
/// a)`, `c` and `a` globals that fold to constants.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S3: a constant, then a uniform half, as an operand)"]
fn s3_expr_fs_20() {
    mali_ref::assert_matches("deqp_fsunif4_s3_expr_fs_20");
}

/// S3, `fragment_ops.interaction.basic_shader.30`: `vec4(e, d, c, b) -
/// vec4(a, 7.5, -8.0, -7)`, `e` the constant 0, `d` the uniform, `c` and
/// `a` varyings.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S3: a constant, then a uniform half, as an operand)"]
fn s3_interaction_30() {
    mali_ref::assert_matches("deqp_fsunif4_s3_interaction_30");
}

/// S5, `shaders.random.basic_expression.fragment.97`: `vec4(b, d, e, d) *
/// vec4(b, c, b, a)`, `b` a varying, `d` the uniform.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S5: a lane, then a uniform half, as an operand)"]
fn s5_expr_fs_97() {
    mali_ref::assert_matches("deqp_fsunif4_s5_expr_fs_97");
}

/// S5, `shaders.random.basic_expression.fragment.74`: `e * d * vec4(c,
/// -1.0, b, a)` after two dead colours, with float and `vec4` uniforms; the
/// pair is a computed lane beside a uniform half.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S5: a lane, then a uniform half, as an operand)"]
fn s5_expr_fs_74() {
    mali_ref::assert_matches("deqp_fsunif4_s5_expr_fs_74");
}

/// S5, `shaders.random.scalar_conversion.fragment.63`: `vec4(g, f, e, 0.0) *
/// vec4(d, c, b, a)`, `g` a varying, `f` the uniform (and an unread
/// mediump `int` uniform).
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S5: a lane, then a uniform half, as an operand)"]
fn s5_conv_fs_63() {
    mali_ref::assert_matches("deqp_fsunif4_s5_conv_fs_63");
}

/// S9, `shaders.random.scalar_conversion.fragment.94`: `gl_FragColor =
/// vec4(bool(c), a, b, a)`, the constant 0.0 and the uniform `a` as the
/// colour's first word.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S9: a constant, then a uniform half, as the colour's first word)"]
fn s9_conv_fs_94() {
    mali_ref::assert_matches("deqp_fsunif4_s9_conv_fs_94");
}

/// S9, `shaders.random.scalar_conversion.combined.94`: the same fragment
/// shader as `s9_conv_fs_94` beside a random vertex shader (mediump float
/// and `int` uniforms, two float attributes) that writes only
/// `gl_Position`.
#[test]
#[ignore = "refused: a mediump fragment shader uniform other than ... (S9: a constant, then a uniform half, as the colour's first word)"]
fn s9_conv_combined_94() {
    mali_ref::assert_matches("deqp_fsunif4_s9_conv_combined_94");
}
