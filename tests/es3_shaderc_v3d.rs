// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 4.2 shader back-end gaps found by the dEQP gles3 shader corpus
//! (docs/es3-shaderc-coverage.md): matrix vertex attributes (a matCxR takes C
//! consecutive locations), integer shift left/right and `trunc()`. Each
//! shader is compiled and linked through `SHC_ONLINE`, which is what
//! glCompileShader and glLinkProgram reach.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shaderc_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// glCompileShader on both stages, then glLinkProgram.
fn build(vs: &str, fs: &str) -> Result<(), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

const FS_VEC4: &str = "#version 300 es\nprecision mediump float;\nin vec4 v;\nout vec4 o;\nvoid main() { o = v; }\n";

#[test]
fn matrix_attributes_build() {
    for (ty, rows, cols) in [
        ("mat2", 2, 2),
        ("mat3", 3, 3),
        ("mat4", 4, 4),
        ("mat2x3", 3, 2),
        ("mat2x4", 4, 2),
        ("mat3x2", 2, 3),
        ("mat3x4", 4, 3),
        ("mat4x2", 2, 4),
        ("mat4x3", 3, 4),
    ] {
        // every column is read, widened to a vec4
        let col = |c: usize| match rows {
            2 => format!("vec4(a_in0[{c}], 0.0, 0.0)"),
            3 => format!("vec4(a_in0[{c}], 0.0)"),
            _ => format!("a_in0[{c}]"),
        };
        let sum = (0..cols).map(col).collect::<Vec<_>>().join(" + ");
        let vs = format!(
            "#version 300 es\nin highp {ty} a_in0;\nin highp vec4 a_pos;\nout highp vec4 v;\n\
             void main() {{\n  v = {sum};\n  gl_Position = a_pos;\n}}\n"
        );
        build(&vs, FS_VEC4).unwrap_or_else(|e| panic!("{ty}: {e}\n{vs}"));
    }
}

#[test]
fn matrix_attribute_product_builds() {
    let vs = "#version 300 es\nin highp mat4x3 a_in0;\nin highp vec4 a_pos;\nout highp vec4 v;\n\
              void main() {\n  v = vec4(a_in0 * a_pos, 1.0);\n  gl_Position = a_pos;\n}\n";
    build(vs, FS_VEC4).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn integer_shifts_build() {
    for (ty, conv) in
        [("int", "float(r)"), ("uint", "float(r)"), ("ivec3", "vec3(r), 1.0"), ("uvec2", "vec2(r), 0.0, 1.0")]
    {
        for op in ["<<", ">>"] {
            let vs = format!(
                "#version 300 es\nin highp {ty} a;\nin highp {ty} b;\nin highp vec4 a_pos;\nout highp vec4 v;\n\
                 void main() {{\n  highp {ty} r = a {op} b;\n  v = vec4({conv});\n  gl_Position = a_pos;\n}}\n"
            );
            build(&vs, FS_VEC4).unwrap_or_else(|e| panic!("{ty} {op}: {e}\n{vs}"));
        }
    }
}

#[test]
fn integer_not_builds_and_runs() {
    let vs = "#version 300 es\nin highp vec4 a;\nout highp vec4 v;\n\
              void main() {\n  int x = int(a.x);\n  v = vec4(float(~x), 0.0, 0.0, 0.0);\n  gl_Position = a;\n}\n";
    build(vs, FS_VEC4).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn integer_shift_by_constant_builds() {
    let vs = "#version 300 es\nin highp int a;\nin highp vec4 a_pos;\nout highp vec4 v;\n\
              void main() {\n  int r = (a << 3) >> 1;\n  v = vec4(float(r));\n  gl_Position = a_pos;\n}\n";
    build(vs, FS_VEC4).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn trunc_builds() {
    for body in ["float r = trunc(a.x); v = vec4(r);", "v = trunc(a);"] {
        let vs = format!(
            "#version 300 es\nin highp vec4 a;\nin highp vec4 a_pos;\nout highp vec4 v;\n\
             void main() {{\n  {body}\n  gl_Position = a_pos;\n}}\n"
        );
        build(&vs, FS_VEC4).unwrap_or_else(|e| panic!("{body}: {e}"));
    }
    // and in the fragment stage
    let vs =
        "#version 300 es\nin highp vec4 a_pos;\nout highp vec4 v;\nvoid main() { v = a_pos; gl_Position = a_pos; }\n";
    let fs = "#version 300 es\nprecision highp float;\nin vec4 v;\nout vec4 o;\nvoid main() { o = trunc(v * 4.0); }\n";
    build(vs, fs).unwrap_or_else(|e| panic!("{e}"));
}

// ---- values, on the QPU interpreter ----------------------------------------

use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// `vs` run on the interpreter; `attrs` gives, per declared attribute
/// location, its components. Returns the last four VPM output rows (the
/// varying `v`) of lane 0.
fn run_vs(vs: &str, attrs: &[(u8, &[f32])]) -> [f32; 4] {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(FS_VEC4).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .expect("the program compiles");
    let uniforms: Vec<u32> =
        p.vs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                2..=5 => 1.0f32.to_bits(),
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    // the VPM input rows: each record's components, in record order
    let mut rows: Vec<[u32; LANES]> = Vec::new();
    for rec in 0..p.attributes.iter().filter(|e| e.1.is_some()).count() {
        let (loc, _) = p.attributes.iter().find(|e| e.1 == Some(rec as u8)).expect("a record has a location");
        let comps = attrs.iter().find(|e| e.0 == *loc).unwrap_or_else(|| panic!("no data for location {loc}")).1;
        assert!(
            p.vs.vattr_sizes[rec] as usize <= comps.len(),
            "record {rec} wants {} components",
            p.vs.vattr_sizes[rec]
        );
        for c in comps.iter().take(p.vs.vattr_sizes[rec] as usize) {
            rows.push([c.to_bits(); LANES]);
        }
    }
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &uniforms, &Inputs::Vertex(VsInputs { vpm: rows }), &env).unwrap_or_else(|e| {
        let listing: Vec<String> =
            p.vs.code
                .instrs
                .iter()
                .map(|i| {
                    let mut s = String::new();
                    yakogl::shaderc::online::v3d::listing::write_instr(&mut s, i).unwrap();
                    s
                })
                .collect();
        panic!("the vertex program runs: {e}\n{listing:#?}\nattributes {:?}", p.attributes)
    });
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
    let v = &out[out.len() - 4..];
    core::array::from_fn(|c| f32::from_bits(v[c][0]))
}

#[test]
fn trunc_values() {
    let vs = "#version 300 es\nin highp vec4 a;\nout highp vec4 v;\nvoid main() { v = trunc(a); gl_Position = a; }\n";
    let got = run_vs(vs, &[(0, &[1.75, -1.75, 0.5, -0.25])]);
    assert_eq!(got, [1.0, -1.0, 0.0, -0.0]);
}

#[test]
fn shift_values() {
    let vs = "#version 300 es\nin highp vec4 a;\nout highp vec4 v;\n\
              void main() {\n  int x = int(a.x);\n  int y = int(a.y);\n  uint z = uint(a.z);\n  \
              v = vec4(float(x << 3), float(y >> 2), float(z >> 1u), float(z << 4u));\n  gl_Position = a;\n}\n";
    // 5 << 3 = 40; -17 >> 2 (arithmetic) = -5; 100 >> 1 = 50; 100 << 4 = 1600
    let got = run_vs(vs, &[(0, &[5.0, -17.0, 100.0, 1.0])]);
    assert_eq!(got, [40.0, -5.0, 50.0, 1600.0]);
}

#[test]
fn matrix_attribute_columns_come_from_consecutive_locations() {
    // a_in0 takes locations 1 and 2 (two columns of three), a_pos location 0
    let vs = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nlayout(location = 1) in highp mat2x3 a_in0;\n\
              out highp vec4 v;\n\
              void main() {\n  v = vec4(a_in0[0].x + a_in0[1].z, a_in0[0].y, a_in0[1].x, a_in0[1].y);\n  gl_Position = a_pos;\n}\n";
    let got = run_vs(vs, &[(0, &[0.0, 0.0, 0.0, 1.0]), (1, &[1.0, 2.0, 3.0]), (2, &[10.0, 20.0, 30.0])]);
    assert_eq!(got, [31.0, 2.0, 10.0, 20.0]);
}
