// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host prediction for U11 (docs/notes/v3d71-record-u11.md section 5):
//! the BCM2712 D0 draw-index shader-state records gl_test case 5 (`scissor`,
//! driver/tools/pvrlab/gl_test.c:2357-2405) makes the V3D 7.1.10 read, built
//! from our own 7.1 link and the merged 7.1 modules, pinned field by field.
//!
//! Case 5 links `gl_test_vs_src` / `gl_test_fs_src` (the ES 1.00 twins
//! tools/shaders/gl_test_es100/color_uv.{vert,frag}; gl_test.c:2367), loads
//! six `cuv_vertex` (pos4 + colour4 + uv2, 40 bytes, gl_test.c:1386 and
//! 1400-1409), clears the 64x64 target (TW, TH: gl_test.c:1088-1089) to black
//! and finishes, then clears again with the scissor box (0, 0, 32, 32) and
//! draws `GL_TRIANGLES, 0, 6` (gl_test.c:2389-2393). Through `fill71` that is
//! two flushes: a `Clear Render Targets` job with no draw, then a job with
//! two draws, the scissored clear's quad (clearplan71: the tile buffer cannot
//! clear a box) and the yellow quad. Both draws' records are pinned; the
//! quad's is the one section 5 names.
//!
//! How the record is built, step by step:
//!  1. `link_sources71` links the program and the clear program
//!     (`yakogl::v3d::backend::CLEAR_VS` / `CLEAR_FS`) at 7.1, as
//!     tests/v3d71_link.rs does; its `ShcV3d` is the prog_data;
//!  2. this test maps that prog_data onto `fill71::Program` (the record's
//!     `record71::DrawDesc` and the C program fields). The map is the test's
//!     own: no src/ module maps a `ShcShader` onto fill71 yet (glue71 is that
//!     module's item); it copies every field the record reads one for one;
//!  3. `fill71::fill` turns case 5's GL calls into the jobs `v3d_job71`
//!     builds, as tests/v3d71_fill.rs does for `gl_scissor`;
//!  4. `admit71::admit_draw`, the call fill71's `build_draw` makes, gives the
//!     `record71::Record`, and `Record::pack` its 32 bytes. The test checks
//!     that each filled C program struct holds the value the record field
//!     copies (driver/src/v3d/v3d_job71.c:688-724, `emit_draw`'s D0 block).
//!
//! The code addresses are the program BO's base plus each program's offset in
//! the linked blob: the table places that BO at run time, so the pins are
//! the offsets (from [`CODE_BASE`], a stand-in for the BO base). The uniforms
//! addresses are where `v3d_job71_draw` writes each stream in the indirect
//! list (`lo->*_uniforms`, v3d_job71.c:672-679, 707-709), not a program value:
//! they are named, not pinned.
//!
//! Prefixes as in the note: `X:` is
//! third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml, `G:` is
//! driver/src/v3d/generated/v3d_packet_v71_pack.h; the XML positions are
//! read from the XML here, so a pin cannot drift from the file.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_case5_record
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

/* ---- step 3: case 5 through fill71 --------------------------------------- */

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

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const BOX: [i32; 4] = [0, 0, 32, 32];

fn the_draw() -> DrawIn<'static> {
    let mut st = draw71::DrawState::gl_default(SIZE, SIZE, false, 1);
    st.viewport = [0.0, 0.0, SIZE as f32, SIZE as f32];
    st.scissor = Some(BOX);
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

/// The two flushes of case 5 (gl_test.c:2387-2394).
fn case5_jobs() -> Vec<FilledJob> {
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
    // flush 1: glClear(GL_COLOR_BUFFER_BIT) to black, glFinish
    let flush1 = [Op::Clear(ClearReq::color(BLACK))];
    let mut jobs = fill71::fill(&fb, record71::BCM2712_D0, &fill71::History::NEW, &q, &flush1)
        .unwrap_or_else(|r| panic!("flush 1: {r:?}"));
    let hist = jobs.last().expect("flush 1 has a job").after;
    // flush 2: glScissor(0, 0, 32, 32), glClear, glDrawArrays, glFinish
    let flush2 = [Op::Clear(ClearReq { scissor: Some(BOX), ..ClearReq::color(BLACK) }), Op::Draw(Box::new(the_draw()))];
    jobs.extend(
        fill71::fill(&fb, record71::BCM2712_D0, &hist, &q, &flush2).unwrap_or_else(|r| panic!("flush 2: {r:?}")),
    );
    jobs
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

/// The program offsets of the link: FS first, then VS, then CS
/// (tests/v3d71_link.rs pins the order; the sizes are the word counts of
/// tools/v3d/gltest-shaders71/ours_color_uv.h: FS 17, VS 27, CS 18 words).
const FS_OFF: u32 = 0;
const VS_OFF: u32 = 17 * 8;
const CS_OFF: u32 = (17 + 27) * 8;

#[test]
fn case5_flushes_into_a_clear_job_and_a_two_draw_job() {
    let jobs = case5_jobs();
    assert_eq!(jobs.len(), 2, "two glFinish, one job each");
    assert!(jobs[0].draws.is_empty(), "the full clear is Clear Render Targets, no draw");
    assert_eq!(jobs[1].draws.len(), 2);
    assert!(jobs[1].draws[0].is_clear, "the scissored clear is a quad");
    assert!(!jobs[1].draws[1].is_clear);
    assert_eq!(jobs[1].draws[1].count, 6);
    assert_eq!(jobs[1].draws[1].prim, ffi71::V3D_PRIM_TRIANGLES);
}

#[test]
fn the_link_places_the_three_programs_as_pinned() {
    let p = gl_program().1;
    assert_eq!(p, Placed { fs: (FS_OFF, 17 * 8), vs: (VS_OFF, 27 * 8), cs: (CS_OFF, 18 * 8) });
}

/// The yellow quad's record (gl_test.c:2393, `glDrawArrays(GL_TRIANGLES, 0,
/// 6)`): the record section 5 says case 5 confirms.
#[test]
fn the_quad_record_is_the_predicted_d0_record() {
    let jobs = case5_jobs();
    let fd = &jobs[1].draws[1];
    let r = record_of(&gl_program().0, &CUV_ATTRS);
    c_copies_the_record(fd, &r);
    let b = r.pack();

    // -- the three code addresses (u11 s.5: "the three code addresses sit at
    //    the same offsets in both 7.1 layouts", X:1294-1310 / X:1357-1373;
    //    u11 s.4 item 2, the three-program model)
    assert_eq!(r.fragment_shader_code_address, CODE_BASE + FS_OFF); // X:1357, bit 67
    assert_eq!(r.vertex_shader_code_address, CODE_BASE + VS_OFF); // X:1363, bit 131
    assert_eq!(r.coordinate_shader_code_address, CODE_BASE + CS_OFF); // X:1369, bit 195
    for (name, want) in [
        ("Fragment Shader Code Address", CODE_BASE + FS_OFF),
        ("Vertex Shader Code Address", CODE_BASE + VS_OFF),
        ("Coordinate Shader Code Address", CODE_BASE + CS_OFF),
    ] {
        // a 29-bit field holds address bits 3..31 (G:4414's unpack << 3)
        assert_eq!(at(&b, D0, name) << 3, want, "{name} at the D0 layout");
        assert_eq!(at(&b, PLAIN, name) << 3, want, "{name} at the plain layout: the same bits");
    }
    // the words: code | 4-way (bit 0) | final section (bit 1)
    assert_eq!(word(&b, 2), CODE_BASE + FS_OFF + 0b01, "FS: 4-way, not in the final section");
    assert_eq!(word(&b, 4), CODE_BASE + VS_OFF + 0b11, "VS: 4-way, final section");
    assert_eq!(word(&b, 6), CODE_BASE + CS_OFF + 0b11, "CS: 4-way, final section");

    // -- the "Differences" trio (tools/v3d/pi5/README.md:72-83; u11 s.2
    //    :99-105 and s.5): read at the D0 layout, the values the program
    //    gives (the shim's)
    assert!(r.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2); // X:1333
    assert!(!r.insert_primitive_id_as_first_varying_to_fragment_shader); // X:1336
    assert!(r.disable_implicit_point_line_varyings); // X:1339
    let trio = [
        "Fragment shader uses real pixel centre W in addition to centroid W2",
        "Insert Primitive ID as first varying to fragment shader",
        "Disable implicit point/line varyings",
    ];
    let d0: Vec<u32> = trio.iter().map(|f| at(&b, D0, f)).collect();
    assert_eq!(d0, [1, 0, 1], "the trio at the D0 bits 15, 18, 21");
    assert_eq!(trio.map(|f| xml_pos(D0, f).0), [15, 18, 21]);
    // the same bytes read at the PLAIN 7.1 layout (bits 12, 15, 18:
    // X:1270, 1273, 1276) show what a plain-layout CLIF decode of the board's
    // record prints: 1 -> 0, 0 -> 1, 1 -> 0, the README's three changed flags
    let plain: Vec<u32> = trio.iter().map(|f| at(&b, PLAIN, f)).collect();
    assert_eq!(plain, [0, 1, 0], "the trio a plain decode shows");
    assert_eq!(trio.map(|f| xml_pos(PLAIN, f).0), [12, 15, 18]);

    // -- the FEP flag at 0 (u11 s.3 and s.4 item 4: Mesa's value for every
    //    shader; X:1341, bit 23 of the D0 record; plain X:1278, bit 20)
    assert!(!r.never_defer_fep_depth_writes);
    assert_eq!(at(&b, D0, "Never defer FEP depth writes"), 0);
    assert_eq!(xml_pos(D0, "Never defer FEP depth writes").0, 23);

    // -- the four draw-index bits at 0 (u11 s.4 item 3; X:1321-1328)
    for f in ["cs_basevertex", "cs_drawindex", "vs_basevertex", "vs_drawindex"] {
        assert_eq!(at(&b, D0, f), 0, "{f}");
    }
    // the early-z pair copied from the FS: 0, 0 (u11 s.4 item 4; X:1330-1331)
    assert!(!r.fragment_shader_does_z_writes && !r.turn_off_early_z_test);

    // -- the rest the board's dump will show, byte 3 on the same in both
    //    layouts (X:1343-1354 / X:1280-1291)
    assert!(r.enable_clipping); // X:1316, bit 1
    assert!(!r.point_size_in_shaded_vertex_data);
    assert_eq!(r.number_of_varyings_in_fragment_shader, 6); // v_color 4 + v_uv 2
    assert_eq!(
        (
            r.coordinate_shader_output_vpm_segment_size,
            r.coordinate_shader_input_vpm_segment_size,
            r.vertex_shader_output_vpm_segment_size,
            r.vertex_shader_input_vpm_segment_size,
        ),
        (1, 0, 2, 0)
    );
    // the flag bytes: clipping (bit 1), centre W (bit 15), implicit
    // varyings disabled (bit 21); byte 3: 6 varyings; bytes 4..8: CS out 1,
    // min 0 / CS in 0, min-1 0 / VS out 2 / VS in 0
    assert_eq!(&b[0..8], &[0x02, 0x80, 0x20, 0x06, 0x01, 0x00, 0x02, 0x00]);
    assert_eq!(word(&b, 0), 0x0620_8002);
    // the uniforms addresses are the builder's (v3d_job71.c:707-709): the
    // record71 description leaves them 0
    assert_eq!([word(&b, 3), word(&b, 5), word(&b, 7)], [0, 0, 0]);
}

/// The scissored clear's quad (gl_test.c:2392, clearplan71's quad): drawn
/// first in the job, so it is the first record the D0 reads in case 5.
#[test]
fn the_clear_quad_record_is_the_predicted_d0_record() {
    let jobs = case5_jobs();
    let fd = &jobs[1].draws[0];
    let r = record_of(&clear_program().0, &[QUAD_ATTR]);
    c_copies_the_record(fd, &r);
    let b = r.pack();
    let p = clear_program().1;
    assert_eq!(r.fragment_shader_code_address, CLEAR_BASE + p.fs.0);
    assert_eq!(r.vertex_shader_code_address, CLEAR_BASE + p.vs.0);
    assert_eq!(r.coordinate_shader_code_address, CLEAR_BASE + p.cs.0);
    assert_eq!((p.fs.0, p.vs.0, p.cs.0), (0, p.fs.1, p.fs.1 + p.vs.1));
    // no varyings, so no centre W; implicit varyings disabled; FEP 0
    let trio = [
        "Fragment shader uses real pixel centre W in addition to centroid W2",
        "Insert Primitive ID as first varying to fragment shader",
        "Disable implicit point/line varyings",
    ];
    assert_eq!(trio.map(|f| at(&b, D0, f)), [0, 0, 1]);
    assert_eq!(trio.map(|f| at(&b, PLAIN, f)), [0, 0, 0]);
    assert_eq!(at(&b, D0, "Never defer FEP depth writes"), 0);
    for f in ["cs_basevertex", "cs_drawindex", "vs_basevertex", "vs_drawindex"] {
        assert_eq!(at(&b, D0, f), 0, "{f}");
    }
    assert_eq!(r.number_of_varyings_in_fragment_shader, 0);
    assert_eq!(&b[0..3], &[0x02, 0x00, 0x20]);
}
