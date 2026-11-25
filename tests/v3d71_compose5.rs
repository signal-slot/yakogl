// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The merged pure V3D 7.1 modules put together a fourth time (compose71d,
//! second half): `tests/v3d71_compose4.rs`'s set (`attr71`, `rt71`,
//! `draw71`, `frame71`, `prim71`) against the three Mesa 7.1 references
//! refs71b added to tools/v3d/pi5/ after compose4's six:
//!
//! * `gl_blendsub` (SUBTRACT, REVERSE_SUBTRACT, MIN and SRC_ALPHA_SATURATE
//!   through `glBlendFuncSeparate` / `glBlendEquationSeparate`): four
//!   blended draws, each with its own `BLEND_CFG`;
//! * `gl_depthfuncs` (NEVER, EQUAL, LEQUAL, GREATER, NOTEQUAL, ALWAYS with
//!   the depth mask on, then LESS with it off): seven draws, each with its
//!   own `CFG_BITS`, the depth write mask changing inside the job;
//! * `gl_primswide` (points of `gl_PointSize` 16, lines of width 2.0 and
//!   4.0, a line strip of width 2.0): the draw packets through `prim71`.
//!
//! Compared per job: the frame packets, the state packets of every draw
//! (`CFG_BITS`, `BLEND_ENABLES`, `BLEND_CFG`, `COLOR_WMASKS`, clip and
//! viewport), the draw packets and the attribute records, as field values
//! and not as BO addresses. What GL each shape issues is
//! tools/v3d/pi5/src/gl_shapes.py (`DRAW2_SHAPES`). The harness is
//! compose4's.
//!
//! Packets of the dumps the modules do not model are in
//! [`KNOWN_WORD_GAPS`], each with its reason. The three these shapes are
//! about (`LINE_WIDTH`, `POINT_SIZE`, `BLEND_CCOLOR`) are pinned twice: a
//! passing test holds the reference's values, and an `#[ignore]` test
//! (`gap_*`) fails until `draw71` has a `PacketSpec` for the packet.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_compose5
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

/// Every packet of the three dumps no module produces, with the reason. A
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

// ---- blend, depth and wide primitives ------------------------------------------------

use draw71::{BlendFactor as F, BlendMode as M, Compare};

fn blend_state(eq: draw71::BlendEq) -> draw71::DrawState {
    let mut d = gl();
    d.targets[0].blend_enable = true;
    d.targets[0].blend = eq;
    d
}

fn eq(rgb: (M, F, F), alpha: (M, F, F)) -> draw71::BlendEq {
    draw71::BlendEq {
        rgb_mode: rgb.0,
        rgb_src: rgb.1,
        rgb_dst: rgb.2,
        alpha_mode: alpha.0,
        alpha_src: alpha.1,
        alpha_dst: alpha.2,
    }
}

/// The packets of one name in a job, in file order.
fn named<'a>(j: &'a ClifJob, name: &str) -> Vec<&'a Pk> {
    j.lists.iter().filter(|p| p.name == name).collect()
}

#[test]
fn gl_blendsub_equations() {
    let jobs = gl_jobs("gl_blendsub");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_blendsub", &jobs);
    // gl_shapes.py blendsub(): glBlendFuncSeparate(src, dst, asrc, adst) and
    // glBlendEquationSeparate(colour, alpha) before each quad
    let draws = [
        dc(blend_state(eq((M::Sub, F::SrcAlpha, F::One), (M::Sub, F::One, F::One))), tris(0, 6)),
        dc(blend_state(eq((M::RSub, F::One, F::One), (M::RSub, F::One, F::One))), tris(6, 6)),
        dc(blend_state(eq((M::Add, F::DstColor, F::Zero), (M::Min, F::One, F::One))), tris(12, 6)),
        dc(blend_state(eq((M::Sub, F::SrcAlphaSaturate, F::InvDstAlpha), (M::Add, F::One, F::Zero))), tris(18, 6)),
    ];
    check_job("gl_blendsub", &jobs[0], job([0.0, 0.0, 0.0, 1.0], window(), false), &draws, &std_attrs());
    // each draw has its own BLEND_CFG (the comparison above is of the last
    // one before each draw: this is that there are four)
    assert_eq!(named(&jobs[0], "BLEND_CFG").len(), 4, "one BLEND_CFG per draw");
    // the KNOWN_WORD_GAPS entry: no glBlendColor in this shape, the
    // constant colour is 0 wherever Mesa sends it
    let cc = named(&jobs[0], "BLEND_CCOLOR");
    assert!(!cc.is_empty(), "the dump has BLEND_CCOLOR");
    assert!(cc.iter().all(|p| p.fields.iter().all(|(_, v)| v == "0")), "the blend colour is 0");
}

/// `Depth-Test Function` and `Z updates enable` of the `CFG_BITS` each draw
/// of a job sees, as the dump prints them.
fn dump_depth_cfg(j: &ClifJob) -> Vec<(String, String)> {
    let field = |p: &Pk, l: &str| p.fields.iter().find(|f| f.0 == l).unwrap_or_else(|| panic!("no {l}")).1.clone();
    states_at_draws(j)
        .iter()
        .map(|s| (field(&s["CFG_BITS"], "Depth-Test Function"), field(&s["CFG_BITS"], "Z updates enable")))
        .collect()
}

fn depthfuncs_zs() -> Zs {
    Zs {
        kind: frame71::DepthKind71::D24,
        attach: Some((43, false, false, true, false)),
        z_clear: 0.5,
        stencil_clear: 0,
        early_ds_clear: false,
    }
}

/// gl_shapes.py depthfuncs(): the depth test on; six functions with the
/// depth mask on, then LESS with it off. `equal_writes` is what the EQUAL
/// draw's `depth_write` is given as.
fn depthfuncs_draws(equal_writes: bool) -> Vec<DrawCall> {
    let funcs = [
        (Compare::Never, true),
        (Compare::Equal, equal_writes),
        (Compare::LEqual, true),
        (Compare::Greater, true),
        (Compare::NotEqual, true),
        (Compare::Always, true),
        (Compare::Less, false),
    ];
    funcs
        .iter()
        .enumerate()
        .map(|(i, (f, w))| {
            let mut d = gl();
            d.depth_test = true;
            d.depth_func = *f;
            d.depth_write = *w;
            dc(d, tris(6 * i as u64, 6))
        })
        .collect()
}

#[test]
fn gl_depthfuncs() {
    let jobs = gl_jobs("gl_depthfuncs");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_depthfuncs", &jobs);
    // the reference itself: the seven functions in the shape's order, Z
    // updates on for all but EQUAL (see gap_depth_equal_z_updates) and the
    // last draw (glDepthMask(GL_FALSE))
    let want = [("0", "1"), ("2", "0"), ("3", "1"), ("4", "1"), ("5", "1"), ("7", "1"), ("1", "0")];
    let got = dump_depth_cfg(&jobs[0]);
    let got: Vec<(&str, &str)> = got.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(got, want, "gl_depthfuncs.clif: (Depth-Test Function, Z updates enable) per draw");
    // with the EQUAL draw's depth write given as off (what Mesa packs), every
    // compared packet of the job is the modules'
    check_job(
        "gl_depthfuncs",
        &jobs[0],
        job([0.0, 0.0, 0.0, 1.0], depthfuncs_zs(), false),
        &depthfuncs_draws(false),
        &std_attrs(),
    );
}

/// GAP (draw71, `CFG_BITS` / `Z updates enable` under GL_EQUAL): with the GL
/// state as the application set it (depth mask on, glDepthFunc(GL_EQUAL)),
/// Mesa packs `Z updates enable` 0 (gl_depthfuncs.clif, the second draw);
/// `draw71::translate` packs 1. A passing EQUAL test writes the value
/// already there, so the pixels are the same; the word differs.
#[test]
#[ignore = "draw71 packs Z updates enable 1 for GL_EQUAL with the depth mask on; Mesa packs 0 (gl_depthfuncs.clif draw 2)"]
fn gap_depth_equal_z_updates() {
    let jobs = gl_jobs("gl_depthfuncs");
    check_job(
        "gl_depthfuncs as GL sets it",
        &jobs[0],
        job([0.0, 0.0, 0.0, 1.0], depthfuncs_zs(), false),
        &depthfuncs_draws(true),
        &std_attrs(),
    );
}

fn floats(j: &ClifJob, name: &str) -> Vec<f32> {
    named(j, name).iter().map(|p| p.fields[0].1.parse().unwrap()).collect()
}

#[test]
fn gl_primswide_points_and_wide_lines() {
    let jobs = gl_jobs("gl_primswide");
    assert_eq!(jobs.len(), 1);
    assert_no_silent_kinds("gl_primswide", &jobs);
    let draws = [
        dc(gl(), prim71::Draw::arrays(prim71::GL_POINTS, 0, 4)),
        dc(gl(), prim71::Draw::arrays(prim71::GL_LINES, 4, 4)),
        dc(gl(), prim71::Draw::arrays(prim71::GL_LINES, 8, 4)),
        dc(gl(), prim71::Draw::arrays(prim71::GL_LINE_STRIP, 12, 4)),
    ];
    check_job("gl_primswide", &jobs[0], job([0.0, 0.0, 0.0, 1.0], window(), false), &draws, &std_attrs());
    // the KNOWN_WORD_GAPS entries, as the reference has them: one LINE_WIDTH
    // and one POINT_SIZE before each draw; the width is glLineWidth's (1.0
    // at the points), the POINT_SIZE packet stays 1.0 because the 16.0 is
    // gl_PointSize's, written by the vertex shader
    assert_eq!(floats(&jobs[0], "LINE_WIDTH"), [1.0, 2.0, 4.0, 2.0], "gl_primswide.clif: Line width per draw");
    assert_eq!(floats(&jobs[0], "POINT_SIZE"), [1.0; 4], "gl_primswide.clif: Point Size per draw");
}

// ---- the gaps -------------------------------------------------------------------

const SHAPES: [&str; 3] = ["gl_blendsub", "gl_depthfuncs", "gl_primswide"];

#[test]
fn known_gaps_are_real_and_explained() {
    let jobs: Vec<ClifJob> = SHAPES.into_iter().flat_map(parse_jobs).collect();
    for g in KNOWN_WORD_GAPS {
        assert!(g.reason.len() > 20, "{}: a reason", g.packet);
        assert!(
            jobs.iter().any(|j| j.lists.iter().any(|p| p.name == g.packet)),
            "{} is in none of the three dumps: drop the gap",
            g.packet
        );
        // a gap is not a packet a module writes
        assert!(!draw71::ALL_SPECS.iter().any(|s| s.clif_name == g.packet), "{} is modelled", g.packet);
    }
}

fn assert_modelled(packet: &str) {
    assert!(
        draw71::ALL_SPECS.iter().any(|s| s.clif_name == packet),
        "draw71::ALL_SPECS has no PacketSpec for {packet}: the reference's packet is not compared"
    );
}

/// GAP (draw71, `LINE_WIDTH`): gl_primswide.clif has Line width 2.0, 4.0 and
/// 2.0 at its three line draws; draw71 has no packet for it (the width goes
/// from fill71's `DrawIn::line_width` to the C job's state).
#[test]
#[ignore = "draw71 has no PacketSpec for LINE_WIDTH (gl_primswide.clif: 2.0, 4.0, 2.0)"]
fn gap_line_width_packet() {
    assert_modelled("LINE_WIDTH");
}

/// GAP (draw71, `POINT_SIZE`): gl_primswide.clif has Point Size 1.0 at each
/// draw (the 16.0 is the shader's gl_PointSize, which the 7.1 table refuses:
/// backend71's "a vertex program writing gl_PointSize").
#[test]
#[ignore = "draw71 has no PacketSpec for POINT_SIZE (gl_primswide.clif: 1.0 at each draw)"]
fn gap_point_size_packet() {
    assert_modelled("POINT_SIZE");
}

/// GAP (draw71, `BLEND_CCOLOR`): gl_blendsub.clif sends the constant colour
/// (0 here); draw71 has no packet for it (fill71 passes the F16 words to
/// the C job's state).
#[test]
#[ignore = "draw71 has no PacketSpec for BLEND_CCOLOR (gl_blendsub.clif: four zero F16 words)"]
fn gap_blend_ccolor_packet() {
    assert_modelled("BLEND_CCOLOR");
}
