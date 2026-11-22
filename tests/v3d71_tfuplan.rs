// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host check of `src/v3d/tfuplan71.rs` against `driver/src/v3d/v3d_tfu71.{h,c}`.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_tfuplan`
//! (the features only let the crate build). The constants and the supported
//! type list are read from the C text, so a change there fails this test.
#![allow(dead_code)]

#[path = "../src/v3d/tfuplan71.rs"]
mod tfuplan71;
use tfuplan71::*;

fn drv(f: &str) -> String {
    let p = format!("{}/driver/src/v3d/{}", env!("CARGO_MANIFEST_DIR"), f);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn define(h: &str, name: &str) -> u32 {
    let l = h.lines().find(|l| l.starts_with(&format!("#define {name} "))).unwrap_or_else(|| panic!("{name}"));
    l.split_whitespace().nth(2).unwrap().trim_end_matches('u').parse().unwrap()
}

#[test]
fn constants_match_c() {
    let h = drv("v3d_tfu71.h");
    assert_eq!(define(&h, "V3D_TFU71_MAX_DIM"), MAX_DIM);
    assert_eq!(define(&h, "V3D_TFU71_IOC_NUMMM_SHIFT"), IOC_NUMMM_SHIFT);
    // NUMMM is 4 bits wide: the next field (FORMAT) starts 8 bits above it
    assert_eq!(define(&h, "V3D_TFU71_IOC_FORMAT_SHIFT"), IOC_NUMMM_SHIFT + 8);
    assert!(h.contains("#define V3D_TFU71_IOC_DIMTW (1u << 0)"));
}

#[test]
fn supported_list_matches_c() {
    let c = drv("v3d_tfu71.c");
    let body = &c[c.find("bool v3d_tfu71_supports").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let (t, f) = body.split_once("return true;").unwrap();
    let nums = |s: &str| -> Vec<u32> {
        s.lines().filter_map(|l| l.trim().strip_prefix("case ")?.split(':').next()?.parse().ok()).collect()
    };
    assert_eq!(nums(t), SUPPORTED_MIPMAP);
    let (f, _) = f.split_once("return !for_mipmap;").unwrap();
    assert_eq!(nums(f), SUPPORTED_COPY_ONLY);
}

fn base(w: u32, h: u32, last: u32) -> Args {
    Args {
        width: w,
        height: h,
        base_level: 0,
        last_level: last,
        src_level: 0,
        ttype: 4,
        cpp: 4,
        src_tiling: Tiling::UifNoXor,
        dst_tiling: Tiling::UifNoXor,
        layer: 0,
        layer_count: 1,
    }
}

#[test]
fn rgba8_256_full_chain() {
    let jobs = plan(&base(256, 256, 8)).unwrap();
    assert_eq!(jobs.len(), 1);
    let j = &jobs[0];
    assert_eq!((j.in_level, j.out_level, j.nummm), (0, 1, 8));
    assert_eq!(j.ioc_mm, 1 | 8 << 4);
    assert_eq!(j.levels.len(), 8);
    // cpp 4: utile 4x4 texels, UIF block 8x8 texels = 256 bytes
    assert_eq!((j.levels[0].width, j.levels[0].height, j.levels[0].bytes), (128, 128, 128 * 128 * 4));
    // the old pins (levels packed upward in whole UIF blocks) were not Mesa's
    // layout: small levels are LINEARTILE/UBLINEAR and sit BELOW the base
    // (v3d_setup_slices, v3d_resource.c:583-767; tests/v3d71_slices.rs)
    assert_eq!((j.levels[7].width, j.levels[7].height, j.levels[7].bytes), (1, 1, 64));
    assert!(j.levels.iter().all(|l| l.offset < j.base_offset));
    assert!(j.levels.windows(2).all(|w| w[1].offset < w[0].offset));
    assert_eq!(j.base_offset % 4096, 0);
}

#[test]
fn non_square_64x16() {
    let j = &plan(&base(64, 16, 6)).unwrap()[0];
    assert_eq!(j.nummm, 6);
    let dims: Vec<_> = j.levels.iter().map(|l| (l.width, l.height)).collect();
    assert_eq!(dims, [(32, 8), (16, 4), (8, 2), (4, 1), (2, 1), (1, 1)]);
}

#[test]
fn cube_face_and_layers() {
    let mut a = base(64, 64, 3);
    a.layer = 2;
    assert_eq!(plan(&a).unwrap()[0].layer, 2);
    a.layer = 0;
    a.layer_count = 6;
    let j = plan(&a).unwrap();
    assert_eq!(j.iter().map(|j| j.layer).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn refusals() {
    let mut a = base(64, 64, 6);
    a.ttype = 20;
    assert_eq!(plan(&a).unwrap_err(), Refusal::UnsupportedType);
    assert!(supports(20, false));
    assert_eq!(plan(&base(16385, 4, 3)).unwrap_err(), Refusal::Dimension);
    assert_eq!(plan(&base(0, 4, 1)).unwrap_err(), Refusal::Dimension);
    assert_eq!(plan(&base(64, 64, 7)).unwrap_err(), Refusal::LevelCount);
    assert_eq!(plan(&base(64, 64, 0)).unwrap_err(), Refusal::NoLevels);
    let mut a = base(64, 64, 3);
    a.base_level = 4;
    assert_eq!(plan(&a).unwrap_err(), Refusal::LevelCount);
    let mut a = base(64, 64, 3);
    a.dst_tiling = Tiling::Raster;
    assert_eq!(plan(&a).unwrap_err(), Refusal::RasterDestination);
    let mut a = base(64, 64, 3);
    a.src_level = 1;
    assert_eq!(plan(&a).unwrap_err(), Refusal::SourceNotBase);
    let mut a = base(64, 64, 3);
    a.src_tiling = Tiling::LinearTile;
    assert_eq!(plan(&a).unwrap_err(), Refusal::SourceNotBase);
    let mut a = base(64, 64, 3);
    a.cpp = 3;
    assert_eq!(plan(&a).unwrap_err(), Refusal::BadCpp);
    // a 16384 side halves 14 times: fits the 4-bit field, 15 does not
    assert_eq!(plan(&base(16384, 16384, 14)).unwrap()[0].nummm, 14);
    assert_eq!(plan(&base(16384, 16384, 15)).unwrap_err(), Refusal::LevelCount);
    assert!(!Refusal::Dimension.message().is_empty());
}
