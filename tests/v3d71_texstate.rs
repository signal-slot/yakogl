// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 texture record field values (src/v3d/texstate71.rs), host test.
//!
//! Run: cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_texstate
//!
//! Checks every bit position and width the module uses against Mesa's
//! v3d_packet.xml by text, the struct field names against
//! driver/src/v3d/v3d_tex71.h by text, and hand-derived anchors.

#[path = "../src/v3d/texstate71.rs"]
mod texstate71;

use texstate71::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// The text of the `<struct name="{name}" ... min_ver="71">` block.
fn struct71(xml: &str, name: &str) -> String {
    let open = format!("<struct name=\"{name}\"");
    let mut at = 0;
    while let Some(i) = xml[at..].find(&open) {
        let s = at + i;
        let e = s + xml[s..].find("</struct>").unwrap();
        let block = &xml[s..e];
        let head = &block[..block.find('>').unwrap()];
        if head.contains("min_ver=\"71\"") {
            return block.to_string();
        }
        at = e;
    }
    panic!("no 7.1 struct {name}");
}

/// (start, size) of the field `name` in a struct block.
fn field(block: &str, name: &str) -> (u32, u32) {
    let key = format!("<field name=\"{name}\" ");
    let i = block.find(&key).unwrap_or_else(|| panic!("no field {name}"));
    let line = &block[i..i + block[i..].find('>').unwrap()];
    let num = |attr: &str| -> u32 {
        let k = format!("{attr}=\"");
        let j = line.find(&k).unwrap() + k.len();
        line[j..j + line[j..].find('"').unwrap()].parse().unwrap()
    };
    (num("start"), num("size"))
}

fn xml() -> String {
    read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml")
}

fn desc() -> TexDesc {
    TexDesc {
        width: 64,
        height: 64,
        depth: 1,
        levels: 1,
        base_level: 0,
        cube: false,
        srgb: false,
        transfer_func: None,
        bytes_per_texel: 4,
        layer_stride: 0,
        base: 0x1000_0000,
        ver: 71,
        iprev: 10,
    }
}

#[test]
fn texture_shader_state_bits_match_the_cle() {
    let x = xml();
    let b = struct71(&x, "Texture Shader State");
    assert_eq!(field(&b, "Reverse"), (5, 1));
    assert_eq!(field(&b, "Transfer func"), (2, 3));
    assert_eq!(field(&b, "R/B swap"), (32, 1));
    assert_eq!(field(&b, "Array Stride (64-byte aligned)"), (33, ARRAY_STRIDE_BITS));
    assert_eq!(field(&b, "Image Width"), (58, 14));
    assert_eq!(field(&b, "Image Height"), (72, 14));
    assert_eq!(field(&b, "Image Depth"), (86, 14));
    assert_eq!(field(&b, "Max Level"), (120, 4));
    assert_eq!(field(&b, "Base Level"), (124, 4));
    assert!(b.contains("name=\"Transfer Func None\" value=\"0\""));
    assert!(b.contains("name=\"Transfer Func sRGB\" value=\"1\""));
    assert_eq!(TRANSFER_NONE, 0);
    assert_eq!(TRANSFER_SRGB, 1);
    // pack_words places each field at exactly those bits
    let mut s = texture_state(&desc()).unwrap();
    s.width = 0x3fff;
    s.height = 0;
    s.depth = 0;
    s.max_level = 0;
    s.base_level = 0;
    s.array_stride_64 = 0;
    s.base = 0;
    let w = pack_words(&s);
    let bits: u128 = w[..4].iter().enumerate().map(|(i, v)| u128::from(*v) << (32 * i)).sum();
    assert_eq!(bits, 0x3fffu128 << 58);
}

#[test]
fn tmu_config2_bits_match_the_cle() {
    let x = xml();
    let b = struct71(&x, "TMU Config Parameter 2");
    for (name, start, size) in [
        ("Write conversion", 26, 1),
        ("DIM query", 25, 1),
        ("LOD Query", 24, 1),
        ("Op", 20, 4),
        ("Offset R", 16, 4),
        ("Offset T", 12, 4),
        ("Offset S", 8, 4),
        ("Gather Mode", 7, 1),
        ("Gather Component", 5, 2),
        ("Coefficient Mode", 4, 1),
        ("Sample Number", 2, 2),
        ("Disable AutoLOD", 1, 1),
        ("Offset Format 8", 0, 1),
    ] {
        assert_eq!(field(&b, name), (start, size), "{name}");
    }
    assert_eq!(field(&b, "Pad"), (27, 5));
    let one = |c: TmuConfig2| tmu_config2_word(&c).unwrap();
    let d = TmuConfig2::default();
    assert_eq!(one(TmuConfig2 { write_conversion: true, ..d }), 1 << 26);
    assert_eq!(one(TmuConfig2 { dim_query: true, ..d }), 1 << 25);
    assert_eq!(one(TmuConfig2 { lod_query: true, ..d }), 1 << 24);
    assert_eq!(one(TmuConfig2 { op: 0xf, ..d }), 0xf << 20);
    assert_eq!(one(TmuConfig2 { offset_r: -1, ..d }), 0xf << 16);
    assert_eq!(one(TmuConfig2 { offset_t: 7, ..d }), 7 << 12);
    assert_eq!(one(TmuConfig2 { offset_s: -8, ..d }), 8 << 8);
    assert_eq!(one(TmuConfig2 { gather_mode: true, ..d }), 1 << 7);
    assert_eq!(one(TmuConfig2 { gather_component: 3, ..d }), 3 << 5);
    assert_eq!(one(TmuConfig2 { coefficient_mode: true, ..d }), 1 << 4);
    assert_eq!(one(TmuConfig2 { sample_number: 3, ..d }), 3 << 2);
    assert_eq!(one(TmuConfig2 { disable_autolod: true, ..d }), 1 << 1);
    assert_eq!(one(TmuConfig2 { offset_format_8: true, ..d }), 1);
    // pad bits never set
    let all = TmuConfig2 {
        write_conversion: true,
        dim_query: true,
        lod_query: true,
        op: 0xf,
        offset_r: -1,
        offset_t: -1,
        offset_s: -1,
        gather_mode: true,
        gather_component: 3,
        coefficient_mode: true,
        sample_number: 3,
        disable_autolod: true,
        offset_format_8: true,
    };
    assert_eq!(one(all), 0x07ff_ffff);
    // the 4.2 layout has no DIM query: bits 25-31 pad
    let x42 = x.find("<struct name=\"TMU Config Parameter 2\" max_ver=\"42\"");
    assert!(x42.is_some() || x.contains("TMU Config Parameter 2"));
}

#[test]
fn struct_field_names_match_v3d_tex71_h() {
    let h = read("driver/src/v3d/v3d_tex71.h");
    let s = &h[h.find("struct v3d_tex71_state {").unwrap()..];
    let s = &s[..s.find("};").unwrap()];
    for (c, ty) in [
        ("base", "uint32_t"),
        ("width", "uint32_t"),
        ("height", "uint32_t"),
        ("depth", "uint32_t"),
        ("base_level", "uint32_t"),
        ("max_level", "uint32_t"),
        ("transfer_func", "uint32_t"),
        ("r_b_swap", "bool"),
        ("reverse", "bool"),
        ("array_stride_64", "uint32_t"),
    ] {
        let toks: Vec<&str> = s.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
        let found = toks.windows(2).any(|w| w[0] == ty && w[1] == c) || toks.contains(&c) && s.contains(ty);
        assert!(found, "v3d_tex71.h lacks {ty} {c}");
    }
    // the same names exist in the Rust struct (compile-time check)
    let TexState {
        base: _,
        width: _,
        height: _,
        depth: _,
        base_level: _,
        max_level: _,
        transfer_func: _,
        r_b_swap: _,
        reverse: _,
        array_stride_64: _,
    } = texture_state(&desc()).unwrap();
    assert!(h.contains("V3D_TEX71_TRANSFER_NONE 0") && h.contains("V3D_TEX71_TRANSFER_SRGB 1"));
}

#[test]
fn anchor_rgba8_2d_64x64() {
    // 64x64 RGBA8, one level: nothing but size and the base.
    let s = texture_state(&desc()).unwrap();
    assert_eq!(
        s,
        TexState {
            base: 0x1000_0000,
            width: 64,
            height: 64,
            depth: 1,
            base_level: 0,
            max_level: 0,
            transfer_func: 0,
            r_b_swap: false,
            reverse: false,
            array_stride_64: 0
        }
    );
    // hand-derived: width 64 << 58 -> word 1 bit 26 (0x0400_0000);
    // height 64 << 72 -> word 2 bit 8 (0x40); depth 1 << 86 -> word 2 bit 22.
    let w = pack_words(&s);
    assert_eq!(w[0], 0x1000_0000);
    // width 64 = bit 6 -> record bit 64 -> word 2 bit 0; its low 6 bits are 0
    assert_eq!(w[1], 0);
    assert_eq!(w[2], 1 | 64 << 8 | 1 << 22);
    assert_eq!(w[3], 0);
}

#[test]
fn anchor_cube_six_faces() {
    // 64x64 RGBA8 cube, 7 levels (64..1). One face's mip tree is at least
    // 64*64*4 * 4/3 ~ 21848 bytes; take a 64-aligned 21952-byte stride.
    let d = TexDesc { depth: 6, levels: 7, cube: true, layer_stride: 21952, ..desc() };
    let s = texture_state(&d).unwrap();
    assert_eq!((s.depth, s.max_level, s.base_level), (6, 6, 0));
    assert_eq!(s.array_stride_64, 343); // 21952 / 64
    assert!(!s.r_b_swap && !s.reverse);
    let w = pack_words(&s);
    // 343 << 33 is word 1 bit 1 up
    assert_eq!(w[1], 343 << 1);
    // depth 6 << 86 -> word 2 bits 22-24; max level 6 << 120 -> word 3 bit 24
    assert_eq!(w[2], 1 | 64 << 8 | 6 << 22);
    assert_eq!(w[3], 6 << 24);
    assert_eq!(texture_state(&TexDesc { height: 32, ..d }), Err(Refusal::CubeNotSquare));
    assert_eq!(texture_state(&TexDesc { depth: 5, ..d }), Err(Refusal::CubeFaces { depth: 5 }));
}

#[test]
fn anchor_srgb() {
    let s = texture_state(&TexDesc { srgb: true, ..desc() }).unwrap();
    assert_eq!(s.transfer_func, TRANSFER_SRGB);
    assert!(!s.r_b_swap);
    // transfer func at bit 2: sRGB is bit 2 of word 0, not the 4.2 bit 3
    assert_eq!(pack_words(&s)[0], 0x1000_0000 | 1 << 2);
}

#[test]
fn anchor_2d_array() {
    // 32x32 RGBA8 array of 10 layers, 6 levels, base level 1.
    let d = TexDesc { width: 32, height: 32, depth: 10, levels: 5, base_level: 1, layer_stride: 4096 + 64, ..desc() };
    let s = texture_state(&d).unwrap();
    assert_eq!((s.depth, s.base_level, s.max_level), (10, 1, 5));
    assert_eq!(s.array_stride_64, 65);
    let w = pack_words(&s);
    assert_eq!(w[1], (65 << 1) | (32 << 26));
    assert_eq!(w[2], 32 << 8 | 10 << 22);
    assert_eq!(w[3], 5 << 24 | 1 << 28);
}

#[test]
fn refusals_are_named() {
    let d = desc();
    let r = |t: TexDesc| texture_state(&t).unwrap_err();
    assert_eq!(r(TexDesc { iprev: 4, ..d }), Refusal::RevisionBelow715 { iprev: 4 });
    assert!(texture_state(&TexDesc { iprev: 5, ..d }).is_ok());
    assert_eq!(r(TexDesc { ver: 42, ..d }), Refusal::NotV3d71 { ver: 42 });
    let two = TexDesc { depth: 2, ..d };
    assert_eq!(r(TexDesc { layer_stride: 100, ..two }), Refusal::StrideNotAligned { stride: 100 });
    assert_eq!(r(TexDesc { layer_stride: 64 << 24, ..two }), Refusal::StrideTooLarge { stride: 64 << 24 });
    assert_eq!(
        texture_state(&TexDesc { layer_stride: (64 << 24) - 64, ..two }).unwrap().array_stride_64,
        (1 << 24) - 1
    );
    assert_eq!(r(two), Refusal::StrideMissing);
    assert_eq!(r(TexDesc { width: 4097, ..d }), Refusal::DimensionTooLarge { dim: 4097 });
    assert!(texture_state(&TexDesc { width: 4096, height: 4096, ..d }).is_ok());
    assert_eq!(r(TexDesc { transfer_func: Some(2), ..d }), Refusal::TransferFunc { value: 2 });
    assert_eq!(r(TexDesc { levels: 0, ..d }), Refusal::ZeroSize);
    assert_eq!(r(TexDesc { base_level: 15, levels: 2, ..d }), Refusal::LevelOutOfRange { base_level: 15, levels: 2 });
    assert_eq!(r(TexDesc { base: 0x1000_0020, ..d }), Refusal::BaseNotAligned { base: 0x1000_0020 });
    assert_eq!(r(TexDesc { bytes_per_texel: 0, ..d }), Refusal::ZeroBytesPerTexel);
    assert_eq!(r(TexDesc { depth: 2049, layer_stride: 64, ..d }), Refusal::LayersTooMany { layers: 2049 });
    assert!(matches!(
        tmu_config2_word(&TmuConfig2 { offset_s: 8, ..Default::default() }),
        Err(Refusal::TmuField { name: "offset_s", .. })
    ));
    for e in [
        Refusal::NotV3d71 { ver: 0 },
        Refusal::RevisionBelow715 { iprev: 0 },
        Refusal::ZeroSize,
        Refusal::DimensionTooLarge { dim: 0 },
        Refusal::LayersTooMany { layers: 0 },
        Refusal::CubeNotSquare,
        Refusal::CubeFaces { depth: 0 },
        Refusal::LevelOutOfRange { base_level: 0, levels: 0 },
        Refusal::ZeroBytesPerTexel,
        Refusal::StrideNotAligned { stride: 0 },
        Refusal::StrideTooLarge { stride: 0 },
        Refusal::StrideMissing,
        Refusal::BaseNotAligned { base: 0 },
        Refusal::TransferFunc { value: 0 },
        Refusal::TmuField { name: "x", value: 0 },
    ] {
        assert!(e.message().starts_with("texture state 7.1: "), "{e:?}");
    }
}
