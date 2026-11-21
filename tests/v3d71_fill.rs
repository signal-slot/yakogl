// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/fill71.rs` (the 7.1 table's fill: GL frame and draw state into
//! the `v3d_job71` descriptors) against Mesa's 7.1 references for the six
//! gl_test shapes, tools/v3d/pi5/gl_{scissor,scisbox,writemask,viewport,
//! fbods,colormask,mrt4,mrt8}.clif (cases 5, 58, 59, 66, 67, 102 and the 4-
//! and 8-target clears; docs/notes/v3d71-gl-table.md section 1,
//! tools/v3d/pi5/README.md "The gl_test shapes").
//!
//! Each shape is written as its GL calls (tools/v3d/pi5/src/gl_shapes.py,
//! one flush per glFinish) through a small GL state model ([`Gl`]), and
//! `fill71::fill` turns every flush into job descriptors. Three checks:
//!
//! 1. **Descriptors** (always): every job's target, clear and draw
//!    descriptors, as the C builder reads them, equal the descriptors
//!    tools/v3d/job71_check.c's `gl` mode builds by hand for the same jobs
//!    (`GJ_*`, job71_check.c:906-963), which tools/v3d/check71.d/job71.sh
//!    part (f) compares with these references packet by packet. Job and
//!    draw counts are also read from the references themselves.
//! 2. **Packets** (with a host C compiler, `$CC` or `cc`): the test builds
//!    `tests/v3d71_fill_harness.c` with driver/src/v3d/v3d_job71.c and
//!    v3d_frame.c at `-DV3D_CL_VER=71`, feeds it the filled descriptors and
//!    decodes the binning list, render list and tile list it writes with the
//!    packet table of Mesa's `cle/v3d_packet.xml` (vendored) for version 71.
//!    They are compared with the reference job by job as job71.sh's
//!    gl_cmp.py compares (the binning state each draw sees, the render and
//!    tile lists packet by packet, every field the dump prints except
//!    addresses, the tile allocation size). Each remaining difference must
//!    match a named entry ([`ALLOW`]) with its reason, and every entry must
//!    match; without a C compiler this half prints a skip line.
//! 3. **Refusals**: each shape the fill does not carry yields its named
//!    refusal (module and text), case 59 on a window whose depth/stencil
//!    lives in the tile buffer only among them.
//!
//! The programs' record fields are compile results, not GL state: they are
//! those of the references' records, as job71_check's `gl_programs` takes
//! them; the shader records, uniform streams and QPU code are not compared
//! here (job71.sh part (f), tests/v3d71_compose2.rs and
//! tests/v3d71_gltest_shaders.rs compare them).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_fill
//! ```
#![allow(dead_code)]

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
mod attr71;
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
mod rt71;
#[path = "../src/v3d/uniform71.rs"]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use ffi71::*;
use fill71::{ClearQuad, ClearReq, ColorBuffer, DrawIn, DsBuffer, FilledJob, Framebuffer, History, Op, Program};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use zskeep71::Known71;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/* ---- the addresses and programs of job71_check's gl mode (job71_check.c:794-852) ---- */

const G_SIZE: u32 = 64;
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

fn vdesc(code: u32, out: u32) -> record71::VertexDesc {
    record71::VertexDesc {
        prog: record71::ProgramDesc { code, uniforms: 0, threads: 4, single_seg: true, propagate_nans: false },
        vpm_input_size: 0,
        vpm_output_size: out,
        min_input_segments: 1,
        min_output_segments: 0,
        ..Default::default()
    }
}

fn reads(r: &[u8]) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[..r.len()].copy_from_slice(r);
    a
}

/// ours_clear's streams (tools/v3d/gltest-shaders71/ours_clear.json): the
/// colour as UNIFORM words 0..3, the viewport words the VS reads.
static CLEAR_FS_U: [uniform71::Word; 4] = [
    uniform71::Word { contents: 1, data: 0 },
    uniform71::Word { contents: 1, data: 1 },
    uniform71::Word { contents: 1, data: 2 },
    uniform71::Word { contents: 1, data: 3 },
];
static CLEAR_VS_U: [uniform71::Word; 6] = [
    uniform71::Word { contents: 0, data: 0 },
    uniform71::Word { contents: 2, data: 0 },
    uniform71::Word { contents: 3, data: 0 },
    uniform71::Word { contents: 5, data: 0 },
    uniform71::Word { contents: 4, data: 0 },
    uniform71::Word { contents: 0, data: 3 },
];

/// gl_test's program (gl_test_vs_src / gl_test_fs_src) as the references'
/// records read: 6 FS inputs, real pixel centre W, VS output segment 2, CS 1,
/// every stage 4-way, VCM cache 4; a_pos read as 4 by both, a_color 4, a_uv 2.
fn gl_program() -> Program<'static> {
    let mut desc = record71::DrawDesc::default();
    desc.fs.prog =
        record71::ProgramDesc { code: G_CODE, uniforms: 0, threads: 4, single_seg: false, propagate_nans: false };
    desc.fs.num_varyings = 6;
    desc.fs.uses_center_w = true;
    desc.vs = vdesc(G_CODE + 0x100, 2);
    desc.cs = vdesc(G_CODE + 0x200, 1);
    Program {
        desc,
        cs_reads: reads(&[4]),
        vs_reads: reads(&[4, 4, 2]),
        cs_vcm_cache_size: 4,
        vs_vcm_cache_size: 4,
        separate_segments: false,
        varyings: Default::default(),
        discards: false,
        fs_uniforms: &[],
        vs_uniforms: &[],
        cs_uniforms: &[],
    }
}

/// The clear quad's program: the record of Mesa's clear blitter (no FS input,
/// VS and CS output segment 1), with ours_clear's uniform streams.
fn clear_program() -> Program<'static> {
    let mut desc = record71::DrawDesc::default();
    desc.fs.prog = record71::ProgramDesc {
        code: G_CODE + 0x300,
        uniforms: 0,
        threads: 4,
        single_seg: false,
        propagate_nans: false,
    };
    desc.vs = vdesc(G_CODE + 0x400, 1);
    desc.cs = vdesc(G_CODE + 0x500, 1);
    Program {
        desc,
        cs_reads: reads(&[4]),
        vs_reads: reads(&[4]),
        cs_vcm_cache_size: 4,
        vs_vcm_cache_size: 4,
        separate_segments: false,
        varyings: Default::default(),
        discards: false,
        fs_uniforms: &CLEAR_FS_U,
        vs_uniforms: &CLEAR_VS_U,
        cs_uniforms: &CLEAR_VS_U,
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
/// The blitter's quad: one 3-float position of stride 36.
const QUAD_ATTR: attr71::Attr = float_attr(3, 36, G_VTX + 0x100);

fn programs() -> &'static (Program<'static>, Program<'static>) {
    static P: std::sync::OnceLock<(Program<'static>, Program<'static>)> = std::sync::OnceLock::new();
    P.get_or_init(|| (gl_program(), clear_program()))
}

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &programs().1, attr: QUAD_ATTR, dummy_addr: G_VTX }
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

/* ---- 1. the descriptors against job71_check.c's GJ_* jobs ------------------ */

#[derive(Clone, Copy, Debug, PartialEq)]
enum Prog {
    GlTest,
    Blit,
}

/// `struct gdraw` (job71_check.c:855-863).
#[derive(Clone, Copy, Debug)]
struct GDraw {
    prog: Prog,
    first: u32,
    colormask: u32,
    scissor: bool,
    depth_func: u32,
    stencil: Option<V3dJob71StencilFace>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GZs {
    None,
    D24S8,
    D16,
}

const G_ALL: u8 = 0x80;

/// `struct gjob` (job71_check.c:869-880).
#[derive(Clone, Debug)]
struct GJob {
    rts: u32,
    zs: GZs,
    load_rt: bool,
    color: u32,
    z: f32,
    stencil: u32,
    zs_load: u8,
    zs_store: u8,
    zs_cleared: bool,
    d: Vec<GDraw>,
}

const BLACK: u32 = 0xff000000;
const BLUE: u32 = 0xffff0000;
const WHITE: u32 = 0xffffffff;
const RED_QUARTER: u32 = 0x400000ff;

fn sf(func: u32, op: u32, r: u32, wm: u32) -> V3dJob71StencilFace {
    V3dJob71StencilFace { func, sfail_op: op, zfail_op: op, zpass_op: op, ref_: r, value_mask: 0xff, write_mask: wm }
}
fn glt(first: u32) -> GDraw {
    GDraw { prog: Prog::GlTest, first, colormask: 0xf, scissor: false, depth_func: 0, stencil: None }
}
fn glt_scissor() -> GDraw {
    GDraw { scissor: true, ..glt(0) }
}
fn glt_less(first: u32) -> GDraw {
    GDraw { depth_func: V3D_COMPARE_FUNC_LESS, ..glt(first) }
}
fn glt_equal(r: u32) -> GDraw {
    GDraw { stencil: Some(sf(V3D_COMPARE_FUNC_EQUAL, V3D_STENCIL_OP_KEEP, r, 0xff)), ..glt(0) }
}
fn blit(mask: u32) -> GDraw {
    GDraw { prog: Prog::Blit, first: 0, colormask: mask, scissor: false, depth_func: 0, stencil: None }
}
fn blit_stencil(r: u32, wm: u32) -> GDraw {
    GDraw { stencil: Some(sf(V3D_COMPARE_FUNC_ALWAYS, V3D_STENCIL_OP_REPLACE, r, wm)), ..blit(0) }
}
fn win(color: u32) -> GJob {
    GJob {
        rts: 1,
        zs: GZs::D24S8,
        load_rt: false,
        color,
        z: 0.0,
        stencil: 0,
        zs_load: 0,
        zs_store: 0,
        zs_cleared: true,
        d: vec![],
    }
}
fn fbo(color: u32) -> GJob {
    GJob { zs: GZs::None, zs_cleared: false, ..win(color) }
}
fn loaded(mut j: GJob) -> GJob {
    j.load_rt = true;
    j.color = 0;
    j
}
fn with(mut j: GJob, d: &[GDraw]) -> GJob {
    j.d = d.to_vec();
    j
}

/// job71_check.c:906-963, job for job (the comment of each table there).
fn gjobs(name: &str) -> Vec<GJob> {
    use ffi71::{V3D_JOB71_ZS_DEPTH as ZD, V3D_JOB71_ZS_STENCIL as ZST};
    match name {
        "scissor" => vec![win(BLACK), with(loaded(win(0)), &[blit(0xf), glt_scissor()])],
        "scisbox" => {
            vec![with(win(BLUE), &[blit(0xf)]), with(win(BLUE), &[glt(0), blit(0xf)]), with(win(BLUE), &[blit(0xf)])]
        }
        "writemask" => vec![
            win(BLUE),
            GJob { z: 1.0, zs_store: ZD, ..with(win(BLUE), &[glt_less(0), glt_less(0)]) },
            GJob { zs_store: ZST, ..with(win(BLUE), &[blit_stencil(0xff, 0x0f), glt_equal(0xff)]) },
            GJob { zs_load: ZST, zs_cleared: false, ..with(loaded(win(0)), &[glt_equal(0x0f)]) },
        ],
        "viewport" => vec![win(WHITE), win(BLUE), with(loaded(win(0)), &[blit(0xf)])],
        "fbods" => vec![
            with(fbo(BLUE), &[glt(0), glt(6)]),
            with(fbo(BLUE), &[glt(0)]),
            fbo(RED_QUARTER),
            GJob { zs: GZs::D16, z: 1.0, zs_store: G_ALL, zs_cleared: true, ..with(fbo(BLUE), &[glt_less(0)]) },
            GJob { zs: GZs::D16, zs_store: G_ALL, ..with(loaded(fbo(0)), &[glt_less(6)]) },
            GJob { zs_store: ZST, ..with(win(BLUE), &[blit_stencil(1, 0xff), glt_equal(0)]) },
            GJob { zs_store: ZST, ..with(win(BLUE), &[glt(6), blit_stencil(1, 0xff), glt_equal(0)]) },
            GJob { zs_store: ZST, ..with(win(BLUE), &[glt(6), blit_stencil(1, 0xff)]) },
            GJob { zs_load: ZST, zs_cleared: false, ..with(loaded(win(0)), &[glt_equal(0)]) },
        ],
        "colormask" => vec![
            win(BLACK),
            with(loaded(win(0)), &[blit(0x9)]),
            win(BLACK),
            with(loaded(win(0)), &[blit(0x2)]),
            win(BLACK),
            with(loaded(win(0)), &[blit(0x4), glt(0)]),
        ],
        "mrt4" => vec![GJob { rts: 4, ..fbo(BLUE) }],
        "mrt8" => vec![GJob { rts: 8, ..fbo(BLUE) }],
        _ => unreachable!(),
    }
}

/// Where fill71 differs from Mesa's job on purpose (the descriptor half):
/// `(shape, job, what, reason)`. Each must be seen.
const DESC_ALLOW: [(&str, usize, &str, &str); 3] = [
    (
        "scisbox",
        2,
        "draws",
        "EMPTYBOX: clearplan71 drops a clear whose scissor box is empty (Nothing); Mesa's blitter draws a quad that covers no pixel",
    ),
    (
        "fbods",
        2,
        "color",
        "RGBALPHA: a GL_RGB target keeps alpha 1 (the 4.2 table's rule, src/v3d/state.rs): its clear writes alpha 1, Mesa's alpha 0.25",
    ),
    (
        "fbods",
        4,
        "z",
        "UNDEFZ: zskeep71 starts an undefined depth plane the job uses at 1.0 (UNDEFINED_Z_BITS), Mesa's zeroed job at 0.0",
    ),
];

/// The Z/S buffer a load or store of `aspects` moves (v3d_job71.c
/// `zs_buffer`), for comparing an aspect field of 0 with an explicit one.
fn zs_buffer(t: &V3dJob71Target, aspects: u8) -> u8 {
    let has_stencil = t.s.address == 0 && t.zs.format == V3D_OUTPUT_IMAGE_FORMAT_D24S8;
    if aspects == 0 {
        1 | if has_stencil { 2 } else { 0 }
    } else {
        aspects
    }
}

fn check_descriptors(name: &str, jobs: &[FilledJob], seen: &mut Vec<(&'static str, usize, &'static str)>) {
    let want = gjobs(name);
    assert_eq!(jobs.len(), want.len(), "gl_{name}: jobs");
    let (glp, blp) = programs();
    for (k, (j, g)) in jobs.iter().zip(&want).enumerate() {
        let w = format!("gl_{name} job {k}");
        let mut allow = |what: &'static str| -> bool {
            if let Some(a) = DESC_ALLOW.iter().find(|a| a.0 == name && a.1 == k && a.2 == what) {
                seen.push((a.0, a.1, a.2));
                true
            } else {
                false
            }
        };
        /* the target: gl_target (job71_check.c:976-1000) */
        let t = &j.target;
        assert_eq!((t.width, t.height, t.tile_alloc, t.rts), (G_SIZE, G_SIZE, G_TILE, g.rts), "{w}: target");
        for n in 0..g.rts as usize {
            let rt = &t.rt[n];
            assert_eq!(
                (rt.address, rt.format, rt.memory_format, rt.height_in_ub_or_stride, rt.r_b_swap),
                (g_rt(n as u32), V3D_OUTPUT_IMAGE_FORMAT_RGBA8, V3D_MEMORY_FORMAT_UIF_NO_XOR, 8, false),
                "{w}: rt {n}"
            );
        }
        let (dt, zs) = match g.zs {
            GZs::None => (V3D_INTERNAL_TYPE_DEPTH_32F, V3dJob71Zs::default()),
            GZs::D24S8 => (
                V3D_INTERNAL_TYPE_DEPTH_24,
                V3dJob71Zs {
                    address: G_ZS,
                    format: V3D_OUTPUT_IMAGE_FORMAT_D24S8,
                    memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
                    height_in_ub: 8,
                },
            ),
            GZs::D16 => (
                V3D_INTERNAL_TYPE_DEPTH_16,
                V3dJob71Zs {
                    address: G_ZS,
                    format: V3D_OUTPUT_IMAGE_FORMAT_D16,
                    memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
                    height_in_ub: 8,
                },
            ),
        };
        assert_eq!((t.depth_type, t.zs, t.s), (dt, zs, V3dJob71Zs::default()), "{w}: depth/stencil target");

        /* the clear: c.* of gl_job (job71_check.c:1086-1097), as the builder
         * reads it */
        let c = &j.clear;
        let load = if g.load_rt { (1u32 << g.rts) - 1 } else { 0 };
        assert_eq!(c.load_rts, load, "{w}: load_rts");
        for n in 0..g.rts as usize {
            if load & (1 << n) == 0 {
                let words = [g.color, 0, 0, 0];
                if c.color[n] != words && allow("color") {
                    assert_eq!(c.color[n], [0xff0000ff, 0, 0, 0], "{w}: rt {n} colour (RGBALPHA)");
                } else {
                    assert_eq!(c.color[n], words, "{w}: rt {n} clear colour");
                }
            }
        }
        if c.z != g.z && allow("z") {
            assert_eq!(c.z, 1.0, "{w}: Z start (UNDEFZ)");
        } else {
            assert_eq!(c.z, g.z, "{w}: Z clear value");
        }
        assert_eq!(c.stencil, g.stencil, "{w}: stencil clear value");
        assert_eq!(
            (c.zs_load, c.zs_store, c.s_load, c.s_store),
            (g.zs_load != 0, g.zs_store != 0, false, false),
            "{w}: Z/S load, store"
        );
        if c.zs_load {
            assert_eq!(zs_buffer(t, c.zs_load_aspects), zs_buffer(t, g.zs_load & !G_ALL), "{w}: Z/S load buffer");
        }
        if c.zs_store {
            assert_eq!(zs_buffer(t, c.zs_store_aspects), zs_buffer(t, g.zs_store & !G_ALL), "{w}: Z/S store buffer");
        }
        /* v3d_job71_end: Early Depth/Stencil Clear = zs_cleared && nothing kept */
        let kept = c.zs_load || c.zs_store;
        assert_eq!(c.zs_cleared && !kept, g.zs_cleared && !kept, "{w}: early Z/S clear");

        /* the draws: gl_job's state and draw (job71_check.c:1040-1084) */
        let gd: Vec<GDraw> = if j.draws.len() != g.d.len() && allow("draws") { vec![] } else { g.d.clone() };
        assert_eq!(j.draws.len(), gd.len(), "{w}: draws");
        for (i, (d, e)) in j.draws.iter().zip(&gd).enumerate() {
            let w = format!("{w} draw {i}");
            let p = if e.prog == Prog::Blit { blp } else { glp };
            assert_eq!(
                (d.fs.code, d.vs.code, d.cs.code),
                (p.desc.fs.prog.code, p.desc.vs.prog.code, p.desc.cs.prog.code),
                "{w}: program"
            );
            assert_eq!(d.is_clear, e.prog == Prog::Blit, "{w}: a clear quad");
            let (prim, count) = if e.prog == Prog::Blit { (V3D_PRIM_TRIANGLE_FAN, 4) } else { (V3D_PRIM_TRIANGLES, 6) };
            assert_eq!(
                (d.prim, d.count, d.first, d.indexed, d.instances),
                (prim, count, e.first, false, 1),
                "{w}: primitive"
            );
            let attrs: Vec<V3dVertexAttr> = if e.prog == Prog::Blit {
                vec![V3dVertexAttr { address: G_VTX + 0x100, stride: 36, components: 3, divisor: 0 }]
            } else {
                vec![
                    V3dVertexAttr { address: G_VTX, stride: 40, components: 4, divisor: 0 },
                    V3dVertexAttr { address: G_VTX + 16, stride: 40, components: 4, divisor: 0 },
                    V3dVertexAttr { address: G_VTX + 32, stride: 40, components: 2, divisor: 0 },
                ]
            };
            assert_eq!(d.attrs, attrs, "{w}: attributes");
            assert_eq!(d.fs.num_inputs, p.desc.fs.num_varyings, "{w}: FS inputs");
            assert_eq!((d.vs.vattr_sizes, d.cs.vattr_sizes), (p.vs_reads, p.cs_reads), "{w}: reads");
            assert_eq!((d.vs.vcm_cache_size, d.cs.vcm_cache_size), (4, 4), "{w}: VCM cache");
            let s = &d.state;
            /* v3d_viewport_gl(0, 0, 64, 64, 0, 1, false, 64) */
            assert_eq!((s.vp.scale, s.vp.translate), ([32.0, 32.0, 0.5], [32.0, 32.0, 0.5]), "{w}: viewport");
            assert_eq!(s.line_width, if e.prog == Prog::Blit { 0.0 } else { 1.0 }, "{w}: line width");
            assert_eq!(s.colormask, e.colormask, "{w}: colour mask");
            assert_eq!(
                (s.scissor, s.scissor_minx, s.scissor_miny, s.scissor_maxx, s.scissor_maxy),
                if e.scissor { (true, 0, 0, 32, 32) } else { (false, 0, 0, 0, 0) },
                "{w}: scissor"
            );
            assert_eq!((s.depth_enabled, s.depth_writemask), (e.depth_func != 0, e.depth_func != 0), "{w}: depth");
            if e.depth_func != 0 {
                assert_eq!(s.depth_func, e.depth_func, "{w}: depth func");
            }
            assert_eq!(s.stencil_enabled, e.stencil.is_some(), "{w}: stencil");
            if let Some(f) = e.stencil {
                assert_eq!((s.stencil_front, s.stencil_back_enabled), (f, false), "{w}: stencil face");
            }
            assert_eq!(
                (s.front_ccw, s.cull_front, s.cull_back, s.offset_tri),
                (false, false, false, false),
                "{w}: raster"
            );
            assert_eq!(
                (s.blend_enabled, s.sample_mask, s.oq_addr, s.wpos),
                (false, 0xf, 0, V3dWpos::default()),
                "{w}: blend, samples"
            );
            if e.prog == Prog::Blit {
                let q = d.quad.expect("a clear quad's vertices");
                assert!(q.iter().all(|v| v[3] == 1.0), "{w}: quad w");
            }
        }
    }
}

#[test]
fn descriptors_of_every_shape_match_job71_checks_jobs() {
    let mut seen = vec![];
    for name in SHAPES {
        let jobs = filled(name);
        check_descriptors(name, &jobs, &mut seen);
        let refs = ref_jobs(name);
        assert_eq!(jobs.len(), refs.len(), "gl_{name}: the reference's jobs");
        for (k, (j, r)) in jobs.iter().zip(&refs).enumerate() {
            let ref_draws = bcl_split(&r.bcl).1.len();
            let ok = j.draws.len() == ref_draws || DESC_ALLOW.iter().any(|a| a.0 == name && a.1 == k && a.2 == "draws");
            assert!(ok, "gl_{name} job {k}: {} draws, the reference has {ref_draws}", j.draws.len());
        }
        println!(
            "gl_{name}: {} job(s), {} draw(s): descriptors as job71_check's",
            jobs.len(),
            jobs.iter().map(|j| j.draws.len()).sum::<usize>()
        );
    }
    for a in DESC_ALLOW {
        assert!(seen.iter().any(|s| (s.0, s.1, s.2) == (a.0, a.1, a.2)), "named difference not seen: {}", a.3);
        println!("named: gl_{} job {} {}: {}", a.0, a.1, a.2, a.3);
    }
}

#[test]
fn the_clear_quads_cover_the_scissor_box() {
    /* case 58's red clear scissored to (16, 16, 32, 32): x and y from -0.5 to
     * +0.5 in clip space */
    let jobs = filled("scisbox");
    let q = jobs[0].draws[0].quad.unwrap();
    assert_eq!(q, [[-0.5, -0.5, 0.0, 1.0], [0.5, -0.5, 0.0, 1.0], [0.5, 0.5, 0.0, 1.0], [-0.5, 0.5, 0.0, 1.0]]);
    /* its colour reaches the clear program as GL uniform words 0..3 */
    assert_eq!(jobs[0].draws[0].fs_words, [1.0f32, 0.0, 0.0, 1.0].map(f32::to_bits));
    /* case 59's stencil quad: the whole target, stencil REPLACE 0xff through 0x0f */
    let jobs = filled("writemask");
    let d = &jobs[2].draws[0];
    assert_eq!(d.quad.unwrap()[2], [1.0, 1.0, 0.0, 1.0]);
    assert_eq!((d.state.stencil_front.ref_, d.state.stencil_front.write_mask), (0xff, 0x0f));
}

#[test]
fn the_ffi_draw_points_into_the_filled_draw() {
    let jobs = filled("scissor");
    let d = &jobs[1].draws[1];
    let f = d.ffi();
    assert!(std::ptr::eq(f.st, &d.state) && std::ptr::eq(f.fs, &d.fs) && std::ptr::eq(f.vs, &d.vs));
    assert_eq!((f.num_attrs, f.attrs), (3, d.attrs.as_ptr()));
    assert!(f.tmu.is_null() && f.num_tmu == 0 && f.vs_tmu.is_null());
    let q = &jobs[1].draws[0];
    let fq = q.ffi();
    assert_eq!((fq.fs_storage.count, unsafe { *fq.fs_storage.words.add(3) }), (4, 1.0f32.to_bits()));
    assert_eq!(unsafe { (*fq.fs).num_uniforms }, 4);
}

/* ---- 3. the refusals --------------------------------------------------------- */

fn refused(g: Gl) -> fill71::Refusal {
    g.refusal.unwrap_or_else(|| panic!("not refused: {} job(s)", g.jobs.len()))
}

#[test]
fn case_59_on_a_tile_only_window_is_refused_by_the_4_2_name() {
    /* the Pi 4's window: the depth/stencil lives in the tile buffer only */
    let mut g = Gl::new(Fb { ds_kept: false, ..Fb::window() });
    shape_writemask(&mut g);
    let r = refused(g);
    assert_eq!(r.module, "zskeep71");
    assert_eq!(
        r.reason,
        "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in"
    );
}

#[test]
fn the_other_five_cases_also_fill_on_a_tile_only_window() {
    for name in ["scissor", "scisbox", "viewport", "colormask"] {
        let mut g = Gl::new(Fb { ds_kept: false, ..Fb::window() });
        match name {
            "scissor" => shape_scissor(&mut g),
            "scisbox" => shape_scisbox(&mut g),
            "viewport" => shape_viewport(&mut g),
            _ => shape_colormask(&mut g),
        }
        assert!(g.refusal.is_none(), "{name}: {:?}", g.refusal);
        for j in &g.jobs {
            assert_eq!(j.target.zs.address, 0, "{name}: no Z/S image");
            assert!(!(j.clear.zs_load || j.clear.zs_store), "{name}: nothing kept");
        }
    }
}

fn one_flush(fb: Fb, f: impl FnOnce(&mut Gl)) -> Result<Vec<FilledJob>, fill71::Refusal> {
    let mut g = Gl::new(fb);
    f(&mut g);
    g.finish();
    match g.refusal {
        Some(r) => Err(r),
        None => Ok(g.jobs),
    }
}

fn expect_refusal(what: &str, r: Result<Vec<FilledJob>, fill71::Refusal>, module: &str, reason: &str) {
    match r {
        Ok(j) => panic!("{what}: filled {} job(s), expected the refusal {module}: {reason}", j.len()),
        Err(e) => {
            assert_eq!(e.module, module, "{what}: {e}");
            assert!(e.reason.contains(reason), "{what}: {e}");
            println!("refused, {what}: {e}");
        }
    }
}

#[test]
fn each_unsupported_shape_has_its_named_refusal() {
    let clear_blue = |g: &mut Gl| {
        g.clear_color = [0.0, 0.0, 1.0, 1.0];
        g.clear(COLOR);
    };
    let fmt = |f: u32| Fb::color(0, f);
    expect_refusal("an RGBA16F target", one_flush(fmt(0x881A), clear_blue), "fill71", "no output image format");
    expect_refusal("an RGB565 target", one_flush(fmt(0x8D62), clear_blue), "fill71", "no output image format");
    expect_refusal("an SNORM target", one_flush(fmt(0x8F97), clear_blue), "rt71", "no render-target row");
    expect_refusal(
        "a depth format as colour",
        one_flush(fmt(fill71::GL_DEPTH_COMPONENT16), clear_blue),
        "rt71",
        "not a colour target",
    );
    expect_refusal(
        "DEPTH32F_STENCIL8",
        one_flush(Fb { ds: Some((1, fill71::GL_DEPTH32F_STENCIL8)), ..fmt(fill71::GL_RGBA8) }, clear_blue),
        "fill71",
        "separate stencil",
    );
    expect_refusal(
        "a colour format as depth",
        one_flush(Fb { ds: Some((1, fill71::GL_RGBA8)), ..fmt(fill71::GL_RGBA8) }, clear_blue),
        "admit71",
        "not a depth or stencil format",
    );
    expect_refusal(
        "nine colour targets",
        one_flush(Fb { colors: (0..9).map(|i| (i, fill71::GL_RGBA8)).collect(), ds: None, ds_kept: true }, clear_blue),
        "fill71",
        "more colour attachments",
    );
    /* the draws */
    let draw_with = |f: fn(&mut draw71::DrawState)| {
        move |g: &mut Gl| {
            f(&mut g.st);
            g.draw(0);
        }
    };
    expect_refusal(
        "4x multisampling",
        one_flush(fmt(fill71::GL_RGBA8), draw_with(|s| s.samples = 4)),
        "draw71",
        "multisampled",
    );
    expect_refusal(
        "a depth range under 0.01",
        one_flush(fmt(fill71::GL_RGBA8), draw_with(|s| s.depth_range = [0.5, 0.505])),
        "draw71",
        "under 0.01",
    );
    expect_refusal(
        "independent blend",
        one_flush(
            Fb { colors: vec![(0, fill71::GL_RGBA8), (1, fill71::GL_RGBA8)], ds: None, ds_kept: true },
            draw_with(|s| {
                s.independent_blend = true;
                s.targets[1].blend_enable = true;
            }),
        ),
        "fill71",
        "one blend state",
    );
    expect_refusal(
        "per-target colour masks",
        one_flush(
            Fb { colors: vec![(0, fill71::GL_RGBA8), (1, fill71::GL_RGBA8)], ds: None, ds_kept: true },
            draw_with(|s| {
                s.independent_blend = true;
                s.targets[1].color_mask = 0x1;
            }),
        ),
        "fill71",
        "one colour write mask",
    );
    expect_refusal(
        "blending with destination alpha on GL_RGB",
        one_flush(
            fmt(fill71::GL_RGB8),
            draw_with(|s| {
                s.targets[0].blend_enable = true;
                s.targets[0].blend.rgb_src = draw71::BlendFactor::DstAlpha;
            }),
        ),
        "draw71",
        "destination alpha",
    );
    /* a clear quad of one of two targets (glDrawBuffers): one mask for all */
    expect_refusal(
        "a scissored clear of one of two targets",
        one_flush(Fb { colors: vec![(0, fill71::GL_RGBA8), (1, fill71::GL_RGBA8)], ds: None, ds_kept: true }, |g| {
            g.draw_buffers = 1;
            g.scissor_on = true;
            g.scissor = [0, 0, 8, 8];
            g.clear_color = [1.0; 4];
            g.clear(COLOR);
        }),
        "fill71",
        "one colour write mask",
    );
    /* the programs and arrays */
    fn custom(g: &mut Gl, prog: &'static Program<'static>, attrs: &'static [attr71::Attr], prim: prim71::Draw) {
        let mut st = g.st;
        st.scissor = None;
        g.ops.push(Op::Draw(Box::new(DrawIn {
            state: st,
            line_width: 1.0,
            blend_color: [0.0; 4],
            program: prog,
            attrs,
            dummy_addr: G_VTX,
            prim,
            vs_words: &[],
            fs_words: &[],
            oq_addr: 0,
        })));
    }
    let leak = |p: Program<'static>| -> &'static Program<'static> { Box::leak(Box::new(p)) };
    static BYTE_ATTRS: [attr71::Attr; 3] = [
        attr71::Attr { gl_type: 0x1401, normalized: true, ..float_attr(4, 40, G_VTX) },
        float_attr(4, 40, G_VTX + 16),
        float_attr(2, 40, G_VTX + 32),
    ];
    let tri = prim71::Draw::arrays(GL_TRIANGLES, 0, 6);
    expect_refusal(
        "a normalized byte attribute",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, &programs().0, &BYTE_ATTRS, tri)),
        "fill71",
        "float vertex attribute records only",
    );
    static TWO_ATTRS: [attr71::Attr; 2] = [float_attr(4, 40, G_VTX), float_attr(4, 40, G_VTX + 16)];
    expect_refusal(
        "a program reading an attribute without a record",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, &programs().0, &TWO_ATTRS, tri)),
        "attr71",
        "has no record",
    );
    static DISABLED: [attr71::Attr; 3] =
        [float_attr(4, 40, G_VTX), attr71::Attr::disabled(), float_attr(2, 40, G_VTX + 32)];
    expect_refusal(
        "a constant attribute without a slot",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, &programs().0, &DISABLED, tri)),
        "attr71",
        "no default-attribute BO",
    );
    let mut p = gl_program();
    p.desc.vs.uses_draw_index = true;
    let p = leak(p);
    expect_refusal(
        "gl_DrawID",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, p, &GL_ATTRS, tri)),
        "record71",
        "gl_DrawID",
    );
    let mut p = gl_program();
    p.desc.cs.uses_base_instance = true;
    let p = leak(p);
    expect_refusal(
        "gl_BaseInstance",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, p, &GL_ATTRS, tri)),
        "fill71",
        "gl_BaseInstance",
    );
    let mut p = gl_program();
    p.separate_segments = true;
    let p = leak(p);
    expect_refusal(
        "separate VPM segments",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, p, &GL_ATTRS, tri)),
        "attr71",
        "separate input and output VPM segments",
    );
    static TMU_U: [uniform71::Word; 1] = [uniform71::Word { contents: 40, data: 0 }];
    let mut p = gl_program();
    p.fs_uniforms = &TMU_U;
    let p = leak(p);
    expect_refusal(
        "a TMU configuration uniform (texture state is P10)",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, p, &GL_ATTRS, tri)),
        "uniform71",
        "TMU config",
    );
    let mut quads = tri;
    quads.mode = 7;
    expect_refusal(
        "GL_QUADS",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, &programs().0, &GL_ATTRS, quads)),
        "prim71",
        "primitive mode",
    );
    let mut based = tri;
    based.base_vertex = 3;
    expect_refusal(
        "a base vertex",
        one_flush(fmt(fill71::GL_RGBA8), |g| custom(g, &programs().0, &GL_ATTRS, based)),
        "fill71",
        "base vertex",
    );
    /* the revision: 7.1.6 has no draw-index record */
    let colors = [ColorBuffer {
        gl_format: fill71::GL_RGBA8,
        address: g_rt(0),
        memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
        height_in_ub_or_stride: 8,
    }];
    let fb = Framebuffer { width: 64, height: 64, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds: None };
    let mut g = Gl::new(fmt(fill71::GL_RGBA8));
    g.draw(0);
    let r = fill71::fill(&fb, record71::Revision { ver: 71, rev: 6 }, &History::NEW, &clear_quad(), &g.ops);
    expect_refusal("V3D 7.1.6", r, "record71", "has no GL Shader State Record Draw Index");
    /* the size: 7681 px */
    let fb = Framebuffer { width: 7681, ..fb };
    let r = fill71::fill(&fb, record71::BCM2712_D0, &History::NEW, &clear_quad(), &[]);
    expect_refusal("a 7681 px target", r, "rt71", "larger than 7680 px");
    for (i, a) in fill71::FILL71_REFUSALS.iter().enumerate() {
        assert!(a.starts_with("V3D 7.1: ") && fill71::FILL71_REFUSALS[..i].iter().all(|b| b != a), "{a}");
    }
}

#[test]
fn eight_targets_and_7680_px_fill() {
    let jobs = filled("mrt8");
    assert_eq!(jobs[0].target.rts, 8);
    let r = one_flush(Fb::color(0, fill71::GL_RGBA8), |g| {
        g.clear_color = [0.0; 4];
        g.clear(COLOR);
    });
    assert!(r.is_ok());
    let colors = [ColorBuffer {
        gl_format: fill71::GL_RGBA8,
        address: g_rt(0),
        memory_format: V3D_MEMORY_FORMAT_RASTER,
        height_in_ub_or_stride: 7680 * 4,
    }];
    let fb = Framebuffer { width: 7680, height: 16, tile_alloc: G_TILE, flip_y: true, colors: &colors, ds: None };
    let ops = [Op::Clear(ClearReq::color([0.0, 1.0, 0.0, 1.0]))];
    let jobs = fill71::fill(&fb, record71::BCM2712_D0, &History::NEW, &clear_quad(), &ops).unwrap();
    assert_eq!((jobs.len(), jobs[0].target.width, jobs[0].clear.color[0][0]), (1, 7680, 0xff00ff00));
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
    RefJob { bcl: get(&bcl), rcl: get(&rcl), tiles, args }
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

/* ---- 2. the packets the builder writes against the references ----------------- */

fn find_cc() -> Option<String> {
    let cands = [std::env::var("CC").unwrap_or_default(), "cc".to_string()];
    for c in cands {
        if c.is_empty() {
            continue;
        }
        let mut parts = c.split_whitespace();
        let Some(prog) = parts.next() else { continue };
        if Command::new(prog).arg("--version").output().is_ok() {
            return Some(c);
        }
    }
    None
}

/// The harness, built once: None without a C compiler.
fn harness() -> Option<&'static str> {
    static H: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        let cc = find_cc()?;
        let out = format!("{}/v3d71_fill_harness", env!("CARGO_TARGET_TMPDIR"));
        let mesa = format!("{ROOT}/third_party/mesa-26.2.1");
        let mut parts = cc.split_whitespace();
        let prog = parts.next().unwrap();
        let r = Command::new(prog)
            .args(parts)
            .args([
                "-O1",
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-D_GNU_SOURCE",
                "-DHAVE_ENDIAN_H",
                "-DV3D_CL_VER=71",
            ])
            .arg(format!("-I{mesa}/src"))
            .arg(format!("-I{mesa}/include"))
            .arg(format!("-I{mesa}/src/broadcom"))
            .arg(format!("-I{ROOT}/driver/src/v3d"))
            .arg(format!("-I{ROOT}/driver/src/v3d/generated"))
            .arg(format!("{ROOT}/tests/v3d71_fill_harness.c"))
            .arg(format!("{ROOT}/driver/src/v3d/v3d_job71.c"))
            .arg(format!("{ROOT}/driver/src/v3d/v3d_frame.c"))
            .args(["-lm", "-o", &out])
            .output()
            .expect("run the C compiler");
        assert!(r.status.success(), "v3d71_fill_harness.c did not build:\n{}", String::from_utf8_lossy(&r.stderr));
        Some(out)
    })
    .as_deref()
}

/// The descriptors as the harness reads them (v3d71_fill_harness.c rd_*).
struct Ser(String);

impl Ser {
    fn u(&mut self, v: u32) {
        self.0.push_str(&format!("{v:x} "));
    }
    fn b(&mut self, v: bool) {
        self.u(v as u32);
    }
    fn f(&mut self, v: f32) {
        self.u(v.to_bits());
    }
    fn zs(&mut self, z: &V3dJob71Zs) {
        for v in [z.address, z.format, z.memory_format, z.height_in_ub] {
            self.u(v);
        }
    }
    fn face(&mut self, s: &V3dJob71StencilFace) {
        for v in [s.func, s.sfail_op, s.zfail_op, s.zpass_op, s.ref_, s.value_mask, s.write_mask] {
            self.u(v);
        }
    }
    fn uniforms(&mut self, p: *const V3dUniform, n: u32) {
        self.u(n);
        for i in 0..n as usize {
            let w = unsafe { *p.add(i) };
            self.u(w.contents);
            self.u(w.data);
        }
    }
    fn vs(&mut self, p: &V3dVsProg) {
        self.u(p.code);
        self.u(p.threads);
        self.b(p.single_seg);
        self.u(p.vpm_input_size);
        self.u(p.vpm_output_size);
        self.b(p.separate_segments);
        self.u(p.vcm_cache_size);
        self.b(p.uses_vid);
        self.b(p.uses_iid);
        self.b(p.uses_biid);
        for v in p.vattr_sizes {
            self.u(u32::from(v));
        }
        self.uniforms(p.uniforms, p.num_uniforms);
    }
    fn job(&mut self, j: &FilledJob) {
        self.u(0xf171);
        let t = &j.target;
        for v in [t.width, t.height, t.tile_alloc, t.rts] {
            self.u(v);
        }
        for rt in &t.rt {
            for v in [rt.address, rt.format, rt.memory_format, rt.height_in_ub_or_stride] {
                self.u(v);
            }
            self.b(rt.r_b_swap);
            self.b(rt.force_alpha_1);
        }
        self.u(t.depth_type);
        self.zs(&t.zs);
        self.zs(&t.s);
        let c = &j.clear;
        self.u(c.load_rts);
        for w in c.color.iter().flatten() {
            self.u(*w);
        }
        self.f(c.z);
        self.u(c.stencil);
        for v in [c.zs_load, c.zs_store, c.s_load, c.s_store, c.zs_cleared] {
            self.b(v);
        }
        self.u(u32::from(c.zs_load_aspects));
        self.u(u32::from(c.zs_store_aspects));
        self.u(j.draws.len() as u32);
        for fd in &j.draws {
            let d = fd.ffi();
            self.u(0xd71);
            let s = unsafe { &*d.st };
            for v in s.vp.scale.iter().chain(&s.vp.translate) {
                self.f(*v);
            }
            self.b(s.scissor);
            for v in [s.scissor_minx, s.scissor_miny, s.scissor_maxx, s.scissor_maxy] {
                self.u(v);
            }
            for v in [s.front_ccw, s.cull_front, s.cull_back, s.offset_tri] {
                self.b(v);
            }
            for v in [s.offset_units, s.offset_scale, s.offset_clamp, s.line_width] {
                self.f(v);
            }
            self.b(s.depth_enabled);
            self.b(s.depth_writemask);
            self.u(s.depth_func);
            self.b(s.stencil_enabled);
            self.b(s.stencil_back_enabled);
            self.face(&s.stencil_front);
            self.face(&s.stencil_back);
            self.b(s.blend_enabled);
            for v in [s.rgb_func, s.rgb_src, s.rgb_dst, s.alpha_func, s.alpha_src, s.alpha_dst, s.colormask] {
                self.u(v);
            }
            for v in s.blend_color_f16 {
                self.u(u32::from(v));
            }
            self.f(s.wpos.y_scale);
            self.f(s.wpos.y_offset);
            self.u(s.sample_mask);
            self.u(s.oq_addr);
            let fs = unsafe { &*d.fs };
            for v in [fs.code, fs.threads] {
                self.u(v);
            }
            self.b(fs.single_seg);
            for v in [
                fs.num_inputs,
                fs.flat_shade_flags,
                fs.noperspective_flags,
                fs.centroid_flags,
                fs.flat_shade_flags_hi,
                fs.noperspective_flags_hi,
                fs.centroid_flags_hi,
            ] {
                self.u(v);
            }
            for v in [
                fs.writes_z,
                fs.disable_ez,
                fs.uses_center_w,
                fs.lock_scoreboard_on_first_thrsw,
                fs.uses_implicit_point_line_varyings,
                fs.uses_pid,
            ] {
                self.b(v);
            }
            self.uniforms(fs.uniforms, fs.num_uniforms);
            self.vs(unsafe { &*d.vs });
            self.vs(unsafe { &*d.cs });
            for st in [d.vs_storage, d.fs_storage] {
                self.u(st.count);
                for i in 0..st.count as usize {
                    self.u(unsafe { *st.words.add(i) });
                }
            }
            self.u(d.num_attrs);
            for i in 0..d.num_attrs as usize {
                let a = unsafe { *d.attrs.add(i) };
                for v in [a.address, a.stride, a.components, a.divisor] {
                    self.u(v);
                }
            }
            for v in [d.prim, d.count, d.first] {
                self.u(v);
            }
            self.b(d.indexed);
            for v in [d.index_type, d.index_buffer, d.index_buffer_bytes, d.index_offset] {
                self.u(v);
            }
            self.b(d.vs_writes_psiz);
            self.u(d.instances);
        }
        self.u(0xe71);
    }
}

/// One job as the builder wrote it.
struct Built {
    bcl: Vec<u8>,
    rcl: Vec<u8>,
    tiles: Vec<u8>,
    qms: u32,
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn build(exe: &str, shape: &str, jobs: &[FilledJob]) -> Vec<Built> {
    let mut s = Ser(String::new());
    s.u(jobs.len() as u32);
    for j in jobs {
        s.job(j);
    }
    let mut child =
        Command::new(exe).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(s.0.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "gl_{shape}: the harness failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let mut built = vec![];
    let (mut ind, mut ind_gpu) = (vec![], 0u32);
    let mut cur = Built { bcl: vec![], rcl: vec![], tiles: vec![], qms: 0 };
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w[0] {
            "job" => assert_eq!(w[2], "1", "gl_{shape} job {}: v3d_job71_begin refused the target", w[1]),
            "draw" => assert_eq!(w[2], "0", "gl_{shape}: draw {} returned {}", w[1], w[2]),
            "end" => assert_eq!(w[1], "1", "gl_{shape}: v3d_job71_end refused the job"),
            "qms" => cur.qms = w[1].parse().unwrap(),
            "bcl" => cur.bcl = hex_bytes(w.get(3).copied().unwrap_or("")),
            "rcl" => cur.rcl = hex_bytes(w.get(3).copied().unwrap_or("")),
            "ind" => {
                ind_gpu = u32::from_str_radix(w[1], 16).unwrap();
                ind = hex_bytes(w.get(3).copied().unwrap_or(""));
            }
            "tiles" => {
                let a = (u32::from_str_radix(w[1], 16).unwrap() - ind_gpu) as usize;
                let b = (u32::from_str_radix(w[2], 16).unwrap() - ind_gpu) as usize;
                cur.tiles = ind[a..b].to_vec();
                built.push(std::mem::replace(&mut cur, Built { bcl: vec![], rcl: vec![], tiles: vec![], qms: 0 }));
            }
            _ => panic!("gl_{shape}: harness line {line}"),
        }
    }
    assert_eq!(built.len(), jobs.len(), "gl_{shape}: jobs built");
    built
}

/// A named difference: `(shape glob, difference glob, id, reason)`. The first
/// four are tools/v3d/check71.d/job71.sh's gl.allow (v3d_job71's own
/// differences from Mesa's jobs); the rest are fill71's choices.
const ALLOW: [(&str, &str, &str, &str); 7] = [
    ("[!m]*", "* job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0", "EZ", "v3d_job71 never enables early Z (job71.sh gl.allow EZ)"),
    ("[!m]*", "* job * bcl draw * POINT_SIZE.Point Size: 1.000000 != 0.125000", "PSIZE", "the clear quad: Mesa's blitter writes point size 0.125, v3d_job71 1.0; a TRIANGLE_FAN does not read it (job71.sh gl.allow PSIZE)"),
    ("[fw]*", "* job * tiles #8 CLEAR_RT: present != absent", "CLRT", "a job that loads every buffer: v3d_job71 still writes CLEAR_RENDER_TARGETS after the stores (job71.sh gl.allow CLRT)"),
    ("scisbox", "scisbox job 2 bcl draws: 0 != 1", "EMPTYBOX", "clearplan71 drops a clear whose scissor box is empty; Mesa's blitter draws a quad that covers no pixel"),
    ("scisbox", "scisbox job 2 submit #0: 12288 != 16384", "EMPTYBOX", "the same job: no draw, so the tile allocation of a job with no draw"),
    ("fbods", "fbods job 2 rcl #* TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1.Clear Color low bits: 4278190335 != 1073742079", "RGBALPHA", "a GL_RGB target keeps alpha 1 (the 4.2 table's rule): its clear writes alpha 1, Mesa's alpha 0.25"),
    ("fbods", "fbods job 4 rcl #* TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES.Z Clear Value: 1.000000 != 0.000000", "UNDEFZ", "zskeep71 starts an undefined depth plane the job uses at 1.0, Mesa's zeroed job at 0.0"),
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

#[test]
fn the_builder_writes_the_references_packets_from_the_filled_descriptors() {
    let Some(exe) = harness() else {
        println!("skip: no C compiler (set CC or install cc): the filled descriptors were not run through v3d_job71");
        return;
    };
    let defs = packet_defs();
    let mut failures = vec![];
    let mut named: BTreeMap<&str, usize> = BTreeMap::new();
    for shape in SHAPES {
        let jobs = filled(shape);
        let built = build(exe, shape, &jobs);
        let refs = ref_jobs(shape);
        assert_eq!(built.len(), refs.len(), "gl_{shape}: jobs");
        let mut diffs = vec![];
        let mut packets = 0;
        for (k, (o, r)) in built.iter().zip(&refs).enumerate() {
            let w = format!("{shape} job {k}");
            let bcl = decode_list(&defs, &o.bcl, &format!("{w} bcl"));
            let rcl = decode_list(&defs, &o.rcl, &format!("{w} rcl"));
            let tiles = decode_list(&defs, &o.tiles, &format!("{w} tiles"));
            packets += r.bcl.len() + r.rcl.len() + r.tiles.len();
            bcl_cmp(&format!("{w} bcl"), &bcl, &r.bcl, &mut diffs);
            seq_cmp(&format!("{w} rcl"), &rcl, &r.rcl, &mut diffs);
            seq_cmp(&format!("{w} tiles"), &tiles, &r.tiles, &mut diffs);
            let ours_args = [o.qms.to_string()];
            for (i, (a, b)) in ours_args.iter().zip(&r.args).enumerate() {
                if a != b {
                    diffs.push(format!("{w} submit #{i}: {a} != {b}"));
                }
            }
        }
        let applying: Vec<_> = ALLOW.iter().filter(|a| glob(a.0.as_bytes(), shape.as_bytes())).collect();
        let mut used = vec![0usize; applying.len()];
        let mut left = 0;
        for d in &diffs {
            match applying.iter().position(|a| glob(a.1.as_bytes(), d.as_bytes())) {
                Some(i) => {
                    used[i] += 1;
                    *named.entry(applying[i].2).or_default() += 1;
                }
                None => {
                    left += 1;
                    failures.push(format!("gl_{shape}: {d}"));
                }
            }
        }
        for (a, n) in applying.iter().zip(&used) {
            if *n == 0 {
                failures.push(format!("gl_{shape}: stale named difference {} ({})", a.2, a.1));
            }
        }
        println!(
            "gl_{shape}: {} job(s), {packets} reference packets, {left} differences, {} named",
            refs.len(),
            used.iter().sum::<usize>()
        );
    }
    for (id, n) in &named {
        let reason = ALLOW.iter().find(|a| a.2 == *id).unwrap().3;
        println!("named {id} ({n}): {reason}");
    }
    assert!(failures.is_empty(), "{} difference(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn the_packet_table_decodes_the_references_own_packet_names() {
    /* every packet name the references print is a 7.1 packet of the table */
    let defs = packet_defs();
    for shape in SHAPES {
        for (k, r) in ref_jobs(shape).iter().enumerate() {
            for p in r.bcl.iter().chain(&r.rcl).chain(&r.tiles) {
                assert!(
                    defs.iter().any(|d| d.name == p.name),
                    "gl_{shape} job {k}: {} is not in the 7.1 table",
                    p.name
                );
            }
        }
    }
}
