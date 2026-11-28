// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host record of gl_test case 58 (`clearscis`,
//! driver/tools/pvrlab/gl_test.c:8106-8173, "glClear honours the scissor
//! box"), one of the Raspberry Pi 5's six P9 cases, through the 7.1 modules
//! as tests/v3d71_case5_record.rs does for case 5: `link_sources71` links
//! the program (`gl_test_vs_src` / `gl_test_fs_src`, the `color_uv` pair) and
//! the clear program, the test maps the prog_data onto `fill71::Program`,
//! and `fill71::fill` turns the case's GL calls into the jobs `v3d_job71`
//! builds. The step-by-step account is that file's header.
//!
//! Case 58 **does draw**: its second frame is a `glDrawArrays(GL_TRIANGLES,
//! 0, 6)` of the green quad. It makes three `glFinish` flushes (the 64x64
//! target, gl_test.c:1088-1089), so three jobs:
//!  1. blue `glClear`, then a scissored red `glClear` (box (16, 16, 32, 32)):
//!     the box is a quad (clearplan71: the tile buffer cannot clear a box),
//!     so one job with one clear draw;
//!  2. blue `glClear`, the green quad draw, then the same scissored clear:
//!     two draws, the GL draw first, the clear quad after it ("the clear lands
//!     after the draw");
//!  3. blue `glClear`, then a scissored clear with a zero-width box
//!     (16, 16, 0, 32), which clears nothing: a job with no draw.
//!
//! The verdict the host gives is a record, not a refusal: no case-58 call
//! meets a refusal (the stencil read-back refusal belongs to cases 59 and
//! 67, tests/v3d71_refuse.rs). So the expected board verdict is **PASS**
//! (`v3d_gl_test: PASS case 58`), provided the pixel checks hold on silicon:
//! the host does not render, the `expect_px` calls are the board's to answer.
//! The pins are the job and draw counts, the clear quads' vertices, the green
//! quad's D0 shader-state record bytes and the code offsets of the link.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_case58_record
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

use fill71::{ClearQuad, ClearReq, ColorBuffer, DrawIn, DsBuffer, FilledDraw, FilledJob, Framebuffer, Op, Program};
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

/* ---- step 3: case 58 through fill71 --------------------------------------- */

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
/// The clear quad's positions: fill71 hands back four vec4 corners.
const QUAD_ATTR: attr71::Attr = float_attr(4, 16, VTX + 0x100);

fn clear_quad() -> ClearQuad<'static> {
    ClearQuad { program: &clear_program().0, attr: QUAD_ATTR, dummy_addr: VTX }
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

const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
/// glScissor(16, 16, 32, 32) (gl_test.c:8124) and the empty box (8157).
const BOX: [i32; 4] = [16, 16, 32, 32];
const EMPTY: [i32; 4] = [16, 16, 0, 32];

/// The green quad's glDrawArrays (gl_test.c:8147): unscissored (the test is
/// off while it draws).
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

fn scis(box_: [i32; 4], c: [f32; 4]) -> Op<'static> {
    Op::Clear(ClearReq { scissor: Some(box_), ..ClearReq::color(c) })
}

/// The three flushes of case 58 (gl_test.c:8122-8166), one `fill` each,
/// the history carried from job to job.
fn case58_jobs() -> Vec<FilledJob> {
    let colors = [ColorBuffer {
        gl_format: fill71::GL_RGBA8,
        address: RT0,
        memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR,
        height_in_ub_or_stride: 8,
    }];
    let ds = Some(DsBuffer {
        gl_format: fill71::GL_DEPTH24_STENCIL8,
        address: ZS,
        memory_format: ffi71::V3D_MEMORY_FORMAT_UIF_NO_XOR,
        height_in_ub: 8,
    });
    let fb = Framebuffer { width: SIZE, height: SIZE, tile_alloc: TILE, flip_y: false, colors: &colors, ds };
    let q = clear_quad();
    let flushes: [Vec<Op<'static>>; 3] = [
        vec![Op::Clear(ClearReq::color(BLUE)), scis(BOX, RED)],
        vec![Op::Clear(ClearReq::color(BLUE)), Op::Draw(Box::new(the_draw())), scis(BOX, RED)],
        vec![Op::Clear(ClearReq::color(BLUE)), scis(EMPTY, RED)],
    ];
    let mut jobs: Vec<FilledJob> = Vec::new();
    let mut hist = fill71::History::NEW;
    for (n, ops) in flushes.iter().enumerate() {
        let js = fill71::fill(&fb, record71::BCM2712_D0, &hist, &q, ops)
            .unwrap_or_else(|r| panic!("flush {}: {r:?}", n + 1));
        assert_eq!(js.len(), 1, "flush {}: one glFinish, one job", n + 1);
        hist = js.last().unwrap().after;
        jobs.extend(js);
    }
    jobs
}

/// The quad `box_` of a GL box on the 64x64 target, in clip space.
fn clip_box(b: [i32; 4]) -> [f32; 4] {
    let c = |v: i32| v as f32 / 32.0 - 1.0;
    [c(b[0]), c(b[1]), c(b[0] + b[2]), c(b[1] + b[3])]
}

fn quad_box(fd: &FilledDraw) -> [f32; 4] {
    let q = fd.quad.expect("a clear draw carries its quad");
    let xs: Vec<f32> = q.iter().map(|v| v[0]).collect();
    let ys: Vec<f32> = q.iter().map(|v| v[1]).collect();
    let mn = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
    let mx = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
    [mn(&xs), mn(&ys), mx(&xs), mx(&ys)]
}

const FS_OFF: u32 = 0;
const VS_OFF: u32 = 17 * 8;
const CS_OFF: u32 = (17 + 27) * 8;

#[test]
fn case58_is_three_jobs_with_one_two_and_no_draws() {
    let jobs = case58_jobs();
    assert_eq!(jobs.iter().map(|j| j.draws.len()).collect::<Vec<_>>(), [1, 2, 0]);
    // job 1: the box is a clear quad
    assert!(jobs[0].draws[0].is_clear);
    // job 2: the draw, then the clear quad after it
    assert!(!jobs[1].draws[0].is_clear && jobs[1].draws[1].is_clear);
    assert_eq!(jobs[1].draws[0].count, 6);
    assert_eq!(jobs[1].draws[0].prim, ffi71::V3D_PRIM_TRIANGLES);
    // job 3: the empty box clears nothing
}

#[test]
fn the_clear_quads_cover_the_box_at_16_16_and_nothing_else() {
    let jobs = case58_jobs();
    let want = clip_box(BOX);
    assert_eq!(want, [-0.5, -0.5, 0.5, 0.5]);
    assert_eq!(quad_box(&jobs[0].draws[0]), want);
    assert_eq!(quad_box(&jobs[1].draws[1]), want);
}

#[test]
fn the_link_places_the_three_programs_as_pinned() {
    let p = gl_program().1;
    assert_eq!(p, Placed { fs: (FS_OFF, 17 * 8), vs: (VS_OFF, 27 * 8), cs: (CS_OFF, 18 * 8) });
}

/// The green quad's record is the case-5 shader's: the same program, no
/// varying added, so the same D0 bytes (tests/v3d71_case5_record.rs).
#[test]
fn the_green_quad_record_is_the_d0_record_of_the_color_uv_program() {
    let jobs = case58_jobs();
    let fd = &jobs[1].draws[0];
    let r = record_of(&gl_program().0, &CUV_ATTRS);
    c_copies_the_record(fd, &r);
    let b = r.pack();
    assert_eq!(r.fragment_shader_code_address, CODE_BASE + FS_OFF);
    assert_eq!(r.vertex_shader_code_address, CODE_BASE + VS_OFF);
    assert_eq!(r.coordinate_shader_code_address, CODE_BASE + CS_OFF);
    assert_eq!(&b[0..8], &[0x02, 0x80, 0x20, 0x06, 0x01, 0x00, 0x02, 0x00]);
    assert_eq!(word(&b, 2), CODE_BASE + FS_OFF + 0b01);
    assert_eq!(word(&b, 4), CODE_BASE + VS_OFF + 0b11);
    assert_eq!(word(&b, 6), CODE_BASE + CS_OFF + 0b11);
}

#[test]
fn the_clear_quad_records_use_the_clear_program() {
    let jobs = case58_jobs();
    for fd in [&jobs[0].draws[0], &jobs[1].draws[1]] {
        let r = record_of(&clear_program().0, &[QUAD_ATTR]);
        c_copies_the_record(fd, &r);
        assert_eq!(r.number_of_varyings_in_fragment_shader, 0);
        assert_eq!(r.fragment_shader_code_address, CLEAR_BASE);
    }
}

fn word(b: &[u8; 32], i: usize) -> u32 {
    u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]])
}
