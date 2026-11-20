// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host check of `src/v3d/record71.rs`, the V3D 7.1.10 draw-index shader
//! record, against Mesa's packet XML and Mesa's own records.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_record`
//! (the features only let the crate build; the test needs none of it).
//!
//! The module is included with `#[path]` (it is not in `src/v3d/mod.rs`
//! yet). What is checked:
//!  - every field's name, position, width and kind and the 32-byte size
//!    against the `GL Shader State Record Draw Index` struct of
//!    `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml` (read by
//!    text), and the packed bytes against an unpacker written here from that
//!    XML (not from the module's table);
//!  - the six gl_test shapes' records in `tools/v3d/pi5/gl_*.clif`: Mesa's
//!    dump decodes the D0 record with the PLAIN 7.1 layout
//!    (`tools/v3d/pi5/README.md` "Differences" item 4), so each dumped
//!    value is put back at its plain bit position to get the record's bytes
//!    and those are read at the D0 layout. Bits 21..23 are past the plain
//!    layout's last flag (bit 20) and not in the dump; and the plain decode
//!    shows nothing at bits 10 and 11 (read as 0);
//!  - each rule of `docs/notes/v3d71-record-u11.md` section 4.
#![allow(dead_code)]

#[path = "../src/v3d/record71.rs"]
mod record71;
use record71::*;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    let p = format!("{ROOT}/{rel}");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn xml() -> String {
    read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml")
}

#[derive(Debug, Clone)]
struct XmlField {
    name: String,
    start: u32,
    size: u32,
    ty: String,
    minus_one: bool,
}

fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!(" {key}=\"");
    let i = line.find(&pat)? + pat.len();
    let j = line[i..].find('"')?;
    Some(&line[i..i + j])
}

/// The fields of the struct whose opening tag starts with `open`.
fn xml_struct(x: &str, open: &str) -> Vec<XmlField> {
    let mut it = x.lines().skip_while(|l| !l.trim_start().starts_with(open));
    assert!(it.next().is_some(), "{open} not in the XML");
    let mut v = Vec::new();
    for l in it {
        if l.contains("</struct>") {
            break;
        }
        if !l.contains("<field ") {
            continue;
        }
        let st = attr(l, "start").unwrap();
        let start = match st.strip_suffix('b') {
            Some(n) => n.parse::<u32>().unwrap() * 8,
            None => st.parse().unwrap(),
        };
        v.push(XmlField {
            name: attr(l, "name").unwrap().to_string(),
            start,
            size: attr(l, "size").unwrap().parse().unwrap(),
            ty: attr(l, "type").unwrap().to_string(),
            minus_one: attr(l, "minus_one") == Some("true"),
        });
    }
    v
}

fn d0_struct() -> Vec<XmlField> {
    xml_struct(&xml(), "<struct name=\"GL Shader State Record Draw Index\" min_ver=\"71\">")
}

fn plain_struct() -> Vec<XmlField> {
    xml_struct(&xml(), "<struct name=\"GL Shader State Record\" min_ver=\"71\">")
}

fn get(bytes: &[u8], start: u32, size: u32) -> u32 {
    let mut v = 0u32;
    for k in 0..size {
        let bit = start + k;
        if bytes[(bit / 8) as usize] >> (bit % 8) & 1 != 0 {
            v |= 1 << k;
        }
    }
    v
}

fn set(bytes: &mut [u8], start: u32, size: u32, v: u32) {
    for k in 0..size {
        if v >> k & 1 != 0 {
            let bit = start + k;
            bytes[(bit / 8) as usize] |= 1 << (bit % 8);
        }
    }
}

/// Unpack `bytes` by an XML struct: name -> logical value (a 29-bit address
/// field is the address's bits 3..31, returned as the address).
fn unpack_xml(f: &[XmlField], bytes: &[u8]) -> Vec<(String, u32)> {
    f.iter()
        .map(|x| {
            let mut v = get(bytes, x.start, x.size);
            if x.minus_one {
                v += 1;
            }
            if x.ty == "address" && x.size == 29 {
                v <<= 3;
            }
            (x.name.clone(), v)
        })
        .collect()
}

#[test]
fn fields_match_the_packet_xml() {
    let x = d0_struct();
    assert_eq!(x.len(), FIELDS.len());
    for (a, b) in x.iter().zip(FIELDS.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!((a.start, a.size), (b.start, b.size), "{}", a.name);
        let k = match (a.ty.as_str(), a.minus_one) {
            ("bool", _) => Kind::Bool,
            ("uint", false) => Kind::Uint,
            ("uint", true) => Kind::UintMinusOne,
            ("address", _) => Kind::Address,
            (t, _) => panic!("type {t}"),
        };
        assert_eq!(k, b.kind, "{}", a.name);
    }
    // the record is 32 bytes: the last field ends at bit 256, none overlap
    let end = x.iter().map(|f| f.start + f.size).max().unwrap();
    assert_eq!(end as usize, RECORD_SIZE * 8);
    let mut seen = [false; 256];
    for f in &x {
        let lo = if f.ty == "address" && f.size == 29 { f.start - 3 } else { f.start };
        let hi = f.start + f.size;
        for b in lo..hi {
            // the 3 flag bits below a code address are their own fields
            if f.ty == "address" && f.size == 29 && b < f.start {
                continue;
            }
            assert!(!seen[b as usize], "bit {b} twice ({})", f.name);
            seen[b as usize] = true;
        }
    }
    let used = seen.iter().filter(|s| **s).count();
    // bit 14 is unused in this layout; every other bit of the 24 flag bits
    // and the 232 above is a field
    assert!(!seen[14]);
    assert_eq!(used, 255);
    // the generated packer agrees on the size
    let g = read("driver/src/v3d/generated/v3d_packet_v71_pack.h");
    assert!(g.contains("#define V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_length     32"));
    // the struct is the D0's, from the "exclusive to 2712D0" comment
    let xs = xml();
    let i = xs.find("<struct name=\"GL Shader State Record Draw Index\"").unwrap();
    assert!(xs[..i].trim_end().ends_with("<!-- This one is exclusive to 2712D0 (V3D 7.1.10) -->"));
    assert_eq!(RECORD_SIZE, 32);
}

fn vertex(base: u32) -> VertexDesc {
    VertexDesc {
        prog: ProgramDesc { code: base, uniforms: base + 0x100, threads: 4, single_seg: true, propagate_nans: false },
        vpm_input_size: 3,
        vpm_output_size: 5,
        min_input_segments: 1,
        min_output_segments: 0,
        ..Default::default()
    }
}

fn sample_draw() -> DrawDesc {
    DrawDesc {
        fs: FragmentDesc {
            prog: ProgramDesc { code: 0x1060, uniforms: 0x13000, threads: 4, single_seg: false, propagate_nans: false },
            num_varyings: 6,
            ..Default::default()
        },
        vs: vertex(0x1090),
        cs: vertex(0x1108),
        point_size: false,
    }
}

#[test]
fn pack_roundtrips_through_an_xml_unpacker() {
    let x = d0_struct();
    // every field gets a distinct value in turn, plus an all-set record
    let d = DrawDesc {
        fs: FragmentDesc {
            prog: ProgramDesc {
                code: 0x00ab_cde8,
                uniforms: 0x1234_5678,
                threads: 4,
                single_seg: true,
                propagate_nans: true,
            },
            writes_z: true,
            disable_ez: true,
            num_varyings: 255,
            uses_center_w: true,
            uses_pid: true,
            lock_scoreboard_on_first_thrsw: true,
            uses_implicit_point_line_varyings: false,
            sample_rate_shading: true,
        },
        vs: VertexDesc {
            prog: ProgramDesc {
                code: 0x00f0_0f08,
                uniforms: 0x2222_2222,
                threads: 2,
                single_seg: true,
                propagate_nans: true,
            },
            vpm_input_size: 15,
            vpm_output_size: 9,
            min_input_segments: 16,
            min_output_segments: 15,
            uses_vid: true,
            uses_iid: true,
            uses_base_instance: true,
            ..Default::default()
        },
        cs: VertexDesc {
            prog: ProgramDesc {
                code: 0x7fff_fff8,
                uniforms: 0xffff_fffc,
                threads: 1,
                single_seg: false,
                propagate_nans: true,
            },
            vpm_input_size: 1,
            vpm_output_size: 15,
            min_input_segments: 2,
            min_output_segments: 7,
            uses_vid: true,
            uses_iid: false,
            uses_base_instance: true,
            ..Default::default()
        },
        point_size: true,
    };
    for d in [d, sample_draw()] {
        let rec = describe(BCM2712_D0, &d).unwrap();
        let bytes = rec.pack();
        let un = unpack_xml(&x, &bytes);
        assert_eq!(un.len(), FIELDS.len());
        for (i, (name, v)) in un.iter().enumerate() {
            assert_eq!(name, FIELDS[i].name);
            assert_eq!(*v, rec.value(i), "{name}");
        }
    }
}

// ---- Mesa's records, from the dumps ------------------------------------

struct Dumped {
    /// (label, value); an address line's value is its absolute address
    vals: Vec<(String, u32)>,
}

fn parse_records(clif: &str) -> Vec<Dumped> {
    let mut out: Vec<Dumped> = Vec::new();
    let mut cur: Option<Dumped> = None;
    for line in clif.lines() {
        let l = line.strip_prefix("MESA: info:").unwrap_or(line).trim();
        if l.starts_with("@format") {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            if l == "@format shadrec_gl_main" {
                cur = Some(Dumped { vals: Vec::new() });
            }
            continue;
        }
        if l.starts_with('@') || l.is_empty() && cur.is_none() {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        let Some(rest) = l.strip_prefix("/*") else {
            // any other packet name ends the record
            out.push(cur.take().unwrap());
            continue;
        };
        let (label, val) = rest.split_once("*/").unwrap();
        let label = label.trim().trim_end_matches(':').trim().to_string();
        let val = val.trim();
        let v = if val.starts_with('[') {
            let i = val.rfind("/* 0x").unwrap() + 5;
            u32::from_str_radix(val[i..].split_whitespace().next().unwrap(), 16).unwrap()
        } else {
            val.split_whitespace().next().unwrap().parse().unwrap()
        };
        c.vals.push((label, v));
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    out
}

impl Dumped {
    fn get(&self, name: &str) -> u32 {
        self.vals.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}")).1
    }
    /// The record's bytes as the PLAIN 7.1 layout placed them.
    fn plain_bytes(&self, plain: &[XmlField]) -> [u8; 32] {
        let mut b = [0u8; 32];
        assert_eq!(self.vals.len(), plain.len());
        for f in plain {
            let v = self.get(&f.name);
            if f.ty == "address" && f.size == 29 {
                set(&mut b, f.start, 29, v >> 3);
            } else if f.minus_one {
                set(&mut b, f.start, f.size, v - 1);
            } else {
                set(&mut b, f.start, f.size, v);
            }
        }
        b
    }
}

/// A draw description from a dumped record: the flags are read from the
/// plain-placed bytes at the D0 layout, everything else by name.
fn desc_from(d: &Dumped, raw: &[u8; 32], d0: &[XmlField]) -> DrawDesc {
    let bit = |name: &str| {
        let f = d0.iter().find(|f| f.name == name).unwrap();
        get(raw, f.start, 1) != 0
    };
    let mk = |p: &str, lc: &str, id: &str| VertexDesc {
        prog: ProgramDesc {
            code: d.get(&format!("{p} Shader Code Address")),
            uniforms: d.get(&format!("{p} Shader Uniforms Address")),
            threads: if d.get(&format!("{p} Shader 4-way threadable")) != 0 { 4 } else { 2 },
            single_seg: d.get(&format!("{p} Shader start in final thread section")) != 0,
            propagate_nans: d.get(&format!("{p} Shader Propagate NaNs")) != 0,
        },
        vpm_input_size: d.get(&format!("{p} Shader input VPM segment size")),
        vpm_output_size: d.get(&format!("{p} Shader output VPM segment size")),
        min_input_segments: d.get(&format!("Min {lc} Shader input segments required in play")),
        min_output_segments: d
            .get(&format!("Min {lc} Shader output segments required in play in addition to VCM cache size")),
        uses_vid: bit(&format!("Vertex ID read by {id} shader")),
        uses_iid: bit(&format!("Instance ID read by {id} shader")),
        uses_base_instance: bit(&format!("Base Instance ID read by {id} shader")),
        uses_base_vertex: bit(if id == "vertex" { "vs_basevertex" } else { "cs_basevertex" }),
        uses_draw_index: bit(if id == "vertex" { "vs_drawindex" } else { "cs_drawindex" }),
    };
    assert_eq!(
        bit("Any shader reads hardware-written Primitive ID"),
        bit("Insert Primitive ID as first varying to fragment shader"),
        "the description sets both primitive-id bits from one flag"
    );
    DrawDesc {
        fs: FragmentDesc {
            prog: ProgramDesc {
                code: d.get("Fragment Shader Code Address"),
                uniforms: d.get("Fragment Shader Uniforms Address"),
                threads: if d.get("Fragment Shader 4-way threadable") != 0 { 4 } else { 2 },
                single_seg: d.get("Fragment Shader start in final thread section") != 0,
                propagate_nans: d.get("Fragment Shader Propagate NaNs") != 0,
            },
            writes_z: bit("Fragment shader does Z writes"),
            disable_ez: bit("Turn off early-z test"),
            num_varyings: d.get("Number of varyings in Fragment Shader"),
            uses_center_w: bit("Fragment shader uses real pixel centre W in addition to centroid W2"),
            uses_pid: bit("Insert Primitive ID as first varying to fragment shader"),
            lock_scoreboard_on_first_thrsw: bit("Do scoreboard wait on first thread switch"),
            // bit 21 is not in the dump: Mesa's value for a program is the
            // table's own choice here
            uses_implicit_point_line_varyings: false,
            sample_rate_shading: bit("Enable Sample Rate Shading"),
        },
        vs: mk("Vertex", "Vertex", "vertex"),
        cs: mk("Coordinate", "Coord", "coordinate"),
        point_size: bit("Point size in shaded vertex data"),
    }
}

const SHAPES: [&str; 6] = ["gl_scissor", "gl_scisbox", "gl_writemask", "gl_viewport", "gl_colormask", "gl_fbods"];

#[test]
fn mesa_records_of_the_gl_test_shapes() {
    let d0 = d0_struct();
    let plain = plain_struct();
    let mut total = 0;
    let mut zw = 0;
    let mut pid = 0;
    for s in SHAPES {
        let out = read(&format!("tools/v3d/pi5/{s}.stdout"));
        assert!(out.contains("renderer V3D 7.1.10.2"), "{s}: not the 7.1.10 revision");
        let recs = parse_records(&read(&format!("tools/v3d/pi5/{s}.clif")));
        assert!(!recs.is_empty(), "{s}: no shader record");
        for (n, r) in recs.iter().enumerate() {
            let raw = r.plain_bytes(&plain);
            let desc = desc_from(r, &raw, &d0);
            let rec = describe(BCM2712_D0, &desc).unwrap_or_else(|e| panic!("{s}[{n}]: {}", e.message()));
            let got = rec.pack();
            // byte 3 on: the same offsets in both layouts
            assert_eq!(&got[3..], &raw[3..], "{s}[{n}] bytes 3..");
            // flag bits 0..20, as far as the plain decode saw them
            let mask = 0x1f_ffffu32;
            let g = u32::from_le_bytes([got[0], got[1], got[2], 0]) & mask;
            let e = u32::from_le_bytes([raw[0], raw[1], raw[2], 0]) & mask;
            assert_eq!(g, e, "{s}[{n}] flag bits 0..20: {g:#x} vs {e:#x}");
            // bits 21.. are the table's: implicit varyings disabled (bit 21),
            // and the unseen no-prim-pack and never-defer bits 22, 23 clear
            assert_eq!(got[2] >> 5 & 1, 1);
            assert_eq!(got[2] >> 6, 0);
            // the D0 flags the dump shows at the draw-index positions
            assert!(!rec.cs_basevertex && !rec.cs_drawindex && !rec.vs_basevertex && !rec.vs_drawindex);
            assert!(!rec.never_defer_fep_depth_writes);
            // the unpacker reads the record back to the same values
            for (i, (name, v)) in unpack_xml(&d0, &got).iter().enumerate() {
                assert_eq!(*v, rec.value(i), "{s}[{n}] {name}");
            }
            total += 1;
            zw += rec.fragment_shader_does_z_writes as u32;
            pid += rec.insert_primitive_id_as_first_varying_to_fragment_shader as u32;
        }
    }
    assert!(total >= 20, "{total} records");
    // the colour-only shapes have no Z writer and no primitive id
    assert_eq!(pid, 0);
    assert_eq!(zw, 0);
}

#[test]
fn scissor_records_by_hand() {
    // gl_scissor.clif, the second GL_SHADER's record (the yellow quad):
    // 6 varyings, VS output 2, three 4-way programs, the VS and CS in the
    // final thread section, the addresses the dump prints.
    let rec = parse_records(&read("tools/v3d/pi5/gl_scissor.clif"));
    assert_eq!(rec.len(), 2);
    let mut d = sample_draw();
    d.fs.prog = ProgramDesc { code: 0x1180, uniforms: 0x13090, threads: 4, single_seg: false, propagate_nans: false };
    d.fs.num_varyings = 6;
    d.fs.uses_center_w = true;
    d.vs = vertex(0x11e0);
    d.vs.prog.uniforms = 0x13090;
    d.vs.vpm_input_size = 0;
    d.vs.vpm_output_size = 2;
    d.cs = vertex(0x12b0);
    d.cs.prog.uniforms = 0x130d8;
    d.cs.vpm_input_size = 0;
    d.cs.vpm_output_size = 1;
    let b = pack_draw(BCM2712_D0, &d).unwrap();
    // flags: enable clipping (bit 1), real pixel centre W (bit 15) and
    // Disable implicit point/line varyings (bit 21)
    assert_eq!(&b[0..3], &[0x02, 0x80, 0x20]);
    assert_eq!(b[3], 6);
    // cs out 1 / min 0, cs in 0 / min-1 0, vs out 2, vs in 0
    assert_eq!(&b[4..8], &[0x01, 0x00, 0x02, 0x00]);
    // fragment: code | 4-way, uniforms; vertex; coordinate
    assert_eq!(&b[8..12], &0x1181u32.to_le_bytes());
    assert_eq!(&b[12..16], &0x13090u32.to_le_bytes());
    assert_eq!(&b[16..20], &0x11e3u32.to_le_bytes()); // 4-way, final section
    assert_eq!(&b[20..24], &0x13090u32.to_le_bytes());
    assert_eq!(&b[24..28], &0x12b3u32.to_le_bytes());
    assert_eq!(&b[28..32], &0x130d8u32.to_le_bytes());
    // the same record as the dump's, flags aside
    let raw = rec[1].plain_bytes(&plain_struct());
    assert_eq!(&b[3..], &raw[3..]);
}

// ---- the rules of v3d71-record-u11.md section 4 ------------------------

#[test]
fn rule1_only_the_draw_index_record_and_named_refusals() {
    for (ver, rev, ok) in [
        (71, 10, true),
        (71, 11, true),
        (71, 7, false),
        (71, 9, false),
        (71, 0, false),
        (42, 0, false),
        (42, 10, false),
        (40, 0, false),
    ] {
        let r = describe(Revision { ver, rev }, &sample_draw());
        assert_eq!(r.is_ok(), ok, "{ver}.{rev}");
        if !ok {
            assert_eq!(r, Err(Refusal::NoDrawIndexRecord { ver, rev }));
        }
    }
    let m = Refusal::NoDrawIndexRecord { ver: 71, rev: 7 }.message();
    assert!(m.contains("7.1.7") && m.contains("Draw Index"), "{m}");
    // the revision rule is Mesa's v3d_device_has_draw_index
    let h = read("third_party/mesa-26.2.1/src/broadcom/common/v3d_device_info.h");
    assert!(h.contains("v3d_device_has_draw_index"));
    assert!(h.contains("ver == 71 && devinfo->rev >= 10") || h.contains("rev >= 10"));
    assert!(has_draw_index_record(BCM2712_D0));
    // 32 bytes; the plain 7.1 record is not this one: its bit 5 is Vertex ID
    // read by vertex shader, here cs_basevertex
    let plain = plain_struct();
    let p5 = plain.iter().find(|f| f.start == 5).unwrap();
    assert_eq!(p5.name, "Vertex ID read by vertex shader");
    assert_eq!(FIELDS[ix::CS_BASEVERTEX].start, 5);
    let mut d = sample_draw();
    d.vs.uses_vid = true;
    let b = pack_draw(BCM2712_D0, &d).unwrap();
    // Vertex ID read by vertex shader is bit 7 here, not 5
    assert_eq!(b[0] >> 5 & 1, 0);
    assert_eq!(b[0] >> 7 & 1, 1);
}

#[test]
fn rule2_three_programs_fill_three_groups() {
    let d = sample_draw();
    let r = describe(BCM2712_D0, &d).unwrap();
    assert_eq!(r.fragment_shader_code_address, 0x1060);
    assert_eq!(r.vertex_shader_code_address, 0x1090);
    assert_eq!(r.coordinate_shader_code_address, 0x1108);
    assert_eq!(r.fragment_shader_uniforms_address, 0x13000);
    assert_eq!(r.vertex_shader_uniforms_address, 0x1090 + 0x100);
    assert_eq!(r.coordinate_shader_uniforms_address, 0x1108 + 0x100);
    // thread counts: 4 sets the flag, 1 and 2 do not; others are refused
    for (t, f) in [(1, false), (2, false), (4, true)] {
        let mut d = d;
        d.fs.prog.threads = t;
        assert_eq!(describe(BCM2712_D0, &d).unwrap().fragment_shader_4_way_threadable, f);
    }
    for t in [0, 3, 8] {
        let mut d = d;
        d.vs.prog.threads = t;
        assert_eq!(describe(BCM2712_D0, &d), Err(Refusal::BadThreadCount(Stage::Vertex, t)));
    }
    let mut d2 = d;
    d2.cs.prog.code = 0x1104;
    assert_eq!(describe(BCM2712_D0, &d2), Err(Refusal::CodeAddressUnaligned(Stage::Coordinate)));
    let mut d3 = d;
    d3.cs.vpm_output_size = 16;
    assert!(matches!(describe(BCM2712_D0, &d3), Err(Refusal::VpmFieldRange(Stage::Coordinate, _))));
    let mut d4 = d;
    d4.vs.min_input_segments = 0;
    assert!(matches!(describe(BCM2712_D0, &d4), Err(Refusal::VpmFieldRange(Stage::Vertex, _))));
    let mut d5 = d;
    d5.fs.num_varyings = 256;
    assert_eq!(describe(BCM2712_D0, &d5), Err(Refusal::TooManyVaryings(256)));
    // single seg is the "start in final thread section" bit
    let mut d6 = d;
    d6.vs.prog.single_seg = false;
    assert!(!describe(BCM2712_D0, &d6).unwrap().vertex_shader_start_in_final_thread_section);
    assert!(describe(BCM2712_D0, &d).unwrap().vertex_shader_start_in_final_thread_section);
}

#[test]
fn rule3_the_four_draw_index_bits_stay_zero_and_a_need_is_refused() {
    let x = d0_struct();
    let pos = |n: &str| x.iter().find(|f| f.name == n).unwrap().start;
    assert_eq!([pos("cs_basevertex"), pos("cs_drawindex"), pos("vs_basevertex"), pos("vs_drawindex")], [5, 6, 10, 11]);
    for mask in 0..64u32 {
        let mut d = sample_draw();
        d.fs.writes_z = mask & 1 != 0;
        d.fs.disable_ez = mask & 2 != 0;
        d.fs.uses_pid = mask & 4 != 0;
        d.vs.uses_vid = mask & 8 != 0;
        d.cs.uses_iid = mask & 16 != 0;
        d.vs.uses_base_instance = mask & 32 != 0;
        let b = pack_draw(BCM2712_D0, &d).unwrap();
        let w = u32::from_le_bytes([b[0], b[1], b[2], 0]);
        for bit in [5, 6, 10, 11] {
            assert_eq!(w >> bit & 1, 0, "mask {mask}");
        }
    }
    let mut d = sample_draw();
    d.vs.uses_base_vertex = true;
    assert_eq!(describe(BCM2712_D0, &d), Err(Refusal::BaseVertex(Stage::Vertex)));
    let mut d = sample_draw();
    d.cs.uses_draw_index = true;
    assert_eq!(describe(BCM2712_D0, &d), Err(Refusal::DrawIndex(Stage::Coordinate)));
    assert!(Refusal::BaseVertex(Stage::Vertex).message().contains("gl_BaseVertex"));
    assert!(Refusal::DrawIndex(Stage::Coordinate).message().contains("gl_DrawID"));
}

#[test]
fn rule4_writes_z_and_disable_ez_are_copied_and_fep_is_zero() {
    let x = d0_struct();
    let pos = |n: &str| x.iter().find(|f| f.name == n).unwrap().start;
    let (zw, ez, fep) =
        (pos("Fragment shader does Z writes"), pos("Turn off early-z test"), pos("Never defer FEP depth writes"));
    assert_eq!((zw, ez, fep), (12, 13, 23));
    for (w, e) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut d = sample_draw();
        d.fs.writes_z = w;
        d.fs.disable_ez = e;
        let b = pack_draw(BCM2712_D0, &d).unwrap();
        let un = unpack_xml(&x, &b);
        let v = |n: &str| un.iter().find(|(m, _)| m == n).unwrap().1;
        assert_eq!(v("Fragment shader does Z writes"), w as u32);
        assert_eq!(v("Turn off early-z test"), e as u32);
        // the FEP flag is 0 whatever the program does
        assert_eq!(v("Never defer FEP depth writes"), 0);
        assert_eq!(b[2] >> 7, 0);
    }
    // a gl_FragDepth / discard program: the compiler's writes_z = disable_ez = 1
    let mut d = sample_draw();
    d.fs.writes_z = true;
    d.fs.disable_ez = true;
    let r = describe(BCM2712_D0, &d).unwrap();
    assert!(r.fragment_shader_does_z_writes && r.turn_off_early_z_test && !r.never_defer_fep_depth_writes);
    // the record never derives anything from writes_z alone
    let mut e = sample_draw();
    e.fs.writes_z = true;
    assert!(!describe(BCM2712_D0, &e).unwrap().turn_off_early_z_test);
}

#[test]
fn rule5_no_early_z_in_cfg_bits_and_tile_mode_keeps_the_disable() {
    let x = xml();
    let section = |open: &str| -> String {
        let mut it = x.lines().skip_while(|l| !l.contains(open));
        it.next().unwrap();
        it.take_while(|l| !l.contains("</packet>")).collect::<Vec<_>>().join("\n")
    };
    let v42 = section("<packet code=\"96\" name=\"Cfg Bits\" max_ver=\"42\">");
    let v71 = section("<packet code=\"96\" name=\"Cfg Bits\" min_ver=\"71\">");
    assert!(v42.contains("Early Z enable") && v42.contains("Early Z updates enable"));
    assert!(!v71.contains("Early Z"), "7.1's CFG_BITS has early-z fields");
    assert!(v71.contains("Z updates enable") && v71.contains("Depth-Test Function"));
    // the tile mode's disable, which the table keeps set, is a packet field
    assert!(x.matches("\"Early-Z disable\" size=\"1\" start=\"46\"").count() >= 1);
    // and the record has no early-z enable of its own
    assert!(FIELDS.iter().all(|f| !f.name.contains("Early Z") && !f.name.contains("Early-Z")));
}
