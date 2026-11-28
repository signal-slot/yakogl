// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host record of gl_test case 66 (`clearvp`, `case_clearvp`,
//! driver/tools/pvrlab/gl_test.c:9224-9307: glClear is not bounded by the
//! viewport), one of Raspberry Pi 5's six P9 cases, through the 7.1 table's
//! fill (`fill71`), as tests/v3d71_case5_record.rs does for case 5.
//!
//! Case 66 links no program of its own and draws nothing: three flushes of
//! `glClear(GL_COLOR_BUFFER_BIT)` on the 64x64 target (TW, TH):
//!  1. viewport (0, 0, 64, 64), white, no scissor;
//!  2. viewport (0, 0, 32, 32), blue: the viewport is not an input of the
//!     clear, so it is the same whole-target tile clear;
//!  3. viewport back to 64x64, scissor (0, 0, 32, 32), green: the tile buffer
//!     cannot clear a box, so clearplan71 draws the clear quad.
//! Pinned: three jobs; jobs 0 and 1 are `Clear Render Targets` jobs with no
//! draw and their clear colours; job 2 holds one draw, the clear quad (4
//! corners, a triangle fan, its D0 shader record the clear program's) with
//! the green clear colour; the quad record's key
//! fields; and the verdict.
//!
//! Expected board verdict: PASS (docs/notes/v3d71-gl-table.md, row "66 clear
//! not bounded by viewport"; no refusal: no shared 4.2 decision is reached).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_case66_record
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

use fill71::{ClearQuad, ClearReq, ColorBuffer, DsBuffer, FilledJob, Framebuffer, Op, Program};
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::link71::link_sources71;
use yakogl::shaderc::{LodKey, ShcShader, ShcV3dUniform, ShcV3dVs};
use yakogl::v3d::backend::{CLEAR_FS, CLEAR_VS};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The clear program's BO (a stand-in for its GPU address).
const CLEAR_BASE: u32 = 0x0021_0000;
const VTX: u32 = 0x0030_0000;
const RT0: u32 = 0x0100_0000;
const ZS: u32 = 0x0200_0000;
const TILE: u32 = 0x0040_0000;
/// TW, TH (gl_test.c:1088-1089).
const SIZE: u32 = 64;
const GL_FLOAT: u32 = 0x1406;

fn stream(u: *const ShcV3dUniform, n: u32) -> &'static [uniform71::Word] {
    if n == 0 {
        return &[];
    }
    // SAFETY: `n` entries alive while the keep is open; copied out here.
    let v: Vec<uniform71::Word> = unsafe { core::slice::from_raw_parts(u, n as usize) }
        .iter()
        .map(|u| uniform71::Word { contents: u.contents, data: u.data })
        .collect();
    Box::leak(v.into_boxed_slice())
}

fn c_source(s: &'static [u8]) -> &'static str {
    core::str::from_utf8(s.strip_suffix(b"\0").expect("NUL-terminated")).expect("UTF-8")
}

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// (offset, bytes) of the clear program's FS, VS, CS in its blob.
type Placed = [(u32, u32); 3];

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

fn header_define(rel: &str, name: &str) -> u32 {
    let h = read(rel);
    let l = h
        .lines()
        .find_map(|l| l.strip_prefix(&format!("#define {name} ")).map(str::to_string))
        .unwrap_or_else(|| panic!("{rel}: no {name}"));
    l.split("/*").next().unwrap().trim().parse().unwrap()
}

/// Links the clear program at 7.1 and maps its prog_data onto
/// `fill71::Program` (the same field-for-field map as case 5's test).
fn clear_program() -> &'static (Program<'static>, Placed) {
    static P: std::sync::OnceLock<(Program<'static>, Placed)> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let discards = header_define("tools/v3d/gltest-shaders71/ours_clear.h", "OURS71_CLEAR_FS_USES_DISCARD") != 0;
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _keep = store::begin_keep().expect("keep");
        let sh: ShcShader = store::with_scratch(|a| {
            link_sources71(
                c_source(CLEAR_VS).as_bytes(),
                None,
                LodKey::NONE,
                c_source(CLEAR_FS).as_bytes(),
                &[],
                a,
                store::keep(),
            )
            .map(|r| r.0)
        })
        .expect("scratch")
        .unwrap_or_else(|e| panic!("link71: {e:?}"));
        let p = &sh.v3d;
        assert_eq!(p.linked, 1);
        let placed =
            [(p.fs.code_off, p.fs.code_bytes), (p.vs.code_off, p.vs.code_bytes), (p.cs.code_off, p.cs.code_bytes)];
        let base = CLEAR_BASE;
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
        let desc = record71::DrawDesc {
            fs,
            vs: vdesc(base, &p.vs),
            cs: vdesc(base, &p.cs),
            point_size: p.vs.writes_psiz != 0,
        };
        let prog = Program {
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
        };
        (prog, placed)
    })
}

const QUAD_ATTR: attr71::Attr = attr71::Attr {
    enabled: true,
    components: 4,
    gl_type: GL_FLOAT,
    normalized: false,
    integer: false,
    stride: 16,
    offset: 0,
    buffer_addr: VTX + 0x100,
    divisor: 0,
    generic: [0, 0, 0, 0x3f80_0000],
    constant_addr: None,
};

const BOX: [i32; 4] = [0, 0, 32, 32];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// The three flushes of case 66. The viewport (steps 1 and 2) is not an
/// input of `ClearReq`: that is the case's point, pinned below.
fn case66_jobs() -> Result<Vec<FilledJob>, fill71::Refusal> {
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
    let q = ClearQuad { program: &clear_program().0, attr: QUAD_ATTR, dummy_addr: VTX };
    let flushes =
        [ClearReq::color(WHITE), ClearReq::color(BLUE), ClearReq { scissor: Some(BOX), ..ClearReq::color(GREEN) }];
    let mut jobs: Vec<FilledJob> = Vec::new();
    let mut hist = fill71::History::NEW;
    for c in flushes {
        let j = fill71::fill(&fb, record71::BCM2712_D0, &hist, &q, &[Op::Clear(c)])?;
        if let Some(l) = j.last() {
            hist = l.after;
        }
        jobs.extend(j);
    }
    Ok(jobs)
}

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

fn at(b: &[u8; 32], field: &str) -> u32 {
    let (s, n) = xml_pos(D0, field);
    (0..n).fold(0, |v, k| {
        let bit = s + k;
        v | (u32::from(b[(bit / 8) as usize] >> (bit % 8) & 1) << k)
    })
}

fn word(b: &[u8; 32], i: usize) -> u32 {
    u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]])
}

/* ---- the pins ---------------------------------------------------------------- */

#[test]
fn case66_is_not_refused_so_the_board_verdict_is_pass() {
    // no named refusal: the whole case fills (59 and 67 are the refused ones)
    let jobs = case66_jobs().unwrap_or_else(|r| panic!("case 66 refused: {r:?}"));
    assert!(!jobs.is_empty());
}

#[test]
fn case66_records_three_jobs_two_tile_clears_and_one_quad() {
    let jobs = case66_jobs().unwrap();
    assert_eq!(jobs.len(), 3, "three glFinish, one job each");
    // steps 1 and 2: Clear Render Targets, no draw; the viewport changes nothing
    assert!(jobs[0].draws.is_empty());
    assert!(jobs[1].draws.is_empty());
    assert_eq!(jobs[0].target.rts, 1);
    assert_eq!((jobs[0].target.width, jobs[0].target.height), (SIZE, SIZE));
    // the clear colours differ (white, blue): the second job's clear is its own
    assert_ne!(jobs[0].clear.color[0], jobs[1].clear.color[0]);
    // step 3: the scissored clear is one quad draw: 4 corners, a fan
    assert_eq!(jobs[2].draws.len(), 1);
    let d = &jobs[2].draws[0];
    assert!(d.is_clear);
    assert_eq!(d.prim, ffi71::V3D_PRIM_TRIANGLE_FAN);
    assert_eq!(d.count, 4);
    assert!(d.quad.is_some());
}

/// The full-viewport and quarter-viewport clears are one plan: jobs 0 and 1
/// differ only in the clear colour.
#[test]
fn case66_viewport_does_not_change_the_clear_job() {
    let jobs = case66_jobs().unwrap();
    let (a, b) = (&jobs[0], &jobs[1]);
    assert_eq!(a.target, b.target);
    let mut ca = a.clear;
    ca.color = b.clear.color;
    assert_eq!(ca, b.clear);
    // the clear colour is not zero-filled: white and blue pack differently
    assert_ne!(a.clear.color[0], [0; 4]);
    assert_ne!(b.clear.color[0], [0; 4]);
}

#[test]
fn case66_quad_record_is_the_clear_programs_d0_record() {
    let jobs = case66_jobs().unwrap();
    let fd = &jobs[2].draws[0];
    let (prog, placed) = clear_program();
    let desc = &prog.desc;
    let r = admit71::admit_draw(
        record71::BCM2712_D0,
        &admit71::Draw {
            attrs: &[QUAD_ATTR],
            cs_reads: prog.cs_reads,
            vs_reads: prog.vs_reads,
            cs_builtins: desc.cs.uses_vid || desc.cs.uses_iid,
            dummy_addr: VTX,
            separate_segments: prog.separate_segments,
            programs: *desc,
        },
    )
    .unwrap_or_else(|r| panic!("admit71: {r:?}"))
    .record;
    // the filled C program structs hold what the record copies
    assert_eq!(fd.fs.code, r.fragment_shader_code_address);
    assert_eq!(fd.vs.code, r.vertex_shader_code_address);
    assert_eq!(fd.cs.code, r.coordinate_shader_code_address);
    assert_eq!(fd.fs.num_inputs, r.number_of_varyings_in_fragment_shader);
    assert_eq!(fd.vs_writes_psiz, r.point_size_in_shaded_vertex_data);
    assert_eq!(fd.fs.writes_z, r.fragment_shader_does_z_writes);
    assert_eq!(fd.fs.disable_ez, r.turn_off_early_z_test);
    let b = r.pack();

    // the clear program's blob order is FS, VS, CS
    let (fs, vs, cs) = (placed[0], placed[1], placed[2]);
    assert_eq!((fs.0, vs.0, cs.0), (0, fs.1, fs.1 + vs.1));
    assert_eq!(r.fragment_shader_code_address, CLEAR_BASE + fs.0);
    assert_eq!(r.vertex_shader_code_address, CLEAR_BASE + vs.0);
    assert_eq!(r.coordinate_shader_code_address, CLEAR_BASE + cs.0);
    for (name, want) in [
        ("Fragment Shader Code Address", CLEAR_BASE + fs.0),
        ("Vertex Shader Code Address", CLEAR_BASE + vs.0),
        ("Coordinate Shader Code Address", CLEAR_BASE + cs.0),
    ] {
        assert_eq!(at(&b, name) << 3, want, "{name}");
    }
    // the D0 trio: no centre W, no primitive id, implicit varyings disabled
    assert_eq!(at(&b, "Fragment shader uses real pixel centre W in addition to centroid W2"), 0);
    assert_eq!(at(&b, "Insert Primitive ID as first varying to fragment shader"), 0);
    assert_eq!(at(&b, "Disable implicit point/line varyings"), 1);
    assert_eq!(at(&b, "Never defer FEP depth writes"), 0);
    for f in ["cs_basevertex", "cs_drawindex", "vs_basevertex", "vs_drawindex"] {
        assert_eq!(at(&b, f), 0, "{f}");
    }
    assert!(r.enable_clipping);
    assert_eq!(r.number_of_varyings_in_fragment_shader, 0);
    assert_eq!(&b[0..3], &[0x02, 0x00, 0x20]);
    assert_eq!(word(&b, 3), 0, "uniforms addresses are the builder's");
}
