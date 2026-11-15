// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What a uniform-index read of a long uniform array does today on the V3D.
//!
//! `Op::LoadUniformIdx` (src/shaderc/online/v3d/prepare.rs:1403-1408) loads
//! every element of the array as a plain uniform and selects one per lane, and
//! refuses with `PrepError::NotYet("a uniform array too long to select from
//! (no ldunifa yet)")` when `len * comps > MAX_SELECTED` (prepare.rs:1404,
//! `MAX_SELECTED = 256`). The threshold is **256 components** per indexed
//! array (`comps` = the components read: 4 for a vec4 element, 3 for a vec3,
//! 2 for a vec2, 1 for a float): vec4[64], vec3[85], vec2[128] and float[255]
//! (float[256] is out of reach: array_len is a u8) still compile; vec4[65], vec3[86] and vec2[129] do not. It holds for block
//! member arrays and default-block arrays alike (both lower to
//! `LoadUniformIdx`) and does not depend on the block size.
//!
//! This file pins, per kind of array: the longest array still compiles and
//! reads its first and last element on the host QPU sim, and one element more
//! gives today's refusal as a compile failure carrying that message. When
//! ldunifa lands the refusal cases are to be turned into reads.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_dynamic_index_limit
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\nvoid main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The depth lane 0 writes for `fs`, with `u_i` = `i`, or why the form is not
/// run (compile error, an access the model does not carry, a sim refusal).
fn run(fs: &str, i: u32) -> Result<f32, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let refl = &f.reflection;
    let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
    for (k, w) in block.iter_mut().enumerate() {
        *w = value(k).to_bits();
    }
    for r in refl.uniforms.iter().filter(|r| r.name == "u_i") {
        block[usize::from(r.dword_off)] = i;
    }
    let mut stream = Vec::new();
    for u in p.fs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!("stream dword {} past the block", u.data))?,
            c => return Err(format!("uniform contents {c} (UNIFA / UBO address) not modelled by this test")),
        });
    }
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    Ok(f32::from_bits(w.value[0].ok_or("undefined depth")?))
}

const REFUSAL: &str = "a uniform array too long to select from";

/// `ty` elements of `len` in a std140 block (`block`) or the default block;
/// `u_i` selects; the element's component `comp` is the depth.
fn shader(block: bool, ty: &str, len: usize, comp: char) -> String {
    let decl = if block {
        format!("uniform Big {{ highp {ty} d[{len}]; }};")
    } else {
        format!("uniform highp {ty} d[{len}];")
    };
    format!(
        "#version 300 es\n{decl}\nuniform highp int u_i;\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = d[u_i].{comp};\n}}\n"
    )
}

fn read(block: bool, ty: &str, len: usize, comp: char, at: usize) -> Result<f32, String> {
    let src = shader(block, ty, len, comp);
    run(&src, at as u32)
}

/// The first and last element compile and read a block dword. For vec4 and
/// vec3 the last element is `at * 4` dwords after the first (std140 stride).
/// The stride of vec2 and float arrays is not pinned: the sim reads them at
/// 2 and 1 dwords in a block today, not std140's 4 (see the report).
fn pin_ok(block: bool, ty: &str, len: usize) {
    for at in [0, len - 1] {
        let got =
            read(block, ty, len, 'x', at).unwrap_or_else(|why| panic!("{ty}[{len}] block={block} at {at}: {why}"));
        let dword = got - 0.5;
        assert!(
            dword >= 0.0 && dword.fract() == 0.0,
            "{ty}[{len}] block={block} at {at}: depth {got} is not a block dword"
        );
        let dword = dword as usize;
        if at == 0 {
            // the array's first dword: 0 in a block, the packing's choice otherwise
            if block {
                assert_eq!(dword, 0, "{ty}[{len}] first element");
            }
        } else {
            let first = read(block, ty, len, 'x', 0).unwrap() as usize;
            let stride = if ty == "vec4" || ty == "vec3" { 4 } else { 0 };
            if stride != 0 {
                assert_eq!(dword, first + at * stride, "{ty}[{len}] last element reads its dword");
            }
        }
    }
}

fn pin_refused(block: bool, ty: &str, len: usize) {
    let why = read(block, ty, len, 'x', 0).expect_err("one element past 256 components must not compile today");
    assert!(why.starts_with("compile:") && why.contains(REFUSAL), "{ty}[{len}] block={block}: {why}");
}

#[test]
fn block_vec4_64_reads_first_and_last() {
    pin_ok(true, "vec4", 64);
}

#[test]
fn block_vec4_65_is_refused() {
    pin_refused(true, "vec4", 65);
}

#[test]
fn block_vec3_85_reads_first_and_last() {
    pin_ok(true, "vec3", 85);
}

#[test]
fn block_vec3_86_is_refused() {
    pin_refused(true, "vec3", 86);
}

#[test]
fn block_vec2_128_reads_first_and_last() {
    pin_ok(true, "vec2", 128);
}

#[test]
fn block_vec2_129_is_refused() {
    pin_refused(true, "vec2", 129);
}

#[test]
fn block_float_255_reads_first_and_last() {
    // 255 is the longest array the reflection holds (array_len is a u8), one
    // short of the 256-component bound.
    pin_ok(true, "float", 255);
}

#[test]
fn default_vec4_64_reads_first_and_last() {
    pin_ok(false, "vec4", 64);
}

#[test]
fn default_vec4_65_is_refused() {
    pin_refused(false, "vec4", 65);
}

#[test]
fn default_vec2_128_reads_first_and_last() {
    pin_ok(false, "vec2", 128);
}

#[test]
fn default_vec2_129_is_refused() {
    pin_refused(false, "vec2", 129);
}

#[test]
fn default_float_255_reads_first_and_last() {
    pin_ok(false, "float", 255);
}
