// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Bit patterns through a 16384-byte uniform block on the V3D (docs/notes/
//! ubo-limits.md section 8; cb52fae8 raised GL_MAX_UNIFORM_BLOCK_SIZE to
//! 16384): +0, -0, +inf, -inf, a quiet NaN, the smallest denormal, FLT_MAX,
//! 0x80000000 and 0xFFFFFFFF as uint and -1 as int, each placed at the first,
//! a middle and the last dword of the block (all other dwords hold a marker
//! that names the dword, so a misplaced read shows), are read by the fragment
//! stage in the QPU interpreter (the sim of `v3d_ubo_shared_stages`) and
//! written out through `gl_FragDepth`; the test compares the bits of the
//! depth write the sim recorded.
//!
//! Why depth and not an integer render target: the harness reads what the
//! shader shows the TLB, and the colour write's config word depends on the
//! target format, while the depth write (`TLB_Z_PER_PIXEL`) is one raw 32-bit
//! lane. A uint / int member is carried with `uintBitsToFloat` /
//! `intBitsToFloat`, a float member with `floatBitsToUint` and back, or
//! directly; the sim moves 32-bit words, so all forms must give the stored bits.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_bit_patterns
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

/// Dwords of the block: 8 members of `vec4[128]`.
const DWORDS: usize = 4096;

/// The special bit patterns, by name.
const PATTERNS: [(&str, u32); 10] = [
    ("+0", 0x0000_0000),
    ("-0", 0x8000_0000),
    ("+inf", 0x7f80_0000),
    ("-inf", 0xff80_0000),
    ("quiet NaN", 0x7fc1_2345),
    ("smallest denormal", 0x0000_0001),
    ("FLT_MAX", 0x7f7f_ffff),
    ("0x80000000u", 0x8000_0000),
    ("0xFFFFFFFFu", 0xffff_ffff),
    ("-1 as int", (-1i32) as u32),
];

/// First, a middle (member 4, entry 0) and the last dword.
const POSITIONS: [usize; 3] = [0, 2048, DWORDS - 1];

/// What the block holds at dword `k` when no pattern is there.
fn marker(k: usize) -> u32 {
    0xa5a5_0000 | k as u32
}

/// The block's words: markers, and `bits` at dword `at`.
fn block(at: usize, bits: u32) -> Vec<u32> {
    (0..DWORDS).map(|k| if k == at { bits } else { marker(k) }).collect()
}

/// The fragment shader: block `Big` of 8 x `ty`[128]; `gl_FragDepth` is `expr`
/// with `X` the member read at dword `at`.
fn fs(ty: &str, at: usize, expr: &str) -> String {
    let members: String = (0..8).map(|m| format!("highp {ty} m{m}[128]; ")).collect();
    let (m, e, c) = (at / 512, (at / 4) % 128, ["x", "y", "z", "w"][at % 4]);
    let x = format!("m{m}[{e}].{c}");
    format!(
        "#version 300 es\nuniform Big {{ {members} }};\nin highp vec4 v;\n\
         layout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = v;\n    gl_FragDepth = {};\n}}\n",
        expr.replace('X', &x)
    )
}

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nout highp vec4 v;\n\
                  void main (void)\n{\n    gl_Position = a_position;\n    v = vec4(0.0);\n}\n";

/// The bits of the fragment stage's depth write of lane 0.
fn run(fs_src: &str, words: &[u32]) -> Result<u32, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs_src).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).map_err(|e| format!("lower vs: {}", e.msg))?;
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower fs: {}", e.msg))?;
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let dwords = usize::from(f.reflection.uniform_dwords);
    let stream: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match (u.contents as u8, u.data) {
                (0 | 40 | 41, d) => Ok(d),
                (1, d) if (d as usize) < dwords => Ok(words[d as usize]),
                (1, d) => Err(format!("stream dword {d} past the block")),
                (2..=5, _) => Ok(1.0f32.to_bits()),
                (c, _) => Err(format!("uniform contents {c} (UBO address) not modelled by this test")),
            })
            .collect::<Result<_, _>>()?;
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("fs sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    w.value[0].ok_or_else(|| "undefined depth".to_string())
}

/// Every pattern at every position, read as `ty` through `expr`.
fn sweep(ty: &str, expr: &str) {
    for at in POSITIONS {
        let src = fs(ty, at, expr);
        for (name, bits) in PATTERNS {
            let got = run(&src, &block(at, bits)).unwrap_or_else(|e| panic!("{name} at dword {at}: {e}"));
            assert_eq!(got, bits, "{ty} {expr}: {name} at dword {at}: got {got:#010x}, want {bits:#010x}");
        }
    }
}

#[test]
fn uint_members_keep_every_bit() {
    sweep("uvec4", "uintBitsToFloat(X)");
}

#[test]
fn float_members_keep_every_bit() {
    sweep("vec4", "X");
}

#[test]
fn float_members_through_floatbitstouint_keep_every_bit() {
    sweep("vec4", "uintBitsToFloat(floatBitsToUint(X))");
}

#[test]
fn int_members_keep_every_bit() {
    sweep("ivec4", "intBitsToFloat(X)");
}

/// The neighbours of a pattern are untouched: reading the dword next to the
/// pattern gives its marker (the pattern is not smeared over the vec4).
#[test]
fn neighbours_of_a_pattern_keep_their_markers() {
    for at in [1usize, 2049, DWORDS - 2] {
        let src = fs("uvec4", at, "uintBitsToFloat(X)");
        let got = run(&src, &block(at + 1, 0x7fc1_2345)).unwrap_or_else(|e| panic!("dword {at}: {e}"));
        assert_eq!(got, marker(at), "dword {at}");
    }
}
