// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 table draws points with gl_PointSize (plan item psiz71):
//! `yakogl_backend_v3d71` (src/v3d/table71.rs) driven through the GL core's
//! backend entries with one `glDrawArrays(GL_POINTS, 0, 4)` whose vertex
//! program writes gl_PointSize, and the point-size state of the job it
//! submits compared with Mesa's for the same GL call: the first draw of
//! tools/v3d/pi5/gl_prims.clif (gl_shapes.py `prims`, `program(POINT_VS,
//! FS)`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table_points
//! ```
//!
//! The OSAL is the mock of tests/v3d71_table_rig/mod.rs, the C builder the
//! real driver/src/v3d/v3d_job71.c (as in tests/v3d71_table_draw.rs), so the
//! binning list and the record decoded here are the bytes the Pi 5 image
//! would submit.
//!
//! Compared with the state in force at Mesa's points draw:
//!
//! * the shader record's `Point size in shaded vertex data` (bit 0 of the
//!   record, the same bit in the plain 7.1 layout Mesa's dump decodes and
//!   in the D0 draw-index layout the table writes);
//! * `POINT_SIZE` (the fixed-function size, 1.0: ES has no glPointSize);
//! * `VERTEX_ARRAY_PRIMS`: mode (POINTS), Length and Index of First Vertex;
//! * `CFG_BITS`' primitive bits (forward and reverse facing enables,
//!   clockwise, line rasterization, the Direct3D point-fill, wireframe and
//!   provoking-vertex modes). At 7.1 CFG_BITS has no point-size bit: a
//!   points draw that writes gl_PointSize differs from one that does not
//!   only in the record.
//!
//! The program is gl_test's `color_uv` (tools/v3d/gltest-shaders71/
//! ours_color_uv.h, read as data) flagged as writing gl_PointSize, the
//! stand-in tests/v3d71_glue_prims.rs uses for gl_shapes.py's POINT_VS: its
//! VPM layout is not compared. Not compared either: blend, depth and
//! stencil state (the GL core's window here has a depth/stencil buffer,
//! gl_shapes.py's has none), the BO addresses and the program fields.
//! `gap_psiz_all_prims` (tests/v3d71_glue_prims.rs) is untouched: one
//! linked program flags gl_PointSize for every primitive it draws.
#![cfg(feature = "gpu-v3d71")]
#![allow(dead_code)]

mod v3d71_table_rig;

use std::collections::BTreeMap;

use v3d71_table_rig::{gpu_block, gpu_write, kicks, log_lines, rig, Kick};
use yakogl::backend::{
    self, GlesAttrib, GlesDraw, GlesDrawTarget, GLES_ATTRIBF_FLOAT32, GLES_CULL_NONE, GLES_PRIM_POINTS, GLES_SOP_KEEP,
    GLES_SURF_RGBA8888,
};
use yakogl::shaderc::{ShcShader, ShcV3dUniform, ShcV3dVs};
use yakogl::v3d::ffi71::V3D_PRIM_POINTS;
use yakogl::v3d::{record71, table71};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn num(s: &str) -> u64 {
    let s = s.trim().trim_end_matches(['u', 'U', 'l', 'L']);
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).unwrap_or_else(|e| panic!("{s}: {e}")),
        None => s.parse().unwrap_or_else(|e| panic!("{s}: {e}")),
    }
}

/* ---- our program (tools/v3d/gltest-shaders71/ours_color_uv.h, read as data) ---- */

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

/* ---- the packet table (tests/v3d71_table_draw.rs) ------------------------------------ */

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

/* ---- the GL core's side (tests/v3d71_table_draw.rs) ---------------------------------- */

/// gl_test's `color_uv` program as the online compiler's link hands it to
/// the GL core (tests/v3d71_table_indexed.rs `color_uv`), its vertex program
/// flagged as writing gl_PointSize when `psiz` (the `writes_psiz` the link
/// sets for a VS that writes it). The code is one upload: the fragment
/// program at 0, the vertex program at 0x100, the coordinate program at
/// 0x200.
fn color_uv(psiz: bool) -> &'static ShcShader {
    let h = c_header(OURS_COLOR_UV);
    let up = "OURS71_COLOR_UV";
    let list = |s: &str| -> (*const ShcV3dUniform, u32) {
        let n = h.d(&format!("{up}_{}_NUM_UNIFORMS", s.to_ascii_uppercase()));
        let words: Vec<ShcV3dUniform> = match h.lists.get(&format!("ours71_color_uv_{s}_uniforms")) {
            Some(v) => v.chunks(2).map(|c| ShcV3dUniform { contents: c[0] as u32, data: c[1] as u32 }).collect(),
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
    assert_eq!(v.vs.writes_psiz, 0, "ours_color_uv itself does not write gl_PointSize");
    v.vs.writes_psiz = psiz as u8;
    Box::leak(Box::new(sh))
}

/// gl_shapes.py's 40-byte vertex: pos4 +0, colour4 +16, uv2 +32.
const STRIDE: u32 = 40;
/// gl_shapes.py `prims`' points draw: `glDrawArrays(GL_POINTS, 0, 4)`.
const POINTS: u32 = 4;
const G_SIZE: u32 = 64;
const DS_ID: u32 = 200;

/// The GL core's side: a 64x64 RGBA8 window with a D24S8, its surface, the
/// program's code and the vertices in a buffer object.
struct Core {
    surface: u32,
    code: u32,
    vbo: u32,
}

impl Core {
    fn new() -> Core {
        let bytes = (POINTS * STRIDE) as usize;
        let vbo = gpu_block(bytes);
        gpu_write(vbo, &(0..bytes).map(|i| (i * 7 + 3) as u8).collect::<Vec<u8>>());
        Core { surface: gpu_block((G_SIZE * G_SIZE * 4) as usize), code: gpu_block(0x300), vbo }
    }

    /// tests/v3d71_table_draw.rs `Core::base`.
    fn base(&self) -> GlesDraw {
        let mut d = GlesDraw::ZERO;
        d.target_gpu = u64::from(self.surface);
        (d.target_w, d.target_h) = (G_SIZE, G_SIZE);
        d.target_format = GLES_SURF_RGBA8888;
        d.target_alpha_bits = 8;
        d.targets[0] = GlesDrawTarget { gpu: u64::from(self.surface), w: G_SIZE, h: G_SIZE, format: 0, out_shift: 0 };
        d.num_targets = 1;
        d.samples = 1;
        d.viewport = [0, 0, G_SIZE as i32, G_SIZE as i32];
        d.depth_range = [0.0, 1.0];
        d.ds_persist = true;
        (d.ds_id, d.depth_id, d.stencil_id) = (DS_ID, DS_ID, DS_ID);
        (d.ds_w, d.ds_h) = (G_SIZE, G_SIZE);
        d.prim = GLES_PRIM_POINTS;
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

    /// `glDrawArrays(GL_POINTS, 0, 4)` with `color_uv` (gl_PointSize written
    /// when `psiz`), every attribute from the buffer object, no depth test.
    fn points(&self, psiz: bool) -> GlesDraw {
        let mut d = self.base();
        d.index_count = POINTS;
        d.instance_count = 1;
        d.line_width = 1.0;
        d.fs = color_uv(psiz);
        d.fs_usc_gpu = u64::from(self.code);
        for (k, (comps, off)) in [(4u8, 0u32), (4, 16), (2, 32)].into_iter().enumerate() {
            d.attribs[k] = GlesAttrib {
                vi_reg: k as u8,
                components: comps,
                format: GLES_ATTRIBF_FLOAT32,
                stride: STRIDE,
                offset: off,
                buffer_gpu: u64::from(self.vbo),
                ..GlesAttrib::ZERO
            };
        }
        d.num_attribs = 3;
        (d.depth_func, d.depth_write) = (backend::GLES_CMP_ALWAYS, false);
        (d.stencil_func, d.stencil_cmp_mask) = (backend::GLES_CMP_ALWAYS, 0xff);
        (d.stencil_back_func, d.stencil_back_cmp_mask) = (backend::GLES_CMP_ALWAYS, 0xff);
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

/* ---- the point-size state ------------------------------------------------------------ */

/// `CFG_BITS`' primitive fields, the ones a points draw depends on.
const CFG_PRIM: [&str; 7] = [
    "Enable Forward Facing Primitive",
    "Enable Reverse Facing Primitive",
    "Clockwise Primitives",
    "Line Rasterization",
    "Direct3D Wireframe triangles mode",
    "Direct3D 'Point-fill' mode",
    "Direct3D Provoking Vertex",
];

/// One draw's point-size state, as the dump prints the values.
#[derive(Clone, Debug, PartialEq)]
struct Psiz {
    /// The record's `Point size in shaded vertex data`.
    in_vdata: u64,
    /// `POINT_SIZE` in force.
    point_size: f32,
    /// `VERTEX_ARRAY_PRIMS`: mode, Length, Index of First Vertex.
    prim: (u64, u64, u64),
    /// [`CFG_PRIM`] of the `CFG_BITS` in force.
    cfg: Vec<u64>,
}

/// A field of a packet body, raw (`minus_one` undone).
fn field(def: &PacketDef, body: &[u8], name: &str) -> u64 {
    let f = def.fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{}.{name}", def.name));
    bits(body, f.start, f.size) + f.minus_one as u64
}

/// Our job's array draws, and how many draws of any kind its binning list
/// holds.
fn ours(k: &Kick) -> (Vec<Psiz>, usize) {
    let defs = packet_defs();
    let psiz_bit = record71::FIELDS.iter().find(|f| f.name == "Point size in shaded vertex data").unwrap();
    assert_eq!((psiz_bit.start, psiz_bit.size), (0, 1));
    let (mut point_size, mut cfg, mut record) = (None, None, None);
    let (mut out, mut draws) = (vec![], 0);
    for (def, body) in walk(&defs, k.bcl(), "bcl") {
        match def.name.as_str() {
            "POINT_SIZE" => point_size = Some(f32::from_bits(field(def, body, "Point Size") as u32)),
            "CFG_BITS" => cfg = Some(CFG_PRIM.iter().map(|n| field(def, body, n)).collect::<Vec<_>>()),
            "GL_SHADER" => {
                /* the packet's address is in 32-byte units */
                let rec = k.at((field(def, body, "address") as u32) << 5, record71::RECORD_SIZE);
                record = Some(bits(rec, psiz_bit.start, psiz_bit.size));
            }
            "VERTEX_ARRAY_PRIMS" => {
                out.push(Psiz {
                    in_vdata: record.expect("GL_SHADER before the draw"),
                    point_size: point_size.expect("POINT_SIZE before the draw"),
                    prim: (
                        field(def, body, "mode"),
                        field(def, body, "Length"),
                        field(def, body, "Index of First Vertex"),
                    ),
                    cfg: cfg.clone().expect("CFG_BITS before the draw"),
                });
                draws += 1;
            }
            "VERTEX_ARRAY_INSTANCED_PRIMS"
            | "VERTEX_ARRAY_SINGLE_INSTANCE_PRIMS"
            | "INDEXED_PRIM_LIST"
            | "INDEXED_INSTANCED_PRIM_LIST" => draws += 1,
            _ => {}
        }
    }
    (out, draws)
}

/// Mesa's array draws of tools/v3d/pi5/gl_prims.clif with the state in
/// force at each; the records (`@format shadrec_gl_main`) in the order the
/// draws name them (the dump lays them out at rising addresses).
fn mesa() -> Vec<Psiz> {
    let text = read("tools/v3d/pi5/gl_prims.clif");
    let mut packets: Vec<(String, BTreeMap<String, String>)> = vec![];
    for line in text.lines() {
        let s = line.strip_prefix("MESA: info: ").unwrap_or(line).trim();
        if s.starts_with("@format shadrec_gl_main") {
            packets.push(("SHADREC".to_string(), BTreeMap::new()));
        } else if s.starts_with('@')
            || (!s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        {
            /* a packet, or another @format: it ends the record before it */
            packets.push((s.to_string(), BTreeMap::new()));
        } else if let (Some(rest), Some(p)) = (s.strip_prefix("/*"), packets.last_mut()) {
            if let Some((label, value)) = rest.split_once(": */") {
                p.1.insert(label.trim().to_string(), value.split_whitespace().next().unwrap_or("").to_string());
            }
        }
    }
    let get = |m: &BTreeMap<String, String>, k: &str| -> String {
        m.get(k).unwrap_or_else(|| panic!("gl_prims: {k}")).clone()
    };
    let n = |m: &BTreeMap<String, String>, k: &str| num(&get(m, k));
    let records: Vec<u64> =
        packets.iter().filter(|p| p.0 == "SHADREC").map(|p| n(&p.1, "Point size in shaded vertex data")).collect();
    let (mut point_size, mut cfg, mut out) = (None, None, vec![]);
    for (name, f) in &packets {
        match name.as_str() {
            "POINT_SIZE" => point_size = Some(get(f, "Point Size").parse::<f32>().unwrap()),
            "CFG_BITS" => cfg = Some(CFG_PRIM.iter().map(|k| n(f, k)).collect::<Vec<_>>()),
            "VERTEX_ARRAY_PRIMS" => out.push(Psiz {
                in_vdata: records[out.len()],
                point_size: point_size.expect("gl_prims: POINT_SIZE first"),
                prim: (n(f, "mode"), n(f, "Length"), n(f, "Index of First Vertex")),
                cfg: cfg.clone().expect("gl_prims: CFG_BITS first"),
            }),
            _ => {}
        }
    }
    assert_eq!(records.len(), out.len(), "gl_prims: one record a draw");
    out
}

/// Mesa's points draw (gl_shapes.py `prims`' first draw).
fn mesa_points() -> Psiz {
    let m = mesa();
    assert_eq!(m.len(), 3, "gl_prims: points, lines, a strip");
    m[0].clone()
}

fn started() -> (v3d71_table_rig::Rig, Core) {
    let g = rig();
    assert_eq!(backend::init(), 0, "{:?}", log_lines());
    (g, Core::new())
}

/// A clear and `d` in one flush: the one kick, and the draws refused on the way.
fn one_job(c: &Core, d: &GlesDraw) -> (Kick, u32) {
    let refused = table71::refused_draws();
    assert_eq!(backend::draw(&c.clear()), 0, "{:?}", log_lines());
    assert_eq!(backend::draw(d), 0, "{:?}", log_lines());
    assert_eq!(backend::finish(), 0, "{:?}", log_lines());
    let ks = kicks();
    assert_eq!(ks.len(), 1, "one job: {:?}", log_lines());
    (ks[0].clone(), table71::refused_draws() - refused)
}

/// Our job's one points draw (the clear is a tile-buffer clear, no draw).
fn the_points_draw(k: &Kick) -> Psiz {
    let (ds, draws) = ours(k);
    assert_eq!((ds.len(), draws), (1, 1), "the one draw, from arrays");
    ds[0].clone()
}

/* ---- the tests -------------------------------------------------------------------------- */

#[test]
fn the_clif_has_the_sized_points_draw() {
    let m = mesa();
    let p = &m[0];
    assert_eq!(p.in_vdata, 1, "POINT_VS writes gl_PointSize");
    assert_eq!(p.prim, (u64::from(V3D_PRIM_POINTS), u64::from(POINTS), 0));
    assert_eq!(p.point_size, 1.0);
    /* the lines and the strip: Mesa's variant without gl_PointSize */
    assert_eq!(m.iter().map(|d| d.in_vdata).collect::<Vec<_>>(), [1, 0, 0]);
    /* CFG_BITS is the same for all three: no point-size bit at 7.1 */
    assert!(m.iter().all(|d| d.cfg == p.cfg));
}

#[test]
fn a_points_draw_writing_gl_pointsize_is_mesas() {
    let (_g, c) = started();
    let (k, refused) = one_job(&c, &c.points(true));
    assert_eq!(refused, 0, "refused_draws() did not grow: {:?}", log_lines());
    assert_eq!(the_points_draw(&k), mesa_points());
}

#[test]
fn points_without_gl_pointsize_leave_the_record_bit_clear() {
    let (_g, c) = started();
    let (k, refused) = one_job(&c, &c.points(false));
    assert_eq!(refused, 0, "refused_draws() did not grow: {:?}", log_lines());
    let o = the_points_draw(&k);
    assert_eq!(o.in_vdata, 0);
    /* everything else is the sized draw's: the record bit is the one field */
    assert_eq!(Psiz { in_vdata: 1, ..o }, mesa_points());
}
