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
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_instanced
//! ```
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
//! What the 7.1 path cannot carry yet is an `#[ignore]`d test naming it.
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

/* ---- what the 7.1 path does not carry yet ------------------------------------------- */

/// backend71 (the 7.1 table's `Draw71` builder) refuses every draw of more
/// than one instance (`REF_INSTANCED`), although prim71, attr71, glue71,
/// fill71 (`FilledDraw::instances`) and v3d_job71.c (the instanced packets)
/// carry it. backend71 reaches the C builder through FFI, so this reads its
/// text instead of linking it; it passes once the refusal is gone.
#[test]
#[ignore = "backend71::draw_of refuses instanced draws (REF_INSTANCED): Draw71 assembles one instance per draw"]
fn backend71_assembles_an_instanced_draw() {
    let src = include_str!("../src/v3d/backend71.rs");
    assert!(
        !src.contains("if fd.instances > 1 {"),
        "backend71::draw_of still refuses gl_instanced's draw (3 instances)"
    );
}
