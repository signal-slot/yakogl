// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the choices of `src/vivante/pe_overwrite.rs` (PE colour OVERWRITE
//! and COMPONENTS) against a model of etnaviv's rule, over every GL colour
//! mask, blend on/off, MSAA and the target formats. Choices only: the
//! register words are tests/gc_tiled_regs_mesa.rs.
//!
//! etnaviv's rule (Mesa 26.2.1, `src/gallium/drivers/etnaviv/`; that
//! directory is not vendored under third_party/, the lines below are cited
//! from the module's own header and checked there):
//! - `etnaviv_blend.c:97`: `fo_allowed = !alpha_enable && !logicop_enable`;
//! - `etnaviv_blend.c:145-151`: `full_overwrite = fo_allowed &&
//!   util_format_colormask_full(desc, colormask)`;
//! - `etnaviv_blend.c:166-169`: COMPONENTS is the pipe colour mask as given;
//! - `etnaviv_state.c:291-293`: OVERWRITE only for a single-sample target.
//!
//! The part that is vendored and read here:
//! `third_party/mesa-26.2.1/src/util/format/u_format.h:751-786`
//! (`util_format_colormask` counts channels with swizzle < 4, so the X byte
//! of an X8 format is no channel; `util_format_colormask_full` is
//! `(~mask & fmt_mask) == 0`).
//!
//! Where the module differs on purpose or by choice, the case is `#[ignore]`
//! with the reason (see the report's issue text).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_pe_overwrite_rules
//! ```

#[path = "../src/vivante/pe_overwrite.rs"]
#[allow(dead_code)]
mod pe_overwrite;

use pe_overwrite::*;

/// `util_format_colormask_full` (u_format.h:782-785).
fn colormask_full(fmt_mask: u32, mask: u32) -> bool {
    (!mask & fmt_mask) & 0xf == 0
}

/// etnaviv's (components, overwrite) for a format whose channel mask is
/// `fmt_mask` (0xf RGBA8, 0x7 B8G8R8X8).
fn etnaviv(fmt_mask: u32, mask: u32, blend: bool, msaa: bool) -> (u32, bool) {
    let fo_allowed = !blend; // etnaviv_blend.c:97 (no logic op in GL ES)
    let full = fo_allowed && colormask_full(fmt_mask, mask); // :145-151
    (mask & 0xf, full && !msaa) // :166-169, etnaviv_state.c:291-293
}

fn ours(mask: u32, blend: bool, alpha_bits: u32, t: PeTarget, msaa: bool) -> (u32, bool) {
    let p = pe_color(mask, blend, alpha_bits, t, msaa);
    (p.components, p.overwrite)
}

#[test]
fn rgba_target_matches_etnaviv_for_every_mask() {
    for mask in 0..16 {
        for blend in [false, true] {
            for msaa in [false, true] {
                for t in [PeTarget::Window, PeTarget::Fbo] {
                    assert_eq!(
                        ours(mask, blend, 8, t, msaa),
                        etnaviv(0xf, mask, blend, msaa),
                        "mask {mask:#x} blend {blend} msaa {msaa}"
                    );
                }
            }
        }
    }
}

#[test]
fn rgba_full_mask_no_blend_overwrites_only_single_sample() {
    assert_eq!(ours(0xf, false, 8, PeTarget::Window, false), (0xf, true));
    assert_eq!(ours(0xf, true, 8, PeTarget::Window, false), (0xf, false));
    assert_eq!(ours(0xf, false, 8, PeTarget::Window, true), (0xf, false));
    assert_eq!(ours(0x7, false, 8, PeTarget::Window, false), (0x7, false));
}

#[test]
fn overwrite_decision_matches_etnaviv_for_the_x_format_window() {
    // OVERWRITE (not COMPONENTS) of B8G8R8X8: the X byte is no channel.
    for mask in 0..16 {
        for blend in [false, true] {
            for msaa in [false, true] {
                assert_eq!(
                    ours(mask, blend, 0, PeTarget::Window, msaa).1,
                    etnaviv(0x7, mask, blend, msaa).1,
                    "mask {mask:#x} blend {blend} msaa {msaa}"
                );
            }
        }
    }
}

#[test]
fn x_format_window_components_without_overwrite_match_etnaviv() {
    // where OVERWRITE is off the COMPONENTS are the mask as given, but
    // the A bit of an X format never reaches the PE (module: `mask & 7`)
    for mask in 0..16 {
        let (c, o) = ours(mask, false, 0, PeTarget::Window, false);
        if !o {
            assert_eq!(c, etnaviv(0x7, mask, false, false).0 & 0x7, "mask {mask:#x}");
        }
    }
}

#[test]
#[ignore = "module widens COMPONENTS to 0xf when a full-RGB X-format window \
            overwrites; etnaviv_blend.c:166-169 keeps the mask as given (0x7)"]
fn x_format_window_components_equal_the_given_mask() {
    for mask in 0..16 {
        for blend in [false, true] {
            assert_eq!(
                ours(mask, blend, 0, PeTarget::Window, false).0,
                etnaviv(0x7, mask, blend, false).0,
                "mask {mask:#x} blend {blend}"
            );
        }
    }
}

#[test]
#[ignore = "deliberate: an FBO GL_RGB texture is RGBA8 sampled without an \
            alpha-1 swizzle, so the module never overwrites it; etnaviv \
            (format.c:359-360 swizzles alpha to 1) overwrites on a full RGB mask"]
fn x_format_fbo_overwrites_like_etnaviv() {
    assert_eq!(ours(0x7, false, 0, PeTarget::Fbo, false).1, etnaviv(0x7, 0x7, false, false).1);
}

#[test]
fn fbo_rgb_keeps_alpha_byte() {
    // the module's own rule, pinned: COMPONENTS mask & 7, never OVERWRITE
    for mask in 0..16 {
        assert_eq!(ours(mask, false, 0, PeTarget::Fbo, false), (mask & 7, false));
    }
}

#[test]
fn colormask_full_semantics_from_u_format_h() {
    assert!(colormask_full(0x7, 0x7));
    assert!(colormask_full(0x7, 0xf));
    assert!(!colormask_full(0x7, 0x3));
    assert!(colormask_full(0xf, 0xf));
    assert!(!colormask_full(0xf, 0x7));
}
