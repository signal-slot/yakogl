// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The V3D 7.1 stencil-op state (src/v3d/draw71.rs) against Mesa's 7.1
//! reference tools/v3d/pi5/gl_stencilops.clif, read as text:
//!
//! * per draw of the binning list, the `STENCIL_CFG` records Mesa emits
//!   (front and back: function, reference, test and write masks, the
//!   sfail/zfail/zpass ops, which faces each record configures) against
//!   `draw71::translate` for the same GL state, field by field and in order;
//! * the `CFG_BITS` of each draw (its `Stencil enable` in particular);
//! * the face slots the records leave in the hardware (front, back) after
//!   each draw, against the emitter's;
//! * the symbolic names the dump prints (`/* LEQUAL */`, `/* INCWRAP */`)
//!   against draw71's `COMPARES` and `STENCIL_OPS` tables, so the GL
//!   enumerants go through `compare_from_gl` / `stencil_op_from_gl` to the
//!   codes Mesa uses.
//!
//! The GL calls are the `stencilops` shape of tools/v3d/pi5/src/gl_shapes.py:
//! a 64x64 framebuffer object with a D24S8 renderbuffer, the depth test
//! (GL_LESS) on, then
//!
//! 1. glStencilFunc(GL_LEQUAL, 0x20, 0x3f), glStencilOp(GL_INCR, GL_DECR,
//!    GL_INVERT), glStencilMask(0x0f): one quad;
//! 2. through the Separate entry points, front ops (REPLACE, KEEP,
//!    INCR_WRAP), back ops (ZERO, DECR_WRAP, INVERT), func NOTEQUAL 0x05
//!    mask 0xf0 on both faces, write mask 0xff: one quad.
//!
//! tests/v3d71_compose4.rs compares the same dump packet by packet through
//! the composed modules; this file pins the stencil records alone, from the
//! GL enumerants.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_stencilops`

#![allow(dead_code)]

#[path = "../src/v3d/draw71.rs"]
mod draw71;

use draw71::{Compare, DrawState, Packet, StatePackets71, StencilFace, StencilOp};

const CLIF: &str = include_str!("../tools/v3d/pi5/gl_stencilops.clif");

// ---- the dump ---------------------------------------------------------------

/// One packet of the dump: its name and, per `/* label: */ value` line, the
/// label, the value's first token and the symbolic name in the trailing
/// comment (`3 /* LEQUAL */` gives `("3", Some("LEQUAL"))`).
#[derive(Clone, Debug)]
struct Pk {
    name: String,
    fields: Vec<(String, String, Option<String>)>,
}

impl Pk {
    fn get(&self, label: &str) -> &(String, String, Option<String>) {
        self.fields
            .iter()
            .find(|f| f.0 == label)
            .unwrap_or_else(|| panic!("{}: no field '{label}' in the dump", self.name))
    }
    fn value(&self, label: &str) -> &str {
        &self.get(label).1
    }
    fn symbol(&self, label: &str) -> Option<&str> {
        self.get(label).2.as_deref()
    }
}

/// The packets of the binning control list (the first `@buffer` holding a
/// `TILE_BINNING_MODE_CFG`), in file order.
fn binning_list() -> Vec<Pk> {
    let mut lists: Vec<Vec<Pk>> = vec![];
    let mut cur: Option<Vec<Pk>> = None;
    let mut in_ctrl = false;
    for raw in CLIF.lines() {
        let line = raw.strip_prefix("MESA: info: ").unwrap_or(raw);
        let t = line.trim();
        if t.starts_with("@buffer") {
            if let Some(l) = cur.take() {
                lists.push(l);
            }
            cur = Some(vec![]);
            in_ctrl = false;
            continue;
        }
        if t.starts_with("@format") {
            in_ctrl = t.starts_with("@format ctrllist");
            continue;
        }
        if !in_ctrl || t.is_empty() || t.starts_with('@') {
            continue;
        }
        let Some(list) = cur.as_mut() else { continue };
        if let Some(rest) = t.strip_prefix("/*") {
            let (label, after) = rest.split_once(":*/").or_else(|| rest.split_once(": */")).expect("a field line");
            let after = after.trim();
            let (value, sym) = match after.split_once("/*") {
                Some((v, s)) => (v.trim().to_string(), Some(s.trim_end_matches("*/").trim().to_string())),
                None => (after.to_string(), None),
            };
            let value = value.split_whitespace().next().unwrap_or("").to_string();
            list.last_mut().expect("a field after a packet").fields.push((label.trim().to_string(), value, sym));
        } else {
            list.push(Pk { name: t.to_string(), fields: vec![] });
        }
    }
    if let Some(l) = cur.take() {
        lists.push(l);
    }
    lists
        .into_iter()
        .find(|l| l.iter().any(|p| p.name == "TILE_BINNING_MODE_CFG"))
        .expect("the dump has a binning list")
}

const DRAW_PACKETS: [&str; 4] =
    ["VERTEX_ARRAY_PRIMS", "INDEXED_PRIM_LIST", "VERTEX_ARRAY_INSTANCED_PRIMS", "INDEXED_INSTANCED_PRIM_LIST"];

/// What the dump sets up for one draw: the `CFG_BITS` in effect (Mesa emits
/// it only when it changes) and the `STENCIL_CFG` records emitted since the
/// previous draw.
struct DumpDraw {
    cfg_bits: Pk,
    stencil: Vec<Pk>,
}

fn dump_draws() -> Vec<DumpDraw> {
    let mut out = vec![];
    let mut cfg: Option<Pk> = None;
    let mut stencil = vec![];
    for p in binning_list() {
        match p.name.as_str() {
            "CFG_BITS" => cfg = Some(p),
            "STENCIL_CFG" => stencil.push(p),
            n if DRAW_PACKETS.contains(&n) => out.push(DumpDraw {
                cfg_bits: cfg.clone().expect("CFG_BITS before the first draw"),
                stencil: std::mem::take(&mut stencil),
            }),
            _ => {}
        }
    }
    assert!(stencil.is_empty(), "STENCIL_CFG after the last draw");
    out
}

// ---- the GL state of the two draws -------------------------------------------

const GL_LESS: u32 = 0x0201;
const GL_LEQUAL: u32 = 0x0203;
const GL_NOTEQUAL: u32 = 0x0205;
const GL_ZERO: u32 = 0;
const GL_KEEP: u32 = 0x1E00;
const GL_REPLACE: u32 = 0x1E01;
const GL_INCR: u32 = 0x1E02;
const GL_DECR: u32 = 0x1E03;
const GL_INVERT: u32 = 0x150A;
const GL_INCR_WRAP: u32 = 0x8507;
const GL_DECR_WRAP: u32 = 0x8508;

/// glStencilFunc(func, reference, value_mask), glStencilOp(sfail, zfail,
/// zpass), glStencilMask(write_mask) for one face, as GL enumerants.
fn face(func: u32, reference: u32, value_mask: u32, ops: [u32; 3], write_mask: u32) -> StencilFace {
    let op = |gl| draw71::stencil_op_from_gl(gl).expect("a GL stencil op");
    StencilFace {
        func: draw71::compare_from_gl(func).expect("a GL compare function"),
        sfail: op(ops[0]),
        zfail: op(ops[1]),
        zpass: op(ops[2]),
        reference,
        value_mask,
        write_mask,
    }
}

/// The framebuffer object of gl_shapes.py (not flipped), one colour target,
/// glEnable(GL_DEPTH_TEST) with glDepthFunc(GL_LESS), glEnable(GL_STENCIL_TEST).
fn base() -> DrawState {
    let mut s = DrawState::gl_default(64, 64, false, 1);
    s.depth_test = true;
    s.depth_func = draw71::compare_from_gl(GL_LESS).unwrap();
    s.stencil_test = true;
    s
}

/// The two draws' GL state. The second is two-sided: the GL state tracker
/// (st/mesa `st_update_depth_stencil_alpha`) enables the back face's own
/// state when the faces differ, as glue71 does.
fn gl_draws() -> [DrawState; 2] {
    let mut a = base();
    a.stencil_front = face(GL_LEQUAL, 0x20, 0x3f, [GL_INCR, GL_DECR, GL_INVERT], 0x0f);
    a.stencil_back = a.stencil_front;
    a.stencil_two_sided = false;

    let mut b = base();
    b.stencil_front = face(GL_NOTEQUAL, 0x05, 0xf0, [GL_REPLACE, GL_KEEP, GL_INCR_WRAP], 0xff);
    b.stencil_back = face(GL_NOTEQUAL, 0x05, 0xf0, [GL_ZERO, GL_DECR_WRAP, GL_INVERT], 0xff);
    b.stencil_two_sided = b.stencil_front != b.stencil_back;
    [a, b]
}

fn emitted(s: &DrawState) -> StatePackets71 {
    draw71::translate(s).unwrap_or_else(|r| panic!("translate refused: {}", r.message()))
}

fn stencil_records(p: &StatePackets71) -> Vec<draw71::StencilCfg> {
    p.stencil[..p.stencil_count].iter().map(|c| c.expect("an emitted STENCIL_CFG")).collect()
}

/// Compare every field of an emitted packet with the dump's packet: the
/// value as the dump prints it, and the field set itself.
fn same_fields<P: Packet>(what: &str, ours: &P, dump: &Pk) {
    let pairs = ours.pairs();
    for (f, v) in &pairs {
        assert_eq!(dump.value(f.name), v.clif_text(), "{what}: field '{}'", f.name);
    }
    let ours_names: Vec<&str> = pairs.iter().map(|(f, _)| f.name).collect();
    for (label, _, _) in &dump.fields {
        assert!(ours_names.contains(&label.as_str()), "{what}: the dump's field '{label}' is not modelled");
    }
}

fn name_of<T: PartialEq + Copy>(table: &[(T, &'static str)], v: T) -> &'static str {
    table.iter().find(|(t, _)| *t == v).map(|(_, n)| *n).expect("every value has a name")
}

// ---- the tests ---------------------------------------------------------------

#[test]
fn the_dump_has_two_draws_and_three_stencil_records() {
    let d = dump_draws();
    assert_eq!(d.len(), 2, "gl_stencilops draws two quads");
    assert_eq!(d[0].stencil.len(), 1, "draw 1: one STENCIL_CFG for both faces");
    assert_eq!(d[1].stencil.len(), 2, "draw 2: a front and a back STENCIL_CFG");
    assert_eq!(d[0].cfg_bits.value("Stencil enable"), "1");
    assert_eq!(d[1].cfg_bits.value("Stencil enable"), "1");
}

#[test]
fn stencil_cfg_records_match_mesa_per_draw() {
    let dump = dump_draws();
    for (i, (s, d)) in gl_draws().iter().zip(&dump).enumerate() {
        let ours = stencil_records(&emitted(s));
        assert_eq!(ours.len(), d.stencil.len(), "draw {}: the number of STENCIL_CFG records", i + 1);
        for (j, (o, m)) in ours.iter().zip(&d.stencil).enumerate() {
            same_fields(&format!("draw {} STENCIL_CFG #{j}", i + 1), o, m);
        }
    }
}

#[test]
fn stencil_cfg_symbolic_names_match_mesa() {
    let dump = dump_draws();
    for (i, (s, d)) in gl_draws().iter().zip(&dump).enumerate() {
        for (j, (o, m)) in stencil_records(&emitted(s)).iter().zip(&d.stencil).enumerate() {
            let what = format!("draw {} STENCIL_CFG #{j}", i + 1);
            let cmp = name_of(&draw71::COMPARES, o.stencil_test_function);
            assert_eq!(m.symbol("Stencil Test Function"), Some(cmp), "{what}: function");
            for (label, op) in [
                ("Stencil Test Fail Op", o.stencil_test_fail_op),
                ("Depth Test Fail Op", o.depth_test_fail_op),
                ("Stencil Pass Op", o.stencil_pass_op),
            ] {
                assert_eq!(m.symbol(label), Some(name_of(&draw71::STENCIL_OPS, op)), "{what}: {label}");
            }
        }
    }
}

#[test]
fn cfg_bits_match_mesa_per_draw() {
    let dump = dump_draws();
    for (i, (s, d)) in gl_draws().iter().zip(&dump).enumerate() {
        let p = emitted(s);
        assert!(p.cfg.stencil_enable, "draw {}: Stencil enable", i + 1);
        same_fields(&format!("draw {} CFG_BITS", i + 1), &p.cfg, &d.cfg_bits);
    }
}

/// The face slots: a record with `Front Config` sets the front face's
/// state, one with `Back Config` the back's. After each draw's records the
/// dump's slots and the emitter's slots hold the same values.
#[test]
fn face_slots_after_each_draw_match_mesa() {
    type Slot = Option<Vec<(String, String)>>;
    fn apply(slots: &mut [Slot; 2], fields: Vec<(String, String)>, front: bool, back: bool) {
        let state: Vec<(String, String)> =
            fields.into_iter().filter(|(l, _)| l != "Front Config" && l != "Back Config").collect();
        if front {
            slots[0] = Some(state.clone());
        }
        if back {
            slots[1] = Some(state);
        }
    }
    let (mut mesa, mut ours): ([Slot; 2], [Slot; 2]) = ([None, None], [None, None]);
    for (i, (s, d)) in gl_draws().iter().zip(dump_draws()).enumerate() {
        for m in &d.stencil {
            let fields = m.fields.iter().map(|(l, v, _)| (l.clone(), v.clone())).collect();
            apply(&mut mesa, fields, m.value("Front Config") == "1", m.value("Back Config") == "1");
        }
        for o in stencil_records(&emitted(s)) {
            let mut fields: Vec<(String, String)> =
                o.pairs().iter().map(|(f, v)| (f.name.to_string(), v.clif_text())).collect();
            // the dump's field order, so the slots compare as lists
            fields.sort_by_key(|(l, _)| d.stencil[0].fields.iter().position(|(dl, _, _)| dl == l));
            apply(&mut ours, fields, o.front_config, o.back_config);
        }
        assert_eq!(ours, mesa, "the face slots after draw {}", i + 1);
        assert!(mesa.iter().all(Option::is_some), "draw {}: both faces configured", i + 1);
    }
}

/// The first draw's one record configures both faces (a one-sided stencil);
/// the second's front record does not configure the back.
#[test]
fn one_sided_then_two_sided() {
    let [a, b] = gl_draws();
    let ra = stencil_records(&emitted(&a));
    assert_eq!((ra[0].front_config, ra[0].back_config), (true, true));
    let rb = stencil_records(&emitted(&b));
    assert_eq!((rb[0].front_config, rb[0].back_config), (true, false));
    assert_eq!((rb[1].front_config, rb[1].back_config), (false, true));
}

/// The GL enumerants of the shape go to the codes the dump prints.
#[test]
fn gl_enumerants_to_codes() {
    let ops = [
        (GL_ZERO, StencilOp::Zero, 0),
        (GL_KEEP, StencilOp::Keep, 1),
        (GL_REPLACE, StencilOp::Replace, 2),
        (GL_INCR, StencilOp::Incr, 3),
        (GL_DECR, StencilOp::Decr, 4),
        (GL_INVERT, StencilOp::Invert, 5),
        (GL_INCR_WRAP, StencilOp::IncWrap, 6),
        (GL_DECR_WRAP, StencilOp::DecWrap, 7),
    ];
    for (gl, op, code) in ops {
        assert_eq!(draw71::stencil_op_from_gl(gl), Ok(op));
        assert_eq!(op as u8, code);
    }
    assert_eq!(draw71::compare_from_gl(GL_LEQUAL), Ok(Compare::LEqual));
    assert_eq!(Compare::LEqual as u8, 3);
    assert_eq!(draw71::compare_from_gl(GL_NOTEQUAL), Ok(Compare::NotEqual));
    assert_eq!(Compare::NotEqual as u8, 5);
    assert_eq!(draw71::stencil_op_from_gl(0x1234), Err(draw71::Refusal71::UnknownStencilOp));
}

/// With the stencil test off the emitter sends no `STENCIL_CFG` and clears
/// `Stencil enable` (the state a draw of the same window has before
/// glEnable(GL_STENCIL_TEST)).
#[test]
fn stencil_test_off_emits_no_record() {
    let mut s = gl_draws()[1];
    s.stencil_test = false;
    let p = emitted(&s);
    assert!(!p.cfg.stencil_enable);
    assert_eq!(p.stencil_count, 0);
    assert!(p.stencil.iter().all(Option::is_none));
}
