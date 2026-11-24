// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/glue71.rs` (the GL core's `GlesDraw` into fill71's inputs) on
//! the six gl_test cases of P9: 5, 58, 59, 66, 67 and 102
//! (driver/tools/pvrlab/gl_test.c; their GL calls are
//! tools/v3d/pi5/src/gl_shapes.py's `gl_scissor`, `gl_scisbox`,
//! `gl_writemask`, `gl_viewport`, `gl_fbods`, `gl_colormask`, as
//! tools/v3d/pi5/README.md "The gl_test shapes" lists them).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_glue
//! ```
//!
//! Each shape is written once, as GL calls ([`Calls`]), and run twice:
//!
//! * **Model**: the GL state model of tests/v3d71_fill.rs and
//!   tests/v3d71_backend71_shapes.rs, which writes fill71's operations
//!   directly (one `Op` per glClear and per draw);
//! * **Core**: the GlesDraws the GL core hands the back end for those calls
//!   (src/gl/draw.rs: a glClear is kept pending with the state it was made
//!   under and goes out with the next draw, fused into it, or before it as
//!   a quad of its own when it is scissored or masks stencil bits, or alone
//!   at glFinish; `plan_clear` drops what the masks and the framebuffer
//!   rule out; the first vertex is in the array offsets), each turned into
//!   fill71's operations by glue71 and filled by `fill71::fill`.
//!
//! Checked: every flush of both fills, with the same jobs (target, clear,
//! history after, and every draw's state, arrays, primitive, uniforms and
//! clear quad; a draw's first vertex counted into its array addresses);
//! the jobs and the draws of each job are the counts of Mesa's references
//! tools/v3d/pi5/gl_*.clif, which tests/v3d71_backend71_shapes.rs pins (with
//! its one named gap, EMPTYBOX: the empty scissored clear of gl_scisbox's
//! third job is no draw here); and the refusals by name: case 59 on a window
//! whose depth/stencil the table keeps in the tile buffer only is zskeep71's
//! refusal, the Pi 4's name, and what glue71 itself refuses is named.
//!
//! The window here is not flipped (`flip_y` false), as Mesa's references
//! were made; a flipped window is checked on its own.
#![allow(dead_code)]

/// The GL core's back-end types, as every test reaches them: glue71 reads
/// `crate::backend`.
mod backend {
    pub use yakogl::backend::*;
}

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(clippy::if_same_then_else)]
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
#[path = "../src/v3d/glue71.rs"]
mod glue71;
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

use backend::*;
use ffi71::*;
use fill71::{ClearQuad, ClearReq, ColorBuffer, DrawIn, DsBuffer, FilledJob, Framebuffer, History, Op, Program};
use std::collections::BTreeMap;
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

/* ---- where the job's buffers sit (job71_check.c's gl mode, as the shapes test) ---- */

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

/* ---- our programs (tools/v3d/gltest-shaders71/ours_*.h, read as data) ---- */

const OURS_COLOR_UV: &str = include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h");
const OURS_CLEAR: &str = include_str!("../tools/v3d/gltest-shaders71/ours_clear.h");

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

/// One of our programs as fill71 takes it (tests/v3d71_backend71_shapes.rs
/// `our_program`), the uniform lists leaked to `'static`.
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
const QUAD_ATTR: attr71::Attr = float_attr(4, 16, G_VTX + 0x100);

fn programs() -> &'static (Program<'static>, Program<'static>) {
    static P: OnceLock<(Program<'static>, Program<'static>)> = OnceLock::new();
    P.get_or_init(|| {
        (our_program(OURS_COLOR_UV, "ours71_color_uv", G_CODE), our_program(OURS_CLEAR, "ours71_clear", G_CODE + 0x300))
    })
}

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &programs().1, attr: QUAD_ATTR, dummy_addr: G_VTX }
}

/* ---- the GL calls of a shape, run by either side ---------------------------- */

const COLOR: u32 = 1;
const DEPTH: u32 = 2;
const STENCIL: u32 = 4;

/// A framebuffer: colour textures and a depth/stencil renderbuffer, by id.
#[derive(Clone, Debug)]
struct Fb {
    colors: Vec<(u32, u32)>,
    ds: Option<(u32, u32)>,
    /// The table keeps the depth/stencil in memory; false: the tile buffer only.
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

/// The GL state the shapes set (each side reads it its own way).
#[derive(Clone, Debug)]
struct GlState {
    fb: Fb,
    color_mask: u8,
    depth_test: bool,
    depth_func: draw71::Compare,
    depth_write: bool,
    stencil_test: bool,
    stencil_func: draw71::Compare,
    stencil_ref: u32,
    stencil_write: u8,
    scissor_on: bool,
    scissor: [i32; 4],
    viewport: [i32; 4],
    clear_color: [f32; 4],
    clear_depth: f32,
    clear_stencil: u32,
    draw_buffers: u8,
}

impl GlState {
    fn new(fb: Fb) -> GlState {
        GlState {
            fb,
            color_mask: 0xf,
            depth_test: false,
            depth_func: draw71::Compare::Less,
            depth_write: true,
            stencil_test: false,
            stencil_func: draw71::Compare::Always,
            stencil_ref: 0,
            stencil_write: 0xff,
            scissor_on: false,
            scissor: [0, 0, G_SIZE as i32, G_SIZE as i32],
            viewport: [0, 0, G_SIZE as i32, G_SIZE as i32],
            clear_color: [0.0; 4],
            clear_depth: 1.0,
            clear_stencil: 0,
            draw_buffers: 1,
        }
    }
}

/// What both sides keep: each buffer's history by object id, the jobs.
#[derive(Default)]
struct Flushes {
    colors: BTreeMap<u32, bool>,
    ds: BTreeMap<u32, (Known71, Known71)>,
    jobs: Vec<FilledJob>,
    refusal: Option<fill71::Refusal>,
}

impl Flushes {
    fn history(&self, fb: &Fb) -> History {
        let mut h = History::NEW;
        for (n, (tex, _)) in fb.colors.iter().enumerate().take(h.colors.len()) {
            h.colors[n] = *self.colors.get(tex).unwrap_or(&false);
        }
        if let Some((rb, _)) = fb.ds {
            (h.z, h.s) = *self.ds.get(&rb).unwrap_or(&(Known71::Untouched, Known71::Untouched));
        }
        h
    }

    fn fill(&mut self, fb: &Fb, f: &Framebuffer<'_>, ops: &[Op<'_>]) {
        if self.refusal.is_some() {
            return;
        }
        match fill71::fill(f, record71::BCM2712_D0, &self.history(fb), &clear_quad(), ops) {
            Ok(jobs) => {
                if let Some(last) = jobs.last() {
                    for (n, (tex, _)) in fb.colors.iter().enumerate() {
                        self.colors.insert(*tex, last.after.colors[n]);
                    }
                    if let Some((rb, _)) = fb.ds {
                        self.ds.insert(rb, (last.after.z, last.after.s));
                    }
                }
                self.jobs.extend(jobs);
            }
            Err(r) => self.refusal = Some(r),
        }
    }
}

trait Calls {
    fn s(&mut self) -> &mut GlState;
    fn clear(&mut self, bits: u32);
    /// glDrawArrays(GL_TRIANGLES, first, 6) with gl_test's program.
    fn draw(&mut self, first: u32);
    /// glFinish.
    fn finish(&mut self);
    /// glBindFramebuffer (between flushes only, as the shapes do).
    fn bind(&mut self, fb: Fb) {
        self.s().fb = fb;
    }
    fn flushes(self) -> Flushes;
}

/* ---- the model: fill71's operations written directly (the shapes test's) ---- */

struct Model {
    s: GlState,
    ops: Vec<Op<'static>>,
    out: Flushes,
}

impl Model {
    fn new(fb: Fb) -> Model {
        Model { s: GlState::new(fb), ops: vec![], out: Flushes::default() }
    }

    fn draw_state(&self) -> draw71::DrawState {
        let s = &self.s;
        let mut st = draw71::DrawState::gl_default(G_SIZE, G_SIZE, false, s.fb.colors.len() as u32);
        st.viewport = s.viewport.map(|v| v as f32);
        for t in &mut st.targets {
            t.color_mask = s.color_mask;
        }
        (st.depth_test, st.depth_func, st.depth_write) = (s.depth_test, s.depth_func, s.depth_write);
        st.stencil_test = s.stencil_test;
        st.stencil_front.func = s.stencil_func;
        st.stencil_front.reference = s.stencil_ref;
        st.stencil_front.value_mask = 0xff;
        st.stencil_front.write_mask = u32::from(s.stencil_write);
        st.scissor = s.scissor_on.then_some(s.scissor);
        st
    }
}

impl Calls for Model {
    fn s(&mut self) -> &mut GlState {
        &mut self.s
    }

    fn clear(&mut self, bits: u32) {
        let s = &self.s;
        self.ops.push(Op::Clear(ClearReq {
            color: (bits & COLOR != 0).then_some(s.clear_color),
            draw_buffers: s.draw_buffers,
            color_mask: s.color_mask,
            depth: (bits & DEPTH != 0).then_some(s.clear_depth),
            depth_mask: s.depth_write,
            stencil: (bits & STENCIL != 0).then_some(s.clear_stencil),
            stencil_mask: s.stencil_write,
            scissor: s.scissor_on.then_some(s.scissor),
        }));
    }

    fn draw(&mut self, first: u32) {
        let st = self.draw_state();
        self.ops.push(Op::Draw(Box::new(DrawIn {
            state: st,
            line_width: 1.0,
            blend_color: [0.0; 4],
            program: &programs().0,
            attrs: &GL_ATTRS,
            dummy_addr: G_VTX,
            prim: prim71::Draw::arrays(GL_TRIANGLES, u64::from(first), 6),
            vs_words: &[],
            fs_words: &[],
            oq_addr: 0,
        })));
    }

    fn finish(&mut self) {
        let ops = std::mem::take(&mut self.ops);
        let fb = self.s.fb.clone();
        let colors: Vec<ColorBuffer> = (0..fb.colors.len())
            .map(|n| ColorBuffer {
                gl_format: fb.colors[n].1,
                address: g_rt(n as u32),
                memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
                height_in_ub_or_stride: 8,
            })
            .collect();
        let ds = fb.ds.map(|(_, fmt)| DsBuffer {
            gl_format: fmt,
            address: if fb.ds_kept { G_ZS } else { 0 },
            memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub: 8,
        });
        let f = Framebuffer { width: G_SIZE, height: G_SIZE, tile_alloc: G_TILE, flip_y: false, colors: &colors, ds };
        self.out.fill(&fb, &f, &ops);
    }

    fn flushes(self) -> Flushes {
        self.out
    }
}

/* ---- the core: the GL core's GlesDraws, through glue71 ------------------------- */

/// A glClear the GL core keeps until the next draw (src/gl/draw.rs
/// `PendingClear`), with the state it was made under (`clear_state`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClearState {
    color_mask: u8,
    depth_mask: bool,
    stencil_write: u8,
    scissor: Option<[i32; 4]>,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    state: ClearState,
    color: bool,
    depth: bool,
    stencil: bool,
    rgba: [f32; 4],
    depth_value: f32,
    stencil_value: u8,
}

/// `plan_clear`'s answer, as the GlesDraw's clear fields.
struct Plan {
    color: bool,
    depth: bool,
    stencil: bool,
    quad: bool,
    scissor: Option<[i32; 4]>,
}

struct Core {
    s: GlState,
    /// The table's depth/stencil format: the model's (`Some`), or picked by
    /// glue71 from the planes (`None`).
    ds_format_known: bool,
    pending: Option<Pending>,
    /// This flush's GlesDraws.
    draws: Vec<GlesDraw>,
    out: Flushes,
}

impl Core {
    fn new(fb: Fb) -> Core {
        Core { s: GlState::new(fb), ds_format_known: true, pending: None, draws: vec![], out: Flushes::default() }
    }

    fn clear_state(&self) -> ClearState {
        let s = &self.s;
        ClearState {
            color_mask: s.color_mask,
            depth_mask: s.depth_write,
            stencil_write: s.stencil_write,
            scissor: s.scissor_on.then_some(s.scissor),
        }
    }

    fn planes(&self) -> (bool, bool) {
        match self.s.fb.ds {
            Some((_, f)) => {
                let r = fill71::ds_format(f).unwrap();
                (r.depth, r.stencil)
            }
            None => (false, false),
        }
    }

    /// src/gl/draw.rs `plan_clear`.
    fn plan(&self, pc: &Pending) -> Option<Plan> {
        let (has_depth, has_stencil) = self.planes();
        let color = pc.color && pc.state.color_mask != 0;
        let depth = pc.depth && pc.state.depth_mask && has_depth;
        let stencil = pc.stencil && pc.state.stencil_write != 0 && has_stencil;
        if !color && !depth && !stencil {
            return None;
        }
        let mut p = Plan { color, depth, stencil, quad: false, scissor: None };
        if let Some(b) = pc.state.scissor {
            if b[2] <= 0 || b[3] <= 0 {
                return None;
            }
            if b[0] > 0 || b[1] > 0 || b[0] + b[2] < G_SIZE as i32 || b[1] + b[3] < G_SIZE as i32 {
                (p.quad, p.scissor) = (true, Some(b));
            }
        }
        if stencil && pc.state.stencil_write != 0xff {
            p.quad = true;
        }
        Some(p)
    }

    /// The target and depth/stencil fields (`resolve_target`, `resolve_targets`,
    /// `resolve_ds_storage`).
    fn base(&self) -> GlesDraw {
        let fb = &self.s.fb;
        let mut d = GlesDraw::ZERO;
        let n = fb.colors.len();
        d.target_gpu = u64::from(g_rt(0));
        (d.target_w, d.target_h) = (G_SIZE, G_SIZE);
        d.target_format = GLES_SURF_RGBA8888;
        d.target_alpha_bits = if fb.colors[0].1 == fill71::GL_RGB8 { 0 } else { 8 };
        for k in 0..n.min(GLES_MAX_TARGETS) {
            d.targets[k] =
                GlesDrawTarget { gpu: u64::from(g_rt(k as u32)), w: G_SIZE, h: G_SIZE, format: 0, out_shift: 0 };
        }
        d.num_targets = n as u8;
        d.samples = 1;
        d.flip_y = false;
        d.viewport = self.s.viewport;
        d.depth_range = [0.0, 1.0];
        if let Some((id, _)) = fb.ds {
            let (depth, stencil) = self.planes();
            d.ds_persist = true;
            d.ds_id = id;
            d.depth_id = if depth { id } else { 0 };
            d.stencil_id = if stencil { id } else { 0 };
            (d.ds_w, d.ds_h) = (G_SIZE, G_SIZE);
        }
        d
    }

    /// `apply_clear_plan` into `d`.
    fn apply(d: &mut GlesDraw, pc: &Pending, p: &Plan) {
        d.do_clear = p.color;
        d.clear_color_mask = if p.color { pc.state.color_mask } else { 0 };
        d.clear_target_mask = 0;
        d.clear_rgba = if p.color { pc.rgba } else { [0.0; 4] };
        d.do_clear_depth = p.depth;
        d.clear_depth = if p.depth { pc.depth_value } else { 1.0 };
        d.do_clear_stencil = p.stencil;
        d.clear_stencil = if p.stencil { pc.stencil_value } else { 0 };
        d.clear_stencil_write_mask = if p.stencil { pc.state.stencil_write } else { 0 };
    }

    /// `emit_pending_clear`: a clear-only GlesDraw.
    fn emit_pending(&mut self) {
        let Some(pc) = self.pending.take() else { return };
        let Some(p) = self.plan(&pc) else { return };
        let mut d = self.base();
        Self::apply(&mut d, &pc, &p);
        d.clear_quad = p.quad;
        (d.scissor, d.scissor_rect) = (p.scissor.is_some(), p.scissor.unwrap_or([0; 4]));
        d.stencil_write_mask = pc.state.stencil_write;
        d.stencil_back_write_mask = pc.state.stencil_write;
        d.color_write = p.color;
        d.color_mask = if p.color { 0xf } else { 0 };
        d.prim = GLES_PRIM_TRIANGLES;
        d.index_count = 0;
        self.draws.push(d);
    }

    fn ds_place(&self) -> glue71::DsPlace {
        let fb = &self.s.fb;
        glue71::DsPlace {
            gl_format: if self.ds_format_known { fb.ds.map(|d| d.1) } else { None },
            address: if fb.ds_kept { G_ZS } else { 0 },
            memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub: 8,
        }
    }
}

const FIRST_STRIDE: u32 = 40;

impl Calls for Core {
    fn s(&mut self) -> &mut GlState {
        &mut self.s
    }

    fn clear(&mut self, bits: u32) {
        /* settle_clear_state: a pending clear of other state goes out first */
        if self.pending.is_some_and(|p| p.state != self.clear_state()) {
            self.emit_pending();
        }
        let state = self.clear_state();
        let s = &self.s;
        let p = self.pending.get_or_insert(Pending {
            state,
            color: false,
            depth: false,
            stencil: false,
            rgba: [0.0; 4],
            depth_value: 1.0,
            stencil_value: 0,
        });
        if bits & COLOR != 0 {
            (p.color, p.rgba) = (true, s.clear_color);
        }
        if bits & DEPTH != 0 {
            (p.depth, p.depth_value) = (true, s.clear_depth);
        }
        if bits & STENCIL != 0 {
            (p.stencil, p.stencil_value) = (true, (s.clear_stencil & 0xff) as u8);
        }
    }

    fn draw(&mut self, first: u32) {
        /* settle_quad_clear: a quad clear is its own GlesDraw before the draw */
        if let Some(pc) = self.pending {
            if self.plan(&pc).is_some_and(|p| p.quad) {
                self.emit_pending();
            }
        }
        let s = self.s.clone();
        let mut d = self.base();
        if let Some(pc) = self.pending.take() {
            if let Some(p) = self.plan(&pc) {
                Self::apply(&mut d, &pc, &p);
            }
        }
        d.prim = GLES_PRIM_TRIANGLES;
        d.index_count = 6;
        d.instance_count = 1;
        d.line_width = 1.0;
        /* the first vertex is in the array offsets */
        for (k, (comps, off)) in [(4u8, 0u32), (4, 16), (2, 32)].into_iter().enumerate() {
            d.attribs[k] = GlesAttrib {
                vi_reg: k as u8,
                components: comps,
                format: GLES_ATTRIBF_FLOAT32,
                stride: FIRST_STRIDE,
                offset: off + first * FIRST_STRIDE,
                buffer_gpu: u64::from(G_VTX),
                ..GlesAttrib::ZERO
            };
        }
        d.num_attribs = 3;
        d.color_write = true;
        d.color_mask = s.color_mask;
        (d.depth_test, d.depth_func, d.depth_write) = (s.depth_test, s.depth_func as u32, s.depth_write);
        (d.scissor, d.scissor_rect) = (s.scissor_on, s.scissor);
        d.stencil = s.stencil_test;
        (d.stencil_func, d.stencil_ref, d.stencil_cmp_mask, d.stencil_write_mask) =
            (s.stencil_func as u32, s.stencil_ref as u8, 0xff, s.stencil_write);
        (d.stencil_back_func, d.stencil_back_ref, d.stencil_back_cmp_mask, d.stencil_back_write_mask) =
            (s.stencil_func as u32, s.stencil_ref as u8, 0xff, s.stencil_write);
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
        self.draws.push(d);
    }

    fn finish(&mut self) {
        self.emit_pending();
        let draws = std::mem::take(&mut self.draws);
        let fb = self.s.fb.clone();
        let mut frame: Option<glue71::Frame71> = None;
        let mut ops: Vec<Op<'static>> = vec![];
        for d in &draws {
            let r = (|| -> Result<(), fill71::Refusal> {
                let mut f = glue71::frame(d, self.ds_place())?;
                /* the table's colour images: the references' UIF layout */
                for c in &mut f.colors[..f.n] {
                    (c.memory_format, c.height_in_ub_or_stride) = (V3D_MEMORY_FORMAT_UIF_NO_XOR, 8);
                }
                match &frame {
                    None => frame = Some(f),
                    Some(g) => assert_eq!(g, &f, "one flush, one framebuffer"),
                }
                let input = if d.index_count != 0 {
                    let attrs: &'static [attr71::Attr] = Box::leak(glue71::attrs(d, &[])?.into_boxed_slice());
                    Some(glue71::DrawInputs {
                        program: &programs().0,
                        attrs,
                        vs_words: &[],
                        fs_words: &[],
                        dummy_addr: G_VTX,
                        oq_addr: 0,
                        indices: None,
                    })
                } else {
                    None
                };
                ops.extend(glue71::ops(d, frame.as_ref().unwrap(), input.as_ref())?);
                Ok(())
            })();
            if let Err(e) = r {
                self.out.refusal.get_or_insert(e);
                return;
            }
        }
        if let Some(f) = frame {
            self.out.fill(&fb, &f.framebuffer(G_TILE), &ops);
        }
    }

    fn flushes(self) -> Flushes {
        self.out
    }
}

/* ---- the shapes: gl_shapes.py's calls (as tests/v3d71_backend71_shapes.rs) ---- */

fn shape_scissor(g: &mut impl Calls) {
    g.s().clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    (g.s().scissor_on, g.s().scissor) = (true, [0, 0, 32, 32]);
    g.clear(COLOR);
    g.draw(0);
    g.finish();
}

fn shape_scisbox(g: &mut impl Calls) {
    for step in 0..3 {
        g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
        g.clear(COLOR);
        if step == 1 {
            g.draw(0);
        }
        (g.s().scissor_on, g.s().scissor) = (true, [16, 16, if step == 2 { 0 } else { 32 }, 32]);
        g.s().clear_color = [1.0, 0.0, 0.0, 1.0];
        g.clear(COLOR);
        g.finish();
        g.s().scissor_on = false;
    }
}

fn shape_writemask(g: &mut impl Calls) {
    use draw71::Compare::*;
    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR);
    g.s().color_mask = 0;
    g.s().clear_color = [1.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.s().color_mask = 0xf;
    g.finish();

    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.s().clear_depth = 1.0;
    g.s().depth_write = true;
    g.clear(COLOR | DEPTH);
    (g.s().depth_test, g.s().depth_func) = (true, Less);
    g.draw(0);
    g.s().depth_write = false;
    g.s().clear_depth = 1.0;
    g.clear(DEPTH);
    g.s().depth_write = true;
    g.draw(0);
    g.finish();
    g.s().depth_test = false;

    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.s().stencil_write = 0xff;
    g.s().clear_stencil = 0;
    g.clear(COLOR | STENCIL);
    g.s().stencil_write = 0x0f;
    g.s().clear_stencil = 0xff;
    g.clear(STENCIL);
    g.s().stencil_write = 0xff;
    g.s().stencil_test = true;
    (g.s().stencil_func, g.s().stencil_ref) = (Equal, 0xff);
    g.draw(0);
    g.finish();
    (g.s().stencil_func, g.s().stencil_ref) = (Equal, 0x0f);
    g.draw(0);
    g.finish();
    g.s().stencil_test = false;
}

fn shape_viewport(g: &mut impl Calls) {
    g.s().clear_color = [1.0, 1.0, 1.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.s().viewport = [0, 0, 32, 32];
    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.s().viewport = [0, 0, 64, 64];
    (g.s().scissor_on, g.s().scissor) = (true, [0, 0, 32, 32]);
    g.s().clear_color = [0.0, 1.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.s().scissor_on = false;
}

/// Case 67: texture 1 (RGBA), texture 2 (GL_RGB), renderbuffers 11 and 12
/// (DEPTH_COMPONENT16), 13 (STENCIL_INDEX8).
fn shape_fbods(g: &mut impl Calls) {
    use draw71::Compare::*;
    let tex = 1;
    g.bind(Fb::color(tex, fill71::GL_RGBA8));
    (g.s().depth_test, g.s().depth_func) = (true, Less);
    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.clear(COLOR | DEPTH);
    g.draw(0);
    g.draw(6);
    g.finish();
    g.s().stencil_test = true;
    (g.s().stencil_func, g.s().stencil_ref) = (Equal, 1);
    g.s().clear_stencil = 0;
    g.clear(COLOR | STENCIL);
    g.draw(0);
    g.finish();
    g.s().stencil_test = false;

    g.bind(Fb::color(2, fill71::GL_RGB8));
    g.s().depth_test = false;
    g.s().clear_color = [1.0, 0.0, 0.0, 0.25];
    g.clear(COLOR);
    g.finish();

    let fbo_a =
        Fb { colors: vec![(tex, fill71::GL_RGBA8)], ds: Some((11, fill71::GL_DEPTH_COMPONENT16)), ds_kept: true };
    let fbo_b =
        Fb { colors: vec![(tex, fill71::GL_RGBA8)], ds: Some((12, fill71::GL_DEPTH_COMPONENT16)), ds_kept: true };
    g.bind(fbo_a);
    (g.s().depth_test, g.s().depth_func) = (true, Less);
    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.s().clear_depth = 1.0;
    g.clear(COLOR | DEPTH);
    g.draw(0);
    g.finish();
    g.bind(fbo_b);
    g.draw(6);
    g.finish();
    g.s().depth_test = false;

    g.bind(Fb { colors: vec![(tex, fill71::GL_RGBA8)], ds: Some((13, fill71::GL_STENCIL_INDEX8)), ds_kept: true });
    g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
    g.s().clear_stencil = 0;
    g.clear(COLOR | STENCIL);
    (g.s().scissor_on, g.s().scissor) = (true, [16, 16, 32, 32]);
    g.s().clear_stencil = 1;
    g.clear(STENCIL);
    g.s().scissor_on = false;
    g.s().stencil_test = true;
    (g.s().stencil_func, g.s().stencil_ref) = (Equal, 0);
    g.draw(0);
    g.finish();
    g.s().stencil_test = false;
    for step in 0..2 {
        g.s().clear_color = [0.0, 0.0, 1.0, 1.0];
        g.s().clear_stencil = 0;
        g.clear(COLOR | STENCIL);
        g.draw(6);
        (g.s().scissor_on, g.s().scissor) = (true, [16, 16, 32, 32]);
        g.s().clear_stencil = 1;
        g.clear(STENCIL);
        g.s().scissor_on = false;
        g.s().clear_stencil = 0;
        if step == 1 {
            g.finish();
        }
        g.s().stencil_test = true;
        (g.s().stencil_func, g.s().stencil_ref) = (Equal, 0);
        g.draw(0);
        g.finish();
        g.s().stencil_test = false;
    }
}

fn shape_colormask(g: &mut impl Calls) {
    g.s().color_mask = 0xf;
    g.s().clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.s().color_mask = 0b1001;
    g.s().clear_color = [1.0, 1.0, 1.0, 0.5];
    g.clear(COLOR);
    g.finish();
    g.s().color_mask = 0xf;
    g.s().clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    (g.s().scissor_on, g.s().scissor) = (true, [0, 0, 32, 32]);
    g.s().color_mask = 0b0010;
    g.s().clear_color = [0.0, 1.0, 0.0, 0.0];
    g.clear(COLOR);
    g.finish();
    g.s().scissor_on = false;
    g.s().color_mask = 0xf;
    g.s().clear_color = [0.0, 0.0, 0.0, 1.0];
    g.clear(COLOR);
    g.finish();
    g.s().color_mask = 0b0100;
    g.s().clear_color = [0.0, 0.0, 1.0, 0.0];
    g.clear(COLOR);
    g.s().color_mask = 0xf;
    g.draw(0);
    g.finish();
}

/// (gl_test case, shape).
const CASES: [(u32, &str); 6] =
    [(5, "scissor"), (58, "scisbox"), (59, "writemask"), (66, "viewport"), (67, "fbods"), (102, "colormask")];

fn run(name: &str, g: &mut impl Calls) {
    match name {
        "scissor" => shape_scissor(g),
        "scisbox" => shape_scisbox(g),
        "writemask" => shape_writemask(g),
        "viewport" => shape_viewport(g),
        "fbods" => shape_fbods(g),
        "colormask" => shape_colormask(g),
        _ => unreachable!(),
    }
}

fn start_fb(name: &str) -> Fb {
    match name {
        "fbods" => Fb::color(0, fill71::GL_RGBA8),
        _ => Fb::window(),
    }
}

fn model(name: &str, fb: Fb) -> Flushes {
    let mut g = Model::new(fb);
    run(name, &mut g);
    g.flushes()
}

fn core(name: &str, fb: Fb, ds_format_known: bool) -> Flushes {
    let mut g = Core::new(fb);
    g.ds_format_known = ds_format_known;
    run(name, &mut g);
    g.flushes()
}

/// A job's descriptors as text, every pointer left out; a draw's first
/// vertex counted into its array addresses (the GL core's convention).
fn job_text(j: &FilledJob) -> String {
    let mut s = format!("target {:?}\nclear {:?}\nafter {:?}\n", j.target, j.clear, j.after);
    for d in &j.draws {
        let attrs: Vec<_> =
            d.attrs.iter().map(|a| (a.address + d.first * a.stride, a.stride, a.components, a.divisor)).collect();
        s += &format!(
            "draw state {:?}\n attrs {attrs:?} prim {} count {} indexed {} instances {} psiz {}\n quad {:?} \
             clear {} fs_words {:?} vs_words {:?}\n fs_u {:?} vs_u {:?} cs_u {:?} code {:#x} {:#x} {:#x}\n",
            d.state,
            d.prim,
            d.count,
            d.indexed,
            d.instances,
            d.vs_writes_psiz,
            d.quad,
            d.is_clear,
            d.fs_words,
            d.vs_words,
            d.fs_uniforms,
            d.vs_uniforms,
            d.cs_uniforms,
            d.fs.code,
            d.vs.code,
            d.cs.code,
        );
    }
    s
}

/* ---- the references' counts (gl_cmp.py's job split, as the shapes test) ---- */

const DRAWS: [&str; 4] =
    ["VERTEX_ARRAY_PRIMS", "VERTEX_ARRAY_INSTANCED_PRIMS", "INDEXED_PRIM_LIST", "INDEXED_INSTANCED_PRIM_LIST"];

/// The draws of each job of tools/v3d/pi5/gl_<shape>.clif.
fn ref_draws(shape: &str) -> Vec<usize> {
    let text = read(&format!("tools/v3d/pi5/gl_{shape}.clif"));
    let mut jobs = vec![];
    let mut cur: Option<usize> = None;
    for raw in text.lines() {
        if raw.starts_with("MESA: error") {
            continue;
        }
        let line = raw.strip_prefix("MESA: info:").map(|l| l.strip_prefix(' ').unwrap_or(l)).unwrap_or(raw);
        if line.starts_with("@createbuf") && cur.is_none() {
            cur = Some(0);
        }
        if let Some(c) = cur.as_mut() {
            if DRAWS.contains(&line.trim_end()) {
                *c += 1;
            }
        }
        if line.starts_with("@wait_render") {
            jobs.push(cur.take().unwrap());
        }
    }
    jobs
}

/* ---- the tests ------------------------------------------------------------------- */

#[test]
fn the_six_cases_fill_through_glue71_as_the_model_does() {
    for (case, name) in CASES {
        let m = model(name, start_fb(name));
        let c = core(name, start_fb(name), true);
        assert!(m.refusal.is_none(), "case {case} gl_{name}: the model refused: {:?}", m.refusal);
        if let Some(r) = &c.refusal {
            panic!("case {case} gl_{name}: refused through glue71: {r}");
        }
        assert_eq!(c.jobs.len(), m.jobs.len(), "case {case} gl_{name}: jobs");
        for (k, (a, b)) in c.jobs.iter().zip(&m.jobs).enumerate() {
            assert_eq!(job_text(a), job_text(b), "case {case} gl_{name} job {k}");
        }
        println!("case {case} gl_{name}: {} job(s), the model's", c.jobs.len());
    }
}

#[test]
fn the_jobs_and_draws_are_the_references_counts() {
    // the counts tests/v3d71_backend71_shapes.rs pins: fill71's split gives
    // the reference's jobs and draws, but where its named gap EMPTYBOX says
    // otherwise (gl_scisbox job 2: the empty scissored clear draws nothing)
    for (case, name) in CASES {
        let c = core(name, start_fb(name), true);
        assert!(c.refusal.is_none(), "case {case}: {:?}", c.refusal);
        let refs = ref_draws(name);
        assert_eq!(c.jobs.len(), refs.len(), "case {case} gl_{name}: jobs");
        for (k, (j, &r)) in c.jobs.iter().zip(&refs).enumerate() {
            let want = if (name, k) == ("scisbox", 2) { 0 } else { r };
            assert_eq!(j.draws.len(), want, "case {case} gl_{name} job {k}: draws");
        }
    }
}

#[test]
fn case_59_on_a_tile_only_window_is_refused_by_the_4_2_name() {
    // the Pi 4's window: the table keeps the depth/stencil in the tile buffer
    // only (DsPlace address 0); the draw that tests the stencil an earlier
    // job wrote is then zskeep71's refusal, the 4.2 table's string
    let c = core("writemask", Fb { ds_kept: false, ..Fb::window() }, true);
    let r = c.refusal.expect("case 59 on a tile-only window is refused");
    assert_eq!(r.module, "zskeep71");
    assert_eq!(
        r.reason,
        "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in"
    );
    // the model gives the same refusal
    let m = model("writemask", Fb { ds_kept: false, ..Fb::window() });
    assert_eq!(m.refusal.map(|r| (r.module, r.reason)), Some((r.module, r.reason)));
}

#[test]
fn the_other_five_cases_also_fill_on_a_tile_only_window() {
    for (case, name) in CASES {
        if case == 59 || case == 67 {
            continue; /* 67 draws into framebuffer objects only */
        }
        let c = core(name, Fb { ds_kept: false, ..Fb::window() }, true);
        assert!(c.refusal.is_none(), "case {case}: {:?}", c.refusal);
        for j in &c.jobs {
            assert_eq!(j.target.zs.address, 0, "case {case}: no Z/S image");
            assert!(!(j.clear.zs_load || j.clear.zs_store), "case {case}: nothing kept");
        }
    }
}

#[test]
fn case_67_fills_its_rgb_target_with_alpha_one() {
    // the 4.2 table's old refusal "a colour format with fewer than 8 alpha
    // bits" is gone (state.rs `job_state` draws them); on 7.1 the GL_RGB
    // texture is fill71's RGB8: loaded with alpha 1, cleared to alpha 1
    let c = core("fbods", start_fb("fbods"), true);
    assert!(c.refusal.is_none(), "{:?}", c.refusal);
    let rgb: Vec<_> = c.jobs.iter().filter(|j| j.target.rt[0].force_alpha_1).collect();
    assert_eq!(rgb.len(), 1, "one job draws into the GL_RGB texture");
    assert_eq!(rgb[0].target.rt[0].format, V3D_OUTPUT_IMAGE_FORMAT_RGBA8);
    let red = rt71::clear_words71(
        &rt71::rt_format71(fill71::GL_RGB8).unwrap(),
        rt71::ClearColor::Float([1.0, 0.0, 0.0, 1.0]),
    )
    .unwrap();
    assert_eq!(rgb[0].clear.color[0], red.words, "red, alpha 1 (not the 0.25 asked)");
}

#[test]
fn the_planes_pick_the_depth_stencil_format_when_the_table_names_none() {
    // without the table's format, D16 renderbuffers become D24 images and
    // the stencil-only one a D24S8 moved as stencil: the same jobs but for
    // the depth type and the image format
    for (case, name) in CASES {
        let fb = start_fb(name);
        let (known, picked) = (core(name, fb.clone(), true), core(name, fb, false));
        assert!(picked.refusal.is_none(), "case {case}: {:?}", picked.refusal);
        assert_eq!(picked.jobs.len(), known.jobs.len(), "case {case}");
        for (a, b) in picked.jobs.iter().zip(&known.jobs) {
            assert_eq!(a.draws.len(), b.draws.len(), "case {case}");
            if b.target.depth_type == V3D_INTERNAL_TYPE_DEPTH_16 {
                assert_eq!(a.target.depth_type, V3D_INTERNAL_TYPE_DEPTH_24, "case {case}");
                assert_eq!(a.target.zs.format, V3D_OUTPUT_IMAGE_FORMAT_D24S8, "case {case}");
            } else {
                assert_eq!(a.target, b.target, "case {case}");
            }
            assert_eq!(a.clear, b.clear, "case {case}");
        }
    }
    assert_eq!(glue71::ds_gl_format(true, true), Some(fill71::GL_DEPTH24_STENCIL8));
    assert_eq!(glue71::ds_gl_format(true, false), Some(fill71::GL_DEPTH_COMPONENT24));
    assert_eq!(glue71::ds_gl_format(false, true), Some(fill71::GL_STENCIL_INDEX8));
    assert_eq!(glue71::ds_gl_format(false, false), None);
}

/* ---- the mapping, field by field ---------------------------------------------- */

/// A window draw of gl_test's quad, as the GL core resolves it.
fn window_draw() -> GlesDraw {
    let mut c = Core::new(Fb::window());
    c.draw(0);
    c.draws.remove(0)
}

fn window_frame(d: &GlesDraw) -> glue71::Frame71 {
    glue71::frame(d, glue71::DsPlace { gl_format: None, address: G_ZS, memory_format: 4, height_in_ub: 8 }).unwrap()
}

#[test]
fn the_frame_is_the_gl_cores_raster_surface_by_default() {
    let mut d = window_draw();
    d.target_w = 63;
    let f = window_frame(&d);
    assert_eq!((f.width, f.height, f.n), (63, 64, 1), "the attachments' intersection");
    let c = f.colors[0];
    assert_eq!((c.gl_format, c.address, c.memory_format), (fill71::GL_RGBA8, g_rt(0), V3D_MEMORY_FORMAT_RASTER));
    assert_eq!(c.height_in_ub_or_stride, 64 * 4, "draw_pitch: the pair-padded width, 4 bytes a pixel");
    d.target_pitch = 80;
    assert_eq!(window_frame(&d).colors[0].height_in_ub_or_stride, 320, "a view into a wider buffer");
    let ds = f.ds.unwrap();
    assert_eq!((ds.gl_format, ds.address), (fill71::GL_DEPTH24_STENCIL8, G_ZS), "the window's D24S8");
    d.target_alpha_bits = 0;
    assert_eq!(window_frame(&d).colors[0].gl_format, fill71::GL_RGB8);
    d.target_alpha_bits = 1;
    assert_eq!(window_frame(&d).colors[0].gl_format, fill71::GL_RGBA8, "RGB5_A1 drawn as its RGBA8 storage");
    d.target_format = GLES_SURF_SRGB8_A8;
    assert_eq!(window_frame(&d).colors[0].gl_format, fill71::GL_SRGB8_ALPHA8);
    d.ds_persist = false;
    assert_eq!(window_frame(&d).ds, None, "a colour-only framebuffer");
}

#[test]
fn a_clear_maps_its_masks_values_and_scissor_box() {
    let mut d = GlesDraw::ZERO;
    assert_eq!(glue71::clear(&d), None, "no clear");
    d.num_targets = 3;
    (d.do_clear, d.clear_rgba) = (true, [0.25, 0.5, 0.75, 1.0]);
    (d.do_clear_stencil, d.clear_stencil, d.stencil_write_mask) = (true, 7, 0x3c);
    d.clear_quad = true;
    (d.scissor, d.scissor_rect) = (true, [1, 2, 3, 4]);
    let c = glue71::clear(&d).unwrap();
    assert_eq!(c.color, Some([0.25, 0.5, 0.75, 1.0]));
    assert_eq!((c.draw_buffers, c.color_mask), (0b111, 0xf), "0 is every draw buffer, every channel");
    assert_eq!((c.depth, c.depth_mask), (None, true));
    assert_eq!((c.stencil, c.stencil_mask), (Some(7), 0x3c), "0 is the draw's stencil write mask");
    assert_eq!(c.scissor, Some([1, 2, 3, 4]));
    (d.clear_target_mask, d.clear_color_mask, d.clear_stencil_write_mask) = (0b010, 0b0101, 0x0f);
    (d.do_clear_depth, d.clear_depth) = (true, 0.5);
    d.clear_quad = false;
    let c = glue71::clear(&d).unwrap();
    assert_eq!((c.draw_buffers, c.color_mask, c.stencil_mask), (0b010, 0b0101, 0x0f));
    assert_eq!(c.depth, Some(0.5));
    assert_eq!(c.scissor, None, "a fused clear is whole; the draw's scissor is the draw's");
}

#[test]
fn a_draw_maps_its_state() {
    use draw71::{BlendFactor, BlendMode, Compare, Cull, StencilOp};
    let mut d = window_draw();
    d.flip_y = true;
    (d.viewport, d.depth_range) = ([1, 2, 30, 40], [0.25, 0.75]);
    (d.cull, d.front_cw) = (GLES_CULL_CW, false);
    (d.polygon_offset, d.polygon_offset_factor, d.polygon_offset_units) = (true, 2.0, 3.0);
    (d.depth_test, d.depth_func, d.depth_write) = (true, GLES_CMP_GEQUAL, false);
    d.stencil = true;
    (d.stencil_sfail, d.stencil_dpfail, d.stencil_dppass) = (GLES_SOP_ZERO, GLES_SOP_INCR_WRAP, GLES_SOP_INVERT);
    (d.blend, d.target_alpha_bits) = (true, 0);
    (d.blend_src_rgb, d.blend_dst_rgb) = (4, 0x14); /* DST_ALPHA, INV_DST_ALPHA */
    (d.blend_src_alpha, d.blend_dst_alpha) = (3, 0x13); /* SRC_ALPHA, INV_SRC_ALPHA */
    (d.blend_eq_rgb, d.blend_eq_alpha) = (2, 4); /* REVERSE_SUBTRACT, MAX */
    d.color_mask = 0b1011;
    let f = window_frame(&d);
    let st = glue71::draw_state(&d, &f).unwrap();
    assert_eq!((st.viewport, st.depth_range, st.flip_y), ([1.0, 2.0, 30.0, 40.0], [0.25, 0.75], true));
    assert_eq!((st.cull, st.front_cw), (Cull::Back, false), "CW culled, CCW in front: the back");
    d.front_cw = true;
    assert_eq!(glue71::draw_state(&d, &f).unwrap().cull, Cull::Front);
    assert_eq!((st.polygon_offset, st.offset_factor, st.offset_units), (true, 2.0, 3.0));
    assert_eq!((st.depth_test, st.depth_func, st.depth_write), (true, Compare::GEqual, false));
    let sf = st.stencil_front;
    assert_eq!((sf.sfail, sf.zfail, sf.zpass), (StencilOp::Zero, StencilOp::IncWrap, StencilOp::Invert));
    assert!(st.stencil_two_sided, "the back face keeps KEEP: the faces differ");
    let b = st.targets[0].blend;
    assert!(st.targets[0].blend_enable);
    assert_eq!((b.rgb_src, b.rgb_dst), (BlendFactor::One, BlendFactor::Zero), "no alpha: dst alpha reads 1");
    assert_eq!((b.alpha_src, b.alpha_dst), (BlendFactor::SrcAlpha, BlendFactor::InvSrcAlpha));
    assert_eq!((b.rgb_mode, b.alpha_mode), (BlendMode::RSub, BlendMode::Max));
    assert_eq!(st.targets[0].color_mask, 0b1011);
    d.color_write = false;
    assert_eq!(glue71::draw_state(&d, &f).unwrap().targets[0].color_mask, 0, "color_write off: no channel");
}

#[test]
fn a_flipped_window_fills_with_its_scissor_flipped() {
    let mut d = window_draw();
    d.flip_y = true;
    (d.scissor, d.scissor_rect) = (true, [0, 0, 32, 16]);
    let f = window_frame(&d);
    let attrs = glue71::attrs(&d, &[]).unwrap();
    let ops = glue71::ops(
        &d,
        &f,
        Some(&glue71::DrawInputs {
            program: &programs().0,
            attrs: &attrs,
            vs_words: &[],
            fs_words: &[],
            dummy_addr: G_VTX,
            oq_addr: 0,
            indices: None,
        }),
    )
    .unwrap();
    let jobs = fill71::fill(&f.framebuffer(G_TILE), record71::BCM2712_D0, &History::NEW, &clear_quad(), &ops).unwrap();
    let s = jobs[0].draws[0].state;
    assert!(s.scissor);
    assert_eq!((s.scissor_minx, s.scissor_miny, s.scissor_maxx, s.scissor_maxy), (0, 48, 32, 64));
    assert!(s.vp.scale[1] < 0.0, "the viewport flips");
}

#[test]
fn prim_and_attrs_map_the_call() {
    let mut d = window_draw();
    assert_eq!(glue71::gl_mode(GLES_PRIM_LINE_LOOP).unwrap(), prim71::GL_LINE_LOOP);
    assert_eq!(glue71::gl_mode(GLES_PRIM_POINTS).unwrap(), prim71::GL_POINTS);
    assert_eq!(glue71::gl_mode(GLES_PRIM_TRI_FAN).unwrap(), prim71::GL_TRIANGLE_FAN);
    let p = glue71::prim(&d, None).unwrap();
    assert_eq!((p.mode, p.first, p.count, p.instances, p.indices), (GL_TRIANGLES, 0, 6, 1, None));
    (d.index_buf_gpu, d.index_size, d.instance_count) = (0x5000, 2, 3);
    let p = glue71::prim(&d, None).unwrap();
    let i = p.indices.unwrap();
    assert_eq!((i.gl_type, i.addr, i.size, p.instances), (prim71::GL_UNSIGNED_SHORT, 0x5000, 12, 3));
    let at = glue71::IndexPlace { addr: 0x6000, size: 64 };
    assert_eq!(glue71::prim(&d, Some(at)).unwrap().indices.unwrap().addr, 0x6000, "the table's copy");
    // a staged array reads from the table's copy; the others at buffer + offset
    let x = [7u8; 16];
    d.attribs[1].cpu = x.as_ptr().cast();
    d.attribs[2].vi_reg = 5;
    (d.attribs[2].per_instance, d.attribs[2].divisor) = (1, 0);
    let a = glue71::attrs(&d, &[0, 0x7000, 0]).unwrap();
    assert_eq!(a.len(), 6, "up to the highest location");
    assert_eq!((a[0].buffer_addr, a[1].buffer_addr, a[5].buffer_addr), (G_VTX, 0x7000, G_VTX + 32));
    assert!(!a[2].enabled && !a[3].enabled && !a[4].enabled, "locations no array names");
    assert_eq!(a[5].divisor, 1, "a per-instance array's divisor 0 is 1");
    let e = glue71::attrs(&d, &[]).unwrap_err();
    assert_eq!((e.module, e.reason.as_str()), ("glue71", glue71::GLUE71_REFUSALS[3]));
}

#[test]
fn what_glue71_does_not_carry_is_refused_by_name() {
    let base = window_draw();
    let f = window_frame(&base);
    let attrs = glue71::attrs(&base, &[]).unwrap();
    let input = glue71::DrawInputs {
        program: &programs().0,
        attrs: &attrs,
        vs_words: &[],
        fs_words: &[],
        dummy_addr: G_VTX,
        oq_addr: 0,
        indices: None,
    };
    assert_eq!(glue71::ops(&base, &f, Some(&input)).unwrap().len(), 1, "the draw itself is carried");
    let refused = |edit: &dyn Fn(&mut GlesDraw)| -> fill71::Refusal {
        let mut d = base;
        edit(&mut d);
        match glue71::frame(&d, glue71::DsPlace::default()) {
            Err(e) => e,
            Ok(_) => glue71::ops(&d, &f, Some(&input)).unwrap_err(),
        }
    };
    let named = |e: fill71::Refusal, k: usize| {
        assert_eq!((e.module, e.reason.as_str()), ("glue71", glue71::GLUE71_REFUSALS[k]));
    };
    named(refused(&|d| d.target_format = GLES_SURF_RGB565), 0);
    named(refused(&|d| d.target_format = GLES_SURF_XRGB8888), 1);
    named(refused(&|d| (d.target_gpu, d.ds_persist) = (0, false)), 2);
    named(refused(&|d| d.target_gpu = 1 << 32), 3);
    named(refused(&|d| d.num_textures = 1), 4);
    named(refused(&|d| d.alpha_to_coverage = true), 5);
    named(refused(&|d| d.targets[0].out_shift = 1), 6);
    named(refused(&|d| d.index_count2 = 3), 7);
    named(refused(&|d| d.instance_id = 2), 8);
    let z = [0.0f32; 4];
    named(refused(&|d| d.depth_texels = z.as_ptr()), 9);
    named(refused(&|d| d.prim = 9), 10);
    named(refused(&|d| d.cull = 7), 11);
    named(refused(&|d| d.depth_func = 8), 12);
    named(refused(&|d| d.stencil_sfail = 8), 13);
    named(refused(&|d| (d.blend, d.blend_src_rgb) = (true, 0x30)), 14);
    named(glue71::ops(&base, &f, None).unwrap_err(), 15);
    // multisampling is draw71's refusal, for a clear as for a draw
    let e = refused(&|d| (d.samples, d.index_count, d.do_clear) = (4, 0, true));
    assert_eq!((e.module, e.reason.as_str()), ("draw71", draw71::Refusal71::Multisample.message()));
    // a clear alone needs no program and maps whatever the draw state holds
    let mut d = base;
    (d.index_count, d.do_clear, d.depth_func) = (0, true, 99);
    assert!(matches!(glue71::ops(&d, &f, None).unwrap().as_slice(), [Op::Clear(_)]));
}

#[test]
fn the_refusal_names_are_distinct_and_7_1s() {
    let mut seen = std::collections::BTreeSet::new();
    for r in glue71::GLUE71_REFUSALS {
        assert!(r.starts_with("V3D 7.1: "), "{r}");
        assert!(seen.insert(r), "twice: {r}");
        assert!(!fill71::FILL71_REFUSALS.contains(&r), "fill71's own: {r}");
    }
}
