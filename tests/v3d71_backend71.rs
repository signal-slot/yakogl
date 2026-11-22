// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/backend71.rs` (the V3D 7.1 job assembly: init identity check,
//! frame admission, clear plan, depth/stencil decision, uniform streams, D0
//! record, attribute records, the C job builder) against Mesa's 7.1
//! references for the Pi 5's two board jobs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_backend71
//! ```
//!
//! The C builder (driver/src/v3d/v3d_job71.c over v3d_frame.c at
//! `-DV3D_CL_VER=71`) runs in a child process, tests/v3d71_backend71_harness.c,
//! built here with `$CC` or `cc` (a test binary cannot link C without a build
//! script); it implements `backend71::Builder71` for the test. Every buffer
//! sits at the board dump's address, so the jobs are compared with their
//! addresses too:
//!
//! * `clear`: tools/v3d/pi5/clear.clif, glClearColor(0, 0, 1, 1) on a 64x64
//!   GL_RGBA8 target (the tile-buffer clear);
//! * `triangle`: tools/v3d/pi5/triangle.clif, the same clear then one
//!   triangle with Mesa's 7.1 programs (tools/v3d/tri-shaders71/
//!   mesa_passthrough_interp_color.h, read here as data).
//!
//! The binning, render and generic tile lists are split into packets with
//! the lengths of driver/src/v3d/generated/v3d_packet_v71_pack.h and decoded
//! through the field positions of Mesa's packet XML
//! (third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml, the version 71
//! packets); the packet sequence of each list must be the dump's, and every
//! value the dump prints (addresses included) must be the packet's. The D0
//! shader record, the attribute records and the uniform streams are compared
//! with the dump as well (the record's flag word against the header's
//! `MESA71_TRI_REC_FLAGS`: Mesa's decoder prints the record at the plain 7.1
//! layout, so its flag lines are not read). The refusal tests check the
//! identity and limit refusals by their names.

#[path = "../src/v3d/admit71.rs"]
#[allow(dead_code)]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(dead_code, clippy::if_same_then_else)]
mod attr71;
#[path = "../src/v3d/backend71.rs"]
#[allow(dead_code)]
mod backend71;
#[path = "../src/v3d/clearplan71.rs"]
#[allow(dead_code)]
mod clearplan71;
#[path = "../src/v3d/ffi71.rs"]
#[allow(dead_code)]
mod ffi71;
#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/ident71.rs"]
#[allow(dead_code)]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
#[allow(dead_code)]
mod limits71;
#[path = "../src/v3d/record71.rs"]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(dead_code, clippy::eq_op)]
mod rt71;
#[path = "../src/v3d/uniform71.rs"]
#[allow(dead_code)]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
#[allow(dead_code)]
mod zskeep71;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use backend71::{Builder71, List71};

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

// ---- the C builder in a child process ---------------------------------------

const HARNESS_C: &str = include_str!("v3d71_backend71_harness.c");

fn find_cc() -> String {
    for c in [std::env::var("CC").unwrap_or_default(), "cc".to_string()] {
        let Some(prog) = c.split_whitespace().next() else { continue };
        if Command::new(prog).arg("--version").output().is_ok() {
            return c;
        }
    }
    panic!("no C compiler ($CC or cc): this test runs driver/src/v3d/v3d_job71.c");
}

fn harness() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let cc = find_cc();
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d71_backend71");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("harness");
        let src = dir.join("harness.c");
        std::fs::write(&src, HARNESS_C).unwrap();
        let mesa = format!("{ROOT}/third_party/mesa-26.2.1");
        let mut parts = cc.split_whitespace();
        let out = Command::new(parts.next().unwrap())
            .args(parts)
            .args(["-O1", "-std=c11", "-Wall", "-Wextra", "-Werror", "-D_GNU_SOURCE", "-DHAVE_ENDIAN_H"])
            .arg("-DV3D_CL_VER=71")
            .args(["-isystem", &format!("{mesa}/src"), "-isystem", &format!("{mesa}/include")])
            .args(["-isystem", &format!("{mesa}/src/broadcom")])
            .arg(format!("-I{ROOT}/driver/src/v3d"))
            .arg(format!("-I{ROOT}/driver/src/v3d/generated"))
            .arg(&src)
            .arg(format!("{ROOT}/driver/src/v3d/v3d_job71.c"))
            .arg(format!("{ROOT}/driver/src/v3d/v3d_frame.c"))
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run the C compiler");
        assert!(out.status.success(), "harness build failed:\n{}", String::from_utf8_lossy(&out.stderr));
        exe
    })
}

/// The raw bytes of a `#[repr(C)]` plain struct.
fn raw<T>(v: &T) -> Vec<u8> {
    // SAFETY: a plain repr(C) struct read as bytes; padding is never read by C.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()).to_vec() }
}

fn raw_slice<T>(v: &[T]) -> Vec<u8> {
    // SAFETY: as `raw`, for a slice of plain structs or words.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)).to_vec() }
}

const CL_SIZE: u32 = 0x4000;

/// `Builder71` over the harness: the lists at the dump's addresses.
struct Harness {
    gpu: [u32; 3],
    lists: [Vec<u8>; 3],
}

fn li(l: List71) -> usize {
    match l {
        List71::Bcl => 0,
        List71::Rcl => 1,
        List71::Ind => 2,
    }
}

struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        a
    }
    fn u32(&mut self) -> u32 {
        u32::from_ne_bytes(self.take(4).try_into().unwrap())
    }
}

impl Builder71 for Harness {
    fn gpu(&self, l: List71) -> u32 {
        self.gpu[li(l)]
    }

    fn build(
        &mut self,
        target: &ffi71::V3dJob71Target,
        draws: &[backend71::Draw71<'_>],
        clear: &ffi71::V3dJob71Clear,
    ) -> Result<backend71::Built71, backend71::BuildError71> {
        let mut inp = vec![];
        let w = |v: &mut Vec<u8>, x: u32| v.extend(x.to_ne_bytes());
        w(&mut inp, 0x3731_4a42);
        w(&mut inp, draws.len() as u32);
        inp.extend(raw(target));
        inp.extend(raw(clear));
        for x in [self.gpu[0], self.gpu[1], self.gpu[2], CL_SIZE, backend71::PAGE_SIZE] {
            w(&mut inp, x);
        }
        for d in draws {
            inp.extend(raw(&d.st));
            inp.extend(raw(&d.fs));
            inp.extend(raw(&d.vs));
            inp.extend(raw(&d.cs));
            for x in [d.prim, d.count, d.first] {
                w(&mut inp, x);
            }
            for n in [d.fs_uniforms.len(), d.vs_uniforms.len(), d.cs_uniforms.len(), d.fs_gl.len(), d.vs_gl.len()] {
                w(&mut inp, n as u32);
            }
            w(&mut inp, d.attrs.len() as u32);
            inp.extend(raw_slice(d.fs_uniforms));
            inp.extend(raw_slice(d.vs_uniforms));
            inp.extend(raw_slice(d.cs_uniforms));
            inp.extend(raw_slice(d.fs_gl));
            inp.extend(raw_slice(d.vs_gl));
            inp.extend(raw_slice(d.attrs));
        }
        let mut child = Command::new(harness()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&inp).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "harness exit {:?}", out.status.code());
        let mut r = Rd(&out.stdout);
        let status = r.u32();
        let at = r.u32() as usize;
        let code = r.u32() as i32;
        let tb: [u32; 8] = core::array::from_fn(|_| r.u32());
        let tiling = ffi71::V3dTiling {
            tile_w: tb[0],
            tile_h: tb[1],
            tiles_x: tb[2],
            tiles_y: tb[3],
            supertile_w: tb[4],
            supertile_h: tb[5],
            frame_w_supertiles: tb[6],
            frame_h_supertiles: tb[7],
        };
        let n = r.u32();
        let layouts = (0..n)
            .map(|_| backend71::DrawLayout71 {
                fs_uniforms: r.u32(),
                vs_uniforms: r.u32(),
                cs_uniforms: r.u32(),
                record: r.u32(),
                attr_records: r.u32(),
            })
            .collect();
        let (ls, le, qms, qts) = (r.u32(), r.u32(), r.u32(), r.u32());
        for l in &mut self.lists {
            let used = r.u32() as usize;
            *l = r.take(used).to_vec();
        }
        match status {
            0 => Ok(backend71::Built71 {
                tiling,
                layouts,
                tile_list: (ls, le),
                tile_alloc_size: qms,
                tile_state_size: qts,
            }),
            1 => Err(backend71::BuildError71::Begin),
            2 => Err(backend71::BuildError71::Draw { index: at, code }),
            _ => Err(backend71::BuildError71::End),
        }
    }

    fn bytes(&self, l: List71) -> &[u8] {
        &self.lists[li(l)]
    }
}

// ---- Mesa's packet XML, version 71 ------------------------------------------

#[derive(Clone, Debug)]
struct XField {
    name: String,
    start: u32,
    size: u32,
    ty: String,
    minus_one: bool,
    default: Option<u64>,
}

#[derive(Clone, Debug)]
struct XPacket {
    code: u8,
    clif: String,
    fields: Vec<XField>,
}

fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!(" {key}=\"");
    let at = line.find(&pat)? + pat.len();
    Some(&line[at..at + line[at..].find('"')?])
}

fn clif_name(name: &str, short: Option<&str>) -> String {
    if let Some(s) = short {
        return s.to_ascii_uppercase();
    }
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn xml_packets() -> Vec<XPacket> {
    let text = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let mut out = vec![];
    let mut cur: Option<XPacket> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("<packet") {
            let ver_ok =
                attr(t, "max_ver").is_none_or(|v| num(v) >= 71) && attr(t, "min_ver").is_none_or(|v| num(v) <= 71);
            let p = XPacket {
                code: num(attr(t, "code").unwrap()) as u8,
                clif: clif_name(attr(t, "name").unwrap(), attr(t, "shortname")),
                fields: vec![],
            };
            if t.ends_with("/>") {
                if ver_ok {
                    out.push(p);
                }
            } else if ver_ok {
                cur = Some(p);
            } else {
                cur = None;
                // skip to its </packet>: a later <field> outside cur is dropped
            }
        } else if t.starts_with("</packet>") {
            if let Some(p) = cur.take() {
                out.push(p);
            }
        } else if t.starts_with("<field") {
            if let Some(p) = cur.as_mut() {
                let s = attr(t, "start").unwrap();
                let start = match s.strip_suffix('b') {
                    Some(b) => num(b) as u32 * 8,
                    None => num(s) as u32,
                };
                p.fields.push(XField {
                    name: attr(t, "name").unwrap().to_string(),
                    start,
                    size: num(attr(t, "size").unwrap()) as u32,
                    ty: attr(t, "type").unwrap_or("uint").to_string(),
                    minus_one: attr(t, "minus_one") == Some("true"),
                    default: attr(t, "default").map(num),
                });
            }
        }
    }
    out
}

fn packets() -> &'static Vec<XPacket> {
    static P: OnceLock<Vec<XPacket>> = OnceLock::new();
    P.get_or_init(xml_packets)
}

/// opcode -> packet length, from the generated 7.1 pack header.
fn lengths() -> &'static BTreeMap<u8, usize> {
    static L: OnceLock<BTreeMap<u8, usize>> = OnceLock::new();
    L.get_or_init(|| {
        let text = read("driver/src/v3d/generated/v3d_packet_v71_pack.h");
        let mut op: BTreeMap<String, u8> = BTreeMap::new();
        let mut len: BTreeMap<String, usize> = BTreeMap::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next() != Some("#define") {
                continue;
            }
            let (Some(name), Some(v)) = (it.next(), it.next()) else { continue };
            let Some(name) = name.strip_prefix("V3D71_") else { continue };
            if let Some(n) = name.strip_suffix("_opcode") {
                op.insert(n.to_string(), num(v) as u8);
            } else if let Some(n) = name.strip_suffix("_length") {
                len.insert(n.to_string(), num(v) as usize);
            }
        }
        let mut m = BTreeMap::new();
        for (n, o) in op {
            let l = len[&n];
            if let Some(prev) = m.insert(o, l) {
                assert_eq!(prev, l, "two lengths for opcode {o}");
            }
        }
        m
    })
}

fn split(list: &[u8]) -> Vec<&[u8]> {
    let mut out = vec![];
    let mut at = 0;
    while at < list.len() {
        let n = *lengths().get(&list[at]).unwrap_or_else(|| panic!("opcode {} at +{at:#x}", list[at]));
        out.push(&list[at..at + n]);
        at += n;
    }
    assert_eq!(at, list.len(), "a packet runs past the list's end");
    out
}

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

/// One packet (or record) as the dump prints it: name and `(label, value)`.
#[derive(Clone, Debug, PartialEq)]
struct Pk {
    name: String,
    fields: Vec<(String, String)>,
}

fn hex(v: u64) -> String {
    format!("0x{v:08x}")
}

/// A packet's bytes as the dump's text, through the XML.
fn decode(bytes: &[u8]) -> Pk {
    let body = &bytes[1..];
    let p = packets()
        .iter()
        .filter(|p| p.code == bytes[0])
        .find(|p| match p.fields.iter().find(|f| f.name == "sub-id") {
            Some(f) => bits(body, f.start, f.size) == f.default.unwrap(),
            None => true,
        })
        .unwrap_or_else(|| panic!("no XML packet for opcode {}", bytes[0]));
    let mut fields = vec![];
    for f in &p.fields {
        let v = bits(body, f.start, f.size);
        let text = match f.ty.as_str() {
            "address" => {
                let base = f.start + f.size - 32;
                hex(bits(body, base, 32) & !((1u64 << (32 - f.size)) - 1))
            }
            "float" => format!("{:.6}", f32::from_bits(v as u32)),
            "f187" => format!("{:.6}", f32::from_bits((v as u32) << 16)),
            "int" => {
                let sh = 64 - f.size;
                (((v << sh) as i64) >> sh).to_string()
            }
            _ => (v + f.minus_one as u64).to_string(),
        };
        fields.push((f.name.clone(), text));
    }
    Pk { name: p.clif.clone(), fields }
}

// ---- the dump -----------------------------------------------------------------

/// A dump value: the first token, an address as its hex (`[null]` is 0).
fn dump_value(rest: &str) -> String {
    if rest.starts_with("[null]") {
        return hex(0);
    }
    if rest.starts_with('[') {
        let h = rest.split("/*").nth(1).and_then(|s| s.split("*/").next()).unwrap().trim();
        return hex(num(h));
    }
    rest.split_whitespace().next().unwrap_or("").to_string()
}

#[derive(Default)]
struct Dump {
    /// buffer name -> its control-list packets in file order
    lists: BTreeMap<String, Vec<Pk>>,
    /// the shader records and attribute records, in file order
    records: Vec<Pk>,
    attrs: Vec<Pk>,
    /// (buffer, byte offset) -> word of the binary sections
    words: BTreeMap<(String, u32), u32>,
}

fn parse_dump(file: &str) -> Dump {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let mut d = Dump::default();
    let (mut buf, mut fmt, mut off) = (String::new(), String::new(), 0u32);
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MESA: info:") else { continue };
        let t = rest.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(b) = t.strip_prefix("@buffer ") {
            buf = b.split_whitespace().next().unwrap().to_string();
            fmt.clear();
            continue;
        }
        if let Some(f) = t.strip_prefix("@format ") {
            fmt = f.split_whitespace().next().unwrap_or("").to_string();
            match fmt.as_str() {
                "binary" => {
                    let a = f.split('+').nth(1).and_then(|s| s.split(']').next()).unwrap();
                    off = num(a) as u32;
                }
                "shadrec_gl_main" => d.records.push(Pk { name: "SHADREC".into(), fields: vec![] }),
                "shadrec_gl_attr" => d.attrs.push(Pk { name: "ATTR".into(), fields: vec![] }),
                _ => {}
            }
            continue;
        }
        if t.starts_with('@') {
            fmt.clear();
            continue;
        }
        if fmt == "binary" {
            for w in t.split_whitespace().filter(|w| w.len() == 10 && w.starts_with("0x")) {
                d.words.insert((buf.clone(), off), num(w) as u32);
                off += 4;
            }
            continue;
        }
        if let Some(body) = t.strip_prefix("/*") {
            let Some(at) = body.find(": */") else { continue };
            let field = (body[..at].trim().to_string(), dump_value(body[at + 4..].trim()));
            let target = match fmt.as_str() {
                "ctrllist" => d.lists.get_mut(&buf).and_then(|v| v.last_mut()),
                "shadrec_gl_main" => d.records.last_mut(),
                "shadrec_gl_attr" => d.attrs.last_mut(),
                _ => None,
            };
            if let Some(p) = target {
                p.fields.push(field);
            }
        } else if fmt == "ctrllist" && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            d.lists.entry(buf.clone()).or_default().push(Pk { name: t.to_string(), fields: vec![] });
        }
    }
    d
}

/// Every value the dump prints equals ours.
fn assert_same(what: &str, got: &Pk, want: &Pk) {
    assert_eq!(got.name, want.name, "{what}: packet");
    for (label, value) in &want.fields {
        let g = got
            .fields
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("{what}: {} has no field {label:?}", want.name));
        assert_eq!(&g.1, value, "{what}: {} / {label}", want.name);
    }
}

fn assert_list(what: &str, ours: &[u8], want: &[Pk]) -> usize {
    let got: Vec<Pk> = split(ours).into_iter().map(decode).collect();
    let names = |v: &[Pk]| v.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&got), names(want), "{what}: the packet sequence");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_same(&format!("{what} #{i}"), g, w);
    }
    got.len()
}

// ---- the board's jobs -----------------------------------------------------------

const GL_RGBA8: u32 = 0x8058;
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// Where the dump put its buffers (tools/v3d/job71_check.c CLEAR_PLACE and
/// TRI_PLACE, the dumps' `@createbuf` names).
struct Place {
    code: u32,
    target: u32,
    bcl: u32,
    vertex: u32,
    ind: u32,
    tile_alloc: u32,
    rcl: u32,
}

const CLEAR_PLACE: Place =
    Place { code: 0, target: 0x9000, bcl: 0x11000, vertex: 0, ind: 0x21000, tile_alloc: 0x15000, rcl: 0x1d000 };
const TRI_PLACE: Place = Place {
    code: 0x1000,
    target: 0x9000,
    bcl: 0x11000,
    vertex: 0x15000,
    ind: 0x119000,
    tile_alloc: 0x11d000,
    rcl: 0x125000,
};

fn table() -> backend71::Table71 {
    // HUB_IDENT1 and HUB_IDENT3 of the board (tools/v3d/pi5/README.md)
    backend71::init(0x0008_1117, 0x0002_0a10).expect("the Pi 5's identity")
}

/// fill71's descriptor for a w x h frame of `n` GL_RGBA8 UIF targets.
fn rgba8_target(w: u32, h: u32, n: u32, p: &Place) -> ffi71::V3dJob71Target {
    let mut t = ffi71::V3dJob71Target { width: w, height: h, tile_alloc: p.tile_alloc, rts: n, ..Default::default() };
    for i in 0..n.min(8) as usize {
        t.rt[i] = ffi71::V3dJob71Rt {
            address: p.target + 0x10_0000 * i as u32,
            format: ffi71::V3D_OUTPUT_IMAGE_FORMAT_RGBA8,
            memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub_or_stride: h.div_ceil(8),
            ..Default::default()
        };
    }
    t.depth_type = ffi71::V3D_INTERNAL_TYPE_DEPTH_32F;
    t
}

fn color(clear: bool) -> admit71::ColorTarget {
    admit71::ColorTarget {
        gl_format: GL_RGBA8,
        uses: admit71::BufferUse { load: false, store: true, clear },
        clear_color: clear.then_some(rt71::ClearColor::Float(BLUE)),
        sampled: false,
    }
}

fn gl_frame(w: u32, h: u32, colors: &[admit71::ColorTarget]) -> admit71::Frame<'_> {
    admit71::Frame { width: w, height: h, colors, depth_stencil: None, msaa: false, double_buffer: false }
}

/// glClear(GL_COLOR_BUFFER_BIT) of every target, no scissor, full masks.
fn color_clear(w: u32, h: u32, n: usize) -> clearplan71::ClearIn {
    let off = clearplan71::TargetIn { cleared: false, mask: 0 };
    let mut targets = [off; clearplan71::MAX_TARGETS];
    for t in targets.iter_mut().take(n) {
        *t = clearplan71::TargetIn { cleared: true, mask: clearplan71::MASK_RGBA };
    }
    clearplan71::ClearIn {
        fb_w: w,
        fb_h: h,
        n_targets: n,
        targets,
        has_depth: false,
        has_stencil: false,
        clear_depth: false,
        clear_stencil: false,
        depth_mask: true,
        stencil_mask: 0xff,
        scissor: clearplan71::ScissorIn { enabled: false, x: 0, y: 0, w: 0, h: 0 },
        ds_valid: false,
        draws_queued: false,
    }
}

fn harness_at(p: &Place) -> Harness {
    Harness { gpu: [p.bcl, p.rcl, p.ind], lists: Default::default() }
}

/// The Mesa triangle programs' header as numbers: `#define`s and arrays.
struct CHeader {
    defs: BTreeMap<String, u64>,
    arrays: BTreeMap<String, Vec<u64>>,
}

fn c_header(rel: &str) -> CHeader {
    let text = read(rel);
    let mut h = CHeader { defs: BTreeMap::new(), arrays: BTreeMap::new() };
    let mut cur: Option<(String, Vec<u64>)> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some((name, mut v)) = cur.take() {
            if t.starts_with("};") {
                h.arrays.insert(name, v);
                continue;
            }
            for tok in t.split([',', '{', '}', ' ']).filter(|s| !s.is_empty()) {
                v.push(num(tok));
            }
            cur = Some((name, v));
            continue;
        }
        if let Some(d) = t.strip_prefix("#define ") {
            let mut it = d.split_whitespace();
            if let (Some(n), Some(v)) = (it.next(), it.next()) {
                if v.starts_with(|c: char| c.is_ascii_digit()) {
                    h.defs.insert(n.to_string(), num(v));
                }
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

const TRI_HEADER: &str = "tools/v3d/tri-shaders71/mesa_passthrough_interp_color.h";

/// Mesa's triangle program, as the 7.1 table would get it from a link.
struct TriProgram {
    fs: ffi71::V3dFsProg,
    vs: ffi71::V3dVsProg,
    cs: ffi71::V3dVsProg,
    vs_u: Vec<ffi71::V3dUniform>,
    cs_u: Vec<ffi71::V3dUniform>,
    attrs: Vec<ffi71::V3dVertexAttr>,
    flags: u32,
}

fn uniforms(h: &CHeader, stage: &str) -> Vec<ffi71::V3dUniform> {
    let words = &h.arrays[&format!("mesa71_tri_{stage}_uniform")];
    let kinds = &h.arrays[&format!("mesa71_tri_{stage}_uniform_kind")];
    words
        .iter()
        .zip(kinds)
        .map(|(&w, &k)| ffi71::V3dUniform {
            contents: k as u32,
            data: if k as u32 == uniform71::CONSTANT { w as u32 } else { 0 },
        })
        .collect()
}

fn tri_program(p: &Place) -> TriProgram {
    let h = c_header(TRI_HEADER);
    // FS, VS, CS back to back from code + 0x60 (the dump's addresses)
    let fs_code = p.code + 0x60;
    let vs_code = fs_code + 8 * h.d("MESA71_TRI_FS_CODE_COUNT");
    let cs_code = vs_code + 8 * h.d("MESA71_TRI_VS_CODE_COUNT");
    let threads = |n: &str| if h.b(n) { 4 } else { 2 };
    // SAFETY: plain C structs, all-zero is valid (null pointers, false, 0)
    let mut fs: ffi71::V3dFsProg = unsafe { std::mem::zeroed() };
    fs.code = fs_code;
    fs.threads = threads("MESA71_TRI_FS_FOUR_THREADS");
    fs.single_seg = h.b("MESA71_TRI_FS_FINAL_THREAD_SECTION");
    fs.num_inputs = h.d("MESA71_TRI_REC_VARYINGS");
    fs.writes_z = h.b("MESA71_TRI_REC_FS_Z_WRITES");
    fs.disable_ez = h.b("MESA71_TRI_REC_EARLY_Z_OFF");
    fs.uses_center_w = h.b("MESA71_TRI_REC_FS_USES_REAL_CENTRE_W");
    fs.lock_scoreboard_on_first_thrsw = h.b("MESA71_TRI_REC_SCOREBOARD_WAIT_FIRST_THRSW");
    fs.uses_implicit_point_line_varyings = !h.b("MESA71_TRI_REC_DISABLE_IMPLICIT_PL_VARYINGS");
    fs.uses_pid = h.b("MESA71_TRI_REC_INSERT_PRIM_ID");
    let vprog = |code: u32, s: &str| {
        // SAFETY: as above
        let mut v: ffi71::V3dVsProg = unsafe { std::mem::zeroed() };
        v.code = code;
        v.threads = threads(&format!("MESA71_TRI_{s}_FOUR_THREADS"));
        v.single_seg = h.b(&format!("MESA71_TRI_{s}_FINAL_THREAD_SECTION"));
        v.vpm_input_size = h.d(&format!("MESA71_TRI_REC_{s}_IN_VPM"));
        v.vpm_output_size = h.d(&format!("MESA71_TRI_REC_{s}_OUT_VPM"));
        v.vcm_cache_size = h.d("MESA71_TRI_VCM_CACHE_BATCHES");
        v
    };
    let mut vs = vprog(vs_code, "VS");
    let mut cs = vprog(cs_code, "CS");
    // struct mesa71_tri_attr { vec_size, type, cs_values, vs_values, stride, max_index, divisor }
    let a = &h.arrays["mesa71_tri_attr"];
    let n = h.d("MESA71_TRI_ATTR_COUNT") as usize;
    assert_eq!(a.len(), 7 * n);
    let mut attrs = vec![];
    for i in 0..n {
        let r = &a[7 * i..7 * i + 7];
        assert_eq!(r[1], 2, "attribute {i} is float");
        cs.vattr_sizes[i] = r[2] as u8;
        vs.vattr_sizes[i] = r[3] as u8;
        attrs.push(ffi71::V3dVertexAttr {
            // gl_ref.py's buffer: positions at 0, colours at 0x30
            address: p.vertex + if i == 0 { 0 } else { 0x30 },
            stride: r[4] as u32,
            components: r[0] as u32,
            divisor: r[6] as u32,
        });
    }
    TriProgram {
        fs,
        vs,
        cs,
        vs_u: uniforms(&h, "vs"),
        cs_u: uniforms(&h, "cs"),
        attrs,
        flags: h.d("MESA71_TRI_REC_FLAGS"),
    }
}

/// GL's initial state for one triangle into a 64x64 FBO, as fill71 maps it
/// (tools/v3d/job71_check.c tri_draw).
fn tri_state() -> ffi71::V3dJob71State {
    ffi71::V3dJob71State {
        vp: ffi71::V3dViewport { scale: [32.0, 32.0, 0.5], translate: [32.0, 32.0, 0.5] },
        front_ccw: false,
        line_width: 1.0,
        colormask: 0xf,
        depth_func: ffi71::V3D_COMPARE_FUNC_LESS,
        sample_mask: 0xf,
        ..Default::default()
    }
}

/// The dump's control lists against ours: binning, render and generic tile
/// list, packet by packet.
fn check_lists(what: &str, h: &Harness, built: &backend71::Built71, d: &Dump, p: &Place) -> [usize; 3] {
    let name = |a: u32| format!("CL_{a:#x}");
    let ind = h.bytes(List71::Ind);
    let (ls, le) = built.tile_list;
    let tile = &ind[(ls - p.ind) as usize..(le - p.ind) as usize];
    [
        assert_list(&format!("{what} binning list"), h.bytes(List71::Bcl), &d.lists[&name(p.bcl)]),
        assert_list(&format!("{what} render list"), h.bytes(List71::Rcl), &d.lists[&name(p.rcl)]),
        assert_list(&format!("{what} generic tile list"), tile, &d.lists[&name(p.ind)]),
    ]
}

#[test]
fn clear_job_is_mesas() {
    let p = &CLEAR_PLACE;
    let t = table();
    let colors = [color(true)];
    let frame = backend71::Frame71 {
        gl: gl_frame(64, 64, &colors),
        target: rgba8_target(64, 64, 1, p),
        clear: Some(color_clear(64, 64, 1)),
        zs: backend71::ZsKeep71::NONE,
    };
    let mut h = harness_at(p);
    let a = backend71::assemble(&t, &frame, &[], &mut h).unwrap_or_else(|e| panic!("clear: {e}"));
    // the plan: the tile-buffer clear of RT 0 to blue (0xffff0000 in the
    // tile buffer, clear.clif's Clear Color low bits), a 64x64 tile
    assert!(matches!(a.frame.clear_plan, Some(clearplan71::ClearPlan::TileClear { targets: 1, .. })));
    assert_eq!(a.frame.clear.color[0][0], 0xffff_0000);
    assert_eq!(a.frame.admitted.tile, (64, 64));
    let d = parse_dump("clear");
    let n = check_lists("clear", &h, &a.built, &d, p);
    // job71.sh (a): 19 / 96 / 27 bytes
    assert_eq!((h.bytes(List71::Bcl).len(), h.bytes(List71::Rcl).len()), (19, 96));
    assert_eq!(a.built.tile_list.1 - a.built.tile_list.0, 27);
    // packets: binning 6, render 19, tile list 9 (job71_check's census)
    assert_eq!(n, [6, 19, 9]);
    assert!(d.records.is_empty() && d.attrs.is_empty());
}

#[test]
fn triangle_job_is_mesas() {
    let p = &TRI_PLACE;
    let t = table();
    let prog = tri_program(p);
    let colors = [color(true)];
    let frame = backend71::Frame71 {
        gl: gl_frame(64, 64, &colors),
        target: rgba8_target(64, 64, 1, p),
        clear: Some(color_clear(64, 64, 1)),
        zs: backend71::ZsKeep71::NONE,
    };
    let draw = backend71::Draw71 {
        st: tri_state(),
        fs: prog.fs,
        vs: prog.vs,
        cs: prog.cs,
        fs_uniforms: &[],
        vs_uniforms: &prog.vs_u,
        cs_uniforms: &prog.cs_u,
        fs_gl: &[],
        vs_gl: &[],
        attrs: &prog.attrs,
        prim: ffi71::V3D_PRIM_TRIANGLES,
        count: 3,
        first: 0,
    };
    let mut h = harness_at(p);
    let a = backend71::assemble(&t, &frame, &[draw], &mut h).unwrap_or_else(|e| panic!("triangle: {e}"));
    let d = parse_dump("triangle");
    check_lists("triangle", &h, &a.built, &d, p);
    assert_eq!(h.bytes(List71::Bcl).len(), 126);

    // the board's indirect layout: uniforms +0x00 (FS, none, and VS), CS
    // +0x40, record +0x60, attribute records +0x80
    let lo = a.built.layouts[0];
    assert_eq!(
        (lo.fs_uniforms, lo.vs_uniforms, lo.cs_uniforms, lo.record, lo.attr_records),
        (p.ind, p.ind, p.ind + 0x40, p.ind + 0x60, p.ind + 0x80)
    );

    // the uniform streams uniform71 filled are the dump's words
    let ind = h.bytes(List71::Ind);
    let buf = format!("CL_{:#x}", p.ind);
    let dump_words: Vec<u32> = (0..0x60).step_by(4).map(|o| d.words[&(buf.clone(), o)]).collect();
    let mut ours = a.draws[0].streams[1].clone();
    ours.resize(0x40 / 4, 0);
    ours.extend(&a.draws[0].streams[2]);
    assert_eq!(ours, dump_words, "the VS and CS uniform streams");
    let ind_words: Vec<u32> = ind[..0x60].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(ind_words, dump_words, "the indirect buffer's uniform words");

    // the D0 record: the flag bits as the board's, every other field as the dump
    let rec = &ind[0x60..0x80];
    assert_eq!(u32::from_le_bytes(rec[..4].try_into().unwrap()) & 0x00ff_ffff, prog.flags);
    assert_eq!(d.records.len(), 1);
    let first_plain = record71::FIELDS.iter().position(|f| f.name == "Number of varyings in Fragment Shader").unwrap();
    let mut ours = Pk { name: "SHADREC".into(), fields: vec![] };
    for f in &record71::FIELDS[first_plain..] {
        let v = bits(rec, f.start, f.size);
        let text = match f.kind {
            record71::Kind::Address if f.size == 29 => hex(v << 3),
            record71::Kind::Address => hex(v),
            record71::Kind::UintMinusOne => (v + 1).to_string(),
            _ => v.to_string(),
        };
        ours.fields.push((f.name.to_string(), text));
    }
    let mut want = d.records[0].clone();
    want.fields.retain(|(l, _)| record71::FIELDS[first_plain..].iter().any(|f| f.name == l));
    assert!(want.fields.len() >= 20, "the record's non-flag fields: {}", want.fields.len());
    assert_same("triangle record", &ours, &want);

    // the attribute records
    assert_eq!(d.attrs.len(), a.draws[0].attrs.count);
    for (i, w) in d.attrs.iter().enumerate() {
        let b = &ind[0x80 + 16 * i..0x90 + 16 * i];
        let mut g = Pk { name: "ATTR".into(), fields: vec![] };
        for (name, start, size) in attr71::FIELDS {
            let mut v = bits(b, start, size);
            let text = if name == "Address" {
                hex(v)
            } else {
                if name == "Vec size" && v == 0 {
                    v = 4;
                }
                v.to_string()
            };
            g.fields.push((name.to_string(), text));
        }
        assert_same(&format!("triangle attr {i}"), &g, w);
    }
}

// ---- refusals, by name ---------------------------------------------------------

#[test]
fn identity_refusals_are_named() {
    let ok = table();
    assert_eq!((ok.rev.ver, ok.rev.rev), (71, 10));
    assert!(record71::has_draw_index_record(ok.rev));
    // the Pi 4's HUB_IDENT1 (TVER 4, REV 2: driver/docs/rpi4-v3d.md)
    let e = backend71::init(0x000e_1124, 0x0000_0000).unwrap_err();
    assert_eq!(e.to_string(), "ident71: V3D 4.2: build gpu-v3d");
    // a 7.1 part with IPREV 5 (and every one below 10)
    let e = backend71::init(0x0008_1117, 0x0002_0510).unwrap_err();
    assert_eq!(e.to_string(), "ident71: V3D 7.1.5: this table is 7.1.10 (BCM2712 D0) only");
    for iprev in 0..10 {
        let e = backend71::init(0x0008_1117, 0x0002_0010 | iprev << 8).unwrap_err();
        assert_eq!(e.module, "ident71");
        assert_eq!(e.reason, format!("V3D 7.1.{iprev}: this table is 7.1.10 (BCM2712 D0) only"));
    }
    for iprev in [10, 11, 16] {
        assert!(backend71::init(0x0008_1117, 0x0002_0010 | iprev << 8).is_ok());
    }
}

fn frame_refusal(w: u32, h: u32, n: usize) -> Result<backend71::FramePlan71, backend71::Refusal> {
    let colors = vec![color(true); n];
    backend71::plan_frame(&backend71::Frame71 {
        gl: gl_frame(w, h, &colors),
        target: rgba8_target(w, h, n as u32, &CLEAR_PLACE),
        clear: Some(color_clear(w, h, n.min(8))),
        zs: backend71::ZsKeep71::NONE,
    })
}

#[test]
fn limit_refusals_are_named() {
    // nine targets
    let e = frame_refusal(64, 64, 9).unwrap_err();
    assert_eq!(e.to_string(), "rt71: v3d 7.1: more than 8 render targets");
    // a side over 7680 px, either one
    for (w, h) in [(7681, 64), (64, 7681)] {
        let e = frame_refusal(w, h, 1).unwrap_err();
        assert_eq!(e.to_string(), "rt71: v3d 7.1: a render target larger than 7680 px");
    }
    // eight targets and 7680 px are admitted
    let p = frame_refusal(64, 64, 8).expect("8 targets");
    assert_eq!(p.cleared_rts, 0xff);
    frame_refusal(7680, 7680, 1).expect("7680 px");
    frame_refusal(7680, 64, 8).expect("8 targets, 7680 px");

    // assemble refuses before the builder runs: the builder is never called
    struct Never;
    impl Builder71 for Never {
        fn gpu(&self, _: List71) -> u32 {
            0x1000
        }
        fn build(
            &mut self,
            _: &ffi71::V3dJob71Target,
            _: &[backend71::Draw71<'_>],
            _: &ffi71::V3dJob71Clear,
        ) -> Result<backend71::Built71, backend71::BuildError71> {
            panic!("the builder was called for a refused frame")
        }
        fn bytes(&self, _: List71) -> &[u8] {
            &[]
        }
    }
    let colors = vec![color(true); 9];
    let f = backend71::Frame71 {
        gl: gl_frame(64, 64, &colors),
        target: rgba8_target(64, 64, 9, &CLEAR_PLACE),
        clear: None,
        zs: backend71::ZsKeep71::NONE,
    };
    let e = backend71::assemble(&table(), &f, &[], &mut Never).unwrap_err();
    assert_eq!(e.to_string(), "rt71: v3d 7.1: more than 8 render targets");
}

#[test]
fn the_c_builder_refuses_the_same_limits() {
    // the builder's own guard (v3d_job71.c target_ok) agrees with rt71's:
    // 9 targets and 7681 px are refused at v3d_job71_begin, 8 and 7680 pass
    let c = ffi71::V3dJob71Clear::default();
    for (w, h, n, ok) in
        [(64, 64, 9, false), (7681, 64, 1, false), (64, 7681, 1, false), (64, 64, 8, true), (7680, 7680, 1, true)]
    {
        let mut hn = harness_at(&CLEAR_PLACE);
        let r = hn.build(&rgba8_target(w, h, n, &CLEAR_PLACE), &[], &c);
        assert_eq!(r.is_ok(), ok, "{w}x{h}, {n} targets: {r:?}");
        if !ok {
            assert_eq!(r.unwrap_err(), backend71::BuildError71::Begin);
        }
    }
}

#[test]
fn draw_refusals_are_named() {
    let t = table();
    let prog = tri_program(&TRI_PLACE);
    let mut vs_u = prog.vs_u.clone();
    let base = backend71::Draw71 {
        st: tri_state(),
        fs: prog.fs,
        vs: prog.vs,
        cs: prog.cs,
        fs_uniforms: &[],
        vs_uniforms: &prog.vs_u,
        cs_uniforms: &prog.cs_u,
        fs_gl: &[],
        vs_gl: &[],
        attrs: &prog.attrs,
        prim: ffi71::V3D_PRIM_TRIANGLES,
        count: 3,
        first: 0,
    };
    backend71::plan_draw(&t, &base, TRI_PLACE.ind).expect("Mesa's triangle");
    // a TMU configuration uniform: the 7.1 texture state is not written here
    vs_u[15].contents = uniform71::TMU_CONFIG_P0;
    let d = backend71::Draw71 { vs_uniforms: &vs_u, ..base };
    let e = backend71::plan_draw(&t, &d, TRI_PLACE.ind).unwrap_err();
    assert_eq!(e.to_string(), "uniform71: TMU config uniform (contents 40): the 7.1 texture state is not written yet");
    // a code address that is not 8-byte aligned (record71)
    let mut fs = prog.fs;
    fs.code += 4;
    let e = backend71::plan_draw(&t, &backend71::Draw71 { fs, ..base }, TRI_PLACE.ind).unwrap_err();
    assert_eq!(e.to_string(), "record71: the fragment shader's code address is not 8-byte aligned");
    // a masked clear is a clear draw: not this table's yet
    let colors = [color(true)];
    let mut ci = color_clear(64, 64, 1);
    ci.targets[0].mask = 0x9;
    let e = backend71::plan_frame(&backend71::Frame71 {
        gl: gl_frame(64, 64, &colors),
        target: rgba8_target(64, 64, 1, &CLEAR_PLACE),
        clear: Some(ci),
        zs: backend71::ZsKeep71::NONE,
    })
    .unwrap_err();
    assert_eq!(e.module, "backend71");
    assert!(e.reason.contains("clear draw"), "{e}");
}
