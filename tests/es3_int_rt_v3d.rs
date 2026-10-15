// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2's half of ES 3.0's integer and float colour render targets
//! (int-rt-v3d), on the host: the TLB render-target rows the back end hands
//! the job builder for every colour surface (`src/v3d/state.rs`
//! `rt_format`, Mesa's `v3dx_format_table.c` and `v3dx_rcl.c`), the clear
//! words the tile buffer starts from (`clear_words`, `v3d_tlb_clear`), the
//! surfaces the GL core may draw into (`backend::INTEGER_RT_FORMATS`), and
//! the compiler's TLB writes of an integer fragment output: 32-bit words
//! with Mesa's 32-bit config (`select.rs` `tlb_config_32b`,
//! vir_emit_tlb_color_write's 32-bit branch), run on the QPU interpreter.
//! The draw path (the typed job, its strides, clears and refusals) is in
//! `src/v3d/backend_tests.rs`, which drives the back end against its fake
//! job builder.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_rt_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::backend::{self, *};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{tlb_config_32b, tlb_config_f16};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::ffi::*;
use yakogl::v3d::state;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/* ---- the render-target rows --------------------------------------------- */

/// Every surface of ES 3.0 table 3.13's integer rows, as the GL core numbers
/// them (`GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST`).
fn integer_surfaces() -> impl Iterator<Item = u8> {
    GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST
}

/// The GL core may draw every integer surface into the V3D
/// (`INTEGER_RT_FORMATS` has all nineteen bits), and not the sRGB one
/// (`SRGB_RT`, docs/es3/srgb.md 5.1).
#[test]
fn every_integer_surface_is_drawable() {
    assert_eq!(backend::INTEGER_RT_FORMATS, (1 << 19) - 1);
    for s in integer_surfaces() {
        assert!(backend::rt_format_drawable(s), "{}", gles_surf_name(s));
        assert!(state::rt_format(s).is_some(), "{}: a TLB row", gles_surf_name(s));
    }
    assert!(!backend::rt_format_drawable(GLES_SURF_SRGB8_A8));
    assert_eq!(state::rt_format(GLES_SURF_SRGB8_A8), None);
}

/// Each row is Mesa's (v3dx_format_table.c `FORMAT(...)` rt column, the
/// internal type and bpp of `v3dX(get_internal_type_bpp_for_output_format)`
/// :220-341, the INT clamp of `clamp_for_format_and_type` v3dx_rcl.c:396-407):
/// the output image format of the format's own name, the internal type of
/// its channel width and signedness, the bpp from the channels' total width
/// (RGB10_A2UI held as 16UI), the INT clamp, and the GL core's bytes a pixel.
#[test]
fn integer_rows_follow_mesas_format_table() {
    /* (surface, output image format, internal type) */
    let rows: [(u8, u32, u32); 19] = [
        (GLES_SURF_R8I, V3D_OUTPUT_IMAGE_FORMAT_R8I, V3D_INTERNAL_TYPE_8I),
        (GLES_SURF_R8UI, V3D_OUTPUT_IMAGE_FORMAT_R8UI, V3D_INTERNAL_TYPE_8UI),
        (GLES_SURF_R16I, V3D_OUTPUT_IMAGE_FORMAT_R16I, V3D_INTERNAL_TYPE_16I),
        (GLES_SURF_R16UI, V3D_OUTPUT_IMAGE_FORMAT_R16UI, V3D_INTERNAL_TYPE_16UI),
        (GLES_SURF_R32I, V3D_OUTPUT_IMAGE_FORMAT_R32I, V3D_INTERNAL_TYPE_32I),
        (GLES_SURF_R32UI, V3D_OUTPUT_IMAGE_FORMAT_R32UI, V3D_INTERNAL_TYPE_32UI),
        (GLES_SURF_RG8I, V3D_OUTPUT_IMAGE_FORMAT_RG8I, V3D_INTERNAL_TYPE_8I),
        (GLES_SURF_RG8UI, V3D_OUTPUT_IMAGE_FORMAT_RG8UI, V3D_INTERNAL_TYPE_8UI),
        (GLES_SURF_RG16I, V3D_OUTPUT_IMAGE_FORMAT_RG16I, V3D_INTERNAL_TYPE_16I),
        (GLES_SURF_RG16UI, V3D_OUTPUT_IMAGE_FORMAT_RG16UI, V3D_INTERNAL_TYPE_16UI),
        (GLES_SURF_RG32I, V3D_OUTPUT_IMAGE_FORMAT_RG32I, V3D_INTERNAL_TYPE_32I),
        (GLES_SURF_RG32UI, V3D_OUTPUT_IMAGE_FORMAT_RG32UI, V3D_INTERNAL_TYPE_32UI),
        (GLES_SURF_RGBA8I, V3D_OUTPUT_IMAGE_FORMAT_RGBA8I, V3D_INTERNAL_TYPE_8I),
        (GLES_SURF_RGBA8UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI, V3D_INTERNAL_TYPE_8UI),
        (GLES_SURF_RGBA16I, V3D_OUTPUT_IMAGE_FORMAT_RGBA16I, V3D_INTERNAL_TYPE_16I),
        (GLES_SURF_RGBA16UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA16UI, V3D_INTERNAL_TYPE_16UI),
        (GLES_SURF_RGBA32I, V3D_OUTPUT_IMAGE_FORMAT_RGBA32I, V3D_INTERNAL_TYPE_32I),
        (GLES_SURF_RGBA32UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI, V3D_INTERNAL_TYPE_32UI),
        (GLES_SURF_RGB10_A2UI, V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2UI, V3D_INTERNAL_TYPE_16UI),
    ];
    for (surf, output, ty) in rows {
        let name = gles_surf_name(surf);
        let r = state::rt_format(surf).expect(name);
        let l = gles_surf_int_layout(surf).expect(name);
        assert_eq!((r.output, r.internal_type), (output, ty), "{name}");
        assert_eq!(r.clamp, V3D_RENDER_TARGET_CLAMP_INT, "{name}: clamp to the target's range");
        assert_eq!(r.bytes, gles_surf_bytes_per_pixel(surf), "{name}: bytes a pixel");
        /* the tile buffer's width: the channels at their width (16 for the
         * 10-bit ones), rounded up to 32, 64 or 128 bits */
        let per = if l.bits == 10 { 16 } else { u32::from(l.bits) };
        let held = u32::from(l.channels) * per;
        let bpp = if held <= 32 {
            V3D_INTERNAL_BPP_32
        } else if held <= 64 {
            V3D_INTERNAL_BPP_64
        } else {
            V3D_INTERNAL_BPP_128
        };
        assert_eq!(r.internal_bpp, bpp, "{name}: internal bpp for {held} bits held");
        assert!(state::typed(surf), "{name}: a typed job");
    }
}

/// The float rows: RGBA16F and R16F are 16F at 64 bpp (R16F too, Mesa's
/// v3dx_format_table.c:263-270), R32F is 32F at 32 bpp; none clamps. The
/// ES 2.0 three stay untyped.
#[test]
fn float_rows_and_the_es2_formats() {
    let f = |s| state::rt_format(s).unwrap();
    assert_eq!((f(GLES_SURF_RGBA16F).output, f(GLES_SURF_RGBA16F).internal_type), (18, V3D_INTERNAL_TYPE_16F));
    assert_eq!((f(GLES_SURF_RGBA16F).internal_bpp, f(GLES_SURF_RGBA16F).bytes), (V3D_INTERNAL_BPP_64, 8));
    assert_eq!((f(GLES_SURF_R16F).output, f(GLES_SURF_R16F).internal_type), (20, V3D_INTERNAL_TYPE_16F));
    assert_eq!((f(GLES_SURF_R16F).internal_bpp, f(GLES_SURF_R16F).bytes), (V3D_INTERNAL_BPP_64, 2));
    assert_eq!((f(GLES_SURF_R32F).output, f(GLES_SURF_R32F).internal_type), (11, V3D_INTERNAL_TYPE_32F));
    assert_eq!((f(GLES_SURF_R32F).internal_bpp, f(GLES_SURF_R32F).bytes), (V3D_INTERNAL_BPP_32, 4));
    for s in [GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F] {
        assert_eq!(f(s).clamp, V3D_RENDER_TARGET_CLAMP_NONE);
        assert!(state::typed(s));
    }
    for s in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565] {
        assert!(!state::typed(s), "{}: the ES 2.0 job, byte for byte as before", gles_surf_name(s));
        assert_eq!(f(s).internal_type, V3D_INTERNAL_TYPE_8);
        assert_eq!(f(s).internal_bpp, V3D_INTERNAL_BPP_32);
    }
}

/// The clear words per internal type (`state::clear_words`, as
/// `v3d_tlb_clear` packs them, v3dx_draw.c:1706-1769): the raw integers of
/// glClearBuffer{i,ui}v narrowed to a byte or a half a channel, kept whole
/// at 32 bits; floats as halves; the ES 2.0 formats their RGBA8 word.
#[test]
fn clear_words_pack_per_internal_type() {
    let ints = [0x7f_u32, 0x80, -1i32 as u32, 0x1_0002].map(f32::from_bits);
    assert_eq!(state::clear_words(GLES_SURF_RGBA8UI, ints), [0x02ff_807f, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_R8I, ints), [0x02ff_807f, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGBA16I, ints), [0x0080_007f, 0x0002_ffff, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGB10_A2UI, ints), [0x0080_007f, 0x0002_ffff, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RG32I, ints), ints.map(f32::to_bits));
    assert_eq!(state::clear_words(GLES_SURF_RGBA16F, [1.0, 2.0, 0.5, -1.0]), [0x4000_3c00, 0xbc00_3800, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_R32F, [0.25, 0.0, 0.0, 1.0]), [0x3e80_0000, 0, 0, 0x3f80_0000]);
    assert_eq!(state::clear_words(GLES_SURF_RGBA8888, [0.0, 1.0, 0.0, 1.0]), [0xff00_ff00, 0, 0, 0]);
}

/* ---- the compiler: integer outputs to the TLB ----------------------------- */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What the fragment shader wrote to the TLB for one pixel (lane 0): each
/// write's config (`Some` for a TLBU write) and value.
fn run_fs(fs: &str) -> Vec<(Option<u32>, u32)> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
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
    assert_eq!(p.fs.num_inputs, 0, "the shaders here read no varying");
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    o.tlb.iter().map(|w| (w.config, w.value[0].expect("a defined colour word"))).collect()
}

/// An `ivec4` output is four 32-bit words to the TLB as they are, the first
/// through TLBU with the 32-bit config for RT 0, vec4 (`tlb_config_32b`: per
/// pixel, RT 7 - 0, type F32 (0 << 6: Mesa 26.2.1 nir_to_vir.c:1974-1977
/// writes every `f32_color_rb` target, integer ones included, with
/// TLB_TYPE_F32_COLOR and never TLB_TYPE_I32_COLOR), size 4 - 1), the rest
/// through TLB: no f16 conversion, so -2 stays 0xfffffffe and 0x7fffffff
/// whole.
#[test]
fn an_ivec4_output_writes_four_raw_words() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout ivec4 o;\n\
              void main() { o = ivec4(1, -2, 3, 0x7fffffff); }\n";
    let w = run_fs(fs);
    assert_eq!(tlb_config_32b(0, 4), 0xffff_ff3f);
    assert_eq!(w, [(Some(0xffff_ff3f), 1), (None, 0xffff_fffe), (None, 3), (None, 0x7fff_ffff)], "config | r, g, b, a");
}

/// A `uvec2` output writes two words (size 2 - 1), an `int` one; a `uint`
/// of 0xffffffff is the word itself.
#[test]
fn narrower_integer_outputs_write_their_components() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout uvec2 o;\n\
              void main() { o = uvec2(0xffffffffu, 7u); }\n";
    assert_eq!(run_fs(fs), [(Some(tlb_config_32b(0, 2)), 0xffff_ffff), (None, 7)]);
    assert_eq!(tlb_config_32b(0, 2), 0xffff_ff3d);
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout int o;\nvoid main() { o = -5; }\n";
    assert_eq!(run_fs(fs), [(Some(tlb_config_32b(0, 1)), -5i32 as u32)]);
    assert_eq!(tlb_config_32b(0, 1), 0xffff_ff3c);
}

/// A float output is what it was: two f16 pairs through TLB with no config
/// (RT 0's f16 vec4 config is the TLB's default, `tlb_config_f16`).
#[test]
fn a_float_output_still_writes_f16_pairs() {
    let fs = "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = vec4(1.0, 0.5, -2.0, 0.0); }\n";
    assert_eq!(tlb_config_f16(0, 4), !0);
    assert_eq!(run_fs(fs), [(None, 0x3800_3c00), (None, 0x0000_c000)]);
}

/// Two outputs of different kinds, in RT order: RT 0's float as f16 pairs,
/// then RT 1's `uvec4` as four words with the 32-bit config naming RT 1
/// (7 - 1 in bits 3..5).
#[test]
fn an_integer_output_beside_a_float_one_names_its_render_target() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\n\
              layout(location = 0) out vec4 c;\nlayout(location = 1) out uvec4 u;\n\
              void main() { c = vec4(1.0); u = uvec4(10u, 20u, 30u, 40u); }\n";
    let w = run_fs(fs);
    assert_eq!(tlb_config_32b(1, 4), 0xffff_ff37);
    assert_eq!(
        w,
        [(None, 0x3c00_3c00), (None, 0x3c00_3c00), (Some(0xffff_ff37), 10), (None, 20), (None, 30), (None, 40)]
    );
}
