// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Probe of the compiler's varying limits (plan #359, `v3d-varyings-probe`):
//! what a vertex/fragment pair with 15 vec4 varyings (60 components), a 16th
//! vec4 vertex output (64 components), and flat, centroid and smooth varyings
//! compiles to and, in the interpreter, carries through. They record the
//! state before the ES 3.0 raise (MAX_VARYING_VECTORS 15,
//! MAX_VERTEX_OUTPUT_COMPONENTS 64); a test that fails today is `#[ignore]`d
//! with the limit it needs, and runs with `--ignored`.

use super::chain::{compile_program, Code, Prim, Target, V3dProgram, VARYING_SLOT_VAR0};
use super::sim::{self, Env, FsInputs, Inputs, Varying, VsInputs, LANES};
use crate::shaderc::online::sema::Stage;
use crate::shaderc::online::{lower, store};
use std::string::String;
use std::vec::Vec;

const TARGET: Target = Target { vpm_size: 65536 };

/// `n` vec4 varyings `v0..`: component c of `v{i}` is `a.x + 16 i + c`, so
/// only a lost, swapped or misplaced varying changes what is read.
fn vs_source(n: usize, es3: bool) -> String {
    let (attr, out) = if es3 { ("in", "out") } else { ("attribute", "varying") };
    let mut s = String::new();
    if es3 {
        s.push_str("#version 300 es\n");
    }
    s.push_str(&std::format!("{attr} vec4 a;\n"));
    for i in 0..n {
        s.push_str(&std::format!("{out} vec4 v{i};\n"));
    }
    s.push_str("void main() {\n");
    for i in 0..n {
        s.push_str(&std::format!(
            "v{i} = vec4(a.x) + vec4({:.1}, {:.1}, {:.1}, {:.1});\n",
            16 * i,
            16 * i + 1,
            16 * i + 2,
            16 * i + 3
        ));
    }
    s.push_str("gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n");
    s
}

/// A fragment shader reading `v0..v{n}` and writing `v{pick} / 1024`: the
/// others are read (and so inputs) through a multiply by zero the compiler
/// may not remove (floats are not folded across `v * 0.0`).
fn fs_source(n: usize, pick: usize, es3: bool) -> String {
    let inn = if es3 { "in" } else { "varying" };
    let mut s = String::new();
    if es3 {
        s.push_str("#version 300 es\n");
    }
    s.push_str("precision highp float;\n");
    for i in 0..n {
        s.push_str(&std::format!("{inn} vec4 v{i};\n"));
    }
    if es3 {
        s.push_str("layout(location = 0) out vec4 o;\n");
    }
    let out = if es3 { "o" } else { "gl_FragColor" };
    s.push_str("void main() {\nvec4 acc = vec4(0.0);\n");
    for i in 0..n {
        s.push_str(&std::format!("acc += v{i} * {};\n", if i == pick { "1.0" } else { "0.0" }));
    }
    s.push_str(&std::format!("{out} = acc / 1024.0;\n}}\n"));
    s
}

fn uniforms_of(c: &Code<'_>) -> Vec<u32> {
    c.uniforms
        .iter()
        .map(|u| match u.contents {
            0 => u.data,
            2..=5 => 1.0f32.to_bits(),
            c => panic!("uniform contents {c} not modelled here"),
        })
        .collect()
}

/// Compile the pair; `Err` is the compiler's refusal.
fn with_program<R>(vs: &str, fs: &str, f: impl FnOnce(&V3dProgram<'_>) -> R) -> Result<R, String> {
    let mut out = None;
    let mut err = None;
    store::with_scratch(|a| {
        let vsrc = a.alloc_str(vs).unwrap();
        let fsrc = a.alloc_str(fs).unwrap();
        let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
        let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
        match compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a) {
            Ok(p) => out = Some(f(&p)),
            Err(e) => err = Some(std::format!("{e:?}")),
        }
    })
    .expect("scratch");
    out.ok_or_else(|| err.unwrap())
}

/// Run the vertex shader on lanes with `a.x = lane` and return the rows it
/// wrote.
fn run_vs(p: &V3dProgram<'_>) -> Vec<[u32; LANES]> {
    let rows = p.attributes.iter().filter(|e| e.1.is_some()).count();
    let row: [u32; LANES] = core::array::from_fn(|l| (l as f32).to_bits());
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let inputs = Inputs::Vertex(VsInputs { vpm: std::vec![row; rows.max(1)] });
    let got = sim::run(p.vs.code.words, &uniforms_of(&p.vs.code), &inputs, &env).expect("the vertex shader runs");
    got.vpm_out.iter().flatten().copied().collect()
}

/// Whether `rows` holds the row of `lane + base` for every lane.
fn has_row(rows: &[[u32; LANES]], base: f32) -> bool {
    rows.iter().any(|r| (0..LANES).all(|l| r[l] == (l as f32 + base).to_bits()))
}

/// Every `v{i}` component `16 i + c` is among the vertex shader's output rows.
fn check_vs_rows(n: usize, es3: bool) -> Result<(), String> {
    with_program(&vs_source(n, es3), &fs_source(n, 0, es3), |p| {
        let rows = run_vs(p);
        for i in 0..n {
            for c in 0..4 {
                assert!(has_row(&rows, (16 * i + c) as f32), "v{i}.{c} is not among the {} rows written", rows.len());
            }
        }
    })
}

/// Run the fragment shader with each input's value `lane + 16 * loc + comp`
/// (by `input_slots`, so the test does not assume the link's order) and
/// compare `acc / 1024`: the picked varying's four components.
fn check_fs(n: usize, pick: usize, es3: bool) -> Result<(), String> {
    with_program(&vs_source(n, es3), &fs_source(n, pick, es3), |p| {
        assert_eq!(p.fs.num_inputs as usize, 4 * n, "FS inputs: one per component read");
        let varyings =
            p.fs.input_slots
                .iter()
                .map(|&(slot, comp)| {
                    let loc = usize::from(slot - VARYING_SLOT_VAR0);
                    Varying {
                        vary: core::array::from_fn(|l| (l as f32 + (16 * loc + usize::from(comp)) as f32).to_bits()),
                        c: [0; LANES],
                    }
                })
                .collect();
        let inputs = FsInputs {
            w: [1.0f32.to_bits(); LANES],
            w_centroid: [1.0f32.to_bits(); LANES],
            z: [0; LANES],
            x: [0; LANES],
            y: [0; LANES],
            back_facing: [false; LANES],
            varyings,
        };
        let env = Env { threads: p.fs.threads, ..Env::default() };
        let o =
            sim::run(p.fs.code.words, &uniforms_of(&p.fs.code), &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                .expect("the fragment shader runs");
        assert_eq!(o.tlb.len(), 2, "one f16 colour, two TLB words");
        let h = |x: f32| u32::from(sim::f32_to_half(x));
        for l in 0..LANES {
            let w = |c: usize| h((l as f32 + (16 * pick + c) as f32) / 1024.0);
            let want = [w(0) | w(1) << 16, w(2) | w(3) << 16];
            let got = [o.tlb[0].value[l].unwrap(), o.tlb[1].value[l].unwrap()];
            assert_eq!(got, want, "lane {l}");
        }
    })
}

#[test]
fn a_vs_writes_15_vec4_varyings() {
    check_vs_rows(15, false).expect("a 15-vector pair compiles");
}

#[test]
fn a_fs_reads_15_vec4_varyings() {
    for pick in [0, 7, 14] {
        check_fs(15, pick, false).expect("a 15-vector pair compiles");
    }
}

/// Fails today with `NotYet("more live values than 2 threads have
/// registers")`: the 69 VPM rows are accepted (select.rs
/// `MAX_VPM_OUTPUT_ROWS`), but 64 computed components are all live at the
/// end of the body (no spilling, the varyings are stored after it).
#[test]
#[ignore = "MAX_VERTEX_OUTPUT_COMPONENTS 64: 16 computed vec4 outputs run out of registers (no spilling yet)"]
fn a_vs_writes_16_vec4_outputs() {
    check_vs_rows(16, false).expect("a 16-vector (64-component) pair compiles");
}

/// The same 15 vectors from a GLSL ES 3.00 pair.
#[cfg(not(feature = "report-es2"))]
#[test]
fn an_es300_pair_carries_15_vec4_varyings() {
    check_vs_rows(15, true).expect("compiles");
    check_fs(15, 14, true).expect("compiles");
}

/// Flat, centroid and smooth varyings together: the program's flag words
/// mark the right inputs.
#[cfg(not(feature = "report-es2"))]
#[test]
fn flat_centroid_and_smooth_varyings_set_their_flags() {
    const VS: &str = "#version 300 es\nin vec4 a;\nflat out vec2 f;\ncentroid out vec2 c;\nout vec2 s;\n\
        void main() { f = a.xy; c = a.xy + vec2(16.0); s = a.xy + vec2(32.0); \
        gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nflat in vec2 f;\ncentroid in vec2 c;\nin vec2 s;\n\
        layout(location = 0) out vec4 o;\nvoid main() { o = vec4(f.x + c.x + s.x, f.y, c.y, s.y) / 256.0; }\n";
    with_program(VS, FS, |p| {
        assert_eq!(p.fs.num_inputs, 6);
        assert_eq!(p.fs.flat_shade_flags.count_ones(), 2, "f.x and f.y");
        assert_eq!(p.fs.centroid_flags.count_ones(), 2, "c.x and c.y");
        assert_eq!(p.fs.flat_shade_flags & p.fs.centroid_flags, 0);
    })
    .expect("compiles");
}

/// 10 floats, 10 vec2 and 10 vec3: 60 components in 30 varyings, all read.
#[test]
fn scalars_and_vec2s_pack_into_60_components() {
    let mut vs = String::from("attribute vec4 a;\n");
    let mut fs = String::from("precision highp float;\n");
    let mut body_v = String::new();
    let mut body_f = String::from("float acc = 0.0;\n");
    for i in 0..10 {
        let decl = std::format!("varying float s{i};\nvarying vec2 t{i};\nvarying vec3 u{i};\n");
        vs.push_str(&decl);
        fs.push_str(&decl);
        body_v.push_str(&std::format!(
            "s{i} = a.x + {i}.0; t{i} = a.xx + vec2({}.0, {}.0); u{i} = a.xxx + vec3({}.0, {}.0, {}.0);\n",
            100 + i,
            200 + i,
            300 + i,
            400 + i,
            500 + i
        ));
        body_f.push_str(&std::format!("acc += s{i} + t{i}.x + t{i}.y + u{i}.x + u{i}.y + u{i}.z;\n"));
    }
    vs.push_str(&std::format!("void main() {{\n{body_v}gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}}\n"));
    fs.push_str(&std::format!("void main() {{\n{body_f}gl_FragColor = vec4(acc / 65536.0);\n}}\n"));
    let n = with_program(&vs, &fs, |p| {
        let rows = run_vs(p);
        for i in 0..10 {
            assert!(has_row(&rows, i as f32), "s{i}");
            assert!(has_row(&rows, (100 + i) as f32), "t{i}.x");
            assert!(has_row(&rows, (500 + i) as f32), "u{i}.z");
        }
        p.fs.num_inputs
    })
    .expect("compiles");
    assert_eq!(n, 60, "every component of the 30 varyings is an FS input");
}
