// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The render image a Pi 4 render target is sampled from (src/v3d/backend.rs
//! `RtImage`, docs/online-shaderc/v3d-backend.md "Render targets sampled
//! from their render image"), checked against the real layout code the
//! board runs: `driver/src/v3d/v3d_tex.c`, Mesa's v3d_setup_slices
//! transcribed, built here with `cc` and loaded with `dlopen` as
//! tests/v3d_glmark2_mem.rs builds its shim (the lib tests fake the C).
//!
//! What is pinned, and why:
//! * The image of glmark2 refract's 1600x1200 colour target sampled at one
//!   level is one UIF level of exactly 7,680,000 bytes, no padding, no XOR:
//!   the raster storage's size. The 7,684,096-byte tiled copies of
//!   tools/cts/RESULTS.md ("Pi 4: the OSAL looks at the job in flight",
//!   option (a)) were this plus the backend's 4 KiB of alignment slack.
//! * Refract samples that target through its 11-level chain
//!   (LINEAR_MIPMAP_LINEAR after a glGenerateMipmap every frame): the chain's
//!   image is Mesa's 11-level layout, derived by hand below (11,001,856
//!   bytes, level 0 at 3,321,856), and the TFU job that makes levels 1..10
//!   from level 0 (`v3d_tex_tfu_pack_mipmap`, Mesa's v3d_generate_mipmap)
//!   is pinned register by register from Mesa's v3d_tfu.h constants.
//! * The numbers the TLB store and the TMU state take from the layout (the
//!   memory format, the height in UIF blocks, the texture state's UIF bits)
//!   agree between the two readers of one image, for the sizes the three
//!   boards' benchmarks render into and for the small ones where Mesa's
//!   layout is not UIF.
//! * Dropping the XOR (`texture::rt_layout`'s rule, as the depth/stencil
//!   buffers drop it) changes the tiling word alone: every offset, stride and
//!   padded height is Mesa's.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::ffi::{c_char, c_int, c_void, CString};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The shim over the real C: a level's slice, the texture state's
/// layout-derived fields, and the mipmap TFU job, as flat words.
const SHIM: &str = r#"
#include "v3d_tex.h"
#include <string.h>

/* out[0..6]: the layout's size and level `level`'s offset, stride, padded
 * height, ub_pad and tiling; 0 when the layout refuses the size. */
int shim_slice(uint32_t w, uint32_t h, uint32_t levels, uint32_t level, uint32_t out[6])
{
	struct v3d_tex_layout l;
	if (!v3d_tex_layout(w, h, levels, 4, &l) || level >= levels)
		return 0;
	out[0] = l.size;
	out[1] = l.slices[level].offset;
	out[2] = l.slices[level].stride;
	out[3] = l.slices[level].padded_height;
	out[4] = l.slices[level].ub_pad;
	out[5] = (uint32_t)l.slices[level].tiling;
	return 1;
}

/* The texture state's layout-derived fields for a one-level layout whose
 * level 0 tiling is forced to `tiling`: out[0] strictly UIF, out[1] XOR
 * enable, out[2] ub_pad, out[3] the array stride in 64-byte units. */
int shim_state(uint32_t w, uint32_t h, uint32_t tiling, uint32_t out[4])
{
	struct v3d_tex_layout l;
	if (!v3d_tex_layout(w, h, 1, 4, &l))
		return 0;
	l.slices[0].tiling = (enum v3d_tex_tiling)tiling;
	struct v3d_tex_state s;
	memset(&s, 0, sizeof(s));
	v3d_tex_state_from_layout(&s, &l);
	out[0] = s.level_0_is_strictly_uif;
	out[1] = s.level_0_xor_enable;
	out[2] = s.level_0_ub_pad;
	out[3] = s.array_stride_64;
	return 1;
}

/* The mipmap TFU job of a `levels`-level RGBA8 image at `image`, level 0's
 * XOR dropped as the render image drops it: out[0..8] = icfg, iia, iis,
 * ica, iua, ioa, ios, coef0; 0 when refused. */
int shim_mip(uint32_t w, uint32_t h, uint32_t levels, uint32_t image, uint32_t out[8])
{
	struct v3d_tex_layout l;
	if (!v3d_tex_layout(w, h, levels, 4, &l))
		return 0;
	if (l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR)
		l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
	struct v3d_tex_tfu_job j;
	if (!v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_RGBA8, image, &j))
		return 0;
	out[0] = j.icfg;
	out[1] = j.iia;
	out[2] = j.iis;
	out[3] = j.ica;
	out[4] = j.iua;
	out[5] = j.ioa;
	out[6] = j.ios;
	out[7] = j.coef0;
	return 1;
}
"#;

fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_rt_sample");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let src = dir.join(format!("shim-{}.c", std::process::id()));
        let so = dir.join(format!("shim-{}.so", std::process::id()));
        std::fs::write(&src, SHIM).expect("the shim source");
        /* the compiler's own temporary files next to the shim: a host whose
         * /tmp is full (no inodes left, as a ccfarm worker's was) must not
         * fail the layout check */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .arg(&src)
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the C shim over v3d_tex.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the shim loads");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in the shim");
    p
}

/// Mesa's `enum v3d_tiling_mode` numbering (v3d_tex.h, also the load/store
/// packets' "Memory Format").
const LT: u32 = 1;
const UB1: u32 = 2;
const UB2: u32 = 3;
const UIF: u32 = 4;
const UIFX: u32 = 5;

/// Level `level` of the real layout: (size, offset, stride, padded height,
/// ub_pad, tiling).
fn slice(w: u32, h: u32, levels: u32, level: u32) -> [u32; 6] {
    // SAFETY: shim_slice's C signature.
    let f: extern "C" fn(u32, u32, u32, u32, *mut u32) -> c_int = unsafe { std::mem::transmute(sym("shim_slice")) };
    let mut out = [0u32; 6];
    assert_eq!(f(w, h, levels, level, out.as_mut_ptr()), 1, "v3d_tex_layout refused {w}x{h} x{levels}");
    out
}

/// Level 0 of the real layout: (size, offset, stride, padded height, ub_pad,
/// tiling).
fn level0(w: u32, h: u32, levels: u32) -> [u32; 6] {
    slice(w, h, levels, 0)
}

/// The texture state's layout bits for a one-level layout tiled `tiling`:
/// (strictly UIF, XOR enable, ub_pad, array stride / 64).
fn state(w: u32, h: u32, tiling: u32) -> [u32; 4] {
    // SAFETY: shim_state's C signature.
    let f: extern "C" fn(u32, u32, u32, *mut u32) -> c_int = unsafe { std::mem::transmute(sym("shim_state")) };
    let mut out = [0u32; 4];
    assert_eq!(f(w, h, tiling, out.as_mut_ptr()), 1, "v3d_tex_layout refused {w}x{h}");
    out
}

/// The mipmap TFU job of a chain image at `image`: `Some([icfg, iia, iis,
/// ica, iua, ioa, ios, coef0])`, or `None` when the packer refuses it.
fn mip_job(w: u32, h: u32, levels: u32, image: u32) -> Option<[u32; 8]> {
    // SAFETY: shim_mip's C signature.
    let f: extern "C" fn(u32, u32, u32, u32, *mut u32) -> c_int = unsafe { std::mem::transmute(sym("shim_mip")) };
    let mut out = [0u32; 8];
    (f(w, h, levels, image, out.as_mut_ptr()) == 1).then_some(out)
}

/// The TLB store's (memory format, height in UB or stride) for a render
/// image: src/v3d/backend.rs `zs_layout` for 4 bytes a pixel (Mesa's
/// v3d_surface_get_height_in_ub_or_stride, v3dx_rcl.c:51-70), with the XOR
/// dropped as `texture::rt_layout` drops it.
fn tlb(padded_height: u32, tiling: u32) -> (u32, u32) {
    let fmt = if tiling == UIFX { UIF } else { tiling };
    (fmt, if fmt == UIF { padded_height / 8 } else { 0 })
}

/// Refract's colour target at one level: one UIF level the size of the
/// raster storage, no padding, no XOR; the 7,684,096-byte copies were this
/// and 4 KiB.
#[test]
fn refracts_1600x1200_render_image_is_the_raster_storages_size() {
    let [size, off, stride, ph, pad, tiling] = level0(1600, 1200, 1);
    assert_eq!((size, off), (7_680_000, 0));
    assert_eq!((stride, ph, pad, tiling), (6400, 1200, 0, UIF));
    assert_eq!(size + 4096, 7_684_096, "RESULTS' tiled-copy request for this target");
    assert_eq!(tlb(ph, tiling), (UIF, 150), "the TLB stores 150 UIF block rows");
    assert_eq!(state(1600, 1200, UIF), [1, 0, 0, 7_680_000 / 64], "strictly UIF, no XOR, no pad");
    /* and the storage the GL core keeps for it is the same bytes */
    assert_eq!(yakogl::v3d::texture::gl_level_layered(1600, 1200, 1, 4, 1).0, 7_680_000);
}

/// Refract samples its colour target through its 11-level chain: the chain
/// image is Mesa's 11-level layout. By hand from v3d_setup_slices (4-byte
/// texels: utile 4x4, UIF block 8x8, page-cache 32 UIF block rows; the
/// power-of-two padding of levels 2.. from level 1's 800x600: 2048x2048):
/// level 10 2x2 LT 64 B at 0; 9 4x4 LT 64 at 64; 8 8x8 UB1 256 at 128;
/// 7 16x16 UB2 1024 at 384; 6 32x32 UIF (4 rows) 4096 at 1408; 5 64x64
/// 16384 at 5504; 4 128x128 65536 at 21888; 3 256x256, 32 rows is 0 mod 32,
/// "perfectly aligned for UIF XOR": no pad, XOR, 262144 at 87424; 2
/// 512x512, 64 rows, XOR likewise, 1048576 at 349568; 1 800x600, 75 rows
/// (11 mod 32: no pad, no XOR), 1920000 at 1398144, its total page-aligned
/// to 1921024 (wider than 32 and taller than 26 rows); 0 1600x1200, 150
/// rows (22 mod 32), 7680000 at 3319168; then level 0 to a page: +2688
/// everywhere, 11,001,856 bytes in all, level 0 at 3,321,856 and level 1
/// at 1,400,832. (A first derivation padded levels 2 and 3 by 6 rows each,
/// 147,456 bytes more; v3d_get_ub_pad pads nothing at a multiple of the
/// page cache's 32 rows and picks the XOR instead, tex_check's 256x256.)
#[test]
fn refracts_11_level_chain_image_is_mesas_layout() {
    let [size, off, stride, ph, pad, tiling] = level0(1600, 1200, 11);
    assert_eq!(size, 11_001_856);
    assert_eq!((off, stride, ph, pad, tiling), (3_321_856, 6400, 1200, 0, UIF));
    assert_eq!(off % 4096, 0, "level 0 on a page");
    assert_eq!(slice(1600, 1200, 11, 1)[1..], [1_400_832, 3200, 600, 0, UIF]);
    assert_eq!(slice(1600, 1200, 11, 2)[1..], [349_568 + 2688, 2048, 512, 0, UIFX]);
    assert_eq!(slice(1600, 1200, 11, 3)[1..], [87_424 + 2688, 1024, 256, 0, UIFX]);
    assert_eq!(slice(1600, 1200, 11, 7)[1..], [384 + 2688, 64, 16, 0, UB2]);
    assert_eq!(slice(1600, 1200, 11, 10)[1..], [2688, 16, 4, 0, LT]);
    /* the levels below 0 sit at the same distances from level 0 whatever
     * the count the layout was asked for (the TMU walks down from level 0),
     * so an image laid out with the chain serves a sample of fewer levels */
    for levels in [2, 3, 6] {
        let base = level0(1600, 1200, levels)[1];
        for level in 1..levels {
            assert_eq!(
                base - slice(1600, 1200, levels, level)[1],
                off - slice(1600, 1200, 11, level)[1],
                "level {level} of a {levels}-level layout"
            );
        }
    }
}

/// The TFU job that makes refract's chain from level 0 of its image
/// (`v3d_tex_tfu_pack_mipmap`), register by register from Mesa's v3d_tfu.h
/// and v3dx_tfu.c (for_mipmap, base level 0, last level 10): ICFG's format
/// is the source's own tiling, V3D33_TFU_ICFG_FORMAT_UIF_NO_XOR (14) at
/// bit 18, TTYPE RGBA8 (4) at bit 9, NUMMM 10 at bit 5, OPAD 0 at bit 22
/// (1200 rows are 150 UIF blocks, no padding); IIA level 0; IIS its 150 UIF
/// block rows; IOA level 0 with IOA_FORMAT_UIF_NO_XOR (6) at bit 3 and
/// DIMTW (bit 0); IOS 1200 << 16 | 1600; ICA, IUA and COEF0 0. Read from
/// mesa-26.2.1-full's v3dx_tfu.c:97-163 and v3d_tfu.h in the dependency
/// layout.
#[test]
fn refracts_chain_is_one_tfu_job_from_level_0_of_its_image() {
    let image = 0x0100_0000;
    let level0 = image + 3_321_856;
    let j = mip_job(1600, 1200, 11, image).expect("packed");
    assert_eq!(j[0], 14 << 18 | 4 << 9 | 10 << 5, "icfg 0x{:x}", j[0]);
    assert_eq!(j[1], level0, "iia");
    assert_eq!(j[2], 150, "iis");
    assert_eq!((j[3], j[4], j[7]), (0, 0, 0), "ica, iua, coef0");
    assert_eq!(j[5], level0 | 6 << 3 | 1, "ioa 0x{:x}", j[5]);
    assert_eq!(j[6], 1200 << 16 | 1600, "ios");
    /* terrain's 800x600 targets, 10 levels: UIF level 0, 75 rows */
    let t = mip_job(800, 600, 10, image).expect("packed");
    assert_eq!(t[0], 14 << 18 | 4 << 9 | 9 << 5);
    assert_eq!((t[2], t[6]), (75, 600 << 16 | 800));
    assert_eq!(t[5] & 0x3f, 6 << 3 | 1);
    /* a small target whose level 0 is UB-linear: the format words follow
     * its tiling (UB2: 13 and 5) and the height word is 0 */
    let s = mip_job(16, 16, 3, image).expect("packed");
    assert_eq!((s[0] >> 18, s[2], s[5] & 0x3f), (13, 0, 5 << 3 | 1));
    /* the Pi 4 window's 1280x800: Mesa pads level 0 by two UIF block rows
     * (100 rows, 100 % 32 = 4), and v3dx_tfu.c:154-162 puts a UIF
     * destination's padding rows in OPAD (ICFG bits 22..) with DIMTW set as
     * without; IIS counts the padded rows */
    let p = mip_job(1280, 800, 11, image).expect("packed");
    assert_eq!(p[0], 2 << 22 | 14 << 18 | 4 << 9 | 10 << 5, "icfg 0x{:x}", p[0]);
    assert_eq!(p[2], 102, "iis: 816 padded rows in UIF blocks");
    /* refused: one level (nothing to make), an image whose level 0 would
     * fall into IOA's low bits */
    assert!(mip_job(1600, 1200, 1, image).is_none());
    assert!(mip_job(16, 16, 3, image + 32).is_none());
}

/// The benchmarks' other target sizes: the 800x600 canvas (terrain,
/// desktop at glmark2's default), the three boards' windows should an
/// application render into a window-sized texture, and small targets, where
/// Mesa's layout is UB-linear or LT and the TLB's height word is 0.
#[test]
fn the_board_sizes_lay_out_as_mesa_and_the_tlb_and_tmu_words_agree() {
    /* (w, h): size, stride, padded height, ub_pad, tiling, by Mesa's rules */
    let cases: [(u32, u32, u32, u32, u32, u32, u32); 7] = [
        /* 800: 25 columns of 32 texels; 75 UB rows, 75 % 32 = 11: no pad */
        (800, 600, 1_920_000, 3200, 600, 0, UIF),
        /* 1024x600: 32 columns; 75 rows: as above */
        (1024, 600, 2_457_600, 4096, 600, 0, UIF),
        /* 1280x800: 40 columns; 100 rows, 100 % 32 = 4 < 6 and 100 >= 32:
         * pad 2 rows (to 102), 102 % 32 != 0: no XOR */
        (1280, 800, 5120 * 816, 5120, 816, 2, UIF),
        /* 1920x720: 60 columns; 90 rows, 90 % 32 = 26, not > 26: no pad */
        (1920, 720, 7680 * 720, 7680, 720, 0, UIF),
        /* 1024x1024: 128 rows, 128 % 32 == 0: Mesa's XOR */
        (1024, 1024, 4096 * 1024, 4096, 1024, 0, UIFX),
        /* 16x16: two UIF blocks wide, UB-linear 2 columns */
        (16, 16, 1024, 64, 16, 0, UB2),
        /* 4x4: one utile, LT */
        (4, 4, 64, 16, 4, 0, LT),
    ];
    for (w, h, size, stride, ph, pad, tiling) in cases {
        let got = level0(w, h, 1);
        assert_eq!(got, [size, 0, stride, ph, pad, tiling], "{w}x{h}");
        let (fmt, hub) = tlb(ph, tiling);
        let st = state(w, h, fmt);
        /* the TMU's state of the image the TLB stored: UIF iff the TLB
         * stored UIF, never the XOR, the same pad */
        assert_eq!(st[0], u32::from(fmt == UIF), "{w}x{h}: strictly UIF");
        assert_eq!(st[1], 0, "{w}x{h}: the XOR is never set on a render image");
        assert_eq!(st[2], pad, "{w}x{h}: the state's ub_pad");
        assert_eq!(hub, if fmt == UIF { ph / 8 } else { 0 }, "{w}x{h}: the TLB's height in UB");
        let _ = UB1;
    }
    /* the XOR dropped changes the tiling word alone: 1024x1024 with XOR
     * has the same numbers, and its state would set the XOR bit */
    assert_eq!(state(1024, 1024, UIFX)[1], 1, "Mesa's layout would enable the XOR");
    assert_eq!(state(1024, 1024, UIF)[..3], [1, 0, 0], "the render image does not");
}
