// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The csbgen packers against the words they replaced (docs/source-form.md
//! step 3): the hand-packed formulas of backend.rs before the change, kept
//! here verbatim as the reference, and every packer call checked against them
//! over a sweep of inputs. The pins in backend.rs (`TEX_IMAGE_WORD0_*`, ...)
//! are the constants these formulas used.

use super::*;

/// TEXSTATE_IMAGE_WORD0.textype CUBE, as the old formula wrote it.
const TEX_TEXTYPE_CUBE: u64 = 3;

/* ---- the old texture descriptor, verbatim (7296d8a3 src/pvr/backend.rs) ---- */

fn old_build_tex_desc(w: &mut [u32], t: &GlesTextureBind, warned: &mut u32, force_mip0: bool) {
    let mut iw0: u64;
    let mut iw1: u64;

    /* An image address outside the heaps is a texture fetch the TCU will
     * fault on (the fault's VA is then this address plus a texel offset,
     * which reads as a small unmapped address). Say so here, where the
     * binding is still known. */
    if t.base_gpu < HEAP_GENERAL_BASE && *warned < 4 {
        *warned += 1;
        glog!(
            "pvr_backend: sampler {}: image address 0x{:x} is not a heap address ({}x{} fmt {})",
            t.unit,
            t.base_gpu,
            t.width,
            t.height,
            t.format
        );
    }
    /* the mip chain the sampler may walk: levels consecutive in memory
     * (Mesa pvr_image_plane_setup_mip_levels) */
    let mut levels: u32 = if t.mip_filter != 0 && t.mip_levels > 1 && !force_mip0 { t.mip_levels as u32 } else { 1 };

    if t.format == GLES_TEXFMT_RGBA8 {
        iw0 = if t.bgra != 0 { TEX_IMAGE_WORD0_BGRA } else { TEX_IMAGE_WORD0_BASE };
        iw1 = TEX_IMAGE_WORD1_BASE | t.stride_texels.wrapping_sub(1) as u64;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(4); /* layer size (bytes) */
        w[7] = TEX_PCK_INFO_RGBA8_UNORM;
    } else if t.format == GLES_TEXFMT_RGB565 {
        iw0 = TEX_IMAGE_WORD0_RGB565;
        iw1 = TEX_IMAGE_WORD1_BASE | t.stride_texels.wrapping_sub(1) as u64;
        levels = 1;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(2);
        w[7] = TEX_PCK_INFO_COMPRESSED_UNORM; /* no pack conversion: the TPU hands back unorm */
    } else if matches!(t.format, GLES_TEXFMT_L8 | GLES_TEXFMT_A8 | GLES_TEXFMT_LA8) {
        iw0 = match t.format {
            GLES_TEXFMT_L8 => TEX_IMAGE_WORD0_L8,
            GLES_TEXFMT_A8 => TEX_IMAGE_WORD0_A8,
            _ => TEX_IMAGE_WORD0_LA8,
        };
        iw1 = TEX_IMAGE_WORD1_BASE | t.stride_texels.wrapping_sub(1) as u64;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(gles_texfmt_bytes(t.format));
        /* Mesa's pack format for R8/R8G8 UNORM is U8888 with the UNORM bit
         * (0x140, pco_pipe_to_pck_format), and pco reads it only for shadow
         * compares and formatless stores; RGB565 samples right with 0x11f.
         * Which one the narrow formats want is the board's to say:
         * `pvr_narrow_la` 1 takes 0x140, 2 takes 0x11f. */
        w[7] = if narrow_la_mode() == 2 { TEX_PCK_INFO_COMPRESSED_UNORM } else { TEX_PCK_INFO_RGBA8_UNORM };
    } else if matches!(t.format, GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F) {
        iw0 = match t.format {
            GLES_TEXFMT_RGBA16F => TEX_IMAGE_WORD0_RGBA16F,
            GLES_TEXFMT_R16F => TEX_IMAGE_WORD0_R16F,
            _ if t.depth_luminance != 0 => TEX_IMAGE_WORD0_R32F_LUMINANCE,
            _ => TEX_IMAGE_WORD0_R32F,
        };
        iw1 = TEX_IMAGE_WORD1_BASE | t.stride_texels.wrapping_sub(1) as u64;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(gles_texfmt_bytes(t.format));
        w[7] = if t.format == GLES_TEXFMT_R32F { TEX_PCK_INFO_F32 } else { TEX_PCK_INFO_F16 };
    } else {
        iw0 = if t.format == GLES_TEXFMT_ETC2_RGB {
            TEX_IMAGE_WORD0_ETC2_RGB
        } else if t.format == GLES_TEXFMT_ETC2_RGBA {
            TEX_IMAGE_WORD0_ETC2_RGBA
        } else {
            TEX_IMAGE_WORD0_ETC2_PT
        };
        /* The TWIDDLED word (texstate.xml IMAGE_WORD1): num_mip_levels at
         * [0:3] and baselevel at [60:63] -- the tail below writes `levels`
         * at [60:63] for the STRIDE word only. It wrote it here too until
         * 2026-09-24, so every ETC texture asked for base level 1 with no
         * levels (the AM62P's filtering.2d *_etc1, format.etc1). The GL
         * core lays the chain out level-major and packed, level l's grid
         * level 0's power-of-two grid halved l times (texture.rs
         * level_block_grid), which is the TPU's walk (Mesa
         * pvr_image_plane_setup_mip_levels: max(1, (POT(W)/4) >> l)
         * blocks), NPOT sizes included. */
        iw1 = TEX_IMAGE_WORD1_COMPRESSED_TPU | (levels as u64 & 0xf); /* twiddled: no stride, no alpha_msb */
        w[4] = t.bytes;
        w[7] = TEX_PCK_INFO_COMPRESSED_UNORM;
    }
    if t.type_ == GLES_TEXTYPE_CUBE_TWIDDLED_PROBE {
        /* The native cube (gl::native_cube, pvr_native_cube): the TPU's own
         * cube over Mesa's twiddled face-major layout, in the format's own
         * word 0 and pack info above. TEXSTATE_IMAGE_WORD0.textype = CUBE (3,
         * texstate.xml:248-253, bits 0..2); the twiddled IMAGE_WORD1
         * (texstate.xml:292-312) is NOT the stride word: num_mip_levels at
         * [0:3], depth = array layers - 1 = 5 at [4:14] (Mesa
         * pvr_arch_tex_state.c:278-286), mipmaps_present [15], texaddr
         * [16:53], baselevel [60:63] = 0, the rest as the format's. The
         * image meta's layer size is one face's full chain, 4 KiB aligned
         * (pvr_image.c), which is what the copy is laid out in. */
        iw0 = (iw0 & !7) | TEX_TEXTYPE_CUBE;
        iw0 |= ((t.width.wrapping_sub(1) as u64) << 34) | ((t.height.wrapping_sub(1) as u64) << 48);
        /* The level count in the IMAGE word is the copy's whole chain, to
         * 1x1 of the face's power-of-two side, whatever the filter: the TPU
         * steps from face to face by the layer size it derives from it,
         * one level's size for a count of 1 and the full chain for any
         * more (Mesa pvr_image_plane_setup_mip_levels adds the rest of the
         * chain only when mip_levels > 1), and gl::native_cube lays every
         * face out as its full chain. With the sampler's count here, every
         * single-level cube read faces 1..5 from the wrong bytes on the
         * AM62P (am62p-board, 33a24bd7: 132 non-mipmap cases with
         * YAKOGL_PVR_NATIVE_CUBE=1). How far the sampler may walk is the
         * SAMPLER's LOD range (`tex_desc_sampler` over `levels`).
         *
         * A draw that samples ONE level gets a one-level copy (level 0 in
         * faces of its own size, gl::native_cube::copy's `single`, decided
         * by the same rule as here: a mipmap filter over more than one
         * level), described as one level. Declaring the whole chain over
         * it let the one-level sampler's maxlod of 1.0 (the min/mag choice
         * the TPU makes on the clamped LOD) read level 1, which the texture
         * never had: 3b634e98's native arm, filtering.cube min != mag (32)
         * and size.cube 256/512 non-mipmap, all white (6). The debug
         * FORCE_MIP0 option does not change the layout here. */
        let single = !(t.mip_filter != 0 && t.mip_levels > 1);
        let chain = if single { 1 } else { 32 - t.width.max(1).next_power_of_two().leading_zeros() };
        /* an ETC cube: the compressed word's mode (TPU decode) and no
         * alpha_msb, as its 2D word above; gl::native_cube lays its faces
         * out as the block grids the TPU walks. Step C's default sent ETC1
         * cubes here with the copy refusing them: filtering.cube.*_etc1 and
         * format.etc1_cube_pot failed on the AM62P (15550645). */
        let etc = matches!(t.format, GLES_TEXFMT_ETC2_RGB | GLES_TEXFMT_ETC2_RGBA | GLES_TEXFMT_ETC2_RGB_PT);
        iw1 = (if etc { TEX_IMAGE_WORD1_COMPRESSED_TPU } else { TEX_IMAGE_WORD1_BASE })
            | (chain.min(15) as u64)
            | (5u64 << 4)
            | (if chain > 1 { 1u64 << 15 } else { 0 })
            | (((t.base_gpu >> 2) & 0x3F_FFFF_FFFF) << 16);
        w[4] = t.bytes / 6; /* one face, 4 KiB aligned (gl::native_cube) */
        w[0] = iw0 as u32;
        w[1] = (iw0 >> 32) as u32;
        w[2] = iw1 as u32;
        w[3] = (iw1 >> 32) as u32;
        old_tex_desc_sampler(w, t, levels);
        /* GL ES 2.0 cube maps are not seamless: each face filters and wraps
         * by its own address modes (dEQP's ES 2.0 reference,
         * seamlessCubeMap false), which SAMPLER_WORD1.cemedge_dontfilter
         * selects (texstate.xml, bit 63; Mesa pvr_arch_sampler.c sets it
         * for VK_EXT_non_seamless_cube_map). ES 3.0 requires seamless: the
         * bit stays clear there. With it clear the AM62P's native arm
         * still failed texture_functions.vertex.texturecubelod in its
         * high-LOD corner, where a face is 2x2 and every sample is on an
         * edge (am62p-board, 33a24bd7). */
        if crate::REPORT_ES2 {
            w[11] |= TEX_SAMPLER_WORD1_CEMEDGE_DONTFILTER_HI;
        }
        return;
    }
    iw0 |= ((t.width.wrapping_sub(1) as u64) << 34) | ((t.height.wrapping_sub(1) as u64) << 48);
    /* STRIDE_IMAGE_WORD1.num_mip_levels is [60:63]; in the twiddled word
     * (the ETC formats, set above) that is baselevel, 0 */
    let twiddled = matches!(t.format, GLES_TEXFMT_ETC2_RGB | GLES_TEXFMT_ETC2_RGBA | GLES_TEXFMT_ETC2_RGB_PT);
    iw1 |= (if twiddled { 0 } else { (levels as u64) << 60 })
        | (if levels > 1 { 1u64 << 15 } else { 0 })
        | (((t.base_gpu >> 2) & 0x3F_FFFF_FFFF) << 16);

    w[0] = iw0 as u32;
    w[1] = (iw0 >> 32) as u32;
    w[2] = iw1 as u32;
    w[3] = (iw1 >> 32) as u32;
    old_tex_desc_sampler(w, t, levels);
}

/// Sampler words 8 and 9 of a descriptor: address modes, filters and the LOD
/// range over `levels` levels; and sampler meta word 0 (dword 12), the depth
/// compare a shadow look-up reads when it runs (Mesa's
/// PCO_SAMPLER_META_COMPARE_OP; the online compiler's shadow_compare).
fn old_tex_desc_sampler(w: &mut [u32], t: &GlesTextureBind, levels: u32) {
    w[12] = t.compare_op as u32;
    w[8] = TEX_SAMPLER_WORD0_LO;
    w[9] = (tpu_addrmode(t.wrap_s) << TEX_SAMPLER_ADDRMODE_U_SHIFT)
        | (tpu_addrmode(t.wrap_t) << TEX_SAMPLER_ADDRMODE_V_SHIFT)
        /* the third axis is never sampled here (2D, and the emulated cube and
         * array targets address inside one tall 2D image) */
        | (TEX_ADDRMODE_CLAMP_TO_EDGE << TEX_SAMPLER_ADDRMODE_W_SHIFT)
        | (if t.linear_filter != 0 { TEX_SAMPLER_MAG_LINEAR_HI } else { 0 })
        | (if t.min_linear != 0 { TEX_SAMPLER_MIN_LINEAR_HI } else { 0 });
    if levels > 1 {
        /* The LOD range covers the chain and stops at its last level: the
         * levels past it are memory nobody wrote (an application may declare
         * a full chain and fill part of it), and the GL core already clamped
         * `levels` to what was written. u4.6, so 64 per level. */
        let maxlod = ((levels - 1) * 64).min(TEX_SAMPLER_MAXLOD_MAX);

        w[8] |= (maxlod & 0x1ff) << 23;
        w[9] |= (maxlod >> 9) & 1;
        if t.mip_filter == 2 {
            w[9] |= TEX_SAMPLER_MIP_LINEAR_HI;
        }
    } else if t.min_linear != t.linear_filter {
        /* The TPU chooses between the min and mag filters on the CLAMPED LOD
         * (<= 0 magnifies, > 0 minifies; Mesa pvr_arch_sampler.c:88-97), and
         * GL chooses on the unclamped one. With maxlod 0 every pixel of a
         * single-level texture magnified, so GL_TEXTURE_MIN_FILTER was never
         * applied: dEQP texture.filtering.2d linear_nearest_* and
         * nearest_linear_* failed on the AM62P with either compiler. A max of
         * LOD 1.0 lets the clamped LOD be positive; the image descriptor still
         * has one level, so the texel still comes from level 0. */
        w[8] |= (64 & 0x1ff) << 23;
    }
}

/* ---- the old fragment-register formulas (7296d8a3 src/pvr/backend.rs) ---- */

fn old_emit(a: u64, fmt: u8, spos: u32, j: u32) -> [u32; 2] {
    const PBE_PACKMODE_R5G6B5: u32 = 0x14;
    const PBE_PACKMODE_F16X4: u32 = 0x0a;
    const PBE_PACKMODE_F16: u32 = 0x33;
    const PBE_PACKMODE_F32: u32 = 0x36;
    const PBE_SOURCE_8PER: u32 = 0x80000;
    const PBE_NORM: u32 = 0x2000;
    let irt = gles_surf_rt_class(fmt);
    let mut e = [0u32; 2];
    e[0] = ((a >> 2) & 0xFFFF_FFFF) as u32;
    e[1] = ((a >> 34) & 0x3F) as u32
        | EOT_EMIT_WORD1_FLAGS
        | (spos << 14)
        | ((j & 7) << 16)
        | (if fmt == GLES_SURF_RGB565 { PBE_PACKMODE_R5G6B5 << 7 } else { 0 });
    if irt != 0 {
        e[1] &= !(PBE_SOURCE_8PER | PBE_NORM | (0x3f << 7));
        e[1] |= (match fmt {
            GLES_SURF_RGBA16F => PBE_PACKMODE_F16X4,
            GLES_SURF_R16F => PBE_PACKMODE_F16,
            _ => PBE_PACKMODE_F32,
        }) << 7;
    }
    e
}

fn old_pbe_swizzle(fmt: u8, alpha_one: bool) -> u64 {
    const PBE_SWIZ_CHAN3_MASK: u64 = 0x7 << 29;
    let swiz = if fmt == GLES_SURF_XRGB8888 || fmt == GLES_SURF_RGB565 { PBE_SWIZZLE_BGRA } else { PBE_SWIZZLE_RGBA };
    if alpha_one {
        (swiz & !PBE_SWIZ_CHAN3_MASK) | PBE_SWIZ_ONE
    } else {
        swiz
    }
}

fn old_pbe_reg(fmt: u8, alpha_one: bool, tw: u32, th: u32, downscale: bool, twiddle: bool) -> [u64; 2] {
    let mut w0 = PBE_TILERELATIVE
        | (if downscale { PBE_DOWNSCALE } else { 0 })
        | (((tw.wrapping_sub(1) / 2) as u64) << 34)
        | old_pbe_swizzle(fmt, alpha_one);
    let mut w1 = th.wrapping_sub(1) as u64 | ((tw.wrapping_sub(1) as u64) << 32);
    if twiddle {
        w0 |= 1u64 << 32;
        w1 |= ((tw.trailing_zeros() as u64) << 60) | ((th.trailing_zeros() as u64) << 28);
    }
    [w0, w1]
}

fn old_zlsctl(tx: u32, ty: u32, load_depth: bool, load_stencil: bool) -> u64 {
    ((((tx - 1) & 0x3ff) as u64) << 3)
        | ((((ty - 1) & 0x3ff) as u64) << 27)
        | (1u64 << 23)
        | (1u64 << 25)
        | ZLSCTL_ZSTOREEN
        | ZLSCTL_SSTOREEN
        | ZLSCTL_FORCEZSTORE
        | (if load_depth { ZLSCTL_ZLOADEN } else { 0 })
        | (if load_stencil { ZLSCTL_SLOADEN } else { 0 })
        | (if load_depth || load_stencil { ZLSCTL_FORCEZLOAD } else { 0 })
}

fn old_bgnd(shader: u32, code: u32, data: u32, data_dw: u32, temps: u32) -> [u64; 3] {
    [
        shader as u64 | ((code as u64) << 32),
        (data as u64) << 32,
        ((CLEAR_ATTACH_DWORDS.div_ceil(16) as u64) << 55)
            | ((data_dw.wrapping_mul(4).div_ceil(4) as u64) << 16)
            | (temps.div_ceil(4) as u64),
    ]
}

/// A small deterministic generator for the sweeps.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

/// A heap address as the allocators hand them out: 16-byte aligned, 40 bits.
fn heap_addr(r: &mut Lcg) -> u64 {
    (HEAP_GENERAL_BASE + ((r.next() as u64) << 4)) & 0xFF_FFFF_FFF0
}

#[test]
fn csbgen_fragment_registers_equal_the_hand_packed_ones() {
    /* the captured / hand-written constants */
    assert_eq!(usc_pixel_output_ctrl(csb::ROGUE_CR_PIXEL_WIDTH_4REGISTERS), USC_PIXEL_OUTPUT_CTRL);
    for w in 0..4 {
        assert_eq!(usc_pixel_output_ctrl(w), (USC_PIXEL_OUTPUT_CTRL & !3) | w, "MRT_STRIDE width {w}");
    }
    for s in [0, 1] {
        assert_eq!(isp_ctl(s, 4, false), ISP_CTL_DEFAULT, "{s} samples");
        assert_eq!(isp_ctl(s, 4, true), ISP_CTL_DEFAULT | ISP_CTL_DBIAS_IS_INT, "{s} samples");
    }
    assert_eq!(isp_ctl(4, 4, false), ISP_CTL_MSAA4);
    assert_eq!(isp_ctl(4, 4, true), ISP_CTL_MSAA4 | ISP_CTL_DBIAS_IS_INT);
    for (s, mode) in [(0, 0), (1, 0), (2, 1), (4, 2), (8, 0)] {
        assert_eq!(isp_aa(s), mode, "{s} samples");
    }
    for st in 0..=255u8 {
        assert_eq!(isp_bgobjvals(st), ISP_BGOBJVALS_MASK | st as u32);
    }
    assert_eq!(te_psg(), TE_PSG_DEFAULT);
    assert_eq!(ppp_ctrl(false), PPP_CTRL_CLIPSPACE);
    assert_eq!(ppp_ctrl(true), PPP_CTRL_PRETRANSFORM);

    let mut r = Lcg(1);
    let fmts =
        [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565, GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F];
    for _ in 0..20000 {
        let fmt = fmts[r.next() as usize % fmts.len()];
        let a = heap_addr(&mut r);
        let spos = r.next() % 4;
        let j = r.next() % 8;
        assert_eq!(pbe_emit_words(a, fmt, spos, j), old_emit(a, fmt, spos, j), "emit fmt {fmt} a {a:#x}");
        /* any size, zero included */
        let tw = if r.next().is_multiple_of(8) { 0 } else { r.next() % 16385 };
        let th = if r.next().is_multiple_of(8) { 0 } else { r.next() % 16385 };
        let alpha_one = r.next() & 1 != 0;
        let down = r.next() & 1 != 0;
        let twiddle = r.next() & 1 != 0 && tw.is_power_of_two() && th.is_power_of_two();
        assert_eq!(
            pbe_reg_words(fmt, alpha_one, tw, th, tw.wrapping_add(1) & !1, down, twiddle),
            old_pbe_reg(fmt, alpha_one, tw, th, down, twiddle),
            "pbe reg fmt {fmt} {tw}x{th}"
        );
        let (tx, ty) = (1 + r.next() % 1100, 1 + r.next() % 1100);
        let (ld, ls) = (r.next() & 1 != 0, r.next() & 1 != 0);
        assert_eq!(isp_zlsctl(tx, ty, ld, ls), old_zlsctl(tx, ty, ld, ls), "zlsctl {tx}x{ty}");
    }
    for (tw, th) in [(64, 32), (1, 1), (16384, 2)] {
        assert_eq!(
            pbe_reg_words(GLES_SURF_RGBA8888, false, tw, th, tw.wrapping_add(1) & !1, false, true),
            old_pbe_reg(GLES_SURF_RGBA8888, false, tw, th, false, true),
            "twiddled {tw}x{th}"
        );
    }
}

#[test]
fn csbgen_background_registers_equal_the_hand_packed_ones() {
    let mut r = Lcg(2);
    for _ in 0..20000 {
        /* PDS heap offsets (PDS_ALIGN), DMA data sizes and temps */
        let (shader, code, data) =
            ((r.next() % 0x10_0000) & !15, (r.next() % 0x10_0000) & !15, (r.next() % 0x10_0000) & !15);
        let (data_dw, temps) = (r.next() % 512, r.next() % 64);
        let new = [
            dw64(csb_pack!(
                ROGUE_CR_PDS_BGRND0_BASE_pack,
                2,
                csb::ROGUE_CR_PDS_BGRND0_BASE {
                    shader_addr: dev_addr(shader as u64),
                    texunicode_addr: dev_addr(code as u64)
                }
            )),
            dw64(csb_pack!(
                ROGUE_CR_PDS_BGRND1_BASE_pack,
                2,
                csb::ROGUE_CR_PDS_BGRND1_BASE { varying_addr: dev_addr(0), texturedata_addr: dev_addr(data as u64) }
            )),
            dw64(csb_pack!(
                ROGUE_CR_PDS_BGRND3_SIZEINFO_pack,
                2,
                csb::ROGUE_CR_PDS_BGRND3_SIZEINFO {
                    usc_sharedsize: CLEAR_ATTACH_DWORDS.div_ceil(16),
                    pds_texturestatesize: data_dw.wrapping_mul(4).div_ceil(4),
                    pds_tempsize: temps.div_ceil(4),
                    ..Default::default()
                }
            )),
        ];
        assert_eq!(new, old_bgnd(shader, code, data, data_dw, temps));
        /* the ISP bases: the address as it is, the stencil's enable in bit 0 */
        let a = heap_addr(&mut r);
        assert_eq!(
            dw64(csb_pack!(ROGUE_CR_ISP_ZLOAD_BASE_pack, 2, csb::ROGUE_CR_ISP_ZLOAD_BASE { addr: dev_addr(a) })),
            a
        );
        assert_eq!(
            dw64(csb_pack!(
                ROGUE_CR_ISP_STENCIL_LOAD_BASE_pack,
                2,
                csb::ROGUE_CR_ISP_STENCIL_LOAD_BASE { addr: dev_addr(a), enable: true }
            )),
            a | ISP_STENCIL_BASE_SEPARATE
        );
        let a4 = a | (r.next() as u64 & 0xc);
        assert_eq!(
            dw64(csb_pack!(ROGUE_CR_ISP_SCISSOR_BASE_pack, 2, csb::ROGUE_CR_ISP_SCISSOR_BASE { addr: dev_addr(a4) })),
            a4
        );
        assert_eq!(
            dw64(csb_pack!(ROGUE_CR_ISP_DBIAS_BASE_pack, 2, csb::ROGUE_CR_ISP_DBIAS_BASE { addr: dev_addr(a4) })),
            a4
        );
    }
}

#[test]
fn csbgen_texture_descriptors_equal_the_hand_packed_ones() {
    let mut r = Lcg(3);
    let fmts = [
        GLES_TEXFMT_RGBA8,
        GLES_TEXFMT_ETC2_RGB,
        GLES_TEXFMT_ETC2_RGBA,
        GLES_TEXFMT_ETC2_RGB_PT,
        GLES_TEXFMT_RGB565,
        GLES_TEXFMT_RGBA16F,
        GLES_TEXFMT_R16F,
        GLES_TEXFMT_R32F,
        GLES_TEXFMT_L8,
        GLES_TEXFMT_A8,
        GLES_TEXFMT_LA8,
    ];
    let wraps = [GLES_WRAP_REPEAT, GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_MIRRORED_REPEAT];
    let types = [GLES_TEXTYPE_2D, GLES_TEXTYPE_CUBE, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE];
    let (mut wn, mut wo) = (0, 0);
    for i in 0..50000 {
        let w = if r.next().is_multiple_of(16) { 0 } else { 1 + r.next() % 4096 };
        let t = GlesTextureBind {
            format: fmts[r.next() as usize % fmts.len()],
            type_: types[r.next() as usize % types.len()],
            base_gpu: (HEAP_GENERAL_BASE + ((r.next() as u64) << 6)) & !3,
            width: w,
            height: if r.next().is_multiple_of(16) { 0 } else { 1 + r.next() % 4096 },
            stride_texels: if r.next().is_multiple_of(16) { 0 } else { w.max(1).next_multiple_of(1 + r.next() % 16) },
            bytes: r.next(),
            linear_filter: (r.next() & 1) as u8,
            min_linear: (r.next() & 1) as u8,
            mip_filter: (r.next() % 3) as u8,
            mip_levels: (r.next() % 18) as u8,
            wrap_s: wraps[r.next() as usize % 3],
            wrap_t: wraps[r.next() as usize % 3],
            bgra: (r.next() & 1) as u8,
            compare_op: (r.next() % 9) as u8,
            depth_luminance: (r.next() & 1) as u8,
            ..GlesTextureBind::ZERO
        };
        let force = i % 7 == 0;
        let mut new = [0u32; TEX_DESC_DWORDS as usize];
        let mut old = [0u32; TEX_DESC_DWORDS as usize];
        build_tex_desc(&mut new, &t, &mut wn, force);
        old_build_tex_desc(&mut old, &t, &mut wo, force);
        assert_eq!(new, old, "{i}: format {} type {} {}x{}", t.format, t.type_, t.width, t.height);
    }
}
