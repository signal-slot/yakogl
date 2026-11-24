// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/backend71.rs` (the V3D 7.1 job assembly) on the jobs of gl_test
//! cases 5, 66 and 102 against Mesa's 7.1 references for their shapes,
//! tools/v3d/pi5/gl_scissor.clif, gl_viewport.clif and gl_colormask.clif
//! (tools/v3d/pi5/README.md "The gl_test shapes"; the case-to-job design is
//! docs/notes/v3d71-table71.md and docs/notes/v3d71-gl-table.md section 5).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_backend71_cases
//! ```
//!
//! The C builder (driver/src/v3d/v3d_job71.c over v3d_frame.c at
//! `-DV3D_CL_VER=71`) runs in a child process, tests/v3d71_backend71_harness.c
//! unchanged, built and driven as tests/v3d71_backend71.rs does.
//!
//! The shapes (gl_test's GL calls, one job per glFinish):
//!
//! * case 5 `case_scissor`, `gl_scissor`: a full black clear; then, scissored
//!   to (0,0,32,32), a clear (a clear quad) and the yellow full-screen quad;
//! * case 66 `case_clearvp`, `gl_viewport`: a white clear at the full
//!   viewport, a blue clear at a quarter viewport (glClear is not bounded by
//!   the viewport: the whole target, a tile-buffer clear like the first), a
//!   green clear scissored to the bottom-left quarter (a clear quad);
//! * case 102 `case_maskclear`, `gl_colormask`: black, an R+A masked clear;
//!   black, a G-only clear scissored to the bottom-left 32x32; black, a
//!   B-only clear and a yellow quad with every channel on.
//!
//! Every job takes two routes:
//!
//! 1. The GL table's route: `fill71::fill` (the GL state model of
//!    tests/v3d71_backend71_shapes.rs, trimmed to colour clears and draws)
//!    then `backend71::assemble_filled`. Our programs are the data of
//!    tools/v3d/gltest-shaders71/ours_color_uv.h (gl_test's pair) and
//!    ours_clear.h (the clear quad), where the reference has Mesa's gl_test
//!    pair and blitter. Each job's buffers sit at the reference job's own
//!    addresses (its binning, render and indirect lists, tile allocation,
//!    render target and vertex buffer). The render and generic tile lists, and
//!    the binning list of a job without a draw, must have the reference's
//!    packet sequence exactly (no packet more, less or elsewhere); a binning
//!    list with draws must have the reference's prologue and epilogue exactly
//!    and, at each draw, the same draw packet and the same state packets in
//!    effect (tools/v3d/check71.d gl_cmp.py's `bcl_split`), its raw sequence's
//!    difference pinned as one entry. Every value the reference prints must be
//!    ours: addresses included in a job without a draw; in a job with a draw
//!    the address fields are not compared (the indirect buffer holds our
//!    programs' uniform streams and records at our layout, Mesa's its own). The D0
//!    shader records' plain fields and the attribute records are compared the
//!    same way (addresses left out), as tests/v3d71_backend71_shapes.rs does.
//!    The differences that remain are each pinned in [`PINNED`] with its
//!    reason; a difference not pinned fails, and so does a pin that no longer
//!    occurs.
//! 2. The direct route, `backend71::assemble` with a `Frame71` (the GL frame
//!    facts and a `clearplan71::ClearIn`, the route of
//!    tests/v3d71_backend71.rs): a whole clear writes the same bytes as route
//!    1; a scissored or masked clear is backend71's named refusal (a clear
//!    draw belongs to route 1), pinned by its text.
//!
//! The tile allocation size (the submit's `ADD_BIN` qms) is not a packet: it
//! is the reference's rounded up to the Pi 5 kernel's 16 KiB page
//! (`backend71::PAGE_SIZE`, tools/v3d/pi5/README.md "Differences" item 2).
#![allow(dead_code)]

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(clippy::if_same_then_else)]
mod attr71;
#[path = "../src/v3d/backend71.rs"]
mod backend71;
#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;
#[path = "../src/v3d/draw71.rs"]
mod draw71;
#[path = "../src/v3d/ffi71.rs"]
mod ffi71;
#[path = "../src/v3d/fill71.rs"]
mod fill71;
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/ident71.rs"]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
mod limits71;
#[path = "../src/v3d/prim71.rs"]
mod prim71;
#[path = "../src/v3d/record71.rs"]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(clippy::eq_op)]
mod rt71;
#[path = "../src/v3d/uniform71.rs"]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use backend71::{Builder71, List71};
use fill71::{ClearQuad, ClearReq, ColorBuffer, DrawIn, DsBuffer, FilledJob, Framebuffer, History, Op, Program};

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

fn hex(v: u64) -> String {
    format!("0x{v:08x}")
}

const SIZE: u32 = 64;
/// Our programs' code (not in any packet; record addresses are not compared).
const CODE: u32 = 0x200000;
/// The window's depth/stencil renderbuffer (GL_DEPTH24_STENCIL8, kept): no
/// job of the three shapes loads or stores it.
const ZS: u32 = 0x2000000;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLES: u32 = 4;
const GL_RGBA8: u32 = 0x8058;

fn table() -> backend71::Table71 {
    // HUB_IDENT1 and HUB_IDENT3 of the board (tools/v3d/pi5/README.md)
    backend71::init(0x0008_1117, 0x0002_0a10).expect("the Pi 5's identity")
}

// ---- our programs (tools/v3d/gltest-shaders71/ours_*.h, read as data) --------

const OURS_COLOR_UV: &str = include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h");
const OURS_CLEAR: &str = include_str!("../tools/v3d/gltest-shaders71/ours_clear.h");

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

/// One of our programs as fill71 takes it (tests/v3d71_backend71_shapes.rs's
/// reading of the same headers), the uniform lists leaked to `'static`.
fn our_program(text: &str, p: &str, code: u32) -> Program<'static> {
    let h = c_header(text);
    let lc = p.to_ascii_lowercase();
    let up = p.to_ascii_uppercase();
    let list = |s: &str| -> &'static [uniform71::Word] {
        let n = h.d(&format!("{up}_{}_NUM_UNIFORMS", s.to_ascii_uppercase()));
        let words: Vec<uniform71::Word> = match h.lists.get(&format!("{lc}_{s}_uniforms")) {
            Some(v) => v.chunks(2).map(|c| uniform71::Word { contents: c[0] as u32, data: c[1] as u32 }).collect(),
            None => vec![],
        };
        assert_eq!(words.len(), n as usize, "{p} {s}: the uniform list");
        Box::leak(words.into_boxed_slice())
    };
    let reads = |s: &str| -> [u8; 16] {
        let v = &h.lists[&format!("{up}_{s}_VATTR_SIZES")];
        core::array::from_fn(|i| v[i] as u8)
    };
    let threads = |s: &str| h.d(&format!("{up}_{s}_THREADS"));
    let vdesc = |s: &str, at: u32| record71::VertexDesc {
        prog: record71::ProgramDesc {
            code: at,
            uniforms: 0,
            threads: threads(s),
            single_seg: h.b(&format!("{up}_{s}_SINGLE_SEG")),
            propagate_nans: false,
        },
        vpm_input_size: h.d(&format!("{up}_{s}_VPM_INPUT_SIZE")),
        vpm_output_size: h.d(&format!("{up}_{s}_VPM_OUTPUT_SIZE")),
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: h.b(&format!("{up}_{s}_USES_VID")),
        uses_iid: h.b(&format!("{up}_{s}_USES_IID")),
        uses_base_instance: h.b(&format!("{up}_{s}_USES_BIID")),
        uses_base_vertex: false,
        uses_draw_index: false,
    };
    let mut desc = record71::DrawDesc::default();
    desc.fs.prog = record71::ProgramDesc {
        code,
        uniforms: 0,
        threads: threads("FS"),
        single_seg: h.b(&format!("{up}_FS_SINGLE_SEG")),
        propagate_nans: false,
    };
    desc.fs.writes_z = h.b(&format!("{up}_FS_WRITES_Z"));
    desc.fs.disable_ez = h.b(&format!("{up}_FS_DISABLE_EZ"));
    desc.fs.num_varyings = h.d(&format!("{up}_FS_NUM_INPUTS"));
    desc.fs.uses_center_w = h.b(&format!("{up}_FS_USES_CENTER_W"));
    desc.fs.uses_pid = h.b(&format!("{up}_FS_USES_PID"));
    desc.fs.lock_scoreboard_on_first_thrsw = h.b(&format!("{up}_FS_LOCK_SCOREBOARD_ON_FIRST_THRSW"));
    desc.fs.uses_implicit_point_line_varyings = h.b(&format!("{up}_FS_USES_IMPLICIT_POINT_LINE_VARYINGS"));
    desc.vs = vdesc("VS", code + 0x100);
    desc.cs = vdesc("CS", code + 0x200);
    desc.point_size = h.b(&format!("{up}_VS_WRITES_PSIZ"));
    Program {
        desc,
        cs_reads: reads("CS"),
        vs_reads: reads("VS"),
        cs_vcm_cache_size: h.d(&format!("{up}_CS_VCM_CACHE_SIZE")),
        vs_vcm_cache_size: h.d(&format!("{up}_VS_VCM_CACHE_SIZE")),
        separate_segments: h.b(&format!("{up}_VS_SEPARATE_SEGMENTS")) || h.b(&format!("{up}_CS_SEPARATE_SEGMENTS")),
        varyings: fill71::VaryingFlags {
            flat: [h.d(&format!("{up}_FS_FLAT_SHADE_FLAGS")), 0],
            noperspective: [h.d(&format!("{up}_FS_NOPERSPECTIVE_FLAGS")), 0],
            centroid: [h.d(&format!("{up}_FS_CENTROID_FLAGS")), 0],
        },
        discards: h.b(&format!("{up}_FS_USES_DISCARD")),
        fs_uniforms: list("fs"),
        vs_uniforms: list("vs"),
        cs_uniforms: list("cs"),
    }
}

/// (gl_test's program, the clear quad's): ours_color_uv and ours_clear.
fn programs() -> &'static (Program<'static>, Program<'static>) {
    static P: OnceLock<(Program<'static>, Program<'static>)> = OnceLock::new();
    P.get_or_init(|| {
        (our_program(OURS_COLOR_UV, "ours71_color_uv", CODE), our_program(OURS_CLEAR, "ours71_clear", CODE + 0x300))
    })
}

const fn float_attr(components: u32, stride: u64, addr: u32) -> attr71::Attr {
    attr71::Attr {
        enabled: true,
        components,
        gl_type: GL_FLOAT,
        normalized: false,
        integer: false,
        stride,
        offset: 0,
        buffer_addr: addr,
        divisor: 0,
        generic: [0, 0, 0, 0x3f80_0000],
        constant_addr: None,
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
        // One directory per process: the gate runs this binary in several
        // rows at once, all with the same CARGO_TARGET_TMPDIR.
        let dir =
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("v3d71_backend71_cases-{}", std::process::id()));
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

/// `Builder71` over the harness: the lists at the reference job's addresses.
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

fn xattr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
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

fn ver_ok(t: &str) -> bool {
    xattr(t, "max_ver").is_none_or(|v| num(v) >= 71) && xattr(t, "min_ver").is_none_or(|v| num(v) <= 71)
}

fn xml_packets() -> Vec<XPacket> {
    let text = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let mut out = vec![];
    let mut cur: Option<XPacket> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("<packet") {
            let ok = ver_ok(t);
            let p = XPacket {
                code: num(xattr(t, "code").unwrap()) as u8,
                clif: clif_name(xattr(t, "name").unwrap(), xattr(t, "shortname")),
                fields: vec![],
            };
            if t.ends_with("/>") {
                if ok {
                    out.push(p);
                }
            } else {
                cur = ok.then_some(p);
            }
        } else if t.starts_with("</packet>") {
            if let Some(p) = cur.take() {
                out.push(p);
            }
        } else if t.starts_with("<field") && ver_ok(t) {
            if let Some(p) = cur.as_mut() {
                let s = xattr(t, "start").unwrap();
                let start = match s.strip_suffix('b') {
                    Some(b) => num(b) as u32 * 8,
                    None => num(s) as u32,
                };
                p.fields.push(XField {
                    name: xattr(t, "name").unwrap().to_string(),
                    start,
                    size: num(xattr(t, "size").unwrap()) as u32,
                    ty: xattr(t, "type").unwrap_or("uint").to_string(),
                    minus_one: xattr(t, "minus_one") == Some("true"),
                    default: xattr(t, "default").map(num),
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

/// One packet (or record) as the dump prints it: name and `(label, value)`;
/// `addrs`: the labels that are addresses (ours only).
#[derive(Clone, Debug, PartialEq)]
struct Pk {
    name: String,
    fields: Vec<(String, String)>,
    addrs: Vec<String>,
}

impl Pk {
    fn new(name: &str) -> Pk {
        Pk { name: name.to_string(), fields: vec![], addrs: vec![] }
    }
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
    let mut out = Pk::new(&p.clif);
    for f in &p.fields {
        let v = bits(body, f.start, f.size);
        let sext = |v: u64| (((v << (64 - f.size)) as i64) >> (64 - f.size)).to_string();
        let text = match f.ty.as_str() {
            "address" => {
                out.addrs.push(f.name.clone());
                let base = f.start + f.size - 32;
                hex(bits(body, base, 32) & !((1u64 << (32 - f.size)) - 1))
            }
            "float" => format!("{:.6}", f32::from_bits(v as u32)),
            "f187" => format!("{:.6}", f32::from_bits((v as u32) << 16)),
            "int" | "s8.8" => sext(v),
            _ => (v + f.minus_one as u64).to_string(),
        };
        out.fields.push((f.name.clone(), text));
    }
    out
}

// ---- the references, job by job ----------------------------------------------

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

/// One job of a reference.
#[derive(Default)]
struct RefJob {
    /// buffer name -> its control-list packets in file order
    lists: BTreeMap<String, Vec<Pk>>,
    /// the shader records and, per record, its attribute records
    records: Vec<Pk>,
    attrs: Vec<Vec<Pk>>,
    /// `@add_bin` and `@add_render` arguments, addresses as numbers
    bin: Vec<u64>,
    render: Vec<u64>,
}

fn buffer_base(name: &str) -> u64 {
    num(name.rsplit('_').next().unwrap())
}

impl RefJob {
    /// The binning and render list buffers (the submit's first arguments).
    fn list_named(&self, at: u64) -> &[Pk] {
        let (_, v) =
            self.lists.iter().find(|(n, _)| buffer_base(n) == at).unwrap_or_else(|| panic!("no list at {at:#x}"));
        v
    }
    fn bcl(&self) -> &[Pk] {
        self.list_named(self.bin[0])
    }
    fn rcl(&self) -> &[Pk] {
        self.list_named(self.render[0])
    }
    fn field(p: &Pk, k: &str) -> String {
        p.fields.iter().find(|f| f.0 == k).unwrap_or_else(|| panic!("{}: no {k}", p.name)).1.clone()
    }
    /// The generic tile list's `[start, end)`.
    fn tile_range(&self) -> (u64, u64) {
        let g = self.rcl().iter().find(|p| p.name == "GENERIC_TILE_LIST").unwrap();
        (num(&Self::field(g, "start")), num(&Self::field(g, "end")))
    }
    /// The indirect buffer: the control-list buffer holding the tile list.
    fn ind(&self) -> (u64, &[Pk]) {
        let (s, _) = self.tile_range();
        let (n, v) = self
            .lists
            .iter()
            .filter(|(n, _)| buffer_base(n) <= s && buffer_base(n) != self.bin[0] && buffer_base(n) != self.render[0])
            .max_by_key(|(n, _)| buffer_base(n))
            .unwrap();
        (buffer_base(n), v)
    }
}

fn ref_jobs(shape: &str) -> Vec<RefJob> {
    let text = read(&format!("tools/v3d/pi5/gl_{shape}.clif"));
    let mut jobs = vec![];
    let mut d = RefJob::default();
    let (mut buf, mut fmt) = (String::new(), String::new());
    let mut started = false;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MESA: info:") else { continue };
        let t = rest.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("@createbuf") {
            started = true;
        }
        if !started {
            continue;
        }
        if t.starts_with("@wait_render") {
            jobs.push(std::mem::take(&mut d));
            started = false;
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
                "shadrec_gl_main" => {
                    d.records.push(Pk::new("SHADREC"));
                    d.attrs.push(vec![]);
                }
                "shadrec_gl_attr" => d.attrs.last_mut().unwrap().push(Pk::new("ATTR")),
                "ctrllist" => {
                    d.lists.entry(buf.clone()).or_default();
                }
                _ => {}
            }
            continue;
        }
        if let Some(k) = t.strip_prefix('@') {
            fmt = k.split_whitespace().next().unwrap_or("").to_string();
            continue;
        }
        if fmt == "add_bin" || fmt == "add_render" {
            let v = if t.starts_with('[') {
                num(t.split("/*").nth(1).unwrap().split("*/").next().unwrap())
            } else {
                num(t)
            };
            if fmt == "add_bin" { &mut d.bin } else { &mut d.render }.push(v);
            continue;
        }
        if let Some(body) = t.strip_prefix("/*") {
            let Some(at) = body.find(": */") else { continue };
            let field = (body[..at].trim().to_string(), dump_value(body[at + 4..].trim()));
            let target = match fmt.as_str() {
                "ctrllist" => d.lists.get_mut(&buf).and_then(|v| v.last_mut()),
                "shadrec_gl_main" => d.records.last_mut(),
                "shadrec_gl_attr" => d.attrs.last_mut().and_then(|g| g.last_mut()),
                _ => None,
            };
            if let Some(p) = target {
                p.fields.push(field);
            }
        } else if fmt == "ctrllist" && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            d.lists.get_mut(&buf).unwrap().push(Pk::new(t));
        }
    }
    jobs
}

const DRAWS: [&str; 4] =
    ["VERTEX_ARRAY_PRIMS", "VERTEX_ARRAY_INSTANCED_PRIMS", "INDEXED_PRIM_LIST", "INDEXED_INSTANCED_PRIM_LIST"];

/// Where a job's buffers sit: the reference job's own addresses.
#[derive(Clone, Copy, Debug)]
struct Place {
    bcl: u32,
    rcl: u32,
    ind: u32,
    tile_alloc: u32,
    rt: u32,
    /// gl_test's vertex buffer (its draw's attribute 0), or 0.
    vertex: u32,
    /// the clear quad's attribute 0 (Mesa's blitter vertex), or 0.
    quad: u32,
}

fn place(r: &RefJob) -> Place {
    let (ind, tiles) = r.ind();
    let store = tiles
        .iter()
        .find(|p| p.name == "STORE" && RefJob::field(p, "Buffer to Store") == "0")
        .expect("the tile list stores render target 0");
    let attr0 = |n: usize| {
        r.attrs.iter().find(|g| g.len() == n).map(|g| num(&RefJob::field(&g[0], "Address")) as u32).unwrap_or(0)
    };
    Place {
        bcl: r.bin[0] as u32,
        rcl: r.render[0] as u32,
        ind: ind as u32,
        tile_alloc: r.bin[2] as u32,
        rt: num(&RefJob::field(store, "Address")) as u32,
        vertex: attr0(3),
        quad: attr0(1),
    }
}

// ---- the GL state model (tests/v3d71_backend71_shapes.rs's, colour only) ------

/// One glFinish: the GL facts of its clear, fill71's jobs and the frame.
struct Flush {
    /// The clear that opens the job (all three shapes clear first).
    clear: ClearReq,
    jobs: Vec<FilledJob>,
    colors: Vec<ColorBuffer>,
    ds: Option<DsBuffer>,
    hist: History,
    place: Place,
}

/// gl_test's window: one GL_RGBA8 colour buffer and a kept
/// GL_DEPTH24_STENCIL8 renderbuffer (tools/v3d/pi5/README.md).
struct Gl {
    st: draw71::DrawState,
    clear_color: [f32; 4],
    color_mask: u8,
    scissor: Option<[i32; 4]>,
    ops: Vec<Op<'static>>,
    hist: History,
    places: Vec<Place>,
    flushes: Vec<Flush>,
}

impl Gl {
    fn new(places: Vec<Place>) -> Gl {
        let mut st = draw71::DrawState::gl_default(SIZE, SIZE, false, 1);
        st.viewport = [0.0, 0.0, SIZE as f32, SIZE as f32];
        Gl {
            st,
            clear_color: [0.0; 4],
            color_mask: 0xf,
            scissor: None,
            ops: vec![],
            hist: History::NEW,
            places,
            flushes: vec![],
        }
    }

    fn mask(&mut self, m: u8) {
        self.color_mask = m;
        for t in &mut self.st.targets {
            t.color_mask = m;
        }
    }

    fn req(&self) -> ClearReq {
        ClearReq {
            color: Some(self.clear_color),
            draw_buffers: 1,
            color_mask: self.color_mask,
            depth: None,
            depth_mask: self.st.depth_write,
            stencil: None,
            stencil_mask: 0xff,
            scissor: self.scissor,
        }
    }

    /// glClear(GL_COLOR_BUFFER_BIT).
    fn clear(&mut self) {
        let r = self.req();
        self.ops.push(Op::Clear(r));
    }

    /// glDrawArrays(GL_TRIANGLES, 0, 6) with gl_test's program and its
    /// 40-byte vertex (pos4 +0, colour4 +16, uv2 +32) at this job's address.
    fn draw(&mut self) {
        let v = self.places[self.flushes.len()].vertex;
        let attrs: &'static [attr71::Attr] =
            Box::leak(Box::new([float_attr(4, 40, v), float_attr(4, 40, v + 16), float_attr(2, 40, v + 32)]));
        let mut st = self.st;
        st.scissor = self.scissor;
        self.ops.push(Op::Draw(Box::new(DrawIn {
            state: st,
            line_width: 1.0,
            blend_color: [0.0; 4],
            program: &programs().0,
            attrs,
            dummy_addr: v,
            prim: prim71::Draw::arrays(GL_TRIANGLES, 0, 6),
            vs_words: &[],
            fs_words: &[],
            oq_addr: 0,
        })));
    }

    /// glFinish: fill the flush at the next reference job's addresses.
    fn finish(&mut self) {
        let ops = std::mem::take(&mut self.ops);
        let p = self.places[self.flushes.len()];
        let colors = vec![ColorBuffer {
            gl_format: GL_RGBA8,
            address: p.rt,
            memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub_or_stride: 8,
        }];
        let ds = Some(DsBuffer {
            gl_format: fill71::GL_DEPTH24_STENCIL8,
            address: ZS,
            memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub: 8,
        });
        let fb =
            Framebuffer { width: SIZE, height: SIZE, tile_alloc: p.tile_alloc, flip_y: false, colors: &colors, ds };
        let quad_at = if p.quad != 0 { p.quad } else { p.ind };
        let quad = ClearQuad { program: &programs().1, attr: float_attr(4, 16, quad_at), dummy_addr: quad_at };
        let jobs = fill71::fill(&fb, record71::BCM2712_D0, &self.hist, &quad, &ops)
            .unwrap_or_else(|e| panic!("job {}: fill71 refused: {e}", self.flushes.len()));
        let Some(Op::Clear(clear)) = ops.first() else { panic!("every flush of these shapes opens with a glClear") };
        let hist = self.hist;
        if let Some(last) = jobs.last() {
            self.hist = last.after;
        }
        self.flushes.push(Flush { clear: *clear, jobs, colors, ds, hist, place: p });
    }
}

/* ---- the shapes: gl_shapes.py's calls (tools/v3d/pi5/src/gl_shapes.py) ---- */

/// Case 5, `case_scissor`.
fn shape_scissor(g: &mut Gl) {
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear();
    g.finish();
    g.scissor = Some([0, 0, 32, 32]);
    g.clear();
    g.draw();
    g.finish();
}

/// Case 66, `case_clearvp`.
fn shape_viewport(g: &mut Gl) {
    g.clear_color = [1.0, 1.0, 1.0, 1.0];
    g.clear();
    g.finish();
    g.st.viewport = [0.0, 0.0, 32.0, 32.0];
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear();
    g.finish();
    g.st.viewport = [0.0, 0.0, 64.0, 64.0];
    g.scissor = Some([0, 0, 32, 32]);
    g.clear_color = [0.0, 1.0, 0.0, 1.0];
    g.clear();
    g.finish();
}

/// Case 102, `case_maskclear`.
fn shape_colormask(g: &mut Gl) {
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear();
    g.finish();
    g.mask(0b1001);
    g.clear_color = [1.0, 1.0, 1.0, 0.5];
    g.clear();
    g.finish();
    g.mask(0xf);
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear();
    g.finish();
    g.scissor = Some([0, 0, 32, 32]);
    g.mask(0b0010);
    g.clear_color = [0.0, 1.0, 0.0, 0.0];
    g.clear();
    g.finish();
    g.scissor = None;
    g.mask(0xf);
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear();
    g.finish();
    g.mask(0b0100);
    g.clear_color = [0.0, 0.0, 1.0, 0.0];
    g.clear();
    g.mask(0xf);
    g.draw();
    g.finish();
}

/// (gl_test case, shape): the three cases of this test.
const CASES: [(u32, &str); 3] = [(5, "scissor"), (66, "viewport"), (102, "colormask")];

fn run(shape: &str) -> (Vec<RefJob>, Vec<Flush>) {
    let refs = ref_jobs(shape);
    let mut g = Gl::new(refs.iter().map(place).collect());
    match shape {
        "scissor" => shape_scissor(&mut g),
        "viewport" => shape_viewport(&mut g),
        "colormask" => shape_colormask(&mut g),
        _ => unreachable!(),
    }
    (refs, g.flushes)
}

// ---- route 1: fill71 and assemble_filled against the reference ---------------

/// A difference our job keeps from Mesa's: `(shape, job, difference, reason)`.
/// Differences are `<list> #<packet> <NAME>.<field>: <ours> != <Mesa's>`,
/// or `draw <n> SHADREC|ATTR <i>.<field>: ...`, or `bcl sequence: +<ours
/// only> -<Mesa's only>`. They are the four of tests/v3d71_backend71_shapes.rs's
/// `ALLOW` that reach these jobs (EZ, PSIZE, QUADATTR; PAGE is the qms
/// assertion) and RESTATE, which that test's state-in-effect comparison
/// does not print. Every job with a draw has a clear quad (draw 0); no job
/// without a draw has any.
const PINNED: [(&str, usize, &str, &str); 22] = [
    ("scissor", 1, "rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", EZ),
    ("scissor", 1, "bcl draw 0 POINT_SIZE.Point Size: 1.000000 != 0.125000", PSIZE),
    ("scissor", 1, "draw 0 ATTR 0.Vec size: 4 != 3", QUADATTR),
    ("scissor", 1, "draw 0 ATTR 0.Stride: 16 != 36", QUADATTR),
    (
        "scissor",
        1,
        "bcl sequence: +CLIPPER_XY +CLIPPER_Z +CLIPZ +VIEWPORT_OFFSET +BLEND_ENABLES +COLOR_WMASKS +OCCLUSION_QUERY_COUNTER_ENABLE +SAMPLE_STATE",
        RESTATE,
    ),
    ("viewport", 2, "rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", EZ),
    ("viewport", 2, "bcl draw 0 POINT_SIZE.Point Size: 1.000000 != 0.125000", PSIZE),
    ("viewport", 2, "draw 0 ATTR 0.Vec size: 4 != 3", QUADATTR),
    ("viewport", 2, "draw 0 ATTR 0.Stride: 16 != 36", QUADATTR),
    ("colormask", 1, "rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", EZ),
    ("colormask", 1, "bcl draw 0 POINT_SIZE.Point Size: 1.000000 != 0.125000", PSIZE),
    ("colormask", 1, "draw 0 ATTR 0.Vec size: 4 != 3", QUADATTR),
    ("colormask", 1, "draw 0 ATTR 0.Stride: 16 != 36", QUADATTR),
    ("colormask", 3, "rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", EZ),
    ("colormask", 3, "bcl draw 0 POINT_SIZE.Point Size: 1.000000 != 0.125000", PSIZE),
    ("colormask", 3, "draw 0 ATTR 0.Vec size: 4 != 3", QUADATTR),
    ("colormask", 3, "draw 0 ATTR 0.Stride: 16 != 36", QUADATTR),
    ("colormask", 5, "rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", EZ),
    ("colormask", 5, "bcl draw 0 POINT_SIZE.Point Size: 1.000000 != 0.125000", PSIZE),
    ("colormask", 5, "draw 0 ATTR 0.Vec size: 4 != 3", QUADATTR),
    ("colormask", 5, "draw 0 ATTR 0.Stride: 16 != 36", QUADATTR),
    (
        "colormask",
        5,
        "bcl sequence: +CLIP +CLIPPER_XY +CLIPPER_Z +CLIPZ +VIEWPORT_OFFSET +OCCLUSION_QUERY_COUNTER_ENABLE +SAMPLE_STATE",
        RESTATE,
    ),
];

const EZ: &str = "v3d_job71 never enables early Z (tools/v3d/check71.d/job71.sh gl.allow EZ)";
const PSIZE: &str = "the clear quad: Mesa's blitter writes point size 0.125, v3d_job71 1.0; a TRIANGLE_FAN does not read it (job71.sh gl.allow PSIZE)";
const QUADATTR: &str = "our clear quad's position is fill71's FilledDraw::quad, four floats a vertex (16 bytes); Mesa's blitter vertex has three (36 bytes)";
const RESTATE: &str = "v3d_job71 writes every draw's whole state before it, Mesa only what changed: the second draw restates packets whose values are already in effect (the state each draw sees is compared above and equal)";

/// One job through route 1, at the reference job's addresses.
struct Ours {
    bcl: Vec<u8>,
    rcl: Vec<u8>,
    tiles: Vec<u8>,
    qms: u32,
    records: Vec<Pk>,
    attrs: Vec<Vec<Pk>>,
}

fn assemble_flush(f: &Flush) -> Ours {
    assert_eq!(f.jobs.len(), 1, "one job per glFinish in these shapes");
    let job = &f.jobs[0];
    let p = f.place;
    let fb =
        Framebuffer { width: SIZE, height: SIZE, tile_alloc: p.tile_alloc, flip_y: false, colors: &f.colors, ds: f.ds };
    let mut h = Harness { gpu: [p.bcl, p.rcl, p.ind], lists: Default::default() };
    let a = backend71::assemble_filled(&table(), &fb, job, &mut h).unwrap_or_else(|e| panic!("backend71 refused: {e}"));
    let ind = h.bytes(List71::Ind);
    let at = |addr: u32, n: usize| &ind[(addr - p.ind) as usize..(addr - p.ind) as usize + n];
    let first = record71::FIELDS.iter().position(|f| f.name == "Number of varyings in Fragment Shader").unwrap();
    let mut records = vec![];
    let mut attrs = vec![];
    for (d, lo) in a.draws.iter().zip(&a.built.layouts) {
        let rec = at(lo.record, record71::RECORD_SIZE);
        let mut r = Pk::new("SHADREC");
        for f in &record71::FIELDS[first..] {
            let v = bits(rec, f.start, f.size);
            let text = match f.kind {
                record71::Kind::Address => {
                    r.addrs.push(f.name.to_string());
                    hex(v)
                }
                record71::Kind::UintMinusOne => (v + 1).to_string(),
                _ => v.to_string(),
            };
            r.fields.push((f.name.to_string(), text));
        }
        records.push(r);
        let group = (0..d.attrs.count)
            .map(|i| {
                let b = at(lo.attr_records + (i * attr71::RECORD_BYTES) as u32, attr71::RECORD_BYTES);
                let mut g = Pk::new("ATTR");
                for (name, start, size) in attr71::FIELDS {
                    let v = bits(b, start, size);
                    let text = match name {
                        "Address" => {
                            g.addrs.push(name.to_string());
                            hex(v)
                        }
                        "Vec size" if v == 0 => "4".to_string(),
                        _ => v.to_string(),
                    };
                    g.fields.push((name.to_string(), text));
                }
                g
            })
            .collect();
        attrs.push(group);
    }
    let (ls, le) = a.built.tile_list;
    Ours {
        bcl: h.bytes(List71::Bcl).to_vec(),
        rcl: h.bytes(List71::Rcl).to_vec(),
        tiles: at(ls, (le - ls) as usize).to_vec(),
        qms: a.built.tile_alloc_size,
        records,
        attrs,
    }
}

/// Every value Mesa prints against ours; addresses only when `addrs`.
fn fields(w: &str, ours: &Pk, want: &Pk, addrs: bool, diffs: &mut Vec<String>) {
    for (label, value) in &want.fields {
        let Some((_, g)) = ours.fields.iter().find(|(l, _)| l == label) else {
            if ours.name == "SHADREC" {
                continue; // the plain 7.1 view's flag fields: not ours to print
            }
            panic!("{w}: {} has no field {label:?}", want.name);
        };
        if !addrs && ours.addrs.contains(label) {
            continue;
        }
        if g != value {
            diffs.push(format!("{w}.{label}: {g} != {value}"));
        }
    }
}

/// A control list: the packet sequence must be the reference's exactly.
fn list(w: &str, ours: &[u8], want: &[Pk], addrs: bool, diffs: &mut Vec<String>) -> usize {
    let got: Vec<Pk> = split(ours).into_iter().map(decode).collect();
    let names = |v: &[Pk]| v.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&got), names(want), "{w}: the packet sequence");
    for (i, (g, p)) in got.iter().zip(want).enumerate() {
        fields(&format!("{w} #{i} {}", p.name), g, p, addrs, diffs);
    }
    got.len()
}

/// tools/v3d/check71.d gl_cmp.py `key`: a state packet's slot.
fn key(p: &Pk) -> String {
    let get = |k: &str| p.fields.iter().find(|f| f.0 == k).map(|f| f.1.clone()).unwrap_or("None".into());
    match p.name.as_str() {
        "STENCIL_CFG" => format!("STENCIL_CFG[front {} back {}]", get("Front Config"), get("Back Config")),
        "BLEND_CFG" => format!("BLEND_CFG[{}]", get("Render Target Mask")),
        n if n.ends_with("_FLAGS") && !n.starts_with("ZERO_ALL") => {
            format!("{n}[{}]", p.fields.first().map(|f| f.1.as_str()).unwrap_or(""))
        }
        n => n.to_string(),
    }
}

/// The state packets in effect at a draw (by `key`) and the draw packet.
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

/// The packet names of `a` and not `b` (`+`) and of `b` and not `a` (`-`),
/// aligned by their longest common subsequence.
fn seq_diff(a: &[Pk], b: &[Pk]) -> Vec<String> {
    let (n, m) = (a.len(), b.len());
    let mut l = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if a[i].name == b[j].name { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n || j < m {
        if i < n && j < m && a[i].name == b[j].name {
            i += 1;
            j += 1;
        } else if j == m || (i < n && l[i + 1][j] >= l[i][j + 1]) {
            out.push(format!("+{}", a[i].name));
            i += 1;
        } else {
            out.push(format!("-{}", b[j].name));
            j += 1;
        }
    }
    out
}

/// A binning list with draws: the prologue and the epilogue packet by
/// packet, and at each draw the draw packet and every state packet in effect
/// (Mesa emits only what changed, v3d_job71 every draw's whole state: the
/// raw sequences' difference is one entry, `bcl sequence: ...`).
fn bcl_list(ours: &[u8], want: &[Pk], diffs: &mut Vec<String>) -> usize {
    let got: Vec<Pk> = split(ours).into_iter().map(decode).collect();
    let (pa, da, ta) = bcl_split(&got);
    let (pb, db, tb) = bcl_split(want);
    let names = |v: &[Pk]| v.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&pa), names(&pb), "bcl prologue: the packet sequence");
    assert_eq!(names(&ta), names(&tb), "bcl epilogue: the packet sequence");
    assert_eq!(da.len(), db.len(), "bcl: the draws");
    for (i, (g, p)) in pa.iter().zip(&pb).chain(ta.iter().zip(&tb)).enumerate() {
        fields(&format!("bcl #{i} {}", p.name), g, p, false, diffs);
    }
    for (n, ((sa, xa), (sb, xb))) in da.iter().zip(&db).enumerate() {
        assert_eq!(xa.name, xb.name, "bcl draw {n}: the draw packet");
        fields(&format!("bcl draw {n} {}", xb.name), xa, xb, false, diffs);
        let ka: Vec<&String> = sa.keys().collect();
        let kb: Vec<&String> = sb.keys().collect();
        assert_eq!(ka, kb, "bcl draw {n}: the state packets in effect");
        for (k, q) in sb {
            fields(&format!("bcl draw {n} {k}"), &sa[k], q, false, diffs);
        }
    }
    let d = seq_diff(&got, want);
    if !d.is_empty() {
        diffs.push(format!("bcl sequence: {}", d.join(" ")));
    }
    got.len()
}

/// Route 1 for one shape: the differences left, and (packets, records).
fn shape_diffs(shape: &str) -> (Vec<(usize, String)>, usize, usize) {
    let (refs, flushes) = run(shape);
    assert_eq!(flushes.len(), refs.len(), "gl_{shape}: jobs");
    let mut out = vec![];
    let (mut packets, mut records) = (0, 0);
    for (k, (f, r)) in flushes.iter().zip(&refs).enumerate() {
        let w = format!("gl_{shape} job {k}");
        let o = assemble_flush(f);
        let draws = r.bcl().iter().filter(|p| DRAWS.contains(&p.name.as_str())).count();
        assert_eq!(o.records.len(), draws, "{w}: draws");
        assert_eq!(r.records.len(), draws, "{w}: Mesa's records");
        // a job without a draw: its buffers are the reference's, addresses
        // and all; with a draw the indirect buffer's layout is ours
        let addrs = draws == 0;
        let mut d = vec![];
        packets += if addrs { list("bcl", &o.bcl, r.bcl(), addrs, &mut d) } else { bcl_list(&o.bcl, r.bcl(), &mut d) };
        packets += list("rcl", &o.rcl, r.rcl(), addrs, &mut d);
        packets += list("tiles", &o.tiles, r.ind().1, addrs, &mut d);
        // the tile allocation: the reference's rounded up to the Pi 5's page
        assert_eq!(u64::from(o.qms), r.bin[3].next_multiple_of(u64::from(backend71::PAGE_SIZE)), "{w}: qms");
        for (n, (a, b)) in o.records.iter().zip(&r.records).enumerate() {
            let plain = b.fields.iter().filter(|(l, _)| a.fields.iter().any(|(m, _)| m == l)).count();
            assert!(plain >= 20, "{w} draw {n}: the record's plain fields: {plain}");
            fields(&format!("draw {n} SHADREC"), a, b, false, &mut d);
            records += 1;
        }
        for (n, (ga, gb)) in o.attrs.iter().zip(&r.attrs).enumerate() {
            assert_eq!(ga.len(), gb.len(), "{w} draw {n}: attribute records");
            for (i, (a, b)) in ga.iter().zip(gb).enumerate() {
                fields(&format!("draw {n} ATTR {i}"), a, b, false, &mut d);
            }
        }
        out.extend(d.into_iter().map(|s| (k, s)));
    }
    (out, packets, records)
}

#[test]
fn the_three_cases_are_mesas_jobs_but_the_pinned_differences() {
    let mut failures = vec![];
    let mut used = [0usize; PINNED.len()];
    for (case, shape) in CASES {
        let (diffs, packets, records) = shape_diffs(shape);
        let mut pinned = 0;
        for (k, d) in &diffs {
            match PINNED.iter().position(|p| p.0 == shape && p.1 == *k && p.2 == d) {
                Some(i) => {
                    used[i] += 1;
                    pinned += 1;
                }
                None => failures.push(format!("case {case} gl_{shape} job {k}: {d}")),
            }
        }
        println!(
            "case {case} gl_{shape}: {packets} packets, {records} records, {} differences, {pinned} pinned",
            diffs.len() - pinned
        );
    }
    for (p, n) in PINNED.iter().zip(&used) {
        if *n != 1 {
            failures.push(format!("pin gl_{} job {} '{}' matched {n} times ({})", p.0, p.1, p.2, p.3));
        }
    }
    assert!(failures.is_empty(), "{} difference(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn the_jobs_without_a_draw_have_no_difference_at_all() {
    // every job that is one tile-buffer clear: zero differences, the
    // addresses included (PINNED names none of them)
    let mut jobs = vec![];
    for (_, shape) in CASES {
        let (refs, _) = run(shape);
        let (diffs, _, _) = shape_diffs(shape);
        for (k, r) in refs.iter().enumerate() {
            if r.bcl().iter().any(|p| DRAWS.contains(&p.name.as_str())) {
                continue;
            }
            assert!(!PINNED.iter().any(|p| p.0 == shape && p.1 == k), "gl_{shape} job {k} is pinned");
            let d: Vec<&String> = diffs.iter().filter(|(j, _)| *j == k).map(|(_, s)| s).collect();
            assert!(d.is_empty(), "gl_{shape} job {k}: {d:?}");
            jobs.push((shape, k));
        }
    }
    // case 5 job 0; case 66 jobs 0 and 1; case 102 jobs 0, 2 and 4
    assert_eq!(
        jobs,
        [("scissor", 0), ("viewport", 0), ("viewport", 1), ("colormask", 0), ("colormask", 2), ("colormask", 4)]
    );
}

// ---- route 2: backend71::assemble with a Frame71 --------------------------------

/// Route 2's render list against Mesa's on a whole clear of gl_test's window
/// (ours != Mesa's): `plan_frame` starts the unused depth/stencil plane at
/// zskeep71's undefined values (Z 1.0) and sets `zs_cleared` only for a TLB
/// clear of a plane, where fill71 (and Mesa) start an unused plane at Mesa's
/// zeroed 0.0 and clear the depth/stencil tile buffer early when nothing is
/// loaded (src/v3d/fill71.rs, the start values; Mesa's `early_zs_clear`).
/// The same on every whole-clear job of the three shapes.
const DIRECT_ZS: [(&str, &str); 2] = [
    ("rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early Depth/Stencil Clear: 0 != 1", DIRECT_ZS_WHY),
    ("rcl #2 TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES.Z Clear Value: 1.000000 != 0.000000", DIRECT_ZS_WHY),
];
const DIRECT_ZS_WHY: &str =
    "backend71::plan_frame's unused window depth/stencil start is not fill71's (Mesa's): Z 1.0 and no early clear";

const REF_CLEAR_DRAW: &str = "backend71: a masked, scissored or partial clear is a clear draw: Frame71 has no draw for it (fill71's clear quad: assemble_filled)";

/// The flush's opening glClear as backend71's `Frame71` (GL facts) over
/// fill71's target descriptor.
fn frame71(f: &Flush) -> (backend71::Frame71<'static>, ffi71::V3dJob71Target) {
    let c = &f.clear;
    let colors: &'static [admit71::ColorTarget] = Box::leak(Box::new([admit71::ColorTarget {
        gl_format: GL_RGBA8,
        uses: admit71::BufferUse { load: f.hist.colors[0], store: true, clear: true },
        clear_color: c.color.map(rt71::ClearColor::Float),
        sampled: false,
    }]));
    let off = clearplan71::TargetIn { cleared: false, mask: 0 };
    let mut targets = [off; clearplan71::MAX_TARGETS];
    targets[0] = clearplan71::TargetIn { cleared: true, mask: c.color_mask };
    let scissor = match c.scissor {
        Some([x, y, w, h]) => clearplan71::ScissorIn { enabled: true, x, y, w, h },
        None => clearplan71::ScissorIn { enabled: false, x: 0, y: 0, w: 0, h: 0 },
    };
    let ci = clearplan71::ClearIn {
        fb_w: SIZE,
        fb_h: SIZE,
        n_targets: 1,
        targets,
        has_depth: true,
        has_stencil: true,
        clear_depth: false,
        clear_stencil: false,
        depth_mask: c.depth_mask,
        stencil_mask: c.stencil_mask,
        scissor,
        ds_valid: false,
        draws_queued: false,
    };
    let target = f.jobs[0].target;
    let ds = admit71::DepthStencil {
        gl_format: fill71::GL_DEPTH24_STENCIL8,
        depth: admit71::BufferUse::default(),
        stencil: admit71::BufferUse::default(),
    };
    let frame = backend71::Frame71 {
        gl: admit71::Frame {
            width: SIZE,
            height: SIZE,
            colors,
            depth_stencil: Some(ds),
            msaa: false,
            double_buffer: false,
        },
        target,
        clear: Some(ci),
        zs: backend71::ZsKeep71 {
            target: zskeep71::Target71 { layout: zskeep71::Layout71::Combined, depth: true, stencil: true },
            z: f.hist.z,
            s: f.hist.s,
            job: zskeep71::Job71::NONE,
        },
    };
    (frame, target)
}

#[test]
fn the_direct_route_builds_the_whole_clears_and_names_the_clear_draws() {
    // (shape, job): assembled by route 2 (true) or refused by name (false)
    let want: [(&str, [bool; 6]); 3] = [
        ("scissor", [true, false, false, false, false, false]),
        ("viewport", [true, true, false, false, false, false]),
        ("colormask", [true, false, true, false, true, false]),
    ];
    for (case, shape) in CASES {
        let (refs, flushes) = run(shape);
        let wanted = want.iter().find(|w| w.0 == shape).unwrap().1;
        for (k, f) in flushes.iter().enumerate() {
            let w = format!("case {case} gl_{shape} job {k}");
            let (frame, target) = frame71(f);
            assert_eq!(frame.target, target);
            let p = f.place;
            let mut h = Harness { gpu: [p.bcl, p.rcl, p.ind], lists: Default::default() };
            match backend71::assemble(&table(), &frame, &[], &mut h) {
                Ok(a) => {
                    assert!(wanted[k], "{w}: assembled, expected the clear-draw refusal");
                    assert!(matches!(a.frame.clear_plan, Some(clearplan71::ClearPlan::TileClear { targets: 1, .. })));
                    // the binning and tile lists are route 1's bytes, which
                    // are Mesa's job exactly
                    let o = assemble_flush(f);
                    assert_eq!(h.bytes(List71::Bcl), &o.bcl[..], "{w}: binning list");
                    let (ls, le) = a.built.tile_list;
                    let ind = h.bytes(List71::Ind);
                    assert_eq!(&ind[(ls - p.ind) as usize..(le - p.ind) as usize], &o.tiles[..], "{w}: tile list");
                    assert_eq!(a.built.tile_alloc_size, o.qms, "{w}: qms");
                    // the render list: Mesa's but the window's unused
                    // depth/stencil start (DIRECT_ZS)
                    let mut d = vec![];
                    list("rcl", h.bytes(List71::Rcl), refs[k].rcl(), true, &mut d);
                    assert_eq!(d, DIRECT_ZS.map(|(s, _)| s), "{w}: {}", DIRECT_ZS[0].1);
                    // which is the frame decisions' one difference from fill71's
                    let mut c = a.frame.clear;
                    assert_eq!((c.z, c.zs_cleared), (1.0, false), "{w}");
                    assert_eq!((f.jobs[0].clear.z, f.jobs[0].clear.zs_cleared), (0.0, true), "{w}");
                    (c.z, c.zs_cleared) = (0.0, true);
                    assert_eq!(c, f.jobs[0].clear, "{w}: the frame decisions");
                }
                Err(e) => {
                    assert!(!wanted[k], "{w}: refused: {e}");
                    // a clear draw: route 1's clear quad, never this route
                    assert_eq!(e.to_string(), REF_CLEAR_DRAW, "{w}");
                    assert!(refs[k].bcl().iter().any(|p| p.name == "GL_SHADER"), "{w}: Mesa draws it too");
                }
            }
        }
        assert_eq!(flushes.len(), refs.len());
        assert!(flushes.len() <= 6);
    }
}

#[test]
fn the_viewport_does_not_bound_a_clear() {
    // case 66: the clear at a quarter viewport is a whole-target tile-buffer
    // clear, like the full-viewport one before it: the two render lists differ
    // only in the clear colour (white, then blue)
    let (refs, flushes) = run("viewport");
    let o: Vec<Ours> = flushes.iter().take(2).map(assemble_flush).collect();
    let a: Vec<Pk> = split(&o[0].rcl).into_iter().map(decode).collect();
    let b: Vec<Pk> = split(&o[1].rcl).into_iter().map(decode).collect();
    let mut d = vec![];
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        fields(&format!("#{i} {}", x.name), x, y, false, &mut d);
    }
    assert_eq!(a.len(), b.len());
    assert_eq!(
        d,
        ["#1 TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1.Clear Color low bits: 4294967295 != 4294901760"],
        "white against blue"
    );
    // and the reference's: the same packets, the same difference
    let mut e = vec![];
    for (i, (x, y)) in refs[0].rcl().iter().zip(refs[1].rcl()).enumerate() {
        let mut x = x.clone();
        x.addrs = x.fields.iter().filter(|f| f.1.starts_with("0x")).map(|f| f.0.clone()).collect();
        fields(&format!("#{i} {}", x.name), &x, y, false, &mut e);
    }
    assert_eq!(d, e);
}
