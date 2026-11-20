// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/format71.rs` (the V3D 7.1 TMU format answers per GL format)
//! against Mesa 26.2.1's own table.
//!
//! The vendored `v3dx_format_table.c` and `cle/v3d_packet.xml` are parsed
//! live (no cc harness: the file needs Mesa's generated pipe-format and
//! packet headers, which are not vendored). The checks:
//!
//! * `MESA_ROWS` is the file's `FORMAT(...)` rows, line by line, with
//!   TEXTURE_DATA_FORMAT numbers from the xml;
//! * every GL format of an independent list (GL enum, type, pipe format
//!   name) answers with that row's type, swizzle and return size, sRGB
//!   exactly for the sRGB pipe formats, no R/B swap and no reverse;
//! * the refusals are the named variants, and no GL format of the list
//!   falls to a default;
//! * the 4.2 answers `src/v3d/texture.rs` `tmu_format` gives (read from its
//!   source) equal the 7.1 table's except where Mesa's table differs.
//!
//! ```text
//! cargo test --no-default-features --features host-test --test v3d71_fmt
//! ```
//! (the module and this test use no crate feature; any set that builds the
//! tests runs it.)
#[path = "../src/v3d/format71.rs"]
mod format71;

use format71::*;
use std::collections::BTreeMap;

const MESA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/mesa-26.2.1/src/");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{MESA}{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// TEXTURE_DATA_FORMAT_<NAME upper-cased> -> value (v3d_packet.xml:328-).
fn xml_types() -> BTreeMap<String, u32> {
    let xml = read("broadcom/cle/v3d_packet.xml");
    let start = xml.find("<enum name=\"Texture Data Formats\">").expect("enum");
    let body = &xml[start..];
    let body = &body[..body.find("</enum>").expect("enum end")];
    let mut m = BTreeMap::new();
    for l in body.lines() {
        let Some(a) = l.find("<value name=\"Texture Data Format ") else { continue };
        let rest = &l[a + "<value name=\"Texture Data Format ".len()..];
        let name = &rest[..rest.find('"').unwrap()];
        let v = rest.split("value=\"").nth(1).unwrap();
        let v: u32 = v[..v.find('"').unwrap()].parse().unwrap();
        m.insert(name.replace(' ', "_").to_uppercase(), v);
    }
    m
}

struct CRow {
    line: u32,
    pipe: String,
    tex: String,
    swiz: String,
    size: u32,
    chans: u32,
}

/// The `FORMAT(pipe, rt, tex, SWIZ_x, size, channels)` rows of the file.
fn c_rows() -> Vec<CRow> {
    let src = read("gallium/drivers/v3d/v3dx_format_table.c");
    // no version conditional: one table for 4.2 and 7.1
    assert!(!src.contains("V3D_VERSION"), "the table became version-dependent: re-read it for 7.1");
    assert!(!src.lines().any(|l| l.trim_start().starts_with("#if")), "the table has a conditional");
    let mut out = Vec::new();
    for (i, l) in src.lines().enumerate() {
        let t = l.trim_start();
        let Some(args) = t.strip_prefix("FORMAT(") else { continue };
        let args = &args[..args.rfind(')').unwrap()];
        let f: Vec<&str> = args.split(',').map(str::trim).collect();
        assert_eq!(f.len(), 6, "line {}", i + 1);
        out.push(CRow {
            line: i as u32 + 1,
            pipe: f[0].to_string(),
            tex: f[2].to_string(),
            swiz: f[3].strip_prefix("SWIZ_").unwrap().to_string(),
            size: f[4].parse().unwrap(),
            chans: f[5].parse().unwrap(),
        });
    }
    out
}

#[test]
fn transcription_is_the_file() {
    let types = xml_types();
    let rows = c_rows();
    assert_eq!(rows.len(), MESA_ROWS.len(), "row count");
    for (c, m) in rows.iter().zip(MESA_ROWS.iter()) {
        let at = format!("v3dx_format_table.c:{} {}", c.line, c.pipe);
        assert_eq!(m.pipe, c.pipe, "{at}");
        assert_eq!(m.line, c.line, "{at}");
        assert_eq!(m.tex, types[&c.tex], "{at}: {}", c.tex);
        assert_eq!(m.swiz, c.swiz, "{at}");
        assert_eq!(m.return_size, c.size, "{at}");
        assert_eq!(m.return_channels, c.chans, "{at}");
    }
}

/// (GL internal format, type, pipe format). Written out here from the GL
/// registry and ES 3.0.6 table 3.2, independent of the module's own match.
const GL_TO_PIPE: &[(u32, u32, &str)] = &[
    (0x1908, 0x1401, "R8G8B8A8_UNORM"),
    (0x1908, 0x8033, "A4B4G4R4_UNORM"),
    (0x1908, 0x8034, "A1B5G5R5_UNORM"),
    (0x1908, 0x1406, "R32G32B32A32_FLOAT"),
    (0x1908, 0x140B, "R16G16B16A16_FLOAT"),
    (0x1908, 0x8D61, "R16G16B16A16_FLOAT"),
    (0x1907, 0x1401, "R8G8B8X8_UNORM"),
    (0x1907, 0x8363, "B5G6R5_UNORM"),
    (0x1909, 0x1401, "L8_UNORM"),
    (0x1906, 0x1401, "A8_UNORM"),
    (0x190A, 0x1401, "L8A8_UNORM"),
    (0x80E1, 0x1401, "B8G8R8A8_UNORM"),
    (0x1902, 0x1403, "Z16_UNORM"),
    (0x1902, 0x1405, "X8Z24_UNORM"),
    (0x1902, 0x1406, "Z32_FLOAT"),
    (0x84F9, 0x84FA, "S8_UINT_Z24_UNORM"),
    (0x84F9, 0x8DAD, "Z32_FLOAT_S8X24_UINT"),
    (0x8229, 0, "R8_UNORM"),
    (0x8F94, 0, "R8_SNORM"),
    (0x822B, 0, "R8G8_UNORM"),
    (0x8F95, 0, "R8G8_SNORM"),
    (0x8051, 0, "R8G8B8X8_UNORM"),
    (0x8F96, 0, "R8G8B8X8_SNORM"),
    (0x8058, 0, "R8G8B8A8_UNORM"),
    (0x8F97, 0, "R8G8B8A8_SNORM"),
    (0x8C41, 0, "R8G8B8X8_SRGB"),
    (0x8C43, 0, "R8G8B8A8_SRGB"),
    (0x8056, 0, "A4B4G4R4_UNORM"),
    (0x8057, 0, "A1B5G5R5_UNORM"),
    (0x8D62, 0, "B5G6R5_UNORM"),
    (0x8059, 0, "R10G10B10A2_UNORM"),
    (0x906F, 0, "R10G10B10A2_UINT"),
    (0x822D, 0, "R16_FLOAT"),
    (0x822F, 0, "R16G16_FLOAT"),
    (0x881A, 0, "R16G16B16A16_FLOAT"),
    (0x822E, 0, "R32_FLOAT"),
    (0x8230, 0, "R32G32_FLOAT"),
    (0x8814, 0, "R32G32B32A32_FLOAT"),
    (0x8C3A, 0, "R11G11B10_FLOAT"),
    (0x8C3D, 0, "R9G9B9E5_FLOAT"),
    (0x8231, 0, "R8_SINT"),
    (0x8232, 0, "R8_UINT"),
    (0x8233, 0, "R16_SINT"),
    (0x8234, 0, "R16_UINT"),
    (0x8235, 0, "R32_SINT"),
    (0x8236, 0, "R32_UINT"),
    (0x8237, 0, "R8G8_SINT"),
    (0x8238, 0, "R8G8_UINT"),
    (0x8239, 0, "R16G16_SINT"),
    (0x823A, 0, "R16G16_UINT"),
    (0x823B, 0, "R32G32_SINT"),
    (0x823C, 0, "R32G32_UINT"),
    (0x8D70, 0, "R32G32B32A32_UINT"),
    (0x8D76, 0, "R16G16B16A16_UINT"),
    (0x8D7C, 0, "R8G8B8A8_UINT"),
    (0x8D82, 0, "R32G32B32A32_SINT"),
    (0x8D88, 0, "R16G16B16A16_SINT"),
    (0x8D8E, 0, "R8G8B8A8_SINT"),
    (0x81A5, 0, "Z16_UNORM"),
    (0x81A6, 0, "X8Z24_UNORM"),
    (0x8CAC, 0, "Z32_FLOAT"),
    (0x88F0, 0, "S8_UINT_Z24_UNORM"),
    (0x8CAD, 0, "Z32_FLOAT_S8X24_UINT"),
    (0x8D64, 0, "ETC2_RGB8"),
    (0x9274, 0, "ETC2_RGB8"),
    (0x9275, 0, "ETC2_SRGB8"),
    (0x9276, 0, "ETC2_RGB8A1"),
    (0x9277, 0, "ETC2_SRGB8A1"),
    (0x9278, 0, "ETC2_RGBA8"),
    (0x9279, 0, "ETC2_SRGBA8"),
    (0x9270, 0, "ETC2_R11_UNORM"),
    (0x9271, 0, "ETC2_R11_SNORM"),
    (0x9272, 0, "ETC2_RG11_UNORM"),
    (0x9273, 0, "ETC2_RG11_SNORM"),
    (0x83F0, 0, "DXT1_RGB"),
    (0x83F1, 0, "DXT1_RGBA"),
    (0x83F2, 0, "DXT3_RGBA"),
    (0x83F3, 0, "DXT5_RGBA"),
    (0x8C4C, 0, "DXT1_SRGB"),
    (0x8C4D, 0, "DXT1_SRGBA"),
    (0x8C4E, 0, "DXT3_SRGBA"),
    (0x8C4F, 0, "DXT5_SRGBA"),
];

/// ASTC: RGBA enum 0x93B0 + n, SRGB8_ALPHA8 enum 0x93D0 + n, in this block order.
const ASTC: [&str; 14] =
    ["4x4", "5x4", "5x5", "6x5", "6x6", "8x5", "8x6", "8x8", "10x5", "10x6", "10x8", "10x10", "12x10", "12x12"];

fn all_gl() -> Vec<(u32, u32, String)> {
    let mut v: Vec<(u32, u32, String)> = GL_TO_PIPE.iter().map(|&(a, b, c)| (a, b, c.to_string())).collect();
    for (i, b) in ASTC.iter().enumerate() {
        v.push((0x93B0 + i as u32, 0, format!("ASTC_{b}")));
        v.push((0x93D0 + i as u32, 0, format!("ASTC_{b}_SRGB")));
    }
    v
}

#[test]
fn every_gl_format_answers_with_its_mesa_row() {
    let rows = c_rows();
    let types = xml_types();
    for (gl, ty, pipe) in all_gl() {
        let at = format!("GL {gl:#06x}/{ty:#06x} -> {pipe}");
        let c = rows.iter().find(|r| r.pipe == pipe).unwrap_or_else(|| panic!("{at}: no Mesa row"));
        let a = tmu_format71(gl, ty).unwrap_or_else(|e| panic!("{at}: refused {e:?}"));
        let cite = format!("{at} (v3dx_format_table.c:{})", c.line);
        assert_eq!(a.pipe, pipe, "{cite}");
        assert_eq!(a.mesa_line, c.line, "{cite}");
        assert_eq!(a.tex_type, types[&c.tex], "{cite}: type {}", c.tex);
        let want: Vec<u8> = c
            .swiz
            .bytes()
            .map(|b| match b {
                b'X' => SWIZZLE_RED,
                b'Y' => SWIZZLE_GREEN,
                b'Z' => SWIZZLE_BLUE,
                b'W' => SWIZZLE_ALPHA,
                b'0' => SWIZZLE_ZERO,
                b'1' => SWIZZLE_ONE,
                _ => panic!("swizzle {}", c.swiz),
            })
            .collect();
        assert_eq!(a.swizzle.to_vec(), want, "{cite}: swizzle {}", c.swiz);
        assert_eq!(a.return_32, c.size == 32, "{cite}: return size {}", c.size);
        let srgb = c.pipe.contains("SRGB");
        assert_eq!(a.transfer, if srgb { Transfer::Srgb } else { Transfer::None }, "{cite}");
        // gallium composes the swizzle: no R/B swap, no reverse (v3d_tex71.h)
        assert!(!a.r_b_swap && !a.reverse, "{cite}");
    }
}

#[test]
fn srgb_is_exactly_the_srgb_formats() {
    let srgb: Vec<u32> = all_gl().iter().filter(|g| g.2.contains("SRGB")).map(|g| g.0).collect();
    // SRGB8, SRGB8_ALPHA8, ETC2 x3, S3TC x4, ASTC x14
    assert_eq!(srgb.len(), 2 + 3 + 4 + 14);
    for (gl, ty, _) in all_gl() {
        let a = tmu_format71(gl, ty).unwrap();
        assert_eq!(a.transfer == Transfer::Srgb, srgb.contains(&gl), "{gl:#x}");
    }
}

#[test]
fn bgrx_egl_image_is_mesas_b8g8r8x8() {
    let a = bgrx8_egl_image();
    assert_eq!((a.pipe, a.mesa_line), ("B8G8R8X8_UNORM", 62));
    assert_eq!(a.tex_type, 4);
    assert_eq!(a.swizzle, [SWIZZLE_BLUE, SWIZZLE_GREEN, SWIZZLE_RED, SWIZZLE_ONE]);
    assert!(!a.return_32);
}

#[test]
fn mesa_line_names_the_pipe_format() {
    let src = read("gallium/drivers/v3d/v3dx_format_table.c");
    let lines: Vec<&str> = src.lines().collect();
    for (gl, ty, _) in all_gl() {
        let a = tmu_format71(gl, ty).unwrap();
        let l = lines[a.mesa_line as usize - 1];
        assert!(l.contains(&format!("FORMAT({},", a.pipe)), "{} line {}: {l}", a.pipe, a.mesa_line);
    }
}

#[test]
fn refusals_are_named() {
    // Mesa has no three-channel 16/32-bit row and no RGB integer row
    for gl in [0x881B, 0x8815, 0x8D7D, 0x8D8F, 0x8D77, 0x8D89, 0x8D71, 0x8D83] {
        assert_eq!(tmu_format71(gl, 0), Err(Refusal71::NoThreeChannelRow), "{gl:#x}");
    }
    // none of those has a pipe row either
    for p in ["R16G16B16_FLOAT", "R32G32B32_FLOAT", "R8G8B8_UINT", "R16G16B16_SINT", "R32G32B32_UINT"] {
        assert!(c_rows().iter().all(|r| r.pipe != p), "{p} became a row");
    }
    assert_eq!(tmu_format71(0x8D48, 0), Err(Refusal71::StencilTexturing));
    // an unsized format with a type no ES table pairs with it
    assert_eq!(tmu_format71(GL_RGBA, GL_UNSIGNED_SHORT_5_6_5), Err(Refusal71::UnsizedTypeNotListed));
    assert_eq!(tmu_format71(GL_RGB, GL_FLOAT), Err(Refusal71::UnsizedTypeNotListed));
    assert_eq!(tmu_format71(GL_DEPTH_COMPONENT, GL_UNSIGNED_BYTE), Err(Refusal71::UnsizedTypeNotListed));
    assert_eq!(tmu_format71(GL_LUMINANCE, 0), Err(Refusal71::UnsizedTypeNotListed));
    // names that are no texture format
    for gl in [0, 1, 0x8D40, 0x9999, 0xFFFF_FFFF] {
        assert_eq!(tmu_format71(gl, 0), Err(Refusal71::UnknownFormat), "{gl:#x}");
    }
}

/// What `tmu_format` in src/v3d/texture.rs answers at 4.2 for a class: the
/// class name, the type constant, the swizzle, and the GL format that class
/// stands for. Transcribed from texture.rs (the `tmu_format` match,
/// `etc2_row`); `texture_rs_still_says_it` checks the source for each.
const TEXTURE_RS: &[(&str, &str, &str, u32, u32)] = &[
    ("GLES_TEXFMT_R16F", "R16F", "X001", 0x822D, 0),
    ("GLES_TEXFMT_RGB565", "RGB565", "XYZ1", 0x8D62, 0),
    ("GLES_TEXFMT_R11G11B10F", "R11F_G11F_B10F", "XYZ1", 0x8C3A, 0),
    ("GLES_TEXFMT_RGB9E5", "RGB9_E5", "XYZ1", 0x8C3D, 0),
    ("GLES_TEXFMT_RGBA16F", "RGBA16F", "XYZW", 0x881A, 0),
    ("GLES_TEXFMT_RGBA32F", "RGBA32F", "XYZW", 0x8814, 0),
    ("GLES_TEXFMT_RGB10A2", "RGB10_A2", "XYZW", 0x8059, 0),
    ("GLES_TEXFMT_RGB10A2UI", "RGB10_A2UI", "XYZW", 0x906F, 0),
    ("GLES_TEXFMT_RGBA8_SNORM", "RGBA8_SNORM", "XYZW", 0x8F97, 0),
    ("GLES_TEXFMT_RGBA8UI", "RGBA8UI", "XYZW", 0x8D7C, 0),
    ("GLES_TEXFMT_RGBA8I", "RGBA8I", "XYZW", 0x8D8E, 0),
    ("GLES_TEXFMT_RGBA16UI", "RGBA16UI", "XYZW", 0x8D76, 0),
    ("GLES_TEXFMT_RGBA16I", "RGBA16I", "XYZW", 0x8D88, 0),
    ("GLES_TEXFMT_RGBA32UI", "RGBA32UI", "XYZW", 0x8D70, 0),
    ("GLES_TEXFMT_RGBA32I", "RGBA32I", "XYZW", 0x8D82, 0),
    ("GLES_TEXFMT_L8", "R8", "XXX1", 0x1909, 0x1401),
    ("GLES_TEXFMT_A8", "R8", "000X", 0x1906, 0x1401),
    ("GLES_TEXFMT_LA8", "RG8", "XXXY", 0x190A, 0x1401),
    ("GLES_TEXFMT_R32F", "R32F", "X001", 0x822E, 0),
    // the default arm: RGBA8, also what GLES_TEXFMT_SRGB8_A8 samples (the sRGB bit is outside tmu_format)
    ("_ =>", "RGBA8", "XYZW", 0x8058, 0),
    // the bgra arm (XRGB8888 EGLImage): B, G, R, ONE
    ("b.bgra", "RGBA8", "ZYX1", 0, 0),
];

/// The ETC2 rows of texture.rs `etc2_row`: variant, type, swizzle, GL enum.
const TEXTURE_RS_ETC2: &[(&str, &str, &str, u32)] = &[
    ("Etc2::Rgb8", "RGB8_ETC2", "XYZ1", 0x9274),
    ("Etc2::Srgb8", "RGB8_ETC2", "XYZ1", 0x9275),
    ("Etc2::Rgb8A1", "RGB8_PUNCHTHROUGH_ALPHA1", "XYZW", 0x9276),
    ("Etc2::Srgb8A1", "RGB8_PUNCHTHROUGH_ALPHA1", "XYZW", 0x9277),
    ("Etc2::Rgba8", "RGBA8_ETC2_EAC", "XYZW", 0x9278),
    ("Etc2::Srgba8", "RGBA8_ETC2_EAC", "XYZW", 0x9279),
    ("Etc2::R11", "R11_EAC", "X001", 0x9270),
    ("Etc2::R11Snorm", "SIGNED_R11_EAC", "X001", 0x9271),
    ("Etc2::Rg11", "RG11_EAC", "XY01", 0x9272),
    ("Etc2::Rg11Snorm", "SIGNED_RG11_EAC", "XY01", 0x9273),
];

fn swz_values(s: &str) -> [u8; 4] {
    let v: Vec<u8> = s
        .bytes()
        .map(|b| match b {
            b'X' => SWIZZLE_RED,
            b'Y' => SWIZZLE_GREEN,
            b'Z' => SWIZZLE_BLUE,
            b'W' => SWIZZLE_ALPHA,
            b'0' => SWIZZLE_ZERO,
            _ => SWIZZLE_ONE,
        })
        .collect();
    [v[0], v[1], v[2], v[3]]
}

fn texture_rs() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/v3d/texture.rs")).expect("texture.rs")
}

#[test]
fn texture_rs_still_says_it() {
    let src = texture_rs();
    // a line names the class and its type constant (texture.rs is read, not changed)
    for (cls, ty, _, _, _) in TEXTURE_RS {
        let want = format!("V3D_TEXTURE_DATA_FORMAT_{ty}");
        if cls.starts_with("GLES_TEXFMT_") {
            assert!(
                src.lines().any(|l| l.contains(&format!("{cls} ")) && l.contains(&want) && l.contains("=>")),
                "{cls}"
            );
        } else if *cls == "b.bgra" {
            assert!(src.lines().any(|l| l.contains("_ if b.bgra != 0") && l.contains(&want)));
        }
    }
    for (e, ty, _, _) in TEXTURE_RS_ETC2 {
        let want = format!("V3D_TEXTURE_DATA_FORMAT_{ty}");
        assert!(src.lines().any(|l| l.contains(&format!("{e} =>")) && l.contains(&want)), "{e}");
    }
}

#[test]
fn texture_rs_4_2_answers_differ_only_where_mesa_differs() {
    let types = xml_types();
    let mut rows: Vec<(String, u32, [u8; 4], u32, u32)> = Vec::new(); // name, type, swizzle, gl, ty
    for (cls, ty, swz, gl, t) in TEXTURE_RS {
        rows.push((cls.to_string(), types[*ty], swz_values(swz), *gl, *t));
    }
    for (e, ty, swz, gl) in TEXTURE_RS_ETC2 {
        rows.push((e.to_string(), types[*ty], swz_values(swz), *gl, 0));
    }
    let mut differs = Vec::new();
    for (name, t, swz, gl, ty) in &rows {
        if *gl == 0 {
            // the EGLImage arm: Mesa's B8G8R8X8_UNORM row
            let a = bgrx8_egl_image();
            assert_eq!((a.tex_type, a.swizzle), (*t, *swz), "{name}");
            continue;
        }
        let a = tmu_format71(*gl, *ty).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        if (a.tex_type, a.swizzle) != (*t, *swz) {
            differs.push(name.clone());
        }
    }
    assert!(differs.is_empty(), "4.2 answers of texture.rs that Mesa's table does not give: {differs:?}");

    // The two places texture.rs answers something Mesa's table has no row for,
    // both documented in its comments, are not in the lists above:
    //  - DEPTH_COMPONENT is an R32F look-up (X001, return 32) whereas Mesa's
    //    depth rows are DEPTH_COMP16 / DEPTH24_X8 / DEPTH_COMP32F, XXXX;
    //  - `depth_luminance` samples that R32F as XXX1 (ES 2.0 OES_depth_texture),
    //    a swizzle Mesa has for no depth row.
    // The 7.1 table answers depth with Mesa's rows (return size 32 for all).
    let src = texture_rs();
    assert!(src.contains("GLES_TEXFMT_R32F if b.depth_luminance != 0"));
    for gl in [0x81A5, 0x81A6, 0x8CAC] {
        let a = tmu_format71(gl, 0).unwrap();
        assert_eq!(a.swizzle, [SWIZZLE_RED; 4], "{gl:#x}");
        assert!(a.return_32, "{gl:#x}");
        assert_ne!(a.tex_type, 29, "Mesa's depth rows are not R32F");
    }
}

#[test]
fn return_32_matches_texture_rs_return_32_classes() {
    // texture.rs return_32: the classes of 32-bit channels are R32F (:91),
    // RGBA32F (:101), RGBA32UI and RGBA32I (:136-137); every other colour
    // format sampled there is 16.
    for (gl, want) in [
        (0x822E, true),
        (0x8814, true),
        (0x8D70, true),
        (0x8D82, true),
        (0x8058, false),
        (0x881A, false),
        (0x822D, false),
        (0x8D7C, false),
        (0x8D76, false),
        (0x8C3A, false),
        (0x8C3D, false),
        (0x8059, false),
        (0x8D62, false),
        (0x8F97, false),
    ] {
        assert_eq!(tmu_format71(gl, 0).unwrap().return_32, want, "{gl:#x}");
    }
}
