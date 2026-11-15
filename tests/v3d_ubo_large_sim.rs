// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D compiler reading a uniform block past dword 1536 (6144 bytes, what
//! `MAX_UNIFORM_BLOCK_SIZE` used to be) up to ES 3.0's 16384. A fragment
//! shader reads `vec4 v[1024]` (4096 dwords) of a std140 block, by constant
//! index (entries 0, 383, 384, 1023; dwords 1532..1535 are entry 383, 1536..
//! is entry 384) and by an index held in a uniform, and the QPU interpreter
//! runs it with a uniform stream built from a block whose dword k holds a
//! distinct value (k + 0.5, exact in f32). The value read reaches the TLB as
//! `gl_FragDepth` (unclamped, f32).
//!
//! This is the compiler-side half of the raise `src/gl/context.rs`
//! MAX_UNIFORM_BLOCK_SIZE makes for the GPU: a block member of any dword index
//! is a `QUNIFORM_UNIFORM` word of the stage's storage that the draw fills, so
//! a block of 16384 bytes links, draws and reads (the GL-side half is
//! `tests/es3_v3d_ubo_size.rs`). What the raise does not buy is the array
//! indexing the `#[ignore]`d cases name: `Op::LoadUniformIdx` selects among at
//! most 256 components (no `ldunifa` yet) and `UniformRow::array_len` is a u8.
//!
//! Only the `contents == 1` (QUNIFORM_UNIFORM) stream words are modelled; a
//! program that needs UNIFA/QUNIFORM_UBO_ADDR is reported.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_large_sim
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

/// A block of `k` members `m0 .. m{k-1}`, each `vec4 m[len]` (`len` is at
/// most 255: `UniformRow::array_len` is a u8), and `gl_FragDepth =
/// <index>.<comp>`.
fn fs(k: usize, len: usize, index: &str, comp: char) -> String {
    let members: String = (0..k).map(|m| format!("highp vec4 m{m}[{len}]; ")).collect();
    format!(
        "#version 300 es\nuniform Big {{ {members}}};\nuniform highp int u_i;\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {index}.{comp};\n}}\n"
    )
}

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

/// Every component of entry `e` of a block of `k` members of `len` vec4
/// reads its dword, by constant or by uniform index (within its member).
fn check(k: usize, len: usize, e: usize, dynamic: bool) {
    let (m, at) = (e / len, e % len);
    for (c, comp) in "xyzw".chars().enumerate() {
        let index = if dynamic { format!("m{m}[u_i]") } else { format!("m{m}[{at}]") };
        let src = fs(k, len, &index, comp);
        let got = run(&src, at as u32).unwrap_or_else(|why| panic!("entry {e}.{comp}: {why}\n{src}"));
        assert_eq!(got, value(e * 4 + c), "entry {e}.{comp} ({k} x vec4[{len}]), dword {}", e * 4 + c);
    }
}

// 1024 vec4 = 16384 bytes as 8 members of 128 (constant) or 16 of 64 (dynamic).

#[test]
fn const_index_0() {
    check(8, 128, 0, false);
}

#[test]
fn const_index_383_last_entry_below_1536_dwords() {
    check(8, 128, 383, false);
}

#[test]
fn const_index_384_first_entry_at_dword_1536() {
    check(8, 128, 384, false);
}

#[test]
fn const_index_1023_last_entry_of_16384_bytes() {
    check(8, 128, 1023, false);
}

#[test]
fn dynamic_index_0() {
    check(16, 64, 0, true);
}

#[test]
fn dynamic_index_383() {
    check(16, 64, 383, true);
}

#[test]
fn dynamic_index_384() {
    check(16, 64, 384, true);
}

#[test]
fn dynamic_index_1023() {
    check(16, 64, 1023, true);
}

/// One array of 65 vec4 (260 dwords) by a uniform index: `Op::LoadUniformIdx`
/// selects among at most 256 components (64 vec4 entries); 64 passes above.
#[test]
#[ignore = "src/shaderc/online/v3d/prepare.rs:1396: LoadUniformIdx selects among at most MAX_SELECTED = 256 components (no ldunifa yet), so a dynamically indexed vec4 array is at most 64 entries"]
fn dynamic_index_in_one_65_entry_array() {
    check(1, 65, 64, true);
}

/// One array of 255 vec4 (1020 dwords): the same limit, at the longest array
/// the reflection can hold.
#[test]
#[ignore = "src/shaderc/online/v3d/prepare.rs:1396: LoadUniformIdx selects among at most MAX_SELECTED = 256 components (no ldunifa yet)"]
fn dynamic_index_in_one_255_entry_array() {
    check(1, 255, 254, true);
}

/// One array of 256 vec4 (the block of 1024 vec4 as 4 x 256).
#[test]
#[ignore = "src/shaderc/online/reflect.rs:813: UniformRow::array_len is a u8 (fit_u8), so an array of 256 elements is rejected by the front end"]
fn const_index_in_one_256_entry_array() {
    check(1, 256, 255, false);
}
