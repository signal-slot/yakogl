// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's effect2d convolution kernels on the Mali (plan #367
//! lower-nonfinite-const, step effect2d: fs 3c3d and ba8e, TEX_MULTI),
//! against the Mesa 26.2.1 drm-shim references
//! `tools/mali/testdata/gm2_fx2d_*`, byte for byte
//! (docs/notes/mali-tex-multi.md section 4):
//!
//! - a kernel's `-1.0` makes the tap a negated texel, folded into the sum
//!   that reads it as a source modifier (`FADD.v2f16 r3, r4^.neg, r6^.neg`);
//! - the zero taps are gone, the centre tap reads the varying as it is
//!   loaded, every other tap is an `FADD_IMM.v2f16` of the converted pair.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_effect2d_tex
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The Laplace kernel (0,1,0;1,-4,1;0,1,0): effect2d 3c3d's shape.
#[test]
fn gm2_fx2d_1_the_laplace_kernel_matches_its_reference() {
    mali_ref::assert_matches("gm2_fx2d_1");
}

/// The sharpen kernel (0,-1,0;-1,5,-1;0,-1,0): four negated taps.
#[test]
fn gm2_fx2d_2_the_sharpen_kernel_matches_its_reference() {
    mali_ref::assert_matches("gm2_fx2d_2");
}

/// The emboss kernel (-2,-1,0;-1,1,1;0,1,2): `-2.0` as `0x40000000.neg.h11`.
#[test]
fn gm2_fx2d_3_the_emboss_kernel_matches_its_reference() {
    mali_ref::assert_matches("gm2_fx2d_3");
}

/// A negated tap read by something other than a sum or product of the
/// colour (a compare) has no reference: refused.
#[test]
fn a_negated_tap_in_a_compare_stays_refused() {
    let vs = "attribute vec4 p; varying vec2 t; void main() { gl_Position = p; t = p.xy; }";
    let fs = "precision mediump float; uniform sampler2D s; varying vec2 t; \
              void main() { vec4 a = -texture2D(s, t + vec2(0.25, 0.0)); vec4 b = texture2D(s, t); \
              gl_FragColor = a.x < b.x ? a : b; }";
    let r = mali_ref::compile_pair(vs, fs);
    assert!(r.is_err(), "a select of a negated tap compiles");
}
