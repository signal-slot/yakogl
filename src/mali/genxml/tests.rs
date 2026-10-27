// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Each refusal of the generated packers on a real v10.xml field, with its
//! message: where Mesa's packer masks, rounds, clamps or ORs, this one
//! refuses, and says which structure, which field and why. Every value is
//! also shown accepted one step inside the limit, so a refusal that fired
//! on everything could not pass.

use super::pack::{PackError, Refusal};
use super::v10::*;
use std::collections::BTreeSet;

fn refusal<T>(r: Result<T, PackError>) -> PackError {
    match r {
        Ok(_) => panic!("packed a value that should be refused"),
        Err(e) => e,
    }
}

#[test]
fn a_value_wider_than_its_field_is_refused() {
    let e = refusal(Scissor { scissor_minimum_x: 0x1_0000, ..Default::default() }.pack());
    assert_eq!(e.refusal, Refusal::TooWide { value: 0x1_0000, bits: 16 });
    assert_eq!(e.to_string(), "mali Scissor.Scissor Minimum X: 0x10000 does not fit 16 bits");
    let ok = Scissor { scissor_minimum_x: 0xffff, ..Default::default() }.pack().expect("16 bits fit");
    assert_eq!(&ok[..4], &[0xff, 0xff, 0, 0]);
}

#[test]
fn a_shr_with_low_bits_set_is_refused() {
    let e = refusal(BlendShader { pc: 0x1003 }.pack());
    assert_eq!(e.refusal, Refusal::LowBitsSet { value: 0x1003, shift: 4 });
    assert_eq!(e.to_string(), "mali Blend Shader.PC: 0x1003 has low bits set that shr(4) would drop");
    /* 0x1000 >> 4 = 0x100 at bit 36: word 1 = 0x100 << 4 */
    let ok = BlendShader { pc: 0x1000 }.pack().expect("aligned");
    assert_eq!(ok, [0, 0, 0, 0, 0x00, 0x10, 0, 0]);
}

#[test]
fn a_log2_of_zero_or_of_a_non_power_of_two_is_refused() {
    for v in [0u32, 3, 0x8000_0001] {
        let e = refusal(LocalStorage { wls_instances: v, ..Default::default() }.pack());
        assert_eq!(e.refusal, Refusal::NotPowerOfTwo { value: u64::from(v) });
    }
    let e = refusal(LocalStorage { wls_instances: 3, ..Default::default() }.pack());
    assert_eq!(e.to_string(), "mali Local Storage.WLS Instances: 0x3 is not a power of two (log2)");
    let ok = LocalStorage { wls_instances: 4, ..Default::default() }.pack().expect("a power of two");
    assert_eq!(ok[4], 2, "log2(4) in word 1");
}

#[test]
fn a_minus_one_of_zero_is_refused() {
    let e = refusal(TilerContext { fb_width: 0, fb_height: 1, layer_count: 1, ..Default::default() }.pack());
    assert_eq!(e.refusal, Refusal::BelowMinus { value: 0, minus: 1 });
    assert_eq!(e.to_string(), "mali Tiler Context.FB Width: 0 is below the 1 that minus(1) subtracts");
    /* Mesa's header leaves it 0 too, and Mesa's own packer asserts on it */
    assert!(TilerContext::default().pack().is_err());
    let ok = TilerContext { fb_width: 1, fb_height: 1, layer_count: 1, ..Default::default() }.pack().expect("1");
    assert_eq!(&ok[12..16], &[0, 0, 0, 0], "1 - 1 = 0 in both");
}

#[test]
fn an_align_field_off_its_alignment_is_refused_not_rounded() {
    let e = refusal(TilerHeap { size: 4097, ..Default::default() }.pack());
    assert_eq!(e.refusal, Refusal::NotAligned { value: 4097, align: 4096 });
    assert_eq!(e.to_string(), "mali Tiler Heap.Size: 0x1001 is not a multiple of 4096 (align(4096))");
    TilerHeap { size: 8192, ..Default::default() }.pack().expect("aligned");
}

#[test]
fn two_non_zero_members_of_one_union_are_refused() {
    /* Mesa's header of the render-target union sets several arms itself
     * (YUV and AFBC YUV both default `YUV Enable` to true, the AFRC arms
     * their writeback mode; Mesa ORs them): refused. A render target is
     * packed as its arm (`RgbRenderTarget`), as Mesa's pan_desc.c does. */
    let e = refusal(RenderTarget::default().pack());
    assert_eq!(e.refusal, Refusal::SharedBits { other: "YUV.YUV Enable" });
    assert_eq!(
        e.to_string(),
        "mali Render Target.AFBC YUV.YUV Enable: shares bits with YUV.YUV Enable, and both are non-zero"
    );

    /* inside one struct: two formats over the same five bits */
    let both = RgbRenderTarget {
        writeback_format: ColorFormat(1),
        writeback_float_format: FloatColorFormat(1),
        ..Default::default()
    };
    let e = refusal(both.pack());
    assert_eq!(e.refusal, Refusal::SharedBits { other: "Writeback Format" });

    /* the preload mask and one of its single-register flags */
    let p = ShaderProgram {
        stage: ShaderStage::VERTEX,
        preload: Preload { r48_r63: 0x80, r55: true, ..Default::default() },
        ..Default::default()
    };
    let e = refusal(p.pack());
    assert_eq!(e.refusal, Refusal::SharedBits { other: "Preload.R48-R63" });
    assert_eq!(e.field, "Preload.R55");

    /* the blend shader and fixed-function halves: one alone packs as that
     * half's own struct, both are refused */
    let mut b = InternalBlend::default();
    b.shader.pc = 0x10;
    assert_eq!(b.pack().expect("one half"), b.shader.pack().expect("the half"));
    b.fixed_function.conversion.memory_format = 1;
    assert_eq!(refusal(b.pack()).refusal, Refusal::SharedBits { other: "Shader.PC" });
}

#[test]
fn a_non_zero_value_the_enum_does_not_name_is_refused() {
    let e = refusal(ShaderProgram { stage: ShaderStage(7), ..Default::default() }.pack());
    assert_eq!(e.refusal, Refusal::NotInEnum { value: 7, enumeration: "Shader stage" });
    assert_eq!(e.to_string(), "mali Shader Program.Stage: 7 is not a value of the enum Shader stage");
    /* zero is Mesa's header default, but no value of this enum: refused (below) */
    assert_eq!(refusal(ShaderProgram::default().pack()).refusal, Refusal::EnumUnset);
    /* a named value that does not fit its field is refused as too wide:
     * AFRC Format 256 in eight bits */
    let e = refusal(AfrcPlane { format: AfrcFormat(256), ..Default::default() }.pack());
    assert_eq!(e.refusal, Refusal::TooWide { value: 256, bits: 8 });
}

#[test]
fn a_signed_value_outside_its_range_is_refused() {
    let tc = |o: i32| TilerContext { fb_width: 1, fb_height: 1, layer_count: 1, layer_offset: o, ..Default::default() };
    for o in [256, -257] {
        assert_eq!(refusal(tc(o).pack()).refusal, Refusal::OutOfRange { value: i64::from(o), bits: 9 });
    }
    assert_eq!(
        refusal(tc(256).pack()).to_string(),
        "mali Tiler Context.Layer offset: 256 is outside the signed 9-bit range"
    );
    tc(255).pack().expect("in range");
    /* -256 is 0x100 in nine bits, at bit 8 of word 4 */
    let b = tc(-256).pack().expect("in range");
    assert_eq!(&b[16..20], &[0, 0, 1, 0]);
}

#[test]
fn a_lod_outside_its_fixed_point_range_or_nan_is_refused_not_clamped() {
    for v in [f32::NAN, -1.0, 32.0] {
        let e = refusal(Sampler { minimum_lod: v, ..Default::default() }.pack());
        assert!(matches!(e.refusal, Refusal::NotFixedPoint { bits: 13, signed: false, .. }), "{v}: {e}");
    }
    assert_eq!(
        refusal(Sampler { minimum_lod: 32.0, ..Default::default() }.pack()).to_string(),
        "mali Sampler.Minimum LOD: 32 is outside the unsigned 13-bit LOD range (8 fractional bits)"
    );
    /* 8191/256, the largest */
    Sampler { minimum_lod: 31.996_094, ..Default::default() }.pack().expect("the largest ulod");
    let e = refusal(Sampler { lod_bias: 128.0, ..Default::default() }.pack());
    assert!(matches!(e.refusal, Refusal::NotFixedPoint { bits: 16, signed: true, .. }), "{e}");
    let b = Sampler { lod_bias: -128.0, ..Default::default() }.pack().expect("the smallest slod");
    assert_eq!(&b[8..10], &[0x00, 0x80], "-32768 in sixteen bits");
}

#[test]
fn unpack_refuses_a_bit_no_field_owns_and_an_unnamed_enum_value() {
    let mut b = BlendShader { pc: 0x1000 }.pack().expect("packs");
    assert_eq!(BlendShader::unpack(&b), Ok(BlendShader { pc: 0x1000 }));
    b[0] = 1;
    let e = refusal(BlendShader::unpack(&b));
    assert_eq!(e.refusal, Refusal::ReservedBits { word: 0, bits: 1 });
    assert_eq!(e.to_string(), "mali Blend Shader: word 0 has bits 0x00000001 set that no field owns");

    let mut s = Sampler::default().pack().expect("packs");
    s[0] = (s[0] & 0xf0) | 3;
    let e = refusal(Sampler::unpack(&s));
    assert_eq!(e.refusal, Refusal::NotInEnum { value: 3, enumeration: "Descriptor Type" });
}

#[test]
fn unpack_as_decodes_the_chosen_union_member_and_refuses_the_rest() {
    let size = PrimitiveSize { fixed_sized: 2.5, size_array: 0 };
    let b = size.pack().expect("packs");
    assert_eq!(PrimitiveSize::unpack_as(&b, &["Fixed Sized"]), Ok(size));
    /* the other arm owns the whole 64 bits: the same bytes read as an array */
    assert_eq!(
        PrimitiveSize::unpack_as(&b, &["Size Array"]),
        Ok(PrimitiveSize { fixed_sized: 0.0, size_array: u64::from(2.5f32.to_bits()) })
    );
    /* no arm: no member owns the set bits */
    let e = refusal(PrimitiveSize::unpack_as(&b, &[]));
    assert!(matches!(e.refusal, Refusal::ReservedBits { word: 0, .. }), "{e}");
    /* the high word is the array's alone */
    let mut high = b;
    high[7] = 1;
    let e = refusal(PrimitiveSize::unpack_as(&high, &["Fixed Sized"]));
    assert_eq!(e.refusal, Refusal::ReservedBits { word: 1, bits: 0x0100_0000 });
    /* two members that share bits cannot both be chosen; a stranger is no member */
    let e = refusal(PrimitiveSize::unpack_as(&b, &["Fixed Sized", "Size Array"]));
    assert!(matches!(e.refusal, Refusal::SharedBits { .. }), "{e}");
    let e = refusal(PrimitiveSize::unpack_as(&b, &["Nope"]));
    assert_eq!(e.refusal, Refusal::NotAMember);
}

/// The messages are identifiers (a measurement groups by them): one text
/// per cause.
#[test]
fn every_refusal_has_its_own_message() {
    let e = |refusal| PackError { structure: "S", field: "f", refusal }.to_string();
    let all = [
        e(Refusal::TooWide { value: 1, bits: 1 }),
        e(Refusal::OutOfRange { value: 1, bits: 1 }),
        e(Refusal::LowBitsSet { value: 1, shift: 1 }),
        e(Refusal::NotPowerOfTwo { value: 1 }),
        e(Refusal::BelowMinus { value: 1, minus: 1 }),
        e(Refusal::NotAligned { value: 1, align: 1 }),
        e(Refusal::NotInEnum { value: 1, enumeration: "E" }),
        e(Refusal::EnumUnset),
        e(Refusal::NotFixedPoint { value: 1.0, bits: 1, signed: false }),
        e(Refusal::SharedBits { other: "g" }),
        e(Refusal::ReservedBits { word: 1, bits: 1 }),
        e(Refusal::TooLargeForType { value: 1 }),
        e(Refusal::NotAMember),
        e(Refusal::Layout),
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn an_enum_left_at_a_zero_it_does_not_name_is_refused() {
    let e = refusal(ShaderProgram::default().pack());
    assert_eq!(e.refusal, Refusal::EnumUnset);
    assert_eq!(e.to_string(), "mali Shader Program.Stage: enum value 0 is not a named value");
    /* the same descriptor with a named stage packs */
    ShaderProgram { stage: ShaderStage::VERTEX, ..Default::default() }.pack().expect("a named stage");
    /* an unnamed non-zero value keeps its own refusal */
    assert!(matches!(
        refusal(ShaderProgram { stage: ShaderStage(7), ..Default::default() }.pack()).refusal,
        Refusal::NotInEnum { value: 7, .. }
    ));
    /* a zero that is named is no refusal: Mesa's header default of an enum
     * that names zero (Sampler's Descriptor Type is set by the header, and
     * the first value of Texture Dimension is 0) */
    Sampler::default().pack().expect("the defaults of a struct whose enums name zero");

    /* a union: the ZS CRC Extension holds a depth plane (AFBC ZS or ZS: the
     * two share bits) and a stencil plane (AFBC S or S: the other pair). All
     * four Write Formats at zero is refused; an arm set stands in for the
     * arm it shares bits with, so one of each pair is enough. A depth plane
     * alone leaves the stencil pair unset, and is refused too (what S Format
     * zero means is not in v10.xml). */
    let e = refusal(ZsCrcExtension::default().pack());
    assert_eq!(e.refusal, Refusal::EnumUnset);
    assert_eq!(e.to_string(), "mali ZS CRC Extension.AFBC ZS.Write Format: enum value 0 is not a named value");
    let mut z = ZsCrcExtension::default();
    z.zs.write_format = ZsFormat::D24S8;
    assert_eq!(
        refusal(z.pack()).to_string(),
        "mali ZS CRC Extension.AFBC S.Write Format: enum value 0 is not a named value"
    );
    z.s.write_format = SFormat::S8;
    z.pack().expect("the ZS and the S arms are the active ones");
}

/// A struct of the scan: name, no-direct-packing, (field, type, has default).
type XmlStruct = (String, bool, Vec<(String, String, bool)>);

/// Every default-less enum field whose zero is no named value, found in
/// v10.xml by a scan that shares no code with the generator, is on the
/// packer's list of fields it refuses at zero: the list is not typed.
#[test]
fn the_unset_enum_leaves_are_the_ones_v10_xml_has() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let xml = std::fs::read_to_string(root.join("third_party/mesa-26.2.1/src/panfrost/genxml/v10.xml"))
        .expect("the vendored v10.xml");
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!(" {name}=\"");
        let at = tag.find(&key)? + key.len();
        Some(tag[at..tag.len() - tag[at..].len() + tag[at..].find('"')?].to_string())
    };
    /* enum name -> does it name zero */
    let mut enums: Vec<(String, bool)> = Vec::new();
    /* struct name -> (no-direct-packing, [(field, type, has default)]) */
    let mut structs: Vec<XmlStruct> = Vec::new();
    let (mut in_enum, mut in_struct) = (false, false);
    for line in xml.lines().map(str::trim) {
        if line.starts_with("<enum ") {
            enums.push((attr(line, "name").expect("enum name"), false));
            in_enum = !line.ends_with("/>");
        } else if line.starts_with("</enum>") {
            in_enum = false;
        } else if in_enum && line.starts_with("<value ") {
            let v = attr(line, "value").expect("enum value");
            let v = v.strip_prefix("0x").map_or_else(|| v.parse::<i64>(), |h| i64::from_str_radix(h, 16));
            enums.last_mut().expect("in an enum").1 |= v.expect("a number") == 0;
        } else if line.starts_with("<struct ") {
            structs.push((attr(line, "name").expect("struct name"), line.contains("no-direct-packing"), Vec::new()));
            in_struct = !line.ends_with("/>");
        } else if line.starts_with("</struct>") {
            in_struct = false;
        } else if in_struct && line.starts_with("<field ") {
            let f =
                (attr(line, "name").expect("name"), attr(line, "type").expect("type"), line.contains(" default=\""));
            structs.last_mut().expect("in a struct").2.push(f);
        }
    }
    fn walk(
        enums: &[(String, bool)],
        structs: &[XmlStruct],
        fields: &[(String, String, bool)],
        path: &str,
        out: &mut Vec<String>,
    ) {
        for (name, ty, has_default) in fields {
            let path = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
            if let Some((_, names_zero)) = enums.iter().find(|(n, _)| n == ty) {
                if !has_default && !names_zero {
                    out.push(path);
                }
            } else if let Some((_, _, sub)) = structs.iter().find(|(n, _, _)| n == ty) {
                walk(enums, structs, sub, &path, out);
            }
        }
    }
    let mut from_xml = BTreeSet::new();
    let mut packable = 0;
    for (name, no_direct, fields) in &structs {
        if *no_direct {
            continue;
        }
        packable += 1;
        let mut leaves = Vec::new();
        walk(&enums, &structs, fields, "", &mut leaves);
        from_xml.extend(leaves.into_iter().map(|l| format!("{name}.{l}")));
    }
    let listed: BTreeSet<String> = UNSET_ENUM_LEAVES.iter().map(|s| s.to_string()).collect();
    assert_eq!(listed.len(), UNSET_ENUM_LEAVES.len(), "a leaf listed twice");
    assert_eq!(listed, from_xml, "the packer's list against the scan of v10.xml");
    /* the scan saw the population it should: the packable structs, and a
     * known positive among the leaves */
    assert_eq!(packable, 100, "structs the scan took as packable");
    assert!(from_xml.contains("Shader Program.Stage"));
    eprintln!("mali: {} packable enum fields refused at an unnamed zero, in {packable} structs", from_xml.len());
}
