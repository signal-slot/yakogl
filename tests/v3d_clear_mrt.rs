// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The clear quad's fragment program for several colour attachments
//! (`src/v3d/clear_mrt.rs`), compiled by the product compiler and run on the
//! QPU interpreter: each cleared render target gets its own uniform's words
//! with its class's TLB config (`tlb_config_32b` for an integer target, the
//! f16 pairs of `tlb_config_f16` for a float one), in RT order, and a target
//! that is not cleared gets no TLB write at all, so the tile buffer keeps its
//! colour. The uniforms sit in the fragment block where
//! [`ClearMrt::uniforms`] says.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_clear_mrt
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{tlb_config_32b, tlb_config_f16, QUNIFORM_UNIFORM};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::clear_mrt::{clear_mrt_source, ClearClass, ClearMrt, ClearMrtError, CLEAR_VS, MAX_RTS};
use ClearClass::{Float, Int, NotCleared, Uint};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// A NUL-terminated source as the `&str` the front end takes.
fn text(src: &[u8]) -> &str {
    let (nul, body) = src.split_last().expect("a source");
    assert_eq!(*nul, 0, "NUL-terminated");
    assert!(!body.contains(&0), "one NUL, at the end");
    std::str::from_utf8(body).expect("ASCII")
}

/// One TLB write of lane 0: its config (`Some` for a TLBU write) and value.
type Write = (Option<u32>, u32);

/// Compile `p` with [`CLEAR_VS`], check its uniform block and RT mask
/// against what `p` says, and run it with the fragment block `block`: the
/// TLB writes of one pixel.
fn run(p: &ClearMrt, block: &[u32]) -> Vec<Write> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(text(CLEAR_VS)).expect("arena");
    let fs = a.alloc_str(text(p.source())).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs)
        .map_err(|e| e.msg.to_string())
        .unwrap_or_else(|e| panic!("the FS lowers: {e}\n{}", text(p.source())));
    /* the block as reflect.rs lays it out is the one the caller fills */
    let decl: Vec<(&str, u16, u16)> = f.module.uniforms.iter().map(|u| (u.name, u.dword_off, u.dwords)).collect();
    let want: Vec<(&str, u16, u16)> = p.uniforms().iter().map(|u| (u.name, u.dword_off, 4)).collect();
    assert_eq!(decl, want, "uniform names and dwords");
    assert_eq!(f.module.uniform_dwords, p.block_dwords());
    assert_eq!(block.len(), usize::from(p.block_dwords()), "the test fills the whole block");
    let prog = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    assert_eq!(prog.fs.rt_mask, p.rt_mask(), "the RTs the program writes");
    let uniforms: Vec<u32> = prog
        .fs
        .code
        .uniforms
        .iter()
        .map(|u| match u.contents {
            0 => u.data,
            QUNIFORM_UNIFORM => block[u.data as usize],
            c => panic!("uniform contents {c} not modelled by this test"),
        })
        .collect();
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    assert_eq!(prog.fs.num_inputs, 0, "the clear reads no varying");
    let env = Env { threads: prog.fs.threads, ..Env::default() };
    let o = sim::run(prog.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    o.tlb.iter().map(|w| (w.config, w.value[0].expect("a defined colour word"))).collect()
}

/// The f16 bits of `x`, for the exactly representable values used here.
fn half(x: f32) -> u32 {
    if x == 0.0 {
        return 0;
    }
    let b = x.to_bits();
    assert_eq!(b & 0x1fff, 0, "{x}: exact in f16");
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    assert!((1..31).contains(&exp), "{x}: a normal f16");
    (b >> 16 & 0x8000) | (exp as u32) << 10 | (b >> 13 & 0x3ff)
}

/// What one RT's clear writes to the TLB, in the order the compiler emits
/// it (tests/es3_int_rt_v3d.rs pins both shapes for one output): a float
/// target two f16 pairs (r | g << 16, b | a << 16) after its config, which
/// RT 0's vec4 leaves out (the TLB's default, `tlb_config_f16(0, 4) == !0`);
/// an integer target its config and then the four words as they are.
fn expected(rt: u32, class: ClearClass, words: [u32; 4]) -> Vec<Write> {
    match class {
        Float => {
            let h = words.map(|w| half(f32::from_bits(w)));
            let c = tlb_config_f16(rt, 4);
            let mut v = Vec::new();
            if c != !0 {
                v.push((Some(c), h[0] | h[1] << 16));
            } else {
                v.push((None, h[0] | h[1] << 16));
            }
            v.push((None, h[2] | h[3] << 16));
            v
        }
        Uint | Int => {
            vec![(Some(tlb_config_32b(rt, 4)), words[0]), (None, words[1]), (None, words[2]), (None, words[3])]
        }
        NotCleared => Vec::new(),
    }
}

/// The RT each write lands in: a TLBU config names it (bits 3..5, 7 - RT),
/// a plain write goes to the RT of the last config (RT 0 before any).
fn rts_written(w: &[Write]) -> Vec<u32> {
    let mut rt = 0;
    w.iter()
        .map(|&(c, _)| {
            if let Some(c) = c {
                rt = 7 - (c >> 3 & 7);
            }
            rt
        })
        .collect()
}

/// Clear `rts` with `colours[rt]` (the f32 bits of a float target's clear
/// colour, the raw words of an integer one) and check every TLB write.
fn check(rts: &[ClearClass], colours: &[[u32; 4]]) {
    let p = clear_mrt_source(rts).expect("a program");
    let mut block = Vec::new();
    for u in p.uniforms() {
        assert_eq!(u.dword_off as usize, block.len(), "{}: the next vec4 of the block", u.name);
        assert_eq!(u.integer, rts[u.rt as usize].is_integer());
        block.extend_from_slice(&colours[u.rt as usize]);
    }
    let got = run(&p, &block);
    let mut want = Vec::new();
    for (rt, &c) in rts.iter().enumerate() {
        want.extend(expected(rt as u32, c, colours[rt]));
    }
    assert_eq!(got, want, "{rts:?}:\n{}", text(p.source()));
    let mut seen = rts_written(&got);
    seen.dedup();
    let cleared: Vec<u32> = (0..rts.len() as u32).filter(|&rt| rts[rt as usize] != NotCleared).collect();
    assert_eq!(seen, cleared, "{rts:?}: TLB writes for the cleared RTs only, in RT order");
}

fn floats(v: [f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}

/// RT 0 float, RT 1 integer: f16 pairs with no config, then the 32-bit
/// config naming RT 1 and its four words, -1 and 0x7fffffff whole.
#[test]
fn float_then_int() {
    check(&[Float, Int], &[floats([1.0, 0.5, -2.0, 0.0]), [-1i32 as u32, 0x7fff_ffff, 3, 0x8000_0000]]);
}

/// RT 0 signed integer, RT 1 float, RT 2 unsigned integer: RT 0's 32-bit
/// config is a TLBU write, RT 1's f16 config names its RT.
#[test]
fn int_float_uint() {
    check(
        &[Int, Float, Uint],
        &[[-5i32 as u32, 6, -7i32 as u32, 8], floats([0.25, 0.75, 1.0, 0.5]), [0xffff_ffff, 0, 0x1_0002, 42]],
    );
}

/// RT 1 not cleared: no output, so no TLB write names RT 1 and the tile
/// buffer keeps its colour; RT 2's integer config names RT 2.
#[test]
fn float_kept_int() {
    check(&[Float, NotCleared, Int], &[floats([0.0, 1.0, 0.0, 1.0]), [0xdead_beef; 4], [1, 2, 3, 4]]);
    let p = clear_mrt_source(&[Float, NotCleared, Int]).unwrap();
    assert_eq!(p.rt_mask(), 0b101);
    assert!(!text(p.source()).contains("u_color1"), "no uniform for the kept RT");
}

/// Four float targets: RT 0 with the default config, RTs 1-3 each with its
/// own f16 config and its own uniform's colour.
#[test]
fn four_floats() {
    check(
        &[Float; MAX_RTS],
        &[
            floats([1.0, 0.0, 0.0, 1.0]),
            floats([0.0, 1.0, 0.0, 0.5]),
            floats([0.0, 0.0, 1.0, 0.25]),
            floats([-1.0, 2.0, 0.125, 0.0]),
        ],
    );
}

/// The kept RT first: RT 0 has no write, so RT 1's float carries its config.
#[test]
fn kept_first_then_float_and_uint() {
    check(&[NotCleared, Float, NotCleared, Uint], &[[0; 4], floats([0.5, 0.5, 0.5, 1.0]), [0; 4], [9, 8, 7, 6]]);
}

/// The signedness is the tile buffer's: a signed and an unsigned target
/// take the same program.
#[test]
fn signed_and_unsigned_share_the_source() {
    let a = clear_mrt_source(&[Float, Int, Int]).unwrap();
    let b = clear_mrt_source(&[Float, Uint, Uint]).unwrap();
    assert_eq!(a.source(), b.source());
    assert_eq!(a.uniforms(), b.uniforms());
}

/// The source of the module's doc example, and the vertex shader is the
/// back end's integer clear one.
#[test]
fn the_source_reads_as_documented() {
    let p = clear_mrt_source(&[Float, NotCleared, Int]).unwrap();
    assert_eq!(
        text(p.source()),
        "#version 300 es\nprecision highp float;\nprecision highp int;\n\
         uniform vec4 u_color0;\nuniform uvec4 u_color2;\n\
         layout(location = 0) out vec4 o0;\nlayout(location = 2) out uvec4 o2;\n\
         void main() { o0 = u_color0; o2 = u_color2; }\n"
    );
    assert_eq!(CLEAR_VS, yakogl::v3d::backend::CLEAR_VS_INT);
    /* the longest source fits */
    let p = clear_mrt_source(&[Uint; MAX_RTS]).unwrap();
    assert!(p.source().len() < 512);
    assert_eq!(p.block_dwords(), 16);
}

/// Five targets, or none cleared, give no program.
#[test]
fn refusals() {
    assert_eq!(clear_mrt_source(&[Float; 5]).err(), Some(ClearMrtError::TooManyTargets));
    assert_eq!(clear_mrt_source(&[NotCleared, NotCleared]).err(), Some(ClearMrtError::NothingCleared));
    assert_eq!(clear_mrt_source(&[]).err(), Some(ClearMrtError::NothingCleared));
}
