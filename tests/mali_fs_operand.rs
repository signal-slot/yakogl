// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 jellyfish fs 8ee5's gradient on the Mali (plan #367
//! lower-nonfinite-const, step fsoperand: a uniform first in `mix`), against
//! the Mesa 26.2.1 drm-shim references `tools/mali/testdata/gm2_fsop_*`,
//! byte for byte (docs/notes/mali-fs-operand.md):
//!
//! - `mix(color1, color2, t)` of two vec3 uniforms is `ffma(t, color2 -
//!   color1, color1)`: the difference `FADD.v2f16 r1, u2.w1, u2.w0.neg` has
//!   a uniform first in a sum and a uniform under the negation, two operand
//!   forms (`Uniform` at `AddA` and at `NegSrc`) no earlier reference drew.
//!
//! gm2_fsop_1 (the product `uv.x * uv.y` as the weight) compiles with its
//! instructions in another order than Mesa's: both differences before both
//! multiply-adds there, each pair's difference and multiply-add together
//! here (the pair graph orders nodes of one key by lane pair). gm2_fsop_5
//! (`color1 + vec3(uv.x * uv.y)`) needs the late fusion of a product that
//! two lane pairs read and stays refused.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_fs_operand
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `vec4(mix(color1, color2, uv.x), 1.0)`: the two differences, then the
/// two multiply-adds by the varying's narrowed lane.
#[test]
fn gm2_fsop_4_a_mix_of_two_uniforms_matches_its_reference() {
    mali_ref::assert_matches("gm2_fsop_4");
}

/// The other gradient shapes the study measured, byte-equal already: they
/// guard the forms this step admits.
#[test]
fn gm2_fsop_2_3_6_7_8_9_match_their_references() {
    for n in ["gm2_fsop_2", "gm2_fsop_3", "gm2_fsop_6", "gm2_fsop_7", "gm2_fsop_8", "gm2_fsop_9"] {
        mali_ref::assert_matches(n);
    }
}

/// jellyfish 8ee5's own shape: compiles, its instructions in another order.
#[test]
#[ignore = "both differences before both multiply-adds in Mesa's program: the pair graph orders one key's nodes by lane pair"]
fn gm2_fsop_1_matches_its_reference() {
    mali_ref::assert_matches("gm2_fsop_1");
}

/// A uniform plus a product two lane pairs read: Mesa fuses the product
/// into each pair's sum (`FMA.v2f16 r1, r0.h00, r0.h11, u2.w0`); the walk
/// keeps it once, a form no reference draws.
#[test]
fn gm2_fsop_5_stays_refused() {
    mali_ref::assert_refused_ref("gm2_fsop_5", "a fragment shader operand of a form or in a place no reference draws");
}
