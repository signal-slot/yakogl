// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/backend71tex.rs` (the textured half of the 7.1 job assembly, P10)
//! against Mesa's 7.1 references tools/v3d/pi5/gl_tex2d.clif, gl_texfmt.clif
//! and gl_cube.clif (tools/v3d/pi5/README.md "The GL calls";
//! docs/notes/v3d71-gl-table.md section 3).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_backend71_tex
//! ```
//!
//! Each shape is its GL calls (tools/v3d/pi5/src/gl_shapes.py, tex2d, texfmt
//! and cube: one job per glFinish) through the GL state model of
//! tests/v3d71_backend71_shapes.rs. `fill71::fill` turns each flush into a
//! `FilledJob`, `texfill71::fill` composes the texture and the sampler, and
//! `backend71tex::assemble_filled` allocates the records, predicts the
//! uniform streams with `texuni71` and builds the job through the C builders
//! (driver/src/v3d/v3d_job71.c and v3d_tex71.c in a child process,
//! tests/v3d71_backend71_tex_harness.c).
//!
//! The programs are Mesa's offline 7.1 binaries of
//! tools/v3d/gltest-mesa71-tex/ (`texture` for tex2d and texfmt, `cube` for
//! cube; read as data from the .json files): tools/v3d/gltest-shaders71-tex/
//! is not on this tip. `texture` is the program of the references
//! (gl_test's UV pair); `cube` is case 41's ES 3.00 pair, not case 115's
//! `textureCube` pair the gl_cube reference draws with (the gap `CUBEPROG`).
//!
//! Compared with the reference, job by job: the binning list, the render
//! list and the generic tile list packet by packet, the tile allocation
//! size, every draw's shader record and attribute records (as
//! tests/v3d71_backend71_shapes.rs, every field the dump prints except
//! addresses), and for every sampled texture: the TEXTURE_SHADER_STATE
//! record field by field (addresses relative to the texture's buffer), the
//! SAMPLER_STATE record's 24 bytes, and the flag bits of the TMU Config
//! Parameter 0 and 1 words in the fragment uniform stream, whose address
//! bits must be our records' addresses at Mesa's alignments (32 and 8
//! bytes). Buffer addresses are the gap `ADDR` (not compared); the code and
//! the other uniform words are the programs' (the gap `PROGRAM`, checked
//! against uniform71 and texuni71 by backend71tex itself). Every other
//! difference must match one named entry of [`ALLOW`] with its reason, and
//! every entry must still match one: a gap that stops matching fails.
//!
//! The helpers (the GL model, the reference parser, the packet decoder, the
//! comparison) are copied from tests/v3d71_backend71_shapes.rs, the texture
//! dump reader from tests/v3d71_compose3.rs (the existing tests are not
//! edited).
#![allow(dead_code)]

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(clippy::if_same_then_else)]
mod attr71;
#[path = "../src/v3d/backend71.rs"]
mod backend71;
#[path = "../src/v3d/backend71tex.rs"]
mod backend71tex;
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
#[path = "../src/v3d/sampler71.rs"]
mod sampler71;
#[path = "../src/v3d/texfill71.rs"]
mod texfill71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;
#[path = "../src/v3d/texuni71.rs"]
mod texuni71;
#[path = "../src/v3d/tfuplan71.rs"]
mod tfuplan71;
#[path = "../src/v3d/uniform71.rs"]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use backend71::List71;
use backend71tex::{Binding71, DrawTex71, Lists71, TexBuilder71};
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
const G_TEX: u32 = 0x3000000;
const G_ARENA: u32 = 0x500000;
const ARENA_SIZE: u32 = 0x1000;
const fn g_rt(n: u32) -> u32 {
    0x1000000 + n * 0x100000
}

const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLES: u32 = 4;
const GL_TRIANGLE_FAN: u32 = 6;

/* ---- Mesa's programs (tools/v3d/gltest-mesa71*, the .json files, read as data) ---- */

const MESA_TEXTURE: &str = include_str!("../tools/v3d/gltest-mesa71-tex/texture.json");
const MESA_CUBE: &str = include_str!("../tools/v3d/gltest-mesa71-tex/cube.json");
const MESA_CLEAR: &str = include_str!("../tools/v3d/gltest-mesa71/clear.json");

/// The text of stage `s`'s object in v3d_shc's JSON: from its key to the
/// next stage's.
fn stage<'a>(json: &'a str, s: &str) -> &'a str {
    let k = format!("\"{s}\": {{");
    let i = json.find(&k).unwrap_or_else(|| panic!("stage {s}")) + k.len();
    let rest = &json[i..];
    let end = ["\"fs\": {", "\"vs\": {", "\"cs\": {"].iter().filter_map(|k| rest.find(k)).min().unwrap_or(rest.len());
    &rest[..end]
}

/// A scalar value of `obj` (a number, a bool or a quoted hex number).
fn jval<'a>(obj: &'a str, key: &str) -> &'a str {
    let k = format!("\"{key}\": ");
    let i = obj.find(&k).unwrap_or_else(|| panic!("{key}")) + k.len();
    let rest = &obj[i..];
    let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
    rest[..end].trim().trim_matches('"')
}

fn jb(obj: &str, key: &str) -> bool {
    match jval(obj, key) {
        "true" => true,
        "false" => false,
        v => panic!("{key}: {v}"),
    }
}

fn ju(obj: &str, key: &str) -> u32 {
    num(jval(obj, key)) as u32
}

/// The text inside the `[ ]` of a flat list.
fn jlist<'a>(obj: &'a str, key: &str) -> &'a str {
    let k = format!("\"{key}\": [");
    let i = obj.find(&k).unwrap_or_else(|| panic!("{key}")) + k.len();
    &obj[i..i + obj[i..].find(']').unwrap()]
}

/// One of Mesa's programs: as fill71 takes it (its lists masked,
/// `backend71tex::masked`) and its own lists.
struct MesaProg {
    program: Program<'static>,
    lists: [Vec<V3dUniform>; 3],
}

fn mesa_program(json: &str, code: u32) -> MesaProg {
    let (fs, vs, cs) = (stage(json, "fs"), stage(json, "vs"), stage(json, "cs"));
    let list = |obj: &str| -> Vec<uniform71::Word> {
        jlist(obj, "uniforms")
            .split('{')
            .skip(1)
            .map(|u| uniform71::Word { contents: ju(u, "contents"), data: ju(u, "data") })
            .collect()
    };
    let reads = |obj: &str| -> [u8; 16] {
        let v: Vec<u8> = jlist(obj, "vattr_sizes").split(',').map(|x| x.trim().parse().unwrap()).collect();
        assert_eq!(v.len(), 16);
        core::array::from_fn(|i| v[i])
    };
    let vdesc = |obj: &str, at: u32| record71::VertexDesc {
        prog: record71::ProgramDesc {
            code: at,
            uniforms: 0,
            threads: ju(obj, "threads"),
            single_seg: jb(obj, "single_seg"),
            propagate_nans: false,
        },
        vpm_input_size: ju(obj, "vpm_input_size"),
        vpm_output_size: ju(obj, "vpm_output_size"),
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: jb(obj, "uses_vid"),
        uses_iid: jb(obj, "uses_iid"),
        uses_base_instance: jb(obj, "uses_biid"),
        uses_base_vertex: false,
        uses_draw_index: false,
    };
    let mut desc = record71::DrawDesc::default();
    desc.fs.prog = record71::ProgramDesc {
        code,
        uniforms: 0,
        threads: ju(fs, "threads"),
        single_seg: jb(fs, "single_seg"),
        propagate_nans: false,
    };
    desc.fs.writes_z = jb(fs, "writes_z");
    desc.fs.disable_ez = jb(fs, "disable_ez");
    desc.fs.num_varyings = ju(fs, "num_inputs");
    desc.fs.uses_center_w = jb(fs, "uses_center_w");
    desc.fs.uses_pid = jb(fs, "uses_pid");
    desc.fs.lock_scoreboard_on_first_thrsw = jb(fs, "lock_scoreboard_on_first_thrsw");
    desc.fs.uses_implicit_point_line_varyings = jb(fs, "uses_implicit_point_line_varyings");
    desc.vs = vdesc(vs, code + 0x100);
    desc.cs = vdesc(cs, code + 0x200);
    desc.point_size = jb(vs, "writes_psiz");
    let own = [list(fs), list(vs), list(cs)];
    let masked =
        |l: &[uniform71::Word]| -> &'static [uniform71::Word] { Box::leak(backend71tex::masked(l).into_boxed_slice()) };
    let program = Program {
        desc,
        cs_reads: reads(cs),
        vs_reads: reads(vs),
        cs_vcm_cache_size: ju(cs, "vcm_cache_size"),
        vs_vcm_cache_size: ju(vs, "vcm_cache_size"),
        separate_segments: jb(vs, "separate_segments") || jb(cs, "separate_segments"),
        varyings: fill71::VaryingFlags {
            flat: [ju(fs, "flat_shade_flags"), 0],
            noperspective: [ju(fs, "noperspective_flags"), 0],
            centroid: [ju(fs, "centroid_flags"), 0],
        },
        discards: jb(fs, "uses_discard"),
        fs_uniforms: masked(&own[0]),
        vs_uniforms: masked(&own[1]),
        cs_uniforms: masked(&own[2]),
    };
    let ffi = |l: &Vec<uniform71::Word>| l.iter().map(|w| V3dUniform { contents: w.contents, data: w.data }).collect();
    MesaProg { program, lists: [ffi(&own[0]), ffi(&own[1]), ffi(&own[2])] }
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

/// gl_test's 24-byte puv vertex: pos4 +0, uv2 +16 (gl_shapes.py puv_quad).
static PUV_ATTRS: [attr71::Attr; 2] = [float_attr(4, 24, G_VTX), float_attr(2, 24, G_VTX + 16)];
/// The cube shape enables only a_pos of the same vertex.
static P_ATTRS: [attr71::Attr; 1] = [float_attr(4, 24, G_VTX)];
/// The clear quad (not drawn by these shapes: their clears are whole).
const QUAD_ATTR: attr71::Attr = float_attr(4, 16, G_VTX + 0x100);

struct Progs {
    texture: MesaProg,
    cube: MesaProg,
    clear: MesaProg,
}

fn progs() -> &'static Progs {
    static P: OnceLock<Progs> = OnceLock::new();
    P.get_or_init(|| Progs {
        texture: mesa_program(MESA_TEXTURE, G_CODE),
        cube: mesa_program(MESA_CUBE, G_CODE + 0x300),
        clear: mesa_program(MESA_CLEAR, G_CODE + 0x600),
    })
}

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &progs().clear.program, attr: QUAD_ATTR, dummy_addr: G_VTX }
}

fn table() -> backend71::Table71 {
    // HUB_IDENT1 and HUB_IDENT3 of the board (tools/v3d/pi5/README.md)
    backend71::init(0x0008_1117, 0x0002_0a10).expect("the Pi 5's identity")
}

// ---- the C builder in a child process ---------------------------------------

const HARNESS_C: &str = include_str!("v3d71_backend71_tex_harness.c");

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
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("v3d71_backend71_tex-{}", std::process::id()));
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
            .arg(format!("{ROOT}/driver/src/v3d/v3d_tex71.c"))
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

/// `TexBuilder71` over the harness: the lists and the arena at fixed
/// addresses.
struct Harness {
    gpu: [u32; 3],
    lists: [Vec<u8>; 3],
    arena: Vec<u8>,
    arena_size: u32,
}

impl Harness {
    fn new() -> Harness {
        Harness { gpu: [G_BCL, G_RCL, G_IND], lists: Default::default(), arena: vec![], arena_size: ARENA_SIZE }
    }
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

impl TexBuilder71 for Harness {
    fn gpu(&self, l: List71) -> u32 {
        self.gpu[li(l)]
    }

    fn arena(&self) -> (u32, u32) {
        (G_ARENA, self.arena_size)
    }

    fn build(
        &mut self,
        target: &ffi71::V3dJob71Target,
        draws: &[backend71tex::BuildDraw71<'_>],
        clear: &ffi71::V3dJob71Clear,
        records: &backend71tex::Records71,
    ) -> Result<backend71::Built71, backend71tex::TexBuildError71> {
        use backend71::BuildError71 as E;
        use backend71tex::TexBuildError71 as TE;
        let mut inp = vec![];
        let w = |v: &mut Vec<u8>, x: u32| v.extend(x.to_ne_bytes());
        w(&mut inp, 0x3731_5442);
        w(&mut inp, draws.len() as u32);
        inp.extend(raw(target));
        inp.extend(raw(clear));
        for x in [self.gpu[0], self.gpu[1], self.gpu[2], CL_SIZE, backend71::PAGE_SIZE] {
            w(&mut inp, x);
        }
        for x in [G_ARENA, self.arena_size, records.tex.len() as u32, records.samplers.len() as u32] {
            w(&mut inp, x);
        }
        for (a, s) in &records.tex {
            w(&mut inp, *a);
            inp.extend(raw(s));
        }
        for (a, s) in &records.samplers {
            w(&mut inp, *a);
            inp.extend(raw(s));
        }
        for bd in draws {
            let d = &bd.draw;
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
            for n in [d.attrs.len(), bd.fs_tmu.len(), bd.vs_tmu.len()] {
                w(&mut inp, n as u32);
            }
            inp.extend(raw_slice(d.fs_uniforms));
            inp.extend(raw_slice(d.vs_uniforms));
            inp.extend(raw_slice(d.cs_uniforms));
            inp.extend(raw_slice(d.fs_gl));
            inp.extend(raw_slice(d.vs_gl));
            inp.extend(raw_slice(d.attrs));
            inp.extend(raw_slice(bd.fs_tmu));
            inp.extend(raw_slice(bd.vs_tmu));
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
        let used = r.u32() as usize;
        self.arena = r.take(used).to_vec();
        match status {
            0 => Ok(backend71::Built71 {
                tiling,
                layouts,
                tile_list: (ls, le),
                tile_alloc_size: qms,
                tile_state_size: qts,
            }),
            1 => Err(TE::Job(E::Begin)),
            2 => Err(TE::Job(E::Draw { index: at, code })),
            3 => Err(TE::Job(E::End)),
            4 => Err(TE::TexState { index: at }),
            5 => Err(TE::Sampler { index: at }),
            _ => Err(TE::Outside),
        }
    }

    fn bytes(&self, l: List71) -> &[u8] {
        &self.lists[li(l)]
    }

    fn arena_bytes(&self) -> &[u8] {
        &self.arena
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

    /// glDrawArrays(GL_TRIANGLES, 0, 6) with `program`.
    fn draw(
        &mut self,
        program: &'static Program<'static>,
        attrs: &'static [attr71::Attr],
        vs_words: &'static [u32],
        fs_words: &'static [u32],
    ) {
        let mut st = self.st;
        st.scissor = self.scissor_on.then_some(self.scissor);
        self.ops.push(Op::Draw(Box::new(DrawIn {
            state: st,
            line_width: 1.0,
            blend_color: [0.0; 4],
            program,
            attrs,
            dummy_addr: G_VTX,
            prim: prim71::Draw::arrays(GL_TRIANGLES, 0, 6),
            vs_words,
            fs_words,
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

/* ---- the texture shapes: gl_shapes.py's calls (tools/v3d/pi5/src/gl_shapes.py:690-767) ---- */

const GL_RGBA: u32 = 0x1908;
const GL_RGBA8: u32 = 0x8058;
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RGB565: u32 = 0x8D62;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;

use sampler71::{Filter, MipFilter, Wrap};
use texture71::{GlTex71, Target as TexTarget};

/// A square texture with its buffer at `G_TEX`, on the Pi 5's 7.1.10.
fn gl_tex(target: TexTarget, internal_format: u32, ty: u32, size: u32, levels: u32) -> GlTex71 {
    GlTex71 {
        target,
        internal_format,
        ty,
        width: size,
        height: size,
        levels,
        base_level: 0,
        max_level: texture71::GL_MAX_LEVEL_DEFAULT,
        swizzle: texture71::GL_SWIZZLE_IDENTITY,
        srgb_decode: true,
        bo_address: G_TEX,
        ver: 71,
        iprev: 10,
    }
}

/// GL's sampler state of a shape: wrap R stays GL's default REPEAT and
/// `cube` is not set (texfill71 takes both from the texture's target).
fn smp(min: Filter, mag: Filter, mip: MipFilter, wrap: Wrap) -> sampler71::Input {
    sampler71::Input {
        min_filter: min,
        mag_filter: mag,
        mip_filter: mip,
        wrap_s: wrap,
        wrap_t: wrap,
        ..sampler71::Input::default_2d()
    }
}

/// What a draw samples on unit 0 of its fragment stage, and its program.
#[derive(Clone)]
struct Bound {
    prog: &'static MesaProg,
    filled: texfill71::Filled71,
}

/// The GL model with the texture each draw samples: per job, per draw of
/// the job (a clear quad has none).
struct TexGl {
    g: Gl,
    pending: Vec<Bound>,
    bound: Vec<Vec<Option<Bound>>>,
}

impl TexGl {
    fn new() -> TexGl {
        TexGl { g: Gl::new(Fb::window()), pending: vec![], bound: vec![] }
    }

    /// glDrawArrays(GL_TRIANGLES, 0, 6) with `prog`, `tex` bound on unit 0
    /// with the sampler state `s`.
    fn draw(
        &mut self,
        prog: &'static MesaProg,
        attrs: &'static [attr71::Attr],
        tex: &GlTex71,
        s: &sampler71::Input,
        vs_words: Vec<u32>,
        fs_words: Vec<u32>,
    ) {
        let filled = texfill71::fill(tex, s).unwrap_or_else(|e| panic!("texfill71: {e}"));
        self.pending.push(Bound { prog, filled });
        self.g.draw(
            &prog.program,
            attrs,
            Box::leak(vs_words.into_boxed_slice()),
            Box::leak(fs_words.into_boxed_slice()),
        );
    }

    fn finish(&mut self) {
        let before = self.g.jobs.len();
        self.g.finish();
        if let Some(r) = &self.g.refusal {
            panic!("fill71 refused: {r}");
        }
        let mut pending = std::mem::take(&mut self.pending).into_iter();
        for job in &self.g.jobs[before..] {
            self.bound.push(job.draws.iter().map(|d| if d.is_clear { None } else { pending.next() }).collect());
        }
    }

    /// gl_shapes.py draw_tex: a clear, the quad, glFinish.
    fn draw_tex(&mut self, clear: [f32; 4], tex: &GlTex71, s: &sampler71::Input) {
        self.g.clear_color = clear;
        self.g.clear(COLOR);
        self.draw(&progs().texture, &PUV_ATTRS, tex, s, vec![], vec![]);
        self.finish();
    }
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn shape_tex2d(t: &mut TexGl) {
    let tex = gl_tex(TexTarget::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64, 1);
    for f in [Filter::Nearest, Filter::Linear] {
        t.draw_tex(BLACK, &tex, &smp(f, f, MipFilter::None, Wrap::Repeat));
    }
}

fn shape_texfmt(t: &mut TexGl) {
    let s = smp(Filter::Nearest, Filter::Nearest, MipFilter::None, Wrap::Repeat);
    t.draw_tex(BLACK, &gl_tex(TexTarget::Tex2d, GL_SRGB8_ALPHA8, GL_UNSIGNED_BYTE, 64, 1), &s);
    t.draw_tex(BLACK, &gl_tex(TexTarget::Tex2d, GL_RGB565, GL_UNSIGNED_SHORT_5_6_5, 64, 1), &s);
}

/// The render job of gl_mipgen: a 7-level texture (levels 1 to 6 are the
/// TFU's, tfuplan71's layout) sampled with LINEAR_MIPMAP_LINEAR.
fn shape_mipgen(t: &mut TexGl) {
    let tex = gl_tex(TexTarget::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64, 7);
    t.draw_tex(BLACK, &tex, &smp(Filter::Linear, Filter::Linear, MipFilter::Linear, Wrap::Repeat));
}

/// tools/v3d/tex_cube_case.h tcc_faces' major axes, GL's face order.
const CUBE_MAJOR: [[f32; 3]; 6] =
    [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];

/// Six 4x4 GL_RGBA faces, CLAMP_TO_EDGE, a 32x32 viewport; per face NEAREST
/// then LINEAR. The program is Mesa's `cube` (its `axis` uniform in both
/// stages' blocks holds the face's major axis), not the reference's pair.
fn shape_cube(t: &mut TexGl) {
    let tex = gl_tex(TexTarget::Cube, GL_RGBA, GL_UNSIGNED_BYTE, 4, 1);
    t.g.st.viewport = [0.0, 0.0, 32.0, 32.0];
    for i in 0..12 {
        let m = CUBE_MAJOR[i / 2];
        let axis: Vec<u32> = [m[0], m[1], m[2], 0.0].iter().map(|v| v.to_bits()).collect();
        let f = if i & 1 == 1 { Filter::Linear } else { Filter::Nearest };
        t.g.clear_color = [0.0; 4];
        t.g.clear(COLOR);
        let vs_words = [axis.clone(), axis.clone()].concat();
        t.draw(&progs().cube, &P_ATTRS, &tex, &smp(f, f, MipFilter::None, Wrap::ClampToEdge), vs_words, axis);
        t.finish();
    }
}

const SHAPES: [&str; 4] = ["tex2d", "texfmt", "cube", "mipgen"];

fn run_shape(name: &str) -> TexGl {
    let mut t = TexGl::new();
    match name {
        "tex2d" => shape_tex2d(&mut t),
        "texfmt" => shape_texfmt(&mut t),
        "cube" => shape_cube(&mut t),
        "mipgen" => shape_mipgen(&mut t),
        _ => unreachable!(),
    }
    t
}

// ---- the texture records: the dump's TMU Config pair ------------------------------

struct TexRec {
    p0: u32,
    p1: u32,
    tex: [u32; 6],
    smp: [u32; 6],
}

struct TexDump {
    /// per job: the (P0, P1) pairs the fragment shaders' uniform streams hold
    jobs: Vec<Vec<TexRec>>,
    /// every `resource_0x...` buffer: start address and dumped extent
    resources: Vec<(u32, u32)>,
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// texstate71.py's reading rule: the words of each fragment shader's uniform
/// stream, each pair (P0, P1) whose P0 is a 32-byte aligned address and P1 an
/// 8-byte aligned one, both with six dumped words behind them.
fn parse_texdump(file: &str) -> TexDump {
    let text = read(&format!("tools/v3d/pi5/{file}.clif"));
    let mut jobs = Vec::new();
    let mut mem: BTreeMap<u32, u32> = BTreeMap::new();
    let mut fs: Vec<u32> = Vec::new();
    let mut unif: Vec<u32> = Vec::new();
    let mut extent: BTreeMap<String, u32> = BTreeMap::new();
    let mut addr: Option<u32> = None;
    let mut cur: Option<(String, u32)> = None;
    let finish = |mem: &BTreeMap<u32, u32>, fs: &[u32], unif: &[u32]| -> Vec<TexRec> {
        let mut recs = Vec::new();
        for &f in fs {
            let end = unif.iter().copied().filter(|&a| a > f).min();
            let mut stream = Vec::new();
            let mut a = f;
            while let Some(w) = mem.get(&a) {
                if end.is_some_and(|e| a >= e) {
                    break;
                }
                stream.push(*w);
                a += 4;
            }
            for p in stream.windows(2) {
                let (ta, sa) = (p[0] & !0xf, p[1] & !0x7);
                let have = |b: u32| (0..6).all(|k| mem.contains_key(&(b + 4 * k)));
                if ta % 32 != 0 || !have(ta) || !have(sa) {
                    continue;
                }
                recs.push(TexRec {
                    p0: p[0],
                    p1: p[1],
                    tex: std::array::from_fn(|k| mem[&(ta + 4 * k as u32)]),
                    smp: std::array::from_fn(|k| mem[&(sa + 4 * k as u32)]),
                });
            }
        }
        recs
    };
    for raw in text.lines() {
        let Some(line) = raw.strip_prefix("MESA: info: ") else { continue };
        if line == "@wait_render_all_cores" {
            jobs.push(finish(&mem, &fs, &unif));
            mem.clear();
            fs.clear();
            unif.clear();
            continue;
        }
        if let Some(r) = line.strip_prefix("@format binary /* [") {
            let (name, off) = r.split_once('+').unwrap();
            let off = hex(off.split(']').next().unwrap());
            let base = hex(name.rsplit('_').next().unwrap());
            addr = Some(base + off);
            cur = Some((name.to_string(), off));
            continue;
        }
        if let Some(r) = line.strip_prefix("@format blank ") {
            let r = &r[r.find('[').unwrap() + 1..r.find(']').unwrap()];
            let (name, range) = r.split_once('+').unwrap();
            let (lo, hi) = range.split_once("..").unwrap();
            let base = hex(name.rsplit('_').next().unwrap());
            let (lo, hi) = (hex(lo), hex(hi));
            for a in ((base + lo) & !3..=base + hi).step_by(4) {
                mem.entry(a).or_insert(0);
            }
            let e = extent.entry(name.to_string()).or_insert(0);
            *e = (*e).max(hi + 1);
            addr = None;
            cur = None;
            continue;
        }
        if line.starts_with('@') {
            addr = None;
            cur = None;
            continue;
        }
        let t = line.trim();
        if let Some(a) = addr.as_mut() {
            if !t.is_empty() && t.split(' ').all(|w| w.len() == 10 && w.starts_with("0x")) {
                let (name, off) = cur.as_mut().unwrap();
                for w in t.split(' ') {
                    mem.insert(*a, hex(w));
                    *a += 4;
                    *off += 4;
                }
                let e = extent.entry(name.clone()).or_insert(0);
                *e = (*e).max(*off);
                continue;
            }
        }
        for which in ["Fragment", "Vertex", "Coordinate"] {
            let key = format!("/* {which} Shader Uniforms Address: */ [");
            if let Some(i) = t.find(&key) {
                let r = &t[i + key.len()..];
                let v = &r[r.find("/* ").unwrap() + 3..];
                let a = hex(v.split(' ').next().unwrap());
                unif.push(a);
                if which == "Fragment" {
                    fs.push(a);
                }
            }
        }
    }
    let resources = extent
        .iter()
        .filter(|(n, _)| n.starts_with("resource_"))
        .map(|(n, e)| (hex(n.rsplit('_').next().unwrap()), *e))
        .collect();
    TexDump { jobs, resources }
}

/// (name, start, size) of every field of the 7.1 Texture Shader State, from
/// the vendored packet XML.
fn tex_fields() -> Vec<(String, u32, u32)> {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let open = "<struct name=\"Texture Shader State\" min_ver=\"71\">";
    let s = xml.find(open).expect("7.1 Texture Shader State");
    let block = &xml[s..s + xml[s..].find("</struct>").unwrap()];
    let attr = |tag: &str, a: &str| -> String {
        let k = format!(" {a}=\"");
        let j = tag.find(&k).unwrap_or_else(|| panic!("{a} in {tag}")) + k.len();
        tag[j..j + tag[j..].find('"').unwrap()].to_string()
    };
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = block[at..].find("<field ") {
        let t0 = at + i;
        let tag = &block[t0..t0 + block[t0..].find('>').unwrap()];
        out.push((attr(tag, "name"), attr(tag, "start").parse().unwrap(), attr(tag, "size").parse().unwrap()));
        at = t0 + tag.len();
    }
    out
}

fn wbits(words: &[u32; 6], start: u32, size: u32) -> u64 {
    let mut v = 0u64;
    for b in 0..size {
        let bit = start + b;
        v |= u64::from(words[(bit / 32) as usize] >> (bit % 32) & 1) << b;
    }
    v
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
/// A named difference: `(shape glob, difference glob, id, reason)`.
/// `EZ` is v3d_job71's own difference from Mesa's jobs (as in
/// tests/v3d71_backend71_shapes.rs); the three `CUBEPROG` entries are the
/// program this test draws gl_cube with. The texture records, the sampler
/// records and the TMU Config flag bits have no entry: they match.
#[rustfmt::skip]
const ALLOW: [(&str, &str, &str, &str); 4] = [
    ("*", "* job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", "EZ", "v3d_job71 never enables early Z (job71.sh gl.allow EZ)"),
    ("cube", "cube job * draw 0 SHADREC.Number of varyings in Fragment Shader: 2 != 3", "CUBEPROG", "the program is Mesa's offline `cube` (gl_test case 41, ES 3.00: a vec2 varying and an `axis` uniform), the reference draws case 115's textureCube pair with a vec3 varying; tools/v3d/gltest-mesa71-tex has no texcube program"),
    ("cube", "cube job * draw 0 ATTR 0.Number of values read by Coordinate shader: 2 != 4", "CUBEPROG", "the same: case 41's coordinate shader reads pos.xy, case 115's writes gl_Position = a_pos"),
    ("cube", "cube job * draw 0 ATTR 0.Number of values read by Vertex shader: 2 != 4", "CUBEPROG", "the same, for the vertex shader"),
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

/// One sampled texture of a draw as backend71tex assembled it: the TMU
/// Config words of the fragment stream and the records they point at.
struct OursTex {
    p0: u32,
    p1: u32,
    tex_addr: u32,
    smp_addr: u32,
    tex: [u32; 6],
    smp: [u32; 6],
}

/// One job as backend71tex assembled it.
struct Ours {
    bcl: Vec<u8>,
    rcl: Vec<u8>,
    tiles: Vec<u8>,
    qms: u32,
    records: Vec<Pk>,
    attrs: Vec<Vec<Pk>>,
    tex: Vec<OursTex>,
    /// (texture records, sampler records) of the job's arena.
    arena: (usize, usize),
}

fn le_words<const N: usize>(b: &[u8]) -> [u32; N] {
    core::array::from_fn(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()))
}

/// The bindings of a job's draws as `assemble_filled` takes them.
fn assemble_job(
    job: &FilledJob,
    fb: &Framebuffer<'_>,
    bound: &[Option<Bound>],
    h: &mut Harness,
) -> Result<backend71tex::AssembledTexJob71, backend71::Refusal> {
    let binds: Vec<Vec<Binding71<'_>>> =
        bound.iter().map(|b| b.iter().map(|b| Binding71 { unit: 0, filled: &b.filled }).collect()).collect();
    let tex: Vec<DrawTex71<'_>> = bound
        .iter()
        .zip(&binds)
        .map(|(b, v)| DrawTex71 {
            fs: v,
            vs: &[],
            lists: b.as_ref().map(|b| Lists71 { fs: &b.prog.lists[0], vs: &b.prog.lists[1], cs: &b.prog.lists[2] }),
        })
        .collect();
    backend71tex::assemble_filled(&table(), fb, job, &tex, h)
}

fn assemble_shape(shape: &str) -> Vec<Ours> {
    let g = run_shape(shape);
    let mut out = vec![];
    for (k, (job, (colors, ds))) in g.g.jobs.iter().zip(&g.g.fbs).enumerate() {
        let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors, ds: *ds };
        let mut h = Harness::new();
        let a = assemble_job(job, &fb, &g.bound[k], &mut h)
            .unwrap_or_else(|e| panic!("gl_{shape} job {k}: backend71tex refused: {e}"));
        let ind = h.bytes(List71::Ind);
        let at = |addr: u32, n: usize| &ind[(addr - G_IND) as usize..(addr - G_IND) as usize + n];
        let (ls, le) = a.built.tile_list;
        let (mut records, mut attrs, mut tex) = (vec![], vec![], vec![]);
        for ((p, lo), b) in a.draws.iter().zip(&a.built.layouts).zip(&g.bound[k]) {
            records.push(decode_record(at(lo.record, record71::RECORD_SIZE)));
            attrs.push(
                (0..p.plan.attrs.count)
                    .map(|i| decode_attr(at(lo.attr_records + (i * attr71::RECORD_BYTES) as u32, attr71::RECORD_BYTES)))
                    .collect(),
            );
            for t in &p.fs_tmu {
                let list = &b.as_ref().unwrap().prog.lists[0];
                let word = |contents: u32| {
                    let i = list.iter().position(|u| u.contents == contents && u.data >> 24 == t.unit).unwrap();
                    le_words::<1>(at(lo.fs_uniforms + 4 * i as u32, 4))[0]
                };
                tex.push(OursTex {
                    p0: word(texuni71::TMU_CONFIG_P0),
                    p1: word(texuni71::TMU_CONFIG_P1),
                    tex_addr: t.tex_state,
                    smp_addr: t.sampler,
                    tex: le_words(backend71tex::record_at(&h, t.tex_state, 24).unwrap()),
                    smp: le_words(backend71tex::record_at(&h, t.sampler, 24).unwrap()),
                });
            }
        }
        out.push(Ours {
            bcl: h.bytes(List71::Bcl).to_vec(),
            rcl: h.bytes(List71::Rcl).to_vec(),
            tiles: at(ls, (le - ls) as usize).to_vec(),
            qms: a.built.tile_alloc_size,
            records,
            attrs,
            tex,
            arena: (a.records.tex.len(), a.records.samplers.len()),
        });
    }
    out
}

/// A sampled texture against the dump's: the texture record field by field
/// (its three buffer addresses relative to the texture's buffer), the
/// sampler record's words and the flag bits of the two TMU Config words.
fn tex_cmp(
    w: &str,
    a: &OursTex,
    b: &TexRec,
    resources: &[(u32, u32)],
    fields: &[(String, u32, u32)],
    diffs: &mut Vec<String>,
) {
    // the words address our records, at Mesa's alignments
    assert_eq!(a.p0 & !0xf, a.tex_addr, "{w}: P0's address is the texture record's");
    assert_eq!(a.p1 & !0x7, a.smp_addr, "{w}: P1's address is the sampler record's");
    assert_eq!(a.tex_addr % 32, 0, "{w}: the texture record is 32-byte aligned");
    assert_eq!(a.smp_addr % 8, 0, "{w}: the sampler record is 8-byte aligned");
    assert_eq!((b.p0 & !0xf) % 32, 0, "{w}: the reference's texture record");
    if a.p0 & 0xf != b.p0 & 0xf {
        diffs.push(format!("{w} TMU_P0.Return words of texture data: {} != {}", a.p0 & 0xf, b.p0 & 0xf));
    }
    if a.p1 & 0x7 != b.p1 & 0x7 {
        diffs.push(format!("{w} TMU_P1.flags: {} != {}", a.p1 & 0x7, b.p1 & 0x7));
    }
    let ref_base = (wbits(&b.tex, 0, 32) as u32) & !63;
    let ref_bo = resources.iter().map(|r| r.0).filter(|r| *r <= ref_base).max().unwrap();
    for (name, start, size) in fields {
        let (x, y) = (wbits(&a.tex, *start, *size), wbits(&b.tex, *start, *size));
        let (x, y) = match name.as_str() {
            // the address without the flag bits it shares its low 6 bits with
            "Texture base pointer" => ((x & !63) - u64::from(G_TEX), (y & !63) - u64::from(ref_bo)),
            "texture_base pointer_Cr" | "texture base pointer Cb" => {
                ((x << 6) - u64::from(G_TEX), (y << 6) - u64::from(ref_bo))
            }
            _ => (x, y),
        };
        if x != y {
            diffs.push(format!("{w} TEXSTATE.{name}: {x} != {y}"));
        }
    }
    for i in 0..6 {
        if a.smp[i] != b.smp[i] {
            diffs.push(format!("{w} SAMPLER word {i}: {:#010x} != {:#010x}", a.smp[i], b.smp[i]));
        }
    }
}

/// Every difference of a shape's jobs from the reference, gl_cmp.py's text.
/// Returns the differences, the reference packets, the records and the
/// sampled textures compared.
fn shape_diffs(shape: &str, defs: &[PacketDef]) -> (Vec<String>, usize, usize, usize) {
    let ours = assemble_shape(shape);
    let refs = ref_jobs(shape);
    let dump = parse_texdump(&format!("gl_{shape}"));
    let fields = tex_fields();
    assert_eq!(ours.len(), refs.len(), "gl_{shape}: jobs");
    assert_eq!(dump.jobs.len(), refs.len(), "gl_{shape}: the texture dump's jobs");
    let mut diffs = vec![];
    let (mut packets, mut draws, mut textures) = (0, 0, 0);
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
        let rt = &dump.jobs[k];
        if o.tex.len() != rt.len() {
            diffs.push(format!("{w} sampled textures: {} != {}", o.tex.len(), rt.len()));
        }
        for (n, (a, b)) in o.tex.iter().zip(rt).enumerate() {
            tex_cmp(&format!("{w} texture {n}"), a, b, &dump.resources, &fields, &mut diffs);
            textures += 1;
        }
    }
    (diffs, packets, draws, textures)
}

#[test]
fn every_shape_assembles_and_matches_the_reference_or_a_named_gap() {
    let defs = packet_defs();
    let mut failures = vec![];
    let mut named: BTreeMap<&str, usize> = BTreeMap::new();
    let mut used = vec![0usize; ALLOW.len()];
    let mut all_textures = 0;
    for shape in SHAPES {
        let (diffs, packets, draws, textures) = shape_diffs(shape, &defs);
        all_textures += textures;
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
        println!(
            "gl_{shape}: {packets} reference packets, {draws} records, {textures} textures, {left} differences, {here} named"
        );
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
    // tex2d 2, texfmt 2, cube 12, mipgen 1: one sampled texture a job
    assert_eq!(all_textures, 17, "the sampled textures compared");
    assert!(failures.is_empty(), "{} difference(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn the_jobs_draws_and_records_are_the_references_counts() {
    for (shape, jobs) in [("tex2d", 2), ("texfmt", 2), ("cube", 12), ("mipgen", 1)] {
        let ours = assemble_shape(shape);
        let refs = ref_jobs(shape);
        assert_eq!((ours.len(), refs.len()), (jobs, jobs), "gl_{shape}: jobs");
        for (k, (o, r)) in ours.iter().zip(&refs).enumerate() {
            let draws = r.bcl.iter().filter(|p| DRAWS.contains(&p.name.as_str())).count();
            assert_eq!(o.records.len(), draws, "gl_{shape} job {k}: draws");
            // one texture record and one sampler record a job; the sampler
            // sits behind the texture record's 24 bytes, 8-byte aligned
            assert_eq!(o.arena, (1, 1), "gl_{shape} job {k}: records");
            assert_eq!((o.tex[0].tex_addr, o.tex[0].smp_addr), (G_ARENA, G_ARENA + 24), "gl_{shape} job {k}");
        }
    }
}

/* ---- the level layout, the records' sharing and the refusals ----------------------- */

#[test]
fn a_mipmapped_texture_is_sampled_from_tfuplan71s_levels() {
    // the 7-level texture of gl_mipgen: the record's base is level 0, which
    // sits above the smaller levels the TFU writes (tfuplan71's offsets)
    let tex = gl_tex(TexTarget::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64, 7);
    let f = texfill71::fill(&tex, &smp(Filter::Linear, Filter::Linear, MipFilter::Linear, Wrap::Repeat)).unwrap();
    let job = tfuplan71::plan(&tfuplan71::Args {
        width: 64,
        height: 64,
        base_level: 0,
        last_level: 6,
        src_level: 0,
        ttype: f.state.type_,
        cpp: f.tex.layout.cpp,
        src_tiling: tfuplan71::Tiling::UifNoXor,
        dst_tiling: tfuplan71::Tiling::UifNoXor,
        layer: 0,
        layer_count: 1,
    })
    .unwrap()
    .remove(0);
    assert_eq!((f.state.base_level, f.state.max_level), (0, 6));
    assert_eq!(u64::from(f.state.base), u64::from(G_TEX) + job.base_offset, "the record's base is the TFU's input");
    assert_eq!(job.levels.len(), 6);
    for l in &job.levels {
        let s = &f.tex.layout.slices[l.level as usize];
        assert_eq!((u64::from(s.offset), u64::from(s.size)), (l.offset, l.bytes), "level {}", l.level);
        assert!(l.offset < job.base_offset, "level {} sits below the base level", l.level);
    }
}

/// One job of two textured draws; `edit` changes the second draw's sampler
/// input and texture.
fn two_draws(
    edit: impl FnOnce(&mut GlTex71, &mut sampler71::Input),
) -> (FilledJob, Vec<ColorBuffer>, Option<DsBuffer>, Vec<Option<Bound>>) {
    let mut t = TexGl::new();
    let mut tex = gl_tex(TexTarget::Tex2d, GL_RGBA8, GL_UNSIGNED_BYTE, 64, 1);
    let mut s = smp(Filter::Nearest, Filter::Nearest, MipFilter::None, Wrap::Repeat);
    t.g.clear_color = BLACK;
    t.g.clear(COLOR);
    t.draw(&progs().texture, &PUV_ATTRS, &tex, &s, vec![], vec![]);
    edit(&mut tex, &mut s);
    t.draw(&progs().texture, &PUV_ATTRS, &tex, &s, vec![], vec![]);
    t.finish();
    assert_eq!(t.g.jobs.len(), 1);
    let (colors, ds) = t.g.fbs.remove(0);
    (t.g.jobs.remove(0), colors, ds, t.bound.remove(0))
}

fn run_two(
    edit: impl FnOnce(&mut GlTex71, &mut sampler71::Input),
    arena_size: u32,
) -> Result<backend71tex::AssembledTexJob71, backend71::Refusal> {
    let (job, colors, ds, bound) = two_draws(edit);
    let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds };
    let mut h = Harness::new();
    h.arena_size = arena_size;
    assemble_job(&job, &fb, &bound, &mut h)
}

#[test]
fn equal_textures_and_samplers_share_a_record_in_a_job() {
    let a = run_two(|_, _| {}, ARENA_SIZE).unwrap();
    assert_eq!((a.records.tex.len(), a.records.samplers.len()), (1, 1));
    assert_eq!(a.draws[0].fs_tmu, a.draws[1].fs_tmu);

    // another filter: the texture record is shared, the sampler is new
    let a = run_two(|_, s| s.mag_filter = Filter::Linear, ARENA_SIZE).unwrap();
    assert_eq!((a.records.tex.len(), a.records.samplers.len()), (1, 2));
    let (d0, d1) = (&a.draws[0].fs_tmu[0], &a.draws[1].fs_tmu[0]);
    assert_eq!(d0.tex_state, d1.tex_state);
    assert_eq!((d0.sampler, d1.sampler), (G_ARENA + 24, G_ARENA + 48));

    // another texture: a second texture record, 32-byte aligned
    let a = run_two(|t, _| t.internal_format = GL_SRGB8_ALPHA8, ARENA_SIZE).unwrap();
    assert_eq!((a.records.tex.len(), a.records.samplers.len()), (2, 1));
    assert_eq!(a.draws[1].fs_tmu[0].tex_state, G_ARENA + 64);
    // the size words of a binding are the base level's
    assert_eq!((d0.width, d0.height, d0.depth, d0.array_size), (64, 64, 1, 1));
}

#[test]
fn the_refusals_are_named() {
    // the arena holds the texture record but not the sampler
    let e = run_two(|_, _| {}, 24).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71tex", backend71tex::REF_ARENA_FULL));

    let (job, colors, ds, bound) = two_draws(|_, _| {});
    let fb = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds };
    let lists = |b: &Bound| Lists71 { fs: &b.prog.lists[0], vs: &b.prog.lists[1], cs: &b.prog.lists[2] };
    let b = bound[0].as_ref().unwrap();
    let bind = [Binding71 { unit: 0, filled: &b.filled }];
    let good = DrawTex71 { fs: &bind, vs: &[], lists: Some(lists(b)) };
    let run = |tex: &[DrawTex71<'_>]| backend71tex::assemble_filled(&table(), &fb, &job, tex, &mut Harness::new());
    run(&[good, good]).expect("the two draws");

    // one entry per draw
    let e = run(&[good]).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71tex", backend71tex::REF_BINDINGS));
    // a TMU config uniform without its unit bound: texuni71's refusal
    let e = run(&[good, DrawTex71 { fs: &[], ..good }]).unwrap_err();
    assert_eq!(e.to_string(), "texuni71: no texture unit 0 bound for the uniform");
    // a unit bound twice
    let twice = [bind[0], bind[0]];
    let e = run(&[good, DrawTex71 { fs: &twice, ..good }]).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71tex", backend71tex::REF_UNIT_TWICE));
    // the masked lists left in place: fill71's constants are not the
    // program's texture words, and uniform71 would write 0 for them, so the
    // program's own lists must come with the draw
    let other = lists(&Bound { prog: &progs().cube, filled: b.filled.clone() });
    let e = run(&[good, DrawTex71 { lists: Some(other), ..good }]).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71tex", backend71tex::REF_LISTS));
    // fill71's masked lists handed on without the program's own: the stream
    // would hold 0 where the TMU words belong, so the binding nothing reads
    // is refused
    let e = run(&[good, DrawTex71 { lists: None, ..good }]).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("backend71tex", backend71tex::REF_UNREAD));
    // fill71 itself still refuses an unmasked texture word (uniform71's
    // refusal): why the lists are masked for it
    let g = {
        let mut g = Gl::new(Fb::window());
        let p: &'static Program<'static> = Box::leak(Box::new(Program {
            fs_uniforms: Box::leak(
                b.prog.lists[0]
                    .iter()
                    .map(|u| uniform71::Word { contents: u.contents, data: u.data })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            ..b.prog.program
        }));
        g.clear(COLOR);
        g.draw(p, &PUV_ATTRS, &[], &[]);
        g.finish();
        g
    };
    assert_eq!(
        g.refusal.expect("fill71 refuses an unmasked list").to_string(),
        "uniform71: TMU config uniform (contents 40): the 7.1 texture state is not written yet"
    );
}
