// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/rt71.rs` (the V3D 7.1 render-target config and `Clear Render
//! Targets` values per GL colour format) against Mesa 26.2.1 and the Mesa
//! 7.1 references on the tree.
//!
//! * the packet field positions are parsed from
//!   `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml` (the
//!   `min_ver="71"` definitions of `Tile Rendering Mode Cfg (Render Target
//!   Part1/2/3)`, `Clear Render Targets` and the `Render Target Type Clamp`
//!   enum) and compared with the module's constants as text;
//! * the internal type and bpp of every GL format are compared with the
//!   `v3dX(get_internal_type_bpp_for_output_format)` switch parsed from
//!   `v3dx_format_table.c`, and the 7.1 type-and-clamp with Mesa's rule
//!   (integer types clamped, sRGB 16F with norm clamping);
//! * the config and clear values the module gives are compared with the
//!   `TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1` blocks decoded in
//!   `tools/v3d/pi5/clear.clif`, `triangle.clif` and `gl_*.clif`, whose
//!   clear colours and formats are the GL calls of
//!   `tools/v3d/pi5/src/gl_shapes.py` (the shapes' colours are written out
//!   below next to the call that sets them).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_rt
//! ```
//! (the module and this test use no crate feature.)
#[path = "../src/v3d/rt71.rs"]
mod rt71;

use rt71::*;
use std::collections::BTreeMap;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ---- the xml ---------------------------------------------------------------

/// The body of the first `<packet ... name="NAME" ...>` whose tag has
/// `min_ver="71"`, as its tag line and field lines.
fn packet_body<'a>(xml: &'a str, name: &str) -> (&'a str, &'a str) {
    let key = format!("name=\"{name}\"");
    let mut pos = 0;
    while let Some(i) = xml[pos..].find("<packet ") {
        let start = pos + i;
        let tag_end = start + xml[start..].find('>').unwrap();
        let tag = &xml[start..=tag_end];
        if tag.contains(&key) && tag.contains("min_ver=\"71\"") {
            if tag.ends_with("/>") {
                return (tag, "");
            }
            let end = tag_end + xml[tag_end..].find("</packet>").unwrap();
            return (tag, &xml[tag_end + 1..end]);
        }
        pos = tag_end;
    }
    panic!("no 7.1 packet {name:?} in the xml");
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let i = line.find(&key)? + key.len();
    Some(&line[i..i + line[i..].find('"')?])
}

/// `(name, start, size)` of every `<field>` of a packet body.
fn fields(body: &str) -> Vec<(String, u32, u32)> {
    body.lines()
        .filter(|l| l.trim_start().starts_with("<field "))
        .map(|l| {
            (
                attr(l, "name").unwrap().to_string(),
                attr(l, "start").unwrap().parse().unwrap(),
                attr(l, "size").unwrap().parse().unwrap(),
            )
        })
        .collect()
}

fn xml() -> String {
    read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml")
}

fn check_part(name: &str, mine: &[(&str, u32, u32)], sub_id: u64, code: u8) {
    let x = xml();
    let (tag, body) = packet_body(&x, name);
    assert_eq!(attr(tag, "code"), Some(code.to_string().as_str()), "{name}: code");
    let mut theirs = fields(body);
    let mut mine: Vec<(String, u32, u32)> = mine.iter().map(|&(n, s, z)| (n.to_string(), s, z)).collect();
    theirs.sort();
    mine.sort();
    assert_eq!(theirs, mine, "{name}: field positions");
    let sub = body.lines().find(|l| l.contains("name=\"sub-id\"")).unwrap();
    assert_eq!(attr(sub, "default"), Some(sub_id.to_string().as_str()), "{name}: sub-id");
    // the payload is 64 bits: the fields tile it (bit 6 of part1/2/3 and a
    // few bits are unused)
    let top = mine.iter().map(|f| f.1 + f.2).max().unwrap();
    assert_eq!(top, 64, "{name}: the fields end at bit 64");
}

#[test]
fn part_fields_match_the_xml() {
    check_part(
        "Tile Rendering Mode Cfg (Render Target Part1)",
        PART1_FIELDS,
        PART1_SUB_ID,
        TILE_RENDERING_MODE_CFG_CODE,
    );
    check_part(
        "Tile Rendering Mode Cfg (Render Target Part2)",
        PART2_FIELDS,
        PART2_SUB_ID,
        TILE_RENDERING_MODE_CFG_CODE,
    );
    check_part(
        "Tile Rendering Mode Cfg (Render Target Part3)",
        PART3_FIELDS,
        PART3_SUB_ID,
        TILE_RENDERING_MODE_CFG_CODE,
    );
}

#[test]
fn clear_rt_is_one_byte_without_fields() {
    let x = xml();
    let (tag, body) = packet_body(&x, "Clear Render Targets");
    assert_eq!(attr(tag, "code"), Some(CLEAR_RT_CODE.to_string().as_str()));
    assert!(tag.ends_with("/>") && body.is_empty(), "the packet has fields: {tag}");
    // 4.2's packet of the same job step is gone at 7.1
    assert!(x.contains("name=\"Clear Tile Buffers\"") && x.contains("max_ver=\"42\""));
}

/// An `<enum name="NAME">`'s `(value name, value)` rows.
fn enum_values(name: &str) -> BTreeMap<String, u32> {
    let x = xml();
    let i = x.find(&format!("<enum name=\"{name}\"")).unwrap();
    let end = i + x[i..].find("</enum>").unwrap();
    x[i..end]
        .lines()
        .filter(|l| l.trim_start().starts_with("<value "))
        .map(|l| (attr(l, "name").unwrap().to_string(), attr(l, "value").unwrap().parse().unwrap()))
        .collect()
}

#[test]
fn type_clamp_and_bpp_values_match_the_xml() {
    let e = enum_values("Render Target Type Clamp");
    for (name, tc) in [
        ("8i", TypeClamp::I8),
        ("16i", TypeClamp::I16),
        ("32i", TypeClamp::I32),
        ("8ui", TypeClamp::Ui8),
        ("16ui", TypeClamp::Ui16),
        ("32ui", TypeClamp::Ui32),
        ("8", TypeClamp::Type8),
        ("16f", TypeClamp::F16),
        ("32f", TypeClamp::F32),
        ("8i_clamped", TypeClamp::I8Clamped),
        ("16i_clamped", TypeClamp::I16Clamped),
        ("32i_clamped", TypeClamp::I32Clamped),
        ("8ui_clamped", TypeClamp::Ui8Clamped),
        ("16ui_clamped", TypeClamp::Ui16Clamped),
        ("32ui_clamped", TypeClamp::Ui32Clamped),
        ("16f_clamp_norm", TypeClamp::F16ClampNorm),
    ] {
        assert_eq!(e[name], tc as u32, "{name}");
    }
    let b = enum_values("Internal BPP");
    assert_eq!((b["32"], b["64"], b["128"]), (BPP_32, BPP_64, BPP_128));
}

// ---- Mesa's internal type and bpp per output format ---------------------------

/// `v3dX(get_internal_type_bpp_for_output_format)`: output format name to
/// (internal type name, bpp name), parsed from the switch.
fn internal_rows() -> BTreeMap<String, (String, String)> {
    let src = read("third_party/mesa-26.2.1/src/gallium/drivers/v3d/v3dx_format_table.c");
    let i = src.find("v3dX(get_internal_type_bpp_for_output_format)").unwrap();
    let body = &src[i..i + src[i..].find("tfu_supports_tex_format").unwrap()];
    let mut rows = BTreeMap::new();
    let mut names: Vec<String> = Vec::new();
    for l in body.lines() {
        let l = l.trim();
        if let Some(n) = l.strip_prefix("case V3D_OUTPUT_IMAGE_FORMAT_") {
            names.push(n.trim_end_matches(':').to_string());
        } else if let Some(t) = l.strip_prefix("*type = V3D_INTERNAL_TYPE_") {
            let ty = t.trim_end_matches(';').to_string();
            for n in &names {
                rows.insert(n.clone(), (ty.clone(), String::new()));
            }
        } else if let Some(b) = l.strip_prefix("*bpp = V3D_INTERNAL_BPP_") {
            let bpp = b.trim_end_matches(';').to_string();
            for n in names.drain(..) {
                rows.get_mut(&n).unwrap().1 = bpp.clone();
            }
        }
    }
    rows
}

/// `(GL internal format, GL name, Mesa output image format, integer, sRGB)`
/// for every colour format the table renders, written from the GL spec and
/// Mesa's `rt` column independently of the module.
const RENDERABLE: &[(u32, &str, &str, bool, bool)] = &[
    (0x8058, "RGBA8", "RGBA8", false, false),
    (0x8051, "RGB8", "RGBA8", false, false),
    (0x822B, "RG8", "RG8", false, false),
    (0x8229, "R8", "R8", false, false),
    (0x8D62, "RGB565", "BGR565", false, false),
    (0x8056, "RGBA4", "ABGR4444", false, false),
    (0x8057, "RGB5_A1", "ABGR1555", false, false),
    (0x8C43, "SRGB8_ALPHA8", "SRGB8_ALPHA8", false, true),
    (0x8059, "RGB10_A2", "RGB10_A2", false, false),
    (0x906F, "RGB10_A2UI", "RGB10_A2UI", true, false),
    (0x881A, "RGBA16F", "RGBA16F", false, false),
    (0x822F, "RG16F", "RG16F", false, false),
    (0x822D, "R16F", "R16F", false, false),
    (0x8C3A, "R11F_G11F_B10F", "R11F_G11F_B10F", false, false),
    (0x8814, "RGBA32F", "RGBA32F", false, false),
    (0x8230, "RG32F", "RG32F", false, false),
    (0x822E, "R32F", "R32F", false, false),
    (0x8D8E, "RGBA8I", "RGBA8I", true, false),
    (0x8237, "RG8I", "RG8I", true, false),
    (0x8231, "R8I", "R8I", true, false),
    (0x8D7C, "RGBA8UI", "RGBA8UI", true, false),
    (0x8238, "RG8UI", "RG8UI", true, false),
    (0x8232, "R8UI", "R8UI", true, false),
    (0x8D88, "RGBA16I", "RGBA16I", true, false),
    (0x8239, "RG16I", "RG16I", true, false),
    (0x8233, "R16I", "R16I", true, false),
    (0x8D76, "RGBA16UI", "RGBA16UI", true, false),
    (0x823A, "RG16UI", "RG16UI", true, false),
    (0x8234, "R16UI", "R16UI", true, false),
    (0x8D82, "RGBA32I", "RGBA32I", true, false),
    (0x823B, "RG32I", "RG32I", true, false),
    (0x8235, "R32I", "R32I", true, false),
    (0x8D70, "RGBA32UI", "RGBA32UI", true, false),
    (0x823C, "RG32UI", "RG32UI", true, false),
    (0x8236, "R32UI", "R32UI", true, false),
];

#[test]
fn every_format_matches_mesas_internal_type_and_bpp() {
    let rows = internal_rows();
    assert!(rows.len() > 30, "parsed {} output formats", rows.len());
    for &(gl, name, out, integer, srgb) in RENDERABLE {
        let (ty, bpp) = &rows[out];
        let rt = rt_format71(gl).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let want_bpp = match bpp.as_str() {
            "32" => BPP_32,
            "64" => BPP_64,
            "128" => BPP_128,
            b => panic!("{b}"),
        };
        assert_eq!(rt.bpp, want_bpp, "{name} bpp");
        assert_eq!(rt.integer, integer, "{name} integer");
        // Mesa's 7.1 clamp_for_format_and_type: integer types clamped, an
        // sRGB format 16F with norm clamping, the rest unclamped
        let want = match (ty.as_str(), integer, srgb) {
            ("8", false, _) => TypeClamp::Type8,
            ("16F", false, true) => TypeClamp::F16ClampNorm,
            ("16F", false, false) => TypeClamp::F16,
            ("32F", false, _) => TypeClamp::F32,
            ("8I", true, _) => TypeClamp::I8Clamped,
            ("8UI", true, _) => TypeClamp::Ui8Clamped,
            ("16I", true, _) => TypeClamp::I16Clamped,
            ("16UI", true, _) => TypeClamp::Ui16Clamped,
            ("32I", true, _) => TypeClamp::I32Clamped,
            ("32UI", true, _) => TypeClamp::Ui32Clamped,
            t => panic!("{name}: {t:?}"),
        };
        assert_eq!(rt.type_clamp, want, "{name} type and clamp");
    }
}

#[test]
fn unrenderable_formats_are_refused_by_name() {
    // depth and stencil
    for gl in [0x81A5, 0x81A6, 0x8CAC, 0x88F0, 0x8CAD, 0x8D48] {
        assert_eq!(rt_format71(gl), Err(Refusal71::NotAColourFormat), "{gl:#x}");
    }
    // SNORM (rt column NO), RGB9_E5, SRGB8, three-channel integers and floats
    for gl in [0x8F97, 0x8F96, 0x8F95, 0x8F94, 0x8C3D, 0x8C41, 0x8D8F, 0x8D7D, 0x8D89, 0x8D83, 0x881B, 0x8815] {
        assert_eq!(rt_format71(gl), Err(Refusal71::FormatNotRenderable), "{gl:#x}");
    }
    for gl in [0, 0x1908, 0xdead] {
        assert_eq!(rt_format71(gl), Err(Refusal71::UnknownFormat), "{gl:#x}");
    }
    for e in [
        Refusal71::FormatNotRenderable,
        Refusal71::NotAColourFormat,
        Refusal71::UnknownFormat,
        Refusal71::TooManyTargets,
        Refusal71::TargetTooLarge,
        Refusal71::EmptyTarget,
        Refusal71::ClearKindMismatch,
        Refusal71::TargetNumberOutOfRange,
    ] {
        assert!(e.as_str().starts_with("v3d 7.1: "));
    }
}

#[test]
fn limits() {
    assert_eq!((MAX_RENDER_TARGETS, MAX_SIZE), (8, 7680));
    for n in 0..=8 {
        assert_eq!(check_targets(n), Ok(()));
    }
    assert_eq!(check_targets(9), Err(Refusal71::TooManyTargets));
    assert_eq!(check_size(7680, 7680), Ok(()));
    assert_eq!(check_size(1, 1), Ok(()));
    assert_eq!(check_size(7681, 64), Err(Refusal71::TargetTooLarge));
    assert_eq!(check_size(64, 7681), Err(Refusal71::TargetTooLarge));
    assert_eq!(check_size(0, 64), Err(Refusal71::EmptyTarget));
    // the limits' source (v3d_limits.h:44, :63)
    let l = read("third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h");
    assert!(l.contains("(ver < 71 ? 4 : 8)") || l.contains("ver < 71 ? 4 : 8"), "render targets");
    assert!(l.contains("7680"), "framebuffer size");
}

// ---- the clear words -----------------------------------------------------------

fn fl(c: [f32; 4]) -> ClearColor {
    ClearColor::Float(c)
}

#[test]
fn unorm8_clear_words() {
    let rt = rt_format71(0x8058).unwrap();
    let w = |c| clear_words71(&rt, fl(c)).unwrap();
    assert_eq!(w([0.0, 0.0, 1.0, 1.0]).low, 0xffff0000);
    assert_eq!(w([0.0, 0.0, 0.0, 1.0]).low, 0xff000000);
    assert_eq!(w([1.0; 4]).low, 0xffffffff);
    assert_eq!(w([1.0, 0.0, 0.0, 0.25]).low, 0x400000ff);
    // half rounds to even: 0.5 * 255 = 127.5 -> 128; 1.5/255 -> 1.5 -> 2
    assert_eq!(w([0.5, 1.5 / 255.0, 0.0, 0.0]).low, 0x0000_0280);
    // out of range clamps; NaN is 0
    assert_eq!(w([2.0, -1.0, f32::NAN, 1.0]).low, 0xff0000ff);
    let r = w([0.0, 0.0, 1.0, 1.0]);
    assert_eq!((r.words, r.mid, r.top), ([0xffff0000, 0, 0, 0], 0, 0));
}

#[test]
fn float_clear_words() {
    // R16F ... RGBA16F: halves at 64 bpp, two to a word, r low
    let rt = rt_format71(0x881A).unwrap();
    let r = clear_words71(&rt, fl([1.0, 0.5, -2.0, 65504.0])).unwrap();
    assert_eq!(r.words, [0x3800_3c00, 0x7bff_c000, 0, 0]);
    assert_eq!(r.low, 0x3800_3c00);
    assert_eq!(r.mid, 0x7bff_c000);
    assert_eq!(r.top, 0);
    // overflow goes to infinity, a float surface keeps values past 1
    let r = clear_words71(&rt, fl([65520.0, f32::INFINITY, 0.0, 0.0])).unwrap();
    assert_eq!(r.words[0], 0x7c00_7c00);
    // sRGB and RGB10_A2: clamped to [0, 1] first
    for gl in [0x8C43, 0x8059] {
        let rt = rt_format71(gl).unwrap();
        let r = clear_words71(&rt, fl([2.0, -1.0, 0.5, f32::NAN])).unwrap();
        assert_eq!(r.words, [0x0000_3c00, 0x0000_3800, 0, 0], "{gl:#x}");
    }
    // 32F: the floats' bits, as many words as the pixel has, 0 past them
    let c = [1.0f32, 2.0, 3.0, 4.0];
    let b = c.map(f32::to_bits);
    assert_eq!(clear_words71(&rt_format71(0x822E).unwrap(), fl(c)).unwrap().words, [b[0], 0, 0, 0]);
    assert_eq!(clear_words71(&rt_format71(0x8230).unwrap(), fl(c)).unwrap().words, [b[0], b[1], 0, 0]);
    let r = clear_words71(&rt_format71(0x8814).unwrap(), fl(c)).unwrap();
    assert_eq!(r.words, b);
}

#[test]
fn integer_clear_words() {
    let sint = |gl, c: [i32; 4]| clear_words71(&rt_format71(gl).unwrap(), ClearColor::Int(c)).unwrap();
    let uint = |gl, c: [u32; 4]| clear_words71(&rt_format71(gl).unwrap(), ClearColor::Uint(c)).unwrap();
    // 8I / 8UI: the low bytes, one word
    assert_eq!(sint(0x8D8E, [-1, 2, -3, 4]).words, [0x04fd_02ff, 0, 0, 0]);
    assert_eq!(uint(0x8D7C, [0x1ff, 2, 3, 4]).words, [0x0403_02ff, 0, 0, 0]);
    // 16I / 16UI at 64 bpp: two words; at 32 bpp (R16I, RG16I) one
    assert_eq!(sint(0x8D88, [-1, 2, -3, 4]).words, [0x0002_ffff, 0x0004_fffd, 0, 0]);
    assert_eq!(sint(0x8239, [-1, 2, 7, 8]).words, [0x0002_ffff, 0, 0, 0]);
    assert_eq!(uint(0x906F, [1023, 3, 0, 0]).words, [0x0003_03ff, 0, 0, 0]);
    // 32I / 32UI: the raw words, one, two or four
    assert_eq!(sint(0x8235, [-5, 6, 7, 8]).words, [0xffff_fffb, 0, 0, 0]);
    assert_eq!(sint(0x823B, [-5, 6, 7, 8]).words, [0xffff_fffb, 6, 0, 0]);
    let r = uint(0x8D70, [0x1111_1111, 0x2222_2222, 0x3344_5566, 0x7788_99aa]);
    assert_eq!(r.words, [0x1111_1111, 0x2222_2222, 0x3344_5566, 0x7788_99aa]);
    // the three packet fields: word 1 + low byte of word 2; the rest of
    // word 2 + word 3
    assert_eq!(r.low, 0x1111_1111);
    assert_eq!(r.mid, 0x66_2222_2222);
    assert_eq!(r.top, 0x77_8899_aa33_4455);
    assert!(r.mid >> 40 == 0 && r.top >> 56 == 0);
}

#[test]
fn clear_kind_must_match_the_target() {
    let int = rt_format71(0x8D8E).unwrap();
    let flt = rt_format71(0x8058).unwrap();
    assert_eq!(clear_words71(&int, fl([0.0; 4])), Err(Refusal71::ClearKindMismatch));
    assert_eq!(clear_words71(&flt, ClearColor::Int([0; 4])), Err(Refusal71::ClearKindMismatch));
    assert_eq!(clear_words71(&flt, ClearColor::Uint([0; 4])), Err(Refusal71::ClearKindMismatch));
}

// ---- the packet words ------------------------------------------------------------

/// Bits `start..start + size` of a payload.
fn bits(p: u64, start: u32, size: u32) -> u64 {
    (p >> start) & (u64::MAX >> (64 - size))
}

#[test]
fn packets_decode_to_what_went_in() {
    let rt = rt_format71(0x8D82).unwrap(); // RGBA32I: 128 bpp, 32I clamped
    let c = clear_words71(&rt, ClearColor::Int([-1, 2, 0x1234_5678, -4])).unwrap();
    let p1 = part1_payload(5, &rt, c.low, 128, 1234).unwrap();
    let get = |p, n: &str, f: &[(&str, u32, u32)]| {
        let &(_, s, z) = f.iter().find(|x| x.0 == n).unwrap();
        bits(p, s, z)
    };
    assert_eq!(get(p1, "sub-id", PART1_FIELDS), 2);
    assert_eq!(get(p1, "Render Target number", PART1_FIELDS), 5);
    assert_eq!(get(p1, "Base Address", PART1_FIELDS), 1234);
    assert_eq!(get(p1, "Stride", PART1_FIELDS), 127, "stored minus one");
    assert_eq!(get(p1, "Internal BPP", PART1_FIELDS), 2);
    assert_eq!(get(p1, "Internal Type and Clamping", PART1_FIELDS), 18);
    assert_eq!(get(p1, "Clear Color low bits", PART1_FIELDS), 0xffff_ffff);
    let p2 = part2_payload(5, c.mid).unwrap();
    assert_eq!(get(p2, "sub-id", PART2_FIELDS), 3);
    assert_eq!(get(p2, "Render Target number", PART2_FIELDS), 5);
    assert_eq!(get(p2, "Clear Color mid bits", PART2_FIELDS), c.mid);
    let p3 = part3_payload(5, c.top).unwrap();
    assert_eq!(get(p3, "sub-id", PART3_FIELDS), 4);
    assert_eq!(get(p3, "Clear Color top bits", PART3_FIELDS), c.top);
    // the whole colour is recoverable from the three fields
    let w2 = (c.mid >> 32) as u32 | ((c.top & 0xff_ffff) as u32) << 8;
    let w3 = (c.top >> 24) as u32;
    assert_eq!([c.low, c.mid as u32, w2, w3], c.words);
    // nine bytes: code 121, then the payload low byte first
    let b = packet_bytes(p1);
    assert_eq!(b[0], 121);
    assert_eq!(u64::from_le_bytes(b[1..].try_into().unwrap()), p1);
    // range checks
    assert!(part1_payload(8, &rt, 0, 32, 0).is_err());
    assert!(part1_payload(0, &rt, 0, 0, 0).is_err());
    assert!(part1_payload(0, &rt, 0, 129, 0).is_err());
    assert!(part1_payload(0, &rt, 0, 32, 2048).is_err());
    assert!(part2_payload(0, 1 << 40).is_err());
    assert!(part3_payload(0, 1 << 56).is_err());
}

#[test]
fn stride_and_base_follow_the_tile() {
    assert_eq!(bpp_words(BPP_32), 1);
    assert_eq!(bpp_words(BPP_64), 2);
    assert_eq!(bpp_words(BPP_128), 4);
    assert_eq!(rt_stride(64, BPP_32), 32);
    assert_eq!(rt_stride(32, BPP_32), 16);
    assert_eq!(rt_stride(64, BPP_128), 128);
    assert_eq!(rt_base_step(32, 16), 64);
}

// ---- the Mesa 7.1 references --------------------------------------------------------

#[derive(Debug)]
struct Part1 {
    nrts: u32,
    tile_w: u32,
    tile_h: u32,
    number: u32,
    base: u32,
    stride: u32,
    bpp: u32,
    type_clamp: u32,
    clear_low: u32,
}

fn clif_value(l: &str) -> Option<(&str, &str)> {
    let l = l.strip_prefix("MESA: info:")?.trim();
    let rest = l.strip_prefix("/*")?;
    let (k, v) = rest.split_once("*/")?;
    Some((k.trim().trim_end_matches(':'), v.split_whitespace().next()?))
}

/// Every `TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1` of a `.clif`, with the
/// target count and tile size of the common packet before it.
fn clif_part1(file: &str) -> Vec<Part1> {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let (mut nrts, mut tw, mut th) = (0, 0, 0);
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(l) = lines.next() {
        if let Some((k, v)) = clif_value(l) {
            match k {
                "Number of Render Targets" => nrts = v.parse().unwrap(),
                "Log2 Tile Width" => tw = 8 << v.parse::<u32>().unwrap(),
                "Log2 Tile Height" => th = 8 << v.parse::<u32>().unwrap(),
                _ => {}
            }
        }
        if l.trim_end().ends_with("TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1") {
            let mut p = Part1 {
                nrts,
                tile_w: tw,
                tile_h: th,
                number: 0,
                base: 0,
                stride: 0,
                bpp: 0,
                type_clamp: 0,
                clear_low: 0,
            };
            while let Some((k, v)) = lines.peek().and_then(|l| clif_value(l)) {
                let n: u64 = v.parse().unwrap();
                match k {
                    "Render Target number" => p.number = n as u32,
                    "Base Address" => p.base = n as u32,
                    "Stride" => p.stride = n as u32,
                    "Internal BPP" => p.bpp = n as u32,
                    "Internal Type and Clamping" => p.type_clamp = n as u32,
                    "Clear Color low bits" => p.clear_low = n as u32,
                    k => panic!("{file}: unexpected field {k:?}"),
                }
                lines.next();
            }
            out.push(p);
        }
    }
    out
}

const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0; 4];
const RED_Q: [f32; 4] = [1.0, 0.0, 0.0, 0.25];

/// What each reference's render lists hold, in file order: `Some(colour)` for
/// a job that starts from a glClear of the whole target (gl_shapes.py's
/// glClearColor), `None` for a job that loads the target instead (a scissored
/// or masked clear is a draw, a later job of the frame reads the earlier
/// one's colour), whose clear colour the builder writes as 0.
fn expected(file: &str) -> Vec<Option<[f32; 4]>> {
    let b = Some(BLUE);
    match file {
        // 64x64 RGBA8 FBO, glClearColor(0, 0, 1, 1)
        "clear" | "triangle" => vec![b],
        // black clear, then a scissored clear and a quad that load
        "gl_scissor" => vec![Some(BLACK), None],
        // three jobs, each a blue clear first
        "gl_scisbox" => vec![b, b, b],
        // blue, blue, blue, then the stencil job that loads the colour
        "gl_writemask" => vec![b, b, b, None],
        // white; blue (quarter viewport); a scissored green clear that loads
        "gl_viewport" => vec![Some(WHITE), b, None],
        // case_fbods frames 1..8 and the stencil steps
        "gl_fbods" => vec![b, b, Some(RED_Q), b, None, b, b, b, None],
        // black, R+A masked (loads), black, G only (loads), black, B only (loads)
        "gl_colormask" => vec![Some(BLACK), None, Some(BLACK), None, Some(BLACK), None],
        // one blue clear of n targets
        "gl_mrt4" => vec![b; 4],
        "gl_mrt8" => vec![b; 8],
        f => panic!("{f}"),
    }
}

#[test]
fn the_references_render_target_parts() {
    let rgba8 = rt_format71(0x8058).unwrap();
    let files = [
        "clear",
        "triangle",
        "gl_scissor",
        "gl_scisbox",
        "gl_writemask",
        "gl_viewport",
        "gl_fbods",
        "gl_colormask",
        "gl_mrt4",
        "gl_mrt8",
    ];
    let mut total = 0;
    for f in files {
        let parts = clif_part1(f);
        let want = expected(f);
        assert_eq!(parts.len(), want.len(), "{f}: render target configs");
        for (i, (p, w)) in parts.iter().zip(&want).enumerate() {
            let at = format!("{f} #{i}");
            // every target of these shapes is an RGBA8 texture or window
            assert_eq!(p.type_clamp, rgba8.type_clamp as u32, "{at}: type and clamping");
            assert_eq!(p.bpp, rgba8.bpp, "{at}: bpp");
            let low = match w {
                Some(c) => clear_words71(&rgba8, fl(*c)).unwrap().low,
                None => 0,
            };
            assert_eq!(p.clear_low, low, "{at}: clear colour low bits");
            // the stride (the packet's own minus-one undone in the clif) and
            // the base of the n-th target in the job
            let stride = rt_stride(p.tile_w, rgba8.bpp);
            assert_eq!(p.stride, stride, "{at}: stride (tile {}x{})", p.tile_w, p.tile_h);
            let n = p.number;
            assert_eq!(p.base, n * rt_base_step(p.tile_h, stride), "{at}: base of target {n}");
            assert!(check_targets(p.nrts).is_ok());
            // the packet the module builds decodes to the clif's numbers
            let pay = part1_payload(n, &rgba8, low, p.stride, p.base).unwrap();
            let get = |name: &str| {
                let &(_, s, z) = PART1_FIELDS.iter().find(|x| x.0 == name).unwrap();
                bits(pay, s, z)
            };
            assert_eq!(get("Clear Color low bits"), p.clear_low as u64);
            assert_eq!(get("Stride") + 1, p.stride as u64);
            assert_eq!(get("Internal BPP"), p.bpp as u64);
            assert_eq!(get("Internal Type and Clamping"), p.type_clamp as u64);
            total += 1;
        }
    }
    assert_eq!(total, 1 + 1 + 2 + 3 + 4 + 3 + 9 + 6 + 4 + 8);
}

#[test]
fn the_mrt_references_use_every_target_number() {
    for (f, n) in [("gl_mrt4", 4u32), ("gl_mrt8", 8)] {
        let parts = clif_part1(f);
        assert!(parts.iter().all(|p| p.nrts == n), "{f}");
        let numbers: Vec<u32> = parts.iter().map(|p| p.number).collect();
        assert_eq!(numbers, (0..n).collect::<Vec<_>>(), "{f}");
        assert_eq!(check_targets(n), Ok(()));
    }
    // 32x32 tiles at 8 targets: base steps of 64
    let p = clif_part1("gl_mrt8");
    assert_eq!((p[0].tile_w, p[0].tile_h), (32, 32));
    assert_eq!(p[7].base, 7 * 64);
}

#[test]
fn the_references_clear_rt_packets() {
    // every reference's tile list ends its clear with CLEAR_RT, never the 4.2
    // store bit or CLEAR_TILE_BUFFERS
    for f in ["clear", "triangle", "gl_scissor", "gl_mrt8"] {
        let t = read(&format!("tools/v3d/pi5/{f}.clif"));
        assert!(t.lines().any(|l| l.trim_end().ends_with("MESA: info: CLEAR_RT")), "{f}");
        assert!(!t.contains("CLEAR_TILE_BUFFERS"), "{f}");
        assert!(!t.contains("Clear buffer being stored: */ 1"), "{f}: the store clear bit");
    }
}
