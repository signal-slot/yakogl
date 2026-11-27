// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The colour-target state of three i.MX8MP dEQP-GLES3 board causes at
//! campaign commit 9bc82915 (`tools/cts/imx8mp-gles3-causes.tsv`,
//! `docs/es3/imx8mp-gles3-compare-9bc82915.tsv` column 2), on the rig of
//! `backend_tests.rs`:
//!
//! - `gc-srgb-target` (46 cases: `fbo.*srgb8_alpha8*`,
//!   `fragment_out.{basic,array}.fixed.srgb8_alpha8_*`). RT0's PE_LOGIC_OP
//!   sRGB bit went out with its `_MASK` bit, which keeps the old value, so the
//!   pixel engine never encoded; and the resolve engine's fill of a clear
//!   stored the linear clear colour. Both are pinned here. The GL core's half
//!   (an sRGB target is B,G,R,A on this part like RGBA8, and glReadPixels,
//!   glBlitFramebuffer and `take_pe_order` treat it as R,G,B,A) is outside
//!   src/vivante.
//! - `gc-rg-int-attachment` (116 cases). The GL core keeps an R or RG 8/16-bit
//!   integer texture incomplete on the GC7000UL (`integer_texture_surface` has
//!   no row for the classes 32-39 it is stored in since 102310ff). The back
//!   end's side, the draw into such a surface, is pinned here: each format's
//!   own PE format, stride and output mode, at RT0 and in the 8-RT bank.
//! - `gc-narrow-target` (27 cases) is the GL core's read-back of R8, RG8,
//!   RGB8 and RGB565 kept in RGBA8888 (e5241007, after 9bc82915). The back
//!   end's part, which this pins: a target the GL core reports without alpha
//!   is never written in its alpha byte, by a draw or by the fill.
//!
//! ```text
//! cargo test --lib --no-default-features --features host-test,gpu-vivante,shaderc-online colour_formats_tests
//! ```

use super::*;

/// The base PE_LOGIC_OP word of a single-target linear draw.
fn logic_base() -> u32 {
    GC_PE_LOGIC_OP_BASE | pe_logic_op_op(GC_LOGIC_OP_COPY) | pe_logic_op_single_buffer(1)
}

/// A draw into `format` at RT0 (the rig's 64x64 target).
fn draw_into(rig: &Rig, format: u8) -> GlesDraw {
    let mut d = rig.draw(3);
    d.target_format = format;
    d.targets[0].format = format;
    d
}

/// A whole-target clear of `rgba` into `format`, by the resolve engine's
/// fill where it takes it.
fn fill_of(rig: &Rig, format: u8, rgba: [f32; 4]) -> Stream {
    gc_debug_rs_clear(true);
    let mut d = draw_into(rig, format);
    d.do_clear = true;
    d.clear_rgba = rgba;
    let s = rig.submit(&d);
    gc_debug_rs_clear(false);
    s
}

/// Every PE_LOGIC_OP word the stream writes.
fn logic_words(s: &Stream) -> Vec<u32> {
    let mut v = Vec::new();
    walk(&s.0, |r, w, _| {
        if r == GC_PE_LOGIC_OP {
            v.push(w);
        }
    });
    v
}

/// gc-srgb-target, the PE half: an sRGB draw writes PE_LOGIC_OP with the
/// sRGB bit and without its mask bit, as Mesa does (etnaviv_state.c:326-327),
/// in every PE_LOGIC_OP the stream holds; a draw into RGBA8888 before and
/// after it writes the bit clear, so an sRGB draw never leaves it set for the
/// next target.
#[test]
fn the_srgb_bit_is_written_without_its_mask() {
    let rig = Rig::new();
    let before = rig.submit(&rig.draw(3));
    let srgb = rig.submit(&draw_into(&rig, GLES_SURF_SRGB8_A8));
    let after = rig.submit(&rig.draw(3));

    let words = logic_words(&srgb);
    assert!(!words.is_empty());
    for w in &words {
        assert_eq!(w & (GC_PE_LOGIC_OP_SRGB | GC_PE_LOGIC_OP_SRGB_MASK), GC_PE_LOGIC_OP_SRGB, "0x{w:08x}");
    }
    assert_eq!(srgb.reg(GC_PE_LOGIC_OP), logic_base() | GC_PE_LOGIC_OP_SRGB);
    for (s, what) in [(&before, "before"), (&after, "after")] {
        for w in logic_words(s) {
            assert_eq!(w & (GC_PE_LOGIC_OP_SRGB | GC_PE_LOGIC_OP_SRGB_MASK), 0, "{what}: 0x{w:08x}");
        }
        assert_eq!(s.reg(GC_PE_LOGIC_OP), logic_base(), "{what} the sRGB draw");
    }
    /* the sRGB target is RGBA8888's PE format otherwise */
    assert_eq!(srgb.reg(GC_PE_COLOR_FORMAT), before.reg(GC_PE_COLOR_FORMAT));
    assert_eq!(srgb.reg(GC_PE_COLOR_STRIDE), before.reg(GC_PE_COLOR_STRIDE));
}

/// gc-srgb-target, the bank: an sRGB draw buffer at RT1 carries
/// RT_CONFIG_SRGB (no mask bit there, etnaviv_state.c:333), RT0 an RGBA8888
/// target beside it leaves PE_LOGIC_OP's bit clear, and an sRGB RT0 beside an
/// RGBA8888 RT1 sets PE_LOGIC_OP's and not RT1's.
#[test]
fn an_srgb_draw_buffer_beside_a_linear_one_encodes_only_itself() {
    let rig = Rig::new();
    let mut fs = *rig.fs;
    fs.viv.num_out = 2;
    for (i, reg) in [3u8, 4].iter().enumerate() {
        fs.viv.out[i] = ShcVivIo { reg: *reg, slot: 4 + i as u8, components: 4, interp: 0, name: [0; 32] };
    }
    let (rgba, _) = pe_color_format(GLES_SURF_RGBA8888);
    let words = rt_config_stride(64 * 4) | rt_config_format(rgba);
    for (rt0, rt1) in [(GLES_SURF_RGBA8888, GLES_SURF_SRGB8_A8), (GLES_SURF_SRGB8_A8, GLES_SURF_RGBA8888)] {
        let mut d = draw_into(&rig, rt0);
        d.fs = &fs;
        d.num_targets = 2;
        d.targets[1] = GlesDrawTarget { gpu: TARGET_GPU + 0x10_0000, w: 64, h: 64, format: rt1, out_shift: 0 };
        let s = rig.submit(&d);
        let rt0_srgb = if rt0 == GLES_SURF_SRGB8_A8 { GC_PE_LOGIC_OP_SRGB } else { 0 };
        let rt1_srgb = if rt1 == GLES_SURF_SRGB8_A8 { GC_RT_CONFIG_SRGB } else { 0 };
        assert_eq!(s.reg(GC_PE_LOGIC_OP), logic_base() | rt0_srgb, "RT0 {rt0}, RT1 {rt1}");
        assert_eq!(s.reg(pe_rt_config_8(1)), words | rt1_srgb, "RT0 {rt0}, RT1 {rt1}");
    }
}

/// gc-srgb-target, the fill: a whole-target clear of an sRGB target by the
/// resolve engine stores the clear colour's red, green and blue encoded
/// (ES 3.0.6 4.1.8; `gl::srgb::encode`), alpha linear, as Mesa packs its RS
/// clear value through the sRGB format (etna_clear_blit_pack_rgba). The fill
/// bypasses the pixel engine's encode, and it stored the linear bytes
/// (fbo.color.clear.srgb8_alpha8, fbo.render.shared_colorbuffer_clear.*_srgb8_alpha8).
/// The RGBA8888 fill of the same colour is unchanged.
#[test]
fn the_resolve_engine_fill_of_an_srgb_target_is_encoded() {
    let rig = Rig::new();
    let rgba = [0.5, 0.2, 1.0, 0.5];
    let srgb = fill_of(&rig, GLES_SURF_SRGB8_A8, rgba);
    let plain = fill_of(&rig, GLES_SURF_RGBA8888, rgba);
    for s in [&srgb, &plain] {
        assert_eq!(s.draws().len(), 1, "the fill, then the draw: no clear quad");
        assert_eq!(s.writes(GC_RS_KICKER).1, 1);
    }
    /* 0.5 -> 188, 0.2 -> 124, 1.0 -> 255 on the curve; alpha 0.5 -> 128 */
    assert_eq!(
        (crate::gl::srgb::encode(0.5), crate::gl::srgb::encode(0.2), crate::gl::srgb::encode(1.0)),
        (0xbc, 0x7c, 0xff)
    );
    for i in 0..4 {
        assert_eq!(srgb.reg(rs_fill_value(i)), 0x80bc_7cff, "A8R8G8B8 of the encoded (0.5, 0.2, 1.0), alpha 0.5");
        assert_eq!(plain.reg(rs_fill_value(i)), 0x8080_33ff, "RGBA8888: the unorm bytes, as before");
    }
    /* the ends of the curve and out-of-range values: 0 and 1 are fixed
     * points, the rest is clamped as the unorm path clamps */
    let ends = fill_of(&rig, GLES_SURF_SRGB8_A8, [0.0, 1.0, -0.5, 2.0]);
    assert_eq!(ends.reg(rs_fill_value(0)), 0xff00_ff00);
    /* the draw that follows the fill encodes through the pixel engine */
    assert_eq!(srgb.reg(GC_PE_LOGIC_OP), logic_base() | GC_PE_LOGIC_OP_SRGB);
}

/// [`rs_fill_rgb`] for every surface: the sRGB one alone takes the curve.
#[test]
fn only_the_srgb_surface_takes_the_curve_in_the_fill_word() {
    let rgb = [0.25, 0.75, 0.04];
    let unorm = (unorm8(0.25) << 16) | (unorm8(0.75) << 8) | unorm8(0.04);
    for f in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565] {
        assert_eq!(rs_fill_rgb(f, rgb), unorm, "surface {f}");
    }
    let e = |v: f32| u32::from(crate::gl::srgb::encode(v));
    assert_eq!(rs_fill_rgb(GLES_SURF_SRGB8_A8, rgb), (e(0.25) << 16) | (e(0.75) << 8) | e(0.04));
    assert_ne!(rs_fill_rgb(GLES_SURF_SRGB8_A8, rgb), unorm);
}

/// The R and RG integer surfaces of 8 and 16 bits a channel, with their PE
/// format (etnaviv_format.c:91, 103, 121, 148: R8I, R16I, G8R8I, G16R16I for
/// both signs), PS output mode and bytes a pixel.
const NARROW_INT: [(u8, u32, u32, u32); 8] = [
    (GLES_SURF_R8I, GC_PE_FORMAT_R8I, GC_PS_OUTPUT_MODE_I8, 1),
    (GLES_SURF_R8UI, GC_PE_FORMAT_R8I, GC_PS_OUTPUT_MODE_U8, 1),
    (GLES_SURF_RG8I, GC_PE_FORMAT_G8R8I, GC_PS_OUTPUT_MODE_I8, 2),
    (GLES_SURF_RG8UI, GC_PE_FORMAT_G8R8I, GC_PS_OUTPUT_MODE_U8, 2),
    (GLES_SURF_R16I, GC_PE_FORMAT_R16I, GC_PS_OUTPUT_MODE_I16, 2),
    (GLES_SURF_R16UI, GC_PE_FORMAT_R16I, GC_PS_OUTPUT_MODE_U16, 2),
    (GLES_SURF_RG16I, GC_PE_FORMAT_G16R16I, GC_PS_OUTPUT_MODE_I16, 4),
    (GLES_SURF_RG16UI, GC_PE_FORMAT_G16R16I, GC_PS_OUTPUT_MODE_U16, 4),
];

/// gc-rg-int-attachment, the back end's side: a draw into an R or RG 8/16-bit
/// integer surface (the surface of the GC7000UL's texture classes 32-39 and
/// of the renderbuffers of those names) is that format's own target: the
/// extended PE format under FORMAT's mask, the linear stride of its own
/// bytes a pixel, the integer output mode, no unorm saturate, no
/// sRGB bit, no tiled shadow and no resolve-engine fill. Mesa renders these
/// formats with the same PE rows (etnaviv_format.c).
#[test]
fn an_r_or_rg_integer_target_is_its_own_pe_format() {
    let rig = Rig::new();
    for (format, pe, mode, bpp) in NARROW_INT {
        assert_ne!(GC_INTEGER_RT_FORMATS & (1 << (format - GLES_SURF_INT_FIRST)), 0, "surface {format}: drawn");
        assert_eq!(gles_surf_bytes_per_pixel(format), bpp, "surface {format}");
        assert_eq!(pe_color_format(format), (pe, true), "surface {format}");
        let s = rig.submit(&draw_into(&rig, format));
        let f = s.reg(GC_PE_COLOR_FORMAT);
        assert_eq!(f & 0xf, 0, "surface {format}: FORMAT left under its mask");
        assert_ne!(f & GC_PE_COLOR_FORMAT_FORMAT_MASK, 0, "surface {format}");
        assert_eq!(f & GC_PE_COLOR_FORMAT_FORMAT_EXT_MASK, 0, "surface {format}");
        assert_eq!(
            f & !(GC_PE_COLOR_FORMAT_OVERWRITE | (0xf << 8)),
            pe_color_format_format_ext(pe) | GC_PE_COLOR_FORMAT_FORMAT_MASK
        );
        assert_eq!(s.reg(GC_PE_COLOR_STRIDE), 64 * bpp, "surface {format}: linear, its own pixel size");
        assert_eq!(s.reg(pe_pipe_color_addr(0)), TARGET_GPU as u32, "surface {format}: the storage, no shadow");
        assert_eq!(s.reg(GC_PE_LOGIC_OP), logic_base(), "surface {format}: SINGLE_BUFFER(1), no sRGB");
        assert_eq!(s.reg(GC_PS_CONTROL_EXT), ps_output_mode(0, mode), "surface {format}");
        assert_eq!(s.reg(GC_PS_CONTROL) & ps_control_saturate_rt(0), 0, "surface {format}: no saturate");

        /* a clear stays the quad (the fill takes A8R8G8B8 and X8R8G8B8 only) */
        let c = fill_of(&rig, format, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(c.writes(GC_RS_KICKER).1, 0, "surface {format}: no fill");
        assert_eq!(c.draws().len(), 2, "surface {format}: the clear quad and the draw");
    }
}

/// gc-rg-int-attachment in the 8-RT bank: each R or RG integer surface at RT1
/// beside an RGBA8888 RT0 is its own PE format and stride in RT_CONFIG, its
/// output mode in PS_CONTROL_EXT's RT1 field, and never blended.
#[test]
fn an_r_or_rg_integer_draw_buffer_is_its_own_rt_config() {
    let rig = Rig::new();
    let mut fs = *rig.fs;
    fs.viv.num_out = 2;
    for (i, reg) in [3u8, 4].iter().enumerate() {
        fs.viv.out[i] = ShcVivIo { reg: *reg, slot: 4 + i as u8, components: 4, interp: 0, name: [0; 32] };
    }
    for (format, pe, mode, bpp) in NARROW_INT {
        let mut d = rig.draw(3);
        d.fs = &fs;
        d.num_targets = 2;
        d.targets[1] = GlesDrawTarget { gpu: TARGET_GPU + 0x10_0000, w: 64, h: 64, format, out_shift: 0 };
        let s = rig.submit(&d);
        assert_eq!(s.reg(pe_rt_config_8(1)), rt_config_stride(64 * bpp) | rt_config_format(pe), "surface {format}");
        assert_eq!(s.reg(pe_rt_addr_8_pipe(1, 0)), (TARGET_GPU + 0x10_0000) as u32, "surface {format}");
        assert_eq!(s.reg(GC_PS_CONTROL_EXT), ps_output_mode(1, mode), "surface {format}: RT0 NORMAL, RT1 integer");
        assert_eq!(s.reg(pe_halti5_rt_alpha_config(1)), 0, "surface {format}");
    }
}

/// gc-narrow-target, the back end's side: the GL core keeps R8, RG8, RGB8 and
/// RGB565 in RGBA8888 and reports no alpha bits for the alpha-less ones
/// (`GlesDraw::target_alpha_bits` 0). A draw into such a target leaves its
/// alpha byte alone (COMPONENTS without A, no OVERWRITE) and reads
/// destination alpha as 1 in a blend (DST_ALPHA is ONE); the RGBA8888 target
/// with alpha keeps the whole-pixel write. What the absent channels read is
/// the GL core's (glReadPixels, e5241007).
#[test]
fn an_alpha_less_rgba8888_target_keeps_its_alpha_byte() {
    let rig = Rig::new();
    let components = |s: &Stream| (s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf;
    let overwrite = |s: &Stream| s.reg(GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_OVERWRITE != 0;

    let with_alpha = {
        let mut d = rig.draw(3);
        d.target_alpha_bits = 8;
        rig.submit(&d)
    };
    assert_eq!((components(&with_alpha), overwrite(&with_alpha)), (0xf, true));

    let mut d = rig.draw(3);
    d.target_alpha_bits = 0;
    let none = rig.submit(&d);
    assert_eq!((components(&none), overwrite(&none)), (0x7, false), "alpha never written");

    d.color_mask = 0x1; /* R alone */
    let r = rig.submit(&d);
    assert_eq!((components(&r), overwrite(&r)), (0x1, false));

    d.color_mask = 0xf;
    d.blend = true;
    d.blend_src_rgb = SHC_BF_DST_ALPHA as u8;
    d.blend_dst_rgb = SHC_BF_ZERO as u8;
    d.blend_src_alpha = SHC_BF_ONE as u8;
    d.blend_dst_alpha = SHC_BF_ZERO as u8;
    let b = rig.submit(&d);
    let src = (b.reg(GC_PE_ALPHA_CONFIG) >> 4) & 0xf;
    assert_eq!(src, GC_BLEND_ONE, "DST_ALPHA of an alpha-less target is ONE");
    assert_eq!((components(&b), overwrite(&b)), (0x7, false));
}
