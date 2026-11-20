// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Our V3D 7.1 (Raspberry Pi 5) programs for the six gl_test cases of P9
//! (docs/notes/v3d71-gl-table.md; the Pi 4's list in
//! docs/notes/pi4-v3d-bringup.md, "gl_test on the V3D": 5, 58, 59, 66, 67,
//! 102 on one context), compiled by `chain71::compile_program71` for the
//! board's VPM. The six cases draw with two programs:
//!
//! | program | sources | cases | at 7.1 |
//! |---|---|---|---|
//! | `color_uv` | `tools/shaders/gl_test_es100/color_uv.vert` + `color_uv.frag` (gl_test's `gl_test_vs_src` + `gl_test_fs_src`, `driver/tools/pvrlab/gl_test_shaders_es100.h`) | 5, 58, 59, 67, 102 (`link_program` in each; 66 links none) | **compiled** |
//! | `clear` | `src/v3d/backend.rs` `CLEAR_VS` + `CLEAR_FS` (the float clear quad the table draws for a clear the TLB cannot do: scissored, stencil-masked, colour-masked; `init_clear_program`) | the clear quad of 5, 58, 59, 66, 67 and 102 | **compiled** |
//!
//! Case by case (`driver/tools/pvrlab/gl_test.c`):
//!
//! * 5 `case_scissor`: `color_uv` (compiled), `clear` for its scissored
//!   clear (compiled);
//! * 58 `case_clearscis`: `color_uv` (compiled), `clear` (compiled);
//! * 59 `case_clearmask`: `color_uv` (compiled), `clear` for its masked
//!   clears (compiled);
//! * 66 `case_clearvp`: no program of its own, `clear` for its scissored
//!   clear (compiled);
//! * 67 `case_fbods`: `color_uv` (compiled), `clear` for its scissored
//!   stencil clears (compiled);
//! * 102 `case_maskclear`: `color_uv` (compiled), `clear` for the clear
//!   under a partial colour mask (compiled).
//!
//! No program of the six cases is refused, so cases71 takes both from here
//! and needs Mesa's offline binaries for none of them. The integer clear
//! program (`CLEAR_FS_INT`) and the several-target one (`clear_mrt`) are not
//! drawn by these cases (each clears one normalised colour buffer) and are
//! not built here.
//!
//! Each program is committed under `tools/v3d/gltest-shaders71/` as
//!
//! * `ours_<program>.json`: chain.rs `v3d_shc_json`'s shape (v3d_shc's JSON
//!   key for key) with `gpu.ver` 71, plus `record71`: the values the BCM2712
//!   D0 shader record (`GL Shader State Record Draw Index`) takes from the
//!   three programs, by Mesa's field names, as Mesa 26.2.1
//!   v3dx_draw.c:477-547 derives them (tests/v3d71_tri_shaders.rs's shape);
//! * `ours_<program>.h`: the same program as a C header (its comment
//!   describes the shape; prefix `ours71_<program>`).
//!
//! Both must be what the compiler produces now, byte for byte. Regenerate
//! with
//!
//! ```text
//! V3D_WRITE_OURS_JSON=1 cargo test --no-default-features \
//!   --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gltest_shaders
//! ```
//!
//! then run it again without the variable: the committed bytes are compiled
//! in (`include_str!`), so the writing run does not compare. The words are
//! also checked as v3d71_tri_shaders.rs checks its pair: each decodes with
//! codec71 to its instruction and re-encodes to itself, verify71 and
//! `registers_fit_threads71` accept every stage, and the 7.1 simulator
//! (`sim::run_gen(Gen::V71, ..)`, sim71) runs each fragment shader (through
//! `tests/v3d71_common`'s runner) and each vertex and coordinate shader.
//! sim71 does not yet run a vertex or coordinate shader to its end (the
//! thread end's STVPMV, see `run_vertex`): until it does, those print a GAP
//! and check the words only.
//!
//! A program the compiler cannot build would be pinned here as a named
//! refusal (the test asserting the exact `V3d71Error`, this table naming
//! it); there is none today.
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
    QUNIFORM_CONSTANT, QUNIFORM_UNIFORM, QUNIFORM_VIEWPORT_X_SCALE, QUNIFORM_VIEWPORT_Y_SCALE,
    QUNIFORM_VIEWPORT_Z_OFFSET, QUNIFORM_VIEWPORT_Z_SCALE,
};
use yakogl::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};
use yakogl::v3d::backend::{CLEAR_FS, CLEAR_VS};

static COLOR_UV_VS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.vert");
static COLOR_UV_FS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.frag");

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/v3d/gltest-shaders71");

/// One program of the six cases.
struct Program {
    /// The file stem under [`DIR`] (`ours_<name>.json` / `.h`) and the C
    /// prefix's tail (`ours71_<name>`).
    name: &'static str,
    /// Where the sources come from, for the header comment.
    sources: &'static str,
    /// The gl_test cases that draw with it.
    cases: &'static str,
    vs: &'static str,
    fs: &'static str,
    committed_json: &'static str,
    committed_header: &'static str,
}

/// A NUL-terminated source of the back end as `&str`.
fn c_source(s: &'static [u8]) -> &'static str {
    let s = s.strip_suffix(b"\0").expect("a NUL-terminated source");
    core::str::from_utf8(s).expect("UTF-8 source")
}

fn programs() -> [Program; 2] {
    [
        Program {
            name: "color_uv",
            sources: "tools/shaders/gl_test_es100/color_uv.vert + color_uv.frag\n *            (gl_test.c's gl_test_vs_src + gl_test_fs_src)",
            cases: "5, 58, 59, 67, 102 (their draws)",
            vs: COLOR_UV_VS,
            fs: COLOR_UV_FS,
            committed_json: include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.json"),
            committed_header: include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h"),
        },
        Program {
            name: "clear",
            sources: "src/v3d/backend.rs CLEAR_VS + CLEAR_FS (the float clear quad)",
            cases: "5, 58, 59, 66, 67, 102 (a clear the TLB cannot do:\n *            scissored, stencil-masked, colour-masked)",
            vs: c_source(CLEAR_VS),
            fs: c_source(CLEAR_FS),
            committed_json: include_str!("../tools/v3d/gltest-shaders71/ours_clear.json"),
            committed_header: include_str!("../tools/v3d/gltest-shaders71/ours_clear.h"),
        },
    ]
}

/// The viewport the simulator runs the vertex and coordinate shaders with
/// (the board's 64x64 target: half the size in 1/64 pixel, y down; depth
/// range 0..1), so the scaled position is an exact integer.
const X_SCALE: f32 = 2048.0;
const Y_SCALE: f32 = -2048.0;
const Z_SCALE: f32 = 0.5;
const Z_OFFSET: f32 = 0.5;

/// The clear quad's colour in the simulator (exact in f16).
const CLEAR_COLOUR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

fn b(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

/// One stage: its name, words and uniform stream `(contents, data)`.
type StageCode = (&'static str, Vec<u64>, Vec<(u32, u32)>);

/// One program's two files' text and what the simulator runs.
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
/// v3dx_draw.c:477-547's order, by Mesa's field names (v3d71_tri_shaders.rs's
/// `record71`).
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

/// The header's comment: `prog`'s sources and cases, then the shape.
fn header_comment(prog: &Program) -> String {
    format!(
        "\
/*
 * Our V3D 7.1 program {name} of gl_test's six P9 cases,
 *   sources: {sources}
 *   cases:   {cases}
 * compiled by YakoGL's online compiler (src/shaderc/online/v3d/chain71.rs,
 * compile_program71) for the Raspberry Pi 5 (BCM2712 D0, V3D 7.1.10). The
 * same program as ours_{name}.json beside it.
 *
 * Generated by tests/v3d71_gltest_shaders.rs, which also compares it byte
 * for byte with the compiler's output; regenerate with V3D_WRITE_OURS_JSON=1
 * (that test's header). Do not edit.
 *
 * Shape (self-contained: <stdint.h> only, no driver header; <P> is the
 * prefix, ours71_{name} here):
 *   <P>_{{fs,vs,cs}}_code[]       const uint64_t, the QPU words, 8-byte aligned
 *   <P>_{{FS,VS,CS}}_NUM_WORDS    their counts
 *   <P>_{{fs,vs,cs}}_uniforms[][2] const uint32_t {{ contents, data }}: the
 *                               uniform stream (Mesa's quniform_contents;
 *                               driver/src/v3d/v3d_frame.h V3D_QUNIFORM_*),
 *                               absent when <P>_<S>_NUM_UNIFORMS is 0
 *   <P>_<S>_<FIELD>             the prog_data the frame code reads
 *                               (struct v3d_fs_prog / v3d_vs_prog fields)
 *   <P>_<S>_VATTR_SIZES         VS and CS: the 16 vattr_sizes, an initializer
 *   <P>_ATTR_<NAME>_RECORD      each read attribute's record (its location in
 *                               the comment); <P>_NUM_ATTRS the records
 *   <P>_UNIFORM_<NAME>_WORD     each GL uniform's first dword in its stage's
 *                               block (the data of a QUNIFORM_UNIFORM entry)
 *   <P>_REC_<FIELD>             the BCM2712 D0 shader record's
 *                               (GL Shader State Record Draw Index) fields
 *                               that come from the programs, Mesa's names,
 *                               as Mesa 26.2.1 v3dx_draw.c:477-547 derives
 *                               them. enable_clipping, the point-size bit,
 *                               the addresses and the VPM configuration's
 *                               As and Ve are the frame code's.
 */
",
        cases = prog.cases,
        sources = prog.sources,
        name = prog.name,
    )
}

/// The header: see [`header_comment`].
fn header71(prog: &Program, p: &V3dProgram71<'_>, vs: &Module<'_>, fs: &Module<'_>) -> String {
    let n = format!("ours71_{}", prog.name);
    let u = n.to_uppercase();
    let mut o = String::new();
    o.push_str("/* SPDX-License-Identifier: Apache-2.0 */\n/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */\n");
    o.push_str(&header_comment(prog));
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
    for (stage, m) in [("vert", vs), ("frag", fs)] {
        for x in m.uniforms {
            writeln!(
                o,
                "#define {u}_UNIFORM_{}_WORD {} /* {stage}, {} dwords */",
                x.name.to_uppercase(),
                x.dword_off,
                x.dwords
            )
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

/// Compile every program at 7.1 (once, in one scratch region: it is one per
/// process), check every stage, and produce the files. In [`programs`]'s
/// order.
fn compile() -> &'static [Compiled] {
    static ONCE: std::sync::OnceLock<Vec<Compiled>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| programs().iter().map(compile_one).collect())
}

fn compile_one(prog: &Program) -> Compiled {
    let name = prog.name;
    store::with_scratch(|a| {
        let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(prog.vs).unwrap())
            .unwrap_or_else(|e| panic!("{name} vertex shader: {}", e.msg));
        let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(prog.fs).unwrap())
            .unwrap_or_else(|e| panic!("{name} fragment shader: {}", e.msg));
        let v = &a.alloc_filled(1, v.module).unwrap()[0];
        let f = &a.alloc_filled(1, f.module).unwrap()[0];
        let p = compile_program71(v, f, Prim::Triangles, TARGET71, a)
            .unwrap_or_else(|e| panic!("{name} at 7.1: {e:?} (pin it as a named refusal: this test's header)"));
        check_stage(&format!("{name} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage(&format!("{name} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage(&format!("{name} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
        /* what the JSON (v3d_shc's shape) cannot carry is checked absent */
        let fs = &p.fs;
        assert_eq!((fs.flat_shade_flags_hi, fs.noperspective_flags_hi, fs.centroid_flags_hi), (0, 0, 0), "{name}");
        assert_eq!(fs.rt_mask, 1, "{name}: the FS writes RT0 only");
        let uni = |c: &Code71<'_>| c.uniforms.iter().map(|u| (u.contents, u.data)).collect::<Vec<_>>();
        Compiled {
            json: json71(&p, v, f),
            header: header71(prog, &p, v, f),
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
    /* The committed bytes are compiled into the binary (include_str!), so
     * the test cache's key follows them; the regeneration writes the files
     * and asks for a rebuild, which then compares. */
    for (prog, c) in programs().iter().zip(compile()) {
        for (ext, text, committed) in [("json", &c.json, prog.committed_json), ("h", &c.header, prog.committed_header)]
        {
            let path = format!("{DIR}/ours_{}.{ext}", prog.name);
            if std::env::var_os("V3D_WRITE_OURS_JSON").is_some() {
                std::fs::write(&path, text).unwrap();
                println!("wrote {path} ({} bytes); run again without V3D_WRITE_OURS_JSON to compare", text.len());
                continue;
            }
            assert!(committed == *text, "{path} is not this compiler's output; regenerate it (see this test's header)");
        }
    }
}

/// The program of `name`.
fn compiled(name: &str) -> &'static Compiled {
    let i = programs().iter().position(|p| p.name == name).unwrap();
    &compile()[i]
}

/// A stream with every contents code this program reads resolved; a GL
/// uniform reads the clear colour (the only GL uniform of these programs is
/// the clear quad's `u_color`, dwords 0..3).
fn resolve(stage: &str, uniforms: &[(u32, u32)]) -> Vec<u32> {
    uniforms
        .iter()
        .map(|&(c, d)| match c {
            QUNIFORM_CONSTANT => d,
            QUNIFORM_UNIFORM if d < 4 => CLEAR_COLOUR[d as usize].to_bits(),
            QUNIFORM_VIEWPORT_X_SCALE => X_SCALE.to_bits(),
            QUNIFORM_VIEWPORT_Y_SCALE => Y_SCALE.to_bits(),
            QUNIFORM_VIEWPORT_Z_OFFSET => Z_OFFSET.to_bits(),
            QUNIFORM_VIEWPORT_Z_SCALE => Z_SCALE.to_bits(),
            c => panic!("{stage}: uniform contents {c} (data {d:#x}) is not one these programs read"),
        })
        .collect()
}

/// Exact in f32 and f16: lane l's position, colour and uv.
fn pos(l: usize) -> [f32; 4] {
    [(l as f32 - 8.0) / 16.0, (8.0 - l as f32) / 32.0, (l % 4) as f32 / 8.0, 1.0]
}
fn colour(l: usize) -> [f32; 4] {
    [l as f32 / 16.0, (15 - l) as f32 / 16.0, ((l * 5) % 16) as f32 / 16.0, 1.0]
}
fn uv(l: usize) -> [f32; 4] {
    [((l * 3) % 8) as f32 / 8.0, ((l * 5 + 1) % 8) as f32 / 8.0, 0.0, 1.0]
}

/// The VS and CS input rows: record k is the k-th of `attrs`, each as many
/// rows as its vattr_sizes entry.
fn rows(sizes: &[u8; 16], attrs: &[fn(usize) -> [f32; 4]]) -> Vec<[u32; LANES]> {
    assert!(sizes[attrs.len()..].iter().all(|&s| s == 0), "{sizes:?}: {} records", attrs.len());
    let mut r = Vec::new();
    for (rec, f) in attrs.iter().enumerate() {
        for c in 0..sizes[rec] as usize {
            r.push(core::array::from_fn(|l| f(l)[c].to_bits()));
        }
    }
    r
}

/// Run the VS or CS in sim71 and return its VPM output rows, or `None` when
/// sim71 stops at the one place it is known not to model yet: a thread end
/// of `stvpmv ; thrsw` (v3d71_tri_shaders.rs `run_vertex`, where the reason
/// is written out). Only that refusal, at that instruction, is let through,
/// printed as a GAP.
fn run_vertex(
    what: &str,
    stage: &str,
    c: &Compiled,
    sizes: &[u8; 16],
    attrs: &[fn(usize) -> [f32; 4]],
) -> Option<Vec<[u32; LANES]>> {
    let (_, words, uniforms) = c.stages.iter().find(|s| s.0 == stage).unwrap();
    let env = Env { threads: 4, ..Env::default() };
    let inputs = Inputs::Vertex(VsInputs { vpm: rows(sizes, attrs) });
    let o = match sim::run_gen(Gen::V71, words, &resolve(stage, uniforms), &inputs, &env) {
        Ok(o) => o,
        Err(e) => {
            let end = words.len() - 3;
            let known = format!("instruction {end}: an ALU register-file write at the thread end");
            let Ok(Instr71::Alu(alu)) = decode71(words[end]) else {
                panic!("{what} {stage}: word {end} is not an ALU word")
            };
            assert!(
                e == known && alu.sig.thrsw && alu.add.op == AddOp71::Stvpmv && alu.mul.op == MulOp71::Nop,
                "{what} {stage}: sim71: {e}"
            );
            println!("GAP {what} {stage}: sim71 stops at the thread end's STVPMV ({e}); the VPM rows are not checked");
            return None;
        }
    };
    assert!(o.tlb.is_empty(), "{what} {stage}: a TLB write");
    Some(
        o.vpm_out
            .iter()
            .enumerate()
            .map(|(r, x)| x.unwrap_or_else(|| panic!("{what} {stage}: VPM row {r} never written")))
            .collect(),
    )
}

/// The scaled coordinate at 7.1: round to nearest even
/// (v3d_nir_lower_io.c:529-543).
fn scaled(v: f32, s: f32) -> u32 {
    (v * s).round_ties_even() as i32 as u32
}

/// The varying values the colour/uv VS writes for FS input (slot, comp):
/// v_color is VARYING_SLOT_VAR0 (32), v_uv VAR1 (33).
fn color_uv_varying(slot: u8, comp: u8, l: usize) -> f32 {
    match slot {
        32 => colour(l)[comp as usize],
        33 => uv(l)[comp as usize],
        s => panic!("slot {s} is not v_color or v_uv"),
    }
}

/// The VS's rows: scaled x, y, Zs, 1/Wc, then `varying(slot, comp, lane)`
/// for each FS input in input_slots order.
fn check_vs_rows(what: &str, out: &[[u32; LANES]], slots: &[(u8, u8)], varying: impl Fn(u8, u8, usize) -> f32) {
    assert_eq!(out.len(), 4 + slots.len(), "{what}: {out:x?}");
    for (l, p) in (0..LANES).map(pos).enumerate() {
        assert_eq!(out[0][l], scaled(p[0], X_SCALE), "{what}: lane {l} scaled x");
        assert_eq!(out[1][l], scaled(p[1], Y_SCALE), "{what}: lane {l} scaled y");
        assert_eq!(f32::from_bits(out[2][l]), p[2] * Z_SCALE + Z_OFFSET, "{what}: lane {l} Zs");
        assert_eq!(f32::from_bits(out[3][l]), 1.0, "{what}: lane {l} 1/Wc");
        for (i, &(slot, comp)) in slots.iter().enumerate() {
            assert_eq!(f32::from_bits(out[4 + i][l]), varying(slot, comp, l), "{what}: lane {l} input {slot}.{comp}");
        }
    }
}

/// The CS's rows: the clip position, then the scaled x and y.
fn check_cs_rows(what: &str, out: &[[u32; LANES]]) {
    assert_eq!(out.len(), 6, "{what}: {out:x?}");
    for (l, p) in (0..LANES).map(pos).enumerate() {
        for (k, v) in p.iter().enumerate() {
            assert_eq!(f32::from_bits(out[k][l]), *v, "{what}: lane {l} clip position {k}");
        }
        assert_eq!(out[4][l], scaled(p[0], X_SCALE), "{what}: lane {l} scaled x");
        assert_eq!(out[5][l], scaled(p[1], Y_SCALE), "{what}: lane {l} scaled y");
    }
}

#[test]
fn color_uv_vertex_shader_writes_the_position_colour_and_uv() {
    let c = compiled("color_uv");
    assert_eq!(c.vs_vattr_sizes[..3], [4, 4, 2], "a_pos, a_color, a_uv");
    let slots: Vec<(u8, u8)> = (0..4).map(|k| (32, k)).chain((0..2).map(|k| (33, k))).collect();
    assert_eq!(c.fs_input_slots, slots, "v_color then v_uv");
    let Some(out) = run_vertex("color_uv", "vs", c, &c.vs_vattr_sizes, &[pos, colour, uv]) else { return };
    check_vs_rows("color_uv vs", &out, &c.fs_input_slots, color_uv_varying);
}

#[test]
fn color_uv_coordinate_shader_writes_the_clip_and_scaled_position() {
    let c = compiled("color_uv");
    assert_eq!(c.cs_vattr_sizes[..3], [4, 0, 0], "the CS reads a_pos only");
    let Some(out) = run_vertex("color_uv", "cs", c, &c.cs_vattr_sizes, &[pos, colour, uv]) else { return };
    check_cs_rows("color_uv cs", &out);
}

#[test]
fn color_uv_fragment_shader_writes_the_colour_times_uv() {
    let c = compiled("color_uv");
    let (_, words, uniforms) = c.stages.iter().find(|s| s.0 == "fs").unwrap();
    let varyings: Vec<[f32; LANES]> = c
        .fs_input_slots
        .iter()
        .map(|&(slot, comp)| core::array::from_fn(|l| color_uv_varying(slot, comp, l)))
        .collect();
    let expect = core::array::from_fn(|l| {
        let (c, t) = (colour(l), uv(l));
        [c[0] * t[0], c[1] * t[1], c[2], c[3]]
    });
    let shape = Shape {
        name: "color_uv fs",
        vs: COLOR_UV_VS,
        fs: COLOR_UV_FS,
        expect,
        varyings,
        threads: 4,
        ..Shape::bare("color_uv fs")
    };
    run71_stream(words, &resolve("fs", uniforms), &shape).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn clear_vertex_shader_writes_the_position() {
    let c = compiled("clear");
    assert_eq!(c.vs_vattr_sizes[..1], [4], "a_pos");
    assert!(c.fs_input_slots.is_empty(), "the clear FS reads no varying");
    let Some(out) = run_vertex("clear", "vs", c, &c.vs_vattr_sizes, &[pos]) else { return };
    check_vs_rows("clear vs", &out, &[], |_, _, _| unreachable!());
}

#[test]
fn clear_coordinate_shader_writes_the_clip_and_scaled_position() {
    let c = compiled("clear");
    assert_eq!(c.cs_vattr_sizes[..1], [4], "a_pos");
    let Some(out) = run_vertex("clear", "cs", c, &c.cs_vattr_sizes, &[pos]) else { return };
    check_cs_rows("clear cs", &out);
}

#[test]
fn clear_fragment_shader_writes_the_clear_colour() {
    let c = compiled("clear");
    let (_, words, uniforms) = c.stages.iter().find(|s| s.0 == "fs").unwrap();
    /* u_color is the stage's dwords 0..3 (CLEAR_FS's one uniform), read
     * once each */
    let mut read: Vec<u32> = uniforms.iter().filter(|u| u.0 == QUNIFORM_UNIFORM).map(|u| u.1).collect();
    read.sort_unstable();
    assert_eq!(read, [0, 1, 2, 3], "{uniforms:?}");
    let p = programs();
    let shape = Shape {
        name: "clear fs",
        vs: p[1].vs,
        fs: p[1].fs,
        uniforms: CLEAR_COLOUR.to_vec(),
        expect: [CLEAR_COLOUR; LANES],
        threads: 4,
        ..Shape::bare("clear fs")
    };
    run71_stream(words, &resolve("fs", uniforms), &shape).unwrap_or_else(|e| panic!("{e}"));
}
