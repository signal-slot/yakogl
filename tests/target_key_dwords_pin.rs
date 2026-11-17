// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-format target key and its dword count, pinned as a literal table:
//! for every format value 0..=255, `gles_surf_target_key(fmt)` and
//! `shc_target_key_dwords` of that key (src/target_key.rs) held against one
//! row.
//!
//! The dword count is the compiler's side of "layout equals emit": the mixed
//! fragment program places a target's colour in that many pixel output
//! registers, and the PowerVR end-of-tile emit reads the same number back
//! (`surf_out_dwords`, src/pvr/backend.rs). The expected counts below were
//! derived by reading `surf_out_dwords`: an integer layout gives
//! `DIV_ROUND_UP(channels * bits, 32)` with the 10-bit RGB10_A2UI word as
//! one (R8/R16/R32 and RG8/RG16, RGBA8 and RGB10_A2UI: 1; RG32 and RGBA16:
//! 2; RGBA32: 4), RGBA16F and the sRGB surface 2, everything else 1. The
//! sRGB surface has no target key (refused in a mix), so its row is `None`.
//! The back end's own cross-check of these counts against `surf_out_dwords`
//! (private to backend.rs) is a later item (pvr-accept).
//!
//! Nothing here is GPU-specific (src/target_key.rs is ungated const code),
//! so the file names no cfg, like tests/gles_surf_class_pin.rs. Feature set:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test target_key_dwords_pin
//! ```

use yakogl::backend::{
    gles_surf_name, GLES_SURF_R11F_G11F_B10F, GLES_SURF_R16F, GLES_SURF_R16I, GLES_SURF_R16UI, GLES_SURF_R32F,
    GLES_SURF_R32I, GLES_SURF_R32UI, GLES_SURF_R8I, GLES_SURF_R8UI, GLES_SURF_RG16I, GLES_SURF_RG16UI, GLES_SURF_RG32F,
    GLES_SURF_RG32I, GLES_SURF_RG32UI, GLES_SURF_RG8I, GLES_SURF_RG8UI, GLES_SURF_RGB10_A2, GLES_SURF_RGB10_A2UI,
    GLES_SURF_RGB565, GLES_SURF_RGBA16F, GLES_SURF_RGBA16I, GLES_SURF_RGBA16UI, GLES_SURF_RGBA32F, GLES_SURF_RGBA32I,
    GLES_SURF_RGBA32UI, GLES_SURF_RGBA8888, GLES_SURF_RGBA8I, GLES_SURF_RGBA8UI, GLES_SURF_SRGB8_A8,
    GLES_SURF_XRGB8888,
};
use yakogl::target_key::{gles_surf_target_key, shc_target_key_dwords};

/// (format, dwords) of every format that has a target key.
const KEYED: &[(u8, u32)] = &[
    (GLES_SURF_RGBA8888, 1),
    (GLES_SURF_XRGB8888, 1),
    (GLES_SURF_RGB565, 1),
    (GLES_SURF_RGBA16F, 2),
    (GLES_SURF_R16F, 1),
    (GLES_SURF_R32F, 1),
    (GLES_SURF_R8I, 1),
    (GLES_SURF_R8UI, 1),
    (GLES_SURF_R16I, 1),
    (GLES_SURF_R16UI, 1),
    (GLES_SURF_R32I, 1),
    (GLES_SURF_R32UI, 1),
    (GLES_SURF_RG8I, 1),
    (GLES_SURF_RG8UI, 1),
    (GLES_SURF_RG16I, 1),
    (GLES_SURF_RG16UI, 1),
    (GLES_SURF_RG32I, 2),
    (GLES_SURF_RG32UI, 2),
    (GLES_SURF_RGBA8I, 1),
    (GLES_SURF_RGBA8UI, 1),
    (GLES_SURF_RGBA16I, 2),
    (GLES_SURF_RGBA16UI, 2),
    (GLES_SURF_RGBA32I, 4),
    (GLES_SURF_RGBA32UI, 4),
    (GLES_SURF_RGB10_A2UI, 1),
    (GLES_SURF_RGB10_A2, 1),
];

/// Formats that are named colour surfaces but have no key (the three extension
/// float surfaces of plan #360 are drawn by no back end yet).
const UNKEYED: &[u8] = &[GLES_SURF_SRGB8_A8, GLES_SURF_RG32F, GLES_SURF_RGBA32F, GLES_SURF_R11F_G11F_B10F];

#[test]
fn target_key_dwords_per_format() {
    let mut named = 0usize;
    for fmt in 0u8..=255 {
        let name = gles_surf_name(fmt);
        let key = gles_surf_target_key(fmt);
        let row = KEYED.iter().filter(|r| r.0 == fmt).count();
        if name == "unknown" {
            assert_eq!(row, 0, "value {fmt} has a row but names no colour format");
            assert_eq!(key, None, "undefined value {fmt} must have no target key");
            continue;
        }
        named += 1;
        if UNKEYED.contains(&fmt) {
            assert_eq!(row, 0, "{name} ({fmt}) must have no keyed row");
            assert_eq!(key, None, "{name} ({fmt}) must have no target key");
            continue;
        }
        assert_eq!(row, 1, "{name} ({fmt}) needs exactly one row, has {row}");
        let want = KEYED.iter().find(|r| r.0 == fmt).unwrap().1;
        let k = key.unwrap_or_else(|| panic!("{name} ({fmt}) has no target key"));
        assert_eq!(shc_target_key_dwords(k), Some(want), "{name} ({fmt}): key {k:#010x} dword count moved");
    }
    assert_eq!(named, KEYED.len() + UNKEYED.len(), "named colour formats and the table are not the same set");
}
