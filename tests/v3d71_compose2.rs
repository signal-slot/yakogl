// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged pure V3D 7.1 modules put together a second time (compose71b):
//! `tests/v3d71_compose.rs`'s set (`attr71`, `rt71`, `draw71`, `frame71`,
//! `ident71`) plus `record71` and `limits71`, against Mesa's 7.1 references
//! in tools/v3d/pi5/ for the shapes compose71 does not reach.
//!
//! * `gl_viewport` (gl_test case 66), `gl_colormask` (102), `gl_writemask`
//!   (59), `gl_mrt8` and `gl_fbods` (67): the frame packets of every job
//!   (binning and rendering config, `Clear Render Targets`, tile-buffer
//!   loads and stores of colour, depth and stencil), the state packets of
//!   every draw, and the attribute records. What GL each shape issues is
//!   tools/v3d/pi5/src/gl_shapes.py; each test below says which job and
//!   draw is which.
//! * the `GL Shader State Record Draw Index` (32 bytes) of every draw of all
//!   nine shapes (compose71's clear, triangle, gl_scissor, gl_mrt4 and the
//!   five above), built through `record71` from the values the dump shows;
//! * the tile size and target-count answers of `limits71` for every job.
//!
//! The dump decodes the D0 record with the plain 7.1 layout
//! (tools/v3d/pi5/README.md "Differences" item 4; docs/notes/v3d71-record-u11.md
//! section 1), so a printed label is not the field at that bit in the D0
//! layout. The reference record is rebuilt here from the printed values at
//! the plain layout's bit positions (`PLAIN`), which gives the 32 bytes Mesa
//! wrote, and those bytes are read back at record71's own positions (`FIELDS`)
//! to make the `DrawDesc` handed to `record71`. Three D0 bits have no field in
//! the plain layout and so are not in the dump at all: bit 21 (`Disable
//! implicit point/line varyings`, 1 for these shaders: the README says the
//! values read at the D0 layout are the shim's), 22 and 23 (0, Mesa never
//! sets them); the test pins them to those values separately.
//!
//! The dump's `@wait_render_all_cores` boundaries do not follow the GL jobs
//! (a job's binner list, render list and tile list are printed in separate
//! buffers, some after the next job's marker), so the packets are taken in
//! file order and the i-th binner list, render list and tile list are job i.
//! Compared like compose71 does: field values and packet bytes, not BO
//! addresses or order of the dump's buffers.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_compose2
//! ```
#[path = "../src/v3d/attr71.rs"]
#[allow(dead_code)]
mod attr71;
#[path = "../src/v3d/draw71.rs"]
#[allow(dead_code)]
mod draw71;
#[path = "../src/v3d/frame71.rs"]
#[allow(dead_code)]
mod frame71;
#[path = "../src/v3d/ident71.rs"]
#[allow(dead_code)]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
#[allow(dead_code)]
mod limits71;
#[path = "../src/v3d/record71.rs"]
#[allow(dead_code)]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(dead_code)]
mod rt71;

use std::collections::BTreeMap;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ---- the dump ------------------------------------------------------------

/// One packet (or attribute record) as a dump prints it: the name and the
/// first token of each `/* label: */ value` line (the whole text for an
/// address).
#[derive(Clone, Debug, PartialEq)]
struct Pk {
    name: String,
    fields: Vec<(String, String)>,
}

/// One job of a dump: the control-list packets in file order and the
/// attribute records, one group per draw (the dump restarts the record
/// index at 0 for each draw's shader state record).
struct ClifJob {
    lists: Vec<Pk>,
    attr_groups: Vec<Vec<Pk>>,
}

fn parse_jobs(file: &str) -> Vec<ClifJob> {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let mut jobs = vec![];
    let mut cur = ClifJob { lists: vec![], attr_groups: vec![] };
    let mut fmt = String::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MESA: info:") else { continue };
        let t = rest.trim_end();
        let tt = t.trim();
        if tt.is_empty() || tt.starts_with("0x") {
            continue;
        }
        if tt == "@wait_render_all_cores" {
            jobs.push(std::mem::replace(&mut cur, ClifJob { lists: vec![], attr_groups: vec![] }));
            continue;
        }
        if tt.starts_with("@buffer") {
            fmt.clear();
            continue;
        }
        if let Some(f) = tt.strip_prefix("@format ") {
            fmt = f.split_whitespace().next().unwrap_or("").to_string();
            if fmt == "shadrec_gl_attr" {
                let idx: u32 = f.split("/*").nth(1).and_then(|s| s.split("*/").next()).unwrap().trim().parse().unwrap();
                if idx == 0 {
                    cur.attr_groups.push(vec![]);
                }
                cur.attr_groups.last_mut().unwrap().push(Pk { name: "ATTR".into(), fields: vec![] });
            }
            continue;
        }
        if tt.starts_with('@') {
            fmt.clear();
            continue;
        }
        if let Some(body) = tt.strip_prefix("/*") {
            let Some(at) = body.find(": */") else { continue };
            let label = body[..at].trim().to_string();
            let rest = body[at + 4..].trim();
            let value = if rest.starts_with('[') {
                rest.to_string()
            } else {
                rest.split_whitespace().next().unwrap_or("").to_string()
            };
            let target = match fmt.as_str() {
                "ctrllist" => cur.lists.last_mut(),
                "shadrec_gl_attr" => cur.attr_groups.last_mut().and_then(|g| g.last_mut()),
                _ => None,
            };
            if let Some(p) = target {
                p.fields.push((label, value));
            }
        } else if fmt == "ctrllist" && tt.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            cur.lists.push(Pk { name: tt.to_string(), fields: vec![] });
        }
    }
    jobs
}

/// The packets from the first `start` to the next `end`, both included.
fn between(lists: &[Pk], start: &str, end: &str) -> Vec<Pk> {
    let a = lists.iter().position(|p| p.name == start).unwrap_or_else(|| panic!("no {start} in the dump"));
    let b = a + lists[a..].iter().position(|p| p.name == end).unwrap_or_else(|| panic!("no {end} in the dump"));
    lists[a..=b].to_vec()
}

// ---- the layouts, read from the modules ------------------------------------

/// How to read one packet's bytes back as dump text.
struct Layout {
    clif: &'static str,
    code: u8,
    /// the low three bits of the first body byte (the sub-id), for packet 121's parts
    sub: Option<u8>,
    fields: Vec<(String, u32, u32)>,
    minus_one: &'static [&'static str],
    floats: &'static [&'static str],
    signed: &'static [&'static str],
}

fn lay(clif: &'static str, spec: &frame71::Spec, sub: Option<u8>, minus_one: &'static [&'static str]) -> Layout {
    Layout {
        clif,
        code: spec.code,
        sub,
        fields: spec.fields.iter().map(|f| (f.name.to_string(), f.start, f.size)).collect(),
        minus_one,
        floats: &["Z Clear Value"],
        signed: &[],
    }
}

fn draw_lay(s: &draw71::PacketSpec) -> Layout {
    Layout {
        clif: s.clif_name,
        code: s.code,
        sub: None,
        fields: s.fields.iter().map(|f| (f.name.to_string(), f.start, f.size)).collect(),
        minus_one: &[],
        floats: &[
            "Viewport Half-Height in 1/64th of pixel",
            "Viewport Half-Width in 1/64th of pixel",
            "Viewport Z Offset (Zc to Zs)",
            "Viewport Z Scale (Zc to Zs)",
            "Maximum Zw",
            "Minimum Zw",
            "Limit",
        ],
        signed: &["Coarse X", "Coarse Y"],
    }
}

fn rt_lay(clif_sub: u8, fields: &[(&str, u32, u32)]) -> Layout {
    Layout {
        clif: "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1",
        code: rt71::TILE_RENDERING_MODE_CFG_CODE,
        sub: Some(clif_sub),
        fields: fields.iter().map(|(n, s, z)| (n.to_string(), *s, *z)).collect(),
        minus_one: &["Stride"],
        floats: &[],
        signed: &[],
    }
}

fn layouts() -> Vec<Layout> {
    use frame71::*;
    let mut v = vec![
        lay("TILE_BINNING_MODE_CFG", &BINNING_CFG, None, &["Height (in pixels)", "Width (in pixels)"]),
        lay("TILE_RENDERING_MODE_CFG_COMMON", &RENDERING_COMMON, Some(0), &["Number of Render Targets"]),
        lay("TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", &ZS_CLEAR_VALUES, Some(1), &[]),
        lay("TILE_LIST_INITIAL_BLOCK_SIZE", &INITIAL_BLOCK_SIZE, None, &[]),
        lay("MULTICORE_RENDERING_TILE_LIST_BASE", &TILE_LIST_SET_BASE, None, &[]),
        lay(
            "MULTICORE_RENDERING_SUPERTILE_CFG",
            &SUPERTILE_CFG,
            None,
            &["Number of Bin Tile Lists", "Supertile Height in Tiles", "Supertile Width in Tiles"],
        ),
        lay("TILE_COORDS", &TILE_COORDS, None, &[]),
        lay("SUPERTILE_COORDS", &SUPERTILE_COORDS, None, &[]),
        lay("GENERIC_TILE_LIST", &GENERIC_TILE_LIST, None, &[]),
        lay("BRANCH_IMPLICIT_TILE", &BRANCH_IMPLICIT_TILE, None, &[]),
        lay("NUMBER_OF_LAYERS", &NUMBER_OF_LAYERS, None, &["Number of Layers"]),
        lay("STORE", &STORE, None, &[]),
        lay("LOAD", &LOAD, None, &[]),
    ];
    for s in draw71::ALL_SPECS {
        v.push(draw_lay(s));
    }
    v.push(rt_lay(rt71::PART1_SUB_ID as u8, rt71::PART1_FIELDS));
    v
}

/// The four-byte-or-less one-byte packets frame71 and rt71 write.
const SIMPLE: [(u8, &str); 8] = [
    (frame71::OP_END_RENDER, "END_RENDER"),
    (frame71::OP_RETURN, "RETURN"),
    (frame71::OP_CLEAR_VCD_CACHE, "CLEAR_VCD_CACHE"),
    (frame71::OP_CLEAR_RT, "CLEAR_RT"),
    (frame71::OP_END_LOADS, "END_LOADS"),
    (frame71::OP_END_TILE, "END_TILE"),
    (frame71::OP_IMPLICIT_TILE_COORDS, "IMPLICIT_TILE_COORDS"),
    (6, "START_TILE_BINNING"),
];

fn bits(body: &[u8], start: u32, size: u32) -> u64 {
    let mut v = 0u64;
    for i in 0..size {
        let at = start + i;
        if body[(at / 8) as usize] >> (at % 8) & 1 != 0 {
            v |= 1 << i;
        }
    }
    v
}

/// Read a packet's bytes back as the dump's text, through the layouts.
fn decode(bytes: &[u8]) -> Pk {
    if bytes.len() == 1 {
        let (_, n) =
            SIMPLE.iter().find(|(c, _)| *c == bytes[0]).unwrap_or_else(|| panic!("one-byte packet {}", bytes[0]));
        return Pk { name: n.to_string(), fields: vec![] };
    }
    let body = &bytes[1..];
    let l = layouts()
        .into_iter()
        .find(|l| l.code == bytes[0] && l.sub.is_none_or(|s| body[0] & 7 == s))
        .unwrap_or_else(|| panic!("no layout for opcode {}", bytes[0]));
    let mut fields = vec![];
    for (name, start, size) in &l.fields {
        let raw = bits(body, *start, *size);
        let text = if l.floats.contains(&name.as_str()) {
            format!("{:.6}", f32::from_bits(raw as u32))
        } else if l.signed.contains(&name.as_str()) {
            let sh = 64 - size;
            (((raw << sh) as i64) >> sh).to_string()
        } else {
            (raw + l.minus_one.contains(&name.as_str()) as u64).to_string()
        };
        fields.push((name.clone(), text));
    }
    Pk { name: l.clif.to_string(), fields }
}

/// An attribute record's 16 bytes as the dump's text (`FIELDS` of attr71;
/// a Vec size of 0 is printed as the count 4).
fn decode_attr(b: &[u8; 16]) -> Pk {
    let mut fields = vec![];
    for (name, start, size) in attr71::FIELDS {
        let mut raw = bits(b, start, size);
        if name == "Vec size" && raw == 0 {
            raw = 4;
        }
        fields.push((name.to_string(), raw.to_string()));
    }
    Pk { name: "ATTR".into(), fields }
}

fn is_address(label: &str) -> bool {
    matches!(label, "Address" | "address" | "start" | "end")
}

/// Every value the dump prints equals the bytes' field, but the addresses.
fn assert_same(what: &str, got: &Pk, want: &Pk) {
    assert_eq!(got.name, want.name, "{what}: packet");
    for (label, value) in &want.fields {
        if is_address(label) {
            continue;
        }
        let g = got
            .fields
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("{what}: {} has no field {label:?} in the module", want.name));
        assert_eq!(&g.1, value, "{what}: {} / {label}", want.name);
    }
}

fn assert_list(what: &str, got: &[Vec<u8>], want: &[Pk]) {
    let names = |v: &[Pk]| v.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    let decoded: Vec<Pk> = got.iter().map(|b| decode(b)).collect();
    assert_eq!(names(&decoded), names(want), "{what}: the packet sequence");
    for (i, (g, w)) in decoded.iter().zip(want).enumerate() {
        assert_same(&format!("{what} #{i}"), g, w);
    }
}

// ---- a backend's assembly --------------------------------------------------

/// `Height in UB or Stride` of a UIF target: the height in 8-row blocks.
fn uif_height(h: u32) -> u32 {
    h.div_ceil(8)
}

/// A GL_RGBA8 target of the job: `Output Image Format` 27 (rgba8), UIF (No
/// XOR).
fn rgba8_surface(height: u32, address: u32) -> frame71::Surface71 {
    frame71::Surface71 {
        format: 27,
        memory_format: 4,
        height_in_ub_or_stride: uif_height(height),
        address,
        ..Default::default()
    }
}

/// What the backend knows about one colour target.
struct Target {
    gl_internal: u32,
    clear: rt71::ClearColor,
    surface: frame71::Surface71,
    load: bool,
}

const GL_RGBA8: u32 = 0x8058;

fn fclear(c: [f32; 4]) -> rt71::ClearColor {
    rt71::ClearColor::Float(c)
}

/// The packets of one job, as byte vectors in the order a 7.1 backend
/// writes them.
struct Assembled {
    binner: Vec<Vec<u8>>,
    render: Vec<Vec<u8>>,
    tile: Vec<Vec<u8>>,
}

/// The target parts of the render list: Part1 (and Part2, Part3 from 64 and
/// 128 bpp) per colour target, the base moving by `rt_base_step`.
fn rt_parts(targets: &[Target], tile_w: u32, tile_h: u32) -> Vec<Vec<u8>> {
    let mut out = vec![];
    let mut base = 0;
    for (n, t) in targets.iter().enumerate() {
        let rt = rt71::rt_format71(t.gl_internal).unwrap();
        let c = rt71::clear_words71(&rt, t.clear).unwrap();
        let stride = rt71::rt_stride(tile_w, rt.bpp);
        let n = n as u32;
        out.push(rt71::packet_bytes(rt71::part1_payload(n, &rt, c.low, stride, base).unwrap()).to_vec());
        if rt.bpp >= rt71::BPP_64 {
            out.push(rt71::packet_bytes(rt71::part2_payload(n, c.mid).unwrap()).to_vec());
        }
        if rt.bpp >= rt71::BPP_128 {
            out.push(rt71::packet_bytes(rt71::part3_payload(n, c.top).unwrap()).to_vec());
        }
        base += rt71::rt_base_step(tile_h, stride);
    }
    out
}

fn assemble(fr: &frame71::Frame71, targets: &[Target]) -> Assembled {
    use frame71::Op71;
    let plan = frame71::plan(fr).unwrap();
    let mut binner = vec![frame71::pack_number_of_layers(1).unwrap().to_vec(), plan.binning.pack().unwrap().to_vec()];
    binner.push(Op71::ClearVcdCache.pack().unwrap());
    let mut render = vec![plan.common.pack().unwrap().to_vec()];
    render.extend(rt_parts(targets, fr.tile_width, fr.tile_height));
    render.push(plan.zs_clear.pack().unwrap().to_vec());
    render.push(plan.initial_block.pack().unwrap().to_vec());
    render.push(frame71::pack_tile_list_base(0x10000, 0).unwrap().to_vec());
    render.push(plan.supertile_cfg.pack().unwrap().to_vec());
    for op in &plan.initial_tiles {
        render.push(op.pack().unwrap());
    }
    render.push(Op71::ClearVcdCache.pack().unwrap());
    render.push(Op71::GenericTileList { start: 0x20000, end: 0x2001b }.pack().unwrap());
    for op in &plan.supertiles {
        render.push(op.pack().unwrap());
    }
    render.push(Op71::EndRender.pack().unwrap());
    let mut tile = vec![Op71::ImplicitTileCoords.pack().unwrap()];
    for op in &plan.list_loads {
        tile.push(op.pack().unwrap());
    }
    tile.push(Op71::EndLoads.pack().unwrap());
    tile.push(Op71::BranchImplicitTile { set: 0 }.pack().unwrap());
    for op in &plan.list_stores {
        tile.push(op.pack().unwrap());
    }
    tile.push(Op71::ReturnFromSubList.pack().unwrap());
    Assembled { binner, render, tile }
}

/// A colour-only frame: each target stored (and loaded when `load`).
fn color_frame(w: u32, h: u32, tile_w: u32, tile_h: u32, targets: &[Target]) -> frame71::Frame71 {
    let mut fr = frame71::Frame71::new(w, h, tile_w, tile_h);
    fr.render_targets = targets.len() as u32;
    for (i, t) in targets.iter().enumerate() {
        fr.rt[i] = Some(frame71::Attach71 { surface: t.surface, load: t.load, end: frame71::EndOp71::Store });
    }
    fr
}

fn rgba8_target(color: [f32; 4], addr: u32, load: bool) -> Target {
    Target { gl_internal: GL_RGBA8, clear: fclear(color), surface: rgba8_surface(64, addr), load }
}

/// The frame packets of the job `j` of a dump against the assembly. The
/// binner is compared on the packets frame71 writes only.
fn check_frame(what: &str, a: &Assembled, j: &ClifJob) {
    let binner: Vec<Pk> = between(&j.lists, "NUMBER_OF_LAYERS", "FLUSH")
        .into_iter()
        .filter(|p| matches!(p.name.as_str(), "NUMBER_OF_LAYERS" | "TILE_BINNING_MODE_CFG" | "CLEAR_VCD_CACHE"))
        .collect();
    assert_list(&format!("{what} binner list"), &a.binner, &binner);
    let render = between(&j.lists, "TILE_RENDERING_MODE_CFG_COMMON", "END_RENDER");
    assert_list(&format!("{what} render list"), &a.render, &render);
    let covered = |n: &str| {
        matches!(
            n,
            "IMPLICIT_TILE_COORDS"
                | "LOAD"
                | "END_LOADS"
                | "BRANCH_IMPLICIT_TILE"
                | "STORE"
                | "CLEAR_RT"
                | "END_TILE"
                | "RETURN"
        )
    };
    let tile: Vec<Pk> =
        between(&j.lists, "IMPLICIT_TILE_COORDS", "RETURN").into_iter().filter(|p| covered(&p.name)).collect();
    assert_list(&format!("{what} tile list"), &a.tile, &tile);
}

/// The dump's state packets at each draw: the last one of each kind seen
/// before the draw's packet.
fn states_at_draws(j: &ClifJob) -> Vec<BTreeMap<String, Pk>> {
    let kinds: Vec<&str> = draw71::ALL_SPECS.iter().map(|s| s.clif_name).collect();
    let mut last: BTreeMap<String, Pk> = BTreeMap::new();
    let mut out = vec![];
    for p in between(&j.lists, "NUMBER_OF_LAYERS", "FLUSH") {
        if kinds.contains(&p.name.as_str()) {
            last.insert(p.name.clone(), p);
        } else if p.name == "VERTEX_ARRAY_PRIMS" {
            out.push(last.clone());
        }
    }
    out
}

/// The module's state packets of one draw as bytes, by kind.
fn state_bytes(d: &draw71::DrawState) -> BTreeMap<&'static str, Vec<u8>> {
    let s = draw71::translate(d).unwrap();
    let mut m: BTreeMap<&'static str, Vec<u8>> = BTreeMap::new();
    m.insert("CLIP", s.clip.pack().to_vec());
    m.insert("CFG_BITS", s.cfg.pack().to_vec());
    m.insert("CLIPPER_XY", s.clipper_xy.pack().to_vec());
    m.insert("CLIPPER_Z", s.clipper_z.pack().to_vec());
    m.insert("CLIPZ", s.clipper_z_min_max.pack().to_vec());
    m.insert("VIEWPORT_OFFSET", s.viewport_offset.pack().to_vec());
    m.insert("BLEND_ENABLES", s.blend_enables.pack().to_vec());
    m.insert("COLOR_WMASKS", s.color_write_masks.pack().to_vec());
    if let Some(o) = s.depth_offset {
        m.insert("DEPTH_OFFSET", o.pack().to_vec());
    }
    for (i, b) in s.blend_cfg.iter().take(s.blend_cfg_count).enumerate() {
        m.insert(["BLEND_CFG", "BLEND_CFG#1", "BLEND_CFG#2", "BLEND_CFG#3"][i.min(3)], b.unwrap().pack().to_vec());
    }
    for (i, c) in s.stencil.iter().take(s.stencil_count).enumerate() {
        m.insert(["STENCIL_CFG", "STENCIL_CFG#1"][i], c.unwrap().pack().to_vec());
    }
    m
}

fn check_draw(what: &str, d: &draw71::DrawState, seen: &BTreeMap<String, Pk>) {
    let mine = state_bytes(d);
    for (kind, theirs) in seen {
        let b = mine.get(kind.as_str()).unwrap_or_else(|| panic!("{what}: the dump has a {kind}, the module none"));
        assert_same(&format!("{what} state"), &decode(b), theirs);
    }
    for kind in mine.keys() {
        assert!(seen.contains_key(*kind), "{what}: the module has a {kind}, the dump none");
    }
}

/// The attribute records of a draw, from a plain description.
fn check_attrs(what: &str, attrs: &[attr71::Attr], cs: &[u8], vs: &[u8], group: &[Pk]) {
    let mut c = [0u8; attr71::MAX_ATTRS];
    let mut v = [0u8; attr71::MAX_ATTRS];
    c[..cs.len()].copy_from_slice(cs);
    v[..vs.len()].copy_from_slice(vs);
    let p = attr71::plan(attrs, &c, &v, false, 0x1000).unwrap();
    assert_eq!(p.count, group.len(), "{what}: number of attribute arrays");
    for (i, want) in group.iter().enumerate() {
        assert_same(&format!("{what} attr {i}"), &decode_attr(&attr71::pack(&p.records[i])), want);
    }
}

fn float_attr(components: u32, stride: u64, addr: u32) -> attr71::Attr {
    attr71::Attr {
        enabled: true,
        components,
        gl_type: attr71::GL_FLOAT,
        stride,
        buffer_addr: addr,
        ..attr71::Attr::disabled()
    }
}

// ---- the dump in file order ------------------------------------------------

/// The segments of `lists` from each `start` packet to the next `end`.
fn segments(lists: &[Pk], start: &str, end: &str) -> Vec<Vec<Pk>> {
    let mut out = vec![];
    let mut cur: Option<Vec<Pk>> = None;
    for p in lists {
        if cur.is_none() && p.name == start {
            cur = Some(vec![]);
        }
        if let Some(c) = cur.as_mut() {
            c.push(p.clone());
            if p.name == end {
                out.push(cur.take().unwrap());
            }
        }
    }
    out
}

/// The GL jobs of a dump: the i-th binner list, render list and tile list
/// together, and the attribute groups of the job's `GL_SHADER` packets.
fn gl_jobs(file: &str) -> Vec<ClifJob> {
    let all = parse_jobs(file);
    let lists: Vec<Pk> = all.iter().flat_map(|j| j.lists.clone()).collect();
    let mut groups: std::collections::VecDeque<Vec<Pk>> = all.into_iter().flat_map(|j| j.attr_groups).collect();
    let binners = segments(&lists, "NUMBER_OF_LAYERS", "FLUSH");
    let renders = segments(&lists, "TILE_RENDERING_MODE_CFG_COMMON", "END_RENDER");
    let tiles = segments(&lists, "IMPLICIT_TILE_COORDS", "RETURN");
    assert_eq!(binners.len(), renders.len(), "{file}: binner and render lists");
    assert_eq!(binners.len(), tiles.len(), "{file}: binner and tile lists");
    let mut jobs = vec![];
    for ((b, r), t) in binners.into_iter().zip(renders).zip(tiles) {
        let n = b.iter().filter(|p| p.name == "GL_SHADER").count();
        let attr_groups = (0..n).map(|_| groups.pop_front().expect("an attribute group per GL_SHADER")).collect();
        let mut l = b;
        l.extend(r);
        l.extend(t);
        jobs.push(ClifJob { lists: l, attr_groups });
    }
    assert!(groups.is_empty(), "{file}: attribute groups left over");
    jobs
}

/// The 32-bit records of the dump in file order: `(label, value text)` per
/// printed line; an address's value is the hex of its trailing comment.
fn parse_records(file: &str) -> Vec<Vec<(String, String)>> {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let mut out: Vec<Vec<(String, String)>> = vec![];
    let mut on = false;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MESA: info:") else { continue };
        let tt = rest.trim();
        if let Some(f) = tt.strip_prefix("@format ") {
            on = f.split_whitespace().next() == Some("shadrec_gl_main");
            if on {
                out.push(vec![]);
            }
            continue;
        }
        if tt.starts_with('@') {
            on = false;
            continue;
        }
        if !on {
            continue;
        }
        let Some(body) = tt.strip_prefix("/*") else { continue };
        let Some(at) = body.find(": */") else { continue };
        let label = body[..at].trim().to_string();
        let value = body[at + 4..].trim().to_string();
        out.last_mut().unwrap().push((label, value));
    }
    out
}

fn rec_value(rec: &[(String, String)], label: &str) -> u32 {
    let (_, v) = rec.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("the dump's record has no {label:?}"));
    if v.starts_with('[') {
        let hex = v.split("/* 0x").nth(1).and_then(|s| s.split(' ').next()).expect("an address comment");
        u32::from_str_radix(hex, 16).unwrap()
    } else {
        v.split_whitespace().next().unwrap().parse().unwrap()
    }
}

#[derive(Clone, Copy)]
enum Pk1 {
    Bool,
    Uint,
    /// printed value is the logical one; the field holds it minus one
    MinusOne,
    /// 29-bit code address: address bits 3..31
    Code,
    Addr32,
}

/// `GL Shader State Record` (the plain 7.1 layout, `v3d_packet.xml:1256-1311`)
/// the dump prints with: label, start bit, size, kind.
const PLAIN: [(&str, u32, u32, Pk1); 39] = [
    ("Point size in shaded vertex data", 0, 1, Pk1::Bool),
    ("Enable clipping", 1, 1, Pk1::Bool),
    ("Vertex ID read by coordinate shader", 2, 1, Pk1::Bool),
    ("Instance ID read by coordinate shader", 3, 1, Pk1::Bool),
    ("Base Instance ID read by coordinate shader", 4, 1, Pk1::Bool),
    ("Vertex ID read by vertex shader", 5, 1, Pk1::Bool),
    ("Instance ID read by vertex shader", 6, 1, Pk1::Bool),
    ("Base Instance ID read by vertex shader", 7, 1, Pk1::Bool),
    ("Fragment shader does Z writes", 8, 1, Pk1::Bool),
    ("Turn off early-z test", 9, 1, Pk1::Bool),
    ("Fragment shader uses real pixel centre W in addition to centroid W2", 12, 1, Pk1::Bool),
    ("Enable Sample Rate Shading", 13, 1, Pk1::Bool),
    ("Any shader reads hardware-written Primitive ID", 14, 1, Pk1::Bool),
    ("Insert Primitive ID as first varying to fragment shader", 15, 1, Pk1::Bool),
    ("Turn off scoreboard", 16, 1, Pk1::Bool),
    ("Do scoreboard wait on first thread switch", 17, 1, Pk1::Bool),
    ("Disable implicit point/line varyings", 18, 1, Pk1::Bool),
    ("No prim pack", 19, 1, Pk1::Bool),
    ("Never defer FEP depth writes", 20, 1, Pk1::Bool),
    ("Number of varyings in Fragment Shader", 24, 8, Pk1::Uint),
    ("Coordinate Shader output VPM segment size", 32, 4, Pk1::Uint),
    ("Min Coord Shader output segments required in play in addition to VCM cache size", 36, 4, Pk1::Uint),
    ("Coordinate Shader input VPM segment size", 40, 4, Pk1::Uint),
    ("Min Coord Shader input segments required in play", 44, 4, Pk1::MinusOne),
    ("Vertex Shader output VPM segment size", 48, 4, Pk1::Uint),
    ("Min Vertex Shader output segments required in play in addition to VCM cache size", 52, 4, Pk1::Uint),
    ("Vertex Shader input VPM segment size", 56, 4, Pk1::Uint),
    ("Min Vertex Shader input segments required in play", 60, 4, Pk1::MinusOne),
    ("Fragment Shader Code Address", 67, 29, Pk1::Code),
    ("Fragment Shader 4-way threadable", 64, 1, Pk1::Bool),
    ("Fragment Shader start in final thread section", 65, 1, Pk1::Bool),
    ("Fragment Shader Propagate NaNs", 66, 1, Pk1::Bool),
    ("Fragment Shader Uniforms Address", 96, 32, Pk1::Addr32),
    ("Vertex Shader Code Address", 131, 29, Pk1::Code),
    ("Vertex Shader 4-way threadable", 128, 1, Pk1::Bool),
    ("Vertex Shader start in final thread section", 129, 1, Pk1::Bool),
    ("Vertex Shader Propagate NaNs", 130, 1, Pk1::Bool),
    ("Vertex Shader Uniforms Address", 160, 32, Pk1::Addr32),
    ("Coordinate Shader Code Address", 195, 29, Pk1::Code),
];

/// The rest of the plain layout (kept apart for the array length above).
const PLAIN_CS: [(&str, u32, u32, Pk1); 4] = [
    ("Coordinate Shader 4-way threadable", 192, 1, Pk1::Bool),
    ("Coordinate Shader start in final thread section", 193, 1, Pk1::Bool),
    ("Coordinate Shader Propagate NaNs", 194, 1, Pk1::Bool),
    ("Coordinate Shader Uniforms Address", 224, 32, Pk1::Addr32),
];

fn put_bits(out: &mut [u8; 32], start: u32, size: u32, v: u32) {
    for k in 0..size {
        if v >> k & 1 != 0 {
            let at = start + k;
            out[(at / 8) as usize] |= 1 << (at % 8);
        }
    }
}

/// The 32 bytes Mesa wrote, from the dump's printed values at the plain
/// layout's positions. Bits the plain layout has no field for (10, 11, 14,
/// 21, 22, 23 of the D0 layout) come out 0.
fn reference_bytes(rec: &[(String, String)]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (label, start, size, kind) in PLAIN.iter().chain(PLAIN_CS.iter()) {
        let v = rec_value(rec, label);
        let packed = match kind {
            Pk1::Bool | Pk1::Uint | Pk1::Addr32 => v,
            Pk1::MinusOne => v - 1,
            Pk1::Code => v >> 3,
        };
        put_bits(&mut out, *start, *size, packed);
    }
    out
}

/// The 47 values of record71's `FIELDS`, read from bytes at its positions.
fn d0_values(b: &[u8; 32]) -> [u32; 47] {
    let mut v = [0u32; 47];
    for (i, f) in record71::FIELDS.iter().enumerate() {
        let raw = bits(b, f.start, f.size) as u32;
        v[i] = match f.kind {
            record71::Kind::Bool | record71::Kind::Uint => raw,
            record71::Kind::UintMinusOne => raw + 1,
            record71::Kind::Address if f.size == 29 => raw << 3,
            record71::Kind::Address => raw,
        };
    }
    v
}

fn program(code: u32, uniforms: u32, threadable: u32, fin: u32, nan: u32) -> record71::ProgramDesc {
    record71::ProgramDesc {
        code,
        uniforms,
        threads: if threadable != 0 { 4 } else { 1 },
        single_seg: fin != 0,
        propagate_nans: nan != 0,
    }
}

/// What the compiler and the GL state would hand `record71` for the draw
/// whose D0 record has these values (`ix` positions).
fn draw_desc(v: &[u32; 47]) -> record71::DrawDesc {
    use record71::ix::*;
    let vertex = |vid, iid, biid, size_out, min_out, size_in, min_in, p| record71::VertexDesc {
        prog: p,
        vpm_input_size: v[size_in],
        vpm_output_size: v[size_out],
        min_input_segments: v[min_in],
        min_output_segments: v[min_out],
        uses_vid: v[vid] != 0,
        uses_iid: v[iid] != 0,
        uses_base_instance: v[biid] != 0,
        uses_base_vertex: false,
        uses_draw_index: false,
    };
    record71::DrawDesc {
        fs: record71::FragmentDesc {
            prog: program(v[FS_CODE], v[FS_UNIFORMS], v[FS_THREADABLE], v[FS_FINAL], v[FS_NAN]),
            writes_z: v[Z_WRITES] != 0,
            disable_ez: v[TURN_OFF_EZ] != 0,
            num_varyings: v[VARYINGS],
            uses_center_w: v[CENTER_W] != 0,
            uses_pid: v[ANY_PID] != 0,
            lock_scoreboard_on_first_thrsw: v[SCOREBOARD_WAIT] != 0,
            // the dump has no bit 21; the README's value is 1 (disabled)
            uses_implicit_point_line_varyings: false,
            sample_rate_shading: v[SAMPLE_RATE] != 0,
        },
        vs: vertex(
            VS_VID,
            VS_IID,
            VS_BIID,
            VS_OUT_SIZE,
            VS_OUT_MIN,
            VS_IN_SIZE,
            VS_IN_MIN,
            program(v[VS_CODE], v[VS_UNIFORMS], v[VS_THREADABLE], v[VS_FINAL], v[VS_NAN]),
        ),
        cs: vertex(
            CS_VID,
            CS_IID,
            CS_BIID,
            CS_OUT_SIZE,
            CS_OUT_MIN,
            CS_IN_SIZE,
            CS_IN_MIN,
            program(v[CS_CODE], v[CS_UNIFORMS], v[CS_THREADABLE], v[CS_FINAL], v[CS_NAN]),
        ),
        point_size: v[POINT_SIZE] != 0,
    }
}

// ---- the jobs of the five shapes ---------------------------------------------

const GL_RGB8: u32 = 0x8051;

fn zs_surface(format: u8) -> frame71::Surface71 {
    frame71::Surface71 { format, memory_format: 4, height_in_ub_or_stride: uif_height(64), ..Default::default() }
}

/// The GL window of gl_test: an RGBA8 target and a D24S8 buffer nothing is
/// stored from (compose71's gl_scissor jobs): `Early Depth/Stencil Clear`
/// set, depth_24; the job's `Early-Z disable` as given.
fn window_job(clear: [f32; 4], load: bool, ez_disable: bool) -> (frame71::Frame71, Vec<Target>) {
    let t = vec![rgba8_target(clear, 0x3000, load)];
    let mut fr = color_frame(64, 64, 64, 64, &t);
    fr.depth_kind = frame71::DepthKind71::D24;
    fr.zs = Some(frame71::ZsAttach71 {
        surface: frame71::Surface71::default(),
        load_z: false,
        load_s: false,
        store_z: false,
        store_s: false,
    });
    fr.early_ds_clear = true;
    fr.early_z_disable = ez_disable;
    (fr, t)
}

/// A framebuffer-object job: colour (`gl_internal`) and a depth and/or
/// stencil buffer of `kind` with the Output Image Format `zs_format`.
#[allow(clippy::too_many_arguments)]
fn fbo_job(
    gl_internal: u32,
    clear: [f32; 4],
    load: bool,
    kind: frame71::DepthKind71,
    zs: Option<(u8, bool, bool, bool, bool)>,
    z_clear: f32,
    ez_disable: bool,
) -> (frame71::Frame71, Vec<Target>) {
    let t = vec![Target { gl_internal, clear: fclear(clear), surface: rgba8_surface(64, 0x3000), load }];
    let mut fr = color_frame(64, 64, 64, 64, &t);
    fr.depth_kind = kind;
    fr.zs = zs.map(|(fmt, lz, ls, sz, ss)| frame71::ZsAttach71 {
        surface: zs_surface(fmt),
        load_z: lz,
        load_s: ls,
        store_z: sz,
        store_s: ss,
    });
    fr.z_clear_bits = z_clear.to_bits();
    fr.early_z_disable = ez_disable;
    (fr, t)
}

/// A job that clears nothing (Mesa's `clear_tlb` is 0): no `Clear Render
/// Targets` ends its tile list (gl_writemask.clif job 4, gl_fbods.clif job 9).
fn no_clear(mut j: (frame71::Frame71, Vec<Target>)) -> (frame71::Frame71, Vec<Target>) {
    j.0.clear_tile_buffers = false;
    j
}

fn gl() -> draw71::DrawState {
    draw71::DrawState::gl_default(64, 64, false, 1)
}

fn blit(mask: u8, stencil: Option<(u32, u32)>) -> draw71::DrawState {
    draw71::DrawState::blitter_clear(64, 64, false, 1, mask, stencil)
}

fn depth_less() -> draw71::DrawState {
    let mut d = gl();
    d.depth_test = true;
    d
}

fn stencil_equal(reference: u32) -> draw71::DrawState {
    let mut d = gl();
    d.stencil_test = true;
    d.stencil_front = draw71::StencilFace {
        func: draw71::Compare::Equal,
        sfail: draw71::StencilOp::Keep,
        zfail: draw71::StencilOp::Keep,
        zpass: draw71::StencilOp::Keep,
        reference,
        value_mask: 0xff,
        write_mask: 0xff,
    };
    d
}

/// One job against the dump's job `i`: the frame packets, each draw's state
/// packets and each draw's attribute records.
fn check_job(what: &str, j: &ClifJob, (fr, t): (frame71::Frame71, Vec<Target>), draws: &[draw71::DrawState]) {
    check_frame(what, &assemble(&fr, &t), j);
    let seen = states_at_draws(j);
    assert_eq!(seen.len(), draws.len(), "{what}: number of draws");
    for (n, (d, s)) in draws.iter().zip(&seen).enumerate() {
        check_draw(&format!("{what} draw {}", n + 1), d, s);
    }
    // gl_test's 40-byte vertex (three attributes), or the blitter's vec3
    for (n, g) in j.attr_groups.iter().enumerate() {
        let w = format!("{what} record {}", n + 1);
        match g.len() {
            1 => check_attrs(&w, &[float_attr(3, 36, 0x15000)], &[4], &[4], g),
            3 => {
                let a = [float_attr(4, 40, 0xe000), float_attr(4, 40, 0xe010), float_attr(2, 40, 0xe020)];
                check_attrs(&w, &a, &[4, 0, 0], &[4, 4, 2], g)
            }
            n => panic!("{w}: {n} attribute records"),
        }
    }
}

#[test]
fn gl_viewport_case_66() {
    let jobs = gl_jobs("gl_viewport");
    assert_eq!(jobs.len(), 3);
    // a white clear; a blue clear at a quarter viewport (a clear is not
    // bounded by glViewport: the tile buffer clears it all); then a green
    // clear scissored to the bottom-left quarter, a quad of the whole target
    // (gl_viewport.clif: Early-Z disable 1, 1, 0)
    check_job("gl_viewport job 1", &jobs[0], window_job([1.0, 1.0, 1.0, 1.0], false, true), &[]);
    check_job("gl_viewport job 2", &jobs[1], window_job([0.0, 0.0, 1.0, 1.0], false, true), &[]);
    check_job("gl_viewport job 3", &jobs[2], window_job([0.0, 0.0, 0.0, 0.0], true, false), &[blit(0xf, None)]);
}

#[test]
fn gl_colormask_case_102() {
    let jobs = gl_jobs("gl_colormask");
    assert_eq!(jobs.len(), 6);
    let black = [0.0, 0.0, 0.0, 1.0];
    let none = [0.0, 0.0, 0.0, 0.0];
    // 1. a black clear; the R+A masked clear is a quad (mask 0b1001)
    check_job("gl_colormask job 1", &jobs[0], window_job(black, false, true), &[]);
    check_job("gl_colormask job 2", &jobs[1], window_job(none, true, false), &[blit(0b1001, None)]);
    // 2. black again, then the scissored G-only clear (mask 0b0010)
    check_job("gl_colormask job 3", &jobs[2], window_job(black, false, true), &[]);
    check_job("gl_colormask job 4", &jobs[3], window_job(none, true, false), &[blit(0b0010, None)]);
    // 3. black, then the B-only clear as a quad and the yellow quad, every
    // channel on
    check_job("gl_colormask job 5", &jobs[4], window_job(black, false, true), &[]);
    check_job("gl_colormask job 6", &jobs[5], window_job(none, true, false), &[blit(0b0100, None), gl()]);
}

#[test]
fn gl_writemask_case_59() {
    let jobs = gl_jobs("gl_writemask");
    assert_eq!(jobs.len(), 4);
    use frame71::DepthKind71::{StencilOnly, D24};
    let blue = [0.0, 0.0, 1.0, 1.0];
    // 1. a blue clear, the masked-off red clear changes nothing
    check_job("gl_writemask job 1", &jobs[0], window_job(blue, false, true), &[]);
    // 2. depth: clear colour and depth 1.0, two depth-tested quads (the
    // masked depth clear in between draws nothing); depth is stored
    check_job(
        "gl_writemask job 2",
        &jobs[1],
        fbo_job(GL_RGBA8, blue, false, D24, Some((43, false, false, true, false)), 1.0, false),
        &[depth_less(), depth_less()],
    );
    // 3. stencil: the 0x0f-masked clear is a quad with a stencil write and no colour, then
    // the draw testing EQUAL 0xff; the stencil is stored for the next job
    check_job(
        "gl_writemask job 3",
        &jobs[2],
        fbo_job(GL_RGBA8, blue, false, StencilOnly, Some((43, false, false, false, true)), 0.0, false),
        &[blit(0, Some((0xff, 0x0f))), stencil_equal(0xff)],
    );
    // 4. the draw testing EQUAL 0x0f reads the stencil of job 3
    check_job(
        "gl_writemask job 4",
        &jobs[3],
        no_clear(fbo_job(GL_RGBA8, [0.0; 4], true, StencilOnly, Some((43, false, true, false, false)), 0.0, false)),
        &[stencil_equal(0x0f)],
    );
}

#[test]
fn gl_fbods_case_67() {
    let jobs = gl_jobs("gl_fbods");
    assert_eq!(jobs.len(), 9);
    use frame71::DepthKind71::{StencilOnly, D16};
    let blue = [0.0, 0.0, 1.0, 1.0];
    let none = frame71::DepthKind71::None;
    // 1. colour only, the depth test on (no depth buffer: no test)
    check_job("gl_fbods job 1", &jobs[0], fbo_job(GL_RGBA8, blue, false, none, None, 0.0, true), &[gl(), gl()]);
    // 2. the stencil test on, no stencil buffer
    check_job("gl_fbods job 2", &jobs[1], fbo_job(GL_RGBA8, blue, false, none, None, 0.0, true), &[gl()]);
    // 3. a GL_RGB target cleared (1, 0, 0, 0.25)
    check_job("gl_fbods job 3", &jobs[2], fbo_job(GL_RGB8, [1.0, 0.0, 0.0, 0.25], false, none, None, 0.0, true), &[]);
    // 4, 5. two framebuffers over one colour texture, a D16 buffer each
    check_job(
        "gl_fbods job 4",
        &jobs[3],
        fbo_job(GL_RGBA8, blue, false, D16, Some((42, false, false, true, false)), 1.0, false),
        &[depth_less()],
    );
    check_job(
        "gl_fbods job 5",
        &jobs[4],
        fbo_job(GL_RGBA8, [0.0; 4], true, D16, Some((42, false, false, true, false)), 0.0, false),
        &[depth_less()],
    );
    // 6. a STENCIL_INDEX8 target: the scissored stencil clear (a quad
    // writing 1), then the draw testing EQUAL 0
    check_job(
        "gl_fbods job 6",
        &jobs[5],
        fbo_job(GL_RGBA8, blue, false, StencilOnly, Some((43, false, false, false, true)), 0.0, false),
        &[blit(0, Some((1, 0xff))), stencil_equal(0)],
    );
    // 7. a draw ahead, then the scissored clear and the stencil draw, one job
    check_job(
        "gl_fbods job 7",
        &jobs[6],
        fbo_job(GL_RGBA8, blue, false, StencilOnly, Some((43, false, false, false, true)), 0.0, false),
        &[gl(), blit(0, Some((1, 0xff))), stencil_equal(0)],
    );
    // 8. the same across a glFinish: the first job ends after the clear
    check_job(
        "gl_fbods job 8",
        &jobs[7],
        fbo_job(GL_RGBA8, blue, false, StencilOnly, Some((43, false, false, false, true)), 0.0, false),
        &[gl(), blit(0, Some((1, 0xff)))],
    );
    // 9. and the stencil draw loads colour and stencil back
    check_job(
        "gl_fbods job 9",
        &jobs[8],
        no_clear(fbo_job(GL_RGBA8, [0.0; 4], true, StencilOnly, Some((43, false, true, false, false)), 0.0, false)),
        &[stencil_equal(0)],
    );
}

// ---- limits71: the tile size and the target count -------------------------------

const NINE_SHAPES: [&str; 9] = [
    "clear",
    "triangle",
    "gl_scissor",
    "gl_mrt4",
    "gl_viewport",
    "gl_colormask",
    "gl_writemask",
    "gl_mrt8",
    "gl_fbods",
];

fn field_u(p: &Pk, label: &str) -> u32 {
    p.fields.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("{} has no {label}", p.name)).1.parse().unwrap()
}

#[test]
fn limits_choose_every_reference_tile_size() {
    use limits71::*;
    let mut seen = 0;
    for shape in NINE_SHAPES {
        let jobs = parse_jobs(shape);
        let lists: Vec<Pk> = jobs.iter().flat_map(|j| j.lists.clone()).collect();
        // every render list carries the targets, their bpp and the tile
        for r in segments(&lists, "TILE_RENDERING_MODE_CFG_COMMON", "END_RENDER") {
            let common = &r[0];
            let n = field_u(common, "Number of Render Targets");
            let tile = (8 << field_u(common, "Log2 Tile Width"), 8 << field_u(common, "Log2 Tile Height"));
            let parts: Vec<&Pk> =
                r.iter().filter(|p| p.name == "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1").collect();
            assert_eq!(parts.len() as u32, n, "{shape}: a Part1 per target");
            let max_bpp = parts.iter().map(|p| field_u(p, "Internal BPP")).max().unwrap();
            let bpp = [Bpp71::Bpp32, Bpp71::Bpp64, Bpp71::Bpp128][max_bpp as usize];
            assert!(n <= MAX_RENDER_TARGETS71, "{shape}: {n} targets");
            assert_eq!(
                choose_tile_size71(n, bpp, false, false),
                Some(tile),
                "{shape}: tile size of {n} target(s) of {bpp:?} (the dump's {tile:?})"
            );
            assert!(tile_size_valid71(tile.0 * tile.1, n * bpp.bytes(), 4), "{shape}: the dump's tile is valid");
            // a double-buffered frame takes the next smaller size, never more
            if let Some(small) = choose_tile_size71(n, bpp, false, true) {
                assert!(small.0 * small.1 < tile.0 * tile.1);
            }
            seen += 1;
        }
        // the binner's width and height are within the framebuffer limit
        for p in lists.iter().filter(|p| p.name == "TILE_BINNING_MODE_CFG") {
            assert!(field_u(p, "Width (in pixels)") <= MAX_FRAMEBUFFER_SIZE71);
            assert!(field_u(p, "Height (in pixels)") <= MAX_FRAMEBUFFER_SIZE71);
        }
    }
    assert!(seen == 1 + 1 + 2 + 1 + 3 + 6 + 4 + 1 + 9, "every shape's render lists were compared, saw {seen}");
}

#[test]
fn limits_agree_with_the_other_modules() {
    use limits71::*;
    assert_eq!(MAX_RENDER_TARGETS71, rt71::MAX_RENDER_TARGETS);
    assert_eq!(MAX_RENDER_TARGETS71, frame71::MAX_RTS);
    assert_eq!(MAX_RENDER_TARGETS71 as usize, draw71::MAX_TARGETS);
    assert_eq!(MAX_FRAMEBUFFER_SIZE71, rt71::MAX_SIZE);
    assert_eq!(MAX_FRAMEBUFFER_SIZE71, frame71::MAX_SIZE);
    assert_eq!(MAX_FRAMEBUFFER_SIZE71, draw71::MAX_SIZE);
    // nine targets: limits71 has no tile size, rt71 and frame71 refuse
    assert_eq!(choose_tile_size71(9, Bpp71::Bpp32, false, false), None);
    assert!(rt71::check_targets(9).is_err());
    // the eight-target job of gl_mrt8 is the one tile size frame71 takes
    let (w, h) = choose_tile_size71(8, Bpp71::Bpp32, false, false).unwrap();
    assert_eq!((w, h), (32, 32));
    let mut fr = frame71::Frame71::new(64, 64, w, h);
    fr.render_targets = 8;
    assert!(frame71::plan(&fr).is_ok());
}

#[test]
fn gl_mrt8_eight_targets_in_the_limits71_tile() {
    let jobs = gl_jobs("gl_mrt8");
    assert_eq!(jobs.len(), 1);
    let (tw, th) = limits71::choose_tile_size71(8, limits71::Bpp71::Bpp32, false, false).unwrap();
    let t: Vec<Target> = (0..8).map(|i| rgba8_target([0.0, 0.0, 1.0, 1.0], 0x10000 * (i + 1), false)).collect();
    let fr = color_frame(64, 64, tw, th, &t);
    let a = assemble(&fr, &t);
    check_frame("gl_mrt8", &a, &jobs[0]);
    let stores = a.tile.iter().filter(|b| decode(b).name == "STORE").count();
    assert_eq!(stores, 8, "one tile-buffer store per target");
}

// ---- record71: the shader record of every draw -----------------------------------

#[test]
fn the_shader_record_of_every_draw_of_the_nine_shapes() {
    let mut draws_seen = 0;
    for shape in NINE_SHAPES {
        let recs = parse_records(shape);
        let jobs = gl_jobs(shape);
        let shaders: usize = jobs.iter().map(|j| j.lists.iter().filter(|p| p.name == "GL_SHADER").count()).sum();
        let prims: usize = jobs.iter().map(|j| j.lists.iter().filter(|p| p.name == "VERTEX_ARRAY_PRIMS").count()).sum();
        assert_eq!(recs.len(), shaders, "{shape}: a record per GL_SHADER packet");
        // Mesa re-emits GL_SHADER when the program or its state changed: a
        // draw is covered by the last GL_SHADER before it, in its job
        assert!(shaders <= prims || prims == 0, "{shape}: {shaders} GL_SHADER for {prims} draws");
        for j in &jobs {
            let mut have = false;
            for p in &j.lists {
                match p.name.as_str() {
                    "GL_SHADER" => have = true,
                    "VERTEX_ARRAY_PRIMS" => assert!(have, "{shape}: a draw before its GL_SHADER"),
                    _ => {}
                }
            }
        }
        draws_seen += prims;
        for (n, rec) in recs.iter().enumerate() {
            let what = format!("{shape} record {}", n + 1);
            let want = reference_bytes(rec);
            let desc = draw_desc(&d0_values(&want));
            let got =
                record71::pack_draw(record71::BCM2712_D0, &desc).unwrap_or_else(|r| panic!("{what}: {}", r.message()));
            // byte 2 bit 5 is `Disable implicit point/line varyings` (D0
            // bit 21), 1 for these shaders; the plain layout the dump
            // prints has no field there
            let mut expect = want;
            expect[2] |= 0x20;
            assert_eq!(got, expect, "{what}: the 32 bytes");
            // and the values Mesa never sets
            let r = record71::describe(record71::BCM2712_D0, &desc).unwrap();
            for i in [
                record71::ix::CS_BASEVERTEX,
                record71::ix::CS_DRAWINDEX,
                record71::ix::VS_BASEVERTEX,
                record71::ix::VS_DRAWINDEX,
                record71::ix::NEVER_DEFER_FEP,
                record71::ix::NO_PRIM_PACK,
                record71::ix::TURN_OFF_SCOREBOARD,
            ] {
                assert_eq!(r.value(i), 0, "{what}: {}", record71::FIELDS[i].name);
            }
            assert_eq!(r.value(record71::ix::ENABLE_CLIPPING), 1, "{what}: Enable clipping");
            assert_eq!(r.value(record71::ix::DISABLE_IMPLICIT), 1, "{what}: Disable implicit point/line varyings");
            // the shaders of these shapes: no z write, early-z allowed
            assert_eq!(rec_value(rec, "Number of varyings in Fragment Shader"), r.value(record71::ix::VARYINGS));
        }
    }
    assert!(draws_seen >= 25, "the nine shapes have draws, saw {draws_seen}");
}

#[test]
fn the_shader_record_refusals_follow_the_revision() {
    let rec = &parse_records("triangle")[0];
    let desc = draw_desc(&d0_values(&reference_bytes(rec)));
    for rev in [record71::Revision { ver: 42, rev: 0 }, record71::Revision { ver: 71, rev: 9 }] {
        assert_eq!(
            record71::describe(rev, &desc).unwrap_err(),
            record71::Refusal::NoDrawIndexRecord { ver: rev.ver, rev: rev.rev }
        );
    }
    // the Pi 5's identity (compose71's board words) is the revision record71 serves
    let id = ident71::check_words(0x0008_1117, 0x0002_0a10).expect("the Pi 5's identity is accepted");
    assert_eq!(id, ident71::decode(0x0008_1117, 0x0002_0a10));
    assert!(record71::has_draw_index_record(record71::BCM2712_D0));
}
