// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/glue71.rs` on refs71b's primitive references: the GlesDraws the
//! GL core hands the back end for tools/v3d/pi5/src/gl_shapes.py's
//! `indexed` and `prims` shapes, turned into fill71's operations by glue71
//! and filled by `fill71::fill`, against Mesa's 7.1 dumps
//! tools/v3d/pi5/gl_indexed.clif and tools/v3d/pi5/gl_prims.clif
//! (tools/v3d/pi5/README.md, "The draw shapes").
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_glue_prims
//! ```
//!
//! Compared per draw, as the packets driver/src/v3d/v3d_job71.c writes
//! from fill71's `FilledDraw` (`emit_state`, `emit_draw`), against the
//! packet state in force at the same draw of Mesa's binning list:
//!
//! * the primitive packet (`INDEXED_PRIM_LIST` or `VERTEX_ARRAY_PRIMS`):
//!   mode, index type, length, `Enable Primitive Restarts`, and where the
//!   draw starts (the first vertex's byte in the array buffer, or the first
//!   index's byte in the index buffer: Mesa gives the buffer and an offset
//!   or a first vertex, the GL core folds both into the address, glue71's
//!   documented choice);
//! * `CFG_BITS`' primitive bits (forward and reverse facing enables,
//!   clockwise, line rasterization);
//! * `POINT_SIZE`, `LINE_WIDTH` and the shader record's `Point size in
//!   shaded vertex data`.
//!
//! Not compared: blend, depth and stencil state (tests/v3d71_compose5.rs),
//! the BO addresses, `INDEX_BUFFER_SETUP`'s size (Mesa gives the whole
//! 4096-byte BO, v3d_job71 the bytes the draw reads; checked to cover the
//! draw's indices), and the programs (ours_color_uv stands in for
//! gl_shapes.py's FS and POINT_VS; the points program is ours_color_uv
//! flagged as writing gl_PointSize).
//!
//! The GL core's GlesDraws are written here as src/gl/draw.rs builds them
//! (`draw_elements`, `restart_to_lists`, `draw_arrays`), each field named
//! at its use. Where the result differs from Mesa, the difference is a
//! named gap: a passing test pins what we write today, and an `#[ignore]`d
//! test named after the gap asserts Mesa's value (it passes once the gap is
//! closed):
//!
//! * `gap_index_widen`: the GL core widens u8 and u32 indices to u16
//!   (draw.rs `draw_elements`: `d.index_size = 2`), so the 8-bit and 32-bit
//!   draws are 16-bit `INDEXED_PRIM_LIST`s where Mesa writes types 0 and 2.
//!   glue71 itself maps an index size of 1 or 4 to Mesa's types
//!   (`glue71_maps_every_index_size`).
//! * `gap_restart_as_lists`: with `GL_PRIMITIVE_RESTART_FIXED_INDEX` the GL
//!   core rewrites the strip into a triangle list without the restart index
//!   (draw.rs `restart_to_lists`): a `TRIANGLES` list of 6 where Mesa writes
//!   a `TRIANGLE_STRIP` of 7 with `Enable Primitive Restarts` 1. A GlesDraw
//!   has no restart field, glue71 writes `restart: false`, and fill71
//!   refuses a restart (`fill71_refuses_restart`).
//! * `gap_psiz_all_prims`: one linked program serves every primitive, so a
//!   program writing gl_PointSize sets `Point size in shaded vertex data`
//!   on the lines and the strip too, where Mesa compiles a variant without
//!   it for non-point primitives. Consistent with our shader's VPM layout.
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
use ffi71::*;
use fill71::{ClearQuad, FilledDraw, History, Op, Program};
use std::collections::BTreeMap;
use std::sync::OnceLock;

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

/* ---- where our buffers sit (tests/v3d71_glue.rs's layout) -------------------- */

const G_SIZE: u32 = 64;
const G_CODE: u32 = 0x200000;
const G_VTX: u32 = 0x300000;
const G_TILE: u32 = 0x400000;
const G_RT: u32 = 0x1000000;
/// The application's element buffer object (`ib.mem.gpu`).
const G_IBO: u32 = 0x500000;
/// Where the table copies a draw's staged indices (`index_cpu`).
const G_STAGED: u32 = 0x600000;
/// gl_test's 40-byte vertex: pos4 +0, colour4 +16, uv2 +32 (gl_shapes.py `upload`).
const STRIDE: u32 = 40;

/* ---- our programs (tools/v3d/gltest-shaders71/ours_*.h, read as data) ------- */

const OURS_COLOR_UV: &str = include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h");
const OURS_CLEAR: &str = include_str!("../tools/v3d/gltest-shaders71/ours_clear.h");

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

/// One of our programs as fill71 takes it (tests/v3d71_glue.rs `our_program`).
fn our_program(text: &str, p: &str, code: u32) -> Program<'static> {
    let h = c_header(text);
    let lc = p.to_ascii_lowercase();
    let up = p.to_ascii_uppercase();
    let list = |s: &str| -> &'static [uniform71::Word] {
        let n = h.d(&format!("{up}_{}_NUM_UNIFORMS", s.to_ascii_uppercase()));
        let words: Vec<uniform71::Word> = match h.lists.get(&format!("{lc}_{s}_uniforms")) {
            Some(v) => v.chunks(2).map(|c| uniform71::Word { contents: c[0] as u32, data: c[1] as u32 }).collect(),
            None => vec![],
        };
        assert_eq!(words.len(), n as usize, "{p} {s}: the uniform list");
        Box::leak(words.into_boxed_slice())
    };
    let reads = |s: &str| -> [u8; 16] {
        let v = &h.lists[&format!("{up}_{s}_VATTR_SIZES")];
        core::array::from_fn(|i| v[i] as u8)
    };
    let threads = |s: &str| h.d(&format!("{up}_{s}_THREADS"));
    let vdesc = |s: &str, at: u32| record71::VertexDesc {
        prog: record71::ProgramDesc {
            code: at,
            uniforms: 0,
            threads: threads(s),
            single_seg: h.b(&format!("{up}_{s}_SINGLE_SEG")),
            propagate_nans: false,
        },
        vpm_input_size: h.d(&format!("{up}_{s}_VPM_INPUT_SIZE")),
        vpm_output_size: h.d(&format!("{up}_{s}_VPM_OUTPUT_SIZE")),
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: h.b(&format!("{up}_{s}_USES_VID")),
        uses_iid: h.b(&format!("{up}_{s}_USES_IID")),
        uses_base_instance: h.b(&format!("{up}_{s}_USES_BIID")),
        uses_base_vertex: false,
        uses_draw_index: false,
    };
    let mut desc = record71::DrawDesc::default();
    desc.fs.prog = record71::ProgramDesc {
        code,
        uniforms: 0,
        threads: threads("FS"),
        single_seg: h.b(&format!("{up}_FS_SINGLE_SEG")),
        propagate_nans: false,
    };
    desc.fs.writes_z = h.b(&format!("{up}_FS_WRITES_Z"));
    desc.fs.disable_ez = h.b(&format!("{up}_FS_DISABLE_EZ"));
    desc.fs.num_varyings = h.d(&format!("{up}_FS_NUM_INPUTS"));
    desc.fs.uses_center_w = h.b(&format!("{up}_FS_USES_CENTER_W"));
    desc.fs.uses_pid = h.b(&format!("{up}_FS_USES_PID"));
    desc.fs.lock_scoreboard_on_first_thrsw = h.b(&format!("{up}_FS_LOCK_SCOREBOARD_ON_FIRST_THRSW"));
    desc.fs.uses_implicit_point_line_varyings = h.b(&format!("{up}_FS_USES_IMPLICIT_POINT_LINE_VARYINGS"));
    desc.vs = vdesc("VS", code + 0x100);
    desc.cs = vdesc("CS", code + 0x200);
    desc.point_size = h.b(&format!("{up}_VS_WRITES_PSIZ"));
    Program {
        desc,
        cs_reads: reads("CS"),
        vs_reads: reads("VS"),
        cs_vcm_cache_size: h.d(&format!("{up}_CS_VCM_CACHE_SIZE")),
        vs_vcm_cache_size: h.d(&format!("{up}_VS_VCM_CACHE_SIZE")),
        separate_segments: h.b(&format!("{up}_VS_SEPARATE_SEGMENTS")) || h.b(&format!("{up}_CS_SEPARATE_SEGMENTS")),
        varyings: fill71::VaryingFlags {
            flat: [h.d(&format!("{up}_FS_FLAT_SHADE_FLAGS")), 0],
            noperspective: [h.d(&format!("{up}_FS_NOPERSPECTIVE_FLAGS")), 0],
            centroid: [h.d(&format!("{up}_FS_CENTROID_FLAGS")), 0],
        },
        discards: h.b(&format!("{up}_FS_USES_DISCARD")),
        fs_uniforms: list("fs"),
        vs_uniforms: list("vs"),
        cs_uniforms: list("cs"),
    }
}

struct Programs {
    /// gl_shapes.py's `program()` (gl_test's VS and FS).
    color: Program<'static>,
    /// gl_shapes.py's `program(POINT_VS, FS)`: writes gl_PointSize.
    points: Program<'static>,
    clear: Program<'static>,
}

fn programs() -> &'static Programs {
    static P: OnceLock<Programs> = OnceLock::new();
    P.get_or_init(|| {
        let color = our_program(OURS_COLOR_UV, "ours71_color_uv", G_CODE);
        let mut points = our_program(OURS_COLOR_UV, "ours71_color_uv", G_CODE);
        points.desc.point_size = true;
        Programs { color, points, clear: our_program(OURS_CLEAR, "ours71_clear", G_CODE + 0x300) }
    })
}

fn clear_quad() -> ClearQuad<'static> {
    let quad = attr71::Attr {
        enabled: true,
        components: 4,
        gl_type: attr71::GL_FLOAT,
        normalized: false,
        integer: false,
        stride: 16,
        offset: 0,
        buffer_addr: G_VTX + 0x1000,
        divisor: 0,
        generic: [0, 0, 0, 0x3f80_0000],
        constant_addr: None,
    };
    ClearQuad { program: &programs().clear, attr: quad, dummy_addr: G_VTX }
}

/* ---- Mesa's dump ---------------------------------------------------------------- */

/// One packet or record of a clif: its name and its `/* label: */ value`
/// fields (the value's first token).
#[derive(Clone, Debug)]
struct Packet {
    name: String,
    fields: Vec<(String, String)>,
}

impl Packet {
    fn get(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("{}: no field {label}", self.name))
    }
    fn n(&self, label: &str) -> u64 {
        num(self.get(label))
    }
    fn f(&self, label: &str) -> f32 {
        self.get(label).parse().unwrap_or_else(|e| panic!("{}: {label}: {e}", self.name))
    }
    /// The byte offset of a `[bo+0x...]` address within its BO.
    fn bo_offset(&self, label: &str) -> u64 {
        let v = self.get(label);
        let off = v.trim_start_matches('[').trim_end_matches(']').split('+').nth(1).unwrap_or("0");
        num(off)
    }
}

fn clif_packets(text: &str) -> Vec<Packet> {
    let mut out: Vec<Packet> = vec![];
    let mut open = false;
    for line in text.lines() {
        let t = line.strip_prefix("MESA: info: ").unwrap_or(line);
        let s = t.trim();
        if let Some(rest) = s.strip_prefix("/*") {
            if !open {
                continue;
            }
            let Some((label, value)) = rest.split_once(":*/").or_else(|| rest.split_once(": */")) else {
                continue;
            };
            let value = value.split_whitespace().next().unwrap_or("").to_string();
            out.last_mut().unwrap().fields.push((label.trim().to_string(), value));
        } else if let Some(f) = s.strip_prefix("@format ") {
            let kind = f.split_whitespace().next().unwrap_or("");
            open = kind.starts_with("shadrec_");
            if open {
                out.push(Packet { name: kind.to_string(), fields: vec![] });
            }
        } else if !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            open = true;
            out.push(Packet { name: s.to_string(), fields: vec![] });
        } else if !s.is_empty() {
            open = false;
        }
    }
    out
}

/// One draw as the hardware sees it, in Mesa's terms (field values, not
/// addresses).
#[derive(Clone, Debug, PartialEq)]
struct Prim {
    /// `INDEXED_PRIM_LIST` or `VERTEX_ARRAY_PRIMS`.
    packet: &'static str,
    /// The packet's `mode` (`V3D_PRIM_*`).
    mode: u32,
    /// `Index type` (0 8-bit, 1 16-bit, 2 32-bit); `None` for a non-indexed draw.
    index_type: Option<u32>,
    length: u32,
    restart: bool,
    /// Where the draw starts: the first index's byte in the index data, or
    /// the first vertex's byte in the vertex buffer (attribute 0).
    start_byte: u64,
    forward: bool,
    reverse: bool,
    clockwise: bool,
    line_rasterization: u32,
    point_size: f32,
    line_width: f32,
    psiz_in_vdata: bool,
}

/// Mesa's draws of a clif's binning list, each with the state in force.
fn mesa_prims(name: &str) -> Vec<(Prim, u64)> {
    let all = clif_packets(&read(&format!("tools/v3d/pi5/{name}.clif")));
    let shadrecs: Vec<&Packet> = all.iter().filter(|p| p.name == "shadrec_gl_main").collect();
    /* the first attribute record after each main record */
    let attr0: Vec<&Packet> = all
        .iter()
        .enumerate()
        .filter(|(_, p)| p.name == "shadrec_gl_main")
        .map(|(i, _)| all[i + 1..].iter().find(|p| p.name == "shadrec_gl_attr").unwrap())
        .collect();
    let mut shader_addrs: Vec<String> = vec![];
    let (mut cfg, mut ps, mut lw, mut shader, mut ib) = (None::<&Packet>, None::<f32>, None::<f32>, None, None);
    let mut out = vec![];
    for p in &all {
        match p.name.as_str() {
            "CFG_BITS" => cfg = Some(p),
            "POINT_SIZE" => ps = Some(p.f("Point Size")),
            "LINE_WIDTH" => lw = Some(p.f("Line width")),
            "GL_SHADER" => {
                let a = p.get("address").to_string();
                let k = match shader_addrs.iter().position(|x| *x == a) {
                    Some(k) => k,
                    None => {
                        shader_addrs.push(a);
                        shader_addrs.len() - 1
                    }
                };
                shader = Some(k);
            }
            "INDEX_BUFFER_SETUP" => ib = Some(p),
            "INDEXED_PRIM_LIST" | "VERTEX_ARRAY_PRIMS" => {
                let c = cfg.expect("CFG_BITS before the draw");
                let k = shader.expect("GL_SHADER before the draw");
                let indexed = p.name == "INDEXED_PRIM_LIST";
                let a0 = attr0[k];
                let start_byte = if indexed {
                    ib.expect("INDEX_BUFFER_SETUP").bo_offset("Address") + p.n("Index Offset")
                } else {
                    a0.bo_offset("Address") + p.n("Index of First Vertex") * a0.n("Stride")
                };
                let ib_size = if indexed { ib.unwrap().n("Size") } else { 0 };
                out.push((
                    Prim {
                        packet: if indexed { "INDEXED_PRIM_LIST" } else { "VERTEX_ARRAY_PRIMS" },
                        mode: p.n("mode") as u32,
                        index_type: indexed.then(|| p.n("Index type") as u32),
                        length: p.n("Length") as u32,
                        restart: indexed && p.n("Enable Primitive Restarts") != 0,
                        start_byte,
                        forward: c.n("Enable Forward Facing Primitive") != 0,
                        reverse: c.n("Enable Reverse Facing Primitive") != 0,
                        clockwise: c.n("Clockwise Primitives") != 0,
                        line_rasterization: c.n("Line Rasterization") as u32,
                        point_size: ps.expect("POINT_SIZE"),
                        line_width: lw.expect("LINE_WIDTH"),
                        psiz_in_vdata: shadrecs[k].n("Point size in shaded vertex data") != 0,
                    },
                    ib_size,
                ));
            }
            _ => {}
        }
    }
    out
}

/// The packets v3d_job71.c writes for one of our filled draws, as [`Prim`].
/// `index_base` / `vertex_base`: where the index data and the vertex buffer
/// start (the offsets are measured from there, as Mesa's are from its BO).
fn our_prim(d: &FilledDraw, index_base: u32) -> Prim {
    /* emit_state: CFG_BITS ("Points and lines are never culled as forward
     * facing"), POINT_SIZE 1.0 (ES has no glPointSize), LINE_WIDTH */
    let polygon = d.prim >= V3D_PRIM_TRIANGLES;
    let a0 = d.attrs[0];
    Prim {
        packet: if d.indexed { "INDEXED_PRIM_LIST" } else { "VERTEX_ARRAY_PRIMS" },
        mode: d.prim,
        index_type: d.indexed.then_some(d.index_type),
        length: d.count,
        /* emit_draw: "no primitive restart" */
        restart: false,
        start_byte: if d.indexed {
            u64::from(d.index_buffer - index_base) + u64::from(d.index_offset)
        } else {
            u64::from(a0.address - G_VTX) + u64::from(d.first) * u64::from(a0.stride)
        },
        forward: !polygon || !d.state.cull_front,
        reverse: !d.state.cull_back,
        clockwise: d.state.front_ccw,
        /* V3D_LINE_RASTERIZATION_DIAMOND_EXIT */
        line_rasterization: 0,
        point_size: 1.0,
        line_width: d.state.line_width,
        psiz_in_vdata: d.vs_writes_psiz,
    }
}

/* ---- the GL core's GlesDraws (src/gl/draw.rs) ----------------------------------- */

/// The window, its fused glClear(GL_COLOR_BUFFER_BIT) at black, and the
/// default GL state (`resolve_target`, `apply_clear_plan`, `build_common`).
fn base(first_draw: bool) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = u64::from(G_RT);
    (d.target_w, d.target_h) = (G_SIZE, G_SIZE);
    d.target_format = GLES_SURF_RGBA8888;
    d.target_alpha_bits = 8;
    d.targets[0] = GlesDrawTarget { gpu: u64::from(G_RT), w: G_SIZE, h: G_SIZE, format: 0, out_shift: 0 };
    d.num_targets = 1;
    d.samples = 1;
    d.viewport = [0, 0, G_SIZE as i32, G_SIZE as i32];
    d.depth_range = [0.0, 1.0];
    if first_draw {
        /* the pending glClear goes out fused into the first draw */
        d.do_clear = true;
        d.clear_color_mask = 0xf;
        d.clear_rgba = [0.0, 0.0, 0.0, 1.0];
        d.clear_depth = 1.0;
    }
    d.instance_count = 1;
    /* glLineWidth's default, and gl_shapes.py's glLineWidth(1.0) */
    d.line_width = 1.0;
    d.color_write = true;
    d.color_mask = 0xf;
    d.depth_func = 1; /* GL_LESS, the default */
    d.depth_write = true;
    (d.stencil_func, d.stencil_cmp_mask, d.stencil_write_mask) = (7, 0xff, 0xff);
    (d.stencil_back_func, d.stencil_back_cmp_mask, d.stencil_back_write_mask) = (7, 0xff, 0xff);
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

/// The three float arrays of gl_shapes.py's `upload` (pos4, colour4, uv2 in
/// a 40-byte vertex); a non-indexed draw's first vertex is in the offsets.
fn arrays(d: &mut GlesDraw, first: u32) {
    for (k, (comps, off)) in [(4u8, 0u32), (4, 16), (2, 32)].into_iter().enumerate() {
        d.attribs[k] = GlesAttrib {
            vi_reg: k as u8,
            components: comps,
            format: GLES_ATTRIBF_FLOAT32,
            stride: STRIDE,
            offset: off + first * STRIDE,
            buffer_gpu: u64::from(G_VTX),
            ..GlesAttrib::ZERO
        };
    }
    d.num_attribs = 3;
}

/// The u16 index scratch the GL core staged (`st.index_scratch.cpu`): only
/// its address is read (glue71 needs it non-null).
static STAGED: [u16; 8] = [0; 8];

/// What a GL draw becomes: the GlesDraw, the program and where the table
/// placed the staged indices.
struct CoreDraw {
    d: GlesDraw,
    points: bool,
    indices: Option<glue71::IndexPlace>,
}

/// draw.rs `draw_elements` on a GL_ELEMENT_ARRAY_BUFFER: `staged` when the
/// indices go through the u16 scratch (u8 and u32 widened, a restart
/// rewritten), else the buffer's own address plus the offset.
fn elements(first_draw: bool, prim: u32, count: u32, staged: bool, offset: u32) -> CoreDraw {
    let mut d = base(first_draw);
    arrays(&mut d, 0);
    d.prim = prim;
    d.index_count = count;
    d.index_size = 2; /* "u8 and u32 arrays were widened above" */
    if staged {
        d.index_buf_gpu = 0x7000_0000; /* the scratch's own address, unused */
        d.index_cpu = STAGED.as_ptr().cast();
        d.index_bytes = count * 2;
    } else {
        d.index_buf_gpu = u64::from(G_IBO + offset);
    }
    let indices = staged.then_some(glue71::IndexPlace { addr: G_STAGED, size: u64::from(count) * 2 });
    CoreDraw { d, points: false, indices }
}

fn arrays_draw(first_draw: bool, prim: u32, first: u32, count: u32) -> CoreDraw {
    let mut d = base(first_draw);
    arrays(&mut d, first);
    d.prim = prim;
    d.index_count = count;
    CoreDraw { d, points: true, indices: None }
}

/// gl_shapes.py `indexed`: u8, u16 at byte 4, u32, then a restart strip.
fn core_indexed() -> Vec<CoreDraw> {
    vec![
        /* u8: read on the CPU and widened */
        elements(true, GLES_PRIM_TRIANGLES, 6, true, 0),
        /* u16 in a float VBO draw, 4-aligned: the buffer in place */
        elements(false, GLES_PRIM_TRIANGLES, 6, false, 4),
        /* u32: widened */
        elements(false, GLES_PRIM_TRIANGLES, 6, true, 0),
        /* (0, 1, 2, 0xffff, 3, 4, 5) as a strip with the fixed restart:
         * restart_to_lists gives two triangles, (0,1,2) and (3,4,5) */
        elements(false, GLES_PRIM_TRIANGLES, 6, true, 0),
    ]
}

/// gl_shapes.py `prims`: one program (POINT_VS) for the three draws.
fn core_prims() -> Vec<CoreDraw> {
    vec![
        arrays_draw(true, GLES_PRIM_POINTS, 0, 4),
        arrays_draw(false, GLES_PRIM_LINES, 4, 4),
        arrays_draw(false, GLES_PRIM_TRI_STRIP, 8, 4),
    ]
}

/// Every GlesDraw through glue71 and one `fill71::fill` (one flush:
/// gl_shapes.py's glFinish).
fn fill(draws: &[CoreDraw]) -> Result<Vec<FilledDraw>, fill71::Refusal> {
    let place = glue71::DsPlace::default();
    let mut frame: Option<glue71::Frame71> = None;
    let mut ops: Vec<Op<'static>> = vec![];
    for c in draws {
        let mut f = glue71::frame(&c.d, place)?;
        for col in &mut f.colors[..f.n] {
            (col.memory_format, col.height_in_ub_or_stride) = (V3D_MEMORY_FORMAT_UIF_NO_XOR, 8);
        }
        match &frame {
            None => frame = Some(f),
            Some(g) => assert_eq!(g, &f, "one flush, one framebuffer"),
        }
        let attrs: &'static [attr71::Attr] = Box::leak(glue71::attrs(&c.d, &[])?.into_boxed_slice());
        let p = programs();
        let input = glue71::DrawInputs {
            program: if c.points { &p.points } else { &p.color },
            attrs,
            vs_words: &[],
            fs_words: &[],
            dummy_addr: G_VTX,
            oq_addr: 0,
            indices: c.indices,
        };
        ops.extend(glue71::ops(&c.d, frame.as_ref().unwrap(), Some(&input))?);
    }
    let f = frame.expect("a draw");
    let jobs = fill71::fill(&f.framebuffer(G_TILE), record71::BCM2712_D0, &History::NEW, &clear_quad(), &ops)?;
    assert_eq!(jobs.len(), 1, "one flush, one job");
    let job = jobs.into_iter().next().unwrap();
    assert!(job.draws.iter().all(|d| !d.is_clear), "the clear is fused, no clear quad");
    Ok(job.draws)
}

/// Our draws as [`Prim`]s, the index offsets measured from the buffer the
/// draw reads (the table's staging copy, or the element buffer).
fn ours(draws: &[CoreDraw]) -> Vec<Prim> {
    let filled = fill(draws).unwrap_or_else(|r| panic!("refused: {}: {}", r.module, r.reason));
    assert_eq!(filled.len(), draws.len());
    filled.iter().zip(draws).map(|(f, c)| our_prim(f, if c.indices.is_some() { G_STAGED } else { G_IBO })).collect()
}

/* ---- the references --------------------------------------------------------------- */

#[test]
fn clifs_have_the_shapes_draws() {
    let ix = mesa_prims("gl_indexed");
    assert_eq!(ix.len(), 4, "gl_indexed: four glDrawElements");
    assert!(ix.iter().all(|(p, _)| p.packet == "INDEXED_PRIM_LIST"));
    assert_eq!(ix.iter().map(|(p, _)| p.index_type).collect::<Vec<_>>(), [Some(0), Some(1), Some(2), Some(1)]);
    assert_eq!(ix.iter().map(|(p, _)| p.restart).collect::<Vec<_>>(), [false, false, false, true]);
    assert_eq!(ix[1].0.start_byte, 4, "GL_UNSIGNED_SHORT at byte offset 4");
    let pr = mesa_prims("gl_prims");
    assert_eq!(
        pr.iter().map(|(p, _)| (p.packet, p.mode, p.length)).collect::<Vec<_>>(),
        [
            ("VERTEX_ARRAY_PRIMS", V3D_PRIM_POINTS, 4),
            ("VERTEX_ARRAY_PRIMS", V3D_PRIM_LINES, 4),
            ("VERTEX_ARRAY_PRIMS", V3D_PRIM_TRIANGLE_STRIP, 4)
        ]
    );
    assert_eq!(pr.iter().map(|(p, _)| p.psiz_in_vdata).collect::<Vec<_>>(), [true, false, false]);
}

/* ---- gl_prims ----------------------------------------------------------------------- */

#[test]
fn gl_prims_points_lines_strip_match_mesa() {
    let mesa = mesa_prims("gl_prims");
    let ours = ours(&core_prims());
    assert_eq!(ours.len(), mesa.len());
    for (k, (o, (m, _))) in ours.iter().zip(&mesa).enumerate() {
        /* gap_psiz_all_prims is the one field apart, pinned below */
        let o = Prim { psiz_in_vdata: m.psiz_in_vdata, ..o.clone() };
        assert_eq!(&o, m, "gl_prims draw {k}");
    }
}

#[test]
fn gl_prims_psiz_pinned() {
    /* one program for every primitive: the flag follows the program */
    let ours = ours(&core_prims());
    assert_eq!(ours.iter().map(|p| p.psiz_in_vdata).collect::<Vec<_>>(), [true, true, true]);
}

#[test]
#[ignore = "gap_psiz_all_prims: one linked program flags gl_PointSize for lines and strips (Mesa: a variant without it)"]
fn gap_psiz_all_prims() {
    let mesa = mesa_prims("gl_prims");
    let ours = ours(&core_prims());
    assert_eq!(
        ours.iter().map(|p| p.psiz_in_vdata).collect::<Vec<_>>(),
        mesa.iter().map(|(p, _)| p.psiz_in_vdata).collect::<Vec<_>>()
    );
}

#[test]
fn glue71_maps_every_primitive() {
    /* the modes gl_prims does not draw: line strip and loop, triangle fan,
     * and triangles, to v3d_job71's V3D_PRIM_* */
    for (gles, v3d) in [
        (GLES_PRIM_POINTS, V3D_PRIM_POINTS),
        (GLES_PRIM_LINES, V3D_PRIM_LINES),
        (GLES_PRIM_LINE_LOOP, V3D_PRIM_LINE_LOOP),
        (GLES_PRIM_LINE_STRIP, V3D_PRIM_LINE_STRIP),
        (GLES_PRIM_TRIANGLES, V3D_PRIM_TRIANGLES),
        (GLES_PRIM_TRI_STRIP, V3D_PRIM_TRIANGLE_STRIP),
        (GLES_PRIM_TRI_FAN, V3D_PRIM_TRIANGLE_FAN),
    ] {
        let o = ours(&[arrays_draw(true, gles, 2, 5)]);
        assert_eq!((o[0].mode, o[0].length, o[0].start_byte), (v3d, 5, 2 * u64::from(STRIDE)), "GLES prim {gles}");
        let polygon = v3d >= V3D_PRIM_TRIANGLES;
        assert!(o[0].forward && o[0].reverse, "GLES prim {gles}: nothing culled");
        assert!(!o[0].clockwise, "GLES prim {gles} (polygon {polygon})");
    }
}

#[test]
fn line_width_follows_the_draw() {
    /* gl_primswide's widths (glLineWidth 2.0 and 4.0) reach LINE_WIDTH */
    for w in [2.0f32, 4.0] {
        let mut c = arrays_draw(true, GLES_PRIM_LINES, 4, 4);
        c.d.line_width = w;
        assert_eq!(ours(&[c])[0].line_width, w);
    }
}

/* ---- gl_indexed --------------------------------------------------------------------- */

#[test]
fn gl_indexed_u16_at_offset_matches_mesa() {
    let mesa = mesa_prims("gl_indexed");
    let ours = ours(&core_indexed());
    assert_eq!(ours.len(), mesa.len());
    /* the draw the GL core takes in place: the same packet, its offset
     * folded into the address */
    assert_eq!(ours[1], mesa[1].0, "u16 at byte 4");
}

#[test]
fn gl_indexed_pinned() {
    let mesa = mesa_prims("gl_indexed");
    let ours = ours(&core_indexed());
    for (k, (o, (m, mesa_ib_size))) in ours.iter().zip(&mesa).enumerate() {
        /* what we write today: every draw a 16-bit TRIANGLES list of 6 from
         * the start of its index data, no restart */
        assert_eq!(
            (o.packet, o.mode, o.index_type, o.length, o.restart),
            ("INDEXED_PRIM_LIST", V3D_PRIM_TRIANGLES, Some(1), 6, false),
            "draw {k}"
        );
        assert_eq!(o.start_byte, if k == 1 { 4 } else { 0 }, "draw {k}");
        /* the state packets agree on every draw */
        let state = |p: &Prim| (p.forward, p.reverse, p.clockwise, p.line_rasterization, p.point_size, p.line_width);
        assert_eq!(state(o), state(m), "draw {k}: CFG_BITS / POINT_SIZE / LINE_WIDTH");
        assert_eq!(o.psiz_in_vdata, m.psiz_in_vdata, "draw {k}");
        assert!(*mesa_ib_size >= 6 * 2, "draw {k}: Mesa's index BO covers the draw");
    }
    /* the triangles Mesa draws in mode 4 from 6 indices: the same count */
    for (m, _) in &mesa[..3] {
        assert_eq!((m.mode, m.length), (V3D_PRIM_TRIANGLES, 6));
    }
}

#[test]
fn index_buffer_setup_covers_the_draw() {
    let draws = core_indexed();
    let filled = fill(&draws).unwrap();
    for (k, f) in filled.iter().enumerate() {
        assert!(f.indexed);
        let read = f.index_offset + f.count * 2;
        assert!(f.index_buffer_bytes >= read, "draw {k}: {} bytes for {read}", f.index_buffer_bytes);
    }
}

#[test]
#[ignore = "gap_index_widen: the GL core widens u8 and u32 indices to u16 (Mesa: index types 0 and 2)"]
fn gap_index_widen() {
    let mesa = mesa_prims("gl_indexed");
    let ours = ours(&core_indexed());
    assert_eq!(ours[0], mesa[0].0, "u8");
    assert_eq!(ours[2], mesa[2].0, "u32");
}

#[test]
#[ignore = "gap_restart_as_lists: a fixed-index restart strip is drawn as a triangle list (Mesa: TRIANGLE_STRIP 7, restart 1)"]
fn gap_restart_as_lists() {
    let mesa = mesa_prims("gl_indexed");
    let ours = ours(&core_indexed());
    assert_eq!(ours[3], mesa[3].0, "restart strip");
}

#[test]
fn glue71_maps_every_index_size() {
    /* glue71 maps the GlesDraw's index size to Mesa's index types: the GL
     * core, not glue71, is what makes gap_index_widen */
    for (size, ty) in [(1u8, 0u32), (2, 1), (4, 2)] {
        let mut c = elements(true, GLES_PRIM_TRIANGLES, 6, false, 0);
        c.d.index_size = size;
        let o = ours(&[c]);
        assert_eq!(o[0].index_type, Some(ty), "index size {size}");
    }
    let mut c = elements(true, GLES_PRIM_TRIANGLES, 6, false, 0);
    c.d.index_size = 3;
    let r = fill(&[c]).unwrap_err();
    assert_eq!(r.module, "prim71");
}

#[test]
fn fill71_refuses_restart() {
    /* a table that kept the strip and its restart (what gap_restart_as_lists
     * needs) would be refused by fill71 today */
    let c = elements(true, GLES_PRIM_TRI_STRIP, 7, false, 0);
    let fr = glue71::frame(&c.d, glue71::DsPlace::default()).unwrap();
    let mut p = glue71::prim(&c.d, None).unwrap();
    assert!(p.indices.is_some_and(|i| !i.restart), "glue71 writes no restart");
    p.indices.as_mut().unwrap().restart = true;
    let attrs = glue71::attrs(&c.d, &[]).unwrap();
    let ops = [Op::Draw(Box::new(fill71::DrawIn {
        state: glue71::draw_state(&c.d, &fr).unwrap(),
        line_width: 1.0,
        blend_color: [0.0; 4],
        program: &programs().color,
        attrs: &attrs,
        dummy_addr: G_VTX,
        prim: p,
        vs_words: &[],
        fs_words: &[],
        oq_addr: 0,
    }))];
    let r = fill71::fill(&fr.framebuffer(G_TILE), record71::BCM2712_D0, &History::NEW, &clear_quad(), &ops)
        .expect_err("restart");
    assert_eq!(r.module, "fill71");
    assert!(r.reason.contains("primitive restart"), "{}", r.reason);
}
