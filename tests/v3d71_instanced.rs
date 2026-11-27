// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 instanced draw records against what Mesa 26.2.1 emitted on
//! the Pi 5 for `glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 3)`
//! (tools/v3d/pi5/gl_instanced.clif; the GL calls are
//! tools/v3d/pi5/src/gl_shapes.py `instanced`: `a_pos` per vertex at
//! location 0 from one buffer, `a_color` and `a_off` at
//! `glVertexAttribDivisor` 1 from a second, 24-byte-stride buffer).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_instanced
//! ```
//!
//! (`gpu-v3d` alone builds every test but the table's, [`table`].)
//!
//! Pinned from the clif, read as text:
//!
//! * the binning list's one draw packet, `VERTEX_ARRAY_INSTANCED_PRIMS`
//!   (mode, instance length, number of instances, first vertex), with no
//!   `BASE_VERTEX_BASE_INSTANCE` (base instance 0) and no plain
//!   `VERTEX_ARRAY_PRIMS`;
//! * the `GL_SHADER` packet's attribute array count and the three
//!   `shadrec_gl_attr` records, their instance divisors in particular;
//! * the shader record's instance id flags (this program reads none) and
//!   the render list's `SET_INSTANCEID 0`.
//!
//! Compared with what yakogl's 7.1 modules give for the same GL calls:
//! prim71 (`pack_draw`, decoded by the xml's own field positions), attr71
//! (`plan`) and glue71 (`prim`, `attrs`) from the GL core's `GlesDraw`.
//! And what the 7.1 table (`yakogl_backend_v3d71`, through backend71's
//! `Draw71` and the C builder v3d_job71.c) submits for the shape's draw: one
//! `VERTEX_ARRAY_INSTANCED_PRIMS` with Mesa's fields and Mesa's attribute
//! records, divisors included (module `table`).
#![allow(dead_code)]

/// The GL core's back-end types, as every test reaches them: glue71 reads
/// `crate::backend`.
mod backend {
    pub use yakogl::backend::*;
}

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(clippy::if_same_then_else)]
mod attr71;
#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;
#[path = "../src/v3d/draw71.rs"]
mod draw71;
#[path = "../src/v3d/ffi71.rs"]
mod ffi71;
#[path = "../src/v3d/fill71.rs"]
mod fill71;
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/glue71.rs"]
mod glue71;
#[path = "../src/v3d/ident71.rs"]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
mod limits71;
#[path = "../src/v3d/prim71.rs"]
mod prim71;
#[path = "../src/v3d/record71.rs"]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(clippy::eq_op)]
mod rt71;
#[path = "../src/v3d/uniform71.rs"]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use backend::*;

#[cfg(feature = "gpu-v3d71")]
mod v3d71_table_rig;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const CLIF: &str = "tools/v3d/pi5/gl_instanced.clif";

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/* ---- the clif, as text ------------------------------------------------------------ */

/// One decoded block of the clif: a packet (`GL_SHADER`, ...) or a record
/// (`shadrec_gl_attr`, `shadrec_gl_main`), with its `/* field: */ value`
/// rows in order.
#[derive(Debug)]
struct Block {
    name: String,
    fields: Vec<(String, String)>,
}

impl Block {
    fn raw(&self, f: &str) -> &str {
        let v = self.fields.iter().find(|(n, _)| n == f);
        v.unwrap_or_else(|| panic!("{}: no field {f:?}", self.name)).1.as_str()
    }

    /// An integer field; an address (`[buf+off] /* 0x... */`) is the bus
    /// address in the trailing comment.
    fn num(&self, f: &str) -> u64 {
        let v = self.raw(f);
        if v.starts_with('[') {
            let h = &v[v.rfind("/* 0x").unwrap_or_else(|| panic!("{f}: {v}")) + 5..];
            return u64::from_str_radix(h.split_whitespace().next().unwrap(), 16).unwrap();
        }
        let t = v.split_whitespace().next().unwrap();
        t.parse().unwrap_or_else(|_| panic!("{}: {f} = {v:?}", self.name))
    }

    fn flag(&self, f: &str) -> bool {
        match self.num(f) {
            0 => false,
            1 => true,
            n => panic!("{f}: {n}"),
        }
    }
}

/// Every block of the clif, in file order. A block opens at a bare upper
/// case packet name or at `@format shadrec_*`; any other `@` line closes it.
fn blocks() -> Vec<Block> {
    let text = read(CLIF);
    let mut out: Vec<Block> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let l = line.strip_prefix("MESA: info:").unwrap_or(line).trim();
        if let Some(rest) = l.strip_prefix("/*") {
            if !open {
                continue;
            }
            let (name, value) = rest.split_once(":").unwrap_or_else(|| panic!("{l}"));
            let value = value.trim_start().strip_prefix("*/").unwrap_or_else(|| panic!("{l}")).trim();
            out.last_mut().unwrap().fields.push((name.trim().to_string(), value.to_string()));
        } else if let Some(f) = l.strip_prefix("@format shadrec_") {
            let name = format!("shadrec_{}", f.split_whitespace().next().unwrap());
            out.push(Block { name, fields: vec![] });
            open = true;
        } else if l.starts_with('@') || l.is_empty() {
            open = false;
        } else if l.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            out.push(Block { name: l.to_string(), fields: vec![] });
            open = true;
        } else {
            open = false;
        }
    }
    out
}

fn only<'a>(b: &'a [Block], name: &str) -> &'a Block {
    let v: Vec<&Block> = b.iter().filter(|x| x.name == name).collect();
    assert_eq!(v.len(), 1, "{CLIF}: {name} blocks");
    v[0]
}

/* ---- Mesa's values, as the shape writes them -------------------------------------- */

/// glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 3).
const MODE: u32 = prim71::GL_TRIANGLES;
const COUNT: u64 = 6;
const INSTANCES: u64 = 3;
/// The buffers Mesa placed the two arrays in (the clif's resource names).
const VBO: u32 = 0xe000;
const IBO: u32 = 0xf000;

/// One clif attribute record's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MesaAttr {
    address: u32,
    vec: u32,
    ty: u32,
    signed: bool,
    normalized: bool,
    read_as_int: bool,
    cs: u32,
    vs: u32,
    divisor: u32,
    stride: u32,
    max_index: u32,
}

fn mesa_attrs() -> Vec<MesaAttr> {
    blocks()
        .iter()
        .filter(|b| b.name == "shadrec_gl_attr")
        .map(|b| MesaAttr {
            address: b.num("Address") as u32,
            vec: b.num("Vec size") as u32,
            ty: b.num("Type") as u32,
            signed: b.flag("Signed int type"),
            normalized: b.flag("Normalized int type"),
            read_as_int: b.flag("Read as int/uint"),
            cs: b.num("Number of values read by Coordinate shader") as u32,
            vs: b.num("Number of values read by Vertex shader") as u32,
            divisor: b.num("Instance Divisor") as u32,
            stride: b.num("Stride") as u32,
            max_index: b.num("Maximum Index") as u32,
        })
        .collect()
}

/// attr71's record as the clif decodes it (a 2-bit vec size of 0 is 4).
fn as_mesa(r: &attr71::Record71) -> MesaAttr {
    MesaAttr {
        address: r.address,
        vec: if r.vec_size == 0 { 4 } else { r.vec_size as u32 },
        ty: r.ty as u32,
        signed: r.signed,
        normalized: r.normalized,
        read_as_int: r.read_as_int,
        cs: r.cs_values as u32,
        vs: r.vs_values as u32,
        divisor: r.divisor as u32,
        stride: r.stride,
        max_index: r.max_index,
    }
}

/// The values each stage of Mesa's program reads, from the clif records
/// (the program's `vattr_sizes`; this test does not compile the shader).
fn reads() -> ([u8; attr71::MAX_ATTRS], [u8; attr71::MAX_ATTRS]) {
    let (mut cs, mut vs) = ([0u8; attr71::MAX_ATTRS], [0u8; attr71::MAX_ATTRS]);
    for (i, a) in mesa_attrs().iter().enumerate() {
        (cs[i], vs[i]) = (a.cs as u8, a.vs as u8);
    }
    (cs, vs)
}

/* ---- the xml ---------------------------------------------------------------------- */

fn xml_attr(line: &str, k: &str) -> Option<String> {
    let p = line.find(&format!("{k}=\""))? + k.len() + 2;
    Some(line[p..p + line[p..].find('"').unwrap()].to_string())
}

/// `(code, [(field, start, size)])` of the xml packet `name`.
fn xml_packet(name: &str) -> (u8, Vec<(String, u32, u32)>) {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let s = xml.find(&format!("name=\"{name}\"")).unwrap_or_else(|| panic!("{name}"));
    let s = xml[..s].rfind("<packet").unwrap();
    let body = &xml[s..s + xml[s..].find("</packet>").unwrap()];
    let code = xml_attr(body.lines().next().unwrap(), "code").unwrap().parse().unwrap();
    let fields = body
        .lines()
        .filter(|l| l.trim_start().starts_with("<field "))
        .map(|l| {
            (
                xml_attr(l, "name").unwrap(),
                xml_attr(l, "start").unwrap().parse().unwrap(),
                xml_attr(l, "size").unwrap().parse().unwrap(),
            )
        })
        .collect();
    (code, fields)
}

/// Decode a packet by the xml's field positions: `(field, value)` rows.
fn xml_decode(name: &str, p: &[u8]) -> Vec<(String, u64)> {
    let (code, fields) = xml_packet(name);
    assert_eq!(p[0], code, "{name}: opcode");
    let bits = fields.iter().map(|f| f.1 + f.2).max().unwrap();
    assert_eq!(p.len(), 1 + (bits as usize).div_ceil(8), "{name}: length");
    fields
        .iter()
        .map(|(n, st, sz)| {
            let v = (0..*sz).fold(0u64, |v, i| {
                let b = st + i;
                v | (u64::from(p[1 + (b / 8) as usize] >> (b % 8) & 1) << i)
            });
            (n.clone(), v)
        })
        .collect()
}

/// The clif block `b` read as the xml packet `name`: `(field, value)` rows
/// in the xml's field order (the clif prints the xml's field names).
fn clif_by_xml(b: &Block, name: &str) -> Vec<(String, u64)> {
    let (_, fields) = xml_packet(name);
    fields.iter().map(|(n, _, _)| (n.clone(), b.num(n))).collect()
}

/* ---- the GL core's draw ----------------------------------------------------------- */

/// The GlesDraw the GL core hands a back end for the shape's draw: the
/// three arrays as buffer objects fetched in place, first vertex 0.
fn core_draw() -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.prim = GLES_PRIM_TRIANGLES;
    d.index_count = COUNT as u32;
    d.instance_count = INSTANCES as u32;
    let arr = |vi_reg: u8, components: u8, stride: u32, offset: u32, buf: u32, per_instance: u8| GlesAttrib {
        vi_reg,
        components,
        format: GLES_ATTRIBF_FLOAT32,
        stride,
        offset,
        buffer_gpu: u64::from(buf),
        per_instance,
        divisor: u16::from(per_instance),
        buffer_bytes: 96,
        ..GlesAttrib::ZERO
    };
    d.attribs[0] = arr(0, 4, 16, 0, VBO, 0);
    d.attribs[1] = arr(1, 4, 24, 0, IBO, 1);
    d.attribs[2] = arr(2, 2, 24, 16, IBO, 1);
    d.num_attribs = 3;
    d
}

/* ---- the draw packet ---------------------------------------------------------------- */

const PRIMS: &str = "Vertex Array Instanced Prims";

#[test]
fn the_binning_list_draws_once_instanced_without_a_base_packet() {
    let b = blocks();
    let p = only(&b, "VERTEX_ARRAY_INSTANCED_PRIMS");
    assert_eq!(
        clif_by_xml(p, PRIMS),
        [
            ("Index of First Vertex".to_string(), 0),
            ("Number of Instances".to_string(), INSTANCES),
            ("Instance Length".to_string(), COUNT),
            ("mode".to_string(), u64::from(MODE)),
        ]
    );
    for none in [
        "VERTEX_ARRAY_PRIMS",
        "BASE_VERTEX_BASE_INSTANCE",
        "INDEXED_PRIM_LIST",
        "INDEXED_INSTANCED_PRIM_LIST",
        "INDEX_BUFFER_SETUP",
    ] {
        assert!(b.iter().all(|x| x.name != none), "{CLIF}: a {none} packet");
    }
    /* the draw follows the GL_SHADER packet that names its three arrays */
    let i = b.iter().position(|x| x.name == "VERTEX_ARRAY_INSTANCED_PRIMS").unwrap();
    assert_eq!(b[i - 1].name, "GL_SHADER");
    assert_eq!(b[i - 1].num("number of attribute arrays"), 3);
}

#[test]
fn prim71_packs_the_same_packet() {
    let p = only(&blocks(), "VERTEX_ARRAY_INSTANCED_PRIMS").num("mode");
    let mut d = prim71::Draw::arrays(MODE, 0, COUNT);
    d.instances = INSTANCES;
    let bytes = prim71::pack_draw(&d).unwrap();
    assert_eq!(bytes[0], prim71::OP_VERTEX_ARRAY_INSTANCED_PRIMS, "no base packet before it");
    assert_eq!(xml_decode(PRIMS, &bytes), clif_by_xml(only(&blocks(), "VERTEX_ARRAY_INSTANCED_PRIMS"), PRIMS));
    assert_eq!(p, u64::from(prim71::prim_mode(MODE).unwrap()));
    /* one instance is the plain packet: Mesa's other references draw with it */
    d.instances = 1;
    assert_eq!(prim71::pack_draw(&d).unwrap()[0], prim71::OP_VERTEX_ARRAY_PRIMS);
}

#[test]
fn glue71_maps_the_core_draw_to_the_same_packet() {
    let d = core_draw();
    assert!(glue71::refusal(&d).is_none(), "{:?}", glue71::refusal(&d));
    let p = glue71::prim(&d, None).unwrap();
    assert_eq!((p.mode, p.first, p.count, p.instances), (MODE, 0, COUNT, INSTANCES));
    assert_eq!((p.indices, p.base_vertex, p.base_instance), (None, 0, 0));
    let bytes = prim71::pack_draw(&p).unwrap();
    assert_eq!(xml_decode(PRIMS, &bytes), clif_by_xml(only(&blocks(), "VERTEX_ARRAY_INSTANCED_PRIMS"), PRIMS));
}

/* ---- the attribute records ----------------------------------------------------------- */

#[test]
fn mesa_records_the_divisors() {
    let m = mesa_attrs();
    assert_eq!(m.len(), 3);
    let f = |address, vec, cs, vs, divisor, stride| MesaAttr {
        address,
        vec,
        ty: attr71::TYPE_FLOAT as u32,
        signed: false,
        normalized: false,
        read_as_int: false,
        cs,
        vs,
        divisor,
        stride,
        max_index: attr71::MAX_INDEX,
    };
    assert_eq!(
        m,
        [f(VBO, 4, 4, 4, 0, 16), f(IBO, 4, 0, 4, 1, 24), f(IBO + 16, 2, 2, 2, 1, 24)],
        "a_pos per vertex; a_color and a_off at divisor 1"
    );
}

#[test]
fn attr71_plans_the_same_records() {
    let a = |components, stride, offset, buffer_addr, divisor| attr71::Attr {
        enabled: true,
        components,
        gl_type: attr71::GL_FLOAT,
        stride,
        offset,
        buffer_addr,
        divisor,
        ..attr71::Attr::disabled()
    };
    let attrs = [a(4, 16, 0, VBO, 0), a(4, 24, 0, IBO, 1), a(2, 24, 16, IBO, 1)];
    let (cs, vs) = reads();
    let plan = attr71::plan(&attrs, &cs, &vs, false, 0x1_0000).unwrap();
    assert_eq!(plan.count, 3, "GL_SHADER's number of attribute arrays");
    let got: Vec<MesaAttr> = plan.records[..plan.count].iter().map(as_mesa).collect();
    assert_eq!(got, mesa_attrs());
}

#[test]
fn glue71_maps_the_core_arrays_to_the_same_records() {
    let attrs = glue71::attrs(&core_draw(), &[]).unwrap();
    assert_eq!(attrs.len(), 3);
    assert_eq!(attrs.iter().map(|a| a.divisor).collect::<Vec<_>>(), [0, 1, 1]);
    let (cs, vs) = reads();
    let plan = attr71::plan(&attrs, &cs, &vs, false, 0x1_0000).unwrap();
    let got: Vec<MesaAttr> = plan.records[..plan.count].iter().map(as_mesa).collect();
    assert_eq!(got, mesa_attrs());
    /* GL's divisor 0 on a per-instance array (the core's "0 means 1") */
    let mut d = core_draw();
    d.attribs[2].divisor = 0;
    assert_eq!(glue71::attrs(&d, &[]).unwrap()[2].divisor, 1);
}

#[test]
fn a_divisor_past_16_bits_is_refused_by_name() {
    let mut a = attr71::Attr { enabled: true, components: 4, stride: 24, buffer_addr: IBO, ..attr71::Attr::disabled() };
    a.divisor = attr71::MAX_DIVISOR;
    let (cs, vs) = reads();
    let r = attr71::plan(&[a, a, a], &cs, &vs, false, 0).unwrap();
    assert_eq!(r.records[1].divisor as u32, attr71::MAX_DIVISOR);
    a.divisor = attr71::MAX_DIVISOR + 1;
    assert_eq!(attr71::plan(&[a, a, a], &cs, &vs, false, 0), Err(attr71::Refusal71::Divisor(0x1_0000)));
}

/* ---- the instance id ----------------------------------------------------------------- */

#[test]
fn the_shader_record_reads_no_instance_id_and_the_tiles_start_at_instance_0() {
    let b = blocks();
    let main = only(&b, "shadrec_gl_main");
    for f in [
        "Instance ID read by coordinate shader",
        "Base Instance ID read by coordinate shader",
        "Instance ID read by vertex shader",
        "Base Instance ID read by vertex shader",
    ] {
        assert!(!main.flag(f), "{f}: INST_VS reads neither gl_InstanceID nor a base instance");
        /* record71 writes these four from the programs' uses_iid / uses_biid */
        assert!(record71::FIELDS.iter().any(|x| x.name == f), "record71 has {f:?}");
    }
    assert_eq!(only(&b, "SET_INSTANCEID").num("Instance ID"), 0);
}

/* ---- the 7.1 table draws it ---------------------------------------------------------- */

/// `yakogl_backend_v3d71` (src/v3d/table71.rs) driven through the GL core's
/// backend entries with the shape's draw, gl_test's `color_uv` program
/// (tools/v3d/gltest-shaders71/ours_color_uv.h, read as data) in place of
/// Mesa's: the OSAL is tests/v3d71_table_rig/mod.rs's mock, the C builder
/// the real driver/src/v3d/v3d_job71.c, so the binning list and records read
/// here are the bytes the Pi 5 image would submit (tests/v3d71_table_draw.rs).
/// It needs `gpu-v3d71`.
#[cfg(feature = "gpu-v3d71")]
mod table {
    use super::*;
    use std::collections::BTreeMap;

    use super::v3d71_table_rig::{self, gpu_block, gpu_write, kicks, log_lines, rig, Kick};
    use yakogl::shaderc::{ShcShader, ShcV3dUniform, ShcV3dVs};
    use yakogl::v3d::table71;

    fn num(s: &str) -> u64 {
        let s = s.trim().trim_end_matches(['u', 'U', 'l', 'L']);
        match s.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).unwrap_or_else(|e| panic!("{s}: {e}")),
            None => s.parse().unwrap_or_else(|e| panic!("{s}: {e}")),
        }
    }

    /// The programs' headers, compiled in (the files are part of the binary).
    const OURS_COLOR_UV: &str = include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h");

    /// A program header as numbers: `#define`s, `{ ... }` defines and arrays.
    struct CHeader {
        defs: BTreeMap<String, u64>,
        lists: BTreeMap<String, Vec<u64>>,
    }

    fn tokens(t: &str) -> Vec<u64> {
        t.split([',', '{', '}', ' ', '\t']).filter(|s| !s.is_empty()).map(num).collect()
    }

    fn c_header(text: &str) -> CHeader {
        let mut h = CHeader { defs: BTreeMap::new(), lists: BTreeMap::new() };
        let mut cur: Option<(String, Vec<u64>)> = None;
        for line in text.lines() {
            let t = line.trim();
            if let Some((name, mut v)) = cur.take() {
                if t.starts_with("};") {
                    h.lists.insert(name, v);
                    continue;
                }
                v.extend(tokens(t));
                cur = Some((name, v));
                continue;
            }
            if let Some(d) = t.strip_prefix("#define ") {
                let Some((n, v)) = d.split_once(' ') else { continue };
                let v = v.split("/*").next().unwrap().trim();
                if v.starts_with('{') {
                    h.lists.insert(n.to_string(), tokens(v));
                } else if v.starts_with(|c: char| c.is_ascii_digit()) {
                    h.defs.insert(n.to_string(), num(v));
                }
            } else if t.starts_with("static const") && t.ends_with('{') {
                let name = t.split('[').next().unwrap().split_whitespace().last().unwrap();
                cur = Some((name.to_string(), vec![]));
            }
        }
        h
    }

    impl CHeader {
        fn d(&self, n: &str) -> u32 {
            *self.defs.get(n).unwrap_or_else(|| panic!("{n}")) as u32
        }
        fn b(&self, n: &str) -> bool {
            self.d(n) != 0
        }
    }

    #[derive(Clone, Debug)]
    struct FieldDef {
        name: String,
        start: u32,
        size: u32,
        ty: String,
        minus_one: bool,
        default: Option<u64>,
    }

    #[derive(Clone, Debug)]
    struct PacketDef {
        code: u8,
        name: String,
        fields: Vec<FieldDef>,
        len: usize,
    }

    fn attr(tag: &str, k: &str) -> Option<String> {
        let pat = format!(" {k}=\"");
        let i = tag.find(&pat)? + pat.len();
        let j = tag[i..].find('"')? + i;
        Some(tag[i..j].to_string())
    }

    fn ver_ok(tag: &str) -> bool {
        let min = attr(tag, "min_ver").map(|v| v.parse::<u32>().unwrap()).unwrap_or(0);
        let max = attr(tag, "max_ver").map(|v| v.parse::<u32>().unwrap()).unwrap_or(999);
        min <= 71 && 71 <= max
    }

    /// The CLIF name of a packet: its shortname or name, upper case, every run
    /// of other characters one `_` (Mesa's clif dump, v3d_decoder.c).
    fn clif_name(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_uppercase());
            } else if !out.ends_with('_') {
                out.push('_');
            }
        }
        out.trim_matches('_').to_string()
    }

    fn packet_defs() -> Vec<PacketDef> {
        let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
        let mut defs = vec![];
        let mut cur: Option<PacketDef> = None;
        for raw in xml.lines() {
            let line = raw.trim();
            if line.starts_with("<packet ") {
                let tag = line;
                if ver_ok(tag) {
                    let name = attr(tag, "shortname").or_else(|| attr(tag, "name")).unwrap();
                    let code = attr(tag, "code").unwrap().parse().unwrap();
                    let p = PacketDef { code, name: clif_name(&name), fields: vec![], len: 1 };
                    if tag.ends_with("/>") {
                        defs.push(p);
                    } else {
                        cur = Some(p);
                    }
                }
                continue;
            }
            if line.starts_with("</packet>") {
                if let Some(mut p) = cur.take() {
                    let end = p.fields.iter().map(|f| f.start + f.size).max().unwrap_or(0);
                    p.len = 1 + end.div_ceil(8) as usize;
                    defs.push(p);
                }
                continue;
            }
            if let (Some(p), true) = (cur.as_mut(), line.starts_with("<field ")) {
                if !ver_ok(line) {
                    continue;
                }
                let start = attr(line, "start").unwrap();
                let start = match start.strip_suffix('b') {
                    Some(b) => b.parse::<u32>().unwrap() * 8,
                    None => start.parse().unwrap(),
                };
                p.fields.push(FieldDef {
                    name: attr(line, "name").unwrap(),
                    start,
                    size: attr(line, "size").unwrap().parse().unwrap(),
                    ty: attr(line, "type").unwrap(),
                    minus_one: attr(line, "minus_one").as_deref() == Some("true"),
                    default: attr(line, "default").map(|v| v.parse().unwrap()),
                });
            }
        }
        defs
    }

    fn bits(body: &[u8], start: u32, size: u32) -> u64 {
        let mut v = 0u64;
        for i in 0..size {
            let b = start + i;
            if body[(b / 8) as usize] >> (b % 8) & 1 != 0 {
                v |= 1 << i;
            }
        }
        v
    }

    /// The packets of a list with their bodies (the bytes after the code).
    fn walk<'d, 'b>(defs: &'d [PacketDef], bytes: &'b [u8], what: &str) -> Vec<(&'d PacketDef, &'b [u8])> {
        let mut out = vec![];
        let mut i = 0;
        while i < bytes.len() {
            let code = bytes[i];
            let cands: Vec<&PacketDef> = defs.iter().filter(|d| d.code == code).collect();
            let def = if cands.len() == 1 {
                cands[0]
            } else {
                let body = &bytes[i + 1..];
                *cands
                    .iter()
                    .find(|d| {
                        d.fields
                            .iter()
                            .filter(|f| f.default.is_some())
                            .all(|f| bits(body, f.start, f.size) == f.default.unwrap())
                    })
                    .unwrap_or_else(|| panic!("{what}: byte {i}: no 7.1 packet of code {code} matches"))
            };
            assert!(i + def.len <= bytes.len(), "{what}: {} at byte {i} runs past the list", def.name);
            out.push((def, &bytes[i + 1..i + def.len]));
            i += def.len;
        }
        out
    }

    /// gl_test's `color_uv` program as the online compiler's link hands it to
    /// the GL core: a fragment `ShcShader` whose `v3d` half describes the three
    /// programs (tools/v3d/gltest-shaders71/ours_color_uv.h, our 7.1 compiler's
    /// pinned output, read as data). The code is one upload: the fragment
    /// program at 0, the vertex program at 0x100, the coordinate program at
    /// 0x200 (the offsets tests/v3d71_backend71_shapes.rs places them at).
    fn color_uv() -> &'static ShcShader {
        static P: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let p = *P.get_or_init(|| {
            let h = c_header(OURS_COLOR_UV);
            let up = "OURS71_COLOR_UV";
            let list = |s: &str| -> (*const ShcV3dUniform, u32) {
                let n = h.d(&format!("{up}_{}_NUM_UNIFORMS", s.to_ascii_uppercase()));
                let words: Vec<ShcV3dUniform> = match h.lists.get(&format!("ours71_color_uv_{s}_uniforms")) {
                    Some(v) => {
                        v.chunks(2).map(|c| ShcV3dUniform { contents: c[0] as u32, data: c[1] as u32 }).collect()
                    }
                    None => vec![],
                };
                assert_eq!(words.len(), n as usize, "{s}: the uniform list");
                (Box::leak(words.into_boxed_slice()).as_ptr(), n)
            };
            let vs = |s: &str, off: u32| -> ShcV3dVs {
                let v = &h.lists[&format!("{up}_{s}_VATTR_SIZES")];
                let (uniforms, num_uniforms) = list(&s.to_ascii_lowercase());
                ShcV3dVs {
                    code_off: off,
                    code_bytes: h.d(&format!("{up}_{s}_NUM_WORDS")) * 8,
                    uniforms,
                    num_uniforms,
                    vpm_input_size: h.d(&format!("{up}_{s}_VPM_INPUT_SIZE")),
                    vpm_output_size: h.d(&format!("{up}_{s}_VPM_OUTPUT_SIZE")),
                    vcm_cache_size: h.d(&format!("{up}_{s}_VCM_CACHE_SIZE")),
                    threads: h.d(&format!("{up}_{s}_THREADS")) as u8,
                    single_seg: h.b(&format!("{up}_{s}_SINGLE_SEG")) as u8,
                    separate_segments: h.b(&format!("{up}_{s}_SEPARATE_SEGMENTS")) as u8,
                    uses_vid: h.b(&format!("{up}_{s}_USES_VID")) as u8,
                    uses_iid: h.b(&format!("{up}_{s}_USES_IID")) as u8,
                    uses_biid: h.b(&format!("{up}_{s}_USES_BIID")) as u8,
                    writes_psiz: h.b(&format!("{up}_{s}_WRITES_PSIZ")) as u8,
                    vattr_sizes: core::array::from_fn(|i| v[i] as u8),
                }
            };
            let mut sh = ShcShader::ZERO;
            sh.usc_dw = 0x300 / 4;
            sh.uses_discard = h.b(&format!("{up}_FS_USES_DISCARD")) as u8;
            let v = &mut sh.v3d;
            v.linked = 1;
            v.num_records = h.d(&format!("{up}_NUM_ATTRS")) as u8;
            for (loc, name) in ["A_POS", "A_COLOR", "A_UV"].iter().enumerate() {
                v.record_of_location[loc] = h.d(&format!("{up}_ATTR_{name}_RECORD")) as u8;
            }
            let (uniforms, num_uniforms) = list("fs");
            let f = &mut v.fs;
            (f.code_off, f.code_bytes) = (0, h.d(&format!("{up}_FS_NUM_WORDS")) * 8);
            (f.uniforms, f.num_uniforms) = (uniforms, num_uniforms);
            f.num_inputs = h.d(&format!("{up}_FS_NUM_INPUTS"));
            f.flat_shade_flags = h.d(&format!("{up}_FS_FLAT_SHADE_FLAGS"));
            f.noperspective_flags = h.d(&format!("{up}_FS_NOPERSPECTIVE_FLAGS"));
            f.centroid_flags = h.d(&format!("{up}_FS_CENTROID_FLAGS"));
            f.threads = h.d(&format!("{up}_FS_THREADS")) as u8;
            f.single_seg = h.b(&format!("{up}_FS_SINGLE_SEG")) as u8;
            f.writes_z = h.b(&format!("{up}_FS_WRITES_Z")) as u8;
            f.disable_ez = h.b(&format!("{up}_FS_DISABLE_EZ")) as u8;
            f.uses_center_w = h.b(&format!("{up}_FS_USES_CENTER_W")) as u8;
            f.lock_scoreboard_on_first_thrsw = h.b(&format!("{up}_FS_LOCK_SCOREBOARD_ON_FIRST_THRSW")) as u8;
            f.uses_implicit_point_line_varyings = h.b(&format!("{up}_FS_USES_IMPLICIT_POINT_LINE_VARYINGS")) as u8;
            f.uses_pid = h.b(&format!("{up}_FS_USES_PID")) as u8;
            f.rt_mask = 1;
            v.vs = vs("VS", 0x100);
            v.cs = vs("CS", 0x200);
            Box::leak(Box::new(sh)) as *const ShcShader as usize
        });
        // SAFETY: leaked above, never freed or written again.
        unsafe { &*(p as *const ShcShader) }
    }

    const G_SIZE: u32 = 64;
    const DS_ID: u32 = 200;

    /// The GL core's side: a 64x64 RGBA8 window with a D24S8, its surface, the
    /// program's code, the per-vertex positions (16-byte stride) and the
    /// per-instance colour and uv (24-byte stride), as the shape's two buffers.
    struct Core {
        surface: u32,
        code: u32,
        vbo: u32,
        ibo: u32,
    }

    impl Core {
        fn new() -> Core {
            let vbo = gpu_block(COUNT as usize * 16);
            gpu_write(vbo, &(0..COUNT as usize * 16).map(|i| (i * 7 + 3) as u8).collect::<Vec<u8>>());
            let ibo = gpu_block(INSTANCES as usize * 24);
            gpu_write(ibo, &(0..INSTANCES as usize * 24).map(|i| (i * 5 + 1) as u8).collect::<Vec<u8>>());
            Core { surface: gpu_block((G_SIZE * G_SIZE * 4) as usize), code: gpu_block(0x300), vbo, ibo }
        }

        /// tests/v3d71_table_draw.rs `Core::base`.
        fn base(&self) -> GlesDraw {
            let mut d = GlesDraw::ZERO;
            d.target_gpu = u64::from(self.surface);
            (d.target_w, d.target_h) = (G_SIZE, G_SIZE);
            d.target_format = GLES_SURF_RGBA8888;
            d.target_alpha_bits = 8;
            d.targets[0] =
                GlesDrawTarget { gpu: u64::from(self.surface), w: G_SIZE, h: G_SIZE, format: 0, out_shift: 0 };
            d.num_targets = 1;
            d.samples = 1;
            d.viewport = [0, 0, G_SIZE as i32, G_SIZE as i32];
            d.depth_range = [0.0, 1.0];
            d.ds_persist = true;
            (d.ds_id, d.depth_id, d.stencil_id) = (DS_ID, DS_ID, DS_ID);
            (d.ds_w, d.ds_h) = (G_SIZE, G_SIZE);
            d.prim = GLES_PRIM_TRIANGLES;
            (d.color_write, d.color_mask) = (true, 0xf);
            (d.stencil_write_mask, d.stencil_back_write_mask) = (0xff, 0xff);
            d.sample_mask = 0xf;
            d
        }

        fn clear(&self) -> GlesDraw {
            let mut d = self.base();
            (d.do_clear, d.clear_rgba, d.clear_color_mask) = (true, [0.0, 0.0, 1.0, 1.0], 0xf);
            d
        }

        /// glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 3): `a_pos` per vertex,
        /// `a_color` and `a_uv` at `divisor` from the second buffer (the
        /// shape's `a_color` and `a_off`).
        fn instanced(&self, divisor: u16) -> GlesDraw {
            let mut d = self.base();
            d.index_count = COUNT as u32;
            d.instance_count = INSTANCES as u32;
            d.line_width = 1.0;
            d.fs = color_uv();
            d.fs_usc_gpu = u64::from(self.code);
            let arr = |vi_reg: u8, components: u8, stride: u32, offset: u32, buf: u32, per_instance: bool| GlesAttrib {
                vi_reg,
                components,
                format: GLES_ATTRIBF_FLOAT32,
                stride,
                offset,
                buffer_gpu: u64::from(buf),
                per_instance: per_instance as u8,
                divisor: if per_instance { divisor } else { 0 },
                ..GlesAttrib::ZERO
            };
            d.attribs[0] = arr(0, 4, 16, 0, self.vbo, false);
            d.attribs[1] = arr(1, 4, 24, 0, self.ibo, true);
            d.attribs[2] = arr(2, 2, 24, 16, self.ibo, true);
            d.num_attribs = 3;
            (d.depth_func, d.depth_write) = (GLES_CMP_LESS, true);
            (d.stencil_func, d.stencil_cmp_mask) = (GLES_CMP_ALWAYS, 0xff);
            (d.stencil_back_func, d.stencil_back_cmp_mask) = (GLES_CMP_ALWAYS, 0xff);
            for op in [
                &mut d.stencil_sfail,
                &mut d.stencil_dpfail,
                &mut d.stencil_dppass,
                &mut d.stencil_back_sfail,
                &mut d.stencil_back_dpfail,
                &mut d.stencil_back_dppass,
            ] {
                *op = GLES_SOP_KEEP;
            }
            d.cull = GLES_CULL_NONE;
            d
        }
    }

    /// The job of a clear and `d`, and the draws refused on the way.
    fn one_job(c: &Core, d: &GlesDraw) -> (Kick, u32) {
        let refused = table71::refused_draws();
        assert_eq!(backend::draw(&c.clear()), 0, "{:?}", log_lines());
        assert_eq!(backend::draw(d), 0, "{:?}", log_lines());
        assert_eq!(backend::finish(), 0, "{:?}", log_lines());
        let ks = kicks();
        assert_eq!(ks.len(), 1, "one job: {:?}", log_lines());
        (ks[0].clone(), table71::refused_draws() - refused)
    }

    /// The job's draw packets (`(name, bytes)`, the code included) and its
    /// attribute records as [`MesaAttr`]s, the address relative to `bases`
    /// (the record's address less the buffer it falls in).
    fn ours(k: &Kick, bases: [u32; 2]) -> (Vec<(String, Vec<u8>)>, Vec<MesaAttr>) {
        let defs = packet_defs();
        let (mut draws, mut attrs) = (vec![], vec![]);
        for (def, body) in walk(&defs, k.bcl(), "bcl") {
            let name = def.name.as_str();
            if name == "GL_SHADER" {
                let field = |n: &str| {
                    let f = def.fields.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("GL_SHADER.{n}"));
                    bits(body, f.start, f.size)
                };
                /* the address is in 32-byte units; the attribute records
                 * follow the 32-byte record (tests/v3d71_table_draw.rs) */
                let at = (field("address") as u32) << 5;
                attrs.clear();
                for i in 0..field("number of attribute arrays") as usize {
                    let b = k.at(at + (record71::RECORD_SIZE + i * attr71::RECORD_BYTES) as u32, attr71::RECORD_BYTES);
                    let get = |n: &str| {
                        let (_, st, sz) = attr71::FIELDS.iter().find(|f| f.0 == n).unwrap_or_else(|| panic!("{n}"));
                        bits(b, *st, *sz) as u32
                    };
                    let address = get("Address");
                    let base = *bases.iter().filter(|&&b| b <= address).max().expect("a record in a buffer");
                    attrs.push(MesaAttr {
                        address: address - base,
                        vec: match get("Vec size") {
                            0 => 4,
                            v => v,
                        },
                        ty: get("Type"),
                        signed: get("Signed int type") != 0,
                        normalized: get("Normalized int type") != 0,
                        read_as_int: get("Read as int/uint") != 0,
                        cs: get("Number of values read by Coordinate shader"),
                        vs: get("Number of values read by Vertex shader"),
                        divisor: get("Instance Divisor"),
                        stride: get("Stride"),
                        max_index: get("Maximum Index"),
                    });
                }
            }
            if name.contains("PRIM") {
                draws.push((def.name.clone(), [&[def.code][..], body].concat()));
            }
        }
        (draws, attrs)
    }

    /// The fields our records share with Mesa's whatever the program: the
    /// shader's reads (`cs`, `vs`) are ours, not Mesa's.
    fn layout(a: &MesaAttr) -> (u32, u32, u32, bool, bool, bool, u32, u32, u32) {
        (a.address, a.vec, a.ty, a.signed, a.normalized, a.read_as_int, a.divisor, a.stride, a.max_index)
    }

    fn started() -> (v3d71_table_rig::Rig, Core) {
        let g = rig();
        assert_eq!(backend::init(), 0, "{:?}", log_lines());
        (g, Core::new())
    }

    #[test]
    fn the_table_draws_the_shape_as_mesas_one_instanced_packet() {
        let (_g, c) = started();
        let (k, refused) = one_job(&c, &c.instanced(1));
        assert_eq!(refused, 0, "refused_draws() did not grow: {:?}", log_lines());
        let (draws, attrs) = ours(&k, [c.vbo, c.ibo]);
        assert_eq!(draws.len(), 1, "one draw packet: {draws:?}");
        let (name, bytes) = &draws[0];
        assert_eq!(name, "VERTEX_ARRAY_INSTANCED_PRIMS", "not one draw per instance");
        let b = blocks();
        let mesa = only(&b, "VERTEX_ARRAY_INSTANCED_PRIMS");
        assert_eq!(xml_decode(PRIMS, bytes), clif_by_xml(mesa, PRIMS), "mode, length, instances, first vertex");
        /* the records: Mesa's, address within its buffer, divisors included */
        let m: Vec<_> = mesa_attrs()
            .iter()
            .map(|a| MesaAttr { address: a.address - if a.address >= IBO { IBO } else { VBO }, ..*a })
            .collect();
        assert_eq!(attrs.iter().map(layout).collect::<Vec<_>>(), m.iter().map(layout).collect::<Vec<_>>());
    }

    #[test]
    fn a_divisor_above_1_is_one_draw_too() {
        let (_g, c) = started();
        assert_eq!(backend::max_instance_divisor(), attr71::MAX_DIVISOR, "the GL core hands every divisor over");
        let (k, refused) = one_job(&c, &c.instanced(2));
        assert_eq!(refused, 0, "refused_draws() did not grow: {:?}", log_lines());
        let (draws, attrs) = ours(&k, [c.vbo, c.ibo]);
        assert_eq!(draws.iter().map(|d| d.0.as_str()).collect::<Vec<_>>(), ["VERTEX_ARRAY_INSTANCED_PRIMS"]);
        assert_eq!(attrs.iter().map(|a| a.divisor).collect::<Vec<_>>(), [0, 2, 2]);
    }

    #[test]
    fn one_instance_is_the_plain_packet() {
        let (_g, c) = started();
        let mut d = c.instanced(1);
        d.instance_count = 1;
        let (k, refused) = one_job(&c, &d);
        assert_eq!(refused, 0);
        let (draws, _) = ours(&k, [c.vbo, c.ibo]);
        assert_eq!(draws.iter().map(|d| d.0.as_str()).collect::<Vec<_>>(), ["VERTEX_ARRAY_PRIMS"]);
    }
}
