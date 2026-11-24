// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The V3D 7.1 read-lost refusals (src/v3d/zskeep71.rs `ZsRefusal71::ReadLost`)
//! are the 4.2 back end's strings (src/v3d/backend.rs, `record`) word for
//! word: gl_test cases 59 and 67 on the Pi 5 must be refused with the Pi 4's
//! message (docs/notes/v3d71-gl-table.md). A decide() input shaped like case 59
//! (a stencil read after a write with no kept buffer) returns the stencil one.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_refuse`

#![allow(dead_code)]

#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use zskeep71::*;

const BACKEND: &str = include_str!("../src/v3d/backend.rs");

fn pinned(r: ZsRefusal71) {
    let lit = format!("\"{}\"", r.message());
    assert!(BACKEND.contains(&lit), "src/v3d/backend.rs lacks the 7.1 refusal {lit}");
}

#[test]
fn stencil_read_lost_is_the_42_string() {
    pinned(ZsRefusal71::ReadLost(Plane71::Stencil));
}

#[test]
fn depth_read_lost_is_the_42_string() {
    pinned(ZsRefusal71::ReadLost(Plane71::Depth));
}

#[test]
fn case_59_shape_is_read_lost_stencil() {
    // Tile-only target, stencil written by an earlier job and not kept, the
    // next job's draw tests stencil.
    let t = Target71 { layout: Layout71::TileOnly, depth: true, stencil: true };
    let mut job = Job71::NONE;
    job.s.draw(true, false);
    let e = decide(&t, Known71::Untouched, Known71::Written, &job).map(|_| ()).unwrap_err();
    assert_eq!(e, ZsRefusal71::ReadLost(Plane71::Stencil));
    pinned(e);
}
