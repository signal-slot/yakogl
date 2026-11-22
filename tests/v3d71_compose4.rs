// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged pure V3D 7.1 modules put together a third time (compose71d):
//! `tests/v3d71_compose.rs`'s and `tests/v3d71_compose2.rs`'s set (`attr71`,
//! `rt71`, `draw71`, `frame71`) plus `prim71`, against Mesa's 7.1
//! references in tools/v3d/pi5/ for the six draw shapes:
//!
//! * `gl_blend` (blend equations and `glBlendColor`), `gl_depthfunc` (depth
//!   test functions, depth mask), `gl_stencilops` (stencil function, ops and
//!   masks, front and back apart): the frame packets of every job, the state
//!   packets of every draw (`CFG_BITS`, `BLEND_ENABLES`, `BLEND_CFG`,
//!   `COLOR_WMASKS`, `STENCIL_CFG`, clip and viewport) and the attribute
//!   records;
//! * `gl_indexed` (8, 16 and 32-bit indices, an index offset, fixed primitive
//!   restart), `gl_instanced` (`glDrawArraysInstanced`, attribute divisors),
//!   `gl_prims` (points, lines, a triangle strip): the draw packets
//!   (`INDEX_BUFFER_SETUP`, `INDEXED_PRIM_LIST`, `VERTEX_ARRAY_PRIMS`,
//!   `VERTEX_ARRAY_INSTANCED_PRIMS`) of every draw, built through `prim71`,
//!   with the same frame, state and attribute checks.
//!
//! What GL each shape issues is tools/v3d/pi5/src/gl_shapes.py. The dump's
//! `@wait_render_all_cores` boundaries do not follow the GL jobs, so the
//! packets are taken in file order like compose2 does. Compared as field
//! values and packet bytes, not BO addresses.
//!
//! Fields of the dump the modules do not model are in [`KNOWN_WORD_GAPS`],
//! each with its reason; a test asserts the gap is real (the dump has the
//! packet) so a list entry cannot go stale silently.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_compose4
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
#[path = "../src/v3d/prim71.rs"]
#[allow(dead_code)]
mod prim71;
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

// ---- draw packets (prim71) -------------------------------------------------

/// The dump's names of the six draw packets, their `prim71` opcodes and
/// byte lengths.
const PRIM_PACKETS: [(&str, &str, u8, usize); 6] = [
    ("INDEXED_PRIM_LIST", "Indexed Prim List", prim71::OP_INDEXED_PRIM_LIST, 10),
    ("INDEXED_INSTANCED_PRIM_LIST", "Indexed Instanced Prim List", prim71::OP_INDEXED_INSTANCED_PRIM_LIST, 14),
    ("VERTEX_ARRAY_PRIMS", "Vertex Array Prims", prim71::OP_VERTEX_ARRAY_PRIMS, 10),
    ("VERTEX_ARRAY_INSTANCED_PRIMS", "Vertex Array Instanced Prims", prim71::OP_VERTEX_ARRAY_INSTANCED_PRIMS, 14),
    ("BASE_VERTEX_BASE_INSTANCE", "Base Vertex Base Instance", prim71::OP_BASE_VERTEX_BASE_INSTANCE, 9),
    ("INDEX_BUFFER_SETUP", "Index Buffer Setup", prim71::OP_INDEX_BUFFER_SETUP, 9),
];

/// A draw packet's bytes as the dump's text.
fn decode_prim(bytes: &[u8]) -> Pk {
    let (clif, xml, _, len) =
        PRIM_PACKETS.iter().find(|p| p.2 == bytes[0]).unwrap_or_else(|| panic!("opcode {}", bytes[0]));
    assert_eq!(bytes.len(), *len, "{clif}: packet length");
    let body = &bytes[1..];
    let fields = prim71::FIELDS
        .iter()
        .filter(|f| f.0 == *xml)
        .map(|(_, name, start, size)| (name.to_string(), bits(body, *start, *size).to_string()))
        .collect();
    Pk { name: clif.to_string(), fields }
}

/// The packets of `pack_draw`'s sequence, split by opcode length.
fn split_draw(mut b: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    while !b.is_empty() {
        let len = PRIM_PACKETS.iter().find(|p| p.2 == b[0]).expect("a draw packet").3;
        out.push(b[..len].to_vec());
        b = &b[len..];
    }
    out
}

/// The draw packets of a job in dump order.
fn dump_draw_packets(j: &ClifJob) -> Vec<Pk> {
    between(&j.lists, "NUMBER_OF_LAYERS", "FLUSH")
        .into_iter()
        .filter(|p| PRIM_PACKETS.iter().any(|k| k.0 == p.name))
        .collect()
}

fn is_prim(name: &str) -> bool {
    matches!(
        name,
        "INDEXED_PRIM_LIST" | "INDEXED_INSTANCED_PRIM_LIST" | "VERTEX_ARRAY_PRIMS" | "VERTEX_ARRAY_INSTANCED_PRIMS"
    )
}

/// The dump's state packets at each draw: the last one of each kind seen
/// before the draw's prim packet. A kind sent twice before one draw (the
/// two `STENCIL_CFG` of a two-sided stencil) keeps the second as `KIND#1`.
fn states_at_draws(j: &ClifJob) -> Vec<BTreeMap<String, Pk>> {
    let kinds: Vec<&str> = draw71::ALL_SPECS.iter().map(|s| s.clif_name).collect();
    let mut last: BTreeMap<String, Pk> = BTreeMap::new();
    let mut fresh: Vec<String> = vec![];
    let mut out = vec![];
    for p in between(&j.lists, "NUMBER_OF_LAYERS", "FLUSH") {
        if kinds.contains(&p.name.as_str()) {
            if fresh.contains(&p.name) {
                last.insert(format!("{}#1", p.name), p);
            } else {
                last.remove(&format!("{}#1", p.name));
                fresh.push(p.name.clone());
                last.insert(p.name.clone(), p);
            }
        } else if is_prim(&p.name) {
            out.push(last.clone());
            fresh.clear();
        }
    }
    out
}

/// A dump packet the modules do not model: its name and why.
struct Gap {
    packet: &'static str,
    reason: &'static str,
}

/// Every packet of the six dumps no module produces, with the reason. A
/// packet here is not compared; the tests assert each one is in a dump (a
/// stale entry fails) and pin the values where they matter.
const KNOWN_WORD_GAPS: &[Gap] = &[
    Gap {
        packet: "BLEND_CCOLOR",
        reason: "draw71 has no PacketSpec for the constant blend colour (four F16 words of glBlendColor);                  gl_blend_equations checks the dump's words are the half floats of glBlendColor's arguments",
    },
    Gap {
        packet: "POINT_SIZE",
        reason: "draw71 has no PacketSpec for Point Size (glPointSize / gl_PointSize); gl_prims_points_lines_strip \
                 pins the dump's value 1.0",
    },
    Gap {
        packet: "LINE_WIDTH",
        reason: "draw71 has no PacketSpec for Line width (glLineWidth); gl_prims_points_lines_strip pins 1.0",
    },
    Gap {
        packet: "PRIM_LIST_FORMAT",
        reason: "frame71 has no Primitive List Format packet (tile list: triangles, not strip or fan)",
    },
    Gap {
        packet: "SET_INSTANCEID",
        reason: "frame71 has no Set InstanceID packet (tile list, Instance ID 0)",
    },
    Gap {
        packet: "VCM_CACHE_SIZE",
        reason: "the binner's VCM cache size is the 7.1 table's own (vpm71q, not a module here)",
    },
    Gap {
        packet: "OCCLUSION_QUERY_COUNTER_ENABLE",
        reason: "occlusion queries are outside the six gl_test cases; no module",
    },
    Gap { packet: "TRANSFORM_FEEDBACK_SPECS", reason: "transform feedback is refused by prim71; no module" },
    Gap {
        packet: "SAMPLE_STATE",
        reason: "draw71 has no Sample State packet (sample mask 15, coverage 1.0: the non-multisample values)",
    },
    Gap {
        packet: "ZERO_ALL_FLATSHADE_FLAGS",
        reason: "varying flag packets belong to the shader link (shaderc), not to a state module",
    },
    Gap { packet: "ZERO_ALL_NOPERSPECTIVE_FLAGS", reason: "as ZERO_ALL_FLATSHADE_FLAGS" },
    Gap { packet: "ZERO_ALL_CENTROID_FLAGS", reason: "as ZERO_ALL_FLATSHADE_FLAGS" },
    Gap { packet: "GL_SHADER", reason: "the packet's record address; record71 (compose2) owns the record" },
];

/// The packet kinds of a dump the compositions neither compare nor list as a
/// gap are listed by this test, so a new packet in a reference is not
/// silently skipped.
fn assert_no_silent_kinds(shape: &str, jobs: &[ClifJob]) {
    let state: Vec<&str> = draw71::ALL_SPECS.iter().map(|s| s.clif_name).collect();
    let frame: Vec<&str> = layouts().iter().map(|l| l.clif).collect();
    let simple: Vec<&str> = SIMPLE.iter().map(|s| s.1).collect();
    let mut silent = std::collections::BTreeSet::new();
    for p in jobs.iter().flat_map(|j| j.lists.iter()) {
        let n = p.name.as_str();
        let known = state.contains(&n)
            || frame.contains(&n)
            || simple.contains(&n)
            || n == "FLUSH"
            || PRIM_PACKETS.iter().any(|k| k.0 == n)
            || KNOWN_WORD_GAPS.iter().any(|g| g.packet == n);
        if !known {
            silent.insert(n.to_string());
        }
    }
    assert!(silent.is_empty(), "{shape}: packets neither compared nor listed in KNOWN_WORD_GAPS: {silent:?}");
}

// ---- the jobs ---------------------------------------------------------------

/// `Height in UB or Stride` of a UIF target: the height in 8-row blocks.
fn uif_height(h: u32) -> u32 {
    h.div_ceil(8)
}

fn rgba8_surface(height: u32, address: u32) -> frame71::Surface71 {
    frame71::Surface71 {
        format: 27,
        memory_format: 4,
        height_in_ub_or_stride: uif_height(height),
        address,
        ..Default::default()
    }
}

struct Target {
    gl_internal: u32,
    clear: rt71::ClearColor,
    surface: frame71::Surface71,
}

const GL_RGBA8: u32 = 0x8058;

struct Assembled {
    binner: Vec<Vec<u8>>,
    render: Vec<Vec<u8>>,
    tile: Vec<Vec<u8>>,
}

fn rt_parts(targets: &[Target], tile_w: u32, tile_h: u32) -> Vec<Vec<u8>> {
    let mut out = vec![];
    let mut base = 0;
    for (n, t) in targets.iter().enumerate() {
        let rt = rt71::rt_format71(t.gl_internal).unwrap();
        let c = rt71::clear_words71(&rt, t.clear).unwrap();
        let stride = rt71::rt_stride(tile_w, rt.bpp);
        let n = n as u32;
        out.push(rt71::packet_bytes(rt71::part1_payload(n, &rt, c.low, stride, base).unwrap()).to_vec());
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

/// What a shape's job has besides its RGBA8 colour target.
struct Zs {
    kind: frame71::DepthKind71,
    /// Output Image Format, load z, load s, store z, store s
    attach: Option<(u8, bool, bool, bool, bool)>,
    z_clear: f32,
    stencil_clear: u8,
    early_ds_clear: bool,
}

/// One job: a colour target (RGBA8, 0x3000) cleared to `clear` and stored.
fn job(clear: [f32; 4], zs: Zs, ez_disable: bool) -> (frame71::Frame71, Vec<Target>) {
    let t = vec![Target {
        gl_internal: GL_RGBA8,
        clear: rt71::ClearColor::Float(clear),
        surface: rgba8_surface(64, 0x3000),
    }];
    let mut fr = frame71::Frame71::new(64, 64, 64, 64);
    fr.render_targets = 1;
    fr.rt[0] = Some(frame71::Attach71 { surface: t[0].surface, load: false, end: frame71::EndOp71::Store });
    fr.depth_kind = zs.kind;
    fr.zs = zs.attach.map(|(fmt, lz, ls, sz, ss)| frame71::ZsAttach71 {
        surface: frame71::Surface71 {
            format: fmt,
            memory_format: 4,
            height_in_ub_or_stride: uif_height(64),
            ..Default::default()
        },
        load_z: lz,
        load_s: ls,
        store_z: sz,
        store_s: ss,
    });
    fr.z_clear_bits = zs.z_clear.to_bits();
    fr.stencil_clear = zs.stencil_clear;
    fr.early_ds_clear = zs.early_ds_clear;
    fr.early_z_disable = ez_disable;
    (fr, t)
}

/// The window of gl_test: an RGBA8 target and a D24S8 buffer nothing is
/// stored from, `Early Depth/Stencil Clear` set (compose71's gl_scissor).
fn window() -> Zs {
    Zs {
        kind: frame71::DepthKind71::D24,
        attach: Some((0, false, false, false, false)),
        z_clear: 0.0,
        stencil_clear: 0,
        early_ds_clear: true,
    }
}

fn gl() -> draw71::DrawState {
    draw71::DrawState::gl_default(64, 64, false, 1)
}

fn float_attr(components: u32, stride: u64, addr: u32, divisor: u32) -> attr71::Attr {
    attr71::Attr {
        enabled: true,
        components,
        gl_type: attr71::GL_FLOAT,
        stride,
        buffer_addr: addr,
        divisor,
        ..attr71::Attr::disabled()
    }
}

/// The attributes of a draw: the descriptions and what the coordinate and
/// vertex shaders read of each.
struct Attrs {
    attrs: Vec<attr71::Attr>,
    cs: Vec<u8>,
    vs: Vec<u8>,
}

/// gl_test's 40-byte vertex: position, colour, texture coordinate.
fn std_attrs() -> Attrs {
    Attrs {
        attrs: vec![float_attr(4, 40, 0xe000, 0), float_attr(4, 40, 0xe010, 0), float_attr(2, 40, 0xe020, 0)],
        cs: vec![4, 0, 0],
        vs: vec![4, 4, 2],
    }
}

fn check_attrs(what: &str, a: &Attrs, group: &[Pk]) {
    let mut c = [0u8; attr71::MAX_ATTRS];
    let mut v = [0u8; attr71::MAX_ATTRS];
    c[..a.cs.len()].copy_from_slice(&a.cs);
    v[..a.vs.len()].copy_from_slice(&a.vs);
    let p = attr71::plan(&a.attrs, &c, &v, false, 0x1000).unwrap();
    assert_eq!(p.count, group.len(), "{what}: number of attribute arrays");
    for (i, want) in group.iter().enumerate() {
        assert_same(&format!("{what} attr {i}"), &decode_attr(&attr71::pack(&p.records[i])), want);
    }
}

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

/// One draw of a job: the GL state and the prim71 draw.
struct DrawCall {
    state: draw71::DrawState,
    draw: prim71::Draw,
}

fn dc(state: draw71::DrawState, draw: prim71::Draw) -> DrawCall {
    DrawCall { state, draw }
}

fn tris(first: u64, count: u64) -> prim71::Draw {
    prim71::Draw::arrays(prim71::GL_TRIANGLES, first, count)
}

/// One job against the dump's job: the frame packets, each draw's state
/// packets, its draw packets and its attribute records.
fn check_job(what: &str, j: &ClifJob, (fr, t): (frame71::Frame71, Vec<Target>), draws: &[DrawCall], at: &Attrs) {
    check_frame(what, &assemble(&fr, &t), j);
    let seen = states_at_draws(j);
    assert_eq!(seen.len(), draws.len(), "{what}: number of draws");
    for (n, (d, s)) in draws.iter().zip(&seen).enumerate() {
        check_draw(&format!("{what} draw {}", n + 1), &d.state, s);
    }
    let mut packets = vec![];
    for d in draws {
        packets.extend(split_draw(&prim71::pack_draw(&d.draw).unwrap_or_else(|e| panic!("{what}: {e}"))));
    }
    let got = dump_draw_packets(j);
    let decoded: Vec<Pk> = packets.iter().map(|b| decode_prim(b)).collect();
    let names = |v: &[Pk]| v.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&decoded), names(&got), "{what}: the draw packet sequence");
    for (i, (g, w)) in decoded.iter().zip(&got).enumerate() {
        assert_same(&format!("{what} draw packet {i}"), g, w);
    }
    // Mesa emits a GL_SHADER (and its attribute records) when the program
    // or vertex state changed: the groups are the job's GL_SHADER packets
    let shaders = j.lists.iter().filter(|p| p.name == "GL_SHADER").count();
    assert_eq!(j.attr_groups.len(), shaders, "{what}: an attribute group per GL_SHADER");
    for (n, g) in j.attr_groups.iter().enumerate() {
        check_attrs(&format!("{what} record {}", n + 1), at, g);
    }
}

// ---- blend, depth and stencil ------------------------------------------------

use draw71::{BlendFactor as F, BlendMode as M, Compare, StencilOp as O};

fn blend_state(eq: draw71::BlendEq) -> draw71::DrawState {
    let mut d = gl();
    d.targets[0].blend_enable = true;
    d.targets[0].blend = eq;
    d
}

/// The F16 of a value exactly representable as one (0.25, 0.5, 0.75).
fn f16_bits(v: f32) -> u32 {
    let b = v.to_bits();
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    assert!((1..31).contains(&exp) && b & 0x1fff == 0, "{v} is not an exact F16");
    ((b >> 16) & 0x8000) | ((exp as u32) << 10) | ((b >> 13) & 0x3ff)
}

#[test]
fn gl_blend_equations() {
    let jobs = gl_jobs("gl_blend");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_blend", &jobs);
    let a = blend_state(draw71::BlendEq {
        rgb_mode: M::Add,
        rgb_src: F::SrcAlpha,
        rgb_dst: F::InvSrcAlpha,
        alpha_mode: M::Add,
        alpha_src: F::One,
        alpha_dst: F::Zero,
    });
    let b = blend_state(draw71::BlendEq {
        rgb_mode: M::RSub,
        rgb_src: F::ConstColor,
        rgb_dst: F::InvConstColor,
        alpha_mode: M::Max,
        alpha_src: F::One,
        alpha_dst: F::One,
    });
    check_job(
        "gl_blend",
        &jobs[0],
        job([0.0, 0.0, 0.0, 1.0], window(), false),
        &[dc(a, tris(0, 6)), dc(b, tris(6, 6))],
        &std_attrs(),
    );
    // the KNOWN_WORD_GAPS entry: BLEND_CCOLOR is in the dump, as the half
    // floats of the blend colour (0.25, 0.5, 0.75, 0.5) glBlendColor sets
    let cc: Vec<&Pk> = jobs[0].lists.iter().filter(|p| p.name == "BLEND_CCOLOR").collect();
    assert_eq!(cc.len(), 2, "one BLEND_CCOLOR before each blended draw");
    let words: Vec<u32> = cc[1].fields.iter().map(|(_, v)| v.parse().unwrap()).collect();
    assert_eq!(words, [0.25, 0.5, 0.75, 0.5].map(f16_bits), "the second draw's BLEND_CCOLOR");
    assert!(cc[0].fields.iter().all(|(_, v)| v == "0"), "the first draw's blend colour is 0");
}

#[test]
fn gl_depthfunc() {
    let jobs = gl_jobs("gl_depthfunc");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_depthfunc", &jobs);
    // the depth test on, the depth mask off: LESS, then GEQUAL
    let mut a = gl();
    a.depth_test = true;
    a.depth_write = false;
    let mut b = a;
    b.depth_func = Compare::GEqual;
    let zs = Zs {
        kind: frame71::DepthKind71::D24,
        attach: Some((43, false, false, true, false)),
        z_clear: 0.5,
        stencil_clear: 0,
        early_ds_clear: false,
    };
    check_job(
        "gl_depthfunc",
        &jobs[0],
        job([0.0, 0.0, 1.0, 1.0], zs, false),
        &[dc(a, tris(0, 6)), dc(b, tris(6, 6))],
        &std_attrs(),
    );
}

fn stencil_face(
    func: Compare,
    ops: (O, O, O),
    reference: u32,
    value_mask: u32,
    write_mask: u32,
) -> draw71::StencilFace {
    draw71::StencilFace { func, sfail: ops.0, zfail: ops.1, zpass: ops.2, reference, value_mask, write_mask }
}

#[test]
fn gl_stencilops() {
    let jobs = gl_jobs("gl_stencilops");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_stencilops", &jobs);
    // 1. LEQUAL 0x20 mask 0x3f, INCR / DECR / INVERT, write mask 0x0f
    let mut a = gl();
    a.depth_test = true;
    a.stencil_test = true;
    a.stencil_front = stencil_face(Compare::LEqual, (O::Incr, O::Decr, O::Invert), 0x20, 0x3f, 0x0f);
    a.stencil_back = a.stencil_front;
    // 2. front NOTEQUAL 5 mask 0xf0, REPLACE / KEEP / INCWRAP; back ZERO /
    // DECWRAP / INVERT; write mask 0xff
    let mut b = a;
    b.stencil_two_sided = true;
    b.stencil_front = stencil_face(Compare::NotEqual, (O::Replace, O::Keep, O::IncWrap), 5, 0xf0, 0xff);
    b.stencil_back = stencil_face(Compare::NotEqual, (O::Zero, O::DecWrap, O::Invert), 5, 0xf0, 0xff);
    let zs = Zs {
        kind: frame71::DepthKind71::D24,
        attach: Some((43, false, false, true, true)),
        z_clear: 1.0,
        stencil_clear: 0x10,
        early_ds_clear: false,
    };
    check_job(
        "gl_stencilops",
        &jobs[0],
        job([0.0, 0.0, 1.0, 1.0], zs, true),
        &[dc(a, tris(0, 6)), dc(b, tris(6, 6))],
        &std_attrs(),
    );
}

// ---- indexed, instanced and point/line draws ----------------------------------

fn elements(mode: u32, gl_type: u32, offset: u64, count: u64, restart: bool) -> prim71::Draw {
    prim71::Draw {
        mode,
        indices: Some(prim71::Indices { gl_type, addr: 0x10000, size: 4096, offset, restart }),
        first: 0,
        count,
        instances: 1,
        base_vertex: 0,
        base_instance: 0,
    }
}

#[test]
fn gl_indexed_draws() {
    let jobs = gl_jobs("gl_indexed");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_indexed", &jobs);
    use prim71::{GL_TRIANGLES, GL_TRIANGLE_STRIP, GL_UNSIGNED_BYTE, GL_UNSIGNED_INT, GL_UNSIGNED_SHORT};
    // 8-bit, 16-bit with a 4-byte index offset, 32-bit, then 16-bit with the
    // fixed restart index in a strip (the buffer is a 4096-byte BO each time)
    let draws = [
        dc(gl(), elements(GL_TRIANGLES, GL_UNSIGNED_BYTE, 0, 6, false)),
        dc(gl(), elements(GL_TRIANGLES, GL_UNSIGNED_SHORT, 4, 6, false)),
        dc(gl(), elements(GL_TRIANGLES, GL_UNSIGNED_INT, 0, 6, false)),
        dc(gl(), elements(GL_TRIANGLE_STRIP, GL_UNSIGNED_SHORT, 0, 7, true)),
    ];
    check_job("gl_indexed", &jobs[0], job([0.0, 0.0, 0.0, 1.0], window(), false), &draws, &std_attrs());
}

#[test]
fn gl_instanced_draw() {
    let jobs = gl_jobs("gl_instanced");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_instanced", &jobs);
    // a_pos per vertex (16-byte vertex), a_color and a_off per instance
    // (divisor 1, 24-byte instance); the coordinate shader reads a_pos and
    // a_off only
    let at = Attrs {
        attrs: vec![float_attr(4, 16, 0xe000, 0), float_attr(4, 24, 0xf000, 1), float_attr(2, 24, 0xf010, 1)],
        cs: vec![4, 0, 2],
        vs: vec![4, 4, 2],
    };
    let mut d = tris(0, 6);
    d.instances = 3;
    check_job("gl_instanced", &jobs[0], job([0.0, 0.0, 0.0, 1.0], window(), false), &[dc(gl(), d)], &at);
}

#[test]
fn gl_prims_points_lines_strip() {
    let jobs = gl_jobs("gl_prims");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_prims", &jobs);
    let draws = [
        dc(gl(), prim71::Draw::arrays(prim71::GL_POINTS, 0, 4)),
        dc(gl(), prim71::Draw::arrays(prim71::GL_LINES, 4, 4)),
        dc(gl(), prim71::Draw::arrays(prim71::GL_TRIANGLE_STRIP, 8, 4)),
    ];
    check_job("gl_prims", &jobs[0], job([0.0, 0.0, 0.0, 1.0], window(), false), &draws, &std_attrs());
    // the gaps: Point Size and Line width are 1.0 in every draw
    for name in ["POINT_SIZE", "LINE_WIDTH"] {
        let sizes: Vec<&Pk> = jobs[0].lists.iter().filter(|p| p.name == name).collect();
        assert!(!sizes.is_empty(), "the dump has {name}");
        assert!(sizes.iter().all(|p| p.fields[0].1 == "1.000000"), "{name} is 1.0");
    }
}

// ---- the gap list ---------------------------------------------------------------

#[test]
fn known_gaps_are_real_and_explained() {
    let jobs: Vec<ClifJob> = ["gl_blend", "gl_depthfunc", "gl_stencilops", "gl_indexed", "gl_instanced", "gl_prims"]
        .into_iter()
        .flat_map(parse_jobs)
        .collect();
    for g in KNOWN_WORD_GAPS {
        assert!(g.reason.len() > 20, "{}: a reason", g.packet);
        assert!(
            jobs.iter().any(|j| j.lists.iter().any(|p| p.name == g.packet)),
            "{} is in none of the six dumps: drop the gap",
            g.packet
        );
        // a gap is not a packet a module writes
        assert!(!draw71::ALL_SPECS.iter().any(|s| s.clif_name == g.packet), "{} is modelled", g.packet);
    }
}
