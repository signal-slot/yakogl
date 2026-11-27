// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 table draws indexed triangles (plan item idx71):
//! `yakogl_backend_v3d71` (src/v3d/table71.rs) driven through the GL core's
//! backend entries with one `glDrawElements(GL_TRIANGLES, 6,
//! GL_UNSIGNED_SHORT, ...)` and gl_test's `color_uv` program, and the
//! indexed packets of the job it submits compared with Mesa's for the same
//! GL call: the second draw of tools/v3d/pi5/gl_indexed.clif (gl_shapes.py
//! `indexed`, GL_UNSIGNED_SHORT at byte offset 4 of an element buffer).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table_indexed
//! ```
//!
//! The OSAL is the mock of tests/v3d71_table_rig/mod.rs, the C builder the
//! real driver/src/v3d/v3d_job71.c (as in tests/v3d71_table_draw.rs), so the
//! binning list decoded here is the bytes the Pi 5 image would submit.
//!
//! Compared, per indexed draw, with Mesa's `INDEX_BUFFER_SETUP` and
//! `INDEXED_PRIM_LIST`: `mode`, `Index type`, `Length`, `Enable Primitive
//! Restarts`, and where the first index is (the setup's address within its
//! buffer plus `Index Offset`: Mesa names the BO and offset 4, the GL core
//! folds the offset into `index_buf_gpu`, glue71's documented choice, so
//! ours is the buffer's byte 4 with offset 0). Not compared: the BO
//! addresses and the setup's `Size` (Mesa gives the whole 4096-byte BO,
//! v3d_job71 the bytes the draw reads; checked to be those bytes).
//!
//! A draw from client memory (or the GL core's u16 scratch) is staged by
//! the table into its data arena: the job reads the GlesDraw's `index_cpu`
//! bytes, not whatever sits at `index_buf_gpu`. Neither draw is refused
//! (`table71::refused_draws()` does not grow). 8-bit and 32-bit indices are
//! `backend71::REF_INDEX_TYPE`'s refusal (tests/v3d71_table_draw.rs): the
//! GL core widens them to u16 before the back end.
#![cfg(feature = "gpu-v3d71")]
#![allow(dead_code)]

mod v3d71_table_rig;

use std::collections::BTreeMap;
use std::ffi::c_void;

use v3d71_table_rig::{gpu_block, gpu_write, kicks, log_lines, rig, Kick};
use yakogl::backend::{
    self, GlesAttrib, GlesDraw, GlesDrawTarget, GLES_ATTRIBF_FLOAT32, GLES_CULL_NONE, GLES_PRIM_TRIANGLES,
    GLES_SOP_KEEP, GLES_SURF_RGBA8888,
};
use yakogl::shaderc::{ShcShader, ShcV3dUniform, ShcV3dVs};
use yakogl::v3d::ffi71::{V3D_INDEX_TYPE_16, V3D_PRIM_TRIANGLES};
use yakogl::v3d::table71;

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
        Box::leak(Box::new(sh)) as *const ShcShader as usize
    });
    // SAFETY: leaked above, never freed or written again.
    unsafe { &*(p as *const ShcShader) }
}

/// gl_test's 40-byte vertex: pos4 +0, colour4 +16, uv2 +32, six of them.
/// A 40-byte vertex: pos4 +0, colour4 +16, uv2 +32, six of them.
const STRIDE: u32 = 40;
const VERTICES: usize = 6;
const G_SIZE: u32 = 64;
const DS_ID: u32 = 200;
/// gl_shapes.py `indexed`'s u16 draw: six indices at byte 4 of the buffer.
const INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];
const OFFSET: u32 = 4;

/// The GL core's side: a 64x64 RGBA8 window with a D24S8, its surface, the
/// program's code, the vertices and an element buffer in GPU memory.
struct Core {
    surface: u32,
    code: u32,
    vbo: u32,
    ibo: u32,
}

impl Core {
    fn new() -> Core {
        let vbo = gpu_block(VERTICES * STRIDE as usize);
        gpu_write(vbo, &(0..VERTICES * STRIDE as usize).map(|i| (i * 7 + 3) as u8).collect::<Vec<u8>>());
        let ibo = gpu_block(4096);
        let mut ib = vec![0xeeu8; OFFSET as usize];
        ib.extend(INDICES.iter().flat_map(|i| i.to_le_bytes()));
        gpu_write(ibo, &ib);
        Core { surface: gpu_block((G_SIZE * G_SIZE * 4) as usize), code: gpu_block(0x300), vbo, ibo }
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

    /// The arrays of `glDrawElements(GL_TRIANGLES, 6, ...)` with `color_uv`,
    /// every attribute from the buffer object; the caller names the indices.
    fn elements(&self) -> GlesDraw {
        let mut d = self.base();
        d.index_count = INDICES.len() as u32;
        d.index_size = 2;
        d.instance_count = 1;
        d.line_width = 1.0;
        d.fs = color_uv();
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
        (d.depth_func, d.depth_write) = (backend::GLES_CMP_LESS, true);
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

    /// From the element buffer at byte [`OFFSET`] (draw.rs `draw_elements`
    /// folds the offset into the address).
    fn buffer_draw(&self) -> GlesDraw {
        let mut d = self.elements();
        d.index_buf_gpu = u64::from(self.ibo + OFFSET);
        d
    }
}

/* ---- the indexed packets ------------------------------------------------------------ */

/// One indexed draw: `INDEX_BUFFER_SETUP`'s address and size, then
/// `INDEXED_PRIM_LIST`'s fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Indexed {
    address: u64,
    size: u64,
    mode: u64,
    index_type: u64,
    length: u64,
    restart: u64,
    offset: u64,
}

/// A field of a packet body, raw (`minus_one` undone).
fn field(def: &PacketDef, body: &[u8], name: &str) -> u64 {
    let f = def.fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{}.{name}", def.name));
    bits(body, f.start, f.size) + f.minus_one as u64
}

/// Our job's indexed draws, and how many draws of any kind its binning
/// list holds.
fn ours(k: &Kick) -> (Vec<Indexed>, usize) {
    let defs = packet_defs();
    let mut setup: Option<(u64, u64)> = None;
    let (mut out, mut draws) = (vec![], 0);
    for (def, body) in walk(&defs, k.bcl(), "bcl") {
        match def.name.as_str() {
            "INDEX_BUFFER_SETUP" => setup = Some((field(def, body, "Address"), field(def, body, "Size"))),
            "INDEXED_PRIM_LIST" => {
                let (address, size) = setup.take().expect("INDEX_BUFFER_SETUP before INDEXED_PRIM_LIST");
                out.push(Indexed {
                    address,
                    size,
                    mode: field(def, body, "mode"),
                    index_type: field(def, body, "Index type"),
                    length: field(def, body, "Length"),
                    restart: field(def, body, "Enable Primitive Restarts"),
                    offset: field(def, body, "Index Offset"),
                });
                draws += 1;
            }
            "VERTEX_ARRAY_PRIMS"
            | "VERTEX_ARRAY_INSTANCED_PRIMS"
            | "VERTEX_ARRAY_SINGLE_INSTANCE_PRIMS"
            | "INDEXED_INSTANCED_PRIM_LIST" => draws += 1,
            _ => {}
        }
    }
    (out, draws)
}

/// Mesa's indexed draws of tools/v3d/pi5/gl_indexed.clif, the setup's
/// address as its offset within the BO (`[resource_0x...+0x...]`).
fn mesa() -> Vec<Indexed> {
    let text = read("tools/v3d/pi5/gl_indexed.clif");
    let mut packets: Vec<(String, BTreeMap<String, String>)> = vec![];
    for line in text.lines() {
        let s = line.strip_prefix("MESA: info: ").unwrap_or(line).trim();
        if !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            packets.push((s.to_string(), BTreeMap::new()));
        } else if let (Some(rest), Some(p)) = (s.strip_prefix("/*"), packets.last_mut()) {
            if let Some((label, value)) = rest.split_once(": */") {
                p.1.insert(label.trim().to_string(), value.split_whitespace().next().unwrap_or("").to_string());
            }
        }
    }
    let n = |m: &BTreeMap<String, String>, k: &str| num(m.get(k).unwrap_or_else(|| panic!("gl_indexed: {k}")));
    let mut setup: Option<(u64, u64)> = None;
    let mut out = vec![];
    for (name, f) in &packets {
        match name.as_str() {
            "INDEX_BUFFER_SETUP" => {
                let a = &f["Address"];
                let off = a.trim_start_matches('[').trim_end_matches(']').split('+').nth(1).unwrap_or("0");
                setup = Some((num(off), n(f, "Size")));
            }
            "INDEXED_PRIM_LIST" => {
                let (address, size) = setup.take().expect("gl_indexed: INDEX_BUFFER_SETUP first");
                out.push(Indexed {
                    address,
                    size,
                    mode: n(f, "mode"),
                    index_type: n(f, "Index type"),
                    length: n(f, "Length"),
                    restart: n(f, "Enable Primitive Restarts"),
                    offset: n(f, "Index Offset"),
                });
            }
            _ => {}
        }
    }
    out
}

/// Mesa's u16 draw at byte 4 (gl_shapes.py `indexed`'s second draw).
fn mesa_u16() -> Indexed {
    let m = mesa();
    assert_eq!(m.len(), 4, "gl_indexed: four glDrawElements");
    let d = m[1];
    assert_eq!((d.index_type, d.length, d.offset), (u64::from(V3D_INDEX_TYPE_16), 6, u64::from(OFFSET)));
    d
}

/// The fields a draw shares with Mesa's whatever the buffer.
fn shape(d: &Indexed) -> (u64, u64, u64, u64) {
    (d.mode, d.index_type, d.length, d.restart)
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

/* ---- the tests -------------------------------------------------------------------------- */

#[test]
fn the_clif_has_the_u16_draw() {
    let d = mesa_u16();
    assert_eq!(shape(&d), (u64::from(V3D_PRIM_TRIANGLES), 1, 6, 0));
    assert_eq!(d.address + d.offset, 4, "the first index at byte 4 of the BO");
}

#[test]
fn a_u16_draw_from_a_buffer_is_mesas_indexed_prim_list() {
    let (_g, c) = started();
    let (k, refused) = one_job(&c, &c.buffer_draw());
    assert_eq!(refused, 0, "refused_draws() did not grow");
    let (ix, draws) = ours(&k);
    assert_eq!((ix.len(), draws), (1, 1), "the one draw, indexed");
    let (o, m) = (ix[0], mesa_u16());
    assert_eq!(shape(&o), shape(&m), "mode, index type, length, restart");
    /* the first index: Mesa's BO byte 4 through Index Offset, ours through
     * the address (the GL core folded the offset) */
    assert_eq!(o.address - u64::from(c.ibo) + o.offset, m.address + m.offset, "where the first index is");
    assert_eq!(o.offset, 0);
    /* the size: the bytes the draw reads (Mesa's is the whole BO) */
    assert_eq!(o.size, 2 * INDICES.len() as u64);
    let read: Vec<u8> = INDICES.iter().flat_map(|i| i.to_le_bytes()).collect();
    assert_eq!(k.at((o.address + o.offset) as u32, read.len()), &read[..], "the job reads the draw's indices");
}

#[test]
fn a_u16_draw_from_client_memory_reads_a_staged_copy() {
    let (_g, c) = started();
    /* the GL core's u16 scratch: its GPU copy is stale here, so a job that
     * read it instead of the table's copy of `index_cpu` would read 0xee */
    let scratch = gpu_block(64);
    gpu_write(scratch, &[0xee; 64]);
    let client: Vec<u8> = INDICES.iter().flat_map(|i| i.to_le_bytes()).collect();
    let mut d = c.elements();
    d.index_buf_gpu = u64::from(scratch);
    (d.index_cpu, d.index_bytes) = (client.as_ptr() as *const c_void, client.len() as u32);
    let (k, refused) = one_job(&c, &d);
    assert_eq!(refused, 0, "refused_draws() did not grow");
    let (ix, draws) = ours(&k);
    assert_eq!((ix.len(), draws), (1, 1), "the one draw, indexed");
    let o = ix[0];
    assert_eq!(shape(&o), shape(&mesa_u16()), "mode, index type, length, restart");
    assert_ne!(o.address, u64::from(scratch), "the table's copy, not the scratch");
    assert_eq!((o.offset, o.size), (0, client.len() as u64));
    assert_eq!(k.at(o.address as u32, client.len()), &client[..], "the job reads the staged indices");
}

#[test]
fn two_indexed_draws_share_a_job() {
    let (_g, c) = started();
    let refused = table71::refused_draws();
    let client: Vec<u8> = [3u16, 2, 1, 0, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect();
    let mut staged = c.elements();
    (staged.index_buf_gpu, staged.index_cpu, staged.index_bytes) =
        (u64::from(gpu_block(64)), client.as_ptr() as *const c_void, client.len() as u32);
    assert_eq!(backend::draw(&c.clear()), 0, "{:?}", log_lines());
    assert_eq!(backend::draw(&c.buffer_draw()), 0, "{:?}", log_lines());
    assert_eq!(backend::draw(&staged), 0, "{:?}", log_lines());
    assert_eq!(backend::finish(), 0, "{:?}", log_lines());
    assert_eq!(table71::refused_draws(), refused, "refused_draws() did not grow");
    let ks = kicks();
    assert_eq!(ks.len(), 1);
    let (ix, draws) = ours(&ks[0]);
    assert_eq!((ix.len(), draws), (2, 2));
    assert_eq!(ix[0].address, u64::from(c.ibo + OFFSET));
    assert_eq!(ks[0].at(ix[1].address as u32, client.len()), &client[..]);
    assert!(ix.iter().all(|d| shape(d) == shape(&mesa_u16())));
}
