// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64): the GL_RGB10_A2 render-target class
//! (`GLES_SURF_RGB10_A2`), back end and online compiler, against Mesa 26.2.1
//! (docs/es3/pvr-formats.md sections 2.5 and 10.1).
//!
//! Mesa renders R10G10B10A2_UNORM so (src/imagination, MIT):
//!
//! - PBE: `FORMAT(R10G10B10A2_UNORM, A2R10B10G10, F16)`
//!   (vulkan/pvr_arch_formats.c:223): pack mode A2R10B10G10 (14,
//!   csbgen/rogue/pbestate.xml), F16 accumulation; `norm` set, the format is
//!   fully normalised (pvr_formats.h:264-273); `source_format`
//!   F16_PER_CHANNEL, channel 0 being 10 bits wide
//!   (pvr_arch_pbe_get_src_format_and_gamma, pvr_arch_job_common.c:72-73);
//!   no gamma; the util_format swizzle X,Y,Z,W.
//! - USC: `to_pbe_format` (pco/pco_nir_pvfio.c:103-104) writes it as
//!   R16G16B16A16_FLOAT: four halves in two pixel output dwords, an even
//!   pair as every 64-bit PBE source.
//! - Clear: the F16 accumulation's packing (pvr_formats.c:190), four halves.
//!
//! The back end's PBE words, output dwords and clear packing are called
//! directly (`yakogl::pvr::backend`), the layout through
//! `yakogl::pvr::mrt_layout`, and the fragment shader is compiled for the
//! back end's class of the surface (`rt_class`: the unsigned-normalised
//! halves class the sRGB target uses) and run through the host byte model
//! (tests/usc_exec). The A2R10B10G10 store is modelled as round(v * 1023)
//! and round(v * 3) of the clamped halves. HOST-ONLY: no GPU, no GL
//! read-back; the GL core does not hand this surface to the PowerVR yet
//! (section 10.1).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_rgb10a2_rt
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::backend::*;
use yakogl::pvr::backend::{clear_uniform, pbe_emit_words, pbe_reg_words, rt_class, surf_out_dwords};
use yakogl::pvr::mrt_layout::{layout, register_layout, Place};
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::*;

mod usc_exec;

const RGB10A2: u8 = GLES_SURF_RGB10_A2;

/// STATE_WORD1 (as a 32-bit word, pbestate.xml): packmode [7:12], norm
/// [13], source_pos [14:15], mrt_index [16:18], source_format [19].
fn state1(fmt: u8, source_pos: u32, mrt_index: u32) -> (u32, u32, u32, u32, u32) {
    let [_, w1] = pbe_emit_words(0x80_0000_1000, fmt, source_pos, mrt_index);
    ((w1 >> 7) & 0x3f, (w1 >> 13) & 1, (w1 >> 14) & 3, (w1 >> 16) & 7, (w1 >> 19) & 1)
}

#[test]
fn pbe_state_is_mesas_a2r10b10g10_f16_row() {
    assert_eq!(yakogl::pvr::csbgen::ROGUE_PBESTATE_PACKMODE_A2R10B10G10, 14);
    assert_eq!(yakogl::pvr::csbgen::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL, 0);
    /* packmode 14, normalised, F16 per channel, at the given source and slot */
    assert_eq!(state1(RGB10A2, 0, 0), (14, 1, 0, 0, 0));
    assert_eq!(state1(RGB10A2, 2, 1), (14, 1, 2, 1, 0));
    /* the address word is the surface's, as for every other format */
    assert_eq!(
        pbe_emit_words(0x80_0000_1000, RGB10A2, 0, 0)[0],
        pbe_emit_words(0x80_0000_1000, GLES_SURF_RGBA16F, 0, 0)[0]
    );
    /* RGB10_A2UI keeps its packed U32 dword from the program (packmode 52,
     * not normalised, 8 per channel): the two classes stay apart */
    assert_eq!(state1(GLES_SURF_RGB10_A2UI, 0, 0), (52, 0, 0, 0, 1));
    /* and the classes around it are unchanged: RGBA16F F16F16F16F16 (10),
     * sRGB U8U8U8U8 normalised F16, RGBA8 U8U8U8U8 8 per channel */
    assert_eq!(state1(GLES_SURF_RGBA16F, 0, 0), (10, 0, 0, 0, 0));
    assert_eq!(state1(GLES_SURF_SRGB8_A8, 0, 0), (0, 1, 0, 0, 0));
    assert_eq!(state1(GLES_SURF_RGBA8888, 0, 0), (0, 1, 0, 0, 1));
}

#[test]
fn register_words_are_the_identity_swizzle_without_gamma() {
    for alpha_one in [false, true] {
        let w = pbe_reg_words(RGB10A2, alpha_one, 64, 32, 64, false, false);
        let rgba = pbe_reg_words(GLES_SURF_RGBA8888, alpha_one, 64, 32, 64, false, false);
        /* RGBA8888's words: chan0..3 in order (SWIZ.ONE on alpha only where
         * the GL format has none, which RGB10_A2 has), no gamma (bit 4) */
        assert_eq!(w, rgba, "alpha_one {alpha_one}");
        assert_eq!(w[0] & 1 << 4, 0, "no gamma");
    }
}

#[test]
fn output_is_two_dwords_on_an_even_pair() {
    assert_eq!(surf_out_dwords(RGB10A2), 2);
    assert_eq!(gles_surf_bytes_per_pixel(RGB10A2), 4, "one 10/10/10/2 word a pixel in memory");
    /* alone: registers 0..2 */
    let l = layout(&[surf_out_dwords(RGB10A2) as u8]).unwrap();
    assert_eq!((l.place[0], l.output_regs, l.tile_buffers), (Place::Register { first: 0 }, 2, 0));
    /* after an RGBA8 target the 64-bit source skips register 1 */
    let d = [surf_out_dwords(GLES_SURF_RGBA8888) as u8, surf_out_dwords(RGB10A2) as u8];
    let l = register_layout(&d).unwrap();
    assert_eq!(l.place[1], Place::Register { first: 2 });
    assert_eq!(state1(RGB10A2, l.place[1].source_pos() as u32, 1).2, 2);
    /* three of them: the third spills to tile buffer 0 (eot3_tb) */
    let l = layout(&[2, 2, 2]).unwrap();
    assert_eq!(l.place[2], Place::TileBuffer { index: 0 });
}

#[test]
fn clear_is_the_clamped_colour_in_halves() {
    let h = |x: f32| u32::from(usc_exec::f32_to_f16(x));
    let c = [1.5f32, 0.5, -0.25, f32::NAN];
    assert_eq!(clear_uniform(RGB10A2, &c), [h(1.0) | h(0.5) << 16, h(0.0) | h(0.0) << 16, 0, 0]);
    let g = [0.25f32, 0.75, 0.125, 1.0];
    assert_eq!(clear_uniform(RGB10A2, &g), clear_uniform(GLES_SURF_RGBA16F, &g));
}

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `src` built under `key`: the record's output dwords and fbfetch, and the
/// two pixel output dwords the byte model leaves over the pixel `pixel`.
fn run(src: &str, key: u32, pixel: [f32; 4]) -> (u8, u8, [f32; 4]) {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let (n, fbfetch, image) = store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).unwrap_or_else(|e| panic!("front end: {}", e.msg));
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).unwrap_or_else(|e| panic!("chain: {e:?}"));
        assert_eq!(rec.blend_key, key);
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        (rec.num_outputs, rec.uses_fbfetch, code.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>())
    })
    .unwrap();
    let h = |x: f32| u32::from(usc_exec::f32_to_f16(x));
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.pixout = h(pixel[0]) | h(pixel[1]) << 16;
    inp.pixout1 = h(pixel[2]) | h(pixel[3]) << 16;
    let out = usc_exec::run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("byte model: {e}"));
    let (w0, w1) = (out[&0], out[&1]);
    let got = [w0, w0 >> 16, w1, w1 >> 16].map(|x| usc_exec::f16_to_f32((x & 0xffff) as u16));
    (n, fbfetch, got)
}

/// What the PBE stores from the halves under A2R10B10G10: red in bits 0..9,
/// green 10..19, blue 20..29, alpha 30..31, each round(v * max).
fn a2r10b10g10(v: [f32; 4]) -> u32 {
    let q = |x: f32, max: f32| (x.clamp(0.0, 1.0) * max).round() as u32;
    q(v[0], 1023.0) | q(v[1], 1023.0) << 10 | q(v[2], 1023.0) << 20 | q(v[3], 3.0) << 30
}

const SRC: &str = "#version 300 es
precision highp float;
layout(location = 0) out vec4 o;
void main() { o = vec4(1.5, 0.5, -0.25, 0.75); }
";

#[test]
fn compiler_writes_clamped_halves_in_two_dwords() {
    let cls = rt_class(RGB10A2);
    assert_eq!(cls, SHC_RT_SRGB, "the unsigned-normalised halves class");
    /* the classes the back end does not change */
    for f in [GLES_SURF_RGBA8888, GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F, GLES_SURF_SRGB8_A8] {
        assert_eq!(rt_class(f), gles_surf_rt_class(f), "{}", gles_surf_name(f));
    }
    let pixel = [0.25f32, 0.75, 0.25, 0.5];
    /* a plain draw: the replace variant, clamped, reading no pixel */
    let (n, fbfetch, got) = run(SRC, SHC_BLEND_KEY_REPLACE | shc_blend_key_target(cls, 0), pixel);
    assert_eq!((n, fbfetch), (2, 0));
    assert_eq!(got, [1.0, 0.5, 0.0, 0.75]);
    assert_eq!(a2r10b10g10(got), 1023 | 512 << 10 | 2 << 30);
    /* add (one, one): s' = (1, 0.5, 0, 0.75) + d, clamped */
    let add = shc_blend_key(SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, 0, 0);
    let (n, fbfetch, got) = run(SRC, add | shc_blend_key_target(cls, 0), pixel);
    assert_eq!((n, fbfetch), (2, 1));
    assert_eq!(got, [1.0, 1.0, 0.25, 1.0]);
    /* over: s' * a + d * (1 - a), a = 0.75 */
    let over = shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, 0, 0);
    let (_, _, got) = run(SRC, over | shc_blend_key_target(cls, 0), pixel);
    assert_eq!(got, [0.8125, 0.5625, 0.0625, 0.6875]);
    /* a colour mask that keeps green and alpha: the pixel's values there */
    let (n, fbfetch, got) = run(SRC, SHC_BLEND_KEY_REPLACE | shc_blend_key_target(cls, 0b1010), pixel);
    assert_eq!((n, fbfetch), (2, 1));
    assert_eq!(got, [1.0, 0.75, 0.0, 0.5]);
}
