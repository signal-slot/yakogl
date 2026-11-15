// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex-stage twin of `v3d_ubo_large_sim`: does a VERTEX shader read a
//! uniform block of 16384 bytes (`vec4` x 1024 = 4096 dwords, past the 1536
//! dwords of the default-uniform budget) whole? The block is 16 members of
//! `highp vec4 m[64]` behind a default block (`uniform highp int u_i`). The
//! shader forwards entry `e` as the varying `v`, by constant index (entries
//! 0, 383, 384 and 1023; dword 4095 is the last component of entry 1023) and
//! by the uniform `u_i` within its member (a run-time index selects among
//! at most 256 components, `prepare.rs:1394`, so a member holds at most 64
//! entries). The QPU interpreter runs the vertex program with a uniform
//! stream built from a block whose dword k holds k + 0.5 (exact in f32), and
//! the four VPM output rows of the varying (the value that reaches the
//! fragment stage's input) are compared with the dwords.
//!
//! The simulator runs a vertex stage (`Inputs::Vertex`), so the host draw
//! path of `v3d_uniform_host` is not needed. Only the `contents == 1`
//! (QUNIFORM_UNIFORM) stream words are modelled; a program that needs a
//! UBO address is reported.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_large_vs
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

const FS: &str = "#version 300 es\nprecision highp float;\nin highp vec4 v;\nout highp vec4 o;\nvoid main (void)\n{\n    o = v;\n}\n";

/// The vertex shader: `k` members `m0 .. m{k-1}` of `vec4 m[len]`, and
/// `v = <index>`.
fn vs(k: usize, len: usize, index: &str) -> String {
    let members: String = (0..k).map(|m| format!("highp vec4 m{m}[{len}]; ")).collect();
    format!(
        "#version 300 es\nuniform Big {{ {members}}};\nuniform highp int u_i;\n\
         in highp vec4 a_position;\nout highp vec4 v;\n\
         void main (void)\n{{\n    gl_Position = a_position;\n    v = {index};\n}}\n"
    )
}

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The four components of `v` of lane 0 for `vs`, with `u_i` = `i`.
fn run(vs: &str, i: u32) -> Result<[f32; 4], String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(vs).expect("arena");
    let fs_ = a.alloc_str(FS).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).map_err(|e| format!("lower: {}", e.msg))?;
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let refl = &v.reflection;
    let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
    for (k, w) in block.iter_mut().enumerate() {
        *w = value(k).to_bits();
    }
    for r in refl.uniforms.iter().filter(|r| r.name == "u_i") {
        block[usize::from(r.dword_off)] = i;
    }
    let mut stream = Vec::new();
    for u in p.vs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 | 40 | 41 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!("stream dword {} past the block", u.data))?,
            2..=5 => 1.0f32.to_bits(),
            c => return Err(format!("uniform contents {c} (UBO address) not modelled by this test")),
        });
    }
    // one VPM input row per component of the attribute records
    let mut rows: Vec<[u32; LANES]> = Vec::new();
    for rec in 0..p.attributes.iter().filter(|e| e.1.is_some()).count() {
        for c in 0..p.vs.vattr_sizes[rec] as usize {
            rows.push([[0.0f32, 0.0, 0.0, 1.0][c.min(3)].to_bits(); LANES]);
        }
    }
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &stream, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .map_err(|e| format!("sim: {e}"))?;
    let out: Vec<[u32; LANES]> =
        o.vpm_out.iter().map(|r| r.ok_or("an output row is not written")).collect::<Result<_, _>>()?;
    let v = &out[out.len() - 4..];
    Ok(core::array::from_fn(|c| f32::from_bits(v[c][0])))
}

/// Entry `e` of a block of `k` members of `len` vec4, by constant or by
/// uniform index (within its member): all four components read their dwords.
fn check(k: usize, len: usize, e: usize, dynamic: bool) {
    let (m, at) = (e / len, e % len);
    let index = if dynamic { format!("m{m}[u_i]") } else { format!("m{m}[{at}]") };
    let src = vs(k, len, &index);
    let got = run(&src, at as u32).unwrap_or_else(|why| panic!("entry {e}: {why}\n{src}"));
    let want: [f32; 4] = core::array::from_fn(|c| value(e * 4 + c));
    assert_eq!(got, want, "entry {e} ({k} x vec4[{len}]), dwords {}..{}", e * 4, e * 4 + 3);
}

#[test]
fn const_index_0_first_dword() {
    check(16, 64, 0, false);
}

#[test]
fn const_index_383_last_entry_below_1536_dwords() {
    check(16, 64, 383, false);
}

#[test]
fn const_index_384_first_entry_at_dword_1536() {
    check(16, 64, 384, false);
}

#[test]
fn const_index_1023_last_dword_4095() {
    check(16, 64, 1023, false);
}

#[test]
fn dynamic_index_0_first_dword() {
    check(16, 64, 0, true);
}

#[test]
fn dynamic_index_384_first_entry_at_dword_1536() {
    check(16, 64, 384, true);
}

#[test]
fn dynamic_index_1023_last_dword_4095() {
    check(16, 64, 1023, true);
}
