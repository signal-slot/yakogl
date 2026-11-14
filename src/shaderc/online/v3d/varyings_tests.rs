// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Probe of the compiler's varying limits (plan #359, `v3d-varyings-probe`):
//! what a vertex/fragment pair with 15 vec4 varyings (60 components), a 16th
//! vec4 vertex output (64 components), and flat, centroid and smooth varyings
//! compiles to and, in the interpreter, carries through. They record the
//! limits the ES 3.0 raise rests on (MAX_VARYING_VECTORS 15,
//! MAX_VERTEX_OUTPUT_COMPONENTS 64).

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

/// A vertex shader writing 16 vec4 (ES 3.0's 64 vertex output components)
/// beside a fragment shader reading 15 compiles: the 16th is not a fragment
/// input, so it takes no VPM row (the chain writes the outputs the fragment
/// shader reads), and the fifteen are all there.
#[test]
fn a_vs_writing_16_vec4_outputs_beside_a_15_vec4_fs_compiles() {
    let rows = with_program(&vs_source(16, false), &fs_source(15, 3, false), |p| {
        assert_eq!(p.fs.num_inputs, 60);
        run_vs(p)
    })
    .expect("compiles");
    for i in 0..15 {
        assert!(has_row(&rows, (16 * i) as f32), "v{i}.0");
    }
}

/// Fifteen vec4 and gl_PointSize: the four fixed-function rows, the point
/// size and 60 varyings are 65 VPM rows, nine sectors (select.rs
/// `VPM_OUTPUT_ROWS_MAX`).
#[test]
fn fifteen_vec4_and_a_point_size_fit_the_vpm() {
    let vs = vs_source(15, false).replace("gl_Position =", "gl_PointSize = 1.0;\ngl_Position =");
    with_program(&vs, &fs_source(15, 0, false), |p| assert_eq!(p.vs.vpm_output_size, 9)).expect("compiles");
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

/// Fifteen vec4 varyings, one in three flat and one in three centroid: the
/// flags land in the three 24-input words of the program (input 24 is the
/// first of word 1, input 48 of word 2), each at its input's own bit.
#[cfg(not(feature = "report-es2"))]
#[test]
fn flat_and_centroid_flags_span_three_words() {
    let qual = |i: usize| ["", "flat ", "centroid "][i % 3];
    let mut vs = vs_source(15, true);
    let mut fs = fs_source(15, 0, true);
    for i in 0..15 {
        vs = vs.replace(&std::format!("out vec4 v{i};"), &std::format!("{}out vec4 v{i};", qual(i)));
        fs = fs.replace(&std::format!("in vec4 v{i};"), &std::format!("{}in vec4 v{i};", qual(i)));
    }
    with_program(&vs, &fs, |p| {
        assert_eq!(p.fs.num_inputs, 60);
        let word = |w: usize, lo: u32, hi: [u32; 2]| if w == 0 { lo } else { hi[w - 1] };
        let (mut flats, mut centroids) = (0, 0);
        for (k, &(slot, _)) in p.fs.input_slots.iter().enumerate() {
            let q = qual(usize::from(slot - VARYING_SLOT_VAR0));
            let (w, bit) = (k / 24, k % 24);
            let flat = word(w, p.fs.flat_shade_flags, p.fs.flat_shade_flags_hi) >> bit & 1;
            let cen = word(w, p.fs.centroid_flags, p.fs.centroid_flags_hi) >> bit & 1;
            assert_eq!((flat, cen), (u32::from(q == "flat "), u32::from(q == "centroid ")), "input {k}");
            flats += flat;
            centroids += cen;
        }
        assert_eq!((flats, centroids), (20, 20), "five flat and five centroid vec4");
        for w in 0..3 {
            let (f, c) = (
                word(w, p.fs.flat_shade_flags, p.fs.flat_shade_flags_hi),
                word(w, p.fs.centroid_flags, p.fs.centroid_flags_hi),
            );
            assert_eq!((f >> 24, c >> 24), (0, 0), "word {w} holds 24 inputs");
        }
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
