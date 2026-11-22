// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL ES 1.00 fragment built-ins at V3D 7.1 against 4.2: the dEQP-GLES2
//! and glmark2 shaders read gl_FragCoord, gl_FrontFacing and gl_PointCoord,
//! use `discard` and write gl_FragColor or gl_FragData[0]. Each small program
//! here (our own text) is compiled by the 4.2 chain (`chain::compile_program`)
//! and the 7.1 chain (`chain71::compile_program71`). Every 7.1 word decodes
//! with codec71 and the instructions pass verify71 (and fit the thread count).
//! Then both fragment shaders run in their interpreters (`sim::run_gen`, the
//! 4.2 model and `sim71`) on the same payload and varyings, and the TLB
//! writes and the multisample flags must be equal. Only fragment shaders run
//! (sim71 cannot yet run vertex or coordinate shaders past the thread end).
//!
//! The payload is a 16-lane (4 quads) fragment: pixels from (3, 5), W 1 and
//! 2, Z varying per lane, quads alternately front and back facing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_fs_builtins
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::select::{
    Uniform, QUNIFORM_CONSTANT, QUNIFORM_FB_PNTC_Y_OFFSET, QUNIFORM_FB_PNTC_Y_SCALE, QUNIFORM_FB_WPOS_Y_OFFSET,
    QUNIFORM_FB_WPOS_Y_SCALE,
};
use yakogl::shaderc::online::v3d::sim::{run_gen, Env, FsInputs, Inputs, Outcome, Varying, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::{lower, store};

const TARGET42: Target = Target { vpm_size: 65536 };
const WPOS_SCALE: f32 = -1.0;
const WPOS_OFFSET: f32 = 64.0;
const PNTC_SCALE: f32 = -1.0;
const PNTC_OFFSET: f32 = 1.0;

/// A vertex shader that writes the varying `v` (vec4) and `gl_Position`.
const VS: &str = "#version 100\nattribute vec4 a;\nvarying vec4 v;\nvoid main() { gl_Position = a; v = a; }\n";
const VS_NOVARY: &str = "#version 100\nattribute vec4 a;\nvoid main() { gl_Position = a; }\n";

/// What a program must show in the run, so an agreement is not vacuous.
#[derive(Clone, Copy)]
struct Expect {
    /// Some lane discarded, and some lane kept.
    discards: bool,
    /// The program has a varying `v` (one fragment input).
    varying: bool,
}

const fn e(discards: bool, varying: bool) -> Expect {
    Expect { discards, varying }
}

/// `(name, fragment source, expectations)`.
const CASES: &[(&str, &str, Expect)] = &[
    (
        "frag_coord_xy",
        "#version 100\nprecision highp float;\nvoid main() { gl_FragColor = vec4(gl_FragCoord.xy / 64.0, 0.25, 1.0); }\n",
        e(false, false),
    ),
    (
        "frag_coord_zw",
        "#version 100\nprecision highp float;\nvoid main() { gl_FragColor = vec4(gl_FragCoord.z, gl_FragCoord.w, 0.5, 1.0); }\n",
        e(false, false),
    ),
    (
        "frag_coord_rcp_w",
        "#version 100\nprecision highp float;\nvoid main() { gl_FragColor = vec4(1.0 / gl_FragCoord.w * 0.25, gl_FragCoord.z, 0.0, 1.0); }\n",
        e(false, false),
    ),
    (
        "front_facing_colour",
        "#version 100\nprecision highp float;\nvoid main() { gl_FragColor = gl_FrontFacing ? vec4(1.0, 0.5, 0.0, 1.0) : vec4(0.0, 0.5, 1.0, 1.0); }\n",
        e(false, false),
    ),
    (
        "front_facing_branch_varying",
        "#version 100\nprecision highp float;\nvarying vec4 v;\nvoid main() { vec4 c = v; if (!gl_FrontFacing) c = c.bgra * 0.5; gl_FragColor = c; }\n",
        e(false, true),
    ),
    (
        "point_coord",
        "#version 100\nprecision highp float;\nvoid main() { gl_FragColor = vec4(gl_PointCoord, 0.0, 1.0); }\n",
        e(false, false),
    ),
    (
        "point_coord_circle",
        "#version 100\nprecision highp float;\nvoid main() { vec2 p = gl_PointCoord - 0.5; if (p.x > -0.02) discard; gl_FragColor = vec4(p + 0.5, 0.0, 1.0); }\n",
        e(true, false),
    ),
    (
        "discard_branch",
        "#version 100\nprecision highp float;\nvarying vec4 v;\nvoid main() { if (v.x > 0.48) discard; gl_FragColor = v; }\n",
        e(true, true),
    ),
    (
        "discard_frag_coord",
        "#version 100\nprecision highp float;\nvoid main() { if (gl_FragCoord.x < 5.0) discard; gl_FragColor = vec4(1.0, gl_FragCoord.y / 32.0, 0.0, 1.0); }\n",
        e(true, false),
    ),
    (
        "discard_front_facing",
        "#version 100\nprecision highp float;\nvarying vec4 v;\nvoid main() { if (!gl_FrontFacing) discard; gl_FragColor = vec4(v.xyz, 1.0); }\n",
        e(true, true),
    ),
    (
        "frag_data0",
        "#version 100\nprecision highp float;\nvarying vec4 v;\nvoid main() { gl_FragData[0] = v * 2.0; }\n",
        e(false, true),
    ),
    (
        "frag_data0_combined",
        "#version 100\nprecision highp float;\nvarying vec4 v;\nvoid main() {\n  vec4 c = gl_FrontFacing ? v : v.zyxw;\n  if (gl_FragCoord.w < 0.75 && c.x > 0.38) discard;\n  gl_FragData[0] = vec4(c.xy + gl_PointCoord * 0.0, gl_FragCoord.z, 1.0);\n}\n",
        e(true, true),
    ),
];

/// The scratch region is one per process: the tests take turns.
static SCRATCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One program as the interpreters need it.
struct Code {
    words: Vec<u64>,
    uniforms: Vec<(u32, u32)>,
    threads: u32,
}

fn code71_checked(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
}

fn code(words: &[u64], us: &[Uniform], threads: u32) -> Code {
    Code { words: words.to_vec(), uniforms: us.iter().map(|u| (u.contents, u.data)).collect(), threads }
}

/// The 4.2 and 7.1 fragment shaders of `fs` (with a fitting VS), and the
/// number of fragment inputs each takes.
fn compile(name: &str, fs: &str, varying: bool) -> (Code, Code, usize, usize) {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let vs_src = if varying { VS } else { VS_NOVARY };
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs_src), (Stage::Fragment, fs)] {
            let l = lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch"))
                .unwrap_or_else(|e| panic!("{name} {stage:?}: front end: {}", e.msg));
            m.push(l.module);
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let p42 = compile_program(v, f, Prim::Triangles, TARGET42, a).unwrap_or_else(|e| panic!("{name}: 4.2: {e:?}"));
        let p71 =
            compile_program71(v, f, Prim::Triangles, TARGET71, a).unwrap_or_else(|e| panic!("{name}: 7.1: {e:?}"));
        code71_checked(&format!("{name} 7.1 fs"), &p71.fs.code, p71.fs.threads, p71.fs.single_seg);
        (
            code(p42.fs.code.words, p42.fs.code.uniforms, p42.fs.threads),
            code(p71.fs.code.words, p71.fs.code.uniforms, p71.fs.threads),
            p42.fs.num_inputs as usize,
            p71.fs.num_inputs as usize,
        )
    })
    .expect("scratch")
}

/// The stream's contents resolved: constants and the frame-buffer
/// transforms are all these programs use (no uniforms, no textures).
fn stream(name: &str, c: &Code) -> Vec<u32> {
    c.uniforms
        .iter()
        .map(|&(k, d)| match k {
            QUNIFORM_CONSTANT => d,
            QUNIFORM_FB_WPOS_Y_SCALE => WPOS_SCALE.to_bits(),
            QUNIFORM_FB_WPOS_Y_OFFSET => WPOS_OFFSET.to_bits(),
            QUNIFORM_FB_PNTC_Y_SCALE => PNTC_SCALE.to_bits(),
            QUNIFORM_FB_PNTC_Y_OFFSET => PNTC_OFFSET.to_bits(),
            k => panic!("{name}: uniform contents {k} (data {d:#x}) not modelled here"),
        })
        .collect()
}

fn pixel(l: usize) -> (f32, f32) {
    let (q, i) = (l / 4, l % 4);
    ((3 + 2 * (q & 1) + (i & 1)) as f32 + 0.5, (5 + 2 * (q >> 1) + (i >> 1)) as f32 + 0.5)
}

/// The payload: W 1 (quads 0, 1) and 2 (quads 2, 3), Z 1/4 + l/64, quads
/// alternately front and back facing, `n` varyings
/// `(1/4 + i/32 + x/64 + y/128) / W` with C 1/8.
fn payload(n: usize) -> Inputs {
    let w = |l: usize| if l < 8 { 1.0f32 } else { 2.0 };
    let varyings = (0..n)
        .map(|i| Varying {
            vary: core::array::from_fn(|l| {
                let (x, y) = pixel(l);
                ((0.25 + i as f32 / 32.0 + x / 64.0 + y / 128.0) / w(l)).to_bits()
            }),
            c: [0.125f32.to_bits(); LANES],
        })
        .collect();
    Inputs::Fragment(Box::new(FsInputs {
        w: core::array::from_fn(|l| w(l).to_bits()),
        w_centroid: core::array::from_fn(|l| w(l).to_bits()),
        z: core::array::from_fn(|l| (0.25 + l as f32 / 64.0).to_bits()),
        x: core::array::from_fn(|l| pixel(l).0.to_bits()),
        y: core::array::from_fn(|l| pixel(l).1.to_bits()),
        back_facing: core::array::from_fn(|l| (l / 4) % 2 == 1),
        varyings,
    }))
}

fn run(name: &str, gen: Gen, c: &Code, inputs: &Inputs) -> Outcome {
    let env = Env { threads: c.threads, quads: true, ..Env::default() };
    run_gen(gen, &c.words, &stream(name, c), inputs, &env).unwrap_or_else(|e| panic!("{name} {gen:?}: {e}"))
}

fn text(o: &Outcome) -> Vec<String> {
    let mut v: Vec<String> =
        o.tlb.iter().map(|t| format!("tlb config {:x?} values {:x?}", t.config, t.value)).collect();
    v.push(format!("msf {:x?}", o.msf));
    v
}

fn check(name: &str) {
    let &(_, fs, expect) = CASES.iter().find(|c| c.0 == name).expect("a case");
    let (c42, c71, n42, n71) = compile(name, fs, expect.varying);
    assert_eq!(n42, n71, "{name}: fragment inputs, 4.2 vs 7.1");
    assert_eq!(n42 > 0, expect.varying, "{name}: fragment inputs");
    /* gl_PointCoord's implicit varyings are not in `num_inputs`: supply spare ones (both read the same) */
    let inputs = payload(n42.max(8));
    let a = run(name, Gen::V42, &c42, &inputs);
    let b = run(name, Gen::V71, &c71, &inputs);
    assert_eq!(text(&a), text(&b), "{name}: 4.2 vs 7.1 outputs");
    let defined: usize = a.tlb.iter().map(|t| t.value.iter().filter(|v| v.is_some()).count()).sum();
    assert!(defined > 0, "{name}: no colour written");
    let discarded = a.msf.iter().filter(|&&m| m == 0).count();
    if expect.discards {
        assert!(discarded > 0 && discarded < LANES, "{name}: {discarded} of {LANES} lanes discarded");
    } else {
        assert_eq!(discarded, 0, "{name}: a lane discarded");
    }
}

macro_rules! cases {
    ($($n:ident),* $(,)?) => { $( #[test] fn $n() { check(stringify!($n)); } )* };
}

cases!(
    frag_coord_xy,
    frag_coord_zw,
    frag_coord_rcp_w,
    front_facing_colour,
    front_facing_branch_varying,
    point_coord,
    point_coord_circle,
    discard_branch,
    discard_frag_coord,
    discard_front_facing,
    frag_data0,
    frag_data0_combined,
);

#[test]
fn every_case_has_a_test() {
    let me = include_str!("shaderc71_fs_builtins.rs");
    for (n, _, _) in CASES {
        assert!(me.contains(&format!("    {n},\n")), "case {n} is not listed in cases!");
    }
}
