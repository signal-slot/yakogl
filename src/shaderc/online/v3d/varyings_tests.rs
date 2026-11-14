// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The compiler's varying limits (plan #359, docs/notes/v3d-varyings.md):
//! what a vertex/fragment pair with 15 vec4 varyings (60 components, ES 3.0's
//! MAX_VARYING_VECTORS), 16 vec4 vertex outputs (64 components,
//! MAX_VERTEX_OUTPUT_COMPONENTS) feeding 15, the point size beside them, and flat,
//! centroid and smooth varyings compiles to and, in the interpreter, carries
//! through; and the one word of flat/centroid flags the shader record
//! carries (chain.rs `FLAGS_WORD_INPUTS`): the qualified inputs go first
//! when one would land past input 23, and more than 24 are refused by name.

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
    vs_source_with(n, es3, &[], false)
}

/// [`vs_source`] with `v{i}` flat for each i in `flat` (GLSL ES 3.00 only),
/// and a gl_PointSize write when `psiz`.
fn vs_source_with(n: usize, es3: bool, flat: &[usize], psiz: bool) -> String {
    let (attr, out) = if es3 { ("in", "out") } else { ("attribute", "varying") };
    let mut s = String::new();
    if es3 {
        s.push_str("#version 300 es\n");
    }
    s.push_str(&std::format!("{attr} vec4 a;\n"));
    for i in 0..n {
        let q = if flat.contains(&i) { "flat " } else { "" };
        s.push_str(&std::format!("{q}{out} vec4 v{i};\n"));
    }
    s.push_str("void main() {\n");
    if psiz {
        s.push_str("gl_PointSize = 2.0;\n");
    }
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
    fs_source_with(n, pick, es3, &[])
}

/// [`fs_source`] with `v{i}` flat for each i in `flat`.
fn fs_source_with(n: usize, pick: usize, es3: bool, flat: &[usize]) -> String {
    let inn = if es3 { "in" } else { "varying" };
    let mut s = String::new();
    if es3 {
        s.push_str("#version 300 es\n");
    }
    s.push_str("precision highp float;\n");
    for i in 0..n {
        let q = if flat.contains(&i) { "flat " } else { "" };
        s.push_str(&std::format!("{q}{inn} vec4 v{i};\n"));
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
    check_fs_with(n, pick, es3, &[])
}

/// [`check_fs`] with `v{i}` flat for each i in `flat`: a flat input's value
/// is in C and noise in the ldvary result (the provoking vertex's value,
/// Mesa emit_flat_varying), a smooth one's the reverse, so an input whose
/// flag is not where its ldvary is shows in the colour.
fn check_fs_with(n: usize, pick: usize, es3: bool, flat: &[usize]) -> Result<(), String> {
    with_program(&vs_source_with(n, es3, flat, false), &fs_source_with(n, pick, es3, flat), |p| {
        assert_eq!(p.fs.num_inputs as usize, 4 * n, "FS inputs: one per component read");
        let noise = [1000.0f32.to_bits(); LANES];
        let varyings =
            p.fs.input_slots
                .iter()
                .enumerate()
                .map(|(i, &(slot, comp))| {
                    let loc = usize::from(slot - VARYING_SLOT_VAR0);
                    let v = core::array::from_fn(|l| (l as f32 + (16 * loc + usize::from(comp)) as f32).to_bits());
                    if i < 32 && p.fs.flat_shade_flags & (1 << i) != 0 {
                        Varying { vary: noise, c: v }
                    } else {
                        Varying { vary: v, c: [0; LANES] }
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

/// MAX_VERTEX_OUTPUT_COMPONENTS 64: a vertex shader with 16 vec4 outputs
/// links with a fragment shader reading 15 of them (MAX_VARYING_VECTORS),
/// and writes what that one reads after the four fixed-function rows. A
/// fragment shader reading all 64 components does not compile (its 64
/// ldvary results are live at once: "more live values than 2 threads have
/// registers"), which is why the V3D answers 15 vectors, not Mesa's 16.
#[test]
fn a_vs_with_16_vec4_outputs_feeds_15() {
    with_program(&vs_source(16, false), &fs_source(15, 14, false), |p| {
        let rows = run_vs(p);
        assert_eq!(rows.len(), 4 + 60, "x, y, z, 1/w and the 60 components read");
        for i in 0..15 {
            for c in 0..4 {
                assert!(has_row(&rows, (16 * i + c) as f32), "v{i}.{c}");
            }
        }
    })
    .expect("a 16-output vertex shader links with a 15-input fragment shader");
    let e = with_program(&vs_source(16, false), &fs_source(16, 0, false), |_| ()).unwrap_err();
    assert_eq!(e, "NotYet(\"more live values than 2 threads have registers (no spilling yet)\")");
}

/// The point size takes its own row before the varyings (Mesa's
/// per_vertex_point_size layout): 4 + 1 + 60 rows, past the 64 the VPM
/// output stopped at before select.rs `VPM_MAX_OUTPUT_ROWS`, and every
/// varying still lands.
#[test]
fn a_vs_writes_15_vec4_varyings_and_the_point_size() {
    with_program(&vs_source_with(16, false, &[], true), &fs_source(15, 0, false), |p| {
        let rows = run_vs(p);
        assert_eq!(rows.len(), 4 + 1 + 60, "x, y, z, 1/w, the point size and 60 varyings");
        assert!(has_row(&rows, 0.0) && has_row(&rows, 16.0 * 14.0 + 3.0), "the first and last varying component");
    })
    .expect("a 15-vector pair with gl_PointSize compiles");
}

/// 64 fragment inputs is the most the shader record names (chain.rs
/// `FS_MAX_INPUTS`); 17 vec4 is refused by name, not emitted wrong.
#[test]
fn more_than_64_fragment_inputs_are_refused_by_name() {
    let e = with_program(&vs_source(17, false), &fs_source(17, 0, false), |_| ()).unwrap_err();
    assert_eq!(e, "NotYet(\"more than 64 fragment shader input components\")");
}

/// A flat vec4 that would be inputs 56-59 of 15 vectors lies past the one
/// flags word, so the flat inputs go first (flags 0xf); the fragment
/// shader's ldvary order and the vertex shader's rows follow, and each
/// picked value still arrives, flat or smooth.
#[cfg(not(feature = "report-es2"))]
#[test]
fn a_flat_varying_past_input_23_moves_into_the_flags_word() {
    with_program(&vs_source_with(15, true, &[14], false), &fs_source_with(15, 14, true, &[14]), |p| {
        assert_eq!(p.fs.flat_shade_flags, 0xf, "v14's four components are inputs 0-3");
        assert_eq!(p.fs.centroid_flags, 0);
        for (i, &(slot, comp)) in p.fs.input_slots.iter().take(4).enumerate() {
            assert_eq!((slot, usize::from(comp)), (VARYING_SLOT_VAR0 + 14, i), "input {i}");
        }
        /* rows 4-7 (after x, y, z, 1/w) are v14's, in the same order */
        let rows = run_vs(p);
        for c in 0..4 {
            let want = |l: usize| (l as f32 + (16 * 14 + c) as f32).to_bits();
            assert!((0..LANES).all(|l| rows[4 + c][l] == want(l)), "row {} is v14.{c}", 4 + c);
        }
    })
    .expect("compiles");
    for pick in [0, 13, 14] {
        check_fs_with(15, pick, true, &[14]).expect("compiles");
    }
}

/// A program whose flat inputs all lie below 24 keeps the link's order.
#[cfg(not(feature = "report-es2"))]
#[test]
fn flat_varyings_within_the_flags_word_keep_their_order() {
    with_program(&vs_source_with(15, true, &[1], false), &fs_source_with(15, 0, true, &[1]), |p| {
        assert_eq!(p.fs.flat_shade_flags, 0xf0, "v1 is inputs 4-7");
        for (i, &(slot, _)) in p.fs.input_slots.iter().enumerate() {
            assert_eq!(usize::from(slot - VARYING_SLOT_VAR0), i / 4, "input {i}");
        }
    })
    .expect("compiles");
}

/// 24 flat components fill the word (15 vectors, six of them flat); a 25th
/// has no bit and is refused by name, never drawn smooth.
#[cfg(not(feature = "report-es2"))]
#[test]
fn more_than_24_flat_components_are_refused_by_name() {
    let six = [9, 10, 11, 12, 13, 14];
    with_program(&vs_source_with(15, true, &six, false), &fs_source_with(15, 0, true, &six), |p| {
        assert_eq!(p.fs.flat_shade_flags, 0x00ff_ffff);
    })
    .expect("24 flat components compile");
    check_fs_with(15, 12, true, &six).expect("compiles");
    let seven = [8, 9, 10, 11, 12, 13, 14];
    let e = with_program(&vs_source_with(15, true, &seven, false), &fs_source_with(15, 0, true, &seven), |_| ())
        .unwrap_err();
    assert_eq!(e, "NotYet(\"more than 24 flat and centroid varying components\")");
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
