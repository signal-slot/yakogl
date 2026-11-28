// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `fill71::fill_each`, the no-allocator core of the 7.1 fill
//! (docs/notes/v3d71-nostd.md section 4, item N5), on the op lists of the
//! six gl_test cases the Pi 5's first board run takes (P9: 5, 58, 59, 66,
//! 67 and 102; driver/tools/pvrlab/gl_test.c `case_scissor`,
//! `case_clearscis`, `case_clearmask`, `case_clearvp`, `case_fbods`,
//! `case_maskclear`).
//!
//! Each case is written as its flushes (one per `glFinish`, or per
//! framebuffer bind), each flush the `fill71::Op`s the GL calls make, as
//! tests/v3d71_case5_record.rs, v3d71_case66_record.rs and
//! v3d71_case102_record.rs write cases 5, 66 and 102. The programs are our
//! 7.1 link of gl_test's `color_uv` pair and of the clear program. Every
//! flush runs three ways:
//!
//!  * `fill71::fill` (the host-test adapter, a `Vec<FilledJob>`);
//!  * `fill_each` with a scratch of one slot an op: the same jobs, or the
//!    same refusal, and never `Full`;
//!  * `fill_each` with a two-draw scratch: the same jobs when every job of
//!    the flush has at most two draws; otherwise `Full` at the op of the
//!    first job's third draw, after exactly the jobs `fill` gives before
//!    that job. A refused flush gives the same refusal (or `Full` first,
//!    when the job that would refuse later has more than two draws).
//!
//! A job is compared as what `v3d_job71` reads: its target, clear and
//! history, and for each draw the C structs behind `ffi()` with the uniform
//! lists, GL blocks and attribute arrays they point at (the pointers
//! themselves differ: the adapter's point at its own copies).
//!
//! Each case's outcome per flush is pinned ([`EXPECT`]); case 67's frame 7
//! (a draw, a scissored stencil clear quad and a second draw in one job) is
//! the one that does not fit two slots.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_fill_each
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d71", feature = "host-test"))]
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

use draw71::{Compare, StencilOp};
use fill71::{
    ClearQuad, ClearReq, ColorBuffer, DrawIn, DrawView, DsBuffer, FillEach, FilledJob, Framebuffer, History, JobView,
    Op, Program,
};
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::link71::link_sources71;
use yakogl::shaderc::{LodKey, ShcShader, ShcV3dUniform, ShcV3dVs};
use yakogl::v3d::backend::{CLEAR_FS, CLEAR_VS};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

static COLOR_UV_VS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.vert");
static COLOR_UV_FS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.frag");

/// Stand-ins for the GPU addresses the table places at run time.
const CODE_BASE: u32 = 0x0020_0000;
const CLEAR_BASE: u32 = 0x0021_0000;
const VTX: u32 = 0x0030_0000;
const TILE: u32 = 0x0040_0000;
/// gl_test's window colour buffer.
const RT0: u32 = 0x0100_0000;
/// Case 67's colour textures: the RGBA one and the RGB one.
const TEX: u32 = 0x0300_0000;
const TEX_RGB: u32 = 0x0310_0000;

/// TW, TH (gl_test.c:1088-1089).
const SIZE: u32 = 64;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLES: u32 = 4;

/* ---- the programs (as tests/v3d71_case5_record.rs links them) ------------ */

fn stream(u: *const ShcV3dUniform, n: u32) -> &'static [uniform71::Word] {
    if n == 0 {
        return &[];
    }
    // SAFETY: the linked program's stream of `n` entries, alive while the
    // keep is open (the caller holds it); copied out before it closes.
    let v: Vec<uniform71::Word> = unsafe { core::slice::from_raw_parts(u, n as usize) }
        .iter()
        .map(|u| uniform71::Word { contents: u.contents, data: u.data })
        .collect();
    Box::leak(v.into_boxed_slice())
}

fn c_source(s: &'static [u8]) -> &'static str {
    core::str::from_utf8(s.strip_suffix(b"\0").expect("NUL-terminated")).expect("UTF-8")
}

/// One process-wide lock: the link's `compile_rt_f32` key is global.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn link(vs: &str, fs: &str, base: u32, discards: bool) -> Program<'static> {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _keep = store::begin_keep().expect("keep");
    let sh: ShcShader = store::with_scratch(|a| {
        link_sources71(vs.as_bytes(), None, LodKey::NONE, fs.as_bytes(), &[], a, store::keep()).map(|r| r.0)
    })
    .expect("scratch")
    .unwrap_or_else(|e| panic!("link71: {e:?}"));
    assert_eq!(sh.v3d.linked, 1);
    program(&sh, base, discards)
}

fn vdesc(base: u32, v: &ShcV3dVs) -> record71::VertexDesc {
    record71::VertexDesc {
        prog: record71::ProgramDesc {
            code: base + v.code_off,
            uniforms: 0,
            threads: u32::from(v.threads),
            single_seg: v.single_seg != 0,
            propagate_nans: false,
        },
        vpm_input_size: v.vpm_input_size,
        vpm_output_size: v.vpm_output_size,
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: v.uses_vid != 0,
        uses_iid: v.uses_iid != 0,
        uses_base_instance: v.uses_biid != 0,
        uses_base_vertex: false,
        uses_draw_index: false,
    }
}

fn program(sh: &ShcShader, base: u32, discards: bool) -> Program<'static> {
    let p = &sh.v3d;
    let fs = record71::FragmentDesc {
        prog: record71::ProgramDesc {
            code: base + p.fs.code_off,
            uniforms: 0,
            threads: u32::from(p.fs.threads),
            single_seg: p.fs.single_seg != 0,
            propagate_nans: false,
        },
        writes_z: p.fs.writes_z != 0,
        disable_ez: p.fs.disable_ez != 0,
        num_varyings: p.fs.num_inputs,
        uses_center_w: p.fs.uses_center_w != 0,
        uses_pid: p.fs.uses_pid != 0,
        lock_scoreboard_on_first_thrsw: p.fs.lock_scoreboard_on_first_thrsw != 0,
        uses_implicit_point_line_varyings: p.fs.uses_implicit_point_line_varyings != 0,
        sample_rate_shading: false,
    };
    let desc =
        record71::DrawDesc { fs, vs: vdesc(base, &p.vs), cs: vdesc(base, &p.cs), point_size: p.vs.writes_psiz != 0 };
    Program {
        desc,
        cs_reads: p.cs.vattr_sizes,
        vs_reads: p.vs.vattr_sizes,
        cs_vcm_cache_size: p.cs.vcm_cache_size,
        vs_vcm_cache_size: p.vs.vcm_cache_size,
        separate_segments: p.vs.separate_segments != 0 || p.cs.separate_segments != 0,
        varyings: fill71::VaryingFlags {
            flat: [p.fs.flat_shade_flags, p.fs.flat_shade_flags_hi],
            noperspective: [p.fs.noperspective_flags, p.fs.noperspective_flags_hi],
            centroid: [p.fs.centroid_flags, p.fs.centroid_flags_hi],
        },
        discards,
        fs_uniforms: stream(p.fs.uniforms, p.fs.num_uniforms),
        vs_uniforms: stream(p.vs.uniforms, p.vs.num_uniforms),
        cs_uniforms: stream(p.cs.uniforms, p.cs.num_uniforms),
    }
}

fn header_define(rel: &str, name: &str) -> u32 {
    let h = read(rel);
    let l = h
        .lines()
        .find_map(|l| l.strip_prefix(&format!("#define {name} ")).map(str::to_string))
        .unwrap_or_else(|| panic!("{rel}: no {name}"));
    l.split("/*").next().unwrap().trim().parse().unwrap()
}

/// (gl_test's `color_uv` program, the clear program).
fn programs() -> &'static (Program<'static>, Program<'static>) {
    static P: std::sync::OnceLock<(Program<'static>, Program<'static>)> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let d_gl = header_define("tools/v3d/gltest-shaders71/ours_color_uv.h", "OURS71_COLOR_UV_FS_USES_DISCARD") != 0;
        let d_cl = header_define("tools/v3d/gltest-shaders71/ours_clear.h", "OURS71_CLEAR_FS_USES_DISCARD") != 0;
        (
            link(COLOR_UV_VS, COLOR_UV_FS, CODE_BASE, d_gl),
            link(c_source(CLEAR_VS), c_source(CLEAR_FS), CLEAR_BASE, d_cl),
        )
    })
}

/* ---- the GL calls --------------------------------------------------------- */

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

/// cuv_attribs (gl_test.c:1400-1409).
static CUV_ATTRS: [attr71::Attr; 3] =
    [float_attr(4, 40, VTX), float_attr(4, 40, VTX + 16), float_attr(2, 40, VTX + 32)];
const QUAD_ATTR: attr71::Attr = float_attr(4, 16, VTX + 0x100);

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &programs().1, attr: QUAD_ATTR, dummy_addr: VTX }
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const R: u8 = 0b0001;
const G: u8 = 0b0010;
const B: u8 = 0b0100;
const A: u8 = 0b1000;

/// The GL state a draw reads that the cases change.
#[derive(Clone, Copy)]
struct Gl {
    scissor: Option<[i32; 4]>,
    depth: Option<Compare>,
    depth_write: bool,
    /// glStencilFunc(EQUAL, ref, 0xff) with KEEP everywhere.
    stencil_equal: Option<u32>,
}

const GL: Gl = Gl { scissor: None, depth: None, depth_write: true, stencil_equal: None };

/// `glDrawArrays(GL_TRIANGLES, first, 6)` with the `color_uv` program.
fn draw(gl: Gl, first: u64) -> Op<'static> {
    let mut st = draw71::DrawState::gl_default(SIZE, SIZE, false, 1);
    st.viewport = [0.0, 0.0, SIZE as f32, SIZE as f32];
    st.scissor = gl.scissor;
    if let Some(f) = gl.depth {
        st.depth_test = true;
        st.depth_func = f;
    }
    st.depth_write = gl.depth_write;
    if let Some(r) = gl.stencil_equal {
        st.stencil_test = true;
        for f in [&mut st.stencil_front, &mut st.stencil_back] {
            f.func = Compare::Equal;
            f.reference = r;
            f.value_mask = 0xff;
            f.write_mask = 0xff;
            (f.sfail, f.zfail, f.zpass) = (StencilOp::Keep, StencilOp::Keep, StencilOp::Keep);
        }
    }
    Op::Draw(Box::new(DrawIn {
        state: st,
        line_width: 1.0,
        blend_color: [0.0; 4],
        program: &programs().0,
        attrs: &CUV_ATTRS,
        dummy_addr: VTX,
        prim: prim71::Draw::arrays(GL_TRIANGLES, first, 6),
        vs_words: &[],
        fs_words: &[],
        oq_addr: 0,
    }))
}

fn clear(c: ClearReq) -> Op<'static> {
    Op::Clear(c)
}

/// `glClear(GL_COLOR_BUFFER_BIT)` to `c`.
fn color(c: [f32; 4]) -> ClearReq {
    ClearReq::color(c)
}

/// A clear of no colour (depth and/or stencil only).
fn no_color() -> ClearReq {
    ClearReq { color: None, ..ClearReq::color(BLACK) }
}

/* ---- the framebuffers ----------------------------------------------------- */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fb {
    /// gl_test's 64x64 window: RGBA8 + D24S8.
    Window,
    /// Case 102's target as tests/v3d71_case102_record.rs writes it: RGBA8,
    /// no depth/stencil.
    ColorOnly,
    /// Case 67: fbo_a, the RGBA texture alone.
    FboA,
    /// Case 67: the GL_RGB texture.
    FboRgb,
    /// Case 67: fbo_a with depth_a, and fbo_b (the same texture) with depth_b.
    FboADepth,
    FboBDepth,
    /// Case 67: fbo_a with the STENCIL_INDEX8 renderbuffer.
    FboAStencil,
}

fn color_buffer(gl_format: u32, address: u32) -> ColorBuffer {
    ColorBuffer { gl_format, address, memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR, height_in_ub_or_stride: 8 }
}

/// A depth/stencil buffer as table71 gives it today: in the tile buffer
/// only (`glue71::DsPlace` at address 0, src/v3d/table71.rs module docs),
/// so a read across a flush is zskeep71's refusal.
fn ds_tile_only(gl_format: u32) -> Option<DsBuffer> {
    Some(DsBuffer { gl_format, address: 0, memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR, height_in_ub: 8 })
}

fn framebuffer(fb: Fb) -> Framebuffer<'static> {
    let (c, ds) = match fb {
        Fb::Window => (color_buffer(fill71::GL_RGBA8, RT0), ds_tile_only(fill71::GL_DEPTH24_STENCIL8)),
        Fb::ColorOnly => (color_buffer(fill71::GL_RGBA8, RT0), None),
        Fb::FboA => (color_buffer(fill71::GL_RGBA8, TEX), None),
        Fb::FboRgb => (color_buffer(fill71::GL_RGB8, TEX_RGB), None),
        Fb::FboADepth | Fb::FboBDepth => {
            (color_buffer(fill71::GL_RGBA8, TEX), ds_tile_only(fill71::GL_DEPTH_COMPONENT16))
        }
        Fb::FboAStencil => (color_buffer(fill71::GL_RGBA8, TEX), ds_tile_only(fill71::GL_STENCIL_INDEX8)),
    };
    let colors: &'static [ColorBuffer] = Box::leak(Box::new([c]));
    Framebuffer { width: SIZE, height: SIZE, tile_alloc: TILE, flip_y: false, colors, ds }
}

/* ---- the six cases' flushes ----------------------------------------------- */

type Flush = (Fb, Vec<Op<'static>>);

/// Case 5, `scissor` (gl_test.c:2357-2405).
fn case5() -> Vec<Flush> {
    let bx = [0, 0, 32, 32];
    vec![
        (Fb::Window, vec![clear(color(BLACK))]),
        (
            Fb::Window,
            vec![clear(ClearReq { scissor: Some(bx), ..color(BLACK) }), draw(Gl { scissor: Some(bx), ..GL }, 0)],
        ),
    ]
}

/// Case 58, `case_clearscis`.
fn case58() -> Vec<Flush> {
    let bx = Some([16, 16, 32, 32]);
    vec![
        (Fb::Window, vec![clear(color(BLUE)), clear(ClearReq { scissor: bx, ..color(RED) })]),
        (Fb::Window, vec![clear(color(BLUE)), draw(GL, 0), clear(ClearReq { scissor: bx, ..color(RED) })]),
        (Fb::Window, vec![clear(color(BLUE)), clear(ClearReq { scissor: Some([16, 16, 0, 32]), ..color(RED) })]),
    ]
}

/// Case 59, `case_clearmask`.
fn case59() -> Vec<Flush> {
    let less = Gl { depth: Some(Compare::Less), ..GL };
    vec![
        (Fb::Window, vec![clear(color(BLUE)), clear(ClearReq { color_mask: 0, ..color(RED) })]),
        (
            Fb::Window,
            vec![
                clear(ClearReq { depth: Some(1.0), ..color(BLUE) }),
                draw(less, 0),
                clear(ClearReq { depth: Some(1.0), depth_mask: false, ..no_color() }),
                draw(less, 0),
            ],
        ),
        (
            Fb::Window,
            vec![
                clear(ClearReq { stencil: Some(0x00), ..color(BLUE) }),
                clear(ClearReq { stencil: Some(0xff), stencil_mask: 0x0f, ..no_color() }),
                draw(Gl { stencil_equal: Some(0xff), ..GL }, 0),
            ],
        ),
        (Fb::Window, vec![draw(Gl { stencil_equal: Some(0x0f), ..GL }, 0)]),
    ]
}

/// Case 66, `case_clearvp` (the viewport is not a clear's input).
fn case66() -> Vec<Flush> {
    vec![
        (Fb::Window, vec![clear(color(WHITE))]),
        (Fb::Window, vec![clear(color(BLUE))]),
        (Fb::Window, vec![clear(ClearReq { scissor: Some([0, 0, 32, 32]), ..color(GREEN) })]),
    ]
}

/// Case 67, `case_fbods`, frames 1 to 8.
fn case67() -> Vec<Flush> {
    let less = Gl { depth: Some(Compare::Less), ..GL };
    let bx = Some([16, 16, 32, 32]);
    let box_stencil = clear(ClearReq { stencil: Some(1), scissor: bx, ..no_color() });
    let blue_s0 = clear(ClearReq { stencil: Some(0), ..color(BLUE) });
    let eq0 = Gl { stencil_equal: Some(0), ..GL };
    vec![
        /* 1: colour only, the depth test on */
        (Fb::FboA, vec![clear(ClearReq { depth: Some(1.0), ..color(BLUE) }), draw(less, 0), draw(less, 6)]),
        /* 2: a stencil test with no stencil buffer */
        (
            Fb::FboA,
            vec![clear(ClearReq { stencil: Some(0), ..color(BLUE) }), draw(Gl { stencil_equal: Some(1), ..GL }, 0)],
        ),
        /* 3: a GL_RGB attachment cleared to alpha 0.25 */
        (Fb::FboRgb, vec![clear(color([1.0, 0.0, 0.0, 0.25]))]),
        /* 4 and 5: one texture, two depth buffers */
        (Fb::FboADepth, vec![clear(ClearReq { depth: Some(1.0), ..color(BLUE) }), draw(less, 0)]),
        (Fb::FboBDepth, vec![draw(less, 6)]),
        /* 6: a scissored stencil-only clear, then a draw where it is 0 */
        (Fb::FboAStencil, vec![blue_s0.clone(), box_stencil.clone(), draw(eq0, 0)]),
        /* 7: the same with a draw ahead, in one kick */
        (Fb::FboAStencil, vec![blue_s0.clone(), draw(GL, 6), box_stencil.clone(), draw(eq0, 0)]),
        /* 8: the same across a kick */
        (Fb::FboAStencil, vec![blue_s0, draw(GL, 6), box_stencil]),
        (Fb::FboAStencil, vec![draw(eq0, 0)]),
    ]
}

/// Case 102, `case_maskclear` (tests/v3d71_case102_record.rs's flushes).
fn case102() -> Vec<Flush> {
    let masked =
        |c: [f32; 4], mask: u8, scissor: Option<[i32; 4]>| clear(ClearReq { color_mask: mask, scissor, ..color(c) });
    let all = R | G | B | A;
    vec![
        (Fb::ColorOnly, vec![masked(BLACK, all, None)]),
        (Fb::ColorOnly, vec![masked([1.0, 1.0, 1.0, 0.5], R | A, None)]),
        (Fb::ColorOnly, vec![masked(BLACK, all, None)]),
        (Fb::ColorOnly, vec![masked([0.0, 1.0, 0.0, 0.0], G, Some([0, 0, 32, 32]))]),
        (Fb::ColorOnly, vec![masked(BLACK, all, None)]),
        (Fb::ColorOnly, vec![masked([0.0, 0.0, 1.0, 0.0], B, None), draw(GL, 0)]),
    ]
}

/* ---- what v3d_job71 reads of a job ---------------------------------------- */

/// # Safety
/// `p` points at `n` live `T`s (or `n` is 0).
unsafe fn slice<'x, T>(p: *const T, n: u32) -> &'x [T] {
    if n == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(p, n as usize) }
    }
}

/// A draw as the C builder reads it through `ffi()`, pointers resolved.
fn draw_text(d: ffi71::V3dJob71Draw, quad: Option<[[f32; 4]; 4]>, is_clear: bool) -> String {
    // SAFETY: `d` was just made by `ffi()` of a draw alive in the caller;
    // every pointer is null with a 0 count or points at that many items.
    unsafe {
        let (st, mut fs, mut vs, mut cs) = (*d.st, *d.fs, *d.vs, *d.cs);
        let fsu = slice(fs.uniforms, fs.num_uniforms);
        let vsu = slice(vs.uniforms, vs.num_uniforms);
        let csu = slice(cs.uniforms, cs.num_uniforms);
        (fs.uniforms, vs.uniforms, cs.uniforms) = (core::ptr::null(), core::ptr::null(), core::ptr::null());
        format!(
            "st {st:?}\nfs {fs:?} {fsu:?}\nvs {vs:?} {vsu:?}\ncs {cs:?} {csu:?}\nvs block {:?}\nfs block {:?}\n\
             attrs {:?}\nprim {} count {} first {} indexed {} type {} ib {:#x} bytes {} off {} tmu {} {} \
             psiz {} instances {}\nquad {quad:?} clear {is_clear}\n",
            slice(d.vs_storage.words, d.vs_storage.count),
            slice(d.fs_storage.words, d.fs_storage.count),
            slice(d.attrs, d.num_attrs),
            d.prim,
            d.count,
            d.first,
            d.indexed,
            d.index_type,
            d.index_buffer,
            d.index_buffer_bytes,
            d.index_offset,
            d.num_tmu,
            d.num_vs_tmu,
            d.vs_writes_psiz,
            d.instances,
        )
    }
}

fn job_head(target: &ffi71::V3dJob71Target, clear: &ffi71::V3dJob71Clear, after: &History, n: usize) -> String {
    format!("target {target:?}\nclear {clear:?}\nafter {after:?}\n{n} draws\n")
}

fn view_text(j: &JobView<'_, '_>) -> String {
    let mut s = job_head(&j.target, &j.clear, &j.after, j.draws.len());
    for d in j.draws {
        s += &draw_text(d.ffi(), d.quad, d.is_clear);
    }
    s
}

fn owned_text(j: &FilledJob) -> String {
    let mut s = job_head(&j.target, &j.clear, &j.after, j.draws.len());
    for d in &j.draws {
        s += &draw_text(d.ffi(), d.quad, d.is_clear);
    }
    s
}

/* ---- the three runs of a flush --------------------------------------------- */

/// `fill_each` with `slots` scratch slots: the jobs handed and how it ended.
fn each(fb: &Framebuffer<'_>, hist: &History, ops: &[Op<'_>], slots: usize) -> (Vec<String>, Result<(), FillEach>) {
    let q = clear_quad();
    let mut scratch = vec![DrawView::new(); slots];
    let mut got = Vec::new();
    let r = fill71::fill_each(fb, record71::BCM2712_D0, hist, &q, ops, &mut scratch, |j| {
        got.push(view_text(j));
        Ok(())
    });
    (got, r)
}

/// The op that makes the third draw of the first job with more than two:
/// the draws of `fill`'s jobs before that job, plus two, are made by a
/// shorter op prefix (a GL draw or a clear quad is one draw of its op, and
/// a later op removes none), so it is the last op of the shortest prefix
/// that makes one more.
fn third_draw_op(fb: &Framebuffer<'_>, hist: &History, ops: &[Op<'_>], jobs: &[FilledJob]) -> Option<usize> {
    let first = jobs.iter().position(|j| j.draws.len() > 2)?;
    let before: usize = jobs[..first].iter().map(|j| j.draws.len()).sum();
    let q = clear_quad();
    let made = |n: usize| -> usize {
        fill71::fill(fb, record71::BCM2712_D0, hist, &q, &ops[..n])
            .map(|j| j.iter().map(|j| j.draws.len()).sum())
            .unwrap_or(usize::MAX)
    };
    (1..=ops.len()).find(|&n| made(n) > before + 2).map(|n| n - 1)
}

/// One flush's outcome: "ok J" (J jobs), "full@N" (two slots full at op N
/// after the jobs before it) or "refused M" (module M).
fn run_flush(fb: Framebuffer<'static>, hist: &History, ops: &[Op<'static>], what: &str) -> (String, Option<History>) {
    let q = clear_quad();
    let want = fill71::fill(&fb, record71::BCM2712_D0, hist, &q, ops);
    let roomy = each(&fb, hist, ops, ops.len().max(1));
    let two = each(&fb, hist, ops, 2);
    match &want {
        Ok(jobs) => {
            let texts: Vec<String> = jobs.iter().map(owned_text).collect();
            assert_eq!(roomy.1, Ok(()), "{what}: a roomy scratch is never full");
            assert_eq!(roomy.0, texts, "{what}: fill_each hands the jobs fill gives");
            match jobs.iter().position(|j| j.draws.len() > 2) {
                None => {
                    assert_eq!(two.1, Ok(()), "{what}: every job fits two slots");
                    assert_eq!(two.0, texts, "{what}: two slots hand the jobs fill gives");
                    (format!("ok {}", jobs.len()), jobs.last().map(|j| j.after))
                }
                Some(k) => {
                    let op = third_draw_op(&fb, hist, ops, jobs).expect("the op of the third draw");
                    assert_eq!(two.1, Err(FillEach::Full { op }), "{what}: two slots are full at the third draw");
                    assert_eq!(two.0, texts[..k], "{what}: the jobs before the full one are handed whole");
                    (format!("full@{op}"), jobs.last().map(|j| j.after))
                }
            }
        }
        Err(r) => {
            assert_eq!(roomy.1, Err(FillEach::Refused(r.clone())), "{what}: the refusal fill gives");
            match &two.1 {
                Err(FillEach::Refused(r2)) => {
                    assert_eq!(r2, r, "{what}: two slots give the same refusal");
                    assert_eq!(two.0, roomy.0, "{what}: the same jobs before the refusal");
                }
                Err(FillEach::Full { op }) => {
                    assert!(*op < ops.len(), "{what}: full at an op of the flush");
                    assert!(roomy.0.starts_with(&two.0), "{what}: the jobs before the full one");
                }
                Ok(()) => panic!("{what}: fill refused ({r:?}) but two slots did not"),
            }
            (format!("refused {}", r.module), None)
        }
    }
}

/// Runs a case's flushes in order, each framebuffer with its own history
/// (case 67's fbo_b and fbo_a share the colour texture: fbo_b starts from
/// what fbo_a stored in it). A refused flush ends the case, as the table's
/// refusal ends a gl_test case.
fn run_case(name: &str, flushes: Vec<Flush>) -> Vec<String> {
    let mut hists: Vec<(Fb, History)> = Vec::new();
    let mut out = Vec::new();
    for (n, (fb, ops)) in flushes.iter().enumerate() {
        let mut hist = hists.iter().find(|(f, _)| f == fb).map_or(History::NEW, |(_, h)| *h);
        if *fb == Fb::FboBDepth {
            if let Some((_, a)) = hists.iter().find(|(f, _)| *f == Fb::FboADepth) {
                hist.colors = a.colors;
            }
        }
        let what = format!("case {name}, flush {}", n + 1);
        let (o, after) = run_flush(framebuffer(*fb), &hist, ops, &what);
        out.push(o);
        match after {
            Some(h) => {
                hists.retain(|(f, _)| f != fb);
                hists.push((*fb, h));
            }
            None if out.last().is_some_and(|o| o.starts_with("refused")) => break,
            None => {}
        }
    }
    out
}

/// Each case's flushes: what `fill` gives and what a two-draw scratch does.
const EXPECT: [(&str, &[&str]); 6] = [
    ("5", &["ok 1", "ok 1"]),
    ("58", &["ok 1", "ok 1", "ok 1"]),
    ("59", &["ok 1", "ok 1", "ok 1", "refused zskeep71"]),
    ("66", &["ok 1", "ok 1", "ok 1"]),
    ("67", &["ok 1", "ok 1", "ok 1", "ok 1", "ok 1", "ok 1", "full@3", "ok 1", "refused zskeep71"]),
    ("102", &["ok 1", "ok 1", "ok 1", "ok 1", "ok 1", "ok 1"]),
];

fn case(name: &str) -> Vec<Flush> {
    match name {
        "5" => case5(),
        "58" => case58(),
        "59" => case59(),
        "66" => case66(),
        "67" => case67(),
        "102" => case102(),
        _ => unreachable!(),
    }
}

#[test]
fn the_six_cases_give_fills_jobs_and_full_past_two_draws() {
    let mut got = Vec::new();
    for (name, _) in EXPECT {
        got.push((name, run_case(name, case(name))));
    }
    let want: Vec<(&str, Vec<String>)> =
        EXPECT.iter().map(|(n, o)| (*n, o.iter().map(|s| s.to_string()).collect())).collect();
    assert_eq!(got, want);
    assert!(
        got.iter().flat_map(|(_, o)| o).any(|o| o.starts_with("full@")),
        "at least one flush of the six does not fit two slots"
    );
}

#[test]
fn a_sink_refusal_stops_the_fill_after_the_job_it_was_handed() {
    let fb = framebuffer(Fb::Window);
    let q = clear_quad();
    /* case 58's second flush is one job: the sink sees it, refuses, and
     * fill_each returns the sink's refusal */
    let ops = &case58()[1].1;
    let mut scratch = [DrawView::new(); 2];
    let mut seen = 0;
    let no = fill71::Refusal { module: "sink", reason: "no".to_string() };
    let r = fill71::fill_each(&fb, record71::BCM2712_D0, &History::NEW, &q, ops, &mut scratch, |_| {
        seen += 1;
        Err(no.clone())
    });
    assert_eq!((seen, r), (1, Err(FillEach::Refused(no))));
}

#[test]
fn an_empty_scratch_is_full_at_the_first_draw_and_hands_a_tile_clear() {
    let fb = framebuffer(Fb::Window);
    let q = clear_quad();
    let mut scratch: [DrawView<'_>; 0] = [];
    /* a whole clear needs no slot */
    let ops = [clear(color(BLUE))];
    let mut jobs = 0;
    let r = fill71::fill_each(&fb, record71::BCM2712_D0, &History::NEW, &q, &ops, &mut scratch, |j| {
        assert!(j.draws.is_empty());
        jobs += 1;
        Ok(())
    });
    assert_eq!((jobs, r), (1, Ok(())));
    /* the draw after it is op 1 */
    let ops = [clear(color(BLUE)), draw(GL, 0)];
    let r = fill71::fill_each(&fb, record71::BCM2712_D0, &History::NEW, &q, &ops, &mut scratch, |_| Ok(()));
    assert_eq!(r, Err(FillEach::Full { op: 1 }));
}

#[test]
fn a_view_borrows_the_programs_lists_and_the_draws_blocks() {
    let fb = framebuffer(Fb::Window);
    let q = clear_quad();
    let words = [0x3f80_0000u32; 4];
    let mut d = match draw(GL, 0) {
        Op::Draw(d) => *d,
        Op::Clear(_) => unreachable!(),
    };
    d.vs_words = &words[..2];
    d.fs_words = &words;
    let ops = [clear(ClearReq { scissor: Some([0, 0, 32, 32]), ..color(RED) }), Op::Draw(Box::new(d))];
    let mut scratch = [DrawView::new(); 2];
    let (gl, cl) = programs();
    let r = fill71::fill_each(&fb, record71::BCM2712_D0, &History::NEW, &q, &ops, &mut scratch, |j| {
        let [quad, draw] = j.draws else { panic!("two draws: {}", j.draws.len()) };
        /* the clear quad: the clear program's lists, its colour inline */
        assert!(quad.is_clear && quad.fs_words.is_empty() && quad.vs_words.is_empty());
        assert_eq!(quad.fs_block(), RED.map(f32::to_bits));
        assert_eq!(quad.fs_uniforms.as_ptr(), cl.fs_uniforms.as_ptr());
        assert_eq!(quad.fs.uniforms.cast::<uniform71::Word>(), cl.fs_uniforms.as_ptr());
        assert_eq!(quad.attrs().len(), 1);
        /* the GL draw: its program's lists and its blocks, not copies */
        assert_eq!(draw.fs_uniforms.as_ptr(), gl.fs_uniforms.as_ptr());
        assert_eq!(draw.vs.uniforms.cast::<uniform71::Word>(), gl.vs_uniforms.as_ptr());
        assert_eq!(draw.cs.num_uniforms as usize, gl.cs_uniforms.len());
        assert_eq!(draw.fs_block().as_ptr(), words.as_ptr());
        assert_eq!(draw.ffi().vs_storage.words, words.as_ptr());
        assert_eq!(draw.ffi().fs_storage.count, 4);
        assert_eq!(draw.attrs().len(), 3);
        Ok(())
    });
    assert_eq!(r, Ok(()));
}
