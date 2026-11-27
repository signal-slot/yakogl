// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The jobs the V3D 7.1 table records (plan item table71-draw):
//! `yakogl_backend_v3d71` (src/v3d/table71.rs) driven through the GL core's
//! backend entries with the GlesDraws of a full `glClear` and of a (scissored)
//! `glDrawArrays` with gl_test's `color_uv` program, and what it submits
//! through the OSAL checked against Mesa's 7.1 reference for the same GL
//! calls, tools/v3d/pi5/gl_scissor.clif (gl_test case 5's shape; the shape
//! `scissor` of tests/v3d71_backend71_shapes.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table_draw
//! ```
//!
//! The OSAL is the mock of tests/v3d71_table_rig/mod.rs: its `submit` keeps
//! each job's `V3dKick` and the memory the job would read. The C builder is
//! the real one (driver/src/v3d/v3d_job71.c over v3d_frame.c at
//! `-DV3D_CL_VER=71`), reached in process through the table's own FFI
//! (`backend71::CBuilder71`, src/v3d/ffi71.rs): the rig builds the two files
//! into a shared library. So the lists compared here are the bytes the Pi 5
//! image would submit for these GlesDraws.
//!
//! # The comparison
//!
//! It is tests/v3d71_backend71_shapes.rs's (tools/v3d/check71.d gl_cmp.py's):
//! the lists are decoded with the packet table of
//! third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml at version 71 and
//! compared with the reference's packets field by field as the dump prints
//! them, addresses left out; the binning list as prologue, the state in
//! effect at each draw, and epilogue; the shader and attribute records the
//! same way. The lists are found from the submit alone: the binning and
//! render lists are the kick's ranges, the generic tile list is the range
//! the render list's `START_ADDRESS_OF_GENERIC_TILE_LIST` names, a draw's
//! record is the address of its `GL_SHADER_STATE` packet.
//!
//! The reference's two jobs are: a full clear; then, scissored to
//! (0,0,32,32), a clear (Mesa's blitter quad) and the draw. The first test
//! here leaves the scissored clear out, so its second job is the scissored
//! draw alone, compared with the reference's second draw and its state (the
//! whole of case 5, clear quad included, is tests/v3d71_table_cases.rs's).
//! A difference must be one entry of
//! [`ALLOW`] (the names tests/v3d71_backend71_shapes.rs gives the same
//! differences, plus the ones the table's own choices make: a raster colour
//! image at the GL core's address, a depth/stencil kept in the tile buffer),
//! and every entry must still occur.
#![cfg(feature = "gpu-v3d71")]

mod v3d71_table_rig;

use std::collections::BTreeMap;
use std::ffi::c_void;

use v3d71_table_rig::{gpu_block, kicks, log_lines, logs_with, osal_counts, rig, Kick};
use yakogl::backend::{
    self, GlesAttrib, GlesDraw, GlesDrawTarget, GLES_ATTRIBF_FLOAT32, GLES_CULL_NONE, GLES_PRIM_TRIANGLES,
    GLES_SOP_KEEP, GLES_SURF_RGBA8888,
};
use yakogl::shaderc::{ShcShader, ShcV3dUniform, ShcV3dVs};
use yakogl::util::{EINVAL, ENOTSUP};
use yakogl::v3d::table71::{self, yakogl_backend_v3d71, REFUSAL_PREFIX};
use yakogl::v3d::{attr71, record71};

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

/* ---- where the job's buffers sit (job71_check.c's gl mode, :794-805) ---- */

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

/* ---- the reference (tests/v3d71_backend71_shapes.rs, gl_cmp.py) ------------------- */

#[derive(Clone, Debug, PartialEq)]
struct Pk {
    name: String,
    fields: Vec<(String, String)>,
}

impl Pk {
    fn get(&self, k: &str) -> Option<&str> {
        self.fields.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }
    fn vals(&self) -> Vec<(String, String)> {
        self.fields.iter().filter(|f| !f.1.starts_with('[')).cloned().collect()
    }
}

struct RefJob {
    bcl: Vec<Pk>,
    rcl: Vec<Pk>,
    tiles: Vec<Pk>,
    args: Vec<String>,
    /// The shader records and, per record, its attribute records.
    records: Vec<Pk>,
    attrs: Vec<Vec<Pk>>,
}

fn field_line(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("/*")?;
    let (label, value) = rest.split_once("*/")?;
    let label = label.trim().strip_suffix(':')?.trim().to_string();
    let value = value.split(" /*").next().unwrap_or("").trim().to_string();
    Some((label, value))
}

fn is_packet(line: &str) -> bool {
    let b = line.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_uppercase()
        && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}

fn ref_jobs(shape: &str) -> Vec<RefJob> {
    let text = read(&format!("tools/v3d/pi5/gl_{shape}.clif"));
    let mut jobs: Vec<Vec<String>> = vec![];
    let mut cur: Option<Vec<String>> = None;
    for raw in text.lines() {
        if raw.starts_with("MESA: error") {
            continue;
        }
        let line = raw.strip_prefix("MESA: info:").map(|l| l.strip_prefix(' ').unwrap_or(l)).unwrap_or(raw);
        if line.starts_with("@createbuf") && cur.is_none() {
            cur = Some(vec![]);
        }
        if let Some(c) = cur.as_mut() {
            c.push(line.to_string());
        }
        if line.starts_with("@wait_render") {
            jobs.push(cur.take().unwrap());
        }
    }
    jobs.iter().map(|j| parse_ref_job(j)).collect()
}

fn parse_ref_job(job: &[String]) -> RefJob {
    let mut lists: Vec<(String, Vec<Pk>)> = vec![];
    let mut sub: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut buf, mut kind) = (String::new(), String::new());
    let mut in_list = false;
    let (mut records, mut attrs): (Vec<Pk>, Vec<Vec<Pk>>) = (vec![], vec![]);
    for line in job {
        if let Some(b) = line.strip_prefix("@buffer ") {
            buf = b.split_whitespace().next().unwrap().to_string();
            kind.clear();
            in_list = false;
            continue;
        }
        if let Some(f) = line.strip_prefix("@format ") {
            kind = f.split_whitespace().next().unwrap().to_string();
            in_list = kind == "ctrllist";
            if kind == "shadrec_gl_main" {
                records.push(Pk { name: "SHADREC".into(), fields: vec![] });
                attrs.push(vec![]);
            } else if kind == "shadrec_gl_attr" {
                attrs.last_mut().unwrap().push(Pk { name: "ATTR".into(), fields: vec![] });
            }
            if in_list && !lists.iter().any(|l| l.0 == buf) {
                lists.push((buf.clone(), vec![]));
            }
            continue;
        }
        if let Some(k) = line.strip_prefix('@') {
            let k = k.split_whitespace().next().unwrap_or("").to_string();
            in_list = false;
            if k == "add_bin" || k == "add_render" {
                sub.insert(k.clone(), vec![]);
            }
            kind = k;
            continue;
        }
        if (kind == "add_bin" || kind == "add_render") && !line.trim().is_empty() {
            sub.get_mut(&kind).unwrap().push(line.trim().split(" /*").next().unwrap().to_string());
            continue;
        }
        if kind == "shadrec_gl_main" || kind == "shadrec_gl_attr" {
            let p = if kind == "shadrec_gl_main" {
                records.last_mut()
            } else {
                attrs.last_mut().and_then(|g| g.last_mut())
            };
            if let (Some(f), Some(p)) = (field_line(line), p) {
                p.fields.push(f);
            }
            continue;
        }
        if !in_list {
            continue;
        }
        let l = lists.iter_mut().find(|l| l.0 == buf).unwrap();
        if is_packet(line) {
            l.1.push(Pk { name: line.to_string(), fields: vec![] });
        } else if let (Some(f), Some(p)) = (field_line(line), l.1.last_mut()) {
            p.fields.push(f);
        }
    }
    let bo = |a: &str| a.trim_start_matches('[').split(['+', ']']).next().unwrap().to_string();
    let bcl = bo(&sub["add_bin"][0]);
    let rcl = bo(&sub["add_render"][0]);
    let get = |n: &str| lists.iter().find(|l| l.0 == n).map(|l| l.1.clone()).unwrap_or_default();
    let tiles = lists.iter().filter(|l| l.0 != bcl && l.0 != rcl).flat_map(|l| l.1.clone()).collect();
    let args = sub["add_bin"].iter().chain(&sub["add_render"]).filter(|a| !a.starts_with('[')).cloned().collect();
    RefJob { bcl: get(&bcl), rcl: get(&rcl), tiles, args, records, attrs }
}

const DRAWS: [&str; 4] =
    ["VERTEX_ARRAY_PRIMS", "VERTEX_ARRAY_INSTANCED_PRIMS", "INDEXED_PRIM_LIST", "INDEXED_INSTANCED_PRIM_LIST"];

/// gl_cmp.py `key`: a state packet's slot.
fn key(p: &Pk) -> String {
    match p.name.as_str() {
        "STENCIL_CFG" => format!(
            "STENCIL_CFG[front {} back {}]",
            p.get("Front Config").unwrap_or("None"),
            p.get("Back Config").unwrap_or("None")
        ),
        "BLEND_CFG" => format!("BLEND_CFG[{}]", p.get("Render Target Mask").unwrap_or("None")),
        n if n.ends_with("_FLAGS") && !n.starts_with("ZERO_ALL") => {
            format!("{n}[{}]", p.fields.first().map(|f| f.1.as_str()).unwrap_or(""))
        }
        n => n.to_string(),
    }
}

/// The state packets in effect at a draw (by gl_cmp.py's `key`) and the draw
/// packet.
type DrawSeen = (BTreeMap<String, Pk>, Pk);

/// gl_cmp.py `bcl_split`: (prologue, [(state, draw packet)], epilogue).
fn bcl_split(ps: &[Pk]) -> (Vec<Pk>, Vec<DrawSeen>, Vec<Pk>) {
    let (mut pro, mut draws, mut tail) = (vec![], vec![], vec![]);
    let mut state: BTreeMap<String, Pk> = BTreeMap::new();
    let mut i = 0;
    while i < ps.len() {
        pro.push(ps[i].clone());
        i += 1;
        if ps[i - 1].name == "START_TILE_BINNING" {
            break;
        }
    }
    for p in &ps[i..] {
        if DRAWS.contains(&p.name.as_str()) {
            draws.push((state.clone(), p.clone()));
            tail.clear();
            continue;
        }
        tail.push(p.clone());
        state.insert(key(p), p.clone());
    }
    (pro, draws, tail)
}

/* ---- the packet table of cle/v3d_packet.xml at version 71 -------------------- */

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

/// A field's value as Mesa's CLIF dump prints its first token.
fn clif_value(f: &FieldDef, raw: u64) -> String {
    let fl = |x: f64| format!("{x:.6}");
    let sext = |v: u64, n: u32| ((v << (64 - n)) as i64) >> (64 - n);
    match f.ty.as_str() {
        "address" => "[addr]".to_string(),
        "bool" => format!("{}", raw & 1),
        "float" => fl(f64::from(f32::from_bits(raw as u32))),
        "f187" => fl(f64::from(f32::from_bits((raw as u32) << 16))),
        /* the dump prints a fixed-point field's raw value (VIEWPORT_OFFSET's
         * Fine X 8192 for 32 pixels) */
        "s8.8" | "int" => format!("{}", sext(raw, f.size)),
        _ => format!("{}", raw + f.minus_one as u64),
    }
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

fn decode_list(defs: &[PacketDef], bytes: &[u8], what: &str) -> Vec<Pk> {
    walk(defs, bytes, what)
        .into_iter()
        .map(|(def, body)| Pk {
            name: def.name.clone(),
            fields: def.fields.iter().map(|f| (f.name.clone(), clif_value(f, bits(body, f.start, f.size)))).collect(),
        })
        .collect()
}

/// A field of every packet called `name`, raw.
fn raw_fields(defs: &[PacketDef], bytes: &[u8], name: &str, field: &str) -> Vec<u64> {
    walk(defs, bytes, name)
        .into_iter()
        .filter(|(d, _)| d.name == name)
        .map(|(d, body)| {
            let f = d.fields.iter().find(|f| f.name == field).unwrap_or_else(|| panic!("{name}.{field}"));
            bits(body, f.start, f.size)
        })
        .collect()
}

/// fnmatch with `*` and `[!x]` / `[xy]` (what ALLOW uses).
fn glob(p: &[u8], s: &[u8]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some(b'*') => (0..=s.len()).any(|i| glob(&p[1..], &s[i..])),
        Some(b'[') => {
            let end = p.iter().position(|&c| c == b']').unwrap();
            let (neg, set) = if p[1] == b'!' { (true, &p[2..end]) } else { (false, &p[1..end]) };
            !s.is_empty() && (set.contains(&s[0]) != neg) && glob(&p[end + 1..], &s[1..])
        }
        Some(&c) => !s.is_empty() && s[0] == c && glob(&p[1..], &s[1..]),
    }
}

fn fld_cmp(w: &str, ours: &Pk, refp: &Pk, diffs: &mut Vec<String>) {
    for (k, v) in refp.vals() {
        let o = ours.get(&k).unwrap_or("<none>");
        if o.starts_with('[') {
            continue;
        }
        if o != v {
            diffs.push(format!("{w}.{k}: {o} != {v}"));
        }
    }
}

/// gl_cmp.py `seq_cmp`, the packet names aligned by their longest common
/// subsequence.
fn seq_cmp(w: &str, a: &[Pk], b: &[Pk], diffs: &mut Vec<String>) {
    let (n, m) = (a.len(), b.len());
    let mut l = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if a[i].name == b[j].name { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i].name == b[j].name {
            fld_cmp(&format!("{w} #{i} {}", a[i].name), &a[i], &b[j], diffs);
            i += 1;
            j += 1;
        } else if j == m || (i < n && l[i + 1][j] >= l[i][j + 1]) {
            diffs.push(format!("{w} #{i} {}: present != absent", a[i].name));
            i += 1;
        } else {
            diffs.push(format!("{w} #{j} {}: absent != present", b[j].name));
            j += 1;
        }
    }
}

fn bcl_cmp(w: &str, a: &[Pk], b: &[Pk], diffs: &mut Vec<String>) {
    let (pa, da, ta) = bcl_split(a);
    let (pb, db, tb) = bcl_split(b);
    seq_cmp(&format!("{w} prologue"), &pa, &pb, diffs);
    if da.len() != db.len() {
        diffs.push(format!("{w} draws: {} != {}", da.len(), db.len()));
    }
    for (n, ((sa, xa), (sb, xb))) in da.iter().zip(&db).enumerate() {
        let w = format!("{w} draw {n}");
        if xa.name != xb.name {
            diffs.push(format!("{w}: {} != {}", xa.name, xb.name));
        } else {
            fld_cmp(&format!("{w} {}", xa.name), xa, xb, diffs);
        }
        for k in sa.keys().chain(sb.keys().filter(|k| !sa.contains_key(*k))) {
            match (sa.get(k), sb.get(k)) {
                (Some(_), None) => diffs.push(format!("{w} {k}: present != absent")),
                (None, Some(_)) => diffs.push(format!("{w} {k}: absent != present")),
                (Some(p), Some(q)) => fld_cmp(&format!("{w} {k}"), p, q, diffs),
                _ => {}
            }
        }
    }
    seq_cmp(&format!("{w} epilogue"), &ta, &tb, diffs);
}

/* ---- the shader and attribute records ------------------------------------------ */

/// Our D0 record's plain fields (from "Number of varyings in Fragment
/// Shader" on) as the dump prints them; addresses as `[addr]`.
fn decode_record(rec: &[u8]) -> Pk {
    let first = record71::FIELDS.iter().position(|f| f.name == "Number of varyings in Fragment Shader").unwrap();
    let mut p = Pk { name: "SHADREC".into(), fields: vec![] };
    for f in &record71::FIELDS[first..] {
        let v = bits(rec, f.start, f.size);
        let text = match f.kind {
            record71::Kind::Address => "[addr]".to_string(),
            record71::Kind::UintMinusOne => (v + 1).to_string(),
            _ => v.to_string(),
        };
        p.fields.push((f.name.to_string(), text));
    }
    p
}

fn decode_attr(b: &[u8]) -> Pk {
    let mut g = Pk { name: "ATTR".into(), fields: vec![] };
    for (name, start, size) in attr71::FIELDS {
        let v = bits(b, start, size);
        let text = match name {
            "Address" => "[addr]".to_string(),
            "Vec size" if v == 0 => "4".to_string(),
            _ => v.to_string(),
        };
        g.fields.push((name.to_string(), text));
    }
    g
}

/// The reference's record fields that our decode has (its flag fields are
/// the plain 7.1 layout's view of the D0 word: not compared).
fn plain_record(r: &Pk) -> Pk {
    let first = record71::FIELDS.iter().position(|f| f.name == "Number of varyings in Fragment Shader").unwrap();
    let mut p = r.clone();
    p.fields.retain(|(l, _)| record71::FIELDS[first..].iter().any(|f| f.name == l));
    p
}

/* ---- the jobs ---------------------------------------------------------------------- */

/* ---- the GL core's side -------------------------------------------------------------- */

const G_SIZE: u32 = 64;
/// The window's depth/stencil identity (any non-zero id).
const DS_ID: u32 = 200;

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
const STRIDE: u32 = 40;
const VERTICES: usize = 6;

fn vertex_bytes() -> Vec<u8> {
    (0..VERTICES * STRIDE as usize).map(|i| (i * 7 + 3) as u8).collect()
}

/// The GL core's side of a test: a 64x64 RGBA8 window with a D24S8, its
/// surface and the program's code in GPU memory, and the vertices both as a
/// buffer object and as client memory.
struct Core {
    surface: u32,
    code: u32,
    vbo: u32,
    client: Vec<u8>,
}

impl Core {
    fn new() -> Core {
        let vbo = gpu_block(VERTICES * STRIDE as usize);
        v3d71_table_rig::gpu_write(vbo, &vertex_bytes());
        Core { surface: gpu_block((G_SIZE * G_SIZE * 4) as usize), code: gpu_block(0x300), vbo, client: vertex_bytes() }
    }

    /// The target and depth/stencil fields (`resolve_target`,
    /// `resolve_ds_storage` of src/gl/draw.rs).
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

    /// `glClear(GL_COLOR_BUFFER_BIT)`: a clear-only GlesDraw.
    fn clear(&self, rgba: [f32; 4]) -> GlesDraw {
        let mut d = self.base();
        (d.do_clear, d.clear_rgba, d.clear_color_mask) = (true, rgba, 0xf);
        d
    }

    /// `glDrawArrays(GL_TRIANGLES, 0, 6)` with `color_uv`: the position from
    /// client memory (staged by the table), colour and uv from the buffer
    /// object.
    fn draw(&self, scissor: Option<[i32; 4]>) -> GlesDraw {
        let mut d = self.base();
        d.index_count = VERTICES as u32;
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
        (d.attribs[0].buffer_gpu, d.attribs[0].offset) = (0, 0);
        (d.attribs[0].cpu, d.attribs[0].cpu_bytes) = (self.client.as_ptr() as *const c_void, self.client.len() as u32);
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
        (d.scissor, d.scissor_rect) = (scissor.is_some(), scissor.unwrap_or([0; 4]));
        d
    }
}

fn started() -> (v3d71_table_rig::Rig, Core) {
    let g = rig();
    assert_eq!(backend::init(), 0, "{:?}", log_lines());
    (g, Core::new())
}

/* ---- a submitted job, decoded ------------------------------------------------------- */

struct Ours {
    bcl: Vec<Pk>,
    rcl: Vec<Pk>,
    tiles: Vec<Pk>,
    qms: u32,
    /// Per draw: its record's address, the record, its attribute records.
    record_at: Vec<u32>,
    records: Vec<Pk>,
    attrs: Vec<Vec<Pk>>,
    /// Per draw, per attribute record: the raw Address field.
    attr_addrs: Vec<Vec<u32>>,
}

fn decode_job(defs: &[PacketDef], k: &Kick, w: &str) -> Ours {
    let (bcl, rcl) = (k.bcl(), k.rcl());
    let start = raw_fields(defs, rcl, "GENERIC_TILE_LIST", "start");
    let end = raw_fields(defs, rcl, "GENERIC_TILE_LIST", "end");
    assert_eq!((start.len(), end.len()), (1, 1), "{w}: one generic tile list");
    let tiles = k.at(start[0] as u32, (end[0] - start[0]) as usize);
    let recs = raw_fields(defs, bcl, "GL_SHADER", "address");
    let counts = raw_fields(defs, bcl, "GL_SHADER", "number of attribute arrays");
    let (mut record_at, mut records, mut attrs, mut attr_addrs) = (vec![], vec![], vec![], vec![]);
    for (a, n) in recs.iter().zip(&counts) {
        /* the packet's address is in 32-byte units; the attribute records
         * follow the 32-byte record */
        let at = (*a as u32) << 5;
        record_at.push(at);
        records.push(decode_record(k.at(at, record71::RECORD_SIZE)));
        let recs: Vec<&[u8]> = (0..*n as usize)
            .map(|i| k.at(at + (record71::RECORD_SIZE + i * attr71::RECORD_BYTES) as u32, attr71::RECORD_BYTES))
            .collect();
        attrs.push(recs.iter().map(|b| decode_attr(b)).collect());
        attr_addrs.push(recs.iter().map(|b| u32::from_le_bytes(b[0..4].try_into().unwrap())).collect());
    }
    Ours {
        bcl: decode_list(defs, bcl, &format!("{w} bcl")),
        rcl: decode_list(defs, rcl, &format!("{w} rcl")),
        tiles: decode_list(defs, tiles, &format!("{w} tiles")),
        qms: k.kick.tile_alloc_size,
        record_at,
        records,
        attrs,
        attr_addrs,
    }
}

/// A named difference from the reference: `(difference glob, id, reason)`.
/// EZ and PAGE are tests/v3d71_backend71_shapes.rs's `ALLOW` entries for the
/// same jobs (v3d_job71's and the Pi 5 kernel's); RASTER is the table's
/// choice of colour image; NOQUAD is this test's own calls (it makes no
/// scissored clear). The depth/stencil
/// the table keeps in the tile buffer makes no difference here: the
/// reference's jobs neither load nor store the window's D24S8.
const ALLOW: [(&str, &str, &str); 8] = [
    ("scissor job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", "EZ", "v3d_job71 never enables early Z (job71.sh gl.allow EZ)"),
    ("scissor job * submit #0: 16384 != 12288", "PAGE", "the tile allocation is rounded to the Pi 5 kernel's 16 KiB page (backend71::PAGE_SIZE); the drm-shim's page is 4 KiB"),
    ("scissor job * tiles #* *.Memory Format: 0 != 4", "RASTER", "the colour image is the GL core's raster surface at its own address (docs/notes/v3d71-table71.md 4.3), Mesa's window a UIF image"),
    ("scissor job * tiles #* *.Height in UB or Stride: 256 != 8", "RASTER", "the same: the raster stride in bytes (64 pixels of 4), not the UIF height in blocks"),
    ("scissor job 1 bcl draws: 1 != 2", "NOQUAD", "the reference's second job clears in the scissor box with Mesa's blitter quad before the draw; this test makes no scissored clear (tests/v3d71_table_cases.rs runs case 5 whole), so the job is the draw alone"),
    ("scissor job 1 records: 1/1 != 2/2", "NOQUAD", "the same: one draw, one record"),
    ("scissor job 1 bcl draw 0 *", "NOQUAD", "the same: our first draw is the triangles, the reference's first the quad; our draw is compared with the reference's second draw apart"),
    ("scissor job 1 draw 0 *", "NOQUAD", "the same, for the records"),
];

/// Every difference of the two jobs from gl_scissor's, gl_cmp.py's text; the
/// reference's draw `ref_draw` of job 1 is the one our job 1's draw is.
fn diffs(ours: &[Ours], refs: &[RefJob]) -> Vec<String> {
    let mut diffs = vec![];
    for (k, (o, r)) in ours.iter().zip(refs).enumerate() {
        let w = format!("scissor job {k}");
        bcl_cmp(&format!("{w} bcl"), &o.bcl, &r.bcl, &mut diffs);
        seq_cmp(&format!("{w} rcl"), &o.rcl, &r.rcl, &mut diffs);
        seq_cmp(&format!("{w} tiles"), &o.tiles, &r.tiles, &mut diffs);
        if let Some(a) = r.args.first() {
            if o.qms.to_string() != *a {
                diffs.push(format!("{w} submit #0: {} != {a}", o.qms));
            }
        }
        if o.records.len() != r.records.len() || o.attrs.len() != r.attrs.len() {
            diffs.push(format!(
                "{w} records: {}/{} != {}/{}",
                o.records.len(),
                o.attrs.len(),
                r.records.len(),
                r.attrs.len()
            ));
        }
        for (n, (a, b)) in o.records.iter().zip(&r.records).enumerate() {
            fld_cmp(&format!("{w} draw {n} SHADREC"), a, &plain_record(b), &mut diffs);
        }
        for (n, (ga, gb)) in o.attrs.iter().zip(&r.attrs).enumerate() {
            if ga.len() != gb.len() {
                diffs.push(format!("{w} draw {n} attribute records: {} != {}", ga.len(), gb.len()));
            }
            for (i, (a, b)) in ga.iter().zip(gb).enumerate() {
                fld_cmp(&format!("{w} draw {n} ATTR {i}"), a, b, &mut diffs);
            }
        }
    }
    diffs
}

/// Our job's one draw against draw `n` of the reference job: the draw
/// packet, the state in effect, the record and the attribute records.
fn draw_diffs(o: &Ours, r: &RefJob, n: usize) -> Vec<String> {
    let mut diffs = vec![];
    let (_, da, _) = bcl_split(&o.bcl);
    let (_, db, _) = bcl_split(&r.bcl);
    assert_eq!(da.len(), 1, "our job has one draw");
    let ((sa, xa), (sb, xb)) = (&da[0], &db[n]);
    let w = "scissor triangle";
    assert_eq!(xa.name, xb.name, "the draw packet");
    fld_cmp(&format!("{w} {}", xa.name), xa, xb, &mut diffs);
    for k in sa.keys().chain(sb.keys().filter(|k| !sa.contains_key(*k))) {
        match (sa.get(k), sb.get(k)) {
            (Some(_), None) => diffs.push(format!("{w} {k}: present != absent")),
            (None, Some(_)) => diffs.push(format!("{w} {k}: absent != present")),
            (Some(p), Some(q)) => fld_cmp(&format!("{w} {k}"), p, q, &mut diffs),
            _ => {}
        }
    }
    let b = plain_record(&r.records[n]);
    assert!(b.fields.len() >= 20, "the record's plain fields: {}", b.fields.len());
    fld_cmp(&format!("{w} SHADREC"), &o.records[0], &b, &mut diffs);
    assert_eq!(o.attrs[0].len(), r.attrs[n].len(), "attribute records");
    for (i, (a, b)) in o.attrs[0].iter().zip(&r.attrs[n]).enumerate() {
        fld_cmp(&format!("{w} ATTR {i}"), a, b, &mut diffs);
    }
    diffs
}

/* ---- the tests ------------------------------------------------------------------------ */

/// gl_test case 5's shape without its scissored clear: `glClear`,
/// `glFinish`; then the scissored draw, `glFinish`. Two jobs, each the
/// reference's but for the named differences.
#[test]
fn a_clear_and_a_scissored_triangle_are_the_references_jobs() {
    let (_g, c) = started();
    assert_eq!(backend::draw(&c.clear([0.0, 0.0, 0.0, 1.0])), 0, "{:?}", log_lines());
    assert_eq!(backend::finish(), 0);
    assert_eq!(backend::draw(&c.draw(Some([0, 0, 32, 32]))), 0, "{:?}", log_lines());
    assert_eq!(backend::finish(), 0);
    assert_eq!(table71::refused_draws(), 0);
    let ks = kicks();
    assert_eq!(ks.len(), 2, "one job per glFinish");

    let defs = packet_defs();
    let refs = ref_jobs("scissor");
    assert_eq!(refs.len(), 2);
    let ours: Vec<Ours> = ks.iter().enumerate().map(|(k, j)| decode_job(&defs, j, &format!("job {k}"))).collect();

    /* the whole jobs: every difference is named, every name still occurs */
    let found = diffs(&ours, &refs);
    let mut used = [0usize; ALLOW.len()];
    let mut left = vec![];
    for d in &found {
        match ALLOW.iter().position(|a| glob(a.0.as_bytes(), d.as_bytes())) {
            Some(i) => used[i] += 1,
            None => left.push(d.clone()),
        }
    }
    for (a, n) in ALLOW.iter().zip(&used) {
        if *n == 0 {
            left.push(format!("stale named difference {} ({})", a.1, a.0));
        }
    }
    assert!(left.is_empty(), "{} difference(s):\n{}\nall:\n{}", left.len(), left.join("\n"), found.join("\n"));

    /* the clear job draws nothing and clears with the tile buffer */
    assert!(ours[0].records.is_empty());
    assert!(ours[0].tiles.iter().any(|p| p.name == "CLEAR_RT"));
    assert!(!ours[0].tiles.iter().any(|p| p.name == "LOAD"), "nothing to load");

    /* the scissored triangle: the reference's second draw of job 1, with no
     * difference at all in the draw packet, the state and the records */
    let d = draw_diffs(&ours[1], &refs[1], 1);
    assert!(d.is_empty(), "the scissored triangle:\n{}", d.join("\n"));
    /* and the job loads what the clear job stored */
    assert!(ours[1].tiles.iter().any(|p| p.name == "LOAD"), "the colour is loaded");
    let clip = ours[1].bcl.iter().rev().find(|p| p.name == "CLIP").expect("a clip window");
    assert_eq!(
        [
            "Clip Window Left Pixel Coordinate",
            "Clip Window Bottom Pixel Coordinate",
            "Clip Window Width in pixels",
            "Clip Window Height in pixels"
        ]
        .map(|f| clip.get(f).unwrap().to_string()),
        ["0", "0", "32", "32"],
        "the scissor box"
    );
}

/// The same triangle without a scissor: the clip window is the target.
#[test]
fn a_plain_triangle_clips_to_the_target() {
    let (_g, c) = started();
    assert_eq!(backend::draw(&c.draw(None)), 0, "{:?}", log_lines());
    assert_eq!(backend::flush(), 0);
    let ks = kicks();
    assert_eq!(ks.len(), 1);
    let defs = packet_defs();
    let o = decode_job(&defs, &ks[0], "job 0");
    let clip = o.bcl.iter().rev().find(|p| p.name == "CLIP").expect("a clip window");
    assert_eq!(clip.get("Clip Window Width in pixels"), Some("64"));
    assert_eq!(clip.get("Clip Window Height in pixels"), Some("64"));
    /* a first job on a surface nothing stored loads nothing */
    assert!(!o.tiles.iter().any(|p| p.name == "LOAD"));
    /* a draw's binning-list bytes stay under the bound the arena is sized by */
    let empty = {
        assert_eq!(backend::draw(&c.clear([0.0; 4])), 0);
        assert_eq!(backend::flush(), 0);
        kicks()[1].bcl().len()
    };
    let per_draw = ks[0].bcl().len() - empty;
    assert!(per_draw as u32 <= table71::BCL_BYTES_PER_DRAW, "a draw is {per_draw} bytes of binning list");
}

/// What the job names is where the GL core's data is: the store goes to the
/// GL core's surface as raster RGBA8 rows (what glReadPixels reads), the
/// record names the program's code, the buffer object's arrays are read in
/// place and the client array from the table's staged copy.
#[test]
fn the_job_names_the_gl_cores_surface_code_and_arrays() {
    let (_g, c) = started();
    assert_eq!(backend::draw(&c.clear([0.0, 0.0, 0.0, 1.0])), 0);
    assert_eq!(backend::draw(&c.draw(None)), 0, "{:?}", log_lines());
    assert_eq!(backend::flush(), 0);
    let ks = kicks();
    assert_eq!(ks.len(), 1, "the clear and the draw are one job");
    let k = &ks[0];
    let defs = packet_defs();
    let o = decode_job(&defs, k, "job 0");

    /* the store: the surface, raster, RGBA8, a stride of 64 pixels */
    let rcl = k.rcl();
    let start = raw_fields(&defs, rcl, "GENERIC_TILE_LIST", "start")[0] as u32;
    let end = raw_fields(&defs, rcl, "GENERIC_TILE_LIST", "end")[0] as u32;
    let tiles = k.at(start, (end - start) as usize);
    assert_eq!(raw_fields(&defs, tiles, "STORE", "Address"), [u64::from(c.surface)]);
    let store = o.tiles.iter().find(|p| p.name == "STORE").unwrap();
    assert_eq!(store.get("Memory Format"), Some("0"), "raster");
    assert_eq!(store.get("Height in UB or Stride"), Some("256"), "bytes a row");
    assert_eq!(store.get("R/B swap"), Some("0"), "R, G, B, A in memory");
    assert_eq!(store.get("Buffer to Store"), Some("0"), "render target 0");
    let rt = o.rcl.iter().find(|p| p.name == "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1").unwrap();
    /* V3D_RENDER_TARGET_TYPE_CLAMP_8 (8), V3D_INTERNAL_BPP_32 (0): RGBA8 in
     * the tile buffer, stored as V3D_OUTPUT_IMAGE_FORMAT_RGBA8 (27) */
    assert_eq!((rt.get("Internal Type and Clamping"), rt.get("Internal BPP")), (Some("8"), Some("0")));
    assert_eq!(store.get("Output Image Format"), Some("27"), "RGBA8");

    /* the record: the three programs at the upload's offsets */
    let rec = k.at(o.record_at[0], record71::RECORD_SIZE);
    let code = |name: &str| {
        let f = record71::FIELDS.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}"));
        (bits(rec, f.start, f.size) as u32) << 3
    };
    assert_eq!(code("Fragment Shader Code Address"), c.code);
    assert_eq!(code("Vertex Shader Code Address"), c.code + 0x100);
    assert_eq!(code("Coordinate Shader Code Address"), c.code + 0x200);

    /* the arrays */
    let a = &o.attr_addrs[0];
    assert_eq!(a.len(), 3);
    assert_eq!((a[1], a[2]), (c.vbo + 16, c.vbo + 32), "the buffer object, in place");
    assert!(a[0] != c.vbo && a[0] != 0, "the client array is a copy");
    assert_eq!(k.at(a[0], c.client.len()), &c.client[..], "the staged copy holds the client's bytes");
    assert_eq!(a[0] % 16, 0);

    /* the binner's buffers are the table's, of the size the job asks */
    assert!(k.kick.tile_alloc != 0 && k.kick.tile_state != 0);
    assert_eq!(k.kick.tile_alloc_size, 16384);
    assert_eq!(k.kick.tile_state_size, 256, "one 64x64 tile");
    let live = v3d71_table_rig::live_allocs();
    for (addr, size) in [(k.kick.tile_alloc, k.kick.tile_alloc_size), (k.kick.tile_state, k.kick.tile_state_size)] {
        assert!(live.iter().any(|a| a.0 == addr && a.1 >= size as usize), "{addr:#x} is an OSAL allocation");
    }
}

/// `kick_id` and `kick_pending` (src/backend.rs, `YakoglBackend`): the kick
/// being recorded is pending from its first draw; after a flush it is the
/// job in flight until the OSAL completed it; the counters count.
#[test]
fn the_kick_ids_follow_the_contract() {
    let (_g, c) = started();
    let t = &yakogl_backend_v3d71;
    let (kick_id, pending) = (t.kick_id.unwrap(), t.kick_pending.unwrap());
    // SAFETY: the table's hooks with their documented arguments.
    unsafe {
        assert_eq!(kick_id(), 1);
        assert_eq!(backend::draw(&c.clear([0.0, 0.0, 0.0, 1.0])), 0);
        assert_eq!(kick_id(), 1, "the clear is in the open frame");
        assert!(pending(1));
        assert_eq!(osal_counts(), (0, 0), "a draw submits nothing");
        assert_eq!(backend::flush(), 0);
        assert_eq!(osal_counts(), (1, 0), "flush submits and does not wait");
        assert_eq!(kick_id(), 2);
        assert!(pending(1), "in flight");
        assert!(pending(2), "the next frame");
        assert_eq!(backend::flush(), 0);
        assert_eq!(osal_counts(), (1, 0), "nothing new to submit");
        assert_eq!(t.wait_kick.unwrap()(1), 0);
        assert_eq!(osal_counts(), (1, 1));
        assert!(!pending(1), "completed");
        assert_eq!(t.wait_kick.unwrap()(1), 0);
        assert_eq!(osal_counts(), (1, 1), "nothing left to wait for");

        /* wait_kick on the open frame submits it and waits */
        assert_eq!(backend::draw(&c.draw(None)), 0, "{:?}", log_lines());
        assert_eq!(kick_id(), 2);
        assert_eq!(t.wait_kick.unwrap()(2), 0);
        assert_eq!(osal_counts(), (2, 2));
        assert!(!pending(2));
        assert_eq!(kick_id(), 3);

        let (mut kicks, mut waits) = (0u32, 0u32);
        t.counters.unwrap()(&mut kicks, &mut waits);
        assert_eq!((kicks, waits), (2, 2));
    }
    /* the OSAL was told when a job was being recorded and when it ended */
    assert_eq!(v3d71_table_rig::job_open_calls(), [true, false, true, false]);
}

/// A whole clear after queued draws opens a new job (clearplan71's
/// `new_job`): the table submits the queued draws first, so the kick after
/// the call holds the clear (the contract's "a draw that ended the open frame
/// is in the next kick"), and the stamp the GL core gave it stays pending.
#[test]
fn a_clear_after_draws_ends_the_open_job_first() {
    let (_g, c) = started();
    let t = &yakogl_backend_v3d71;
    // SAFETY: the table's hooks with their documented arguments.
    unsafe {
        assert_eq!(backend::draw(&c.draw(None)), 0, "{:?}", log_lines());
        let stamped = t.kick_id.unwrap()();
        assert_eq!(stamped, 1);
        assert_eq!(backend::draw(&c.clear([1.0, 0.0, 0.0, 1.0])), 0);
        assert_eq!(osal_counts().0, 1, "the draw's job went first");
        assert_eq!(t.kick_id.unwrap()(), 2, "the clear is in the next kick");
        assert_eq!(backend::finish(), 0);
        assert_eq!(osal_counts().0, 2);
        assert!(!t.kick_pending.unwrap()(stamped) && !t.kick_pending.unwrap()(2));
    }
    let ks = kicks();
    let defs = packet_defs();
    let first = decode_job(&defs, &ks[0], "job 0");
    let second = decode_job(&defs, &ks[1], "job 1");
    assert_eq!((first.records.len(), second.records.len()), (1, 0));
    assert!(!second.tiles.iter().any(|p| p.name == "LOAD"), "the clear loads nothing");
}

/// A draw on another framebuffer ends the open frame: one job per target.
#[test]
fn another_framebuffer_ends_the_open_frame() {
    let (_g, c) = started();
    let other = gpu_block((G_SIZE * G_SIZE * 4) as usize);
    assert_eq!(backend::draw(&c.clear([0.0, 0.0, 0.0, 1.0])), 0);
    let mut d = c.clear([0.0, 1.0, 0.0, 1.0]);
    (d.target_gpu, d.targets[0].gpu) = (u64::from(other), u64::from(other));
    d.ds_persist = false; /* a colour-only framebuffer object */
    assert_eq!(backend::draw(&d), 0, "{:?}", log_lines());
    assert_eq!(osal_counts().0, 1, "the window's clear was submitted");
    assert_eq!(backend::finish(), 0);
    let ks = kicks();
    assert_eq!(ks.len(), 2);
    let defs = packet_defs();
    let store = |k: &Kick| {
        let rcl = k.rcl();
        let start = raw_fields(&defs, rcl, "GENERIC_TILE_LIST", "start")[0] as u32;
        let end = raw_fields(&defs, rcl, "GENERIC_TILE_LIST", "end")[0] as u32;
        raw_fields(&defs, k.at(start, (end - start) as usize), "STORE", "Address")
    };
    assert_eq!(store(&ks[0]), [u64::from(c.surface)]);
    assert_eq!(store(&ks[1]), [u64::from(other)]);
}

/// The platform's draw budget bounds the open frame: the draw past it ends
/// the job, and the next job loads what that one stored.
#[test]
fn the_draw_budget_ends_a_job() {
    let g = rig();
    v3d71_table_rig::set_draws_per_job(2);
    assert_eq!(backend::init(), 0);
    let c = Core::new();
    for _ in 0..3 {
        assert_eq!(backend::draw(&c.draw(None)), 0, "{:?}", log_lines());
    }
    assert_eq!(osal_counts().0, 1, "two draws went when the third came");
    assert_eq!(backend::finish(), 0);
    let ks = kicks();
    let defs = packet_defs();
    let jobs: Vec<Ours> = ks.iter().map(|k| decode_job(&defs, k, "job")).collect();
    assert_eq!(jobs.iter().map(|j| j.records.len()).collect::<Vec<_>>(), [2, 1]);
    assert!(jobs[1].tiles.iter().any(|p| p.name == "LOAD"));
    /* each job of one arena set waits for the one before it */
    assert_eq!(osal_counts(), (2, 2));
    drop(g);
}

/// What the table cannot record comes back as a named refusal, counted and
/// logged once per cause, and leaves the open frame as it was.
#[test]
fn a_refused_state_is_a_named_refusal() {
    let (_g, c) = started();
    assert_eq!(backend::draw(&c.clear([0.0, 0.0, 0.0, 1.0])), 0);
    let line = |why: &str| format!("{REFUSAL_PREFIX}{why}");
    let mut refused = 0;
    let mut check = |d: &GlesDraw, errno: i32, why: &str| {
        refused += 1;
        assert_eq!(backend::draw(d), -errno, "{why}: {:?}", log_lines());
        assert_eq!(logs_with(&line(why)).len(), 1, "{why}: {:?}", log_lines());
        assert_eq!(table71::refused_draws(), refused, "{why}");
    };

    /* the 4.2 table's shared decisions, the same strings (src/v3d/state.rs) */
    let mut d = c.draw(None);
    d.alpha_to_coverage = true;
    check(&d, ENOTSUP, "alpha-to-coverage is not implemented");
    let mut d = c.draw(None);
    d.target_format = 0xee;
    check(&d, EINVAL, "a colour target of a format the backend does not know");

    /* glue71's: a texture binding */
    let mut d = c.draw(None);
    d.num_textures = 1;
    check(&d, ENOTSUP, "glue71: V3D 7.1: a draw sampling textures (the 7.1 texture state is not glued yet)");
    /* draw71's, through glue71: multisampling */
    let mut d = c.draw(None);
    d.samples = 4;
    check(&d, ENOTSUP, "draw71: V3D 7.1: multisampled drawing (the 7.1 table has no 4x oversample state yet)");

    /* backend71's: an indexed draw of 8-bit or 32-bit indices, from a
     * buffer and from client memory (u16 draws: tests/v3d71_table_indexed.rs) */
    let wide = "backend71: an indexed draw of 8-bit or 32-bit indices: the 7.1 table assembles 16-bit indices only";
    let mut d = c.draw(None);
    (d.index_buf_gpu, d.index_size) = (u64::from(c.vbo), 4);
    check(&d, ENOTSUP, wide);
    let idx = [0u8, 1, 2, 3, 4, 5];
    let mut d = c.draw(None);
    (d.index_cpu, d.index_bytes, d.index_size) = (idx.as_ptr() as *const c_void, 6, 1);
    check(&d, ENOTSUP, wide);

    /* a malformed table: a program that is not linked */
    let mut unlinked = *color_uv();
    unlinked.v3d.linked = 0;
    let mut d = c.draw(None);
    d.fs = &unlinked;
    check(&d, EINVAL, "the fragment shader is not a program linked for the V3D");
    let mut d = c.draw(None);
    d.fs = core::ptr::null();
    check(&d, ENOTSUP, "glue71: V3D 7.1: a draw with no linked program");

    /* the open frame is as it was: the first clear and nothing else */
    assert_eq!(osal_counts().0, 0, "no refusal submitted anything");
    assert_eq!(backend::draw(&c.draw(None)), 0, "{:?}", log_lines());
    assert_eq!(backend::finish(), 0);
    let ks = kicks();
    assert_eq!(ks.len(), 1);
    let o = decode_job(&packet_defs(), &ks[0], "job 0");
    assert_eq!(o.records.len(), 1, "the clear and the one draw");
    assert!(o.tiles.iter().any(|p| p.name == "CLEAR_RT"));
}

/// gl_test case 59's read: the window's depth/stencil stays in the tile
/// buffer in this cut, so a draw that tests stencil an earlier job wrote is
/// refused by the 4.2 table's string (src/v3d/zskeep71.rs `ReadLost`).
#[test]
fn stencil_an_earlier_job_wrote_is_refused_by_the_4_2_name() {
    let (_g, c) = started();
    /* job 1 writes stencil: REPLACE on a draw */
    let mut d = c.draw(None);
    d.stencil = true;
    (d.stencil_dppass, d.stencil_back_dppass) = (backend::GLES_SOP_REPLACE, backend::GLES_SOP_REPLACE);
    (d.stencil_ref, d.stencil_back_ref) = (1, 1);
    assert_eq!(backend::draw(&d), 0, "{:?}", log_lines());
    assert_eq!(backend::finish(), 0);
    /* job 2 tests it */
    let mut d = c.draw(None);
    d.stencil = true;
    (d.stencil_func, d.stencil_back_func) = (backend::GLES_CMP_EQUAL, backend::GLES_CMP_EQUAL);
    assert_eq!(backend::draw(&d), -ENOTSUP);
    let want = "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in";
    /* the 4.2 table's line but for the prefix: no module name in front */
    assert_eq!(logs_with(want), [format!("{REFUSAL_PREFIX}{want}")], "{:?}", log_lines());
    assert_eq!(osal_counts().0, 1, "only the first job");
}
