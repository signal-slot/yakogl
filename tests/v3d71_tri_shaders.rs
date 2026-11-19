// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Our V3D 7.1 (Raspberry Pi 5) triangle shaders: the pass-through pair
//! `tools/shaders/gl_test_es100/passthrough.vert` + `interp_color.frag`
//! compiled by `chain71::compile_program71` for the board's VPM, committed as
//!
//! * `tools/v3d/tri-shaders71/ours_passthrough_interp_color.json`: the
//!   4.2 file's shape (`tools/v3d/tri-shaders/ours_passthrough_interp_color.json`,
//!   written by chain.rs `v3d_shc_json`, which is v3d_shc's JSON key for key)
//!   with `gpu.ver` 71, plus `record71`: the values the BCM2712 D0 shader
//!   record (`GL Shader State Record Draw Index`, tools/v3d/pi5/README.md
//!   "Differences" item 4) takes from the three programs, by Mesa's field
//!   names and as Mesa 26.2.1 v3dx_draw.c:477-547 derives them;
//! * `tools/v3d/tri-shaders71/ours_passthrough_interp_color.h`: the same
//!   program as a C header (its comment describes the shape).
//!
//! Both must be what the compiler produces now, byte for byte, or a board
//! run would test a compiler that no longer exists. Regenerate with
//!
//! ```text
//! V3D_WRITE_OURS_JSON=1 cargo test --no-default-features \
//!   --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_tri_shaders
//! ```
//!
//! (the variable the 4.2 file's generator uses), then run it again without
//! the variable: the committed bytes are compiled in (`include_str!`), so
//! the writing run does not compare. The words are also checked: each
//! decodes with codec71 to its instruction and re-encodes to itself,
//! verify71 and `registers_fit_threads71` accept every stage, and the 7.1
//! simulator (`sim::run_gen(Gen::V71, ..)`, sim71) runs them: the vertex
//! shader writes the scaled position, Zs, 1/Wc and the colour varying, the
//! coordinate shader the clip and scaled position, and the fragment shader
//! the interpolated colour (through `tests/v3d71_common`'s runner). sim71
//! does not yet run a vertex or coordinate shader to its end (see
//! `run_vertex`): until it does, those two print a GAP and check the words
//! only.
//!
//! Feature set: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

mod v3d71_common;

use std::fmt::Write;

use v3d71_common::{run71_stream, Shape};
use yakogl::shaderc::online::ir::{IoKind, Module};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::{attribute_locations, Prim};
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, V3dProgram71, VsProg71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71, same_instr71};
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Instr71, MulOp71};
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_CONSTANT, QUNIFORM_VIEWPORT_X_SCALE, QUNIFORM_VIEWPORT_Y_SCALE, QUNIFORM_VIEWPORT_Z_OFFSET,
    QUNIFORM_VIEWPORT_Z_SCALE,
};
use yakogl::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

static PASSTHROUGH_VS: &str = include_str!("../tools/shaders/gl_test_es100/passthrough.vert");
static INTERP_FS: &str = include_str!("../tools/shaders/gl_test_es100/interp_color.frag");

const JSON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/v3d/tri-shaders71/ours_passthrough_interp_color.json");
const HEADER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/v3d/tri-shaders71/ours_passthrough_interp_color.h");
static COMMITTED_JSON: &str = include_str!("../tools/v3d/tri-shaders71/ours_passthrough_interp_color.json");
static COMMITTED_HEADER: &str = include_str!("../tools/v3d/tri-shaders71/ours_passthrough_interp_color.h");

/// The C identifier prefix of the header.
const PREFIX: &str = "ours71_tri";

/// The viewport the simulator runs the vertex and coordinate shaders with
/// (the board's 64x64 target: half the size in 1/64 pixel, y down; depth
/// range 0..1), so the scaled position is an exact integer.
const X_SCALE: f32 = 2048.0;
const Y_SCALE: f32 = -2048.0;
const Z_SCALE: f32 = 0.5;
const Z_OFFSET: f32 = 0.5;

fn b(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

/// One stage: its name, words and uniform stream `(contents, data)`.
type StageCode = (&'static str, Vec<u64>, Vec<(u32, u32)>);

/// The two files' text and what the simulator runs.
struct Compiled {
    json: String,
    header: String,
    /// Stage name, words, uniform stream.
    stages: Vec<StageCode>,
    vs_vattr_sizes: [u8; 16],
    cs_vattr_sizes: [u8; 16],
    fs_input_slots: Vec<(u8, u8)>,
}

/// The D0 shader record's fields that come from the programs, in
/// v3dx_draw.c:477-547's order, by Mesa's field names. The constant
/// fields (`enable_clipping` true, `point_size_in_shaded_vertex_data` false
/// for a triangle draw), the addresses and the VPM configuration's `As` and
/// `Ve` (v3d_compute_vpm_config, from the VPM sizes and vcm_cache_size
/// below) are the frame code's.
fn record71(p: &V3dProgram71<'_>) -> Vec<(&'static str, u32)> {
    let f = &p.fs;
    vec![
        ("fragment_shader_does_z_writes", f.writes_z as u32),
        ("turn_off_early_z_test", f.disable_ez as u32),
        ("fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2", f.uses_center_w as u32),
        ("any_shader_reads_hardware_written_primitive_id", f.uses_pid as u32),
        ("insert_primitive_id_as_first_varying_to_fragment_shader", f.uses_pid as u32),
        ("do_scoreboard_wait_on_first_thread_switch", f.lock_scoreboard_on_first_thrsw as u32),
        ("disable_implicit_point_line_varyings", !f.uses_implicit_point_line_varyings as u32),
        ("number_of_varyings_in_fragment_shader", f.num_inputs),
        ("coordinate_shader_input_vpm_segment_size", p.cs.vpm_input_size),
        ("vertex_shader_input_vpm_segment_size", p.vs.vpm_input_size),
        ("coordinate_shader_output_vpm_segment_size", p.cs.vpm_output_size),
        ("vertex_shader_output_vpm_segment_size", p.vs.vpm_output_size),
        ("coordinate_shader_4_way_threadable", (p.cs.threads == 4) as u32),
        ("vertex_shader_4_way_threadable", (p.vs.threads == 4) as u32),
        ("fragment_shader_4_way_threadable", (f.threads == 4) as u32),
        ("coordinate_shader_start_in_final_thread_section", p.cs.single_seg as u32),
        ("vertex_shader_start_in_final_thread_section", p.vs.single_seg as u32),
        ("fragment_shader_start_in_final_thread_section", f.single_seg as u32),
        ("vertex_id_read_by_coordinate_shader", p.cs.uses_vid as u32),
        ("instance_id_read_by_coordinate_shader", p.cs.uses_iid as u32),
        ("vertex_id_read_by_vertex_shader", p.vs.uses_vid as u32),
        ("instance_id_read_by_vertex_shader", p.vs.uses_iid as u32),
    ]
}

/// One `uniforms` / `code` tail of a stage, chain.rs `v3d_shc_json`'s.
fn tail(o: &mut String, c: &Code71<'_>) {
    o.push_str("\n    \"uniforms\": [");
    for (i, u) in c.uniforms.iter().enumerate() {
        let sep = if i > 0 { ", " } else { "" };
        write!(o, "{sep}{{ \"contents\": {}, \"data\": \"0x{:08x}\" }}", u.contents, u.data).unwrap();
    }
    o.push_str("],\n    \"code\": [");
    for (i, w) in c.words.iter().enumerate() {
        write!(o, "{}\"0x{w:016x}\"", if i > 0 { ", " } else { "" }).unwrap();
    }
    o.push_str("] }");
}

/// Each attribute declaration's records, as chain.rs `v3d_shc_json` lists
/// them (one per column, `None` for a column not read).
fn attributes(p: &V3dProgram71<'_>, vs: &Module<'_>) -> Vec<(String, u8, Vec<Option<u8>>)> {
    vs.inputs
        .iter()
        .filter(|d| d.kind == IoKind::Attribute)
        .map(|d| {
            let recs = (0..attribute_locations(d.gl_type))
                .map(|k| p.attributes.iter().find(|e| e.0 == d.location + k).and_then(|e| e.1))
                .collect();
            (d.name.to_string(), d.location, recs)
        })
        .collect()
}

/// `p` in the 4.2 file's JSON shape (chain.rs `v3d_shc_json`, line for
/// line, `gpu.ver` 71), then `record71`.
fn json71(p: &V3dProgram71<'_>, vs: &Module<'_>, fs: &Module<'_>) -> String {
    let mut o = String::new();
    write!(
        o,
        "{{\n  \"gpu\": {{ \"ver\": 71, \"vpm_size\": {}, \"vpm_size_source\": \"given\", \"prim\": \"triangles\", \
         \"per_vertex_point_size\": false, \"compiler\": \"yakogl-online\" }},\n",
        TARGET71.vpm_size
    )
    .unwrap();
    o.push_str("  \"uniform_names\": [");
    let mut n = 0;
    for (stage, m) in [("vert", vs), ("frag", fs)] {
        for u in m.uniforms {
            let sep = if n > 0 { ", " } else { "" };
            write!(
                o,
                "{sep}{{ \"stage\": \"{stage}\", \"name\": \"{}\", \"word\": {}, \"words\": {} }}",
                u.name, u.dword_off, u.dwords
            )
            .unwrap();
            n += 1;
        }
    }
    o.push_str("],\n  \"attributes\": [");
    for (i, (name, loc, recs)) in attributes(p, vs).iter().enumerate() {
        let recs: Vec<String> = recs.iter().map(|r| r.map_or("null".into(), |r| r.to_string())).collect();
        let sep = if i > 0 { ", " } else { "" };
        write!(o, "{sep}{{ \"name\": \"{name}\", \"location\": {loc}, \"records\": [{}] }}", recs.join(", ")).unwrap();
    }
    o.push_str("],\n  \"fs\": { ");
    let f = &p.fs;
    write!(
        o,
        "\"threads\": {}, \"single_seg\": {}, \"spill_size\": 0, \"tmu_dirty_rcl\": {}, \"num_inputs\": {}, \
         \"input_slots\": [",
        f.threads,
        b(f.single_seg),
        b(f.tmu_dirty_rcl),
        f.num_inputs
    )
    .unwrap();
    for (i, (slot, comp)) in f.input_slots.iter().enumerate() {
        write!(o, "{}{{ \"slot\": {slot}, \"component\": {comp} }}", if i > 0 { ", " } else { "" }).unwrap();
    }
    write!(
        o,
        "], \"flat_shade_flags\": \"0x{:08x}\", \"noperspective_flags\": \"0x{:08x}\", \"centroid_flags\": \"0x{:08x}\", \
         \"writes_z\": {}, \"disable_ez\": {}, \"uses_center_w\": {}, \"lock_scoreboard_on_first_thrsw\": {}, \
         \"uses_discard\": {}, \"uses_implicit_point_line_varyings\": {}, \"uses_pid\": {}, \"force_per_sample_msaa\": {},",
        f.flat_shade_flags,
        f.noperspective_flags,
        f.centroid_flags,
        b(f.writes_z),
        b(f.disable_ez),
        b(f.uses_center_w),
        b(f.lock_scoreboard_on_first_thrsw),
        b(f.uses_discard),
        b(f.uses_implicit_point_line_varyings),
        b(f.uses_pid),
        b(f.force_per_sample_msaa)
    )
    .unwrap();
    tail(&mut o, &f.code);
    o.push_str(",\n");
    for (name, v) in [("vs", &p.vs), ("cs", &p.cs)] {
        write!(
            o,
            "  \"{name}\": {{ \"threads\": {}, \"single_seg\": {}, \"spill_size\": 0, \"tmu_dirty_rcl\": {}, \
             \"vpm_input_size\": {}, \"vpm_output_size\": {}, \"separate_segments\": {}, \"vcm_cache_size\": {}, \
             \"uses_vid\": {}, \"uses_iid\": {}, \"uses_biid\": {}, \"writes_psiz\": {}, \"vattr_sizes\": [",
            v.threads,
            b(v.single_seg),
            b(v.tmu_dirty_rcl),
            v.vpm_input_size,
            v.vpm_output_size,
            b(v.separate_segments),
            v.vcm_cache_size,
            b(v.uses_vid),
            b(v.uses_iid),
            b(v.uses_biid),
            b(v.writes_psiz)
        )
        .unwrap();
        let sizes: Vec<String> = v.vattr_sizes.iter().map(|s| s.to_string()).collect();
        o.push_str(&sizes.join(", "));
        o.push_str("],");
        tail(&mut o, &v.code);
        o.push_str(",\n");
    }
    o.push_str("  \"record71\": { \"layout\": \"GL Shader State Record Draw Index (BCM2712 D0, V3D 7.1.10)\"");
    for (k, v) in record71(p) {
        write!(o, ",\n    \"{k}\": {v}").unwrap();
    }
    o.push_str(" }\n}\n");
    o
}

const HEADER_COMMENT: &str = "\
/*
 * Our V3D 7.1 pass-through triangle program: tools/shaders/gl_test_es100/
 * passthrough.vert + interp_color.frag, compiled by YakoGL's online
 * compiler (src/shaderc/online/v3d/chain71.rs, compile_program71) for the
 * Raspberry Pi 5 (BCM2712 D0, V3D 7.1.10). The same program as
 * ours_passthrough_interp_color.json beside it.
 *
 * Generated by tests/v3d71_tri_shaders.rs, which also compares it byte for
 * byte with the compiler's output; regenerate with V3D_WRITE_OURS_JSON=1
 * (that test's header). Do not edit.
 *
 * Shape (self-contained: <stdint.h> only, no driver header; <P> is the
 * prefix, ours71_tri here):
 *   <P>_{fs,vs,cs}_code[]       const uint64_t, the QPU words, 8-byte aligned
 *   <P>_{FS,VS,CS}_NUM_WORDS    their counts
 *   <P>_{fs,vs,cs}_uniforms[][2] const uint32_t { contents, data }: the
 *                               uniform stream (Mesa's quniform_contents;
 *                               driver/src/v3d/v3d_frame.h V3D_QUNIFORM_*),
 *                               absent when <P>_<S>_NUM_UNIFORMS is 0
 *   <P>_<S>_<FIELD>             the prog_data the frame code reads
 *                               (struct v3d_fs_prog / v3d_vs_prog fields)
 *   <P>_<S>_VATTR_SIZES         VS and CS: the 16 vattr_sizes, an initializer
 *   <P>_ATTR_<NAME>_RECORD      each read attribute's record (its location in
 *                               the comment); <P>_NUM_ATTRS the records
 *   <P>_REC_<FIELD>             the BCM2712 D0 shader record's
 *                               (GL Shader State Record Draw Index) fields
 *                               that come from the programs, Mesa's names,
 *                               as Mesa 26.2.1 v3dx_draw.c:477-547 derives
 *                               them. enable_clipping, the point-size bit,
 *                               the addresses and the VPM configuration's
 *                               As and Ve are the frame code's.
 */
";

/// The header: see HEADER_COMMENT.
fn header71(p: &V3dProgram71<'_>, vs: &Module<'_>) -> String {
    let n = PREFIX;
    let u = PREFIX.to_uppercase();
    let mut o = String::new();
    o.push_str("/* SPDX-License-Identifier: Apache-2.0 */\n/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */\n");
    o.push_str(HEADER_COMMENT);
    writeln!(o, "#ifndef {u}_PROG_H\n#define {u}_PROG_H\n\n#include <stdint.h>\n").unwrap();
    writeln!(o, "#define {u}_COMPILER \"yakogl-online\"").unwrap();
    writeln!(o, "#define {u}_VER 71").unwrap();
    writeln!(o, "#define {u}_VPM_SIZE {}", TARGET71.vpm_size).unwrap();
    let attrs = attributes(p, vs);
    let nrec = attrs.iter().flat_map(|a| a.2.iter().flatten()).map(|r| r + 1).max().unwrap_or(0);
    writeln!(o, "#define {u}_NUM_ATTRS {nrec}").unwrap();
    for (name, loc, recs) in &attrs {
        for (k, r) in recs.iter().enumerate() {
            let nm = if recs.len() > 1 { format!("{}_{k}", name.to_uppercase()) } else { name.to_uppercase() };
            match r {
                Some(r) => writeln!(o, "#define {u}_ATTR_{nm}_RECORD {r} /* location {} */", *loc as usize + k),
                None => writeln!(o, "/* attribute {name} (location {}) is not read */", *loc as usize + k),
            }
            .unwrap();
        }
    }
    o.push('\n');
    let code = |o: &mut String, s: &str, c: &Code71<'_>| {
        let su = s.to_uppercase();
        writeln!(o, "#define {u}_{su}_NUM_WORDS {}", c.words.len()).unwrap();
        writeln!(o, "static const uint64_t {n}_{s}_code[{}] __attribute__((aligned(8))) = {{", c.words.len()).unwrap();
        for w in c.words {
            writeln!(o, "\t0x{w:016x}ull,").unwrap();
        }
        o.push_str("};\n");
        writeln!(o, "#define {u}_{su}_NUM_UNIFORMS {}", c.uniforms.len()).unwrap();
        if !c.uniforms.is_empty() {
            writeln!(o, "static const uint32_t {n}_{s}_uniforms[{}][2] = {{", c.uniforms.len()).unwrap();
            for x in c.uniforms {
                writeln!(o, "\t{{ {}, 0x{:08x}u }},", x.contents, x.data).unwrap();
            }
            o.push_str("};\n");
        }
    };
    let f = &p.fs;
    code(&mut o, "fs", &f.code);
    for (k, v) in [
        ("THREADS", f.threads),
        ("SINGLE_SEG", f.single_seg as u32),
        ("NUM_INPUTS", f.num_inputs),
        ("FLAT_SHADE_FLAGS", f.flat_shade_flags),
        ("NOPERSPECTIVE_FLAGS", f.noperspective_flags),
        ("CENTROID_FLAGS", f.centroid_flags),
        ("WRITES_Z", f.writes_z as u32),
        ("DISABLE_EZ", f.disable_ez as u32),
        ("USES_CENTER_W", f.uses_center_w as u32),
        ("LOCK_SCOREBOARD_ON_FIRST_THRSW", f.lock_scoreboard_on_first_thrsw as u32),
        ("USES_DISCARD", f.uses_discard as u32),
        ("USES_IMPLICIT_POINT_LINE_VARYINGS", f.uses_implicit_point_line_varyings as u32),
        ("USES_PID", f.uses_pid as u32),
        ("FORCE_PER_SAMPLE_MSAA", f.force_per_sample_msaa as u32),
        ("TMU_DIRTY_RCL", f.tmu_dirty_rcl as u32),
    ] {
        writeln!(o, "#define {u}_FS_{k} {v}").unwrap();
    }
    o.push('\n');
    let stage = |o: &mut String, s: &str, v: &VsProg71<'_>| {
        code(o, s, &v.code);
        let su = s.to_uppercase();
        for (k, x) in [
            ("THREADS", v.threads),
            ("SINGLE_SEG", v.single_seg as u32),
            ("VPM_INPUT_SIZE", v.vpm_input_size),
            ("VPM_OUTPUT_SIZE", v.vpm_output_size),
            ("SEPARATE_SEGMENTS", v.separate_segments as u32),
            ("VCM_CACHE_SIZE", v.vcm_cache_size),
            ("USES_VID", v.uses_vid as u32),
            ("USES_IID", v.uses_iid as u32),
            ("USES_BIID", v.uses_biid as u32),
            ("WRITES_PSIZ", v.writes_psiz as u32),
            ("TMU_DIRTY_RCL", v.tmu_dirty_rcl as u32),
        ] {
            writeln!(o, "#define {u}_{su}_{k} {x}").unwrap();
        }
        let sizes: Vec<String> = v.vattr_sizes.iter().map(|s| s.to_string()).collect();
        writeln!(o, "#define {u}_{su}_VATTR_SIZES {{ {} }}\n", sizes.join(", ")).unwrap();
    };
    stage(&mut o, "vs", &p.vs);
    stage(&mut o, "cs", &p.cs);
    o.push_str("/* GL Shader State Record Draw Index (BCM2712 D0), from the programs */\n");
    for (k, v) in record71(p) {
        writeln!(o, "#define {u}_REC_{} {v}", k.to_uppercase()).unwrap();
    }
    writeln!(o, "\n#endif /* {u}_PROG_H */").unwrap();
    o
}

/// One stage's words decode to its instructions and re-encode to
/// themselves, and verify71 accepts them at the stage's thread count.
fn check_stage(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
        assert_eq!(encode71(&back), Ok(w), "{what}: word {n} does not re-encode to itself");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
    assert!(c.back_edges.is_empty() && c.fwd_branches.is_empty(), "{what}: a straight-line program has a branch");
}

/// Compile the pair at 7.1 (once: the scratch region is one per process),
/// check every stage, and produce both files.
fn compile() -> &'static Compiled {
    static ONCE: std::sync::OnceLock<Compiled> = std::sync::OnceLock::new();
    ONCE.get_or_init(compile_once)
}

fn compile_once() -> Compiled {
    store::with_scratch(|a| {
        let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(PASSTHROUGH_VS).unwrap())
            .unwrap_or_else(|e| panic!("passthrough.vert: {}", e.msg));
        let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(INTERP_FS).unwrap())
            .unwrap_or_else(|e| panic!("interp_color.frag: {}", e.msg));
        let v = &a.alloc_filled(1, v.module).unwrap()[0];
        let f = &a.alloc_filled(1, f.module).unwrap()[0];
        let p = compile_program71(v, f, Prim::Triangles, TARGET71, a)
            .unwrap_or_else(|e| panic!("passthrough+interp_color at 7.1: {e:?}"));
        check_stage("fs", &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage("vs", &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage("cs", &p.cs.code, p.cs.threads, p.cs.single_seg);
        /* what the JSON (v3d_shc's shape) cannot carry is checked absent */
        let fs = &p.fs;
        assert_eq!((fs.flat_shade_flags_hi, fs.noperspective_flags_hi, fs.centroid_flags_hi), (0, 0, 0));
        assert_eq!(fs.rt_mask, 1, "the FS writes RT0 only");
        let uni = |c: &Code71<'_>| c.uniforms.iter().map(|u| (u.contents, u.data)).collect::<Vec<_>>();
        Compiled {
            json: json71(&p, v, f),
            header: header71(&p, v),
            stages: vec![
                ("fs", p.fs.code.words.to_vec(), uni(&p.fs.code)),
                ("vs", p.vs.code.words.to_vec(), uni(&p.vs.code)),
                ("cs", p.cs.code.words.to_vec(), uni(&p.cs.code)),
            ],
            vs_vattr_sizes: p.vs.vattr_sizes,
            cs_vattr_sizes: p.cs.vattr_sizes,
            fs_input_slots: p.fs.input_slots.to_vec(),
        }
    })
    .expect("scratch")
}

#[test]
fn the_committed_files_are_this_compilers_output() {
    let c = compile();
    /* The committed bytes are compiled into the binary (include_str!), so
     * the test cache's key follows them; the regeneration writes the files
     * and asks for a rebuild, which then compares. */
    for (path, text, committed) in [(JSON, &c.json, COMMITTED_JSON), (HEADER, &c.header, COMMITTED_HEADER)] {
        if std::env::var_os("V3D_WRITE_OURS_JSON").is_some() {
            std::fs::write(path, text).unwrap();
            println!("wrote {path} ({} bytes); run again without V3D_WRITE_OURS_JSON to compare", text.len());
            continue;
        }
        assert!(committed == *text, "{path} is not this compiler's output; regenerate it (see this test's header)");
    }
}

/// A stream with every contents code this program reads resolved.
fn resolve(stage: &str, uniforms: &[(u32, u32)]) -> Vec<u32> {
    uniforms
        .iter()
        .map(|&(c, d)| match c {
            QUNIFORM_CONSTANT => d,
            QUNIFORM_VIEWPORT_X_SCALE => X_SCALE.to_bits(),
            QUNIFORM_VIEWPORT_Y_SCALE => Y_SCALE.to_bits(),
            QUNIFORM_VIEWPORT_Z_OFFSET => Z_OFFSET.to_bits(),
            QUNIFORM_VIEWPORT_Z_SCALE => Z_SCALE.to_bits(),
            c => panic!("{stage}: uniform contents {c} (data {d:#x}) is not one a pass-through triangle reads"),
        })
        .collect()
}

/// Exact in f32 and f16: lane l's position and colour.
fn pos(l: usize) -> [f32; 4] {
    [(l as f32 - 8.0) / 16.0, (8.0 - l as f32) / 32.0, (l % 4) as f32 / 8.0, 1.0]
}
fn colour(l: usize) -> [f32; 4] {
    [l as f32 / 16.0, (15 - l) as f32 / 16.0, ((l * 5) % 16) as f32 / 16.0, 1.0]
}

/// The VS and CS input rows: record 0 (a_pos) then record 1 (a_color), each
/// as many rows as its vattr_sizes entry.
fn rows(sizes: &[u8; 16]) -> Vec<[u32; LANES]> {
    assert!(sizes[2..].iter().all(|&s| s == 0), "{sizes:?}: two records");
    let mut r = Vec::new();
    for (rec, f) in [(0, pos as fn(usize) -> [f32; 4]), (1, colour)] {
        for c in 0..sizes[rec] as usize {
            r.push(core::array::from_fn(|l| f(l)[c].to_bits()));
        }
    }
    r
}

/// Run the VS or CS in sim71 and return its VPM output rows, or `None` when
/// sim71 stops at the one place it is known not to model yet:
/// `sim71::alu_rf_writes` counts every non-NOP operation as writing its
/// waddr, also STVPMV, which has no destination, so a thread end of
/// `stvpmv ; thrsw` (Mesa's own 7.1 VS and CS end so, tools/v3d/pi5/
/// triangle.clif; qpu_validate.c:373-420 and verify71 accept it) stops as
/// "an ALU register-file write at the thread end" (tests/v3d_sim_mesa.rs
/// SIM71_UNMODELLED, where every Mesa 7.1 pair stops). Only that refusal,
/// at that instruction, is let through, printed as a GAP; once sim71 reads
/// the thread end as verify71 does, the rows are checked with no change
/// here.
fn run_vertex(stage: &str, c: &Compiled, sizes: &[u8; 16]) -> Option<Vec<[u32; LANES]>> {
    let (_, words, uniforms) = c.stages.iter().find(|s| s.0 == stage).unwrap();
    let env = Env { threads: 4, ..Env::default() };
    let inputs = Inputs::Vertex(VsInputs { vpm: rows(sizes) });
    let o = match sim::run_gen(Gen::V71, words, &resolve(stage, uniforms), &inputs, &env) {
        Ok(o) => o,
        Err(e) => {
            let end = words.len() - 3;
            let known = format!("instruction {end}: an ALU register-file write at the thread end");
            let Ok(Instr71::Alu(alu)) = decode71(words[end]) else { panic!("{stage}: word {end} is not an ALU word") };
            assert!(
                e == known && alu.sig.thrsw && alu.add.op == AddOp71::Stvpmv && alu.mul.op == MulOp71::Nop,
                "{stage}: sim71: {e}"
            );
            println!("GAP {stage}: sim71 stops at the thread end's STVPMV ({e}); the VPM rows are not checked");
            return None;
        }
    };
    assert!(o.tlb.is_empty(), "{stage}: a TLB write");
    Some(
        o.vpm_out
            .iter()
            .enumerate()
            .map(|(r, x)| x.unwrap_or_else(|| panic!("{stage}: VPM row {r} never written")))
            .collect(),
    )
}

/// The scaled coordinate at 7.1: round to nearest even
/// (v3d_nir_lower_io.c:529-543).
fn scaled(v: f32, s: f32) -> u32 {
    (v * s).round_ties_even() as i32 as u32
}

#[test]
fn the_vertex_shader_writes_the_position_and_the_colour() {
    let c = compile();
    assert_eq!(c.vs_vattr_sizes[..2], [4, 4]);
    let Some(out) = run_vertex("vs", c, &c.vs_vattr_sizes) else { return };
    /* scaled x, y, Zs, 1/Wc, then the FS's inputs in input_slots order */
    assert_eq!(out.len(), 4 + c.fs_input_slots.len(), "{out:x?}");
    for (l, p) in (0..LANES).map(pos).enumerate() {
        assert_eq!(out[0][l], scaled(p[0], X_SCALE), "lane {l} scaled x");
        assert_eq!(out[1][l], scaled(p[1], Y_SCALE), "lane {l} scaled y");
        assert_eq!(f32::from_bits(out[2][l]), p[2] * Z_SCALE + Z_OFFSET, "lane {l} Zs");
        assert_eq!(f32::from_bits(out[3][l]), 1.0, "lane {l} 1/Wc");
        for (i, &(slot, comp)) in c.fs_input_slots.iter().enumerate() {
            assert_eq!(slot, 32, "v_color is VARYING_SLOT_VAR0");
            assert_eq!(f32::from_bits(out[4 + i][l]), colour(l)[comp as usize], "lane {l} v_color.{comp}");
        }
    }
}

#[test]
fn the_coordinate_shader_writes_the_clip_and_scaled_position() {
    let c = compile();
    assert_eq!(c.cs_vattr_sizes[..2], [4, 0], "the CS reads a_pos only");
    let Some(out) = run_vertex("cs", c, &c.cs_vattr_sizes) else { return };
    assert_eq!(out.len(), 6, "{out:x?}");
    for (l, p) in (0..LANES).map(pos).enumerate() {
        for (k, v) in p.iter().enumerate() {
            assert_eq!(f32::from_bits(out[k][l]), *v, "lane {l} clip position {k}");
        }
        assert_eq!(out[4][l], scaled(p[0], X_SCALE), "lane {l} scaled x");
        assert_eq!(out[5][l], scaled(p[1], Y_SCALE), "lane {l} scaled y");
    }
}

#[test]
fn the_fragment_shader_writes_the_interpolated_colour() {
    let c = compile();
    let (_, words, uniforms) = c.stages.iter().find(|s| s.0 == "fs").unwrap();
    let varyings: Vec<[f32; LANES]> = (0..4).map(|k| core::array::from_fn(|l| colour(l)[k])).collect();
    let shape = Shape {
        name: "passthrough+interp_color fs",
        vs: PASSTHROUGH_VS,
        fs: INTERP_FS,
        expect: core::array::from_fn(colour),
        varyings,
        threads: 4,
        ..Shape::bare("passthrough+interp_color fs")
    };
    /* the stream holds no TLB configuration word (RT0's default specifier,
     * the one run71_stream reads); anything else it held would be named */
    let stream: Vec<u32> = uniforms
        .iter()
        .map(|&(ct, d)| match ct {
            QUNIFORM_CONSTANT => d,
            ct => panic!("fs: uniform contents {ct} (data {d:#x}) is not one a pass-through FS reads"),
        })
        .collect();
    run71_stream(words, &stream, &shape).unwrap_or_else(|e| panic!("{e}"));
}
