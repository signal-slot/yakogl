// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The colour format -> target class and integer layout map, pinned as a
//! literal table: for every format value 0..=255, `gles_surf_rt_class` and
//! `gles_surf_int_layout` (src/backend.rs) held against one row.
//!
//! The GL core's `current_blend_key` (src/gl/draw.rs) and the PowerVR back
//! end each build the per-target key of one colour attachment from its
//! format when they key a draw that mixes integer attachments with float or
//! unorm ones (docs/notes/pvr-int-mixed-targets.md 2.1, 4 and 5): an
//! integer format through its `gles_surf_int_layout`, every other format
//! through its `gles_surf_rt_class`. Both sides read these same two
//! functions; a program that packs a target by one mapping while the emit
//! counts it by the other writes one target's pixels into another target's
//! registers. The table below is the one record both items must move in the
//! same commit as the two functions move.
//!
//! The walk covers every value the `u8` format can hold, not only the
//! defined ones. Each format src/backend.rs names (`gles_surf_name` not
//! "unknown": today the constants `GLES_SURF_RGBA8888` ..
//! `GLES_SURF_RGB10_A2`, values 0..=26) has exactly one row pinning its
//! class and its layout (channels, bits a channel, signed, or `None`);
//! every other value must answer the fallback the two functions have for
//! it (the RGBA8 class, no integer layout, name "unknown"). A new colour
//! surface constant that joins the name table without a row here fails the
//! walk instead of sliding into the keys unnoticed.
//!
//! The pinned rows also hold the two properties the design reads off the
//! map: every value `gles_surf_is_integer` calls integer has an integer
//! layout (the per-target integer codes are built from it), and no sRGB
//! value (`gles_surf_is_srgb`) answers one (its key is the class
//! `SHC_RT_SRGB` today and the reserved slot code `0x04` of the design,
//! never an integer layout).
//!
//! Nothing here is GPU- or feature-specific -- the map is the same on every
//! back end -- so the file names no cfg: the gate's whole-config sweep
//! (`tools/check.sh`'s CONFIGS loop, a plain `cargo test` per feature set)
//! builds and runs it in every host configuration, and it runs on the
//! PowerVR feature set the plan's items name:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gles_surf_class_pin
//! ```

use yakogl::backend::{
    gles_surf_int_layout, gles_surf_is_integer, gles_surf_is_srgb, gles_surf_name, gles_surf_rt_class, GlesIntLayout,
    GLES_SURF_INT_FIRST, GLES_SURF_INT_LAST, GLES_SURF_R16F, GLES_SURF_R16I, GLES_SURF_R16UI, GLES_SURF_R32F,
    GLES_SURF_R32I, GLES_SURF_R32UI, GLES_SURF_R8I, GLES_SURF_R8UI, GLES_SURF_RG16I, GLES_SURF_RG16UI, GLES_SURF_RG32I,
    GLES_SURF_RG32UI, GLES_SURF_RG8I, GLES_SURF_RG8UI, GLES_SURF_RGB10_A2, GLES_SURF_RGB10_A2UI, GLES_SURF_RGB565,
    GLES_SURF_RGBA16F, GLES_SURF_RGBA16I, GLES_SURF_RGBA16UI, GLES_SURF_RGBA32I, GLES_SURF_RGBA32UI,
    GLES_SURF_RGBA8888, GLES_SURF_RGBA8I, GLES_SURF_RGBA8UI, GLES_SURF_SRGB8_A8, GLES_SURF_XRGB8888,
};
use yakogl::shaderc::{SHC_RT_F16, SHC_RT_F16X4, SHC_RT_F32, SHC_RT_RGBA8, SHC_RT_SRGB};

/// One pinned row of the map.
struct Pin {
    /// The format value, named by its src/backend.rs constant.
    format: u8,
    /// What `gles_surf_rt_class` must answer for it.
    rt_class: u32,
    /// What `gles_surf_int_layout` must answer for it: a layout with its
    /// channel count, bits a channel and signedness, or `None` when the
    /// format has no integer layout.
    int_layout: Option<GlesIntLayout>,
}

/// A pinned integer layout, spelled (channels, bits a channel, signed).
const fn layout(channels: u8, bits: u8, signed: bool) -> GlesIntLayout {
    GlesIntLayout { channels, bits, signed }
}

/// The whole map, one row for each colour surface format src/backend.rs
/// defines, in the order of the format values. What each row says:
///
/// - the integer formats answer the class `SHC_RT_RGBA8` from
///   `gles_surf_rt_class` -- their own key is built from the layout, not
///   the class (`shc_blend_key_int_rt` today, the integer slot codes of
///   docs/notes/pvr-int-mixed-targets.md 2.2), and the caller routes them
///   through `gles_surf_int_layout` before it reads a class. The row pins
///   today's raw answer;
/// - `GLES_SURF_SRGB8_A8` is a class of its own, refused in a mix (the
///   design 5), and no integer layout;
/// - `GLES_SURF_RGB10_A2` is a unorm surface today: class RGBA8, no
///   integer layout, although its word is the 10/10/10/2 one.
const PINS: &[Pin] = &[
    Pin { format: GLES_SURF_RGBA8888, rt_class: SHC_RT_RGBA8, int_layout: None },
    Pin { format: GLES_SURF_XRGB8888, rt_class: SHC_RT_RGBA8, int_layout: None },
    Pin { format: GLES_SURF_RGB565, rt_class: SHC_RT_RGBA8, int_layout: None },
    Pin { format: GLES_SURF_RGBA16F, rt_class: SHC_RT_F16X4, int_layout: None },
    Pin { format: GLES_SURF_R16F, rt_class: SHC_RT_F16, int_layout: None },
    Pin { format: GLES_SURF_R32F, rt_class: SHC_RT_F32, int_layout: None },
    Pin { format: GLES_SURF_R8I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(1, 8, true)) },
    Pin { format: GLES_SURF_R8UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(1, 8, false)) },
    Pin { format: GLES_SURF_R16I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(1, 16, true)) },
    Pin { format: GLES_SURF_R16UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(1, 16, false)) },
    Pin { format: GLES_SURF_R32I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(1, 32, true)) },
    Pin { format: GLES_SURF_R32UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(1, 32, false)) },
    Pin { format: GLES_SURF_RG8I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(2, 8, true)) },
    Pin { format: GLES_SURF_RG8UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(2, 8, false)) },
    Pin { format: GLES_SURF_RG16I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(2, 16, true)) },
    Pin { format: GLES_SURF_RG16UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(2, 16, false)) },
    Pin { format: GLES_SURF_RG32I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(2, 32, true)) },
    Pin { format: GLES_SURF_RG32UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(2, 32, false)) },
    Pin { format: GLES_SURF_RGBA8I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 8, true)) },
    Pin { format: GLES_SURF_RGBA8UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 8, false)) },
    Pin { format: GLES_SURF_RGBA16I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 16, true)) },
    Pin { format: GLES_SURF_RGBA16UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 16, false)) },
    Pin { format: GLES_SURF_RGBA32I, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 32, true)) },
    Pin { format: GLES_SURF_RGBA32UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 32, false)) },
    Pin { format: GLES_SURF_RGB10_A2UI, rt_class: SHC_RT_RGBA8, int_layout: Some(layout(4, 10, false)) },
    Pin { format: GLES_SURF_SRGB8_A8, rt_class: SHC_RT_SRGB, int_layout: None },
    Pin { format: GLES_SURF_RGB10_A2, rt_class: SHC_RT_RGBA8, int_layout: None },
];

/// Walk 0..=255: every value src/backend.rs names as a colour format is
/// pinned exactly once, to the class and layout the two functions answer;
/// every value it does not name answers the fallback.
#[test]
fn pinned_table_is_the_whole_map() {
    let mut named_count = 0usize;
    for format in 0u8..=255 {
        let name = gles_surf_name(format);
        let mut pinned_rows = 0usize;
        let mut pin: Option<&Pin> = None;
        for p in PINS {
            if p.format == format {
                pinned_rows += 1;
                pin = Some(p);
            }
        }
        if name != "unknown" {
            named_count += 1;
            assert_eq!(
                pinned_rows, 1,
                "{name} (value {format}) is pinned {pinned_rows} times, exactly one row is wanted"
            );
            let p = pin.unwrap_or_else(|| panic!("{name} (value {format}) has no pinned row"));
            assert_eq!(
                gles_surf_rt_class(format),
                p.rt_class,
                "target class of {name} ({format}) moved away from the pinned row"
            );
            assert_eq!(
                gles_surf_int_layout(format),
                p.int_layout,
                "integer layout of {name} ({format}) moved away from the pinned row"
            );
        } else {
            assert_eq!(
                pinned_rows, 0,
                "value {format} has a pinned row but src/backend.rs names no colour format for it"
            );
            assert_eq!(
                gles_surf_rt_class(format),
                SHC_RT_RGBA8,
                "the undefined value {format} must answer the fallback class SHC_RT_RGBA8"
            );
            assert_eq!(
                gles_surf_int_layout(format),
                None,
                "the undefined value {format} must answer no integer layout"
            );
        }
    }
    assert_eq!(
        named_count,
        PINS.len(),
        "the colour formats src/backend.rs names and the pinned table are not the same set"
    );
}

/// `gles_surf_is_integer` and `gles_surf_int_layout` agree over 0..=255:
/// every integer colour format has a layout, and the walk holds exactly
/// the formats the `GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST` bracket
/// covers. Each layout is a shape the ES 3.0 integer table has.
#[test]
fn every_integer_format_has_an_int_layout() {
    let mut walked = 0usize;
    for format in 0u8..=255 {
        if !gles_surf_is_integer(format) {
            continue;
        }
        walked += 1;
        let name = gles_surf_name(format);
        let Some(l) = gles_surf_int_layout(format) else {
            panic!("the integer colour format {name} ({format}) answers no integer layout");
        };
        let channels = l.channels;
        assert!(matches!(channels, 1 | 2 | 4), "{name}: {channels} channels, not 1, 2 or 4");
        if l.bits == 10 {
            // The 10-bit width exists only as the RGB10_A2UI word: 10/10/10
            // with alpha's 2 on top, the one case `GlesIntLayout` names.
            assert_eq!(l.channels, 4, "{name}: a 10-bit integer format must be the four-channel RGB10_A2UI word");
        } else {
            assert!(matches!(l.bits, 8 | 16 | 32), "{name}: {} bits a channel, not 8, 16 or 32", l.bits);
        }
    }
    assert_eq!(
        walked,
        (GLES_SURF_INT_LAST - GLES_SURF_INT_FIRST + 1) as usize,
        "the integer values in the 0..=255 walk and the GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST bracket disagree"
    );
}

/// No sRGB colour format maps to an integer layout: the sRGB surface keeps
/// the class `SHC_RT_SRGB` and is refused in a mix (the design's 5), and
/// never keys as an integer target.
#[test]
fn no_srgb_format_maps_to_an_integer_layout() {
    let mut srgb_count = 0usize;
    for format in 0u8..=255 {
        if !gles_surf_is_srgb(format) {
            continue;
        }
        srgb_count += 1;
        let name = gles_surf_name(format);
        assert_eq!(
            gles_surf_int_layout(format),
            None,
            "the sRGB colour format {name} ({format}) answers an integer layout"
        );
        assert!(!gles_surf_is_integer(format), "the sRGB colour format {name} ({format}) sits in the integer bracket");
    }
    assert_eq!(srgb_count, 1, "src/backend.rs must name exactly one sRGB colour format");
}
