// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 `modf(x, out i)` in a fragment shader on the PowerVR online
//! back end: the shape of dEQP-GLES3's `builtin_functions.common.modf.*_fragment`
//! cases. The front end lowers modf to `i = trunc(x)` and `x - trunc(x)`
//! (`lower.rs` `modf`), so the fraction keeps the sign of `x`; dEQP's shader
//! executor then writes both results, as `floatBitsToUint`, to two integer
//! attachments of one layout (`o_out0` at location 0, `o_out1` at 1: R32UI
//! for `float`, RG32UI for `vec2`, RGBA32UI for `vec3` and `vec4`). The
//! integer render target variant (`shc_blend_key_int_rt`) packs each output
//! for the key's layout, output `n` into the pixel's dwords from `n * d`, `d`
//! the layout's dword count, the registers `src/pvr/backend.rs` emits
//! attachment `n` from. Two RGBA32UI outputs take eight dwords, past the
//! pixel's four output registers, and are refused by name.
//!
//! The sources follow dEQP's `glsShaderExecUtil.cpp` fragment executor and
//! `es3fShaderCommonFunctionTests.cpp` (VK-GL-CTS, Apache-2.0, Copyright (c)
//! 2014 The Android Open Source Project and The Khronos Group Inc.).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_modf
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    blend_key: u32,
    image: Vec<u8>,
}

/// One fragment shader through the chain under the variant `key`, or the
/// refusal.
fn compile_for(src: &str, key: u32) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(Rec {
            num_outputs: rec.num_outputs,
            blend_key: rec.blend_key,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
    })
    .unwrap()
}

/// The program's operations, in order: (name, the first destination's bank
/// and index).
fn ops(image: &[u8]) -> Vec<(&'static str, Option<(u32, u32)>)> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let Some(op) = ops_for(r, ph).next() else { continue };
            let d0 = g.dst.as_ref().and_then(|d| dsts(d)[0]).map(|o| (o.bank, o.index));
            out.push((op.name, d0));
        }
        off += g.total;
    }
    out
}

/// The pixel output registers a program writes, in order: the destinations
/// of the moves into the bank the last operation writes (the program ends
/// on its last pixel output write).
fn pixel_outputs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i)).collect()
}

/// dEQP's fragment executor for `modf` of `ty` at `prec`: the input from a
/// varying, both results written as their bits to integer outputs 0 and 1.
fn modf_fragment(ty: &str, prec: &str) -> String {
    let uty = match ty {
        "float" => "uint",
        "vec2" => "uvec2",
        "vec3" => "uvec3",
        _ => "uvec4",
    };
    format!(
        "#version 300 es\n\
         in {prec} {ty} vtx_out_in0;\n\
         layout(location = 0) out highp {uty} o_out0;\n\
         layout(location = 1) out highp {uty} o_out1;\n\
         void main (void)\n{{\n\
         \t{prec} {ty} in0 = vtx_out_in0;\n\
         \t{prec} {ty} out0;\n\
         \t{prec} {ty} out1;\n\
         \tout0 = modf(in0, out1);\n\
         \to_out0 = floatBitsToUint(out0);\n\
         \to_out1 = floatBitsToUint(out1);\n\
         }}\n"
    )
}

/// The integer variant of a 32-bit unsigned target of `channels` channels.
fn u32_key(channels: u8) -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels, bits: 32, signed: false })
}

/// `float` and `vec2` at every precision: both outputs packed one after the
/// other, two R32UI dwords or two RG32UI pairs, and the record says so.
#[test]
fn modf_float_and_vec2_write_both_outputs() {
    for prec in ["lowp", "mediump", "highp"] {
        for (ty, channels) in [("float", 1u8), ("vec2", 2)] {
            let key = u32_key(channels);
            let rec = compile_for(&modf_fragment(ty, prec), key).unwrap_or_else(|e| panic!("{ty} {prec}: {e}"));
            let n = 2 * channels;
            assert_eq!(rec.num_outputs, n, "{ty} {prec}: the two outputs' dwords");
            assert_eq!(rec.blend_key, key, "{ty} {prec}: the variant's key in the record");
            let outs = pixel_outputs(&rec.image);
            assert_eq!(outs.len(), usize::from(n), "{ty} {prec}: one write a dword: {outs:?}");
            let first = outs[0];
            assert_eq!(outs, (first..first + u32::from(n)).collect::<Vec<_>>(), "{ty} {prec}: consecutive dwords");
        }
    }
}

/// The whole part is `trunc(x)` and the fraction `x - trunc(x)`, so the
/// fraction keeps the sign of `x`: the modf program is, to the byte, the
/// program that writes those two expressions, through the trunc the PowerVR
/// already selects (pco's `bcsel(x < 0, ceil(x), floor(x))`).
#[test]
fn modf_is_trunc_and_the_difference() {
    for prec in ["lowp", "mediump", "highp"] {
        for (ty, channels) in [("float", 1u8), ("vec2", 2)] {
            let key = u32_key(channels);
            let modf = compile_for(&modf_fragment(ty, prec), key).expect("modf compiles");
            let spelled =
                modf_fragment(ty, prec).replace("out0 = modf(in0, out1);", "out1 = trunc(in0); out0 = in0 - out1;");
            let spelled = compile_for(&spelled, key).expect("trunc and subtract compile");
            assert_eq!(modf.image, spelled.image, "{ty} {prec}");
        }
    }
}

/// Output 1's dwords follow output 0's: two R32UI outputs are, to the byte,
/// one RG32UI output of both values, and two RG32UI outputs one RGBA32UI
/// output -- the registers the back end emits attachment 1 from.
#[test]
fn second_output_follows_the_first() {
    let fs =
        |decls: &str, body: &str| format!("#version 300 es\nflat in highp uvec4 a;\n{decls}void main() {{ {body} }}\n");
    let pairs = [
        (
            fs(
                "layout(location = 0) out highp uint o0;\nlayout(location = 1) out highp uint o1;\n",
                "o0 = a.x; o1 = a.y;",
            ),
            fs("layout(location = 0) out highp uvec2 o;\n", "o = a.xy;"),
            1u8,
        ),
        (
            fs(
                "layout(location = 0) out highp uvec2 o0;\nlayout(location = 1) out highp uvec2 o1;\n",
                "o0 = a.xy; o1 = a.zw;",
            ),
            fs("layout(location = 0) out highp uvec4 o;\n", "o = a;"),
            2,
        ),
    ];
    for (two, one, channels) in pairs {
        let two = compile_for(&two, u32_key(channels)).expect("two outputs compile");
        let one = compile_for(&one, u32_key(2 * channels)).expect("one output compiles");
        assert_eq!(one.num_outputs, two.num_outputs, "{channels}");
        assert_eq!(one.image, two.image, "{channels}");
    }
}

/// Two RGBA32UI outputs take eight dwords: refused by name, never written
/// past the pixel's four output registers.
#[test]
fn modf_vec3_and_vec4_are_refused_by_name() {
    for ty in ["vec3", "vec4"] {
        let e = compile_for(&modf_fragment(ty, "highp"), u32_key(4)).expect_err("refused");
        assert!(e.contains("integer colour outputs past the pixel's four output registers"), "{ty}: {e}");
    }
}

/// One output under the integer key is what it was: its packed dwords
/// alone, and the same program whether or not a second shader with two
/// outputs was compiled in between.
#[test]
fn one_integer_output_is_unchanged() {
    let one = "#version 300 es\nprecision highp float;\nin highp float v;\nlayout(location = 0) out highp uint o;\n\
               void main() { float w; o = floatBitsToUint(modf(v, w) + w); }\n";
    let a = compile_for(one, u32_key(1)).expect("compiles");
    assert_eq!(a.num_outputs, 1);
    assert_eq!(pixel_outputs(&a.image).len(), 1);
    compile_for(&modf_fragment("vec2", "highp"), u32_key(2)).expect("compiles");
    let b = compile_for(one, u32_key(1)).expect("compiles");
    assert_eq!(a.image, b.image);
}

/// Outputs at locations that leave a gap, or a key whose layout packs
/// narrower channels into a shared dword: the first refused by name, the
/// second packed per output (16-bit pairs, one dword an output).
#[test]
fn gaps_are_refused_and_narrow_layouts_pack_per_output() {
    let gap = "#version 300 es\nin highp float v;\nlayout(location = 0) out highp uint a;\n\
               layout(location = 2) out highp uint b;\nvoid main() { a = floatBitsToUint(v); b = 1u; }\n";
    let e = compile_for(gap, u32_key(1)).expect_err("refused");
    assert!(e.contains("other than colour outputs 0.."), "{e}");
    let rg16 = shc_blend_key_int_rt(ShcIntRt { channels: 2, bits: 16, signed: false });
    let two = "#version 300 es\nin highp vec2 v;\nlayout(location = 0) out highp uvec2 a;\n\
               layout(location = 1) out highp uvec2 b;\nvoid main() { a = floatBitsToUint(v); b = floatBitsToUint(v.yx); }\n";
    let rec = compile_for(two, rg16).expect("compiles");
    assert_eq!(rec.num_outputs, 2);
    assert_eq!(pixel_outputs(&rec.image).len(), 2);
}
