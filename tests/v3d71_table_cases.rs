// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! gl_test's six P9 cases on the V3D 7.1 table (plan item table71-cases):
//! cases 5, 58, 59, 66, 67 and 102 (`CONFIG_V3D_GL_TEST_CASES` of the rpi_5
//! image), each as the GL calls of its function in
//! driver/tools/pvrlab/gl_test.c, through the GL core's entry points
//! (`glClear`, `glDrawArrays`, `glFinish`, ...) with `yakogl_backend_v3d71`
//! (src/v3d/table71.rs) as `g_backend`, and a verdict for each.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table_cases
//! ```
//!
//! The GL core is the real one: a context with a 64x64 window surface
//! (`gl_context_create(TW, TH)`, as gl_test's), programs compiled and linked
//! by the shader back end the build has, buffer objects, framebuffer objects.
//! What is compared of a program is its record's description, which the
//! 4.2 link and link71 give alike for gl_test's pair and the clear program,
//! so the test holds whichever of the two the build's link entry calls.
//! The OSAL is the mock of tests/v3d71_table_rig/mod.rs (its `submit` keeps
//! each job and the memory it would read) and the C builder is the real
//! driver/src/v3d/v3d_job71.c. Nothing renders on the host, so a case's
//! pixel checks (`expect_px`) are not run: every `glReadPixels` of the case is
//! called where the case has it and its result is not looked at. A verdict
//! here is what the table did with the case's calls:
//!
//! | case | verdict | checked |
//! |---|---|---|
//! | 5 scissor | recorded | no GL error, no refusal, the jobs are Mesa's `gl_scissor.clif` |
//! | 58 clear with scissor | recorded | the same against `gl_scisbox.clif` |
//! | 66 clear not bounded by the viewport | recorded | the same against `gl_viewport.clif` |
//! | 102 clear under a partial colour mask | recorded | the same against `gl_colormask.clif` |
//! | 59 clear with write masks | **refused by name** | the last draw (stencil test across a `glFinish`) is `GL_INVALID_OPERATION`, the log names [`READS_STENCIL`], the three jobs before it are submitted |
//! | 67 FBO limits | **refused by name** | frames 1 to 7 recorded (the GL_RGB frame too), frame 8's draw refused by [`READS_STENCIL`] |
//!
//! [`READS_STENCIL`] is the 4.2 table's string (src/v3d/backend.rs
//! `fn record`; its 7.1 twin is zskeep71's `ReadLost`): the table keeps
//! depth/stencil in the tile buffer only, so stencil one job wrote cannot be
//! tested by the next. The refusal "a colour format with fewer than 8 alpha
//! bits" that the Pi 4's first run printed for case 67 is not in the tree any
//! more (tests/v3d71_refuse67.rs), so no table can produce it; case 67's
//! GL_RGB frame is recorded and its refusal is the stencil one.
//!
//! # The comparison with Mesa
//!
//! tests/v3d71_table_draw.rs's (tools/v3d/check71.d gl_cmp.py's): each
//! submitted job is decoded with the packet table of
//! third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml at version 71 and
//! compared with the reference's job of the same number, field by field as
//! the dump prints them, addresses left out: the binning list (prologue, the
//! state in effect at each draw, epilogue), the render list, the generic
//! tile list, the tile allocation size, and each draw's shader record and
//! attribute records. A difference must match one entry of [`ALLOW`], each
//! named with its reason, and every entry must still match one.
//!
//! The references were drawn into a framebuffer object standing for the
//! window (tools/v3d/pi5/src/gl_shapes.py); here the cases draw into the
//! real window surface, which is stored top-down (`GlesDraw::flip_y`). That
//! is the entries named `FLIP`.
//!
//! tests/v3d71_gl_table.rs (P9's acceptance c3) includes this file as its
//! module `cases` and calls the `pub(crate)` items (the cases' GL calls,
//! `recorded`, the rig).
#![cfg(all(feature = "gpu-v3d71", feature = "host-test", feature = "shaderc-online"))]

pub(crate) mod v3d71_table_rig;

use std::collections::BTreeMap;
use std::ffi::c_void;

use v3d71_table_rig::{kicks, log_lines, logs_with, osal_counts, rig, Kick, Rig};
use yakogl::backend;
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context::{gl_context_create, gl_context_destroy, GlContext};
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::v3d::table71::{self, REFUSAL_PREFIX};
use yakogl::v3d::{attr71, record71};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/* ---- the reference (tests/v3d71_table_draw.rs, gl_cmp.py) --------------------------- */

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

/* ---- a submitted job, decoded ------------------------------------------------------- */

pub(crate) struct Ours {
    bcl: Vec<Pk>,
    rcl: Vec<Pk>,
    tiles: Vec<Pk>,
    qms: u32,
    /// Per draw: the record, its attribute records.
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
    let (mut records, mut attrs, mut attr_addrs) = (vec![], vec![], vec![]);
    for (a, n) in recs.iter().zip(&counts) {
        /* the packet's address is in 32-byte units; the attribute records
         * follow the 32-byte record */
        let at = (*a as u32) << 5;
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
        records,
        attrs,
        attr_addrs,
    }
}

/// Every difference of our jobs from the reference's jobs of the same
/// numbers, in gl_cmp.py's text, each line starting `<shape> job <n>`.
fn diffs(shape: &str, ours: &[Ours], refs: &[RefJob]) -> Vec<String> {
    let mut diffs = vec![];
    for (k, (o, r)) in ours.iter().zip(refs).enumerate() {
        let w = format!("{shape} job {k}");
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

/// A named difference from the reference: `(shapes, difference glob, id,
/// reason)`. An entry is for the shapes it lists and must occur in each of
/// them. EZ, PSIZE, EMPTYBOX, RGBALPHA, UNDEFZ, PAGE, QUADATTR and RECORDDUP
/// are tests/v3d71_backend71_shapes.rs's entries for the same jobs; the
/// others are the table's and the GL core's own choices.
const ALLOW: &[(&str, &str, &str, &str)] = &[
    ("scissor scisbox writemask viewport fbods colormask", "* job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", "EZ", "v3d_job71 never enables early Z (job71.sh gl.allow EZ)"),
    ("scissor scisbox writemask viewport fbods colormask", "* job * bcl draw * POINT_SIZE.Point Size: 1.000000 != 0.125000", "PSIZE", "the clear quad: Mesa's blitter writes point size 0.125, v3d_job71 1.0; a TRIANGLE_FAN does not read it"),
    ("scissor scisbox writemask viewport fbods colormask", "* job * draw * ATTR 0.Vec size: 4 != 3", "QUADATTR", "our clear quad's position is fill71's FilledDraw::quad, four floats (x, y, z, w) a vertex; Mesa's blitter writes three"),
    ("scissor scisbox writemask viewport fbods colormask", "* job * draw * ATTR 0.Stride: 16 != 36", "QUADATTR", "the same: our quad's vertex is 16 bytes, Mesa's blitter vertex 36"),
    ("scissor scisbox writemask viewport fbods colormask", "* job * tiles #* *.Memory Format: 0 != 4", "RASTER", "the colour image is the GL core's raster surface at its own address (docs/notes/v3d71-table71.md 4.3), Mesa's a UIF image"),
    ("scissor scisbox writemask viewport fbods colormask", "* job * tiles #* *.Height in UB or Stride: 256 != 8", "RASTER", "the same: the raster stride in bytes (64 pixels of 4), not the UIF height in blocks"),
    ("scissor writemask viewport fbods colormask", "* job * submit #0: 16384 != 12288", "PAGE", "the tile allocation is rounded to the Pi 5 kernel's 16 KiB page (backend71::PAGE_SIZE); the drm-shim's page is 4 KiB"),
    ("scissor scisbox writemask viewport colormask", "* job * bcl draw * CFG_BITS.Clockwise Primitives: 1 != 0", "FLIP", "gl_test's window surface is stored top-down (GlesDraw::flip_y), which mirrors the winding; the reference's window is a framebuffer object"),
    ("scissor scisbox writemask viewport colormask", "* job * bcl draw * CLIPPER_XY.Viewport Half-Height in 1/64th of pixel: -2048.000000 != 2048.000000", "FLIP", "the same: the viewport's y scale is negative"),
    ("scissor", "scissor job 1 bcl draw 1 CLIP.Clip Window Bottom Pixel Coordinate: 32 != 0", "FLIP", "the same: GL's bottom-left 32x32 box is rows 32 to 63 of a top-down surface"),
    ("scisbox", "scisbox job 2 bcl draws: 0 != 1", "EMPTYBOX", "the GL core drops a clear whose scissor box is empty (src/gl/draw.rs plan_clear); Mesa's blitter draws a quad that covers no pixel"),
    ("scisbox", "scisbox job 2 records: 0/0 != 1/1", "EMPTYBOX", "the same: no draw, no record"),
    ("writemask fbods", "* job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early Depth/Stencil Clear: 1 != 0", "TILEONLY", "the table keeps depth/stencil in the tile buffer only (glue71::DsPlace at address 0): a job with a Z/S buffer stores none of it, so v3d_job71 sets the early clear; Mesa stores the plane the job wrote"),
    ("writemask fbods", "* job * tiles #* STORE: absent != present", "TILEONLY", "the same: Mesa's store of the depth or stencil plane (what the refused read of cases 59 and 67 would load)"),
    ("fbods", "fbods job * bcl draw * VERTEX_ARRAY_PRIMS.Index of First Vertex: 0 != 6", "FIRST", "glDrawArrays(GL_TRIANGLES, 6, 6): the GL core puts the first vertex into the arrays' offsets (as for the 4.2 table), Mesa into the packet"),
    ("fbods", "fbods job 0 records: 2/2 != 1/1", "RECORDDUP", "two draws with the same programs and arrays: v3d_job71 writes a shader record per draw, Mesa reuses the first"),
    ("fbods", "fbods job 2 rcl #* TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1.Clear Color low bits: 4278190335 != 1073742079", "RGBALPHA", "frame 3's GL_RGB target keeps alpha 1 (the 4.2 table's rule): its clear writes alpha 1, Mesa's alpha 0.25. The frame is drawn, not refused"),
    ("fbods", "fbods job [34] rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Internal Depth Type: 1 != 2", "D16", "GlesDraw does not carry the depth format: a GL_DEPTH_COMPONENT16 renderbuffer is a 24-bit plane, Mesa's 16 (docs/notes/v3d71-table71.md 2.1, F3)"),
    ("fbods", "fbods job 4 rcl #* TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES.Z Clear Value: 1.000000 != 0.000000", "UNDEFZ", "zskeep71 starts an undefined depth plane the job uses at 1.0, Mesa's zeroed job at 0.0"),
];

/// The submitted jobs against the first `ks.len()` jobs of the reference
/// `gl_<shape>.clif`, which has `ref_jobs_total` in all: every difference is
/// one of [`ALLOW`]'s entries for the shape, and each of those occurs.
fn check_against_mesa(shape: &str, ks: &[Kick], ref_jobs_total: usize) -> Vec<Ours> {
    let defs = packet_defs();
    let refs = ref_jobs(shape);
    assert_eq!(refs.len(), ref_jobs_total, "{shape}: the reference's jobs");
    assert!(ks.len() <= refs.len(), "{shape}: {} jobs, the reference has {}", ks.len(), refs.len());
    let ours: Vec<Ours> = ks.iter().enumerate().map(|(k, j)| decode_job(&defs, j, &format!("job {k}"))).collect();
    let found = diffs(shape, &ours, &refs);
    let mine: Vec<_> = ALLOW.iter().filter(|a| a.0.split(' ').any(|s| s == shape)).collect();
    let mut used = vec![0usize; mine.len()];
    let mut left = vec![];
    for d in &found {
        match mine.iter().position(|a| glob(a.1.as_bytes(), d.as_bytes())) {
            Some(i) => used[i] += 1,
            None => left.push(d.clone()),
        }
    }
    for (a, n) in mine.iter().zip(&used) {
        if *n == 0 {
            left.push(format!("stale named difference {} ({})", a.2, a.1));
        }
    }
    assert!(
        left.is_empty(),
        "{shape}: {} difference(s) without a name:\n{}\nall:\n{}",
        left.len(),
        left.join("\n"),
        found.join("\n")
    );
    ours
}

/* ---- gl_test's helpers (driver/tools/pvrlab/gl_test.c) ------------------------------ */

/// TW, TH (gl_test.c:1088-1089).
const TW: GLsizei = 64;
const TH: GLsizei = 64;

/// gl_test's ES 1.00 pair (`gl_test_vs_src`, `gl_test_fs_src`): a_pos 0,
/// a_color 1, a_uv 2; the colour times (u, v, 1, 1).
const COLOR_UV_VS: &str = concat!(include_str!("../tools/shaders/gl_test_es100/color_uv.vert"), "\0");
const COLOR_UV_FS: &str = concat!(include_str!("../tools/shaders/gl_test_es100/color_uv.frag"), "\0");

/// The 4.2 table's string for a read of stencil an earlier job wrote
/// (src/v3d/backend.rs `fn record`), which zskeep71 repeats.
pub(crate) const READS_STENCIL: &str =
    "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in";

/// `struct cuv_vertex`: pos4, colour4, uv2 (40 bytes).
pub(crate) type Vtx = [f32; 10];

/// `cuv_set`.
fn cuv(x: f32, y: f32, z: f32, r: f32, g: f32, b: f32) -> Vtx {
    [x, y, z, 1.0, r, g, b, 1.0, 1.0, 1.0]
}

const FULL: [[f32; 2]; 6] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// `solid_quad`: a full-screen quad of one colour at depth `z`.
pub(crate) fn solid_quad(r: f32, g: f32, b: f32, z: f32) -> Vec<Vtx> {
    FULL.iter().map(|c| cuv(c[0], c[1], z, r, g, b)).collect()
}

/// A case stops at its first failed `check_gl_error`, as gl_test's does
/// (`return -1`): the label of the check and the error.
pub(crate) type Case = Result<(), String>;
/// A case's function ([`gl`]).
pub(crate) type CaseFn = unsafe fn() -> Case;

/// `check_gl_error`.
fn check_gl_error(what: &str) -> Case {
    // SAFETY: a plain GL call.
    match unsafe { glGetError() } {
        GL_NO_ERROR => Ok(()),
        e => Err(format!("{what}: GL error {e:#06x}")),
    }
}

/// The GL calls of the cases; every one needs the current context of [`Gl`].
pub(crate) mod gl {
    use super::*;

    /// `link_program(gl_test_vs_src, gl_test_fs_src)`.
    pub unsafe fn link_program() -> GLuint {
        let compile = |ty: GLenum, src: &str| {
            let s = glCreateShader(ty);
            let p = src.as_ptr() as *const GLchar;
            glShaderSource(s, 1, &p, core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_ne!(ok, 0, "shader {ty:#x} did not compile: {:?}", log_lines());
            s
        };
        let vs = compile(GL_VERTEX_SHADER, COLOR_UV_VS);
        let fs = compile(GL_FRAGMENT_SHADER, COLOR_UV_FS);
        let prog = glCreateProgram();
        glAttachShader(prog, vs);
        glAttachShader(prog, fs);
        glLinkProgram(prog);
        let mut ok: GLint = 0;
        glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
        assert_ne!(ok, 0, "the program did not link: {:?}", log_lines());
        prog
    }

    /// `upload_vbo` and `cuv_attribs`.
    pub unsafe fn upload(v: &[Vtx]) {
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(v) as GLsizeiptr,
            v.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        for (i, (n, off)) in [(4, 0usize), (4, 16), (2, 32)].into_iter().enumerate() {
            glVertexAttribPointer(i as GLuint, n, GL_FLOAT, 0, 40, off as *const c_void);
            glEnableVertexAttribArray(i as GLuint);
        }
    }

    /// `read_topdown`: the case's glReadPixels (the pixels are not looked at:
    /// nothing renders on the host).
    pub unsafe fn read_topdown() {
        let mut px = vec![0u32; (TW * TH) as usize];
        glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }

    /// `reset_state` (gl_test.c:1472): what gl_test puts back before a case.
    pub unsafe fn reset_state() {
        while glGetError() != GL_NO_ERROR {}
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, TW, TH);
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_CULL_FACE);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_BLEND);
        glDepthMask(1);
        glDepthFunc(GL_LESS);
        glFrontFace(GL_CCW);
        glCullFace(GL_BACK);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glColorMask(1, 1, 1, 1);
        glStencilMask(0xff);
        glDisable(GL_STENCIL_TEST);
        glClearDepthf(1.0);
        glClearStencil(0);
        glUseProgram(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, 0);
        for a in 0..8 {
            glDisableVertexAttribArray(a);
        }
        glStencilFunc(GL_ALWAYS, 0, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
    }

    /// Case 5, `case_scissor` (gl_test.c:2358).
    pub unsafe fn case_scissor() -> Case {
        glUseProgram(link_program());
        upload(&FULL.iter().map(|c| cuv(c[0], c[1], 0.0, 1.0, 1.0, 0.0)).collect::<Vec<_>>());
        glDisable(GL_SCISSOR_TEST);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 32, 32);
        glClear(GL_COLOR_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        check_gl_error("draw")?;
        read_topdown();
        Ok(())
    }

    /// Case 58, `case_clearscis` (gl_test.c:8106).
    pub unsafe fn case_clearscis() -> Case {
        glUseProgram(link_program());
        upload(&solid_quad(0.0, 1.0, 0.0, 0.0));
        /* the whole target blue, then red inside a 32x32 box at GL (16,16) */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glEnable(GL_SCISSOR_TEST);
        glScissor(16, 16, 32, 32);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        check_gl_error("scissored clear")?;
        read_topdown();
        /* a scissored clear after a draw lands after it */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glEnable(GL_SCISSOR_TEST);
        glScissor(16, 16, 32, 32);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        check_gl_error("clear between draws")?;
        read_topdown();
        /* an empty scissor box clears nothing at all */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glEnable(GL_SCISSOR_TEST);
        glScissor(16, 16, 0, 32);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        read_topdown();
        check_gl_error("clearscis")
    }

    /// Case 59, `case_clearmask` (gl_test.c:8178).
    pub unsafe fn case_clearmask() -> Case {
        glUseProgram(link_program());
        /* colour: a clear with every channel masked off changes nothing */
        upload(&solid_quad(0.0, 1.0, 0.0, 0.0));
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glColorMask(0, 0, 0, 0);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glColorMask(1, 1, 1, 1);
        glFinish();
        check_gl_error("masked colour clear")?;
        read_topdown();
        /* depth: glDepthMask(GL_FALSE) then a depth clear changes nothing */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearDepthf(1.0);
        glDepthMask(1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        upload(&solid_quad(1.0, 0.0, 0.0, 0.0));
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glDepthMask(0);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glDepthMask(1);
        upload(&solid_quad(0.0, 1.0, 0.0, 0.5));
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        glDisable(GL_DEPTH_TEST);
        check_gl_error("masked depth clear")?;
        read_topdown();
        /* stencil: the write mask says which bits a clear may change */
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glStencilMask(0xff);
        glClearStencil(0x00);
        glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glStencilMask(0x0f);
        glClearStencil(0xff);
        glClear(GL_STENCIL_BUFFER_BIT);
        glStencilMask(0xff);
        glEnable(GL_STENCIL_TEST);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        upload(&solid_quad(0.0, 1.0, 0.0, 0.0));
        glStencilFunc(GL_EQUAL, 0xff, 0xff);
        glDrawArrays(GL_TRIANGLES, 0, 6); /* must not draw */
        glFinish();
        check_gl_error("masked stencil clear")?;
        read_topdown();
        glStencilFunc(GL_EQUAL, 0x0f, 0xff);
        glDrawArrays(GL_TRIANGLES, 0, 6); /* must draw */
        glFinish();
        glDisable(GL_STENCIL_TEST);
        check_gl_error("stencil read-back")?;
        read_topdown();
        check_gl_error("clearmask")
    }

    /// Case 66, `case_clearvp` (gl_test.c:9245).
    pub unsafe fn case_clearvp() -> Case {
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        /* step 1: the whole surface, white */
        glViewport(0, 0, TW, TH);
        glClearColor(1.0, 1.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        check_gl_error("clear at the full viewport")?;
        read_topdown();
        /* step 2: a quarter viewport, blue */
        glViewport(0, 0, TW / 2, TH / 2);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        check_gl_error("clear at a quarter viewport")?;
        read_topdown();
        /* step 3: the scissor does bound a clear */
        glViewport(0, 0, TW, TH);
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, TW / 2, TH / 2);
        glClearColor(0.0, 1.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        check_gl_error("scissored clear")?;
        read_topdown();
        Ok(())
    }

    /// `fbods_color_tex`.
    unsafe fn fbods_color_tex(format: GLenum) -> GLuint {
        let mut tex: GLuint = 0;
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, TW, TH, 0, format, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        tex
    }

    unsafe fn complete(what: &str) -> Case {
        match glCheckFramebufferStatus(GL_FRAMEBUFFER) {
            GL_FRAMEBUFFER_COMPLETE => Ok(()),
            s => Err(format!("{what} FBO status {s:#x}")),
        }
    }

    /// Case 67, `case_fbods` (gl_test.c:8727), the build without
    /// `GL_TEST_VIVANTE`: frames 1 to 8.
    pub unsafe fn case_fbods() -> Case {
        let (mut fbo_a, mut fbo_b, mut depth_a, mut depth_b): (GLuint, GLuint, GLuint, GLuint) = (0, 0, 0, 0);
        glUseProgram(link_program());
        /* 0..5: red at z = -0.5 (near). 6..11: green at z = +0.5 (far). */
        let mut v = solid_quad(1.0, 0.0, 0.0, -0.5);
        v.extend(solid_quad(0.0, 1.0, 0.0, 0.5));
        upload(&v);
        glViewport(0, 0, TW, TH);

        /* frame 1: colour only, and GL_DEPTH_TEST enabled */
        let tex = fbods_color_tex(GL_RGBA);
        glGenFramebuffers(1, &mut fbo_a);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        complete("colour-only")?;
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glDrawArrays(GL_TRIANGLES, 6, 6);
        glFinish();
        check_gl_error("no depth attachment")?;
        read_topdown();

        /* frame 2: the same framebuffer, and a stencil test */
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 1, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glClearStencil(0);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        check_gl_error("no stencil attachment")?;
        read_topdown();
        glDisable(GL_STENCIL_TEST);

        /* frame 3: a GL_RGB colour attachment */
        {
            let rgb_tex = fbods_color_tex(GL_RGB);
            let mut rgb_fbo: GLuint = 0;
            glGenFramebuffers(1, &mut rgb_fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, rgb_fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rgb_tex, 0);
            complete("RGB")?;
            glDisable(GL_DEPTH_TEST);
            glClearColor(1.0, 0.0, 0.0, 0.25);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
            check_gl_error("RGB clear")?;
            read_topdown();
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &rgb_fbo);
            glDeleteTextures(1, &rgb_tex);
        }

        /* frames 4 and 5: two framebuffer objects over one colour texture,
         * each with a depth buffer of its own */
        glGenRenderbuffers(1, &mut depth_a);
        glBindRenderbuffer(GL_RENDERBUFFER, depth_a);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);
        glGenRenderbuffers(1, &mut depth_b);
        glBindRenderbuffer(GL_RENDERBUFFER, depth_b);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_a);
        glGenFramebuffers(1, &mut fbo_b);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_b);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_b);
        complete("shared-colour")?;

        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        check_gl_error("shared colour, A")?;
        read_topdown();

        glBindFramebuffer(GL_FRAMEBUFFER, fbo_b);
        glDrawArrays(GL_TRIANGLES, 6, 6);
        glFinish();
        check_gl_error("shared colour, B")?;
        read_topdown();

        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo_b);
        glDeleteRenderbuffers(1, &depth_a);
        glDeleteRenderbuffers(1, &depth_b);
        glDisable(GL_DEPTH_TEST);

        /* frame 6: a scissored stencil-only clear */
        glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0);
        let mut sten: GLuint = 0;
        glGenRenderbuffers(1, &mut sten);
        glBindRenderbuffer(GL_RENDERBUFFER, sten);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, TW, TH);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, sten);
        complete("stencil")?;
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_SCISSOR_TEST);
        glScissor(16, 16, 32, 32);
        glClearStencil(1);
        glClear(GL_STENCIL_BUFFER_BIT);
        glDisable(GL_SCISSOR_TEST);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 0, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        glDisable(GL_STENCIL_TEST);
        check_gl_error("scissored stencil clear")?;
        read_topdown();

        /* frames 7 and 8: the same clear with a draw ahead of it, and the
         * stencil-testing draw in the same kick (7) or in the next one (8) */
        for step in 0..2 {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClearStencil(0);
            glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 6, 6);
            glEnable(GL_SCISSOR_TEST);
            glScissor(16, 16, 32, 32);
            glClearStencil(1);
            glClear(GL_STENCIL_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
            glClearStencil(0);
            if step == 1 {
                glFinish(); /* the box must survive a kick */
            }
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            glDisable(GL_STENCIL_TEST);
            check_gl_error(["stencil box, one kick", "stencil box across a kick"][step])?;
            read_topdown();
        }
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &sten);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo_a);
        glDeleteTextures(1, &tex);
        check_gl_error("fbods")
    }

    /// Case 102, `case_maskclear` (gl_test.c:23147).
    pub unsafe fn case_maskclear() -> Case {
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_BLEND);
        /* 1. full screen, R and A */
        glColorMask(1, 1, 1, 1);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glColorMask(1, 0, 0, 1);
        glClearColor(1.0, 1.0, 1.0, 0.5);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glColorMask(1, 1, 1, 1);
        check_gl_error("clear 1")?;
        read_topdown();
        /* 2. scissored, G only */
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 32, 32);
        glColorMask(0, 1, 0, 0);
        glClearColor(0.0, 1.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        check_gl_error("clear 2")?;
        read_topdown();
        /* 3. fused into a draw, B only */
        glUseProgram(link_program());
        let c = [[-1.0, -1.0], [0.0, -1.0], [0.0, 0.0], [-1.0, -1.0], [0.0, 0.0], [-1.0, 0.0]];
        upload(&c.iter().map(|c: &[f32; 2]| cuv(c[0], c[1], 0.0, 1.0, 1.0, 0.0)).collect::<Vec<_>>());
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glColorMask(0, 0, 1, 0);
        glClearColor(0.0, 0.0, 1.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glColorMask(1, 1, 1, 1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        check_gl_error("clear 3")?;
        read_topdown();
        Ok(())
    }
}

/* ---- the run ---------------------------------------------------------------------------- */

/// A test's GL: the rig's OSAL and table, the table initialised, and a
/// context with gl_test's 64x64 window surface, destroyed with it.
pub(crate) struct Gl {
    ctx: *mut GlContext,
    _rig: Rig,
}

impl Gl {
    pub(crate) fn new() -> Gl {
        let rig = rig();
        assert_eq!(backend::init(), 0, "{:?}", log_lines());
        // SAFETY: the OSAL and the table are installed; the rig's lock makes
        // this the only context.
        let ctx = unsafe { gl_context_create(TW as u32, TH as u32) };
        assert!(!ctx.is_null());
        // SAFETY: the context is current.
        unsafe { gl::reset_state() };
        Gl { ctx, _rig: rig }
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        // SAFETY: the context `new` made; nothing holds a reference to it.
        unsafe { gl_context_destroy(self.ctx) };
    }
}

/// The refusal lines the table logged.
pub(crate) fn refusals() -> Vec<String> {
    logs_with(REFUSAL_PREFIX)
}

/// A recorded case: every GL call of it went through with no GL error and
/// no refusal, and it submitted the reference's jobs.
pub(crate) fn recorded(shape: &str, case: CaseFn, ref_jobs_total: usize) -> (Gl, Vec<Kick>, Vec<Ours>) {
    let g = Gl::new();
    // SAFETY: `g`'s context is current.
    let r = unsafe { case() };
    assert_eq!(r, Ok(()), "{shape}: {:?}", log_lines());
    assert_eq!(table71::refused_draws(), 0, "{shape}: {:?}", refusals());
    assert!(refusals().is_empty() && logs_with("GL error").is_empty(), "{shape}: {:?}", log_lines());
    assert_eq!(logs_with("clear program built").len(), 1, "{shape}: {:?}", log_lines());
    let ks = kicks();
    assert_eq!(ks.len(), ref_jobs_total, "{shape}: one job per glFinish, as the reference");
    let ours = check_against_mesa(shape, &ks, ref_jobs_total);
    (g, ks, ours)
}

/// A case refused by name: it stops at the check `stops_at` with
/// `GL_INVALID_OPERATION`, one draw is refused, the log names
/// [`READS_STENCIL`] as the 4.2 table does (the string alone after the
/// prefix) and the GL core says which call was not recorded; the `jobs`
/// before it were submitted and are the reference's first ones.
pub(crate) fn refused(shape: &str, case: CaseFn, stops_at: &str, jobs: usize, ref_jobs_total: usize) -> Vec<Ours> {
    let _g = Gl::new();
    // SAFETY: the context is current.
    let r = unsafe { case() };
    assert_eq!(r, Err(format!("{stops_at}: GL error 0x0502")), "{shape}: {:?}", log_lines());
    assert_eq!(table71::refused_draws(), 1, "{shape}: {:?}", refusals());
    assert_eq!(refusals(), [format!("{REFUSAL_PREFIX}{READS_STENCIL}")], "{shape}");
    assert_eq!(logs_with("glDrawArrays was not recorded (-95)").len(), 1, "{shape}: {:?}", log_lines());
    assert!(logs_with("alpha bits").is_empty(), "{shape}: {:?}", log_lines());
    assert_eq!(osal_counts().0, jobs, "{shape}: the jobs before the refused draw");
    check_against_mesa(shape, &kicks(), ref_jobs_total)
}

/// The state packet `name` in effect at draw `n` of a job's binning list.
fn state_at(o: &Ours, n: usize, name: &str) -> Pk {
    let (_, draws, _) = bcl_split(&o.bcl);
    draws[n].0.get(name).unwrap_or_else(|| panic!("no {name} at draw {n}")).clone()
}

/// The draw packet of draw `n`.
fn draw_packet(o: &Ours, n: usize) -> Pk {
    bcl_split(&o.bcl).1[n].1.clone()
}

/// The four vertices a clear quad's attribute record names, as the job
/// would read them.
fn quad_vertices(k: &Kick, o: &Ours, n: usize) -> [[f32; 4]; 4] {
    assert_eq!(o.attr_addrs[n].len(), 1, "the quad has one array");
    let at = o.attr_addrs[n][0];
    assert_eq!(at % 16, 0);
    let b = k.at(at, 64);
    core::array::from_fn(|v| core::array::from_fn(|c| f32::from_le_bytes(b[v * 16 + c * 4..][..4].try_into().unwrap())))
}

/// A box in clip space as fill71 writes a quad: TRIANGLE_FAN order.
fn quad_box(x0: f32, y0: f32, x1: f32, y1: f32) -> [[f32; 4]; 4] {
    [[x0, y0, 0.0, 1.0], [x1, y0, 0.0, 1.0], [x1, y1, 0.0, 1.0], [x0, y1, 0.0, 1.0]]
}

fn is_quad(o: &Ours, n: usize) -> bool {
    let p = draw_packet(o, n);
    p.name == "VERTEX_ARRAY_PRIMS" && p.get("mode") == Some("6") && p.get("Length") == Some("4")
}

/// Case 5: recorded. Job 0 is the whole clear; job 1 the scissored clear as
/// a quad over GL's bottom-left 32x32 and the scissored triangles.
#[test]
fn case_5_scissor_is_recorded_as_mesas_jobs() {
    let (_g, ks, ours) = recorded("scissor", gl::case_scissor, 2);
    assert!(ours[0].records.is_empty() && ours[0].tiles.iter().any(|p| p.name == "CLEAR_RT"));
    assert_eq!(ours[1].records.len(), 2, "the clear quad and the draw");
    assert!(is_quad(&ours[1], 0) && !is_quad(&ours[1], 1));
    assert_eq!(quad_vertices(&ks[1], &ours[1], 0), quad_box(-1.0, -1.0, 0.0, 0.0), "the scissor box");
    /* the quad's own state is unscissored (the box is its vertices) */
    assert_eq!(state_at(&ours[1], 0, "CLIP").get("Clip Window Height in pixels"), Some("64"));
    assert_eq!(state_at(&ours[1], 1, "CLIP").get("Clip Window Height in pixels"), Some("32"));
    assert!(ours[1].tiles.iter().any(|p| p.name == "LOAD"), "job 1 loads job 0's clear");
}

/// Case 58: recorded. The scissored clear is a quad, after the draw in the
/// second job; the empty box of the third makes no draw.
#[test]
fn case_58_clear_with_scissor_is_recorded_as_mesas_jobs() {
    let (_g, ks, ours) = recorded("scisbox", gl::case_clearscis, 3);
    assert_eq!(ours.iter().map(|o| o.records.len()).collect::<Vec<_>>(), [1, 2, 0]);
    assert!(is_quad(&ours[0], 0));
    assert_eq!(quad_vertices(&ks[0], &ours[0], 0), quad_box(-0.5, -0.5, 0.5, 0.5), "the box at (16,16), 32x32");
    assert!(!is_quad(&ours[1], 0) && is_quad(&ours[1], 1), "the clear lands after the draw");
    assert_eq!(quad_vertices(&ks[1], &ours[1], 1), quad_box(-0.5, -0.5, 0.5, 0.5));
    /* each job clears the whole target first (the tile buffer's clear) */
    assert!(ours.iter().all(|o| !o.tiles.iter().any(|p| p.name == "LOAD")));
}

/// Case 66: recorded. The viewport bounds no clear (two tile-buffer
/// clears); the scissor does (a quad over the bottom-left quarter).
#[test]
fn case_66_clear_not_bounded_by_the_viewport_is_recorded_as_mesas_jobs() {
    let (_g, ks, ours) = recorded("viewport", gl::case_clearvp, 3);
    assert_eq!(ours.iter().map(|o| o.records.len()).collect::<Vec<_>>(), [0, 0, 1]);
    assert!(is_quad(&ours[2], 0));
    assert_eq!(quad_vertices(&ks[2], &ours[2], 0), quad_box(-1.0, -1.0, 0.0, 0.0));
}

/// Case 102: recorded. The clear under the R|A mask is a quad over the whole
/// target with that mask, the scissored G clear a quad over its box, and the
/// B clear a quad ahead of the draw it is followed by.
#[test]
fn case_102_clear_under_a_partial_colour_mask_is_recorded_as_mesas_jobs() {
    let (_g, ks, ours) = recorded("colormask", gl::case_maskclear, 6);
    assert_eq!(ours.iter().map(|o| o.records.len()).collect::<Vec<_>>(), [0, 1, 0, 1, 0, 2]);
    /* COLOR_WMASKS holds the channels NOT written, four bits a target, the
     * one mask for all eight (the reference's values) */
    let mask = |job: usize, n: usize| state_at(&ours[job], n, "COLOR_WMASKS").get("Mask").map(str::to_string);
    assert_eq!(quad_vertices(&ks[1], &ours[1], 0), quad_box(-1.0, -1.0, 1.0, 1.0), "the whole target");
    assert_eq!(mask(1, 0).as_deref(), Some("1717986918"), "0x66666666: R and A written");
    assert_eq!(quad_vertices(&ks[3], &ours[3], 0), quad_box(-1.0, -1.0, 0.0, 0.0), "the scissor box");
    assert_eq!(mask(3, 0).as_deref(), Some("3722304989"), "0xdddddddd: G written");
    assert!(is_quad(&ours[5], 0) && !is_quad(&ours[5], 1));
    assert_eq!(mask(5, 0).as_deref(), Some("3149642683"), "0xbbbbbbbb: B written");
    assert_eq!(mask(5, 1).as_deref(), Some("0"), "the draw writes every channel");
}

/// Case 59: **refused by name**. The colour part, the depth part and the
/// stencil clears with the draw testing `EQUAL 0xff` are three jobs, the
/// reference's first three; the clear through `glStencilMask(0x0f)` is a
/// quad that replaces through the mask. The last draw tests the stencil
/// that third job wrote: the window's depth/stencil is in the tile buffer
/// only, so it is refused with the 4.2 table's string and the case stops at
/// its "stencil read-back" check.
#[test]
fn case_59_clear_with_write_masks_is_refused_by_the_stencil_name() {
    let ours = refused("writemask", gl::case_clearmask, "stencil read-back", 3, 4);
    assert_eq!(ours.iter().map(|o| o.records.len()).collect::<Vec<_>>(), [0, 2, 2]);
    assert!(is_quad(&ours[2], 0) && !is_quad(&ours[2], 1), "the masked stencil clear, then the draw");
}

/// Case 67: **refused by name**, at frame 8. Frames 1 to 7 are eight jobs,
/// the reference's first eight, the GL_RGB frame (job 2) among them with
/// alpha 1: no refusal speaks of alpha bits. Frame 8's draw tests the
/// stencil box across a `glFinish` and is refused as case 59's.
#[test]
fn case_67_fbo_limits_is_refused_by_the_stencil_name_at_frame_8() {
    let ours = refused("fbods", gl::case_fbods, "stencil box across a kick", 8, 9);
    assert_eq!(ours.iter().map(|o| o.records.len()).collect::<Vec<_>>(), [2, 1, 0, 1, 1, 2, 3, 2]);
    /* frame 3: the GL_RGB target is cleared with the tile buffer */
    assert!(ours[2].tiles.iter().any(|p| p.name == "CLEAR_RT"));
}

/// The board's run: the six cases in `CONFIG_V3D_GL_TEST_CASES`' order in
/// one context, `reset_state` before each. Four are recorded, 59 and 67 are
/// refused by the one name, and a refused case leaves nothing behind that
/// changes the cases after it.
#[test]
fn the_six_cases_in_the_boards_order_give_four_recorded_and_two_named_refusals() {
    let _g = Gl::new();
    let cases: [(u32, CaseFn, Case, usize); 6] = [
        (5, gl::case_scissor, Ok(()), 2),
        (58, gl::case_clearscis, Ok(()), 3),
        (59, gl::case_clearmask, Err("stencil read-back: GL error 0x0502".to_string()), 3),
        (66, gl::case_clearvp, Ok(()), 3),
        (67, gl::case_fbods, Err("stencil box across a kick: GL error 0x0502".to_string()), 8),
        (102, gl::case_maskclear, Ok(()), 6),
    ];
    let (mut jobs, mut refused) = (0, 0);
    for (n, case, verdict, kicks) in cases {
        // SAFETY: the context is current.
        let r = unsafe {
            gl::reset_state();
            case()
        };
        assert_eq!(r, verdict, "case {n}: {:?}", log_lines());
        jobs += kicks;
        refused += u32::from(verdict.is_err());
        assert_eq!(osal_counts().0, jobs, "case {n}: its jobs");
        assert_eq!(table71::refused_draws(), refused, "case {n}: {:?}", refusals());
    }
    /* one cause, logged once (the second is counted) */
    assert_eq!(refusals(), [format!("{REFUSAL_PREFIX}{READS_STENCIL}")]);
    assert_eq!(logs_with("clear program built").len(), 1, "one clear program for the run");
}

/// Two clear quads in one job have a slot each in the data arena: two
/// scissored clears in one flush keep their own boxes.
#[test]
fn two_clear_quads_of_one_job_keep_their_own_vertices() {
    let g = Gl::new();
    // SAFETY: the context is current.
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 32, 32);
        glClear(GL_COLOR_BUFFER_BIT);
        glFlush();
        glScissor(32, 32, 32, 32);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
    }
    assert_eq!(check_gl_error("two quads"), Ok(()));
    assert_eq!(table71::refused_draws(), 0, "{:?}", refusals());
    let ks = kicks();
    let defs = packet_defs();
    let quads: Vec<[[f32; 4]; 4]> = ks
        .iter()
        .flat_map(|k| {
            let o = decode_job(&defs, k, "job");
            (0..o.records.len()).map(|n| quad_vertices(k, &o, n)).collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(quads, [quad_box(-1.0, -1.0, 0.0, 0.0), quad_box(0.0, 0.0, 1.0, 1.0)]);
    drop(g);
}
