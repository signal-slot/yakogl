// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/draw71.rs` (the V3D 7.1 per-draw state packets) against Mesa
//! 26.2.1 and against the packets Mesa emitted for the six gl_test shapes.
//!
//! * Every packet's opcode, field names, positions, widths and types equal
//!   `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`'s definition
//!   for version 71, read by text; the lengths and opcodes equal the
//!   generated 7.1 packer's (`driver/src/v3d/generated/`, read only); the
//!   enums equal the XML's.
//! * Each pack function round-trips: the bytes, read back through the
//!   layout, give the struct's values; hand-computed byte anchors.
//! * For each shape of tools/v3d/pi5 (gl_scissor 5, gl_scisbox 58,
//!   gl_writemask 59, gl_viewport 66, gl_colormask 102, gl_mrt8): the GL
//!   state of the shape's draws is built from tools/v3d/pi5/src/gl_shapes.py
//!   (the calls it makes are checked present in that file's source), and the
//!   distinct values of every packet kind the module gives equal the
//!   distinct values the `.clif` dump decoded for that kind. (The `.stdout`
//!   files hold only the renderer and glGetError lines; the decoded packets
//!   are in the `.clif` files.)
//! * Every refusal has an anchor.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_draw
//! ```
#[path = "../src/v3d/draw71.rs"]
mod draw71;

use draw71::*;
use std::collections::BTreeSet;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const XML: &str = "third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml";
const PACKER: &str = "driver/src/v3d/generated/v3d_packet_v71_pack.h";

// ---- v3d_packet.xml by text ------------------------------------------------

/// The value of attribute `key` in one XML tag line.
fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!(" {key}=\"");
    let at = line.find(&pat)? + pat.len();
    let end = line[at..].find('"')?;
    Some(&line[at..at + end])
}

/// Whether a tag's min_ver/max_ver reach version 71.
fn is_71(line: &str) -> bool {
    attr(line, "min_ver").map_or(true, |v| v.parse::<u32>().unwrap() <= 71)
        && attr(line, "max_ver").map_or(true, |v| v.parse::<u32>().unwrap() >= 71)
}

/// (code, [(name, start, size, type)]) of the `<packet name=xml_name>` that
/// applies at 7.1.
fn xml_packet(xml: &str, xml_name: &str) -> (u32, Vec<(String, u32, u32, String)>) {
    let mut found = None;
    let mut lines = xml.lines().peekable();
    while let Some(l) = lines.next() {
        let t = l.trim();
        if !t.starts_with("<packet ") || attr(t, "name") != Some(xml_name) || !is_71(t) {
            continue;
        }
        assert!(found.is_none(), "{xml_name}: two 7.1 definitions");
        let code: u32 = attr(t, "code").unwrap().parse().unwrap();
        let mut fields = vec![];
        if !t.ends_with("/>") {
            for f in lines.by_ref() {
                let f = f.trim();
                if f.starts_with("</packet>") {
                    break;
                }
                if f.starts_with("<field ") {
                    fields.push((
                        attr(f, "name").unwrap().to_string(),
                        attr(f, "start").unwrap().parse().unwrap(),
                        attr(f, "size").unwrap().parse().unwrap(),
                        attr(f, "type").unwrap().to_string(),
                    ));
                }
            }
        }
        found = Some((code, fields));
    }
    found.unwrap_or_else(|| panic!("{xml_name}: no 7.1 definition"))
}

fn xml_enum(xml: &str, name: &str) -> Vec<(String, u32)> {
    let open = format!("<enum name=\"{name}\"");
    let mut out = vec![];
    let mut inside = false;
    for l in xml.lines() {
        let t = l.trim();
        if t.starts_with(&open) {
            inside = true;
        } else if inside && t.starts_with("</enum>") {
            return out;
        } else if inside && t.starts_with("<value ") {
            out.push((attr(t, "name").unwrap().to_string(), attr(t, "value").unwrap().parse().unwrap()));
        }
    }
    panic!("{name}: no enum");
}

#[test]
fn every_packet_layout_is_the_xml_71_definition() {
    let xml = read(XML);
    for spec in ALL_SPECS {
        let (code, fields) = xml_packet(&xml, spec.xml_name);
        assert_eq!(u32::from(spec.code), code, "{}: opcode", spec.xml_name);
        let ours: Vec<(String, u32, u32, String)> =
            spec.fields.iter().map(|f| (f.name.to_string(), f.start, f.size, f.ty.to_string())).collect();
        let mut a = ours.clone();
        let mut b = fields.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "{}: fields (name, start, size, type)", spec.xml_name);
    }
}

#[test]
fn lengths_and_opcodes_are_the_generated_71_packers() {
    let h = read(PACKER);
    let lines: Vec<&str> = h.lines().collect();
    for spec in ALL_SPECS {
        let len_def = format!("#define V3D71_{}_length", spec.c_name);
        let len: usize = lines
            .iter()
            .find(|l| l.starts_with(&len_def) && l[len_def.len()..].starts_with(|c: char| c == ' ' || c == '\t'))
            .unwrap_or_else(|| panic!("{}: no length", spec.c_name))
            .split_whitespace()
            .last()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(spec.length(), len, "{}: length", spec.c_name);
        let hdr = format!("#define V3D71_{}_header", spec.c_name);
        let at = lines
            .iter()
            .position(|l| l.starts_with(&hdr) && !l[hdr.len()..].starts_with('_'))
            .unwrap_or_else(|| panic!("{}: no header", spec.c_name));
        let opc = lines[at + 1].split('=').nth(1).unwrap().trim().trim_end_matches(',');
        assert_eq!(opc.parse::<u8>().unwrap(), spec.code, "{}: opcode", spec.c_name);
    }
}

#[test]
fn enums_are_the_xml_enums() {
    let xml = read(XML);
    let check = |name: &str, ours: Vec<(&str, u8)>, complete: bool| {
        let theirs = xml_enum(&xml, name);
        for (n, v) in &ours {
            assert!(
                theirs.iter().any(|(tn, tv)| tn == n && *tv == u32::from(*v)),
                "{name}: {n} = {v} is not in the XML"
            );
        }
        if complete {
            assert_eq!(ours.len(), theirs.len(), "{name}: a value is missing");
        }
    };
    check("Compare Function", COMPARES.iter().map(|(e, n)| (*n, *e as u8)).collect(), true);
    check("Blend Factor", BLEND_FACTORS.iter().map(|(e, n)| (*n, *e as u8)).collect(), true);
    // GL ES has no MUL, SCREEN, DARKEN or LIGHTEN equation
    check("Blend Mode", BLEND_MODES.iter().map(|(e, n)| (*n, *e as u8)).collect(), false);
    check("Stencil Op", STENCIL_OPS.iter().map(|(e, n)| (*n, *e as u8)).collect(), true);
    check("Z Clip Mode", Z_CLIP_MODES.iter().map(|(e, n)| (*n, *e as u8)).collect(), true);
}

#[test]
fn seventy_one_differs_from_42_where_the_notes_say() {
    let xml = read(XML);
    let (_, cfg) = xml_packet(&xml, "Cfg Bits");
    let names: Vec<&str> = cfg.iter().map(|f| f.0.as_str()).collect();
    assert!(!names.contains(&"Early Z enable") && !names.contains(&"Early Z updates enable"));
    assert!(names.contains(&"Z Clipping mode"));
    let e = early_z(FragProgram { writes_z: true, discards: false });
    assert!(!e.cfg_bits_has_early_z_fields && e.tile_mode_early_z_disable);
    let (_, blend) = xml_packet(&xml, "Blend Cfg");
    assert!(blend.contains(&("Render Target Mask".into(), 24, 8, "uint".into())), "8 targets");
    let (_, xy) = xml_packet(&xml, "Clipper XY Scaling");
    assert!(xy.iter().all(|f| f.0.contains("1/64th")), "1/64 pixel");
}

// ---- pack ------------------------------------------------------------------

/// Read a field back out of packed bytes.
fn unpack(bytes: &[u8], f: &Field) -> u64 {
    let mut v = 0u64;
    for b in 0..f.size {
        let at = 8 + f.start + b;
        if bytes[(at / 8) as usize] >> (at % 8) & 1 != 0 {
            v |= 1 << b;
        }
    }
    v
}

fn bits_of(v: Val, f: &Field) -> u64 {
    match v {
        Val::U(v) => v,
        Val::I(v) => (v as u64) & ((1u64 << f.size) - 1),
        Val::F(v) => u64::from(v.to_bits()),
    }
}

fn roundtrip<P: Packet>(p: &P, bytes: &[u8]) {
    assert_eq!(bytes.len(), P::SPEC.length());
    assert_eq!(bytes[0], P::SPEC.code);
    let pairs = p.pairs();
    assert_eq!(pairs.len(), P::SPEC.fields.len(), "{}: every field once", P::SPEC.xml_name);
    for f in P::SPEC.fields {
        assert_eq!(pairs.iter().filter(|(g, _)| g.name == f.name).count(), 1, "{}", f.name);
    }
    for (f, v) in pairs {
        assert_eq!(unpack(bytes, f), bits_of(v, f), "{}: {}", P::SPEC.xml_name, f.name);
    }
}

const GL_W: u32 = 64;

fn fbo(nr: u32) -> DrawState {
    DrawState::gl_default(GL_W, GL_W, false, nr)
}

#[test]
fn pack_roundtrips_and_anchors() {
    let mut d = fbo(1);
    d.depth_test = true;
    d.depth_func = Compare::LEqual;
    d.stencil_test = true;
    d.stencil_two_sided = true;
    d.stencil_back.func = Compare::Greater;
    d.stencil_back.reference = 77;
    d.polygon_offset = true;
    d.offset_factor = 2.0;
    d.offset_units = -1.5;
    d.offset_clamp = 0.25;
    d.cull = Cull::Back;
    d.front_cw = true;
    d.independent_blend = true;
    d.targets[0].blend_enable = true;
    d.targets[0].blend.rgb_src = BlendFactor::SrcAlpha;
    d.targets[0].blend.rgb_dst = BlendFactor::InvSrcAlpha;
    d.nr_targets = 8;
    d.targets[7].blend_enable = true;
    d.targets[7].blend.rgb_mode = BlendMode::RSub;
    d.targets[7].color_mask = 0x5;
    d.viewport = [-150.0, -30.0, 200.0, 40.0];
    let p = translate(&d).unwrap();
    roundtrip(&p.clip, &p.clip.pack());
    roundtrip(&p.cfg, &p.cfg.pack());
    roundtrip(&p.depth_offset.unwrap(), &p.depth_offset.unwrap().pack());
    roundtrip(&p.clipper_xy, &p.clipper_xy.pack());
    roundtrip(&p.clipper_z, &p.clipper_z.pack());
    roundtrip(&p.clipper_z_min_max, &p.clipper_z_min_max.pack());
    roundtrip(&p.viewport_offset, &p.viewport_offset.pack());
    roundtrip(&p.blend_enables, &p.blend_enables.pack());
    assert_eq!(p.blend_cfg_count, 2);
    for c in p.blend_cfg.iter().flatten() {
        roundtrip(c, &c.pack());
    }
    roundtrip(&p.color_write_masks, &p.color_write_masks.pack());
    assert_eq!(p.stencil_count, 2);
    for s in p.stencil.iter().flatten() {
        roundtrip(s, &s.pack());
    }
    // blend: target 0 and target 7 only; target 7's mask nibble is 0xa
    assert_eq!(p.blend_enables.mask, 0x81);
    assert_eq!(p.color_write_masks.mask >> 28, 0xa);
    assert_eq!(p.blend_cfg[1].unwrap().render_target_mask, 0x80);
    // a negative translation moves into the coarse block count
    assert_eq!((p.viewport_offset.coarse_x, p.viewport_offset.coarse_y), (-1, -1));
    // anchors, computed by hand from the layouts
    let def = translate(&fbo(1)).unwrap();
    assert_eq!(def.cfg.pack(), [96, 0x03, 0x70, 0x40]);
    assert_eq!(def.clip.pack(), [107, 0, 0, 0, 0, 64, 0, 64, 0]);
    assert_eq!(def.blend_enables.pack(), [83, 0]);
    assert_eq!(def.color_write_masks.pack(), [87, 0, 0, 0, 0]);
    assert_eq!(def.clipper_xy.pack()[1..5], 2048f32.to_bits().to_le_bytes());
    assert_eq!(def.viewport_offset.pack(), [108, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00]);
    assert!(def.depth_offset.is_none() && def.blend_cfg_count == 0 && def.stencil_count == 0);
    // f187 is the top half of the float
    assert_eq!(f187(2.0), 0x4000);
    let dof = p.depth_offset.unwrap();
    assert_eq!((dof.factor_f187, dof.units_f187), (0x4000, f187(-1.5)));
    // translate x = -50: coarse -1, fine 64 - 50 = 14 pixels
    assert_eq!(p.viewport_offset.fine_x, 14 * 256);
}

#[test]
fn state_rules() {
    // facing: culling is for polygons; the window's flip inverts the bit
    let mut d = fbo(1);
    d.cull = Cull::Back;
    let c = translate(&d).unwrap().cfg;
    assert!(c.enable_forward_facing_primitive && !c.enable_reverse_facing_primitive);
    d.polygon = false;
    let c = translate(&d).unwrap().cfg;
    assert!(c.enable_forward_facing_primitive && c.enable_reverse_facing_primitive);
    d.polygon = true;
    d.cull = Cull::Front;
    let c = translate(&d).unwrap().cfg;
    assert!(!c.enable_forward_facing_primitive && c.enable_reverse_facing_primitive);
    d.cull = Cull::FrontAndBack;
    let c = translate(&d).unwrap().cfg;
    assert!(!c.enable_forward_facing_primitive && !c.enable_reverse_facing_primitive);
    let mut w = DrawState::gl_default(GL_W, GL_W, true, 1);
    assert!(translate(&w).unwrap().cfg.clockwise_primitives);
    w.front_cw = true;
    assert!(!translate(&w).unwrap().cfg.clockwise_primitives);
    assert!(!translate(&fbo(1)).unwrap().cfg.clockwise_primitives);
    // depth: the test off is ALWAYS without Z updates
    let mut z = fbo(1);
    z.depth_func = Compare::Greater;
    let c = translate(&z).unwrap().cfg;
    assert_eq!((c.depth_test_function, c.z_updates_enable), (Compare::Always, false));
    z.depth_test = true;
    z.depth_write = false;
    let c = translate(&z).unwrap().cfg;
    assert_eq!((c.depth_test_function, c.z_updates_enable), (Compare::Greater, false));
    // blend without independence: one packet for all 8 targets
    let mut bl = fbo(2);
    bl.targets[0].blend_enable = true;
    bl.targets[0].blend.rgb_mode = BlendMode::Max;
    bl.targets[0].blend.rgb_src = BlendFactor::SrcColor;
    let p = translate(&bl).unwrap();
    assert!(p.cfg.blend_enable);
    assert_eq!((p.blend_enables.mask, p.blend_cfg_count), (0xff, 1));
    let cfg = p.blend_cfg[0].unwrap();
    assert_eq!(cfg.render_target_mask, 0xff);
    // GL ignores the factors of MAX: ONE, ONE
    assert_eq!((cfg.color_blend_src_factor, cfg.color_blend_dst_factor), (BlendFactor::One, BlendFactor::One));
    // stencil: one-sided configures both faces; the reference is clamped
    let mut s = fbo(1);
    s.stencil_test = true;
    s.stencil_front.reference = 300;
    s.stencil_front.value_mask = 0xffff_ff0f;
    let p = translate(&s).unwrap();
    let f = p.stencil[0].unwrap();
    assert!(f.front_config && f.back_config);
    assert_eq!((f.stencil_ref_value, f.stencil_test_mask, f.stencil_write_mask), (255, 0x0f, 0xff));
    // early Z: nothing in the packets, the program's answers are kept
    let e = early_z(FragProgram { writes_z: false, discards: true });
    assert!(e.program_needs_late_z && !e.program_disables_job_early_z);
    let e = early_z(FragProgram { writes_z: true, discards: false });
    assert!(e.program_needs_late_z && e.program_disables_job_early_z && e.tile_mode_early_z_disable);
    let e = early_z(FragProgram::default());
    assert!(!e.program_needs_late_z && e.tile_mode_early_z_disable && !e.cfg_bits_has_early_z_fields);
    // the flip of a window: the scissor box is mirrored, the half height negative
    let mut f = DrawState::gl_default(GL_W, GL_W, true, 1);
    f.scissor = Some([0, 0, 32, 16]);
    let p = translate(&f).unwrap();
    assert_eq!((p.clip.bottom, p.clip.height), (48, 16));
    assert!(p.clipper_xy.half_height_1_64 < 0.0);
}

// ---- refusals --------------------------------------------------------------

#[test]
fn every_refusal_has_an_anchor() {
    let mut seen = BTreeSet::new();
    let mut hit = |d: DrawState, want: Refusal71| {
        assert_eq!(translate(&d).err(), Some(want), "{want:?}");
        seen.insert(format!("{want:?}"));
    };
    let mut d = fbo(1);
    d.target_w = 7681;
    hit(d, Refusal71::TargetSize);
    let mut d = fbo(1);
    d.target_h = 0;
    hit(d, Refusal71::TargetSize);
    hit(fbo(9), Refusal71::TooManyTargets);
    let mut d = fbo(1);
    d.samples = 4;
    hit(d, Refusal71::Multisample);
    let mut d = fbo(1);
    d.viewport[2] = f32::NAN;
    hit(d, Refusal71::ViewportNotFinite);
    let mut d = fbo(1);
    d.viewport[3] = -1.0;
    hit(d, Refusal71::ViewportNotFinite);
    let mut d = fbo(1);
    d.depth_range = [0.5, 0.505];
    hit(d, Refusal71::DepthScaleTooSmall);
    let mut d = fbo(1);
    d.polygon_offset = true;
    d.offset_units = f32::INFINITY;
    hit(d, Refusal71::OffsetNotFinite);
    let mut d = fbo(1);
    d.viewport = [70000.0, 0.0, 10.0, 10.0];
    hit(d, Refusal71::ClipWindowRange);
    let mut d = fbo(1);
    d.viewport = [20000.0, 0.0, 10.0, 10.0];
    hit(d, Refusal71::ViewportOffsetRange);
    let mut d = fbo(1);
    d.targets[0].blend_enable = true;
    d.targets[0].blend.rgb_dst = BlendFactor::DstAlpha;
    d.targets[0].has_alpha = false;
    hit(d, Refusal71::BlendReadsMissingAlpha);
    // the GL enumerants
    assert_eq!(compare_from_gl(0x0200), Ok(Compare::Never));
    assert_eq!(compare_from_gl(0x0207), Ok(Compare::Always));
    for bad in [0x01ff, 0x0208, 0] {
        assert_eq!(compare_from_gl(bad), Err(Refusal71::UnknownCompare));
    }
    assert_eq!(blend_factor_from_gl(0x8003), Ok(BlendFactor::ConstAlpha));
    assert_eq!(blend_factor_from_gl(0x0308), Ok(BlendFactor::SrcAlphaSaturate));
    assert_eq!(blend_factor_from_gl(0x0303), Ok(BlendFactor::InvSrcAlpha));
    assert_eq!(blend_factor_from_gl(0x0309), Err(Refusal71::UnknownBlendFactor)); // GL_SRC1_COLOR is ES-less
    assert_eq!(blend_mode_from_gl(0x800B), Ok(BlendMode::RSub));
    assert_eq!(blend_mode_from_gl(0x8009), Err(Refusal71::UnknownBlendEquation));
    assert_eq!(stencil_op_from_gl(0x8508), Ok(StencilOp::DecWrap));
    assert_eq!(stencil_op_from_gl(0x1E04), Err(Refusal71::UnknownStencilOp));
    for r in [
        Refusal71::UnknownCompare,
        Refusal71::UnknownBlendFactor,
        Refusal71::UnknownBlendEquation,
        Refusal71::UnknownStencilOp,
    ] {
        seen.insert(format!("{r:?}"));
    }
    // every variant was anchored, and every message names the revision
    for r in Refusal71::ALL {
        assert!(seen.contains(&format!("{r:?}")), "{r:?} has no anchor");
        assert!(r.message().starts_with("V3D 7.1: "), "{r:?}");
    }
}

// ---- the six gl_test shapes against the CLIF dumps -------------------------

type Decoded = Vec<(String, String)>;

/// Every packet of a CLIF dump: (name, [(field label, first token of value)]).
fn parse_clif(rel: &str) -> Vec<(String, Decoded)> {
    let mut out: Vec<(String, Decoded)> = vec![];
    for l in read(rel).lines() {
        let Some(rest) = l.strip_prefix("MESA: info:") else { continue };
        let t = rest.trim();
        if t.is_empty() || t.starts_with("@") {
            continue;
        }
        if let Some(body) = t.strip_prefix("/*") {
            let Some(at) = body.find(": */") else { continue };
            let label = body[..at].trim().to_string();
            let value = body[at + 4..].split_whitespace().next().unwrap_or("").to_string();
            if let Some(p) = out.last_mut() {
                p.1.push((label, value));
            }
        } else if t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            out.push((t.to_string(), vec![]));
        }
    }
    out
}

fn sorted(mut d: Decoded) -> Decoded {
    d.sort();
    d
}

fn module_values<P: Packet>(p: &P) -> Decoded {
    sorted(p.pairs().iter().map(|(f, v)| (f.name.to_string(), v.clif_text())).collect())
}

/// kind (CLIF name) -> distinct decoded values of the module's packets.
fn module_sets(draws: &[StatePackets71]) -> Vec<(&'static str, BTreeSet<Decoded>)> {
    let mut sets: Vec<(&'static str, BTreeSet<Decoded>)> =
        ALL_SPECS.iter().map(|s| (s.clif_name, BTreeSet::new())).collect();
    let mut add = |name: &str, v: Decoded| {
        sets.iter_mut().find(|(n, _)| *n == name).unwrap().1.insert(v);
    };
    for p in draws {
        add("CLIP", module_values(&p.clip));
        add("CFG_BITS", module_values(&p.cfg));
        if let Some(o) = &p.depth_offset {
            add("DEPTH_OFFSET", module_values(o));
        }
        add("CLIPPER_XY", module_values(&p.clipper_xy));
        add("CLIPPER_Z", module_values(&p.clipper_z));
        add("CLIPZ", module_values(&p.clipper_z_min_max));
        add("VIEWPORT_OFFSET", module_values(&p.viewport_offset));
        add("BLEND_ENABLES", module_values(&p.blend_enables));
        for c in p.blend_cfg.iter().flatten() {
            add("BLEND_CFG", module_values(c));
        }
        add("COLOR_WMASKS", module_values(&p.color_write_masks));
        for s in p.stencil.iter().flatten() {
            add("STENCIL_CFG", module_values(s));
        }
    }
    sets
}

fn clif_sets(rel: &str) -> Vec<(&'static str, BTreeSet<Decoded>)> {
    let clif = parse_clif(rel);
    ALL_SPECS
        .iter()
        .map(|s| {
            let set =
                clif.iter().filter(|(n, _)| n == s.clif_name).map(|(_, v)| sorted(v.clone())).collect::<BTreeSet<_>>();
            (s.clif_name, set)
        })
        .collect()
}

/// The body of `def <name>(self):` in gl_shapes.py.
fn shape_source(name: &str) -> String {
    let py = read("tools/v3d/pi5/src/gl_shapes.py");
    let head = format!("    def {name}(self):");
    let at = py.find(&head).unwrap_or_else(|| panic!("gl_shapes.py has no {name}"));
    let rest = &py[at + head.len()..];
    let end = rest.find("\n    def ").unwrap_or(rest.len());
    rest[..end].to_string()
}

fn check_shape(shape: &str, calls: &[&str], draws: &[DrawState]) {
    let src = shape_source(shape);
    for c in calls {
        assert!(src.contains(c), "{shape}: gl_shapes.py no longer has {c}");
    }
    let packets: Vec<StatePackets71> = draws.iter().map(|d| translate(d).unwrap()).collect();
    let ours = module_sets(&packets);
    let theirs = clif_sets(&format!("tools/v3d/pi5/gl_{shape}.clif"));
    for ((n, a), (m, b)) in ours.iter().zip(theirs.iter()) {
        assert_eq!(n, m);
        assert_eq!(a, b, "gl_{shape}.clif: the distinct {n} packets");
    }
}

#[test]
fn case_5_scissor() {
    // job 2: the clear drawn as a quad (the box is its geometry), then the
    // yellow quad under glScissor(0, 0, 32, 32)
    let clear = DrawState::blitter_clear(64, 64, false, 1, 0xf, None);
    let mut quad = fbo(1);
    quad.scissor = Some([0, 0, 32, 32]);
    check_shape(
        "scissor",
        &["gl.glEnable(GL_SCISSOR_TEST)", "gl.glScissor(0, 0, 32, 32)", "gl.glDrawArrays(GL_TRIANGLES, 0, 6)"],
        &[clear, quad],
    );
    let p = translate(&quad).unwrap();
    assert_eq!(p.clip.pack(), [107, 0, 0, 0, 0, 32, 0, 32, 0]);
}

#[test]
fn case_58_clear_with_scissor() {
    // jobs 1 to 3: a green quad and the scissored clears (quads over the
    // whole target); the empty box (16, 16, 0, 32) is a scissor of no
    // pixels: the clip window is 0 x 0, which Mesa never prints because its
    // clear quad is empty
    let full = fbo(1);
    let clear = DrawState::blitter_clear(64, 64, false, 1, 0xf, None);
    check_shape(
        "scisbox",
        &["gl.glScissor(16, 16, 0 if step == 2 else 32, 32)", "gl.glDrawArrays(GL_TRIANGLES, 0, 6)"],
        &[full, clear],
    );
    let mut empty = fbo(1);
    empty.scissor = Some([16, 16, 0, 32]);
    let c = translate(&empty).unwrap().clip;
    assert_eq!((c.width, c.height), (0, 0));
    let mut boxed = fbo(1);
    boxed.scissor = Some([16, 16, 32, 32]);
    let c = translate(&boxed).unwrap().clip;
    assert_eq!((c.left, c.bottom, c.width, c.height), (16, 16, 32, 32));
}

#[test]
fn case_59_clear_with_write_masks() {
    // the red depth-tested quad (GL_LESS, depth writes on), the stencil
    // clear drawn as a quad (colour mask 0, stencil write mask 0x0f, value
    // 0xff), the quads testing EQUAL 0xff and EQUAL 0x0f
    let mut red = fbo(1);
    red.depth_test = true;
    red.depth_func = Compare::Less;
    let clear = DrawState::blitter_clear(64, 64, false, 1, 0, Some((0xff, 0x0f)));
    let mut eq_ff = fbo(1);
    eq_ff.stencil_test = true;
    eq_ff.stencil_front.func = Compare::Equal;
    eq_ff.stencil_front.reference = 0xff;
    eq_ff.stencil_front.value_mask = 0xff;
    eq_ff.stencil_front.write_mask = 0xff;
    let mut eq_0f = eq_ff;
    eq_0f.stencil_front.reference = 0x0f;
    check_shape(
        "writemask",
        &[
            "gl.glColorMask(F, F, F, F)",
            "gl.glDepthFunc(GL_LESS)",
            "gl.glStencilMask(0x0f)",
            "gl.glStencilFunc(GL_EQUAL, 0xff, 0xff)",
            "gl.glStencilFunc(GL_EQUAL, 0x0f, 0xff)",
        ],
        &[red, clear, eq_ff, eq_0f],
    );
    // the all-off colour mask is the packet's 0xffffffff
    let all_off = translate(&DrawState::blitter_clear(64, 64, false, 1, 0, None)).unwrap();
    assert_eq!(all_off.color_write_masks.mask, 0xffff_ffff);
}

#[test]
fn case_66_clear_not_bounded_by_viewport() {
    // the one draw of the file is the scissored green clear: a quad over
    // the whole target, in the full viewport state; the GL viewport of the
    // quarter-size clear is not in any packet of it
    let clear = DrawState::blitter_clear(64, 64, false, 1, 0xf, None);
    check_shape(
        "viewport",
        &["gl.glViewport(0, 0, TW // 2, TH // 2)", "gl.glScissor(0, 0, TW // 2, TH // 2)"],
        &[clear],
    );
    // a real draw under the quarter viewport is bounded by it
    let mut q = fbo(1);
    q.viewport = [0.0, 0.0, 32.0, 32.0];
    let p = translate(&q).unwrap();
    assert_eq!((p.clip.width, p.clip.height), (32, 32));
    assert_eq!(p.clipper_xy.half_width_1_64, 1024.0);
    assert_eq!((p.viewport_offset.fine_x, p.viewport_offset.fine_y), (16 * 256, 16 * 256));
    // the clear is not
    assert_eq!(translate(&clear).unwrap().clip.width, 64);
}

#[test]
fn case_102_clear_under_partial_colour_mask() {
    // R+A, G, B masked clears drawn as quads, and the yellow quad with
    // every channel on
    let ra = DrawState::blitter_clear(64, 64, false, 1, 0b1001, None);
    let g = DrawState::blitter_clear(64, 64, false, 1, 0b0010, None);
    let bl = DrawState::blitter_clear(64, 64, false, 1, 0b0100, None);
    let all = fbo(1);
    check_shape(
        "colormask",
        &[
            "gl.glColorMask(T, F, F, T)",
            "gl.glColorMask(F, T, F, F)",
            "gl.glColorMask(F, F, T, F)",
            "gl.glScissor(0, 0, 32, 32)",
        ],
        &[ra, g, bl, all],
    );
    // 8 targets: the one mask fills all eight nibbles
    let p = translate(&DrawState::blitter_clear(64, 64, false, 8, 0b1001, None)).unwrap();
    assert_eq!(p.color_write_masks.mask, 0x6666_6666);
}

#[test]
fn mrt8_eight_targets_and_no_state_packets() {
    let src = shape_source("mrt8");
    assert!(src.contains("self.mrt(8)"));
    assert!(read("tools/v3d/pi5/src/gl_shapes.py").contains("gl.glDrawBuffers(n, bufs)"));
    // the job has no draw: none of this module's packets is in the dump, but
    // the common mode config names 8 targets
    let clif = parse_clif("tools/v3d/pi5/gl_mrt8.clif");
    for (n, set) in clif_sets("tools/v3d/pi5/gl_mrt8.clif") {
        assert!(set.is_empty(), "gl_mrt8.clif holds a {n}");
    }
    let rts: Vec<&str> = clif
        .iter()
        .filter(|(n, _)| n == "TILE_RENDERING_MODE_CFG_COMMON")
        .flat_map(|(_, f)| f.iter())
        .filter(|(l, _)| l == "Number of Render Targets")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(rts, ["8"]);
    // the module draws 8 targets, each with its own mask and blend bit
    let mut d = fbo(8);
    d.independent_blend = true;
    for i in 0..8 {
        d.targets[i].color_mask = i as u8 + 1;
        d.targets[i].blend_enable = i % 2 == 1;
    }
    let p = translate(&d).unwrap();
    assert_eq!(p.blend_enables.mask, 0xaa);
    assert_eq!(p.blend_cfg_count, 4);
    assert_eq!(
        p.color_write_masks.mask,
        0xf & !1
            | (0xf & !2) << 4
            | (0xf & !3) << 8
            | (0xf & !4) << 12
            | (0xf & !5) << 16
            | (0xf & !6) << 20
            | (0xf & !7) << 24
            | (0xf & !8) << 28
    );
    assert_eq!(translate(&fbo(9)).err(), Some(Refusal71::TooManyTargets));
}
