// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged pure V3D 7.1 modules (`attr71`, `rt71`, `draw71`, `frame71`,
//! `ident71`) put together the way a 7.1 backend would, against Mesa's 7.1
//! references in tools/v3d/pi5/ (compose71).
//!
//! Each module has its own test. This file calls only their public
//! functions, from the GL-level description of four shapes, packs every
//! packet they cover to bytes and compares the bytes, decoded through the
//! modules' own field positions, with every value the `.clif` dump prints
//! (tools/v3d/pi5/README.md: Mesa's `V3D_DEBUG=clif` text, one packet name
//! line and `/* label: */ value` lines under it; `[bo+off]` addresses and BO
//! numbers are not compared):
//!
//! * `clear.clif`: a 64x64 RGBA8 target cleared to blue (rt71's
//!   `Clear Render Targets` and Part1 words, frame71's binning and
//!   rendering config, the two dummy tiles, the tile-buffer store);
//! * `triangle.clif`: the same job with one Gouraud triangle (draw71's state
//!   packets, attr71's two attribute records);
//! * `gl_scissor.clif`: gl_test case 5, two jobs: a black clear, then a
//!   blitter-clear quad and a scissored quad (clip window, a tile-buffer
//!   load, three attribute records);
//! * `gl_mrt4.clif`: four RGBA8 targets in a 64x32 tile (four Part1 words
//!   with bases 0, 128, 256, 384, four stores).
//!
//! Only packets the four modules cover are compared; what they do not cover
//! (`PRIM_LIST_FORMAT`, `SET_INSTANCEID`, `POINT_SIZE`, the shader record,
//! `START_TILE_BINNING`, `FLUSH`, ...) is left out of both sides. The
//! binner's state is compared per draw as the dump's state at that draw
//! (Mesa re-emits a packet only when it changed). The Frame71 inputs the
//! modules take as plain values (the Output Image Format 27, the UIF memory
//! format, the tile size, the early-Z and depth-type bits of the job) are
//! written here as the backend decides them, each with the dump's line.
//!
//! The last tests compare what the modules say twice (the 8 targets and
//! 7680 px limits, the refusals) and the board's identity words with
//! `ident71`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_compose
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

// ---- the four shapes --------------------------------------------------------

#[test]
fn clear_job() {
    let jobs = parse_jobs("clear");
    assert_eq!(jobs.len(), 1);
    // glClearColor(0, 0, 1, 1) on a 64x64 GL_RGBA8 FBO (README: clear.clif)
    let t = [rgba8_target([0.0, 0.0, 1.0, 1.0], 0x9000, false)];
    let fr = color_frame(64, 64, 64, 64, &t);
    let a = assemble(&fr, &t);
    check_frame("clear", &a, &jobs[0]);
    assert!(states_at_draws(&jobs[0]).is_empty(), "the clear job has no draw");
}

#[test]
fn triangle_job() {
    let jobs = parse_jobs("triangle");
    assert_eq!(jobs.len(), 1);
    let t = [rgba8_target([0.0, 0.0, 1.0, 1.0], 0x9000, false)];
    let fr = color_frame(64, 64, 64, 64, &t);
    check_frame("triangle", &assemble(&fr, &t), &jobs[0]);

    // one Gouraud triangle, GL's initial state, an FBO
    let draws = states_at_draws(&jobs[0]);
    assert_eq!(draws.len(), 1);
    check_draw("triangle", &draw71::DrawState::gl_default(64, 64, false, 1), &draws[0]);

    // attributes 0 and 1, vec4 float at stride 16, the coordinate shader
    // reads attribute 0 only
    assert_eq!(jobs[0].attr_groups.len(), 1);
    let attrs = [float_attr(4, 16, 0x15000), float_attr(4, 16, 0x15030)];
    check_attrs("triangle", &attrs, &[4, 0], &[4, 4], &jobs[0].attr_groups[0]);
}

#[test]
fn gl_scissor_case_5() {
    let jobs = parse_jobs("gl_scissor");
    assert_eq!(jobs.len(), 2);

    // job 1: the black clear (0, 0, 0, 1). The window has depth and stencil
    // (a GL_DEPTH24_STENCIL8 renderbuffer, neither stored): depth_24 and
    // `Early Depth/Stencil Clear`, `Early-Z disable` set (gl_scissor.clif
    // :39-41).
    let mut t = [rgba8_target([0.0, 0.0, 0.0, 1.0], 0x3000, false)];
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
    check_frame("gl_scissor job 1", &assemble(&fr, &t), &jobs[0]);
    assert!(states_at_draws(&jobs[0]).is_empty());

    // job 2: no clear (colour 0), render target 0 loaded back, `Early-Z
    // disable` clear (gl_scissor.clif:334)
    t[0] = rgba8_target([0.0, 0.0, 0.0, 0.0], 0x3000, true);
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
    fr.early_z_disable = false;
    check_frame("gl_scissor job 2", &assemble(&fr, &t), &jobs[1]);

    // draw 1: the clear as a blitter quad over the whole target; draw 2:
    // the yellow quad under glScissor(0, 0, 32, 32)
    let draws = states_at_draws(&jobs[1]);
    assert_eq!(draws.len(), 2);
    check_draw("gl_scissor draw 1", &draw71::DrawState::blitter_clear(64, 64, false, 1, 0xf, None), &draws[0]);
    let mut d2 = draw71::DrawState::gl_default(64, 64, false, 1);
    d2.scissor = Some([0, 0, 32, 32]);
    check_draw("gl_scissor draw 2", &d2, &draws[1]);

    // attribute records: the blitter's vec3 position at stride 36, then
    // gl_test's 40-byte vertex (attributes 0, 1, 2: vec4, vec4, vec2)
    assert_eq!(jobs[1].attr_groups.len(), 2);
    check_attrs("gl_scissor draw 1", &[float_attr(3, 36, 0x15000)], &[4], &[4], &jobs[1].attr_groups[0]);
    let attrs = [float_attr(4, 40, 0xe000), float_attr(4, 40, 0xe010), float_attr(2, 40, 0xe020)];
    check_attrs("gl_scissor draw 2", &attrs, &[4, 0, 0], &[4, 4, 2], &jobs[1].attr_groups[1]);
}

#[test]
fn gl_mrt4_four_targets() {
    let jobs = parse_jobs("gl_mrt4");
    assert_eq!(jobs.len(), 1);
    // glDrawBuffers(4), all cleared to blue; four 32 bpp targets make
    // Mesa choose a 64x32 tile (gl_mrt4.clif:34)
    let t: Vec<Target> = (0..4).map(|i| rgba8_target([0.0, 0.0, 1.0, 1.0], 0x10000 * (i + 1), false)).collect();
    let fr = color_frame(64, 64, 64, 32, &t);
    let a = assemble(&fr, &t);
    check_frame("gl_mrt4", &a, &jobs[0]);
    let parts = a.render.iter().filter(|b| decode(b).name == "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1").count();
    assert_eq!(parts, 4);
}

// ---- the modules against each other -----------------------------------------

#[test]
fn the_limits_agree() {
    assert_eq!(rt71::MAX_RENDER_TARGETS, frame71::MAX_RTS);
    assert_eq!(rt71::MAX_RENDER_TARGETS as usize, draw71::MAX_TARGETS);
    assert_eq!(rt71::MAX_SIZE, frame71::MAX_SIZE);
    assert_eq!(rt71::MAX_SIZE, draw71::MAX_SIZE);
    assert_eq!(attr71::MAX_ATTRS, 16);
}

#[test]
fn the_refusals_agree_at_the_limits() {
    // nine targets: refused by all three modules
    assert!(rt71::check_targets(9).is_err());
    let mut fr = frame71::Frame71::new(64, 64, 32, 32);
    fr.render_targets = 9;
    assert_eq!(frame71::plan(&fr).unwrap_err(), frame71::Refusal71::TooManyRenderTargets);
    assert_eq!(
        draw71::translate(&draw71::DrawState::gl_default(64, 64, false, 9)).unwrap_err(),
        draw71::Refusal71::TooManyTargets
    );
    // eight are fine everywhere
    assert!(rt71::check_targets(8).is_ok());
    let mut fr = frame71::Frame71::new(64, 64, 32, 32);
    fr.render_targets = 8;
    assert!(frame71::plan(&fr).is_ok());
    assert!(draw71::translate(&draw71::DrawState::gl_default(64, 64, false, 8)).is_ok());
    // 7680 px is the largest side; 7681 is refused by all three
    assert!(rt71::check_size(7680, 7680).is_ok());
    assert!(rt71::check_size(7681, 64).is_err());
    assert!(frame71::plan(&frame71::Frame71::new(7680, 7680, 64, 64)).is_ok());
    assert_eq!(frame71::plan(&frame71::Frame71::new(7681, 64, 64, 64)).unwrap_err(), frame71::Refusal71::FrameTooLarge);
    assert!(draw71::translate(&draw71::DrawState::gl_default(7680, 7680, false, 1)).is_ok());
    assert_eq!(
        draw71::translate(&draw71::DrawState::gl_default(7681, 64, false, 1)).unwrap_err(),
        draw71::Refusal71::TargetSize
    );
}

#[test]
fn eight_targets_compose_with_the_mrt8_reference() {
    // gl_mrt8.clif: eight GL_RGBA8 targets in a 32x32 tile, bases 0, 64,
    // 128 ... (rt71's own reference test checks the Part1 words)
    let jobs = parse_jobs("gl_mrt8");
    assert_eq!(jobs.len(), 1);
    let t: Vec<Target> = (0..8).map(|i| rgba8_target([0.0, 0.0, 1.0, 1.0], 0x10000 * (i + 1), false)).collect();
    let fr = color_frame(64, 64, 32, 32, &t);
    check_frame("gl_mrt8", &assemble(&fr, &t), &jobs[0]);
}

#[test]
fn the_clear_is_clear_render_targets_never_the_store_bit() {
    // every Store the frame plan writes has "Clear buffer being stored" 0,
    // and the tile list ends in packet 25 (the dumps' CLEAR_RT)
    let t = [rgba8_target([0.0, 0.0, 1.0, 1.0], 0x9000, false)];
    let a = assemble(&color_frame(64, 64, 64, 64, &t), &t);
    for b in a.render.iter().chain(&a.tile) {
        let p = decode(b);
        if p.name == "STORE" {
            assert!(p.fields.contains(&("Clear buffer being stored".to_string(), "0".to_string())));
        }
    }
    let n = a.tile.len();
    assert_eq!(a.tile[n - 3], vec![rt71::CLEAR_RT_CODE]);
    assert_eq!(rt71::CLEAR_RT_CODE, frame71::OP_CLEAR_RT);
}

#[test]
fn the_board_identity_passes_ident71() {
    // HUB_IDENT1 and HUB_IDENT3 of tools/v3d/pi5/v3d_ident.txt's board
    // (README: `0x42554856 0x00081117 0x00001900 0x00020a10`)
    let text = read("tools/v3d/pi5/README.md");
    assert!(text.contains("0x42554856 0x00081117 0x00001900 0x00020a10"));
    let id = ident71::check_words(0x0008_1117, 0x0002_0a10).expect("the Pi 5's identity is accepted");
    assert_eq!(id, ident71::decode(0x0008_1117, 0x0002_0a10));
}
