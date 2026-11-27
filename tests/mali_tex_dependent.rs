// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! TEX_DEPENDENT of plan #367 on the Mali (lower-train-2, from
//! proto-tex-dependent): a fragment look-up whose coordinate is computed
//! from another look-up's texel (glmark2 refract fs 3fae's chain of three),
//! against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_nbshrf_8` and `_9`, byte for byte
//! (docs/notes/mali-tex-dependent.md).
//!
//! Such a shader takes TEX_MULTI's route (`nir_fs::multi16`,
//! docs/notes/mali-tex-multi.md). What TEX_DEPENDENT adds: the texel a
//! coordinate reads is its look-up's word (`r2^.h00` of `FMA.v2f16 r0, r1^,
//! r2^.h00, r0^`); the coordinates are fp16 pair sums and products over the
//! converted varyings, constants and those words; a 2D texel's one lane
//! read once by a later coordinate is grafted into its reader as GLSL's
//! `opt_tree_grafting` does, and the nodes stand in glsl_to_nir's walk of
//! the grafted tree (`nir_fs::tree_order`), so a varying read before the
//! look-up in the expression is loaded and converted before the look-up's
//! coordinate.
//!
//! Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region. The references
//! read no uniform but their samplers (drawn with `--uniform-values`, which
//! sets none here).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_tex_dependent
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// Two look-ups, the second at `(vp.xyz + vn * d).st * 0.5 + 0.5` with `d`
/// the first's red: `vp` and `vn` loaded and converted first (the grafted
/// `d` is visited after `vn`), the first coordinate `FMA.v2f16 r2, r0,
/// 0.5.h11, 0.5.h11`, its look-up writing the red alone (`.r`, no
/// `.skip`: helpers need it for the next coordinate), then `FMA.v2f16 r0,
/// r1^, r2^.h00, r0^`.
#[test]
fn gm2_nbshrf_8_one_dependent_look_up_matches_its_reference() {
    mali_ref::assert_matches("gm2_nbshrf_8");
}

/// Three chained look-ups (refract fs 3fae's chain with its refractions
/// replaced by products): `e = vp.xyz + vn * 1.5` (`FMA.v2f16 r1, r1^,
/// u2.w0, r0`, 1.5 a pushed pair), the second look-up `.rg` for the
/// `n.st` the third coordinate reads (`FMA.v2f16 r0, r1^, u3.w1, r0^`).
#[test]
fn gm2_nbshrf_9_three_chained_look_ups_match_their_reference() {
    mali_ref::assert_matches("gm2_nbshrf_9");
}

const VS: &str = "attribute vec4 a_pos; attribute vec3 a_n; varying vec3 vn; varying vec4 vp; \
                  void main() { gl_Position = a_pos; vp = a_pos * 0.5; vn = a_n; }";

/// Dependent coordinates of forms no reference draws stay refused: a
/// uniform in the coordinate, an operation other than a sum or a product
/// on its way (`abs`), a dependent look-up under a discard.
#[test]
fn forms_without_a_reference_stay_refused() {
    let head = "precision mediump float; uniform sampler2D m0; uniform sampler2D m1; uniform vec2 u; \
                varying vec3 vn; varying vec4 vp; void main() { float d = texture2D(m0, vp.xy * 0.5 + 0.5).x;";
    let bodies = [
        "gl_FragColor = texture2D(m1, vp.xy + u * d); }",
        "gl_FragColor = texture2D(m1, abs(vp.xy + vn.xy * d)); }",
        "if (d < 0.25) discard; gl_FragColor = texture2D(m1, vp.xy + vn.xy * d); }",
    ];
    for b in bodies {
        let fs = format!("{head} {b}");
        match mali_ref::compile_pair(VS, &fs) {
            Ok(_) => panic!("`{b}` compiles, but no reference draws it"),
            Err(e) => assert!(e.contains("Valhall:"), "`{b}`: {e}"),
        }
    }
}
