// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Our V3D 7.1 (Raspberry Pi 5) programs for the gl_test texture, mipmap
//! and cube-map cases of P10 (docs/notes/v3d71-gl-table.md), compiled by
//! `chain71::compile_program71` for the board's VPM, the way
//! tests/v3d71_gltest_shaders.rs does it for the six P9 programs.
//!
//! Selection rule (shared with the Mesa-side item `mesa71tex`): in
//! `driver/tools/pvrlab/gl_test.c`'s dispatch table (`switch (c)`), the
//! first case that samples a 2D texture filled by `glTexImage2D`, the first
//! that calls `glGenerateMipmap`, and the first that samples a
//! `GL_TEXTURE_CUBE_MAP`, with the sources of
//! `driver/tools/pvrlab/gl_test_shaders_es100.h` (the set a V3D build
//! takes). Every distinct program those cases link is pinned. The sources
//! are read from that header's string literals, so the pin cannot drift
//! from it. `tools/v3d/gltest-shaders71-tex/README.md` names the cases,
//! their lines and the state of each program; it is generated here too.
//!
//! Each pinned program is `ours_<name>.json` (chain.rs `v3d_shc_json`'s
//! shape, `gpu.ver` 71, plus `record71`) and `ours_<name>.h` (prefix
//! `ours71_<name>`), exactly the files of `tools/v3d/gltest-shaders71/`. A
//! program the compiler cannot build has no file: its README line is
//! `refused: <error>`, the compiler's own error text. All files are
//! compared byte for byte. Regenerate with
//!
//! ```text
//! V3D_WRITE_OURS_JSON=1 cargo test --no-default-features \
//!   --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gltest_tex_shaders
//! ```
//!
//! Feature set: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::fmt::Write;

use yakogl::shaderc::online::ir::{IoKind, Module};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::{attribute_locations, Prim};
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, V3dProgram71, VsProg71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71, same_instr71};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

static ES100_HEADER: &str = include_str!("../driver/tools/pvrlab/gl_test_shaders_es100.h");
static GL_TEST_C: &str = include_str!("../driver/tools/pvrlab/gl_test.c");

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/v3d/gltest-shaders71-tex");

/// The text of `static const char <sym>[] = "..." "...";` in the ES 1.00
/// set's header (C string literals: `\n`, `\t`, `\"`, `\\` only).
fn header_source(sym: &str) -> String {
    let start =
        ES100_HEADER.find(&format!("static const char {sym}[] =")).unwrap_or_else(|| panic!("{sym} not in the header"));
    let rest = &ES100_HEADER[start..];
    let rest = &rest[..rest.find(";\n").unwrap()];
    let mut out = String::new();
    for line in rest.lines().skip(1) {
        let lit = line
            .trim()
            .strip_prefix('"')
            .and_then(|l| l.strip_suffix('"'))
            .unwrap_or_else(|| panic!("{sym}: {line:?}"));
        let mut it = lit.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            out.push(match it.next().unwrap() {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                e => panic!("{sym}: escape {e}"),
            });
        }
    }
    out
}

/// One program of the three cases.
struct Program {
    /// The file stem under [`DIR`] (`ours_<name>.json` / `.h`) and the C
    /// prefix's tail (`ours71_<name>`).
    name: &'static str,
    /// The header's symbols of the two stages, and where they come from.
    vs_sym: &'static str,
    fs_sym: &'static str,
    sources: &'static str,
    /// The gl_test cases (and `link_program` lines) that draw with it.
    cases: &'static str,
}

fn programs() -> [Program; 3] {
    [
        Program {
            name: "uv_texture",
            vs_sym: "gl_test_uv_vs_src",
            fs_sym: "gl_test_texture_fs_src",
            sources: "tools/shaders/gl_test_es100/uv.vert + texture.frag\n *            (gl_test.c's gl_test_uv_vs_src + gl_test_texture_fs_src)",
            cases: "2 texture (case_texture: the 2D, the mipmap and the glGenerateMipmap draws)",
        },
        Program {
            name: "uv_texture2",
            vs_sym: "gl_test_uv_vs_src",
            fs_sym: "gl_test_texture2_fs_src",
            sources: "tools/shaders/gl_test_es100/uv.vert + texture2.frag\n *            (gl_test.c's gl_test_uv_vs_src + gl_test_texture2_fs_src)",
            cases: "2 texture (case_texture: the two-texture-unit draws)",
        },
        Program {
            name: "cube",
            vs_sym: "gl_test_es300_cube_vs_src",
            fs_sym: "gl_test_es300_cube_fs_src",
            sources: "tools/shaders/es300/cube.vert + cube.frag\n *            (gl_test.c's gl_test_es300_cube_vs_src + gl_test_es300_cube_fs_src)",
            cases: "41 es3cube (case_es3cube)",
        },
    ]
}

fn b(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

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
 * Our V3D 7.1 program {name} of gl_test's texture, mipmap and cube cases (P10),
 *   sources: {sources}
 *   cases:   {cases}
 * compiled by YakoGL's online compiler (src/shaderc/online/v3d/chain71.rs,
 * compile_program71) for the Raspberry Pi 5 (BCM2712 D0, V3D 7.1.10). The
 * same program as ours_{name}.json beside it.
 *
 * Generated by tests/v3d71_gltest_tex_shaders.rs, which also compares it byte
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

/// What the compiler did with one program.
enum Outcome {
    /// The two files' text.
    Pinned { json: String, header: String },
    /// The compiler's own error text.
    Refused(String),
}

/// Compile every program at 7.1 (once, in one scratch region: it is one per
/// process), check every stage, and produce the files. In [`programs`]'s
/// order.
fn compile() -> &'static [Outcome] {
    static ONCE: std::sync::OnceLock<Vec<Outcome>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| programs().iter().map(compile_one).collect())
}

fn compile_one(prog: &Program) -> Outcome {
    let name = prog.name;
    let (vs_src, fs_src) = (header_source(prog.vs_sym), header_source(prog.fs_sym));
    store::with_scratch(|a| {
        let v = match lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(&vs_src).unwrap()) {
            Ok(v) => v,
            Err(e) => return Outcome::Refused(format!("vertex shader: {}", e.msg)),
        };
        let f = match lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&fs_src).unwrap()) {
            Ok(f) => f,
            Err(e) => return Outcome::Refused(format!("fragment shader: {}", e.msg)),
        };
        let v = &a.alloc_filled(1, v.module).unwrap()[0];
        let f = &a.alloc_filled(1, f.module).unwrap()[0];
        let p = match compile_program71(v, f, Prim::Triangles, TARGET71, a) {
            Ok(p) => p,
            Err(e) => return Outcome::Refused(format!("compile_program71: {e:?}")),
        };
        check_stage(&format!("{name} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
        check_stage(&format!("{name} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
        check_stage(&format!("{name} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
        /* what the JSON (v3d_shc's shape) cannot carry is checked absent */
        let fs = &p.fs;
        assert_eq!((fs.flat_shade_flags_hi, fs.noperspective_flags_hi, fs.centroid_flags_hi), (0, 0, 0), "{name}");
        assert_eq!(fs.rt_mask, 1, "{name}: the FS writes RT0 only");
        Outcome::Pinned { json: json71(&p, v, f), header: header71(prog, &p, v, f) }
    })
    .expect("scratch")
}

/// The first line (1-based) of `gl_test.c` at or after `from` that holds
/// `needle`. The README's citations follow the file, so an edit above them
/// moves the numbers (the committed README then differs and is regenerated).
fn cited(from: usize, needle: &str) -> usize {
    GL_TEST_C
        .lines()
        .enumerate()
        .skip(from.saturating_sub(1))
        .find(|(_, l)| l.contains(needle))
        .map(|(i, _)| i + 1)
        .unwrap_or_else(|| panic!("gl_test.c has no line with {needle:?} after line {from}"))
}

fn readme(out: &[Outcome]) -> String {
    let mut o = String::new();
    o.push_str(
        "\
# Our V3D 7.1 programs of gl_test's texture, mipmap and cube cases (P10)

Generated by `tests/v3d71_gltest_tex_shaders.rs` (which compares this file
byte for byte; regeneration: that test's header). Do not edit. The P9
programs are in `../gltest-shaders71/`.

## Selection rule

Shared with the Mesa-side item `mesa71tex`. In `driver/tools/pvrlab/gl_test.c`'s
dispatch table (`switch (c)`, in order) take the first case that samples a 2D
texture filled by `glTexImage2D`, the first that calls `glGenerateMipmap`, and
the first that samples a `GL_TEXTURE_CUBE_MAP`. Sources: the ES 1.00 set,
`driver/tools/pvrlab/gl_test_shaders_es100.h` (generated from
`tools/shaders/gl_test_es100/`; what a V3D build takes). Every distinct
program those cases link is pinned.

## Cases
",
    );
    let tex_fn = cited(1, "static int case_texture(void)");
    let cube_fn = cited(1, "static int case_es3cube(void)");
    let tex8 = cited(tex_fn, "glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8");
    let mip0 = cited(tex_fn, "glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4");
    let mip2 = cited(mip0, "glTexImage2D(GL_TEXTURE_2D, 2, GL_RGBA, 1, 1");
    let genmip = cited(mip2, "glGenerateMipmap(GL_TEXTURE_2D)");
    let prog2 = cited(genmip, "link_program(gl_test_uv_vs_src, gl_test_texture2_fs_src)");
    let cube_tex = cited(cube_fn, "glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f");
    let c2 = cited(1, "case 2: rc = case_texture()");
    let c41 = cited(1, "case 41: rc = case_es3cube()");
    writeln!(
        o,
        "
| shape | case | dispatch | function | what it does |
|---|---|---|---|---|
| 2D texture | 2 `texture` | gl_test.c:{c2} | `case_texture`, gl_test.c:{} | `glTexImage2D` at gl_test.c:{} (an 8x8 RGBA texture, NEAREST, CLAMP_TO_EDGE), sampled by `uv_texture` |
| mipmap | 2 `texture` (the first with `glGenerateMipmap`) | gl_test.c:{c2} | `case_texture` | levels 0..2 by `glTexImage2D` at gl_test.c:{}-{}, then `glGenerateMipmap(GL_TEXTURE_2D)` at gl_test.c:{}, sampled by `uv_texture`; NEAREST_MIPMAP_NEAREST and LINEAR_MIPMAP_LINEAR |
| cube map | 41 `es3cube` | gl_test.c:{c41} | `case_es3cube`, gl_test.c:{} | six 8x8 faces by `glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f)` at gl_test.c:{}, NEAREST, CLAMP_TO_EDGE; sampled by `cube` per face axis, then +Y as a framebuffer attachment |

Case 2 is both the first 2D-texture case and the first `glGenerateMipmap`
case, so the 2D and the mipmap shapes share their programs; its second draw
set (gl_test.c:{prog2}) adds `uv_texture2`. The cube case is ES 3.00: the ES 1.00
set carries its two sources verbatim (`gl_test_es300_cube_*_src` in
`gl_test_shaders_es100.h`), but that set's `GL_TEST_SHADERS_SKIP_REASON` skips
case 41 (\"ES 3.00 source, outside the ES 2.0 compiler\"), so a V3D build of
the ES 1.00 set does not run it. The program is pinned because the rule names
the case; the board needs it for the seamless cube case.
",
        tex_fn,
        tex8,
        mip0,
        mip2,
        genmip,
        cube_fn,
        cube_tex,
    )
    .unwrap();
    o.push_str("\n## Programs\n\n");
    for (prog, r) in programs().iter().zip(out) {
        writeln!(o, "### `{}`\n", prog.name).unwrap();
        writeln!(o, "* vertex stage `{}`, fragment stage `{}` (`gl_test_shaders_es100.h`)", prog.vs_sym, prog.fs_sym)
            .unwrap();
        writeln!(o, "* sources: {}", prog.sources.replace("\n *            ", " ")).unwrap();
        writeln!(o, "* cases: {}", prog.cases).unwrap();
        match r {
            Outcome::Pinned { .. } => writeln!(
                o,
                "* state: pinned: `ours_{0}.h` and `ours_{0}.json` (compiled by `compile_program71`)",
                prog.name
            )
            .unwrap(),
            Outcome::Refused(why) => {
                writeln!(o, "* state: refused: {why}").unwrap();
                writeln!(o, "* no files: the table needs Mesa's offline binary for this program (`mesa71tex`)")
                    .unwrap();
            }
        }
        o.push('\n');
    }
    o
}

#[test]
fn the_committed_files_are_this_compilers_output() {
    let write = std::env::var_os("V3D_WRITE_OURS_JSON").is_some();
    let out = compile();
    let check = |path: String, text: &str| {
        if write {
            std::fs::write(&path, text).unwrap();
            println!("wrote {path} ({} bytes); run again without V3D_WRITE_OURS_JSON to compare", text.len());
            return;
        }
        let committed =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}; regenerate (this test's header)"));
        assert!(committed == text, "{path} is not this compiler's output; regenerate it (see this test's header)");
    };
    for (prog, r) in programs().iter().zip(out) {
        match r {
            Outcome::Pinned { json, header } => {
                check(format!("{DIR}/ours_{}.json", prog.name), json);
                check(format!("{DIR}/ours_{}.h", prog.name), header);
            }
            Outcome::Refused(_) => {
                for ext in ["json", "h"] {
                    let p = format!("{DIR}/ours_{}.{ext}", prog.name);
                    assert!(!std::path::Path::new(&p).exists() || write, "{p} exists for a refused program");
                }
            }
        }
    }
    check(format!("{DIR}/README.md"), &readme(out));
}

#[test]
fn the_selected_cases_are_the_first_of_their_kind() {
    /* case 2 is the table's third entry, and no case of the table ahead of
     * 41 names a cube map target in its function */
    let sw = GL_TEST_C.find("\tcase 0: rc = case_triangle(").unwrap();
    let to41 = GL_TEST_C[sw..].find("\tcase 41: rc = case_es3cube(").unwrap();
    let table = &GL_TEST_C[sw..sw + to41];
    let at = |c: u32| table.find(&format!("\tcase {c}: rc")).unwrap_or_else(|| panic!("case {c}"));
    assert!(at(0) < at(1) && at(1) < at(2) && at(2) < at(3), "case 2 is the table's third entry");
    /* (a case with a multi-line body, `case 16:`, has no `rc =` on its line:
     * its function is found by the `rc = ` that follows) */
    for part in table.split("rc = ").skip(1) {
        let f = part.split('(').next().unwrap();
        let def = GL_TEST_C.find(&format!("static int {f}(")).unwrap_or_else(|| panic!("{f}"));
        let end = GL_TEST_C[def..].find("\n}\n").unwrap();
        assert!(!GL_TEST_C[def..def + end].contains("GL_TEXTURE_CUBE_MAP"), "{f} samples a cube map before case 41");
    }
}
