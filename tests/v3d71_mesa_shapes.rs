// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa's V3D 7.1 programs for the four P8 shapes (docs/notes/pre-study-rpi5.md
//! section 8: pass-through, uniform, `if`, texture; `tests/v3d71_common`),
//! the reference side of P8's host comparison (`tests/v3d71_ref`).
//!
//! With `V3D_SHC` or `MESA_V3D_TREE` (tools/v3d/README.md), each shape is
//! compiled by `v3d_shc --device 71` and:
//!
//! * every FS, VS and CS word decodes at 7.1 and re-encodes to itself
//!   (codec71), and Mesa's FS runs in sim71 with the shape's inputs and gives
//!   the shape's colour in every lane (`v3d71_common::run71_stream` with
//!   Mesa's own uniform stream): the shapes' expectations and Mesa's code
//!   vouch for each other;
//! * the fs/vs/cs instruction counts are printed and pinned ([`PINNED`]).
//!
//! First the binary is vouched for: color_uv (tools/shaders) must compile to
//! the counts `tools/v3d/qpu_counts_71.tsv` holds for it, the table the H2
//! build wrote. A binary that is not that build fails here, before any shape.
//!
//! Without either variable, with one naming no binary, with a v3d_shc built
//! before `--device` existed, or without python3 or glslangValidator, the
//! shapes test prints `GAP ...` naming what is missing and checks nothing: a
//! gap, not a pass. The JSON-reader test needs no tool and always runs.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_mesa_shapes
//! -- --nocapture`.

// The V3D compiler and sim exist only in a gpu-v3d build.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

mod v3d71_common;
mod v3d71_ref;

use v3d71_common::{run71_stream, shapes, Shape};
use v3d71_ref::{compile71, find_v3d_shc, Gap, Mesa71};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};

/// Mesa 26.2.1's 7.1 instruction counts per shape (fs, vs, cs), measured
/// with the build tools/v3d/README.md describes (`v3d_shc --device 71`,
/// rev 7, compat_rev 0), 2026-11-15. All four FS run with 4 threads.
const PINNED: [(&str, [usize; 3]); 4] =
    [("passthrough", [9, 23, 15]), ("uniform", [6, 15, 15]), ("if", [15, 16, 15]), ("texture", [9, 19, 15])];

/// The reference binary, or the gap printed by the caller.
fn shc() -> Result<std::path::PathBuf, Gap> {
    find_v3d_shc()
}

fn gap(test: &str, g: &Gap) {
    println!("GAP     v3d71_mesa_shapes {test}: {}", g.0);
}

fn counts(m: &Mesa71) -> [usize; 3] {
    [m.fs.code.len(), m.vs.code.len(), m.cs.code.len()]
}

/// The counts tools/v3d/qpu_counts_71.tsv holds for color_uv.
fn color_uv_row() -> [usize; 3] {
    let tsv = include_str!("../tools/v3d/qpu_counts_71.tsv");
    let row =
        tsv.lines().find(|l| l.starts_with("color_uv.vert\tcolor_uv.frag\t")).expect("qpu_counts_71.tsv has color_uv");
    let n: Vec<usize> = row.split('\t').skip(2).map(|c| c.parse().expect("a count")).collect();
    [n[0], n[1], n[2]]
}

/// color_uv through the same steps as a shape must give the H2 table's
/// counts: the binary found is the reference build, at 7.1.
fn vouch(shc: &std::path::Path) -> Result<(), Gap> {
    let shape = Shape {
        vs: include_str!("../tools/shaders/color_uv.vert"),
        fs: include_str!("../tools/shaders/color_uv.frag"),
        ..Shape::bare("color_uv")
    };
    let m = compile71(shc, &shape)?.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(m.ver, 71, "v3d_shc --device 71 reports gpu.ver {}", m.ver);
    let (got, want) = (counts(&m), color_uv_row());
    println!("v3d_shc 7.1 color_uv: fs {} vs {} cs {} (qpu_counts_71.tsv: {want:?})", got[0], got[1], got[2]);
    assert_eq!(got, want, "{}: color_uv at 7.1 is not the H2 table's; this binary is not the reference", shc.display());
    Ok(())
}

/// Every word of every stage decodes at 7.1 and re-encodes to itself.
fn roundtrip(name: &str, m: &Mesa71) -> Result<(), String> {
    for (st, code) in [("fs", &m.fs.code), ("vs", &m.vs.code), ("cs", &m.cs.code)] {
        for (pc, &w) in code.iter().enumerate() {
            let i = decode71(w).map_err(|e| format!("{name} {st} word {pc} ({w:#018x}): {e:?}"))?;
            let back = encode71(&i).map_err(|e| format!("{name} {st} word {pc}: re-encode: {e:?}"))?;
            if back != w {
                return Err(format!("{name} {st} word {pc}: {w:#018x} re-encodes as {back:#018x}"));
            }
        }
    }
    Ok(())
}

/// The JSON reader and the stream resolution, on v3d_shc's layout (the
/// fields tools/v3d/v3d_shc.c prints that the harness reads). Needs no tool,
/// so it runs where the comparison is a gap.
#[test]
fn the_harness_reads_v3d_shc_json_and_resolves_the_stream() {
    let json = r#"{
  "gpu": { "ver": 71, "vpm_size": 65536, "vpm_size_source": "assumed", "rev": 7, "compat_rev": 0 },
  "fs": { "num_inputs": 0, "input_slots": [], "flat_shade_flags": "0x00000000", "uniforms": [{ "contents": 1, "data": "0x00000006" }, { "contents": 0, "data": "0xffffff00" }], "code": ["0x3d803186bb800000", "0x3c203186bb800000"], "threads": 4, "single_seg": false, "spill_size": 0 },
  "vs": { "uniforms": [], "code": ["0x3c203186bb800000"], "threads": 1 },
  "cs": { "uniforms": [], "code": [], "threads": 1 },
  "uniform_names": [{ "stage": "frag", "name": "u_t", "slot": 1, "vec4s": 1 }, { "stage": "frag", "name": "u_tex", "binding": 0, "vec4s": 0 }],
  "attributes": [{ "name": "a_pos", "location": 0, "records": [0, null] }]
}"#;
    let m = v3d71_ref::parse(json).expect("parses");
    assert_eq!((m.ver, counts(&m), m.fs.threads), (71, [2, 1, 0], 4));
    assert_eq!(m.fs.code[0], 0x3d80_3186_bb80_0000);
    assert_eq!(m.fs.uniforms, vec![(1, 6), (0, 0xffff_ff00)]);
    assert!(m.uniform_names[1].sampler && !m.uniform_names[0].sampler);
    // u_t at slot 1: dword 6 is component 2 of the shape's uniform values.
    let shape = Shape { uniforms: vec![0.1, 0.2, 0.3, 0.4], ..Shape::bare("reader") };
    assert_eq!(m.fs_stream(&shape), Ok(vec![0.3f32.to_bits(), 0xffff_ff00]));
    // A dword past the shape's values, and an unmodelled contents, are refused.
    let short = Shape { uniforms: vec![0.1], ..Shape::bare("reader") };
    assert!(m.fs_stream(&short).is_err());
    let mut odd = m.clone();
    odd.fs.uniforms.push((50, 0));
    assert!(odd.fs_stream(&shape).unwrap_err().contains("contents 50"));
    assert!(v3d71_ref::parse("{ \"gpu\": { \"ver\": 71 } }").is_err());
}

#[test]
fn mesas_7_1_programs_for_the_four_shapes() {
    let shc = match shc() {
        Ok(p) => p,
        Err(g) => return gap("mesas_7_1_programs_for_the_four_shapes", &g),
    };
    if let Err(g) = vouch(&shc) {
        return gap("mesas_7_1_programs_for_the_four_shapes", &g);
    }
    let mut wrong = Vec::new();
    for shape in shapes() {
        let m = match compile71(&shc, &shape) {
            Err(g) => return gap("mesas_7_1_programs_for_the_four_shapes", &g),
            Ok(Err(e)) => {
                wrong.push(e);
                continue;
            }
            Ok(Ok(m)) => m,
        };
        let got = counts(&m);
        println!(
            "v3d_shc 7.1 {:<12} fs {:>3} vs {:>3} cs {:>3} (fs threads {}, uniforms {:?})",
            shape.name, got[0], got[1], got[2], m.fs.threads, m.fs.uniforms
        );
        let pin = PINNED.iter().find(|(n, _)| *n == shape.name).map(|(_, c)| *c);
        if pin != Some(got) {
            wrong.push(format!("{}: counts {got:?}, pinned {pin:?}", shape.name));
        }
        if let Err(e) = roundtrip(shape.name, &m) {
            wrong.push(e);
        }
        let run = m.fs_stream(&shape).and_then(|stream| {
            let shape = Shape { threads: m.fs.threads, ..shape.clone() };
            run71_stream(&m.fs.code, &stream, &shape)
        });
        match run {
            Ok(o) => println!("  sim71: Mesa's fs gives the shape's colour in 16 lanes ({} executed)", o.instructions),
            Err(e) => wrong.push(format!("{}: Mesa's fs in sim71: {e}", shape.name)),
        }
        // Negative control: lane 5 expecting another colour must be caught.
        let mut control = Shape { threads: m.fs.threads, ..shape.clone() };
        control.expect[5][1] += 0.25;
        let stream = m.fs_stream(&shape).unwrap_or_default();
        if run71_stream(&m.fs.code, &stream, &control).is_ok() {
            wrong.push(format!("{}: a wrong lane-5 colour passed (the comparison is blind)", shape.name));
        }
    }
    assert!(wrong.is_empty(), "Mesa's 7.1 shapes:\n{}", wrong.join("\n"));
}
