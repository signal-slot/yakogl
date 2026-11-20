// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! src/v3d/sampler71.rs: the V3D 7.1 sampler state field values and the
//! 24-byte record, against Mesa's 7.1 SAMPLER_STATE (checked by text) and
//! `driver/src/v3d/v3d_tex71.h` (the struct's field names, by text).
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_sampler`

#[path = "../src/v3d/sampler71.rs"]
mod sampler71;

use sampler71::*;

fn le64(b: &[u8; SAMPLER_BYTES]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

fn word(b: &[u8; SAMPLER_BYTES], k: usize) -> u32 {
    u32::from_le_bytes(b[8 + 4 * k..12 + 4 * k].try_into().unwrap())
}

fn record(i: &Input) -> [u8; SAMPLER_BYTES] {
    pack(&sampler71(i).expect("accepted")).expect("packs")
}

/// The XML's "Sampler State" struct as (name, start, size).
fn xml_fields() -> Vec<(String, u32, u32)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let xml = std::fs::read_to_string(path).expect("v3d_packet.xml");
    let at = xml.find("<struct name=\"Sampler State\">").expect("struct");
    let body = &xml[at..at + xml[at..].find("</struct>").unwrap()];
    let attr = |line: &str, key: &str| -> String {
        let k = format!("{key}=\"");
        let s = line.find(&k).unwrap() + k.len();
        line[s..s + line[s..].find('"').unwrap()].to_string()
    };
    body.lines()
        .filter(|l| l.contains("<field "))
        .map(|l| (attr(l, "name"), attr(l, "start").parse().unwrap(), attr(l, "size").parse().unwrap()))
        .collect()
}

#[test]
fn field_positions_match_the_7_1_xml() {
    let xml = xml_fields();
    assert_eq!(xml.len(), FIELDS.len());
    for (x, f) in xml.iter().zip(FIELDS.iter()) {
        assert_eq!((x.0.as_str(), x.1, x.2), *f);
    }
    // the XML's enum values
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let xml = std::fs::read_to_string(path).unwrap();
    for v in [
        "name=\"0000\" value=\"0\"",
        "name=\"0001\" value=\"1\"",
        "name=\"1111\" value=\"2\"",
        "name=\"Follows\" value=\"7\"",
        "name=\"REPEAT\" value=\"0\"",
        "name=\"CLAMP\" value=\"1\"",
        "name=\"MIRROR\" value=\"2\"",
        "name=\"BORDER\" value=\"3\"",
        "name=\"MIRROR_ONCE\" value=\"4\"",
    ] {
        assert!(xml.contains(v), "{v}");
    }
}

#[test]
fn struct_field_names_match_v3d_tex71_h() {
    let h = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/v3d/v3d_tex71.h")).unwrap();
    let at = h.find("struct v3d_tex71_sampler {").unwrap();
    let body = &h[at..at + h[at..].find("\n};").unwrap()];
    // strip comments, then take the declarators
    let mut text = String::new();
    let mut rest = body["struct v3d_tex71_sampler {".len()..].to_string();
    while let Some(a) = rest.find("/*") {
        text.push_str(&rest[..a]);
        let b = rest[a..].find("*/").unwrap();
        rest = rest[a + b + 2..].to_string();
    }
    text.push_str(&rest);
    let mut c_names = Vec::new();
    for decl in text.split(';') {
        let d = decl.trim();
        if d.is_empty() {
            continue;
        }
        let (_, names) = d.split_once(|c: char| c.is_whitespace()).unwrap();
        for n in names.split(',') {
            let n = n.trim();
            c_names.push(n.split('[').next().unwrap().to_string());
        }
    }
    // the Rust struct's names through its Debug output
    let dbg = format!("{:?}", sampler71(&Input::default_2d()).unwrap());
    let inner = &dbg[dbg.find('{').unwrap() + 1..dbg.rfind('}').unwrap()];
    let mut rs_names = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth == 0 => {
                rs_names.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    rs_names.push(cur);
    let rs_names: Vec<String> = rs_names.iter().map(|s| s.trim().split(':').next().unwrap().to_string()).collect();
    assert_eq!(rs_names, c_names);
}

#[test]
fn nearest_2d() {
    let i = Input::default_2d();
    let s = sampler71(&i).unwrap();
    assert!(s.min_nearest && s.mag_nearest && s.mip_nearest && s.mip_none);
    assert_eq!((s.wrap_s, s.wrap_t, s.wrap_r), (WRAP_REPEAT, WRAP_REPEAT, WRAP_REPEAT));
    assert_eq!((s.compare_func, s.max_anisotropy, s.border_color_mode, s.border_color), (0, 0, BORDER_0000, [0; 4]));
    let r = record(&i);
    // filters nearest (bits 0-2); min LOD clamps to 0, max to the
    // mip-none 1/256 (u4.8: 1)
    assert_eq!(le64(&r) & 0x7, 0x7);
    assert_eq!((le64(&r) >> 8) & 0xfff, 0);
    assert_eq!((le64(&r) >> 20) & 0xfff, 1);
    assert_eq!(&r[8..], &[0u8; 16]);
}

#[test]
fn linear_2d_clamp() {
    let i = Input {
        min_filter: Filter::Linear,
        mag_filter: Filter::Linear,
        wrap_s: Wrap::ClampToEdge,
        wrap_t: Wrap::MirroredRepeat,
        wrap_r: Wrap::MirrorClampToEdge,
        ..Input::default_2d()
    };
    let s = sampler71(&i).unwrap();
    assert!(!s.min_nearest && !s.mag_nearest && s.mip_nearest && s.mip_none);
    let b = le64(&record(&i));
    assert_eq!(b & 0x3, 0, "min and mag filter linear");
    assert_eq!(b >> 48 & 7, 1);
    assert_eq!(b >> 51 & 7, 2);
    assert_eq!(b >> 54 & 7, 4);
}

#[test]
fn trilinear() {
    let i = Input {
        min_filter: Filter::Linear,
        mag_filter: Filter::Linear,
        mip_filter: MipFilter::Linear,
        min_lod: 0.0,
        max_lod: 12.0,
        lod_bias: -0.5,
        ..Input::default_2d()
    };
    let s = sampler71(&i).unwrap();
    assert!(!s.mip_nearest && !s.mip_none);
    let b = le64(&record(&i));
    assert_eq!(b & 0x7, 0);
    assert_eq!(b >> 8 & 0xfff, 0);
    assert_eq!(b >> 20 & 0xfff, 12 * 256);
    assert_eq!((b >> 32 & 0xffff) as u16 as i16, -128);
    // nearest mip filter with mip levels
    let n = Input { mip_filter: MipFilter::Nearest, ..i };
    let s = sampler71(&n).unwrap();
    assert!(s.mip_nearest && !s.mip_none);
    assert_eq!(le64(&record(&n)) & 0x4, 0x4);
    // LODs clamp to 0..15 as Mesa's do, and max is at least min
    let w = Input { min_lod: -1000.0, max_lod: 1000.0, ..i };
    let b = le64(&record(&w));
    assert_eq!((b >> 8 & 0xfff, b >> 20 & 0xfff), (0, 15 * 256));
    let sw = Input { min_lod: 5.0, max_lod: 2.0, ..i };
    let b = le64(&record(&sw));
    assert_eq!((b >> 8 & 0xfff, b >> 20 & 0xfff), (5 * 256, 5 * 256));
}

#[test]
fn anisotropy_steps() {
    for (taps, en, f) in [(0, 0, 0), (1, 1, 0), (2, 1, 0), (3, 1, 1), (4, 1, 1), (8, 1, 2), (16, 1, 3)] {
        let b = le64(&record(&Input { max_anisotropy: taps, ..Input::default_2d() }));
        assert_eq!((b >> 3 & 1, b >> 61 & 3), (en, f), "{taps}");
    }
    assert_eq!(sampler71(&Input { max_anisotropy: 17, ..Input::default_2d() }), Err(Refusal::AnisotropyOutOfRange));
}

#[test]
fn border_float_is_32_bit_words_not_f16() {
    let c = [0.25f32, 0.5, 0.75, 0.125];
    let i = Input {
        wrap_s: Wrap::ClampToBorder,
        wrap_t: Wrap::ClampToBorder,
        border: Border::Float(c),
        ..Input::default_2d()
    };
    let s = sampler71(&i).unwrap();
    assert_eq!(s.border_color_mode, BORDER_FOLLOWS);
    assert_eq!(s.border_color, c.map(f32::to_bits));
    assert_eq!(s.border_color[0], 0x3e80_0000, "f32 0.25, not f16 0x3400");
    let r = record(&i);
    assert_eq!(le64(&r) >> 58 & 7, 7);
    assert_eq!(le64(&r) >> 48 & 7, 3);
    for (k, v) in c.iter().enumerate() {
        assert_eq!(word(&r, k), v.to_bits());
    }
    // the three shortcuts
    let mode =
        |b| sampler71(&Input { wrap_s: Wrap::ClampToBorder, border: Border::Float(b), ..Input::default_2d() }).unwrap();
    assert_eq!(mode([0.0; 4]).border_color_mode, BORDER_0000);
    assert_eq!(mode([-0.0, 0.0, 0.0, 0.0]).border_color_mode, BORDER_0000);
    assert_eq!(mode([0.0, 0.0, 0.0, 1.0]).border_color_mode, BORDER_0001);
    assert_eq!(mode([1.0; 4]).border_color_mode, BORDER_1111);
    assert_eq!(mode([1.0, 1.0, 1.0, 0.0]).border_color_mode, BORDER_FOLLOWS);
    // no wrap reads the border: 0000, words zero, whatever the colour
    let s = sampler71(&Input { border: Border::Float(c), ..Input::default_2d() }).unwrap();
    assert_eq!((s.border_color_mode, s.border_color), (BORDER_0000, [0; 4]));
    assert_eq!(
        sampler71(&Input {
            wrap_s: Wrap::ClampToBorder,
            border: Border::Float([f32::NAN, 0.0, 0.0, 0.0]),
            ..Input::default_2d()
        }),
        Err(Refusal::NanBorder)
    );
}

#[test]
fn border_integer_words_as_given() {
    let w = [7u32, 0xffff_ffff, 0x8000_0000, 1];
    let i = Input { wrap_r: Wrap::ClampToBorder, border: Border::Int(w), ..Input::default_2d() };
    let s = sampler71(&i).unwrap();
    assert_eq!((s.border_color_mode, s.border_color), (BORDER_FOLLOWS, w));
    let r = record(&i);
    for (k, v) in w.iter().enumerate() {
        assert_eq!(word(&r, k), *v);
    }
    // an integer (0, 0, 0, 1) is not the float shortcut
    let one = Input { wrap_r: Wrap::ClampToBorder, border: Border::Int([0, 0, 0, 1]), ..Input::default_2d() };
    let s = sampler71(&one).unwrap();
    assert_eq!((s.border_color_mode, s.border_color), (BORDER_FOLLOWS, [0, 0, 0, 1]));
    let zero = Input { wrap_r: Wrap::ClampToBorder, border: Border::Int([0; 4]), ..Input::default_2d() };
    assert_eq!(sampler71(&zero).unwrap().border_color_mode, BORDER_0000);
}

#[test]
fn shadow_compare() {
    // GL order NEVER..ALWAYS is the field's enum
    for f in 0..8 {
        let i = Input {
            min_filter: Filter::Linear,
            mag_filter: Filter::Linear,
            compare_enabled: true,
            compare_func: f,
            ..Input::default_2d()
        };
        assert_eq!(sampler71(&i).unwrap().compare_func, f);
        assert_eq!(le64(&record(&i)) >> 4 & 7, u64::from(f));
    }
    // off: NEVER (0) whatever the function says
    let off = Input { compare_enabled: false, compare_func: 5, ..Input::default_2d() };
    assert_eq!(sampler71(&off).unwrap().compare_func, 0);
    let bad = Input { compare_enabled: true, compare_func: 8, ..Input::default_2d() };
    assert_eq!(sampler71(&bad), Err(Refusal::CompareFuncOutOfRange));
}

#[test]
fn seamless_cube() {
    let cube = Input {
        min_filter: Filter::Linear,
        mag_filter: Filter::Linear,
        mip_filter: MipFilter::Linear,
        wrap_s: Wrap::ClampToEdge,
        wrap_t: Wrap::ClampToEdge,
        wrap_r: Wrap::ClampToEdge,
        min_lod: 0.0,
        max_lod: 1000.0,
        cube: true,
        seamless: true,
        ..Input::default_2d()
    };
    let s = sampler71(&cube).unwrap();
    assert_eq!((s.wrap_s, s.wrap_t, s.wrap_r), (WRAP_CLAMP, WRAP_CLAMP, WRAP_CLAMP));
    // no cube or seamless bit anywhere: the record is the 2D record's
    let flat = Input { cube: false, ..cube };
    assert_eq!(record(&cube), record(&flat));
    for (n, _, _) in FIELDS {
        assert!(!n.to_lowercase().contains("seamless") && !n.to_lowercase().contains("cube"));
    }
    // non-seamless is refused by name; a non-cube ignores the flag
    let ns = Input { seamless: false, ..cube };
    assert_eq!(sampler71(&ns), Err(Refusal::CubeNonSeamless));
    assert!(Refusal::CubeNonSeamless.message().contains("seamless"));
    assert!(sampler71(&Input { seamless: false, ..flat }).is_ok());
}

#[test]
fn refusals_are_named() {
    let d = Input::default_2d();
    assert_eq!(sampler71(&Input { min_lod: f32::NAN, ..d }), Err(Refusal::NanLod));
    assert_eq!(sampler71(&Input { max_lod: f32::NAN, ..d }), Err(Refusal::NanLod));
    assert_eq!(sampler71(&Input { lod_bias: f32::NAN, ..d }), Err(Refusal::NanBias));
    assert_eq!(sampler71(&Input { lod_bias: 128.0, ..d }), Err(Refusal::BiasOutOfRange));
    assert!(sampler71(&Input { lod_bias: -128.0, ..d }).is_ok());
    for r in [
        Refusal::CubeNonSeamless,
        Refusal::CompareFuncOutOfRange,
        Refusal::NanLod,
        Refusal::NanBias,
        Refusal::BiasOutOfRange,
        Refusal::AnisotropyOutOfRange,
        Refusal::NanBorder,
    ] {
        assert!(r.message().starts_with("V3D 7.1"));
    }
}

#[test]
fn pack_refuses_what_the_c_packer_refuses() {
    let ok = sampler71(&Input::default_2d()).unwrap();
    assert!(pack(&ok).is_some());
    assert!(pack(&Sampler71 { wrap_s: 5, ..ok }).is_none());
    assert!(pack(&Sampler71 { border_color_mode: 3, ..ok }).is_none());
    assert!(pack(&Sampler71 { compare_func: 8, ..ok }).is_none());
    assert!(pack(&Sampler71 { border_color: [1, 0, 0, 0], ..ok }).is_none());
    assert!(pack(&Sampler71 { lod_bias: 128.0, ..ok }).is_none());
}
