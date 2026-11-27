// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 refract (24)'s front `refract()` on the Mali (plan #367
//! lower-train-2, from proto-fs-refract, the FS_REFRACT family), against the
//! Mesa 26.2.1 drm-shim reference `tools/mali/testdata/gm2_nbshrf_6`, byte
//! for byte (docs/notes/mali-fs-refract.md):
//!
//! - `k < 0.0 ? 0.0 : eta * I - (eta * d + sqrt(k)) * N` is a 16-bit select
//!   of a constant and a computed lane on the lane's own compare, and the
//!   colour's `* 0.5 + 0.5` moves into it (nir_opt_algebraic.py:3429-3433):
//!   `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x38000000.h11`, `FCMP_OR.v2f16.lt.m1
//!   r2, r5^.h00, 0x0, 0x0`, then `MUX.v2i16 r0, r0^, 0x38000000.h11, r2.h00`;
//! - `k` and `eta * d + sqrt(k)` are one lane each (Mesa's `float k`), `1.0 -
//!   eta * eta * (1.0 - d * d)` with the value first (`FMA.v2f16 r5,
//!   r5^.neg.h00, u2.w0, 0x3C000000.h11`, GLSL's `reassociate_constant`);
//! - `eta * I - t * N` with both products `t * N` before both multiply-adds
//!   (`FMA.v2f16 r0, u2.w1, r0^, r2^.neg`).
//!
//! The narrowness of each rule is guarded by the refusals other tests pin:
//! the compare products' `fmul(b2f(a), b)` selects (`tests/mali_deqp_fclane.rs`
//! `bcast_compare_*`, `tests/mali_deqp_uniarray.rs` `struct_array`) and the
//! matrix products whose steps the lane table also records as 0
//! (`tests/mali_deqp_unitype.rs`'s mat4, byte-equal). dEQP's scalar
//! `refract` (`tests/mali_deqp_arith.rs`) moves to its colour lane.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_fs_refract
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// `vec4(refract(normalize(vp.xyz), normalize(vn), 1.5) * 0.5 + 0.5, 1.0)`:
/// every region (the position and varying programs, the fragment program),
/// the facts and the FAU words (`0x40804080`, 2.25 in both halves, and
/// `0x3e003e00`, 1.5) as Mesa's.
#[test]
fn gm2_nbshrf_6_refract_of_two_varyings_matches_its_reference() {
    mali_ref::assert_matches("gm2_nbshrf_6");
}

/// dEQP's scalar `refract` of three varyings gets past the select and stops
/// at its colour lane: pinned, so the item that lowers it moves these.
#[test]
fn deqp_scalar_refract_stops_at_its_colour_lane() {
    let lane = "a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet";
    let moved: Vec<String> = ["deqp_arith_op_geometric_refract_fs_highp", "deqp_arith_op_geometric_refract_fs_mediump"]
        .iter()
        .filter_map(|n| mali_ref::refusal_change(n, lane))
        .collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}
