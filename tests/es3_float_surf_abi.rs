// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The backend ABI's three extension float colour surfaces (plan #360):
//! GLES_SURF_RG32F, GLES_SURF_RGBA32F and GLES_SURF_R11F_G11F_B10F. Pinned:
//!
//! - the values 27, 28 and 29 and the FLOAT_FIRST/LAST bracket;
//! - bytes a pixel (8, 16, 4), the sampled texel format and the name;
//! - `FLOAT_RT_FORMATS` is 0, so `rt_format_drawable` is false for all three
//!   on this GPU, until the host-test override `HOST_FLOAT_RT_FORMATS` sets
//!   a bit (and then only that bit's surface);
//! - src/backend.h carries the same values and bytes per pixel (the
//!   generated ABI check pins structs only, not these constants);
//! - the neighbours keep their answers.
//!
//! Nothing draws or produces these surfaces yet: the GL core and every back
//! end are untouched.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_float_surf_abi
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_float_surf_abi
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_float_surf_abi
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use yakogl::backend::*;

/* HOST_FLOAT_RT_FORMATS is process-wide: serialise the tests that read it. */
static LOCK: Mutex<()> = Mutex::new(());

const NEW: [(u8, u8, u32, &str); 3] = [
    (GLES_SURF_RG32F, 27, 8, "RG32F"),
    (GLES_SURF_RGBA32F, 28, 16, "RGBA32F"),
    (GLES_SURF_R11F_G11F_B10F, 29, 4, "R11F_G11F_B10F"),
];

#[test]
fn values_sizes_names() {
    for (s, v, bpp, name) in NEW {
        assert_eq!(s, v, "{name}");
        assert_eq!(gles_surf_bytes_per_pixel(s), bpp, "{name}");
        assert_eq!(gles_surf_name(s), name);
        assert!(gles_surf_is_ext_float(s));
        assert!(!gles_surf_is_integer(s) && !gles_surf_is_srgb(s));
    }
    assert_eq!((GLES_SURF_FLOAT_FIRST, GLES_SURF_FLOAT_LAST), (27, 29));
    assert!(!gles_surf_is_ext_float(GLES_SURF_RGB10_A2) && !gles_surf_is_ext_float(30));
    assert_eq!(gles_surf_texfmt(GLES_SURF_RGBA32F), GLES_TEXFMT_RGBA32F);
    assert_eq!(gles_surf_texfmt(GLES_SURF_R11F_G11F_B10F), GLES_TEXFMT_R11G11B10F);
    /* neighbours unchanged */
    assert_eq!(GLES_SURF_RGB10_A2, 26);
    assert_eq!(gles_surf_bytes_per_pixel(GLES_SURF_R32F), 4);
    assert_eq!(gles_surf_bytes_per_pixel(GLES_SURF_RGBA16F), 8);
    assert_eq!(gles_surf_bytes_per_pixel(GLES_SURF_RGBA32UI), 16);
}

#[test]
fn undrawable_by_default() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(FLOAT_RT_FORMATS, 0);
    assert_eq!(HOST_FLOAT_RT_FORMATS.load(Ordering::Relaxed), 0);
    for (s, _, _, name) in NEW {
        assert!(!rt_format_drawable(s), "{name}");
    }
    /* the surfaces that were always drawable stay so */
    assert!(rt_format_drawable(GLES_SURF_RGBA16F) && rt_format_drawable(GLES_SURF_R32F));
}

#[test]
fn host_override_sets_one_bit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HOST_FLOAT_RT_FORMATS.store(1 << (GLES_SURF_RGBA32F - GLES_SURF_FLOAT_FIRST), Ordering::Relaxed);
    let got = NEW.map(|(s, ..)| rt_format_drawable(s));
    HOST_FLOAT_RT_FORMATS.store(0, Ordering::Relaxed);
    assert_eq!(got, [false, true, false]);
    assert!(!rt_format_drawable(GLES_SURF_RGBA32F));
}

#[test]
fn c_header_agrees() {
    let h = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backend.h")).unwrap();
    for (_, v, bpp, name) in NEW {
        let sym = format!("GLES_SURF_{name}");
        assert!(h.contains(&format!("{sym} = {v}")), "backend.h: {sym} = {v}");
        if bpp != 4 {
            assert!(h.contains(&format!("format == {sym}")), "backend.h bytes per pixel of {sym}");
        }
    }
    assert!(h.contains("format == GLES_SURF_RGBA32F ? 16u"));
    assert!(h.contains("format == GLES_SURF_RGBA16F || format == GLES_SURF_RG32F ? 8u"));
}
