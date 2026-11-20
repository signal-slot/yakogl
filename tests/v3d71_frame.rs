// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/frame71.rs` (the V3D 7.1 frame-level packets) against Mesa
//! 26.2.1's `cle/v3d_packet.xml` and the reference dumps in
//! tools/v3d/pi5/.
//!
//! * every packet's opcode, body length, field positions and widths, and
//!   which fields are stored minus one, equal the XML's 7.1 definition
//!   (parsed live, by text);
//! * the limits (8 render targets, 7680 px, 256 supertiles) and the
//!   Store/Load buffer numbers are anchored in the XML, the README and
//!   Mesa's v3d_limits.h;
//! * every frame of the Mesa 7.1 dumps (clear, triangle and the eight gl_*
//!   shapes: tools/v3d/pi5/*.clif; the `.stdout` files carry the renderer
//!   and the framebuffer size, not the packets) is rebuilt as a
//!   [`Frame71`] from its own binning and render list, and the plan's
//!   packets, packed to bytes and decoded by the XML's positions, equal
//!   every value the dump prints;
//! * every refusal has an anchor.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_frame
//! ```
#[path = "../src/v3d/frame71.rs"]
mod frame71;

use frame71::*;
use std::collections::BTreeSet;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ---- the XML ------------------------------------------------------------

struct XmlField {
    name: String,
    start: u32,
    size: u32,
    minus_one: bool,
}

struct XmlPacket {
    code: u32,
    fields: Vec<XmlField>,
    /// the text between the tags (enum values)
    body: String,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let pat = format!(" {name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')?;
    Some(tag[i..i + j].to_string())
}

/// The 7.1 definition of the packet called `name`: no `max_ver` under 71 and
/// no `min_ver` over 71. Exactly one must exist.
fn xml_packet(xml: &str, name: &str) -> XmlPacket {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<packet ") {
        rest = &rest[i..];
        let tag_end = rest.find('>').unwrap();
        let tag = &rest[..tag_end];
        let selfclosed = tag.ends_with('/');
        let body_end = if selfclosed { tag_end } else { rest.find("</packet>").unwrap() };
        let ok_ver = attr(tag, "max_ver").map_or(true, |v| v.parse::<u32>().unwrap() >= 71)
            && attr(tag, "min_ver").map_or(true, |v| v.parse::<u32>().unwrap() <= 71);
        if attr(tag, "name").as_deref() == Some(name) && ok_ver {
            let body = &rest[tag_end..body_end];
            let mut fields = Vec::new();
            let mut b = body;
            while let Some(k) = b.find("<field ") {
                b = &b[k..];
                let e = b.find('>').unwrap();
                let ft = &b[..e];
                fields.push(XmlField {
                    name: attr(ft, "name").unwrap(),
                    start: attr(ft, "start").unwrap().parse().unwrap(),
                    size: attr(ft, "size").unwrap().parse().unwrap(),
                    minus_one: attr(ft, "minus_one").as_deref() == Some("true"),
                });
                b = &b[e..];
            }
            found.push(XmlPacket { code: attr(tag, "code").unwrap().parse().unwrap(), fields, body: body.to_string() });
        }
        rest = &rest[tag_end..];
    }
    assert_eq!(found.len(), 1, "one 7.1 definition of {name}");
    found.pop().unwrap()
}

/// The fields of a spec that the 7.1 XML stores minus one.
fn minus_one(spec: &Spec, xml: &str) -> BTreeSet<&'static str> {
    let x = xml_packet(xml, spec.name);
    spec.fields.iter().filter(|f| x.fields.iter().any(|g| g.name == f.name && g.minus_one)).map(|f| f.name).collect()
}

#[test]
fn every_spec_is_the_xml_definition() {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    for spec in SPECS.iter() {
        let x = xml_packet(&xml, spec.name);
        assert_eq!(x.code, spec.code as u32, "{}: opcode", spec.name);
        assert_eq!(x.fields.len(), spec.fields.len(), "{}: field count", spec.name);
        for g in &x.fields {
            let f = spec
                .fields
                .iter()
                .find(|f| f.name == g.name)
                .unwrap_or_else(|| panic!("{}: field {} missing", spec.name, g.name));
            assert_eq!((f.start, f.size), (g.start, g.size), "{}: {}", spec.name, g.name);
        }
        let bits = x.fields.iter().map(|g| g.start + g.size).max().unwrap();
        assert_eq!(spec.body as u32, bits.div_ceil(8), "{}: body length", spec.name);
    }
    for (name, code) in SIMPLE {
        let x = xml_packet(&xml, name);
        assert_eq!(x.code, code as u32, "{name}");
        assert!(x.fields.is_empty(), "{name}: no fields");
    }
    assert_eq!(OP_END_RENDER, 13);
    assert_eq!(OP_RETURN, 18);
    assert_eq!(OP_CLEAR_VCD_CACHE, 19);
    assert_eq!(OP_CLEAR_RT, 25);
    assert_eq!(OP_END_LOADS, 26);
    assert_eq!(OP_END_TILE, 27);
    assert_eq!(OP_IMPLICIT_TILE_COORDS, 125);
}

#[test]
fn minus_one_fields_are_the_xml_ones() {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let want = |names: &[&'static str]| names.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(minus_one(&BINNING_CFG, &xml), want(&["Height (in pixels)", "Width (in pixels)"]));
    assert_eq!(minus_one(&RENDERING_COMMON, &xml), want(&["Number of Render Targets"]));
    assert_eq!(
        minus_one(&SUPERTILE_CFG, &xml),
        want(&["Number of Bin Tile Lists", "Supertile Height in Tiles", "Supertile Width in Tiles"])
    );
    assert_eq!(minus_one(&NUMBER_OF_LAYERS, &xml), want(&["Number of Layers"]));
    for s in
        [&ZS_CLEAR_VALUES, &INITIAL_BLOCK_SIZE, &TILE_LIST_SET_BASE, &TILE_COORDS, &SUPERTILE_COORDS, &STORE, &LOAD]
    {
        assert!(minus_one(s, &xml).is_empty(), "{}", s.name);
    }
}

#[test]
fn limits_and_numbers_are_anchored() {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    // the Store packet names RT 0-7 at 7.1 (4-7 carry min_ver="71"); the
    // Load packet names RT 0-3 only
    let store = xml_packet(&xml, STORE.name);
    for n in 0..8 {
        assert!(store.body.contains(&format!("name=\"Render target {n}\" value=\"{n}\"")), "store RT{n}");
    }
    for n in 4..8 {
        assert!(
            store.body.contains(&format!("name=\"Render target {n}\" value=\"{n}\" min_ver=\"71\"")),
            "store RT{n} is 7.1"
        );
    }
    assert_eq!(MAX_RTS, 8);
    let load = xml_packet(&xml, LOAD.name);
    for n in 0..4 {
        assert!(load.body.contains(&format!("name=\"Render target {n}\" value=\"{n}\"")));
    }
    assert!(!load.body.contains("Render target 4"), "the Load names no RT above 3");
    for (name, v) in
        [("None", BUFFER_NONE), ("Z", BUFFER_Z), ("Stencil", BUFFER_STENCIL), ("Z+Stencil", BUFFER_ZSTENCIL)]
    {
        let pat = format!("name=\"{name}\" value=\"{v}\"");
        assert!(store.body.contains(&pat) && load.body.contains(&pat), "{name}");
    }
    // Output Image Format d24s8, the Z buffer choice
    assert!(xml.lines().any(|l| l.contains("name=\"d24s8\"") && l.contains(&format!("value=\"{FORMAT_D24S8}\""))));
    // Internal Depth Type
    for (n, v) in [("depth_32f", 0), ("depth_24", 1), ("depth_16", 2)] {
        assert!(xml.contains(&format!("<value name=\"{n}\" value=\"{v}\"/>")), "{n}");
    }
    assert_eq!(DepthKind71::D24.internal_type(), 1);
    assert_eq!(DepthKind71::StencilOnly.internal_type(), 1);
    assert_eq!(DepthKind71::D16.internal_type(), 2);
    // the binning block sizes
    assert!(xml.contains("name=\"tile allocation block size 64b\" value=\"0\""));
    assert!(xml.contains("name=\"tile allocation block size 128b\" value=\"1\""));
    // Mesa's supertile limit and the framebuffer limit
    let lim = read("third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h");
    assert!(lim.contains(&format!("#define V3D_MAX_SUPERTILES {MAX_SUPERTILES}")));
    let readme = read("tools/v3d/pi5/README.md");
    assert!(readme.contains(&format!("framebuffer {MAX_SIZE}")), "README: framebuffer 7680");
    // Clear Render Targets replaces Clear Tile Buffers (max_ver 42): no
    // fields, 7.1 only
    assert!(xml.contains("name=\"Clear Render Targets\" cl=\"R\" min_ver=\"71\"/>"));
}

// ---- packing ------------------------------------------------------------

fn get(bytes: &[u8], spec: &Spec, name: &str) -> u64 {
    assert_eq!(bytes[0], spec.code);
    assert_eq!(bytes.len(), spec.body + 1, "{}: length", spec.name);
    let f = spec.fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{}: {name}", spec.name));
    let mut v = 0u64;
    for i in 0..f.size {
        let bit = f.start + i;
        if bytes[1 + (bit / 8) as usize] >> (bit % 8) & 1 != 0 {
            v |= 1 << i;
        }
    }
    v
}

/// Every bit of the body belongs to a field: pack with all fields at their
/// maximum and count the set bits.
#[test]
fn a_full_packet_sets_exactly_its_field_bits() {
    let surf = Surface71 {
        format: 63,
        memory_format: 5,
        height_in_ub_or_stride: (1 << 20) - 1,
        address: u32::MAX,
        r_b_swap: true,
        channel_reverse: true,
        flip_y: true,
        height: 0xffff,
        decimate: 3,
        dither: 0,
        force_alpha_1: false,
    };
    let st = Transfer71 { buffer: 11, surface: surf }.pack_store().unwrap();
    let ones: u32 = st[1..].iter().map(|b| b.count_ones()).sum();
    // all fields but "Clear buffer being stored" (18), Dither (2 bits, 0 here)
    // and Memory Format's top bit
    let want: u32 = STORE
        .fields
        .iter()
        .filter(|f| !["Clear buffer being stored", "Dither Mode"].contains(&f.name))
        .map(|f| f.size)
        .sum();
    // Memory Format 5 (0b101) and buffer 11 (0b1011) are not all ones
    assert_eq!(ones, want - 2, "store bits");
    assert_eq!(get(&st, &STORE, "Clear buffer being stored"), 0);
    let mut ld = surf;
    ld.force_alpha_1 = true;
    let l = Transfer71 { buffer: 11, surface: ld }.pack_load().unwrap();
    let ones: u32 = l[1..].iter().map(|b| b.count_ones()).sum();
    let want: u32 = LOAD.fields.iter().map(|f| f.size).sum();
    assert_eq!(ones, want - 2);
}

#[test]
fn store_never_sets_the_clear_bit() {
    for fmt in [0u8, 27, 43, 63] {
        for dither in 0..4 {
            let t =
                Transfer71 { buffer: BUFFER_RT0, surface: Surface71 { format: fmt, dither, ..Surface71::default() } };
            let b = t.pack_store().unwrap();
            assert_eq!(get(&b, &STORE, "Clear buffer being stored"), 0);
        }
    }
}

// ---- the reference dumps --------------------------------------------------

#[derive(Debug, Clone)]
struct Pk {
    name: String,
    fields: Vec<(String, String)>,
}

impl Pk {
    fn val(&self, label: &str) -> &str {
        &self.fields.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("{}: {label}", self.name)).1
    }
    fn num(&self, label: &str) -> u32 {
        let v = self.val(label);
        v.strip_prefix("0x").map_or_else(|| v.parse().unwrap(), |h| u32::from_str_radix(h, 16).unwrap())
    }
    fn addr(&self) -> u32 {
        let v = &self.fields.iter().find(|(l, _)| l == "Address").unwrap().1;
        match v.find("/* 0x") {
            Some(i) => u32::from_str_radix(&v[i + 5..i + 13], 16).unwrap(),
            None => 0,
        }
    }
}

fn parse_clif(name: &str) -> Vec<Pk> {
    let text = read(&format!("tools/v3d/pi5/{name}.clif"));
    let mut out: Vec<Pk> = Vec::new();
    for line in text.lines() {
        let l = line.strip_prefix("MESA: info: ").unwrap_or(line);
        if !l.is_empty()
            && !l.starts_with(' ')
            && !l.starts_with('@')
            && l.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            out.push(Pk { name: l.to_string(), fields: Vec::new() });
        } else if l.starts_with("  /*") {
            let Some(p) = out.last_mut() else { continue };
            let inner = l.trim_start().strip_prefix("/*").unwrap();
            let (label, rest) = inner.split_once(": */").unwrap();
            let rest = rest.trim();
            let val = if rest.starts_with('[') {
                rest.to_string()
            } else {
                rest.split_whitespace().next().unwrap_or("").to_string()
            };
            // keep the whole text for addresses, the first token otherwise
            p.fields.push((label.trim().to_string(), if label.trim() == "Address" { rest.to_string() } else { val }));
        }
    }
    out
}

/// The frames of a dump. A job's three lists (binner, render, generic tile
/// list) are three buffers that a dump writes in no fixed order, so the
/// k-th of each belongs to the k-th frame: the binning config packet, the
/// render list (`TILE_RENDERING_MODE_CFG_COMMON` to `END_RENDER`) and the
/// tile list (`IMPLICIT_TILE_COORDS` to `RETURN`), in that order.
fn frames(pks: &[Pk]) -> Vec<Vec<Pk>> {
    let (mut bins, mut renders, mut lists): (Vec<Pk>, Vec<Vec<Pk>>, Vec<Vec<Pk>>) = (vec![], vec![], vec![]);
    let mut open: Option<(bool, &str)> = None; // (is render, closing packet)
    for p in pks {
        match p.name.as_str() {
            "TILE_BINNING_MODE_CFG" => bins.push(p.clone()),
            "TILE_RENDERING_MODE_CFG_COMMON" => {
                renders.push(Vec::new());
                open = Some((true, "END_RENDER"));
            }
            "IMPLICIT_TILE_COORDS" => {
                lists.push(Vec::new());
                open = Some((false, "RETURN"));
            }
            _ => {}
        }
        if let Some((is_render, close)) = open {
            let v = if is_render { renders.last_mut() } else { lists.last_mut() }.unwrap();
            v.push(p.clone());
            if p.name == close {
                open = None;
            }
        }
    }
    assert!(bins.len() == renders.len() && bins.len() == lists.len(), "one binner, render and tile list per frame");
    bins.into_iter().zip(renders).zip(lists).map(|((b, r), l)| std::iter::once(b).chain(r).chain(l).collect()).collect()
}

fn surface_of(p: &Pk, load: bool) -> Surface71 {
    Surface71 {
        format: p.num(if load { "Input Image Format" } else { "Output Image Format" }) as u8,
        memory_format: p.num("Memory Format") as u8,
        height_in_ub_or_stride: p.num("Height in UB or Stride"),
        address: p.addr(),
        r_b_swap: p.num("R/B swap") != 0,
        channel_reverse: p.num("Channel Reverse") != 0,
        flip_y: p.num("Flip Y") != 0,
        height: p.num("Height"),
        decimate: p.num("Decimate mode") as u8,
        dither: if load { 0 } else { p.num("Dither Mode") as u8 },
        force_alpha_1: load && p.num("Force alpha 1") != 0,
    }
}

fn transfer_of(p: &Pk) -> Transfer71 {
    let load = p.name == "LOAD";
    Transfer71 {
        buffer: p.num(if load { "Buffer to Load" } else { "Buffer to Store" }) as u8,
        surface: surface_of(p, load),
    }
}

fn spec_of(name: &str) -> &'static Spec {
    match name {
        "TILE_BINNING_MODE_CFG" => &BINNING_CFG,
        "TILE_RENDERING_MODE_CFG_COMMON" => &RENDERING_COMMON,
        "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES" => &ZS_CLEAR_VALUES,
        "TILE_LIST_INITIAL_BLOCK_SIZE" => &INITIAL_BLOCK_SIZE,
        "MULTICORE_RENDERING_SUPERTILE_CFG" => &SUPERTILE_CFG,
        "TILE_COORDS" => &TILE_COORDS,
        "SUPERTILE_COORDS" => &SUPERTILE_COORDS,
        "STORE" => &STORE,
        "LOAD" => &LOAD,
        _ => panic!("no spec for {name}"),
    }
}

/// Every value the dump prints for `p` equals the field of `bytes` (the
/// packed packet), through the XML's own positions, minus-one fields
/// included.
fn assert_bytes_match_dump(bytes: &[u8], p: &Pk, xml: &str) {
    let spec = spec_of(&p.name);
    let m1 = minus_one(spec, xml);
    for (label, value) in &p.fields {
        if label == "Address" {
            assert_eq!(get(bytes, spec, "Address") as u32, p.addr(), "{}", p.name);
            continue;
        }
        let got = get(bytes, spec, label) + m1.contains(label.as_str()) as u64;
        let want =
            if label == "Z Clear Value" { value.parse::<f32>().unwrap().to_bits() as u64 } else { p.num(label) as u64 };
        let got = if label == "Z Clear Value" { got - m1.contains(label.as_str()) as u64 } else { got };
        assert_eq!(got, want, "{} / {label}", p.name);
    }
}

fn to_op(p: &Pk) -> Option<Op71> {
    Some(match p.name.as_str() {
        "TILE_COORDS" => Op71::TileCoords { column: p.num("tile column number"), row: p.num("tile row number") },
        "IMPLICIT_TILE_COORDS" => Op71::ImplicitTileCoords,
        "END_LOADS" => Op71::EndLoads,
        "LOAD" => Op71::Load(transfer_of(p)),
        "STORE" => Op71::Store(transfer_of(p)),
        "CLEAR_RT" => Op71::ClearRt,
        "END_TILE" => Op71::EndTile,
        "SUPERTILE_COORDS" => Op71::SupertileCoords {
            column: p.num("column number in supertiles"),
            row: p.num("row number in supertiles"),
        },
        _ => return None,
    })
}

/// The generic tile list: from its `IMPLICIT_TILE_COORDS` on (the buffer
/// sits before or after the render list in a dump, with the draws' own
/// packets in it).
fn tile_list(fr: &[Pk]) -> &[Pk] {
    let first = fr.iter().position(|p| p.name == "IMPLICIT_TILE_COORDS").unwrap();
    &fr[first..]
}

/// Rebuild the frame description from a dump frame: what the dump shows of
/// the frame, nothing of the plan.
fn describe(fr: &[Pk]) -> Frame71 {
    let bin = &fr[0];
    let common = fr.iter().find(|p| p.name == "TILE_RENDERING_MODE_CFG_COMMON").unwrap();
    let zs_clear = fr.iter().find(|p| p.name == "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES").unwrap();
    let list = tile_list(&fr);
    let end_loads = list.iter().position(|p| p.name == "END_LOADS").unwrap();
    let branch = list.iter().position(|p| p.name == "BRANCH_IMPLICIT_TILE").unwrap();
    let loads: Vec<&Pk> = list[..end_loads].iter().filter(|p| p.name == "LOAD").collect();
    let tail = &list[branch..];
    let end_tile = tail.iter().position(|p| p.name == "END_TILE").unwrap();
    let stores: Vec<&Pk> = tail[..end_tile].iter().filter(|p| p.name == "STORE").collect();
    let clear = tail[..end_tile].iter().any(|p| p.name == "CLEAR_RT");

    let mut f = Frame71::new(
        common.num("Image Width (pixels)"),
        common.num("Image Height (pixels)"),
        8 << common.num("Log2 Tile Width"),
        8 << common.num("Log2 Tile Height"),
    );
    assert_eq!(bin.num("Width (in pixels)"), f.width);
    assert_eq!(bin.num("Height (in pixels)"), f.height);
    f.render_targets = common.num("Number of Render Targets");
    f.msaa = common.num("Multisample Mode (4x)") != 0;
    f.double_buffer = common.num("Double-buffer in non-ms mode") != 0;
    f.depth_buffer_disable = common.num("Depth-buffer disable") != 0;
    f.early_z_gt = common.num("Early-Z Test and Update Direction") != 0;
    f.early_z_disable = common.num("Early-Z disable") != 0;
    f.early_ds_clear = common.num("Early Depth/Stencil Clear") != 0;
    f.clear_tile_buffers = clear;
    f.z_clear_bits = zs_clear.val("Z Clear Value").parse::<f32>().unwrap().to_bits();
    f.stencil_clear = zs_clear.num("Stencil Clear Value") as u8;

    // the buffers the dump's loads and stores name; a buffer that is only
    // loaded is invalidated at the end
    let depth_type = common.num("Internal Depth Type");
    let mut all: Vec<(&Pk, bool)> = loads.iter().map(|p| (*p, true)).collect();
    all.extend(stores.iter().map(|p| (*p, false)));
    for (p, is_load) in all {
        let t = transfer_of(p);
        if matches!(t.buffer, BUFFER_Z | BUFFER_ZSTENCIL | BUFFER_STENCIL) && t.surface.format != FORMAT_S8 {
            // the depth surface: the parts the buffer number names
            let (z, s) = (t.buffer != BUFFER_STENCIL, t.buffer != BUFFER_Z);
            let a = f.zs.get_or_insert(ZsAttach71 {
                surface: t.surface,
                load_z: false,
                load_s: false,
                store_z: false,
                store_s: false,
            });
            if is_load {
                a.load_z |= z;
                a.load_s |= s;
            } else {
                a.store_z |= z;
                a.store_s |= s;
            }
            continue;
        }
        let slot: &mut Option<Attach71> = match t.buffer {
            0..=7 => &mut f.rt[t.buffer as usize],
            BUFFER_STENCIL => &mut f.stencil,
            BUFFER_NONE => continue,
            b => panic!("buffer {b}"),
        };
        match slot {
            Some(a) => {
                if is_load {
                    a.load = true;
                    a.surface.force_alpha_1 = t.surface.force_alpha_1;
                } else {
                    a.end = EndOp71::Store;
                }
            }
            None => {
                *slot = Some(Attach71 {
                    surface: t.surface,
                    load: is_load,
                    end: if is_load { EndOp71::Invalidate } else { EndOp71::Store },
                })
            }
        }
    }
    let stencil_only = f.zs.is_some_and(|z| !z.load_z && !z.store_z);
    f.depth_kind = match (depth_type, f.zs.is_some(), f.stencil.is_some()) {
        _ if stencil_only => DepthKind71::StencilOnly,
        (0, true, _) => DepthKind71::D32F,
        (0, false, false) => DepthKind71::None,
        (1, _, _) => DepthKind71::D24,
        (2, _, _) => DepthKind71::D16,
        t => panic!("depth type {t:?}"),
    };
    // a depth buffer the frame neither loads nor stores is invisible in the
    // dump: attach an invalidated one so the description agrees with the
    // common packet's depth type
    if matches!(f.depth_kind, DepthKind71::D24 | DepthKind71::D16 | DepthKind71::D32F) && f.zs.is_none() {
        f.zs = Some(ZsAttach71 {
            surface: Surface71::default(),
            load_z: false,
            load_s: false,
            store_z: false,
            store_s: false,
        });
    }
    f
}

const SHAPES: [&str; 10] = [
    "clear",
    "triangle",
    "gl_scissor",
    "gl_scisbox",
    "gl_writemask",
    "gl_viewport",
    "gl_fbods",
    "gl_colormask",
    "gl_mrt4",
    "gl_mrt8",
];

#[test]
fn every_dump_frame_is_rebuilt_from_its_description() {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let mut total = 0;
    for shape in SHAPES {
        let pks = parse_clif(shape);
        let fs = frames(&pks);
        assert!(!fs.is_empty(), "{shape}: frames");
        // the .stdout names the renderer and the framebuffer size
        let out = read(&format!("tools/v3d/pi5/{shape}.stdout"));
        assert!(out.contains("renderer V3D 7.1.10.2"), "{shape}: renderer");
        let size = out.split("framebuffer ").nth(1).unwrap().split(',').next().unwrap().to_string();
        for (k, fr) in fs.iter().enumerate() {
            let what = format!("{shape} frame {k}");
            let d = describe(&fr);
            assert_eq!(format!("{}x{}", d.width, d.height), size, "{what}: size");
            let plan = plan(&d).unwrap_or_else(|e| panic!("{what}: {}", e.message()));
            let get1 = |n: &str| fr.iter().find(|p| p.name == n).unwrap();
            assert_bytes_match_dump(&plan.binning.pack().unwrap(), get1("TILE_BINNING_MODE_CFG"), &xml);
            assert_bytes_match_dump(&plan.common.pack().unwrap(), get1("TILE_RENDERING_MODE_CFG_COMMON"), &xml);
            assert_bytes_match_dump(
                &plan.zs_clear.pack().unwrap(),
                get1("TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES"),
                &xml,
            );
            assert_bytes_match_dump(&plan.initial_block.pack().unwrap(), get1("TILE_LIST_INITIAL_BLOCK_SIZE"), &xml);
            assert_bytes_match_dump(
                &plan.supertile_cfg.pack().unwrap(),
                get1("MULTICORE_RENDERING_SUPERTILE_CFG"),
                &xml,
            );

            // the render list between the supertile cfg and the generic list
            let cfg = fr.iter().position(|p| p.name == "MULTICORE_RENDERING_SUPERTILE_CFG").unwrap();
            let gen = fr.iter().position(|p| p.name == "GENERIC_TILE_LIST").unwrap();
            let initial: Vec<Op71> = fr[cfg + 1..gen].iter().filter_map(to_op).collect();
            assert_eq!(plan.initial_tiles, initial, "{what}: initial tiles");
            let end = fr.iter().position(|p| p.name == "END_RENDER").unwrap();
            let supers: Vec<Op71> = fr[gen..end].iter().filter_map(to_op).collect();
            assert_eq!(plan.supertiles, supers, "{what}: supertile coordinates");

            // the generic list
            let list = tile_list(&fr);
            let el = list.iter().position(|p| p.name == "END_LOADS").unwrap();
            let loads: Vec<Op71> = list[..el].iter().filter(|p| p.name == "LOAD").filter_map(to_op).collect();
            assert_eq!(plan.list_loads, loads, "{what}: loads");
            let br = list.iter().position(|p| p.name == "BRANCH_IMPLICIT_TILE").unwrap();
            let et = list.iter().position(|p| p.name == "END_TILE").unwrap();
            let stores: Vec<Op71> = list[br..=et].iter().filter_map(to_op).collect();
            assert_eq!(plan.list_stores, stores, "{what}: stores");
            // and the packed bytes equal the dump, packet by packet
            for (op, p) in plan.list_loads.iter().zip(list[..el].iter().filter(|p| p.name == "LOAD")) {
                assert_bytes_match_dump(&op.pack().unwrap(), p, &xml);
            }
            for (op, p) in plan.list_stores.iter().zip(list[br..=et].iter().filter(|p| p.name == "STORE")) {
                // a plan with the NONE store has the same packet
                assert_bytes_match_dump(&op.pack().unwrap(), p, &xml);
            }
            for op in plan.initial_tiles.iter().zip(fr[cfg + 1..gen].iter().filter(|p| to_op(p).is_some())) {
                if let Op71::TileCoords { .. } | Op71::Store(_) = op.0 {
                    assert_bytes_match_dump(&op.0.pack().unwrap(), op.1, &xml);
                }
            }
            for (op, p) in plan.supertiles.iter().zip(fr[gen..end].iter().filter(|p| p.name == "SUPERTILE_COORDS")) {
                assert_bytes_match_dump(&op.pack().unwrap(), p, &xml);
            }
            total += 1;
        }
    }
    // clear 1, triangle 1, scissor 2, scisbox 3, writemask 4, viewport 3,
    // fbods 9, colormask 6, mrt4 1, mrt8 1
    assert_eq!(total, 31);
}

#[test]
fn the_dumps_cover_the_variants_the_plan_must_answer() {
    // at least: stencil-only, depth_16, depth_24, loaded colour, 4 and 8
    // render targets, a 2x2 supertile frame, a 1x2 one
    let mut seen = BTreeSet::new();
    for shape in SHAPES {
        let pks = parse_clif(shape);
        for fr in frames(&pks) {
            let d = describe(&fr);
            let p = plan(&d).unwrap();
            seen.insert(format!("kind {:?}", d.depth_kind));
            seen.insert(format!("rts {}", d.render_targets));
            seen.insert(format!("loads {}", p.list_loads.len()));
            seen.insert(format!("tiles {}x{}", d.tile_width, d.tile_height));
            seen.insert(format!(
                "supertiles {}x{}",
                p.supertile_cfg.frame_width_supertiles, p.supertile_cfg.frame_height_supertiles
            ));
        }
    }
    for want in [
        "kind None",
        "kind D24",
        "kind D16",
        "kind StencilOnly",
        "rts 1",
        "rts 4",
        "rts 8",
        "loads 0",
        "loads 1",
        "tiles 64x64",
        "tiles 64x32",
        "tiles 32x32",
        "supertiles 1x1",
        "supertiles 1x2",
        "supertiles 2x2",
    ] {
        assert!(seen.contains(want), "{want} not in {seen:?}");
    }
}

// ---- the plan's own rules --------------------------------------------------

#[test]
fn supertile_growth_is_mesas() {
    assert_eq!(choose_supertile(1, 1), (1, 1, 1, 1));
    assert_eq!(choose_supertile(2, 2), (1, 1, 2, 2));
    // 16 x 16 = 256 supertiles is the most that stays at 1x1
    assert_eq!(choose_supertile(16, 16), (1, 1, 16, 16));
    // 17 x 17 tiles: 289 supertiles; height grows first (sw is not < sh)
    let (sw, sh, fw, fh) = choose_supertile(17, 17);
    assert_eq!((sw, sh), (1, 2));
    assert_eq!((fw, fh), (17, 9));
    // the largest frame: 960 x 960 tiles of 8 px
    let (sw, sh, fw, fh) = choose_supertile(960, 960);
    assert!(fw < MAX_SUPERTILES && fh < MAX_SUPERTILES && fw * fh <= MAX_SUPERTILES);
    assert!(sw * fw >= 960 && sh * fh >= 960);
}

#[test]
fn stores_loads_and_the_clear() {
    let mut f = Frame71::new(64, 64, 64, 64);
    let rt = |addr| Attach71 {
        surface: Surface71 {
            format: 27,
            memory_format: 4,
            height_in_ub_or_stride: 8,
            address: addr,
            ..Surface71::default()
        },
        load: false,
        end: EndOp71::Store,
    };
    f.rt[0] = Some(rt(0x1000));
    let p = plan(&f).unwrap();
    // one store, the clear and the end of the tile: 7.1's list has no
    // clear bit in the store
    assert_eq!(p.list_stores.len(), 3);
    assert_eq!(p.list_stores[1], Op71::ClearRt);
    assert_eq!(p.list_stores[2], Op71::EndTile);
    assert_eq!(p.list_stores[1].pack().unwrap(), vec![OP_CLEAR_RT]);
    // no clear: the list ends at the store
    f.clear_tile_buffers = false;
    assert_eq!(plan(&f).unwrap().list_stores.len(), 2);
    // an invalidated target: no store, so buffer NONE
    f.rt[0].as_mut().unwrap().end = EndOp71::Invalidate;
    let p = plan(&f).unwrap();
    assert!(matches!(p.list_stores[0], Op71::Store(Transfer71 { buffer: BUFFER_NONE, .. })));
    // a loaded and stored target: one load, one store, loads first
    f.rt[0].as_mut().unwrap().end = EndOp71::Store;
    f.rt[0].as_mut().unwrap().load = true;
    let p = plan(&f).unwrap();
    assert_eq!(p.list_loads.len(), 1);
    // the double-buffer initial clear is in both dummy tiles
    f.double_buffer = true;
    let p = plan(&f).unwrap();
    assert_eq!(p.initial_tiles.iter().filter(|o| **o == Op71::ClearRt).count(), 2);
    f.double_buffer = false;
    let p = plan(&f).unwrap();
    assert_eq!(p.initial_tiles.iter().filter(|o| **o == Op71::ClearRt).count(), 1);

    // Z and stencil: the buffer number follows the parts in use
    let surf = Surface71 {
        format: FORMAT_D24S8,
        memory_format: 4,
        height_in_ub_or_stride: 8,
        address: 0x2000,
        ..Surface71::default()
    };
    let mut zs = ZsAttach71 { surface: surf, load_z: true, load_s: true, store_z: true, store_s: true };
    f.depth_kind = DepthKind71::D24;
    f.zs = Some(zs);
    let p = plan(&f).unwrap();
    let bufs = |v: &[Op71]| {
        v.iter()
            .filter_map(|o| match o {
                Op71::Load(t) | Op71::Store(t) => Some(t.buffer),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(bufs(&p.list_loads), vec![BUFFER_RT0, BUFFER_ZSTENCIL]);
    assert_eq!(bufs(&p.list_stores), vec![BUFFER_RT0, BUFFER_ZSTENCIL]);
    // depth alone is Z, stencil alone Stencil (a D24S8 surface either way)
    zs.load_s = false;
    zs.store_s = false;
    f.zs = Some(zs);
    let p = plan(&f).unwrap();
    assert_eq!(bufs(&p.list_loads), vec![BUFFER_RT0, BUFFER_Z]);
    assert_eq!(bufs(&p.list_stores), vec![BUFFER_RT0, BUFFER_Z]);
    f.depth_kind = DepthKind71::StencilOnly;
    f.zs = Some(ZsAttach71 { load_z: false, store_z: false, load_s: true, store_s: true, ..zs });
    let p = plan(&f).unwrap();
    assert_eq!(bufs(&p.list_stores), vec![BUFFER_RT0, BUFFER_STENCIL]);
    // a separate stencil surface: loads Stencil first, stores it last
    f.depth_kind = DepthKind71::D32F;
    f.zs = Some(zs);
    let sep = Attach71 { surface: Surface71 { format: FORMAT_S8, ..surf }, load: true, end: EndOp71::Store };
    f.stencil = Some(sep);
    let p = plan(&f).unwrap();
    assert_eq!(bufs(&p.list_loads), vec![BUFFER_RT0, BUFFER_STENCIL, BUFFER_Z]);
    assert_eq!(bufs(&p.list_stores), vec![BUFFER_RT0, BUFFER_Z, BUFFER_STENCIL]);
}

#[test]
fn tile_list_base_and_layers() {
    let b = pack_tile_list_base(0x1_5000, 0).unwrap();
    assert_eq!(get(&b, &TILE_LIST_SET_BASE, "address"), 0x1_5000 >> 6);
    assert_eq!(get(&b, &TILE_LIST_SET_BASE, "Tile List Set Number"), 0);
    let l = pack_number_of_layers(1).unwrap();
    assert_eq!(get(&l, &NUMBER_OF_LAYERS, "Number of Layers"), 0);
    let g = Op71::GenericTileList { start: 0x21000, end: 0x2101b }.pack().unwrap();
    assert_eq!(get(&g, &GENERIC_TILE_LIST, "start"), 0x21000);
    assert_eq!(get(&g, &GENERIC_TILE_LIST, "end"), 0x2101b);
    assert_eq!(Op71::BranchImplicitTile { set: 0 }.pack().unwrap(), vec![21, 0]);
    // the one-byte packets are the XML's opcodes
    for (op, code) in [
        (Op71::ClearVcdCache, OP_CLEAR_VCD_CACHE),
        (Op71::EndRender, OP_END_RENDER),
        (Op71::ReturnFromSubList, OP_RETURN),
        (Op71::ClearRt, OP_CLEAR_RT),
        (Op71::EndLoads, OP_END_LOADS),
        (Op71::EndTile, OP_END_TILE),
        (Op71::ImplicitTileCoords, OP_IMPLICIT_TILE_COORDS),
    ] {
        assert_eq!(op.pack().unwrap(), vec![code]);
    }
}

// ---- refusals ----------------------------------------------------------------

fn base() -> Frame71 {
    Frame71::new(64, 64, 64, 64)
}

fn refused(f: &Frame71) -> Refusal71 {
    plan(f).expect_err("refused")
}

#[test]
fn every_refusal_has_an_anchor() {
    let mut f = base();
    f.width = 0;
    assert_eq!(refused(&f), Refusal71::EmptyFrame);
    let mut f = base();
    f.height = MAX_SIZE + 1;
    assert_eq!(refused(&f), Refusal71::FrameTooLarge);
    // the limit itself is accepted
    let mut f = base();
    f.width = MAX_SIZE;
    f.height = MAX_SIZE;
    f.tile_width = 8;
    f.tile_height = 8;
    assert!(plan(&f).is_ok());
    let mut f = base();
    f.render_targets = MAX_RTS + 1;
    assert_eq!(refused(&f), Refusal71::TooManyRenderTargets);
    let mut f = base();
    f.tile_width = 128;
    assert_eq!(refused(&f), Refusal71::TileSize);
    let mut f = base();
    f.tile_height = 24;
    assert_eq!(refused(&f), Refusal71::TileSize);
    let mut f = base();
    f.msaa = true;
    f.double_buffer = true;
    assert_eq!(refused(&f), Refusal71::MsaaDoubleBuffer);
    let mut f = base();
    f.depth_kind = DepthKind71::D16;
    assert_eq!(refused(&f), Refusal71::DepthKind);
    let mut f = base();
    f.zs =
        Some(ZsAttach71 { surface: Surface71::default(), load_z: false, load_s: false, store_z: true, store_s: false });
    assert_eq!(refused(&f), Refusal71::DepthKind);
    // a separate stencil next to a stencil part of the depth surface
    let mut f = base();
    f.depth_kind = DepthKind71::D24;
    f.zs =
        Some(ZsAttach71 { surface: Surface71::default(), load_z: false, load_s: false, store_z: true, store_s: true });
    f.stencil = Some(Attach71 { surface: Surface71::default(), load: false, end: EndOp71::Store });
    assert_eq!(refused(&f), Refusal71::DepthKind);
    // stencil-only with a depth part
    let mut f = base();
    f.depth_kind = DepthKind71::StencilOnly;
    f.zs =
        Some(ZsAttach71 { surface: Surface71::default(), load_z: false, load_s: false, store_z: true, store_s: false });
    assert_eq!(refused(&f), Refusal71::DepthKind);
    let mut f = base();
    f.render_targets = 8;
    f.rt[5] = Some(Attach71 { surface: Surface71::default(), load: true, end: EndOp71::Store });
    assert_eq!(refused(&f), Refusal71::LoadRenderTarget);
    // a store of RT 5 is fine
    f.rt[5].as_mut().unwrap().load = false;
    assert!(plan(&f).is_ok());
    // an attachment past the render-target count
    let mut f = base();
    f.rt[3] = Some(Attach71 { surface: Surface71::default(), load: false, end: EndOp71::Store });
    assert_eq!(refused(&f), Refusal71::TooManyRenderTargets);
    // field ranges
    let mut f = base();
    f.rt[0] = Some(Attach71 {
        surface: Surface71 { memory_format: 6, ..Surface71::default() },
        load: false,
        end: EndOp71::Store,
    });
    assert_eq!(refused(&f), Refusal71::FieldRange("Memory Format"));
    f.rt[0].as_mut().unwrap().surface = Surface71 { decimate: 2, ..Surface71::default() };
    assert_eq!(refused(&f), Refusal71::FieldRange("Decimate mode"));
    f.rt[0].as_mut().unwrap().surface = Surface71 { height_in_ub_or_stride: 1 << 20, ..Surface71::default() };
    assert_eq!(refused(&f), Refusal71::FieldRange("Height in UB or Stride"));
    f.rt[0].as_mut().unwrap().surface = Surface71 { format: 64, ..Surface71::default() };
    assert_eq!(refused(&f), Refusal71::FieldRange("Output Image Format"));
    // packet-level
    assert_eq!(pack_tile_list_base(0x1004, 0), Err(Refusal71::Unaligned));
    let dither = Transfer71 { buffer: 0, surface: Surface71 { dither: 1, ..Surface71::default() } };
    assert_eq!(dither.pack_load(), Err(Refusal71::WrongPacketField));
    assert!(dither.pack_store().is_ok());
    let alpha = Transfer71 { buffer: 0, surface: Surface71 { force_alpha_1: true, ..Surface71::default() } };
    assert_eq!(alpha.pack_store(), Err(Refusal71::WrongPacketField));
    assert!(alpha.pack_load().is_ok());
}

#[test]
fn refusal_messages_are_named() {
    for r in [
        Refusal71::EmptyFrame,
        Refusal71::FrameTooLarge,
        Refusal71::TooManyRenderTargets,
        Refusal71::TileSize,
        Refusal71::MsaaDoubleBuffer,
        Refusal71::DepthKind,
        Refusal71::LoadRenderTarget,
        Refusal71::FieldRange("x"),
        Refusal71::Unaligned,
        Refusal71::WrongPacketField,
    ] {
        assert!(r.message().starts_with("V3D 7.1: "), "{r:?}");
    }
}
