// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 tile size and the tile-buffer bpp field with four render
//! targets (v3d-tile4-test, docs/notes/mrt4-v3d-pvr.md "The tile size").
//!
//! Mesa 26.2.1 `v3d_choose_tile_size`, V3D 4.x branch
//! (third_party/mesa-26.2.1/src/broadcom/common/v3d_util.c:177-200): the
//! tile size is `tile_sizes[idx]` of {64x64, 64x32, 32x32, 32x16, 16x16,
//! 16x8, 8x8} (:133-140), `idx = (count > 4 ? 3 : count > 2 ? 2 : count > 1
//! ? 1 : 0) + (msaa ? 2 : 0) + max_internal_bpp` with the bpp enum 0 = 32,
//! 1 = 64, 2 = 128 (the largest over the render targets, v3dx_draw.c
//! `start_binning`, v3dx_rcl.c `emit_rcl`). Three and four targets share one
//! step, so four targets cost nothing over three: 32x32 at 32 bpp, 32x16 at
//! 64 bpp, 16x16 at 128 bpp; the table of this file is Mesa's, not ours.
//!
//! The chooser is `v3d_frame_tiling_bpp` (driver/src/v3d/v3d_frame.c:49-94,
//! `idx = (rts > 2 ? 2 : rts > 1 ? 1 : 0) + ... + max_bpp`) and the bpp
//! field is `job_max_bpp` (driver/src/v3d/v3d_job.c:301-310) written by
//! `v3d_job_begin` into TILE_BINNING_MODE_CFG (body bits 8-11 count minus
//! one, 12-13 bpp) and by `v3d_job_end` into TILE_RENDERING_MODE_CFG_COMMON
//! (body bits 4-7 count minus one, 40-41 bpp; v3d_packet.xml:943,996). Both
//! files are built here as a shared object of their own and driven directly,
//! no GL stack and no HOST_DRAW_TARGETS: the job builder takes four targets
//! (`rts` 4) whatever DRAW_TARGETS says. The GL-level walk is
//! tests/es3_four_rt_v3d.rs. Result: the chooser and the job read RT 3
//! (no `#[ignore]` needed).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_tiles_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use yakogl::v3d::ffi::*;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The builder's C files as a shared object of this test's own.
fn lib() -> usize {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_mrt4_tiles");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_mrt4_tiles-{}.so", std::process::id()));
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args([
                "driver/src/v3d/v3d_job.c",
                "driver/src/v3d/v3d_frame.c",
                "driver/src/v3d/v3d_tex.c",
                "driver/src/v3d/v3d_tiling_mesa.c",
                "-lm",
                "-o",
            ])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the V3D job builder builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the job builder loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    })
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib() as *mut c_void, c.as_ptr()) };
    assert!(!p.is_null(), "{name} is exported");
    p
}

/// The chooser for a 256x256 frame: the tile it answers.
fn tile(rts: u32, msaa: bool, max_bpp: u32) -> (u32, u32) {
    type F = unsafe extern "C" fn(u32, u32, u32, bool, u32, *mut V3dTiling);
    // SAFETY: the C function's signature.
    let f: F = unsafe { std::mem::transmute::<*mut c_void, F>(sym("v3d_frame_tiling_bpp")) };
    // SAFETY: V3dTiling is plain integers.
    let mut t: V3dTiling = unsafe { std::mem::zeroed() };
    // SAFETY: as above, `t` the out-parameter.
    unsafe { f(256, 256, rts, msaa, max_bpp, &mut t) };
    (t.tile_w, t.tile_h)
}

/// Mesa's `tile_sizes`, v3d_util.c:133-140.
const MESA: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];

/// Mesa's V3D 4.x rule, v3d_util.c:177-200, written out.
fn mesa_tile(count: u32, msaa: bool, bpp: u32) -> (u32, u32) {
    let by_count = if count > 4 {
        3
    } else if count > 2 {
        2
    } else if count > 1 {
        1
    } else {
        0
    };
    MESA[(by_count + if msaa { 2 } else { 0 } + bpp) as usize]
}

const FMT32: u32 = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;
const FMT64: u32 = V3D_OUTPUT_IMAGE_FORMAT_RGBA16F;
const FMT128: u32 = V3D_OUTPUT_IMAGE_FORMAT_RGBA32I;

/// Run a job of `fmts.len()` typed targets through the real builder and
/// read the two packets back: (binning count - 1, binning bpp, common count
/// - 1, common bpp).
fn job_fields(fmts: &[u32]) -> (u32, u32, u32, u32) {
    type Begin = unsafe extern "C" fn(*mut V3dJob, *const V3dJobTarget, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool;
    type End = unsafe extern "C" fn(*mut V3dJob, *const V3dJobClear, *mut u32, *mut u32) -> bool;
    // SAFETY: the C functions' signatures.
    let (begin, end): (Begin, End) = unsafe {
        (
            std::mem::transmute::<*mut c_void, Begin>(sym("v3d_job_begin")),
            std::mem::transmute::<*mut c_void, End>(sym("v3d_job_end")),
        )
    };
    let (w, h) = (256u32, 256u32);
    let mut t =
        V3dJobTarget { width: w, height: h, address: 0x100000, stride: w * 16, typed: true, ..Default::default() };
    t.tile_alloc = 0x800000;
    t.rts = fmts.len() as u32;
    for (n, f) in fmts.iter().enumerate() {
        t.rt_format[n] = *f;
        if n > 0 {
            t.rt[n - 1] = V3dJobRt { address: 0x100000 * (n as u32 + 1), stride: w * 16, ..Default::default() };
        }
    }
    let mut bufs = [vec![0u8; 1 << 16], vec![0u8; 1 << 16], vec![0u8; 1 << 16]];
    let mut cls: Vec<V3dCl> = bufs
        .iter_mut()
        .enumerate()
        .map(|(i, b)| V3dCl::new(b.as_mut_ptr(), 0x1000000 + (i as u32) * 0x100000, b.len() as u32))
        .collect();
    // SAFETY: a plain-data C struct, then filled by v3d_job_begin.
    let mut j: V3dJob = unsafe { std::mem::zeroed() };
    let c = V3dJobClear::default();
    let p = cls.as_mut_ptr();
    // SAFETY: three distinct live lists, a live target.
    let ok = unsafe { begin(&mut j, &t, p, p.add(1), p.add(2)) };
    assert!(ok, "v3d_job_begin takes {} targets", fmts.len());
    // SAFETY: the job `begin` made.
    let ok = unsafe { end(&mut j, &c, std::ptr::null_mut(), std::ptr::null_mut()) };
    assert!(ok, "v3d_job_end takes {} targets", fmts.len());
    let (bcl, rcl) = (&bufs[0][..cls[0].used as usize], &bufs[1][..cls[1].used as usize]);
    let body = |l: &[u8], code: u8, ok: &dyn Fn(u64) -> bool| -> u64 {
        for i in 0..l.len().saturating_sub(8) {
            if l[i] == code {
                let mut b = [0u8; 8];
                b.copy_from_slice(&l[i + 1..i + 9]);
                let v = u64::from_le_bytes(b);
                if ok(v) {
                    return v;
                }
            }
        }
        panic!("packet {code} not found");
    };
    // TILE_BINNING_MODE_CFG: width - 1 at bits 32-47, height - 1 at 48-63.
    let bin = body(bcl, 120, &|v| (v >> 32) & 0xffff == 255 && (v >> 48) == 255);
    // TILE_RENDERING_MODE_CFG_COMMON: sub-id 0, width at bits 8-23.
    let com = body(rcl, 121, &|v| v & 0xf == 0 && (v >> 8) & 0xffff == 256 && (v >> 24) & 0xffff == 256);
    (((bin >> 8) & 0xf) as u32, ((bin >> 12) & 3) as u32, ((com >> 4) & 0xf) as u32, ((com >> 40) & 3) as u32)
}

#[test]
fn chooser_matches_mesa_for_every_count_and_bpp() {
    for rts in 1..=4 {
        for bpp in 0..=2 {
            for msaa in [false, true] {
                assert_eq!(tile(rts, msaa, bpp), mesa_tile(rts, msaa, bpp), "rts {rts} bpp {bpp} msaa {msaa}");
            }
        }
    }
}

/// The three-target values and the four-target ones side by side.
#[test]
fn four_targets_take_the_three_target_tile_at_each_bpp() {
    for (bpp, want) in [(0, (32, 32)), (1, (32, 16)), (2, (16, 16))] {
        assert_eq!(tile(3, false, bpp), want, "3 targets, bpp enum {bpp}");
        assert_eq!(tile(4, false, bpp), want, "4 targets, bpp enum {bpp}");
    }
    // one and two targets are the steps above
    assert_eq!(tile(1, false, 0), (64, 64));
    assert_eq!(tile(2, false, 0), (64, 32));
    assert_eq!(tile(4, false, 1), (32, 16));
}

#[test]
fn job_fields_four_targets_of_one_width() {
    assert_eq!(job_fields(&[FMT32; 4]), (3, 0, 3, 0), "4 x 32 bpp");
    assert_eq!(job_fields(&[FMT64; 4]), (3, 1, 3, 1), "4 x 64 bpp");
    assert_eq!(job_fields(&[FMT128; 4]), (3, 2, 3, 2), "4 x 128 bpp");
}

/// The field is the maximum over all four targets, RT 3 included.
#[test]
fn job_fields_mixed_widths_take_the_maximum() {
    assert_eq!(job_fields(&[FMT32, FMT32, FMT32, FMT64]), (3, 1, 3, 1), "only RT 3 is 64 bpp");
    assert_eq!(job_fields(&[FMT32, FMT32, FMT32, FMT128]), (3, 2, 3, 2), "only RT 3 is 128 bpp");
    assert_eq!(job_fields(&[FMT128, FMT32, FMT32, FMT32]), (3, 2, 3, 2), "only RT 0 is 128 bpp");
    assert_eq!(job_fields(&[FMT32, FMT64, FMT32, FMT32]), (3, 1, 3, 1), "only RT 1 is 64 bpp");
    assert_eq!(job_fields(&[FMT32, FMT64, FMT128, FMT32]), (3, 2, 3, 2), "32, 64, 128, 32");
    assert_eq!(job_fields(&[FMT64, FMT32, FMT64, FMT32]), (3, 1, 3, 1), "64, 32, 64, 32");
}

/// An sRGB8_ALPHA8 or RGB10_A2 target is a 64-bpp tile buffer (16F halves,
/// v3d_job.c rt_internal), so four of them are the 32x16 tile.
#[test]
fn job_fields_srgb_targets_count_as_64_bpp() {
    assert_eq!(job_fields(&[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8; 4]), (3, 1, 3, 1));
    assert_eq!(job_fields(&[FMT32, FMT32, FMT32, V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2]), (3, 1, 3, 1));
}

/// Three targets beside the four: the same bpp, the count differs.
#[test]
fn job_fields_three_targets_for_comparison() {
    assert_eq!(job_fields(&[FMT32; 3]), (2, 0, 2, 0));
    assert_eq!(job_fields(&[FMT32, FMT32, FMT64]), (2, 1, 2, 1));
    assert_eq!(job_fields(&[FMT32, FMT128, FMT32]), (2, 2, 2, 2));
}

/// The tile the hardware takes from the packet's count and bpp equals the
/// chooser's answer for the job: the four-target values.
#[test]
fn job_fields_give_the_tile_of_mesas_rule() {
    for (fmts, bpp, want) in [
        (vec![FMT32; 4], 0, (32, 32)),
        (vec![FMT64; 4], 1, (32, 16)),
        (vec![FMT128; 4], 2, (16, 16)),
        (vec![FMT32, FMT32, FMT32, FMT128], 2, (16, 16)),
        (vec![FMT32, FMT64, FMT32, FMT32], 1, (32, 16)),
    ] {
        let (bc, bb, cc, cb) = job_fields(&fmts);
        assert_eq!((bc + 1, cc + 1, bb, cb), (4, 4, bpp, bpp));
        assert_eq!(tile(bc + 1, false, bb), want);
        assert_eq!(mesa_tile(bc + 1, false, bb), want);
    }
}
