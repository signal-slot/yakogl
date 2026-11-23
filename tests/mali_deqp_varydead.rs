// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! `deqp_varydead_*` references (`tools/mali/testdata/`): the ten
//! dEQP-GLES2 `shaders.random.*.fragment.*` pairs of issue #11309, drawn
//! with `tools/mali/deqp_ref.sh` (drm-shim) from the board run's sources.
//!
//! The fragment shader declares varyings whose loads end dead: a store to
//! `gl_FragColor` a later store overwrites, or a branch of a constant `if`
//! that is never taken. Mesa's pre-link passes remove them
//! (nir_opt_copy_prop_vars, nir_opt_dead_write_vars, nir_opt_dead_cf), so
//! its link removes those varyings from the vertex shader. The program's
//! vertex shader here kept them and the link refused the pair (`a Mali link
//! whose stages place a varying at different offsets`).
//!
//! Eight compile byte-equal. Two link and differ in the fragment code alone
//! ([`FS_DIFFERS`], other defects than the link): their tests are
//! `#[ignore]`d and `fs_differs_pinned` asserts that state.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varydead
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references whose pair links with Mesa's vertex words and whose
/// fragment words differ, without the `deqp_varydead_` prefix.
const FS_DIFFERS: &[&str] = &["basic_expression_fragment_85", "comparison_ops_fragment_41"];

/// Varyings `g`, `a`, `b`; `gl_FragColor = a` is the last of four stores.
/// Mesa stores `a` alone, at offset 0.
#[test]
fn basic_expression_fragment_53() {
    mali_ref::assert_matches("deqp_varydead_basic_expression_fragment_53");
}

#[test]
fn basic_expression_fragment_9() {
    mali_ref::assert_matches("deqp_varydead_basic_expression_fragment_9");
}

/// `c.zw + vec2(3.0, -5.5)` at fp16: Mesa adds one packed immediate
/// (`FADD_IMM.v2f16 #0xC5804200`), the back end two FAU words.
#[test]
#[ignore = "differs in fs.fragment only (a v2f16 add of two constants); pinned by fs_differs_pinned"]
fn basic_expression_fragment_85() {
    mali_ref::assert_matches("deqp_varydead_basic_expression_fragment_85");
}

#[test]
fn comparison_ops_fragment_17() {
    mali_ref::assert_matches("deqp_varydead_comparison_ops_fragment_17");
}

/// `int(j) - -7 - 6`: Mesa adds one constant (`IADD_IMM #0x1`), the back
/// end two (`#0x7`, `#0xFFFFFFFA`).
#[test]
#[ignore = "differs in fs.fragment only (two integer adds of constants); pinned by fs_differs_pinned"]
fn comparison_ops_fragment_41() {
    mali_ref::assert_matches("deqp_varydead_comparison_ops_fragment_41");
}

#[test]
fn comparison_ops_fragment_45() {
    mali_ref::assert_matches("deqp_varydead_comparison_ops_fragment_45");
}

/// The varyings read only in `if (bool(d.p))` with `const ivec4 d`, and
/// overwritten there: Mesa takes the branch and stores `c` alone.
#[test]
fn conditionals_fragment_47() {
    mali_ref::assert_matches("deqp_varydead_conditionals_fragment_47");
}

#[test]
fn scalar_conversion_fragment_17() {
    mali_ref::assert_matches("deqp_varydead_scalar_conversion_fragment_17");
}

#[test]
fn scalar_conversion_fragment_53() {
    mali_ref::assert_matches("deqp_varydead_scalar_conversion_fragment_53");
}

#[test]
fn swizzle_fragment_17() {
    mali_ref::assert_matches("deqp_varydead_swizzle_fragment_17");
}

/// Each of [`FS_DIFFERS`] links, and every difference from its reference
/// is in the fragment code: the vertex shader the link builds is Mesa's.
#[test]
fn fs_differs_pinned() {
    let mut changed = Vec::new();
    for name in FS_DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_varydead_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = mali_ref::differences(&r, &pair);
                if d.is_empty() {
                    changed.push(format!("{name}: now byte-equal; remove its #[ignore] and its entry"));
                }
                for line in d.iter().filter(|l| !l.starts_with("fs.fragment") && !l.starts_with("fs:")) {
                    changed.push(format!("{name}: differs outside the fragment code: {line}"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "the pinned states moved:\n{}", changed.join("\n"));
}
