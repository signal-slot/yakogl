// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! gl_test case 67 (FBO limits) on the 7.1 path: what decides the GL_RGB
//! colour target and what it returns today.
//!
//! The task text and docs/notes/v3d71-gl-table.md section 5 say the shared
//! `src/v3d/state.rs` refuses a GL_RGB frame with "a colour format with
//! fewer than 8 alpha bits". At this tree that refusal is **gone**: state.rs
//! draws a format with fewer than 8 alpha bits (GL_RGB, RGB565: stored as
//! RGBA8, alpha loaded as 1 by `force_alpha_1`, no draw writes it, the
//! colour mask's alpha bit is cleared when `target_alpha_bits == 0`; RGBA4
//! and RGB5_A1 keep the GL mask), pinned by state.rs's
//! `a_format_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha`
//! and backend_tests.rs's `a_target_with_fewer_alpha_bits_is_drawn_...`.
//! So case 67 is no longer a refusal on the 4.2 path, and this test pins
//! the *current* behaviour on both sides:
//!
//! * (a) the refusal literal is absent from state.rs as a message (only the
//!   words "fewer than 8 alpha bits" remain, in comments that say it is
//!   drawn), and the drawing pins above are still there;
//! * (b) `rt71::rt_format71` (the pure 7.1 module that decides a colour
//!   target: `rt71.rs`; `format71.rs` decides only TMU sampling and
//!   `frame71.rs` only packet packing) admits GL_RGB8 and GL_RGB565 as the
//!   8-bit type-8 tile-buffer target, 32 bpp, exactly as it does RGBA8, and
//!   has no alpha-bits notion to refuse with. The decision that keeps alpha
//!   at 1 is state.rs's `job_state` colour-mask rule
//!   (`target_alpha_bits == 0` => `colormask &= 0x7`) plus the job's
//!   `force_alpha_1` load; the 7.1 table must route through it (or repeat
//!   it) rather than rely on rt71;
//! * (c) the case's GL_RGBA8 target is admitted with the same row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_refuse67
//! ```
//! (the module and this test use no crate feature.)
#[path = "../src/v3d/rt71.rs"]
mod rt71;

use rt71::*;

const STATE_RS: &str = include_str!("../src/v3d/state.rs");
const BACKEND_TESTS_RS: &str = include_str!("../src/v3d/backend_tests.rs");

const GL_RGB8: u32 = 0x8051;
const GL_RGB565: u32 = 0x8D62;
const GL_RGBA8: u32 = 0x8058;

#[test]
fn state_rs_no_longer_carries_the_alpha_bits_refusal() {
    // the old message as a string literal (opening quote then the text)
    let old = "\"a colour format with fewer than 8 alpha bits";
    assert!(!STATE_RS.contains(old), "state.rs refuses again: the 7.1 table must too");
    assert!(!BACKEND_TESTS_RS.contains(old));
    // what is left of the words is the "is drawn" documentation and pins
    assert!(STATE_RS.contains("fewer than 8 alpha bits is drawn"));
    assert!(STATE_RS.contains("a_format_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha"));
    assert!(BACKEND_TESTS_RS.contains("a_target_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha_one"));
    // the shared decision the 7.1 table must route through
    assert!(STATE_RS.contains("if d.target_alpha_bits == 0 {"));
    assert!(STATE_RS.contains("st.colormask &= 0x7;"));
}

#[test]
fn rt71_admits_gl_rgb_with_no_alpha_bits_refusal() {
    let rgba = rt_format71(GL_RGBA8).expect("RGBA8 is a render target");
    for f in [GL_RGB8, GL_RGB565] {
        let rt = rt_format71(f).unwrap_or_else(|e| panic!("{f:#x}: {}", e.as_str()));
        assert_eq!(rt, rgba, "{f:#x} is the RGBA8 row (stored as RGBA8)");
        assert_eq!(
            (rt.type_clamp, rt.bpp, rt.class, rt.integer),
            (TypeClamp::Type8, BPP_32, ClearClass::Unorm8, false)
        );
    }
    // no Refusal71 variant speaks of alpha
    for r in [
        Refusal71::FormatNotRenderable,
        Refusal71::NotAColourFormat,
        Refusal71::UnknownFormat,
        Refusal71::TooManyTargets,
        Refusal71::TargetTooLarge,
        Refusal71::EmptyTarget,
        Refusal71::ClearKindMismatch,
        Refusal71::TargetNumberOutOfRange,
    ] {
        assert!(!r.as_str().contains("alpha"), "{}", r.as_str());
    }
}

#[test]
fn rgba8_target_of_case_67_is_admitted() {
    let rt = rt_format71(GL_RGBA8).expect("GL_RGBA8 admitted");
    assert_eq!((rt.type_clamp, rt.bpp, rt.class), (TypeClamp::Type8, BPP_32, ClearClass::Unorm8));
}
