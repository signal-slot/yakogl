// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! stage71: depth and packed texture storage, and the transfer function and
//! R/B swap per revision, in src/v3d/format71.rs and texture71.rs, pinned
//! against Mesa 26.2.1's vendored format table and packet XML and the 7.1
//! references (tools/v3d/pi5/gl_texfmt.clif, via README.md's record table).
//!
//! Run: cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texfix

#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;

use format71::*;
use texture71::*;

fn tex(internal_format: u32, ty: u32, size: u32) -> GlTex71 {
    GlTex71 {
        target: Target::Tex2d,
        internal_format,
        ty,
        width: size,
        height: size,
        levels: 1,
        base_level: 0,
        max_level: GL_MAX_LEVEL_DEFAULT,
        swizzle: GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: 0,
        ver: 71,
        iprev: 10,
    }
}

/// Fix 1: the Z32F+S8 pair's texture half is the Z32F buffer
/// (v3dx_format_table.c:155-156); `compose` still refuses it by name.
#[test]
fn fix1_depth32f_stencil8_texture_half_is_z32f() {
    let t = tex(0x8CAD, 0, 64);
    assert_eq!(compose(&t).unwrap_err(), Refusal::SeparateStencil { pipe: "Z32_FLOAT_S8X24_UINT" });
    let half = compose_depth_half(&t).unwrap();
    let z = compose(&tex(0x8CAC, 0, 64)).unwrap();
    assert_eq!(half, z);
    assert_eq!(half.row.pipe, "Z32_FLOAT");
    assert_eq!(half.layout.cpp, 4);
    assert!(half.return_32);
    // DEPTH_COMP32F is texture type 23 (v3dx_format_table.c:152 and :156)
    assert_eq!(half.tex_type, 23);
    // the stencil-only rows stay without a texture size
    for p in ["S8X24_UINT", "X32_S8X24_UINT"] {
        assert_eq!(texel_bytes(p), None);
    }
    // every other format is its own half
    for f in [0x8058, 0x81A6, 0x88F0, 0x8CAC] {
        assert_eq!(depth_texture_half(f), f);
    }
}

/// Fix 2: texel bytes are an explicit table: one word for the packed and
/// depth formats, the channel digits for array formats.
#[test]
fn fix2_packed_and_depth_texel_bytes() {
    for (pipe, bytes) in [
        ("A4B4G4R4_UNORM", 2),
        ("A1B5G5R5_UNORM", 2),
        ("X1B5G5R5_UNORM", 2),
        ("B5G6R5_UNORM", 2),
        ("R10G10B10A2_UNORM", 4),
        ("R10G10B10A2_UINT", 4),
        ("R11G11B10_FLOAT", 4),
        ("R9G9B9E5_FLOAT", 4),
        ("Z16_UNORM", 2),
        ("X8Z24_UNORM", 4),
        ("S8_UINT_Z24_UNORM", 4),
        ("Z32_FLOAT", 4),
        ("Z32_FLOAT_S8X24_UINT", 4),
    ] {
        assert_eq!(texel_bytes(pipe), Some(bytes), "{pipe}");
        assert!(mesa_row(pipe).is_some(), "{pipe}");
    }
    assert_eq!(texel_bytes("NO_SUCH"), None);
    assert_eq!(texel_bytes("ETC2_RGB8"), None);
    for r in MESA_ROWS.iter() {
        if let Some(b) = texel_bytes(r.pipe) {
            assert!([1, 2, 4, 8, 16].contains(&b), "{}", r.pipe);
            if r.pipe != "Z32_FLOAT_S8X24_UINT" {
                assert_eq!(cpp_of(r.pipe), Ok(b), "{}", r.pipe);
            }
        }
    }
    // a depth texture's storage through the composition
    let d16 = compose(&tex(0x81A5, 0, 64)).unwrap();
    assert_eq!((d16.layout.cpp, d16.tex_type, d16.return_32), (2, 21, true));
    let d24 = compose(&tex(0x88F0, 0, 64)).unwrap();
    assert_eq!((d24.layout.cpp, d24.tex_type, d24.row.pipe), (4, 24, "S8_UINT_Z24_UNORM"));
    let r565 = compose(&tex(0x8D62, 0x8363, 64)).unwrap();
    assert_eq!((r565.layout.cpp, r565.tex_type, r565.r_b_swap), (2, 6, false));
}

/// Fix 4: the transfer function is the sRGB pipe format's (gl_texfmt job 1
/// word 0 `0x0000f004`, bit 2); the R/B swap bit is 0 on the GL path at every
/// revision (gl_texfmt job 2) and exists in the Vulkan-style form only from
/// 7.1.5.
#[test]
fn fix4_transfer_and_rb_swap_per_revision() {
    let srgb = compose(&tex(0x8C43, 0x1401, 64)).unwrap();
    assert_eq!(srgb.transfer_func, 1);
    assert_eq!(compose(&tex(0x8058, 0x1401, 64)).unwrap().transfer_func, 0);
    // SKIP_DECODE views the sRGB format as linear
    assert_eq!(compose(&GlTex71 { srgb_decode: false, ..tex(0x8C43, 0x1401, 64) }).unwrap().transfer_func, 0);
    for r in MESA_ROWS.iter() {
        let row = row71(r.pipe);
        assert_eq!(row.transfer == Transfer::Srgb, r.pipe.contains("SRGB"), "{}", r.pipe);
        assert!(!row.r_b_swap && !row.reverse, "{}", r.pipe);
    }
    for iprev in 5..=12 {
        let t = compose(&GlTex71 { iprev, ..tex(0x8058, 0x1401, 64) }).unwrap();
        assert!(!t.r_b_swap && !t.reverse);
    }
    // 7.1.2 has no swap bit and a different stride field: refused by name
    assert!(compose(&GlTex71 { iprev: 2, ..tex(0x8058, 0x1401, 64) }).is_err());
    // the Vulkan-style form: BGRA's ZYXW as the bit + XYZW, 7.1.5 and later
    let bgra = row71("B8G8R8A8_UNORM");
    assert_eq!(bgra.swizzle, [SWIZZLE_BLUE, SWIZZLE_GREEN, SWIZZLE_RED, SWIZZLE_ALPHA]);
    let v = rb_swap_form(&bgra, 10).unwrap();
    assert!(v.r_b_swap);
    assert_eq!(v.swizzle, [SWIZZLE_RED, SWIZZLE_GREEN, SWIZZLE_BLUE, SWIZZLE_ALPHA]);
    assert_eq!(v.tex_type, bgra.tex_type);
    let x = rb_swap_form(&row71("B8G8R8X8_SRGB"), 5).unwrap();
    assert_eq!((x.swizzle[3], x.transfer), (SWIZZLE_ONE, Transfer::Srgb));
    assert_eq!(rb_swap_form(&bgra, 4), None);
    assert_eq!(rb_swap_form(&row71("R8G8B8A8_UNORM"), 10), None);
    assert_eq!(rb_swap_form(&row71("R8_UNORM"), 10), None);
}
