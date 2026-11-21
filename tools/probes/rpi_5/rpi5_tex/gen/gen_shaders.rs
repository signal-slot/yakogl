// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! rpi5_tex's two programs through our V3D 7.1 online compiler
//! (`chain71::compile_program71`, Prim::Triangles, TARGET71), written as one
//! C header for the probe. Built and run by gen_shaders.sh as a throwaway
//! crate that depends on yakogl; not part of the workspace.
//!
//!   gen_shaders <shaders dir> <out header>
//!
//! A program the compiler refuses ends the run with the refusal's name: the
//! probe then has no header to build with, never a stale one.

use std::fmt::Write;

use yakogl::shaderc::online::ir::{IoKind, Module};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::{attribute_locations, Prim};
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, V3dProgram71, VsProg71, TARGET71};
use yakogl::shaderc::online::{lower, store};

/// (C name, vertex source file, fragment source file, attribute names in
/// record order)
const PROGRAMS: [(&str, &str, &str, [&str; 2]); 2] =
    [("tex2d", "tex2d.vert", "tex2d.frag", ["a_pos", "a_uv"]), ("cube", "cube.vert", "cube.frag", ["a_pos", "a_dir"])];

fn code(o: &mut String, p: &str, stage: &str, c: &Code71<'_>) {
    writeln!(o, "static const uint64_t rpi5_tex_{p}_{stage}_code[{}] = {{", c.words.len()).unwrap();
    for w in c.words {
        writeln!(o, "\t0x{w:016x}ull,").unwrap();
    }
    writeln!(o, "}};").unwrap();
    writeln!(o, "static const struct v3d_uniform rpi5_tex_{p}_{stage}_uniforms[{}] = {{", c.uniforms.len().max(1))
        .unwrap();
    for u in c.uniforms {
        writeln!(o, "\t{{ {}u, 0x{:08x}u }},", u.contents, u.data).unwrap();
    }
    if c.uniforms.is_empty() {
        writeln!(o, "\t{{ 0u, 0u }}, /* none: num_uniforms is 0 */").unwrap();
    }
    writeln!(o, "}};").unwrap();
}

fn vs_prog(o: &mut String, p: &str, stage: &str, v: &VsProg71<'_>) {
    assert!(!v.writes_psiz, "{p} {stage}: writes gl_PointSize");
    let sizes: Vec<String> = v.vattr_sizes.iter().map(|s| s.to_string()).collect();
    writeln!(
        o,
        "static const struct v3d_vs_prog rpi5_tex_{p}_{stage} = {{\n\t.threads = {},\n\t.single_seg = {},\n\
         \t.vpm_input_size = {},\n\t.vpm_output_size = {},\n\t.separate_segments = {},\n\t.vcm_cache_size = {},\n\
         \t.uses_vid = {},\n\t.uses_iid = {},\n\t.uses_biid = {},\n\t.vattr_sizes = {{ {} }},\n\
         \t.uniforms = rpi5_tex_{p}_{stage}_uniforms,\n\t.num_uniforms = {},\n}};",
        v.threads,
        v.single_seg,
        v.vpm_input_size,
        v.vpm_output_size,
        v.separate_segments,
        v.vcm_cache_size,
        v.uses_vid,
        v.uses_iid,
        v.uses_biid,
        sizes.join(", "),
        v.code.uniforms.len()
    )
    .unwrap();
}

fn program(o: &mut String, name: &str, attrs: &[&str; 2], p: &V3dProgram71<'_>, vs: &Module<'_>) {
    let u = name.to_uppercase();
    writeln!(o, "\n/* ---- {name} ---- */").unwrap();
    code(o, name, "fs", &p.fs.code);
    code(o, name, "vs", &p.vs.code);
    code(o, name, "cs", &p.cs.code);
    let f = &p.fs;
    assert_eq!((f.flat_shade_flags_hi, f.noperspective_flags_hi, f.centroid_flags_hi), (0, 0, 0), "{name}");
    assert_eq!(f.rt_mask, 1, "{name}: the FS writes RT0 only");
    writeln!(
        o,
        "static const struct v3d_fs_prog rpi5_tex_{name}_fs = {{\n\t.threads = {},\n\t.single_seg = {},\n\
         \t.num_inputs = {},\n\t.flat_shade_flags = 0x{:x},\n\t.noperspective_flags = 0x{:x},\n\
         \t.centroid_flags = 0x{:x},\n\t.writes_z = {},\n\t.disable_ez = {},\n\t.uses_center_w = {},\n\
         \t.lock_scoreboard_on_first_thrsw = {},\n\t.uses_implicit_point_line_varyings = {},\n\
         \t.uses_pid = {},\n\t.uniforms = rpi5_tex_{name}_fs_uniforms,\n\t.num_uniforms = {},\n}};",
        f.threads,
        f.single_seg,
        f.num_inputs,
        f.flat_shade_flags,
        f.noperspective_flags,
        f.centroid_flags,
        f.writes_z,
        f.disable_ez,
        f.uses_center_w,
        f.lock_scoreboard_on_first_thrsw,
        f.uses_implicit_point_line_varyings,
        f.uses_pid,
        f.code.uniforms.len()
    )
    .unwrap();
    vs_prog(o, name, "vs", &p.vs);
    vs_prog(o, name, "cs", &p.cs);
    /* the attribute records in the order the probe gives them: each name's
     * one record (no matrix attributes here) must be record i */
    let decls: Vec<_> = vs.inputs.iter().filter(|d| d.kind == IoKind::Attribute).collect();
    assert_eq!(decls.len(), attrs.len(), "{name}: attributes");
    for (i, want) in attrs.iter().enumerate() {
        let d = decls.iter().find(|d| d.name == *want).unwrap_or_else(|| panic!("{name}: no attribute {want}"));
        assert_eq!(attribute_locations(d.gl_type), 1, "{name} {want}");
        let rec = p.attributes.iter().find(|e| e.0 == d.location).and_then(|e| e.1);
        assert_eq!(rec, Some(i as u8), "{name} {want}: record {rec:?}, the probe gives it record {i}");
    }
    writeln!(o, "#define RPI5_TEX_{u}_ATTRS {}u /* records: {} */", attrs.len(), attrs.join(", ")).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: gen_shaders <shaders dir> <out header>");
    let dir = &args[1];
    let mut o = String::new();
    for line in [
        "/* SPDX-License-Identifier: Apache-2.0 */",
        "/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */",
        "/*",
        " * GENERATED by tools/probes/rpi_5/rpi5_tex/gen/gen_shaders.sh: do not edit.",
        " * rpi5_tex's programs (shaders/tex2d.*, shaders/cube.*) compiled by our",
        " * V3D 7.1 online compiler (src/shaderc/online/v3d/chain71.rs",
        " * compile_program71, triangles, VPM 65536) for the Raspberry Pi 5's",
        " * V3D 7.1.10. Uniform contents are Mesa's quniform_contents (40/41: TMU",
        " * config P0/P1, data = sampler binding << 24 | the packed bits; the",
        " * probe ORs in its record addresses). Code addresses are 0: the probe",
        " * places the code. One translation unit includes this header.",
        " */",
        "#ifndef RPI5_TEX_SHADERS_OURS_H",
        "#define RPI5_TEX_SHADERS_OURS_H",
        "",
        "#include <stdint.h>",
        "",
        "#include \"v3d_frame.h\"",
    ] {
        o.push_str(line);
        o.push('\n');
    }
    for (name, vsf, fsf, attrs) in PROGRAMS {
        let vs_src = std::fs::read_to_string(format!("{dir}/{vsf}")).unwrap_or_else(|e| panic!("{vsf}: {e}"));
        let fs_src = std::fs::read_to_string(format!("{dir}/{fsf}")).unwrap_or_else(|e| panic!("{fsf}: {e}"));
        let ok = store::with_scratch(|a| {
            let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(&vs_src).unwrap())
                .unwrap_or_else(|e| panic!("{name} vertex shader: {}", e.msg));
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&fs_src).unwrap())
                .unwrap_or_else(|e| panic!("{name} fragment shader: {}", e.msg));
            let v = &a.alloc_filled(1, v.module).unwrap()[0];
            let f = &a.alloc_filled(1, f.module).unwrap()[0];
            match compile_program71(v, f, Prim::Triangles, TARGET71, a) {
                Ok(p) => {
                    program(&mut o, name, &attrs, &p, v);
                    true
                }
                Err(e) => {
                    eprintln!("gen_shaders: {name}: our 7.1 compiler refused it: {e:?}");
                    false
                }
            }
        })
        .expect("scratch");
        if !ok {
            std::process::exit(1);
        }
    }
    o.push_str("\n#endif\n");
    std::fs::write(&args[2], o).unwrap_or_else(|e| panic!("{}: {e}", args[2]));
}
