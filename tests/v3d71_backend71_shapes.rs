// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/backend71.rs`'s fill71 path (`assemble_filled`) against Mesa's
//! 7.1 references for the six gl_test shapes and the 4- and 8-target clears,
//! tools/v3d/pi5/gl_{scissor,scisbox,writemask,viewport,fbods,colormask,
//! mrt4,mrt8}.clif (cases 5, 58, 59, 66, 67, 102; tools/v3d/pi5/README.md
//! "The gl_test shapes"; docs/notes/v3d71-gl-table.md sections 2 to 6).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_backend71_shapes
//! ```
//!
//! Each shape is its GL calls (tools/v3d/pi5/src/gl_shapes.py, one flush per
//! glFinish) through the GL state model of tests/v3d71_fill.rs, `fill71::fill`
//! turns each flush into `FilledJob`s (masked and scissored clears are clear
//! quads, a whole clear after queued draws opens a new job), and
//! `backend71::assemble_filled` builds every job through the C builder
//! (driver/src/v3d/v3d_job71.c in a child process, tests/v3d71_backend71_harness.c,
//! as tests/v3d71_backend71.rs runs it) and checks every draw's uniform
//! streams, D0 shader record and attribute records against uniform71,
//! record71 and attr71.
//!
//! The programs are ours, not Mesa's: gl_test's pair is
//! tools/v3d/gltest-shaders71/ours_color_uv.h and the clear quad's is
//! ours_clear.h (our 7.1 compiler's output, pinned), read here as data.
//!
//! Compared with the reference, job by job: the binning list (the state each
//! draw sees, the draw packets, prologue and epilogue), the render list and
//! the generic tile list packet by packet, the tile allocation size, and for
//! every draw the shader record's fields (the plain ones: Mesa's dump decodes
//! the D0 record's flag word at the plain 7.1 layout, tools/v3d/pi5/README.md
//! "Differences" item 4) and its attribute records; every field the dump
//! prints except addresses. The shim's buffers are 4 KiB allocations at
//! addresses of its own per job, so the addresses are the gap `ADDR` (not
//! compared); the uniform streams and QPU code are our programs', not Mesa's
//! (the gap `PROGRAM`, checked against uniform71 instead). Every other
//! difference must match one named entry of [`ALLOW`] with its reason, and
//! every entry must still match one: a gap that stops matching fails.
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

use backend71::{Builder71, List71};
use ffi71::*;
use fill71::{ClearQuad, ClearReq, ColorBuffer, DrawIn, DsBuffer, FilledJob, Framebuffer, History, Op, Program};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use zskeep71::Known71;

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

const G_SIZE: u32 = 64;
const G_BCL: u32 = 0x100000;
const G_RCL: u32 = 0x110000;
const G_IND: u32 = 0x120000;
const G_CODE: u32 = 0x200000;
const G_VTX: u32 = 0x300000;
const G_TILE: u32 = 0x400000;
const G_ZS: u32 = 0x2000000;
const fn g_rt(n: u32) -> u32 {
    0x1000000 + n * 0x100000
}

const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLES: u32 = 4;
const GL_TRIANGLE_FAN: u32 = 6;

/* ---- our programs (tools/v3d/gltest-shaders71/ours_*.h, read as data) ---- */

/// The programs' headers, compiled in (the files are part of the binary).
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

/// One of our programs as fill71 takes it, the uniform lists leaked to
/// `'static` (one per process).
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
    // the record's flag fields the header states from the same programs
    assert_eq!(desc.fs.num_varyings, h.d(&format!("{up}_REC_NUMBER_OF_VARYINGS_IN_FRAGMENT_SHADER")));
    assert_eq!(desc.vs.vpm_output_size, h.d(&format!("{up}_REC_VERTEX_SHADER_OUTPUT_VPM_SEGMENT_SIZE")));
    assert_eq!(desc.cs.vpm_output_size, h.d(&format!("{up}_REC_COORDINATE_SHADER_OUTPUT_VPM_SEGMENT_SIZE")));
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

/// gl_test's 40-byte vertex: pos4 +0, colour4 +16, uv2 +32.
static GL_ATTRS: [attr71::Attr; 3] =
    [float_attr(4, 40, G_VTX), float_attr(4, 40, G_VTX + 16), float_attr(2, 40, G_VTX + 32)];
/// Our clear quad: fill71's `FilledDraw::quad`, four (x, y, z, w) floats a
/// vertex (16 bytes), where the caller writes it.
const QUAD_ATTR: attr71::Attr = float_attr(4, 16, G_VTX + 0x100);

/// (gl_test's program, the clear quad's): ours_color_uv and ours_clear.
fn programs() -> &'static (Program<'static>, Program<'static>) {
    static P: OnceLock<(Program<'static>, Program<'static>)> = OnceLock::new();
    P.get_or_init(|| {
        (our_program(OURS_COLOR_UV, "ours71_color_uv", G_CODE), our_program(OURS_CLEAR, "ours71_clear", G_CODE + 0x300))
    })
}

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &programs().1, attr: QUAD_ATTR, dummy_addr: G_VTX }
}

fn table() -> backend71::Table71 {
    // HUB_IDENT1 and HUB_IDENT3 of the board (tools/v3d/pi5/README.md)
    backend71::init(0x0008_1117, 0x0002_0a10).expect("the Pi 5's identity")
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
        // rows at once (one per feature set), all with the same
        // CARGO_TARGET_TMPDIR; a shared `harness` would be rebuilt by one row
        // while another runs it.
        let dir =
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("v3d71_backend71_shapes-{}", std::process::id()));
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

/* ---- the GL state model ---------------------------------------------------- */

const COLOR: u32 = 1;
const DEPTH: u32 = 2;
const STENCIL: u32 = 4;

/// A framebuffer of the shape: colour textures and a depth/stencil
/// renderbuffer, by id (what a flush's history is kept per).
#[derive(Clone, Debug)]
struct Fb {
    colors: Vec<(u32, u32)>,
    ds: Option<(u32, u32)>,
    /// The depth/stencil buffer has memory (a renderbuffer); false: it lives
    /// in the tile buffer only (a window without a kept depth/stencil).
    ds_kept: bool,
}

impl Fb {
    fn window() -> Fb {
        Fb { colors: vec![(100, fill71::GL_RGBA8)], ds: Some((200, fill71::GL_DEPTH24_STENCIL8)), ds_kept: true }
    }
    fn color(tex: u32, fmt: u32) -> Fb {
        Fb { colors: vec![(tex, fmt)], ds: None, ds_kept: true }
    }
}

/// The GL calls of a shape, flush by flush; the history of every buffer.
struct Gl {
    fb: Fb,
    st: draw71::DrawState,
    clear_color: [f32; 4],
    clear_depth: f32,
    clear_stencil: u32,
    stencil_write: u8,
    scissor_on: bool,
    scissor: [i32; 4],
    draw_buffers: u8,
    ops: Vec<Op<'static>>,
    colors: BTreeMap<u32, bool>,
    ds: BTreeMap<u32, (Known71, Known71)>,
    jobs: Vec<FilledJob>,
    /// Each job's framebuffer (colour buffers, depth/stencil buffer).
    fbs: Vec<(Vec<ColorBuffer>, Option<DsBuffer>)>,
    refusal: Option<fill71::Refusal>,
}

impl Gl {
    fn new(fb: Fb) -> Gl {
        let mut st = draw71::DrawState::gl_default(G_SIZE, G_SIZE, false, 1);
        st.viewport = [0.0, 0.0, G_SIZE as f32, G_SIZE as f32];
        Gl {
            fb,
            st,
            clear_color: [0.0; 4],
            clear_depth: 1.0,
            clear_stencil: 0,
            stencil_write: 0xff,
            scissor_on: false,
            scissor: [0, 0, G_SIZE as i32, G_SIZE as i32],
            draw_buffers: 1,
            ops: vec![],
            colors: BTreeMap::new(),
            ds: BTreeMap::new(),
            jobs: vec![],
            fbs: vec![],
            refusal: None,
        }
    }

    fn bind(&mut self, fb: Fb) {
        assert!(self.ops.is_empty(), "bind between flushes only (glBindFramebuffer flushes)");
        self.fb = fb;
    }

    fn color_mask(&mut self, m: u8) {
        for t in &mut self.st.targets {
            t.color_mask = m;
        }
    }

    fn stencil_func(&mut self, func: draw71::Compare, r: u32) {
        self.st.stencil_front.func = func;
        self.st.stencil_front.reference = r;
        self.st.stencil_front.value_mask = 0xff;
    }

    fn stencil_mask(&mut self, m: u8) {
        self.stencil_write = m;
        self.st.stencil_front.write_mask = u32::from(m);
    }

    fn clear(&mut self, bits: u32) {
        self.ops.push(Op::Clear(ClearReq {
            color: (bits & COLOR != 0).then_some(self.clear_color),
            draw_buffers: self.draw_buffers,
            color_mask: self.st.targets[0].color_mask,
            depth: (bits & DEPTH != 0).then_some(self.clear_depth),
            depth_mask: self.st.depth_write,
            stencil: (bits & STENCIL != 0).then_some(self.clear_stencil),
            stencil_mask: self.stencil_write,
            scissor: self.scissor_on.then_some(self.scissor),
        }));
    }

    /// glDrawArrays(GL_TRIANGLES, first, 6) with gl_test's program.
    fn draw(&mut self, first: u64) {
        let mut st = self.st;
        st.scissor = self.scissor_on.then_some(self.scissor);
        self.ops.push(Op::Draw(Box::new(DrawIn {
            state: st,
            line_width: 1.0,
            blend_color: [0.0; 4],
            program: &programs().0,
            attrs: &GL_ATTRS,
            dummy_addr: G_VTX,
            prim: prim71::Draw::arrays(GL_TRIANGLES, first, 6),
            vs_words: &[],
            fs_words: &[],
            oq_addr: 0,
        })));
    }

    fn framebuffer(&self) -> (Vec<ColorBuffer>, Option<DsBuffer>) {
        let colors = self
            .fb
            .colors
            .iter()
            .enumerate()
            .map(|(n, &(_, fmt))| ColorBuffer {
                gl_format: fmt,
                address: g_rt(n as u32),
                memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
                height_in_ub_or_stride: 8,
            })
            .collect();
        let ds = self.fb.ds.map(|(_, fmt)| DsBuffer {
            gl_format: fmt,
            address: if self.fb.ds_kept { G_ZS } else { 0 },
            memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub: 8,
        });
        (colors, ds)
    }

    fn history(&self) -> History {
        let mut h = History::NEW;
        for (n, (tex, _)) in self.fb.colors.iter().enumerate().take(h.colors.len()) {
            h.colors[n] = *self.colors.get(tex).unwrap_or(&false);
        }
        if let Some((rb, _)) = self.fb.ds {
            (h.z, h.s) = *self.ds.get(&rb).unwrap_or(&(Known71::Untouched, Known71::Untouched));
        }
        h
    }

    /// glFinish: fill the flush, keep its jobs and the buffers' history.
    fn finish(&mut self) {
        let ops = std::mem::take(&mut self.ops);
        if self.refusal.is_some() {
            return;
        }
        let (colors, ds) = self.framebuffer();
        let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds };
        match fill71::fill(&fb, record71::BCM2712_D0, &self.history(), &clear_quad(), &ops) {
            Ok(jobs) => {
                if let Some(last) = jobs.last() {
                    for (n, (tex, _)) in self.fb.colors.iter().enumerate() {
                        self.colors.insert(*tex, last.after.colors[n]);
                    }
                    if let Some((rb, _)) = self.fb.ds {
                        self.ds.insert(rb, (last.after.z, last.after.s));
                    }
                }
                for _ in &jobs {
                    self.fbs.push((colors.clone(), ds));
                }
                self.jobs.extend(jobs);
            }
            Err(r) => self.refusal = Some(r),
        }
    }
}

/* ---- the shapes: gl_shapes.py's calls (tools/v3d/pi5/src/gl_shapes.py:324-567) ---- */

fn shape_scissor(g: &mut Gl) {
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.scissor_on = true;
    g.scissor = [0, 0, 32, 32];
    g.clear(COLOR);
    g.draw(0);
    g.finish();
}

fn shape_scisbox(g: &mut Gl) {
    for step in 0..3 {
        g.clear_color = [0.0, 0.0, 1.0, 1.0];
        g.clear(COLOR);
        if step == 1 {
            g.draw(0);
        }
        g.scissor_on = true;
        g.scissor = [16, 16, if step == 2 { 0 } else { 32 }, 32];
        g.clear_color = [1.0, 0.0, 0.0, 1.0];
        g.clear(COLOR);
        g.finish();
        g.scissor_on = false;
    }
}

fn shape_writemask(g: &mut Gl) {
    use draw71::Compare::*;
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR);
    g.color_mask(0);
    g.clear_color = [1.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.color_mask(0xf);
    g.finish();

    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear_depth = 1.0;
    g.st.depth_write = true;
    g.clear(COLOR | DEPTH);
    g.st.depth_test = true;
    g.st.depth_func = Less;
    g.draw(0);
    g.st.depth_write = false;
    g.clear_depth = 1.0;
    g.clear(DEPTH);
    g.st.depth_write = true;
    g.draw(0);
    g.finish();
    g.st.depth_test = false;

    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.stencil_mask(0xff);
    g.clear_stencil = 0;
    g.clear(COLOR | STENCIL);
    g.stencil_mask(0x0f);
    g.clear_stencil = 0xff;
    g.clear(STENCIL);
    g.stencil_mask(0xff);
    g.st.stencil_test = true;
    g.stencil_func(Equal, 0xff);
    g.draw(0);
    g.finish();
    g.stencil_func(Equal, 0x0f);
    g.draw(0);
    g.finish();
    g.st.stencil_test = false;
}

fn shape_viewport(g: &mut Gl) {
    g.clear_color = [1.0, 1.0, 1.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.st.viewport = [0.0, 0.0, 32.0, 32.0];
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.st.viewport = [0.0, 0.0, 64.0, 64.0];
    g.scissor_on = true;
    g.scissor = [0, 0, 32, 32];
    g.clear_color = [0.0, 1.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.scissor_on = false;
}

/// Case 67: texture 1 (RGBA), texture 2 (GL_RGB), renderbuffers 11 and 12
/// (DEPTH_COMPONENT16), 13 (STENCIL_INDEX8).
fn shape_fbods(g: &mut Gl) {
    use draw71::Compare::*;
    let tex = 1;
    g.bind(Fb::color(tex, fill71::GL_RGBA8));
    g.st.depth_test = true;
    g.st.depth_func = Less;
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR | DEPTH);
    g.draw(0);
    g.draw(6);
    g.finish();
    g.st.stencil_test = true;
    g.stencil_func(Equal, 1);
    g.clear_stencil = 0;
    g.clear(COLOR | STENCIL);
    g.draw(0);
    g.finish();
    g.st.stencil_test = false;

    g.bind(Fb::color(2, fill71::GL_RGB8));
    g.st.depth_test = false;
    g.clear_color = [1.0, 0.0, 0.0, 0.25];
    g.clear(COLOR);
    g.finish();

    let fbo_a =
        Fb { colors: vec![(tex, fill71::GL_RGBA8)], ds: Some((11, fill71::GL_DEPTH_COMPONENT16)), ds_kept: true };
    let fbo_b =
        Fb { colors: vec![(tex, fill71::GL_RGBA8)], ds: Some((12, fill71::GL_DEPTH_COMPONENT16)), ds_kept: true };
    g.bind(fbo_a);
    g.st.depth_test = true;
    g.st.depth_func = Less;
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear_depth = 1.0;
    g.clear(COLOR | DEPTH);
    g.draw(0);
    g.finish();
    g.bind(fbo_b);
    g.draw(6);
    g.finish();
    g.st.depth_test = false;

    g.bind(Fb { colors: vec![(tex, fill71::GL_RGBA8)], ds: Some((13, fill71::GL_STENCIL_INDEX8)), ds_kept: true });
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear_stencil = 0;
    g.clear(COLOR | STENCIL);
    g.scissor_on = true;
    g.scissor = [16, 16, 32, 32];
    g.clear_stencil = 1;
    g.clear(STENCIL);
    g.scissor_on = false;
    g.st.stencil_test = true;
    g.stencil_func(Equal, 0);
    g.draw(0);
    g.finish();
    g.st.stencil_test = false;
    for step in 0..2 {
        g.clear_color = [0.0, 0.0, 1.0, 1.0];
        g.clear_stencil = 0;
        g.clear(COLOR | STENCIL);
        g.draw(6);
        g.scissor_on = true;
        g.scissor = [16, 16, 32, 32];
        g.clear_stencil = 1;
        g.clear(STENCIL);
        g.scissor_on = false;
        g.clear_stencil = 0;
        if step == 1 {
            g.finish();
        }
        g.st.stencil_test = true;
        g.stencil_func(Equal, 0);
        g.draw(0);
        g.finish();
        g.st.stencil_test = false;
    }
}

fn shape_colormask(g: &mut Gl) {
    g.color_mask(0xf);
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.color_mask(0b1001);
    g.clear_color = [1.0, 1.0, 1.0, 0.5];
    g.clear(COLOR);
    g.finish();
    g.color_mask(0xf);
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.scissor_on = true;
    g.scissor = [0, 0, 32, 32];
    g.color_mask(0b0010);
    g.clear_color = [0.0, 1.0, 0.0, 0.0];
    g.clear(COLOR);
    g.finish();
    g.scissor_on = false;
    g.color_mask(0xf);
    g.clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.color_mask(0b0100);
    g.clear_color = [0.0, 0.0, 1.0, 0.0];
    g.clear(COLOR);
    g.color_mask(0xf);
    g.draw(0);
    g.finish();
}

fn shape_mrt(g: &mut Gl, n: u32) {
    g.bind(Fb { colors: (0..n).map(|i| (300 + i, fill71::GL_RGBA8)).collect(), ds: None, ds_kept: true });
    g.st.nr_targets = n;
    g.draw_buffers = ((1u32 << n) - 1) as u8;
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR);
    g.finish();
}

const SHAPES: [&str; 8] = ["scissor", "scisbox", "writemask", "viewport", "fbods", "colormask", "mrt4", "mrt8"];

fn run_shape(name: &str) -> Gl {
    let fb = match name {
        "fbods" | "mrt4" | "mrt8" => Fb::color(0, fill71::GL_RGBA8),
        _ => Fb::window(),
    };
    let mut g = Gl::new(fb);
    match name {
        "scissor" => shape_scissor(&mut g),
        "scisbox" => shape_scisbox(&mut g),
        "writemask" => shape_writemask(&mut g),
        "viewport" => shape_viewport(&mut g),
        "fbods" => shape_fbods(&mut g),
        "colormask" => shape_colormask(&mut g),
        "mrt4" => shape_mrt(&mut g, 4),
        "mrt8" => shape_mrt(&mut g, 8),
        _ => unreachable!(),
    }
    g
}

fn filled(name: &str) -> Vec<FilledJob> {
    let g = run_shape(name);
    if let Some(r) = &g.refusal {
        panic!("gl_{name}: refused: {r}");
    }
    g.jobs
}

/* ---- the references (gl_cmp.py's parse, tools/v3d/check71.d/job71.sh:221-411) ---- */

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

fn decode_list(defs: &[PacketDef], bytes: &[u8], what: &str) -> Vec<Pk> {
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
        let body = &bytes[i + 1..i + def.len];
        let fields = def.fields.iter().map(|f| (f.name.clone(), clif_value(f, bits(body, f.start, f.size)))).collect();
        out.push(Pk { name: def.name.clone(), fields });
        i += def.len;
    }
    out
}

/// A named difference: `(shape glob, difference glob, id, reason)`. The
/// first five are tests/v3d71_fill.rs's (v3d_job71's and fill71's own
/// differences from Mesa's jobs); the rest are this path's: the Pi 5's page
/// size and our clear program.
const ALLOW: [(&str, &str, &str, &str); 10] = [
    ("[!m]*", "* job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", "EZ", "v3d_job71 never enables early Z (job71.sh gl.allow EZ)"),
    ("[!m]*", "* job * bcl draw * POINT_SIZE.Point Size: 1.000000 != 0.125000", "PSIZE", "the clear quad: Mesa's blitter writes point size 0.125, v3d_job71 1.0; a TRIANGLE_FAN does not read it (job71.sh gl.allow PSIZE)"),
    ("[fw]*", "* job * tiles #8 CLEAR_RT: present != absent", "CLRT", "a job that loads every buffer: v3d_job71 still writes CLEAR_RENDER_TARGETS after the stores (job71.sh gl.allow CLRT)"),
    ("scisbox", "scisbox job 2 *", "EMPTYBOX", "clearplan71 drops a clear whose scissor box is empty; Mesa's blitter draws a quad that covers no pixel (no draw, no record)"),
    ("fbods", "fbods job 2 rcl #* TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1.Clear Color low bits: 4278190335 != 1073742079", "RGBALPHA", "a GL_RGB target keeps alpha 1 (the 4.2 table's rule): its clear writes alpha 1, Mesa's alpha 0.25"),
    ("fbods", "fbods job 4 rcl #* TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES.Z Clear Value: 1.000000 != 0.000000", "UNDEFZ", "zskeep71 starts an undefined depth plane the job uses at 1.0, Mesa's zeroed job at 0.0"),
    ("*", "* job * submit #0: 16384 != 12288", "PAGE", "the tile allocation is rounded to the Pi 5 kernel's 16 KiB page (backend71::PAGE_SIZE, tools/v3d/pi5/README.md 'Differences' item 2); the drm-shim's page is 4 KiB"),
    ("[!m]*", "* job * draw * ATTR 0.Vec size: 4 != 3", "QUADATTR", "our clear quad's position is fill71's FilledDraw::quad, four floats (x, y, z, w) a vertex; Mesa's blitter writes three"),
    ("[!m]*", "* job * draw * ATTR 0.Stride: 16 != 36", "QUADATTR", "the same: our quad's vertex is 16 bytes, Mesa's blitter vertex 36"),
    ("fbods", "fbods job 0 records: 2/2 != 1/1", "RECORDDUP", "two draws with the same programs and arrays: v3d_job71 writes a shader record (and GL_SHADER) per draw, Mesa reuses the first"),
];

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

/// One job as backend71 assembled it.
struct Ours {
    bcl: Vec<u8>,
    rcl: Vec<u8>,
    tiles: Vec<u8>,
    qms: u32,
    records: Vec<Pk>,
    attrs: Vec<Vec<Pk>>,
}

fn assemble_shape(shape: &str) -> Vec<Ours> {
    let g = run_shape(shape);
    if let Some(r) = &g.refusal {
        panic!("gl_{shape}: fill71 refused: {r}");
    }
    let t = table();
    let mut out = vec![];
    for (k, (job, (colors, ds))) in g.jobs.iter().zip(&g.fbs).enumerate() {
        let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors, ds: *ds };
        let mut h = Harness { gpu: [G_BCL, G_RCL, G_IND], lists: Default::default() };
        let a = backend71::assemble_filled(&t, &fb, job, &mut h)
            .unwrap_or_else(|e| panic!("gl_{shape} job {k}: backend71 refused: {e}"));
        let ind = h.bytes(List71::Ind);
        let at = |addr: u32, n: usize| &ind[(addr - G_IND) as usize..(addr - G_IND) as usize + n];
        let (ls, le) = a.built.tile_list;
        let mut records = vec![];
        let mut attrs = vec![];
        for (p, lo) in a.draws.iter().zip(&a.built.layouts) {
            records.push(decode_record(at(lo.record, record71::RECORD_SIZE)));
            attrs.push(
                (0..p.attrs.count)
                    .map(|i| decode_attr(at(lo.attr_records + (i * attr71::RECORD_BYTES) as u32, attr71::RECORD_BYTES)))
                    .collect(),
            );
        }
        out.push(Ours {
            bcl: h.bytes(List71::Bcl).to_vec(),
            rcl: h.bytes(List71::Rcl).to_vec(),
            tiles: at(ls, (le - ls) as usize).to_vec(),
            qms: a.built.tile_alloc_size,
            records,
            attrs,
        });
    }
    out
}

/// Every difference of a shape's jobs from the reference, gl_cmp.py's text.
fn shape_diffs(shape: &str, defs: &[PacketDef]) -> (Vec<String>, usize, usize) {
    let ours = assemble_shape(shape);
    let refs = ref_jobs(shape);
    assert_eq!(ours.len(), refs.len(), "gl_{shape}: jobs");
    let mut diffs = vec![];
    let (mut packets, mut draws) = (0, 0);
    for (k, (o, r)) in ours.iter().zip(&refs).enumerate() {
        let w = format!("{shape} job {k}");
        let bcl = decode_list(defs, &o.bcl, &format!("{w} bcl"));
        let rcl = decode_list(defs, &o.rcl, &format!("{w} rcl"));
        let tiles = decode_list(defs, &o.tiles, &format!("{w} tiles"));
        packets += r.bcl.len() + r.rcl.len() + r.tiles.len();
        bcl_cmp(&format!("{w} bcl"), &bcl, &r.bcl, &mut diffs);
        seq_cmp(&format!("{w} rcl"), &rcl, &r.rcl, &mut diffs);
        seq_cmp(&format!("{w} tiles"), &tiles, &r.tiles, &mut diffs);
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
            let b = plain_record(b);
            // the dump's plain fields: the varyings, VPM sizes, threading,
            // VCM cache and code addresses (the existing triangle test's bar)
            assert!(b.fields.len() >= 20, "{w} draw {n}: the record's plain fields: {}", b.fields.len());
            fld_cmp(&format!("{w} draw {n} SHADREC"), a, &b, &mut diffs);
            draws += 1;
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
    (diffs, packets, draws)
}

#[test]
fn every_shape_assembles_and_matches_the_reference_or_a_named_gap() {
    let defs = packet_defs();
    let mut failures = vec![];
    let mut named: BTreeMap<&str, usize> = BTreeMap::new();
    let mut used = vec![0usize; ALLOW.len()];
    for shape in SHAPES {
        let (diffs, packets, draws) = shape_diffs(shape, &defs);
        let mut left = 0;
        let mut here = 0;
        for d in &diffs {
            match ALLOW
                .iter()
                .position(|a| glob(a.0.as_bytes(), shape.as_bytes()) && glob(a.1.as_bytes(), d.as_bytes()))
            {
                Some(i) => {
                    used[i] += 1;
                    here += 1;
                    *named.entry(ALLOW[i].2).or_default() += 1;
                }
                None => {
                    left += 1;
                    failures.push(format!("gl_{shape}: {d}"));
                }
            }
        }
        println!("gl_{shape}: {packets} reference packets, {draws} records, {left} differences, {here} named");
    }
    for (a, n) in ALLOW.iter().zip(&used) {
        if *n == 0 {
            failures.push(format!("stale named difference {} ({} / {})", a.2, a.0, a.1));
        }
    }
    for (id, n) in &named {
        let reason = ALLOW.iter().find(|a| a.2 == *id).unwrap().3;
        println!("named {id} ({n}): {reason}");
    }
    assert!(failures.is_empty(), "{} difference(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn the_jobs_and_draws_are_the_references_counts() {
    // fill71's split (a whole clear after queued draws opens a job; a clear
    // quad is a draw) gives the reference's jobs; the draws of each job are
    // the reference's but where a named gap says otherwise (EMPTYBOX)
    for shape in SHAPES {
        let ours = assemble_shape(shape);
        let refs = ref_jobs(shape);
        assert_eq!(ours.len(), refs.len(), "gl_{shape}: jobs");
        for (k, (o, r)) in ours.iter().zip(&refs).enumerate() {
            let draws = r.bcl.iter().filter(|p| DRAWS.contains(&p.name.as_str())).count();
            let want = if (shape, k) == ("scisbox", 2) { 0 } else { draws };
            assert_eq!(o.records.len(), want, "gl_{shape} job {k}: draws");
        }
    }
}

/* ---- refusals, by name ----------------------------------------------------------- */

fn one_job(fd_edit: impl FnOnce(&mut fill71::FilledDraw)) -> Result<backend71::AssembledJob71, backend71::Refusal> {
    let mut g = Gl::new(Fb::window());
    g.clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR);
    g.draw(0);
    g.finish();
    let (mut job, (colors, ds)) = (g.jobs.remove(0), g.fbs.remove(0));
    fd_edit(&mut job.draws[0]);
    let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds };
    let mut h = Harness { gpu: [G_BCL, G_RCL, G_IND], lists: Default::default() };
    backend71::assemble_filled(&table(), &fb, &job, &mut h)
}

#[test]
fn what_draw71_does_not_carry_is_refused_by_name() {
    one_job(|_| {}).expect("gl_test's draw");
    let e = one_job(|d| d.indexed = true).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71", backend71::FILLED_REFUSALS[0]));
    let e = one_job(|d| d.instances = 2).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71", backend71::FILLED_REFUSALS[1]));
    let e = one_job(|d| d.vs_writes_psiz = true).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71", backend71::FILLED_REFUSALS[2]));
    // every draw is planned before the builder runs: a uniform the 7.1 table
    // does not fill yet is uniform71's named refusal
    let e = one_job(|d| d.vs_uniforms[0].contents = uniform71::TMU_CONFIG_P0).unwrap_err();
    assert_eq!(e.to_string(), "uniform71: TMU config uniform (contents 40): the 7.1 texture state is not written yet");
}

#[test]
fn a_target_other_than_the_framebuffer_is_refused() {
    let mut g = Gl::new(Fb::window());
    g.clear(COLOR);
    g.finish();
    let (mut job, (colors, ds)) = (g.jobs.remove(0), g.fbs.remove(0));
    job.target.width = 32;
    let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds };
    let mut h = Harness { gpu: [G_BCL, G_RCL, G_IND], lists: Default::default() };
    let e = backend71::assemble_filled(&table(), &fb, &job, &mut h).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71", backend71::FILLED_REFUSALS[3]));
}
