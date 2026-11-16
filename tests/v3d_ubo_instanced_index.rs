// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A vertex shader indexes the `vec4` array at the end of a 16384-byte uniform
//! block with `gl_InstanceID`, the way a glDrawArraysInstanced of 64 instances
//! runs it: instance `i` reads element `i` of `vec4 inst[64]` (256
//! components, the `MAX_SELECTED` threshold of src/shaderc/online/v3d/
//! prepare.rs:1403-1408, so the run-time index still compiles), moves its
//! quad to its own region by the element's `xy` and forwards `zw` as its
//! colour. The block is pad members of 960 `vec4` and `inst` last, so it ends
//! at byte 16384 and `inst` is dwords 3840..4095.
//!
//! The QPU interpreter runs the vertex program (`Inputs::Vertex`) as the
//! driver does for an instanced draw: VPM input row 0 is `gl_InstanceID` (one
//! lane per instance, 16 lanes a run, four runs for 64 instances), the block
//! is a uniform stream of dwords holding distinct values, and every
//! instance's position offset and colour are compared with its element's
//! dwords. `vec4 inst[65]` (260 components) is refused at compile, as
//! tests/v3d_ubo_dynamic_index_limit.rs pins for the fragment stage.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_instanced_index
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

const REFUSAL: &str = "a uniform array too long to select from";

/// A block of 1024 `vec4` ending in `inst[len]`: 14 pad members of 64, one
/// pad member for the rest, then `inst`.
fn vs(len: usize) -> String {
    let pad_rest = 1024 - len - 14 * 64;
    let mut members: String = (0..14).map(|m| format!("highp vec4 p{m}[64]; ")).collect();
    members.push_str(&format!("highp vec4 q[{pad_rest}]; highp vec4 inst[{len}]; "));
    format!(
        "#version 300 es\nuniform Big {{ {members}}};\nin highp vec4 a_position;\nout highp vec4 v;\n\
         void main (void)\n{{\n    vec4 e = inst[gl_InstanceID];\n    \
         gl_Position = a_position + vec4(e.xy, 0.0, 0.0);\n    v = vec4(e.zw, 0.25, 0.75);\n}}\n"
    )
}

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// Every output row of one run of 16 instances `base..base + 16`, with
/// `a_position` = (0, 0, 0, 1).
fn run(src: &str, base: u32) -> Result<Vec<[u32; LANES]>, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(src).expect("arena");
    let fs_ = a.alloc_str(FS).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).map_err(|e| format!("lower: {}", e.msg))?;
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let block: Vec<u32> = (0..usize::from(v.reflection.uniform_dwords)).map(|k| value(k).to_bits()).collect();
    let mut stream = Vec::new();
    for u in p.vs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 | 40 | 41 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!("stream dword {} past the block", u.data))?,
            2..=5 => 1.0f32.to_bits(),
            c => return Err(format!("uniform contents {c} (UBO address) not modelled by this test")),
        });
    }
    /* row 0: gl_InstanceID, one instance per lane; then a_position */
    let mut rows: Vec<[u32; LANES]> = vec![core::array::from_fn(|l| base + l as u32)];
    for rec in 0..p.attributes.iter().filter(|e| e.1.is_some()).count() {
        for c in 0..p.vs.vattr_sizes[rec] as usize {
            rows.push([[0.0f32, 0.0, 0.0, 1.0][c.min(3)].to_bits(); LANES]);
        }
    }
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &stream, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .map_err(|e| format!("sim: {e}"))?;
    o.vpm_out.iter().map(|r| r.ok_or("an output row is not written".to_string())).collect()
}

#[test]
fn block_vec4_64_instance_id_reads_its_element() {
    let src = vs(64);
    for base in (0..64u32).step_by(LANES) {
        let out = run(&src, base).unwrap_or_else(|why| panic!("instances {base}..: {why}\n{src}"));
        let v = &out[out.len() - 4..];
        for l in 0..LANES {
            let i = (base as usize) + l;
            /* inst starts at dword 960 * 4; element i is 4 dwords */
            let d = 3840 + i * 4;
            let want_v = [value(d + 2), value(d + 3), 0.25, 0.75];
            let got_v: Vec<f32> = (0..4).map(|c| f32::from_bits(v[c][l])).collect();
            assert_eq!(got_v, want_v, "instance {i}: colour from dwords {}..{}", d + 2, d + 3);
            /* the position offset (x, y) = dwords d, d + 1: rows 0 and 1 are the
             * screen-space x and y as integers (the viewport scale is 1.0 here,
             * the value is rounded to whole pixels), then z and 1/w */
            for (c, row) in out[..2].iter().enumerate() {
                let want = value(d + c);
                let got = row[l] as f32;
                assert!(
                    (got - want).abs() <= 0.5,
                    "instance {i}: position {} is {got}, dword {} is {want}",
                    ["x", "y"][c],
                    d + c
                );
            }
        }
    }
}

#[test]
fn block_vec4_65_instance_id_is_refused() {
    let why = run(&vs(65), 0).expect_err("65 vec4 (260 components) must not compile today");
    assert!(why.starts_with("compile:") && why.contains(REFUSAL), "{why}");
}
