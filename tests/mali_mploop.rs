// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's function and loop fragment shaders on the Mali (plan #367
//! lower-nonfinite-const, step mploop), against the Mesa 26.2.1 drm-shim
//! references `tools/mali/testdata/gm2_mploop_*` (original reductions of
//! function fs 849c and loop fs 3678), byte for byte
//! (docs/notes/mali-fragcoord-fp16-ops.md, mali-int-loops.md):
//!
//! - function fs 849c: `fract(sqrt(t * t))` in the fp16 arithmetic after
//!   gl_FragCoord's highp head is `FRSQ.f16` then `FRCP.f16` of one lane
//!   (`lower_fsqrt`);
//! - loop fs 3678: a mediump loop to a uniform int bound, which Mesa keeps,
//!   is still refused (LOOP_FP16).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_mploop
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// 849c's shape: the head, then five calls of the `sqrt` step.
#[test]
fn gm2_mploop_2_five_sqrt_steps_match_their_reference() {
    mali_ref::assert_matches("gm2_mploop_2");
}

/// One step: `FMA.v2f16 r0, r0^.h00, r0^.h00` of the square, `FRSQ.f16`,
/// `FRCP.f16`, then the fract.
#[test]
fn gm2_mploop_3_one_sqrt_step_matches_its_reference() {
    mali_ref::assert_matches("gm2_mploop_3");
}

/// The control-flow reference of the same function shape, refused at the
/// same `sqrt` before.
#[test]
fn gm2_intloop_func_fs_matches_its_reference() {
    mali_ref::assert_matches("gm2_intloop_func_fs");
}

/// 3678's shape: the loop Mesa keeps, its fp16 value carried across the back
/// edge. Pinned, so the item that lowers it updates this file.
#[test]
fn gm2_mploop_1_the_mediump_kept_loop_stays_refused() {
    mali_ref::assert_refused_ref("gm2_mploop_1", "a loop Mesa keeps (a run-time bound) in a mediump fragment shader");
}

const HEAD: &str = "#if defined(GL_FRAGMENT_PRECISION_HIGH)
#define P highp
#else
#define P
#endif
precision mediump float;
varying float v;
";

/// `sqrt` of a varying's lane beside the island is no lane of the island:
/// refused as before.
#[test]
fn sqrt_of_a_varying_beside_the_island_stays_refused() {
    let vs = "attribute vec4 p; varying float v; void main() { gl_Position = p; v = p.x; }";
    let fs = format!(
        "{HEAD}void main() {{ P vec2 fc = gl_FragCoord.xy; float d = fract(fc.x * fc.y * 0.0001); \
         gl_FragColor = vec4(d + sqrt(v), 0.0, 0.0, 1.0); }}"
    );
    assert!(mali_ref::compile_pair(vs, &fs).is_err(), "sqrt of a varying beside the island compiles");
}
