// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/attr71.rs` (V3D 7.1 vertex attribute records) against Mesa
//! 26.2.1's v3d_packet.xml and the attribute records Mesa emitted on the
//! Pi 5 (tools/v3d/pi5/*.clif; the decoded `shadrec_gl_attr` blocks are in
//! the CLIF dumps, the .stdout files carry no records).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_attr
//! ```
#[path = "../src/v3d/attr71.rs"]
mod attr71;

use attr71::*;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The xml struct's `<field name size start>` rows (start in bits; `Nb` is
/// bytes) and the enum values of `Type`.
fn xml_struct() -> (Vec<(String, u32, u32)>, Vec<(String, u32)>) {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let s = xml.find("<struct name=\"GL Shader State Attribute Record\">").expect("struct");
    let body = &xml[s..s + xml[s..].find("</struct>").unwrap()];
    let attr = |line: &str, k: &str| -> String {
        let p = line.find(&format!("{k}=\"")).unwrap() + k.len() + 2;
        line[p..p + line[p..].find('"').unwrap()].to_string()
    };
    let (mut fields, mut values) = (vec![], vec![]);
    for line in body.lines() {
        let l = line.trim();
        if l.starts_with("<field ") {
            let start = attr(l, "start");
            let start = match start.strip_suffix('b') {
                Some(n) => n.parse::<u32>().unwrap() * 8,
                None => start.parse().unwrap(),
            };
            fields.push((attr(l, "name"), start, attr(l, "size").parse().unwrap()));
        } else if l.starts_with("<value ") {
            values.push((attr(l, "name"), attr(l, "value").parse().unwrap()));
        }
    }
    (fields, values)
}

#[test]
fn fields_and_size_match_the_xml() {
    let (fields, values) = xml_struct();
    assert_eq!(fields.len(), FIELDS.len());
    for (x, m) in fields.iter().zip(FIELDS.iter()) {
        assert_eq!((x.0.as_str(), x.1, x.2), *m);
    }
    /* the packet size: the highest bit used, in bytes */
    let end = fields.iter().map(|f| f.1 + f.2).max().unwrap();
    assert_eq!(end as usize / 8, RECORD_BYTES);
    let want = [
        ("Attribute half-float", TYPE_HALF_FLOAT),
        ("Attribute float", TYPE_FLOAT),
        ("Attribute fixed", TYPE_FIXED),
        ("Attribute byte", TYPE_BYTE),
        ("Attribute short", TYPE_SHORT),
        ("Attribute int", TYPE_INT),
        ("Attribute int2_10_10_10", TYPE_INT2_10_10_10),
    ];
    assert_eq!(values.len(), want.len());
    for (v, w) in values.iter().zip(want) {
        assert_eq!((v.0.as_str(), v.1), (w.0, w.1 as u32));
    }
    /* the divisor limit is Mesa's v3d_limits.h */
    let lim = read("third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h");
    assert!(lim.contains("V3D_MAX_VERTEX_ATTRIB_DIVISOR 0xffff"));
    assert_eq!(MAX_DIVISOR, 0xffff);
}

/// Pack by the xml's own positions, independently of `pack`.
fn pack_by_xml(r: &Record71) -> [u8; 16] {
    let (fields, _) = xml_struct();
    let vals: [u64; 11] = [
        r.address as u64,
        r.vec_size as u64,
        r.ty as u64,
        r.signed as u64,
        r.normalized as u64,
        r.read_as_int as u64,
        r.cs_values as u64,
        r.vs_values as u64,
        r.divisor as u64,
        r.stride as u64,
        r.max_index as u64,
    ];
    let mut w: u128 = 0;
    for (f, v) in fields.iter().zip(vals) {
        assert!(v < 1u64 << f.2, "{} overflows", f.0);
        w |= (v as u128) << f.1;
    }
    w.to_le_bytes()[..16].try_into().unwrap()
}

#[test]
fn pack_follows_the_xml() {
    let r = Record71 {
        address: 0x0001_5030,
        vec_size: 3,
        ty: TYPE_INT2_10_10_10,
        signed: true,
        normalized: true,
        read_as_int: true,
        cs_values: 9,
        vs_values: 12,
        divisor: 0xabcd,
        stride: 0x1234_5678,
        max_index: MAX_INDEX,
    };
    assert_eq!(pack(&r), pack_by_xml(&r));
    let z = Record71 {
        address: 0,
        vec_size: 0,
        ty: 0,
        signed: false,
        normalized: false,
        read_as_int: false,
        cs_values: 0,
        vs_values: 0,
        divisor: 0,
        stride: 0,
        max_index: 0,
    };
    assert_eq!(pack(&z), [0; 16]);
}

/// One decoded `shadrec_gl_attr` block.
#[derive(Debug)]
struct Dump {
    addr: u32,
    vec: u32,
    ty: u8,
    signed: bool,
    norm: bool,
    int: bool,
    cs: u8,
    vs: u8,
    div: u32,
    stride: u32,
    maxi: u32,
}

fn dumps(file: &str) -> Vec<Dump> {
    let text = read(file);
    let mut out = vec![];
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].contains("@format shadrec_gl_attr") {
            let blk = &lines[i + 1..i + 12];
            let val = |k: &str| -> &str {
                let l = blk.iter().find(|l| l.contains(&format!("{k}: */"))).unwrap_or_else(|| panic!("{k}"));
                l.split(": */").nth(1).unwrap().trim()
            };
            let num = |k: &str| -> u32 { val(k).split_whitespace().next().unwrap().parse().unwrap() };
            let flag = |k: &str| val(k).starts_with('0') == false;
            let a = val("Address");
            let hex = a
                .rsplit("/* 0x")
                .next()
                .unwrap()
                .trim_end_matches(" */")
                .trim_end_matches('*')
                .trim()
                .trim_end_matches('/')
                .trim();
            out.push(Dump {
                addr: u32::from_str_radix(hex, 16).unwrap(),
                vec: num("Vec size"),
                ty: num("Type") as u8,
                signed: flag("Signed int type"),
                norm: flag("Normalized int type"),
                int: flag("Read as int/uint"),
                cs: num("Number of values read by Coordinate shader") as u8,
                vs: num("Number of values read by Vertex shader") as u8,
                div: num("Instance Divisor"),
                stride: num("Stride"),
                maxi: num("Maximum Index"),
            });
            i += 12;
        } else {
            i += 1;
        }
    }
    out
}

/// Rebuild each dumped record from its description and compare every field.
fn check_dump(file: &str, min: usize) {
    let d = dumps(file);
    assert!(d.len() >= min, "{file}: {} records", d.len());
    for r in &d {
        assert_eq!(r.ty, TYPE_FLOAT, "{file}: only float attributes are emitted");
        let a = Attr {
            enabled: true,
            components: r.vec,
            gl_type: GL_FLOAT,
            normalized: r.norm,
            integer: r.int,
            stride: r.stride as u64,
            offset: 0,
            buffer_addr: r.addr,
            divisor: r.div,
            generic: [0; 4],
            constant_addr: None,
        };
        let mut cs = [0u8; MAX_ATTRS];
        let mut vs = [0u8; MAX_ATTRS];
        cs[0] = r.cs;
        vs[0] = r.vs;
        /* cs_builtins false and one record: the GFXH-930 rule adds a read
         * only when none is loaded, so a dump with cs 0 needs a second
         * record that loads one */
        let attrs = [a, a];
        let mut cs2 = cs;
        cs2[1] = 1;
        let p = plan(&attrs, &if r.cs == 0 { cs2 } else { cs }, &vs, false, 0x100).unwrap();
        let g = p.records[0];
        assert_eq!(g.address, r.addr);
        assert_eq!(g.vec_size as u32, r.vec & 3);
        assert_eq!(g.ty, r.ty);
        assert_eq!((g.signed, g.normalized, g.read_as_int), (r.signed, r.norm, r.int));
        assert_eq!((g.cs_values, g.vs_values), (r.cs, r.vs));
        assert_eq!((g.divisor as u32, g.stride, g.max_index), (r.div, r.stride, r.maxi));
        /* vec size 4 is the field value 0 (the dump prints the count) */
        assert_eq!(pack(&g)[4] & 3, (r.vec & 3) as u8);
    }
}

#[test]
fn triangle_records() {
    let d = dumps("tools/v3d/pi5/triangle.clif");
    assert_eq!(d.len(), 2);
    assert_eq!((d[0].cs, d[0].vs, d[1].cs, d[1].vs), (4, 4, 0, 4));
    assert_eq!((d[0].stride, d[1].stride, d[0].vec), (16, 16, 4));
    check_dump("tools/v3d/pi5/triangle.clif", 2);
}

#[test]
fn gl_case_records() {
    /* 40-byte vertex: positions vec 4, colours vec 4; 36-byte: vec 3 ... */
    check_dump("tools/v3d/pi5/gl_fbods.clif", 30);
    check_dump("tools/v3d/pi5/gl_colormask.clif", 6);
    check_dump("tools/v3d/pi5/gl_writemask.clif", 13);
    let mut vecs: Vec<u32> = dumps("tools/v3d/pi5/gl_fbods.clif")
        .iter()
        .chain(dumps("tools/v3d/pi5/gl_scisbox.clif").iter())
        .chain(dumps("tools/v3d/pi5/gl_writemask.clif").iter())
        .map(|d| d.vec)
        .collect();
    vecs.sort();
    vecs.dedup();
    assert!(vecs.contains(&2) && vecs.contains(&3) && vecs.contains(&4), "{vecs:?}");
}

fn enabled(t: u32, n: u32) -> Attr {
    Attr { enabled: true, components: n, gl_type: t, buffer_addr: 0x4000, stride: 16, ..Attr::disabled() }
}

fn one(a: Attr, reads: u8) -> Result<Record71, Refusal71> {
    let mut vs = [0u8; MAX_ATTRS];
    vs[0] = reads;
    let mut cs = [0u8; MAX_ATTRS];
    cs[0] = reads;
    plan(&[a], &cs, &vs, false, 0x2000).map(|p| p.records[0])
}

#[test]
fn gl_types_map_to_7_1_types() {
    let rows = [
        (GL_BYTE, TYPE_BYTE, true),
        (GL_UNSIGNED_BYTE, TYPE_BYTE, false),
        (GL_SHORT, TYPE_SHORT, true),
        (GL_UNSIGNED_SHORT, TYPE_SHORT, false),
        (GL_INT, TYPE_INT, true),
        (GL_UNSIGNED_INT, TYPE_INT, false),
        (GL_FLOAT, TYPE_FLOAT, false),
        (GL_HALF_FLOAT, TYPE_HALF_FLOAT, false),
        (GL_HALF_FLOAT_OES, TYPE_HALF_FLOAT, false),
        (GL_FIXED, TYPE_FIXED, false),
    ];
    for (t, ty, sg) in rows {
        let r = one(enabled(t, 3), 3).unwrap();
        assert_eq!((r.ty, r.signed, r.vec_size, r.stride, r.address), (ty, sg, 3, 16, 0x4000), "{t:#x}");
    }
    for (t, sg) in [(GL_INT_2_10_10_10_REV, true), (GL_UNSIGNED_INT_2_10_10_10_REV, false)] {
        let r = one(enabled(t, 4), 4).unwrap();
        assert_eq!((r.ty, r.signed, r.vec_size), (TYPE_INT2_10_10_10, sg, 0));
    }
    /* normalized applies to integers, not float */
    let mut a = enabled(GL_UNSIGNED_BYTE, 4);
    a.normalized = true;
    assert!(one(a, 4).unwrap().normalized);
    let mut a = enabled(GL_FLOAT, 4);
    a.normalized = true;
    assert!(!one(a, 4).unwrap().normalized);
    /* integer attributes */
    let mut a = enabled(GL_INT, 2);
    a.integer = true;
    assert!(one(a, 2).unwrap().read_as_int);
    /* offset joins the address, divisor is carried */
    let mut a = enabled(GL_FLOAT, 2);
    a.offset = 8;
    a.divisor = 3;
    let r = one(a, 2).unwrap();
    assert_eq!((r.address, r.divisor), (0x4008, 3));
}

#[test]
fn disabled_attributes() {
    /* read: a stride-0 vec4 record over the draw's own slot */
    let mut a = Attr::disabled();
    a.constant_addr = Some(0x7000);
    let r = one(a, 4).unwrap();
    assert_eq!((r.address, r.stride, r.vec_size, r.ty, r.divisor), (0x7000, 0, 0, TYPE_FLOAT, 0));
    assert_eq!(constant_bytes(&a), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3f]);
    /* integer generic value */
    a.integer = true;
    a.generic = [1, 2, 3, 4];
    let r = one(a, 4).unwrap();
    assert!(r.read_as_int);
    assert_eq!(r.ty, TYPE_INT);
    assert_eq!(constant_bytes(&a)[4..8], 2u32.to_le_bytes());
    /* not read: dummy address, no reads, no slot needed */
    let r = one(Attr::disabled(), 0).unwrap();
    assert_eq!((r.address, r.cs_values, r.vs_values, r.stride), (0x2000, 1, 0, 0), "cs loads one (GFXH-930)");
}

#[test]
fn gfxh_rules_and_empty_draw() {
    let mut cs = [0u8; MAX_ATTRS];
    let mut vs = [0u8; MAX_ATTRS];
    vs[0] = 4;
    vs[1] = 4;
    let at = [enabled(GL_FLOAT, 4), enabled(GL_FLOAT, 4)];
    /* cs reads nothing: the last record loads one value */
    let p = plan(&at, &cs, &vs, false, 0).unwrap();
    assert_eq!((p.records[0].cs_values, p.records[1].cs_values), (0, 1));
    /* cs uses vertex/instance id: the first */
    let p = plan(&at, &cs, &vs, true, 0).unwrap();
    assert_eq!((p.records[0].cs_values, p.records[1].cs_values), (1, 0));
    /* cs reads: untouched */
    cs[0] = 4;
    let p = plan(&at, &cs, &vs, true, 0).unwrap();
    assert_eq!((p.records[0].cs_values, p.records[1].cs_values), (4, 0));
    /* no attribute at all: the dummy record */
    let p = plan(&[], &[0; 16], &[0; 16], false, 0x1234).unwrap();
    assert_eq!(p.count, 1);
    let r = p.records[0];
    assert_eq!(
        (r.address, r.vec_size, r.ty, r.cs_values, r.vs_values, r.stride, r.max_index),
        (0x1234, 1, TYPE_FLOAT, 1, 1, 0, 0)
    );
}

#[test]
fn vpm_sizes() {
    assert_eq!(vpm_input_size(false), Ok(0));
    assert_eq!(vpm_input_size(true), Err(Refusal71::SeparateSegments));
    let mut r = [0u8; MAX_ATTRS];
    r[0] = 4;
    r[1] = 4;
    assert_eq!(input_rows(&r, false, false), 8);
    assert_eq!(input_rows(&r, true, false), 9);
    r[0] = 0;
    assert_eq!(input_rows(&r, false, true), 6, "id row and the driver row");
}

#[test]
fn refusals() {
    use Refusal71::*;
    let a = enabled(GL_FLOAT, 4);
    /* a constant attribute the program reads, with nowhere to read it from */
    assert_eq!(one(Attr::disabled(), 4), Err(ConstantAttribute));
    assert!(ConstantAttribute.message().starts_with("V3D 7.1: a constant vertex attribute"));
    assert_eq!(one(enabled(GL_DOUBLE, 4), 4), Err(Type(GL_DOUBLE)));
    assert_eq!(one(enabled(0x1234, 4), 4), Err(Type(0x1234)));
    assert_eq!(one(enabled(GL_FLOAT, 0), 4), Err(Components(0)));
    assert_eq!(one(enabled(GL_FLOAT, 5), 4), Err(Components(5)));
    assert_eq!(one(enabled(GL_INT_2_10_10_10_REV, 3), 3), Err(PackedComponents));
    let mut b = a;
    b.integer = true;
    assert_eq!(one(b, 4), Err(IntegerType));
    let mut b = enabled(GL_BYTE, 4);
    b.integer = true;
    b.normalized = true;
    assert_eq!(one(b, 4), Err(IntegerType));
    let mut b = a;
    b.stride = u32::MAX as u64 + 1;
    assert_eq!(one(b, 4), Err(Stride(1 << 32)));
    b.stride = u32::MAX as u64;
    assert!(one(b, 4).is_ok());
    let mut b = a;
    b.divisor = 0x10000;
    assert_eq!(one(b, 4), Err(Divisor(0x10000)));
    b.divisor = 0xffff;
    assert_eq!(one(b, 4).unwrap().divisor, 0xffff);
    let mut b = a;
    b.buffer_addr = u32::MAX;
    b.offset = 1;
    assert_eq!(one(b, 4), Err(Address));
    assert_eq!(one(a, 16), Err(Reads(16)));
    /* more attributes than the limit; a read past the records */
    let many = [a; MAX_ATTRS + 1];
    assert_eq!(plan(&many, &[0; 16], &[0; 16], false, 0), Err(TooManyAttributes(17)));
    let mut vs = [0u8; MAX_ATTRS];
    vs[3] = 1;
    assert_eq!(plan(&[a, a], &[0; 16], &vs, false, 0), Err(ReadWithoutRecord(3)));
    /* every refusal names V3D 7.1 */
    for r in [
        ConstantAttribute,
        Type(0),
        Components(0),
        PackedComponents,
        IntegerType,
        TooManyAttributes(0),
        Stride(0),
        Divisor(0),
        Reads(0),
        ReadWithoutRecord(0),
        Address,
        SeparateSegments,
    ] {
        assert!(r.message().starts_with("V3D 7.1: "), "{r:?}");
    }
}
