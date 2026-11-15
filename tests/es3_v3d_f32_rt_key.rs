// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 compiler's f32 render-target key (plan #360, item
//! v3d-f32-key): `compile_program_rt`'s `rt_f32`, bit n naming render
//! target n as a 32-bit float target (R32F, RG32F, RGBA32F). A float output
//! to such a target is written to the TLB as one 32-bit word per declared
//! component with the 32-bit config (`tlb_config_32b`), as an integer
//! output is, in place of two f16 pairs: Mesa's `f32_color_rb` key
//! (v3d_compile.h `struct v3d_fs_key`), vir_emit_tlb_color_write's 32-bit
//! branch (nir_to_vir.c:1974-1977, 2005-2021).
//!
//! Nothing sets the key yet (item v3d-arm will). With the key off the
//! program is the one the compiler made before the key existed: the
//! fingerprints in [`PINNED`] were taken with `compile_program_keyed` at
//! 069dd17b, before this change.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_f32_rt_key
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, compile_program_rt, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::isa::{AddOp, Instr};
use yakogl::shaderc::online::v3d::select::{tlb_config_32b, tlb_config_f16};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::LodKey;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// The Raspberry Pi 4's VPM.
const TARGET: Target = Target { vpm_size: 65536 };

/// The magic write addresses of the TLB (`V3D_QPU_WADDR_TLB`, `_TLBU`).
const WADDR_TLB: u8 = 7;
const WADDR_TLBU: u8 = 8;

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const VS_C: &str = "#version 300 es\nin vec4 p;\nout vec4 c;\nvoid main() { c = p * 0.5; gl_Position = p; }\n";
const VS_100: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";

/// `vs` + `fs` compiled with the render-target key `rt_f32` (`None`: through
/// `compile_program_keyed`, the entry point without the key), handed to `f`.
fn with_program<R>(vs: &str, fs: &str, rt_f32: Option<u8>, f: impl FnOnce(&V3dProgram<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = match rt_f32 {
        None => compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, LodKey::NONE, LodKey::NONE, &a),
        Some(k) => {
            compile_program_rt(&v.module, &fr.module, Prim::Triangles, TARGET, LodKey::NONE, LodKey::NONE, k, &a)
        }
    }
    .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    f(&p)
}

/* ---- the key off: today's code ------------------------------------------ */

/// FNV-1a over the program's three code streams and uniform streams.
fn fingerprint(p: &V3dProgram<'_>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |x: u64| {
        for b in x.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for c in [&p.fs.code, &p.vs.code, &p.cs.code] {
        eat(c.words.len() as u64);
        for &w in c.words {
            eat(w);
        }
        eat(c.uniforms.len() as u64);
        for u in c.uniforms {
            eat((u64::from(u.contents) << 32) | u64::from(u.data));
        }
    }
    h
}

/// Float, integer and mixed outputs, every width, one to four RTs, a
/// varying, a discard, ES 1.00's gl_FragColor and gl_FragData.
const SHADERS: [(&str, &str, &str); 10] = [
    (
        "vec4",
        VS,
        "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = vec4(1.0, 0.5, -2.0, 0.0); }\n",
    ),
    ("vec3", VS, "#version 300 es\nprecision highp float;\nout vec3 o;\nvoid main() { o = vec3(1.0, 0.5, -2.0); }\n"),
    ("vec2", VS, "#version 300 es\nprecision highp float;\nout vec2 o;\nvoid main() { o = vec2(1.0, 0.5); }\n"),
    ("float", VS, "#version 300 es\nprecision highp float;\nout float o;\nvoid main() { o = 3.0; }\n"),
    ("varying", VS_C, "#version 300 es\nprecision highp float;\nin vec4 c;\nout vec4 o;\nvoid main() { o = c; }\n"),
    (
        "discard",
        VS_C,
        "#version 300 es\nprecision highp float;\nin vec4 c;\nout vec4 o;\n\
         void main() { if (c.x > 0.25) discard; o = c.yxwz; }\n",
    ),
    (
        "mrt4",
        VS,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 a;\nlayout(location = 1) out vec2 b;\n\
         layout(location = 2) out float c;\nlayout(location = 3) out vec4 d;\n\
         void main() { a = vec4(1.0); b = vec2(0.25, 2.0); c = -1.0; d = vec4(0.5, 0.0, 1.0, 4.0); }\n",
    ),
    (
        "int+float",
        VS,
        "#version 300 es\nprecision highp int;\nprecision highp float;\n\
         layout(location = 0) out vec4 c;\nlayout(location = 1) out uvec4 u;\n\
         void main() { c = vec4(1.0); u = uvec4(10u, 20u, 30u, 40u); }\n",
    ),
    ("fragcolor", VS_100, "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.0, 0.25, 0.5, 1.0); }\n"),
    ("fragdata", VS_100, "precision mediump float;\nvoid main() { gl_FragData[0] = vec4(0.5, 1.0, 0.0, 0.75); }\n"),
];

/// Each shader's [`fingerprint`] with `compile_program_keyed` at 069dd17b,
/// before the key existed.
const PINNED: [(&str, u64); 10] = [
    /* vec3 is the same program as vec4: a float output writes all four lanes, the one not
     * declared as 0 (prepare_fragment_with) */
    ("vec4", 0x2b14369d47d323d0),
    ("vec3", 0x2b14369d47d323d0),
    ("vec2", 0x37c9653e8a4deb83),
    ("float", 0x69f126a18fc4485b),
    ("varying", 0x4e8b560d01a8220e),
    ("discard", 0xf2943629a8d2e17d),
    ("mrt4", 0x08a2e5074b329ee1),
    ("int+float", 0x91a14c17643402ee),
    ("fragcolor", 0xcecf7c5ee58accf6),
    ("fragdata", 0x395d136e1c5adc7d),
];

/// With the key off, through either entry point, every program is the one
/// the compiler made before the key existed, word for word.
#[test]
fn key_off_code_is_unchanged() {
    for ((name, vs, fs), (pname, pinned)) in SHADERS.iter().zip(PINNED) {
        assert_eq!(*name, pname);
        let keyed = with_program(vs, fs, None, fingerprint);
        let rt0 = with_program(vs, fs, Some(0), fingerprint);
        assert_eq!(keyed, rt0, "{name}: compile_program_rt with 0 is compile_program_keyed");
        assert_eq!(keyed, pinned, "{name}: {keyed:#018x}, pinned {pinned:#018x}");
        let (k, r) = (with_program(vs, fs, None, |p| p.fs.rt_f32), with_program(vs, fs, Some(0), |p| p.fs.rt_f32));
        assert_eq!((k, r), (0, 0), "{name}: FsProg::rt_f32");
    }
}

/// A key bit for a render target the shader does not write changes nothing.
#[test]
fn key_of_an_unwritten_target_changes_nothing() {
    for (name, vs, fs) in SHADERS.iter().filter(|s| s.0 != "mrt4" && s.0 != "int+float" && s.0 != "fragdata") {
        let off = with_program(vs, fs, Some(0), fingerprint);
        assert_eq!(with_program(vs, fs, Some(0b1110), fingerprint), off, "{name}: RTs 1..3 keyed, RT 0 written");
    }
}

/* ---- the key on: the TLB writes, instruction by instruction ------------- */

/// The fragment shader's TLB writes, in program order: (written through
/// TLBU, the ALU that writes it, is it a VFPACK).
fn tlb_writes(p: &V3dProgram<'_>) -> Vec<(bool, &'static str, bool)> {
    let mut v = Vec::new();
    for i in p.fs.code.instrs {
        let Instr::Alu(alu) = i else { continue };
        if alu.add.magic && (alu.add.waddr == WADDR_TLB || alu.add.waddr == WADDR_TLBU) {
            let vfpack = alu.add.op == AddOp::Vfpack;
            v.push((alu.add.waddr == WADDR_TLBU, "add", vfpack));
        }
        if alu.mul.magic && (alu.mul.waddr == WADDR_TLB || alu.mul.waddr == WADDR_TLBU) {
            v.push((alu.mul.waddr == WADDR_TLBU, "mul", false));
        }
    }
    v
}

/// What the fragment shader wrote to the TLB for one pixel (lane 0), on the
/// QPU interpreter: each write's config (`Some` for a TLBU write) and value.
fn run_fs(p: &V3dProgram<'_>) -> Vec<(Option<u32>, u32)> {
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    assert_eq!(p.fs.num_inputs, 0, "the shaders run here read no varying");
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    o.tlb.iter().map(|w| (w.config, w.value[0].expect("a defined colour word"))).collect()
}

fn f(x: f32) -> u32 {
    x.to_bits()
}

/// Off, a vec4 output to RT 0 is two VFPACKs to TLB with no config (the
/// TLB's default); keyed, it is four MOVs (the mul ALU), the first to TLBU
/// with the 32-bit config for RT 0, vec4, and the words are the floats as
/// they are: 1/3 and 1e10, which no f16 holds, arrive whole.
#[test]
fn a_keyed_vec4_writes_four_32bit_words() {
    let fs = "#version 300 es\nprecision highp float;\nout vec4 o;\n\
              void main() { o = vec4(0.333333343, 1.0e10, -2.0, 0.0); }\n";
    with_program(VS, fs, Some(0), |p| {
        assert_eq!(tlb_writes(p), [(false, "add", true), (false, "add", true)], "off: two f16 pairs");
        assert_eq!(tlb_config_f16(0, 4), !0);
    });
    with_program(VS, fs, Some(1), |p| {
        assert_eq!(p.fs.rt_f32, 1);
        assert_eq!(
            tlb_writes(p),
            [(true, "mul", false), (false, "mul", false), (false, "mul", false), (false, "mul", false)],
            "keyed: four 32-bit words, the first through TLBU"
        );
        assert_eq!(tlb_config_32b(0, 4), 0xffff_ff3f);
        assert_eq!(
            run_fs(p),
            [(Some(0xffff_ff3f), f(0.333_333_34)), (None, f(1.0e10)), (None, f(-2.0)), (None, 0)],
            "config | r, g, b, a"
        );
    });
}

/// A keyed vec3, vec2 and float write as many words as they have
/// components, the config's vector size following (size - 1 in bits 0..1).
#[test]
fn fewer_components_write_fewer_words() {
    let cases: [(&str, &[u32]); 3] = [
        ("vec3 o;\nvoid main() { o = vec3(1.5, 1.0e-10, 7.0); }", &[f(1.5), f(1.0e-10), f(7.0)]),
        ("vec2 o;\nvoid main() { o = vec2(-0.1, 65536.0); }", &[f(-0.1), f(65536.0)]),
        ("float o;\nvoid main() { o = 100000.0; }", &[f(100000.0)]),
    ];
    for (decl, want) in cases {
        let fs = format!("#version 300 es\nprecision highp float;\nout {decl}\n");
        let n = want.len();
        with_program(VS, &fs, Some(1), |p| {
            let w = tlb_writes(p);
            assert_eq!(w.len(), n, "{decl}: {w:?}");
            assert!(w.iter().all(|e| e.1 == "mul" && !e.2), "{decl}: MOVs, no VFPACK: {w:?}");
            assert!(w[0].0 && w[1..].iter().all(|e| !e.0), "{decl}: only the first through TLBU: {w:?}");
            let mut expect = vec![(Some(tlb_config_32b(0, n as u32)), want[0])];
            expect.extend(want[1..].iter().map(|&x| (None, x)));
            assert_eq!(run_fs(p), expect, "{decl}");
        });
    }
    assert_eq!(
        [tlb_config_32b(0, 3), tlb_config_32b(0, 2), tlb_config_32b(0, 1)],
        [0xffff_ff3e, 0xffff_ff3d, 0xffff_ff3c]
    );
}

/// The key is per render target: with RT 1 keyed, RT 0's vec4 is still two
/// f16 pairs, RT 1's four 32-bit words with the config naming RT 1 (7 - 1
/// in bits 3..5).
#[test]
fn the_key_is_per_render_target() {
    let fs = "#version 300 es\nprecision highp float;\n\
              layout(location = 0) out vec4 a;\nlayout(location = 1) out vec4 b;\n\
              void main() { a = vec4(1.0); b = vec4(10.0, 20.0, 30.0, 1.0e6); }\n";
    with_program(VS, fs, Some(0b10), |p| {
        assert_eq!(tlb_config_32b(1, 4), 0xffff_ff37);
        assert_eq!(
            run_fs(p),
            [
                (None, 0x3c00_3c00),
                (None, 0x3c00_3c00),
                (Some(0xffff_ff37), f(10.0)),
                (None, f(20.0)),
                (None, f(30.0)),
                (None, f(1.0e6)),
            ]
        );
    });
    with_program(VS, fs, Some(0b01), |p| {
        let w = run_fs(p);
        assert_eq!(w[..4], [(Some(0xffff_ff3f), f(1.0)), (None, f(1.0)), (None, f(1.0)), (None, f(1.0))]);
        assert_eq!(w.len(), 6, "RT 1 as two f16 pairs: {w:?}");
        assert_eq!(w[4].0, Some(tlb_config_f16(1, 4)), "RT 1's f16 config through TLBU");
    });
}

/// ES 1.00's gl_FragColor is a vec4: keyed, four words.
#[test]
fn a_keyed_gl_fragcolor_writes_four_words() {
    let fs = "precision highp float;\nvoid main() { gl_FragColor = vec4(0.0, 0.25, 0.5, 3.0e5); }\n";
    with_program(VS_100, fs, Some(1), |p| {
        assert_eq!(run_fs(p), [(Some(0xffff_ff3f), 0), (None, f(0.25)), (None, f(0.5)), (None, f(3.0e5))]);
    });
}

/// An integer output is written as 32-bit words already: the key changes
/// nothing for it.
#[test]
fn an_integer_output_is_the_same_keyed_or_not() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout ivec4 o;\n\
              void main() { o = ivec4(1, -2, 3, 0x7fffffff); }\n";
    let off = with_program(VS, fs, Some(0), fingerprint);
    assert_eq!(with_program(VS, fs, Some(1), fingerprint), off);
}
