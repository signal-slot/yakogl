// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The clamps of select71's rule 6 at V3D 7.1, end to end: GLSL through
//! `chain71::compile_program71`, then the fragment shader's words in
//! `sim::run_gen(Gen::V71, ..)` (sim71).
//!
//! `clamp(x, 0.0, 1.0)` (and the same clamp written as `min(max(..))` or
//! `max(min(..))`), which 4.2 builds as an FMAX with the small immediate 0
//! and an FMIN with 1.0, becomes one FMOV with the SAT unpack, Mesa's
//! `fsat` (nir_to_vir.c:1863-1895); another `max(x, 0.0)` becomes an FMOV
//! with the MAX0 unpack (Mesa's `fclamp_pos`), so no FMAX or FMIN with
//! those immediates is left. Each lane's value, below 0, inside [0, 1] and
//! above 1, comes out as the clamp gives it, and a clamp whose inner result
//! is read again keeps its two operations.
//!
//! Every word round-trips through codec71 and the stage passes verify71.
//!
//! Run with `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_clamp`.
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", feature = "no-shader-table"))]

use std::sync::Mutex;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa71::{AddOp71, Instr71, MulOp71, Unpack71};
use yakogl::shaderc::online::v3d::listing71::write_instr71;
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::select::QUNIFORM_CONSTANT;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::v3d::verify71::verify71;
use yakogl::shaderc::online::{lower, store};

const VS: &str = "attribute vec4 a_pos;\nattribute vec4 a_v;\nvarying vec4 v;\n\
                  void main() {\n    gl_Position = a_pos;\n    v = a_v;\n}\n";

/// Lane `l`'s value of component `k`: -0.5 .. 1.375 in steps of 1/8, each
/// exact in f16, never -0.0 (whose clamp's sign sim71 leaves undescribed).
fn value(l: usize, k: usize) -> f32 {
    ((l + 3 * k) % LANES) as f32 / 8.0 - 0.5
}

/// What the fragment shader's words do: the instruction counts of the
/// clamp forms, the listing, and RT0's colour per lane.
struct Run {
    sat: usize,
    max0: usize,
    imm_clamps: usize,
    words: usize,
    text: String,
    colour: [[f32; 4]; LANES],
}

/// An FMAX or FMIN with the small immediate 0.0 or 1.0 (4.2's clamp halves).
fn imm_clamp(i: &Instr71) -> bool {
    let Instr71::Alu(a) = i else { return false };
    if !matches!(a.add.op, AddOp71::Fmax | AddOp71::Fmin) {
        return false;
    }
    a.read_operands().take(2).any(|(o, imm)| {
        imm && matches!(yakogl::shaderc::online::v3d::isa71::Alu71::small_imm(o), Some(0) | Some(0x3f80_0000))
    })
}

fn run(fs: &str) -> Run {
    /* the scratch region is one per process: the tests take turns */
    static SCRATCH: Mutex<()> = Mutex::new(());
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, VS), (Stage::Fragment, fs)] {
            let l = lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch"))
                .unwrap_or_else(|e| panic!("front end: {}", e.msg));
            m.push(l.module);
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let p = compile_program71(v, f, Prim::Triangles, TARGET71, a).expect("compiles at 7.1");
        let code = &p.fs.code;
        let mut text = String::new();
        for (n, i) in code.instrs.iter().enumerate() {
            text.push_str(&format!("{n:3}: "));
            write_instr71(&mut text, i).unwrap();
            text.push('\n');
        }
        for (n, (&w, i)) in code.words.iter().zip(code.instrs).enumerate() {
            assert_eq!(encode71(i), Ok(w), "word {n}\n{text}");
            assert_eq!(decode71(w).as_ref(), Ok(i), "word {n} round trip\n{text}");
        }
        assert_eq!(verify71(code.instrs, p.fs.single_seg), Ok(()), "verify71\n{text}");
        let unpacked = |u: Unpack71| {
            code.instrs
                .iter()
                .filter(|i| match i {
                    Instr71::Alu(a) => {
                        (a.add.op == AddOp71::Fmov && a.add.a.unpack == u)
                            || (a.mul.op == MulOp71::Fmov && a.mul.a.unpack == u)
                    }
                    Instr71::Branch(_) => false,
                })
                .count()
        };
        let (sat, max0) = (unpacked(Unpack71::Sat), unpacked(Unpack71::Max0));
        let imm_clamps = code.instrs.iter().filter(|i| imm_clamp(i)).count();

        /* sim71: W = 1 and C = 0, so each varying is its own value */
        let one = 1.0f32.to_bits();
        let varyings = p
            .fs
            .input_slots
            .iter()
            .map(|&(_, k)| Varying { vary: core::array::from_fn(|l| value(l, k as usize).to_bits()), c: [0; LANES] })
            .collect();
        let inputs = FsInputs {
            w: [one; LANES],
            w_centroid: [one; LANES],
            z: [0.5f32.to_bits(); LANES],
            x: [0.5f32.to_bits(); LANES],
            y: [0.5f32.to_bits(); LANES],
            back_facing: [false; LANES],
            varyings,
        };
        let stream: Vec<u32> = code
            .uniforms
            .iter()
            .map(|u| {
                assert_eq!(u.contents, QUNIFORM_CONSTANT, "a uniform other than a constant\n{text}");
                u.data
            })
            .collect();
        let env = Env { threads: p.fs.threads, ..Env::default() };
        let o = sim::run_gen(Gen::V71, code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("sim71: {e}\n{text}"));
        assert_eq!(o.tlb.len(), 2, "two colour TLB writes (r,g and b,a)\n{text}");
        let pair = |n: usize, l: usize| o.tlb[n].value[l].unwrap_or_else(|| panic!("lane {l} undefined\n{text}"));
        let h = |w: u32, hi: bool| sim::half_to_f32(if hi { (w >> 16) as u16 } else { w as u16 });
        let colour = core::array::from_fn(|l| {
            let (rg, ba) = (pair(0, l), pair(1, l));
            [h(rg, false), h(rg, true), h(ba, false), h(ba, true)]
        });
        Run { sat, max0, imm_clamps, words: code.words.len(), text, colour }
    })
    .expect("scratch")
}

fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[test]
fn clamps_to_0_1_are_one_fmov_sat_and_max0_another() {
    let fs = "precision mediump float;\nvarying vec4 v;\nvoid main() {\n    gl_FragColor = \
              vec4(clamp(v.x, 0.0, 1.0), min(max(v.y, 0.0), 1.0), max(v.z, 0.0), max(min(v.w, 1.0), 0.0));\n}\n";
    let r = run(fs);
    let text = &r.text;
    assert_eq!(r.imm_clamps, 0, "no FMAX/FMIN with 0.0 or 1.0 left\n{text}");
    assert_eq!(r.sat, 3, "three clamps to [0, 1]\n{text}");
    assert_eq!(r.max0, 1, "one max(x, 0.0)\n{text}");
    for l in 0..LANES {
        let want = [sat(value(l, 0)), sat(value(l, 1)), value(l, 2).max(0.0), sat(value(l, 3))];
        assert_eq!(r.colour[l], want, "lane {l}\n{text}");
    }
    /* four varyings at two words each, the four FMOVs paired, the two TLB
     * writes and the thread end: a moved count is a decision */
    assert_eq!(r.words, 14, "{} words\n{text}", r.words);
}

/// A clamp whose `max(x, 0.0)` is read again keeps it (as an FMOV with
/// MAX0) and its FMIN: rule 6 drops the inner half only when the outer one
/// is its only reader.
#[test]
fn a_clamp_whose_inner_result_is_read_again_keeps_both_halves() {
    let fs = "precision mediump float;\nvarying vec4 v;\nvoid main() {\n    float m = max(v.x, 0.0);\n    \
              gl_FragColor = vec4(min(m, 1.0), m * v.y, v.z, v.w);\n}\n";
    let r = run(fs);
    let text = &r.text;
    assert_eq!(r.sat, 0, "{text}");
    assert_eq!(r.max0, 1, "{text}");
    assert_eq!(r.imm_clamps, 1, "the FMIN with 1.0 stays\n{text}");
    for l in 0..LANES {
        let m = value(l, 0).max(0.0);
        let want = [m.min(1.0), m * value(l, 1), value(l, 2), value(l, 3)];
        assert_eq!(r.colour[l], want, "lane {l}\n{text}");
    }
}
