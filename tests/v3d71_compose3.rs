// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged pure V3D 7.1 modules put together a third time (compose71c):
//! the texture references of tools/v3d/pi5/ (P10), as `tests/v3d71_compose.rs`
//! and `tests/v3d71_compose2.rs` did for the P9 shapes.
//!
//! * `gl_tex2d` (2 jobs), `gl_texfmt` (2), `gl_cube` (12) and `gl_mipgen`'s one
//!   render job (its TFU job is not in a CLIF dump): the frame packets of
//!   every job (`frame71`, `rt71`), the state packets of every draw
//!   (`draw71`), the draw packet (`prim71`, `VERTEX_ARRAY_PRIMS`), the
//!   attribute records (`attr71`), the draw-index shader-state record
//!   (`record71`) and the tile size (`limits71`);
//! * the texture shader state and the sampler state of every job
//!   (`texture71` over `format71` and `texstate71`; `sampler71`), read from
//!   the words the dump holds in its uniform stream's TMU Config pair (the
//!   reading rule of tools/v3d/pi5/src/texstate71.py), against the GL calls
//!   of tools/v3d/pi5/README.md "The GL calls".
//!
//! The helpers (`parse_jobs`, `decode`, `decode_attr`, `assemble`, ...) are
//! copied from compose2 (the existing tests are not edited).
//!
//! Known gaps: packets the dumps carry that no merged module produces. They
//! are pinned in `KNOWN_GAPS`; the test `the_gaps_are_the_pinned_ones`
//! computes the set from the dumps and fails when it differs, so a module
//! that starts producing one of them (or a new reference packet) changes the
//! list on purpose. The two texture words of each sampled texture (`TMU
//! Config Parameter 0/1`) are in `KNOWN_WORD_GAPS`: `uniform71` refuses them
//! by name and texuni71 does not exist yet, so only their consistency with
//! the records they point at is checked.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_compose3
//! ```
#![allow(dead_code)]
#[path = "../src/v3d/attr71.rs"]
#[allow(dead_code)]
mod attr71;
#[path = "../src/v3d/draw71.rs"]
#[allow(dead_code)]
mod draw71;
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/frame71.rs"]
#[allow(dead_code)]
mod frame71;
#[path = "../src/v3d/ident71.rs"]
#[allow(dead_code)]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
#[allow(dead_code)]
mod limits71;
#[path = "../src/v3d/prim71.rs"]
mod prim71;
#[path = "../src/v3d/record71.rs"]
#[allow(dead_code)]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(dead_code)]
mod rt71;
#[path = "../src/v3d/sampler71.rs"]
mod sampler71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;

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

// ---- the texture records: the dump's TMU Config pair ------------------------------

struct TexRec {
    p0: u32,
    p1: u32,
    tex: [u32; 6],
    smp: [u32; 6],
}

struct TexDump {
    /// per job: the (P0, P1) pairs the fragment shaders' uniform streams hold
    jobs: Vec<Vec<TexRec>>,
    /// every `resource_0x...` buffer: start address and dumped extent
    resources: Vec<(u32, u32)>,
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// texstate71.py's reading rule: the words of each fragment shader's uniform
/// stream, each pair (P0, P1) whose P0 is a 32-byte aligned address and P1 an
/// 8-byte aligned one, both with six dumped words behind them.
fn parse_texdump(file: &str) -> TexDump {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let mut jobs = Vec::new();
    let mut mem: BTreeMap<u32, u32> = BTreeMap::new();
    let mut fs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut extent: BTreeMap<String, u32> = BTreeMap::new();
    let mut addr: Option<u32> = None;
    let mut cur: Option<(String, u32)> = None;
    let finish = |mem: &BTreeMap<u32, u32>, fs: &[u32], unif: &[u32]| -> Vec<TexRec> {
        let mut recs = Vec::new();
        for &f in fs {
            let end = unif.iter().copied().filter(|&a| a > f).min();
            let mut stream = Vec::new();
            let mut a = f;
            while let Some(w) = mem.get(&a) {
                if end.is_some_and(|e| a >= e) {
                    break;
                }
                stream.push(*w);
                a += 4;
            }
            for p in stream.windows(2) {
                let (ta, sa) = (p[0] & !0xf, p[1] & !0x7);
                let have = |b: u32| (0..6).all(|k| mem.contains_key(&(b + 4 * k)));
                if ta % 32 != 0 || !have(ta) || !have(sa) {
                    continue;
                }
                recs.push(TexRec {
                    p0: p[0],
                    p1: p[1],
                    tex: std::array::from_fn(|k| mem[&(ta + 4 * k as u32)]),
                    smp: std::array::from_fn(|k| mem[&(sa + 4 * k as u32)]),
                });
            }
        }
        recs
    };
    for raw in text.lines() {
        let Some(line) = raw.strip_prefix("MESA: info: ") else { continue };
        if line == "@wait_render_all_cores" {
            jobs.push(finish(&mem, &fs, &unif));
            mem.clear();
            fs.clear();
            unif.clear();
            continue;
        }
        if let Some(r) = line.strip_prefix("@format binary /* [") {
            let (name, off) = r.split_once('+').unwrap();
            let off = hex(off.split(']').next().unwrap());
            let base = hex(name.rsplit('_').next().unwrap());
            addr = Some(base + off);
            cur = Some((name.to_string(), off));
            continue;
        }
        if let Some(r) = line.strip_prefix("@format blank ") {
            let r = &r[r.find('[').unwrap() + 1..r.find(']').unwrap()];
            let (name, range) = r.split_once('+').unwrap();
            let (lo, hi) = range.split_once("..").unwrap();
            let base = hex(name.rsplit('_').next().unwrap());
            let (lo, hi) = (hex(lo), hex(hi));
            for a in ((base + lo) & !3..=base + hi).step_by(4) {
                mem.entry(a).or_insert(0);
            }
            let e = extent.entry(name.to_string()).or_insert(0);
            *e = (*e).max(hi + 1);
            addr = None;
            cur = None;
            continue;
        }
        if line.starts_with('@') {
            addr = None;
            cur = None;
            continue;
        }
        let t = line.trim();
        if let Some(a) = addr.as_mut() {
            if !t.is_empty() && t.split(' ').all(|w| w.len() == 10 && w.starts_with("0x")) {
                let (name, off) = cur.as_mut().unwrap();
                for w in t.split(' ') {
                    mem.insert(*a, hex(w));
                    *a += 4;
                    *off += 4;
                }
                let e = extent.entry(name.clone()).or_insert(0);
                *e = (*e).max(*off);
                continue;
            }
        }
        for which in ["Fragment", "Vertex", "Coordinate"] {
            let key = format!("/* {which} Shader Uniforms Address: */ [");
            if let Some(i) = t.find(&key) {
                let r = &t[i + key.len()..];
                let v = &r[r.find("/* ").unwrap() + 3..];
                let a = hex(v.split(' ').next().unwrap());
                unif.push(a);
                if which == "Fragment" {
                    fs.push(a);
                }
            }
        }
    }
    let resources = extent
        .iter()
        .filter(|(n, _)| n.starts_with("resource_"))
        .map(|(n, e)| (hex(n.rsplit('_').next().unwrap()), *e))
        .collect();
    TexDump { jobs, resources }
}

/// (name, start, size) of every field of the 7.1 Texture Shader State, from
/// the vendored packet XML.
fn tex_fields() -> Vec<(String, u32, u32)> {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let open = "<struct name=\"Texture Shader State\" min_ver=\"71\">";
    let s = xml.find(open).expect("7.1 Texture Shader State");
    let block = &xml[s..s + xml[s..].find("</struct>").unwrap()];
    let attr = |tag: &str, a: &str| -> String {
        let k = format!(" {a}=\"");
        let j = tag.find(&k).unwrap_or_else(|| panic!("{a} in {tag}")) + k.len();
        tag[j..j + tag[j..].find('"').unwrap()].to_string()
    };
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = block[at..].find("<field ") {
        let t0 = at + i;
        let tag = &block[t0..t0 + block[t0..].find('>').unwrap()];
        out.push((attr(tag, "name"), attr(tag, "start").parse().unwrap(), attr(tag, "size").parse().unwrap()));
        at = t0 + tag.len();
    }
    out
}

fn wbits(words: &[u32; 6], start: u32, size: u32) -> u64 {
    let mut v = 0u64;
    for b in 0..size {
        let bit = start + b;
        v |= u64::from(words[(bit / 32) as usize] >> (bit % 32) & 1) << b;
    }
    v
}

fn wput(words: &mut [u32; 6], start: u32, size: u32, v: u64) {
    assert!(size == 64 || v < 1 << size, "value {v} in {size} bits");
    for b in 0..size {
        if v >> b & 1 == 1 {
            let bit = start + b;
            words[(bit / 32) as usize] |= 1 << (bit % 32);
        }
    }
}

/// Every field of the 7.1 record from the composed values; the C writer's
/// constants (chroma pointers, offsets 1) and the fields nothing sets are as
/// tests/v3d71_gl_tex.rs has them.
fn expected_tex_fields(t: &texture71::Tex71) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    let mut s = |k: &str, v: u64| {
        m.insert(k.to_string(), v);
    };
    s("Texture base pointer", u64::from(t.base));
    s("Transfer func", u64::from(t.transfer_func));
    s("Reverse", u64::from(t.reverse));
    s("R/B swap", u64::from(t.r_b_swap));
    s("Array Stride (64-byte aligned)", u64::from(t.array_stride_64));
    s("Image Width", u64::from(t.width));
    s("Image Height", u64::from(t.height));
    s("Image Depth", u64::from(t.depth));
    s("Texture type", u64::from(t.tex_type));
    s("Extended", u64::from(t.extended));
    s("Swizzle R", u64::from(t.swizzle[0]));
    s("Swizzle G", u64::from(t.swizzle[1]));
    s("Swizzle B", u64::from(t.swizzle[2]));
    s("Swizzle A", u64::from(t.swizzle[3]));
    s("Max Level", u64::from(t.max_level));
    s("Base Level", u64::from(t.base_level));
    s("Level 0 UB_PAD", u64::from(t.level_0_ub_pad));
    s("Level 0 XOR enable", u64::from(t.level_0_xor_enable));
    s("Level 0 is strictly UIF", u64::from(t.level_0_is_strictly_uif));
    s("texture base pointer Cb", u64::from(t.base >> 6));
    s("texture_base pointer_Cr", u64::from(t.base >> 6));
    s("Chroma offset x", 1);
    s("Chroma offset y", 1);
    s("UIF XOR disable", 0);
    s("Flip texture X Axis", 0);
    s("Flip texture Y Axis", 0);
    s("Pad", 0);
    m
}

// ---- the cases ------------------------------------------------------------------------

use texture71::{GlTex71, Target as TexTarget};

const GL_RGBA: u32 = 0x1908;
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RGB565: u32 = 0x8D62;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;

fn gl_tex(target: TexTarget, internal_format: u32, ty: u32, size: u32, levels: u32) -> GlTex71 {
    GlTex71 {
        target,
        internal_format,
        ty,
        width: size,
        height: size,
        levels,
        base_level: 0,
        max_level: texture71::GL_MAX_LEVEL_DEFAULT,
        swizzle: texture71::GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: 0,
        ver: 71,
        iprev: 10,
    }
}

use sampler71::{Filter, MipFilter, Wrap};

fn smp(min: Filter, mag: Filter, mip: MipFilter, wrap: Wrap, cube: bool) -> sampler71::Input {
    sampler71::Input {
        min_filter: min,
        mag_filter: mag,
        mip_filter: mip,
        wrap_s: wrap,
        wrap_t: wrap,
        // Mesa's cube sampler carries wrap R = CLAMP_TO_EDGE (the dump's wrap R is
        // 1 for gl_cube): the GL core must set it, the default 2D sampler has REPEAT
        wrap_r: if cube { Wrap::ClampToEdge } else { Wrap::Repeat },
        cube,
        ..sampler71::Input::default_2d()
    }
}

/// One job of a texture reference: the GL texture and sampler its draw
/// samples, the viewport of the draw, and the dump's job number.
struct Case {
    shape: &'static str,
    job: usize,
    tex: GlTex71,
    sampler: sampler71::Input,
    viewport: (u32, u32),
}

fn cases() -> Vec<Case> {
    use Filter::{Linear, Nearest};
    let n = |s: &'static str, j, t, f, m, w| Case {
        shape: s,
        job: j,
        tex: t,
        sampler: smp(f, f, m, w, false),
        viewport: (64, 64),
    };
    let mut v = vec![];
    // gl_tex2d: a 64x64 RGBA8, NEAREST then LINEAR, REPEAT
    let rgba = gl_tex(TexTarget::Tex2d, 0x8058, GL_UNSIGNED_BYTE, 64, 1);
    v.push(n("gl_tex2d", 1, rgba, Nearest, MipFilter::None, Wrap::Repeat));
    v.push(n("gl_tex2d", 2, rgba, Linear, MipFilter::None, Wrap::Repeat));
    // gl_texfmt: sRGB8_ALPHA8 then RGB565, NEAREST
    let srgb = gl_tex(TexTarget::Tex2d, GL_SRGB8_ALPHA8, GL_UNSIGNED_BYTE, 64, 1);
    let r565 = gl_tex(TexTarget::Tex2d, GL_RGB565, GL_UNSIGNED_SHORT_5_6_5, 64, 1);
    v.push(n("gl_texfmt", 1, srgb, Nearest, MipFilter::None, Wrap::Repeat));
    v.push(n("gl_texfmt", 2, r565, Nearest, MipFilter::None, Wrap::Repeat));
    // gl_mipgen: 7 levels, LINEAR_MIPMAP_LINEAR min, LINEAR mag
    v.push(Case {
        shape: "gl_mipgen",
        job: 1,
        tex: gl_tex(TexTarget::Tex2d, 0x8058, GL_UNSIGNED_BYTE, 64, 7),
        sampler: smp(Linear, Linear, MipFilter::Linear, Wrap::Repeat, false),
        viewport: (64, 64),
    });
    // gl_cube: six 4x4 faces, CLAMP_TO_EDGE, per face NEAREST then LINEAR, 32x32 viewport
    for i in 0..12 {
        let f = if i & 1 == 1 { Linear } else { Nearest };
        v.push(Case {
            shape: "gl_cube",
            job: i + 1,
            tex: gl_tex(TexTarget::Cube, GL_RGBA, GL_UNSIGNED_BYTE, 4, 1),
            sampler: smp(f, f, MipFilter::None, Wrap::ClampToEdge, true),
            viewport: (32, 32),
        });
    }
    v
}

fn revision(shape: &str) -> (u32, u32) {
    let out = read(&format!("tools/v3d/pi5/{shape}.stdout"));
    let r = out.lines().find_map(|l| l.strip_prefix("gl_shapes: renderer V3D ")).expect("renderer line");
    let n: Vec<u32> = r.trim().split('.').map(|x| x.parse().unwrap()).collect();
    (n[0] * 10 + n[1], n[2])
}

// ---- the render jobs ----------------------------------------------------------------------

/// The frame of a texture case: a black clear (a cube's is (0,0,0,0)) in the
/// GL window, then one draw of the whole quad.
fn tex_job(c: &Case) -> (frame71::Frame71, Vec<Target>) {
    let clear = if c.shape == "gl_cube" { [0.0; 4] } else { [0.0, 0.0, 0.0, 1.0] };
    window_job(clear, false, false)
}

/// The draw packet `VERTEX_ARRAY_PRIMS` of a quad: Mesa's values from the
/// dump, the bytes from prim71.
fn check_prim(what: &str, j: &ClifJob) {
    let want: Vec<&Pk> = j.lists.iter().filter(|p| p.name == "VERTEX_ARRAY_PRIMS").collect();
    assert_eq!(want.len(), 1, "{what}: one draw");
    let b = prim71::pack_array(prim71::GL_TRIANGLES, 0, 6, 1).unwrap();
    assert_eq!(b[0], prim71::OP_VERTEX_ARRAY_PRIMS);
    let got = Pk {
        name: "VERTEX_ARRAY_PRIMS".into(),
        fields: vec![
            ("mode".into(), bits(&b[1..], 0, 8).to_string()),
            ("Length".into(), bits(&b[1..], 8, 32).to_string()),
            ("Index of First Vertex".into(), bits(&b[1..], 40, 32).to_string()),
        ],
    };
    assert_same(&format!("{what} draw packet"), &got, want[0]);
}

fn check_tex_job(c: &Case, j: &ClifJob) {
    let what = format!("{} job {}", c.shape, c.job);
    let (fr, t) = tex_job(c);
    check_frame(&what, &assemble(&fr, &t), j);
    let seen = states_at_draws(j);
    assert_eq!(seen.len(), 1, "{what}: one draw");
    check_draw(&format!("{what} draw"), &draw71::DrawState::gl_default(c.viewport.0, c.viewport.1, false, 1), &seen[0]);
    check_prim(&what, j);
    // gl_test's puv vertex: pos4 at 0 (and uv2 at 16), 24 bytes; the cube's
    // shader reads only a_pos
    assert_eq!(j.attr_groups.len(), 1, "{what}: one record group");
    let g = &j.attr_groups[0];
    match c.shape {
        "gl_cube" => check_attrs(&what, &[float_attr(4, 24, 0xe000)], &[4], &[4], g),
        _ => check_attrs(&what, &[float_attr(4, 24, 0xe000), float_attr(2, 24, 0xe010)], &[4, 0], &[4, 2], g),
    }
}

const SHAPES: [(&str, usize); 4] = [("gl_tex2d", 2), ("gl_texfmt", 2), ("gl_cube", 12), ("gl_mipgen", 1)];

#[test]
fn texture_shapes_frame_draw_and_attribute_packets() {
    let mut n = 0;
    for (shape, njobs) in SHAPES {
        let jobs = gl_jobs(shape);
        assert_eq!(jobs.len(), njobs, "{shape}: jobs");
        for c in cases().iter().filter(|c| c.shape == shape) {
            check_tex_job(c, &jobs[c.job - 1]);
            n += 1;
        }
    }
    assert_eq!(n, 17);
}

#[test]
fn texture_shapes_tile_size() {
    for (shape, _) in SHAPES {
        let lists: Vec<Pk> = parse_jobs(shape).into_iter().flat_map(|j| j.lists).collect();
        for r in segments(&lists, "TILE_RENDERING_MODE_CFG_COMMON", "END_RENDER") {
            let tile = (8 << field_u(&r[0], "Log2 Tile Width"), 8 << field_u(&r[0], "Log2 Tile Height"));
            assert_eq!(limits71::choose_tile_size71(1, limits71::Bpp71::Bpp32, false, false), Some(tile), "{shape}");
        }
    }
}

fn field_u(p: &Pk, label: &str) -> u32 {
    p.fields.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("{} has no {label}", p.name)).1.parse().unwrap()
}

#[test]
fn texture_shapes_shader_records() {
    for (shape, _) in SHAPES {
        let recs = parse_records(shape);
        let jobs = gl_jobs(shape);
        let shaders: usize = jobs.iter().map(|j| j.lists.iter().filter(|p| p.name == "GL_SHADER").count()).sum();
        assert_eq!(recs.len(), shaders, "{shape}: a record per GL_SHADER packet");
        for (n, rec) in recs.iter().enumerate() {
            let what = format!("{shape} record {}", n + 1);
            let want = reference_bytes(rec);
            let desc = draw_desc(&d0_values(&want));
            let got =
                record71::pack_draw(record71::BCM2712_D0, &desc).unwrap_or_else(|r| panic!("{what}: {}", r.message()));
            let mut expect = want;
            expect[2] |= 0x20;
            assert_eq!(got, expect, "{what}: the 32 bytes");
        }
    }
}

// ---- texture71 and sampler71 against the dump's records ------------------------------------

#[test]
fn texture_and_sampler_records_match_mesa() {
    let fields = tex_fields();
    let mut checked = 0;
    for (shape, njobs) in SHAPES {
        let dump = parse_texdump(shape);
        let (ver, iprev) = revision(shape);
        assert_eq!((ver, iprev), (71, 10), "{shape}: the Pi 5's 7.1.10");
        assert_eq!(dump.jobs.len(), njobs, "{shape}: jobs");
        for c in cases().iter().filter(|c| c.shape == shape) {
            let what = format!("{shape} job {}", c.job);
            let recs = &dump.jobs[c.job - 1];
            assert_eq!(recs.len(), 1, "{what}: one texture record");
            let r = &recs[0];
            let got: BTreeMap<String, u64> = fields
                .iter()
                .map(|(n, s, z)| {
                    let v = wbits(&r.tex, *s, *z);
                    (n.clone(), if n == "Texture base pointer" { v & !63 } else { v })
                })
                .collect();
            let base = got["Texture base pointer"] as u32;
            let &(bo, extent) = dump.resources.iter().filter(|(a, _)| *a <= base).max_by_key(|(a, _)| *a).unwrap();
            let mut d = c.tex;
            d.bo_address = bo;
            let t = texture71::compose(&d).unwrap_or_else(|e| panic!("{what}: {e}"));
            let want = expected_tex_fields(&t);
            assert_eq!(want.keys().collect::<Vec<_>>(), got.keys().collect::<Vec<_>>(), "{what}: field list");
            for (k, v) in &got {
                assert_eq!(want[k], *v, "{what}: {k}");
            }
            let mut words = [0u32; 6];
            for (n, s, z) in &fields {
                wput(&mut words, *s, *z, want[n]);
            }
            assert_eq!(words, r.tex, "{what}: the six texture words");
            assert!(t.layout.size <= extent, "{what}: layout past the dumped buffer");
            // the pure words texstate71 packs itself agree with the record
            let packed = texstate71::pack_words(
                &texstate71::texture_state(&texstate71::TexDesc {
                    width: t.width,
                    height: t.height,
                    depth: if d.target == TexTarget::Cube { 6 } else { 1 },
                    levels: t.max_level - t.base_level + 1,
                    base_level: t.base_level,
                    cube: d.target == TexTarget::Cube,
                    srgb: t.transfer_func == 1,
                    transfer_func: None,
                    bytes_per_texel: texture71::cpp_of(t.row.pipe).unwrap(),
                    layer_stride: u64::from(t.layout.layer_stride),
                    base: t.base,
                    ver,
                    iprev,
                })
                .unwrap(),
            );
            for (n, s, z) in &fields {
                if matches!(
                    n.as_str(),
                    "Texture base pointer"
                        | "Transfer func"
                        | "Reverse"
                        | "R/B swap"
                        | "Array Stride (64-byte aligned)"
                        | "Image Width"
                        | "Image Height"
                        | "Image Depth"
                        | "Max Level"
                        | "Base Level"
                ) {
                    let mut w = [0u32; 6];
                    w[..4].copy_from_slice(&packed[..4]);
                    let v = wbits(&w, *s, *z);
                    let v = if n == "Texture base pointer" { v & !63 } else { v };
                    assert_eq!(v, got[n], "{what}: texstate71 word, {n}");
                }
            }
            // the sampler's 24 bytes
            let s = sampler71::sampler71(&c.sampler).unwrap_or_else(|e| panic!("{what}: {}", e.message()));
            let bytes = sampler71::pack(&s).unwrap_or_else(|| panic!("{what}: sampler pack"));
            let want_bytes: Vec<u8> = r.smp.iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(&bytes[..], &want_bytes[..], "{what}: the sampler record");
            // P0/P1: not built by any module (KNOWN_WORD_GAPS); they point at the records
            checked += 1;
        }
    }
    assert_eq!(checked, 17);
}

// ---- the pinned gaps ---------------------------------------------------------------------

/// The packets the four dumps carry that no merged module produces (so
/// neither side compares them): the names, sorted.
const KNOWN_GAPS: &[&str] = &[
    "BLEND_CCOLOR",
    "FLUSH",
    "GL_SHADER",
    "LINE_WIDTH",
    "OCCLUSION_QUERY_COUNTER_ENABLE",
    "POINT_SIZE",
    "PRIM_LIST_FORMAT",
    "SAMPLE_STATE",
    "SET_INSTANCEID",
    "START_TILE_BINNING",
    "TRANSFORM_FEEDBACK_SPECS",
    "VCM_CACHE_SIZE",
    "ZERO_ALL_CENTROID_FLAGS",
    "ZERO_ALL_FLATSHADE_FLAGS",
    "ZERO_ALL_NOPERSPECTIVE_FLAGS",
];

/// The words a texture draw's uniform stream carries that no module builds
/// yet (texuni71): per sampled texture, TMU Config Parameter 0 (texture state
/// address, return words) and Parameter 1 (sampler state address, flags).
const KNOWN_WORD_GAPS: &[&str] = &["TMU_CONFIG_P0", "TMU_CONFIG_P1"];

#[test]
fn the_gaps_are_the_pinned_ones() {
    let covered: Vec<&str> = {
        let mut v: Vec<&str> = layouts().iter().map(|l| l.clif).collect();
        // the one-byte packets frame71 writes (not START_TILE_BINNING)
        v.extend(SIMPLE.iter().map(|(_, n)| *n).filter(|n| *n != "START_TILE_BINNING"));
        v.extend(["VERTEX_ARRAY_PRIMS"]);
        v
    };
    let mut gaps = std::collections::BTreeSet::new();
    for (shape, _) in SHAPES {
        for j in parse_jobs(shape) {
            for p in j.lists {
                if !covered.contains(&p.name.as_str()) {
                    gaps.insert(p.name);
                }
            }
        }
    }
    let gaps: Vec<String> = gaps.into_iter().collect();
    assert_eq!(gaps, KNOWN_GAPS, "the packets no module produces changed");
    assert_eq!(KNOWN_WORD_GAPS.len(), 2);
}
