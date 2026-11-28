// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! gl_test case 102 (`maskclear`, driver/tools/pvrlab/gl_test.c:23479-23557;
//! dispatch :24603) through the 7.1 host path, the way
//! tests/v3d71_case5_record.rs does case 5: our own 7.1 link, the merged 7.1
//! modules, `fill71::fill` for the jobs, `admit71` for the one shader-state
//! record the case reads.
//!
//! Case 102 is a colour clear under a partial `glColorMask`, three shapes
//! over the 64x64 target (TW, TH), six `glFinish`, so six flushes:
//!  1. black, then R+A on (1,1,1,0.5) -> (1,0,0,0.5): flushes 1 and 2;
//!  2. black, then the scissor (0,0,32,32) with G only: flushes 3 and 4;
//!  3. black (flush 5), then B only, the mask set back to all, and
//!     `glDrawArrays(GL_TRIANGLES, 0, 6)` of the yellow quad over the
//!     bottom-left quadrant (flush 6).
//! A full clear is the tile buffer's `Clear Render Targets` (no draw); a
//! partial mask or a scissor box is the clear quad, a draw (clearplan71),
//! and the fused draw follows it in the same job.
//!
//! Expected board verdict: PASS. No stencil is read (the target has no
//! depth/stencil buffer in this case's frames), so case 59's shared
//! refusal ("the draw reads stencil an earlier job wrote...") is not
//! reached; the program (`color_uv`) and the clear program both link.
//!
//! The only record the case's draws read beyond the clear quad's is the
//! `color_uv` D0 record of flush 6's draw; it is the same record as case
//! 5's (same program, same attributes), pinned here to the same bytes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_case102_record
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]
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

use fill71::{ClearQuad, ClearReq, ColorBuffer, DrawIn, FilledDraw, FilledJob, Framebuffer, Op, Program};
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

/// A stand-in for the program BO's GPU address (8-byte aligned; the table
/// allocates the BO at run time). Every code address pinned below is this
/// plus the program's offset in the linked blob.
const CODE_BASE: u32 = 0x0020_0000;
/// The clear program's BO (another program, another BO).
const CLEAR_BASE: u32 = 0x0021_0000;
/// The vertex buffer, the colour image, the depth/stencil image and the
/// tile allocation: stand-ins (none reaches the shader record).
const VTX: u32 = 0x0030_0000;
const RT0: u32 = 0x0100_0000;
const ZS: u32 = 0x0200_0000;
const TILE: u32 = 0x0040_0000;

/// TW, TH (gl_test.c:1088-1089).
const SIZE: u32 = 64;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLES: u32 = 4;

/* ---- step 1: the link ---------------------------------------------------- */

fn stream(u: *const ShcV3dUniform, n: u32) -> &'static [uniform71::Word] {
    if n == 0 {
        return &[];
    }
    // SAFETY: the record's stream of `n` entries, alive while the keep is
    // (the caller holds it); copied out before it closes.
    let v: Vec<uniform71::Word> = unsafe { core::slice::from_raw_parts(u, n as usize) }
        .iter()
        .map(|u| uniform71::Word { contents: u.contents, data: u.data })
        .collect();
    Box::leak(v.into_boxed_slice())
}

fn c_source(s: &'static [u8]) -> &'static str {
    core::str::from_utf8(s.strip_suffix(b"\0").expect("NUL-terminated")).expect("UTF-8")
}

/// Where the link put each program in its blob: (offset, bytes) of FS, VS, CS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    fs: (u32, u32),
    vs: (u32, u32),
    cs: (u32, u32),
}

/// One process-wide lock: the link's `compile_rt_f32` key is global.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Links (vs, fs) at 7.1 and maps the prog_data while the keep is open
/// (step 2), the program BO at `base`.
fn link(vs: &str, fs: &str, base: u32, discards: bool) -> (Program<'static>, Placed) {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _keep = store::begin_keep().expect("keep");
    let sh: ShcShader = store::with_scratch(|a| {
        link_sources71(vs.as_bytes(), None, LodKey::NONE, fs.as_bytes(), &[], a, store::keep()).map(|r| r.0)
    })
    .expect("scratch")
    .unwrap_or_else(|e| panic!("link71: {e:?}"));
    let p = &sh.v3d;
    assert_eq!(p.linked, 1);
    let placed = Placed {
        fs: (p.fs.code_off, p.fs.code_bytes),
        vs: (p.vs.code_off, p.vs.code_bytes),
        cs: (p.cs.code_off, p.cs.code_bytes),
    };
    (program(&sh, base, discards), placed)
}

/* ---- step 2: prog_data onto fill71::Program (the test's own map) --------- */

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
        // v3d_job71 writes As 1, Ve 0 (v3d_job71.c:710-713): no GS
        min_input_segments: 1,
        min_output_segments: 0,
        uses_vid: v.uses_vid != 0,
        uses_iid: v.uses_iid != 0,
        uses_base_instance: v.uses_biid != 0,
        uses_base_vertex: false,
        uses_draw_index: false,
    }
}

/// `discards`: `ShcV3dFs` carries no discard flag; the pinned header's
/// `OURS71_<P>_FS_USES_DISCARD` is the compile's answer (0 for both).
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

/// A linked program and where its three programs sit in the blob.
type Linked = (Program<'static>, Placed);

/// (case 5's program, the clear program).
fn programs() -> &'static (Linked, Linked) {
    static P: std::sync::OnceLock<(Linked, Linked)> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let d_gl = header_define("tools/v3d/gltest-shaders71/ours_color_uv.h", "OURS71_COLOR_UV_FS_USES_DISCARD") != 0;
        let d_cl = header_define("tools/v3d/gltest-shaders71/ours_clear.h", "OURS71_CLEAR_FS_USES_DISCARD") != 0;
        (
            link(COLOR_UV_VS, COLOR_UV_FS, CODE_BASE, d_gl),
            link(c_source(CLEAR_VS), c_source(CLEAR_FS), CLEAR_BASE, d_cl),
        )
    })
}

fn gl_program() -> &'static Linked {
    &programs().0
}

fn clear_program() -> &'static Linked {
    &programs().1
}

/* ---- case 102's GL calls through fill71 -------------------------------------- */

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

/// cuv_attribs (gl_test.c:1400-1409): locations 0, 1, 2 at +0, +16, +32 of
/// a 40-byte vertex.
static CUV_ATTRS: [attr71::Attr; 3] =
    [float_attr(4, 40, VTX), float_attr(4, 40, VTX + 16), float_attr(2, 40, VTX + 32)];
const QUAD_ATTR: attr71::Attr = float_attr(4, 16, VTX + 0x100);

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &clear_program().0, attr: QUAD_ATTR, dummy_addr: VTX }
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const BOX: [i32; 4] = [0, 0, 32, 32];
const R: u8 = 0b0001;
const G: u8 = 0b0010;
const B: u8 = 0b0100;
const A: u8 = 0b1000;

fn framebuffer<'a>(colors: &'a [ColorBuffer]) -> Framebuffer<'a> {
    let ds = None;
    Framebuffer { width: SIZE, height: SIZE, tile_alloc: TILE, flip_y: false, colors, ds }
}

fn masked(c: [f32; 4], mask: u8, scissor: Option<[i32; 4]>) -> Op<'static> {
    Op::Clear(ClearReq { color_mask: mask, scissor, ..ClearReq::color(c) })
}

/// The quad draw of step 3 (gl_test.c:23541-23549): the mask is all again.
fn the_draw() -> DrawIn<'static> {
    let mut st = draw71::DrawState::gl_default(SIZE, SIZE, false, 1);
    st.viewport = [0.0, 0.0, SIZE as f32, SIZE as f32];
    DrawIn {
        state: st,
        line_width: 1.0,
        blend_color: [0.0; 4],
        program: &gl_program().0,
        attrs: &CUV_ATTRS,
        dummy_addr: VTX,
        prim: prim71::Draw::arrays(GL_TRIANGLES, 0, 6),
        vs_words: &[],
        fs_words: &[],
        oq_addr: 0,
    }
}

/// The six flushes, each a `glFinish`: (the jobs of each flush).
fn case102_flushes() -> Vec<Vec<FilledJob>> {
    let colors = [ColorBuffer {
        gl_format: fill71::GL_RGBA8,
        address: RT0,
        memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR,
        height_in_ub_or_stride: 8,
    }];
    let fb = framebuffer(&colors);
    let q = clear_quad();
    let all = R | G | B | A;
    let ops: Vec<Vec<Op<'static>>> = vec![
        vec![masked(BLACK, all, None)],
        vec![masked([1.0, 1.0, 1.0, 0.5], R | A, None)],
        vec![masked(BLACK, all, None)],
        vec![masked([0.0, 1.0, 0.0, 0.0], G, Some(BOX))],
        vec![masked(BLACK, all, None)],
        vec![masked([0.0, 0.0, 1.0, 0.0], B, None), Op::Draw(Box::new(the_draw()))],
    ];
    let mut hist = fill71::History::NEW;
    let mut out = Vec::new();
    for (n, flush) in ops.iter().enumerate() {
        let jobs = fill71::fill(&fb, record71::BCM2712_D0, &hist, &q, flush)
            .unwrap_or_else(|r| panic!("flush {}: {r:?}", n + 1));
        hist = jobs.last().expect("a job per flush").after;
        out.push(jobs);
    }
    out
}

/* ---- step 4: the record of a filled draw ---------------------------------- */

/// What `build_draw` asks admit71 for (fill71.rs, "the programs"), with the
/// draw's own attributes.
fn record_of(p: &Program<'_>, attrs: &[attr71::Attr]) -> record71::Record {
    let desc = &p.desc;
    admit71::admit_draw(
        record71::BCM2712_D0,
        &admit71::Draw {
            attrs,
            cs_reads: p.cs_reads,
            vs_reads: p.vs_reads,
            cs_builtins: desc.cs.uses_vid || desc.cs.uses_iid,
            dummy_addr: VTX,
            separate_segments: p.separate_segments,
            programs: *desc,
        },
    )
    .unwrap_or_else(|r| panic!("admit71: {r:?}"))
    .record
}

/// The C builder copies these program-struct fields into the record
/// (v3d_job71.c:688-724); the filled structs must hold the record's values.
fn c_copies_the_record(fd: &FilledDraw, r: &record71::Record) {
    assert_eq!(fd.vs_writes_psiz, r.point_size_in_shaded_vertex_data);
    assert_eq!(fd.fs.writes_z, r.fragment_shader_does_z_writes);
    assert_eq!(fd.fs.disable_ez, r.turn_off_early_z_test);
    assert_eq!(fd.fs.uses_center_w, r.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2);
    assert_eq!(fd.fs.uses_pid, r.insert_primitive_id_as_first_varying_to_fragment_shader);
    assert_eq!(fd.fs.lock_scoreboard_on_first_thrsw, r.do_scoreboard_wait_on_first_thread_switch);
    assert_eq!(!fd.fs.uses_implicit_point_line_varyings, r.disable_implicit_point_line_varyings);
    assert_eq!(fd.fs.num_inputs, r.number_of_varyings_in_fragment_shader);
    assert_eq!(fd.fs.code, r.fragment_shader_code_address);
    assert_eq!(fd.vs.code, r.vertex_shader_code_address);
    assert_eq!(fd.cs.code, r.coordinate_shader_code_address);
    assert_eq!(fd.cs.vpm_input_size, r.coordinate_shader_input_vpm_segment_size);
    assert_eq!(fd.vs.vpm_input_size, r.vertex_shader_input_vpm_segment_size);
    assert_eq!(fd.cs.vpm_output_size, r.coordinate_shader_output_vpm_segment_size);
    assert_eq!(fd.vs.vpm_output_size, r.vertex_shader_output_vpm_segment_size);
    assert_eq!(fd.fs.threads == 4, r.fragment_shader_4_way_threadable);
    assert_eq!(fd.vs.threads == 4, r.vertex_shader_4_way_threadable);
    assert_eq!(fd.cs.threads == 4, r.coordinate_shader_4_way_threadable);
    assert_eq!(fd.fs.single_seg, r.fragment_shader_start_in_final_thread_section);
    assert_eq!(fd.vs.single_seg, r.vertex_shader_start_in_final_thread_section);
    assert_eq!(fd.cs.single_seg, r.coordinate_shader_start_in_final_thread_section);
    assert_eq!(fd.vs.uses_vid, r.vertex_id_read_by_vertex_shader);
    assert_eq!(fd.vs.uses_iid, r.instance_id_read_by_vertex_shader);
    assert_eq!(fd.cs.uses_vid, r.vertex_id_read_by_coordinate_shader);
    assert_eq!(fd.cs.uses_iid, r.instance_id_read_by_coordinate_shader);
    // fixed by the builder (v3d_job71.c:682-684, 710-713)
    assert_eq!(r.min_coord_shader_input_segments_required_in_play, 1);
    assert_eq!(r.min_vertex_shader_input_segments_required_in_play, 1);
    assert_eq!(r.min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size, 0);
    assert_eq!(r.min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size, 0);
}

/* ---- the XML positions ---------------------------------------------------- */

/// (start, size) of `field` in the struct whose tag opens with `open`, read
/// from X: (`Nb` starts are byte numbers).
fn xml_pos(open: &str, field: &str) -> (u32, u32) {
    let x = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let mut it = x.lines().skip_while(|l| !l.trim_start().starts_with(open));
    assert!(it.next().is_some(), "{open} not in X:");
    let attr = |l: &str, k: &str| -> String {
        let pat = format!(" {k}=\"");
        let i = l.find(&pat).unwrap() + pat.len();
        l[i..i + l[i..].find('"').unwrap()].to_string()
    };
    for l in it {
        let l = l.trim();
        if l.starts_with("</struct>") {
            break;
        }
        if l.starts_with("<field ") && attr(l, "name") == field {
            let s = attr(l, "start");
            let start = match s.strip_suffix('b') {
                Some(b) => b.parse::<u32>().unwrap() * 8,
                None => s.parse().unwrap(),
            };
            return (start, attr(l, "size").parse().unwrap());
        }
    }
    panic!("{field} not in {open}");
}

const D0: &str = "<struct name=\"GL Shader State Record Draw Index\"";
const PLAIN: &str = "<struct name=\"GL Shader State Record\" min_ver=\"71\"";

fn bits(b: &[u8; 32], start: u32, size: u32) -> u32 {
    (0..size).fold(0, |v, k| {
        let bit = start + k;
        v | (u32::from(b[(bit / 8) as usize] >> (bit % 8) & 1) << k)
    })
}

fn at(b: &[u8; 32], layout: &str, field: &str) -> u32 {
    let (s, n) = xml_pos(layout, field);
    bits(b, s, n)
}

fn word(b: &[u8; 32], i: usize) -> u32 {
    u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]])
}

/* ---- the pins ---------------------------------------------------------------- */

/// The program offsets of the link (as tests/v3d71_case5_record.rs pins:
/// FS 17, VS 27, CS 18 words of ours_color_uv.h).
const FS_OFF: u32 = 0;
const VS_OFF: u32 = 17 * 8;
const CS_OFF: u32 = (17 + 27) * 8;

/// The clear colour as the tile buffer holds it, RGBA8 packed R in the low
/// byte: opaque black.
const PACKED_BLACK: u32 = 0xff00_0000;

#[test]
fn six_flushes_make_six_jobs_with_three_clear_quads_and_one_draw() {
    let fl = case102_flushes();
    assert_eq!(fl.len(), 6, "six glFinish");
    assert!(fl.iter().all(|f| f.len() == 1), "one job per flush");
    let draws: Vec<usize> = fl.iter().map(|f| f[0].draws.len()).collect();
    // the three full-mask black clears are Clear Render Targets (no draw);
    // the partial masks are one clear quad each; flush 6 adds the yellow quad
    assert_eq!(draws, [0, 1, 0, 1, 0, 2]);
    let jobs: usize = fl.iter().map(Vec::len).sum();
    let all_draws: usize = draws.iter().sum();
    assert_eq!((jobs, all_draws), (6, 4));
    for n in [0, 2, 4] {
        assert_eq!(fl[n][0].clear.load_rts, 0, "flush {}: a full clear loads nothing", n + 1);
        assert_eq!(fl[n][0].clear.color[0][0], PACKED_BLACK, "flush {}: black, alpha 1", n + 1);
    }
    for n in [1, 3, 5] {
        // a masked clear keeps what the target holds: the job loads it
        assert_eq!(fl[n][0].clear.load_rts, 1, "flush {}: the masked clear loads RT 0", n + 1);
        assert!(fl[n][0].draws[0].is_clear);
    }
}

#[test]
fn the_clear_quads_carry_the_masks_and_boxes() {
    let fl = case102_flushes();
    let q = |n: usize| &fl[n][0].draws[0];
    // R+A (shape 1), G (shape 2, scissored), B (shape 3)
    assert_eq!(q(1).state.colormask, u32::from(R | A));
    assert_eq!(q(3).state.colormask, u32::from(G));
    assert_eq!(q(5).state.colormask, u32::from(B));
    for n in [1, 3, 5] {
        assert_eq!(q(n).count, 4, "a TRIANGLE_FAN quad");
        assert_eq!(q(n).prim, ffi71::V3D_PRIM_TRIANGLE_FAN);
    }
    // the full-target quads span clip space; the scissored one is the
    // bottom-left quarter, (0,0,32,32) of 64x64 = clip -1..0
    let full = [[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0], [-1.0, 1.0, 0.0, 1.0]];
    let quarter = [[-1.0, -1.0, 0.0, 1.0], [0.0, -1.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0], [-1.0, 0.0, 0.0, 1.0]];
    assert_eq!(q(1).quad, Some(full));
    assert_eq!(q(3).quad, Some(quarter));
    assert_eq!(q(5).quad, Some(full));
}

#[test]
fn the_fused_draw_writes_all_four_channels_after_the_blue_clear() {
    let fl = case102_flushes();
    let d = &fl[5][0].draws[1];
    assert!(!d.is_clear);
    assert_eq!((d.count, d.first, d.prim), (6, 0, ffi71::V3D_PRIM_TRIANGLES));
    assert_eq!(d.state.colormask, 0xf, "glColorMask is all again before the draw");
    assert!(!d.indexed);
}

#[test]
fn the_link_places_the_three_programs_as_pinned() {
    assert_eq!(gl_program().1, Placed { fs: (FS_OFF, 17 * 8), vs: (VS_OFF, 27 * 8), cs: (CS_OFF, 18 * 8) });
}

/// The yellow quad's record (gl_test.c:23549, `glDrawArrays(GL_TRIANGLES, 0,
/// 6)`): `color_uv` with the cuv attributes, the record case 5 reads.
#[test]
fn the_quad_record_is_case_5s_d0_record() {
    let fl = case102_flushes();
    let fd = &fl[5][0].draws[1];
    let r = record_of(&gl_program().0, &CUV_ATTRS);
    c_copies_the_record(fd, &r);
    let b = r.pack();
    assert_eq!(r.fragment_shader_code_address, CODE_BASE + FS_OFF);
    assert_eq!(r.vertex_shader_code_address, CODE_BASE + VS_OFF);
    assert_eq!(r.coordinate_shader_code_address, CODE_BASE + CS_OFF);
    assert_eq!(word(&b, 2), CODE_BASE + FS_OFF + 0b01);
    assert_eq!(word(&b, 4), CODE_BASE + VS_OFF + 0b11);
    assert_eq!(word(&b, 6), CODE_BASE + CS_OFF + 0b11);
    let trio = [
        "Fragment shader uses real pixel centre W in addition to centroid W2",
        "Insert Primitive ID as first varying to fragment shader",
        "Disable implicit point/line varyings",
    ];
    assert_eq!(trio.map(|f| at(&b, D0, f)), [1, 0, 1], "the trio at the D0 bits 15, 18, 21");
    assert_eq!(trio.map(|f| at(&b, PLAIN, f)), [0, 1, 0], "what a plain-layout decode shows");
    assert_eq!(at(&b, D0, "Never defer FEP depth writes"), 0);
    for f in ["cs_basevertex", "cs_drawindex", "vs_basevertex", "vs_drawindex"] {
        assert_eq!(at(&b, D0, f), 0, "{f}");
    }
    assert_eq!(r.number_of_varyings_in_fragment_shader, 6);
    assert_eq!(&b[0..8], &[0x02, 0x80, 0x20, 0x06, 0x01, 0x00, 0x02, 0x00]);
    assert_eq!([word(&b, 3), word(&b, 5), word(&b, 7)], [0, 0, 0]);
}

/// The clear quads' record: the same `clear` program for all three, no
/// varyings, so no centre W; the first record the D0 reads in flush 2.
#[test]
fn the_clear_quad_record_is_the_d0_record_of_the_clear_program() {
    let fl = case102_flushes();
    let r = record_of(&clear_program().0, &[QUAD_ATTR]);
    let p = clear_program().1;
    for n in [1, 3, 5] {
        c_copies_the_record(&fl[n][0].draws[0], &r);
    }
    let b = r.pack();
    assert_eq!(r.fragment_shader_code_address, CLEAR_BASE + p.fs.0);
    assert_eq!(r.vertex_shader_code_address, CLEAR_BASE + p.vs.0);
    assert_eq!(r.coordinate_shader_code_address, CLEAR_BASE + p.cs.0);
    assert_eq!((p.fs.0, p.vs.0, p.cs.0), (0, p.fs.1, p.fs.1 + p.vs.1));
    let trio = [
        "Fragment shader uses real pixel centre W in addition to centroid W2",
        "Insert Primitive ID as first varying to fragment shader",
        "Disable implicit point/line varyings",
    ];
    assert_eq!(trio.map(|f| at(&b, D0, f)), [0, 0, 1]);
    assert_eq!(r.number_of_varyings_in_fragment_shader, 0);
    assert_eq!(&b[0..3], &[0x02, 0x00, 0x20]);
}

/// No refusal: every flush fills (case102_flushes panics on one), and none
/// of the shared refusals case 59 and 67 name is among the sources. Case 102
/// has no depth/stencil buffer, so the stencil read-lost decision is not
/// asked. Expected board verdict: PASS.
#[test]
fn case_102_is_not_refused() {
    assert_eq!(case102_flushes().len(), 6);
}
